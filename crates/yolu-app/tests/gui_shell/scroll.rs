//! 縦のスクロールの共通の部品（`ui::scroll`）: ホイール・つまみを掴んで動かす・溝を押して移る・ペン（winit が egui のポインタと Touch に
//! 変えた入力）で同じことができる、つまみの太さと掴める幅がカラーセットの欄（egui の `ScrollArea`・`ScrollStyle::thin`）と同じ、
//! 中身が収まるときはつまみを出さない。部品だけを置いた窓と、本物の窓（レイヤーの欄・一覧の窓）で確かめる。
use crate::common;

use egui::epaint::Shape;
use egui::{pos2, vec2, Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, Rect, TouchPhase};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::ui::scroll::{Scroll, BAR_WIDTH, HANDLE_MIN, THIN_WIDTH};
use yolu_app::YoluApp;

/// つまみだけを置いた窓。
struct Pane {
    ready: bool,
    offset: f32,
    content: f32,
    view: Rect,
    dragging: bool,
}

fn draw(ui: &mut egui::Ui, p: &mut Pane) {
    if !p.ready {
        // 書体は次のフレームから効く
        YoluApp::setup(ui.ctx());
        p.ready = true;
        ui.ctx().request_repaint();
        return;
    }
    p.view = Rect::from_min_size(ui.max_rect().min + vec2(20.0, 20.0), vec2(200.0, 120.0));
    // パネルの地（絵で、アプリの中と同じ背景に見えるように）
    ui.painter().rect_filled(ui.max_rect(), 0.0, yolu_app::ui::theme::PANEL_BG);
    ui.painter().rect_filled(p.view, 0.0, yolu_app::ui::theme::CONTROL_BG);
    let bar = Scroll::begin(ui, p.view, p.content, &mut p.offset);
    p.dragging = bar.end(ui, "pane", &mut p.offset);
}

fn pane(content: f32) -> Harness<'static, Pane> {
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(260.0, 180.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(60)
        .renderer(common::shared_gpu::renderer())
        .build_ui_state(
            draw,
            Pane {
                ready: false,
                offset: 0.0,
                content,
                view: Rect::ZERO,
                dragging: false,
            },
        );
    h.run();
    h
}

/// 中身 360・見える所 120 のとき: ずらせる量 240、つまみの長さ 40、つまみが動ける幅 80（1 画素で 3 動く）。
const CONTENT: f32 = 360.0;

fn handle(h: &Harness<'_, Pane>) -> Rect {
    let p = h.state();
    Scroll::new(p.view, p.content, &mut p.offset.clone()).handle_rect(p.offset)
}

/// つまみの上の点（x は掴める帯の真ん中）。
fn on_bar(h: &Harness<'_, Pane>, dy: f32) -> Pos2 {
    let view = h.state().view;
    pos2(view.right() - BAR_WIDTH / 2.0, view.top() + dy)
}

fn rects(shape: &Shape, out: &mut Vec<Rect>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|s| rects(s, out)),
        Shape::Rect(r) => out.push(r.rect),
        _ => {}
    }
}

/// 描いたつまみの矩形（右の縁に付いていて、つまみの長さの矩形のうち一番細いもの。溝は縦いっぱいの高さ）。
fn drawn_handle(h: &Harness<'_, Pane>) -> Rect {
    let view = h.state().view;
    let length = handle(h).height();
    let mut all = Vec::new();
    for s in &h.output().shapes {
        rects(&s.shape, &mut all);
    }
    all.into_iter()
        .filter(|r| (r.right() - view.right()).abs() < 0.01 && (r.height() - length).abs() < 0.5)
        .min_by(|a, b| a.width().total_cmp(&b.width()))
        .unwrap_or_else(|| panic!("つまみが描かれていない"))
}

fn press(h: &Harness<'_, Pane>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
}

fn release(h: &Harness<'_, Pane>, at: Pos2) {
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
}

