//! lilToon の見た目が読むユーザーチャンネルの絵（GPU の 2D テクスチャの配列。レイヤー 1 つがチャンネル 1 つ）。
//!
//! 標準の 6 チャンネルは `paint::Paint` が持つ。ユーザーチャンネルは、見た目の設定（`MaterialLook`）がスロットに割り当てたものだけを、
//! ここで配列に持つ（最大 [`MAX_LAYERS`]。GL のために 1 つでも 2 レイヤーで作る）。値は合成の straight RGBA を乗算済みにしたもの（シェーダーが、何も描いていない所の値
//! （チャンネルの既定）に重ねて読む）。行は文書と同じ下から上。
//!
//! - 初めと、チャンネルの並び・文書・縮めが替わったときは全部を作り直し、ほかは core が「変わった」と言うタイルだけを合成して上げる
//!   （`Paint` と同じ道。ミップは変わった範囲だけ作り直す）。
//! - 大きさは標準のチャンネルと同じ縮め（`paint_shift`）から始め、ユーザーチャンネルの全部（ミップ込み）が渡された予算に収まるまで、
//!   さらに 2 の累乗で縮める（マスクは標準のチャンネルより粗くなることがある）。予算は 3D ビューの全体の予算の計画（`render`）が決める:
//!   今のセットは標準のチャンネルの残りから、ほかのセットは持つかどうかの計画に入れて。どちらもセットごとに [`USER_BUDGET_BYTES`] まで。

use std::sync::atomic::{AtomicU64, Ordering};

use eframe::egui_wgpu::wgpu;
use yolu_core::{Channel, Document, LayerKind, Rect as DocRect, RowOrder, TileCoord};

use super::paint::{align, choose_shift, mip_bytes, plan_regions, reduce_premultiplied};

/// 1 つのセットが持てるユーザーチャンネルのレイヤーの数（シェーダーの `NU` と同じ）。
pub const MAX_LAYERS: usize = 16;
/// 1 つのセットのユーザーチャンネルのレイヤーの全部（ミップ込み）の GPU のバイト数の上限（全体の予算の残りがもっと少なければそれ）。
pub const USER_BUDGET_BYTES: u64 = 256 << 20;
/// 配列のレイヤーの数の下限。GL はレイヤーが 1 つのテクスチャを 2D として作り、配列として読めないので、1 つでも 2 レイヤーで作る。
const MIN_LAYERS: usize = 2;
/// `layer_index` の 1 つ分のずらし（一様バッファの動的なずらしの揃え）。
const LAYER_STRIDE: u64 = 256;

/// 持つレイヤーの数（チャンネルの数、ただし [`MIN_LAYERS`] 以上）。
fn layer_count(channels: usize) -> usize {
    channels.max(MIN_LAYERS)
}

/// ユーザーチャンネル `count` 個（[`MAX_LAYERS`] まで）を持つときの縮めの段とバイト数（ミップ込み）。標準のチャンネルの縮め
/// `paint_shift` から始め、辺が `limit` に、バイト数が `budget`（[`USER_BUDGET_BYTES`] まで）に収まるまで縮める。0 個なら持たない。
pub fn plan(size: [u32; 2], count: usize, paint_shift: u32, limit: u32, budget: u64) -> (u32, u64) {
    if count == 0 {
        return (0, 0);
    }
    let per_texel = 4 * layer_count(count.min(MAX_LAYERS)) as u64;
    let own = choose_shift(size, per_texel, limit, budget.min(USER_BUDGET_BYTES));
    let shift = own.max(paint_shift);
    let reduced = [
        size[0].div_ceil(1 << shift).max(1),
        size[1].div_ceil(1 << shift).max(1),
    ];
    (shift, mip_bytes(reduced, per_texel))
}
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct State {
    doc_id: u128,
    doc_size: (u32, u32),
    channels: Vec<Channel>,
    shift: u32,
    size: [u32; 2],
    levels: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// [レイヤー][段] の 1 段・1 レイヤーだけの見え方（ミップを作るときの描き先）。
    level_views: Vec<Vec<wgpu::TextureView>>,
    /// [段] の、その段だけ・全部のレイヤーの配列の見え方（ミップを作るときの読む元）。
    level_arrays: Vec<wgpu::TextureView>,
    serial: u64,
}

