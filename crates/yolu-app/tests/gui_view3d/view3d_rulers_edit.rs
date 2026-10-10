//! 編集のモードの 3D の定規（egui_kittest。試しの立方体を正面に見て、3D ビューの入力の道で）: 印で選ぶ（レイヤーとプロパティの行も選ぶ）、G/R/S（途中は続けて変える
//! 操作で、決めて 1 回の取り消し、Esc・右クリックで戻す。軸の固定・2 回目の定規の向きの軸・打った値）、Alt+G/R/S の既定、Delete・H・目を閉じる、
//! ギズモ（移動の矢印・回す輪・端の点の四角。大きさのつまみは無い）のドラッグと端の点が面に乗って動くこと。絵は選んだ定規とギズモ（日英）。
use crate::common::*;
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::fillfx::gizmo::{self, Target};
use yolu_app::lang::Lang;
use yolu_app::mode::{EditorMode, ModeAction};
use yolu_app::objects::{self, Object, ObjectAction};
use yolu_app::state::{Action, AppState};
use yolu_app::view3d::shape_gizmo::Handle;
use yolu_app::YoluApp;
use yolu_core::glam::{DVec3, Vec3};
use yolu_core::{LayerId, Ruler, RulerId, RulerKind, RulerPlace};

type H = Harness<'static, YoluApp>;

fn st(h: &H) -> &AppState {
    &h.state().state
}

/// 試しの立方体を手前（−Z の側）から正面に見た 3D ビューに、手前の面の上の斜めの直線定規を 1 本置き、編集のモードにする。
fn edit_view() -> (H, Rect, LayerId) {
    let mut h = app(1280.0, 860.0, 256);
    h.state_mut().apply(Action::LoadDemoModel);
    h.state_mut().state.view3d.camera.yaw = 0.0;
    h.state_mut().state.view3d.camera.pitch = 0.0;
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let layer = st(&h).selected_layer.expect("レイヤー");
    crate::common::rulers::add(
        &mut h.state_mut().state,
        Ruler::model(
            RulerId(1),
            RulerKind::Line,
            DVec3::new(-0.3, -0.2, -0.5),
            DVec3::new(0.3, 0.2, -0.5),
            DVec3::NEG_Z,
        ),
    );
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Edit)));
    h.run();
    assert_eq!(st(&h).mode, EditorMode::Edit);
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect, layer)
}

fn screen_of(h: &H, rect: Rect, p: Vec3) -> Pos2 {
    let view = st(h).view3d.camera.view(rect.width(), rect.height());
    let s = view.to_screen(p).expect("カメラの前");
    pos2(rect.left() + s.x, rect.top() + s.y)
}

fn the_ruler(h: &H) -> Ruler {
    let layer = st(h).selected_layer.expect("レイヤー");
    crate::common::rulers::of(st(h), layer).remove(0)
}

fn object(h: &H) -> Object {
    let r = the_ruler(h);
    Object::Ruler {
        layer: st(h).selected_layer.unwrap(),
        id: r.id,
    }
}

fn points(r: &Ruler) -> (DVec3, DVec3, DVec3) {
    match r.place {
        RulerPlace::Model { a, b, up } => (a, b, up),
        RulerPlace::Canvas { .. } => panic!("2D の定規"),
    }
}

fn marker_at(h: &H, rect: Rect) -> Pos2 {
    screen_of(h, rect, objects::marker_of(st(h), object(h)).expect("印"))
}

fn select(h: &mut H) {
    let o = object(h);
    h.state_mut()
        .apply(Action::Object(ObjectAction::Select(Some(o))));
    h.run();
}

fn undo_count(h: &H) -> usize {
    st(h).doc.undo_count()
}

fn transforming(h: &H) -> bool {
    objects::transforming(st(h))
}

fn start(h: &mut H, at: Pos2, k: Key) {
    move_to(h, at);
    h.run();
    key(h, k, Modifiers::NONE);
    h.run();
}

