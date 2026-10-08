//! 2D のキャンバスと 3D ビューの入力を揃える: 3D の Shift + 押しの直線（マウスもペンも。前の終点から、前が無ければ 45° 刻みの固定）・Ctrl（ペンは描かず、
//! マウスは描く）・タッチの力が筆圧になること・バケツの近い色・クローンの設定（揃える・全レイヤーから）が 2D と 3D で一つであること。
//! 実際の入力（マウスのイベント・ペンの点・Touch）を流して確かめる。
use crate::common;
use crate::view3d_brush::{cube_view, press_with, release_with, screen_of};

use common::*;
use egui::{accesskit::Role, pos2, vec2, Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::engine::{composite_pixel, Rgba8, Tilt};
use yolu_app::pen::adjust::PressureAdjust;
use yolu_app::pen::PenSample;
use yolu_app::state::Tool;
use yolu_app::{Tab, YoluApp};
use yolu_core::geometry::pick;
use yolu_core::glam::{Vec2, Vec3};

type H = Harness<'static, YoluApp>;

const SIZE: u32 = 256;

fn painted_count(h: &H) -> usize {
    let doc = &h.state().state.doc;
    let mut n = 0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            if composite_pixel(doc, x, y)[3] > 0 {
                n += 1;
            }
        }
    }
    n
}

fn alpha_sum(h: &H) -> u64 {
    let doc = &h.state().state.doc;
    let mut n = 0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            n += composite_pixel(doc, x, y)[3] as u64;
        }
    }
    n
}

/// 画面の点の下の面の UV が指す画素（今のテクスチャセットの面だけ）。
fn texel_at(h: &H, rect: Rect, at: Pos2) -> Option<(u32, u32)> {
    let s = &h.state().state;
    let model = s.view3d.model.as_ref()?;
    let view = s.view3d.camera.view(rect.width(), rect.height());
    let hit = pick(
        &model.geometry,
        &view,
        Vec2::new(at.x - rect.left(), at.y - rect.top()),
    )?;
    (hit.material == s.view3d.material).then_some((
        ((hit.uv.x * SIZE as f32) as u32).min(SIZE - 1),
        ((hit.uv.y * SIZE as f32) as u32).min(SIZE - 1),
    ))
}

/// 画面の点の下の画素のまわり（半径 `r` 画素）に、何か塗られているか。
fn ink_near(h: &H, rect: Rect, at: Pos2, r: u32) -> bool {
    let Some((x, y)) = texel_at(h, rect, at) else {
        return false;
    };
    let doc = &h.state().state.doc;
    for yy in y.saturating_sub(r)..=(y + r).min(SIZE - 1) {
        for xx in x.saturating_sub(r)..=(x + r).min(SIZE - 1) {
            if composite_pixel(doc, xx, yy)[3] > 0 {
                return true;
            }
        }
    }
    false
}

fn hold(h: &mut H, m: Modifiers) {
    h.event(Event::ModifiersChanged(m));
    h.step();
}

fn strokes(h: &H) -> usize {
    h.state().state.doc.undo_count()
}

fn last_point(h: &H) -> Pos2 {
    h.state()
        .state
        .view3d
        .input
        .last_point
        .expect("描いているストロークの終点")
}

fn has_previous_end(h: &H) -> bool {
    h.state().state.view3d.input.previous_end.is_some()
}

/// 細い線で見分けやすくする（文書は 256 角）。
fn thin_brush(h: &mut H) {
    h.state_mut().state.brush.radius = 2.0;
    h.state_mut().state.brush.hardness = 1.0;
    h.state_mut().state.m2.random_seed = false;
}

fn sample(at: Pos2, contact: bool, time_ms: u32) -> PenSample {
    PenSample {
        pos: [at.x, at.y],
        pressure: if contact { 1.0 } else { 0.0 },
        tilt: Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 5,
        time_ms,
    }
}

/// ペンで `points` をなぞる（触れる → 動く → 離す。1 点ごとに 1 フレーム）。
fn pen_drag(h: &mut H, points: &[Pos2]) {
    for (i, p) in points.iter().enumerate() {
        h.state().pen().push(sample(*p, true, i as u32 * 10));
        h.step();
    }
    h.state()
        .pen()
        .push(sample(*points.last().unwrap(), false, 1000));
    h.step();
    h.run();
}

