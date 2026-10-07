//! 全体の筆圧の調整（CLIP STUDIO の「筆圧の調整」に当たる）: ペンの筆圧を、ブラシへ渡す前に、端末ごとの下限・上限と曲線で直す。
//! ペンの個体ごとに筆圧の出方が違う（軽く触れても出る・強く押しても 1 に届かない）ので、描き手の普段の強さがブラシの 0〜1 に収まるように
//! 合わせる。設定（`settings`）に入り、文書にもブラシにも入らない。
//!
//! 直し方: p（ペンの筆圧 0〜1）→ t = clamp((p − 下限) / (上限 − 下限), 0, 1) → 曲線(t)。既定（下限 0・上限 1・直線）は p をそのまま返す
//! （浮動小数の演算を挟まない）ので、今までと同じ。マウスの筆圧は 1 のままで、ここを通さない。
//!
//! 自動で決める式（[`fit`]）: 描いた線の筆圧（0 を除く）の 10・50・90 パーセンタイル（最近傍の順位）を p10・p50・p90 とし、
//! 幅 w = p90 − p10 として、下限 = p10 − w/8、上限 = p90 + w/8（0〜1 に収める）。これで p10 が 0.1、p90 が 0.9 に写る（直線のとき。
//! 片側が 0 か 1 に収められたときは、その側の写り先が 0.1・0.9 からずれる）。収めたあとの上限 − 下限が [`MIN_SPAN`] に足りなければ、
//! 上限を上げ（1 で止まれば下限を下げ）て広げる。
//! さらに p50 を t50 = (p50 − 下限)/(上限 − 下限) へ写したとき t50 が 0.5 から 0.02 以上ずれていれば、曲線に点 (t50, 0.5) を足して
//! 中央値が 0.5 になるよう曲げる（点は (0,0)・(t50, 0.5)・(1,1) の 3 つで、間隔は 0.1 以上）。曲線を足すと p10・p90 の写り先は
//! 0.1・0.9 でなくなる（中央値を 0.5 に合わせるほうを優先する）。w が 0.08 未満（筆圧がほぼ一定）か、筆圧のある点が 24 未満なら決めない。

use crate::engine::{CoreError, PressureResponse};
use yolu_core::curve::{Curve, CurvePoint};

/// 上限 − 下限の最小。これより狭いと、わずかな筆圧の違いで 0 と 1 を行き来する。
pub const MIN_SPAN: f32 = 0.1;
/// 自動で決めるのに要る、筆圧のある点の数。
pub const MIN_SAMPLES: usize = 24;
/// 自動で決めるのに要る、10〜90 パーセンタイルの幅。
const MIN_FIT_WIDTH: f32 = 0.08;

/// 全体の筆圧の調整（下限・上限・曲線）。作るときに検査するので、持っている値は常に範囲内。曲線の点は画面の精度（f32）に丸める
/// （設定のファイルが f32 で書くので、保存して読み戻しても同じ値になる）。
#[derive(Clone, Debug, PartialEq)]
pub struct PressureAdjust {
    low: f32,
    high: f32,
    /// 最小値 0 の応え（曲線だけを使う）。
    curve: PressureResponse,
}

impl Default for PressureAdjust {
    /// 下限 0・上限 1・直線（ペンの筆圧をそのまま使う）。
    fn default() -> Self {
        PressureAdjust {
            low: 0.0,
            high: 1.0,
            curve: PressureResponse::default(),
        }
    }
}

impl PressureAdjust {
    /// 下限・上限（0〜1、上限 − 下限が [`MIN_SPAN`] 以上）と曲線の点（空か (0,0)(1,1) は直線）から作る。範囲外は断る。
    pub fn new(low: f32, high: f32, curve: Vec<CurvePoint>) -> Result<PressureAdjust, CoreError> {
        if !low.is_finite()
            || !high.is_finite()
            || low < 0.0
            || high > 1.0
            || high - low < MIN_SPAN - 1e-6
        {
            return Err(CoreError::InvalidArgument("筆圧の下限・上限"));
        }
        Ok(PressureAdjust {
            low,
            high,
            curve: PressureResponse::new(0.0, curve)?.rounded_to_f32(),
        })
    }

    pub fn low(&self) -> f32 {
        self.low
    }