fn type_keys(h: &mut H, keys: &[Key]) {
    for k in keys {
        key(h, *k, Modifiers::NONE);
    }
    h.run();
}

fn near(a: DVec3, b: DVec3) -> bool {
    a.distance(b) < 1e-4
}

fn center(r: &Ruler) -> DVec3 {
    let (a, b, _) = points(r);
    (a + b) * 0.5
}

#[test]
fn the_marker_of_a_3d_ruler_selects_it_with_its_layer_and_its_row_and_2d_or_hidden_rulers_have_none(
) {
    let (mut h, rect, layer) = edit_view();
    // 2D の定規と、表示を切った 3D の定規は印を持たない
    crate::common::rulers::line(&mut h.state_mut().state, (0.0, 3.0), (60.0, 3.0));
    let mut hidden = Ruler::model(
        RulerId(1),
        RulerKind::Concentric,
        DVec3::new(0.2, 0.2, -0.5),
        DVec3::new(0.4, 0.2, -0.5),
        DVec3::NEG_Z,
    );
    hidden.visible = false;
    crate::common::rulers::add(&mut h.state_mut().state, hidden);
    let marks: Vec<Object> = objects::markers(st(&h))
        .into_iter()
        .map(|m| m.object)
        .filter(|o| matches!(o, Object::Ruler { .. }))
        .collect();
    assert_eq!(marks.len(), 1, "{marks:?}");
    // 2 点の真ん中に印
    let first = crate::common::rulers::of(st(&h), layer).remove(0);
    let at = marker_at(&h, rect);
    assert!(at.distance(screen_of(&h, rect, center(&first).as_vec3())) < 1e-3);
    // 別のレイヤーを選んでから、印を押す: 定規とそのレイヤーとプロパティの行を選ぶ
    let other = h.state_mut().state.doc.add_layer("別").unwrap();
    h.state_mut().state.select_single_layer(other);
    h.run();
    let steps = undo_count(&h);
    click(&mut h, at);
    assert_eq!(st(&h).objects.selected, Some(marks[0]));
    assert_eq!(st(&h).selected_layer, Some(layer));
    assert_eq!(st(&h).rulers.selected, Some((layer, first.id)));
    assert!(!st(&h).is_stroking());
    assert_eq!(undo_count(&h), steps, "選ぶだけ。文書を変えない");
    // 何も無い所を押すと外れる
    click(&mut h, rect.min + vec2(12.0, 12.0));
    assert_eq!(st(&h).objects.selected, None);
}

#[test]
fn g_moves_a_3d_ruler_by_the_mouse_an_axis_and_a_typed_value_and_escape_puts_it_back() {
    let (mut h, rect, _) = edit_view();
    select(&mut h);
    let before = the_ruler(&h);
    let steps = undo_count(&h);
    let at = marker_at(&h, rect);
    // マウスで右へ（途中は文書へ続けて入る）
    start(&mut h, at, Key::G);
    assert!(transforming(&h));
    move_to(&h, at + vec2(60.0, 0.0));
    h.run();
    let moved = the_ruler(&h);
    let (a0, b0, up0) = points(&before);
    let (a1, b1, up1) = points(&moved);
    assert!(
        a1.x > a0.x + 0.05 && (a1 - a0).distance(b1 - b0) < 1e-6,
        "a・b が一緒に動く"
    );
    assert!(near(up1, up0), "向きは変わらない");
    // X と 0.3: 打った値はマウスより先
    type_keys(&mut h, &[Key::X, Key::Num0, Key::Period, Key::Num3]);
    let typed = the_ruler(&h);
    assert!(near(points(&typed).0, a0 + DVec3::new(0.3, 0.0, 0.0)));
    assert!(near(points(&typed).1, b0 + DVec3::new(0.3, 0.0, 0.0)));
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert!(!transforming(&h));
    assert_eq!(undo_count(&h), steps + 1, "1 回の取り消し");
    assert!(!st(&h).doc.is_coalescing());
    // Esc: 戻して履歴に残さない
    let decided = the_ruler(&h);
    let at = marker_at(&h, rect);
    start(&mut h, at, Key::G);
    move_to(&h, at + vec2(0.0, -50.0));
    h.run();
    assert_ne!(the_ruler(&h), decided);
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(!transforming(&h));
    assert_eq!(the_ruler(&h), decided);
    assert_eq!(undo_count(&h), steps + 1);
    // 右クリックでもやめる
    start(&mut h, at, Key::G);
    move_to(&h, at + vec2(40.0, 10.0));
    h.run();
    press(&h, at + vec2(40.0, 10.0), PointerButton::Secondary);
    h.step();
    release(&h, at + vec2(40.0, 10.0), PointerButton::Secondary);
    h.run();
    assert!(!transforming(&h));
    assert_eq!(the_ruler(&h), decided);
    assert_eq!(undo_count(&h), steps + 1);
    // 取り消しで始めの値へ
    h.state_mut().state.apply(Action::Undo);
    assert_eq!(the_ruler(&h), before);
}

