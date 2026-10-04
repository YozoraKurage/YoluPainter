//! 取り込んだ筆先・質感の画像のファイル（設定のフォルダの `brushes/images/<SHA-256>.png`）。
//!
//! 画像は名前・大きさ・画素の SHA-256 で名づける（内容が同じなら同じファイル。ABR のプリセットが同じ筆先を共有しても 1 枚しか
//! 置かず、同じファイルを取り込み直しても増えない）。形は 8 bit の灰色の PNG で、値は 255 − 覆い（暗いほど塗る。Krita・GIMP の
//! 筆先の約束と同じなので、ほかのアプリでもそのまま開ける）、行は上から。名前は PNG の `Title`（iTXt, UTF-8）に持つ。
//! 読むときは名前と画素から指紋を取り直してファイル名と突き合わせるので、壊れたファイル・名前だけ付け替えたファイルは
//! 別の画像として通らない。

use std::io::Cursor;

use sha2::{Digest, Sha256};
use yolu_core::BrushTip;

use super::store::StoreError;

/// 画像 1 ファイルの大きさの上限（2048 × 2048 の灰色の PNG が収まる）。
pub const MAX_IMAGE_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// PNG を展開して持ってよい大きさの上限。
const MAX_DECODED_BYTES: usize = 8 * 1024 * 1024;
/// 名前の長さの上限（バイト）。
const MAX_NAME_BYTES: usize = 4096;
const TITLE: &str = "Title";

/// 内容の SHA-256（小文字の 16 進 64 文字）。ファイル名になる。名前も含める（名前が違えば別の画像）。
pub fn fingerprint(tip: &BrushTip) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"yolupainter-tip 1\0");
    hasher.update(tip.name().as_bytes());
    hasher.update([0u8]);
    hasher.update(tip.width().to_le_bytes());
    hasher.update(tip.height().to_le_bytes());
    hasher.update(tip.alpha());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 指紋の形（小文字の 16 進 64 文字）か。ファイル名に使うので、パスの区切りなどが紛れ込まないことをここで保つ。
