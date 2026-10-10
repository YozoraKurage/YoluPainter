//! 図形の欄: オプションバー（図形の種類・線か塗り、スナップ）と、左のドックのツールプロパティ（図形の線か塗り・角の丸み・直径と不透明度）。
//! 図形の種類はサブツールの一覧（`subtool`）でも選べる。2D のキャンバスと 3D ビューで同じ欄。定規のツールの欄は `rulers::tool`。

use super::Figure;
use crate::panels::properties::{choice_buttons, slider_row, ChoiceButton};
use crate::{
    state::AppState,
    ui::{
        theme as t,
        widgets::{self as w, NumberFormat, Rows},
    },
};
use egui::{pos2, vec2, Rect, Ui};

/// 図形のオプションバー: 図形の種類・線か塗り、スナップ 2 つ。定規のツールのオプションバーは `rulers::tool::options`。
pub fn options(ui: &mut Ui, app: &mut AppState, r: Rect, mut x: f32) {
    let lang = app.lang;
    let enabled = !app.is_stroking();
    let mut button = |ui: &mut Ui, id: &str, label: &str, selected: bool| {
        let width = w::text_width(ui.painter(), label, t::LABEL) + 22.0;
        let at = Rect::from_min_size(pos2(x, r.top() + 6.0), vec2(width, r.height() - 12.0));
        x += width + 3.0;
        w::button(ui, at, id, label, selected, enabled, None, None).clicked()
    };
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
    let at = |x: f32| Rect::from_min_size(pos2(x, r.top() + 6.0), vec2(28.0, r.height() - 12.0));
    crate::rulers::tool::snap_buttons(ui, app, at(x), at(x + 31.0));
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