#[test]
fn r_turns_around_the_center_with_the_direction_and_s_scales_the_distance_keeping_the_direction() {
    let (mut h, rect, _) = edit_view();
    select(&mut h);
    let before = the_ruler(&h);
    let (a0, b0, up0) = points(&before);
    let c0 = center(&before);
    // X の軸のまわりに 90 度: 中心は動かず、長さは同じ、向きは軸のまわりに回る
    let at = marker_at(&h, rect);
    start(&mut h, at, Key::R);
    type_keys(&mut h, &[Key::X, Key::Num9, Key::Num0]);
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    let turned = the_ruler(&h);
    let (a1, b1, up1) = points(&turned);
    assert!(near(center(&turned), c0), "中心は動かない");
    assert!(
        (a1.distance(b1) - a0.distance(b0)).abs() < 1e-5,
        "長さは同じ"
    );
    assert!(
        up1.dot(up0).abs() < 1e-5,
        "−Z の向きが X の軸のまわりに 90 度回って Y の向きに: {up1}"
    );
    assert!(up1.x.abs() < 1e-5, "回す軸の成分は動かない");
    h.state_mut().state.apply(Action::Undo);
    assert_eq!(the_ruler(&h), before);
    // S: 倍率 2（軸なし）で中心から a・b の距離が 2 倍、向きは変えない
    let at = marker_at(&h, rect);
    start(&mut h, at, Key::S);
    type_keys(&mut h, &[Key::Num2]);
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    let scaled = the_ruler(&h);
    let (a2, b2, up2) = points(&scaled);
    assert!(near(center(&scaled), c0));
    assert!((a2.distance(b2) - 2.0 * a0.distance(b0)).abs() < 1e-5);
    assert!(near(up2, up0), "向きは変えない");
    assert!(((b2 - a2).normalize() - (b0 - a0).normalize()).length() < 1e-5);
}

#[test]
fn the_second_axis_press_is_the_axis_of_the_ruler_where_x_follows_a_to_b() {
    let (mut h, rect, _) = edit_view();
    select(&mut h);
    let before = the_ruler(&h);
    let (a0, b0, _) = points(&before);
    let along = (b0 - a0).normalize();
    let at = marker_at(&h, rect);
    start(&mut h, at, Key::G);
    // X の 1 回目は世界の軸、2 回目は定規の向きの軸（a→b の向き）
    type_keys(&mut h, &[Key::X, Key::X, Key::Num0, Key::Period, Key::Num5]);
    let t = st(&h).objects.transform.as_ref().expect("途中");
    let axis = t.axis_direction(0, true);
    assert!((axis.as_dvec3() - along).length() < 1e-5, "{axis} {along}");
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    let moved = the_ruler(&h);
    assert!(near(points(&moved).0, a0 + along * 0.5), "a→b の向きへ 0.5");
    assert!(near(points(&moved).1, b0 + along * 0.5));
}

