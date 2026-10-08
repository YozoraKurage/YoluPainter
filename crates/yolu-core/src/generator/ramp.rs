//! 独立した色・不透明度の分岐点、中点、PCHIP の値カーブ（`crate::curve::Curve`）。
//!
//! 版 24 まではここまで。グラデーションマップ用に、色の混ぜ方（`MixMode`・輝度の補正）と、色の分岐点の区間ごとの混合率曲線を足した
//! （どれも既定は「なし」で、`Ramp::new` で作ったランプは昔どおりの評価になる）。評価の順は、入力 → 値のカーブ（ランプの位置）→
//! 区間を探す → 区間の重み（混合率曲線があればそれ、無ければ中点）→ 混色モードで色を混ぜる。
use super::mixing::{mix, LuminanceCorrection, MixMode, StopColor};
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use super::mixing::{mix_lanes, MixLanes, StopLanes};
use super::{unit, Error};
use crate::curve::{Curve, CurvePoint};
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use crate::math::simd::{self, Lanes};
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
    /// `colors` と同じ並びの、混色の式が読む形（線形の光・Oklab・彩度）。`colors` から決まる写しで、`new` だけが作る。
    stops: Vec<StopColor>,
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
        let stops = colors.iter().map(|s| StopColor::new(s.color)).collect();
        Ok(Self {
            colors,
            opacities,
            curve,
            mix: MixMode::Standard,
            correction: LuminanceCorrection::default(),
            segments: Vec::new(),
            stops,
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
        let [mut r, mut g, mut b] = mix(
            &self.stops[c - 1],
            &self.stops[c],
            t,
            self.mix,
            self.correction,
        );
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

/// 区間の重み（`sample_unchecked` の `weight` の N 画素ぶん）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn weight_lanes<V: Lanes>(x: V::F, a: V::F, b: V::F, m: V::F) -> V::F {
    let (half, one) = (V::splat(0.5), V::splat(1.));
    let t = simd::clamp01::<V>(V::div(V::sub(x, a), V::sub(b, a)));
    let low = V::div(V::mul(half, t), m);
    let high = V::add(half, V::div(V::mul(half, V::sub(t, m)), V::sub(one, m)));
    V::select(V::le(t, m), low, high)
}
/// `(v + 0.5).floor() as u8`（0〜255 に収めた整数の値）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn byte_lanes<V: Lanes>(v: V::F) -> V::F {
    V::min(
        V::max(V::floor(V::add(v, V::splat(0.5))), V::splat(0.)),
        V::splat(255.),
    )
}
impl Ramp {
    /// `evaluate_unchecked` の N 画素ぶん（結果は R・G・B・A のレーン。0〜255 の整数の値）。値のカーブ・区間の重み・色の混ぜ（通常・リニア・
    /// 知覚的）・不透明度は 1 画素の式と同じ演算を同じ順に並べたもので、バイトは同じ。区間の混合率曲線は、N 画素が同じ区間ならその曲線の
    /// レーンの式で、違う区間にまたがる組は 1 画素ずつの曲線の式で重みを出す（どちらも曲線の 1 画素の式と同じ値）。
    ///
    /// # Safety
    /// `V` の命令を持つ CPU で、その命令を有効にした `#[target_feature]` 付きの入口の中から呼ぶ。
    #[inline(always)]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    pub(super) unsafe fn evaluate_lanes<V: MixLanes>(
        &self,
        input: V::F,
        scalar: bool,
    ) -> [V::F; 4] {
        let x = self.curve.value_lanes::<V>(input);
        // 区間の番号: 位置が x より小さい分岐点の数 + 1（`sample_unchecked` の `while` と同じ）
        let (mut ci, mut ai) = (V::splat(1.), V::splat(1.));
        let (one, zero) = (V::splat(1.), V::splat(0.));
        for c in &self.colors[1..self.colors.len() - 1] {
            ci = V::add(ci, V::select(V::lt(V::splat(c.position), x), one, zero));
        }
        for a in &self.opacities[1..self.opacities.len() - 1] {
            ai = V::add(ai, V::select(V::lt(V::splat(a.position), x), one, zero));
        }
        let (mut cs, mut as_) = ([0.; 4], [0.; 4]);
        V::store_f64(&mut cs, ci);
        V::store_f64(&mut as_, ai);
        let (colors, opacities) = (&self.colors, &self.opacities);
        let c0 = |l: usize| &colors[cs[l] as usize - 1];
        let c1 = |l: usize| &colors[cs[l] as usize];
        let a0 = |l: usize| &opacities[as_[l] as usize - 1];
        let a1 = |l: usize| &opacities[as_[l] as usize];
        let (p0, p1) = (
            V::from_fn(|l| c0(l).position),
            V::from_fn(|l| c1(l).position),
        );
        let mut t = weight_lanes::<V>(x, p0, p1, V::from_fn(|l| c0(l).midpoint));
        if !self.segments.is_empty() {
            t = self.segment_weight_lanes::<V>(x, p0, p1, t, &cs);
        }
        let u = weight_lanes::<V>(
            x,
            V::from_fn(|l| a0(l).position),
            V::from_fn(|l| a1(l).position),
            V::from_fn(|l| a0(l).midpoint),
        );
        let stops = &self.stops;
        let from = StopLanes::<V>::gather(|l| &stops[cs[l] as usize - 1]);
        let to = StopLanes::<V>::gather(|l| &stops[cs[l] as usize]);
        let mut rgb = mix_lanes::<V>(&from, &to, t, self.mix, self.correction);
        if scalar {
            let luma = V::add(
                V::add(
                    V::mul(V::splat(0.2126), rgb[0]),
                    V::mul(V::splat(0.7152), rgb[1]),
                ),
                V::mul(V::splat(0.0722), rgb[2]),
            );
            rgb = [luma; 3];
        }
        let (o0, o1) = (V::from_fn(|l| a0(l).opacity), V::from_fn(|l| a1(l).opacity));
        [
            byte_lanes::<V>(rgb[0]),
            byte_lanes::<V>(rgb[1]),
            byte_lanes::<V>(rgb[2]),
            simd::to_byte::<V>(V::add(o0, V::mul(V::sub(o1, o0), u))),
        ]
    }

    /// 混合率曲線のある区間のレーンの重みを、曲線の値に差し替える（`sample_unchecked` の `segment_curve` の分岐と同じ値）。`cs` はレーンごとの
    /// 区間の番号 + 1、`midpoint` は中点の重み（曲線の無い区間のレーンはこれのまま）。
    #[inline(always)]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    unsafe fn segment_weight_lanes<V: Lanes>(
        &self,
        x: V::F,
        p0: V::F,
        p1: V::F,
        midpoint: V::F,
        cs: &[f64; 4],
    ) -> V::F {
        let same = cs[..V::N].iter().all(|c| *c == cs[0]);
        if same {
            return match self.segment_curve(cs[0] as usize - 1) {
                Some(curve) => curve
                    .value_lanes::<V>(simd::clamp01::<V>(V::div(V::sub(x, p0), V::sub(p1, p0)))),
                None => midpoint,
            };
        }
        let mut xs = [0.; 4];
        V::store_f64(&mut xs, x);
        let colors = &self.colors;
        let curved = V::from_fn(|l| {
            if self.segment_curve(cs[l] as usize - 1).is_some() {
                1.
            } else {
                0.
            }
        });
        let value = V::from_fn(|l| {
            let k = cs[l] as usize - 1;
            match self.segment_curve(k) {
                Some(curve) => curve.value_unchecked(clamp01(
                    (xs[l] - colors[k].position) / (colors[k + 1].position - colors[k].position),
                )),
                None => 0.,
            }
        });
        V::select(V::eq(curved, V::splat(1.)), value, midpoint)
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

#[cfg(all(test, any(target_arch = "x86_64", target_arch = "aarch64")))]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 / (1u64 << 53) as f64
        }
        fn byte(&mut self) -> u8 {
            (self.next() >> 8) as u8
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }
    /// `n` 個の、間隔が 0.0001 以上で 0..1 の位置（端を含むことがある）。
    fn positions(rng: &mut Rng, n: usize) -> Vec<f64> {
        loop {
            let mut v: Vec<f64> = (0..n)
                .map(|_| (rng.unit() * 1000.).round() / 1000.)
                .collect();
            if rng.below(2) == 0 {
                v[0] = 0.;
                v[n - 1] = 1.;
            }
            v.sort_by(f64::total_cmp);
            if v.windows(2).all(|w| w[1] - w[0] >= 0.0001) {
                return v;
            }
        }
    }
    fn random_ramp(rng: &mut Rng, mode: MixMode, with_segments: bool) -> Ramp {
        let nc = 2 + rng.below(6);
        let na = 2 + rng.below(5);
        let colors = positions(rng, nc)
            .into_iter()
            .map(|position| ColorStop {
                position,
                color: Rgba8::new(rng.byte(), rng.byte(), rng.byte(), rng.byte()),
                midpoint: 0.01 + 0.98 * rng.unit(),
            })
            .collect();
        let opacities = positions(rng, na)
            .into_iter()
            .map(|position| OpacityStop {
                position,
                opacity: rng.unit(),
                midpoint: 0.01 + 0.98 * rng.unit(),
            })
            .collect();
        let curve = (rng.below(2) == 0).then(|| {
            let mut points = vec![CurvePoint {
                x: 0.,
                y: rng.unit(),
            }];
            let mut x = 0.;
            while x + 0.05 < 0.9 && points.len() < 6 {
                x += 0.05 + 0.2 * rng.unit();
                if x < 0.97 {
                    points.push(CurvePoint { x, y: rng.unit() });
                }
            }
            points.push(CurvePoint {
                x: 1.,
                y: rng.unit(),
            });
            points
        });
        let mut ramp = Ramp::new(colors, opacities, curve).unwrap();
        ramp = ramp.with_mixing(mode, LuminanceCorrection::High);
        if with_segments {
            ramp = ramp
                .with_segment_curve(0, Some(Ramp::curve_from_midpoint(0.3)))
                .unwrap();
        }
        ramp
    }

    /// SIMD の道（AVX2・SSE4.1・NEON）の N 画素ぶんのランプの色は、1 画素の式とバイトまで同じ。分岐点の数・中点・値のカーブ・混色・
    /// 混合率曲線をいろいろに変え、分岐点ちょうどの入力・両端・範囲外を通る。
    #[test]
    fn lane_ramps_equal_the_scalar_evaluation_on_every_simd_level() {
        #[allow(clippy::needless_range_loop)]
        unsafe fn check<V: MixLanes>() {
            let mut rng = Rng(0x5eed);
            let mut checked = 0;
            for round in 0..120 {
                let mode = MixMode::ALL[round % 3];
                let ramp = random_ramp(&mut rng, mode, round % 7 == 6);
                let mut inputs = vec![0., 1., 0.5, -0.25, 1.75, 0.0001, 0.9999];
                for s in ramp.colors().iter().map(|c| c.position) {
                    inputs.extend([s, s - 1e-9, s + 1e-9]);
                }
                for s in ramp.opacities().iter().map(|a| a.position) {
                    inputs.extend([s, s - 1e-9, s + 1e-9]);
                }
                inputs.extend((0..400).map(|_| rng.unit()));
                for scalar in [false, true] {
                    for chunk in inputs.chunks(V::N) {
                        if chunk.len() < V::N {
                            continue;
                        }
                        let lanes = ramp.evaluate_lanes::<V>(V::load_f64(chunk), scalar);
                        let mut got = [[0.; 4]; 4];
                        for (ch, lane) in lanes.into_iter().enumerate() {
                            V::store_f64(&mut got[ch], lane);
                        }
                        for k in 0..V::N {
                            let want = ramp.evaluate_unchecked(chunk[k], scalar).to_array();
                            for ch in 0..4 {
                                assert_eq!(
                                    got[ch][k] as u8,
                                    want[ch],
                                    "round {round} {mode:?} scalar {scalar} 入力 {} チャンネル {ch}",
                                    chunk[k]
                                );
                                assert_eq!(got[ch][k], f64::from(want[ch]));
                            }
                            checked += 1;
                        }
                    }
                }
            }
            assert!(checked > 20_000, "{checked}");
        }
        crate::math::simd::on_each_level!(check);
    }

    /// 端の色の組（同じ色どうし・黒と白・0 と 255 の成分・補色）と透明な不透明度の分岐点で、混色 3 種 × 輝度の補正 5 段階 × 混合率曲線の
    /// 有無のレーンの結果が 1 画素の式とバイトまで同じ。入力は両端・分岐点ちょうど・範囲の外と、区間をまたぐ組を含む密な掃引。
    #[test]
    fn lane_mixing_equals_the_scalar_mixing_at_the_edges() {
        #[allow(clippy::needless_range_loop)]
        unsafe fn check<V: MixLanes>() {
            let pairs = [
                ([90, 90, 90], [90, 90, 90]),
                ([200, 10, 60], [200, 10, 60]),
                ([0, 0, 0], [255, 255, 255]),
                ([255, 0, 255], [0, 255, 0]),
                ([20, 40, 220], [250, 230, 30]),
                ([0, 0, 1], [1, 0, 0]),
            ];
            let mut checked = 0;
            for (first, last) in pairs {
                let stop = |position, rgb: [u8; 3]| ColorStop {
                    position,
                    color: Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
                    midpoint: 0.37,
                };
                let opacity = |position, opacity| OpacityStop {
                    position,
                    opacity,
                    midpoint: 0.5,
                };
                let base = Ramp::new(
                    vec![stop(0.0, first), stop(0.5, [128, 64, 32]), stop(1.0, last)],
                    vec![opacity(0.0, 0.0), opacity(0.6, 1.0), opacity(1.0, 0.0)],
                    None,
                )
                .unwrap();
                for mode in MixMode::ALL {
                    for correction in LuminanceCorrection::ALL {
                        for curved in [false, true] {
                            let mut ramp = base.with_mixing(mode, correction);
                            if curved {
                                ramp = ramp
                                    .with_segment_curve(1, Some(Ramp::curve_from_midpoint(0.8)))
                                    .unwrap();
                            }
                            let mut inputs = vec![-0.5, 0.0, 1.0, 1.5, 0.5, 0.6];
                            inputs.extend([0.5 - 1e-12, 0.5 + 1e-12, 1e-300, 1.0 - 1e-16]);
                            inputs.extend((0..=257).map(|k| f64::from(k) / 257.0));
                            for chunk in inputs.chunks(V::N) {
                                if chunk.len() < V::N {
                                    continue;
                                }
                                for scalar in [false, true] {
                                    let lanes =
                                        ramp.evaluate_lanes::<V>(V::load_f64(chunk), scalar);
                                    let mut got = [[0.; 4]; 4];
                                    for (ch, lane) in lanes.into_iter().enumerate() {
                                        V::store_f64(&mut got[ch], lane);
                                    }
                                    for k in 0..V::N {
                                        let want =
                                            ramp.evaluate_unchecked(chunk[k], scalar).to_array();
                                        for ch in 0..4 {
                                            assert_eq!(
                                                got[ch][k],
                                                f64::from(want[ch]),
                                                "{first:?}→{last:?} {mode:?} {correction:?} 曲線 {curved} 入力 {} チャンネル {ch}",
                                                chunk[k]
                                            );
                                        }
                                        checked += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            assert!(checked > 40_000, "{checked}");
        }
        crate::math::simd::on_each_level!(check);
    }
}
