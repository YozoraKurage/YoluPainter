//! ツールのキーの動き方（押すと切り替え／押している間だけ／短く押すと切り替え・長押しで押している間だけ）の、2D の入力の道での試験
//! （egui_kittest。キーの事象・マウスのドラッグ・ペンの点・ツールバーの押し）。状態の細かい決まりは `src/toolkeys/tests.rs`、3D は gui_view3d。
//! 1 フレームは 1/60 秒（長押しの境は 0.25 秒）。
use crate::common;

use common::*;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::brushes::BrushKey;
use yolu_app::lang::Lang;
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::toolkeys::ToolKeyMode;
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

fn st(h: &H) -> &AppState {
    &h.state().state
}

/// 太い線を 1 本引いた文書（256 × 256）と、線の中心の画面の点。ブラシの太さは 30。
fn painted() -> (H, Pos2) {
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.brush.radius = 30.0;
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -60.0, 0.0), offset(c, 60.0, 0.0)]);
    assert_eq!(canvas_pixel(&h, c)[3], 255);
    (h, c)
}

fn set_mode(h: &mut H, command: &'static str, mode: ToolKeyMode) {
    h.state_mut()
        .state
        .apply(Action::ToolKeyMode(command, mode));
    h.run();
}

fn key_event(h: &H, key: Key, pressed: bool, repeat: bool, modifiers: Modifiers) {
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers,
    });
}

fn down(h: &mut H, key: Key) {
    key_event(h, key, true, false, Modifiers::NONE);
    h.step();
}

fn up(h: &mut H, key: Key) {
    key_event(h, key, false, false, Modifiers::NONE);
    h.step();
}

fn frames(h: &mut H, n: usize) {
    for _ in 0..n {
        h.step();
    }
}

/// 時間を使わずに引く短いストローク（押す・動く・離すを 1 フレームずつ）。
fn quick_stroke(h: &mut H, points: &[Pos2]) {
    press(h, points[0], PointerButton::Primary);
    h.step();
    for p in &points[1..] {
        move_to(h, *p);
        h.step();
    }
    release(h, *points.last().unwrap(), PointerButton::Primary);
    h.step();
}

fn is_eraser_with_small_brush(h: &mut H) {
    assert_eq!(st(h).tool, Tool::Eraser);
    h.state_mut().state.brush.radius = 4.0;
}

#[test]
fn a_key_in_the_default_tap_mode_switches_for_good() {
    let (mut h, _) = painted();
    down(&mut h, Key::E);
    assert_eq!(st(&h).tool, Tool::Eraser);
    frames(&mut h, 40);
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).tool, Tool::Eraser, "長く押しても、離したら戻さない");
    assert!(!st(&h).temp_tool.is_active());
}

#[test]
fn hold_mode_erases_while_the_key_is_down_and_returns_to_the_previous_brush() {
    let (mut h, c) = painted();
    set_mode(&mut h, "tool.eraser", ToolKeyMode::Hold);
    let steps = st(&h).doc.undo_count();
    down(&mut h, Key::E);
    is_eraser_with_small_brush(&mut h);
    assert!(st(&h).temp_tool.is_active());
    drag(&mut h, &[offset(c, -40.0, 0.0), offset(c, 40.0, 0.0)]);
    assert_eq!(st(&h).doc.undo_count(), steps + 1);
    assert!(canvas_pixel(&h, c)[3] < 255, "消しゴムで消えた");
    assert_eq!(st(&h).tool, Tool::Eraser, "押している間は消しゴムのまま");
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).tool, Tool::Brush);
    assert_eq!(st(&h).brush.radius, 30.0, "前のブラシの設定に戻る");
    assert_eq!(
        st(&h).toolset.set.active(),
        st(&h).toolset.set.first_of(Tool::Brush)
    );
    assert!(!st(&h).temp_tool.is_active());
    // 戻ったあと、また描ける（ブラシで描く）
    let off = offset(c, 0.0, 80.0);
    drag(&mut h, &[offset(off, -30.0, 0.0), offset(off, 30.0, 0.0)]);
    assert_eq!(canvas_pixel(&h, off)[3], 255);
}

