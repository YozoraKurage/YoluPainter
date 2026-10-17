//! 線の補正の拡張: 手ぶれ補正の速度による調整・はらい・後補正。2D のストローク（[`super::StrokeState`]）と 3D の面のストローク
//! （`geometry::SurfaceStroke`）で同じ式を使う。長さの単位は呼び手のもの（2D は文書の画素、3D は画面の点）。速さは、影響元の速度と同じ
//! 筆の速さを、ブラシの速さの上限（[`super::Influences::speed_max`]）で割って 0〜1 に収めた値。
//!
//! - 手ぶれ補正の速度による調整（[`StabilizerSpeed`]）: 糸の長さを、直前の筆の点の速さで変える。
//! - はらい（[`StrokeAssist::sweep`]）: 筆圧が下がるとき、線の太さが追いつく速さを抑える。下がり方の上限は、筆が進んだ長さあたり
//!   `SWEEP_FALL ÷ はらい`（はらい 0 は上限なしで、今までと同じ）。上がるときは、そのまま追う。
//! - 後補正（[`PostCorrection`]）: 離したときに、入力の点の列を、許しの距離より小さな揺れを捨てた折れ線（Ramer–Douglas–Peucker）を
//!   許しと同じ幅でならした線へ置き直す（[`smooth`]）。まっすぐな辺はまっすぐ、角は許しほどの丸みだけ。描いている間の点は描き直しの
//!   ときに捨てる（描き直しは呼び手）。

use glam::DVec2;

use super::dynamics::f64_max;
use super::settings::StrokeAssist;
use crate::math::clamp01;

/// 手ぶれ補正の速度による調整の補正タイプ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StabilizerSpeed {
    /// すばやく描いたときに弱く補正する: 止まっているとき糸の長さのまま、速さの上限で半分。
    WeakenWhenFast,
    /// ゆっくり描いたときに補正をかける: 止まっているとき糸の長さのまま、速さの上限の [`SLOW_SHARE`] で 0（それより速い所は補正しない）。
    SlowOnly,
}

/// 後補正。描いている間は補正の無い線を見せ、離したときに入力の点の列を整えて描き直す（1 回の Undo）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostCorrection {
    pub on: bool,
    /// 強さ（0〜[`MAX_POST_STRENGTH`]）。許しの距離は 強さ × `POST_TOLERANCE`（表示倍率による調整が入りなら画面の点、切なら文書の画素）。
    pub strength: f64,
    /// 速度による調整: 速く描いた所ほど許しを小さくする（速さの上限で半分）。
    pub by_speed: bool,
    /// 表示倍率による調整: 許しの距離を画面の点で決める（切なら文書の画素）。
    pub by_view_scale: bool,
    /// ストロークを始めたときの表示倍率（文書の画素 1 が画面で何点か）。設定ではなく、[`super::Brush::at_view_scale`] がストロークの
    /// 始めに入れる値（保存しない。既定は 1）。
    pub view_scale: f64,
}

impl Default for PostCorrection {
    /// 切。入れたときの値は、強さ 10・速度による調整と表示倍率による調整が入り。
    fn default() -> Self {
        PostCorrection {
            on: false,
            strength: 10.0,
            by_speed: true,
            by_view_scale: true,
            view_scale: 1.0,
        }
    }
}

/// 手ぶれ補正の速度による調整の「ゆっくり」の範囲（速さの上限に対する割合）。
pub const SLOW_SHARE: f64 = 0.2;
/// 後補正の強さの上限。
pub const MAX_POST_STRENGTH: f64 = 100.0;
/// 後補正の強さ 1 あたりの許しの距離（画面の点か文書の画素）。
const POST_TOLERANCE: f64 = 0.1;
/// はらい 1（100%）での、筆が進んだ長さ 1 あたりの筆圧の下がり方の上限。
const SWEEP_FALL: f64 = 0.0019;
/// 後補正の折れ線を刻む間隔の上限（呼び手の単位）。
const POST_PIECE_LENGTH: f64 = 2.0;
/// 後補正の折れ線を刻む間隔の下限（呼び手の単位）。
const MIN_POST_STEP: f64 = 0.25;
/// 後補正の 1 本の区間が出す点の数の上限（長い線は、刻む間隔を広げてこの数に収める）。
const MAX_POST_POINTS: usize = 1 << 20;

impl PostCorrection {
    /// 描き直すか（入りで強さが 0 より大きい）。
    pub fn active(&self) -> bool {
        self.on && self.strength > 0.0
    }

