//! アセット管理: 個人のライブラリ（プロジェクトをまたいで使う素材のフォルダ）と、サムネイルの別スレッドの仕事場・ディスクのキャッシュ。
//!
//! 置き場は 3 つの段で考える。①個人のライブラリ（このモジュール。設定の「棚の場所」のフォルダ。複数のプロジェクトと Unity 版が同じ
//! フォルダを指せる）、②プロジェクトの棚（`crate::shelf`。.ylp の resources）、③Unity のプロジェクト（Live Link でつないでいる間。第 5 波）。
//! 画面（`panels::assets`）は ① と ② を切り替えて、同じ格子で見せる。
//!
//! - 一覧: フォルダの下をたどる（別のスレッド。`yolu_io::library::list`）。読めないファイル・リンク・使えない名前は読み飛ばして理由を残す。
//! - 項目の情報とサムネイル: 見えている項目だけ別のスレッドで見て（`probe`）、中身の札（SHA-256）でディスクのキャッシュに覚える。
//!   画面のスレッドは、できた結果を受け取るだけ（1 フレームに数個）。
//! - プロジェクトで使う: ファイルを読んで検証し、写しを .ylp の棚へ入れる（出どころは `library`。参照だけにしない）。別のスレッドで走り、
//!   やめられる。同じ中身は 1 つ（棚の `add` が札で見分ける）。
//! - ライブラリへ入れる: 棚の素材のバイト列を、種類のフォルダへ書く（別のスレッドで走り、やめられる）。同じバイト列があれば書かない。
//! - 消す: 確かめてから、ファイルだけを消す。ほかのプロジェクトの写しと、置いた層は変わらない。
//! - ブラシは、今の `brushes/`（利用者のブラシ）をそのまま一覧に出す（ライブラリのフォルダへは写さない）。
//!
//! ③ Unity のプロジェクト（Live Link でつないでいる間だけ並ぶ置き場）を足すときの差し込み口:
//! - 置き場: `Source` に 1 つ足し（`ALL`・`name`・`tooltip`）、`panels::assets` の `cards` / `footer` / `menu_entries` の振り分けに
//!   同じ形の枝を足す（`library_cards` と同じく、一覧から `Card` を作る）。一覧は Live Link の「アセットの一覧（種類・名前・GUID・大きさ）」の
//!   答えを、このモジュールの `LibraryState` と同じ形（項目の並び・選び・見た結果の `HashMap`）の状態に受ける。
//! - サムネイル: 見えている項目だけ `service::Service` に頼む（札は GUID と内容の札）。仕事の中で Live Link の「サムネイル」を呼び、
//!   `shelf::Picture` にして返す。同じ内容の札の絵は `cache::Cache`（`Cache::key("unity", 内容の札)`）に覚える。仕事は `Cancel` を見て切り上げる。
//! - 中身: 「プロジェクトで使う」は `ShelfState::spawn_import` の仕事の中で、Live Link の「中身（テクスチャの画素）」を取り、
//!   `shelf::stage_library_image` と同じ形の足し方（出どころ `unityAsset`: GUID・パス・内容の札。yolu-io の `Shelf::add_image` は出どころを
//!   受け取れる）で棚の写しへ足す。元が変わった印は、`ProjectIndex` と同じく、棚の出どころの札と一覧の札を比べて付ける。
pub mod cache;
pub mod image;
pub mod ops;
pub mod probe;
pub mod service;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use egui::TextureHandle;
use yolu_io::library as files;
use yolu_io::shelf::Shelf;

use self::cache::Cache;
use self::probe::{Limits, Target};
use self::service::{Service, Work};
use crate::lang::Lang;
use crate::shelf::{
    Inspected, ItemKind, Unreadable, IMAGE_THUMB_PIXELS, IMPORT_LIMIT, PREVIEW_BUDGET,
};

/// ライブラリのファイルの項目の ID の前置き（プロジェクトの棚の ID は UUID、組み込みは `builtin:` なので重ならない）。
pub const LIBRARY_PREFIX: &str = "library:";
/// 利用者のブラシの項目の ID の前置き（残りは `BrushKey::token`）。
pub const BRUSH_PREFIX: &str = "brush:";
/// 項目を見るスレッドの数（描いている間も画面を止めないよう、少なく）。
pub const PROBE_WORKERS: usize = 2;
/// 1 フレームに画面のスレッドが受け取るできあがりの数。
pub const PROBES_PER_FRAME: usize = 8;
/// 覚えておく項目の情報（サムネイルを含む）の数の上限。超えたら、見ていない古いものから捨てる（また見えれば作り直す）。
pub const MAX_KNOWN: usize = 1024;

