//! 値のカーブの編集の部品（`ui::curve::curve_editor`。ランプの値のカーブ・トーンカーブ・筆圧のカーブが共通で使う）: 点の無い所を押すと足して動かせ、
//! 点をドラッグで動かし、右クリックか枠の外へ離すと消す。ドラッグは離すまで値を返さず（1 回の変更）、Esc で元へ戻る。
mod common;
use egui::{pos2, vec2, Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::ui::curve::{curve_editor, HEIGHT};
use yolu_core::curve::{Curve, CurvePoint};

struct State {
    curve: Curve,
    enabled: bool,
    /// 部品が新しいカーブを返した回数（1 回の操作で 1 回だけのはず）。
    changes: usize,
}

const RECT: Rect = Rect {
    min: pos2(10.0, 10.0),
    max: pos2(210.0, 10.0 + HEIGHT),
};

fn harness(curve: Curve) -> Harness<'static, State> {
    common::gpu_thread::builder()
        .with_size(vec2(240.0, 160.0))
        .build_ui_state(
            |ui, state: &mut State| {
                if let Some(next) = curve_editor(ui, RECT, "c", &state.curve, "tip", state.enabled)
                {
                    state.curve = next;
                    state.changes += 1;
                }
            },
            State {
                curve,
                enabled: true,
                changes: 0,
            },
        )
}

/// 目盛り（0〜1）から画面の位置（描く範囲は枠より 6 小さい）。
fn at(x: f64, y: f64) -> Pos2 {
    let g = RECT.shrink(6.0);
    pos2(
        g.left() + x as f32 * g.width(),
        g.bottom() - y as f32 * g.height(),
    )
}

fn button(h: &Harness<'_, State>, pos: Pos2, button: PointerButton, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: Modifiers::NONE,
    });
}

fn step(h: &mut Harness<'_, State>) {
    h.run_steps(2);
}

fn press(h: &mut Harness<'_, State>, pos: Pos2) {
    h.hover_at(pos);
    step(h);
    button(h, pos, PointerButton::Primary, true);
    step(h);
}

fn move_to(h: &mut Harness<'_, State>, pos: Pos2) {
    h.hover_at(pos);
    step(h);
}

fn release(h: &mut Harness<'_, State>, pos: Pos2) {
    button(h, pos, PointerButton::Primary, false);
    step(h);
}

fn three_points() -> Curve {
    Curve::new(vec![
        CurvePoint { x: 0., y: 0. },
        CurvePoint { x: 0.5, y: 0.5 },
        CurvePoint { x: 1., y: 1. },
    ])
    .unwrap()
}

#[test]
fn pressing_an_empty_spot_adds_a_point_that_follows_the_drag_and_is_returned_once_on_release() {
    let mut h = harness(Curve::identity());
    press(&mut h, at(0.4, 0.8));
    assert_eq!(h.state().changes, 0, "押しただけでは返さない");
    move_to(&mut h, at(0.4, 0.9));
    move_to(&mut h, at(0.45, 0.3));
    assert_eq!(h.state().changes, 0, "ドラッグの間は下書き");
    assert_eq!(h.state().curve, Curve::identity(), "元のまま");
    release(&mut h, at(0.45, 0.3));
    let s = h.state();
    assert_eq!(s.changes, 1, "離したとき 1 回だけ");
    assert_eq!(s.curve.points().len(), 3);
    let p = s.curve.points()[1];
    assert!(
        (p.x - 0.45).abs() < 0.02 && (p.y - 0.3).abs() < 0.02,
        "{p:?}"
    );
}

#[test]
fn dragging_a_point_moves_it_and_the_ends_only_move_up_and_down() {
    let mut h = harness(three_points());
    press(&mut h, at(0.5, 0.5));
    move_to(&mut h, at(0.3, 0.7));
    release(&mut h, at(0.3, 0.7));
    assert_eq!(h.state().changes, 1);
    let p = h.state().curve.points()[1];
    assert!(
        (p.x - 0.3).abs() < 0.02 && (p.y - 0.7).abs() < 0.02,
        "{p:?}"
    );
    // 左の端を右上へ引いても x は 0 のまま
    press(&mut h, at(0.0, 0.0));
    move_to(&mut h, at(0.2, 0.4));
    release(&mut h, at(0.2, 0.4));
    assert_eq!(h.state().changes, 2);
    let q = h.state().curve.points()[0];
    assert_eq!(q.x, 0.0);
    assert!((q.y - 0.4).abs() < 0.02, "{q:?}");
}

