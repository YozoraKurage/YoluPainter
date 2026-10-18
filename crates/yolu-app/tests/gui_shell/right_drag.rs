//! 右ボタンを押したまま動かす操作: 押し始めた所が振る舞いを決める。一覧で押し始めたらその一覧がスクロールし（中身がポインタについて来る向き。
//! 外へ出ても続く）、ツールの帯で押し始めたら通ったツールを選ぶ。キャンバスで押し始めた右ボタンは今のまま（スポイト）。動かさずに離せば今の
//! 右クリック（メニュー）。開いているメニューの中は効かない。並べ替えは左ボタンだけで始まる。
use crate::common;
use crate::common::drive::Drive;

use common::*;
use egui::{pos2, vec2, Event, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::state::{Action, PopupKind, Tool};
use yolu_app::{Tab, YoluApp};

fn right_press(h: &mut Harness<'_, YoluApp>, at: Pos2) {
    press(h, at, PointerButton::Secondary);
    h.step();
}

fn right_move(h: &mut Harness<'_, YoluApp>, at: Pos2) {
    move_to(h, at);
    h.step();
}

fn right_release(h: &mut Harness<'_, YoluApp>, at: Pos2) {
    release(h, at, PointerButton::Secondary);
    h.step();
    h.run();
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

/// レイヤーが 30 枚あり、一覧を `scroll` だけ送った 1280×800 のウィンドウ。
fn layers_window(scroll: f32) -> Harness<'static, YoluApp> {
    let mut h = app(1280.0, 800.0, 128);
    for _ in 0..30 {
        h.act(Action::NewLayer);
        h.run();
    }
    h.ui_mut().layer_scroll = scroll;
    h.run();
    h
}

/// レイヤーの一覧の行の上の点（つまみの幅の外）。
fn on_list(h: &Harness<'_, YoluApp>) -> Pos2 {
    let body = layers_body(h);
    pos2(body.left() + 90.0, body.center().y)
}

fn order(h: &Harness<'_, YoluApp>) -> Vec<yolu_app::engine::LayerId> {
    h.doc().layers().iter().map(|l| l.id()).collect()
}

/// 一覧の最後まで送ったときの `layer_scroll`（ずらせる最大の量）。
fn max_scroll(h: &mut Harness<'_, YoluApp>) -> f32 {
    h.ui_mut().layer_scroll = 1.0e6;
    h.run();
    h.ui().layer_scroll
}

#[test]
fn a_right_drag_on_the_layers_list_scrolls_it_by_grabbing_and_does_not_reorder_or_open_the_menu() {
    let mut h = layers_window(300.0);
    let start = on_list(&h);
    let (ids, selected) = (order(&h), h.selected_layer());
    right_press(&mut h, start);
    assert!(near(h.ui().layer_scroll, 300.0), "押しただけでは動かない");
    // 遊びを超えた最初の点から数える: 上へ 50 動かすと 50 増える（掴んで動かす）
    right_move(&mut h, vec2(0.0, -10.0).to_pos(start));
    assert!(near(h.ui().layer_scroll, 300.0), "動かし始めの点が基準");
    right_move(&mut h, vec2(0.0, -30.0).to_pos(start));
    right_move(&mut h, vec2(0.0, -60.0).to_pos(start));
    assert!(near(h.ui().layer_scroll, 350.0), "{}", h.ui().layer_scroll);
    // 下へ戻すと減る
    right_move(&mut h, vec2(0.0, 40.0).to_pos(start));
    assert!(near(h.ui().layer_scroll, 250.0), "{}", h.ui().layer_scroll);
    assert!(h.ui().layer_drag.is_none(), "並べ替えは始まらない");
    right_release(&mut h, vec2(0.0, 40.0).to_pos(start));
    assert_eq!(order(&h), ids, "レイヤーの並びは変わらない");
    assert_eq!(h.selected_layer(), selected, "押した行を選ばない");
    assert!(h.popup().is_none(), "動かしたらメニューは出ない");
    // 離したら止まる
    let after = h.ui().layer_scroll;
    right_move(&mut h, vec2(0.0, -100.0).to_pos(start));
    assert!(near(h.ui().layer_scroll, after));
}

#[test]
fn a_right_drag_on_the_layers_list_stays_in_range() {
    let mut h = layers_window(0.0);
    let max = max_scroll(&mut h);
    assert!(max > 100.0, "{max}");
    let start = on_list(&h);
    right_press(&mut h, start);
    right_move(&mut h, vec2(0.0, -10.0).to_pos(start));
    // 行き過ぎても最後まで。そこから戻すと、行き過ぎた分を覚えず、戻した分だけすぐ付いて来る
    right_move(&mut h, vec2(0.0, -300.0).to_pos(start));
    assert!(near(h.ui().layer_scroll, max), "最後で止まる");
    right_move(&mut h, vec2(0.0, -280.0).to_pos(start));
    assert!(
        near(h.ui().layer_scroll, max - 20.0),
        "{}",
        h.ui().layer_scroll
    );
    right_release(&mut h, start);
    // 先頭でも同じ
    h.ui_mut().layer_scroll = 50.0;
    h.run();
    right_press(&mut h, start);
    right_move(&mut h, vec2(0.0, 10.0).to_pos(start));
    right_move(&mut h, vec2(0.0, 150.0).to_pos(start));
    assert!(near(h.ui().layer_scroll, 0.0), "先頭で止まる");
    right_release(&mut h, start);
}

#[test]
fn a_right_click_without_moving_still_opens_the_layer_menu() {
    let mut h = layers_window(300.0);
    let start = on_list(&h);
    right_press(&mut h, start);
    // 遊びの内の動きは数えない
    right_move(&mut h, vec2(0.0, -3.0).to_pos(start));
    right_release(&mut h, vec2(0.0, -3.0).to_pos(start));
    assert!(near(h.ui().layer_scroll, 300.0));
    assert!(
        matches!(
            h.popup().as_ref().map(|p| p.kind),
            Some(PopupKind::LayerContext(_))
        ),
        "{:?}",
        h.popup().as_ref().map(|p| p.kind)
    );
}

#[test]
fn a_right_drag_that_starts_in_the_list_keeps_scrolling_over_the_canvas_and_the_canvas_does_nothing(
) {
    let mut h = layers_window(300.0);
    let max = max_scroll(&mut h);
    h.ui_mut().layer_scroll = 300.0;
    h.run();
    let start = on_list(&h);
    let canvas = canvas_rect(&h).center();
    let (revision, main) = (h.doc().revision(), h.color().main);
    right_press(&mut h, start);
    right_move(&mut h, vec2(0.0, -10.0).to_pos(start));
    right_move(&mut h, canvas);
    let expected = (300.0 - (canvas.y - (start.y - 10.0))).clamp(0.0, max);
    assert!(
        near(h.ui().layer_scroll, expected) && expected != 300.0,
        "{} {expected}",
        h.ui().layer_scroll
    );
    right_move(&mut h, vec2(0.0, 25.0).to_pos(canvas));
    let expected = (300.0 - (canvas.y + 25.0 - (start.y - 10.0))).clamp(0.0, max);
    assert!(
        near(h.ui().layer_scroll, expected),
        "{}",
        h.ui().layer_scroll
    );
    assert!(h.canvas().eyedrop.is_none() && !h.app().is_stroking());
    right_release(&mut h, canvas);
    assert!(h.canvas().eyedrop.is_none() && !h.app().is_stroking());
    assert_eq!(h.doc().revision(), revision);
    assert_eq!(h.color().main, main, "スポイトは動かない");
    assert!(h.popup().is_none());
}

#[test]
fn a_right_drag_that_starts_on_the_canvas_does_not_scroll_the_list_it_passes_over() {
    let mut h = layers_window(300.0);
    h.act(Action::SelectTool(Tool::Brush));
    h.run();
    let canvas = canvas_rect(&h).center();
    let over = on_list(&h);
    right_press(&mut h, canvas);
    assert!(
        h.canvas().eyedrop.is_some(),
        "キャンバスの右ボタンは今のまま（スポイト）"
    );
    right_move(&mut h, vec2(0.0, -20.0).to_pos(over));
    right_move(&mut h, vec2(0.0, 60.0).to_pos(over));
    right_move(&mut h, vec2(0.0, -80.0).to_pos(over));
    assert!(near(h.ui().layer_scroll, 300.0), "{}", h.ui().layer_scroll);
    assert!(h.canvas().eyedrop.is_some(), "スポイトは続く");
    right_release(&mut h, over);
    assert!(near(h.ui().layer_scroll, 300.0));
}

#[test]
fn the_brush_list_scrolls_with_a_right_drag() {
    let mut h = app(1280.0, 900.0, 128);
    for _ in 0..30 {
        h.get_by_label("今の設定を新しいブラシに").click();
        h.run();
    }
    // ブラシの一覧
    let sub_tab = h.state().tab_rects[&Tab::SubTools];
    let list_at = pos2(sub_tab.left() + 80.0, sub_tab.bottom() + 70.0);
    h.brushes_mut().ui.list_scroll = 200.0;
    h.run();
    assert_eq!(h.brushes().ui.list_scroll, 200.0);
    right_press(&mut h, list_at);
    right_move(&mut h, vec2(0.0, -10.0).to_pos(list_at));
    right_move(&mut h, vec2(0.0, -70.0).to_pos(list_at));
    let scrolled = h.brushes().ui.list_scroll;
    assert!(near(scrolled, 260.0), "{scrolled}");
    right_release(&mut h, vec2(0.0, -70.0).to_pos(list_at));
}

#[test]
fn the_tool_properties_scroll_with_a_right_drag() {
    // 低いウィンドウでは、ツールプロパティの中身があふれる
    let mut h = app(1280.0, 640.0, 128);
    let props_tab = h.state().tab_rects[&Tab::ToolProperties];
    let size_tab = h.state().tab_rects[&Tab::BrushSize];
    let body = egui::Rect::from_min_max(
        pos2(props_tab.left() - 2.0, props_tab.bottom()),
        pos2(props_tab.left() + 300.0, size_tab.top()),
    );
    let brush = Tool::Brush as usize;
    assert_eq!(h.subtools().ui.props_scroll[brush], 0.0);
    let from = pos2(props_tab.left() + 40.0, body.center().y);
    right_press(&mut h, from);
    right_move(&mut h, vec2(0.0, -10.0).to_pos(from));
    right_move(&mut h, vec2(0.0, -50.0).to_pos(from));
    let props = h.subtools().ui.props_scroll[brush];
    assert!(near(props, 40.0), "{props}");
    right_release(&mut h, vec2(0.0, -50.0).to_pos(from));
}

#[test]
fn a_right_drag_inside_an_open_menu_does_not_scroll_it() {
    let mut h = app(1280.0, 420.0, 128);
    let title = menu_title(&h, "フィルター");
    click(&mut h, title.center());
    let body = h.popup().as_ref().expect("メニューが開く").state.rect;
    let inside = body.center();
    right_press(&mut h, inside);
    right_move(&mut h, vec2(0.0, -10.0).to_pos(inside));
    right_move(&mut h, vec2(0.0, -80.0).to_pos(inside));
    let scroll = h.popup().as_ref().map(|p| p.state.scroll);
    assert_eq!(scroll, Some(0.0), "右ドラッグではメニューの中を送らない");
    // 低いウィンドウなので、ホイールなら送れる（右ドラッグを除いたのが意味を持つ）
    if h.popup().is_none() {
        click(&mut h, title.center());
    }
    let inside = h.popup().as_ref().unwrap().state.rect.center();
    move_to(&h, inside);
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -200.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    h.run();
    assert!(h.popup().as_ref().is_some_and(|p| p.state.scroll > 0.0));
}

fn active_slot(h: &Harness<'_, YoluApp>) -> Option<yolu_app::toolset::SlotId> {
    h.toolset().set.active()
}

fn slot_centers(h: &Harness<'_, YoluApp>) -> Vec<Pos2> {
    h.ui()
        .toolset_slot_rects
        .iter()
        .map(|(_, r)| r.center())
        .collect()
}

#[test]
fn a_right_drag_on_the_tool_strip_selects_the_tools_it_passes_over() {
    let mut h = app(1280.0, 800.0, 128);
    let (rects, centers) = (h.ui().toolset_slot_rects.clone(), slot_centers(&h));
    assert!(rects.len() >= 4);
    let before = active_slot(&h);
    right_press(&mut h, centers[0]);
    assert_eq!(active_slot(&h), before, "押しただけでは選ばない");
    right_move(&mut h, centers[1]);
    assert_eq!(active_slot(&h), Some(rects[1].0));
    right_move(&mut h, centers[2]);
    assert_eq!(active_slot(&h), Some(rects[2].0));
    right_move(&mut h, centers[3]);
    assert_eq!(active_slot(&h), Some(rects[3].0));
    // 帯の外へ出たら選ばない（そのまま）。戻れば選ぶ
    let outside = canvas_rect(&h).center();
    right_move(&mut h, outside);
    assert_eq!(active_slot(&h), Some(rects[3].0));
    right_move(&mut h, centers[1]);
    assert_eq!(active_slot(&h), Some(rects[1].0));
    // 最後に乗せたツールで終わる。メニューは出ない
    right_release(&mut h, centers[2]);
    assert_eq!(
        active_slot(&h),
        Some(rects[1].0),
        "離した位置のツールには替わらない"
    );
    assert!(h.popup().is_none());
    right_move(&mut h, centers[0]);
    assert_eq!(active_slot(&h), Some(rects[1].0), "離したあとは選ばない");
}

#[test]
fn a_right_click_on_the_tool_strip_without_moving_opens_its_menu_and_a_drag_from_outside_selects_nothing(
) {
    let mut h = app(1280.0, 800.0, 128);
    let (rects, centers) = (h.ui().toolset_slot_rects.clone(), slot_centers(&h));
    let before = active_slot(&h);
    right_press(&mut h, centers[2]);
    right_release(&mut h, centers[2]);
    assert_eq!(
        h.popup().as_ref().map(|p| p.kind),
        Some(PopupKind::M2(yolu_app::m2_menu::Popup::ToolStripContext))
    );
    assert_eq!(active_slot(&h), before, "動かさずに離しても選ばない");
    *h.popup_mut() = None;
    h.run();
    // 帯の外（キャンバス）で押し始めた右ボタンを、帯の上へ持ってきても選ばない
    let canvas = canvas_rect(&h).center();
    right_press(&mut h, canvas);
    for c in &centers[1..4] {
        right_move(&mut h, *c);
    }
    assert_eq!(active_slot(&h), before);
    right_release(&mut h, centers[3]);
    assert_eq!(active_slot(&h), before);
    assert!(rects.len() >= 4);
}

#[test]
fn the_edit_mode_strip_also_selects_by_right_drag() {
    use yolu_app::mode::{EditTool, EditorMode, ModeAction};
    let mut h = app(1280.0, 800.0, 128);
    h.act(Action::Mode(ModeAction::Set(EditorMode::Edit)));
    h.run();
    let rect = |h: &Harness<'_, YoluApp>, tool: EditTool| {
        rect_of(h, tool.name(h.lang()), |r| {
            r.left() < 60.0 && r.top() > 62.0
        })
    };
    let (a, b, c) = (
        rect(&h, EditTool::Select).center(),
        rect(&h, EditTool::Move).center(),
        rect(&h, EditTool::Scale).center(),
    );
    assert_eq!(EditTool::shown(h.app()), EditTool::Select);
    right_press(&mut h, a);
    right_move(&mut h, b);
    assert_eq!(EditTool::shown(h.app()), EditTool::Move);
    right_move(&mut h, c);
    assert_eq!(EditTool::shown(h.app()), EditTool::Scale);
    right_release(&mut h, c);
    assert_eq!(EditTool::shown(h.app()), EditTool::Scale);
}

