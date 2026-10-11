//! 視点の操作をどこでも同じに: 3D の Alt + 左ドラッグのスナップ回転（正面・上へ吸い付く）・右ボタンを押している間の W/A/S/D/Q/E（視点の移動。
//! ほかのキーの割り当てに渡さない）・2D の Alt + 左ドラッグの回転（15° 刻み、Shift で自由）・押しの始めの Alt は視点で途中の Alt はツールの修飾・
//! 既定から外した組み合わせ（Shift + 右・Alt + Shift + 左・R + ドラッグ・Shift + 中）・ショートカットの一覧の名前。右ボタンのスポイトは `eyedrop`。
use crate::common;

use common::*;
use egui::{vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::state::{Action, Tool};
use yolu_app::view3d::{navigation, Nav};
use yolu_app::{Tab, YoluApp};
use yolu_core::glam::{Vec2, Vec3};

type H = Harness<'static, YoluApp>;

fn mouse(h: &H, at: Pos2, button: PointerButton, pressed: bool, modifiers: Modifiers) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button,
        pressed,
        modifiers,
    });
}

/// 修飾キーを押した（押したままにする。`NONE` で離す）。
fn mods(h: &mut H, m: Modifiers) {
    h.event(Event::ModifiersChanged(m));
    h.step();
}

fn key_event(h: &mut H, key: Key, pressed: bool, modifiers: Modifiers) {
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    });
    h.step();
}

fn move_mouse(h: &mut H, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
}

/// 表示域の中心から `radius`・`degrees` の点へ動かす。
fn move_on_circle(h: &mut H, radius: f32, degrees: f32) {
    let at = on_circle(h, radius, degrees);
    move_mouse(h, at);
}

fn camera(h: &H) -> yolu_core::geometry::OrbitCamera {
    h.state().state.view3d.camera
}

fn tool(h: &H) -> Tool {
    h.state().state.tool
}

fn nav(h: &H) -> Option<(Nav, PointerButton)> {
    h.state().state.view3d.input.nav
}

fn cube_view() -> (H, Rect) {
    let mut h = app(1100.0, 760.0, 256);
    h.state_mut().state.view3d.load_demo();
    h.state_mut().state.view3d.camera.yaw = -40.0;
    h.state_mut().state.view3d.camera.pitch = 15.0;
    click_tab(&mut h, Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect)
}

fn screen_of(h: &H, rect: Rect, p: Vec3) -> Pos2 {
    let view = camera(h).view(rect.width(), rect.height());
    let s = view.to_screen(p).expect("カメラの前");
    egui::pos2(rect.left() + s.x, rect.top() + s.y)
}

fn painted(h: &H) -> usize {
    let doc = &h.state().state.doc;
    let mut n = 0;
    for y in 0..doc.height() {
        for x in 0..doc.width() {
            if yolu_app::engine::composite_pixel(doc, x, y)[3] > 0 {
                n += 1;
            }
        }
    }
    n
}

fn strokes(h: &H) -> usize {
    h.state().state.doc.undo_count()
}

// ───────── 3D: Alt + 左ドラッグのスナップ回転 ─────────

#[test]
fn alt_drag_in_3d_snaps_to_the_front_and_leaves_it_when_dragged_past_the_angle() {
    let (mut h, rect) = cube_view();
    let at = rect.center();
    {
        let c = &mut h.state_mut().state.view3d.camera;
        c.yaw = 210.0;
        c.pitch = 0.0;
    }
    let alt = Modifiers::ALT;
    mods(&mut h, alt);
    mouse(&h, at, PointerButton::Primary, true, alt);
    h.step();
    assert_eq!(nav(&h), Some((Nav::SnapOrbit, PointerButton::Primary)));
    // yaw 210° → 190°（正面の 180° から 10°）: 正面へ吸い付く
    move_mouse(&mut h, at + vec2(-20.0 / 0.35, 0.0));
    assert_eq!((camera(&h).yaw, camera(&h).pitch), (180.0, 0.0));
    // 175°（5° 手前から越えた先）も正面のまま
    move_mouse(&mut h, at + vec2(-100.0, 0.0));
    assert_eq!((camera(&h).yaw, camera(&h).pitch), (180.0, 0.0));
    // 15° より外へ出すと、回した分の向きに戻る（吸い付いた向きから回し直しているのではない）
    move_mouse(&mut h, at + vec2(-200.0, 0.0));
    assert!((camera(&h).yaw - 140.0).abs() < 1e-3, "{}", camera(&h).yaw);
    assert_eq!(camera(&h).pitch, 0.0);
    // 戻して 199.5°（正面から 19.5°）: 吸い付かない
    move_mouse(&mut h, at + vec2(-30.0, 0.0));
    assert!((camera(&h).yaw - 199.5).abs() < 1e-3, "{}", camera(&h).yaw);
    mouse(
        &h,
        at + vec2(-30.0, 0.0),
        PointerButton::Primary,
        false,
        alt,
    );
    h.step();
    mods(&mut h, Modifiers::NONE);
    assert!(nav(&h).is_none());
    assert!(
        !h.state().state.doc.can_undo(),
        "回すだけで文書は変わらない"
    );
    assert!(!h.state().state.is_stroking());
}

