//! 3D ビューの選択範囲の重ね: 文書の選択範囲（と、クイックマスクの見た目・選択ペンの途中の被覆）を、文書と同じ大きさの R8 のテクスチャ
//! （ミップつき）にして、面の UV から読み、モデルの面の上に描く（`shaders/selection_overlay.wgsl`）。
//! - 2D のキャンバスの重ね（`selection::overlay::TileOverlay`）と同じ作り: 選択範囲が変わったら、変わったタイルだけを上げる（前に上げた選択範囲と
//!   中身を見比べて決める。いつも見比べるので、呼ぶ側は変わったタイルを知らせない）。ミップは、変わった箱の分だけ作り直す。
//! - 縁の点線は流さない（位相は 0 のまま）。流すと、何もしていない間も選択範囲がある限り 3D を描き直すことになる（描き直しは、軽いモデルでも描かないフレームの
//!   数倍の時間）。2D のキャンバスの点線は流れる。
//! - 縁は UV の上で引く: 面の上で継ぎ目をまたいで続く選択範囲でも、隣のアイランドの画素が選ばれていなければ、継ぎ目に線が出る
//!   （3D の選択のツールは、継ぎ目のにじみの画素も選ぶので、にじみの幅の内では出ない）。
//! - 描くのは今のテクスチャセットの面だけ（選択範囲はセットごと）。深さは面の描きのまま読み（書かない）、面と同じ描き先・サンプル数。
//! - GPU のメモリは 3D の絵の予算（`View3dRenderer::total_budget`）に数える: 今のセットの絵・lilToon の配列を引いた残りに、ほかのセットより先に
//!   入れる。入らなければ重ねを出さず（今までの動きのまま）、`View3dStats::overlay_skipped` で知らせる。文書の幅か高さが GPU のテクスチャの大きさの上限
//!   （`max_texture_dimension_2d`）を超えるときも、重ねを作れないので同じに出さない（`View3dStats::overlay_too_large`）。

use std::collections::{HashMap, HashSet};

use eframe::egui_wgpu::wgpu;
use yolu_core::glam::Vec3;
use yolu_core::{SelectionMask, TileCoord};

/// 変わったタイルがこの数以上なら、変わった箱を 1 回で上げる。
const BATCH_TILES: usize = 16;

/// ミップを積む 1 段（`shaders/mip.wgsl`）の描き先の形式。
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// 縁の線の太さ（半分。画面の点）と、点線の 1 周期（画面の点。黒と白が半分ずつ）。2D の縁の点線と同じ。
const LINE_HALF_POINTS: f32 = 0.75;
const DASH_PERIOD_POINTS: f32 = 8.0;

/// 重ねを出していない理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// 3D の絵の予算（GPU のメモリ）に入らない。
    Budget,
    /// 文書の幅か高さが、GPU のテクスチャの大きさの上限を超える。
    Size,
}

/// 何を重ねるか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// 選択範囲の縁（白と黒の点線）。
    Ants,
    /// 赤い重ね（クイックマスク）。
    Red,
}

/// 選択ペンの途中の被覆（追加: 青、削除: 橙）。
#[derive(Clone)]
pub struct Cover {
    pub mask: SelectionMask,
    pub erase: bool,
}

/// 1 フレームの重ねの入力（パネルが、描く前に `View3dRenderer::set_overlay` で渡す）。
#[derive(Clone)]
pub struct OverlayInput {
    pub style: Style,
    /// 見せる量（選択範囲。クイックマスクの途中なら、被覆を重ねた見た目）。
    pub mask: Option<SelectionMask>,
    pub cover: Option<Cover>,
    /// 画面の 1 点が何画素か（線の太さを点で決める）。
    pub pixels_per_point: f32,
    /// 縁の白の値（ガンマ。仕上げ（トーンマッピング）を通さなければ 1）。
    pub white: f32,
}

impl OverlayInput {
    /// 持ちたい GPU のバイト（ミップ込み。何も見せなければ 0）。
    pub fn wanted_bytes(&self) -> u64 {
        let one = |m: &SelectionMask| texture_bytes([m.width(), m.height()]);
        let main = self.mask.as_ref().filter(|m| !m.is_empty()).map_or(0, one);
        let cover = self.cover.as_ref().map_or(0, |c| one(&c.mask));
        main + cover
    }

