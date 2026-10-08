//! ポーズの変更: FBX（や試しの人形）のスキンを持ち、ポーズを変えたら core でスキニングし、休みの形のスナップショットの位置だけを
//! 差し替えて（隣り合わせはそのまま、BVH は refit）3D ビューのモデルにする。計算は core の `skin` と `geometry`、読み込みは
//! yolu-model。ここは状態（今のポーズ・選んだ骨・取り消しの並び）と、いつ組み直すかだけ。
//!
//! - ポーズの取り消しは画素の取り消しと別の並び（`undo`・`redo`）。ポーズは文書の画素と別の状態で、画素の履歴は
//!   文書ごとにメモリの予算で古いものが消えるので、混ぜると片方の都合で片方が消える。ポーズのモードの間は Ctrl+Z がこちらへ来る。
//! - ストロークの最中はポーズを変えない（描いている間の当たりと遮蔽の覚えは、そのスナップショットのもの）。断って知らせる。
//! - FBX は別のスレッドで読む（読み込み・変換・休みの形の組み立て）。読み終わったらフレームの初めに入れ替える。読み込みは途中で取り消せる
//!   （`loads`。利用者が取り消す・別のモデルを読み始める・結果の受け口を捨てる・アプリを終える）。取り消した読み込みは途中の物を捨て、今のモデルは
//!   前のまま（半分だけ入れない）。進み具合は読み込みのスレッドが書き、状態の表示が読む。
//! - 今のポーズは、プロジェクトのモデル（FBX）のものだけ `.ylp` の根の `pose.json` に残り、同じモデルを開くと戻る（`stored`。形式は上げない状態の
//!   エントリ）。変えると「変更あり」の印が付く（`sync_modified`）。名前を付けてモデルをまたいで使うのは個人の設定のフォルダのプリセット（`presets`）。
//! - ポーズの数値の編集と戻しは `edit`、ボーンの影響で面を隠す・隠し方のプリセットは `hide`、ポーズのプリセット（保存・当てる・左右反転）は
//!   `presets`、FBX のテイクとフレームからポーズにするのは `takes`（どれもポーズの取り消しの並びとは別の持ち物は持たない: 数値の編集・戻し・
//!   プリセットやテイクを当てるのは 1 つの取り消しの段、隠すのは見せ方の状態で取り消しの対象ではない）。

pub mod edit;
pub mod hide;
pub mod loads;
pub mod presets;
pub mod stored;
pub mod takes;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use yolu_core::geometry::{
    model_triangles, BvhUpdate, GeometryError, ModelMesh, SurfaceGeometry, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::skin::{demo_figure, FigureDetail, Pose, Rig};
use yolu_model::{load_fbx_with, LoadControl, ModelLimits};

use loads::LoadProgress;

use super::model::{ViewError, ViewModel};
use super::View3dState;
use crate::jobs::{JobSpec, Polled, Worker};
use crate::lang::Lang;
use crate::mode::EditorMode;
use crate::notice::{Kind, Source};
use crate::state::{AppState, DialogRequest};

/// 取り消しの並びの長さ（1 つはポーズの写し。骨 500・BlendShape 200 で約 20 KiB）。
pub const MAX_POSE_HISTORY: usize = 256;

/// ポーズの操作（メニュー・ボタン・キーから）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoseAction {
    /// ファイルを選ぶウィンドウを頼む（選ばれたら FBX を開く）。
    OpenFbx,
    /// 読んでいる FBX を取り消す（今のモデルは前のまま）。
    CancelLoad,
    /// 試しの人形を読む（試験の口。メニューには置かない）。
    LoadFigure,
    /// ポーズのモード（ギズモ）を入れる・切る。
    ToggleMode,
    /// ファイルにあったときのポーズへ戻す（取り消せる）。
    Reset,
    Undo,
    Redo,
    /// 欄でテイクを選ぶ（テイクを持つ FBX の番号・その中のテイクの番号）。
    ChooseTake(usize, usize),
    /// 選んでいるテイクとフレームのポーズにする（裏で求め、終わったら取り消しの 1 段として当てる）。
    ApplyTake,
}

/// 組み直しの時間（ミリ秒。状態の表示と試験用）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PoseTimings {
    pub skin_ms: f64,
    pub refit_ms: f64,
    pub total_ms: f64,
}

/// 読み込んだスキンとポーズ。
pub struct PoseSession {
    pub rig: Arc<Rig>,
    pose: Pose,
    rest: Arc<SurfaceGeometry>,
    undo: Vec<Pose>,
    redo: Vec<Pose>,
    /// 続けて変えている操作（ギズモ・スライダーのドラッグ）の始まりのポーズ。
    edit_start: Option<Pose>,
    pub selected: Option<usize>,
    /// 木で開いている骨。
    pub expanded: HashSet<usize>,
    pub tree_scroll: f32,
    /// 次のフレームで木をこの骨まで送る（3D ビューで選んだとき）。
    pub reveal: Option<usize>,
    /// このセッションが最後に入れたモデルの世代（ほかのモデルに替わったら、このセッションは終わる）。
    model_revision: u32,
    pub timings: PoseTimings,
    /// 読み込みの知らせ（近似・飛ばしたもの）。
    pub warnings: Vec<String>,
    /// インスペクターが見せるオイラー角（数値で入れた値を、同じ回転の別の表し方へ読み替えて見せない）。
    pub euler_hint: Option<edit::EulerHint>,
    /// ボーンの影響で隠す面の組み立て（手で足した項目と、入れているプリセット）。
    pub hide: hide::HideState,
    /// 最後にポーズのプリセットを当てたとき（開いたときにファイルのポーズを戻したときも）、このモデルの骨へ対応させられず飛ばした項目。
    pub preset_notes: Vec<presets::Skipped>,
    /// ポーズを変えた回数（取り消し・やり直し・戻すも数える。欄で選ぶテイクとフレームを変えたのも数える。このセッションが始まってから）。
    /// 「変更あり」の印と復旧の書き置きの鍵。
    pub edits: u64,
    /// FBX のテイク（一覧・欄の選び・求めている途中の仕事）。
    pub takes: takes::TakeState,
}

impl PoseSession {
    pub fn pose(&self) -> &Pose {
        &self.pose
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }
    pub fn is_editing(&self) -> bool {
        self.edit_start.is_some()
    }
    /// 休みのポーズと違うか。
    pub fn is_posed(&self) -> bool {
        self.pose != self.rig.rest_pose()
    }
    /// 休みの形のスナップショット（ポーズのたびに refit する元）。
    pub fn rest_geometry(&self) -> &Arc<SurfaceGeometry> {
        &self.rest
    }
}

struct Loaded {
    rig: Rig,
    rest: SurfaceGeometry,
    meshes: Vec<ModelMesh>,
    warnings: Vec<String>,
    takes: Vec<takes::TakeSource>,
}

