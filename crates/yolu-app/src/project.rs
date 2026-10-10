//! .ylp（Unity 版と同じ作業ファイル）の開く・保存・新規。読み書きと検証は yolu-io、ここは画面の状態（テクスチャセット）との受け渡しだけ。
//!
//! - 開く: yolu-io の `SaveTarget::open_within`（ZIP・manifest・正本を流して検証し、保存で外からの書き換えを見張る印を取る。上限は設定の
//!   「レイヤーのメモリ」の予算から）で読み、セットごとに正本（`SetDocument`）をレイヤーごとに流して core の文書へ変える（`to_core`。透明の画素の
//!   RGB・文書とレイヤーの ID を保つ。ファイル全体・正本全体をメモリに組まない）。core で扱えない中身
//!   （調整の種類が使わない値が既定でないものなど。`core_issues`）のあるセットは**読むだけ**にして理由を出す（黙って捨てない）。手動の ID の色は文書へ戻る。
//!   グループ・マスク・塗りつぶし・調整・クリッピング・チャンネルごとの合成・ユーザーチャンネルと、効果（フィルター・Generator・Anchor・
//!   塗りつぶしの画像・グラデーション・パス）は core が持つので、描けて保存で保たれる。
//!   読むだけのセットは、保存した合成の PNG（`composite/Color.png`）を 1 枚のレイヤーにして見せる（描けない。Live Link でも Unity に
//!   見せる）。
//! - 保存: 形式 7 で書く（開いたのが古い形式なら yolu-io の `upgraded` で上げてから）。開いた後に描いた・変えたセットだけ core の文書の
//!   写しを正本の元にし（`DocumentSource::from_core`。書くときにレイヤーごとに流して作り、大きければ版 26 で分ける。Color の合成の PNG も書く）、
//!   描いていないセット・読むだけのセット・知らないエントリは開いた時のバイト列のまま（ファイルから流して写す）残す。保存した後は、書いた
//!   ファイルを指すプロジェクト（`SaveReport::project`）を次の保存・書き置きの元にする。セットの並び・名前・マテリアルの鍵・今のセットは `with_sets`、ファイルが無かったプロジェクトは `create`。書くのは
//!   yolu-io の安全な保存（検証した一時ファイルから 1 回の置き換え。上書きなら前の版は `<名前>-backups~/` に、設定の「退避を残す数」
//!   （既定はすべて）だけ残す。開いた後に外で書き換えられていたら断る）。PSD を「今のセットへ」取り込み直して文書を替えたセットは、古い
//!   PSD の原本（Unity 版が持つ `imported-original.psd`）を持ち越さない（新しい文書の原本ではない。前の版は退避に残る）。
//!   画面のスレッドは頼みの組み立て（文書の写し）と結果を受けるところまでで、重い所は裏のスレッド（`save`・`capture`）。
//!   配布用に作品の写しを書く「配布用に保存」は `distribute`（材料と組み立ては `capture` を共有する）。

use std::path::{Path, PathBuf};

use yolu_core::mesh_maps::MeshMapKind;
use yolu_io::{Project, SaveTarget, SetDocument, WriterInfo};

pub(crate) mod capture;
pub(crate) mod save;
pub use save::{
    save_for_ops, save_from, SaveHold, SaveOutcome, SaveProgress, SaveState, SavedFacts,
};

/// 1 枚のメッシュマップの読み込みの上限（予算。壊れた・大きすぎるものは読まずに知らせる）。
const MESH_MAP_LIMIT_BYTES: usize = 512 * 1024 * 1024;

use crate::engine::{Channel, Document, TileCoord};
use crate::lang::Lang;
use crate::notice::{Kind as NoticeKind, Source};
use crate::sets::TextureSets;
use crate::shelf::ShelfState;
use crate::state::{blank_document_in, AppState, DEFAULT_DOCUMENT_SIZE};

/// 開いた・保存した .ylp。
pub struct ProjectFile {
    /// ファイルの場所。復旧から開いたもの（まだファイルが無い）は空。
    path: PathBuf,
    /// 開いた・保存した時の印（保存で、外から書き換えられていないかを見る）。復旧から開いたものは無い。
    target: Option<SaveTarget>,
    /// 開いた・保存した時の中身（次の保存で、描いていないセット・読むだけのセット・知らないエントリをそのまま残す）。`Arc` なのは、
    /// 復旧の書き置きが主のスレッドで毎回 `Project` を複製せず、共有して別のスレッドへ渡すため。
    original: std::sync::Arc<Project>,
}

impl std::fmt::Debug for ProjectFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectFile")
            .field("path", &self.path)
            .field("format", &self.original.info().format)
            .field("sets", &self.original.sets().len())
            .finish()
    }
}

