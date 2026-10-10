//! 編集のモードの点の印と G/R/S、ポーズのモードのボーンの G/R/S の画面の試験（egui_kittest。描画は wgpu のソフトの描画）: 印を押して選ぶ・
//! 重なりを巡る・何も無い所で外す、G のあとマウス・X・打った値・Enter・左クリックで決める、Esc・右クリックでやめる、途中の Esc とクリックが
//! 下の 3D ビュー・設定の欄へ届かない、押したままの G の繰り返しで始め直さない、ハンドルのドラッグの途中は断る、フォーカスを失ったら決める、
//! 編集の Delete・Q・H、ポーズの R。絵は印と、G の途中（日英）。
use crate::common;

use common::*;
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::engine::Tilt;
use yolu_app::fillfx::gizmo::{self, Target};
use yolu_app::fillfx::FillOp;
use yolu_app::lang::Lang;
use yolu_app::mode::{EditTool, EditorMode, ModeAction};
use yolu_app::objects::{self, Object, ObjectAction};
use yolu_app::pen::PenSample;
use yolu_app::state::{Action, PopupKind};
use yolu_app::view3d::pose::PoseAction;
use yolu_app::view3d::shape_gizmo::{Handle, Shape};
use yolu_app::YoluApp;
use yolu_core::fill_image::ProjectionMode;
use yolu_core::glam::Vec3;
use yolu_core::{Channel, LayerId};

type H = Harness<'static, YoluApp>;

/// 試しの立方体を手前（−Z の側）から正面に見た 3D ビュー（画面の右が +X）で、投影の箱・グラデーションデカール・点のグラデーションを持つ
/// 塗りつぶしレイヤーを作り、編集のモードにする。
fn edit_view() -> (H, Rect, LayerId) {
    let mut h = app(1280.0, 860.0, 256);
    h.state_mut().apply(Action::LoadDemoModel);
    h.state_mut().state.view3d.camera.yaw = 0.0;
    h.state_mut().state.view3d.camera.pitch = 0.0;
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let app = &mut h.state_mut().state;
    app.apply(Action::M2(yolu_app::m2::Edit::NewFill));
    let layer = app.selected_layer.expect("塗りつぶしレイヤー");
    app.apply(Action::Fill(FillOp::ProjectionMode {
        layer,
        mode: ProjectionMode::Planar,
    }));
    app.apply(Action::Fill(FillOp::AddGradient {
        layer,
        channel: Channel::Color,
    }));
    app.apply(Action::Fill(FillOp::AddPoints {
        layer,
        channel: Channel::Roughness,
    }));
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Edit)));
    h.run();
    assert_eq!(h.state().state.mode, EditorMode::Edit);
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect, layer)
}

fn screen_of(h: &H, rect: Rect, p: Vec3) -> Pos2 {
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(rect.width(), rect.height());
    let s = view.to_screen(p).expect("カメラの前");
    pos2(rect.left() + s.x, rect.top() + s.y)
}

fn marker_at(h: &H, rect: Rect, o: Object) -> Pos2 {
    screen_of(
        h,
        rect,
        objects::marker_of(&h.state().state, o).expect("印"),
    )
}

fn the_box(layer: LayerId) -> Object {
    Object::Shape(Target::Projection(layer))
}

fn shape(h: &H, o: Object) -> Shape {
    let Object::Shape(t) = o else {
        panic!("形");
    };
    objects::shape_of(&h.state().state, t).expect("形")
}

fn transforming(h: &H) -> bool {
    let app = &h.state().state;
    let open = objects::transforming(app);
    assert_eq!(
        open,
        matches!(
            app.popup.as_ref().map(|p| p.kind),
            Some(PopupKind::Transform)
        ),
        "途中とポップアップは一緒"
    );
    open
}

fn undo_count(h: &H) -> usize {
    h.state().state.doc.undo_count()
}

fn select(h: &mut H, o: Option<Object>) {
    h.state_mut().apply(Action::Object(ObjectAction::Select(o)));
    h.run();
}

