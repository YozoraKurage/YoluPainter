//! 面を隠す（5 回目）: マルチ 3D ビュー（別のタブで増やす形）のハイドのモードで、2 つのビューに別のプリセットを当てた形。左は何も隠さず引き出しを閉じ、
//! 右は服を隠して引き出しを開いている。引き出し・シェルフ（左下に固定）・ボーンの丸はビューごとに持つ。ビューの見た目（見出し・左上の表示の種類の
//! ドロップダウン・右上のアイコンの列）はマルチ 3D ビューの下書き（案 B）に合わせて、この下書きの中で描いた（その下書きの部品は別のブランチにあり、
//! ここからは呼べない）。

use egui::{pos2, vec2, Rect, Ui};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align};

use super::hideface3_common::{chrome, pane, Mode, Pane, ShelfLook};
use super::hideface_common as common;
use crate::page::SketchCtx;

const HEAD: f32 = 24.0;
const GAP: f32 = 2.0;

pub fn draw(ui: &mut Ui, ctx: &mut SketchCtx) {
    let area = ui.max_rect();
    ui.advance_cursor_after_rect(area);
    let views = chrome(ui, ctx, area, Mode::Hide, false, false);
    w::fill(ui.painter(), views, t::BORDER);
    let half = (views.width() - GAP) / 2.0;
    let left = Rect::from_min_size(views.min, vec2(half, views.height()));
    let right = Rect::from_min_size(
        pos2(left.right() + GAP, views.top()),
        vec2(half, views.height()),
    );
    // 服（背の高い箱）を隠したビューと、何も隠さないビュー
    let cloth = vec![common::TALL_FRONT, common::TALL_TOP, common::TALL_SIDE];
    one(ui, ctx, left, 0, vec![], vec![], false, true);
    one(ui, ctx, right, 1, cloth, vec![3], true, false);
}

#[allow(clippy::too_many_arguments)]
fn one(
    ui: &mut Ui,
    ctx: &mut SketchCtx,
    rect: Rect,
    index: usize,
    hidden: Vec<usize>,
    selected_bones: Vec<usize>,
    drawer_open: bool,
    focused: bool,
) {
    let lang = ctx.lang;
    // ドックのタブの見出し（3D ビュー n · 表示の種類）
    let head = Rect::from_min_size(rect.min, vec2(rect.width(), HEAD));
    let p = ui.painter().clone();
    w::fill(&p, head, t::PANEL_HEADER);
    let title = if index == 0 {
        format!(
            "{} · {}",
            lang.pick("3D ビュー", "3D View"),
            lang.pick("マテリアル", "Material")
        )
    } else {
        format!(
            "{} 2 · {}",
            lang.pick("3D ビュー", "3D View"),
            lang.pick("中立", "Neutral")
        )
    };
    w::text(
        &p,
        Rect::from_min_max(pos2(head.left() + 10.0, head.top()), head.max),
        &title,
        t::HEADER,
        Align::Left,
    );
    if focused {
        w::fill(
            &p,
            Rect::from_min_size(head.min, vec2(head.width().min(190.0), 2.0)),
            t::ACCENT,
        );
    }

    let view = Rect::from_min_max(pos2(rect.left(), head.bottom()), rect.max);
    let (hide_preset, pose_preset) = if index == 0 { (0, 0) } else { (2, 1) };
    let key = if index == 0 {
        "multi-left"
    } else {
        "multi-right"
    };
    pane(
        ui,
        ctx,
        view,
        &Pane {
            key,
            mode: Mode::Hide,
            bone_hidden: false,
            hidden,
            selected_faces: vec![],
            pointed: None,
            selected_bones,
            applied: true,
            shelf: ShelfLook::Open,
            hide_preset,
            pose_preset,
            drawer_open,
            zoom: 1.0,
            shading_icon: false,
        },
    );

    // 左上の表示の種類のドロップダウン
    let dd = Rect::from_min_size(view.min + vec2(8.0, 8.0), vec2(170.0, 24.0));
    let _ = w::dropdown(
        ui,
        dd,
        ("hideface5.multi.shading", index),
        None,
        if index == 0 {
            lang.pick("マテリアル (PBR)", "Material (PBR)")
        } else {
            lang.pick("中立", "Neutral")
        },
        Some(lang.pick("3D の表示", "3D Shading")),
        true,
        0.0,
    );
}
