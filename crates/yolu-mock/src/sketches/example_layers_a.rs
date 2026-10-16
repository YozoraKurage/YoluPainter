//! レイヤーの欄（今の形を部品で描き直した物）: 上に合成方法と不透明度、中に行の一覧、下に操作の帯。

use egui::{pos2, vec2, Rect, Ui};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, NumberFormat, SliderSpec};

use super::layers_common::{self as common, ROWS, ROW_HEIGHT, TOOLBAR_HEIGHT};
use crate::page::SketchCtx;

pub fn draw(ui: &mut Ui, ctx: &mut SketchCtx) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);

    // 上: 合成方法（左）と不透明度（右）
    let top = Rect::from_min_size(
        pos2(r.left() + t::PADDING, r.top() + 6.0),
        vec2(r.width() - 2.0 * t::PADDING, 24.0),
    );
    let left_w = ((top.width() - 6.0) * 0.42).floor();
    let blend = Rect::from_min_size(top.min, vec2(left_w, top.height()));
    let opacity = Rect::from_min_max(pos2(top.left() + left_w + 6.0, top.top()), top.max);
    let selected = ctx.values.index("layers.selected", 0).min(ROWS.len() - 1);
    let blend_name = ctx.pick(ROWS[selected].blend.0, ROWS[selected].blend.1);
    let _ = w::dropdown(ui, blend, "sketch.blend", None, blend_name, None, true, 0.0);
    let value = ctx
        .values
        .float("layers.top-opacity", ROWS[selected].opacity);
    let spec = SliderSpec::new(
        ctx.pick("不透明度", "Opacity"),
        0.0,
        100.0,
        NumberFormat::int("%"),
    );
    let out = w::slider(ui, opacity, "sketch.opacity", value, &spec);
    if out.changed {
        ctx.values.set_float("layers.top-opacity", out.value);
    }

    // 中: 行の一覧
    let list_top = top.bottom() + 6.0;
    let list = Rect::from_min_max(
        pos2(r.left(), list_top),
        pos2(r.right(), (r.bottom() - TOOLBAR_HEIGHT).max(list_top)),
    );
    w::fill(ui.painter(), list, t::CONTROL_BG);
    for (i, d) in ROWS.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + i as f32 * ROW_HEIGHT),
            vec2(list.width(), ROW_HEIGHT),
        );
        if row.top() >= list.bottom() {
            break;
        }
        common::row(ui, ctx, list, row, i, d, 26.0);
    }

    // 下: 操作の帯
    let bar = Rect::from_min_size(
        pos2(r.left(), list.bottom()),
        vec2(r.width(), TOOLBAR_HEIGHT),
    );
    common::toolbar(ui, ctx, bar);
}