/// ポインタを `at` に置いて G（R・S）を押して離す。
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

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn pressing_markers_selects_objects_and_their_layer_without_painting() {
    let (mut h, rect, layer) = edit_view();
    let base = h.state().state.doc.layers()[0].id();
    h.state_mut().state.select_single_layer(base);
    let undo = undo_count(&h);
    // 箱の印を押す: 箱とそのレイヤーを選ぶ
    let at = marker_at(&h, rect, the_box(layer));
    click(&mut h, at);
    let first = h.state().state.objects.selected.expect("選んだ");
    assert_eq!(h.state().state.selected_layer, Some(layer));
    // 同じ所でもう一度押すと、重なった次の物（グラデーションデカールも外形の中心にある）
    click(&mut h, at);
    let second = h.state().state.objects.selected.expect("選んだ");
    assert_ne!(first, second);
    // 乗せた印を覚える（大きく描く）
    let point = Object::Point {
        layer,
        channel: Channel::Roughness,
        index: 0,
    };
    let p = marker_at(&h, rect, point);
    move_to(&h, p);
    h.run();
    assert_eq!(h.state().state.objects.hover, Some(point));
    click(&mut h, p);
    assert_eq!(h.state().state.objects.selected, Some(point));
    // 何も無い所で外す
    click(&mut h, rect.min + vec2(12.0, 12.0));
    assert_eq!(h.state().state.objects.selected, None);
    // 描かない・文書を変えない
    assert!(!h.state().state.is_stroking());
    assert_eq!(undo_count(&h), undo);
}

#[test]
fn g_follows_the_mouse_then_an_axis_and_a_typed_value_and_ends_by_enter_click_escape_or_right_click(
) {
    let (mut h, rect, layer) = edit_view();
    let o = the_box(layer);
    select(&mut h, Some(o));
    let start_shape = shape(&h, o);
    let undo = undo_count(&h);
    let at = marker_at(&h, rect, o);
    // G: マウスで右へ動かすと +X へ（途中は 1 つのまとめ）。キーで始めても、帯は移動のツールを光らせる
    start(&mut h, at, Key::G);
    assert!(transforming(&h));
    assert_eq!(EditTool::shown(&h.state().state), EditTool::Move);
    assert_eq!(h.state().state.edit_tool, EditTool::Select);
    move_to(&h, at + vec2(60.0, 0.0));
    h.run();
    assert!(shape(&h, o).center[0] - start_shape.center[0] > 0.05);
    // X と 0.3: 打った値はマウスより先
    type_keys(&mut h, &[Key::X, Key::Num0, Key::Period, Key::Num3]);
    assert!(near(shape(&h, o).center[0], start_shape.center[0] + 0.3));
    assert!(near(shape(&h, o).center[1], start_shape.center[1]));
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert!(!transforming(&h));
    assert!(near(shape(&h, o).center[0], start_shape.center[0] + 0.3));
    assert_eq!(undo_count(&h), undo + 1, "1 回の取り消し");
    let moved = shape(&h, o);
    // 左クリックで決める: その押しは下の 3D ビューへ届かない（印を選び直さない・ハンドルのドラッグを始めない）
    let at = marker_at(&h, rect, o);
    start(&mut h, at, Key::G);
    move_to(&h, at + vec2(0.0, -40.0));
    h.run();
    click(&mut h, at + vec2(0.0, -40.0));
    assert!(!transforming(&h));
    assert!(shape(&h, o).center[1] > moved.center[1] + 0.05);
    assert_eq!(undo_count(&h), undo + 2);
    let app = &h.state().state;
    assert_eq!(app.objects.selected, Some(o));
    assert!(app.fillfx.drag.is_none() && app.view3d.input.nav.is_none());
    assert!(!app.is_stroking());
    let moved = shape(&h, o);
    // Esc でやめる: 戻して、履歴に残さない
    let at = marker_at(&h, rect, o);
    start(&mut h, at, Key::G);
    assert!(transforming(&h));
    move_to(&h, at + vec2(50.0, 30.0));
    h.run();
    assert_ne!(shape(&h, o), moved);
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(!transforming(&h));
    assert_eq!(shape(&h, o), moved);
    assert_eq!(undo_count(&h), undo + 2);
    // 右クリックでやめる: メニューを開かない・回し始めない（回転は中心のまわりにポインタを回す）
    start(&mut h, at + vec2(60.0, 0.0), Key::R);
    move_to(&h, at + vec2(50.0, -50.0));
    h.run();
    assert_ne!(shape(&h, o), moved);
    press(&h, at + vec2(50.0, -50.0), PointerButton::Secondary);
    h.step();
    move_to(&h, at + vec2(70.0, -30.0));
    h.step();
    release(&h, at + vec2(70.0, -30.0), PointerButton::Secondary);
    h.run();
    assert!(!transforming(&h));
    assert_eq!(shape(&h, o), moved);
    assert_eq!(undo_count(&h), undo + 2);
    let app = &h.state().state;
    assert!(app.popup.is_none(), "右クリックのメニューを開かない");
    assert!(app.view3d.input.nav.is_none());
}

