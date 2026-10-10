//! ポーズの欄（ドックのタブ「ポーズ」）の試験（egui_kittest。描画は wgpu のソフトの描画）: タブの置き場・別のウィンドウ・節の開閉・日英
//! （英語に日本語が残らない・文字が切れない・「…」に詰められない）、ボーンのインスペクター（数値・項目ごと・ボーン・ボーンと子・全部の戻し・
//! 取り消しの段・描いている間は断る）、BlendShape の戻し、ボーンの影響で面を隠す（ボタン・描かれず当たらない・項目の外し・保存したプリセットを
//! 入れる/外す・消す・合わないボーンの理由）。試験のモデルは試しの人形（ユーザーの FBX は使わない）。
use crate::common;

use common::*;
use egui::accesskit::Role;
use egui::{epaint::Shape, pos2, vec2, Event, Key, Modifiers, Pos2, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, SnapshotResults};
use yolu_app::lang::Lang;
use yolu_app::pen::PenInput;
use yolu_app::state::{Action, AppState};
use yolu_app::ui::widgets;
use yolu_app::view3d::pose::edit;
use yolu_app::view3d::pose::hide::store::PresetEntry;
use yolu_app::view3d::pose::hide::{self, DEFAULT_THRESHOLD};
use yolu_app::view3d::pose::presets;
use yolu_app::view3d::pose::{self, PoseAction};
use yolu_app::{Tab, YoluApp};
use yolu_core::glam::{Quat, Vec3};

type H = Harness<'static, YoluApp>;

/// 試しの人形を読んだウィンドウ（ポーズのタブはドックの隅のまま）。
fn figure(doc: u32) -> H {
    let mut h = app(1280.0, 860.0, doc);
    h.state_mut().apply(Action::Pose(PoseAction::LoadFigure));
    h.run();
    h
}

/// ポーズのタブをドックから外して、左の上に浮いたウィンドウにする（egui_dock は外したウィンドウの大きさを渡した矩形の 0.8 倍にする。ドックの隅の狭い場所では節がスクロールになるので、欄が広く見えるように）。
fn float_pose_tab(h: &mut H) {
    {
        let dock = &mut h.state_mut().dock;
        let path = dock.find_tab(&Tab::Pose).expect("ポーズのタブ");
        dock.detach_tab(
            path,
            Rect::from_min_size(pos2(8.0, 40.0), vec2(400.0, 980.0)),
        );
    }
    h.run();
}

fn session(h: &H) -> &pose::PoseSession {
    h.state().state.view3d.pose.session.as_ref().unwrap()
}

fn bone(h: &H, name: &str) -> usize {
    session(h)
        .rig
        .bones()
        .iter()
        .position(|b| b.name == name)
        .unwrap()
}

/// 木で選んだことにする（ボーンの欄が出る）。
fn select(h: &mut H, name: &str) -> usize {
    let b = bone(h, name);
    {
        let s = h.state_mut().state.view3d.pose.session.as_mut().unwrap();
        s.selected = Some(b);
        yolu_app::view3d::gizmo::reveal(s, b);
    }
    h.run();
    b
}

fn local(h: &H, b: usize) -> yolu_core::skin::BoneTransform {
    session(h).pose().locals[b]
}

fn rest(h: &H, b: usize) -> yolu_core::skin::BoneTransform {
    session(h).rig.bones()[b].rest
}

fn undo_len(h: &H) -> usize {
    session(h).undo_len()
}

fn full(h: &H) -> usize {
    h.state()
        .state
        .view3d
        .full_model()
        .unwrap()
        .triangle_count()
}

fn shown(h: &H) -> usize {
    h.state()
        .state
        .view3d
        .model
        .as_ref()
        .unwrap()
        .triangle_count()
}

