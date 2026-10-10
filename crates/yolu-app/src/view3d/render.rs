//! 3D ビューの wgpu の描画（自前。埋め込みの Unity のプレイヤー（UaaL）は後で同じ枠に差し込む）。
//!
//! - 描き先は自前の色（Rgba8Unorm）と深度のテクスチャで、egui のネイティブのテクスチャとして画像で貼る（egui の描画のパスの MSAA・
//!   深度の設定に左右されない）。描くのはカメラ・大きさ・モデル・塗った絵・表示の設定のどれかが変わったときだけ。
//! - アンチエイリアスは MSAA: 面は多サンプルの色・深度へ描き、パスの終わりに解決（resolve）して、解決後の 1 サンプルの絵だけをこの先（ブルーム・
//!   トーンマッピング・egui）が読む。サンプル数は設定の選び（`Display::post`）を、機材が対応する数・描き先のメモリの上限（`plan_samples`）で下げたもの。
//!   ブルームとトーンマッピングは解決のあと（HDR の 1 サンプル）に当てる。画面の点からの当たりは CPU のレイ（モデルの三角形）で、描いた絵は読まない。
//! - 塗った絵は標準の 6 チャンネルのテクスチャ（`paint`。変わったタイルだけを上げる）。今のセットの絵に加えて、ほかのテクスチャセットの絵も
//!   縮めた段で持って見せる（`prepare_sets`。マテリアルごとに束ね（group 1）を替えて描く。頂点はマテリアル順に並べる）。面の見え方は
//!   `shaders/scene.wgsl`: マテリアル（Unity の
//!   Standard と同じ BRDF の PBR）・中立（Unity 版のプレビューの簡単な明暗）・チャンネルだけ（光なし）。環境（`environment`。空・スタジオを
//!   CPU で焼いた GGX のキューブと SH）と背景、トーンマッピングと露出（HDR の描き先に描いて `shaders/tonemap.wgsl` で 8 bit へ）は Unity 版と同じ作り。
//! - 色の約束: 出力は「画面にそのまま出す値」（ガンマ）。中立・チャンネルだけはガンマの空間のまま、マテリアルはリニアで解いて最後にガンマへ。
//!   egui はネイティブのテクスチャの値をガンマの値として読む。
//! - 接線（法線マップの向き）は MikkTSpace で別のスレッドで作る（`ViewModel::tangents`）。出来るまでは前のモデルの接線を使い、無ければ法線マップを
//!   読まない（描き始めを待たせない。ポーズで毎回モデルが替わるときも描きを止めない）。

use std::sync::Arc;
use std::thread::JoinHandle;

use eframe::egui_wgpu::{self, wgpu};
use wgpu::util::DeviceExt;
use yolu_core::geometry::{OrbitCamera, Projection};
use yolu_core::glam::{Mat4, Vec3, Vec4};
use yolu_core::mesh_maps::BakedMeshMap;
use yolu_core::Document;

use super::brdf::{self, Curve};
use super::display::{
    clamp_samples, Display, EnvKind, Shading, DEFAULT_SAMPLES, KEY_BITS, SAMPLE_CHOICES,
};
use super::environment::{self, Baked, Source, FACE_SIZE, MIP_COUNT};
use super::look_gpu::{self, LookBudget, LookGpu, SetDraw};
use super::model::ViewModel;
use super::other_sets::OtherSet;
use super::paint::{ImageTexture, Paint, PaintStats, Slot, UvSource};
use super::received_layers::BUDGET_BYTES as RECEIVED_BUDGET_BYTES;
use super::selection_overlay::{OverlayGpu, OverlayInput};
use super::tangents::Tangent;
use super::user_layers::USER_BUDGET_BYTES;

/// 背景（Unity 版の 3D ビューのカメラの背景 (0.12, 0.13, 0.15)）。
pub const BACKGROUND: [f64; 3] = [0.12, 0.13, 0.15];
const LDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// 同じ描き先の sRGB の見え方（lilToon の半透明をリニアで重ねる）。
const LDR_SRGB: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub(super) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// 面の描き先の形式（選択範囲の重ねも同じ形式で描く）。
pub(super) const LDR_FORMAT: wgpu::TextureFormat = LDR;
pub(super) const HDR_FORMAT: wgpu::TextureFormat = HDR;
/// 頂点 1 つ: 位置 3・法線 3・UV 2・接線 4（f32）。絵を貼るか・どの絵かは、マテリアルごとの描きで束ね（group 1）が決める。
pub(super) const VERTEX_FLOATS: usize = 12;
/// 今のセットでないセットの絵の一辺の上限（縮めて持つ。今のセットは文書の大きさのまま）。
pub const OTHER_SET_MAX_SIZE: u32 = 1024;
/// 1 フレームの同期（今のセットとほかのセットの合成・上げ）にかけてよい時間。ほかのセットの絵を新しく作り始める（文書を合成して縮める。
/// 重い）のは、この時間に収まっているあいだと、そのフレームの最初の 1 つだけ。残りは次のフレームから（セットの多いプロジェクトで
/// 1 フレームに重なって止まらないように。小さな文書は 1 フレームで全部作る）。
pub const BUILD_FRAME_BUDGET: std::time::Duration = std::time::Duration::from_millis(40);
/// 一様バッファの大きさ（`shaders/scene.wgsl` の `Uniforms`）。
const UNIFORM_BYTES: u64 = 128 + 9 * 16 + 9 * 16 + 64 + 16 + 9 * 16;
/// 影のマップの 1 辺（Depth32Float で 16 MiB。Unity 版と同じ 2048）。
pub const SHADOW_SIZE: u32 = 2048;
/// ブルームの拡大で、1 つ小さい段を足すときの重み（等比数列の公比）。大きいほど遠くまで広がる。
const BLOOM_SCATTER: f32 = 0.6;
/// ブルームの段の数の上限（半分の大きさから 1 段ごとに半分）。
const BLOOM_MAX_LEVELS: u32 = 6;
/// 持っておく lilToon のパイプラインの数の目安（超えたら、そのフレームで使わないものを捨てる）。
const LIL_PIPELINES_KEPT: usize = 48;

/// 上げた量・描いた回数（試験と状態の表示用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View3dStats {
    /// 最後の同期で上げたタイルの数（全チャンネルの合計）。
    pub last_tiles: usize,
    /// 最後の同期でチャンネルごとに上げたタイルの数（Color・Metallic・Roughness・Normal・Emission・Height の順）。
    pub last_slot_tiles: [usize; 6],
    /// 最後の同期で全部を作り直したか。
    pub last_rebuilt: bool,
    /// これまでに上げたタイルの数。
    pub total_tiles: usize,
    pub total_slot_tiles: [usize; 6],
    /// これまでに 3D を描いた回数（変わらなければ描かない）。
    pub renders: usize,
    /// そのうちトーンマッピングを当てた回数。
    pub tone_mapped_renders: usize,
    /// 文書から塗った絵へ縮めた段（0 なら同じ大きさ）。
    pub paint_level: u32,
    /// その縮めが、GPU のテクスチャの辺の上限でなくバイトの予算で決まったか。
    pub paint_by_budget: bool,
    /// チャンネルのテクスチャ（ミップ込み）の GPU のバイト数。
    pub paint_bytes: u64,
    /// 見せているメッシュマップを元の絵から縮めた段（0 なら同じ大きさ。見せていなければ 0）。
    pub map_level: u32,
    /// 今 GPU に上げているモデルの世代（`ViewModel::revision`）。モデルを入れ替えたのに追いついていないと、見える形と当たる形がずれる。
    pub mesh_revision: u32,
    /// これまでに頂点を上げ直した回数。
    pub mesh_uploads: usize,
    /// これまでに環境を焼いた回数。
    pub env_bakes: usize,
    /// これまでに影のマップを描いた回数（光の向き・モデルが変わったときだけ描き直す。カメラでは描き直さない）。
    pub shadow_renders: usize,
    /// 直前の描きで影を使ったか。
    pub shadow_active: bool,
    /// 今の頂点が持つ接線が、そのモデルの接線か（false なら前のモデルの接線か、法線マップを使わない）。
    pub tangents_exact: bool,
    /// 最後の `prepare` の CPU の時間（マイクロ秒）: 全体と、そのうち塗った絵の同期（合成・縮め・上げ・ミップの積み）。GPU の実行は含まない。
    pub last_prepare_us: u64,
    pub last_sync_us: u64,
    /// 粗く合成した絵を見せているタイルの数（ドラッグの間。終われば正確に上げ直して 0）。
    pub paint_coarse_tiles: usize,
    /// 塗った絵のミップマップを UV の上の画素だけで作った回数（チャンネルごとに 1 回。これまでの合計）と、粗い絵を見せている間に全部のテクセルの箱の平均で作った回数。
    pub paint_weighted_mip_builds: u64,
    pub paint_coarse_mip_builds: u64,
    /// 今のセットでないセットの絵を持っている数（GPU に作ってあるもの）。
    pub other_sets: usize,
    /// 持ちたいが、メモリの予算が足りずに持っていないセットの数（その面は絵の無い描き方）。
    pub other_skipped: usize,
    /// 持ちたいが、まだ作っていないセットの数（1 フレームに作る数を絞っているので、次のフレームから）。
    pub other_pending: usize,
    /// 今のセットの lilToon のスロットが読むユーザーチャンネルの配列のバイト数（ミップ込み）と、文書から縮めた段（持っていなければ 0）。
    pub user_bytes: u64,
    pub user_level: u32,
    /// 今のセットの、Live Link で Unity から受けた絵の配列のバイト数（ミップ込み）とレイヤーの大きさ（持っていなければ 0）。
    pub received_bytes: u64,
    pub received_size: [u32; 2],
    /// これまでに lilToon の値（一様バッファの中身）を作った回数（今のセットとほかのセット。値が変わらないフレームでは作らない）。
    pub look_params_builds: u64,
    /// ほかのセットの絵（ユーザーチャンネルの配列と受けた絵の配列を含む）のバイト数（ミップ込み）と、いちばん縮めた段。
    pub other_bytes: u64,
    pub other_level: u32,
    /// 今のセットだった絵を、GPU の中のミップのコピーで縮めてほかのセットへ回した回数（文書を合成し直さなかった回数。これまでの合計）。
    pub other_demotions: usize,
    /// 最後の同期の途中で GPU に持っていた絵（今のセットとほかのセット。ユーザーチャンネルの配列と受けた絵の配列を含む。ミップ込み）のバイト数の最大。今のセットが替わるフレームでも、
    /// 前の絵と新しい絵が満量で重なって予算を超えないことの記録（直前のフレームの終わりの分から数える）。
    pub peak_bytes: u64,
    /// ほかのセットの合成の作業用のバッファが抱えているバイト数（CPU。作り終えたら手放すので、普通は 0）。
    pub other_scratch_bytes: u64,
    /// lilToon の半透明を、トーンマッピングなしの描き先でリニアに重ねられるか（描き先の sRGB の見え方を作れる機材。GL は作れないので
    /// ガンマの値のまま重ねる）。
    pub linear_transparent: bool,
    /// 持っている lilToon のパイプラインの数と、これまでに作った数（ソフトの描画は使う機能とスロットの読み方ごとに作る）。
    pub lil_pipelines: usize,
    pub lil_pipeline_builds: usize,
    /// 今の描き先のサンプル数（選びを、機材が対応する数・描き先のメモリの上限で下げたもの。1 は多サンプルなし）。
    pub samples: u32,
    /// 今の描き先（色・深度・多サンプル・HDR・ブルームの段）の GPU のバイト数の見積もり（`target_bytes`）。
    pub target_bytes: u64,
    /// これまでにブルームを足して描いた回数。
    pub bloom_renders: usize,
    /// 選択範囲の重ねが今持っている GPU のバイト数（ミップ込み。出していなければ 0）と、これまでに上げたタイルの数、持ちたいが 3D の絵の予算に入らずに
    /// 出していないか。
    pub overlay_bytes: u64,
    pub overlay_tiles: u64,
    pub overlay_skipped: bool,
    /// 出していない理由が、文書の大きさが GPU のテクスチャの辺の上限を超えること（`overlay_skipped` のときだけ。そうでなければ予算）。
    pub overlay_too_large: bool,
}

impl From<PaintStats> for View3dStats {
    fn from(p: PaintStats) -> Self {
        View3dStats {
            last_tiles: p.last_tiles,
            last_slot_tiles: p.last_slot_tiles,
            last_rebuilt: p.last_rebuilt,
            total_tiles: p.total_tiles,
            total_slot_tiles: p.total_slot_tiles,
            paint_level: p.level,
            paint_by_budget: p.by_budget,
            paint_bytes: p.gpu_bytes,
            paint_coarse_tiles: p.coarse_tiles,
            paint_weighted_mip_builds: p.weighted_mip_builds,
            paint_coarse_mip_builds: p.coarse_mip_builds,
            ..View3dStats::default()
        }
    }
}

struct Target {
    view: wgpu::TextureView,
    /// 同じ色のテクスチャの sRGB の見え方（lilToon の半透明をリニアで重ねる）。機材が見え方の替えを持たなければ無い。
    srgb_view: Option<wgpu::TextureView>,
    size: [u32; 2],
    id: egui::TextureId,
}

/// 仕上げ（ブルーム・トーンマッピング）を通すときの HDR の描き先（解決後の 1 サンプル）。
struct HdrTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
}

/// 面を描く深度と、サンプル数が 2 以上のときの多サンプルの色（パスの終わりに解決して、`Target`・`HdrTarget` へ）。
struct SceneBuffers {
    size: [u32; 2],
    samples: u32,
    /// 多サンプルの色が HDR の形式か（仕上げを通すとき）。サンプル数 1 では使わない。
    hdr: bool,
    depth: wgpu::TextureView,
    _depth_texture: wgpu::Texture,
    color: Option<MsaaColor>,
}

struct MsaaColor {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// sRGB の見え方（8 bit の描き先で、半透明をリニアで重ねる 2 つ目のパスが使う）。機材が見え方の替えを持たなければ無い。
    srgb_view: Option<wgpu::TextureView>,
}

/// ブルームの縮小・拡大の段（半分の大きさから、1 段ごとに半分。ミップの 1 段が 1 つの描き先）。
struct BloomChain {
    _texture: wgpu::Texture,
    /// 段ごとの見え方（描き先にも、次の段の入力にもなる）。
    views: Vec<wgpu::TextureView>,
    /// 元の絵の大きさ。
    size: [u32; 2],
}

/// ブルームのパイプラインと入力（初めてブルームを使うときに作る。使わなければ何も作らない）。
struct BloomPipes {
    prefilter: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
}

struct GpuMesh {
    buffer: wgpu::Buffer,
    vertices: u32,
    /// 上げたモデルの世代（`ViewModel::revision`）。モデルの見分けはアドレスでなく世代で行う: ポーズの変更はモデルを何度も入れ替え、
    /// 落とした古いモデルのアドレスを次の新しいモデルが使うことがあり、そうなると上げ直しも描き直しも起きない。世界は
    /// `View3dState::next_revision` で作るので、モデルごとに違う。
    model: u32,
    /// 上げた接線の見分け（0 なら接線なし）。
    tangent_stamp: usize,
    /// マテリアル順に並べた頂点の範囲（マテリアルごとに別の束ねで描く）。
    ranges: Vec<MaterialRange>,
}

