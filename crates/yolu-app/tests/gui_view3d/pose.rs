//! ポーズの変更の画面の試験（egui_kittest。描画は wgpu のソフトの描画）: 試しの人形を読む、面を押して骨を選ぶ、ギズモで回す
//! （取り消し・Esc・フォーカスを失う）、ポーズを付けた形に描く、描いている間はポーズを変えない、BlendShape のスライダー、
//! 取り消しの行き先（読むだけのセット・別のタブ）、ファイルのウィンドウの頼み、1 フレームに複数のポインタの動き・モデルの入れ替え。
//! ポーズの欄はドックのタブ（`Tab::Pose`）。欄の中の試験（インスペクター・戻し・面を隠す・日英）は `pose_ui.rs`。
use crate::common;

use common::*;
use egui::{pos2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::engine::composite_pixel;
use yolu_app::mode::EditorMode;
use yolu_app::sets::{MaterialRef, TextureSets};
use yolu_app::state::{blank_document, Action, DialogRequest};
use yolu_app::view3d::gizmo;
use yolu_app::view3d::pose::{self, PoseAction};
use yolu_app::YoluApp;
use yolu_core::glam::{Quat, Vec2, Vec3};

/// 試しの人形を読んで、3D ビューのタブが前に出たウィンドウ。
fn figure_view(doc: u32) -> (Harness<'static, YoluApp>, Rect) {
    let mut h = app(1280.0, 860.0, doc);
    h.state_mut().apply(Action::Pose(PoseAction::LoadFigure));
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブが前に出た");
    (h, rect)
}

/// ポーズのタブをドックから外して、左の上に浮いたウィンドウにする（egui_dock は外したウィンドウの大きさを渡した矩形の 0.8 倍にする。ドックの隅の狭い場所では節がスクロールになるので、欄が広く見えるように）。
fn float_pose_tab(h: &mut Harness<'static, YoluApp>) {
    {
        let dock = &mut h.state_mut().dock;
        let path = dock.find_tab(&yolu_app::Tab::Pose).expect("ポーズのタブ");
        dock.detach_tab(
            path,
            Rect::from_min_size(pos2(8.0, 40.0), egui::vec2(400.0, 980.0)),
        );
    }
    h.run();
}

fn screen_of(h: &Harness<'_, YoluApp>, rect: Rect, p: Vec3) -> Pos2 {
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(rect.width(), rect.height());
    let s = view.to_screen(p).expect("カメラの前");
    pos2(rect.left() + s.x, rect.top() + s.y)
}

fn bone(h: &Harness<'_, YoluApp>, name: &str) -> usize {
    let s = h.state().state.view3d.pose.session.as_ref().unwrap();
    s.rig.bones().iter().position(|b| b.name == name).unwrap()
}

/// 右腕の筒（メッシュ 1）の三角形のうち、x が from..to（休みの形）で、カメラに向いた三角形の真ん中の画面の点と、その三角形の
/// 通し番号・UV。
fn arm_point(h: &Harness<'_, YoluApp>, rect: Rect, from: f32, to: f32) -> (Pos2, usize, Vec2) {
    let app = &h.state().state;
    let s = app.view3d.pose.session.as_ref().unwrap();
    let rest = s.rest_geometry();
    let model = app.view3d.model.as_ref().unwrap();
    let first = s.rig.meshes()[0].mesh.triangle_count();
    let count = s.rig.meshes()[1].mesh.triangle_count();
    let cam = app.view3d.camera.position();
    for i in first..first + count {
        let r = &rest.triangles()[i];
        let x = (r.a.x + r.b.x + r.c.x) / 3.0;
        if x < from || x > to {
            continue;
        }
        let t = &model.geometry.triangles()[i];
        let c = (t.a + t.b + t.c) / 3.0;
        if t.normal().dot((cam - c).normalize()) > 0.6 {
            return (screen_of(h, rect, c), i, (t.uv_a + t.uv_b + t.uv_c) / 3.0);
        }
    }
    panic!("カメラに向いた腕の三角形が無い");
}

/// ギズモの輪 axis の、カメラ側で、ほかの輪から離れた点。
fn ring_point(h: &Harness<'_, YoluApp>, rect: Rect, axis: usize) -> Pos2 {
    let r = gizmo::current_rings(&h.state().state, rect).expect("輪");
    let others: Vec<Vec2> = r
        .points
        .iter()
        .enumerate()
        .filter(|(a, _)| *a != axis)
        .flat_map(|(_, p)| p.iter().map(|x| x.1))
        .collect();
    let (_, s, _) = r.points[axis]
        .iter()
        .filter(|(_, _, front)| *front)
        .max_by(|a, b| {
            let d = |s: Vec2| {
                others
                    .iter()
                    .map(|o| (*o - s).length())
                    .fold(f32::MAX, f32::min)
            };
            d(a.1).total_cmp(&d(b.1))
        })
        .expect("輪の前の点");
    pos2(rect.left() + s.x, rect.top() + s.y)
}

fn rotation(h: &Harness<'_, YoluApp>, b: usize) -> Quat {
    h.state()
        .state
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .pose()
        .locals[b]
        .rotation
}

fn undo_len(h: &Harness<'_, YoluApp>) -> usize {
    h.state()
        .state
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .undo_len()
}

fn select_upper_arm(h: &mut Harness<'static, YoluApp>, rect: Rect) -> usize {
    h.state_mut().apply(Action::Pose(PoseAction::ToggleMode));
    h.run();
    // 上腕（x = 0.24〜0.4）の面を押す
    let (at, _, _) = arm_point(h, rect, 0.24, 0.4);
    click(h, at);
    let upper = bone(h, "右上腕");
    assert_eq!(
        h.state()
            .state
            .view3d
            .pose
            .session
            .as_ref()
            .unwrap()
            .selected,
        Some(upper),
        "面を押すと、そこを動かす骨"
    );
    upper
}

#[test]
fn loading_the_figure_shows_the_pose_panel_and_tree() {
    let (mut h, rect) = figure_view(256);
    {
        let app = &h.state().state;
        assert!(app.view3d.pose.session.is_some());
        assert_eq!(app.view3d.model.as_ref().unwrap().name, "試しの人形");
    }
    // 3D の表示域はポーズの欄に削られない（欄はドックのタブ）
    assert!(rect.width() > 500.0, "{rect:?}");
    h.state_mut().state.ui.sections.insert("pose.hide", false);
    float_pose_tab(&mut h);
    // 木: 根と、その下の 1 段が開いている
    for name in ["腰", "背骨", "右太もも"] {
        assert!(h.query_by_label(name).is_some(), "{name}");
    }
    assert!(
        h.query_by_label("右上腕").is_none(),
        "閉じた枝の下は出さない"
    );
    // BlendShape のスライダー（メッシュが 2 つなのでメッシュの名前を挟む）
    assert!(h.query_by_label("おなか").is_some());
    assert!(h.query_by_label("頭を伸ばす").is_some());
}

#[test]
fn the_gizmo_rotates_the_selected_bone_with_undo_escape_and_focus_loss() {
    let (mut h, rect) = figure_view(256);
    float_pose_tab(&mut h);
    let upper = select_upper_arm(&mut h, rect);
    assert!(h.query_by_label("右上腕").is_some(), "選んだ骨まで木が開く");
    assert!(
        !h.state().state.doc.can_undo(),
        "ポーズのモードでは描かない"
    );
    let rest = rotation(&h, upper);
    // Z の輪を掴んで、輪に沿って動かす
    let at = ring_point(&h, rect, 2);
    let r = gizmo::current_rings(&h.state().state, rect).unwrap();
    let c = pos2(
        rect.left() + r.center_screen.x,
        rect.top() + r.center_screen.y,
    );
    let radial = (at - c).normalized();
    let tangent = egui::vec2(-radial.y, radial.x);
    press(&h, at, PointerButton::Primary);
    h.step();
    assert!(h.state().state.view3d.pose.drag.is_some(), "輪を掴んだ");
    for k in 1..=4 {
        move_to(&h, at + tangent * (10.0 * k as f32));
        h.step();
    }
    release(&h, at + tangent * 40.0, PointerButton::Primary);
    h.run();
    let turned = rotation(&h, upper);
    assert!(turned.angle_between(rest) > 0.2, "回った: {turned}");
    assert_eq!(undo_len(&h), 1, "ドラッグ 1 回で取り消し 1 つ");
    assert!(h.state().state.view3d.pose.drag.is_none());
    assert!(!h.state().state.doc.can_undo(), "画素は変わらない");
    // ポーズのモードでは Ctrl+Z がポーズを戻す
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(rotation(&h, upper).angle_between(rest) < 1e-5);
    key(&h, Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    h.run();
    assert!(rotation(&h, upper).angle_between(turned) < 1e-5);
    // Esc は始まりへ戻す（取り消しに積まない）
    let at = ring_point(&h, rect, 0);
    press(&h, at, PointerButton::Primary);
    h.step();
    move_to(&h, at + egui::vec2(25.0, 25.0));
    h.step();
    assert!(
        rotation(&h, upper).angle_between(turned) > 1e-3,
        "動かしている途中"
    );
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    release(&h, at + egui::vec2(25.0, 25.0), PointerButton::Primary);
    h.run();
    assert!(rotation(&h, upper).angle_between(turned) < 1e-5);
    assert_eq!(undo_len(&h), 1);
    // フォーカスを失ったら、そこまでを確定する
    let at = ring_point(&h, rect, 1);
    press(&h, at, PointerButton::Primary);
    h.step();
    move_to(&h, at + egui::vec2(-30.0, 10.0));
    h.step();
    h.event(Event::WindowFocused(false));
    h.run();
    assert!(h.state().state.view3d.pose.drag.is_none());
    assert_eq!(undo_len(&h), 2);
    assert!(!h
        .state()
        .state
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .is_editing());
    // 戻す
    h.state_mut().apply(Action::Pose(PoseAction::Reset));
    h.run();
    assert!(!h
        .state()
        .state
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .is_posed());
}

#[test]
fn painting_lands_on_the_posed_shape_in_the_arm_uvs() {
    let (mut h, rect) = figure_view(512);
    h.state_mut().state.color.set_main([0.9, 0.1, 0.1, 1.0]);
    h.state_mut().state.brush.radius = 4.0;
    // 右腕を下ろして肘を曲げる
    let (upper, lower) = (bone(&h, "右上腕"), bone(&h, "右前腕"));
    let mut p = h
        .state()
        .state
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .pose()
        .clone();
    p.locals[upper].rotation = Quat::from_rotation_z(-1.2);
    p.locals[lower].rotation = Quat::from_rotation_y(0.6);
    pose::set_pose(&mut h.state_mut().state.view3d, p).unwrap();
    h.run();
    // 前腕（x = 0.55〜0.7）の、ポーズを付けた後の場所を押して描く
    let (at, tri, uv) = arm_point(&h, rect, 0.55, 0.7);
    let rest = h
        .state()
        .state
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .rest_geometry()
        .triangles()[tri];
    let rest_center = (rest.a + rest.b + rest.c) / 3.0;
    assert!(
        (screen_of(&h, rect, rest_center) - at).length() > 40.0,
        "休みの形とは違う場所"
    );
    click(&mut h, at);
    let app = &h.state().state;
    assert!(app.doc.can_undo(), "{}", app.message);
    let size = app.doc.width() as f32;
    let px = composite_pixel(&app.doc, (uv.x * size) as u32, (uv.y * size) as u32);
    assert!(
        px[3] > 0 && px[0] > 150,
        "押した三角形の UV に描いた: {px:?}"
    );
    // 描いたのは腕の UV アイランド（u 0.3〜0.47）の中だけ
    let doc = &app.doc;
    for y in (0..doc.height()).step_by(2) {
        for x in (0..doc.width()).step_by(2) {
            if composite_pixel(doc, x, y)[3] > 0 {
                let u = x as f32 / size;
                assert!((0.3..0.47).contains(&u), "アイランドの外 ({x}, {y})");
            }
        }
    }
    // 画素の取り消しはポーズに触れない（ポーズのモードではない）
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(!h.state().state.doc.can_undo());
    assert!(h
        .state()
        .state
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .is_posed());
}

#[test]
fn the_pose_does_not_change_during_a_stroke_and_no_stroke_is_left_behind() {
    let (mut h, rect) = figure_view(256);
    // 胴に描き始める
    let s = h.state().state.view3d.pose.session.as_ref().unwrap();
    let model = h.state().state.view3d.model.clone().unwrap();
    let cam = h.state().state.view3d.camera.position();
    let n = s.rig.meshes()[0].mesh.triangle_count();
    let t = model.geometry.triangles()[..n]
        .iter()
        .find(|t| {
            let c = (t.a + t.b + t.c) / 3.0;
            t.normal().dot((cam - c).normalize()) > 0.8 && c.y > 1.1
        })
        .copied()
        .unwrap();
    let at = screen_of(&h, rect, (t.a + t.b + t.c) / 3.0);
    press(&h, at, PointerButton::Primary);
    h.step();
    move_to(&h, at + egui::vec2(6.0, 0.0));
    h.step();
    assert!(h.state().state.is_stroking());
    let revision = h.state().state.view3d.model.as_ref().unwrap().revision();
    for a in [
        PoseAction::ToggleMode,
        PoseAction::Reset,
        PoseAction::LoadFigure,
    ] {
        h.state_mut().apply(Action::Pose(a));
        assert_eq!(h.state().state.message, "描いている間はできません。");
    }
    let p = h
        .state()
        .state
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .pose()
        .clone();
    assert!(pose::set_pose(&mut h.state_mut().state.view3d, p).is_err());
    assert_eq!(h.state().state.mode, EditorMode::Paint);
    h.step();
    assert_eq!(
        h.state().state.view3d.model.as_ref().unwrap().revision(),
        revision
    );
    // Esc でストロークを捨てる: 取り残さない
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    release(&h, at, PointerButton::Primary);
    h.run();
    assert!(!h.state().state.is_stroking());
    assert!(!h.state().state.doc.has_active_stroke());
    assert!(!h.state().state.doc.can_undo());
    // もう一度描いて、フォーカスを失ったら確定する
    press(&h, at, PointerButton::Primary);
    h.step();
    h.event(Event::WindowFocused(false));
    h.run();
    assert!(!h.state().state.doc.has_active_stroke());
    assert!(h.state().state.doc.can_undo());
    release(&h, at, PointerButton::Primary);
    h.run();
    // 描き終えたらポーズを変えられる
    h.state_mut().apply(Action::Pose(PoseAction::ToggleMode));
    assert_eq!(h.state().state.mode, EditorMode::Pose);
}

#[test]
fn a_blend_shape_slider_changes_the_shape_as_one_undo_step() {
    let (mut h, _rect) = figure_view(256);
    h.state_mut().state.ui.sections.insert("pose.hide", false);
    float_pose_tab(&mut h);
    let before: Vec<Vec3> = h.state().state.view3d.model.as_ref().unwrap().meshes[0]
        .positions
        .clone();
    let slider = h.get_by_label("おなか").rect();
    // 2 行のスライダー: 2 行目の溝を左から右の 7 割まで（右端は戻すボタンの分だけ空けてある）
    let y = slider.bottom() - 6.0;
    let points: Vec<Pos2> = (0..=6)
        .map(|k| {
            pos2(
                slider.left() + 4.0 + (slider.width() * 0.7 - 4.0) * k as f32 / 6.0,
                y,
            )
        })
        .collect();
    drag(&mut h, &points);
    let app = &h.state().state;
    let s = app.view3d.pose.session.as_ref().unwrap();
    let w = s.pose().blend_weights[0][0];
    assert!((60.0..=90.0).contains(&w), "重み {w}");
    assert_eq!(s.undo_len(), 1, "ドラッグ 1 回で取り消し 1 つ");
    let after = &app.view3d.model.as_ref().unwrap().meshes[0].positions;
    assert!(
        before
            .iter()
            .zip(after)
            .any(|(a, b)| (*a - *b).length() > 0.01),
        "おなかが出た"
    );
    assert!(!app.doc.can_undo());
    h.state_mut().apply(Action::Pose(PoseAction::Undo));
    h.run();
    assert_eq!(
        h.state().state.view3d.model.as_ref().unwrap().meshes[0].positions,
        before
    );
}

/// 上腕を z まわりに angle 回したポーズにする（取り消しに 1 つ積む）。
fn bend_upper_arm(h: &mut Harness<'static, YoluApp>, angle: f32) {
    let upper = bone(h, "右上腕");
    let mut p = h
        .state()
        .state
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .pose()
        .clone();
    p.locals[upper].rotation = Quat::from_rotation_z(angle);
    pose::set_pose(&mut h.state_mut().state.view3d, p).unwrap();
}

fn is_posed(h: &Harness<'_, YoluApp>) -> bool {
    h.state()
        .state
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .is_posed()
}

#[test]
fn pose_undo_works_in_a_read_only_texture_set() {
    let (mut h, _) = figure_view(256);
    let upper = bone(&h, "右上腕");
    bend_upper_arm(&mut h, -0.5);
    bend_upper_arm(&mut h, -1.0);
    // 今のテクスチャセットを読むだけにする（ポーズは文書を変えないので、取り消しは断られない）
    let (doc, _) = blank_document(256, 256);
    let (sets, doc) = TextureSets::from_parts(
        vec![(
            "00000000-0000-4000-8000-000000000011".into(),
            "読むだけ".into(),
            MaterialRef::PendingSlot(0),
            Some("フィルターのあるレイヤーがあります".into()),
            doc,
        )],
        0,
    );
    h.state_mut().state.replace_sets(sets, doc);
    h.state_mut().apply(Action::Pose(PoseAction::ToggleMode));
    h.run();
    let app = &h.state().state;
    assert!(app.read_only_reason().is_some());
    assert!(app.mode == EditorMode::Pose && app.view3d.visible);
    assert_eq!(undo_len(&h), 2);
    // Ctrl+Z・Ctrl+Shift+Z は、読むだけのセットでもポーズを戻す・やり直す
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(undo_len(&h), 1);
    assert!(rotation(&h, upper).angle_between(Quat::from_rotation_z(-0.5)) < 1e-5);
    assert!(
        !h.state().state.message.contains("読むだけ"),
        "{}",
        h.state().state.message
    );
    key(&h, Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    h.run();
    assert_eq!(undo_len(&h), 2);
    // 編集メニューからも同じ（経路で結果が分かれない）
    let at = menu_title(&h, "編集").center();
    click(&mut h, at);
    let at = popup_item(&h, "取り消し").center();
    click(&mut h, at);
    assert_eq!(undo_len(&h), 1);
    assert!(h.state().state.read_only_reason().is_some());
    // ポーズのモードでなければ、読むだけのセットの文書の取り消しは断られる
    h.state_mut().apply(Action::Pose(PoseAction::ToggleMode));
    h.state_mut().apply(Action::Undo);
    assert!(
        h.state().state.message.contains("読むだけ"),
        "{}",
        h.state().state.message
    );
    assert_eq!(undo_len(&h), 1, "ポーズは動かない");
}

#[test]
fn undo_goes_to_the_pixels_while_the_3d_tab_is_behind_the_canvas() {
    let (mut h, _) = figure_view(256);
    bend_upper_arm(&mut h, -0.8);
    // ペイントのモードで 2D に描いてから、ポーズのモードへ（ポーズのモードの 2D のキャンバスは見るだけ）
    click_tab(&mut h, yolu_app::Tab::Canvas);
    h.run();
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -20.0, 0.0), offset(c, 20.0, 0.0)]);
    assert!(canvas_pixel(&h, c)[3] > 0, "描けた");
    assert!(h.state().state.doc.can_undo());
    h.state_mut().apply(Action::Pose(PoseAction::ToggleMode));
    h.run();
    // キャンバスのタブが前（ポーズのモードは残る）
    assert!(h.state().view3d_rect().is_none(), "3D ビューは裏");
    assert!(!h.state().state.view3d.visible);
    assert_eq!(h.state().state.mode, EditorMode::Pose, "モードは残る");
    // Ctrl+Z は見えているキャンバスの画素を戻し、見えていないポーズは戻さない
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(canvas_pixel(&h, c)[3], 0, "画素が戻った");
    assert!(is_posed(&h), "見えていないポーズは戻らない");
    assert_eq!(undo_len(&h), 1);
    // 3D ビューへ戻れば、続きから（Ctrl+Z はポーズ）
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    assert!(h.state().state.view3d.visible);
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(!is_posed(&h), "ポーズが戻った");
}

