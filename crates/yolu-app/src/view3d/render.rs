//! 3D ビューの wgpu の描画（自前。埋め込みの Unity のプレイヤー（UaaL）は後で同じ枠に差し込む）。
//!
//! - 描き先は自前の色（Rgba8Unorm）と深度のテクスチャで、egui のネイティブのテクスチャとして画像で貼る（egui の描画のパスの MSAA・
//!   深度の設定に左右されない）。描くのはカメラ・大きさ・モデル・塗った絵のどれかが変わったときだけ。
//! - 塗った絵は文書の合成の写し（見せるだけ。正本は core の straight RGBA8）。初めと文書が変わったときだけ全部を作り、あとは core が
//!   「変わった」と言うタイルだけを合成して上げる。テクスチャは乗算済み（補間で透明の縁が黒くならない）で、行は core と同じ下から上
//!   （UV の v がそのまま文書の y）。GPU のテクスチャの上限を超える文書は 2 の累乗で縮めて持つ。ミップマップは変わった範囲だけ作り直す。
//! - 色はガンマの空間のまま計算する（Unity 版のプレビューのシェーダーと同じ式: UV 24 マスの市松の灰色の上に絵、
//!   0.35 + 0.65 × saturate(n·L)）。egui はネイティブのテクスチャの値をガンマの値として読む。

use eframe::egui_wgpu::{self, wgpu};
use wgpu::util::DeviceExt;
use yolu_core::geometry::OrbitCamera;
use yolu_core::{Channel, Document, Rect as DocRect, RowOrder, TileCoord};

use super::model::ViewModel;

const SCENE_SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
    light_dir: vec4<f32>,
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var paint_tex: texture_2d<f32>;
@group(0) @binding(2) var paint_sampler: sampler;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) paint: f32,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) paint: f32,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    o.clip = u.view_proj * vec4<f32>(v.position, 1.0);
    o.normal = v.normal;
    o.uv = v.uv;
    o.paint = v.paint;
    return o;
}

@fragment
fn fs_main(o: VsOut) -> @location(0) vec4<f32> {
    // 乗算済みの絵（v は文書の y と同じ向き）
    let texel = textureSample(paint_tex, paint_sampler, o.uv);
    let cell = floor(o.uv.x * 24.0) + floor(o.uv.y * 24.0);
    let checker = cell - 2.0 * floor(cell * 0.5);
    let background = mix(vec3<f32>(0.24, 0.24, 0.24), vec3<f32>(0.34, 0.34, 0.34), checker);
    var color = background;
    if (o.paint > 0.5) {
        color = background * (1.0 - texel.a) + texel.rgb;
    }
    let n = normalize(o.normal);
    let lit = clamp(dot(n, normalize(u.light_dir.xyz)), 0.0, 1.0);
    let light = vec3<f32>(0.35, 0.35, 0.35) + vec3<f32>(0.65, 0.65, 0.65) * lit;
    return vec4<f32>(color * mix(vec3<f32>(1.0, 1.0, 1.0), light, u.params.x), 1.0);
}
"#;

const MIP_SHADER: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    // 1 つ上の段の 2 × 2 の真ん中を双線形で読む（乗算済みの箱のフィルター）
    let size = vec2<f32>(max(textureDimensions(src) / 2u, vec2<u32>(1u, 1u)));
    return textureSampleLevel(src, samp, p.xy / size, 0.0);
}
"#;

/// 背景（Unity 版の 3D ビューのカメラの背景 (0.12, 0.13, 0.15)）。
pub const BACKGROUND: [f64; 3] = [0.12, 0.13, 0.15];
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// 頂点 1 つ: 位置 3・法線 3・UV 2・絵を貼るか 1（f32）。
const VERTEX_FLOATS: usize = 9;
/// 塗った絵のテクスチャの一辺の上限（GPU の上限がもっと小さければそれ）。
const MAX_PAINT_SIZE: u32 = 8192;

