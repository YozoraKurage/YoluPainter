//! 2D のキャンバスと 3D ビューを揃える（その 2）: 3D でもクイックマスクのブラシ・消しゴムが選択ペン・選択消しとして働くこと、2D の対称と
//! 3D の対称がどちらのビューのストロークにも効くこと（両方なら写しの数は掛け算）、3D でも予算を超えたらストロークごと取り消すこと。
//! 実際の入力（3D ビューと 2D のキャンバスのマウスのイベント）を流して確かめる。
use crate::common;
use crate::view3d_brush::{cube_view, screen_of};

use common::*;
use egui::{Key, Modifiers, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::engine::composite_pixel;
use yolu_app::rulers::RulerAction;
use yolu_app::selection::{SelAction, SelUiOp};
use yolu_app::state::{Action, Tool};
use yolu_app::{Tab, YoluApp};
use yolu_core::glam::{DVec3, Vec3};

type H = Harness<'static, YoluApp>;

const SIZE: u32 = 256;

fn st(h: &H) -> &yolu_app::state::AppState {
    &h.state().state
}

fn message(h: &H) -> String {
    st(h).message.clone()
}

/// 文書の全画素の合成（レイヤーの画素が変わっていないかの比較）。
fn snapshot(h: &H) -> Vec<u8> {
    let doc = &st(h).doc;
    doc.composite(doc.bounds()).unwrap()
}

fn alpha(h: &H, x: u32, y: u32) -> u8 {
    composite_pixel(&st(h).doc, x, y)[3]
}

fn painted(h: &H) -> Vec<(u32, u32, u8)> {
    let mut out = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let a = alpha(h, x, y);
            if a > 0 {
                out.push((x, y, a));
            }
        }
    }
    out
}

/// 立方体の面の点（手前の面なら facing は −Z）の、文書の画素の座標（UV × 文書の大きさ）。
fn texel_of(h: &H, p: Vec3, facing: Vec3) -> (f64, f64) {
    let model = st(h).view3d.model.as_ref().expect("モデル");
    let hit = model
        .geometry
        .find_closest_point(p, 0.01, facing, 1 << 20, -1)
        .unwrap()
        .expect("面の点");
    (hit.uv.x as f64 * SIZE as f64, hit.uv.y as f64 * SIZE as f64)
}

/// (x, y) のまわり（半径 r 画素）に何か塗られているか。
fn ink_near(h: &H, (x, y): (f64, f64), r: u32) -> bool {
    let (cx, cy) = (x.floor() as u32, y.floor() as u32);
    (cy.saturating_sub(r)..=(cy + r).min(SIZE - 1))
        .any(|yy| (cx.saturating_sub(r)..=(cx + r).min(SIZE - 1)).any(|xx| alpha(h, xx, yy) > 0))
}

fn selected_near(h: &H, (x, y): (f64, f64)) -> u8 {
    st(h)
        .doc
        .selection()
        .map_or(0, |m| m.amount(x.floor() as u32, y.floor() as u32))
}

fn thin_hard_brush(h: &mut H) {
    let s = &mut h.state_mut().state;
    s.brush.radius = 4.0;
    s.brush.hardness = 1.0;
    s.brush.opacity = 1.0;
    s.m2.random_seed = false;
    s.color.set_main([0.9, 0.1, 0.1, 1.0]);
}

fn undo(h: &mut H) {
    key(h, Key::Z, Modifiers::COMMAND);
    h.run();
}

fn quick_on(h: &mut H) {
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(true)))));
    h.run();
}

/// 選んでいるレイヤーの対称定規を全部外す。
fn clear_rulers(h: &mut H) {
    let s = &mut h.state_mut().state;
    let layer = s.selected_layer.unwrap();
    let ids: Vec<_> = common::rulers::of(s, layer).iter().map(|r| r.id).collect();
    if !ids.is_empty() {
        s.apply(Action::Ruler(RulerAction::Delete { owner: layer, ids }));
    }
    h.run();
}

/// 2D の縦の対称定規（文書の x = 128）。
fn vertical_2d(h: &mut H) {
    common::rulers::vertical(&mut h.state_mut().state, SIZE as f64 / 2.0);
    h.run();
}

/// 3D の鏡（X に直交する面、モデルの原点）。
fn mirror_3d(h: &mut H) {
    common::rulers::mirror_3d(&mut h.state_mut().state, DVec3::ZERO, DVec3::X);
    h.run();
}

