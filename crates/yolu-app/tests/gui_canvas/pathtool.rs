//! パスのツール（P）の振る舞い（2D のキャンバスと 3D のビュー）。どれも「操作 → 文書が変わる → 1 回の Undo で戻る」と、ロックでの断り、
//! 保存して開き直しても同じ、Esc・フォーカス喪失・取りこぼしでドラッグを取り残さない、日本語と英語。`headless_` で始まる試験は画面を描かず、
//! Wine でも回る。egui_kittest の試験は画面の操作と見た目。
use crate::common;

use common::*;
use egui::{epaint::Shape, pos2, vec2, Event, Key, Modifiers, Pos2, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::engine::{composite_pixel, Channel};
use yolu_app::lang::Lang;
use yolu_app::matpaint::MatAction;
use yolu_app::pathtool::edit::TangentValue;
use yolu_app::pathtool::edit::{Place, PointOp};
use yolu_app::pathtool::list::ListOp;
use yolu_app::pathtool::{self, BrushEdit, PathAction, StyleEdit};
use yolu_app::state::{Action, AppState, StrokeSource, Tool};
use yolu_app::view3d::model::ViewModel;
use yolu_app::YoluApp;
use yolu_core::geometry::{ModelMesh, Submesh};
use yolu_core::glam::{DVec2, Vec2, Vec3};
use yolu_core::paths::{self as core_paths, PathKind};
use yolu_core::{LayerId, LayerLocks, LayerPath};

// ───────── 2D のツール ─────────

const RECT: fn() -> Rect = || Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0));

fn state(size: u32) -> AppState {
    let mut s = AppState::new(size, size);
    s.apply(Action::SelectTool(Tool::Path));
    s
}

fn view(s: &AppState) -> yolu_app::canvas::view::CanvasView {
    s.view.view(RECT(), s.doc.width(), s.doc.height())
}

/// キャンバスの点（画素の座標）が見える画面の点。
fn at(s: &AppState, x: f64, y: f64) -> Pos2 {
    view(s).to_screen(x, y)
}

fn press2d(s: &mut AppState, x: f64, y: f64) {
    let v = view(s);
    pathtool::canvas::press(s, &v, at(s, x, y), StrokeSource::Mouse);
}

fn move2d(s: &mut AppState, x: f64, y: f64) {
    let v = view(s);
    pathtool::canvas::moved(s, &v, at(s, x, y), StrokeSource::Mouse);
}

fn release2d(s: &mut AppState, x: f64, y: f64) {
    let v = view(s);
    pathtool::canvas::release(s, &v, at(s, x, y), StrokeSource::Mouse);
}

fn click2d(s: &mut AppState, x: f64, y: f64) {
    press2d(s, x, y);
    release2d(s, x, y);
}

fn drag2d(s: &mut AppState, from: (f64, f64), to: (f64, f64)) {
    press2d(s, from.0, from.1);
    move2d(s, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
    move2d(s, to.0, to.1);
    release2d(s, to.0, to.1);
}

fn path(s: &AppState) -> Option<LayerPath> {
    s.path_layer().map(|(_, p)| p.clone())
}

fn canvas_points(s: &AppState) -> Vec<(f64, f64, f64)> {
    match path(s) {
        Some(LayerPath::Canvas(c)) => c.points.iter().map(|p| (p.x, p.y, p.pressure)).collect(),
        other => panic!("2D のパスが無い: {other:?}"),
    }
}

fn pixel(s: &AppState, x: u32, y: u32) -> [u8; 4] {
    composite_pixel(&s.doc, x, y)
}

#[test]
fn headless_clicks_add_points_and_the_first_one_makes_a_path_layer_each_one_undo() {
    let mut s = state(256);
    let base = s.selected_layer.unwrap();
    assert_eq!(s.doc.layers().len(), 1);
    click2d(&mut s, 40.0, 40.0);
    assert_eq!(
        s.doc.layers().len(),
        2,
        "パスの無いレイヤーには、その上に新しいパスレイヤーを作る: {}",
        s.message
    );
    let layer = s.selected_layer.unwrap();
    assert_ne!(layer, base);
    assert!(
        s.doc.layer(base).unwrap().path().is_none(),
        "元のレイヤーには触れない"
    );
    assert_eq!(s.doc.undo_count(), 1);
    click2d(&mut s, 120.0, 60.0);
    click2d(&mut s, 200.0, 180.0);
    assert_eq!(
        canvas_points(&s)
            .iter()
            .map(|p| (p.0, p.1))
            .collect::<Vec<_>>()
            .len(),
        3
    );
    assert_eq!(
        s.doc.layers().len(),
        2,
        "2 点目からは同じレイヤーのパスを描き直す"
    );
    assert_eq!(s.doc.undo_count(), 3, "点を足すたびに 1 回の Undo");
    // 点と点を結ぶ曲線が、画素になっている（点の所と、点の間）
    assert!(pixel(&s, 40, 40)[3] > 0 && pixel(&s, 200, 180)[3] > 0);
    assert!(pixel(&s, 80, 50)[3] > 0, "点の間も描かれている");
    assert_eq!(pixel(&s, 40, 200)[3], 0, "曲線から離れた所は描かない");
    assert_eq!(s.path_selected_index(), Some(2), "足した点を選ぶ");
    // 1 回ずつ戻る
    s.apply(Action::Undo);
    assert_eq!(canvas_points(&s).len(), 2);
    assert_eq!(pixel(&s, 200, 180)[3], 0);
    s.apply(Action::Undo);
    s.apply(Action::Undo);
    assert_eq!(
        s.doc.layers().len(),
        1,
        "最初の点を戻すとレイヤーごと消える"
    );
    s.apply(Action::Redo);
    assert_eq!(s.doc.layers().len(), 2);
    let redone = s
        .doc
        .layers()
        .iter()
        .find(|l| l.path().is_some())
        .expect("パスレイヤーが戻る");
    assert_eq!(redone.path().unwrap().point_count(), 1);
}

#[test]
fn headless_the_canvas_edge_is_the_limit_for_new_and_moved_points() {
    let mut s = state(128);
    click2d(&mut s, -30.0, 50.0);
    assert_eq!(s.doc.layers().len(), 1, "キャンバスの外には足さない");
    assert!(s.message.contains("外"), "{}", s.message);
    click2d(&mut s, 20.0, 20.0);
    click2d(&mut s, 60.0, 60.0);
    // 外へ動かしても、縁に収まる
    drag2d(&mut s, (60.0, 60.0), (400.0, -90.0));
    assert_eq!(canvas_points(&s)[1].0, 128.0);
    assert_eq!(canvas_points(&s)[1].1, 0.0);
    s.lang = Lang::En;
    s.message.clear();
    click2d(&mut s, 500.0, 500.0);
    assert_eq!(s.message, "Outside the canvas");
}

#[test]
fn headless_dragging_a_point_moves_it_in_one_undo_and_a_click_only_selects() {
    let mut s = state(256);
    for (x, y) in [(30.0, 30.0), (120.0, 90.0), (210.0, 30.0)] {
        click2d(&mut s, x, y);
    }
    let undo = s.doc.undo_count();
    // 動かさずに離す: 選ぶだけ。文書は変わらない
    let revision = s.doc.revision();
    click2d(&mut s, 120.0, 90.0);
    assert_eq!(s.doc.revision(), revision);
    assert_eq!(s.path_selected_index(), Some(1));
    assert_eq!(s.doc.undo_count(), undo);
    // 動かす
    drag2d(&mut s, (120.0, 90.0), (120.0, 200.0));
    assert_eq!(s.doc.undo_count(), undo + 1);
    let p = canvas_points(&s);
    assert!(
        (p[1].0 - 120.0).abs() < 1.0 && (p[1].1 - 200.0).abs() < 1.0,
        "{p:?}"
    );
    assert_eq!(p.len(), 3);
    assert!(
        pixel(&s, 120, 200)[3] > 0 && pixel(&s, 120, 90)[3] == 0,
        "描き直されている"
    );
    s.apply(Action::Undo);
    let p = canvas_points(&s);
    assert!((p[1].1 - 90.0).abs() < 1.0, "{p:?}");
    assert!(!s.is_stroking());
}

#[test]
fn headless_a_drag_shows_only_in_the_overlay_until_it_is_released() {
    let mut s = state(256);
    for (x, y) in [(30.0, 30.0), (120.0, 90.0), (210.0, 30.0)] {
        click2d(&mut s, x, y);
    }
    let revision = s.doc.revision();
    press2d(&mut s, 120.0, 90.0);
    assert!(
        s.is_stroking(),
        "ドラッグの間は描いている間と同じに、ほかの操作を断る"
    );
    move2d(&mut s, 130.0, 150.0);
    assert_eq!(s.doc.revision(), revision, "離すまで文書は変えない");
    s.apply(Action::NewLayer);
    assert_eq!(s.doc.layers().len(), 2, "ドラッグの間はレイヤーを足せない");
    s.apply(Action::Undo);
    assert_eq!(s.doc.revision(), revision, "ドラッグの間は Undo も断る");
    release2d(&mut s, 130.0, 150.0);
    assert!(s.doc.revision() > revision);
    assert!(!s.is_stroking());
}

#[test]
fn headless_a_click_on_the_curve_inserts_a_point_between_its_neighbours() {
    let mut s = state(256);
    click2d(&mut s, 30.0, 100.0);
    click2d(&mut s, 200.0, 100.0);
    s.apply(Action::Path(PathAction::Point(PointOp::Width {
        index: 0,
        pressure: 0.2,
    })));
    s.apply(Action::Path(PathAction::Point(PointOp::Width {
        index: 1,
        pressure: 0.8,
    })));
    let v = view(&s);
    // 2 点の曲線の上（まっすぐ）
    let mid = v.to_screen(115.0, 100.0);
    pathtool::canvas::press(&mut s, &v, mid, StrokeSource::Mouse);
    pathtool::canvas::release(&mut s, &v, mid, StrokeSource::Mouse);
    let p = canvas_points(&s);
    assert_eq!(p.len(), 3, "曲線の上を押すと、その区間に差し込む");
    assert!((p[1].0 - 115.0).abs() < 1.0);
    assert!((p[1].2 - 0.5).abs() < 1e-9, "太さは両側の平均: {p:?}");
    assert_eq!(s.path_selected_index(), Some(1));
    // 離れた所は、終わりに足す
    click2d(&mut s, 200.0, 220.0);
    let p = canvas_points(&s);
    assert_eq!(p.len(), 4);
    assert!((p[3].1 - 220.0).abs() < 1.0);
}

#[test]
fn headless_delete_close_open_and_width_are_one_undo_each() {
    let mut s = state(256);
    for (x, y) in [
        (40.0, 40.0),
        (200.0, 40.0),
        (230.0, 120.0),
        (200.0, 200.0),
        (40.0, 200.0),
    ] {
        click2d(&mut s, x, y);
    }
    let n = s.doc.undo_count();
    // 閉じる: 始めの点を終わりに足す
    s.apply(Action::Path(PathAction::Point(PointOp::Close)));
    assert_eq!(canvas_points(&s).len(), 6);
    assert!(pathtool::edit::path_is_closed(&path(&s).unwrap()));
    assert_eq!(s.doc.undo_count(), n + 1);
    assert!(pixel(&s, 40, 120)[3] > 0, "閉じた辺（左）が描かれる");
    s.apply(Action::Path(PathAction::Point(PointOp::Close)));
    assert!(s.message.contains("閉じて"), "{}", s.message);
    assert_eq!(s.doc.undo_count(), n + 1, "もう閉じているので何もしない");
    // 閉じたパスの始めを動かすと終わりも動く
    drag2d(&mut s, (40.0, 40.0), (20.0, 20.0));
    let p = canvas_points(&s);
    assert_eq!((p[0].0, p[0].1), (p[5].0, p[5].1));
    assert!((p[0].0 - 20.0).abs() < 1.0);
    // 点の太さ（選んでいる点）
    s.apply(Action::Path(PathAction::Select(Some(1))));
    s.apply(Action::Path(PathAction::Point(PointOp::Width {
        index: 1,
        pressure: 0.25,
    })));
    assert!((canvas_points(&s)[1].2 - 0.25).abs() < 1e-9);
    // 消す: 選んでいる点。閉じたままの輪で消える
    s.apply(Action::Path(PathAction::DeleteSelected));
    assert_eq!(canvas_points(&s).len(), 5);
    assert!(pathtool::edit::path_is_closed(&path(&s).unwrap()));
    // 選んでいる点が無ければ最後の点
    s.apply(Action::Path(PathAction::Select(None)));
    let last = canvas_points(&s)[3];
    s.apply(Action::Path(PathAction::DeleteSelected));
    let p = canvas_points(&s);
    assert_eq!(p.len(), 4, "終わりの複製でなく最後の点を消し、輪のまま");
    assert!(pathtool::edit::path_is_closed(&path(&s).unwrap()));
    assert!(
        p.iter().all(|q| (q.0, q.1) != (last.0, last.1)),
        "最後の点（輪の 4 つ目）が消えた"
    );
    // 3 点の輪から消すと、2 点の開いたパス
    s.apply(Action::Path(PathAction::Point(PointOp::Open)));
    assert!(!pathtool::edit::path_is_closed(&path(&s).unwrap()));
    assert_eq!(canvas_points(&s).len(), 3);
    // 取り消すと逆順に戻る
    s.apply(Action::Undo);
    assert!(pathtool::edit::path_is_closed(&path(&s).unwrap()));
    s.apply(Action::Undo);
    assert_eq!(canvas_points(&s).len(), 5);
}

#[test]
fn headless_removing_every_point_clears_the_pixels_and_keeps_the_path_layer() {
    let mut s = state(128);
    click2d(&mut s, 30.0, 30.0);
    let layer = s.selected_layer.unwrap();
    assert!(pixel(&s, 30, 30)[3] > 0);
    s.apply(Action::Path(PathAction::DeleteSelected));
    assert_eq!(canvas_points(&s).len(), 0);
    assert_eq!(pixel(&s, 30, 30)[3], 0, "点が無ければ画素も無い");
    assert_eq!(s.selected_layer, Some(layer));
    // 点の無いパスにも足せる（同じレイヤーのまま）
    click2d(&mut s, 60.0, 60.0);
    assert_eq!(canvas_points(&s).len(), 1);
    assert_eq!(s.doc.layers().len(), 2);
    // 点の無いパスでの削除・閉じるは何も起こさない
    s.apply(Action::Path(PathAction::DeleteSelected));
    s.apply(Action::Path(PathAction::DeleteSelected));
    let undo = s.doc.undo_count();
    s.apply(Action::Path(PathAction::DeleteSelected));
    s.apply(Action::Path(PathAction::Point(PointOp::Close)));
    assert_eq!(s.doc.undo_count(), undo);
}

#[test]
fn headless_escape_cancels_a_drag_and_losing_focus_commits_up_to_the_last_position() {
    let mut s = state(256);
    for (x, y) in [(40.0, 40.0), (120.0, 120.0), (200.0, 40.0)] {
        click2d(&mut s, x, y);
    }
    let before = canvas_points(&s);
    let undo = s.doc.undo_count();
    // Esc: 点は元の所に残り、ドラッグは残らない
    press2d(&mut s, 120.0, 120.0);
    move2d(&mut s, 150.0, 200.0);
    assert!(s.path.drag.is_some());
    assert_eq!(s.path_selected_index(), Some(1), "掴んだ点は選んでいる");
    assert!(s.path_cancel(1));
    assert!(s.path.drag.is_none() && !s.is_stroking());
    assert_eq!(canvas_points(&s), before);
    assert_eq!(s.doc.undo_count(), undo);
    assert_eq!(
        s.path_selected_index(),
        Some(1),
        "ドラッグを捨てた Esc は、選んだ点を残す"
    );
    // 同じフレームの 2 つ目の Esc（2D と 3D が並んで見えているとき）は、選んだ点を外さず、素通しにもならない
    assert!(s.path_cancel(1), "同じフレームの Esc は 1 回だけ扱う");
    assert_eq!(s.path_selected_index(), Some(1));
    // 次のフレームの Esc で、選んだ点を外す
    assert!(s.path_cancel(2));
    assert_eq!(s.path_selected_index(), None);
    // 点を選んでいなければ、パスの編集を抜ける（次の点は新しいパスを始める）
    let id = path(&s).unwrap().id();
    assert!(s.path_cancel(3));
    assert!(s.path_layer().is_none());
    assert!(!s.path_cancel(4), "何も無ければ、ほかの Esc の扱いへ");
    s.apply(Action::Path(PathAction::SelectPath(Some(id))));
    assert_eq!(path(&s).unwrap().id(), id);
    // フォーカスを失う: そこまでを確定する
    press2d(&mut s, 120.0, 120.0);
    move2d(&mut s, 150.0, 200.0);
    s.path_finish_drag();
    assert!(s.path.drag.is_none() && !s.is_stroking());
    assert!((canvas_points(&s)[1].1 - 200.0).abs() < 1.0);
    assert_eq!(s.doc.undo_count(), undo + 1);
    // ドラッグ中にツールを替えると捨てる
    press2d(&mut s, 150.0, 200.0);
    move2d(&mut s, 10.0, 10.0);
    s.apply(Action::SelectTool(Tool::Brush));
    assert!(s.path.drag.is_none() && s.path.selected.is_none());
    assert!(
        (canvas_points(&s)[1].0 - 150.0).abs() < 1.0,
        "ツールを替えると動かさない"
    );
    // Esc で、選んだ点も外せる（ドラッグが無いとき）
    s.apply(Action::SelectTool(Tool::Path));
    click2d(&mut s, 40.0, 40.0);
    assert_eq!(s.path_selected_index(), Some(0));
    assert!(s.path_cancel(5));
    assert_eq!(s.path_selected_index(), None);
    assert!(s.path_cancel(6), "パスの編集を抜ける");
    assert!(!s.path_cancel(7), "何も無ければ、ほかの Esc の扱いへ");
}

#[test]
fn headless_a_pen_touch_is_a_press_and_a_lift_is_a_release() {
    let mut s = state(256);
    let v = view(&s);
    for (x, y) in [(40.0, 40.0), (120.0, 120.0), (200.0, 40.0)] {
        let p = at(&s, x, y);
        pathtool::canvas::pen_sample(&mut s, &v, p, 1, true);
        // 触れたまま次の点が来ても、点を足し直さない
        pathtool::canvas::pen_sample(&mut s, &v, p, 1, true);
        pathtool::canvas::pen_sample(&mut s, &v, p, 1, false);
    }
    assert_eq!(canvas_points(&s).len(), 3);
    assert!(s.path.pen_down.is_none());
    // ペンで掴んで動かす
    let from = at(&s, 120.0, 120.0);
    let to = at(&s, 120.0, 190.0);
    pathtool::canvas::pen_sample(&mut s, &v, from, 2, true);
    pathtool::canvas::pen_sample(&mut s, &v, to, 2, true);
    pathtool::canvas::pen_sample(&mut s, &v, to, 2, false);
    assert!((canvas_points(&s)[1].1 - 190.0).abs() < 1.0);
    // 別のペンの触れは、動かしている間は無視する
    pathtool::canvas::pen_sample(&mut s, &v, from, 3, true);
    pathtool::canvas::pen_sample(&mut s, &v, from, 4, true);
    assert_eq!(
        s.path.pen_down,
        Some(pathtool::PenDown {
            id: 3,
            surface: false
        })
    );
}

#[test]
fn headless_a_locked_layer_refuses_and_nothing_changes() {
    let mut s = state(256);
    click2d(&mut s, 40.0, 40.0);
    click2d(&mut s, 120.0, 120.0);
    let layer = s.selected_layer.unwrap();
    let before = canvas_points(&s);
    let (undo, revision) = (s.doc.undo_count(), s.doc.revision());
    for (locks, why) in [
        (LayerLocks::PIXELS, "画素"),
        (LayerLocks::TRANSPARENCY, "透明部分"),
        (LayerLocks::ALL, "すべて"),
    ] {
        s.doc.set_layer_locks(layer, locks).unwrap();
        let (undo, revision) = (s.doc.undo_count(), s.doc.revision());
        s.message.clear();
        click2d(&mut s, 200.0, 40.0);
        assert!(s.message.contains(why), "{locks:?}: {}", s.message);
        assert_eq!(canvas_points(&s), before, "{locks:?}");
        assert_eq!((s.doc.undo_count(), s.doc.revision()), (undo, revision));
        // 動かす・消す・太さも断る
        drag2d(&mut s, (120.0, 120.0), (60.0, 200.0));
        s.apply(Action::Path(PathAction::DeleteSelected));
        s.apply(Action::Path(PathAction::Point(PointOp::Close)));
        assert_eq!(canvas_points(&s), before, "{locks:?}");
        assert_eq!((s.doc.undo_count(), s.doc.revision()), (undo, revision));
    }
    // 位置のロックは点の編集（画素を描き直す）を妨げない。画素だけのロックは断られた
    s.doc.set_layer_locks(layer, LayerLocks::POSITION).unwrap();
    click2d(&mut s, 200.0, 40.0);
    assert_eq!(canvas_points(&s).len(), 3);
    assert!(s.doc.undo_count() > undo || s.doc.revision() > revision);
    // 英語
    s.doc.set_layer_locks(layer, LayerLocks::PIXELS).unwrap();
    s.lang = Lang::En;
    s.message.clear();
    click2d(&mut s, 50.0, 200.0);
    assert!(
        s.message.contains("locked") && s.message.contains("pixels"),
        "{}",
        s.message
    );
}

#[test]
fn headless_rasterize_keeps_the_pixels_and_makes_the_layer_paintable_in_one_undo() {
    let mut s = state(256);
    click2d(&mut s, 40.0, 40.0);
    click2d(&mut s, 200.0, 180.0);
    let layer = s.selected_layer.unwrap();
    // パスレイヤーには手で描けない
    let err = s.begin_paint_stroke(layer, false).map(|_| ()).unwrap_err();
    assert!(err.to_string().contains("パス"), "{err}");
    let painted = pixel(&s, 100, 100);
    assert!(pixel(&s, 40, 40)[3] > 0);
    let undo = s.doc.undo_count();
    s.apply(Action::Path(PathAction::Rasterize(layer)));
    assert!(s.doc.layer(layer).unwrap().path().is_none());
    assert_eq!(s.doc.undo_count(), undo + 1);
    assert_eq!(pixel(&s, 100, 100), painted, "今の画素はそのまま");
    assert_eq!(s.path_selected_index(), None);
    // 普通に塗れる
    let stroke = s
        .begin_paint_stroke(layer, false)
        .expect("パスを外したら塗れる");
    s.doc.cancel_stroke(stroke);
    s.apply(Action::Undo);
    assert!(
        s.doc.layer(layer).unwrap().path().is_some(),
        "戻すとパスも戻る"
    );
    // すべてのロックでは外せない
    s.doc.set_layer_locks(layer, LayerLocks::ALL).unwrap();
    s.apply(Action::Path(PathAction::Rasterize(layer)));
    assert!(s.doc.layer(layer).unwrap().path().is_some());
    assert!(s.message.contains("すべて"), "{}", s.message);
}

/// 「新規パスレイヤー」を当てる（レイヤーのメニューと同じ操作）。
fn new_path_layer(s: &mut AppState) {
    s.apply(Action::LayerMenu(yolu_app::layermenu::Op::PathLayer));
}

#[test]
fn headless_a_new_path_layer_is_empty_above_the_selected_layer_and_the_first_click_fills_it() {
    let mut s = AppState::new(128, 128);
    assert_ne!(s.tool, Tool::Path);
    let base = s.selected_layer.unwrap();
    s.apply(Action::NewLayer);
    let top = s.selected_layer.unwrap();
    s.selected_layer = Some(base);
    let undo = s.doc.undo_count();
    new_path_layer(&mut s);
    // 選んでいたレイヤーのすぐ上に、点の無い 1 本のパスのレイヤー。選んで、パスのツールへ替える
    let made = s.selected_layer.unwrap();
    let order: Vec<_> = s.doc.layers().iter().map(|l| l.id()).collect();
    assert_eq!(order, vec![base, made, top], "{}", s.message);
    assert_eq!(s.doc.layers().len(), 3);
    assert_eq!(s.tool, Tool::Path);
    assert_eq!(s.doc.undo_count(), undo + 1, "1 回の Undo");
    assert!(s.modified);
    let layer = s.doc.layer(made).unwrap();
    assert_eq!(layer.paths().len(), 1);
    assert!(layer.has_paths());
    assert_eq!(canvas_points(&s).len(), 0, "点は無い");
    assert!(
        (0..128).all(|x| pixel(&s, x, 64)[3] == 0),
        "点が無ければ画素も無い"
    );
    // 最初の点は、新しいレイヤーを作らずに、このレイヤーのパスへ入る
    click2d(&mut s, 40.0, 40.0);
    assert_eq!(s.doc.layers().len(), 3, "{}", s.message);
    assert_eq!(s.selected_layer, Some(made));
    assert_eq!(canvas_points(&s).len(), 1);
    assert!(pixel(&s, 40, 40)[3] > 0);
    // 1 回ずつ戻る: 点 → レイヤー
    s.apply(Action::Undo);
    assert_eq!(canvas_points(&s).len(), 0);
    s.apply(Action::Undo);
    assert_eq!(s.doc.layers().len(), 2);
    assert!(s.doc.layer(made).is_none());
    s.apply(Action::Redo);
    assert_eq!(s.doc.layers().len(), 3);
    assert!(s.doc.layer(made).unwrap().has_paths());
}

#[test]
fn headless_a_new_path_layer_goes_on_top_without_a_selection_and_inside_the_selected_group() {
    let mut s = AppState::new(64, 64);
    let base = s.selected_layer.unwrap();
    s.selected_layer = None;
    new_path_layer(&mut s);
    let made = s.selected_layer.unwrap();
    assert_eq!(s.doc.layers().last().unwrap().id(), made, "一番上");
    // グループの中のレイヤーを選んでいれば、同じグループの中のすぐ上
    s.apply(Action::M2(yolu_app::m2::Edit::NewGroup));
    let group = s.selected_layer.unwrap();
    s.apply(Action::M2(yolu_app::m2::Edit::Move {
        id: base,
        parent: Some(group),
        position: 0,
    }));
    s.selected_layer = Some(base);
    new_path_layer(&mut s);
    let inside = s.selected_layer.unwrap();
    assert_eq!(s.doc.layer(inside).unwrap().parent(), Some(group));
    let siblings = s.doc.children_of(Some(group)).unwrap();
    assert_eq!(siblings, vec![base, inside]);
}

#[test]
fn headless_the_first_3d_point_turns_a_new_path_layer_into_a_surface_path_in_the_same_layer() {
    let (mut s, rect) = state3d(two_material_plate());
    s.tool = Tool::Brush;
    new_path_layer(&mut s);
    let layer = s.selected_layer.unwrap();
    let id = s.doc.layer(layer).unwrap().paths()[0].id();
    assert_eq!(s.doc.layers().len(), 2);
    assert!(matches!(path(&s), Some(LayerPath::Canvas(_))));
    // 3D の面に最初の点を置くと、同じレイヤー・同じ ID のまま 3D のパスになる（新しいレイヤーは作らない）
    click3d(&mut s, rect, Vec3::new(-0.8, -0.4, 0.0));
    assert_eq!(s.doc.layers().len(), 2, "{}", s.message);
    assert_eq!(s.selected_layer, Some(layer));
    let entries = s.doc.layer(layer).unwrap().paths();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id(), id, "同じパス");
    assert_eq!(surface_path(&s).points.len(), 1);
    assert!(alpha_at(&s, 0.1, 0.30) > 0, "面に投影した画素");
    // 点のあるパスには、別の側の点は足せない（今までどおり）
    s.message.clear();
    click2d(&mut s, 30.0, 30.0);
    assert!(s.message.contains("モデルの上のパス"), "{}", s.message);
    assert_eq!(surface_path(&s).points.len(), 1);
    // 1 回ずつ戻ると、点の無い 2D のパスのレイヤーへ戻る
    s.apply(Action::Undo);
    assert!(matches!(path(&s), Some(LayerPath::Canvas(c)) if c.points.is_empty()));
    assert_eq!(s.doc.layers().len(), 2);
    // 2D の点なら、2D のパスのまま
    click2d(&mut s, 30.0, 30.0);
    assert_eq!(canvas_points(&s).len(), 1);
}

