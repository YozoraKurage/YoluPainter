//! モード（ペイント・編集・ポーズ）とパイメニューの画面の試験（egui_kittest。描画は wgpu のソフトの描画）: オプションバーのドロップダウン・
//! Ctrl+Tab のモードのパイ・表示のメニュー・ツールの帯・骨の木での切り替え、編集・ポーズで描かない（2D・3D）、パイの開き方（すぐ離す・
//! 押したまま離す）・数字・Esc・右クリック、視点のパイの各向き、描いている間は替えない、ギズモのドラッグは確定してから替える。絵は
//! ドロップダウンとパイ（日英）。
use crate::common;

use common::*;
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::lang::Lang;
use yolu_app::mode::{EditTool, EditorMode, ModeAction};
use yolu_app::pie::PieAction;
use yolu_app::state::{Action, PopupKind, Tool};
use yolu_app::view3d::gizmo;
use yolu_app::view3d::pose::PoseAction;
use yolu_app::YoluApp;
use yolu_core::geometry::AxisView;

type H = Harness<'static, YoluApp>;

/// 試しの人形を読んで、3D ビューのタブが前に出たウィンドウ。
fn figure_view() -> (H, Rect) {
    let mut h = app(1280.0, 860.0, 256);
    h.state_mut().apply(Action::Pose(PoseAction::LoadFigure));
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブが前に出た");
    (h, rect)
}

fn mode(h: &H) -> EditorMode {
    h.state().state.mode
}

fn pie_open(h: &H) -> Option<String> {
    let app = &h.state().state;
    match app.popup.as_ref().map(|p| p.kind) {
        Some(PopupKind::Pie) => app.pie.open.as_ref().map(|o| o.menu.clone()),
        _ => None,
    }
}

fn pie_center(h: &H) -> Pos2 {
    h.state()
        .state
        .pie
        .open
        .as_ref()
        .expect("パイが開いている")
        .run
        .center
}

/// 開いているパイの項目（真ん中の近く。同じ名前のメニューの見出し・ドックのタブと取り違えない）。
fn pie_item(h: &H, label: &str) -> Rect {
    let c = pie_center(h);
    rect_of(h, label, |r| (r.center() - c).length() < 220.0)
}

fn key_event(h: &H, key: Key, pressed: bool, modifiers: Modifiers) {
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    });
}

/// ポインタを `at` に置いて、Ctrl+Tab を押して離す（同じフレーム。すぐ離したので、パイは開いたままクリックを待つ）。
fn tap_mode_pie(h: &mut H, at: Pos2) {
    move_to(h, at);
    h.run();
    key(h, Key::Tab, Modifiers::CTRL);
    h.run();
}

/// 3D の面の上の点（試しの人形の胴。カメラに向いた三角形の真ん中）。
fn body_point(h: &H, rect: Rect) -> Pos2 {
    let app = &h.state().state;
    let model = app.view3d.model.clone().unwrap();
    let s = app.view3d.pose.session.as_ref().unwrap();
    let n = s.rig.meshes()[0].mesh.triangle_count();
    let cam = app.view3d.camera.position();
    let t = model.geometry.triangles()[..n]
        .iter()
        .find(|t| {
            let c = (t.a + t.b + t.c) / 3.0;
            t.normal().dot((cam - c).normalize()) > 0.8 && c.y > 1.1
        })
        .copied()
        .unwrap();
    let view = app.view3d.camera.view(rect.width(), rect.height());
    let s = view.to_screen((t.a + t.b + t.c) / 3.0).unwrap();
    pos2(rect.left() + s.x, rect.top() + s.y)
}

