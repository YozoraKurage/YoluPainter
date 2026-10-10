//! 2D のキャンバスのクローンの元: クローンのブラシで Alt + 左を動かさずに離すと、そこが元になる（動かせば表示を回すだけ）。次のストロークは、始めの点で
//! `offset = 元 − 始めの点` を決めて元から写す（揃えるなら以後は同じ offset、揃えないならストロークごとに決め直す）。「ずれ」の欄はそれを表示する欄で、
//! 元から決めた offset はブラシの「変えた」に数えない。「揃える」「全レイヤーから」は 3D ビューと同じ設定。実際の入力（マウスのイベント）を流して確かめる。
use crate::common;

use common::*;
use egui::{accesskit::Role, pos2, Event, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::brushes::{BrushAction, BrushKey, Category};
use yolu_app::engine::{composite_pixel, BrushEffect, DVec2, Rgba8};
use yolu_app::state::{Action, Tool};
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

const SIZE: u32 = 256;
const RED: (u32, u32) = (50, 50);
const GREEN: (u32, u32) = (120, 50);

/// 赤い四角（左下の角が (50, 50)）と緑の四角（(120, 50)）を置いた文書で、クローンのブラシ（半径 3）。
fn clone_app() -> H {
    let mut h = app(1280.0, 800.0, SIZE);
    {
        let s = &mut h.state_mut().state;
        s.tool = Tool::Brush;
        s.m2.random_seed = false;
        s.brush.radius = 3.0;
        s.brush.hardness = 1.0;
        s.m2.brush.effect = BrushEffect::Clone {
            offset: DVec2::new(64.0, 0.0),
        };
        let layer = s.selected_layer.expect("レイヤー");
        for ((x0, y0), color) in [
            (RED, Rgba8::new(255, 0, 0, 255)),
            (GREEN, Rgba8::new(0, 255, 0, 255)),
        ] {
            for y in y0..y0 + 20 {
                for x in x0..x0 + 20 {
                    s.doc.set_pixel(layer, x, y, color).unwrap();
                }
            }
        }
        s.doc.clear_history().unwrap();
    }
    h.run();
    h
}

/// 文書の点（y は上が正）の画面の位置。
fn at(h: &H, x: f64, y: f64) -> Pos2 {
    let s = &h.state().state;
    s.view
        .view(canvas_rect(h), s.doc.width(), s.doc.height())
        .to_screen(x, y)
}

fn pixel(h: &H, x: u32, y: u32) -> [u8; 4] {
    composite_pixel(&h.state().state.doc, x, y)
}

fn is_red(p: [u8; 4]) -> bool {
    p[0] > 200 && p[1] < 40 && p[2] < 40 && p[3] > 200
}

fn is_green(p: [u8; 4]) -> bool {
    p[1] > 200 && p[0] < 40 && p[2] < 40 && p[3] > 200
}

fn is_clear(p: [u8; 4]) -> bool {
    p[3] == 0
}

fn press_with(h: &H, at: Pos2, modifiers: Modifiers) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers,
    });
}

fn release_with(h: &H, at: Pos2, modifiers: Modifiers) {
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers,
    });
}

fn hold_alt(h: &mut H) {
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.step();
}

