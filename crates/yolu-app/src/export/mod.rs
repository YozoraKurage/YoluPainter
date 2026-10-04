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
//! - **チャンネルの画像**（ファイル → 書き出し）: 描くチャンネルを PNG に、全部のセットの使っている全チャンネルをフォルダに。値は
//!   `yolu_core::export::channel_image`（詰めない・色を掛けない合成そのまま、Normal は文書の Y の向き）で、名前は
//!   `<名前>[_<セット名>]_<チャンネル>.png`（チャンネルは言語によらず英語の綴り）。書く手順・余白・取消はテンプレートと同じ道を通す。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

use egui::Vec2;
use yolu_core::export::{
    build, channel_image, ExportError as CalcError, ExportImage, ExportImageKind, ExportScalar,
    ExportTemplate,
};
use yolu_core::glam::DVec2;
use yolu_core::padding::{self, Reach};
use yolu_core::{Channel, ChannelKind, ColorSpace, Document};
use yolu_io::export::{
    clashes, existing_files, file_name, plan_template, sanitize, write_images, ExportError,
    ExportFile, Overwrite, PlanSet, WriteOptions, WrittenImage,
};
use yolu_io::NativeDocument;

use crate::bake::Occlusion;
use crate::state::{AppState, DialogRequest};

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
    /// 描くチャンネルを PNG に。書き出す先のファイルを選ぶ窓を頼む。
    ChannelDialog,
    /// 書き出す先のファイルが決まった（もうあるファイルは、選ぶ窓が置き換えてよいと確かめている）。
    ChannelTo(PathBuf),
    /// 書き出す先のファイルに、選ぶ窓が確かめていない名前（利用者が打った名前に拡張子を足したもの）が決まった。もうあれば、置き換える前に
    /// 確かめの窓を出す。
    ChannelNamed(PathBuf),
    /// 全部のテクスチャセットの使っている全チャンネルを画像に。書き出す先のフォルダを選ぶ窓を頼む。
    ChannelsDialog,
    /// 書き出す先のフォルダが決まった。もうあるファイルがあれば、確かめの窓を出す。
    ChannelsTo(PathBuf),
}

/// 何を書き出すか（確かめの窓の「置き換える」が、同じものをもう一度計画するのに使う）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum What {
    /// テンプレート（ID）の画像。
    Template(String),
    /// 全チャンネルの画像。
    Channels,
    /// 描くチャンネルの 1 枚の PNG（書き出し先のファイル）。
    ChannelFile(PathBuf),
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
    /// 効いていない効果（読むマップ・画像が使えない）があり、書き出した画像には入っていない（セット・効果）。
    InactiveEffects(String, Vec<yolu_core::InactiveEffect>),
}

/// もうあるファイルの確かめ（置き換えるか）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub what: What,
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
    /// 画面に出す名前（テンプレートの名前・「チャンネル」）。
    template: String,
    dir: PathBuf,
    /// 結果の窓を出すか（1 枚のチャンネルの PNG は、状態の帯だけ。注意も帯の文に入る）。
    report: bool,
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
        Note::InactiveEffects(set, effects) => {
            let first = effects.first().map(|e| lang.inactive_effect(e)).unwrap_or_default();
            lang.pick(
                format!("「{set}」の効いていない効果 {} 件は書き出しに入っていません: {first}", effects.len()),
                format!("{} inactive effect(s) of \"{set}\" are not in the exported images: {first}", effects.len()),
            )
        }
    }
}

/// 1 セットぶんの書き出しの入力（始めるときに写す）。
struct SetInput {
    native: NativeDocument,
    /// 効果の入力（焼いたマップ・モデルのルート・画像。正本には入らないので、写した文書へ渡し直す）。
    inputs: yolu_core::EffectInputs,
    occlusion: Option<Vec<u8>>,
    /// UV の三角形（画素の座標）。塗り広げないなら None。
    uv: Option<Vec<[DVec2; 3]>>,
}

