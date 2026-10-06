//! ランプの編集の部品（`ui::ramp`）: 分岐点の編集（足す・動かす・消す）、離した直後の表示、共通のパネルの操作。
use crate::common;
use std::cell::RefCell;

use egui::{pos2, vec2, Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::ui::ramp::{self, ops, Selection, STOPS_HEIGHT};
use yolu_core::generator::Ramp;

const RECT: Rect = Rect {
    min: pos2(10.0, 10.0),
    max: pos2(250.0, 10.0 + STOPS_HEIGHT),
};

struct State {
    ramp: Ramp,
    selection: Selection,
    /// 部品が新しいランプを返した回数。
    changes: usize,
    /// 文書へ当てるのを次のフレームまで遅らせる（返したあとに別の所で当たる場合）。
    late: bool,
    queued: Option<Ramp>,
    /// 色の分岐点のダブルクリックで出た、色の選びの頼み。
    requests: Vec<ramp::ColorRequest>,
}

thread_local! {
    /// 部品が描いたランプ（フレームごと）。
    static PAINTED: RefCell<Vec<Ramp>> = const { RefCell::new(Vec::new()) };
}

fn harness(ramp: Ramp, late: bool) -> Harness<'static, State> {
    PAINTED.with(|p| p.borrow_mut().clear());
    common::gpu_thread::builder()
        .with_size(vec2(260.0, 110.0))
        .with_step_dt(0.05)
        .build_ui_state(
            |ui, state: &mut State| {
                if let Some(q) = state.queued.take() {
                    state.ramp = q;
                }
                let current = state.ramp.clone();
                if let Some(next) = ramp::stops_editor(
                    ui,
                    RECT,
                    "r",
                    &current,
                    &mut state.selection,
                    false,
                    "tip",
                    true,
                ) {
                    state.changes += 1;
                    if state.late {
                        state.queued = Some(next);
                    } else {
                        state.ramp = next;
                    }
                }
                if let Some(r) = ramp::last_painted(ui, "r") {
                    PAINTED.with(|p| p.borrow_mut().push(r));
                }
                if let Some(request) = ramp::take_color_request(ui, "r") {
                    state.requests.push(request);
                }
            },
            State {
                ramp,
                selection: Selection::default(),
                changes: 0,
                late,
                queued: None,
                requests: Vec::new(),
            },
        )
}

fn three() -> Ramp {
    ops::add_color(&Ramp::default(), 0.5, false).unwrap().0
}

/// 色の分岐点の行の、位置 p（0〜1）の画面の点。
fn stop_at(p: f64) -> Pos2 {
    let left = RECT.left() + 7.0;
    let width = RECT.width() - 14.0;
    pos2(left + p as f32 * width, RECT.top() + 67.0)
}

fn button(h: &Harness<'_, State>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
}

fn press(h: &mut Harness<'_, State>, pos: Pos2) {
    h.hover_at(pos);
    h.run_steps(2);
    button(h, pos, true);
    h.run_steps(2);
}

fn painted_stop(ramp: &Ramp) -> f64 {
    ramp.colors()[1].position
}

#[test]
fn the_frame_a_stop_is_released_shows_the_moved_stop_and_never_the_old_position() {
    for late in [false, true] {
        let mut h = harness(three(), late);
        press(&mut h, stop_at(0.5));
        h.hover_at(stop_at(0.8));
        h.run_steps(2);
        PAINTED.with(|p| p.borrow_mut().clear());
        button(&h, stop_at(0.8), false);
        h.step();
        assert_eq!(h.state().changes, 1, "離したとき 1 回だけ返す");
        PAINTED.with(|p| {
            let painted = p.borrow();
            let first = painted.first().expect("離したフレームを描いた");
            assert!(
                (painted_stop(first) - 0.8).abs() < 0.02,
                "late={late}: 離したフレームが元の位置 {} を描いた",
                painted_stop(first)
            );
        });
        // 文書が追いついたあと（遅らせる場合は次のフレーム）も動かした位置のまま
        h.run_steps(3);
        PAINTED.with(|p| {
            for r in p.borrow().iter() {
                assert!(
                    (painted_stop(r) - 0.8).abs() < 0.02,
                    "late={late}: {}",
                    painted_stop(r)
                );
            }
        });
        assert!((painted_stop(&h.state().ramp) - 0.8).abs() < 0.02);
    }
}