/// 足せない種類のファイルの断りの文言（`reason` が言語ごとの短い文にする）。
pub const REFUSAL_UNSUPPORTED: &str = "ライブラリへ追加できるのは PNG と .ylsmart だけです";
/// 画像の読めない理由の文言（`reason` が言語ごとの短い文にする）。
pub const REFUSAL_PNG: &str = "PNG として読めません";
pub const REFUSAL_PNG_SIZE: &str = "画像の 1 辺は 1〜8192 です";

/// 16 ビットの PNG を 8 ビットへ丸めて使ったときの知らせ（使う・置くの知らせに足す）。
pub fn rounded_note(lang: Lang) -> &'static str {
    lang.pick("16 bit を 8 bit にしました", "Reduced from 16 to 8 bits")
}

pub fn library_id(rel: &str) -> String {
    format!("{LIBRARY_PREFIX}{rel}")
}

/// ライブラリのファイルの項目の ID から、相対パスへ。
pub fn rel_of(id: &str) -> Option<&str> {
    id.strip_prefix(LIBRARY_PREFIX)
}

pub fn is_library_id(id: &str) -> bool {
    id.starts_with(LIBRARY_PREFIX)
}

/// アセットの欄が見せる置き場。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Source {
    /// このプロジェクトの棚（.ylp の resources と、同梱のスマートマテリアル）。
    #[default]
    Project,
    /// 個人のライブラリ（フォルダ）。
    Library,
}

impl Source {
    pub const ALL: [Source; 2] = [Source::Project, Source::Library];

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Source::Project => lang.pick("プロジェクト", "Project"),
            Source::Library => lang.pick("ライブラリ", "Library"),
        }
    }

    pub fn tooltip(self, lang: Lang) -> &'static str {
        match self {
            Source::Project => lang.pick(
                "このプロジェクトの棚（.ylp に保存される）",
                "This project's shelf (saved in the .ylp)",
            ),
            Source::Library => lang.pick(
                "個人のライブラリ（フォルダ。ほかのプロジェクトからも使える）",
                "Your library (a folder, shared by your projects)",
            ),
        }
    }
}

/// ライブラリのフォルダ・画像の断りの短い文（そうでない断りは None）。
pub fn known_reason(lang: Lang, e: &yolu_io::Error) -> Option<String> {
    let m = e.to_string();
    let has = |text: &str| m.contains(text);
    let short = if has(files::REFUSAL_ROOT_LINK) {
        lang.pick(
            "ライブラリの場所がリンクです",
            "The library folder is a link",
        )
    } else if has(files::REFUSAL_ROOT_NOT_FOLDER) {
        lang.pick(
            "ライブラリの場所がフォルダではありません",
            "The library location is not a folder",
        )
    } else if has(files::REFUSAL_LINK) {
        lang.pick("リンクはたどりません", "Links are not followed")
    } else if has(files::REFUSAL_PATH) {
        lang.pick("名前が使えません", "Name not allowed")
    } else if has(files::REFUSAL_NOT_FILE) {
        lang.pick("ファイルではありません", "Not a file")
    } else if has(files::REFUSAL_TOO_LARGE) {
        lang.pick("大きすぎます", "Too large")
    } else if has(files::REFUSAL_EMPTY) {
        lang.pick("空のファイルです", "Empty file")
    } else if has(files::REFUSAL_NO_NAME) {
        lang.pick("名前を決められません", "Cannot choose a name")
    } else if has(REFUSAL_UNSUPPORTED) {
        lang.pick("PNG と .ylsmart だけです", "PNG and .ylsmart only")
    } else if has(REFUSAL_PNG_SIZE) {
        lang.pick(REFUSAL_PNG_SIZE, "Image sides must be 1 to 8192")
    } else if has(REFUSAL_PNG) {
        lang.pick(REFUSAL_PNG, "Not a readable PNG")
    } else {
        return None;
    };
    Some(short.to_owned())
}

/// 断りの理由の短い文（ライブラリのフォルダの断り・画像の断りは言語ごとに、ほかは棚の言い方）。
pub fn reason(lang: Lang, e: &yolu_io::Error) -> String {
    known_reason(lang, e).unwrap_or_else(|| crate::shelf::io_reason(lang, e))
}

