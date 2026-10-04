//! 筆先の画像（C# の BrushTip）と、組み込みの筆先・紙の質感（C# の BuiltInBrushes の生成）。
//!
//! 筆先は 1 画素 1 バイトの覆い（0〜255）で、行優先・一番下の行が先（画布と同じ左下原点）。上の行から並ぶ形式（PNG・GIMP・ABR・
//! CLIP STUDIO）を読む側は行を反転して渡す。組み込みの筆先は決まった雑音から作るので、第三者の絵もライセンスも入らない。

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use super::random::NetRandom;
use crate::error::CoreError;
use crate::math::{clamp01, to_byte};

/// 取り込んだ（または組み込みの）筆先の画像。作った後は変わらない（ストロークはワーカーからも読む）。
#[derive(Clone, PartialEq, Eq)]
pub struct BrushTip {
    name: String,
    width: u32,
    height: u32,
    alpha: Vec<u8>,
}

impl std::fmt::Debug for BrushTip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "BrushTip({:?}, {}×{})",
            self.name, self.width, self.height
        )
    }
}

impl BrushTip {
    /// 1 辺の上限（C# の MaxSize）。
    pub const MAX_SIZE: u32 = 2048;

    /// 幅・高さ（1〜2048）と覆い（幅 × 高さ バイト、行は下から）。
    pub fn new(name: &str, width: u32, height: u32, alpha: Vec<u8>) -> Result<BrushTip, CoreError> {
        if width < 1 || height < 1 || width > Self::MAX_SIZE || height > Self::MAX_SIZE {
            return Err(CoreError::InvalidArgument("筆先の大きさ（1〜2048）"));
        }
        if alpha.len() != width as usize * height as usize {
            return Err(CoreError::InvalidArgument(
                "筆先の覆いの長さが幅 × 高さでない",
            ));
        }
        Ok(BrushTip {
            name: name.to_string(),
            width,
            height,
            alpha,
        })
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    /// 覆い（行は下から）。
    pub fn alpha(&self) -> &[u8] {
        &self.alpha
    }
    /// 画素 (x, y) の覆い（0〜255）。
    pub fn at(&self, x: u32, y: u32) -> u8 {
        self.alpha[(y * self.width + x) as usize]
    }

    /// 正規化した座標（0〜1）の覆い（0〜1、双線形）。[0,1]² の外は 0。画素の中心は (i + 0.5) / 大きさ。
    #[inline]
    pub fn sample(&self, u: f64, v: f64) -> f64 {
        if u < 0.0 || v < 0.0 || u > 1.0 || v > 1.0 {
            return 0.0;
        }
        self.bilinear(
            u * self.width as f64 - 0.5,
            v * self.height as f64 - 0.5,
            false,
        )
    }

    /// 無限に並べた筆先の、画素の座標での覆い（0〜1、双線形。紙の質感）。
    #[inline]
    pub fn sample_tiled(&self, x: f64, y: f64) -> f64 {
        self.sample_tiled_in(&self.tiled_row(y), x)
    }

    /// 並べた読みの y の側（行の添字と端数）。同じ行の画素で使い回す。
    #[inline]
    pub(crate) fn tiled_row(&self, y: f64) -> TiledRow {
        let y = y - 0.5;
        let y0 = y.floor() as i32;
        let h = self.height as i32;
        let wy0 = y0.rem_euclid(h);
        let wy1 = if wy0 + 1 == h { 0 } else { wy0 + 1 };
        TiledRow {
            row0: (wy0 as u32 * self.width) as usize,
            row1: (wy1 as u32 * self.width) as usize,
            fy: y - y0 as f64,
        }
    }

    /// 並べた読みの x の側（C# の Bilinear(x − 0.5, y − 0.5, wrap) と同じ 4 つの画素・同じ式。回り込みは軸ごとに剰余 1 回）。
    #[inline]
    pub(crate) fn sample_tiled_in(&self, r: &TiledRow, x: f64) -> f64 {
        let x = x - 0.5;
        let x0 = x.floor() as i32;
        let fx = x - x0 as f64;
        let w = self.width as i32;
        let wx0 = x0.rem_euclid(w);
        let wx1 = if wx0 + 1 == w { 0 } else { wx0 + 1 };
        let al = &self.alpha;
        let a = al[r.row0 + wx0 as usize] as f64;
        let b = al[r.row0 + wx1 as usize] as f64;
        let c = al[r.row1 + wx0 as usize] as f64;
        let d = al[r.row1 + wx1 as usize] as f64;
        let fy = r.fy;
        ((a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy) / 255.0
    }

    #[inline]
    fn bilinear(&self, x: f64, y: f64, wrap: bool) -> f64 {
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let fx = x - x0 as f64;
        let fy = y - y0 as f64;
        let a = self.texel(x0, y0, wrap);
        let b = self.texel(x0 + 1, y0, wrap);
        let c = self.texel(x0, y0 + 1, wrap);
        let d = self.texel(x0 + 1, y0 + 1, wrap);
        ((a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy) / 255.0
    }

    #[inline]
    fn texel(&self, mut x: i32, mut y: i32, wrap: bool) -> f64 {
        let (w, h) = (self.width as i32, self.height as i32);
        if wrap {
            x %= w;
            if x < 0 {
                x += w;
            }
            y %= h;
            if y < 0 {
                y += h;
            }
        } else if x < 0 || y < 0 || x >= w || y >= h {
            return 0.0;
        }
        self.alpha[(y * w + x) as usize] as f64
    }
}

/// 並べた読みの 1 行分（[`BrushTip::tiled_row`]）。
pub(crate) struct TiledRow {
    row0: usize,
    row1: usize,
    fy: f64,
}

/// 組み込みの筆先・紙の質感の名前（C# の BuiltInBrushes.TipIds と同じ）。
pub const BUILTIN_TIPS: [&str; 7] = [
    "grain",
    "noisy-disc",
    "charcoal",
    "bristles",
    "dots",
    "rim",
    "rounded-square",
];

/// 組み込みの筆先（"grain"・"noisy-disc"・"charcoal"・"bristles"・"dots"・"rim"・"rounded-square"）。知らない名前は None。
/// 初めて呼んだときに全部を 1 回だけ作る。生成の式を変えると保存したブラシの見た目が変わるので、変えるときは新しい名前にする。
pub fn builtin_tip(id: &str) -> Option<Arc<BrushTip>> {
    static TIPS: OnceLock<HashMap<&'static str, Arc<BrushTip>>> = OnceLock::new();
    TIPS.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("grain", Arc::new(grain(128, 7)));
        m.insert("noisy-disc", Arc::new(noisy_disc(128, 11)));
        m.insert("charcoal", Arc::new(charcoal(128, 80, 13)));
        m.insert("bristles", Arc::new(bristles(32, 128, 17)));
        m.insert("dots", Arc::new(dots(128, 19)));
        m.insert("rim", Arc::new(rim(128, 23)));
        m.insert("rounded-square", Arc::new(rounded_square(96)));
        m
    })
    .get(id)
    .cloned()
}

// ───── 決まった雑音（C# の BuiltInBrushes と同じ式・同じ演算の順） ─────

fn hash(x: i32, y: i32, seed: i32) -> f64 {
    let mut h = x
        .wrapping_mul(374761393)
        .wrapping_add(y.wrapping_mul(668265263))
        .wrapping_add(seed.wrapping_mul(1442695041)) as u32;
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^= h >> 16;
    (h & 0xFFFFFF) as f64 / 0x1000000 as f64
}

/// 周期 period の格子の値の雑音（並べて継ぎ目が無い）。smoothstep で補間、0〜1。
fn value_noise(x: f64, y: f64, period: i32, seed: i32) -> f64 {
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let mut fx = x - x0 as f64;
    let mut fy = y - y0 as f64;
    fx = fx * fx * (3.0 - 2.0 * fx);
    fy = fy * fy * (3.0 - 2.0 * fy);
    let wrap = |v: i32| {
        let v = v % period;
        if v < 0 {
            v + period
        } else {
            v
        }
    };
    let a = hash(wrap(x0), wrap(y0), seed);
    let b = hash(wrap(x0 + 1), wrap(y0), seed);
    let c = hash(wrap(x0), wrap(y0 + 1), seed);
    let d = hash(wrap(x0 + 1), wrap(y0 + 1), seed);
    (a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy
}

/// size 画素で継ぎ目の無いフラクタルの値の雑音。
fn fbm(px: i32, py: i32, size: i32, base_cells: i32, octaves: i32, seed: i32) -> f64 {
    let (mut sum, mut amplitude, mut total) = (0.0, 1.0, 0.0);
    let mut cells = base_cells;
    for o in 0..octaves {
        sum += amplitude
            * value_noise(
                (px as f64 + 0.5) * cells as f64 / size as f64,
                (py as f64 + 0.5) * cells as f64 / size as f64,
                cells,
                seed + o * 101,
            );
        total += amplitude;
        amplitude *= 0.5;
        cells *= 2;
    }
    sum / total
}

fn smooth(e0: f64, e1: f64, v: f64) -> f64 {
    let t = clamp01((v - e0) / (e1 - e0));
    t * t * (3.0 - 2.0 * t)
}

fn make(name: &str, w: i32, h: i32, f: impl Fn(i32, i32) -> f64) -> BrushTip {
    let mut a = vec![0u8; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            a[(y * w + x) as usize] = to_byte(clamp01(f(x, y)));
        }
    }
    BrushTip::new(name, w as u32, h as u32, a).expect("組み込みの筆先の大きさ")
}

fn radial(x: i32, y: i32, w: i32, h: i32) -> f64 {
    let u = (x as f64 + 0.5) / w as f64 * 2.0 - 1.0;
    let v = (y as f64 + 0.5) / h as f64 * 2.0 - 1.0;
    (u * u + v * v).sqrt()
}

/// libm の pow を必ず呼ぶ（LLVM が pow(x, 2) を x·x に置き換えると、C# の Math.Pow と最後のビットが違い得る）。
#[inline(never)]
fn pow(x: f64, y: f64) -> f64 {
    x.powf(std::hint::black_box(y))
}

fn grain(size: i32, seed: i32) -> BrushTip {
    make("grain", size, size, |x, y| {
        smooth(0.25, 0.75, fbm(x, y, size, 8, 4, seed))
    })
}

fn noisy_disc(size: i32, seed: i32) -> BrushTip {
    make("noisy-disc", size, size, |x, y| {
        (1.0 - smooth(0.8, 1.0, radial(x, y, size, size)))
            * smooth(0.3, 0.6, fbm(x, y, size, 6, 4, seed))
    })
}

fn charcoal(w: i32, h: i32, seed: i32) -> BrushTip {
    make("charcoal", w, h, |x, y| {
        (1.0 - smooth(0.7, 1.0, radial(x, y, w, h))) * smooth(0.25, 0.65, fbm(x, y, w, 10, 3, seed))
    })
}

fn bristles(w: i32, h: i32, seed: i32) -> BrushTip {
    // 筆先の高さ方向に並んだ毛の塊（向きに沿わせると、線に直交する）
    let mut a = vec![0.0f64; (w * h) as usize];
    let mut r = NetRandom::new(seed);
    for _ in 0..26 {
        let cy = r.next_double() * h as f64;
        let ry = 1.0 + r.next_double() * 3.5;
        let rx = w as f64 * (0.25 + 0.2 * r.next_double());
        let cx = w as f64 / 2.0 + (r.next_double() - 0.5) * w as f64 * 0.3;
        let strength = 0.45 + 0.55 * r.next_double();
        for y in 0..h {
            for x in 0..w {
                let dx = (x as f64 + 0.5 - cx) / rx;
                let dy = (y as f64 + 0.5 - cy) / ry;
                let d = dx * dx + dy * dy;
                if d < 1.0 {
                    let i = (y * w + x) as usize;
                    a[i] = f64_max(a[i], strength * (1.0 - d));
                }
            }
        }
    }
    make("bristles", w, h, |x, y| a[(y * w + x) as usize])
}

fn dots(size: i32, seed: i32) -> BrushTip {
    let mut a = vec![0.0f64; (size * size) as usize];
    let mut r = NetRandom::new(seed);
    for _ in 0..18 {
        let angle = r.next_double() * std::f64::consts::PI * 2.0;
        let dist = r.next_double().sqrt() * size as f64 * 0.4;
        let radius = 3.0 + r.next_double() * 7.0;
        let cx = size as f64 / 2.0 + angle.cos() * dist;
        let cy = size as f64 / 2.0 + angle.sin() * dist;
        for y in 0..size {
            for x in 0..size {
                let ex = x as f64 + 0.5 - cx;
                let ey = y as f64 + 0.5 - cy;
                let d = (ex * ex + ey * ey).sqrt() / radius;
                if d < 1.0 {
                    let i = (y * size + x) as usize;
                    a[i] = f64_max(a[i], 1.0 - smooth(0.7, 1.0, d));
                }
            }
        }
    }
    make("dots", size, size, |x, y| a[(y * size + x) as usize])
}

fn rim(size: i32, seed: i32) -> BrushTip {
    // 縁ほど濃い水たまり（乾きかけの水彩）
    make("rim", size, size, |x, y| {
        let d = radial(x, y, size, size);
        if d >= 1.0 {
            return 0.0;
        }
        let ring = 0.35 + 0.65 * (-pow((d - 0.86) / 0.07, 2.0)).exp();
        ring * (1.0 - smooth(0.92, 1.0, d)) * (0.85 + 0.15 * fbm(x, y, size, 8, 2, seed))
    })
}

fn rounded_square(size: i32) -> BrushTip {
    make("rounded-square", size, size, |x, y| {
        let u = ((x as f64 + 0.5) / size as f64 * 2.0 - 1.0).abs();
        let v = ((y as f64 + 0.5) / size as f64 * 2.0 - 1.0).abs();
        1.0 - smooth(0.9, 1.0, pow(pow(u, 6.0) + pow(v, 6.0), 1.0 / 6.0))
    })
}

/// C# の Math.Max（有限の値だけが来る）。
#[inline(always)]
fn f64_max(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tips_validate_their_input() {
        // C# BrushTests.BrushTipsValidateTheirInput
        assert!(BrushTip::new("a", 0, 4, vec![]).is_err());
        assert!(BrushTip::new("a", 2049, 1, vec![0; 2049]).is_err());
        assert!(BrushTip::new("a", 2, 2, vec![0; 3]).is_err());
        let t = BrushTip::new("a", 2, 1, vec![0, 255]).unwrap();
        assert_eq!(t.at(1, 0), 255);
    }

    #[test]
    fn sampling_is_bilinear_and_zero_outside() {
        let t = BrushTip::new("a", 2, 1, vec![0, 255]).unwrap();
        assert_eq!(t.sample(-0.01, 0.5), 0.0);
        assert_eq!(t.sample(0.75, 0.5), 1.0); // 画素 1 の中心は (0.75, 0.5)
        assert_eq!(t.sample(0.5, 0.5), 0.5); // 2 つの画素の中ほど
        assert_eq!(t.sample(0.75, 0.25), 0.75); // 行の中心から 1/4 だけ下（下の外は 0）
        assert_eq!(t.sample_tiled(1.5, 0.5), 1.0);
        assert_eq!(t.sample_tiled(3.5, 0.5), 1.0); // 繰り返す
        assert_eq!(t.sample_tiled(1.0, 0.5), 0.5);
    }

    #[test]
    fn every_builtin_tip_exists() {
        for id in BUILTIN_TIPS {
            let t = builtin_tip(id).unwrap();
            assert_eq!(t.name(), id);
            assert!(t.alpha().iter().any(|&a| a > 0), "{id}");
        }
        assert!(builtin_tip("nope").is_none());
    }
}
