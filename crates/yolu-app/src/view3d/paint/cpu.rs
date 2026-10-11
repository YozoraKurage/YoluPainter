//! CPU の道: 文書を CPU で合成し、チャンネルの形式へ直して（`reduce*`）、塗り広げて上げる。ドラッグの間の粗い上げも。

use super::*;

impl Paint {
    /// 矩形（文書の画素。縮めの境に合わせてある。タイルの数つき）ごとに合成して上げる。上げたタイルの数と、段 0 の変わった範囲（x0 y0 x1 y1）。
    pub(super) fn upload(
        &mut self,
        doc: &Document,
        slot: Slot,
        rects: &[(DocRect, usize)],
    ) -> (usize, Option<[u32; 4]>) {
        let shift = self.set.as_ref().expect("作った").shift;
        let rings = self.rings(doc);
        let bounds = doc.bounds();
        let mut dirty: Option<[u32; 4]> = None;
        let mut uploaded = 0;
        for (rect, tiles) in rects {
            // 塗り広げるなら、矩形の周りを塗り広げの幅だけ広げた所まで上げる（変わったタイルの色が届く所）。その中の値を画像全体を
            // 塗り広げたときと同じにするため、合成はさらに幅だけ広げた所から
            let (rect, pad) = match rings.as_deref() {
                Some(r) => {
                    let grown = expand(*rect, r.reach(), bounds);
                    match (r.kinds_in(*rect), r.kinds_in(grown).1) {
                        // アイランドの中のテクセルが変わり、届く所に塗り広げるテクセルがある: 周りまで塗り広げ直す
                        ((true, _), true) => {
                            let inner = align(grown, shift, bounds);
                            (inner, Some((r, expand(inner, r.reach(), bounds))))
                        }
                        // アイランドの中が無い（変化はほかの塗り広げに届かない）が、矩形の中に塗り広げるテクセルがある: 矩形の中だけ
                        ((false, true), _) => (*rect, Some((r, grown))),
                        // 塗り広げるテクセルが届く所に無い: そのまま
                        _ => (*rect, None),
                    }
                }
                None => (*rect, None),
            };
            let Some((data, dw, dh)) = self.region_texels(doc, slot, rect, shift, pad) else {
                continue;
            };
            let (dx, dy) = (rect.x >> shift, rect.y >> shift);
            let set = self.set.as_ref().expect("作った");
            let texture = &set.textures[slot.index()].as_ref().expect("作った").texture;
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: dx, y: dy, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(dw * slot.bytes_per_texel()),
                    rows_per_image: Some(dh),
                },
                wgpu::Extent3d {
                    width: dw,
                    height: dh,
                    depth_or_array_layers: 1,
                },
            );
            dirty = union_dirty(dirty, Some([dx, dy, dx + dw, dy + dh]));
            uploaded += tiles;
        }
        (uploaded, dirty)
    }

    /// ドラッグの間の仮の絵: タイルを歩幅 `DRAG_STRIDE` で粗く合成し（効果の出力も粗く評価する。`Document::composite_coarse_tiles`）、
    /// 正確な絵と同じ矩形のまとめ（`plan_regions`）ごとに、粗い画素のままチャンネルの形式へ直してから、最も近い画素で絵の大きさへ
    /// 広げて 1 回で上げる。まとめた矩形に入ったタイルは全部粗く合成する。粗く上げたタイル・上げたタイルの数・段 0 の変わった範囲を返す。
    pub(super) fn upload_coarse(
        &mut self,
        doc: &Document,
        slot: Slot,
        coords: &[TileCoord],
    ) -> (Vec<TileCoord>, usize, Option<[u32; 4]>) {
        let shift = self.set.as_ref().expect("作った").shift;
        let bounds = doc.bounds();
        let rects = plan_regions(
            rects_of(doc, slot, coords, &[], shift),
            shift,
            bounds,
            false,
        );
        let ts = doc.tile_size();
        let mut covered: Vec<TileCoord> = Vec::new();
        for (r, _) in &rects {
            for ty in r.y / ts..(r.y + r.height).div_ceil(ts) {
                for tx in r.x / ts..(r.x + r.width).div_ceil(ts) {
                    covered.push(TileCoord { x: tx, y: ty });
                }
            }
        }
        covered.sort();
        covered.dedup();
        let Ok(tiles) = doc.composite_coarse_tiles(slot.channel(), &covered, DRAG_STRIDE) else {
            return (Vec::new(), 0, None);
        };
        let s = DRAG_STRIDE;
        let step = s.trailing_zeros();
        let more = shift.saturating_sub(step);
        let factor = 1u32 << step.saturating_sub(shift);
        let settings = doc.normal_settings();
        let bpt = slot.bytes_per_texel() as usize;
        let mut dirty = None;
        let mut uploaded = 0;
        for (rect, n) in &rects {
            // 矩形の粗い画素（行は下から。粗い座標の原点は文書の左下）
            let (bx, by) = (rect.x / s, rect.y / s);
            let (bw, bh) = (rect.width.div_ceil(s), rect.height.div_ceil(s));
            let mut band = vec![0u8; bw as usize * bh as usize * 4];
            for tile in &tiles {
                let (tw, th) = tile.size();
                let (tx, ty) = (tile.rect.x / s, tile.rect.y / s);
                let x0 = tx.max(bx);
                let x1 = (tx + tw).min(bx + bw);
                let y0 = ty.max(by);
                let y1 = (ty + th).min(by + bh);
                if x0 >= x1 || y0 >= y1 {
                    continue;
                }
                let len = (x1 - x0) as usize * 4;
                for y in y0..y1 {
                    let from = (((y - ty) * tw + (x0 - tx)) * 4) as usize;
                    let to = (((y - by) * bw + (x0 - bx)) * 4) as usize;
                    band[to..to + len].copy_from_slice(&tile.pixels[from..from + len]);
                }
            }
            let local = DocRect::new(0, 0, bw, bh);
            let converted = match slot {
                Slot::Color => Some(reduce_srgb_premultiplied(&band, local, more)),
                Slot::Emission => Some(reduce_emission(&band, local, more)),
                Slot::Normal => output_from_composites(&band, None, bw, bh, &settings)
                    .ok()
                    .map(|out| {
                        reduce(&out, local, more, 4, |p| {
                            [p[0] as u32, p[1] as u32, p[2] as u32, 255]
                        })
                    }),
                Slot::Metallic | Slot::Roughness | Slot::Height => {
                    Some(reduce(&band, local, more, 1, |p| {
                        [((p[0] as u32 * p[3] as u32 + 127) / 255), 0, 0, 0]
                    }))
                }
            };
            let Some((texels, sw, sh)) = converted else {
                continue;
            };
            let (dx, dy) = (rect.x >> shift, rect.y >> shift);
            let dw = rect.width.div_ceil(1 << shift);
            let dh = rect.height.div_ceil(1 << shift);
            let data = widen(&texels, sw, sh, bpt, factor, dw, dh);
            let set = self.set.as_ref().expect("作った");
            let texture = &set.textures[slot.index()].as_ref().expect("作った").texture;
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: dx, y: dy, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(dw * slot.bytes_per_texel()),
                    rows_per_image: Some(dh),
                },
                wgpu::Extent3d {
                    width: dw,
                    height: dh,
                    depth_or_array_layers: 1,
                },
            );
            dirty = union_dirty(dirty, Some([dx, dy, dx + dw, dy + dh]));
            uploaded += n;
        }
        (covered, uploaded, dirty)
    }

    /// 矩形のテクセル（チャンネルの形式で、縮めた後）。`pad` があれば、その外の矩形（`rect` を塗り広げの幅だけ広げたもの）から
    /// 合成して塗り広げ、`rect` の分を返す。
    pub(super) fn region_texels(
        &mut self,
        doc: &Document,
        slot: Slot,
        rect: DocRect,
        shift: u32,
        pad: Option<(&Rings, DocRect)>,
    ) -> Option<(Vec<u8>, u32, u32)> {
        // 合成する矩形（塗り広げるなら、その入力の範囲）
        let src = pad.map_or(rect, |(_, outer)| outer);
        let rings = pad.map(|(r, _)| r);
        let n = (src.width * src.height * 4) as usize;
        self.scratch.resize(n, 0);
        match slot {
            Slot::Normal => {
                // 作る設定なら、Height を 1 画素ずつ外へ広げて Sobel の隣を読めるようにする（キャンバスの端は Clamp と同じ端のまま）
                let settings = doc.normal_settings();
                let pad = u32::from(settings.derive_from_height());
                let outer = expand(src, pad, doc.bounds());
                let m = (outer.width * outer.height * 4) as usize;
                self.scratch.resize(m, 0);
                doc.composite_into(
                    Channel::Normal,
                    outer,
                    &mut self.scratch,
                    RowOrder::BottomUp,
                )
                .ok()?;
                let height = if settings.derive_from_height() {
                    self.scratch_height.resize(m, 0);
                    doc.composite_into(
                        Channel::Height,
                        outer,
                        &mut self.scratch_height,
                        RowOrder::BottomUp,
                    )
                    .ok()?;
                    Some(&self.scratch_height[..])
                } else {
                    None
                };
                let output = output_from_composites(
                    &self.scratch,
                    height,
                    outer.width,
                    outer.height,
                    &settings,
                )
                .ok()?;
                // Sobel のために外へ広げた分を切り取り、塗り広げる（書き出しも Normal の出力の画像を塗り広げる）
                let mut cut = cut_rect(&output, outer, src);
                let px = pad_region(&mut cut, src, rect, rings)?;
                Some(reduce(&px, rect, shift, 4, |p| {
                    [p[0] as u32, p[1] as u32, p[2] as u32, 255]
                }))
            }
            Slot::Color => {
                doc.composite_into(slot.channel(), src, &mut self.scratch, RowOrder::BottomUp)
                    .ok()?;
                let px = pad_region(&mut self.scratch, src, rect, rings)?;
                Some(reduce_srgb_premultiplied(&px, rect, shift))
            }
            Slot::Emission => {
                doc.composite_into(slot.channel(), src, &mut self.scratch, RowOrder::BottomUp)
                    .ok()?;
                let px = pad_region(&mut self.scratch, src, rect, rings)?;
                Some(reduce_emission(&px, rect, shift))
            }
            Slot::Metallic | Slot::Roughness | Slot::Height => {
                doc.composite_into(slot.channel(), src, &mut self.scratch, RowOrder::BottomUp)
                    .ok()?;
                let px = pad_region(&mut self.scratch, src, rect, rings)?;
                // 値 × アルファ（書き出しと同じ整数の式。塗っていない所は 0）
                Some(reduce(&px, rect, shift, 1, |p| {
                    [((p[0] as u32 * p[3] as u32 + 127) / 255), 0, 0, 0]
                }))
            }
        }
    }
}

