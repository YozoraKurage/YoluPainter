//! 2D の図形と定規。定規は文書 ID ごとのセッション状態で、保存形式には含めない。
pub mod canvas;
pub mod props;

use crate::state::{AppState, StrokeSource};
use std::collections::HashMap;
use yolu_core::glam::DVec2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Figure {
    #[default]
    Line,
    Rectangle,
    Ellipse,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RulerKind {
    #[default]
    Line,
    Parallel,
    Concentric,
    Perspective,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ruler {
    pub kind: RulerKind,
    pub a: DVec2,
    pub b: DVec2,
    pub two_points: bool,
}

/// ストロークの開始位置から凍結する寄せ先。パースの二つの候補は最初の移動の向きで決める。
#[derive(Clone, Copy, Debug, PartialEq)]
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
}

fn direction(v: DVec2) -> DVec2 {
    v.try_normalize().unwrap_or(DVec2::X)
}

impl Ruler {
    /// 二つの端点が離れている定規だけ置く。長さが無いと向きが X に落ちて、意図しない水平の寄せ先になる（図形と同じ 0.01 以下は捨てる）。
    pub fn is_placeable(&self) -> bool {
        self.a.distance(self.b) > 0.01
    }

    pub fn constraint(self, start: DVec2) -> Constraint {
        match self.kind {
            RulerKind::Line => Constraint::Line {
                origin: self.a,
                direction: direction(self.b - self.a),
            },
            RulerKind::Parallel => Constraint::Line {
                origin: start,
                direction: direction(self.b - self.a),
            },
            RulerKind::Concentric => Constraint::Circle {
                center: self.a,
                radius: start.distance(self.a),
                start,
            },
            RulerKind::Perspective => Constraint::Perspective {
                start,
                a: self.a,
                b: self.two_points.then_some(self.b),
            },
        }
    }
}

impl Constraint {
    pub fn project(&mut self, point: DVec2) -> DVec2 {
        match *self {
            Self::Line { origin, direction } => {
                origin + direction * (point - origin).dot(direction)
            }
            Self::Circle {
                center,
                radius,
                start,
            } => (point - center)
                .try_normalize()
                .map_or(start, |d| center + d * radius),
            Self::Perspective { start, a, b } => {
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

#[derive(Clone, Copy, Debug)]
pub struct Drag {
    pub source: StrokeSource,
    pub start: DVec2,
    pub current: DVec2,
    pub shift: bool,
    pub alt: bool,
    pub ruler: bool,
    /// 既存の定規は端点か中心を動かせる。線から始めた場合は全体を移動。
    pub original: Option<Ruler>,
    pub handle: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Drafting {
    pub figure: Figure,
    pub fill: bool,
    pub corner: f32,
    pub ruler_kind: RulerKind,
    pub two_points: bool,
    pub snap: bool,
    pub rulers: HashMap<u128, Ruler>,
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
    pub fn drafting_cancel(&mut self) -> bool {
        // ペンの接触の札は離すまで残す。Esc の後の接触点で形を作り直さない。
        self.drafting.drag.take().is_some()
    }

    pub fn ruler(&self) -> Option<Ruler> {
        self.drafting.rulers.get(&self.doc.id()).copied()
    }

    pub fn toggle_snap(&mut self) {
        if !self.is_stroking() {
            self.drafting.snap = !self.drafting.snap;
        }
    }
}
