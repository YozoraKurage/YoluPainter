//! メッシュマップのベイク（Unity 版の `TexturePaintWindow.MeshBake` と同じ決まり）。3D ビューのモデルから、テクスチャセットの
//! マテリアルを使うスロットと文書の大きさで、法線・位置・AO・曲率・厚みなどを焼き、セットのメッシュマップ（`MeshMapSet`）に入れる。
//!
//! - **別のスレッドで焼く**: 始めるときに入力（モデルの写し）・設定・スロット・文書の ID と大きさを固定し、焼くのは `yolu_gpu::bake_mesh_maps`
//!   （ウィンドウで選んだ「自動・GPU・CPU」。自動は使えるハードウェアの GPU があれば GPU、なければ CPU。GPU が使えない・壊れた・予算を超えたときは
//!   理由つきで CPU の `yolu_core::mesh_maps::bake`（rayon で並列）に戻る。進み具合は共有の値、取消は `AtomicBool`）。画面は毎フレーム
//!   `poll_bake` で終わりを見る。使った場所と戻った理由はセットの記録（`MeshMapSet::run`）と結果の一文に出る。
//! - **結果を使う前に確かめる**: 終わったとき、セット・文書（ID と大きさ）・スロット・モデルの形（ポーズを含む）が始めたときと同じなら
//!   マップを入れ、違えば捨てる。取消・時間切れ・拒否・捨てた結果では、前のマップは変えない。残りのセットも焼かない。
//! - **セットごとに焼く**: ウィンドウでチェックしたセットを並びの順に 1 つずつ。モデルに無いマテリアルのセットは焼かない。
//! - **古いマップを黙って使わない**: 由来（モデルの指紋・大きさ・スロット・設定・エンジンの版）が今と違うマップは `Stale`、モデルが無ければ
//!   `Unverified` と言い、書き出しの AO にも使わない（`occlusion_for_export`）。
//! - 描いている間は始めない。焼いている間の描画は許す（結果の意味を変えない）。ポーズやモデルが変われば結果を捨てる。
//! - **モデルの同一性は `Arc` の弱い参照で見る**（`ModelId`）。アドレスだけを覚えると、旧モデルが解放されたあとに別のモデルが同じ
//!   アドレスに置かれて同じと取り違える（ポーズを変えるたびにモデルは作り直される）。
//! - **入力（モデルの写しと指紋）を作る場所**: 始める・書き出すときは必要ならその場で作る（`bake_input`）。ウィンドウの状態表示は毎フレーム
//!   求めるので、モデルが替わったら別のスレッドで作り（`bake_input_nowait`）、できるまで状態は「確認中」にする。作りかけの入力は、ウィンドウを
//!   閉じていれば手放す（`release_idle_bake_input`）ので、ウィンドウを閉じているあいだは、モデルの差し替えもポーズの変更も追わない（前の入力のまま
//!   照合する）。例外は、.ylp を開いてモデルを読み直した直後の照合し直し 1 回（`ReopenCheck`）。
//!
//! 高ポリからの投影（参照）は、ウィンドウで選べるようになるまで使わない（参照なしで焼く）。

pub mod adopt;
pub mod input;
pub mod maps;
pub mod overlap;
pub mod overlay;
pub mod uvmap;
pub mod window;

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, Weak};

use yolu_gpu::{bake_mesh_maps, GpuBakeSlot, RayQueryWhy};
pub use yolu_gpu::{BakeAdapter, BakeBackend, BakeRun, FallbackKind, GpuBakeMethod};

pub use maps::MeshMapSet;
pub use overlay::{MeshMapView, Overlay};
use yolu_core::export::occlusion_byte;
use yolu_core::mesh_maps::{
    material_identity, MeshBakeBudget, MeshBakeInput, MeshBakeReport, MeshBakeResult,
    MeshBakeSettings, MeshBakeStatus, MeshIdSource, MeshMapCheck, MeshMapExpectation, MeshMapKind,
    MeshMapState,
};

use crate::jobs::{JobCard, JobSpec, Polled, Worker};
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::{Action, AppState};
use crate::view3d::model::ViewModel;
use crate::windows::CloseJob;

/// ベイクの操作（`Action::Bake`）。
#[derive(Clone, Debug, PartialEq)]
pub enum BakeAction {
    OpenWindow,
    CloseWindow,
    /// チェックしたマップを、チェックしたテクスチャセットごとに焼き始める。
    Start,
    /// 焼いているのを取り消す（止まったら前のマップはそのまま）。
    Cancel,
    /// 焼くマップのチェック。最後の 1 つは外せない。
    Map(MeshMapKind, bool),
    /// 焼くテクスチャセットのチェック（セットの uid）。最後の 1 つは外せない。
    Set(u32, bool),
    /// 2D のキャンバスに重ねて見るもの。
    View(MeshMapView),
    /// 焼く場所（自動・GPU・CPU）。次のベイクから効く。
    Backend(BakeBackend),
    /// 重なった UV のテクセルの持ち主の決め方（今のセットの文書。1 回の Undo）と、手でアイランドを選ぶ。
    Priority(overlap::PriorityOp),
}

/// 焼いている 1 回の仕事（1 つのテクスチャセット）。始めたときの条件を持つ。
struct Job {
    uid: u32,
    name: String,
    settings: MeshBakeSettings,
    progress: Arc<Mutex<(f64, String)>>,
    worker: Worker<Result<(MeshBakeResult, BakeRun), String>>,
    doc_id: u128,
    size: (u32, u32),
    slots: Vec<i32>,
    /// 始めたときのモデル。
    model: ModelId,
}

/// GPU が使えるかの確認（ウィンドウの状態表示のために、別のスレッドで 1 回だけ作ってみる）。
#[derive(Clone, Debug, Default)]
pub enum GpuProbe {
    /// まだ確かめていない。
    #[default]
    Unknown,
    Probing {
        allow_software: bool,
    },
    Done {
        /// ソフトウェアの描画を許して確かめたか（「GPU」を選んだとき）。
        allow_software: bool,
        result: Result<BakeAdapter, String>,
    },
}

/// モデル（`Arc<ViewModel>`）の同一性の控え。弱い参照で持つので、旧モデルが解放されても割り当ては残り、別のモデルが同じ
/// アドレスに置かれて「同じ」と見なされることがない（生のアドレスを覚えて比べると、解放のあとの再利用を同じモデルと取り違える。
/// ポーズを変えるたびにモデルは作り直され、1 つ前の割り当てがすぐ次のモデルに使われやすい）。持つのは割り当ての枠だけで、
/// モデルの中身は握らない。
#[derive(Clone)]
struct ModelId(Weak<ViewModel>);

impl ModelId {
    fn of(model: &Arc<ViewModel>) -> ModelId {
        ModelId(Arc::downgrade(model))
    }

    fn is(&self, model: &Arc<ViewModel>) -> bool {
        Weak::ptr_eq(&self.0, &Arc::downgrade(model))
    }
}

/// .ylp を開いてモデルを読み直した直後の、焼いたマップの照合し直しを待つ印。開いた瞬間の 3D ビューは試しの立方体で、焼いたマップは立方体と
/// 照合して「古い」になる。読み終えたモデルの入力（指紋）ができて効果の入力へ渡るまで、作りかけの入力を手放さない
/// （ウィンドウが閉じていると、毎フレーム作り直しては手放し、いつまでもできない）。済んだら手放し、あとは追わない。
struct ReopenCheck {
    /// 印を付けたときのプロジェクトの世代（`np.generation`。別のプロジェクトにしたら無効）。
    generation: u64,
    /// 照合するモデル: 読み終えた時点のもの（ポーズを戻した後の形。描いている最中で入れ替わりを待っているなら、そのモデル）。その後ポーズの変更や
    /// モデルの差し替えで替わったら、追わずに手放す。
    model: ModelId,
}

/// 作った入力（モデルの形ごとに 1 回）。失敗も覚える（毎フレーム作り直さない）。
struct CachedInput {
    model: ModelId,
    input: Result<Arc<MeshBakeInput>, String>,
}

/// 別のスレッドで作っている入力（ウィンドウの状態表示のため。モデルが替わるたびに UI のスレッドで作り直さない）。
struct PendingInput {
    model: ModelId,
    rx: Receiver<Result<MeshBakeInput, String>>,
}