/// 上げた量・描いた回数（試験と状態の表示用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View3dStats {
    /// 最後の同期で上げたタイルの数。
    pub last_tiles: usize,
    /// 最後の同期で全部を作り直したか。
    pub last_rebuilt: bool,
    /// これまでに上げたタイルの数。
    pub total_tiles: usize,
    /// これまでに 3D を描いた回数（変わらなければ描かない）。
    pub renders: usize,
    /// 文書から塗った絵へ縮めた段（0 なら同じ大きさ）。
    pub paint_level: u32,
}

struct Target {
    view: wgpu::TextureView,
    depth: wgpu::TextureView,
    size: [u32; 2],
    id: egui::TextureId,
}

struct GpuMesh {
    buffer: wgpu::Buffer,
    vertices: u32,
    model: usize,
    material: i32,
}

struct PaintTexture {
    texture: wgpu::Texture,
    /// 段ごとの 1 段だけの見え方（ミップを作るとき、上の段を読んで下の段へ描く）。
    levels: Vec<wgpu::TextureView>,
    /// 段 i を作るときに読む、段 i − 1 の束ね。
    mip_binds: Vec<wgpu::BindGroup>,
    scene_bind: wgpu::BindGroup,
    size: [u32; 2],
    /// 文書 → 絵の縮め（2 の shift 乗）。
    shift: u32,
    doc_id: u128,
    doc_size: (u32, u32),
    serial: u64,
    /// 中身が変わるたびに増える（描き直しの鍵）。
    version: u64,
}

#[derive(Clone, Copy, PartialEq)]
struct SceneKey {
    camera: [u32; 6],
    size: [u32; 2],
    model: usize,
    material: i32,
    paint: u64,
}

/// 3D ビューの描画（wgpu の装置は eframe と同じもの）。
pub struct View3dRenderer {
    rs: egui_wgpu::RenderState,
    scene: wgpu::RenderPipeline,
    scene_layout: wgpu::BindGroupLayout,
    mip: wgpu::RenderPipeline,
    mip_layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    paint_sampler: wgpu::Sampler,
    mip_sampler: wgpu::Sampler,
    target: Option<Target>,
    mesh: Option<GpuMesh>,
    paint: Option<PaintTexture>,
    last_key: Option<SceneKey>,
    buffer: Vec<u8>,
    pub stats: View3dStats,
}