fn release_alt(h: &mut H) {
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

/// Alt を押して、`from` から `via` の順に動かして離す（`via` が空なら動かさずに離す）。
fn alt_press(h: &mut H, from: Pos2, via: &[Pos2]) {
    hold_alt(h);
    press_with(h, from, Modifiers::ALT);
    h.step();
    for p in via {
        move_to(h, *p);
        h.step();
    }
    release_with(h, via.last().copied().unwrap_or(from), Modifiers::ALT);
    h.step();
    release_alt(h);
}

/// 文書の点を、Alt を押して動かさずにクリックする。
fn alt_click(h: &mut H, x: f64, y: f64) {
    let p = at(h, x, y);
    alt_press(h, p, &[]);
}

/// 文書の点 `from` から `to` へ、普通にドラッグする。
fn stroke(h: &mut H, from: (f64, f64), to: (f64, f64)) {
    let (a, b) = (at(h, from.0, from.1), at(h, to.0, to.1));
    let path: Vec<Pos2> = (0..=6).map(|i| a + (b - a) * (i as f32 / 6.0)).collect();
    drag(h, &path);
}

fn clone_offset(h: &H) -> DVec2 {
    match h.state().state.m2.brush.effect {
        BrushEffect::Clone { offset } => offset,
        other => panic!("クローンのブラシではない: {other:?}"),
    }
}

fn source(h: &H) -> Option<(f64, f64)> {
    let s = &h.state().state;
    s.clone.canvas_source_for(s.doc.id())
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.6
}

#[test]
fn an_alt_click_sets_the_source_and_the_next_stroke_copies_from_it() {
    let mut h = clone_app();
    assert_eq!(source(&h), None);
    // Alt + 左を動かさずに離す: そこが元になる。ストロークも表示の回転も始めない
    alt_click(&mut h, 60.0, 60.0);
    let (x, y) = source(&h).expect("元を決めた");
    assert!(near(x, 60.0) && near(y, 60.0), "{x} {y}");
    assert_eq!(h.state().state.view.angle, 0.0);
    assert!(!h.state().state.doc.can_undo(), "文書は変わらない");
    assert!(h.state().state.message.contains("クローンの元を決めました"));
    // 次のストロークは、始めの点が元に重なる
    stroke(&mut h, (60.0, 160.0), (70.0, 160.0));
    let offset = clone_offset(&h);
    assert!(near(offset.x, 0.0) && near(offset.y, -100.0), "{offset:?}");
    // ブラシの半径（3）の内側は、元の赤い四角（50〜69）の内側から写す
    for x in 58..=66 {
        assert!(
            is_red(pixel(&h, x, 160)),
            "x = {x}: {:?}",
            pixel(&h, x, 160)
        );
    }
    assert!(is_red(pixel(&h, 60, 60)), "元はそのまま");
    assert!(is_clear(pixel(&h, 90, 160)));
    // 1 本のストロークは 1 回の Undo
    assert_eq!(h.state().state.doc.undo_count(), 1);
}

#[test]
fn without_a_source_the_stroke_copies_with_the_offset_of_the_brush_as_before() {
    let mut h = clone_app();
    // 元を決めていない: ブラシの offset で写す。(60, 160) の (70, −100) 先は緑の四角の中
    let own = DVec2::new(70.0, -100.0);
    h.state_mut().state.m2.brush.effect = BrushEffect::Clone { offset: own };
    stroke(&mut h, (60.0, 160.0), (66.0, 160.0));
    for x in 58..=64 {
        assert!(
            is_green(pixel(&h, x, 160)),
            "x = {x}: {:?}",
            pixel(&h, x, 160)
        );
    }
    assert_eq!(clone_offset(&h), own, "ブラシの offset は変わらない");
    assert_eq!(source(&h), None);
}

#[test]
fn aligned_keeps_the_offset_of_the_first_stroke_and_unaligned_starts_every_stroke_on_the_source() {
    for aligned in [true, false] {
        let mut h = clone_app();
        h.state_mut().state.clone.set_aligned(aligned);
        alt_click(&mut h, 60.0, 60.0);
        stroke(&mut h, (60.0, 160.0), (70.0, 160.0));
        let first = clone_offset(&h);
        assert!(near(first.x, 0.0) && near(first.y, -100.0), "{first:?}");
        // 2 本目は (130, 160) から。揃える: 同じ offset なので、緑の四角（(130, 60) 付近）を写す。揃えない: 始めが元（赤）に重なる
        stroke(&mut h, (130.0, 160.0), (138.0, 160.0));
        let second = clone_offset(&h);
        if aligned {
            assert!(
                near(second.x, 0.0) && near(second.y, -100.0),
                "揃える: {second:?}"
            );
            assert!(is_green(pixel(&h, 132, 160)), "{:?}", pixel(&h, 132, 160));
        } else {
            assert!(
                near(second.x, -70.0) && near(second.y, -100.0),
                "揃えない: {second:?}"
            );
            assert!(is_red(pixel(&h, 132, 160)), "{:?}", pixel(&h, 132, 160));
        }
        assert_eq!(h.state().state.doc.undo_count(), 2, "aligned = {aligned}");
    }
}

#[test]
fn an_alt_drag_rotates_the_view_and_leaves_the_source_alone_and_a_small_jitter_is_still_a_click() {
    let mut h = clone_app();
    alt_click(&mut h, 60.0, 60.0);
    let before = source(&h).expect("元");
    // 動かして離す: 表示を回すだけ。元は変わらない
    let c = canvas_rect(&h).center();
    alt_press(
        &mut h,
        c + egui::vec2(150.0, 0.0),
        &[c + egui::vec2(140.0, 55.0), c + egui::vec2(115.0, 96.0)],
    );
    assert_ne!(h.state().state.view.angle, 0.0, "表示を回した");
    assert_eq!(source(&h), Some(before), "元は変わらない");
    assert!(h.state().state.canvas.clone_press.is_none());
    // 少しの揺れ（遊びの内）で離しても、表示は回らず、そこが元になる
    h.state_mut().state.view.set_angle(0.0);
    h.run();
    let p = at(&h, 90.0, 90.0);
    alt_press(&mut h, p, &[p + egui::vec2(2.0, 1.0)]);
    assert_eq!(h.state().state.view.angle, 0.0, "揺れでは回らない");
    let (x, y) = source(&h).expect("元");
    assert!(
        near(x, 90.0) && near(y, 90.0),
        "押した点が元になる: {x} {y}"
    );
    // 遊びを超えて動かしてから戻って離しても、クリックではない
    let q = at(&h, 30.0, 30.0);
    alt_press(&mut h, q, &[q + egui::vec2(20.0, 0.0), q]);
    let (x, y) = source(&h).expect("元");
    assert!(near(x, 90.0) && near(y, 90.0), "元は変わらない: {x} {y}");
}

#[test]
fn the_source_is_only_set_with_the_clone_brush_and_never_outside_the_canvas() {
    let mut h = clone_app();
    // クローンのブラシでなければ、Alt + 左は表示を回すだけで、元を決めない
    h.state_mut().state.m2.brush.effect = BrushEffect::Paint;
    alt_click(&mut h, 60.0, 60.0);
    assert_eq!(source(&h), None);
    // 消しゴムでも決めない
    h.state_mut().state.m2.brush.effect = BrushEffect::Clone {
        offset: DVec2::new(64.0, 0.0),
    };
    h.state_mut().state.tool = Tool::Eraser;
    alt_click(&mut h, 60.0, 60.0);
    assert_eq!(source(&h), None);
    h.state_mut().state.tool = Tool::Brush;
    // キャンバスの外は断る。表示域の中の、文書の外の点が要るので、少し縮めて余白を作る（表示域の形で、文書が余白なしに収まることがある）
    h.state_mut().state.view.zoom = 0.9;
    h.run();
    let outside = (-2.0, 60.0);
    assert!(
        canvas_rect(&h).contains(at(&h, outside.0, outside.1)),
        "表示域の中の、文書の外"
    );
    h.state_mut().state.message.clear();
    alt_click(&mut h, outside.0, outside.1);
    assert_eq!(source(&h), None);
    assert_eq!(h.state().state.message, "キャンバスの外です。");
    // 英語でも
    h.state_mut()
        .state
        .apply(Action::M2Ui(yolu_app::m2::UiOp::Language(
            yolu_app::lang::Lang::En,
        )));
    h.state_mut().state.message.clear();
    alt_click(&mut h, outside.0, outside.1);
    assert_eq!(h.state().state.message, "Outside the canvas.");
}

#[test]
fn the_pen_clicks_the_source_the_same_way_and_a_set_switch_forgets_it() {
    let mut h = clone_app();
    // ペン（Windows Ink の点）の Alt + 触れて動かさずに離す
    hold_alt(&mut h);
    let p = at(&h, 70.0, 70.0);
    let sample = |contact: bool| yolu_app::pen::PenSample {
        pos: [p.x, p.y],
        pressure: if contact { 1.0 } else { 0.0 },
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 5,
        time_ms: 0,
    };
    h.state().pen().push(sample(true));
    h.step();
    h.state().pen().push(sample(false));
    h.step();
    release_alt(&mut h);
    let (x, y) = source(&h).expect("ペンでも元を決める");
    assert!(near(x, 70.0) && near(y, 70.0), "{x} {y}");
    assert!(!h.state().state.doc.can_undo());
    // テクスチャセットを替えると、前の文書の点は使わない
    h.state_mut().state.add_texture_set().unwrap();
    h.run();
    assert_eq!(source(&h), None);
}

#[test]
fn the_source_mark_follows_the_copied_point_while_drawing() {
    let mut h = clone_app();
    alt_click(&mut h, 60.0, 60.0);
    h.run();
    // 描いていないあいだは、決めた元の点
    assert!(h.state().state.canvas.clone_offset.is_none());
    // 描いている間: 押した点と最初の移動で、写している元の点へ動く
    let (a, b) = (at(&h, 60.0, 160.0), at(&h, 66.0, 160.0));
    press(&h, a, PointerButton::Primary);
    h.step();
    move_to(&h, b);
    h.step();
    let s = &h.state().state;
    let offset = s.canvas.clone_offset.expect("描いている間の offset");
    assert!(near(offset.x, 0.0) && near(offset.y, -100.0), "{offset:?}");
    let (ex, ey) = s.canvas.current_end.expect("今の終点");
    assert!(near(ex + offset.x, 66.0) && near(ey + offset.y, 60.0));
    release(&h, b, PointerButton::Primary);
    h.run();
    assert!(h.state().state.canvas.clone_offset.is_none());
}

fn select_builtin_clone(h: &mut H) {
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Select(BrushKey::Builtin(
            "clone",
        ))));
    h.run();
}