/// ベイクの画面の状態と走っている仕事。
#[derive(Default)]
pub struct BakeState {
    /// 焼く設定（プロジェクトで 1 つ。保存したマップの由来・古さの判定と同じ値。大きさ・スロットはセットごとに決まる）。
    pub settings: MeshBakeSettings,
    /// 焼かないテクスチャセット（uid。既定は全部焼く）。
    pub skipped: HashSet<u32>,
    pub window: Option<window::BakeWindow>,
    /// 2D のキャンバスに重ねて見るもの。
    pub view: MeshMapView,
    pub overlay: Overlay,
    /// 最後のベイクの結果の一文と、成功か（ウィンドウの下に出す）。
    pub outcome: Option<(String, bool)>,
    /// 今の並びでここまでに焼いたセットの UV の注意（`uv_warning`。同じ文は 1 つ）。複数のセットの最後のまとめの知らせに添える。
    uv_notes: Vec<String>,
    /// 焼く場所（既定は自動）。
    pub backend: BakeBackend,
    /// GPU のデバイスとシェーダー（焼くたびに作り直さない。別のスレッドから共有する）。
    gpu: Arc<GpuBakeSlot>,
    probe: Arc<Mutex<GpuProbe>>,
    /// GPU の確認の世代（確認に影響する設定が変わるたびに進む。古い世代の結果は書き戻さない）。
    probe_generation: Arc<AtomicU64>,
    job: Option<Job>,
    queue: VecDeque<u32>,
    total: usize,
    finished: usize,
    input: Option<CachedInput>,
    pending: Option<PendingInput>,
    /// .ylp を開いた直後の照合し直しを待っている（`AppState::expect_reopen_check`）。
    reopen_check: Option<ReopenCheck>,
    /// 別のスレッドで入力を作り始めた回数（`bake_input_nowait`）。試験が、作りかけを毎フレーム立てていないことを数える。
    input_builds: u64,
    /// 手でアイランドを選んでいる（次に 2D・3D で押したアイランドをこの一覧へ入れる）。
    pub pick: Option<overlap::Picking>,
    /// 手で選ぶアイランドの索引（モデルの入力ごと）。
    islands: Option<Arc<overlap::Islands>>,
    /// 別のスレッドで作っているアイランドの索引（入力ごとに 1 つ。UI のスレッドを止めない）。
    pending_islands: Option<overlap::PendingIslands>,
    /// ポリゴン塗りつぶしの右クリックで、アイランドの索引ができるのを待っているメニュー（できたフレームで開く）。
    pub(crate) menu_wait: Option<overlap::MenuWait>,
    /// ポリゴン塗りつぶしの右ボタンを押した所（3D か）。離した所が近ければアイランドのメニューを開く。
    pub(crate) menu_press: Option<(egui::Pos2, bool, std::time::Instant)>,
    /// ウィンドウの UV の見取り図か一覧の行でポインタを置いているアイランド（キャンバスと 3D ビューでも同じアイランドを強調する）。
    pub map_hover: Option<usize>,
    /// ウィンドウの UV の見取り図の中身（モデル・マテリアル・アイランドごと）。
    map: Option<Arc<uvmap::MapData>>,
    /// 試験用: 次の仕事を、取消が来るまで始めずに止めておく（始めるときに下ろす）。
    #[doc(hidden)]
    pub park_next: bool,
    /// 試験用: 次の仕事を、焼き始めて最初の確認（GPU なら 1 回目の dispatch のあと、CPU なら最初の行のあと）で、取消が来るまで
    /// 止めておく。取消が準備の中でなく、焼いている間に効くことを確かめる（始めるときに下ろす）。
    #[doc(hidden)]
    pub park_mid_bake: bool,
}

/// 進み具合（ウィンドウ・仕事の札が出す）。
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub fraction: f64,
    /// 段階の名前（core の "Preparing" "Baking" "Padding" "Done"）。
    pub phase: String,
    pub set: String,
    /// 何枚目のセットか（1 から）と、全部の数。
    pub index: usize,
    pub total: usize,
    pub canceling: bool,
}

impl BakeState {
    pub fn is_baking(&self) -> bool {
        self.job.is_some()
    }

    /// 焼く場所の選びに合わせた、GPU が使えるかの確認の今の状態。
    pub fn gpu_probe(&self) -> GpuProbe {
        self.probe.lock().map(|p| p.clone()).unwrap_or_default()
    }

    /// 設定の「ベイクで RT コアを使う」を、GPU のデバイスへ反映する（毎フレーム呼んでよい）。ray query はデバイスの作り方が変わるので、値が
    /// 変わったら次の確認・ベイクでデバイスを作り直し、GPU の確認の結果を捨てる。環境変数 `YOLUPAINTER_BAKE_RAY_QUERY` が切なら、設定は効かない。
    pub fn follow_ray_query(&self, setting: bool) {
        let on = setting && yolu_gpu::ray_query_env_allows();
        if self.gpu.ray_query() != on {
            self.gpu.set_ray_query(on);
            if let Ok(mut p) = self.probe.lock() {
                // 世代を進める: 確認の途中のスレッドが、古い設定での結果を書き戻さない
                self.probe_generation.fetch_add(1, Ordering::Relaxed);
                *p = GpuProbe::Unknown;
            }
        }
    }

    /// 試験用: GPU の確認の世代。
    #[doc(hidden)]
    pub fn probe_generation_for_test(&self) -> u64 {
        self.probe_generation.load(Ordering::Relaxed)
    }

    /// 試験用: GPU のデバイスを ray query つきで作る設定になっているか。
    #[doc(hidden)]
    pub fn ray_query_enabled(&self) -> bool {
        self.gpu.ray_query()
    }

    /// 選んだ場所で GPU を使うはずなのに、確かめていなければ別のスレッドで確かめ始める（毎フレーム呼んでよい）。
    /// `ray_query` は設定の「ベイクで RT コアを使う」。
    pub fn ensure_gpu_probe(&self, ray_query: bool) {
        self.follow_ray_query(ray_query);
        let allow_software = match self.backend {
            BakeBackend::Cpu => return,
            BakeBackend::Auto => false,
            BakeBackend::Gpu => true,
        };
        let Ok(mut state) = self.probe.lock() else {
            return;
        };
        match &*state {
            GpuProbe::Probing { allow_software: a } if *a == allow_software => return,
            GpuProbe::Done {
                allow_software: a, ..
            } if *a == allow_software => return,
            _ => {}
        }
        *state = GpuProbe::Probing { allow_software };
        let (slot, shared) = (self.gpu.clone(), self.probe.clone());
        let (generations, generation) = (
            self.probe_generation.clone(),
            self.probe_generation.load(Ordering::Relaxed),
        );
        let spawned = std::thread::Builder::new()
            .name("yolu-gpu-probe".into())
            .spawn(move || {
                let result = slot.probe(allow_software);
                if let Ok(mut p) = shared.lock() {
                    // 確かめている間に設定が変わっていたら、古い設定の結果は捨てる（次のフレームが確かめ直す）
                    if generations.load(Ordering::Relaxed) == generation {
                        *p = GpuProbe::Done {
                            allow_software,
                            result,
                        };
                    }
                }
            });
        if let Err(e) = spawned {
            *state = GpuProbe::Done {
                allow_software,
                result: Err(e.to_string()),
            };
        }
    }

    /// 試験用: GPU の確認の結果を決めておく（別のスレッドで確かめない。画面の見た目を揺らさないため）。
    #[doc(hidden)]
    pub fn fix_gpu_probe(&self, allow_software: bool, result: Result<BakeAdapter, String>) {
        if let Ok(mut p) = self.probe.lock() {
            *p = GpuProbe::Done {
                allow_software,
                result,
            };
        }
    }

    /// GPU が使えるかを別のスレッドで確かめている最中。
    pub fn is_probing_gpu(&self) -> bool {
        matches!(self.gpu_probe(), GpuProbe::Probing { .. })
    }

    /// ウィンドウの状態表示のために、モデルの入力を別のスレッドで作っている（アイランドの索引を作っている・右クリックのメニューが待っているときも、
    /// 描き直しを続けるために真）。
    pub fn is_checking(&self) -> bool {
        self.pending.is_some() || self.pending_islands.is_some() || self.menu_wait.is_some()
    }

