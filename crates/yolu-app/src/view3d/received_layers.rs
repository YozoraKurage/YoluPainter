//! Live Link で Unity から受けた、描いていないスロットの絵（影色のテクスチャ・マットキャップの絵など）を GPU に持つ: 2D テクスチャの
//! 配列で、層 1 つがスロット 1 つ（シェーダーの元の番号 40〜55。`shaders/liltoon.wgsl`）。
//!
//! - 持つのはスロットの並びで先の [`MAX_RECEIVED_LAYERS`] 枚まで（超えたスロットは割り当てのない既定で描き、欄のスロットの行が
//!   「描かない」と出す。`look_gpu::dropped_received`）。
//! - 層の大きさはそろえる（配列なので）: 絵の幅・高さの一番大きいものを、辺の上限（[`MAX_LAYER_SIZE`] と 3D ビューの決めた上限の小さいほう）
//!   と、全部の層（ミップ込み）がバイトの予算（[`BUDGET_BYTES`] と、3D ビューの全体の予算の計画が渡す残りの小さいほう）に収まるまで
//!   2 の累乗で縮めたもの。1 × 1 まで縮めても収まらなければ 1 × 1 で持つ（ユーザーチャンネルの配列と同じ決まり）。違う大きさの絵は CPU で
//!   合わせる（UV は 0〜1 なので縦横の比が違っても同じ所を読む）。ミップも CPU で作る（受けた時だけで、毎フレームはしない）。
//! - 値は Unity が読むのと同じ straight の RGBA8（乗算済みにしない。シェーダーも割り戻さない）。A は色の不透明度とは限らず、ノーマルマップの
//!   DXT5nm は A に X を持つので、A が 0 の所の RGB も落とさない。縮め・ミップは成分ごとの平均（Unity のミップの作りと同じく straight のまま）。
//!   sRGB の絵の RGB は、シェーダーがスロットの印でリニアへ直す。
//! - 絵の並びと中身（`Arc` の同一性）と大きさが前と同じなら作り直さない。

use std::sync::Arc;

use eframe::egui_wgpu::wgpu;
use yolu_core::look::ReceivedImage;

/// 1 つのセットが持てる受けた絵の層の数（シェーダーの元の番号 40〜55。`liltoon.wgsl` の `NR` と同じ）。
pub const MAX_RECEIVED_LAYERS: usize = 16;
/// 層の辺の上限。
pub const MAX_LAYER_SIZE: u32 = 1024;
/// 1 つのセットの受けた絵の全部（ミップ込み）の GPU のバイト数の上限（16 層の 1024² が入る。全体の予算の残りがもっと少なければそれ）。
pub const BUDGET_BYTES: u64 = 96 << 20;
/// 配列の層の数の下限（GL は 1 層の配列を読めない）。
const MIN_LAYERS: usize = 2;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct State {
    /// 層の並び（スロットの名前と絵）。
    layers: Vec<(String, Arc<ReceivedImage>)>,
    /// 層の大きさ。
    size: [u32; 2],
    view: wgpu::TextureView,
    bytes: u64,
}

/// 受けた絵の配列（セットごと）。
pub struct ReceivedLayers {
    device: wgpu::Device,
    queue: wgpu::Queue,
    blank: wgpu::TextureView,
    state: Option<State>,
    /// 作り直すたびに増える（束ねの鍵）。
    version: u64,
}

impl ReceivedLayers {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> ReceivedLayers {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-received-blank"),
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
            texture.as_image_copy(),
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
        let blank = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        ReceivedLayers {
            device: device.clone(),
            queue: queue.clone(),
            blank,
            state: None,
            version: 0,
        }
    }

    /// 同じ道具の、まっさらな別の配列（ほかのセット用）。
    pub fn sibling(&self) -> ReceivedLayers {
        ReceivedLayers {
            device: self.device.clone(),
            queue: self.queue.clone(),
            blank: self.blank.clone(),
            state: None,
            version: 0,
        }
    }