impl ProjectFile {
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// ファイルがあるか（復旧から開いたものは、保存先を利用者が選ぶまで無い。元の .ylp には書かない）。
    pub fn is_file(&self) -> bool {
        self.target.is_some()
    }
    /// ファイルの .ylp の形式（開いた古い形式は、保存するまでそのまま）。
    pub fn format(&self) -> i32 {
        self.original.info().format
    }
    /// 開いた時の yolu-io の知らせ（移行したこと・知らないエントリなど）。
    pub fn notes(&self) -> &[yolu_io::Note] {
        self.original.notes()
    }
    /// 開いた・保存した時の中身。
    pub fn project(&self) -> &Project {
        &self.original
    }
    /// 開いた・保存した時の中身の共有（複製しない）。
    pub fn project_shared(&self) -> std::sync::Arc<Project> {
        self.original.clone()
    }
}

/// .ylp の ylp.json に書く書き手（Unity の版の欄はスタンドアロンなので "standalone"）。名前は Unity 版と同じ "YoluPainter"
/// （読み手は名前を見ない。Unity 版は 1〜256 文字の文字列として読み、版の新しさを断るときの文に書き手として出すだけ。
/// Unity 版かスタンドアロン版かは Unity の版の欄で分かる）。以前の版は内部の名前 "YoluPainter-rs" を書いていたが、読み手はどちらも受ける。
pub fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        unity: "standalone".into(),
    }
}

/// 正本を core の文書へ。扱えない中身があれば、その理由（多ければ初めの 3 つと数）。`source_budget` は、この文書のレイヤーの画素に
/// 許すバイト数（超えれば、読むだけのセットにして理由を出す）。
pub(crate) fn to_core(
    native: &SetDocument,
    lang: Lang,
    source_budget: u64,
) -> Result<Document, String> {
    let issues = native.core_issues();
    if !issues.is_empty() {
        return Err(lang.unsupported_features(&issues));
    }
    // 効果の入力（焼いたメッシュマップ・モデルのルート・画像）は開いたあとに文書へ渡す（`fx::inputs`）。入力がそろわない効果を持つセットは、
    // そのとき読むだけにして足りない入力を言う（`AppState::lock_sets_missing_inputs`）。入力がそろえば編集できる
    // 理由だけを返す（呼ぶ側が「読むだけです」「編集できません」と何がを言う）
    native
        .to_core_within(Some(source_budget))
        .map_err(|e| lang.io_error(&e))
}

/// `to_core` の、途中の panic を受け止める形。止まったセットは理由の文にして続ける（ほかのセットは開く）。受け止めた panic は
/// 落ちた記録にせず、普段のログへ 1 行だけ書く。
pub(crate) fn to_core_caught(
    native: &SetDocument,
    lang: Lang,
    source_budget: u64,
) -> Result<Document, String> {
    caught_as_text(
        lang,
        std::panic::AssertUnwindSafe(|| to_core(native, lang, source_budget)),
    )
}

/// 仕事の途中の panic を、理由の文の失敗にする（受け止めた panic は落ちた記録にしない）。
pub(crate) fn caught_as_text<T>(
    lang: Lang,
    work: impl FnOnce() -> Result<T, String> + std::panic::UnwindSafe,
) -> Result<T, String> {
    crate::crash::handled(work).unwrap_or_else(|_| Err(reading_stopped(lang)))
}

fn reading_stopped(lang: Lang) -> String {
    lang.pick(
        "プロジェクトを読む途中で止まりました",
        "Reading the project stopped",
    )
    .into()
}

/// 保存した合成の PNG から、見せるだけの文書（1 枚のレイヤー）を作る。大きさが正本と違えば使わない。
pub(crate) fn preview_document(
    png: Option<&[u8]>,
    width: u32,
    height: u32,
    lang: Lang,
) -> (Document, Option<String>) {
    let blank = || {
        let mut doc = Document::new(width, height).expect("正本の大きさは検証済み");
        let _ = doc.add_layer(lang.pick("保存した合成（読むだけ）", "Saved composite (read-only)"));
        let _ = doc.clear_history();
        doc
    };
    let Some(png) = png else {
        return (
            blank(),
            Some(
                lang.pick("保存した合成の絵なし", "No saved composite")
                    .into(),
            ),
        );
    };
    let image = match image::load_from_memory_with_format(png, image::ImageFormat::Png) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            return (
                blank(),
                Some(lang.pick(
                    format!("保存した合成の絵を読めません（{e}）"),
                    format!("Invalid saved composite ({e})"),
                )),
            )
        }
    };
    if image.width() != width || image.height() != height {
        return (
            blank(),
            Some(lang.pick(
                format!(
                    "保存した合成の絵の大きさ {}×{} がキャンバスの {width}×{height} と違う",
                    image.width(),
                    image.height()
                ),
                format!(
                    "Saved composite size {}×{} differs from canvas {width}×{height}",
                    image.width(),
                    image.height()
                ),
            )),
        );
    }
    let mut doc = Document::new(width, height).expect("正本の大きさは検証済み");
    let layer = doc
        .add_layer(lang.pick("保存した合成（読むだけ）", "Saved composite (read-only)"))
        .expect("空の文書にレイヤーを足せる");
    let ts = doc.tile_size();
    let mut tile = vec![0u8; (ts * ts * 4) as usize];
    let raw = image.as_raw();
    for ty in 0..height.div_ceil(ts) {
        for tx in 0..width.div_ceil(ts) {
            tile.fill(0);
            let mut any = false;
            for r in 0..ts {
                let y = ty * ts + r; // 文書の行（下から）
                if y >= height {
                    break;
                }
                let png_row = (height - 1 - y) as usize; // PNG は上の行から
                let x0 = tx * ts;
                let n = ts.min(width - x0) as usize;
                let src = &raw[(png_row * width as usize + x0 as usize) * 4..][..n * 4];
                tile[(r * ts) as usize * 4..][..n * 4].copy_from_slice(src);
                any |= src.iter().any(|&b| b != 0);
            }
            if any {
                let _ = doc.import_tile(layer, Channel::Color, TileCoord::new(tx, ty), &tile);
            }
        }
    }
    let _ = doc.clear_history();
    (doc, None)
}

