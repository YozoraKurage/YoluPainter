//! 独立した色・不透明度の分岐点、中点、PCHIP の値カーブ。
use super::{unit, Error};
use crate::{
    math::{clamp01, to_byte},
    Rgba8,
};

/// 色の分岐点。`color` の α は評価に使わず、`Ramp::new` が C# の `GradientStop` と同じく 255 にそろえる
/// （不透明度は独立した `OpacityStop` が持つ）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorStop {
    pub position: f64,
    pub color: Rgba8,
    pub midpoint: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpacityStop {
    pub position: f64,
    pub opacity: f64,
    pub midpoint: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurvePoint {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    colors: Vec<ColorStop>,
    opacities: Vec<OpacityStop>,
    curve: Vec<CurvePoint>,
    tangents: Vec<f64>,
}
impl Ramp {
    /// 履歴に積む大きさ（C# の `GradientRamp.ByteSize`: 64 + 色・不透明度・カーブの点の数 × 24）。
    pub fn byte_size(&self) -> u64 {
        64 + 24 * (self.colors.len() + self.opacities.len() + self.curve.len()) as u64
    }
}
impl Default for Ramp {
    fn default() -> Self {
        Self::new(
            vec![
                ColorStop {
                    position: 0.,
                    color: Rgba8::new(0, 0, 0, 255),
                    midpoint: 0.5,
                },
                ColorStop {
                    position: 1.,
                    color: Rgba8::new(255, 255, 255, 255),
                    midpoint: 0.5,
                },
            ],
            vec![
                OpacityStop {
                    position: 0.,
                    opacity: 1.,
                    midpoint: 0.5,
                },
                OpacityStop {
                    position: 1.,
                    opacity: 1.,
                    midpoint: 0.5,
                },
            ],
            None,
        )
        .unwrap()
    }
}
impl Ramp {
    pub fn new(
        mut colors: Vec<ColorStop>,
        opacities: Vec<OpacityStop>,
        curve: Option<Vec<CurvePoint>>,
    ) -> Result<Self, Error> {
        let curve =
            curve.unwrap_or_else(|| vec![CurvePoint { x: 0., y: 0. }, CurvePoint { x: 1., y: 1. }]);
        fn positions(p: impl Iterator<Item = f64>, gap: f64) -> bool {
            let mut last = None;
            for x in p {
                if !unit(x) || last.is_some_and(|v| x - v < gap) {
                    return false;
                }
                last = Some(x);
            }
            true
        }
        if !(2..=32).contains(&colors.len())
            || !(2..=32).contains(&opacities.len())
            || !(2..=16).contains(&curve.len())
            || !positions(colors.iter().map(|s| s.position), 0.0001)
            || !positions(opacities.iter().map(|s| s.position), 0.0001)
            || !positions(curve.iter().map(|p| p.x), 0.02 - 1e-6)
            || curve[0].x != 0.
            || curve[curve.len() - 1].x != 1.
            || colors
                .iter()
                .any(|s| !s.midpoint.is_finite() || !(0.01..=0.99).contains(&s.midpoint))
            || opacities.iter().any(|s| {
                !unit(s.opacity) || !s.midpoint.is_finite() || !(0.01..=0.99).contains(&s.midpoint)
            })
            || curve.iter().any(|p| !unit(p.y))
        {
            return Err(Error::Invalid("ランプの点・中点・カーブが範囲外です"));
        }
        for s in &mut colors {
            s.color.a = 255;
        }
        let n = curve.len();
        let mut m = vec![0.; n];
        let mut h = vec![0.; n - 1];
        let mut d = vec![0.; n - 1];
        for k in 0..n - 1 {
            h[k] = curve[k + 1].x - curve[k].x;
            d[k] = (curve[k + 1].y - curve[k].y) / h[k];
        }
        if n == 2 {
            m[0] = d[0];
            m[1] = d[0];
        } else {
            for k in 1..n - 1 {
                if d[k - 1] * d[k] > 0. {
                    let w1 = 2. * h[k] + h[k - 1];
                    let w2 = h[k] + 2. * h[k - 1];
                    m[k] = (w1 + w2) / (w1 / d[k - 1] + w2 / d[k]);
                }
            }
            fn edge(h0: f64, h1: f64, d0: f64, d1: f64) -> f64 {
                let m = ((2. * h0 + h1) * d0 - h0 * d1) / (h0 + h1);
                fn sign(x: f64) -> i32 {
                    if x > 0. {
                        1
                    } else if x < 0. {
                        -1
                    } else {
                        0
                    }
                }
                if sign(m) != sign(d0) {
                    0.
                } else if sign(d0) != sign(d1) && m.abs() > (3. * d0).abs() {
                    3. * d0
                } else {
                    m
                }
            }
            m[0] = edge(h[0], h[1], d[0], d[1]);
            m[n - 1] = edge(h[n - 2], h[n - 3], d[n - 2], d[n - 3]);
        }
        Ok(Self {
            colors,
            opacities,
            curve,
            tangents: m,
        })
    }
    pub fn colors(&self) -> &[ColorStop] {
        &self.colors
    }
    pub fn opacities(&self) -> &[OpacityStop] {
        &self.opacities
    }
    pub fn curve(&self) -> &[CurvePoint] {
        &self.curve
    }
    pub fn curve_value(&self, input: f64) -> Result<f64, Error> {
        if !input.is_finite() {
            return Err(Error::Invalid("ランプの入力は有限値が必要です"));
        }
        Ok(self.curve_value_unchecked(input))
    }
    fn curve_value_unchecked(&self, input: f64) -> f64 {
        let x = clamp01(input);
        let mut k = 1;
        while k < self.curve.len() - 1 && self.curve[k].x < x {
            k += 1;
        }
        let a = self.curve[k - 1];
        let b = self.curve[k];
        let h = b.x - a.x;
        let t = (x - a.x) / h;
        let t2 = t * t;
        let t3 = t2 * t;
        clamp01(
            (2. * t3 - 3. * t2 + 1.) * a.y
                + (t3 - 2. * t2 + t) * h * self.tangents[k - 1]
                + (-2. * t3 + 3. * t2) * b.y
                + (t3 - t2) * h * self.tangents[k],
        )
    }
    pub fn evaluate(&self, input: f64, scalar: bool) -> Result<Rgba8, Error> {
        self.sample_stops(self.curve_value(input)?, scalar)
    }
    pub(super) fn evaluate_unchecked(&self, input: f64, scalar: bool) -> Rgba8 {
        self.sample_unchecked(self.curve_value_unchecked(input), scalar)
    }
    pub fn sample_stops(&self, x: f64, scalar: bool) -> Result<Rgba8, Error> {
        if !x.is_finite() {
            return Err(Error::Invalid("ランプの入力は有限値が必要です"));
        }
        Ok(self.sample_unchecked(x, scalar))
    }
    fn sample_unchecked(&self, x: f64, scalar: bool) -> Rgba8 {
        let x = clamp01(x);
        let mut c = 1;
        let mut a = 1;
        while c < self.colors.len() - 1 && self.colors[c].position < x {
            c += 1;
        }
        while a < self.opacities.len() - 1 && self.opacities[a].position < x {
            a += 1;
        }
        let c0 = self.colors[c - 1];
        let c1 = self.colors[c];
        let a0 = self.opacities[a - 1];
        let a1 = self.opacities[a];
        fn weight(x: f64, a: f64, b: f64, m: f64) -> f64 {
            let t = clamp01((x - a) / (b - a));
            if t <= m {
                0.5 * t / m
            } else {
                0.5 + 0.5 * (t - m) / (1. - m)
            }
        }
        let t = weight(x, c0.position, c1.position, c0.midpoint);
        let u = weight(x, a0.position, a1.position, a0.midpoint);
        let lerp = |a: u8, b: u8| a as f64 + (b as f64 - a as f64) * t;
        let mut r = lerp(c0.color.r, c1.color.r);
        let mut g = lerp(c0.color.g, c1.color.g);
        let mut b = lerp(c0.color.b, c1.color.b);
        if scalar {
            r = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            g = r;
            b = r;
        }
        Rgba8::new(
            (r + 0.5).floor() as u8,
            (g + 0.5).floor() as u8,
            (b + 0.5).floor() as u8,
            to_byte(a0.opacity + (a1.opacity - a0.opacity) * u),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    BlackWhite,
    WhiteBlack,
    ForegroundBackground,
    ForegroundTransparent,
    WarmCool,
}
impl Ramp {
    pub fn preset(preset: Preset, foreground: Rgba8, background: Rgba8) -> Self {
        let (first, last) = match preset {
            Preset::BlackWhite => return Self::default(),
            Preset::WhiteBlack => (Rgba8::new(255, 255, 255, 255), Rgba8::new(0, 0, 0, 255)),
            Preset::ForegroundBackground => (foreground, background),
            Preset::ForegroundTransparent => (foreground, foreground),
            Preset::WarmCool => (Rgba8::new(255, 110, 40, 255), Rgba8::new(40, 110, 255, 255)),
        };
        Self::new(
            vec![
                ColorStop {
                    position: 0.,
                    color: first,
                    midpoint: 0.5,
                },
                ColorStop {
                    position: 1.,
                    color: last,
                    midpoint: 0.5,
                },
            ],
            vec![
                OpacityStop {
                    position: 0.,
                    opacity: 1.,
                    midpoint: 0.5,
                },
                OpacityStop {
                    position: 1.,
                    opacity: if preset == Preset::ForegroundTransparent {
                        0.
                    } else {
                        1.
                    },
                    midpoint: 0.5,
                },
            ],
            None,
        )
        .unwrap()
    }
}
