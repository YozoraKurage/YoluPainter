//! ダイナミクスの式（C# の BrushDynamics.cs）: デュアルブラシの合わせ方、ペンの傾き、色の変化（HSV）、フェード。

use super::random::NetRandom;
use super::settings::{ColorDynamics, DualBrushMode};
use crate::math::{clamp01, to_byte};
use crate::types::Rgba8;

impl DualBrushMode {
    /// 主の被覆率 main（0〜1）と 2 つ目の溜まり dual（0〜1）を合わせた被覆率（0〜1）。main = 0 なら必ず 0（C# の DualBrush.Combine）。
    #[inline]
    pub fn combine(self, main: f64, dual: f64) -> f64 {
        if main <= 0.0 {
            return 0.0;
        }
        let r = match self {
            DualBrushMode::Multiply => main * dual,
            DualBrushMode::Darken => f64_min(main, dual),
            DualBrushMode::Overlay => {
                if main < 0.5 {
                    2.0 * main * dual
                } else {
                    1.0 - 2.0 * (1.0 - main) * (1.0 - dual)
                }
            }
            DualBrushMode::ColorDodge => {
                if dual >= 1.0 {
                    1.0
                } else {
                    f64_min(1.0, main / (1.0 - dual))
                }
            }
            DualBrushMode::ColorBurn => {
                if main >= 1.0 {
                    1.0
                } else if dual <= 0.0 {
                    0.0
                } else {
                    1.0 - f64_min(1.0, (1.0 - main) / dual)
                }
            }
            DualBrushMode::LinearBurn => main + dual - 1.0,
            DualBrushMode::HardMix => {
                if main + dual >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            DualBrushMode::Subtract => main - dual,
        };
        clamp01(r)
    }
}

/// ペンの傾き（C# の PenTilt）。傾きは直立からの角度（ラジアン）をキャンバスの X 軸・Y 軸の向きに分けたもの。
/// 2 つを合わせた直立からの傾きは tan²θ = tan²θx + tan²θy。
pub mod pen_tilt {
    /// 傾きの上限（π/2）。
    pub const MAX_ANGLE: f64 = std::f64::consts::FRAC_PI_2;

    /// 直立 0 〜 寝かせきって 1。傾きの情報が無い入力（マウス）は 0。
    pub fn amount(tilt_x: f64, tilt_y: f64) -> f64 {
        let (ax, ay) = (tilt_x.abs(), tilt_y.abs());
        if ax <= 0.0 && ay <= 0.0 {
            return 0.0;
        }
        if ax >= MAX_ANGLE - 1e-9 || ay >= MAX_ANGLE - 1e-9 {
            return 1.0;
        }
        let (tx, ty) = (ax.tan(), ay.tan());
        let v = (tx * tx + ty * ty).sqrt().atan() / MAX_ANGLE;
        if 1.0 < v {
            1.0
        } else {
            v
        }
    }

    /// ペンが倒れている向き（ラジアン、キャンバスの X 軸から反時計回り）。傾きが無ければ 0。
    pub fn azimuth(tilt_x: f64, tilt_y: f64) -> f64 {
        if tilt_x == 0.0 && tilt_y == 0.0 {
            return 0.0;
        }
        let limit = |v: f64| {
            let hi = if MAX_ANGLE - 1e-9 < v {
                MAX_ANGLE - 1e-9
            } else {
                v
            };
            if -MAX_ANGLE + 1e-9 > hi {
                -MAX_ANGLE + 1e-9
            } else {
                hi
            }
        };
        limit(tilt_y).tan().atan2(limit(tilt_x).tan())
    }
}

impl ColorDynamics {
    /// 色を揺らす項目があるか（C# の HasColorDynamics）。
    pub fn is_active(&self) -> bool {
        self.foreground_background > 0.0
            || self.hue > 0.0
            || self.saturation > 0.0
            || self.brightness > 0.0
            || self.purity != 0.0
    }

