//! 塗りつぶしのチャンネルの源にしたベイクしたマップ（16 bit の成分と覆い）を、塗りつぶしの画素（straight RGBA8）として読む口。
//!
//! マップの値はデータ（法線・位置・AO などの線形の値）なので、色の変換をしない（カラーのチャンネルに置いても sRGB へ直さない）。
//! 成分が 1 つのマップは灰色、3 つのマップは RGB。スカラーのチャンネルで読むときは、3 つの成分を画像と同じ輝度の式で 1 つにする。
//! 16 bit → 8 bit は `v / 257` を丸める（0 → 0、65535 → 255。整数の値で、ちょうど半分になる値は無い）。
use super::{canceled, dimensions, zeroes, FillError};
use crate::Rgba8;
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;

/// ベイクしたマップ 1 枚（左下原点・行優先。成分は画素ごとに `channels` 個）。
#[derive(Clone, Copy)]
pub struct BakedSource<'a> {
    width: u32,
    height: u32,
    values: &'a [u16],
    coverage: &'a [u8],
    channels: usize,
    /// 3 つの成分を輝度で 1 つにするか（スカラーのチャンネル）。
    luminance: bool,
}

impl<'a> BakedSource<'a> {
    /// 長さを確かめて作る。成分の数は 1 か 3。
    pub fn new(
        width: u32,
        height: u32,
        values: &'a [u16],
        coverage: &'a [u8],
        channels: usize,
        luminance: bool,
    ) -> Result<Self, FillError> {
        let n = dimensions(width, height)?;
        if !matches!(channels, 1 | 3) || values.len() != n * channels || coverage.len() != n {
            return Err(FillError::Invalid("マップの要素数"));
        }
        Ok(BakedSource {
            width,
            height,
            values,
            coverage,
            channels,
            luminance,
        })
    }

    /// 文書（`doc_width` × `doc_height`）の画素 (x, y) の値。UV で置き方が既定（繰り返し 1・ずらし 0・回転 0）の塗りつぶしが読む形:
    /// 大きさが文書と同じなら画素を直に、違えば u16 のまま画素の中心で双線形に引き（端は端の画素。覆いのある画素だけで重みを割る）、
    /// 最後に 8 bit へ丸める。覆いのある画素に当たらなければ None（呼び手がチャンネルの値を見せる）。アルファは 255。
    pub fn sample(&self, x: u32, y: u32, doc_width: u32, doc_height: u32) -> Option<Rgba8> {
        if (self.width, self.height) == (doc_width, doc_height) {
            let i = y as usize * self.width as usize + x as usize;
            if self.coverage.get(i).copied().unwrap_or(0) == 0 {
                return None;
            }
            let at = i * self.channels;
            let rgb = if self.channels == 3 {
                [
                    byte(self.values[at]),
                    byte(self.values[at + 1]),
                    byte(self.values[at + 2]),
                ]
            } else {
                [byte(self.values[at]); 3]
            };
            return Some(self.finish(rgb));
        }
        let fx = (f64::from(x) + 0.5) * f64::from(self.width) / f64::from(doc_width) - 0.5;
        let fy = (f64::from(y) + 0.5) * f64::from(self.height) / f64::from(doc_height) - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (mut sum, mut total) = ([0.0f64; 3], 0.0f64);
        for (dy, wy) in [(0i64, 1.0 - ty), (1, ty)] {
            for (dx, wx) in [(0i64, 1.0 - tx), (1, tx)] {
                let w = wx * wy;
                if w <= 0.0 {
                    continue;
                }
                let sx = (x0 as i64 + dx).clamp(0, i64::from(self.width) - 1) as usize;
                let sy = (y0 as i64 + dy).clamp(0, i64::from(self.height) - 1) as usize;
                let i = sy * self.width as usize + sx;
                if self.coverage[i] == 0 {
                    continue;
                }
                let at = i * self.channels;
                for (k, s) in sum.iter_mut().enumerate() {
                    *s += w * f64::from(self.values[at + k.min(self.channels - 1)]);
                }
                total += w;
            }
        }
        if total <= 0.0 {
            return None;
        }
        Some(self.finish(sum.map(|s| (s / total / 257.0).round().clamp(0.0, 255.0) as u8)))
    }

    /// マップの大きさの straight RGBA8（覆いの無い画素は透明の 0）。UV 以外の投影・置き方を変えた UV が、画像と同じ置き方で引く
    /// ミップマップの元。行ごとに取消を見る。
    pub fn rgba8(&self, cancel: Option<&AtomicBool>) -> Result<Vec<u8>, FillError> {
        let n = dimensions(self.width, self.height)?;
        let mut out: Vec<u8> = zeroes(n * 4)?;
        let w = self.width as usize;
        out.par_chunks_mut(w * 4).enumerate().try_for_each(
            |(y, row)| -> Result<(), FillError> {
                canceled(cancel)?;
                for (x, p) in row.chunks_exact_mut(4).enumerate() {
                    let i = y * w + x;
                    if self.coverage[i] == 0 {
                        continue;
                    }
                    let at = i * self.channels;
                    let rgb = if self.channels == 3 {
                        [
                            byte(self.values[at]),
                            byte(self.values[at + 1]),
                            byte(self.values[at + 2]),
                        ]
                    } else {
                        [byte(self.values[at]); 3]
                    };
                    // 輝度はミップマップの段 0 を読むときに掛ける（画像と同じ）。ここでは成分のまま
                    p.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
                }
                Ok(())
            },
        )?;
        Ok(out)
    }

    fn finish(&self, [r, g, b]: [u8; 3]) -> Rgba8 {
        if self.luminance && self.channels == 3 {
            // ミップマップの段 0 の輝度と同じ整数の式（`ImageMipChain::read`）
            let l = ((2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b) + 5000)
                / 10000) as u8;
            Rgba8::new(l, l, l, 255)
        } else {
            Rgba8::new(r, g, b, 255)
        }
    }
}

/// 16 bit の値を 8 bit へ（`v / 257` を丸める）。
pub(crate) fn byte(v: u16) -> u8 {
    ((u32::from(v) + 128) / 257) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_bits_round_to_the_nearest_byte() {
        assert_eq!(byte(0), 0);
        assert_eq!(byte(65535), 255);
        assert_eq!(byte(128), 0);
        assert_eq!(byte(129), 1);
        for v in 0..=255u16 {
            assert_eq!(byte(v * 257), v as u8);
        }
    }

    #[test]
    fn a_resampled_map_divides_only_by_the_covered_weight() {
        // 2 × 1 のマップの右だけ覆いがある: 4 × 1 の文書の、右の画素に届く画素は右の値（左の 0 を混ぜない）。左の画素にしか
        // 届かない左端は値が無い
        let values = [0u16, 65535];
        let coverage = [0u8, 1];
        let s = BakedSource::new(2, 1, &values, &coverage, 1, false).unwrap();
        assert_eq!(s.sample(0, 0, 4, 1), None);
        for x in 1..4 {
            assert_eq!(s.sample(x, 0, 4, 1), Some(Rgba8::new(255, 255, 255, 255)));
        }
        let none = [0u8, 0];
        let s = BakedSource::new(2, 1, &values, &none, 1, false).unwrap();
        assert_eq!(s.sample(1, 0, 4, 1), None);
    }
}
