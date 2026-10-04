//! ライブラリの PNG を読む。寸法はヘッダーで先に見て、大きすぎる画像は画素を展開する前に断る。
//!
//! 読むと straight RGBA8 になる。パレット・グレー・インターレース・1〜4 ビットの PNG は、画素の値を変えずに 8 ビットの RGBA へ
//! 広げるだけ。16 ビットの PNG だけは値が丸められる（`rounds_to_8_bit`。使うときに知らせる）。ICC・ガンマ（iCCP・gAMA）は使わず、
//! 保存された値が画像そのもの（Unity 版の `RgbaPng` と同じ。Unity 版は、自分の読み方に当てはまらない PNG を Unity の復号器で読み、
//! 「Unity が復号した」と知らせる）。
use image::ImageDecoder;

/// 取り込める画像の 1 辺の上限（棚の画像・core の画像の入力と同じ）。
pub const MAX_SIDE: u32 = 8192;

/// 読めない理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// PNG として読めない（壊れている・PNG でない）。
    Unreadable,
    /// 1 辺が 1〜8192 の外。
    Size,
}

fn decoder(
    bytes: &[u8],
) -> Result<image::codecs::png::PngDecoder<std::io::Cursor<&[u8]>>, Problem> {
    image::codecs::png::PngDecoder::new(std::io::Cursor::new(bytes))
        .map_err(|_| Problem::Unreadable)
}

/// PNG のヘッダー（IHDR）の、1 画素の 1 色あたりのビット数が 16 か（読むと 8 ビットへ丸められる）。ヘッダーが読めなければ false。
pub fn rounds_to_8_bit(bytes: &[u8]) -> bool {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.len() >= 25 && bytes[..8] == SIGNATURE && &bytes[12..16] == b"IHDR" && bytes[24] == 16
}

/// 画像の寸法（画素は展開しない）。
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), Problem> {
    let (w, h) = decoder(bytes)?.dimensions();
    if !(1..=MAX_SIDE).contains(&w) || !(1..=MAX_SIDE).contains(&h) {
        return Err(Problem::Size);
    }
    Ok((w, h))
}

