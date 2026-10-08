//! チャンネルのテクスチャ: 作り・1 枚の絵・平らな法線・読み戻し（試験用）・バイト数。

use super::*;

impl Paint {
    /// 試験用: チャンネルの 1 段の中身を CPU へ読む（形式のバイト列、行は下から。大きさつき）。使っていないチャンネル・段が無いときは None。
    pub fn read_level(&self, slot: Slot, level: u32) -> Option<(Vec<u8>, [u32; 2])> {
        let texture = self.set.as_ref()?.textures[slot.index()].as_ref()?;
        if level as usize >= texture.levels.len() {
            return None;
        }
        let (w, h) = (
            (texture.size[0] >> level).max(1),
            (texture.size[1] >> level).max(1),
        );
        Some((
            self.read_texture_level(&texture.texture, level, [w, h], slot.bytes_per_texel()),
            [w, h],
        ))
    }

    /// 試験用: 重みの絵の 1 段の中身を CPU へ読む（1 テクセル 1 バイト、行は下から。大きさつき）。塗り広げていない（重みの絵が無い）・段が無いときは None。
    pub fn read_weight_level(&self, level: u32) -> Option<(Vec<u8>, [u32; 2])> {
        let weights = self.weights.as_ref()?;
        if level as usize >= weights.views.len() {
            return None;
        }
        let (w, h) = (
            (weights.size[0] >> level).max(1),
            (weights.size[1] >> level).max(1),
        );
        Some((
            self.read_texture_level(&weights.textures[level as usize], 0, [w, h], 1),
            [w, h],
        ))
    }

    pub(super) fn read_texture_level(
        &self,
        texture: &wgpu::Texture,
        level: u32,
        [w, h]: [u32; 2],
        bytes_per_texel: u32,
    ) -> Vec<u8> {
        let row = (w * bytes_per_texel).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yolu-3d-read"),
            size: row as u64 * h as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("yolu-3d-read"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |r| {
            r.expect("読み出しの対応付け");
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let mapped = buffer.slice(..).get_mapped_range().expect("読み出しの範囲");
        let line = (w * bytes_per_texel) as usize;
        let mut out = Vec::with_capacity(line * h as usize);
        for y in 0..h as usize {
            out.extend_from_slice(&mapped[y * row as usize..y * row as usize + line]);
        }
        drop(mapped);
        buffer.unmap();
        out
    }

    pub(super) fn gpu_bytes(&self) -> u64 {
        let Some(set) = &self.set else { return 0 };
        let channels: u64 = Slot::ALL
            .iter()
            .filter(|s| set.textures[s.index()].is_some())
            .map(|s| {
                (0..set.levels)
                    .map(|l| {
                        let w = (set.size[0] >> l).max(1) as u64;
                        let h = (set.size[1] >> l).max(1) as u64;
                        w * h * s.bytes_per_texel() as u64
                    })
                    .sum::<u64>()
            })
            .sum();
        channels + self.weights.as_ref().map_or(0, |w| w.bytes)
    }

    /// ミップを作れる形のテクスチャ（段ごとの見え方と、上の段を読む束ねつき）。
    pub(super) fn make_texture(
        &self,
        format: wgpu::TextureFormat,
        size: [u32; 2],
        levels: u32,
    ) -> ChannelTexture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-paint"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
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
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("yolu-3d-mip"),
                    layout: &self.mips.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&level_views[i - 1]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.mips.sampler),
                        },
                    ],
                })
            })
            .collect();
        let view = texture.create_view(&Default::default());
        ChannelTexture {
            size,
            texture,
            levels: level_views,
            mip_binds,
            view,
            weighted_binds: None,
            plain_dirty: Vec::new(),
        }
    }

    /// 文書の大きさと関係なく、1 枚の絵（straight RGBA8、行は下から。メッシュマップの表示用の 8 bit・マットキャップの画像）をミップつきの
    /// テクスチャにして返す（乗算済みにして上げる）。`srgb` の絵は sRGB の形式で、リニアで乗算済みにする（読むとリニアの乗算済み。
    /// Color と同じ）。そうでない絵は値のまま乗算済みにする（メッシュマップはガンマの値のまま見せる）。GPU のテクスチャの辺の上限か
    /// 予算（`budget` の半分。ミップ込み）を超える大きさは、2 の累乗で縮めて持つ（`ImageTexture::level`）。大きさ 0・バイト数が合わない絵は None。
    pub fn create_image(
        &self,
        rgba: &[u8],
        size: [u32; 2],
        srgb: bool,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<ImageTexture> {
        if size[0] == 0 || size[1] == 0 || rgba.len() as u64 != size[0] as u64 * size[1] as u64 * 4
        {
            return None;
        }
        let limit = self.device.limits().max_texture_dimension_2d;
        let shift = choose_shift(size, 4, limit, self.budget / 2);
        let region = DocRect::new(0, 0, size[0], size[1]);
        let (data, w, h) = if srgb {
            reduce_srgb_premultiplied(rgba, region, shift)
        } else {
            reduce_premultiplied(rgba, region, shift)
        };
        let format = if srgb { RGBA_SRGB } else { RGBA };
        let levels = 32 - w.max(h).leading_zeros();
        let texture = self.make_texture(format, [w, h], levels);
        self.queue.write_texture(
            texture.texture.as_image_copy(),
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let _ = self.rebuild_mips(format, &texture, None, encoder, [0, 0, w, h]);
        Some(ImageTexture {
            texture,
            level: shift,
        })
    }

    /// Normal のテクスチャの段 0 を平らな法線で塗る（上げたタイルの外 = 塗っていない所は平ら）。別の積みですぐに出す: 後から上げるタイルの
    /// 書き込み（`write_texture`）は次の `submit` の頭で行われるので、同じ積みの中で塗ると上げたタイルを上書きしてしまう。
    pub(super) fn clear_flat_normal(&self) {
        let set = self.set.as_ref().expect("作った");
        let texture = set.textures[Slot::Normal.index()].as_ref().expect("作った");
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("yolu-3d-normal-flat"),
            });
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("yolu-3d-normal-flat"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &texture.levels[0],
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 128.0 / 255.0,
                        g: 128.0 / 255.0,
                        b: 1.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
        self.queue.submit(Some(encoder.finish()));
    }
}
