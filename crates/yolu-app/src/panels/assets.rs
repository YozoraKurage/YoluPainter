//! アセットのパネル（ブラシ・マテリアル・スマートマテリアルの置き場）。M1 では枠だけ。

use egui::Ui;

use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Rows};

pub fn show(ui: &mut Ui) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let mut rows = Rows::new(r, 8.0);
    let row = rows.row(60.0, 0.0);
    w::wrapped_text(ui.painter(), row, "準備中", t::LABEL_DIM);
}