/// .ylp を開いて今の状態を置き換える。開けなければ何も変えずに理由を出す。レイヤーの画素は、設定の予算（256 MiB を下回らない）まで読む。
pub fn open_into(state: &mut AppState, path: &Path) {
    let budget = state.load_source_bytes();
    open_within(state, path, budget);
}

/// `open_into` の、1 つのテクスチャセットのレイヤーの画素に許すバイト数を指定する形。超えるセットは読むだけにして、理由（予算）を出す。
pub(crate) fn open_within(state: &mut AppState, path: &Path, budget: u64) {
    if state.is_saving() {
        let lang = state.lang;
        state.refuse(
            Source::Open,
            lang.with_reason(cannot_open(lang, path), crate::lang::refusals::saving(lang)),
        );
        return;
    }
    // ファイルは流して読む（全エントリを確かめ、正本の画素はメモリに読まない）。上限は「レイヤーのメモリ」の予算から（`Limits`）
    let limits = yolu_io::Limits::from_layer_pixels(budget);
    let (project, target) = match SaveTarget::open_within(path, &limits) {
        Ok(x) => x,
        Err(e) => {
            let lang = state.lang;
            state.fail(
                Source::Open,
                lang.with_reason(cannot_open(lang, path), lang.io_error(&e)),
            );
            return;
        }
    };
    // 退避（前の版。`<名前>.ylp-backups~` の中）は、保存先を持たない文書として開く（開いた退避を書き換えない。保存の頼みは名前を付けて保存になる）
    let opened = if yolu_io::backup_origin(path).is_some() {
        drop(target);
        Opened::Backup(path.to_path_buf())
    } else {
        Opened::File(path.to_path_buf(), target)
    };
    open_project(state, project, opened, budget);
}

/// 開いた中身の出どころ。
enum Opened {
    /// .ylp のファイル（保存先と、開いた時の印つき）。
    File(PathBuf, SaveTarget),
    /// 退避のフォルダーの中の .ylp（前の版）。保存先を持たず、モデルのファイルの参照は元の .ylp のフォルダーから解く。
    Backup(PathBuf),
    /// 復旧の世代（まだファイルが無い）。
    Recovered,
}

/// 「「ファイル」を開けません」（理由は `Lang::with_reason` で添える）。
fn cannot_open(lang: Lang, path: &Path) -> String {
    let file = lang.quote(&path.display().to_string());
    lang.pick(format!("{file}を開けません"), format!("Cannot open {file}"))
}

/// 復旧の世代から読んだプロジェクトで今の状態を置き換える。保存していない「名称未設定（復旧）」として開き、元の .ylp には
/// つながない（保存先は利用者が選ぶ）。どのセットも、次の保存で正本と合成の PNG を書き直す。
pub fn open_recovered(state: &mut AppState, project: Project) {
    if state.is_saving() {
        state.refuse(
            Source::Recovery,
            state.lang.with_reason(
                state
                    .lang
                    .pick("復旧を開けません", "Cannot open the recovery"),
                crate::lang::refusals::saving(state.lang),
            ),
        );
        return;
    }
    let budget = state.load_source_bytes();
    open_project(state, project, Opened::Recovered, budget);
}

