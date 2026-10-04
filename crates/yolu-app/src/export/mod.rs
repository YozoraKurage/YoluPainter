//! 書き出しのテンプレート（Unity の Standard / URP・HDRP・lilToon が読む詰め合わせの PNG）をフォルダへ書く。Unity 版の
//! `ExportTemplates` と同じ決まりで、計算は core（`yolu_core::export`・`padding`）、書くのは yolu-io の `export::write_images`。
//!
//! - **全部のテクスチャセットを 1 回で**: 画像の名前は `<名前>[_<セット名>]_<接尾辞>.png`（セットが複数のときだけセット名）。名前が
//!   同じになる画像・もうあるファイル（画面で確かめてから置き換える）を、1 枚も書く前に断る。書く前の検査・一時ファイル・置き換えの
//!   手順は `write_images` に任せ、複数のセットの画像を **1 回の呼び出し**で書くので、取消・失敗のときは元のファイルが 1 つも変わらない
//!   （置き換えの途中の失敗だけは、済んだ分が置き換わっていると知らせる）。
//! - **別のスレッドで書く**: 文書は作業の最中に変わるので、始めるときに正本（`NativeDocument`）へ写して渡す（写すのはこのスレッド）。
//!   画像を作る・塗り広げる・PNG にする・読み戻して確かめるのは別のスレッドで、進み具合と取消がある。
//! - **パディング**: 画像のテクセルのうち UV の三角形が覆わない所を、境目の色で塗り広げる（Unity 版の `ExportPadding`。既定は届くかぎり
//!   全部）。UV は 3D ビューのモデルから、セットのマテリアルの三角形を使う。モデルが無い・セットの面が無ければ塗り広げず、そう知らせる。
//! - **AO**: セットに今の条件で焼いたメッシュマップ（AO）があれば使う（古いものは使わず知らせる）。無ければ遮蔽なし（白）。
//! - 読むだけのセット（core で扱えない中身の合成を見せているだけ）は書き出さない。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

use egui::Vec2;
use yolu_core::export::{build, ExportError as CalcError, ExportTemplate};
use yolu_core::glam::DVec2;
use yolu_core::padding::{self, Reach};
use yolu_io::export::{
    clashes, existing_files, plan_template, write_images, ExportError, ExportFile, Overwrite,
    PlanSet, WriteOptions, WrittenImage,
};
use yolu_io::NativeDocument;

use crate::bake::Occlusion;
use crate::state::{AppState, DialogRequest};

/// 画像を作る作業と塗り広げの作業のメモリの上限（Unity 版の StrokeBudgetBytes の上の方と同じ）。
pub const WORKING_BYTES: u64 = 512 * 1024 * 1024;

/// 書き出しの操作（`Action::Export`）。
#[derive(Clone, Debug, PartialEq)]
pub enum ExportAction {
    /// テンプレート（ID）で書き出す。書き出す先のフォルダを選ぶ窓を頼む。
    Template(String),
    /// 書き出す先のフォルダが決まった。もうあるファイルがあれば、確かめの窓を出す。
    TemplateTo { id: String, dir: PathBuf },
    /// 確かめの窓の「置き換える」。
    ConfirmReplace,
    /// 確かめの窓の「やめる」。
    CancelConfirm,
    /// 書いている最中の取消（元のファイルは変えない）。
    Cancel,
    /// 結果の窓を閉じる。
    DismissReport,
}

/// 書き出しの計画で出た注意（言語は表示のときに決める）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// 3D ビューにモデルが無く、UV が分からないので塗り広げなかった。
    NoModel,
    /// そのセットのマテリアルの三角形が無く、塗り広げなかった。
    NoUv(String),
    /// 焼いた AO が今の条件のものではないので使わなかった（セット・理由）。
    StaleOcclusion(String, String),
    /// AO を読む画像があるが、焼いた AO が無いセットがある（遮蔽なし = 白）。
    WhiteOcclusion,
    /// 読むだけのセットを書き出さなかった。
    ReadOnly(String),
}

/// もうあるファイルの確かめ（置き換えるか）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub template: String,
    pub dir: PathBuf,
    /// もうあるファイルの名前。
    pub existing: Vec<String>,
    /// 書く画像の数。
    pub total: usize,
}

