//! 2D のキャンバスの定規のツール: 押した所に編集できる定規のつまみ（`HANDLE_POINTS` の内）があればそのつまみをドラッグ、無ければ新しい定規を引く。
//! ドラッグの間は形だけを重ね（文書を変えない）、離したとき 1 回の取り消しで作る・動かす。Esc・フォーカスの喪失・ツールの切り替えは
//! `AppState::drafting_cancel` が捨てる。

use egui::{Modifiers, Pos2};
use yolu_core::glam::DVec2;
use yolu_core::{LayerId, Ruler, RulerKind, RulerPlace};

use super::active::canvas_points;
use super::ops::new_ruler;
use super::{Grab, Handle, RulerAction, RulerDrag, RulerState, HANDLE_POINTS};
use crate::canvas::view::CanvasView;
use crate::state::{Action, AppState, StrokeSource};

/// 種類ごとの、つまみの位置（文書の座標）。
pub fn handles(r: &Ruler) -> Vec<(Handle, DVec2)> {
    let Some((a, b)) = canvas_points(r) else {
        return Vec::new();
    };
    match r.kind {
        // 直線・平行線は、2 点の真ん中に全体を動かすつまみも付く
        RulerKind::Line | RulerKind::Parallel => {
            vec![(Handle::A, a), (Handle::B, b), (Handle::Mid, (a + b) * 0.5)]
        }
        RulerKind::Perspective if !r.two_points => vec![(Handle::A, a)],
        _ => vec![(Handle::A, a), (Handle::B, b)],
    }
}

/// つまみを動かしたあとの 2 点（`delta` は押した所からの動き。向きを決めるつまみ B は、向きの丸め `round` を通す）。
fn moved_points(
    r: &Ruler,
    handle: Handle,
    delta: DVec2,
    round: &dyn Fn(DVec2, DVec2) -> DVec2,
) -> (DVec2, DVec2) {
    let (a, b) = canvas_points(r).unwrap_or((DVec2::ZERO, DVec2::X));
    let whole = (a + delta, b + delta);
    match (r.kind, handle) {
        (RulerKind::Line, Handle::A) => (a + delta, b),
        (RulerKind::Line, Handle::B) => (a, round(a, b + delta)),
        (_, Handle::Mid) => whole,
        // 平行線・同心円・対称の A は中心（向きや半径はそのまま）、1 点のパースも消失点ごと
        (RulerKind::Perspective, Handle::A) if r.two_points => (a + delta, b),
        (_, Handle::A) => whole,
        (RulerKind::Perspective, Handle::B) => (a, b + delta),
        (_, Handle::B) => (a, round(a, b + delta)),
    }
}

/// 向き `a`→`b` を、Shift なら 45 度、角度の刻みが入っていれば刻みの倍数へ丸めた `b`（長さは保つ）。
pub fn round_direction(a: DVec2, b: DVec2, shift: bool, step: Option<u32>) -> DVec2 {
    let d = b - a;
    let length = d.length();
    if length < 1e-9 {
        return b;
    }
    let quantum = if shift {
        45.0
    } else if let Some(step) = step {
        step as f64
    } else {
        return b;
    };
    let angle = (d.y.atan2(d.x).to_degrees() / quantum).round() * quantum;
    let radians = angle.to_radians();
    // 軸の向きは正確な値で
    let (sin, cos) = match angle.rem_euclid(360.0) {
        0.0 => (0.0, 1.0),
        90.0 => (1.0, 0.0),
        180.0 => (0.0, -1.0),
        270.0 => (-1.0, 0.0),
        _ => radians.sin_cos(),
    };
    a + DVec2::new(cos, sin) * length
}

/// ドラッグの今の定規（新しいものも、動かしている既存のものも）。
pub fn dragged_ruler(state: &RulerState, drag: &RulerDrag) -> Ruler {
    let step = state.angle_step.then_some(state.step);
    let round = |a: DVec2, b: DVec2| round_direction(a, b, drag.shift, step);
    match &drag.grab {
        Some(grab) => {
            let (a, b) = moved_points(
                &grab.original,
                grab.handle,
                drag.current - drag.start,
                &round,
            );
            let mut r = grab.original.clone();
            r.place = RulerPlace::Canvas { a, b };
            r
        }
        None => new_ruler(state, drag.start, round(drag.start, drag.current)),
    }
}