    /// 設定の値（表示倍率を除く）が既定と同じか。
    pub fn is_default_setting(&self) -> bool {
        PostCorrection {
            view_scale: 1.0,
            ..*self
        } == PostCorrection::default()
    }

    /// 許しの距離。`screen` は呼び手の単位が画面の点か（3D の面）、偽なら文書の画素（2D のキャンバス）。
    pub(crate) fn tolerance(&self, screen: bool) -> f64 {
        let base = self.strength * POST_TOLERANCE;
        let scale = self.view_scale;
        match (self.by_view_scale, screen) {
            (true, false) => base / scale,
            (false, true) => base * scale,
            _ => base,
        }
    }
}

impl StrokeAssist {
    /// 直前の筆の速さ `speed` での糸の長さ（速度による調整が無ければ [`StrokeAssist::stabilizer`] そのもの）。
    pub(crate) fn stabilizer_length(&self, speed: f64, speed_max: f64) -> f64 {
        match self.stabilizer_speed {
            None => self.stabilizer,
            Some(kind) => {
                let fast = clamp01(speed / speed_max);
                match kind {
                    StabilizerSpeed::WeakenWhenFast => self.stabilizer * (1.0 - 0.5 * fast),
                    StabilizerSpeed::SlowOnly => {
                        self.stabilizer * (1.0 - clamp01(speed / (speed_max * SLOW_SHARE)))
                    }
                }
            }
        }
    }

    /// 速度による調整・はらい・後補正のどれかを使うか（使わないブラシは、今までと同じ線）。
    pub fn uses_correction(&self) -> bool {
        self.stabilizer_speed.is_some() || self.sweep > 0.0 || !self.post.is_default_setting()
    }
}

/// はらいの進み: 直前の筆の点の筆圧（はらいの後）。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Sweep {
    held: Option<f64>,
}

impl Sweep {
    /// 筆の点の筆圧（`moved` は直前の筆の点からの距離）。はらいが 0 なら `pressure` のまま（何も計算しない）。
    pub(crate) fn pressure(&mut self, assist: &StrokeAssist, pressure: f64, moved: f64) -> f64 {
        if assist.sweep <= 0.0 {
            return pressure;
        }
        let p = match self.held {
            None => pressure,
            Some(held) => f64_max(pressure, held - SWEEP_FALL / assist.sweep * moved),
        };
        self.held = Some(p);
        p
    }
}

/// 後補正の入力の点（2D と 3D の入力の共通の形）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PostPoint {
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
    pub time: f64,
    pub tilt: DVec2,
    pub rotation: f64,
}

fn dist(a: &PostPoint, b: &PostPoint) -> f64 {
    ((b.x - a.x) * (b.x - a.x) + (b.y - a.y) * (b.y - a.y)).sqrt()
}