/// ファイルの状態（長さと更新時刻）。変わったら、情報を作り直す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    len: u64,
    modified: Option<u128>,
}

impl Stamp {
    fn of(entry: &files::Entry) -> Stamp {
        Stamp {
            len: entry.len,
            modified: entry
                .modified
                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos()),
        }
    }
}

/// ライブラリのファイル 1 つを見た結果。
pub struct LibInfo {
    /// 種類（スマート素材は、ファイルの中でマテリアルかマスクか。読めないものはマテリアル）。
    pub kind: ItemKind,
    pub inspected: Inspected,
    /// ファイルのバイト列の SHA-256（作っていなければ空）。
    pub sha256: String,
    /// 中身の札（スマート素材はファイルの札と同じ、画像は棚の画像と同じ画素の札。分からなければ空）。
    pub content: String,
}

/// 格子に並べる、ライブラリのファイル 1 つ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibItem {
    /// 項目の ID（`library:` + 相対パス）。
    pub id: String,
    pub rel: String,
    pub name: String,
    pub kind: ItemKind,
    pub len: u64,
}

struct Known {
    stamp: Stamp,
    info: LibInfo,
    used: u64,
}

/// いまのプロジェクトの棚にあるものの索引（種類・中身の札と、出どころのライブラリのファイル）。
#[derive(Default)]
pub struct ProjectIndex<'a> {
    contents: std::collections::HashSet<(&'static str, &'a str)>,
    files: std::collections::HashSet<(&'static str, &'a str)>,
}

impl ProjectIndex<'_> {
    /// ライブラリのファイル `rel`（中身の札 `content`。分からなければ空）が棚にあるか。スマート素材はファイルの札、画像は画素の札で
    /// 見分け、出どころがこのファイルの項目も数える（Unity 版と同じ）。
    pub fn contains(&self, kind: files::Kind, rel: &str, content: &str) -> bool {
        let tag = match kind {
            files::Kind::Image => "image",
            files::Kind::Smart => "smart",
            files::Kind::Brush => "brush",
            files::Kind::Material => "material",
        };
        self.files.contains(&(tag, rel))
            || (!content.is_empty() && self.contents.contains(&(tag, content)))
    }
}

/// 別のスレッドで見た結果。
struct Probed {
    rel: String,
    stamp: Stamp,
    info: LibInfo,
}

struct ListingRun {
    folder: PathBuf,
    rx: Receiver<Result<files::Listing, Unreadable>>,
    cancel: Arc<AtomicBool>,
}