#[test]
fn opening_an_fbx_asks_for_the_file_window_instead_of_opening_one() {
    let (mut h, _) = figure_view(256);
    // 欄のボタン（ポーズのタブの頭）
    click_tab(&mut h, yolu_app::Tab::Pose);
    let at = h.get_by_label("FBX を開く").rect().center();
    click(&mut h, at);
    assert_eq!(
        h.state().state.dialog_request,
        Some(DialogRequest::OpenModel),
        "ウィンドウは YoluApp が開く（試験では開かず、頼みが残る）"
    );
    assert!(!h.state().state.view3d.pose.is_loading());
    h.state_mut().state.dialog_request = None;
    // メニュー（ファイル）には無い（モデルは、新規プロジェクト・プロジェクト設定・ドロップ・ポーズの欄で開く）
    let at = menu_title(&h, "ファイル").center();
    click(&mut h, at);
    assert!(h.query_by_label("3D ビューに FBX を開く…").is_none());
    assert!(!h.state().state.view3d.pose.is_loading());
}

/// ギズモの輪 2 を掴んで、輪に沿った向き（画面の点の単位の向き）と掴んだ点を返す。
fn grab_ring(h: &mut Harness<'static, YoluApp>, rect: Rect) -> (Pos2, egui::Vec2) {
    let at = ring_point(h, rect, 2);
    let r = gizmo::current_rings(&h.state().state, rect).unwrap();
    let c = pos2(
        rect.left() + r.center_screen.x,
        rect.top() + r.center_screen.y,
    );
    let radial = (at - c).normalized();
    press(h, at, PointerButton::Primary);
    h.step();
    assert!(h.state().state.view3d.pose.drag.is_some(), "輪を掴んだ");
    (at, egui::vec2(-radial.y, radial.x))
}