/// 3D ビューの 2 つの面の点の間を引く。
fn drag_3d(h: &mut H, rect: Rect, from: Vec3, to: Vec3) {
    let (a, b) = (screen_of(h, rect, from), screen_of(h, rect, to));
    let path: Vec<Pos2> = (0..=8).map(|i| a + (b - a) * (i as f32 / 8.0)).collect();
    drag(h, &path);
}

/// 2D のキャンバスの文書の点の、画面の点。
fn canvas_screen(h: &H, (x, y): (f64, f64)) -> Pos2 {
    let r = canvas_rect(h);
    let s = st(h);
    s.view
        .view(r, s.doc.width(), s.doc.height())
        .to_screen(x, y)
}

/// 3D ビューの面の点を押して離す。
fn click_3d(h: &mut H, rect: Rect, p: Vec3) {
    let at = screen_of(h, rect, p);
    click(h, at);
}

/// 2D のキャンバスの文書の点を押して離す。
fn click_2d(h: &mut H, texel: (f64, f64)) {
    let at = canvas_screen(h, texel);
    click(h, at);
}

// ───────── クイックマスク ─────────

#[test]
fn quick_mask_in_3d_paints_the_selection_and_erases_it_and_never_touches_the_layer() {
    let (mut h, rect) = cube_view();
    thin_hard_brush(&mut h);
    quick_on(&mut h);
    let layer = snapshot(&h);
    let front = Vec3::new(0.25, 0.0, -0.5);
    let texel = texel_of(&h, front, Vec3::NEG_Z);
    // ブラシ: 選択ペン（1 ストロークが 1 回の取り消し）
    click_3d(&mut h, rect, front);
    assert_eq!(message(&h), "選択範囲: 追加", "2D の選択ペンと同じ知らせ");
    assert_eq!(
        selected_near(&h, texel),
        255,
        "押した面の UV の画素が選択範囲に"
    );
    assert_eq!(snapshot(&h), layer, "レイヤーの画素は変わらない");
    assert_eq!(st(&h).doc.undo_count(), 1);
    assert!(!st(&h).is_stroking() && st(&h).sel.pen.is_none());
    // 線を引くと、通った面の画素も
    let other = Vec3::new(-0.25, 0.1, -0.5);
    drag_3d(&mut h, rect, Vec3::new(-0.25, -0.1, -0.5), other);
    assert_eq!(st(&h).doc.undo_count(), 2);
    assert_eq!(selected_near(&h, texel_of(&h, other, Vec3::NEG_Z)), 255);
    // 消しゴム: 選択消し
    h.state_mut().state.apply(Action::SelectTool(Tool::Eraser));
    h.run();
    thin_hard_brush(&mut h);
    click_3d(&mut h, rect, front);
    assert_eq!(selected_near(&h, texel), 0, "消した所は外れる");
    assert_eq!(st(&h).doc.undo_count(), 3);
    assert_eq!(snapshot(&h), layer);
    // 取り消しは文書の取り消し
    undo(&mut h);
    assert_eq!(selected_near(&h, texel), 255);
    // Esc で捨てたストロークは何も残さない
    let steps = st(&h).doc.undo_count();
    let at = screen_of(&h, rect, Vec3::new(0.0, -0.3, -0.5));
    press(&h, at, egui::PointerButton::Primary);
    h.step();
    move_to(&h, screen_of(&h, rect, Vec3::new(0.1, -0.3, -0.5)));
    h.step();
    assert!(st(&h).is_stroking(), "3D のクイックマスクのストロークの間");
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    release(&h, at, egui::PointerButton::Primary);
    h.run();
    assert_eq!(st(&h).doc.undo_count(), steps);
    assert!(!st(&h).is_stroking() && st(&h).sel.pen.is_none());
    // 切ると重ねは消え、選択範囲は残る。ブラシは普通に描く
    let selection = st(&h).doc.selection().cloned();
    key(&h, Key::Q, Modifiers::SHIFT);
    h.run();
    assert!(!st(&h).sel.quick);
    assert_eq!(
        st(&h).sel.quick_overlay.texture_count(),
        0,
        "重ねの絵は捨てる"
    );
    assert_eq!(st(&h).doc.selection().cloned(), selection);
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    h.run();
    thin_hard_brush(&mut h);
    click_3d(&mut h, rect, front);
    assert!(ink_near(&h, texel, 2), "選択範囲の内側に普通に描ける");
}