/// 粗い合成で上げたタイル（`covered`）を、離したときに正確に上げ直すタイルとして覚える。粗い合成が失敗して何も上げなかったとき（`covered` が空）は、
/// 頼んだタイル（`requested`）を覚える（覚えないと、そのタイルは粗くも正確にも上がらないまま、離したあとも古い絵が残る）。
pub(super) fn held_after_coarse(
    covered: Vec<TileCoord>,
    requested: &[TileCoord],
) -> Vec<TileCoord> {
    if covered.is_empty() {
        requested.to_vec()
    } else {
        covered
    }
}

/// 矩形 `outer` の画素（RGBA8）から、その中の矩形 `inner` を切り出す。
pub(super) fn cut_rect(pixels: &[u8], outer: DocRect, inner: DocRect) -> Vec<u8> {
    let (cx, cy) = (inner.x - outer.x, inner.y - outer.y);
    let mut cut = Vec::with_capacity((inner.width * inner.height * 4) as usize);
    for row in 0..inner.height {
        let start = (((cy + row) * outer.width + cx) * 4) as usize;
        cut.extend_from_slice(&pixels[start..start + inner.width as usize * 4]);
    }
    cut
}

/// 矩形 `src` の合成（straight RGBA8）を、段の地図があれば塗り広げ、その中の矩形 `rect` の分を返す（`src` が `rect` なら切らない）。
pub(super) fn pad_region<'a>(
    pixels: &'a mut [u8],
    src: DocRect,
    rect: DocRect,
    rings: Option<&Rings>,
) -> Option<std::borrow::Cow<'a, [u8]>> {
    if let Some(r) = rings {
        r.dilate_region(pixels, src, rect).ok()?;
    }
    if src == rect {
        return Some(std::borrow::Cow::Borrowed(pixels));
    }
    Some(std::borrow::Cow::Owned(cut_rect(pixels, src, rect)))
}

