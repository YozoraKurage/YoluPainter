//! 3D ビューの光の向きを回す操作（影のテクスチャを描きながら、光の当たり方を素早く確かめる）。
//! - Shift + 右ドラッグ（ペンはサイドボタン + Shift）: 横の動きで方位、縦の動きで高さ（右ドラッグの視点の回転と同じ点あたりの角度）。
//! - 自動で回す（`Display::light_spin`）: 方位だけを一定の速さで回し、切るとその時の方位で止まる。
//!
//! 光の向きは表示の設定（`Display::light_yaw`・`light_pitch`。全部のビューで同じ値）で、文書の変更でも取り消しの段でもない（スライダーと同じ）。
//! 動かすのは `View3dState::apply_display`（`Action` は通さない: ドラッグの 1 動きごとに記録や確定待ちの処理を回さない）。

use egui::Context;
use yolu_core::geometry::ORBIT_DEGREES_PER_POINT;
use yolu_core::glam::Vec3;

use super::display::Op;
use super::Nav;
use crate::state::AppState;

/// 光の方位が 1 周する秒数。
pub const SPIN_SECONDS: f32 = 12.0;
/// 1 フレームの長さの上限（秒）。止まったあとの最初のフレームで飛ばない。
const MAX_DT: f32 = 0.1;

/// 方位を -180〜180 度へ畳む。
fn wrap(yaw: f32) -> f32 {
    (yaw + 180.0).rem_euclid(360.0) - 180.0
}

/// 光の向きを回すドラッグの最中か（今のビューの押し）。
pub fn dragging(app: &AppState) -> bool {
    matches!(app.view3d.view.input.nav, Some((Nav::Light, _)))
}

/// 光の向きを回している最中か（ドラッグの最中か、自動で回している間。軸の印の隣に光の印を出す）。
pub fn active(app: &AppState) -> bool {
    dragging(app) || app.view3d.view.display.light_spin
}

/// 方位が増えると光が画面のどちらへ寄るか: 右なら 1、左なら -1（光の動く水平の接線とカメラの右の内積の符号。真横で決まらなければ 1）。
pub fn turn_direction(view: &super::ViewState) -> f32 {
    let yaw = view.display.light_yaw.to_radians();
    let around = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
    if around.dot(view.camera.rotation() * Vec3::X) < 0.0 {
        -1.0
    } else {
        1.0
    }
}

/// 画面の動き (dx, dy)（点。右・下が正）で光の向きを回す。`turn` は `turn_direction`（押したときに決めた向き）。横の動きで方位を、光が画面の動きと同じ
/// 向きへ寄るように回し、縦の動きで高さを、上へのドラッグで上げる（-89〜89 度で止める）。
pub fn drag(app: &mut AppState, turn: f32, dx: f32, dy: f32) {
    let d = &app.view3d.view.display;
    let (yaw, pitch) = (d.light_yaw, d.light_pitch);
    app.view3d.apply_display(Op::LightYaw(wrap(
        yaw + turn * dx * ORBIT_DEGREES_PER_POINT,
    )));
    app.view3d
        .apply_display(Op::LightPitch(pitch - dy * ORBIT_DEGREES_PER_POINT));
}

/// 時間を `dt` 秒進める: 自動で回している間、方位を回す（1 周 `SPIN_SECONDS` 秒）。進めたら true。描いている最中（ストロークの間・離したあと塗り終えるまで）は
/// 進めない。手で回している間（ドラッグ）も進めない（手の動きと競わない）。
pub fn advance(app: &mut AppState, dt: f32) -> bool {
    let input = &app.view3d.view.input;
    if !app.view3d.view.display.light_spin
        || app.is_stroking()
        || input.stroke.is_some()
        || input.released
        || dragging(app)
    {
        return false;
    }
    let step = 360.0 / SPIN_SECONDS * dt.clamp(0.0, MAX_DT);
    let yaw = app.view3d.view.display.light_yaw;
    app.view3d.apply_display(Op::LightYaw(wrap(yaw + step)));
    true
}

/// フレームごとに 1 回（アプリのフレームの終わり。3D のタブが見えている間だけ）: 自動で回し、描き直しを頼み続ける。ウィンドウが隠れている間は
/// eframe がフレームを回さないので、何もしない（フレームを無駄に回さない）。
pub fn tick(ctx: &Context, app: &mut AppState) {
    // egui が同じフレームをもう一度回す（2 回目以降の pass）ときは進めない（フレームごとに 1 回）
    if !app.view3d.visible || !app.view3d.view.display.light_spin || ctx.current_pass_index() != 0 {
        return;
    }
    let dt = ctx.input(|i| i.unstable_dt);
    if advance(app, dt) {
        ctx.request_repaint();
    }
}