    /// 束ねる見え方（持っていなければ透明の 1 × 1 × 2）。
    pub fn view(&self) -> &wgpu::TextureView {
        self.state.as_ref().map_or(&self.blank, |s| &s.view)
    }

    /// 束ねの鍵。
    pub fn version(&self) -> u64 {
        self.version
    }

    /// GPU のバイト数（ミップ込み）。
    pub fn bytes(&self) -> u64 {
        self.state.as_ref().map_or(0, |s| s.bytes)
    }

    /// 層の大きさ（持っていなければ None）。
    pub fn size(&self) -> Option<[u32; 2]> {
        self.state.as_ref().map(|s| s.size)
    }

    /// スロットの層の番号（持っていなければ None）。
    pub fn layer_of(&self, slot: &str) -> Option<usize> {
        self.state
            .as_ref()?
            .layers
            .iter()
            .position(|(name, _)| name == slot)
    }

    /// 持つ絵を合わせる（[`MAX_RECEIVED_LAYERS`] まで。`limit` は辺の上限、`budget` はバイトの予算。[`plan`]）。並びと中身と大きさが前と同じなら
    /// 何もしない。作り直したら true。
    pub fn sync(&mut self, wanted: &[(String, Arc<ReceivedImage>)], limit: u32, budget: u64) -> bool {
        let wanted = &wanted[..wanted.len().min(MAX_RECEIVED_LAYERS)];
        let size = plan(wanted, limit, budget).0;
        let same = match &self.state {
            Some(s) => {
                s.size == size
                    && s.layers.len() == wanted.len()
                    && s.layers
                        .iter()
                        .zip(wanted)
                        .all(|(a, b)| a.0 == b.0 && Arc::ptr_eq(&a.1, &b.1))
            }
            None => wanted.is_empty(),
        };
        if same {
            return false;
        }
        self.version += 1;
        // 前の配列は新しい配列を作る前に手放す（両方が重ならないように）
        self.state = None;
        if wanted.is_empty() {
            return true;
        }
        let levels = 32 - size[0].max(size[1]).leading_zeros();
        let count = wanted.len().max(MIN_LAYERS);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-received"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: count as u32,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (layer, (_, image)) in wanted.iter().enumerate() {
            let mut level = resample(image, size);
            let mut dims = size;
            for mip in 0..levels {
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: mip,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer as u32,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &level,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(dims[0] * 4),
                        rows_per_image: Some(dims[1]),
                    },
                    wgpu::Extent3d {
                        width: dims[0],
                        height: dims[1],
                        depth_or_array_layers: 1,
                    },
                );
                if mip + 1 < levels {
                    (level, dims) = halve(&level, dims);
                }
            }
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        self.state = Some(State {
            layers: wanted.to_vec(),
            size,
            view,
            bytes: mip_bytes(size) * count as u64,
        });
        true
    }
}

