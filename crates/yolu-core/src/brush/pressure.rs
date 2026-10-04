//! 筆圧の応え（CLIP STUDIO の「影響元設定」の筆圧に当たる）: 筆圧 p（0〜1）を、項目（大きさ・不透明度・流量・硬さ）ごとの係数へ
//! 写す。係数 = 最小値 + (1 − 最小値) × 曲線(p)。最小値は筆圧 0 のときの係数（0〜1）、曲線は点を通る単調な 3 次エルミート
//! （共通の値のカーブ [`crate::curve::Curve`] ＝ 塗りつぶしのグラデーション・トーンカーブと同じ PCHIP。点は 2〜16 個、両端は入力 0 と 1、
//! 間隔は 0.02 以上、値は 0〜1）。
//!
//! 項目を筆圧で変えるかどうか（切り替え）はここに無く、これまでどおり [`super::BrushSettings`] の `pressure_size`・`pressure_opacity`・
//! `pressure_flow` と、拡張の [`super::Controls::pressure_hardness`] が持つ。切っている項目は筆圧を使わない（係数 1）。
//!
//! **既定（最小値 0・直線）は筆圧をそのまま返す**（浮動小数の演算を 1 つも挟まない）ので、応えを足す前のストロークとバイト単位で
//! 同じになる。直線は「曲線の点が無い」か「(0,0) と (1,1) の 2 点」で、どちらも同じ既定として扱う。
//!

use crate::curve::{Curve, CurvePoint};
use crate::error::CoreError;
use crate::math::{clamp01, require_finite};

/// 直線の曲線の点（両端だけ）。
pub const STRAIGHT: [CurvePoint; 2] =
    [CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 1.0 }];

/// 曲線の点の数の上限。
pub const MAX_CURVE_POINTS: usize = Curve::MAX_POINTS;

/// 筆圧 1 項目の応え（最小値と曲線）。作るときに検査するので、持っている値はいつも範囲の中。
#[derive(Clone, Debug, PartialEq)]
pub struct PressureResponse {
    min: f64,
    /// 直線でない曲線（直線は None）。
    curve: Option<Curve>,
}

impl Default for PressureResponse {
    /// 最小値 0・直線（筆圧をそのまま使う）。
    fn default() -> Self {
        PressureResponse {
            min: 0.0,
            curve: None,
        }
    }
}

impl PressureResponse {
    /// 最小値（0〜1）と曲線の点（空か 2 点の直線は直線）から作る。範囲外・有限でない値・形の合わない曲線は断る。
    pub fn new(min: f64, curve: Vec<CurvePoint>) -> Result<PressureResponse, CoreError> {
        let curve = if curve.is_empty() {
            Curve::identity()
        } else {
            Curve::new(curve).map_err(|_| {
                CoreError::InvalidArgument(
                    "筆圧の曲線（点 2〜16・両端は 0 と 1・間隔 0.02 以上・値 0〜1）",
                )
            })?
        };
        PressureResponse::from_curve(min, curve)
    }

    /// 最小値（0〜1）と、検査済みの曲線から作る（編集の部品が返した曲線をそのまま入れる口。直線は直線として扱う）。
    pub fn from_curve(min: f64, curve: Curve) -> Result<PressureResponse, CoreError> {
        require_finite(min, "pressure minimum")?;
        if !(0.0..=1.0).contains(&min) {
            return Err(CoreError::InvalidArgument("筆圧の最小値（0〜1）"));
        }
        Ok(PressureResponse {
            min,
            curve: (!curve.is_identity()).then_some(curve),
        })
    }

    /// 最小値だけを変えた写し（曲線はそのまま）。
    pub fn with_min(&self, min: f64) -> Result<PressureResponse, CoreError> {
        PressureResponse::new(min, self.curve().to_vec())
    }

    /// 曲線だけを変えた写し（最小値はそのまま）。
    pub fn with_curve(&self, curve: Vec<CurvePoint>) -> Result<PressureResponse, CoreError> {
        PressureResponse::new(self.min, curve)
    }

    /// 検査済みの曲線へ差し替えた写し（最小値はそのまま。編集の部品が返した曲線を入れる）。
    pub fn with_curve_shape(&self, curve: Curve) -> Result<PressureResponse, CoreError> {
        PressureResponse::from_curve(self.min, curve)
    }

    /// 筆圧 0 のときの係数（0〜1）。
    pub fn min(&self) -> f64 {
        self.min
    }

    /// 曲線の点（直線は空）。
    pub fn curve(&self) -> &[CurvePoint] {
        self.curve.as_ref().map_or(&[], |c| c.points())
    }

    /// 編集の部品へ渡す曲線（直線なら両端の 2 点の直線）。
    pub fn curve_shape(&self) -> Curve {
        self.curve.clone().unwrap_or_default()
    }