/// 押して、点ごとに 1 フレームで動かし、離す（マウス）。
fn mouse_drag(h: &mut Harness<'_, Pane>, from: Pos2, to: &[Pos2]) {
    press(h, from);
    h.step();
    for p in to {
        h.event(Event::PointerMoved(*p));
        h.step();
    }
    release(h, *to.last().unwrap_or(&from));
    h.step();
    h.run();
}

/// ペンの接触（winit が WM_POINTER のペンから作る入力と同じ: 触れた点で Touch の Start と、ポインタの移動・左ボタンの押し。動かす間は
/// Touch の Move とポインタの移動。離すとポインタの離しと Touch の End と、ポインタが消える）。
fn pen_drag(h: &mut Harness<'_, Pane>, from: Pos2, to: &[Pos2]) {
    let touch = |phase, pos| Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(7),
        phase,
        pos,
        force: Some(0.5),
    };
    h.event(touch(TouchPhase::Start, from));
    press(h, from);
    h.step();
    for p in to {
        h.event(touch(TouchPhase::Move, *p));
        h.event(Event::PointerMoved(*p));
        h.step();
    }
    let end = *to.last().unwrap_or(&from);
    release(h, end);
    h.event(touch(TouchPhase::End, end));
    h.event(Event::PointerGone);
    h.step();
    h.run();
}

#[test]
fn the_numbers_are_the_color_sets_egui_scroll_area() {
    // カラーセットの欄は egui の ScrollArea（`ScrollStyle::thin`）。共通の部品の寸法は、その値と同じ
    let thin = egui::style::ScrollStyle::thin();
    assert_eq!(BAR_WIDTH, thin.bar_width, "掴める幅・広がったつまみの太さ");
    assert_eq!(THIN_WIDTH, thin.floating_width, "普段のつまみの太さ");
    assert_eq!(HANDLE_MIN, thin.handle_min_length, "つまみの最小の長さ");
    // アプリの配色が選ぶスクロールの形は thin のまま（カラーセットの見た目が変わっていない）
    let ctx = egui::Context::default();
    YoluApp::setup(&ctx);
    assert_eq!(ctx.global_style().spacing.scroll.bar_width, thin.bar_width);
    assert_eq!(ctx.global_style().spacing.scroll.floating_width, thin.floating_width);
}

#[test]
fn the_wheel_scrolls_inside_the_view_and_stops_at_both_ends() {
    let mut h = pane(CONTENT);
    let view = h.state().view;
    h.event(Event::PointerMoved(view.center()));
    h.step();
    h.event(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, -50.0),
        modifiers: Modifiers::NONE,
        phase: TouchPhase::Move,
    });
    h.run();
    assert!((h.state().offset - 50.0).abs() < 0.5, "{}", h.state().offset);
    // 上へ戻しすぎても 0 より小さくならない・下へ送りすぎてもずらせる量を超えない
    h.event(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, 500.0),
        modifiers: Modifiers::NONE,
        phase: TouchPhase::Move,
    });
    h.run();
    assert_eq!(h.state().offset, 0.0);
    h.event(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, -5000.0),
        modifiers: Modifiers::NONE,
        phase: TouchPhase::Move,
    });
    h.run();
    assert_eq!(h.state().offset, CONTENT - view.height());
    // 見える所の外ではホイールを受けない
    h.state_mut().offset = 0.0;
    h.event(Event::PointerMoved(view.right_bottom() + vec2(30.0, 30.0)));
    h.step();
    h.event(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, -50.0),
        modifiers: Modifiers::NONE,
        phase: TouchPhase::Move,
    });
    h.run();
    assert_eq!(h.state().offset, 0.0);
}