#[test]
fn escape_that_cancels_a_transform_does_not_close_the_3d_settings_or_deselect_pixels() {
    let (mut h, rect, layer) = edit_view();
    let o = the_box(layer);
    // 選択範囲（Ctrl+A）と、開いた 3D ビューの設定の欄
    key(&h, Key::A, Modifiers::COMMAND);
    h.run();
    assert!(h.state().state.doc.selection().is_some());
    h.state_mut().state.view3d.display.settings_open = true;
    select(&mut h, Some(o));
    let before = shape(&h, o);
    let at = marker_at(&h, rect, o);
    start(&mut h, at, Key::S);
    assert!(transforming(&h));
    move_to(&h, at + vec2(40.0, 40.0));
    h.run();
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(!transforming(&h));
    assert_eq!(shape(&h, o), before);
    let app = &h.state().state;
    assert!(
        app.view3d.display.settings_open,
        "Esc は G/R/S をやめるだけ"
    );
    assert!(app.doc.selection().is_some(), "選択範囲はそのまま");
    // 何も開いていなければ、Esc は今までどおり設定の欄を閉じる
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(!h.state().state.view3d.display.settings_open);
}

#[test]
fn a_held_g_does_not_restart_after_confirming_and_g_is_refused_during_a_handle_drag() {
    let (mut h, rect, layer) = edit_view();
    let o = the_box(layer);
    select(&mut h, Some(o));
    let at = marker_at(&h, rect, o);
    move_to(&h, at);
    h.run();
    // G を押したまま、Enter で決める
    h.event(Event::Key {
        key: Key::G,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    assert!(transforming(&h));
    key(&h, Key::Enter, Modifiers::NONE);
    h.step();
    assert!(!transforming(&h));
    // 押したままのキーの繰り返し（OS の repeat の押し）で始め直さない。途中に来た繰り返しも数えない
    for _ in 0..3 {
        h.event(Event::Key {
            key: Key::G,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: Modifiers::NONE,
        });
        h.step();
        assert!(!transforming(&h));
    }
    h.event(Event::Key {
        key: Key::G,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    // ハンドルのドラッグの途中: G は断る（ドラッグはそのまま続く）
    let handle = gizmo::handle_point(&h.state().state, rect, Handle::MoveX).expect("X のハンドル");
    press(&h, handle, PointerButton::Primary);
    h.step();
    move_to(&h, handle + vec2(20.0, 0.0));
    h.step();
    assert!(h.state().state.fillfx.drag.is_some());
    key(&h, Key::G, Modifiers::NONE);
    h.step();
    assert!(!transforming(&h));
    assert!(h.state().state.fillfx.drag.is_some());
    assert!(
        h.state().state.message.contains("ほかの操作の途中です"),
        "{}",
        h.state().state.message
    );
    release(&h, handle + vec2(20.0, 0.0), PointerButton::Primary);
    h.run();
    assert!(h.state().state.fillfx.drag.is_none());
    // フォーカスを失ったら、そこまでで決める（取り残さない）
    let undo = undo_count(&h);
    let before = shape(&h, o);
    let at = marker_at(&h, rect, o);
    start(&mut h, at, Key::G);
    move_to(&h, at + vec2(30.0, 0.0));
    h.run();
    h.event(Event::WindowFocused(false));
    h.run();
    assert!(!transforming(&h));
    assert_ne!(shape(&h, o), before);
    assert_eq!(undo_count(&h), undo + 1);
    assert!(!h.state().state.doc.is_coalescing());
}

#[test]
fn delete_q_and_h_act_on_objects_in_edit_mode_and_delete_does_not_erase_pixels() {
    let (mut h, _, layer) = edit_view();
    let decal = Object::Shape(Target::Gradient(layer, Channel::Color));
    // 描けるレイヤーの選択範囲: 編集のモードの Delete は画素を消さない
    let base = h.state().state.doc.layers()[0].id();
    h.state_mut().state.select_single_layer(base);
    h.state_mut()
        .state
        .doc
        .set_pixel(base, 10, 10, yolu_app::engine::Rgba8::new(200, 30, 30, 255))
        .unwrap();
    let painted = yolu_app::engine::composite_pixel(&h.state().state.doc, 10, 10);
    key(&h, Key::A, Modifiers::COMMAND);
    h.run();
    assert!(h.state().state.doc.selection().is_some());
    let undo = undo_count(&h);
    key(&h, Key::Delete, Modifiers::NONE);
    h.run();
    assert_eq!(
        yolu_app::engine::composite_pixel(&h.state().state.doc, 10, 10),
        painted
    );
    assert_eq!(undo_count(&h), undo);
    assert!(
        h.state().state.message.contains("選んだ物がありません"),
        "{}",
        h.state().state.message
    );
    // 選んだグラデーションデカールを消す
    select(&mut h, Some(decal));
    key(&h, Key::Delete, Modifiers::NONE);
    h.run();
    assert!(objects::marker_of(&h.state().state, decal).is_none());
    assert_eq!(undo_count(&h), undo + 1);
    // Q で取っ手を隠す・出す
    let o = the_box(layer);
    select(&mut h, Some(o));
    assert_eq!(
        gizmo::target(&h.state().state),
        Some(Target::Projection(layer))
    );
    key(&h, Key::Q, Modifiers::NONE);
    h.run();
    assert_eq!(gizmo::target(&h.state().state), None);
    key(&h, Key::Q, Modifiers::NONE);
    h.run();
    assert!(gizmo::target(&h.state().state).is_some());
    // H で選んだ物の印を隠し、Alt+H で出す
    key(&h, Key::H, Modifiers::NONE);
    h.run();
    assert!(!objects::markers(&h.state().state)
        .iter()
        .any(|m| m.object == o));
    key(&h, Key::H, Modifiers::ALT);
    h.run();
    assert!(objects::markers(&h.state().state)
        .iter()
        .any(|m| m.object == o));
}

#[test]
fn r_turns_the_selected_bone_in_pose_mode_and_escape_leaves_it() {
    let mut h = app(1280.0, 860.0, 256);
    h.state_mut().apply(Action::Pose(PoseAction::LoadFigure));
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブが前に出た");
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Pose)));
    let b = {
        let s = h.state().state.view3d.pose.session.as_ref().unwrap();
        s.rig
            .bones()
            .iter()
            .position(|b| b.name == "右上腕")
            .unwrap()
    };
    h.state_mut()
        .state
        .view3d
        .pose
        .session
        .as_mut()
        .unwrap()
        .selected = Some(b);
    h.run();
    let local = |h: &H| {
        h.state()
            .state
            .view3d
            .pose
            .session
            .as_ref()
            .unwrap()
            .pose()
            .locals[b]
    };
    let undo = |h: &H| {
        h.state()
            .state
            .view3d
            .pose
            .session
            .as_ref()
            .unwrap()
            .undo_len()
    };
    let rest = local(&h);
    start(&mut h, rect.center(), Key::R);
    assert!(transforming(&h));
    type_keys(&mut h, &[Key::Z, Key::Num9, Key::Num0]);
    assert!(local(&h).rotation.angle_between(rest.rotation) > 1.0);
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert!(!transforming(&h));
    assert_eq!(undo(&h), 1);
    let turned = local(&h);
    // G のあと Esc: 戻して積まない
    start(&mut h, rect.center(), Key::G);
    move_to(&h, rect.center() + vec2(40.0, 0.0));
    h.run();
    assert_ne!(local(&h).translation, turned.translation);
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert_eq!(local(&h), turned);
    assert_eq!(undo(&h), 1);
    // ペイントのモードの G は G/R/S ではない
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Paint)));
    h.run();
    start(&mut h, rect.center(), Key::G);
    assert!(!transforming(&h));
}

