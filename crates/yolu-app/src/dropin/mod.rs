//! ファイルの取り込みのウィンドウ: ウィンドウに落とした PSD と画像（PNG・JPEG・TGA）を 1 つのウィンドウの行にし、行ごとに入れ先を選んで取り込む。
//!
//! - 入れ先は「アセットに入れるだけ」（画像だけ）・テクスチャセット（ファイルの名前の新しいレイヤーセット）・レイヤーセット（その一番上）。
//!   既定は何も選んでいない形で、読めるファイルの入れ先が全部決まるまで取り込めない。「すべて」は、その入れ先に入れられる行にだけ当てる
//!   （PSD の行にアセットを当てない。大きさの違う PSD の行にそのテクスチャセットを当てない）。
//! - 画像はアセットに入れ（PNG 以外も straight RGBA8。同じ中身ならその画像）、レイヤーの入れ先なら、メニューの「画像の塗りつぶしレイヤー」と
//!   同じ塗りつぶしレイヤー（UV の投影）を 1 枚。PSD は取り込みの確かめ（落とす・変わる・無視。行を開いて見る）を通した文書のレイヤーを、
//!   core の `Document::insert_layers_from`・`insert_layers_as_layer_set` で入れる。大きさがテクスチャセットと違う PSD は、そのテクスチャセットへ
//!   入れない（大きさを変えない）。
//! - 取り消しは入れ先の文書ごとに 1 回（同じテクスチャセットへの複数のファイルは 1 回にまとめる）。描いている最中・読むだけのセット・アセットの保存の
//!   最中は断る。読むのは別のスレッド（PSD は取り込みと同じ `psd::read_copy`、画像は `fillfx::read_image`）。

pub mod window;

use std::path::{Path, PathBuf};

use yolu_core::fill_image::ProjectionMode;
use yolu_core::{Channel, ChannelKind, Document, HistoryKind, ImageId, LayerId, LayerSetId};
use yolu_io::psd::{CopyOutcome, ImportNote};

use crate::jobs::{JobSpec, Polled, Worker};
use crate::lang::Lang;
use crate::notice::Source;
use crate::shelf::Picture;
use crate::state::{AppState, UiSlot};

/// ウィンドウの名前（`windows::window_rect` で矩形を引く名前）。
pub const WINDOW: &str = "import-files";

/// サムネイルの長い辺（画素）。
const THUMB_SIDE: u32 = 72;

/// 入れ先 1 つ。テクスチャセットは uid（プロセスの中で一意）で持つ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dest {
    /// レイヤーにはせず、アセットに入れるだけ（画像だけ）。
    Asset,
    /// そのテクスチャセットの新しいレイヤーセット（名前はファイルの名前）。
    NewSet(u32),
    /// 既にあるレイヤーセットの一番上。
    Set(u32, LayerSetId),
}

impl Dest {
    /// 入れ先のテクスチャセットの uid（アセットは None）。
    pub fn texture_set(self) -> Option<u32> {
        match self {
            Dest::Asset => None,
            Dest::NewSet(uid) | Dest::Set(uid, _) => Some(uid),
        }
    }
}

/// 入れ先の一覧を開いている箱。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuFor {
    /// 「すべて」の箱。
    All,
    /// その行の箱（行の番号 `Row::id`）。
    Row(u64),
}

/// ファイルの取り込みのウィンドウの操作（`Action::DropIn`）。
#[derive(Clone, Debug, PartialEq)]
pub enum DropOp {
    /// 落としたファイルを行にしてウィンドウを開く（開いていれば行を足す）。
    Open(Vec<PathBuf>),
    /// 行の入れ先を選ぶ。
    SetDest(u64, Dest),
    /// 「すべて」の入れ先を選ぶ（入れられる行にだけ当てる）。
    SetAll(Dest),
    /// 入れ先の一覧を開く・閉じる。
    Menu(Option<MenuFor>),
    /// PSD の行の取り込みの確かめを開く・閉じる。
    ToggleCheck(u64),
    /// 一覧から外す。
    Remove(u64),
    /// 「やめる」・閉じる。
    Cancel,
    /// 「取り込む」。
    Import,
}