    /// 試験用: 別のスレッドで入力を作り始めた回数（毎フレーム作りかけを立てては捨てていないことを数える）。
    #[doc(hidden)]
    pub fn input_builds_started(&self) -> u64 {
        self.input_builds
    }

    pub fn progress(&self) -> Option<Progress> {
        let job = self.job.as_ref()?;
        let (fraction, phase) = job.progress.lock().map(|p| p.clone()).unwrap_or_default();
        Some(Progress {
            fraction,
            phase,
            set: job.name.clone(),
            index: self.finished + 1,
            total: self.total.max(1),
            canceling: job.worker.is_canceled(),
        })
    }
}

// ───────── 仕事の表 ─────────

/// ベイク（ウィンドウを閉じているあいだは札を出す）。
pub(crate) const JOB: JobSpec = JobSpec {
    repaint: true,
    card: Some(card),
    close: Some(|app| app.bake.is_baking().then_some(CloseJob::Bake)),
    cancel: Some(|app| app.apply(Action::Bake(BakeAction::Cancel))),
    poll_while_stopping: Some(AppState::poll_bake),
    ..JobSpec::new("bake", |app| app.bake.is_baking())
};

/// ウィンドウの状態表示のための確かめ（モデルの入力を作る・GPU が使えるかを見る）。描き直すだけ（止めて待たない）。
pub(crate) const CHECK_JOB: JobSpec = JobSpec {
    repaint: true,
    ..JobSpec::new("bake.check", |app| {
        app.bake.is_checking() || app.bake.is_probing_gpu()
    })
};

fn card(app: &AppState, lang: Lang) -> Option<JobCard> {
    if app.bake.window.is_some() {
        return None;
    }
    let p = app.bake.progress()?;
    let set = if p.total > 1 {
        format!(
            "{} {}/{}: {} · ",
            lang.pick("セット", "Set"),
            p.index,
            p.total,
            p.set
        )
    } else {
        String::new()
    };
    Some(JobCard {
        text: format!(
            "{} — {set}{}… {}%",
            lang.pick("メッシュマップをベイク", "Baking mesh maps"),
            if p.canceling {
                lang.pick("取り消し中", "Canceling").to_owned()
            } else {
                phase_label(lang, &p.phase)
            },
            (p.fraction * 100.0) as i32
        ),
        fraction: Some(p.fraction as f32),
        cancel: Some(Action::Bake(BakeAction::Cancel)),
        canceling: p.canceling,
    })
}

// ───────── 名前 ─────────

/// 焼けたが気をつけること（UV の面積が 0 の三角形・縮退した三角形・UV が重なるテクセル）を 1 つの文にする。数は書かない
/// （数は記録 `MeshBakeReport` に残り、試験と計測が読む）。無ければ None。
pub fn uv_warning(lang: Lang, report: &MeshBakeReport) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    if report.zero_uv_area_triangles > 0 {
        parts.push(lang.pick(
            "UV の面積が 0 の三角形があり、その面は焼けません。",
            "Some triangles have no UV area; those faces are not baked.",
        ));
    }
    if report.degenerate_triangles > 0 {
        parts.push(lang.pick(
            "縮退した三角形があり、その面は焼けません。",
            "Some triangles are degenerate; those faces are not baked.",
        ));
    }
    if report.overlap_texels > 0 {
        parts.push(lang.pick(
            "UV が重なる所があり、重なったテクセルにはどちらか一方の面だけが焼かれます。",
            "Some UVs overlap; an overlapping texel takes only one of the faces.",
        ));
    }
    (!parts.is_empty()).then(|| crate::crash::problem(parts.join(" ")))
}

pub fn kind_label(lang: Lang, kind: MeshMapKind) -> &'static str {
    match kind {
        MeshMapKind::WorldNormal => lang.pick("ワールド法線", "World Normal"),
        MeshMapKind::Position => lang.pick("位置", "Position"),
        MeshMapKind::AmbientOcclusion => {
            lang.pick("アンビエントオクルージョン", "Ambient Occlusion")
        }
        MeshMapKind::Curvature => lang.pick("曲率", "Curvature"),
        MeshMapKind::Thickness => lang.pick("厚み", "Thickness"),
        MeshMapKind::TangentNormal => lang.pick("接空間法線", "Tangent Normal"),
        MeshMapKind::Height => lang.pick("ハイト", "Height"),
        MeshMapKind::Id => lang.pick("ID", "ID"),
        MeshMapKind::BentNormal => lang.pick("ベントノーマル", "Bent Normal"),
        MeshMapKind::Opacity => lang.pick("不透明度", "Opacity"),
    }
}

/// 短い名前（一覧の幅に収める。長い名前はツールチップ）。
pub fn kind_short(lang: Lang, kind: MeshMapKind) -> &'static str {
    match kind {
        MeshMapKind::AmbientOcclusion => lang.pick("AO", "AO"),
        other => kind_label(lang, other),
    }
}

/// 高ポリ（参照）が無いと一様になるマップ。
pub fn needs_reference(kind: MeshMapKind) -> bool {
    matches!(
        kind,
        MeshMapKind::TangentNormal | MeshMapKind::Height | MeshMapKind::Opacity
    )
}

/// 焼く場所の名前（切り替えのボタン）。
pub fn backend_label(lang: Lang, backend: BakeBackend) -> &'static str {
    match backend {
        BakeBackend::Auto => lang.pick("自動", "Auto"),
        BakeBackend::Gpu => "GPU",
        BakeBackend::Cpu => "CPU",
    }
}

/// 焼く場所の意味（ツールチップ）。
pub fn backend_help(lang: Lang, backend: BakeBackend) -> &'static str {
    match backend {
        BakeBackend::Auto => lang.pick(
            "使えるハードウェアの GPU があれば GPU、なければ CPU で焼く",
            "Bake on a hardware GPU when there is one, otherwise on the CPU",
        ),
        BakeBackend::Gpu => lang.pick(
            "GPU で焼く（ソフトウェアの描画も使う）。使えなければ理由つきで CPU",
            "Bake on the GPU (software rendering counts too); falls back to the CPU with a reason",
        ),
        BakeBackend::Cpu => lang.pick("CPU で焼く", "Bake on the CPU"),
    }
}

/// アダプターの名前（ウィンドウの状態・結果の一文）。
fn adapter_text(lang: Lang, a: &BakeAdapter) -> String {
    let software = if a.software {
        lang.pick("ソフトウェア", "software")
    } else {
        ""
    };
    let ray = if a.ray_query { "ray query" } else { "" };
    let extra = [a.backend.as_str(), software, ray]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(lang.pick("・", " · "));
    format!("{}{}", a.name, paren(lang, &extra))
}

/// 補足のかっこ（日本語は全角で詰め、英語は半角で前に空白）。
fn paren(lang: Lang, inner: &str) -> String {
    lang.pick(format!("（{inner}）"), format!(" ({inner})"))
}

/// GPU で焼けなかった理由の短い文（詳細の文はツールチップに出す）。
pub fn fallback_text(lang: Lang, kind: FallbackKind) -> &'static str {
    match kind {
        FallbackKind::Unavailable => lang.pick("GPU を使えません", "No usable GPU"),
        FallbackKind::Budget => lang.pick("GPU の予算を超えました", "Over the GPU budget"),
        FallbackKind::Failed => lang.pick("GPU の処理に失敗しました", "The GPU run failed"),
    }
}