    pub fn high(&self) -> f32 {
        self.high
    }

    /// 曲線の点（直線は空）。
    pub fn curve(&self) -> &[CurvePoint] {
        self.curve.curve()
    }

    /// 編集の部品（`ui::curve::curve_editor`）へ渡す曲線（直線なら両端の 2 点の直線）。
    pub fn curve_shape(&self) -> Curve {
        self.curve.curve_shape()
    }

    /// 既定（下限 0・上限 1・直線）か。
    pub fn is_default(&self) -> bool {
        self.low == 0.0 && self.high == 1.0 && self.curve.is_identity()
    }

    /// 下限・上限を変えた写し。範囲は収める（下限 0〜上限 − [`MIN_SPAN`]、上限 下限 + [`MIN_SPAN`]〜1）。動かしたほうが、
    /// 相手から [`MIN_SPAN`] の所で止まる（相手は動かさない。今の値が [`MIN_SPAN`] 以上の幅なら、相手を動かす必要は起きない）。
    pub fn with_range(&self, low: f32, high: f32, moved_low: bool) -> PressureAdjust {
        let (mut low, mut high) = (low.clamp(0.0, 1.0), high.clamp(0.0, 1.0));
        if moved_low {
            // 下限を動かした: 上限の手前で止める。上限が MIN_SPAN に満たない（作るときの検査があるので起きない）ときだけ上限を上げる
            if high >= MIN_SPAN {
                low = low.min(high - MIN_SPAN);
            } else {
                low = 0.0;
                high = MIN_SPAN;
            }
        } else if low + MIN_SPAN <= 1.0 {
            high = high.max(low + MIN_SPAN);
        } else {
            low = 1.0 - MIN_SPAN;
            high = 1.0;
        }
        PressureAdjust {
            low,
            high,
            curve: self.curve.clone(),
        }
    }

    /// 曲線を替えた写し（編集の部品が返した、検査済みの曲線を入れる。画面の精度 f32 に丸める）。
    pub fn with_curve_shape(&self, curve: Curve) -> Result<PressureAdjust, CoreError> {
        Ok(PressureAdjust {
            low: self.low,
            high: self.high,
            curve: PressureResponse::from_curve(0.0, curve)?.rounded_to_f32(),
        })
    }

    /// ペンの筆圧 p（0〜1）をブラシへ渡す筆圧にする。既定なら p そのもの。
    pub fn apply(&self, pressure: f32) -> f32 {
        if self.is_default() {
            return pressure;
        }
        let p = if pressure.is_finite() {
            pressure.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let t = ((p - self.low) / (self.high - self.low)).clamp(0.0, 1.0);
        self.curve.apply(t as f64).clamp(0.0, 1.0) as f32
    }
}

/// 自動で決められなかった理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitError {
    /// 筆圧のある点が [`MIN_SAMPLES`] 未満。
    TooFew,
    /// 筆圧がほぼ一定（10〜90 パーセンタイルの幅が 0.08 未満）。
    TooNarrow,
}

