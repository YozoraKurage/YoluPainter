//! 図形と、寄せ先の式。定規はレイヤーが持つ文書の値で `rulers`（2D のキャンバスも 3D ビューも、定規から寄せ先を作るのは `rulers`）、3D ビューの図形は
//! `view3d::draft`。ここの `Constraint` は、ストロークの始めに凍結する寄せ先の式（`Constraint::project`）で、点・寄せ先・輪郭の式は 2D と 3D で同じ
//! （3D は表示域の画面の点で測る）。
pub mod canvas;
pub mod props;

use std::sync::Arc;

use crate::state::{AppState, StrokeSource};
use yolu_core::glam::DVec2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Figure {
    #[default]
    Line,
    Rectangle,
    Ellipse,
}

/// ストロークの開始位置から凍結する寄せ先。パースの二つの候補は最初の移動の向きで決める。
#[derive(Clone, Debug, PartialEq)]
pub enum Constraint {
    Line {
        origin: DVec2,
        direction: DVec2,
    },
    Circle {
        center: DVec2,
        radius: f64,
        start: DVec2,
    },
    Perspective {
        start: DVec2,
        a: DVec2,
        b: Option<DVec2>,
    },
    /// 閉じた折れ線（3D の同心円を画面へ写した、256 に分けた楕円）。`segments` は円を回る順の線分で、カメラの後ろへ出た所は None。
    /// `last` は前に寄せた線分の番号（遠さの同じ候補が複数あるとき、前の点の近くを先に取る）。
    Polyline {
        segments: Arc<[Option<(DVec2, DVec2)>]>,
        start: DVec2,
        last: usize,
    },
}

fn direction(v: DVec2) -> DVec2 {
    v.try_normalize().unwrap_or(DVec2::X)
}

/// 点 `p` から線分 `a`–`b` の一番近い点。
fn nearest_on_segment(a: DVec2, b: DVec2, p: DVec2) -> DVec2 {
    let ab = b - a;
    let length_squared = ab.length_squared();
    if length_squared < 1e-18 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / length_squared).clamp(0.0, 1.0)
}

/// 前の点の近くを先に取る範囲（線分の数の割合。256 分割なら前後 16 本）と、その近くの点が一番近い点よりどれだけ遠くても前の点の近くを取るか（画面の点）。
const POLYLINE_LOCAL: usize = 16;
const POLYLINE_MARGIN_POINTS: f64 = 1.5;

/// 円を回る順の番号 `a`・`b`（`n` 個の輪）の間の隔たり。
fn ring_gap(a: usize, b: usize, n: usize) -> usize {
    let d = a.abs_diff(b);
    d.min(n - d)
}

impl Constraint {
    /// 閉じた折れ線への寄せ先（`start` に一番近い線分から始める）。線分が 1 つも無ければ None。
    pub fn polyline(segments: Vec<Option<(DVec2, DVec2)>>, start: DVec2) -> Option<Constraint> {
        let (_, last, _) = nearest_segment(&segments, start, None)?;
        Some(Constraint::Polyline {
            segments: segments.into(),
            start,
            last,
        })
    }
}

/// 点 `p` に一番近い線分（遠さ・番号・線分の上の点）。前の線分 `last` があれば、その前後 `POLYLINE_LOCAL` 本の中の一番近い点を先に取り、
/// 全体で一番近い点より `POLYLINE_MARGIN_POINTS` 以上遠くなければそちらにする（円の中を動いても、反対側へ飛ばない）。
fn nearest_segment(
    segments: &[Option<(DVec2, DVec2)>],
    p: DVec2,
    last: Option<usize>,
) -> Option<(f64, usize, DVec2)> {
    let candidates = segments.iter().enumerate().filter_map(|(i, s)| {
        let (a, b) = (*s)?;
        let q = nearest_on_segment(a, b, p);
        Some((q.distance(p), i, q))
    });
    let n = segments.len();
    let best = candidates.clone().min_by(|a, b| a.0.total_cmp(&b.0))?;
    let local = last.and_then(|last| {
        let near: Vec<(f64, usize, DVec2)> = candidates
            .clone()
            .filter(|c| ring_gap(c.1, last, n) <= POLYLINE_LOCAL)
            .collect();
        let dmin = near.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
        // 前の近くの候補のうち、一番近い遠さ（同じ遠さの内は、前の線分に近い方）
        near.into_iter()
            .filter(|c| c.0 <= dmin * (1.0 + 1e-6) + 1e-9)
            .min_by_key(|c| ring_gap(c.1, last, n))
    });
    match local {
        Some(l) if l.0 <= best.0 + POLYLINE_MARGIN_POINTS => Some(l),
        _ => Some(best),
    }
}