impl View3dRenderer {
    pub fn new(rs: &egui_wgpu::RenderState) -> View3dRenderer {
        let device = &rs.device;
        let scene_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yolu-3d-scene"),
            source: wgpu::ShaderSource::Wgsl(SCENE_SHADER.into()),
        });
        let mip_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yolu-3d-mip"),
            source: wgpu::ShaderSource::Wgsl(MIP_SHADER.into()),
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yolu-3d-scene"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(1),
                sampler_entry(2),
            ],
        });
        let mip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yolu-3d-mip"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        let scene_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("yolu-3d-scene"),
                bind_group_layouts: &[Some(&scene_layout)],
                immediate_size: 0,
            });
        let mip_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yolu-3d-mip"),
            bind_group_layouts: &[Some(&mip_layout)],
            immediate_size: 0,
        });
        let attributes =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32];
        let scene = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                    format: COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let mip = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("yolu-3d-mip"),
            layout: Some(&mip_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &mip_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &mip_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yolu-3d-uniforms"),
            size: 96,
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
        let mip_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("yolu-3d-mip"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        View3dRenderer {
            rs: rs.clone(),
            scene,
            scene_layout,
            mip,
            mip_layout,
            uniforms,
            paint_sampler,
            mip_sampler,
            target: None,
            mesh: None,
            paint: None,
            last_key: None,
            buffer: Vec::new(),
            stats: View3dStats::default(),
        }
    }

    /// 文書の変わった所を上げ、要れば描き直して、egui に貼るテクスチャを返す（size は物理の画素）。
    pub fn prepare(
        &mut self,
        doc: &Document,
        model: &std::sync::Arc<ViewModel>,
        material: i32,
        camera: &OrbitCamera,
        size: [u32; 2],
    ) -> egui::TextureId {
        let size = [size[0].max(1), size[1].max(1)];
        let mut encoder = self
            .rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("yolu-3d"),
            });
        self.sync_paint(doc, &mut encoder);
        self.ensure_mesh(model, material);
        let resized = self.ensure_target(size);
        let paint = self.paint.as_ref().expect("同期した");
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
            model: std::sync::Arc::as_ptr(model) as usize,
            material,
            paint: paint.version,
        };
        // 絵が変わればミップも鍵の版も変わるので、描かないフレームは何も積んでいない（出さずに捨てる）
        if resized || self.last_key != Some(key) {
            self.draw_scene(&mut encoder, camera, size);
            self.last_key = Some(key);
            self.stats.renders += 1;
            self.rs.queue.submit(Some(encoder.finish()));
        }
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
            format: COLOR_FORMAT,
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

    /// モデルとテクスチャセットの頂点を上げる（変わったときだけ）。
    fn ensure_mesh(&mut self, model: &std::sync::Arc<ViewModel>, material: i32) {
        let key = std::sync::Arc::as_ptr(model) as usize;
        if self
            .mesh
            .as_ref()
            .is_some_and(|m| m.model == key && m.material == material)
        {
            return;
        }
        let triangles: usize = model.meshes.iter().map(|m| m.triangle_count()).sum();
        let mut data: Vec<u8> = Vec::with_capacity(triangles * 3 * VERTEX_FLOATS * 4);
        for mesh in &model.meshes {
            let normals = mesh.vertex_normals();
            for s in &mesh.submeshes {
                let paint = if s.material == material { 1.0f32 } else { 0.0 };
                for &i in &s.indices {
                    let i = i as usize;
                    let p = mesh.positions[i];
                    let n = normals[i];
                    let uv = mesh.uvs.get(i).copied().unwrap_or_default();
                    for v in [p.x, p.y, p.z, n.x, n.y, n.z, uv.x, uv.y, paint] {
                        data.extend_from_slice(&v.to_le_bytes());
                    }
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
        });
    }

    /// 塗った絵を文書に合わせる: 初めと文書が変わったときは全部、ほかは変わったタイルだけを合成して上げる。
    fn sync_paint(&mut self, doc: &Document, encoder: &mut wgpu::CommandEncoder) {
        let (w, h) = (doc.width(), doc.height());
        let since = self.paint.as_ref().map_or(0, |p| p.serial);
        let changed = doc.changed_tiles(Channel::Color, since);
        let serial = doc.change_serial();
        let rebuild = match &self.paint {
            None => true,
            Some(p) => p.doc_id != doc.id() || p.doc_size != (w, h) || changed.is_none(),
        };
        let coords: Vec<TileCoord> = if rebuild {
            self.create_paint(doc);
            // 中身のあるタイル（どれかの層が持つタイル）だけを合成する。ほかは透明（作ったテクスチャは 0）
            let mut all: Vec<TileCoord> = doc
                .layers()
                .iter()
                .filter_map(|l| l.surface(Channel::Color))
                .flat_map(|s| s.tile_coords())
                .collect();
            all.sort();
            all.dedup();
            all
        } else {
            changed.unwrap_or_default()
        };
        let paint = self.paint.as_mut().expect("作った");
        paint.serial = serial;
        let shift = paint.shift;
        let mut dirty: Option<[u32; 4]> = None; // 段 0 の画素の x0 y0 x1 y1
        let mut uploaded = 0;
        for coord in &coords {
            let Some(region) = doc.tile_rect(*coord) else {
                continue;
            };
            self.buffer
                .resize((region.width * region.height * 4) as usize, 0);
            if doc
                .composite_into(Channel::Color, region, &mut self.buffer, RowOrder::BottomUp)
                .is_err()
            {
                continue;
            }
            let (data, dw, dh) = reduce_premultiplied(&self.buffer, region, shift);
            let (dx, dy) = (region.x >> shift, region.y >> shift);
            self.rs.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &paint.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: dx, y: dy, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(dw * 4),
                    rows_per_image: Some(dh),
                },
                wgpu::Extent3d {
                    width: dw,
                    height: dh,
                    depth_or_array_layers: 1,
                },
            );
            let r = [dx, dy, dx + dw, dy + dh];
            dirty = Some(match dirty {
                None => r,
                Some(d) => [
                    d[0].min(r[0]),
                    d[1].min(r[1]),
                    d[2].max(r[2]),
                    d[3].max(r[3]),
                ],
            });
            uploaded += 1;
        }
        if uploaded > 0 || rebuild {
            paint.version += 1;
        }
        if let Some(d) = dirty {
            Self::rebuild_mips(&self.mip, paint, encoder, d);
        }
        self.stats.last_tiles = uploaded;
        self.stats.last_rebuilt = rebuild;
        self.stats.total_tiles += uploaded;
        self.stats.paint_level = shift;
    }

    fn create_paint(&mut self, doc: &Document) {
        let device = &self.rs.device;
        let limit = device.limits().max_texture_dimension_2d.min(MAX_PAINT_SIZE);
        let (w, h) = (doc.width(), doc.height());
        let mut shift = 0;
        while w.div_ceil(1 << shift) > limit || h.div_ceil(1 << shift) > limit {
            shift += 1;
        }
        let size = [w.div_ceil(1 << shift), h.div_ceil(1 << shift)];
        let levels = 32 - size[0].max(size[1]).leading_zeros();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-paint"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let level_views: Vec<wgpu::TextureView> = (0..levels)
            .map(|i| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: i,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let mip_binds = (1..levels as usize)
            .map(|i| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("yolu-3d-mip"),
                    layout: &self.mip_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&level_views[i - 1]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.mip_sampler),
                        },
                    ],
                })
            })
            .collect();
        let full = texture.create_view(&Default::default());
        let scene_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yolu-3d-scene"),
            layout: &self.scene_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&full),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.paint_sampler),
                },
            ],
        });
        let version = self.paint.as_ref().map_or(0, |p| p.version + 1);
        self.paint = Some(PaintTexture {
            texture,
            levels: level_views,
            mip_binds,
            scene_bind,
            size,
            shift,
            doc_id: doc.id(),
            doc_size: (w, h),
            serial: 0,
            version,
        });
    }

    /// 段 0 の dirty（x0 y0 x1 y1）の下の段を作り直す（その範囲だけを鋏で切って描く）。
    fn rebuild_mips(
        pipeline: &wgpu::RenderPipeline,
        paint: &PaintTexture,
        encoder: &mut wgpu::CommandEncoder,
        dirty: [u32; 4],
    ) {
        for level in 1..paint.levels.len() {
            let w = (paint.size[0] >> level).max(1);
            let h = (paint.size[1] >> level).max(1);
            // 奇数の大きさの補間がはみ出す分、1 画素ずつ広げる
            let x0 = (dirty[0] >> level).saturating_sub(1);
            let y0 = (dirty[1] >> level).saturating_sub(1);
            let x1 = (dirty[2].div_ceil(1 << level) + 1).min(w);
            let y1 = (dirty[3].div_ceil(1 << level) + 1).min(h);
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("yolu-3d-mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &paint.levels[level],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &paint.mip_binds[level - 1], &[]);
            pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
            pass.draw(0..3, 0..1);
        }
    }

    fn draw_scene(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        camera: &OrbitCamera,
        size: [u32; 2],
    ) {
        let view = camera.view(size[0] as f32, size[1] as f32);
        let mut u = Vec::with_capacity(96);
        for v in view.view_projection().to_cols_array() {
            u.extend_from_slice(&v.to_le_bytes());
        }
        // 光へ向かう向き（Unity 版のプレビューの既定 (−0.3, 0.65, −0.7)）と、照明あり
        for v in [-0.3f32, 0.65, -0.7, 0.0, 1.0, 0.0, 0.0, 0.0] {
            u.extend_from_slice(&v.to_le_bytes());
        }
        self.rs.queue.write_buffer(&self.uniforms, 0, &u);
        let target = self.target.as_ref().expect("作った");
        let paint = self.paint.as_ref().expect("作った");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("yolu-3d-scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
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
        if let Some(mesh) = &self.mesh {
            pass.set_pipeline(&self.scene);
            pass.set_bind_group(0, &paint.scene_bind, &[]);
            pass.set_vertex_buffer(0, mesh.buffer.slice(..));
            pass.draw(0..mesh.vertices, 0..1);
        }
    }
}

/// straight の RGBA8（行は下から）を乗算済みにし、2^shift の箱で縮める（文書の端の欠けた箱は中の画素だけで平均する）。
fn reduce_premultiplied(straight: &[u8], region: DocRect, shift: u32) -> (Vec<u8>, u32, u32) {
    let premul = |p: &[u8]| -> [u32; 4] {
        let a = p[3] as u32;
        [
            (p[0] as u32 * a + 127) / 255,
            (p[1] as u32 * a + 127) / 255,
            (p[2] as u32 * a + 127) / 255,
            a,
        ]
    };
    let (w, h) = (region.width, region.height);
    if shift == 0 {
        let mut out = Vec::with_capacity(straight.len());
        for p in straight.as_chunks::<4>().0 {
            let q = premul(p);
            out.extend_from_slice(&[q[0] as u8, q[1] as u8, q[2] as u8, q[3] as u8]);
        }
        return (out, w, h);
    }
    let block = 1u32 << shift;
    let (dw, dh) = (w.div_ceil(block), h.div_ceil(block));
    let mut out = Vec::with_capacity((dw * dh * 4) as usize);
    for by in 0..dh {
        for bx in 0..dw {
            let mut sum = [0u32; 4];
            let mut n = 0u32;
            for y in by * block..((by + 1) * block).min(h) {
                for x in bx * block..((bx + 1) * block).min(w) {
                    let i = ((y * w + x) * 4) as usize;
                    let q = premul(&straight[i..i + 4]);
                    for k in 0..4 {
                        sum[k] += q[k];
                    }
                    n += 1;
                }
            }
            for s in sum {
                out.push(((s + n / 2) / n) as u8);
            }
        }
    }
    (out, dw, dh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduce_averages_premultiplied_blocks_and_partial_edges() {
        // 3 × 2 の赤（不透明・半透明・透明）を 2 で縮めると 2 × 1
        let px = |r: u8, a: u8| [r, 0, 0, a];
        let mut s = Vec::new();
        for p in [
            px(255, 255),
            px(255, 128),
            px(9, 0),
            px(255, 255),
            px(255, 128),
            px(9, 0),
        ] {
            s.extend_from_slice(&p);
        }
        let (out, w, h) = reduce_premultiplied(&s, DocRect::new(0, 0, 3, 2), 1);
        assert_eq!((w, h), (2, 1));
        // 左の箱: (255 + 128 + 255 + 128) / 4、右の箱は透明だけ（RGB も 0。乗算済みなので色は消える）
        assert_eq!(&out[0..4], &[192, 0, 0, 192]);
        assert_eq!(&out[4..8], &[0, 0, 0, 0]);
        let (same, w, h) = reduce_premultiplied(&s, DocRect::new(0, 0, 3, 2), 0);
        assert_eq!((w, h), (3, 2));
        assert_eq!(&same[4..8], &[128, 0, 0, 128]);
    }
}