#[test]
fn reordering_starts_with_the_left_button_only() {
    let mut h = app(1280.0, 800.0, 128);
    let ids: Vec<_> = h.toolset().set.slots().iter().map(|s| s.id).collect();
    let rects = h.ui().toolset_slot_rects.clone();
    let from = rects[0].1.center();
    let below = pos2(from.x, rects[3].1.center().y + 8.0);
    // 右ボタンでツールの帯のボタンを引いても、並びは変わらない
    right_press(&mut h, from);
    right_move(&mut h, below);
    assert!(h.toolset().ui.dragging().is_none(), "引き始めない");
    right_release(&mut h, below);
    let same: Vec<_> = h.toolset().set.slots().iter().map(|s| s.id).collect();
    assert_eq!(same, ids);
    // 左ボタン（ペンの接触も egui では左ボタン）なら動く
    h.event(touch_start(from));
    press(&h, from, PointerButton::Primary);
    h.step();
    move_to(&h, vec2(0.0, 12.0).to_pos(from));
    h.step();
    move_to(&h, below);
    h.step();
    assert!(
        h.toolset().ui.dragging().is_some(),
        "左ボタンなら引き始める"
    );
    release(&h, below, PointerButton::Primary);
    h.run();
    let moved: Vec<_> = h.toolset().set.slots().iter().map(|s| s.id).collect();
    assert_ne!(moved, ids, "ツールが動いた");
}