#[test]
fn the_dropdown_switches_modes_and_the_edit_and_pose_screens_swap_the_tool_strip() {
    let (mut h, _) = figure_view();
    assert_eq!(mode(&h), EditorMode::Paint);
    assert!(
        h.query_by_label("ブラシ（B）").is_some(),
        "ペイントのツールの帯"
    );
    // オプションバーの左端: 今のモードの名前
    let dropdown = bar_rect(&h, "モード: ペイント");
    assert!(dropdown.left() < 20.0, "左端: {dropdown:?}");
    click(&mut h, dropdown.center());
    assert_eq!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::Mode)
    );
    let at = popup_item(&h, "編集").center();
    click(&mut h, at);
    assert_eq!(mode(&h), EditorMode::Edit);
    assert!(h.state().state.popup.is_none());
    let dropdown = bar_rect(&h, "モード: 編集");
    // 編集のツールの帯: 選択・移動・回転・拡縮（ブラシのツールは出ない）
    for name in ["選択", "移動", "回転", "拡縮"] {
        assert!(
            h.get_all_by_label(name).any(|n| n.rect().left() < 44.0),
            "{name}"
        );
    }
    assert!(
        h.query_by_label("ブラシ（B）").is_none(),
        "描くツールは出ない"
    );
    let at = rect_of(&h, "移動", |r| r.left() < 44.0).center();
    click(&mut h, at);
    assert_eq!(h.state().state.edit_tool, EditTool::Move);
    // ポーズへ
    click(&mut h, dropdown.center());
    let at = popup_item(&h, "ポーズ").center();
    click(&mut h, at);
    assert_eq!(mode(&h), EditorMode::Pose);
    assert!(bar_rect(&h, "モード: ポーズ").width() > 0.0);
    // 開いて外を押すと閉じる（モードは替わらない）
    let dropdown = bar_rect(&h, "モード: ポーズ");
    click(&mut h, dropdown.center());
    click(&mut h, pos2(700.0, 500.0));
    assert!(h.state().state.popup.is_none());
    assert_eq!(mode(&h), EditorMode::Pose);
}

#[test]
fn without_bones_the_pose_item_is_disabled_with_a_reason() {
    let mut h = app(1280.0, 860.0, 256);
    let dropdown = bar_rect(&h, "モード: ペイント");
    click(&mut h, dropdown.center());
    let at = popup_item(&h, "ポーズ").center();
    hover_and_wait(&mut h, at);
    assert!(
        h.query_by_label("ボーンのあるモデルがありません").is_some(),
        "押せない理由のツールチップ"
    );
    click(&mut h, at);
    assert_eq!(mode(&h), EditorMode::Paint);
}

#[test]
fn the_view_menu_and_a_drawing_tool_switch_modes_and_a_bone_in_the_tree_enters_pose() {
    let (mut h, _) = figure_view();
    // 表示 → モード → 編集
    let at = menu_title(&h, "表示").center();
    click(&mut h, at);
    let at = popup_item(&h, "モード").center();
    click(&mut h, at);
    let sub = h.state().state.popup.as_ref().unwrap().state.sub_rects()[0];
    let at = rect_of(&h, "編集", |r| sub.contains_rect(r)).center();
    click(&mut h, at);
    assert_eq!(mode(&h), EditorMode::Edit);
    // 編集のメニューのツール（描くツール）を選ぶとペイントへ
    let at = menu_title(&h, "編集").center();
    click(&mut h, at);
    let at = popup_item(&h, "消しゴム").center();
    click(&mut h, at);
    assert_eq!(mode(&h), EditorMode::Paint);
    assert_eq!(h.state().state.tool, Tool::Eraser);
    // 骨の木でボーンを選ぶとポーズへ
    click_tab(&mut h, yolu_app::Tab::Pose);
    h.run();
    let at = h.get_by_label("腰").rect().center();
    click(&mut h, at);
    assert_eq!(mode(&h), EditorMode::Pose);
}

