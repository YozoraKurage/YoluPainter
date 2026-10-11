//! メッシュマップの GPU ベイク（compute。使えるアダプターでは ray query）。
//!
//! 準備（受け手の選別・BVH・曲率・投影・接空間・ID・レイの方向列）と後始末（余白・記録・由来）は `yolu-core::mesh_maps` の
//! `MeshBakePlan` を共有し、行ごとの計算（UV の所有者の判定・投影・各マップの値・レイ）だけを `bake.wgsl` で行う。式の正本は CPU で、
//! 結果は CPU と全バイト一致しない（f32 の丸め・UV の覆いの境の判定・レイの当たり外れ）。マップごとの差は `tests/bake.rs` が測る。
//!
//! 資源の決まり:
//! - 入力は読み取り専用、出力は別のバッファ（同じバッファを読みながら書かない）。
//! - GPU の予算（入力の配列・出力・読み戻し）を超える大きさは、行の帯（タイル）に分けて焼く。1 行も入らなければ理由を返す。
//! - 1 回の dispatch は時間で大きさを合わせ（目標 `target_dispatch_ms`）、TDR（Windows の GPU の応答なし）を避ける。
//! - 読み戻しは帯ごとに完了を待って渡し、失敗・時間切れのあとはこのインスタンスを使わない（作り直す）。
//! - 取消・時間切れ・進捗コールバックの false は dispatch の合間に見て、CPU と同じ空の結果を返す（既存の正本を変えない）。
//!
//! GPU が無い・壊れている・予算を超える・時間切れは `GpuBakeError` で理由を返す。`bake_mesh_maps` は CPU の `bake` に戻る。
use bytemuck::Zeroable;
use pack::{Packed, Params, NONE};
use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
use wgpu::util::DeviceExt;
use yolu_core::mesh_maps::{
    MeshBakeBudget, MeshBakeInput, MeshBakePlan, MeshBakePlanOutcome, MeshBakeRaw, MeshBakeResult,
    MeshBakeSettings, MeshMapError,
};

mod auto;
mod pack;
mod rayquery;
#[cfg(test)]
mod tests;
pub use auto::{bake_mesh_maps, BakeBackend, BakeRun, FallbackKind, GpuBakeSlot};

const COMMON: &str = include_str!("bake.wgsl");
const TRACE_COMPUTE: &str = include_str!("trace_compute.wgsl");
/// 1 つの dispatch の待ちの上限。これを超えたら GPU を使うのをやめる（デバイスが止まっている）。
const WAIT: Duration = Duration::from_secs(30);
/// 1 つのワークグループのスレッド数（`bake.wgsl` の `@workgroup_size`）。
const GROUP: u32 = 64;
/// 1 回の dispatch のテクセル数の上限の最大値（1 次元のワークグループ数の上限 65535 × `GROUP`）。65535 は WebGPU が保証する
/// 値で、デバイスの上限がもっと小さければ `dispatch_limit` がそれに合わせる。2 のべきの `1 << 22` は 65536 グループになり超える。
/// GPU で焼く座標の絶対値の上限。三角形の面積・三重積（座標の 3 乗）が f32 の上限（約 3.4e38）を超えない大きさ（1e9 の 3 乗 = 1e27）。
/// 実際のモデル（メートルでもミリメートルでも）はこれより遥かに小さい。
pub const MAX_GPU_COORDINATE: f64 = 1.0e9;
const MAX_DISPATCH_TEXELS: u32 = 65535 * GROUP;
/// 1 回の dispatch のレイの数の上限（u32 のカウンターが桁あふれしない大きさ）。
const MAX_RAYS_PER_DISPATCH: u64 = 1 << 28;
/// 最初の 1 回の dispatch のレイの数の目安（時間を測る前なので軽く）。
const FIRST_RAYS: u64 = 1 << 20;

/// このデバイスで 1 回の dispatch に入れてよいテクセル数の上限（1 次元のワークグループ数の上限 × `GROUP`、`MAX_DISPATCH_TEXELS` まで）。
fn dispatch_limit(max_workgroups_per_dimension: u32) -> u32 {
    (u64::from(max_workgroups_per_dimension) * u64::from(GROUP))
        .clamp(u64::from(GROUP), u64::from(MAX_DISPATCH_TEXELS)) as u32
}
/// dispatch の大きさが育つ上限: レイの数（u32 のカウンターが桁あふれしない大きさ）と `dispatch_limit` の小さい方。
fn max_chunk(rays_per_texel: u64, dispatch_limit: u32) -> u32 {
    (MAX_RAYS_PER_DISPATCH / rays_per_texel.max(1))
        .clamp(u64::from(GROUP), u64::from(dispatch_limit)) as u32
}