/// 書く画像 1 枚。
struct PlannedImage {
    /// `Plan::sets` の中の位置。
    set: usize,
    /// `Plan::template` の画像の番号（ファイル名の接尾辞・取り込みの種類の元）。
    image: usize,
    name: String,
    /// チャンネルの画像（`yolu_core::export::channel_image`）か。None ならテンプレートの画像（`build`）。
    channel: Option<Channel>,
}

struct Plan {
    sets: Vec<SetInput>,
    files: Vec<PlannedImage>,
    /// テンプレートの書き出しはそのテンプレート、チャンネルの書き出しは、書く画像ごとの取り決め（接尾辞・種類）だけを持つ入れ物。
    template: ExportTemplate,
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

/// チャンネルの画像のファイル名の末尾。標準のチャンネルは言語によらず英語の綴り（Unity 版と同じ）、ユーザーチャンネルは名前（使えない文字は
/// `_`。空なら番号）。
pub fn channel_suffix(doc: &Document, channel: Channel) -> String {
    if let Some(name) = channel.standard_name() {
        return name.to_owned();
    }
    let name = doc
        .channel_info(channel)
        .map(|i| sanitize(&i.name))
        .unwrap_or_default();
    if name.is_empty() || name.starts_with('.') {
        format!("Channel{}", channel.index())
    } else {
        name
    }
}

/// 選ぶ窓が返したファイルから、書き出しの操作を決める。拡張子が無ければ `.png` を足す（窓の種類で付かない環境がある）。足した名前は
/// 窓が確かめていないので `ChannelNamed`（もうあれば置き換える前に確かめる）、付いていればそのまま（窓が確かめた名前）。
pub fn channel_action(chosen: PathBuf) -> ExportAction {
    if chosen.extension().is_none() {
        ExportAction::ChannelNamed(chosen.with_extension("png"))
    } else {
        ExportAction::ChannelTo(chosen)
    }
}

/// 描くチャンネルの PNG の、書き出す先を選ぶ窓に出す初めのファイル名（全チャンネルの書き出しと同じ決まり）。
pub fn default_channel_file_name(state: &AppState) -> String {
    let doc = &state.doc;
    let image = channel_image_spec(doc, state.m2.paint_channel);
    let set = (state.sets.len() > 1).then(|| state.sets.current().name.as_str());
    file_name(&stem(state), set, &image).unwrap_or_else(|_| "Texture.png".into())
}

/// チャンネルの画像の取り決め（接尾辞と、取り込みの種類。書く中身は `channel_image` が作る）。
fn channel_image_spec(doc: &Document, channel: Channel) -> ExportImage {
    let suffix = channel_suffix(doc, channel);
    let info = doc.channel_info(channel);
    match info {
        _ if channel == Channel::Color => ExportImage::of(suffix, ExportImageKind::BaseColor),
        Some(i) if i.kind == ChannelKind::Normal => {
            ExportImage::of(suffix, ExportImageKind::Normal)
        }
        Some(i) if i.color_space == ColorSpace::Srgb => {
            ExportImage::of(suffix, ExportImageKind::Emission)
        }
        _ => ExportImage::pack(
            suffix,
            ExportScalar::Zero,
            ExportScalar::Zero,
            ExportScalar::Zero,
            ExportScalar::One,
        ),
    }
}

/// チャンネルの書き出しの範囲。
enum Which<'a> {
    /// 全部のセットの使っている全チャンネル。
    All { stem: &'a str },
    /// 1 つのセットの 1 つのチャンネルを、決まったファイル名で。
    One {
        set: usize,
        channel: Channel,
        file_name: String,
    },
}

impl AppState {
    /// 書き出すセット（読むだけのセットは除く）の番号。除いたセットは注意に積む。
    fn exportable_sets(&self, notes: &mut Vec<Note>) -> Vec<usize> {
        let mut indices = Vec::new();
        for i in 0..self.sets.len() {
            let set = self.sets.get(i).expect("範囲内");
            if set.read_only.is_some() {
                notes.push(Note::ReadOnly(set.name.clone()));
            } else {
                indices.push(i);
            }
        }
        indices
    }

