//! 0〜1 から 0〜1 への値のカーブ（点の並びと、その間を単調に補間する PCHIP の 3 次の接線）。
//!
//! グラデーションのランプの値のカーブ（`generator::Ramp`）、トーンカーブ、筆圧のカーブが同じ型を使う。点は x が 0 から 1 まで
//! 増える 2〜16 個で、両端の x は 0 と 1、y は 0〜1。隣り合う点の横の間隔は [`Curve::MIN_GAP`] 以上。評価の式は C# の
//! `GradientRamp` の値のカーブと同じで、入力は 0〜1 に収めてから 3 次のエルミートで引き、結果も 0〜1 に収める。
use crate::math::clamp01;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use crate::math::simd::{self, Lanes};
use std::fmt;

/// カーブの点（横が入力、縦が出力）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurvePoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid(&'static str),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for Error {}

fn unit(x: f64) -> bool {
    x.is_finite() && (0. ..=1.).contains(&x)
}

/// 検査済みの値のカーブ。作れた時点で点・間隔・端が正しく、接線は点から決まる（`PartialEq` は点と接線の両方を比べる）。
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    points: Vec<CurvePoint>,
    tangents: Vec<f64>,
}

impl Default for Curve {
    fn default() -> Self {
        Self::identity()
    }
}

impl Curve {
    /// 点の数の下限と上限。
    pub const MIN_POINTS: usize = 2;
    pub const MAX_POINTS: usize = 16;
    /// 隣り合う点の横の間隔の下限（丸めの余裕を 1e-6 だけ残す。C# の `GradientRamp` と同じ）。
    pub const MIN_GAP: f64 = 0.02 - 1e-6;

    /// 入力をそのまま返す直線 (0, 0)〜(1, 1)。
    pub fn identity() -> Self {
        Self::new(vec![
            CurvePoint { x: 0., y: 0. },
            CurvePoint { x: 1., y: 1. },
        ])
        .expect("直線は検査を通る")
    }

    /// 点の並びから作る。数・x の増え方と間隔・端の x（0 と 1）・y が範囲外ならエラー。
    pub fn new(points: Vec<CurvePoint>) -> Result<Self, Error> {
        let bad = Error::Invalid("カーブの点が範囲外です");
        if !(Self::MIN_POINTS..=Self::MAX_POINTS).contains(&points.len()) {
            return Err(bad);
        }
        let mut last: Option<f64> = None;
        for p in &points {
            if !unit(p.x) || !unit(p.y) || last.is_some_and(|v| p.x - v < Self::MIN_GAP) {
                return Err(bad);
            }
            last = Some(p.x);
        }
        if points[0].x != 0. || points[points.len() - 1].x != 1. {
            return Err(bad);
        }
        let n = points.len();
        let mut m = vec![0.; n];
        let mut h = vec![0.; n - 1];
        let mut d = vec![0.; n - 1];
        for k in 0..n - 1 {
            h[k] = points[k + 1].x - points[k].x;
            d[k] = (points[k + 1].y - points[k].y) / h[k];
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
            points,
            tangents: m,
        })
    }

    pub fn points(&self) -> &[CurvePoint] {
        &self.points
    }

    /// 入力をそのまま返す直線か（点が (0, 0) と (1, 1) の 2 つだけ）。
    pub fn is_identity(&self) -> bool {
        self.points.len() == 2 && self.points[0].y == 0. && self.points[1].y == 1.
    }

    /// 履歴に積む大きさ（C# の `GradientRamp.ByteSize` の数え方: 64 + 点の数 × 24）。
    pub fn byte_size(&self) -> u64 {
        64 + 24 * self.points.len() as u64
    }

    /// 入力 `x` の出力（0〜1）。`x` が有限でなければエラー。範囲外の入力は 0〜1 に収めてから引く。
    pub fn value(&self, x: f64) -> Result<f64, Error> {
        if !x.is_finite() {
            return Err(Error::Invalid("カーブの入力は有限値が必要です"));
        }
        Ok(self.value_unchecked(x))
    }

    /// `value` の有限かどうかの検査を呼び手が済ませた版（NaN は通さない前提）。
    pub fn value_unchecked(&self, input: f64) -> f64 {
        let x = clamp01(input);
        let mut k = 1;
        while k < self.points.len() - 1 && self.points[k].x < x {
            k += 1;
        }
        let a = self.points[k - 1];
        let b = self.points[k];
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
}