/// 書き終えた結果（短い一覧）。
#[derive(Clone, Debug)]
pub struct Report {
    pub template: String,
    pub dir: PathBuf,
    pub images: Vec<WrittenImage>,
    pub notes: Vec<Note>,
}

struct Job {
    template: String,
    dir: PathBuf,
    total: usize,
    done: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    notes: Vec<Note>,
    rx: Receiver<Result<Vec<WrittenImage>, ExportError>>,
}

/// 書き出しの状態。
pub struct ExportState {
    /// 塗り広げの量（Unity 版の `PainterSettings.ExportPadding` と同じ: -1 は届くかぎり全部、0 は塗り広げない、n は n 段）。
    pub padding: i32,
    pub confirm: Option<Confirm>,
    pub report: Option<Report>,
    pub confirm_offset: Vec2,
    pub report_offset: Vec2,
    job: Option<Job>,
    /// 試験用: 次の仕事を、取消が来るまで始めずに止めておく（始めるときに下ろす）。
    #[doc(hidden)]
    pub park_next: bool,
}

impl Default for ExportState {
    fn default() -> Self {
        ExportState {
            padding: -1,
            confirm: None,
            report: None,
            confirm_offset: Vec2::ZERO,
            report_offset: Vec2::ZERO,
            job: None,
            park_next: false,
        }
    }
}

/// 書いている最中の進み具合。
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub template: String,
    /// 作っている画像の番号（1 から）と全部の数。
    pub index: usize,
    pub total: usize,
    pub canceling: bool,
}

impl ExportState {
    pub fn is_exporting(&self) -> bool {
        self.job.is_some()
    }

    pub fn progress(&self) -> Option<Progress> {
        let job = self.job.as_ref()?;
        Some(Progress {
            template: job.template.clone(),
            index: (job.done.load(Ordering::Relaxed) + 1).min(job.total),
            total: job.total,
            canceling: job.cancel.load(Ordering::Relaxed),
        })
    }
}

/// 注意の文。
pub fn note_text(lang: crate::lang::Lang, note: &Note) -> String {
    match note {
        Note::NoModel => lang
            .pick(
                "塗り広げなし: 3D ビューにモデルが無く UV が分かりません",
                "No padding: no model in the 3D view, so the UVs are unknown",
            )
            .into(),
        Note::NoUv(set) => lang.pick(
            format!("「{set}」は塗り広げなし: マテリアルの UV の三角形がありません"),
            format!("No padding for \"{set}\": no UV triangle of its material"),
        ),
        Note::StaleOcclusion(set, why) => lang.pick(
            format!("「{set}」の AO は古いので使いません: {why}"),
            format!("The AO of \"{set}\" is stale and is not used: {why}"),
        ),
        Note::WhiteOcclusion => lang
            .pick(
                "焼いた AO が無いセットの遮蔽は白（遮蔽なし）です",
                "A set without a current AO bake has white occlusion (none)",
            )
            .into(),
        Note::ReadOnly(set) => lang.pick(
            format!("読むだけのセット「{set}」は書き出しません"),
            format!("Read-only set \"{set}\" was not exported"),
        ),
    }
}

/// 1 セットぶんの書き出しの入力（始めるときに写す）。
struct SetInput {
    native: NativeDocument,
    occlusion: Option<Vec<u8>>,
    /// UV の三角形（画素の座標）。塗り広げないなら None。
    uv: Option<Vec<[DVec2; 3]>>,
}

struct Plan {
    sets: Vec<SetInput>,
    /// (セットの番号, テンプレートの画像の番号, ファイル名)。
    files: Vec<(usize, usize, String)>,
    notes: Vec<Note>,
}