fn model_revision(h: &Harness<'_, YoluApp>) -> u32 {
    h.state().state.view3d.model.as_ref().unwrap().revision()
}

fn drag_angle(h: &Harness<'_, YoluApp>) -> f32 {
    h.state().state.view3d.pose.drag.as_ref().unwrap().angle
}

/// events を 1 フレームに全部入れて回す（`Harness::step` は、待たせたイベントを 1 つずつ別のフレームで流すので、
/// 高いポーリングのマウスやペンのように 1 フレームに複数届く形は、入力へ直に積む）。
fn one_frame(h: &mut Harness<'static, YoluApp>, events: impl IntoIterator<Item = Event>) {
    h.input_mut().events.extend(events);
    h.step();
}

fn moved(at: Pos2, tangent: egui::Vec2, from: usize, to: usize) -> Vec<Event> {
    (from..=to)
        .map(|k| Event::PointerMoved(at + tangent * k as f32))
        .collect()
}

#[test]
fn many_pointer_moves_in_one_frame_rebuild_the_pose_once() {
    let (mut h, rect) = figure_view(256);
    let upper = select_upper_arm(&mut h, rect);
    let (at, tangent) = grab_ring(&mut h, rect);
    let before = model_revision(&h);
    // 1 フレームに 12 回のポインタの動き
    one_frame(&mut h, moved(at, tangent, 1, 12));
    assert_eq!(
        model_revision(&h) - before,
        1,
        "途中の位置では組み直さない（最後の 1 回だけ）"
    );
    let angle = drag_angle(&h);
    let turned = rotation(&h, upper);
    assert!(angle.abs() > 0.05, "{angle}");
    // 動きのあとにボタンを離す: 離す前の動きまで当ててから確定する
    let mut events = moved(at, tangent, 13, 16);
    events.push(Event::PointerButton {
        pos: at + tangent * 16.0,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    one_frame(&mut h, events);
    h.run();
    assert!(h.state().state.view3d.pose.drag.is_none());
    assert_eq!(undo_len(&h), 1);
    assert!(
        rotation(&h, upper).angle_between(turned) > 1e-3,
        "離す前の動きまで当たっている"
    );
    // 同じ最後の位置へ 1 回で動かした場合と、同じ角度・同じ回転
    let (mut h2, rect2) = figure_view(256);
    let upper2 = select_upper_arm(&mut h2, rect2);
    let (at2, tangent2) = grab_ring(&mut h2, rect2);
    assert_eq!((at2, tangent2), (at, tangent));
    let before2 = model_revision(&h2);
    one_frame(&mut h2, moved(at2, tangent2, 12, 12));
    assert_eq!(model_revision(&h2) - before2, 1);
    assert!((drag_angle(&h2) - angle).abs() < 1e-6);
    assert!(rotation(&h2, upper2).angle_between(turned) < 1e-6);
}

#[test]
fn the_renderer_catches_up_when_the_model_is_replaced_twice_in_a_frame() {
    let (mut h, _) = figure_view(256);
    h.run();
    for k in 0..40 {
        // 1 フレームに 2 回入れ替える（古いモデルを落とし、新しいモデルが続く。アドレスは再利用されうる）
        bend_upper_arm(&mut h, -0.2 - 0.01 * k as f32);
        bend_upper_arm(&mut h, -0.9 - 0.01 * k as f32);
        let renders = h.state().view3d_stats().unwrap().renders;
        h.step();
        let stats = h.state().view3d_stats().unwrap();
        assert_eq!(
            stats.mesh_revision,
            model_revision(&h),
            "{k}: GPU の頂点が最後のモデルに追いついた"
        );
        assert_eq!(stats.renders, renders + 1, "{k}: 描き直した");
    }
}

#[test]
fn pose_gizmo_snapshot() {
    let (mut h, rect) = figure_view(256);
    select_upper_arm(&mut h, rect);
    // ポインタは輪の外へ（強調しない）
    move_to(&h, pos2(rect.right() - 10.0, rect.bottom() - 10.0));
    h.run();
    h.snapshot("pose_gizmo");
}

/// 計測（release で: cargo test --release -p yolu-app --test gui_view3d -- --ignored --nocapture pose::）。7 万三角形の試しの人形で、
/// ポーズを 1 回変えてから描けるまで: CPU（スキニング・refit・モデル）、次のフレーム（頂点を GPU へ上げて描く。kittest の
/// ソフトの描画）、最初のダブ。
#[test]
#[ignore]
fn measure_pose_to_paint_on_the_avatar_figure() {
    use std::time::Instant;
    use yolu_core::skin::{demo_figure, FigureDetail};
    let mut h = app(1280.0, 860.0, 2048);
    pose::load_rig(
        &mut h.state_mut().state.view3d,
        demo_figure(FigureDetail::AVATAR),
    )
    .unwrap();
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().unwrap();
    let upper = bone(&h, "右上腕");
    let lower = bone(&h, "右前腕");
    let (mut cpu, mut frame, mut idle, mut dab) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for k in 0..7 {
        let mut p = h
            .state()
            .state
            .view3d
            .pose
            .session
            .as_ref()
            .unwrap()
            .pose()
            .clone();
        p.locals[upper].rotation = Quat::from_rotation_z(-0.3 - 0.1 * k as f32);
        p.locals[lower].rotation = Quat::from_rotation_y(0.2 + 0.1 * k as f32);
        let clock = Instant::now();
        pose::set_pose(&mut h.state_mut().state.view3d, p).unwrap();
        cpu.push(clock.elapsed().as_secs_f64() * 1000.0);
        let clock = Instant::now();
        h.step();
        frame.push(clock.elapsed().as_secs_f64() * 1000.0);
        let clock = Instant::now();
        h.step();
        idle.push(clock.elapsed().as_secs_f64() * 1000.0);
        let (at, _, _) = arm_point(&h, rect, 0.55, 0.7);
        let clock = Instant::now();
        click(&mut h, at);
        dab.push(clock.elapsed().as_secs_f64() * 1000.0);
        h.state_mut().apply(Action::Undo);
        h.run();
    }
    let median = |mut v: Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    };
    let t = h
        .state()
        .state
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .timings;
    println!(
        "人形 {} 三角形: ポーズ → CPU {:.2} ms（最後: スキニング {:.2}・写しと refit {:.2}）、次のフレーム {:.1} ms（変わらないフレーム {:.1} ms）、最初のダブを押して離すまでのフレーム {:.1} ms",
        h.state().state.view3d.model.as_ref().unwrap().triangle_count(),
        median(cpu),
        t.skin_ms,
        t.refit_ms,
        median(frame),
        median(idle),
        median(dab)
    );
}