#[test]
fn escape_that_cancels_a_transform_does_not_close_the_color_window() {
    use egui_kittest::kittest::Queryable;
    let mut h = app(1280.0, 1400.0, 256);
    h.state_mut().apply(Action::LoadDemoModel);
    h.state_mut().state.view3d.camera.yaw = 0.0;
    h.state_mut().state.view3d.camera.pitch = 0.0;
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブ");
    let app = &mut h.state_mut().state;
    app.apply(Action::M2(yolu_app::m2::Edit::NewFill));
    let layer = app.selected_layer.unwrap();
    app.apply(Action::Fill(FillOp::ProjectionMode {
        layer,
        mode: ProjectionMode::Planar,
    }));
    h.run();
    // 塗りつぶしの色の欄から、色のウィンドウを開く
    let swatch = h.get_by_label("カラー の値").rect();
    click(&mut h, swatch.center());
    assert!(yolu_app::panels::color_window::is_open(&h.ctx));
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Edit)));
    h.run();
    let o = the_box(layer);
    select(&mut h, Some(o));
    let at = marker_at(&h, rect, o);
    start(&mut h, at, Key::G);
    assert!(transforming(&h));
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(!transforming(&h));
    assert!(
        yolu_app::panels::color_window::is_open(&h.ctx),
        "Esc は G/R/S をやめるだけ"
    );
    // 何も開いていなければ、Esc は今までどおり色のウィンドウを閉じる
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(!yolu_app::panels::color_window::is_open(&h.ctx));
}

