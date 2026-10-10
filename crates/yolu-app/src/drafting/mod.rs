//! 図形と、寄せ先の式。2D の定規はレイヤーが持つ文書の値で `rulers`、3D ビューの図形と、画面の上に貼り付く定規（3D の定規が文書に入るまで）は
//! `view3d::draft`。ここの `Ruler`・`RulerKind` は、画面に貼り付く 3D の定規と、寄せ先の式（`Ruler::constraint`・`Constraint::project`。
//! 2D の定規もこの式を通る）の形。点・寄せ先・輪郭の式は 2D と 3D で同じ（3D は表示域の画面の点で測る）。
pub mod canvas;
pub mod props;

use crate::state::{AppState, StrokeSource};
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

/// 定規を引く・動かすドラッグの今の定規。original があれば、handle（1・2 は端点、0 は全体）を動かす。無ければ start から current への
/// 新しい定規（Shift で 45 度刻み）。
pub fn dragged_ruler(
    original: Option<Ruler>,
    handle: usize,
    (start, current): (DVec2, DVec2),
    shift: bool,
    kind: RulerKind,
    two_points: bool,
) -> Ruler {
    if let Some(mut r) = original {
        match handle {
            1 => r.a = current,
            2 => r.b = current,
            _ => {
                r.a += current - start;
                r.b += current - start;
            }
        }
        r
    } else {
        let (a, b) = endpoints(start, current, Figure::Line, shift, false);
        Ruler {
            kind,
            a,
            b,
            two_points,
        }
    }
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

    /// 画面に貼り付く 3D の定規の種類（ツールで選んでいる種類のうち、これに当たるもの。対称は 3D の定規が文書に入るまで画面に貼り付けない）。
    pub fn screen_ruler_kind(&self) -> Option<RulerKind> {
        match self.rulers.kind {
            yolu_core::RulerKind::Line => Some(RulerKind::Line),
            yolu_core::RulerKind::Parallel => Some(RulerKind::Parallel),
            yolu_core::RulerKind::Concentric => Some(RulerKind::Concentric),
            yolu_core::RulerKind::Perspective => Some(RulerKind::Perspective),
            yolu_core::RulerKind::Symmetry => None,
        }
    }

    /// 定規を消す: 選んでいる定規（文書の定規）。無ければ、3D ビューの画面に貼り付く定規。
    pub fn delete_rulers(&mut self) {
        if let Some((owner, ruler)) = self.selected_ruler().map(|(o, r)| (o, r.id)) {
            self.apply(crate::state::Action::Ruler(
                crate::rulers::RulerAction::Delete {
                    owner,
                    ids: vec![ruler],
                },
            ));
        } else {
            self.view3d.ruler = None;
        }
    }

    /// 消せる定規があるか（選んでいる定規か、3D ビューの画面に貼り付く定規）。
    pub fn has_ruler(&self) -> bool {
        self.selected_ruler().is_some() || self.view3d.ruler.is_some()
    }
}