#[test]
fn a_stop_that_the_document_refuses_goes_back_after_a_moment() {
    // 返した値を文書が受けなければ（呼び手の値が元のまま）、長く嘘の位置を見せ続けない
    let mut h = harness(three(), false);
    press(&mut h, stop_at(0.5));
    h.hover_at(stop_at(0.8));
    h.run_steps(2);
    button(&h, stop_at(0.8), false);
    h.step();
    // 呼び手が値を元へ戻す（文書が断った）
    h.state_mut().ramp = three();
    for _ in 0..40 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    PAINTED.with(|p| {
        let last = p.borrow().last().cloned().unwrap();
        assert!(
            (painted_stop(&last) - 0.5).abs() < 1e-9,
            "{}",
            painted_stop(&last)
        );
    });
}

fn click(h: &mut Harness<'_, State>, pos: Pos2) {
    h.hover_at(pos);
    h.run_steps(2);
    button(h, pos, true);
    h.step();
    button(h, pos, false);
    h.step();
}

#[test]
fn double_clicking_a_colour_stop_selects_it_and_asks_for_the_colour_picker_next_to_it() {
    let mut h = harness(three(), false);
    click(&mut h, stop_at(0.5));
    assert!(h.state().requests.is_empty(), "1 回では出さない");
    click(&mut h, stop_at(0.5));
    let request = *h
        .state()
        .requests
        .last()
        .expect("ダブルクリックで頼みが出る");
    assert_eq!(request.index, 1);
    assert_eq!(
        h.state().selection,
        Selection {
            index: 1,
            alpha: false
        }
    );
    assert!(
        (request.anchor.center().x - stop_at(0.5).x).abs() < 1.0
            && (request.anchor.center().y - stop_at(0.5).y).abs() < 1.0,
        "分岐点の印のあたり: {:?}",
        request.anchor
    );
    assert_eq!(h.state().changes, 0, "押しただけでは値は変わらない");
    // 端の分岐点も
    h.run_steps(10);
    click(&mut h, stop_at(1.0));
    click(&mut h, stop_at(1.0));
    assert_eq!(h.state().requests.last().unwrap().index, 2);
    // 頼みは 1 回だけ取り出せる
    h.run_steps(2);
    assert_eq!(h.state().requests.len(), 2);
}

#[test]
fn double_clicking_an_empty_spot_adds_a_stop_and_asks_for_its_colour() {
    let mut h = harness(Ramp::default(), false);
    click(&mut h, stop_at(0.5));
    click(&mut h, stop_at(0.5));
    assert_eq!(h.state().ramp.colors().len(), 3, "最初の押しで分岐点を足す");
    assert_eq!(h.state().requests.last().unwrap().index, 1);
    assert_eq!(h.state().selection.index, 1);
}

#[test]
fn double_clicking_the_opacity_row_or_a_midpoint_does_not_ask_for_a_colour() {
    let mut h = harness(three(), false);
    let top = RECT.top() + 7.0;
    let on_opacity = pos2(stop_at(0.0).x, top);
    click(&mut h, on_opacity);
    click(&mut h, on_opacity);
    assert!(h.state().requests.is_empty());
    // 色の行の中点（ひし形）も
    let mid = pos2(stop_at(0.25).x, RECT.top() + 54.0);
    click(&mut h, mid);
    click(&mut h, mid);
    assert!(h.state().requests.is_empty());
}