/// 読んだ PSD（取り込みの確かめの知らせと、入れる文書）。
pub struct PsdRead {
    pub(crate) doc: Box<Document>,
    pub(crate) notes: Vec<ImportNote>,
}

impl PsdRead {
    pub fn size(&self) -> (u32, u32) {
        (self.doc.width(), self.doc.height())
    }
    pub fn notes(&self) -> &[ImportNote] {
        &self.notes
    }
}

/// 読んだ画像（左下原点・straight RGBA8）とサムネイル。
pub struct ImageRead {
    pub(crate) rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// 形式の名前（PNG・JPEG・TGA）。
    pub format: &'static str,
    /// GPU へまだ上げていないサムネイル（上の行から）。
    pub(crate) thumb: Option<Picture>,
    pub(crate) slot: UiSlot,
}

/// 行の中身。
pub enum Content {
    /// 別のスレッドで読んでいる。
    Reading,
    Psd(Box<PsdRead>),
    Image(Box<ImageRead>),
    /// 取り込めない（形式の名前と短い理由）。
    Refused {
        format: String,
        reason: String,
    },
}

/// ウィンドウの 1 行（落としたファイル 1 つ）。
pub struct Row {
    pub id: u64,
    pub path: PathBuf,
    /// ファイルの名前（拡張子つき）。
    pub name: String,
    pub content: Content,
    pub dest: Option<Dest>,
    /// PSD の取り込みの確かめを開いている。
    pub open: bool,
}

impl Row {
    pub fn is_psd(&self) -> bool {
        matches!(self.content, Content::Psd(_)) || kind_of(&self.path) == Kind::Psd
    }
    /// 読めて、入れ先を選べる行。
    pub fn is_readable(&self) -> bool {
        matches!(self.content, Content::Psd(_) | Content::Image(_))
    }
    pub fn is_refused(&self) -> bool {
        matches!(self.content, Content::Refused { .. })
    }
}

type Read = (u64, Content);

/// ファイルの取り込みのウィンドウの状態。行が 1 つでもあればウィンドウを出す。
#[derive(Default)]
pub struct DropInState {
    pub rows: Vec<Row>,
    /// 「すべて」の箱に出す入れ先（最後に選んだ物）。
    pub all: Option<Dest>,
    /// 入れ先の一覧を開いている箱。
    pub menu: Option<MenuFor>,
    next_id: u64,
    readers: Vec<Worker<Read>>,
    /// ウィンドウを見出しのドラッグで動かした量。
    pub offset: egui::Vec2,
    /// 行の一覧のずらした量。
    pub scroll: f32,
}

impl DropInState {
    pub fn is_open(&self) -> bool {
        !self.rows.is_empty()
    }
    /// 別のスレッドで読んでいる。
    pub fn is_reading(&self) -> bool {
        !self.readers.is_empty()
    }
    pub fn row(&self, id: u64) -> Option<&Row> {
        self.rows.iter().find(|r| r.id == id)
    }
    fn row_mut(&mut self, id: u64) -> Option<&mut Row> {
        self.rows.iter_mut().find(|r| r.id == id)
    }
    /// 「取り込む」を押せるか: 読んでいる行が無く、読める行が 1 つ以上あり、その入れ先が全部決まっている。
    pub fn ready(&self) -> bool {
        let readable: Vec<&Row> = self.rows.iter().filter(|r| r.is_readable()).collect();
        !readable.is_empty()
            && !self
                .rows
                .iter()
                .any(|r| matches!(r.content, Content::Reading))
            && readable.iter().all(|r| r.dest.is_some())
    }
    fn close(&mut self) {
        for r in &self.readers {
            r.cancel();
        }
        self.readers.clear();
        self.rows.clear();
        self.all = None;
        self.menu = None;
        self.scroll = 0.0;
    }
}

/// ファイルの種類（拡張子で決める）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Psd,
    Image(image::ImageFormat),
    /// 画像だが読めない形式（新しい依存が要る物）。
    OtherImage,
    Other,
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