#[test]
fn edit_and_pose_modes_do_not_draw_in_3d_or_2d_but_the_view_and_the_eyedropper_still_work() {
    let (mut h, rect) = figure_view();
    let at = body_point(&h, rect);
    for m in [EditorMode::Edit, EditorMode::Pose] {
        h.state_mut().apply(Action::Mode(ModeAction::Set(m)));
        h.run();
        drag(&mut h, &[at, at + vec2(8.0, 0.0), at + vec2(16.0, 0.0)]);
        let app = &h.state().state;
        assert!(!app.doc.can_undo(), "{m:?}: 3D に描かない");
        assert!(!app.is_stroking());
    }
    // 2D: 見るだけ（描かない・選択範囲も作らない）
    click_tab(&mut h, yolu_app::Tab::Canvas);
    h.run();
    let c = canvas_rect(&h).center();
    for (m, tool) in [
        (EditorMode::Edit, Tool::Brush),
        (EditorMode::Pose, Tool::SelectRect),
        (EditorMode::Edit, Tool::Fill),
    ] {
        h.state_mut().apply(Action::SelectTool(tool));
        h.state_mut().apply(Action::Mode(ModeAction::Set(m)));
        h.run();
        drag(&mut h, &[offset(c, -30.0, -20.0), offset(c, 30.0, 20.0)]);
        let app = &h.state().state;
        assert_eq!(canvas_pixel(&h, c)[3], 0, "{m:?} {tool:?}: 描かない");
        assert!(!app.doc.can_undo(), "{m:?} {tool:?}");
        assert!(
            app.doc.selection().is_none(),
            "{m:?} {tool:?}: 選択範囲を作らない"
        );
    }
    // 視点の操作（Alt + 左ドラッグで回す）とスポイト（右ボタン）は効く
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.step();
    h.event(Event::PointerMoved(offset(c, 100.0, 0.0)));
    h.event(Event::PointerButton {
        pos: offset(c, 100.0, 0.0),
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::ALT,
    });
    h.step();
    for k in 1..=6 {
        h.event(Event::PointerMoved(offset(c, 100.0, 15.0 * k as f32)));
        h.step();
    }
    h.event(Event::PointerButton {
        pos: offset(c, 100.0, 90.0),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::ALT,
    });
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    assert!(h.state().state.view.angle != 0.0, "表示が回った");
    press(&h, c, PointerButton::Secondary);
    h.step();
    assert!(
        h.state().state.canvas.eyedrop.is_some(),
        "右ボタンのスポイト"
    );
    release(&h, c, PointerButton::Secondary);
    h.run();
    assert!(!h.state().state.doc.can_undo());
}

#[test]
fn paint_keys_do_nothing_outside_paint_mode_and_everywhere_keys_still_work() {
    let (mut h, _) = figure_view();
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Edit)));
    h.run();
    for k in [Key::E, Key::G, Key::B] {
        key(&h, k, Modifiers::NONE);
        h.run();
        assert_eq!(h.state().state.tool, Tool::Brush, "{k:?}");
        assert_eq!(mode(&h), EditorMode::Edit, "{k:?}");
    }
    // どこでもの段: Ctrl+Tab のパイは編集でも開く
    let at = h.state().view3d_rect().unwrap().center();
    tap_mode_pie(&mut h, at);
    assert_eq!(pie_open(&h).as_deref(), Some("mode"));
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(pie_open(&h).is_none());
    // ペイントに戻すと、ツールのキーが効く
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Paint)));
    key(&h, Key::E, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.tool, Tool::Eraser);
}

#[test]
fn a_quick_ctrl_tab_opens_the_mode_pie_at_the_pointer_and_a_click_chooses() {
    let (mut h, rect) = figure_view();
    let at = rect.center() + vec2(-40.0, 30.0);
    tap_mode_pie(&mut h, at);
    assert_eq!(pie_open(&h).as_deref(), Some("mode"));
    assert_eq!(pie_center(&h), at, "ポインタの所に開く");
    // 上: 編集・右: ポーズ・左: ペイント
    let c = pie_center(&h);
    let edit = pie_item(&h, "編集");
    let pose = pie_item(&h, "ポーズ");
    let paint = pie_item(&h, "ペイント");
    assert!(edit.center().y < c.y && (edit.center().x - c.x).abs() < 1.0);
    assert!(pose.left() > c.x && (pose.center().y - c.y).abs() < 1.0);
    assert!(paint.right() < c.x && (paint.center().y - c.y).abs() < 1.0);
    // 開いている間は下の入力を止める（キーの表にも、3D ビューにも届かない）
    key(&h, Key::E, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.tool, Tool::Brush);
    assert_eq!(pie_open(&h).as_deref(), Some("mode"));
    click(&mut h, edit.center());
    assert_eq!(mode(&h), EditorMode::Edit);
    assert!(pie_open(&h).is_none());
    assert!(h.state().state.popup.is_none());
    assert!(
        !h.state().state.doc.can_undo(),
        "パイを押した押しで描かない"
    );
}

