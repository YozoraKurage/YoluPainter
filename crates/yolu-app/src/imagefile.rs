//! 外の画像ファイル（PNG・JPEG・TGA・WebP・TIFF・BMP・EXR・HDR・GIF）を straight RGBA8 で読む。
//!
//! 取り込みのウィンドウ・棚への画像の取り込み・ライブラリへの追加が同じ読み方を使う。
//! - 読めるのは `FORMATS` の表の形式だけ。拡張子で決める（中身の見分けはしない）。GIF は最初のフレームだけ。
//! - 1 辺は 1〜8192、確保は 1 GiB まで（読む前にピークを数えて断る）。寸法はヘッダーで先に見て、画素を読む前に断る。
//! - 整数の画素（8 ビット・16 ビット）は `image` の変換のまま 8 ビットへ（16 ビットは丸める）。ICC・ガンマは使わない。
//! - 浮動小数の画素（EXR・HDR・浮動小数の TIFF）は線形の値として読み、色（R・G・B）は 0〜1 で切ってから sRGB へ直して 8 ビットへ、
//!   アルファは線形のまま 0〜1 で切って 8 ビットへ。1 を超える値は 1 に潰れる（トーンマップはしない）。データのチャンネル
//!   （ノーマル・ラフネスなど）の線形のままの取り込みはここではしない（どの画像もカラーとして入る）。

use std::io::{BufRead, Seek};
use std::path::Path;

use image::ImageDecoder;

/// 取り込める画像の 1 辺の上限（core の画像の入力と同じ）。
pub const MAX_SIDE: u32 = 8192;
/// 読むときに確保してよい上限（バイト）。
const MAX_ALLOC: u64 = 1 << 30;

/// 読める形式（拡張子と、画面に出す形式の名前）。拡張子は小文字で、同じ形式は続けて並べる。
pub const FORMATS: &[(&str, image::ImageFormat, &str)] = &[
    ("png", image::ImageFormat::Png, "PNG"),
    ("jpg", image::ImageFormat::Jpeg, "JPEG"),
    ("jpeg", image::ImageFormat::Jpeg, "JPEG"),
    ("tga", image::ImageFormat::Tga, "TGA"),
    ("webp", image::ImageFormat::WebP, "WebP"),
    ("tif", image::ImageFormat::Tiff, "TIFF"),
    ("tiff", image::ImageFormat::Tiff, "TIFF"),
    ("bmp", image::ImageFormat::Bmp, "BMP"),
    ("exr", image::ImageFormat::OpenExr, "EXR"),
    ("hdr", image::ImageFormat::Hdr, "HDR"),
    ("gif", image::ImageFormat::Gif, "GIF"),
];

/// 画像だが読めない形式の拡張子（取り込みのウィンドウに名前だけ出して「未対応の形式」と断る）。
pub const UNSUPPORTED_EXTENSIONS: &[&str] = &["psb", "dds", "avif", "heic", "jxl", "qoi"];

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// 拡張子から、読める画像の形式（読めない拡張子は None）。
pub fn format_of(path: &Path) -> Option<image::ImageFormat> {
    let ext = extension(path);
    FORMATS.iter().find(|f| f.0 == ext).map(|f| f.1)
}

/// 形式の名前（画面に出す）。
pub fn format_label(format: image::ImageFormat) -> &'static str {
    FORMATS.iter().find(|f| f.1 == format).map_or("", |f| f.2)
}

/// ファイルを選ぶウィンドウの絞り込みに渡す、読める拡張子（小文字と大文字。OS によっては大文字小文字を区別するので両方）。
pub fn extensions() -> Vec<String> {
    FORMATS
        .iter()
        .flat_map(|f| [f.0.to_owned(), f.0.to_ascii_uppercase()])
        .collect()
}