fn disabled(h: &H, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

fn center_of(h: &H, label: &str) -> Pos2 {
    h.get_by_label(label).rect().center()
}

/// 数の欄を押して打つ欄にし、全部選んで書き換えて Enter。
fn type_over(h: &mut H, at: Pos2, text: &str) {
    click(h, at);
    key(h, Key::A, Modifiers::COMMAND);
    h.step();
    h.event(Event::Text(text.to_owned()));
    h.step();
    key(h, Key::Enter, Modifiers::NONE);
    h.run();
}

const POSITION_X: &str = "X: 位置（親からの相対）";
const ROTATION_X: &str = "X: 回転（度。Unity のインスペクターと同じ並び）";
const ROTATION_Z: &str = "Z: 回転（度。Unity のインスペクターと同じ並び）";
const SCALE_Y: &str = "Y: 大きさ（親からの相対）";

// ───────── タブ ─────────

#[test]
fn the_pose_tab_joins_properties_when_a_skinned_model_loads_and_stays_after_the_model_goes() {
    let mut h = app(1280.0, 860.0, 64);
    assert!(
        h.state().dock.find_tab(&Tab::Pose).is_none(),
        "モデルが無いあいだは、ドックにポーズのタブを出さない"
    );
    assert!(!h.state().tab_rects.contains_key(&Tab::Pose));
    h.state_mut().apply(Action::Pose(PoseAction::LoadFigure));
    h.run();
    {
        let dock = &h.state().dock;
        let pose = dock.find_tab(&Tab::Pose).expect("ポーズのタブが足された");
        let layers = dock.find_tab(&Tab::Layers).unwrap();
        assert_eq!(
            (pose.surface, pose.node),
            (layers.surface, layers.node),
            "右のドックのレイヤーと同じ組"
        );
        // レイヤーは見えたまま（足したタブを前へ出さない）
        let leaf = dock.leaf(layers.node_path()).unwrap();
        assert_eq!(leaf.tabs[leaf.active.0], Tab::Layers);
    }
    assert!(h.state().tab_rects.contains_key(&Tab::Pose));
    assert!(
        h.query_by_label("ボーン").is_none(),
        "前に出ていないので欄は描かれない"
    );
    click_tab(&mut h, Tab::Pose);
    assert!(h.query_by_label("ボーン").is_some());
    // 別のモデルに替わって欄が空になっても、タブは残り、頭のボタンだけが出る（節も案内の文も出さない）
    h.state_mut().apply(Action::LoadDemoModel);
    h.run();
    h.run();
    assert!(h.state().dock.find_tab(&Tab::Pose).is_some());
    for label in ["取り消し", "やり直し", "FBX を開く"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    for label in ["ボーン", "面を隠す", "BlendShape"] {
        assert!(h.query_by_label(label).is_none(), "{label}");
    }
    assert!(disabled(&h, "取り消し") && disabled(&h, "やり直し"));
    assert!(!disabled(&h, "FBX を開く"));
    // 英語
    h.state_mut().state.lang = Lang::En;
    h.run();
    assert_eq!(Tab::Pose.title_in(Lang::En), "Pose");
    assert_eq!(Tab::Pose.title_in(Lang::Ja), "ポーズ");
    assert!(h.query_by_label("Open FBX").is_some());
    assert!(h.query_by_label("FBX を開く").is_none());
    // もう一度読むと、タブは 1 つのまま（足し直さない）
    h.state_mut().apply(Action::Pose(PoseAction::LoadFigure));
    h.run();
    let tabs: usize = h
        .state()
        .dock
        .iter_all_tabs()
        .filter(|(_, tab)| **tab == Tab::Pose)
        .count();
    assert_eq!(tabs, 1);
}

#[test]
fn the_pose_tab_is_added_back_after_the_layout_is_reset() {
    let mut h = figure(64);
    assert!(h.state().dock.find_tab(&Tab::Pose).is_some());
    // 浮かせたあと、配置を初めに戻すと、ポーズのタブがドックの隅へ戻る（足し直される）
    float_pose_tab(&mut h);
    h.state_mut().state.reset_layout = true;
    h.run();
    h.run();
    let dock = &h.state().dock;
    let pose = dock.find_tab(&Tab::Pose).expect("足し直された");
    assert_eq!(pose.surface, egui_dock::SurfaceIndex::main());
    let layers = dock.find_tab(&Tab::Layers).unwrap();
    assert_eq!((pose.surface, pose.node), (layers.surface, layers.node));
}

#[test]
fn the_pose_tab_floats_in_its_own_window_and_keeps_working() {
    let mut h = figure(256);
    float_pose_tab(&mut h);
    assert!(
        h.state().dock.find_tab(&Tab::Pose).is_none() && h.state().detached.contains(Tab::Pose),
        "ドックから外れて別のウィンドウにいる"
    );
    // ウィンドウの中で、木も節も使える
    for label in ["ボーン", "腰", "背骨", "面を隠す", "BlendShape", "おなか"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    let hand = bone(&h, "右手");
    {
        let s = h.state_mut().state.view3d.pose.session.as_mut().unwrap();
        s.selected = Some(hand);
        yolu_app::view3d::gizmo::reveal(s, hand);
    }
    h.run();
    assert!(
        h.query_by_label("右手").is_some(),
        "選んだボーンまで木が開く"
    );
    assert!(
        h.query_by_label(ROTATION_X).is_some(),
        "インスペクターが出る"
    );
    // 3D ビューは全幅のまま（欄に削られない）
    assert!(h.state().view3d_rect().unwrap().width() > 500.0);
}

#[test]
fn sections_open_and_close_and_the_choice_is_remembered() {
    let mut h = figure(256);
    float_pose_tab(&mut h);
    assert!(h.query_by_label("腰").is_some());
    // ボーンの節を閉じる: 木が消える
    h.get_by_label("ボーン").click();
    h.run();
    assert_eq!(h.state().state.ui.sections.get("pose.bones"), Some(&false));
    assert!(h.query_by_label("腰").is_none(), "閉じたら木は出ない");
    assert!(h.query_by_label("面を隠す").is_some(), "ほかの節はそのまま");
    h.get_by_label("ボーン").click();
    h.run();
    assert!(h.query_by_label("腰").is_some());
    // BlendShape の節
    assert!(h.query_by_label("おなか").is_some());
    h.get_by_label("BlendShape").click();
    h.run();
    assert!(h.query_by_label("おなか").is_none());
    assert_eq!(h.state().state.ui.sections.get("pose.shapes"), Some(&false));
    // 面を隠すの節
    assert!(h.query_by_label("選んだボーンの面を隠す").is_some());
    h.get_by_label("面を隠す").click();
    h.run();
    assert!(h.query_by_label("選んだボーンの面を隠す").is_none());
    // 開き直すと、閉じたままの節は閉じたまま（タブを裏へ回しても）
    click_tab(&mut h, Tab::Canvas);
    click_tab(&mut h, Tab::Pose);
    assert!(h.query_by_label("おなか").is_none());
    assert!(h.query_by_label("腰").is_some());
}

// ───────── インスペクター ─────────

#[test]
fn the_inspector_edits_a_bone_by_numbers_and_each_change_is_one_pose_undo_step() {
    let mut h = figure(256);
    h.state_mut().state.ui.sections.insert("pose.hide", false);
    float_pose_tab(&mut h);
    let upper = select(&mut h, "右上腕");
    let steps = undo_len(&h);
    // ボタンはまだ戻す物が無いので押せない
    for label in [
        "位置を戻す",
        "回転を戻す",
        "大きさを戻す",
        "ボーンを戻す",
        "ボーンと子を戻す",
    ] {
        assert!(disabled(&h, label), "{label}");
    }
    // 回転の Z を打つ（Unity と同じ並びの度）
    let at = center_of(&h, ROTATION_Z);
    type_over(&mut h, at, "30");
    assert_eq!(undo_len(&h), steps + 1);
    let degrees = edit::rotation_to_degrees(local(&h, upper).rotation);
    assert!(
        (degrees[2] - 30.0).abs() < 1e-3 && degrees[0].abs() < 1e-3,
        "{degrees:?}"
    );
    assert!(!disabled(&h, "回転を戻す"));
    assert!(disabled(&h, "位置を戻す"), "位置は動いていない");
    // X が 90° を超える角を打っても、打った値のまま欄に出る
    let at = center_of(&h, ROTATION_X);
    type_over(&mut h, at, "120");
    assert_eq!(undo_len(&h), steps + 2);
    assert_eq!(edit::euler_degrees(session(&h), upper)[0], 120.0);
    // 位置の X をドラッグ（1 画素 0.001）: ドラッグ 1 回で 1 段
    let before = local(&h, upper).translation;
    let c = center_of(&h, POSITION_X);
    drag(&mut h, &[c, c + vec2(10.0, 0.0), c + vec2(40.0, 0.0)]);
    assert_eq!(undo_len(&h), steps + 3, "ドラッグ 1 回で取り消し 1 つ");
    let moved = local(&h, upper).translation - before;
    assert!(
        (moved.x - 0.04).abs() < 1e-4 && moved.y == 0.0 && moved.z == 0.0,
        "{moved}"
    );
    // 大きさの Y を打つ
    let at = center_of(&h, SCALE_Y);
    type_over(&mut h, at, "1.5");
    assert_eq!(local(&h, upper).scale, Vec3::new(1.0, 1.5, 1.0));
    assert_eq!(undo_len(&h), steps + 4);
    // 同じ値を打っても積まない
    let at = center_of(&h, SCALE_Y);
    type_over(&mut h, at, "1.5");
    assert_eq!(undo_len(&h), steps + 4);
    // 範囲の外は端へ
    let at = center_of(&h, SCALE_Y);
    type_over(&mut h, at, "99999");
    assert_eq!(local(&h, upper).scale.y, 1000.0);
    h.state_mut().apply(Action::Pose(PoseAction::Undo));
    h.run();
    assert_eq!(local(&h, upper).scale.y, 1.5, "取り消すと 1 つ前");
}

#[test]
fn dragging_a_field_and_pressing_escape_puts_the_value_back_without_an_undo_step() {
    let mut h = figure(256);
    h.state_mut().state.ui.sections.insert("pose.hide", false);
    float_pose_tab(&mut h);
    let upper = select(&mut h, "右上腕");
    let before = local(&h, upper);
    let c = center_of(&h, POSITION_X);
    press(&h, c, egui::PointerButton::Primary);
    h.step();
    for dx in [10.0, 30.0] {
        move_to(&h, c + vec2(dx, 0.0));
        h.step();
    }
    assert!(
        local(&h, upper).translation.x > before.translation.x,
        "動かしている途中"
    );
    assert!(session(&h).is_editing(), "ドラッグの間は続けて変える操作");
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    release(&h, c + vec2(30.0, 0.0), egui::PointerButton::Primary);
    h.run();
    assert_eq!(local(&h, upper), before, "Esc で押し始めの値へ");
    assert_eq!(undo_len(&h), 0, "取り消しの段は積まない");
    assert!(!session(&h).is_editing());
}

#[test]
fn losing_focus_in_the_middle_of_a_field_drag_commits_what_was_dragged() {
    let mut h = figure(256);
    h.state_mut().state.ui.sections.insert("pose.hide", false);
    float_pose_tab(&mut h);
    let upper = select(&mut h, "右上腕");
    let before = local(&h, upper).translation;
    let c = center_of(&h, POSITION_X);
    press(&h, c, egui::PointerButton::Primary);
    h.step();
    for dx in [10.0, 30.0] {
        move_to(&h, c + vec2(dx, 0.0));
        h.step();
    }
    assert!(session(&h).is_editing());
    h.event(Event::WindowFocused(false));
    h.run();
    assert!(!session(&h).is_editing(), "フォーカスを失ったら確定する");
    assert_eq!(undo_len(&h), 1, "そこまでで取り消し 1 つ");
    assert!(local(&h, upper).translation.x > before.x);
}

#[test]
fn the_reset_buttons_restore_a_part_a_bone_its_children_and_everything_in_one_step_each() {
    let mut h = figure(256);
    h.state_mut().state.ui.sections.insert("pose.hide", false);
    float_pose_tab(&mut h);
    let (upper, lower, hand) = (bone(&h, "右上腕"), bone(&h, "右前腕"), bone(&h, "右手"));
    select(&mut h, "右上腕");
    let mut p = session(&h).pose().clone();
    for b in [upper, lower, hand] {
        p.locals[b].translation += Vec3::new(0.02, 0.0, 0.0);
        p.locals[b].rotation = Quat::from_rotation_z(0.5);
        p.locals[b].scale = Vec3::new(1.0, 1.2, 1.0);
    }
    pose::set_pose(&mut h.state_mut().state.view3d, p).unwrap();
    h.run();
    let steps = undo_len(&h);
    // 項目ごと（欄の横の小さなボタン）
    h.get_by_label("位置を戻す").click();
    h.run();
    assert_eq!(local(&h, upper).translation, rest(&h, upper).translation);
    assert_ne!(
        local(&h, upper).rotation,
        rest(&h, upper).rotation,
        "回転はそのまま"
    );
    assert_eq!(undo_len(&h), steps + 1);
    assert!(disabled(&h, "位置を戻す"), "もう戻す物が無い");
    h.get_by_label("回転を戻す").click();
    h.get_by_label("大きさを戻す").click();
    h.run();
    assert_eq!(local(&h, upper), rest(&h, upper));
    assert_eq!(undo_len(&h), steps + 3);
    assert!(disabled(&h, "ボーンを戻す"));
    assert_ne!(local(&h, lower), rest(&h, lower), "子は動いたまま");
    // このボーンと子
    assert!(!disabled(&h, "ボーンと子を戻す"));
    h.get_by_label("ボーンと子を戻す").click();
    h.run();
    for b in [upper, lower, hand] {
        assert_eq!(local(&h, b), rest(&h, b));
    }
    assert_eq!(undo_len(&h), steps + 4);
    // 取り消すと、3 つとも動いた状態へ（1 段）
    h.state_mut().apply(Action::Pose(PoseAction::Undo));
    h.run();
    assert_ne!(local(&h, lower), rest(&h, lower));
    assert_ne!(local(&h, hand), rest(&h, hand));
    // このボーンだけ
    h.get_by_label("ボーンを戻す").click();
    h.run();
    assert_ne!(
        local(&h, lower),
        rest(&h, lower),
        "このボーンの子は戻らない"
    );
    // 見出しの戻すボタン: すべてのボーン
    let all = "すべてのボーンを戻す（BlendShape はそのまま）";
    let mut p = session(&h).pose().clone();
    p.blend_weights[0][0] = 40.0;
    pose::set_pose(&mut h.state_mut().state.view3d, p).unwrap();
    h.run();
    h.get_by_label(all).click();
    h.run();
    assert_eq!(
        session(&h).pose().locals,
        session(&h).rig.rest_pose().locals
    );
    assert_eq!(
        session(&h).pose().blend_weights[0][0],
        40.0,
        "BlendShape はそのまま"
    );
}

#[test]
fn blend_shapes_reset_one_by_one_and_all_at_once() {
    let mut h = figure(256);
    h.state_mut().state.ui.sections.insert("pose.hide", false);
    h.state_mut().state.ui.sections.insert("pose.bones", false);
    float_pose_tab(&mut h);
    let mut p = session(&h).pose().clone();
    p.blend_weights[0][0] = 70.0;
    p.blend_weights[5][0] = 30.0;
    pose::set_pose(&mut h.state_mut().state.view3d, p).unwrap();
    h.run();
    let steps = undo_len(&h);
    let one = "この BlendShape を戻す";
    let buttons: Vec<Rect> = h.get_all_by_label(one).map(|n| n.rect()).collect();
    assert_eq!(buttons.len(), 2, "スライダーごと");
    click(&mut h, buttons[0].center());
    assert_eq!(session(&h).pose().blend_weights[0][0], 0.0);
    assert_eq!(session(&h).pose().blend_weights[5][0], 30.0);
    assert_eq!(undo_len(&h), steps + 1);
    assert!(
        h.get_all_by_label(one)
            .next()
            .unwrap()
            .accesskit_node()
            .is_disabled(),
        "戻した行はもう押せない"
    );
    // すべての BlendShape（見出しの戻すボタン。ボーンには触れない）
    let upper = bone(&h, "右上腕");
    let mut p = session(&h).pose().clone();
    p.locals[upper].rotation = Quat::from_rotation_z(0.4);
    pose::set_pose(&mut h.state_mut().state.view3d, p).unwrap();
    h.run();
    h.get_by_label("すべての BlendShape を戻す（ボーンはそのまま）")
        .click();
    h.run();
    assert_eq!(
        session(&h).pose().blend_weights,
        session(&h).rig.rest_pose().blend_weights
    );
    assert_ne!(local(&h, upper).rotation, rest(&h, upper).rotation);
    // 取り消すと、30 に戻る
    h.state_mut().apply(Action::Pose(PoseAction::Undo));
    h.run();
    assert_eq!(session(&h).pose().blend_weights[5][0], 30.0);
}

#[test]
fn nothing_in_the_pose_tab_changes_while_stroking() {
    let mut h = figure(256);
    h.state_mut().state.ui.sections.insert("pose.hide", false);
    float_pose_tab(&mut h);
    select(&mut h, "右上腕");
    // 胴に描き始める（本物のポインタで）
    let rect = h.state().view3d_rect().unwrap();
    let model = h.state().state.view3d.model.clone().unwrap();
    let cam = h.state().state.view3d.camera.position();
    let n = session(&h).rig.meshes()[0].mesh.triangle_count();
    let t = model.geometry.triangles()[..n]
        .iter()
        .find(|t| {
            let c = (t.a + t.b + t.c) / 3.0;
            t.normal().dot((cam - c).normalize()) > 0.8 && c.y > 1.1
        })
        .copied()
        .unwrap();
    let at = screen_of(&h, rect, (t.a + t.b + t.c) / 3.0);
    press(&h, at, egui::PointerButton::Primary);
    h.step();
    move_to(&h, at + vec2(6.0, 0.0));
    h.step();
    assert!(h.state().state.is_stroking());
    h.run();
    for label in [ROTATION_X, POSITION_X, SCALE_Y] {
        assert!(disabled(&h, label), "{label}: 描いている間は欄を使えない");
    }
    for label in [
        "位置を戻す",
        "ボーンを戻す",
        "ボーンと子を戻す",
        "FBX を開く",
    ] {
        assert!(disabled(&h, label), "{label}");
    }
    // 終わると使える
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    release(&h, at, egui::PointerButton::Primary);
    h.run();
    assert!(!h.state().state.is_stroking());
    assert!(!disabled(&h, ROTATION_X));
}

#[test]
fn a_hide_sets_delete_button_waits_for_the_stroke_and_the_file_stays() {
    let dir = temp_dir("delete-while-stroking");
    let mut h = figure(256);
    h.state_mut()
        .state
        .view3d
        .pose
        .hide_presets
        .attach(dir.clone());
    let id = h
        .state_mut()
        .state
        .view3d
        .pose
        .hide_presets
        .add(
            "頭だけ",
            vec![PresetEntry {
                path: ["腰", "背骨", "胸", "首", "頭"].map(String::from).to_vec(),
                threshold: 0.5,
                children: true,
            }],
        )
        .unwrap();
    float_pose_tab(&mut h);
    assert!(!disabled(&h, "この隠し方を消す"));
    // 描いている最中は押せず、ファイルも一覧もそのまま
    {
        let app = &mut h.state_mut().state;
        let layer = app.selected_layer.unwrap();
        let settings = app.stroke_settings(false);
        app.stroke = Some(app.doc.begin_stroke(layer, &settings).unwrap());
    }
    h.run();
    assert!(h.state().state.is_stroking());
    assert!(disabled(&h, "この隠し方を消す"));
    hide::delete_preset(&mut h.state_mut().state, id);
    assert!(h.state().state.view3d.pose.hide_presets.get(id).is_some());
    assert!(dir.join(format!("hide-{id}.ylhide")).exists());
    std::fs::remove_dir_all(dir).unwrap();
}

// ───────── 面を隠す ─────────

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

/// 右腕の筒（メッシュ 1）の、カメラに向いた三角形の真ん中の画面の点。
fn arm_point(h: &H, rect: Rect) -> Pos2 {
    let app = &h.state().state;
    let s = session(h);
    let model = app.view3d.model.as_ref().unwrap();
    let first = s.rig.meshes()[0].mesh.triangle_count();
    let count = s.rig.meshes()[1].mesh.triangle_count();
    let cam = app.view3d.camera.position();
    for i in first..first + count {
        let t = &model.geometry.triangles()[i];
        let c = (t.a + t.b + t.c) / 3.0;
        if c.x > 0.3 && c.x < 0.6 && t.normal().dot((cam - c).normalize()) > 0.6 {
            return screen_of(h, rect, c);
        }
    }
    panic!("カメラに向いた腕の三角形が無い");
}

#[test]
fn hiding_a_bones_surfaces_removes_them_from_the_view_and_from_painting() {
    let mut h = figure(512);
    h.state_mut().state.color.set_main([0.9, 0.1, 0.1, 1.0]);
    h.state_mut().state.brush.radius = 4.0;
    float_pose_tab(&mut h);
    let rect = h.state().view3d_rect().unwrap();
    let at = arm_point(&h, rect);
    // 隠す前は、腕に描ける
    click(&mut h, at);
    assert!(
        h.state().state.doc.can_undo(),
        "{}",
        h.state().state.message
    );
    h.state_mut().state.apply(Action::Undo);
    h.run();
    assert!(!h.state().state.doc.can_undo());
    let total = full(&h);
    // 右上腕を選んで、子を含めて隠す（既定: しきい値 50%・子を含める）
    select(&mut h, "右上腕");
    assert!(!disabled(&h, "選んだボーンの面を隠す"));
    h.get_by_label("選んだボーンの面を隠す").click();
    h.run();
    assert!(shown(&h) < total, "腕の面が消えた");
    assert_eq!(full(&h), total);
    let entries = &session(&h).hide.entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].threshold, DEFAULT_THRESHOLD);
    assert!(entries[0].children);
    // 項目の行（名前としきい値・子）と、外すボタン
    assert!(h.query_by_label("この項目を外す").is_some());
    // 隠した面は当たらない: 当たりの幾何にも、スポイトにも無く、そこをなぞっても描かれない
    {
        let app = &h.state().state;
        let view = app.view3d.camera.view(rect.width(), rect.height());
        let local = yolu_core::glam::Vec2::new(at.x - rect.left(), at.y - rect.top());
        let shown_hit =
            yolu_core::geometry::pick(&app.view3d.model.as_ref().unwrap().geometry, &view, local);
        let full_hit =
            yolu_core::geometry::pick(&app.view3d.full_model().unwrap().geometry, &view, local);
        assert!(shown_hit.is_none(), "見せる形に腕の面は無い: {shown_hit:?}");
        assert!(full_hit.is_some(), "受けたままの形には腕がある");
    }
    h.state_mut().state.message.clear();
    assert!(!yolu_app::eyedrop::pick_surface(
        &mut h.state_mut().state,
        rect,
        at
    ));
    assert_eq!(
        h.state().state.message,
        "ポインタの下にモデルがありません",
        "スポイトは隠した面を読まない"
    );
    click(&mut h, at);
    assert!(!h.state().state.doc.can_undo(), "隠した面には描かない");
    assert!(!h.state().state.is_stroking());
    // 外すと元へ（描ける）
    h.get_by_label("この項目を外す").click();
    h.run();
    assert_eq!(shown(&h), total);
    assert!(h.query_by_label("この項目を外す").is_none());
    click(&mut h, at);
    assert!(h.state().state.doc.can_undo(), "外したらまた描ける");
}

/// 人形のメッシュ（0 胴・1 右腕・2 左腕・3 右脚・4 左脚・5 頭）の、カメラに向いた三角形のうち真ん中が `keep` を満たす最初のものの、
/// 画面の点と受けたままの形の番号。
fn mesh_point(h: &H, rect: Rect, mesh: usize, keep: impl Fn(Vec3) -> bool) -> (Pos2, u32) {
    let app = &h.state().state;
    let s = session(h);
    let full = app.view3d.full_model().unwrap();
    let first: usize = s.rig.meshes()[..mesh]
        .iter()
        .map(|m| m.mesh.triangle_count())
        .sum();
    let count = s.rig.meshes()[mesh].mesh.triangle_count();
    let cam = app.view3d.camera.position();
    for i in first..first + count {
        let t = &full.geometry.triangles()[i];
        let c = (t.a + t.b + t.c) / 3.0;
        if keep(c) && t.normal().dot((cam - c).normalize()) > 0.8 {
            return (screen_of(h, rect, c), i as u32);
        }
    }
    panic!("カメラに向いた三角形が無い（メッシュ {mesh}）");
}

fn selected(h: &H) -> Option<usize> {
    session(h).selected
}

#[test]
fn pressing_a_hidden_bones_surface_does_not_pick_it_and_the_shifted_faces_pick_the_right_bone() {
    let mut h = figure(256);
    let rect = h.state().view3d_rect().unwrap();
    h.state_mut()
        .state
        .apply(Action::Pose(PoseAction::ToggleMode));
    h.run();
    let far = select(&mut h, "左足");
    let (arm, arm_face) = mesh_point(&h, rect, 1, |c| c.x > 0.25 && c.x < 0.4);
    let (left_arm, left_face) = mesh_point(&h, rect, 2, |c| c.x < -0.25 && c.x > -0.4);
    // 隠す前は、押した面の下のボーンが選ばれる
    click(&mut h, arm);
    assert_eq!(selected(&h), Some(bone(&h, "右上腕")));
    // 右腕を（子まで）隠す: そこの面は当たらないので、押してもボーンは替わらない
    select(&mut h, "左足");
    let upper = bone(&h, "右上腕");
    hide::hide_bone(&mut h.state_mut().state, upper);
    h.run();
    assert!(
        h.state().state.view3d.is_face_hidden(arm_face),
        "腕の面は隠れた"
    );
    assert_eq!(selected(&h), Some(far));
    click(&mut h, arm);
    assert_eq!(selected(&h), Some(far), "隠した面は選べない");
    // 右腕の分だけ番号がずれた左腕の面は、受けたままの形の番号へ直されて、左上腕が選ばれる
    let state = &h.state().state.view3d;
    let removed = full(&h) - shown(&h);
    assert!(removed > 0);
    let shifted = (0..shown(&h) as u32)
        .find(|i| state.full_triangle(*i) == Some(left_face))
        .expect("左腕の面は見せる形にある");
    assert_eq!(
        shifted,
        left_face - removed as u32,
        "見せる形の番号は、隠した面の分だけ受けたままの形とずれる"
    );
    click(&mut h, left_arm);
    assert_eq!(selected(&h), Some(bone(&h, "左上腕")));
}

#[test]
fn with_a_material_hidden_the_shown_face_numbers_are_mapped_back_before_picking_the_bone() {
    let mut h = figure(256);
    let rect = h.state().view3d_rect().unwrap();
    h.state_mut()
        .state
        .apply(Action::Pose(PoseAction::ToggleMode));
    h.run();
    let far = select(&mut h, "左足");
    let (arm, arm_face) = mesh_point(&h, rect, 1, |c| c.x > 0.25 && c.x < 0.4);
    let (head, head_face) = mesh_point(&h, rect, 5, |_| true);
    // 「肌」のマテリアル（胴・腕・脚）のテクスチャセットの目を閉じる: 顔の球だけが残る
    let skin = h
        .state()
        .state
        .sets
        .iter()
        .find(|s| s.bound == Some(0))
        .map(|s| s.uid)
        .expect("肌のマテリアルのセット");
    h.state_mut().state.toggle_set_visible(skin);
    h.run();
    let sphere = session(&h).rig.meshes()[5].mesh.triangle_count();
    assert_eq!(shown(&h), sphere, "顔の球だけが残る");
    let first_shown = h.state().state.view3d.full_triangle(0).unwrap();
    assert_eq!(
        first_shown as usize,
        full(&h) - sphere,
        "見せる形の 0 番は、受けたままの形の 0 番ではなく球の最初の面"
    );
    assert!(first_shown <= head_face);
    // 隠したマテリアルの腕の面は選べない
    click(&mut h, arm);
    assert_eq!(selected(&h), Some(far));
    assert!(
        !h.state().state.view3d.is_face_hidden(arm_face),
        "面の印ではなくマテリアルで隠している"
    );
    // 顔の球の面を押すと、番号を直して頭が選ばれる（直さなければ、番号の小さい胴のボーンになる）
    click(&mut h, head);
    assert_eq!(selected(&h), Some(bone(&h, "頭")));
    // 目を開けると、腕も選べる（頭の輪が腕の近くにあるので、遠いボーンを選び直してから押す）
    h.state_mut().state.toggle_set_visible(skin);
    h.run();
    assert_eq!(shown(&h), full(&h));
    select(&mut h, "左足");
    click(&mut h, arm);
    assert_eq!(selected(&h), Some(bone(&h, "右上腕")));
}

#[test]
fn children_and_the_threshold_are_taken_from_the_fields_for_the_next_entry() {
    let mut h = figure(256);
    float_pose_tab(&mut h);
    let total = full(&h);
    select(&mut h, "右前腕");
    // 子を含めない（チェックを外す）
    h.get_by_role_and_label(Role::CheckBox, "子を含める")
        .click();
    h.run();
    assert!(!session(&h).hide.children);
    h.get_by_label("選んだボーンの面を隠す").click();
    h.run();
    let alone = total - shown(&h);
    assert!(!session(&h).hide.entries[0].children);
    // 子を含める（同じボーンへ足し直すと入れ替わる）
    h.get_by_role_and_label(Role::CheckBox, "子を含める")
        .click();
    h.run();
    h.get_by_label("選んだボーンの面を隠す").click();
    h.run();
    assert_eq!(session(&h).hide.entries.len(), 1, "同じボーンは 1 項目");
    assert!(total - shown(&h) > alone);
    // しきい値の欄（スライダー）の値は次の項目に使われる
    hide::set_next(&mut h.state_mut().state, 0.9, true);
    h.run();
    assert!(h.query_by_label("しきい値").is_some());
    assert!((session(&h).hide.threshold - 0.9).abs() < 1e-6);
    // 全部が隠れる隠し方は断る（根・子を含める・しきい値 0）
    select(&mut h, "腰");
    hide::set_next(&mut h.state_mut().state, 0.0, true);
    let before = shown(&h);
    h.get_by_label("選んだボーンの面を隠す").click();
    h.run();
    assert_eq!(shown(&h), before, "全部は隠さない");
    assert!(
        h.state().state.message.contains("すべての面"),
        "{}",
        h.state().state.message
    );
    assert_eq!(session(&h).hide.entries.len(), 1);
    // 見出しの「すべて表示する」
    h.get_by_label("すべて表示する").click();
    h.run();
    assert_eq!(shown(&h), total);
    assert!(session(&h).hide.is_clear());
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-pose-ui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    dir
}

#[test]
fn presets_save_apply_combine_and_delete_from_the_tab_and_survive_a_restart() {
    let dir = temp_dir("presets");
    let mut h = figure(256);
    h.state_mut()
        .state
        .view3d
        .pose
        .hide_presets
        .attach(dir.clone());
    float_pose_tab(&mut h);
    let total = full(&h);
    // 頭を隠して、名前をつけて保存
    select(&mut h, "頭");
    h.get_by_label("選んだボーンの面を隠す").click();
    h.run();
    let head_hidden = total - shown(&h);
    assert!(head_hidden > 0);
    assert!(!disabled(&h, "保存"));
    let save = h.get_by_label("保存").rect();
    type_over(&mut h, pos2(save.left() - 80.0, save.center().y), "頭だけ");
    assert_eq!(h.state().state.view3d.pose.hide_name, "頭だけ");
    h.get_by_label("保存").click();
    h.run();
    let presets = &h.state().state.view3d.pose.hide_presets;
    assert_eq!(presets.items().len(), 1);
    assert_eq!(presets.items()[0].name, "頭だけ");
    assert_eq!(
        presets.items()[0].entries[0]
            .path
            .last()
            .map(String::as_str),
        Some("頭")
    );
    assert!(
        h.state().state.view3d.pose.hide_name.is_empty(),
        "保存したら名前の欄は空へ"
    );
    assert!(
        std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().ends_with(".ylhide")),
        "個人の設定のフォルダに書いた"
    );
    // 手の項目を足して、もう 1 つ保存（名前が同じなら番号が付く）
    select(&mut h, "右手");
    h.get_by_label("選んだボーンの面を隠す").click();
    h.run();
    h.get_by_label("保存").click();
    h.run();
    let names: Vec<String> = h
        .state()
        .state
        .view3d
        .pose
        .hide_presets
        .items()
        .iter()
        .map(|p| p.name.clone())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert_eq!(names[1], "隠し方", "名前の欄が空なら既定の名前");
    // すべて表示 → プリセットを入れる
    h.get_by_label("すべて表示する").click();
    h.run();
    assert_eq!(shown(&h), total);
    let first = h
        .get_by_role_and_label(Role::CheckBox, "頭だけ")
        .rect()
        .center();
    click(&mut h, first);
    assert_eq!(
        total - shown(&h),
        head_hidden,
        "入れたプリセットの分だけ隠れる"
    );
    // もう 1 つ入れると和（頭 ∪ 頭と手）
    let second = h
        .get_by_role_and_label(Role::CheckBox, "隠し方")
        .rect()
        .center();
    click(&mut h, second);
    assert!(total - shown(&h) > head_hidden, "全部の和");
    assert_eq!(session(&h).hide.presets.len(), 2);
    // 外す
    click(&mut h, second);
    assert_eq!(total - shown(&h), head_hidden);
    // 再起動: 同じフォルダを読む別のアプリで、同じプリセット
    let mut again = YoluApp::for_context(
        &egui::Context::default(),
        AppState::new(64, 64),
        PenInput::detached(),
    );
    again.state.view3d.pose.hide_presets.attach(dir.clone());
    assert_eq!(
        again.state.view3d.pose.hide_presets.items(),
        h.state().state.view3d.pose.hide_presets.items()
    );
    // 消す（入れているものは外れる）
    let delete: Vec<Rect> = h
        .get_all_by_label("この隠し方を消す")
        .map(|n| n.rect())
        .collect();
    assert_eq!(delete.len(), 2);
    click(&mut h, delete[0].center());
    assert_eq!(h.state().state.view3d.pose.hide_presets.items().len(), 1);
    assert_eq!(
        shown(&h),
        total,
        "入れていたプリセットを消したら隠すのをやめる"
    );
    assert!(h
        .query_by_role_and_label(Role::CheckBox, "頭だけ")
        .is_none());
    assert_eq!(
        std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".ylhide"))
            .count(),
        1
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_preset_whose_bones_do_not_fit_is_skipped_with_a_reason_shown_as_a_notice() {
    let mut h = figure(256);
    float_pose_tab(&mut h);
    let total = full(&h);
    h.state_mut()
        .state
        .view3d
        .pose
        .hide_presets
        .add(
            "合わない",
            vec![
                PresetEntry {
                    path: ["腰", "背骨", "胸", "首", "頭"].map(String::from).to_vec(),
                    threshold: 0.5,
                    children: true,
                },
                PresetEntry {
                    path: ["別の根", "頭"].map(String::from).to_vec(),
                    threshold: 0.5,
                    children: true,
                },
            ],
        )
        .unwrap();
    h.run();
    let at = h
        .get_by_role_and_label(Role::CheckBox, "合わない")
        .rect()
        .center();
    click(&mut h, at);
    assert!(shown(&h) < total, "合う項目は効く");
    let skipped = &session(&h).hide.skipped;
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].path, "別の根/頭");
    assert!(skipped[0].describe(Lang::Ja).contains("ボーンがありません"));
    assert!(
        texts(&h).iter().any(|t| t == "知らせ 1 件"),
        "{:?}",
        texts(&h)
    );
}