#[test]
fn releasing_the_key_in_the_middle_of_a_stroke_finishes_the_line_before_returning() {
    let (mut h, c) = painted();
    set_mode(&mut h, "tool.eraser", ToolKeyMode::Hold);
    down(&mut h, Key::E);
    is_eraser_with_small_brush(&mut h);
    let (a, b, d) = (
        offset(c, -50.0, 0.0),
        offset(c, 0.0, 0.0),
        offset(c, 50.0, 0.0),
    );
    press(&h, a, PointerButton::Primary);
    h.step();
    move_to(&h, b);
    h.step();
    assert!(st(&h).is_stroking());
    up(&mut h, Key::E);
    frames(&mut h, 30);
    assert_eq!(st(&h).tool, Tool::Eraser, "描いている途中は戻さない");
    assert!(st(&h).temp_tool.is_active());
    // 続きを描ける（線は途中で切れない）
    move_to(&h, d);
    h.step();
    release(&h, d, PointerButton::Primary);
    h.step();
    h.run();
    assert_eq!(st(&h).tool, Tool::Brush, "描き終えたら戻る");
    for x in [-45.0, -10.0, 10.0, 45.0] {
        assert!(
            canvas_pixel(&h, offset(c, x, 0.0))[3] < 255,
            "キーを離したあとの区間も消えた: {x}"
        );
    }
    assert_eq!(
        st(&h).doc.undo_count(),
        2,
        "線は 1 本（描いた線と消した線）"
    );
}

#[test]
fn tap_or_hold_keeps_the_tool_after_a_short_press_and_goes_back_after_a_long_one() {
    let (mut h, c) = painted();
    set_mode(&mut h, "tool.eraser", ToolKeyMode::TapOrHold);
    // 短く押して離す: 消しゴムのまま
    down(&mut h, Key::E);
    frames(&mut h, 3);
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).tool, Tool::Eraser);
    assert!(!st(&h).temp_tool.is_active());
    // 長押しして離す（描かない）: 戻る
    down(&mut h, Key::B);
    assert_eq!(st(&h).tool, Tool::Brush);
    down(&mut h, Key::E);
    frames(&mut h, 30);
    assert_eq!(st(&h).tool, Tool::Eraser);
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).tool, Tool::Brush, "長押しは離すと戻る");
    // 押している間に描いて、短く離す: 戻る
    down(&mut h, Key::E);
    quick_stroke(&mut h, &[offset(c, -20.0, 0.0), offset(c, 20.0, 0.0)]);
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).tool, Tool::Brush, "押している間に描いたら戻る");
}

#[test]
fn choosing_a_tool_on_the_toolbar_while_the_key_is_down_cancels_the_return() {
    let (mut h, _) = painted();
    set_mode(&mut h, "tool.eraser", ToolKeyMode::Hold);
    down(&mut h, Key::E);
    assert!(st(&h).temp_tool.is_active());
    let fill = st(&h).toolset.set.first_of(Tool::Fill).unwrap();
    let at = st(&h)
        .toolset
        .ui
        .slot_rects
        .iter()
        .find(|(s, _)| *s == fill)
        .map(|(_, r)| r.center())
        .unwrap();
    click(&mut h, at);
    assert_eq!(st(&h).tool, Tool::Fill);
    assert!(!st(&h).temp_tool.is_active(), "戻すのをやめた");
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).tool, Tool::Fill, "選んだツールが残る");
    // 同じ消しゴムのボタンを押し直しても、そのまま消しゴム
    down(&mut h, Key::B);
    down(&mut h, Key::E);
    assert!(st(&h).temp_tool.is_active());
    let eraser = st(&h).toolset.set.first_of(Tool::Eraser).unwrap();
    let at = st(&h)
        .toolset
        .ui
        .slot_rects
        .iter()
        .find(|(s, _)| *s == eraser)
        .map(|(_, r)| r.center())
        .unwrap();
    click(&mut h, at);
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).tool, Tool::Eraser);
}