#[test]
fn dragging_the_handle_with_the_mouse_moves_the_content_by_the_ratio() {
    let mut h = pane(CONTENT);
    let length = handle(&h).height();
    assert_eq!(length, 40.0);
    // つまみの真ん中から 20 画素下へ: 1 画素で 3（ずらせる量 240 ÷ つまみが動ける幅 80）
    let from = on_bar(&h, 20.0);
    mouse_drag(&mut h, from, &[from + vec2(0.0, 10.0), from + vec2(0.0, 20.0)]);
    assert!((h.state().offset - 60.0).abs() < 0.5, "{}", h.state().offset);
    // 離したあとは動かない。もう一度、つまみの上端近くを掴むと、掴んだ位置を保って動く（つまみがポインタの下へ跳ばない）
    let before = h.state().offset;
    let grab = on_bar(&h, handle(&h).top() - h.state().view.top() + 5.0);
    mouse_drag(&mut h, grab, &[grab + vec2(0.0, 8.0)]);
    assert!((h.state().offset - (before + 24.0)).abs() < 0.5, "{}", h.state().offset);
    // 範囲を越えて引いても端で止まる
    let grab = on_bar(&h, handle(&h).top() - h.state().view.top() + 5.0);
    mouse_drag(&mut h, grab, &[grab + vec2(0.0, 500.0)]);
    assert_eq!(h.state().offset, CONTENT - h.state().view.height());
    let grab = on_bar(&h, handle(&h).top() - h.state().view.top() + 5.0);
    mouse_drag(&mut h, grab, &[grab - vec2(0.0, 500.0)]);
    assert_eq!(h.state().offset, 0.0);
}

#[test]
fn pressing_the_groove_moves_the_handle_under_the_pointer_and_dragging_continues() {
    let mut h = pane(CONTENT);
    // つまみ（上から 0〜40）の下の溝を押す: つまみの真ん中が押した所へ来る（上端 40 → ずらした量 120）
    let at = on_bar(&h, 60.0);
    press(&h, at);
    h.step();
    h.step();
    assert!((h.state().offset - 120.0).abs() < 0.5, "{}", h.state().offset);
    // 押したまま動かすと、つまみがついてくる
    h.event(Event::PointerMoved(at + vec2(0.0, 10.0)));
    h.step();
    h.step();
    assert!((h.state().offset - 150.0).abs() < 0.5, "{}", h.state().offset);
    release(&h, at + vec2(0.0, 10.0));
    h.step();
    h.run();
    assert!(!h.state().dragging);
    assert!((h.state().offset - 150.0).abs() < 0.5);
    // 一番下の端の外を押しても、つまみは端まで（ずらせる量を超えない）
    let at = on_bar(&h, 119.0);
    press(&h, at);
    h.step();
    h.step();
    release(&h, at);
    h.run();
    assert_eq!(h.state().offset, CONTENT - h.state().view.height());
}

#[test]
fn a_pen_touch_grabs_the_handle_and_the_groove_like_the_mouse() {
    let mut h = pane(CONTENT);
    // つまみを掴んで動かす（マウスと同じ結果）
    let from = on_bar(&h, 20.0);
    pen_drag(&mut h, from, &[from + vec2(0.0, 10.0), from + vec2(0.0, 20.0)]);
    assert!((h.state().offset - 60.0).abs() < 0.5, "{}", h.state().offset);
    // 溝に触れる: つまみが触れた所へ移り、そのまま動かせる
    h.state_mut().offset = 0.0;
    h.run();
    let at = on_bar(&h, 60.0);
    pen_drag(&mut h, at, &[at + vec2(0.0, 10.0)]);
    assert!((h.state().offset - 150.0).abs() < 0.5, "{}", h.state().offset);
    // 触れて離しただけ（動かさない）でも、溝なら移る
    h.state_mut().offset = 0.0;
    h.run();
    let at = on_bar(&h, 100.0);
    pen_drag(&mut h, at, &[]);
    assert!(h.state().offset > 120.0, "{}", h.state().offset);
}

#[test]
fn the_handle_is_thin_at_rest_and_widens_when_the_pointer_comes_like_the_color_sets() {
    let mut h = pane(CONTENT);
    h.run();
    assert_eq!(drawn_handle(&h).width(), THIN_WIDTH);
    // 掴める帯に乗せると、広がる（つまみの太さ 10）
    h.event(Event::PointerMoved(on_bar(&h, 20.0)));
    h.run();
    assert_eq!(drawn_handle(&h).width(), BAR_WIDTH);
    // 離れると、細く戻る
    h.event(Event::PointerMoved(pos2(5.0, 5.0)));
    h.run();
    assert_eq!(drawn_handle(&h).width(), THIN_WIDTH);
    // 掴める帯は、細いあいだも 10 点の幅（細い 2 点の上でなくても、右の縁から 10 点以内なら掴める）
    let view = h.state().view;
    let edge = pos2(view.right() - BAR_WIDTH + 1.0, view.top() + 20.0);
    mouse_drag(&mut h, edge, &[edge + vec2(0.0, 10.0)]);
    assert!((h.state().offset - 30.0).abs() < 0.5, "{}", h.state().offset);
}

