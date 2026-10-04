//! PSD の読み込みと書き出し（RGB8 の PSD。Unity 版の `ImportPsd`・`ExportPsd` と同じ考え方）。コーデックは yolu-io の `psd`。
//!
//! - **読み込み**（新しいテクスチャセットか、今のセットの文書として）: 別のスレッドで読む（ファイルを読む・`psd::read`・core の文書への変換）。
//!   編集できる（`EditableRaster`）うえ core の文書に変えられるものだけを入れる。グループ（入れ子・通過/分離）・塗りつぶし（単色）・調整
//!   （反転・レベル補正・色相/彩度）・マスク・クリッピングは core の層になる。**原本を保つだけ（`PreserveOnly`）・拒否（`Rejected`）・
//!   core で扱えない中身（キャンバス外の画素）は、何も変えずに理由（診断の一覧）を窓で見せる**。層のロック（lspf）は core の層のロックとして入る。
//!   名前だけでレイヤーを結び付けない（`to_core` は ID で扱う）。PSD の原本は書き換えない。今のセットの文書を替える読み込みは、
//!   読み終わったときに描いている最中か、読んでいる間に文書が変わっていれば入れない（描きかけのストロークを取り残さず、描いたものを黙って捨てない）。
//! - **書き出し**（今の文書）: チャンネルごとに 1 つの PSD を書く（窓で方式とチャンネルを選ぶ。既定は Color だけを「焼き込んで書く」）。
//!   ラスター・グループ・単色の塗りつぶし・調整・クリッピング・マスク（有効/無効・濃度）・層のロックは PSD の形で書き、PSD に形の無いもの
//!   （フィルター・Generator・画像・パス・反転したマスク・半透明の塗りつぶし・クリッピングされたグループなど）は、評価した画素にして書く・
//!   刻みへ丸める・落とすのどれかにして、書く前の確かめの窓に層の名前つきで全部並べる（利用者が「書く」を押すまで何も書かない。黙って捨てない）。
//!   文書は 1 バイトも変えない（効果は文書に残る。書き出しは写し）。始めるときに文書の写し（履歴の無い、タイルを共有する写し）を取り、
//!   計画・焼き込み・書き込みは別のスレッドで行う: 計画（何を焼くか）→ 確かめ → 構築と一時ファイルへの書き込み・読み戻しの確かめ → 最後に
//!   置き換え。取り込んだ PSD と同じファイル・複数のチャンネルで名前が重なるファイルは、置き換える前に確かめる。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

use egui::Vec2;
use yolu_core::{Channel, Document};
use yolu_io::psd::{
    self, CompatibilityMode, Diagnostic, ExportControl, ExportMode, ExportNote, ExportOptions,
    ExportPlan, Limits,
};

use crate::lang::Lang;
use crate::psd_export::blocker_text;
use crate::sets::{guid_string, unique_name, MaterialRef};
use crate::state::{AppState, DialogRequest};

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
    /// 読み込む PSD を選ぶ窓を頼む。
    ImportDialog(PsdTarget),
    /// PSD を読み込む（別のスレッド）。
    Import { path: PathBuf, target: PsdTarget },
    /// 書き出しの設定の窓（方式・チャンネル）を開く。
    ExportDialog,
    /// 設定の窓で方式を選ぶ。
    SetExportMode(ExportMode),
    /// 設定の窓でチャンネルを入れる・外す（最後の 1 つは外せない）。
    ToggleExportChannel(Channel),
    /// 設定の窓の「やめる」。
    CancelExportOptions,
    /// 設定の窓の「書き出し…」: 書き出す先を選ぶ窓を頼む（窓は閉じる）。
    ChooseExportFile,
    /// 今の文書を、選んだ設定で PSD に書き出す（先のファイルが決まった）。複数のチャンネルでは `<名前>_<チャンネル>.psd` を並べて書く。
    Export(PathBuf),
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
    /// 結果の窓を閉じる。
    DismissReport,
}

/// 結果の窓の 1 行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// 注意（読み込めない理由・見え方が変わる所）か、お知らせか。
    pub warning: bool,
    pub text: String,
}

/// 読み書きの結果（窓に出す）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub importing: bool,
    pub file: String,
    /// 読み込めた・書けた。
    pub ok: bool,
    /// 窓の見出しの下の 1 行（名前・状態・短い理由）。
    pub summary: String,
    pub lines: Vec<Line>,
}

enum Output {
    Imported {
        doc: Box<Document>,
        diagnostics: Vec<Diagnostic>,
    },
    Refused {
        mode: Option<CompatibilityMode>,
        reason: String,
        diagnostics: Vec<Diagnostic>,
    },
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
    /// 書く PSD を読み戻したが、読めなかった。
    Unreadable(yolu_io::Error),
    /// 書く PSD を読み戻したが、編集できる PSD として読めなかった（原本の保持のみ・拒否・診断つき）。
    NotEditable(CompatibilityMode),
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

    /// 窓と状態の帯に出す理由。`importing` は取消のとき「何も書いていません」を付けるかの違いだけ。
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
            Self::Unreadable(e) => lang.pick(
                format!("書く PSD を読み戻せません: {}", lang.io_error(e)),
                format!("Cannot read back the PSD to write: {}", lang.io_error(e)),
            ),
            Self::NotEditable(mode) => {
                let name = mode_name(lang, *mode);
                lang.pick(
                    format!("書く PSD を編集できる PSD として読み戻せません（{name}）"),
                    format!("The PSD to write does not read back as an editable PSD ({name})"),
                )
            }
            Self::PartlyReplaced { done, cause } => {
                let cause = lang.file_error(cause);
                lang.pick(
                    format!(
                        "置き換えの途中で失敗しました（先の {done} 個は置き換わっています）: {cause}"
                    ),
                    format!(
                        "Failed partway through replacing ({done} earlier file(s) already replaced): {cause}"
                    ),
                )
            }
            Self::Message { ja, en } => lang.pick(ja.clone(), en.clone()),
        }
    }
}

