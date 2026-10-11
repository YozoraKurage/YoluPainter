//! 新規プロジェクトのウィンドウ（Ctrl+N）と、プロジェクトの構成（ファイル ▸ プロジェクト設定）。Unity 版の `NewProjectWindow` と同じ 1 つのウィンドウで、
//! 新規では「テンプレート・モデル・どのマテリアルをテクスチャセットにするか・解像度・法線の形式・作った後のメッシュマップのベイク」を、
//! 構成では今のプロジェクトを直す（セットの追加・削除・名前・マテリアル・大きさ、モデルの差し替えと読み直し、法線の形式）。
//!
//! - モデル（FBX）は別のスレッドで読む（取消と「もう一度」つき）。読み終えたモデルはウィンドウが持ち、決めたとき 3D ビューにそのまま入れる
//!   （2 回読まない）。ウィンドウを閉じれば捨てる。元のファイルは読むだけで、書き換えない。
//! - マテリアルの組は `model::SceneModel` の記録から決める（同じマテリアルを使うメッシュは 1 つの組 = 1 つのセット）。セットの
//!   マテリアルの鍵と、モデルへの照合の順は `sets::match_materials`（識別子 → 未割り当て → 名前 → スロットの番号。.ylp と同じ）。
//! - 構成の適用は、失敗しうる大きさの変更を先に全セット分準備し（1 つでも断られたら何も変えない）、そのあと失敗しない
//!   モデルとセットの入れ替えをする。消すセット・大きさを変えるセット・モデルの差し替えは、適用の前に一覧で確かめる（取り消せない）。
//! - 開いているプロジェクトのモデルのファイルは `NpState::model_file` に持ち、保存で .ylp の view.json に残して（`reopen`）、開くと
//!   読み直す。Live Link のモデル（Unity のシーンのもの）は同じ照合で結び付くだけで、ここでは替えない（新規のウィンドウは、モデルを選んでいなければ、
//!   そのマテリアルの組を出して、選んだ分のセットを作る）。
//!
//! 画面に説明文は置かない: 名前・状態・短い理由だけで、説明はツールチップ。

mod configure;
mod create;
pub mod reopen;
pub mod window;

use std::path::{Path, PathBuf};

pub use configure::{resampling_name, unused_groups, ConfirmKind, Plan, PlanRow};
use egui::Vec2;
pub use reopen::{is_network_path, relative_model_path, resolve_model_path, Reopen};

use crate::engine::{CanvasResampling, Channel, Document, NormalSettings, NormalYDirection};
use crate::jobs::{JobCard, JobSpec};
use crate::lang::Lang;
use crate::model::SceneModel;
use crate::notice::Source;
use crate::sets::{match_materials, material_from_link, name_for, MaterialRef};
use crate::state::{AppState, DialogRequest};
use crate::view3d::pose::{self, PrepareJob, PreparedModel};
use yolu_protocol::MaterialKey as LinkKey;

/// 解像度の選択肢（一辺。Unity 版と同じ）。
pub const RESOLUTIONS: [u32; 5] = [512, 1024, 2048, 4096, 8192];
/// 1 つのプロジェクトのテクスチャセットの上限（.ylp の決まり）。
pub const MAX_SETS: usize = yolu_io::MAX_PROJECT_SETS;
/// セットの名前の長さ（UTF-16 の数。.ylp の決まり）。
pub const MAX_NAME: usize = 256;
/// 新規プロジェクトの解像度の初めの値。
pub const DEFAULT_RESOLUTION: u32 = 2048;

/// 新規プロジェクトの始め方（最初のレイヤーで使うチャンネル。Unity 版の `ProjectTemplate` と同じ並び）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Template {
    #[default]
    Pbr,
    LilToon,
    ColorOnly,
}

impl Template {
    pub const ALL: [Template; 3] = [Template::Pbr, Template::LilToon, Template::ColorOnly];

    /// 最初のレイヤーで有効にするチャンネル（列挙の順）。
    pub fn channels(self) -> &'static [Channel] {
        match self {
            Template::Pbr => &[
                Channel::Color,
                Channel::Roughness,
                Channel::Metallic,
                Channel::Height,
                Channel::Normal,
                Channel::Emission,
            ],
            Template::LilToon => &[Channel::Color, Channel::Normal, Channel::Emission],
            Template::ColorOnly => &[Channel::Color],
        }
    }

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Template::Pbr => lang.pick("PBR（すべてのチャンネル）", "PBR (all channels)"),
            Template::LilToon => lang.pick(
                "lilToon（カラー・ノーマル・エミッション）",
                "lilToon (Color, Normal, Emission)",
            ),
            Template::ColorOnly => lang.pick("カラーのみ", "Color only"),
        }
    }

    /// ツールチップ: 使うチャンネルの並び（`doc` はチャンネルの名前を引くだけ）。
    pub fn tooltip(self, lang: Lang, doc: &Document) -> String {
        let names: Vec<String> = self
            .channels()
            .iter()
            .map(|c| crate::m2::channel_name(lang, doc, *c))
            .collect();
        format!(
            "{}: {}",
            lang.pick("チャンネル", "Channels"),
            names.join(" · ")
        )
    }
}