    /// 重ねるテクスチャのどれかが、辺の上限 `limit` を超えるか。
    pub fn exceeds(&self, limit: u32) -> bool {
        let over = |m: &SelectionMask| m.width() > limit || m.height() > limit;
        self.mask
            .as_ref()
            .filter(|m| !m.is_empty())
            .is_some_and(over)
            || self.cover.as_ref().is_some_and(|c| over(&c.mask))
    }
}

/// ミップの段の数（辺が 1 になるまで）。
fn level_count(size: [u32; 2]) -> u32 {
    32 - size[0].max(size[1]).max(1).leading_zeros()
}

/// ミップ込みのバイト数（R8）。
pub fn texture_bytes(size: [u32; 2]) -> u64 {
    (0..level_count(size))
        .map(|l| ((size[0] >> l).max(1) as u64) * ((size[1] >> l).max(1) as u64))
        .sum()
}

/// 文書と同じ大きさの 1 枚（量の R8。ミップつき）。
struct MaskTexture {
    size: [u32; 2],
    levels: u32,
    texture: wgpu::Texture,
    /// 段ごとの見え方（1 段だけ。ミップを積む描き先と読み元）。
    views: Vec<wgpu::TextureView>,
    /// 全部の段の見え方（面を描くときに読む）。
    full: wgpu::TextureView,
    /// 上げた選択範囲。
    last: Option<SelectionMask>,
    /// 上げ直すたびに増える。
    version: u64,
}

impl MaskTexture {
    fn new(device: &wgpu::Device, size: [u32; 2]) -> MaskTexture {
        let levels = level_count(size);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-selection"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
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
        let views = (0..levels)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let full = texture.create_view(&Default::default());
        MaskTexture {
            size,
            levels,
            texture,
            views,
            full,
            last: None,
            version: 0,
        }
    }

    /// 段 0 の箱 `[x0, y0, x1, y1]` に、行の幅 `stride` の画素を書く。
    fn write(&self, queue: &wgpu::Queue, [x0, y0, x1, y1]: [u32; 4], data: &[u8], stride: u32) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: x0, y: y0, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(y1 - y0),
            },
            wgpu::Extent3d {
                width: x1 - x0,
                height: y1 - y0,
                depth_or_array_layers: 1,
            },
        );
    }

    /// 選択範囲に合わせる。上げたタイルの数と、変わったか。
    fn update(
        &mut self,
        queue: &wgpu::Queue,
        mask: &SelectionMask,
        mips: &MipBuilder,
        encoder: &mut wgpu::CommandEncoder,
    ) -> (usize, bool) {
        if self.last.as_ref().is_some_and(|l| l.same_as(mask)) {
            return (0, false);
        }
        let ts = mask.tile_size();
        let (w, h) = (mask.width(), mask.height());
        // 見るタイル: 前と今のどちらかに量があるタイル（最初の 1 回は今のもの全部）。中身を見比べて、変わったタイルだけを上げる
        let mut coords: Vec<TileCoord> = mask.tile_coords();
        if let Some(last) = self.last.as_ref() {
            let seen: HashSet<TileCoord> = coords.iter().copied().collect();
            coords.extend(last.tile_coords().into_iter().filter(|c| !seen.contains(c)));
        }
        coords.retain(|c| c.x * ts < w && c.y * ts < h);
        let mut new_tile = vec![0u8; (ts * ts) as usize];
        let mut old_tile = vec![0u8; (ts * ts) as usize];
        let changed: Vec<TileCoord> = match self.last.as_ref() {
            None => coords,
            Some(last) => coords
                .into_iter()
                .filter(|c| {
                    new_tile.fill(0);
                    let _ = mask.copy_tile(*c, &mut new_tile);
                    old_tile.fill(0);
                    let _ = last.copy_tile(*c, &mut old_tile);
                    old_tile != new_tile
                })
                .collect(),
        };
        let uploaded = changed.len();
        let mut dirty: Option<[u32; 4]> = None;
        let extent = |c: &TileCoord| {
            let (x0, y0) = (c.x * ts, c.y * ts);
            [x0, y0, (x0 + ts).min(w), (y0 + ts).min(h)]
        };
        for c in &changed {
            let [x0, y0, x1, y1] = extent(c);
            dirty = Some(match dirty {
                None => [x0, y0, x1, y1],
                Some([a, b, c2, d]) => [a.min(x0), b.min(y0), c2.max(x1), d.max(y1)],
            });
        }
        if let Some([bx0, by0, bx1, by1]) = dirty {
            if changed.len() >= BATCH_TILES {
                // 多いときは、変わった箱を 1 枚の絵にまとめて 1 回で上げる（タイルごとの上げは、1 回ごとの手間が積もる）
                let rw = bx1 - bx0;
                let mut region = vec![0u8; (rw * (by1 - by0)) as usize];
                let (tx0, ty0) = (bx0 / ts, by0 / ts);
                let (tx1, ty1) = (bx1.div_ceil(ts), by1.div_ceil(ts));
                for ty in ty0..ty1 {
                    for tx in tx0..tx1 {
                        let c = TileCoord::new(tx, ty);
                        new_tile.fill(0);
                        if !mask.copy_tile(c, &mut new_tile).unwrap_or(false) {
                            continue;
                        }
                        let [x0, y0, x1, y1] = extent(&c);
                        for row in 0..(y1 - y0) {
                            let from = (row * ts) as usize;
                            let to = ((y0 - by0 + row) * rw + (x0 - bx0)) as usize;
                            let n = (x1 - x0) as usize;
                            region[to..to + n].copy_from_slice(&new_tile[from..from + n]);
                        }
                    }
                }
                self.write(queue, [bx0, by0, bx1, by1], &region, rw);
            } else {
                for c in &changed {
                    new_tile.fill(0);
                    let _ = mask.copy_tile(*c, &mut new_tile);
                    self.write(queue, extent(c), &new_tile, ts);
                }
            }
        }
        self.last = Some(mask.clone());
        if let Some(region) = dirty {
            mips.build(encoder, self, region);
            self.version += 1;
        }
        (uploaded, true)
    }
}