impl Drop for ListingRun {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// 別のスレッドで走っているライブラリへの書き込み（ライブラリへ入れる・ファイルを足す）。
pub(crate) struct PendingWrite {
    pub(crate) name: String,
    pub(crate) rx: Receiver<ops::WriteDone>,
    /// やめたことをスレッドへ伝える旗（書き込みの区切りで切り上げる）。
    pub(crate) cancel: Arc<AtomicBool>,
}

impl Drop for PendingWrite {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

fn probe_kind(kind: files::Kind) -> ItemKind {
    match kind {
        files::Kind::Image => ItemKind::Image,
        files::Kind::Smart => ItemKind::SmartMaterial,
        files::Kind::Brush => ItemKind::Brush,
        files::Kind::Material => ItemKind::Material,
    }
}

/// 個人のライブラリの状態（一覧・見た結果・選び・書き込みの仕事）。
pub struct LibraryState {
    /// 画面が見せている置き場。
    pub source: Source,
    /// 選んでいる項目の ID（`library:` + 相対パス、または `brush:` + ブラシの札）。
    pub selected: Option<String>,
    pub scroll: f32,
    /// ライブラリの格子の範囲（このフレームで描いたときだけ入る。窓に落としたファイルが格子の上かを見る。
    /// 棚・チャンネルのタブを開いている間は入らない）。
    pub grid_rect: Option<egui::Rect>,
    /// 消してよいか確かめている相対パス（確かめの窓は `YoluApp` が出す）。
    pub pending_remove: Option<String>,
    /// 項目を見るときの上限（試験で小さくする）。
    pub limits: Limits,
    cache: Option<Arc<Cache>>,
    context: Option<egui::Context>,
    /// いま一覧にしているフォルダ（設定が変わったら一覧を捨てる）。
    folder: Option<PathBuf>,
    listed: bool,
    problem: Option<Unreadable>,
    entries: Vec<files::Entry>,
    index: HashMap<String, usize>,
    skipped: Vec<files::Skipped>,
    truncated: bool,
    stale: bool,
    run: Option<ListingRun>,
    known: HashMap<String, Known>,
    /// 直近に頼んだ（見えている）項目（覚えている情報が上限を超えたとき、これは捨てない）。
    visible_now: std::collections::HashSet<String>,
    inspector: Service<Probed>,
    clock: u64,
    pub(crate) write: Option<PendingWrite>,
    /// 走っている書き込みのスレッドの数（やめた書き込みのスレッドが終わるまで次の書き込みを断る）。
    pub(crate) running: Arc<AtomicUsize>,
    /// 試験用: true の間、書き込みのスレッドは仕事を始めずに待つ。
    pub(crate) hold: Arc<AtomicBool>,
}

impl Default for LibraryState {
    fn default() -> Self {
        LibraryState {
            source: Source::Project,
            selected: None,
            scroll: 0.0,
            grid_rect: None,
            pending_remove: None,
            limits: Limits {
                read: IMPORT_LIMIT,
                preview: PREVIEW_BUDGET,
                thumb_pixels: IMAGE_THUMB_PIXELS,
            },
            cache: None,
            context: None,
            folder: None,
            listed: false,
            problem: None,
            entries: Vec::new(),
            index: HashMap::new(),
            skipped: Vec::new(),
            truncated: false,
            stale: true,
            run: None,
            known: HashMap::new(),
            visible_now: Default::default(),
            inspector: Service::new(PROBE_WORKERS),
            clock: 0,
            write: None,
            running: Arc::new(AtomicUsize::new(0)),
            hold: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl LibraryState {
    /// サムネイルのキャッシュのフォルダ（None ならディスクには覚えない）。
    pub fn attach_cache(&mut self, dir: Option<PathBuf>) {
        self.cache = dir.map(|d| Arc::new(Cache::new(d)));
    }

    /// 上限を指定してキャッシュを付ける（試験用）。
    pub fn attach_cache_with(&mut self, cache: Cache) {
        self.cache = Some(Arc::new(cache));
    }

    pub fn cache(&self) -> Option<&Arc<Cache>> {
        self.cache.as_ref()
    }

    /// できたときに画面を描き直させる口を覚える。
    pub fn set_context(&mut self, ctx: &egui::Context) {
        self.inspector.set_context(ctx.clone());
        self.context = Some(ctx.clone());
    }

    /// 新しいフレームの始まり（見えている項目を `request_probes` で頼む前に 1 度）。
    pub fn begin_frame(&self) {
        self.inspector.begin_frame();
    }

    /// 次の書き込みを始められない理由（書き込みが走っている間。やめた書き込みのスレッドが終わるまでも）。
    pub fn busy_reason(&self, lang: Lang) -> Option<&'static str> {
        if self.write.is_some() {
            Some(lang.pick(
                "ライブラリへ書き込み中です",
                "Already writing to the library",
            ))
        } else if self.running.load(Ordering::SeqCst) > 0 {
            Some(lang.pick(
                "やめた書き込みを終えています",
                "Finishing the cancelled write",
            ))
        } else {
            None
        }
    }

    /// 書き込み中の名前（なければ None）。
    pub fn writing_name(&self) -> Option<&str> {
        self.write.as_ref().map(|w| w.name.as_str())
    }

    /// 試験用: true の間、書き込みのスレッドは仕事を始めずに待つ（途中の画面・取り消しを確かめるため）。
    pub fn hold_writes(&self, hold: bool) {
        self.hold.store(hold, Ordering::SeqCst);
    }

    /// 試験用: 走っている書き込みのスレッドがすべて終わるまで待つ。
    pub fn wait_idle(&self) {
        while self.running.load(Ordering::SeqCst) > 0 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    // ───────── 一覧 ─────────

    /// 一覧にするフォルダを決め、古い・まだなら読み始める（別のスレッド）。設定でフォルダが替わったら、前のフォルダの一覧と情報を捨てる。
    pub fn prepare(&mut self, folder: Option<PathBuf>) {
        if self.folder != folder {
            self.folder = folder;
            self.run = None;
            self.clear_listing();
            self.stale = true;
        }
        if self.stale && self.run.is_none() {
            self.start_listing();
        }
    }

    fn clear_listing(&mut self) {
        self.listed = false;
        self.problem = None;
        self.entries.clear();
        self.index.clear();
        self.skipped.clear();
        self.truncated = false;
        self.known.clear();
        self.selected = self.selected.take().filter(|id| !is_library_id(id));
    }

    /// 一覧を読み直す頼み（次の `prepare` が読み始める。読んでいる最中なら、それが終わってからもう一度）。読めなかった項目の情報は捨てる
    /// （ファイルが変わっていなくても、一時的な失敗は読み直しでやり直せる）。
    pub fn refresh(&mut self) {
        self.stale = true;
        self.known.retain(|_, known| {
            !matches!(
                known.info.inspected.block,
                Some(crate::shelf::Block::Unreadable(_))
            )
        });
    }

    fn start_listing(&mut self) {
        let Some(folder) = self.folder.clone() else {
            self.stale = false;
            self.listed = true;
            self.problem = Some(Unreadable::pair(
                "ライブラリの場所がありません",
                "No library location",
            ));
            return;
        };
        self.stale = false;
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let repaint = self.context.clone();
        let root = folder.clone();
        let spawned = std::thread::Builder::new()
            .name("yolu-library-list".into())
            .spawn(move || {
                let result = files::list(&root, Some(&flag))
                    .map_err(|e| Unreadable::pair(&reason(Lang::Ja, &e), &reason(Lang::En, &e)));
                let _ = tx.send(result);
                if let Some(ctx) = repaint {
                    ctx.request_repaint();
                }
            });
        if spawned.is_err() {
            self.listed = true;
            self.problem = Some(Unreadable::pair("一覧を読めません", "Cannot read the list"));
            return;
        }
        self.run = Some(ListingRun { folder, rx, cancel });
    }

    /// 読み終えた一覧を受け取る。受け取ったら true。
    fn poll_listing(&mut self) -> bool {
        let Some(run) = &self.run else { return false };
        let result = match run.rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err(Unreadable::pair("一覧を読めません", "Cannot read the list"))
            }
        };
        let run = self.run.take().expect("上で確かめた");
        if self.folder.as_ref() != Some(&run.folder) {
            return true;
        }
        match result {
            Ok(listing) => self.adopt(listing),
            Err(why) => {
                self.clear_listing();
                self.listed = true;
                self.problem = Some(why);
            }
        }
        true
    }

    fn adopt(&mut self, listing: files::Listing) {
        let index: HashMap<String, usize> = listing
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.rel.clone(), i))
            .collect();
        // 変わらないファイルの情報は残す（長さと更新時刻が同じ）
        self.known.retain(|rel, known| {
            index
                .get(rel)
                .is_some_and(|i| Stamp::of(&listing.entries[*i]) == known.stamp)
        });
        self.entries = listing.entries;
        self.index = index;
        self.skipped = listing.skipped;
        self.truncated = listing.truncated;
        self.problem = None;
        self.listed = true;
        if let Some(id) = &self.selected {
            if rel_of(id).is_some_and(|rel| !self.index.contains_key(rel)) {
                self.selected = None;
            }
        }
    }