#[test]
fn alt_drag_in_3d_snaps_to_the_other_axis_views_too() {
    // 向きが軸のどれかに 15° 以内で近ければ、その軸の向きになる（背面・右・左）
    for (yaw, pitch, wanted) in [
        (12.0f32, 6.0f32, (0.0f32, 0.0f32)),
        (-78.0, -8.0, (-90.0, 0.0)),
        (101.0, 9.0, (90.0, 0.0)),
    ] {
        let (mut h, rect) = cube_view();
        let at = rect.center();
        {
            // 吸い付く範囲の外から始めて、そこまで回す
            let c = &mut h.state_mut().state.view3d.camera;
            c.yaw = yaw + 40.0;
            c.pitch = pitch;
        }
        mods(&mut h, Modifiers::ALT);
        mouse(&h, at, PointerButton::Primary, true, Modifiers::ALT);
        h.step();
        move_mouse(&mut h, at + vec2(-40.0 / 0.35, 0.0));
        let c = camera(&h);
        assert_eq!((c.yaw, c.pitch), wanted, "{yaw} {pitch}");
        mouse(&h, at, PointerButton::Primary, false, Modifiers::ALT);
        h.step();
        mods(&mut h, Modifiers::NONE);
    }
}

#[test]
fn snapping_to_the_top_view_keeps_the_view_working_and_the_surface_paintable() {
    let (mut h, rect) = cube_view();
    let at = rect.center();
    {
        let c = &mut h.state_mut().state.view3d.camera;
        c.yaw = 180.0;
        c.pitch = 70.0;
    }
    mods(&mut h, Modifiers::ALT);
    mouse(&h, at, PointerButton::Primary, true, Modifiers::ALT);
    h.step();
    // pitch 70° → 80°: 真上から 10°。上へ吸い付いて ±90°
    let moved = at + vec2(0.0, 10.0 / 0.35);
    move_mouse(&mut h, moved);
    let c = camera(&h);
    assert_eq!((c.yaw, c.pitch), (180.0, 90.0));
    mouse(&h, moved, PointerButton::Primary, false, Modifiers::ALT);
    h.step();
    mods(&mut h, Modifiers::NONE);
    assert_eq!(camera(&h).pitch, 90.0, "離しても真上のまま");
    // 真上の見え方で、画面の中心は注視点（レイも写しも壊れていない）
    let view = c.view(rect.width(), rect.height());
    let center = view.to_screen(c.target).expect("カメラの前");
    assert!((center - Vec2::new(rect.width() / 2.0, rect.height() / 2.0)).length() < 1e-2);
    // 上の面（y = 0.5）に描ける
    let on_top = screen_of(&h, rect, Vec3::new(0.0, 0.5, 0.0));
    drag(&mut h, &[on_top, offset(on_top, 8.0, 0.0)]);
    assert_eq!(strokes(&h), 1);
    assert!(painted(&h) > 0, "真上から面に描けた");
    assert_eq!(camera(&h).pitch, 90.0, "描いても向きは変わらない");
    // そこからもう一度回せる（pitch が ±90° でも orbit が壊れない）
    mouse(&h, at, PointerButton::Secondary, true, Modifiers::NONE);
    h.step();
    move_mouse(&mut h, at + vec2(20.0, -20.0));
    mouse(
        &h,
        at + vec2(20.0, -20.0),
        PointerButton::Secondary,
        false,
        Modifiers::NONE,
    );
    h.step();
    let after = camera(&h);
    assert!(after.pitch.is_finite() && after.pitch <= 89.0 && after.pitch > 80.0);
    assert!(after.yaw > 180.0);
}

#[test]
fn alt_click_without_moving_still_sets_the_clone_source_and_dragging_snap_orbits_instead() {
    let (mut h, rect) = cube_view();
    {
        let s = &mut h.state_mut().state;
        s.m2.brush.effect = yolu_app::engine::BrushEffect::Clone {
            offset: Default::default(),
        };
        s.view3d.camera.yaw = 210.0;
        s.view3d.camera.pitch = 0.0;
    }
    let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    let alt = Modifiers::ALT;
    // 動かさずに離す: 元を決める（視点は動かない）
    let before = camera(&h);
    mods(&mut h, alt);
    mouse(&h, at, PointerButton::Primary, true, alt);
    h.step();
    assert!(h.state().state.view3d.input.clone_press.is_some());
    mouse(&h, at, PointerButton::Primary, false, alt);
    h.step();
    mods(&mut h, Modifiers::NONE);
    assert_eq!(camera(&h), before, "クリックで視点は動かない");
    assert!(h.state().state.clone.source.is_some(), "元を決めた");
    // 動かしたらスナップ回転（元は決めない）
    h.state_mut().state.clone.source = None;
    mods(&mut h, alt);
    mouse(&h, at, PointerButton::Primary, true, alt);
    h.step();
    move_mouse(&mut h, at + vec2(-60.0, 0.0));
    mouse(
        &h,
        at + vec2(-60.0, 0.0),
        PointerButton::Primary,
        false,
        alt,
    );
    h.step();
    mods(&mut h, Modifiers::NONE);
    assert!(camera(&h).yaw < before.yaw, "回った");
    assert!(
        h.state().state.clone.source.is_none(),
        "動かしたら元は決めない"
    );
    assert!(!h.state().state.doc.can_undo());
}

// ───────── 3D: 右ボタンを押している間の W/A/S/D/Q/E ─────────

fn fly_speed_per_frame(h: &H) -> f32 {
    camera(h).model_radius * navigation::FLY_SPEED / 60.0
}

/// 右ボタンを押す（動かさない）。
fn press_right(h: &mut H, at: Pos2) {
    mouse(h, at, PointerButton::Secondary, true, Modifiers::NONE);
    h.step();
    assert_eq!(nav(h), Some((Nav::Orbit, PointerButton::Secondary)));
}

fn release_right(h: &mut H, at: Pos2) {
    mouse(h, at, PointerButton::Secondary, false, Modifiers::NONE);
    h.step();
}

