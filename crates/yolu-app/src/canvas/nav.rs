//! 2D のキャンバスの押しの行き先（表示の回す・パン・拡縮、選択の組み合わせ方、スポイト、クローンの元、ツール）と、表示を動かす操作の押す・
//! 動く・離す。押した瞬間に、実際のボタン・修飾・押しながらのキーで、ドラッグの操作（`keymap::gesture`）と、動かさずに離したときの操作
//! （`keymap::click_gesture`）を別々に引く（`start_of`・`click_of`）。ドラッグの操作があれば始め、離しの操作は、ドラッグの有無によらず
//! 覚えて、動かさずに離したら行う（動いたら捨てる）。どちらも無ければ、左ボタンはツールの押し、ほかのボタンは何もしない。順は、押しながらの
//! キー（R・Space・Ctrl+Space。左ボタン）→ 視点の行（パン・回転）→ 選択のツールの選択の行 → スポイト → ツール。マウスとペン（サイドボタンは
//! 右ボタン）が同じ関数を通る。回すのは 15° 刻みが既定で、Shift を押していれば自由。回すドラッグは、押した所から `gesture::CLICK_MOVE` を
//! 超えて動くまで回さない。

use egui::{Modifiers, PointerButton, Pos2, Rect};

use super::{delta_angle, pointer_angle, ROTATE_DEAD_ZONE};
use crate::canvas::view::ROTATE_STEP;
use crate::engine::SelectionCombine;
use crate::gesture::{self, ZoomDrag};
use crate::keymap::Operation;
use crate::notice::Source;
use crate::state::{AppState, RotateDrag};

/// 押しで始める物。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Rotate,
    Pan,
    Zoom,
    /// スポイト（押した所から、離した所の色を取る）。
    Pick,
    /// 選択のツールの、組み合わせ方の決まった押し。
    Select(SelectionCombine),
    /// ツールの押し（左ボタン）。
    Tool,
    Nothing,
}

impl Start {
    /// 表示を動かす操作か。
    pub fn moves_view(self) -> bool {
        matches!(self, Start::Rotate | Start::Pan | Start::Zoom)
    }
}

/// この押しで始める物（実際のボタン・修飾・押しながらのキーで引く）。
pub fn start_of(app: &AppState, button: PointerButton, m: &Modifiers) -> Start {
    // 押しながらのキー（左ボタン）
    if button == PointerButton::Primary {
        if app.canvas.rotate_key_held {
            return Start::Rotate;
        }
        if gesture::zoom_chord(m, app.canvas.space_held) {
            return Start::Zoom;
        }
        if app.canvas.space_held {
            return Start::Pan;
        }
    }
    // 視点の行（組み合わせの表の canvas）
    match crate::keymap::gesture_where("canvas", button, m, false, |op| {
        matches!(op, Operation::Pan | Operation::Rotate)
    }) {
        Some(Operation::Pan) => return Start::Pan,
        Some(Operation::Rotate) => return Start::Rotate,
        _ => {}
    }
    // 選択のツールの、選択の行（組み合わせ方）
    if app.mode.paints() && app.tool.is_select() {
        if let Some(combine) = crate::selection::combine_for(button, m) {
            return Start::Select(combine);
        }
    }
    // スポイト（修飾を書いたとおりに。ポリゴン塗りつぶしの右ボタンはアイランドのメニュー）
    if crate::keymap::gesture_exact("canvas", button, m, app.canvas.space_held)
        == Some(Operation::Pick)
        && !(button == PointerButton::Secondary && app.right_opens_island_menu())
    {
        return Start::Pick;
    }
    if button == PointerButton::Primary {
        Start::Tool
    } else {
        Start::Nothing
    }
}

/// この押しを動かさずに離したときの操作（クローンの元。クローンのブラシのときだけ）。
pub fn click_of(app: &AppState, button: PointerButton, m: &Modifiers) -> Option<Operation> {
    match crate::keymap::click_gesture("canvas", button, m, app.canvas.space_held) {
        Some(Operation::CloneSource) if crate::clone_source::active(app) => {
            Some(Operation::CloneSource)
        }
        _ => None,
    }
}

/// 表示を動かす操作を始める（`start` が表示を動かす物のとき。始めたら true）。
pub fn start(
    app: &mut AppState,
    rect: Rect,
    pos: Pos2,
    button: PointerButton,
    start: Start,
    m: &Modifiers,
) -> bool {
    match start {
        Start::Rotate => {
            app.canvas.rotating = Some(RotateDrag {
                start_angle: app.view.angle,
                start_pan: app.view.pan,
                swept: 0.0,
                last_pointer_angle: pointer_angle(rect, pos),
                press: pos,
                moved: false,
            });
        }
        Start::Zoom => app.canvas.zooming = Some(ZoomDrag::new(pos, m.alt)),
        Start::Pan => app.canvas.panning = true,
        _ => return false,
    }
    app.canvas.nav_button = Some(button);
    true
}

/// 離しの操作を覚える（動かさずに離したら行う）。
pub fn note_click(app: &mut AppState, pos: Pos2, button: PointerButton, click: Option<Operation>) {
    app.canvas.clone_press = match click {
        Some(Operation::CloneSource) => Some((pos, button)),
        _ => None,
    };
}

/// ポインタが動いた（`previous` は前の位置）。回している・パンしている・拡縮している、のどれかだけ動かす。
/// 回すのは 15° 刻みで、`free`（Shift を押している）なら自由。
pub fn moved(app: &mut AppState, rect: Rect, pos: Pos2, previous: Pos2, free: bool) {
    if app
        .canvas
        .clone_press
        .is_some_and(|(start, _)| start.distance(pos) > gesture::CLICK_MOVE)
    {
        app.canvas.clone_press = None; // 動かした: ドラッグの操作だけ
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

/// ボタンを離した（ペンが離れた）。`button` が始めた物だけを終える。動かさずに離した拡縮は、押した点を中心に拡大（Alt を押して押していたら
/// 縮小）。動かさずに離したクローンの元の指定は、ここで決める（`pos` は離した点）。
pub fn released(app: &mut AppState, rect: Rect, pos: Pos2, button: PointerButton) {
    if app.canvas.clone_press.is_some_and(|(_, b)| b == button) {
        if let Some((start, _)) = app.canvas.clone_press.take() {
            if start.distance(pos) <= gesture::CLICK_MOVE {
                set_clone_source(app, rect, start);
            }
        }
    }
    if app.canvas.nav_button.is_some_and(|b| b != button) {
        return;
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
    app.canvas.panning = false;
    app.canvas.nav_button = None;
}

/// ビューを動かす操作の途中を全部やめる（フォーカスを失ったとき）。
pub fn cancel(app: &mut AppState) {
    app.canvas.clone_press = None;
    app.canvas.rotating = None;
    app.canvas.zooming = None;
    app.canvas.panning = false;
    app.canvas.nav_button = None;
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