/// モデルのマテリアルの組（同じマテリアルを使うメッシュ・サブメッシュは 1 つ）。
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// モデルの記録のマテリアルの番号。
    pub index: usize,
    /// 画面と、セットの既定の名前に使う名前（マテリアルの無い組は Unassigned）。
    pub name: String,
    /// .ylp に残す鍵。
    pub key: MaterialRef,
    /// Live Link と同じ形の鍵（照合に使う）。
    pub link_key: LinkKey,
    /// このマテリアルを使うメッシュの名前。
    pub meshes: Vec<String>,
    /// このマテリアルを使うスロット（メッシュ × サブメッシュ）の数。
    pub slots: usize,
}

/// モデルの記録からマテリアルの組を作る。
pub fn groups_of(model: &SceneModel) -> Vec<Group> {
    model
        .materials
        .iter()
        .enumerate()
        .map(|(index, info)| Group {
            index,
            name: set_name_for(&info.key),
            key: material_from_link(&info.key),
            link_key: info.key.clone(),
            meshes: model
                .material_meshes
                .get(index)
                .cloned()
                .unwrap_or_default(),
            slots: model.slots.iter().filter(|&&m| m as usize == index).count(),
        })
        .collect()
}

/// マテリアルの組から付けるセットの既定の名前（制御文字を除き、256 文字まで。空なら Material）。
fn set_name_for(key: &LinkKey) -> String {
    let clean: String = name_for(key).chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim();
    if clean.is_empty() {
        return "Material".into();
    }
    let mut out = String::new();
    for c in clean.chars() {
        if out.encode_utf16().count() + c.len_utf16() > MAX_NAME {
            break;
        }
        out.push(c);
    }
    out
}

/// 鍵の並びを、モデルのマテリアルの組に照合する（`sets::match_materials` と同じ順。1 つの組は 1 つの鍵だけ）。返すのは鍵ごとの組の番号。
pub fn match_keys(keys: &[&MaterialRef], groups: &[Group], slots: &[u32]) -> Vec<Option<usize>> {
    let materials: Vec<LinkKey> = groups.iter().map(|g| g.link_key.clone()).collect();
    let by_material = match_materials(keys, &materials, slots);
    let mut out = vec![None; keys.len()];
    for (group, set) in by_material.iter().enumerate() {
        if let Some(set) = set {
            out[*set] = Some(group);
        }
    }
    out
}

/// 選び直していないときにテクスチャセットにするマテリアルの組（先頭から上限まで）。
fn default_chosen(count: usize) -> Vec<usize> {
    (0..count.min(MAX_SETS)).collect()
}

/// ウィンドウで編むテクスチャセット 1 つ（`uid` が None なら足すセット）。
#[derive(Clone, Debug, PartialEq)]
pub struct SetDraft {
    pub uid: Option<u32>,
    pub name: String,
    /// 描くマテリアル: ウィンドウが見せるマテリアルの組の番号。None は今のモデルに無い（鍵のまま）。
    pub material: Option<usize>,
    /// セットのマテリアルの鍵（足すセットは選んだ組の鍵）。モデルを替えたとき、これで新しいモデルへ付け直す。
    pub key: MaterialRef,
    /// 欲しい大きさ（画素）。
    pub size: (u32, u32),
    /// 開いたときの大きさ（足すセットは (0, 0)）。
    pub current: (u32, u32),
    pub read_only: bool,
}

impl SetDraft {
    /// 大きさを変えるか（もうあるセットで、今と違う）。
    pub fn resizes(&self) -> bool {
        self.uid.is_some() && self.size != self.current
    }
}

/// ウィンドウのモデルの読み込みの様子。
pub enum Prep {
    /// 読んでいない（モデルを決めていない）。
    Idle,
    Loading {
        path: PathBuf,
        job: PrepareJob,
    },
    Ready {
        path: PathBuf,
        model: Box<PreparedModel>,
        /// マテリアルの組を読むための記録（3D ビューには入れていない）。
        scene: Box<SceneModel>,
    },
    Failed {
        path: PathBuf,
        error: crate::view3d::model::ViewError,
    },
    /// 利用者が取り消した（「もう一度」で読み直せる）。
    Canceled {
        path: PathBuf,
    },
}

/// ウィンドウの中で開いているドロップダウン。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dropdown {
    Template,
    Resolution,
    Normal,
    Resampling,
    /// セットの下書きの大きさ・マテリアル（下書きの番号）。
    DraftSize(usize),
    DraftMaterial(usize),
}

/// 新規プロジェクト・プロジェクトの構成のウィンドウの状態。
pub struct NpWindow {
    /// false なら新規プロジェクト。
    pub configure: bool,
    // 新規
    pub template: Template,
    pub bake: bool,
    /// テクスチャセットにするマテリアルの組の番号（None は全部）。
    pub materials: Option<Vec<usize>>,
    // 共通
    pub resolution: u32,
    pub normal: NormalYDirection,
    /// ウィンドウを開いたときの法線の形式（構成。セットごとに持てるので、利用者が `normal` を変えたときだけ全セットへ当てる）。
    pub normal_opened: NormalYDirection,
    // モデル: 新規は選んだモデル。構成は替える・読み直すモデル（None なら今のまま）
    pub model: Option<PathBuf>,
    pub prep: Prep,
    /// 構成: 同じモデルを読み直す。
    pub reload: bool,
    // 構成
    pub drafts: Vec<SetDraft>,
    /// ウィンドウを開いたときのセットの並び（uid）。ウィンドウの外でセットが増減したら（Live Link のモデルが来たなど）、下書きが古いのでウィンドウを閉じる。
    pub sets_seen: Vec<u32>,
    pub resampling: Option<CanvasResampling>,
    /// 適用の前の確かめ（あれば一覧のウィンドウを出す）。
    pub confirm: Option<Plan>,
    /// 決められない理由（下の帯の左に出す）。
    pub error: Option<String>,
    // 画面
    pub offset: Vec2,
    pub list_scroll: f32,
    pub dropdown: Option<(Dropdown, crate::ui::menu::PopupState)>,
    pub confirm_offset: Vec2,
}

