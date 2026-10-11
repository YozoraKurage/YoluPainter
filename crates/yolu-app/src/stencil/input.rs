//! ステンシルのキーとドラッグ（Unity 版と同じ）: Y を押しているあいだ、2D のキャンバスか 3D のビューでのドラッグが置き場を動かす
//! （左 = 回す（Shift で 15° 刻み）、中か Ctrl+左 = 動かす、右か Alt+左 = 大きさ）。N を押しているあいだはステンシルを使わない。
//! ドラッグの最中の Esc は始めの置き場に戻す。Y を離してもドラッグはボタンを離すまで続く。フォーカスを失ったら、押していた印を捨て、
//! ドラッグは今の置き場で終える（離したのを受け取れないので）。ストロークの最中・ポップアップが開いているあいだは始めない。ドラッグの最中は、
//! ほかのボタンを押しても何も始めない（ストローク・パン・3D のカメラ）。

use egui::{Event, Key, PointerButton, Pos2, Rect};

use super::frame::{MAX_SIZE, MIN_SIZE, ROTATE_STEP};
use crate::canvas::view::normalize_angle;
use crate::notice::Source;
use crate::state::AppState;

/// 回す角度を測らない、ステンシルの中心からの距離（画面の点）。
const ROTATE_DEAD_ZONE: f32 = 4.0;
/// 大きさをドラッグで変える速さ（この点数で e 倍）。
const SCALE_POINTS_PER_E: f32 = 200.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragKind {
    Move,
    Rotate,
    Scale,
}

/// 置き場を動かしているドラッグ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StencilDrag {
    pub kind: DragKind,
    /// 押したボタン（離したら終わる）。
    pub button: PointerButton,
    /// 押した表示域（ドラッグの間はこの表示域の割合で動かす）。
    pub view: Rect,
    from: Pos2,
    start_center: [f32; 2],
    start_size: f32,
    start_angle: f32,
    swept: f32,
    last_angle: f32,
}

fn delta_angle(from: f32, to: f32) -> f32 {
    let d = (to - from) % 360.0;
    if d > 180.0 {
        d - 360.0
    } else if d < -180.0 {
        d + 360.0
    } else {
        d
    }
}

impl StencilDrag {
    /// ステンシルの中心（画面の点）。
    fn center_in(&self, center: [f32; 2]) -> Pos2 {
        egui::pos2(
            self.view.left() + center[0] * self.view.width(),
            self.view.top() + center[1] * self.view.height(),
        )
    }

    fn pointer_angle(&self, center: [f32; 2], pointer: Pos2) -> f32 {
        let d = pointer - self.center_in(center);
        d.y.atan2(d.x).to_degrees()
    }
}

impl super::StencilState {
    /// ドラッグを始める（押した点・表示域）。始められなければ false。
    fn begin_drag(&mut self, kind: DragKind, button: PointerButton, view: Rect, at: Pos2) {
        let mut drag = StencilDrag {
            kind,
            button,
            view,
            from: at,
            start_center: self.center,
            start_size: self.size,
            start_angle: self.angle,
            swept: 0.0,
            last_angle: 0.0,
        };
        drag.last_angle = drag.pointer_angle(self.center, at);
        self.drag = Some(drag);
    }

    /// ポインタが p に動いた。shift なら回すのを 15° 刻みに。
    pub fn update_drag(&mut self, p: Pos2, shift: bool) {
        let Some(mut drag) = self.drag else {
            return;
        };
        match drag.kind {
            DragKind::Move => {
                self.set_center(
                    drag.start_center[0] + (p.x - drag.from.x) / drag.view.width().max(1.0),
                    drag.start_center[1] + (p.y - drag.from.y) / drag.view.height().max(1.0),
                );
            }
            DragKind::Scale => {
                // 右・上へ動かすと大きく、左・下へで小さく（200 点で e 倍）
                let moved = (p.x - drag.from.x) - (p.y - drag.from.y);
                self.size = (drag.start_size * (moved / SCALE_POINTS_PER_E).exp())
                    .clamp(MIN_SIZE, MAX_SIZE);
            }
            DragKind::Rotate => {
                if (p - drag.center_in(self.center)).length() < ROTATE_DEAD_ZONE {
                    return;
                }
                let a = drag.pointer_angle(self.center, p);
                drag.swept += delta_angle(drag.last_angle, a);
                drag.last_angle = a;
                let mut target = drag.start_angle + drag.swept;
                if shift {
                    target = (target / ROTATE_STEP).round() * ROTATE_STEP;
                }
                self.angle = normalize_angle(target);
                self.drag = Some(drag);
            }
        }
    }

