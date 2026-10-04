//! 3D ビューの wgpu の描画（自前。埋め込みの Unity のプレイヤー（UaaL）は後で同じ枠に差し込む）。
//!
//! - 描き先は自前の色（Rgba8Unorm）と深度のテクスチャで、egui のネイティブのテクスチャとして画像で貼る（egui の描画のパスの MSAA・
//!   深度の設定に左右されない）。描くのはカメラ・大きさ・モデル・塗った絵・表示の設定のどれかが変わったときだけ。
//! - 塗った絵は標準の 6 チャンネルのテクスチャ（`paint`。変わったタイルだけを上げる）、面の見え方は `shaders/scene.wgsl`: マテリアル（Unity の
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
use yolu_core::geometry::OrbitCamera;
use yolu_core::glam::{Mat4, Vec3, Vec4};
use yolu_core::mesh_maps::BakedMeshMap;
use yolu_core::Document;

use super::brdf::{self, Curve};
use super::display::{Display, EnvKind, Shading};
use super::environment::{self, Baked, Source, FACE_SIZE, MIP_COUNT};
use super::model::ViewModel;
use super::paint::{ImageTexture, Paint, PaintStats, Slot};
use super::tangents::Tangent;

/// 背景（Unity 版の 3D ビューのカメラの背景 (0.12, 0.13, 0.15)）。
pub const BACKGROUND: [f64; 3] = [0.12, 0.13, 0.15];
const LDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// 頂点 1 つ: 位置 3・法線 3・UV 2・接線 4・絵を貼るか 1（f32）。
const VERTEX_FLOATS: usize = 13;
/// 一様バッファの大きさ（`shaders/scene.wgsl` の `Uniforms`）。
const UNIFORM_BYTES: u64 = 128 + 9 * 16 + 9 * 16 + 64 + 16;
/// 影のマップの 1 辺（Depth32Float で 16 MiB。Unity 版と同じ 2048）。
pub const SHADOW_SIZE: u32 = 2048;

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
            ..View3dStats::default()
        }
    }
}

struct Target {
    view: wgpu::TextureView,
    depth: wgpu::TextureView,
    size: [u32; 2],
    id: egui::TextureId,
}

/// トーンマッピングのときの HDR の描き先。
struct HdrTarget {
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
    size: [u32; 2],
}

struct GpuMesh {
    buffer: wgpu::Buffer,
    vertices: u32,
    /// 上げたモデルの世代（`ViewModel::revision`）。モデルの見分けはアドレスでなく世代で行う: ポーズの変更はモデルを何度も入れ替え、
    /// 落とした古いモデルのアドレスを次の新しいモデルが使うことがあり、そうなると上げ直しも描き直しも起きない。世界は
    /// `View3dState::next_revision` で作るので、モデルごとに違う。
    model: u32,
    material: i32,
    /// 上げた接線の見分け（0 なら接線なし）。
    tangent_stamp: usize,
}

#[derive(Clone, Copy, PartialEq)]
struct SceneKey {
    camera: [u32; 6],
    size: [u32; 2],
    /// モデルの世代（`GpuMesh::model` と同じ）。
    model: u32,
    material: i32,
    paint: u64,
    tangents: usize,
    env: u64,
    map: u64,
    display: [u32; 20],
}