#[test]
fn choosing_another_tool_by_key_or_another_brush_from_the_list_while_held_keeps_that_choice() {
    let (mut h, _) = painted();
    set_mode(&mut h, "tool.eraser", ToolKeyMode::Hold);
    down(&mut h, Key::E);
    down(&mut h, Key::M);
    assert_eq!(st(&h).tool, Tool::SelectRect);
    up(&mut h, Key::M);
    up(&mut h, Key::E);
    h.run();
    assert_eq!(
        st(&h).tool,
        Tool::SelectRect,
        "別のキーで選んだツールが残る"
    );
    // 一覧で同じ消しゴムのツールの別のブラシを選んでも、そのブラシのまま
    down(&mut h, Key::B);
    down(&mut h, Key::E);
    assert!(st(&h).temp_tool.is_active());
    h.state_mut()
        .state
        .apply(Action::Brush(yolu_app::brushes::BrushAction::Select(
            BrushKey::Builtin("soft-eraser"),
        )));
    h.run();
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).tool, Tool::Eraser);
    assert_eq!(
        st(&h).brushes.lib.current(),
        BrushKey::Builtin("soft-eraser")
    );
    assert!(!st(&h).temp_tool.is_active());
}

#[test]
fn losing_the_focus_returns_at_once_or_after_the_stroke() {
    let (mut h, c) = painted();
    set_mode(&mut h, "tool.eraser", ToolKeyMode::Hold);
    down(&mut h, Key::E);
    h.event(Event::WindowFocused(false));
    h.step();
    h.run();
    assert_eq!(st(&h).tool, Tool::Brush, "描いていなければすぐ戻る");
    h.event(Event::WindowFocused(true));
    h.step();
    // 描いている途中でフォーカスを失う
    down(&mut h, Key::E);
    press(&h, offset(c, -30.0, 40.0), PointerButton::Primary);
    h.step();
    move_to(&h, offset(c, 0.0, 40.0));
    h.step();
    assert!(st(&h).is_stroking());
    let steps = st(&h).doc.undo_count();
    h.event(Event::WindowFocused(false));
    h.step();
    frames(&mut h, 5);
    // フォーカスを失うと、線はそこまでで確定する（キャンバスの決まり）。ツールはそのあとに戻る
    assert!(!st(&h).is_stroking());
    assert_eq!(st(&h).doc.undo_count(), steps + 1, "描いた所までが残る");
    assert_eq!(st(&h).tool, Tool::Brush);
    assert!(!st(&h).temp_tool.is_active());
}

#[test]
fn a_key_for_the_tool_already_selected_changes_nothing_and_a_release_does_nothing() {
    let (mut h, _) = painted();
    set_mode(&mut h, "tool.brush", ToolKeyMode::Hold);
    let before = (st(&h).tool, st(&h).brush.radius);
    down(&mut h, Key::B);
    assert!(!st(&h).temp_tool.is_active());
    frames(&mut h, 30);
    up(&mut h, Key::B);
    h.run();
    assert_eq!((st(&h).tool, st(&h).brush.radius), before);
}

#[test]
fn key_repeat_does_not_take_the_tool_back_while_held() {
    let (mut h, _) = painted();
    set_mode(&mut h, "tool.eraser", ToolKeyMode::Hold);
    down(&mut h, Key::E);
    let fill = st(&h).toolset.set.first_of(Tool::Fill).unwrap();
    h.state_mut()
        .state
        .apply(Action::Tools(yolu_app::toolset::ToolsetAction::Select(
            fill,
        )));
    for _ in 0..5 {
        key_event(&h, Key::E, true, true, Modifiers::NONE);
        h.step();
    }
    assert_eq!(st(&h).tool, Tool::Fill);
    up(&mut h, Key::E);
    assert_eq!(st(&h).tool, Tool::Fill);
}

#[test]
fn a_shifted_tool_key_ends_with_the_key_even_if_shift_is_released_first() {
    let (mut h, _) = painted();
    set_mode(&mut h, "tool.gradient", ToolKeyMode::Hold);
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    key_event(&h, Key::G, true, false, Modifiers::SHIFT);
    h.step();
    h.step();
    assert_eq!(st(&h).tool, Tool::Gradient);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.step();
    frames(&mut h, 20);
    assert_eq!(
        st(&h).tool,
        Tool::Gradient,
        "Shift を先に離しても、G を押している間"
    );
    up(&mut h, Key::G);
    h.run();
    assert_eq!(st(&h).tool, Tool::Brush);
}

