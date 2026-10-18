//! プロジェクト設定: 行のマテリアルの箱を複数選べる形にして開いた状態（マテリアルごとのチェックの行。ほかのセットに属する行は、右にそのセットの名前）。

use crate::page::SketchCtx;

use super::multimat_common::{config_window, Mode};

pub fn draw(ui: &mut egui::Ui, ctx: &mut SketchCtx) {
    config_window(ui, ctx, Mode::Dropdown);
}
