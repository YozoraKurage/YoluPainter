//! 2D のキャンバスと 3D ビューが共有する入力の段: フレームの入力を 1 本に並べてマウスとペンの点を 1 つの形にする（`Stream`・`PointerSample`）、
//! 押した点が一番上のビューか（`on_top`）、押しの持ち主（`owner`）。

pub mod owner;
pub mod sample;

pub use owner::{owner, Owner, Press, PressCtx};
pub use sample::{Item, Phase, PointerSample, Stream};

use egui::{Pos2, Rect, Ui};

/// この点で一番上にあるのが `ui` のレイヤーか（ポップアップ・浮いたウィンドウが上にあれば、ビューは押しを受けない）。
pub fn on_top(ui: &Ui, rect: Rect, p: Pos2) -> bool {
    rect.contains(p)
        && ui
            .ctx()
            .layer_id_at(p)
            .is_none_or(|layer| layer == ui.layer_id())
}