impl NpWindow {
    fn blank(configure: bool) -> NpWindow {
        NpWindow {
            configure,
            template: Template::default(),
            bake: false,
            materials: None,
            resolution: DEFAULT_RESOLUTION,
            normal: NormalYDirection::OpenGL,
            normal_opened: NormalYDirection::OpenGL,
            model: None,
            prep: Prep::Idle,
            reload: false,
            drafts: Vec::new(),
            sets_seen: Vec::new(),
            resampling: None,
            confirm: None,
            error: None,
            offset: Vec2::ZERO,
            list_scroll: 0.0,
            dropdown: None,
            confirm_offset: Vec2::ZERO,
        }
    }

    /// ウィンドウが参照する今のモデル（読み終えたモデルの代わりに、ウィンドウの外のモデルを見せるとき）: 構成では今のモデル、新規ではモデルを選んで
    /// いないときの Live Link のモデル（Unity のシーンのもの。ここでは替えないが、そのマテリアルにセットを作れる）。
    fn app_model<'a>(&self, app: &'a AppState) -> Option<&'a SceneModel> {
        if self.configure {
            app.model.as_ref()
        } else if self.model.is_none() {
            app.model.as_ref().filter(|m| m.is_link())
        } else {
            None
        }
    }

    /// ウィンドウが見せるマテリアルの組: 読み終えたモデルがあればそれ。なければ構成では今のモデル、新規では Live Link のモデル（選んでいなければ）。
    pub fn groups(&self, app: &AppState) -> Vec<Group> {
        match &self.prep {
            Prep::Ready { scene, .. } => groups_of(scene),
            _ => self.app_model(app).map(groups_of).unwrap_or_default(),
        }
    }

    /// `groups` のスロットの表（照合に使う）。
    pub fn slots(&self, app: &AppState) -> Vec<u32> {
        match &self.prep {
            Prep::Ready { scene, .. } => scene.slots.clone(),
            _ => self
                .app_model(app)
                .map(|m| m.slots.clone())
                .unwrap_or_default(),
        }
    }

    /// 新規: テクスチャセットにするマテリアルの組の番号（番号の順）。選び直していなければ、先頭から上限（`MAX_SETS`）まで。
    pub fn chosen(&self, count: usize) -> Vec<usize> {
        let mut chosen: Vec<usize> = match &self.materials {
            Some(m) => m.iter().copied().filter(|g| *g < count).collect(),
            None => default_chosen(count),
        };
        chosen.sort_unstable();
        chosen.dedup();
        chosen.truncate(MAX_SETS);
        chosen
    }

    /// 上限のため、テクスチャセットにならないマテリアルの組の数（上限を超えるマテリアルを持つモデルだけ）。
    pub fn over_limit(&self, count: usize) -> usize {
        if count > MAX_SETS && self.chosen(count).len() == MAX_SETS {
            count - MAX_SETS
        } else {
            0
        }
    }

    /// 読み込みの知らせ（近似したもの・飛ばしたもの）。
    pub fn notes(&self) -> &[String] {
        match &self.prep {
            Prep::Ready { model, .. } => model.warnings(),
            _ => &[],
        }
    }

    /// ウィンドウが読んでいる・読んだ・読めなかったモデルのファイル。
    pub fn model_path(&self) -> Option<&Path> {
        match &self.prep {
            Prep::Loading { path, .. }
            | Prep::Ready { path, .. }
            | Prep::Failed { path, .. }
            | Prep::Canceled { path } => Some(path),
            Prep::Idle => None,
        }
    }

    /// モデルを読んでいる最中か。
    pub fn is_loading(&self) -> bool {
        matches!(self.prep, Prep::Loading { .. })
    }

    /// 決めてよいか（モデルを選んでいるなら、読み終えている）。
    pub fn is_ready(&self) -> bool {
        self.model.is_none() || matches!(self.prep, Prep::Ready { .. })
    }

    /// 構成: セットの下書きのマテリアルを、ウィンドウが見せるマテリアルの組に鍵で照合し直す（モデルを替えた・戻したとき）。
    fn rematch_drafts(&mut self, app: &AppState) {
        let groups = self.groups(app);
        let slots = self.slots(app);
        let keys: Vec<&MaterialRef> = self.drafts.iter().map(|d| &d.key).collect();
        let matched = match_keys(&keys, &groups, &slots);
        for (draft, group) in self.drafts.iter_mut().zip(matched) {
            draft.material = group;
        }
        self.error = None;
    }

    /// ウィンドウに落とした・選んだモデルを読み始める（前の読み込みは取り消す）。
    fn start_loading(&mut self, app: &mut AppState, path: PathBuf) {
        if let Prep::Loading { job, .. } = &self.prep {
            job.cancel();
        }
        let job = pose::prepare_fbx(&mut app.view3d, &path, yolu_model::ModelLimits::default());
        self.model = Some(path.clone());
        self.prep = Prep::Loading { path, job };
        self.error = None;
        // 読み終えるまで、下書きは今のモデルのマテリアルに付けておく（前に読んだ別のモデルの番号のまま残さない）
        if self.configure {
            self.rematch_drafts(app);
        }
    }

    fn stop_loading(&mut self) {
        if let Prep::Loading { job, .. } = &self.prep {
            job.cancel();
        }
    }
}