// ───────── 矩形の決め方 ─────────

/// ドラッグの間、そのチャンネルを粗く合成して見せてよいか。Height から作る Normal は、Sobel が隣の画素を読むので粗くしない。
pub(super) fn coarse_allowed(doc: &Document, slot: Slot) -> bool {
    doc.tile_size().is_multiple_of(DRAG_STRIDE) && !(slot == Slot::Normal && doc.derives_normal())
}

/// 段 0 の変わった範囲（x0 y0 x1 y1）を合わせる。
pub(super) fn union_dirty(a: Option<[u32; 4]>, b: Option<[u32; 4]>) -> Option<[u32; 4]> {
    match (a, b) {
        (Some(d), Some(r)) => Some([
            d[0].min(r[0]),
            d[1].min(r[1]),
            d[2].max(r[2]),
            d[3].max(r[3]),
        ]),
        (a, b) => a.or(b),
    }
}

/// テクセルの並び（`sw` × `sh`、1 テクセル `bpt` バイト）を、1 つを `factor` × `factor` に広げて、`dw` × `dh` に切る（足りない所は端を延ばす）。
pub(super) fn widen(
    texels: &[u8],
    sw: u32,
    sh: u32,
    bpt: usize,
    factor: u32,
    dw: u32,
    dh: u32,
) -> Vec<u8> {
    let mut out = vec![0u8; dw as usize * dh as usize * bpt];
    if sw == 0 || sh == 0 {
        return out;
    }
    out.par_chunks_mut(dw as usize * bpt)
        .enumerate()
        .for_each(|(y, row)| {
            let sy = (y as u32 / factor).min(sh - 1) as usize;
            for (x, o) in row.chunks_exact_mut(bpt).enumerate() {
                let sx = (x as u32 / factor).min(sw - 1) as usize;
                let i = (sy * sw as usize + sx) * bpt;
                o.copy_from_slice(&texels[i..i + bpt]);
            }
        });
    out
}