/// ミップを 1 段ずつ積む（1 つ上の段の 2 × 2 を双線形で読んで下の段へ描く。変わった箱の分だけ）。
struct MipBuilder {
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl MipBuilder {
    fn new(device: &wgpu::Device) -> MipBuilder {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yolu-3d-selection-mip"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/mip.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yolu-3d-selection-mip"),
            entries: &[
                texture_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yolu-3d-selection-mip"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("yolu-3d-selection-mip"),
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
            label: Some("yolu-3d-selection-mip"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        MipBuilder {
            device: device.clone(),
            pipeline,
            layout,
            sampler,
        }
    }

    /// 段 1 から順に、変わった箱 `region`（段 0 のテクセル。x0, y0, x1, y1）の分だけ作り直す（読む近傍の 1 テクセルぶん広げる）。
    fn build(&self, encoder: &mut wgpu::CommandEncoder, target: &MaskTexture, region: [u32; 4]) {
        for level in 1..target.levels {
            let w = (target.size[0] >> level).max(1);
            let h = (target.size[1] >> level).max(1);
            let x0 = (region[0] >> level).saturating_sub(1);
            let y0 = (region[1] >> level).saturating_sub(1);
            let x1 = (region[2].div_ceil(1 << level) + 1).min(w);
            let y1 = (region[3].div_ceil(1 << level) + 1).min(h);
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("yolu-3d-selection-mip"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &target.views[level as usize - 1],
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("yolu-3d-selection-mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.views[level as usize],
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
            pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// 重ねの GPU の持ち物（`View3dRenderer` が 1 つ持つ）。
pub struct OverlayGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    mips: MipBuilder,
    module: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    /// 被覆が無いときに束ねる 1 × 1 の 0。
    dummy: wgpu::TextureView,
    _dummy_texture: wgpu::Texture,
    /// 描き先の形式（HDR か 8 bit）とサンプル数ごとのパイプライン。
    pipelines: HashMap<(bool, u32), wgpu::RenderPipeline>,
    input: Option<OverlayInput>,
    main: Option<MaskTexture>,
    cover: Option<MaskTexture>,
    bind: Option<(wgpu::BindGroup, [u64; 4])>,
    /// 今のフレームで描くか。
    active: bool,
    /// 持ちたいが予算に入らずに出していないか。
    skipped: bool,
    /// 描き直しの鍵（見えるものが変わると変わる）。
    key: u64,
    /// テクスチャを作るたびに増える（鍵と束ねの見分け）。
    serial: u64,
    /// 上げたタイルの数の累計（変わったタイルだけ。まとめて 1 回で上げたときも、含めたタイルの数）。
    pub uploaded_tiles: u64,
    /// GPU のテクスチャの辺の上限（GPU が決める）。
    limit: u32,
}

impl OverlayGpu {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene_layout: &wgpu::BindGroupLayout,
    ) -> OverlayGpu {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yolu-3d-selection-overlay"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/selection_overlay.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yolu-3d-selection-overlay"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
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
            label: Some("yolu-3d-selection-overlay"),
            bind_group_layouts: &[Some(scene_layout), Some(&layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("yolu-3d-selection-overlay"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yolu-3d-selection-overlay"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let dummy_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-selection-dummy"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            dummy_texture.as_image_copy(),
            &[0u8],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(1),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let dummy = dummy_texture.create_view(&Default::default());
        OverlayGpu {
            device: device.clone(),
            queue: queue.clone(),
            mips: MipBuilder::new(device),
            module,
            pipeline_layout,
            layout,
            sampler,
            params,
            dummy,
            _dummy_texture: dummy_texture,
            pipelines: HashMap::new(),
            input: None,
            main: None,
            cover: None,
            bind: None,
            active: false,
            skipped: false,
            key: 0,
            serial: 0,
            uploaded_tiles: 0,
            limit: device.limits().max_texture_dimension_2d,
        }
    }

    /// 次の `sync` が見せる入力（None なら何も重ねない）。
    pub fn set_input(&mut self, input: Option<OverlayInput>) {
        self.input = input;
    }

    /// 次の `sync` で持ちたい GPU のバイト（ミップ込み。何も見せなければ 0）。
    pub fn wanted_bytes(&self) -> u64 {
        self.input.as_ref().map_or(0, OverlayInput::wanted_bytes)
    }

    /// 持ちたいテクスチャの辺が、GPU のテクスチャの辺の上限を超えるか（超えるときは作れないので出さない）。
    pub fn too_large(&self) -> bool {
        self.input.as_ref().is_some_and(|i| i.exceeds(self.limit))
    }

    /// GPU のテクスチャの辺の上限。
    pub fn limit(&self) -> u32 {
        self.limit
    }

    /// 今持っている GPU のバイト（ミップ込み）。
    pub fn bytes(&self) -> u64 {
        [&self.main, &self.cover]
            .into_iter()
            .flatten()
            .map(|t| texture_bytes(t.size))
            .sum()
    }

    /// 持ちたいが予算に入らずに出していないか。
    pub fn skipped(&self) -> bool {
        self.skipped
    }

    /// このフレームに描くか。
    pub fn active(&self) -> bool {
        self.active
    }

    /// 描き直しの鍵（見えるものが変わると変わる）。
    pub fn key(&self) -> u64 {
        self.key
    }

    /// 入力を GPU に合わせる。`allowed` は、予算に入るか（入らなければ持たず、`skipped` にする）。`hdr`・`samples` は描き先の形。
    pub fn sync(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        allowed: bool,
        hdr: bool,
        samples: u32,
        vertex_floats: usize,
    ) {
        let wanted = self.wanted_bytes();
        let Some(input) = self.input.clone().filter(|_| wanted > 0) else {
            self.release();
            self.active = false;
            self.skipped = false;
            self.key = 0;
            return;
        };
        if !allowed {
            self.release();
            self.active = false;
            self.skipped = true;
            self.key = 0;
            return;
        }
        self.skipped = false;
        let size_of = |m: &SelectionMask| [m.width(), m.height()];
        // 見せる量（空なら持たない）と被覆
        let mask = input.mask.as_ref().filter(|m| !m.is_empty());
        for (slot, source) in [
            (&mut self.main, mask),
            (&mut self.cover, input.cover.as_ref().map(|c| &c.mask)),
        ] {
            match source {
                None => *slot = None,
                Some(mask) => {
                    if slot.as_ref().is_none_or(|t| t.size != size_of(mask)) {
                        self.serial += 1;
                        let mut texture = MaskTexture::new(&self.device, size_of(mask));
                        texture.version = self.serial << 24;
                        *slot = Some(texture);
                    }
                    let texture = slot.as_mut().expect("作った");
                    let (tiles, changed) = texture.update(&self.queue, mask, &self.mips, encoder);
                    if changed {
                        self.uploaded_tiles += tiles as u64;
                    }
                }
            }
        }
        self.ensure_pipeline(hdr, samples, vertex_floats);
        // 束ね（テクスチャが替わったときだけ）
        let ident = |t: &Option<MaskTexture>| t.as_ref().map_or(0, |t| t.version + 1);
        let bind_key = [ident(&self.main), ident(&self.cover), 0, 0];
        if self.bind.as_ref().is_none_or(|(_, k)| *k != bind_key) {
            let main = self.main.as_ref().map_or(&self.dummy, |t| &t.full);
            let cover = self.cover.as_ref().map_or(&self.dummy, |t| &t.full);
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("yolu-3d-selection-overlay"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(main),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(cover),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.params.as_entire_binding(),
                    },
                ],
            });
            self.bind = Some((bind, bind_key));
        }
        // 値
        let ppp = input.pixels_per_point.max(0.1);
        let style = if self.main.is_some() {
            match input.style {
                Style::Ants => 1.0,
                Style::Red => 2.0,
            }
        } else {
            0.0
        };
        let cover = input
            .cover
            .as_ref()
            .map_or(0.0, |c| if c.erase { 2.0 } else { 1.0 });
        let values: [f32; 8] = [
            style,
            0.0,
            LINE_HALF_POINTS * ppp,
            DASH_PERIOD_POINTS * ppp,
            input.white,
            cover,
            0.0,
            0.0,
        ];
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.queue.write_buffer(&self.params, 0, &bytes);
        self.active = true;
        // 鍵: 見えるものが変わると変わる
        let mix = |a: u64, b: u64| (a ^ b).wrapping_mul(0x0000_0100_0000_01b3);
        let mut key = 0xcbf2_9ce4_8422_2325u64;
        for part in [
            ident(&self.main),
            ident(&self.cover),
            style.to_bits() as u64,
            ppp.to_bits() as u64,
            input.white.to_bits() as u64,
            cover.to_bits() as u64,
        ] {
            key = mix(key, part);
        }
        self.key = key;
    }

    /// 持っているテクスチャを手放す（GPU のメモリを返す）。
    fn release(&mut self) {
        self.main = None;
        self.cover = None;
        self.bind = None;
    }

    fn ensure_pipeline(&mut self, hdr: bool, samples: u32, vertex_floats: usize) {
        if self.pipelines.contains_key(&(hdr, samples)) {
            return;
        }
        // サンプル数が替わったら、前の数のものは捨てる
        self.pipelines.retain(|(_, s), _| *s == samples);
        let attributes = [
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 24,
                shader_location: 2,
            },
        ];
        let format = if hdr {
            super::render::HDR_FORMAT
        } else {
            super::render::LDR_FORMAT
        };
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("yolu-3d-selection-overlay"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: (vertex_floats * 4) as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    })],
                },
                // 面と同じ（表は時計回り、裏は描かない）
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                // 深さは面の描きのまま読む（書かない）。同じ面を描き直すので、少し手前へ寄せて、同じ深さの面が落ちないようにする
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: super::render::DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState {
                        constant: -2,
                        slope_scale: -1.0,
                        clamp: 0.0,
                    },
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        self.pipelines.insert((hdr, samples), pipeline);
    }

    /// 面の描きのパスの終わりに、今のセットの面（`ranges` は頂点の範囲）へ重ねる。
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        globals: &wgpu::BindGroup,
        vertices: &wgpu::Buffer,
        ranges: &[(u32, u32)],
        hdr: bool,
        samples: u32,
    ) {
        let (true, Some((bind, _)), Some(pipeline)) =
            (self.active, &self.bind, self.pipelines.get(&(hdr, samples)))
        else {
            return;
        };
        if ranges.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, globals, &[]);
        pass.set_bind_group(1, bind, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        for &(start, end) in ranges {
            pass.draw(start..end, 0..1);
        }
    }
}