fn open_effect_detail(h: &mut H) {
    let ui = &mut h.state_mut().state.brushes.ui;
    ui.detail.open = true;
    ui.detail.category = Category::Effect;
    ui.detail.scroll = 0.0;
    h.run();
}

fn slider_value(h: &H, label: &str) -> f64 {
    h.get_by_role_and_label(Role::Slider, label)
        .accesskit_node()
        .numeric_value()
        .unwrap_or_else(|| panic!("{label} の値"))
}

fn disabled(h: &H, role: Role, label: &str) -> bool {
    h.get_by_role_and_label(role, label)
        .accesskit_node()
        .is_disabled()
}

#[test]
fn the_offset_fields_show_the_decided_offset_and_the_brush_is_not_marked_as_changed() {
    let mut h = clone_app();
    select_builtin_clone(&mut h);
    let key = h.state().state.brushes.lib.current();
    assert!(
        !h.state().state.brush_is_modified(key),
        "選んだだけでは変えていない"
    );
    open_effect_detail(&mut h);
    assert!((slider_value(&h, "ずれ X") - 64.0).abs() < 1e-6);
    // 元を決めて描いた: 欄は決まった offset を見せるが、ブラシは「変えた」にならない（一覧の印・元に戻す）
    h.state_mut().state.brushes.ui.detail.open = false;
    h.run();
    alt_click(&mut h, 60.0, 60.0);
    stroke(&mut h, (60.0, 160.0), (70.0, 160.0));
    assert!(near(clone_offset(&h).y, -100.0));
    assert!(
        !h.state().state.brush_is_modified(key),
        "元から決めた offset"
    );
    h.state_mut().state.brush_sync();
    assert!(
        h.state()
            .state
            .brushes
            .lib
            .entry(key)
            .unwrap()
            .edited
            .is_none(),
        "変えたままの設定として覚えない"
    );
    open_effect_detail(&mut h);
    assert!(slider_value(&h, "ずれ X").abs() < 0.6);
    assert!((slider_value(&h, "ずれ Y") + 100.0).abs() < 0.6);
    // 揃える: 欄は直せる。直した値は、ブラシの設定を直したので「変えた」になり、次のストロークから使う
    assert!(!disabled(&h, Role::Slider, "ずれ X"));
    let track = h.get_by_role_and_label(Role::Slider, "ずれ X").rect();
    click(
        &mut h,
        pos2(track.left() + track.width() * 0.75, track.center().y),
    );
    let edited = clone_offset(&h);
    assert!(edited.x > 500.0, "欄で直した: {edited:?}");
    assert!(
        h.state().state.brush_is_modified(key),
        "欄で直した offset は変えた"
    );
    h.state_mut().state.brushes.ui.detail.open = false;
    h.run();
    stroke(&mut h, (130.0, 160.0), (138.0, 160.0));
    assert_eq!(clone_offset(&h), edited, "直した値のまま次のストロークへ");
}