    /// 編集の元にする曲線の点（直線なら両端の 2 点）。
    pub fn curve_points(&self) -> Vec<CurvePoint> {
        match &self.curve {
            Some(c) => c.points().to_vec(),
            None => STRAIGHT.to_vec(),
        }
    }

    /// 最小値 0・直線か（このとき [`PressureResponse::apply`] は筆圧をそのまま返す）。
    pub fn is_identity(&self) -> bool {
        self.min == 0.0 && self.curve.is_none()
    }

    /// 筆圧 p（0〜1）の係数。恒等なら p そのもの。
    #[inline]
    pub fn apply(&self, pressure: f64) -> f64 {
        if self.is_identity() {
            return pressure;
        }
        let p = if pressure.is_finite() {
            clamp01(pressure)
        } else {
            0.0
        };
        let shaped = match &self.curve {
            Some(curve) => curve.value_unchecked(p),
            None => p,
        };
        self.min + (1.0 - self.min) * shaped
    }

    /// 最小値と曲線の点を、画面の精度（f32）に丸めた写し（保存して読み戻しても同じ値になる形）。丸めて範囲を外れるときは元のまま。
    pub fn rounded_to_f32(&self) -> PressureResponse {
        let f = |v: f64| v as f32 as f64;
        let points = self
            .curve()
            .iter()
            .map(|p| CurvePoint {
                x: f(p.x),
                y: f(p.y),
            })
            .collect();
        PressureResponse::new(f(self.min), points).unwrap_or_else(|_| self.clone())
    }
}

/// ブラシの筆圧の応え（項目ごと）。項目を筆圧で変えるかの切り替えは含まない（モジュールの説明の通り、設定の bool が持つ）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PressureResponses {
    pub size: PressureResponse,
    pub opacity: PressureResponse,
    pub flow: PressureResponse,
    pub hardness: PressureResponse,
}

impl PressureResponses {
    /// どの項目も既定（最小値 0・直線）か。
    pub fn is_identity(&self) -> bool {
        self.size.is_identity()
            && self.opacity.is_identity()
            && self.flow.is_identity()
            && self.hardness.is_identity()
    }

    /// 全項目を画面の精度（f32）に丸めた写し。
    pub fn rounded_to_f32(&self) -> PressureResponses {
        PressureResponses {
            size: self.size.rounded_to_f32(),
            opacity: self.opacity.rounded_to_f32(),
            flow: self.flow.rounded_to_f32(),
            hardness: self.hardness.rounded_to_f32(),
        }
    }
}