/// GPU ベイクが使えない・続けられない理由。
#[derive(Debug)]
pub enum GpuBakeError {
    /// アダプター・デバイス・シェーダーが使えない（先頭が "GPU 利用不可:"）。
    Unavailable(String),
    /// 予算・デバイスの上限を超える。
    Budget(String),
    /// 実行中の失敗（デバイスの消失・時間切れ・検証エラー・量子化の前の NaN・無限大・範囲外の値）。以後このインスタンスは使えない。
    /// NaN・無限大は GPU が作った場合だけ数える（WGSL は NaN を作らない前提の最適化を許す）。
    Failed(String),
    /// 準備の拒否。CPU の `bake` も同じ理由で断る（設定・UV・予算・手動 ID 色）。CPU へ戻しても同じなので、そのまま見せる。
    Refused(MeshMapError),
}
impl fmt::Display for GpuBakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(s) | Self::Budget(s) | Self::Failed(s) => f.write_str(s),
            Self::Refused(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for GpuBakeError {}
fn failed(e: impl fmt::Display) -> GpuBakeError {
    GpuBakeError::Failed(e.to_string())
}
fn unavailable(e: impl fmt::Display) -> GpuBakeError {
    GpuBakeError::Unavailable(format!("GPU 利用不可: {e}"))
}

/// ベイクに使うアダプターの選び方と資源の上限。
#[derive(Clone, Debug)]
pub struct GpuBakeOptions {
    /// 入力の配列・出力・読み戻しの合計の上限（バイト）。
    pub budget_bytes: u64,
    /// ソフトウェアの描画（llvmpipe・WARP など `DeviceType::Cpu`）を使ってよいか。
    /// 既定は使わない（ソフトウェアの GPU は CPU のマルチスレッドより遅いため、自動のときは CPU で焼く）。
    pub allow_software: bool,
    /// ハードウェアの ray query（RT コア）を使ってよいか。使えるアダプターで、自己照合に通ったときだけ使い、そうでなければ compute に戻る。
    /// 既定は入。環境変数 `YOLUPAINTER_BAKE_RAY_QUERY` が `0`・`false`・`off`・`no` なら既定が切になる（アプリは設定で切り替える）。
    /// 実験機能のドライバーが固まったり応答なし（TDR）になったときは、自己照合が同じ道の中で行われるので compute に戻せない:
    /// そのような PC では切にする。Vulkan（RTX 3050 Laptop）と Metal（Apple M4）で動かして確かめた。DX12 は DXC（新しいシェーダー
    /// コンパイラー）が無いと ray query が出ないので、Windows では Vulkan のアダプターを先に選ぶ。
    pub ray_query: bool,
    /// 1 回の dispatch の時間の目標（ミリ秒）。
    pub target_dispatch_ms: f64,
}
/// ray query の既定を切にする環境変数の名前（`0`・`false`・`off`・`no` で切。未設定・ほかの値は入）。
pub const RAY_QUERY_ENV: &str = "YOLUPAINTER_BAKE_RAY_QUERY";
/// 値が切（`0`・`false`・`off`・`no`）を表すか。環境変数と設定のファイルが同じ読みをする。
pub fn is_off_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no"
    )
}
/// 環境変数の値から ray query の既定を決める。未設定・空・知らない値は入、`0`・`false`・`off`・`no` は切。
fn flag_enabled(value: Option<&str>) -> bool {
    !value.is_some_and(is_off_value)
}
/// 環境変数 `YOLUPAINTER_BAKE_RAY_QUERY` が ray query を許すか。最初に読んだ値をプロセスの間持つ（毎回読み直さない）。
pub fn ray_query_env_allows() -> bool {
    static ALLOWS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ALLOWS.get_or_init(|| flag_enabled(std::env::var(RAY_QUERY_ENV).ok().as_deref()))
}
impl Default for GpuBakeOptions {
    fn default() -> Self {
        Self {
            budget_bytes: 768 << 20,
            allow_software: false,
            ray_query: ray_query_env_allows(),
            target_dispatch_ms: 50.0,
        }
    }
}

/// ray query を使えない理由の種類と詳しい文（使えるなら None）。`supported`: アダプターが機能を持つ、`device_created`: ray query つきで
/// デバイスを作れた、`device_error`: 作れなかったときの理由。
fn ray_query_unavailable(
    supported: bool,
    device_created: bool,
    device_error: Option<&str>,
) -> Option<(RayQueryWhy, String)> {
    if !supported {
        return Some((
            RayQueryWhy::NotSupported,
            "compute（ray query を使わない理由: このアダプターは ray query に対応していない）"
                .into(),
        ));
    }
    if !device_created {
        let why = device_error.unwrap_or("理由は不明");
        return Some((
            RayQueryWhy::Device,
            format!(
                "compute（ray query を使わない理由: ray query つきのデバイスを作れなかった: {why}）"
            ),
        ));
    }
    None
}

/// 使っているアダプターの情報。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeAdapter {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    /// ソフトウェアの描画（CPU で動く）。
    pub software: bool,
    /// ハードウェアの ray query が使える（ベイクで使ったかは `GpuBakeStats::method`）。
    pub ray_query: bool,
}
impl fmt::Display for BakeAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}（{}、{}）", self.name, self.backend, self.device_type)
    }
}

/// レイをたどる道。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuBakeMethod {
    /// BVH を compute でたどる（どの GPU でも動く）。
    Compute,
    /// ハードウェアの ray query。
    RayQuery,
}

/// ray query を使わなかった理由の種類。画面は種類から言語ごとの短い文を作る（自己照合の数などの詳しい文は
/// `GpuBakeStats::ray_query_note` とログだけに残す）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RayQueryWhy {
    /// `GpuBakeOptions::ray_query` が切（設定か環境変数。どちらかは呼び出し側が知る）。
    Disabled,
    /// アダプターが ray query に対応していない。
    NotSupported,
    /// この焼きは ray query の対象外（名前の対応・三角形が多すぎる・予算）。
    NotApplicable,
    /// ray query つきのデバイスを作れなかった（compute のデバイスで作り直した）。
    Device,
    /// ray query のシェーダー・パイプラインを作れない。
    Shader,
    /// 加速構造を作れない。
    Accel,
    /// 自己照合を回せない。
    CheckRun,
    /// 自己照合に通らない。
    CheckFailed,
    /// 前の焼きが ray query で途中で失敗したので、compute に切り替えた。
    RunFailed,
}

#[derive(Clone, Debug)]
pub struct GpuBakeStats {
    pub method: GpuBakeMethod,
    pub dispatches: u32,
    pub max_dispatch_ms: f64,
    /// 1 回の dispatch で焼いたテクセルの最大数（デバイスの 1 次元のワークグループ数の上限 × 64 を超えない）。
    pub max_dispatch_texels: u32,
    /// 帯（出力をまとめて読み戻す行の組）の数。
    pub bands: u32,
    /// GPU に置いた入力の配列の大きさ（バイト）。
    pub input_bytes: u64,
    /// 出力と読み戻しの 1 帯の大きさ（バイト）。
    pub band_bytes: u64,
    /// ray query の道について、使った説明（自己照合の結果）か、使わなかった詳しい理由（数を含む。ログ・試験用で、画面には出さない）。
    pub ray_query_note: Option<String>,
    /// ray query を使わなかった理由の種類（使ったとき・compute を選んでいて ray query に関わらないときは None）。
    pub ray_query_why: Option<RayQueryWhy>,
}

pub struct GpuBaked {
    pub result: MeshBakeResult,
    pub stats: GpuBakeStats,
}

struct Compiled {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    selfcheck: wgpu::ComputePipeline,
}