// ───────── ポーズのプリセット ─────────

/// 右上腕を回して・動かし、頭を傾けたポーズにする（取り消しに 1 段）。
fn bend_arm(h: &mut H) {
    let mut p = session(h).pose().clone();
    let arm = bone(h, "右上腕");
    let head = bone(h, "頭");
    p.locals[arm].rotation = Quat::from_euler(yolu_core::glam::EulerRot::XYZ, 0.2, 0.4, -0.9);
    p.locals[arm].translation += Vec3::new(0.02, 0.01, -0.03);
    p.locals[head].rotation = Quat::from_rotation_z(0.3);
    pose::set_pose(&mut h.state_mut().state.view3d, p).unwrap();
    h.run();
}

fn near(a: &yolu_core::skin::BoneTransform, b: &yolu_core::skin::BoneTransform) -> bool {
    (a.translation - b.translation).length() < 1e-5
        && a.rotation.dot(b.rotation).abs() > 0.999_999
        && (a.scale - b.scale).abs().max_element() < 1e-5
}

fn pose_preset_files(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".ylpose"))
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn pose_presets_save_apply_mirror_overwrite_rename_and_delete_from_the_tab() {
    let dir = temp_dir("pose-presets");
    let mut h = figure(256);
    h.state_mut()
        .state
        .view3d
        .pose
        .pose_presets
        .attach(dir.clone());
    float_pose_tab(&mut h);
    let (arm, left_arm, head) = (bone(&h, "右上腕"), bone(&h, "左上腕"), bone(&h, "頭"));
    // 休みの形では保存できない（保存するものが無い）
    assert!(disabled(&h, "ポーズを保存"));
    bend_arm(&mut h);
    let bent = session(&h).pose().clone();
    assert!(!disabled(&h, "ポーズを保存"));
    // 名前を付けて保存
    let save = h.get_by_label("ポーズを保存").rect();
    type_over(&mut h, pos2(save.left() - 60.0, save.center().y), "構え");
    assert_eq!(h.state().state.view3d.pose.preset_name, "構え");
    h.get_by_label("ポーズを保存").click();
    h.run();
    let id = {
        let presets = &h.state().state.view3d.pose.pose_presets;
        assert_eq!(presets.items().len(), 1);
        assert_eq!(presets.items()[0].name, "構え");
        assert_eq!(presets.items()[0].entries.len(), 2, "動かした骨の 2 本だけ");
        presets.items()[0].id
    };
    assert_eq!(pose_preset_files(&dir), 1, "個人の設定のフォルダに書いた");
    assert!(
        h.state().state.view3d.pose.preset_name.is_empty(),
        "保存したら名前の欄は空へ"
    );
    // 名前の欄が空なら既定の名前
    h.get_by_label("ポーズを保存").click();
    h.run();
    assert_eq!(
        h.state().state.view3d.pose.pose_presets.items()[1].name,
        "ポーズ"
    );
    // 休みの形へ戻して、名前を押して当てる: 取り消しの 1 段
    h.get_by_label("ポーズを戻す（ファイルのポーズへ）").click();
    h.run();
    assert!(!session(&h).is_posed());
    // 休みの形では上書きも押せない（保存したポーズが項目 0 の「休みの形」に置き換わって戻せなくなる）
    let rest_overwrites: Vec<_> = h.get_all_by_label("今のポーズで上書き").collect();
    assert_eq!(rest_overwrites.len(), 2, "プリセットの数だけ並ぶ");
    assert!(rest_overwrites
        .iter()
        .all(|n| n.accesskit_node().is_disabled()));
    let at = rest_overwrites[0].rect().center();
    drop(rest_overwrites);
    click(&mut h, at);
    {
        let presets = &h.state().state.view3d.pose.pose_presets;
        assert_eq!(
            presets.items()[0].entries.len(),
            2,
            "押しても上書きされない"
        );
        assert_eq!(presets.items()[1].entries.len(), 2);
    }
    let steps = undo_len(&h);
    h.get_by_label("構え").click();
    h.run();
    assert_eq!(undo_len(&h), steps + 1, "当てるのは取り消しの 1 段");
    for (i, (a, b)) in session(&h)
        .pose()
        .locals
        .iter()
        .zip(&bent.locals)
        .enumerate()
    {
        assert!(near(a, b), "骨 {i}");
    }
    assert!(
        h.state().state.message.contains("構え"),
        "{}",
        h.state().state.message
    );
    h.get_by_label("取り消し").click();
    h.run();
    assert!(!session(&h).is_posed(), "取り消すと当てる前");
    // 左右を反転して当てる（行の右のアイコン。1 行目は「構え」）
    let mirror = h
        .get_all_by_label("左右を反転して当てる")
        .map(|n| n.rect())
        .next()
        .unwrap();
    click(&mut h, mirror.center());
    assert!(
        session(&h).pose().locals[left_arm] != session(&h).rig.bones()[left_arm].rest,
        "左腕が動く"
    );
    assert_eq!(
        session(&h).pose().locals[arm],
        session(&h).rig.bones()[arm].rest,
        "右腕は休みのまま"
    );
    assert!(
        near(&session(&h).pose().locals[head], &bent.locals[head]),
        "対にならない頭はそのまま"
    );
    assert!(
        h.state().state.message.contains("左右反転"),
        "{}",
        h.state().state.message
    );
    // 今のポーズで上書き（名前はそのまま・ほかは変わらない）。左右を反転して当てたあとは休みの形と違うので押せる
    assert!(h
        .get_all_by_label("今のポーズで上書き")
        .all(|n| !n.accesskit_node().is_disabled()));
    let overwrite = h
        .get_all_by_label("今のポーズで上書き")
        .map(|n| n.rect())
        .next()
        .unwrap();
    click(&mut h, overwrite.center());
    {
        let presets = &h.state().state.view3d.pose.pose_presets;
        assert_eq!(presets.get(id).unwrap().name, "構え");
        let paths: Vec<String> = presets
            .get(id)
            .unwrap()
            .entries
            .iter()
            .map(|e| e.path_text())
            .collect();
        assert_eq!(paths.len(), 2, "左腕と頭: {paths:?}");
        assert!(paths.iter().any(|p| p.ends_with("左上腕")), "{paths:?}");
        assert_eq!(presets.items()[1].entries.len(), 2, "ほかは変わらない");
    }
    assert_eq!(pose_preset_files(&dir), 2);
    // 名前を変える（編集のアイコンで欄になる。Enter で決める）
    let edit_button = h
        .get_all_by_label("名前を変える")
        .map(|n| n.rect())
        .next()
        .unwrap();
    click(&mut h, edit_button.center());
    h.run();
    assert_eq!(h.state().state.view3d.pose.preset_rename, Some(id));
    key(&h, Key::A, Modifiers::COMMAND);
    h.step();
    h.event(Event::Text("左を曲げる".to_owned()));
    h.step();
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    h.run();
    assert_eq!(
        h.state()
            .state
            .view3d
            .pose
            .pose_presets
            .get(id)
            .unwrap()
            .name,
        "左を曲げる"
    );
    assert_eq!(
        h.state().state.view3d.pose.preset_rename,
        None,
        "決めたら欄は元の名前の表示へ"
    );
    assert!(h.query_by_label("左を曲げる").is_some());
    // 読み直しても同じ（名前の変更も上書きもファイルに入っている）
    let mut again = pose_presets_in(&dir);
    assert!(again.problems.is_empty());
    assert_eq!(
        again.items(),
        h.state().state.view3d.pose.pose_presets.items()
    );
    // 消す: 選んだ 1 つだけ
    let delete = h
        .get_all_by_label("このポーズを消す")
        .map(|n| n.rect())
        .next()
        .unwrap();
    click(&mut h, delete.center());
    assert!(h.state().state.view3d.pose.pose_presets.get(id).is_none());
    assert_eq!(h.state().state.view3d.pose.pose_presets.items().len(), 1);
    assert_eq!(pose_preset_files(&dir), 1);
    assert!(h.query_by_label("左を曲げる").is_none());
    again.attach(dir.clone());
    assert_eq!(again.items().len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

fn pose_presets_in(dir: &std::path::Path) -> presets::store::Presets {
    let mut p = presets::store::Presets::default();
    p.attach(dir.to_path_buf());
    p
}

#[test]
fn pose_preset_buttons_wait_for_the_stroke_and_nothing_is_applied_or_written() {
    let dir = temp_dir("pose-presets-stroking");
    let mut h = figure(256);
    h.state_mut()
        .state
        .view3d
        .pose
        .pose_presets
        .attach(dir.clone());
    float_pose_tab(&mut h);
    bend_arm(&mut h);
    let id = presets::save_preset(&mut h.state_mut().state, "構え").unwrap();
    pose::reset(&mut h.state_mut().state.view3d).unwrap();
    h.run();
    for label in ["左右を反転して当てる", "名前を変える", "このポーズを消す"]
    {
        assert!(!disabled(&h, label), "{label}");
    }
    assert!(
        disabled(&h, "今のポーズで上書き"),
        "休みの形では上書きだけ押せない"
    );
    // 描いている最中は、保存も当てるのも消すのも押せない
    {
        let app = &mut h.state_mut().state;
        let layer = app.selected_layer.unwrap();
        let settings = app.stroke_settings(false);
        app.stroke = Some(app.doc.begin_stroke(layer, &settings).unwrap());
    }
    h.run();
    assert!(h.state().state.is_stroking());
    for label in ["左右を反転して当てる", "名前を変える", "このポーズを消す"]
    {
        assert!(disabled(&h, label), "{label}");
    }
    assert!(disabled(&h, "構え"), "名前のボタンも押せない");
    let steps = undo_len(&h);
    assert!(!presets::apply_preset(&mut h.state_mut().state, id, false));
    assert_eq!(undo_len(&h), steps);
    assert!(!session(&h).is_posed());
    assert_eq!(
        h.state().state.message,
        yolu_app::view3d::model::ViewError::Stroking.to_string()
    );
    assert_eq!(pose_preset_files(&dir), 1);
    // 終わると使える
    {
        let app = &mut h.state_mut().state;
        let stroke = app.stroke.take().unwrap();
        app.doc.cancel_stroke(stroke);
    }
    h.run();
    assert!(!disabled(&h, "左右を反転して当てる"));
    h.get_by_label("構え").click();
    h.run();
    assert!(session(&h).is_posed());
    // 休みの形と違うポーズなら上書きも押せるが、描いている最中は上書きも保存も押せない
    h.run();
    assert!(!disabled(&h, "今のポーズで上書き"));
    assert!(!disabled(&h, "ポーズを保存"));
    {
        let app = &mut h.state_mut().state;
        let layer = app.selected_layer.unwrap();
        let settings = app.stroke_settings(false);
        app.stroke = Some(app.doc.begin_stroke(layer, &settings).unwrap());
    }
    h.run();
    assert!(disabled(&h, "今のポーズで上書き"));
    assert!(disabled(&h, "ポーズを保存"));
    {
        let app = &mut h.state_mut().state;
        let stroke = app.stroke.take().unwrap();
        app.doc.cancel_stroke(stroke);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_pose_preset_whose_bones_do_not_fit_applies_what_fits_and_shows_the_reasons_as_a_notice() {
    use yolu_app::view3d::pose::presets::store::PoseEntry;
    let mut h = figure(256);
    float_pose_tab(&mut h);
    let entry = |path: &[&str]| PoseEntry {
        path: path.iter().map(|s| s.to_string()).collect(),
        translation: Vec3::ZERO,
        rotation: Quat::from_rotation_y(0.5),
        scale: Vec3::ONE,
    };
    h.state_mut()
        .state
        .view3d
        .pose
        .pose_presets
        .add(
            "合わない",
            vec![
                entry(&["腰", "背骨", "胸", "首", "頭"]),
                entry(&["別の根", "頭"]),
            ],
        )
        .unwrap();
    h.run();
    assert!(
        !texts(&h).iter().any(|t| t.starts_with("知らせ")),
        "当てるまでは知らせない"
    );
    h.get_by_label("合わない").click();
    h.run();
    let head = bone(&h, "頭");
    assert!(
        session(&h).pose().locals[head] != session(&h).rig.bones()[head].rest,
        "合う骨は当たる"
    );
    let notes = &session(&h).preset_notes;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].path, "別の根/頭");
    assert!(notes[0].describe(Lang::Ja).contains("ボーンがありません"));
    assert!(
        texts(&h).iter().any(|t| t == "知らせ 1 件"),
        "{:?}",
        texts(&h)
    );
    assert!(
        h.state().state.message.contains("飛ばしたボーン 1 件"),
        "{}",
        h.state().state.message
    );
    // 1 つも合わないものは、ポーズを変えない
    let before = session(&h).pose().clone();
    let none = h
        .state_mut()
        .state
        .view3d
        .pose
        .pose_presets
        .add("全部合わない", vec![entry(&["別の根"])])
        .unwrap();
    assert!(!presets::apply_preset(
        &mut h.state_mut().state,
        none,
        false
    ));
    assert_eq!(session(&h).pose(), &before);
    // 読めなかったファイルも、同じ印で知らせる
    let dir = temp_dir("pose-presets-broken");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("pose-1.ylpose"), "壊れた").unwrap();
    h.state_mut()
        .state
        .view3d
        .pose
        .pose_presets
        .attach(dir.clone());
    h.run();
    assert!(
        texts(&h).iter().any(|t| t == "知らせ 2 件"),
        "{:?}",
        texts(&h)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_app_keeps_pose_presets_in_the_folder_next_to_its_settings_and_reads_them_back() {
    let dir = temp_dir("pose-presets-app");
    let settings = dir.join("settings.conf");
    let ctx = egui::Context::default();
    let mut app =
        YoluApp::for_context_with_settings(&ctx, Some(settings.clone()), PenInput::detached());
    app.state.apply(Action::Pose(PoseAction::LoadFigure));
    let mut p = app
        .state
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .pose()
        .clone();
    let arm = app
        .state
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .rig
        .bones()
        .iter()
        .position(|b| b.name == "右上腕")
        .unwrap();
    p.locals[arm].rotation = Quat::from_rotation_z(-0.8);
    pose::set_pose(&mut app.state.view3d, p).unwrap();
    let id = presets::save_preset(&mut app.state, "構え").unwrap();
    assert!(dir
        .join("pose_presets")
        .join(format!("pose-{id}.ylpose"))
        .is_file());
    // 次の起動で、同じポーズのプリセットが一覧にある（隠し方のフォルダとは別）
    let again =
        YoluApp::for_context_with_settings(&ctx, Some(settings.clone()), PenInput::detached());
    assert_eq!(again.state.view3d.pose.pose_presets.items().len(), 1);
    assert_eq!(again.state.view3d.pose.pose_presets.items()[0].name, "構え");
    assert!(again.state.view3d.pose.hide_presets.items().is_empty());
    // 壊れたファイルは読み飛ばして理由を残し、ほかは読む
    std::fs::write(dir.join("pose_presets").join("pose-9.ylpose"), "x").unwrap();
    let broken = YoluApp::for_context_with_settings(&ctx, Some(settings), PenInput::detached());
    assert_eq!(broken.state.view3d.pose.pose_presets.items().len(), 1);
    assert_eq!(broken.state.view3d.pose.pose_presets.problems.len(), 1);
    assert_eq!(
        broken.state.view3d.pose.pose_presets.problems[0].file,
        "pose-9.ylpose"
    );
    // 設定が無ければ、この起動の間だけ
    let mut none = YoluApp::for_context_with_settings(&ctx, None, PenInput::detached());
    assert!(none.state.view3d.pose.pose_presets.dir().is_none());
    none.state.apply(Action::Pose(PoseAction::LoadFigure));
    assert!(presets::save_preset(&mut none.state, "メモリ").is_some());
    std::fs::remove_dir_all(dir).unwrap();
}

// ───────── 日英・文字の収まり ─────────

fn drawn(shape: &Shape, out: &mut Vec<String>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|s| drawn(s, out)),
        Shape::Text(text) => out.push(text.galley.job.text.clone()),
        _ => {}
    }
}

fn texts(h: &H) -> Vec<String> {
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        drawn(&shape.shape, &mut out);
    }
    out
}