    /// ドラッグを終える（revert なら始めの置き場に戻す。Esc）。
    pub fn end_drag(&mut self, revert: bool) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if revert {
            self.center = drag.start_center;
            self.size = drag.start_size;
            self.angle = drag.start_angle;
        }
    }

    /// フォーカスを失った: 押していた印を捨て、ドラッグは今の置き場で終える。
    pub fn release_input(&mut self) {
        self.key_held = false;
        self.ignore_held = false;
        self.end_drag(false);
    }
}

/// T と N を見る（毎フレームの始め）。押し始めは、文字を打っていない・ポップアップが開いていないとき。N は修飾キー無しで押し始めたとき
/// だけ（Ctrl+N は新しいプロジェクト、Ctrl+Shift+N は新しいレイヤー）。押し始めたあとは、離すまで続く（Ctrl を足しても外れない）。
pub fn update_keys(ctx: &egui::Context, app: &mut AppState) {
    let typing = ctx.egui_wants_keyboard_input();
    let blocked = app.popup.is_some() || app.ui.popup_was_open;
    let (t, n, modifiers, focus_lost) = ctx.input(|i| {
        (
            crate::keymap::hold_down(i, "stencil.transform_hold"),
            crate::keymap::hold_down(i, "stencil.bypass_hold"),
            i.modifiers,
            i.events
                .iter()
                .any(|e| matches!(e, Event::WindowFocused(false))),
        )
    });
    if focus_lost {
        app.stencil.release_input();
        return;
    }
    // 編集・ポーズのモードでは描かないので、Y・N を押しても効かない（ペイントのモードの行。`keymap::Scope::Paint`）
    let paints = app.mode.paints();
    let st = &mut app.stencil;
    st.key_held =
        paints && t && (st.key_held || !typing && !blocked && !modifiers.command && !modifiers.alt);
    st.ignore_held = paints && n && (st.ignore_held || !typing && !blocked && !modifiers.any());
}

/// 1 つの入力イベントを見る（rect は押した表示域（2D のキャンバスか 3D の中身）、over はこの点がその表示域の一番上にあるか）。
/// ステンシルが受け持ったら true（呼び手は、そのイベントでほかの操作をしない）。
pub fn handle_event(
    app: &mut AppState,
    event: &Event,
    rect: Rect,
    over: bool,
    modifiers: &egui::Modifiers,
) -> bool {
    if let Some(drag) = app.stencil.drag {
        return match event {
            Event::PointerMoved(p) => {
                // 回す間の刻み（組み合わせの表の、始めたあとに効く修飾。既定は Shift）
                let snap = crate::keymap::modifier_held(
                    "stencil",
                    crate::keymap::Operation::SnapStencilRotation,
                    modifiers,
                );
                app.stencil.update_drag(*p, snap);
                true
            }
            // ドラッグの最中は、ほかのボタンを押しても何も始めない（ストロークも、キャンバスのパンも、3D の回しも）。離すほうは、ドラッグの
            // ボタンだけがドラッグを終え、ほかのボタンはキャンバスへ通す（ドラッグの前から押していたボタンの状態を、キャンバスが戻せるように）
            Event::PointerButton { pressed: true, .. } => true,
            Event::PointerButton {
                button,
                pressed: false,
                ..
            } => {
                if *button == drag.button {
                    app.stencil.end_drag(false);
                    true
                } else {
                    false
                }
            }
            Event::Key {
                key: Key::Escape,
                pressed: true,
                ..
            } => {
                app.stencil.end_drag(true);
                true
            }
            Event::WindowFocused(false) => {
                app.stencil.release_input();
                false
            }
            _ => false,
        };
    }
    let Event::PointerButton {
        pos,
        button,
        pressed: true,
        modifiers,
    } = event
    else {
        return false;
    };
    if !app.stencil.key_held
        || app.is_stroking()
        || app.popup.is_some()
        || app.ui.popup_was_open
        || !over
    {
        return false;
    }
    // 押し方で決まるドラッグの種類は `keymap::GESTURES`（Y を押しながら）
    let kind = match crate::keymap::gesture("stencil", *button, modifiers, true) {
        Some(crate::keymap::Operation::MoveStencil) => DragKind::Move,
        Some(crate::keymap::Operation::ScaleStencil) => DragKind::Scale,
        Some(crate::keymap::Operation::RotateStencil) => DragKind::Rotate,
        _ => return false,
    };
    if app.stencil.image.is_none() {
        app.refuse(
            Source::Stencil,
            app.lang
                .pick("ステンシルの画像がありません。", "No stencil image."),
        );
        return true;
    }
    app.stencil.begin_drag(kind, *button, rect, *pos);
    true
}

