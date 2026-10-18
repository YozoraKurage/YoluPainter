//! テクスチャセットの一覧: 複数のマテリアルを結び付けたセットは、名前の右にマテリアルの名前を薄く並べる（1 つだけのセットは名前だけ）。
//! 行の寸法・目・状態の印・解像度・下の帯は本物の一覧（`panels/texture_sets.rs`）と同じ。

use egui::{pos2, vec2, Color32, Rect, Ui};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align};

use super::multimat_common::{DummySet, MATERIALS, SETS};
use crate::page::SketchCtx;

const ROW_HEIGHT: f32 = 28.0;
const TOOLBAR_HEIGHT: f32 = 28.0;

pub fn draw(ui: &mut Ui, ctx: &mut SketchCtx) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let toolbar = Rect::from_min_max(
        pos2(r.left(), (r.bottom() - TOOLBAR_HEIGHT).max(r.top())),
        r.max,
    );
    let list = Rect::from_min_max(r.min, pos2(r.right(), toolbar.top()));
    w::fill(ui.painter(), list, t::CONTROL_BG);
    for (i, set) in SETS.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + i as f32 * ROW_HEIGHT),
            vec2(list.width(), ROW_HEIGHT),
        );
        set_row(ui, ctx, list, row, i, set);
    }
    toolbar_buttons(ui, ctx, toolbar);
}

fn set_row(ui: &mut Ui, ctx: &SketchCtx, list: Rect, row: Rect, i: usize, set: &DummySet) {
    let selected = i == 0;
    let painter = ui.painter_at(list);
    if selected {
        w::fill(&painter, row, t::ACCENT_SOFT);
        w::fill(
            &painter,
            Rect::from_min_size(row.min, vec2(3.0, row.height())),
            t::ACCENT,
        );
    }
    w::hline(
        &painter,
        row.left(),
        row.right(),
        row.bottom() - 1.0,
        t::BORDER,
    );
    let eye = Rect::from_min_size(
        pos2(row.left() + 4.0, row.top() + 3.0),
        vec2(24.0, row.height() - 6.0),
    );
    let _ = w::icon_button(
        ui,
        eye,
        ("mm.eye", i),
        "visibility",
        ctx.pick("3D ビューで隠す", "Hide in the 3D View"),
        false,
        true,
        16.0,
    );
    let resolution = "2048";
    let res_w = w::text_width(&painter, resolution, t::LABEL_DIM) + 4.0;
    let res_rect = Rect::from_min_max(
        pos2(row.right() - 8.0 - res_w, row.top()),
        pos2(row.right() - 8.0, row.bottom()),
    );
    let icon_rect = Rect::from_min_size(
        pos2(res_rect.left() - 22.0, row.top() + 5.0),
        vec2(18.0, row.height() - 10.0),
    );
    let name_rect = Rect::from_min_max(
        pos2(eye.right() + 6.0, row.top() + 4.0),
        pos2(icon_rect.left() - 4.0, row.bottom() - 4.0),
    );
    // マテリアルの無いセットは、本物と同じ薄い切れた鎖
    if set.materials.is_empty() {
        w::icon(&painter, icon_rect, "link_off", t::TEXT_DIM, 15.0);
    }
    w::text(&painter, res_rect, resolution, t::LABEL_DIM, Align::Right);
    let color = if selected { Color32::WHITE } else { t::TEXT };
    let name_w = w::text_width(&painter, set.name, t::LABEL);
    let name = w::fit(&painter, set.name, name_rect.width(), t::LABEL);
    w::text(
        &painter,
        name_rect,
        &name,
        t::LABEL.with_color(color),
        Align::Left,
    );
    // 複数のマテリアルのセットだけ、名前の右にマテリアルの名前を薄く
    if set.materials.len() > 1 {
        let names: Vec<&str> = set.materials.iter().map(|&m| MATERIALS[m]).collect();
        let text = names.join("・");
        let at = Rect::from_min_max(
            pos2(name_rect.left() + name_w + 10.0, name_rect.top()),
            name_rect.max,
        );
        if at.width() > 8.0 {
            let shown = w::fit(&painter, &text, at.width(), t::LABEL_DIM);
            w::text(&painter, at, &shown, t::LABEL_DIM, Align::Left);
        }
    }
}

fn toolbar_buttons(ui: &mut Ui, ctx: &SketchCtx, bar: Rect) {
    let p = ui.painter().clone();
    w::fill(&p, bar, t::PANEL_HEADER);
    w::hline(&p, bar.left(), bar.right(), bar.top(), t::BORDER);
    let button =
        |x: f32| Rect::from_min_size(pos2(x, bar.top() + 2.0), vec2(26.0, bar.height() - 4.0));
    let _ = w::icon_button(
        ui,
        button(bar.left() + 4.0),
        "mm.add",
        "add",
        ctx.pick("空のテクスチャセットを追加", "Add an Empty Texture Set"),
        false,
        true,
        16.0,
    );
    let _ = w::icon_button(
        ui,
        button(bar.left() + 34.0),
        "mm.remove",
        "delete",
        ctx.pick("このテクスチャセットを消す", "Remove This Texture Set"),
        false,
        true,
        16.0,
    );
    let bake = button(bar.right() - 60.0);
    let _ = w::icon_button(
        ui,
        bake,
        "mm.bake",
        "local_fire_department",
        ctx.pick("メッシュマップをベイク", "Bake Mesh Maps"),
        false,
        true,
        16.0,
    );
    let _ = w::icon_button(
        ui,
        button(bar.right() - 30.0),
        "mm.configure",
        "tune",
        ctx.pick("プロジェクト設定", "Project Configuration"),
        false,
        true,
        16.0,
    );
}