/// 描いた文字が、描く先のクリップの中に収まっている（切れていない）。
fn clipped_texts(h: &H) -> Vec<String> {
    fn walk(shape: &Shape, clip: Rect, out: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, out)),
            Shape::Text(text) => {
                let bounds = Rect::from_min_size(text.pos, text.galley.size());
                let visible = clip.y_range().contains(bounds.center().y);
                if visible
                    && (bounds.left() < clip.left() - 1.0 || bounds.right() > clip.right() + 1.0)
                {
                    out.push(format!(
                        "{}: {bounds:?} clip={clip:?}",
                        text.galley.job.text
                    ));
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, shape.clip_rect, &mut out);
    }
    out
}

fn app_in(width: f32, height: f32, lang: Lang) -> H {
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(width, height))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(common::shared_gpu::renderer())
        .build_eframe(move |cc| {
            with_render_state_cpu_canvas(
                YoluApp::for_context(
                    &cc.egui_ctx,
                    AppState::new_in(64, 64, lang),
                    PenInput::detached(),
                ),
                cc.wgpu_render_state.as_ref(),
            )
        });
    // 中央は 1 つの組（`common::app` と同じ並び）
    h.state_mut().dock = common::tabbed_center_dock(width);
    h.run();
    h
}

/// 欄を縦にずらしながら（先頭から終わりまで）、そのつど `visit` に見せる（狭いドックでは、節の下のほうはスクロールした先にある）。
fn scan(h: &mut H, what: &str, visit: &mut dyn FnMut(&mut H, &str)) {
    let mut offset = 0.0;
    loop {
        h.state_mut().state.view3d.pose.panel_scroll = offset;
        h.run();
        visit(h, &format!("{what} @{offset}"));
        if offset > h.state().state.view3d.pose.panel_content {
            break;
        }
        offset += 120.0;
    }
    h.state_mut().state.view3d.pose.panel_scroll = 0.0;
    h.run();
}