struct Loading {
    name: String,
    /// 受け口を捨てたら、読み込みも止める（結果の行き先が無い）。
    worker: Worker<Result<Loaded, ViewError>>,
    progress: Arc<LoadProgress>,
    revision: u32,
}

/// 3D ビューのポーズの状態（`View3dState::pose`）。
#[derive(Default)]
pub struct PoseEditor {
    pub session: Option<PoseSession>,
    loading: Option<Loading>,
    /// ギズモのドラッグ（`super::gizmo`）。
    pub drag: Option<super::gizmo::GizmoDrag>,
    /// ギズモの上にポインタがある輪（強調して描く）。
    pub hover_axis: Option<usize>,
    /// 次のフレームで 3D ビューのタブを前に出す（FBX・試しの人形・Live Link のモデルが入ったとき）。
    pub focus: bool,
    /// 隠し方のプリセット（個人の設定のフォルダ。モデルをまたいで残る）。
    pub hide_presets: hide::store::Presets,
    /// ポーズの欄（ドックのタブ）の縦のスクロールと、前のフレームの中身の高さ。
    pub panel_scroll: f32,
    pub panel_content: f32,
    /// 隠し方を保存する名前の欄（決めた文字。空なら既定の名前）。
    pub hide_name: String,
    /// ポーズのプリセット（個人の設定のフォルダ。モデルをまたいで残る）。
    pub pose_presets: presets::store::Presets,
    /// ポーズを保存する名前の欄（決めた文字。空なら既定の名前）。
    pub preset_name: String,
    /// 名前を変えているプリセットと、その欄に初めのフォーカスを渡したか。
    pub preset_rename: Option<u32>,
    pub preset_rename_started: bool,
    /// 「変更あり」の印へ数え終えた、セッションの `edits`（フレームごとに、増えていれば印を付ける）。
    pub seen_edits: u64,
    /// このビューが始めた FBX の読み込みのスレッド（取り消し・終わるときに止まるのを待つため。結果の受け口とは別）。
    pub loads: loads::Loads,
}

/// このビューが始めた FBX の読み込み（終わる前に止める。読むだけの仕事なので、閉じる前の確かめには入れない）。登録した旗は、結果の
/// 受け口を捨てたあとのスレッドの分も持つ。
pub(crate) const JOB: JobSpec = JobSpec {
    cancel: Some(|app| {
        app.view3d.pose.cancel_loading();
        app.view3d.pose.cancel_loads();
    }),
    ..JobSpec::new("fbx", |app| app.view3d.pose.loads_running() > 0)
};

impl PoseEditor {
    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }
    /// 読んでいるファイルの名前。
    pub fn loading_name(&self) -> Option<&str> {
        self.loading.as_ref().map(|l| l.name.as_str())
    }
    /// 読んでいる FBX の進み具合（読み込みを始めていない・まだ知らせが無ければ None）。
    pub fn loading_fraction(&self) -> Option<f32> {
        self.loading.as_ref().and_then(|l| l.progress.fraction())
    }
    /// 読んでいる FBX を取り消す（結果は捨てる。今のモデルは前のまま）。
    pub fn cancel_loading(&mut self) {
        self.loading = None;
    }
    /// このビューが始めた読み込みを全部取り消す（ウィンドウの準備・.ylp を開いたときの読み込みを含む。終わるとき）。
    pub fn cancel_loads(&self) {
        self.loads.cancel_all();
    }
    /// まだ止まっていない読み込みのスレッドの数。
    pub fn loads_running(&self) -> usize {
        self.loads.running()
    }
}

/// 休みの形のメッシュとスナップショットを組む（読み込みのスレッドでも呼ぶ）。
fn build_rest(
    rig: &Rig,
    revision: u32,
    cancel: Option<&AtomicBool>,
) -> Result<(Vec<ModelMesh>, SurfaceGeometry), ViewError> {
    let cancelled = || cancel.is_some_and(|c| c.load(Ordering::Relaxed));
    if cancelled() {
        return Err(ViewError::Cancelled);
    }
    let meshes = rig.deform(&rig.rest_pose())?;
    if cancelled() {
        return Err(ViewError::Cancelled);
    }
    let triangles = model_triangles(&meshes).ok_or(ViewError::BadMeshIndex)?;
    if triangles.is_empty() {
        return Err(ViewError::NoTriangles);
    }
    let geometry = match cancel {
        Some(c) => {
            SurfaceGeometry::build_cancelable(triangles, revision, DEFAULT_WELD_TOLERANCE, c)
        }
        None => SurfaceGeometry::new(triangles, revision, DEFAULT_WELD_TOLERANCE),
    };
    let geometry = match geometry {
        Ok(g) => g,
        // 組む途中で止めたのは、ほかの失敗と区別して「取り消した」にする
        Err(GeometryError::Canceled) => return Err(ViewError::Cancelled),
        Err(e) => return Err(e.into()),
    };
    Ok((meshes, geometry))
}

fn install(view3d: &mut View3dState, loaded: Loaded) {
    let rig = Arc::new(loaded.rig);
    let rest = Arc::new(loaded.rest);
    let revision = rest.revision();
    let model = ViewModel::with_geometry(
        rig.name(),
        loaded.meshes,
        rig.materials().iter().cloned().map(Some).collect(),
        rest.clone(),
    );
    if view3d.material < 0 || view3d.material as usize >= model.materials.len().max(1) {
        view3d.material = 0;
    }
    let mut expanded = HashSet::new();
    // 根と、その下の 1 段を開いておく
    for r in rig.roots() {
        expanded.insert(r);
        expanded.extend(rig.children(r).iter().map(|&c| c as usize));
    }
    view3d.pose.session = Some(PoseSession {
        pose: rig.rest_pose(),
        rig,
        rest,
        undo: Vec::new(),
        redo: Vec::new(),
        edit_start: None,
        selected: None,
        expanded,
        tree_scroll: 0.0,
        reveal: None,
        model_revision: revision,
        timings: PoseTimings::default(),
        warnings: loaded.warnings,
        euler_hint: None,
        hide: hide::HideState::default(),
        preset_notes: Vec::new(),
        edits: 0,
        takes: takes::TakeState::new(loaded.takes),
    });
    view3d.pose.seen_edits = 0;
    view3d.pose.drag = None;
    // 前のモデルの隠す面は引き継がない（三角形の番号が別のモデルのもの）
    view3d.set_face_mask(None);
    view3d.set_model(model);
}

/// スキンをそのまま読む（試しの人形・試験。UI のスレッドで組む）。
pub fn load_rig(view3d: &mut View3dState, rig: Rig) -> Result<(), ViewError> {
    if view3d.input.stroke.is_some() {
        return Err(ViewError::Stroking);
    }
    let revision = view3d.next_revision();
    let (meshes, rest) = build_rest(&rig, revision, None)?;
    install(
        view3d,
        Loaded {
            rig,
            rest,
            meshes,
            warnings: Vec::new(),
            takes: Vec::new(),
        },
    );
    Ok(())
}