#[test]
fn delete_h_and_the_eye_act_on_the_selected_ruler_and_undo_brings_it_back() {
    let (mut h, rect, layer) = edit_view();
    select(&mut h);
    let o = object(&h);
    let id = the_ruler(&h).id;
    // H で印を隠し、Alt+H で出す
    key(&h, Key::H, Modifiers::NONE);
    h.run();
    assert!(!objects::markers(st(&h)).iter().any(|m| m.object == o));
    key(&h, Key::H, Modifiers::ALT);
    h.run();
    assert!(objects::markers(st(&h)).iter().any(|m| m.object == o));
    // 目（表示）を切ると印が無くなり、選びも外れる
    select(&mut h);
    let mut r = the_ruler(&h);
    r.visible = false;
    h.state_mut()
        .state
        .apply(Action::Ruler(yolu_app::rulers::RulerAction::Replace {
            owner: layer,
            ruler: r,
            coalesce: false,
        }));
    h.run();
    assert!(!objects::markers(st(&h)).iter().any(|m| m.object == o));
    assert_eq!(objects::selected(st(&h)), None);
    h.state_mut().state.apply(Action::Undo);
    h.run();
    // Delete で消す（1 回の取り消し）
    select(&mut h);
    let steps = undo_count(&h);
    key(&h, Key::Delete, Modifiers::NONE);
    h.run();
    assert!(crate::common::rulers::of(st(&h), layer).is_empty());
    assert_eq!(undo_count(&h), steps + 1);
    assert_eq!(st(&h).objects.selected, None);
    assert_eq!(st(&h).rulers.selected, None);
    h.state_mut().state.apply(Action::Undo);
    assert_eq!(crate::common::rulers::of(st(&h), layer).len(), 1);
    assert_eq!(the_ruler(&h).id, id);
    // 選んでいなければ、Delete は断る
    h.state_mut()
        .apply(Action::Object(ObjectAction::Select(None)));
    h.run();
    key(&h, Key::Delete, Modifiers::NONE);
    h.run();
    assert_eq!(crate::common::rulers::of(st(&h), layer).len(), 1);
    assert!(
        st(&h).message.contains("選んだ物がありません"),
        "{}",
        st(&h).message
    );
    let _ = rect;
}

#[test]
fn alt_g_r_and_s_reset_the_center_the_axes_and_the_length_in_one_undo_step_each() {
    let (mut h, rect, _) = edit_view();
    select(&mut h);
    let before = the_ruler(&h);
    let (a0, b0, _) = points(&before);
    let length = a0.distance(b0);
    let steps = undo_count(&h);
    // Alt+G: 中心がモデルの境界の中心（立方体の中心＝原点）へ。向きと長さは同じ
    let at = marker_at(&h, rect);
    move_to(&h, at);
    h.run();
    key(&h, Key::G, Modifiers::ALT);
    h.run();
    let g = the_ruler(&h);
    assert!(center(&g).length() < 1e-5, "{}", center(&g));
    assert!((points(&g).0.distance(points(&g).1) - length).abs() < 1e-5);
    assert_eq!(undo_count(&h), steps + 1);
    // Alt+R: 向きがモデルの軸へ。up は −Z のまま、a→b はその次に近い軸（X）
    key(&h, Key::R, Modifiers::ALT);
    h.run();
    let r = the_ruler(&h);
    let (ra, rb, rup) = points(&r);
    assert!(near(rup, DVec3::NEG_Z), "{rup}");
    assert!(
        near((rb - ra).normalize(), DVec3::X),
        "{}",
        (rb - ra).normalize()
    );
    assert!(center(&r).length() < 1e-5, "中心は動かない");
    assert!((ra.distance(rb) - length).abs() < 1e-5);
    assert_eq!(undo_count(&h), steps + 2);
    // Alt+S: a–b の距離がモデルの大きさ（対角線）の 1/4 へ
    key(&h, Key::S, Modifiers::ALT);
    h.run();
    let s = the_ruler(&h);
    let diagonal = 3.0f64.sqrt();
    assert!((points(&s).0.distance(points(&s).1) - diagonal / 4.0).abs() < 1e-4);
    assert!(center(&s).length() < 1e-5);
    assert_eq!(undo_count(&h), steps + 3);
}