trait ToPos {
    fn to_pos(self, from: Pos2) -> Pos2;
}

impl ToPos for egui::Vec2 {
    fn to_pos(self, from: Pos2) -> Pos2 {
        from + self
    }
}

// ───────── 値を動かす部品は左ボタンだけ ─────────

/// 一覧の中に、スライダーと数の入れ物を置いたウィンドウ。
struct Values {
    ready: bool,
    offset: f32,
    view: egui::Rect,
    slider: f32,
    number: f64,
    slider_rect: egui::Rect,
    number_rect: egui::Rect,
    released: usize,
}

fn draw_values(ui: &mut egui::Ui, v: &mut Values) {
    use yolu_app::ui::numfield::{number_field, NumSpec};
    use yolu_app::ui::scroll::Scroll;
    use yolu_app::ui::widgets::{self as w, NumberFormat, SliderSpec};
    if !v.ready {
        YoluApp::setup(ui.ctx());
        v.ready = true;
        ui.ctx().request_repaint();
        return;
    }
    v.view = egui::Rect::from_min_size(ui.max_rect().min + vec2(20.0, 20.0), vec2(220.0, 120.0));
    let bar = Scroll::begin(ui, v.view, 400.0, &mut v.offset);
    let top = v.view.top() - v.offset;
    v.slider_rect = egui::Rect::from_min_size(pos2(v.view.left(), top), vec2(190.0, 30.0));
    v.number_rect = egui::Rect::from_min_size(pos2(v.view.left(), top + 40.0), vec2(120.0, 24.0));
    let format = NumberFormat {
        decimals: 2,
        trim: true,
        suffix: "",
    };
    let out = w::slider(
        ui,
        v.slider_rect,
        "values.slider",
        v.slider,
        &SliderSpec::new("値", 0.0, 100.0, format),
    );
    if out.changed {
        v.slider = out.value;
    }
    v.released += usize::from(out.released);
    let n = number_field(
        ui,
        v.number_rect,
        "values.number",
        "X",
        v.number,
        &NumSpec::new(0.0, 100.0, 0.5, 2),
        None,
        None,
        true,
    );
    if n.changed {
        v.number = n.value;
    }
    bar.end(ui, "values", &mut v.offset);
}

