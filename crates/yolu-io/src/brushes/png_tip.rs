//! PNG の筆先。Krita・GIMP の約束で読む: 暗いほど塗り、白と透明は塗らない（被覆率 = (1 − 輝度) × アルファ）。
//!
//! 1 辺が上限を超える画像は、画素を読む前に断る。デコーダーの診断（英語）は運ばず、読めなければ `NotPng`。

use std::io::Cursor;

use yolu_core::BrushTip;

use super::error::{Fault, Result, SizedItem};

pub(crate) fn read_png_tip(bytes: &[u8], name: &str) -> Result<BrushTip> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    // パレット・8 bit 未満・tRNS を展開し、16 bit は上位バイトにする（読み込み後は 8 bit の灰・灰+アルファ・RGB・RGBA だけ）
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    decoder.set_limits(png::Limits {
        bytes: 64 * 1024 * 1024,
    });
    let mut reader = decoder.read_info().map_err(|e| match e {
        png::DecodingError::LimitsExceeded => Fault::PngLimits,
        _ => Fault::NotPng,
    })?;
    let (width, height) = (reader.info().width, reader.info().height);
    let side = BrushTip::MAX_SIZE;
    if width < 1 || height < 1 || width > side || height > side {
        return Err(Fault::SizeOutOfRange {
            what: SizedItem::Image,
            width: width as i64,
            height: height as i64,
        });
    }
    let size = reader.output_buffer_size().ok_or(Fault::PngLimits)?;
    let mut buffer = vec![0u8; size];
    let frame = reader.next_frame(&mut buffer).map_err(|e| match e {
        png::DecodingError::LimitsExceeded => Fault::PngLimits,
        _ => Fault::NotPng,
    })?;
    let (w, h) = (frame.width as usize, frame.height as usize);
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err(Fault::NotPng), // EXPAND で展開済みのはず
    };
    if frame.bit_depth != png::BitDepth::Eight || buffer.len() < w * h * channels {
        return Err(Fault::NotPng);
    }
    let mut alpha = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let p = &buffer[(y * w + x) * channels..(y * w + x + 1) * channels];
            let (lum, a) = match channels {
                1 => (p[0] as u32, 255u32),
                2 => (p[0] as u32, p[1] as u32),
                3 => (luminance(p[0], p[1], p[2]), 255),
                _ => (luminance(p[0], p[1], p[2]), p[3] as u32),
            };
            // PNG の行は上から。筆先は下の行が先
            alpha[(h - 1 - y) * w + x] = ((255 - lum) * a / 255) as u8;
        }
    }
    BrushTip::new(name, w as u32, h as u32, alpha).map_err(|_| Fault::SizeOutOfRange {
        what: SizedItem::Image,
        width: w as i64,
        height: h as i64,
    })
}

fn luminance(r: u8, g: u8, b: u8) -> u32 {
    (r as u32 * 299 + g as u32 * 587 + b as u32 * 114 + 500) / 1000
}