#[test]
fn w_moves_the_view_forward_while_the_right_button_is_down_and_never_switches_the_tool() {
    let (mut h, rect) = cube_view();
    let at = rect.center();
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    h.run();
    let before = camera(&h);
    press_right(&mut h, at);
    // W を押す（そのフレームから動く）。10 フレーム
    key_event(&mut h, Key::W, true, Modifiers::NONE);
    for _ in 0..9 {
        h.step();
    }
    let after = camera(&h);
    let moved = after.target - before.target;
    let expected = 10.0 * fly_speed_per_frame(&h);
    assert!(
        (moved.length() - expected).abs() < expected * 0.1,
        "動いた量 {} / 予想 {expected}",
        moved.length()
    );
    let forward = before.rotation() * Vec3::Z;
    assert!(moved.normalize().dot(forward) > 0.9999, "カメラの向きへ");
    // カメラも一緒に動き、距離と向きは変わらない
    assert!(((after.position() - before.position()) - moved).length() < 1e-5);
    assert_eq!(after.distance, before.distance);
    assert_eq!((after.yaw, after.pitch), (before.yaw, before.pitch));
    // ツールは替わらない（W は自動選択の割り当て）
    assert_eq!(tool(&h), Tool::Brush);
    key_event(&mut h, Key::W, false, Modifiers::NONE);
    // W を離したら止まる
    let stopped = camera(&h).target;
    for _ in 0..3 {
        h.step();
    }
    assert_eq!(camera(&h).target, stopped);
    release_right(&mut h, at);
    assert!(!h.state().state.doc.can_undo());
    // 右ボタンを離していれば、W は今までどおり自動選択のツールに替わる
    let still = camera(&h).target;
    key_event(&mut h, Key::W, true, Modifiers::NONE);
    assert_eq!(tool(&h), Tool::Wand, "右を押していなければ W は自動選択");
    assert_eq!(camera(&h).target, still, "動かさない");
}

#[test]
fn s_a_d_q_e_move_the_view_along_the_camera_axes_and_shift_is_three_times_faster() {
    let table = [
        (Key::S, Vec3::NEG_Z),
        (Key::A, Vec3::NEG_X),
        (Key::D, Vec3::X),
        (Key::Q, Vec3::NEG_Y),
        (Key::E, Vec3::Y),
    ];
    for (key, local) in table {
        let (mut h, rect) = cube_view();
        let at = rect.center();
        let before = camera(&h);
        press_right(&mut h, at);
        key_event(&mut h, key, true, Modifiers::NONE);
        for _ in 0..4 {
            h.step();
        }
        let moved = camera(&h).target - before.target;
        let wanted = before.rotation() * local;
        assert!(moved.length() > 0.0, "{key:?}");
        assert!(
            moved.normalize().dot(wanted) > 0.9999,
            "{key:?}: {moved} / {wanted}"
        );
        assert!(
            (moved.length() - 5.0 * fly_speed_per_frame(&h)).abs()
                < 5.0 * fly_speed_per_frame(&h) * 0.1,
            "{key:?}"
        );
    }
    // Shift で 3 倍
    let (mut h, rect) = cube_view();
    let at = rect.center();
    let before = camera(&h);
    press_right(&mut h, at);
    mods(&mut h, Modifiers::SHIFT);
    key_event(&mut h, Key::W, true, Modifiers::SHIFT);
    for _ in 0..4 {
        h.step();
    }
    let moved = camera(&h).target - before.target;
    let expected = 3.0 * 5.0 * fly_speed_per_frame(&h);
    assert!(
        (moved.length() - expected).abs() < expected * 0.1,
        "{} / {expected}",
        moved.length()
    );
    // 反対のキーを一緒に押せば打ち消し合う。斜めは速くならない（W + D は 1 つ分の速さ）
    let target = camera(&h).target;
    key_event(&mut h, Key::S, true, Modifiers::SHIFT);
    for _ in 0..3 {
        h.step();
    }
    assert_eq!(camera(&h).target, target, "W と S は打ち消し合う");
    key_event(&mut h, Key::S, false, Modifiers::SHIFT);
    key_event(&mut h, Key::D, true, Modifiers::SHIFT);
    let mid = camera(&h).target;
    h.step();
    let step = camera(&h).target - mid;
    assert!(
        (step.length() - 3.0 * fly_speed_per_frame(&h)).abs() < 3.0 * fly_speed_per_frame(&h) * 0.1,
        "斜めでも 1 つ分の速さ: {}",
        step.length()
    );
}

#[test]
fn the_keys_are_not_passed_to_the_shortcut_table_while_the_right_button_is_down() {
    let (mut h, rect) = cube_view();
    let at = rect.center();
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    h.state_mut().state.color.set_main([1.0, 0.0, 0.0, 1.0]);
    h.run();
    press_right(&mut h, at);
    // E（消しゴム）・S（選択ペン）・W（自動選択）・D（初期の色）・Q（取っ手）はどれも割り当てへ行かない
    for key in [Key::E, Key::S, Key::W, Key::D, Key::Q, Key::A] {
        key_event(&mut h, key, true, Modifiers::NONE);
        key_event(&mut h, key, false, Modifiers::NONE);
    }
    assert_eq!(tool(&h), Tool::Brush);
    assert_eq!(
        h.state().state.color.main,
        [1.0, 0.0, 0.0, 1.0],
        "D は色を替えない"
    );
    release_right(&mut h, at);
    // 右を離せば、同じキーが割り当てどおりに働く
    key_event(&mut h, Key::D, true, Modifiers::NONE);
    key_event(&mut h, Key::D, false, Modifiers::NONE);
    assert_eq!(
        h.state().state.color.main,
        [0.0, 0.0, 0.0, 1.0],
        "D は初期の色"
    );
    key_event(&mut h, Key::E, true, Modifiers::NONE);
    assert_eq!(tool(&h), Tool::Eraser);
}

