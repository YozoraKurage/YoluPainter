//! 見本の画像: セット・チャンネル・最大の辺 → PNG。画素は core の正本の合成（`yolu_core::export::channel_image`。書き出しのチャンネルの
//! 画像と同じ）で、縮めるときは箱の平均（透明を含む所は、不透明度で重みを付けた色の平均なので、縁が黒くにじまない）。拡大はしない。
//! 縮めない大きさ（最大の辺が文書の長い辺以上）では、合成の画素そのままの PNG になる。

use serde_json::json;
use yolu_core::{Channel, Document};

use crate::command::PreviewArgs;
use crate::error::{ErrorCode, OpError};
use crate::host::SetView;
use crate::refs::{channel_name, resolve_channel};
use crate::reply::{PreviewInfo, Reply};
use crate::value::Bytes;

/// 最大の辺の既定。
pub const DEFAULT_MAX_EDGE: u32 = 512;
/// 最大の辺の上限（返事に載せる PNG が大きくなりすぎないように）。
pub const MAX_EDGE: u32 = 2048;
/// 合成の作業に許すバイト数（法線の出力。書き出しの合成の PNG と同じ値）。
const WORKING_BYTES: u64 = 256 * 1024 * 1024;

/// `preview` を当てる。
pub fn render(view: &SetView<'_>, args: &PreviewArgs) -> Result<Reply, OpError> {
    let doc = view.editable_doc()?;
    let channel = match &args.channel {
        Some(name) => resolve_channel(doc, name)?,
        None => Channel::Color,
    };
    let max_edge = args.max_edge.unwrap_or(DEFAULT_MAX_EDGE);
    if !(1..=MAX_EDGE).contains(&max_edge) {
        return Err(OpError::invalid_value(
            format!("max_edge は 1〜{MAX_EDGE} です"),
            format!("max_edge must be between 1 and {MAX_EDGE}"),
        )
        .with_data(json!({"min": 1, "max": MAX_EDGE})));
    }
    let image = render_image(doc, channel, max_edge)?;
    Ok(Reply::Preview(PreviewInfo {
        set: view.id.to_owned(),
        channel: channel_name(doc, channel),
        width: image.width,
        height: image.height,
        source_width: doc.width(),
        source_height: doc.height(),
        png: Bytes(image.png),
        inactive_effects: crate::doc_ops::inactive_texts(doc),
    }))
}

/// 作った見本。
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

/// チャンネルの合成を、長い辺が `max_edge` に収まるように縮めた PNG にする。
pub fn render_image(doc: &Document, channel: Channel, max_edge: u32) -> Result<Image, OpError> {
    let rgba =
        yolu_core::export::channel_image(doc, channel, WORKING_BYTES).map_err(|e| match e {
            yolu_core::export::ExportError::Core(core) => OpError::from_core(&core),
            other => OpError::new(
                ErrorCode::Budget,
                format!("見本を作れません: {other}"),
                "The preview cannot be built within the working memory limit",
            ),
        })?;
    let (w, h) = (doc.width(), doc.height());
    let (tw, th) = fit(w, h, max_edge);
    let pixels = if (tw, th) == (w, h) {
        rgba
    } else {
        shrink(&rgba, w, h, tw, th)
    };
    let png = encode_png(&pixels, tw, th).map_err(|e| {
        OpError::new(
            ErrorCode::Internal,
            format!("PNG を作れません: {e}"),
            format!("Cannot encode the PNG: {e}"),
        )
    })?;
    Ok(Image {
        width: tw,
        height: th,
        png,
    })
}

/// 縮めた大きさ。長い辺が `max_edge` に（元より大きくしない）、短い辺は比を保って四捨五入（1 以上）。
pub fn fit(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let long = width.max(height);
    if long <= max_edge {
        return (width, height);
    }
    let scale = |side: u32| {
        ((u64::from(side) * u64::from(max_edge) + u64::from(long) / 2) / u64::from(long)).max(1)
            as u32
    };
    (scale(width), scale(height))
}

/// 箱の平均で縮める（左下原点・行優先の straight RGBA8。出力も同じ並び）。出力の 1 画素は、元の画素のうち重なる分を面積で重み付けして平均する
/// （整数の重み。端の画素は重なる割合だけ）。RGB は不透明度でも重み付ける（不透明度が 0 の所の RGB は効かない）。不透明度は面積だけの平均。
pub fn shrink(src: &[u8], width: u32, height: u32, out_w: u32, out_h: u32) -> Vec<u8> {
    debug_assert_eq!(src.len(), width as usize * height as usize * 4);
    let spans = |from: u32, to: u32| -> Vec<Vec<(usize, u64)>> {
        // 出力の i 番目（元の座標で [i*from/to, (i+1)*from/to)）に重なる元の画素と、その重なりの長さ（to 倍した単位）
        (0..to)
            .map(|i| {
                let lo = u64::from(i) * u64::from(from);
                let hi = lo + u64::from(from);
                let first = (lo / u64::from(to)) as usize;
                let last = ((hi - 1) / u64::from(to)) as usize;
                (first..=last)
                    .map(|p| {
                        let a = (p as u64 * u64::from(to)).max(lo);
                        let b = ((p as u64 + 1) * u64::from(to)).min(hi);
                        (p, b - a)
                    })
                    .collect()
            })
            .collect()
    };
    let xs = spans(width, out_w);
    let ys = spans(height, out_h);
    let area = u64::from(width) * u64::from(height);
    let mut out = vec![0u8; out_w as usize * out_h as usize * 4];
    for (oy, yspan) in ys.iter().enumerate() {
        for (ox, xspan) in xs.iter().enumerate() {
            let (mut sa, mut sr, mut sg, mut sb) = (0u64, 0u64, 0u64, 0u64);
            for &(py, wy) in yspan {
                for &(px, wx) in xspan {
                    let w = wx * wy;
                    let i = (py * width as usize + px) * 4;
                    let a = u64::from(src[i + 3]);
                    sa += w * a;
                    sr += w * a * u64::from(src[i]);
                    sg += w * a * u64::from(src[i + 1]);
                    sb += w * a * u64::from(src[i + 2]);
                }
            }
            let o = (oy * out_w as usize + ox) * 4;
            // 不透明度が 0 の所（sa が 0）の RGB は 0 のまま
            for (k, sum) in [sr, sg, sb].into_iter().enumerate() {
                out[o + k] = (sum + sa / 2).checked_div(sa).unwrap_or(0) as u8;
            }
            out[o + 3] = ((sa + area / 2) / area) as u8;
        }
    }
    out
}

