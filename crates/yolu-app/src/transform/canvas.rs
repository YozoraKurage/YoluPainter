//! 移動・変形の道具のキャンバスの入力（押す・動く・離す・Esc・Enter・矢印キー）と、キャンバスの上の表示（動かすものの外枠とハンドル、
//! ドラッグ中の変形後の外枠）。ドラッグの間は文書を変えず、離したところ（か Enter）で `Edit::Transform` を 1 回当てる。

use egui::{Color32, CursorIcon, Modifiers, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use super::{arrow_to_canvas, collapses, handle_points, handles_usable, hit, Bounds, Drag, Mode};
use crate::canvas::view::CanvasView;
use crate::layerops::Xform;
use crate::m2::Edit;
use crate::state::{Action, AppState, StrokeSource, Tool};

impl AppState {
    /// 動かすものの範囲（文書の版・動かす層が変わるまで覚える）。
    pub fn transform_bounds_cached(&mut self) -> Option<Bounds> {
        let ids = self.transform_targets();
        let revision = self.doc.revision();
        if let Some((r, cached, bounds)) = &self.transform.cache {
            if *r == revision && *cached == ids {
                return *bounds;
            }
        }
        let bounds = self.transform_bounds();
        self.transform.cache = Some((revision, ids, bounds));
        bounds
    }

    /// 道具を替えたとき・窓がフォーカスを失ったとき: 途中のドラッグは何も変えずに捨てる。
    pub fn transform_cancel_drag(&mut self) -> bool {
        self.transform.advanced.draft = None;
        self.transform.advanced.session = None;
        self.transform.pen_down = None;
        self.transform.drag.take().is_some()
    }
}

/// 押した（ペン・マウス）。
pub fn press(
    app: &mut AppState,
    view: &CanvasView,
    pos: Pos2,
    source: StrokeSource,
    modifiers: Modifiers,
) {
    if app.is_stroking()
        || !matches!(app.tool, Tool::Move | Tool::Liquify)
        || app.transform.drag.is_some()
    {
        return;
    }
    if let Some(reason) = app.read_only_reason().map(str::to_owned) {
        app.message = format!(
            "{}: {reason}",
            app.lang
                .pick("読むだけのテクスチャセットです", "Read-only texture set")
        );
        return;
    }
    let Some(bounds) = super::advanced::interaction_bounds(app) else {
        app.message = if app.transform_targets().is_empty() {
            app.lang.pick(
                "動かす画素のあるレイヤーがありません。",
                "No layer with pixels to move.",
            )
        } else if app.doc.selection().is_some() {
            app.lang.pick(
                "選択範囲の中に動かす画素がありません。",
                "No pixels to move inside the selection.",
            )
        } else {
            app.lang
                .pick("動かす画素がありません。", "No pixels to move.")
        }
        .into();
        return;
    };
    super::advanced::press(app, view, pos, bounds, modifiers);
    let canvas = view.to_canvas(pos);
    app.transform.drag = Some(Drag {
        mode: hit(view, bounds, pos),
        source,
        bounds,
        start: canvas,
        current: canvas,
        shift: modifiers.shift,
    });
}

/// ポインタが動いた。
pub fn moved(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource, shift: bool) {
    super::advanced::moved(app, view, pos, source);
    let canvas = view.to_canvas(pos);
    if let Some(drag) = app.transform.drag.as_mut() {
        if drag.source == source {
            drag.current = canvas;
            drag.shift = shift;
        }
    }
}

/// 離した。その位置で確定する。
pub fn release(
    app: &mut AppState,
    view: &CanvasView,
    pos: Pos2,
    source: StrokeSource,
    shift: bool,
) {
    if app
        .transform
        .drag
        .as_ref()
        .is_none_or(|d| d.source != source)
    {
        return;
    }
    moved(app, view, pos, source, shift);
    commit(app);
}

/// ドラッグを今の位置で確定する（離した・Enter）。動かしていない・何も変わらない変形は当てない。
pub fn commit(app: &mut AppState) {
    if super::advanced::commit(app) {
        return;
    }
    let Some(drag) = app.transform.drag.take() else {
        return;
    };
    let xform = match drag.mode {
        Mode::Move => {
            let (dx, dy) = drag.delta();
            if (dx, dy) == (0, 0) {
                return;
            }
            Xform::Move { dx, dy }
        }
        _ => {
            let t = drag.transform();
            if t == yolu_core::Affine2D::IDENTITY {
                return;
            }
            if collapses(&t) {
                app.message = app
                    .lang
                    .pick(
                        "潰れてしまうので変形しません。",
                        "That would collapse the layer.",
                    )
                    .into();
                return;
            }
            Xform::Affine(t)
        }
    };
    app.apply(Action::M2(Edit::Transform(xform)));
}

/// Esc: ドラッグを何も変えずにやめる。何かあったか。ペンで押している間のペンの番号は残す（残りの動きを新しいドラッグにしない。離したら外れる）。
pub fn cancel(app: &mut AppState) -> bool {
    app.transform.advanced.draft = None;
    let any = app.transform.drag.take().is_some();
    if any {
        app.message = app
            .lang
            .pick("変形をやめました。", "Transform cancelled.")
            .into();
    }
    any
}

/// ペン（Windows Ink）の 1 点。触れた・動いた・離したを、押す・動く・離すにする。
pub fn pen_sample(
    app: &mut AppState,
    view: &CanvasView,
    pos: Pos2,
    pointer_id: u32,
    contact: bool,
    modifiers: Modifiers,
) {
    let source = StrokeSource::Pen(pointer_id);
    match (contact, app.transform.pen_down) {
        (true, None) => {
            app.transform.pen_down = Some(pointer_id);
            press(app, view, pos, source, modifiers);
        }
        (true, Some(id)) if id == pointer_id => moved(app, view, pos, source, modifiers.shift),
        (false, Some(id)) if id == pointer_id => {
            app.transform.pen_down = None;
            release(app, view, pos, source, modifiers.shift);
        }
        _ => {}
    }
}

/// 矢印キー: 1 画素（Shift で 10）。`screen` は画面の向き（右・下が正）。表示を回していても画面の向きに動く。
pub fn arrow(app: &mut AppState, screen: (f64, f64), shift: bool) {
    let Some(rect) = app.ui.canvas_rect else {
        return;
    };
    let view = app.view.view(rect, app.doc.width(), app.doc.height());
    let (dx, dy) = arrow_to_canvas(&view, screen);
    let step = if shift { 10 } else { 1 };
    app.apply(Action::M2(Edit::Transform(Xform::Move {
        dx: dx * step,
        dy: dy * step,
    })));
}

/// ポインタの下のカーソル（ドラッグ中はドラッグの種類）。
pub fn cursor(app: &mut AppState, view: &CanvasView, hover: Option<Pos2>) -> CursorIcon {
    if app.tool == Tool::Liquify {
        return CursorIcon::Crosshair;
    }
    let mode = match (&app.transform.drag, hover) {
        (Some(drag), _) => drag.mode,
        (None, Some(p)) => match app.transform_bounds_cached() {
            Some(b) => hit(view, b, p),
            None => return CursorIcon::NotAllowed,
        },
        (None, None) => return CursorIcon::Default,
    };
    match mode {
        Mode::Move => CursorIcon::Move,
        Mode::Rotate => CursorIcon::Crosshair,
        Mode::Scale {
            anchor,
            handle,
            axes,
        } => match axes {
            1 => CursorIcon::ResizeHorizontal,
            2 => CursorIcon::ResizeVertical,
            _ if (handle.0 - anchor.0) * (handle.1 - anchor.1) > 0.0 => CursorIcon::ResizeNeSw,
            _ => CursorIcon::ResizeNwSe,
        },
    }
}

// ───────── 表示 ─────────

pub(super) fn outline(painter: &Painter, points: Vec<Pos2>) {
    painter.add(Shape::line(
        points.clone(),
        Stroke::new(3.0, Color32::from_black_alpha(140)),
    ));
    painter.add(Shape::line(
        points,
        Stroke::new(1.2, Color32::from_white_alpha(230)),
    ));
}

fn corners(view: &CanvasView, b: Bounds, map: impl Fn((f64, f64)) -> (f64, f64)) -> Vec<Pos2> {
    [(b.0, b.1), (b.2, b.1), (b.2, b.3), (b.0, b.3), (b.0, b.1)]
        .iter()
        .map(|c| {
            let (x, y) = map((c.0 as f64, c.1 as f64));
            view.to_screen(x, y)
        })
        .collect()
}

/// 動かすものの外枠とハンドル（ドラッグ中は変形後の外枠だけ）。移動の道具のときだけ。
pub fn paint_overlay(painter: &Painter, view: &CanvasView, app: &mut AppState) {
    if super::advanced::paint(painter, view, app) {
        return;
    }
    if app.tool != Tool::Move {
        return;
    }
    if let Some(drag) = &app.transform.drag {
        let t = drag.transform();
        outline(painter, corners(view, drag.bounds, |p| t.apply(p.0, p.1)));
        return;
    }
    let Some(b) = app.transform_bounds_cached() else {
        return;
    };
    outline(painter, corners(view, b, |p| p));
    if !handles_usable(view, b) {
        return;
    }
    for (x, y) in handle_points(b) {
        let at = view.to_screen(x, y);
        let r = Rect::from_center_size(at, Vec2::splat(7.0));
        painter.rect_filled(r, 1.0, Color32::from_black_alpha(160));
        painter.rect_filled(r.shrink(1.0), 1.0, Color32::from_white_alpha(240));
    }
}