#[test]
fn the_offset_fields_are_disabled_with_a_reason_when_the_source_decides_every_stroke() {
    let mut h = clone_app();
    select_builtin_clone(&mut h);
    open_effect_detail(&mut h);
    // 元が無いあいだは、欄で決める。「揃える」は元が無いので薄い
    assert!(!disabled(&h, Role::Slider, "ずれ X"));
    assert!(disabled(&h, Role::CheckBox, "揃える"));
    h.state_mut().state.brushes.ui.detail.open = false;
    h.run();
    alt_click(&mut h, 60.0, 60.0);
    open_effect_detail(&mut h);
    assert!(
        !disabled(&h, Role::CheckBox, "揃える"),
        "元を決めたので使える"
    );
    assert!(!disabled(&h, Role::Slider, "ずれ X"));
    // 揃えない: ストロークごとに元から決めるので、欄は無効（理由はツールチップ）
    h.state_mut().state.clone.set_aligned(false);
    h.run();
    assert!(disabled(&h, Role::Slider, "ずれ X"));
    assert!(disabled(&h, Role::Slider, "ずれ Y"));
    let target = h
        .get_by_role_and_label(Role::Slider, "ずれ X")
        .rect()
        .center();
    hover_and_wait(&mut h, target);
    assert!(
        h.query_by_label("揃えないときは、ストロークごとに始めが元に重なる")
            .is_some(),
        "理由のツールチップ"
    );
    // 揃えるに戻せば、また直せる
    h.state_mut().state.clone.set_aligned(true);
    move_to(&h, egui::pos2(2.0, 2.0));
    h.run();
    assert!(!disabled(&h, Role::Slider, "ずれ X"));
}