#[test]
fn ctrl_keys_still_reach_the_table_while_the_right_button_is_down() {
    let (mut h, rect) = cube_view();
    let at = rect.center();
    press_right(&mut h, at);
    let before = camera(&h).target;
    // Ctrl+A（すべてを選択）は視点の移動ではなく、割り当てのまま
    // winit は Linux・Windows では Ctrl を ctrl と command の両方で届ける
    let ctrl = Modifiers::CTRL | Modifiers::COMMAND;
    mods(&mut h, ctrl);
    key_event(&mut h, Key::A, true, ctrl);
    for _ in 0..3 {
        h.step();
    }
    assert_eq!(camera(&h).target, before, "Ctrl を押していれば動かさない");
    assert!(h.state().state.doc.selection().is_some(), "Ctrl+A は効く");
}

#[test]
fn the_view_stops_flying_when_the_right_button_is_released_escape_is_pressed_or_the_focus_is_lost()
{
    for how in ["release", "escape", "focus"] {
        let (mut h, rect) = cube_view();
        let at = rect.center();
        let started = camera(&h).target;
        press_right(&mut h, at);
        key_event(&mut h, Key::W, true, Modifiers::NONE);
        for _ in 0..3 {
            h.step();
        }
        assert_ne!(camera(&h).target, started, "動いている");
        match how {
            "release" => release_right(&mut h, at),
            "escape" => key_event(&mut h, Key::Escape, true, Modifiers::NONE),
            _ => {
                h.event(Event::WindowFocused(false));
                h.step();
            }
        }
        assert!(nav(&h).is_none(), "{how}: 視点の操作は終わった");
        let stopped = camera(&h).target;
        for _ in 0..4 {
            h.step();
        }
        assert_eq!(
            camera(&h).target,
            stopped,
            "{how}: W を押したままでも止まる"
        );
    }
}

/// 繰り返し（OS のキーの繰り返し）の押し。
fn key_repeat(h: &mut H, key: Key, modifiers: Modifiers) {
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: true,
        modifiers,
    });
    h.step();
}

#[test]
fn releasing_the_right_button_first_keeps_the_held_fly_keys_off_the_table_until_they_are_released()
{
    let (mut h, rect) = cube_view();
    let at = rect.center();
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    h.state_mut().state.color.set_main([1.0, 0.0, 0.0, 1.0]);
    h.run();
    press_right(&mut h, at);
    // 移動の間に W・D・Shift+Q を押したままにする
    key_event(&mut h, Key::W, true, Modifiers::NONE);
    key_event(&mut h, Key::D, true, Modifiers::NONE);
    mods(&mut h, Modifiers::SHIFT);
    key_event(&mut h, Key::Q, true, Modifiers::SHIFT);
    // 右ボタンだけを先に離す。キーは押したまま（OS のキーの繰り返し）
    release_right(&mut h, at);
    assert!(nav(&h).is_none());
    let before = camera(&h).target;
    for _ in 0..3 {
        key_repeat(&mut h, Key::W, Modifiers::SHIFT);
        key_repeat(&mut h, Key::D, Modifiers::SHIFT);
        key_repeat(&mut h, Key::Q, Modifiers::SHIFT);
    }
    assert_eq!(tool(&h), Tool::Brush, "W（自動選択）に替わらない");
    assert_eq!(
        h.state().state.color.main,
        [1.0, 0.0, 0.0, 1.0],
        "D（初期の色）に戻らない"
    );
    assert!(
        !h.state().state.sel.quick,
        "Shift+Q（クイックマスク）が入らない"
    );
    assert_eq!(camera(&h).target, before, "右を離したので、もう動かさない");
    // Shift を離し、修飾なしの繰り返しでも同じ
    mods(&mut h, Modifiers::NONE);
    key_repeat(&mut h, Key::W, Modifiers::NONE);
    key_repeat(&mut h, Key::D, Modifiers::NONE);
    assert_eq!(tool(&h), Tool::Brush);
    assert_eq!(h.state().state.color.main, [1.0, 0.0, 0.0, 1.0]);
    // 押していなかったキーは、今までどおり表へ届く（X は色の入れ替え）
    key_event(&mut h, Key::X, true, Modifiers::NONE);
    key_event(&mut h, Key::X, false, Modifiers::NONE);
    assert_eq!(h.state().state.color.sub, [1.0, 0.0, 0.0, 1.0], "X は効く");
    // W を離して押し直せば、表へ届く（自動選択）。D・Q は押したままなので、まだ届かない
    key_event(&mut h, Key::W, false, Modifiers::NONE);
    key_event(&mut h, Key::W, true, Modifiers::NONE);
    assert_eq!(tool(&h), Tool::Wand, "押し直した W は自動選択");
    key_repeat(&mut h, Key::D, Modifiers::NONE);
    assert_eq!(
        h.state().state.color.sub,
        [1.0, 0.0, 0.0, 1.0],
        "押したままの D はまだ届かない"
    );
    // D も離して押し直せば、初期の色
    key_event(&mut h, Key::D, false, Modifiers::NONE);
    key_event(&mut h, Key::D, true, Modifiers::NONE);
    assert_eq!(
        h.state().state.color.main,
        [0.0, 0.0, 0.0, 1.0],
        "押し直した D は初期の色"
    );
}