/// 頂点の並びの中の、1 つのマテリアルの範囲。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MaterialRange {
    material: i32,
    start: u32,
    count: u32,
}

/// 1 つのセットの絵を束ねたもの（group 1: 6 チャンネルのテクスチャとパラメータ）。`uid`・`layout_version` が替わったら作り直す。
struct SetBind {
    bind: wgpu::BindGroup,
    uid: u64,
    layout_version: u64,
    /// 見た目の持ち物の束ねの鍵（ユーザーチャンネルの配列・マットキャップの絵）。
    look: (u64, u64, u64),
}

/// 今のセットでないセットの絵（文書の ID でセットを見分ける。マテリアルは今の割り当て）。
struct HeldSet {
    doc_id: u128,
    material: i32,
    paint: Paint,
    /// 見た目の設定の持ち物（lilToon の値・ユーザーチャンネル・マットキャップの絵）。
    look: LookGpu,
    bind: Option<SetBind>,
}

/// マテリアルごとの描きで使う束ね。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    Current,
    Held(usize),
    Blank,
}

#[derive(Clone, Copy, PartialEq)]
struct SceneKey {
    /// 注視点・yaw・pitch・距離と、投影（0 透視・1 正投影）・正投影の見える高さ。
    camera: [u32; 8],
    size: [u32; 2],
    /// モデルの世代（`GpuMesh::model` と同じ）。
    model: u32,
    material: i32,
    /// 今のセットとほかのセットの絵の鍵（`paint_key`）。
    paint: u64,
    tangents: usize,
    env: u64,
    map: u64,
    display: [u32; KEY_BITS],
    /// 描き先のサンプル数（機材と描き先のメモリで下げたあとの数）。
    samples: u32,
    /// 全部のセットの見た目の鍵。
    looks: u64,
    /// 選択範囲の重ねの鍵（`OverlayGpu::key`）。
    overlay: u64,
}

struct Pipelines {
    scene: wgpu::RenderPipeline,
    background: wgpu::RenderPipeline,
}

/// lilToon の描き方のパイプラインの選び（描き先の形式・Cull・半透明・輪郭線）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LilPipe {
    hdr: bool,
    /// 描き先の sRGB の見え方へ、リニアの値で描く（半透明をリニアで重ねる）。
    srgb: bool,
    cull: u8,
    transparent: bool,
    outline: bool,
    /// パイプラインの定数（使う機能とスロットの読み方。実機は全部入り）。
    spec: look_gpu::LilSpec,
    /// 描き先のサンプル数。
    samples: u32,
}

/// 影のマップ（光から見た深さ。影を初めて使うときに作る）。
struct ShadowMap {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// 影のパスへ渡す行列（`shaders/shadow.wgsl`）と、その束ね。
    matrix_buffer: wgpu::Buffer,
    bind: wgpu::BindGroup,
    /// 今の絵を描いたときの鍵。同じなら描き直さない。
    drawn: Option<ShadowKey>,
    /// 世界 → (u, v, 深さ)、外接球の直径（世界）。
    world_to_shadow: Mat4,
    diameter: f32,
}

#[derive(Clone, Copy, PartialEq)]
struct ShadowKey {
    model: u32,
    vertices: u32,
    light: [u32; 3],
}

struct ToneMap {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
}

/// 焼いた環境の GPU のキューブ。
struct EnvCube {
    view: wgpu::TextureView,
    _texture: wgpu::Texture,
}

struct EnvState {
    /// 焼いた元（種類と空の色）。
    baked_for: Option<(EnvKind, [f32; 9])>,
    baked: Option<Baked>,
    cube: Option<EnvCube>,
    /// 焼くたびに増える（束ねと描き直しの鍵）。
    version: u64,
}

/// 接線を作る別のスレッドの、仕事の前に呼ぶ口（試験が、接線が着く前のフレームを決定的に作る・作業の失敗を起こすのに使う）。
pub type TangentHook = Arc<dyn Fn() + Send + Sync>;

/// 接線を作っている別のスレッド。
struct Inflight {
    model: Arc<ViewModel>,
    worker: JoinHandle<()>,
}

/// 3D で見せる焼いたメッシュマップ（`key` が同じなら作り直さない）。
pub struct MeshMapSource<'a> {
    pub key: u64,
    pub map: &'a BakedMeshMap,
}

/// 3D ビューの描画（wgpu の装置は eframe と同じもの）。
pub struct View3dRenderer {
    rs: egui_wgpu::RenderState,
    /// 今のセットの絵。
    paint: Paint,
    current_bind: Option<SetBind>,
    /// 今のセットでないセットの絵（予算に入る分だけ。今のセットから近い順に持つ）。
    held: Vec<HeldSet>,
    /// 絵を持たない面の束ね（既定の 1 × 1、絵を貼らない）。
    blank_bind: wgpu::BindGroup,
    /// メモリの予算の全体（今のセットの絵のバイト数を引いた残りに、ほかのセットが入る）。
    total_budget: u64,
    /// 今のセットのユーザーチャンネルの配列に渡す予算（全体から今のセットの標準のチャンネルの絵を引いた残り。`sync_sets` が決める）。
    user_budget: u64,
    /// 今のセットの、Unity から受けた絵の配列に渡す予算（標準のチャンネルの絵とユーザーチャンネルの配列を引いた残り。`sync_sets` が決める）。
    received_budget: u64,
    /// ほかのセットの絵の辺の上限。
    other_cap: u32,
    /// 予算が足りずに絵を持っていないセットのマテリアル。
    unpainted: Vec<i32>,
    /// 持ちたいが、1 フレームの作る数の上限で待たせているセットの数。
    pending_builds: usize,
    /// ミップのコピーで縮めた回数（`View3dStats::other_demotions`）。
    demotions: usize,
    /// 最後の同期の途中で GPU に持っていた絵（今のセットとほかのセット）のバイト数の最大（`View3dStats::peak_bytes`）。
    peak_bytes: u64,
    /// ほかのセットの絵を新しく作り始めてよい 1 フレームの時間（`BUILD_FRAME_BUDGET`）。
    build_budget: std::time::Duration,
    /// ほかのセットの絵を見せるか（計測が、今のセットだけを同期する前の実装と同じ仕事と比べるために切る。普段は true）。
    show_others: bool,
    scene_layout: wgpu::BindGroupLayout,
    set_layout: wgpu::BindGroupLayout,
    /// 選択範囲の重ね（今のセットの面の上に、縁・赤い重ね・選択ペンの被覆を描く）と、3D の絵の予算に入るか（`sync_sets` が決める）。
    overlay: OverlayGpu,
    overlay_allowed: bool,
    /// 重ねを出せない理由が、辺の上限か（`sync_sets` が決める）。
    overlay_too_large: bool,
    ldr: Pipelines,
    hdr: Pipelines,
    /// 面と背景のパイプラインをサンプル数ごとに作り直すための持ち物。
    background_pipeline_layout: wgpu::PipelineLayout,
    /// 今の描き先のサンプル数（`ldr`・`hdr`・lilToon のパイプラインがこの数で作ってある）。
    samples: u32,
    /// 機材が面の描き先（8 bit・HDR・深度）に使えるサンプル数（昇順。1 を含む）。
    supported: Vec<u32>,
    /// 描き先のメモリの上限の指定（None は 3D の絵の予算と同じ量。試験・計測が小さくして、サンプル数を下げる道を通す）。
    /// 設定の「GPU のメモリ」が配る 3 つの予算（`gpu_memory::Budgets`）には入らない別の勘定。
    target_budget: Option<u64>,
    /// 面のシェーダー（scene.wgsl と lilToon の部品）と、そのパイプラインの形（lilToon のパイプラインを使うときに作る）。
    scene_module: wgpu::ShaderModule,
    scene_pipeline_layout: wgpu::PipelineLayout,
    lil_pipelines: std::collections::HashMap<LilPipe, wgpu::RenderPipeline>,
    lil_pipeline_builds: usize,
    /// 今のセットの見た目の持ち物と、絵の無い面の見た目（標準）。
    current_look: LookGpu,
    _blank_look: LookGpu,
    /// 1 × 1 の白（マットキャップの絵が無いときに束ねる）。
    white_view: wgpu::TextureView,
    _white_texture: wgpu::Texture,
    tone: ToneMap,
    shadow_pipeline: wgpu::RenderPipeline,
    shadow_layout: wgpu::BindGroupLayout,
    shadow_sampler: wgpu::Sampler,
    dummy_shadow: wgpu::TextureView,
    _dummy_shadow_texture: wgpu::Texture,
    shadow: Option<ShadowMap>,
    /// 影のマップを作るたびに増える（束ねの鍵）。
    shadow_version: u64,
    /// モデルの外接（世代・中心・外接球の半径）。影の光のカメラを決める。
    bounds: Option<(u32, Vec3, f32)>,
    uniforms: wgpu::Buffer,
    paint_sampler: wgpu::Sampler,
    /// 塗った絵のサンプラーの異方性の上限（GPU が異方性フィルタリングを持たなければ 1）。
    paint_anisotropy: u16,
    env_sampler: wgpu::Sampler,
    dummy_cube: wgpu::TextureView,
    _dummy_texture: wgpu::Texture,
    env: EnvState,
    /// 見せているメッシュマップの絵（`MeshMapSource::key` と一緒）。
    map: Option<(u64, ImageTexture)>,
    /// 作れなかったメッシュマップの `key`（同じ絵を毎フレーム作り直さない）。
    map_failed: Option<u64>,
    /// 絵を入れ替えるたびに増える（束ねと描き直しの鍵）。
    map_version: u64,
    target: Option<Target>,
    hdr_target: Option<HdrTarget>,
    scene: Option<SceneBuffers>,
    bloom: Option<BloomPipes>,
    bloom_chain: Option<BloomChain>,
    /// ブルームを使わないあいだ、仕上げの束ねに入れる 1 × 1 の黒。
    dummy_bloom: wgpu::TextureView,
    _dummy_bloom_texture: wgpu::Texture,
    bloom_sampler: wgpu::Sampler,
    mesh: Option<GpuMesh>,
    bind: Option<(wgpu::BindGroup, (u64, u64, u64))>,
    last_key: Option<SceneKey>,
    /// 接線を作っているモデルのスレッドと、最後に出来た接線（ポーズで同じ形のモデルが続くとき、出来るまでこれを使う）。
    inflight: Option<Inflight>,
    last_tangents: Option<Arc<Vec<Tangent>>>,
    /// 今の表示が接線を読むか（読まないあいだは、作っていてもウィンドウの描き直しを求めない）。
    tangents_wanted: bool,
    /// 接線を作るスレッドが接線を残さずに終わったモデルの世代（同じモデルで作り直さない。法線マップは読まない）。
    tangents_failed: Option<u32>,
    tangent_hook: Option<TangentHook>,
    /// 描き先のテクスチャに sRGB の見え方を作れるか（wgpu の `DownlevelFlags::VIEW_FORMATS`。GL には無い）。
    srgb_views: bool,
    /// ソフトの描画（アダプタが CPU。llvmpipe）: lilToon のパイプラインを、使う機能とスロットの読み方だけで作る（`look_gpu::LilSpec`）。
    software: bool,
    pub stats: View3dStats,
}

/// 1 つのセットの絵の束ね（group 1）。`painted` なら絵を貼り、そうでなければ市松（`paint` は既定の 1 × 1 を見せる持ち物でよい）。
/// `current` は今のセット（焼いたメッシュマップはその面だけに見せる）。パラメータは束ねと同じ寿命で変わらない（Normal を使い始めた・
/// やめたときは、束ねごと作り直す）。見た目の設定（`look`）の値は別のバッファで、値が変わっても束ねは作り直さない（ユーザーチャンネルの
/// 配列・マットキャップの絵が替わったときだけ）。
fn make_set_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    paint: &Paint,
    painted: bool,
    current: bool,
    look: &LookGpu,
    white: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let flags = [
        f32::from(painted),
        f32::from(painted && paint.has(Slot::Normal)),
        f32::from(painted && current),
        0.0,
    ];
    let bytes: Vec<u8> = flags.iter().flat_map(|v| v.to_le_bytes()).collect();
    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("yolu-3d-set-params"),
        contents: &bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let mut entries: Vec<wgpu::BindGroupEntry> = Slot::ALL
        .iter()
        .map(|slot| wgpu::BindGroupEntry {
            binding: slot.index() as u32,
            resource: wgpu::BindingResource::TextureView(paint.view(*slot)),
        })
        .collect();
    entries.push(wgpu::BindGroupEntry {
        binding: 6,
        resource: params.as_entire_binding(),
    });
    entries.push(wgpu::BindGroupEntry {
        binding: 7,
        resource: look.buffer.as_entire_binding(),
    });
    entries.push(wgpu::BindGroupEntry {
        binding: 8,
        resource: wgpu::BindingResource::TextureView(look.users.view()),
    });
    for (binding, which) in [(9u32, 0usize), (10, 1)] {
        entries.push(wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(look.image_view(which).unwrap_or(white)),
        });
    }
    entries.push(wgpu::BindGroupEntry {
        binding: 11,
        resource: wgpu::BindingResource::TextureView(look.received.view()),
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("yolu-3d-set"),
        layout,
        entries: &entries,
    })
}

fn texture_entry(
    binding: u32,
    dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}

fn depth_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// ブルームの段の大きさ（最初の段は元の絵の半分。1 段ごとに半分で、1 画素未満にはしない）。
pub fn bloom_levels(size: [u32; 2]) -> Vec<[u32; 2]> {
    let base = [(size[0] / 2).max(1), (size[1] / 2).max(1)];
    // 小さい絵では段を減らす（最後の段が 4 画素ほどになるまで）
    let count = (size[0].min(size[1]).max(4).ilog2() - 2).clamp(1, BLOOM_MAX_LEVELS);
    (0..count)
        .map(|k| [(base[0] >> k).max(1), (base[1] >> k).max(1)])
        .collect()
}

/// 面の描き先の GPU のバイト数の見積もり（テクスチャの大きさの式。ドライバの詰め物は含めない）。内訳は、解決後の 8 bit（egui が読む）・
/// 深度（サンプルごと）・サンプル数が 2 以上のときの多サンプルの色（`hdr` なら HDR の形式）・仕上げを通すときの解決後の HDR・ブルームの段。
pub fn target_bytes(size: [u32; 2], samples: u32, hdr: bool, bloom: bool) -> u64 {
    let pixels = size[0] as u64 * size[1] as u64;
    let samples = samples.max(1) as u64;
    let mut bytes = pixels * 4 + pixels * 4 * samples;
    if samples > 1 {
        bytes += pixels * samples * if hdr { 8 } else { 4 };
    }
    if hdr {
        bytes += pixels * 8;
    }
    if bloom {
        bytes += bloom_levels(size)
            .iter()
            .map(|l| l[0] as u64 * l[1] as u64 * 8)
            .sum::<u64>();
    }
    bytes
}