/// パスの編集を抜ける操作（Esc と、プロパティの「新しいパス」）。
fn leave_path_edit(s: &mut AppState, how: &str) {
    match how {
        // 点を選んでいれば、1 回目の Esc は点の選びを外すだけなので、2 回押す（フレームの番号は毎回違う）
        "Esc" => {
            static FRAME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1000);
            for _ in 0..2 {
                s.path_cancel(FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            }
        }
        _ => s.apply(Action::Path(PathAction::SelectPath(None))),
    }
}

#[test]
fn headless_leaving_the_edit_of_a_new_path_layer_keeps_its_one_empty_path_for_the_first_point_in_2d(
) {
    for how in ["Esc", "新しいパス"] {
        let mut s = AppState::new(128, 128);
        new_path_layer(&mut s);
        let layer = s.selected_layer.unwrap();
        let id = s.doc.layer(layer).unwrap().paths()[0].id();
        leave_path_edit(&mut s, how);
        click2d(&mut s, 40.0, 40.0);
        assert_eq!(s.doc.layers().len(), 2, "{how}: {}", s.message);
        let entries = s.doc.layer(layer).unwrap().paths();
        assert_eq!(
            entries.len(),
            1,
            "{how}: 空のパスと新しいパスの 2 本にしない"
        );
        assert_eq!(entries[0].path.point_count(), 1, "{how}");
        assert_eq!(entries[0].id(), id, "{how}: 同じパスに入る");
        assert!(pixel(&s, 40, 40)[3] > 0, "{how}");
        // 点のあるパスでは、抜けると次の点は 2 本目を始める（今までどおり）
        leave_path_edit(&mut s, how);
        click2d(&mut s, 90.0, 90.0);
        assert_eq!(s.doc.layer(layer).unwrap().paths().len(), 2, "{how}");
    }
}

#[test]
fn headless_leaving_the_edit_of_a_new_path_layer_keeps_its_one_empty_path_for_the_first_point_in_3d(
) {
    for how in ["Esc", "新しいパス"] {
        let (mut s, rect) = state3d(two_material_plate());
        new_path_layer(&mut s);
        let layer = s.selected_layer.unwrap();
        let id = s.doc.layer(layer).unwrap().paths()[0].id();
        leave_path_edit(&mut s, how);
        s.message.clear();
        click3d(&mut s, rect, Vec3::new(-0.8, -0.4, 0.0));
        assert_eq!(s.doc.layers().len(), 2, "{how}: {}", s.message);
        let entries = s.doc.layer(layer).unwrap().paths();
        assert_eq!(entries.len(), 1, "{how}: 空のパスを残さない");
        assert_eq!(entries[0].id(), id, "{how}");
        assert_eq!(surface_path(&s).points.len(), 1, "{how}");
        assert!(alpha_at(&s, 0.1, 0.30) > 0, "{how}");
        // 1 回の取り消しで、点の無い 2D のパスへ戻る
        s.apply(Action::Undo);
        assert!(
            matches!(path(&s), Some(LayerPath::Canvas(c)) if c.points.is_empty()),
            "{how}"
        );
    }
}

#[test]
fn headless_leaving_the_edit_after_removing_every_point_also_keeps_one_path() {
    let mut s = state(128);
    click2d(&mut s, 30.0, 30.0);
    let layer = s.selected_layer.unwrap();
    s.apply(Action::Path(PathAction::DeleteSelected));
    assert!(s.path_cancel(3));
    click2d(&mut s, 60.0, 60.0);
    let entries = s.doc.layer(layer).unwrap().paths();
    assert_eq!(entries.len(), 1, "空のパスを残さない: {}", s.message);
    assert_eq!(entries[0].path.point_count(), 1);
}

#[test]
fn headless_removing_every_point_of_a_surface_path_lets_the_next_click_choose_the_side_again() {
    let (mut s, rect) = state3d(two_material_plate());
    click3d(&mut s, rect, Vec3::new(-0.8, -0.4, 0.0));
    let layer = s.selected_layer.unwrap();
    s.apply(Action::Path(PathAction::DeleteSelected));
    assert!(matches!(path(&s), Some(LayerPath::Surface(p)) if p.points.is_empty()));
    click2d(&mut s, 30.0, 30.0);
    assert_eq!(s.doc.layers().len(), 2, "{}", s.message);
    assert_eq!(s.selected_layer, Some(layer));
    assert_eq!(canvas_points(&s).len(), 1);
}

#[test]
fn headless_a_new_path_layer_is_refused_while_stroking_and_in_a_read_only_set() {
    let mut s = AppState::new(64, 64);
    s.sets.get_mut(0).unwrap().read_only = Some("試験".into());
    new_path_layer(&mut s);
    assert_eq!(s.doc.layers().len(), 1);
    assert!(
        s.message.contains("読むだけ") && s.message.contains("試験"),
        "{}",
        s.message
    );
    s.sets.get_mut(0).unwrap().read_only = None;
    // 描いている最中は作らない
    let layer = s.selected_layer.unwrap();
    let settings = s.stroke_settings(false);
    let stroke = s.doc.begin_stroke(layer, &settings).unwrap();
    assert!(s.is_stroking());
    s.message.clear();
    new_path_layer(&mut s);
    assert_eq!(s.doc.layers().len(), 1);
    assert!(!s.message.is_empty());
    s.doc.cancel_stroke(stroke);
}