/// 開いた中身（ファイルなら保存先と印つき）で今の状態を置き換える。
fn open_project(state: &mut AppState, project: Project, file: Opened, budget: u64) {
    let entries = project.migrated_entries();
    let mut parts = Vec::with_capacity(project.sets().len());
    let mut read_only = Vec::new();
    let mut selection_issues = Vec::new();
    // セットごとの正本を、別々のスレッドで流して core の文書にする（セットは互いに独立。持つのはスレッドごとにレイヤー 1 枚ぶん）
    let lang = state.lang;
    let converted: Vec<Result<Document, String>> = std::thread::scope(|scope| {
        let running: Vec<_> = project
            .sets()
            .iter()
            .map(|set| scope.spawn(move || to_core_caught(&set.document, lang, budget)))
            .collect();
        running
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| Err(reading_stopped(lang))))
            .collect()
    });
    for (set, converted) in project.sets().iter().zip(converted) {
        let native = &set.document;
        let (w, h) = (native.width() as u32, native.height() as u32);
        match converted {
            Ok(mut doc) => {
                // 選択範囲（selection.bin）は文書に戻す（読めなければ選択なしで開き、理由を出す）
                if let Err(e) =
                    crate::selection::io::restore_into(&mut doc, set.selection.as_ref(), state.lang)
                {
                    selection_issues.push(lang.in_set(&set.name, &e));
                }
                // 見た目の設定（look.json）。読めなければ標準で開いて理由を言う（ファイルには残る）
                if let Err(e) =
                    crate::look::io::restore_into(&mut doc, &project, &set.id, state.lang)
                {
                    selection_issues.push(lang.in_set(&set.name, &e));
                }
                // 名前を付けて残した選択範囲（selections.json）。読めない項目は飛ばして理由を言う（ファイルには残る）
                let saved = project
                    .saved_selections(&set.id)
                    .map_err(|e| state.lang.io_error(&e));
                if let Err(e) = saved.and_then(|read| {
                    crate::selection::io::restore_saved_into(&mut doc, read, state.lang)
                }) {
                    selection_issues.push(lang.in_set(&set.name, &e));
                }
                parts.push((
                    set.id.clone(),
                    set.name.clone(),
                    set.material.clone(),
                    None,
                    doc,
                ))
            }
            Err(reason) => {
                read_only.push(set.name.clone());
                let png = entries
                    .get(&format!("sets/{}/composite/Color.png", set.id))
                    .and_then(|b| b.bytes().ok());
                let (mut doc, note) = preview_document(png.as_deref(), w, h, state.lang);
                // 読むだけのセットも、見た目の設定で 3D に見せる（読めなければ標準のまま、通常のセットと同じ理由を言う）
                let look =
                    crate::look::io::restore_into(&mut doc, &project, &set.id, state.lang).err();
                if let Some(e) = &look {
                    selection_issues.push(lang.in_set(&set.name, e));
                }
                let reason = match note {
                    Some(n) => format!("{reason}。{n}"),
                    None => reason,
                };
                let reason = match look {
                    Some(e) => format!("{reason}。{e}"),
                    None => reason,
                };
                parts.push((
                    set.id.clone(),
                    set.name.clone(),
                    set.material.clone(),
                    Some(reason),
                    doc,
                ));
            }
        }
    }
    // 復旧の世代から開いたプロジェクトの大きなエントリ（読むだけのセットの正本を含む）は、読むときに世代の外へ置き直してある
    // （`recovery::pool::load`）。その世代が後で整理・破棄されても保存できる
    let current = project
        .sets()
        .iter()
        .position(|s| s.id == project.current_set())
        .unwrap_or(0);
    let count = parts.len();
    // 前のプロジェクトのモデル（FBX・試しの人形）は引きずらない（このプロジェクトのモデルは、参照があれば読み直す）
    state.np_project_replaced();
    state.drop_project_model();
    let (mut sets, doc) = TextureSets::from_parts(parts, current);
    // メッシュマップ（セットごとの派生物）。壊れていれば読まずに知らせる（ファイルのエントリはそのまま残る）
    let (mut map_count, mut map_problems) = (0usize, Vec::new());
    let mut adopted_before = false;
    for (i, set) in project.sets().iter().enumerate() {
        let mut loaded_kinds = Vec::new();
        for kind in MeshMapKind::ALL {
            match project.mesh_map(&set.id, kind, MESH_MAP_LIMIT_BYTES) {
                Ok(Some(map)) => {
                    if let Some(target) = sets.get_mut(i) {
                        // 焼く設定は 1 つなので、保存したマップの条件にそろえる（開いただけで古くならない）
                        crate::bake::adopt::adopt(&mut state.bake.settings, &map);
                        loaded_kinds.push(kind);
                        target.mesh_maps.load(map);
                        map_count += 1;
                    }
                }
                Ok(None) => {}
                Err(e) => map_problems.push(lang.with_reason(
                    lang.pick(
                        format!("{}の {} を読めません", lang.quote(&set.name), kind.name()),
                        format!(
                            "Cannot read the {} of {}",
                            kind.name(),
                            lang.quote(&set.name)
                        ),
                    ),
                    lang.io_error(&e),
                )),
            }
        }
        if !loaded_kinds.is_empty() {
            crate::bake::adopt::adopt_kinds(
                &mut state.bake.settings,
                &loaded_kinds,
                adopted_before,
            );
            adopted_before = true;
        }
    }
    state.replace_sets(sets, doc);
    state.shelf = ShelfState::from_project(&project).inherit_running_from(&state.shelf);
    // 次に名前を付けて保存を開くフォルダー（退避は元の .ylp のフォルダー。ほかは前の保存・OS の既定）
    state.save_folder = None;
    let mut text = match &file {
        Opened::File(path, _) => {
            state.project_name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| state.lang.pick("名称未設定", "Untitled").into());
            state.modified = false;
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            state
                .lang
                .pick(format!("開きました: {name}。"), format!("Opened: {name}."))
        }
        Opened::Backup(path) => {
            // 題名と保存の名前の候補は元の .ylp の名前（退避の名前の時刻の部分ではない）。始まりの場所も元の .ylp のフォルダー
            let origin = yolu_io::backup_origin(path);
            let stem = |p: &Path| p.file_stem().map(|s| s.to_string_lossy().into_owned());
            state.project_name = origin
                .as_deref()
                .and_then(stem)
                .or_else(|| stem(path))
                .unwrap_or_else(|| state.lang.pick("名称未設定", "Untitled").into());
            state.save_folder = origin
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf);
            // ファイルのとおりに開いたので、変更は無い
            state.modified = false;
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            state.lang.pick(
                format!("開きました: {name}（退避）。"),
                format!("Opened: {name} (backup)."),
            )
        }
        Opened::Recovered => {
            state.project_name = crate::recovery::recovered_name(state.lang).into();
            // 書き置きの正本から開いた文書は、保存した .ylp とは別物（保存していない）。合成の PNG も無いので、全セットを書き直す
            state.modified = true;
            for i in 0..state.sets.len() {
                if let Some(set) = state.sets.get_mut(i) {
                    set.saved = None;
                }
            }
            state.lang.pick(
                format!("復旧しました（テクスチャセット {count}）。"),
                format!("Recovered ({count} texture sets)."),
            )
        }
    };
    if !read_only.is_empty() {
        text += " ";
        text += &state.lang.with_reason(
            state.lang.pick(
                "読むだけで開いたテクスチャセットがあります",
                "Some texture sets opened read-only",
            ),
            read_only.join(state.lang.pick("・", ", ")),
        );
    }
    if !selection_issues.is_empty() {
        text += &format!(" {}", selection_issues.join(" "));
    }
    if let Some(notice) = unreadable_shelf_notice(state) {
        text += &notice;
    }
    if map_count > 0 {
        text += &state.lang.pick(
            format!(" メッシュマップ {map_count} 枚。"),
            format!(" {map_count} mesh map(s)."),
        );
    }
    if !map_problems.is_empty() {
        let lang = state.lang;
        text += " ";
        text += &map_problems.join(lang.pick("", " "));
        text += lang.pick(
            "メッシュマップはファイルには残っています。",
            " The mesh maps are kept in the file.",
        );
    }
    // io の知らせのうち、セットごとの変換の理由は上で言ったので除く
    let notes: Vec<String> = project
        .notes()
        .iter()
        .filter(|n| !matches!(n, yolu_io::Note::SetNotConvertible { .. }))
        .map(|n| state.lang.project_note(n))
        .collect();
    if !notes.is_empty() {
        text += &state.lang.pick(
            format!(" {}", notes.join(" ")),
            format!(" {}.", notes.join("; ")),
        );
    }
    let view_model = project.view_model();
    let livelink = project.livelink();
    // 開けたが気をつけること（読むだけのセット・選択範囲・棚・メッシュマップ・io の知らせ）があれば注意
    let mut kind = if read_only.is_empty()
        && selection_issues.is_empty()
        && state.shelf.unavailable.is_none()
        && map_problems.is_empty()
        && notes.is_empty()
    {
        NoticeKind::Info
    } else {
        NoticeKind::Warning
    };
    // モデルのファイルの参照を解く .ylp の場所（退避はそのファイル。復旧は無い）と、保存先（ファイルだけが持つ）
    let (file_path, path, target) = match file {
        Opened::File(path, target) => (path.clone(), path, Some(target)),
        Opened::Backup(path) => (path, PathBuf::new(), None),
        Opened::Recovered => (PathBuf::new(), PathBuf::new(), None),
    };
    state.project = Some(ProjectFile {
        path,
        target,
        original: std::sync::Arc::new(project),
    });
    // 効果の入力（焼いたマップ・モデル・棚の画像）を渡し、入力がそろわない効果を持つセットは読むだけにする
    let waiting = state.lock_sets_missing_inputs();
    if !waiting.is_empty() {
        kind = NoticeKind::Warning;
        text += " ";
        text += &state.lang.with_reason(
            state.lang.pick(
                "効果の入力がそろわないので、読むだけのテクスチャセットがあります",
                "Some texture sets are read-only because their effect inputs are missing",
            ),
            waiting.join(state.lang.pick("・", ", ")),
        );
    }
    // モデルのファイルの参照（view.json）があれば、別のスレッドで読み直す（読み終えたら結び付ける）。参照は .ylp からの相対の
    // パスなので、ファイルの無いプロジェクト（復旧した世代）では読み直さない（保存し直した後に開けば読む）
    // Live Link の相手の文書（livelink.json）なら、Unity なしで同じモデルとポーズに開き直す（モデルのファイルの参照は残すだけ）
    let link_note = match livelink.map(|b| b.map(|b| crate::livelink::store::restore(&b))) {
        Ok(Some(Ok(request))) => {
            state.link_reopen = Some(request);
            None
        }
        Ok(Some(Err(_))) | Err(_) => Some(state.lang.pick(
            "Live Link のモデルは自動では開きません（ファイルには残っています）。",
            "The Live Link model is not reopened automatically (kept in the file).",
        )),
        Ok(None) => None,
    };
    if let Some(note) = link_note {
        text += &format!(" {note}");
        kind = NoticeKind::Warning;
    }
    let note = match view_model {
        Ok(Some(stored)) if state.link_reopen.is_some() => {
            state.np.model_file = Some(crate::newproject::reopen::resolve_model_path(
                &stored, &file_path,
            ));
            None
        }
        Ok(Some(_)) if file_path.as_os_str().is_empty() => None,
        Ok(Some(stored)) => {
            crate::newproject::reopen::start(state, &file_path, &stored, &text, kind)
        }
        Ok(None) => None,
        Err(e) => Some(state.lang.pick(
            format!("モデルの参照を読めません（{}）。", state.lang.io_error(&e)),
            format!(
                "Cannot read the model reference ({}).",
                state.lang.io_error(&e)
            ),
        )),
    };
    // モデルの参照の但し書き（読めない参照・ネットワーク上のモデル）は気をつけること
    if let Some(note) = note {
        text += &format!(" {note}");
        kind = NoticeKind::Warning;
    }
    state.notify(kind, Source::Open, text);
}