/// 左下原点の straight RGBA8 を、上の行から並べた RGBA の PNG にする。
pub fn encode_png(
    bottom_up: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, png::EncodingError> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        let stride = width as usize * 4;
        let mut top_down = Vec::with_capacity(bottom_up.len());
        for row in bottom_up.chunks_exact(stride).rev() {
            top_down.extend_from_slice(row);
        }
        writer.write_image_data(&top_down)?;
    }
    Ok(out)
}

/// 見本の PNG を、左下原点の straight RGBA8 に読み戻す（試験・呼び手が正本の合成と比べる）。
pub fn decode_png(png_bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), png::DecodingError> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info()?;
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or(png::DecodingError::LimitsExceeded)?
    ];
    let info = reader.next_frame(&mut buf)?;
    let (w, h) = (info.width, info.height);
    let stride = w as usize * 4;
    let mut bottom_up = Vec::with_capacity(stride * h as usize);
    for row in buf[..info.buffer_size()].chunks_exact(stride).rev() {
        bottom_up.extend_from_slice(row);
    }
    Ok((w, h, bottom_up))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_size_fits_the_longest_side_and_never_enlarges() {
        assert_eq!(fit(100, 50, 200), (100, 50));
        assert_eq!(fit(100, 50, 100), (100, 50));
        assert_eq!(fit(1000, 500, 100), (100, 50));
        assert_eq!(fit(500, 1000, 100), (50, 100));
        assert_eq!(fit(1000, 3, 100), (100, 1));
        assert_eq!(fit(1024, 1024, 1), (1, 1));
    }

    #[test]
    fn a_whole_number_shrink_is_the_plain_average() {
        // 4x2 → 2x1: 左の 2x2 の平均と右の 2x2 の平均（不透明）
        let mut src = Vec::new();
        for y in 0..2u8 {
            for x in 0..4u8 {
                src.extend_from_slice(&[x * 10 + y, 100, 200, 255]);
            }
        }
        let out = shrink(&src, 4, 2, 2, 1);
        // R: (0 + 10 + 1 + 11) / 4 = 5.5 → 6、(20 + 30 + 21 + 31) / 4 = 25.5 → 26
        assert_eq!(out, vec![6, 100, 200, 255, 26, 100, 200, 255]);
    }

    #[test]
    fn transparent_pixels_do_not_darken_the_color_of_an_edge() {
        // 2x1 → 1x1: 不透明の赤と、透明（RGB は 0）
        let src = [255, 0, 0, 255, 0, 0, 0, 0];
        let out = shrink(&src, 2, 1, 1, 1);
        assert_eq!(out, vec![255, 0, 0, 128]);
        // 全部透明なら RGB は 0
        assert_eq!(
            shrink(&[9, 9, 9, 0, 9, 9, 9, 0], 2, 1, 1, 1),
            vec![0, 0, 0, 0]
        );
    }

    #[test]
    fn a_fractional_shrink_weights_the_edge_pixels_by_their_overlap() {
        // 3x1 → 2x1: 出力の 0 番は元の [0, 1.5)、1 番は [1.5, 3)
        let px = |v: u8| [v, v, v, 255];
        let src: Vec<u8> = [px(0), px(90), px(180)].concat();
        let out = shrink(&src, 3, 1, 2, 1);
        // 0 番: (0 * 1 + 90 * 0.5) / 1.5 = 30、1 番: (90 * 0.5 + 180 * 1) / 1.5 = 150
        assert_eq!(out[0], 30);
        assert_eq!(out[4], 150);
    }

    #[test]
    fn png_round_trips_and_stores_the_top_row_first() {
        let (w, h) = (3u32, 2u32);
        let bottom_up: Vec<u8> = (0..w * h * 4).map(|i| (i * 7 + 3) as u8).collect();
        let png = encode_png(&bottom_up, w, h).unwrap();
        let (dw, dh, back) = decode_png(&png).unwrap();
        assert_eq!((dw, dh), (w, h));
        assert_eq!(back, bottom_up);
    }
}