#[test]
fn the_canvas_shows_aligned_and_all_layers_and_the_toggles_reach_the_2d_stroke() {
    let mut h = clone_app();
    select_builtin_clone(&mut h);
    open_effect_detail(&mut h);
    // 2D だけを出していても、揃える・全レイヤーからが並ぶ（前は 3D を出しているときだけ「揃える」が出た）
    let _ = h.get_by_role_and_label(Role::CheckBox, "揃える");
    let all = h
        .get_by_role_and_label(Role::CheckBox, "全レイヤーから")
        .rect();
    assert!(!h.state().state.clone.all_layers);
    click(&mut h, all.center());
    assert!(
        h.state().state.clone.all_layers,
        "画面の切り替えが共有の設定を変える"
    );
    // 2D のストロークも「全レイヤーから」を使う（合成の参照元を凍結する）
    h.state_mut().state.brushes.ui.detail.open = false;
    h.run();
    alt_click(&mut h, 60.0, 60.0);
    assert_eq!(h.state().state.doc.clone_source_bytes(), 0);
    let (a, b) = (at(&h, 60.0, 160.0), at(&h, 66.0, 160.0));
    press(&h, a, PointerButton::Primary);
    h.step();
    move_to(&h, b);
    h.step();
    assert!(
        h.state().state.doc.clone_source_bytes() > 0,
        "描いている間は、全レイヤーの合成を読む"
    );
    release(&h, b, PointerButton::Primary);
    h.run();
}