/// ミップ込みの 1 層のバイト数。
fn mip_bytes(size: [u32; 2]) -> u64 {
    let (mut w, mut h, mut total) = (size[0] as u64, size[1] as u64, 0u64);
    loop {
        total += w * h * 4;
        if w == 1 && h == 1 {
            return total;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
}

/// 層の大きさとバイト数（ミップ込み）。絵（[`MAX_RECEIVED_LAYERS`] まで数える）の一番大きい幅・高さを、辺が `limit`（[`MAX_LAYER_SIZE`] まで）に、
/// 全部の層のバイト数が `budget`（[`BUDGET_BYTES`] まで）に収まるまで 2 の累乗で縮める。1 × 1 まで縮めても収まらなければ 1 × 1。
/// 絵が無ければ持たない（0 バイト）。
pub fn plan(images: &[(String, Arc<ReceivedImage>)], limit: u32, budget: u64) -> ([u32; 2], u64) {
    if images.is_empty() {
        return ([0, 0], 0);
    }
    let w = images.iter().map(|(_, i)| i.width).max().unwrap_or(1).max(1);
    let h = images.iter().map(|(_, i)| i.height).max().unwrap_or(1).max(1);
    let count = images.len().clamp(MIN_LAYERS, MAX_RECEIVED_LAYERS) as u64;
    let limit = limit.clamp(1, MAX_LAYER_SIZE);
    let budget = budget.min(BUDGET_BYTES);
    let mut size = [w, h];
    while size[0] > limit || size[1] > limit || mip_bytes(size) * count > budget {
        if size == [1, 1] {
            break;
        }
        size = [(size[0] / 2).max(1), (size[1] / 2).max(1)];
    }
    (size, mip_bytes(size) * count)
}

/// 絵を `size` に合わせる（straight のまま成分ごとに。同じ大きさはそのまま、縮めは箱の平均、広げは双線形）。行の向きは変えない（下から）。
pub fn resample(image: &ReceivedImage, size: [u32; 2]) -> Vec<u8> {
    let (sw, sh) = (image.width as usize, image.height as usize);
    let (dw, dh) = (size[0] as usize, size[1] as usize);
    let src: Vec<[f32; 4]> = image
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| p.map(f32::from))
        .collect();
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        for x in 0..dw {
            // 出力の画素が覆う元の範囲（縮めは箱で平均、広げは中心を双線形で読む）
            let (x0, x1) = (
                x as f32 * sw as f32 / dw as f32,
                (x + 1) as f32 * sw as f32 / dw as f32,
            );
            let (y0, y1) = (
                y as f32 * sh as f32 / dh as f32,
                (y + 1) as f32 * sh as f32 / dh as f32,
            );
            let mut acc = [0f32; 4];
            if x1 - x0 >= 1.0 && y1 - y0 >= 1.0 {
                let (xa, xb) = (x0.floor() as usize, (x1.ceil() as usize).min(sw));
                let (ya, yb) = (y0.floor() as usize, (y1.ceil() as usize).min(sh));
                let mut n = 0f32;
                for sy in ya..yb {
                    for sx in xa..xb {
                        let p = src[sy * sw + sx];
                        for k in 0..4 {
                            acc[k] += p[k];
                        }
                        n += 1.0;
                    }
                }
                acc.iter_mut().for_each(|v| *v /= n.max(1.0));
            } else {
                let fx = ((x0 + x1) * 0.5 - 0.5).clamp(0.0, sw as f32 - 1.0);
                let fy = ((y0 + y1) * 0.5 - 0.5).clamp(0.0, sh as f32 - 1.0);
                let (ix, iy) = (fx.floor() as usize, fy.floor() as usize);
                let (jx, jy) = ((ix + 1).min(sw - 1), (iy + 1).min(sh - 1));
                let (tx, ty) = (fx - ix as f32, fy - iy as f32);
                let at = |xx: usize, yy: usize| src[yy * sw + xx];
                let (a, b, c, d) = (at(ix, iy), at(jx, iy), at(ix, jy), at(jx, jy));
                for k in 0..4 {
                    let top = a[k] + (b[k] - a[k]) * tx;
                    let bottom = c[k] + (d[k] - c[k]) * tx;
                    acc[k] = top + (bottom - top) * ty;
                }
            }
            let o = (y * dw + x) * 4;
            for k in 0..4 {
                out[o + k] = acc[k].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// 1 段小さいミップ（2 × 2 の成分ごとの平均。straight のまま）。
fn halve(src: &[u8], size: [u32; 2]) -> (Vec<u8>, [u32; 2]) {
    let (w, h) = (size[0] as usize, size[1] as usize);
    let (dw, dh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        for x in 0..dw {
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for sy in (y * 2)..((y * 2 + 2).min(h)) {
                for sx in (x * 2)..((x * 2 + 2).min(w)) {
                    let i = (sy * w + sx) * 4;
                    for k in 0..4 {
                        acc[k] += src[i + k] as u32;
                    }
                    n += 1;
                }
            }
            let o = (y * dw + x) * 4;
            for k in 0..4 {
                out[o + k] = ((acc[k] + n / 2) / n.max(1)) as u8;
            }
        }
    }
    (out, [dw as u32, dh as u32])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Arc<ReceivedImage> {
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                pixels.extend_from_slice(&f(x, y));
            }
        }
        Arc::new(ReceivedImage {
            width: w,
            height: h,
            srgb: true,
            pixels: pixels.into(),
        })
    }

    #[test]
    fn the_layer_size_is_the_largest_image_within_the_limits_and_the_budget() {
        let small = ("a".to_owned(), image(256, 128, |_, _| [0; 4]));
        let big = ("b".to_owned(), image(2048, 2048, |_, _| [0; 4]));
        let full = BUDGET_BYTES;
        assert_eq!(plan(&[], MAX_LAYER_SIZE, full), ([0, 0], 0), "絵が無ければ持たない");
        // 1 枚でも 2 層（GL）で数える
        assert_eq!(
            plan(std::slice::from_ref(&small), MAX_LAYER_SIZE, full),
            ([256, 128], mip_bytes([256, 128]) * 2)
        );
        assert_eq!(plan(&[small.clone(), big.clone()], MAX_LAYER_SIZE, full).0, [1024, 1024]);
        // 16 層の 1024² はミップ込みで約 85 MiB（1 つのセットの上限の中）
        let many: Vec<_> = (0..MAX_RECEIVED_LAYERS)
            .map(|i| (format!("s{i}"), big.1.clone()))
            .collect();
        let (s, bytes) = plan(&many, MAX_LAYER_SIZE, full);
        assert_eq!(s, [1024, 1024]);
        assert_eq!(bytes, mip_bytes(s) * MAX_RECEIVED_LAYERS as u64);
        assert!(bytes <= BUDGET_BYTES);
        // 辺の上限（ほかのセットの上限）と、全体の予算の残りで縮める
        assert_eq!(plan(&many, 256, full).0, [256, 256]);
        let (s, bytes) = plan(&many, MAX_LAYER_SIZE, mip_bytes([512, 512]) * 16 - 1);
        assert_eq!(s, [256, 256]);
        assert!(bytes < mip_bytes([512, 512]) * 16);
        // 1 × 1 まで縮めても収まらなければ 1 × 1（ユーザーチャンネルの配列と同じ決まり）
        assert_eq!(plan(&many, MAX_LAYER_SIZE, 0), ([1, 1], 4 * 16));
    }

    #[test]
    fn resampling_keeps_straight_values_averages_down_and_keeps_the_same_size() {
        // 同じ大きさ: そのまま（A が 0 の所の RGB も落とさない。DXT5nm のノーマルマップは A に X を持つ）
        let half = image(2, 1, |x, _| {
            if x == 0 {
                [200, 100, 50, 255]
            } else {
                [255, 100, 50, 0]
            }
        });
        assert_eq!(resample(&half, [2, 1]), vec![200, 100, 50, 255, 255, 100, 50, 0]);
        // 縮め: 成分ごとの箱の平均（A の小さい画素の RGB も同じ重み）
        let checker = image(4, 4, |x, y| {
            if (x + y) % 2 == 0 {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 0]
            }
        });
        let down = resample(&checker, [2, 2]);
        assert!(down
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0].abs_diff(128) <= 1 && p[3].abs_diff(128) <= 1));
        let nm = image(2, 2, |_, _| [255, 140, 0, 3]);
        assert_eq!(resample(&nm, [1, 1]), vec![255, 140, 0, 3]);
        // 広げ: 端の値を保つ（1 × 1 は全部同じ色）
        let one = image(1, 1, |_, _| [10, 20, 30, 0]);
        assert!(resample(&one, [4, 4])
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [10, 20, 30, 0]));
        // ミップも straight のまま成分ごと
        let (m, s) = halve(&[255, 140, 0, 0, 255, 140, 0, 10, 255, 140, 0, 0, 255, 140, 0, 10], [2, 2]);
        assert_eq!(s, [1, 1]);
        assert_eq!(m, vec![255, 140, 0, 5]);
    }
}
