//! レイヤーの欄の案: 行の右に合成方法と不透明度を出す（不透明度は左右のドラッグで変える）。上の合成方法と不透明度の行は無くす。

use egui::{pos2, vec2, Rect, Ui};
use yolu_app::ui::numfield::{self, NumSpec};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align};

use super::layers_common::{self as common, ROWS, ROW_HEIGHT, TOOLBAR_HEIGHT};
use crate::page::SketchCtx;

/// 合成方法の名前を出す幅と、不透明度の欄の幅。
const BLEND_WIDTH: f32 = 52.0;
const OPACITY_WIDTH: f32 = 34.0;
/// ロックの印（行の右端）の幅。
const LOCK_WIDTH: f32 = 22.0;

pub fn draw(ui: &mut Ui, ctx: &mut SketchCtx) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);

    let list = Rect::from_min_max(
        pos2(r.left(), r.top()),
        pos2(r.right(), (r.bottom() - TOOLBAR_HEIGHT).max(r.top())),
    );
    w::fill(ui.painter(), list, t::CONTROL_BG);
    let spec = NumSpec::new(0.0, 100.0, 0.5, 0);
    for (i, d) in ROWS.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + i as f32 * ROW_HEIGHT),
            vec2(list.width(), ROW_HEIGHT),
        );
        if row.top() >= list.bottom() {
            break;
        }
        // 名前は、右の合成方法・不透明度・ロックの手前まで
        let right = LOCK_WIDTH + OPACITY_WIDTH + BLEND_WIDTH + 12.0;
        common::row(ui, ctx, list, row, i, d, right);
        let y = row.top() + 4.0;
        let h = row.height() - 8.0;
        let blend = Rect::from_min_size(
            pos2(
                row.right() - LOCK_WIDTH - OPACITY_WIDTH - BLEND_WIDTH - 8.0,
                y,
            ),
            vec2(BLEND_WIDTH, h),
        );
        let painter = ui.painter_at(list);
        w::text(
            &painter,
            blend,
            ctx.pick(d.blend.0, d.blend.1),
            t::LABEL_DIM,
            Align::Right,
        );
        let opacity = Rect::from_min_size(
            pos2(row.right() - LOCK_WIDTH - OPACITY_WIDTH - 2.0, y),
            vec2(OPACITY_WIDTH, h),
        );
        let key = format!("layers.opacity.{i}");
        let value = ctx.values.float(&key, d.opacity);
        let out = numfield::number_field(
            ui,
            opacity,
            ("sketch.opacity", i),
            "",
            f64::from(value),
            &spec,
            Some(ctx.pick("不透明度", "Opacity")),
            None,
            true,
        );
        if out.changed {
            ctx.values.set_float(&key, out.value as f32);
        }
    }

    let bar = Rect::from_min_size(
        pos2(r.left(), list.bottom()),
        vec2(r.width(), TOOLBAR_HEIGHT),
    );
    common::toolbar(ui, ctx, bar);
}