/// アプリの状態から、このフレームの重ねの入力を作る（選択範囲が無く、見せるものも無ければ None）。描いている間の被覆・見た目を札へ反映するので、
/// 2D のキャンバスが出ていなくても読める。縁の点線は止めたままなので、描き直しは頼まない（何もしていない間は 3D を描き直さない）。
pub fn input(app: &mut crate::state::AppState, ctx: &egui::Context) -> Option<OverlayInput> {
    crate::selection::pen::sync(app);
    let (style, mask) = if app.sel.quick {
        // クイックマスク: 描いている間はその見た目（被覆を重ねたもの）、そうでなければ選択範囲
        let preview = app
            .sel
            .pen
            .as_ref()
            .filter(|a| a.quick)
            .and_then(|a| a.stroke.preview().cloned());
        (Style::Red, preview.or_else(|| app.doc.selection().cloned()))
    } else {
        (Style::Ants, app.doc.selection().cloned())
    };
    let cover = app.sel.pen.as_ref().filter(|a| !a.quick).map(|a| Cover {
        mask: a.stroke.cover().clone(),
        erase: a.stroke.erase,
    });
    if mask.as_ref().is_none_or(SelectionMask::is_empty) && cover.is_none() {
        return None;
    }
    Some(OverlayInput {
        style,
        mask,
        cover,
        pixels_per_point: ctx.pixels_per_point(),
        white: edge_white(&app.view3d.display),
    })
}