#[test]
fn the_gizmo_of_a_3d_ruler_has_arrows_rings_and_end_squares_but_no_size_knobs() {
    let (mut h, rect, layer) = edit_view();
    select(&mut h);
    let id = the_ruler(&h).id;
    assert_eq!(gizmo::target(st(&h)), Some(Target::Ruler(layer, id)));
    let app = st(&h);
    for handle in [
        Handle::MoveFree,
        Handle::MoveX,
        Handle::MoveY,
        Handle::RotateX,
        Handle::RotateY,
        Handle::RotateZ,
        Handle::EndA,
        Handle::EndB,
    ] {
        assert!(
            gizmo::handle_point(app, rect, handle).is_some(),
            "{handle:?} が出ている"
        );
    }
    for knob in [Handle::SizeXPos, Handle::SizeYNeg, Handle::SizeZPos] {
        assert!(
            gizmo::handle_point(app, rect, knob).is_none(),
            "{knob:?}: 大きさのつまみは出さない"
        );
    }
    // 対称定規は端の点が b だけ（a は中心で、印のほうで動かす）
    let mut s = Ruler::model(
        RulerId(1),
        RulerKind::Symmetry,
        DVec3::new(0.0, 0.2, -0.5),
        DVec3::new(0.0, 0.2, -0.2),
        DVec3::Y,
    );
    s.lines = 2;
    crate::common::rulers::add(&mut h.state_mut().state, s);
    let sym = Object::Ruler {
        layer,
        id: st(&h).rulers.selected.unwrap().1,
    };
    h.state_mut()
        .apply(Action::Object(ObjectAction::Select(Some(sym))));
    h.run();
    assert!(gizmo::handle_point(st(&h), rect, Handle::EndB).is_some());
    assert!(gizmo::handle_point(st(&h), rect, Handle::EndA).is_none());
}

#[test]
fn dragging_an_arrow_moves_the_ruler_in_one_step_and_escape_puts_it_back() {
    let (mut h, rect, _) = edit_view();
    select(&mut h);
    let before = the_ruler(&h);
    let steps = undo_count(&h);
    let handle = gizmo::handle_point(st(&h), rect, Handle::MoveX).expect("X の矢印");
    press(&h, handle, PointerButton::Primary);
    h.step();
    move_to(&h, handle + vec2(40.0, 0.0));
    h.step();
    move_to(&h, handle + vec2(80.0, 0.0));
    h.step();
    let moved = the_ruler(&h);
    let (a0, b0, _) = points(&before);
    let (a1, b1, _) = points(&moved);
    assert!(a1.x > a0.x + 0.05, "{a1}");
    assert!((a1 - a0).distance(b1 - b0) < 1e-5, "a・b が一緒に");
    assert!(
        (a1.y - a0.y).abs() < 1e-5 && (a1.z - a0.z).abs() < 1e-5,
        "X の軸に沿う"
    );
    release(&h, handle + vec2(80.0, 0.0), PointerButton::Primary);
    h.run();
    assert_eq!(undo_count(&h), steps + 1, "1 回の取り消し");
    // Esc でドラッグの前へ
    let decided = the_ruler(&h);
    let handle = gizmo::handle_point(st(&h), rect, Handle::MoveX).expect("X の矢印");
    press(&h, handle, PointerButton::Primary);
    h.step();
    move_to(&h, handle + vec2(0.0, 0.0) + vec2(30.0, 0.0));
    h.step();
    assert_ne!(the_ruler(&h), decided);
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    release(&h, handle + vec2(30.0, 0.0), PointerButton::Primary);
    h.run();
    assert_eq!(the_ruler(&h), decided);
    assert_eq!(undo_count(&h), steps + 1);
}

