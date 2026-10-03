//! 変わったタイルを、Unity のテクスチャの CPU の写し（画像全体、下の行から）へ写す。帯（タイルを横に並べた小さな画像）にも写せば、
//! C# はその帯だけを GPU へ上げて CopyTexture でタイルの所へ置ける（4096² の全体を上げ直さない）。

use yolu_protocol::{ShmError, TileRead};

use crate::session::ChannelState;

/// 写した結果。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopyOutcome {
    pub tiles: u32,
    pub torn: u32,
    pub remaining: u32,
    /// 写した画素の範囲（x_min, y_min, x_max, y_max。max は含まない。写していなければ全部 0）。
    pub bbox: [u32; 4],
}

/// 帯: タイルを左から並べた画像（幅 = 帯のタイルの数 × タイルの大きさ、高さ = タイルの大きさ、下の行から）と、並べたタイルの座標。
pub struct Strip<'a> {
    pub pixels: &'a mut [u8],
    pub capacity: usize,
    pub coords: &'a mut [u32],
}

/// 汚れたタイルを写す（行優先の順）。帯があれば、帯のタイルの数まで写して止める（残りは次に）。ちぎれたタイルは汚れたまま残す。
pub fn copy_dirty(
    ch: &mut ChannelState,
    image: &mut [u8],
    mut strip: Option<Strip<'_>>,
) -> Result<CopyOutcome, ShmError> {
    let img = ch
        .image
        .as_ref()
        .ok_or(ShmError::Invalid("共有メモリを開けていない"))?;
    let l = *img.layout();
    if image.len() != l.width as usize * l.height as usize * 4 {
        return Err(ShmError::OutOfRange("画像の大きさ"));
    }
    let ts = l.tile_size as usize;
    if let Some(s) = &strip {
        if s.capacity == 0
            || s.pixels.len() != s.capacity * ts * ts * 4
            || s.coords.len() < s.capacity * 2
        {
            return Err(ShmError::OutOfRange("帯の大きさ"));
        }
    }
    let mut out = CopyOutcome {
        bbox: [u32::MAX, u32::MAX, 0, 0],
        ..CopyOutcome::default()
    };
    let width = l.width as usize;
    for i in 0..l.tile_count() {
        if !ch.dirty[i] {
            continue;
        }
        if let Some(s) = &strip {
            if out.tiles as usize >= s.capacity {
                break;
            }
        }
        let (x, y) = (
            (i % l.tiles_x as usize) as u32,
            (i / l.tiles_x as usize) as u32,
        );
        let (x0, y0, tw, th) = l.tile_rect(x, y);
        let read = match &mut strip {
            Some(s) => {
                let n = out.tiles as usize;
                let stride = s.capacity * ts * 4;
                let r = img.read_tile_rows(x, y, s.pixels, n * ts * 4, stride, true)?;
                if r == TileRead::Complete {
                    // 帯から画像へ（共有メモリを 2 度読まない。帯と画像が同じ中身になる）
                    for row in 0..th as usize {
                        let src = row * stride + n * ts * 4;
                        let dst = ((y0 as usize + row) * width + x0 as usize) * 4;
                        image[dst..dst + tw as usize * 4]
                            .copy_from_slice(&s.pixels[src..src + tw as usize * 4]);
                    }
                    s.coords[n * 2] = x;
                    s.coords[n * 2 + 1] = y;
                }
                r
            }
            None => img.read_tile_into_image(x, y, image)?,
        };
        if read == TileRead::Torn {
            out.torn += 1;
            continue;
        }
        ch.dirty[i] = false;
        ch.dirty_count -= 1;
        out.tiles += 1;
        out.bbox[0] = out.bbox[0].min(x0);
        out.bbox[1] = out.bbox[1].min(y0);
        out.bbox[2] = out.bbox[2].max(x0 + tw);
        out.bbox[3] = out.bbox[3].max(y0 + th);
    }
    if out.tiles == 0 {
        out.bbox = [0; 4];
    }
    out.remaining = ch.dirty_count;
    Ok(out)
}