impl Drop for NpWindow {
    fn drop(&mut self) {
        // ウィンドウを閉じたら、読んでいる途中のモデルは止める（結果は捨てる）
        self.stop_loading();
    }
}

/// 開いている・保存するプロジェクトのモデルについての、ウィンドウの外の状態。
#[derive(Default)]
pub struct NpState {
    pub window: Option<NpWindow>,
    /// 今のプロジェクトのモデルのファイル（FBX）。Live Link・試しの人形・モデル無しは None。保存で view.json に残し、構成の
    /// 「読み直す」が使う。ファイルが見つからなくても、参照は残す（保存で失わない）。
    pub model_file: Option<PathBuf>,
    /// .ylp を開いたときにモデルを読んでいる（読み終えたら結び付ける）。
    pub reopening: Option<Reopen>,
    /// プロジェクトを替えるたびに増える（読み終えたモデルが、替わったあとのプロジェクトに入らないように）。
    pub generation: u64,
    /// テクスチャセットのパネルの「消す」の確かめを待っているセット（uid）。
    pub remove_confirm: Option<Vec<u32>>,
    pub remove_offset: Vec2,
}

impl std::fmt::Debug for NpState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NpState")
            .field("window", &self.window.is_some())
            .field("model_file", &self.model_file)
            .field("generation", &self.generation)
            .finish()
    }
}

/// 下書きを直す操作。
#[derive(Clone, Debug, PartialEq)]
pub enum DraftOp {
    Name(String),
    Material(Option<usize>),
    Size(u32, u32),
    Remove,
}

/// 新規プロジェクト・プロジェクトの構成の操作（`Action::Project`）。ウィンドウの部品・メニュー・ボタンは同じ道を通す。
#[derive(Clone, Debug, PartialEq)]
pub enum NpAction {
    /// 新規プロジェクトのウィンドウを開く（保存していない変更の確かめは呼ぶ側）。
    OpenNew,
    /// プロジェクトの構成のウィンドウを開く。
    OpenConfigure,
    /// FBX を開く（3D ビューの「FBX を開く」・ウィンドウに落としたファイル）。何も触っていないプロジェクトなら新規プロジェクトのウィンドウに、
    /// 作業のあるプロジェクトならプロジェクトの構成のウィンドウで、そのモデルに替える。
    OpenModel(PathBuf),
    Close,
    /// ウィンドウのモデルを選ぶ（ファイルのウィンドウの結果）。
    ChooseModel(PathBuf),
    /// モデルを決めない（新規）／替えるのをやめる（構成）。
    ClearModel,
    /// 構成: 今のモデルのファイルを読み直す。
    Reload,
    CancelPrepare,
    PrepareAgain,
    Template(Template),
    Resolution(u32),
    Normal(NormalYDirection),
    BakeAfter(bool),
    /// 新規: マテリアルの組をテクスチャセットにするか。
    Material(usize, bool),
    Draft(usize, DraftOp),
    /// 構成: 空のセットを下書きに足す。
    AddDraft,
    /// 構成: セットの無いマテリアルの組に、空のセットを 1 つずつ下書きに足す。
    AddUnused,
    Resampling(Option<CanvasResampling>),
    /// 作る（新規）・適用する（構成。確かめがいるなら一覧のウィンドウが先に出る）。
    Submit,
    ConfirmApply,
    ConfirmCancel,
    /// テクスチャセットのパネル: 空のセットを足す。
    AddSet,
    /// テクスチャセットのパネル: 確かめて消す。
    RemoveSets(Vec<u32>),
    ConfirmRemove,
    CancelRemove,
    /// ウィンドウのモデルを選ぶファイルのウィンドウを頼む（ウィンドウは `YoluApp` が開く）。
    ChooseDialog,
    /// .ylp を開いたときに読んでいるモデルを取り消す。
    CancelReopen,
}

/// プロジェクトの構成のウィンドウのモデルの読み込みと、開いたあとのモデルの読み直し（読み直しは札を出す）。構成のウィンドウと、セットを消す確かめは、
/// キーの割り当てを止める。読み込みは閉じる前の確かめに入れない（読むだけの仕事。止めるのは FBX の読み込みの行 `view3d::pose::JOB`）。
pub(crate) const JOB: JobSpec = JobSpec {
    repaint: true,
    card: Some(|app, lang| {
        let r = app.np.reopening.as_ref()?;
        Some(JobCard {
            text: format!(
                "{} — {}",
                lang.pick("モデルを読み込み中", "Loading the model"),
                r.file_name()
            ),
            fraction: r.fraction(),
            cancel: Some(crate::state::Action::Project(NpAction::CancelReopen)),
            canceling: false,
        })
    }),
    modal: Some(|app| app.np.window.is_some() || app.np.remove_confirm.is_some()),
    ..JobSpec::new("model", AppState::np_is_busy)
};