    /// 計画に出たセット（`indices` の位置。出てきた順）の文書を写し、塗り広げの UV を決める（このスレッド。写せなければ理由）。
    /// 返すのは写した入力と、`indices` の位置からその入力の位置への対応。UV の無いセット・モデルの無いときは注意に積む。
    #[allow(clippy::type_complexity)]
    fn copy_sets(
        &mut self,
        indices: &[usize],
        used: &[usize],
        occlusions: &[Option<Vec<u8>>],
        notes: &mut Vec<Note>,
    ) -> Result<(Vec<SetInput>, Vec<Option<usize>>), String> {
        let lang = self.lang;
        let reach = Reach::from_setting(self.export.padding).map_err(|e| e.to_string())?;
        let padding_on = reach != Reach::Texels(0);
        let model = self.view3d.full_model().cloned();
        let mut sets: Vec<SetInput> = Vec::new();
        let mut position: Vec<Option<usize>> = vec![None; indices.len()];
        let mut no_model = false;
        for &p in used {
            let index = indices[p];
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
            // 効かない効果は黙って入力のまま書かず、書き出した画像に入っていないことを言う
            let inactive = self.set_doc(index).inactive_effect_list();
            if !inactive.is_empty() {
                notes.push(Note::InactiveEffects(name.clone(), inactive));
            }
            let inputs = self.set_doc(index).effect_inputs().clone();
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
                inputs,
                occlusion: occlusions[p].clone(),
                uv,
            });
            position[p] = Some(sets.len() - 1);
        }
        if no_model {
            notes.push(Note::NoModel);
        }
        Ok((sets, position))
    }

    /// 書き出す画像と名前を決め、文書を写す（このスレッド。写せなければ理由）。
    fn plan_export(&mut self, template: &ExportTemplate, stem: &str) -> Result<Plan, String> {
        let lang = self.lang;
        Reach::from_setting(self.export.padding).map_err(|e| e.to_string())?;
        let mut notes = Vec::new();
        let indices = self.exportable_sets(&mut notes);
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
            return Err(self.clash_message(&clash));
        }
        // 写す（planned に出たセットだけ）
        let mut used: Vec<usize> = Vec::new();
        for p in &planned {
            if !used.contains(&p.set) {
                used.push(p.set);
            }
        }
        let (sets, position) = self.copy_sets(&indices, &used, &occlusions, &mut notes)?;
        let mut files = Vec::with_capacity(planned.len());
        let mut white = false;
        for p in &planned {
            let slot = position[p.set].expect("使うセットは写した");
            let wants_occlusion = template.images[p.image]
                .scalars()
                .contains(&yolu_core::export::ExportScalar::Occlusion);
            if wants_occlusion && sets[slot].occlusion.is_none() {
                white = true;
            }
            files.push(PlannedImage {
                set: slot,
                image: p.image,
                name: p.file_name.clone(),
                channel: None,
            });
        }
        if white {
            notes.push(Note::WhiteOcclusion);
        }
        Ok(Plan {
            sets,
            files,
            template: template.clone(),
            notes,
        })
    }

    /// 同じファイルになる名前があるときの断りの文。
    fn clash_message(&self, clash: &[String]) -> String {
        self.lang.pick(
            format!(
                "書き出しませんでした: 同じファイルになる画像があります: {}",
                clash.join("、")
            ),
            format!(
                "Nothing was exported: these images would write the same file: {}",
                clash.join(", ")
            ),
        )
    }

    /// チャンネルの画像（描くチャンネルの 1 枚、または全セットの全チャンネル）と名前を決め、文書を写す（このスレッド）。
    fn plan_channels(&mut self, which: Which<'_>) -> Result<Plan, String> {
        let lang = self.lang;
        Reach::from_setting(self.export.padding).map_err(|e| e.to_string())?;
        let mut notes = Vec::new();
        // (indices の位置, チャンネル, ファイル名)
        let mut wanted: Vec<(usize, Channel, String)> = Vec::new();
        let indices = match &which {
            Which::All { .. } => self.exportable_sets(&mut notes),
            Which::One { set, .. } => {
                if let Some(reason) = self.sets.get(*set).and_then(|s| s.read_only.clone()) {
                    return Err(format!(
                        "{}: {reason}",
                        lang.pick("読むだけのテクスチャセットです", "Read-only texture set")
                    ));
                }
                vec![*set]
            }
        };
        let several = self.sets.len() > 1;
        let mut images: Vec<ExportImage> = Vec::new();
        let mut image_of: Vec<usize> = Vec::new();
        for (position, &index) in indices.iter().enumerate() {
            let doc = self.set_doc(index);
            let channels: Vec<Channel> = match &which {
                Which::All { .. } => doc
                    .channels()
                    .into_iter()
                    .filter(|c| yolu_core::export::uses(doc, *c))
                    .collect(),
                Which::One { channel, .. } => vec![*channel],
            };
            for channel in channels {
                let image = channel_image_spec(doc, channel);
                let name = match &which {
                    Which::All { stem } => file_name(
                        stem,
                        several.then(|| self.sets.get(index).expect("範囲内").name.as_str()),
                        &image,
                    )
                    .map_err(|e| e.to_string())?,
                    Which::One { file_name, .. } => file_name.clone(),
                };
                wanted.push((position, channel, name));
                image_of.push(images.len());
                images.push(image);
            }
        }
        if wanted.is_empty() {
            return Err(lang
                .pick(
                    "書き出すものがありません: どのレイヤーもチャンネルを使っていません",
                    "Nothing to export: no layer uses any channel",
                )
                .into());
        }
        let clash = clashes(wanted.iter().map(|w| w.2.as_str()));
        if !clash.is_empty() {
            return Err(self.clash_message(&clash));
        }
        let mut used: Vec<usize> = Vec::new();
        for (position, _, _) in &wanted {
            if !used.contains(position) {
                used.push(*position);
            }
        }
        let occlusions = vec![None; indices.len()];
        let (sets, position) = self.copy_sets(&indices, &used, &occlusions, &mut notes)?;
        let files = wanted
            .into_iter()
            .zip(image_of)
            .map(|((set, channel, name), image)| PlannedImage {
                set: position[set].expect("使うセットは写した"),
                image,
                name,
                channel: Some(channel),
            })
            .collect();
        Ok(Plan {
            sets,
            files,
            template: ExportTemplate::new("channels", "channels", images),
            notes,
        })
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
            ExportAction::TemplateTo { id, dir } => {
                self.start_export(&What::Template(id), &dir, false)
            }
            ExportAction::ConfirmReplace => {
                if let Some(c) = self.export.confirm.take() {
                    match &c.what {
                        What::ChannelFile(path) => self.start_channel_png(path),
                        what => self.start_export(what, &c.dir, true),
                    }
                }
            }
            ExportAction::ChannelDialog => {
                if self.is_stroking() {
                    self.message = lang
                        .pick("描いている間はできません。", "Not while drawing.")
                        .into();
                    return;
                }
                self.dialog_request = Some(DialogRequest::ExportChannel);
            }
            ExportAction::ChannelTo(path) => self.start_channel_png(&path),
            ExportAction::ChannelNamed(path) => self.confirm_channel_png(path),
            ExportAction::ChannelsDialog => {
                if self.is_stroking() {
                    self.message = lang
                        .pick("描いている間はできません。", "Not while drawing.")
                        .into();
                    return;
                }
                self.dialog_request = Some(DialogRequest::ExportChannelsFolder);
            }
            ExportAction::ChannelsTo(dir) => self.start_export(&What::Channels, &dir, false),
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

    /// 書き出しの作業（画像を作る・塗り広げる）が使えるメモリ。設定の「1 回の操作」（自動は物理メモリから）。
    pub fn export_working_bytes(&self) -> u64 {
        self.prefs.settings.budgets(self.prefs.ram_mib).stroke
    }

    /// 書き出せる状態か（描いていない・ほかの書き出しが走っていない）。だめなら理由を知らせて false。
    fn export_ready(&mut self) -> bool {
        let lang = self.lang;
        if self.is_stroking() {
            self.message = lang
                .pick("描いている間はできません。", "Not while drawing.")
                .into();
            return false;
        }
        if self.export.job.is_some() {
            self.message = lang
                .pick("書き出し中です。", "An export is running.")
                .into();
            return false;
        }
        true
    }

    /// テンプレートか全チャンネルの画像を `dir` へ書き出す。もうあるファイルがあれば（`replace` でなければ）、確かめの窓を出す。
    fn start_export(&mut self, what: &What, dir: &Path, replace: bool) {
        let lang = self.lang;
        if !self.export_ready() {
            return;
        }
        let stem = stem(self);
        let planned = match what {
            What::Template(id) => match ExportTemplate::built_in_by_id(id) {
                Some(template) => self.plan_export(&template, &stem),
                None => {
                    self.message = lang.pick(
                        format!("書き出しのテンプレート「{id}」はありません。"),
                        format!("No export template \"{id}\"."),
                    );
                    return;
                }
            },
            What::Channels => self.plan_channels(Which::All { stem: &stem }),
            // 1 枚のファイルは確かめの窓の「置き換える」で始まる（`ConfirmReplace`）。ここへは来ない
            What::ChannelFile(path) => {
                self.start_channel_png(path);
                return;
            }
        };
        let plan = match planned {
            Ok(p) => p,
            Err(e) => {
                self.message = e;
                return;
            }
        };
        if !replace {
            let existing = existing_files(dir, plan.files.iter().map(|f| f.name.as_str()));
            if !existing.is_empty() {
                self.export.confirm = Some(Confirm {
                    what: what.clone(),
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
        let label = match what {
            What::Template(_) => plan.template.name.clone(),
            What::Channels => lang.pick("チャンネル", "Channels").into(),
            What::ChannelFile(path) => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        };
        self.launch(plan, dir, replace, label, true);
    }

    /// 拡張子を足した名前の PNG: もうあれば、置き換える前に確かめる（選ぶ窓は、足す前の名前しか確かめていない）。無ければそのまま書く。
    fn confirm_channel_png(&mut self, path: PathBuf) {
        let lang = self.lang;
        if !self.export_ready() {
            return;
        }
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            self.start_channel_png(&path);
            return;
        };
        let dir = match path.parent() {
            Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
            _ => PathBuf::from("."),
        };
        if existing_files(&dir, std::iter::once(name.as_str())).is_empty() {
            self.start_channel_png(&path);
            return;
        }
        self.export.confirm = Some(Confirm {
            what: What::ChannelFile(path),
            dir,
            existing: vec![name],
            total: 1,
        });
        self.message = lang.pick(
            "もうあるファイル 1 個を置き換えるか確かめます。",
            "Confirm replacing 1 existing file.",
        )
        .into();
    }

    /// 描くチャンネルを `path` の 1 枚の PNG に書き出す（選ぶ窓が置き換えてよいと確かめているので、もうあれば置き換える）。
    fn start_channel_png(&mut self, path: &Path) {
        let lang = self.lang;
        if !self.export_ready() {
            return;
        }
        let Some(file_name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            self.message = lang
                .pick("書き出す先のファイルがありません。", "No file to write to.")
                .into();
            return;
        };
        let dir = match path.parent() {
            Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let set = self.sets.current_index();
        let channel = self.m2.paint_channel;
        let plan = match self.plan_channels(Which::One {
            set,
            channel,
            file_name: file_name.clone(),
        }) {
            Ok(p) => p,
            Err(e) => {
                self.message = e;
                return;
            }
        };
        self.launch(plan, &dir, true, file_name, false);
    }

    /// 計画を別のスレッドで書き始める（`label` は状態の帯・結果に出す名前。`report` なら終わりに結果の窓を出す）。
    fn launch(&mut self, plan: Plan, dir: &Path, replace: bool, label: String, report: bool) {
        let lang = self.lang;
        let reach = Reach::from_setting(self.export.padding).unwrap_or(Reach::Fill);
        let total = plan.files.len();
        let done = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let input = WorkerInput {
            dir: dir.to_path_buf(),
            template: plan.template,
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
            working_bytes: self.export_working_bytes(),
            source_bytes: self.load_source_bytes(),
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
        self.message = if total == 1 {
            lang.pick(
                format!("書き出し中: {label}…"),
                format!("Exporting: {label}…"),
            )
        } else {
            lang.pick(
                format!("書き出し中: {label}（{total} 枚）…"),
                format!("Exporting: {label} ({total} images)…"),
            )
        };
        self.export.job = Some(Job {
            template: label,
            dir: dir.to_path_buf(),
            report,
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
                let mut text = if job.report {
                    lang.pick(
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
                    )
                } else {
                    // 1 枚のファイルは、書いたファイルの場所を出す
                    let path = images
                        .first()
                        .map(|i| i.path.display().to_string())
                        .unwrap_or_default();
                    lang.pick(
                        format!("書き出しました: {path}。"),
                        format!("Exported {path}."),
                    )
                };
                for n in &job.notes {
                    text += &format!(" {}。", note_text(lang, n));
                }
                self.message = text;
                if job.report {
                    self.export.report = Some(Report {
                        template: job.template,
                        dir: job.dir,
                        images,
                        notes: job.notes,
                    });
                }
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
    files: Vec<PlannedImage>,
    overwrite: Overwrite,
    reach: Reach,
    done: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    park: bool,
    /// 画像を作る作業と塗り広げの作業のメモリの上限（設定の「1 回の操作」。Unity 版が塗り広げに渡す StrokeBudgetBytes と同じ）。
    working_bytes: u64,
    /// 写した文書を戻すときに 1 つの文書に許す層の画素（設定の「レイヤーの画素」。256 MiB を下回らない）。
    source_bytes: u64,
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
        working_bytes,
        source_bytes,
    } = input;
    if park {
        crate::windows::park_until_canceled(&cancel);
    }
    let docs = sets
        .iter()
        .map(|s| {
            let mut doc = s
                .native
                .to_core_within(Some(source_bytes))
                .map_err(|e| ExportError::Io(format!("写した文書を戻せません: {e}")))?;
            // 正本は効果の入力を持たないので、写す前の文書に渡していた入力を渡し直す（渡さないと、Generator・画像が入力のまま通る）
            doc.set_effect_inputs(s.inputs.clone())
                .map_err(|e| ExportError::Io(format!("効果の入力を渡せません: {e}")))?;
            Ok::<_, ExportError>(doc)
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
        .map(|f| ExportFile {
            name: f.name.clone(),
            image: &template.images[f.image],
            width: docs[f.set].width(),
            height: docs[f.set].height(),
        })
        .collect();
    let options = WriteOptions {
        overwrite,
        cancel: Some(&cancel),
    };
    write_images(&dir, &export_files, &options, |i| {
        done.store(i, Ordering::Relaxed);
        let file = &files[i];
        let doc = &docs[file.set];
        let pixels = match file.channel {
            Some(channel) => channel_image(doc, channel, working_bytes),
            None => build(
                doc,
                &template.images[file.image],
                sets[file.set].occlusion.as_deref(),
                working_bytes,
            ),
        }
        .map_err(|e: CalcError| ExportError::from(e))?;
        match &coverage[file.set] {
            Some(keep) => Ok(padding::dilate_cancellable(
                &pixels,
                doc.width(),
                doc.height(),
                keep,
                reach,
                working_bytes,
                Some(&cancel),
            )?),
            None => Ok(pixels),
        }
    })
}

#[cfg(test)]
mod tests;
