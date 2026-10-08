//! 2D のキャンバスを動かす操作（Alt + 左ドラッグで回す・Space + 左ドラッグでパン・Ctrl+Space + 左ドラッグで拡縮。R を押しながらの左ドラッグも、
//! 割り当てがあれば回す）の、押す・動く・離す。回すのは 15° 刻みが既定で、Shift を押していれば自由。修飾は押しの始めに持っているもので決める。
//! マウスとペンが同じ関数を通る（ペンの押しの行き先は `pen::PenPress`）。中ボタンのパンと、割り当てがあれば中ボタンの回転はマウスだけ（`mod.rs`）。

use egui::{Modifiers, Pos2, Rect};

use super::{delta_angle, pointer_angle, ROTATE_DEAD_ZONE};
use crate::canvas::view::ROTATE_STEP;
use crate::gesture::{self, ZoomDrag};
use crate::state::{AppState, RotateDrag};

/// この押しがビューを動かす操作になるか（R を押している・Space を押している・Alt + 左ドラッグで回す組み合わせ。`press` が始める押し）。
pub fn starts_view(app: &AppState, modifiers: &Modifiers) -> bool {
    app.canvas.rotate_key_held || app.canvas.space_held || rotates(modifiers)
}

/// 左ボタンのこの修飾が、表示を回す組み合わせか（`keymap::GESTURES` の表。Alt）。
fn rotates(modifiers: &Modifiers) -> bool {
    crate::keymap::gesture("canvas", egui::PointerButton::Primary, modifiers, false)
        == Some(crate::keymap::Operation::Rotate)
}

/// 押した点で、ビューを動かす操作を始める（R・Ctrl+Space・Space・Alt の順。押していなければ何もしない）。始めたか。
pub fn press(app: &mut AppState, rect: Rect, pos: Pos2, modifiers: &Modifiers) -> bool {
    if app.canvas.rotate_key_held || (!app.canvas.space_held && rotates(modifiers)) {
        app.canvas.rotating = Some(RotateDrag {
            start_angle: app.view.angle,
            start_pan: app.view.pan,
            swept: 0.0,
            last_pointer_angle: pointer_angle(rect, pos),
        });
        true
    } else if gesture::zoom_chord(modifiers, app.canvas.space_held) {
        app.canvas.zooming = Some(ZoomDrag::new(pos, modifiers.alt));
        true
    } else if app.canvas.space_held {
        app.canvas.panning = true;
        true
    } else {
        false
    }
}

/// ポインタが動いた（`previous` は前の位置）。回している・中ボタンで回している・パンしている・拡縮している、のどれかだけ動かす。
/// 回すのは 15° 刻みで、`free`（Shift を押している）なら自由。
pub fn moved(app: &mut AppState, rect: Rect, pos: Pos2, previous: Pos2, free: bool) {
    if let Some(mut drag) = app.canvas.rotating {
        if (pos - rect.center()).length() >= ROTATE_DEAD_ZONE {
            let a = pointer_angle(rect, pos);
            drag.swept += delta_angle(drag.last_pointer_angle, a);
            drag.last_pointer_angle = a;
            let mut swept = drag.swept;
            if !free {
                swept = ((drag.start_angle + swept) / ROTATE_STEP).round() * ROTATE_STEP
                    - drag.start_angle;
            }
            app.view
                .rotate_from(drag.start_angle, drag.start_pan, swept);
            app.canvas.rotating = Some(drag);
        }
    } else if app.canvas.middle_rotating {
        if (previous - rect.center()).length() >= ROTATE_DEAD_ZONE
            && (pos - rect.center()).length() >= ROTATE_DEAD_ZONE
        {
            app.view.rotate_by(delta_angle(
                pointer_angle(rect, previous),
                pointer_angle(rect, pos),
            ));
        }
    } else if app.canvas.panning {
        app.view.pan += pos - previous;
    } else if let Some(mut zoom) = app.canvas.zooming {
        let dx = zoom.moved_to(pos);
        // 動かさずに離せば拡大（クリック）なので、少しの揺れでは拡縮しない
        if !zoom.is_click() {
            let k = (dx * gesture::ZOOM_PER_POINT).exp();
            app.view.zoom_to(app.view.zoom * k, Some(zoom.anchor), rect);
        }
        app.canvas.zooming = Some(zoom);
    }
}

/// ボタンを離した（ペンが離れた）。動かさずに離した拡縮は、押した点を中心に拡大（Alt を押して押していたら縮小）。
pub fn released(app: &mut AppState, rect: Rect) {
    if let Some(zoom) = app.canvas.zooming.take() {
        if zoom.is_click() {
            let k = if zoom.out {
                1.0 / gesture::CLICK_ZOOM
            } else {
                gesture::CLICK_ZOOM
            };
            app.view.zoom_to(app.view.zoom * k, Some(zoom.anchor), rect);
        }
    }
    app.canvas.rotating = None;
    if !app.canvas.middle_rotating {
        app.canvas.panning = false;
    }
}

/// ビューを動かす操作の途中を全部やめる（フォーカスを失ったとき）。
pub fn cancel(app: &mut AppState) {
    app.canvas.rotating = None;
    app.canvas.zooming = None;
    app.canvas.panning = false;
    app.canvas.middle_rotating = false;
}

/// ペンがビューを動かしている最中か（egui のポインタの代わりの入力が離れたように見えても、ペンが離すまで続ける）。
pub fn pen_driven(app: &AppState) -> bool {
    app.canvas
        .pen_press
        .is_some_and(|p| p.kind == crate::pen::PressKind::View)
}
