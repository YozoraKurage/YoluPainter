//! プロパティの欄（Substance Painter の並び）: 頭にタブ（ブラシ｜アルファ｜ステンシル｜マテリアル）、選んだタブの中身だけを出す。
//! ブラシのタブは直径・流量・不透明度（2 行のスライダーの右にペンのボタン = 筆圧で変えるか）と間隔、アルファのタブは硬さ
//! （Unity 版と同じく、硬さは先端の形の側）。ステンシルとマテリアルは M2 以降。

use egui::{pos2, vec2, Color32, Rect, Ui};

use crate::state::{AppState, BrushState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};

pub const TABS: [&str; 4] = ["ブラシ", "アルファ", "ステンシル", "マテリアル"];
pub const TAB_ICONS: [&str; 4] = ["paint_brush", "shapes", "square", "layers"];

/// 筆圧に従わせられるスライダー（2 行目の右にペンのボタン）。
#[allow(clippy::too_many_arguments)]
fn pen_slider(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    spec: SliderSpec,
    shown: f32,
    pressure: &mut bool,
    pen_tooltip: &str,
) -> Option<f32> {
    const PEN: f32 = 24.0;
    let row = rows.slider_row();
    let out = w::slider(ui, row, id, shown, &spec.inset(PEN + 4.0));
    let button = Rect::from_min_size(
        pos2(row.right() - PEN, row.bottom() - 20.0),
        vec2(PEN, 20.0),
    );
    if w::icon_button(
        ui,
        button,
        (id, "pen"),
        "stylus",
        pen_tooltip,
        *pressure,
        true,
        15.0,
    )
    .clicked()
    {
        *pressure = !*pressure;
    }
    out.changed.then_some(out.value)
}

fn section(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    key: &'static str,
    title: &str,
    icon: &str,
    reset: Option<&str>,
) -> (bool, bool) {
    let open = app.section_open(key, true);
    let header = rows.full_row(t::PANEL_HEADER_HEIGHT, 5.0);
    let out = w::section_header(ui, header, ("section", key), title, open, Some(icon), reset);
    if out.open != open {
        app.sections.insert(key, out.open);
    }
    (out.open, out.reset)
}

fn placeholder(ui: &mut Ui, rows: &mut Rows, text: &str) {
    let r = rows.row(48.0, 4.0);
    w::wrapped_text(ui.painter(), r, text, t::LABEL_DIM);
}

