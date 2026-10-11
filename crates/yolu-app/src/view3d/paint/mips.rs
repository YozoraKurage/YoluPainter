//! ミップマップ: 全部のテクセルの箱の平均と、UV の上の画素だけで作る押し引き（重みの絵）。

use super::*;

/// 重みつきのミップ（`shaders/mip_weighted.wgsl`）の束ねの形とパイプライン。3 つの読み（色・重み・描き先の重み）は、押しも引きも同じ形。
pub(super) fn make_weighted_mips(device: &wgpu::Device) -> WeightedMips {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("yolu-3d-mip-weighted"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/mip_weighted.wgsl").into()),
    });
    let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("yolu-3d-mip-weighted"),
        entries: &[texture_entry(0), texture_entry(1), texture_entry(2)],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("yolu-3d-mip-weighted"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let make = |entry: &'static str, format| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("yolu-3d-mip-weighted"),
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
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let formats = [RGBA, RGBA_SRGB, SCALAR];
    WeightedMips {
        layout,
        push: formats.map(|f| make("fs_push", f)),
        pull: formats.map(|f| make("fs_pull", f)),
    }
}

impl Paint {
    /// 重みの絵を、今のセットの形・縮めに合わせる（塗り広げないなら持たない）。作るには段の地図が要る（`rings`）。
    pub(super) fn ensure_weights(&mut self, doc: &Document) {
        let key = self.set.as_ref().and_then(|set| {
            let shape = set.pad.filter(|_| self.pad_texels > 0)?;
            Some(WeightKey {
                shape,
                doc_size: (doc.width(), doc.height()),
                reach: (self.pad_texels << set.shift).min(padding::MAX_RING_REACH),
                shift: set.shift,
            })
        });
        let Some(key) = key else {
            self.weights = None;
            return;
        };
        if self.weights.as_ref().is_some_and(|w| w.key == key) {
            return;
        }
        // 前の絵を先に手放す（作る間に 2 つ持たない）
        self.weights = None;
        let Some(rings) = self.rings(doc) else {
            return;
        };
        let set = self.set.as_ref().expect("確かめた");
        let (size, levels) = (set.size, set.levels);
        let pyramid = weight_pyramid(coverage_weights(&rings, key.shift, size), size, levels);
        let mut textures = Vec::with_capacity(levels as usize);
        let mut views = Vec::with_capacity(levels as usize);
        let mut holes = Vec::with_capacity(levels as usize);
        for (level, texels) in pyramid.iter().enumerate() {
            let (w, h) = ((size[0] >> level).max(1), (size[1] >> level).max(1));
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("yolu-3d-mip-weights"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: SCALAR,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            self.queue.write_texture(
                texture.as_image_copy(),
                texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            views.push(texture.create_view(&Default::default()));
            textures.push(texture);
            holes.push(texels.contains(&0));
        }
        self.weights = Some(Weights {
            uid: next_uid(),
            key,
            size,
            textures,
            views,
            holes,
            bytes: mip_bytes(size, 1),
        });
    }

    /// チャンネルのテクスチャの、重みつきのミップの束ねを、今の重みの絵に合わせる。
    pub(super) fn ensure_weighted_binds(&mut self, slot: Slot) {
        let (Some(weights), Some(set)) = (self.weights.as_ref(), self.set.as_mut()) else {
            return;
        };
        let Some(texture) = set.textures[slot.index()].as_mut() else {
            return;
        };
        if texture
            .weighted_binds
            .as_ref()
            .is_some_and(|b| b.weights_uid == weights.uid)
            || weights.views.len() != texture.levels.len()
            || weights.size != texture.size
        {
            return;
        }
        let levels = texture.levels.len();
        let bind = |color: &wgpu::TextureView, source: usize, target: usize| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("yolu-3d-mip-weighted"),
                layout: &self.mips.weighted.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(color),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&weights.views[source]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&weights.views[target]),
                    },
                ],
            })
        };
        let push = (1..levels)
            .map(|l| bind(&texture.levels[l - 1], l - 1, l))
            .collect();
        let pull = (1..levels.saturating_sub(1))
            .map(|l| bind(&texture.levels[l + 1], l + 1, l))
            .collect();
        texture.weighted_binds = Some(WeightedBinds {
            weights_uid: weights.uid,
            push,
            pull,
        });
    }

    /// 段 0 の dirty（x0 y0 x1 y1）の下の段を作り直す（その範囲だけを鋏で切って描く）。`weights` があれば UV の上の画素だけで作る（押し引き。
    /// 束ねが重みの絵に合っていること）。無ければ、全部のテクセルの箱の平均。
    pub(super) fn rebuild_mips(
        &self,
        format: wgpu::TextureFormat,
        texture: &ChannelTexture,
        weights: Option<&Weights>,
        encoder: &mut wgpu::CommandEncoder,
        dirty: [u32; 4],
    ) -> MipsBuilt {
        if let (Some(weights), Some(binds)) = (weights, texture.weighted_binds.as_ref()) {
            if binds.weights_uid == weights.uid {
                self.rebuild_mips_weighted(format, texture, weights, binds, encoder, dirty);
                return MipsBuilt::Weighted;
            }
        }
        let pipeline = match format {
            SCALAR => &self.mips.scalar,
            RGBA_SRGB => &self.mips.srgb,
            _ => &self.mips.rgba,
        };
        let mut touched: Vec<Option<[u32; 4]>> = vec![None];
        for level in 1..texture.levels.len() {
            let w = (texture.size[0] >> level).max(1);
            let h = (texture.size[1] >> level).max(1);
            // 奇数の大きさの補間がはみ出す分、1 画素ずつ広げる
            let x0 = (dirty[0] >> level).saturating_sub(1);
            let y0 = (dirty[1] >> level).saturating_sub(1);
            let x1 = (dirty[2].div_ceil(1 << level) + 1).min(w);
            let y1 = (dirty[3].div_ceil(1 << level) + 1).min(h);
            if x1 <= x0 || y1 <= y0 {
                touched.push(None);
                continue;
            }
            touched.push(Some([x0, y0, x1, y1]));
            let mut pass = begin_mip_pass(encoder, &texture.levels[level]);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &texture.mip_binds[level - 1], &[]);
            pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
            pass.draw(0..3, 0..1);
        }
        MipsBuilt::Plain(touched)
    }

    /// 押し引きのミップ: 段 0 の変わった範囲から、押し（細かい段から粗い段へ、重みつきの平均）で変わる範囲を段ごとに出して作り、そのあと引き（粗い段から
    /// 細かい段へ、重みが 0 のテクセルを 1 つ粗い段から埋める）で、押しで書き換わった範囲と、変わった粗いテクセルを読むテクセルを埋め直す。
    ///
    /// 重みが 0 でないテクセルの近く（となり）で埋めるテクセルは、読む 4 つの中に重みが 0 でないテクセルがあり、その値だけで決まる（それが押した値で、
    /// 引きで書き換わらない）ので、引きの範囲が押しの範囲のすぐ外までで足りる。重みが 0 でないテクセルから離れたテクセル（描画では読まれない）は、
    /// 粗い段の埋めた値を読むので、範囲の外では前の値のまま残ることがある。
    pub(super) fn rebuild_mips_weighted(
        &self,
        format: wgpu::TextureFormat,
        texture: &ChannelTexture,
        weights: &Weights,
        binds: &WeightedBinds,
        encoder: &mut wgpu::CommandEncoder,
        dirty: [u32; 4],
    ) {
        let index = WeightedMips::index(format);
        let levels = texture.levels.len();
        let dim = |l: usize| ((texture.size[0] >> l).max(1), (texture.size[1] >> l).max(1));
        let draw = |encoder: &mut wgpu::CommandEncoder,
                    level: usize,
                    pipeline: &wgpu::RenderPipeline,
                    bind: &wgpu::BindGroup,
                    r: [u32; 4]| {
            let mut pass = begin_mip_pass(encoder, &texture.levels[level]);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.set_scissor_rect(r[0], r[1], r[2] - r[0], r[3] - r[1]);
            pass.draw(0..3, 0..1);
        };
        // 押し（段 0 は押さない）。粗い絵を見せている間に全部のテクセルの箱の平均で書き換えた範囲も、押し直す
        let mut pushed: Vec<Option<[u32; 4]>> = vec![None];
        let mut previous = clip_rect(dirty, dim(0));
        for level in 1..levels {
            let derived = previous.map(|r| parent_rect(r, dim(level - 1), dim(level)));
            let carried = texture.plain_dirty.get(level).copied().flatten();
            previous = match (derived, carried) {
                (Some(a), Some(b)) => Some(union_rect(a, b)),
                (a, b) => a.or(b),
            };
            if let Some(rect) = previous {
                draw(
                    encoder,
                    level,
                    &self.mips.weighted.push[index],
                    &binds.push[level - 1],
                    rect,
                );
            }
            pushed.push(previous);
        }
        // 引き
        for level in (1..levels.saturating_sub(1)).rev() {
            if !weights.holes[level] {
                continue;
            }
            let Some(own) = pushed[level] else { continue };
            let rect = match pushed[level + 1] {
                Some(coarse) => union_rect(own, children_rect(coarse, dim(level), dim(level + 1))),
                None => own,
            };
            draw(
                encoder,
                level,
                &self.mips.weighted.pull[index],
                &binds.pull[level - 1],
                rect,
            );
        }
    }
}