    /// 読み込み中か（一覧を読んでいる・頼んだ項目の結果を待っている）。
    pub fn is_busy(&self) -> bool {
        self.run.is_some() || self.stale || self.inspector.pending() > 0
    }

    /// 一覧を読めなかった・場所が無い理由。
    pub fn problem(&self) -> Option<&Unreadable> {
        self.problem.as_ref()
    }

    /// 一覧を（空でも）読み終えたか。
    pub fn is_listed(&self) -> bool {
        self.listed
    }

    /// 読み飛ばしたファイル（相対パスと理由）。
    pub fn skipped(&self) -> &[files::Skipped] {
        &self.skipped
    }

    /// 個数の上限で打ち切ったか。
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn entry(&self, rel: &str) -> Option<&files::Entry> {
        self.index.get(rel).map(|i| &self.entries[*i])
    }

    // ───────── 見た結果 ─────────

    /// 見た結果（まだ・古ければ None）。
    pub fn info(&self, rel: &str) -> Option<&LibInfo> {
        let entry = self.entry(rel)?;
        self.known
            .get(rel)
            .filter(|k| k.stamp == Stamp::of(entry))
            .map(|k| &k.info)
    }

    /// 項目の種類（見ていればファイルの中の種類、まだなら拡張子から）。
    pub fn kind_of(&self, entry: &files::Entry) -> ItemKind {
        self.info(&entry.rel)
            .map_or_else(|| probe_kind(entry.kind), |i| i.kind)
    }