impl AppState {
    /// 新規プロジェクト・プロジェクトの構成の操作を当てる。
    pub fn np_apply(&mut self, action: NpAction) {
        let lang = self.lang;
        let stroking = self.is_stroking();
        let refuse = |s: &mut AppState| {
            s.refuse(Source::Project, crate::lang::refusals::during_stroke(lang))
        };
        match action {
            NpAction::OpenNew => {
                if stroking {
                    return refuse(self);
                }
                self.np_open_new();
            }
            NpAction::OpenConfigure => {
                if stroking {
                    return refuse(self);
                }
                self.np_open_configure(None);
            }
            NpAction::OpenModel(path) => {
                if stroking {
                    return refuse(self);
                }
                self.np_open_model(path);
            }
            NpAction::Close => self.np.window = None,
            NpAction::ChooseDialog => self.dialog_request = Some(DialogRequest::ProjectModel),
            NpAction::CancelReopen => {
                if self.np.reopening.take().is_some() {
                    self.info(
                        Source::Project,
                        lang.pick(
                            "モデルの読み込みを取り消しました。",
                            "Model loading canceled.",
                        ),
                    );
                }
            }
            NpAction::AddSet => {
                if let Err(e) = self.add_texture_set() {
                    self.refuse(Source::Project, e);
                }
            }
            NpAction::RemoveSets(uids) => self.np_ask_remove(uids),
            NpAction::ConfirmRemove => {
                if let Some(uids) = self.np.remove_confirm.take() {
                    match self.remove_sets(&uids) {
                        Ok(names) => self.info(
                            Source::Project,
                            lang.pick(
                                format!("テクスチャセットを消しました: {}。", names.join("・")),
                                format!("Removed texture sets: {}.", names.join(", ")),
                            ),
                        ),
                        Err(e) => self.refuse(Source::Project, e),
                    }
                }
            }
            NpAction::CancelRemove => self.np.remove_confirm = None,
            other => self.np_window_action(other),
        }
    }

    /// ウィンドウを開いている間の操作（ウィンドウが無ければ何もしない）。
    fn np_window_action(&mut self, action: NpAction) {
        let Some(mut win) = self.np.window.take() else {
            return;
        };
        let keep = self.np_window_step(&mut win, action);
        if keep {
            self.np.window = Some(win);
        }
    }

    /// ウィンドウ 1 つへの操作。ウィンドウを残すなら true（作った・適用した・やめたなら false）。
    fn np_window_step(&mut self, win: &mut NpWindow, action: NpAction) -> bool {
        match action {
            NpAction::ChooseModel(path) => {
                if win.configure && self.model.as_ref().is_some_and(|m| m.is_link()) {
                    // Live Link のモデルは Unity のシーンのもの（ここでは替えない）
                } else {
                    // モデルが替わると、見せた確かめの一覧（計画）は古い。落としたファイルは確かめが出ている間にも届く
                    win.confirm = None;
                    win.reload = false;
                    win.materials = None;
                    win.start_loading(self, path);
                    win.dropdown = None;
                }
            }
            NpAction::ClearModel => {
                win.confirm = None;
                win.stop_loading();
                win.model = None;
                win.reload = false;
                win.prep = Prep::Idle;
                win.materials = None;
                if win.configure {
                    win.rematch_drafts(self);
                }
            }
            NpAction::Reload => {
                if win.reload {
                    // もう一度押すと、読み直すのをやめる
                    return self.np_window_step(win, NpAction::ClearModel);
                }
                if let Some(path) = self.np.model_file.clone() {
                    win.confirm = None;
                    win.materials = None;
                    win.start_loading(self, path);
                    win.reload = true;
                }
            }
            NpAction::CancelPrepare => {
                if let Prep::Loading { path, job } = &win.prep {
                    job.cancel();
                    win.confirm = None;
                    win.prep = Prep::Canceled { path: path.clone() };
                }
            }
            NpAction::PrepareAgain => {
                if let Some(path) = win.model_path().map(Path::to_path_buf) {
                    win.confirm = None;
                    win.start_loading(self, path);
                }
            }
            NpAction::Template(t) => win.template = t,
            NpAction::Resolution(r) => {
                if RESOLUTIONS.contains(&r) {
                    win.resolution = r;
                }
            }
            NpAction::Normal(n) => win.normal = n,
            NpAction::BakeAfter(on) => win.bake = on,
            NpAction::Material(group, on) => {
                let count = win.groups(self).len();
                let mut chosen = win.chosen(count);
                win.error = None;
                if on {
                    if !chosen.contains(&group) && group < count {
                        if chosen.len() >= MAX_SETS {
                            win.error = Some(crate::lang::refusals::set_limit(self.lang));
                            return true;
                        }
                        chosen.push(group);
                    }
                } else if chosen.len() > 1 {
                    chosen.retain(|g| *g != group);
                }
                chosen.sort_unstable();
                win.materials = (chosen != default_chosen(count)).then_some(chosen);
            }
            NpAction::Draft(i, op) => configure::draft_op(self, win, i, op),
            NpAction::AddDraft => configure::add_draft(self, win),
            NpAction::AddUnused => configure::add_unused(self, win),
            NpAction::Resampling(r) => win.resampling = r,
            NpAction::Submit => return self.np_submit(win),
            NpAction::ConfirmCancel => win.confirm = None,
            NpAction::ConfirmApply => {
                // 確かめの一覧が出ていないときの決定は何もしない（出さずに適用しない）。見せた一覧と今の計画が違うとき（確かめの間に
                // ウィンドウが変わった）は適用せず、今の計画の一覧を出し直す
                if let Some(shown) = win.confirm.take() {
                    match configure::plan(self, win) {
                        Ok(now) if now == shown => return self.np_run(win),
                        Ok(now) => win.confirm = now.needs_confirm().then_some(now),
                        Err(e) => win.error = Some(e),
                    }
                }
            }
            NpAction::Close => return false,
            // ウィンドウの外の操作はここへ来ない
            _ => {}
        }
        true
    }