/// RT コアを使わなかった理由の短い文（焼く場所の行のツールチップ。理由の種類から言語ごとに作る。自己照合の数などの詳しい文は出さない）。
/// `env_off` は環境変数で切っていること（`RayQueryWhy::Disabled` の言い分けに使う）。
pub fn ray_query_why_text(lang: Lang, why: RayQueryWhy, env_off: bool) -> String {
    let (head, reason) = match why {
        RayQueryWhy::Disabled if env_off => (
            lang.pick("RT コアは使いません", "RT cores are off"),
            lang.pick(
                "環境変数で切っています",
                "turned off by an environment variable",
            ),
        ),
        RayQueryWhy::Disabled => (
            lang.pick("RT コアは使いません", "RT cores are off"),
            lang.pick("設定で切っています", "turned off in the settings"),
        ),
        other => (
            lang.pick("RT コアを使えませんでした", "RT cores were not used"),
            match other {
                RayQueryWhy::NotSupported => {
                    lang.pick("この GPU は対応していません", "not supported by this GPU")
                }
                RayQueryWhy::NotApplicable => {
                    lang.pick("このベイクは対象外です", "not available for this bake")
                }
                RayQueryWhy::Device => lang.pick(
                    "デバイスを作れませんでした",
                    "the GPU device could not be created",
                ),
                RayQueryWhy::Shader => lang.pick(
                    "シェーダーを作れませんでした",
                    "the shader could not be built",
                ),
                RayQueryWhy::Accel => lang.pick(
                    "加速構造を作れませんでした",
                    "the acceleration structure could not be built",
                ),
                RayQueryWhy::CheckRun => {
                    lang.pick("照合を回せませんでした", "the check could not be run")
                }
                RayQueryWhy::CheckFailed => {
                    lang.pick("照合に通りませんでした", "it did not pass the check")
                }
                RayQueryWhy::RunFailed => lang.pick(
                    "焼いている途中で失敗したので止めました",
                    "it failed while baking, so it was turned off",
                ),
                RayQueryWhy::Disabled => unreachable!("上で扱った"),
            },
        ),
    };
    paren_text(lang, head, reason)
}
/// 「頭（理由）」。日本語は全角のかっこで詰め、英語は半角で前に空白。
fn paren_text(lang: Lang, head: &str, inner: &str) -> String {
    format!("{head}{}", paren(lang, inner))
}

/// 焼く場所の一行（ウィンドウの状態・記録）。`warn` は CPU に戻った注意、`detail` はツールチップに出す詳しい理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceLine {
    pub text: String,
    pub warn: bool,
    pub detail: Option<String>,
}

/// GPU の確認の状態の一行（`Cpu` を選んでいるときは無し）。
pub fn probe_line(lang: Lang, backend: BakeBackend, probe: &GpuProbe) -> Option<PlaceLine> {
    if backend == BakeBackend::Cpu {
        return None;
    }
    let want_software = backend == BakeBackend::Gpu;
    Some(match probe {
        GpuProbe::Done {
            allow_software,
            result,
        } if *allow_software == want_software => match result {
            Ok(a) => PlaceLine {
                text: format!("GPU: {}", adapter_text(lang, a)),
                warn: false,
                detail: None,
            },
            Err(detail) => PlaceLine {
                text: format!(
                    "{}{}",
                    lang.pick("CPU で焼く", "Bakes on the CPU"),
                    paren(lang, fallback_text(lang, FallbackKind::Unavailable))
                ),
                warn: true,
                detail: Some(detail.clone()),
            },
        },
        _ => PlaceLine {
            text: lang.pick("GPU を確認中", "Checking the GPU").to_owned(),
            warn: false,
            detail: None,
        },
    })
}

/// 最後のベイクを行った場所の一行（結果の一文・記録の行）。`Cpu` を選んだときの CPU は注意にしない。
pub fn run_line(lang: Lang, run: &BakeRun) -> PlaceLine {
    match &run.gpu {
        Some((adapter, stats)) => {
            let method = match stats.method {
                GpuBakeMethod::Compute => "compute",
                GpuBakeMethod::RayQuery => "ray query",
            };
            let software = if adapter.software {
                lang.pick("ソフトウェア・", "software · ")
            } else {
                ""
            };
            PlaceLine {
                text: format!(
                    "GPU {}{}",
                    adapter.name,
                    paren(lang, &format!("{software}{method}"))
                ),
                warn: false,
                // RT コアを使わなかった理由の種類（使えたときは何も出さない。自己照合の数は `ray_query_note` とログだけ）
                detail: stats
                    .ray_query_why
                    .map(|why| ray_query_why_text(lang, why, !yolu_gpu::ray_query_env_allows())),
            }
        }
        None => match run.fallback_kind {
            Some(kind) => PlaceLine {
                text: format!("CPU{}", paren(lang, fallback_text(lang, kind))),
                warn: true,
                detail: run.fallback_reason.clone(),
            },
            None => PlaceLine {
                text: "CPU".to_owned(),
                warn: false,
                detail: None,
            },
        },
    }
}

pub fn phase_label(lang: Lang, phase: &str) -> String {
    match phase {
        "Preparing" => lang.pick("準備中", "Preparing"),
        "Baking" => lang.pick("ベイク中", "Baking"),
        "Padding" => lang.pick("余白", "Padding"),
        "Done" => lang.pick("完了", "Done"),
        other => other,
    }
    .to_owned()
}

pub fn id_source_label(lang: Lang, source: MeshIdSource) -> &'static str {
    match source {
        MeshIdSource::MaterialSlot => lang.pick("スロット", "Slot"),
        MeshIdSource::Mesh => lang.pick("メッシュ", "Mesh"),
        MeshIdSource::UvIsland => lang.pick("UV アイランド", "UV Island"),
        MeshIdSource::MeshPart => lang.pick("メッシュの部品", "Mesh Part"),
        MeshIdSource::VertexColor => lang.pick("頂点カラー", "Vertex Color"),
        MeshIdSource::MaterialAsset => lang.pick("マテリアル", "Material"),
    }
}

/// ウィンドウで選べる ID の分け方（頂点カラーと素材の識別は、入力に渡さないので選べない）。
pub const ID_SOURCES: [MeshIdSource; 4] = [
    MeshIdSource::MaterialSlot,
    MeshIdSource::Mesh,
    MeshIdSource::UvIsland,
    MeshIdSource::MeshPart,
];

/// 状態の名前（一覧の右端）。
pub fn state_label(lang: Lang, state: Option<MeshMapState>) -> &'static str {
    match state {
        None => lang.pick("未", "Not baked"),
        Some(MeshMapState::Current) => lang.pick("最新", "Current"),
        Some(MeshMapState::Stale) => lang.pick("古い", "Stale"),
        Some(MeshMapState::Unverified) => lang.pick("未確認", "Unchecked"),
    }
}

/// スロットの見せ方（"0" や "0, 2"）。
pub fn slot_list(slots: &[i32]) -> String {
    if slots.is_empty() {
        "—".into()
    } else {
        slots
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// 古い理由（1 行。由来の文字列に `;` を足して折り返せるように）。
pub fn stale_reasons(check: &MeshMapCheck) -> String {
    check
        .reasons
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join(" / ")
}

/// 焼いた AO を書き出しに使えるか。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OcclusionState {
    /// 焼いていない。
    None,
    /// 焼いてあるが今の条件のものではない（使わない）。理由。
    Stale(String),
    /// 今の条件で焼いたもの。
    Current,
}

/// 書き出しの AO の元。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Occlusion {
    /// 焼いていない。
    None,
    /// 焼いてあるが今の条件のものではない（使わない）。理由。
    Stale(String),
    /// 今の条件で焼いたもの（1 テクセル 1 バイト。覆わないテクセルは 255）。
    Bytes(Vec<u8>),
}

impl AppState {
    /// 今のモデルの形（試しの立方体を含む）。
    fn bake_model(&self) -> Option<&Arc<crate::view3d::model::ViewModel>> {
        self.view3d.full_model()
    }

    /// セットが付いているモデルのマテリアルの番号（モデルの記録の無い立方体は、今のセットがマテリアル 0）。
    pub(crate) fn set_material(&self, index: usize) -> Option<i32> {
        let model = self.view3d.full_model()?;
        let set = self.sets.get(index)?;
        match &self.model {
            Some(record) => {
                // 3D が指しているモデルが記録と同じものか（形ごと渡されたメッシュの世代）
                if record.is_mesh() && self.view3d.link_generation() != Some(record.generation) {
                    return None;
                }
                let _ = model;
                set.bound.map(|m| m as i32)
            }
            None => (index == self.sets.current_index()).then_some(0),
        }
    }

    /// セットが受け持つスロット（モデルに無ければ None）。
    pub fn set_slots(&self, index: usize) -> Option<Vec<i32>> {
        let material = self.set_material(index)?;
        let slots = input::slots_of_material(self.bake_model()?, material);
        (!slots.is_empty()).then_some(slots)
    }