#[test]
fn dragging_an_end_square_puts_the_end_on_the_surface_or_on_the_camera_plane_in_one_step() {
    let (mut h, rect, _) = edit_view();
    select(&mut h);
    let before = the_ruler(&h);
    let (a0, _, up0) = points(&before);
    let steps = undo_count(&h);
    let end = gizmo::handle_point(st(&h), rect, Handle::EndB).expect("b の四角");
    // 面の上の別の点へ: b がその面の点になる
    let target = screen_of(&h, rect, Vec3::new(0.4, -0.3, -0.5));
    press(&h, end, PointerButton::Primary);
    h.step();
    move_to(&h, (end + target.to_vec2()) / 2.0);
    h.step();
    move_to(&h, target);
    h.step();
    let moved = the_ruler(&h);
    let (a1, b1, up1) = points(&moved);
    assert!(near(a1, a0), "a は動かない");
    assert!(near(up1, up0));
    assert!(
        b1.distance(DVec3::new(0.4, -0.3, -0.5)) < 5e-3,
        "面の点: {b1}"
    );
    release(&h, target, PointerButton::Primary);
    h.run();
    assert_eq!(undo_count(&h), steps + 1, "1 回の取り消し");
    // モデルの外へ出すと、前の点を通りカメラに向いた面の上（奥行きは変わらない）
    let end = gizmo::handle_point(st(&h), rect, Handle::EndB).expect("b の四角");
    let outside = rect.min + vec2(60.0, 60.0);
    press(&h, end, PointerButton::Primary);
    h.step();
    move_to(&h, outside);
    h.step();
    let (_, b2, _) = points(&the_ruler(&h));
    assert!((b2.z - b1.z).abs() < 1e-3, "奥行きは同じ: {b2} {b1}");
    assert!(b2.distance(b1) > 0.05, "動いた");
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    release(&h, outside, PointerButton::Primary);
    h.run();
    assert_eq!(points(&the_ruler(&h)).1, b1, "Esc で戻る");
    assert_eq!(undo_count(&h), steps + 1);
    assert!(
        st(&h).message.contains("定規の操作をやめました"),
        "形でなく定規の操作とそのまま言う: {}",
        st(&h).message
    );
}

#[test]
fn an_end_square_grabbed_off_center_moves_by_the_pointer_movement_without_jumping() {
    let (mut h, rect, _) = edit_view();
    select(&mut h);
    let before = the_ruler(&h);
    let (_, b0, _) = points(&before);
    let center = gizmo::handle_point(st(&h), rect, Handle::EndB).expect("b の四角");
    // 四角の中心から数点ずらして押す（四角の中: 4 点・3 点）
    let pressed = center + vec2(4.0, -3.0);
    press(&h, pressed, PointerButton::Primary);
    h.step();
    assert!(st(&h).fillfx.drag.is_some(), "四角を掴んだ");
    // 動かす前の最初の動き: 端は 10 点だけ動く（ポインタの真下へ跳ばない）
    let to = pressed + vec2(10.0, 0.0);
    move_to(&h, to);
    h.step();
    let (_, b1, _) = points(&the_ruler(&h));
    let moved = screen_of(&h, rect, b1.as_vec3());
    let expected = center + vec2(10.0, 0.0);
    assert!(
        moved.distance(expected) < 1.5,
        "端は押す前の位置 + 動かした量: {moved} {expected}"
    );
    assert!(
        moved.distance(to) > 3.0,
        "ポインタの真下へは跳んでいない: {moved} {to}"
    );
    assert!(b1.distance(b0) > 1e-4, "動いた");
    release(&h, to, PointerButton::Primary);
    h.run();
}