#[test]
fn a_key_held_before_the_right_button_and_repeating_after_it_is_released_stays_off_the_table() {
    let (mut h, rect) = cube_view();
    let at = rect.center();
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    h.run();
    // 右ボタンを押す前から W を押している（そのとき W は表へ届く）
    key_event(&mut h, Key::W, true, Modifiers::NONE);
    assert_eq!(tool(&h), Tool::Wand);
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    press_right(&mut h, at);
    key_repeat(&mut h, Key::W, Modifiers::NONE);
    release_right(&mut h, at);
    key_repeat(&mut h, Key::W, Modifiers::NONE);
    assert_eq!(tool(&h), Tool::Brush, "右を離したあとの繰り返しも届かない");
}

/// 視点の押した所の面の点（押す前のカメラで引く）。
fn surface_point_at(h: &H, rect: Rect, at: Pos2) -> Vec3 {
    let model = h.state().state.view3d.model.clone().expect("モデル");
    yolu_core::geometry::pick(
        &model.geometry,
        &camera(h).view(rect.width(), rect.height()),
        Vec2::new(at.x - rect.left(), at.y - rect.top()),
    )
    .expect("面に当たる")
    .position
}

#[test]
fn a_small_wobble_of_an_alt_click_or_a_right_click_neither_snaps_the_view_nor_moves_the_picked_point(
) {
    // Alt + クリック（クローンの元）: 軸の近くの向き（正面の 10° 手前）で 4 点以内の揺れ。視点は変わらず、元は押した所の面の点
    let (mut h, rect) = cube_view();
    {
        let s = &mut h.state_mut().state;
        s.m2.brush.effect = yolu_app::engine::BrushEffect::Clone {
            offset: Default::default(),
        };
        s.view3d.camera.yaw = 190.0;
        s.view3d.camera.pitch = 0.0;
    }
    let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, 0.5));
    let expected = surface_point_at(&h, rect, at);
    let before = camera(&h);
    let alt = Modifiers::ALT;
    mods(&mut h, alt);
    mouse(&h, at, PointerButton::Primary, true, alt);
    h.step();
    for wobble in [vec2(2.0, 1.0), vec2(-1.0, 2.0), vec2(2.0, 1.0)] {
        move_mouse(&mut h, at + wobble);
        assert_eq!(camera(&h), before, "遊びの内は回さない（スナップもしない）");
    }
    mouse(&h, at + vec2(2.0, 1.0), PointerButton::Primary, false, alt);
    h.step();
    mods(&mut h, Modifiers::NONE);
    assert_eq!(camera(&h), before);
    let source = h.state().state.clone.source.expect("元を決めた");
    assert!(
        (source.position - expected).length() < 1e-4,
        "元は押した所の面の点: {} / {expected}",
        source.position
    );
    // 右クリック（スポイト）: 揺れでは視点が動かず、スポイトの候補も残る
    h.state_mut().state.clone.source = None;
    h.state_mut().state.color.set_main([0.0, 1.0, 0.0, 1.0]);
    mouse(&h, at, PointerButton::Secondary, true, Modifiers::NONE);
    h.step();
    move_mouse(&mut h, at + vec2(3.0, 1.0));
    assert_eq!(camera(&h), before, "右ドラッグも遊びの内は回さない");
    assert!(
        h.state().state.view3d.input.eyedrop.is_some(),
        "印と取る点が揃う（まだ候補）"
    );
    mouse(
        &h,
        at + vec2(3.0, 1.0),
        PointerButton::Secondary,
        false,
        Modifiers::NONE,
    );
    h.step();
    assert_eq!(camera(&h), before);
    assert!(h.state().state.view3d.input.eyedrop.is_none());
}

#[test]
fn past_the_wobble_the_whole_motion_from_the_pressed_point_is_applied() {
    // 右ドラッグ: 4 点以内の揺れのあと 60 点動かすと、押した所からの 60 点ぶんの回転（遊びの分を引かない）
    let (mut h, rect) = cube_view();
    let at = rect.center();
    h.state_mut().state.view3d.camera.yaw = 190.0;
    h.state_mut().state.view3d.camera.pitch = 0.0;
    mouse(&h, at, PointerButton::Secondary, true, Modifiers::NONE);
    h.step();
    move_mouse(&mut h, at + vec2(2.0, 1.0));
    assert_eq!(camera(&h).yaw, 190.0);
    move_mouse(&mut h, at + vec2(60.0, 0.0));
    let c = camera(&h);
    assert!((c.yaw - (190.0 + 60.0 * 0.35)).abs() < 1e-3, "{}", c.yaw);
    assert!(c.pitch.abs() < 1e-3);
    assert!(
        h.state().state.view3d.input.eyedrop.is_none(),
        "動かしたのでスポイトにしない"
    );
    // そのあとは、動いた分ずつ
    move_mouse(&mut h, at + vec2(80.0, 0.0));
    assert!((camera(&h).yaw - (190.0 + 80.0 * 0.35)).abs() < 1e-3);
    mouse(
        &h,
        at + vec2(80.0, 0.0),
        PointerButton::Secondary,
        false,
        Modifiers::NONE,
    );
    h.step();
    // Alt + 左ドラッグ（スナップ回転）: 同じ。軸から離れた向きまで動かすと、押した所からの動きが全部当たる
    h.state_mut().state.view3d.camera.yaw = 190.0;
    let alt = Modifiers::ALT;
    mods(&mut h, alt);
    mouse(&h, at, PointerButton::Primary, true, alt);
    h.step();
    move_mouse(&mut h, at + vec2(1.0, 2.0));
    assert_eq!(camera(&h).yaw, 190.0);
    move_mouse(&mut h, at + vec2(60.0, 0.0));
    assert!((camera(&h).yaw - 211.0).abs() < 1e-3, "{}", camera(&h).yaw);
    mouse(&h, at + vec2(60.0, 0.0), PointerButton::Primary, false, alt);
    h.step();
    mods(&mut h, Modifiers::NONE);
    // パン（Space + 左）には遊びを入れない（今までどおり 1 点から動く）
    key_event(&mut h, Key::Space, true, Modifiers::NONE);
    let target = camera(&h).target;
    mouse(&h, at, PointerButton::Primary, true, Modifiers::NONE);
    h.step();
    move_mouse(&mut h, at + vec2(2.0, 0.0));
    assert_ne!(camera(&h).target, target, "パンは遊びなしで動く");
}