/// 開いたときの知らせのうち、棚を読めなかった分（読めた棚には None。先頭に空白を置いて、知らせの文へ続ける）。
pub fn unreadable_shelf_notice(state: &AppState) -> Option<String> {
    let reason = state.shelf.unavailable.as_ref()?.reason(state.lang);
    Some(state.lang.pick(
        format!(" プロジェクトのアセットを読めません（{reason}）。"),
        format!(" The project's assets cannot be read ({reason})."),
    ))
}

/// 新しいプロジェクト（空の 2048² のセット 1 つ）にする。Live Link のモデルがあれば、そのマテリアルにセットを付ける。前のプロジェクトの
/// モデル（FBX・試しの人形）は外す。テンプレート・モデル・解像度などを選ぶウィンドウは `newproject`。
pub fn new_into(state: &mut AppState) {
    if state.is_saving() {
        state.refuse(
            Source::Project,
            state.lang.with_reason(
                state.lang.pick(
                    "新しいプロジェクトを作れません",
                    "Cannot create a new project",
                ),
                crate::lang::refusals::saving(state.lang),
            ),
        );
        return;
    }
    state.np_project_replaced();
    state.drop_project_model();
    let (doc, _) = blank_document_in(DEFAULT_DOCUMENT_SIZE, DEFAULT_DOCUMENT_SIZE, state.lang);
    let sets = TextureSets::first_in(&doc, state.lang);
    state.replace_sets(sets, doc);
    state.shelf = ShelfState::default().inherit_running_from(&state.shelf);
    state.project = None;
    state.project_name = state.lang.pick("名称未設定", "Untitled").into();
    state.save_folder = None;
    state.modified = false;
    state.info(
        Source::Project,
        state
            .lang
            .pick("新しいプロジェクトを作りました。", "New project created."),
    );
}

