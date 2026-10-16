//! 本物のパネルがよく使う並び（`panels/` の今のコードの組み立てを、同じ部品で描き直した物）。下書きは、ここから並びを選んで使う。

use egui::{pos2, vec2, Rect, Ui};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, NumberFormat, SliderSpec};

use super::{rows, Entry, Section};
use crate::page::SketchCtx;
use crate::sketches::layers_common::{self as parts, ROWS, ROW_HEIGHT, TOOLBAR_HEIGHT};

pub fn section() -> Section {
    Section {
        title: ("パネルで使う並び", "Arrangements the panels use"),
        entries: vec![
            Entry::new(
                "panels/layers.rs: blend and opacity row",
                (
                    "合成方法と不透明度を 1 行に",
                    "Blend mode and opacity in one row",
                ),
                44.0,
                blend_and_opacity,
            ),
            Entry::new(
                "panels/layers.rs: layer_row",
                (
                    "目・サムネイル・名前・ロックの行",
                    "Row of eye, thumbnail, name, lock",
                ),
                3.0 * ROW_HEIGHT + 12.0,
                layer_rows,
            ),
            Entry::new(
                "panels/layers.rs: toolbar",
                ("下の操作の帯", "Bottom strip of icon buttons"),
                TOOLBAR_HEIGHT + 12.0,
                toolbar,
            ),
            Entry::new(
                "panels/properties.rs: section",
                (
                    "見出しの下に入力部品を積む",
                    "Input parts stacked under a header",
                ),
                176.0,
                property_section,
            ),
        ],
    }
}

fn blend_and_opacity(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let top = Rect::from_min_size(
        pos2(sample.left() + t::PADDING, sample.top() + 10.0),
        vec2(sample.width() - 2.0 * t::PADDING, 24.0),
    );
    let left_w = ((top.width() - 6.0) * 0.42).floor();
    let blend = Rect::from_min_size(top.min, vec2(left_w, top.height()));
    let opacity = Rect::from_min_max(pos2(top.left() + left_w + 6.0, top.top()), top.max);
    let _ = w::dropdown(
        ui,
        blend,
        "blend",
        None,
        ctx.pick("通常", "Normal"),
        None,
        true,
        0.0,
    );
    let value = ctx.values.float("gallery.parts.opacity", 100.0);
    let spec = SliderSpec::new(
        ctx.pick("不透明度", "Opacity"),
        0.0,
        100.0,
        NumberFormat::int("%"),
    );
    let out = w::slider(ui, opacity, "opacity", value, &spec);
    if out.changed {
        ctx.values.set_float("gallery.parts.opacity", out.value);
    }
}

fn layer_rows(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let list = sample.shrink2(vec2(0.0, 6.0));
    w::fill(ui.painter(), list, t::CONTROL_BG);
    for (i, d) in ROWS.iter().take(3).enumerate() {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + i as f32 * ROW_HEIGHT),
            vec2(list.width(), ROW_HEIGHT),
        );
        parts::row(ui, ctx, list, row, i, d, 26.0);
    }
}

fn toolbar(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let bar = Rect::from_min_size(
        pos2(sample.left(), sample.top() + 6.0),
        vec2(sample.width(), TOOLBAR_HEIGHT),
    );
    parts::toolbar(ui, ctx, bar);
}

fn property_section(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let mut rows = rows(sample);
    let band = Rect::from_min_size(sample.min, vec2(sample.width(), 24.0));
    let open = ctx.values.bool("gallery.parts.section", true);
    let out = w::section_header(
        ui,
        band,
        "section",
        ctx.pick("ブラシ", "Brush"),
        open,
        Some("paint_brush"),
        None,
    );
    ctx.values.set_bool("gallery.parts.section", out.open);
    rows.space(30.0);
    if !out.open {
        return;
    }
    for (key, ja, en, default) in [
        ("size", "サイズ", "Size", 24.0),
        ("flow", "流量", "Flow", 80.0),
    ] {
        let r = rows.slider_row();
        let name = format!("gallery.parts.{key}");
        let value = ctx.values.float(&name, default);
        let spec = SliderSpec::new(
            ctx.pick(ja, en),
            0.0,
            100.0,
            NumberFormat::int(if key == "size" { " px" } else { "%" }),
        );
        let out = w::slider(ui, r, key, value, &spec);
        if out.changed {
            ctx.values.set_float(&name, out.value);
        }
    }
    let r = rows.row(22.0, 4.0);
    let on = ctx.values.bool("gallery.parts.smooth", true);
    let next = w::toggle(
        ui,
        r,
        "smooth",
        ctx.pick("手ぶれ補正", "Stabilizer"),
        on,
        None,
        true,
    );
    ctx.values.set_bool("gallery.parts.smooth", next);
    let r = rows.row(24.0, 0.0);
    let _ = w::dropdown(
        ui,
        r,
        "mode",
        Some(ctx.pick("描き方", "Mode")),
        ctx.pick("通常", "Normal"),
        None,
        true,
        0.0,
    );
}