#[test]
fn right_dragging_from_straight_above_does_not_jump_by_a_degree_first() {
    let (mut h, rect) = cube_view();
    let at = rect.center();
    {
        let c = &mut h.state_mut().state.view3d.camera;
        c.yaw = 180.0;
        c.pitch = 90.0;
    }
    mouse(&h, at, PointerButton::Secondary, true, Modifiers::NONE);
    h.step();
    // 横にだけ動かす: pitch は 90° のまま（89° に止められない）
    move_mouse(&mut h, at + vec2(20.0, 0.0));
    assert_eq!(camera(&h).pitch, 90.0);
    // さらに向こうへ（下へ）動かしても 90° のまま
    move_mouse(&mut h, at + vec2(20.0, 20.0));
    assert_eq!(camera(&h).pitch, 90.0);
    // 戻す向き（上へ）には、動いた分だけなめらかに離れる
    move_mouse(&mut h, at + vec2(20.0, -20.0));
    assert!(
        (camera(&h).pitch - (90.0 - 40.0 * 0.35)).abs() < 1e-3,
        "{}",
        camera(&h).pitch
    );
}

#[test]
fn flying_makes_the_right_button_release_no_longer_a_click() {
    let (mut h, rect) = cube_view();
    let at = rect.center();
    h.state_mut().state.color.set_main([0.0, 1.0, 0.0, 1.0]);
    press_right(&mut h, at);
    assert!(
        h.state().state.view3d.input.eyedrop.is_some(),
        "押しただけならスポイトの候補"
    );
    key_event(&mut h, Key::W, true, Modifiers::NONE);
    h.step();
    assert!(
        h.state().state.view3d.input.eyedrop.is_none(),
        "動かしたらスポイトにしない"
    );
    key_event(&mut h, Key::W, false, Modifiers::NONE);
    release_right(&mut h, at);
    h.run();
    assert_eq!(
        h.state().state.color.main,
        [0.0, 1.0, 0.0, 1.0],
        "色は変わらない"
    );
}

// ───────── 3D: 外した組み合わせ ─────────

#[test]
fn shift_with_the_right_button_and_alt_shift_with_the_left_no_longer_pan_in_3d() {
    let (mut h, rect) = cube_view();
    let at = rect.center();
    let start = camera(&h);
    // Shift + 右ドラッグ: パンしない（回す）
    mods(&mut h, Modifiers::SHIFT);
    mouse(&h, at, PointerButton::Secondary, true, Modifiers::SHIFT);
    h.step();
    move_mouse(&mut h, at + vec2(40.0, 10.0));
    mouse(
        &h,
        at + vec2(40.0, 10.0),
        PointerButton::Secondary,
        false,
        Modifiers::SHIFT,
    );
    h.step();
    mods(&mut h, Modifiers::NONE);
    let after = camera(&h);
    assert_eq!(after.target, start.target, "Shift + 右でパンしない");
    assert_ne!(after.yaw, start.yaw, "回す");
    // Alt + Shift + 左ドラッグ: パンしない（スナップ回転）
    let start = camera(&h);
    let both = Modifiers::ALT | Modifiers::SHIFT;
    mods(&mut h, both);
    mouse(&h, at, PointerButton::Primary, true, both);
    h.step();
    move_mouse(&mut h, at + vec2(40.0, 10.0));
    mouse(
        &h,
        at + vec2(40.0, 10.0),
        PointerButton::Primary,
        false,
        both,
    );
    h.step();
    mods(&mut h, Modifiers::NONE);
    assert_eq!(
        camera(&h).target,
        start.target,
        "Alt + Shift + 左でパンしない"
    );
    assert_ne!(camera(&h).yaw, start.yaw);
    assert!(!h.state().state.doc.can_undo());
    // Shift + 右を動かさずに離しても、スポイトにはならない（修飾は書いたとおり）
    h.state_mut().state.color.set_main([0.0, 1.0, 0.0, 1.0]);
    mods(&mut h, Modifiers::SHIFT);
    mouse(&h, at, PointerButton::Secondary, true, Modifiers::SHIFT);
    h.step();
    assert!(h.state().state.view3d.input.eyedrop.is_none());
    mouse(&h, at, PointerButton::Secondary, false, Modifiers::SHIFT);
    h.step();
    mods(&mut h, Modifiers::NONE);
    assert_eq!(h.state().state.color.main, [0.0, 1.0, 0.0, 1.0]);
}

// ───────── 2D: Alt + 左ドラッグの回転 ─────────

/// 表示域の中心から `radius` の、`degrees`（画面で時計回りが正、0 が右）の点。
fn on_circle(h: &H, radius: f32, degrees: f32) -> Pos2 {
    let c = canvas_rect(h).center();
    let a = degrees.to_radians();
    c + vec2(radius * a.cos(), radius * a.sin())
}

fn angle(h: &H) -> f32 {
    h.state().state.view.angle
}

fn near_step(degrees: f32) -> bool {
    (degrees / 15.0 - (degrees / 15.0).round()).abs() < 1e-3
}

