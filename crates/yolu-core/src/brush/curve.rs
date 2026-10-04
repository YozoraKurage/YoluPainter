//! 入力の点を結ぶ曲線（C# の StrokeCurve）: centripetal Catmull-Rom（α = 0.5。区間の中で尖りも自己交差も作らず、点の間隔が
//! 不揃いでも行き過ぎが小さい）。区間 p1 → p2 を、前後の点 p0・p3 で向きを決めて結ぶ。曲線は制御点を必ず通り、一直線に並んだ
//! 点では直線のまま。前後の点が無い端は [`reflect`] の点（端の点の向こうへ折り返した点）で補う。

/// 重なった点とみなす距離（画素）。これより近い点の間は区間を作らない。
pub const COINCIDENT_DISTANCE: f64 = 1e-6;

/// 区間 p1 → p2 の t（0 で p1、1 で p2）の位置（Barry と Goldman のピラミッド形の式）。
#[allow(clippy::too_many_arguments)]
pub fn point(
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    x3: f64,
    y3: f64,
    t: f64,
) -> (f64, f64) {
    let k01 = knot(x0, y0, x1, y1);
    let k12 = knot(x1, y1, x2, y2);
    let k23 = knot(x2, y2, x3, y3);
    let (t0, t1) = (0.0, k01);
    let t2 = t1 + k12;
    let t3 = t2 + k23;
    let u = t1 + k12 * t;
    let (a1x, a1y) = (mix(x0, x1, t0, t1, u), mix(y0, y1, t0, t1, u));
    let (a2x, a2y) = (mix(x1, x2, t1, t2, u), mix(y1, y2, t1, t2, u));
    let (a3x, a3y) = (mix(x2, x3, t2, t3, u), mix(y2, y3, t2, t3, u));
    let (b1x, b1y) = (mix(a1x, a2x, t0, t2, u), mix(a1y, a2y, t0, t2, u));
    let (b2x, b2y) = (mix(a2x, a3x, t1, t3, u), mix(a2y, a3y, t1, t3, u));
    (mix(b1x, b2x, t1, t2, u), mix(b1y, b2y, t1, t2, u))
}

/// 端の外の点: p を about の向こうへ折り返した点（2·about − p）。
#[inline]
pub fn reflect(x: f64, y: f64, about_x: f64, about_y: f64) -> (f64, f64) {
    (2.0 * about_x - x, 2.0 * about_y - y)
}

/// 2 点が [`COINCIDENT_DISTANCE`] より近いか。
#[inline]
pub fn coincident(x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
    let (dx, dy) = (x1 - x0, y1 - y0);
    dx * dx + dy * dy < COINCIDENT_DISTANCE * COINCIDENT_DISTANCE
}

// 結び目の間隔は距離の平方根（centripetal）。重なった前後の点でも割り算が壊れないよう下限を置く。
#[inline]
fn knot(xa: f64, ya: f64, xb: f64, yb: f64) -> f64 {
    let (dx, dy) = (xb - xa, yb - ya);
    let k = (dx * dx + dy * dy).sqrt().sqrt();
    if 1e-9 > k {
        1e-9
    } else {
        k
    }
}

#[inline]
fn mix(a: f64, b: f64, ta: f64, tb: f64, u: f64) -> f64 {
    (tb - u) / (tb - ta) * a + (u - ta) / (tb - ta) * b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curve_passes_through_its_points_and_keeps_a_line_straight() {
        // C# StrokeCurveTests.TheCurvePassesThroughItsPointsAndKeepsALineStraight
        let (x, y) = point(0.0, 0.0, 10.0, 3.0, 20.0, -4.0, 31.0, 2.0, 0.0);
        assert!((x - 10.0).abs() < 1e-9 && (y - 3.0).abs() < 1e-9);
        let (x, y) = point(0.0, 0.0, 10.0, 3.0, 20.0, -4.0, 31.0, 2.0, 1.0);
        assert!((x - 20.0).abs() < 1e-9 && (y + 4.0).abs() < 1e-9);
        for i in 0..=10 {
            let t = i as f64 / 10.0;
            let (x, y) = point(0.0, 0.0, 2.0, 1.0, 10.0, 5.0, 11.0, 5.5, t);
            assert!((y - x * 0.5).abs() < 1e-9, "t={t}");
        }
        assert_eq!(reflect(1.0, 2.0, 3.0, 3.0), (5.0, 4.0));
        assert!(coincident(1.0, 1.0, 1.0 + 1e-7, 1.0));
        assert!(!coincident(1.0, 1.0, 1.0 + 1e-5, 1.0));
    }
}