/// 1 つのダブ（か 1 回の画素の呼び出し）の、筆圧を応えに通した後の不透明度と流量の係数（切っている項目は 1）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PressureScale {
    pub opacity: f64,
    pub flow: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64) -> CurvePoint {
        CurvePoint { x, y }
    }

    #[test]
    fn the_default_and_a_straight_curve_return_the_pressure_untouched() {
        for r in [
            PressureResponse::default(),
            PressureResponse::new(0.0, vec![]).unwrap(),
            PressureResponse::new(0.0, STRAIGHT.to_vec()).unwrap(),
        ] {
            assert!(r.is_identity());
            assert_eq!(r, PressureResponse::default());
            for p in [0.0, 1e-300, 0.1234567891011, 0.5, 0.999999999, 1.0] {
                assert_eq!(r.apply(p).to_bits(), p.to_bits());
            }
        }
    }

    #[test]
    fn the_minimum_lifts_pressure_zero_and_keeps_one() {
        let r = PressureResponse::new(0.25, vec![]).unwrap();
        assert!(!r.is_identity());
        assert_eq!(r.apply(0.0), 0.25);
        assert_eq!(r.apply(1.0), 1.0);
        assert_eq!(r.apply(0.5), 0.25 + 0.75 * 0.5);
        // 最小値 1 は筆圧を無視して 1
        let full = PressureResponse::new(1.0, vec![]).unwrap();
        assert_eq!(full.apply(0.0), 1.0);
        assert_eq!(full.apply(0.3), 1.0);
    }

    #[test]
    fn a_curve_bends_the_response_and_the_minimum_applies_after_it() {
        let soft = vec![pt(0.0, 0.0), pt(0.5, 0.8), pt(1.0, 1.0)];
        let r = PressureResponse::new(0.0, soft.clone()).unwrap();
        assert!((r.apply(0.5) - 0.8).abs() < 1e-12, "{}", r.apply(0.5));
        assert_eq!(r.apply(0.0), 0.0);
        assert_eq!(r.apply(1.0), 1.0);
        let mut last = 0.0;
        for i in 0..=100 {
            let v = r.apply(i as f64 / 100.0);
            assert!(v >= last - 1e-12 && (0.0..=1.0).contains(&v), "{i}: {v}");
            last = v;
        }
        let both = PressureResponse::new(0.2, soft).unwrap();
        assert!((both.apply(0.5) - (0.2 + 0.8 * 0.8)).abs() < 1e-12);
        assert_eq!(both.apply(0.0), 0.2);
        assert_eq!(both.apply(1.0), 1.0);
    }

    #[test]
    fn a_curve_that_ends_below_one_scales_the_top() {
        let r = PressureResponse::new(0.0, vec![pt(0.0, 0.0), pt(1.0, 0.6)]).unwrap();
        assert!((r.apply(1.0) - 0.6).abs() < 1e-12);
        assert!((r.apply(0.5) - 0.3).abs() < 1e-12);
    }

    #[test]
    fn bad_minimums_and_curves_are_refused() {
        for min in [-0.1, 1.01, f64::NAN, f64::INFINITY] {
            assert!(PressureResponse::new(min, vec![]).is_err(), "{min}");
        }
        let bad: Vec<Vec<CurvePoint>> = vec![
            vec![pt(0.0, 0.0)],                                           // 1 点
            vec![pt(0.0, 0.0), pt(0.5, 0.5)],                             // 終わりが 1 でない
            vec![pt(0.1, 0.0), pt(1.0, 1.0)],                             // 始まりが 0 でない
            vec![pt(0.0, 0.0), pt(0.5, 1.5), pt(1.0, 1.0)],               // 値が 1 を超える
            vec![pt(0.0, 0.0), pt(0.5, -0.1), pt(1.0, 1.0)],              // 値が負
            vec![pt(0.0, 0.0), pt(0.5, f64::NAN), pt(1.0, 1.0)],          // 有限でない
            vec![pt(0.0, 0.0), pt(0.01, 0.5), pt(1.0, 1.0)],              // 間隔が狭い
            vec![pt(0.0, 0.0), pt(0.6, 0.5), pt(0.4, 0.7), pt(1.0, 1.0)], // 入力の順でない
            (0..17)
                .map(|i| pt(i as f64 / 16.0, i as f64 / 16.0))
                .collect(), // 17 点
        ];
        for (i, curve) in bad.into_iter().enumerate() {
            assert!(PressureResponse::new(0.0, curve).is_err(), "case {i}");
        }
        let sixteen: Vec<CurvePoint> = (0..16)
            .map(|i| pt(i as f64 / 15.0, i as f64 / 15.0))
            .collect();
        assert!(PressureResponse::new(0.0, sixteen).is_ok());
    }

    #[test]
    fn a_checked_curve_goes_in_and_comes_back_out_unchanged() {
        let shaped = Curve::new(vec![pt(0.0, 0.0), pt(0.4, 0.7), pt(1.0, 1.0)]).unwrap();
        let r = PressureResponse::from_curve(0.1, shaped.clone()).unwrap();
        assert_eq!(r.curve_shape(), shaped);
        assert_eq!(r.curve().len(), 3);
        assert_eq!(r.min(), 0.1);
        // 点の列から作ったものと同じ（評価も含めて）
        let from_points = PressureResponse::new(0.1, shaped.points().to_vec()).unwrap();
        assert_eq!(r, from_points);
        for p in [0.0, 0.2, 0.4, 0.77, 1.0] {
            assert_eq!(r.apply(p).to_bits(), from_points.apply(p).to_bits());
        }
        // 差し替えは最小値を保つ。直線へ戻すと既定の曲線（点なし）に戻る
        let back = r.with_curve_shape(Curve::identity()).unwrap();
        assert_eq!(back.min(), 0.1);
        assert!(back.curve().is_empty());
        assert_eq!(back, PressureResponse::new(0.1, vec![]).unwrap());
        assert_eq!(PressureResponse::default().curve_shape(), Curve::identity());
        assert!(PressureResponse::from_curve(1.5, Curve::identity()).is_err());
        assert!(PressureResponse::from_curve(f64::NAN, Curve::identity()).is_err());
    }

    #[test]
    fn rounding_to_f32_is_stable_and_keeps_the_shape() {
        let r = PressureResponse::new(
            0.123456789012345,
            vec![
                pt(0.0, 0.0),
                pt(0.3333333333333333, 0.7777777777777777),
                pt(1.0, 1.0),
            ],
        )
        .unwrap();
        let once = r.rounded_to_f32();
        assert_eq!(once.rounded_to_f32(), once);
        assert_eq!(once.min(), 0.123_456_79_f32 as f64);
        assert_eq!(once.curve().len(), 3);
        assert!((once.apply(0.4) - r.apply(0.4)).abs() < 1e-6);
    }
}