/// ポーズの欄の代表の状態を順に出して、そのつど `visit` に見せる。
fn pose_states(h: &mut H, visit: &mut dyn FnMut(&mut H, &str)) {
    h.state_mut().apply(Action::Pose(PoseAction::LoadFigure));
    h.run();
    click_tab(h, Tab::Pose);
    scan(h, "no selection", visit);
    let upper = select(h, "右上腕");
    scan(h, "inspector", visit);
    // 隠す: 1 項目、保存したプリセット、合わない項目の知らせ
    {
        let app = &mut h.state_mut().state;
        hide::set_next(app, 0.5, true);
        hide::hide_bone(app, upper);
    }
    h.run();
    scan(h, "hide entry", visit);
    {
        let app = &mut h.state_mut().state;
        let id = hide::save_preset(app, "").expect("保存できる");
        let other = app
            .view3d
            .pose
            .hide_presets
            .add(
                "Unmatched",
                vec![PresetEntry {
                    path: vec!["x".into(), "y".into()],
                    threshold: 0.5,
                    children: false,
                }],
            )
            .unwrap();
        hide::toggle_preset(app, id);
        hide::toggle_preset(app, other);
    }
    h.run();
    scan(h, "presets and notice", visit);
    // ポーズのプリセット: 保存した名前（既定の名前）・飛ばしたボーンの知らせ
    bend_arm(h);
    {
        let app = &mut h.state_mut().state;
        let id = presets::save_preset(app, "").expect("保存できる");
        let other = app
            .view3d
            .pose
            .pose_presets
            .add(
                "Unmatched",
                vec![presets::store::PoseEntry {
                    path: vec!["x".into(), "y".into()],
                    translation: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                    scale: Vec3::ONE,
                }],
            )
            .unwrap();
        presets::apply_preset(app, id, true);
        presets::apply_preset(app, other, false);
    }
    h.run();
    scan(h, "pose presets and notice", visit);
}