#[test]
fn holding_ctrl_tab_and_releasing_toward_an_item_runs_it_and_a_long_hold_in_the_middle_closes() {
    let (mut h, rect) = figure_view();
    let at = rect.center();
    move_to(&h, at);
    h.run();
    key_event(&h, Key::Tab, true, Modifiers::CTRL);
    h.step();
    assert_eq!(pie_open(&h).as_deref(), Some("mode"));
    assert_eq!(
        h.state().state.pie.open.as_ref().unwrap().run.key,
        Some(Key::Tab),
        "押しているキーを覚える"
    );
    // 右（ポーズ）の向きへ動かして離す: 箱に届かなくても、向きで指す
    move_to(&h, at + vec2(50.0, 6.0));
    h.step();
    key_event(&h, Key::Tab, false, Modifiers::CTRL);
    h.step();
    h.run();
    assert_eq!(mode(&h), EditorMode::Pose);
    assert!(pie_open(&h).is_none());
    // 長く押して真ん中で離すと、何もせずに閉じる
    key_event(&h, Key::Tab, true, Modifiers::CTRL);
    h.step();
    assert_eq!(pie_open(&h).as_deref(), Some("mode"));
    for _ in 0..30 {
        h.step();
    }
    key_event(&h, Key::Tab, false, Modifiers::CTRL);
    h.step();
    h.run();
    assert!(pie_open(&h).is_none());
    assert_eq!(mode(&h), EditorMode::Pose);
    // すぐ離したら開いたまま（クリックを待つ）
    key_event(&h, Key::Tab, true, Modifiers::CTRL);
    h.step();
    key_event(&h, Key::Tab, false, Modifiers::CTRL);
    h.step();
    h.run();
    assert_eq!(pie_open(&h).as_deref(), Some("mode"));
}

#[test]
fn digits_choose_and_escape_a_right_click_or_a_click_in_the_middle_close_the_pie() {
    let (mut h, rect) = figure_view();
    let at = rect.center();
    // 数字は上から時計回り: 1 = 上（編集）、3 = 右（ポーズ）、7 = 左（ペイント）
    for (digit, wanted) in [
        (Key::Num1, EditorMode::Edit),
        (Key::Num3, EditorMode::Pose),
        (Key::Num7, EditorMode::Paint),
    ] {
        tap_mode_pie(&mut h, at);
        key(&h, digit, Modifiers::NONE);
        h.run();
        assert_eq!(mode(&h), wanted, "{digit:?}");
        assert!(pie_open(&h).is_none());
    }
    // 空いている向きの数字は何もしない（開いたまま）
    tap_mode_pie(&mut h, at);
    key(&h, Key::Num5, Modifiers::NONE);
    h.run();
    assert_eq!(pie_open(&h).as_deref(), Some("mode"));
    // Esc
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(pie_open(&h).is_none());
    // 右クリック（スポイトにもならない）
    tap_mode_pie(&mut h, at);
    press(&h, at + vec2(0.0, 60.0), PointerButton::Secondary);
    h.step();
    release(&h, at + vec2(0.0, 60.0), PointerButton::Secondary);
    h.run();
    assert!(pie_open(&h).is_none());
    assert!(h.state().state.view3d.input.eyedrop.is_none());
    // 真ん中のクリック
    tap_mode_pie(&mut h, at);
    click(&mut h, at);
    assert!(pie_open(&h).is_none());
    assert_eq!(mode(&h), EditorMode::Paint);
    assert!(!h.state().state.doc.can_undo(), "閉じた押しで描かない");
}