/// ボタンを離したのを取りこぼしたとき（ウィンドウの外で離したなど）に、押していなければドラッグを終える（イベントを全部見た後で）。
pub fn settle(app: &mut AppState, any_button_down: bool) {
    if app.stencil.drag.is_some() && !any_button_down {
        app.stencil.end_drag(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stencil::StencilState;
    use egui::{pos2, vec2};

    fn state_with_image() -> StencilState {
        let mut s = StencilState::default();
        s.set_image_rgba("a", 2, 2, &[255; 16]).unwrap();
        s
    }

    fn view() -> Rect {
        Rect::from_min_size(pos2(100.0, 50.0), vec2(400.0, 300.0))
    }

    #[test]
    fn move_follows_the_pointer_in_view_fractions_and_escape_reverts() {
        let mut s = state_with_image();
        s.begin_drag(
            DragKind::Move,
            PointerButton::Middle,
            view(),
            pos2(300.0, 200.0),
        );
        s.update_drag(pos2(340.0, 170.0), false);
        assert!((s.center[0] - 0.6).abs() < 1e-6 && (s.center[1] - 0.4).abs() < 1e-6);
        s.end_drag(true);
        assert_eq!(s.center, [0.5, 0.5]);
        assert!(s.drag.is_none());
    }

    #[test]
    fn scale_grows_up_and_right_and_is_clamped() {
        let mut s = state_with_image();
        s.begin_drag(
            DragKind::Scale,
            PointerButton::Secondary,
            view(),
            pos2(300.0, 200.0),
        );
        s.update_drag(pos2(400.0, 200.0), false); // 100 点右へ
        assert!((s.size - 0.6 * (0.5f32).exp()).abs() < 1e-5);
        s.update_drag(pos2(300.0 - 100_000.0, 200.0), false);
        assert_eq!(s.size, MIN_SIZE);
        s.update_drag(pos2(300.0 + 100_000.0, 200.0), false);
        assert_eq!(s.size, MAX_SIZE);
        s.end_drag(false);
        assert_eq!(s.size, MAX_SIZE, "離したら今の置き場のまま");
    }

    #[test]
    fn rotate_sweeps_clockwise_ignores_the_dead_zone_and_snaps_with_shift() {
        let mut s = state_with_image();
        let c = pos2(300.0, 200.0); // 表示域の中心 = ステンシルの中心
        s.begin_drag(
            DragKind::Rotate,
            PointerButton::Primary,
            view(),
            pos2(400.0, 200.0),
        );
        s.update_drag(pos2(c.x + 1.0, c.y + 1.0), false); // 中心の近く: 測らない
        assert_eq!(s.angle, 0.0);
        // 右 → 真下: 時計回りに 90°
        s.update_drag(pos2(400.0, 200.0), false);
        s.update_drag(pos2(370.7, 270.7), false);
        s.update_drag(pos2(300.0, 300.0), false);
        assert!((s.angle - 90.0).abs() < 0.5, "{}", s.angle);
        // さらに回しても連続する（-180 で折り返さない: 180 を越えたら (-180, 180] に収める）
        s.update_drag(pos2(229.3, 270.7), false);
        s.update_drag(pos2(200.0, 200.0), false);
        assert!((s.angle - 180.0).abs() < 0.5, "{}", s.angle);
        // Shift: 15° 刻み
        s.update_drag(pos2(229.3, 129.3), true);
        assert_eq!(s.angle, -135.0, "{}", s.angle);
        s.end_drag(true);
        assert_eq!(s.angle, 0.0);
    }

    #[test]
    fn losing_focus_drops_the_keys_and_keeps_the_placement() {
        let mut s = state_with_image();
        s.key_held = true;
        s.ignore_held = true;
        s.begin_drag(
            DragKind::Move,
            PointerButton::Middle,
            view(),
            pos2(300.0, 200.0),
        );
        s.update_drag(pos2(310.0, 200.0), false);
        s.release_input();
        assert!(!s.key_held && !s.ignore_held && s.drag.is_none());
        assert!(s.center[0] > 0.5);
    }
}
