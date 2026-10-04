//! 数値の欄（`ui::numfield`）の振る舞い。欄だけを置いた窓で、本物のポインタとキーで確かめる。欄を並べる側（投影の欄・Undo の 1 段）の配線は
//! `fillfx_gui.rs` が窓の全体で確かめる。
mod common;

use egui::{vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::ui::numfield::{number_field, NumSpec};
use yolu_app::YoluApp;

struct Field {
    ready: bool,
    value: f64,
    /// 欄が `changed` を返したときの値（呼び手が受け取った順）。
    changes: Vec<f64>,
    spec: NumSpec,
    enabled: bool,
    rect: Rect,
}

fn draw(ui: &mut egui::Ui, f: &mut Field) {
    if !f.ready {
        // 書体は次のフレームから効く
        YoluApp::setup(ui.ctx());
        f.ready = true;
        ui.ctx().request_repaint();
        return;
    }
    f.rect = Rect::from_min_size(ui.max_rect().min + vec2(20.0, 20.0), vec2(120.0, 22.0));
    let out = number_field(
        ui,
        f.rect,
        "n",
        "U",
        f.value,
        &f.spec,
        Some("tip"),
        None,
        f.enabled,
    );
    if out.changed {
        f.value = out.value;
        f.changes.push(out.value);
    }
}

fn field(value: f64, spec: NumSpec) -> Harness<'static, Field> {
    let mut h = Harness::builder()
        .with_size(vec2(240.0, 80.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(60)
        .build_ui_state(
            draw,
            Field {
                ready: false,
                value,
                changes: Vec::new(),
                spec,
                enabled: true,
                rect: Rect::ZERO,
            },
        );
    h.run();
    h
}

fn press(h: &Harness<'_, Field>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
}

fn release(h: &Harness<'_, Field>, at: Pos2) {
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
}

/// 欄の中央から x 方向へ、点ごとに 1 フレームでなぞる（押したまま返る）。
fn hold(h: &mut Harness<'_, Field>, dx: &[f32]) -> Pos2 {
    let c = h.state().rect.center();
    press(h, c);
    h.step();
    let mut at = c;
    for d in dx {
        at = c + vec2(*d, 0.0);
        h.event(Event::PointerMoved(at));
        h.step();
    }
    at
}

fn lift(h: &mut Harness<'_, Field>, at: Pos2) {
    release(h, at);
    h.step();
    h.run();
}

fn key(h: &Harness<'_, Field>, key: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        h.event(Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        });
    }
}

fn editing(h: &Harness<'_, Field>) -> bool {
    h.query_by_role(egui::accesskit::Role::TextInput).is_some()
}

fn spec() -> NumSpec {
    // 1 画素あたり 0.0333、小数 2 桁、範囲 -2〜5
    NumSpec::new(-2.0, 5.0, 0.0333, 2)
}

#[test]
fn dragging_counts_from_where_the_press_began_and_rounds_to_the_digits() {
    let mut h = field(1.0, spec());
    let at = hold(&mut h, &[5.0, 10.0]);
    // 10 画素 × 0.0333 = 0.333 → 小数 2 桁へ丸める（桁の外のごみを呼び手へ渡さない）
    assert_eq!(h.state().value, 1.33);
    assert!(
        h.state()
            .changes
            .iter()
            .all(|v| (v * 100.0).fract().abs() < 1e-9),
        "{:?}",
        h.state().changes
    );
    // 戻せば、押し始めの値から数えるので元の値に戻る
    let back = h.state().rect.center();
    h.event(Event::PointerMoved(back));
    h.step();
    assert_eq!(h.state().value, 1.0);
    h.event(Event::PointerMoved(back + vec2(-40.0, 0.0)));
    h.step();
    assert_eq!(
        h.state().value,
        -0.33,
        "左へは減る（1 − 40 × 0.0333 = −0.332）"
    );
    lift(&mut h, at);
    assert!(!editing(&h), "ドラッグの離しは打つ欄を開かない");
}

#[test]
fn dragging_clamps_to_the_range_and_shift_makes_it_ten_times_finer() {
    let mut h = field(1.0, spec());
    let at = hold(&mut h, &[400.0]);
    assert_eq!(h.state().value, 5.0, "上の端で止まる");
    lift(&mut h, at);
    let mut h = field(1.0, spec());
    let c = h.state().rect.center();
    press(&h, c);
    h.step();
    // Shift を押したまま動かす（1 画素あたり 0.00333、桁は 1 つ増える）
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.event(Event::PointerMoved(c + vec2(30.0, 0.0)));
    h.step();
    assert_eq!(h.state().value, 1.1, "{:?}", h.state().changes);
}

#[test]
fn escape_during_a_drag_puts_the_value_back_and_the_rest_of_the_drag_is_ignored() {
    let mut h = field(1.0, spec());
    let at = hold(&mut h, &[10.0, 40.0]);
    assert!(h.state().value > 2.0);
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    assert_eq!(h.state().value, 1.0, "押し始めの値へ戻る");
    // 押したまま動かしても、離すまで受けない
    let c = h.state().rect.center();
    h.event(Event::PointerMoved(c + vec2(80.0, 0.0)));
    h.step();
    assert_eq!(h.state().value, 1.0);
    lift(&mut h, at + vec2(40.0, 0.0));
    assert_eq!(h.state().value, 1.0);
    assert!(!editing(&h), "Esc で止めた離しは打つ欄を開かない");
    // 次の押しからは、また動かせる
    let at = hold(&mut h, &[10.0]);
    assert!(h.state().value > 1.0);
    lift(&mut h, at);
}

#[test]
fn a_small_drag_changes_the_value_and_does_not_open_the_field_but_a_plain_click_does() {
    let mut h = field(1.0, spec());
    // 打つ欄にならない距離（egui のクリックの許す動きの内）でも、動かしたなら値を変えて打つ欄は開かない
    let at = hold(&mut h, &[4.0]);
    assert!(h.state().value > 1.0, "{}", h.state().value);
    lift(&mut h, at);
    assert!(!editing(&h), "動かしたなら打つ欄を開かない");
    // 動かさないクリックは打つ欄
    let c = h.state().rect.center();
    press(&h, c);
    h.step();
    lift(&mut h, c);
    assert!(editing(&h), "クリックで打つ欄が開く");
}

#[test]
fn typing_a_value_and_enter_sets_it_clamped_and_escape_or_nonsense_throws_it_away() {
    let mut h = field(1.0, spec());
    let open = |h: &mut Harness<'_, Field>| {
        let c = h.state().rect.center();
        press(h, c);
        h.step();
        lift(h, c);
        assert!(editing(h));
    };
    let type_over = |h: &mut Harness<'_, Field>, text: &str| {
        key(h, Key::A, Modifiers::COMMAND);
        h.step();
        h.event(Event::Text(text.to_owned()));
        h.step();
    };
    // 範囲の中の値
    open(&mut h);
    type_over(&mut h, "2.5");
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().value, 2.5);
    assert!(!editing(&h), "Enter で閉じる");
    // 範囲の外は端へ
    open(&mut h);
    type_over(&mut h, "99");
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().value, 5.0, "上の端");
    open(&mut h);
    type_over(&mut h, "-99");
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().value, -2.0, "下の端");
    // 数でない文字は捨てる（値は変わらない）
    let changes = h.state().changes.len();
    open(&mut h);
    type_over(&mut h, "abc");
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().value, -2.0);
    assert!(!editing(&h));
    // Esc で捨てる
    open(&mut h);
    type_over(&mut h, "3");
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().value, -2.0, "Esc は打った値を使わない");
    assert!(!editing(&h));
    assert_eq!(
        h.state().changes.len(),
        changes,
        "捨てたものは呼び手へ渡さない"
    );
}

#[test]
fn a_disabled_field_neither_drags_nor_opens() {
    let mut h = field(1.0, spec());
    h.state_mut().enabled = false;
    h.run();
    let at = hold(&mut h, &[30.0]);
    lift(&mut h, at);
    assert_eq!(h.state().value, 1.0);
    let c = h.state().rect.center();
    press(&h, c);
    h.step();
    lift(&mut h, c);
    assert!(!editing(&h));
    assert!(h.state().changes.is_empty());
    let _ = h.get_by_label("U").rect();
}