/// 面の描き先（8 bit・HDR・深度。8 bit は sRGB の見え方も）で機材が使えるサンプル数（昇順。1 を含む）。多サンプルを解決（resolve）できる形式だけ。
/// 装置が `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` を持たないあいだは、WebGPU が保証する 1 と 4 だけ（`wgpu_configuration` が、機材が持つときは装置に要求する）。
pub fn supported_samples(
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    srgb_views: bool,
) -> Vec<u32> {
    let specific = device
        .features()
        .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
    let features = |format: wgpu::TextureFormat| {
        if specific {
            adapter.get_texture_format_features(format)
        } else {
            format.guaranteed_format_features(device.features())
        }
    };
    let mut formats = vec![LDR, HDR];
    if srgb_views {
        formats.push(LDR_SRGB);
    }
    let colors: Vec<_> = formats.into_iter().map(features).collect();
    let depth = features(DEPTH_FORMAT);
    SAMPLE_CHOICES
        .into_iter()
        .filter(|&n| {
            n == 1
                || (depth.flags.sample_count_supported(n)
                    && colors.iter().all(|f| {
                        f.flags.sample_count_supported(n)
                            && f.flags
                                .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
                            && f.allowed_usages
                                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
                    }))
        })
        .collect()
}

/// 描き先のサンプル数を決める: 機材が対応する数（`supported`）のうち `want` 以下の最大から始め、描き先の見積もり（`target_bytes`）が
/// 上限 `budget` を超えるあいだ、次に小さい対応する数へ下げる（1 まで。1 は多サンプルなしで、上限を超えても使う: 描けなくはしない）。
pub fn plan_samples(
    supported: &[u32],
    want: u32,
    size: [u32; 2],
    hdr: bool,
    bloom: bool,
    budget: u64,
) -> u32 {
    let mut samples = clamp_samples(want, supported);
    while samples > 1 && target_bytes(size, samples, hdr, bloom) > budget {
        samples = clamp_samples(samples - 1, supported);
    }
    samples
}

/// ウィンドウの面の設定: 先に溜めるフレームは 1 枚（`SurfaceConfig::LOW_LATENCY`。eframe の既定の `HIGH_THROUGHPUT` は 2 枚）で、
/// 同期は `vsync` なら垂直同期を待つ（`AutoVsync` = 使えるなら FifoRelaxed → Fifo の順）、そうでなければ待たない（`AutoNoVsync` = 使えるなら Immediate → Mailbox → Fifo の順）。
/// 待たないと、フレームの出る間隔が垂直同期の枠に揃わない代わりに、ペンの入力から線が画面に出るまでの遅れが縮む
/// （描き直しが速すぎて回りすぎないよう、アプリが自分でフレームの間隔に下限をかける。`pacing`）。
/// wgpu が面の出し方に Fifo しか出さない道（wgpu-hal 30.0.1 の OpenGL は、Windows 以外では Fifo だけ）では、どちらの設定でも出し方は Fifo で、
/// 下限だけがかかる。Vulkan（lavapipe）では `AutoVsync` が FifoRelaxed、`AutoNoVsync` が Immediate になる（wgpu-core の振り替えのログで確かめた）。
pub fn surface_config(vsync: bool) -> egui_wgpu::SurfaceConfig {
    egui_wgpu::SurfaceConfig {
        present_mode: if vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        },
        ..egui_wgpu::SurfaceConfig::LOW_LATENCY
    }
}

/// 製品のウィンドウの wgpu の設定: eframe の既定に、アダプター固有の形式の機能（2× と 8× の多サンプルが使えるかを調べるのに要る）を、
/// 機材が持つときだけ装置へ足す。機能を足しても、使える形式・上限は増えるだけで減らない。
///
/// 面の設定は `surface_config(vsync)`（設定「垂直同期」。既定は待たない）。2D に描く操作は、入力から画面までの遅れをなめらかさより先にする。
/// eframe の 1 つの描画器が、別ウィンドウに出したビューポートを含む全ての面へこの設定を使い、起動のあとに切り替える口は無い
/// （変えた設定は次の起動から効く）。
pub fn wgpu_configuration(vsync: bool) -> egui_wgpu::WgpuConfiguration {
    let mut config =
        egui_wgpu::WgpuConfiguration::default().with_surface_config(surface_config(vsync));
    if let egui_wgpu::WgpuSetup::CreateNew(setup) = &mut config.wgpu_setup {
        let base = setup.device_descriptor.clone();
        setup.device_descriptor = Arc::new(move |adapter| {
            let mut descriptor = base(adapter);
            descriptor.required_features |=
                adapter.features() & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
            descriptor
        });
    }
    config
}

/// 面と背景のパイプライン（描き先の形式とサンプル数ごと）。
fn build_pipelines(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    scene_layout: &wgpu::PipelineLayout,
    background_layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    samples: u32,
) -> Pipelines {
    let attributes = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4
    ];
    Pipelines {
        scene: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("yolu-3d-scene"),
            layout: Some(scene_layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: (VERTEX_FLOATS * 4) as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Unity の表（左手系で外から見て時計回り）。切り取りの空間の y は上なので、画面でも時計回りが表
                front_face: wgpu::FrontFace::Cw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: multisample(samples),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        }),
        background: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("yolu-3d-background"),
            layout: Some(background_layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_bg"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: multisample(samples),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("fs_bg"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        }),
    }
}

/// パイプラインのサンプル数（描き先の多サンプルと同じ数）。
fn multisample(count: u32) -> wgpu::MultisampleState {
    wgpu::MultisampleState {
        count,
        ..Default::default()
    }
}

/// 塗った絵のサンプラーの異方性の上限。斜めに見た面は、画面で縦と横の縮み方が違うので、等方のミップだと強く縮む側に合わせて全体が
/// ぼやける。異方性フィルタリングは、縮みの小さい側の細かさを残す。wgpu の上限（16）まで。
const PAINT_ANISOTROPY: u16 = 16;

/// GPU が異方性フィルタリングを持つとき `wanted`（1〜16 に収める）、持たないとき 1。
/// wgpu も持たない GPU では 1 に直すが、使っている値を確かめられるよう、ここで決める。
fn supported_anisotropy(flags: wgpu::DownlevelFlags, wanted: u16) -> u16 {
    if flags.contains(wgpu::DownlevelFlags::ANISOTROPIC_FILTERING) {
        wanted.clamp(1, 16)
    } else {
        1
    }
}

/// サンプラーの既定の異方性。CPU で描くアダプター（`software`。lavapipe・WARP など）では 1 のままにする。lavapipe で測ると、異方性 16 は球の 1 フレームが
/// 約 28 ms から約 36 ms に増え、拡大して見る絵（粗さの段・法線）の境が実 GPU（d3d12 の OpenGL）より大きくぼける（絵の比べの正解や Unity との
/// 差の上限が、等方の絵で決まっているのも同じ）。ソフトで描く場面は斜めの鮮明さを求める用途でもない。試験が `set_paint_anisotropy` で上げて確かめる。
///
/// 判定は wgpu のアダプターの種類（`device_type == Cpu`。lavapipe・WARP はこれで報告される）。Mesa の d3d12 の OpenGL が WARP の上で動くときのように、
/// 種類が CPU と報告されないものは見分けられず、16 になりうる。
fn default_anisotropy(flags: wgpu::DownlevelFlags, software: bool) -> u16 {
    if software {
        1
    } else {
        supported_anisotropy(flags, PAINT_ANISOTROPY)
    }
}

/// 塗った絵（標準のチャンネル・ユーザーチャンネル・lilToon が読む画像）を読むサンプラー。異方性が 1 を超えるときは、wgpu の決まりで
/// 拡大・縮小・ミップの 3 つの補間がすべて Linear でなければならない。
///
/// 絵を読むサンプラーはこれだけ。環境のキューブ（`env_sampler`）は段を明示して読む（粗さからの `textureSampleLevel`）ので、異方性は効かない。
/// ブルーム・影のサンプラーは絵ではなく、画面の効果と深さの比較。
fn paint_sampler_descriptor(anisotropy: u16) -> wgpu::SamplerDescriptor<'static> {
    wgpu::SamplerDescriptor {
        label: Some("yolu-3d-paint"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        anisotropy_clamp: anisotropy,
        ..Default::default()
    }
}

