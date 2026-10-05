//! 独立した色・不透明度の分岐点、中点、PCHIP の値カーブ（`crate::curve::Curve`）。
//!
//! 版 24 まではここまで。グラデーションマップ用に、色の混ぜ方（`MixMode`・輝度の補正）と、色の分岐点の区間ごとの混合率曲線を足した
//! （どれも既定は「なし」で、`Ramp::new` で作ったランプは昔どおりの評価になる）。評価の順は、入力 → 値のカーブ（ランプの位置）→
//! 区間を探す → 区間の重み（混合率曲線があればそれ、無ければ中点）→ 混色モードで色を混ぜる。
use super::mixing::{mix, LuminanceCorrection, MixMode};
use super::{unit, Error};
use crate::curve::{Curve, CurvePoint};
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
#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    colors: Vec<ColorStop>,
    opacities: Vec<OpacityStop>,
    curve: Curve,
    mix: MixMode,
    correction: LuminanceCorrection,
    /// 色の分岐点 k と k + 1 の間の混合率曲線。空なら全部「なし」、あれば数は色の分岐点の数 − 1（全部なしのときは空に正規化する）。
    segments: Vec<Option<Curve>>,
}
impl Ramp {
    /// 履歴に積む大きさ（C# の `GradientRamp.ByteSize`: 64 + 色・不透明度・カーブの点の数 × 24。混合率曲線があれば、その点の数 × 24 と 16 を足す）。
    pub fn byte_size(&self) -> u64 {
        64 + 24 * (self.colors.len() + self.opacities.len() + self.curve.points().len()) as u64
            + self
                .segments
                .iter()
                .flatten()
                .map(|c| 16 + 24 * c.points().len() as u64)
                .sum::<u64>()
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
        let curve = match curve {
            Some(points) => Curve::new(points),
            None => Ok(Curve::identity()),
        };
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
        let invalid = Error::Invalid("ランプの点・中点・カーブが範囲外です");
        let Ok(curve) = curve else {
            return Err(invalid);
        };
        if !(2..=32).contains(&colors.len())
            || !(2..=32).contains(&opacities.len())
            || !positions(colors.iter().map(|s| s.position), 0.0001)
            || !positions(opacities.iter().map(|s| s.position), 0.0001)
            || colors
                .iter()
                .any(|s| !s.midpoint.is_finite() || !(0.01..=0.99).contains(&s.midpoint))
            || opacities.iter().any(|s| {
                !unit(s.opacity) || !s.midpoint.is_finite() || !(0.01..=0.99).contains(&s.midpoint)
            })
        {
            return Err(invalid);
        }
        for s in &mut colors {
            s.color.a = 255;
        }
        Ok(Self {
            colors,
            opacities,
            curve,
            mix: MixMode::Standard,
            correction: LuminanceCorrection::default(),
            segments: Vec::new(),
        })
    }
    /// 値のカーブだけを差し替えた新しいランプ（カーブは検査済みなので失敗しない）。混色・混合率曲線は残す。
    pub fn with_value_curve(&self, curve: Curve) -> Self {
        Self {
            curve,
            ..self.clone()
        }
    }
    /// 色・不透明度の分岐点だけを差し替えた新しいランプ（値のカーブ・混色は残す）。混合率曲線は、色の分岐点の数が同じなら残し、
    /// 違えば（足した・消した）全部外す（どの区間の曲線かが決められないので。足す・消すの操作は `ui::ramp::ops` が区間を分けて・つないで渡す）。
    pub fn with_stops(
        &self,
        colors: Vec<ColorStop>,
        opacities: Vec<OpacityStop>,
    ) -> Result<Self, Error> {
        let keep = colors.len() == self.colors.len();
        let mut next = Self::new(colors, opacities, Some(self.curve.points().to_vec()))?;
        next.mix = self.mix;
        next.correction = self.correction;
        if keep {
            next.segments = self.segments.clone();
        }
        Ok(next)
    }
    /// 色の混ぜ方。
    pub fn mix_mode(&self) -> MixMode {
        self.mix
    }
    /// 輝度の補正（知覚的でなければ使わない。常に既定の値）。
    pub fn luminance_correction(&self) -> LuminanceCorrection {
        self.correction
    }
    /// 混色（モードと輝度の補正）だけを差し替えた新しいランプ。知覚的でないときの輝度の補正は使われないので、既定へそろえる
    /// （見えない値で、同じ見た目のランプが別の値にならないように）。
    pub fn with_mixing(&self, mode: MixMode, correction: LuminanceCorrection) -> Self {
        Self {
            mix: mode,
            correction: if mode == MixMode::Perceptual {
                correction
            } else {
                LuminanceCorrection::default()
            },
            ..self.clone()
        }
    }
    /// 色の分岐点 `k` と次の分岐点の間の混合率曲線（無ければ `None`。重みは中点で決まる）。
    pub fn segment_curve(&self, k: usize) -> Option<&Curve> {
        self.segments.get(k).and_then(Option::as_ref)
    }
    /// 全部の区間の混合率曲線（空か、色の分岐点の数 − 1 個）。
    pub fn segment_curves(&self) -> &[Option<Curve>] {
        &self.segments
    }
    /// 混合率曲線を全部差し替えた新しいランプ。数が 0 か色の分岐点の数 − 1 でなければエラー。全部なしなら空にそろえる。
    pub fn with_segment_curves(&self, segments: Vec<Option<Curve>>) -> Result<Self, Error> {
        if !segments.is_empty() && segments.len() != self.colors.len() - 1 {
            return Err(Error::Invalid("混合率曲線の数が区間の数と合いません"));
        }
        Ok(Self {
            segments: if segments.iter().all(Option::is_none) {
                Vec::new()
            } else {
                segments
            },
            ..self.clone()
        })
    }
    /// 区間 `k` の混合率曲線を差し替える（`None` で外す）。区間が無ければエラー。
    pub fn with_segment_curve(&self, k: usize, curve: Option<Curve>) -> Result<Self, Error> {
        if k + 1 >= self.colors.len() {
            return Err(Error::Invalid("混合率曲線の区間が範囲外です"));
        }
        let mut list = if self.segments.is_empty() {
            vec![None; self.colors.len() - 1]
        } else {
            self.segments.clone()
        };
        list[k] = curve;
        self.with_segment_curves(list)
    }
    /// 混色（Standard 以外）か混合率曲線を使っているか。使っていれば、版 24 までの形（と PSD のグラデーション）では表せない。
    pub fn uses_mixing(&self) -> bool {
        self.mix != MixMode::Standard || self.segments.iter().any(Option::is_some)
    }
    /// 混色と混合率曲線を外した新しいランプ（混色を持たない場所へ渡すとき）。
    pub fn without_mixing(&self) -> Self {
        Self {
            mix: MixMode::Standard,
            correction: LuminanceCorrection::default(),
            segments: Vec::new(),
            ..self.clone()
        }
    }
    /// 中点 `m`（区間の中で重みが 0.5 になる位置）と同じ向きの混合率曲線（混合率曲線を使い始めるときの出発点）。(0,0)・(m,0.5)・(1,1) を通る。
    /// 直線の中点（0.5）なら直線。曲線の点の間隔の下限（0.02）の外の中点は内側へ寄せる。
    pub fn curve_from_midpoint(midpoint: f64) -> Curve {
        if (midpoint - 0.5).abs() < 1e-9 || !midpoint.is_finite() {
            return Curve::identity();
        }
        let m = midpoint.clamp(0.03, 0.97);
        Curve::new(vec![
            CurvePoint { x: 0., y: 0. },
            CurvePoint { x: m, y: 0.5 },
            CurvePoint { x: 1., y: 1. },
        ])
        .unwrap_or_else(|_| Curve::identity())
    }
    /// 値のカーブ（`crate::curve::Curve`）。
    pub fn value_curve(&self) -> &Curve {
        &self.curve
    }
    pub fn colors(&self) -> &[ColorStop] {
        &self.colors
    }
    pub fn opacities(&self) -> &[OpacityStop] {
        &self.opacities
    }
    pub fn curve(&self) -> &[CurvePoint] {
        self.curve.points()
    }
    pub fn curve_value(&self, input: f64) -> Result<f64, Error> {
        if !input.is_finite() {
            return Err(Error::Invalid("ランプの入力は有限値が必要です"));
        }
        Ok(self.curve_value_unchecked(input))
    }
    fn curve_value_unchecked(&self, input: f64) -> f64 {
        self.curve.value_unchecked(input)
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
        let t = match self.segment_curve(c - 1) {
            Some(curve) => {
                curve.value_unchecked(clamp01((x - c0.position) / (c1.position - c0.position)))
            }
            None => weight(x, c0.position, c1.position, c0.midpoint),
        };
        let u = weight(x, a0.position, a1.position, a0.midpoint);
        let [mut r, mut g, mut b] = mix(c0.color, c1.color, t, self.mix, self.correction);
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
