//! 曲線とランプ（値のカーブ・グラデーションの分岐点）。

use egui::{pos2, vec2, Rect, Ui};
use yolu_app::ui::curve::{self, CurveStyle};
use yolu_app::ui::ramp::{self, Selection};
use yolu_app::ui::theme as t;
use yolu_core::curve::Curve;
use yolu_core::generator::Ramp;

use super::{remember, store, Entry, Section};
use crate::page::SketchCtx;

pub fn section() -> Section {
    Section {
        title: ("曲線とランプ", "Curves and ramps"),
        entries: vec![
            Entry::new(
                "curve::curve_editor",
                ("値のカーブの編集", "Value curve editor"),
                134.0,
                curve_editor,
            ),
            Entry::new(
                "curve::curve_editor_with",
                (
                    "色・分布・斜め線つきのカーブ",
                    "Curve with color, backdrop, diagonal",
                ),
                134.0,
                curve_editor_styled,
            ),
            Entry::new(
                "ramp::stops_editor",
                ("グラデーションの分岐点", "Gradient stops"),
                94.0,
                stops_editor,
            ),
            Entry::new(
                "ramp::curve_editor",
                ("ランプの値のカーブ", "Ramp value curve"),
                134.0,
                ramp_curve,
            ),
        ],
    }
}

fn area(sample: Rect, height: f32) -> Rect {
    Rect::from_min_size(
        pos2(sample.left() + 10.0, sample.top() + 9.0),
        vec2(sample.width() - 20.0, height),
    )
}

fn curve_editor(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let current = remember(ui, "curve", Curve::identity);
    if let Some(next) = curve::curve_editor(
        ui,
        area(sample, curve::HEIGHT),
        "curve",
        &current,
        ctx.pick("カーブ", "Curve"),
        true,
    ) {
        store(ui, "curve", next);
    }
}

fn curve_editor_styled(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let current = remember(ui, "curve", Curve::identity);
    let backdrop: Vec<f32> = (0..48)
        .map(|i| {
            let x = i as f32 / 47.0;
            (-((x - 0.45) * 4.0).powi(2)).exp()
        })
        .collect();
    let style = CurveStyle {
        line: Some(t::WARNING),
        backdrop: Some(&backdrop),
        diagonal: true,
    };
    if let Some(next) = curve::curve_editor_with(
        ui,
        area(sample, curve::HEIGHT),
        "styled",
        &current,
        &style,
        ctx.pick("カーブ", "Curve"),
        true,
    ) {
        store(ui, "curve", next);
    }
}

fn stops_editor(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let current = remember(ui, "ramp", Ramp::default);
    let mut selection = remember(ui, "selection", Selection::default);
    if let Some(next) = ramp::stops_editor(
        ui,
        area(sample, ramp::STOPS_HEIGHT),
        "stops",
        &current,
        &mut selection,
        false,
        ctx.pick("グラデーション", "Gradient"),
        true,
    ) {
        store(ui, "ramp", next);
    }
    store(ui, "selection", selection);
}

fn ramp_curve(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let current = remember(ui, "ramp", Ramp::default);
    if let Some(next) = ramp::curve_editor(
        ui,
        area(sample, ramp::CURVE_HEIGHT),
        "ramp-curve",
        &current,
        ctx.pick("カーブ", "Curve"),
        true,
    ) {
        store(ui, "ramp", next);
    }
}
