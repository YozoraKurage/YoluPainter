//! 新しいプロジェクトのウィンドウ（複数のマテリアルをまとめる案）。

use crate::page::SketchCtx;

use super::multimat_new_common::{new_window, Mode};

pub fn draw(ui: &mut egui::Ui, ctx: &mut SketchCtx) {
    new_window(ui, ctx, Mode::BMenu);
}
