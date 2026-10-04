use super::{budget, canceled, dimensions, zeroes, FillError};
use crate::math::to_byte;
use crate::Rgba8;
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// 0 段の画素: 借用か、共有して持つか（文書が画像のミップマップを持ち続けるときは共有）。
enum Original<'a> {
    Borrowed(&'a [u8]),
    Shared(Arc<[u8]>),
}
impl Original<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            Original::Borrowed(b) => b,
            Original::Shared(a) => a,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Conversion {
    #[default]
    None,
    LinearToSrgb,
}

/// 段 0 は借用、追加段だけを所有。予算は C# 同様、追加段の RGBA8 の総バイト数。
pub struct ImageMipChain<'a> {
    original: Original<'a>,
    levels: Vec<(usize, usize, Vec<u8>)>,
    lut: Option<[u8; 256]>,
    luminance: bool,
    bytes: u64,
}
impl<'a> ImageMipChain<'a> {
    pub fn extra_bytes(width: u32, height: u32) -> Result<u64, FillError> {
        dimensions(width, height)?;
        let (mut w, mut h) = (width as u64, height as u64);
        let mut sum = 0;
        while w > 1 || h > 1 {
            w = (w / 2).max(1);
            h = (h / 2).max(1);
            sum += w * h * 4;
        }
        Ok(sum)
    }
    pub fn build(
        pixels: &'a [u8],
        width: u32,
        height: u32,
        conversion: Conversion,
        luminance: bool,
        limit: u64,
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, FillError> {
        Self::build_from(
            Original::Borrowed(pixels),
            width,
            height,
            conversion,
            luminance,
            limit,
            cancel,
        )
    }
    /// 0 段の画素を共有して持つ形（文書が使い回すミップマップ）。借用の `build` と同じ値になる。
    pub fn build_shared(
        pixels: Arc<[u8]>,
        width: u32,
        height: u32,
        conversion: Conversion,
        luminance: bool,
        limit: u64,
        cancel: Option<&AtomicBool>,
    ) -> Result<ImageMipChain<'static>, FillError> {
        ImageMipChain::build_from(
            Original::Shared(pixels),
            width,
            height,
            conversion,
            luminance,
            limit,
            cancel,
        )
    }
    fn build_from(
        original: Original<'a>,
        width: u32,
        height: u32,
        conversion: Conversion,
        luminance: bool,
        limit: u64,
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, FillError> {
        let pixels_len = original.bytes().len();
        let n = dimensions(width, height)?;
        if pixels_len != n * 4 {
            return Err(FillError::Invalid("画像のバイト数"));
        }
        let bytes = Self::extra_bytes(width, height)?;
        budget(bytes, limit)?;
        canceled(cancel)?;
        let lut = (conversion == Conversion::LinearToSrgb).then(|| {
            std::array::from_fn(|i| {
                let c = i as f64 / 255.;
                to_byte(if c <= 0.0031308 {
                    12.92 * c
                } else {
                    1.055 * c.powf(1. / 2.4) - 0.055
                })
            })
        });
        let mut chain = Self {
            original,
            levels: vec![(width as usize, height as usize, Vec::new())],
            lut,
            luminance,
            bytes,
        };
        let (mut w, mut h) = (width as usize, height as usize);
        while w > 1 || h > 1 {
            canceled(cancel)?;
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let level = chain.levels.len() - 1;
            let mut data = zeroes(nw * nh * 4)?;
            data.par_chunks_mut(nw * 4).enumerate().try_for_each(
                |(y, row)| -> Result<(), FillError> {
                    canceled(cancel)?;
                    let (y0, ny) = span(y, h, nh);
                    for x in 0..nw {
                        let (x0, nx) = span(x, w, nw);
                        let mut acc = Acc::default();
                        let weight = 1. / (nx * ny) as f64;
                        for v in 0..ny {
                            for u in 0..nx {
                                acc.add(weight, chain.read(level, x0 + u, y0 + v));
                            }
                        }
                        row[x * 4..x * 4 + 4]
                            .copy_from_slice(&acc.resolve(Rgba8::TRANSPARENT).to_array());
                    }
                    Ok(())
                },
            )?;
            chain.levels.push((nw, nh, data));
            w = nw;
            h = nh;
        }
        canceled(cancel)?;
        Ok(chain)
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }
    pub fn size(&self, level: usize) -> Option<(u32, u32)> {
        self.levels.get(level).map(|p| (p.0 as u32, p.1 as u32))
    }
    pub(crate) fn wh(&self, k: usize) -> (usize, usize) {
        (self.levels[k].0, self.levels[k].1)
    }
    pub(crate) fn read(&self, k: usize, x: usize, y: usize) -> Rgba8 {
        let d = if k == 0 {
            self.original.bytes()
        } else {
            &self.levels[k].2
        };
        let o = (y * self.levels[k].0 + x) * 4;
        let (mut r, mut g, mut b) = (d[o], d[o + 1], d[o + 2]);
        if k == 0 {
            if let Some(lut) = &self.lut {
                r = lut[r as usize];
                g = lut[g as usize];
                b = lut[b as usize];
            }
            if self.luminance {
                r = ((2126 * r as u32 + 7152 * g as u32 + 722 * b as u32 + 5000) / 10000) as u8;
                g = r;
                b = r;
            }
        }
        Rgba8::new(r, g, b, d[o + 3])
    }
}
fn span(i: usize, n: usize, half: usize) -> (usize, usize) {
    if n == 1 {
        (0, 1)
    } else {
        (i * 2, if i == half - 1 && n % 2 == 1 { 3 } else { 2 })
    }
}
#[derive(Default)]
pub(crate) struct Acc {
    a: f64,
    r: f64,
    g: f64,
    b: f64,
    zw: f64,
    zr: f64,
    zg: f64,
    zb: f64,
    count: u32,
    first: Rgba8,
    same: bool,
}
impl Acc {
    pub fn add(&mut self, w: f64, c: Rgba8) {
        if self.count == 0 {
            self.first = c;
            self.same = true;
        } else if c != self.first {
            self.same = false;
        }
        self.count += 1;
        if c.a == 0 {
            self.zw += w;
            self.zr += w * c.r as f64;
            self.zg += w * c.g as f64;
            self.zb += w * c.b as f64;
            return;
        }
        let k = w * c.a as f64;
        self.a += k;
        self.r += k * c.r as f64;
        self.g += k * c.g as f64;
        self.b += k * c.b as f64;
    }
    fn clear(&self) -> Rgba8 {
        if self.zw > 0. {
            Rgba8::new(
                to_byte(self.zr / self.zw / 255.),
                to_byte(self.zg / self.zw / 255.),
                to_byte(self.zb / self.zw / 255.),
                0,
            )
        } else {
            Rgba8::TRANSPARENT
        }
    }
    pub fn alpha(&self) -> f64 {
        if self.count == 0 {
            0.
        } else if self.same {
            self.first.a as f64
        } else {
            self.a
        }
    }
    pub fn resolve(&self, fallback: Rgba8) -> Rgba8 {
        if self.count == 0 {
            return fallback;
        }
        if self.same {
            return self.first;
        }
        let a = to_byte(self.a / 255.);
        if a == 0 {
            return self.clear();
        }
        Rgba8::new(
            to_byte(self.r / self.a / 255.),
            to_byte(self.g / self.a / 255.),
            to_byte(self.b / self.a / 255.),
            a,
        )
    }
    pub fn scaled(&self, scale: f64) -> Rgba8 {
        if self.count == 0 {
            return Rgba8::TRANSPARENT;
        }
        if self.same {
            return Rgba8::new(
                self.first.r,
                self.first.g,
                self.first.b,
                to_byte(self.first.a as f64 / 255. * scale),
            );
        }
        if self.a <= 0. {
            return self.clear();
        }
        Rgba8::new(
            to_byte(self.r / self.a / 255.),
            to_byte(self.g / self.a / 255.),
            to_byte(self.b / self.a / 255.),
            to_byte(self.a / 255. * scale),
        )
    }
}
