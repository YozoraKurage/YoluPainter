//! PSD の読み込みと書き出し（RGB8 の PSD。Unity 版の `ImportPsd`・`ExportPsd` と同じ考え方）。コーデックは yolu-io の `psd`。
//!
//! - **読み込み**（新しいテクスチャセットか、今のセットの文書として）: **写しとしての取り込み**。別のスレッドで、ファイルを流して読み
//!   （`psd::import_copy`。原本のバイト列は持たない）、core の文書にする。グループ（入れ子・通過/分離）・塗りつぶし（単色）・調整・マスク・
//!   クリッピングは core のレイヤーになり、レイヤーのロック（lspf）は core のレイヤーのロックとして入る。合成に効かない情報は持たず、評価できない効果などはレイヤーの
//!   画素のまま取り込み、**無視・落とす・変わるものは、取り込む前に確認のウィンドウへレイヤーの名前と機能の名前で並べる**（0 件ならウィンドウを出さない。「取り込む」を
//!   押すまで何も入れない）。取り込めない（PSB・RGB8 以外・予算を超える・壊れている）ものだけ、何も変えずに理由を結果のウィンドウで見せる。レイヤーの数・
//!   画素の上限は設定の「レイヤーのメモリ」の予算（`load_source_bytes`）から決まる。レイヤー ID は PSD のものを持つが、欠落・重複には新しい ID を振る
//!   （名前では結び付けない）。PSD の原本は書き換えない（取り込んだファイルへ書き出すときは置き換える前に確かめ、書き出しはいつも新しい PSD）。
//!   今のセットの文書を替える読み込みは、確かめを終えて入れるときに描いている最中か、読んでいる間に文書が変わっていれば入れない
//!   （描きかけのストロークを取り残さず、描いたものを黙って捨てない）。
//! - **書き出し**（今の文書）: チャンネルごとに 1 つの PSD を書く（ウィンドウで方式とチャンネルを選ぶ。既定は Color だけを「焼き込んで書く」）。
//!   ラスター・グループ・単色の塗りつぶし・調整・クリッピング・マスク（有効/無効・濃度）・レイヤーのロックは PSD の形で書き、PSD に形の無いもの
//!   （フィルター・Generator・画像・パス・反転したマスク・半透明の塗りつぶし・クリッピングされたグループなど）は、評価した画素にして書く・
//!   刻みへ丸める・落とすのどれかにして、書く前の確認のウィンドウにレイヤーの名前つきで全部並べる（利用者が「書く」を押すまで何も書かない。黙って捨てない）。
//!   文書は 1 バイトも変えない（効果は文書に残る。書き出しは写し）。始めるときに文書の写し（履歴の無い、タイルを共有する写し）を取り、
//!   計画・焼き込み・書き込みは別のスレッドで行う: 計画（何を焼くか）→ 確かめ → レイヤーを 1 枚ずつ評価して RLE で圧縮し、一時ファイルへ流して書く →
//!   一時ファイルを流して読み戻して確かめる（`psd::verify_stream`）→ 最後に置き換え。メモリにはレイヤー 1 枚ぶんだけを持つので、実物の大きさ（4096²・
//!   数十レイヤー）の文書を書ける。キャンバス・レイヤーの記録の数の上限は設定の「レイヤーのメモリ」の予算（`load_source_bytes`。取り込みと同じ）から決まり、
//!   PSD の 2 GiB は圧縮したあとの大きさで書きながら見る。超えたらレイヤーの名前つきの理由（`Overrun`）で断り、一時ファイルは残さない。
//!   取り込んだ PSD と同じファイル・複数のチャンネルで名前が重なるファイルは、置き換える前に確かめる。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use egui::Vec2;
use yolu_core::{Channel, Document};
use yolu_io::psd::{
    self, CopyOptions, CopyOutcome, CopyRefusal, ExportControl, ExportError, ExportMode,
    ExportNote, ExportOptions, ExportPlan, ImportNote, Overrun,
};

use crate::jobs::{JobCard, JobSpec, Polled, Worker};
use crate::lang::Lang;
use crate::notice::{Kind as NoticeKind, Source};
use crate::psd_export::blocker_text;
use crate::sets::{guid_string, unique_name, MaterialRef};
use crate::state::{Action, AppState, DialogRequest};
use crate::windows::CloseJob;

/// 取り込み・書き出しの計画・書き込みのスレッドのスタック。グループの入れ子（文書は `MAX_GROUP_DEPTH` 段まで。取り込みは統合画像と
/// 照らすために合成し、書き出しは計画で再帰する）の再帰に足りる大きさ（合成の実測は `MAX_GROUP_DEPTH` の説明: 64 段は Linux で 320KB。
/// 計画・書き込みの再帰は測っていない。8MiB は標準の 2MiB より大きく取った余裕）。
const PSD_THREAD_STACK: usize = 8 * 1024 * 1024;

/// PSD の仕事のスレッドの作り方（名前と、大きなスタック）。
fn psd_thread(name: &str) -> std::thread::Builder {
    std::thread::Builder::new()
        .name(name.into())
        .stack_size(PSD_THREAD_STACK)
}

/// 読み込んだ PSD の行き先。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsdTarget {
    /// 新しいテクスチャセットとして足す。
    NewSet,
    /// 今のセットの文書を、読み込んだ文書に替える（今の文書は Undo で戻せない）。
    CurrentSet,
}

/// PSD の操作（`Action::Psd`）。
#[derive(Clone, Debug, PartialEq)]
pub enum PsdAction {
    /// 読み込む PSD を選ぶウィンドウを頼む。
    ImportDialog(PsdTarget),
    /// PSD を読み込む（別のスレッド）。
    Import { path: PathBuf, target: PsdTarget },
    /// 書き出しの設定のウィンドウ（方式・チャンネル）を開く。
    ExportDialog,
    /// 設定のウィンドウで方式を選ぶ。
    SetExportMode(ExportMode),
    /// 設定のウィンドウでチャンネルを入れる・外す（最後の 1 つは外せない）。
    ToggleExportChannel(Channel),
    /// 設定のウィンドウの「やめる」。
    CancelExportOptions,
    /// 設定のウィンドウの「書き出し…」: 書き出す先を選ぶウィンドウを頼む（ウィンドウは閉じる）。
    ChooseExportFile,
    /// 今の文書を、選んだ設定で PSD に書き出す（先のファイルが決まった）。複数のチャンネルでは `<名前>_<チャンネル>.psd` を並べて書く。
    Export(PathBuf),
    /// 取り込みの確かめ（無視・落とす・変わるもの）の「取り込む」。
    ConfirmImport,
    /// 取り込みの確かめの「やめる」。
    CancelImport,
    /// 置き換える確かめの「置き換える」。
    ConfirmReplace,
    /// 置き換える確かめの「やめる」。
    CancelConfirm,
    /// 書く前の確かめ（焼く・丸める・落とす）の「書く」。
    ConfirmWrite,
    /// 書く前の確かめの「やめる」。
    CancelWrite,
    /// 読み書きの取消。
    Cancel,
    /// 結果のウィンドウを閉じる。
    DismissReport,
}

/// 結果のウィンドウの 1 行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// 注意（読み込めない理由・見え方が変わる所）か、お知らせか。
    pub warning: bool,
    pub text: String,
    /// 行のツールチップ（説明はここに置く）。
    pub tooltip: Option<String>,
}
impl Line {
    fn new(warning: bool, text: impl Into<String>) -> Self {
        Self {
            warning,
            text: text.into(),
            tooltip: None,
        }
    }
}

/// 読み書きの結果（ウィンドウに出す）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub importing: bool,
    pub file: String,
    /// 読み込めた・書けた。
    pub ok: bool,
    /// ウィンドウの見出しの下の 1 行（名前・状態・短い理由）。
    pub summary: String,
    pub lines: Vec<Line>,
}

enum Output {
    Imported {
        doc: Box<Document>,
        notes: Vec<ImportNote>,
    },
    /// 取り込めない理由（何も変えない）。
    Refused(CopyRefusal),
    /// 計画ができた（書く前の確かめか、すぐ書く）。
    Planned(Box<Run>),
    /// 書いた PSD（ファイルと大きさ）。
    Exported { files: Vec<(PathBuf, usize)> },
}

/// 別のスレッドの仕事が失敗した理由。文は画面の言語で作り（`text`）、取消などの判定は文字列でなく種類で見る。
#[derive(Debug)]
enum Failure {
    /// 利用者の取消。
    Canceled,
    /// 仕事のスレッドが結果を返さずに止まった。
    Stopped,
    /// ファイルの読み書きの失敗。
    File(std::io::Error),
    /// 書き出しの中の失敗（予算・書けない中身など。取消は `Canceled`）。
    Export(yolu_io::Error),
    /// 書いた PSD を読み戻して確かめたが、読めなかった（壊れている・取り込みで落とすものがある・長さが合わない）。
    Unreadable(yolu_io::Error),
    /// 予算・形式の上限（キャンバス・レイヤーの数・画素の合計・ファイルの 2 GiB）で書けない。レイヤーの名前つき。
    Overrun(Overrun),
    /// 置き換えの途中で失敗した。先の `done` 個は置き換わっている。
    PartlyReplaced { done: usize, cause: std::io::Error },
    /// 上のどれでもない理由（日本語・英語）。
    Message { ja: String, en: String },
}

impl Failure {
    fn message(ja: impl Into<String>, en: impl Into<String>) -> Self {
        Self::Message {
            ja: ja.into(),
            en: en.into(),
        }
    }

    /// ウィンドウと状態の帯に出す理由。`importing` は取消のとき「何も書いていません」を付けるかの違いだけ。
    fn text(&self, lang: Lang, importing: bool) -> String {
        match self {
            Self::Canceled if importing => lang.pick("取り消しました", "Cancelled").into(),
            Self::Canceled => lang
                .pick(
                    "取り消しました（何も書いていません）",
                    "Cancelled (nothing was written)",
                )
                .into(),
            Self::Stopped => lang
                .pick("PSD の処理が止まりました", "The PSD job stopped")
                .into(),
            Self::File(e) => lang.file_error(e),
            Self::Export(e) => lang.io_error(e),
            Self::Unreadable(e) => lang.with_reason(
                lang.pick(
                    "書く PSD を読み戻せません",
                    "Cannot read back the PSD to write",
                ),
                lang.io_error(e),
            ),
            Self::Overrun(over) => overrun_text(lang, over),
            Self::PartlyReplaced { done, cause } => {
                let cause = lang.file_error(cause);
                lang.pick(
                    format!(
                        "置き換えの途中で失敗しました（{cause}）。先の {done} 個は置き換わっています。"
                    ),
                    format!(
                        "Failed partway through replacing ({cause}). {done} earlier file(s) already replaced."
                    ),
                )
            }
            Self::Message { ja, en } => lang.pick(ja.clone(), en.clone()),
        }
    }

    /// 理由のツールチップ（予算で断ったものは、設定で上げられること。説明はここに置く）。
    fn tooltip(&self, lang: Lang) -> Option<String> {
        let Self::Overrun(over) = self else {
            return None;
        };
        Some(overrun_tooltip(lang, over).into())
    }
}

/// 予算・形式の上限で書けない理由（レイヤーの名前つき。取り込みの断りと同じ言い回し）。
fn overrun_text(lang: Lang, over: &Overrun) -> String {
    match over {
        Overrun::Canvas { width, height } => lang.pick(
            format!("キャンバスが大きすぎます（{width}×{height}）"),
            format!("Canvas too large ({width}×{height})"),
        ),
        Overrun::Side {
            width,
            height,
            limit,
        } => lang.pick(
            format!("キャンバスの辺が長すぎます（{width}×{height}・1 辺 {limit} まで）"),
            format!("Canvas side too long ({width}×{height}; up to {limit} per side)"),
        ),
        Overrun::Layers { count, limit } => lang.pick(
            format!("レイヤーが多すぎます（{count} 件・上限 {limit} 件）"),
            format!("Too many layers ({count}; limit {limit})"),
        ),
        Overrun::Memory { layer } if layer.is_empty() => lang.pick(
            "画素の予算を超えました".into(),
            "Pixel budget exceeded".into(),
        ),
        Overrun::Memory { layer } => lang.pick(
            format!("「{layer}」でレイヤーのメモリの予算を超えました"),
            format!("Layer memory budget exceeded at \"{layer}\""),
        ),
        Overrun::Extra => lang.pick(
            "レイヤーの付加情報が予算を超えました".into(),
            "Layer extra data exceeds the budget".into(),
        ),
        Overrun::FileSize { layer } if layer.is_empty() => lang.pick(
            "PSD が 2 GiB を超えます".into(),
            "The PSD would exceed 2 GiB".into(),
        ),
        Overrun::FileSize { layer } => lang.pick(
            format!("「{layer}」を書くと PSD が 2 GiB を超えます"),
            format!("Writing \"{layer}\" would make the PSD exceed 2 GiB"),
        ),
    }
}

/// 上限の理由のツールチップ: 予算で断ったものは設定で上げられること、PSD の 2 GiB・辺 30000 は形式の上限であること。
fn overrun_tooltip(lang: Lang, over: &Overrun) -> &'static str {
    match over {
        Overrun::FileSize { .. } => lang.pick(
            "PSD は 1 ファイル 2 GiB までです",
            "A PSD file is limited to 2 GiB",
        ),
        Overrun::Side { .. } if !over.raised_by_budget() => lang.pick(
            "PSD は 1 辺 30000 までです",
            "A PSD side is limited to 30000 pixels",
        ),
        Overrun::Layers { .. } => lang.pick(
            "上限は設定の「レイヤーのメモリ」から決まります（グループは区切りの記録も数えます）。上げると書き出せることがあります",
            "The limit follows Layer memory in Settings (each group also takes a divider record). Raising it may let this export succeed",
        ),
        _ => lang.pick(
            "上限は設定の「レイヤーのメモリ」から決まります。上げると書き出せることがあります",
            "The limit follows Layer memory in Settings. Raising it may let this export succeed",
        ),
    }
}

enum Kind {
    Import(PsdTarget),
    /// 書き出し（計画の仕事も、書く仕事も）。
    Export,
}

struct Job {
    kind: Kind,
    file: String,
    worker: Worker<Result<Output, Failure>>,
    /// 今のセットの文書を替える読み込みが始まったときの、セットの uid・文書の ID・版（読んでいる間に変わっていたら入れない）。
    guard: Option<(u32, u128, u64)>,
}

/// 書き出しの設定（ウィンドウで選ぶ）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportSettings {
    /// 方式。
    pub mode: ExportMode,
    /// 書くチャンネル（番号の順。1 つ以上）。
    pub channels: Vec<Channel>,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            mode: ExportMode::Bake,
            channels: vec![Channel::Color],
        }
    }
}

/// 置き換える確かめ（書き出す先として選んだファイルと、置き換えるファイルの一覧）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replace {
    /// 書き出す先として選んだファイル（「置き換える」で同じ書き出しをもう一度始める）。
    pub path: PathBuf,
    /// 置き換えるファイル。
    pub files: Vec<PathBuf>,
    /// 取り込んだ PSD を含む。
    pub imported: bool,
}

/// 1 回の書き出し: 計画した文書の写しと、書く先・計画（`targets` と `plans` は同じ並び）。
pub struct Run {
    snapshot: Arc<Document>,
    targets: Vec<(Channel, PathBuf)>,
    plans: Vec<ExportPlan>,
    /// 選んだファイルの名前（札・ウィンドウの見出しの下）。
    file: String,
    /// 書き出しに許すレイヤーの画素のバイト数（設定の「レイヤーのメモリ」。始めたときの値）。
    budget: u64,
}

/// 書く前の確かめ（焼く・丸める・落とす）の待ち。
pub struct NotesConfirm {
    run: Run,
}

impl NotesConfirm {
    /// 計画した文書の写し（チャンネルの名前を引く）。
    pub fn document(&self) -> &Document {
        &self.run.snapshot
    }
    /// 選んだファイルの名前。
    pub fn file(&self) -> &str {
        &self.run.file
    }
    /// チャンネルごとの注記（書く順）。注記の無いチャンネルは含めない。
    pub fn sections(&self) -> Vec<(Channel, Vec<ExportNote>)> {
        self.run
            .targets
            .iter()
            .zip(&self.run.plans)
            .filter(|(_, p)| !p.notes.is_empty())
            .map(|((c, _), p)| (*c, p.notes.clone()))
            .collect()
    }
}

/// PSD の状態。
#[derive(Default)]
pub struct PsdState {
    pub report: Option<Report>,
    pub report_offset: Vec2,
    /// 取り込んだ PSD の場所（同じファイルへ書き出すときに確かめる）。
    imported: Vec<PathBuf>,
    /// 置き換えるかの確かめ。
    pub confirm: Option<Replace>,
    pub confirm_offset: Vec2,
    /// 取り込みの確かめ（無視・落とす・変わるもの）の待ち。読んだ文書をここに持ち、「取り込む」で入れる。
    pub import_check: Option<crate::psd_import::ImportCheck>,
    pub import_window: crate::psd_import::CheckWindow,
    /// 書き出しの設定と、そのウィンドウ。
    pub export: ExportSettings,
    pub options_open: bool,
    pub options_offset: Vec2,
    pub options_scroll: f32,
    /// 書く前の確かめ（焼く・丸める・落とす）の待ち。
    pub notes_confirm: Option<NotesConfirm>,
    pub notes_offset: Vec2,
    job: Option<Job>,
    /// 試験用: 次の仕事を、取消が来るまで始めずに止めておく（始めるときに下ろす）。
    #[doc(hidden)]
    pub park_next: bool,
    /// 試験用: 次の書く仕事（確かめのあと）を、取消が来るまで始めずに止めておく。
    #[doc(hidden)]
    pub park_write: bool,
}