    /// モデルから作った入力（同じモデルのあいだは作り直さない）。モデルが無い・作れないときは理由。始める・書き出すときの
    /// 入力で、作るのが済んでいなければここで作る（別のスレッドが同じモデルのものを作っていればその終わりを待つ）。
    pub fn bake_input(&mut self) -> Result<Arc<MeshBakeInput>, String> {
        let lang = self.lang;
        let Some(model) = self.view3d.full_model().cloned() else {
            return Err(lang.pick("モデルがありません", "No model").into());
        };
        if let Some(cached) = self.bake.input.as_ref().filter(|c| c.model.is(&model)) {
            return cached.input.clone();
        }
        let built = match self.bake.pending.take() {
            Some(p) if p.model.is(&model) => p.rx.recv().unwrap_or_else(|_| {
                Err(lang
                    .pick("入力を作れませんでした", "The input was not built")
                    .into())
            }),
            // 別のモデルのものは使わない（作り終えても捨てられる）
            _ => input::build_input(&model),
        };
        self.cache_input(&model, built)
    }

    /// ウィンドウの状態表示のための入力。作ってあればそれ。モデルが替わっていれば別のスレッドで作り直し（UI のスレッドを止めない。
    /// 作っているのは 1 つだけで、モデルがまた替われば終わってから最新のものを作る）、できるまでは None。ポーズを変えている間は
    /// 始めない（毎フレーム別の形になる）。
    pub fn bake_input_nowait(&mut self) -> Option<Result<Arc<MeshBakeInput>, String>> {
        let lang = self.lang;
        let Some(model) = self.view3d.full_model().cloned() else {
            self.bake.pending = None;
            return Some(Err(lang.pick("モデルがありません", "No model").into()));
        };
        if let Some(cached) = self.bake.input.as_ref().filter(|c| c.model.is(&model)) {
            return Some(cached.input.clone());
        }
        if let Some(p) = self.bake.pending.take() {
            let lost = || -> Result<MeshBakeInput, String> {
                Err(lang
                    .pick("入力を作れませんでした", "The input was not built")
                    .into())
            };
            match p.rx.try_recv() {
                Ok(built) if p.model.is(&model) => return Some(self.cache_input(&model, built)),
                Err(TryRecvError::Disconnected) if p.model.is(&model) => {
                    return Some(self.cache_input(&model, lost()));
                }
                // 古いモデルの結果は捨てて、今のモデルを作り直す
                Ok(_) | Err(TryRecvError::Disconnected) => {}
                Err(TryRecvError::Empty) => {
                    self.bake.pending = Some(p);
                    return None;
                }
            }
        }
        if self.view3d.pose.drag.is_some() {
            return None;
        }
        let (tx, rx) = channel();
        let worker_model = model.clone();
        let spawned = std::thread::Builder::new()
            .name("yolu-bake-input".into())
            .spawn(move || {
                let _ = tx.send(input::build_input(&worker_model));
            });
        match spawned {
            Ok(_) => {
                self.bake.input_builds += 1;
                self.bake.pending = Some(PendingInput {
                    model: ModelId::of(&model),
                    rx,
                });
                None
            }
            Err(e) => Some(self.cache_input(&model, Err(e.to_string()))),
        }
    }

    /// 手元の入力（作ってあるもの。今のモデルのものとは限らない。無ければ None）。効果の入力の同期が、モデルの差し替え・ポーズの変更を追わないときに使う。
    pub(crate) fn bake_input_held(&self) -> Option<Result<Arc<MeshBakeInput>, String>> {
        self.bake.input.as_ref().map(|c| c.input.clone())
    }

    /// 作り終えている今のモデルの入力（作っている最中・まだ作っていなければ None。作り始めも待ちもしない。ベイクのウィンドウが入力を作っている
    /// あいだ、効果の入力が毎フレーム横から見る）。
    pub(crate) fn bake_input_ready(&self) -> Option<Result<Arc<MeshBakeInput>, String>> {
        let model = self.view3d.full_model()?;
        self.bake
            .input
            .as_ref()
            .filter(|c| c.model.is(model))
            .map(|c| c.input.clone())
    }

    fn cache_input(
        &mut self,
        model: &Arc<ViewModel>,
        built: Result<MeshBakeInput, String>,
    ) -> Result<Arc<MeshBakeInput>, String> {
        let built = built.map(Arc::new);
        self.bake.input = Some(CachedInput {
            model: ModelId::of(model),
            input: built.clone(),
        });
        built
    }

    /// セット 1 つを焼く設定（大きさは文書、スロットはセットのもの）とスロット。
    fn bake_target(&self, index: usize) -> Result<(MeshBakeSettings, Vec<i32>), String> {
        let lang = self.lang;
        let set = self
            .sets
            .get(index)
            .ok_or_else(|| lang.pick("テクスチャセットがありません", "No such texture set"))?;
        let Some(slots) = self.set_slots(index) else {
            return Err(lang.pick(
                format!(
                    "テクスチャセット「{}」のマテリアルは、読み込んだモデルにありません",
                    set.name
                ),
                format!(
                    "The material of texture set \"{}\" is not in the loaded model",
                    set.name
                ),
            ));
        };
        let doc = self.set_doc(index);
        let mut settings = self.bake.settings.clone();
        // 手動の ID の色はセットの文書の状態（ID マップに入る。別のモデルのものなら core が焼く前に断る）
        settings.manual_id_colors = doc.id_colors().clone();
        // 重なった UV の持ち主の決め方もセットの文書の状態（手で選んだアイランドが別のモデルのものなら core が焼く前に断る）
        settings.overlap = doc.bake_priority().clone();
        settings.width = doc.width() as i32;
        settings.height = doc.height() as i32;
        settings.target_slot = slots[0];
        settings.target_slots = if slots.len() > 1 {
            slots.clone()
        } else {
            Vec::new()
        };
        settings.validate().map_err(|e| e.to_string())?;
        Ok((settings, slots))
    }

    /// マップを使う側の今の条件（モデル・セットの文書の大きさ・スロット・ウィンドウの設定）。入力が None ならモデルが無い。
    pub fn mesh_map_expectation(
        &self,
        index: usize,
        input: Option<&MeshBakeInput>,
    ) -> MeshMapExpectation {
        let doc = self.set_doc(index);
        let slots = self.set_slots(index);
        // 手動の ID の色を直したら、前の ID マップは古い（焼いたときの設定と同じ手動の色で比べる）
        let mut settings = self.bake.settings.clone();
        settings.manual_id_colors = doc.id_colors().clone();
        // 持ち主の決め方を変えたら、前のマップは古い
        settings.overlap = doc.bake_priority().clone();
        MeshMapExpectation {
            mesh_hash: input.map(|i| i.hash().to_owned()),
            topology_hash: input.map(|i| i.topology_hash().to_owned()),
            reference_hash: None,
            width: doc.width() as i32,
            height: doc.height() as i32,
            target_slot: slots.as_ref().map_or(-2, |s| s[0]),
            target_slots: slots.unwrap_or_default(),
            uv_channel: 0,
            settings: Some(settings),
            material_identity: input
                .map(|i| material_identity(i, None))
                .unwrap_or_default(),
        }
    }

    /// セットのマップ 1 枚の今の状態（焼いていなければ None）。入力を作る必要があればここで作る。
    pub fn mesh_map_check(&mut self, index: usize, kind: MeshMapKind) -> Option<MeshMapCheck> {
        self.sets.get(index)?.mesh_maps.get(kind)?;
        let input = self.bake_input().ok();
        self.mesh_map_check_with(index, kind, input.as_deref())
    }

    /// 渡した入力でのマップ 1 枚の今の状態（焼いていなければ None。入力が None ならモデルが無いとして照合する）。
    pub fn mesh_map_check_with(
        &self,
        index: usize,
        kind: MeshMapKind,
        input: Option<&MeshBakeInput>,
    ) -> Option<MeshMapCheck> {
        let map = self.sets.get(index)?.mesh_maps.get(kind)?;
        let expected = self.mesh_map_expectation(index, input);
        Some(map.provenance().check(&expected))
    }