#[test]
fn s_on_one_axis_stretches_a_and_b_along_it_so_the_direction_changes_and_up_stays() {
    let mut h = app(1280.0, 860.0, 256);
    h.state_mut().apply(Action::LoadDemoModel);
    h.state_mut().state.view3d.camera.yaw = 0.0;
    h.state_mut().state.view3d.camera.pitch = 0.0;
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    // a→b は up（−Z）と直角でない（面の上の点から斜めの面の点へ引いた直線）
    crate::common::rulers::add(
        &mut h.state_mut().state,
        Ruler::model(
            RulerId(1),
            RulerKind::Line,
            DVec3::new(-0.3, -0.2, -0.5),
            DVec3::new(0.3, 0.2, -0.2),
            DVec3::NEG_Z,
        ),
    );
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Edit)));
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    select(&mut h);
    let before = the_ruler(&h);
    let (a0, b0, up0) = points(&before);
    let c0 = center(&before);
    let at = marker_at(&h, rect);
    start(&mut h, at, Key::S);
    // X の軸（定規の向きの軸）で 2 倍
    type_keys(&mut h, &[Key::X, Key::Num2]);
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    let after = the_ruler(&h);
    let (a1, b1, up1) = points(&after);
    assert!(near(up1, up0), "向きは変わらない");
    assert!(near(center(&after), c0), "中心は動かない");
    let (d0, d1) = ((b0 - a0), (b1 - a1));
    // 向きに直交する成分（up の成分）は変わらず、軸の成分が 2 倍
    assert!((d1.dot(up0) - d0.dot(up0)).abs() < 1e-5);
    let flat = |d: DVec3| (d - up0 * d.dot(up0)).length();
    assert!((flat(d1) - 2.0 * flat(d0)).abs() < 1e-5);
    assert!(
        d0.normalize().dot(d1.normalize()) < 1.0 - 1e-3,
        "a→b の向きは変わる"
    );
}

#[test]
fn alt_s_does_nothing_to_a_one_point_perspective_and_resizes_a_two_point_one() {
    let (mut h, rect, layer) = edit_view();
    let mut p = Ruler::model(
        RulerId(1),
        RulerKind::Perspective,
        DVec3::new(0.0, 0.3, -0.5),
        DVec3::new(0.2, 0.3, -0.5),
        DVec3::NEG_Z,
    );
    crate::common::rulers::add(&mut h.state_mut().state, p.clone());
    let id = st(&h).rulers.selected.unwrap().1;
    let o = Object::Ruler { layer, id };
    h.state_mut()
        .apply(Action::Object(ObjectAction::Select(Some(o))));
    h.run();
    let at = screen_of(&h, rect, objects::marker_of(st(&h), o).unwrap());
    move_to(&h, at);
    h.run();
    let steps = undo_count(&h);
    let ruler = |h: &H| {
        crate::common::rulers::of(st(h), layer)
            .into_iter()
            .find(|r| r.id == id)
            .unwrap()
    };
    let before = ruler(&h);
    key(&h, Key::S, Modifiers::ALT);
    h.run();
    assert_eq!(
        ruler(&h),
        before,
        "1 点のパースは b を使わないので何もしない"
    );
    assert_eq!(undo_count(&h), steps, "空の取り消しの段を作らない");
    // 2 点にすると b も使うので、距離を変える
    p = ruler(&h);
    p.two_points = true;
    h.state_mut()
        .state
        .apply(Action::Ruler(yolu_app::rulers::RulerAction::Replace {
            owner: layer,
            ruler: p,
            coalesce: false,
        }));
    h.run();
    let steps = undo_count(&h);
    key(&h, Key::S, Modifiers::ALT);
    h.run();
    assert_eq!(undo_count(&h), steps + 1);
    let (a, b, _) = points(&ruler(&h));
    assert!((a.distance(b) - 3.0f64.sqrt() / 4.0).abs() < 1e-4);
}

