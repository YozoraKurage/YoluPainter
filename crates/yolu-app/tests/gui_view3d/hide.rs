//! ハイドのモード（egui_kittest。描画は wgpu のソフトの描画）: モードへ入る・出る、3D ビューの右の引き出し（ビューごとの開閉・下でビューの
//! 押しが始まらない）、UV アイランドを押して選び H で隠す（そのビューで見えず・描けず・当たらない）・Alt+H で戻す、2 つのビューで別の隠し方
//! （シェルフの目）・別のポーズのプリセット・同じ形の共有、ボーンの丸（ズームで 3〜9 点・掴める範囲・ポーズのモードで丸が先）。
//! スクリーンショットは日英。テストのモデルは試しの立方体と試しの人形（ユーザーの FBX は使わない）。
use crate::common;
use crate::common::drive::{Drive, DriveApp};

use common::*;
use egui::{pos2, vec2, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_dock::{Split, TabDestination, TabInsert};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::lang::Lang;
use yolu_app::mode::{EditorMode, ModeAction};
use yolu_app::state::Action;
use yolu_app::view3d::bone_marks;
use yolu_app::view3d::hide::{HideAction, PoseChoice, ViewPose};
use yolu_app::view3d::pose::PoseAction;
use yolu_app::view3d::ViewId;
use yolu_app::{Tab, YoluApp};

type H = Harness<'static, YoluApp>;

const SECOND: ViewId = ViewId(1);

/// 試しの立方体を読んだウィンドウ（中央の組は 3D ビューを前に）。
fn cube() -> H {
    let mut h = app(1600.0, 900.0, 128);
    h.view3d_mut().load_demo();
    h.run();
    click_tab(&mut h, Tab::View3d);
    h
}

/// 試しの人形を読んだウィンドウ（中央の組は 3D ビューを前に）。
fn figure() -> H {
    let mut h = app(1600.0, 900.0, 128);
    h.act(Action::Pose(PoseAction::LoadFigure));
    h.run();
    click_tab(&mut h, Tab::View3d);
    h
}

/// 3D ビュー 2 を足して、3D ビュー 1 の右に並べる（ポインタはビューの外）。
fn add_second(h: &mut H) {
    h.get_by_label("3D ビューを追加").click();
    h.run();
    let dock = h.dock_mut();
    let from = dock.find_tab(&Tab::View3dMore(SECOND)).expect("足したタブ");
    let to = dock.find_tab(&Tab::View3d).expect("3D ビュー").node_path();
    dock.move_tab(
        from,
        TabDestination::Node(to, TabInsert::Split(Split::Right)),
    );
    move_to(h, pos2(1.0, 1.0));
    h.run();
}

fn rect_of_view(h: &H, id: ViewId) -> Rect {
    h.view3d_rect_of(id).expect("ビューを描いた")
}

/// ポインタをこのビューのこの点へ（乗ったビューが今のビューになる）。
fn hover_at(h: &mut H, at: Pos2) {
    move_to(h, at);
    h.run();
}

fn enter(h: &mut H, mode: EditorMode) {
    h.act(Action::Mode(ModeAction::Set(mode)));
    h.run();
    assert_eq!(h.mode(), mode, "{}", h.message());
}

/// 今のビューの見せる形の三角形の数。
fn shown(h: &H) -> usize {
    h.view3d().model.as_ref().map_or(0, |m| m.triangle_count())
}

fn shown_in(h: &H, id: ViewId) -> usize {
    h.view3d().view_model(id).map_or(0, |m| m.triangle_count())
}

/// このビューの画面で、立方体の面（アイランド）`island` の真ん中（カメラに向いていなければ None）。
fn island_point(h: &H, id: ViewId, island: u32) -> Option<Pos2> {
    let rect = rect_of_view(h, id);
    let v3 = h.view3d();
    let view = v3.view_of(id)?.camera.view(rect.width(), rect.height());
    let g = &v3.full_model()?.geometry;
    let (a, b) = (
        &g.triangles()[island as usize * 2],
        &g.triangles()[island as usize * 2 + 1],
    );
    let centre = (a.a + a.b + a.c + b.a + b.b + b.c) / 6.0;
    if a.normal().dot(view.to_viewer(centre)) <= 0.1 {
        return None;
    }
    let s = view.to_screen(centre)?;
    Some(pos2(rect.left() + s.x, rect.top() + s.y))
}

/// カメラに向いた面（アイランドの番号と、その画面の点）のうち、引き出しとシェルフに覆われていない最初のもの。狭いビューで全部の面が
/// 引き出しに覆われていれば、そのビューの引き出しを閉じてから探す（引き出しは右の端からビューを覆う）。
fn facing_island(h: &mut H, id: ViewId) -> (u32, Pos2) {
    let find = |h: &H| {
        let rect = rect_of_view(h, id);
        let open = h.view3d().view_of(id).unwrap().hide.drawer_open;
        let drawer = yolu_app::panels::hide_drawer::layout(rect, rect.top() + 40.0, open);
        (0..6)
            .filter_map(|i| island_point(h, id, i).map(|p| (i, p)))
            .find(|(_, p)| {
                !drawer.panel.is_some_and(|r| r.expand(4.0).contains(*p))
                    && !drawer.handle.expand(4.0).contains(*p)
                    && p.x > rect.left() + 70.0
            })
    };
    if let Some(found) = find(h) {
        return found;
    }
    h.act(Action::Hide(HideAction::ToggleDrawer(id)));
    h.run();
    find(h).expect("カメラに向いた面")
}

// ───────── モード・引き出し ─────────

#[test]
fn hide_mode_is_refused_without_a_model_and_the_mode_menu_lists_it() {
    let mut h = app_default(1400.0, 800.0, 64);
    h.act(Action::Mode(ModeAction::Set(EditorMode::Hide)));
    h.run();
    assert_eq!(h.mode(), EditorMode::Paint);
    assert_eq!(
        h.message(),
        "ハイドのモードにできません（モデルがありません）。"
    );
    h.view3d_mut().load_demo();
    h.run();
    // オプションバーのドロップダウンから入る
    let at = bar_rect(&h, "モード: ペイント").center();
    click(&mut h, at);
    let at = popup_item(&h, "ハイド").center();
    click(&mut h, at);
    assert_eq!(h.mode(), EditorMode::Hide);
    // 帯は UV アイランドの選び・隠す（名前とキー）
    for label in [
        "UV アイランドを選ぶ",
        "四角で選ぶ",
        "すべて選択（Ctrl+A）",
        "選択を解除（Ctrl+D）",
        "選んだ面を隠す（H）",
        "すべて表示（Alt+H）",
    ] {
        assert!(
            h.query_all_by_label(label).next().is_some(),
            "{label} が無い"
        );
    }
    // ペイントへ戻る
    enter(&mut h, EditorMode::Paint);
    assert!(h.query_by_label("UV アイランドを選ぶ").is_none());
}

#[test]
fn the_drawer_opens_and_closes_per_view_and_presses_under_it_do_not_reach_the_view() {
    let mut h = cube();
    add_second(&mut h);
    enter(&mut h, EditorMode::Hide);
    // どのビューにも引き出しが開いて出る（つまみの名前は「ハイド」）
    assert_eq!(h.query_all_by_label("ハイド").count(), 2);
    for id in [ViewId::FIRST, SECOND] {
        assert!(h.view3d().view_of(id).unwrap().hide.drawer_open);
    }
    // 3D ビュー 2 のつまみで閉じると、3D ビュー 2 だけ閉じる
    let second = rect_of_view(&h, SECOND);
    let handle = h
        .query_all_by_label("ハイド")
        .map(|n| n.rect())
        .find(|r| second.contains(r.center()))
        .unwrap();
    hover_at(&mut h, handle.center());
    click(&mut h, handle.center());
    assert!(!h.view3d().view_of(SECOND).unwrap().hide.drawer_open);
    assert!(h.view3d().view_of(ViewId::FIRST).unwrap().hide.drawer_open);
    // 3D ビュー 1 の引き出しの中（何も無い所）でドラッグしても、カメラは回らず、選びも変わらない
    let first = rect_of_view(&h, ViewId::FIRST);
    let at = pos2(first.right() - 30.0, first.bottom() - 40.0);
    hover_at(&mut h, at);
    let camera = h.view3d().view_of(ViewId::FIRST).unwrap().camera;
    for button in [PointerButton::Primary, PointerButton::Secondary] {
        press(&h, at, button);
        h.step();
        for dx in [10.0, 20.0, 30.0] {
            move_to(&h, at + vec2(-dx, 0.0));
            h.step();
        }
        release(&h, at + vec2(-30.0, 0.0), button);
        h.run();
    }
    assert_eq!(h.view3d().view_of(ViewId::FIRST).unwrap().camera, camera);
    assert!(h.view3d().view.hide.selected.is_empty());
    assert!(h.view3d().view.hide.rect.is_none());
    // モードを出ると引き出しは出ない（開閉の印は残る）
    enter(&mut h, EditorMode::Paint);
    assert_eq!(h.query_all_by_label("ハイド").count(), 0);
}

// ───────── UV アイランドを隠す ─────────

#[test]
fn the_drawer_is_not_drawn_while_the_3d_view_settings_panel_is_open() {
    let mut h = cube();
    enter(&mut h, EditorMode::Hide);
    assert_eq!(h.query_all_by_label("ハイド").count(), 1);
    // 設定のパネルは同じ右側に出るので、開いている間は引き出しとつまみを描かない
    h.view3d_mut().view.display.settings_open = true;
    h.run();
    assert_eq!(h.query_all_by_label("ハイド").count(), 0);
    // 閉じれば戻る（開閉の状態は変えない）
    h.view3d_mut().view.display.settings_open = false;
    h.run();
    assert_eq!(h.query_all_by_label("ハイド").count(), 1);
    assert!(h.view3d().view.hide.drawer_open);
}

#[test]
fn a_picked_uv_island_hides_with_h_in_that_view_only_and_alt_h_brings_it_back() {
    let mut h = cube();
    add_second(&mut h);
    enter(&mut h, EditorMode::Hide);
    let (island, at) = facing_island(&mut h, SECOND);
    hover_at(&mut h, at);
    assert_eq!(h.view3d().active_view(), SECOND);
    assert_eq!(h.view3d().view.hide.pointed, Some(island), "指した面");
    click(&mut h, at);
    assert_eq!(h.view3d().view.hide.selected, vec![island]);
    key(&h, Key::H, Modifiers::NONE);
    h.run();
    assert_eq!(shown(&h), 10, "3D ビュー 2 で面が消えた");
    assert_eq!(shown_in(&h, ViewId::FIRST), 12, "3D ビュー 1 には出ている");
    // 2 つの形の頂点は両方持つ: ビューを描くたびに上げ直さない
    h.run();
    let uploads = h.view3d_stats().unwrap().mesh_uploads;
    let cameras: Vec<_> = [ViewId::FIRST, SECOND]
        .iter()
        .map(|id| h.view3d().view_of(*id).unwrap().camera)
        .collect();
    for yaw in [1.0, 2.0, 3.0] {
        for (id, c) in [ViewId::FIRST, SECOND].iter().zip(&cameras) {
            h.view3d_mut().view_of_mut(*id).unwrap().camera.yaw = c.yaw + yaw;
        }
        h.step();
    }
    assert_eq!(
        h.view3d_stats().unwrap().mesh_uploads,
        uploads,
        "形ごとの頂点を使い回す"
    );
    for (id, c) in [ViewId::FIRST, SECOND].iter().zip(cameras) {
        h.view3d_mut().view_of_mut(*id).unwrap().camera = c;
    }
    h.run();
    // 隠した面は当たらない（押しても選べない・ポインタの下に面が無い）
    click(&mut h, at);
    assert!(h.view3d().view.hide.selected.is_empty());
    {
        let rect = rect_of_view(&h, SECOND);
        let app = h.app();
        let view = app.view3d().view.camera.view(rect.width(), rect.height());
        let local = yolu_core::glam::Vec2::new(at.x - rect.left(), at.y - rect.top());
        let hit =
            yolu_core::geometry::pick(&app.view3d().model.as_ref().unwrap().geometry, &view, local);
        assert!(hit.is_none_or(|h| !app
            .view3d()
            .is_face_hidden(app.view3d().full_triangle(h.triangle).unwrap())));
    }
    // ペイントへ戻って隠した面の所を描いても、そのビューでは描けない
    enter(&mut h, EditorMode::Paint);
    let undo = h.doc().undo_count();
    hover_at(&mut h, at);
    drag(&mut h, &[at, at + vec2(4.0, 2.0), at + vec2(8.0, 4.0)]);
    assert_eq!(h.doc().undo_count(), undo, "隠した面には描かない");
    // 3D ビュー 1 では同じ面に描ける
    let first = island_point(&h, ViewId::FIRST, island).expect("3D ビュー 1 でも向いている");
    hover_at(&mut h, first);
    drag(
        &mut h,
        &[first, first + vec2(4.0, 2.0), first + vec2(8.0, 4.0)],
    );
    assert!(h.doc().undo_count() > undo, "3D ビュー 1 では描ける");
    // Alt+H で戻す（3D ビュー 2）
    enter(&mut h, EditorMode::Hide);
    let at = rect_of_view(&h, SECOND).center();
    hover_at(&mut h, at);
    key(&h, Key::H, Modifiers::ALT);
    h.run();
    assert_eq!(shown_in(&h, SECOND), 12);
    // 引き出しの「隠している物」に UV アイランドの行が出る
    let (island, at) = facing_island(&mut h, SECOND);
    hover_at(&mut h, at);
    click(&mut h, at);
    key(&h, Key::H, Modifiers::NONE);
    h.run();
    let name = format!("UV アイランド {}", island + 1);
    assert_eq!(
        yolu_app::view3d::hide::entries(h.app())
            .iter()
            .map(|e| e.name.clone())
            .collect::<Vec<_>>(),
        vec![name]
    );
    if !h.view3d().view_of(SECOND).unwrap().hide.drawer_open {
        h.act(Action::Hide(HideAction::ToggleDrawer(SECOND)));
        h.run();
    }
    assert!(h.query_by_label("この項目を外す").is_some());
}

#[test]
fn the_rectangle_tool_selects_the_islands_inside_and_ctrl_a_ctrl_d_select_all_and_none() {
    let mut h = cube();
    enter(&mut h, EditorMode::Hide);
    h.act(Action::Hide(HideAction::Tool(
        yolu_app::view3d::hide::HideTool::Rectangle,
    )));
    let rect = rect_of_view(&h, ViewId::FIRST);
    // 立方体の全部を囲む（引き出しの外から）
    let a = pos2(rect.left() + 20.0, rect.top() + 60.0);
    let b = pos2(rect.left() + rect.width() * 0.55, rect.bottom() - 60.0);
    hover_at(&mut h, a);
    drag(&mut h, &[a, a.lerp(b, 0.5), b]);
    let picked = h.view3d().view.hide.selected.clone();
    assert!(
        (1..=3).contains(&picked.len()),
        "見えている面（裏の面は除く）: {picked:?}"
    );
    key(&h, Key::D, Modifiers::COMMAND);
    h.run();
    assert!(h.view3d().view.hide.selected.is_empty());
    key(&h, Key::A, Modifiers::COMMAND);
    h.run();
    assert_eq!(h.view3d().view.hide.selected.len(), 6);
    // 全部を隠すのは断る
    key(&h, Key::H, Modifiers::NONE);
    h.run();
    assert_eq!(shown(&h), 12);
    assert!(h.message().contains("すべての面"), "{}", h.message());
}

// ───────── 2 つのビュー・シェルフ ─────────

#[test]
fn the_shelf_eye_and_the_pose_preset_are_per_view_and_the_same_pose_shares_one_shape() {
    let mut h = figure();
    add_second(&mut h);
    // ポーズのプリセットを 1 つ作る（今のポーズを曲げて保存し、今のポーズは戻す）
    {
        let s = h.view3d().pose.session.as_ref().unwrap();
        let bone = s
            .rig
            .bones()
            .iter()
            .position(|b| b.name == "右前腕")
            .unwrap();
        let mut pose = s.pose().clone();
        pose.locals[bone].rotation = yolu_core::glam::Quat::from_rotation_z(1.2);
        yolu_app::view3d::pose::set_pose(h.view3d_mut(), pose).unwrap();
        assert!(yolu_app::view3d::pose::presets::save_preset(h.app_mut(), "曲げ").is_some());
        yolu_app::view3d::pose::reset(h.view3d_mut()).unwrap();
    }
    h.run();
    let first = rect_of_view(&h, ViewId::FIRST);
    // 3D ビュー 1 のシェルフ（左下）を開いて、ポーズのプリセットを選ぶ
    let arrow = h
        .query_all_by_label("プリセットを選ぶ")
        .map(|n| n.rect())
        .find(|r| first.contains(r.center()))
        .expect("3D ビュー 1 のシェルフ");
    hover_at(&mut h, arrow.center());
    click(&mut h, arrow.center());
    assert!(h.view3d().view_of(ViewId::FIRST).unwrap().hide.shelf_open);
    let pose_box = rect_of(&h, "今のポーズ", |r| first.contains(r.center()));
    click(&mut h, pose_box.center());
    let at = popup_item(&h, "曲げ").center();
    click(&mut h, at);
    assert!(matches!(
        h.view3d().view_of(ViewId::FIRST).unwrap().pose,
        ViewPose::Preset(..)
    ));
    assert_eq!(h.view3d().view_of(SECOND).unwrap().pose, ViewPose::Current);
    let a = h.view3d().view_model(ViewId::FIRST).unwrap().clone();
    let b = h.view3d().view_model(SECOND).unwrap().clone();
    assert!(
        !std::sync::Arc::ptr_eq(&a, &b),
        "3D ビュー 1 だけポーズが替わる"
    );
    // 3D ビュー 2 も同じプリセットにすると、同じ形を共有する（モデル 1 体分）
    h.act(Action::Hide(HideAction::ChoosePose(
        SECOND,
        PoseChoice::Preset(match h.view3d().view_of(ViewId::FIRST).unwrap().pose {
            ViewPose::Preset(id, _) => id,
            _ => unreachable!(),
        }),
    )));
    h.run();
    assert!(std::sync::Arc::ptr_eq(
        h.view3d().view_model(ViewId::FIRST).unwrap(),
        h.view3d().view_model(SECOND).unwrap()
    ));
    // シェルフの目: 3D ビュー 1 で隠した物を、3D ビュー 1 だけで当てる・外す
    let total = h.view3d().full_model().unwrap().triangle_count();
    hover_at(&mut h, first.center());
    let bone = h
        .view3d()
        .pose
        .session
        .as_ref()
        .unwrap()
        .rig
        .bones()
        .iter()
        .position(|b| b.name == "頭")
        .unwrap();
    yolu_app::view3d::pose::hide::hide_bone(h.app_mut(), bone);
    h.run();
    assert!(shown_in(&h, ViewId::FIRST) < total);
    assert_eq!(shown_in(&h, SECOND), total, "3D ビュー 2 は何も隠さない");
    let eye = h
        .query_all_by_label("このビューで隠す")
        .map(|n| n.rect())
        .find(|r| first.contains(r.center()))
        .unwrap();
    hover_at(&mut h, eye.center());
    click(&mut h, eye.center());
    assert!(!h.view3d().view_of(ViewId::FIRST).unwrap().hide.applied);
    assert_eq!(shown_in(&h, ViewId::FIRST), total, "目を閉じたら隠さない");
    click(&mut h, eye.center());
    assert!(shown_in(&h, ViewId::FIRST) < total);
}

// ───────── ボーンの丸 ─────────

#[test]
fn bone_marks_are_3_to_9_points_with_the_zoom_and_take_the_press_before_the_face_in_pose_mode() {
    let mut h = figure();
    enter(&mut h, EditorMode::Pose);
    let rect = rect_of_view(&h, ViewId::FIRST);
    let marks = bone_marks::marks(h.app(), rect);
    assert_eq!(
        marks.len(),
        h.view3d().pose.session.as_ref().unwrap().rig.bones().len()
    );
    for m in &marks {
        assert!((3.0..=9.0).contains(&m.radius), "{m:?}");
    }
    // 引くと下限、寄ると上限
    let camera = h.view3d().view.camera;
    h.view3d_mut().view.camera.distance = camera.distance * 20.0;
    assert!(bone_marks::marks(h.app(), rect)
        .iter()
        .all(|m| m.radius == bone_marks::MIN_RADIUS));
    h.view3d_mut().view.camera.distance = camera.distance * 0.05;
    assert!(bone_marks::marks(h.app(), rect)
        .iter()
        .any(|m| m.radius == bone_marks::MAX_RADIUS));
    h.view3d_mut().view.camera = camera;
    h.run();
    // 面の上の丸のうち、その点の面からは別のボーンが選ばれる丸を押すと、丸のボーン（丸が先）。掴める範囲は見た目 + 3 点
    let marks = bone_marks::marks(h.app(), rect);
    let target = marks
        .iter()
        .filter(|m| {
            marks
                .iter()
                .all(|o| o.bone == m.bone || (o.at - m.at).length() > o.radius + m.radius + 8.0)
        })
        .find(|m| {
            let near = m.at + vec2(m.radius + 2.5, 0.0);
            rect.shrink2(vec2(rect.width() * 0.3, rect.height() * 0.2))
                .contains(m.at)
                && yolu_app::view3d::gizmo::bone_at(h.app(), rect, near)
                    .is_some_and(|b| b != m.bone)
        })
        .copied()
        .expect("面からは別のボーンが選ばれる、ほかの丸から離れた丸");
    let near = target.at + vec2(target.radius + 2.5, 0.0);
    let face_bone = yolu_app::view3d::gizmo::bone_at(h.app(), rect, near);
    hover_at(&mut h, near);
    click(&mut h, near);
    assert_eq!(
        h.view3d().pose.session.as_ref().unwrap().selected,
        Some(target.bone),
        "丸の掴める範囲（面のボーン {face_bone:?} より先）"
    );
    // 丸の外（面の上）は今どおり面からボーンを選ぶ
    let far = target.at + vec2(target.radius + bone_marks::GRAB_MARGIN + 3.0, 0.0);
    assert_eq!(bone_marks::bone_at(h.app(), rect, far), None);
    // ハイドのモードでも丸でボーンを選べる（ボーンで隠すに使う。引き出しは閉じて、丸を覆わないように）
    enter(&mut h, EditorMode::Hide);
    h.act(Action::Hide(HideAction::ToggleDrawer(ViewId::FIRST)));
    h.view3d_mut().pose.session.as_mut().unwrap().selected = None;
    hover_at(&mut h, target.at);
    click(&mut h, target.at);
    assert_eq!(
        h.view3d().pose.session.as_ref().unwrap().selected,
        Some(target.bone)
    );
    assert!(
        h.view3d().view.hide.selected.is_empty(),
        "丸の上はアイランドを選ばない"
    );
}

// ───────── スクリーンショット ─────────

/// ハイドのモードの 3D ビュー（引き出しを開いた・閉じた形、シェルフを開いた形）とボーンの丸（ポーズのモード）、2 つのビュー。日英。
#[test]
fn snapshot_hide_mode() {
    for (lang, tail) in [(Lang::Ja, ""), (Lang::En, "_en")] {
        let mut h = figure();
        h.set_lang(lang);
        h.run();
        enter(&mut h, EditorMode::Hide);
        let rect = rect_of_view(&h, ViewId::FIRST);
        // 頭をボーンで隠し、胴の UV アイランドを 1 つ選んだ形
        let bone = h
            .view3d()
            .pose
            .session
            .as_ref()
            .unwrap()
            .rig
            .bones()
            .iter()
            .position(|b| b.name == "頭")
            .unwrap();
        h.view3d_mut().pose.session.as_mut().unwrap().selected = Some(bone);
        h.act(Action::Hide(HideAction::HideBone));
        h.run();
        // 胴の上（背骨の丸の右、丸の掴める範囲の外）を押す
        let spine = h
            .view3d()
            .pose
            .session
            .as_ref()
            .unwrap()
            .rig
            .bones()
            .iter()
            .position(|b| b.name == "背骨")
            .unwrap();
        let mark = bone_marks::marks(h.app(), rect)
            .into_iter()
            .find(|m| m.bone == spine)
            .unwrap();
        let at = mark.at + vec2(mark.radius + bone_marks::GRAB_MARGIN + 4.0, 0.0);
        hover_at(&mut h, at);
        click(&mut h, at);
        assert_eq!(h.view3d().view.hide.selected.len(), 1, "胴を選んだ");
        let area = Rect::from_min_max(pos2(0.0, 0.0), rect.max);
        common::shot::clean_region(&mut h, &format!("hide_mode_drawer_open{tail}"), area);
        h.act(Action::Hide(HideAction::ToggleDrawer(ViewId::FIRST)));
        h.act(Action::Hide(HideAction::ToggleShelf(ViewId::FIRST)));
        h.run();
        common::shot::clean_region(&mut h, &format!("hide_mode_drawer_closed{tail}"), rect);
        enter(&mut h, EditorMode::Pose);
        common::shot::clean_region(&mut h, &format!("hide_bone_marks_pose{tail}"), rect);
    }
}

#[test]
fn snapshot_hide_mode_two_views() {
    for (lang, tail) in [(Lang::Ja, ""), (Lang::En, "_en")] {
        let mut h = cube();
        h.set_lang(lang);
        add_second(&mut h);
        enter(&mut h, EditorMode::Hide);
        let (_, at) = facing_island(&mut h, SECOND);
        hover_at(&mut h, at);
        click(&mut h, at);
        key(&h, Key::H, Modifiers::NONE);
        h.act(Action::Hide(HideAction::ToggleShelf(ViewId::FIRST)));
        let area = rect_of_view(&h, ViewId::FIRST).union(rect_of_view(&h, SECOND));
        common::shot::clean_region(&mut h, &format!("hide_mode_two_views{tail}"), area);
    }
}

// ───────── 計測（cargo test -p yolu-app --test gui_view3d -- --ignored --nocapture hide::measure） ─────────

/// 2 つのビューで違う隠し方・違うポーズのときの 1 フレームの時間と、GPU に持つ形の数（1 つのビュー・同じ形の 2 つのビューと比べる）。
#[test]
#[ignore = "計測"]
fn measure_two_views_with_different_hiding_and_poses() {
    use std::time::Instant;
    let frames = 120;
    let run = |case: usize| {
        let mut h = app_default(1600.0, 900.0, 2048);
        h.act(Action::Pose(PoseAction::LoadFigure));
        h.run();
        if case > 0 {
            add_second(&mut h);
        }
        if case == 2 {
            // 3D ビュー 2: 頭を隠し、初期ポーズ（今のポーズを曲げておく）
            let s = h.view3d().pose.session.as_ref().unwrap();
            let arm = s
                .rig
                .bones()
                .iter()
                .position(|b| b.name == "右前腕")
                .unwrap();
            let head = s.rig.bones().iter().position(|b| b.name == "頭").unwrap();
            let mut pose = s.pose().clone();
            pose.locals[arm].rotation = yolu_core::glam::Quat::from_rotation_z(1.0);
            yolu_app::view3d::pose::set_pose(h.view3d_mut(), pose).unwrap();
            h.view3d_mut().activate(SECOND);
            yolu_app::view3d::pose::hide::hide_bone(h.app_mut(), head);
            h.act(Action::Hide(HideAction::ChoosePose(
                SECOND,
                PoseChoice::Rest,
            )));
            h.view3d_mut().activate(ViewId::FIRST);
        }
        h.run();
        let adapter = h.view3d_adapter().unwrap_or_default();
        let before = h.view3d_stats().unwrap();
        let mut times = Vec::new();
        for i in 0..frames {
            for id in h.view3d().view_ids() {
                h.view3d_mut().view_of_mut(id).unwrap().camera.yaw = i as f32;
            }
            let started = Instant::now();
            h.step();
            h.view3d_wait_gpu();
            times.push(started.elapsed().as_micros() as u64);
        }
        times.sort_unstable();
        let mean = times.iter().sum::<u64>() as f64 / times.len() as f64 / 1000.0;
        let p95 = times[times.len() * 95 / 100] as f64 / 1000.0;
        let (models, _) = h.view3d().frame_models();
        let triangles: usize = models.iter().map(|m| m.triangle_count()).sum();
        let after = h.view3d_stats().unwrap();
        println!(
            "  頂点の上げ {}・3D を描いた回数 {}・影 {}・環境 {}",
            after.mesh_uploads - before.mesh_uploads,
            after.renders - before.renders,
            after.shadow_renders - before.shadow_renders,
            after.env_bakes - before.env_bakes
        );
        (adapter, mean, p95, models.len(), triangles)
    };
    for (case, name) in [
        (0, "ビュー 1"),
        (1, "ビュー 2・同じ形"),
        (2, "ビュー 2・違う隠し方とポーズ"),
    ] {
        let (adapter, mean, p95, shapes, triangles) = run(case);
        println!(
            "GPU: {adapter}、{name}: 1 フレーム 平均 {mean:.2} ms・95% {p95:.2} ms（{frames} フレーム）、見せる形 {shapes} 組・三角形の合計 {triangles}"
        );
    }
}