#[test]
fn alt_drag_rotates_the_2d_view_in_15_degree_steps_and_shift_makes_it_free() {
    let mut h = app(1280.0, 800.0, 256);
    let alt = Modifiers::ALT;
    // 0° から 0 → 40 → 78°（15° の倍数でない）まで回す
    mods(&mut h, alt);
    mouse(
        &h,
        on_circle(&h, 150.0, 0.0),
        PointerButton::Primary,
        true,
        alt,
    );
    h.step();
    assert!(h.state().state.canvas.rotating.is_some());
    let mut seen = Vec::new();
    for degrees in [10.0, 22.0, 40.0, 61.0, 78.0] {
        move_on_circle(&mut h, 150.0, degrees);
        seen.push(angle(&h));
    }
    assert!(seen.iter().all(|a| near_step(*a)), "15° ごと: {seen:?}");
    assert_eq!(seen.last(), Some(&75.0), "78° は 75° に");
    mouse(
        &h,
        on_circle(&h, 150.0, 78.0),
        PointerButton::Primary,
        false,
        alt,
    );
    h.step();
    mods(&mut h, Modifiers::NONE);
    assert!(h.state().state.canvas.rotating.is_none());
    assert_eq!(strokes(&h), 0, "描かない");
    // Shift も押していれば自由（押している間に切り替わる）
    h.state_mut().state.view.set_angle(0.0);
    let both = Modifiers::ALT | Modifiers::SHIFT;
    mods(&mut h, both);
    mouse(
        &h,
        on_circle(&h, 150.0, 0.0),
        PointerButton::Primary,
        true,
        both,
    );
    h.step();
    move_on_circle(&mut h, 150.0, 22.0);
    assert!((angle(&h) - 22.0).abs() < 0.5, "自由: {}", angle(&h));
    assert!(!near_step(angle(&h)));
    // Shift を離せば、15° 刻みに戻る
    mods(&mut h, Modifiers::ALT);
    move_on_circle(&mut h, 150.0, 23.0);
    assert!(near_step(angle(&h)), "{}", angle(&h));
    mouse(
        &h,
        on_circle(&h, 150.0, 23.0),
        PointerButton::Primary,
        false,
        Modifiers::ALT,
    );
    h.step();
    mods(&mut h, Modifiers::NONE);
}

#[test]
fn escape_during_the_2d_rotation_returns_to_the_starting_angle() {
    let mut h = app(1280.0, 800.0, 256);
    let alt = Modifiers::ALT;
    mods(&mut h, alt);
    mouse(
        &h,
        on_circle(&h, 150.0, 0.0),
        PointerButton::Primary,
        true,
        alt,
    );
    h.step();
    move_on_circle(&mut h, 150.0, 50.0);
    assert_ne!(angle(&h), 0.0);
    key_event(&mut h, Key::Escape, true, alt);
    assert_eq!(angle(&h), 0.0, "Esc で元の角度へ");
    mouse(
        &h,
        on_circle(&h, 150.0, 50.0),
        PointerButton::Primary,
        false,
        alt,
    );
    h.step();
    mods(&mut h, Modifiers::NONE);
}

#[test]
fn r_drag_and_shift_middle_drag_no_longer_rotate_the_2d_view() {
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    let center = canvas_rect(&h).center();
    // R を押しながらの左ドラッグ: 回さない（描く）
    key_event(&mut h, Key::R, true, Modifiers::NONE);
    drag(
        &mut h,
        &[
            offset(center, 100.0, 0.0),
            offset(center, 70.0, 70.0),
            offset(center, 0.0, 100.0),
        ],
    );
    key_event(&mut h, Key::R, false, Modifiers::NONE);
    assert_eq!(angle(&h), 0.0, "R + ドラッグで回らない");
    assert_eq!(strokes(&h), 1, "R を押していても描く");
    // Shift + 中ボタンのドラッグ: 回さない（パン）
    let before = h.state().state.view.pan;
    mods(&mut h, Modifiers::SHIFT);
    mouse(&h, center, PointerButton::Middle, true, Modifiers::SHIFT);
    h.step();
    assert!(h.state().state.canvas.rotating.is_none() && h.state().state.canvas.panning);
    move_mouse(&mut h, offset(center, 30.0, 10.0));
    mouse(
        &h,
        offset(center, 30.0, 10.0),
        PointerButton::Middle,
        false,
        Modifiers::SHIFT,
    );
    h.step();
    mods(&mut h, Modifiers::NONE);
    assert_eq!(angle(&h), 0.0, "Shift + 中ボタンで回らない");
    assert_ne!(h.state().state.view.pan, before, "動かすだけ（パン）");
}

// ───────── 押しの始めの Alt は視点、途中の Alt はツール ─────────

/// 文書の点 (x, y) の画面の点。
fn doc_point(h: &H, x: f64, y: f64) -> Pos2 {
    let s = &h.state().state;
    s.view
        .view(canvas_rect(h), s.doc.width(), s.doc.height())
        .to_screen(x, y)
}

fn selected(h: &H, x: u32, y: u32) -> bool {
    h.state()
        .state
        .doc
        .selection()
        .is_some_and(|m| m.amount(x, y) > 0)
}