/// 試しの人形を読む。
pub fn load_figure(view3d: &mut View3dState) -> Result<(), ViewError> {
    load_rig(view3d, demo_figure(FigureDetail::SMALL))
}

/// 進み具合のうち、ファイルの読み込みと変換（yolu-model）が占める分（残りは休みの形の組み立て）。
const MODEL_SHARE: f32 = 0.85;

/// FBX を読んで休みの形まで組む（呼んだスレッドで。取消の旗は yolu-model の区切りと、組み立ての区切りで見る）。
fn load_blocking(
    path: &Path,
    limits: &ModelLimits,
    revision: u32,
    cancel: &AtomicBool,
    progress: &LoadProgress,
) -> Result<Loaded, ViewError> {
    let note = |fraction: f32| {
        progress.set(fraction * MODEL_SHARE);
        #[cfg(test)]
        loads::hold::tick(path, cancel);
    };
    let model = load_fbx_with(
        path,
        limits,
        LoadControl {
            cancel: Some(cancel),
            progress: Some(&note),
        },
    )?;
    if cancel.load(Ordering::Relaxed) {
        return Err(ViewError::Cancelled);
    }
    progress.set(MODEL_SHARE);
    let (meshes, rest) = build_rest(&model.rig, revision, Some(cancel))?;
    progress.set(1.0);
    let takes = takes::TakeSource::whole(
        path.to_path_buf(),
        model.takes,
        model.rig.bones().len(),
        model.rig.meshes().len(),
    );
    Ok(Loaded {
        rig: model.rig,
        rest,
        meshes,
        warnings: model.report.warnings,
        takes: takes.into_iter().collect(),
    })
}

/// 読み込みのスレッドを始める（結果の受け口と進み具合を返す。受け口を捨てると読み込みも止める）。スレッドは `view3d` に登録する
/// （終わるときに止まるのを待つため）。スレッドを作れなければ panic（`std::thread::spawn` と同じ）。
fn spawn_load<T: Send + 'static>(
    view3d: &mut View3dState,
    path: &Path,
    limits: ModelLimits,
    revision: u32,
    wrap: fn(Loaded) -> T,
) -> (Worker<Result<T, ViewError>>, Arc<LoadProgress>) {
    let cancel = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(LoadProgress::default());
    let finished = view3d.pose.loads.register(&cancel);
    let path: PathBuf = path.to_path_buf();
    let shared = progress.clone();
    let worker = Worker::spawn_on(
        std::thread::Builder::new().name("yolu-fbx-load".into()),
        cancel,
        move |tx, flag| {
            let _finished = finished;
            let _ =
                tx.send(load_blocking(&path, &limits, revision, flag.flag(), &shared).map(wrap));
        },
    )
    .expect("failed to spawn thread")
    .cancel_on_drop();
    (worker, progress)
}

/// 別のスレッドで読み終えた、まだ 3D ビューに入れていないモデル（新規プロジェクト・プロジェクトの構成のウィンドウが持ち、決めたときに
/// `install_prepared` で入れる。ウィンドウを閉じれば、入れずに捨てる）。
pub struct PreparedModel(Loaded);

impl PreparedModel {
    /// 読んだスキン（マテリアルの組・メッシュの名前をウィンドウが読む）。
    pub fn rig(&self) -> &Rig {
        &self.0.rig
    }
    /// 読み込みの知らせ（近似したもの・飛ばしたもの）。
    pub fn warnings(&self) -> &[String] {
        &self.0.warnings
    }
    /// 休みの形の三角形のスナップショット（3D のパスの指紋を、入れる前のモデルと比べるために読む）。
    pub fn geometry(&self) -> &SurfaceGeometry {
        &self.0.rest
    }
}

/// 読み込みの結果を受ける口（裏のスレッドと、取消の旗）。
pub struct PrepareJob {
    /// 受け口を捨てたら、読み込みも止める（ウィンドウを閉じた・別のモデルを読み始めた。結果の行き先が無い）。
    worker: Worker<Result<PreparedModel, ViewError>>,
    progress: Arc<LoadProgress>,
}

impl PrepareJob {
    /// 読み終わっていれば結果（まだなら None。スレッドが落ちたら読み込みが止まったとして返す）。
    pub fn poll(&self) -> Option<Result<PreparedModel, ViewError>> {
        match self.worker.poll() {
            Polled::Message(r) => Some(r),
            Polled::Empty => None,
            Polled::Lost => Some(Err(ViewError::LoadStopped)),
        }
    }
    /// 取り消す（読み込みは次の区切りで止まり、途中の物と結果は捨てる）。
    pub fn cancel(&self) {
        self.worker.cancel();
    }
    /// 進み具合（0〜1。読み込みのスレッドからまだ知らせが無ければ None）。
    pub fn fraction(&self) -> Option<f32> {
        self.progress.fraction()
    }
    /// 進み具合を知らせる（試験用: 終わらない読み込みの表示を確かめる）。
    #[doc(hidden)]
    pub fn report_progress(&self, fraction: f32) {
        self.progress.set(fraction);
    }
    /// 終わらない読み込み（試験用）。返す送り口を持っているあいだは読み込み中のまま、送れば（失敗で）終わる。
    #[doc(hidden)]
    pub fn parked() -> (
        PrepareJob,
        std::sync::mpsc::Sender<Result<PreparedModel, ViewError>>,
    ) {
        let (worker, tx) = Worker::parked();
        (
            PrepareJob {
                worker: worker.cancel_on_drop(),
                progress: Arc::new(LoadProgress::default()),
            },
            tx,
        )
    }
}

/// 別のスレッドで組んだスキン（Live Link の相手: FBX を並べたもの）と、呼び手の残りの結果。受け口を捨てたら、組むのも止める。
pub struct RigJob<T> {
    worker: Worker<Result<(PreparedModel, T), ViewError>>,
}

impl<T> RigJob<T> {
    /// 終わっていれば結果（まだなら None。スレッドが落ちたら読み込みが止まったとして返す）。
    pub fn poll(&self) -> Option<Result<(PreparedModel, T), ViewError>> {
        match self.worker.poll() {
            Polled::Message(r) => Some(r),
            Polled::Empty => None,
            Polled::Lost => Some(Err(ViewError::LoadStopped)),
        }
    }
    /// 取り消す。
    pub fn cancel(&self) {
        self.worker.cancel();
    }
    /// 試験用: 終わるまで待つ（上限 120 秒）。
    #[doc(hidden)]
    pub fn wait(&self) -> Result<(PreparedModel, T), ViewError> {
        match self.worker.wait(std::time::Duration::from_secs(120)) {
            Polled::Message(r) => r,
            Polled::Empty | Polled::Lost => Err(ViewError::LoadStopped),
        }
    }
}