/// GPU でメッシュマップを焼く。作るのに時間がかかる（デバイスとシェーダー）ので、続けて使い回す。
pub struct BakeGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: BakeAdapter,
    options: GpuBakeOptions,
    compute: Compiled,
    errors: Arc<Mutex<Vec<String>>>,
    failed: Option<String>,
    /// ray query の機能つきでデバイスを作れたか。
    ray_query: bool,
    /// ray query つきでデバイスを作ろうとして失敗した理由（compute のデバイスに戻った）。
    ray_query_device_error: Option<String>,
    /// アダプターが ray query の機能を持つか（デバイスを ray query つきで作れたかとは別。作れなかった理由は `ray_query_device_error`）。
    adapter_supports_ray_query: bool,
    /// 前の焼きが ray query で途中で失敗したので、ray query を止めている理由（`GpuBakeSlot` が作り直すときに渡す）。
    ray_query_blocked: Option<String>,
    /// デバイスが失われた（ドライバーの固まり・応答なし）。
    lost: Arc<AtomicBool>,
    /// GPU の完了を待つ・読み戻す所で失敗した（時間切れ・割り付けの失敗。固まりの疑い）。
    hung: bool,
    /// 今の `bake` が ray query の道で焼いている（失敗したとき、compute でやり直せるかの判断に使う）。
    ray_query_running: bool,
    /// ray query のシェーダー（初めて使うときに作る。作れなければ理由）。
    rq: Option<Result<Compiled, String>>,
    /// 試験用: 次の `bake` の、レイをたどる道を決めたあとの途中で、この理由の失敗にする（`MidFailure` はそのとき ray query の道で焼いていた
    /// ことにするか、固まり・応答なしの失敗にするか）。
    inject_mid: Option<(String, MidFailure)>,
    /// 試験用: 次の `bake` の途中で、この理由の失敗にする。
    inject: Option<String>,
    /// 試験用: `bake` のたびに compute の道で自己照合のレイを飛ばして、結果を `selfcheck_probe` に置く。
    probe_selfcheck: bool,
    selfcheck_probe: Option<Vec<u32>>,
}

fn adapter_info(info: &wgpu::AdapterInfo, ray_query: bool) -> BakeAdapter {
    BakeAdapter {
        name: info.name.clone(),
        backend: format!("{:?}", info.backend),
        device_type: match info.device_type {
            wgpu::DeviceType::DiscreteGpu => "ディスクリート GPU",
            wgpu::DeviceType::IntegratedGpu => "内蔵 GPU",
            wgpu::DeviceType::VirtualGpu => "仮想 GPU",
            wgpu::DeviceType::Cpu => "ソフトウェア",
            wgpu::DeviceType::Other => "その他",
        }
        .into(),
        software: info.device_type == wgpu::DeviceType::Cpu,
        ray_query,
    }
}

/// compute の束縛（`bake.wgsl` の @binding と同じ並び）。
fn layout_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    (0..12)
        .map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: match binding {
                    0 => wgpu::BufferBindingType::Uniform,
                    9..=11 => wgpu::BufferBindingType::Storage { read_only: false },
                    _ => wgpu::BufferBindingType::Storage { read_only: true },
                },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        })
        .collect()
}

/// アダプターの優先順位: ハードウェアを先に（ディスクリート > 内蔵 > 仮想 > その他 > ソフトウェア）、同じなら Vulkan・DX12・Metal・GL の順。
fn rank(info: &wgpu::AdapterInfo) -> (u8, u8) {
    (
        match info.device_type {
            wgpu::DeviceType::DiscreteGpu => 0,
            wgpu::DeviceType::IntegratedGpu => 1,
            wgpu::DeviceType::VirtualGpu => 2,
            wgpu::DeviceType::Other => 3,
            wgpu::DeviceType::Cpu => 4,
        },
        match info.backend {
            wgpu::Backend::Vulkan => 0,
            wgpu::Backend::Dx12 => 1,
            wgpu::Backend::Metal => 2,
            _ => 3,
        },
    )
}

/// 試験用の途中の失敗の種類（`BakeGpu::fail_midway_for_test`）。
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidFailure {
    /// compute の道で焼いている間の失敗。
    Compute,
    /// ray query の道で焼いている間の、固まりではない失敗（NaN・範囲外の値など）。
    RayQuery,
    /// ray query の道で焼いている間の、完了待ちの時間切れ（固まりの疑い）。
    RayQueryHung,
}

/// 準備の間の取消・時間切れの確認で通知する進み具合（`MeshBakePlan` の準備が終わった所）。
const PREPARE_FRACTION: f64 = 0.05;
/// ray query の準備ができたときの (pipeline, 束縛, 説明, 追加の旗)。
type RqReady = (wgpu::ComputePipeline, wgpu::BindGroup, String, u32);
/// ray query の準備が compute に戻る・止まる理由。
enum RqStop {
    /// compute に戻る（理由の種類と詳しい文）。
    Why(RayQueryWhy, String),
    /// 取消・時間切れ・進捗の中止（空の結果。焼きはここで終わる）。
    Stopped(Box<MeshBakeResult>),
}
/// 準備の区切りで取消・時間切れを見る。
fn stop_check(
    plan: &mut MeshBakePlan<'_>,
    progress: &mut dyn FnMut(f64, &str) -> bool,
) -> Result<(), RqStop> {
    match plan.checkpoint(PREPARE_FRACTION, "Preparing", progress) {
        Some(result) => Err(RqStop::Stopped(Box::new(result))),
        None => Ok(()),
    }
}

/// ray query つきのデバイスの作成・準備（`rq`。None は試さない）が失敗したら、その理由を持って ray query なしの作り方（`plain`）に
/// 戻る。ray query の失敗で CPU に落とさない。
async fn with_ray_query_fallback<T, F, G>(
    rq: Option<F>,
    plain: impl FnOnce(Option<String>) -> G,
) -> Result<T, GpuBakeError>
where
    F: std::future::Future<Output = Result<T, String>>,
    G: std::future::Future<Output = Result<T, GpuBakeError>>,
{
    let device_error = match rq {
        Some(attempt) => match attempt.await {
            Ok(done) => return Ok(done),
            Err(e) => Some(e),
        },
        None => None,
    };
    plain(device_error).await
}