#[test]
fn a_quick_mask_stroke_in_3d_changes_the_red_overlay_of_the_canvas_beside_it() {
    use egui_dock::{DockState, NodeIndex};
    let (mut h, _) = cube_view();
    thin_hard_brush(&mut h);
    // キャンバスと 3D ビューを並べる
    let mut dock = DockState::new(vec![Tab::Canvas]);
    dock.main_surface_mut()
        .split_right(NodeIndex::root(), 0.5, vec![Tab::View3d]);
    h.state_mut().dock = dock;
    h.run();
    quick_on(&mut h);
    let rect = h.state().view3d_rect().expect("3D も出ている");
    let front = Vec3::new(0.25, 0.0, -0.5);
    click_3d(&mut h, rect, front);
    assert_eq!(selected_near(&h, texel_of(&h, front, Vec3::NEG_Z)), 255);
    h.run();
    assert!(
        st(&h).sel.quick_overlay.texture_count() > 0,
        "キャンバスの赤い重ねが選択範囲を見せる"
    );
}

/// ペンの点（筆圧・消しゴムの端）。
fn pen(
    at: Pos2,
    contact: bool,
    pressure: f32,
    eraser: bool,
    time_ms: u32,
) -> yolu_app::pen::PenSample {
    yolu_app::pen::PenSample {
        pos: [at.x, at.y],
        pressure: if contact { pressure } else { 0.0 },
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser,
        barrel: false,
        pointer_id: 7,
        time_ms,
    }
}

fn pen_dot(h: &mut H, at: Pos2, pressure: f32, eraser: bool) {
    h.state().pen().push(pen(at, true, pressure, eraser, 0));
    h.step();
    h.state().pen().push(pen(at, false, 0.0, eraser, 50));
    h.step();
    h.run();
}

#[test]
fn quick_mask_in_3d_follows_the_pen_pressure_and_the_pen_eraser_end() {
    let (mut h, rect) = cube_view();
    thin_hard_brush(&mut h);
    h.state_mut().state.brush.pressure_opacity = true;
    quick_on(&mut h);
    let front = Vec3::new(0.25, 0.0, -0.5);
    let texel = texel_of(&h, front, Vec3::NEG_Z);
    let at = screen_of(&h, rect, front);
    // 筆圧 0.5 で不透明度も半分（2D の選択ペンと同じ量）
    pen_dot(&mut h, at, 0.5, false);
    let half = selected_near(&h, texel);
    assert!((120..=135).contains(&half), "{half}");
    assert_eq!(st(&h).doc.undo_count(), 1);
    // 満の筆圧で満量
    pen_dot(&mut h, at, 1.0, false);
    assert_eq!(selected_near(&h, texel), 255);
    // ペンの消しゴムの端は選択消し（ツールはブラシのまま）
    pen_dot(&mut h, at, 1.0, true);
    assert_eq!(selected_near(&h, texel), 0);
    assert_eq!(st(&h).doc.undo_count(), 3);
    assert_eq!(st(&h).tool, Tool::Brush);
}

#[test]
fn a_3d_quick_mask_stroke_past_the_pen_budget_is_dropped_and_leaves_nothing() {
    let (mut h, rect) = cube_view();
    thin_hard_brush(&mut h);
    quick_on(&mut h);
    let layer = snapshot(&h);
    // 被覆も投影の塗りも入らない作業の予算
    h.state_mut().state.sel.pen_budget = 1024;
    drag_3d(
        &mut h,
        rect,
        Vec3::new(-0.3, 0.0, -0.5),
        Vec3::new(0.3, 0.0, -0.5),
    );
    assert_eq!(message(&h), "ストロークが大きすぎます");
    assert!(st(&h).doc.selection().is_none());
    assert_eq!(st(&h).doc.undo_count(), 0);
    assert!(!st(&h).is_stroking() && st(&h).sel.pen.is_none());
    assert!(st(&h).view3d.input.cover.is_none());
    assert_eq!(snapshot(&h), layer);
    // 予算を戻せば描ける
    h.state_mut().state.sel.pen_budget = yolu_app::selection::pen::PEN_BUDGET_BYTES;
    let front = Vec3::new(0.25, 0.0, -0.5);
    click_3d(&mut h, rect, front);
    assert_eq!(selected_near(&h, texel_of(&h, front, Vec3::NEG_Z)), 255);
}