impl Constraint {
    pub fn project(&mut self, point: DVec2) -> DVec2 {
        match self {
            &mut Self::Line { origin, direction } => {
                origin + direction * (point - origin).dot(direction)
            }
            &mut Self::Circle {
                center,
                radius,
                start,
            } => (point - center)
                .try_normalize()
                .map_or(start, |d| center + d * radius),
            Self::Polyline {
                segments,
                start,
                last,
            } => match nearest_segment(segments, point, Some(*last)) {
                Some((_, index, at)) => {
                    *last = index;
                    at
                }
                None => *start,
            },
            &mut Self::Perspective { start, a, b } => {
                let motion = point - start;
                if motion.length_squared() < 1e-8 {
                    return start;
                }
                let da = direction(a - start);
                let db = b.map(|b| direction(b - start));
                let d = db
                    .filter(|db| motion.dot(*db).abs() > motion.dot(da).abs())
                    .unwrap_or(da);
                *self = Self::Line {
                    origin: start,
                    direction: d,
                };
                self.project(point)
            }
        }
    }
}

/// 図形のドラッグの途中。
#[derive(Clone, Copy, Debug)]
pub struct Drag {
    pub source: StrokeSource,
    pub start: DVec2,
    pub current: DVec2,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Drafting {
    pub figure: Figure,
    pub fill: bool,
    pub corner: f32,
    pub drag: Option<Drag>,
    pub pen_down: Option<u32>,
}

pub fn endpoints(a: DVec2, mut b: DVec2, figure: Figure, shift: bool, alt: bool) -> (DVec2, DVec2) {
    let mut d = b - a;
    if shift {
        if figure == Figure::Line {
            let angle = (d.y.atan2(d.x) / std::f64::consts::FRAC_PI_4).round()
                * std::f64::consts::FRAC_PI_4;
            d = DVec2::new(angle.cos(), angle.sin()) * d.length();
        } else {
            let side = d.abs().max_element();
            d = DVec2::new(
                if d.x < 0.0 { -side } else { side },
                if d.y < 0.0 { -side } else { side },
            );
        }
        b = a + d;
    }
    (if alt { a - d } else { a }, b)
}

/// プレビューと描画に同じ点列を使う。曲線は 1/4 画素以下の弦の誤差を目安に分割し、上限を設ける。
pub fn outline(figure: Figure, a: DVec2, b: DVec2, corner: f64) -> Vec<DVec2> {
    if figure == Figure::Line {
        return vec![a, b];
    }
    let lo = a.min(b);
    let hi = a.max(b);
    let half = (hi - lo) * 0.5;
    if figure == Figure::Ellipse {
        let n = ((std::f64::consts::PI * (half.max_element() / 0.5).sqrt()).ceil() as usize)
            .clamp(16, 2048);
        return (0..=n)
            .map(|i| {
                let t = i as f64 * std::f64::consts::TAU / n as f64;
                (lo + hi) * 0.5 + DVec2::new(t.cos(), t.sin()) * half
            })
            .collect();
    }
    let radius = corner.max(0.0).min(half.min_element());
    if radius == 0.0 {
        return vec![lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y), lo];
    }
    let n = ((radius / 0.5).sqrt().ceil() as usize).clamp(4, 512);
    let mut points = Vec::with_capacity(4 * (n + 1) + 1);
    for (k, center) in [
        DVec2::new(hi.x - radius, hi.y - radius),
        DVec2::new(lo.x + radius, hi.y - radius),
        DVec2::new(lo.x + radius, lo.y + radius),
        DVec2::new(hi.x - radius, lo.y + radius),
    ]
    .into_iter()
    .enumerate()
    {
        for i in 0..=n {
            let t = (k as f64 + i as f64 / n as f64) * std::f64::consts::FRAC_PI_2;
            points.push(center + DVec2::new(t.cos(), t.sin()) * radius);
        }
    }
    points.push(points[0]);
    points
}