#[test]
fn a_cancelled_or_unchanged_stroke_does_not_fix_the_aligned_offset() {
    let mut h = clone_app();
    alt_click(&mut h, 60.0, 60.0);
    // Esc で取りやめた最初のストローク: 揃える offset は決まらない（3D の先の基準と同じ）。次のストロークの始めで決め直す
    let (a, b) = (at(&h, 60.0, 160.0), at(&h, 66.0, 160.0));
    press(&h, a, PointerButton::Primary);
    h.step();
    move_to(&h, b);
    h.step();
    key(&h, egui::Key::Escape, Modifiers::NONE);
    h.step();
    release(&h, b, PointerButton::Primary);
    h.run();
    assert!(!h.state().state.doc.can_undo(), "取りやめた");
    stroke(&mut h, (130.0, 160.0), (138.0, 160.0));
    let offset = clone_offset(&h);
    assert!(
        near(offset.x, -70.0) && near(offset.y, -100.0),
        "決め直した: {offset:?}"
    );
    assert!(is_red(pixel(&h, 132, 160)), "始めが元（赤）に重なる");
    // 確定して画素が変わったので、以後は同じ offset
    stroke(&mut h, (130.0, 200.0), (138.0, 200.0));
    assert_eq!(clone_offset(&h), offset);

    // 何も変わらなかった（透明を透明へ写した）ストロークの後も、決め直す
    let mut h = clone_app();
    alt_click(&mut h, 200.0, 200.0);
    stroke(&mut h, (60.0, 160.0), (66.0, 160.0));
    assert!(!h.state().state.doc.can_undo(), "画素は変わらない");
    stroke(&mut h, (130.0, 160.0), (138.0, 160.0));
    let offset = clone_offset(&h);
    assert!(
        near(offset.x, 70.0) && near(offset.y, 40.0),
        "決め直した: {offset:?}"
    );
}

#[test]
fn an_esc_during_the_alt_press_does_not_set_the_source() {
    let mut h = clone_app();
    let p = at(&h, 60.0, 60.0);
    hold_alt(&mut h);
    press_with(&h, p, Modifiers::ALT);
    h.step();
    key(&h, egui::Key::Escape, Modifiers::ALT);
    h.step();
    release_with(&h, p, Modifiers::ALT);
    h.step();
    release_alt(&mut h);
    assert_eq!(source(&h), None, "取りやめた押しは元にしない（3D と同じ）");
    assert!(h.state().state.canvas.clone_press.is_none());
    assert_eq!(h.state().state.view.angle, 0.0);
}

#[test]
fn the_decided_offset_never_marks_the_brush_and_a_set_switch_gives_the_brush_its_own_offset_back() {
    let mut h = clone_app();
    select_builtin_clone(&mut h);
    let key = h.state().state.brushes.lib.current();
    let own = clone_offset(&h);
    alt_click(&mut h, 60.0, 60.0);
    stroke(&mut h, (60.0, 160.0), (70.0, 160.0));
    assert_ne!(clone_offset(&h), own);
    assert!(!h.state().state.brush_is_modified(key));
    // 揃えるを入れ直しても、元を決め直しても、ブラシは「変えた」にならない
    h.state_mut().state.clone.set_aligned(false);
    h.state_mut().state.clone.set_aligned(true);
    h.run();
    assert!(!h.state().state.brush_is_modified(key), "揃えるの入れ直し");
    alt_click(&mut h, 130.0, 60.0);
    assert!(!h.state().state.brush_is_modified(key), "元の決め直し");
    stroke(&mut h, (60.0, 200.0), (66.0, 200.0));
    assert!(!h.state().state.brush_is_modified(key), "決め直した offset");
    // テクスチャセットを替える: 元を忘れ、ブラシの offset に戻る（元の無い 2D は、ブラシの offset で写す）
    h.state_mut().state.add_texture_set().unwrap();
    h.run();
    assert_eq!(source(&h), None);
    assert_eq!(clone_offset(&h), own, "ブラシの offset に戻る");
    assert!(!h.state().state.brush_is_modified(key));
}

#[test]
fn a_focus_loss_forgets_the_touch_force_on_the_canvas() {
    let mut h = clone_app();
    let p = at(&h, 60.0, 160.0);
    h.event(Event::Touch {
        device_id: egui::TouchDeviceId(0),
        id: egui::TouchId(1),
        phase: egui::TouchPhase::Start,
        pos: p,
        force: Some(0.3),
    });
    h.step();
    assert!(h.state().state.canvas.touch_pressure.is_some());
    // 終わりを受け取れないまま、フォーカスを失った: 次のマウスの押しは 1 に戻す
    h.event(Event::WindowFocused(false));
    h.step();
    assert_eq!(h.state().state.canvas.touch_pressure, None);
}