/// 片方のビューで選択ペン（クイックマスク）のストロークが動いている間、もう片方のビューのクイックマスクのストロークは始めない
/// （動いているストロークの被覆を上書きしない）。2D のクイックマスクのストロークを始めの口から始めておき、3D ビューを押す。
#[test]
fn a_quick_mask_stroke_on_the_canvas_is_not_overwritten_by_a_press_in_the_3d_view() {
    let (mut h, rect) = cube_view();
    thin_hard_brush(&mut h);
    quick_on(&mut h);
    let front = Vec3::new(0.25, 0.0, -0.5);
    let texel = texel_of(&h, front, Vec3::NEG_Z);
    let began = yolu_app::selection::quick::begin(
        &mut h.state_mut().state,
        yolu_app::state::StrokeSource::Pen(7),
        false,
    );
    assert_eq!(began, Some(true));
    click_3d(&mut h, rect, front);
    assert_eq!(message(&h), "描いている間はできません。");
    assert!(
        st(&h)
            .sel
            .pen
            .as_ref()
            .is_some_and(|a| a.source == yolu_app::state::StrokeSource::Pen(7)),
        "キャンバスのストロークのまま"
    );
    assert!(st(&h).view3d.input.cover.is_none());
    assert!(yolu_app::selection::quick::finish(
        &mut h.state_mut().state,
        true
    ));
    assert_eq!(selected_near(&h, texel), 0);
    assert_eq!(st(&h).doc.undo_count(), 0);
}

/// 3D ビューのクイックマスクのストロークが動いている間は、2D のクイックマスクのストロークも始めない（2D の始めの口を直に呼ぶ）。
#[test]
fn a_quick_mask_stroke_in_3d_is_not_overwritten_by_a_canvas_stroke() {
    let (mut h, rect) = cube_view();
    thin_hard_brush(&mut h);
    quick_on(&mut h);
    let front = Vec3::new(0.25, 0.0, -0.5);
    let at = screen_of(&h, rect, front);
    press(&h, at, egui::PointerButton::Primary);
    h.step();
    assert!(st(&h).view3d.input.cover.is_some());
    let began = yolu_app::selection::quick::begin(
        &mut h.state_mut().state,
        yolu_app::state::StrokeSource::Pen(9),
        false,
    );
    assert_eq!(began, Some(false));
    assert_eq!(message(&h), "描いている間はできません。");
    assert!(st(&h)
        .sel
        .pen
        .as_ref()
        .is_some_and(|a| a.source == yolu_app::state::StrokeSource::Mouse));
    release(&h, at, egui::PointerButton::Primary);
    h.run();
    assert_eq!(selected_near(&h, texel_of(&h, front, Vec3::NEG_Z)), 255);
    assert_eq!(st(&h).doc.undo_count(), 1);
}

// ───────── 対称 ─────────

#[test]
fn the_2d_symmetry_also_mirrors_a_3d_stroke_in_uv() {
    let (mut h, rect) = cube_view();
    thin_hard_brush(&mut h);
    vertical_2d(&mut h); // 文書の x = 128
    let base = st(&h).doc.undo_count();
    let front = Vec3::new(0.25, 0.0, -0.5);
    click_3d(&mut h, rect, front);
    assert!(message(&h).is_empty(), "{}", message(&h));
    assert_eq!(st(&h).doc.undo_count(), base + 1, "写しも 1 回の取り消し");
    let dots = painted(&h);
    let (x, y) = texel_of(&h, front, Vec3::NEG_Z);
    assert!(ink_near(&h, (x, y), 2));
    assert!(ink_near(&h, (SIZE as f64 - x, y), 2), "UV の平面で写った所");
    // 軸が画素の境にあるので、写しは元の左右反転と同じ量
    for &(px, py, a) in &dots {
        assert_eq!(alpha(&h, SIZE - 1 - px, py), a, "({px}, {py})");
    }
    // 2D の対称定規を外せば、元の側だけ
    undo(&mut h);
    clear_rulers(&mut h);
    click_3d(&mut h, rect, front);
    assert!(!ink_near(&h, (SIZE as f64 - x, y), 2));
}

