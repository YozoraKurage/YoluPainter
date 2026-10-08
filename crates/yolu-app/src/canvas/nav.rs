//! 2D のキャンバスを動かす操作（Alt + 左ドラッグで回す・Space + 左ドラッグでパン・Ctrl+Space + 左ドラッグで拡縮。R を押しながらの左ドラッグも、
//! 割り当てがあれば回す）の、押す・動く・離す。回すのは 15° 刻みが既定で、Shift を押していれば自由。修飾は押しの始めに持っているもので決める。
//! マウスとペンが同じ関数を通る（ペンの押しの行き先は `pen::PenPress`）。中ボタンのパンと、割り当てがあれば中ボタンの回転はマウスだけ（`mod.rs`）。
//! 回すドラッグは、押した所から `gesture::CLICK_MOVE` を超えて動くまで回さない。クローンのブラシで Alt + 左を動かさずに離すと、そこがクローンの元
//! （動かせば回すだけ）。

use egui::{Modifiers, Pos2, Rect};

use super::{delta_angle, pointer_angle, ROTATE_DEAD_ZONE};
use crate::canvas::view::ROTATE_STEP;
use crate::gesture::{self, ZoomDrag};
use crate::notice::Source;
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
/// 動かさずに離したときの行き先も、ここで決める: クローンのブラシで Alt + 左なら、押した点をクローンの元にする（`released`）。
pub fn press(app: &mut AppState, rect: Rect, pos: Pos2, modifiers: &Modifiers) -> bool {
    app.canvas.clone_press = None;
    if app.canvas.rotate_key_held || (!app.canvas.space_held && rotates(modifiers)) {
        app.canvas.rotating = Some(RotateDrag {
            start_angle: app.view.angle,
            start_pan: app.view.pan,
            swept: 0.0,
            last_pointer_angle: pointer_angle(rect, pos),
            press: pos,
            moved: false,
        });
        if crate::keymap::click_gesture(
            "canvas",
            egui::PointerButton::Primary,
            modifiers,
            app.canvas.space_held,
        ) == Some(crate::keymap::Operation::CloneSource)
            && crate::clone_source::active(app)
        {
            app.canvas.clone_press = Some(pos);
        }
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
    if app
        .canvas
        .clone_press
        .is_some_and(|start| start.distance(pos) > gesture::CLICK_MOVE)
    {
        app.canvas.clone_press = None; // 動かした: 回すだけ
    }
    if let Some(mut drag) = app.canvas.rotating {
        // 押した所から遊びを超えるまで回さない（超えたら、押した所からの動きを全部当てる: 角度は押したときの向きからの合計）
        if !drag.moved {
            if drag.press.distance(pos) <= gesture::CLICK_MOVE {
                return;
            }
            drag.moved = true;
            app.canvas.rotating = Some(drag);
        }
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

/// ボタンを離した（ペンが離れた）。動かさずに離した拡縮は、押した点を中心に拡大（Alt を押して押していたら縮小）。動かさずに離したクローンの元の指定は、
/// ここで決める（`pos` は離した点）。
pub fn released(app: &mut AppState, rect: Rect, pos: Pos2) {
    if let Some(start) = app.canvas.clone_press.take() {
        if start.distance(pos) <= gesture::CLICK_MOVE {
            set_clone_source(app, rect, start);
        }
    }
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
    app.canvas.clone_press = None;
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

/// 押した点（画面の点）の下の文書の点を、クローンの元にする（キャンバスの外は断る）。
fn set_clone_source(app: &mut AppState, rect: Rect, at: Pos2) {
    let view = app.view.view(rect, app.doc.width(), app.doc.height());
    let (x, y) = view.to_canvas(at);
    let (w, h) = (app.doc.width() as f64, app.doc.height() as f64);
    if !(0.0..w).contains(&x) || !(0.0..h).contains(&y) {
        app.refuse(
            Source::Canvas,
            app.lang.pick("キャンバスの外です。", "Outside the canvas."),
        );
        return;
    }
    app.clone.set_canvas_source(app.doc.id(), (x, y));
    app.info(
        Source::Canvas,
        app.lang
            .pick("クローンの元を決めました。", "Clone source set."),
    );
}