fn pen_at(at: Pos2, contact: bool) -> yolu_app::pen::PenSample {
    yolu_app::pen::PenSample {
        pos: [at.x, at.y],
        pressure: 0.8,
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 7,
        time_ms: 0,
    }
}

fn pen_frames(h: &mut H, steps: &[(Pos2, bool)]) {
    for (at, contact) in steps {
        h.state().pen().push(pen_at(*at, *contact));
        h.step();
    }
}

#[test]
fn a_pen_stroke_started_while_held_finishes_before_the_tool_returns() {
    for mode in [ToolKeyMode::Hold, ToolKeyMode::TapOrHold] {
        let (mut h, c) = painted();
        set_mode(&mut h, "tool.eraser", mode);
        down(&mut h, Key::E);
        is_eraser_with_small_brush(&mut h);
        let (a, b, d) = (
            offset(c, -40.0, 0.0),
            offset(c, 0.0, 0.0),
            offset(c, 40.0, 0.0),
        );
        pen_frames(&mut h, &[(a, true), (b, true)]);
        assert!(st(&h).is_stroking(), "{mode:?}");
        up(&mut h, Key::E);
        assert_eq!(st(&h).tool, Tool::Eraser, "{mode:?}: ペンの線の途中");
        pen_frames(&mut h, &[(d, true), (d, false)]);
        h.run();
        assert!(!st(&h).is_stroking(), "{mode:?}");
        assert_eq!(st(&h).tool, Tool::Brush, "{mode:?}: ペンで描き終えたら戻る");
        for x in [-35.0, 10.0, 35.0] {
            assert!(
                canvas_pixel(&h, offset(c, x, 0.0))[3] < 255,
                "{mode:?}: ペンの線が全部消えた: {x}"
            );
        }
    }
}

/// 押している間だけ切り替えたツールで、ペンの押しを OS に奪われて離しが補われても、ツールが戻らないまま残らない: キーを先に離して待っていても、押しを後で
/// 奪われても、ドラッグを終えて（図形は取りやめ・グラデーションは最後の位置で塗る）、キーが離れていれば前のツールに戻る。
#[test]
fn a_pen_press_taken_by_the_os_during_a_hold_tool_drag_lets_the_tool_return() {
    for (command, key, shift, tool, paints) in [
        ("tool.shape", Key::U, false, Tool::Shape, false),
        ("tool.gradient", Key::G, true, Tool::Gradient, true),
    ] {
        for key_up_first in [true, false] {
            let label = format!("{tool:?} key_up_first={key_up_first}");
            let (mut h, c) = painted();
            h.state_mut().state.drafting.fill = true;
            set_mode(&mut h, command, ToolKeyMode::Hold);
            let m = if shift {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            };
            h.event(Event::ModifiersChanged(m));
            key_event(&h, key, true, false, m);
            h.step();
            h.step();
            assert_eq!(st(&h).tool, tool, "{label}");
            h.event(Event::ModifiersChanged(Modifiers::NONE));
            h.step();
            let (a, b) = (offset(c, -50.0, 0.0), offset(c, 50.0, 40.0));
            pen_frames(&mut h, &[(a, true), (b, true)]);
            let dragging = |h: &H| st(h).drafting.drag.is_some() || st(h).gradient.drag.is_some();
            assert!(dragging(&h), "{label}: ペンのドラッグが始まった");
            if key_up_first {
                up(&mut h, key);
                frames(&mut h, 20);
                assert_eq!(st(&h).tool, tool, "{label}: ドラッグの途中は戻さない");
                assert!(dragging(&h), "{label}");
            }
            // 押しを奪われて、離しが補われた
            let steps = st(&h).doc.undo_count();
            h.state().pen().push_lost(pen_at(b, false));
            h.step();
            h.run();
            assert!(!dragging(&h), "{label}: ドラッグは残らない");
            assert!(st(&h).canvas.pen_press.is_none(), "{label}");
            assert_eq!(
                st(&h).doc.undo_count(),
                steps + usize::from(paints),
                "{label}: 図形は取りやめ・グラデーションは最後の位置で塗る"
            );
            if key_up_first {
                assert_eq!(st(&h).tool, Tool::Brush, "{label}: 奪われたあとで戻る");
                assert!(!st(&h).temp_tool.is_active(), "{label}");
            } else {
                assert_eq!(
                    st(&h).tool,
                    tool,
                    "{label}: キーを押している間は、そのツール"
                );
                assert!(st(&h).temp_tool.is_active(), "{label}");
                up(&mut h, key);
                h.run();
                assert_eq!(st(&h).tool, Tool::Brush, "{label}: キーを離したら戻る");
                assert!(!st(&h).temp_tool.is_active(), "{label}");
            }
        }
    }
}