impl BakeGpu {
    /// 使えるアダプターを選んで準備する。使えなければ理由（先頭が "GPU 利用不可:"）。
    pub fn new(options: GpuBakeOptions) -> Result<Self, GpuBakeError> {
        pollster::block_on(Self::create(options))
    }
    async fn create(options: GpuBakeOptions) -> Result<Self, GpuBakeError> {
        // Instance はデバイスごとに作る。プロセスで共有すると、GL（EGL）では別のデバイスが 1 つの GL のコンテキストを分け合い、
        // 別のスレッドから同時に使うと make_current が失敗して落ちる（ベイクの裏のスレッドとキャンバスの合成が同時に動く）
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let mut adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let found = adapters.len();
        adapters.retain(|a| {
            let info = a.get_info();
            let l = a.limits();
            (options.allow_software || info.device_type != wgpu::DeviceType::Cpu)
                && a.get_downlevel_capabilities()
                    .flags
                    .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
                && l.max_storage_buffers_per_shader_stage >= 11
                && l.max_compute_invocations_per_workgroup >= GROUP
        });
        adapters.sort_by_key(|a| {
            let info = a.get_info();
            let rq = options.ray_query
                && a.features()
                    .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
            (rank(&info).0, u8::from(!rq), rank(&info).1)
        });
        let Some(adapter) = adapters.into_iter().next() else {
            return Err(unavailable(if found == 0 {
                "アダプターがありません".to_string()
            } else if options.allow_software {
                "compute シェーダーを使えるアダプターがありません".to_string()
            } else {
                format!("使えるハードウェアの GPU がありません（アダプター {found} 個はソフトウェアか compute 不可）")
            }));
        };
        let info = adapter.get_info();
        let adapter_supports_ray_query = adapter
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
        let request = |ray_query: bool| wgpu::DeviceDescriptor {
            label: Some("メッシュマップのベイク"),
            required_features: if ray_query {
                wgpu::Features::EXPERIMENTAL_RAY_QUERY
            } else {
                wgpu::Features::empty()
            },
            required_limits: adapter.limits(),
            experimental_features: if ray_query {
                // SAFETY: 実験機能（ray query）を使うことの了承。使うのは加速構造の構築とシェーダーの ray query だけで、
                // 自己照合に通らなければ使わない。
                unsafe { wgpu::ExperimentalFeatures::enabled() }
            } else {
                wgpu::ExperimentalFeatures::disabled()
            },
            ..Default::default()
        };
        let want_rq = options.ray_query && adapter_supports_ray_query;
        let rq_attempt = async {
            let (device, queue) = adapter
                .request_device(&request(true))
                .await
                .map_err(|e| e.to_string())?;
            // デバイスは作れても、その後の準備（compute のシェーダー・最初の確認）で失敗する GPU がある
            Self::assemble(
                device,
                queue,
                &info,
                options.clone(),
                true,
                adapter_supports_ray_query,
                None,
            )
            .await
            .map_err(|e| e.to_string())
        };
        with_ray_query_fallback(want_rq.then_some(rq_attempt), |device_error| async {
            let (device, queue) = adapter
                .request_device(&request(false))
                .await
                .map_err(unavailable)?;
            Self::assemble(
                device,
                queue,
                &info,
                options.clone(),
                false,
                adapter_supports_ray_query,
                device_error,
            )
            .await
        })
        .await
    }
    /// 作ったデバイスに、エラーの受け口・compute のシェーダーを用意して、使える形にする。
    async fn assemble(
        device: wgpu::Device,
        queue: wgpu::Queue,
        info: &wgpu::AdapterInfo,
        options: GpuBakeOptions,
        ray_query: bool,
        adapter_supports_ray_query: bool,
        ray_query_device_error: Option<String>,
    ) -> Result<Self, GpuBakeError> {
        let errors = Arc::new(Mutex::new(Vec::new()));
        let sink = errors.clone();
        // 既定の処理はパニックなので、捕まえられなかった誤りは覚えておいて、次の確認で失敗として返す。
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            if let Ok(mut v) = sink.lock() {
                v.push(e.to_string());
            }
        }));
        let sink = errors.clone();
        let lost = Arc::new(AtomicBool::new(false));
        let lost_flag = lost.clone();
        device.set_device_lost_callback(move |reason, message| {
            lost_flag.store(true, Ordering::Relaxed);
            if let Ok(mut v) = sink.lock() {
                v.push(format!("デバイスが失われました（{reason:?}）: {message}"));
            }
        });
        let mut this = Self {
            adapter: adapter_info(info, ray_query),
            compute: Self::compile(
                &device,
                &format!("{COMMON}\n{TRACE_COMPUTE}"),
                &layout_entries(),
            )
            .await?,
            device,
            queue,
            options,
            errors,
            failed: None,
            ray_query,
            ray_query_device_error,
            adapter_supports_ray_query,
            ray_query_blocked: None,
            lost,
            hung: false,
            ray_query_running: false,
            rq: None,
            inject: None,
            inject_mid: None,
            probe_selfcheck: false,
            selfcheck_probe: None,
        };
        this.check_errors()?;
        Ok(this)
    }
    async fn compile(
        device: &wgpu::Device,
        source: &str,
        entries: &[wgpu::BindGroupLayoutEntry],
    ) -> Result<Compiled, GpuBakeError> {
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ベイクの束縛"),
            entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // DXC（DX12 の新しいシェーダーコンパイラー）はラベルをファイル名として渡されるので、日本語だと読めずに失敗する。ASCII にする
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh_map_bake"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline_for = |entry: &'static str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let pipeline = pipeline_for("bake");
        let selfcheck = pipeline_for("selfcheck");
        let mut problem = None;
        for scope in [internal, validation] {
            if let Some(e) = scope.pop().await {
                problem = Some(e.to_string());
            }
        }
        match problem {
            Some(e) => Err(unavailable(format!("ベイクのシェーダーを作れません: {e}"))),
            None => Ok(Compiled {
                layout,
                pipeline,
                selfcheck,
            }),
        }
    }
    pub fn adapter(&self) -> &BakeAdapter {
        &self.adapter
    }
    /// このインスタンスを作ったときの `GpuBakeOptions::ray_query`（デバイスの作り方が変わるので、後から切り替えられない）。
    pub fn ray_query_option(&self) -> bool {
        self.options.ray_query
    }
    /// ray query を止めて作ったときの理由を覚える（`GpuBakeSlot` が、前の焼きの失敗のあと compute だけで作り直すときに渡す）。
    pub(crate) fn block_ray_query(&mut self, reason: &str) {
        self.ray_query_blocked = Some(reason.to_owned());
    }
    /// 直前の `bake` の失敗が、ray query の道を使っていて、デバイスの消失でも待ち時間切れでもないか（compute でやり直せば焼けるかもしれない
    /// 失敗。固まり・応答なしのドライバーは、同じ GPU を使わず CPU に戻す）。
    pub(crate) fn failed_on_ray_query_only(&self) -> bool {
        self.ray_query_running && !self.lost.load(Ordering::Relaxed) && !self.hung
    }
    /// このデバイスで 1 回の dispatch に使えるワークグループ数（1 次元）の上限。
    pub fn max_workgroups_per_dimension(&self) -> u32 {
        self.device.limits().max_compute_workgroups_per_dimension
    }
    /// 失敗して使えなくなっていれば、その理由。
    pub fn failure(&self) -> Option<&str> {
        self.failed.as_deref()
    }
    /// 試験用: 次の `bake` を、準備のあとの途中で `Failed` にする（そのあとこのインスタンスは壊れた状態になり、
    /// `GpuBakeSlot` は次の呼び出しで作り直す）。
    #[doc(hidden)]
    pub fn fail_for_test(&mut self, reason: &str) {
        self.inject = Some(reason.to_owned());
    }
    /// 試験用: 次の `bake` を、レイをたどる道を決めたあとに `Failed` にする。
    #[doc(hidden)]
    pub fn fail_midway_for_test(&mut self, reason: &str, kind: MidFailure) {
        self.inject_mid = Some((reason.to_owned(), kind));
    }
    /// 試験用: 以後の `bake` で、compute の道の自己照合のレイ（ray query の道が合わせるべき答え）も飛ばして覚える。
    /// ray query の使えない環境でも、この答えの形（並び・値の範囲）は確かめられる。
    #[doc(hidden)]
    pub fn probe_selfcheck_for_test(&mut self) {
        self.probe_selfcheck = true;
    }
    /// 試験用: 最後の `bake` で飛ばした自己照合の答え（レイごとに 8 ワード）。
    #[doc(hidden)]
    pub fn selfcheck_for_test(&self) -> Option<&[u32]> {
        self.selfcheck_probe.as_deref()
    }
    fn check_errors(&mut self) -> Result<(), GpuBakeError> {
        let found: Vec<String> = self
            .errors
            .lock()
            .map(|mut v| std::mem::take(&mut *v))
            .unwrap_or_default();
        if found.is_empty() {
            return Ok(());
        }
        let reason = found.join(" / ");
        self.failed = Some(reason.clone());
        Err(GpuBakeError::Failed(reason))
    }

    /// GPU で焼く。取消・時間切れ・進捗の false は `Ok` で空の結果（既存の正本を変えない）。進捗は CPU の `bake` と同じ
    /// "Preparing" → "Baking" → "Padding" → "Done"。GPU のエラーは `Err`（このインスタンスは使えなくなる）。
    pub fn bake(
        &mut self,
        input: &MeshBakeInput,
        settings: &MeshBakeSettings,
        budget: &MeshBakeBudget,
        cancel: Option<&AtomicBool>,
        reference: Option<&MeshBakeInput>,
        mut progress: impl FnMut(f64, &str) -> bool,
    ) -> Result<GpuBaked, GpuBakeError> {
        if let Some(reason) = &self.failed {
            return Err(GpuBakeError::Failed(format!(
                "GPU は作り直しが必要です: {reason}"
            )));
        }
        self.ray_query_running = false;
        // f32 で計算する GPU は、座標が大きすぎると積が桁あふれする。NaN・無限大の検出は GPU が作った場合だけで（WGSL は作らない
        // 前提の最適化を許す）、コンテナの実 GPU では黙って誤った値になった。焼く前に座標の大きさで決め打ちに断り、CPU（f64）に任せる
        for m in std::iter::once(input).chain(reference) {
            let (min, max) = m.bounds();
            if min
                .iter()
                .chain(&max)
                .any(|v| !v.is_finite() || v.abs() > MAX_GPU_COORDINATE)
            {
                return Err(GpuBakeError::Failed(format!(
                    "座標が大きすぎて GPU の f32 では焼けません（絶対値 {MAX_GPU_COORDINATE:e} まで）。NaN・無限大を避けて CPU で焼きます"
                )));
            }
        }
        let progress: &mut dyn FnMut(f64, &str) -> bool = &mut progress;
        let plan = match MeshBakePlan::prepare(input, settings, budget, cancel, reference, progress)
            .map_err(GpuBakeError::Refused)?
        {
            MeshBakePlanOutcome::Ready(plan) => plan,
            MeshBakePlanOutcome::Stopped(result) => {
                return Ok(GpuBaked {
                    result,
                    stats: GpuBakeStats::none(),
                })
            }
        };
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let result = self.run(plan, progress);
        let mut scope_error = None;
        for scope in [internal, memory, validation] {
            if let Some(e) = pollster::block_on(scope.pop()) {
                scope_error = Some(e.to_string());
            }
        }
        let result = match (result, scope_error) {
            (Ok(r), None) => r,
            (Err(GpuBakeError::Refused(e)), _) => return Err(GpuBakeError::Refused(e)),
            (Err(GpuBakeError::Budget(e)), None) => return Err(GpuBakeError::Budget(e)),
            (Err(e), scope) => {
                let reason = scope.map_or_else(|| e.to_string(), |s| format!("{e} / {s}"));
                self.failed = Some(reason.clone());
                return Err(GpuBakeError::Failed(reason));
            }
            (Ok(_), Some(s)) => {
                self.failed = Some(s.clone());
                return Err(GpuBakeError::Failed(s));
            }
        };
        self.check_errors()?;
        Ok(result)
    }

    fn run(
        &mut self,
        mut plan: MeshBakePlan<'_>,
        progress: &mut dyn FnMut(f64, &str) -> bool,
    ) -> Result<GpuBaked, GpuBakeError> {
        if let Some(reason) = self.inject.take() {
            return Err(GpuBakeError::Failed(reason));
        }
        let settings = plan.settings().clone();
        let packed = {
            let scene = plan.scene();
            Packed::new(&scene, self.options.budget_bytes)?
        };
        let limits = self.device.limits();
        let (width, height) = (settings.width as u64, settings.height as u64);
        let texels = width * height;
        let out_stride = u64::from(packed.params.out_stride);
        let input_bytes = packed.bytes();
        if packed.largest_binding() > limits.max_storage_buffer_binding_size
            || packed.largest_binding() > limits.max_buffer_size
        {
            return Err(GpuBakeError::Budget(format!(
                "入力の配列（{} MiB）がこの GPU の 1 つのバッファの上限を超えます",
                packed.largest_binding() >> 20
            )));
        }
        if input_bytes >= self.options.budget_bytes {
            return Err(GpuBakeError::Budget(format!(
                "入力の配列が GPU の予算を超えます（{} MiB、予算 {} MiB）",
                input_bytes >> 20,
                self.options.budget_bytes >> 20
            )));
        }
        // 出力（texels × (out_stride + 1) ワード）と読み戻しの同じ大きさ。1 つのバッファの上限とも比べる。
        let bytes_per_texel = (out_stride + 1) * 4;
        let rows_by_budget =
            (self.options.budget_bytes - input_bytes) / (bytes_per_texel * 2 * width);
        let rows_by_limit = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size)
            .min(1 << 31)
            / (bytes_per_texel * width);
        let rows = rows_by_budget.min(rows_by_limit).min(height);
        if rows == 0 {
            return Err(GpuBakeError::Budget(format!(
                "出力の 1 行（{} MiB）が GPU の予算に入りません",
                (bytes_per_texel * 2 * width) >> 20
            )));
        }
        let band_texels_max = rows * width;
        let band_bytes = band_texels_max * bytes_per_texel;

        let device = &self.device;
        let init = |label: &str, data: &[u8], usage: wgpu::BufferUsages| {
            let pad = [0u8; 16];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: if data.is_empty() { &pad } else { data },
                usage,
            })
        };
        let storage = wgpu::BufferUsages::STORAGE;
        let lo_f = init("lo_f", bytemuck::cast_slice(&packed.lo_f), storage);
        let lo_u = init("lo_u", bytemuck::cast_slice(&packed.lo_u), storage);
        let hi_f = init("hi_f", bytemuck::cast_slice(&packed.hi_f), storage);
        let hi_u = init("hi_u", bytemuck::cast_slice(&packed.hi_u), storage);
        let idx = init("idx", bytemuck::cast_slice(&packed.idx), storage);
        let bvh_nodes = init(
            "bvh_nodes",
            bytemuck::cast_slice(&packed.bvh_nodes),
            storage,
        );
        let bvh_tris = init("bvh_tris", bytemuck::cast_slice(&packed.bvh_tris), storage);
        let misc = init("misc", bytemuck::cast_slice(&packed.misc), storage);
        let params_buffer = init(
            "params",
            bytemuck::bytes_of(&packed.params),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let make = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        // 自己照合のレイの結果も入る大きさにする
        let out_size = (band_texels_max * out_stride * 4).max(rayquery::CHECK_BYTES);
        let out = make("out", out_size, storage | wgpu::BufferUsages::COPY_SRC);
        let cov = make(
            "cov",
            band_texels_max * 4,
            storage | wgpu::BufferUsages::COPY_SRC,
        );
        let counters = make(
            "counters",
            16,
            storage | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        );
        let readback = wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST;
        let out_rb = make("out_rb", out_size, readback);
        let cov_rb = make("cov_rb", band_texels_max * 4, readback);
        let counters_rb = make("counters_rb", 16, readback);
        let buffers = [
            &params_buffer,
            &lo_f,
            &lo_u,
            &hi_f,
            &hi_u,
            &idx,
            &bvh_nodes,
            &bvh_tris,
            &misc,
            &out,
            &cov,
            &counters,
        ];
        let entries: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ベイク"),
            layout: &self.compute.layout,
            entries: &entries,
        });

        if self.probe_selfcheck {
            let mut probe = packed.params;
            probe.local_start = 0;
            probe.count = rayquery::CHECK_RAYS;
            self.queue
                .write_buffer(&params_buffer, 0, bytemuck::bytes_of(&probe));
            let selfcheck = self.compute.selfcheck.clone();
            let answers = self.run_selfcheck(&selfcheck, &group, &params_buffer, &out_rb, &out)?;
            self.selfcheck_probe = Some(answers);
        }
        // レイをたどる道: ハードウェアの ray query が使えて自己照合に通れば ray query、そうでなければ compute。
        let mut method = GpuBakeMethod::Compute;
        let ray_query_note: Option<String>;
        let ray_query_why: Option<RayQueryWhy>;
        let mut pipeline = self.compute.pipeline.clone();
        let mut group = group;
        let mut rq_flags = 0u32;
        if let Some(blocked) = &self.ray_query_blocked {
            ray_query_why = Some(RayQueryWhy::RunFailed);
            ray_query_note = Some(format!(
                "compute（ray query を使わない理由: 前の焼きが ray query の途中で失敗したので止めた: {blocked}）"
            ));
        } else if !self.options.ray_query {
            ray_query_why = Some(RayQueryWhy::Disabled);
            ray_query_note = Some("compute（ray query は切）".to_string());
        } else if let Some((why, note)) = ray_query_unavailable(
            self.adapter_supports_ray_query,
            self.ray_query,
            self.ray_query_device_error.as_deref(),
        ) {
            ray_query_why = Some(why);
            ray_query_note = Some(note);
        } else {
            let spare = self
                .options
                .budget_bytes
                .saturating_sub(input_bytes + band_bytes);
            match self.try_ray_query(
                &packed,
                spare,
                &buffers,
                &group,
                &params_buffer,
                &out,
                &out_rb,
                &mut plan,
                progress,
            ) {
                Ok((rq_pipeline, rq_group, summary, flags)) => {
                    method = GpuBakeMethod::RayQuery;
                    pipeline = rq_pipeline;
                    group = rq_group;
                    rq_flags = flags;
                    ray_query_note = Some(summary);
                    ray_query_why = None;
                    self.ray_query_running = true;
                }
                Err(RqStop::Why(why, detail)) => {
                    ray_query_why = Some(why);
                    ray_query_note = Some(format!("compute（ray query を使わない理由: {detail}）"));
                }
                Err(RqStop::Stopped(result)) => {
                    return Ok(GpuBaked {
                        result: *result,
                        stats: GpuBakeStats {
                            input_bytes,
                            band_bytes,
                            ..GpuBakeStats::none()
                        },
                    })
                }
            }
        }
        if let Some((reason, kind)) = self.inject_mid.take() {
            self.ray_query_running = kind != MidFailure::Compute;
            self.hung = kind == MidFailure::RayQueryHung;
            return Err(failed(reason));
        }
        let mut raw = MeshBakeRaw {
            outputs: settings
                .maps
                .iter()
                .map(|k| vec![0u16; texels as usize * k.channels()])
                .collect(),
            coverage: vec![0u8; texels as usize],
            rays: 0,
            projected: 0,
            missed: 0,
        };
        let mut stats = GpuBakeStats {
            method,
            dispatches: 0,
            max_dispatch_ms: 0.0,
            max_dispatch_texels: 0,
            bands: 0,
            input_bytes,
            band_bytes,
            ray_query_note,
            ray_query_why,
        };
        let rays_per_texel = packed.rays_per_texel;
        let chunk_cap = max_chunk(
            rays_per_texel,
            dispatch_limit(limits.max_compute_workgroups_per_dimension),
        );
        let mut chunk = ((FIRST_RAYS / rays_per_texel).max(u64::from(GROUP)))
            .min(u64::from(chunk_cap))
            .min(256.max(u64::from(GROUP))) as u32;
        let mut params = packed.params;
        params.flags |= rq_flags;
        let total_rows = height as usize;
        if let Some(stopped) = plan.checkpoint(0.05, "Baking", progress) {
            return Ok(GpuBaked {
                result: stopped,
                stats,
            });
        }
        let mut row = 0usize;
        while row < total_rows {
            let band_rows = (rows as usize).min(total_rows - row);
            let band_texels = band_rows as u32 * width as u32;
            params.band_first = (row as u32) * width as u32;
            params.band_texels = band_texels;
            stats.bands += 1;
            let mut local = 0u32;
            while local < band_texels {
                let n = chunk.min(band_texels - local);
                params.local_start = local;
                params.count = n;
                self.queue
                    .write_buffer(&params_buffer, 0, bytemuck::bytes_of(&params));
                self.queue.write_buffer(&counters, 0, &[0u8; 16]);
                let mut encoder = self.device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    pass.dispatch_workgroups(n.div_ceil(GROUP), 1, 1);
                }
                encoder.copy_buffer_to_buffer(&counters, 0, &counters_rb, 0, 16);
                let started = Instant::now();
                let submission = self.queue.submit([encoder.finish()]);
                let counts = self.read(&counters_rb, 16, submission)?;
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                stats.dispatches += 1;
                stats.max_dispatch_ms = stats.max_dispatch_ms.max(elapsed);
                stats.max_dispatch_texels = stats.max_dispatch_texels.max(n);
                let c: &[u32] = bytemuck::cast_slice(&counts);
                raw.rays += u64::from(c[0]);
                raw.projected += u64::from(c[1]);
                raw.missed += u64::from(c[2]);
                if c[3] > 0 {
                    // 量子化の前に数えた NaN・無限大。量子化で黙って 0 や端の値になる前に、この結果を使わず CPU に戻す。
                    return Err(failed(format!(
                        "GPU が NaN または無限大の値を {} 個作りました（帯の先頭の行 {}）",
                        c[3], row
                    )));
                }
                local += n;
                // 次の大きさを時間で合わせる（満杯で回して余裕があれば倍、目標を超えたら半分）。
                let target = self.options.target_dispatch_ms;
                if elapsed > target {
                    chunk = (chunk / 2).max(GROUP);
                } else if elapsed < target / 2.0 && n == chunk {
                    chunk = chunk.saturating_mul(2).min(chunk_cap);
                }
                let done = (row as f64 * width as f64 + f64::from(local)) / texels as f64;
                if let Some(stopped) = plan.checkpoint(0.05 + 0.85 * done, "Baking", progress) {
                    return Ok(GpuBaked {
                        result: stopped,
                        stats,
                    });
                }
            }
            // 帯の出力を読み戻して、正本の形（u16・行優先）に並べる。
            let mut encoder = self.device.create_command_encoder(&Default::default());
            let out_bytes = u64::from(band_texels) * out_stride * 4;
            let cov_bytes = u64::from(band_texels) * 4;
            encoder.copy_buffer_to_buffer(&out, 0, &out_rb, 0, out_bytes);
            encoder.copy_buffer_to_buffer(&cov, 0, &cov_rb, 0, cov_bytes);
            let submission = self.queue.submit([encoder.finish()]);
            let out_bytes_read = self.read(&out_rb, out_bytes, submission.clone())?;
            let cov_bytes_read = self.read(&cov_rb, cov_bytes, submission)?;
            let words: &[u32] = bytemuck::cast_slice(&out_bytes_read);
            let cov_words: &[u32] = bytemuck::cast_slice(&cov_bytes_read);
            let first = row * width as usize;
            let mut offset = 0usize;
            // 16 bit・覆い 0〜2 に収まらない値は、u16・u8 に切る前の u32 のうちに見つける（切ると 256 以上が黙って通る）。
            let mut out_of_range = false;
            for (m, kind) in settings.maps.iter().enumerate() {
                let ch = kind.channels();
                let dst = &mut raw.outputs[m][first * ch..(first + band_texels as usize) * ch];
                for (t, texel) in dst.chunks_exact_mut(ch).enumerate() {
                    for (c, v) in texel.iter_mut().enumerate() {
                        let word = words[t * out_stride as usize + offset + c];
                        out_of_range |= word > u32::from(u16::MAX);
                        *v = word as u16;
                    }
                }
                offset += ch;
            }
            for (v, w) in raw.coverage[first..first + band_texels as usize]
                .iter_mut()
                .zip(cov_words)
            {
                out_of_range |= *w > 2;
                *v = *w as u8;
            }
            if out_of_range {
                return Err(failed(
                    "GPU が範囲外の値（16 bit を超える画素、または 0〜2 でない覆い）を返しました",
                ));
            }
            row += band_rows;
        }
        let result = plan.finish(raw, progress).map_err(GpuBakeError::Refused)?;
        Ok(GpuBaked { result, stats })
    }

    /// 自己照合のレイを飛ばして、結果（レイごとに `rayquery::CHECK_STRIDE` ワード）を返す。
    fn run_selfcheck(
        &mut self,
        pipeline: &wgpu::ComputePipeline,
        group: &wgpu::BindGroup,
        params_buffer: &wgpu::Buffer,
        out_rb: &wgpu::Buffer,
        out: &wgpu::Buffer,
    ) -> Result<Vec<u32>, GpuBakeError> {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(rayquery::CHECK_RAYS.div_ceil(GROUP), 1, 1);
        }
        let bytes = rayquery::CHECK_BYTES;
        encoder.copy_buffer_to_buffer(out, 0, out_rb, 0, bytes);
        let submission = self.queue.submit([encoder.finish()]);
        let data = self.read(out_rb, bytes, submission)?;
        let _ = params_buffer;
        Ok(bytemuck::cast_slice::<u8, u32>(&data).to_vec())
    }

    /// ray query の道を用意する。加速構造を作り、同じレイを compute の道と ray query の道で飛ばして答え（当たったか・距離・三角形・
    /// 重心座標）を比べ、合えば ray query の (pipeline, 束縛, 説明, 追加の旗) を返す。作れない・合わないなら理由の種類と詳しい文
    /// （compute に戻る）。準備の区切りごとに取消・時間切れを見て、止めるなら `RqStop::Stopped`。ray query に固有の失敗（シェーダー・
    /// 加速構造の検証、メモリ）は、この中のエラーの受け口で受け止める。ただし完了待ち・読み戻しの失敗（時間切れ）はそこで `failed` が
    /// 立つので、その GPU は次の焼きで作り直される。
    #[allow(clippy::too_many_arguments)]
    fn try_ray_query(
        &mut self,
        packed: &Packed,
        spare: u64,
        buffers: &[&wgpu::Buffer; 12],
        compute_group: &wgpu::BindGroup,
        params_buffer: &wgpu::Buffer,
        out: &wgpu::Buffer,
        out_rb: &wgpu::Buffer,
        plan: &mut MeshBakePlan<'_>,
        progress: &mut dyn FnMut(f64, &str) -> bool,
    ) -> Result<RqReady, RqStop> {
        let limits = self.device.limits();
        rayquery::applicable(packed, &limits, spare)
            .map_err(|e| RqStop::Why(RayQueryWhy::NotApplicable, e))?;
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let result = self.try_ray_query_inner(
            packed,
            buffers,
            compute_group,
            params_buffer,
            out,
            out_rb,
            plan,
            progress,
        );
        let mut scope_error = None;
        for scope in [internal, memory, validation] {
            if let Some(e) = pollster::block_on(scope.pop()) {
                scope_error = Some(e.to_string());
            }
        }
        match (result, scope_error) {
            (Ok(r), None) => Ok(r),
            (Ok(_), Some(e)) => Err(RqStop::Why(RayQueryWhy::Accel, e)),
            (Err(RqStop::Why(why, e)), Some(s)) => Err(RqStop::Why(why, format!("{e} / {s}"))),
            (Err(e), _) => Err(e),
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn try_ray_query_inner(
        &mut self,
        packed: &Packed,
        buffers: &[&wgpu::Buffer; 12],
        compute_group: &wgpu::BindGroup,
        params_buffer: &wgpu::Buffer,
        out: &wgpu::Buffer,
        out_rb: &wgpu::Buffer,
        plan: &mut MeshBakePlan<'_>,
        progress: &mut dyn FnMut(f64, &str) -> bool,
    ) -> Result<RqReady, RqStop> {
        stop_check(plan, progress)?;
        if self.rq.is_none() {
            self.rq = Some(
                pollster::block_on(Self::compile(
                    &self.device,
                    &rayquery::source(COMMON),
                    &rayquery::layout_entries(),
                ))
                .map_err(|e| e.to_string()),
            );
        }
        let (rq_layout, rq_pipeline, rq_selfcheck) = match self.rq.as_ref().expect("作った") {
            Ok(c) => (c.layout.clone(), c.pipeline.clone(), c.selfcheck.clone()),
            Err(e) => {
                return Err(RqStop::Why(
                    RayQueryWhy::Shader,
                    format!("ray query のシェーダー・パイプラインを作れない: {e}"),
                ))
            }
        };
        let mut params = packed.params;
        params.local_start = 0;
        params.count = rayquery::CHECK_RAYS;
        let limit = params.ao_max.max(params.th_max).max(params.range);
        let need_cull = params.flags & pack::F_BACK != 0;
        self.queue
            .write_buffer(params_buffer, 0, bytemuck::bytes_of(&params));
        let compute_selfcheck = self.compute.selfcheck.clone();
        let check_run = |e: GpuBakeError| RqStop::Why(RayQueryWhy::CheckRun, e.to_string());
        let c = self
            .run_selfcheck(
                &compute_selfcheck,
                compute_group,
                params_buffer,
                out_rb,
                out,
            )
            .map_err(check_run)?;
        // 裏表の規約が compute の道と逆（裏面を飛ばすレイだけが合わない）なら、頂点の並びを入れ替えてもう一度だけ確かめる。
        let mut flip = false;
        loop {
            stop_check(plan, progress)?;
            let mut stopped = None;
            let accel = rayquery::build(&self.device, &self.queue, packed, flip, &mut || {
                stopped = plan.checkpoint(PREPARE_FRACTION, "Preparing", progress);
                stopped.is_some()
            });
            let accel = match accel {
                Ok(a) => a,
                Err(rayquery::BuildError::Stopped) => {
                    return Err(RqStop::Stopped(Box::new(stopped.expect("止めた"))))
                }
                Err(rayquery::BuildError::Failed(e)) => {
                    return Err(RqStop::Why(RayQueryWhy::Accel, e))
                }
            };
            stop_check(plan, progress)?;
            let mut entries: Vec<_> = buffers
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupEntry {
                    binding: i as u32,
                    resource: b.as_entire_binding(),
                })
                .collect();
            entries.push(wgpu::BindGroupEntry {
                binding: 12,
                resource: accel.tlas_low.as_binding(),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: 13,
                resource: accel.tlas_high.as_binding(),
            });
            let rq_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ベイク（ray query）"),
                layout: &rq_layout,
                entries: &entries,
            });
            let flags = if flip { pack::F_RQ_FLIP } else { 0 };
            let mut check_params = params;
            check_params.flags |= flags;
            self.queue
                .write_buffer(params_buffer, 0, bytemuck::bytes_of(&check_params));
            let r = self
                .run_selfcheck(&rq_selfcheck, &rq_group, params_buffer, out_rb, out)
                .map_err(check_run)?;
            match rayquery::compare(&c, &r, limit, need_cull) {
                rayquery::Verdict::Pass(summary) => {
                    // 加速構造は束縛（TLAS → BLAS）が持ち続ける。頂点のバッファは構築が済んだので手放してよい。
                    drop(accel);
                    let how = if flip {
                        "頂点の並びを入れ替え、"
                    } else {
                        ""
                    };
                    return Ok((
                        rq_pipeline,
                        rq_group,
                        format!("ray query（{how}{summary}）"),
                        flags,
                    ));
                }
                rayquery::Verdict::CullReversed(_) if !flip => flip = true,
                rayquery::Verdict::CullReversed(why) => {
                    return Err(RqStop::Why(
                        RayQueryWhy::CheckFailed,
                        format!("頂点の並びを入れ替えても{why}"),
                    ))
                }
                rayquery::Verdict::Fail(why) => {
                    return Err(RqStop::Why(RayQueryWhy::CheckFailed, why))
                }
            }
        }
    }

    /// 読み戻し用のバッファを、提出した作業の完了を待って読み、割り付けを解く。
    fn read(
        &mut self,
        buffer: &wgpu::Buffer,
        bytes: u64,
        submission: wgpu::SubmissionIndex,
    ) -> Result<Vec<u8>, GpuBakeError> {
        let (tx, rx) = mpsc::sync_channel(1);
        buffer
            .slice(..bytes)
            .map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
        let waited = self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(WAIT),
        });
        let received = waited
            .map_err(failed)
            .and_then(|_| rx.recv_timeout(Duration::from_secs(1)).map_err(failed))
            .and_then(|r| r.map_err(failed));
        if let Err(e) = received {
            self.hung = true;
            self.failed = Some(e.to_string());
            return Err(e);
        }
        let data = buffer
            .slice(..bytes)
            .get_mapped_range()
            .map_err(failed)?
            .to_vec();
        buffer.unmap();
        self.check_errors()?;
        Ok(data)
    }
}

impl GpuBakeStats {
    fn none() -> Self {
        Self {
            method: GpuBakeMethod::Compute,
            dispatches: 0,
            max_dispatch_ms: 0.0,
            max_dispatch_texels: 0,
            bands: 0,
            input_bytes: 0,
            band_bytes: 0,
            ray_query_note: None,
            ray_query_why: None,
        }
    }
}

#[allow(dead_code)]
fn _assert_layout() {
    // Params の大きさは 16 バイトの倍数（uniform の束縛の規則）。
    const _: () = assert!(std::mem::size_of::<Params>().is_multiple_of(16));
    let _ = Params::zeroed();
    let _ = NONE;
}