/// ミップの 1 段へ描く（`Load`: 範囲の外は前のまま）パス。
pub(super) fn begin_mip_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &'a wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("yolu-3d-mip"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
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
    })
}

// ───────── 重みの絵 ─────────

/// 段 `n`（辺の長さ）の画素 `y` が入る、1 つ下の段（`m`）の箱の番号。箱の真ん中を、上の段の座標へ比例して写した位置に置く（シェーダーの `first_of` と
/// 同じ写し）。
pub(in crate::view3d) fn box_of(y: u32, n: u32, m: u32) -> u32 {
    ((2 * y as u64 + 1) * m as u64 / (2 * n as u64)) as u32
}

/// 段 `m`（辺の長さ）の箱 `o` が受け持つ、1 つ上の段（`n`）の画素の始まり（終わりは `o + 1` の始まり）。
pub(in crate::view3d) fn box_start(o: u32, n: u32, m: u32) -> u32 {
    ((2 * o as u64 * n as u64 + m as u64 - 1) / (2 * m as u64)) as u32
}

/// 段 0 の重み: 表示のテクセルごとに、その箱（縮めの 2^shift 四方の文書の画素。端の欠けた箱は中の画素だけ）の全部の画素が、覆い・塗り広げの中なら 255、
/// 1 つでも外なら 0。縮めて持つ絵の色は箱の全部の画素の平均なので、箱の一部だけが中のテクセル（塗り広げの幅の外の縁）の色には、外の画素の
/// 色（塗りつぶしの色など）が混ざっている。そのテクセルの重みを 0 にして、押しに使わない。アイランドの縁の箱は、塗り広げが箱より外まで届くので
/// 全部が中になる。縮めないなら中のテクセルが 255、外が 0。行は下から（文書の y）。
pub(in crate::view3d) fn coverage_weights(rings: &Rings, shift: u32, size: [u32; 2]) -> Vec<u8> {
    let (dw, dh) = (rings.width(), rings.height());
    let block = 1u32 << shift;
    let mut out = vec![0u8; size[0] as usize * size[1] as usize];
    out.par_chunks_mut(size[0] as usize)
        .enumerate()
        .for_each(|(j, row)| {
            let j = j as u32;
            let (y0, y1) = (j * block, ((j + 1) * block).min(dh));
            for (i, o) in row.iter_mut().enumerate() {
                let i = i as u32;
                let (x0, x1) = (i * block, ((i + 1) * block).min(dw));
                let whole = (y0..y1).all(|y| (x0..x1).all(|x| rings.ring(x, y).is_some()));
                *o = if whole { 255 } else { 0 };
            }
        });
    out
}