#[test]
fn the_view_pie_turns_the_camera_to_each_axis_and_frames_the_set() {
    let (mut h, rect) = figure_view();
    let at = rect.center() + vec2(0.0, 40.0);
    let open = |h: &mut H| {
        move_to(h, at);
        h.run();
        h.state_mut()
            .apply(Action::Pie(PieAction::Open("view".into())));
        h.run();
        assert_eq!(pie_open(h).as_deref(), Some("view"));
    };
    for (label, view) in [
        ("正面", AxisView::Front),
        ("背面", AxisView::Back),
        ("右", AxisView::Right),
        ("左", AxisView::Left),
        ("上", AxisView::Top),
        ("下", AxisView::Bottom),
    ] {
        open(&mut h);
        let item = pie_item(&h, label);
        click(&mut h, item.center());
        let camera = h.state().state.view3d.camera;
        let (yaw, pitch) = view.orientation();
        assert_eq!(camera.pitch, pitch, "{label}");
        assert!(
            ((camera.yaw - yaw).rem_euclid(360.0)).min((yaw - camera.yaw).rem_euclid(360.0)) < 1e-3,
            "{label}: {} / {yaw}",
            camera.yaw
        );
        assert!(pie_open(&h).is_none());
        // 軸の向きでは正投影（設定の既定）
        assert!(camera.is_orthographic(), "{label}");
    }
    // 正投影の項目: 透視と正投影を切り替える（押すと閉じる）
    open(&mut h);
    let ortho = pie_item(&h, "正投影");
    click(&mut h, ortho.center());
    assert!(pie_open(&h).is_none());
    assert!(!h.state().state.view3d.camera.is_orthographic());
    // 収める: 3D の . と同じ（選んだセットを画面に収める）
    h.state_mut().state.view3d.camera.distance *= 3.0;
    let mut expected = h.state().state.view3d.camera;
    let bounds = yolu_app::view3d::navigation::selected_bounds(&h.state().state.view3d).unwrap();
    expected.frame_bounds(&bounds, rect.width(), rect.height());
    open(&mut h);
    let frame = pie_item(&h, "収める");
    click(&mut h, frame.center());
    let camera = h.state().state.view3d.camera;
    assert!(
        (camera.distance - expected.distance).abs() < 1e-4,
        "{camera:?}"
    );
    assert!((camera.target - expected.target).length() < 1e-4);
}

#[test]
fn the_mode_does_not_change_while_drawing() {
    let (mut h, rect) = figure_view();
    let at = body_point(&h, rect);
    press(&h, at, PointerButton::Primary);
    h.step();
    move_to(&h, at + vec2(6.0, 0.0));
    h.step();
    assert!(h.state().state.is_stroking());
    // Ctrl+Tab のパイは開かない・メニューからも替えない
    key(&h, Key::Tab, Modifiers::CTRL);
    h.step();
    assert!(pie_open(&h).is_none());
    assert_eq!(h.state().state.message, "描いている間はできません。");
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Edit)));
    assert_eq!(mode(&h), EditorMode::Paint);
    release(&h, at + vec2(6.0, 0.0), PointerButton::Primary);
    h.run();
    assert!(!h.state().state.is_stroking());
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Edit)));
    assert_eq!(mode(&h), EditorMode::Edit);
}

#[test]
fn switching_modes_commits_a_gizmo_drag_first() {
    let (mut h, rect) = figure_view();
    // 上腕を選んで、Z の輪を掴んで回している途中
    let (_, grab) = grab_upper_arm_ring(&mut h, rect);
    move_to(&h, grab + vec2(20.0, 10.0));
    h.step();
    // ペイントへ: ドラッグをそこまでで確定してから替える（取り消しに 1 つ）
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Paint)));
    let app = &h.state().state;
    assert_eq!(app.mode, EditorMode::Paint);
    assert!(app.view3d.pose.drag.is_none());
    let s = app.view3d.pose.session.as_ref().unwrap();
    assert!(!s.is_editing());
    assert_eq!(s.undo_len(), 1);
    assert!(s.is_posed());
    release(&h, grab + vec2(20.0, 10.0), PointerButton::Primary);
    h.run();
    assert_eq!(
        h.state()
            .state
            .view3d
            .pose
            .session
            .as_ref()
            .unwrap()
            .undo_len(),
        1
    );
    assert!(!h.state().state.doc.can_undo(), "離した押しで描かない");
}

