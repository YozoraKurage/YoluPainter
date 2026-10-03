use crate::{check, Result};
use flate2::{write::ZlibEncoder, Compression};
use std::io::Write;
use yolu_core::Document;

/// 現在のColor合成をRGBA8 PNGへ書く。上下の向き、行フィルター、チャンク配置は
/// Unity版RgbaPngと同じ。圧縮バイトはdeflate実装の版にも依存する。
pub fn composite_png(doc: &Document) -> Result<Vec<u8>> {
    check(
        doc.width() <= 8192 && doc.height() <= 8192,
        "PNGの寸法の上限は8192です",
    )?;
    let rgba = doc.composite(doc.bounds())?;
    encode(&rgba, doc.width(), doc.height())
}
fn encode(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let stride = width as usize * 4;
    let mut compressed = ZlibEncoder::new(Vec::new(), Compression::default());
    let mut row = vec![0; stride + 1];
    let mut best = row.clone();
    for y in (0..height as usize).rev() {
        let mut best_score = u64::MAX;
        for filter in 0..=4 {
            row[0] = filter;
            let mut score = 0;
            for i in 0..stride {
                let x = rgba[y * stride + i];
                let a = if i >= 4 { rgba[y * stride + i - 4] } else { 0 };
                let b = if y + 1 < height as usize {
                    rgba[(y + 1) * stride + i]
                } else {
                    0
                };
                let c = if i >= 4 && y + 1 < height as usize {
                    rgba[(y + 1) * stride + i - 4]
                } else {
                    0
                };
                let predicted = match filter {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                    _ => paeth(a, b, c),
                };
                let v = x.wrapping_sub(predicted);
                row[i + 1] = v;
                score += u64::from(v).min(256 - u64::from(v));
                if score >= best_score {
                    break;
                }
            }
            if score < best_score {
                best_score = score;
                best.copy_from_slice(&row);
            }
        }
        compressed.write_all(&best)?;
    }
    let zlib = compressed.finish()?;
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::from(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    for bytes in zlib.chunks(1 << 18) {
        chunk(&mut out, b"IDAT", bytes);
    }
    chunk(&mut out, b"IEND", &[]);
    Ok(out)
}
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i32::from(a) + i32::from(b) - i32::from(c);
    let da = (p - i32::from(a)).abs();
    let db = (p - i32::from(b)).abs();
    let dc = (p - i32::from(c)).abs();
    if da <= db && da <= dc {
        a
    } else if db <= dc {
        b
    } else {
        c
    }
}
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(kind);
    out.extend(data);
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    out.extend(crc.finalize().to_be_bytes());
}