    fn np_submit(&mut self, win: &mut NpWindow) -> bool {
        win.error = None;
        if !win.configure {
            return self.np_run(win);
        }
        match configure::plan(self, win) {
            Ok(plan) if plan.needs_confirm() => {
                win.confirm = Some(plan);
                true
            }
            Ok(_) => self.np_run(win),
            Err(e) => {
                win.error = Some(e);
                true
            }
        }
    }

    /// 作る・適用する（確かめは済んだ）。うまくいけばウィンドウを閉じ（false）、断られたら理由をウィンドウに出して残す（true）。
    fn np_run(&mut self, win: &mut NpWindow) -> bool {
        let result = if win.configure {
            configure::apply(self, win)
        } else {
            create::create_from_window(self, win)
        };
        match result {
            Ok((kind, message)) => {
                self.notify(kind, Source::Project, message);
                false
            }
            Err(e) => {
                win.error = Some(e);
                true
            }
        }
    }

    fn np_open_new(&mut self) {
        // 今のモデルのファイルがあれば、新しいプロジェクトの初めのモデルにする（Unity 版と同じ。「×」で外せる）。ネットワークのパスで
        // 残っている参照（開くときに自動では触らなかったもの）は、利用者が選んだモデルではないので、確かめにも読み込みにも使わない
        let model = self
            .np
            .model_file
            .clone()
            .filter(|p| !is_network_path(&p.to_string_lossy()))
            .filter(|p| p.is_file());
        self.np_open_new_with(model);
    }

    /// 新規プロジェクトのウィンドウを開く。`model` があれば初めのモデルとして読み始める。
    fn np_open_new_with(&mut self, model: Option<PathBuf>) {
        let mut win = NpWindow::blank(false);
        if let Some(path) = model {
            win.start_loading(self, path);
        }
        self.np.window = Some(win);
    }

    /// 構成のウィンドウを開く。`model` があればそのモデルに替える下書きで開く。
    fn np_open_configure(&mut self, model: Option<PathBuf>) {
        let mut win = NpWindow::blank(true);
        win.normal = self.doc.normal_settings().file_direction();
        win.normal_opened = win.normal;
        win.resolution = nearest_resolution(self.doc.width());
        win.sets_seen = self.sets.iter().map(|s| s.uid).collect();
        let groups = self.model.as_ref().map(groups_of).unwrap_or_default();
        for i in 0..self.sets.len() {
            let set = self.sets.get(i).expect("範囲内");
            let doc = self.set_doc(i);
            win.drafts.push(SetDraft {
                uid: Some(set.uid),
                name: set.name.clone(),
                material: set.bound.map(|m| m as usize).filter(|m| *m < groups.len()),
                key: set.material.clone(),
                size: (doc.width(), doc.height()),
                current: (doc.width(), doc.height()),
                read_only: set.read_only.is_some(),
            });
        }
        if let Some(path) = model {
            win.start_loading(self, path);
        }
        self.np.window = Some(win);
    }

    /// FBX を開く。何も触っていないプロジェクトなら、その FBX で新規プロジェクトを作るウィンドウ。作業のあるプロジェクトなら、構成のウィンドウで
    /// そのモデルに替える下書き（Live Link のモデルが付いているときは、Unity のシーンのモデルを替えないので、新規のウィンドウ）。
    fn np_open_model(&mut self, path: PathBuf) {
        // ウィンドウが開いていれば、そのウィンドウのモデルにする
        if self.np.window.is_some() {
            self.np_window_action(NpAction::ChooseModel(path));
            return;
        }
        if self.is_pristine() || self.model.as_ref().is_some_and(|m| m.is_link()) {
            self.np_open_new_with(Some(path));
        } else {
            self.np_open_configure(Some(path));
        }
    }

    /// 開いた・保存したファイルが無く、描いていない・変えていない（最初のままの）プロジェクトか。
    pub fn is_pristine(&self) -> bool {
        self.project.is_none()
            && !self.modified
            && self.sets.len() == 1
            && !self.doc.can_undo()
            && self.sets.current().read_only.is_none()
            && !self
                .doc
                .layers()
                .iter()
                .any(crate::engine::layer_has_pixels)
    }