    /// 焼けるテクスチャセット（チェックしていて、モデルにマテリアルがあるもの。並びの順）。
    pub fn bakeable_sets(&self) -> Vec<usize> {
        (0..self.sets.len())
            .filter(|&i| {
                self.sets
                    .get(i)
                    .is_some_and(|s| !self.bake.skipped.contains(&s.uid))
                    && self.set_slots(i).is_some()
            })
            .collect()
    }

    /// 今ベイクを始められない理由（始められれば None）。ウィンドウのボタンと `Start` が同じ理由で断る。入力を作る必要があればここで作る。
    pub fn bake_refusal(&mut self) -> Option<String> {
        self.refusal(true)
    }

    /// ウィンドウに出す理由。`bake_refusal` と同じ順で、入力を作っている最中は入力の理由を出さない（押せば `Start` が作るか、作り終えるのを
    /// 待って断る。ウィンドウを描くたびに UI のスレッドで作らない）。
    pub fn bake_refusal_nowait(&mut self) -> Option<String> {
        self.refusal(false)
    }

    fn refusal(&mut self, wait: bool) -> Option<String> {
        let lang = self.lang;
        if self.is_stroking() {
            return Some(crate::lang::refusals::during_stroke(lang).into());
        }
        if self.bake.job.is_some() {
            return Some(lang.pick("ベイク中です", "Already baking").into());
        }
        if self.view3d.pose.drag.is_some() {
            return Some(
                lang.pick("ポーズを変えている間はできません", "Not while posing")
                    .into(),
            );
        }
        if self.view3d.full_model().is_none() {
            return Some(lang.pick("モデルがありません", "No model").into());
        }
        if self.bake.settings.maps.is_empty() {
            return Some(
                lang.pick("チェックしたマップがありません", "No map is checked")
                    .into(),
            );
        }
        if self.bakeable_sets().is_empty() {
            let any_in_model = (0..self.sets.len()).any(|i| self.set_slots(i).is_some());
            return Some(
                if any_in_model {
                    lang.pick(
                        "チェックしたテクスチャセットがありません",
                        "No texture set is checked",
                    )
                } else {
                    lang.pick(
                        "モデルに付いたテクスチャセットがありません",
                        "No texture set is on the model",
                    )
                }
                .into(),
            );
        }
        let input = if wait {
            Some(self.bake_input())
        } else {
            self.bake_input_nowait()
        };
        match input {
            Some(Err(e)) => Some(lang.with_reason(lang.pick("ベイクできません", "Cannot bake"), e)),
            Some(Ok(_)) | None => None,
        }
    }

    /// セットに AO のマップが焼いてあるか（今の条件のものかは [`AppState::occlusion_state`]）。
    pub fn has_baked_occlusion(&self, index: usize) -> bool {
        self.sets
            .get(index)
            .is_some_and(|s| s.mesh_maps.get(MeshMapKind::AmbientOcclusion).is_some())
    }

    /// 焼いた AO を書き出しに使えるか（バイト列は作らない）。`input` は今のモデルの入力（無ければ照合できない）。
    pub fn occlusion_state(&self, index: usize, input: Option<&MeshBakeInput>) -> OcclusionState {
        let lang = self.lang;
        let Some(map) = self
            .sets
            .get(index)
            .and_then(|s| s.mesh_maps.get(MeshMapKind::AmbientOcclusion))
        else {
            return OcclusionState::None;
        };
        let doc = self.set_doc(index);
        if (map.width(), map.height()) != (doc.width() as usize, doc.height() as usize) {
            return OcclusionState::Stale(
                lang.pick("焼いた大きさがキャンバスと違う", "baked size differs")
                    .into(),
            );
        }
        let expected = self.mesh_map_expectation(index, input);
        let check = map.provenance().check(&expected);
        match check.state {
            MeshMapState::Current => OcclusionState::Current,
            MeshMapState::Unverified => OcclusionState::Stale(
                lang.pick("モデルが無く照合できない", "no model to check against")
                    .into(),
            ),
            MeshMapState::Stale => OcclusionState::Stale(stale_reasons(&check)),
        }
    }

    /// 書き出しに使う AO（今の条件で焼いたものだけ）。
    pub fn occlusion_for_export(&mut self, index: usize) -> Occlusion {
        if !self.has_baked_occlusion(index) {
            return Occlusion::None;
        }
        let input = self.bake_input().ok();
        match self.occlusion_state(index, input.as_deref()) {
            OcclusionState::None => Occlusion::None,
            OcclusionState::Stale(why) => Occlusion::Stale(why),
            OcclusionState::Current => {
                let map = self
                    .sets
                    .get(index)
                    .and_then(|s| s.mesh_maps.get(MeshMapKind::AmbientOcclusion))
                    .expect("今の条件と照合できたマップ");
                let (w, h) = (map.width(), map.height());
                let mut bytes = vec![255u8; w * h];
                for y in 0..h {
                    for x in 0..w {
                        let i = y * w + x;
                        if map.coverage()[i] != 0 {
                            let v = map.value(x as i32, y as i32, 0).unwrap_or(1.0);
                            bytes[i] = occlusion_byte(v);
                        }
                    }
                }
                Occlusion::Bytes(bytes)
            }
        }
    }

    /// .ylp を開いてモデルを読み直したとき（`newproject::reopen`・Live Link の開き直し）に呼ぶ: 焼いたマップを持つセットがあれば、読み終えた
    /// モデルの入力（指紋）が効果の入力へ渡って照合し直しが 1 回済むまで、作りかけの入力を手放さない。開いた瞬間のマップは試しの立方体と
    /// 照合して「古い」で、読み終えた後に入力を作る間もウィンドウが閉じていると、読むだけにならずに動き続けるセット（位置の空間のノイズなど）は
    /// 「古い」のまま、いつまでも照合し直せなかった。済んだら今までどおり手放し、ポーズの変更・モデルの差し替えには追わない。
    pub(crate) fn expect_reopen_check(&mut self) {
        self.bake.reopen_check =
            self.view3d
                .latest_model()
                .filter(|m| !m.demo)
                .map(|model| ReopenCheck {
                    generation: self.np.generation,
                    model: ModelId::of(model),
                });
    }

    /// 今のモデルの入力を、作って待つか（ウィンドウ・ID の色のツール・アイランドのメニュー・入力待ちの読むだけのセット・開いた直後の照合し直し・
    /// 焼いた AO を照合する書き出しのウィンドウがあるとき。焼いたマップがあるのに手元の入力が 1 つも無いときも、作る）。待つあいだは、作りかけを手放さず（`release_idle_bake_input`）、
    /// 効果の入力の同期も今のモデルの入力が届くまで待つ。待たないときは、手元の入力のまま照合して、新しく作り始めない（モデルの差し替え・
    /// ポーズの変更は追わない）。手放す側と作る側が別の条件で動くと、作りかけを毎フレーム立てては捨てて、入力がいつまでも届かない。
    pub(crate) fn bake_input_followed(&mut self) -> bool {
        self.bake.window.is_some()
            || self.tool == crate::state::Tool::IdSelect
            || self.island_menu_open()
            || self.sets.iter().any(|s| s.waiting_inputs)
            || self.reopen_check_waiting()
            || self.export_window_follows_input()
            || (self.bake.input.is_none() && self.sets.iter().any(|s| !s.mesh_maps.is_empty()))
    }

    /// アイランドのメニュー（ポリゴン塗りつぶしの右クリック）を開いている間も、アイランドの強調と選んだ項目が入力を使う
    /// （右クリックを押してから離すまで・離したメニューがアイランドの索引を待っている間も同じ。押したまま離さなかったときは、少しで手放す）。
    fn island_menu_open(&self) -> bool {
        matches!(
            self.popup.as_ref().map(|p| p.kind),
            Some(crate::state::PopupKind::BakeIsland { .. })
        ) || self.bake.menu_wait.is_some()
            || self.bake.menu_press.is_some()
    }