fn pen_sample(at: Pos2, contact: bool, time_ms: u32) -> PenSample {
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

#[test]
fn a_held_delete_deletes_one_point_and_leaves_nothing_selected() {
    let (mut h, _, layer) = edit_view();
    // 3 つ目の点を加える
    let app = &mut h.state_mut().state;
    let mut g = app
        .doc
        .layer(layer)
        .unwrap()
        .fill_points(Channel::Roughness)
        .unwrap()
        .clone();
    let mut third = g.points[0];
    third.position[1] += 0.3;
    g.points.push(third);
    app.apply(Action::Fill(FillOp::Points {
        layer,
        channel: Channel::Roughness,
        points: Some(Box::new(g)),
        coalesce: false,
    }));
    h.run();
    select(
        &mut h,
        Some(Object::Point {
            layer,
            channel: Channel::Roughness,
            index: 0,
        }),
    );
    let undo = undo_count(&h);
    let count = |h: &H| {
        h.state()
            .state
            .doc
            .layer(layer)
            .unwrap()
            .fill_points(Channel::Roughness)
            .unwrap()
            .points
            .len()
    };
    // Delete を押したまま（OS の繰り返しの押しが続く）
    let event = |pressed: bool, repeat: bool| Event::Key {
        key: Key::Delete,
        physical_key: None,
        pressed,
        repeat,
        modifiers: Modifiers::NONE,
    };
    h.event(event(true, false));
    h.step();
    for _ in 0..4 {
        h.event(event(true, true));
        h.step();
    }
    h.event(event(false, false));
    h.run();
    assert_eq!(count(&h), 2, "1 つだけ消す");
    assert_eq!(undo_count(&h), undo + 1);
    assert_eq!(h.state().state.objects.selected, None);
}

#[test]
fn dropping_a_shelf_image_on_the_model_in_edit_mode_places_a_decal_and_selects_it() {
    let (mut h, rect, _) = edit_view();
    let rgba: Vec<u8> = [
        [255u8, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255; 4],
    ]
    .concat();
    let id = h
        .state_mut()
        .state
        .shelf
        .add_image(Lang::Ja, "四色", &rgba, 2, 2)
        .expect("棚へ入る");
    let layers = h.state().state.doc.layers().len();
    let at = screen_of(&h, rect, Vec3::new(0.2, 0.1, -0.5));
    egui::DragAndDrop::set_payload(
        &h.ctx,
        yolu_app::shelf::ShelfDrag {
            id,
            kind: yolu_app::shelf::ItemKind::Image,
            name: "四色".into(),
        },
    );
    move_to(&h, at);
    h.step();
    release(&h, at, PointerButton::Primary);
    h.run();
    let app = &h.state().state;
    assert_eq!(app.doc.layers().len(), layers + 1, "{}", app.message);
    let placed = app.selected_layer.expect("置いたレイヤー");
    assert_eq!(
        app.doc.layer(placed).unwrap().projection().mode,
        ProjectionMode::Decal
    );
    assert_eq!(app.mode, EditorMode::Edit);
    assert_eq!(
        app.objects.selected,
        Some(Object::Shape(Target::Projection(placed))),
        "置いたデカールを選ぶ"
    );
}

#[test]
fn the_move_tool_drags_the_selected_object_with_the_mouse_or_the_pen_in_one_undo_step() {
    let (mut h, rect, layer) = edit_view();
    let o = the_box(layer);
    h.state_mut()
        .apply(Action::Mode(ModeAction::Tool(EditTool::Move)));
    select(&mut h, Some(o));
    let start_shape = shape(&h, o);
    let undo = undo_count(&h);
    // マウス: 印から押して動かし、途中で X、離して決める
    let at = marker_at(&h, rect, o);
    press(&h, at, PointerButton::Primary);
    h.step();
    assert!(transforming(&h));
    move_to(&h, at + vec2(40.0, -30.0));
    h.step();
    key(&h, Key::X, Modifiers::NONE);
    h.step();
    move_to(&h, at + vec2(60.0, -30.0));
    h.step();
    assert!(transforming(&h), "押している間は続く");
    release(&h, at + vec2(60.0, -30.0), PointerButton::Primary);
    h.run();
    assert!(!transforming(&h));
    let moved = shape(&h, o);
    assert!(moved.center[0] - start_shape.center[0] > 0.05, "{moved:?}");
    assert!(near(moved.center[1], start_shape.center[1]), "X だけ");
    assert_eq!(undo_count(&h), undo + 1);
    let app = &h.state().state;
    assert!(app.fillfx.drag.is_none() && app.view3d.input.nav.is_none() && !app.is_stroking());
    // Esc でやめる（離しても決めない）
    let at = marker_at(&h, rect, o);
    press(&h, at, PointerButton::Primary);
    h.step();
    move_to(&h, at + vec2(30.0, 30.0));
    h.step();
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    release(&h, at + vec2(30.0, 30.0), PointerButton::Primary);
    h.run();
    assert_eq!(shape(&h, o), moved);
    assert_eq!(undo_count(&h), undo + 1);
    // ペン: 触れて動かして離す
    let at = marker_at(&h, rect, o);
    for (i, p) in [at, at + vec2(0.0, -20.0), at + vec2(0.0, -45.0)]
        .iter()
        .enumerate()
    {
        h.state().pen().push(pen_sample(*p, true, i as u32 * 10));
        h.step();
    }
    assert!(transforming(&h));
    h.state()
        .pen()
        .push(pen_sample(at + vec2(0.0, -45.0), false, 100));
    h.step();
    h.run();
    assert!(!transforming(&h));
    let penned = shape(&h, o);
    assert!(penned.center[1] - moved.center[1] > 0.05, "{penned:?}");
    assert_eq!(undo_count(&h), undo + 2);
}

#[test]
fn the_rotate_tool_turns_the_bone_under_the_pointer_in_pose_mode() {
    let mut h = app(1280.0, 860.0, 256);
    h.state_mut().apply(Action::Pose(PoseAction::LoadFigure));
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブが前に出た");
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Pose)));
    h.state_mut()
        .apply(Action::Mode(ModeAction::Tool(EditTool::Rotate)));
    h.run();
    let local = |h: &H, b: usize| {
        h.state()
            .state
            .view3d
            .pose
            .session
            .as_ref()
            .unwrap()
            .pose()
            .locals[b]
    };
    let undo = |h: &H| {
        h.state()
            .state
            .view3d
            .pose
            .session
            .as_ref()
            .unwrap()
            .undo_len()
    };
    // ボーンの面の上で、そのボーンの輪に掛からない点を探し、選んで押す（輪の上なら輪の回転になるので、Esc でやめて次の点へ）
    let mut found = None;
    'search: for dy in (-200..=200).step_by(40) {
        for dx in (-120..=120).step_by(40) {
            let at = rect.center() + vec2(dx as f32, dy as f32);
            let Some(b) = yolu_app::view3d::gizmo::bone_at(&h.state().state, rect, at) else {
                continue;
            };
            h.state_mut()
                .state
                .view3d
                .pose
                .session
                .as_mut()
                .unwrap()
                .selected = Some(b);
            h.run();
            move_to(&h, at);
            h.step();
            press(&h, at, PointerButton::Primary);
            h.step();
            if transforming(&h) {
                found = Some((at, b));
                break 'search;
            }
            assert!(
                h.state().state.view3d.pose.drag.is_some(),
                "輪の上でなければ G/R/S"
            );
            key(&h, Key::Escape, Modifiers::NONE);
            h.step();
            release(&h, at, PointerButton::Primary);
            h.run();
        }
    }
    let (at, b) = found.expect("ボーンの面の上の点");
    assert_eq!(undo(&h), 0);
    let rest = local(&h, b);
    let local = |h: &H| local(h, b);
    for p in [
        at + vec2(40.0, 0.0),
        at + vec2(60.0, -40.0),
        at + vec2(20.0, -70.0),
    ] {
        move_to(&h, p);
        h.step();
    }
    release(&h, at + vec2(20.0, -70.0), PointerButton::Primary);
    h.run();
    assert!(!transforming(&h));
    assert!(local(&h).rotation.angle_between(rest.rotation) > 0.05);
    assert_eq!(undo(&h), 1);
}