    /// 毎フレーム: ウィンドウ・.ylp を開いたときのモデルの読み込みの終わりを受ける。
    pub fn poll_newproject(&mut self) {
        // 構成のウィンドウを開いている間に、ウィンドウの外でセットが増減した（Live Link のモデルが来た・開き直した）: 下書きが古いので閉じる
        if self.np.window.as_ref().is_some_and(|w| {
            w.configure
                && !w
                    .sets_seen
                    .iter()
                    .copied()
                    .eq(self.sets.iter().map(|s| s.uid))
        }) {
            self.np.window = None;
            self.warn(
                Source::Project,
                self.lang.pick(
                    "プロジェクトが変わったので、プロジェクト設定を閉じました。",
                    "The project changed; Project Configuration was closed.",
                ),
            );
        }
        if let Some(mut win) = self.np.window.take() {
            if let Prep::Loading { path, job } = &win.prep {
                if let Some(result) = job.poll() {
                    let path = path.clone();
                    win.prep = match result {
                        Ok(model) => {
                            let scene = SceneModel::from_rig_as(model.rig(), 0);
                            Prep::Ready {
                                path,
                                model: Box::new(model),
                                scene: Box::new(scene),
                            }
                        }
                        Err(error) => Prep::Failed { path, error },
                    };
                    if win.configure {
                        win.rematch_drafts(self);
                    }
                }
            }
            self.np.window = Some(win);
        }
        reopen::poll(self);
    }

    /// ウィンドウが裏の仕事（モデルの読み込み）をしているか（描き直しを続ける）。
    pub fn np_is_busy(&self) -> bool {
        self.np.window.as_ref().is_some_and(NpWindow::is_loading) || self.np.reopening.is_some()
    }

    /// テクスチャセットのパネルの「消す」: 確認のウィンドウを出す（最後の 1 つは消せない）。
    fn np_ask_remove(&mut self, uids: Vec<u32>) {
        let lang = self.lang;
        let uids: Vec<u32> = uids
            .into_iter()
            .filter(|u| self.sets.index_of(*u).is_some())
            .collect();
        if uids.is_empty() {
            return;
        }
        if uids.len() >= self.sets.len() {
            self.refuse(
                Source::Project,
                lang.pick(
                    "プロジェクトには少なくとも 1 つのテクスチャセットが要ります。",
                    "A project keeps at least one texture set.",
                ),
            );
            return;
        }
        if self.is_stroking() {
            self.refuse(Source::Project, crate::lang::refusals::during_stroke(lang));
            return;
        }
        self.np.remove_confirm = Some(uids);
    }

    /// 今のプロジェクトを替えるとき（新規・開く・作る）: 前のプロジェクトのモデルの読み込みを止め、世代を進める。Live Link の裏の仕事は、
    /// `project_epoch` の違いで見分けて止める（`LiveLink::poll`）。
    pub fn np_project_replaced(&mut self) {
        self.project_epoch += 1;
        self.np.generation += 1;
        self.np.reopening = None;
        self.np.window = None;
        self.np.remove_confirm = None;
    }

    /// ポーズを付けられるモデル（FBX・試しの人形）を外し、3D ビューを試しの立方体に戻す（新しい・開いたプロジェクトが、前のプロジェクトの
    /// モデルを引きずらないように）。Live Link のモデルも、ポーズのセッションごと外れる（相手は `LiveLink::poll` が忘れる）。
    pub fn drop_project_model(&mut self) {
        self.np.model_file = None;
        let rig_model = self
            .model
            .as_ref()
            .is_some_and(|m| matches!(m.source, crate::model::ModelSource::Rig { .. }));
        if rig_model || self.view3d.pose.session.is_some() {
            self.model = None;
            self.view3d.load_demo();
            self.view3d.pose.session = None;
            self.view3d.pose.drag = None;
            crate::mode::leave_pose_without_rig(self);
        }
    }
}

/// 解像度の選択肢のうち、辺に一番近いもの。
pub fn nearest_resolution(side: u32) -> u32 {
    *RESOLUTIONS
        .iter()
        .min_by_key(|r| r.abs_diff(side))
        .expect("選択肢は空でない")
}

/// 大きさの表示（正方形は一辺、ほかは 幅×高さ）。
pub fn size_text(size: (u32, u32)) -> String {
    if size.0 == size.1 {
        size.0.to_string()
    } else {
        format!("{}×{}", size.0, size.1)
    }
}

/// 新しい空のセットの文書: 大きさ・使うチャンネル・Normal の設定を指定した、空のレイヤー 1 枚（履歴なし）。
pub fn new_set_document(
    width: u32,
    height: u32,
    tile_size: u32,
    lang: Lang,
    channels: &[Channel],
    normal: NormalSettings,
) -> Result<Document, String> {
    let core = |e: crate::engine::CoreError| lang.core_error(&e);
    let mut doc = Document::with_tile_size(width, height, tile_size).map_err(core)?;
    let layer = doc
        .add_layer(&format!("{} 1", lang.pick("レイヤー", "Layer")))
        .map_err(core)?;
    for channel in channels {
        doc.set_channel_enabled(layer, *channel, true)
            .map_err(core)?;
    }
    if doc.normal_settings() != normal {
        doc.set_normal_settings(normal, false).map_err(core)?;
    }
    doc.clear_history().map_err(core)?;
    crate::look::apply_new_set_look(&mut doc);
    Ok(doc)
}