#[test]
fn the_3d_symmetry_also_mirrors_a_2d_stroke_through_the_model() {
    let (mut h, _) = cube_view();
    thin_hard_brush(&mut h);
    mirror_3d(&mut h); // X に直交する面、モデルの原点
    click_tab(&mut h, Tab::Canvas);
    h.run();
    let base = st(&h).doc.undo_count();
    let a = texel_of(&h, Vec3::new(0.25, 0.1, -0.5), Vec3::NEG_Z);
    let b = texel_of(&h, Vec3::new(-0.25, 0.1, -0.5), Vec3::NEG_Z);
    assert!((a.0 - b.0).abs() > 20.0, "{a:?} {b:?}");
    click_2d(&mut h, a);
    assert!(message(&h).is_empty(), "{}", message(&h));
    assert!(ink_near(&h, a, 2));
    assert!(ink_near(&h, b, 2), "モデルの反対側の UV に塗れる");
    assert_eq!(st(&h).doc.undo_count(), base + 1);
    // 写しの面が無いとき（面をモデルの外へずらす）は、その写しだけ飛ばして知らせる
    undo(&mut h);
    clear_rulers(&mut h);
    common::rulers::mirror_3d(
        &mut h.state_mut().state,
        DVec3::new(0.45, 0.0, 0.0),
        DVec3::X,
    );
    h.run();
    click_2d(&mut h, a);
    assert!(ink_near(&h, a, 2));
    assert!(!ink_near(&h, b, 2));
    assert_eq!(message(&h), "近くに面が無い対称の写しは飛ばしました");
}

#[test]
fn both_symmetries_multiply_the_copies_in_both_views() {
    let (mut h, rect) = cube_view();
    thin_hard_brush(&mut h);
    mirror_3d(&mut h);
    vertical_2d(&mut h);
    let p = Vec3::new(0.25, 0.1, -0.5);
    let a = texel_of(&h, p, Vec3::NEG_Z);
    let b = texel_of(&h, Vec3::new(-0.25, 0.1, -0.5), Vec3::NEG_Z);
    let spots = [a, b, (SIZE as f64 - a.0, a.1), (SIZE as f64 - b.0, b.1)];
    // 3D ビュー
    click_3d(&mut h, rect, p);
    for s in spots {
        assert!(ink_near(&h, s, 2), "3D: {s:?}");
    }
    let in_3d = painted(&h).len();
    undo(&mut h);
    assert!(painted(&h).is_empty());
    // 2D のキャンバス（同じ点に同じ 4 つ）
    click_tab(&mut h, Tab::Canvas);
    h.run();
    click_2d(&mut h, a);
    for s in spots {
        assert!(ink_near(&h, s, 2), "2D: {s:?}");
    }
    assert!(
        painted(&h).len() > in_3d / 4,
        "{} {in_3d}",
        painted(&h).len()
    );
}

#[test]
fn smudge_refuses_the_2d_symmetry_in_3d_and_the_3d_symmetry_in_2d() {
    let (mut h, rect) = cube_view();
    h.state_mut().state.m2.brush.effect = yolu_app::engine::BrushEffect::Smudge { strength: 1.0 };
    vertical_2d(&mut h);
    let base = st(&h).doc.undo_count();
    let p = Vec3::new(0.25, 0.1, -0.5);
    drag_3d(&mut h, rect, p, Vec3::new(0.3, 0.1, -0.5));
    assert_eq!(message(&h), "指先・クローンでは対称を使えません");
    assert!(st(&h).doc.undo_count() == base && !st(&h).is_stroking());
    clear_rulers(&mut h);
    mirror_3d(&mut h);
    let base = st(&h).doc.undo_count();
    click_tab(&mut h, Tab::Canvas);
    h.run();
    let a = texel_of(&h, p, Vec3::NEG_Z);
    click_2d(&mut h, a);
    assert!(st(&h).doc.undo_count() == base && !st(&h).is_stroking());
    assert!(!message(&h).is_empty(), "断った理由を出す");
}

