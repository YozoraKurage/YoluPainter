use super::{Figure, RulerKind};
use crate::{
    state::{AppState, Tool},
    ui::{
        theme as t,
        widgets::{self as w, NumberFormat, SliderSpec},
    },
};
use egui::{pos2, vec2, Rect, Ui};

pub fn snap_button(ui: &mut Ui, app: &mut AppState, at: Rect) {
    if w::icon_button(
        ui,
        at,
        "drafting.snap",
        "grid_dots",
        app.lang
            .pick("定規にスナップ（Ctrl+1）", "Snap to Ruler (Ctrl+1)"),
        app.drafting.snap,
        !app.is_stroking(),
        20.0,
    )
    .clicked()
    {
        app.toggle_snap();
    }
}

pub fn options(ui: &mut Ui, app: &mut AppState, r: Rect, mut x: f32) {
    let lang = app.lang;
    let enabled = !app.is_stroking();
    let mut button = |ui: &mut Ui, id: &str, label: &str, selected: bool| {
        let width = w::text_width(ui.painter(), label, t::LABEL) + 22.0;
        let at = Rect::from_min_size(pos2(x, r.top() + 6.0), vec2(width, r.height() - 12.0));
        x += width + 3.0;
        w::button(ui, at, id, label, selected, enabled, None, None).clicked()
    };
    if app.tool == Tool::Shape {
        for (f, id, name) in [
            (Figure::Line, "line", lang.pick("直線", "Line")),
            (
                Figure::Rectangle,
                "rectangle",
                lang.pick("長方形", "Rectangle"),
            ),
            (Figure::Ellipse, "ellipse", lang.pick("楕円", "Ellipse")),
        ] {
            if button(ui, id, name, app.drafting.figure == f) {
                app.drafting.figure = f;
                if f == Figure::Line {
                    app.drafting.fill = false;
                }
            }
        }
        if button(
            ui,
            "drafting.outline",
            lang.pick("線で描く", "Outline"),
            !app.drafting.fill,
        ) {
            app.drafting.fill = false;
        }
        if app.drafting.figure != Figure::Line
            && button(
                ui,
                "drafting.fill",
                lang.pick("塗る", "Fill"),
                app.drafting.fill,
            )
        {
            app.drafting.fill = true;
        }
    } else {
        for (kind, id, name) in [
            (
                RulerKind::Line,
                "ruler.line",
                lang.pick("直線定規", "Straight Ruler"),
            ),
            (
                RulerKind::Parallel,
                "ruler.parallel",
                lang.pick("平行線", "Parallel"),
            ),
            (
                RulerKind::Concentric,
                "ruler.circle",
                lang.pick("同心円", "Concentric"),
            ),
            (
                RulerKind::Perspective,
                "ruler.perspective",
                lang.pick("パース", "Perspective"),
            ),
        ] {
            if button(ui, id, name, app.drafting.ruler_kind == kind) {
                app.drafting.ruler_kind = kind;
                if let Some(r) = app.drafting.rulers.get_mut(&app.doc.id()) {
                    r.kind = kind;
                }
            }
        }
        if app.drafting.ruler_kind == RulerKind::Perspective {
            for (two, id, name) in [
                (false, "ruler.one", lang.pick("1 点", "1 Point")),
                (true, "ruler.two", lang.pick("2 点", "2 Points")),
            ] {
                if button(ui, id, name, app.drafting.two_points == two) {
                    app.drafting.two_points = two;
                    if let Some(r) = app.drafting.rulers.get_mut(&app.doc.id()) {
                        r.two_points = two;
                    }
                }
            }
        }
        if button(ui, "ruler.delete", lang.pick("削除", "Delete"), false) {
            app.drafting.rulers.remove(&app.doc.id());
        }
    }
    if app.tool == Tool::Shape && app.drafting.figure == Figure::Rectangle {
        let at = Rect::from_min_size(pos2(x, r.top() + 6.0), vec2(150.0, r.height() - 12.0));
        let out = w::slider(
            ui,
            at,
            "drafting.corner",
            app.drafting.corner,
            &SliderSpec::new(
                lang.pick("角の丸み", "Corner Radius"),
                0.0,
                256.0,
                NumberFormat::int(" px"),
            ),
        );
        if enabled && out.changed {
            app.drafting.corner = out.value;
        }
        x += 158.0;
    }
    snap_button(
        ui,
        app,
        Rect::from_min_size(pos2(x, r.top() + 6.0), vec2(28.0, r.height() - 12.0)),
    );
}