// ───────── 絵 ─────────

fn crop(h: &mut H, rect: Rect, name: &str) {
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        rect.left().max(0.0).floor() as u32,
        rect.top().max(0.0).floor() as u32,
        rect.width().ceil() as u32,
        rect.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

#[test]
fn snapshot_the_markers_and_a_grab_in_both_languages() {
    let (mut h, rect, layer) = edit_view();
    // 点を選び、箱の印に乗せる（白・乗せて大きい・選んで橙）
    let point = Object::Point {
        layer,
        channel: Channel::Roughness,
        index: 0,
    };
    select(&mut h, Some(point));
    move_to(&h, marker_at(&h, rect, the_box(layer)));
    h.run();
    crop(&mut h, rect, "edit_markers");
    for (lang, name) in [(Lang::Ja, "edit_grab_ja"), (Lang::En, "edit_grab_en")] {
        let (mut h, rect, layer) = edit_view();
        h.state_mut().state.set_language(lang);
        let o = the_box(layer);
        select(&mut h, Some(o));
        let at = marker_at(&h, rect, o);
        start(&mut h, at, Key::G);
        type_keys(
            &mut h,
            &[Key::X, Key::Num0, Key::Period, Key::Num2, Key::Num5],
        );
        assert!(transforming(&h));
        crop(&mut h, rect, name);
    }
}
