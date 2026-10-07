//! 角と取っ手のある区間の曲線（3 次のベジェ）。両端が滑らかな区間は呼び手が今までの centripetal Catmull–Rom の式のまま描き
//! （同じバイト）、どちらかの端が角か取っ手の区間だけをここで作る。
//!
//! 滑らかな端の制御点は、Catmull–Rom の曲線のその端での微分から作る（Barry–Goldman の形は区間の中で 3 次の多項式なので、
//! 制御点 `p ± m · (t2 − t1) / 3` のベジェは同じ曲線）。角の端は点そのもの（そこで折れる）、取っ手の端は点 + 取っ手。

use glam::{DVec2, DVec3, Vec3};

use super::{CanvasPoint, Tangent};
use crate::geometry::unity::magnitude;

/// 区間（`p[1]` → `p[2]`）の、`p[1]` の出る側と `p[2]` の入る側の制御点（2D）。両端が滑らかなら None（Catmull–Rom で描く）。
pub(super) fn canvas_controls(p: [CanvasPoint; 4]) -> Option<[DVec2; 4]> {
    if p[1].tangent.is_smooth() && p[2].tangent.is_smooth() {
        return None;
    }
    let v = p.map(|q| DVec2::new(q.x, q.y));
    let knot = |a: DVec2, b: DVec2| {
        ((a.x - b.x).powi(2) + (a.y - b.y).powi(2))
            .sqrt()
            .sqrt()
            .max(1e-6)
    };
    let t1 = knot(v[0], v[1]);
    let t2 = t1 + knot(v[1], v[2]);
    let t3 = t2 + knot(v[2], v[3]);
    let dt = t2 - t1;
    let c1 = match p[1].tangent {
        Tangent::Smooth => {
            let m = (v[1] - v[0]) / t1 - (v[2] - v[0]) / t2 + (v[2] - v[1]) / dt;
            v[1] + m * (dt / 3.0)
        }
        Tangent::Corner => v[1],
        Tangent::Handles { outgoing, .. } => v[1] + outgoing,
    };
    let c2 = match p[2].tangent {
        Tangent::Smooth => {
            let m = (v[2] - v[1]) / dt - (v[3] - v[1]) / (t3 - t1) + (v[3] - v[2]) / (t3 - t2);
            v[2] - m * (dt / 3.0)
        }
        Tangent::Corner => v[2],
        Tangent::Handles { incoming, .. } => v[2] + incoming,
    };
    Some([v[1], c1, c2, v[2]])
}

/// 区間（`p[1]` → `p[2]`）の制御点（3D。位置は休みの形のモデルの空間）。両端が滑らかなら None。
pub(super) fn surface_controls(p: [Vec3; 4], ends: [Tangent<Vec3>; 2]) -> Option<[Vec3; 4]> {
    if ends[0].is_smooth() && ends[1].is_smooth() {
        return None;
    }
    let knot = |a: Vec3, b: Vec3| magnitude(a - b).sqrt().max(1e-6);
    let t1 = knot(p[0], p[1]);
    let t2 = t1 + knot(p[1], p[2]);
    let t3 = t2 + knot(p[2], p[3]);
    let dt = t2 - t1;
    let c1 = match ends[0] {
        Tangent::Smooth => {
            let m = (p[1] - p[0]) / t1 - (p[2] - p[0]) / t2 + (p[2] - p[1]) / dt;
            p[1] + m * (dt / 3.0)
        }
        Tangent::Corner => p[1],
        Tangent::Handles { outgoing, .. } => p[1] + outgoing,
    };
    let c2 = match ends[1] {
        Tangent::Smooth => {
            let m = (p[2] - p[1]) / dt - (p[3] - p[1]) / (t3 - t1) + (p[3] - p[2]) / (t3 - t2);
            p[2] - m * (dt / 3.0)
        }
        Tangent::Corner => p[2],
        Tangent::Handles { incoming, .. } => p[2] + incoming,
    };
    Some([p[1], c1, c2, p[2]])
}