/// 線分 a→b から点 p までの距離。
fn off_segment(p: &PostPoint, a: &PostPoint, b: &PostPoint) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 <= 0.0 {
        return dist(p, a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    let (qx, qy) = (a.x + dx * t, a.y + dy * t);
    ((p.x - qx) * (p.x - qx) + (p.y - qy) * (p.y - qy)).sqrt()
}

fn lerp_point(a: &PostPoint, b: &PostPoint, t: f64) -> PostPoint {
    PostPoint {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
        pressure: a.pressure + (b.pressure - a.pressure) * t,
        time: a.time + (b.time - a.time) * t,
        tilt: a.tilt + (b.tilt - a.tilt) * t,
        rotation: super::lerp_angle(a.rotation, b.rotation, t),
    }
}

/// 点ごとの許し（速度による調整なら、その点の速さで小さくする）。
fn tolerances(points: &[PostPoint], tolerance: f64, speed_max: Option<f64>) -> Vec<f64> {
    let n = points.len();
    (0..n)
        .map(|i| {
            let Some(max) = speed_max else {
                return tolerance;
            };
            let (a, b) = (&points[i.saturating_sub(1)], &points[(i + 1).min(n - 1)]);
            let dt = b.time - a.time;
            let speed = if dt > 0.0 {
                let path: f64 = (i.saturating_sub(1)..(i + 1).min(n - 1))
                    .map(|k| dist(&points[k], &points[k + 1]))
                    .sum();
                path / dt
            } else {
                0.0
            };
            tolerance * (1.0 - 0.5 * clamp01(speed / max))
        })
        .collect()
}

/// 1 本の区間（Shift の直線の角で区切った入力の点の列）を整えた点の列。始めと終わりの点はそのまま。点が 3 つ未満・許しが 0・
/// 捨てる点が無いときは None（描き直さない）。`speed_max` は速度による調整のときだけ渡す。
///
/// 1. 許しより小さな揺れを捨てる（Ramer–Douglas–Peucker。区間の中でいちばん離れた点が、その点の許しを超えれば残して分ける）。残した点を
///    結ぶ折れ線は、元の点の列から許しより離れない。
/// 2. 折れ線を等しい間隔で刻み、許しと同じ幅の重み（四則だけの核。`exp` などの libm の関数は OS ごとにバイトがずれるので使わない）でならす（端は点で折り返すので、端の点と、まっすぐな辺はそのまま）。
///    折れ線の継ぎ目の折れ（滑らかな曲線を折れ線にした角）は消え、描いた角は許しほどの丸みだけ残る。
/// 3. 筆圧・時刻・傾き・回転は、元の点を折れ線の上の同じ割合の所へ置いて、その間を直線で補う。
pub(crate) fn smooth(
    points: &[PostPoint],
    tolerance: f64,
    speed_max: Option<f64>,
) -> Option<Vec<PostPoint>> {
    let n = points.len();
    if n < 3 || !tolerance.is_finite() || tolerance <= 0.0 {
        return None;
    }
    let tol = tolerances(points, tolerance, speed_max);
    let kept = kept_points(points, &tol);
    if kept.len() == n {
        return None;
    }
    let (along, total) = arc_positions(points, &kept);
    if total.is_nan() || total <= 0.0 {
        return None;
    }
    let (line, step) = resample(points, &kept, &along, total, &tol);
    let mut out = blur(&line, step);
    let last = out.len() - 1;
    out[0] = points[0];
    out[last] = points[n - 1];
    Some(out)
}

/// 手順 1: 許しより小さな揺れを捨てて、残す点の番号（始めと終わりを含む、昇順）を返す。
fn kept_points(points: &[PostPoint], tol: &[f64]) -> Vec<usize> {
    let n = points.len();
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (mut far, mut over) = (a, 0.0);
        for i in a + 1..b {
            let d = off_segment(&points[i], &points[a], &points[b]) - tol[i];
            if d > over {
                over = d;
                far = i;
            }
        }
        if far != a {
            keep[far] = true;
            stack.push((a, far));
            stack.push((far, b));
        }
    }
    (0..n).filter(|&i| keep[i]).collect()
}

/// 元の点の、折れ線の上の位置（弧の長さ）と折れ線の全長。残した点の間では、元の点の列に沿った長さの割合のまま。
fn arc_positions(points: &[PostPoint], kept: &[usize]) -> (Vec<f64>, f64) {
    let n = points.len();
    let mut along = vec![0.0; n];
    let mut base = 0.0;
    for w in kept.windows(2) {
        let (ka, kb) = (w[0], w[1]);
        let chord = dist(&points[ka], &points[kb]);
        let path: f64 = (ka..kb).map(|k| dist(&points[k], &points[k + 1])).sum();
        let mut walked = 0.0;
        for i in ka..kb {
            along[i] = base
                + if path > 0.0 {
                    chord * walked / path
                } else {
                    0.0
                };
            walked += dist(&points[i], &points[i + 1]);
        }
        base += chord;
    }
    along[n - 1] = base;
    (along, base)
}