/// 進み具合（仕事の札）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub importing: bool,
    pub file: String,
    pub canceling: bool,
}

impl PsdState {
    pub fn is_busy(&self) -> bool {
        self.job.is_some()
    }

    /// ほかの道（Live Link の元の絵）で取り込んだ PSD の場所を覚える（同じファイルへ書き出すときに確かめる）。走っている取り込みは
    /// 失敗・取消で最後の 1 つを外すので、先頭に入れる。
    pub(crate) fn remember_imported(&mut self, path: PathBuf) {
        self.imported.insert(0, path);
    }

    pub fn progress(&self) -> Option<Progress> {
        let job = self.job.as_ref()?;
        Some(Progress {
            importing: matches!(job.kind, Kind::Import(_)),
            file: job.file.clone(),
            canceling: job.worker.is_canceled(),
        })
    }
}

/// PSD の取り込み・書き出し（札・閉じる前の確かめ・止める）。置き換え・設定・書く前・取り込みの確認のウィンドウは、キーの割り当てを止める。
pub(crate) const JOB: JobSpec = JobSpec {
    repaint: true,
    card: Some(|app, lang| {
        let p = app.psd.progress()?;
        Some(JobCard {
            text: format!(
                "{} — {}",
                if p.importing {
                    lang.pick("PSD を読み込み中", "Reading PSD")
                } else {
                    lang.pick("PSD を書き出し中", "Writing PSD")
                },
                p.file
            ),
            fraction: None,
            cancel: Some(Action::Psd(PsdAction::Cancel)),
            canceling: p.canceling,
        })
    }),
    close: Some(|app| {
        let progress = app.psd.progress()?;
        Some(if progress.importing {
            CloseJob::PsdImport
        } else {
            CloseJob::PsdExport
        })
    }),
    cancel: Some(|app| app.apply(Action::Psd(PsdAction::Cancel))),
    poll_while_stopping: Some(AppState::poll_psd),
    modal: Some(|app| {
        app.psd.confirm.is_some()
            || app.psd.options_open
            || app.psd.notes_confirm.is_some()
            || app.psd.import_check.is_some()
    }),
    ..JobSpec::new("psd", |app| app.psd.is_busy())
};

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// ファイル名からセットの名前の元を作る（拡張子を除き、制御文字は `_` に、長すぎれば 128 文字まで。空なら "PSD"）。
fn set_name_from(file: &str) -> String {
    let stem: String = Path::new(file)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .take(128)
        .collect();
    if stem.trim().is_empty() {
        "PSD".into()
    } else {
        stem.trim().to_owned()
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// 書き出す先のファイル（チャンネルごと）。1 つのチャンネルは選んだファイルそのまま、複数のチャンネルは `<名前>_<チャンネル>.psd` を並べる
/// （名前は選んだファイルの拡張子を除いたもの。チャンネルの綴りは書き出しの画像と同じ）。名前が重なるときは理由を返す。
fn export_targets(
    doc: &Document,
    path: &Path,
    channels: &[Channel],
) -> Result<Vec<(Channel, PathBuf)>, String> {
    if let [only] = channels {
        return Ok(vec![(*only, path.to_path_buf())]);
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Texture".into());
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let targets: Vec<(Channel, PathBuf)> = channels
        .iter()
        .map(|c| {
            (
                *c,
                dir.join(format!(
                    "{stem}_{}.psd",
                    crate::export::channel_suffix(doc, *c)
                )),
            )
        })
        .collect();
    let mut seen: Vec<String> = Vec::new();
    for (_, p) in &targets {
        let name = file_name(p).to_lowercase();
        if seen.contains(&name) {
            return Err(name);
        }
        seen.push(name);
    }
    Ok(targets)
}

/// 書き出しの設定のうち、文書にあるチャンネルだけ（空なら Color）。
fn export_channels(doc: &Document, settings: &ExportSettings) -> Vec<Channel> {
    let have = doc.channels();
    let mut channels: Vec<Channel> = settings
        .channels
        .iter()
        .copied()
        .filter(|c| have.contains(c))
        .collect();
    if channels.is_empty() {
        channels.push(Channel::Color);
    }
    channels
}

/// 書き出す先を選ぶウィンドウに出す初めのファイル名（1 つのチャンネルが Color 以外なら末尾にチャンネルの名前）。
pub fn default_export_name(state: &AppState) -> String {
    let stem = crate::export::stem(state);
    let base = if state.sets.len() > 1 {
        format!("{stem}_{}", state.sets.current().name)
    } else {
        stem
    };
    match export_channels(&state.doc, &state.psd.export).as_slice() {
        [only] if *only != Channel::Color => {
            format!(
                "{base}_{}.psd",
                crate::export::channel_suffix(&state.doc, *only)
            )
        }
        _ => format!("{base}.psd"),
    }
}

impl AppState {
    pub fn psd_apply(&mut self, action: PsdAction) {
        let lang = self.lang;
        let stroking = self.is_stroking();
        let refuse =
            |s: &mut AppState| s.refuse(Source::Psd, crate::lang::refusals::during_stroke(lang));
        match action {
            PsdAction::ImportDialog(target) => {
                if stroking {
                    return refuse(self);
                }
                if target == PsdTarget::CurrentSet {
                    if let Some(reason) = self.read_only_reason() {
                        self.refuse(
                            Source::Psd,
                            crate::lang::refusals::read_only_set(lang, reason),
                        );
                        return;
                    }
                }
                self.dialog_request = Some(DialogRequest::PsdImport(target));
            }
            PsdAction::Import { path, target } => {
                if stroking {
                    return refuse(self);
                }
                self.start_psd_import(&path, target);
            }
            PsdAction::ExportDialog => {
                if stroking {
                    return refuse(self);
                }
                if let Some(reason) = self.read_only_reason() {
                    self.refuse(
                        Source::Psd,
                        crate::lang::refusals::read_only_set(lang, reason),
                    );
                    return;
                }
                // 文書にあるチャンネルだけを残す（消したユーザーチャンネルを選んだままにしない）
                self.psd.export.channels = export_channels(&self.doc, &self.psd.export);
                self.psd.options_scroll = 0.0;
                self.psd.options_open = true;
            }
            PsdAction::SetExportMode(mode) => self.psd.export.mode = mode,
            PsdAction::ToggleExportChannel(c) => {
                let channels = &mut self.psd.export.channels;
                if let Some(at) = channels.iter().position(|x| *x == c) {
                    // 最後の 1 つは外せない（書くチャンネルが無くならない）
                    if channels.len() > 1 {
                        channels.remove(at);
                    }
                } else {
                    channels.push(c);
                    channels.sort();
                }
            }
            PsdAction::CancelExportOptions => self.psd.options_open = false,
            PsdAction::ChooseExportFile => {
                if stroking {
                    return refuse(self);
                }
                self.psd.options_open = false;
                self.dialog_request = Some(DialogRequest::PsdExport);
            }
            PsdAction::Export(path) => {
                if stroking {
                    return refuse(self);
                }
                self.start_psd_export(&path, false);
            }
            PsdAction::ConfirmImport => {
                if let Some(check) = self.psd.import_check.take() {
                    let crate::psd_import::ImportCheck {
                        doc,
                        file,
                        target,
                        guard,
                        ..
                    } = check;
                    self.install_checked(*doc, &file, target, guard);
                }
            }
            PsdAction::CancelImport => {
                if self.psd.import_check.take().is_some() {
                    self.psd.imported.pop();
                    self.info(
                        Source::Psd,
                        lang.pick("PSD の取り込みをやめました。", "PSD import canceled."),
                    );
                }
            }
            PsdAction::ConfirmReplace => {
                if let Some(replace) = self.psd.confirm.take() {
                    self.start_psd_export(&replace.path, true);
                }
            }
            PsdAction::CancelConfirm => {
                if self.psd.confirm.take().is_some() {
                    self.info(
                        Source::Psd,
                        lang.pick("PSD の書き出しをやめました。", "PSD export canceled."),
                    );
                }
            }
            PsdAction::ConfirmWrite => {
                if let Some(confirm) = self.psd.notes_confirm.take() {
                    self.start_psd_write(confirm.run);
                }
            }
            PsdAction::CancelWrite => {
                if self.psd.notes_confirm.take().is_some() {
                    self.info(
                        Source::Psd,
                        lang.pick("PSD の書き出しをやめました。", "PSD export canceled."),
                    );
                }
            }
            PsdAction::Cancel => {
                if let Some(job) = &self.psd.job {
                    job.worker.cancel();
                    self.info(
                        Source::Psd,
                        lang.pick("PSD の処理を取り消しています…", "Canceling the PSD job…"),
                    );
                }
            }
            PsdAction::DismissReport => self.psd.report = None,
        }
    }

    fn start_psd_import(&mut self, path: &Path, target: PsdTarget) {
        let lang = self.lang;
        if self.psd.job.is_some() || self.psd.import_check.is_some() {
            self.refuse(
                Source::Psd,
                lang.pick("PSD を処理中です。", "A PSD job is running."),
            );
            return;
        }
        if target == PsdTarget::CurrentSet {
            if let Some(reason) = self.read_only_reason() {
                self.refuse(
                    Source::Psd,
                    crate::lang::refusals::read_only_set(lang, reason),
                );
                return;
            }
        }
        // 新しいセットとして足す PSD は、セットの数の上限に当たっているなら、読み始める前に断る
        if target == PsdTarget::NewSet && self.sets.len() >= crate::newproject::MAX_SETS {
            self.refuse(Source::Psd, crate::lang::refusals::set_limit(lang));
            return;
        }
        let path_owned = path.to_path_buf();
        let park = std::mem::take(&mut self.psd.park_next);
        // レイヤーの数・画素の上限は、設定の「レイヤーのメモリ」の予算から決める（.ylp を開くときと同じ）
        let budget = self.load_source_bytes();
        let spawned = Worker::spawn_on(
            psd_thread("yolu-psd-import"),
            Arc::default(),
            move |tx, flag| {
                if park {
                    crate::windows::park_until_canceled(flag.flag());
                }
                let _ = tx.send(import_worker(&path_owned, budget, flag.flag()));
            },
        );
        let worker = match spawned {
            Ok(w) => w,
            Err(e) => {
                self.fail(
                    Source::Psd,
                    lang.with_reason(
                        lang.pick("PSD を読み込めません", "Cannot read the PSD"),
                        lang.thread_error(&e),
                    ),
                );
                return;
            }
        };
        let file = file_name(path);
        self.info(
            Source::Psd,
            lang.pick(
                format!("PSD を読み込み中: {file}"),
                format!("Reading PSD: {file}"),
            ),
        );
        let guard = (target == PsdTarget::CurrentSet)
            .then(|| (self.sets.current().uid, self.doc.id(), self.doc.revision()));
        self.psd.job = Some(Job {
            kind: Kind::Import(target),
            file,
            worker,
            guard,
        });
        // 取り込んだ場所を覚える（同じファイルへ書き出すときに確かめる）
        self.psd.imported.push(path.to_path_buf());
    }

    /// 書き出せない理由をウィンドウ（結果のウィンドウ）に並べ、何も書かない。
    /// `kind` は知らせの種類（読むだけのセットは断り、名前の重なり・予算・書けない場所は失敗）。
    fn refuse_psd_export(&mut self, kind: NoticeKind, file: &str, why: Vec<String>) {
        let lines = why.into_iter().map(|text| Line::new(true, text)).collect();
        self.refuse_psd_export_lines(kind, file, lines)
    }

    /// `refuse_psd_export` の、行にツールチップ（設定で上げられること）が付くもの。
    fn refuse_psd_export_lines(&mut self, kind: NoticeKind, file: &str, lines: Vec<Line>) {
        let lang = self.lang;
        let text = lang.with_reason(
            lang.pick("PSD に書き出せません", "Cannot export PSD"),
            lines.first().map(|l| l.text.clone()).unwrap_or_default(),
        );
        self.notify(kind, Source::Psd, text);
        self.psd.report = Some(Report {
            importing: false,
            file: file.to_owned(),
            ok: false,
            summary: lang
                .pick(
                    "書き出せません。何も書いていません",
                    "Cannot export. Nothing was written",
                )
                .into(),
            lines,
        });
    }

    fn start_psd_export(&mut self, path: &Path, confirmed: bool) {
        let lang = self.lang;
        if self.psd.job.is_some() {
            self.refuse(
                Source::Psd,
                lang.pick("PSD を処理中です。", "A PSD job is running."),
            );
            return;
        }
        let file = file_name(path);
        if let Some(reason) = self.read_only_reason() {
            let why = vec![crate::lang::refusals::read_only_set(lang, reason)];
            return self.refuse_psd_export(NoticeKind::Refusal, &file, why);
        }
        let channels = export_channels(&self.doc, &self.psd.export);
        let mode = self.psd.export.mode;
        let targets = match export_targets(&self.doc, path, &channels) {
            Ok(t) => t,
            Err(name) => {
                let why = vec![lang.pick(
                    format!("チャンネルのファイル名{}が重なります", lang.quote(&name)),
                    format!("The channel file name {} clashes", lang.quote(&name)),
                )];
                return self.refuse_psd_export(NoticeKind::Error, &file, why);
            }
        };
        // 文書そのものが書き出せるか（キャンバス・レイヤーの数の予算。設定の「レイヤーのメモリ」から決まる）を、何も作らずに断る
        let budget = self.load_source_bytes();
        let ctl = ExportControl {
            source_budget: Some(budget),
            ..ExportControl::default()
        };
        for (channel, _) in &targets {
            if let Err(e) = psd::check_exportable(&self.doc, *channel, &ctl) {
                let failure = export_failure(e);
                let line = Line {
                    warning: true,
                    text: failure.text(lang, false),
                    tooltip: failure.tooltip(lang),
                };
                return self.refuse_psd_export_lines(NoticeKind::Error, &file, vec![line]);
            }
        }
        // 置き換えるファイル: 取り込んだ PSD と、複数のチャンネルで書き分けた名前のうちもうあるもの（選ぶウィンドウが確かめたのは選んだ名前だけ）
        if !confirmed {
            let imported: Vec<&(Channel, PathBuf)> = targets
                .iter()
                .filter(|(_, t)| self.psd.imported.iter().any(|p| same_file(p, t)))
                .collect();
            let files: Vec<PathBuf> = targets
                .iter()
                .filter(|(_, t)| {
                    self.psd.imported.iter().any(|p| same_file(p, t))
                        || (targets.len() > 1 && t.exists())
                })
                .map(|(_, t)| t.clone())
                .collect();
            if !files.is_empty() {
                self.psd.confirm = Some(Replace {
                    path: path.to_path_buf(),
                    files,
                    imported: !imported.is_empty(),
                });
                return;
            }
        }
        // 文書の写し（履歴の無い、タイルを共有する写し。文書は変えず、描き続けてよい）
        let snapshot = match self.doc.capture_snapshot() {
            Ok(d) => Arc::new(d),
            Err(e) => {
                return self.refuse_psd_export(NoticeKind::Error, &file, vec![lang.core_error(&e)])
            }
        };
        self.psd.notes_confirm = None;
        let park = std::mem::take(&mut self.psd.park_next);
        let name = file.clone();
        let spawned = Worker::spawn_on(
            psd_thread("yolu-psd-plan"),
            Arc::default(),
            move |tx, flag| {
                if park {
                    crate::windows::park_until_canceled(flag.flag());
                }
                let _ = tx.send(plan_worker(
                    snapshot,
                    mode,
                    targets,
                    name,
                    budget,
                    flag.flag(),
                ));
            },
        );
        let worker = match spawned {
            Ok(w) => w,
            Err(e) => {
                self.fail(
                    Source::Psd,
                    lang.with_reason(
                        lang.pick("PSD に書き出せません", "Cannot export PSD"),
                        lang.thread_error(&e),
                    ),
                );
                return;
            }
        };
        self.info(
            Source::Psd,
            lang.pick(
                format!("PSD に書き出し中: {file}"),
                format!("Writing PSD: {file}"),
            ),
        );
        self.psd.job = Some(Job {
            kind: Kind::Export,
            file,
            worker,
            guard: None,
        });
    }

    /// 計画（と、あれば利用者の確かめ）が済んだ書き出しを、別のスレッドで書く。
    fn start_psd_write(&mut self, run: Run) {
        let lang = self.lang;
        let file = run.file.clone();
        let park = std::mem::take(&mut self.psd.park_write);
        let spawned = Worker::spawn_on(
            psd_thread("yolu-psd-export"),
            Arc::default(),
            move |tx, flag| {
                if park {
                    crate::windows::park_until_canceled(flag.flag());
                }
                let _ = tx.send(write_worker(run, flag.flag()));
            },
        );
        let worker = match spawned {
            Ok(w) => w,
            Err(e) => {
                self.fail(
                    Source::Psd,
                    lang.with_reason(
                        lang.pick("PSD に書き出せません", "Cannot export PSD"),
                        lang.thread_error(&e),
                    ),
                );
                return;
            }
        };
        self.info(
            Source::Psd,
            lang.pick(
                format!("PSD に書き出し中: {file}"),
                format!("Writing PSD: {file}"),
            ),
        );
        self.psd.job = Some(Job {
            kind: Kind::Export,
            file,
            worker,
            guard: None,
        });
    }

    /// 計画ができた: 書けないものがあれば理由を見せて何も書かず、焼く・丸める・落とすものがあれば確かめを待ち、無ければすぐ書く。
    fn finish_psd_plan(&mut self, run: Run) {
        let many = run.targets.len() > 1;
        let why: Vec<String> = run
            .targets
            .iter()
            .zip(&run.plans)
            .flat_map(|((c, _), plan)| {
                plan.blockers
                    .iter()
                    .map(|b| blocker_text(self.lang, &run.snapshot, many.then_some(*c), b))
                    .collect::<Vec<_>>()
            })
            .collect();
        if !why.is_empty() {
            let file = run.file.clone();
            return self.refuse_psd_export(NoticeKind::Error, &file, why);
        }
        if run.plans.iter().all(|p| p.notes.is_empty()) {
            self.start_psd_write(run);
        } else {
            self.psd.notes_confirm = Some(NotesConfirm { run });
        }
    }

    /// 終わった PSD の仕事を受ける（フレームの初めに）。
    pub fn poll_psd(&mut self) {
        let lang = self.lang;
        let Some(job) = &self.psd.job else {
            return;
        };
        let result = match job.worker.poll() {
            Polled::Message(r) => r,
            Polled::Empty => return,
            Polled::Lost => Err(Failure::Stopped),
        };
        let job = self.psd.job.take().expect("上で見た");
        let importing = matches!(job.kind, Kind::Import(_));
        let output = match result {
            Ok(o) => o,
            Err(e) => {
                if importing {
                    // 読めなかった取り込みは覚えない
                    self.psd.imported.pop();
                }
                let canceled = matches!(e, Failure::Canceled);
                let text = e.text(lang, importing);
                if canceled {
                    self.info(Source::Psd, text);
                } else if importing {
                    let message = lang.with_reason(
                        lang.pick("PSD を読み込めません", "Cannot read the PSD"),
                        text,
                    );
                    self.fail(Source::Psd, message);
                } else {
                    // 書き出せなかった理由（予算の超過・書けない場所など）は結果のウィンドウにも出す（知らせは `refuse_psd_export_lines` の 1 回）。
                    // 取消は利用者の操作なので出さない
                    let line = Line {
                        warning: true,
                        text,
                        tooltip: e.tooltip(lang),
                    };
                    self.refuse_psd_export_lines(NoticeKind::Error, &job.file, vec![line]);
                }
                return;
            }
        };
        match (output, job.kind) {
            (Output::Refused(why), _) => {
                self.psd.imported.pop();
                let reason = crate::lang::psd_copy_refusal(lang, &why);
                self.fail(
                    Source::Psd,
                    lang.with_reason(
                        lang.pick("PSD を読み込めません", "Cannot import the PSD"),
                        &reason,
                    ),
                );
                self.psd.report = Some(Report {
                    importing: true,
                    file: job.file,
                    ok: false,
                    summary: lang
                        .pick(
                            "読み込めません。ファイルは変えていません",
                            "Cannot import. The file was not modified",
                        )
                        .into(),
                    lines: vec![Line {
                        warning: true,
                        text: reason,
                        tooltip: crate::lang::psd_copy_refusal_tooltip(lang, &why),
                    }],
                });
            }
            (Output::Imported { doc, notes }, Kind::Import(target)) => {
                // 無視だけ（見え方にも内容にも効かない）なら確かめずに取り込む。落とす・変わるものがあれば確認のウィンドウ
                if notes
                    .iter()
                    .all(|n| n.action == yolu_io::psd::ImportAction::Ignored)
                {
                    self.install_checked(*doc, &job.file, target, job.guard);
                } else {
                    // 無視・落とす・変わるものがある: 利用者が確かめるまで、何も入れない
                    self.info(
                        Source::Psd,
                        lang.pick(
                            format!("PSD を読みました: {}（取り込みの確かめ待ち）", job.file),
                            format!("Read {} (waiting for the import check)", job.file),
                        ),
                    );
                    self.psd.import_check = Some(crate::psd_import::ImportCheck {
                        doc,
                        notes,
                        file: job.file,
                        target,
                        guard: job.guard,
                    });
                }
            }
            (Output::Planned(run), Kind::Export) => self.finish_psd_plan(*run),
            (Output::Exported { files }, Kind::Export) => {
                self.info(
                    Source::Psd,
                    match files.as_slice() {
                        [(path, bytes)] => lang.pick(
                            format!(
                                "PSD に書き出しました: {}（{} バイト）。",
                                path.display(),
                                bytes
                            ),
                            format!("Wrote PSD: {} ({} bytes).", path.display(), bytes),
                        ),
                        many => {
                            let dir = many[0]
                                .0
                                .parent()
                                .map(|d| d.display().to_string())
                                .unwrap_or_default();
                            lang.pick(
                                format!("PSD を {} 個書き出しました: {dir}", many.len()),
                                format!("Wrote {} PSDs: {dir}", many.len()),
                            )
                        }
                    },
                );
            }
            _ => {}
        }
    }

    /// 読み込んで確かめを終えた文書を入れる。今のセットの文書を替える読み込みは、描いている最中か、読み始めてから文書が変わっていれば
    /// 入れない（描いたものを黙って捨てず、描きかけのストロークを取り残さない。ストロークの確定で文書の版が進むので、終わるまで待っても入らない）。
    fn install_checked(
        &mut self,
        doc: Document,
        file: &str,
        target: PsdTarget,
        guard: Option<(u32, u128, u64)>,
    ) {
        let lang = self.lang;
        if let Some((uid, id, revision)) = guard {
            if self.is_stroking() {
                self.psd.imported.pop();
                self.warn(
                    Source::Psd,
                    lang.pick(
                        format!("描いている間に読み終わったので、{file} は入れませんでした。"),
                        format!("{file} finished reading while drawing, so it was not imported."),
                    ),
                );
                return;
            }
            if self.sets.current().uid != uid
                || self.doc.id() != id
                || self.doc.revision() != revision
            {
                self.psd.imported.pop();
                self.warn(
                    Source::Psd,
                    lang.pick(
                        format!(
                            "読み込んでいる間にプロジェクトが変わったので、{file} は入れませんでした。"
                        ),
                        format!("The project changed while reading, so {file} was not imported."),
                    ),
                );
                return;
            }
        }
        // 読んでいる間にセットが増えて上限に当たったら、入れない（読み始める前の確かめと同じ理由）
        if target == PsdTarget::NewSet && self.sets.len() >= crate::newproject::MAX_SETS {
            self.psd.imported.pop();
            self.warn(Source::Psd, crate::lang::refusals::set_limit(lang));
            return;
        }
        let layers = doc.layers().len();
        let switched = self.install_psd(doc, target, file);
        let mut text = lang.pick(
            format!(
                "PSD を読み込みました（{file}・レイヤー {layers}）。PSD 自体は書き換えません。"
            ),
            format!("Imported {file} ({layers} layers). The PSD itself is never rewritten."),
        );
        if switched {
            self.info(Source::Psd, text);
        } else {
            // 入れたが、描いている間なので足したセットへは切り替えていない（気をつけること）
            text += lang.pick(
                " 描いている間なので、切り替えていません。",
                " Not switched to it while drawing.",
            );
            self.warn(Source::Psd, text);
        }
    }

    /// 読み込んだ文書を行き先へ入れる。足したセットへ切り替えられたか（描いている間は切り替えない）を返す。
    fn install_psd(&mut self, doc: Document, target: PsdTarget, file: &str) -> bool {
        let mut switched = true;
        match target {
            PsdTarget::NewSet => {
                let stem = set_name_from(file);
                let name = unique_name(
                    &stem,
                    self.sets
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .into_iter(),
                );
                let index = self.sets.push(
                    guid_string(doc.id()),
                    name.clone(),
                    false,
                    MaterialRef::Material { name, asset: None },
                    None,
                    doc,
                );
                self.bind_model();
                switched = self.switch_set(index).is_ok();
            }
            PsdTarget::CurrentSet => {
                if let Some(set) = self.sets.get_mut(self.sets.current_index()) {
                    set.saved = None;
                }
                // 大きさが変わりうるので、表示は既定に戻す
                self.install_document(doc, crate::sets::Keep::reset());
            }
        }
        self.modified = true;
        switched
    }

    /// 試験用: PSD の仕事が終わるまで待って受ける（待ちの上限は 120 秒）。
    #[doc(hidden)]
    pub fn wait_psd(&mut self) {
        crate::jobs::wait_until_idle(
            self,
            "PSD の処理が終わらない（ハング検出上限）",
            |s| s.psd.job.is_some(),
            Self::poll_psd,
        );
    }
}

/// 別のスレッドの読み込み: ファイルを流して読み（原本は持たない）、core の文書にする。取り込めなければ理由を `Refused` で返す（何も変えない）。
/// `budget` は、この文書のレイヤーの画素に許すバイト数（設定の「レイヤーのメモリ」）。レイヤーの数・キャンバス・レイヤーの画素の上限はここから決まる。
fn import_worker(path: &Path, budget: u64, cancel: &AtomicBool) -> Result<Output, Failure> {
    let outcome = read_copy(path, budget, cancel).map_err(|e| match e {
        ReadCopyError::File(io) => Failure::File(io),
        ReadCopyError::Canceled => Failure::Canceled,
        ReadCopyError::Other(e) => Failure::Export(e),
    })?;
    Ok(match outcome {
        CopyOutcome::Refused(why) => Output::Refused(why),
        CopyOutcome::Imported(imported) => {
            let psd::CopyImport { document, notes } = *imported;
            Output::Imported {
                doc: Box::new(document),
                notes,
            }
        }
    })
}

/// [`read_copy`] が読めなかった理由。
#[derive(Debug)]
pub(crate) enum ReadCopyError {
    /// ファイルを開けない・読めない。
    File(std::io::Error),
    /// 取消の旗が立った。
    Canceled,
    /// 上のどれでもない（core の誤りなど）。
    Other(yolu_io::Error),
}

/// PSD のファイルを流して読み、写しとして core の文書にする（取り込みの道。Live Link の元の絵もここを通り、同じ文書と知らせになる）。
/// 原本は開いて読むだけで、書き換えない。`budget` は、この文書のレイヤーの画素に許すバイト数。
pub(crate) fn read_copy(
    path: &Path,
    budget: u64,
    cancel: &AtomicBool,
) -> Result<CopyOutcome, ReadCopyError> {
    let file = std::fs::File::open(path).map_err(ReadCopyError::File)?;
    let mut reader = std::io::BufReader::with_capacity(256 * 1024, file);
    let outcome = psd::import_copy(
        &mut reader,
        &CopyOptions {
            source_budget: budget,
            cancel: Some(cancel),
        },
    )
    .map_err(|e| match e {
        yolu_io::Error::Io(io) => ReadCopyError::File(io),
        e if is_cancel(&e) => ReadCopyError::Canceled,
        e => ReadCopyError::Other(e),
    })?;
    if cancel.load(Ordering::Relaxed) {
        return Err(ReadCopyError::Canceled);
    }
    Ok(outcome)
}

/// [`read_copy`] を、取り込みの仕事と同じスタックの大きさのスレッドで読む（ほかの裏の仕事のスレッドから呼ぶ。統合画像と照らす合成の
/// グループの入れ子の再帰に、取り込みの仕事と同じだけの余裕を持たせる）。読み終えるまで待つ。
pub(crate) fn read_copy_on_psd_stack(
    path: &Path,
    budget: u64,
    cancel: &AtomicBool,
) -> Result<CopyOutcome, ReadCopyError> {
    std::thread::scope(|scope| {
        psd_thread("yolu-psd-import")
            .spawn_scoped(scope, || read_copy(path, budget, cancel))
            .map_err(|e| ReadCopyError::Other(yolu_io::Error::Io(e)))?
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

fn is_cancel(e: &yolu_io::Error) -> bool {
    matches!(e, yolu_io::Error::Core(yolu_core::CoreError::Cancelled))
}

fn export_error(e: yolu_io::Error) -> Failure {
    if is_cancel(&e) {
        Failure::Canceled
    } else {
        Failure::Export(e)
    }
}

/// 書き出しの失敗: 予算・形式の上限は種類を保ち（レイヤーの名前つきの文とツールチップを作る）、ほかは取消かその文。
fn export_failure(e: ExportError) -> Failure {
    match e {
        ExportError::Overrun(over) => Failure::Overrun(over),
        ExportError::Other(e) => export_error(e),
    }
}

/// 別のスレッドの計画: 選んだチャンネルごとに、何を焼き・丸め・落とすかを決める（画素は作らない。丸める調整は、丸めた文書との合成の差を測る）。
fn plan_worker(
    snapshot: Arc<Document>,
    mode: ExportMode,
    targets: Vec<(Channel, PathBuf)>,
    file: String,
    budget: u64,
    cancel: &AtomicBool,
) -> Result<Output, Failure> {
    let ctl = ExportControl {
        cancel: Some(cancel),
        source_budget: Some(budget),
        ..ExportControl::default()
    };
    let mut plans = Vec::with_capacity(targets.len());
    for (channel, _) in &targets {
        if cancel.load(Ordering::Relaxed) {
            return Err(Failure::Canceled);
        }
        plans.push(
            psd::plan_export(&snapshot, &ExportOptions::new(*channel, mode), &ctl)
                .map_err(export_error)?,
        );
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Failure::Canceled);
    }
    Ok(Output::Planned(Box::new(Run {
        snapshot,
        targets,
        plans,
        file,
        budget,
    })))
}

/// 別のスレッドの書き出し: チャンネルごとに、PSD を一時ファイルへレイヤーを 1 枚ずつ流して書き（焼く・圧縮する。メモリにはレイヤー 1 枚ぶんだけ）、流して読み戻して
/// 確かめ（`yolu_io::psd::stage_verified`）、全部が済んでから最後に置き換える。途中の失敗・取消では一時ファイルを消し、元のファイルは変えない。
fn write_worker(run: Run, cancel: &AtomicBool) -> Result<Output, Failure> {
    let ctl = ExportControl {
        cancel: Some(cancel),
        source_budget: Some(run.budget),
        ..ExportControl::default()
    };
    // 一時ファイルは持ち主（`psd::Staged`）が消す（途中で抜けても）
    let mut staged: Vec<psd::Staged> = Vec::with_capacity(run.targets.len());
    for ((_, path), plan) in run.targets.iter().zip(&run.plans) {
        staged.push(
            psd::stage_verified(path, plan, &run.snapshot, &ctl)
                .map_err(|e| write_failure(path, e))?,
        );
        if cancel.load(Ordering::Relaxed) {
            return Err(Failure::Canceled);
        }
    }
    let files: Vec<(PathBuf, usize)> = staged
        .iter()
        .map(|s| (s.path().to_path_buf(), s.len() as usize))
        .collect();
    commit(staged)?;
    Ok(Output::Exported { files })
}

/// 検証つきの書き出しの失敗を、画面の理由にする。読み戻しの失敗は、壊れている（読めない）か、ファイルの読み込みの失敗か取消か。
fn write_failure(path: &Path, e: psd::WriteError) -> Failure {
    use psd::WriteError as W;
    match e {
        W::NotAFile if path.file_name().is_none() => {
            Failure::message("ファイル名がありません", "The file has no name")
        }
        W::NotAFile => Failure::message(
            format!(
                "{}は通常のファイルではありません",
                Lang::Ja.quote(&path.display().to_string())
            ),
            format!(
                "{} is not a regular file",
                Lang::En.quote(&path.display().to_string())
            ),
        ),
        W::CreateTemp { source, .. } | W::Sync { source, .. } | W::Rewind { source, .. } => {
            Failure::File(source)
        }
        W::Write(e) => export_failure(e),
        W::ReadBack(e) if is_cancel(&e) => Failure::Canceled,
        W::ReadBack(e) => Failure::Unreadable(e),
        W::Compare(e) => read_failure(e),
        W::Mismatch => Failure::message(
            "書いたファイルの読み戻しが一致しません",
            "The written file does not read back identically",
        ),
        W::Commit(e) => Failure::File(e),
        W::Exists => Failure::File(std::io::ErrorKind::AlreadyExists.into()),
    }
}

/// 読み戻しの失敗: 取消、ファイルの読み込みの失敗、それ以外。
fn read_failure(e: yolu_io::Error) -> Failure {
    match e {
        e if is_cancel(&e) => Failure::Canceled,
        yolu_io::Error::Io(io) => Failure::File(io),
        e => Failure::Unreadable(e),
    }
}

/// 同じフォルダの一時ファイルへ `fill` で書き、`check` で確かめる（試験用: PSD でない中身で、一時ファイルの後始末と置き換えを確かめる）。
#[cfg(test)]
fn stage(
    path: &Path,
    fill: impl FnOnce(&mut std::io::BufWriter<&mut std::fs::File>) -> Result<(), Failure>,
    check: impl FnOnce(&Path) -> Result<(), Failure>,
) -> Result<psd::Staged, Failure> {
    psd::stage_with(
        path,
        fill,
        |temp, _| check(temp),
        |e| write_failure(path, e),
    )
}

/// 書いた PSD を読み戻して確かめる（試験用: 書いた長さ・レイヤーの数・CRC を与えて、確かめの断り方を見る。`None` は書いた記録が無い）。
#[cfg(test)]
fn verify_written(
    path: &Path,
    written: Option<psd::Written>,
    cancel: &AtomicBool,
) -> Result<(), Failure> {
    let written = written.ok_or_else(|| write_failure(path, psd::WriteError::Mismatch))?;
    let mut file = std::fs::File::open(path).map_err(Failure::File)?;
    psd::check_written(path, &mut file, written, Some(cancel)).map_err(|e| write_failure(path, e))
}

/// 同じ中身の書き出しを試験で使う: バイト列を書いて、読み戻して一致を確かめる。
#[cfg(test)]
fn stage_bytes(path: &Path, bytes: &[u8]) -> Result<psd::Staged, Failure> {
    use std::io::Write;
    stage(
        path,
        |out| out.write_all(bytes).map_err(Failure::File),
        |temp| {
            if std::fs::read(temp).map_err(Failure::File)? != bytes {
                return Err(Failure::message(
                    "書いたファイルの読み戻しが一致しません",
                    "The written file does not read back identically",
                ));
            }
            Ok(())
        },
    )
}

/// 最後に 1 回ずつ置き換える。途中で失敗したら、残りの一時ファイルを消し（捨てた `psd::Staged` が消す）、済んだ分は置き換わっていると知らせる。
fn commit(staged: Vec<psd::Staged>) -> Result<(), Failure> {
    for (done, s) in staged.into_iter().enumerate() {
        if let Err(e) = s.commit(psd::Commit::Replace) {
            let cause = match e {
                psd::WriteError::Commit(e) => e,
                other => std::io::Error::other(other.to_string()),
            };
            return Err(if done == 0 {
                Failure::File(cause)
            } else {
                Failure::PartlyReplaced { done, cause }
            });
        }
    }
    Ok(())
}

/// 1 つのファイルを、一時ファイルへ書き・読み戻し・置き換えまで通す（途中で失敗しても元のファイルは変わらず、一時ファイルは残らない）。試験用。
#[cfg(test)]
fn write_replacing(path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    commit(vec![stage_bytes(path, bytes)?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Action;
    use std::sync::atomic::AtomicU32;
    use yolu_core::{Channel, LayerId, TileCoord};
    use yolu_io::psd::{CompatibilityMode, Compression, Limits};

    /// 試験用の一時フォルダ（終わると消す）。
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Dir {
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!(
                "yolu-app-psd-{name}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }

        fn files(&self) -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 文書の左下の `w` × `h` を不透明の色で塗る。
    fn paint(doc: &mut Document, layer: LayerId, w: u32, h: u32, rgba: [u8; 4]) {
        let ts = doc.tile_size();
        for ty in 0..h.div_ceil(ts) {
            for tx in 0..w.div_ceil(ts) {
                let mut tile = vec![0u8; (ts * ts * 4) as usize];
                for y in 0..ts.min(h - ty * ts) {
                    for x in 0..ts.min(w - tx * ts) {
                        tile[((y * ts + x) * 4) as usize..][..4].copy_from_slice(&rgba);
                    }
                }
                doc.import_tile(layer, Channel::Color, TileCoord::new(tx, ty), &tile)
                    .unwrap();
            }
        }
    }

    /// 2 枚のレイヤー（赤の下地・半透明の青）を持つ 64 × 64 の状態。
    fn painted() -> AppState {
        let mut s = AppState::new(64, 64);
        let first = s.selected_layer.unwrap();
        paint(&mut s.doc, first, 64, 64, [255, 0, 0, 255]);
        s.apply(Action::NewLayer);
        let second = s.selected_layer.unwrap();
        paint(&mut s.doc, second, 32, 32, [0, 0, 255, 255]);
        // PSD は不透明度を 1/255 の刻みで持つので、その刻みの値にしておく
        s.doc
            .set_layer_opacity(second, 128.0 / 255.0, false)
            .unwrap();
        s
    }

    fn composite(s: &AppState) -> Vec<u8> {
        s.doc.composite(s.doc.bounds()).unwrap()
    }

    #[test]
    fn writing_then_importing_as_a_new_set_keeps_the_layers_and_the_picture() {
        let dir = Dir::new("round");
        let mut a = painted();
        let path = dir.0.join("Body.psd");
        a.apply(Action::Psd(PsdAction::Export(path.clone())));
        assert!(a.psd.is_busy(), "別のスレッドで書く");
        a.wait_psd();
        assert!(a.message.contains("書き出しました"), "{}", a.message);
        assert_eq!(dir.files(), ["Body.psd"], "一時ファイルは残らない");
        let before = composite(&a);

        let mut b = AppState::new(32, 32);
        b.modified = false;
        b.apply(Action::Psd(PsdAction::Import {
            path: path.clone(),
            target: PsdTarget::NewSet,
        }));
        assert!(b.psd.is_busy());
        assert_eq!(b.sets.len(), 1, "読み終わるまでセットは増えない");
        b.wait_psd();
        assert_eq!(b.sets.len(), 2);
        assert_eq!(b.sets.current().name, "Body", "セットの名前はファイル名");
        assert_eq!(b.sets.current_index(), 1, "追加したセットへ替える");
        assert_eq!((b.doc.width(), b.doc.height()), (64, 64));
        assert_eq!(b.doc.layers().len(), 2);
        assert!(composite(&b) == before, "合成は同じ");
        assert!(b.modified);
        assert!(!b.doc.can_undo(), "読み込みは Undo の段に入らない");
        assert!(b.message.contains("PSD を読み込みました"), "{}", b.message);
        // 同じ名前なら番号を付ける
        b.apply(Action::Psd(PsdAction::Import {
            path,
            target: PsdTarget::NewSet,
        }));
        b.wait_psd();
        assert_eq!(b.sets.current().name, "Body 2");
        // 元の（描いていなかった）セットはそのまま
        assert_eq!((b.set_doc(0).width(), b.set_doc(0).layers().len()), (32, 1));
    }

    /// レイヤーのロックは、書き出す PSD の lspf を通って、取り込んだ文書のレイヤーに戻る（断らず、黙って外しもしない）。
    #[test]
    fn locks_come_back_through_a_psd_export_and_import() {
        use yolu_core::LayerLocks;
        let dir = Dir::new("locks");
        let mut a = painted();
        let layers: Vec<LayerId> = a.doc.layers().iter().map(|l| l.id()).collect();
        a.doc
            .set_layer_locks(layers[0], LayerLocks::TRANSPARENCY | LayerLocks::POSITION)
            .unwrap();
        a.doc.set_layer_locks(layers[1], LayerLocks::ALL).unwrap();
        let path = dir.0.join("Locked.psd");
        a.apply(Action::Psd(PsdAction::Export(path.clone())));
        a.wait_psd();
        assert!(a.message.contains("書き出しました"), "{}", a.message);
        let mut b = AppState::new(32, 32);
        b.apply(Action::Psd(PsdAction::Import {
            path,
            target: PsdTarget::NewSet,
        }));
        b.wait_psd();
        assert!(b.message.contains("PSD を読み込みました"), "{}", b.message);
        assert_eq!(b.doc.layers().len(), 2);
        let [low, top] = [b.doc.layers()[0].id(), b.doc.layers()[1].id()];
        assert_eq!(
            b.doc.layer(low).unwrap().locks(),
            LayerLocks::TRANSPARENCY | LayerLocks::POSITION
        );
        assert_eq!(
            b.doc.effective_locks(top).unwrap(),
            LayerLocks::from_bits(15).unwrap(),
            "すべては個別を含んで効く"
        );
        assert!(!b.doc.can_undo(), "読み込みは Undo の段に入らない");
        assert!(composite(&b) == composite(&a), "ロックは合成を変えない");
    }

    #[test]
    fn importing_into_the_current_set_replaces_its_document() {
        let dir = Dir::new("current");
        let mut a = painted();
        let path = dir.0.join("x.psd");
        a.apply(Action::Psd(PsdAction::Export(path.clone())));
        a.wait_psd();
        let expected = composite(&a);

        let mut b = AppState::new(32, 32);
        let old_id = b.doc.id();
        b.apply(Action::Psd(PsdAction::Import {
            path,
            target: PsdTarget::CurrentSet,
        }));
        b.wait_psd();
        assert_eq!(b.sets.len(), 1, "セットは増えない");
        assert_ne!(b.doc.id(), old_id);
        assert_eq!((b.doc.width(), b.doc.layers().len()), (64, 2));
        assert!(composite(&b) == expected, "合成は同じ");
        assert!(b.selected_layer.is_some_and(|id| b.doc.layer(id).is_some()));
        assert!(b.modified);
        // 読むだけのセットには入れない
        b.sets.get_mut(0).unwrap().read_only = Some("試験".into());
        b.apply(Action::Psd(PsdAction::ImportDialog(PsdTarget::CurrentSet)));
        assert_eq!(b.dialog_request, None);
        assert!(b.message.contains("読むだけ"), "{}", b.message);
    }

    #[test]
    fn a_current_set_import_is_not_installed_when_the_document_changed_while_reading() {
        let dir = Dir::new("guard");
        let mut a = painted();
        let path = dir.0.join("g.psd");
        a.apply(Action::Psd(PsdAction::Export(path.clone())));
        a.wait_psd();
        // 読んでいる間に描いた（レイヤーを足した）: 入れずに知らせる
        let mut b = AppState::new(32, 32);
        let id = b.doc.id();
        b.apply(Action::Psd(PsdAction::Import {
            path: path.clone(),
            target: PsdTarget::CurrentSet,
        }));
        b.apply(Action::NewLayer);
        b.wait_psd();
        assert_eq!(b.doc.id(), id, "文書は替えない");
        assert_eq!(b.doc.layers().len(), 2, "描いたものは残る");
        assert!(
            b.message.contains("プロジェクトが変わった"),
            "{}",
            b.message
        );
        assert!(b.psd.report.is_none());
        // 読んでいる間にテクスチャセットが替わった（別のプロジェクトを開いた）ときも入れない
        let mut c = AppState::new(32, 32);
        c.apply(Action::Psd(PsdAction::Import {
            path,
            target: PsdTarget::CurrentSet,
        }));
        crate::project::new_into(&mut c);
        c.wait_psd();
        assert_eq!(c.doc.width(), 2048, "新しいプロジェクトの文書のまま");
        assert!(
            c.message.contains("プロジェクトが変わった"),
            "{}",
            c.message
        );
        // 新しいセットとして足す読み込みは、読んでいる間に描いても足せる
        let mut d = AppState::new(32, 32);
        d.apply(Action::Psd(PsdAction::Import {
            path: dir.0.join("g.psd"),
            target: PsdTarget::NewSet,
        }));
        d.apply(Action::NewLayer);
        d.wait_psd();
        assert_eq!(d.sets.len(), 2);
    }

    #[test]
    fn stopping_jobs_cancels_a_running_write_and_leaves_no_temp_files() {
        let dir = Dir::new("stop");
        let mut a = painted();
        // 取消が来るまで始めない仕事にする（取消が効いたことを、書き終わる速さに頼らず確かめる）
        a.psd.park_next = true;
        a.apply(Action::Psd(PsdAction::Export(dir.0.join("s.psd"))));
        assert!(a.psd.is_busy());
        crate::windows::stop_jobs(&mut a, std::time::Duration::from_secs(30));
        assert!(!a.psd.is_busy(), "取消で止まる");
        assert!(a.message.contains("取り消しました"), "{}", a.message);
        assert!(dir.files().is_empty(), "何も書かない: {:?}", dir.files());
        assert!(a.psd.report.is_none());
    }

    #[test]
    fn the_set_name_comes_from_the_file_name() {
        assert_eq!(set_name_from("Body.psd"), "Body");
        assert_eq!(set_name_from("a.b.psd"), "a.b");
        assert_eq!(set_name_from(".psd"), ".psd", "拡張子だけの名前はそのまま");
        assert_eq!(set_name_from("x\u{7}y.psd"), "x_y");
        assert_eq!(set_name_from("  .psd"), "PSD");
        assert_eq!(
            set_name_from(&format!("{}.psd", "あ".repeat(300)))
                .chars()
                .count(),
            128
        );
    }

    /// 書き出しはメニューで設定のウィンドウを開き、「書き出し…」で書き出す先を選ぶウィンドウを頼む（読み込みは先のファイルを選ぶだけ）。
    #[test]
    fn the_menu_opens_the_export_settings_and_only_import_asks_for_the_file_at_once() {
        let mut s = AppState::new(32, 32);
        s.apply(Action::Psd(PsdAction::ImportDialog(PsdTarget::NewSet)));
        assert_eq!(
            s.dialog_request,
            Some(DialogRequest::PsdImport(PsdTarget::NewSet))
        );
        s.dialog_request = None;
        s.apply(Action::Psd(PsdAction::ExportDialog));
        assert!(s.psd.options_open);
        assert_eq!(s.dialog_request, None);
        s.apply(Action::Psd(PsdAction::ChooseExportFile));
        assert!(!s.psd.options_open);
        assert_eq!(s.dialog_request, Some(DialogRequest::PsdExport));
        // やめる
        s.dialog_request = None;
        s.apply(Action::Psd(PsdAction::ExportDialog));
        s.apply(Action::Psd(PsdAction::CancelExportOptions));
        assert!(!s.psd.options_open && s.dialog_request.is_none());
    }

    #[test]
    fn files_that_cannot_be_imported_are_refused_with_reasons_and_change_nothing() {
        let dir = Dir::new("refuse");
        let good = {
            let s = painted();
            let projected = psd::Document::from_core(&s.doc).unwrap();
            psd::write(&projected, &Limits::default()).unwrap()
        };
        // PSB（版 2）・16 bit・壊れたファイルは取り込めない
        let mut psb = good.clone();
        psb[4..6].copy_from_slice(&2u16.to_be_bytes());
        std::fs::write(dir.0.join("big.psb.psd"), &psb).unwrap();
        let mut deep = good.clone();
        deep[22..24].copy_from_slice(&16u16.to_be_bytes());
        std::fs::write(dir.0.join("deep.psd"), &deep).unwrap();
        std::fs::write(dir.0.join("broken.psd"), b"8BPS-not-a-psd").unwrap();

        let mut s = AppState::new(32, 32);
        let before = (s.sets.len(), s.doc.id(), s.modified);
        for (file, expect_text) in [
            ("big.psb.psd", "PSB"),
            ("deep.psd", "8 bit"),
            ("broken.psd", "途中で切れて"),
        ] {
            s.apply(Action::Psd(PsdAction::Import {
                path: dir.0.join(file),
                target: PsdTarget::NewSet,
            }));
            s.wait_psd();
            assert_eq!(
                (s.sets.len(), s.doc.id(), s.modified),
                before,
                "{file}: 何も変えない"
            );
            assert!(
                s.message.contains("読み込めません"),
                "{file}: {}",
                s.message
            );
            let report = s.psd.report.take().expect(file);
            assert!(!report.ok && report.importing, "{file}");
            assert!(
                report.lines.iter().any(|l| l.text.contains(expect_text)),
                "{file}: {:?}",
                report.lines
            );
            assert!(report.lines[0].warning);
            assert!(
                s.psd.import_check.is_none(),
                "{file}: 確認のウィンドウは出さない"
            );
        }
        // 英語の文言
        s.lang = Lang::En;
        s.apply(Action::Psd(PsdAction::Import {
            path: dir.0.join("deep.psd"),
            target: PsdTarget::NewSet,
        }));
        s.wait_psd();
        assert_eq!(
            s.message,
            "Cannot import the PSD (Only RGB 8-bit PSDs can be imported (RGB, 16-bit))."
        );
        // 読めないファイル
        s.apply(Action::Psd(PsdAction::Import {
            path: dir.0.join("nothing.psd"),
            target: PsdTarget::NewSet,
        }));
        s.wait_psd();
        assert!(s.message.starts_with("Cannot read"), "{}", s.message);
        assert_eq!(s.sets.len(), 1);
    }

    /// レイヤー ID を持たない 2 レイヤーの PSD（原本を保つ読みは編集できない内容として断る。写しとしては、新しい ID を振って取り込める）。
    fn without_layer_ids() -> Vec<u8> {
        let layer = |id: i32, name: &str, rgba: [u8; 4]| psd::Layer {
            id,
            name: name.into(),
            width: 8,
            height: 8,
            pixels_rgba: rgba.repeat(64),
            ..psd::Layer::default()
        };
        let doc = psd::Document {
            width: 8,
            height: 8,
            layers: vec![
                layer(2, "上", [0, 0, 255, 255]),
                layer(1, "下", [255, 0, 0, 255]),
            ],
            composite_rgba: None,
        };
        let mut bytes = psd::write(&doc, &Limits::default()).unwrap();
        // 書き手はレイヤー ID を必ず書くので、lyid のタグを同じ長さの別のタグ（レイヤー名の元）に書き換えて、ID の無い PSD にする
        let mut at = 0;
        while let Some(i) = bytes[at..].windows(4).position(|w| w == b"lyid") {
            bytes[at + i..at + i + 4].copy_from_slice(b"lnsr");
            at += i + 4;
        }
        bytes
    }

    /// `without_layer_ids` に、確かめの要るもの（変わる）を 1 つ足した PSD: 下のレイヤーの合成モードを、取り込めない「ディゾルブ」にする
    /// （取り込みでは通常になる。レイヤー ID の振り直しとレイヤーのメタデータは無視なので、それだけでは確認のウィンドウを出さない）。
    fn needing_a_check() -> Vec<u8> {
        let mut bytes = without_layer_ids();
        let i = bytes
            .windows(8)
            .position(|w| w == b"8BIMnorm")
            .expect("レイヤーの記録の合成モード");
        bytes[i + 4..i + 8].copy_from_slice(b"diss");
        bytes
    }

    /// 無視だけ（レイヤー ID の振り直し・知らないメタデータ）の PSD は、確かめずにそのまま入る。
    #[test]
    fn a_psd_with_only_ignored_information_is_imported_without_the_check() {
        let dir = Dir::new("ignored-only");
        let path = dir.0.join("Ids.psd");
        std::fs::write(&path, without_layer_ids()).unwrap();
        let mut s = AppState::new(32, 32);
        s.apply(Action::Psd(PsdAction::Import {
            path,
            target: PsdTarget::NewSet,
        }));
        s.wait_psd();
        assert!(s.psd.import_check.is_none(), "確認のウィンドウを出さない");
        assert_eq!(s.sets.len(), 2);
        let names: Vec<&str> = s.doc.layers().iter().map(|l| l.name()).collect();
        assert_eq!(names, ["下", "上"]);
    }

    /// 無視・落とす・変わるものがあれば、確かめるまで何も入れない。「やめる」は何も変えず、「取り込む」で入る。
    #[test]
    fn what_the_psd_cannot_keep_is_checked_first_and_nothing_is_imported_until_the_yes() {
        let dir = Dir::new("check");
        let path = dir.0.join("Ids.psd");
        let bytes = needing_a_check();
        assert_eq!(
            psd::read(&bytes, &Limits::default()).unwrap().mode(),
            CompatibilityMode::PreserveOnly
        );
        std::fs::write(&path, &bytes).unwrap();
        let mut s = AppState::new(32, 32);
        let before = (s.sets.len(), s.doc.id(), s.modified);
        s.apply(Action::Psd(PsdAction::Import {
            path: path.clone(),
            target: PsdTarget::NewSet,
        }));
        s.wait_psd();
        assert_eq!(
            (s.sets.len(), s.doc.id(), s.modified),
            before,
            "確かめるまで入れない"
        );
        let check = s.psd.import_check.as_ref().expect("確認のウィンドウ");
        assert_eq!(check.file(), "Ids.psd");
        let ids = check
            .notes()
            .iter()
            .find(|n| n.feature == yolu_io::psd::ImportFeature::LayerIds)
            .expect("レイヤー ID");
        assert_eq!(ids.layers, ["下", "上"], "取り込みの順（下から上）");
        assert_eq!(
            check.notes().len(),
            3,
            "レイヤー ID・メタデータ（無視）と合成モード（変わる）: {:?}",
            check.notes()
        );
        assert!(check
            .notes()
            .iter()
            .any(|n| n.feature == yolu_io::psd::ImportFeature::BlendMode
                && n.action == yolu_io::psd::ImportAction::Changed));
        assert!(s.message.contains("確かめ待ち"), "{}", s.message);
        // 確かめている間は、ほかの取り込みを始めない
        s.apply(Action::Psd(PsdAction::Import {
            path: path.clone(),
            target: PsdTarget::NewSet,
        }));
        assert!(!s.psd.is_busy(), "{}", s.message);
        // やめる: 何も変えず、取り込んだ場所も覚えない
        s.apply(Action::Psd(PsdAction::CancelImport));
        assert!(s.psd.import_check.is_none());
        assert_eq!((s.sets.len(), s.doc.id(), s.modified), before);
        assert!(s.psd.imported.is_empty());
        assert!(s.message.contains("やめました"), "{}", s.message);
        // もう一度読んで、取り込む
        s.apply(Action::Psd(PsdAction::Import {
            path: path.clone(),
            target: PsdTarget::NewSet,
        }));
        s.wait_psd();
        s.apply(Action::Psd(PsdAction::ConfirmImport));
        assert!(s.psd.import_check.is_none());
        assert_eq!(s.sets.len(), 2);
        assert_eq!(s.sets.current().name, "Ids");
        let names: Vec<&str> = s.doc.layers().iter().map(|l| l.name()).collect();
        assert_eq!(names, ["下", "上"]);
        let ids: Vec<u128> = s.doc.layers().iter().map(|l| l.id().0 >> 96).collect();
        assert!(ids.iter().all(|i| *i > 0) && ids[0] != ids[1], "{ids:?}");
        assert!(s.message.contains("PSD を読み込みました"), "{}", s.message);
        assert!(!s.doc.can_undo(), "読み込みは Undo の段に入らない");
        // 今のセットへの取り込みも、確かめ → 取り込むで文書が替わる
        s.apply(Action::Psd(PsdAction::Import {
            path,
            target: PsdTarget::CurrentSet,
        }));
        s.wait_psd();
        let old = s.doc.id();
        assert!(s.psd.import_check.is_some());
        s.apply(Action::Psd(PsdAction::ConfirmImport));
        assert_ne!(s.doc.id(), old);
        assert_eq!(s.doc.layers().len(), 2);
    }

    /// 確かめを待つ間に文書が変わったら、今のセットへの取り込みは入れない（描いたものを黙って捨てない）。
    #[test]
    fn a_current_set_import_waiting_for_the_check_is_not_installed_over_a_changed_document() {
        let dir = Dir::new("check-guard");
        let path = dir.0.join("Ids.psd");
        std::fs::write(&path, needing_a_check()).unwrap();
        let mut s = AppState::new(32, 32);
        s.apply(Action::Psd(PsdAction::Import {
            path,
            target: PsdTarget::CurrentSet,
        }));
        s.wait_psd();
        assert!(s.psd.import_check.is_some());
        let drawn = s.selected_layer.unwrap();
        paint(&mut s.doc, drawn, 8, 8, [1, 2, 3, 255]);
        let id = s.doc.id();
        s.apply(Action::Psd(PsdAction::ConfirmImport));
        assert_eq!(s.doc.id(), id, "文書は替わらない");
        assert!(
            s.message.contains("プロジェクトが変わった"),
            "{}",
            s.message
        );
        assert!(s.psd.import_check.is_none());
    }

    /// 取り消した読み込みは確認のウィンドウを出さず、何も入れない。
    #[test]
    fn canceling_a_read_before_the_check_leaves_no_check_window() {
        let dir = Dir::new("check-cancel");
        let path = dir.0.join("Ids.psd");
        std::fs::write(&path, needing_a_check()).unwrap();
        let mut s = AppState::new(32, 32);
        s.psd.park_next = true;
        s.apply(Action::Psd(PsdAction::Import {
            path,
            target: PsdTarget::NewSet,
        }));
        assert!(s.psd.is_busy());
        s.apply(Action::Psd(PsdAction::Cancel));
        s.wait_psd();
        assert!(s.psd.import_check.is_none());
        assert_eq!(s.sets.len(), 1);
        assert!(s.message.contains("取り消しました"), "{}", s.message);
        assert!(s.psd.imported.is_empty());
    }

    /// はみ出した画素は切り捨てて取り込み、本当に落ちるものは落としたこととして確かめに出る。
    #[test]
    fn pixels_outside_the_canvas_are_cut_and_told_as_dropped() {
        let dir = Dir::new("outside");
        let mut wide = psd::Document::from_core(&painted().doc).unwrap();
        // 一番下のレイヤー（キャンバスいっぱいの赤）を右へずらして、キャンバスの外へ画素を出す
        wide.layers.last_mut().unwrap().left = 20;
        wide.composite_rgba = None;
        std::fs::write(
            dir.0.join("wide.psd"),
            psd::write(&wide, &Limits::default()).unwrap(),
        )
        .unwrap();
        let mut s = AppState::new(32, 32);
        s.apply(Action::Psd(PsdAction::Import {
            path: dir.0.join("wide.psd"),
            target: PsdTarget::NewSet,
        }));
        s.wait_psd();
        let check = s.psd.import_check.as_ref().expect("確認のウィンドウ");
        let note = check
            .notes()
            .iter()
            .find(|n| n.feature == yolu_io::psd::ImportFeature::OutsideCanvas)
            .expect("キャンバス外の画素");
        assert_eq!(note.action, yolu_io::psd::ImportAction::Dropped);
        s.apply(Action::Psd(PsdAction::ConfirmImport));
        assert_eq!(s.sets.len(), 2);
    }

    /// 書き出しの確認のウィンドウの注記（機能と結果）を、チャンネルごとに日本語で。
    fn lines(s: &AppState) -> Vec<(String, String, String)> {
        let confirm = s.psd.notes_confirm.as_ref().expect("確認のウィンドウ");
        confirm
            .sections()
            .into_iter()
            .flat_map(|(_, notes)| notes)
            .map(|n| {
                let (what, how) = crate::psd_export::note_columns(s.lang, &n);
                (n.layer, what, how)
            })
            .collect()
    }

    /// PSD に形の無いもの（反転したマスク・クリッピングされたグループ・半透明の塗りつぶし）は、書く前にレイヤーの名前つきで確かめ、「書く」までは
    /// 何も書かない。書いたあとの取り込みは、書き出したチャンネルの今の合成と同じ。
    #[test]
    fn export_asks_before_baking_what_a_psd_cannot_hold_and_writes_nothing_until_then() {
        let dir = Dir::new("blockers");
        let path = dir.0.join("out.psd");
        let mut s = painted();
        // 反転したマスク（PSD に非破壊の反転が無い）。マスクそのものは書ける
        let layer = s.selected_layer.unwrap();
        s.apply(Action::M2(crate::m2::Edit::AddMask(layer)));
        s.doc.set_mask_pixel(layer, 4, 4, 200).unwrap();
        s.apply(Action::Psd(PsdAction::Export(path.clone())));
        s.wait_psd();
        assert!(s.psd.notes_confirm.is_none(), "マスクは書ける");
        assert!(path.exists());
        std::fs::remove_file(&path).unwrap();
        s.doc.set_layer_mask_inverted(layer, true).unwrap();
        let (revision, undo) = (s.doc.revision(), s.doc.undo_count());
        s.apply(Action::Psd(PsdAction::Export(path.clone())));
        assert!(s.psd.is_busy(), "計画は別のスレッド");
        s.wait_psd();
        assert!(!s.psd.is_busy());
        assert!(dir.files().is_empty(), "確かめるまで何も書かない");
        let name = s.doc.layer(layer).unwrap().name().to_owned();
        assert_eq!(
            lines(&s),
            [(
                name.clone(),
                "反転したマスク".to_owned(),
                "マスクの画素へ".to_owned()
            )]
        );
        s.lang = Lang::En;
        assert_eq!(
            lines(&s),
            [(
                name,
                "Inverted mask".to_owned(),
                "To mask pixels".to_owned()
            )]
        );
        s.lang = Lang::Ja;
        // やめる: 何も書かない
        s.apply(Action::Psd(PsdAction::CancelWrite));
        assert!(s.psd.notes_confirm.is_none() && dir.files().is_empty());
        assert!(s.message.contains("やめました"), "{}", s.message);
        // 書く: 書いた PSD を取り込むと、見た目は同じ（反転は画素になる）
        let expected = composite(&s);
        s.apply(Action::Psd(PsdAction::Export(path.clone())));
        s.wait_psd();
        s.apply(Action::Psd(PsdAction::ConfirmWrite));
        assert!(s.psd.is_busy());
        s.wait_psd();
        assert!(s.message.contains("書き出しました"), "{}", s.message);
        assert_eq!(dir.files(), ["out.psd"], "一時ファイルは残らない");
        let mut t = AppState::new(32, 32);
        t.apply(Action::Psd(PsdAction::Import {
            path: path.clone(),
            target: PsdTarget::CurrentSet,
        }));
        t.wait_psd();
        assert_eq!(composite(&t), expected);
        let imported = t.doc.layers().iter().find(|l| l.mask().is_some()).unwrap();
        assert!(!imported.mask().unwrap().inverted(), "反転は画素になった");
        // 文書は変わらない（反転も残る。書き出しは写し）
        assert_eq!((s.doc.revision(), s.doc.undo_count()), (revision, undo));
        assert!(s.doc.layer(layer).unwrap().mask().unwrap().inverted());

        // クリッピングされたグループ・半透明の塗りつぶし
        let mut u = painted();
        u.apply(Action::M2(crate::m2::Edit::NewGroup));
        let group = u.selected_layer.unwrap();
        u.doc.set_layer_clipping(group, true).unwrap();
        u.apply(Action::M2(crate::m2::Edit::NewFill));
        let fill = u.selected_layer.unwrap();
        u.doc
            .set_fill_value(
                fill,
                Channel::Color,
                Some(yolu_core::Rgba8::new(1, 2, 3, 100)),
                false,
            )
            .unwrap();
        u.apply(Action::Psd(PsdAction::Export(dir.0.join("u.psd"))));
        u.wait_psd();
        let kinds: Vec<String> = lines(&u).into_iter().map(|l| l.1).collect();
        assert_eq!(kinds, ["半透明の塗りつぶし", "クリッピングされたグループ"]);
        u.lang = Lang::En;
        let kinds: Vec<String> = lines(&u).into_iter().map(|l| l.1).collect();
        assert_eq!(kinds, ["Translucent fill", "Clipped group"]);
        assert!(!dir.0.join("u.psd").exists());
        // 読むだけのセットは書かない
        let mut v = painted();
        v.sets.get_mut(0).unwrap().read_only = Some("試験".into());
        v.apply(Action::Psd(PsdAction::Export(dir.0.join("v.psd"))));
        assert!(!v.psd.is_busy());
        assert!(v.psd.report.as_ref().unwrap().lines[0]
            .text
            .contains("読むだけ"));
        assert!(!dir.0.join("v.psd").exists());
    }

    /// グループ・塗りつぶし・調整・マスクは PSD に書けて、取り込み直すと同じレイヤーになる（画面の操作で作った文書で、書き出して取り込む）。
    #[test]
    fn groups_fills_adjustments_and_masks_export_and_come_back_as_the_same_layers() {
        let dir = Dir::new("m2-round-trip");
        let path = dir.0.join("m2.psd");
        let mut s = painted();
        let base = s.selected_layer.unwrap();
        s.apply(Action::M2(crate::m2::Edit::AddMask(base)));
        s.doc.set_mask_pixel(base, 3, 3, 200).unwrap();
        s.apply(Action::M2(crate::m2::Edit::NewFill));
        s.apply(Action::M2(crate::m2::Edit::NewAdjustment(
            crate::m2::AdjustmentKind::Invert,
        )));
        s.apply(Action::M2(crate::m2::Edit::NewGroup));
        let expected: Vec<_> = s
            .doc
            .layers()
            .iter()
            .map(|l| (l.name().to_string(), l.kind(), l.mask().is_some()))
            .collect();
        let composite = s.doc.composite(s.doc.bounds()).unwrap();
        s.apply(Action::Psd(PsdAction::Export(path.clone())));
        s.wait_psd();
        assert!(s.psd.notes_confirm.is_none(), "焼くものは無い");
        assert!(
            s.psd.report.as_ref().is_none_or(|r| r.ok),
            "{:?}",
            s.psd.report
        );
        assert!(path.exists());
        let mut t = AppState::new(32, 32);
        t.apply(Action::Psd(PsdAction::Import {
            path,
            target: PsdTarget::CurrentSet,
        }));
        t.wait_psd();
        let got: Vec<_> = t
            .doc
            .layers()
            .iter()
            .map(|l| (l.name().to_string(), l.kind(), l.mask().is_some()))
            .collect();
        assert_eq!(got, expected);
        assert_eq!(t.doc.composite(t.doc.bounds()).unwrap(), composite);
    }

    /// 厳密な `from_core` は PSD に形が無い中身を黙って落とさず、機能ごとの理由で断る（焼き込みの書き出しは別）。レイヤーのロックは PSD に書けるので断らない。
    /// Color の PSD は Color の合成を書き、ほかのチャンネルの合成の違いは断る理由にならない。
    #[test]
    fn from_core_refuses_what_it_cannot_write_as_it_is_and_writes_the_locks() {
        use yolu_core::{BlendMode, ChannelBlend, LayerLocks};
        let mut s = painted();
        let layer = s.selected_layer.unwrap();
        s.apply(Action::M2(crate::m2::Edit::AddMask(layer)));
        s.doc.set_layer_mask_inverted(layer, true).unwrap();
        let err = psd::Document::from_core(&s.doc).unwrap_err().to_string();
        assert!(err.contains("反転"), "{err}");
        assert_eq!(psd::export_blockers(&s.doc).len(), 1);
        // Color と違う、ほかのチャンネルの合成（Color の合成は PSD のレイヤーの合成として書け、ほかのチャンネルはそのチャンネルの PSD の値になる）
        let mut t = painted();
        let layer = t.selected_layer.unwrap();
        t.doc
            .set_channel_blend(
                layer,
                Channel::Color,
                ChannelBlend::new(Some(BlendMode::Multiply), None),
                false,
            )
            .unwrap();
        t.doc
            .set_channel_blend(
                layer,
                Channel::Roughness,
                ChannelBlend::new(Some(BlendMode::Screen), None),
                false,
            )
            .unwrap();
        assert!(psd::Document::from_core(&t.doc).is_ok());
        assert!(psd::export_blockers(&t.doc).is_empty());
        // レイヤーのロック: from_core は lspf のビットで書く
        let mut v = painted();
        let layer = v.selected_layer.unwrap();
        v.doc.set_layer_locks(layer, LayerLocks::PIXELS).unwrap();
        assert!(psd::export_blockers(&v.doc).is_empty());
        let projected = psd::Document::from_core(&v.doc).expect("ロックは書ける");
        assert!(projected.layers.iter().any(|l| l.locks == 2));
    }

    #[test]
    fn exporting_over_the_imported_file_asks_first_and_replacing_is_atomic() {
        let dir = Dir::new("replace");
        let mut a = painted();
        let path = dir.0.join("src.psd");
        a.apply(Action::Psd(PsdAction::Export(path.clone())));
        a.wait_psd();
        let original = std::fs::read(&path).unwrap();
        // 取り込む
        let mut s = AppState::new(32, 32);
        s.apply(Action::Psd(PsdAction::Import {
            path: path.clone(),
            target: PsdTarget::CurrentSet,
        }));
        s.wait_psd();
        // 絵を変えて、取り込んだファイルへ書き出す: 確かめる（まだ書かない）
        let layer = s.selected_layer.unwrap();
        paint(&mut s.doc, layer, 8, 8, [0, 255, 0, 255]);
        s.apply(Action::Psd(PsdAction::Export(path.clone())));
        assert!(!s.psd.is_busy());
        let replace = s.psd.confirm.as_ref().expect("確かめ");
        assert_eq!(replace.path, path);
        assert_eq!(replace.files, std::slice::from_ref(&path));
        assert!(replace.imported);
        s.apply(Action::Psd(PsdAction::CancelConfirm));
        assert!(s.psd.confirm.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), original, "やめたら元のまま");
        // 置き換える
        s.apply(Action::Psd(PsdAction::Export(path.clone())));
        s.apply(Action::Psd(PsdAction::ConfirmReplace));
        assert!(s.psd.is_busy());
        s.wait_psd();
        assert_ne!(std::fs::read(&path).unwrap(), original);
        assert_eq!(dir.files(), ["src.psd"], "一時ファイルは残らない");
        // 別のファイルへは確かめない
        let other = dir.0.join("other.psd");
        s.apply(Action::Psd(PsdAction::Export(other.clone())));
        assert!(s.psd.confirm.is_none());
        s.wait_psd();
        assert!(other.exists());
    }

    #[test]
    fn write_replacing_keeps_the_old_file_when_it_cannot_write() {
        let dir = Dir::new("atomic");
        let path = dir.0.join("a.psd");
        write_replacing(&path, b"one").unwrap();
        write_replacing(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert_eq!(dir.files(), ["a.psd"]);
        // フォルダは置き換えない
        let folder = dir.0.join("folder.psd");
        std::fs::create_dir(&folder).unwrap();
        let err = write_replacing(&folder, b"x").unwrap_err();
        assert!(
            err.text(Lang::Ja, false)
                .contains("通常のファイルではありません"),
            "{err:?}"
        );
        assert!(
            err.text(Lang::En, false).contains("is not a regular file"),
            "{err:?}"
        );
        // 先のフォルダが無ければ書けず、元のファイルは変わらない
        let missing = dir.0.join("missing").join("b.psd");
        assert!(write_replacing(&missing, b"x").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert_eq!(dir.files(), ["a.psd", "folder.psd"]);
    }

    /// 何にもクリッピングされないグループ（兄弟の一番下）のクリッピングの印は、外して書く。確認のウィンドウに、レイヤーの名前つきで「落とす」と出る。
    #[test]
    fn a_clipping_mark_that_clips_nothing_is_listed_as_dropped_and_not_written() {
        let dir = Dir::new("idle-mark");
        let path = dir.0.join("out.psd");
        let mut s = painted();
        let all: Vec<LayerId> = s.doc.layers().iter().map(|l| l.id()).collect();
        let group = s.doc.group_layers(&all, "組").unwrap();
        s.doc.set_layer_clipping(group, true).unwrap();
        s.apply(Action::Psd(PsdAction::Export(path.clone())));
        s.wait_psd();
        assert!(dir.files().is_empty(), "確かめるまで何も書かない");
        assert_eq!(
            lines(&s),
            [(
                "組".to_owned(),
                "効いていないクリッピングの印".to_owned(),
                "落とす".to_owned()
            )]
        );
        s.lang = Lang::En;
        assert_eq!(
            lines(&s),
            [(
                "組".to_owned(),
                "Idle clipping mark".to_owned(),
                "Dropped".to_owned()
            )]
        );
        s.lang = Lang::Ja;
        let summary = crate::psd_export::summary_for_test(
            s.lang,
            &s.psd.notes_confirm.as_ref().unwrap().sections(),
        );
        assert_eq!(summary, "落とす 1");
        s.apply(Action::Psd(PsdAction::ConfirmWrite));
        s.wait_psd();
        assert!(s.message.contains("書き出しました"), "{}", s.message);
        let mut t = AppState::new(32, 32);
        t.apply(Action::Psd(PsdAction::Import {
            path,
            target: PsdTarget::CurrentSet,
        }));
        t.wait_psd();
        assert!(t.doc.layers().iter().all(|l| !l.clipping()), "印は書かない");
        assert_eq!(composite(&t), composite(&s), "見た目は同じ");
        // 文書の印は残る（書き出しは写し）
        assert!(s.doc.layer(group).unwrap().clipping());
    }

    /// 刻みの間のトーンカーブは最寄りの刻みへ丸め、グラデーションマップの値のカーブは停止点へ展開する。確認のウィンドウの注記に、レイヤーの名前つきで丸めた値
    /// （多いときは先頭の数個とほか何個）・停止点の数・合成の最大の差が日英で出て、書いたあとの取り込みは注記の差の範囲に収まる。
    #[test]
    fn color_adjustments_between_steps_are_listed_with_their_values_and_differences() {
        use yolu_core::curve::{Curve, CurvePoint};
        use yolu_core::generator::{ColorStop, OpacityStop, Ramp};
        use yolu_core::{AdjustmentSettings, GradientMap, Rgba8, ToneChannel, ToneCurves};
        let dir = Dir::new("color-adjust");
        let path = dir.0.join("out.psd");
        let mut s = painted();
        let curve = Curve::new(
            [(0.0, 0.0), (0.301, 0.31), (0.702, 0.65), (1.0, 1.0)]
                .map(|(x, y)| CurvePoint { x, y })
                .to_vec(),
        )
        .unwrap();
        s.doc
            .add_adjustment_layer(
                "曲線",
                AdjustmentSettings::tone_curve(
                    ToneCurves::identity().with_curve(ToneChannel::Composite, curve),
                ),
                None,
                None,
            )
            .unwrap();
        let stop = |position, v| ColorStop {
            position,
            color: Rgba8::new(v, v, v, 255),
            midpoint: 0.5,
        };
        let opaque = |position| OpacityStop {
            position,
            opacity: 1.0,
            midpoint: 0.5,
        };
        let ramp = Ramp::new(
            vec![stop(0.0, 0), stop(1.0, 255)],
            vec![opaque(0.0), opaque(1.0)],
            Some(
                [(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)]
                    .map(|(x, y)| CurvePoint { x, y })
                    .to_vec(),
            ),
        )
        .unwrap();
        s.doc
            .add_adjustment_layer(
                "マップ",
                AdjustmentSettings::gradient_map(GradientMap::new(ramp, false)),
                None,
                None,
            )
            .unwrap();
        s.apply(Action::Psd(PsdAction::Export(path.clone())));
        s.wait_psd();
        assert!(dir.files().is_empty(), "確かめるまで何も書かない");
        let ja = lines(&s);
        // 注記は上のレイヤーから
        let [(map, map_what, map_how), (tone, tone_what, tone_how)] = ja.as_slice() else {
            panic!("{ja:?}")
        };
        assert_eq!((tone.as_str(), map.as_str()), ("曲線", "マップ"));
        assert!(
            tone_what.contains("RGB 点 2 の入力 76.755→77") && tone_what.ends_with("、ほか 1"),
            "{tone_what}"
        );
        assert!(
            map_what.contains("カーブ → 停止点 色")
                && !map_what.contains("混色")
                && map_what.ends_with("不透明度 2"),
            "{map_what}"
        );
        assert!(tone_how.starts_with("最大差 ") && map_how.starts_with("最大差 "));
        s.lang = Lang::En;
        let en = lines(&s);
        assert!(
            en[1].1.contains("RGB point 2 input 76.755→77") && en[1].1.ends_with(", 1 more"),
            "{}",
            en[1].1
        );
        assert!(
            en[0].1.contains("Curve → stops: ")
                && !en[0].1.contains("ixing")
                && en[0].2.starts_with("Max diff ")
        );
        for (_, what, how) in &en {
            for text in [what, how] {
                assert!(
                    !text.chars().any(|c| ('\u{3000}'..='\u{30ff}').contains(&c)
                        || ('\u{4e00}'..='\u{9fff}').contains(&c)),
                    "英語の画面に日本語が残っている: {text}"
                );
            }
        }
        s.lang = Lang::Ja;
        let summary = crate::psd_export::summary_for_test(
            s.lang,
            &s.psd.notes_confirm.as_ref().unwrap().sections(),
        );
        assert_eq!(summary, "丸める 2");
        let diff = |how: &str| how.trim_start_matches("最大差 ").parse::<u8>().unwrap();
        let allowed = diff(tone_how) + diff(map_how);
        let before = composite(&s);
        s.apply(Action::Psd(PsdAction::ConfirmWrite));
        s.wait_psd();
        assert!(s.message.contains("書き出しました"), "{}", s.message);
        assert_eq!(composite(&s), before, "書き出しは文書を変えない");
        let mut t = AppState::new(32, 32);
        t.apply(Action::Psd(PsdAction::Import {
            path,
            target: PsdTarget::CurrentSet,
        }));
        t.wait_psd();
        let worst = composite(&t)
            .iter()
            .zip(&before)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= allowed, "{worst} > {allowed}");
    }

    /// 複数のチャンネルは 1 つずつ置き換えるので、途中で失敗すると先の分だけ置き換わる。残りの一時ファイルは消え、済んだ数を知らせる。
    #[test]
    fn a_failure_midway_through_replacing_leaves_the_earlier_files_replaced_and_says_how_many() {
        let dir = Dir::new("partial");
        let [a, b, c] = ["a", "b", "c"].map(|n| dir.0.join(format!("{n}.psd")));
        std::fs::write(&a, b"old-a").unwrap();
        std::fs::write(&b, b"old-b").unwrap();
        let staged: Vec<psd::Staged> = [&a, &b, &c]
            .iter()
            .map(|p| stage_bytes(p, b"new").unwrap())
            .collect();
        assert_eq!(dir.files().len(), 5, "元の 2 つと一時ファイル 3 つ");
        // 2 つ目の一時ファイルを無くして、その置き換えを失敗させる
        std::fs::remove_file(staged[1].temp_path()).unwrap();
        let err = commit(staged).unwrap_err();
        assert!(
            matches!(err, Failure::PartlyReplaced { done: 1, .. }),
            "{err:?}"
        );
        assert_eq!(
            std::fs::read(&a).unwrap(),
            b"new",
            "先の分は置き換わっている"
        );
        assert_eq!(std::fs::read(&b).unwrap(), b"old-b", "失敗した分は元のまま");
        assert!(!c.exists(), "残りは置き換えない");
        assert_eq!(
            dir.files(),
            ["a.psd", "b.psd"],
            "残りの一時ファイルは消える"
        );
        assert!(err
            .text(Lang::Ja, false)
            .contains("先の 1 個は置き換わっています"));
        assert!(err
            .text(Lang::En, false)
            .contains("1 earlier file(s) already replaced"));
    }

    /// 最初の 1 つで失敗したときは何も置き換わらず、部分的な置き換えとは言わない。
    #[test]
    fn a_failure_on_the_first_replacement_changes_nothing() {
        let dir = Dir::new("first");
        let [a, b] = ["a", "b"].map(|n| dir.0.join(format!("{n}.psd")));
        std::fs::write(&a, b"old-a").unwrap();
        let staged: Vec<psd::Staged> = [&a, &b]
            .iter()
            .map(|p| stage_bytes(p, b"new").unwrap())
            .collect();
        std::fs::remove_file(staged[0].temp_path()).unwrap();
        let err = commit(staged).unwrap_err();
        assert!(matches!(err, Failure::File(_)), "{err:?}");
        assert_eq!(std::fs::read(&a).unwrap(), b"old-a");
        assert!(!b.exists());
        assert_eq!(dir.files(), ["a.psd"]);
    }

    /// 書いている途中の失敗（取消・予算・確かめの失敗）は、一時ファイルを消し、あったファイルはそのまま。書き込みの途中・確かめの途中のどちらでも。
    #[test]
    fn a_failure_while_writing_or_checking_removes_the_temp_file_and_keeps_the_old_file() {
        use std::io::Write;
        let dir = Dir::new("stage-fail");
        let path = dir.0.join("keep.psd");
        std::fs::write(&path, b"old").unwrap();
        // 大きく書いてから失敗する（書き込みの途中）
        let err = stage(
            &path,
            |out| {
                out.write_all(&vec![7u8; 3 << 20]).map_err(Failure::File)?;
                Err(Failure::Overrun(Overrun::FileSize {
                    layer: "下絵".into(),
                }))
            },
            |_| Ok(()),
        )
        .map(|_| ())
        .unwrap_err();
        assert!(matches!(err, Failure::Overrun(_)), "{err:?}");
        assert_eq!(dir.files(), ["keep.psd"]);
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        // 書き終えてから、確かめで失敗する（確かめの途中の取消も同じ）
        for failure in [
            Failure::Canceled,
            Failure::message("読み戻せません", "Cannot read back"),
        ] {
            let mut failure = Some(failure);
            let err = stage(
                &path,
                |out| out.write_all(b"new").map_err(Failure::File),
                |temp| {
                    assert!(temp.exists(), "確かめるときは一時ファイルがある");
                    Err(failure.take().unwrap())
                },
            )
            .map(|_| ())
            .unwrap_err();
            assert!(
                matches!(err, Failure::Canceled | Failure::Message { .. }),
                "{err:?}"
            );
            assert_eq!(dir.files(), ["keep.psd"]);
            assert_eq!(std::fs::read(&path).unwrap(), b"old");
        }
        // 通れば、置き換えるまでは元のファイルのまま
        let staged = stage(
            &path,
            |out| out.write_all(b"new").map_err(Failure::File),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(staged.len(), 3);
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        commit(vec![staged]).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(dir.files(), ["keep.psd"]);
    }

    /// 評価・焼き込み・圧縮の途中の panic でも、一時ファイルは残らない（巨大になり得る書きかけを書き先のフォルダに残さない）。済んだ分の一時ファイルも同じ。
    #[test]
    fn a_panic_while_writing_removes_every_temp_file() {
        use std::io::Write;
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let dir = Dir::new("stage-panic");
        let [a, b] = ["a", "b"].map(|n| dir.0.join(format!("{n}.psd")));
        std::fs::write(&b, b"old").unwrap();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut staged = vec![stage_bytes(&a, b"new").unwrap()];
            assert_eq!(
                dir.files().len(),
                3,
                "元のファイルと、確かめ済みの一時ファイル 1 つ"
            );
            staged.push(
                stage(
                    &b,
                    |out| {
                        out.write_all(&vec![1u8; 3 << 20]).map_err(Failure::File)?;
                        panic!("レイヤーの評価の途中の panic");
                    },
                    |_| Ok(()),
                )
                .unwrap(),
            );
        }));
        assert!(result.is_err());
        assert_eq!(
            dir.files(),
            ["b.psd"],
            "一時ファイルは全部消え、元のファイルはそのまま"
        );
        assert_eq!(std::fs::read(&b).unwrap(), b"old");
        // 確かめの途中の panic も同じ
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = stage(
                &b,
                |out| out.write_all(b"new").map_err(Failure::File),
                |_| panic!("確かめの途中の panic"),
            );
        }));
        assert!(result.is_err());
        assert_eq!(dir.files(), ["b.psd"]);
        // ほかのプロセスの一時ファイル（同じ形の名前）は自分のものではないので、書いても失敗しても消さない（自分の一時ファイルは
        // プロセスの番号と通し番号で重ならない）
        let name = ".b.psd.1-0.pending~";
        std::fs::write(dir.0.join(name), b"not ours").unwrap();
        let err = stage(
            &b,
            |out| out.write_all(b"new").map_err(Failure::File),
            |_| Err(Failure::Canceled),
        )
        .map(|_| ())
        .unwrap_err();
        assert!(matches!(err, Failure::Canceled), "{err:?}");
        assert_eq!(std::fs::read(dir.0.join(name)).unwrap(), b"not ours");
    }

    /// 書いている途中（先頭を書いた直後）の取消は、一時ファイルを残さず、元のファイルのまま。書き出しの道（`write_psd` を一時ファイルへ）で。
    #[test]
    fn a_cancel_raised_midway_through_streaming_to_the_temp_file_leaves_nothing_behind() {
        use std::io::{Seek, Write};
        /// 書き始めに取消の旗を立てる出力（ファイルの上に重ねる）。
        struct Trip<'a, W: Write + Seek>(&'a mut W, &'a AtomicBool);
        impl<W: Write + Seek> Write for Trip<'_, W> {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                let n = self.0.write(buf)?;
                self.1.store(true, Ordering::Relaxed);
                Ok(n)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                self.0.flush()
            }
        }
        impl<W: Write + Seek> Seek for Trip<'_, W> {
            fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
                self.0.seek(to)
            }
        }
        let dir = Dir::new("stage-cancel");
        let path = dir.0.join("keep.psd");
        std::fs::write(&path, b"old").unwrap();
        let s = painted();
        let cancel = AtomicBool::new(false);
        let ctl = ExportControl {
            cancel: Some(&cancel),
            ..ExportControl::default()
        };
        let plan = psd::plan_export(
            &s.doc,
            &ExportOptions::new(Channel::Color, ExportMode::Bake),
            &ctl,
        )
        .unwrap();
        let err = stage(
            &path,
            |out| {
                assert!(dir.files().len() == 2, "書いている間は一時ファイルがある");
                plan.write_psd(&s.doc, &ctl, &mut Trip(out, &cancel), Compression::Rle)
                    .map(|_| ())
                    .map_err(export_failure)
            },
            |_| panic!("取消のあとは確かめない"),
        )
        .map(|_| ())
        .unwrap_err();
        assert!(matches!(err, Failure::Canceled), "{err:?}");
        assert_eq!(dir.files(), ["keep.psd"]);
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
    }

    /// 書いた PSD を流して読み戻して確かめ、読めなければ置き換えない。理由は画面の言語で出し、内部の名前は出さない。
    #[test]
    fn a_psd_that_does_not_read_back_is_refused_with_a_localized_reason() {
        let dir = Dir::new("verify");
        let s = painted();
        let ctl = ExportControl::default();
        let plan = psd::plan_export(
            &s.doc,
            &ExportOptions::new(Channel::Color, ExportMode::Bake),
            &ctl,
        )
        .unwrap();
        let path = dir.0.join("v.psd");
        let mut file = std::fs::File::create(&path).unwrap();
        let written = plan
            .write_psd(&s.doc, &ctl, &mut file, Compression::Rle)
            .unwrap();
        drop(file);
        let calm = AtomicBool::new(false);
        verify_written(&path, Some(written), &calm).expect("書いた PSD は読み戻せる");
        // 取消は壊れた PSD の断りとは別の種類
        let raised = AtomicBool::new(true);
        let err = verify_written(&path, Some(written), &raised).unwrap_err();
        assert!(matches!(err, Failure::Canceled), "{err:?}");
        // 書いた長さ・レイヤーの数と合わなければ、読み戻しが一致しない
        for wrong in [
            None,
            Some(psd::Written {
                layers: written.layers + 1,
                ..written
            }),
            Some(psd::Written {
                bytes: written.bytes + 1,
                ..written
            }),
        ] {
            let err = verify_written(&path, wrong, &calm).unwrap_err();
            assert!(matches!(err, Failure::Message { .. }), "{err:?}");
        }
        let bytes = std::fs::read(&path).unwrap();
        // 構造を壊さない画素のビット化け（最後の 1 バイトは統合画像の最後の行の画素）は、復号は通るが、書いたバイト列とは違うので置き換えない
        let mut flipped = bytes.clone();
        *flipped.last_mut().unwrap() ^= 1;
        let p = dir.0.join("flipped.psd");
        std::fs::write(&p, &flipped).unwrap();
        psd::verify_stream(
            &mut std::fs::File::open(&p)
                .map(std::io::BufReader::new)
                .unwrap(),
            None,
        )
        .expect("構造は壊れていないので、復号だけでは見つからない");
        let err = verify_written(&p, Some(written), &calm).unwrap_err();
        assert!(matches!(err, Failure::Message { .. }), "{err:?}");
        assert!(err.text(Lang::Ja, false).contains("読み戻しが一致しません"));
        // 大きなドキュメント形式（PSB）・途中で切れた PSD・PSD でないものは読み戻せない
        let mut psb = bytes.clone();
        psb[5] = 2;
        let cut = bytes[..bytes.len() / 2].to_vec();
        for (name, broken) in [("psb", psb), ("cut", cut), ("text", b"not a psd".to_vec())] {
            let p = dir.0.join(format!("{name}.psd"));
            std::fs::write(&p, &broken).unwrap();
            let err = verify_written(&p, Some(written), &calm).unwrap_err();
            assert!(matches!(err, Failure::Unreadable(_)), "{name}: {err:?}");
            let (ja, en) = (err.text(Lang::Ja, false), err.text(Lang::En, false));
            assert!(ja.starts_with("書く PSD を読み戻せません"), "{ja}");
            assert!(en.starts_with("Cannot read back the PSD to write"), "{en}");
            assert!(
                !en.chars().any(|c| c > '\u{2000}'),
                "英語に日本語の診断を出さない: {en}"
            );
        }
    }

    /// 取消・止まった・読み戻せない・途中の失敗は、種類で見分けて、画面の言語の文にする。
    #[test]
    fn failures_are_worded_in_the_screen_language() {
        let canceled = Failure::Canceled;
        assert_eq!(
            canceled.text(Lang::Ja, false),
            "取り消しました（何も書いていません）"
        );
        assert_eq!(
            canceled.text(Lang::En, false),
            "Cancelled (nothing was written)"
        );
        assert_eq!(canceled.text(Lang::En, true), "Cancelled");
        assert_eq!(
            Failure::Stopped.text(Lang::En, false),
            "The PSD job stopped"
        );
        let unreadable = Failure::Unreadable(yolu_io::Error::InvalidData("壊れている".into()));
        assert!(unreadable
            .text(Lang::En, false)
            .starts_with("Cannot read back the PSD to write"));
        assert!(!unreadable.text(Lang::En, false).contains("壊れている"));
        assert!(unreadable.text(Lang::Ja, false).contains("壊れている"));
        let partly = Failure::PartlyReplaced {
            done: 2,
            cause: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };
        assert!(partly.text(Lang::En, false).contains("Access denied"));
        assert!(partly
            .text(Lang::Ja, false)
            .contains("アクセスが拒否されました"));
    }

    /// 予算・形式の上限は、種類から日英の文（レイヤーの名前つき）とツールチップを作る。予算で断るものは設定で上げられることを、PSD の 2 GiB は形式の上限であることを言う。
    #[test]
    fn overruns_are_worded_in_both_languages_with_the_layer_name_and_a_tooltip() {
        let overruns = [
            Overrun::Canvas {
                width: 9000,
                height: 8000,
            },
            Overrun::Side {
                width: 32768,
                height: 16,
                limit: 30000,
            },
            Overrun::Side {
                width: 9000,
                height: 100,
                limit: 8192,
            },
            Overrun::Layers {
                count: 300,
                limit: 256,
            },
            Overrun::Memory {
                layer: "Sketch".into(),
            },
            Overrun::Extra,
            Overrun::FileSize {
                layer: "Paint".into(),
            },
        ];
        let has_japanese = |t: &str| {
            t.chars()
                .any(|c| matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}'))
        };
        for over in &overruns {
            let failure = Failure::Overrun(over.clone());
            let (ja, en) = (failure.text(Lang::Ja, false), failure.text(Lang::En, false));
            assert!(has_japanese(&ja) && !has_japanese(&en), "{ja} / {en}");
            let (tj, te) = (
                failure.tooltip(Lang::Ja).unwrap(),
                failure.tooltip(Lang::En).unwrap(),
            );
            assert!(has_japanese(&tj) && !has_japanese(&te), "{tj} / {te}");
            if over.raised_by_budget() {
                assert!(
                    tj.contains("レイヤーのメモリ") && te.contains("Layer memory"),
                    "{tj} / {te}"
                );
                assert!(te.ends_with("succeed"), "英語の文が終わっている: {te}");
            } else if matches!(over, Overrun::Side { .. }) {
                assert!(
                    tj.contains("30000") && te.contains("30000"),
                    "辺の上限は予算を上げても書けない: {tj} / {te}"
                );
                assert!(
                    !tj.contains("レイヤーのメモリ") && !te.contains("Layer memory"),
                    "{tj} / {te}"
                );
            } else {
                assert!(tj.contains("2 GiB") && te.contains("2 GiB"), "{tj} / {te}");
            }
        }
        let named = Failure::Overrun(Overrun::Memory {
            layer: "法線".into(),
        });
        assert!(named.text(Lang::Ja, false).contains("「法線」"));
        assert!(named.text(Lang::En, false).contains("\"法線\""));
        assert!(Failure::Canceled.tooltip(Lang::Ja).is_none());
    }

    #[test]
    fn it_does_not_start_while_drawing() {
        let dir = Dir::new("busy");
        let big = dir.0.join("big.psd");
        let mut a = painted();
        a.apply(Action::Psd(PsdAction::Export(big.clone())));
        a.wait_psd();
        assert!(big.exists());
        let mut s = AppState::new(32, 32);
        let layer = s.selected_layer.unwrap();
        let brush = s.stroke_settings(false);
        let stroke = s.doc.begin_stroke(layer, &brush).unwrap();
        s.apply(Action::Psd(PsdAction::Import {
            path: big.clone(),
            target: PsdTarget::NewSet,
        }));
        assert!(!s.psd.is_busy());
        assert_eq!(s.message, "描いている間はできません。");
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("never.psd"))));
        assert!(!s.psd.is_busy());
        s.apply(Action::Psd(PsdAction::ExportDialog));
        assert_eq!(s.dialog_request, None);
        s.doc.end_stroke(stroke).unwrap();
        assert_eq!(dir.files(), ["big.psd"]);
        // 描き終えれば始められる
        s.apply(Action::Psd(PsdAction::Import {
            path: big,
            target: PsdTarget::NewSet,
        }));
        assert!(s.psd.is_busy());
        s.wait_psd();
        assert_eq!(s.sets.len(), 2);
    }

    #[test]
    fn canceling_a_read_adds_nothing_and_canceling_a_write_changes_nothing() {
        let dir = Dir::new("cancel");
        let path = dir.0.join("c.psd");
        let mut a = painted();
        a.apply(Action::Psd(PsdAction::Export(path.clone())));
        a.wait_psd();
        let original = std::fs::read(&path).unwrap();
        // 読み込み（新しいセット・今のセット）: 取消が来るまで始めない仕事で、取消が効いたことを必ず確かめる
        for target in [PsdTarget::NewSet, PsdTarget::CurrentSet] {
            let mut s = AppState::new(32, 32);
            let before = (s.sets.len(), s.doc.id(), s.modified);
            s.psd.park_next = true;
            s.apply(Action::Psd(PsdAction::Import {
                path: path.clone(),
                target,
            }));
            assert!(s.psd.is_busy());
            s.apply(Action::Psd(PsdAction::Cancel));
            assert!(s.psd.progress().unwrap().canceling);
            s.wait_psd();
            assert!(s.message.contains("取り消しました"), "{}", s.message);
            assert_eq!((s.sets.len(), s.doc.id(), s.modified), before, "{target:?}");
            assert!(s.psd.report.is_none());
            // 取り消した読み込みは覚えない（同じファイルへ書き出すとき確かめない）
            s.apply(Action::Psd(PsdAction::Export(path.clone())));
            assert!(s.psd.confirm.is_none(), "{target:?}");
            s.wait_psd();
            assert!(path.exists());
            std::fs::write(&path, &original).unwrap();
        }
        // 書き出し: 取り消したら、あったファイルはそのまま
        let mut s = painted();
        s.psd.park_next = true;
        s.apply(Action::Psd(PsdAction::Export(path.clone())));
        assert!(s.psd.is_busy());
        s.apply(Action::Psd(PsdAction::Cancel));
        s.wait_psd();
        assert!(s.message.contains("取り消しました"), "{}", s.message);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(dir.files(), ["c.psd"], "一時ファイルも残らない");
    }

    #[test]
    fn a_current_set_import_that_finishes_while_drawing_is_not_installed_and_the_stroke_stays() {
        let dir = Dir::new("stroke");
        let mut a = painted();
        let path = dir.0.join("d.psd");
        a.apply(Action::Psd(PsdAction::Export(path.clone())));
        a.wait_psd();
        // 今のセットの文書を替える読み込み: 読んでいる間に描き始めて、まだ離していない
        let mut b = AppState::new(32, 32);
        let id = b.doc.id();
        b.modified = false;
        b.apply(Action::Psd(PsdAction::Import {
            path: path.clone(),
            target: PsdTarget::CurrentSet,
        }));
        assert!(b.psd.is_busy());
        let layer = b.selected_layer.unwrap();
        let brush = b.stroke_settings(false);
        let stroke = b.doc.begin_stroke(layer, &brush).unwrap();
        b.wait_psd();
        assert_eq!(b.doc.id(), id, "文書は替えない");
        assert!(
            b.is_stroking(),
            "描きかけのストロークは取り残さず、そのまま"
        );
        assert!(!b.modified);
        assert!(b.message.contains("描いている間"), "{}", b.message);
        assert!(b.psd.report.is_none());
        b.doc.end_stroke(stroke).unwrap();
        assert_eq!((b.doc.width(), b.doc.layers().len()), (32, 1));
        // 取り込めなかったので、同じファイルへ書き出すとき確かめない（取り込んだ記録を残さない）
        b.apply(Action::Psd(PsdAction::Export(path.clone())));
        assert!(b.psd.confirm.is_none());
        b.wait_psd();
        // 描き終えてからなら入る
        b.apply(Action::Psd(PsdAction::Import {
            path: path.clone(),
            target: PsdTarget::CurrentSet,
        }));
        b.wait_psd();
        assert_eq!(
            (b.doc.width(), b.doc.layers().len()),
            (32, 1),
            "書き出した 1 レイヤーの文書"
        );
        // 英語
        let mut c = AppState::new(32, 32);
        c.lang = Lang::En;
        c.apply(Action::Psd(PsdAction::Import {
            path: path.clone(),
            target: PsdTarget::CurrentSet,
        }));
        let layer = c.selected_layer.unwrap();
        let brush = c.stroke_settings(false);
        let stroke = c.doc.begin_stroke(layer, &brush).unwrap();
        c.wait_psd();
        assert!(c.message.contains("while drawing"), "{}", c.message);
        c.doc.end_stroke(stroke).unwrap();
        // 新しいセットとして足す読み込みは足すが、描いている間は切り替えない
        let mut d = AppState::new(32, 32);
        d.apply(Action::Psd(PsdAction::Import {
            path,
            target: PsdTarget::NewSet,
        }));
        let layer = d.selected_layer.unwrap();
        let brush = d.stroke_settings(false);
        let stroke = d.doc.begin_stroke(layer, &brush).unwrap();
        d.wait_psd();
        assert_eq!(d.sets.len(), 2);
        assert_eq!(d.sets.current_index(), 0, "描いている間は切り替えない");
        assert!(d.is_stroking());
        assert!(d.message.contains("切り替えていません"), "{}", d.message);
        d.doc.end_stroke(stroke).unwrap();
    }

    /// Roughness にも描いた 64 × 64 の状態（2 枚のレイヤー）。
    fn painted_in_two_channels() -> AppState {
        let mut s = painted();
        let layers: Vec<LayerId> = s.doc.layers().iter().map(|l| l.id()).collect();
        for (i, id) in layers.iter().enumerate() {
            s.doc
                .set_channel_enabled(*id, Channel::Roughness, true)
                .unwrap();
            paint_channel(
                &mut s.doc,
                *id,
                Channel::Roughness,
                48,
                32,
                [60 + 40 * i as u8, 90, 130, 255],
            );
        }
        s
    }

    fn paint_channel(
        doc: &mut Document,
        layer: LayerId,
        channel: Channel,
        w: u32,
        h: u32,
        rgba: [u8; 4],
    ) {
        let ts = doc.tile_size();
        for ty in 0..h.div_ceil(ts) {
            for tx in 0..w.div_ceil(ts) {
                let mut tile = vec![0u8; (ts * ts * 4) as usize];
                for y in 0..ts.min(h - ty * ts) {
                    for x in 0..ts.min(w - tx * ts) {
                        tile[((y * ts + x) * 4) as usize..][..4].copy_from_slice(&rgba);
                    }
                }
                doc.import_tile(layer, channel, TileCoord::new(tx, ty), &tile)
                    .unwrap();
            }
        }
    }

    /// 複数のチャンネルは `<名前>_<チャンネル>.psd` を並べて書き、それぞれを取り込むと、そのチャンネルの今の合成と同じ。もうあるファイルは
    /// 置き換える前に確かめ、やめれば何も変えない。
    #[test]
    fn several_channels_write_one_psd_each_that_composites_like_that_channel() {
        let dir = Dir::new("channels");
        let mut s = painted_in_two_channels();
        let rough = s
            .doc
            .composite_channel(Channel::Roughness, s.doc.bounds())
            .unwrap();
        let color = composite(&s);
        s.psd.export.channels = vec![Channel::Color, Channel::Roughness, Channel::Height];
        let before = yolu_io::NativeDocument::from_core(&s.doc)
            .unwrap()
            .to_bytes();
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("Body.psd"))));
        s.wait_psd();
        assert!(s.psd.notes_confirm.is_none() && s.psd.confirm.is_none());
        assert_eq!(
            dir.files(),
            ["Body_Color.psd", "Body_Height.psd", "Body_Roughness.psd"]
        );
        assert!(s.message.contains("3 個"), "{}", s.message);
        for (name, expected) in [("Body_Color.psd", &color), ("Body_Roughness.psd", &rough)] {
            let mut t = AppState::new(32, 32);
            t.apply(Action::Psd(PsdAction::Import {
                path: dir.0.join(name),
                target: PsdTarget::CurrentSet,
            }));
            t.wait_psd();
            assert_eq!(&composite(&t), expected, "{name}");
        }
        assert_eq!(
            yolu_io::NativeDocument::from_core(&s.doc)
                .unwrap()
                .to_bytes(),
            before,
            "正本のバイトは変わらない"
        );
        // もうあるファイル: 確かめる（複数のチャンネルは、選んだ名前でなく書き分けた名前を書く）
        let original = std::fs::read(dir.0.join("Body_Roughness.psd")).unwrap();
        paint_channel(
            &mut s.doc,
            s.selected_layer.unwrap(),
            Channel::Roughness,
            8,
            8,
            [1, 2, 3, 255],
        );
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("Body.psd"))));
        assert!(!s.psd.is_busy());
        let replace = s.psd.confirm.as_ref().expect("確かめ");
        assert_eq!(replace.files.len(), 3);
        assert!(!replace.imported);
        s.apply(Action::Psd(PsdAction::CancelConfirm));
        assert_eq!(
            std::fs::read(dir.0.join("Body_Roughness.psd")).unwrap(),
            original
        );
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("Body.psd"))));
        s.apply(Action::Psd(PsdAction::ConfirmReplace));
        s.wait_psd();
        assert_ne!(
            std::fs::read(dir.0.join("Body_Roughness.psd")).unwrap(),
            original
        );
        assert_eq!(
            dir.files().len(),
            3,
            "一時ファイルは残らない: {:?}",
            dir.files()
        );
        // 1 つのチャンネルは選んだファイルそのまま
        s.psd.export.channels = vec![Channel::Height];
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("one.psd"))));
        s.wait_psd();
        assert!(dir.0.join("one.psd").exists());
        // 書き分けた名前が重なるユーザーチャンネルは、書く前に断る
        let info = |name: &str| yolu_core::ChannelInfo {
            name: name.into(),
            kind: yolu_core::ChannelKind::Scalar,
            color_space: yolu_core::ColorSpace::Linear,
            default: yolu_core::Rgba8::new(0, 0, 0, 255),
        };
        let a = s.doc.add_channel(info("Same")).unwrap();
        let b = s.doc.add_channel(info("same")).unwrap();
        s.psd.export.channels = vec![a, b];
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("clash.psd"))));
        assert!(!s.psd.is_busy());
        assert!(s.message.contains("重なります"), "{}", s.message);
        assert_eq!(dir.files().len(), 4);
    }

    /// 設定のウィンドウの操作: 最後の 1 つのチャンネルは外せず、消えたチャンネルは次に開くときに外れる。複数チャンネルの確かめは、チャンネルごとに並ぶ。
    #[test]
    fn the_export_settings_keep_one_channel_and_the_notes_are_listed_per_channel() {
        let dir = Dir::new("settings");
        let mut s = painted_in_two_channels();
        assert_eq!(s.psd.export, ExportSettings::default());
        s.apply(Action::Psd(PsdAction::ToggleExportChannel(Channel::Height)));
        s.apply(Action::Psd(PsdAction::ToggleExportChannel(
            Channel::Roughness,
        )));
        assert_eq!(
            s.psd.export.channels,
            [Channel::Color, Channel::Roughness, Channel::Height],
            "番号の順"
        );
        for c in [Channel::Color, Channel::Roughness, Channel::Height] {
            s.apply(Action::Psd(PsdAction::ToggleExportChannel(c)));
        }
        assert_eq!(
            s.psd.export.channels,
            [Channel::Height],
            "最後の 1 つは外せない"
        );
        s.apply(Action::Psd(PsdAction::SetExportMode(ExportMode::Flat)));
        assert_eq!(s.psd.export.mode, ExportMode::Flat);
        // 消したユーザーチャンネルは、開くときに外れる（空なら Color）
        let user = s.doc.add_channel(yolu_core::ChannelInfo {
            name: "X".into(),
            kind: yolu_core::ChannelKind::Scalar,
            color_space: yolu_core::ColorSpace::Linear,
            default: yolu_core::Rgba8::new(0, 0, 0, 255),
        });
        let user = user.unwrap();
        s.psd.export.channels = vec![user];
        s.doc.remove_channel(user).unwrap();
        s.apply(Action::Psd(PsdAction::ExportDialog));
        assert_eq!(s.psd.export.channels, [Channel::Color]);
        // 平らに 1 枚: 確かめは出ず、1 枚のレイヤーの PSD
        s.psd.options_open = false;
        s.psd.export.channels = vec![Channel::Color, Channel::Roughness];
        let layer = s.selected_layer.unwrap();
        s.apply(Action::M2(crate::m2::Edit::AddMask(layer)));
        s.doc.set_layer_mask_inverted(layer, true).unwrap();
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("flat.psd"))));
        s.wait_psd();
        assert!(s.psd.notes_confirm.is_none());
        let mut t = AppState::new(32, 32);
        t.apply(Action::Psd(PsdAction::Import {
            path: dir.0.join("flat_Roughness.psd"),
            target: PsdTarget::CurrentSet,
        }));
        t.wait_psd();
        assert_eq!(t.doc.layers().len(), 1);
        // 焼き込み: 反転したマスクが 2 つのチャンネルの確かめに並ぶ
        s.apply(Action::Psd(PsdAction::SetExportMode(ExportMode::Bake)));
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("both.psd"))));
        s.wait_psd();
        let confirm = s.psd.notes_confirm.as_ref().expect("確かめ");
        let sections = confirm.sections();
        assert_eq!(
            sections
                .iter()
                .map(|(c, n)| (*c, n.len()))
                .collect::<Vec<_>>(),
            [(Channel::Color, 1), (Channel::Roughness, 1)]
        );
        s.apply(Action::Psd(PsdAction::CancelWrite));
        assert!(!dir.0.join("both_Color.psd").exists());
    }

    /// 書く仕事を取り消したら、ファイルは何も変わらず、一時ファイルも残らない（計画と確かめのあとに止めた場合）。
    #[test]
    fn canceling_the_write_after_the_check_changes_no_file_and_leaves_no_temp_file() {
        let dir = Dir::new("write-cancel");
        let path = dir.0.join("w.psd");
        std::fs::write(&path, b"old").unwrap();
        let mut s = painted();
        let layer = s.selected_layer.unwrap();
        s.apply(Action::M2(crate::m2::Edit::AddMask(layer)));
        s.doc.set_layer_mask_inverted(layer, true).unwrap();
        s.apply(Action::Psd(PsdAction::Export(path.clone())));
        s.wait_psd();
        assert!(s.psd.notes_confirm.is_some());
        s.psd.park_write = true;
        s.apply(Action::Psd(PsdAction::ConfirmWrite));
        assert!(s.psd.is_busy());
        s.apply(Action::Psd(PsdAction::Cancel));
        s.wait_psd();
        assert!(s.message.contains("取り消しました"), "{}", s.message);
        assert!(s.psd.report.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(dir.files(), ["w.psd"]);
    }

    /// 焼いたレイヤーの画素の合計が、以前の固定の上限（128 MiB）を超えても書ける。レイヤーは 1 枚ずつ流して書き、RLE で圧縮するので、メモリにはレイヤー 1 枚ぶんだけで、
    /// ファイルは小さい。書いたファイルを読み込み直すと、レイヤーの数も合成も同じ。
    #[test]
    fn baked_layers_beyond_the_old_fixed_budget_are_written_streamed_and_compressed() {
        let dir = Dir::new("bake-budget");
        let mut s = AppState::new(4096, 4096);
        for n in 0..3 {
            s.doc
                .add_fill_layer(
                    &format!("ガラス{n}"),
                    &[(
                        Channel::Color,
                        yolu_core::Rgba8::new(1 + n as u8, 2, 3, 100),
                    )],
                    None,
                )
                .unwrap();
        }
        let path = dir.0.join("big.psd");
        s.apply(Action::Psd(PsdAction::Export(path.clone())));
        s.wait_psd();
        assert_eq!(
            s.psd.notes_confirm.as_ref().unwrap().sections()[0].1.len(),
            3
        );
        let layers = s.doc.layers().len();
        s.apply(Action::Psd(PsdAction::ConfirmWrite));
        s.wait_psd();
        assert!(s.message.contains("PSD に書き出しました"), "{}", s.message);
        assert!(s.psd.report.is_none());
        assert_eq!(dir.files(), ["big.psd"], "一時ファイルは残らない");
        // 全面の 4096² のレイヤー 3 枚 + 統合画像は無圧縮なら 256 MiB。RLE で小さくなっている
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size < 8 * 1024 * 1024, "{size}");
        // 読み込み直すと、レイヤーの数も合成も同じ
        let mut back = AppState::new(32, 32);
        back.apply(Action::Psd(PsdAction::Import {
            path,
            target: PsdTarget::NewSet,
        }));
        back.wait_psd();
        assert_eq!(back.sets.len(), 2, "{}", back.message);
        assert_eq!(back.doc.layers().len(), layers);
        let rect = yolu_core::Rect::new(0, 0, 4096, 8);
        assert_eq!(
            back.doc.composite(rect).unwrap(),
            s.doc.composite(rect).unwrap()
        );
    }

    /// レイヤーの記録（グループは区切りも数える）が設定の予算から決まる上限を超えるときは、計画のあとの確かめでなく書き始めてから分かる。理由を結果のウィンドウに出し
    /// （日英・設定で上げられることはツールチップで）、何も書かず、一時ファイルも残さない。
    #[test]
    fn layer_records_over_the_budget_are_refused_with_the_reason_and_the_hint_and_nothing_is_left()
    {
        let dir = Dir::new("records");
        let mut s = AppState::new(16, 16);
        // 設定の予算 256 MiB のレイヤーの記録は 256 件まで。初めのレイヤー 1 枚とグループ 130 個は、区切りの記録も数えて 261 件になる
        s.prefs.settings.source_budget = crate::settings::Budget::Mib(256);
        for n in 0..130 {
            s.doc.add_group(&format!("グループ{n}"), None).unwrap();
        }
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("g.psd"))));
        s.wait_psd();
        assert!(
            s.message
                .contains("レイヤーが多すぎます（261 件・上限 256 件）"),
            "{}",
            s.message
        );
        let report = s.psd.report.take().expect("理由のウィンドウ");
        assert!(!report.ok && !report.importing);
        assert!(report.lines[0].warning);
        assert!(
            report.lines[0]
                .tooltip
                .as_deref()
                .is_some_and(|t| t.contains("レイヤーのメモリ") && t.contains("区切り")),
            "{:?}",
            report.lines[0].tooltip
        );
        assert!(
            dir.files().is_empty(),
            "書いていない・一時ファイルも無い: {:?}",
            dir.files()
        );
        s.lang = Lang::En;
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("g.psd"))));
        s.wait_psd();
        assert!(
            s.message.contains("Too many layers (261; limit 256)"),
            "{}",
            s.message
        );
        let report = s.psd.report.take().unwrap();
        assert!(report.lines[0]
            .tooltip
            .as_deref()
            .unwrap()
            .contains("Layer memory"));
        assert!(dir.files().is_empty());
        // 予算を上げれば書ける
        s.prefs.settings.source_budget = crate::settings::Budget::Mib(512);
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("g.psd"))));
        s.wait_psd();
        assert_eq!(dir.files(), ["g.psd"], "{}", s.message);
    }

    /// 書けないもの（Normal の DirectX 向きのレベル補正）は、確認のウィンドウでなく理由のウィンドウに出し、何も書かない。複数チャンネルでは、チャンネルの名前つき。
    #[test]
    fn what_cannot_be_baked_is_refused_with_the_channel_and_nothing_is_written() {
        use yolu_core::{AdjustmentSettings, NormalSettings, NormalYDirection};
        let dir = Dir::new("hard");
        let mut s = painted();
        s.doc
            .set_normal_settings(
                NormalSettings::default().with_file_direction(NormalYDirection::DirectX),
                false,
            )
            .unwrap();
        s.doc
            .add_adjustment_layer(
                "レベル",
                AdjustmentSettings::levels(20.0 / 255.0, 230.0 / 255.0, 1.37, 0.0, 1.0).unwrap(),
                None,
                None,
            )
            .unwrap();
        s.psd.export.channels = vec![Channel::Color, Channel::Normal];
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("n.psd"))));
        s.wait_psd();
        assert!(s.psd.notes_confirm.is_none());
        let report = s.psd.report.take().expect("理由のウィンドウ");
        assert!(!report.ok);
        assert!(
            report.lines[0].text.contains("ノーマル") && report.lines[0].text.contains("レベル"),
            "{:?}",
            report.lines
        );
        assert!(dir.files().is_empty());
        s.lang = Lang::En;
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("n.psd"))));
        s.wait_psd();
        let report = s.psd.report.take().expect("理由のウィンドウ");
        assert!(
            report.lines[0].text.starts_with("Normal: "),
            "{:?}",
            report.lines
        );
    }

    /// 書けない場所（フォルダが無い・通常のファイルでない先）は、理由を結果のウィンドウに出して何も変えない。複数のチャンネルの途中で書けなくても、
    /// 先に書いた分は置き換えず、一時ファイルも残さない。
    #[test]
    fn a_place_that_cannot_be_written_says_why_and_replaces_no_file() {
        let dir = Dir::new("unwritable");
        let mut s = painted_in_two_channels();
        s.apply(Action::Psd(PsdAction::Export(
            dir.0.join("missing").join("a.psd"),
        )));
        s.wait_psd();
        assert!(s.message.contains("書き出せません"), "{}", s.message);
        let report = s.psd.report.take().expect("理由のウィンドウ");
        assert!(!report.ok && !report.importing && report.lines[0].warning);
        assert!(dir.files().is_empty());
        // 2 つ目の先がフォルダ: 1 つ目も置き換えない
        s.psd.export.channels = vec![Channel::Color, Channel::Roughness];
        std::fs::write(dir.0.join("Body_Color.psd"), b"old").unwrap();
        std::fs::create_dir(dir.0.join("Body_Roughness.psd")).unwrap();
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("Body.psd"))));
        s.apply(Action::Psd(PsdAction::ConfirmReplace));
        s.wait_psd();
        let report = s.psd.report.take().expect("理由のウィンドウ");
        assert!(
            report.lines[0]
                .text
                .contains("通常のファイルではありません"),
            "{:?}",
            report.lines
        );
        assert_eq!(std::fs::read(dir.0.join("Body_Color.psd")).unwrap(), b"old");
        assert_eq!(dir.files(), ["Body_Color.psd", "Body_Roughness.psd"]);
    }

    #[test]
    fn a_canvas_over_a_limit_is_refused_with_the_reason_and_the_hint_where_the_budget_helps_and_changes_nothing(
    ) {
        let dir = Dir::new("budget");
        let mut s = AppState::new(32, 32);
        // レイヤーの画素の予算（設定の「レイヤーのメモリ」）を下げる。読み込みは 256 MiB を下回らない
        s.prefs.settings.source_budget = crate::settings::Budget::Mib(16);
        let before = (s.sets.len(), s.doc.id(), s.modified);
        // ファイルの大きさだけでは断らない（原本を保たないので、保持の上限は掛けない）。PSD でなければ、その理由で断る
        let huge = dir.0.join("huge.psd");
        let limit = Limits::default().max_source_bytes as u64;
        std::fs::File::create(&huge)
            .unwrap()
            .set_len(limit + 1)
            .unwrap();
        s.apply(Action::Psd(PsdAction::Import {
            path: huge,
            target: PsdTarget::NewSet,
        }));
        s.wait_psd();
        assert!(!s.message.contains("MiB"), "{}", s.message);
        assert!(s.psd.report.take().is_some_and(|r| !r.ok));
        // 読み込みのキャンバスが .ylp の辺の上限を超える（幅・高さを書き換えたファイル）
        let mut wide = {
            let projected = psd::Document::from_core(&painted().doc).unwrap();
            psd::write(&projected, &Limits::default()).unwrap()
        };
        wide[14..18].copy_from_slice(&9000u32.to_be_bytes());
        wide[18..22].copy_from_slice(&9000u32.to_be_bytes());
        std::fs::write(dir.0.join("wide.psd"), &wide).unwrap();
        s.apply(Action::Psd(PsdAction::Import {
            path: dir.0.join("wide.psd"),
            target: PsdTarget::CurrentSet,
        }));
        s.wait_psd();
        assert!(
            s.message.contains("キャンバスが大きすぎます"),
            "{}",
            s.message
        );
        let report = s.psd.report.take().expect("理由のウィンドウ");
        assert!(!report.ok && report.importing);
        assert!(report.lines[0].warning && report.lines[0].text.contains("9000×9000"));
        // 辺が .ylp の上限（8192）を超える: 設定の予算を上げても取り込めないので、予算の案内は出さない
        assert!(
            report.lines[0].text.contains("8192"),
            "{}",
            report.lines[0].text
        );
        assert!(
            report.lines[0].tooltip.is_none(),
            "{:?}",
            report.lines[0].tooltip
        );
        assert_eq!((s.sets.len(), s.doc.id(), s.modified), before);
        // 英語
        s.lang = Lang::En;
        s.apply(Action::Psd(PsdAction::Import {
            path: dir.0.join("wide.psd"),
            target: PsdTarget::CurrentSet,
        }));
        s.wait_psd();
        assert!(s.message.contains("Canvas too large"), "{}", s.message);
        let report = s.psd.report.take().unwrap();
        assert!(
            report.lines[0].text.contains("limit 8192"),
            "{}",
            report.lines[0].text
        );
        assert!(report.lines[0].tooltip.is_none());
        // 書き出しのキャンバス（設定の予算 256 MiB に入らない 9000²）: 何も作らず、一時ファイルも作らない。理由は日英で、設定で上げられることはツールチップで
        let mut big = AppState::new(9000, 9000);
        big.prefs.settings.source_budget = crate::settings::Budget::Mib(256);
        big.apply(Action::Psd(PsdAction::Export(dir.0.join("big.psd"))));
        assert!(!big.psd.is_busy());
        assert!(
            big.message.contains("キャンバスが大きすぎます"),
            "{}",
            big.message
        );
        let report = big.psd.report.take().expect("理由のウィンドウ");
        assert!(!report.ok && !report.importing);
        assert!(report.lines[0].warning && report.lines[0].text.contains("9000×9000"));
        assert!(
            report.lines[0]
                .tooltip
                .as_deref()
                .is_some_and(|t| t.contains("レイヤーのメモリ")),
            "{:?}",
            report.lines[0].tooltip
        );
        big.lang = Lang::En;
        big.apply(Action::Psd(PsdAction::Export(dir.0.join("big.psd"))));
        assert!(big.message.contains("Canvas too large"), "{}", big.message);
        let report = big.psd.report.take().unwrap();
        assert!(report.lines[0]
            .tooltip
            .as_deref()
            .unwrap()
            .contains("Layer memory"));
        assert_eq!(dir.files(), ["huge.psd", "wide.psd"], "書いていない");
    }

    /// PSD を今のセットへ入れると、前の文書を指す画面の途中の状態を戻し、表示も既定に戻す（大きさが変わりうる）。
    #[test]
    fn importing_into_the_current_set_settles_the_ui_and_resets_the_view() {
        use crate::sets::install_testing::{assert_settled, stir, zoom};
        let mut s = AppState::new(32, 32);
        zoom(&mut s);
        stir(&mut s);
        let (doc, _) = crate::state::blank_document(64, 64);
        assert!(s.install_psd(doc, PsdTarget::CurrentSet, "a.psd"));
        assert_settled(&s, "PSD を今のセットへ");
        assert_eq!(s.view, crate::canvas::view::ViewState::default());
        assert_eq!(s.ui.layer_scroll, 0.0);
        assert!(s.selected_layer.is_some());
        assert!(s.modified);
    }
}
