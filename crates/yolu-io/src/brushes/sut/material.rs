//! 素材（`MaterialFile.FileData`）から画像（PNG）を取り出す。
//!
//! 公開の解析では、`FileData` は無圧縮の tar で、その中に素材のプレビュー（`thumbnail/thumbnail.png`）の PNG がある。元の大きさの
//! 画像は CLIP STUDIO 独自の入れ物（`.layer`・`.c2f`）の中にあり、読まない。tar でないときは、先頭が PNG か、PNG が
//! 埋め込まれていればその最後の 1 枚を取る。

use super::tar;

/// 1 枚の PNG の大きさの上限（バイト）。
pub(super) const MAX_PNG_BYTES: usize = 16 * 1024 * 1024;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// 素材から取り出した画像。
pub(super) struct Image {
    pub png: Vec<u8>,
    /// プレビューの画像か（原寸ではない）。
    pub preview: bool,
}

fn is_png(data: &[u8]) -> bool {
    data.starts_with(PNG_SIGNATURE)
}

/// 最後の PNG の署名から、その後ろの最後の `IEND` の鎖（4 バイトの CRC まで）までを切り出す。
fn scan(blob: &[u8]) -> Option<&[u8]> {
    let start = blob
        .windows(PNG_SIGNATURE.len())
        .rposition(|w| w == PNG_SIGNATURE)?;
    let tail = &blob[start..];
    let iend = tail.windows(4).rposition(|w| w == b"IEND")?;
    let end = iend + 4 + 4;
    Some(&tail[..end.min(tail.len())])
}

/// 素材の画像。見つからなければ None。
pub(super) fn extract(blob: &[u8]) -> Option<Image> {
    if tar::looks_like_tar(blob) {
        let entries = tar::entries(blob);
        // プレビューでない PNG（原寸）を優先し、大きい方を取る。無ければプレビュー
        let mut best: Option<(bool, &[u8])> = None;
        for entry in entries.iter().take(32) {
            if !is_png(entry.data) || entry.data.len() > MAX_PNG_BYTES {
                continue;
            }
            let preview = entry.name.to_ascii_lowercase().contains("thumbnail");
            let better = match best {
                None => true,
                Some((best_preview, data)) => {
                    (best_preview && !preview)
                        || (best_preview == preview && entry.data.len() > data.len())
                }
            };
            if better {
                best = Some((preview, entry.data));
            }
        }
        if let Some((preview, data)) = best {
            return Some(Image {
                png: data.to_vec(),
                preview,
            });
        }
    }
    if is_png(blob) && blob.len() <= MAX_PNG_BYTES {
        return Some(Image {
            png: blob.to_vec(),
            preview: false,
        });
    }
    let found = scan(blob)?;
    if found.len() > MAX_PNG_BYTES || !is_png(found) {
        return None;
    }
    Some(Image {
        png: found.to_vec(),
        preview: true,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tar::build::tar;
    use super::*;

    fn png(tag: u8, size: usize) -> Vec<u8> {
        let mut v = PNG_SIGNATURE.to_vec();
        v.extend(std::iter::repeat_n(tag, size));
        v.extend_from_slice(b"\0\0\0\0IEND\xAE\x42\x60\x82");
        v
    }

    #[test]
    fn the_thumbnail_is_the_preview_and_a_full_size_png_wins_over_it() {
        let thumb = png(1, 10);
        let blob = tar(&[
            ("thumbnail/thumbnail.png", &thumb),
            ("x.layer", b"not a png"),
        ]);
        let image = extract(&blob).unwrap();
        assert!(image.preview);
        assert_eq!(image.png, thumb);
        let big = png(2, 40);
        let blob = tar(&[("thumbnail/thumbnail.png", &thumb), ("tip.png", &big)]);
        let image = extract(&blob).unwrap();
        assert!(!image.preview);
        assert_eq!(image.png, big);
    }

    #[test]
    fn a_plain_png_or_an_embedded_one_is_taken_and_anything_else_is_not() {
        let p = png(3, 5);
        assert!(!extract(&p).unwrap().preview);
        let mut wrapped = b"C2FHEADER....".to_vec();
        wrapped.extend_from_slice(&p);
        wrapped.extend_from_slice(b"trailing bytes");
        let image = extract(&wrapped).unwrap();
        assert!(image.preview);
        assert_eq!(image.png, p);
        assert!(extract(b"nothing here").is_none());
        assert!(extract(&[]).is_none());
        // 画像かどうかは取り出しでは確かめない（先頭が署名なら渡し、読めるかは復号が決める）。埋め込みは IEND まで揃っているものだけ
        assert!(extract(PNG_SIGNATURE).is_some());
        let mut truncated = b"xx".to_vec();
        truncated.extend_from_slice(PNG_SIGNATURE);
        truncated.extend_from_slice(b"no end chunk");
        assert!(extract(&truncated).is_none());
    }
}