#[test]
fn the_pose_tab_fits_at_ordinary_window_sizes_in_both_languages_and_english_has_no_japanese() {
    widgets::record_truncations(true);
    for (width, height) in [(1600.0, 960.0), (1280.0, 800.0)] {
        for lang in Lang::ALL {
            let mut h = app_in(width, height, lang);
            let mut data: Vec<String> = Vec::new();
            pose_states(&mut h, &mut |h, what| {
                if data.is_empty() {
                    let s = session(h);
                    data.push(s.rig.name().to_owned());
                    data.extend(s.rig.bones().iter().map(|b| b.name.clone()));
                    data.extend(s.rig.materials().iter().cloned());
                    for m in s.rig.meshes() {
                        data.push(m.mesh.name.clone());
                        data.extend(m.blend_shapes.iter().map(|b| b.name.clone()));
                    }
                    data.sort_by_key(|d| std::cmp::Reverse(d.len()));
                }
                let clipped = clipped_texts(h);
                assert!(
                    clipped.is_empty(),
                    "{lang:?} {width}x{height} {what}: {clipped:#?}"
                );
                widgets::take_truncations();
                h.step();
                let truncated = widgets::take_truncations();
                assert!(
                    truncated.is_empty(),
                    "{lang:?} {width}x{height} {what}: 「…」に詰められた文字 {truncated:#?}"
                );
                if lang == Lang::En {
                    // 名前（ファイルの中身）を除いて、日本語が残っていない。保存した名前（既定の名前）も英語
                    let left: Vec<String> = texts(h)
                        .into_iter()
                        .map(|mut t| {
                            for d in &data {
                                t = t.replace(d.as_str(), "");
                            }
                            t
                        })
                        .filter(|t| has_japanese(t))
                        .collect();
                    assert!(left.is_empty(), "{width}x{height} {what}: {left:?}");
                }
            });
            // 英語に日本語が無い見張りが働いていること: 日本語に替えると日本語が見つかる
            if lang == Lang::En {
                h.state_mut().state.lang = Lang::Ja;
                h.run();
                assert!(texts(&h)
                    .iter()
                    .any(|t| has_japanese(t) && t.contains("ボーン")));
            }
        }
    }
    widgets::record_truncations(false);
}