/// straight の RGBA8（行は下から）を乗算済みにし、2^shift の箱で縮める（文書の端の欠けた箱は中の画素だけで平均する）。
pub fn reduce_premultiplied(straight: &[u8], region: DocRect, shift: u32) -> (Vec<u8>, u32, u32) {
    reduce(straight, region, shift, 4, |p| {
        let a = p[3] as u32;
        [
            (p[0] as u32 * a + 127) / 255,
            (p[1] as u32 * a + 127) / 255,
            (p[2] as u32 * a + 127) / 255,
            a,
        ]
    })
}

/// straight の sRGB の RGBA8（行は下から）を、リニアで乗算済みにして sRGB に符号化し直し（`Rgba8UnormSrgb` が読むとリニアの乗算済み）、
/// 2^shift の箱でリニアのまま平均して縮める。不透明な画素は元のバイトのまま（8 bit の sRGB → 16 bit のリニア → 8 bit の sRGB は元に戻る）。
pub fn reduce_srgb_premultiplied(
    straight: &[u8],
    region: DocRect,
    shift: u32,
) -> (Vec<u8>, u32, u32) {
    let (decode, encode) = (srgb_decode16(), srgb_encode16());
    reduce_with(
        straight,
        region,
        shift,
        4,
        |p| {
            let a = p[3] as u32;
            [
                (decode[p[0] as usize] as u32 * a + 127) / 255,
                (decode[p[1] as usize] as u32 * a + 127) / 255,
                (decode[p[2] as usize] as u32 * a + 127) / 255,
                a,
            ]
        },
        |q| {
            [
                encode[q[0] as usize],
                encode[q[1] as usize],
                encode[q[2] as usize],
                q[3] as u8,
            ]
        },
    )
}