#[test]
fn an_alt_held_at_the_start_rotates_the_view_but_one_pressed_during_the_drag_is_the_tools_modifier()
{
    // 長方形選択: 押したあとに Alt を押すと「中心から」。押した点の反対側にも選択が伸びる
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut()
        .state
        .apply(Action::SelectTool(Tool::SelectRect));
    h.run();
    let (p0, p1) = (doc_point(&h, 128.0, 128.0), doc_point(&h, 168.0, 158.0));
    mouse(&h, p0, PointerButton::Primary, true, Modifiers::NONE);
    h.step();
    mods(&mut h, Modifiers::ALT);
    move_mouse(&mut h, p1);
    mouse(&h, p1, PointerButton::Primary, false, Modifiers::ALT);
    h.step();
    mods(&mut h, Modifiers::NONE);
    h.run();
    assert!(selected(&h, 150, 140), "押した点から動かした側");
    assert!(
        selected(&h, 106, 116),
        "Alt を途中で押したので、反対側にも伸びる（中心から）"
    );
    assert_eq!(angle(&h), 0.0, "回さない");
    // Alt を押さずに同じ操作: 反対側には伸びない
    h.state_mut()
        .state
        .apply(Action::Sel(yolu_app::selection::SelAction::Edit(
            yolu_app::selection::SelEdit::Clear,
        )));
    h.run();
    mouse(&h, p0, PointerButton::Primary, true, Modifiers::NONE);
    h.step();
    move_mouse(&mut h, p1);
    mouse(&h, p1, PointerButton::Primary, false, Modifiers::NONE);
    h.run();
    assert!(selected(&h, 150, 140));
    assert!(!selected(&h, 106, 116), "Alt なしは中心からにならない");
    // 押しの始めに Alt を持っていれば、選択を作らず表示を回す
    h.state_mut()
        .state
        .apply(Action::Sel(yolu_app::selection::SelAction::Edit(
            yolu_app::selection::SelEdit::Clear,
        )));
    h.run();
    mods(&mut h, Modifiers::ALT);
    mouse(&h, p0, PointerButton::Primary, true, Modifiers::ALT);
    h.step();
    assert!(h.state().state.canvas.rotating.is_some());
    let far = doc_point(&h, 200.0, 40.0);
    move_mouse(&mut h, far);
    mouse(&h, far, PointerButton::Primary, false, Modifiers::ALT);
    h.step();
    mods(&mut h, Modifiers::NONE);
    h.run();
    assert!(h.state().state.doc.selection().is_none(), "選択は作らない");
    assert_ne!(angle(&h), 0.0, "表示を回した");
}

#[test]
fn an_alt_pressed_after_a_brush_stroke_began_does_not_turn_it_into_a_rotation() {
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    h.run();
    let c = canvas_rect(&h).center();
    mouse(&h, c, PointerButton::Primary, true, Modifiers::NONE);
    h.step();
    mods(&mut h, Modifiers::ALT);
    move_mouse(&mut h, offset(c, 40.0, 0.0));
    move_mouse(&mut h, offset(c, 80.0, 0.0));
    assert!(
        h.state().state.canvas.rotating.is_none(),
        "始まった描画のまま"
    );
    mouse(
        &h,
        offset(c, 80.0, 0.0),
        PointerButton::Primary,
        false,
        Modifiers::ALT,
    );
    h.step();
    mods(&mut h, Modifiers::NONE);
    h.run();
    assert_eq!(strokes(&h), 1);
    assert_eq!(angle(&h), 0.0);
}

// ───────── ショートカットの一覧 ─────────

#[test]
fn the_default_key_tables_name_the_new_operations_in_both_languages() {
    use yolu_app::lang::Lang;
    for lang in Lang::ALL {
        // アプリの表から作る、既定の割り当ての表（設定のウィンドウの「ショートカット」の区分・文書と同じ表）
        let tables = yolu_app::shortcuts::guide::tables(lang);
        let has = |line: &str| tables.lines().any(|l| l == line);
        let left = lang.pick("左ボタン", "Left Button");
        let (open, close) = lang.pick(("（", "）"), (" (", ")"));
        let mode = lang.pick("どこでも・視点", "Everywhere & View");
        assert!(
            has(&format!(
                "| {}{open}3D{close} | Alt+{left} | {mode} |",
                lang.pick("スナップ回転", "Snap Orbit")
            )),
            "{lang:?}"
        );
        assert!(
            has(&format!(
                "| {}{open}2D{close} | Alt+{left} | {mode} |",
                lang.pick("回転", "Rotate")
            )),
            "2D の回転: {lang:?}"
        );
        assert!(
            has(&format!(
                "| {}{open}3D{close} | Alt+{} | {} |",
                lang.pick("クローンの元を決める", "Set Clone Source"),
                lang.pick(
                    "左ボタンを動かさずに離す",
                    "Left Button Released without Moving"
                ),
                lang.pick("ペイント", "Paint"),
            )),
            "{lang:?}"
        );
        for (ja, en, key) in [
            (
                "前へ移動（右ボタン中）",
                "Move Forward (While Right Button Held)",
                "W",
            ),
            (
                "後ろへ移動（右ボタン中）",
                "Move Back (While Right Button Held)",
                "S",
            ),
            (
                "左へ移動（右ボタン中）",
                "Move Left (While Right Button Held)",
                "A",
            ),
            (
                "右へ移動（右ボタン中）",
                "Move Right (While Right Button Held)",
                "D",
            ),
            (
                "下へ移動（右ボタン中）",
                "Move Down (While Right Button Held)",
                "Q",
            ),
            (
                "上へ移動（右ボタン中）",
                "Move Up (While Right Button Held)",
                "E",
            ),
        ] {
            assert!(
                has(&format!("| {} | `{key}` |", lang.pick(ja, en))),
                "{ja}: {lang:?}"
            );
        }
        // R を押しながらの回転は、既定では割り当てが無い（表に出ない）
        let everywhere = tables
            .split(&format!("### {}", lang.pick("ペイント", "Paint")))
            .next()
            .unwrap();
        assert!(!everywhere.contains("| `R` |"), "{lang:?}");
    }
}