/// 書き出しのファイル名の元（開いたプロジェクトのファイル名。無ければ Texture）。
pub fn stem(state: &AppState) -> String {
    state
        .project
        .as_ref()
        .and_then(|p| p.path().file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Texture".into())
}

/// セットのマテリアルの UV の三角形（画素の座標。文書の大きさをかける）。
fn uv_triangles(
    model: &crate::view3d::model::ViewModel,
    material: i32,
    width: u32,
    height: u32,
) -> Vec<[DVec2; 3]> {
    let (w, h) = (width as f64, height as f64);
    let at = |uv: yolu_core::glam::Vec2| DVec2::new(uv.x as f64 * w, uv.y as f64 * h);
    model
        .geometry
        .triangles()
        .iter()
        .filter(|t| t.material == material)
        .map(|t| [at(t.uv_a), at(t.uv_b), at(t.uv_c)])
        .collect()
}

impl AppState {
    /// 書き出す画像と名前を決め、文書を写す（このスレッド。写せなければ理由）。
    fn plan_export(&mut self, template: &ExportTemplate, stem: &str) -> Result<Plan, String> {
        let lang = self.lang;
        let reach = Reach::from_setting(self.export.padding).map_err(|e| e.to_string())?;
        let padding_on = reach != Reach::Texels(0);
        let mut notes = Vec::new();
        // 書き出すセット（読むだけのセットは除く）
        let mut indices = Vec::new();
        for i in 0..self.sets.len() {
            let set = self.sets.get(i).expect("範囲内");
            if set.read_only.is_some() {
                notes.push(Note::ReadOnly(set.name.clone()));
            } else {
                indices.push(i);
            }
        }
        let several = self.sets.len() > 1;
        // AO（今の条件で焼いたものだけ）
        let mut occlusions = Vec::new();
        for &i in &indices {
            let occlusion = self.occlusion_for_export(i);
            if let Occlusion::Stale(why) = &occlusion {
                notes.push(Note::StaleOcclusion(
                    self.sets.get(i).expect("範囲内").name.clone(),
                    why.clone(),
                ));
            }
            occlusions.push(match occlusion {
                Occlusion::Bytes(b) => Some(b),
                _ => None,
            });
        }
        let planned = {
            let plan_sets: Vec<PlanSet<'_>> = indices
                .iter()
                .zip(&occlusions)
                .map(|(&i, o)| PlanSet {
                    name: several.then(|| self.sets.get(i).expect("範囲内").name.as_str()),
                    document: self.set_doc(i),
                    has_occlusion: o.is_some(),
                })
                .collect();
            plan_template(stem, &plan_sets, template).map_err(|e| e.to_string())?
        };
        if planned.is_empty() {
            return Err(lang.pick(
                format!(
                    "書き出すものがありません: どのレイヤーも「{}」が読むチャンネルを使っていません",
                    template.name
                ),
                format!(
                    "Nothing to export for \"{}\": no layer uses a channel it reads",
                    template.name
                ),
            ));
        }
        let names: Vec<&str> = planned.iter().map(|p| p.file_name.as_str()).collect();
        let clash = clashes(names.iter().copied());
        if !clash.is_empty() {
            return Err(lang.pick(
                format!(
                    "書き出しませんでした: 同じファイルになる画像があります: {}",
                    clash.join("、")
                ),
                format!(
                    "Nothing was exported: these images would write the same file: {}",
                    clash.join(", ")
                ),
            ));
        }
        // 写す（planned に出たセットだけ）
        let model = self.view3d.full_model().cloned();
        let mut sets: Vec<SetInput> = Vec::new();
        let mut position: Vec<Option<usize>> = vec![None; indices.len()];
        let mut files = Vec::with_capacity(planned.len());
        let mut no_model = false;
        let mut white = false;
        for p in &planned {
            let slot = match position[p.set] {
                Some(s) => s,
                None => {
                    let index = indices[p.set];
                    let (w, h) = {
                        let d = self.set_doc(index);
                        (d.width(), d.height())
                    };
                    let native = NativeDocument::from_core(self.set_doc(index)).map_err(|e| {
                        lang.pick(
                            format!("文書を写せません: {e}"),
                            format!("Cannot copy the document: {e}"),
                        )
                    })?;
                    let name = self.sets.get(index).expect("範囲内").name.clone();
                    let uv = if !padding_on {
                        None
                    } else {
                        match (&model, self.set_material(index)) {
                            (None, _) => {
                                no_model = true;
                                None
                            }
                            (Some(m), Some(material)) => {
                                let tris = uv_triangles(m, material, w, h);
                                if tris.is_empty() {
                                    notes.push(Note::NoUv(name.clone()));
                                    None
                                } else {
                                    Some(tris)
                                }
                            }
                            (Some(_), None) => {
                                notes.push(Note::NoUv(name.clone()));
                                None
                            }
                        }
                    };
                    sets.push(SetInput {
                        native,
                        occlusion: occlusions[p.set].clone(),
                        uv,
                    });
                    position[p.set] = Some(sets.len() - 1);
                    sets.len() - 1
                }
            };
            let wants_occlusion = template.images[p.image]
                .scalars()
                .contains(&yolu_core::export::ExportScalar::Occlusion);
            if wants_occlusion && sets[slot].occlusion.is_none() {
                white = true;
            }
            files.push((slot, p.image, p.file_name.clone()));
        }
        if no_model {
            notes.push(Note::NoModel);
        }
        if white {
            notes.push(Note::WhiteOcclusion);
        }
        Ok(Plan { sets, files, notes })
    }

    pub fn export_apply(&mut self, action: ExportAction) {
        let lang = self.lang;
        match action {
            ExportAction::Template(id) => {
                if self.is_stroking() {
                    self.message = lang
                        .pick("描いている間はできません。", "Not while drawing.")
                        .into();
                    return;
                }
                if ExportTemplate::built_in_by_id(&id).is_none() {
                    self.message = lang.pick(
                        format!("書き出しのテンプレート「{id}」はありません。"),
                        format!("No export template \"{id}\"."),
                    );
                    return;
                }
                self.dialog_request = Some(DialogRequest::ExportFolder(id));
            }
            ExportAction::TemplateTo { id, dir } => self.start_export(&id, &dir, false),
            ExportAction::ConfirmReplace => {
                if let Some(c) = self.export.confirm.take() {
                    self.start_export(&c.template, &c.dir, true);
                }
            }
            ExportAction::CancelConfirm => {
                if self.export.confirm.take().is_some() {
                    self.message = lang
                        .pick(
                            "書き出しをやめました（何も書いていません）。",
                            "Export canceled (nothing was written).",
                        )
                        .into();
                }
            }
            ExportAction::Cancel => {
                if let Some(job) = &self.export.job {
                    job.cancel.store(true, Ordering::Relaxed);
                    self.message = lang
                        .pick("書き出しを取り消しています…", "Canceling the export…")
                        .into();
                }
            }
            ExportAction::DismissReport => self.export.report = None,
        }
    }

    fn start_export(&mut self, id: &str, dir: &Path, replace: bool) {
        let lang = self.lang;
        if self.is_stroking() {
            self.message = lang
                .pick("描いている間はできません。", "Not while drawing.")
                .into();
            return;
        }
        if self.export.job.is_some() {
            self.message = lang
                .pick("書き出し中です。", "An export is running.")
                .into();
            return;
        }
        let Some(template) = ExportTemplate::built_in_by_id(id) else {
            self.message = lang.pick(
                format!("書き出しのテンプレート「{id}」はありません。"),
                format!("No export template \"{id}\"."),
            );
            return;
        };
        let stem = stem(self);
        let plan = match self.plan_export(&template, &stem) {
            Ok(p) => p,
            Err(e) => {
                self.message = e;
                return;
            }
        };
        if !replace {
            let existing = existing_files(dir, plan.files.iter().map(|f| f.2.as_str()));
            if !existing.is_empty() {
                self.export.confirm = Some(Confirm {
                    template: id.to_owned(),
                    dir: dir.to_path_buf(),
                    existing: existing
                        .iter()
                        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                        .collect(),
                    total: plan.files.len(),
                });
                self.message = lang.pick(
                    format!(
                        "もうあるファイル {} 個を置き換えるか確かめます。",
                        existing.len()
                    ),
                    format!("Confirm replacing {} existing file(s).", existing.len()),
                );
                return;
            }
        }
        let reach = Reach::from_setting(self.export.padding).unwrap_or(Reach::Fill);
        let total = plan.files.len();
        let done = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let input = WorkerInput {
            dir: dir.to_path_buf(),
            template: template.clone(),
            sets: plan.sets,
            files: plan.files,
            overwrite: if replace {
                Overwrite::Replace
            } else {
                Overwrite::Refuse
            },
            reach,
            done: done.clone(),
            cancel: cancel.clone(),
            park: std::mem::take(&mut self.export.park_next),
        };
        let spawned = std::thread::Builder::new()
            .name("yolu-export".into())
            .spawn(move || {
                let _ = tx.send(run(input));
            });
        if let Err(e) = spawned {
            self.message = format!("{e}");
            return;
        }
        self.message = lang.pick(
            format!("書き出し中: {}（{total} 枚）…", template.name),
            format!("Exporting: {} ({total} images)…", template.name),
        );
        self.export.job = Some(Job {
            template: template.name,
            dir: dir.to_path_buf(),
            total,
            done,
            cancel,
            notes: plan.notes,
            rx,
        });
    }

    /// 終わった書き出しを受ける（フレームの初めに）。
    pub fn poll_export(&mut self) {
        let lang = self.lang;
        let Some(job) = &self.export.job else {
            return;
        };
        let result = match job.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ExportError::Io(
                lang.pick("書き出しが止まりました", "The export stopped")
                    .into(),
            )),
        };
        let job = self.export.job.take().expect("上で見た");
        match result {
            Ok(images) => {
                let mut text = lang.pick(
                    format!(
                        "書き出しました: {} 枚（{}）→ {}。",
                        images.len(),
                        job.template,
                        job.dir.display()
                    ),
                    format!(
                        "Exported {} image(s) ({}) to {}.",
                        images.len(),
                        job.template,
                        job.dir.display()
                    ),
                );
                for n in &job.notes {
                    text += &format!(" {}。", note_text(lang, n));
                }
                self.message = text;
                self.export.report = Some(Report {
                    template: job.template,
                    dir: job.dir,
                    images,
                    notes: job.notes,
                });
            }
            Err(ExportError::Cancelled) => {
                self.message = lang
                    .pick(
                        "書き出しを取り消しました（元のファイルは変えていません）。",
                        "Export canceled (no file was changed).",
                    )
                    .into();
            }
            Err(e) => {
                self.message = lang.pick(
                    format!("書き出せません: {e}"),
                    format!("Cannot export: {e}"),
                );
            }
        }
    }

    /// 試験用: 書き出しが終わるまで待って受ける（待ちの上限は 120 秒）。
    #[doc(hidden)]
    pub fn wait_export(&mut self) {
        let start = Instant::now();
        while self.export.job.is_some() {
            self.poll_export();
            if self.export.job.is_none() {
                break;
            }
            assert!(
                start.elapsed().as_secs() < 120,
                "書き出しが終わらない（ハング検出上限）"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

struct WorkerInput {
    dir: PathBuf,
    template: ExportTemplate,
    sets: Vec<SetInput>,
    files: Vec<(usize, usize, String)>,
    overwrite: Overwrite,
    reach: Reach,
    done: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    park: bool,
}

/// 別のスレッドの本体: 写した正本を文書に戻し、塗り広げの覆いを作り、全部の画像を 1 回の `write_images` で書く。
fn run(input: WorkerInput) -> Result<Vec<WrittenImage>, ExportError> {
    let WorkerInput {
        dir,
        template,
        sets,
        files,
        overwrite,
        reach,
        done,
        cancel,
        park,
    } = input;
    if park {
        crate::windows::park_until_canceled(&cancel);
    }
    let docs = sets
        .iter()
        .map(|s| {
            s.native
                .to_core()
                .map_err(|e| ExportError::Io(format!("写した文書を戻せません: {e}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut coverage: Vec<Option<Vec<bool>>> = Vec::with_capacity(sets.len());
    for (set, doc) in sets.iter().zip(&docs) {
        if cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
        coverage.push(match &set.uv {
            Some(tris) => {
                let c = padding::coverage(doc.width(), doc.height(), tris.iter().copied())
                    .map_err(ExportError::from)?;
                c.contains(&true).then_some(c)
            }
            None => None,
        });
    }
    let export_files: Vec<ExportFile<'_>> = files
        .iter()
        .map(|(set, image, name)| ExportFile {
            name: name.clone(),
            image: &template.images[*image],
            width: docs[*set].width(),
            height: docs[*set].height(),
        })
        .collect();
    let options = WriteOptions {
        overwrite,
        cancel: Some(&cancel),
    };
    write_images(&dir, &export_files, &options, |i| {
        done.store(i, Ordering::Relaxed);
        let (set, image, _) = &files[i];
        let doc = &docs[*set];
        let pixels = build(
            doc,
            &template.images[*image],
            sets[*set].occlusion.as_deref(),
            WORKING_BYTES,
        )
        .map_err(|e: CalcError| ExportError::from(e))?;
        match &coverage[*set] {
            Some(keep) => Ok(padding::dilate_cancellable(
                &pixels,
                doc.width(),
                doc.height(),
                keep,
                reach,
                WORKING_BYTES,
                Some(&cancel),
            )?),
            None => Ok(pixels),
        }
    })
}

#[cfg(test)]
mod tests;