/// スキンを別のスレッドで組み（`work` が読む・並べる。取消の旗を区切りで見る）、休みの形まで作る（3D ビューには入れない。入れるのは
/// `install_prepared`）。`work` はスキン・読み込みの知らせ・テイクを持つ FBX（スキンのどこに入ったか）・呼び手の残りの結果を返す。
/// スレッドは `view3d` に登録する（終わるときに止まるのを待つ）。
pub fn prepare_rig_with<T: Send + 'static>(
    view3d: &mut View3dState,
    work: impl FnOnce(&AtomicBool) -> Result<(Rig, Vec<String>, Vec<takes::TakeSource>, T), ViewError>
        + Send
        + 'static,
) -> RigJob<T> {
    let revision = view3d.next_revision();
    let cancel = Arc::new(AtomicBool::new(false));
    let finished = view3d.pose.loads.register(&cancel);
    let worker = Worker::spawn_on(
        std::thread::Builder::new().name("yolu-livelink-rig".into()),
        cancel,
        move |tx, flag| {
            let _finished = finished;
            let flag = flag.flag();
            let result = work(flag).and_then(|(rig, warnings, takes, rest)| {
                if flag.load(Ordering::Relaxed) {
                    return Err(ViewError::Cancelled);
                }
                let (meshes, geometry) = build_rest(&rig, revision, Some(flag))?;
                Ok((
                    PreparedModel(Loaded {
                        rig,
                        rest: geometry,
                        meshes,
                        warnings,
                        takes,
                    }),
                    rest,
                ))
            });
            let _ = tx.send(result);
        },
    )
    .expect("failed to spawn thread")
    .cancel_on_drop();
    RigJob { worker }
}

/// FBX を別のスレッドで読み始める（3D ビューには入れない）。`view3d` は世代の番号を取るためだけに借りる。
pub fn prepare_fbx(view3d: &mut View3dState, path: &Path, limits: ModelLimits) -> PrepareJob {
    let revision = view3d.next_revision();
    let (worker, progress) = spawn_load(view3d, path, limits, revision, PreparedModel);
    PrepareJob { worker, progress }
}

/// 終わらない読み込みの送り口（`park_loading` が返す。持っているあいだは読み込み中のまま）。
#[doc(hidden)]
pub struct ParkedLoad {
    _tx: std::sync::mpsc::Sender<Result<Loaded, ViewError>>,
}

/// 終わらない読み込みを置く（試験用: ポーズの欄の「読み込み中」の行と取り消しのボタンを確かめる）。`fraction` は進み具合。
/// 取り消すか、返す物を捨てると読み込み中でなくなる。
#[doc(hidden)]
pub fn park_loading(view3d: &mut View3dState, name: &str, fraction: Option<f32>) -> ParkedLoad {
    let (worker, tx) = Worker::parked();
    let progress = Arc::new(LoadProgress::default());
    if let Some(f) = fraction {
        progress.set(f);
    }
    let revision = view3d.next_revision();
    view3d.pose.loading = Some(Loading {
        name: name.to_owned(),
        worker: worker.cancel_on_drop(),
        progress,
        revision,
    });
    ParkedLoad { _tx: tx }
}

/// 読み終えたモデルを 3D ビューに入れる（ポーズのセッションも始まる。描いている最中なら、形はストロークが終わってから入れ替わる）。
pub fn install_prepared(view3d: &mut View3dState, prepared: PreparedModel) {
    install(view3d, prepared.0);
}

/// 次のフレームで 3D ビューのタブを前に出す。
pub fn request_focus(view3d: &mut View3dState) {
    view3d.pose.focus = true;
}

/// FBX を別のスレッドで読み始める（読んでいる途中のものは取り消す）。終わったら `poll` が入れる。
pub fn open_fbx(view3d: &mut View3dState, path: &Path) {
    open_fbx_with(view3d, path, ModelLimits::default());
}

pub fn open_fbx_with(view3d: &mut View3dState, path: &Path, limits: ModelLimits) {
    // 前の読み込みは取り消す（新しい方だけが入る。前の受け口を捨てると、スレッドは次の区切りで止まる）
    view3d.pose.loading = None;
    let revision = view3d.next_revision();
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (worker, progress) = spawn_load(view3d, path, limits, revision, |loaded| loaded);
    view3d.pose.loading = Some(Loading {
        name,
        worker,
        progress,
        revision,
    });
}

/// 「「名前」を読み込めません（理由）。」（ポーズのモデルの読み込みの失敗）。
fn cannot_load(lang: Lang, name: &str, error: &ViewError) -> String {
    let name = lang.quote(name);
    lang.with_reason(
        lang.pick(
            format!("{name}を読み込めません"),
            format!("Cannot load {name}"),
        ),
        lang.view_error(error),
    )
}

#[cfg(test)]
pub(crate) fn wait_for_load(view3d: &mut View3dState) -> (Option<String>, bool) {
    if let Some(loading) = &mut view3d.pose.loading {
        let Polled::Message(result) = loading.worker.wait(std::time::Duration::from_secs(120))
        else {
            panic!("FBXの完了通知が来ない（受信切断またはハング検出上限）");
        };
        let (worker, tx) = Worker::parked();
        tx.send(result).unwrap();
        loading.worker = worker.cancel_on_drop();
    }
    poll(view3d)
}

/// `poll_in`の、日本語のもの（試験・言語を持たない呼び出し）。知らせの種類は捨てる。
pub fn poll(view3d: &mut View3dState) -> (Option<String>, bool) {
    let (message, installed) = poll_in(view3d, Lang::Ja);
    (message.map(|(_, text)| text), installed)
}

/// フレームの初めに: 読み終わった FBX を入れ、ほかのモデルに替わったセッションを終える。知らせる文（言語に合わせる）とその種類
/// （読めた: 済んだ知らせ、読めなかった所がある: 注意、読めない・止まった: 失敗、取り消した: 済んだ知らせ）と、モデルを入れたかを返す。
pub fn poll_in(view3d: &mut View3dState, lang: Lang) -> (Option<(Kind, String)>, bool) {
    let mut message = None;
    let mut installed = false;
    if let Some(loading) = &view3d.pose.loading {
        match loading.worker.poll() {
            Polled::Message(Ok(loaded)) => {
                let name = loading.name.clone();
                let stale = loaded.rest.revision() != loading.revision;
                view3d.pose.loading = None;
                if !stale {
                    let warnings = loaded.warnings.len();
                    install(view3d, loaded);
                    installed = true;
                    // 名前と、読めなかった所があるか（件数は状態）。三角形・骨の数や読んだ時間は出さない
                    message = Some(if warnings > 0 {
                        (
                            Kind::Warning,
                            lang.pick(
                                format!("{name}を読み込みました（知らせ {warnings} 件）"),
                                format!("Loaded {name} ({warnings} notices)"),
                            ),
                        )
                    } else {
                        (
                            Kind::Info,
                            lang.pick(format!("{name}を読み込みました"), format!("Loaded {name}")),
                        )
                    });
                }
            }
            Polled::Message(Err(e)) => {
                let name = lang.quote(&loading.name);
                message = Some(if matches!(e, ViewError::Cancelled) {
                    (
                        Kind::Info,
                        lang.pick(
                            format!("{name}の読み込みを取り消しました。"),
                            format!("Loading {name} was canceled."),
                        ),
                    )
                } else {
                    (Kind::Error, cannot_load(lang, &loading.name, &e))
                });
                view3d.pose.loading = None;
            }
            Polled::Empty => {}
            Polled::Lost => {
                message = Some((
                    Kind::Error,
                    cannot_load(lang, &loading.name, &ViewError::LoadStopped),
                ));
                view3d.pose.loading = None;
            }
        }
    }
    // ほかのモデル（試しの立方体・Live Link）に替わったら、このセッションは終わる
    let latest = view3d.latest_model().map(|m| m.revision());
    if let Some(s) = &view3d.pose.session {
        if latest != Some(s.model_revision) {
            view3d.pose.session = None;
            view3d.pose.drag = None;
            view3d.set_face_mask(None);
        }
    }
    (message, installed)
}