#[test]
fn escape_during_a_drag_puts_it_back_and_returns_nothing() {
    let mut h = harness(three_points());
    let before = h.state().curve.clone();
    press(&mut h, at(0.5, 0.5));
    move_to(&mut h, at(0.6, 0.9));
    h.key_press(egui::Key::Escape);
    step(&mut h);
    move_to(&mut h, at(0.7, 0.9));
    release(&mut h, at(0.7, 0.9));
    assert_eq!(h.state().changes, 0);
    assert_eq!(h.state().curve, before);
}

#[test]
fn releasing_outside_the_frame_removes_the_point_but_not_an_end() {
    let mut h = harness(three_points());
    press(&mut h, at(0.5, 0.5));
    move_to(&mut h, pos2(RECT.center().x, RECT.bottom() + 60.0));
    release(&mut h, pos2(RECT.center().x, RECT.bottom() + 60.0));
    assert_eq!(h.state().changes, 1);
    assert_eq!(h.state().curve.points().len(), 2);
    // 端は枠の外へ出しても消えない
    let mut h = harness(three_points());
    press(&mut h, at(1.0, 1.0));
    move_to(&mut h, pos2(RECT.center().x, RECT.bottom() + 60.0));
    release(&mut h, pos2(RECT.center().x, RECT.bottom() + 60.0));
    assert_eq!(h.state().curve.points().len(), 3);
}

#[test]
fn right_click_removes_a_middle_point_and_leaves_the_ends_and_two_points() {
    let mut h = harness(three_points());
    h.hover_at(at(0.5, 0.5));
    step(&mut h);
    button(&h, at(0.5, 0.5), PointerButton::Secondary, true);
    step(&mut h);
    button(&h, at(0.5, 0.5), PointerButton::Secondary, false);
    step(&mut h);
    assert_eq!(h.state().changes, 1);
    assert_eq!(h.state().curve, Curve::identity());
    // 端は消せない
    h.hover_at(at(1.0, 1.0));
    step(&mut h);
    button(&h, at(1.0, 1.0), PointerButton::Secondary, true);
    step(&mut h);
    assert_eq!(h.state().changes, 1);
}

#[test]
fn a_disabled_editor_ignores_the_pointer() {
    let mut h = harness(three_points());
    h.state_mut().enabled = false;
    step(&mut h);
    press(&mut h, at(0.5, 0.5));
    move_to(&mut h, at(0.3, 0.9));
    release(&mut h, at(0.3, 0.9));
    assert_eq!(h.state().changes, 0);
    assert_eq!(h.state().curve, three_points());
}

#[test]
fn the_frame_the_drag_is_released_still_shows_the_moved_point_not_the_old_one() {
    // 呼び手は返された値を、部品が描いたあとに当てる（実際の画面と同じ）。離したフレームに古い値を描くと、点が元の位置へ一瞬戻る。
    use std::cell::RefCell;
    thread_local! {
        static PAINTED: RefCell<Vec<Curve>> = const { RefCell::new(Vec::new()) };
    }
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(240.0, 160.0))
        .build_ui_state(
            |ui, state: &mut State| {
                if let Some(next) = curve_editor(ui, RECT, "c", &state.curve, "tip", state.enabled)
                {
                    state.curve = next;
                    state.changes += 1;
                }
                if let Some(c) = yolu_app::ui::curve::last_painted(ui, "c") {
                    PAINTED.with(|p| p.borrow_mut().push(c));
                }
            },
            State {
                curve: three_points(),
                enabled: true,
                changes: 0,
            },
        );
    press(&mut h, at(0.5, 0.5));
    move_to(&mut h, at(0.3, 0.7));
    PAINTED.with(|p| p.borrow_mut().clear());
    button(&h, at(0.3, 0.7), PointerButton::Primary, false);
    h.step();
    let moved = h.state().curve.points()[1];
    assert!((moved.x - 0.3).abs() < 0.02, "{moved:?}");
    PAINTED.with(|p| {
        let painted = p.borrow();
        let first = painted.first().expect("離したフレームを描いた");
        assert!(
            (first.points()[1].x - 0.3).abs() < 0.02,
            "離したフレームに元の位置を描いた: {:?}",
            first.points()[1]
        );
        assert!(
            painted
                .iter()
                .all(|c| (c.points()[1].x - 0.3).abs() < 0.02),
            "離したあとのどのフレームも動かした位置"
        );
    });
}
