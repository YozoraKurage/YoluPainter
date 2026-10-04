//! 写し（core の `PixelClipboard`。左下原点・下の行が先）と OS の画像（上の行が先）の変換、と OS の画像の指紋。

use std::hash::{DefaultHasher, Hash, Hasher};

use yolu_core::{Channel, CoreError, PixelClipboard};

use super::os::ClipImage;

/// 写しを OS の画像（straight RGBA8、上の行から）にする。
pub fn to_image(clip: &PixelClipboard) -> ClipImage {
    let (width, height) = (clip.width(), clip.height());
    ClipImage {
        width,
        height,
        rgba: flip_rows(clip.pixels(), width as usize * 4),
    }
}

/// OS の画像を、外の画像の写し（`PixelClipboard::from_image`）にする。行はその場で並べ替える（画像を読んだ分のほかに、画像の大きさの
/// メモリを確保しない）。
pub fn from_image(mut image: ClipImage, channel: Channel) -> Result<PixelClipboard, CoreError> {
    flip_rows_in_place(&mut image.rgba, image.width as usize * 4);
    PixelClipboard::from_image(image.width, image.height, image.rgba, channel)
}

/// 行の並びを逆にした複製（上から ↔ 下から）。写しは共有なので、OS へ書く画像は複製を作る。
fn flip_rows(pixels: &[u8], row_bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len());
    for row in pixels.chunks_exact(row_bytes).rev() {
        out.extend_from_slice(row);
    }
    out
}

/// 行の並びをその場で逆にする。
fn flip_rows_in_place(pixels: &mut [u8], row_bytes: usize) {
    if row_bytes == 0 {
        return;
    }
    let rows = pixels.len() / row_bytes;
    for top in 0..rows / 2 {
        let (upper, lower) = pixels.split_at_mut((rows - 1 - top) * row_bytes);
        upper[top * row_bytes..(top + 1) * row_bytes].swap_with_slice(&mut lower[..row_bytes]);
    }
}

/// OS の画像の指紋。透明な画素の RGB は OS の側（ほかのアプリ・形式の変換）で落ちることがあるので入れない。アプリが最後に書いた
/// 画像と、今 OS にある画像が同じかを見るのに使う（同じなら、透明画素の RGB まで正確なアプリの中の写しを貼る）。
pub fn fingerprint(image: &ClipImage) -> u64 {
    let mut hasher = DefaultHasher::new();
    (image.width, image.height).hash(&mut hasher);
    let mut block = [0u8; 4096];
    for chunk in image.rgba.chunks(block.len()) {
        let block = &mut block[..chunk.len()];
        block.copy_from_slice(chunk);
        for pixel in block.as_chunks_mut::<4>().0 {
            if pixel[3] == 0 {
                pixel[..3].fill(0);
            }
        }
        hasher.write(block);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(width: u32, height: u32) -> PixelClipboard {
        let rgba = (0..width * height * 4).map(|i| (i % 251) as u8).collect();
        PixelClipboard::new(
            width,
            height,
            0,
            0,
            width,
            height,
            rgba,
            yolu_core::ClipboardSource::Layer,
            Channel::Color,
        )
        .unwrap()
    }

    #[test]
    fn rows_flip_between_the_core_order_and_the_os_order() {
        let c = clip(3, 2);
        let image = to_image(&c);
        assert_eq!((image.width, image.height), (3, 2));
        // 核の一番下の行（0 行目）が、OS の画像では最後の行
        assert_eq!(&image.rgba[12..24], &c.pixels()[0..12]);
        assert_eq!(&image.rgba[0..12], &c.pixels()[12..24]);
        let back = from_image(image, Channel::Roughness).unwrap();
        assert_eq!(back.pixels(), c.pixels());
        assert_eq!(back.channel(), Channel::Roughness);
        assert_eq!(back.source(), yolu_core::ClipboardSource::External);
        assert_eq!(back.document_size(), (3, 2));
    }

    #[test]
    fn flipping_in_place_gives_the_same_rows_as_the_copy_for_every_row_count() {
        for rows in 0..8usize {
            let original: Vec<u8> = (0..rows * 6).map(|i| (i * 7 % 251) as u8).collect();
            let mut flipped = original.clone();
            flip_rows_in_place(&mut flipped, 6);
            assert_eq!(flipped, flip_rows(&original, 6), "{rows} 行");
        }
        let mut nothing: Vec<u8> = Vec::new();
        flip_rows_in_place(&mut nothing, 0);
    }

    #[test]
    fn the_fingerprint_ignores_the_rgb_of_transparent_pixels_only() {
        let a = ClipImage::new(2, 1, vec![1, 2, 3, 0, 9, 9, 9, 255]).unwrap();
        let b = ClipImage::new(2, 1, vec![7, 7, 7, 0, 9, 9, 9, 255]).unwrap();
        let c = ClipImage::new(2, 1, vec![1, 2, 3, 0, 9, 9, 8, 255]).unwrap();
        let d = ClipImage::new(1, 2, vec![1, 2, 3, 0, 9, 9, 9, 255]).unwrap();
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert_ne!(fingerprint(&a), fingerprint(&c), "色の違い");
        assert_ne!(fingerprint(&a), fingerprint(&d), "大きさの違い");
    }

    #[test]
    fn a_clip_image_needs_matching_sizes() {
        assert!(ClipImage::new(2, 2, vec![0; 16]).is_some());
        assert!(ClipImage::new(2, 2, vec![0; 15]).is_none());
        assert!(ClipImage::new(0, 2, vec![]).is_none());
    }
}
