//! パスの曲線の見せ方と掴み方。core が描くのは制御点を通る centripetal Catmull-Rom（区間 s は `[s-1, s, s+1, s+2]` の 4 点、両端は繰り返す）で、
//! ここは同じ式を f64 で持ち、画面に投影した折れ線にして、重ね表示と点・区間の当たり判定だけに使う（画素を描くのは core）。
//! 2D は z を 0 にした画素の座標、3D はモデルの空間の位置をそのまま渡す。

use egui::Pos2;
use yolu_core::glam::DVec3;
use yolu_core::paths::bezier::{display_controls, display_eval, smooth_handles};
use yolu_core::paths::Tangent;

use super::edit::TangentValue;

/// 3 成分の点（2D は z = 0）。
pub type P3 = [f64; 3];

fn to_tangent(t: TangentValue) -> Tangent<DVec3> {
    match t {
        TangentValue::Smooth => Tangent::Smooth,
        TangentValue::Corner => Tangent::Corner,
        TangentValue::Handles { incoming, outgoing } => Tangent::Handles {
            incoming: DVec3::from_array(incoming),
            outgoing: DVec3::from_array(outgoing),
        },
    }
}

/// 区間 `s` の曲線の点（両端が滑らかなら Catmull–Rom、角・取っ手があればベジェ。core と同じ式の倍精度）。`tangents` が空なら
/// 全部滑らか。
pub fn segment_point(points: &[P3], tangents: &[TangentValue], s: usize, t: f64) -> P3 {
    let w = window(points, s);
    let ends = [
        tangents.get(s).copied().unwrap_or(TangentValue::Smooth),
        tangents.get(s + 1).copied().unwrap_or(TangentValue::Smooth),
    ];
    match display_controls(w.map(DVec3::from_array), ends.map(to_tangent)) {
        None => centripetal(w, t),
        Some(c) => display_eval(c, t).to_array(),
    }
}

/// 点 `i` を取っ手の点にするときの初めの取っ手（今の曲線の曲がりを変えない値）。(入る側, 出る側)。
pub fn initial_handles(points: &[P3], i: usize) -> (P3, P3) {
    let n = points.len();
    let window_in = (i >= 1 && i < n).then(|| window(points, i - 1).map(DVec3::from_array));
    let window_out = (i + 1 < n).then(|| window(points, i).map(DVec3::from_array));
    let (a, b) = smooth_handles(window_in, window_out);
    (a.to_array(), b.to_array())
}

/// 区間 1 つの標本の数の上限。
const MAX_SAMPLES: usize = 96;

fn knot(a: P3, b: P3) -> f64 {
    let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    d.sqrt().max(1e-6)
}

/// 区間（`p[1]` → `p[2]`）の媒介変数 `t`（0〜1）の点。core の `curve` と同じ式。
pub fn centripetal(p: [P3; 4], t: f64) -> P3 {
    let t1 = knot(p[0], p[1]);
    let t2 = t1 + knot(p[1], p[2]);
    let t3 = t2 + knot(p[2], p[3]);
    let u = t1 + (t2 - t1) * t;
    let f = |c: usize| {
        let l =
            |a: f64, b: f64, ta: f64, tb: f64| (tb - u) / (tb - ta) * a + (u - ta) / (tb - ta) * b;
        let a1 = l(p[0][c], p[1][c], 0.0, t1);
        let a2 = l(p[1][c], p[2][c], t1, t2);
        let a3 = l(p[2][c], p[3][c], t2, t3);
        l(l(a1, a2, 0.0, t2), l(a2, a3, t1, t3), t1, t2)
    };
    [f(0), f(1), f(2)]
}

/// 区間 `s`（`points[s]` → `points[s + 1]`）の 4 つの制御点（端の外は端を繰り返す）。`s + 1` が範囲内であること。
pub fn window(points: &[P3], s: usize) -> [P3; 4] {
    let n = points.len();
    [
        points[s.saturating_sub(1)],
        points[s],
        points[s + 1],
        points[(s + 2).min(n - 1)],
    ]
}

/// 画面に投影した曲線: 区間ごとの標本（`project` が None を返した所は None）。標本の間隔はおよそ `step` 画素。`tangents` は点ごとの
/// 接線（空なら全部滑らか）。
pub fn sample_screen(
    points: &[P3],
    tangents: &[TangentValue],
    project: &dyn Fn(P3) -> Option<Pos2>,
    step: f32,
) -> Vec<Vec<Option<Pos2>>> {
    (0..points.len().saturating_sub(1))
        .map(|s| {
            let w = window(points, s);
            // 取っ手で膨らむ区間は、制御多角形の画面の長さで標本を増やす
            let ends = [
                tangents.get(s).copied().unwrap_or(TangentValue::Smooth),
                tangents.get(s + 1).copied().unwrap_or(TangentValue::Smooth),
            ];
            let polygon: Vec<P3> =
                match display_controls(w.map(DVec3::from_array), ends.map(to_tangent)) {
                    None => vec![w[1], w[2]],
                    Some(c) => c.iter().map(|v| v.to_array()).collect(),
                };
            let screen: Option<Vec<Pos2>> = polygon.iter().map(|p| project(*p)).collect();
            let n = match screen {
                Some(v) => {
                    let len: f32 = v.windows(2).map(|q| q[0].distance(q[1])).sum();
                    ((len / step.max(1.0)).ceil() as usize).clamp(4, MAX_SAMPLES)
                }
                _ => 8,
            };
            (0..=n)
                .map(|k| project(segment_point(points, tangents, s, k as f64 / n as f64)))
                .collect()
        })
        .collect()
}