#[test]
fn the_brush_and_color_panels_are_dimmed_with_a_reason_outside_paint_mode() {
    let (mut h, _) = figure_view();
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Edit)));
    h.run();
    let at = dock_rect(&h, "直径").center();
    hover_and_wait(&mut h, at);
    assert!(h.query_by_label("ペイントのモードで使います").is_some());
    // 押しても変わらない
    let radius = h.state().state.brush.radius;
    click(&mut h, at + vec2(40.0, 0.0));
    assert_eq!(h.state().state.brush.radius, radius);
}

/// ポーズのモードで右上腕を選び、Z の輪を掴んで少し回している途中にする（掴んだ点を返す）。
fn grab_upper_arm_ring(h: &mut H, rect: Rect) -> (usize, Pos2) {
    h.state_mut().apply(Action::Pose(PoseAction::ToggleMode));
    h.run();
    let upper = {
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
        .selected = Some(upper);
    h.run();
    let r = gizmo::current_rings(&h.state().state, rect).expect("輪");
    let (_, s, _) = r.points[2]
        .iter()
        .find(|(_, _, front)| *front)
        .copied()
        .unwrap();
    let grab = pos2(rect.left() + s.x, rect.top() + s.y);
    press(h, grab, PointerButton::Primary);
    h.step();
    assert!(h.state().state.view3d.pose.drag.is_some(), "輪を掴んだ");
    move_to(h, grab + vec2(20.0, 20.0));
    h.step();
    (upper, grab + vec2(20.0, 20.0))
}

fn bone_rotation(h: &H, bone: usize) -> yolu_core::glam::Quat {
    h.state()
        .state
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .pose()
        .locals[bone]
        .rotation
}

#[test]
fn a_pie_does_not_open_during_a_drag_and_the_drag_goes_on_unchanged() {
    let (mut h, rect) = figure_view();
    // ポーズのギズモのドラッグの途中: 開かずに断り、ドラッグは続く（確定もしない）
    let (upper, at) = grab_upper_arm_ring(&mut h, rect);
    let turned = bone_rotation(&h, upper);
    key(&h, Key::Tab, Modifiers::CTRL);
    h.step();
    assert!(pie_open(&h).is_none());
    assert_eq!(
        h.state().state.message,
        "パイメニューを開けません（ほかの操作の途中です）。"
    );
    {
        let app = &h.state().state;
        assert_eq!(app.mode, EditorMode::Pose);
        assert!(app.view3d.pose.drag.is_some(), "ドラッグは続く");
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert!(s.is_editing() && s.undo_len() == 0, "確定していない");
    }
    assert!(
        bone_rotation(&h, upper).angle_between(turned) < 1e-6,
        "キーで回らない"
    );
    // パイがあったら「ペイント」を指す向き（左）へ動かしても、ただのドラッグとして回るだけ
    move_to(&h, at + vec2(-120.0, 0.0));
    h.step();
    assert!(pie_open(&h).is_none());
    assert_eq!(mode(&h), EditorMode::Pose);
    release(&h, at + vec2(-120.0, 0.0), PointerButton::Primary);
    h.run();
    assert_eq!(
        h.state()
            .state
            .view3d
            .pose
            .session
            .as_ref()
            .unwrap()
            .undo_len(),
        1,
        "離したときに 1 回の取り消し"
    );
    // 右ドラッグで回している途中: 開かず、カメラはキーで変わらない
    let c = rect.center();
    press(&h, c, PointerButton::Secondary);
    h.step();
    move_to(&h, c + vec2(30.0, 10.0));
    h.step();
    assert!(h.state().state.view3d.input.nav.is_some());
    let camera = h.state().state.view3d.camera;
    key(&h, Key::Tab, Modifiers::CTRL);
    h.step();
    assert!(pie_open(&h).is_none());
    assert_eq!(h.state().state.view3d.camera, camera);
    release(&h, c + vec2(30.0, 10.0), PointerButton::Secondary);
    h.run();
    // 何も押していなければ開く
    key(&h, Key::Tab, Modifiers::CTRL);
    h.run();
    assert_eq!(pie_open(&h).as_deref(), Some("mode"));
}

#[test]
fn escape_that_closes_a_pie_or_a_menu_does_not_reach_the_canvas() {
    let mut h = app(1280.0, 860.0, 256);
    h.state_mut().apply(Action::SelectTool(Tool::Polygon));
    h.run();
    let c = canvas_rect(&h).center();
    click(&mut h, offset(c, -60.0, -40.0));
    click(&mut h, offset(c, 60.0, -40.0));
    assert_eq!(h.state().state.sel.polygon.len(), 2, "多角形の点を 2 つ");
    // パイを Esc で閉じても、多角形の点はそのまま
    tap_mode_pie(&mut h, offset(c, 0.0, 60.0));
    assert_eq!(pie_open(&h).as_deref(), Some("mode"));
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(pie_open(&h).is_none());
    assert_eq!(h.state().state.sel.polygon.len(), 2);
    // メニューを Esc で閉じても同じ
    let at = menu_title(&h, "表示").center();
    click(&mut h, at);
    assert!(h.state().state.popup.is_some());
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(h.state().state.popup.is_none());
    assert_eq!(h.state().state.sel.polygon.len(), 2);
    // 何も開いていなければ、Esc は多角形の途中をやめる（今までどおり）
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(h.state().state.sel.polygon.is_empty());
}

#[test]
fn a_quick_ctrl_tab_at_the_corners_and_the_top_edge_stays_open() {
    let (mut h, _) = figure_view();
    for at in [
        pos2(3.0, 3.0),
        pos2(1277.0, 3.0),
        pos2(3.0, 857.0),
        pos2(1277.0, 857.0),
        pos2(640.0, 3.0),
    ] {
        tap_mode_pie(&mut h, at);
        assert_eq!(pie_open(&h).as_deref(), Some("mode"), "{at:?}");
        assert_ne!(
            pie_center(&h),
            at,
            "{at:?}: 項目が入るように真ん中をずらした"
        );
        assert_eq!(mode(&h), EditorMode::Paint, "{at:?}: 項目を実行しない");
        key(&h, Key::Escape, Modifiers::NONE);
        h.run();
        assert!(pie_open(&h).is_none());
    }
}

#[test]
fn holding_tab_after_choosing_does_not_reopen_the_pie_by_key_repeat() {
    let (mut h, rect) = figure_view();
    move_to(&h, rect.center());
    h.run();
    key_event(&h, Key::Tab, true, Modifiers::CTRL);
    h.step();
    assert_eq!(pie_open(&h).as_deref(), Some("mode"));
    // Tab を押したまま、項目をクリックで選ぶ
    let edit = pie_item(&h, "編集");
    click(&mut h, edit.center());
    assert_eq!(mode(&h), EditorMode::Edit);
    assert!(pie_open(&h).is_none());
    // 押したままのキーの繰り返し（OS の repeat の押し）では開かない
    for _ in 0..3 {
        h.event(Event::Key {
            key: Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: Modifiers::CTRL,
        });
        h.step();
    }
    h.run();
    assert!(pie_open(&h).is_none());
    key_event(&h, Key::Tab, false, Modifiers::CTRL);
    h.run();
    // 押し直せば開く
    key(&h, Key::Tab, Modifiers::CTRL);
    h.run();
    assert_eq!(pie_open(&h).as_deref(), Some("mode"));
}

fn alt_click(h: &mut H, at: Pos2) {
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerMoved(at));
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::ALT,
        });
        h.step();
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