/// 読み戻した PSD の扱いの名前（編集できない・診断つきの理由に出す）。
fn mode_name(lang: Lang, mode: CompatibilityMode) -> &'static str {
    match mode {
        CompatibilityMode::PreserveOnly => lang.pick("原本の保持のみ", "Preserve only"),
        CompatibilityMode::Rejected => lang.pick("拒否", "Rejected"),
        CompatibilityMode::EditableRaster => lang.pick("診断あり", "Has diagnostics"),
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
    cancel: Arc<AtomicBool>,
    rx: Receiver<Result<Output, Failure>>,
    /// 今のセットの文書を替える読み込みが始まったときの、セットの uid・文書の ID・版（読んでいる間に変わっていたら入れない）。
    guard: Option<(u32, u128, u64)>,
}

/// 書き出しの設定（窓で選ぶ）。
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
    /// 選んだファイルの名前（札・窓の見出しの下）。
    file: String,
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
    /// 書き出しの設定と、その窓。
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

    pub fn progress(&self) -> Option<Progress> {
        let job = self.job.as_ref()?;
        Some(Progress {
            importing: matches!(job.kind, Kind::Import(_)),
            file: job.file.clone(),
            canceling: job.cancel.load(Ordering::Relaxed),
        })
    }
}

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

/// 診断の行（多ければ初めの `MAX_LINES` 件と数）。
const MAX_LINES: usize = 40;

