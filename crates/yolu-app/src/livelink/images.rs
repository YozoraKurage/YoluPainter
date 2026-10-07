//! 頼みの絵のファイル（PNG・TGA・JPG・PSD）を読む。PSD は層を重ねた 1 枚にする（層のままは別の段）。読んだ絵は straight RGBA8・下の行が先
//! （文書の画素・`PixelClipboard::from_image` と同じ並び）。拡大縮小・リニアから sRGB への直しも置く。
//!
//! - 読むのは裏の仕事のスレッド（ファイルの読みと復号は重い）。取消の旗を区切りで見る。
//! - 大きさの上限: 辺 [`MAX_SIDE`]（元の絵の上限。Unity 版の画像の上限と同じ）、ファイル [`MAX_FILE_BYTES`]。超えるものは読まずに断る。
//! - 透明な画素の RGB も保つ（PNG・TGA は復号したまま。拡大縮小はアルファで重みを付け、透明な画素の RGB を混ぜない）。

use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rayon::prelude::*;

/// 絵の辺の上限。
pub const MAX_SIDE: u32 = 8192;
/// 絵のファイルの大きさの上限。
pub const MAX_FILE_BYTES: u64 = 512 << 20;

/// 読んだ絵（straight RGBA8、下の行が先）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// 読めなかった理由（人に見せる短い文の元）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PictureError {
    /// ファイルを開けない・読めない。
    Io(String),
    /// 知らない形式（拡張子）。
    Format,
    /// 復号できない（壊れている）。
    Decode(String),
    /// 大きすぎる（幅・高さ）。
    TooLarge(u32, u32),
    Cancelled,
}

impl PictureError {
    pub fn text(&self, lang: crate::lang::Lang) -> String {
        match self {
            PictureError::Io(e) => lang.pick(
                format!("ファイルを読めません（{e}）"),
                format!("Cannot read the file ({e})"),
            ),
            PictureError::Format => lang.pick("読めない形式です", "Unsupported format").into(),
            PictureError::Decode(e) => lang.pick(
                format!("絵として読めません（{e}）"),
                format!("Cannot decode the picture ({e})"),
            ),
            PictureError::TooLarge(w, h) => lang.pick(
                format!("大きすぎます（{w}×{h}）"),
                format!("Too large ({w}×{h})"),
            ),
            PictureError::Cancelled => lang.pick("取り消しました", "Cancelled").into(),
        }
    }
}

/// 拡張子で形式を決めて読む。
pub fn read_picture(path: &Path, cancel: &AtomicBool) -> Result<Picture, PictureError> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let format = match ext.as_str() {
        "png" => Some(image::ImageFormat::Png),
        "tga" => Some(image::ImageFormat::Tga),
        "jpg" | "jpeg" => Some(image::ImageFormat::Jpeg),
        "psd" => None,
        _ => return Err(PictureError::Format),
    };
    let meta = std::fs::metadata(path).map_err(|e| PictureError::Io(e.to_string()))?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err(PictureError::Io(if meta.is_file() {
            "too large".into()
        } else {
            "not a file".into()
        }));
    }
    let bytes = std::fs::read(path).map_err(|e| PictureError::Io(e.to_string()))?;
    if cancel.load(Ordering::Relaxed) {
        return Err(PictureError::Cancelled);
    }
    match format {
        Some(format) => decode(&bytes, format),
        None => {
            let limits = yolu_io::psd::Limits {
                max_dimension: MAX_SIDE,
                max_canvas_pixels: MAX_SIDE as u64 * MAX_SIDE as u64,
                ..yolu_io::psd::Limits::default()
            };
            let (width, height, mut rgba) =
                yolu_io::psd::read_flattened(&bytes, &limits, Some(cancel)).map_err(|e| {
                    if cancel.load(Ordering::Relaxed) {
                        PictureError::Cancelled
                    } else {
                        PictureError::Decode(e.to_string())
                    }
                })?;
            flip_rows(&mut rgba, width as usize);
            Ok(Picture {
                width,
                height,
                rgba,
            })
        }
    }
}

