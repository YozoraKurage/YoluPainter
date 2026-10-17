//! Tab でパネル・メニュー・ツールの帯を全部隠した形（ビューだけが残る）。画面全体を覆い、ビューと、フォーカスのあるビューの縁だけを
//! 描く（隅のアイコンと軸の印も隠れる）。

use egui::Ui;
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets as w;

use super::multiview_common::{self as c, Cam, Shade, BACK_ANGLE, PERSPECTIVE, PERSPECTIVE_B};
use crate::page::SketchCtx;

/// 左右 2 つ（案 B のタブを並べた形）。左（マテリアル）にフォーカス。
pub fn draw_two(ui: &mut Ui, _ctx: &mut SketchCtx) {
    c::cover(ui, "tab-two", |ui, screen| {
        w::fill(ui.painter(), screen, t::BORDER);
        let cells = c::grid(screen, 2, 1);
        let list: [(Cam, Shade); 2] = [
            (PERSPECTIVE, Shade::Material),
            (PERSPECTIVE_B, Shade::Channel(1)),
        ];
        for (i, (r, (cam, shade))) in cells.iter().zip(list).enumerate() {
            c::draw_view(ui, *r, cam, shade, None, 1.0);
            if i == 0 {
                c::focus_border(ui, *r);
            }
        }
    });
}

/// フォーカスのあるビューだけが画面いっぱいに出る形（隠すと、ほかのビューも引っ込む案）。
pub fn draw_one(ui: &mut Ui, _ctx: &mut SketchCtx) {
    c::cover(ui, "tab-one", |ui, screen| {
        c::draw_view(ui, screen, BACK_ANGLE, Shade::Material, None, 1.0);
    });
}