/// 段 0 の重みから、全部の段の重み（段 1 以降は 1 つ上の段の箱の平均の切り上げ。含めば最低 1、全部 255 なら 255）。
pub(in crate::view3d) fn weight_pyramid(
    level0: Vec<u8>,
    size: [u32; 2],
    levels: u32,
) -> Vec<Vec<u8>> {
    let mut out = vec![level0];
    for level in 1..levels as usize {
        let (n, m) = (
            [
                (size[0] >> (level - 1)).max(1),
                (size[1] >> (level - 1)).max(1),
            ],
            [(size[0] >> level).max(1), (size[1] >> level).max(1)],
        );
        let above = &out[level - 1];
        let mut next = vec![0u8; m[0] as usize * m[1] as usize];
        next.par_chunks_mut(m[0] as usize)
            .enumerate()
            .for_each(|(j, row)| {
                let j = j as u32;
                let (y0, y1) = (box_start(j, n[1], m[1]), box_start(j + 1, n[1], m[1]));
                for (i, o) in row.iter_mut().enumerate() {
                    let i = i as u32;
                    let (x0, x1) = (box_start(i, n[0], m[0]), box_start(i + 1, n[0], m[0]));
                    let mut sum = 0u32;
                    for y in y0..y1 {
                        for x in x0..x1 {
                            sum += above[(y * n[0] + x) as usize] as u32;
                        }
                    }
                    *o = sum.div_ceil((x1 - x0) * (y1 - y0)) as u8;
                }
            });
        out.push(next);
    }
    out
}