/// 画像のファイルとして扱う拡張子か（読める形式と、読めない画像の形式）。
pub fn is_image_extension(path: &Path) -> bool {
    let ext = extension(path);
    FORMATS.iter().any(|f| f.0 == ext) || UNSUPPORTED_EXTENSIONS.contains(&ext.as_str())
}

/// 画像だが読めない形式の拡張子か（`UNSUPPORTED_EXTENSIONS`）。
pub fn is_unsupported_image(path: &Path) -> bool {
    UNSUPPORTED_EXTENSIONS.contains(&extension(path).as_str())
}

/// 読めない理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// その形式として読めない（壊れている・形式が違う）。
    Unreadable,
    /// 1 辺が 1〜8192 の外。
    Sides,
    /// 確保の上限（1 GiB）を超える。
    TooLarge,
}

/// ファイルを読めなかった理由（言葉は `Lang::image_read_error`）。
#[derive(Debug)]
pub enum ReadError {
    /// ファイルを開けない。
    Io(std::io::Error),
    /// 画像として読めない・大きすぎる。
    Image(Problem),
}

/// ファイルを straight RGBA8（上の行が先）で読む。
pub fn read_file(
    path: &Path,
    format: image::ImageFormat,
) -> Result<(Vec<u8>, u32, u32), ReadError> {
    let file = std::fs::File::open(path).map_err(ReadError::Io)?;
    decode(std::io::BufReader::new(file), format).map_err(ReadError::Image)
}