    /// 開いた直後の照合し直しをまだ待っているか。待たなくなったら（済んだ・別のプロジェクト・焼いたマップが無い・照合するモデルから替わった）
    /// 印も外す。
    fn reopen_check_waiting(&mut self) -> bool {
        let Some(check) = self.bake.reopen_check.as_ref() else {
            return false;
        };
        let maps = self.sets.iter().any(|s| !s.mesh_maps.is_empty());
        // 照合するモデル（描いている最中なら、入れ替わりを待っているモデル）のままで、その入力がまだできていない。できた入力は、焼いたマップが
        // あるあいだ手放さず、毎フレームの同期が効果の入力へ渡す
        let waiting = check.generation == self.np.generation
            && maps
            && self.view3d.latest_model().is_some_and(|m| {
                check.model.is(m) && !self.bake.input.as_ref().is_some_and(|c| c.model.is(m))
            });
        if !waiting {
            self.bake.reopen_check = None;
        }
        waiting
    }

    /// ウィンドウを閉じていて、焼いたマップも走っているベイクも無ければ、作った入力を手放す（大きなモデルの写しを持ち続けない。要るときに
    /// 作り直す）。作っている最中のものも、今のモデルの入力を待つ理由（`bake_input_followed`）が無ければ手放す。毎フレーム呼ぶ。
    pub fn release_idle_bake_input(&mut self) {
        // 待つ理由（ID の色のツールは強調・部品の欄が毎フレーム入力を求める、入力がそろうのを待つ読むだけのセット、開いた直後の照合し直し、など）が
        // あるあいだに作りかけを手放すと、毎フレーム作り直しが始まって終わらない。効果の入力の同期も同じ条件で入力を作る
        if !self.bake_input_followed() {
            self.bake.pending = None;
        }
        let island_menu = self.island_menu_open();
        if self.bake.input.is_some()
            && self.bake.window.is_none()
            && !island_menu
            && self.bake.job.is_none()
            && self.sets.iter().all(|s| s.mesh_maps.is_empty())
        {
            self.bake.input = None;
        }
        self.release_overlap_caches();
    }

    /// 手で選ぶアイランドの索引（`Islands`）とウィンドウの見取り図（`MapData`）は、ベイクの入力・モデルの形を握る。入力を手放したあと・別の入力に
    /// 替わったあとに残すと、入力の写しも古いモデル・ポーズの形も解放されない。索引は今の入力の物だけ、見取り図はウィンドウを開いている間の
    /// 今の索引の上の物だけ持つ（アイランドのメニューを開いている間も、メニューが使うのは索引で、見取り図ではない）。毎フレーム呼ぶ。
    fn release_overlap_caches(&mut self) {
        let input = self.bake.input.as_ref().and_then(|c| c.input.as_ref().ok());
        if !self
            .bake
            .islands
            .as_ref()
            .is_some_and(|i| input.is_some_and(|input| i.is_on(input)))
        {
            self.bake.islands = None;
        }
        if !self
            .bake
            .pending_islands
            .as_ref()
            .is_some_and(|p| input.is_some_and(|input| p.is_on(input)))
        {
            self.bake.pending_islands = None;
        }
        let map_on_islands = match (&self.bake.map, &self.bake.islands) {
            (Some(m), Some(i)) => m.is_on(i),
            (Some(_), None) => false,
            (None, _) => true,
        };
        if self.bake.window.is_none() || !map_on_islands {
            self.bake.map = None;
        }
    }

    // ───────── 操作 ─────────

    pub fn bake_apply(&mut self, action: BakeAction) {
        let lang = self.lang;
        match action {
            BakeAction::OpenWindow => {
                if self.bake.window.is_none() {
                    self.bake.window = Some(window::BakeWindow::default());
                }
                self.bake
                    .ensure_gpu_probe(self.prefs.settings.bake_ray_query);
            }
            BakeAction::Backend(backend) => {
                self.bake.backend = backend;
                self.bake
                    .ensure_gpu_probe(self.prefs.settings.bake_ray_query);
            }
            BakeAction::CloseWindow => {
                self.bake.window = None;
                self.bake.pick = None;
                self.bake.map_hover = None;
                // 見取り図はウィンドウの物（形を握り続けない）。アイランドのメニューが使うのはアイランドの索引で、見取り図ではない
                self.bake.map = None;
            }
            BakeAction::Priority(op) => self.bake_priority_apply(op),
            BakeAction::Start => self.start_bake(),
            BakeAction::Cancel => self.cancel_bake(),
            BakeAction::Map(kind, on) => {
                let maps = &mut self.bake.settings.maps;
                if on {
                    if !maps.contains(&kind) {
                        maps.push(kind);
                        maps.sort_by_key(|k| MeshMapKind::ALL.iter().position(|a| a == k));
                    }
                } else if maps.len() > 1 {
                    maps.retain(|k| *k != kind);
                } else {
                    self.refuse(
                        Source::Bake,
                        lang.pick(
                            "マップを 1 つは残します。",
                            "At least one map stays checked.",
                        ),
                    );
                }
            }
            BakeAction::Set(uid, on) => {
                if on {
                    self.bake.skipped.remove(&uid);
                } else {
                    let checked = self.bakeable_sets();
                    let last = checked.len() == 1
                        && self.sets.get(checked[0]).is_some_and(|s| s.uid == uid);
                    if last {
                        self.refuse(
                            Source::Bake,
                            lang.pick(
                                "テクスチャセットを 1 つは残します。",
                                "At least one texture set stays checked.",
                            ),
                        );
                    } else {
                        self.bake.skipped.insert(uid);
                    }
                }
            }
            BakeAction::View(view) => {
                self.bake.view = view;
                self.sync_mesh_map_view();
            }
        }
    }

    /// 見ているマップが今のセットに無ければ、重ね表示をやめる。
    pub fn sync_mesh_map_view(&mut self) {
        let maps = &self.sets.current().mesh_maps;
        let valid = match self.bake.view {
            MeshMapView::None => true,
            MeshMapView::Coverage => !maps.is_empty(),
            MeshMapView::Kind(k) => maps.get(k).is_some(),
        };
        if !valid {
            self.bake.view = MeshMapView::None;
        }
    }

    fn start_bake(&mut self) {
        if let Some(reason) = self.bake_refusal() {
            self.refuse(Source::Bake, reason.clone());
            self.bake.outcome = Some((reason, false));
            return;
        }
        let queue: VecDeque<u32> = self
            .bakeable_sets()
            .into_iter()
            .filter_map(|i| self.sets.get(i).map(|s| s.uid))
            .collect();
        self.bake.total = queue.len();
        self.bake.finished = 0;
        self.bake.queue = queue;
        self.bake.outcome = None;
        self.bake.uv_notes.clear();
        self.start_next_bake();
    }

    /// 並びの次のセットを焼き始める。始めたら true（断ったら理由を知らせて残りを捨てる）。
    fn start_next_bake(&mut self) -> bool {
        let lang = self.lang;
        while let Some(uid) = self.bake.queue.pop_front() {
            let Some(index) = self.sets.index_of(uid) else {
                self.bake.finished += 1;
                continue;
            };
            match self.prepare_bake(index) {
                Ok(job) => {
                    self.info(
                        Source::Bake,
                        lang.pick("メッシュマップをベイク中…", "Baking mesh maps…"),
                    );
                    self.bake.job = Some(job);
                    return true;
                }
                Err(reason) => {
                    self.bake.queue.clear();
                    self.bake_ended(
                        crate::notice::Kind::Error,
                        lang.with_reason(lang.pick("ベイクできません", "Cannot bake"), reason),
                        false,
                    );
                    return false;
                }
            }
        }
        false
    }