/// 文書のどれかのレイヤーが使っているチャンネル（番号の順。何も使っていなければ Color）。新しい空のセットが同じチャンネルを使う。
pub fn used_channels(doc: &Document) -> Vec<Channel> {
    let mut used: Vec<Channel> = doc
        .layers()
        .iter()
        .flat_map(|l| l.enabled_channels())
        .filter(|c| c.is_standard())
        .collect();
    used.sort();
    used.dedup();
    if used.is_empty() {
        used.push(Channel::Color);
    }
    used
}

/// 名前を大文字小文字を区別せず重ならないように（「 2」「 3」…）。
pub(crate) fn unique<'a>(base: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    crate::sets::unique_name(base, taken)
}

/// 試験用: ウィンドウが開いているか。
pub fn is_open(app: &AppState) -> bool {
    app.np.window.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sets::MaterialAsset;
    use yolu_protocol::{MaterialInfo, MeshData, Model, Submesh};

    fn info(key: LinkKey) -> MaterialInfo {
        MaterialInfo {
            key,
            shader: String::new(),
            textures: vec![],
            routes: vec![],
        }
    }

    fn named(name: &str) -> LinkKey {
        LinkKey::Material {
            name: name.into(),
            asset: None,
        }
    }

    fn scene(materials: Vec<LinkKey>, meshes: &[(&str, &[u32])]) -> SceneModel {
        SceneModel::from_link(&Model {
            generation: 1,
            name: "試し".into(),
            materials: materials.into_iter().map(info).collect(),
            meshes: meshes
                .iter()
                .map(|(name, mats)| MeshData {
                    key: (*name).into(),
                    name: (*name).into(),
                    skinned: false,
                    positions: vec![[0.0; 3]; 3],
                    normals: vec![],
                    uv0: vec![],
                    submeshes: mats
                        .iter()
                        .map(|m| Submesh {
                            material: *m,
                            indices: vec![0, 1, 2],
                        })
                        .collect(),
                })
                .collect(),
        })
    }

    #[test]
    fn groups_list_the_meshes_that_share_a_material_once() {
        let model = scene(
            vec![named("Skin"), LinkKey::Unassigned, named("")],
            &[
                ("Body", &[0, 1]),
                ("Face", &[0]),
                ("Body2", &[0, 0]),
                ("Gap", &[2]),
            ],
        );
        let groups = groups_of(&model);
        assert_eq!(groups.len(), 3);
        assert_eq!(
            groups[0].meshes,
            ["Body", "Face", "Body2"],
            "同じメッシュは 1 回、並びの順"
        );
        assert_eq!(groups[0].slots, 4);
        assert_eq!(groups[1].name, "Unassigned");
        assert_eq!(groups[1].key, MaterialRef::Unassigned);
        assert_eq!(groups[2].name, "Material", "空の名前は Material");
    }

    #[test]
    fn keys_are_matched_to_groups_in_the_unity_order() {
        let model = scene(
            vec![
                LinkKey::Material {
                    name: "Skin".into(),
                    asset: Some(("a".repeat(32), 7)),
                },
                named("Hair"),
                LinkKey::Unassigned,
                named("Eye"),
            ],
            &[("m", &[0, 1, 2, 3])],
        );
        let groups = groups_of(&model);
        let by_identity = MaterialRef::Material {
            name: "Old name".into(),
            asset: Some(MaterialAsset {
                guid: "a".repeat(32),
                file_id: 7,
            }),
        };
        let by_name_ignoring_case = MaterialRef::Material {
            name: "hair".into(),
            asset: None,
        };
        let unassigned = MaterialRef::Unassigned;
        let by_slot = MaterialRef::PendingSlot(3);
        let gone = MaterialRef::Material {
            name: "Gone".into(),
            asset: None,
        };
        let keys = [
            &gone,
            &by_slot,
            &unassigned,
            &by_name_ignoring_case,
            &by_identity,
        ];
        let got = match_keys(&keys, &groups, &model.slots);
        assert_eq!(got, [None, Some(3), Some(2), Some(1), Some(0)]);
    }

    #[test]
    fn a_new_set_document_has_the_template_channels_and_no_history() {
        let normal = NormalSettings::DEFAULT.with_file_direction(NormalYDirection::DirectX);
        let doc =
            new_set_document(64, 32, 16, Lang::En, Template::LilToon.channels(), normal).unwrap();
        assert_eq!((doc.width(), doc.height(), doc.tile_size()), (64, 32, 16));
        assert_eq!(doc.layers().len(), 1);
        assert_eq!(doc.layers()[0].name(), "Layer 1");
        assert_eq!(
            used_channels(&doc),
            [Channel::Color, Channel::Normal, Channel::Emission]
        );
        assert_eq!(
            doc.normal_settings().file_direction(),
            NormalYDirection::DirectX
        );
        assert!(!doc.can_undo());
        // 何も使っていない文書は Color
        let empty = Document::new(8, 8).unwrap();
        assert_eq!(used_channels(&empty), [Channel::Color]);
    }

    #[test]
    fn sizes_and_resolutions_read_short() {
        assert_eq!(size_text((512, 512)), "512");
        assert_eq!(size_text((256, 128)), "256×128");
        assert_eq!(nearest_resolution(1000), 1024);
        assert_eq!(nearest_resolution(5000), 4096);
        assert_eq!(nearest_resolution(64), 512);
    }
}