/// 線分 `a`–`b` の上で点 `p` にいちばん近い点。
pub fn closest_on_segment(p: Pos2, a: Pos2, b: Pos2) -> Pos2 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 <= f32::EPSILON {
        return a;
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    a + ab * t
}

/// 点 `p` から線分 `a`–`b` までの距離。
pub fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    p.distance(closest_on_segment(p, a, b))
}

/// ポインタに一番近い区間（曲線から `radius` 画素以内のもの）の番号と、その区間の上でポインタに一番近い画面の点。
pub fn nearest_segment(
    samples: &[Vec<Option<Pos2>>],
    pointer: Pos2,
    radius: f32,
) -> Option<(usize, Pos2)> {
    let mut best: Option<(usize, Pos2, f32)> = None;
    for (s, run) in samples.iter().enumerate() {
        for pair in run.windows(2) {
            let (Some(a), Some(b)) = (pair[0], pair[1]) else {
                continue;
            };
            let at = closest_on_segment(pointer, a, b);
            let d = at.distance(pointer);
            if d <= radius && best.is_none_or(|(_, _, e)| d < e) {
                best = Some((s, at, d));
            }
        }
    }
    best.map(|(s, at, _)| (s, at))
}

/// 画面の点のうちポインタに一番近いもの（`radius` 画素以内）の番号。同じ距離なら後ろの点（後から足した点を掴む）。
pub fn nearest_point(points: &[Option<Pos2>], pointer: Pos2, radius: f32) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, p) in points.iter().enumerate() {
        let Some(p) = p else { continue };
        let d = p.distance(pointer);
        if d <= radius && best.is_none_or(|(_, e)| d <= e) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    #[test]
    fn the_curve_passes_through_its_control_points() {
        let pts: Vec<P3> = vec![
            [0.0, 0.0, 0.0],
            [10.0, 5.0, 0.0],
            [20.0, -5.0, 0.0],
            [30.0, 0.0, 0.0],
        ];
        for s in 0..3 {
            let w = window(&pts, s);
            assert_eq!(centripetal(w, 0.0), pts[s]);
            let end = centripetal(w, 1.0);
            for c in 0..3 {
                assert!((end[c] - pts[s + 1][c]).abs() < 1e-9, "{end:?}");
            }
        }
    }

    #[test]
    fn a_straight_run_stays_straight() {
        let pts: Vec<P3> = vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [30.0, 0.0, 0.0]];
        for k in 0..=10 {
            let p = centripetal(window(&pts, 0), k as f64 / 10.0);
            assert!(
                p[1].abs() < 1e-9 && p[2].abs() < 1e-9 && (0.0..=10.0).contains(&p[0]),
                "{p:?}"
            );
        }
    }

    #[test]
    fn nearest_segment_and_point_respect_the_radius() {
        let pts: Vec<P3> = vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 100.0, 0.0]];
        let project = |p: P3| Some(pos2(p[0] as f32, p[1] as f32));
        let samples = sample_screen(&pts, &[], &project, 5.0);
        assert_eq!(samples.len(), 2);
        // 各区間の真ん中の標本の近くを指すと、その区間（曲線は点の間でふくらむので、標本から測る）
        for s in 0..2 {
            let mid = samples[s][samples[s].len() / 2].unwrap();
            let (found, at) = nearest_segment(&samples, mid + vec2(2.0, 2.0), 8.0).expect("近い");
            assert_eq!(found, s);
            assert!(at.distance(mid) < 4.0, "{at:?} {mid:?}");
        }
        assert_eq!(nearest_segment(&samples, pos2(-60.0, 200.0), 8.0), None);
        let screen: Vec<Option<Pos2>> = pts.iter().map(|p| project(*p)).collect();
        assert_eq!(nearest_point(&screen, pos2(103.0, 4.0), 8.0), Some(1));
        assert_eq!(nearest_point(&screen, pos2(50.0, 50.0), 8.0), None);
        // 同じ距離なら後ろの点
        let overlapping = vec![Some(pos2(10.0, 10.0)), Some(pos2(10.0, 10.0))];
        assert_eq!(nearest_point(&overlapping, pos2(11.0, 10.0), 8.0), Some(1));
    }

    #[test]
    fn unprojectable_samples_break_the_run() {
        let pts: Vec<P3> = vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]];
        let samples = sample_screen(&pts, &[], &|_| None, 5.0);
        assert!(samples[0].iter().all(Option::is_none));
        assert_eq!(nearest_segment(&samples, pos2(5.0, 0.0), 8.0), None);
    }
}