/// 開いた・保存した時のプロジェクト（`base`）から、文書を別の物に替えたセットの ID（セットの今の文書の ID が、`base` の同じセットの正本の
/// 文書 ID と違うもの。PSD を「今のセットへ」取り込み直すと文書は新しい ID になる）。`sets` はセットの ID と今の文書の ID。保存と配布用の
/// 写しが、古い PSD の原本（`imported-original.psd`）を持ち越さないために使う。
pub(crate) fn replaced_sets<'a>(
    base: Option<&Project>,
    sets: impl Iterator<Item = (&'a str, u128)>,
) -> Vec<String> {
    let Some(base) = base else { return Vec::new() };
    sets.filter(|(id, doc)| {
        base.sets()
            .iter()
            .find(|s| s.id == *id)
            .is_some_and(|s| s.document.id() != crate::sets::guid_string(*doc))
    })
    .map(|(id, _)| id.to_owned())
    .collect()
}

pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// 保存した .ylp が、`limits`（今の「レイヤーのメモリ」の予算から）を超えて開き直せないときの短い知らせ。上限は大きな形（`YLP-4`）だけに
/// かかる（今の形は今の上限に収まるときだけ書く）。
pub(crate) fn reopen_note(
    lang: Lang,
    project: &Project,
    limits: &yolu_io::Limits,
) -> Option<String> {
    let archive = project.original_archive();
    if archive.manifest_version() < 4 {
        return None;
    }
    limits
        .check(
            archive
                .entries()
                .iter()
                .map(|(name, blob)| (name.as_str(), blob.len())),
        )
        .err()?;
    Some(
        lang.pick(
            " 今の「レイヤーのメモリ」の予算では開き直せません。",
            " Too large to reopen within the current Layer memory budget.",
        )
        .into(),
    )
}