pub fn show(ui: &mut Ui, app: &mut AppState) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let strip = Rect::from_min_size(r.min, vec2(r.width(), t::PROPERTY_TAB_STRIP_HEIGHT));
    app.property_tab = w::tab_strip(
        ui,
        strip,
        "props.tabs",
        &TABS,
        &TAB_ICONS,
        app.property_tab.min(TABS.len() - 1),
    );
    let body = Rect::from_min_max(pos2(r.left(), strip.bottom()), r.max);
    let mut rows = Rows::new(body, 0.0);
    match app.property_tab {
        0 => {
            let (open, reset) = section(
                ui,
                app,
                &mut rows,
                "brush",
                "ブラシ",
                "paint_brush",
                Some("ブラシの既定の値に戻す"),
            );
            if reset {
                let hardness = app.brush.hardness;
                app.brush = BrushState {
                    hardness,
                    ..BrushState::default()
                };
            }
            if open {
                rows.indent = t::SECTION_INDENT;
                let b = &mut app.brush;
                let spec = SliderSpec::new("直径", 1.0, 256.0, NumberFormat::int(" px"))
                    .tooltip("ブラシの直径（[ と ]）");
                if let Some(v) = pen_slider(
                    ui,
                    &mut rows,
                    "brush.size",
                    spec,
                    b.radius * 2.0,
                    &mut b.pressure_size,
                    "筆圧で直径を変える",
                ) {
                    b.radius = (v / 2.0).max(0.5);
                }
                let spec = SliderSpec::new("流量", 0.0, 100.0, NumberFormat::int("%"))
                    .tooltip("ダブ 1 つが足す量");
                if let Some(v) = pen_slider(
                    ui,
                    &mut rows,
                    "brush.flow",
                    spec,
                    b.flow * 100.0,
                    &mut b.pressure_flow,
                    "筆圧で流量を変える",
                ) {
                    b.flow = v / 100.0;
                }
                let spec = SliderSpec::new("不透明度", 0.0, 100.0, NumberFormat::int("%"))
                    .tooltip("1 本のストロークが覆える上限");
                if let Some(v) = pen_slider(
                    ui,
                    &mut rows,
                    "brush.opacity",
                    spec,
                    b.opacity * 100.0,
                    &mut b.pressure_opacity,
                    "筆圧で不透明度を変える",
                ) {
                    b.opacity = v / 100.0;
                }
                let spec = SliderSpec::new("間隔", 1.0, 100.0, NumberFormat::int("%"))
                    .tooltip("ダブの間隔（直径に対する割合）");
                let out = w::slider(
                    ui,
                    rows.slider_row(),
                    "brush.spacing",
                    b.spacing * 100.0,
                    &spec,
                );
                if out.changed {
                    b.spacing = out.value / 100.0;
                }
            }
        }
        1 => {
            let (open, reset) = section(
                ui,
                app,
                &mut rows,
                "alpha",
                "アルファ",
                "shapes",
                Some("先端の形の既定の値に戻す"),
            );
            if reset {
                app.brush.hardness = BrushState::default().hardness;
            }
            if open {
                rows.indent = t::SECTION_INDENT;
                let spec = SliderSpec::new("硬さ", 0.0, 100.0, NumberFormat::int("%"))
                    .tooltip("丸い先端の縁の硬さ");
                let out = w::slider(
                    ui,
                    rows.slider_row(),
                    "brush.hardness",
                    app.brush.hardness * 100.0,
                    &spec,
                );
                if out.changed {
                    app.brush.hardness = out.value / 100.0;
                }
                // 先端の形の見本（中心から縁へ、硬さのとおりに薄くなる）
                let preview = rows.row(72.0, 4.0);
                tip_preview(ui, preview, app.brush.hardness);
            }
        }
        2 => {
            let _ = section(ui, app, &mut rows, "stencil", "ステンシル", "square", None);
            placeholder(ui, &mut rows, "ステンシルは準備中です（M2 以降）。");
        }
        _ => {
            let _ = section(ui, app, &mut rows, "material", "マテリアル", "layers", None);
            placeholder(ui, &mut rows, "マテリアルで塗るのは準備中です（M2 以降）。");
        }
    }
}

/// 丸い先端の覆いの見本（仮の core と同じ形: 半径の硬さ倍までは 1、そこから縁へなめらかに 0）。
fn tip_preview(ui: &mut Ui, r: Rect, hardness: f32) {
    let p = ui.painter();
    w::rounded(p, r, t::CONTROL_BG, 3.0);
    w::outline(p, r, t::BORDER, 1.0, 3.0);
    let radius = r.height() * 0.5 - 6.0;
    let center = r.center();
    const STEPS: usize = 24;
    let mut previous = 0.0;
    for i in (1..=STEPS).rev() {
        let d = i as f32 / STEPS as f32;
        let coverage = if d <= hardness || hardness >= 1.0 {
            1.0
        } else {
            let t = ((d - hardness) / (1.0 - hardness)).clamp(0.0, 1.0);
            1.0 - t * t * (3.0 - 2.0 * t)
        };
        // 外の輪から順に重ねるので、その輪の覆いになるように足りない分だけ足す
        let add = ((coverage - previous) / (1.0 - previous).max(1e-3)).clamp(0.0, 1.0);
        previous = previous + (1.0 - previous) * add;
        p.circle_filled(
            center,
            radius * d,
            Color32::from_white_alpha(w::to_byte(add)),
        );
    }
}