    /// 次のダブ（またはストローク）の色（C# の ColorDynamics.Next）。色は保存された値のまま（エンコードされた空間）HSV で動かす。
    /// 決まった順: 描画色/背景色 → 純度 → 色相 → 彩度 → 明るさ。0 の項目は乱数を引かない。アルファは描画色と背景色の間で混ぜるだけ。
    pub fn next(&self, color: Rgba8, random: &mut NetRandom) -> Rgba8 {
        let mut c = color;
        if self.foreground_background > 0.0 {
            let t = self.foreground_background * random.next_double();
            let b = self.secondary;
            c = Rgba8::new(
                mix(c.r, b.r, t),
                mix(c.g, b.g, t),
                mix(c.b, b.b, t),
                mix(c.a, b.a, t),
            );
        }
        if self.purity == 0.0 && self.hue <= 0.0 && self.saturation <= 0.0 && self.brightness <= 0.0
        {
            return c;
        }
        let (mut h, mut sat, mut v) =
            rgb_to_hsv(c.r as f64 / 255.0, c.g as f64 / 255.0, c.b as f64 / 255.0);
        if self.purity > 0.0 {
            sat += (1.0 - sat) * self.purity;
        } else if self.purity < 0.0 {
            sat *= 1.0 + self.purity;
        }
        if self.hue > 0.0 {
            h += self.hue * (random.next_double() * 2.0 - 1.0) * 0.5;
            h -= h.floor();
        }
        if self.saturation > 0.0 {
            sat += self.saturation * (random.next_double() * 2.0 - 1.0);
        }
        if self.brightness > 0.0 {
            v += self.brightness * (random.next_double() * 2.0 - 1.0);
        }
        let (r, g, b) = hsv_to_rgb(h, clamp01(sat), clamp01(v));
        Rgba8::new(to_byte(r), to_byte(g), to_byte(b), c.a)
    }
}

#[inline]
fn mix(a: u8, b: u8, t: f64) -> u8 {
    to_byte((a as f64 + (b as i32 - a as i32) as f64 * t) / 255.0)
}

/// RGB（0〜1）から HSV（h は 0〜1 で 1 周。灰色は h = 0, s = 0）。教科書の式（C# の ColorDynamics.RgbToHsv）。
pub fn rgb_to_hsv(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = f64_max(r, f64_max(g, b));
    let min = f64_min(r, f64_min(g, b));
    let d = max - min;
    let v = max;
    let s = if max <= 0.0 { 0.0 } else { d / max };
    let mut h = 0.0;
    if d <= 0.0 {
        return (h, s, v);
    }
    if max == r {
        h = (g - b) / d;
    } else if max == g {
        h = 2.0 + (b - r) / d;
    } else {
        h = 4.0 + (r - g) / d;
    }
    h /= 6.0;
    if h < 0.0 {
        h += 1.0;
    }
    (h, s, v)
}

/// HSV から RGB（C# の ColorDynamics.HsvToRgb）。
pub fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (f64, f64, f64) {
    let h = (h - h.floor()) * 6.0;
    let i = (h.floor() as i32) % 6;
    let f = h - h.floor();
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

/// フェードの係数: 長さ length（描点の数）にわたって 1 から 0 へ線形に下がる。index は 0 から。0 は切（C# の BrushStroke.Fade）。
#[inline]
pub(crate) fn fade(length: u32, index: u64) -> f64 {
    if length == 0 {
        return 1.0;
    }
    let v = 1.0 - index as f64 / length as f64;
    if 0.0 > v {
        0.0
    } else {
        v
    }
}

/// C# の Math.Min（有限の値だけが来る）。
#[inline(always)]
pub(crate) fn f64_min(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}

/// C# の Math.Max（有限の値だけが来る）。
#[inline(always)]
pub(crate) fn f64_max(a: f64, b: f64) -> f64 {
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
    fn hsv_conversion_is_the_textbook_one() {
        // C# BrushDynamicsTests.HsvConversionIsTheTextbookOne
        assert_eq!(rgb_to_hsv(1.0, 0.0, 0.0), (0.0, 1.0, 1.0));
        let (h, s, v) = rgb_to_hsv(0.0, 1.0, 0.0);
        assert!((h - 1.0 / 3.0).abs() < 1e-12 && s == 1.0 && v == 1.0);
        assert_eq!(rgb_to_hsv(0.5, 0.5, 0.5), (0.0, 0.0, 0.5));
        for (r, g, b) in [(0.2, 0.4, 0.9), (0.9, 0.1, 0.5), (0.3, 0.3, 0.1)] {
            let (h, s, v) = rgb_to_hsv(r, g, b);
            let (r2, g2, b2) = hsv_to_rgb(h, s, v);
            assert!((r - r2).abs() < 1e-12 && (g - g2).abs() < 1e-12 && (b - b2).abs() < 1e-12);
        }
    }

    #[test]
    fn dual_modes_combine_coverage() {
        // C# BrushDynamicsTests.DualModesCombineCoverage
        use DualBrushMode::*;
        for m in DualBrushMode::ALL {
            assert_eq!(m.combine(0.0, 0.7), 0.0, "{m:?}");
        }
        assert_eq!(Multiply.combine(0.5, 0.5), 0.25);
        assert_eq!(Darken.combine(0.5, 0.25), 0.25);
        assert_eq!(Overlay.combine(0.25, 0.5), 0.25);
        assert_eq!(Overlay.combine(0.75, 0.5), 0.75);
        assert_eq!(ColorDodge.combine(0.25, 0.5), 0.5);
        assert_eq!(ColorDodge.combine(0.25, 1.0), 1.0);
        assert_eq!(ColorBurn.combine(0.5, 0.0), 0.0);
        assert_eq!(ColorBurn.combine(0.75, 0.5), 0.5);
        assert_eq!(LinearBurn.combine(0.5, 0.25), 0.0);
        assert_eq!(HardMix.combine(0.5, 0.5), 1.0);
        assert_eq!(HardMix.combine(0.5, 0.25), 0.0);
        assert_eq!(Subtract.combine(0.75, 0.25), 0.5);
    }

    #[test]
    fn pen_tilt_is_measured_from_upright() {
        // C# BrushDynamicsTests.PenTiltIsMeasuredFromUpright
        assert_eq!(pen_tilt::amount(0.0, 0.0), 0.0);
        assert_eq!(pen_tilt::amount(pen_tilt::MAX_ANGLE, 0.0), 1.0);
        assert!((pen_tilt::amount(std::f64::consts::FRAC_PI_4, 0.0) - 0.5).abs() < 1e-12);
        assert!((pen_tilt::amount(0.0, -std::f64::consts::FRAC_PI_4) - 0.5).abs() < 1e-12);
        assert_eq!(pen_tilt::azimuth(0.0, 0.0), 0.0);
        assert!((pen_tilt::azimuth(0.3, 0.3) - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
        assert!((pen_tilt::azimuth(0.0, 0.4) - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    }

    #[test]
    fn fade_falls_linearly_to_zero() {
        assert_eq!(fade(0, 99), 1.0);
        assert_eq!(fade(4, 0), 1.0);
        assert_eq!(fade(4, 2), 0.5);
        assert_eq!(fade(4, 9), 0.0);
    }
}