/// ポーズを当てる（スキニング → 三角形の写し → 休みの形の refit → モデルを入れ替える）。
fn apply(view3d: &mut View3dState, pose: Pose) -> Result<(), ViewError> {
    let revision = view3d.next_revision();
    let s = view3d.pose.session.as_mut().ok_or(ViewError::NoPoseModel)?;
    let clock = Instant::now();
    let meshes = s.rig.deform(&pose)?;
    let skin_ms = clock.elapsed().as_secs_f64() * 1000.0;
    let clock2 = Instant::now();
    let triangles = model_triangles(&meshes).ok_or(ViewError::BadMeshIndex)?;
    let geometry = s.rest.reposition(triangles, revision, BvhUpdate::Refit)?;
    let refit_ms = clock2.elapsed().as_secs_f64() * 1000.0;
    let model = ViewModel::with_geometry(
        s.rig.name(),
        meshes,
        s.rig.materials().iter().cloned().map(Some).collect(),
        Arc::new(geometry),
    )
    .with_rest(s.rest.clone());
    s.pose = pose;
    s.edits += 1;
    s.model_revision = revision;
    s.timings = PoseTimings {
        skin_ms,
        refit_ms,
        total_ms: clock.elapsed().as_secs_f64() * 1000.0,
    };
    view3d.set_model(model);
    Ok(())
}

fn push_undo(s: &mut PoseSession, before: Pose) {
    s.undo.push(before);
    if s.undo.len() > MAX_POSE_HISTORY {
        s.undo.remove(0);
    }
    s.redo.clear();
}

/// 1 回で終わる変更（取り消しに 1 つ積む）。
pub fn set_pose(view3d: &mut View3dState, pose: Pose) -> Result<(), ViewError> {
    if view3d.input.stroke.is_some() {
        return Err(ViewError::Stroking);
    }
    let s = view3d.pose.session.as_mut().ok_or(ViewError::NoPoseModel)?;
    s.rig.check_pose(&pose)?;
    if s.pose == pose {
        return Ok(());
    }
    let before = s.pose.clone();
    apply(view3d, pose)?;
    if let Some(s) = view3d.pose.session.as_mut() {
        push_undo(s, before);
    }
    Ok(())
}

/// 開いた .ylp のポーズを戻す（取り消しの段にも「変更あり」の印にも数えない: 開いた直後の状態）。
pub fn restore_pose(view3d: &mut View3dState, pose: Pose) -> Result<(), ViewError> {
    let s = view3d.pose.session.as_ref().ok_or(ViewError::NoPoseModel)?;
    s.rig.check_pose(&pose)?;
    apply(view3d, pose)?;
    if let Some(s) = view3d.pose.session.as_mut() {
        s.undo.clear();
        s.redo.clear();
        s.edits = 0;
    }
    view3d.pose.seen_edits = 0;
    Ok(())
}

/// 続けて変える操作を始める（ギズモ・スライダーのドラッグ）。
pub fn begin_edit(view3d: &mut View3dState) -> Result<(), ViewError> {
    if view3d.input.stroke.is_some() {
        return Err(ViewError::Stroking);
    }
    let s = view3d.pose.session.as_mut().ok_or(ViewError::NoPoseModel)?;
    if s.edit_start.is_none() {
        s.edit_start = Some(s.pose.clone());
    }
    Ok(())
}

/// 続けて変えている途中のポーズ（取り消しには積まない）。
pub fn edit(view3d: &mut View3dState, pose: Pose) -> Result<(), ViewError> {
    let s = view3d.pose.session.as_ref().ok_or(ViewError::NoPoseModel)?;
    if s.edit_start.is_none() {
        return Err(ViewError::NoPoseEdit);
    }
    s.rig.check_pose(&pose)?;
    if s.pose == pose {
        return Ok(());
    }
    apply(view3d, pose)
}

/// 続けて変える操作を終える（commit なら変わっていれば取り消しに 1 つ積む。でなければ始まりへ戻す）。
pub fn end_edit(view3d: &mut View3dState, commit: bool) {
    let Some(s) = view3d.pose.session.as_mut() else {
        return;
    };
    let Some(start) = s.edit_start.take() else {
        return;
    };
    if commit {
        if s.pose != start {
            push_undo(s, start);
        }
    } else if s.pose != start {
        let _ = apply(view3d, start);
    }
}

/// 取り消し（変えていなければ false）。
pub fn undo(view3d: &mut View3dState) -> Result<bool, ViewError> {
    step(view3d, true)
}

pub fn redo(view3d: &mut View3dState) -> Result<bool, ViewError> {
    step(view3d, false)
}

fn step(view3d: &mut View3dState, back: bool) -> Result<bool, ViewError> {
    if view3d.input.stroke.is_some() {
        return Err(ViewError::Stroking);
    }
    let Some(s) = view3d.pose.session.as_mut() else {
        return Ok(false);
    };
    if s.edit_start.is_some() {
        return Ok(false);
    }
    let target = if back { s.undo.pop() } else { s.redo.pop() };
    let Some(target) = target else {
        return Ok(false);
    };
    let current = s.pose.clone();
    if let Err(e) = apply(view3d, target.clone()) {
        // 当てられなければ並びを元へ
        if let Some(s) = view3d.pose.session.as_mut() {
            if back {
                s.undo.push(target);
            } else {
                s.redo.push(target);
            }
        }
        return Err(e);
    }
    if let Some(s) = view3d.pose.session.as_mut() {
        if back {
            s.redo.push(current);
        } else {
            s.undo.push(current);
        }
    }
    Ok(true)
}

/// ファイルにあったときのポーズへ（取り消せる）。
pub fn reset(view3d: &mut View3dState) -> Result<(), ViewError> {
    let rest = match &view3d.pose.session {
        Some(s) => s.rig.rest_pose(),
        None => return Ok(()),
    };
    set_pose(view3d, rest)
}