#[test]
fn the_floating_pose_window_fits_too() {
    widgets::record_truncations(true);
    for lang in Lang::ALL {
        let mut h = app_in(1280.0, 860.0, lang);
        h.state_mut().apply(Action::Pose(PoseAction::LoadFigure));
        h.run();
        float_pose_tab(&mut h);
        select(&mut h, "右上腕");
        let clipped = clipped_texts(&h);
        assert!(clipped.is_empty(), "{lang:?}: {clipped:#?}");
        widgets::take_truncations();
        h.step();
        let truncated = widgets::take_truncations();
        assert!(truncated.is_empty(), "{lang:?}: {truncated:#?}");
    }
    widgets::record_truncations(false);
}

#[test]
fn pose_tab_snapshots() {
    let mut snapshots = SnapshotResults::new();
    let mut h = figure(256);
    h.state_mut().state.ui.sections.insert("pose.shapes", false);
    float_pose_tab(&mut h);
    select(&mut h, "右上腕");
    let mut p = session(&h).pose().clone();
    let upper = bone(&h, "右上腕");
    p.locals[upper].rotation = Quat::from_euler(yolu_core::glam::EulerRot::YXZ, 0.3, 0.2, -0.6);
    pose::set_pose(&mut h.state_mut().state.view3d, p).unwrap();
    h.run();
    snapshots.add(h.try_snapshot("pose_tab_inspector"));
    // 隠す: 項目とプリセット
    hide::set_next(&mut h.state_mut().state, 0.5, true);
    hide::hide_bone(&mut h.state_mut().state, upper);
    let app = &mut h.state_mut().state;
    hide::save_preset(app, "右腕").unwrap();
    h.state_mut().state.ui.sections.insert("pose.bones", false);
    h.run();
    snapshots.add(h.try_snapshot("pose_tab_hide"));
}