#[test]
fn the_toolbar_dot_marks_the_tool_that_comes_back_and_the_tooltips_stay_plain() {
    let (mut h, _) = painted();
    set_mode(&mut h, "tool.eraser", ToolKeyMode::Hold);
    let brush = st(&h).toolset.set.first_of(Tool::Brush).unwrap();
    assert_eq!(st(&h).temp_tool_return_slot(), None);
    assert!(h.query_by_label("ブラシ（B）").is_some());
    down(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).temp_tool_return_slot(), Some(brush));
    // 戻る先のボタンも、ふだんのツールチップ（アクセシビリティの名前）のまま。説明は添えない
    assert!(h.query_by_label("ブラシ（B）").is_some());
    assert!(h.query_all_by_label_contains("離すと戻る").next().is_none());
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).temp_tool_return_slot(), None);
    assert!(h.query_by_label("ブラシ（B）").is_some());
    // 英語
    h.state_mut().state.set_language(Lang::En);
    h.run();
    down(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).temp_tool_return_slot(), Some(brush));
    assert!(h.query_by_label("Brush (B)").is_some());
    assert!(h
        .query_all_by_label_contains("Returns on release")
        .next()
        .is_none());
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

#[test]
fn releasing_the_key_in_the_middle_of_a_path_point_rectangle_drag_keeps_the_rectangle() {
    let (mut h, c) = painted();
    set_mode(&mut h, "tool.path", ToolKeyMode::Hold);
    down(&mut h, Key::P);
    assert_eq!(st(&h).tool, Tool::Path);
    for p in [
        offset(c, -40.0, 60.0),
        offset(c, 0.0, 90.0),
        offset(c, 40.0, 60.0),
    ] {
        click(&mut h, p);
    }
    assert!(st(&h).path_layer().is_some(), "パスを置いた");
    // Shift + 押して、点を矩形で選ぶドラッグを始める
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.step();
    press_with(&h, offset(c, -60.0, 40.0), Modifiers::SHIFT);
    h.step();
    move_to(&h, offset(c, 0.0, 70.0));
    h.step();
    assert!(st(&h).path.rect.is_some(), "矩形のドラッグが始まった");
    up(&mut h, Key::P);
    frames(&mut h, 20);
    assert_eq!(st(&h).tool, Tool::Path, "矩形のドラッグの途中は戻さない");
    assert!(st(&h).path.rect.is_some(), "矩形は捨てられない");
    move_to(&h, offset(c, 60.0, 100.0));
    h.step();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    release(&h, offset(c, 60.0, 100.0), PointerButton::Primary);
    h.step();
    h.run();
    assert!(st(&h).path.rect.is_none());
    assert_eq!(st(&h).tool, Tool::Brush, "ドラッグが終わったら戻る");
}