/// ユーザーチャンネルの配列（セットごと）。
pub struct UserLayers {
    device: wgpu::Device,
    queue: wgpu::Queue,
    mip_pipeline: wgpu::RenderPipeline,
    mip_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// レイヤーの番号（`LAYER_STRIDE` ごとに 1 つ。ミップを作るシェーダーが動的なずらしで読む）。
    layer_index: wgpu::Buffer,
    /// 何も持たないときに束ねる 1 × 1 × 2（透明）。
    blank: wgpu::TextureView,
    state: Option<State>,
    /// 中身が変わるたびに増える（描き直しの鍵）。
    version: u64,
    /// テクスチャを作り直した世代（束ねの鍵）。
    layout_version: u64,
    uid: u64,
    scratch: Vec<u8>,
}

fn next_uid() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl UserLayers {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> UserLayers {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yolu-3d-user-mip"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/user_mip.wgsl").into()),
        });
        let mip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yolu-3d-user-mip"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yolu-3d-user-mip"),
            bind_group_layouts: &[Some(&mip_layout)],
            immediate_size: 0,
        });
        let mip_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("yolu-3d-user-mip"),
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
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("yolu-3d-user-mip"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let layer_index = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yolu-3d-user-mip-layer"),
            size: LAYER_STRIDE * MAX_LAYERS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut indices = vec![0u8; (LAYER_STRIDE * MAX_LAYERS as u64) as usize];
        for layer in 0..MAX_LAYERS {
            let at = layer * LAYER_STRIDE as usize;
            indices[at..at + 4].copy_from_slice(&(layer as u32).to_le_bytes());
        }
        queue.write_buffer(&layer_index, 0, &indices);
        let blank_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-user-blank"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: MIN_LAYERS as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            blank_texture.as_image_copy(),
            &[0u8; 4 * MIN_LAYERS],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: MIN_LAYERS as u32,
            },
        );
        let blank = blank_texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        UserLayers {
            device: device.clone(),
            queue: queue.clone(),
            mip_pipeline,
            mip_layout,
            sampler,
            layer_index,
            blank,
            state: None,
            version: 0,
            layout_version: 0,
            uid: next_uid(),
            scratch: Vec::new(),
        }
    }

    /// 同じツールの、まっさらな別の配列（ほかのセット用）。
    pub fn sibling(&self) -> UserLayers {
        UserLayers {
            device: self.device.clone(),
            queue: self.queue.clone(),
            mip_pipeline: self.mip_pipeline.clone(),
            mip_layout: self.mip_layout.clone(),
            sampler: self.sampler.clone(),
            layer_index: self.layer_index.clone(),
            blank: self.blank.clone(),
            state: None,
            version: 0,
            layout_version: 0,
            uid: next_uid(),
            scratch: Vec::new(),
        }
    }

    /// シェーダーへ渡す見え方（持っていなければ 1 × 1 × 2 の透明）。
    pub fn view(&self) -> &wgpu::TextureView {
        self.state.as_ref().map_or(&self.blank, |s| &s.view)
    }

    /// 持っているチャンネルの並び（レイヤーの番号の順）。
    pub fn channels(&self) -> &[Channel] {
        self.state.as_ref().map_or(&[], |s| &s.channels)
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// 束ねの鍵（持ち物の入れ物の番号と、作り直しの世代）。
    pub fn bind_key(&self) -> (u64, u64) {
        (self.uid, self.layout_version)
    }

    /// GPU のバイト数（ミップ込み）。
    pub fn bytes(&self) -> u64 {
        self.state.as_ref().map_or(0, |s| {
            mip_bytes(s.size, 4) * layer_count(s.channels.len()) as u64
        })
    }

    /// 縮めた段（文書から。持っていなければ 0）。
    pub fn level(&self) -> u32 {
        self.state.as_ref().map_or(0, |s| s.shift)
    }

    /// 文書に合わせる。`channels` は持つユーザーチャンネル（文書にあるもの。[`MAX_LAYERS`] まで）、`paint_shift` は標準のチャンネルの縮め、
    /// `limit` は辺の上限、`budget` はバイトの予算（[`plan`]）。空なら持ち物を捨てる。
    pub fn sync(
        &mut self,
        doc: &Document,
        channels: &[Channel],
        paint_shift: u32,
        limit: u32,
        budget: u64,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let channels: Vec<Channel> = channels.iter().copied().take(MAX_LAYERS).collect();
        if channels.is_empty() {
            if self.state.take().is_some() {
                self.layout_version += 1;
                self.version += 1;
            }
            return;
        }
        let size = [doc.width(), doc.height()];
        let (shift, _) = plan(size, channels.len(), paint_shift, limit, budget);
        let rebuild = self.state.as_ref().is_none_or(|s| {
            s.doc_id != doc.id()
                || s.doc_size != (doc.width(), doc.height())
                || s.channels != channels
                || s.shift != shift
                || s.serial > doc.change_serial()
        });
        let since = if rebuild {
            self.create(doc, channels.clone(), shift);
            None
        } else {
            self.state.as_ref().map(|s| s.serial)
        };
        let serial = doc.change_serial();
        let mut changed = rebuild;
        for (layer, channel) in channels.iter().enumerate() {
            let rects = match since {
                None => full_rects(doc, *channel, shift),
                Some(since) => changed_rects(doc, *channel, since, shift),
            };
            if rects.is_empty() {
                continue;
            }
            let rects = plan_regions(rects, shift, doc.bounds(), false);
            if let Some(dirty) = self.upload(doc, *channel, layer as u32, &rects) {
                self.rebuild_mips(layer, encoder, dirty);
                changed = true;
            }
        }
        if let Some(s) = self.state.as_mut() {
            s.serial = serial;
        }
        if changed {
            self.version += 1;
        }
    }

    fn create(&mut self, doc: &Document, channels: Vec<Channel>, shift: u32) {
        let size = [
            doc.width().div_ceil(1 << shift).max(1),
            doc.height().div_ceil(1 << shift).max(1),
        ];
        let levels = 32 - size[0].max(size[1]).leading_zeros();
        let layers = layer_count(channels.len()) as u32;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-user-layers"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: layers,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let level_arrays = (0..levels)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let level_views = (0..channels.len() as u32)
            .map(|layer| {
                (0..levels)
                    .map(|level| {
                        texture.create_view(&wgpu::TextureViewDescriptor {
                            dimension: Some(wgpu::TextureViewDimension::D2),
                            base_mip_level: level,
                            mip_level_count: Some(1),
                            base_array_layer: layer,
                            array_layer_count: Some(1),
                            ..Default::default()
                        })
                    })
                    .collect()
            })
            .collect();
        // 作った直後の中身は 0（透明）。描いていないタイルは上げないので、そのまま「何も描いていない」になる
        self.state = Some(State {
            doc_id: doc.id(),
            doc_size: (doc.width(), doc.height()),
            channels,
            shift,
            size,
            levels,
            texture,
            view,
            level_views,
            level_arrays,
            serial: 0,
        });
        self.layout_version += 1;
    }

    /// 矩形ごとに合成してレイヤーへ上げる。段 0 の変わった範囲（x0 y0 x1 y1）。
    fn upload(
        &mut self,
        doc: &Document,
        channel: Channel,
        layer: u32,
        rects: &[(DocRect, usize)],
    ) -> Option<[u32; 4]> {
        let shift = self.state.as_ref()?.shift;
        let mut dirty: Option<[u32; 4]> = None;
        for (rect, _) in rects {
            let n = (rect.width * rect.height * 4) as usize;
            self.scratch.resize(n, 0);
            if doc
                .composite_into(channel, *rect, &mut self.scratch, RowOrder::BottomUp)
                .is_err()
            {
                continue;
            }
            let (data, dw, dh) = reduce_premultiplied(&self.scratch, *rect, shift);
            let (dx, dy) = (rect.x >> shift, rect.y >> shift);
            let state = self.state.as_ref()?;
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &state.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: dx,
                        y: dy,
                        z: layer,
                    },
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
        }
        // 作業用のバッファは文書の帯の大きさになる。持ち続けない
        self.scratch = Vec::new();
        dirty
    }

    fn rebuild_mips(&self, layer: usize, encoder: &mut wgpu::CommandEncoder, dirty: [u32; 4]) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        for level in 1..state.levels as usize {
            let w = (state.size[0] >> level).max(1);
            let h = (state.size[1] >> level).max(1);
            let x0 = (dirty[0] >> level).saturating_sub(1);
            let y0 = (dirty[1] >> level).saturating_sub(1);
            let x1 = (dirty[2].div_ceil(1 << level) + 1).min(w);
            let y1 = (dirty[3].div_ceil(1 << level) + 1).min(h);
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("yolu-3d-user-mip"),
                layout: &self.mip_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &state.level_arrays[level - 1],
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.layer_index,
                            offset: 0,
                            size: wgpu::BufferSize::new(16),
                        }),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("yolu-3d-user-mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &state.level_views[layer][level],
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
            pass.set_pipeline(&self.mip_pipeline);
            pass.set_bind_group(0, &bind, &[(layer as u64 * LAYER_STRIDE) as u32]);
            pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
            pass.draw(0..3, 0..1);
        }
    }
}