fn diagnostic_lines(lang: Lang, diagnostics: &[Diagnostic]) -> Vec<Line> {
    let mut lines: Vec<Line> = diagnostics
        .iter()
        .take(MAX_LINES)
        .map(|d| Line {
            warning: !d.is_informational(),
            text: format!("{}: {}", d.code, d.message),
        })
        .collect();
    if diagnostics.len() > MAX_LINES {
        lines.push(Line {
            warning: false,
            text: lang.pick(
                format!("ほか {} 件", diagnostics.len() - MAX_LINES),
                format!("and {} more", diagnostics.len() - MAX_LINES),
            ),
        });
    }
    lines
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

/// 書き出す先を選ぶ窓に出す初めのファイル名（1 つのチャンネルが Color 以外なら末尾にチャンネルの名前）。
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
        let refuse = |s: &mut AppState| {
            s.message = lang
                .pick("描いている間はできません。", "Not while drawing.")
                .into()
        };
        match action {
            PsdAction::ImportDialog(target) => {
                if stroking {
                    return refuse(self);
                }
                if target == PsdTarget::CurrentSet {
                    if let Some(reason) = self.read_only_reason() {
                        self.message = format!(
                            "{}: {reason}",
                            lang.pick("読むだけのテクスチャセットです", "Read-only texture set")
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
                    self.message = format!(
                        "{}: {reason}",
                        lang.pick("読むだけのテクスチャセットです", "Read-only texture set")
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
            PsdAction::ConfirmReplace => {
                if let Some(replace) = self.psd.confirm.take() {
                    self.start_psd_export(&replace.path, true);
                }
            }
            PsdAction::CancelConfirm => {
                if self.psd.confirm.take().is_some() {
                    self.message = lang
                        .pick("PSD の書き出しをやめました。", "PSD export canceled.")
                        .into();
                }
            }
            PsdAction::ConfirmWrite => {
                if let Some(confirm) = self.psd.notes_confirm.take() {
                    self.start_psd_write(confirm.run);
                }
            }
            PsdAction::CancelWrite => {
                if self.psd.notes_confirm.take().is_some() {
                    self.message = lang
                        .pick("PSD の書き出しをやめました。", "PSD export canceled.")
                        .into();
                }
            }
            PsdAction::Cancel => {
                if let Some(job) = &self.psd.job {
                    job.cancel.store(true, Ordering::Relaxed);
                    self.message = lang
                        .pick("PSD の処理を取り消しています…", "Canceling the PSD job…")
                        .into();
                }
            }
            PsdAction::DismissReport => self.psd.report = None,
        }
    }

    fn start_psd_import(&mut self, path: &Path, target: PsdTarget) {
        let lang = self.lang;
        if self.psd.job.is_some() {
            self.message = lang
                .pick("PSD を処理中です。", "A PSD job is running.")
                .into();
            return;
        }
        if target == PsdTarget::CurrentSet {
            if let Some(reason) = self.read_only_reason() {
                self.message = format!(
                    "{}: {reason}",
                    lang.pick("読むだけのテクスチャセットです", "Read-only texture set")
                );
                return;
            }
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (flag, path_owned) = (cancel.clone(), path.to_path_buf());
        let park = std::mem::take(&mut self.psd.park_next);
        let spawned = std::thread::Builder::new()
            .name("yolu-psd-import".into())
            .spawn(move || {
                if park {
                    crate::windows::park_until_canceled(&flag);
                }
                let _ = tx.send(import_worker(&path_owned, &flag));
            });
        if let Err(e) = spawned {
            self.message = e.to_string();
            return;
        }
        let file = file_name(path);
        self.message = lang.pick(
            format!("PSD を読み込み中: {file}"),
            format!("Reading PSD: {file}"),
        );
        let guard = (target == PsdTarget::CurrentSet)
            .then(|| (self.sets.current().uid, self.doc.id(), self.doc.revision()));
        self.psd.job = Some(Job {
            kind: Kind::Import(target),
            file,
            cancel,
            rx,
            guard,
        });
        // 取り込んだ場所を覚える（同じファイルへ書き出すときに確かめる）
        self.psd.imported.push(path.to_path_buf());
    }

    /// 書き出せない理由を窓（結果の窓）に並べ、何も書かない。
    fn refuse_psd_export(&mut self, file: &str, why: Vec<String>) {
        let lang = self.lang;
        self.message = format!(
            "{}: {}",
            lang.pick("PSD に書き出せません", "Cannot export PSD"),
            why.first().cloned().unwrap_or_default()
        );
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
            lines: why
                .into_iter()
                .map(|text| Line {
                    warning: true,
                    text,
                })
                .collect(),
        });
    }

    fn start_psd_export(&mut self, path: &Path, confirmed: bool) {
        let lang = self.lang;
        if self.psd.job.is_some() {
            self.message = lang
                .pick("PSD を処理中です。", "A PSD job is running.")
                .into();
            return;
        }
        let file = file_name(path);
        if let Some(reason) = self.read_only_reason() {
            let why = vec![lang.pick(
                format!("読むだけのテクスチャセットです: {reason}"),
                format!("Read-only texture set: {reason}"),
            )];
            return self.refuse_psd_export(&file, why);
        }
        let channels = export_channels(&self.doc, &self.psd.export);
        let mode = self.psd.export.mode;
        let targets = match export_targets(&self.doc, path, &channels) {
            Ok(t) => t,
            Err(name) => {
                let why = vec![lang.pick(
                    format!("チャンネルのファイル名が重なります: {name}"),
                    format!("Channel file names clash: {name}"),
                )];
                return self.refuse_psd_export(&file, why);
            }
        };
        // 文書そのものが書き出せるか（画布・層の数の予算）を、何も作らずに断る
        for (channel, _) in &targets {
            if let Err(e) = psd::check_exportable(&self.doc, *channel) {
                return self.refuse_psd_export(&file, vec![lang.io_error(&e)]);
            }
        }
        // 置き換えるファイル: 取り込んだ PSD と、複数のチャンネルで書き分けた名前のうちもうあるもの（選ぶ窓が確かめたのは選んだ名前だけ）
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
            Err(e) => return self.refuse_psd_export(&file, vec![lang.core_error(&e)]),
        };
        self.psd.notes_confirm = None;
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let flag = cancel.clone();
        let park = std::mem::take(&mut self.psd.park_next);
        let name = file.clone();
        let spawned = std::thread::Builder::new()
            .name("yolu-psd-plan".into())
            .spawn(move || {
                if park {
                    crate::windows::park_until_canceled(&flag);
                }
                let _ = tx.send(plan_worker(snapshot, mode, targets, name, &flag));
            });
        if let Err(e) = spawned {
            self.message = e.to_string();
            return;
        }
        self.message = lang.pick(
            format!("PSD に書き出し中: {file}"),
            format!("Writing PSD: {file}"),
        );
        self.psd.job = Some(Job {
            kind: Kind::Export,
            file,
            cancel,
            rx,
            guard: None,
        });
    }

    /// 計画（と、あれば利用者の確かめ）が済んだ書き出しを、別のスレッドで書く。
    fn start_psd_write(&mut self, run: Run) {
        let lang = self.lang;
        let file = run.file.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let flag = cancel.clone();
        let park = std::mem::take(&mut self.psd.park_write);
        let spawned = std::thread::Builder::new()
            .name("yolu-psd-export".into())
            .spawn(move || {
                if park {
                    crate::windows::park_until_canceled(&flag);
                }
                let _ = tx.send(write_worker(run, &flag));
            });
        if let Err(e) = spawned {
            self.message = e.to_string();
            return;
        }
        self.message = lang.pick(
            format!("PSD に書き出し中: {file}"),
            format!("Writing PSD: {file}"),
        );
        self.psd.job = Some(Job {
            kind: Kind::Export,
            file,
            cancel,
            rx,
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
            return self.refuse_psd_export(&file, why);
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
        let result = match job.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(Failure::Stopped),
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
                self.message = if canceled {
                    text.clone()
                } else {
                    lang.pick(
                        format!(
                            "PSD を{}できません: {text}",
                            if importing {
                                "読み込み"
                            } else {
                                "書き出し"
                            }
                        ),
                        format!(
                            "Cannot {} the PSD: {text}",
                            if importing { "read" } else { "write" }
                        ),
                    )
                };
                // 書き出せなかった理由（予算の超過・書けない場所など）は結果の窓にも出す。取消は利用者の操作なので出さない
                if !importing && !canceled {
                    self.refuse_psd_export(&job.file, vec![text]);
                }
                return;
            }
        };
        match (output, job.kind) {
            (
                Output::Refused {
                    mode,
                    reason,
                    diagnostics,
                },
                _,
            ) => {
                self.psd.imported.pop();
                let mode_text = match mode {
                    Some(m @ (CompatibilityMode::PreserveOnly | CompatibilityMode::Rejected)) => {
                        mode_name(lang, m)
                    }
                    _ => lang.pick("読み込めません", "Cannot import"),
                };
                self.message = lang.pick(
                    format!("PSD を読み込めません（{mode_text}）: {reason}"),
                    format!("Cannot import the PSD ({mode_text}): {reason}"),
                );
                let mut lines = vec![Line {
                    warning: true,
                    text: reason,
                }];
                lines.extend(diagnostic_lines(lang, &diagnostics));
                self.psd.report = Some(Report {
                    importing: true,
                    file: job.file,
                    ok: false,
                    summary: lang.pick(
                        format!("{mode_text}。ファイルは変えていません"),
                        format!("{mode_text}. The file was not modified"),
                    ),
                    lines,
                });
            }
            (Output::Imported { doc, diagnostics }, Kind::Import(target)) => {
                // 今のセットの文書を替える読み込みは、描いている最中か、読んでいる間に文書が変わっていたら入れない（描いたものを
                // 黙って捨てず、描きかけのストロークを取り残さない。ストロークの確定で文書の版が進むので、終わるまで待っても入らない）
                if let Some((uid, id, revision)) = job.guard {
                    if self.is_stroking() {
                        self.psd.imported.pop();
                        self.message = lang.pick(
                            format!(
                                "描いている間に読み終わったので、{} は入れませんでした。",
                                job.file
                            ),
                            format!(
                                "{} finished reading while drawing, so it was not imported.",
                                job.file
                            ),
                        );
                        return;
                    }
                    if self.sets.current().uid != uid
                        || self.doc.id() != id
                        || self.doc.revision() != revision
                    {
                        self.psd.imported.pop();
                        self.message = lang.pick(
                            format!(
                                "読み込んでいる間に文書が変わったので、{} は入れませんでした。",
                                job.file
                            ),
                            format!(
                                "The document changed while reading, so {} was not imported.",
                                job.file
                            ),
                        );
                        return;
                    }
                }
                let layers = doc.layers().len();
                let switched = self.install_psd(*doc, target, &job.file);
                self.message = lang.pick(
                    format!(
                        "PSD を読み込みました: {}（レイヤー {layers}）。PSD 自体は書き換えません。",
                        job.file
                    ),
                    format!(
                        "Imported {} ({layers} layers). The PSD itself is never rewritten.",
                        job.file
                    ),
                );
                if !switched {
                    self.message += lang.pick(
                        " 描いている間なので、切り替えていません。",
                        " Not switched to it while drawing.",
                    );
                }
                if !diagnostics.is_empty() {
                    self.message += &lang.pick(
                        format!(" 注意 {} 件。", diagnostics.len()),
                        format!(" {} note(s).", diagnostics.len()),
                    );
                    self.psd.report = Some(Report {
                        importing: true,
                        file: job.file,
                        ok: true,
                        summary: lang
                            .pick("読み込みました。注意があります", "Imported with notes")
                            .into(),
                        lines: diagnostic_lines(lang, &diagnostics),
                    });
                }
            }
            (Output::Planned(run), Kind::Export) => self.finish_psd_plan(*run),
            (Output::Exported { files }, Kind::Export) => {
                self.message = match files.as_slice() {
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
                };
            }
            _ => {}
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
                self.doc = doc;
                self.document_replaced();
                self.selected_layer = None;
                self.layer_scroll = 0.0;
                self.renaming = None;
                self.layer_drag = None;
                self.popup = None;
                // 前の文書の座標で打った多角形の点・量を聞く窓は、新しい文書へ持ち越さない
                self.sel_doc_changed();
                self.ensure_selection();
                if let Some(set) = self.sets.get_mut(self.sets.current_index()) {
                    set.saved = None;
                }
                self.sync_view3d();
            }
        }
        self.modified = true;
        switched
    }

    /// 試験用: PSD の仕事が終わるまで待って受ける（待ちの上限は 120 秒）。
    #[doc(hidden)]
    pub fn wait_psd(&mut self) {
        let start = Instant::now();
        while self.psd.job.is_some() {
            self.poll_psd();
            if self.psd.job.is_none() {
                break;
            }
            assert!(
                start.elapsed().as_secs() < 120,
                "PSD の処理が終わらない（ハング検出上限）"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

/// 別のスレッドの読み込み: 読めなければ理由と診断を `Refused` で返す（何も変えない）。
fn import_worker(path: &Path, cancel: &AtomicBool) -> Result<Output, Failure> {
    let limits = Limits::default();
    let refused =
        |reason: String, mode: Option<CompatibilityMode>, diagnostics: Vec<Diagnostic>| {
            Ok(Output::Refused {
                mode,
                reason,
                diagnostics,
            })
        };
    let meta = std::fs::metadata(path).map_err(Failure::File)?;
    if meta.len() > limits.max_source_bytes as u64 {
        return refused(
            format!(
                "ファイルが {} MiB を超えています",
                limits.max_source_bytes / (1024 * 1024)
            ),
            None,
            Vec::new(),
        );
    }
    let bytes = std::fs::read(path).map_err(Failure::File)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(Failure::Canceled);
    }
    let result = match psd::read(&bytes, &limits) {
        Ok(r) => r,
        Err(e) => return refused(e.to_string(), Some(CompatibilityMode::Rejected), Vec::new()),
    };
    let diagnostics = result.diagnostics().to_vec();
    match result.mode() {
        CompatibilityMode::Rejected => {
            let reason = diagnostics
                .first()
                .map(|d| d.message.clone())
                .unwrap_or_else(|| "読めない PSD です".into());
            return refused(reason, Some(CompatibilityMode::Rejected), diagnostics);
        }
        CompatibilityMode::PreserveOnly => {
            return refused(
                "編集できない内容を含みます（原本をそのまま保つだけです）".into(),
                Some(CompatibilityMode::PreserveOnly),
                diagnostics,
            );
        }
        CompatibilityMode::EditableRaster => {}
    }
    let document = result.document().ok_or_else(|| {
        Failure::message("編集用の文書がありません", "There is no editable document")
    })?;
    // core に入れられない内容は、黙って外さず断る（層のロックは core が持ち、`.ylp` にも書ける）
    let issues = document.core_issues();
    if !issues.is_empty() {
        let mut reason = issues
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("、");
        if issues.len() > 3 {
            reason += &format!(" ほか {} 件", issues.len() - 3);
        }
        return refused(
            format!("このアプリで扱えない内容があります（{reason}）"),
            Some(CompatibilityMode::EditableRaster),
            diagnostics,
        );
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Failure::Canceled);
    }
    match result.to_core() {
        Ok(doc) => Ok(Output::Imported {
            doc: Box::new(doc),
            diagnostics,
        }),
        Err(e) => refused(
            e.to_string(),
            Some(CompatibilityMode::EditableRaster),
            diagnostics,
        ),
    }
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

/// 別のスレッドの計画: 選んだチャンネルごとに、何を焼き・丸め・落とすかを決める（画素は作らない。丸める調整は、丸めた文書との合成の差を測る）。
fn plan_worker(
    snapshot: Arc<Document>,
    mode: ExportMode,
    targets: Vec<(Channel, PathBuf)>,
    file: String,
    cancel: &AtomicBool,
) -> Result<Output, Failure> {
    let ctl = ExportControl {
        cancel: Some(cancel),
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
    })))
}

/// 別のスレッドの書き出し: チャンネルごとに PSD を作り（焼く）、一時ファイルへ書いて読み戻して確かめ、全部が済んでから最後に置き換える。
fn write_worker(run: Run, cancel: &AtomicBool) -> Result<Output, Failure> {
    let ctl = ExportControl {
        cancel: Some(cancel),
    };
    let mut staged: Vec<Staged> = Vec::with_capacity(run.targets.len());
    let result = (|| -> Result<(), Failure> {
        for ((_, path), plan) in run.targets.iter().zip(&run.plans) {
            let exported = plan.build(&run.snapshot, &ctl).map_err(export_error)?;
            let bytes = psd::write(&exported.document, &Limits::default()).map_err(export_error)?;
            drop(exported);
            verify_readable(&bytes, Some(cancel))?;
            if cancel.load(Ordering::Relaxed) {
                return Err(Failure::Canceled);
            }
            staged.push(stage(path, &bytes)?);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(Failure::Canceled);
        }
        Ok(())
    })();
    if let Err(e) = result {
        discard(&staged);
        return Err(e);
    }
    let files: Vec<(PathBuf, usize)> = staged.iter().map(|s| (s.path.clone(), s.len)).collect();
    commit(staged)?;
    Ok(Output::Exported { files })
}

/// 書く PSD を読み戻して確かめる（読めて、診断は注記だけ）。読めない PSD は書かない。統合画像が層の重ねと食い違うという指摘（`CompositeMismatch`。
/// 層の不透明度を 1/255 の刻みに丸める差など）だけは、PSD としては正しいので書く。
fn verify_readable(bytes: &[u8], cancel: Option<&AtomicBool>) -> Result<(), Failure> {
    let read = psd::read_cancellable(bytes, &Limits::default(), cancel).map_err(|e| {
        if is_cancel(&e) {
            Failure::Canceled
        } else {
            Failure::Unreadable(e)
        }
    })?;
    let only_preview = read.mode() == CompatibilityMode::PreserveOnly
        && read
            .diagnostics()
            .iter()
            .all(|d| d.is_informational() || d.code == "CompositeMismatch");
    let editable = read.mode() == CompatibilityMode::EditableRaster
        && read.diagnostics().iter().all(Diagnostic::is_informational);
    if editable || only_preview {
        Ok(())
    } else {
        Err(Failure::NotEditable(read.mode()))
    }
}

/// 一時ファイルへ書いて読み戻した、まだ置き換えていないファイル。
struct Staged {
    path: PathBuf,
    temp: PathBuf,
    len: usize,
}

/// 同じフォルダの一時ファイルへ書き、読み戻して一致を確かめる（途中で失敗したら一時ファイルを消す）。通常のファイル以外の先は断る。
fn stage(path: &Path, bytes: &[u8]) -> Result<Staged, Failure> {
    use std::io::Write;
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| Failure::message("ファイル名がありません", "The file has no name"))?
        .to_string_lossy();
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.is_file() {
            return Err(Failure::message(
                format!("通常のファイルではありません: {}", path.display()),
                format!("Not a regular file: {}", path.display()),
            ));
        }
    }
    let temp = dir.join(format!(".{name}.{}.tmp~", std::process::id()));
    let result = (|| -> Result<(), Failure> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(Failure::File)?;
        file.write_all(bytes).map_err(Failure::File)?;
        file.sync_all().map_err(Failure::File)?;
        drop(file);
        if std::fs::read(&temp).map_err(Failure::File)? != bytes {
            return Err(Failure::message(
                "書いたファイルの読み戻しが一致しません",
                "The written file does not read back identically",
            ));
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(Staged {
            path: path.to_path_buf(),
            temp,
            len: bytes.len(),
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(e)
        }
    }
}

/// 一時ファイルを消す（元のファイルは変わらない）。
fn discard(staged: &[Staged]) {
    for s in staged {
        let _ = std::fs::remove_file(&s.temp);
    }
}

/// 最後に 1 回ずつ置き換える。途中で失敗したら、残りの一時ファイルを消し、済んだ分は置き換わっていると知らせる。
fn commit(staged: Vec<Staged>) -> Result<(), Failure> {
    for (done, s) in staged.iter().enumerate() {
        if let Err(e) = std::fs::rename(&s.temp, &s.path) {
            discard(&staged[done..]);
            return Err(if done == 0 {
                Failure::File(e)
            } else {
                Failure::PartlyReplaced { done, cause: e }
            });
        }
    }
    Ok(())
}

/// 1 つのファイルを、一時ファイルへ書き・読み戻し・置き換えまで通す（途中で失敗しても元のファイルは変わらず、一時ファイルは残らない）。試験用。
#[cfg(test)]
fn write_replacing(path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    commit(vec![stage(path, bytes)?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Action;
    use std::sync::atomic::AtomicU32;
    use yolu_core::{Channel, LayerId, TileCoord};

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
        assert_eq!(b.sets.current_index(), 1, "足したセットへ替える");
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

    /// 層のロックは、書き出す PSD の lspf を通って、取り込んだ文書の層に戻る（断らず、黙って外しもしない）。
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
        assert!(b.message.contains("文書が変わった"), "{}", b.message);
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
        assert!(c.message.contains("文書が変わった"), "{}", c.message);
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

    /// 書き出しはメニューで設定の窓を開き、「書き出し…」で書き出す先を選ぶ窓を頼む（読み込みは先のファイルを選ぶだけ）。
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
    fn preserve_only_rejected_and_unsupported_files_are_refused_with_reasons_and_change_nothing() {
        let dir = Dir::new("refuse");
        let good = {
            let s = painted();
            let projected = psd::Document::from_core(&s.doc).unwrap();
            psd::write(&projected, &Limits::default()).unwrap()
        };
        // PSB（版 2）は原本の保持だけ
        let mut psb = good.clone();
        psb[4..6].copy_from_slice(&2u16.to_be_bytes());
        std::fs::write(dir.0.join("big.psb.psd"), &psb).unwrap();
        // 壊れたファイルは拒否
        std::fs::write(dir.0.join("broken.psd"), b"8BPS-not-a-psd").unwrap();
        // キャンバスの外にはみ出す画素を持つ PSD は、切り捨てずに断る
        let mut wide = psd::Document::from_core(&painted().doc).unwrap();
        wide.layers[0].left = 20;
        wide.composite_rgba = None; // 統合画像は層から計算し直す（層と統合画像の食い違いで原本の保持だけになるのを避ける）
        std::fs::write(
            dir.0.join("wide.psd"),
            psd::write(&wide, &Limits::default()).unwrap(),
        )
        .unwrap();

        let mut s = AppState::new(32, 32);
        let before = (s.sets.len(), s.doc.id(), s.modified);
        for (file, expect_mode, expect_text) in [
            ("big.psb.psd", "原本の保持のみ", "PSB"),
            ("broken.psd", "拒否", ""),
            ("wide.psd", "読み込めません", "キャンバス外"),
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
            assert!(s.message.contains(expect_mode), "{file}: {}", s.message);
            let report = s.psd.report.take().expect(file);
            assert!(!report.ok && report.importing, "{file}");
            assert!(
                report.lines.iter().any(|l| l.text.contains(expect_text)),
                "{file}: {:?}",
                report.lines
            );
            assert!(report.lines[0].warning);
        }
        // 英語の文言
        s.lang = Lang::En;
        s.apply(Action::Psd(PsdAction::Import {
            path: dir.0.join("broken.psd"),
            target: PsdTarget::NewSet,
        }));
        s.wait_psd();
        assert!(
            s.message.starts_with("Cannot import the PSD"),
            "{}",
            s.message
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

    #[test]
    fn diagnostics_become_lines_with_informational_ones_marked_and_the_list_capped() {
        let diag = |code: &str, i: usize| Diagnostic {
            code: code.into(),
            message: format!("m{i}"),
            offset: 0,
            length: 0,
        };
        let many: Vec<Diagnostic> = (0..45)
            .map(|i| {
                diag(
                    if i == 0 {
                        "CompositeDiffers"
                    } else {
                        "ColorData"
                    },
                    i,
                )
            })
            .collect();
        let lines = diagnostic_lines(Lang::Ja, &many);
        assert_eq!(lines.len(), MAX_LINES + 1);
        assert_eq!(lines[0].text, "CompositeDiffers: m0");
        assert!(!lines[0].warning, "見え方の差の知らせは注意ではない");
        assert!(lines[1].warning);
        assert_eq!(lines.last().unwrap().text, "ほか 5 件");
        assert_eq!(
            diagnostic_lines(Lang::En, &many).last().unwrap().text,
            "and 5 more"
        );
        assert!(diagnostic_lines(Lang::Ja, &[]).is_empty());
    }

    /// 書き出しの確かめの窓の注記（機能と結果）を、チャンネルごとに日本語で。
    fn lines(s: &AppState) -> Vec<(String, String, String)> {
        let confirm = s.psd.notes_confirm.as_ref().expect("確かめの窓");
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

    /// PSD に形の無いもの（反転したマスク・クリッピングされたグループ・半透明の塗りつぶし）は、書く前に層の名前つきで確かめ、「書く」までは
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

    /// グループ・塗りつぶし・調整・マスクは PSD に書けて、取り込み直すと同じ層になる（画面の操作で作った文書で、書き出して取り込む）。
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

    /// 厳密な `from_core` は PSD に形が無い中身を黙って落とさず、機能ごとの理由で断る（焼き込みの書き出しは別）。層のロックは PSD に書けるので断らない。
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
        // Color と違う、ほかのチャンネルの合成（Color の合成は PSD の層の合成として書け、ほかのチャンネルはそのチャンネルの PSD の値になる）
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
        // 層のロック: from_core は lspf のビットで書く
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
            err.text(Lang::En, false).contains("Not a regular file"),
            "{err:?}"
        );
        // 先のフォルダが無ければ書けず、元のファイルは変わらない
        let missing = dir.0.join("missing").join("b.psd");
        assert!(write_replacing(&missing, b"x").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert_eq!(dir.files(), ["a.psd", "folder.psd"]);
    }

    /// 何にもクリッピングされないグループ（兄弟の一番下）のクリッピングの印は、外して書く。確かめの窓に、層の名前つきで「落とす」と出る。
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

    /// 刻みの間のトーンカーブは最寄りの刻みへ丸め、グラデーションマップの値のカーブは停止点へ展開する。確かめの窓の注記に、層の名前つきで丸めた値
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
        // 注記は上の層から
        let [(map, map_what, map_how), (tone, tone_what, tone_how)] = ja.as_slice() else {
            panic!("{ja:?}")
        };
        assert_eq!((tone.as_str(), map.as_str()), ("曲線", "マップ"));
        assert!(
            tone_what.contains("RGB 点 2 の入力 76.755→77") && tone_what.ends_with("、ほか 1"),
            "{tone_what}"
        );
        assert!(
            map_what.contains("カーブ → 停止点 色") && map_what.ends_with("不透明度 2"),
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
        assert!(en[0].1.contains("Curve → stops: ") && en[0].2.starts_with("Max diff "));
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
        let staged: Vec<Staged> = [&a, &b, &c]
            .iter()
            .map(|p| stage(p, b"new").unwrap())
            .collect();
        assert_eq!(dir.files().len(), 5, "元の 2 つと一時ファイル 3 つ");
        // 2 つ目の一時ファイルを無くして、その置き換えを失敗させる
        std::fs::remove_file(&staged[1].temp).unwrap();
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
        let staged: Vec<Staged> = [&a, &b].iter().map(|p| stage(p, b"new").unwrap()).collect();
        std::fs::remove_file(&staged[0].temp).unwrap();
        let err = commit(staged).unwrap_err();
        assert!(matches!(err, Failure::File(_)), "{err:?}");
        assert_eq!(std::fs::read(&a).unwrap(), b"old-a");
        assert!(!b.exists());
        assert_eq!(dir.files(), ["a.psd"]);
    }

    /// 書く PSD を読み戻して編集できる PSD でなければ書かない。理由は画面の言語の名前で出し、内部の名前は出さない。
    #[test]
    fn a_psd_that_does_not_read_back_as_editable_is_refused_with_a_localized_reason() {
        let s = painted();
        let ctl = ExportControl { cancel: None };
        let plan = psd::plan_export(
            &s.doc,
            &ExportOptions::new(Channel::Color, ExportMode::Bake),
            &ctl,
        )
        .unwrap();
        let exported = plan.build(&s.doc, &ctl).unwrap();
        let bytes = psd::write(&exported.document, &Limits::default()).unwrap();
        verify_readable(&bytes, None).expect("書いた PSD は読み戻せる");
        // 読み戻しの確かめ（統合画像を層の重ねと照らす所）も取消に従う。壊れた PSD の断りとは別の種類
        let raised = AtomicBool::new(true);
        let err = verify_readable(&bytes, Some(&raised)).unwrap_err();
        assert!(matches!(err, Failure::Canceled), "{err:?}");

        // 大きなドキュメント形式（PSB）は原本を保つだけ
        let mut psb = bytes.clone();
        psb[5] = 2;
        let err = verify_readable(&psb, None).unwrap_err();
        assert!(
            matches!(err, Failure::NotEditable(CompatibilityMode::PreserveOnly)),
            "{err:?}"
        );
        let ja = err.text(Lang::Ja, false);
        let en = err.text(Lang::En, false);
        assert!(
            ja.contains("編集できる PSD として読み戻せません（原本の保持のみ）"),
            "{ja}"
        );
        assert!(
            en.contains("does not read back as an editable PSD (Preserve only)"),
            "{en}"
        );
        assert!(!ja.contains("PreserveOnly") && !en.contains("PreserveOnly"));

        // 途中で切れた PSD と、PSD でないものも編集できる PSD としては読めない
        assert!(matches!(
            verify_readable(&bytes[..bytes.len() / 2], None),
            Err(Failure::NotEditable(_))
        ));
        let err = verify_readable(b"not a psd", None).unwrap_err();
        assert!(
            matches!(err, Failure::NotEditable(CompatibilityMode::Rejected)),
            "{err:?}"
        );
        assert!(err.text(Lang::Ja, false).ends_with("（拒否）"));
        assert!(err.text(Lang::En, false).ends_with("(Rejected)"));
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
        for mode in [
            CompatibilityMode::EditableRaster,
            CompatibilityMode::PreserveOnly,
            CompatibilityMode::Rejected,
        ] {
            for lang in [Lang::Ja, Lang::En] {
                let text = Failure::NotEditable(mode).text(lang, false);
                assert!(!text.contains("EditableRaster") && !text.contains("PreserveOnly"));
            }
        }
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

    /// 設定の窓の操作: 最後の 1 つのチャンネルは外せず、消えたチャンネルは次に開くときに外れる。複数チャンネルの確かめは、チャンネルごとに並ぶ。
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
        // 平らに 1 枚: 確かめは出ず、1 枚の層の PSD
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

    /// 焼いた層が画素の予算を超えたら、書く前の確かめのあとでも、理由を結果の窓に出して何も書かない。
    #[test]
    fn baked_layers_over_the_pixel_budget_are_refused_with_the_reason_after_the_check() {
        let dir = Dir::new("bake-budget");
        let mut s = AppState::new(4096, 4096);
        for n in 0..3 {
            s.doc
                .add_fill_layer(
                    &format!("ガラス{n}"),
                    &[(Channel::Color, yolu_core::Rgba8::new(1, 2, 3, 100))],
                    None,
                )
                .unwrap();
        }
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("big.psd"))));
        s.wait_psd();
        assert_eq!(
            s.psd.notes_confirm.as_ref().unwrap().sections()[0].1.len(),
            3
        );
        s.apply(Action::Psd(PsdAction::ConfirmWrite));
        s.wait_psd();
        assert!(s.message.contains("予算"), "{}", s.message);
        let report = s.psd.report.take().expect("理由の窓");
        assert!(!report.ok && !report.importing);
        assert!(report.lines[0].warning && report.lines[0].text.contains("予算"));
        assert!(dir.files().is_empty(), "書いていない: {:?}", dir.files());
    }

    /// 書けないもの（Normal の DirectX 向きのレベル補正）は、確かめの窓でなく理由の窓に出し、何も書かない。複数チャンネルでは、チャンネルの名前つき。
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
        let report = s.psd.report.take().expect("理由の窓");
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
        let report = s.psd.report.take().expect("理由の窓");
        assert!(
            report.lines[0].text.starts_with("Normal: "),
            "{:?}",
            report.lines
        );
    }

    /// 書けない場所（フォルダが無い・通常のファイルでない先）は、理由を結果の窓に出して何も変えない。複数のチャンネルの途中で書けなくても、
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
        let report = s.psd.report.take().expect("理由の窓");
        assert!(!report.ok && !report.importing && report.lines[0].warning);
        assert!(dir.files().is_empty());
        // 2 つ目の先がフォルダ: 1 つ目も置き換えない
        s.psd.export.channels = vec![Channel::Color, Channel::Roughness];
        std::fs::write(dir.0.join("Body_Color.psd"), b"old").unwrap();
        std::fs::create_dir(dir.0.join("Body_Roughness.psd")).unwrap();
        s.apply(Action::Psd(PsdAction::Export(dir.0.join("Body.psd"))));
        s.apply(Action::Psd(PsdAction::ConfirmReplace));
        s.wait_psd();
        let report = s.psd.report.take().expect("理由の窓");
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
    fn a_file_or_canvas_over_the_budget_is_refused_with_the_reason_and_changes_nothing() {
        let dir = Dir::new("budget");
        let mut s = AppState::new(32, 32);
        let before = (s.sets.len(), s.doc.id(), s.modified);
        // ファイルの大きさ（読まずに断る。穴あきのファイルなのでディスクは使わない）
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
        assert!(s.message.contains("MiB を超えています"), "{}", s.message);
        let report = s.psd.report.take().expect("理由の窓");
        assert!(!report.ok && report.importing);
        assert!(report.lines[0].warning && report.lines[0].text.contains("MiB"));
        assert_eq!((s.sets.len(), s.doc.id(), s.modified), before);
        // 読み込みのキャンバス（幅・高さを予算の外へ書き換えたファイル）
        let mut wide = {
            let projected = psd::Document::from_core(&painted().doc).unwrap();
            psd::write(&projected, &Limits::default()).unwrap()
        };
        wide[14..18].copy_from_slice(&5000u32.to_be_bytes());
        wide[18..22].copy_from_slice(&5000u32.to_be_bytes());
        std::fs::write(dir.0.join("wide.psd"), &wide).unwrap();
        s.apply(Action::Psd(PsdAction::Import {
            path: dir.0.join("wide.psd"),
            target: PsdTarget::CurrentSet,
        }));
        s.wait_psd();
        assert!(
            s.message.contains("拒否") && s.message.contains("予算"),
            "{}",
            s.message
        );
        let report = s.psd.report.take().expect("理由の窓");
        assert!(!report.ok && report.importing);
        assert!(report.lines[0].warning && report.lines[0].text.contains("予算"));
        assert_eq!((s.sets.len(), s.doc.id(), s.modified), before);
        // 書き出しのキャンバス: 一時ファイルも作らない
        let mut big = AppState::new(5000, 5000);
        big.apply(Action::Psd(PsdAction::Export(dir.0.join("big.psd"))));
        assert!(!big.psd.is_busy());
        assert!(big.message.contains("予算"), "{}", big.message);
        let report = big.psd.report.take().expect("理由の窓");
        assert!(!report.ok && !report.importing);
        assert!(report.lines[0].warning && report.lines[0].text.contains("予算"));
        assert_eq!(dir.files(), ["huge.psd", "wide.psd"], "書いていない");
    }
}