#[test]
fn releasing_the_key_in_the_middle_of_a_gradient_drag_waits_for_the_drag() {
    let (mut h, c) = painted();
    set_mode(&mut h, "tool.gradient", ToolKeyMode::Hold);
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    key_event(&h, Key::G, true, false, Modifiers::SHIFT);
    h.step();
    h.step();
    assert_eq!(st(&h).tool, Tool::Gradient);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.step();
    press(&h, offset(c, -50.0, 0.0), PointerButton::Primary);
    h.step();
    move_to(&h, offset(c, 0.0, 0.0));
    h.step();
    assert!(
        st(&h).gradient.drag.is_some(),
        "グラデーションのドラッグが始まった"
    );
    up(&mut h, Key::G);
    frames(&mut h, 20);
    assert_eq!(st(&h).tool, Tool::Gradient, "ドラッグの途中は戻さない");
    assert!(st(&h).gradient.drag.is_some());
    let steps = st(&h).doc.undo_count();
    move_to(&h, offset(c, 50.0, 0.0));
    h.step();
    release(&h, offset(c, 50.0, 0.0), PointerButton::Primary);
    h.step();
    h.run();
    assert_eq!(
        st(&h).doc.undo_count(),
        steps + 1,
        "ドラッグした分が塗られた"
    );
    assert_eq!(st(&h).tool, Tool::Brush);
}

#[test]
fn pressing_the_same_key_again_during_the_stroke_keeps_the_eraser_until_the_stroke_and_the_key_end()
{
    for mode in [ToolKeyMode::Hold, ToolKeyMode::TapOrHold] {
        let (mut h, c) = painted();
        set_mode(&mut h, "tool.eraser", mode);
        down(&mut h, Key::E);
        let a = offset(c, -50.0, 0.0);
        press(&h, a, PointerButton::Primary);
        h.step();
        move_to(&h, offset(c, -20.0, 0.0));
        h.step();
        assert!(st(&h).is_stroking(), "{mode:?}");
        up(&mut h, Key::E);
        frames(&mut h, 20);
        // 同じ線のうちに、同じキーを押し直す
        down(&mut h, Key::E);
        assert_eq!(st(&h).tool, Tool::Eraser, "{mode:?}");
        assert!(st(&h).temp_tool.is_active(), "{mode:?}: 続ける");
        move_to(&h, offset(c, 20.0, 0.0));
        h.step();
        release(&h, offset(c, 20.0, 0.0), PointerButton::Primary);
        h.step();
        h.run();
        assert_eq!(
            st(&h).tool,
            Tool::Eraser,
            "{mode:?}: キーを押している間は消しゴム"
        );
        up(&mut h, Key::E);
        h.run();
        assert_eq!(st(&h).tool, Tool::Brush, "{mode:?}: 離したら戻る");
        assert!(!st(&h).temp_tool.is_active(), "{mode:?}");
    }
}

#[test]
fn a_hold_key_pressed_over_another_returns_to_the_earlier_tool_first() {
    let (mut h, c) = painted();
    set_mode(&mut h, "tool.eraser", ToolKeyMode::Hold);
    set_mode(&mut h, "tool.fill", ToolKeyMode::Hold);
    down(&mut h, Key::E);
    down(&mut h, Key::G);
    assert_eq!(st(&h).tool, Tool::Fill);
    // バケツで塗る（1 回の押し）
    quick_stroke(&mut h, &[offset(c, 0.0, 100.0), offset(c, 0.0, 100.0)]);
    up(&mut h, Key::G);
    h.run();
    assert_eq!(
        st(&h).tool,
        Tool::Eraser,
        "E をまだ押している間は消しゴムに戻る"
    );
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).tool, Tool::Brush);
}

#[test]
fn the_brush_in_use_comes_back_after_the_temporary_eraser() {
    let (mut h, _) = painted();
    h.state_mut()
        .state
        .apply(Action::Brush(yolu_app::brushes::BrushAction::Select(
            BrushKey::Builtin("hard-round"),
        )));
    h.run();
    let before = st(&h).brushes.lib.current();
    set_mode(&mut h, "tool.eraser", ToolKeyMode::Hold);
    down(&mut h, Key::E);
    assert_ne!(st(&h).brushes.lib.current(), before);
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).brushes.lib.current(), before);
}