/// 決まったパスの FBX を開く。何も触っていないプロジェクトなら、その FBX で新規プロジェクトを作るウィンドウ、作業のあるプロジェクトなら、
/// プロジェクトの構成のウィンドウでそのモデルに替える下書き（どのマテリアルをテクスチャセットにするか・大きさ・照合を、決めてから入れる）。
/// 別のスレッドで読む。ファイルを選ぶウィンドウは `YoluApp` が開く（`DialogRequest::OpenModel`。ウィンドウに落としたファイルもここへ来る）ので、
/// ここは OS のウィンドウに頼らない。描いている最中は断る。
pub fn open_file(app: &mut AppState, path: &Path) {
    app.np_apply(crate::newproject::NpAction::OpenModel(path.to_path_buf()));
}

/// 操作を当てる（`Action::Pose`）。描いている最中は、モードを切ることのほかは断る（読み込みの取り消しは、描いていても通す）。
pub fn apply_action(app: &mut AppState, action: PoseAction) {
    if action == PoseAction::CancelLoad {
        // 読み込みの結果は 3D ビューに入れていない。取り消すと、途中の物を捨てて今のモデルは前のまま
        if let Some(name) = app.view3d.pose.loading_name().map(str::to_owned) {
            app.view3d.pose.cancel_loading();
            app.info(
                Source::Pose,
                app.lang.pick(
                    format!("{name}の読み込みを取り消しました。"),
                    format!("Cancelled loading {name}."),
                ),
            );
        }
        return;
    }
    if app.is_stroking() {
        app.refuse(Source::Pose, app.lang.view_error(&ViewError::Stroking));
        return;
    }
    let result = match action {
        PoseAction::OpenFbx => {
            // ウィンドウは YoluApp が開く（選ばれたら `open_file`）
            app.dialog_request = Some(DialogRequest::OpenModel);
            Ok(())
        }
        PoseAction::CancelLoad => Ok(()),
        PoseAction::LoadFigure => load_figure(&mut app.view3d).map(|()| {
            app.view3d.pose.focus = true;
            // 試しの人形はファイルのモデルではない（プロジェクトのモデルの参照は外す）
            app.np.model_file = None;
            let note = app.bind_rig_model();
            if let Some(s) = &app.view3d.pose.session {
                let mut text = app.lang.pick(
                    format!("{}を読み込みました", s.rig.name()),
                    format!("Loaded {}", s.rig.name()),
                );
                let mut kind = Kind::Info;
                if let Some((note_kind, note)) = note {
                    text += &format!(" {note}");
                    kind = note_kind;
                }
                app.notify(kind, Source::Pose, text);
            }
        }),
        PoseAction::ToggleMode => {
            // ポーズのモードとペイントのモードを行き来する（ギズモのドラッグの途中なら、そこまでを確定してから。`set_mode`）
            if app.view3d.pose.session.is_some() {
                app.set_mode(if app.mode == EditorMode::Pose {
                    EditorMode::Paint
                } else {
                    EditorMode::Pose
                });
            }
            Ok(())
        }
        PoseAction::Reset => reset(&mut app.view3d),
        PoseAction::Undo => undo(&mut app.view3d).map(|done| {
            if done {
                app.info(
                    Source::Pose,
                    app.lang.pick("ポーズを取り消しました。", "Pose undone."),
                );
            }
        }),
        PoseAction::Redo => redo(&mut app.view3d).map(|done| {
            if done {
                app.info(
                    Source::Pose,
                    app.lang.pick("ポーズをやり直しました。", "Pose redone."),
                );
            }
        }),
        PoseAction::ChooseTake(source, take) => {
            takes::choose(app, source, take);
            Ok(())
        }
        PoseAction::ApplyTake => takes::start(&mut app.view3d),
    };
    if let Err(e) = result {
        let text = app.lang.view_error(&e);
        app.notify(e.notice_kind(), Source::Pose, text);
    }
}

/// ポーズのモードで、3D ビューが見えているときだけ、取り消し・やり直しをポーズへ回す（キャンバスのタブへ移って 2D を描いている
/// ときに、見えていないポーズが戻らないように。モードはそのまま残り、3D ビューへ戻れば続きから）。
pub fn owns_undo(app: &AppState) -> bool {
    app.mode == EditorMode::Pose && app.view3d.pose.session.is_some() && app.view3d.visible
}

/// ポーズを変えていたら「変更あり」の印を付ける（ポーズは .ylp に残る。プロジェクトのモデル・Live Link の相手の無い試しの人形のポーズは
/// 残らないので数えない）。開いたときに戻したポーズ・モデルを入れた直後は変えたことにならない（`restore_pose`・`install`）。
pub fn sync_modified(app: &mut AppState) {
    let Some(edits) = app.view3d.pose.session.as_ref().map(|s| s.edits) else {
        return;
    };
    if edits != app.view3d.pose.seen_edits {
        app.view3d.pose.seen_edits = edits;
        if app.np.model_file.is_some() || app.link_target.is_some() {
            app.modified = true;
        }
    }
}