/// 縁の白の値（ガンマ）: 仕上げ（露出・曲線）を通したあとに白く（0.95 ほど）見える値。通さなければ 1。曲線は 1 を超える値を切るので、
/// 通したあとの値が 0.95 になる値を、二分で探す（黒の線は 0 のまま 0）。
pub fn edge_white(display: &super::display::Display) -> f32 {
    if !display.uses_hdr_path() {
        return 1.0;
    }
    let after =
        |g: f32| super::brdf::tone_map(Vec3::splat(g), display.tone_map, display.exposure).x;
    let (mut lo, mut hi) = (0.0f32, 64.0f32);
    if after(hi) < 0.95 {
        return hi;
    }
    for _ in 0..24 {
        let mid = (lo + hi) * 0.5;
        if after(mid) < 0.95 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view3d::brdf::Curve;
    use crate::view3d::display::Display;

    #[test]
    fn the_bytes_count_every_mip_level_down_to_one_texel() {
        assert_eq!(level_count([1, 1]), 1);
        assert_eq!(level_count([4, 4]), 3);
        assert_eq!(level_count([8, 2]), 4);
        assert_eq!(texture_bytes([1, 1]), 1);
        assert_eq!(texture_bytes([4, 4]), 16 + 4 + 1);
        assert_eq!(texture_bytes([8, 2]), 16 + 4 + 2 + 1);
        // 2048²: 約 4/3 倍
        let n = texture_bytes([2048, 2048]);
        assert!(n > 2048 * 2048 * 4 / 3 - 4 && n < 2048 * 2048 * 4 / 3 + 4096);
    }

    #[test]
    fn the_edge_white_is_one_without_the_finish_and_brighter_through_a_curve_so_it_looks_white() {
        let mut display = Display {
            tone_map: Curve::None,
            exposure: 0.0,
            ..Display::default()
        };
        if !display.uses_hdr_path() {
            assert_eq!(edge_white(&display), 1.0);
        }
        for (curve, ev) in [
            (Curve::Aces, 0.0),
            (Curve::Neutral, 0.0),
            (Curve::Aces, 1.0),
            (Curve::Aces, -2.0),
        ] {
            display.tone_map = curve;
            display.exposure = ev;
            assert!(display.uses_hdr_path());
            let white = edge_white(&display);
            let after = crate::view3d::brdf::tone_map(Vec3::splat(white), curve, ev).x;
            assert!(
                (after - 0.95).abs() < 0.01,
                "{curve:?} {ev}: {white} → {after}"
            );
        }
    }
}