fn values_window() -> Harness<'static, Values> {
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(280.0, 180.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(60)
        .renderer(common::shared_gpu::renderer())
        .build_ui_state(
            draw_values,
            Values {
                ready: false,
                offset: 0.0,
                view: egui::Rect::ZERO,
                slider: 10.0,
                number: 10.0,
                slider_rect: egui::Rect::ZERO,
                number_rect: egui::Rect::ZERO,
                released: 0,
            },
        );
    h.run();
    h
}

fn values_drag(h: &mut Harness<'_, Values>, button: PointerButton, from: Pos2, to: &[Pos2]) {
    h.event(Event::PointerMoved(from));
    h.event(Event::PointerButton {
        pos: from,
        button,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for p in to {
        h.event(Event::PointerMoved(*p));
        h.step();
    }
    h.event(Event::PointerButton {
        pos: *to.last().unwrap_or(&from),
        button,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.run();
}

#[test]
fn the_slider_and_the_number_field_ignore_the_right_button_and_the_list_scrolls_instead() {
    // 右ボタン: 値は変わらず、決めたことにもならず、一覧が送られる
    let mut h = values_window();
    let (slider_at, number_at) = (
        pos2(
            h.state().slider_rect.center().x + 40.0,
            h.state().slider_rect.bottom() - 6.0,
        ),
        h.state().number_rect.center(),
    );
    for (at, what) in [(slider_at, "スライダー"), (number_at, "数の入れ物")] {
        h.state_mut().offset = 0.0;
        h.run();
        values_drag(
            &mut h,
            PointerButton::Secondary,
            at,
            &[
                at + vec2(8.0, -10.0),
                at + vec2(60.0, -50.0),
                at + vec2(-30.0, -70.0),
            ],
        );
        let s = h.state();
        assert_eq!((s.slider, s.number), (10.0, 10.0), "{what}: 値は変わらない");
        assert_eq!(s.released, 0, "{what}: 決めたことにならない");
        assert!(s.offset > 30.0, "{what}: 一覧が送られる {}", s.offset);
    }
    // 左ボタン（ペンの接触も egui では左ボタン）: 値が動く
    let mut h = values_window();
    let at = pos2(
        h.state().slider_rect.center().x + 60.0,
        h.state().slider_rect.bottom() - 6.0,
    );
    values_drag(
        &mut h,
        PointerButton::Primary,
        at,
        &[at + vec2(10.0, 0.0), at + vec2(30.0, 0.0)],
    );
    assert!(
        h.state().slider > 10.0,
        "スライダーは動く {}",
        h.state().slider
    );
    assert_eq!(h.state().released, 1);
    let at = h.state().number_rect.center();
    values_drag(
        &mut h,
        PointerButton::Primary,
        at,
        &[at + vec2(10.0, 0.0), at + vec2(30.0, 0.0)],
    );
    assert!(
        h.state().number > 10.0,
        "数の入れ物は動く {}",
        h.state().number
    );
}

// ───────── 一覧の中のカーブ・ランプ ─────────

struct Editors {
    ready: bool,
    offset: f32,
    view: egui::Rect,
    curve: yolu_core::curve::Curve,
    ramp: yolu_core::generator::Ramp,
    selection: yolu_app::ui::ramp::Selection,
    curve_rect: egui::Rect,
    ramp_rect: egui::Rect,
    changes: usize,
}

fn three_point_curve() -> yolu_core::curve::Curve {
    use yolu_core::curve::{Curve, CurvePoint};
    Curve::new(vec![
        CurvePoint { x: 0., y: 0. },
        CurvePoint { x: 0.5, y: 0.5 },
        CurvePoint { x: 1., y: 1. },
    ])
    .unwrap()
}

fn draw_editors(ui: &mut egui::Ui, e: &mut Editors) {
    use yolu_app::ui::curve::{curve_editor, HEIGHT};
    use yolu_app::ui::ramp::{ops, stops_editor, STOPS_HEIGHT};
    use yolu_app::ui::scroll::Scroll;
    if !e.ready {
        YoluApp::setup(ui.ctx());
        e.ready = true;
        ui.ctx().request_repaint();
        return;
    }
    e.view = egui::Rect::from_min_size(ui.max_rect().min + vec2(20.0, 20.0), vec2(220.0, 230.0));
    let bar = Scroll::begin(ui, e.view, 700.0, &mut e.offset);
    let top = e.view.top() - e.offset;
    e.curve_rect = egui::Rect::from_min_size(pos2(e.view.left(), top), vec2(200.0, HEIGHT));
    e.ramp_rect = egui::Rect::from_min_size(
        pos2(e.view.left(), top + HEIGHT + 10.0),
        vec2(200.0, STOPS_HEIGHT),
    );
    if let Some(next) = curve_editor(ui, e.curve_rect, "c", &e.curve, "tip", true) {
        e.curve = next;
        e.changes += 1;
    }
    let current = e.ramp.clone();
    if let Some(next) = stops_editor(
        ui,
        e.ramp_rect,
        "r",
        &current,
        &mut e.selection,
        false,
        "tip",
        true,
    ) {
        e.ramp = next;
        e.changes += 1;
    }
    let _ = ops::add_color;
    bar.end(ui, "editors", &mut e.offset);
}

fn editors_window() -> Harness<'static, Editors> {
    use yolu_app::ui::ramp::{ops, Selection};
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(280.0, 280.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(60)
        .renderer(common::shared_gpu::renderer())
        .build_ui_state(
            draw_editors,
            Editors {
                ready: false,
                offset: 0.0,
                view: egui::Rect::ZERO,
                curve: three_point_curve(),
                ramp: ops::add_color(&yolu_core::generator::Ramp::default(), 0.5, false)
                    .unwrap()
                    .0,
                selection: Selection::default(),
                curve_rect: egui::Rect::ZERO,
                ramp_rect: egui::Rect::ZERO,
                changes: 0,
            },
        );
    h.run();
    h
}

fn curve_middle(e: &Editors) -> Pos2 {
    e.curve_rect.shrink(6.0).center()
}

fn ramp_middle(e: &Editors) -> Pos2 {
    let r = e.ramp_rect;
    pos2(r.left() + 7.0 + 0.5 * (r.width() - 14.0), r.top() + 67.0)
}

fn editors_drag(h: &mut Harness<'_, Editors>, button: PointerButton, from: Pos2, to: &[Pos2]) {
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton {
        pos: from,
        button,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for p in to {
        h.event(Event::PointerMoved(*p));
        h.step();
    }
    h.event(Event::PointerButton {
        pos: *to.last().unwrap_or(&from),
        button,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.run();
}

#[test]
fn a_right_drag_from_a_curve_or_ramp_point_in_a_list_scrolls_and_only_a_right_click_removes() {
    for on_curve in [true, false] {
        let mut h = editors_window();
        let at = |h: &Harness<'_, Editors>| {
            if on_curve {
                curve_middle(h.state())
            } else {
                ramp_middle(h.state())
            }
        };
        let count = |h: &Harness<'_, Editors>| {
            if on_curve {
                h.state().curve.points().len()
            } else {
                h.state().ramp.colors().len()
            }
        };
        assert_eq!(count(&h), 3);
        // 右ドラッグ: 点は消えず、一覧が送られる
        let from = at(&h);
        editors_drag(
            &mut h,
            PointerButton::Secondary,
            from,
            &[from + vec2(0.0, -10.0), from + vec2(0.0, -50.0)],
        );
        assert_eq!(count(&h), 3, "{on_curve}: 点は消えない");
        assert_eq!(h.state().changes, 0);
        assert!(h.state().offset > 30.0, "{on_curve}: {}", h.state().offset);
        // 動かさずに右クリック: 消える
        h.state_mut().offset = 0.0;
        h.run();
        let from = at(&h);
        editors_drag(&mut h, PointerButton::Secondary, from, &[from]);
        assert_eq!(count(&h), 2, "{on_curve}: 右クリックで消える");
        assert_eq!(h.state().changes, 1);
    }
}

// ───────── 入れ子の一覧・つまみの帯 ─────────

struct Nested {
    ready: bool,
    outer: f32,
    inner: f32,
    outer_view: egui::Rect,
    inner_view: egui::Rect,
}

fn draw_nested(ui: &mut egui::Ui, n: &mut Nested) {
    use yolu_app::ui::scroll::Scroll;
    if !n.ready {
        YoluApp::setup(ui.ctx());
        n.ready = true;
        ui.ctx().request_repaint();
        return;
    }
    n.outer_view =
        egui::Rect::from_min_size(ui.max_rect().min + vec2(20.0, 20.0), vec2(220.0, 200.0));
    let outer = Scroll::begin(ui, n.outer_view, 600.0, &mut n.outer);
    n.inner_view = egui::Rect::from_min_size(
        pos2(
            n.outer_view.left() + 10.0,
            n.outer_view.top() + 30.0 - n.outer,
        ),
        vec2(170.0, 90.0),
    );
    let inner = Scroll::begin(ui, n.inner_view, 300.0, &mut n.inner);
    inner.end(ui, "inner", &mut n.inner);
    outer.end(ui, "outer", &mut n.outer);
}

fn nested_window() -> Harness<'static, Nested> {
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(280.0, 260.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(60)
        .renderer(common::shared_gpu::renderer())
        .build_ui_state(
            draw_nested,
            Nested {
                ready: false,
                outer: 0.0,
                inner: 0.0,
                outer_view: egui::Rect::ZERO,
                inner_view: egui::Rect::ZERO,
            },
        );
    h.run();
    h
}

fn nested_drag(h: &mut Harness<'_, Nested>, from: Pos2, to: &[Pos2]) {
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for p in to {
        h.event(Event::PointerMoved(*p));
        h.step();
    }
    h.event(Event::PointerButton {
        pos: *to.last().unwrap_or(&from),
        button: PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.run();
}

#[test]
fn in_nested_lists_only_the_inner_list_scrolls_even_from_its_scroll_bar() {
    // 内側の一覧の中で押し始める
    let mut h = nested_window();
    let from = h.state().inner_view.center() - vec2(20.0, 0.0);
    nested_drag(
        &mut h,
        from,
        &[from + vec2(0.0, -10.0), from + vec2(0.0, -50.0)],
    );
    assert!(near(h.state().inner, 40.0), "{}", h.state().inner);
    assert_eq!(h.state().outer, 0.0, "外側は動かない");
    // 内側のつまみの帯の上で押し始める: つまみ・溝は掴まず、内側が右ドラッグで送られる
    let mut h = nested_window();
    let view = h.state().inner_view;
    let from = pos2(view.right() - 5.0, view.bottom() - 10.0);
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    assert_eq!(h.state().inner, 0.0, "押しただけでつまみは跳ばない");
    for dy in [-10.0, -50.0] {
        h.event(Event::PointerMoved(from + vec2(0.0, dy)));
        h.step();
    }
    assert!(near(h.state().inner, 40.0), "{}", h.state().inner);
    assert_eq!(h.state().outer, 0.0, "外側でなく内側が持ち主");
}

#[test]
fn the_scroll_bar_is_grabbed_by_the_left_button_only() {
    let mut h = values_window();
    let view = h.state().view;
    let from = pos2(view.right() - 5.0, view.bottom() - 10.0);
    // 左ボタン: 溝を押すとつまみがそこへ移る（今までどおり）
    values_drag(&mut h, PointerButton::Primary, from, &[from]);
    assert!(h.state().offset > 100.0, "{}", h.state().offset);
    // 右ボタン: 押しても跳ばず、ドラッグは中身がポインタについて来る
    let mut h = values_window();
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    assert_eq!(h.state().offset, 0.0, "押しただけでは動かない");
    for dy in [-10.0, -50.0] {
        h.event(Event::PointerMoved(from + vec2(0.0, dy)));
        h.step();
    }
    assert!(near(h.state().offset, 40.0), "{}", h.state().offset);
}

#[test]
fn a_released_slider_drag_still_counts_when_the_right_button_is_pressed_and_released_in_the_middle()
{
    let mut h = values_window();
    let track = |h: &Harness<'_, Values>, dx: f32| {
        pos2(
            h.state().slider_rect.center().x + dx,
            h.state().slider_rect.bottom() - 6.0,
        )
    };
    let a = track(&h, 0.0);
    h.event(Event::PointerMoved(a));
    h.event(Event::PointerButton {
        pos: a,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    let b = track(&h, 30.0);
    h.event(Event::PointerMoved(b));
    h.step();
    let moved = h.state().slider;
    assert!(moved > 10.0);
    // 左を押したまま、右を押して離す
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: b,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.event(Event::PointerButton {
        pos: b,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.run();
    assert_eq!(h.state().released, 1, "決めたことになる");
    assert!(
        h.state().slider >= moved - 0.01,
        "値は残る {}",
        h.state().slider
    );
}

#[test]
fn a_number_field_pressed_with_the_right_button_ignores_a_left_press_elsewhere() {
    let mut h = values_window();
    let at = h.state().number_rect.center();
    let elsewhere = pos2(
        h.state().view.right() - 40.0,
        h.state().view.bottom() - 30.0,
    );
    h.event(Event::PointerMoved(at));
    h.step();
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    // 右を押したまま、別の所で左を押して動かす
    h.event(Event::PointerMoved(elsewhere));
    h.event(Event::PointerButton {
        pos: elsewhere,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::PointerMoved(elsewhere + vec2(30.0, 0.0)));
    h.step();
    assert_eq!(h.state().number, 10.0);
}

// ───────── 毎フレームの動き・やめ方・上のレイヤー ─────────

#[test]
fn after_pulling_past_the_end_the_list_follows_the_pointer_back_at_once() {
    let mut h = layers_window(0.0);
    let max = max_scroll(&mut h);
    let start = on_list(&h);
    right_press(&mut h, start);
    right_move(&mut h, vec2(0.0, -10.0).to_pos(start));
    right_move(&mut h, vec2(0.0, -300.0).to_pos(start));
    assert!(near(h.ui().layer_scroll, max));
    // 行き過ぎた分は覚えない: 戻した分だけ、すぐ付いて来る
    right_move(&mut h, vec2(0.0, -280.0).to_pos(start));
    assert!(
        near(h.ui().layer_scroll, max - 20.0),
        "{}",
        h.ui().layer_scroll
    );
    right_release(&mut h, start);
}

#[test]
fn the_wheel_alone_scrolls_the_layers_list_and_not_from_the_toolbar_above_it() {
    let mut h = layers_window(300.0);
    let in_list = on_list(&h);
    // 操作の帯の上では効かない
    move_to(&h, in_list - vec2(0.0, 30.0));
    h.step();
    let wheel = |h: &mut Harness<'_, YoluApp>| {
        h.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -100.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        for _ in 0..12 {
            h.step();
        }
    };
    wheel(&mut h);
    assert!(near(h.ui().layer_scroll, 300.0), "{}", h.ui().layer_scroll);
    move_to(&h, in_list);
    h.step();
    wheel(&mut h);
    assert!(h.ui().layer_scroll > 360.0, "{}", h.ui().layer_scroll);
}

#[test]
fn the_wheel_still_works_during_a_right_drag() {
    let mut h = layers_window(300.0);
    let start = on_list(&h);
    right_press(&mut h, start);
    right_move(&mut h, vec2(0.0, 10.0).to_pos(start));
    right_move(&mut h, vec2(0.0, 30.0).to_pos(start));
    // ホイールはポインタが一覧の上にあるときだけ効く（一覧は操作の帯の下から始まるので、下へ動かして一覧の中に置く）
    let before = h.ui().layer_scroll;
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -100.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    for _ in 0..12 {
        h.step();
    }
    assert!(
        h.ui().layer_scroll > before + 60.0,
        "{before} -> {}",
        h.ui().layer_scroll
    );
    right_release(&mut h, vec2(0.0, 30.0).to_pos(start));
}

#[test]
fn escape_and_losing_focus_end_the_right_drag() {
    for focus in [false, true] {
        let mut h = layers_window(300.0);
        let start = on_list(&h);
        right_press(&mut h, start);
        right_move(&mut h, vec2(0.0, -10.0).to_pos(start));
        right_move(&mut h, vec2(0.0, -40.0).to_pos(start));
        let moved = h.ui().layer_scroll;
        assert!(near(moved, 330.0), "{moved}");
        if focus {
            h.event(Event::WindowFocused(false));
        } else {
            key(&h, egui::Key::Escape, Modifiers::NONE);
        }
        h.step();
        right_move(&mut h, vec2(0.0, -90.0).to_pos(start));
        assert!(
            near(h.ui().layer_scroll, moved),
            "{focus}: やめたあとは動かない {}",
            h.ui().layer_scroll
        );
        right_release(&mut h, start);
        // 次の右ドラッグは普通に始まる
        right_press(&mut h, start);
        right_move(&mut h, vec2(0.0, -10.0).to_pos(start));
        right_move(&mut h, vec2(0.0, -30.0).to_pos(start));
        assert!(
            near(h.ui().layer_scroll, moved + 20.0),
            "{}",
            h.ui().layer_scroll
        );
        right_release(&mut h, start);
    }
}

#[test]
fn a_right_drag_that_starts_under_a_popup_does_not_scroll_the_list_below() {
    let mut h = layers_window(300.0);
    let start = on_list(&h);
    // 右クリックのメニューを、一覧の上に開く
    right_press(&mut h, start);
    right_release(&mut h, start);
    let body = h.popup().as_ref().expect("メニューが開く").state.rect;
    assert!(body.contains(start + vec2(8.0, 8.0)));
    let inside = start + vec2(8.0, 8.0);
    right_press(&mut h, inside);
    right_move(&mut h, vec2(0.0, -10.0).to_pos(inside));
    right_move(&mut h, vec2(0.0, -60.0).to_pos(inside));
    assert!(near(h.ui().layer_scroll, 300.0), "{}", h.ui().layer_scroll);
}