struct Pipelines {
    scene: wgpu::RenderPipeline,
    background: wgpu::RenderPipeline,
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
    paint: Paint,
    scene_layout: wgpu::BindGroupLayout,
    ldr: Pipelines,
    hdr: Pipelines,
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
    mesh: Option<GpuMesh>,
    bind: Option<(wgpu::BindGroup, (u64, u64, u64, u64))>,
    last_key: Option<SceneKey>,
    /// 接線を作っているモデルのスレッドと、最後に出来た接線（ポーズで同じ形のモデルが続くとき、出来るまでこれを使う）。
    inflight: Option<Inflight>,
    last_tangents: Option<Arc<Vec<Tangent>>>,
    /// 今の表示が接線を読むか（読まないあいだは、作っていても窓の描き直しを求めない）。
    tangents_wanted: bool,
    /// 接線を作るスレッドが接線を残さずに終わったモデルの世代（同じモデルで作り直さない。法線マップは読まない）。
    tangents_failed: Option<u32>,
    tangent_hook: Option<TangentHook>,
    pub stats: View3dStats,
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

impl View3dRenderer {
    pub fn new(rs: &egui_wgpu::RenderState) -> View3dRenderer {
        let device = &rs.device;
        let scene_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yolu-3d-scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/scene.wgsl").into()),
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
        entries.extend((1..=6).map(|b| texture_entry(b, d2)));
        entries.push(sampler_entry(7));
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
        let scene_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("yolu-3d-scene"),
                bind_group_layouts: &[Some(&scene_layout)],
                immediate_size: 0,
            });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4, 4 => Float32
        ];
        let make = |format: wgpu::TextureFormat| Pipelines {
            scene: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("yolu-3d-scene"),
                layout: Some(&scene_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &scene_module,
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
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &scene_module,
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
                layout: Some(&scene_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &scene_module,
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
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &scene_module,
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
        };
        let ldr = make(LDR);
        let hdr = make(HDR);
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
        let paint_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("yolu-3d-paint"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
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
        View3dRenderer {
            rs: rs.clone(),
            paint: Paint::new(rs),
            scene_layout,
            ldr,
            hdr,
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
            mesh: None,
            bind: None,
            last_key: None,
            inflight: None,
            last_tangents: None,
            tangents_wanted: false,
            tangents_failed: None,
            tangent_hook: None,
            stats: View3dStats::default(),
        }
    }

    /// 塗った絵のバイトの予算を決める（試験が小さくして、縮めの道を通す）。
    pub fn set_paint_budget(&mut self, bytes: u64) {
        self.paint.set_budget(bytes);
    }

    /// 試験用: 塗った絵を捨てる（次の描きが文書から全部を作り直す）。
    pub fn invalidate_paint(&mut self) {
        self.paint.invalidate();
    }

    /// 試験用: 塗った絵のチャンネルの 1 段の中身（`Paint::read_level`）。
    pub fn read_paint_level(&self, slot: Slot, level: u32) -> Option<(Vec<u8>, [u32; 2])> {
        self.paint.read_level(slot, level)
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

    /// 接線を作っている最中で、今の表示がそれを読むか（出来たら描き直したいので、窓は次のフレームも要る）。光なしの表示へ替えた・法線マップを
    /// 使う層が無くなったときは、作っていても求めない（絵は変わらないのに窓を回し続けない）。
    pub fn wants_repaint(&self) -> bool {
        self.tangents_wanted && self.inflight.is_some()
    }

    /// 文書の変わった所を上げ、要れば描き直して、egui に貼るテクスチャを返す（size は物理の画素）。
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
        let started = std::time::Instant::now();
        let size = [size[0].max(1), size[1].max(1)];
        let mut encoder = self
            .rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("yolu-3d"),
            });
        self.paint.sync(doc, &mut encoder);
        let sync_us = started.elapsed().as_micros() as u64;
        self.sync_environment(display);
        self.sync_map(display, map, &mut encoder);
        let use_normal_map = self.paint.has(Slot::Normal) && !display.is_unlit();
        self.ensure_mesh(model, material, use_normal_map);
        let resized = self.ensure_target(size);
        let tone = display.uses_tone_map();
        if tone {
            self.ensure_hdr_target(size);
        }
        self.ensure_shadow(display);
        self.ensure_bind();
        let key = SceneKey {
            camera: [
                camera.target.x.to_bits(),
                camera.target.y.to_bits(),
                camera.target.z.to_bits(),
                camera.yaw.to_bits(),
                camera.pitch.to_bits(),
                camera.distance.to_bits(),
            ],
            size,
            model: model.revision(),
            material,
            paint: self.paint.version(),
            tangents: self.mesh.as_ref().map_or(0, |m| m.tangent_stamp),
            env: self.env.version,
            map: self.map_version,
            display: display.key_bits(),
        };
        // 絵が変わればミップも鍵の版も変わるので、描かないフレームは何も積んでいない（出さずに捨てる）
        if resized || self.last_key != Some(key) {
            if display.uses_shadows() {
                self.update_shadow(model, display.light_direction(), &mut encoder);
            }
            self.draw_scene(&mut encoder, camera, size, display, use_normal_map);
            self.last_key = Some(key);
            self.stats.renders += 1;
            if tone {
                self.stats.tone_mapped_renders += 1;
            }
            self.rs.queue.submit(Some(encoder.finish()));
        }
        let paint: View3dStats = self.paint.stats.into();
        self.stats = View3dStats {
            renders: self.stats.renders,
            tone_mapped_renders: self.stats.tone_mapped_renders,
            mesh_revision: self.stats.mesh_revision,
            mesh_uploads: self.stats.mesh_uploads,
            env_bakes: self.stats.env_bakes,
            shadow_renders: self.stats.shadow_renders,
            shadow_active: self.shadow_in_use(display),
            map_level: self.map.as_ref().map_or(0, |(_, t)| t.level()),
            tangents_exact: self.stats.tangents_exact,
            last_prepare_us: started.elapsed().as_micros() as u64,
            last_sync_us: sync_us,
            ..paint
        };
        self.target.as_ref().expect("作った").id
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
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = color.create_view(&Default::default());
        let depth = depth.create_view(&Default::default());
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
            depth,
            size,
            id,
        });
        true
    }

    /// トーンマッピングのための HDR の描き先（大きさに合わせる）。
    fn ensure_hdr_target(&mut self, size: [u32; 2]) {
        if self.hdr_target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
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
        let bind = self
            .rs
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("yolu-3d-tonemap"),
                layout: &self.tone.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.tone.params.as_entire_binding(),
                    },
                ],
            });
        self.hdr_target = Some(HdrTarget { view, bind, size });
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
        match self.paint.create_image(&rgba, size, encoder) {
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

    /// 束ね（チャンネルのテクスチャ・環境のキューブ・メッシュマップ）を、変わったときだけ作り直す。
    fn ensure_bind(&mut self) {
        let key = (
            self.paint.layout_version(),
            self.env.version,
            self.map_version,
            self.shadow_version,
        );
        if self.bind.as_ref().is_some_and(|(_, k)| *k == key) {
            return;
        }
        let cube = self.env.cube.as_ref().map_or(&self.dummy_cube, |c| &c.view);
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: self.uniforms.as_entire_binding(),
        }];
        for slot in Slot::ALL {
            entries.push(wgpu::BindGroupEntry {
                binding: 1 + slot.index() as u32,
                resource: wgpu::BindingResource::TextureView(self.paint.view(slot)),
            });
        }
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

    /// モデルとテクスチャセットの頂点を上げる（変わったときだけ）。法線マップを使うときは接線も。
    fn ensure_mesh(&mut self, model: &Arc<ViewModel>, material: i32, use_normal_map: bool) {
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
            .is_some_and(|m| m.model == key && m.material == material && m.tangent_stamp == stamp)
        {
            self.stats.tangents_exact = exact;
            return;
        }
        let mut data: Vec<u8> = Vec::with_capacity(corners * VERTEX_FLOATS * 4);
        let mut corner = 0usize;
        for mesh in &model.meshes {
            let normals = mesh.vertex_normals();
            for s in &mesh.submeshes {
                let paint = if s.material == material { 1.0f32 } else { 0.0 };
                for &i in &s.indices {
                    let i = i as usize;
                    let p = mesh.positions[i];
                    let n = normals[i];
                    let uv = mesh.uvs.get(i).copied().unwrap_or_default();
                    // 接線が無いとき（法線マップを使わない・まだ作っている）は 0。シェーダーは `mode.w` で読まない
                    let t = tangents
                        .as_ref()
                        .and_then(|t| t.get(corner))
                        .copied()
                        .unwrap_or([0.0; 4]);
                    for v in [
                        p.x, p.y, p.z, n.x, n.y, n.z, uv.x, uv.y, t[0], t[1], t[2], t[3], paint,
                    ] {
                        data.extend_from_slice(&v.to_le_bytes());
                    }
                    corner += 1;
                }
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
            material,
            tangent_stamp: stamp,
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
        let p = view.position;
        f.extend_from_slice(&[p.x, p.y, p.z, 0.0]);
        let l = display.light_direction();
        f.extend_from_slice(&[l.x, l.y, l.z, 0.0]);
        // マテリアル表示の光（Unity 版: 色 × 0.769 × 強さ をリニアへ）と、環境が無いときの一様な環境光（色 × 0.4 をリニアへ）
        let direct = display
            .light_color
            .map(|c| brdf::srgb_to_linear(c * 0.769 * display.light_intensity));
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
        debug_assert_eq!(f.len() * 4, UNIFORM_BYTES as usize);
        f.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn draw_scene(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        camera: &OrbitCamera,
        size: [u32; 2],
        display: &Display,
        use_normal_map: bool,
    ) {
        let bytes = self.uniform_bytes(camera, size, display, use_normal_map);
        self.rs.queue.write_buffer(&self.uniforms, 0, &bytes);
        let tone = display.uses_tone_map();
        let target = self.target.as_ref().expect("作った");
        let (color_view, pipelines) = if tone {
            let hdr = self.hdr_target.as_ref().expect("作った");
            (&hdr.view, &self.hdr)
        } else {
            (&target.view, &self.ldr)
        };
        let bind = &self.bind.as_ref().expect("作った").0;
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("yolu-3d-scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: BACKGROUND[0],
                            g: BACKGROUND[1],
                            b: BACKGROUND[2],
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
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
                pass.set_pipeline(&pipelines.scene);
                pass.set_bind_group(0, bind, &[]);
                pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                pass.draw(0..mesh.vertices, 0..1);
            }
        }
        if tone {
            let (curve, ev) = (display.tone_map, display.exposure);
            let curve = match curve {
                Curve::None => 0.0,
                Curve::Neutral => 1.0,
                Curve::Aces => 2.0,
            };
            let params: Vec<u8> = [curve, ev.exp2(), 0.0, 0.0]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect();
            self.rs.queue.write_buffer(&self.tone.params, 0, &params);
            let hdr = self.hdr_target.as_ref().expect("作った");
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
            pass.set_bind_group(0, &hdr.bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }
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
