//! 素材（`MaterialFile.FileData`）から画像（PNG）を取り出す。
//!
//! `FileData` は無圧縮の tar。元の大きさの画像は CLIP STUDIO 独自の入れ物 C2F（`data/material.layer`。先頭が `\x89C2F`）の中にあり、
//! 読めたらそれを使う（[`c2f`]。灰色の PNG にして返す。暗いほど塗る）。読めないときは、素材のプレビュー（`thumbnail/thumbnail.png`）の PNG。
//! tar でないときは、先頭が PNG か、PNG が埋め込まれていればその最後の 1 枚を取る。
//!
//! 本物の `.sut`（CLIP STUDIO の書き出し）では、プレビューがどの素材にも同じ汎用の絵のことがある（素材の絵ではない）。知っている汎用の
//! 絵（[`PLACEHOLDER_PREVIEWS`]）は画像として使わない（違う絵を筆先にしない）。

use super::{c2f, tar};

/// 1 枚の PNG の大きさの上限（バイト）。
pub(super) const MAX_PNG_BYTES: usize = 16 * 1024 * 1024;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// CLIP STUDIO の独自の入れ物の先頭。
const C2F_SIGNATURE: &[u8] = b"\x89C2F";

/// 素材の絵ではない汎用のプレビュー（PNG の SHA-256）。本物の `.sut` で、全部の素材に同じ絵が入っていたもの。
const PLACEHOLDER_PREVIEWS: &[&str] = &["ef425e25330db3d6288c9c42d2229de46fd7d6c470474a5df8b77e46917e99e9"];

fn is_placeholder(png: &[u8], placeholders: &[&str]) -> bool {
    placeholders.contains(&crate::hash(png).as_str())
}

/// 素材の画像が CLIP STUDIO 独自の入れ物（`.layer`・`.c2f`）だけに入っているか（tar の中に独自の入れ物がある）。読めないことを
/// 知らせるのに使う。
pub(super) fn has_proprietary_image(blob: &[u8]) -> bool {
    tar::looks_like_tar(blob)
        && tar::entries(blob).iter().take(32).any(|e| {
            let name = e.name.to_ascii_lowercase();
            e.data.starts_with(C2F_SIGNATURE) || name.ends_with(".layer") || name.ends_with(".c2f")
        })
}

/// C2F の面（黒の不透明度 v）を灰色の PNG にする。輝度は 255 − v: 黒のインクを白い紙に重ねた見え方で、暗いほど塗る（PNG の筆先と同じ向き）。
fn plane_png(plane: &c2f::Plane) -> Option<Vec<u8>> {
    let gray: Vec<u8> = plane.values.iter().map(|v| 255 - v).collect();
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, plane.width, plane.height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.write_header().ok()?.write_image_data(&gray).ok()?;
    (out.len() <= MAX_PNG_BYTES).then_some(out)
}

/// 素材の独自の入れ物（C2F）の元の画像。読めなければ None。
fn layer_image(entries: &[tar::Entry<'_>], work: &c2f::Work) -> Option<Image> {
    entries
        .iter()
        .take(32)
        .filter(|e| e.data.starts_with(C2F_SIGNATURE))
        .find_map(|e| c2f::read(e.data, work).ok())
        .and_then(|plane| plane_png(&plane))
        .map(|png| Image {
            png,
            preview: false,
        })
}

/// 素材の種類（素材の `icedata/layerData.xml` の `systemtag` から）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// ブラシの筆先の画像（`BrushPattern`）。
    Tip,
    /// 質感の画像（`PaperTexture`）。
    Texture,
    /// ほかの種類（どちらでもない・両方の印が付いている）。
    Other,
}

/// 素材の種類。`layerData.xml` が無い・読めないときは None。
pub(super) fn kind(blob: &[u8]) -> Option<Kind> {
    if !tar::looks_like_tar(blob) {
        return None;
    }
    let entries = tar::entries(blob);
    let xml = entries
        .iter()
        .take(32)
        .find(|e| e.name.to_ascii_lowercase().ends_with("layerdata.xml"))?;
    if xml.data.len() > 64 * 1024 {
        return None;
    }
    let text = String::from_utf8_lossy(xml.data);
    let tail = &text[text.find("key=\"systemtag\"")?..];
    let section = &tail[..tail.find("</datalist>")?];
    let tags: Vec<&str> = section
        .split("<data>")
        .skip(1)
        .filter_map(|part| part.split("</data>").next())
        .take(16)
        .collect();
    let tip = tags.contains(&"BrushPattern");
    let texture = tags.contains(&"PaperTexture");
    Some(match (tip, texture) {
        (true, false) => Kind::Tip,
        (false, true) => Kind::Texture,
        _ => Kind::Other,
    })
}

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

/// 素材の画像。見つからなければ None。`work` は 1 回の取り込みで素材の C2F を読むのに使える仕事（全部の素材で共有）。
pub(super) fn extract(blob: &[u8], work: &c2f::Work) -> Option<Image> {
    extract_with(blob, PLACEHOLDER_PREVIEWS, work)
}