#[test]
fn symmetric_strokes_from_both_views_survive_save_and_reopen() {
    let dir = crate::common::tmp::test_dir("parity-symmetry");
    let path = dir.join("sym.ylp");
    let (mut h, rect) = cube_view();
    thin_hard_brush(&mut h);
    mirror_3d(&mut h);
    common::rulers::horizontal(&mut h.state_mut().state, SIZE as f64 / 2.0);
    h.run();
    click_3d(&mut h, rect, Vec3::new(0.25, 0.1, -0.5));
    click_tab(&mut h, Tab::Canvas);
    h.run();
    let a = texel_of(&h, Vec3::new(0.3, -0.2, -0.5), Vec3::NEG_Z);
    click_2d(&mut h, a);
    let saved = snapshot(&h);
    assert!(painted(&h).len() > 40);
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    assert!(message(&h).starts_with("保存しました"), "{}", message(&h));
    let mut again = app(800.0, 600.0, SIZE);
    again
        .state_mut()
        .state
        .apply(Action::OpenProject(path.clone()));
    again.run();
    assert_eq!(snapshot(&again), saved, "{}", message(&again));
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── 予算 ─────────

#[test]
fn a_3d_stroke_over_the_budget_is_cancelled_whole_with_the_same_words_as_2d() {
    let (mut h, rect) = cube_view();
    thin_hard_brush(&mut h);
    let before = snapshot(&h);
    h.state_mut()
        .state
        .doc
        .set_stroke_budget_bytes(4096)
        .unwrap();
    drag_3d(
        &mut h,
        rect,
        Vec3::new(-0.3, 0.0, -0.5),
        Vec3::new(0.3, 0.0, -0.5),
    );
    let in_3d = message(&h);
    assert!(!in_3d.is_empty());
    assert_eq!(snapshot(&h), before, "文書は前と同じバイト");
    assert_eq!(st(&h).doc.undo_count(), 0);
    assert!(!st(&h).is_stroking() && !st(&h).doc.has_active_stroke());
    // 2D のキャンバスで同じ予算を超えたときと同じ文
    click_tab(&mut h, Tab::Canvas);
    h.run();
    h.state_mut().state.message.clear();
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -30.0, 0.0), offset(c, 30.0, 0.0)]);
    assert_eq!(message(&h), in_3d);
    assert_eq!(snapshot(&h), before);
}

/// 塗り始めてから断られる筋書き: 文書（256 角、タイル 128 角）の手前の面（タイル (0, 0)）から辺をまたいで右の面（タイル (0, 1)）へ
/// 引く。手前の面だけを引いて測った投影の塗りと巻き戻しの量から、手前の面は塗れて、右の面へ入って巻き戻しが 1 タイル増えると
/// 入らない予算にする。途中まで塗った画素も戻り、文書は前と同じバイト・履歴も増えない。
#[test]
fn a_3d_stroke_refused_midway_takes_back_what_it_had_painted() {
    let (mut h, rect) = cube_view();
    thin_hard_brush(&mut h);
    h.state_mut().state.brush.radius = 10.0;
    let before = snapshot(&h);
    let front: Vec<Pos2> = (0..=6)
        .map(|i| screen_of(&h, rect, Vec3::new(-0.3 + 0.1 * i as f32, 0.0, -0.5)))
        .collect();
    // 測る: 手前の面だけを引いて、Esc で捨てる
    press(&h, front[0], egui::PointerButton::Primary);
    h.step();
    for p in &front[1..] {
        move_to(&h, *p);
        h.step();
    }
    let (fixed, largest, rollback) = {
        let s = st(&h);
        let surface = s.view3d.input.surface.as_ref().expect("3D のストローク");
        (
            surface.projection_fixed_bytes(),
            surface.projection_stats().largest_dab_bytes,
            s.doc.active_stroke_stats().unwrap().rollback_bytes,
        )
    };
    assert!(rollback > 0);
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    release(&h, front[6], egui::PointerButton::Primary);
    h.run();
    assert_eq!(snapshot(&h), before);
    h.state_mut()
        .state
        .doc
        .set_stroke_budget_bytes(fixed + largest + rollback)
        .unwrap();
    h.state_mut().state.message.clear();
    // 引く: 手前の面は塗れる
    press(&h, front[0], egui::PointerButton::Primary);
    h.step();
    for p in &front[1..] {
        move_to(&h, *p);
        h.step();
    }
    assert!(st(&h).doc.has_active_stroke(), "{}", message(&h));
    assert_ne!(snapshot(&h), before, "途中まで塗っている");
    // 辺をまたいで右の面へ
    for i in 0..=8 {
        let p = screen_of(&h, rect, Vec3::new(0.5, 0.0, -0.4 + 0.1 * i as f32));
        move_to(&h, p);
        h.step();
    }
    release(
        &h,
        screen_of(&h, rect, Vec3::new(0.5, 0.0, 0.4)),
        egui::PointerButton::Primary,
    );
    h.run();
    assert_eq!(
        message(&h),
        "1 回の操作のメモリの予算を超えます（取り消しました）",
        "2D の予算超えと同じ文"
    );
    assert_eq!(
        snapshot(&h),
        before,
        "途中まで塗った画素も戻り、前と同じバイト"
    );
    assert_eq!(st(&h).doc.undo_count(), 0);
    assert!(!st(&h).is_stroking() && !st(&h).doc.has_active_stroke());
}