#[test]
fn nothing_is_grabbed_or_drawn_when_the_content_fits() {
    let mut h = pane(100.0);
    let view = h.state().view;
    let scroll = Scroll::new(view, 100.0, &mut 0.0);
    assert!(!scroll.needed());
    assert_eq!(scroll.reserved(), 0.0);
    // 押してもずれず、つまみは描かれない
    let at = on_bar(&h, 30.0);
    mouse_drag(&mut h, at, &[at + vec2(0.0, 30.0)]);
    assert_eq!(h.state().offset, 0.0);
    assert!(h.query_by_role(egui::accesskit::Role::ScrollBar).is_none());
    // 中身がちょうど収まるときも、あふれるときだけ幅を引く
    let scroll = Scroll::new(view, view.height() + 1.0, &mut 0.0);
    assert!(scroll.needed());
    assert_eq!(scroll.reserved(), BAR_WIDTH);
    // 中身が減って収まるようになったら、ずらした量は 0 に戻る（先頭より前を見せない）
    h.state_mut().content = CONTENT;
    h.state_mut().offset = 200.0;
    h.run();
    h.state_mut().content = 100.0;
    h.run();
    assert_eq!(h.state().offset, 0.0);
}

#[test]
fn the_scroll_bar_has_its_own_node_for_screen_readers() {
    let h = pane(CONTENT);
    let node = h.get_by_role(egui::accesskit::Role::ScrollBar);
    let view = h.state().view;
    assert_eq!(node.rect().width(), BAR_WIDTH);
    assert_eq!(node.rect().right(), view.right());
    assert_eq!(node.rect().height(), view.height());
}

// ───────── 本物の窓 ─────────

#[test]
fn the_layers_list_scrolls_by_dragging_its_handle_with_a_pen() {
    use yolu_app::state::Action;
    let mut h = common::app(1280.0, 800.0, 128);
    for _ in 0..40 {
        h.state_mut().apply(Action::NewLayer);
    }
    h.run();
    let bars: Vec<Rect> = h
        .get_all_by_role(egui::accesskit::Role::ScrollBar)
        .map(|n| n.rect())
        .collect();
    assert!(!bars.is_empty(), "あふれる一覧にはつまみがある");
    // 右の列（レイヤーの欄）のつまみ
    let bar = *bars
        .iter()
        .find(|r| r.left() > 900.0)
        .unwrap_or_else(|| panic!("{bars:?}"));
    assert_eq!(h.state().state.layer_scroll, 0.0);
    let from = pos2(bar.center().x, bar.top() + 6.0);
    let touch = |phase, pos| Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(1),
        phase,
        pos,
        force: Some(0.4),
    };
    h.event(touch(TouchPhase::Start, from));
    h.event(Event::PointerMoved(from));
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    let to = from + vec2(0.0, 30.0);
    h.event(touch(TouchPhase::Move, to));
    h.event(Event::PointerMoved(to));
    h.step();
    h.step();
    h.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.event(touch(TouchPhase::End, to));
    h.event(Event::PointerGone);
    h.run();
    assert!(
        h.state().state.layer_scroll > 20.0,
        "ペンでつまみを引いたぶん一覧が動く: {}",
        h.state().state.layer_scroll
    );
}