fn extract_with(blob: &[u8], placeholders: &[&str], work: &c2f::Work) -> Option<Image> {
    if tar::looks_like_tar(blob) {
        let entries = tar::entries(blob);
        // 独自の入れ物の元の画像が読めればそれ。読めなければ、プレビューでない PNG（原寸）を優先し、大きい方を取る。無ければプレビュー
        if let Some(image) = layer_image(&entries, work) {
            return Some(image);
        }
        let mut best: Option<(bool, &[u8])> = None;
        for entry in entries.iter().take(32) {
            if !is_png(entry.data) || entry.data.len() > MAX_PNG_BYTES {
                continue;
            }
            let preview = entry.name.to_ascii_lowercase().contains("thumbnail");
            if preview && is_placeholder(entry.data, placeholders) {
                continue;
            }
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
        // tar の中に使える PNG が無ければ無い（埋め込みの PNG を探し直すと、外した汎用の絵を拾ってしまう）
        return best.map(|(preview, data)| Image {
            png: data.to_vec(),
            preview,
        });
    }
    if is_png(blob) && blob.len() <= MAX_PNG_BYTES {
        return Some(Image {
            png: blob.to_vec(),
            preview: false,
        });
    }
    let found = scan(blob)?;
    if found.len() > MAX_PNG_BYTES || !is_png(found) || is_placeholder(found, placeholders) {
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

    fn work() -> c2f::Work {
        c2f::Work::new()
    }

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
        let image = extract(&blob, &work()).unwrap();
        assert!(image.preview);
        assert_eq!(image.png, thumb);
        let big = png(2, 40);
        let blob = tar(&[("thumbnail/thumbnail.png", &thumb), ("tip.png", &big)]);
        let image = extract(&blob, &work()).unwrap();
        assert!(!image.preview);
        assert_eq!(image.png, big);
    }

    #[test]
    fn a_known_placeholder_preview_is_not_taken_and_the_proprietary_image_is_reported() {
        let thumb = png(4, 10);
        let blob = tar(&[("thumbnail/thumbnail.png", &thumb), ("data/material.layer", b"\x89C2F\r\n\x1a\nbody")]);
        let hash = crate::hash(&thumb);
        assert!(extract_with(&blob, &[hash.as_str()], &work()).is_none(), "汎用の絵は筆先にしない");
        assert!(extract_with(&blob, &[], &work()).is_some(), "知らない絵はこれまでどおりプレビューとして使う");
        assert!(has_proprietary_image(&blob));
        assert!(!has_proprietary_image(&tar(&[("thumbnail/thumbnail.png", &thumb)])));
        assert!(!has_proprietary_image(&thumb), "tar でないものは数えない");
    }

    #[test]
    fn a_plain_png_or_an_embedded_one_is_taken_and_anything_else_is_not() {
        let p = png(3, 5);
        assert!(!extract(&p, &work()).unwrap().preview);
        let mut wrapped = b"C2FHEADER....".to_vec();
        wrapped.extend_from_slice(&p);
        wrapped.extend_from_slice(b"trailing bytes");
        let image = extract(&wrapped, &work()).unwrap();
        assert!(image.preview);
        assert_eq!(image.png, p);
        assert!(extract(b"nothing here", &work()).is_none());
        assert!(extract(&[], &work()).is_none());
        // 画像かどうかは取り出しでは確かめない（先頭が署名なら渡し、読めるかは復号が決める）。埋め込みは IEND まで揃っているものだけ
        assert!(extract(PNG_SIGNATURE, &work()).is_some());
        let mut truncated = b"xx".to_vec();
        truncated.extend_from_slice(PNG_SIGNATURE);
        truncated.extend_from_slice(b"no end chunk");
        assert!(extract(&truncated, &work()).is_none());
    }
}

#[cfg(test)]
mod kind_tests {
    use super::super::tar::build::tar;
    use super::*;

    fn xml(tags: &[&str]) -> Vec<u8> {
        let data: String = tags.iter().map(|t| format!("<data>{t}</data>")).collect();
        format!("<infolist><info><datalist key=\"systemtag\">{data}</datalist><datalist key=\"scaling\"><data>BrushPattern</data></datalist></info></infolist>").into_bytes()
    }

    #[test]
    fn the_kind_comes_from_the_system_tags_only() {
        let of = |tags: &[&str]| kind(&tar(&[("icedata/layerData.xml", &xml(tags))]));
        assert_eq!(of(&["Resizable", "BrushPattern"]), Some(Kind::Tip));
        assert_eq!(of(&["PaperTexture"]), Some(Kind::Texture));
        assert_eq!(of(&["Resizable"]), Some(Kind::Other));
        assert_eq!(of(&["BrushPattern", "PaperTexture"]), Some(Kind::Other));
        // 別の datalist の中の BrushPattern は数えない
        assert_eq!(of(&[]), Some(Kind::Other));
        // layerData.xml が無い・systemtag が無い・tar でない
        assert_eq!(kind(&tar(&[("x.txt", b"x")])), None);
        let no_tag = tar(&[("icedata/layerData.xml", b"<infolist/>")]);
        assert_eq!(kind(&no_tag), None);
        assert_eq!(kind(b"not a tar"), None);
    }
}
