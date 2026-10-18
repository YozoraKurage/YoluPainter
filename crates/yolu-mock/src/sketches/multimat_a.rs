//! 案 A: マテリアルを全部ほかのセットへ移されたセットは、行が残り、マテリアルの欄に警告を出す（消すのは行のごみ箱）。

use crate::page::SketchCtx;

use super::multimat_common::{config_window, Mode};

pub fn draw(ui: &mut egui::Ui, ctx: &mut SketchCtx) {
    config_window(ui, ctx, Mode::KeepWarn);
}