/// ドックの隅（右の列の下。既定の置き場）のまま使うときの見え方。
#[test]
fn pose_tab_docked_snapshot() {
    let mut snapshots = SnapshotResults::new();
    let mut h = figure(256);
    click_tab(&mut h, Tab::Pose);
    select(&mut h, "右上腕");
    snapshots.add(h.try_snapshot("pose_tab_docked"));
}

/// ポーズのプリセットの節: 保存した名前の一覧（左右反転・上書き・名前を変える・消すの印）と、飛ばしたボーンの知らせ。
#[test]
fn pose_presets_snapshot() {
    use yolu_app::view3d::pose::presets::store::PoseEntry;
    let mut snapshots = SnapshotResults::new();
    let mut h = figure(256);
    float_pose_tab(&mut h);
    for section in ["pose.bones", "pose.hide", "pose.shapes"] {
        h.state_mut().state.ui.sections.insert(section, false);
    }
    bend_arm(&mut h);
    {
        let app = &mut h.state_mut().state;
        presets::save_preset(app, "右を曲げる").unwrap();
        presets::save_preset(app, "").unwrap();
        let other = app
            .view3d
            .pose
            .pose_presets
            .add(
                "合わない",
                vec![PoseEntry {
                    path: vec!["別の根".into(), "頭".into()],
                    translation: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                    scale: Vec3::ONE,
                }],
            )
            .unwrap();
        presets::apply_preset(app, other, false);
    }
    h.run();
    snapshots.add(h.try_snapshot("pose_tab_presets"));
}

#[test]
fn a_pose_edit_through_the_inspector_survives_the_undo_redo_buttons() {
    let mut h = figure(256);
    h.state_mut().state.ui.sections.insert("pose.hide", false);
    float_pose_tab(&mut h);
    let upper = select(&mut h, "右上腕");
    let at = center_of(&h, ROTATION_Z);
    type_over(&mut h, at, "45");
    let turned = local(&h, upper).rotation;
    // 欄の頭の取り消し・やり直しボタン
    h.get_by_label("取り消し").click();
    h.run();
    assert_eq!(local(&h, upper).rotation, rest(&h, upper).rotation);
    h.get_by_label("やり直し").click();
    h.run();
    assert_eq!(local(&h, upper).rotation, turned);
    // ポーズを全部戻すボタンも 1 段
    let steps = undo_len(&h);
    h.get_by_label("ポーズを戻す（ファイルのポーズへ）").click();
    h.run();
    assert_eq!(undo_len(&h), steps + 1);
}

// ───────── 読み込み中の行 ─────────

/// モデルが無くなったあとの欄（タブだけが残る）で、読み込み中は名前・割合・取り消しのボタンが出る。割合が分かるまでは割合を出さない。
/// 取り消すと読み込み中でなくなる（ポーズのタブは残る）。説明の文は置かない（ボタンの説明はツールチップ）。
#[test]
fn the_panel_shows_a_loading_row_with_progress_and_a_cancel_button() {
    for lang in Lang::ALL {
        let mut h = figure(64);
        h.state_mut().state.lang = lang;
        click_tab(&mut h, Tab::Pose);
        h.state_mut().apply(Action::LoadDemoModel);
        h.run();
        h.run();
        assert!(h.state().dock.find_tab(&Tab::Pose).is_some());
        let (loading, cancel_label, done) = match lang {
            Lang::Ja => (
                "読み込み中: 新.fbx",
                "読み込みを取り消す",
                "新.fbxの読み込みを取り消しました。",
            ),
            Lang::En => (
                "Loading: 新.fbx",
                "Cancel loading",
                "Cancelled loading 新.fbx.",
            ),
        };
        // 割合がまだ分からない間: 名前だけ
        // 読み込み中は描き直しを頼み続けるので、`run` ではなく数フレームだけ回す
        let hold = pose::park_loading(&mut h.state_mut().state.view3d, "新.fbx", None);
        h.run_steps(3);
        assert!(
            texts(&h).iter().any(|t| t == loading),
            "{lang:?}: {:?}",
            texts(&h)
        );
        assert!(h.query_by_label(cancel_label).is_some(), "{lang:?}");
        // 割合が分かったら添える
        drop(hold);
        let _hold = pose::park_loading(&mut h.state_mut().state.view3d, "新.fbx", Some(0.42));
        h.run_steps(3);
        assert!(
            texts(&h).iter().any(|t| *t == format!("{loading} 42%")),
            "{lang:?}: {:?}",
            texts(&h)
        );
        assert!(clipped_texts(&h).is_empty(), "{:?}", clipped_texts(&h));
        // 取り消すと、読み込み中でなくなる
        h.get_by_label(cancel_label).click();
        h.run_steps(3);
        assert!(!h.state().state.view3d.pose.is_loading(), "{lang:?}");
        assert_eq!(h.state().state.message, done);
        assert!(
            !texts(&h).iter().any(|t| t.starts_with(loading)),
            "{lang:?}"
        );
        assert!(h.state().dock.find_tab(&Tab::Pose).is_some(), "タブは残る");
    }
}