/// 全部を作るときに合成するタイル（キャンバス全体に効くレイヤー・効果があれば全タイル、無ければレイヤーが面を持つタイル。`paint` と同じ決まり）。
fn full_rects(doc: &Document, channel: Channel, shift: u32) -> Vec<(DocRect, usize)> {
    let whole = doc.layers().iter().any(|l| {
        (matches!(l.kind(), LayerKind::Fill | LayerKind::Adjustment)
            && l.is_channel_enabled(channel))
            || l.has_active_filters(channel)
            || l.mask().is_some_and(|m| m.has_active_filters())
    });
    let mut coords: Vec<TileCoord> = if whole {
        doc.canvas_tiles().collect()
    } else {
        doc.layers()
            .iter()
            .filter_map(|l| l.surface(channel))
            .flat_map(|s| s.tile_coords())
            .collect()
    };
    coords.sort();
    coords.dedup();
    rects(doc, &coords, shift)
}

fn changed_rects(
    doc: &Document,
    channel: Channel,
    since: u64,
    shift: u32,
) -> Vec<(DocRect, usize)> {
    let coords = doc.changed_tiles(channel, since).unwrap_or_default();
    rects(doc, &coords, shift)
}

fn rects(doc: &Document, coords: &[TileCoord], shift: u32) -> Vec<(DocRect, usize)> {
    let bounds = doc.bounds();
    let mut out: Vec<(DocRect, usize)> = coords
        .iter()
        .filter_map(|c| doc.tile_rect(*c))
        .map(|r| (align(r, shift, bounds), 1))
        .collect();
    out.sort_by_key(|(r, _)| (r.y, r.x, r.width, r.height));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_shrinks_to_the_budget_and_caps_the_layers() {
        // 1024² の 16 レイヤー（ミップ込み約 85 MiB）はセットごとの上限に収まる
        let (shift, bytes) = plan([1024, 1024], 16, 0, 8192, u64::MAX);
        assert_eq!(shift, 0);
        assert_eq!(bytes, mip_bytes([1024, 1024], 64));
        // 17 個目からは持たない（16 レイヤーと同じ）
        assert_eq!(plan([1024, 1024], 17, 0, 8192, u64::MAX), (shift, bytes));
        // 予算が少なければ 2 の累乗で縮める。標準のチャンネルの縮めより細かくはしない
        let (shift, bytes) = plan([1024, 1024], 2, 0, 8192, 1 << 20);
        assert_eq!(shift, 2, "256² × 2 レイヤー × 4 B ≈ 0.67 MiB");
        assert!(bytes <= 1 << 20);
        assert_eq!(plan([1024, 1024], 2, 3, 8192, u64::MAX).0, 3);
        // 1 つでも 2 レイヤーで作る（GL）。0 個は持たない
        assert_eq!(
            plan([64, 64], 1, 0, 8192, u64::MAX).1,
            mip_bytes([64, 64], 8)
        );
        assert_eq!(plan([64, 64], 0, 0, 8192, u64::MAX), (0, 0));
        // 全体の残りが 0 でも、縮めの下限（1 × 1）で止まる（標準のチャンネルの絵と同じ決まり）
        assert_eq!(plan([1024, 1024], 2, 0, 8192, 0), (10, 8));
        // 8192² の 16 レイヤーはセットごとの上限（256 MiB）に収まる 1024² まで縮める（2048² は約 341 MiB）
        let (shift, bytes) = plan([8192, 8192], 16, 0, 8192, u64::MAX);
        assert_eq!(shift, 3);
        assert!(bytes <= USER_BUDGET_BYTES);
    }
}