impl AppState {
    /// 図形と定規のドラッグをやめる（2D のキャンバスと 3D ビューの両方）。何かあったか。
    pub fn drafting_cancel(&mut self) -> bool {
        let surface = self
            .view3d
            .input
            .draft
            .take_if(|d| d.kind != crate::view3d::draft::DraftKind::Gradient)
            .is_some();
        // レイヤーの一覧で持っている定規のアイコンも、落とさずにやめる（キャンバスが隠れても一覧のドラッグは続くので、キャンバスだけの取りやめには入れない）
        let icon = self.rulers.icon_drag.take().is_some();
        self.drafting_cancel_canvas() || surface || icon
    }

    /// 2D のキャンバスの図形と定規のドラッグだけをやめる（キャンバスが隠れたとき）。
    pub fn drafting_cancel_canvas(&mut self) -> bool {
        // ペンの接触の札は離すまで残す。Esc の後の接触点で形を作り直さない。
        let shape = self.drafting.drag.take().is_some();
        let ruler = self.rulers.drag.take().is_some();
        shape || ruler
    }

    /// 定規を消す: 選んでいる定規（2D でも 3D でも文書の定規）。
    pub fn delete_rulers(&mut self) {
        if let Some((owner, ruler)) = self.selected_ruler().map(|(o, r)| (o, r.id)) {
            self.apply(crate::state::Action::Ruler(
                crate::rulers::RulerAction::Delete {
                    owner,
                    ids: vec![ruler],
                },
            ));
        }
    }

    /// 消せる定規があるか（選んでいる定規）。
    pub fn has_ruler(&self) -> bool {
        self.selected_ruler().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 中心 (0, 0)・半径 100 の閉じた折れ線（`n` 分割）。
    fn circle(n: usize) -> Vec<Option<(DVec2, DVec2)>> {
        let at = |k: usize| {
            let t = k as f64 / n as f64 * std::f64::consts::TAU;
            DVec2::new(t.cos(), t.sin()) * 100.0
        };
        (0..n).map(|k| Some((at(k), at(k + 1)))).collect()
    }

    #[test]
    fn a_polyline_takes_the_nearest_point_of_the_closed_curve() {
        let mut c = Constraint::polyline(circle(256), DVec2::new(100.0, 0.0)).expect("折れ線");
        let p = c.project(DVec2::new(200.0, 0.1));
        assert!((p - DVec2::new(100.0, 0.0)).length() < 0.1, "{p}");
        // 円の中の点は、遠くの（前の点から離れた）一番近い点へ寄る（256 角形なので、円との差は 1.5 点の内）
        let p = c.project(DVec2::new(0.0, 30.0));
        assert!((p - DVec2::new(0.0, 100.0)).length() < 1.5, "{p}");
        let p = c.project(DVec2::new(-1000.0, 0.0));
        assert!((p - DVec2::new(-100.0, 0.0)).length() < 1.5, "{p}");
    }

    #[test]
    fn a_point_near_equally_far_from_both_sides_stays_near_the_previous_point_until_the_other_side_is_clearly_nearer(
    ) {
        let mut c = Constraint::polyline(circle(256), DVec2::new(100.0, 0.0)).unwrap();
        let first = c.project(DVec2::new(100.0, 0.0));
        // 中心（どの点も同じ遠さ）でも、前の点の近くに居続ける（反対側へ飛ばない）
        let center = c.project(DVec2::ZERO);
        assert!(center.distance(first) < 15.0, "{center}");
        // 中心から少しだけ左（反対側が 0.8 点ほど近い）でも、前の点を含む前後の線分（円の 1/8 ほど）の内にとどまる
        let near = c.project(DVec2::new(-0.4, 0.0));
        assert!(near.distance(first) < 45.0, "{near}");
        // はっきり反対側が近ければ、そちらへ（1.5 点を超える差）
        let jumped = c.project(DVec2::new(-5.0, 0.0));
        assert!(jumped.x < -90.0, "{jumped}");
    }

    #[test]
    fn a_polyline_with_gaps_only_uses_the_segments_that_exist() {
        // カメラの後ろへ出た所（None）は候補にしない
        let mut segments = circle(64);
        for s in segments.iter_mut().take(32) {
            *s = None;
        }
        let mut c = Constraint::polyline(segments, DVec2::new(-100.0, 0.0)).unwrap();
        let p = c.project(DVec2::new(100.0, 0.0));
        assert!(p.y <= 1e-9, "残った下半分の上: {p}");
        assert!(Constraint::polyline(vec![None; 8], DVec2::ZERO).is_none());
    }
}