impl Curve {
    /// `value_unchecked` の N 画素ぶん（同じ演算を同じ順で、区間はレーンごとに引く）。値のビットはスカラーと同じ。
    ///
    /// # Safety
    /// `V` の命令を持つ CPU で、その命令を有効にした `#[target_feature]` 付きの入口の中から呼ぶ。
    #[inline(always)]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    pub(crate) unsafe fn value_lanes<V: Lanes>(&self, input: V::F) -> V::F {
        let x = simd::clamp01::<V>(input);
        let last = self.points.len() - 1;
        // 区間の番号 k（1 から last まで）: 点 1..last のうち x より小さい x を持つものの数 + 1
        let mut count = V::splat(1.);
        for p in &self.points[1..last] {
            count = V::add(
                count,
                V::select(V::lt(V::splat(p.x), x), V::splat(1.), V::splat(0.)),
            );
        }
        let mut ks = [0.; 4];
        V::store_f64(&mut ks, count);
        let points = &self.points;
        let tangents = &self.tangents;
        let ax = V::from_fn(|l| points[ks[l] as usize - 1].x);
        let ay = V::from_fn(|l| points[ks[l] as usize - 1].y);
        let bx = V::from_fn(|l| points[ks[l] as usize].x);
        let by = V::from_fn(|l| points[ks[l] as usize].y);
        let m0 = V::from_fn(|l| tangents[ks[l] as usize - 1]);
        let m1 = V::from_fn(|l| tangents[ks[l] as usize]);
        let h = V::sub(bx, ax);
        let t = V::div(V::sub(x, ax), h);
        let t2 = V::mul(t, t);
        let t3 = V::mul(t2, t);
        let (one, two, three) = (V::splat(1.), V::splat(2.), V::splat(3.));
        // (2t³ − 3t² + 1)·a.y + (t³ − 2t² + t)·h·m0 + (−2t³ + 3t²)·b.y + (t³ − t²)·h·m1
        let h00 = V::add(V::sub(V::mul(two, t3), V::mul(three, t2)), one);
        let h10 = V::add(V::sub(t3, V::mul(two, t2)), t);
        let h01 = V::add(V::mul(V::splat(-2.), t3), V::mul(three, t2));
        let h11 = V::sub(t3, t2);
        let sum = V::add(
            V::add(
                V::add(V::mul(h00, ay), V::mul(V::mul(h10, h), m0)),
                V::mul(h01, by),
            ),
            V::mul(V::mul(h11, h), m1),
        );
        simd::clamp01::<V>(sum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(list: &[(f64, f64)]) -> Vec<CurvePoint> {
        list.iter().map(|&(x, y)| CurvePoint { x, y }).collect()
    }

    #[test]
    fn identity_returns_the_input_and_is_the_default() {
        let c = Curve::identity();
        assert_eq!(c, Curve::default());
        assert!(c.is_identity());
        for x in [0., 0.1, 0.5, 0.987, 1.] {
            assert!((c.value(x).unwrap() - x).abs() < 1e-12, "{x}");
        }
        // 範囲外は端に収める
        assert_eq!(c.value(-3.).unwrap(), 0.);
        assert_eq!(c.value(7.).unwrap(), 1.);
        assert!(c.value(f64::NAN).is_err());
        assert!(c.value(f64::INFINITY).is_err());
    }

    #[test]
    fn known_values_pass_through_the_points_and_stay_monotone() {
        let c = Curve::new(pts(&[(0., 0.1), (0.25, 0.3), (0.6, 0.7), (1., 0.95)])).unwrap();
        for p in c.points() {
            assert!((c.value(p.x).unwrap() - p.y).abs() < 1e-12, "{p:?}");
        }
        // PCHIP は点の間で行き過ぎない（単調な点列は単調な曲線になる）
        let mut prev = c.value(0.).unwrap();
        for i in 1..=1000 {
            let y = c.value(f64::from(i) / 1000.).unwrap();
            assert!(y >= prev - 1e-12, "{i}");
            prev = y;
        }
        assert!(!c.is_identity());
        assert_eq!(c.byte_size(), 64 + 24 * 4);
    }

    #[test]
    fn invalid_points_are_refused() {
        assert!(Curve::new(vec![]).is_err());
        assert!(Curve::new(pts(&[(0., 0.)])).is_err(), "1 点");
        assert!(
            Curve::new(pts(&[(0.1, 0.), (1., 1.)])).is_err(),
            "始まりが 0 でない"
        );
        assert!(
            Curve::new(pts(&[(0., 0.), (0.9, 1.)])).is_err(),
            "終わりが 1 でない"
        );
        assert!(
            Curve::new(pts(&[(0., 0.), (0.01, 0.5), (1., 1.)])).is_err(),
            "間隔"
        );
        assert!(Curve::new(pts(&[(0., 0.), (0.5, 0.5), (0.5, 0.6), (1., 1.)])).is_err());
        assert!(
            Curve::new(pts(&[(0., 0.), (0.7, 0.5), (0.3, 0.6), (1., 1.)])).is_err(),
            "逆順"
        );
        assert!(
            Curve::new(pts(&[(0., -0.1), (1., 1.)])).is_err(),
            "y が範囲外"
        );
        assert!(Curve::new(pts(&[(0., 0.), (1., 1.0001)])).is_err());
        assert!(Curve::new(pts(&[(0., f64::NAN), (1., 1.)])).is_err());
        assert!(Curve::new(pts(&[(0., 0.), (f64::NAN, 0.5), (1., 1.)])).is_err());
        // 点の数の上限は 16
        let many = |n: usize| -> Vec<(f64, f64)> {
            (0..n).map(|i| (i as f64 / (n - 1) as f64, 0.5)).collect()
        };
        assert!(Curve::new(pts(&many(16))).is_ok());
        assert!(Curve::new(pts(&many(17))).is_err());
        // 間隔の下限ちょうど（0.02）は通る
        assert!(Curve::new(pts(&[(0., 0.), (0.02, 0.5), (1., 1.)])).is_ok());
    }

    #[test]
    fn two_points_are_a_straight_line_between_the_ends() {
        let c = Curve::new(pts(&[(0., 0.2), (1., 0.8)])).unwrap();
        assert!((c.value(0.5).unwrap() - 0.5).abs() < 1e-12);
        assert!(!c.is_identity());
    }
}