impl View3dRenderer {
    pub fn new(rs: &egui_wgpu::RenderState) -> View3dRenderer {
        let device = &rs.device;
        let downlevel = rs.adapter.get_downlevel_capabilities().flags;
        let srgb_views = downlevel.contains(wgpu::DownlevelFlags::VIEW_FORMATS);
        let paint_anisotropy = default_anisotropy(
            downlevel,
            rs.adapter.get_info().device_type == wgpu::DeviceType::Cpu,
        );
        // 面のシェーダー: 標準（scene.wgsl）と lilToon の再現（`shaders/liltoon/` の部品をつないだもの。scene.wgsl の一様バッファ・束ね・
        // 関数を使う）を 1 つのモジュールに
        let scene_source = format!(
            "{}\n{}",
            include_str!("shaders/scene.wgsl"),
            super::liltoon_wgsl::SOURCE
        );
        let scene_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yolu-3d-scene"),
            source: wgpu::ShaderSource::Wgsl(scene_source.into()),
        });
        let tone_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yolu-3d-tonemap"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/tonemap.wgsl").into()),
        });
        let d2 = wgpu::TextureViewDimension::D2;
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }];
        // 塗った絵のサンプラーは、lilToon の輪郭線の頂点（太さのマスク）も読む
        let mut paint_sampler_entry = sampler_entry(7);
        paint_sampler_entry.visibility = wgpu::ShaderStages::VERTEX_FRAGMENT;
        entries.push(paint_sampler_entry);
        entries.push(texture_entry(8, wgpu::TextureViewDimension::Cube));
        entries.push(sampler_entry(9));
        entries.push(texture_entry(10, d2));
        entries.push(depth_entry(11));
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 12,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
            count: None,
        });
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yolu-3d-scene"),
            entries: &entries,
        });
        // group 1: 面ごとの絵（6 チャンネルのテクスチャ）と、その持ち主のパラメータ（x: 絵を貼る、y: 法線マップを読む）
        let mut set_entries: Vec<wgpu::BindGroupLayoutEntry> =
            (0..6).map(|b| texture_entry(b, d2)).collect();
        set_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 6,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        // 7: lilToon の値、8: ユーザーチャンネルの配列、9・10: マットキャップの絵（look_gpu）
        set_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 7,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        set_entries.push(texture_entry(8, wgpu::TextureViewDimension::D2Array));
        set_entries.push(texture_entry(9, d2));
        set_entries.push(texture_entry(10, d2));
        // 11: Unity から受けた、描いていないスロットの絵の配列（look_gpu・received_layers）
        set_entries.push(texture_entry(11, wgpu::TextureViewDimension::D2Array));
        // 輪郭線の頂点が太さのマスク（どのチャンネルのことも）と値を読む
        for e in &mut set_entries {
            e.visibility = wgpu::ShaderStages::VERTEX_FRAGMENT;
        }
        let set_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yolu-3d-set"),
            entries: &set_entries,
        });
        let scene_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("yolu-3d-scene"),
                bind_group_layouts: &[Some(&scene_layout), Some(&set_layout)],
                immediate_size: 0,
            });
        // 背景は面ごとの絵を読まない（group 1 を束ねなくてよい）
        let background_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("yolu-3d-background"),
                bind_group_layouts: &[Some(&scene_layout)],
                immediate_size: 0,
            });
        // 機材が使えるサンプル数。初めは既定の数（描くときに選びと描き先のメモリで決めて、数が変わればパイプラインを作り直す）
        let supported = supported_samples(&rs.adapter, device, srgb_views);
        let samples = clamp_samples(DEFAULT_SAMPLES, &supported);
        let ldr = build_pipelines(
            device,
            &scene_module,
            &scene_pipeline_layout,
            &background_pipeline_layout,
            LDR,
            samples,
        );
        let hdr = build_pipelines(
            device,
            &scene_module,
            &scene_pipeline_layout,
            &background_pipeline_layout,
            HDR,
            samples,
        );
        let tone_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yolu-3d-tonemap"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: d2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // ブルームの絵（強さが 0 のあいだは読まない。束ねには 1 × 1 の黒が入る）
                texture_entry(2, d2),
                sampler_entry(3),
            ],
        });
        let tone_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yolu-3d-tonemap"),
            bind_group_layouts: &[Some(&tone_layout)],
            immediate_size: 0,
        });
        let tone_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("yolu-3d-tonemap"),
            layout: Some(&tone_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &tone_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &tone_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: LDR,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let tone_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yolu-3d-tonemap"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // ブルームを使わないあいだに仕上げの束ねへ入れる、1 × 1 の黒（HDR）
        let dummy_bloom_texture = device.create_texture_with_data(
            &rs.queue,
            &wgpu::TextureDescriptor {
                label: Some("yolu-3d-bloom-dummy"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[0u8; 8],
        );
        let dummy_bloom = dummy_bloom_texture.create_view(&Default::default());
        let bloom_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("yolu-3d-bloom"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shadow_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yolu-3d-shadow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow.wgsl").into()),
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yolu-3d-shadow"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("yolu-3d-shadow"),
                bind_group_layouts: &[Some(&shadow_layout)],
                immediate_size: 0,
            });
        let position_only = wgpu::vertex_attr_array![0 => Float32x3];
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("yolu-3d-shadow"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shadow_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: (VERTEX_FLOATS * 4) as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &position_only,
                })],
            },
            // 光から見て表も裏も深さに書く（Unity 版の CASTER は Cull Off）
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("yolu-3d-shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        // 影を使わないあいだに束ねる、1 × 1 の深さ
        let dummy_shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-shadow-dummy"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let dummy_shadow = dummy_shadow_texture.create_view(&Default::default());
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yolu-3d-uniforms"),
            size: UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let paint_sampler = device.create_sampler(&paint_sampler_descriptor(paint_anisotropy));
        let env_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("yolu-3d-env"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        // 環境を使わないあいだに束ねる、1 × 1 のキューブ（黒）
        let dummy = device.create_texture_with_data(
            &rs.queue,
            &wgpu::TextureDescriptor {
                label: Some("yolu-3d-env-dummy"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 6,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[0u8; 6 * 8],
        );
        let dummy_cube = dummy.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        let paint = Paint::new(rs);
        let white_texture = device.create_texture_with_data(
            &rs.queue,
            &wgpu::TextureDescriptor {
                label: Some("yolu-3d-white"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: LDR,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[255u8; 4],
        );
        let white_view = white_texture.create_view(&Default::default());
        let current_look = LookGpu::new(device, &rs.queue);
        let blank_look = current_look.sibling(device);
        let blank_bind = make_set_bind(
            device,
            &set_layout,
            &paint,
            false,
            false,
            &blank_look,
            &white_view,
        );
        let overlay = OverlayGpu::new(device, &rs.queue, &scene_layout);
        View3dRenderer {
            rs: rs.clone(),
            paint,
            current_bind: None,
            held: Vec::new(),
            blank_bind,
            total_budget: super::paint::PAINT_BUDGET_BYTES,
            user_budget: USER_BUDGET_BYTES,
            received_budget: RECEIVED_BUDGET_BYTES,
            other_cap: OTHER_SET_MAX_SIZE,
            unpainted: Vec::new(),
            pending_builds: 0,
            demotions: 0,
            peak_bytes: 0,
            build_budget: BUILD_FRAME_BUDGET,
            show_others: true,
            scene_layout,
            set_layout,
            overlay,
            overlay_allowed: true,
            overlay_too_large: false,
            ldr,
            hdr,
            background_pipeline_layout,
            samples,
            supported,
            target_budget: None,
            scene_module,
            scene_pipeline_layout,
            lil_pipelines: std::collections::HashMap::new(),
            lil_pipeline_builds: 0,
            current_look,
            _blank_look: blank_look,
            white_view,
            _white_texture: white_texture,
            tone: ToneMap {
                pipeline: tone_pipeline,
                layout: tone_layout,
                params: tone_params,
            },
            shadow_pipeline,
            shadow_layout,
            shadow_sampler,
            dummy_shadow,
            _dummy_shadow_texture: dummy_shadow_texture,
            shadow: None,
            shadow_version: 0,
            bounds: None,
            uniforms,
            paint_sampler,
            paint_anisotropy,
            env_sampler,
            dummy_cube,
            _dummy_texture: dummy,
            env: EnvState {
                baked_for: None,
                baked: None,
                cube: None,
                version: 0,
            },
            map: None,
            map_failed: None,
            map_version: 0,
            target: None,
            hdr_target: None,
            scene: None,
            bloom: None,
            bloom_chain: None,
            dummy_bloom,
            _dummy_bloom_texture: dummy_bloom_texture,
            bloom_sampler,
            mesh: None,
            bind: None,
            last_key: None,
            inflight: None,
            last_tangents: None,
            tangents_wanted: false,
            tangents_failed: None,
            tangent_hook: None,
            srgb_views,
            software: rs.adapter.get_info().device_type == wgpu::DeviceType::Cpu,
            stats: View3dStats::default(),
        }
    }

    /// GPU のテクスチャの辺の上限（GPU が決める。文書の幅か高さがこれを超えると、選択範囲の重ねは出せない）。
    pub fn max_texture_dimension(&self) -> u32 {
        self.overlay.limit()
    }

    /// 次に描くときの、選択範囲の重ね（None なら何も重ねない）。
    pub fn set_overlay(&mut self, input: Option<OverlayInput>) {
        self.overlay.set_input(input);
    }

    /// 塗った絵のバイトの予算を決める（設定の GPU のメモリ・試験が小さくして、縮めの道を通す）。今のセットの絵を引いた残りに、
    /// ほかのセットの絵が入る。上げたときは、予算で縮めていた今のセットの絵を元の大きさへ戻す（ほかのセットは、替わるたびに
    /// 上限だけで決め直すので、ここでは触らない）。
    pub fn set_paint_budget(&mut self, bytes: u64) {
        self.total_budget = bytes;
        self.paint.set_budget_and_regrow(bytes);
    }

    /// 塗った絵の全体のバイトの予算（今のセットとほかのセットの合計）。
    pub fn paint_budget(&self) -> u64 {
        self.total_budget
    }

    /// 面の描き先（多サンプル・HDR・ブルームの段）に使ってよいバイト数。絵のテクスチャとは別の勘定で、同じ量まで（指定が無ければ）。
    /// 設定の合計（3D の絵・キャンバス・棚への配り）の中ではなく外に足す量なので、描き先が上限まで載ると、合計に加えてこの量だけ使う。
    /// 超える見積もりのサンプル数は、超えない最大の数へ下げる（`plan_samples`）。
    pub fn target_budget(&self) -> u64 {
        self.target_budget.unwrap_or(self.total_budget)
    }

    /// 描き先のメモリの上限を決める（試験・計測用。None で既定へ）。
    pub fn set_target_budget(&mut self, bytes: Option<u64>) {
        self.target_budget = bytes;
    }

    /// 塗った絵のサンプラーが今使っている異方性の上限（GPU が持たなければ 1）。
    pub fn paint_anisotropy(&self) -> u16 {
        self.paint_anisotropy
    }

    /// 塗った絵のサンプラーの異方性の上限を変える（試験・計測用。1 で等方。GPU が持たなければ 1 のまま）。
    pub fn set_paint_anisotropy(&mut self, wanted: u16) {
        let flags = self.rs.adapter.get_downlevel_capabilities().flags;
        let clamp = supported_anisotropy(flags, wanted);
        if clamp == self.paint_anisotropy {
            return;
        }
        self.paint_anisotropy = clamp;
        self.paint_sampler = self
            .rs
            .device
            .create_sampler(&paint_sampler_descriptor(clamp));
        // サンプラーは group 0 の束ねに入っている。絵は同じでも描き直す
        self.bind = None;
        self.last_key = None;
    }

    /// 機材が面の描き先に使えるサンプル数（昇順。1 を含む）。アンチエイリアスの選びに出す。
    pub fn supported_samples(&self) -> &[u32] {
        &self.supported
    }

    /// 選んだサンプル数 `want` から、今の描き先で使うサンプル数を決める（`plan_samples`）。
    pub fn plan_samples(&self, want: u32, size: [u32; 2], hdr: bool, bloom: bool) -> u32 {
        plan_samples(
            &self.supported,
            want,
            size,
            hdr,
            bloom,
            self.target_budget(),
        )
    }

    /// ほかのセットの絵を新しく作り始めてよい 1 フレームの時間を決める（試験が 0 にして、1 フレームに 1 つずつの道を通す）。
    pub fn set_other_build_budget(&mut self, budget: std::time::Duration) {
        self.build_budget = budget;
    }

    /// 計測用: false にすると、ほかのセットの絵を見せない（今のセットの絵だけを同期する。`prepare` と同じ仕事）。
    pub fn set_show_other_sets(&mut self, show: bool) {
        self.show_others = show;
    }

    /// 試験・計測用: 表示の写しを UV の外へ塗り広げる幅（表示のテクセル。0 は塗り広げない前の仕事）。塗り広げるかどうかが変われば
    /// 次の同期が全部を作り直す。幅だけを変えたときは、作ってある絵は前の幅のまま（`invalidate_paint` で作り直す）。
    pub fn set_display_padding(&mut self, texels: u32) {
        self.paint.set_padding_texels(texels);
        for h in &mut self.held {
            h.paint.set_padding_texels(texels);
        }
    }

    /// ほかのセットの絵の辺の上限を決める（試験が小さくして、縮めの道を通す）。
    pub fn set_other_cap(&mut self, cap: u32) {
        self.other_cap = cap.max(1);
    }

    /// 試験用: 塗った絵を捨てる（次の描きが文書から全部を作り直す。ほかのセットの絵も）。
    pub fn invalidate_paint(&mut self) {
        self.paint.invalidate();
        self.held.clear();
        self.current_bind = None;
    }

    /// メモリの予算が足りずに絵を持っていないセットのマテリアル（最後に描いたときの）。
    pub fn unpainted_materials(&self) -> &[i32] {
        &self.unpainted
    }

    /// 絵を作ってあるほかのセットのマテリアル（最後に描いたときの。作った順）。
    pub fn held_materials(&self) -> Vec<i32> {
        self.held
            .iter()
            .filter(|h| h.paint.is_built())
            .map(|h| h.material)
            .collect()
    }

    /// 試験用: ほかのセットの絵のチャンネルの 1 段の中身（`Paint::read_level`）と、縮めた段。そのマテリアルのセットの絵が無ければ None。
    pub fn read_other_level(
        &self,
        material: i32,
        slot: Slot,
        level: u32,
    ) -> Option<(Vec<u8>, [u32; 2], u32)> {
        let held = self
            .held
            .iter()
            .find(|h| h.material == material && h.paint.is_built())?;
        let (texels, size) = held.paint.read_level(slot, level)?;
        Some((texels, size, held.paint.level()))
    }

    /// 試験用: 塗った絵のチャンネルの 1 段の中身（`Paint::read_level`）。
    pub fn read_paint_level(&self, slot: Slot, level: u32) -> Option<(Vec<u8>, [u32; 2])> {
        self.paint.read_level(slot, level)
    }

    /// 試験用: 塗った絵の重みの絵の 1 段の中身（`Paint::read_weight_level`。1 テクセル 1 バイト）。
    pub fn read_paint_weight_level(&self, level: u32) -> Option<(Vec<u8>, [u32; 2])> {
        self.paint.read_weight_level(level)
    }

    /// 試験用: ほかのセットの絵の重みの絵の 1 段の中身。そのマテリアルのセットの絵が無ければ None。
    pub fn read_other_weight_level(
        &self,
        material: i32,
        level: u32,
    ) -> Option<(Vec<u8>, [u32; 2])> {
        self.held
            .iter()
            .find(|h| h.material == material && h.paint.is_built())?
            .paint
            .read_weight_level(level)
    }

    /// 試験用: 接線を作るスレッドが仕事の前に呼ぶ口（`TangentHook`）。
    pub fn set_tangent_hook(&mut self, hook: Option<TangentHook>) {
        self.tangent_hook = hook;
    }

    /// GPU の積みが終わるまで待つ（計測用: `prepare` が返した後も GPU は動いているので、フレームの本当の時間を測るときに）。
    pub fn wait_gpu(&self) {
        let _ = self.rs.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// 描いている GPU の名前と API（計測の記録用）。
    pub fn adapter_name(&self) -> String {
        let info = self.rs.adapter.get_info();
        format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type)
    }

    /// 接線を作っている最中で、今の表示がそれを読むか（出来たら描き直したいので、ウィンドウは次のフレームも要る）。光なしの表示へ替えた・法線マップを
    /// 使うレイヤーが無くなったときは、作っていても求めない（絵は変わらないのにウィンドウを回し続けない）。ほかのセットの絵を、1 フレームに作る数の上限で
    /// 待たせているあいだも求める。
    pub fn wants_repaint(&self) -> bool {
        (self.tangents_wanted && self.inflight.is_some()) || self.pending_builds > 0
    }

    /// 文書の変わった所を上げ、要れば描き直して、egui に貼るテクスチャを返す（size は物理の画素）。今のセットだけを見せる
    /// （`prepare_sets` にほかのセットを渡さない形）。
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        doc: &Document,
        model: &Arc<ViewModel>,
        material: i32,
        camera: &OrbitCamera,
        size: [u32; 2],
        display: &Display,
        map: Option<&MeshMapSource>,
    ) -> egui::TextureId {
        self.prepare_sets(doc, model, material, camera, size, display, map, &[])
    }

    /// `prepare` に、今のセットでないセット（`others`）の絵を足して見せる。今のセットは文書の大きさのまま、ほかのセットは縮めた段で
    /// 持ち（メモリの予算に入る。今のセットから近い順に、足りなければ遠いものから持たない）、そのセットの文書が変わったときだけ
    /// 上げ直す。
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_sets(
        &mut self,
        doc: &Document,
        model: &Arc<ViewModel>,
        material: i32,
        camera: &OrbitCamera,
        size: [u32; 2],
        display: &Display,
        map: Option<&MeshMapSource>,
        others: &[OtherSet<'_>],
    ) -> egui::TextureId {
        let started = std::time::Instant::now();
        let others = if self.show_others { others } else { &[] };
        let size = [size[0].max(1), size[1].max(1)];
        let mut encoder = self
            .rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("yolu-3d"),
            });
        self.sync_sets(doc, model, material, others, started, &mut encoder);
        self.sync_looks(doc, others, &mut encoder);
        let sync_us = started.elapsed().as_micros() as u64;
        self.sync_environment(display);
        self.sync_map(display, map, &mut encoder);
        let use_normal_map =
            (self.any_normal_map() || self.looks_want_tangents()) && !display.is_unlit();
        self.ensure_mesh(model, use_normal_map);
        let tone = display.uses_tone_map();
        let bloom = display.uses_bloom();
        let hdr = display.uses_hdr_path();
        let samples = self.plan_samples(display.post.antialias, size, hdr, bloom);
        self.set_samples(samples);
        let mut resized = self.ensure_target(size);
        resized |= self.ensure_scene(size, samples, hdr);
        // 仕上げ・ブルームを通さないあいだは、その描き先（HDR の 1 サンプル・ブルームの段）を持たない（メモリ）
        if hdr {
            self.ensure_hdr_target(size);
        } else {
            self.hdr_target = None;
        }
        if bloom {
            self.ensure_bloom(size);
        } else {
            self.bloom_chain = None;
        }
        self.ensure_shadow(display);
        self.ensure_bind();
        self.ensure_set_binds();
        self.overlay.sync(
            &mut encoder,
            self.overlay_allowed,
            hdr,
            samples,
            VERTEX_FLOATS,
        );
        let key = SceneKey {
            camera: [
                camera.target.x.to_bits(),
                camera.target.y.to_bits(),
                camera.target.z.to_bits(),
                camera.yaw.to_bits(),
                camera.pitch.to_bits(),
                camera.distance.to_bits(),
                u32::from(camera.is_orthographic()),
                match camera.projection {
                    Projection::Orthographic { height } => height.to_bits(),
                    Projection::Perspective => 0,
                },
            ],
            size,
            model: model.revision(),
            material,
            paint: self.paint_key(material),
            tangents: self.mesh.as_ref().map_or(0, |m| m.tangent_stamp),
            env: self.env.version,
            map: self.map_version,
            display: display.key_bits(),
            samples,
            looks: self.looks_key(),
            overlay: self.overlay.key(),
        };
        // 絵が変わればミップも鍵の版も変わるので、描かないフレームは何も積んでいない（出さずに捨てる）
        if resized || self.last_key != Some(key) {
            if display.uses_shadows() {
                self.update_shadow(model, display.light_direction(), &mut encoder);
            }
            self.draw_scene(
                &mut encoder,
                camera,
                size,
                display,
                use_normal_map,
                material,
            );
            self.last_key = Some(key);
            self.stats.renders += 1;
            if tone {
                self.stats.tone_mapped_renders += 1;
            }
            if bloom {
                self.stats.bloom_renders += 1;
            }
            self.rs.queue.submit(Some(encoder.finish()));
        }
        let paint: View3dStats = self.paint.stats.into();
        self.stats = View3dStats {
            renders: self.stats.renders,
            tone_mapped_renders: self.stats.tone_mapped_renders,
            bloom_renders: self.stats.bloom_renders,
            samples,
            target_bytes: target_bytes(size, samples, hdr, bloom),
            mesh_revision: self.stats.mesh_revision,
            mesh_uploads: self.stats.mesh_uploads,
            env_bakes: self.stats.env_bakes,
            shadow_renders: self.stats.shadow_renders,
            shadow_active: self.shadow_in_use(display),
            map_level: self.map.as_ref().map_or(0, |(_, t)| t.level()),
            tangents_exact: self.stats.tangents_exact,
            last_prepare_us: started.elapsed().as_micros() as u64,
            last_sync_us: sync_us,
            other_sets: self.held.iter().filter(|h| h.paint.is_built()).count(),
            other_skipped: self.unpainted.len(),
            other_pending: self.pending_builds,
            user_bytes: self.current_look.users.bytes(),
            received_bytes: self.current_look.received.bytes(),
            received_size: self.current_look.received.size().unwrap_or_default(),
            look_params_builds: self.current_look.params_builds(),
            user_level: self.current_look.users.level(),
            other_bytes: self
                .held
                .iter()
                .map(|h| h.paint.bytes() + h.look.bytes())
                .sum(),
            other_level: self.held.iter().map(|h| h.paint.level()).max().unwrap_or(0),
            other_demotions: self.demotions,
            peak_bytes: self.peak_bytes,
            linear_transparent: self.srgb_views,
            lil_pipelines: self.lil_pipelines.len(),
            lil_pipeline_builds: self.lil_pipeline_builds,
            overlay_bytes: self.overlay.bytes(),
            overlay_tiles: self.overlay.uploaded_tiles,
            overlay_skipped: self.overlay.skipped(),
            overlay_too_large: self.overlay.skipped() && self.overlay_too_large,
            other_scratch_bytes: self
                .held
                .iter()
                .map(|h| h.paint.scratch_bytes() as u64)
                .sum(),
            ..paint
        };
        self.target.as_ref().expect("作った").id
    }

    /// 今のセットとほかのセット（`others`）の絵を、文書に合わせる。
    ///
    /// 順番は、GPU のメモリが予算を超えて重ならないように決めてある: (1) 今のセットが替わったら前の絵を取り出す、(2) 新しい今のセットの
    /// 絵が取る分（`Paint::planned_bytes`）を予算から引いて、ほかのセットの持つ・持たないを決め、持たないものを捨てる、(3) 前の絵がまだ
    /// 見せるセット（`others` にある）なら、ほかのセットの持ち物へ回してその場で上限の大きさへ縮める（ミップの段をコピー。
    /// `Paint::demote`。文書を合成し直さない。辺が奇数などでコピーできないときだけ、その場で文書から縮めて作り直す）、(4) 新しい今のセットを
    /// 作る、(5) ほかのセットを文書に合わせる。ほかのセットは、今のセットから並びの上で近い順に、予算（全体から今のセットの絵を引いた
    /// 残り）に収まる分だけ持つ。収まらない（遠い）セットは持たず、その面は絵の無い描き方で、マテリアルを `unpainted` に残す。モデルの面が
    /// 1 つも無いマテリアル（隠した・全部の面を隠した）のセットは持たない。持ち物は文書が変わったとき（版・変化の記録）だけ同期し、新しく
    /// 作り始めるのは 1 フレームの時間（`BUILD_FRAME_BUDGET`）に収まるあいだと、そのフレームの最初の 1 つだけ。
    /// lilToon の見た目のセットが持つユーザーチャンネルの配列と、Live Link で Unity から受けた絵の配列も同じ予算に入る: 今のセットの配列は
    /// 標準のチャンネルの絵の残りから（ユーザーチャンネルは `user_budget`、受けた絵はその残りの `received_budget`。ほかのセットより先）、
    /// ほかのセットは標準のチャンネルの絵と 2 つの配列を合わせた分で持つ・持たないを決める。配列を作るのは `sync_looks`（今のセットが
    /// 替わったら、前のセットの配列はここで手放す）。
    fn sync_sets(
        &mut self,
        doc: &Document,
        model: &ViewModel,
        material: i32,
        others: &[OtherSet<'_>],
        started: std::time::Instant,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let id = doc.id();
        // 前のフレームの終わりに持っていた絵の分から数える（同期の途中の最大が、予算を超えていないかの記録）
        self.peak_bytes = self.picture_bytes();
        let mut previous: Option<u128> = None;
        if self.paint.doc_id().is_some_and(|d| d != id) {
            // 前のセットのユーザーチャンネルの配列は、新しい絵を作る前に手放す（前のセットをほかのセットとして持つなら、`sync_looks` が
            // ほかのセットの大きさで作り直す）
            self.current_look = self.current_look.sibling(&self.rs.device);
            let mut fresh = self.paint.sibling();
            fresh.set_budget(self.total_budget);
            fresh.stats = self.paint.stats;
            let old = std::mem::replace(&mut self.paint, fresh);
            self.current_bind = None;
            // まだ見せるセットなら、ほかのセットの持ち物へ（縮めるのは下。満量のままの間も、持っている絵の数に入る）。見せないなら、
            // ここで捨てる（GPU のメモリを返す）
            if let Some(old_id) = old
                .doc_id()
                .filter(|d| others.iter().any(|o| o.doc.id() == *d))
            {
                self.held.retain(|h| h.doc_id != old_id);
                self.held.push(HeldSet {
                    doc_id: old_id,
                    material: -1,
                    paint: old,
                    look: self.current_look.sibling(&self.rs.device),
                    bind: None,
                });
                previous = Some(old_id);
            }
        }
        // 今のセットになった文書は、ほかのセットとしては持たない
        self.held.retain(|h| h.doc_id != id);
        // 表示の写しの塗り広げの元（UV の形が変われば、続く計画と同期が全部を作り直す）
        let uv = |material: i32| UvSource {
            geometry: model.geometry.clone(),
            material,
        };
        self.paint.set_padding(Some(uv(material)));

        let shown = model_materials(model);
        let mut want: Vec<&OtherSet<'_>> = others
            .iter()
            .filter(|o| o.doc.id() != id && o.material != material && shown.contains(&o.material))
            .collect();
        want.sort_by_key(|o| o.rank);
        let mut remaining = self
            .total_budget
            .saturating_sub(self.paint.planned_bytes(doc));
        // 今のセットの lilToon のユーザーチャンネルの配列は、標準のチャンネルの絵の残りから（セットごとの上限まで）。ほかのセットより先
        let limit = self.user_limit();
        self.user_budget = remaining;
        remaining = remaining.saturating_sub(look_gpu::planned_user_bytes(
            doc,
            self.paint.planned_level(doc),
            limit,
            self.user_budget,
        ));
        // Unity から受けた絵の配列も、その残りから（セットごとの上限まで）
        self.received_budget = remaining;
        remaining = remaining.saturating_sub(look_gpu::planned_received_bytes(
            doc,
            limit,
            self.received_budget,
        ));
        // 選択範囲の重ね（文書と同じ大きさの R8。ミップ込み）も、ほかのセットより先に入れる。入らなければ重ねを出さない
        let overlay = self.overlay.wanted_bytes();
        self.overlay_too_large = self.overlay.too_large();
        self.overlay_allowed = !self.overlay_too_large && overlay <= remaining;
        if self.overlay_allowed {
            remaining -= overlay;
        }
        let mut keep: Vec<&OtherSet<'_>> = Vec::with_capacity(want.len());
        self.unpainted.clear();
        for o in want {
            // ほかのセットは、標準のチャンネルの絵とユーザーチャンネルの配列と受けた絵の配列を合わせて持つか持たないか（一部だけは持たない）
            let (shift, bytes) = self.paint.estimate(o.doc, self.other_cap);
            let other_limit = limit.min(self.other_cap);
            let need = bytes
                + look_gpu::planned_user_bytes(o.doc, shift, other_limit, USER_BUDGET_BYTES)
                + look_gpu::planned_received_bytes(o.doc, other_limit, RECEIVED_BUDGET_BYTES);
            if need <= remaining {
                remaining -= need;
                keep.push(o);
            } else {
                self.unpainted.push(o.material);
            }
        }
        // 持たなくなったものは、新しい今のセットの絵を作る前に捨てる（GPU のメモリを返す）
        self.held
            .retain(|h| keep.iter().any(|o| o.doc.id() == h.doc_id));
        let mut builds = 0;
        self.pending_builds = 0;
        // 前のセットの絵は、新しい絵を作る前に縮める（満量の前の絵と、満量の新しい絵が重ならないように）
        if let Some(old_id) = previous {
            // 持たないと決めたなら、上の「捨てる」で無くなっている
            if let Some(held) = self.held.iter_mut().find(|h| h.doc_id == old_id) {
                let o = keep
                    .iter()
                    .find(|o| o.doc.id() == old_id)
                    .expect("持つものだけが残っている");
                held.material = o.material;
                held.paint.set_budget(u64::MAX);
                held.paint.set_cap(Some(self.other_cap));
                held.paint.set_padding(Some(uv(o.material)));
                if held.paint.demote(o.doc) {
                    self.demotions += 1;
                } else if held.paint.needs_rebuild(o.doc) {
                    // コピーでは縮められない: 満量のまま残さず、その場で文書から縮めて作り直す（作り直しの枠に数える）
                    held.paint.sync(o.doc, encoder);
                    builds += 1;
                }
                // 今のセットだった間に抱えた作業用のバッファも手放す（ほかのセットになったら要らない）
                held.paint.release_scratch();
                self.note_peak();
            }
        }
        self.paint.sync(doc, encoder);
        self.note_peak();
        for o in keep {
            let at = match self.held.iter().position(|h| h.doc_id == o.doc.id()) {
                Some(at) => at,
                None => {
                    let mut paint = self.paint.sibling();
                    paint.set_budget(u64::MAX);
                    self.held.push(HeldSet {
                        doc_id: o.doc.id(),
                        material: o.material,
                        paint,
                        look: self.current_look.sibling(&self.rs.device),
                        bind: None,
                    });
                    self.held.len() - 1
                }
            };
            let held = &mut self.held[at];
            held.material = o.material;
            // ほかのセットは上限だけで縮める（予算は上の計画で見た。今のセットだった絵の予算の値を持ち越さない）
            held.paint.set_budget(u64::MAX);
            held.paint.set_cap(Some(self.other_cap));
            held.paint.set_padding(Some(uv(o.material)));
            if held.paint.demote(o.doc) {
                self.demotions += 1;
            }
            let rebuild = held.paint.needs_rebuild(o.doc);
            if !rebuild && held.paint.synced_with(o.doc) {
                continue;
            }
            if rebuild {
                if builds > 0 && started.elapsed() >= self.build_budget {
                    self.pending_builds += 1;
                    continue;
                }
                builds += 1;
            }
            held.paint.sync(o.doc, encoder);
            // 作業用のバッファは文書の大きさになる。ほかのセットが数に比例して抱えないように、作り終えたら手放す
            held.paint.release_scratch();
            self.note_peak();
        }
    }

    /// 今 GPU に持っている絵（今のセットとほかのセット。lilToon のユーザーチャンネルの配列と受けた絵の配列を含む）のバイト数。
    fn picture_bytes(&self) -> u64 {
        self.paint.bytes()
            + self.overlay.bytes()
            + self.current_look.bytes()
            + self
                .held
                .iter()
                .map(|h| h.paint.bytes() + h.look.bytes())
                .sum::<u64>()
    }

    /// ユーザーチャンネルの配列の辺の上限（GPU の上限と 8192 の小さいほう）。
    fn user_limit(&self) -> u32 {
        self.rs.device.limits().max_texture_dimension_2d.min(8192)
    }

    /// 同期の途中で持っていた絵のバイト数の最大を更新する。
    fn note_peak(&mut self) {
        self.peak_bytes = self.peak_bytes.max(self.picture_bytes());
    }

    /// どれかのセットの絵が Normal を使っているか（法線マップの接線を作る・読むかの全体の決め）。
    fn any_normal_map(&self) -> bool {
        self.paint.has(Slot::Normal)
            || self
                .held
                .iter()
                .any(|h| h.paint.is_built() && h.paint.has(Slot::Normal))
    }

    /// 全部のセットの絵の鍵（絵の中身・作りの世代・持ち主・マテリアルのどれかが変わると変わる。描き直しの鍵）。
    fn paint_key(&self, material: i32) -> u64 {
        let mix = |a: u64, b: u64| (a ^ b).wrapping_mul(0x0000_0100_0000_01b3);
        let one = |k: u64, p: &Paint, material: i32| {
            [
                p.uid(),
                p.version(),
                p.layout_version(),
                material as u32 as u64,
                u64::from(p.is_built()),
            ]
            .into_iter()
            .fold(k, mix)
        };
        let mut key = one(0xcbf2_9ce4_8422_2325, &self.paint, material);
        for h in &self.held {
            key = one(key, &h.paint, h.material);
        }
        key
    }

    /// セットごとの束ね（group 1）を、絵の作りが替わったときだけ作り直す。
    fn ensure_set_binds(&mut self) {
        let device = self.rs.device.clone();
        let layout = &self.set_layout;
        let white = &self.white_view;
        let stale = |bind: &Option<SetBind>, paint: &Paint, look: &LookGpu| {
            bind.as_ref().is_none_or(|b| {
                b.uid != paint.uid()
                    || b.layout_version != paint.layout_version()
                    || b.look != look.bind_key()
            })
        };
        if stale(&self.current_bind, &self.paint, &self.current_look) {
            self.current_bind = Some(SetBind {
                bind: make_set_bind(
                    &device,
                    layout,
                    &self.paint,
                    true,
                    true,
                    &self.current_look,
                    white,
                ),
                uid: self.paint.uid(),
                layout_version: self.paint.layout_version(),
                look: self.current_look.bind_key(),
            });
        }
        for h in &mut self.held {
            if h.paint.is_built() && stale(&h.bind, &h.paint, &h.look) {
                h.bind = Some(SetBind {
                    bind: make_set_bind(&device, layout, &h.paint, true, false, &h.look, white),
                    uid: h.paint.uid(),
                    layout_version: h.paint.layout_version(),
                    look: h.look.bind_key(),
                });
            }
        }
    }

    /// 見た目の設定を GPU へ（今のセットとほかのセット。値・ユーザーチャンネルの配列・マットキャップの絵）。ほかのセットは、
    /// 絵を持っているものだけ（`others` から文書を引く）。
    fn sync_looks(
        &mut self,
        doc: &Document,
        others: &[OtherSet<'_>],
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let queue = self.rs.queue.clone();
        let limit = self.user_limit();
        let budget = LookBudget {
            limit,
            users: self.user_budget,
            received: self.received_budget,
        };
        self.current_look
            .sync(doc, &self.paint, budget, &queue, encoder);
        self.note_peak();
        // ほかのセットの配列の大きさは、持つかどうかの計画（`sync_sets`）と同じ上限・予算で決まる
        let other_limit = limit.min(self.other_cap);
        for i in 0..self.held.len() {
            let h = &mut self.held[i];
            if !h.paint.is_built() {
                continue;
            }
            if let Some(o) = others.iter().find(|o| o.doc.id() == h.doc_id) {
                let budget = LookBudget {
                    limit: other_limit,
                    users: USER_BUDGET_BYTES,
                    received: RECEIVED_BUDGET_BYTES,
                };
                h.look.sync(o.doc, &h.paint, budget, &queue, encoder);
                self.note_peak();
            }
        }
    }

    /// 全部のセットの見た目の鍵（描き直しの鍵）。
    fn looks_key(&self) -> u64 {
        let mut key = self.current_look.key();
        for h in &self.held {
            key =
                (key ^ h.look.key() ^ h.material as u32 as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        key
    }

    /// どれかのセットが lilToon のノーマルマップを使うか（接線を作る）。
    fn looks_want_tangents(&self) -> bool {
        self.current_look.wants_tangents
            || self
                .held
                .iter()
                .any(|h| h.paint.is_built() && h.look.wants_tangents)
    }

    /// このマテリアルの面の描き方（今のセット・ほかのセット・絵の無い面は標準）。
    fn draw_of(&self, pick: Pick) -> SetDraw {
        match pick {
            Pick::Current => self.current_look.draw,
            Pick::Held(i) => self.held[i].look.draw,
            Pick::Blank => SetDraw::STANDARD,
        }
    }

    /// このマテリアルの面を描くときの束ね（今のセットの絵・ほかのセットの絵・絵の無い描き方）。
    fn pick(&self, material: i32, current: i32) -> Pick {
        if current >= 0 && material == current {
            return Pick::Current;
        }
        match self
            .held
            .iter()
            .position(|h| h.material == material && h.paint.is_built() && h.bind.is_some())
        {
            Some(i) => Pick::Held(i),
            None => Pick::Blank,
        }
    }

    /// 描き先を大きさに合わせる（作り直したら true）。
    fn ensure_target(&mut self, size: [u32; 2]) -> bool {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return false;
        }
        let device = &self.rs.device;
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-color"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LDR,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: if self.srgb_views { &[LDR_SRGB] } else { &[] },
        });
        let view = color.create_view(&Default::default());
        let srgb_view = self.srgb_views.then(|| {
            color.create_view(&wgpu::TextureViewDescriptor {
                format: Some(LDR_SRGB),
                ..Default::default()
            })
        });
        let mut renderer = self.rs.renderer.write();
        // 表示域と描き先は同じ画素の数なので、補間しない
        let id = match &self.target {
            Some(old) => {
                renderer.update_egui_texture_from_wgpu_texture(
                    device,
                    &view,
                    wgpu::FilterMode::Nearest,
                    old.id,
                );
                old.id
            }
            None => renderer.register_native_texture(device, &view, wgpu::FilterMode::Nearest),
        };
        drop(renderer);
        self.target = Some(Target {
            view,
            srgb_view,
            size,
            id,
        });
        true
    }

    /// 面を描く深度と（サンプル数が 2 以上なら）多サンプルの色を、大きさ・サンプル数・色の形式に合わせる（作り直したら true）。
    /// 作り直す前に前のものを手放す（描き先のメモリが、前と新しいので重ならない）。
    fn ensure_scene(&mut self, size: [u32; 2], samples: u32, hdr: bool) -> bool {
        // 多サンプルの色の形式は、仕上げを通すときだけ HDR。サンプル数 1 では色を持たないので形式は関係ない
        let hdr = hdr && samples > 1;
        if self
            .scene
            .as_ref()
            .is_some_and(|b| b.size == size && b.samples == samples && b.hdr == hdr)
        {
            return false;
        }
        self.scene = None;
        let device = &self.rs.device;
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth = depth_texture.create_view(&Default::default());
        let color = (samples > 1).then(|| {
            let srgb = !hdr && self.srgb_views;
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("yolu-3d-color-msaa"),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: if hdr { HDR } else { LDR },
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: if srgb { &[LDR_SRGB] } else { &[] },
            });
            let view = texture.create_view(&Default::default());
            let srgb_view = srgb.then(|| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    format: Some(LDR_SRGB),
                    ..Default::default()
                })
            });
            MsaaColor {
                _texture: texture,
                view,
                srgb_view,
            }
        });
        self.scene = Some(SceneBuffers {
            size,
            samples,
            hdr,
            depth,
            _depth_texture: depth_texture,
            color,
        });
        true
    }

    /// 面と背景・lilToon のパイプラインを、描き先のサンプル数に合わせる（数が変わったときだけ作り直す）。
    fn set_samples(&mut self, samples: u32) {
        if samples == self.samples {
            return;
        }
        let device = &self.rs.device;
        let (module, scene, background) = (
            &self.scene_module,
            &self.scene_pipeline_layout,
            &self.background_pipeline_layout,
        );
        self.ldr = build_pipelines(device, module, scene, background, LDR, samples);
        self.hdr = build_pipelines(device, module, scene, background, HDR, samples);
        // lilToon のパイプラインはサンプル数が違えば使えない（使うときに作る）
        self.lil_pipelines.clear();
        self.samples = samples;
    }

    /// ブルームのパイプライン（初めて使うときに作る）と、段のテクスチャを、絵の大きさに合わせる。
    fn ensure_bloom(&mut self, size: [u32; 2]) {
        if self.bloom.is_none() {
            self.bloom = Some(self.build_bloom());
        }
        if self.bloom_chain.as_ref().is_some_and(|c| c.size == size) {
            return;
        }
        self.bloom_chain = None;
        let levels = bloom_levels(size);
        let texture = self.rs.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-bloom"),
            size: wgpu::Extent3d {
                width: levels[0][0],
                height: levels[0][1],
                depth_or_array_layers: 1,
            },
            mip_level_count: levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let views = (0..levels.len() as u32)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        self.bloom_chain = Some(BloomChain {
            _texture: texture,
            views,
            size,
        });
    }

    fn build_bloom(&self) -> BloomPipes {
        let device = &self.rs.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yolu-3d-bloom"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/bloom.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yolu-3d-bloom"),
            entries: &[
                texture_entry(0, wgpu::TextureViewDimension::D2),
                sampler_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yolu-3d-bloom"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |label: &str, entry: &str, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR,
                        blend,
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // 拡大は 1 つ大きい段へ加算で重ねる
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        BloomPipes {
            prefilter: make("yolu-3d-bloom-prefilter", "fs_prefilter", None),
            down: make("yolu-3d-bloom-down", "fs_down", None),
            up: make("yolu-3d-bloom-up", "fs_up", Some(add)),
            layout,
            params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("yolu-3d-bloom"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }

    /// 仕上げ（ブルーム・トーンマッピング）のための HDR の描き先（解決後の 1 サンプル。大きさに合わせる）。
    fn ensure_hdr_target(&mut self, size: [u32; 2]) {
        if self.hdr_target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        self.hdr_target = None;
        let texture = self.rs.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-hdr"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        self.hdr_target = Some(HdrTarget {
            _texture: texture,
            view,
            size,
        });
    }

    /// 環境を焼く（種類か空の色が変わったときだけ。回転と明るさは焼かずに使う所で掛ける）。
    fn sync_environment(&mut self, display: &Display) {
        if display.env == EnvKind::None {
            return;
        }
        let colors = display.sky;
        let key = (
            display.env,
            [
                colors.zenith[0],
                colors.zenith[1],
                colors.zenith[2],
                colors.horizon[0],
                colors.horizon[1],
                colors.horizon[2],
                colors.ground[0],
                colors.ground[1],
                colors.ground[2],
            ],
        );
        if self.env.baked_for == Some(key) {
            return;
        }
        let source = match display.env {
            EnvKind::Studio => Source::Studio,
            _ => Source::Sky(colors),
        };
        let baked = environment::bake(&source);
        let texture = self.rs.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-env"),
            size: wgpu::Extent3d {
                width: FACE_SIZE,
                height: FACE_SIZE,
                depth_or_array_layers: 6,
            },
            mip_level_count: MIP_COUNT,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for mip in 0..MIP_COUNT {
            let size = (FACE_SIZE >> mip).max(1);
            self.rs.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &baked.mip_bytes(mip as usize),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 8),
                    rows_per_image: Some(size),
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 6,
                },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        self.env.cube = Some(EnvCube {
            view,
            _texture: texture,
        });
        self.env.baked = Some(baked);
        self.env.baked_for = Some(key);
        self.env.version += 1;
        self.stats.env_bakes += 1;
    }

    /// メッシュマップだけを見せるとき、その絵を作る（同じマップなら作り直さない）。
    fn sync_map(
        &mut self,
        display: &Display,
        source: Option<&MeshMapSource>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let (Shading::MeshMap(_), Some(source)) = (display.shading, source) else {
            return;
        };
        if self.map.as_ref().is_some_and(|(k, _)| *k == source.key)
            || self.map_failed == Some(source.key)
        {
            return;
        }
        let rgba = source.map.to_rgba8(false);
        let size = [source.map.width() as u32, source.map.height() as u32];
        // メッシュマップはガンマの値のまま見せる（光なしの表示）ので、リニアにしない形式で
        match self.paint.create_image(&rgba, size, false, encoder) {
            Some(texture) => {
                self.map = Some((source.key, texture));
                self.map_failed = None;
            }
            None => {
                // 作れない絵（大きさ 0 など）は覚えて、同じ絵では毎フレーム作り直さない
                self.map = None;
                self.map_failed = Some(source.key);
            }
        }
        self.map_version += 1;
    }

    /// 束ね（group 0: 環境のキューブ・メッシュマップ・影。面ごとの絵は `ensure_set_binds`）を、変わったときだけ作り直す。
    fn ensure_bind(&mut self) {
        let key = (self.env.version, self.map_version, self.shadow_version);
        if self.bind.as_ref().is_some_and(|(_, k)| *k == key) {
            return;
        }
        let cube = self.env.cube.as_ref().map_or(&self.dummy_cube, |c| &c.view);
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: self.uniforms.as_entire_binding(),
        }];
        entries.push(wgpu::BindGroupEntry {
            binding: 7,
            resource: wgpu::BindingResource::Sampler(&self.paint_sampler),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 8,
            resource: wgpu::BindingResource::TextureView(cube),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 9,
            resource: wgpu::BindingResource::Sampler(&self.env_sampler),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 10,
            resource: wgpu::BindingResource::TextureView(
                self.map
                    .as_ref()
                    .map_or(self.paint.blank_view(), |(_, t)| t.view()),
            ),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 11,
            resource: wgpu::BindingResource::TextureView(
                self.shadow.as_ref().map_or(&self.dummy_shadow, |s| &s.view),
            ),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 12,
            resource: wgpu::BindingResource::Sampler(&self.shadow_sampler),
        });
        let bind = self
            .rs
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("yolu-3d-scene"),
                layout: &self.scene_layout,
                entries: &entries,
            });
        self.bind = Some((bind, key));
    }

    /// 法線マップに使う接線を決める（出来ていればそれ。出来ていなければ別のスレッドで作り、出来るまでは前の接線）。
    fn pick_tangents(
        &mut self,
        model: &Arc<ViewModel>,
        corners: usize,
    ) -> (Option<Arc<Vec<Tangent>>>, bool) {
        if let Some(f) = &self.inflight {
            if let Some(t) = f.model.tangents_ready() {
                self.last_tangents = Some(t.clone());
                self.inflight = None;
            } else if f.worker.is_finished() {
                // 接線を残さずにスレッドが終わった（作業の失敗）。そのモデルは作り直さず、法線マップは読まない
                self.tangents_failed = Some(f.model.revision());
                self.inflight = None;
            }
        }
        if let Some(t) = model.tangents_ready() {
            self.last_tangents = Some(t.clone());
            return (Some(t.clone()), true);
        }
        if self.inflight.is_none() && self.tangents_failed != Some(model.revision()) {
            let worker = model.clone();
            let hook = self.tangent_hook.clone();
            self.inflight = Some(Inflight {
                model: model.clone(),
                worker: std::thread::spawn(move || {
                    if let Some(hook) = hook {
                        hook();
                    }
                    worker.tangents();
                }),
            });
        }
        let stale = self.last_tangents.clone().filter(|t| t.len() == corners);
        (stale, false)
    }

    /// モデルの頂点を上げる（変わったときだけ）。法線マップを使うときは接線も。頂点はマテリアル順（同じマテリアルはモデルの並びの順）に
    /// 並べて、マテリアルごとの範囲を覚える（セットごとに束ねを替えて描くため）。接線の番号は、モデルの角の並びのまま読む。
    fn ensure_mesh(&mut self, model: &Arc<ViewModel>, use_normal_map: bool) {
        let key = model.revision();
        let triangles: usize = model.meshes.iter().map(|m| m.triangle_count()).sum();
        let corners = triangles * 3;
        self.tangents_wanted = use_normal_map;
        let (tangents, exact) = if use_normal_map {
            self.pick_tangents(model, corners)
        } else {
            (None, true)
        };
        let stamp = tangents.as_ref().map_or(0, |t| Arc::as_ptr(t) as usize);
        if self
            .mesh
            .as_ref()
            .is_some_and(|m| m.model == key && m.tangent_stamp == stamp)
        {
            self.stats.tangents_exact = exact;
            return;
        }
        // サブメッシュをマテリアル順に（安定。角の番号 = 接線の番号は、モデルの並びで数える）
        let normals: Vec<Vec<yolu_core::glam::Vec3>> =
            model.meshes.iter().map(|m| m.vertex_normals()).collect();
        let mut subs: Vec<(i32, usize, usize, usize)> = Vec::new();
        let mut corner = 0usize;
        for (mi, mesh) in model.meshes.iter().enumerate() {
            for (si, s) in mesh.submeshes.iter().enumerate() {
                subs.push((s.material, mi, si, corner));
                corner += s.indices.len();
            }
        }
        subs.sort_by_key(|(material, ..)| *material);
        let mut data: Vec<u8> = Vec::with_capacity(corners * VERTEX_FLOATS * 4);
        let mut ranges: Vec<MaterialRange> = Vec::new();
        for (material, mi, si, first) in subs {
            let mesh = &model.meshes[mi];
            let s = &mesh.submeshes[si];
            let start = (data.len() / (VERTEX_FLOATS * 4)) as u32;
            for (k, &i) in s.indices.iter().enumerate() {
                let i = i as usize;
                let p = mesh.positions[i];
                let n = normals[mi][i];
                let uv = mesh.uvs.get(i).copied().unwrap_or_default();
                // 接線が無いとき（法線マップを使わない・まだ作っている）は 0。シェーダーは `mode.w` で読まない
                let t = tangents
                    .as_ref()
                    .and_then(|t| t.get(first + k))
                    .copied()
                    .unwrap_or([0.0; 4]);
                for v in [
                    p.x, p.y, p.z, n.x, n.y, n.z, uv.x, uv.y, t[0], t[1], t[2], t[3],
                ] {
                    data.extend_from_slice(&v.to_le_bytes());
                }
            }
            let count = s.indices.len() as u32;
            match ranges.last_mut() {
                Some(r) if r.material == material => r.count += count,
                _ => ranges.push(MaterialRange {
                    material,
                    start,
                    count,
                }),
            }
        }
        let buffer = self
            .rs
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("yolu-3d-mesh"),
                contents: &data,
                usage: wgpu::BufferUsages::VERTEX,
            });
        self.mesh = Some(GpuMesh {
            buffer,
            vertices: (data.len() / (VERTEX_FLOATS * 4)) as u32,
            model: key,
            tangent_stamp: stamp,
            ranges,
        });
        self.stats.mesh_revision = key;
        self.stats.mesh_uploads += 1;
        self.stats.tangents_exact = exact;
    }

    /// 影を使っていて、そのマップが描けているか。
    fn shadow_in_use(&self, display: &Display) -> bool {
        display.uses_shadows() && self.shadow.as_ref().is_some_and(|s| s.drawn.is_some())
    }

    /// 影を初めて使うときに、影のマップ（深さ）を作る。使わないあいだは 1 × 1 の深さを束ねておくので、16 MiB は要らない。
    fn ensure_shadow(&mut self, display: &Display) {
        if !display.uses_shadows() || self.shadow.is_some() {
            return;
        }
        let device = &self.rs.device;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-shadow-map"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let matrix_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yolu-3d-shadow-matrix"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yolu-3d-shadow"),
            layout: &self.shadow_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: matrix_buffer.as_entire_binding(),
            }],
        });
        self.shadow = Some(ShadowMap {
            _texture: texture,
            view,
            matrix_buffer,
            bind,
            drawn: None,
            world_to_shadow: Mat4::IDENTITY,
            diameter: 1.0,
        });
        self.shadow_version += 1;
    }

    /// モデルの外接球（中心・半径）。世代が同じなら前の値。
    fn model_bounds(&mut self, model: &ViewModel) -> (Vec3, f32) {
        let key = model.revision();
        if let Some((k, c, r)) = self.bounds {
            if k == key {
                return (c, r);
            }
        }
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for mesh in &model.meshes {
            for p in mesh.positions.iter().filter(|p| p.is_finite()) {
                min = min.min(*p);
                max = max.max(*p);
            }
        }
        let (center, radius) = if min.cmple(max).all() {
            ((min + max) * 0.5, (max - min).length() * 0.5)
        } else {
            (Vec3::ZERO, 1.0)
        };
        // 外接球より少し大きく（Unity 版と同じ 2%）。0 に潰れた形でも行列が壊れないように下を押さえる
        let radius = (radius * 1.02).max(1e-4);
        self.bounds = Some((key, center, radius));
        (center, radius)
    }

    /// 光から見た深さを描く（光の向き・モデルの世代が変わったときだけ。光のカメラはモデルの外接球に合わせた平行投影）。
    fn update_shadow(
        &mut self,
        model: &Arc<ViewModel>,
        to_light: Vec3,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let Some(mesh) = self.mesh.as_ref().filter(|m| m.vertices > 0) else {
            return;
        };
        let key = ShadowKey {
            model: mesh.model,
            vertices: mesh.vertices,
            light: [
                to_light.x.to_bits(),
                to_light.y.to_bits(),
                to_light.z.to_bits(),
            ],
        };
        if self.shadow.as_ref().is_none_or(|s| s.drawn == Some(key)) {
            return;
        }
        let (center, radius) = self.model_bounds(model);
        let shadow = self.shadow.as_mut().expect("確かめた");
        let mesh = self.mesh.as_ref().expect("確かめた");
        shadow.diameter = radius * 2.0;
        shadow.world_to_shadow = shadow_matrix(center, radius, to_light);
        let bytes: Vec<u8> = shadow
            .world_to_shadow
            .to_cols_array()
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        self.rs.queue.write_buffer(&shadow.matrix_buffer, 0, &bytes);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("yolu-3d-shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &shadow.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &shadow.bind, &[]);
            pass.set_vertex_buffer(0, mesh.buffer.slice(..));
            pass.draw(0..mesh.vertices, 0..1);
        }
        shadow.drawn = Some(key);
        self.stats.shadow_renders += 1;
    }

    /// シェーダーへ渡す値（`shaders/scene.wgsl` の `Uniforms`）。
    fn uniform_bytes(
        &self,
        camera: &OrbitCamera,
        size: [u32; 2],
        display: &Display,
        use_normal_map: bool,
    ) -> Vec<u8> {
        let view = camera.view(size[0] as f32, size[1] as f32);
        let view_proj = view.view_projection();
        let mut f: Vec<f32> = Vec::with_capacity(UNIFORM_BYTES as usize / 4);
        f.extend_from_slice(&view_proj.to_cols_array());
        f.extend_from_slice(&view_proj.inverse().to_cols_array());
        // w: 正投影なら 1（lilToon の `lilIsPerspective()` が偽になる所と背景の向き）
        let p = view.position;
        f.extend_from_slice(&[p.x, p.y, p.z, f32::from(view.is_orthographic())]);
        let l = display.light_direction();
        f.extend_from_slice(&[l.x, l.y, l.z, 0.0]);
        // マテリアル表示の光（Unity のディレクショナルライトの `_LightColor0` と同じ値。`Display::direct_light`）と、環境が無いときの
        // 一様な環境光（色 × 0.4 をリニアへ）
        let direct = display.direct_light();
        f.extend_from_slice(&[direct[0], direct[1], direct[2], 0.0]);
        let flat = display.ambient.map(|c| brdf::srgb_to_linear(c * 0.4));
        f.extend_from_slice(&[flat[0], flat[1], flat[2], 0.0]);
        // 中立の表示（Unity 版: 光 0.65 × 強さ × 色、環境光 0.7 × 色。ガンマの空間）
        let neutral_light = display
            .light_color
            .map(|c| 0.65 * display.light_intensity * c);
        f.extend_from_slice(&[neutral_light[0], neutral_light[1], neutral_light[2], 0.0]);
        let neutral_ambient = display.ambient.map(|c| 0.7 * c);
        f.extend_from_slice(&[
            neutral_ambient[0],
            neutral_ambient[1],
            neutral_ambient[2],
            0.0,
        ]);
        let env_on =
            display.env != EnvKind::None && self.env.baked.is_some() && !display.is_unlit();
        f.extend_from_slice(&[
            f32::from(env_on),
            display.env_intensity,
            brdf::mip_of_roughness(brdf::NEUTRAL_REFLECTION_ROUGHNESS),
            0.0,
        ]);
        let radians = display.env_rotation.to_radians();
        let bg_mip = display.env_blur.clamp(0.0, 1.0) * brdf::REFLECTION_STEPS as f32;
        f.extend_from_slice(&[radians.cos(), radians.sin(), bg_mip, 0.0]);
        let (mode, channel) = match display.shading {
            Shading::Material => (0.0, 0.0),
            Shading::Neutral => (1.0, 0.0),
            Shading::Channel(c) => (2.0, Slot::of(c).map_or(0, |s| s.index()) as f32),
            Shading::MeshMap(_) => (3.0, 0.0),
        };
        let tangents_ready = self.mesh.as_ref().is_some_and(|m| m.tangent_stamp != 0);
        f.extend_from_slice(&[
            mode,
            channel,
            f32::from(use_normal_map),
            f32::from(tangents_ready),
        ]);
        let sh = self.env.baked.as_ref().map_or([Vec3::ZERO; 9], |b| b.sh);
        for c in sh {
            f.extend_from_slice(&[c.x, c.y, c.z, 0.0]);
        }
        // 影: 世界 → (u, v, 深さ) と、読み方。radius と normal_offset（影の 1 画素の世界の大きさ × 1.5）は Unity 版の PreviewShadowMap.Apply と同じ値。
        // 深さの偏りは別の方式: Unity 版は (1.5 + radius × 大きさ × 0.35) / 大きさの定数、こちらは 1.5 画素ぶんに面の傾きの分をシェーダーが足す
        let (matrix, params) = match self.shadow.as_ref().filter(|_| self.shadow_in_use(display)) {
            Some(s) => {
                let radius = lerp(
                    0.75 / SHADOW_SIZE as f32,
                    0.025,
                    display.shadow_softness.clamp(0.0, 1.0),
                );
                let size = SHADOW_SIZE as f32;
                // 深さの偏りの基本は 1.5 画素ぶん（面の傾きに応じた分はシェーダーが足す）
                let bias = 1.5 / size;
                let normal_offset = s.diameter / size * 1.5;
                (s.world_to_shadow, [1.0, radius, bias, normal_offset])
            }
            None => (Mat4::IDENTITY, [0.0; 4]),
        };
        f.extend_from_slice(&matrix.to_cols_array());
        f.extend_from_slice(&params);
        // lilToon の光: 環境の SH（回転・明るさ込み）か一様な環境光を、Unity の unity_SH* の形で。カメラの上と手前への向き
        let lil_sh = if env_on {
            let baked = self.env.baked.as_ref().map_or([Vec3::ZERO; 9], |b| b.sh);
            let rotated = super::look_gpu::rotate_sh_y(&baked, radians);
            rotated.map(|c| c * display.env_intensity)
        } else {
            let mut flat_sh = [Vec3::ZERO; 9];
            flat_sh[0] = Vec3::from(flat);
            flat_sh
        };
        for c in super::look_gpu::unity_sh(&lil_sh) {
            f.extend_from_slice(&c);
        }
        let up = view.rotation * Vec3::Y;
        let front = -view.forward;
        f.extend_from_slice(&[up.x, up.y, up.z, 0.0]);
        f.extend_from_slice(&[front.x, front.y, front.z, 0.0]);
        debug_assert_eq!(f.len() * 4, UNIFORM_BYTES as usize);
        f.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// lilToon のパイプライン（無ければ作る）。輪郭線は前面を捨てる（lilToon の `_OutlineCull` の既定）・深さは Less（`_OutlineZTest` の既定）、
    /// 半透明の面は乗算済みで重ね（`One`・`OneMinusSrcAlpha`）、半透明の輪郭線は `SrcAlpha`・`OneMinusSrcAlpha`（lilToon の設定の既定）。
    /// どれも深さを書く（lilToon の半透明も `_ZWrite` は 1）。
    fn ensure_lil_pipeline(&mut self, key: LilPipe) {
        if self.lil_pipelines.contains_key(&key) {
            return;
        }
        let device = &self.rs.device;
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4
        ];
        let cull = match key.cull {
            0 => None,
            1 => Some(wgpu::Face::Front),
            _ => Some(wgpu::Face::Back),
        };
        let blend = match (key.transparent, key.outline) {
            (false, _) => None,
            (true, false) => Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            (true, true) => Some(wgpu::BlendState::ALPHA_BLENDING),
        };
        // 全部入り（実機）は定数を渡さない（シェーダーの既定の全部入り）
        let constants: Vec<(&str, f64)> = if key.spec == look_gpu::LilSpec::ALL {
            Vec::new()
        } else {
            key.spec.constants().to_vec()
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(if key.outline {
                "yolu-3d-liltoon-outline"
            } else {
                "yolu-3d-liltoon"
            }),
            layout: Some(&self.scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.scene_module,
                entry_point: Some(if key.outline { "vs_outline" } else { "vs_main" }),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &constants,
                    ..Default::default()
                },
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: (VERTEX_FLOATS * 4) as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: cull,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(if key.outline {
                    wgpu::CompareFunction::Less
                } else {
                    wgpu::CompareFunction::LessEqual
                }),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: multisample(key.samples),
            fragment: Some(wgpu::FragmentState {
                module: &self.scene_module,
                entry_point: Some(match (key.outline, key.srgb) {
                    (false, false) => "fs_liltoon",
                    (false, true) => "fs_liltoon_linear",
                    (true, false) => "fs_outline",
                    (true, true) => "fs_outline_linear",
                }),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &constants,
                    ..Default::default()
                },
                targets: &[Some(wgpu::ColorTargetState {
                    format: if key.srgb {
                        LDR_SRGB
                    } else if key.hdr {
                        HDR
                    } else {
                        LDR
                    },
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.lil_pipelines.insert(key, pipeline);
        self.lil_pipeline_builds += 1;
    }

    fn draw_scene(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        camera: &OrbitCamera,
        size: [u32; 2],
        display: &Display,
        use_normal_map: bool,
        current_material: i32,
    ) {
        let bytes = self.uniform_bytes(camera, size, display, use_normal_map);
        self.rs.queue.write_buffer(&self.uniforms, 0, &bytes);
        // 仕上げ（ブルーム・露出・トーンマッピング）を通すときは、HDR の描き先へ描く
        let hdr_path = display.uses_hdr_path();
        let samples = self.samples;
        // マテリアルごとの描き（範囲・束ね・描き方）。隣り合うマテリアルが同じ束ね（絵の無いもの）なら 1 回にまとめる
        let lil_on = display.shading == Shading::Material;
        let mut draws: Vec<(u32, u32, Pick, SetDraw)> = Vec::new();
        if let Some(mesh) = &self.mesh {
            let mut i = 0;
            while i < mesh.ranges.len() {
                let pick = self.pick(mesh.ranges[i].material, current_material);
                let start = mesh.ranges[i].start;
                let mut end = start + mesh.ranges[i].count;
                i += 1;
                while i < mesh.ranges.len()
                    && pick == Pick::Blank
                    && self.pick(mesh.ranges[i].material, current_material) == pick
                {
                    end += mesh.ranges[i].count;
                    i += 1;
                }
                let mut draw = if lil_on {
                    self.draw_of(pick)
                } else {
                    SetDraw::STANDARD
                };
                // 実機は全部入りの 1 本（入切のたびにパイプラインを作り直さない）。ソフトの描画だけ使う機能で作る
                if !self.software {
                    draw.spec = look_gpu::LilSpec::ALL;
                }
                if draw.lil && draw.invisible {
                    continue;
                }
                draws.push((start, end, pick, draw));
            }
        }
        // 半透明はリニアで重ねる（Unity と同じ）。8 bit の描き先なら、2 つ目のパスで sRGB の見え方へ描く。HDR の描き先（トーンマッピング）と、
        // sRGB の見え方を作れない機材（GL）は、ガンマの値のまま同じパスで重ねる
        let is_transparent =
            |d: &SetDraw| d.lil && d.mode == super::look_gpu::RenderModeAlias::Transparent;
        let linear_blend = !hdr_path && self.srgb_views;
        // 要る lilToon のパイプラインを先に作る（描きのパスの中では作れない）
        let mut needed: Vec<LilPipe> = Vec::new();
        for (_, _, _, d) in &draws {
            if !d.lil {
                continue;
            }
            let transparent = is_transparent(d);
            let srgb = transparent && linear_blend;
            needed.push(LilPipe {
                hdr: hdr_path,
                srgb,
                cull: d.cull,
                transparent,
                outline: false,
                spec: d.spec,
                samples,
            });
            if d.outline {
                needed.push(LilPipe {
                    hdr: hdr_path,
                    srgb,
                    cull: 1,
                    transparent,
                    outline: true,
                    spec: d.spec,
                    samples,
                });
            }
        }
        // ソフトの描画は機能とスロットの読み方ごとに作るので、溜まりすぎたら今のフレームで使わないものを捨てる
        if self.lil_pipelines.len() + needed.len() > LIL_PIPELINES_KEPT {
            self.lil_pipelines.retain(|key, _| needed.contains(key));
        }
        for key in needed {
            self.ensure_lil_pipeline(key);
        }
        let any_transparent = draws.iter().any(|(_, _, _, d)| is_transparent(d));
        let target = self.target.as_ref().expect("作った");
        let scene = self.scene.as_ref().expect("作った");
        // 解決後の 1 サンプルの絵（仕上げを通すなら HDR、そうでなければ egui が読む 8 bit）と、面を描く先（多サンプルならその色）
        let (resolved, pipelines) = if hdr_path {
            let hdr = self.hdr_target.as_ref().expect("作った");
            (&hdr.view, &self.hdr)
        } else {
            (&target.view, &self.ldr)
        };
        let color_view = scene.color.as_ref().map_or(resolved, |c| &c.view);
        let bind = &self.bind.as_ref().expect("作った").0;
        let set_bind = |pick: Pick| {
            match pick {
                Pick::Current => self.current_bind.as_ref().map(|b| &b.bind),
                Pick::Held(at) => self.held[at].bind.as_ref().map(|b| &b.bind),
                Pick::Blank => None,
            }
            .unwrap_or(&self.blank_bind)
        };
        let lil = |key: LilPipe| self.lil_pipelines.get(&key).expect("先に作った");
        // 半透明を別のパスで描くときは、深さを残す
        let second_pass = any_transparent && linear_blend;
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("yolu-3d-scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    // 多サンプルは、パスの終わりに解決する（半透明を別のパスで重ねるときは、そのパスの終わりで）
                    resolve_target: (scene.color.is_some() && !second_pass).then_some(resolved),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: BACKGROUND[0],
                            g: BACKGROUND[1],
                            b: BACKGROUND[2],
                            a: 1.0,
                        }),
                        // 解決したあとの多サンプルの色は要らない
                        store: if scene.color.is_some() && !second_pass {
                            wgpu::StoreOp::Discard
                        } else {
                            wgpu::StoreOp::Store
                        },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &scene.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: if second_pass {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if display.shows_background() && self.env.baked.is_some() {
                pass.set_pipeline(&pipelines.background);
                pass.set_bind_group(0, bind, &[]);
                pass.draw(0..3, 0..1);
            }
            if let Some(mesh) = &self.mesh {
                pass.set_bind_group(0, bind, &[]);
                pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                // 1. 不透明・カットアウト（Unity の描く順: 不透明 → アルファテスト → 半透明）。輪郭線はその面の後
                for (start, end, pick, d) in draws.iter().filter(|(_, _, _, d)| !is_transparent(d))
                {
                    pass.set_bind_group(1, set_bind(*pick), &[]);
                    if d.lil {
                        pass.set_pipeline(lil(LilPipe {
                            hdr: hdr_path,
                            srgb: false,
                            cull: d.cull,
                            transparent: false,
                            outline: false,
                            spec: d.spec,
                            samples,
                        }));
                        pass.draw(*start..*end, 0..1);
                        if d.outline {
                            pass.set_pipeline(lil(LilPipe {
                                hdr: hdr_path,
                                srgb: false,
                                cull: 1,
                                transparent: false,
                                outline: true,
                                spec: d.spec,
                                samples,
                            }));
                            pass.draw(*start..*end, 0..1);
                        }
                    } else {
                        pass.set_pipeline(&pipelines.scene);
                        pass.draw(*start..*end, 0..1);
                    }
                }
                // 2. 半透明（HDR の描き先なら同じパスで。モデルの並びの順。面ごとの並べ替えはしない）
                if !second_pass {
                    for (start, end, pick, d) in
                        draws.iter().filter(|(_, _, _, d)| is_transparent(d))
                    {
                        pass.set_bind_group(1, set_bind(*pick), &[]);
                        pass.set_pipeline(lil(LilPipe {
                            hdr: hdr_path,
                            srgb: false,
                            cull: d.cull,
                            transparent: true,
                            outline: false,
                            spec: d.spec,
                            samples,
                        }));
                        pass.draw(*start..*end, 0..1);
                        if d.outline {
                            pass.set_pipeline(lil(LilPipe {
                                hdr: hdr_path,
                                srgb: false,
                                cull: 1,
                                transparent: true,
                                outline: true,
                                spec: d.spec,
                                samples,
                            }));
                            pass.draw(*start..*end, 0..1);
                        }
                    }
                }
                // 3. 選択範囲の重ね（今のセットの面だけ。面と同じ深さで読む。半透明を別のパスで重ねるときは、その半透明の下になる）
                let current: Vec<(u32, u32)> = draws
                    .iter()
                    .filter(|(_, _, pick, _)| *pick == Pick::Current)
                    .map(|(start, end, _, _)| (*start, *end))
                    .collect();
                self.overlay
                    .draw(&mut pass, bind, &mesh.buffer, &current, hdr_path, samples);
            }
        }
        if second_pass {
            if let Some(mesh) = &self.mesh {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("yolu-3d-transparent"),
                    color_attachments: &[Some(match scene.color.as_ref() {
                        // 多サンプル: 面を描いた多サンプルの色（sRGB の見え方）へ重ね、パスの終わりに 8 bit の描き先（sRGB の見え方）へ解決する
                        Some(msaa) => wgpu::RenderPassColorAttachment {
                            view: msaa.srgb_view.as_ref().expect("見え方を作れる機材だけ"),
                            depth_slice: None,
                            resolve_target: Some(
                                target.srgb_view.as_ref().expect("見え方を作れる機材だけ"),
                            ),
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Discard,
                            },
                        },
                        None => wgpu::RenderPassColorAttachment {
                            view: target.srgb_view.as_ref().expect("見え方を作れる機材だけ"),
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &scene.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_bind_group(0, bind, &[]);
                pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                for (start, end, pick, d) in draws.iter().filter(|(_, _, _, d)| is_transparent(d)) {
                    pass.set_bind_group(1, set_bind(*pick), &[]);
                    pass.set_pipeline(lil(LilPipe {
                        hdr: false,
                        srgb: true,
                        cull: d.cull,
                        transparent: true,
                        outline: false,
                        spec: d.spec,
                        samples,
                    }));
                    pass.draw(*start..*end, 0..1);
                    if d.outline {
                        pass.set_pipeline(lil(LilPipe {
                            hdr: false,
                            srgb: true,
                            cull: 1,
                            transparent: true,
                            outline: true,
                            spec: d.spec,
                            samples,
                        }));
                        pass.draw(*start..*end, 0..1);
                    }
                }
            }
        }
        if hdr_path {
            let bloom = display.uses_bloom();
            if bloom {
                self.encode_bloom(encoder, display);
            }
            let (curve, ev) = (display.tone_map, display.exposure);
            let curve = match curve {
                Curve::None => 0.0,
                Curve::Neutral => 1.0,
                Curve::Aces => 2.0,
            };
            // ブルームの絵の和は、散らしの重み（`BLOOM_SCATTER`）の等比数列の和 1 / (1 − 散らし) になっているので、掛けて 1 に戻す
            let bloom_gain = if bloom {
                display.post.bloom_strength * (1.0 - BLOOM_SCATTER)
            } else {
                0.0
            };
            let params: Vec<u8> = [curve, ev.exp2(), bloom_gain, 0.0]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect();
            self.rs.queue.write_buffer(&self.tone.params, 0, &params);
            let hdr = self.hdr_target.as_ref().expect("作った");
            let bloom_view = match (bloom, self.bloom_chain.as_ref()) {
                (true, Some(chain)) => &chain.views[0],
                _ => &self.dummy_bloom,
            };
            let tone_bind = self
                .rs
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("yolu-3d-tonemap"),
                    layout: &self.tone.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&hdr.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: self.tone.params.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(bloom_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                        },
                    ],
                });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("yolu-3d-tonemap"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.tone.pipeline);
            pass.set_bind_group(0, &tone_bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// ブルームの段を積む: HDR の絵（解決後）から明るい所を半分の大きさへ取り出し（最初の段）、1 段ごとに半分へ縮めて、
    /// 小さい段から順に 1 つ大きい段へ重みつきで足し戻す。結果は最初の段（`BloomChain::views[0]`）。
    fn encode_bloom(&self, encoder: &mut wgpu::CommandEncoder, display: &Display) {
        let (Some(pipes), Some(chain), Some(hdr)) = (
            self.bloom.as_ref(),
            self.bloom_chain.as_ref(),
            self.hdr_target.as_ref(),
        ) else {
            return;
        };
        let threshold = display.post.bloom_threshold;
        // ひざ: しきい値の下でなめらかに立ち上げる幅（しきい値の半分）
        let params: Vec<u8> = [
            threshold,
            threshold * 0.5 + 1e-4,
            BLOOM_SCATTER,
            display.exposure.exp2(),
        ]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
        self.rs.queue.write_buffer(&pipes.params, 0, &params);
        let bind = |source: &wgpu::TextureView| {
            self.rs
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("yolu-3d-bloom"),
                    layout: &pipes.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(source),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: pipes.params.as_entire_binding(),
                        },
                    ],
                })
        };
        let mut run = |label: &str,
                       target: &wgpu::TextureView,
                       load: wgpu::LoadOp<wgpu::Color>,
                       pipeline: &wgpu::RenderPipeline,
                       source: &wgpu::TextureView| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind(source), &[]);
            pass.draw(0..3, 0..1);
        };
        let clear = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
        run(
            "yolu-3d-bloom-prefilter",
            &chain.views[0],
            clear,
            &pipes.prefilter,
            &hdr.view,
        );
        for k in 1..chain.views.len() {
            run(
                "yolu-3d-bloom-down",
                &chain.views[k],
                clear,
                &pipes.down,
                &chain.views[k - 1],
            );
        }
        for k in (1..chain.views.len()).rev() {
            run(
                "yolu-3d-bloom-up",
                &chain.views[k - 1],
                wgpu::LoadOp::Load,
                &pipes.up,
                &chain.views[k],
            );
        }
    }
}