/// フレームの初めに（app から）: 読み込みを見て、知らせを出す。3D ビューのタブを前に出すなら true。
pub fn frame(app: &mut AppState, ctx: &egui::Context) -> bool {
    // ウィンドウに落とした FBX を開く
    let dropped = ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("fbx")))
    });
    if let Some(path) = dropped {
        open_file(app, &path);
    }
    // 続けて変える操作（欄の数値・スライダーのドラッグ）が、ボタンの離れを受け取れないまま残らないように。フォーカスを失ったら、
    // そこまでを確定する（離したのを受け取れないので。ギズモのドラッグは 3D ビューの入力が同じ決まりで終える）
    let (down, focus_lost) = ctx.input(|i| {
        (
            i.pointer.any_down(),
            i.raw
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::WindowFocused(false))),
        )
    });
    edit::finish_live_edit(app, down && !focus_lost);
    let (message, installed) = poll_in(&mut app.view3d, app.lang);
    // ほかのモデルに替わってセッションが終わったら、ポーズのモードを出る
    crate::mode::leave_pose_without_rig(app);
    takes::poll(app);
    sync_modified(app);
    // 別のモデルに替わっていたら記録を外し、FBX を入れたらマテリアルごとにセットを結び付ける
    app.sync_rig_model();
    let note = if installed {
        app.bind_rig_model()
    } else {
        None
    };
    if let Some((mut kind, mut m)) = message {
        if let Some((note_kind, note)) = note {
            m += &format!(" {note}");
            // 結び付けの但し書き（モデルに無いセット・上限）は、済んだ知らせを注意にする
            if kind == Kind::Info {
                kind = note_kind;
            }
        }
        app.notify(kind, Source::Pose, m);
    }
    let taking = app
        .view3d
        .pose
        .session
        .as_ref()
        .is_some_and(|s| s.takes.is_running());
    if app.view3d.pose.is_loading() || taking {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    let focus = installed || app.view3d.pose.focus;
    app.view3d.pose.focus = false;
    focus
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Action, AppState};
    use yolu_core::glam::{Quat, Vec3};

    fn app() -> AppState {
        let mut app = AppState::new(64, 64);
        app.apply(Action::Pose(PoseAction::LoadFigure));
        assert!(app.view3d.pose.session.is_some(), "{}", app.message);
        app
    }

    fn bone(app: &AppState, name: &str) -> usize {
        let s = app.view3d.pose.session.as_ref().unwrap();
        s.rig.bones().iter().position(|b| b.name == name).unwrap()
    }

    fn bent(app: &AppState, angle: f32) -> Pose {
        let s = app.view3d.pose.session.as_ref().unwrap();
        let mut p = s.pose().clone();
        p.locals[bone(app, "右上腕")].rotation = Quat::from_rotation_z(angle);
        p
    }

    fn positions(app: &AppState) -> Vec<Vec3> {
        let m = app.view3d.model.as_ref().unwrap();
        m.geometry
            .triangles()
            .iter()
            .flat_map(|t| [t.a, t.b, t.c])
            .collect()
    }

    #[test]
    fn posing_refits_the_snapshot_and_undo_redo_restore_it() {
        let mut app = app();
        let rest_positions = positions(&app);
        let rest = app.view3d.model.clone().unwrap();
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert_eq!(s.rest_geometry().revision(), rest.revision());
        assert!(!s.is_posed());
        {
            let p = bent(&app, -1.2);
            set_pose(&mut app.view3d, p)
        }
        .unwrap();
        let posed = app.view3d.model.clone().unwrap();
        assert!(posed.revision() > rest.revision(), "新しい世代");
        assert_eq!(posed.triangle_count(), rest.triangle_count());
        assert_ne!(positions(&app), rest_positions);
        assert_eq!(
            posed.geometry.brush_scale(),
            rest.geometry.brush_scale(),
            "ブラシの大きさは変わらない"
        );
        assert_eq!(posed.name, rest.name, "カメラはそのまま（同じ名前・数）");
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert!(s.is_posed() && s.can_undo() && !s.can_redo());
        assert!(s.timings.total_ms > 0.0);
        assert!(undo(&mut app.view3d).unwrap());
        assert_eq!(positions(&app), rest_positions, "取り消すと休みの形");
        assert!(redo(&mut app.view3d).unwrap());
        assert_ne!(positions(&app), rest_positions);
        assert!(!redo(&mut app.view3d).unwrap(), "やり直すものは無い");
        // 戻すも取り消せる
        reset(&mut app.view3d).unwrap();
        assert_eq!(positions(&app), rest_positions);
        assert_eq!(app.view3d.pose.session.as_ref().unwrap().undo_len(), 2);
        undo(&mut app.view3d).unwrap();
        assert_ne!(positions(&app), rest_positions);
    }

    #[test]
    fn a_drag_is_one_undo_step_and_cancel_returns_to_the_start() {
        let mut app = app();
        begin_edit(&mut app.view3d).unwrap();
        for a in [-0.2, -0.5, -0.9] {
            {
                let p = bent(&app, a);
                edit(&mut app.view3d, p)
            }
            .unwrap();
        }
        end_edit(&mut app.view3d, true);
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert_eq!(s.undo_len(), 1, "ドラッグ 1 回で 1 つ");
        let after = s.pose().clone();
        begin_edit(&mut app.view3d).unwrap();
        {
            let p = bent(&app, 0.7);
            edit(&mut app.view3d, p)
        }
        .unwrap();
        end_edit(&mut app.view3d, false);
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert_eq!(s.pose(), &after, "やめたら始まりのポーズ");
        assert_eq!(s.undo_len(), 1);
        // 変えずに終えたドラッグは積まない
        begin_edit(&mut app.view3d).unwrap();
        end_edit(&mut app.view3d, true);
        assert_eq!(app.view3d.pose.session.as_ref().unwrap().undo_len(), 1);
        // 始めずに edit は断る・形の違うポーズは断る
        assert!({
            let p = bent(&app, 0.3);
            edit(&mut app.view3d, p)
        }
        .is_err());
        let mut wrong = bent(&app, 0.3);
        wrong.locals.pop();
        assert!(set_pose(&mut app.view3d, wrong).is_err());
    }

    #[test]
    fn history_is_capped() {
        let mut app = app();
        for i in 0..MAX_POSE_HISTORY + 10 {
            {
                let p = bent(&app, i as f32 * 0.001 + 0.01);
                set_pose(&mut app.view3d, p)
            }
            .unwrap();
        }
        assert_eq!(
            app.view3d.pose.session.as_ref().unwrap().undo_len(),
            MAX_POSE_HISTORY
        );
    }

    #[test]
    fn pose_changes_are_refused_while_stroking() {
        let mut app = app();
        let layer = app.selected_layer.unwrap();
        let settings = app.stroke_settings(false);
        let stroke = app.doc.begin_stroke(layer, &settings).unwrap();
        app.stroke = Some(stroke);
        app.view3d.input.stroke = Some(crate::state::StrokeSource::Mouse);
        let before = app.view3d.model.clone().unwrap().revision();
        assert!({
            let p = bent(&app, -1.0);
            set_pose(&mut app.view3d, p)
        }
        .is_err());
        assert!(begin_edit(&mut app.view3d).is_err());
        app.apply(Action::Pose(PoseAction::Reset));
        app.apply(Action::Pose(PoseAction::ToggleMode));
        assert_ne!(app.mode, EditorMode::Pose, "描いている間はモードも変えない");
        assert_eq!(app.message, ViewError::Stroking.to_string());
        app.apply(Action::Pose(PoseAction::OpenFbx));
        assert_eq!(
            app.dialog_request, None,
            "描いている間はファイルのウィンドウを頼まない"
        );
        open_file(&mut app, Path::new("読まれない.fbx"));
        assert!(!app.view3d.pose.is_loading(), "描いている間は読み始めない");
        assert_eq!(app.view3d.model.clone().unwrap().revision(), before);
        let stroke = app.stroke.take().unwrap();
        app.doc.cancel_stroke(stroke);
        app.view3d.stroke_ended();
        assert!({
            let p = bent(&app, -1.0);
            set_pose(&mut app.view3d, p)
        }
        .is_ok());
    }

    #[test]
    fn undo_goes_to_the_pose_only_in_pose_mode() {
        let mut app = app();
        // 画素を 1 つ変える（文書の取り消しに 1 つ）
        let layer = app.selected_layer.unwrap();
        let settings = app.stroke_settings(false);
        let mut stroke = app.doc.begin_stroke(layer, &settings).unwrap();
        stroke
            .add_point(&mut app.doc, 10.0, 10.0, 1.0, yolu_core::glam::DVec2::ZERO)
            .unwrap();
        app.doc.end_stroke(stroke).unwrap();
        {
            let p = bent(&app, -1.0);
            set_pose(&mut app.view3d, p)
        }
        .unwrap();
        assert!(app.doc.can_undo());
        // ポーズのモード: Ctrl+Z はポーズ（3D ビューが見えているあいだ）
        app.apply(Action::Pose(PoseAction::ToggleMode));
        assert_eq!(app.mode, EditorMode::Pose);
        app.view3d.visible = false;
        assert!(
            !owns_undo(&app),
            "3D ビューが別のタブの裏なら、ポーズへは回さない"
        );
        app.view3d.visible = true;
        assert!(app.can_undo());
        app.apply(Action::Undo);
        assert!(!app.view3d.pose.session.as_ref().unwrap().is_posed());
        assert!(app.doc.can_undo(), "画素はそのまま");
        assert!(!app.can_undo(), "ポーズの取り消しはもう無い");
        app.apply(Action::Redo);
        assert!(app.view3d.pose.session.as_ref().unwrap().is_posed());
        // モードを切ると画素
        app.apply(Action::Pose(PoseAction::ToggleMode));
        app.apply(Action::Undo);
        assert!(!app.doc.can_undo());
        assert!(
            app.view3d.pose.session.as_ref().unwrap().is_posed(),
            "ポーズはそのまま"
        );
    }

    #[test]
    fn another_model_ends_the_session() {
        let mut app = app();
        app.mode = EditorMode::Pose;
        assert!(!poll(&mut app.view3d).1);
        assert!(app.view3d.pose.session.is_some(), "自分のモデルなら続く");
        {
            let p = bent(&app, -1.0);
            set_pose(&mut app.view3d, p)
        }
        .unwrap();
        poll(&mut app.view3d);
        assert!(app.view3d.pose.session.is_some(), "ポーズを付けても続く");
        app.apply(Action::LoadDemoModel);
        poll(&mut app.view3d);
        assert!(app.view3d.pose.session.is_none());
        // フレームの初め（`frame`）に、セッションの無いポーズのモードを出る
        crate::mode::leave_pose_without_rig(&mut app);
        assert_eq!(app.mode, EditorMode::Paint);
    }

    /// 最小の ASCII の FBX: 三角形 1 つ（UV つき）、骨なし。
    pub(super) const TRIANGLE_FBX: &str = "; FBX 7.4.0 project file\nFBXHeaderExtension:  {\n\tFBXVersion: 7400\n}\nGlobalSettings:  {\n\tVersion: 1000\n\tProperties70:  {\n\t\tP: \"UnitScaleFactor\", \"double\", \"Number\", \"\",100\n\t}\n}\nObjects:  {\n\tModel: 100, \"Model::Tri\", \"Mesh\" {\n\t\tVersion: 232\n\t}\n\tGeometry: 200, \"Geometry::Tri\", \"Mesh\" {\n\t\tVertices: *9 {\n\t\t\ta: 0,0,0,1,0,0,0,1,0\n\t\t}\n\t\tPolygonVertexIndex: *3 {\n\t\t\ta: 0,1,-3\n\t\t}\n\t\tLayerElementUV: 0 {\n\t\t\tMappingInformationType: \"ByPolygonVertex\"\n\t\t\tReferenceInformationType: \"Direct\"\n\t\t\tUV: *6 {\n\t\t\t\ta: 0,0,1,0,0,1\n\t\t\t}\n\t\t}\n\t\tLayer: 0 {\n\t\t\tLayerElement:  {\n\t\t\t\tType: \"LayerElementUV\"\n\t\t\t\tTypedIndex: 0\n\t\t\t}\n\t\t}\n\t}\n}\nConnections:  {\n\tC: \"OO\",100,0\n\tC: \"OO\",200,100\n}\n";

    fn wait(app: &mut AppState) -> (Option<String>, bool) {
        wait_for_load(&mut app.view3d)
    }

    #[test]
    fn fbx_loads_in_the_background_and_bad_files_report_a_reason() {
        let dir = std::env::temp_dir().join(format!("yolu-app-pose-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 最小の ASCII の FBX: 三角形 1 つ（UV つき）、骨なし
        let text = TRIANGLE_FBX;
        let good = dir.join("三角.fbx");
        std::fs::write(&good, text).unwrap();
        let bad = dir.join("壊れた.fbx");
        std::fs::write(&bad, &text.as_bytes()[..60]).unwrap();
        let mut app = AppState::new(64, 64);
        open_fbx(&mut app.view3d, &good);
        assert!(app.view3d.pose.is_loading());
        assert_eq!(app.view3d.pose.loading_name(), Some("三角.fbx"));
        let (message, installed) = wait(&mut app);
        assert!(installed, "{message:?}");
        // 名前を知らせる（三角形・骨の数や時間は出さない）
        let message = message.unwrap();
        assert!(
            message.contains("三角.fbx") && !message.contains("三角形"),
            "{message}"
        );
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert_eq!(s.rig.name(), "三角");
        assert_eq!(app.view3d.model.as_ref().unwrap().triangle_count(), 1);
        // 壊れたファイル: 理由を知らせ、今のモデルは残す
        open_fbx(&mut app.view3d, &bad);
        let (message, installed) = wait(&mut app);
        assert!(!installed);
        assert!(message
            .unwrap()
            .starts_with("「壊れた.fbx」を読み込めません（"));
        assert!(app.view3d.pose.session.is_some());
        // 大きすぎる
        let limits = ModelLimits {
            max_file_bytes: 16,
            ..ModelLimits::default()
        };
        open_fbx_with(&mut app.view3d, &good, limits);
        let (message, _) = wait(&mut app);
        assert!(message.unwrap().contains("大きすぎます"));
        // 続けて開くと、前の読み込みは捨てる（新しいほうだけ入る）
        open_fbx(&mut app.view3d, &bad);
        open_fbx(&mut app.view3d, &good);
        let (_, installed) = wait(&mut app);
        assert!(installed);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn open_file_goes_through_the_new_project_window_and_the_menu_only_asks_for_the_dialog() {
        let dir = std::env::temp_dir().join(format!("yolu-app-pose-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("三角.fbx");
        std::fs::write(&path, TRIANGLE_FBX).unwrap();
        let mut app = AppState::new(64, 64);
        // 状態のレイヤーは OS のウィンドウを開かない: 頼みを残すだけ
        app.apply(Action::Pose(PoseAction::OpenFbx));
        assert_eq!(app.dialog_request, Some(DialogRequest::OpenModel));
        assert!(!app.view3d.pose.is_loading());
        // 選ばれたパス（ウィンドウに落としたものも）は、何も触っていないプロジェクトなら新規プロジェクトのウィンドウで読む（3D ビューには、
        // 決めるまで入れない）
        open_file(&mut app, &path);
        let win = app
            .np
            .window
            .as_ref()
            .expect("新規プロジェクトのウィンドウ");
        assert!(!win.configure && win.is_loading());
        assert!(!app.view3d.pose.is_loading() && app.view3d.pose.session.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