/// PNG・TGA・JPG のバイト列を読む（上の行から読んだ絵を、下の行が先へ並べ替える）。
pub fn decode(bytes: &[u8], format: image::ImageFormat) -> Result<Picture, PictureError> {
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_SIDE as u64 * MAX_SIDE as u64 * 8);
    reader.limits(limits);
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| PictureError::Decode(e.to_string()))?;
    if w > MAX_SIDE || h > MAX_SIDE || w == 0 || h == 0 {
        return Err(PictureError::TooLarge(w, h));
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_SIDE as u64 * MAX_SIDE as u64 * 8);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|e| PictureError::Decode(e.to_string()))?;
    let mut rgba = image.into_rgba8().into_raw();
    flip_rows(&mut rgba, w as usize);
    Ok(Picture {
        width: w,
        height: h,
        rgba,
    })
}

/// 行の並びを逆にする（上の行から ⇄ 下の行から）。
pub fn flip_rows(rgba: &mut [u8], width: usize) {
    let row = width * 4;
    if row == 0 {
        return;
    }
    let rows = rgba.len() / row;
    for y in 0..rows / 2 {
        let (top, bottom) = rgba.split_at_mut((rows - 1 - y) * row);
        top[y * row..(y + 1) * row].swap_with_slice(&mut bottom[..row]);
    }
}

/// 軸 1 本の、出力の位置ごとの元の画素（先頭の位置と重み）。縮めは箱の平均、広げは双線形、同じ大きさは 1 対 1。
struct Taps {
    start: usize,
    weights: Vec<f32>,
}

fn axis(source: usize, target: usize) -> Vec<Taps> {
    (0..target)
        .map(|i| {
            if source == target {
                Taps {
                    start: i,
                    weights: vec![1.0],
                }
            } else if source > target {
                let (lo, hi) = (
                    i as f64 * source as f64 / target as f64,
                    (i + 1) as f64 * source as f64 / target as f64,
                );
                let first = lo.floor() as usize;
                let last = ((hi.ceil() as usize).min(source)).max(first + 1) - 1;
                let weights = (first..=last)
                    .map(|j| {
                        let overlap = hi.min((j + 1) as f64) - lo.max(j as f64);
                        (overlap.max(0.0) / (hi - lo)) as f32
                    })
                    .collect();
                Taps {
                    start: first,
                    weights,
                }
            } else {
                let c = (i as f64 + 0.5) * source as f64 / target as f64 - 0.5;
                if c <= 0.0 {
                    Taps {
                        start: 0,
                        weights: vec![1.0],
                    }
                } else if c >= (source - 1) as f64 {
                    Taps {
                        start: source - 1,
                        weights: vec![1.0],
                    }
                } else {
                    let a = c.floor();
                    let f = (c - a) as f32;
                    Taps {
                        start: a as usize,
                        weights: vec![1.0 - f, f],
                    }
                }
            }
        })
        .collect()
}

/// straight RGBA8（下の行が先）を `to` の大きさへ拡大縮小する。アルファで重みを付けて混ぜ（透明な画素の RGB は、周りが全部透明のときだけ
/// 使う）、同じ画素しか重ならない所はその画素のまま。
pub fn resample(src: &[u8], from: [u32; 2], to: [u32; 2]) -> Vec<u8> {
    let (sw, sh) = (from[0] as usize, from[1] as usize);
    let (dw, dh) = (to[0] as usize, to[1] as usize);
    let xs = axis(sw, dw);
    let ys = axis(sh, dh);
    let mut out = vec![0u8; dw * dh * 4];
    out.par_chunks_mut(dw * 4).enumerate().for_each(|(y, row)| {
        let ty = &ys[y];
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let tx = &xs[x];
            // 1 画素しか重ならない（同じ大きさ・端）ならそのまま
            if tx.weights.len() == 1 && ty.weights.len() == 1 {
                let at = ((ty.start * sw) + tx.start) * 4;
                px.copy_from_slice(&src[at..at + 4]);
                continue;
            }
            let (mut a, mut r, mut g, mut b) = (0f32, 0f32, 0f32, 0f32);
            let (mut zw, mut zr, mut zg, mut zb) = (0f32, 0f32, 0f32, 0f32);
            let mut first: Option<[u8; 4]> = None;
            let mut same = true;
            for (j, wy) in ty.weights.iter().enumerate() {
                for (i, wx) in tx.weights.iter().enumerate() {
                    let w = wy * wx;
                    if w <= 0.0 {
                        continue;
                    }
                    let at = (((ty.start + j) * sw) + tx.start + i) * 4;
                    let p = [src[at], src[at + 1], src[at + 2], src[at + 3]];
                    match first {
                        Some(f) if f != p => same = false,
                        None => first = Some(p),
                        _ => {}
                    }
                    if p[3] == 0 {
                        zw += w;
                        zr += w * p[0] as f32;
                        zg += w * p[1] as f32;
                        zb += w * p[2] as f32;
                        continue;
                    }
                    let k = w * p[3] as f32;
                    a += k;
                    r += k * p[0] as f32;
                    g += k * p[1] as f32;
                    b += k * p[2] as f32;
                }
            }
            if same {
                px.copy_from_slice(&first.unwrap_or([0; 4]));
                continue;
            }
            let byte = |v: f32| v.round().clamp(0.0, 255.0) as u8;
            let alpha = byte(a);
            if alpha == 0 {
                if zw > 0.0 {
                    px.copy_from_slice(&[byte(zr / zw), byte(zg / zw), byte(zb / zw), 0]);
                }
                continue;
            }
            px.copy_from_slice(&[byte(r / a), byte(g / a), byte(b / a), alpha]);
        }
    });
    out
}

