//! グラデーションのツールのキャンバスの入力（押す・動く・離す・Esc）と、ドラッグ中の線の表示。形はキャンバスの座標（左下が原点）で決まり、
//! 表示を回している・反転しているときは回って見える。ストロークの最中・読むだけのセットでは始めない。フォーカスを失う・Esc・ツールの
//! 切り替えで、途中の線は何も塗らずに捨てる（取り残さない）。

use egui::{Color32, Painter, Pos2, Shape, Stroke};

use super::{GradientDrag, GradientOp};
use crate::canvas::view::CanvasView;
use crate::notice::Source;
use crate::state::{Action, AppState, StrokeSource, Tool};

/// 押した（ペン・マウス）。
pub fn press(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource) {
    if app.is_stroking()
        || app.tool != Tool::Gradient
        || app.gradient.drag.is_some()
        || app.view3d.input.draft.is_some()
    {
        return;
    }
    if let Some(reason) = app.read_only_reason().map(str::to_owned) {
        app.refuse(
            Source::Gradient,
            crate::lang::refusals::read_only_set(app.lang, &reason),
        );
        return;
    }
    let canvas = view.to_canvas(pos);
    app.gradient.drag = Some(GradientDrag {
        source,
        start: canvas,
        current: canvas,
        start_screen: pos,
    });
}

/// ポインタが動いた。
pub fn moved(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource) {
    let Some(drag) = app.gradient.drag.as_mut() else {
        return;
    };
    if drag.source == source {
        drag.current = view.to_canvas(pos);
    }
}

/// 離した: 始点から終点へ塗る（動かしていなければ何もしない）。
pub fn release(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource) {
    if app
        .gradient
        .drag
        .as_ref()
        .is_none_or(|d| d.source != source)
    {
        return;
    }
    let Some(drag) = app.gradient.drag.take() else {
        return;
    };
    let end = view.to_canvas(pos);
    app.apply(Action::Gradient(GradientOp::Apply {
        start: drag.start,
        end,
    }));
}

/// ペン（Windows Ink）の 1 点。触れた・動いた・離したを、押す・動く・離すにする。
pub fn pen_sample(
    app: &mut AppState,
    view: &CanvasView,
    pos: Pos2,
    pointer_id: u32,
    contact: bool,
) {
    let source = StrokeSource::Pen(pointer_id);
    match (contact, app.gradient.pen_down) {
        (true, None) => {
            app.gradient.pen_down = Some(pointer_id);
            press(app, view, pos, source);
        }
        (true, Some(id)) if id == pointer_id => moved(app, view, pos, source),
        (false, Some(id)) if id == pointer_id => {
            app.gradient.pen_down = None;
            release(app, view, pos, source);
        }
        _ => {}
    }
}

/// Esc: 途中の線を捨てる。何かあったか。
pub fn cancel(app: &mut AppState) -> bool {
    let any = app.gradient_cancel_drag();
    if any {
        app.info(
            Source::Gradient,
            app.lang
                .pick("グラデーションをやめました。", "Gradient cancelled."),
        );
    }
    any
}

/// ドラッグ中の線（始点から今の点まで。黒の縁取りと白、両端の点）。
pub fn paint_overlay(painter: &Painter, view: &CanvasView, app: &AppState) {
    if app.tool != Tool::Gradient {
        return;
    }
    let Some(drag) = &app.gradient.drag else {
        return;
    };
    let a = view.to_screen(drag.start.0, drag.start.1);
    let b = view.to_screen(drag.current.0, drag.current.1);
    paint_line(painter, a, b);
}

/// ドラッグ中の始点から今の点までの線（2D のキャンバスと 3D ビューで同じ見た目）。
pub(crate) fn paint_line(painter: &Painter, a: Pos2, b: Pos2) {
    painter.add(Shape::line_segment(
        [a, b],
        Stroke::new(3.0, Color32::from_black_alpha(140)),
    ));
    painter.add(Shape::line_segment(
        [a, b],
        Stroke::new(1.2, Color32::from_white_alpha(230)),
    ));
    for p in [a, b] {
        painter.circle_filled(p, 3.5, Color32::WHITE);
        painter.circle_stroke(p, 3.5, Stroke::new(1.0, Color32::from_black_alpha(180)));
    }
}