/// マウスで `points` をなぞる（押している修飾をイベントにも添える）。
fn mouse_drag(h: &mut H, points: &[Pos2], m: Modifiers) {
    press_with(h, points[0], m);
    h.step();
    for p in &points[1..] {
        move_to(h, *p);
        h.step();
    }
    release_with(h, *points.last().unwrap(), m);
    h.step();
    h.run();
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Device {
    Mouse,
    Pen,
}

fn drag_with(h: &mut H, device: Device, points: &[Pos2], m: Modifiers) {
    hold(h, m);
    match device {
        Device::Mouse => mouse_drag(h, points, m),
        Device::Pen => pen_drag(h, points),
    }
    hold(h, Modifiers::NONE);
}

fn world(h: &H, rect: Rect, x: f32, y: f32) -> Pos2 {
    screen_of(h, rect, Vec3::new(x, y, -0.5))
}

#[test]
fn shift_press_draws_a_line_from_the_previous_end_for_the_mouse_and_the_pen() {
    for device in [Device::Mouse, Device::Pen] {
        // Shift + 押し: 前のストロークの終点から押した点までの直線
        let (mut h, rect) = cube_view();
        thin_brush(&mut h);
        let (a, b) = (world(&h, rect, -0.3, 0.0), world(&h, rect, 0.3, 0.0));
        let mid = world(&h, rect, 0.0, 0.0);
        drag_with(&mut h, device, &[a, a + vec2(3.0, 0.0)], Modifiers::NONE);
        assert_eq!(strokes(&h), 1, "{device:?}");
        assert!(has_previous_end(&h), "{device:?}: 終点を面の点で覚える");
        assert!(!ink_near(&h, rect, mid, 2), "{device:?}: 線を引く前");
        drag_with(&mut h, device, &[b], Modifiers::SHIFT);
        assert_eq!(strokes(&h), 2, "{device:?}: 直線も 1 回の Undo");
        for t in [0.25, 0.5, 0.75] {
            let p = a + vec2(3.0, 0.0) + (b - a - vec2(3.0, 0.0)) * t;
            assert!(ink_near(&h, rect, p, 2), "{device:?}: t = {t}");
        }
        assert!(ink_near(&h, rect, b, 2), "{device:?}: 押した点まで");

        // Shift を押さない押しは、押した点だけ
        let (mut plain, rect) = cube_view();
        thin_brush(&mut plain);
        drag_with(
            &mut plain,
            device,
            &[a, a + vec2(3.0, 0.0)],
            Modifiers::NONE,
        );
        drag_with(&mut plain, device, &[b], Modifiers::NONE);
        assert!(!ink_near(&plain, rect, mid, 2), "{device:?}: 間は塗らない");
        assert!(painted_count(&h) > painted_count(&plain) * 3, "{device:?}");
        // 1 回の Undo で線が消える（最初のストロークは残る）
        h.state_mut().state.doc.undo().unwrap();
        h.run();
        assert!(!ink_near(&h, rect, mid, 2));
        assert!(ink_near(&h, rect, a, 2));
    }
}

#[test]
fn shift_press_then_drag_keeps_drawing_normally_past_the_jitter() {
    for device in [Device::Mouse, Device::Pen] {
        let (mut h, rect) = cube_view();
        thin_brush(&mut h);
        let (a, b) = (world(&h, rect, -0.3, 0.0), world(&h, rect, 0.1, 0.0));
        drag_with(&mut h, device, &[a, a + vec2(3.0, 0.0)], Modifiers::NONE);
        // Shift + 押して、そのままドラッグ: 押した点のぶれの内（8 点）は押した点のまま、超えたら普通に（向きを固定せずに）描く
        let around = [
            b,
            b + vec2(3.0, 2.0),
            b + vec2(10.0, 22.0),
            b + vec2(14.0, 40.0),
        ];
        hold(&mut h, Modifiers::SHIFT);
        match device {
            Device::Mouse => {
                press_with(&h, around[0], Modifiers::SHIFT);
                h.step();
                move_to(&h, around[1]);
                h.step();
                assert_eq!(last_point(&h), b, "{device:?}: ぶれの内は押した点のまま");
                for p in &around[2..] {
                    move_to(&h, *p);
                    h.step();
                }
                assert_eq!(last_point(&h), around[3], "{device:?}: 超えたら普通に描く");
                release_with(&h, around[3], Modifiers::SHIFT);
                h.step();
            }
            Device::Pen => {
                for (i, p) in around.iter().enumerate() {
                    h.state().pen().push(sample(*p, true, i as u32 * 10));
                    h.step();
                    if i == 1 {
                        assert_eq!(last_point(&h), b, "{device:?}: ぶれの内は押した点のまま");
                    }
                }
                assert_eq!(last_point(&h), around[3], "{device:?}: 超えたら普通に描く");
                h.state().pen().push(sample(around[3], false, 1000));
                h.step();
            }
        }
        hold(&mut h, Modifiers::NONE);
        h.run();
        assert!(ink_near(&h, rect, around[3], 2), "{device:?}");
        assert!(
            ink_near(&h, rect, b + vec2(12.0, 30.0), 3),
            "{device:?}: 動いた線の途中"
        );
        assert_eq!(strokes(&h), 2, "{device:?}");
    }
}

#[test]
fn shift_press_without_a_previous_end_locks_the_first_direction_to_45_degrees() {
    for device in [Device::Mouse, Device::Pen] {
        let (mut h, rect) = cube_view();
        thin_brush(&mut h);
        assert!(!has_previous_end(&h));
        let p = world(&h, rect, -0.2, 0.0);
        hold(&mut h, Modifiers::SHIFT);
        // 横から 12° ずれて動かす: 45° 刻みの最も近い向き（横）に固定される。ぶれの内は押した点のまま
        let far = p + vec2(60.0, 13.0);
        match device {
            Device::Mouse => {
                press_with(&h, p, Modifiers::SHIFT);
                h.step();
                move_to(&h, p + vec2(4.0, 1.0));
                h.step();
                assert_eq!(last_point(&h), p, "{device:?}: ぶれの内");
                move_to(&h, p + vec2(30.0, 6.0));
                h.step();
                move_to(&h, far);
                h.step();
                let end = last_point(&h);
                assert!((end.y - p.y).abs() < 1e-3, "{device:?}: 横に固定 {end:?}");
                assert!(end.x > p.x + 55.0, "{device:?}: {end:?}");
                release_with(&h, far, Modifiers::SHIFT);
                h.step();
            }
            Device::Pen => {
                for (i, q) in [p, p + vec2(4.0, 1.0), p + vec2(30.0, 6.0), far]
                    .into_iter()
                    .enumerate()
                {
                    h.state().pen().push(sample(q, true, i as u32 * 10));
                    h.step();
                }
                let end = last_point(&h);
                assert!((end.y - p.y).abs() < 1e-3, "{device:?}: 横に固定 {end:?}");
                h.state().pen().push(sample(far, false, 1000));
                h.step();
            }
        }
        hold(&mut h, Modifiers::NONE);
        h.run();
        assert!(
            ink_near(&h, rect, p + vec2(30.0, 0.0), 2),
            "{device:?}: 固定した線の上"
        );
        assert!(
            !ink_near(&h, rect, p + vec2(30.0, 6.0), 0),
            "{device:?}: 動かした線の上は塗らない"
        );
        // 前の終点は、このストロークの終点
        assert!(has_previous_end(&h), "{device:?}");
    }
}

#[test]
fn a_previous_end_from_another_model_is_not_used() {
    let (mut h, rect) = cube_view();
    thin_brush(&mut h);
    let (a, b) = (world(&h, rect, -0.3, 0.0), world(&h, rect, 0.3, 0.0));
    let mid = world(&h, rect, 0.0, 0.0);
    drag_with(
        &mut h,
        Device::Mouse,
        &[a, a + vec2(3.0, 0.0)],
        Modifiers::NONE,
    );
    assert!(has_previous_end(&h));
    // モデルを読み直す（世代が変わる）: 前の終点は今のモデルの面ではない
    h.state_mut().state.view3d.load_demo();
    h.run();
    drag_with(&mut h, Device::Mouse, &[b], Modifiers::SHIFT);
    assert!(!ink_near(&h, rect, mid, 2), "間は塗らない");
    assert!(ink_near(&h, rect, b, 2), "押した点は塗る");
}

#[test]
fn ctrl_paints_with_the_mouse_and_never_with_the_pen() {
    let (mut h, rect) = cube_view();
    let at = world(&h, rect, 0.0, 0.0);
    drag_with(
        &mut h,
        Device::Mouse,
        &[at, at + vec2(5.0, 0.0)],
        Modifiers::CTRL,
    );
    assert_eq!(strokes(&h), 1, "マウスの Ctrl は描く");
    assert!(painted_count(&h) > 0);
    let (mut pen, rect) = cube_view();
    let at = world(&pen, rect, 0.0, 0.0);
    drag_with(
        &mut pen,
        Device::Pen,
        &[at, at + vec2(5.0, 0.0)],
        Modifiers::CTRL,
    );
    assert_eq!(strokes(&pen), 0, "ペンの Ctrl は描かない");
    assert_eq!(painted_count(&pen), 0);
}

fn touch(h: &mut H, at: Pos2, phase: egui::TouchPhase, force: Option<f32>) {
    h.event(Event::Touch {
        device_id: egui::TouchDeviceId(0),
        id: egui::TouchId(1),
        phase,
        pos: at,
        force,
    });
}

/// 指（Windows Ink の無い環境のペン）の Touch の力 `force` でなぞった文書。
fn touched(force: Option<f32>, adjust: PressureAdjust) -> H {
    let (mut h, rect) = cube_view();
    {
        let s = &mut h.state_mut().state;
        s.prefs.settings.pressure = adjust;
        s.brush.pressure_size = false;
        s.brush.pressure_opacity = true;
        s.brush.pressure_flow = false;
        s.m2.random_seed = false;
    }
    let (from, to) = (world(&h, rect, -0.2, 0.0), world(&h, rect, 0.2, 0.0));
    if force.is_some() {
        touch(&mut h, from, egui::TouchPhase::Start, force);
    }
    press(&h, from, PointerButton::Primary);
    h.step();
    for i in 1..=8 {
        let at = from + (to - from) * (i as f32 / 8.0);
        if force.is_some() {
            touch(&mut h, at, egui::TouchPhase::Move, force);
        }
        move_to(&h, at);
        h.step();
    }
    release(&h, to, PointerButton::Primary);
    if force.is_some() {
        touch(&mut h, to, egui::TouchPhase::End, force);
    }
    h.run();
    h
}

#[test]
fn a_touch_force_becomes_the_pressure_of_the_3d_mouse_path() {
    let full = touched(None, PressureAdjust::default());
    let light = touched(Some(0.3), PressureAdjust::default());
    assert_eq!(strokes(&light), 1);
    assert!(painted_count(&light) > 0);
    assert!(
        alpha_sum(&light) < alpha_sum(&full) * 7 / 10,
        "力 0.3 は 1.0 より薄い: {} / {}",
        alpha_sum(&light),
        alpha_sum(&full)
    );
    // 力が 1 のタッチは、タッチの無いマウスと同じ
    let one = touched(Some(1.0), PressureAdjust::default());
    assert_eq!(alpha_sum(&one), alpha_sum(&full));
    // 終えたら、次のマウスの押しは 1 に戻る（古い力を持ち越さない）
    let (mut h, rect) = cube_view();
    let (from, to) = (world(&h, rect, -0.2, 0.0), world(&h, rect, 0.2, 0.0));
    touch(&mut h, from, egui::TouchPhase::Start, Some(0.3));
    touch(&mut h, from, egui::TouchPhase::End, None);
    h.step();
    assert_eq!(h.state().state.canvas.touch_pressure, None);
    drag(&mut h, &[from, to]);
    assert!(painted_count(&h) > 0);
}

#[test]
fn a_touch_force_goes_through_the_global_adjustment_in_3d_like_a_pen_point() {
    // 下限 0.25・上限 0.75 の調整を入れたタッチの 0.375 は、調整しないタッチの 0.25 と同じ線（2 の冪の値なので丸めが無い）
    let adjusted = touched(
        Some(0.375),
        PressureAdjust::new(0.25, 0.75, vec![]).unwrap(),
    );
    let plain = touched(Some(0.25), PressureAdjust::default());
    assert!(alpha_sum(&plain) > 0);
    assert_eq!(alpha_sum(&adjusted), alpha_sum(&plain));
    let unadjusted = touched(Some(0.375), PressureAdjust::default());
    assert_ne!(alpha_sum(&unadjusted), alpha_sum(&plain));
}

// ───────── バケツの近い色 ─────────

fn snapshot(h: &H) -> Vec<u8> {
    let doc = &h.state().state.doc;
    doc.composite(doc.bounds()).unwrap()
}

/// 文書の全面に縦の仕切り（画素の列 `x - 3` と `x + 3`）を立て、緑で近い色のバケツ。
fn wall_columns(h: &mut H, x: u32) {
    let s = &mut h.state_mut().state;
    s.tool = Tool::Fill;
    s.color.set_main([0.0, 1.0, 0.0, 1.0]);
    s.region.by_color = true;
    s.region.tolerance = 0;
    let layer = s.selected_layer.expect("レイヤー");
    for column in [x - 3, x + 3] {
        for y in 0..SIZE {
            s.doc
                .set_pixel(layer, column, y, Rgba8::new(0, 0, 0, 255))
                .unwrap();
        }
    }
    s.doc.clear_history().unwrap();
}

#[test]
fn the_similar_color_bucket_fills_the_same_area_in_3d_as_a_2d_click_on_that_pixel() {
    let (mut h3, rect) = cube_view();
    let at = world(&h3, rect, 0.0, 0.0);
    let (x, y) = texel_at(&h3, rect, at).expect("面の上");
    assert!((4..SIZE - 4).contains(&x));
    wall_columns(&mut h3, x);
    let mut h2 = app(1280.0, 800.0, SIZE);
    wall_columns(&mut h2, x);
    let before = snapshot(&h3);
    // 3D: 面を押す
    click(&mut h3, at);
    // 2D: 同じ画素を押す
    let screen = {
        let s = &h2.state().state;
        s.view
            .view(canvas_rect(&h2), SIZE, SIZE)
            .to_screen(x as f64 + 0.5, y as f64 + 0.5)
    };
    click(&mut h2, screen);
    assert_eq!(h3.state().state.doc.undo_count(), 1, "1 回の Undo");
    assert_eq!(snapshot(&h3), snapshot(&h2), "2D と同じ範囲");
    assert_ne!(snapshot(&h3), before);
    let green = [0, 255, 0, 255];
    for column in [x - 2, x, x + 2] {
        assert_eq!(composite_pixel(&h3.state().state.doc, column, 0), green);
    }
    assert_ne!(composite_pixel(&h3.state().state.doc, x + 4, 0), green);
    // 1 回の Undo で戻る
    h3.state_mut().state.doc.undo().unwrap();
    assert_eq!(snapshot(&h3), before);
}

#[test]
fn the_leftover_drag_of_the_similar_color_bucket_works_in_3d() {
    let (mut h, rect) = cube_view();
    let at = world(&h, rect, 0.0, 0.0);
    let (x, y) = texel_at(&h, rect, at).expect("面の上");
    {
        let s = &mut h.state_mut().state;
        s.tool = Tool::Fill;
        s.color.set_main([0.0, 1.0, 0.0, 1.0]);
        s.region.by_color = true;
        s.region.tolerance = 0;
        s.region.color.leftovers = true;
        s.region.color.reference = yolu_app::region::color::Reference::Visible;
        s.brush.radius = 2.0;
        // 押した画素を囲む部屋（四辺が壁）を、線のレイヤーに作る
        let lines = s.doc.add_layer("線").unwrap();
        for i in 0..=14 {
            for (px, py) in [
                (x - 7 + i, y - 7),
                (x - 7 + i, y + 7),
                (x - 7, y - 7 + i),
                (x + 7, y - 7 + i),
            ] {
                s.doc
                    .set_pixel(lines, px, py, Rgba8::new(0, 0, 0, 255))
                    .unwrap();
            }
        }
        s.doc.clear_history().unwrap();
    }
    let layer = h.state().state.selected_layer.expect("レイヤー");
    let alpha = |h: &H, px: u32, py: u32| {
        h.state()
            .state
            .doc
            .layer(layer)
            .unwrap()
            .pixel(yolu_app::engine::Channel::Color, px, py)
            .unwrap()
            .a
    };
    // 押して、少しなぞって離す: 触れた部屋の塗り残しを塗る
    drag(&mut h, &[at, at + vec2(4.0, 2.0), at + vec2(8.0, 3.0)]);
    h.run();
    assert!(h.state().state.region.leftover_drag.is_none());
    assert!(alpha(&h, x, y) > 0, "触れた部屋");
    assert!(
        alpha(&h, x - 5, y - 5) > 0 && alpha(&h, x + 5, y + 5) > 0,
        "部屋の中の塗り残し"
    );
    assert_eq!(alpha(&h, x - 7, y), 0, "壁は塗らない");
    assert_eq!(alpha(&h, 0, 0), 0, "部屋の外は塗らない");
    assert_eq!(h.state().state.doc.undo_count(), 1, "1 回の Undo");
}

// ───────── クローンの設定を 2D と 3D で共有する ─────────

fn select_clone(h: &mut H) {
    use yolu_app::brushes::{BrushAction, BrushKey};
    h.state_mut()
        .state
        .apply(yolu_app::state::Action::Brush(BrushAction::Select(
            BrushKey::Builtin("clone"),
        )));
    h.run();
}

fn open_effect_detail(h: &mut H) {
    let ui = &mut h.state_mut().state.brushes.ui;
    ui.detail.open = true;
    ui.detail.category = yolu_app::brushes::Category::Effect;
    ui.detail.scroll = 0.0;
    h.run();
}

fn close_detail(h: &mut H) {
    h.state_mut().state.brushes.ui.detail.open = false;
    h.run();
}

fn toggle(h: &mut H, label: &str) {
    let rect = h.get_by_role_and_label(Role::CheckBox, label).rect();
    click(h, rect.center());
}

/// 3D の面に元を決めて（Alt + 左を動かさずに離す）、手前の面を 1 本なぞる。
fn alt_click_3d(h: &mut H, at: Pos2) {
    hold(h, Modifiers::ALT);
    press_with(h, at, Modifiers::ALT);
    h.step();
    release_with(h, at, Modifiers::ALT);
    h.run();
    hold(h, Modifiers::NONE);
}

/// 手前の面（UV の左下のアイランド）に、場所で色の違う模様を置く（写した画素で、どこから写したかが分かる）。
fn front_pattern(h: &mut H) {
    let s = &mut h.state_mut().state;
    s.m2.random_seed = false;
    let layer = s.selected_layer.expect("レイヤー");
    for y in 10..118 {
        for x in 10..75 {
            s.doc
                .set_pixel(
                    layer,
                    x,
                    y,
                    Rgba8::new((x * 3) as u8, (y * 2) as u8, 90, 255),
                )
                .unwrap();
        }
    }
    s.doc.clear_history().unwrap();
}

fn composite(h: &H) -> Vec<u8> {
    let doc = &h.state().state.doc;
    doc.composite(doc.bounds()).unwrap()
}

/// 画面の「揃える」を、`tab` のタブを出して押す（効果の詳細を開いて閉じる。3D のタブへ戻す）。
fn toggle_aligned_in(h: &mut H, tab: Tab) {
    click_tab(h, tab);
    h.run();
    open_effect_detail(h);
    toggle(h, "揃える");
    close_detail(h);
    click_tab(h, Tab::View3d);
    h.run();
}

/// 3D の面の点 (x, y)（手前の面）から右へ短くなぞる。
fn surface_stroke(h: &mut H, x: f32, y: f32) {
    let rect = h.state().view3d_rect().expect("3D のタブ");
    let a = world(h, rect, x, y);
    drag(h, &[a, a + vec2(6.0, 0.0), a + vec2(12.0, 0.0)]);
}

/// 3D に元を決めて 2 本写す（間に `between`）。`toggle_in` のタブで「揃える」を切ってから描く。
fn clone_twice_in_3d(toggle_in: Option<Tab>, between: fn(&mut H)) -> (Vec<u8>, bool) {
    let (mut h, rect) = cube_view();
    select_clone(&mut h);
    front_pattern(&mut h);
    let center = world(&h, rect, 0.0, 0.0);
    alt_click_3d(&mut h, center);
    if let Some(tab) = toggle_in {
        toggle_aligned_in(&mut h, tab);
        assert!(!h.state().state.clone.aligned, "{tab:?} で切った");
    }
    surface_stroke(&mut h, -0.2, 0.15);
    between(&mut h);
    surface_stroke(&mut h, 0.15, -0.2);
    let aligned = h.state().state.clone.aligned;
    (composite(&h), aligned)
}

/// 2D に元を決めて 2 本写す。`toggle_in` のタブで「揃える」を切ってから描く。
fn clone_twice_in_2d(toggle_in: Option<Tab>) -> Vec<u8> {
    let (mut h, _) = cube_view();
    select_clone(&mut h);
    front_pattern(&mut h);
    click_tab(&mut h, Tab::Canvas);
    h.run();
    let r = canvas_rect(&h);
    let at = |h: &H, x: f64, y: f64| {
        let s = &h.state().state;
        s.view
            .view(r, s.doc.width(), s.doc.height())
            .to_screen(x, y)
    };
    hold(&mut h, Modifiers::ALT);
    let source = at(&h, 40.0, 60.0);
    press_with(&h, source, Modifiers::ALT);
    h.step();
    release_with(&h, source, Modifiers::ALT);
    h.run();
    hold(&mut h, Modifiers::NONE);
    assert!(h
        .state()
        .state
        .clone
        .canvas_source_for(h.state().state.doc.id())
        .is_some());
    if let Some(tab) = toggle_in {
        click_tab(&mut h, tab);
        h.run();
        open_effect_detail(&mut h);
        toggle(&mut h, "揃える");
        close_detail(&mut h);
        click_tab(&mut h, Tab::Canvas);
        h.run();
        assert!(!h.state().state.clone.aligned);
    }
    // 揃える: 2 本目は 1 本目と同じずれ（(170, 230) − (110, 140) = 模様の (60, 90)）。揃えない: 2 本目も元 (40, 60) から
    for (x, y) in [(150.0, 200.0), (170.0, 230.0)] {
        let (a, b) = (at(&h, x, y), at(&h, x + 6.0, y));
        drag(&mut h, &[a, a + (b - a) * 0.5, b]);
    }
    composite(&h)
}

#[test]
fn toggling_aligned_in_either_view_changes_the_second_stroke_in_the_other_view() {
    // 3D に描く 2 本目: 2D のタブで切っても、3D のタブで切っても同じ結果になり、切らないときとは写す所が違う
    let (on, aligned) = clone_twice_in_3d(None, |_| {});
    assert!(aligned);
    let (off_from_2d, _) = clone_twice_in_3d(Some(Tab::Canvas), |_| {});
    let (off_from_3d, _) = clone_twice_in_3d(Some(Tab::View3d), |_| {});
    assert!(
        off_from_2d == off_from_3d,
        "2D で切っても 3D で切っても同じ"
    );
    assert!(
        on != off_from_2d,
        "揃えるの入り・切りで、2 本目が写す所が変わる"
    );
    // 2D に描く 2 本目: 3D のタブで切っても、2D のタブで切っても同じ
    let on = clone_twice_in_2d(None);
    let off_from_3d = clone_twice_in_2d(Some(Tab::View3d));
    let off_from_2d = clone_twice_in_2d(Some(Tab::Canvas));
    assert!(
        off_from_3d == off_from_2d,
        "3D で切っても 2D で切っても同じ"
    );
    assert!(
        on != off_from_3d,
        "揃えるの入り・切りで、2 本目が写す所が変わる"
    );
}

#[test]
fn the_3d_aligned_destination_survives_reloading_the_brush_like_the_2d_offset() {
    let (plain, _) = clone_twice_in_3d(None, |_| {});
    let eraser_round_trip = |h: &mut H| {
        let d = h.state().state.clone.destination;
        assert!(d.is_some(), "1 本目で先の基準が決まる");
        h.state_mut()
            .state
            .apply(yolu_app::state::Action::SelectTool(Tool::Eraser));
        h.run();
        h.state_mut()
            .state
            .apply(yolu_app::state::Action::SelectTool(Tool::Brush));
        h.run();
        assert_eq!(h.state().state.clone.destination, d);
    };
    let reselect = |h: &mut H| select_clone(h);
    for between in [eraser_round_trip as fn(&mut H), reselect] {
        let (pixels, _) = clone_twice_in_3d(None, between);
        assert!(
            pixels == plain,
            "ブラシを読み直しても、2 本目は同じ所から写す"
        );
    }
}

#[test]
fn the_offset_fields_are_disabled_with_the_reason_while_only_the_3d_view_can_be_painted() {
    for lang in yolu_app::lang::Lang::ALL {
        let (mut h, rect) = cube_view();
        h.state_mut()
            .state
            .apply(yolu_app::state::Action::M2Ui(yolu_app::m2::UiOp::Language(
                lang,
            )));
        select_clone(&mut h);
        assert!(h.state().state.paints_only_in_3d());
        open_effect_detail(&mut h);
        let (x_label, reason) = (
            lang.pick("ずれ X", "Offset X"),
            lang.pick(
                "3D では、元の面の点で決まる",
                "On 3D surfaces the source point decides this",
            ),
        );
        fn slider<'a>(h: &'a H, label: &'a str) -> egui_kittest::Node<'a> {
            h.get_by_role_and_label(Role::Slider, label)
        }
        assert!(
            slider(&h, x_label).accesskit_node().is_disabled(),
            "{lang:?}"
        );
        let target = slider(&h, x_label).rect().center();
        hover_and_wait(&mut h, target);
        assert!(h.query_by_label(reason).is_some(), "{lang:?}: {reason}");
        // 元を決めても、3D だけのあいだは欄で決まらない
        move_to(&h, pos2(2.0, 2.0));
        close_detail(&mut h);
        let center = world(&h, rect, 0.0, 0.0);
        alt_click_3d(&mut h, center);
        open_effect_detail(&mut h);
        assert!(
            slider(&h, x_label).accesskit_node().is_disabled(),
            "{lang:?}"
        );
        // キャンバスのタブへ戻せば（2D に描ける）、欄で決められる
        click_tab(&mut h, Tab::Canvas);
        h.run();
        assert!(!h.state().state.paints_only_in_3d());
        assert!(
            !slider(&h, x_label).accesskit_node().is_disabled(),
            "{lang:?}"
        );
    }
}

#[test]
fn a_stroke_that_ends_off_the_model_keeps_its_last_point_on_the_surface_for_the_shift_line() {
    let (mut h, rect) = cube_view();
    thin_brush(&mut h);
    // 手前の面から左へ、モデルの外（表示域の左の端）まで描いて離す
    let a = world(&h, rect, -0.3, 0.0);
    let off = pos2(rect.left() + 3.0, a.y);
    let path: Vec<Pos2> = (0..=12)
        .map(|i| a + (off - a) * (i as f32 / 12.0))
        .collect();
    drag_with(&mut h, Device::Mouse, &path, Modifiers::NONE);
    assert_eq!(strokes(&h), 1);
    let end = h
        .state()
        .state
        .view3d
        .input
        .previous_end
        .expect("面の上の最後の点を覚える");
    assert!(
        end.position.x < -0.3,
        "モデルの左の縁の近く: {:?}",
        end.position
    );
    // Shift + 押し: その点から押した点までの直線（モデルの外で終えても、前の終点がある）
    let b = world(&h, rect, 0.3, 0.0);
    let mid = world(&h, rect, 0.0, 0.0);
    assert!(!ink_near(&h, rect, mid, 2));
    drag_with(&mut h, Device::Mouse, &[b], Modifiers::SHIFT);
    assert_eq!(strokes(&h), 2);
    assert!(ink_near(&h, rect, mid, 2), "前の終点からの線");
}

#[test]
fn a_texture_set_switch_forgets_the_3d_clone_source_and_a_focus_loss_the_touch_force() {
    let (mut h, rect) = cube_view();
    select_clone(&mut h);
    let center = world(&h, rect, 0.0, 0.0);
    alt_click_3d(&mut h, center);
    assert!(h.state().state.clone.source.is_some());
    // テクスチャセットを替えた: 元は前のテクスチャセットの面なので忘れる（「揃える」などの設定は残る）
    h.state_mut().state.clone.all_layers = true;
    h.state_mut().state.add_texture_set().unwrap();
    h.run();
    assert!(h.state().state.clone.source.is_none());
    assert!(h.state().state.clone.destination.is_none());
    assert!(h.state().state.clone.all_layers);
    // タッチの終わりを受け取れないままフォーカスを失ったら、力を忘れる（次のマウスの押しは 1）
    touch(&mut h, center, egui::TouchPhase::Start, Some(0.3));
    h.step();
    assert!(h.state().state.canvas.touch_pressure.is_some());
    h.event(Event::WindowFocused(false));
    h.step();
    assert_eq!(h.state().state.canvas.touch_pressure, None);
}

#[test]
fn a_previous_end_outside_the_view_still_starts_the_shift_line_on_the_screen() {
    let (mut h, rect) = cube_view();
    thin_brush(&mut h);
    let a = world(&h, rect, -0.4, 0.0);
    drag_with(
        &mut h,
        Device::Mouse,
        &[a, a + vec2(3.0, 0.0)],
        Modifiers::NONE,
    );
    let end = h.state().state.view3d.input.previous_end.expect("前の終点");
    // 寄って右へずらす: 前の終点はカメラの前にあるが、表示域の外
    {
        let camera = &mut h.state_mut().state.view3d.camera;
        camera.target = Vec3::new(0.3, 0.0, -0.5);
        camera.distance = 0.9;
    }
    h.run();
    let outside = screen_of(&h, rect, end.position);
    assert!(
        !rect.contains(outside),
        "表示域の外: {outside:?} / {rect:?}"
    );
    // Shift + 押し: 表示域の外の点から押した点までの、画面の上の直線。見えている所だけ塗る
    let b = world(&h, rect, 0.35, 0.0);
    let between = world(&h, rect, 0.1, 0.0);
    assert!(rect.contains(between) && !ink_near(&h, rect, between, 2));
    h.state_mut().state.message.clear();
    drag_with(&mut h, Device::Mouse, &[b], Modifiers::SHIFT);
    assert_eq!(strokes(&h), 2, "{}", h.state().state.message);
    assert!(ink_near(&h, rect, between, 2), "前の終点の向きから来る線");
    assert!(ink_near(&h, rect, b, 2));
}

#[test]
fn the_end_of_a_touch_is_read_even_without_a_model() {
    // 3D のタブだけを出していて、モデルが無い（イベントを読む前に戻る所）: タッチの終わりで力を忘れる
    let mut h = app(1100.0, 760.0, SIZE);
    click_tab(&mut h, Tab::View3d);
    h.run();
    assert!(h.state().state.view3d.model.is_none());
    assert!(!h.state().state.ui.canvas_visible);
    h.state_mut().state.canvas.touch_pressure = Some(0.3);
    touch(&mut h, pos2(400.0, 300.0), egui::TouchPhase::End, None);
    h.step();
    assert_eq!(h.state().state.canvas.touch_pressure, None);
    // フォーカスを失ったときも
    h.state_mut().state.canvas.touch_pressure = Some(0.3);
    h.event(Event::WindowFocused(false));
    h.step();
    assert_eq!(h.state().state.canvas.touch_pressure, None);
}