/// 手順 2 の前半: 折れ線を等しい間隔（許しの半分。0.25〜2 画素、点の数の上限まで）で刻む。刻んだ点（筆圧などは元の点から補う）と
/// その所の許し、刻んだ間隔を返す。
fn resample(
    points: &[PostPoint],
    kept: &[usize],
    along: &[f64],
    total: f64,
    tol: &[f64],
) -> (Vec<(PostPoint, f64)>, f64) {
    let n = points.len();
    let sigma_min = tol.iter().copied().fold(f64::INFINITY, f64::min);
    let step = (sigma_min / 2.0).clamp(MIN_POST_STEP, POST_PIECE_LENGTH);
    let count = ((total / step).ceil() as usize).clamp(2, MAX_POST_POINTS);
    let step = total / count as f64;
    let mut line = Vec::with_capacity(count + 1);
    let (mut seg, mut i) = (0usize, 0usize);
    let mut vertex = vec![0.0; kept.len()];
    for s in 1..kept.len() {
        vertex[s] = vertex[s - 1] + dist(&points[kept[s - 1]], &points[kept[s]]);
    }
    for j in 0..=count {
        let u = if j == count { total } else { step * j as f64 };
        while seg + 2 < kept.len() && u > vertex[seg + 1] {
            seg += 1;
        }
        while i + 2 < n && u > along[i + 1] {
            i += 1;
        }
        let (a, b) = (&points[kept[seg]], &points[kept[seg + 1]]);
        let len = vertex[seg + 1] - vertex[seg];
        let t = if len > 0.0 {
            ((u - vertex[seg]) / len).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let span = along[i + 1] - along[i];
        let f = if span > 0.0 {
            ((u - along[i]) / span).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let mut p = lerp_point(&points[i], &points[i + 1], f);
        p.x = a.x + (b.x - a.x) * t;
        p.y = a.y + (b.y - a.y) * t;
        let sigma = tol[i] + (tol[i + 1] - tol[i]) * f;
        line.push((p, sigma));
    }
    (line, step)
}

/// ならしの重み。許しの 3 倍（`reach`）の外で 0 になる、四則だけの核 `(1 − (d / 3σ)²)³`（トライウェイト。標準偏差は σ）。
/// `exp` などの libm の関数は OS ごとに最後の 1 桁がずれるので使わない（四則と sqrt は IEEE で決まる）。
fn smooth_weight(d: f64, sigma: f64) -> f64 {
    let u = d / (3.0 * sigma);
    let rest = 1.0 - u * u;
    if rest <= 0.0 {
        0.0
    } else {
        rest * rest * rest
    }
}

/// 手順 2 の後半: 刻んだ点を許しと同じ幅の重みでならす（端は点で折り返す: 端の外の点は 2·端 − 内の点。端の点はそのまま）。
fn blur(line: &[(PostPoint, f64)], step: f64) -> Vec<PostPoint> {
    let last = line.len() - 1;
    let at = |k: isize| -> (f64, f64) {
        if k < 0 {
            let (e, q) = (line[0].0, line[((-k) as usize).min(last)].0);
            (2.0 * e.x - q.x, 2.0 * e.y - q.y)
        } else if k as usize > last {
            let (e, q) = (line[last].0, line[(2 * last).saturating_sub(k as usize)].0);
            (2.0 * e.x - q.x, 2.0 * e.y - q.y)
        } else {
            (line[k as usize].0.x, line[k as usize].0.y)
        }
    };
    let mut out = Vec::with_capacity(line.len());
    for (j, &(p, sigma)) in line.iter().enumerate() {
        if j == 0 || j == last {
            out.push(p);
            continue;
        }
        let reach = ((3.0 * sigma / step).ceil() as isize).max(1);
        let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
        for k in -reach..=reach {
            let w = smooth_weight(k as f64 * step, sigma);
            let (x, y) = at(j as isize + k);
            sx += w * x;
            sy += w * y;
            sw += w;
        }
        out.push(PostPoint {
            x: sx / sw,
            y: sy / sw,
            ..p
        });
    }
    out
}

/// 入力の点の列（Shift の直線の角 `breaks` で区切る。値はその角の後の最初の点の番号）を区間ごとに整えた、描き直す入力の列と、区間の終わりで
/// 角にするか。どの区間も変わらなければ None。区間は、前の区間の最後の点から始める（その点は前の区間が描いたので、描き直しの列には入れない）。
pub(crate) fn plan(
    points: &[PostPoint],
    breaks: &[usize],
    tolerance: f64,
    speed_max: Option<f64>,
) -> Option<Vec<(Vec<PostPoint>, bool)>> {
    let mut bounds: Vec<usize> = breaks
        .iter()
        .copied()
        .filter(|&b| b > 0 && b < points.len())
        .collect();
    bounds.dedup();
    let mut segments = Vec::with_capacity(bounds.len() + 1);
    let mut changed = false;
    let mut start = 0;
    let ends: Vec<(usize, bool)> = bounds
        .iter()
        .map(|&b| (b, true))
        .chain(std::iter::once((
            points.len(),
            breaks.contains(&points.len()),
        )))
        .collect();
    for (end, corner) in ends {
        if end <= start {
            continue;
        }
        // 前の区間の最後の点を、この区間の始まりの点にして整える
        let from = start.saturating_sub(1);
        let span = &points[from..end];
        let skip = start - from;
        let points_out = match smooth(span, tolerance, speed_max) {
            Some(s) => {
                changed = true;
                s.into_iter().skip(skip).collect()
            }
            None => span[skip..].to_vec(),
        };
        segments.push((points_out, corner));
        start = end;
    }
    changed.then_some(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64, t: f64) -> PostPoint {
        PostPoint {
            x,
            y,
            pressure: 1.0,
            time: t,
            tilt: DVec2::ZERO,
            rotation: 0.0,
        }
    }

    #[test]
    fn small_wobbles_are_dropped_and_the_ends_stay() {
        let points: Vec<PostPoint> = (0..=50)
            .map(|i| {
                p(
                    i as f64 * 2.0,
                    if i % 2 == 0 { 0.3 } else { -0.3 },
                    i as f64,
                )
            })
            .collect();
        let out = smooth(&points, 1.0, None).expect("揺れを捨てる");
        assert_eq!((out[0].x, out[0].y), (points[0].x, points[0].y));
        let last = out.last().unwrap();
        assert_eq!((last.x, last.y), (100.0, 0.3));
        // 端と端を結ぶ線の上にそろう
        for q in &out {
            let line = 0.3 * q.x / 100.0;
            assert!((q.y - line).abs() < 0.32, "{q:?}");
        }
        // 揺れより小さな許しなら全部の点が残り、描き直さない
        assert!(smooth(&points, 0.1, None).is_none());
    }

    #[test]
    fn corners_beyond_the_tolerance_are_kept() {
        let points = [p(0.0, 0.0, 0.0), p(50.0, 50.0, 1.0), p(100.0, 0.0, 2.0)];
        assert!(smooth(&points, 1.0, None).is_none(), "角は残る");
        let points = [
            p(0.0, 0.0, 0.0),
            p(25.0, 25.2, 0.5),
            p(50.0, 50.0, 1.0),
            p(75.0, 25.0, 1.5),
            p(100.0, 0.0, 2.0),
        ];
        let out = smooth(&points, 1.0, None).unwrap();
        // 角は許しほどの丸みだけ（角の点から許しの内）
        let top = out.iter().map(|q| q.y).fold(f64::MIN, f64::max);
        assert!(top > 49.0 && top <= 50.0, "{top}");
    }

    #[test]
    fn a_zigzag_keeps_straight_sides_and_its_corners() {
        let vertices = [(0.0, 0.0), (50.0, 40.0), (100.0, 0.0), (150.0, 40.0)];
        let mut points = Vec::new();
        for w in vertices.windows(2) {
            for k in 0..10 {
                let t = k as f64 / 10.0;
                let wobble = if k % 2 == 0 { 0.2 } else { -0.2 };
                points.push(p(
                    w[0].0 + (w[1].0 - w[0].0) * t,
                    w[0].1 + (w[1].1 - w[0].1) * t + wobble,
                    points.len() as f64,
                ));
            }
        }
        points.push(p(150.0, 40.0, points.len() as f64));
        let out = smooth(&points, 1.0, None).unwrap();
        for q in &out {
            // どの点も、いちばん近い辺から許しより離れない（辺が曲がって膨らまない）
            let d = vertices
                .windows(2)
                .map(|w| {
                    let (a, b) = (p(w[0].0, w[0].1, 0.0), p(w[1].0, w[1].1, 0.0));
                    off_segment(q, &a, &b)
                })
                .fold(f64::INFINITY, f64::min);
            assert!(d < 1.0, "{q:?} {d}");
        }
    }

    #[test]
    fn the_smoothing_weight_uses_only_basic_arithmetic() {
        // 中心は 1、許しの 3 倍で 0、その外も 0。値は四則だけで決まる（(1 − (d/3σ)²)³）ので、OS によらず同じバイト
        assert_eq!(smooth_weight(0.0, 2.0), 1.0);
        assert_eq!(smooth_weight(6.0, 2.0), 0.0);
        assert_eq!(smooth_weight(-9.0, 2.0), 0.0);
        assert_eq!(smooth_weight(3.0, 2.0), 0.75 * 0.75 * 0.75);
        assert_eq!(smooth_weight(3.0, 2.0), smooth_weight(-3.0, 2.0));
        // 外へ向かって単調に減る
        let mut last = 1.0;
        for k in 1..=12 {
            let w = smooth_weight(k as f64 * 0.5, 2.0);
            assert!(w < last);
            last = w;
        }
    }

    #[test]
    fn speed_shrinks_the_tolerance() {
        let slow: Vec<PostPoint> = (0..=20)
            .map(|i| p(i as f64, if i % 2 == 0 { 0.4 } else { -0.4 }, i as f64))
            .collect();
        // 止まった速さでは許し 1 のまま捨てる
        assert!(smooth(&slow, 1.0, Some(3000.0)).is_some());
        // 速さの上限では半分の許し（0.5 < 0.8 の揺れ）で、捨てない
        let fast: Vec<PostPoint> = slow
            .iter()
            .map(|q| PostPoint {
                time: q.time / 10000.0,
                ..*q
            })
            .collect();
        assert!(smooth(&fast, 1.0, Some(3000.0)).is_none());
        assert!(smooth(&fast, 1.0, None).is_some());
    }

    #[test]
    fn the_view_scale_converts_the_tolerance() {
        let post = PostCorrection {
            on: true,
            strength: 10.0,
            view_scale: 2.0,
            ..PostCorrection::default()
        };
        assert_eq!(post.tolerance(false), 0.5);
        assert_eq!(post.tolerance(true), 1.0);
        let fixed = PostCorrection {
            by_view_scale: false,
            ..post
        };
        assert_eq!(fixed.tolerance(false), 1.0);
        assert_eq!(fixed.tolerance(true), 2.0);
        assert!(post.active() && !PostCorrection::default().active());
        assert!(PostCorrection {
            view_scale: 3.0,
            ..PostCorrection::default()
        }
        .is_default_setting());
    }

    #[test]
    fn the_stabilizer_length_follows_the_speed_type() {
        let mut a = StrokeAssist {
            stabilizer: 20.0,
            ..StrokeAssist::default()
        };
        assert_eq!(a.stabilizer_length(1000.0, 3000.0), 20.0);
        a.stabilizer_speed = Some(StabilizerSpeed::WeakenWhenFast);
        assert_eq!(a.stabilizer_length(0.0, 3000.0), 20.0);
        assert_eq!(a.stabilizer_length(3000.0, 3000.0), 10.0);
        assert_eq!(a.stabilizer_length(1500.0, 3000.0), 15.0);
        a.stabilizer_speed = Some(StabilizerSpeed::SlowOnly);
        assert_eq!(a.stabilizer_length(0.0, 3000.0), 20.0);
        assert_eq!(a.stabilizer_length(300.0, 3000.0), 10.0);
        assert_eq!(a.stabilizer_length(600.0, 3000.0), 0.0);
        assert_eq!(a.stabilizer_length(6000.0, 3000.0), 0.0);
    }

    #[test]
    fn the_sweep_limits_how_fast_pressure_falls() {
        let a = StrokeAssist {
            sweep: 1.0,
            ..StrokeAssist::default()
        };
        let mut s = Sweep::default();
        assert_eq!(s.pressure(&a, 1.0, 0.0), 1.0);
        let p = s.pressure(&a, 0.0, 100.0);
        assert!((p - (1.0 - SWEEP_FALL * 100.0)).abs() < 1e-12);
        // 上がるときはそのまま
        assert_eq!(s.pressure(&a, 1.0, 1.0), 1.0);
        // はらい 0 は何もしない
        let mut s = Sweep::default();
        let none = StrokeAssist::default();
        assert_eq!(s.pressure(&none, 1.0, 0.0), 1.0);
        assert_eq!(s.pressure(&none, 0.0, 1.0), 0.0);
        // 半分なら下がり方は 2 倍まで
        let half = StrokeAssist { sweep: 0.5, ..a };
        let mut s = Sweep::default();
        s.pressure(&half, 1.0, 0.0);
        let p = s.pressure(&half, 0.0, 100.0);
        assert!((p - (1.0 - 2.0 * SWEEP_FALL * 100.0)).abs() < 1e-12);
    }

    #[test]
    fn segments_start_from_the_previous_corner() {
        let mut points: Vec<PostPoint> = (0..=10)
            .map(|i| {
                p(
                    i as f64 * 3.0,
                    if i % 2 == 0 { 0.2 } else { -0.2 },
                    i as f64,
                )
            })
            .collect();
        points.push(p(30.0, 30.0, 11.0));
        let out = plan(&points, &[11], 1.0, None).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out[0].1, "角で区切る");
        assert!(!out[1].1);
        assert_eq!(
            out[1].0,
            vec![points[11]],
            "2 点の区間はそのまま、前の点は入れない"
        );
        let first = &out[0].0;
        assert_eq!(first[0], points[0]);
        assert!(plan(&points, &[11], 0.01, None).is_none());
    }
}