/// モデルの面が（見せる形に）あるマテリアル（昇順・重ならない）。
fn model_materials(model: &ViewModel) -> Vec<i32> {
    let mut v: Vec<i32> = model
        .meshes
        .iter()
        .flat_map(|m| m.submeshes.iter())
        .filter(|s| !s.indices.is_empty())
        .map(|s| s.material)
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// 世界 → 影のマップの (u, v, 深さ)（光のカメラ: 外接球に合わせた平行投影）。u・v は 0〜1、深さは 0〜1 で 0 が光の側、外接球の直径が 1。
/// 光の向きに沿う軸が深さで、残る 2 軸は真上・真下からのときだけ基準の上を替える（Unity 版の PreviewShadowMap と同じ）。
fn shadow_matrix(center: Vec3, radius: f32, to_light: Vec3) -> Mat4 {
    let forward = -to_light.try_normalize().unwrap_or(Vec3::Y);
    let up_ref = if forward.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let right = up_ref.cross(forward).normalize();
    let up = forward.cross(right);
    let d = radius * 2.0;
    Mat4::from_cols(
        Vec4::new(right.x / d, up.x / d, forward.x / d, 0.0),
        Vec4::new(right.y / d, up.y / d, forward.y / d, 0.0),
        Vec4::new(right.z / d, up.z / d, forward.z / d, 0.0),
        Vec4::new(
            0.5 - right.dot(center) / d,
            0.5 - up.dot(center) / d,
            0.5 - forward.dot(center) / d,
            1.0,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ウィンドウの面は、先に溜めるフレームを 1 枚に絞る設定（`LOW_LATENCY` の溜め）で作る。同期は、既定（設定「垂直同期」が切）は待たず、入のときだけ待つ。
    #[test]
    fn the_window_surface_is_configured_for_low_latency_and_waits_for_vsync_only_when_asked() {
        let latency = egui_wgpu::SurfaceConfig::LOW_LATENCY.desired_maximum_frame_latency;
        assert_eq!(latency, Some(1));
        for (vsync, mode) in [
            (false, wgpu::PresentMode::AutoNoVsync),
            (true, wgpu::PresentMode::AutoVsync),
        ] {
            let config = wgpu_configuration(vsync);
            assert_eq!(config.surface.present_mode, mode, "vsync={vsync}");
            assert_eq!(config.surface.desired_maximum_frame_latency, latency);
            assert_eq!(config.surface, surface_config(vsync));
            assert_ne!(config.surface, egui_wgpu::SurfaceConfig::HIGH_THROUGHPUT);
        }
        // 待たないほうが既定の形（設定の既定と同じ向き）
        let default = surface_config(crate::settings::Settings::default().vsync);
        assert_eq!(default.present_mode, wgpu::PresentMode::AutoNoVsync);
    }

    /// 塗った絵のサンプラー: 異方性は 16 まで、GPU が持たなければ 1。異方性を使うときは、wgpu が求める 3 つの補間がすべて Linear。
    #[test]
    fn the_paint_sampler_uses_anisotropy_only_when_the_adapter_has_it() {
        let with = wgpu::DownlevelFlags::ANISOTROPIC_FILTERING;
        let without = wgpu::DownlevelFlags::empty();
        assert_eq!(supported_anisotropy(with, PAINT_ANISOTROPY), 16);
        assert_eq!(supported_anisotropy(without, PAINT_ANISOTROPY), 1);
        // 既定: 実 GPU は 16、持たない機材と CPU で描くアダプターは 1
        assert_eq!(default_anisotropy(with, false), 16);
        assert_eq!(default_anisotropy(without, false), 1);
        assert_eq!(default_anisotropy(with, true), 1);
        assert_eq!(default_anisotropy(without, true), 1);
        // 範囲の外は 1〜16 に収める（0 は wgpu が断る値）
        assert_eq!(supported_anisotropy(with, 0), 1);
        assert_eq!(supported_anisotropy(with, 1), 1);
        assert_eq!(supported_anisotropy(with, 64), 16);
        for anisotropy in [1u16, 16] {
            let d = paint_sampler_descriptor(anisotropy);
            assert_eq!(d.anisotropy_clamp, anisotropy);
            assert_eq!(d.mag_filter, wgpu::FilterMode::Linear);
            assert_eq!(d.min_filter, wgpu::FilterMode::Linear);
            assert_eq!(d.mipmap_filter, wgpu::MipmapFilterMode::Linear);
            assert_eq!(d.address_mode_u, wgpu::AddressMode::Repeat);
            assert_eq!(d.address_mode_v, wgpu::AddressMode::Repeat);
        }
    }

    #[test]
    fn the_shadow_matrix_fits_the_bounding_sphere_into_the_unit_cube() {
        let (center, radius) = (Vec3::new(1.0, -2.0, 0.5), 3.0);
        for light in [
            Vec3::new(-0.3, 0.65, -0.7),
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::new(1.0, 0.0, 0.0),
        ] {
            let l = light.normalize();
            let m = shadow_matrix(center, radius, light);
            let at = |p: Vec3| m.transform_point3(p);
            let c = at(center);
            assert!((c - Vec3::splat(0.5)).length() < 1e-5, "{light:?}: {c:?}");
            // 光の側の端は深さ 0、反対の端は 1
            assert!(at(center + l * radius).z.abs() < 1e-5, "{light:?}");
            assert!((at(center - l * radius).z - 1.0).abs() < 1e-5, "{light:?}");
            // 球の上のどの点も 0〜1 の箱に入る
            for i in 0..64 {
                let a = i as f32 * 0.7;
                let p = center
                    + radius
                        * Vec3::new(a.sin() * (a * 1.9).cos(), (a * 1.3).sin(), a.cos())
                            .normalize();
                let q = at(p);
                assert!(
                    q.cmpge(Vec3::splat(-1e-5)).all() && q.cmple(Vec3::splat(1.0 + 1e-5)).all(),
                    "{light:?}: {p:?} → {q:?}"
                );
            }
            // 光に直交する 2 軸は直交で、深さと混ざらない（u・v は光の向きに依らない）
            let step = at(center + l * 0.5) - c;
            assert!(step.x.abs() < 1e-5 && step.y.abs() < 1e-5, "{step:?}");
        }
    }
}