/// 段ごとの範囲を合わせる（`into` の長さを足りるだけ伸ばして、段ごとに範囲の和を取る）。
pub(super) fn merge_levels(into: &mut Vec<Option<[u32; 4]>>, other: &[Option<[u32; 4]>]) {
    if into.len() < other.len() {
        into.resize(other.len(), None);
    }
    for (level, rect) in other.iter().enumerate() {
        if let Some(rect) = rect {
            into[level] = Some(into[level].map_or(*rect, |own| union_rect(own, *rect)));
        }
    }
}

/// 範囲（x0 y0 x1 y1）を大きさ `dim` の中に切る。空なら None。
pub(super) fn clip_rect(r: [u32; 4], dim: (u32, u32)) -> Option<[u32; 4]> {
    let r = [
        r[0].min(dim.0),
        r[1].min(dim.1),
        r[2].min(dim.0),
        r[3].min(dim.1),
    ];
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}

/// 範囲（段 `n` の画素）を入れる、1 つ下の段（`m`）の箱の範囲（押しで書き換わる範囲）。
pub(super) fn parent_rect(r: [u32; 4], n: (u32, u32), m: (u32, u32)) -> [u32; 4] {
    [
        box_of(r[0], n.0, m.0),
        box_of(r[1], n.1, m.1),
        box_of(r[2] - 1, n.0, m.0) + 1,
        box_of(r[3] - 1, n.1, m.1) + 1,
    ]
}

/// 1 つ粗い段（`m`）の範囲を、双線形で読む段 `n` のテクセルの範囲（読む 4 つのどれかが範囲に入りうるテクセル。余裕を持って広げる）。
pub(super) fn children_rect(r: [u32; 4], n: (u32, u32), m: (u32, u32)) -> [u32; 4] {
    let lo = |v: u32, n: u32, m: u32| ((v as u64 * n as u64 / m as u64) as u32).saturating_sub(2);
    let hi = |v: u32, n: u32, m: u32| ((v as u64 * n as u64).div_ceil(m as u64) as u32 + 2).min(n);
    [
        lo(r[0], n.0, m.0),
        lo(r[1], n.1, m.1),
        hi(r[2], n.0, m.0),
        hi(r[3], n.1, m.1),
    ]
}

pub(super) fn union_rect(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

// ───────── 縮めの決め方 ─────────