    /// サムネイルのテクスチャ（できていれば。初めて使うときに GPU へ上げる）。
    pub fn texture(&mut self, ctx: &egui::Context, rel: &str) -> Option<TextureHandle> {
        let stamp = Stamp::of(self.entry(rel)?);
        let known = self.known.get_mut(rel).filter(|k| k.stamp == stamp)?;
        known.info.inspected.texture(ctx, &format!("library:{rel}"))
    }

    /// 見えている項目のうちまだ見ていないものを、別のスレッドへ頼む（頼み済みは見に来た印だけ）。ブラシ・マテリアルはその場で決める。
    pub fn request_probes(&mut self, rels: &[String]) {
        self.clock += 1;
        self.visible_now = rels.iter().cloned().collect();
        let Some(root) = self.folder.clone() else {
            return;
        };
        for rel in rels {
            let Some(entry) = self.entry(rel).cloned() else {
                continue;
            };
            let stamp = Stamp::of(&entry);
            if let Some(known) = self.known.get_mut(rel).filter(|k| k.stamp == stamp) {
                known.used = self.clock;
                continue;
            }
            let target = Target {
                root: root.clone(),
                rel: entry.rel.clone(),
                kind: entry.kind,
                len: entry.len,
            };
            let limits = self.limits;
            if matches!(entry.kind, files::Kind::Brush | files::Kind::Material) {
                let info = probe::probe(&target, limits, None, &service::Cancel::default());
                self.known.insert(
                    rel.clone(),
                    Known {
                        stamp,
                        info,
                        used: self.clock,
                    },
                );
                continue;
            }
            let key = format!("{rel}\0{}\0{}", stamp.len, stamp.modified.unwrap_or(0));
            let cache = self.cache.clone();
            self.inspector.request(&key, move || -> Work<Probed> {
                Box::new(move |cancel| Probed {
                    rel: target.rel.clone(),
                    stamp,
                    info: probe::probe(&target, limits, cache.as_deref(), cancel),
                })
            });
        }
        self.evict();
    }

    /// 覚えている情報が上限を超えたら、いま見えていない古いものから上限の 3/4 まで捨てる（また見えれば作り直す）。
    fn evict(&mut self) {
        if self.known.len() <= MAX_KNOWN {
            return;
        }
        let mut old: Vec<(u64, String)> = self
            .known
            .iter()
            .filter(|(rel, _)| !self.visible_now.contains(rel.as_str()))
            .map(|(rel, k)| (k.used, rel.clone()))
            .collect();
        old.sort();
        for (_, rel) in old.into_iter().take(self.known.len() - MAX_KNOWN * 3 / 4) {
            self.known.remove(&rel);
        }
    }

    /// できあがった項目を `limit` 個まで受け取って覚える（一覧から消えた・変わったファイルの結果は捨てる）。受け取った数。
    pub fn poll_probes(&mut self, limit: usize) -> usize {
        let done = self.inspector.poll(limit);
        let count = done.len();
        for (key, result) in done {
            let rel = key.split('\0').next().unwrap_or("").to_owned();
            let Some(entry) = self.entry(&rel) else {
                continue;
            };
            let current = Stamp::of(entry);
            match result {
                Some(Probed { rel, stamp, info }) => {
                    if stamp == current {
                        let used = self.clock;
                        self.known.insert(rel, Known { stamp, info, used });
                    }
                }
                // 仕事が途中で落ちたファイルは、読めない印で覚える（頼み直し続けない）
                None => {
                    let kind = probe_kind(entry.kind);
                    let used = self.clock;
                    self.known.insert(
                        rel,
                        Known {
                            stamp: current,
                            info: LibInfo {
                                kind,
                                inspected: Inspected::bare(Some(crate::shelf::Block::Unreadable(
                                    Unreadable::same("?"),
                                ))),
                                sha256: String::new(),
                                content: String::new(),
                            },
                            used,
                        },
                    );
                }
            }
        }
        self.evict();
        count
    }

    /// 毎フレーム: 読み終えた一覧と、できあがった項目を受け取る。受け取ったものがあれば true。
    pub fn poll(&mut self) -> bool {
        let listed = self.poll_listing();
        let probed = self.poll_probes(PROBES_PER_FRAME);
        listed || probed > 0
    }