fn kind_of(path: &Path) -> Kind {
    match extension(path).as_str() {
        "psd" => Kind::Psd,
        "png" => Kind::Image(image::ImageFormat::Png),
        "jpg" | "jpeg" => Kind::Image(image::ImageFormat::Jpeg),
        "tga" => Kind::Image(image::ImageFormat::Tga),
        "webp" | "tif" | "tiff" | "exr" | "bmp" | "gif" | "hdr" | "psb" | "dds" | "avif"
        | "heic" | "jxl" | "qoi" => Kind::OtherImage,
        _ => Kind::Other,
    }
}

/// このファイルを落としたらウィンドウを出すか（PSD と画像。読めない形式の画像も、理由を見せるために出す）。
pub fn opens_window(path: &Path) -> bool {
    matches!(kind_of(path), Kind::Psd | Kind::Image(_) | Kind::OtherImage)
}

/// 形式の名前（行の 2 行目）。
fn format_name(path: &Path) -> String {
    match kind_of(path) {
        Kind::Psd => "PSD".into(),
        Kind::Image(image::ImageFormat::Png) => "PNG".into(),
        Kind::Image(image::ImageFormat::Jpeg) => "JPEG".into(),
        Kind::Image(_) => "TGA".into(),
        Kind::OtherImage | Kind::Other => {
            let e = extension(path);
            match e.as_str() {
                "tif" | "tiff" => "TIFF".into(),
                "webp" => "WebP".into(),
                _ => e.to_ascii_uppercase(),
            }
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// ファイルの名前から、レイヤーセット・レイヤー・アセットの名前の元を作る（拡張子を除き、制御文字は `_` に、128 文字まで。空なら `fallback`）。
fn stem_name(path: &Path, fallback: &str) -> String {
    let stem: String = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .take(128)
        .collect();
    let stem = stem.trim();
    if stem.is_empty() {
        fallback.into()
    } else {
        stem.into()
    }
}

/// 上の行から並んだ RGBA8 の、長い辺 `THUMB_SIDE` までのサムネイル（近い画素を取る）。
fn thumbnail(rgba_top_down: &[u8], w: u32, h: u32) -> Picture {
    let scale = (THUMB_SIDE as f32 / w.max(h) as f32).min(1.0);
    let tw = ((w as f32 * scale).round() as u32).max(1);
    let th = ((h as f32 * scale).round() as u32).max(1);
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    for y in 0..th {
        let sy = ((y as u64 * h as u64) / th as u64) as usize;
        for x in 0..tw {
            let sx = ((x as u64 * w as u64) / tw as u64) as usize;
            let i = (sy * w as usize + sx) * 4;
            out.extend_from_slice(&rgba_top_down[i..i + 4]);
        }
    }
    Picture {
        width: tw,
        height: th,
        rgba: out,
    }
}

/// 別のスレッドで 1 つのファイルを読む。
fn read_file(
    path: &Path,
    lang: Lang,
    budget: u64,
    cancel: &std::sync::atomic::AtomicBool,
) -> Content {
    let refused = |reason: String| Content::Refused {
        format: format_name(path),
        reason,
    };
    match kind_of(path) {
        Kind::Psd => match crate::psd::read_copy_on_psd_stack(path, budget, cancel) {
            Ok(CopyOutcome::Imported(imported)) => {
                let yolu_io::psd::CopyImport { document, notes } = *imported;
                Content::Psd(Box::new(PsdRead {
                    doc: Box::new(document),
                    notes,
                }))
            }
            Ok(CopyOutcome::Refused(why)) => refused(crate::lang::psd_copy_refusal(lang, &why)),
            Err(crate::psd::ReadCopyError::File(e)) => refused(lang.file_error(&e)),
            Err(crate::psd::ReadCopyError::Canceled) => refused(String::new()),
            Err(crate::psd::ReadCopyError::Other(e)) => refused(lang.io_error(&e)),
        },
        Kind::Image(format) => match crate::fillfx::read_image(path, format, lang) {
            Ok((mut rgba, w, h)) => {
                let thumb = thumbnail(&rgba, w, h);
                crate::fillfx::flip_rows(&mut rgba, w as usize * 4);
                Content::Image(Box::new(ImageRead {
                    rgba,
                    width: w,
                    height: h,
                    format: match format {
                        image::ImageFormat::Png => "PNG",
                        image::ImageFormat::Jpeg => "JPEG",
                        _ => "TGA",
                    },
                    thumb: Some(thumb),
                    slot: UiSlot::new(),
                }))
            }
            Err(why) => refused(why),
        },
        Kind::OtherImage | Kind::Other => refused(lang.t("dropin-unsupported-format").into()),
    }
}

/// 裏の読み込み（進み具合の表示は出さない。ウィンドウの行が読んでいる形を見せる）。ウィンドウはキーの割り当てを止める。
pub(crate) const JOB: JobSpec = JobSpec {
    repaint: true,
    cancel: Some(|app| {
        for r in &app.dropin.readers {
            r.cancel();
        }
    }),
    poll_while_stopping: Some(AppState::poll_dropin),
    modal: Some(|app| app.dropin.is_open()),
    ..JobSpec::new("dropin", |app| app.dropin.is_reading())
};

/// 機能の表の行（毎フレーム、読み終わった行を受ける）。
pub(crate) const FEATURE: crate::features::Feature = crate::features::Feature {
    poll: Some((
        crate::features::PollStage::BeforeDroppedPsds,
        AppState::poll_dropin,
    )),
    ..crate::features::Feature::new("dropin")
};

/// 入れ先の文書へ入れる 1 つのファイル。
enum Item<'a> {
    Psd(&'a Document),
    Image { image: ImageId, name: String },
}

impl AppState {
    pub fn dropin_apply(&mut self, op: DropOp) {
        match op {
            DropOp::Open(paths) => self.dropin_open(paths),
            DropOp::SetDest(id, dest) => {
                if self.dropin_problem(id, dest).is_none() {
                    if let Some(row) = self.dropin.row_mut(id) {
                        if row.is_readable() {
                            row.dest = Some(dest);
                        }
                    }
                }
                self.dropin.menu = None;
            }
            DropOp::SetAll(dest) => {
                let ids: Vec<u64> = self
                    .dropin
                    .rows
                    .iter()
                    .filter(|r| r.is_readable())
                    .map(|r| r.id)
                    .collect();
                for id in ids {
                    if self.dropin_problem(id, dest).is_none() {
                        if let Some(row) = self.dropin.row_mut(id) {
                            row.dest = Some(dest);
                        }
                    }
                }
                self.dropin.all = Some(dest);
                self.dropin.menu = None;
            }
            DropOp::Menu(menu) => self.dropin.menu = menu,
            DropOp::ToggleCheck(id) => {
                if let Some(row) = self.dropin.row_mut(id) {
                    row.open = !row.open;
                }
            }
            DropOp::Remove(id) => {
                self.dropin.rows.retain(|r| r.id != id);
                if self.dropin.menu == Some(MenuFor::Row(id)) {
                    self.dropin.menu = None;
                }
                if self.dropin.rows.is_empty() {
                    self.dropin.close();
                }
            }
            DropOp::Cancel => {
                if self.dropin.is_open() {
                    self.dropin.close();
                    self.info(Source::Layer, self.lang.t("dropin-import-canceled"));
                }
            }
            DropOp::Import => self.dropin_import(),
        }
    }

    /// 落としたファイルを行にし、読み始める。描いている最中は断る（何も始めない）。
    fn dropin_open(&mut self, paths: Vec<PathBuf>) {
        let lang = self.lang;
        if self.is_stroking() {
            self.refuse(Source::Layer, crate::lang::refusals::during_stroke(lang));
            return;
        }
        let mut reads = Vec::new();
        for path in paths {
            // 同じファイルは 1 行
            if self.dropin.rows.iter().any(|r| r.path == path) {
                continue;
            }
            self.dropin.next_id += 1;
            let id = self.dropin.next_id;
            let content = match kind_of(&path) {
                Kind::Psd | Kind::Image(_) => {
                    reads.push((id, path.clone()));
                    Content::Reading
                }
                Kind::OtherImage | Kind::Other => Content::Refused {
                    format: format_name(&path),
                    reason: lang.t("dropin-unsupported-format").into(),
                },
            };
            self.dropin.rows.push(Row {
                id,
                name: file_name(&path),
                path,
                content,
                dest: None,
                open: false,
            });
        }
        if reads.is_empty() {
            return;
        }
        let budget = self.load_source_bytes();
        let spawned = Worker::spawn("yolu-dropin-read", move |tx, cancel| {
            for (id, path) in reads {
                if cancel.is_set() {
                    return;
                }
                let content = read_file(&path, lang, budget, cancel.flag());
                if tx.send((id, content)).is_err() {
                    return;
                }
            }
        });
        match spawned {
            Ok(worker) => self.dropin.readers.push(worker.cancel_on_drop()),
            Err(e) => {
                let reason = lang.thread_error(&e);
                for row in &mut self.dropin.rows {
                    if matches!(row.content, Content::Reading) {
                        row.content = Content::Refused {
                            format: format_name(&row.path),
                            reason: reason.clone(),
                        };
                    }
                }
            }
        }
    }

    /// 読み終わった行を受ける（フレームの初めに）。
    pub fn poll_dropin(&mut self) {
        let mut done = Vec::new();
        let mut finished = Vec::new();
        for (i, worker) in self.dropin.readers.iter().enumerate() {
            loop {
                match worker.poll() {
                    Polled::Message(read) => done.push(read),
                    Polled::Empty => break,
                    Polled::Lost => {
                        finished.push(i);
                        break;
                    }
                }
            }
        }
        for i in finished.into_iter().rev() {
            self.dropin.readers.remove(i);
        }
        for (id, content) in done {
            if let Some(row) = self.dropin.row_mut(id) {
                row.content = content;
            }
        }
        // 止まったスレッドが読み残した行は、読めない行にする
        if self.dropin.readers.is_empty() {
            let lang = self.lang;
            for row in &mut self.dropin.rows {
                if matches!(row.content, Content::Reading) {
                    row.content = Content::Refused {
                        format: format_name(&row.path),
                        reason: lang.t("dropin-not-read").into(),
                    };
                }
            }
        }
    }

    /// テスト用: 読み終わるまで待って受ける（待ちの上限は 120 秒）。
    #[doc(hidden)]
    pub fn wait_dropin(&mut self) {
        crate::jobs::wait_until_idle(
            self,
            "ファイルの読み込みが終わらない（ハング検出上限）",
            |s| s.dropin.is_reading(),
            Self::poll_dropin,
        );
    }

    /// 行をその入れ先へ入れられない短い理由（入れられるなら None）。一覧の行を押せなくする・「すべて」で当てない・取り込む前に確かめる。
    pub fn dropin_problem(&self, id: u64, dest: Dest) -> Option<String> {
        let lang = self.lang;
        let row = self.dropin.row(id)?;
        match dest {
            Dest::Asset => match &row.content {
                Content::Image(_) => None,
                _ => Some(lang.t("dropin-no-psd").into()),
            },
            Dest::NewSet(uid) | Dest::Set(uid, _) => {
                let Some(index) = self.session.sets.index_of(uid) else {
                    return Some(lang.t("sets-texture-set-not-found").into());
                };
                if self.session.sets.get(index)?.read_only.is_some() {
                    return Some(lang.t("dropin-read-only").into());
                }
                let doc = self.set_doc(index);
                if let Dest::Set(_, set) = dest {
                    if !doc.layer_sets().iter().any(|s| s.id == set) {
                        return Some(lang.core_error(&yolu_core::CoreError::InvalidArgument(
                            yolu_core::Reason::LayerSetDoesNotExist,
                        )));
                    }
                }
                if let Content::Psd(psd) = &row.content {
                    if psd.size() != (doc.width(), doc.height()) {
                        return Some(lang.t("dropin-size-differs").into());
                    }
                }
                None
            }
        }
    }

    /// 「取り込む」: 画像をアセットへ入れ、入れ先の文書ごとに 1 回の Undo でレイヤーを入れ、最後の入れ先のテクスチャセット・レイヤーセットへ
    /// 切り替える。前の確かめ（描いている最中・アセットの保存・読むだけのセット・PSD の大きさ）で断るときは何も変えない。
    /// 文書の側（レイヤーの数・入れ子・予算・チャンネルの対応）が入れ先の途中で断ったときは、入れ終えた入れ先は残り（入れ先ごとに 1 回の Undo で
    /// 戻せる）、入れられなかった入れ先の行は一覧に残してウィンドウを開いたままにする。
    fn dropin_import(&mut self) {
        if !self.dropin_may_import() {
            return;
        }
        let Some(resources) = self.dropin_add_assets() else {
            return;
        };
        let Some(images) = self.dropin_use_images(&resources) else {
            return;
        };
        let rows = std::mem::take(&mut self.dropin.rows);
        let outcome = self.dropin_import_sets(&rows, &images);
        self.dropin_finish(rows, &images, outcome);
    }

    /// 取り込みを始めてよいか（始めてよくないなら知らせて false）。
    fn dropin_may_import(&mut self) -> bool {
        let lang = self.lang;
        if !self.dropin.ready() {
            return false;
        }
        if self.is_stroking() {
            self.refuse(Source::Layer, crate::lang::refusals::during_stroke(lang));
            return false;
        }
        let has_images = self
            .dropin
            .rows
            .iter()
            .any(|r| matches!(r.content, Content::Image(_)));
        if has_images && (self.shelf_refuse_while_saving() || !self.shelf_writable()) {
            return false;
        }
        // 入れ先の確かめ（読むだけのセット・大きさ）。断るなら何も変えない
        for row in self.dropin.rows.iter().filter(|r| r.is_readable()) {
            let dest = row.dest.expect("ready が確かめた");
            if let Some(why) = self.dropin_problem(row.id, dest) {
                let text = lang.with_reason(
                    lang.f("dropin-cannot-import", &[("name", &lang.quote(&row.name))]),
                    why,
                );
                self.refuse(Source::Layer, text);
                return false;
            }
        }
        true
    }

    /// 画像の行をアセットへ入れる（同じ中身ならその画像）。行の番号とアセットの ID の組。断られたら知らせて None。
    fn dropin_add_assets(&mut self) -> Option<Vec<(u64, String)>> {
        let lang = self.lang;
        let mut resources: Vec<(u64, String)> = Vec::new();
        for row in &self.dropin.rows {
            let Content::Image(img) = &row.content else {
                continue;
            };
            let before = self.shelf.resources().len();
            let name = stem_name(&row.path, "image");
            match self
                .shelf
                .add_image(lang, &name, &img.rgba, img.width, img.height)
            {
                Ok(id) => {
                    if self.shelf.resources().len() > before {
                        self.session.mark_modified();
                    }
                    resources.push((row.id, id));
                }
                Err(why) => {
                    let text = lang.with_reason(
                        lang.f("dropin-cannot-import", &[("name", &lang.quote(&row.name))]),
                        why,
                    );
                    self.fail(Source::Assets, text);
                    return None;
                }
            }
        }
        Some(resources)
    }

    /// レイヤーにする画像を効果の入力にする（全部のセットの文書へ渡る）。断られたら、それまでの分を手放して知らせて None。
    fn dropin_use_images(&mut self, resources: &[(u64, String)]) -> Option<Vec<(u64, ImageId)>> {
        let lang = self.lang;
        let mut images: Vec<(u64, ImageId)> = Vec::new();
        let wanted: Vec<(u64, String, String)> = self
            .dropin
            .rows
            .iter()
            .filter(|r| matches!(r.content, Content::Image(_)) && r.dest != Some(Dest::Asset))
            .map(|r| {
                let rid = resources
                    .iter()
                    .find(|(id, _)| *id == r.id)
                    .expect("足した");
                (r.id, r.name.clone(), rid.1.clone())
            })
            .collect();
        for (row_id, row_name, rid) in wanted {
            match self.use_shelf_image(&rid) {
                Ok(image) => images.push((row_id, image)),
                Err(why) => {
                    for (_, image) in images {
                        self.release_shelf_image(image);
                    }
                    let text = lang.with_reason(
                        lang.f("dropin-cannot-import", &[("name", &lang.quote(&row_name))]),
                        why,
                    );
                    self.fail(Source::Layer, text);
                    return None;
                }
            }
        }
        Some(images)
    }

    /// 入れ先のテクスチャセットごとに（初めに出てきた順）、そこへ入れる行をまとめて 1 回の Undo で入れる。断られた入れ先で止める。
    fn dropin_import_sets(&mut self, rows: &[Row], images: &[(u64, ImageId)]) -> SetsOutcome {
        let lang = self.lang;
        let paint = self.m2.paint_channel;
        let mut order: Vec<u32> = Vec::new();
        for row in rows {
            if let Some(uid) = row.dest.and_then(Dest::texture_set) {
                if !order.contains(&uid) {
                    order.push(uid);
                }
            }
        }
        let mut outcome = SetsOutcome::default();
        for uid in order {
            let index = self.session.sets.index_of(uid).expect("確かめた");
            let items = items_for(rows, uid, images);
            let doc = self.set_doc_mut(index);
            doc.end_coalescing();
            let result = doc.batch_as(HistoryKind::AddLayer, |d| put_items(lang, d, &items, paint));
            match result {
                Ok(set) => {
                    self.session.mark_modified();
                    outcome
                        .used
                        .extend(items.iter().filter_map(|(_, _, item)| match item {
                            Item::Image { image, .. } => Some(*image),
                            Item::Psd(_) => None,
                        }));
                    if let Some(set) = set {
                        outcome.last = Some((index, set));
                    }
                    outcome.done.push(uid);
                }
                Err(e) => {
                    let name = self
                        .session
                        .sets
                        .get(index)
                        .map(|s| s.name.clone())
                        .unwrap_or_default();
                    outcome.failed = Some(lang.with_reason(
                        lang.f("dropin-cannot-import-into", &[("name", &lang.quote(&name))]),
                        lang.core_error(&e),
                    ));
                    break;
                }
            }
        }
        outcome
    }

    /// 後始末と切り替え: 使わなかった画像を手放し、入れた PSD を覚え、入れ終えた行を一覧から外す（断られた入れ先の行は残してウィンドウを
    /// 開いたままにする）。最後の入れ先のテクスチャセット・レイヤーセットへ切り替えて知らせる。
    fn dropin_finish(&mut self, rows: Vec<Row>, images: &[(u64, ImageId)], outcome: SetsOutcome) {
        let lang = self.lang;
        // どのレイヤーも指さなかった画像は手放す（残した行は、もう一度取り込むときに取り直す）
        for (_, image) in images {
            if !outcome.used.contains(image) {
                self.release_shelf_image(*image);
            }
        }
        // 入れ終えた行（アセットだけの行と、入れ終えたテクスチャセットの行）と、読めない行
        let imported = |r: &Row| {
            r.is_readable()
                && r.dest
                    .and_then(Dest::texture_set)
                    .is_none_or(|uid| outcome.done.contains(&uid))
        };
        for row in rows.iter().filter(|r| imported(r)) {
            if matches!(row.content, Content::Psd(_)) {
                self.psd.remember_imported(row.path.clone());
            }
        }
        let count = rows.iter().filter(|r| imported(r)).count();
        if outcome.failed.is_some() {
            self.dropin.rows = rows
                .into_iter()
                .filter(|r| (r.is_readable() && !imported(r)) || r.is_refused())
                .collect();
            self.dropin.menu = None;
        } else {
            self.dropin.close();
        }
        if let Some((index, set)) = outcome.last {
            if index != self.session.sets.current_index() {
                let _ = self.switch_set(index);
            }
            if index == self.session.sets.current_index() {
                self.select_layer_set(set);
                self.select_top_of_layer_set(set);
                self.set_edit_mask(false);
            }
        }
        match outcome.failed {
            Some(text) => self.fail(Source::Layer, text),
            None => self.info(
                Source::Layer,
                lang.f("dropin-imported-files", &[("count", &count)]),
            ),
        }
    }
}

/// `dropin_import_sets` の結果。
#[derive(Default)]
struct SetsOutcome {
    /// 最後に入れたテクスチャセットの番号とレイヤーセット。
    last: Option<(usize, LayerSetId)>,
    /// 入れ終えたテクスチャセットの uid。
    done: Vec<u32>,
    /// レイヤーに使った画像。
    used: Vec<ImageId>,
    /// 断られた入れ先の知らせ。
    failed: Option<String>,
}

/// テクスチャセット `uid` へ入れる行（入れ先・名前のもと・中身）。
fn items_for<'a>(
    rows: &'a [Row],
    uid: u32,
    images: &[(u64, ImageId)],
) -> Vec<(Dest, String, Item<'a>)> {
    rows.iter()
        .filter(|r| r.dest.and_then(Dest::texture_set) == Some(uid))
        .filter_map(|r| {
            let item = match &r.content {
                Content::Psd(psd) => Item::Psd(&psd.doc),
                Content::Image(_) => Item::Image {
                    image: images.iter().find(|(id, _)| *id == r.id)?.1,
                    name: stem_name(&r.path, "image"),
                },
                _ => return None,
            };
            Some((r.dest?, stem_name(&r.path, "PSD"), item))
        })
        .collect()
}

/// 1 つの文書へ、入れ先ごとにレイヤーセットを作る・選んで中身を入れる。最後に入れたレイヤーセットを返す。
fn put_items(
    lang: Lang,
    d: &mut Document,
    items: &[(Dest, String, Item)],
    paint: Channel,
) -> Result<Option<LayerSetId>, yolu_core::CoreError> {
    let mut last_set = None;
    for (dest, base, item) in items {
        let set = match dest {
            Dest::NewSet(_) => {
                let name = unique_layer_set_name_in(lang, d, base);
                match item {
                    Item::Psd(source) => d.insert_layers_as_layer_set(source, &name)?.0,
                    Item::Image { .. } => d.add_layer_set(&name)?,
                }
            }
            Dest::Set(_, set) => {
                if let Item::Psd(source) = item {
                    d.insert_layers_from(source, *set)?;
                }
                *set
            }
            Dest::Asset => continue,
        };
        if let Item::Image { image, name } = item {
            add_image_fill(d, set, name, *image, paint)?;
        }
        last_set = Some(set);
    }
    Ok(last_set)
}

/// 文書の中のほかのレイヤーセットと名前が重ならない（大文字小文字を区別しない）ように、要れば「 2」「 3」… を付けた名前。
fn unique_layer_set_name_in(lang: Lang, doc: &Document, base: &str) -> String {
    let names: Vec<String> = (0..doc.layer_sets().len())
        .map(|i| crate::m2::layersets::display_name(lang, doc, i))
        .collect();
    crate::sets::unique_name(base, names.iter().map(String::as_str))
}

/// レイヤーセット `set` の一番上に、画像の塗りつぶしレイヤー（メニューの「画像の塗りつぶしレイヤー」と同じ: UV の投影、画像を使えない所は
/// チャンネルの既定の値）を足す。チャンネルは描くチャンネル（文書に無ければ Color）。
fn add_image_fill(
    d: &mut Document,
    set: LayerSetId,
    name: &str,
    image: ImageId,
    paint: Channel,
) -> Result<LayerId, yolu_core::CoreError> {
    let channel = if d.channel_info(paint).is_some() {
        paint
    } else {
        Channel::Color
    };
    let kind = d
        .channel_info(channel)
        .map_or(ChannelKind::Color, |c| c.kind);
    let value = crate::fillfx::default_fallback(kind);
    let above = d.children_in_set(set, None)?.last().copied();
    let id = d.add_fill_layer(name, &[(channel, value)], above)?;
    if above.is_none() {
        d.move_layers_to_set(&[id], set)?;
    }
    d.set_fill_image(id, channel, Some(image))?;
    d.set_fill_projection(
        id,
        crate::fillfx::placement::new_projection(ProjectionMode::Uv, None),
        false,
    )?;
    Ok(id)
}