/// 読み出し口から straight RGBA8（上の行が先）で読む。寸法はヘッダーで先に見て、画素を読む前に断る。
///
/// 確保の上限（`MAX_ALLOC`）は、`image` が守らない読み方（`DynamicImage::from_decoder` と EXR）を下で数え直して守る。
pub fn decode<R: BufRead + Seek>(
    source: R,
    format: image::ImageFormat,
) -> Result<(Vec<u8>, u32, u32), Problem> {
    if format == image::ImageFormat::OpenExr {
        return decode_exr(source);
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    let mut reader = image::ImageReader::with_format(source, format);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().map_err(|e| match e {
        image::ImageError::Limits(_) => Problem::Sides,
        _ => Problem::Unreadable,
    })?;
    let (w, h) = decoder.dimensions();
    if !(1..=MAX_SIDE).contains(&w) || !(1..=MAX_SIDE).contains(&h) {
        return Err(Problem::Sides);
    }
    decoder
        .set_limits(limits)
        .map_err(|_| Problem::Unreadable)?;
    // `from_decoder` は `max_alloc` を見ない。読む前に確保のピークを数えて断る
    if !within_alloc(format, decoder.total_bytes(), w, h) {
        return Err(Problem::TooLarge);
    }
    if matches!(
        decoder.color_type(),
        image::ColorType::Rgb32F | image::ColorType::Rgba32F
    ) {
        return decode_float(decoder, w, h);
    }
    let img = image::DynamicImage::from_decoder(decoder).map_err(|e| match e {
        image::ImageError::Limits(_) => Problem::TooLarge,
        _ => Problem::Unreadable,
    })?;
    Ok((img.into_rgba8().into_raw(), w, h))
}

/// RGBA8 の出力のバイト数。
fn rgba8_bytes(w: u32, h: u32) -> u64 {
    u64::from(w) * u64::from(h) * 4
}

/// 読むときの確保のピークが `MAX_ALLOC` 以下か。`raw` はデコーダーが出すバッファ（`total_bytes`）。ピークは、デコーダーのバッファと
/// RGBA8 の出力の合計。TIFF のデコーダーは画素の Vec を作ってから呼び出し側のバッファへ写すので、読む間はバッファの 2 倍になる。
fn within_alloc(format: image::ImageFormat, raw: u64, w: u32, h: u32) -> bool {
    let reading = if format == image::ImageFormat::Tiff {
        raw.saturating_mul(2)
    } else {
        raw
    };
    reading.max(raw.saturating_add(rgba8_bytes(w, h))) <= MAX_ALLOC
}

/// 浮動小数の画素（HDR・浮動小数の TIFF。Rgb32F か Rgba32F）を、f32 の写しを作らずに 1 画素ずつ RGBA8 へ写して読む。
/// 確保のピークは数えてある（`decode`）。
fn decode_float(
    decoder: impl ImageDecoder,
    w: u32,
    h: u32,
) -> Result<(Vec<u8>, u32, u32), Problem> {
    let channels = usize::from(decoder.color_type().channel_count());
    let mut raw = vec![0u8; usize::try_from(decoder.total_bytes()).map_err(|_| Problem::TooLarge)?];
    decoder.read_image(&mut raw).map_err(|e| match e {
        image::ImageError::Limits(_) => Problem::TooLarge,
        _ => Problem::Unreadable,
    })?;
    let mut out = Vec::with_capacity(rgba8_bytes(w, h) as usize);
    for px in raw.chunks_exact(channels * 4) {
        let sample =
            |i: usize| f32::from_ne_bytes([px[i * 4], px[i * 4 + 1], px[i * 4 + 2], px[i * 4 + 3]]);
        let alpha = if channels == 4 {
            alpha8(sample(3))
        } else {
            255
        };
        out.extend_from_slice(&[srgb8(sample(0)), srgb8(sample(1)), srgb8(sample(2)), alpha]);
    }
    Ok((out, w, h))
}

/// EXR を `exr` クレートで直に読み、f32 の全画面のバッファを作らずに、画素を置く関数の中で RGBA8（色は sRGB・アルファは線形のまま）にする。
/// 寸法（表示ウィンドウ）はヘッダーで先に見て、1 辺の上限の外なら画素を読まずに断る。確保は RGBA8 の出力とチャンクの分だけ。
/// レイヤーは最初の、深さの無い RGB のレイヤー。データウィンドウが表示ウィンドウからはみ出す画素は捨て、データウィンドウの外は
/// （アルファの無いファイルは不透明の黒、あるファイルは透明）で埋める。
fn decode_exr<R: BufRead + Seek>(source: R) -> Result<(Vec<u8>, u32, u32), Problem> {
    use exr::prelude::*;
    let chunks = exr::block::read(source, false).map_err(|_| Problem::Unreadable)?;
    let header = chunks
        .headers()
        .iter()
        .find(|header| {
            !header.deep
                && ["R", "G", "B"].iter().all(|c| {
                    header
                        .channels
                        .find_index_of_channel(&Text::from(*c))
                        .is_some()
                })
        })
        .ok_or(Problem::Unreadable)?;
    let has_alpha = header
        .channels
        .find_index_of_channel(&Text::from("A"))
        .is_some();
    let display = header.shared_attributes.display_window;
    let (w, h) = (display.size.width(), display.size.height());
    let side = MAX_SIDE as usize;
    if !(1..=side).contains(&w) || !(1..=side).contains(&h) {
        return Err(Problem::Sides);
    }
    let offset = header.own_attributes.layer_position - display.position;
    let background = [0, 0, 0, if has_alpha { 0 } else { 255 }];
    let result = read()
        .no_deep_data()
        .largest_resolution_level()
        .rgba_channels(
            move |_size, _channels| background.repeat(w * h),
            move |out: &mut Vec<u8>, at, (r, g, b, a): (f32, f32, f32, f32)| {
                let at = at.to_i32() + offset;
                if (0..w as i32).contains(&at.x()) && (0..h as i32).contains(&at.y()) {
                    let i = (at.y() as usize * w + at.x() as usize) * 4;
                    out[i..i + 4].copy_from_slice(&[srgb8(r), srgb8(g), srgb8(b), alpha8(a)]);
                }
            },
        )
        .first_valid_layer()
        .all_attributes()
        .from_chunks(chunks)
        .map_err(|_| Problem::Unreadable)?;
    Ok((result.layer_data.channel_data.pixels, w as u32, h as u32))
}

/// 線形の値（0〜1 で切る。NaN は 0）を sRGB の 8 ビットへ。丸めの境目は `SRGB8_BOUNDS` の表で決めるので、OS の数学のライブラリによらず同じ値になる。
fn srgb8(linear: f32) -> u8 {
    let v = if linear.is_nan() {
        0.0
    } else {
        linear.clamp(0.0, 1.0)
    };
    SRGB8_BOUNDS.partition_point(|&bound| bound <= v) as u8
}

/// 8 ビットの sRGB の値 k（0〜254）と k + 1 の境目（符号化した値が (k + 0.5) / 255）の線形の値。線形の値がこれ以上なら k + 1 以上になる。
/// `f64` で「e が 0.04045 以下なら e / 12.92、そうでなければ ((e + 0.055) / 1.055)^2.4」を計算して f32 に丸めた物で、
/// テスト `srgb8_bounds_match_the_formula` が作り方と照らす。
#[rustfmt::skip]
const SRGB8_BOUNDS: [f32; 255] = [
    0.0001517635, 0.0004552905, 0.0007588175, 0.0010623444, 0.0013658714,
    0.0016693984, 0.0019729254, 0.0022764525, 0.0025799794, 0.0028835062,
    0.0031883009, 0.0035092593, 0.003848315, 0.004205748, 0.004581833,
    0.0049768374, 0.005391024, 0.0058246506, 0.0062779696, 0.0067512277,
    0.0072446684, 0.0077585303, 0.0082930485, 0.008848453, 0.0094249705,
    0.010022826, 0.010642237, 0.011283421, 0.0119465925, 0.01263196,
    0.013339732, 0.014070112, 0.014823303, 0.015599503, 0.01639891,
    0.017221715, 0.018068114, 0.018938294, 0.019832443, 0.020750744,
    0.021693382, 0.022660539, 0.02365239, 0.024669115, 0.025710888,
    0.026777882, 0.02787027, 0.02898822, 0.030131903, 0.03130148,
    0.032497123, 0.03371899, 0.034967244, 0.036242045, 0.037543554,
    0.038871925, 0.04022732, 0.041609887, 0.043019786, 0.044457164,
    0.04592217, 0.047414962, 0.048935685, 0.050484486, 0.052061506,
    0.053666897, 0.055300802, 0.05696336, 0.058654718, 0.060375012,
    0.062124383, 0.063902974, 0.06571092, 0.06754835, 0.06941541,
    0.071312234, 0.073238954, 0.07519571, 0.07718261, 0.07919982,
    0.08124744, 0.083325624, 0.08543449, 0.087574154, 0.08974477,
    0.09194644, 0.0941793, 0.096443474, 0.098739095, 0.10106627,
    0.10342513, 0.105815805, 0.1082384, 0.110693045, 0.11317986,
    0.11569897, 0.11825048, 0.12083452, 0.1234512, 0.12610064,
    0.12878296, 0.13149826, 0.13424668, 0.1370283, 0.13984327,
    0.14269169, 0.14557366, 0.14848931, 0.15143873, 0.15442206,
    0.15743938, 0.16049083, 0.1635765, 0.16669649, 0.16985093,
    0.17303991, 0.17626357, 0.17952198, 0.18281525, 0.1861435,
    0.18950683, 0.19290535, 0.19633915, 0.19980834, 0.20331304,
    0.20685335, 0.21042934, 0.21404114, 0.21768884, 0.22137256,
    0.2250924, 0.22884843, 0.23264076, 0.2364695, 0.24033478,
    0.24423663, 0.2481752, 0.25215057, 0.25616285, 0.26021212,
    0.26429847, 0.26842204, 0.2725829, 0.2767811, 0.2810168,
    0.2852901, 0.28960103, 0.29394972, 0.2983363, 0.3027608,
    0.30722335, 0.31172404, 0.31626296, 0.32084018, 0.32545584,
    0.33010998, 0.33480275, 0.33953416, 0.34430438, 0.34911346,
    0.3539615, 0.35884857, 0.36377478, 0.36874023, 0.37374496,
    0.37878913, 0.38387278, 0.388996, 0.3941589, 0.39936152,
    0.40460402, 0.40988642, 0.41520882, 0.42057136, 0.42597404,
    0.43141702, 0.43690035, 0.44242412, 0.44798842, 0.4535933,
    0.45923892, 0.4649253, 0.47065252, 0.4764207, 0.48222992,
    0.48808023, 0.49397177, 0.49990454, 0.5058787, 0.5118943,
    0.5179514, 0.5240501, 0.5301905, 0.5363727, 0.54259676,
    0.5488627, 0.55517066, 0.5615207, 0.5679129, 0.5743473,
    0.58082414, 0.58734334, 0.593905, 0.6005092, 0.6071561,
    0.6138457, 0.6205781, 0.62735337, 0.6341716, 0.6410329,
    0.64793724, 0.6548848, 0.66187567, 0.6689098, 0.67598736,
    0.68310845, 0.6902731, 0.69748133, 0.7047334, 0.71202916,
    0.7193688, 0.72675246, 0.73418003, 0.7416518, 0.7491677,
    0.7567278, 0.7643323, 0.7719811, 0.7796744, 0.7874123,
    0.79519475, 0.8030219, 0.81089383, 0.8188105, 0.8267722,
    0.8347788, 0.8428305, 0.8509273, 0.8590692, 0.8672565,
    0.87548906, 0.88376707, 0.89209056, 0.9004596, 0.9088742,
    0.91733456, 0.9258406, 0.9343926, 0.94299036, 0.95163417,
    0.96032405, 0.96906, 0.97784215, 0.98667055, 0.99554527,
];

/// 線形のアルファ（0〜1 で切る。NaN は 0）を 8 ビットへ（sRGB の曲線は掛けない）。
fn alpha8(linear: f32) -> u8 {
    let v = if linear.is_nan() {
        0.0
    } else {
        linear.clamp(0.0, 1.0)
    };
    (v * 255.0).round() as u8
}

/// 線形の RGBA の浮動小数（4 値ずつ）を、色は sRGB に直し、アルファはそのまま 8 ビットにした RGBA8 へ。
pub fn linear_float_to_srgb8(rgba: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.as_chunks::<4>().0 {
        out.extend_from_slice(&[srgb8(px[0]), srgb8(px[1]), srgb8(px[2]), alpha8(px[3])]);
    }
    out
}

/// straight RGBA8 を PNG のバイト列にする（ライブラリには PNG で置く）。
pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, image::ImageError> {
    let mut out = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut out),
        rgba,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_values_are_clipped_to_0_1_and_encoded_to_srgb() {
        let out = linear_float_to_srgb8(&[
            0.5,
            1.0,
            0.0,
            0.5, // 0.5 の線形は sRGB で 188、アルファは線形のまま 128
            2.5,
            -0.3,
            f32::NAN,
            7.0, // 1 を超える→255、負→0、NaN→0、アルファの 1 超→255
        ]);
        assert_eq!(out, [188, 255, 0, 128, 255, 0, 0, 255]);
    }

    /// 画像をメモリに書き出す（形式は `format`）。
    fn write(image: &image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
        let mut out = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut out), format)
            .unwrap();
        out
    }

    fn read(bytes: &[u8], format: image::ImageFormat) -> Result<(Vec<u8>, u32, u32), Problem> {
        decode(std::io::Cursor::new(bytes), format)
    }

    /// 2 × 1 の、左が `a`・右が `b` の RGBA8。
    fn pair(a: [u8; 4], b: [u8; 4]) -> image::DynamicImage {
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(2, 1, [a, b].concat()).unwrap())
    }

    #[test]
    fn lossless_formats_read_back_the_same_straight_rgba8() {
        let img = pair([200, 100, 50, 255], [10, 20, 30, 128]);
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::WebP,
            image::ImageFormat::Tiff,
            image::ImageFormat::Bmp,
            image::ImageFormat::Tga,
        ] {
            let (rgba, w, h) = read(&write(&img, format), format).unwrap_or_else(|e| {
                panic!("{}: {e:?}", format_label(format));
            });
            assert_eq!((w, h), (2, 1), "{}", format_label(format));
            assert_eq!(
                rgba,
                [200, 100, 50, 255, 10, 20, 30, 128],
                "{}",
                format_label(format)
            );
        }
    }

    #[test]
    fn jpeg_and_gif_read_a_flat_color_closely() {
        let flat = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([120, 60, 200, 255]),
        ));
        for format in [image::ImageFormat::Jpeg, image::ImageFormat::Gif] {
            let bytes = write(&flat, format);
            let (rgba, w, h) = read(&bytes, format).unwrap();
            assert_eq!((w, h), (8, 8), "{}", format_label(format));
            for (got, want) in rgba[..4].iter().zip([120u8, 60, 200, 255]) {
                assert!(
                    got.abs_diff(want) <= 6,
                    "{}: {:?}",
                    format_label(format),
                    &rgba[..4]
                );
            }
        }
    }

    #[test]
    fn a_16_bit_png_and_tiff_are_rounded_to_8_bits_like_a_png() {
        let img = image::DynamicImage::ImageRgba16(
            image::ImageBuffer::from_raw(1, 1, vec![0x1234u16, 0x8000, 0xffff, 0x4321]).unwrap(),
        );
        for format in [image::ImageFormat::Png, image::ImageFormat::Tiff] {
            let (rgba, _, _) = read(&write(&img, format), format).unwrap();
            assert_eq!(rgba, img.to_rgba8().into_raw(), "{}", format_label(format));
        }
    }

    #[test]
    fn exr_and_hdr_are_clipped_to_0_1_and_read_as_srgb_color() {
        // 線形の 0.5 は sRGB で 188。1 を超える値は 255、負の値は 0。アルファは線形のまま
        let exr = image::DynamicImage::ImageRgba32F(
            image::ImageBuffer::from_raw(2, 1, vec![0.5f32, 1.0, 0.0, 0.5, 4.0, -2.0, 0.25, 3.0])
                .unwrap(),
        );
        let (rgba, w, h) = read(
            &write(&exr, image::ImageFormat::OpenExr),
            image::ImageFormat::OpenExr,
        )
        .unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgba, [188, 255, 0, 128, 255, 0, 137, 255]);
        // HDR は RGB だけ（アルファは不透明）。Radiance の形式は 8 ビットの仮数で、0.5 は 188 前後に収まる
        let hdr = image::DynamicImage::ImageRgb32F(
            image::ImageBuffer::from_raw(2, 1, vec![0.5f32, 0.5, 0.5, 8.0, -1.0, 1.0]).unwrap(),
        );
        let (rgba, w, h) = read(
            &write(&hdr, image::ImageFormat::Hdr),
            image::ImageFormat::Hdr,
        )
        .unwrap();
        assert_eq!((w, h), (2, 1));
        for c in &rgba[..3] {
            assert!(c.abs_diff(188) <= 2, "{rgba:?}");
        }
        assert_eq!(rgba[3], 255);
        assert_eq!(&rgba[4..], [255, 0, 255, 255], "1 超は 255・負は 0");
    }

    #[test]
    fn broken_and_oversized_files_are_refused_before_the_pixels_are_read() {
        for format in [
            image::ImageFormat::WebP,
            image::ImageFormat::Tiff,
            image::ImageFormat::Bmp,
            image::ImageFormat::OpenExr,
            image::ImageFormat::Hdr,
            image::ImageFormat::Gif,
        ] {
            assert_eq!(
                read(b"not an image at all", format).unwrap_err(),
                Problem::Unreadable,
                "{}",
                format_label(format)
            );
        }
        let wide = image::DynamicImage::ImageRgb8(image::RgbImage::new(MAX_SIDE + 1, 1));
        let bytes = write(&wide, image::ImageFormat::Bmp);
        assert_eq!(
            read(&bytes, image::ImageFormat::Bmp).unwrap_err(),
            Problem::Sides
        );
    }

    #[test]
    fn png_bytes_made_from_another_format_read_back() {
        let (rgba, w, h) = (vec![1u8, 2, 3, 4, 5, 6, 7, 8], 2, 1);
        let bytes = encode_png(&rgba, w, h).unwrap();
        assert_eq!(read(&bytes, image::ImageFormat::Png).unwrap(), (rgba, w, h));
    }

    #[test]
    fn the_table_covers_every_readable_extension_once() {
        let mut seen = std::collections::HashSet::new();
        for (ext, format, label) in FORMATS {
            assert!(seen.insert(*ext), "{ext}");
            assert_eq!(ext.to_ascii_lowercase(), *ext);
            assert!(!label.is_empty());
            assert_eq!(
                format_of(Path::new(&format!("a.{}", ext.to_ascii_uppercase()))),
                Some(*format)
            );
        }
        assert!(UNSUPPORTED_EXTENSIONS.iter().all(|e| !seen.contains(e)));
    }

    /// EXR のバイト列。表示ウィンドウは `display`（原点から）、データウィンドウは `data_at` から `data_size`。アルファのあるなしは `alpha`。
    fn exr_with_windows(
        display: (usize, usize),
        data_at: (i32, i32),
        data_size: (usize, usize),
        alpha: bool,
    ) -> Vec<u8> {
        use exr::prelude::*;
        let attributes = LayerAttributes {
            layer_position: Vec2(data_at.0, data_at.1),
            ..LayerAttributes::default()
        };
        // 画素は左から (0.5, 1, 0, 0.5)・(1, 1, 1, 1)・(0.25, 0, 0, 0)
        let color = |at: Vec2<usize>| match at.x() {
            0 => (0.5f32, 1.0f32, 0.0f32, 0.5f32),
            1 => (1.0, 1.0, 1.0, 1.0),
            _ => (0.25, 0.0, 0.0, 0.0),
        };
        let image_attributes = ImageAttributes::new(IntegerBounds::from_dimensions(display));
        let mut out = std::io::Cursor::new(Vec::new());
        if alpha {
            let layer = Layer::new(
                data_size,
                attributes,
                Encoding::FAST_LOSSLESS,
                SpecificChannels::rgba(color),
            );
            Image::empty(image_attributes)
                .with_layer(layer)
                .write()
                .to_buffered(&mut out)
                .unwrap();
        } else {
            let layer = Layer::new(
                data_size,
                attributes,
                Encoding::FAST_LOSSLESS,
                SpecificChannels::rgb(move |at: Vec2<usize>| {
                    let (r, g, b, _) = color(at);
                    (r, g, b)
                }),
            );
            Image::empty(image_attributes)
                .with_layer(layer)
                .write()
                .to_buffered(&mut out)
                .unwrap();
        }
        out.into_inner()
    }

    #[test]
    fn exr_places_the_data_window_inside_the_display_window() {
        let exr = image::ImageFormat::OpenExr;
        // データウィンドウが表示ウィンドウの 1 つ右から: 左の画素はデータの外（アルファのあるファイルは透明、無いファイルは不透明の黒）
        let with_alpha = read(&exr_with_windows((3, 1), (1, 0), (2, 1), true), exr).unwrap();
        assert_eq!(
            with_alpha,
            (vec![0, 0, 0, 0, 188, 255, 0, 128, 255, 255, 255, 255], 3, 1)
        );
        let no_alpha = read(&exr_with_windows((3, 1), (1, 0), (2, 1), false), exr).unwrap();
        assert_eq!(
            no_alpha,
            (
                vec![0, 0, 0, 255, 188, 255, 0, 255, 255, 255, 255, 255],
                3,
                1
            )
        );
        // データウィンドウが表示ウィンドウからはみ出す画素は捨てる
        let cropped = read(&exr_with_windows((2, 1), (0, 0), (3, 1), true), exr).unwrap();
        assert_eq!(cropped, (vec![188, 255, 0, 128, 255, 255, 255, 255], 2, 1));
    }

    #[test]
    fn exr_wider_than_the_limit_is_refused_by_its_header() {
        let wide = exr_with_windows(
            (MAX_SIDE as usize + 1, 1),
            (0, 0),
            (MAX_SIDE as usize + 1, 1),
            false,
        );
        assert_eq!(
            read(&wide, image::ImageFormat::OpenExr).unwrap_err(),
            Problem::Sides
        );
    }

    #[test]
    fn srgb8_bounds_match_the_formula() {
        // 表は f64 で計算して f32 に丸めた物と一致する
        let mut previous = 0.0f32;
        for (k, &bound) in SRGB8_BOUNDS.iter().enumerate() {
            let e = (k as f64 + 0.5) / 255.0;
            let linear = if e <= 0.04045 {
                e / 12.92
            } else {
                ((e + 0.055) / 1.055).powf(2.4)
            };
            assert_eq!(bound, linear as f32, "k = {k}");
            assert!(bound > previous, "増えていく: k = {k}");
            previous = bound;
            // 境目ちょうどは k + 1、その 1 つ下の f32 は k
            assert_eq!(usize::from(srgb8(bound)), k + 1, "k = {k}");
            assert_eq!(
                usize::from(srgb8(f32::from_bits(bound.to_bits() - 1))),
                k,
                "k = {k}"
            );
        }
        assert_eq!((srgb8(0.0), srgb8(1.0), srgb8(0.5)), (0, 255, 188));
        assert_eq!((srgb8(f32::NAN), srgb8(-1.0), srgb8(9.0)), (0, 0, 255));
    }

    #[test]
    fn srgb8_agrees_with_the_curve_except_on_the_rounding_edges() {
        // 曲線そのまま（f64）で丸めた値と、境目から離れた所では必ず一致する
        for i in 0..=20_000u32 {
            let v = i as f64 / 20_000.0;
            let encoded = if v <= 0.003_130_8 {
                12.92 * v
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            } * 255.0;
            if (encoded.fract() - 0.5).abs() < 1e-3 {
                continue;
            }
            assert_eq!(f64::from(srgb8(v as f32)), encoded.round(), "v = {v}");
        }
    }

    #[test]
    fn the_allocation_peak_is_counted_before_reading() {
        let rgb32f = |side: u32| u64::from(side) * u64::from(side) * 12;
        let rgba32f = |side: u32| u64::from(side) * u64::from(side) * 16;
        let hdr = image::ImageFormat::Hdr;
        let tiff = image::ImageFormat::Tiff;
        // RGB32F の 8192² は 768 MiB + 256 MiB でちょうど 1 GiB（≤ で通す）
        assert!(within_alloc(hdr, rgb32f(8192), 8192, 8192));
        // 1 画素ぶん増えただけでも断る
        assert!(!within_alloc(hdr, rgb32f(8192) + 12, 8192, 8192));
        assert!(!within_alloc(hdr, rgba32f(8192), 8192, 8192));
        // TIFF は読む間バッファの 2 倍
        assert!(!within_alloc(tiff, rgb32f(8192), 8192, 8192));
        assert!(within_alloc(tiff, rgb32f(6600), 6600, 6600));
        assert!(within_alloc(tiff, rgba32f(5700), 5700, 5700));
        assert!(!within_alloc(tiff, rgba32f(5800), 5800, 5800));
        // 16 ビットの RGBA の 8192² は 512 MiB の 2 倍でちょうど 1 GiB
        assert!(within_alloc(tiff, 8192 * 8192 * 8, 8192, 8192));
    }
}