pub(super) fn eval2(c: [DVec2; 4], t: f64) -> DVec2 {
    let s = 1.0 - t;
    c[0] * (s * s * s) + c[1] * (3.0 * s * s * t) + c[2] * (3.0 * s * t * t) + c[3] * (t * t * t)
}

pub(super) fn eval3(c: [Vec3; 4], t: f32) -> Vec3 {
    let s = 1.0 - t;
    c[0] * (s * s * s) + c[1] * (3.0 * s * s * t) + c[2] * (3.0 * s * t * t) + c[3] * (t * t * t)
}

/// 制御多角形の長さ（曲線の長さ以上。標本の数を決めるのに使う）。
pub(super) fn length2(c: [DVec2; 4]) -> f64 {
    c[0].distance(c[1]) + c[1].distance(c[2]) + c[2].distance(c[3])
}

pub(super) fn length3(c: [Vec3; 4]) -> f32 {
    magnitude(c[1] - c[0]) + magnitude(c[2] - c[1]) + magnitude(c[3] - c[2])
}

/// 画面に見せる曲線のための、倍精度の同じ式（2D は z = 0）。区間（`p[1]` → `p[2]`）の両端の接線から、ベジェの制御点を返す
/// （両端が滑らかなら None。呼び手は Catmull–Rom で見せる）。
pub fn display_controls(p: [DVec3; 4], ends: [Tangent<DVec3>; 2]) -> Option<[DVec3; 4]> {
    if ends[0].is_smooth() && ends[1].is_smooth() {
        return None;
    }
    let knot = |a: DVec3, b: DVec3| a.distance(b).sqrt().max(1e-6);
    let t1 = knot(p[0], p[1]);
    let t2 = t1 + knot(p[1], p[2]);
    let t3 = t2 + knot(p[2], p[3]);
    let dt = t2 - t1;
    let c1 = match ends[0] {
        Tangent::Smooth => p[1] + smooth_out(p, t1, t2) * (dt / 3.0),
        Tangent::Corner => p[1],
        Tangent::Handles { outgoing, .. } => p[1] + outgoing,
    };
    let c2 = match ends[1] {
        Tangent::Smooth => p[2] - smooth_in(p, t1, t2, t3) * (dt / 3.0),
        Tangent::Corner => p[2],
        Tangent::Handles { incoming, .. } => p[2] + incoming,
    };
    Some([p[1], c1, c2, p[2]])
}

fn smooth_out(p: [DVec3; 4], t1: f64, t2: f64) -> DVec3 {
    (p[1] - p[0]) / t1 - (p[2] - p[0]) / t2 + (p[2] - p[1]) / (t2 - t1)
}

fn smooth_in(p: [DVec3; 4], t1: f64, t2: f64, t3: f64) -> DVec3 {
    (p[2] - p[1]) / (t2 - t1) - (p[3] - p[1]) / (t3 - t1) + (p[3] - p[2]) / (t3 - t2)
}

/// 滑らかな点の、Catmull–Rom と同じ曲がりになる取っ手（倍精度）。`window_in` は点が終わりの区間（`[p-2, p-1, p, p+1]`）、
/// `window_out` は点が始めの区間（`[p-1, p, p+1, p+2]`）。端の点で区間が無い側は 0。滑らかな点を取っ手の点に替えるとき、曲線を
/// 動かさない初めの値に使う。
pub fn smooth_handles(
    window_in: Option<[DVec3; 4]>,
    window_out: Option<[DVec3; 4]>,
) -> (DVec3, DVec3) {
    let knots = |p: [DVec3; 4]| {
        let knot = |a: DVec3, b: DVec3| a.distance(b).sqrt().max(1e-6);
        let t1 = knot(p[0], p[1]);
        let t2 = t1 + knot(p[1], p[2]);
        let t3 = t2 + knot(p[2], p[3]);
        (t1, t2, t3)
    };
    let incoming = window_in.map_or(DVec3::ZERO, |p| {
        let (t1, t2, t3) = knots(p);
        -smooth_in(p, t1, t2, t3) * ((t2 - t1) / 3.0)
    });
    let outgoing = window_out.map_or(DVec3::ZERO, |p| {
        let (t1, t2, _) = knots(p);
        smooth_out(p, t1, t2) * ((t2 - t1) / 3.0)
    });
    (incoming, outgoing)
}

