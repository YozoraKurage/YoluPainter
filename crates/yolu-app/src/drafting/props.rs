//! 図形と定規の欄: オプションバー（図形の種類・線か塗り、定規の種類・点の数・削除、スナップ）と、左のドックのツールプロパティ（図形の線か塗り・
//! 角の丸み・直径と不透明度、定規の点の数・削除・スナップ）。図形の種類と定規の種類はサブツールの一覧（`subtool`）でも選べる。

use super::{Figure, RulerKind};
use crate::panels::properties::{choice_buttons, slider_row, toggle_row, ChoiceButton};
use crate::{
    state::{Action, AppState, Tool},
    ui::{
        theme as t,
        widgets::{self as w, NumberFormat, Rows},
    },
};
use egui::{pos2, vec2, Rect, Ui};

pub fn snap_button(ui: &mut Ui, app: &mut AppState, at: Rect) {
    if w::icon_button(
        ui,
        at,
        "drafting.snap",
        "grid_dots",
        &crate::shortcuts::tip_with_key(
            app.lang,
            app.lang.pick("定規にスナップ", "Snap to Ruler"),
            &Action::ToggleRulerSnap,
        ),
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
    snap_button(
        ui,
        app,
        Rect::from_min_size(pos2(x, r.top() + 6.0), vec2(28.0, r.height() - 12.0)),
    );
}

/// 図形のツールプロパティ: 図形の種類・線か塗り・角の丸み（長方形）と、描くときの直径・不透明度（ブラシと共通）。
pub fn shape_props(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    let lang = app.lang;
    let enabled = !app.is_stroking();
    let figures = [Figure::Line, Figure::Rectangle, Figure::Ellipse];
    let items = [
        ChoiceButton {
            id: "props.line",
            label: lang.pick("直線", "Line"),
            selected: app.drafting.figure == Figure::Line,
            enabled,
            tooltip: None,
        },
        ChoiceButton {
            id: "props.rectangle",
            label: lang.pick("長方形", "Rectangle"),
            selected: app.drafting.figure == Figure::Rectangle,
            enabled,
            tooltip: None,
        },
        ChoiceButton {
            id: "props.ellipse",
            label: lang.pick("楕円", "Ellipse"),
            selected: app.drafting.figure == Figure::Ellipse,
            enabled,
            tooltip: None,
        },
    ];
    if let Some(i) = choice_buttons(ui, rows, &items) {
        app.drafting.figure = figures[i];
        if figures[i] == Figure::Line {
            app.drafting.fill = false;
        }
    }
    if app.drafting.figure != Figure::Line {
        let items = [
            ChoiceButton {
                id: "props.outline",
                label: lang.pick("線で描く", "Outline"),
                selected: !app.drafting.fill,
                enabled,
                tooltip: None,
            },
            ChoiceButton {
                id: "props.fill",
                label: lang.pick("塗る", "Fill"),
                selected: app.drafting.fill,
                enabled,
                tooltip: None,
            },
        ];
        if let Some(i) = choice_buttons(ui, rows, &items) {
            app.drafting.fill = i == 1;
        }
    }
    if app.drafting.figure == Figure::Rectangle {
        if let Some(v) = slider_row(
            ui,
            rows,
            "drafting.corner",
            lang.pick("角の丸み", "Corner Radius"),
            app.drafting.corner,
            (0.0, 256.0),
            NumberFormat::int(" px"),
            None,
            enabled,
        ) {
            app.drafting.corner = v;
        }
    }
    let shared = lang.pick("ブラシと共通", "Shared with the brush");
    if let Some(v) = slider_row(
        ui,
        rows,
        "drafting.size",
        lang.pick("直径", "Size"),
        app.brush.radius * 2.0,
        (1.0, 256.0),
        NumberFormat::int(" px"),
        Some(shared),
        enabled && !(app.drafting.fill && app.drafting.figure != Figure::Line),
    ) {
        app.brush.radius = (v / 2.0).max(0.5);
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "drafting.opacity",
        lang.pick("不透明度", "Opacity"),
        app.brush.opacity * 100.0,
        (0.0, 100.0),
        NumberFormat::int("%"),
        Some(shared),
        enabled,
    ) {
        app.brush.opacity = v / 100.0;
    }
}

/// 定規のツールプロパティ: 定規の種類・パースの点の数・削除・スナップ。
pub fn ruler_props(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    let lang = app.lang;
    let enabled = !app.is_stroking();
    let kinds = [
        RulerKind::Line,
        RulerKind::Parallel,
        RulerKind::Concentric,
        RulerKind::Perspective,
    ];
    let items = [
        ChoiceButton {
            id: "props.ruler.line",
            label: lang.pick("直線定規", "Straight Ruler"),
            selected: app.drafting.ruler_kind == RulerKind::Line,
            enabled,
            tooltip: None,
        },
        ChoiceButton {
            id: "props.ruler.parallel",
            label: lang.pick("平行線", "Parallel"),
            selected: app.drafting.ruler_kind == RulerKind::Parallel,
            enabled,
            tooltip: None,
        },
        ChoiceButton {
            id: "props.ruler.circle",
            label: lang.pick("同心円", "Concentric"),
            selected: app.drafting.ruler_kind == RulerKind::Concentric,
            enabled,
            tooltip: None,
        },
        ChoiceButton {
            id: "props.ruler.perspective",
            label: lang.pick("パース", "Perspective"),
            selected: app.drafting.ruler_kind == RulerKind::Perspective,
            enabled,
            tooltip: None,
        },
    ];
    if let Some(i) = choice_buttons(ui, rows, &items) {
        app.drafting.ruler_kind = kinds[i];
        if let Some(r) = app.drafting.rulers.get_mut(&app.doc.id()) {
            r.kind = kinds[i];
        }
    }
    if app.drafting.ruler_kind == RulerKind::Perspective {
        let items = [
            ChoiceButton {
                id: "props.ruler.one",
                label: lang.pick("1 点", "1 Point"),
                selected: !app.drafting.two_points,
                enabled,
                tooltip: None,
            },
            ChoiceButton {
                id: "props.ruler.two",
                label: lang.pick("2 点", "2 Points"),
                selected: app.drafting.two_points,
                enabled,
                tooltip: None,
            },
        ];
        if let Some(i) = choice_buttons(ui, rows, &items) {
            app.drafting.two_points = i == 1;
            if let Some(r) = app.drafting.rulers.get_mut(&app.doc.id()) {
                r.two_points = i == 1;
            }
        }
    }
    let delete = [ChoiceButton {
        id: "props.ruler.delete",
        label: lang.pick("削除", "Delete"),
        selected: false,
        enabled: enabled && app.ruler().is_some(),
        tooltip: None,
    }];
    if choice_buttons(ui, rows, &delete).is_some() {
        app.drafting.rulers.remove(&app.doc.id());
    }
    if let Some(v) = toggle_row(
        ui,
        rows,
        "props.ruler.snap",
        lang.pick("定規にスナップ", "Snap to Ruler"),
        app.drafting.snap,
        crate::shortcuts::shortcut_text(&Action::ToggleRulerSnap).as_deref(),
        enabled,
    ) {
        if v != app.drafting.snap {
            app.toggle_snap();
        }
    }
}