pub fn is_fingerprint(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// 画像を PNG にする。
pub fn encode(tip: &BrushTip) -> Result<Vec<u8>, StoreError> {
    let (w, h) = (tip.width() as usize, tip.height() as usize);
    // 筆先の行は下から、PNG の行は上から。値は 255 − 覆い
    let mut gray = vec![0u8; w * h];
    for y in 0..h {
        let from = &tip.alpha()[(h - 1 - y) * w..(h - y) * w];
        for (x, a) in from.iter().enumerate() {
            gray[y * w + x] = 255 - a;
        }
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, tip.width(), tip.height());
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .add_itxt_chunk(TITLE.into(), tip.name().into())
        .map_err(|_| StoreError::BadImage(String::new()))?;
    let mut writer = encoder
        .write_header()
        .map_err(|_| StoreError::BadImage(String::new()))?;
    writer
        .write_image_data(&gray)
        .map_err(|_| StoreError::BadImage(String::new()))?;
    writer
        .finish()
        .map_err(|_| StoreError::BadImage(String::new()))?;
    Ok(out)
}

/// PNG から画像を読む。この形（8 bit の灰色・上限以内の大きさ）でなければ断る。
pub fn decode(bytes: &[u8]) -> Result<BrushTip, StoreError> {
    let bad = || StoreError::BadImage(String::new());
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: MAX_DECODED_BYTES,
    });
    let mut reader = decoder.read_info().map_err(|_| bad())?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    if info.color_type != png::ColorType::Grayscale
        || info.bit_depth != png::BitDepth::Eight
        || width < 1
        || height < 1
        || width > BrushTip::MAX_SIZE
        || height > BrushTip::MAX_SIZE
    {
        return Err(bad());
    }
    let name = match info.utf8_text.iter().find(|c| c.keyword == TITLE) {
        Some(chunk) => chunk.get_text().map_err(|_| bad())?,
        None => String::new(),
    };
    if name.len() > MAX_NAME_BYTES {
        return Err(bad());
    }
    let size = reader.output_buffer_size().ok_or_else(bad)?;
    if size != width as usize * height as usize {
        return Err(bad());
    }
    let mut gray = vec![0u8; size];
    reader.next_frame(&mut gray).map_err(|_| bad())?;
    let (w, h) = (width as usize, height as usize);
    let mut alpha = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            alpha[(h - 1 - y) * w + x] = 255 - gray[y * w + x];
        }
    }
    BrushTip::new(&name, width, height, alpha).map_err(|_| bad())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tip(name: &str, w: u32, h: u32) -> BrushTip {
        let alpha: Vec<u8> = (0..w * h).map(|i| (i * 7 % 256) as u8).collect();
        BrushTip::new(name, w, h, alpha).unwrap()
    }

    #[test]
    fn an_image_round_trips_exactly_including_its_name() {
        for (name, w, h) in [("粒子 grain", 5, 3), ("", 1, 1), ("a\nb", 40, 64)] {
            let t = tip(name, w, h);
            let png = encode(&t).unwrap();
            assert_eq!(decode(&png).unwrap(), t, "{name:?}");
        }
    }

    #[test]
    fn rows_are_flipped_and_values_inverted_like_a_krita_tip() {
        // 筆先の下の行（先頭）が PNG の最後の行になり、覆い 255 が黒（0）になる
        let t = BrushTip::new("t", 2, 2, vec![255, 0, 0, 128]).unwrap();
        let png = encode(&t).unwrap();
        let mut reader = png::Decoder::new(Cursor::new(&png[..]))
            .read_info()
            .unwrap();
        let mut buffer = vec![0u8; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut buffer).unwrap();
        assert_eq!(buffer, [255, 127, 0, 255]);
    }

    #[test]
    fn the_fingerprint_depends_on_name_size_and_pixels_only() {
        let a = tip("a", 4, 4);
        assert_eq!(fingerprint(&a), fingerprint(&tip("a", 4, 4)));
        assert!(is_fingerprint(&fingerprint(&a)));
        assert_ne!(fingerprint(&a), fingerprint(&tip("b", 4, 4)));
        assert_ne!(
            fingerprint(&a),
            fingerprint(&tip("a", 8, 2)),
            "同じ画素数でも大きさが違えば別"
        );
        let mut other = a.alpha().to_vec();
        other[3] ^= 1;
        assert_ne!(
            fingerprint(&a),
            fingerprint(&BrushTip::new("a", 4, 4, other).unwrap())
        );
        assert!(!is_fingerprint("../x"));
        assert!(!is_fingerprint(&fingerprint(&a).to_uppercase()));
    }

    #[test]
    fn anything_but_an_eight_bit_gray_png_within_the_limit_is_refused() {
        assert!(decode(b"not a png").is_err());
        // 8 bit の RGB
        let mut rgb = Vec::new();
        {
            let mut e = png::Encoder::new(&mut rgb, 2, 2);
            e.set_color(png::ColorType::Rgb);
            e.set_depth(png::BitDepth::Eight);
            let mut w = e.write_header().unwrap();
            w.write_image_data(&[0u8; 12]).unwrap();
        }
        assert!(decode(&rgb).is_err());
        // 上限を超える大きさは画素を読む前に断る
        let mut big = Vec::new();
        {
            let mut e = png::Encoder::new(&mut big, BrushTip::MAX_SIZE + 1, 1);
            e.set_color(png::ColorType::Grayscale);
            e.set_depth(png::BitDepth::Eight);
            let mut w = e.write_header().unwrap();
            w.write_image_data(&vec![0u8; BrushTip::MAX_SIZE as usize + 1])
                .unwrap();
        }
        assert!(decode(&big).is_err());
    }
}