#[test]
fn in_orthographic_a_3d_ruler_moves_by_g_and_by_its_arrow() {
    let (mut h, rect, _) = edit_view();
    h.state_mut().state.view3d.camera.set_orthographic(true);
    h.run();
    assert!(st(&h).view3d.camera.is_orthographic());
    select(&mut h);
    let before = the_ruler(&h);
    let (a0, b0, _) = points(&before);
    let steps = undo_count(&h);
    // G: 打った値で X へ 0.3
    let at = marker_at(&h, rect);
    start(&mut h, at, Key::G);
    type_keys(&mut h, &[Key::X, Key::Num0, Key::Period, Key::Num3]);
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    let (a1, b1, _) = points(&the_ruler(&h));
    assert!(near(a1, a0 + DVec3::new(0.3, 0.0, 0.0)));
    assert!(near(b1, b0 + DVec3::new(0.3, 0.0, 0.0)));
    assert_eq!(undo_count(&h), steps + 1);
    // 矢印のドラッグ
    let handle = gizmo::handle_point(st(&h), rect, Handle::MoveX).expect("X の矢印");
    press(&h, handle, PointerButton::Primary);
    h.step();
    move_to(&h, handle + vec2(30.0, 0.0));
    h.step();
    move_to(&h, handle + vec2(60.0, 0.0));
    h.step();
    let (a2, b2, _) = points(&the_ruler(&h));
    assert!(a2.x > a1.x + 0.02, "{a2}");
    assert!((a2 - a1).distance(b2 - b1) < 1e-5);
    release(&h, handle + vec2(60.0, 0.0), PointerButton::Primary);
    h.run();
    assert_eq!(undo_count(&h), steps + 2);
}

#[test]
fn replacing_the_project_drops_the_edit_mode_selection_and_hidden_marks() {
    let dir = crate::common::tmp::test_dir("view3d-rulers-edit");
    let path = dir.join("p.ylp");
    let (mut h, _, layer) = edit_view();
    select(&mut h);
    let o = object(&h);
    h.state_mut().state.objects.hidden.push(Object::Ruler {
        layer,
        id: RulerId(0x1234),
    });
    assert_eq!(objects::selected(st(&h)), Some(o));
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    h.state_mut().state.apply(Action::OpenProject(path));
    h.run();
    assert_eq!(
        st(&h).objects.selected,
        None,
        "前のプロジェクトの選びは外す"
    );
    assert!(st(&h).objects.hidden.is_empty(), "隠した印も外す");
    assert_eq!(st(&h).rulers.selected, None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_read_only_set_refuses_to_move_or_delete_a_ruler() {
    let (mut h, rect, layer) = edit_view();
    select(&mut h);
    h.state_mut().state.sets.get_mut(0).unwrap().read_only = Some("テスト".into());
    let before = the_ruler(&h);
    let at = marker_at(&h, rect);
    start(&mut h, at, Key::G);
    assert!(!transforming(&h));
    key(&h, Key::Delete, Modifiers::NONE);
    h.run();
    assert_eq!(the_ruler(&h), before);
    assert_eq!(crate::common::rulers::of(st(&h), layer).len(), 1);
}

#[test]
fn snapshots_of_a_selected_ruler_with_its_gizmo_in_both_languages() {
    let mut snapshots = egui_kittest::SnapshotResults::new();
    for lang in Lang::ALL {
        let (mut h, _, _) = edit_view();
        h.state_mut().state.lang = lang;
        select(&mut h);
        h.state_mut().state.message.clear();
        h.event(Event::PointerGone);
        h.run();
        h.snapshot(format!("view3d_ruler_selected_{}", lang.pick("ja", "en")));
        snapshots.extend_harness(&mut h);
    }
}