/// 保存の知らせのうち退避の文。前の版は、退避する設定で上書きしたときだけ残る（退避しない設定・新規の保存では作らない）。
/// 古い退避の整理で消せなかったものは、保存の成功と別に知らせる（保存は済んでいる）。
fn backup_text(lang: Lang, path: &Path, report: &yolu_io::SaveReport) -> String {
    let mut text = String::new();
    if report.backup.is_some() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        text += &lang.pick(
            format!(" 前の版は {name}-backups~ に残しました。"),
            format!(" Previous version: {name}-backups~."),
        );
    }
    if let Some(first) = report.prune_failures.first() {
        let n = report.prune_failures.len();
        let reason = lang.file_error(&first.error);
        text += &lang.pick(
            format!(" 古い退避 {n} 件を消せませんでした（{reason}）。"),
            format!(
                " Could not delete {n} old backup{} ({reason}).",
                if n == 1 { "" } else { "s" }
            ),
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 保存した大きな形（`YLP-4`）の .ylp を今の予算で開き直せないときは、保存の知らせで言う（一様なタイルのレイヤーは core の画素が小さい
    /// まま正本だけが大きくなる）。開き直せるもの・今の形のものには何も言わない。
    #[test]
    fn a_saved_file_the_budget_cannot_reopen_is_told_when_saving() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/project-reopen-tests")
            .join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = AppState::new(128, 128);
        let ts = s.doc.tile_size();
        let flat = [40u8, 80, 120, 255].repeat((ts * ts) as usize);
        for i in 0..8 {
            let id = s.doc.add_layer(&format!("平ら {i}")).unwrap();
            for ty in 0..128 / ts {
                for tx in 0..128 / ts {
                    s.doc
                        .import_tile(id, Channel::Color, TileCoord::new(tx, ty), &flat)
                        .unwrap();
                }
            }
        }
        s.modified = true;
        let small = yolu_io::Thresholds {
            classic_total_bytes: 1 << 10,
            ..yolu_io::Thresholds::REAL
        };
        let path = dir.join("平ら.ylp");
        small.scoped(|| s.apply(crate::state::Action::SaveProjectAs(path.clone())));
        assert!(s.message.starts_with("保存しました"), "{}", s.message);
        let saved = s.project.as_ref().unwrap().project();
        assert_eq!(saved.original_archive().manifest_version(), 4);
        let document: u64 = saved
            .original_archive()
            .entries()
            .iter()
            .filter(|(n, _)| n.contains("document.utpaint"))
            .map(|(_, b)| b.len())
            .sum();
        assert!(document > 4 * 1000 * s.doc.allocated_bytes(), "{document}");
        let tight = yolu_io::Limits {
            document_bytes: document - 1,
            other_bytes: yolu_io::Limits::default().other_bytes,
        };
        let note = reopen_note(Lang::Ja, saved, &tight).expect("開き直せないと言う");
        assert!(
            note.contains("「レイヤーのメモリ」の予算") && !note.contains("MiB"),
            "{note}"
        );
        assert!(reopen_note(Lang::En, saved, &tight)
            .unwrap()
            .contains("Layer memory budget"));
        // 言ったとおり、その上限の読み手は断る
        assert!(matches!(
            yolu_io::Package::open(&path, &tight),
            Err(yolu_io::Error::Budget(_))
        ));
        let enough = yolu_io::Limits {
            document_bytes: document,
            ..tight
        };
        assert_eq!(reopen_note(Lang::Ja, saved, &enough), None);
        yolu_io::Package::open(&path, &enough).unwrap();
        // 今の形（今の上限に収まる）には上限の予算はかからない
        let classic = dir.join("今の形.ylp");
        s.apply(crate::state::Action::SaveProjectAs(classic));
        let saved = s.project.as_ref().unwrap().project();
        assert!(saved.original_archive().manifest_version() <= 3);
        assert_eq!(reopen_note(Lang::Ja, saved, &tight), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_backup_text_names_the_kept_version_and_the_old_ones_that_could_not_be_deleted() {
        use yolu_io::{FileStamp, PruneFailure, SaveReport};
        let report = |backup: bool, failures: usize| SaveReport {
            stamp: FileStamp {
                sha256: String::new(),
                length: 0,
                modified: std::time::UNIX_EPOCH,
            },
            project: None,
            backup: backup.then(|| PathBuf::from("a.ylp-backups~/a-20260101T000000000Z.ylp")),
            prune_failures: (0..failures)
                .map(|i| PruneFailure {
                    path: PathBuf::from(format!("old-{i}.ylp")),
                    error: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
                })
                .collect(),
        };
        let path = Path::new("dir/a.ylp");
        // 退避しない設定・新規の保存は、何も言わない
        assert_eq!(backup_text(Lang::Ja, path, &report(false, 0)), "");
        assert_eq!(backup_text(Lang::En, path, &report(false, 0)), "");
        assert_eq!(
            backup_text(Lang::Ja, path, &report(true, 0)),
            " 前の版は a.ylp-backups~ に残しました。"
        );
        assert_eq!(
            backup_text(Lang::En, path, &report(true, 0)),
            " Previous version: a.ylp-backups~."
        );
        // 整理で消せなかったものは件数と理由（OS のエラーの種類）。保存の成功の文は消さない
        let one = backup_text(Lang::En, path, &report(true, 1));
        assert_eq!(
            one,
            " Previous version: a.ylp-backups~. Could not delete 1 old backup (Access denied)."
        );
        let many = backup_text(Lang::En, path, &report(true, 3));
        assert!(
            many.ends_with("Could not delete 3 old backups (Access denied)."),
            "{many}"
        );
        let ja = backup_text(Lang::Ja, path, &report(true, 2));
        assert!(
            ja.contains("古い退避 2 件を消せませんでした（アクセスが拒否されました）"),
            "{ja}"
        );
        // 退避しない設定のとき整理は走らないが、理由だけがある報告でも文にする
        assert!(backup_text(Lang::En, path, &report(false, 1))
            .starts_with(" Could not delete 1 old backup"));
    }

    #[test]
    fn a_set_is_read_within_the_given_pixel_budget_and_a_refusal_names_the_budget() {
        use yolu_core::Rgba8;
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/project-budget-tests")
            .join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("budget.ylp");
        // 256 × 256 の 4 タイル（128 × 128 × 4 バイト = 64 KiB ずつ）を塗った文書を保存する
        let mut source = AppState::new(256, 256);
        let layer = source.selected_layer.unwrap();
        for (x, y) in [(0, 0), (128, 0), (0, 128), (128, 128)] {
            source
                .doc
                .set_pixel(layer, x, y, Rgba8::new(1, 2, 3, 255))
                .unwrap();
        }
        let bytes = source.doc.allocated_bytes();
        assert_eq!(bytes, 4 * 65536);
        source.apply(crate::state::Action::SaveProjectAs(path.clone()));
        assert!(!source.modified, "{}", source.message);
        // ちょうどの予算なら読める
        let native = yolu_io::NativeDocument::from_core(&source.doc).unwrap();
        assert_eq!(
            to_core(
                &yolu_io::SetDocument::in_memory(native.clone()),
                Lang::Ja,
                bytes
            )
            .unwrap()
            .allocated_bytes(),
            bytes
        );
        // 1 バイト足りなければ、壊れたファイルではなく予算として断る（日英）
        let ja = to_core(
            &yolu_io::SetDocument::in_memory(native.clone()),
            Lang::Ja,
            bytes - 1,
        )
        .err()
        .expect("断る");
        assert!(ja.contains("予算"), "{ja}");
        let en = to_core(
            &yolu_io::SetDocument::in_memory(native.clone()),
            Lang::En,
            bytes - 1,
        )
        .err()
        .expect("断る");
        assert_eq!(en, "Size, count or memory limit exceeded");
        // 開く: 予算に収まれば編集できるセット、収まらなければ読むだけのセット（理由つき）。どちらも元のファイルは変えない
        let mut opened = AppState::new(64, 64);
        open_within(&mut opened, &path, bytes);
        assert!(opened.read_only_reason().is_none(), "{}", opened.message);
        assert_eq!(opened.doc.allocated_bytes(), bytes);
        let mut refused = AppState::new(64, 64);
        open_within(&mut refused, &path, bytes - 1);
        assert!(
            refused
                .read_only_reason()
                .is_some_and(|r| r.contains("予算")),
            "{:?}",
            refused.read_only_reason()
        );
        assert!(
            refused.message.contains("読むだけで開いたテクスチャセット"),
            "{}",
            refused.message
        );
        // 普通の開き方は設定の予算（既定は 256 MiB 以上）で読む
        let mut normal = AppState::new(64, 64);
        open_into(&mut normal, &path);
        assert!(normal.read_only_reason().is_none(), "{}", normal.message);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preview_flips_the_png_rows_to_bottom_up() {
        // 3 × 2: 上の行が赤、下の行が透明（RGB は残す）
        let mut img = image::RgbaImage::new(3, 2);
        for x in 0..3 {
            img.put_pixel(x, 0, image::Rgba([255, 0, 0, 255]));
            img.put_pixel(x, 1, image::Rgba([10, 20, 30, 0]));
        }
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (doc, note) = preview_document(Some(&png), 3, 2, Lang::Ja);
        assert!(note.is_none());
        let px = |x, y| crate::engine::layer_pixel(&doc.layers()[0], x, y);
        assert_eq!(
            px(1, 1),
            [255, 0, 0, 255],
            "PNG の上の行は文書の上（y = 1）"
        );
        assert_eq!(px(1, 0), [10, 20, 30, 0], "透明の画素の RGB も保つ");
        assert!(!doc.can_undo());
        let (_, note) = preview_document(Some(&png), 4, 2, Lang::Ja);
        assert!(note.unwrap().contains("大きさ"));
        let (doc, note) = preview_document(None, 8, 8, Lang::Ja);
        assert!(note.is_some());
        assert_eq!((doc.width(), doc.layers().len()), (8, 1));
    }
}