/// 組み込みのクローンのブラシで、元を決めて 2 本描く（1 本目と 2 本目の間に `between`）。2 本目の offset と、文書の全画素。
fn two_strokes_with(between: fn(&mut H)) -> (DVec2, Vec<u8>) {
    let mut h = clone_app();
    select_builtin_clone(&mut h);
    h.state_mut().state.m2.random_seed = false;
    alt_click(&mut h, 60.0, 60.0);
    stroke(&mut h, (60.0, 160.0), (70.0, 160.0));
    between(&mut h);
    stroke(&mut h, (130.0, 160.0), (138.0, 160.0));
    let doc = &h.state().state.doc;
    let key = h.state().state.brushes.lib.current();
    assert!(
        !h.state().state.brush_is_modified(key),
        "元から決めた offset は変えたに数えない"
    );
    (clone_offset(&h), doc.composite(doc.bounds()).unwrap())
}

#[test]
fn the_aligned_offset_survives_reloading_the_brush_like_the_3d_destination() {
    let (offset, plain) = two_strokes_with(|_| {});
    assert!(near(offset.x, 0.0) && near(offset.y, -100.0), "{offset:?}");
    // 消しゴムへ替えて戻る・同じブラシを選び直す・元に戻す: どれも揃える offset は続く（2 本目も同じ所から写す）
    let eraser_round_trip = |h: &mut H| {
        h.state_mut().state.apply(Action::SelectTool(Tool::Eraser));
        h.run();
        h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
        h.run();
    };
    let reselect = |h: &mut H| select_builtin_clone(h);
    let revert = |h: &mut H| {
        h.state_mut()
            .state
            .apply(Action::Brush(BrushAction::Revert(BrushKey::Builtin(
                "clone",
            ))));
        h.run();
    };
    for (name, between) in [
        ("消しゴムとの往復", eraser_round_trip as fn(&mut H)),
        ("選び直し", reselect),
        ("元に戻す", revert),
    ] {
        let (after, pixels) = two_strokes_with(between);
        assert_eq!(after, offset, "{name}");
        assert!(pixels == plain, "{name}: 2 本目も同じ所から写す");
    }
}

#[test]
fn choosing_the_same_effect_type_again_keeps_the_brush_unmodified_and_its_own_offset() {
    let mut h = clone_app();
    select_builtin_clone(&mut h);
    let key = h.state().state.brushes.lib.current();
    let own = clone_offset(&h);
    alt_click(&mut h, 60.0, 60.0);
    stroke(&mut h, (60.0, 160.0), (70.0, 160.0));
    let decided = clone_offset(&h);
    assert_ne!(decided, own);
    // 種類のメニューで、今と同じ「クローン」を選び直す
    h.state_mut()
        .state
        .apply(Action::M2Ui(yolu_app::m2::UiOp::Brush(
            yolu_app::m2::BrushOp::Effect(yolu_app::m2::EffectKind::Clone),
        )));
    h.run();
    assert_eq!(clone_offset(&h), decided);
    assert!(
        !h.state().state.brush_is_modified(key),
        "変更ありにならない"
    );
    // 元を忘れたら、ブラシの offset に戻る
    h.state_mut().state.add_texture_set().unwrap();
    h.run();
    assert_eq!(clone_offset(&h), own);
    assert!(!h.state().state.brush_is_modified(key));
    // 種類を替えて戻すと、その種類の既定の offset（揃える offset は、決まっていれば入れ直す）
    h.state_mut().state.switch_set(0).unwrap();
    h.run();
    alt_click(&mut h, 60.0, 60.0);
    // 前のストロークの上（もう赤い所）に写しても画素は変わらず、offset は決まらないので、別の所に描く
    stroke(&mut h, (60.0, 200.0), (70.0, 200.0));
    let decided = clone_offset(&h);
    assert!(
        near(decided.x, 0.0) && near(decided.y, -140.0),
        "{decided:?}"
    );
    for kind in [
        yolu_app::m2::EffectKind::Blur,
        yolu_app::m2::EffectKind::Clone,
    ] {
        h.state_mut()
            .state
            .apply(Action::M2Ui(yolu_app::m2::UiOp::Brush(
                yolu_app::m2::BrushOp::Effect(kind),
            )));
        h.run();
    }
    assert_eq!(clone_offset(&h), decided, "揃える offset は続く");
}
