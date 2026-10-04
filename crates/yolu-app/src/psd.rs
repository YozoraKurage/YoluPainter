//! PSD の読み込みと書き出し（RGB8 の PSD。Unity 版の `ImportPsd`・`ExportPsd` と同じ考え方）。コーデックは yolu-io の `psd`。
//!
//! - **読み込み**（新しいテクスチャセットか、今のセットの文書として）: 別のスレッドで読む（ファイルを読む・`psd::read`・core の文書への変換）。
//!   編集できる（`EditableRaster`）うえ core の文書に変えられるものだけを入れる。グループ（入れ子・通過/分離）・塗りつぶし（単色）・調整
//!   （反転・レベル補正・色相/彩度）・マスク・クリッピングは core の層になる。**原本を保つだけ（`PreserveOnly`）・拒否（`Rejected`）・
//!   core で扱えない中身（キャンバス外の画素）は、何も変えずに理由（診断の一覧）を窓で見せる**。層のロック（lspf）は core の層のロックとして入る。
//!   名前だけでレイヤーを結び付けない（`to_core` は ID で扱う）。PSD の原本は書き換えない。今のセットの文書を替える読み込みは、
//!   読み終わったときに描いている最中か、読んでいる間に文書が変わっていれば入れない（描きかけのストロークを取り残さず、描いたものを黙って捨てない）。
//! - **書き出し**（今の文書）: Color の写しを書く。ラスター・グループ・単色の塗りつぶし・調整・クリッピング・マスク（有効/無効・濃度）・層のロックは
//!   PSD の形で書き、反転したマスク・クリッピングされたグループ・半透明の塗りつぶし・刻みの間の調整・Color 以外のチャンネルの中身・Color と
//!   違うチャンネルごとの合成は、平らにせず理由を見せて書かない（黙って捨てない）。書くのは別のスレッドで、一時ファイルへ書いて読み戻して確かめ、
//!   最後に 1 回の置き換え。取り込んだ PSD と同じファイルへ書くときは確かめる。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

use egui::Vec2;
use yolu_core::Document;
use yolu_io::psd::{self, CompatibilityMode, Diagnostic, Limits};

use crate::lang::Lang;
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
    /// 書き出す先を選ぶ窓を頼む。
    ExportDialog,
    /// 今の文書を PSD に書き出す。
    Export(PathBuf),
    /// 取り込んだ PSD を置き換える確かめの「置き換える」。
    ConfirmReplace,
    /// 確かめの「やめる」。
    CancelConfirm,
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
    Exported {
        bytes: usize,
    },
}

enum Kind {
    Import(PsdTarget),
    Export(PathBuf),
}

struct Job {
    kind: Kind,
    file: String,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Result<Output, String>>,
    /// 今のセットの文書を替える読み込みが始まったときの、セットの uid・文書の ID・版（読んでいる間に変わっていたら入れない）。
    guard: Option<(u32, u128, u64)>,
}