/// 押した所に掴めるつまみがあるか（編集できる定規のつまみのうち、画面で `HANDLE_POINTS` の内の一番近いもの。選んでいる定規が先）。
fn grab_at(app: &AppState, view: &CanvasView, pos: Pos2) -> Option<Grab> {
    let mut best: Option<(f32, bool, Grab)> = None;
    for shown in app.rulers_shown(super::Place::Canvas) {
        if !shown.editable {
            continue;
        }
        for (handle, point) in handles(shown.ruler) {
            let distance = view.to_screen(point.x, point.y).distance(pos);
            if distance >= HANDLE_POINTS {
                continue;
            }
            let better = best.as_ref().is_none_or(|(near, was_selected, _)| {
                distance < *near - 0.5
                    || (distance < *near + 0.5 && shown.selected && !was_selected)
            });
            if better {
                best = Some((
                    distance,
                    shown.selected,
                    Grab {
                        owner: shown.owner,
                        original: shown.ruler.clone(),
                        handle,
                    },
                ));
            }
        }
    }
    best.map(|(_, _, grab)| grab)
}

/// 押した（定規のツール）。
pub fn press(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource, m: Modifiers) {
    if app.is_stroking() || app.rulers.drag.is_some() {
        return;
    }
    let (x, y) = view.to_canvas(pos);
    let p = DVec2::new(x, y);
    let grab = grab_at(app, view, pos);
    if let Some(g) = &grab {
        app.rulers.selected = Some((g.owner, g.original.id));
    }
    app.rulers.drag = Some(RulerDrag {
        source,
        start: p,
        current: p,
        shift: m.shift,
        grab,
    });
}

/// 動いた（この入力で始めたドラッグだけ）。
pub fn moved(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource, m: Modifiers) {
    if let Some(d) = app.rulers.drag.as_mut().filter(|d| d.source == source) {
        let (x, y) = view.to_canvas(pos);
        d.current = DVec2::new(x, y);
        d.shift = m.shift;
    }
}

/// 離した: 新しい定規を作る・つまみを動かす（引いた長さが短すぎる・動かしていないときは何もしない）。
pub fn release(
    app: &mut AppState,
    view: &CanvasView,
    pos: Pos2,
    source: StrokeSource,
    m: Modifiers,
) {
    if app.rulers.drag.as_ref().is_none_or(|d| d.source != source) {
        return;
    }
    moved(app, view, pos, source, m);
    let Some(drag) = app.rulers.drag.take() else {
        return;
    };
    let ruler = dragged_ruler(&app.rulers, &drag);
    if ruler.validate().is_err() {
        return;
    }
    match drag.grab {
        Some(grab) => {
            if ruler != grab.original {
                app.apply(Action::Ruler(RulerAction::Replace {
                    owner: grab.owner,
                    ruler,
                    coalesce: false,
                }));
            }
        }
        None => app.apply(Action::Ruler(RulerAction::Create(ruler))),
    }
}