    /// 試験用: 一覧を読み終え、頼んだ仕事がすべて終わるまで待って、できた分を受け取る。
    pub fn wait_probes(&mut self) {
        let start = std::time::Instant::now();
        while self.run.is_some() {
            self.poll_listing();
            assert!(start.elapsed().as_secs() < 30, "一覧の待ちすぎ");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        self.inspector.wait_done();
        while self.poll_probes(usize::MAX) > 0 {}
    }

    /// 試験用: true の間、別のスレッドは新しい項目を見始めない。
    pub fn hold_probes(&self, hold: bool) {
        self.inspector.hold(hold);
    }

    /// 頼んであって、結果をまだ受け取っていない項目の数（試験用）。
    pub fn probes_pending(&self) -> usize {
        self.inspector.pending()
    }

    /// 覚えている項目の数（試験用）。
    pub fn known_count(&self) -> usize {
        self.known.len()
    }

    // ───────── 一覧に出す項目 ─────────

    /// 種類の絞り込み（None はすべて）・名前または相対パスの検索（大文字小文字を区別しない）に合うファイル（一覧の並びのまま）。
    /// まだ見ていない .ylsmart は、マテリアルかマスクかファイルを開くまで分からないので、スマートの絞り込み（マテリアル・マスクのどちらも）では
    /// 候補として残す（残さないと、見に行く前に除かれて、マスクのファイルが一度も出ない）。見た結果が出たら、その種類で絞り直される。
    pub fn visible(&self, filter: Option<ItemKind>, search: &str) -> Vec<LibItem> {
        let needle = search.trim().to_lowercase();
        self.entries
            .iter()
            .filter_map(|e| {
                let kind = self.kind_of(e);
                let kind = match filter {
                    Some(f) if self.could_be(e, f) => f,
                    Some(_) => return None,
                    None => kind,
                };
                if !needle.is_empty()
                    && !e.name().to_lowercase().contains(&needle)
                    && !e.rel.to_lowercase().contains(&needle)
                {
                    return None;
                }
                Some(LibItem {
                    id: library_id(&e.rel),
                    rel: e.rel.clone(),
                    name: e.name().to_owned(),
                    kind,
                    len: e.len,
                })
            })
            .collect()
    }

    /// 項目が種類 `kind` でありうるか（見ていればその種類。見ていない .ylsmart はスマートの種類のどちらでもありうる）。
    fn could_be(&self, entry: &files::Entry, kind: ItemKind) -> bool {
        match self.info(&entry.rel) {
            Some(info) => info.kind == kind,
            None => {
                probe_kind(entry.kind) == kind
                    || (entry.kind == files::Kind::Smart && kind.is_smart())
            }
        }
    }

    /// いまのプロジェクトの棚にあるものの索引（一覧の全部を調べても、棚の個数に比例する時間で済む）。
    pub fn project_index<'a>(&self, shelf: &'a Shelf) -> ProjectIndex<'a> {
        let mut index = ProjectIndex::default();
        for r in shelf.resources() {
            let tag = match r.kind.as_str() {
                "image" => "image",
                "smartMaterial" | "smartMask" => "smart",
                "brush" => "brush",
                "material" => "material",
                _ => continue,
            };
            index.contents.insert((tag, r.content.as_str()));
            let origin = &r.metadata["origin"];
            if origin["type"] == "library" {
                if let Some(file) = origin["file"].as_str() {
                    index.files.insert((tag, file));
                }
            }
        }
        index
    }

    /// 項目が、いまのプロジェクトの棚に（同じ中身で）あるか。
    pub fn in_project(&self, shelf: &Shelf, rel: &str) -> bool {
        let content = self.info(rel).map(|i| i.content.as_str()).unwrap_or("");
        self.entry(rel)
            .is_some_and(|e| self.project_index(shelf).contains(e.kind, rel, content))
    }

    /// 画素の札が同じ画像のファイル（見たものだけ）。無ければ None。
    pub fn find_image_content(&self, content: &str) -> Option<&str> {
        if content.is_empty() {
            return None;
        }
        self.entries
            .iter()
            .find(|e| {
                e.kind == files::Kind::Image
                    && self
                        .info(&e.rel)
                        .is_some_and(|i| i.kind == ItemKind::Image && i.content == content)
            })
            .map(|e| e.rel.as_str())
    }
}