/// PSD の状態。
#[derive(Default)]
pub struct PsdState {
    pub report: Option<Report>,
    pub report_offset: Vec2,
    /// 取り込んだ PSD の場所（同じファイルへ書き出すときに確かめる）。
    imported: Vec<PathBuf>,
    /// 取り込んだ PSD を置き換えるかの確かめ（書き出す先）。
    pub confirm: Option<PathBuf>,
    pub confirm_offset: Vec2,
    job: Option<Job>,
    /// 試験用: 次の仕事を、取消が来るまで始めずに止めておく（始めるときに下ろす）。
    #[doc(hidden)]
    pub park_next: bool,
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

/// 書き出せない理由（層の名前つき。画面の言語）。何が断られるかの判断は `psd::export_blockers`（`from_core` が断るのと同じもの。
/// `from_core` の断りは日本語の診断）で、ここは理由の種類から画面の文を作る。
pub fn export_blockers(lang: Lang, doc: &Document) -> Vec<String> {
    psd::export_blockers(doc)
        .iter()
        .map(|b| blocker_text(lang, b))
        .collect()
}

fn blocker_text(lang: Lang, b: &psd::Blocker) -> String {
    use psd::Refusal::*;
    let name = &b.layer;
    match &b.refusal {
        OtherChannel(label) => lang.pick(
            format!("「{name}」が Color 以外のチャンネルを使っています（{label}）"),
            format!("\"{name}\" uses a channel other than Color ({label})"),
        ),
        ClippedGroup => lang.pick(
            format!("「{name}」はクリッピングされたグループです"),
            format!("\"{name}\" is a clipped group"),
        ),
        InvertedMask => lang.pick(
            format!("「{name}」のマスクは反転しています"),
            format!("\"{name}\" has an inverted mask"),
        ),
        ChannelBlend(label) => lang.pick(
            format!("「{name}」の {label} の合成が Color と違います"),
            format!("\"{name}\" blends differently in {label} than in Color"),
        ),
        ColorDisabled => lang.pick(
            format!("「{name}」の Color が無効です"),
            format!("\"{name}\" has Color disabled"),
        ),
        FillWithoutColor => lang.pick(
            format!("「{name}」は Color の値が無い、または Color が無効な塗りつぶしです"),
            format!("\"{name}\" is a fill without a Color value or with Color disabled"),
        ),
        FillTranslucent => lang.pick(
            format!("「{name}」は半透明の塗りつぶしです"),
            format!("\"{name}\" is a translucent fill"),
        ),
        AdjustmentColorDisabled => lang.pick(
            format!("「{name}」は Color で無効な調整です"),
            format!("\"{name}\" is an adjustment disabled in Color"),
        ),
        LevelsBetweenSteps => lang.pick(
            format!("「{name}」のレベル補正は PSD の刻みの間にあります"),
            format!("\"{name}\" has levels between PSD's steps"),
        ),
        LevelsRange => lang.pick(
            format!("「{name}」のレベル補正の入力が PSD の範囲に収まりません"),
            format!("\"{name}\" has a levels input outside PSD's range"),
        ),
        HueSaturationBetweenSteps => lang.pick(
            format!("「{name}」の色相・彩度は PSD の刻みの間にあります"),
            format!("\"{name}\" has hue/saturation between PSD's steps"),
        ),
        GradientMapBetweenSteps => lang.pick(
            format!("「{name}」のグラデーションマップは PSD の刻みの間にあります"),
            format!("\"{name}\" has a gradient map between PSD's steps"),
        ),
        GradientMapCurve => lang.pick(
            format!("「{name}」のグラデーションマップに値のカーブがあります"),
            format!("\"{name}\" has a gradient map with a value curve"),
        ),
        ToneCurveBetweenSteps => lang.pick(
            format!("「{name}」のトーンカーブは PSD の刻みの間にあります"),
            format!("\"{name}\" has a tone curve between PSD's steps"),
        ),
        ColorBalanceBetweenSteps => lang.pick(
            format!("「{name}」のカラーバランスは PSD の刻みの間にあります"),
            format!("\"{name}\" has color balance between PSD's steps"),
        ),
        BrightnessContrastBetweenSteps => lang.pick(
            format!("「{name}」の明るさ・コントラストは PSD の刻みの間にあります"),
            format!("\"{name}\" has brightness/contrast between PSD's steps"),
        ),
        Effects => lang.pick(
            format!("「{name}」にフィルターか Generator があります"),
            format!("\"{name}\" has filters or generators"),
        ),
        Anchor => lang.pick(
            format!("「{name}」に Anchor があります"),
            format!("\"{name}\" has an anchor"),
        ),
        Path => lang.pick(
            format!("「{name}」にパスがあります"),
            format!("\"{name}\" has a path"),
        ),
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
                self.dialog_request = Some(DialogRequest::PsdExport);
            }
            PsdAction::Export(path) => {
                if stroking {
                    return refuse(self);
                }
                self.start_psd_export(&path, false);
            }
            PsdAction::ConfirmReplace => {
                if let Some(path) = self.psd.confirm.take() {
                    self.start_psd_export(&path, true);
                }
            }
            PsdAction::CancelConfirm => {
                if self.psd.confirm.take().is_some() {
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

    fn start_psd_export(&mut self, path: &Path, confirmed: bool) {
        let lang = self.lang;
        if self.psd.job.is_some() {
            self.message = lang
                .pick("PSD を処理中です。", "A PSD job is running.")
                .into();
            return;
        }
        let file = file_name(path);
        let refuse = |s: &mut AppState, why: Vec<String>| {
            let summary = lang.pick(
                "書き出せません。何も書いていません",
                "Cannot export. Nothing was written",
            );
            s.message = format!(
                "{}: {}",
                lang.pick("PSD に書き出せません", "Cannot export PSD"),
                why.first().cloned().unwrap_or_default()
            );
            s.psd.report = Some(Report {
                importing: false,
                file: file.clone(),
                ok: false,
                summary: summary.into(),
                lines: why
                    .into_iter()
                    .map(|text| Line {
                        warning: true,
                        text,
                    })
                    .collect(),
            });
        };
        if let Some(reason) = self.read_only_reason() {
            let why = vec![lang.pick(
                format!("読むだけのテクスチャセットです: {reason}"),
                format!("Read-only texture set: {reason}"),
            )];
            return refuse(self, why);
        }
        let blockers = export_blockers(lang, &self.doc);
        if !blockers.is_empty() {
            return refuse(self, blockers);
        }
        if !confirmed && self.psd.imported.iter().any(|p| same_file(p, path)) {
            self.psd.confirm = Some(path.to_path_buf());
            return;
        }
        // 文書を PSD の形へ写す（このスレッド）。書き込みは別のスレッド
        let projected = match psd::Document::from_core(&self.doc) {
            Ok(d) => d,
            Err(e) => return refuse(self, vec![e.to_string()]),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (flag, target) = (cancel.clone(), path.to_path_buf());
        let park = std::mem::take(&mut self.psd.park_next);
        let spawned = std::thread::Builder::new()
            .name("yolu-psd-export".into())
            .spawn(move || {
                if park {
                    crate::windows::park_until_canceled(&flag);
                }
                let _ = tx.send(export_worker(&projected, &target, &flag));
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
            kind: Kind::Export(path.to_path_buf()),
            file,
            cancel,
            rx,
            guard: None,
        });
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
            Err(TryRecvError::Disconnected) => Err(lang
                .pick("PSD の処理が止まりました", "The PSD job stopped")
                .into()),
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
                self.message = lang.pick(
                    format!(
                        "PSD を{}できません: {e}",
                        if importing {
                            "読み込み"
                        } else {
                            "書き出し"
                        }
                    ),
                    format!(
                        "Cannot {} the PSD: {e}",
                        if importing { "read" } else { "write" }
                    ),
                );
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
                    Some(CompatibilityMode::PreserveOnly) => {
                        lang.pick("原本の保持のみ", "Preserve only")
                    }
                    Some(CompatibilityMode::Rejected) => lang.pick("拒否", "Rejected"),
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
            (Output::Exported { bytes }, Kind::Export(path)) => {
                self.message = lang.pick(
                    format!(
                        "PSD に書き出しました: {}（{} バイト）。",
                        path.display(),
                        bytes
                    ),
                    format!("Wrote PSD: {} ({} bytes).", path.display(), bytes),
                );
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
fn import_worker(path: &Path, cancel: &AtomicBool) -> Result<Output, String> {
    let limits = Limits::default();
    let refused =
        |reason: String, mode: Option<CompatibilityMode>, diagnostics: Vec<Diagnostic>| {
            Ok(Output::Refused {
                mode,
                reason,
                diagnostics,
            })
        };
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
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
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if cancel.load(Ordering::Relaxed) {
        return Err("取り消しました".into());
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
    let document = result.document().ok_or("編集用の文書がありません")?;
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
        return Err("取り消しました".into());
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

/// 別のスレッドの書き出し: PSD にして、一時ファイルへ書いて読み戻して確かめ、最後に 1 回の置き換え。
fn export_worker(doc: &psd::Document, path: &Path, cancel: &AtomicBool) -> Result<Output, String> {
    let bytes = psd::write(doc, &Limits::default()).map_err(|e| e.to_string())?;
    if cancel.load(Ordering::Relaxed) {
        return Err("取り消しました（何も書いていません）".into());
    }
    write_replacing(path, &bytes)?;
    Ok(Output::Exported { bytes: bytes.len() })
}

/// 同じフォルダの一時ファイルへ書き、読み戻して一致を確かめてから置き換える（途中で失敗しても元のファイルは変わらず、一時ファイルは残らない）。
pub fn write_replacing(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| "ファイル名がありません".to_string())?
        .to_string_lossy();
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.is_file() {
            return Err(format!("通常のファイルではありません: {}", path.display()));
        }
    }
    let temp = dir.join(format!(".{name}.{}.tmp~", std::process::id()));
    let result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        if std::fs::read(&temp).map_err(|e| e.to_string())? != bytes {
            return Err("書いたファイルの読み戻しが一致しません".into());
        }
        std::fs::rename(&temp, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
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

    #[test]
    fn the_menu_only_asks_for_the_file() {
        let mut s = AppState::new(32, 32);
        s.apply(Action::Psd(PsdAction::ImportDialog(PsdTarget::NewSet)));
        assert_eq!(
            s.dialog_request,
            Some(DialogRequest::PsdImport(PsdTarget::NewSet))
        );
        s.apply(Action::Psd(PsdAction::ExportDialog));
        assert_eq!(s.dialog_request, Some(DialogRequest::PsdExport));
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

    #[test]
    fn export_refuses_what_a_psd_cannot_hold_and_writes_nothing() {
        let dir = Dir::new("blockers");
        let path = dir.0.join("out.psd");
        let mut s = painted();
        assert!(export_blockers(Lang::Ja, &s.doc).is_empty());
        // 反転したマスク（PSD に非破壊の反転が無い）。マスクそのものは書ける
        let layer = s.selected_layer.unwrap();
        s.apply(Action::M2(crate::m2::Edit::AddMask(layer)));
        assert!(
            export_blockers(Lang::Ja, &s.doc).is_empty(),
            "マスクは書ける"
        );
        s.doc.set_layer_mask_inverted(layer, true).unwrap();
        let why = export_blockers(Lang::Ja, &s.doc);
        assert_eq!(why.len(), 1);
        assert!(why[0].contains("反転"), "{why:?}");
        s.apply(Action::Psd(PsdAction::Export(path.clone())));
        assert!(!s.psd.is_busy());
        assert!(s.message.contains("書き出せません"), "{}", s.message);
        let report = s.psd.report.take().unwrap();
        assert!(!report.ok && !report.importing && report.lines[0].text.contains("反転"));
        assert!(dir.files().is_empty(), "何も書かない");
        // 英語
        assert!(export_blockers(Lang::En, &s.doc)[0].contains("inverted"));
        // クリッピングされたグループ・半透明の塗りつぶし
        let mut t = painted();
        t.apply(Action::M2(crate::m2::Edit::NewGroup));
        let group = t.selected_layer.unwrap();
        t.doc.set_layer_clipping(group, true).unwrap();
        let why = export_blockers(Lang::Ja, &t.doc);
        assert!(
            why.iter().any(|w| w.contains("クリッピングされたグループ")),
            "{why:?}"
        );
        t.apply(Action::M2(crate::m2::Edit::NewFill));
        let fill = t.selected_layer.unwrap();
        t.doc
            .set_fill_value(
                fill,
                Channel::Color,
                Some(yolu_core::Rgba8::new(1, 2, 3, 100)),
                false,
            )
            .unwrap();
        let why = export_blockers(Lang::En, &t.doc);
        assert!(
            why.iter().any(|w| w.contains("translucent fill")),
            "{why:?}"
        );
        // Color 以外のチャンネル
        let mut u = painted();
        let layer = u.selected_layer.unwrap();
        u.doc
            .set_channel_enabled(layer, Channel::Metallic, true)
            .unwrap();
        let why = export_blockers(Lang::Ja, &u.doc);
        assert!(why[0].contains("Color 以外"), "{why:?}");
        // 読むだけのセット
        let mut v = painted();
        v.sets.get_mut(0).unwrap().read_only = Some("試験".into());
        v.apply(Action::Psd(PsdAction::Export(path)));
        assert!(!v.psd.is_busy());
        assert!(v.psd.report.as_ref().unwrap().lines[0]
            .text
            .contains("読むだけ"));
        assert!(dir.files().is_empty());
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
        assert!(
            export_blockers(Lang::Ja, &s.doc).is_empty(),
            "{:?}",
            export_blockers(Lang::Ja, &s.doc)
        );
        let expected: Vec<_> = s
            .doc
            .layers()
            .iter()
            .map(|l| (l.name().to_string(), l.kind(), l.mask().is_some()))
            .collect();
        let composite = s.doc.composite(s.doc.bounds()).unwrap();
        s.apply(Action::Psd(PsdAction::Export(path.clone())));
        s.wait_psd();
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

    /// `from_core` は PSD に形が無い中身を黙って落とさず、機能ごとの理由で断る。先の断りは同じものを層の名前つきで画面の言語で言う。
    /// 層のロックは PSD に書けるので、どちらも断らない。
    #[test]
    fn from_core_refuses_what_the_up_front_refusal_names_and_writes_the_locks() {
        use yolu_core::{BlendMode, ChannelBlend, LayerLocks};
        let mut s = painted();
        let layer = s.selected_layer.unwrap();
        s.apply(Action::M2(crate::m2::Edit::AddMask(layer)));
        s.doc.set_layer_mask_inverted(layer, true).unwrap();
        let err = psd::Document::from_core(&s.doc).unwrap_err().to_string();
        assert!(err.contains("反転"), "{err}");
        assert_eq!(export_blockers(Lang::Ja, &s.doc).len(), 1);
        // Color と違う、ほかのチャンネルの合成（Color の合成は PSD の層の合成として書ける）
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
        assert!(psd::Document::from_core(&t.doc).is_ok());
        assert!(export_blockers(Lang::Ja, &t.doc).is_empty());
        t.doc
            .set_channel_blend(
                layer,
                Channel::Roughness,
                ChannelBlend::new(Some(BlendMode::Screen), None),
                false,
            )
            .unwrap();
        let err = psd::Document::from_core(&t.doc).unwrap_err().to_string();
        assert!(err.contains("Roughness") && err.contains("合成"), "{err}");
        let why = export_blockers(Lang::Ja, &t.doc);
        assert!(why.iter().any(|w| w.contains("Roughness")), "{why:?}");
        assert!(export_blockers(Lang::En, &t.doc)[0].contains("blends differently in Roughness"));
        // 層のロック: 先の断りは出さず、from_core は lspf のビットで書く
        let mut v = painted();
        let layer = v.selected_layer.unwrap();
        v.doc.set_layer_locks(layer, LayerLocks::PIXELS).unwrap();
        assert!(export_blockers(Lang::Ja, &v.doc).is_empty());
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
        assert_eq!(s.psd.confirm.as_deref(), Some(path.as_path()));
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
        assert!(err.contains("通常のファイルではありません"), "{err}");
        // 先のフォルダが無ければ書けず、元のファイルは変わらない
        let missing = dir.0.join("missing").join("b.psd");
        assert!(write_replacing(&missing, b"x").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert_eq!(dir.files(), ["a.psd", "folder.psd"]);
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