    fn prepare_bake(&mut self, index: usize) -> Result<Job, String> {
        let lang = self.lang;
        let input = self.bake_input()?;
        let (settings, slots) = self.bake_target(index)?;
        let model = self
            .view3d
            .full_model()
            .map(ModelId::of)
            .ok_or_else(|| lang.pick("モデルがありません", "No model").to_owned())?;
        let set = self.sets.get(index).expect("確かめた");
        let (uid, name) = (set.uid, set.name.clone());
        let doc = self.set_doc(index);
        let (doc_id, size) = (doc.id(), (doc.width(), doc.height()));
        let progress = Arc::new(Mutex::new((0.0, "Preparing".to_owned())));
        let (shared, run_settings) = (progress.clone(), settings.clone());
        let park = std::mem::take(&mut self.bake.park_next);
        let park_mid = std::mem::take(&mut self.bake.park_mid_bake);
        let backend = self.bake.backend;
        self.bake
            .follow_ray_query(self.prefs.settings.bake_ray_query);
        let gpu = self.bake.gpu.clone();
        let worker = Worker::spawn("yolu-bake", move |tx, cancel| {
            let flag = cancel.flag();
            if park {
                crate::windows::park_until_canceled(flag);
            }
            let mut baking = 0;
            let result = bake_mesh_maps(
                backend,
                &gpu,
                &input,
                &run_settings,
                &MeshBakeBudget::default(),
                Some(flag),
                None,
                |fraction, phase| {
                    if park_mid && phase == "Baking" {
                        // 1 回目は焼き始めの通知。2 回目が、最初の dispatch（行）を終えたあと
                        baking += 1;
                        if baking == 2 {
                            crate::windows::park_until_canceled(flag);
                        }
                    }
                    if let Ok(mut p) = shared.lock() {
                        *p = (fraction, phase.to_owned());
                    }
                    true
                },
            )
            .map_err(|e| e.to_string());
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())?;
        Ok(Job {
            uid,
            name,
            settings,
            progress,
            worker,
            doc_id,
            size,
            slots,
            model,
        })
    }

    /// 取り消しを頼む（止まったら `poll_bake` が知らせる。残りのセットは焼かない）。
    fn cancel_bake(&mut self) {
        let lang = self.lang;
        if let Some(job) = &self.bake.job {
            self.bake.queue.clear();
            job.worker.cancel();
            self.info(
                Source::Bake,
                lang.pick(
                    "メッシュマップのベイクを取り消しています…",
                    "Canceling the mesh-map bake…",
                ),
            );
        }
    }

    /// 終わったベイクを入れる（フレームの初めに）。
    pub fn poll_bake(&mut self) {
        let lang = self.lang;
        let Some(job) = &self.bake.job else {
            return;
        };
        let result = match job.worker.poll() {
            Polled::Message(r) => r,
            Polled::Empty => return,
            Polled::Lost => Err(lang.pick("ベイクが止まりました", "The bake stopped").into()),
        };
        let job = self.bake.job.take().expect("上で見た");
        self.finish_bake(job, result);
    }

    /// 始めたときから、結果の意味を変えるものが変わったか。変わったものの名前。
    fn bake_changed(&self, job: &Job) -> Option<&'static str> {
        let lang = self.lang;
        let Some(index) = self.sets.index_of(job.uid) else {
            return Some(lang.pick("テクスチャセット", "the texture set"));
        };
        let doc = self.set_doc(index);
        if doc.id() != job.doc_id || (doc.width(), doc.height()) != job.size {
            return Some(lang.pick("キャンバス", "the canvas"));
        }
        if self.set_slots(index).as_deref() != Some(&job.slots[..]) {
            return Some(lang.pick("マテリアルのスロット", "the material slots"));
        }
        if !self.view3d.full_model().is_some_and(|m| job.model.is(m)) {
            return Some(lang.pick("モデルかそのポーズ", "the model or its pose"));
        }
        None
    }

    fn finish_bake(&mut self, job: Job, result: Result<(MeshBakeResult, BakeRun), String>) {
        let lang = self.lang;
        self.bake.finished += 1;
        let (result, run) = match result {
            Ok(r) => r,
            Err(e) => {
                self.bake.queue.clear();
                return self.bake_ended(
                    crate::notice::Kind::Error,
                    lang.with_reason(
                        lang.pick(
                            "メッシュマップをベイクできません",
                            "Cannot bake the mesh maps",
                        ),
                        e,
                    ),
                    false,
                );
            }
        };
        match result.status {
            MeshBakeStatus::Canceled | MeshBakeStatus::TimedOut => {
                self.bake.queue.clear();
                // 時間切れは失敗、取り消しは利用者の操作の結果（済んだ知らせ）
                let (kind, text) = if result.status == MeshBakeStatus::TimedOut {
                    (
                        crate::notice::Kind::Error,
                        lang.pick(
                            "ベイクが時間切れになりました（前のマップはそのまま）",
                            "The bake hit its time limit; the previous maps are unchanged",
                        ),
                    )
                } else {
                    (
                        crate::notice::Kind::Info,
                        lang.pick(
                            "ベイクを取り消しました（前のマップはそのまま）",
                            "The bake was canceled; the previous maps are unchanged",
                        ),
                    )
                };
                return self.bake_ended(kind, text.into(), false);
            }
            MeshBakeStatus::Completed => {}
        }
        if let Some(what) = self.bake_changed(&job) {
            self.bake.queue.clear();
            return self.bake_ended(
                crate::notice::Kind::Warning,
                lang.pick(
                    format!("{what}が変わったので、焼いた結果を捨てました（前のマップはそのまま）"),
                    format!("Discarded the bake because {what} changed; the previous maps are unchanged"),
                ),
                false,
            );
        }
        let index = self.sets.index_of(job.uid).expect("上で確かめた");
        let kinds = result
            .maps
            .iter()
            .map(|m| kind_short(lang, m.kind()))
            .collect::<Vec<_>>()
            .join(lang.pick("・", ", "));
        let count = result.maps.len();
        if let Some(set) = self.sets.get_mut(index) {
            // 記録と場所は、今のマップを焼いたベイクのもの。取消・時間切れ・捨てた結果では変えない（前のマップが残るので、それを
            // 焼いた記録も残す。準備の途中で止めた GPU を「GPU で焼いた」と見せない）
            set.mesh_maps.set_report(result.report.clone());
            set.mesh_maps.set_run(run.clone());
            set.mesh_maps.put(result.maps);
        }
        self.modified = true;
        if self.bake.view == MeshMapView::None && index == self.sets.current_index() {
            self.bake.view = MeshMapView::Coverage;
        }
        let secs = result.report.total_seconds;
        let place = run_line(lang, &run).text;
        let mut text = lang.pick(
            format!(
                "{}の {}×{} のメッシュマップ {count} 枚（{kinds}）を {secs:.2} 秒で焼きました（スロット {}・{place}）。",
                lang.quote(&job.name),
                job.settings.width,
                job.settings.height,
                slot_list(&job.slots)
            ),
            format!(
                "Baked {count} map(s) of {} at {}×{} ({kinds}) in {secs:.2} s (slot {}, {place}).",
                lang.quote(&job.name),
                job.settings.width,
                job.settings.height,
                slot_list(&job.slots)
            ),
        );
        // 焼けたが、UV の面積が 0 の三角形・縮退した三角形・UV が重なるテクセルがあれば、1 つの注意にする（数は書かない。数は記録に残る）
        let kind = match uv_warning(lang, &result.report) {
            Some(warning) => {
                text.push(' ');
                text += &warning;
                if !self.bake.uv_notes.contains(&warning) {
                    self.bake.uv_notes.push(warning);
                }
                crate::notice::Kind::Warning
            }
            None => crate::notice::Kind::Info,
        };
        self.bake_ended(kind, text, true);
        if !self.bake.queue.is_empty() {
            // 次のセット（始められなかった理由は `start_next_bake` が知らせる）
            self.start_next_bake();
        } else if self.bake.total > 1 {
            let mut text = lang.pick(
                format!(
                    "{} 個のテクスチャセットのメッシュマップを焼きました。",
                    self.bake.total
                ),
                format!("Baked the mesh maps of {} texture sets.", self.bake.total),
            );
            // どれかのセットに UV の注意があれば、まとめも注意（種類は重いほう）。まとめで、そのセットの注意の文を上書きして消さない
            let mut kind = crate::notice::Kind::Info;
            for warning in &self.bake.uv_notes {
                text.push(' ');
                text += warning;
                kind = kind.worse(crate::notice::Kind::Warning);
            }
            self.bake_ended(kind, text, true);
        }
    }

    fn bake_ended(&mut self, kind: crate::notice::Kind, text: String, ok: bool) {
        self.notify(kind, Source::Bake, text.clone());
        self.bake.outcome = Some((text, ok));
    }

    /// 試験用: 焼いている仕事が終わるまで待って入れる（待ちの上限は 120 秒）。
    #[doc(hidden)]
    pub fn wait_bake(&mut self) {
        crate::jobs::wait_until_idle(
            self,
            "ベイクが終わらない（ハング検出上限）",
            |s| s.bake.job.is_some(),
            Self::poll_bake,
        );
    }
}

#[cfg(test)]
mod tests;