/// リニアの RGB（A はそのまま）を sRGB の画素へ直す。
pub fn linear_to_srgb(pixels: &mut [u8]) {
    let table: Vec<u8> = (0..=255u32)
        .map(|v| {
            let c = v as f64 / 255.0;
            let s = if c <= 0.003_130_8 {
                12.92 * c
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            };
            (s * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect();
    for p in pixels.as_chunks_mut::<4>().0 {
        p[0] = table[p[0] as usize];
        p[1] = table[p[1] as usize];
        p[2] = table[p[2] as usize];
    }
}

/// 長い辺を `max` 以下へ縮めた絵（収まれば元のまま）。
pub fn fit_within(picture: Picture, max: u32) -> Picture {
    let long = picture.width.max(picture.height);
    if long <= max {
        return picture;
    }
    let scale = max as f64 / long as f64;
    let w = ((picture.width as f64 * scale).round() as u32).max(1);
    let h = ((picture.height as f64 * scale).round() as u32).max(1);
    Picture {
        width: w,
        height: h,
        rgba: resample(&picture.rgba, [picture.width, picture.height], [w, h]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, p: [u8; 4]) -> Vec<u8> {
        p.repeat(w * h)
    }

    #[test]
    fn the_same_size_keeps_every_pixel_even_the_rgb_of_transparent_ones() {
        let src: Vec<u8> = (0..4 * 4 * 4).map(|i| (i * 7 % 256) as u8).collect();
        assert_eq!(resample(&src, [4, 4], [4, 4]), src);
    }

    #[test]
    fn shrinking_averages_boxes_and_a_flat_area_stays_exact() {
        let src = [
            [0, 0, 0, 255],
            [100, 0, 0, 255],
            [0, 100, 0, 255],
            [100, 100, 0, 255],
        ]
        .concat();
        assert_eq!(resample(&src, [2, 2], [1, 1]), [50, 50, 0, 255]);
        let flat = solid(8, 8, [13, 77, 201, 255]);
        assert_eq!(
            resample(&flat, [8, 8], [3, 5]),
            solid(3, 5, [13, 77, 201, 255])
        );
    }

    #[test]
    fn transparent_pixels_do_not_darken_their_opaque_neighbours() {
        let src = [[255, 0, 0, 255], [0, 0, 0, 0]].concat();
        assert_eq!(resample(&src, [2, 1], [1, 1]), [255, 0, 0, 128]);
        let src = [[10, 20, 30, 0], [30, 40, 50, 0]].concat();
        assert_eq!(resample(&src, [2, 1], [1, 1]), [20, 30, 40, 0]);
    }

    #[test]
    fn enlarging_is_bilinear_and_keeps_the_corners() {
        let src = [[0, 0, 0, 255], [200, 0, 0, 255]].concat();
        let out = resample(&src, [2, 1], [4, 1]);
        assert_eq!(out[..4], [0, 0, 0, 255]);
        assert_eq!(out[12..], [200, 0, 0, 255]);
        let reds: Vec<u8> = out.as_chunks::<4>().0.iter().map(|p| p[0]).collect();
        assert!(reds.windows(2).all(|w| w[0] <= w[1]), "{reds:?}");
    }

    #[test]
    fn linear_pixels_become_srgb_pixels_and_alpha_is_untouched() {
        let mut px = [0, 0, 0, 7, 255, 255, 255, 255, 55, 55, 55, 128];
        linear_to_srgb(&mut px);
        assert_eq!(px[..8], [0, 0, 0, 7, 255, 255, 255, 255]);
        assert_eq!(px[8..11], [128, 128, 128]);
        assert_eq!(px[11], 128);
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yolu-ll-img-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 2 × 2: 上の行 赤・透明（RGB 付き）、下の行 緑・青。
    fn sample_top_down() -> Vec<u8> {
        [
            [255, 0, 0, 255],
            [10, 20, 30, 0],
            [0, 255, 0, 255],
            [0, 0, 255, 128],
        ]
        .concat()
    }

    #[test]
    fn png_and_tga_read_bottom_row_first_and_keep_transparent_rgb() {
        let dir = temp("formats");
        let img = image::RgbaImage::from_raw(2, 2, sample_top_down()).unwrap();
        let cancel = AtomicBool::new(false);
        for ext in ["png", "tga"] {
            let path = dir.join(format!("a.{ext}"));
            img.save(&path).unwrap();
            let p = read_picture(&path, &cancel).unwrap();
            assert_eq!((p.width, p.height), (2, 2), "{ext}");
            // 下の行（緑・青）が先
            assert_eq!(
                p.rgba,
                [
                    [0, 255, 0, 255],
                    [0, 0, 255, 128],
                    [255, 0, 0, 255],
                    [10, 20, 30, 0]
                ]
                .concat(),
                "{ext}"
            );
        }
        let jpg = dir.join("a.jpg");
        image::DynamicImage::ImageRgba8(img.clone())
            .to_rgb8()
            .save(&jpg)
            .unwrap();
        let p = read_picture(&jpg, &cancel).unwrap();
        assert_eq!((p.width, p.height), (2, 2));
        assert!(p.rgba.chunks(4).all(|c| c[3] == 255), "JPG は不透明");
        // 知らない形式・無いファイル・壊れたファイル
        std::fs::write(dir.join("a.bmp"), b"BM").unwrap();
        assert_eq!(
            read_picture(&dir.join("a.bmp"), &cancel),
            Err(PictureError::Format)
        );
        assert!(matches!(
            read_picture(&dir.join("none.png"), &cancel),
            Err(PictureError::Io(_))
        ));
        std::fs::write(dir.join("bad.png"), b"not a png").unwrap();
        assert!(matches!(
            read_picture(&dir.join("bad.png"), &cancel),
            Err(PictureError::Decode(_))
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_psd_is_read_flattened() {
        let dir = temp("psd");
        let doc = yolu_io::psd::Document {
            width: 2,
            height: 2,
            layers: vec![yolu_io::psd::Layer {
                id: 1,
                width: 2,
                height: 2,
                pixels_rgba: sample_top_down(),
                ..yolu_io::psd::Layer::default()
            }],
            composite_rgba: None,
        };
        let written = yolu_io::psd::write(&doc, &yolu_io::psd::Limits::default()).unwrap();
        let path = dir.join("a.psd");
        std::fs::write(&path, &written).unwrap();
        let p = read_picture(&path, &AtomicBool::new(false)).unwrap();
        assert_eq!((p.width, p.height), (2, 2));
        assert_eq!(p.rgba[..8], [[0, 255, 0, 255], [0, 0, 255, 128]].concat());
        assert_eq!(p.rgba[8..12], [255, 0, 0, 255]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_picture_is_fitted_within_a_side() {
        let p = Picture {
            width: 8,
            height: 4,
            rgba: solid(8, 4, [1, 2, 3, 255]),
        };
        let f = fit_within(p.clone(), 4);
        assert_eq!((f.width, f.height), (4, 2));
        assert_eq!(fit_within(p.clone(), 8), p);
    }
}