/// PNG を straight RGBA8（上の行が先）で読む。寸法を先に見て、確保にも上限を持つ。
pub fn decode(bytes: &[u8]) -> Result<image::RgbaImage, Problem> {
    let mut decoder = decoder(bytes)?;
    let (w, h) = decoder.dimensions();
    if !(1..=MAX_SIDE).contains(&w) || !(1..=MAX_SIDE).contains(&h) {
        return Err(Problem::Size);
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(1 << 30);
    decoder
        .set_limits(limits)
        .map_err(|_| Problem::Unreadable)?;
    Ok(image::DynamicImage::from_decoder(decoder)
        .map_err(|_| Problem::Unreadable)?
        .into_rgba8())
}

/// 行の並びを上下逆にする（1 行は `stride` バイト）。文書の向き（下の行が先）と画像の向き（上の行が先）を入れ替える。
pub fn flip_rows(raw: &mut [u8], stride: usize) {
    let rows = raw.len() / stride;
    for y in 0..rows / 2 {
        let (head, tail) = raw.split_at_mut((rows - 1 - y) * stride);
        head[y * stride..(y + 1) * stride].swap_with_slice(&mut tail[..stride]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut img = image::RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                img.put_pixel(x, y, image::Rgba(f(x, y)));
            }
        }
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn dimensions_are_read_without_decoding_and_limits_are_enforced() {
        assert_eq!(dimensions(&png(3, 2, |_, _| [0; 4])), Ok((3, 2)));
        assert_eq!(dimensions(b"not a png"), Err(Problem::Unreadable));
        assert_eq!(dimensions(&[]), Err(Problem::Unreadable));
        // 1 辺が上限を超える PNG は、画素を展開する前（ヘッダーだけ）で断る
        let wide = |w: u32, h: u32| {
            let mut out = Vec::new();
            let mut encoder = png::Encoder::new(&mut out, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&vec![0u8; w as usize * h as usize * 4])
                .unwrap();
            writer.finish().unwrap();
            out
        };
        assert_eq!(dimensions(&wide(MAX_SIDE + 1, 1)), Err(Problem::Size));
        assert_eq!(dimensions(&wide(1, MAX_SIDE + 1)), Err(Problem::Size));
        assert_eq!(dimensions(&wide(MAX_SIDE, 1)), Ok((MAX_SIDE, 1)));
        assert_eq!(decode(&wide(MAX_SIDE + 1, 1)).unwrap_err(), Problem::Size);
    }

    #[test]
    fn only_a_16_bit_png_is_rounded_to_8_bits_by_reading() {
        let write = |color: png::ColorType, depth: png::BitDepth, data: &[u8], palette: bool| {
            let mut out = Vec::new();
            let mut encoder = png::Encoder::new(&mut out, 1, 1);
            encoder.set_color(color);
            encoder.set_depth(depth);
            if palette {
                encoder.set_palette(vec![10, 20, 30]);
            }
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(data).unwrap();
            writer.finish().unwrap();
            out
        };
        let sixteen = write(
            png::ColorType::Rgba,
            png::BitDepth::Sixteen,
            &[0x12; 8],
            false,
        );
        assert!(rounds_to_8_bit(&sixteen));
        assert_eq!(decode(&sixteen).unwrap().get_pixel(0, 0).0, [0x12; 4]);
        let grey16 = write(
            png::ColorType::Grayscale,
            png::BitDepth::Sixteen,
            &[0x80, 0x80],
            false,
        );
        assert!(rounds_to_8_bit(&grey16));
        // 値が変わらない形（8 ビットの RGBA・グレー・パレット・2 ビットのグレー）は、丸めたと言わない
        for (color, depth, data, palette) in [
            (
                png::ColorType::Rgba,
                png::BitDepth::Eight,
                vec![1, 2, 3, 4],
                false,
            ),
            (
                png::ColorType::Grayscale,
                png::BitDepth::Eight,
                vec![7],
                false,
            ),
            (png::ColorType::Indexed, png::BitDepth::Eight, vec![0], true),
            (
                png::ColorType::Grayscale,
                png::BitDepth::Two,
                vec![0b1100_0000],
                false,
            ),
        ] {
            let bytes = write(color, depth, &data, palette);
            assert!(!rounds_to_8_bit(&bytes), "{color:?} {depth:?}");
            assert!(decode(&bytes).is_ok(), "{color:?} {depth:?}");
        }
        assert!(!rounds_to_8_bit(b"not a png"));
        assert!(!rounds_to_8_bit(&sixteen[..20]));
    }

    #[test]
    fn decoding_keeps_straight_pixels_including_transparent_rgb() {
        let bytes = png(2, 2, |x, y| {
            [x as u8 * 100, y as u8 * 50, 9, if x == y { 0 } else { 255 }]
        });
        let img = decode(&bytes).unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 9, 0]);
        assert_eq!(img.get_pixel(1, 0).0, [100, 0, 9, 255]);
        assert_eq!(img.get_pixel(0, 1).0, [0, 50, 9, 255]);
        assert_eq!(img.get_pixel(1, 1).0, [100, 50, 9, 0]);
        // 切れた PNG は読めない
        let cut = &bytes[..bytes.len() - 20];
        assert_eq!(decode(cut).unwrap_err(), Problem::Unreadable);
    }

    #[test]
    fn rows_flip_between_image_and_document_order() {
        let mut raw = vec![1, 1, 2, 2, 3, 3];
        flip_rows(&mut raw, 2);
        assert_eq!(raw, vec![3, 3, 2, 2, 1, 1]);
        let mut odd = vec![1, 2, 3, 4];
        flip_rows(&mut odd, 2);
        assert_eq!(odd, vec![3, 4, 1, 2]);
        let mut none: Vec<u8> = Vec::new();
        flip_rows(&mut none, 4);
        assert!(none.is_empty());
    }
}