#[test]
fn headless_a_new_path_layer_survives_saving_and_opening() {
    let dir = std::env::temp_dir().join(format!("yolu-newpathlayer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = AppState::new(64, 64);
    new_path_layer(&mut s);
    let layer = s.selected_layer.unwrap();
    let name = s.doc.layer(layer).unwrap().name().to_owned();
    let file = dir.join("empty-path.ylp");
    s.apply(Action::SaveProjectAs(file.clone()));
    assert!(file.exists(), "{}", s.message);
    let mut t = AppState::new(32, 32);
    t.apply(Action::OpenProject(file));
    let opened = t
        .doc
        .layer(layer)
        .unwrap_or_else(|| panic!("{}", t.message));
    assert_eq!(opened.name(), name);
    assert!(opened.has_paths());
    assert_eq!(
        opened.path().unwrap().point_count(),
        0,
        "点の無いパスのまま"
    );
    // 開いたあとも、パスのツールで点を置ける
    t.apply(Action::SelectTool(Tool::Path));
    t.selected_layer = Some(layer);
    click2d(&mut t, 20.0, 20.0);
    assert_eq!(t.doc.layers().len(), 2, "{}", t.message);
    assert_eq!(canvas_points(&t).len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_masks_read_only_sets_and_foreign_paths_are_refused_with_a_reason() {
    // マスク
    let mut s = state(128);
    let layer = s.selected_layer.unwrap();
    s.apply(Action::M2(yolu_app::m2::Edit::AddMask(layer)));
    s.apply(Action::M2Ui(yolu_app::m2::UiOp::EditMask(true)));
    click2d(&mut s, 30.0, 30.0);
    assert_eq!(s.doc.layers().len(), 1);
    assert!(s.message.contains("マスク"), "{}", s.message);
    s.apply(Action::M2Ui(yolu_app::m2::UiOp::EditMask(false)));
    // 読むだけのセット
    s.sets.get_mut(0).unwrap().read_only = Some("試験".into());
    s.message.clear();
    click2d(&mut s, 30.0, 30.0);
    assert_eq!(s.doc.layers().len(), 1);
    assert!(
        s.message.contains("読むだけ") && s.message.contains("試験"),
        "{}",
        s.message
    );
    s.sets.get_mut(0).unwrap().read_only = None;
    // 2D のパスレイヤーに 3D の点（逆も）は足せない
    click2d(&mut s, 30.0, 30.0);
    s.apply(Action::Path(PathAction::Point(PointOp::Add(
        Place::Surface {
            triangle: 0,
            u: 0.1,
            v: 0.1,
        },
    ))));
    assert_eq!(canvas_points(&s).len(), 1);
    assert!(s.message.contains("キャンバス"), "{}", s.message);
    // 非標準のチャンネルでは描けない（core の理由を日本語・英語で）
    let mut t = state(128);
    t.lang = Lang::En;
    let channel = t
        .doc
        .add_channel(yolu_app::m2::new_channel_info(
            "AO".into(),
            yolu_app::engine::ChannelKind::Scalar,
        ))
        .unwrap();
    t.apply(Action::M2Ui(yolu_app::m2::UiOp::PaintChannel(channel)));
    click2d(&mut t, 30.0, 30.0);
    assert_eq!(t.doc.layers().len(), 1, "{}", t.message);
    assert!(
        t.message.starts_with("Unsupported value or operation"),
        "{}",
        t.message
    );
}

#[test]
fn headless_brush_edits_redraw_the_path_and_without_a_path_they_set_the_next_brush() {
    let mut s = state(256);
    // パスが無い: 今のブラシを替える
    s.apply(Action::Path(PathAction::Brush(BrushEdit::Diameter(40.0))));
    assert_eq!(s.brush.radius, 20.0);
    s.apply(Action::Path(PathAction::Brush(BrushEdit::Hardness(0.25))));
    assert_eq!(s.brush.hardness, 0.25);
    assert_eq!(s.doc.layers().len(), 1);
    // そのブラシでパスを作る
    s.color.set_main([0.9, 0.1, 0.1, 1.0]);
    click2d(&mut s, 60.0, 128.0);
    click2d(&mut s, 200.0, 128.0);
    let Some(LayerPath::Canvas(c)) = path(&s) else {
        panic!()
    };
    assert_eq!((c.brush.0.radius, c.brush.0.hardness), (20.0, 0.25));
    assert_eq!(c.brush.0.color.r, 230);
    let thick = (0..256).filter(|y| pixel(&s, 130, *y)[3] > 0).count();
    // パスのブラシを替えると、描き直す（1 回の Undo）
    let undo = s.doc.undo_count();
    s.apply(Action::Path(PathAction::Brush(BrushEdit::Diameter(10.0))));
    assert_eq!(s.doc.undo_count(), undo + 1);
    let thin = (0..256).filter(|y| pixel(&s, 130, *y)[3] > 0).count();
    assert!(thin < thick, "{thin} < {thick}");
    let Some(LayerPath::Canvas(c)) = path(&s) else {
        panic!()
    };
    assert_eq!(c.brush.0.radius, 5.0);
    assert_eq!(
        s.brush.radius, 20.0,
        "パスのブラシを替えても、今のブラシは変わらない"
    );
    // 同じ値には描き直さない
    s.apply(Action::Path(PathAction::Brush(BrushEdit::Diameter(10.0))));
    assert_eq!(s.doc.undo_count(), undo + 1);
    // 太さの影響の切り替え
    s.apply(Action::Path(PathAction::Brush(BrushEdit::PressureSize(
        false,
    ))));
    let Some(LayerPath::Canvas(c)) = path(&s) else {
        panic!()
    };
    assert!(!c.brush.0.pressure_size);
    s.apply(Action::Undo);
    s.apply(Action::Undo);
    let Some(LayerPath::Canvas(c)) = path(&s) else {
        panic!()
    };
    assert_eq!(c.brush.0.radius, 20.0);
    // 「ブラシを使う」: 今のブラシと色をパスに
    s.brush.radius = 8.0;
    s.color.set_main([0.1, 0.2, 0.9, 1.0]);
    s.apply(Action::Path(PathAction::UseBrush));
    let Some(LayerPath::Canvas(c)) = path(&s) else {
        panic!()
    };
    assert_eq!((c.brush.0.radius, c.brush.0.color.b), (8.0, 230));
    assert!(c.material.is_none());
}

#[test]
fn headless_a_material_path_paints_every_channel_of_the_group_in_one_undo() {
    let mut s = state(128);
    s.apply(Action::Mat(MatAction::Enabled(true)));
    s.apply(Action::Mat(MatAction::Channel(Channel::Roughness, true)));
    s.mat.set_scalar(Channel::Roughness, 0.8);
    click2d(&mut s, 30.0, 64.0);
    click2d(&mut s, 100.0, 64.0);
    let layer = s.selected_layer.unwrap();
    let Some(LayerPath::Canvas(c)) = path(&s) else {
        panic!()
    };
    let channels: Vec<Channel> = c
        .material
        .as_ref()
        .unwrap()
        .iter()
        .map(|p| p.channel)
        .collect();
    assert_eq!(channels, vec![Channel::Color, Channel::Roughness]);
    for channel in [Channel::Color, Channel::Roughness] {
        let px = s
            .doc
            .layer(layer)
            .unwrap()
            .surface(channel)
            .unwrap()
            .pixel(60, 64)
            .unwrap();
        assert!(px.a > 0, "{channel:?}");
    }
    let r = s
        .doc
        .layer(layer)
        .unwrap()
        .surface(Channel::Roughness)
        .unwrap()
        .pixel(60, 64)
        .unwrap();
    assert!((r.r as i32 - 204).abs() <= 1, "ラフネスの値 {r:?}");
    s.apply(Action::Undo);
    assert_eq!(canvas_points(&s).len(), 1);
    // 組を持つパスに「ブラシを使う」で、今の組の値を当てる
    s.mat.set_scalar(Channel::Roughness, 0.2);
    s.apply(Action::Path(PathAction::UseBrush));
    let r = s
        .doc
        .layer(layer)
        .unwrap()
        .surface(Channel::Roughness)
        .unwrap()
        .pixel(30, 64)
        .unwrap();
    assert!((r.r as i32 - 51).abs() <= 1, "{r:?}");
    assert_eq!(s.path_layer().unwrap().1.channels().len(), 2);
    // 組をやめて「ブラシを使う」: 組を外す
    s.apply(Action::Mat(MatAction::Enabled(false)));
    s.apply(Action::Path(PathAction::UseBrush));
    assert_eq!(s.path_layer().unwrap().1.channels(), vec![Channel::Color]);
    assert!(s
        .doc
        .layer(layer)
        .unwrap()
        .surface(Channel::Roughness)
        .is_none_or(|sf| sf.pixel(30, 64).unwrap().a == 0));
}

#[test]
fn headless_a_path_survives_saving_and_opening() {
    let dir = std::env::temp_dir().join(format!("yolu-pathtool-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = state(128);
    s.color.set_main([0.2, 0.7, 0.3, 1.0]);
    for (x, y) in [(20.0, 20.0), (100.0, 40.0), (60.0, 100.0)] {
        click2d(&mut s, x, y);
    }
    s.apply(Action::Path(PathAction::Point(PointOp::Close)));
    s.apply(Action::Path(PathAction::Point(PointOp::Width {
        index: 1,
        pressure: 0.4,
    })));
    let layer = s.selected_layer.unwrap();
    let before = canvas_points(&s);
    let before_pixels: Vec<[u8; 4]> = (0..128).map(|x| pixel(&s, x, 64)).collect();
    let file = dir.join("path.ylp");
    s.apply(Action::SaveProjectAs(file.clone()));
    assert!(file.exists(), "{}", s.message);
    let mut t = AppState::new(64, 64);
    t.apply(Action::OpenProject(file));
    let opened = t.doc.layer(layer).expect("同じレイヤー");
    let Some(LayerPath::Canvas(c)) = opened.path() else {
        panic!("パスが戻らない: {}", t.message)
    };
    assert_eq!(
        c.points
            .iter()
            .map(|p| (p.x, p.y, p.pressure))
            .collect::<Vec<_>>(),
        before
    );
    assert!(pathtool::edit::path_is_closed(&LayerPath::Canvas(
        c.clone()
    )));
    assert_eq!(c.brush.0.color.g, 179);
    let after_pixels: Vec<[u8; 4]> = (0..128).map(|x| pixel(&t, x, 64)).collect();
    assert_eq!(before_pixels, after_pixels, "画素も同じ");
    // 開いたあとも、同じツールで直せる
    t.apply(Action::SelectTool(Tool::Path));
    t.selected_layer = Some(layer);
    let undo = t.doc.undo_count();
    t.apply(Action::Path(PathAction::Point(PointOp::Open)));
    assert_eq!(t.doc.undo_count(), undo + 1, "{}", t.message);
    let _ = std::fs::remove_dir_all(&dir);
}

// ───────── 3D のビュー ─────────

/// 2 つのマテリアルの板（左 x −1〜0 がマテリアル 0、右 0〜1 がマテリアル 1。どちらも z = 0 で、正面から見える）。UV は 0〜1。
fn two_material_plate() -> ViewModel {
    let v = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let uv = |x: f32, y: f32| Vec2::new(x, y);
    let mesh = ModelMesh {
        name: "板".into(),
        positions: vec![
            v(-1.0, -1.0),
            v(0.0, -1.0),
            v(-1.0, 1.0),
            v(0.0, 1.0),
            v(0.0, -1.0),
            v(1.0, -1.0),
            v(0.0, 1.0),
            v(1.0, 1.0),
        ],
        normals: Vec::new(),
        uvs: vec![
            uv(0.0, 0.0),
            uv(0.5, 0.0),
            uv(0.0, 1.0),
            uv(0.5, 1.0),
            uv(0.5, 0.0),
            uv(1.0, 0.0),
            uv(0.5, 1.0),
            uv(1.0, 1.0),
        ],
        submeshes: vec![
            Submesh {
                material: 0,
                indices: vec![0, 2, 1, 2, 3, 1],
            },
            Submesh {
                material: 1,
                indices: vec![4, 6, 5, 6, 7, 5],
            },
        ],
    };
    ViewModel::new(
        "板",
        vec![mesh],
        vec![Some("左".into()), Some("右".into())],
        1,
    )
    .expect("モデル")
}

/// 板に UV の向きだけを替えたもの（指紋が替わる。位置は同じ）。
fn flipped_plate() -> ViewModel {
    let mut m = two_material_plate();
    let mut meshes = m.meshes.clone();
    for uv in &mut meshes[0].uvs {
        uv.x = 1.0 - uv.x;
    }
    m = ViewModel::new("板 2", meshes, m.materials.clone(), 2).expect("モデル");
    m
}

fn state3d(model: ViewModel) -> (AppState, Rect) {
    state3d_sized(model, 256)
}

fn state3d_sized(model: ViewModel, size: u32) -> (AppState, Rect) {
    let mut s = state(size);
    s.view3d.set_model(model);
    s.view3d.material = 0;
    s.view3d.camera.yaw = 0.0;
    s.view3d.camera.pitch = 0.0;
    (
        s,
        Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0)),
    )
}

fn at3d(s: &AppState, rect: Rect, p: Vec3) -> Pos2 {
    let view = s.view3d.camera.view(rect.width(), rect.height());
    let q = view.to_screen(p).expect("カメラの前");
    pos2(rect.left() + q.x, rect.top() + q.y)
}

fn click3d(s: &mut AppState, rect: Rect, p: Vec3) {
    let a = at3d(s, rect, p);
    pathtool::surface::press(s, rect, a, StrokeSource::Mouse);
    pathtool::surface::release(s, rect, a, StrokeSource::Mouse);
}

fn surface_path(s: &AppState) -> core_paths::SurfacePath {
    match path(s) {
        Some(LayerPath::Surface(p)) => p,
        other => panic!("3D のパスが無い: {other:?} {}", s.message),
    }
}

/// 文書の UV の点（0〜1）の Color の画素のアルファ。
fn alpha_at(s: &AppState, u: f32, v: f32) -> u8 {
    pixel(
        s,
        (u * s.doc.width() as f32) as u32,
        (v * s.doc.height() as f32) as u32,
    )[3]
}

#[test]
fn headless_3d_clicks_add_points_on_the_face_and_paint_the_surface_one_undo_each() {
    let (mut s, rect) = state3d(two_material_plate());
    let base = s.selected_layer.unwrap();
    click3d(&mut s, rect, Vec3::new(-0.8, -0.4, 0.0));
    assert_eq!(s.doc.layers().len(), 2, "{}", s.message);
    assert_ne!(s.selected_layer, Some(base));
    click3d(&mut s, rect, Vec3::new(-0.2, 0.5, 0.0));
    let p = surface_path(&s);
    assert_eq!(p.points.len(), 2);
    assert_eq!(s.doc.undo_count(), 2);
    let model = s.view3d.full_model().unwrap().clone();
    assert_eq!(
        p.model_fingerprint,
        core_paths::fingerprint(&model.geometry),
        "モデルの指紋を持つ"
    );
    // 面に投影した画素: 左の板（UV の左半分）にだけ
    assert!(alpha_at(&s, 0.1, 0.30) > 0, "最初の点の近く");
    assert!(alpha_at(&s, 0.4, 0.75) > 0, "2 点目の近く");
    assert_eq!(alpha_at(&s, 0.9, 0.5), 0, "右の板には描かない");
    s.apply(Action::Undo);
    assert_eq!(surface_path(&s).points.len(), 1);
    assert_eq!(alpha_at(&s, 0.4, 0.75), 0);
    s.apply(Action::Undo);
    assert_eq!(s.doc.layers().len(), 1);
    s.apply(Action::Redo);
    s.apply(Action::Redo);
    let redone = s
        .doc
        .layers()
        .iter()
        .find(|l| l.path().is_some())
        .expect("パスレイヤーが戻る");
    assert_eq!(redone.path().unwrap().point_count(), 2);
}

#[test]
fn headless_3d_refuses_other_texture_sets_the_outside_and_a_2d_path_layer() {
    let (mut s, rect) = state3d(two_material_plate());
    // ほかのテクスチャセット（右の板のマテリアル 1）の面
    click3d(&mut s, rect, Vec3::new(0.5, 0.0, 0.0));
    assert_eq!(s.doc.layers().len(), 1);
    assert!(
        s.message.contains("ほかのテクスチャセット") && s.message.contains('右'),
        "{}",
        s.message
    );
    s.lang = Lang::En;
    click3d(&mut s, rect, Vec3::new(0.5, 0.0, 0.0));
    assert!(
        s.message.starts_with("Surface of another texture set"),
        "{}",
        s.message
    );
    // モデルの外
    s.message.clear();
    let a = rect.min + vec2(3.0, 3.0);
    pathtool::surface::press(&mut s, rect, a, StrokeSource::Mouse);
    pathtool::surface::release(&mut s, rect, a, StrokeSource::Mouse);
    assert_eq!(s.message, "Not on the model");
    assert_eq!(s.doc.layers().len(), 1);
    // 2D のパスレイヤーには 3D の点を足さない
    click2d(&mut s, 30.0, 30.0);
    s.message.clear();
    click3d(&mut s, rect, Vec3::new(-0.5, 0.0, 0.0));
    assert!(s.message.contains("canvas path"), "{}", s.message);
    assert_eq!(canvas_points(&s).len(), 1);
    // モデルが無い / 今のセットがモデルに無い
    let mut t = state(128);
    t.apply(Action::Path(PathAction::Point(PointOp::Add(
        Place::Surface {
            triangle: 0,
            u: 0.2,
            v: 0.2,
        },
    ))));
    assert_eq!(t.message, "モデルがありません");
    t.view3d.set_model(two_material_plate());
    t.view3d.material = -1;
    t.apply(Action::Path(PathAction::Point(PointOp::Add(
        Place::Surface {
            triangle: 0,
            u: 0.2,
            v: 0.2,
        },
    ))));
    assert!(
        t.message.contains("このモデルにありません"),
        "{}",
        t.message
    );
    assert_eq!(t.doc.layers().len(), 1);
}

#[test]
fn headless_3d_dragging_a_point_moves_it_across_faces_and_stays_on_this_texture_set() {
    let (mut s, rect) = state3d(two_material_plate());
    click3d(&mut s, rect, Vec3::new(-0.8, -0.5, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.5, 0.5, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.1, -0.5, 0.0));
    let undo = s.doc.undo_count();
    let before = surface_path(&s);
    // 動かさずに離す: 選ぶだけ
    let rev = s.doc.revision();
    click3d(&mut s, rect, Vec3::new(-0.5, 0.5, 0.0));
    assert_eq!((s.path_selected_index(), s.doc.revision()), (Some(1), rev));
    // 掴んで動かす
    let from = at3d(&s, rect, Vec3::new(-0.5, 0.5, 0.0));
    let to = at3d(&s, rect, Vec3::new(-0.3, 0.8, 0.0));
    pathtool::surface::press(&mut s, rect, from, StrokeSource::Mouse);
    assert!(s.path.drag.is_some() && s.is_stroking());
    pathtool::surface::moved(&mut s, rect, from + vec2(30.0, -30.0), StrokeSource::Mouse);
    pathtool::surface::moved(&mut s, rect, to, StrokeSource::Mouse);
    assert_eq!(s.doc.revision(), rev, "離すまで文書は変えない");
    pathtool::surface::release(&mut s, rect, to, StrokeSource::Mouse);
    assert_eq!(s.doc.undo_count(), undo + 1);
    let after = surface_path(&s);
    assert_ne!(before.points[1], after.points[1]);
    assert_eq!(before.points[0], after.points[0]);
    assert_eq!(before.points[1].pressure, after.points[1].pressure);
    assert!(alpha_at(&s, 0.35, 0.9) > 0, "新しい所へ描き直す");
    // 右の板（ほかのテクスチャセット）へは動かせない: 置けない所では前の置き場所のまま、離しても動かさない
    let from = at3d(&s, rect, Vec3::new(-0.3, 0.8, 0.0));
    let right = at3d(&s, rect, Vec3::new(0.6, 0.0, 0.0));
    let n = s.doc.undo_count();
    pathtool::surface::press(&mut s, rect, from, StrokeSource::Mouse);
    pathtool::surface::moved(&mut s, rect, right, StrokeSource::Mouse);
    pathtool::surface::release(&mut s, rect, right, StrokeSource::Mouse);
    assert_eq!(s.doc.undo_count(), n, "{}", s.message);
    // Esc で捨てる・フォーカスを失うと確定
    pathtool::surface::press(&mut s, rect, from, StrokeSource::Mouse);
    let there = at3d(&s, rect, Vec3::new(-0.6, 0.0, 0.0));
    pathtool::surface::moved(&mut s, rect, there, StrokeSource::Mouse);
    assert!(s.path_cancel(9));
    assert!(s.path.drag.is_none());
    assert_eq!(s.doc.undo_count(), n);
    pathtool::surface::press(&mut s, rect, from, StrokeSource::Mouse);
    pathtool::surface::moved(&mut s, rect, there, StrokeSource::Mouse);
    s.path_finish_drag();
    assert!(s.path.drag.is_none());
    assert_eq!(s.doc.undo_count(), n + 1);
}

#[test]
fn headless_3d_clicking_the_curve_inserts_a_surface_point() {
    let (mut s, rect) = state3d(two_material_plate());
    click3d(&mut s, rect, Vec3::new(-0.9, 0.0, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.1, 0.0, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.5, 0.0, 0.0));
    let p = surface_path(&s);
    assert_eq!(p.points.len(), 3, "曲線の上を押すと差し込む");
    // 差し込んだ点は 2 点の間（モデルの x が真ん中）
    let g = &s.view3d.full_model().unwrap().geometry;
    let x = |i: usize| {
        yolu_core::paths::point_position(g, &p.points[i])
            .unwrap()
            .0
            .x
    };
    assert!(x(0) < x(1) && x(1) < x(2), "{} {} {}", x(0), x(1), x(2));
}

#[test]
fn headless_3d_hidden_materials_do_not_shift_the_triangle_numbers_of_the_path() {
    let (mut s, rect) = state3d(two_material_plate());
    // 右の板（マテリアル 1）のセットで描く
    s.view3d.material = 1;
    click3d(&mut s, rect, Vec3::new(0.5, 0.2, 0.0));
    let full = s.view3d.full_model().unwrap().clone();
    let first = surface_path(&s).points[0].triangle;
    assert!(
        full.geometry.triangles()[first as usize].material == 1,
        "受けたままの形の番号"
    );
    // 左の板を隠す: 見せる形は右の板だけ（三角形の番号が 2 ずれる）。それでも点の番号は変わらない
    s.view3d.set_hidden(vec![0]);
    assert_eq!(s.view3d.model.as_ref().unwrap().triangle_count(), 2);
    assert_eq!(s.view3d.full_triangle(0), Some(2));
    assert_eq!(s.view3d.full_triangle(1), Some(3));
    assert_eq!(s.view3d.full_triangle(2), None);
    click3d(&mut s, rect, Vec3::new(0.8, -0.6, 0.0));
    let p = surface_path(&s);
    assert_eq!(p.points.len(), 2, "{}", s.message);
    for pt in &p.points {
        assert!(full.geometry.triangles()[pt.triangle as usize].material == 1);
    }
    assert_eq!(
        p.model_fingerprint,
        core_paths::fingerprint(&full.geometry),
        "指紋は受けたままの形のもの"
    );
    // 隠したままでも描き直せる（受けたままの形で描く）
    s.apply(Action::Path(PathAction::Redraw));
    assert!(alpha_at(&s, 0.8, 0.6) > 0, "{}", s.message);
    // 隠していないセットの目を開けても、同じパス
    s.view3d.set_hidden(vec![]);
    assert_eq!(surface_path(&s), p);
}

#[test]
fn headless_a_path_on_another_model_is_not_edited_and_follows_a_replaced_model() {
    let (mut s, rect) = state3d(two_material_plate());
    click3d(&mut s, rect, Vec3::new(-0.7, -0.3, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.3, 0.4, 0.0));
    let layer = s.selected_layer.unwrap();
    let old_print = surface_path(&s).model_fingerprint;
    let old_alpha = alpha_at(&s, 0.15, 0.35);
    assert!(old_alpha > 0);
    // 別の（UV が違う）モデルに入れ替えて、セットを結び付け直す: 位置が近い面へ付け直して描き直す
    s.view3d.set_model(flipped_plate());
    s.view3d.material = 0;
    s.sync_view3d();
    let p = surface_path(&s);
    assert_ne!(p.model_fingerprint, old_print);
    let new_model = s.view3d.full_model().unwrap().clone();
    assert_eq!(
        p.model_fingerprint,
        core_paths::fingerprint(&new_model.geometry)
    );
    assert_eq!(s.selected_layer, Some(layer));
    assert!(s.message.contains("描き直し"), "{}", s.message);
    // UV を左右に入れ替えたので、画素は反対側（u = 0.85）に描かれる。元の所は空
    assert!(alpha_at(&s, 0.85, 0.35) > 0, "新しい UV の所に描き直した");
    assert_eq!(alpha_at(&s, 0.15, 0.35), 0);
    // 1 回の Undo で前の状態（前のモデルのパスと画素）に戻る
    s.apply(Action::Undo);
    assert_eq!(surface_path(&s).model_fingerprint, old_print);
    assert!(alpha_at(&s, 0.15, 0.35) > 0);
    // 戻した状態は今のモデルと合わない: 編集は断る
    let n = s.doc.undo_count();
    s.message.clear();
    click3d(&mut s, rect, Vec3::new(-0.5, 0.0, 0.0));
    assert_eq!(s.doc.undo_count(), n);
    assert!(s.message.contains("別のモデル"), "{}", s.message);
    s.apply(Action::Path(PathAction::Redraw));
    assert_eq!(s.doc.undo_count(), n, "別のモデルのパスは描き直さない");
}

#[test]
fn headless_a_3d_path_survives_saving_and_opening_and_binds_only_to_the_same_model() {
    let dir = std::env::temp_dir().join(format!("yolu-pathtool3d-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (mut s, rect) = state3d(two_material_plate());
    s.apply(Action::Mat(MatAction::Enabled(true)));
    s.apply(Action::Mat(MatAction::Channel(Channel::Roughness, true)));
    for p in [
        Vec3::new(-0.8, -0.4, 0.0),
        Vec3::new(-0.4, 0.5, 0.0),
        Vec3::new(-0.1, -0.3, 0.0),
    ] {
        click3d(&mut s, rect, p);
    }
    s.apply(Action::Path(PathAction::Point(PointOp::Width {
        index: 1,
        pressure: 0.5,
    })));
    let layer = s.selected_layer.unwrap();
    let before = surface_path(&s);
    let before_alpha = alpha_at(&s, 0.1, 0.3);
    assert!(before_alpha > 0);
    let file = dir.join("path3d.ylp");
    s.apply(Action::SaveProjectAs(file.clone()));
    assert!(file.exists(), "{}", s.message);

    // 同じ形のモデルを読むと、パスは結び付いて、そのまま編集できる
    let mut t = AppState::new(64, 64);
    t.apply(Action::OpenProject(file.clone()));
    let Some(LayerPath::Surface(opened)) = t.doc.layer(layer).and_then(|l| l.path().cloned())
    else {
        panic!("3D のパスが戻らない: {}", t.message)
    };
    assert_eq!(
        opened, before,
        "点・太さ・ブラシ・組・指紋が保存して開いても同じ"
    );
    assert_eq!(alpha_at(&t, 0.1, 0.3), before_alpha, "画素も同じ");
    t.apply(Action::SelectTool(Tool::Path));
    t.selected_layer = Some(layer);
    t.view3d.set_model(two_material_plate());
    t.view3d.material = 0;
    t.view3d.camera.yaw = 0.0;
    t.view3d.camera.pitch = 0.0;
    t.sync_view3d();
    assert_eq!(
        *t.path_fingerprint(&t.view3d.full_model().unwrap().geometry),
        before.model_fingerprint
    );
    let undo = t.doc.undo_count();
    click3d(&mut t, rect, Vec3::new(-0.2, 0.8, 0.0));
    assert_eq!(surface_path(&t).points.len(), 4, "{}", t.message);
    assert_eq!(t.doc.undo_count(), undo + 1);

    // 別の形のモデル（最初に読んだもの）では結び付かない: 編集しない・描き直さない
    let mut u = AppState::new(64, 64);
    u.apply(Action::OpenProject(file));
    u.apply(Action::SelectTool(Tool::Path));
    u.selected_layer = Some(layer);
    u.view3d.set_model(flipped_plate());
    u.view3d.material = 0;
    u.view3d.camera.yaw = 0.0;
    u.view3d.camera.pitch = 0.0;
    let undo = u.doc.undo_count();
    click3d(&mut u, rect, Vec3::new(-0.2, 0.8, 0.0));
    assert_eq!(surface_path(&u).points.len(), 3);
    assert_eq!(u.doc.undo_count(), undo);
    assert!(u.message.contains("別のモデル"), "{}", u.message);
    assert_eq!(surface_path(&u), before, "パスはそのまま残る");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_a_path_that_cannot_follow_the_new_model_becomes_pixels_and_a_locked_one_stays() {
    let (mut s, rect) = state3d(two_material_plate());
    click3d(&mut s, rect, Vec3::new(-0.7, -0.3, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.3, 0.4, 0.0));
    let layer = s.selected_layer.unwrap();
    // 左のマテリアル（0）が新しいモデルに無い: 画素にする
    let old = s.view3d.full_model().unwrap().clone();
    let g = &old.geometry;
    let p = surface_path(&s);
    let far = rebind_far_model();
    let report = pathtool::rebind::rebind_document(&mut s.doc, g, &far.geometry, 0, Lang::Ja);
    assert_eq!(report.len(), 1);
    assert_eq!(
        report[0].outcome,
        pathtool::rebind::Outcome::Rasterized,
        "{report:?}"
    );
    assert!(
        report[0].reason.as_deref().unwrap().contains("点 1"),
        "{report:?}"
    );
    assert!(s.doc.layer(layer).unwrap().path().is_none());
    assert!(alpha_at(&s, 0.15, 0.35) > 0, "画素は残る");
    s.apply(Action::Undo);
    assert!(s.doc.layer(layer).unwrap().path().is_some());
    // 画素のロックでは描き直せないが、画素にもできない（すべてのロックでない）→ 画素にする。すべてのロックでは残す
    s.doc.set_layer_locks(layer, LayerLocks::PIXELS).unwrap();
    let report = pathtool::rebind::rebind_document(&mut s.doc, g, &far.geometry, 0, Lang::En);
    assert_eq!(report[0].outcome, pathtool::rebind::Outcome::Rasterized);
    s.apply(Action::Undo);
    s.doc.set_layer_locks(layer, LayerLocks::ALL).unwrap();
    let report = pathtool::rebind::rebind_document(&mut s.doc, g, &far.geometry, 0, Lang::En);
    assert_eq!(report[0].outcome, pathtool::rebind::Outcome::KeptLocked);
    assert_eq!(report[0].reason.as_deref(), Some("The layer is locked"));
    assert_eq!(
        s.doc.layer(layer).unwrap().path().unwrap(),
        &LayerPath::Surface(p),
        "そのまま残る"
    );
    // 指紋が同じ（ポーズだけ）なら何もしない
    assert!(pathtool::rebind::rebind_document(&mut s.doc, g, g, 0, Lang::Ja).is_empty());
}

/// 板から遠く離れた別のメッシュ（付け直せない）。
fn rebind_far_model() -> ViewModel {
    let mut meshes = two_material_plate().meshes;
    for p in &mut meshes[0].positions {
        p.z += 50.0;
    }
    for uv in &mut meshes[0].uvs {
        uv.x = 1.0 - uv.x;
    }
    ViewModel::new(
        "遠い板",
        meshes,
        vec![Some("左".into()), Some("右".into())],
        3,
    )
    .expect("モデル")
}

/// Live Link で受けた 2 つのマテリアルの板（左がマテリアル 0、右が 1）。flip なら UV の左右を入れ替える（指紋が替わる）。
fn link_plate(generation: u32, flip: bool) -> yolu_protocol::Model {
    use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh as WireSubmesh};
    let info = |name: &str| MaterialInfo {
        key: MaterialKey::Material {
            name: name.into(),
            asset: None,
        },
        shader: String::new(),
        textures: vec![],
        routes: vec![],
    };
    let u = |x: f32| if flip { 1.0 - x } else { x };
    Model {
        generation,
        name: "板".into(),
        materials: vec![info("左"), info("右")],
        meshes: vec![MeshData {
            key: "0".into(),
            name: "板".into(),
            skinned: false,
            positions: vec![
                [-1.0, -1.0, 0.0],
                [0.0, -1.0, 0.0],
                [-1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, -1.0, 0.0],
                [1.0, -1.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            normals: vec![],
            uv0: vec![
                [u(0.0), 0.0],
                [u(0.5), 0.0],
                [u(0.0), 1.0],
                [u(0.5), 1.0],
                [u(0.5), 0.0],
                [u(1.0), 0.0],
                [u(0.5), 1.0],
                [u(1.0), 1.0],
            ],
            submeshes: vec![
                WireSubmesh {
                    material: 0,
                    indices: vec![0, 2, 1, 2, 3, 1],
                },
                WireSubmesh {
                    material: 1,
                    indices: vec![4, 6, 5, 6, 7, 5],
                },
            ],
        }],
    }
}

#[test]
fn headless_replacing_the_live_link_model_redraws_the_paths_of_every_texture_set() {
    let mut s = state(256);
    let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0));
    let (_, shape) = s.receive_link_model(&link_plate(1, false));
    shape.unwrap();
    // 最初のモデルを受けるとカメラはそのモデルの既定の位置になるので、板（外向きの法線は −Z）を正面から見る向きは受けたあとに決める
    // （同じ名前・同じ三角形のモデルを受け直してもカメラは動かない）
    s.view3d.camera.yaw = 0.0;
    s.view3d.camera.pitch = 0.0;
    assert_eq!(s.sets.len(), 2, "マテリアルごとにセット");
    // 左のセット（今のセット）と右のセットに、それぞれ 3D のパス
    click3d(&mut s, rect, Vec3::new(-0.7, -0.3, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.3, 0.4, 0.0));
    let left_layer = s.selected_layer.unwrap();
    let left_uid = s.sets.get(0).unwrap().uid;
    let right_uid = s.sets.get(1).unwrap().uid;
    s.apply(Action::SelectSet(right_uid));
    click3d(&mut s, rect, Vec3::new(0.3, -0.3, 0.0));
    click3d(&mut s, rect, Vec3::new(0.7, 0.4, 0.0));
    let right_layer = s.selected_layer.unwrap();
    let old_print = surface_path(&s).model_fingerprint;
    assert!(alpha_at(&s, 0.65, 0.35) > 0, "右の板の最初の点（u = 0.65）");
    // UV の左右が入れ替わった新しいモデルを受ける: どちらのセットのパスも描き直す
    let (_, shape) = s.receive_link_model(&link_plate(2, true));
    shape.unwrap();
    let new_print = core_paths::fingerprint(&s.view3d.full_model().unwrap().geometry);
    assert_ne!(old_print, new_print);
    // 今のセット（右）: UV が入れ替わったので、画素は反対側に
    let Some(LayerPath::Surface(p)) = s.doc.layer(right_layer).and_then(|l| l.path().cloned())
    else {
        panic!("右のパスが無い: {}", s.message)
    };
    assert_eq!(p.model_fingerprint, new_print);
    assert!(
        alpha_at(&s, 0.35, 0.35) > 0 && alpha_at(&s, 0.65, 0.35) == 0,
        "右のセットの画素が新しい UV の所へ"
    );
    // 今でないセット（左）の文書も付け直す
    s.apply(Action::SelectSet(left_uid));
    let Some(LayerPath::Surface(p)) = s.doc.layer(left_layer).and_then(|l| l.path().cloned())
    else {
        panic!("左のパスが無い")
    };
    assert_eq!(p.model_fingerprint, new_print);
    assert!(
        alpha_at(&s, 0.85, 0.35) > 0 && alpha_at(&s, 0.15, 0.35) == 0,
        "左のセットの画素も新しい UV の所へ"
    );
    assert!(s.message.contains("パス 2 本を描き直し"), "{}", s.message);
    // 1 回の Undo（セットごと）で前のモデルのパスに戻る
    s.apply(Action::Undo);
    assert_eq!(surface_path(&s).model_fingerprint, old_print);
    // 同じ形のモデルをもう 1 度受けても、何もしない
    let (_, shape) = s.receive_link_model(&link_plate(3, true));
    shape.unwrap();
    let undo = s.doc.undo_count();
    let (_, shape) = s.receive_link_model(&link_plate(4, true));
    shape.unwrap();
    assert_eq!(s.doc.undo_count(), undo, "指紋が同じなら付け直さない");
}

#[test]
fn headless_posing_does_not_unbind_a_path_and_a_same_shape_model_is_not_a_replacement() {
    let (mut s, rect) = state3d(two_material_plate());
    click3d(&mut s, rect, Vec3::new(-0.7, -0.3, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.3, 0.4, 0.0));
    let before = surface_path(&s);
    // 位置だけが替わったモデル（ポーズ）
    let mut meshes = two_material_plate().meshes;
    for p in &mut meshes[0].positions {
        p.x += 0.1;
    }
    let posed =
        ViewModel::new("板", meshes, vec![Some("左".into()), Some("右".into())], 5).unwrap();
    s.view3d.set_model(posed);
    assert!(
        s.view3d.take_replaced().is_none(),
        "三角形・UV が同じなら差し替えではない"
    );
    s.sync_view3d();
    assert_eq!(surface_path(&s), before, "パスはそのまま");
    assert!(
        s.message.is_empty() || !s.message.contains("差し替え"),
        "{}",
        s.message
    );
    // 再描き直しは、今のポーズの面に描く
    s.apply(Action::Path(PathAction::Redraw));
    assert!(alpha_at(&s, 0.15, 0.35) > 0);
}

/// 選んでいるレイヤーの Color の画素（文書の並び）。
fn layer_color_bytes(s: &AppState) -> Vec<u8> {
    let id = s.selected_layer.unwrap();
    s.doc
        .layer(id)
        .unwrap()
        .surface(Channel::Color)
        .unwrap()
        .canvas_bytes()
        .unwrap()
}

#[test]
fn headless_a_3d_path_is_drawn_on_the_rest_shape_so_a_pose_does_not_change_its_pixels() {
    let (mut s, rect) = state3d(two_material_plate());
    click3d(&mut s, rect, Vec3::new(-0.8, -0.5, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.5, 0.3, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.2, -0.2, 0.0));
    let before = layer_color_bytes(&s);
    assert!(before.chunks(4).any(|p| p[3] > 0), "描いた: {}", s.message);
    let rest = s.view3d.full_model().unwrap().geometry.clone();
    // 左の面だけを横に 1.8 倍に伸ばしたポーズ（UV は同じ。ポーズの形で描くと UV の上の太さが変わる）
    let mut meshes = two_material_plate().meshes;
    for p in &mut meshes[0].positions[..4] {
        p.x = -1.0 + (p.x + 1.0) * 1.8;
    }
    let names = vec![Some("左".into()), Some("右".into())];
    let posed = ViewModel::new("板", meshes.clone(), names.clone(), 5)
        .unwrap()
        .with_rest(rest.clone());
    s.view3d.set_model(posed);
    assert!(s.view3d.take_replaced().is_none());
    s.sync_view3d();
    s.apply(Action::Path(PathAction::Redraw));
    assert_eq!(
        layer_color_bytes(&s),
        before,
        "休みの形で描くので、ポーズを変えても同じ画素: {}",
        s.message
    );
    // ブラシの直径の換算も休みの形（ポーズで大きさが変わらない）
    s.apply(Action::Path(PathAction::Brush(BrushEdit::Diameter(20.0))));
    let unit = yolu_core::geometry::world_radius(&rest, 1.0, s.doc.width()) as f64;
    assert_eq!(surface_path(&s).brush.0.radius, 10.0 * unit);
    s.apply(Action::Undo);
    // 休みの形を持たない形（ポーズの区別が無い Live Link の古い口の最初の形など）は、その形が休みの形
    let unposed = ViewModel::new("板", meshes, names, 6).unwrap();
    s.view3d.set_model(unposed);
    s.sync_view3d();
    s.apply(Action::Path(PathAction::Redraw));
    assert_ne!(
        layer_color_bytes(&s),
        before,
        "伸ばした形を休みの形として描けば、太さが変わる（上の確かめが空振りでないこと）"
    );
}

#[test]
fn headless_a_posed_figure_redraws_its_path_on_the_rest_pose() {
    use yolu_app::view3d::pose::{self, PoseAction};
    let mut s = state(128);
    s.apply(Action::Pose(PoseAction::LoadFigure));
    assert!(s.view3d.pose.session.is_some(), "{}", s.message);
    s.sync_view3d();
    let model = s.view3d.full_model().unwrap().clone();
    let material = s.view3d.material;
    // 描くテクスチャセットの三角形から、離れた 3 つ
    let picks: Vec<u32> = (0..model.geometry.triangle_count() as u32)
        .filter(|&t| model.geometry.triangles()[t as usize].material == material)
        .step_by(97)
        .take(3)
        .collect();
    assert_eq!(picks.len(), 3);
    for t in picks {
        s.apply(Action::Path(PathAction::Point(PointOp::Add(
            Place::Surface {
                triangle: t,
                u: 0.3,
                v: 0.3,
            },
        ))));
    }
    assert_eq!(surface_path(&s).points.len(), 3, "{}", s.message);
    let before = layer_color_bytes(&s);
    assert!(before.chunks(4).any(|p| p[3] > 0));
    // すべての骨を回す
    let session = s.view3d.pose.session.as_ref().unwrap();
    let mut p = session.pose().clone();
    for l in &mut p.locals {
        l.rotation = yolu_core::glam::Quat::from_rotation_z(0.35) * l.rotation;
    }
    pose::set_pose(&mut s.view3d, p).unwrap();
    s.sync_view3d();
    assert!(s.view3d.pose.session.as_ref().unwrap().is_posed());
    s.apply(Action::Path(PathAction::Redraw));
    assert_eq!(layer_color_bytes(&s), before, "{}", s.message);
}

#[test]
fn headless_a_3d_pen_touch_is_a_press_and_a_lift_is_a_release() {
    let (mut s, rect) = state3d(two_material_plate());
    for p in [Vec3::new(-0.8, -0.4, 0.0), Vec3::new(-0.4, 0.5, 0.0)] {
        let a = at3d(&s, rect, p);
        pathtool::surface::pen_sample(&mut s, rect, a, 1, true, true);
        pathtool::surface::pen_sample(&mut s, rect, a, 1, true, true);
        pathtool::surface::pen_sample(&mut s, rect, a, 1, false, true);
    }
    assert_eq!(surface_path(&s).points.len(), 2);
    assert!(s.path.pen_down.is_none());
    // 押してよい所でなければ触れても始めない
    let a = at3d(&s, rect, Vec3::new(-0.2, 0.0, 0.0));
    pathtool::surface::pen_sample(&mut s, rect, a, 1, true, false);
    assert!(s.path.pen_down.is_none());
    assert_eq!(surface_path(&s).points.len(), 2);
}

/// 2D のキャンバスと 3D のビューが並んで見えているとき、同じペンの 1 点が両方のビューに渡る（どちらが先かはドックの並び次第。
/// `over_*` はその点がそのビューの上か。呼ぶ条件は `canvas/mod.rs`・`view3d/input.rs` と同じ）。
fn pen_in_both_views(
    s: &mut AppState,
    rect: Rect,
    at: Pos2,
    id: u32,
    contact: bool,
    over: (bool, bool),
    surface_first: bool,
) {
    let canvas = |s: &mut AppState| {
        if s.path.pen_in(false) || over.0 || !contact {
            let v = view(s);
            pathtool::canvas::pen_sample(s, &v, at, id, contact);
        }
    };
    let surface = |s: &mut AppState| {
        if over.1 || s.path.pen_in(true) || !contact {
            pathtool::surface::pen_sample(s, rect, at, id, contact, over.1);
        }
    };
    if surface_first {
        surface(s);
        canvas(s);
    } else {
        canvas(s);
        surface(s);
    }
}

#[test]
fn headless_a_pen_lift_seen_by_the_other_view_does_not_strand_the_drag() {
    for surface_first in [false, true] {
        let order = if surface_first {
            "3D が先"
        } else {
            "2D が先"
        };
        // 3D で触れて点を掴み、離したサンプルを 2D が見ても、3D が離したのを受け取って確定する
        let (mut s, rect) = state3d(two_material_plate());
        click3d(&mut s, rect, Vec3::new(-0.7, -0.3, 0.0));
        click3d(&mut s, rect, Vec3::new(-0.3, 0.4, 0.0));
        let before = surface_path(&s);
        let undo = s.doc.undo_count();
        let from = at3d(&s, rect, Vec3::new(-0.3, 0.4, 0.0));
        let to = at3d(&s, rect, Vec3::new(-0.6, 0.6, 0.0));
        let both = |s: &mut AppState, at: Pos2, contact: bool, over: (bool, bool)| {
            pen_in_both_views(s, rect, at, 1, contact, over, surface_first)
        };
        both(&mut s, from, true, (false, true));
        assert!(
            s.path.drag.is_some_and(|d| d.surface),
            "{order}: 3D のドラッグ"
        );
        assert_eq!(s.path.pen_down.map(|p| p.surface), Some(true), "{order}");
        // 触れたまま 2D のキャンバスの上へ動いても、触れた 3D のビューが続ける
        both(&mut s, to, true, (true, false));
        assert_eq!(s.path.pen_down.map(|p| p.surface), Some(true), "{order}");
        both(&mut s, to, false, (true, false));
        assert!(
            s.path.drag.is_none() && s.path.pen_down.is_none(),
            "{order}"
        );
        assert!(!s.is_stroking(), "{order}: 離したあとに取り残さない");
        assert_eq!(
            s.doc.undo_count(),
            undo + 1,
            "{order}: 動かしたのは 1 回の Undo"
        );
        assert_ne!(surface_path(&s).points[1], before.points[1], "{order}");

        // 2D で触れて点を掴み、離したサンプルを 3D が見ても、2D が離したのを受け取って確定する
        let mut t = state(256);
        t.view3d.set_model(two_material_plate());
        t.view3d.material = 0;
        for (x, y) in [(40.0, 40.0), (120.0, 120.0), (200.0, 40.0)] {
            click2d(&mut t, x, y);
        }
        let undo = t.doc.undo_count();
        let both = |t: &mut AppState, at: Pos2, contact: bool, over: (bool, bool)| {
            pen_in_both_views(t, rect, at, 2, contact, over, surface_first)
        };
        let (from, to) = (at(&t, 120.0, 120.0), at(&t, 120.0, 190.0));
        both(&mut t, from, true, (true, false));
        assert!(
            t.path.drag.is_some_and(|d| !d.surface),
            "{order}: 2D のドラッグ"
        );
        assert_eq!(t.path.pen_down.map(|p| p.surface), Some(false), "{order}");
        both(&mut t, to, true, (true, false));
        both(&mut t, to, false, (true, false));
        assert!(
            t.path.drag.is_none() && t.path.pen_down.is_none(),
            "{order}"
        );
        assert!(!t.is_stroking(), "{order}: 離したあとに取り残さない");
        assert_eq!(t.doc.undo_count(), undo + 1, "{order}");
        assert!((canvas_points(&t)[1].1 - 190.0).abs() < 1.0, "{order}");
        // 触れたまま 3D のビューの上へ動いても、触れた 2D のビューが続ける（3D のビューは新しく始めない）
        let (from, to) = (at(&t, 120.0, 190.0), at(&t, 120.0, 100.0));
        both(&mut t, from, true, (true, false));
        both(&mut t, to, true, (false, true));
        assert_eq!(t.path.pen_down.map(|p| p.surface), Some(false), "{order}");
        both(&mut t, to, false, (false, true));
        assert!(
            t.path.drag.is_none() && t.path.pen_down.is_none(),
            "{order}"
        );
        assert!(!t.is_stroking(), "{order}");
        assert_eq!(t.doc.undo_count(), undo + 2, "{order}");
        assert!((canvas_points(&t)[1].1 - 100.0).abs() < 1.0, "{order}");
    }
}

#[test]
fn headless_a_material_3d_path_uses_the_material_group_and_the_brush_size_follows_the_model() {
    let (mut s, rect) = state3d(two_material_plate());
    s.apply(Action::Mat(MatAction::Enabled(true)));
    s.apply(Action::Mat(MatAction::Channel(Channel::Metallic, true)));
    s.mat.set_scalar(Channel::Metallic, 1.0);
    s.apply(Action::Path(PathAction::Brush(BrushEdit::Diameter(24.0))));
    click3d(&mut s, rect, Vec3::new(-0.8, 0.0, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.2, 0.0, 0.0));
    let p = surface_path(&s);
    assert_eq!(p.material.as_ref().unwrap().len(), 2);
    let g = &s.view3d.full_model().unwrap().geometry;
    let unit = yolu_core::geometry::world_radius(g, 1.0, s.doc.width());
    assert!(
        (p.brush.0.radius as f32 - 12.0 * unit).abs() < 1e-6,
        "半径はモデルの大きさに合わせる（{} と {}）",
        p.brush.0.radius,
        12.0 * unit
    );
    let layer = s.selected_layer.unwrap();
    let metallic = s
        .doc
        .layer(layer)
        .unwrap()
        .surface(Channel::Metallic)
        .unwrap()
        .pixel(64, 128)
        .unwrap();
    assert!(metallic.a > 0 && metallic.r > 250, "{metallic:?}");
    // 直径のスライダーの値はモデルから画素に戻す（太さを替えると描き直す）
    s.apply(Action::Path(PathAction::Brush(BrushEdit::Diameter(48.0))));
    assert!((surface_path(&s).brush.0.radius as f32 - 24.0 * unit).abs() < 1e-6);
    s.apply(Action::Path(PathAction::Brush(BrushEdit::Spacing(0.5))));
    assert_eq!(surface_path(&s).brush.0.spacing, 0.5);
}

#[test]
fn headless_the_tool_is_in_the_strip_with_a_key_and_both_languages() {
    assert!(Tool::ALL.contains(&Tool::Path));
    assert_eq!(Tool::Path.key(), "P");
    assert_eq!(Tool::Path.id(), "path");
    assert_ne!(Tool::Path.name_in(Lang::Ja), Tool::Path.name_in(Lang::En));
    let mut s = AppState::new(64, 64);
    for lang in Lang::ALL {
        s.lang = lang;
        for refusal in [
            yolu_app::pathtool::edit::Refusal::TooMany,
            yolu_app::pathtool::edit::Refusal::NeedThree,
            yolu_app::pathtool::edit::Refusal::AlreadyClosed,
            yolu_app::pathtool::edit::Refusal::NotClosed,
            yolu_app::pathtool::edit::Refusal::NoPoint,
            yolu_app::pathtool::edit::Refusal::OtherKind,
        ] {
            assert!(!yolu_app::lang::refusals::path_edit(lang, refusal).is_empty());
        }
    }
    let ja =
        yolu_app::lang::refusals::path_edit(Lang::Ja, yolu_app::pathtool::edit::Refusal::NeedThree);
    let en =
        yolu_app::lang::refusals::path_edit(Lang::En, yolu_app::pathtool::edit::Refusal::NeedThree);
    assert!(ja != en && en.is_ascii());
    let _ = LayerId(0);
}

// ───────── 評価の失敗・遮り・モデルの入れ替え中のドラッグ ─────────

fn add_canvas(s: &mut AppState, x: f64, y: f64) {
    s.apply(Action::Path(PathAction::Point(PointOp::Add(
        Place::Canvas { x, y },
    ))));
}

fn add_surface(s: &mut AppState, triangle: u32, u: f64, v: f64) {
    s.apply(Action::Path(PathAction::Point(PointOp::Add(
        Place::Surface { triangle, u, v },
    ))));
}

/// 文書の「何も変わらない」の確かめに使う（レイヤーの数・Undo の数・版・選んでいるレイヤーのパスと点）。
fn doc_state(
    s: &AppState,
) -> (
    usize,
    usize,
    impl PartialEq + std::fmt::Debug,
    Option<LayerPath>,
    Option<usize>,
) {
    (
        s.doc.layers().len(),
        s.doc.undo_count(),
        s.doc.revision(),
        path(s),
        s.path_selected_index(),
    )
}

/// 失敗した操作が、文書・選んだ点を変えず、ドラッグも残さず、画面の言語で短い理由を出す。
fn assert_refused_with(s: &mut AppState, words: [(Lang, &str); 2], op: impl Fn(&mut AppState)) {
    for (lang, word) in words {
        s.lang = lang;
        let before = doc_state(s);
        s.message.clear();
        op(s);
        assert_eq!(doc_state(s), before, "{lang:?}: 文書は変わらない");
        assert!(
            s.message.contains(word),
            "{lang:?}: 「{word}」が無い: {}",
            s.message
        );
        assert!(s.path.drag.is_none() && !s.is_stroking(), "{lang:?}");
    }
    s.lang = Lang::Ja;
}

#[test]
fn headless_a_failed_evaluation_changes_nothing_and_says_why_in_both_languages() {
    // 2D、パスのあるレイヤー: 直径 1・間隔 1% の細かい標本が、幅の広いキャンバスの端から端で上限（400 万）を超える
    let mut s = AppState::new(12000, 16);
    s.apply(Action::SelectTool(Tool::Path));
    s.apply(Action::Path(PathAction::Brush(BrushEdit::Diameter(1.0))));
    s.apply(Action::Path(PathAction::Brush(BrushEdit::Spacing(0.01))));
    add_canvas(&mut s, 10.0, 8.0);
    assert!(path(&s).is_some(), "{}", s.message);
    assert_eq!(canvas_points(&s).len(), 1);
    assert_refused_with(
        &mut s,
        [(Lang::Ja, "長すぎ"), (Lang::En, "too long")],
        |s| add_canvas(s, 11990.0, 8.0),
    );
    assert_eq!(canvas_points(&s).len(), 1);

    // 2D、新しいレイヤー: 画素の予算が足りない（作りかけのレイヤーを残さない）
    let mut t = state(128);
    t.doc.set_source_budget_bytes(0).unwrap();
    assert_refused_with(&mut t, [(Lang::Ja, "予算"), (Lang::En, "budget")], |t| {
        add_canvas(t, 30.0, 30.0)
    });
    assert_eq!(t.doc.layers().len(), 1);
    assert!(path(&t).is_none());

    // 2D、パスのあるレイヤー: 進行中のストロークの巻き戻しの予算が足りない
    let mut u = state(128);
    add_canvas(&mut u, 30.0, 30.0);
    add_canvas(&mut u, 100.0, 60.0);
    u.doc.set_stroke_budget_bytes(1).unwrap();
    assert_refused_with(&mut u, [(Lang::Ja, "予算"), (Lang::En, "budget")], |u| {
        add_canvas(u, 60.0, 100.0)
    });
    assert_eq!(canvas_points(&u).len(), 2);
    assert_refused_with(&mut u, [(Lang::Ja, "予算"), (Lang::En, "budget")], |u| {
        u.apply(Action::Path(PathAction::Redraw))
    });

    // 3D: 三角形がモデルに無い点（パスのあるレイヤー・新しいレイヤー）
    let (mut v, rect) = state3d(two_material_plate());
    assert_refused_with(
        &mut v,
        [(Lang::Ja, "三角形"), (Lang::En, "triangle")],
        |v| add_surface(v, 99_999, 0.1, 0.1),
    );
    assert_eq!(v.doc.layers().len(), 1, "新しいレイヤーを作らない");
    click3d(&mut v, rect, Vec3::new(-0.7, -0.3, 0.0));
    click3d(&mut v, rect, Vec3::new(-0.3, 0.4, 0.0));
    assert_eq!(surface_path(&v).points.len(), 2);
    assert_refused_with(
        &mut v,
        [(Lang::Ja, "三角形"), (Lang::En, "triangle")],
        |v| add_surface(v, 99_999, 0.1, 0.1),
    );
    // 3D: ブラシの間隔に対してパスが長すぎる（直径をごく小さく・間隔を 1% に）
    v.apply(Action::Path(PathAction::Brush(BrushEdit::Spacing(0.01))));
    assert_eq!(surface_path(&v).brush.0.spacing, 0.01, "{}", v.message);
    assert_refused_with(
        &mut v,
        [(Lang::Ja, "長すぎ"), (Lang::En, "too long")],
        |v| v.apply(Action::Path(PathAction::Brush(BrushEdit::Diameter(1e-5)))),
    );
    assert_eq!(surface_path(&v).points.len(), 2);
}

#[test]
fn headless_the_point_limit_refuses_through_the_path_action_in_2d_and_3d() {
    use core_paths::{CanvasPoint, PathPoint, MAX_POINTS};
    // 2D: 4096 点のパスに、足す・差し込むを断る（Undo・画素は変えない）
    let mut s = state(256);
    click2d(&mut s, 40.0, 40.0);
    click2d(&mut s, 200.0, 40.0);
    let layer = s.selected_layer.unwrap();
    let Some(LayerPath::Canvas(mut c)) = path(&s) else {
        panic!()
    };
    c.points = (0..MAX_POINTS)
        .map(|i| {
            let (row, col) = (i / 200, i % 200);
            let x = 20.0 + if row % 2 == 0 { col } else { 199 - col } as f64;
            CanvasPoint::new(x, 20.0 + row as f64 * 5.0, 1.0).unwrap()
        })
        .collect();
    s.doc.set_canvas_path(layer, c).unwrap();
    assert_eq!(canvas_points(&s).len(), MAX_POINTS);
    assert_refused_with(&mut s, [(Lang::Ja, "4096"), (Lang::En, "4096")], |s| {
        add_canvas(s, 230.0, 230.0)
    });
    assert_refused_with(&mut s, [(Lang::Ja, "4096"), (Lang::En, "4096")], |s| {
        s.apply(Action::Path(PathAction::Point(PointOp::Insert {
            segment: 0,
            place: Place::Canvas { x: 30.0, y: 30.0 },
        })))
    });
    assert_eq!(canvas_points(&s).len(), MAX_POINTS);

    // 3D
    let (mut t, rect) = state3d(two_material_plate());
    click3d(&mut t, rect, Vec3::new(-0.7, -0.3, 0.0));
    click3d(&mut t, rect, Vec3::new(-0.3, 0.4, 0.0));
    let layer = t.selected_layer.unwrap();
    let mut p = surface_path(&t);
    let first = p.points[0];
    p.points = (0..MAX_POINTS)
        .map(|i| PathPoint {
            u: 0.05 + (i % 100) as f64 * 0.002,
            v: 0.2 + (i / 100) as f64 * 0.001,
            ..first
        })
        .collect();
    let model = t.view3d.full_model().unwrap().clone();
    let rendered = core_paths::render_surface(
        &p,
        &model.geometry,
        &core_paths::Options {
            width: t.doc.width(),
            height: t.doc.height(),
            tile_size: t.doc.tile_size(),
            ..Default::default()
        },
    )
    .expect("4096 点でも描ける");
    t.doc
        .set_path(layer, LayerPath::Surface(p), rendered.channels)
        .unwrap();
    assert_eq!(surface_path(&t).points.len(), MAX_POINTS);
    assert_refused_with(&mut t, [(Lang::Ja, "4096"), (Lang::En, "4096")], |t| {
        add_surface(t, first.triangle, 0.3, 0.3)
    });
    assert_eq!(surface_path(&t).points.len(), MAX_POINTS);
}

#[test]
fn headless_every_evaluation_failure_has_a_short_reason_in_both_languages() {
    use yolu_core::geometry::DabRefusal;
    use yolu_core::CoreError;
    let errors = [
        core_paths::Error::Invalid("2D の点は有限の ±1000000 画素、筆圧は 0..1 です"),
        core_paths::Error::ModelMismatch,
        core_paths::Error::MissingTriangle,
        core_paths::Error::TooManySamples,
        core_paths::Error::Canceled,
        core_paths::Error::Core(CoreError::SourceBudgetExceeded),
        core_paths::Error::Core(CoreError::StrokeBudgetExceeded),
        core_paths::Error::Dab(DabRefusal::PixelBudget),
        core_paths::Error::Dab(DabRefusal::TriangleBudget),
    ];
    for error in &errors {
        let (ja, en) = (
            yolu_app::lang::path_error(Lang::Ja, error),
            yolu_app::lang::path_error(Lang::En, error),
        );
        assert!(
            has_japanese(&ja) && ja.chars().count() <= 40,
            "{error:?}: {ja}"
        );
        assert!(
            en.is_ascii() && en.len() <= 70 && ja != en,
            "{error:?}: {en}"
        );
        assert!(!ja.ends_with('。') && !en.ends_with('.'), "{error:?}");
    }
    // 文の中身も確かめる（種類ごとに別の理由）
    let text = |e: core_paths::Error| yolu_app::lang::path_error(Lang::En, &e);
    assert!(text(core_paths::Error::ModelMismatch).contains("model"));
    assert!(text(core_paths::Error::MissingTriangle).contains("triangle"));
    assert!(text(core_paths::Error::TooManySamples).contains("too long"));
    assert!(text(core_paths::Error::Core(CoreError::SourceBudgetExceeded)).contains("budget"));
}

/// 奥の板（マテリアル 0、x −1〜1、z = 0）と、その手前（`front` の z）で右側を遮る板（マテリアル 1、x 0.1〜1）。
/// 奥の板の三角形は 0（頂点 (-1,-1)・(-1,1)・(1,-1)）と 1（頂点 (-1,1)・(1,1)・(1,-1)）。
fn plate_behind_blocker(front: f32) -> ViewModel {
    let quad = |name: &str, z: f32, x0: f32, material: i32| ModelMesh {
        name: name.into(),
        positions: vec![
            Vec3::new(x0, -1.0, z),
            Vec3::new(1.0, -1.0, z),
            Vec3::new(x0, 1.0, z),
            Vec3::new(1.0, 1.0, z),
        ],
        normals: Vec::new(),
        uvs: vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
        ],
        submeshes: vec![Submesh {
            material,
            indices: vec![0, 2, 1, 2, 3, 1],
        }],
    };
    ViewModel::new(
        "板",
        vec![quad("奥", 0.0, -1.0, 0), quad("手前", front, 0.1, 1)],
        vec![Some("左".into()), Some("右".into())],
        1,
    )
    .expect("モデル")
}

/// 正面から見て、奥の板の右側が手前の板に隠れる状態。
fn occlusion_state() -> (AppState, Rect) {
    // 遮蔽と点の印はモデル・カメラ・点数で決まる。画素への再描画は小さい文書で行う。
    let size = 32;
    let (probe, rect) = state3d_sized(two_material_plate(), size);
    let camera = probe.view3d.camera.view(rect.width(), rect.height());
    let front = if camera.position.z >= 0.0 { 0.5 } else { -0.5 };
    state3d_sized(plate_behind_blocker(front), size)
}

/// 重ね表示が描いた点の印（中心と塗りの色）。
fn marker_fills(s: &AppState, rect: Rect) -> Vec<(Pos2, egui::Color32)> {
    fn walk(shape: &Shape, out: &mut Vec<(Pos2, egui::Color32)>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            Shape::Rect(r) if r.fill != egui::Color32::TRANSPARENT => {
                out.push((r.rect.center(), r.fill))
            }
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        pathtool::surface::paint_overlay(&ui.painter_at(rect), s, rect, None);
    });
    output.textures_delta.clear();
    let mut out = Vec::new();
    for shape in &output.shapes {
        walk(&shape.shape, &mut out);
    }
    out
}

fn fill_at(fills: &[(Pos2, egui::Color32)], at: Pos2) -> egui::Color32 {
    fills
        .iter()
        .find(|(p, _)| p.distance(at) < 1.0)
        .map(|(_, c)| *c)
        .unwrap_or_else(|| panic!("{at:?} に印が無い: {fills:?}"))
}

#[test]
fn headless_3d_points_behind_the_model_are_dimmed_and_not_grabbed_up_to_256_points() {
    let dim = pathtool::PATH_COLOR.gamma_multiply(0.35);
    let (mut s, rect) = occlusion_state();
    // 奥の板の上に、見える点 A・手前の板に遮られる点 C・見える点 B（C が B より前の 2 番目）
    add_surface(&mut s, 0, 0.5, 0.25); // (-0.5, 0)
    add_surface(&mut s, 1, 0.25, 0.5); // (0.5, 0)
    add_surface(&mut s, 1, 0.05, 0.2); // (-0.5, 0.6)
    assert_eq!(surface_path(&s).points.len(), 3, "{}", s.message);
    let a = at3d(&s, rect, Vec3::new(-0.5, 0.0, 0.0));
    let c = at3d(&s, rect, Vec3::new(0.5, 0.0, 0.0));
    let b = at3d(&s, rect, Vec3::new(-0.5, 0.6, 0.0));
    let fills = marker_fills(&s, rect);
    assert_eq!(fill_at(&fills, a), pathtool::PATH_COLOR, "見える点");
    assert_eq!(
        fill_at(&fills, b),
        egui::Color32::WHITE,
        "選んでいる点（最後）"
    );
    assert_eq!(fill_at(&fills, c), dim, "遮られる点は薄く出す");

    // 遮られる点は掴まない（足す・差し込む先も手前の板で、ほかのテクスチャセットの面として断る）
    let (undo, points) = (s.doc.undo_count(), surface_path(&s).points.clone());
    s.message.clear();
    pathtool::surface::press(&mut s, rect, c, StrokeSource::Mouse);
    assert!(s.path.drag.is_none(), "掴まない");
    assert!(
        s.message.contains("ほかのテクスチャセット"),
        "{}",
        s.message
    );
    assert_eq!(s.doc.undo_count(), undo);
    assert_eq!(surface_path(&s).points, points);
    // 同じ場所でも、手前の板が見せる形に無ければ（遮らなければ）掴む
    s.view3d.set_hidden(vec![1]);
    pathtool::surface::press(&mut s, rect, c, StrokeSource::Mouse);
    assert_eq!(s.path.drag.map(|d| d.index), Some(1), "遮りが無ければ掴む");
    pathtool::surface::release(&mut s, rect, c, StrokeSource::Mouse);
    assert_eq!(s.doc.undo_count(), undo, "動かさなければ文書は変わらない");
    s.view3d.set_hidden(Vec::new());

    // 判定は 256 点まで: ちょうど 256 点なら調べる。257 点になると調べず、全部見えるものとして出して掴める
    for i in 0..253 {
        add_surface(&mut s, 0, 0.5, 0.25 + 0.0002 * (i + 1) as f64);
    }
    assert_eq!(surface_path(&s).points.len(), 256, "{}", s.message);
    assert_eq!(fill_at(&marker_fills(&s, rect), c), dim, "256 点は調べる");
    pathtool::surface::press(&mut s, rect, c, StrokeSource::Mouse);
    assert!(s.path.drag.is_none(), "256 点では遮られる点を掴まない");
    add_surface(&mut s, 0, 0.5, 0.25 + 0.0002 * 254.0);
    assert_eq!(surface_path(&s).points.len(), 257, "{}", s.message);
    assert_eq!(
        fill_at(&marker_fills(&s, rect), c),
        pathtool::PATH_COLOR,
        "257 点は調べず、全部見えるものとして出す"
    );
    pathtool::surface::press(&mut s, rect, c, StrokeSource::Mouse);
    assert_eq!(s.path.drag.map(|d| d.index), Some(1), "257 点では掴める");
    pathtool::surface::release(&mut s, rect, c, StrokeSource::Mouse);
}

#[test]
fn headless_replacing_the_model_while_a_3d_point_is_held_drops_the_drag() {
    let (mut s, rect) = state3d(two_material_plate());
    click3d(&mut s, rect, Vec3::new(-0.7, -0.3, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.3, 0.4, 0.0));
    let held = at3d(&s, rect, Vec3::new(-0.3, 0.4, 0.0));
    let to = at3d(&s, rect, Vec3::new(-0.6, 0.6, 0.0));
    pathtool::surface::press(&mut s, rect, held, StrokeSource::Mouse);
    pathtool::surface::moved(&mut s, rect, to, StrokeSource::Mouse);
    assert!(s.path.drag.is_some_and(|d| d.surface && d.target.is_some()));
    assert!(s.is_stroking());
    // 掴んだまま、UV の違うモデルに入れ替わる: パスは新しいモデルへ付け直し、掴んでいた点の動きは捨てる
    s.view3d.set_model(flipped_plate());
    s.view3d.material = 0;
    s.sync_view3d();
    assert!(
        s.path.drag.is_none() && !s.is_stroking(),
        "ドラッグを取り残さない"
    );
    let rebound = surface_path(&s);
    assert_eq!(
        rebound.model_fingerprint,
        core_paths::fingerprint(&s.view3d.full_model().unwrap().geometry)
    );
    let undo = s.doc.undo_count();
    pathtool::surface::release(&mut s, rect, to, StrokeSource::Mouse);
    assert_eq!(
        s.doc.undo_count(),
        undo,
        "前のモデルの三角形の番号を、付け直したパスへ当てない"
    );
    assert_eq!(surface_path(&s), rebound);
    // 2D の点を掴んでいるあいだの入れ替えは、2D のドラッグに触れない
    let mut t = state(256);
    t.view3d.set_model(two_material_plate());
    t.view3d.material = 0;
    for (x, y) in [(40.0, 40.0), (120.0, 120.0), (200.0, 40.0)] {
        click2d(&mut t, x, y);
    }
    press2d(&mut t, 120.0, 120.0);
    move2d(&mut t, 150.0, 200.0);
    t.view3d.set_model(flipped_plate());
    t.view3d.material = 0;
    t.sync_view3d();
    assert!(
        t.path.drag.is_some_and(|d| !d.surface),
        "2D のドラッグは続く"
    );
    release2d(&mut t, 150.0, 200.0);
    assert!((canvas_points(&t)[1].1 - 200.0).abs() < 1.0);
}

#[test]
fn headless_the_model_change_message_counts_the_outcomes_and_names_the_first_reason() {
    for (lang, redrawn, rasterized, because) in [
        (
            Lang::Ja,
            "描き直し",
            "画素にし",
            "レイヤー「パス 1」のパスは描き直せません（点 1",
        ),
        (
            Lang::En,
            "redrawn",
            "rasterized",
            "Cannot redraw the path of layer \"Path 1\" (Point 1",
        ),
    ] {
        // 付け直せないモデル（板から遠く離れたメッシュ）に入れ替わる: 画素にし、レイヤーの名前と最初の理由を知らせる
        let (mut s, rect) = state3d(two_material_plate());
        s.lang = lang;
        click3d(&mut s, rect, Vec3::new(-0.7, -0.3, 0.0));
        click3d(&mut s, rect, Vec3::new(-0.3, 0.4, 0.0));
        s.view3d.set_model(rebind_far_model());
        s.view3d.material = 0;
        s.sync_view3d();
        assert!(s.path_layer().is_none(), "画素にした");
        assert!(s.message.contains(rasterized), "{lang:?}: {}", s.message);
        assert!(
            !s.message.contains(redrawn),
            "0 件は出さない: {}",
            s.message
        );
        assert!(s.message.contains(because), "{lang:?}: {}", s.message);
        // 付け直せるときは、描き直した件数だけ（理由は無い）
        let (mut t, rect) = state3d(two_material_plate());
        t.lang = lang;
        click3d(&mut t, rect, Vec3::new(-0.7, -0.3, 0.0));
        click3d(&mut t, rect, Vec3::new(-0.3, 0.4, 0.0));
        t.view3d.set_model(flipped_plate());
        t.view3d.material = 0;
        t.sync_view3d();
        assert!(t.message.contains(redrawn), "{lang:?}: {}", t.message);
        assert!(!t.message.contains(rasterized), "{lang:?}: {}", t.message);
        assert!(
            !t.message.contains(lang.pick("（", " (")),
            "{lang:?}: {}",
            t.message
        );
    }
}

// ───────── パスの一覧（1 つのレイヤーに何本ものパス） ─────────

fn list_op(s: &mut AppState, op: ListOp) {
    s.apply(Action::Path(PathAction::List(op)));
}

fn entries(s: &AppState) -> Vec<core_paths::LayerPathEntry> {
    s.path_entries().to_vec()
}

/// 1 つのレイヤーに 2 本の横の線（y = 40 と y = 200）。(レイヤー, 下のパス, 上のパス)。
fn two_lines(s: &mut AppState) -> (LayerId, u128, u128) {
    click2d(s, 40.0, 40.0);
    click2d(s, 120.0, 40.0);
    let a = path(s).unwrap().id();
    s.apply(Action::Path(PathAction::SelectPath(None)));
    click2d(s, 40.0, 200.0);
    click2d(s, 120.0, 200.0);
    let b = path(s).unwrap().id();
    (s.selected_layer.unwrap(), a, b)
}

#[test]
fn headless_enter_finishes_a_path_and_the_next_clicks_start_another_in_the_same_layer() {
    let mut s = state(256);
    click2d(&mut s, 40.0, 40.0);
    click2d(&mut s, 120.0, 40.0);
    let layer = s.selected_layer.unwrap();
    let first = path(&s).unwrap().id();
    let undo = s.doc.undo_count();
    // Enter（パスの編集を抜ける）は文書を変えない
    s.apply(Action::Path(PathAction::SelectPath(None)));
    assert!(s.path_layer().is_none());
    assert_eq!(s.doc.undo_count(), undo);
    click2d(&mut s, 40.0, 200.0);
    click2d(&mut s, 120.0, 200.0);
    assert_eq!(
        s.doc.layers().len(),
        2,
        "同じレイヤーに加える: {}",
        s.message
    );
    assert_eq!(s.selected_layer, Some(layer));
    let list = entries(&s);
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id(), first, "前のパスは下");
    assert_eq!(path(&s).unwrap().id(), list[1].id(), "新しいパスを編集する");
    assert_eq!(s.doc.undo_count(), undo + 2, "点ごとに 1 回の Undo");
    assert!(pixel(&s, 80, 40)[3] > 0 && pixel(&s, 80, 200)[3] > 0);
    // 編集していないパスの曲線を押すと、そのパスを選ぶ（点は置かない）
    click2d(&mut s, 80.0, 40.0);
    assert_eq!(path(&s).unwrap().id(), first);
    assert_eq!(s.doc.undo_count(), undo + 2);
    assert_eq!(canvas_points(&s).len(), 2);
    // Undo: 2 本目の 2 点目、2 本目（一覧から外れる）
    s.apply(Action::Undo);
    s.apply(Action::Undo);
    assert_eq!(entries(&s).len(), 1);
    assert_eq!(pixel(&s, 80, 200)[3], 0);
    assert!(pixel(&s, 80, 40)[3] > 0);
}

#[test]
fn headless_list_operations_are_one_undo_each() {
    let mut s = state(256);
    let (_, a, b) = two_lines(&mut s);
    let undo = s.doc.undo_count();
    // 名前（空の名前は番号の名前）
    assert_eq!(
        pathtool::list::display_name(Lang::Ja, &entries(&s), 0),
        "パス 1"
    );
    list_op(&mut s, ListOp::Rename(a, "  線\n".into()));
    assert_eq!(entries(&s)[0].name, "線", "前後の空白と制御文字を除く");
    assert_eq!(s.doc.undo_count(), undo + 1);
    // 隠すと画素から消える（点と設定は残る）
    list_op(&mut s, ListOp::Visible(a, false));
    assert_eq!(pixel(&s, 80, 40)[3], 0);
    assert!(pixel(&s, 80, 200)[3] > 0);
    assert_eq!(entries(&s)[0].path.point_count(), 2);
    s.apply(Action::Undo);
    assert!(pixel(&s, 80, 40)[3] > 0);
    // 複製: すぐ上に、新しい ID と「〜のコピー」の名前、写しを選ぶ
    list_op(&mut s, ListOp::Duplicate(a));
    let list = entries(&s);
    assert_eq!(list.len(), 3);
    assert_eq!(list[1].name, "線 のコピー");
    assert_ne!(list[1].id(), a);
    assert_eq!(list[1].path.point_count(), 2);
    assert_eq!(path(&s).unwrap().id(), list[1].id());
    let copy = list[1].id();
    // 並べ替え（上へ = 後に描く）
    list_op(&mut s, ListOp::Move(a, pathtool::list::Toward::Up));
    let order: Vec<u128> = entries(&s).iter().map(|e| e.id()).collect();
    assert_eq!(order, vec![copy, a, b]);
    // 削除: 画素からも消える
    list_op(&mut s, ListOp::Delete(b));
    assert_eq!(entries(&s).len(), 2);
    assert_eq!(pixel(&s, 80, 200)[3], 0);
    // コピーは文書を変えない。貼り付けは一覧の上に新しい ID で加える
    let n = s.doc.undo_count();
    list_op(&mut s, ListOp::Copy(a));
    assert_eq!(s.doc.undo_count(), n);
    list_op(&mut s, ListOp::Paste);
    let list = entries(&s);
    assert_eq!(list.len(), 3);
    assert!(list
        .iter()
        .all(|e| e.id() != list[2].id() || e.name == "線 のコピー"));
    assert_eq!(s.doc.undo_count(), n + 1);
    // 1 つの操作が 1 回の Undo: 全部戻すと 2 本の名前の無い一覧
    for _ in 0..5 {
        s.apply(Action::Undo);
    }
    let list = entries(&s);
    assert_eq!(list.iter().map(|e| e.id()).collect::<Vec<_>>(), vec![a, b]);
    assert!(list.iter().all(|e| e.name.is_empty() && e.visible));
    assert_eq!(s.doc.undo_count(), undo);
}

#[test]
fn headless_a_3d_path_is_renamed_without_the_model_and_without_redrawing() {
    let (mut s, rect) = state3d(two_material_plate());
    click3d(&mut s, rect, Vec3::new(-0.7, -0.3, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.3, 0.4, 0.0));
    let (layer, id) = (s.selected_layer.unwrap(), path(&s).unwrap().id());
    let pixels = |s: &AppState| {
        s.doc
            .layer(layer)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .to_canvas_bytes()
    };
    let drawn = pixels(&s);
    assert!(drawn.iter().any(|v| *v != 0));
    let undo = s.doc.undo_count();
    // モデルを読んでいなくても、別のモデルを読んでいても、名前は替えられる（画素は変えない）
    s.view3d.model = None;
    list_op(&mut s, ListOp::Rename(id, "腕".into()));
    assert_eq!(entries(&s)[0].name, "腕", "{}", s.message);
    assert_eq!(s.doc.undo_count(), undo + 1);
    assert_eq!(pixels(&s), drawn);
    s.view3d.set_model(flipped_plate());
    s.view3d.material = 0;
    list_op(&mut s, ListOp::Rename(id, "脚".into()));
    assert_eq!(entries(&s)[0].name, "脚", "{}", s.message);
    assert_eq!(s.doc.undo_count(), undo + 2, "1 回ずつの Undo");
    assert_eq!(pixels(&s), drawn, "描き直さない");
    s.apply(Action::Undo);
    assert_eq!(entries(&s)[0].name, "腕");
    s.apply(Action::Undo);
    assert_eq!(entries(&s)[0].name, "");
    assert_eq!(pixels(&s), drawn);
}

#[test]
fn headless_settings_and_positions_paste_onto_another_path() {
    let mut s = state(256);
    let (_, a, b) = two_lines(&mut s);
    // b を太くする
    s.apply(Action::Path(PathAction::SelectPath(Some(b))));
    s.apply(Action::Path(PathAction::Brush(BrushEdit::Diameter(60.0))));
    let find = |s: &AppState, id: u128| entries(s).into_iter().find(|e| e.id() == id).unwrap().path;
    let (pa, pb) = (find(&s, a), find(&s, b));
    assert_ne!(pathtool::path_brush(&pa), pathtool::path_brush(&pb));
    // b の設定を a へ: 点は a のまま
    list_op(&mut s, ListOp::Copy(b));
    let n = s.doc.undo_count();
    list_op(&mut s, ListOp::PasteSettings(a));
    let next = find(&s, a);
    assert_eq!(pathtool::path_brush(&next), pathtool::path_brush(&pb));
    assert_eq!(next.point_count(), pa.point_count());
    assert!(pixel(&s, 80, 40 + 25)[3] > 0, "太くなった");
    // b の位置を a へ: ブラシは今の a のまま
    list_op(&mut s, ListOp::PastePositions(a));
    match (find(&s, a), &pb) {
        (LayerPath::Canvas(x), LayerPath::Canvas(y)) => assert_eq!(x.points, y.points),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.doc.undo_count(), n + 2);
    s.apply(Action::Undo);
    s.apply(Action::Undo);
    assert_eq!(find(&s, a), pa);
    // 写していなければ断る
    let mut t = state(256);
    let (_, a, _) = two_lines(&mut t);
    let n = t.doc.undo_count();
    list_op(&mut t, ListOp::PasteSettings(a));
    assert_eq!(t.doc.undo_count(), n);
    assert!(
        t.message.contains("コピーしたパスがありません"),
        "{}",
        t.message
    );
}

#[test]
fn headless_a_list_survives_saving_and_opening() {
    let dir = std::env::temp_dir().join(format!("yolu-pathlist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("list.ylp");
    let mut s = state(128);
    click2d(&mut s, 20.0, 20.0);
    click2d(&mut s, 100.0, 20.0);
    let a = path(&s).unwrap().id();
    s.apply(Action::Path(PathAction::SelectPath(None)));
    click2d(&mut s, 20.0, 100.0);
    click2d(&mut s, 100.0, 100.0);
    list_op(&mut s, ListOp::Rename(a, "上の線".into()));
    list_op(&mut s, ListOp::Visible(a, false));
    let before = entries(&s);
    s.apply(Action::SaveProjectAs(file.clone()));
    assert!(file.exists(), "{}", s.message);
    let mut t = AppState::new(64, 64);
    t.apply(Action::OpenProject(file));
    let layer = t
        .doc
        .layers()
        .iter()
        .find(|l| l.has_paths())
        .expect("パスレイヤー")
        .id();
    t.selected_layer = Some(layer);
    assert_eq!(entries(&t), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_a_replaced_model_moves_every_path_of_a_list() {
    let (mut s, rect) = state3d(two_material_plate());
    click3d(&mut s, rect, Vec3::new(-0.7, -0.3, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.3, 0.4, 0.0));
    s.apply(Action::Path(PathAction::SelectPath(None)));
    click3d(&mut s, rect, Vec3::new(-0.8, 0.6, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.2, 0.7, 0.0));
    assert_eq!(entries(&s).len(), 2, "{}", s.message);
    s.view3d.set_model(flipped_plate());
    s.view3d.material = 0;
    s.sync_view3d();
    let print = core_paths::fingerprint(&s.view3d.full_model().unwrap().geometry);
    let list = entries(&s);
    assert_eq!(list.len(), 2, "一覧の全部を付け直す: {}", s.message);
    for e in &list {
        match &e.path {
            LayerPath::Surface(p) => assert_eq!(p.model_fingerprint, print),
            other => panic!("{other:?}"),
        }
    }
}

// ───────── 点の接線（角・取っ手） ─────────

fn tangent_of(s: &AppState, i: usize) -> TangentValue {
    pathtool::edit::point_tangent(&path(s).unwrap(), i).unwrap()
}

fn three_points(s: &mut AppState) {
    for (x, y) in [(40.0, 60.0), (128.0, 200.0), (216.0, 60.0)] {
        click2d(s, x, y);
    }
}

#[test]
fn headless_corner_and_handles_are_one_undo_each_and_moving_keeps_them() {
    let mut s = state(256);
    three_points(&mut s);
    let smooth = layer_color_bytes(&s);
    s.apply(Action::Path(PathAction::Select(Some(1))));
    let undo = s.doc.undo_count();
    s.apply(Action::Path(PathAction::SetCorner(true)));
    assert_eq!(tangent_of(&s, 1), TangentValue::Corner);
    assert_eq!(s.doc.undo_count(), undo + 1);
    assert_ne!(layer_color_bytes(&s), smooth, "角で折れる");
    // 動かしても角のまま
    drag2d(&mut s, (128.0, 200.0), (128.0, 180.0));
    assert_eq!(tangent_of(&s, 1), TangentValue::Corner);
    s.apply(Action::Undo);
    s.apply(Action::Path(PathAction::SetCorner(false)));
    assert_eq!(tangent_of(&s, 1), TangentValue::Smooth);
    assert_eq!(layer_color_bytes(&s), smooth, "滑らかに戻すと同じ画素");
    // 取っ手: 今の曲がりのままの向き（入る側と出る側が反対を向く）
    s.apply(Action::Path(PathAction::SetHandles(true)));
    let TangentValue::Handles { incoming, outgoing } = tangent_of(&s, 1) else {
        panic!("{:?}", tangent_of(&s, 1))
    };
    let (a, b) = (
        DVec2::new(incoming[0], incoming[1]),
        DVec2::new(outgoing[0], outgoing[1]),
    );
    assert!(a.length() > 1.0 && b.length() > 1.0, "{a} {b}");
    assert!(
        a.normalize().dot(b.normalize()) < -0.999,
        "反対向き: {a} {b}"
    );
    // 頂点の点は左右対称なので、取っ手は横向き
    assert!(b.y.abs() < 1e-6 && b.x > 0.0, "{b}");
    s.apply(Action::Undo);
    assert_eq!(tangent_of(&s, 1), TangentValue::Smooth);
}

#[test]
fn headless_dragging_a_handle_aligns_breaks_with_alt_and_scales_with_ctrl() {
    let mut s = state(256);
    three_points(&mut s);
    s.apply(Action::Path(PathAction::Select(Some(1))));
    s.apply(Action::Path(PathAction::SetHandles(true)));
    let TangentValue::Handles { incoming, outgoing } = tangent_of(&s, 1) else {
        panic!()
    };
    let len_in = DVec2::new(incoming[0], incoming[1]).length();
    let end = (128.0 + outgoing[0], 200.0 + outgoing[1]);
    // 出る側の先を上へ: 入る側は向きだけ合わせる（長さは今のまま）
    let undo = s.doc.undo_count();
    drag2d(&mut s, end, (128.0 + 30.0, 200.0 + 30.0));
    assert_eq!(s.doc.undo_count(), undo + 1, "{}", s.message);
    let TangentValue::Handles { incoming, outgoing } = tangent_of(&s, 1) else {
        panic!()
    };
    let (a, b) = (
        DVec2::new(incoming[0], incoming[1]),
        DVec2::new(outgoing[0], outgoing[1]),
    );
    assert!((b - DVec2::new(30.0, 30.0)).length() < 1.0, "{b}");
    assert!((a.length() - len_in).abs() < 1e-6, "{a}");
    assert!(a.normalize().dot(b.normalize()) < -0.9999);
    // Alt: 動かした側だけ（折れる）
    s.path.input.alt = true;
    drag2d(&mut s, (158.0, 230.0), (158.0, 200.0));
    s.path.input.alt = false;
    let TangentValue::Handles { incoming, outgoing } = tangent_of(&s, 1) else {
        panic!()
    };
    assert_eq!(DVec2::new(incoming[0], incoming[1]), a, "入る側はそのまま");
    assert!((DVec2::new(outgoing[0], outgoing[1]) - DVec2::new(30.0, 0.0)).length() < 1.0);
    // 折れた取っ手は、Alt を離しても片側だけ動く
    drag2d(&mut s, (158.0, 200.0), (168.0, 200.0));
    let TangentValue::Handles { incoming, .. } = tangent_of(&s, 1) else {
        panic!()
    };
    assert_eq!(DVec2::new(incoming[0], incoming[1]), a);
    // Ctrl: 揃った取っ手の両側を同じ割合で伸ばす
    let mut t = state(256);
    three_points(&mut t);
    t.apply(Action::Path(PathAction::Select(Some(1))));
    t.apply(Action::Path(PathAction::SetHandles(true)));
    let TangentValue::Handles { incoming, outgoing } = tangent_of(&t, 1) else {
        panic!()
    };
    let (a, b) = (
        DVec2::new(incoming[0], incoming[1]),
        DVec2::new(outgoing[0], outgoing[1]),
    );
    let end = (128.0 + b.x, 200.0 + b.y);
    t.path.input.ctrl = true;
    drag2d(&mut t, end, (128.0 + b.x * 2.0, 200.0 + b.y * 2.0));
    let TangentValue::Handles { incoming, .. } = tangent_of(&t, 1) else {
        panic!()
    };
    assert!((DVec2::new(incoming[0], incoming[1]) - a * 2.0).length() < 1.0);
}

#[test]
fn headless_double_clicking_a_point_toggles_the_corner() {
    let mut s = state(256);
    three_points(&mut s);
    s.path.input.now = Some(10.0);
    click2d(&mut s, 128.0, 200.0);
    s.path.input.now = Some(10.2);
    click2d(&mut s, 128.0, 200.0);
    assert_eq!(tangent_of(&s, 1), TangentValue::Corner);
    // 間が空けば、ダブルクリックではない
    s.path.input.now = Some(11.0);
    click2d(&mut s, 128.0, 200.0);
    s.path.input.now = Some(12.0);
    click2d(&mut s, 128.0, 200.0);
    assert_eq!(tangent_of(&s, 1), TangentValue::Corner);
    s.path.input.now = Some(12.1);
    click2d(&mut s, 128.0, 200.0);
    assert_eq!(tangent_of(&s, 1), TangentValue::Smooth);
}

#[test]
fn headless_3d_handles_are_kept_in_the_rest_shape_space() {
    let (mut s, rect) = state3d(two_material_plate());
    click3d(&mut s, rect, Vec3::new(-0.8, -0.5, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.5, 0.3, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.2, -0.5, 0.0));
    s.apply(Action::Path(PathAction::Select(Some(1))));
    s.apply(Action::Path(PathAction::SetHandles(true)));
    let TangentValue::Handles { outgoing, .. } = tangent_of(&s, 1) else {
        panic!("{}", s.message)
    };
    assert!(
        outgoing[0] > 0.0 && outgoing[2].abs() < 1e-6,
        "{outgoing:?}"
    );
    // 取っ手の先を、面（z = 0）の上でドラッグする
    let from = Vec3::new(-0.5 + outgoing[0] as f32, 0.3 + outgoing[1] as f32, 0.0);
    let to = Vec3::new(-0.3, 0.6, 0.0);
    let (a, b) = (at3d(&s, rect, from), at3d(&s, rect, to));
    pathtool::surface::press(&mut s, rect, a, StrokeSource::Mouse);
    pathtool::surface::moved(&mut s, rect, a + (b - a) * 0.5, StrokeSource::Mouse);
    pathtool::surface::release(&mut s, rect, b, StrokeSource::Mouse);
    let TangentValue::Handles { outgoing, .. } = tangent_of(&s, 1) else {
        panic!()
    };
    assert!(
        (outgoing[0] - 0.2).abs() < 0.01 && (outgoing[1] - 0.3).abs() < 0.01,
        "{outgoing:?}"
    );
    let before = layer_color_bytes(&s);
    // 左の面を横に 2 倍に伸ばしたポーズ: 見える取っ手は 2 倍だが、持つ値（休みの形）は同じで、描いた画素も同じ
    let rest = s.view3d.full_model().unwrap().geometry.clone();
    let mut meshes = two_material_plate().meshes;
    for p in &mut meshes[0].positions[..4] {
        p.x = -1.0 + (p.x + 1.0) * 2.0;
    }
    let names = vec![Some("左".into()), Some("右".into())];
    let posed = ViewModel::new("板", meshes, names, 5)
        .unwrap()
        .with_rest(rest);
    s.view3d.set_model(posed);
    s.sync_view3d();
    s.apply(Action::Path(PathAction::Redraw));
    assert_eq!(layer_color_bytes(&s), before);
    // 見えている先（伸ばした形）をつかんで、同じ所へ戻すと値は変わらない
    let shown_end = Vec3::new(-1.0 + (-0.5 + 1.0 + 0.2) * 2.0, 0.6, 0.0);
    let a = at3d(&s, rect, shown_end);
    let n = s.doc.undo_count();
    pathtool::surface::press(&mut s, rect, a, StrokeSource::Mouse);
    assert!(
        s.path.drag.is_some_and(|d| d.handle.is_some()),
        "見えている先をつかむ"
    );
    let b = at3d(
        &s,
        rect,
        Vec3::new(-1.0 + (-0.5 + 1.0 + 0.1) * 2.0, 0.6, 0.0),
    );
    pathtool::surface::moved(&mut s, rect, b, StrokeSource::Mouse);
    pathtool::surface::release(&mut s, rect, b, StrokeSource::Mouse);
    assert_eq!(s.doc.undo_count(), n + 1);
    let TangentValue::Handles { outgoing, .. } = tangent_of(&s, 1) else {
        panic!()
    };
    assert!(
        (outgoing[0] - 0.1).abs() < 0.01,
        "見えている長さの半分（休みの形）: {outgoing:?}"
    );
}

// ───────── パスの種類 ─────────

fn kind_of(s: &AppState) -> PathKind {
    path(s).unwrap().style().kind
}

#[test]
fn headless_the_kind_redraws_the_path_in_one_undo_and_sets_the_next_path() {
    let mut s = state(128);
    for (x, y) in [(20.0, 20.0), (100.0, 20.0), (60.0, 100.0)] {
        click2d(&mut s, x, y);
    }
    let stroke = layer_color_bytes(&s);
    assert_eq!(pixel(&s, 60, 50)[3], 0, "ストロークは内側を塗らない");
    let undo = s.doc.undo_count();
    s.apply(Action::Path(PathAction::Kind(PathKind::Fill)));
    assert_eq!(kind_of(&s), PathKind::Fill);
    assert_eq!(s.doc.undo_count(), undo + 1);
    assert!(pixel(&s, 60, 50)[3] > 0, "塗りは内側を塗る");
    s.apply(Action::Undo);
    assert_eq!(kind_of(&s), PathKind::Stroke);
    assert_eq!(layer_color_bytes(&s), stroke);
    // パスの編集を抜けているあいだは、次に作るパスの種類
    s.apply(Action::Path(PathAction::SelectPath(None)));
    s.apply(Action::Path(PathAction::Kind(PathKind::Erase)));
    assert_eq!(s.doc.undo_count(), undo, "文書は変えない");
    click2d(&mut s, 10.0, 60.0);
    click2d(&mut s, 120.0, 60.0);
    assert_eq!(kind_of(&s), PathKind::Erase);
    assert_eq!(entries(&s).len(), 2);
    assert_eq!(pixel(&s, 40, 60)[3], 0, "消しゴムのパスが前のパスを消す");
}

#[test]
fn headless_a_ribbon_reads_an_asset_image_and_refuses_without_one() {
    let mut s = state(128);
    click2d(&mut s, 10.0, 64.0);
    click2d(&mut s, 118.0, 64.0);
    // アセットに画像が無ければ、リボンは選べない
    let entries_now = yolu_app::panels::path_props::kind_entries(&s);
    let ribbon = entries_now
        .iter()
        .find(|e| e.label() == Some("リボン"))
        .unwrap();
    assert!(matches!(
        ribbon,
        yolu_app::ui::menu::Entry::Item { enabled: false, .. }
    ));
    // 2 × 1 の画像（左が赤・右が青）を伸ばして並べる
    let rid = s
        .shelf
        .add_image(Lang::Ja, "帯", &[255, 0, 0, 255, 0, 0, 255, 255], 2, 1)
        .unwrap();
    let image = yolu_app::fx::inputs::image_id(&rid).unwrap();
    let kind = PathKind::Ribbon(core_paths::Ribbon {
        image,
        mode: core_paths::RibbonMode::Stretch,
        spacing: 1.0,
    });
    s.apply(Action::Path(PathAction::Kind(kind)));
    assert_eq!(kind_of(&s), kind, "{}", s.message);
    let (left, right) = (pixel(&s, 20, 64), pixel(&s, 108, 64));
    assert!(left[0] > 200 && left[2] < 50, "{left:?}");
    assert!(right[2] > 200 && right[0] < 50, "{right:?}");
    // 画像を使っているので、アセットから消す前の確かめに名前が出る
    assert!(!yolu_app::fillfx::image_users(&s, &rid).is_empty());
}

// ───────── 筆先・投影の深さ ─────────

fn style_of(s: &AppState) -> core_paths::PathStyle {
    path(s).unwrap().style().clone()
}

#[test]
fn headless_the_tip_angle_and_depth_are_one_undo_each_and_use_brush_takes_the_brush_tip() {
    let mut s = state(128);
    click2d(&mut s, 20.0, 64.0);
    click2d(&mut s, 108.0, 64.0);
    let round = layer_color_bytes(&s);
    let square = yolu_core::builtin_tip("rounded-square").unwrap();
    let undo = s.doc.undo_count();
    s.apply(Action::Path(PathAction::Style(StyleEdit::Tip(Some(
        square.clone(),
    )))));
    assert_eq!(style_of(&s).tip.as_deref(), Some(&*square));
    assert_eq!(s.doc.undo_count(), undo + 1);
    assert_ne!(layer_color_bytes(&s), round);
    s.apply(Action::Path(PathAction::Style(StyleEdit::Angle(30.0))));
    s.apply(Action::Path(PathAction::Style(StyleEdit::Follow(true))));
    let style = style_of(&s);
    assert_eq!((style.angle, style.follow), (30.0, true));
    assert_eq!(s.doc.undo_count(), undo + 3);
    s.apply(Action::Undo);
    s.apply(Action::Undo);
    s.apply(Action::Undo);
    assert_eq!(style_of(&s), core_paths::PathStyle::default());
    assert_eq!(layer_color_bytes(&s), round);
    // ブラシを使う: 今のブラシの筆先・角度・向きも写す
    s.m2.brush.tip.image = Some(square.clone());
    s.m2.brush.tip.angle = 45.0;
    s.apply(Action::Path(PathAction::UseBrush));
    let style = style_of(&s);
    assert_eq!(style.tip.as_deref(), Some(&*square));
    assert_eq!(style.angle, 45.0);
    // 筆先の選択肢: 丸・今のブラシの筆先・組み込み
    let entries = yolu_app::panels::path_props::tip_entries(&s);
    assert!(entries.iter().any(|e| e.label() == Some("丸")));
    assert!(entries
        .iter()
        .any(|e| e.label().is_some_and(|l| l.starts_with("今のブラシの筆先"))));
}

#[test]
fn headless_a_3d_path_keeps_its_projection_depth() {
    let (mut s, rect) = state3d(two_material_plate());
    click3d(&mut s, rect, Vec3::new(-0.8, -0.5, 0.0));
    click3d(&mut s, rect, Vec3::new(-0.2, 0.5, 0.0));
    let auto = layer_color_bytes(&s);
    s.apply(Action::Path(PathAction::Style(StyleEdit::Depth(Some(2.0)))));
    assert_eq!(surface_path(&s).style.depth, Some(2.0), "{}", s.message);
    // 平らな板なので、深さを決めても同じ画素
    assert_eq!(layer_color_bytes(&s), auto);
    s.apply(Action::Path(PathAction::Style(StyleEdit::Depth(None))));
    assert_eq!(surface_path(&s).style.depth, None);
}

// ───────── 対称・反転・点の範囲選択・プリセット ─────────

#[test]
fn headless_symmetry_and_reverse_are_one_undo_each() {
    let mut s = state(128);
    click2d(&mut s, 10.0, 30.0);
    click2d(&mut s, 40.0, 90.0);
    let before = canvas_points(&s);
    assert_eq!(pixel(&s, 118, 30)[3], 0);
    // 対称定規が無ければ入れられない（理由が出る。取り消しの段も増えない）
    let none = s.doc.undo_count();
    s.apply(Action::Path(PathAction::Symmetry(true)));
    assert_eq!(style_of(&s).symmetry, core_paths::PathSymmetry::None);
    assert!(s.message.contains("対称定規"), "{}", s.message);
    assert_eq!(s.doc.undo_count(), none);
    // 効いている対称定規（縦、中心 x = 64）の値を写して映す
    common::rulers::vertical(&mut s, 64.0);
    // 対称定規はあるが「特殊定規にスナップ」が切れているときは、切れていることを理由にする（対称定規が無いとは言わない）
    s.rulers.snap_special = false;
    s.apply(Action::Path(PathAction::Symmetry(true)));
    assert_eq!(style_of(&s).symmetry, core_paths::PathSymmetry::None);
    assert!(
        s.message.contains("特殊定規にスナップが切れています"),
        "{}",
        s.message
    );
    assert!(!s.message.contains("対称定規がありません"), "{}", s.message);
    assert_eq!(s.doc.undo_count(), none + 1, "定規を置いた 1 回のまま");
    s.rulers.snap_special = true;
    let undo = s.doc.undo_count();
    s.apply(Action::Path(PathAction::Symmetry(true)));
    assert!(matches!(
        style_of(&s).symmetry,
        core_paths::PathSymmetry::Canvas(_)
    ));
    assert!(pixel(&s, 117, 30)[3] > 0, "映した側");
    assert_eq!(s.doc.undo_count(), undo + 1);
    // 向きの反転: 点の並びが逆、1 回の Undo
    s.apply(Action::Path(PathAction::Reverse));
    let reversed = canvas_points(&s);
    assert_eq!(reversed[0], before[1]);
    assert_eq!(reversed[1], before[0]);
    assert_eq!(s.doc.undo_count(), undo + 2);
    s.apply(Action::Undo);
    s.apply(Action::Undo);
    assert_eq!(canvas_points(&s), before);
    assert_eq!(pixel(&s, 117, 30)[3], 0);
}

#[test]
fn headless_shift_drag_selects_points_and_width_and_delete_apply_to_all_of_them() {
    let mut s = state(256);
    for (x, y) in [
        (30.0, 30.0),
        (80.0, 40.0),
        (130.0, 30.0),
        (180.0, 40.0),
        (230.0, 30.0),
    ] {
        click2d(&mut s, x, y);
    }
    // Shift を押して、真ん中の 3 点を囲む
    s.path.input.shift = true;
    drag2d(&mut s, (60.0, 10.0), (200.0, 60.0));
    s.path.input.shift = false;
    assert_eq!(s.path_selected_indices(), vec![1, 2, 3]);
    assert_eq!(canvas_points(&s).len(), 5, "囲むだけで点は足さない");
    let undo = s.doc.undo_count();
    s.apply(Action::Path(PathAction::Widths(0.25)));
    let widths: Vec<f64> = canvas_points(&s).iter().map(|p| p.2).collect();
    assert_eq!(widths, vec![1.0, 0.25, 0.25, 0.25, 1.0]);
    assert_eq!(s.doc.undo_count(), undo + 1, "全部で 1 回の Undo");
    s.apply(Action::Path(PathAction::SetCorner(true)));
    for i in 1..=3 {
        assert_eq!(tangent_of(&s, i), TangentValue::Corner);
    }
    assert_eq!(tangent_of(&s, 0), TangentValue::Smooth);
    s.apply(Action::Path(PathAction::DeleteSelected));
    let xs: Vec<f64> = canvas_points(&s).iter().map(|p| p.0).collect();
    assert_eq!(xs, vec![30.0, 230.0]);
    assert_eq!(s.doc.undo_count(), undo + 3);
    // 全部の点を選ぶ・Esc で外す
    s.apply(Action::Path(PathAction::SelectAllPoints));
    assert_eq!(s.path_selected_indices(), vec![0, 1]);
    assert!(s.path_cancel(50));
    assert!(s.path_selected_indices().is_empty());
}

#[test]
fn headless_presets_save_apply_and_delete_through_the_settings_folder() {
    use yolu_app::pathtool::presets::PresetOp;
    let dir = std::env::temp_dir().join(format!("yolu-pathpresets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    let mut s = state(128);
    s.path.presets.attach(dir.clone());
    click2d(&mut s, 10.0, 60.0);
    click2d(&mut s, 118.0, 60.0);
    s.apply(Action::Path(PathAction::Brush(BrushEdit::Diameter(40.0))));
    s.apply(Action::Path(PathAction::Kind(PathKind::Erase)));
    s.apply(Action::Path(PathAction::Preset(PresetOp::Save)));
    assert_eq!(s.path.presets.list().len(), 1, "{}", s.message);
    let id = s.path.presets.list()[0].id;
    assert_eq!(s.path.presets.list()[0].name, "消しゴム");
    // 読み直しても同じ
    let mut again = yolu_app::pathtool::presets::PathPresets::default();
    again.attach(dir.clone());
    assert_eq!(again.list(), s.path.presets.list());
    // 別のパスへ当てる: 種類と直径（1 回の Undo）
    s.apply(Action::Path(PathAction::SelectPath(None)));
    s.apply(Action::Path(PathAction::Kind(PathKind::Stroke)));
    click2d(&mut s, 10.0, 100.0);
    click2d(&mut s, 118.0, 100.0);
    let undo = s.doc.undo_count();
    s.apply(Action::Path(PathAction::Preset(PresetOp::Apply(id))));
    assert_eq!(kind_of(&s), PathKind::Erase);
    assert_eq!(pathtool::path_brush(&path(&s).unwrap()).0.radius, 20.0);
    assert_eq!(s.doc.undo_count(), undo + 1);
    s.apply(Action::Path(PathAction::Preset(PresetOp::Delete(id))));
    assert!(s.path.presets.list().is_empty());
    let mut empty = yolu_app::pathtool::presets::PathPresets::default();
    empty.attach(dir.clone());
    assert!(empty.list().is_empty(), "ファイルも消える");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_a_preset_carries_the_anti_alias_with_or_without_a_selected_path() {
    use yolu_app::engine::AntiAlias;
    use yolu_app::pathtool::presets::PresetOp;
    let dir = std::env::temp_dir().join(format!("yolu-pathpresets-aa-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    let mut s = state(128);
    s.path.presets.attach(dir.clone());
    click2d(&mut s, 10.0, 60.0);
    click2d(&mut s, 118.0, 60.0);
    s.apply(Action::Path(PathAction::Brush(BrushEdit::AntiAlias(
        AntiAlias::Strong,
    ))));
    assert_eq!(
        pathtool::path_brush(&path(&s).unwrap()).0.anti_alias,
        AntiAlias::Strong
    );
    s.apply(Action::Path(PathAction::Preset(PresetOp::Save)));
    let id = s.path.presets.list()[0].id;
    // 選んでいるパスへ当てる
    s.apply(Action::Path(PathAction::Brush(BrushEdit::AntiAlias(
        AntiAlias::None,
    ))));
    assert_eq!(
        pathtool::path_brush(&path(&s).unwrap()).0.anti_alias,
        AntiAlias::None
    );
    s.apply(Action::Path(PathAction::Preset(PresetOp::Apply(id))));
    assert_eq!(
        pathtool::path_brush(&path(&s).unwrap()).0.anti_alias,
        AntiAlias::Strong
    );
    // パスを選んでいないときは、次に作るパスが取る今のブラシへ
    let mut t = state(128);
    t.path.presets.attach(dir.clone());
    assert!(path(&t).is_none());
    t.brush.anti_alias = AntiAlias::None;
    t.apply(Action::Path(PathAction::Preset(PresetOp::Apply(id))));
    assert_eq!(t.brush.anti_alias, AntiAlias::Strong);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_a_preset_rename_rewrites_its_file_numbers_a_taken_name_and_keeps_an_empty_one() {
    use yolu_app::pathtool::presets::{PathPresets, PresetOp, RenameError};
    let dir = std::env::temp_dir().join(format!("yolu-pathpresets-rename-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    let mut s = state(128);
    s.path.presets.attach(dir.clone());
    click2d(&mut s, 10.0, 60.0);
    click2d(&mut s, 118.0, 60.0);
    s.apply(Action::Path(PathAction::Preset(PresetOp::Save)));
    s.apply(Action::Path(PathAction::Preset(PresetOp::Save)));
    let ids: Vec<u32> = s.path.presets.list().iter().map(|p| p.id).collect();
    let names = |s: &AppState| -> Vec<String> {
        s.path
            .presets
            .list()
            .iter()
            .map(|p| p.name.clone())
            .collect()
    };
    assert_eq!(names(&s), ["ストローク", "ストローク 2"]);
    // 入力欄を開く（文書は変えない）
    let undo = s.doc.undo_count();
    s.apply(Action::Path(PathAction::Preset(PresetOp::BeginRename(
        ids[0],
    ))));
    assert_eq!(s.path.preset_renaming, Some(ids[0]));
    // 前後の空白と制御文字を除く・ファイルを書き直す
    s.apply(Action::Path(PathAction::Preset(PresetOp::Rename(
        ids[0],
        "  太い線\n".into(),
    ))));
    assert_eq!(names(&s), ["太い線", "ストローク 2"]);
    let mut again = PathPresets::default();
    again.attach(dir.clone());
    assert_eq!(again.list(), s.path.presets.list(), "読み直しても同じ");
    // ほかのプリセットと同じ名前には番号を付ける・空の名前は前のまま
    s.apply(Action::Path(PathAction::Preset(PresetOp::Rename(
        ids[1],
        "太い線".into(),
    ))));
    assert_eq!(names(&s), ["太い線", "太い線 2"]);
    s.apply(Action::Path(PathAction::Preset(PresetOp::Rename(
        ids[1],
        "   ".into(),
    ))));
    assert_eq!(names(&s), ["太い線", "太い線 2"]);
    assert_eq!(s.path.presets.rename(ids[1], " "), Err(RenameError::Empty));
    assert_eq!(s.path.presets.rename(999, "x"), Err(RenameError::Missing));
    // 同じ名前（自分の名前）には番号を付けない
    assert_eq!(s.path.presets.rename(ids[0], "太い線").unwrap(), "太い線");
    assert_eq!(
        s.doc.undo_count(),
        undo,
        "名前の変更は文書の取り消しに積まない"
    );
    // 名前を変えている途中で消すと、入力欄も閉じる
    s.apply(Action::Path(PathAction::Preset(PresetOp::Delete(ids[0]))));
    assert_eq!(s.path.preset_renaming, None);
    let mut after = PathPresets::default();
    after.attach(dir.clone());
    let left: Vec<&str> = after.list().iter().map(|p| p.name.as_str()).collect();
    assert_eq!(left, ["太い線 2"]);
    let _ = std::fs::remove_dir_all(&dir);
}

// ───────── 塗りつぶしレイヤーのパス ─────────

#[test]
fn headless_a_path_on_a_fill_layer_stays_on_that_layer_over_the_fill() {
    let mut s = state(128);
    s.apply(Action::M2(yolu_app::m2::Edit::NewFill));
    let fill = s.selected_layer.unwrap();
    let layers = s.doc.layers().len();
    let fill_px = pixel(&s, 60, 60);
    assert_eq!(fill_px[3], 255, "塗りつぶし");
    s.apply(Action::SelectTool(Tool::Path));
    s.color.set_main([0.0, 0.0, 1.0, 1.0]);
    click2d(&mut s, 10.0, 60.0);
    click2d(&mut s, 118.0, 60.0);
    assert_eq!(
        s.doc.layers().len(),
        layers,
        "新しいレイヤーは作らない: {}",
        s.message
    );
    assert_eq!(s.selected_layer, Some(fill));
    assert!(s.doc.layer(fill).unwrap().has_paths());
    let on = pixel(&s, 60, 60);
    assert!(on[2] > 200 && on[0] < 50, "パスは塗りつぶしの上: {on:?}");
    assert_eq!(pixel(&s, 60, 20), fill_px, "パスの外は塗りつぶしのまま");
    // 塗りつぶしレイヤーのパスは画素にしない
    s.apply(Action::Path(PathAction::Rasterize(fill)));
    assert!(s.doc.layer(fill).unwrap().has_paths());
    s.apply(Action::Undo);
    s.apply(Action::Undo);
    assert!(!s.doc.layer(fill).unwrap().has_paths());
    assert_eq!(pixel(&s, 60, 60), fill_px);
}

// ───────── 画面の操作（egui_kittest） ─────────

fn canvas_pos(h: &Harness<'_, YoluApp>, x: f64, y: f64) -> Pos2 {
    let app = h.state();
    let r = canvas_rect(h);
    app.state
        .view
        .view(r, app.state.doc.width(), app.state.doc.height())
        .to_screen(x, y)
}

fn path_app(size: u32) -> Harness<'static, YoluApp> {
    let mut h = app(1280.0, 800.0, size);
    key(&h, Key::P, Modifiers::NONE);
    h.run();
    h
}

#[test]
fn p_selects_the_path_tool_and_the_strip_and_menu_show_it() {
    let mut h = app(1280.0, 800.0, 256);
    assert_eq!(h.state().state.tool, Tool::Brush);
    key(&h, Key::P, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.tool, Tool::Path);
    // ツールの帯のボタンは、名前とキーのツールチップ
    h.get_by_label("パス（P）");
    // 編集のメニューにも並ぶ
    let at = menu_title(&h, "編集").center();
    click(&mut h, at);
    popup_item(&h, "パス");
}

#[test]
fn clicking_the_canvas_adds_points_and_the_overlay_follows() {
    let mut h = path_app(256);
    for (x, y) in [(50.0, 60.0), (130.0, 190.0), (210.0, 80.0)] {
        let at = canvas_pos(&h, x, y);
        click(&mut h, at);
    }
    let s = &h.state().state;
    assert_eq!(s.doc.layers().len(), 2, "{}", s.message);
    assert_eq!(s.path_layer().unwrap().1.point_count(), 3);
    assert_eq!(s.doc.undo_count(), 3);
    // 点の印（橙）が見える
    let image = h.render().expect("描ける");
    let p = canvas_pos(&h, 130.0, 190.0);
    let orange = |pos: Pos2| {
        let px = image
            .get_pixel(pos.x.round() as u32, pos.y.round() as u32)
            .0;
        px[0] > 200 && px[1] > 150 && px[2] < 120
    };
    // 選んだ点（最後の点）は白、ほかの点は橙
    assert!(orange(p), "点の印");
    {
        let (_, p) = h.state().state.path_layer().unwrap();
        assert!(p.is_canvas());
        assert_eq!(p.point_count(), 3);
    }
    h.snapshot("path_canvas_points");
}

#[test]
fn dragging_a_point_with_the_mouse_moves_it_once() {
    let mut h = path_app(256);
    for (x, y) in [(50.0, 60.0), (130.0, 190.0), (210.0, 80.0)] {
        let at = canvas_pos(&h, x, y);
        click(&mut h, at);
    }
    let n = h.state().state.doc.undo_count();
    let from = canvas_pos(&h, 130.0, 190.0);
    let to = canvas_pos(&h, 130.0, 120.0);
    drag(&mut h, &[from, from + (to - from) * 0.5, to]);
    let s = &h.state().state;
    assert_eq!(s.doc.undo_count(), n + 1);
    let pts = match s.path_layer().unwrap().1 {
        LayerPath::Canvas(c) => c.points.clone(),
        _ => unreachable!(),
    };
    assert!((pts[1].y - 120.0).abs() < 1.5, "{pts:?}");
}

#[test]
fn escape_during_a_drag_leaves_the_point_and_focus_loss_commits() {
    let mut h = path_app(256);
    for (x, y) in [(50.0, 60.0), (130.0, 190.0), (210.0, 80.0)] {
        let at = canvas_pos(&h, x, y);
        click(&mut h, at);
    }
    let n = h.state().state.doc.undo_count();
    let from = canvas_pos(&h, 130.0, 190.0);
    let to = canvas_pos(&h, 60.0, 30.0);
    press(&h, from, egui::PointerButton::Primary);
    h.step();
    move_to(&h, to);
    h.step();
    assert!(h.state().state.path.drag.is_some());
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(h.state().state.path.drag.is_none());
    assert_eq!(
        h.state().state.path_selected_index(),
        Some(1),
        "ドラッグを捨てた Esc は、選んだ点を残す"
    );
    release(&h, to, egui::PointerButton::Primary);
    h.run();
    assert_eq!(
        h.state().state.doc.undo_count(),
        n,
        "Esc で捨てた動きは当てない"
    );
    // もう 1 度の Esc で、選んだ点を外す
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.path_selected_index(), None);
    // フォーカスを失う: そこまでを確定
    press(&h, from, egui::PointerButton::Primary);
    h.step();
    move_to(&h, to);
    h.step();
    h.event(Event::WindowFocused(false));
    h.run();
    assert!(h.state().state.path.drag.is_none());
    assert_eq!(h.state().state.doc.undo_count(), n + 1);
    h.event(Event::WindowFocused(true));
    release(&h, to, egui::PointerButton::Primary);
    h.run();
    assert_eq!(
        h.state().state.doc.undo_count(),
        n + 1,
        "離したのが後から来ても、もう 1 度は当てない"
    );
}

#[test]
fn a_release_outside_the_window_is_not_missed() {
    let mut h = path_app(256);
    for (x, y) in [(50.0, 60.0), (130.0, 190.0)] {
        let at = canvas_pos(&h, x, y);
        click(&mut h, at);
    }
    let n = h.state().state.doc.undo_count();
    let from = canvas_pos(&h, 130.0, 190.0);
    let to = canvas_pos(&h, 110.0, 120.0);
    press(&h, from, egui::PointerButton::Primary);
    h.step();
    move_to(&h, to);
    h.step();
    assert!(h.state().state.path.drag.is_some());
    // 離したイベントが届かないまま、次のフレーム（ボタンは離れている）
    h.step();
    h.step();
    assert!(h.state().state.path.drag.is_none() || h.state().state.doc.undo_count() == n);
}

#[test]
fn delete_and_backspace_remove_the_selected_point_and_the_buttons_follow_the_state() {
    let mut h = path_app(256);
    for (x, y) in [(50.0, 60.0), (130.0, 190.0), (210.0, 80.0)] {
        let at = canvas_pos(&h, x, y);
        click(&mut h, at);
    }
    // 真ん中の点を選んで Delete
    let mid = canvas_pos(&h, 130.0, 190.0);
    click(&mut h, mid);
    assert_eq!(h.state().state.path_selected_index(), Some(1));
    key(&h, Key::Delete, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.path_layer().unwrap().1.point_count(), 2);
    // 選んでいる点が無ければ最後の点（Backspace）
    key(&h, Key::Backspace, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.path_layer().unwrap().1.point_count(), 1);
    // 閉じるは 3 点以上
    for close in h.get_all_by_label("閉じる") {
        assert!(
            close.accesskit_node().is_disabled(),
            "点が足りない間は押せない"
        );
    }
    // 文字を打っているあいだは Delete が点を消さない（名前の入力欄）
    assert_eq!(h.state().state.path_layer().unwrap().1.point_count(), 1);
    // Undo でも戻る
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(h.state().state.path_layer().unwrap().1.point_count(), 2);
}

#[test]
fn the_panel_and_options_bar_show_names_and_values_in_both_languages_without_instruction_text() {
    let mut h = path_app(256);
    for (x, y) in [(50.0, 60.0), (130.0, 190.0), (210.0, 80.0), (60.0, 220.0)] {
        let at = canvas_pos(&h, x, y);
        click(&mut h, at);
    }
    for label in [
        "パス",
        "パスのブラシ",
        "閉じる",
        "点を消す",
        "ブラシを使う",
        "ラスタライズ",
        "太さ",
        "直径",
        "硬さ",
        "間隔",
        "不透明度",
        "流量",
    ] {
        let n = h.get_all_by_label(label).count();
        assert!(n >= 1, "{label}");
    }
    assert_eq!(h.state().state.path_layer().unwrap().1.point_count(), 4);
    // 名前と値と短い状態だけで、使い方の説明の文は置かない
    no_instruction_text(&mut h, Lang::Ja, "パネル");
    // 閉じる→開く
    let close = h.get_all_by_label("閉じる").next().unwrap().rect().center();
    click(&mut h, close);
    assert!(yolu_app::pathtool::edit::path_is_closed(
        h.state().state.path_layer().unwrap().1
    ));
    assert!(h.get_all_by_label("開く").count() >= 1);
    h.state_mut().state.set_language(Lang::En);
    h.run();
    for label in [
        "Path",
        "Path Brush",
        "Open",
        "Delete Point",
        "Use Brush",
        "Rasterize",
        "Width",
        "Size",
        "Hardness",
        "Spacing",
        "Opacity",
        "Flow",
    ] {
        assert!(h.get_all_by_label(label).count() >= 1, "{label}");
    }
    // 閉じたパスは終わりに始めの点の複製を持つ（4 点の輪は 5 点）
    assert_eq!(h.state().state.path_layer().unwrap().1.point_count(), 5);
    no_instruction_text(&mut h, Lang::En, "panel");
    h.snapshot("path_panel_english");
}

#[test]
fn rasterize_is_off_for_a_fill_layer_in_the_options_bar_the_panel_and_the_layer_menu() {
    let mut h = path_app(256);
    let raster = h.state().state.selected_layer.unwrap();
    h.state_mut()
        .state
        .apply(Action::M2(yolu_app::m2::Edit::NewFill));
    h.run();
    let fill = h.state().state.selected_layer.unwrap();
    assert_ne!(fill, raster);
    for (x, y) in [(50.0, 60.0), (130.0, 190.0)] {
        let at = canvas_pos(&h, x, y);
        click(&mut h, at);
    }
    assert!(h.state().state.doc.layer(fill).unwrap().has_paths());
    assert!(!h.state().state.path_can_rasterize(fill));
    // オプションバーとパスの欄のボタンは、どちらも押せない
    let nodes: Vec<_> = h.get_all_by_label("ラスタライズ").collect();
    assert!(
        nodes.len() >= 2,
        "オプションバーと欄の 2 つ: {}",
        nodes.len()
    );
    assert!(
        nodes.iter().all(|n| n.accesskit_node().is_disabled()),
        "塗りつぶしレイヤーのパスは画素にできない"
    );
    // レイヤーの右クリックのメニューも押せない（理由はツールチップ）
    let rasterize = |h: &Harness<'_, YoluApp>, layer| {
        yolu_app::shell::popup_entries(
            &h.state().state,
            yolu_app::state::PopupKind::LayerContext(layer),
        )
        .into_iter()
        .find_map(|e| match e {
            yolu_app::ui::menu::Entry::Item {
                action: Action::Path(PathAction::Rasterize(_)),
                enabled,
                tooltip,
                ..
            } => Some((enabled, tooltip)),
            _ => None,
        })
    };
    let (enabled, tooltip) = rasterize(&h, fill).expect("パスのあるレイヤーには出る");
    assert!(!enabled && tooltip.is_some_and(|t| t.contains("塗りつぶし")));
    // ラスターレイヤーのパスは、これまでどおり画素にできる
    h.state_mut().state.selected_layer = Some(raster);
    h.state_mut().state.apply(Action::SelectTool(Tool::Path));
    h.run();
    for (x, y) in [(50.0, 60.0), (130.0, 190.0)] {
        let at = canvas_pos(&h, x, y);
        click(&mut h, at);
    }
    let layer = h.state().state.selected_layer.unwrap();
    assert!(h.state().state.path_can_rasterize(layer));
    let (enabled, _) = rasterize(&h, layer).expect("パスのあるレイヤーには出る");
    assert!(enabled);
    assert!(h
        .get_all_by_label("ラスタライズ")
        .all(|n| !n.accesskit_node().is_disabled()));
}

#[test]
fn rasterizing_from_the_panel_and_the_layer_menu_removes_the_path() {
    let mut h = path_app(256);
    for (x, y) in [(50.0, 60.0), (130.0, 190.0)] {
        let at = canvas_pos(&h, x, y);
        click(&mut h, at);
    }
    let ras = h
        .get_all_by_label("ラスタライズ")
        .last()
        .unwrap()
        .rect()
        .center();
    click(&mut h, ras);
    assert!(h.state().state.path_layer().is_none());
    assert_eq!(h.state().state.message, "ラスタライズしました。");
    h.state_mut().state.apply(Action::Undo);
    h.run();
    assert!(h.state().state.path_layer().is_some());
    // レイヤーの右クリックのメニュー
    let layer = h.state().state.selected_layer.unwrap();
    let entries = yolu_app::shell::popup_entries(
        &h.state().state,
        yolu_app::state::PopupKind::LayerContext(layer),
    );
    assert!(entries.iter().any(|e| matches!(
        e,
        yolu_app::ui::menu::Entry::Item {
            action: Action::Path(PathAction::Rasterize(_)),
            ..
        }
    )));
    // パスの無いレイヤーには出ない
    let other = h.state().state.doc.layers()[0].id();
    let entries = yolu_app::shell::popup_entries(
        &h.state().state,
        yolu_app::state::PopupKind::LayerContext(other),
    );
    assert!(!entries.iter().any(|e| matches!(
        e,
        yolu_app::ui::menu::Entry::Item {
            action: Action::Path(_),
            ..
        }
    )));
}

#[test]
fn view3d_clicks_add_points_and_the_overlay_marks_them() {
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.view3d.load_demo();
    h.state_mut().state.view3d.camera.yaw = -40.0;
    h.state_mut().state.view3d.camera.pitch = 15.0;
    click_tab(&mut h, yolu_app::Tab::View3d);
    key(&h, Key::P, Modifiers::NONE);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    let screen_of = |h: &Harness<'_, YoluApp>, p: Vec3| {
        let view = h
            .state()
            .state
            .view3d
            .camera
            .view(rect.width(), rect.height());
        let q = view.to_screen(p).expect("カメラの前");
        pos2(rect.left() + q.x, rect.top() + q.y)
    };
    for p in [
        Vec3::new(0.5, 0.1, -0.3),
        Vec3::new(0.5, 0.2, 0.2),
        Vec3::new(0.2, 0.2, -0.5),
    ] {
        let at = screen_of(&h, p);
        click(&mut h, at);
    }
    let s = &h.state().state;
    assert_eq!(s.doc.layers().len(), 2, "{}", s.message);
    assert_eq!(s.path_layer().unwrap().1.point_count(), 3, "{}", s.message);
    assert!(matches!(s.path_layer().unwrap().1, LayerPath::Surface(_)));
    // 3D の絵に橙の点の印が見える
    let image = h.render().expect("描ける");
    let p = screen_of(&h, Vec3::new(0.5, 0.1, -0.3));
    let mut found = false;
    for dy in -6..=6 {
        for dx in -6..=6 {
            let px = image
                .get_pixel((p.x as i32 + dx) as u32, (p.y as i32 + dy) as u32)
                .0;
            if px[0] > 200 && px[1] > 150 && px[2] < 120 {
                found = true;
            }
        }
    }
    assert!(found, "点の印");
    {
        let (_, p) = h.state().state.path_layer().unwrap();
        assert!(!p.is_canvas());
        assert_eq!(p.point_count(), 3);
    }
    // GL と Vulkan の市松の境界で 1 画素だけ標本位置が変わる。点の印は上で直接確かめる。
    // 通常の色差の閾値は変えず、この画像だけ 1 画素を許す（基準画像は撮り直さない）。
    let mut options = egui_kittest::SnapshotOptions::new();
    options.max_failed_pixels = options.max_failed_pixels.max(1);
    h.snapshot_options("path_view3d_points", &options);
    // 1 回の Undo ずつ戻る
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(h.state().state.path_layer().unwrap().1.point_count(), 2);
}

// ───────── 言語と文字の収まり ─────────

fn drawn_texts(shape: &Shape, out: &mut Vec<String>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|s| drawn_texts(s, out)),
        Shape::Text(text) => out.push(text.galley.job.text.clone()),
        _ => {}
    }
}

/// 描いた文字が、描く先のクリップの中に横にはみ出していない。
fn clipped_texts(h: &Harness<'_, YoluApp>) -> Vec<String> {
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

fn sized_app(width: f32, height: f32, lang: Lang) -> Harness<'static, YoluApp> {
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(width, height))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(common::shared_gpu::renderer())
        .build_eframe(move |cc| {
            YoluApp::for_context(
                &cc.egui_ctx,
                AppState::new_in(256, 256, lang),
                yolu_app::pen::PenInput::detached(),
            )
            .with_render_state(cc.wgpu_render_state.as_ref())
        });
    // 中央は 1 つの組（`common::app` と同じ並び）
    h.state_mut().dock = common::tabbed_center_dock(width);
    h.run();
    h
}

/// 狭い英語の画面では、パスのブラシのアンチエイリアスの名前を上の行に置き、値の箱を詰めずに 1 行に出す（ほかの選びは名前と箱が 1 行）。
#[test]
fn snapshot_the_narrow_path_panel_puts_the_anti_alias_name_above_its_box() {
    let mut h = sized_app(960.0, 640.0, Lang::En);
    key(&h, Key::P, Modifiers::NONE);
    h.run();
    let aa = h.get_by_label("Anti-aliasing: Medium").rect();
    let kind = h.get_by_label("Type: Stroke").rect();
    assert!(
        aa.left() + 20.0 < kind.left(),
        "名前が上の行にあれば、箱は行の左から始まる: {aa:?} {kind:?}"
    );
    h.event(Event::PointerGone);
    h.step();
    h.snapshot("path_panel_narrow_english");
}

/// 画面に描いた文字（重複なし）。
fn screen_texts(h: &Harness<'_, YoluApp>) -> std::collections::BTreeSet<String> {
    let mut texts = Vec::new();
    for shape in &h.output().shapes {
        drawn_texts(&shape.shape, &mut texts);
    }
    texts
        .into_iter()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .collect()
}

/// 説明の文（使い方・状態の説明）に見える文字か: 文の終わりの印がある、「〜ます」「〜です」「〜ください」で終わる、
/// 操作の動詞（クリック・ドラッグ・Click・Drag・Press）で始まる、または名前・値・短い状態にしては長い。
fn looks_like_instruction(text: &str) -> bool {
    let limit = if has_japanese(text) { 24 } else { 36 };
    text.ends_with(['。', '.', '!', '！', '?', '？', ':', '：'])
        || text.contains('。')
        || text.ends_with("ます")
        || text.ends_with("です")
        || text.ends_with("ください")
        || text.contains("クリック")
        || text.contains("ドラッグ")
        || ["Click", "Double-click", "Drag", "Press", "Hold", "Tap"]
            .iter()
            .any(|verb| text.starts_with(verb))
        || text.chars().count() > limit
}

#[test]
fn the_instruction_text_check_tells_sentences_from_names() {
    for sentence in [
        "曲線をクリックして点を追加します",
        "点を掴んでドラッグすると動かせます。",
        "キャンバスをクリックしてください",
        "Click the canvas to add a point",
        "Drag a point to move it.",
        "The path is drawn over the brush from the point list",
    ] {
        assert!(looks_like_instruction(sentence), "{sentence}");
    }
    for name in [
        "パス",
        "太さで直径を変える",
        "複数のチャンネルを一度に塗る",
        "点を消す",
        "ブラシを使う",
        "2D · 4 点 · カラー",
        "Paint several channels at once",
        "Width changes the opacity",
        "3D · 2 points · Color, Roughness",
        "Delete Point",
    ] {
        assert!(!looks_like_instruction(name), "{name}");
    }
}

/// パスのツールだけの画面（同じ状態で、ツールをブラシにしたときに無い文字）に、説明の文が無い。見るのは、パスのツールの帯・プロパティ・
/// オプションバーで増える文字だけ（状態の帯の知らせなど、ツールに関係なく出るものは、別の試験が短さを確かめる）。
fn no_instruction_text(h: &mut Harness<'static, YoluApp>, lang: Lang, what: &str) {
    let with_path = screen_texts(h);
    let selected = h.state().state.path.selected;
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    h.run();
    let without = screen_texts(h);
    h.state_mut().state.apply(Action::SelectTool(Tool::Path));
    h.state_mut().state.path.selected = selected;
    h.run();
    let only_path: Vec<&String> = with_path.difference(&without).collect();
    assert!(
        only_path.len() >= 4,
        "{lang:?} {what}: パスのツールの画面の文字が見つからない: {only_path:?}"
    );
    let sentences: Vec<&&String> = only_path
        .iter()
        .filter(|t| looks_like_instruction(t))
        .collect();
    assert!(
        sentences.is_empty(),
        "{lang:?} {what}: 説明の文がある: {sentences:?}"
    );
}

/// パスのツールの画面（2D のパス・3D のパス・点を選んだ状態）の文字が、日英どちらでも欄に収まり、詰められず、英語に日本語が残らない。
#[test]
fn the_path_tool_text_fits_without_truncation_and_stays_in_the_language() {
    yolu_app::ui::widgets::record_truncations(true);
    for (width, height) in [(1280.0, 800.0), (960.0, 640.0)] {
        for lang in Lang::ALL {
            let mut h = sized_app(width, height, lang);
            key(&h, Key::P, Modifiers::NONE);
            h.run();
            let check = |h: &mut Harness<'static, YoluApp>, what: &str| {
                yolu_app::ui::widgets::take_truncations();
                h.step();
                // 最小のウィンドウで詰まる、ほかのパネルの既知の文字（gui_shell/i18n.rs の一覧）は数えない
                const KNOWN: [&str; 18] = [
                    "エミッション",
                    "カスタム",
                    "カラー",
                    "テクスチャセット 1",
                    "ノーマル",
                    "ハイト",
                    "ペイント",
                    "テクスチャセット 1",
                    "Texture Set 1",
                    "メタリック",
                    "ラフネス",
                    "手ぶれ補正と入り抜き",
                    "Color",
                    "Custom",
                    "Emission",
                    "Height",
                    "Metallic",
                    "Normal",
                ];
                let truncated: Vec<String> = yolu_app::ui::widgets::take_truncations()
                    .into_iter()
                    .filter(|t| !KNOWN.contains(&t.as_str()) && t != "Paint" && t != "Roughness")
                    .collect();
                assert!(
                    truncated.is_empty(),
                    "{lang:?} {width}x{height} {what}: {truncated:#?}"
                );
                let clipped = clipped_texts(h);
                assert!(
                    clipped.is_empty(),
                    "{lang:?} {width}x{height} {what}: {clipped:#?}"
                );
                no_instruction_text(h, lang, &format!("{width}x{height} {what}"));
                if lang == Lang::En {
                    let mut texts = Vec::new();
                    for shape in &h.output().shapes {
                        drawn_texts(&shape.shape, &mut texts);
                    }
                    let left: Vec<_> = texts.into_iter().filter(|t| has_japanese(t)).collect();
                    assert!(
                        left.is_empty(),
                        "{lang:?} {width}x{height} {what}: {left:?}"
                    );
                }
            };
            check(&mut h, "no path");
            for (x, y) in [(50.0, 60.0), (130.0, 190.0), (210.0, 80.0), (60.0, 220.0)] {
                let at = canvas_pos(&h, x, y);
                click(&mut h, at);
            }
            check(&mut h, "2D path");
            h.state_mut()
                .state
                .apply(Action::Path(PathAction::Point(PointOp::Close)));
            h.run();
            check(&mut h, "closed");
            h.state_mut()
                .state
                .apply(Action::Mat(MatAction::Enabled(true)));
            h.run();
            check(&mut h, "material");
            let layer = h.state().state.selected_layer.unwrap();
            h.state_mut()
                .state
                .apply(Action::Path(PathAction::Rasterize(layer)));
            // 3D のパス
            h.state_mut().state.view3d.load_demo();
            click_tab(&mut h, yolu_app::Tab::View3d);
            h.state_mut().state.apply(Action::NewLayer);
            h.run();
            let rect = h.state().view3d_rect().expect("3D のタブを描いた");
            for p in [Vec3::new(0.5, 0.1, -0.3), Vec3::new(0.5, 0.2, 0.2)] {
                let view = h
                    .state()
                    .state
                    .view3d
                    .camera
                    .view(rect.width(), rect.height());
                let q = view.to_screen(p).expect("カメラの前");
                click(&mut h, pos2(rect.left() + q.x, rect.top() + q.y));
            }
            assert!(
                matches!(
                    h.state().state.path_layer().map(|(_, p)| p.is_canvas()),
                    Some(false)
                ),
                "{}",
                h.state().state.message
            );
            check(&mut h, "3D path");
        }
    }
    yolu_app::ui::widgets::record_truncations(false);
}

// ───────── 確かめの絵（一覧・取っ手・右クリック・種類・筆先・プリセット・塗りつぶしレイヤー。日英） ─────────

fn review_app(lang: Lang) -> Harness<'static, YoluApp> {
    let mut h = app(1360.0, 1400.0, 256);
    h.state_mut().state.set_language(lang);
    key(&h, Key::P, Modifiers::NONE);
    h.run();
    h
}

fn act(h: &mut Harness<'_, YoluApp>, a: PathAction) {
    h.state_mut().state.apply(Action::Path(a));
    h.run();
}

/// 3 本のパスの一覧（1 本は隠し・名前つき）と、取っ手の点。
fn three_paths(h: &mut Harness<'_, YoluApp>) {
    for line in [
        [(30.0, 40.0), (120.0, 90.0), (220.0, 50.0)],
        [(30.0, 200.0), (128.0, 150.0), (220.0, 210.0)],
        [(40.0, 120.0), (128.0, 128.0), (210.0, 120.0)],
    ] {
        for (x, y) in line {
            let at = canvas_pos(h, x, y);
            click(h, at);
        }
        act(h, PathAction::SelectPath(None));
    }
    let ids: Vec<u128> = h
        .state()
        .state
        .path_entries()
        .iter()
        .map(|e| e.id())
        .collect();
    let name = match h.state().state.lang {
        Lang::Ja => "縁取り",
        Lang::En => "Outline",
    };
    act(h, PathAction::List(ListOp::Rename(ids[0], name.into())));
    act(h, PathAction::List(ListOp::Visible(ids[2], false)));
    act(h, PathAction::SelectPath(Some(ids[1])));
    act(h, PathAction::Select(Some(1)));
    act(h, PathAction::SetHandles(true));
}

#[test]
fn review_pictures_of_the_list_handles_and_context_menu() {
    let mut shots = egui_kittest::SnapshotResults::new();
    for (lang, tag) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let mut h = review_app(lang);
        three_paths(&mut h);
        h.snapshot(format!("path_review_list_{tag}"));
        // 一覧の行の右クリック
        let row = rect_of(
            &h,
            if lang == Lang::Ja {
                "縁取り"
            } else {
                "Outline"
            },
            |r| r.left() < 320.0,
        );
        press(&h, row.center(), egui::PointerButton::Secondary);
        h.step();
        release(&h, row.center(), egui::PointerButton::Secondary);
        h.run();
        assert!(h.state().state.popup.is_some());
        h.snapshot(format!("path_review_context_{tag}"));
        no_instruction_text(&mut h, lang, "path review");
        shots.extend_harness(&mut h);
    }
}

#[test]
fn renaming_a_preset_from_its_menu_puts_a_name_field_in_place_of_the_button() {
    use yolu_app::pathtool::presets::PresetOp;
    let mut shots = egui_kittest::SnapshotResults::new();
    for (lang, tag) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let mut h = review_app(lang);
        for (x, y) in [(30.0, 128.0), (226.0, 128.0)] {
            let at = canvas_pos(&h, x, y);
            click(&mut h, at);
        }
        act(&mut h, PathAction::Preset(PresetOp::Save));
        let preset = h.state().state.path.presets.list()[0].clone();
        let button_label = lang.pick("プリセット", "Presets");
        let button = rect_of(&h, button_label, |r| r.left() < 320.0);
        click(&mut h, button.center());
        let rename = popup_item(&h, lang.pick("プリセットの名前を変更", "Rename Preset"));
        click(&mut h, rename.center());
        let item = rect_of(&h, &preset.name, |r| r.left() > rename.right() - 4.0);
        click(&mut h, item.center());
        assert!(h.state().state.popup.is_none());
        assert_eq!(h.state().state.path.preset_renaming, Some(preset.id));
        assert!(
            h.query_all_by_label(button_label)
                .all(|n| n.rect().left() >= 320.0),
            "ボタンの所が名前の入力欄になる"
        );
        // Esc でやめる（名前も、選んだ点も、パスの編集も変わらない）
        let point = h.state().state.path_selected_index();
        assert!(point.is_some());
        h.event(Event::Text("x".into()));
        key(&h, Key::Escape, Modifiers::NONE);
        h.run();
        assert_eq!(h.state().state.path.preset_renaming, None);
        assert_eq!(h.state().state.path.presets.list()[0].name, preset.name);
        assert_eq!(
            h.state().state.path_selected_index(),
            point,
            "入力欄の Esc は点を外さない"
        );
        assert!(
            h.state().state.path_layer().is_some(),
            "Esc はパスの編集を抜けない"
        );
        // 一覧の行の名前の入力欄の Esc も同じ
        let row = rect_of(&h, lang.pick("パス 1", "Path 1"), |r| r.left() < 320.0);
        let at = pos2(row.center().x, row.center().y);
        for _ in 0..2 {
            press(&h, at, egui::PointerButton::Primary);
            release(&h, at, egui::PointerButton::Primary);
            h.step();
        }
        h.run();
        assert!(h.state().state.path.renaming.is_some());
        // 一覧の行を押すと点の選びは外れるので、選び直してから
        act(&mut h, PathAction::Select(point));
        assert!(h.state().state.path.renaming.is_some());
        key(&h, Key::Escape, Modifiers::NONE);
        h.run();
        assert!(h.state().state.path.renaming.is_none());
        assert_eq!(h.state().state.path_selected_index(), point);
        // 入力欄の外の Esc は今までどおり点を外す
        key(&h, Key::Escape, Modifiers::NONE);
        h.run();
        assert_eq!(h.state().state.path_selected_index(), None);
        act(&mut h, PathAction::Select(point));
        // Enter で決める
        click(&mut h, button.center());
        let rename = popup_item(&h, lang.pick("プリセットの名前を変更", "Rename Preset"));
        click(&mut h, rename.center());
        let item = rect_of(&h, &preset.name, |r| r.left() > rename.right() - 4.0);
        click(&mut h, item.center());
        let name = lang.pick("太い線", "Thick");
        key(&h, Key::A, Modifiers::COMMAND);
        h.event(Event::Text(name.into()));
        h.run();
        h.snapshot(format!("path_review_preset_rename_{tag}"));
        key(&h, Key::Enter, Modifiers::NONE);
        h.run();
        assert_eq!(h.state().state.path.presets.list()[0].name, name);
        assert_eq!(h.state().state.path.preset_renaming, None);
        rect_of(&h, button_label, |r| r.left() < 320.0);
        no_instruction_text(&mut h, lang, "path preset rename");
        shots.extend_harness(&mut h);
    }
}

#[test]
fn review_pictures_of_the_kinds_tips_and_presets() {
    let mut shots = egui_kittest::SnapshotResults::new();
    for (lang, tag) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let mut h = review_app(lang);
        for (x, y) in [(30.0, 128.0), (128.0, 60.0), (226.0, 128.0)] {
            let at = canvas_pos(&h, x, y);
            click(&mut h, at);
        }
        // リボン（伸ばす）
        let rid = h
            .state_mut()
            .state
            .shelf
            .add_image(lang, "band", &[255, 80, 0, 255, 0, 120, 255, 255], 2, 1)
            .unwrap();
        let image = yolu_app::fx::inputs::image_id(&rid).unwrap();
        act(
            &mut h,
            PathAction::Kind(PathKind::Ribbon(core_paths::Ribbon {
                image,
                mode: core_paths::RibbonMode::Tile,
                spacing: 1.0,
            })),
        );
        act(&mut h, PathAction::Brush(BrushEdit::Diameter(40.0)));
        h.snapshot(format!("path_review_ribbon_{tag}"));
        // ストローク・画像の筆先・沿う向き
        act(&mut h, PathAction::Kind(PathKind::Stroke));
        act(
            &mut h,
            PathAction::Style(StyleEdit::Tip(yolu_core::builtin_tip("bristles"))),
        );
        act(&mut h, PathAction::Style(StyleEdit::Follow(true)));
        h.snapshot(format!("path_review_tip_{tag}"));
        // プリセットの一覧（1 つ保存してから。置き場の無いプリセットは画面の中だけ）
        act(
            &mut h,
            PathAction::Preset(yolu_app::pathtool::presets::PresetOp::Save),
        );
        let presets = rect_of(
            &h,
            if lang == Lang::Ja {
                "プリセット"
            } else {
                "Presets"
            },
            |r| r.left() < 320.0,
        );
        click(&mut h, presets.center());
        assert!(h.state().state.popup.is_some());
        let rename = popup_item(
            &h,
            if lang == Lang::Ja {
                "プリセットの名前を変更"
            } else {
                "Rename Preset"
            },
        );
        hover_and_wait(&mut h, rename.center());
        h.snapshot(format!("path_review_presets_{tag}"));
        no_instruction_text(&mut h, lang, "path review kinds");
        shots.extend_harness(&mut h);
    }
}

#[test]
fn review_pictures_of_the_3d_handles_and_depth_and_a_fill_layer() {
    let mut shots = egui_kittest::SnapshotResults::new();
    for (lang, tag) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let mut h = review_app(lang);
        h.state_mut().state.view3d.load_demo();
        h.state_mut().state.view3d.camera.yaw = -40.0;
        h.state_mut().state.view3d.camera.pitch = 15.0;
        // 中央の表示域は右の列が広い分だけ狭いので、3 つの点が収まるようにカメラを引く
        h.state_mut().state.view3d.camera.distance *= 1.3;
        click_tab(&mut h, yolu_app::Tab::View3d);
        let rect = h.state().view3d_rect().expect("3D のタブを描いた");
        let screen_of = |h: &Harness<'_, YoluApp>, p: Vec3| {
            let view = h
                .state()
                .state
                .view3d
                .camera
                .view(rect.width(), rect.height());
            let q = view.to_screen(p).expect("カメラの前");
            pos2(rect.left() + q.x, rect.top() + q.y)
        };
        for p in [
            Vec3::new(0.5, 0.0, -0.3),
            Vec3::new(0.5, 0.25, 0.0),
            Vec3::new(0.5, 0.0, 0.25),
        ] {
            let at = screen_of(&h, p);
            assert!(rect.contains(at), "{at:?} {rect:?}");
            click(&mut h, at);
        }
        assert_eq!(
            h.state().state.path_layer().map(|(_, p)| p.point_count()),
            Some(3),
            "{}",
            h.state().state.message
        );
        act(&mut h, PathAction::Select(Some(1)));
        act(&mut h, PathAction::SetHandles(true));
        act(&mut h, PathAction::Style(StyleEdit::Depth(Some(2.0))));
        h.snapshot(format!("path_review_3d_{tag}"));
        shots.extend_harness(&mut h);
        // 塗りつぶしレイヤーの欄のパスの節
        let mut f = review_app(lang);
        f.state_mut().state.color.set_main([0.85, 0.75, 0.4, 1.0]);
        f.state_mut()
            .state
            .apply(Action::M2(yolu_app::m2::Edit::NewFill));
        f.run();
        let fill = f.state().state.selected_layer.unwrap();
        f.state_mut().state.color.set_main([0.1, 0.2, 0.9, 1.0]);
        for (x, y) in [(30.0, 128.0), (128.0, 60.0), (226.0, 128.0)] {
            let at = canvas_pos(&f, x, y);
            click(&mut f, at);
        }
        assert!(f.state().state.doc.layer(fill).unwrap().has_paths());
        f.state_mut().state.apply(Action::SelectTool(Tool::Brush));
        // 塗りつぶしレイヤーの欄の下（パスの節）まで送る
        f.state_mut().state.m2.props_scroll = 100_000.0;
        f.run();
        f.snapshot(format!("path_review_fill_layer_{tag}"));
        shots.extend_harness(&mut f);
    }
}
