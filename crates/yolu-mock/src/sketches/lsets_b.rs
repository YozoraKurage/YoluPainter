//! レイヤーセットの画面: 出力の割り当てを、チャンネルのパネルの行に入れる（行の右に、出力元のレイヤーセットのドロップダウン）。

use egui::Ui;

use super::lsets_common::{scene, Show};
use crate::page::SketchCtx;

/// 右クリックのメニューを開いた形。
pub fn draw(ui: &mut Ui, ctx: &mut SketchCtx) {
    scene(ui, ctx, Show::Menu);
}

/// 出力の割り当てのドロップダウンを開いた形。
pub fn draw_pick(ui: &mut Ui, ctx: &mut SketchCtx) {
    scene(ui, ctx, Show::Pick);
}