#[test]
fn releasing_the_key_in_the_middle_of_a_selection_drag_keeps_the_shape_until_the_drag_ends() {
    let (mut h, c) = painted();
    set_mode(&mut h, "tool.select_rectangle", ToolKeyMode::Hold);
    down(&mut h, Key::M);
    assert_eq!(st(&h).tool, Tool::SelectRect);
    press(&h, offset(c, -40.0, -40.0), PointerButton::Primary);
    h.step();
    move_to(&h, offset(c, 0.0, 0.0));
    h.step();
    up(&mut h, Key::M);
    frames(&mut h, 20);
    assert_eq!(
        st(&h).tool,
        Tool::SelectRect,
        "形をドラッグしている途中は戻さない"
    );
    move_to(&h, offset(c, 40.0, 40.0));
    h.step();
    release(&h, offset(c, 40.0, 40.0), PointerButton::Primary);
    h.step();
    h.run();
    assert!(
        st(&h).doc.selection().is_some(),
        "ドラッグした形が選択範囲になった"
    );
    assert_eq!(st(&h).tool, Tool::Brush);
}

#[test]
fn a_polygon_selection_in_progress_keeps_the_tool_until_it_is_finished_or_cancelled() {
    let (mut h, c) = painted();
    set_mode(&mut h, "tool.select_polygon", ToolKeyMode::Hold);
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    key_event(&h, Key::L, true, false, Modifiers::SHIFT);
    h.step();
    h.step();
    assert_eq!(st(&h).tool, Tool::Polygon);
    for p in [
        offset(c, -40.0, -40.0),
        offset(c, 40.0, -40.0),
        offset(c, 0.0, 40.0),
    ] {
        click(&mut h, p);
    }
    assert!(st(&h).sel_has_polygon_point());
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    up(&mut h, Key::L);
    frames(&mut h, 20);
    assert_eq!(st(&h).tool, Tool::Polygon, "多角形の途中は戻さない");
    // Esc で取りやめると、戻る
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(!st(&h).sel_has_polygon_point());
    assert_eq!(st(&h).tool, Tool::Brush);
}

#[test]
fn cancelling_the_stroke_with_escape_after_the_release_lets_the_tool_return() {
    let (mut h, c) = painted();
    set_mode(&mut h, "tool.eraser", ToolKeyMode::Hold);
    down(&mut h, Key::E);
    let before = alpha_at(&h, c);
    press(&h, offset(c, -30.0, 0.0), PointerButton::Primary);
    h.step();
    move_to(&h, offset(c, 0.0, 0.0));
    h.step();
    assert!(st(&h).is_stroking());
    up(&mut h, Key::E);
    frames(&mut h, 5);
    assert_eq!(st(&h).tool, Tool::Eraser);
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(!st(&h).is_stroking());
    assert_eq!(st(&h).tool, Tool::Brush, "取りやめたら戻る");
    assert_eq!(alpha_at(&h, c), before, "取りやめた線は残らない");
    release(&h, offset(c, 0.0, 0.0), PointerButton::Primary);
    h.run();
}

fn alpha_at(h: &H, p: Pos2) -> [u8; 4] {
    canvas_pixel(h, p)
}

fn crop(h: &mut H, rect: egui::Rect, name: &str) {
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        rect.left().max(0.0).floor() as u32,
        rect.top().max(0.0).floor() as u32,
        rect.width().ceil() as u32,
        rect.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

#[test]
fn snapshot_the_toolbar_dot_and_tip_of_the_tool_that_comes_back_in_both_languages() {
    for (lang, name) in [
        (Lang::Ja, "tool_keys_return_ja"),
        (Lang::En, "tool_keys_return_en"),
    ] {
        let (mut h, c) = painted();
        h.state_mut().state.set_language(lang);
        set_mode(&mut h, "tool.eraser", ToolKeyMode::Hold);
        down(&mut h, Key::E);
        let brush = st(&h).toolset.set.first_of(Tool::Brush).unwrap();
        let at = st(&h)
            .toolset
            .ui
            .slot_rects
            .iter()
            .find(|(s, _)| *s == brush)
            .map(|(_, r)| r.center())
            .unwrap();
        let area = egui::Rect::from_min_size(egui::pos2(0.0, 60.0), egui::vec2(300.0, 250.0));
        // 点（ポインタをほかへ置いて）
        move_to(&h, c);
        frames(&mut h, 3);
        crop(&mut h, area, name);
        // ツールチップ（戻る先のボタンの上で）
        hover_and_wait(&mut h, at);
        crop(&mut h, area, &format!("{name}_tip"));
    }
}