fn shift_click(h: &mut H, at: Pos2) {
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerMoved(at));
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::SHIFT,
        });
        h.step();
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

#[test]
fn clone_sources_the_shift_line_and_the_bucket_do_nothing_outside_paint_mode() {
    let (mut h, rect) = figure_view();
    let at = body_point(&h, rect);
    h.state_mut().state.m2.brush.effect = yolu_app::engine::BrushEffect::Clone {
        offset: Default::default(),
    };
    for m in [EditorMode::Edit, EditorMode::Pose] {
        h.state_mut().apply(Action::Mode(ModeAction::Set(m)));
        h.run();
        // 3D: Alt＋クリックのクローンの元・Shift＋クリックの直線
        alt_click(&mut h, at);
        assert!(h.state().state.clone.source.is_none(), "{m:?}: 3D の元");
        shift_click(&mut h, at);
        assert!(!h.state().state.doc.can_undo(), "{m:?}: 3D の直線");
        assert!(!h.state().state.is_stroking());
    }
    // 2D: Alt＋クリックのクローンの元・Shift＋クリックの直線
    click_tab(&mut h, yolu_app::Tab::Canvas);
    h.run();
    let c = canvas_rect(&h).center();
    for m in [EditorMode::Edit, EditorMode::Pose] {
        h.state_mut().apply(Action::Mode(ModeAction::Set(m)));
        h.run();
        alt_click(&mut h, c);
        let doc = h.state().state.doc.id();
        assert!(
            h.state().state.clone.canvas_source_for(doc).is_none(),
            "{m:?}: 2D の元"
        );
        shift_click(&mut h, offset(c, 40.0, 0.0));
        assert!(!h.state().state.doc.can_undo(), "{m:?}: 2D の直線");
    }
    // 近い色のバケツ（2D と 3D）
    h.state_mut().apply(Action::SelectTool(Tool::Fill));
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Edit)));
    h.run();
    click(&mut h, c);
    assert!(!h.state().state.doc.can_undo(), "2D のバケツ");
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    click(&mut h, at);
    assert!(!h.state().state.doc.can_undo(), "3D のバケツ");
    assert!(!h.state().state.is_stroking());
    // ペイントへ戻すと、同じ Alt＋クリックで 3D の元が決まる
    h.state_mut().apply(Action::SelectTool(Tool::Brush));
    h.state_mut().state.m2.brush.effect = yolu_app::engine::BrushEffect::Clone {
        offset: Default::default(),
    };
    assert_eq!(mode(&h), EditorMode::Paint);
    alt_click(&mut h, at);
    assert!(
        h.state().state.clone.source.is_some(),
        "ペイントでは元が決まる"
    );
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
fn snapshot_the_mode_dropdown_in_both_languages() {
    for (lang, name, current) in [
        (Lang::Ja, "mode_dropdown_ja", "モード: 編集"),
        (Lang::En, "mode_dropdown_en", "Mode: Edit"),
    ] {
        let (mut h, _) = figure_view();
        h.state_mut().state.set_language(lang);
        h.state_mut()
            .apply(Action::Mode(ModeAction::Set(EditorMode::Edit)));
        h.run();
        let dropdown = bar_rect(&h, current);
        click(&mut h, dropdown.center());
        move_to(&h, pos2(700.0, 500.0));
        h.run();
        let popup = h.state().state.popup.as_ref().unwrap().state.rect;
        crop(&mut h, dropdown.union(popup).expand(4.0), name);
    }
}

#[test]
fn snapshot_the_mode_and_view_pies_in_both_languages() {
    for (lang, menu, name) in [
        (Lang::Ja, "mode", "mode_pie_ja"),
        (Lang::En, "mode", "mode_pie_en"),
        (Lang::Ja, "view", "view_pie_ja"),
        (Lang::En, "view", "view_pie_en"),
    ] {
        let (mut h, rect) = figure_view();
        h.state_mut().state.set_language(lang);
        move_to(&h, rect.center());
        h.run();
        h.state_mut()
            .apply(Action::Pie(PieAction::Open(menu.into())));
        h.run();
        assert_eq!(pie_open(&h).as_deref(), Some(menu));
        let c = pie_center(&h);
        crop(&mut h, Rect::from_center_size(c, vec2(480.0, 300.0)), name);
    }
}