/// Emission: 書き出しと同じバイト（ガンマの値 × α。整数の式）。縮めるときはリニアで平均する（sRGB の形式のミップと同じ）。
pub(super) fn reduce_emission(straight: &[u8], region: DocRect, shift: u32) -> (Vec<u8>, u32, u32) {
    let (decode, encode) = (srgb_decode16(), srgb_encode16());
    reduce_with(
        straight,
        region,
        shift,
        4,
        |p| {
            let a = p[3] as u32;
            let g = |v: u8| decode[((v as u32 * a + 127) / 255) as usize] as u32;
            [g(p[0]), g(p[1]), g(p[2]), a]
        },
        |q| {
            [
                encode[q[0] as usize],
                encode[q[1] as usize],
                encode[q[2] as usize],
                q[3] as u8,
            ]
        },
    )
}

/// sRGB の 8 bit → リニアの 16 bit（0〜65535）。
pub(super) fn srgb_decode16() -> &'static [u16; 256] {
    static TABLE: std::sync::OnceLock<[u16; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            (crate::view3d::brdf::srgb_to_linear(i as f32 / 255.0) * 65535.0).round() as u16
        })
    })
}

/// リニアの 16 bit → sRGB の 8 bit（四捨五入）。
pub(super) fn srgb_encode16() -> &'static [u8] {
    static TABLE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=u16::MAX)
            .map(|i| {
                (crate::view3d::brdf::linear_to_srgb(i as f32 / 65535.0) * 255.0)
                    .round()
                    .min(255.0) as u8
            })
            .collect()
    })
}

/// 矩形の画素ごとに `map`（4 バイト → 最大 4 つの値）を当てて、2^shift の箱で平均して縮める。`channels` は出力のバイト数（1 か 4）。
pub(super) fn reduce(
    src: &[u8],
    region: DocRect,
    shift: u32,
    channels: usize,
    map: impl Fn(&[u8]) -> [u32; 4] + Sync,
) -> (Vec<u8>, u32, u32) {
    reduce_with(src, region, shift, channels, map, |q| q.map(|v| v as u8))
}

/// `reduce` の、平均した値をバイトへ直す式（`finish`）を選べる形（`map` の値は 8 bit を超えてよい）。出力の行ごとに並べて計算する
/// （行の中の式は 1 つの道なので、並べ方によらず同じバイト）。
pub(super) fn reduce_with(
    src: &[u8],
    region: DocRect,
    shift: u32,
    channels: usize,
    map: impl Fn(&[u8]) -> [u32; 4] + Sync,
    finish: impl Fn([u32; 4]) -> [u8; 4] + Sync,
) -> (Vec<u8>, u32, u32) {
    let (w, h) = (region.width, region.height);
    let block = 1u32 << shift;
    let (dw, dh) = (w.div_ceil(block), h.div_ceil(block));
    let mut out = vec![0u8; (dw * dh) as usize * channels];
    if out.is_empty() {
        return (out, dw, dh);
    }
    out.par_chunks_mut(dw as usize * channels)
        .enumerate()
        .for_each(|(by, row)| {
            let by = by as u32;
            if shift == 0 {
                let line = &src[(by * w * 4) as usize..((by + 1) * w * 4) as usize];
                for (p, o) in line
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(row.chunks_exact_mut(channels))
                {
                    o.copy_from_slice(&finish(map(p))[..channels]);
                }
                return;
            }
            for (bx, o) in row.chunks_exact_mut(channels).enumerate() {
                let bx = bx as u32;
                let mut sum = [0u32; 4];
                let mut n = 0u32;
                for y in by * block..((by + 1) * block).min(h) {
                    for x in bx * block..((bx + 1) * block).min(w) {
                        let i = ((y * w + x) * 4) as usize;
                        let q = map(&src[i..i + 4]);
                        for k in 0..4 {
                            sum[k] += q[k];
                        }
                        n += 1;
                    }
                }
                o.copy_from_slice(&finish(sum.map(|s| (s + n / 2) / n))[..channels]);
            }
        });
    (out, dw, dh)
}