/// 倍精度のベジェの点（画面に見せる曲線）。
pub fn display_eval(c: [DVec3; 4], t: f64) -> DVec3 {
    let s = 1.0 - t;
    c[0] * (s * s * s) + c[1] * (3.0 * s * s * t) + c[2] * (3.0 * s * t * t) + c[3] * (t * t * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cp(x: f64, y: f64) -> CanvasPoint {
        CanvasPoint::new(x, y, 1.0).unwrap()
    }

    /// 滑らかな端の制御点は、Catmull–Rom の曲線と同じ曲線になる（片方の端だけ角にしても、滑らかな端の向きは同じ）。
    #[test]
    fn a_smooth_end_has_the_catmull_rom_direction() {
        let p = [cp(0.0, 0.0), cp(10.0, 5.0), cp(20.0, -5.0), cp(30.0, 0.0)];
        // 両方を「滑らか」にした制御点の曲線（ここでは試しに 2 つ目の端を Handles にした制御点の c1 を見る）
        let mut q = p;
        q[2].tangent = Tangent::Corner;
        let c = canvas_controls(q).unwrap();
        // 媒介変数の小さな所で、Catmull–Rom の曲線と同じ向きに出る
        let cr = super::super::render::curve_for_test(p, 1e-4);
        let bz = eval2(c, 1e-4);
        let d_cr = (DVec2::new(cr.0, cr.1) - DVec2::new(10.0, 5.0)).normalize();
        let d_bz = (bz - DVec2::new(10.0, 5.0)).normalize();
        assert!(d_cr.dot(d_bz) > 0.999_999, "{d_cr} {d_bz}");
    }

    #[test]
    fn two_corners_make_a_straight_segment() {
        let mut p = [cp(0.0, 0.0), cp(10.0, 5.0), cp(20.0, -5.0), cp(30.0, 0.0)];
        p[1].tangent = Tangent::Corner;
        p[2].tangent = Tangent::Corner;
        let c = canvas_controls(p).unwrap();
        for k in 0..=10 {
            let q = eval2(c, k as f64 / 10.0);
            // 10,5 → 20,-5 の直線の上
            assert!((q.y - (5.0 - (q.x - 10.0))).abs() < 1e-9, "{q}");
        }
    }

    #[test]
    fn the_display_handles_of_a_smooth_point_keep_the_curve() {
        let pts = [
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(10.0, 5.0, 0.0),
            DVec3::new(20.0, -5.0, 0.0),
            DVec3::new(30.0, 0.0, 0.0),
        ];
        let w = |s: usize| {
            [
                pts[s.saturating_sub(1)],
                pts[s],
                pts[s + 1],
                pts[(s + 2).min(3)],
            ]
        };
        // 点 1 を取っ手の点にしても、点 1 の両側の区間の曲線は変わらない
        let (incoming, outgoing) = smooth_handles(Some(w(0)), Some(w(1)));
        let handles = Tangent::Handles { incoming, outgoing };
        let after = display_controls(w(1), [handles, Tangent::Smooth]).unwrap();
        let before = display_controls(w(1), [Tangent::Smooth, Tangent::Corner]).unwrap();
        assert!(after[1].distance(before[1]) < 1e-9);
        let after_in = display_controls(w(0), [Tangent::Smooth, handles]).unwrap();
        let before_in = display_controls(w(0), [Tangent::Corner, Tangent::Smooth]).unwrap();
        assert!(after_in[2].distance(before_in[2]) < 1e-9);
    }
}