#[test]
fn a_list_window_keeps_its_scroll_between_frames_even_when_the_caller_passes_zero() {
    use yolu_app::windows::{show_list, Button, ListSpec, Row};
    struct Host {
        ready: bool,
        offset: egui::Vec2,
        rows: usize,
    }
    fn spec(rows: usize) -> ListSpec {
        ListSpec {
            id: "scroll-test",
            title: "t".into(),
            icon: "folder_open",
            modal: false,
            width: 420.0,
            summary: None,
            rows: (0..rows).map(|i| Row::text(format!("row {i}"), false)).collect(),
            buttons: vec![Button { label: "OK".into(), primary: true, tooltip: None }],
            close_label: "Close".into(),
        }
    }
    fn draw_host(ui: &mut egui::Ui, host: &mut Host) {
        if !host.ready {
            YoluApp::setup(ui.ctx());
            host.ready = true;
            ui.ctx().request_repaint();
            return;
        }
        // 呼ぶ側が毎フレーム 0 を渡す（確認・報告の窓の呼び方）
        let mut scroll = 0.0;
        let _ = show_list(&ui.ctx().clone(), &spec(host.rows), &mut host.offset, &mut scroll);
    }
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(600.0, 700.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(60)
        .build_ui_state(draw_host, Host { ready: false, offset: egui::Vec2::ZERO, rows: 30 });
    h.run();
    let bar = h.get_by_role(egui::accesskit::Role::ScrollBar).rect();
    let first = text_top(&h, "row 0").expect("先頭の行が見える");
    // つまみを引いて、窓が次のフレームも同じ位置を保つ（ずらした量が毎回 0 に戻らない）
    let from = pos2(bar.center().x, bar.top() + 5.0);
    h.event(Event::PointerMoved(from));
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for dy in [10.0, 20.0, 30.0] {
        h.event(Event::PointerMoved(from + vec2(0.0, dy)));
        h.step();
    }
    h.event(Event::PointerButton { pos: from + vec2(0.0, 30.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
    h.run();
    let moved = text_top(&h, "row 0");
    assert!(
        moved.is_none_or(|y| y < first - 20.0),
        "窓の一覧がつまみで動き、離したあとも戻らない: {first} → {moved:?}"
    );
    // ホイールでも動く（一覧の上にポインタを置いて、上へ戻す）
    let view = h.get_by_role(egui::accesskit::Role::ScrollBar).rect();
    h.event(Event::PointerMoved(view.center() - vec2(60.0, 0.0)));
    h.step();
    h.event(Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(0.0, 1000.0), modifiers: Modifiers::NONE, phase: TouchPhase::Move });
    h.run();
    h.run();
    assert_eq!(text_top(&h, "row 0"), Some(first), "ホイールで先頭まで戻る");
}

/// 描いた文字（`Shape::Text`）のうち、`text` の行の上端（画面の点）。描いていなければ None。
fn text_top<S>(h: &Harness<'_, S>, text: &str) -> Option<f32> {
    fn find(shape: &Shape, text: &str) -> Option<f32> {
        match shape {
            Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text)),
            Shape::Text(t) if t.galley.job.text == text => Some(t.pos.y),
            _ => None,
        }
    }
    h.output().shapes.iter().find_map(|s| find(&s.shape, text))
}

/// 絵: つまみの普段（細い）と、ポインタを乗せたとき・掴んでいるとき（太い）、途中までずらしたとき。
#[test]
fn the_handle_looks_right_at_rest_hovered_and_grabbed() {
    let mut h = pane(CONTENT);
    h.state_mut().offset = 90.0;
    h.run();
    let shot = |h: &mut Harness<'_, Pane>, name: &str| {
        let view = h.state().view;
        let image = h.render().expect("描画");
        let rect = view.expand(8.0);
        let cropped = image::imageops::crop_imm(
            &image,
            rect.left().floor() as u32,
            rect.top().floor() as u32,
            rect.width().ceil() as u32,
            rect.height().ceil() as u32,
        )
        .to_image();
        egui_kittest::image_snapshot(&cropped, name);
    };
    shot(&mut h, "scroll_handle_rest");
    let at = on_bar(&h, handle(&h).center().y - h.state().view.top());
    h.event(Event::PointerMoved(at));
    h.run();
    shot(&mut h, "scroll_handle_hovered");
    press(&h, at);
    h.step();
    h.step();
    shot(&mut h, "scroll_handle_grabbed");
}