/// 描いた線の筆圧（ペンの値。0 を含んでよい）から、調整を決める（式はモジュールの説明）。純粋で決定的。
pub fn fit(samples: &[f32]) -> Result<PressureAdjust, FitError> {
    let mut sorted: Vec<f32> = samples
        .iter()
        .copied()
        .filter(|p| p.is_finite() && *p > 0.0)
        .map(|p| p.min(1.0))
        .collect();
    if sorted.len() < MIN_SAMPLES {
        return Err(FitError::TooFew);
    }
    sorted.sort_by(f32::total_cmp);
    let quantile = |q: f32| sorted[((sorted.len() - 1) as f32 * q).round() as usize];
    let (p10, p50, p90) = (quantile(0.1), quantile(0.5), quantile(0.9));
    let width = p90 - p10;
    if width < MIN_FIT_WIDTH {
        return Err(FitError::TooNarrow);
    }
    let mut low = (p10 - width / 8.0).max(0.0);
    let mut high = (p90 + width / 8.0).min(1.0);
    // 幅が 0.08 以上でも、片側だけ 0 か 1 に収められると上限 − 下限は 0.1 に届かないことがある（1.125 w〜1.25 w の間）。
    // 上限を上げ、1 で止まったぶんは下限を下げて、広げる
    if high - low < MIN_SPAN {
        high = (low + MIN_SPAN).min(1.0);
        low = low.min(high - MIN_SPAN).max(0.0);
    }
    let t50 = (p50 - low) / (high - low);
    let curve = if (t50 - 0.5).abs() > 0.02 {
        let t50 = t50.clamp(0.1, 0.9) as f64;
        vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: t50, y: 0.5 },
            CurvePoint { x: 1.0, y: 1.0 },
        ]
    } else {
        Vec::new()
    };
    PressureAdjust::new(low, high, curve).map_err(|_| FitError::TooNarrow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64) -> CurvePoint {
        CurvePoint { x, y }
    }

    /// 一様に並んだ筆圧 lo〜hi（n 個）。
    fn uniform(lo: f32, hi: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| lo + (hi - lo) * i as f32 / (n - 1) as f32)
            .collect()
    }

    #[test]
    fn the_default_returns_the_pressure_untouched() {
        let a = PressureAdjust::default();
        assert!(a.is_default());
        for p in [0.0f32, 1e-30, 0.1234567, 0.5, 0.999999, 1.0] {
            assert_eq!(a.apply(p).to_bits(), p.to_bits());
        }
        assert_eq!(PressureAdjust::new(0.0, 1.0, vec![]).unwrap(), a);
        assert_eq!(
            PressureAdjust::new(
                0.0,
                1.0,
                vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 1.0 }]
            )
            .unwrap(),
            a
        );
    }

    #[test]
    fn the_range_stretches_the_pen_pressure_and_the_curve_bends_it() {
        let a = PressureAdjust::new(0.2, 0.8, vec![]).unwrap();
        assert_eq!(a.apply(0.0), 0.0);
        assert_eq!(a.apply(0.2), 0.0);
        assert!((a.apply(0.5) - 0.5).abs() < 1e-6);
        assert!((a.apply(0.35) - 0.25).abs() < 1e-6);
        assert_eq!(a.apply(0.8), 1.0);
        assert_eq!(a.apply(1.0), 1.0);
        let bent = PressureAdjust::new(
            0.0,
            1.0,
            vec![
                CurvePoint { x: 0.0, y: 0.0 },
                CurvePoint { x: 0.5, y: 0.8 },
                CurvePoint { x: 1.0, y: 1.0 },
            ],
        )
        .unwrap();
        assert!((bent.apply(0.5) - 0.8).abs() < 1e-6);
        let mut last = 0.0;
        for i in 0..=100 {
            let v = bent.apply(i as f32 / 100.0);
            assert!(v >= last && (0.0..=1.0).contains(&v));
            last = v;
        }
        // 有限でない筆圧は 0
        assert_eq!(bent.apply(f32::NAN), 0.0);
    }

    #[test]
    fn bad_ranges_and_curves_are_refused() {
        assert!(PressureAdjust::new(-0.1, 1.0, vec![]).is_err());
        assert!(PressureAdjust::new(0.0, 1.1, vec![]).is_err());
        assert!(PressureAdjust::new(0.5, 0.55, vec![]).is_err());
        assert!(PressureAdjust::new(f32::NAN, 1.0, vec![]).is_err());
        assert!(PressureAdjust::new(0.0, 1.0, vec![CurvePoint { x: 0.0, y: 0.0 }]).is_err());
        assert!(PressureAdjust::new(0.1, 0.19, vec![]).is_err());
        assert!(PressureAdjust::new(0.1, 0.2, vec![]).is_ok());
    }

    #[test]
    fn moving_one_end_stops_at_the_minimum_span_from_the_other_and_never_moves_it() {
        let a = PressureAdjust::default();
        // 端: 動かしたほうが相手の手前で止まる
        let b = a.with_range(0.95, 1.0, true);
        assert!((b.high() - b.low() - MIN_SPAN).abs() < 1e-6);
        assert_eq!(b.high(), 1.0);
        let c = a.with_range(0.0, 0.02, false);
        assert!((c.high() - c.low() - MIN_SPAN).abs() < 1e-6);
        assert_eq!(c.low(), 0.0);
        assert_eq!(a.with_range(0.2, 0.7, true).low(), 0.2);
        // 余裕のある所（相手を動かせる場合）でも相手は動かさない: 下限 0.2・上限 0.3 で、下限を 0.25 へ → 下限は 0.2 で止まり、上限は 0.3 のまま
        let narrow = PressureAdjust::new(0.2, 0.3, vec![]).unwrap();
        let d = narrow.with_range(0.25, 0.3, true);
        assert!(
            (d.low() - 0.2).abs() < 1e-6 && d.high() == 0.3,
            "{} {}",
            d.low(),
            d.high()
        );
        // 上限を 0.25 へ → 上限は 0.3 で止まり、下限は 0.2 のまま
        let e = narrow.with_range(0.2, 0.25, false);
        assert!(
            (e.high() - 0.3).abs() < 1e-6 && e.low() == 0.2,
            "{} {}",
            e.low(),
            e.high()
        );
        // 下限を相手の向こうまで動かしても、相手の手前で止まる
        let f = narrow.with_range(0.9, 0.3, true);
        assert!((f.low() - 0.2).abs() < 1e-6 && f.high() == 0.3);
        // 曲線は変えない
        let curved =
            PressureAdjust::new(0.0, 1.0, vec![pt(0.0, 0.0), pt(0.5, 0.8), pt(1.0, 1.0)]).unwrap();
        assert_eq!(curved.with_range(0.1, 0.9, true).curve(), curved.curve());
    }

    #[test]
    fn the_fit_maps_the_tenth_and_ninetieth_percentiles_to_a_tenth_and_nine_tenths() {
        // 筆圧が 0.2〜0.6 に収まるペン: 一様な分布なら p10 = 0.24、p90 = 0.56、中央値は真ん中
        let samples = uniform(0.2, 0.6, 201);
        let fit = fit(&samples).unwrap();
        let (p10, p90) = (0.24f32, 0.56f32);
        assert!((fit.apply(p10) - 0.1).abs() < 0.01, "{}", fit.apply(p10));
        assert!((fit.apply(p90) - 0.9).abs() < 0.01, "{}", fit.apply(p90));
        assert!(fit.curve().is_empty(), "中央値が真ん中なら直線のまま");
        assert!(fit.low() > 0.0 && fit.high() < 1.0);
        // どの筆圧も 0〜1 に収まり、増える
        let mut last = -1.0;
        for p in uniform(0.0, 1.0, 101) {
            let v = fit.apply(p);
            assert!(v >= last && (0.0..=1.0).contains(&v));
            last = v;
        }
    }

    #[test]
    fn the_fit_bends_the_curve_so_the_median_becomes_one_half() {
        // 軽い筆圧が多い（二乗で寄せた分布）: 中央値が範囲の下の方にあるので、曲線で持ち上げる
        let samples: Vec<f32> = uniform(0.0, 1.0, 301)
            .into_iter()
            .map(|t| 0.05 + 0.7 * t * t)
            .collect();
        let fit = fit(&samples).unwrap();
        assert_eq!(fit.curve().len(), 3);
        let mut sorted = samples.clone();
        sorted.sort_by(f32::total_cmp);
        let median = sorted[sorted.len() / 2];
        assert!(
            (fit.apply(median) - 0.5).abs() < 0.01,
            "{}",
            fit.apply(median)
        );
        // 同じ入力は同じ結果（決定的）、順序によらない
        let mut reversed = samples.clone();
        reversed.reverse();
        assert_eq!(super::fit(&reversed).unwrap(), fit);
    }

    #[test]
    fn the_fit_ignores_zeros_and_refuses_when_there_is_nothing_to_fit() {
        let mut with_zero = uniform(0.2, 0.6, 201);
        with_zero.extend(std::iter::repeat_n(0.0, 500));
        assert_eq!(
            fit(&with_zero).unwrap(),
            fit(&uniform(0.2, 0.6, 201)).unwrap()
        );
        assert_eq!(fit(&[]).unwrap_err(), FitError::TooFew);
        assert_eq!(fit(&uniform(0.2, 0.6, 23)).unwrap_err(), FitError::TooFew);
        assert!(fit(&uniform(0.2, 0.6, 24)).is_ok());
        assert_eq!(fit(&vec![0.5; 100]).unwrap_err(), FitError::TooNarrow);
        assert_eq!(
            fit(&uniform(0.5, 0.57, 100)).unwrap_err(),
            FitError::TooNarrow
        );
        // 有限でない値は数えない
        let mut noisy = uniform(0.2, 0.6, 201);
        noisy.extend([f32::NAN, f32::INFINITY]);
        assert_eq!(fit(&noisy).unwrap(), fit(&uniform(0.2, 0.6, 201)).unwrap());
    }

    /// 下の 10% が lo、上の 10% が hi の分布（lo・hi の幅がそのまま p90 − p10。点は 100 個）。
    fn two_levels(lo: f32, hi: f32) -> Vec<f32> {
        let mut v = vec![lo; 20];
        v.extend(std::iter::repeat_n(hi, 80));
        v
    }

    #[test]
    fn a_narrow_fit_clamped_on_one_side_widens_to_the_minimum_span_instead_of_panicking() {
        // 強く押して飽和するペン: p10 = 0.915・p90 = 1.0（幅 0.085）。上限が 1 に収まり、そのままだと幅 0.0956 になる
        let fit = fit(&two_levels(0.915, 1.0)).unwrap();
        assert!(
            fit.high() == 1.0 && fit.high() - fit.low() >= MIN_SPAN - 1e-6,
            "{} {}",
            fit.low(),
            fit.high()
        );
        assert!((fit.low() - 0.9).abs() < 1e-5, "{}", fit.low());
        assert_eq!(fit.apply(1.0), 1.0);
        // 中央値が上限に張り付くので曲線も付くが、どの筆圧も 0〜1 に写って増える
        let mut last = -1.0;
        for k in 0..=100 {
            let v = fit.apply(k as f32 / 100.0);
            assert!(v >= last && (0.0..=1.0).contains(&v));
            last = v;
        }
        // 軽くしか押さないペン: p10 = 0.001・p90 = 0.086（幅 0.085）。下限が 0 に収まり、そのままだと幅 0.0966 になる
        let fit = fit_of(&two_levels(0.001, 0.086));
        assert!(
            fit.low() == 0.0 && fit.high() - fit.low() >= MIN_SPAN - 1e-6,
            "{} {}",
            fit.low(),
            fit.high()
        );
        assert!((fit.high() - 0.1).abs() < 1e-6, "{}", fit.high());
        assert_eq!(fit.apply(0.0), 0.0);
        assert_eq!(fit.apply(0.1), 1.0);
        // 境界の幅（0.08 ちょうど付近）でも落ちない
        for (lo, hi) in [
            (0.915, 1.0),
            (0.911, 1.0),
            (0.001, 0.0861),
            (0.0005, 0.0856),
        ] {
            let fit = fit_of(&two_levels(lo, hi));
            assert!(
                fit.high() - fit.low() >= MIN_SPAN - 1e-6,
                "{lo} {hi}: {} {}",
                fit.low(),
                fit.high()
            );
        }
    }

    /// 決まる前提の入力から調整を取り出す（決まらなければ失敗）。
    fn fit_of(samples: &[f32]) -> PressureAdjust {
        super::fit(samples).expect("決まる")
    }

    #[test]
    fn the_fit_never_panics_and_always_returns_a_valid_adjustment_over_a_grid_of_distributions() {
        // 下の 10% が p10、上の 10% が p90 の分布を、0〜1 の細かい格子で総当たりする。決まるか TooNarrow で、決まったときは
        // 範囲が 0〜1 に収まり幅が MIN_SPAN 以上、どの筆圧も 0〜1 に写って増える
        let steps = 100;
        let mut decided = 0;
        for i in 0..=steps {
            for j in i..=steps {
                let (p10, p90) = (
                    i as f32 / steps as f32 * 0.9999 + 0.0001,
                    j as f32 / steps as f32,
                );
                if p90 < p10 {
                    continue;
                }
                match fit(&two_levels(p10, p90)) {
                    Ok(f) => {
                        decided += 1;
                        assert!(
                            f.low() >= 0.0
                                && f.high() <= 1.0
                                && f.high() - f.low() >= MIN_SPAN - 1e-6,
                            "{p10} {p90}"
                        );
                        let mut last = -1.0;
                        for k in 0..=20 {
                            let v = f.apply(k as f32 / 20.0);
                            assert!(v >= last && (0.0..=1.0).contains(&v), "{p10} {p90}");
                            last = v;
                        }
                    }
                    Err(e) => assert_eq!(e, FitError::TooNarrow, "{p10} {p90}"),
                }
            }
        }
        assert!(decided > 1000, "{decided}");
    }

    #[test]
    fn when_the_fit_adds_a_curve_the_median_is_one_half_and_the_outer_percentiles_are_not_pinned_to_a_tenth_and_nine_tenths(
    ) {
        let percentile = |samples: &[f32], f: f32| {
            let mut sorted = samples.to_vec();
            sorted.sort_by(f32::total_cmp);
            sorted[((sorted.len() - 1) as f32 * f).round() as usize]
        };
        // 縁に収められない分布（0.3〜0.9 を二乗で寄せる。中央値は範囲の下の方）: 下限・上限だけなら p10・p90 は 0.1・0.9 に写る
        // が、中央値を 0.5 に合わせる曲線が付くと、p10・p90 はその曲がりの分だけ 0.1・0.9 から離れる
        let samples: Vec<f32> = uniform(0.0, 1.0, 301)
            .into_iter()
            .map(|t| 0.3 + 0.6 * t * t)
            .collect();
        let fit = fit_of(&samples);
        assert_eq!(fit.curve().len(), 3);
        let (p10, p50, p90) = (
            percentile(&samples, 0.1),
            percentile(&samples, 0.5),
            percentile(&samples, 0.9),
        );
        let straight = PressureAdjust::new(fit.low(), fit.high(), vec![]).unwrap();
        assert!(
            (straight.apply(p10) - 0.1).abs() < 1e-3,
            "{}",
            straight.apply(p10)
        );
        assert!(
            (straight.apply(p90) - 0.9).abs() < 1e-3,
            "{}",
            straight.apply(p90)
        );
        let (m10, m50, m90) = (fit.apply(p10), fit.apply(p50), fit.apply(p90));
        // 測った値: p10 → 約 0.168、p50 → 0.5、p90 → 約 0.962（曲線が持ち上げるので、どちらも 0.1・0.9 より内側でなく外側へ）
        assert!((m50 - 0.5).abs() < 0.01, "{m50}");
        assert!(m10 > 0.12 && m10 < 0.2, "{m10}");
        assert!(m90 > 0.93 && m90 < 1.0, "{m90}");
        // 軽い筆圧が多い分布（0.05 + 0.7 t²。下限が 0 に収まる）でも中央値は 0.5
        let light: Vec<f32> = uniform(0.0, 1.0, 301)
            .into_iter()
            .map(|t| 0.05 + 0.7 * t * t)
            .collect();
        let fit = fit_of(&light);
        assert_eq!(fit.curve().len(), 3);
        let (m10, m50, m90) = (
            fit.apply(percentile(&light, 0.1)),
            fit.apply(percentile(&light, 0.5)),
            fit.apply(percentile(&light, 0.9)),
        );
        // 測った値: p10 → 約 0.146、p50 → 0.5、p90 → 約 0.967
        assert!((m50 - 0.5).abs() < 0.01, "{m50}");
        assert!(m10 > 0.12 && m10 < 0.2, "{m10}");
        assert!(m90 > 0.93 && m90 < 1.0, "{m90}");
        // 中央値が上に寄る分布では、逆に下へ曲がる
        let heavy: Vec<f32> = uniform(0.0, 1.0, 301)
            .into_iter()
            .map(|t| 0.25 + 0.7 * t.sqrt())
            .collect();
        let fit = fit_of(&heavy);
        assert_eq!(fit.curve().len(), 3);
        assert!((fit.apply(percentile(&heavy, 0.5)) - 0.5).abs() < 0.01);
    }

    #[test]
    fn a_pen_that_already_uses_the_whole_range_keeps_almost_the_same_pressure() {
        // 0〜1 を一様に使うペン: 下限・上限は縁のすぐ内側で、ほぼ直線のまま
        let fit = fit(&uniform(0.0, 1.0, 1001)).unwrap();
        assert!(
            fit.low() < 0.01 && fit.high() > 0.99,
            "{} {}",
            fit.low(),
            fit.high()
        );
        assert!(fit.curve().is_empty());
        for p in uniform(0.0, 1.0, 21) {
            assert!((fit.apply(p) - p).abs() < 0.02, "{p}: {}", fit.apply(p));
        }
    }
}