/// 掴んでいる既存の定規の持ち主とつまみ（描画が、掴んでいる定規を動かした形に替える）。
pub fn grabbed(app: &AppState) -> Option<(LayerId, Ruler)> {
    let drag = app.rulers.drag.as_ref()?;
    let grab = drag.grab.as_ref()?;
    Some((grab.owner, dragged_ruler(&app.rulers, drag)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::RulerId;

    fn id() -> RulerId {
        RulerId(1)
    }

    #[test]
    fn directions_round_to_the_step_and_shift_wins() {
        let a = DVec2::new(10.0, 10.0);
        let b = a + DVec2::new(10.0, 3.0); // 約 16.7 度
        let r = round_direction(a, b, false, Some(15));
        assert!(
            ((r - a).length() - (b - a).length()).abs() < 1e-9,
            "長さは保つ"
        );
        let angle = (r - a).y.atan2((r - a).x).to_degrees();
        assert!((angle - 15.0).abs() < 1e-9, "{angle}");
        assert_eq!(
            round_direction(a, b, false, None),
            b,
            "刻みが切なら丸めない"
        );
        let shifted = round_direction(a, b, true, Some(15));
        assert!(
            (shifted - a).y.abs() < 1e-9 && (shifted - a).x > 0.0,
            "Shift は 45 度刻み"
        );
        // 軸の向きは正確な値
        let up = round_direction(a, a + DVec2::new(0.4, 9.0), false, Some(15));
        assert_eq!(up.x, a.x);
    }

    #[test]
    fn handles_follow_the_kind() {
        let line = Ruler::canvas(id(), RulerKind::Line, DVec2::ZERO, DVec2::new(10.0, 0.0));
        let kinds: Vec<Handle> = handles(&line).into_iter().map(|h| h.0).collect();
        assert_eq!(kinds, [Handle::A, Handle::B, Handle::Mid]);
        // 平行線にも真ん中のつまみ（全体を動かす）。同心円・対称は中心 A と向き・半径 B だけ
        let parallel = Ruler::canvas(
            id(),
            RulerKind::Parallel,
            DVec2::ZERO,
            DVec2::new(10.0, 0.0),
        );
        assert_eq!(
            handles(&parallel),
            [
                (Handle::A, DVec2::ZERO),
                (Handle::B, DVec2::new(10.0, 0.0)),
                (Handle::Mid, DVec2::new(5.0, 0.0))
            ]
        );
        for kind in [RulerKind::Concentric, RulerKind::Symmetry] {
            let r = Ruler::canvas(id(), kind, DVec2::ZERO, DVec2::new(10.0, 0.0));
            let kinds: Vec<Handle> = handles(&r).into_iter().map(|h| h.0).collect();
            assert_eq!(kinds, [Handle::A, Handle::B], "{kind:?}");
        }
        let mut persp = Ruler::canvas(id(), RulerKind::Perspective, DVec2::ZERO, DVec2::X * 5.0);
        assert_eq!(handles(&persp).len(), 1, "1 点のパースは消失点だけ");
        persp.two_points = true;
        assert_eq!(handles(&persp).len(), 2);
        let model = Ruler::model(
            id(),
            RulerKind::Line,
            yolu_core::glam::DVec3::ZERO,
            yolu_core::glam::DVec3::X,
            yolu_core::glam::DVec3::Z,
        );
        assert!(
            handles(&model).is_empty(),
            "3D の定規は 2D のつまみを持たない"
        );
    }

    #[test]
    fn moving_a_handle_changes_the_points_the_kind_says() {
        let round = |_: DVec2, b: DVec2| b;
        let d = DVec2::new(3.0, 4.0);
        let (a, b) = (DVec2::new(10.0, 10.0), DVec2::new(30.0, 10.0));
        let line = Ruler::canvas(id(), RulerKind::Line, a, b);
        assert_eq!(moved_points(&line, Handle::A, d, &round), (a + d, b));
        assert_eq!(moved_points(&line, Handle::B, d, &round), (a, b + d));
        assert_eq!(moved_points(&line, Handle::Mid, d, &round), (a + d, b + d));
        let parallel = Ruler::canvas(id(), RulerKind::Parallel, a, b);
        assert_eq!(
            moved_points(&parallel, Handle::Mid, d, &round),
            (a + d, b + d),
            "平行線の真ん中は全体"
        );
        for kind in [
            RulerKind::Parallel,
            RulerKind::Concentric,
            RulerKind::Symmetry,
        ] {
            let r = Ruler::canvas(id(), kind, a, b);
            assert_eq!(
                moved_points(&r, Handle::A, d, &round),
                (a + d, b + d),
                "{kind:?} の A は全体"
            );
            assert_eq!(
                moved_points(&r, Handle::B, d, &round),
                (a, b + d),
                "{kind:?} の B"
            );
        }
        let mut persp = Ruler::canvas(id(), RulerKind::Perspective, a, b);
        assert_eq!(
            moved_points(&persp, Handle::A, d, &round),
            (a + d, b + d),
            "1 点は全体"
        );
        persp.two_points = true;
        assert_eq!(moved_points(&persp, Handle::A, d, &round), (a + d, b));
        assert_eq!(moved_points(&persp, Handle::B, d, &round), (a, b + d));
    }
}
