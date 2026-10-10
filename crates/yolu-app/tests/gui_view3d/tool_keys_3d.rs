//! ツールのキーの動き方（押している間だけ・短押し／長押し）の、3D ビューの入力の道での試験（egui_kittest。試しの立方体の上で、マウスとペン。
//! ストローク・パスの矩形・グラデーションと図形と定規のドラッグの途中で離したとき）。
//! 2D は gui_canvas、状態の細かい決まりは `src/toolkeys/tests.rs`。1 フレームは 1/60 秒（長押しの境は 0.25 秒）。
use crate::common;
use crate::view3d_brush::{cube_view, press_with, release_with, screen_of};

use common::*;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::engine::Tilt;
use yolu_app::pen::PenSample;
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::toolkeys::ToolKeyMode;
use yolu_app::YoluApp;
use yolu_core::glam::Vec3;

type H = Harness<'static, YoluApp>;

fn st(h: &H) -> &AppState {
    &h.state().state
}

/// 文書の全画素の不透明度の合計（消した量の比較）。
fn alpha_sum(h: &H) -> u64 {
    let doc = &st(h).doc;
    doc.composite(doc.bounds())
        .unwrap()
        .chunks(4)
        .map(|p| u64::from(p[3]))
        .sum()
}

fn set_mode(h: &mut H, mode: ToolKeyMode) {
    h.state_mut()
        .state
        .apply(Action::ToolKeyMode("tool.eraser", mode));
    h.run();
}

fn key_event(h: &H, key: Key, pressed: bool) {
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
}

fn down(h: &mut H, key: Key) {
    key_event(h, key, true);
    h.step();
}

fn up(h: &mut H, key: Key) {
    key_event(h, key, false);
    h.step();
}

fn frames(h: &mut H, n: usize) {
    for _ in 0..n {
        h.step();
    }
}

/// 手前の面（−Z）の上の線（左 → 右）。
const A: Vec3 = Vec3::new(-0.3, 0.0, -0.5);
const M: Vec3 = Vec3::new(0.0, 0.0, -0.5);
const B: Vec3 = Vec3::new(0.3, 0.0, -0.5);

/// 手前の面に太めの線を引いた立方体（ブラシで）。
fn painted() -> (H, Rect) {
    let (mut h, rect) = cube_view();
    h.state_mut().state.brush.radius = 14.0;
    let (a, b) = (screen_of(&h, rect, A), screen_of(&h, rect, B));
    drag(&mut h, &line(a, b, 8));
    assert!(alpha_sum(&h) > 0, "ブラシで描けた: {}", st(&h).message);
    (h, rect)
}

fn line(a: Pos2, b: Pos2, n: usize) -> Vec<Pos2> {
    (0..=n)
        .map(|i| a + (b - a) * (i as f32 / n as f32))
        .collect()
}

/// 時間を使わずに引く短い線（押す・動く・離すを 1 フレームずつ）。
fn quick(h: &mut H, points: &[Pos2]) {
    press(h, points[0], PointerButton::Primary);
    h.step();
    for p in &points[1..] {
        move_to(h, *p);
        h.step();
    }
    release(h, *points.last().unwrap(), PointerButton::Primary);
    h.step();
}

#[test]
fn hold_mode_erases_on_the_model_while_the_key_is_down_and_returns() {
    let (mut h, rect) = painted();
    let painted = alpha_sum(&h);
    set_mode(&mut h, ToolKeyMode::Hold);
    down(&mut h, Key::E);
    assert_eq!(st(&h).tool, Tool::Eraser);
    h.state_mut().state.brush.radius = 8.0;
    let (a, b) = (screen_of(&h, rect, A), screen_of(&h, rect, B));
    drag(&mut h, &line(a, b, 8));
    assert!(
        alpha_sum(&h) < painted,
        "3D の上で消えた: {}",
        st(&h).message
    );
    assert_eq!(st(&h).tool, Tool::Eraser, "押している間は消しゴム");
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).tool, Tool::Brush);
    assert_eq!(st(&h).brush.radius, 14.0, "前のブラシの設定に戻る");
}

#[test]
fn releasing_the_key_in_the_middle_of_a_3d_stroke_finishes_the_whole_line_first() {
    // 基準: キーを押したまま最後まで消す
    let (mut h, rect) = painted();
    set_mode(&mut h, ToolKeyMode::Hold);
    down(&mut h, Key::E);
    h.state_mut().state.brush.radius = 8.0;
    let path = line(screen_of(&h, rect, A), screen_of(&h, rect, B), 8);
    quick(&mut h, &path);
    let full = alpha_sum(&h);
    up(&mut h, Key::E);
    h.run();
    // 同じ線を、途中でキーを離して消す
    let (mut h, rect) = painted();
    set_mode(&mut h, ToolKeyMode::Hold);
    down(&mut h, Key::E);
    h.state_mut().state.brush.radius = 8.0;
    let path = line(screen_of(&h, rect, A), screen_of(&h, rect, B), 8);
    press(&h, path[0], PointerButton::Primary);
    h.step();
    for p in &path[1..5] {
        move_to(&h, *p);
        h.step();
    }
    assert!(st(&h).is_stroking());
    up(&mut h, Key::E);
    frames(&mut h, 30);
    assert_eq!(st(&h).tool, Tool::Eraser, "描いている途中は戻さない");
    assert!(st(&h).is_stroking());
    for p in &path[5..] {
        move_to(&h, *p);
        h.step();
    }
    release(&h, *path.last().unwrap(), PointerButton::Primary);
    h.step();
    h.run();
    assert_eq!(st(&h).tool, Tool::Brush, "描き終えたら戻る");
    assert_eq!(alpha_sum(&h), full, "線は途中で切れず、最後まで消えた");
}

#[test]
fn tap_or_hold_on_the_model_keeps_after_a_short_press_and_returns_after_a_long_press_or_a_stroke() {
    let (mut h, rect) = painted();
    set_mode(&mut h, ToolKeyMode::TapOrHold);
    down(&mut h, Key::E);
    frames(&mut h, 3);
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).tool, Tool::Eraser, "短く押したら切り替えたまま");
    down(&mut h, Key::B);
    down(&mut h, Key::E);
    frames(&mut h, 30);
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).tool, Tool::Brush, "長押しは戻る");
    // 押している間に描いたら、短くても戻る
    down(&mut h, Key::E);
    let path = line(screen_of(&h, rect, A), screen_of(&h, rect, M), 4);
    quick(&mut h, &path);
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).tool, Tool::Brush);
}

fn pen_at(at: Pos2, contact: bool) -> PenSample {
    PenSample {
        pos: [at.x, at.y],
        pressure: 0.8,
        tilt: Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 5,
        time_ms: 0,
    }
}

#[test]
fn a_pen_stroke_on_the_model_started_while_held_finishes_before_the_tool_returns() {
    for mode in [ToolKeyMode::Hold, ToolKeyMode::TapOrHold] {
        let (mut h, rect) = painted();
        let painted = alpha_sum(&h);
        set_mode(&mut h, mode);
        down(&mut h, Key::E);
        h.state_mut().state.brush.radius = 8.0;
        let path = line(screen_of(&h, rect, A), screen_of(&h, rect, B), 8);
        for p in &path[..5] {
            h.state().pen().push(pen_at(*p, true));
            h.step();
        }
        assert!(st(&h).is_stroking(), "{mode:?}");
        up(&mut h, Key::E);
        frames(&mut h, 3);
        assert_eq!(st(&h).tool, Tool::Eraser, "{mode:?}: ペンの線の途中");
        for p in &path[5..] {
            h.state().pen().push(pen_at(*p, true));
            h.step();
        }
        h.state().pen().push(pen_at(*path.last().unwrap(), false));
        h.step();
        h.run();
        assert!(!st(&h).is_stroking(), "{mode:?}");
        assert_eq!(st(&h).tool, Tool::Brush, "{mode:?}: 描き終えたら戻る");
        assert!(alpha_sum(&h) < painted, "{mode:?}: ペンで消えた");
    }
}

#[test]
fn choosing_a_tool_on_the_toolbar_while_held_in_3d_keeps_that_tool() {
    let (mut h, _) = painted();
    set_mode(&mut h, ToolKeyMode::Hold);
    down(&mut h, Key::E);
    let fill = st(&h).toolset.set.first_of(Tool::Fill).unwrap();
    let at = st(&h)
        .toolset
        .ui
        .slot_rects
        .iter()
        .find(|(s, _)| *s == fill)
        .map(|(_, r)| r.center())
        .expect("ツールの列のボタン");
    click(&mut h, at);
    assert_eq!(st(&h).tool, Tool::Fill);
    up(&mut h, Key::E);
    h.run();
    assert_eq!(st(&h).tool, Tool::Fill);
}

#[test]
fn losing_the_focus_during_a_3d_stroke_returns_after_the_stroke_ends() {
    let (mut h, rect) = painted();
    set_mode(&mut h, ToolKeyMode::Hold);
    down(&mut h, Key::E);
    let path = line(screen_of(&h, rect, A), screen_of(&h, rect, B), 8);
    press(&h, path[0], PointerButton::Primary);
    h.step();
    move_to(&h, path[3]);
    h.step();
    assert!(st(&h).is_stroking());
    let steps = st(&h).doc.undo_count();
    h.event(Event::WindowFocused(false));
    h.step();
    frames(&mut h, 5);
    // フォーカスを失うと、3D の線もそこまでで確定する（ビューの決まり）。ツールはそのあとに戻る
    assert!(!st(&h).is_stroking());
    assert_eq!(st(&h).doc.undo_count(), steps + 1, "描いた所までが残る");
    assert_eq!(st(&h).tool, Tool::Brush);
    assert!(!st(&h).temp_tool.is_active());
}

#[test]
fn releasing_the_key_in_the_middle_of_a_3d_path_rectangle_drag_keeps_the_rectangle() {
    let (mut h, rect) = cube_view();
    h.state_mut()
        .state
        .apply(Action::ToolKeyMode("tool.path", ToolKeyMode::Hold));
    h.run();
    down(&mut h, Key::P);
    assert_eq!(st(&h).tool, Tool::Path);
    // 手前の面に 3 点のパスを置く
    for (x, y) in [(-0.35, -0.3), (0.0, 0.3), (0.35, -0.3)] {
        let at = screen_of(&h, rect, Vec3::new(x, y, -0.5));
        click(&mut h, at);
    }
    assert!(st(&h).path_layer().is_some(), "パスを置いた");
    // Shift + 押して、点を矩形で選ぶドラッグを始める
    let (a, b) = (
        screen_of(&h, rect, Vec3::new(-0.45, -0.45, -0.5)),
        screen_of(&h, rect, Vec3::new(0.45, 0.45, -0.5)),
    );
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.step();
    press_with(&h, a, Modifiers::SHIFT);
    h.step();
    move_to(&h, b);
    h.step();
    assert!(
        st(&h).path.rect.is_some_and(|r| r.surface),
        "3D の矩形のドラッグが始まった"
    );
    up(&mut h, Key::P);
    frames(&mut h, 20);
    assert_eq!(st(&h).tool, Tool::Path, "矩形のドラッグの途中は戻さない");
    assert!(st(&h).path.rect.is_some(), "矩形は捨てられない");
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    release_with(&h, b, Modifiers::NONE);
    h.step();
    h.run();
    assert!(st(&h).path.rect.is_none());
    assert_eq!(st(&h).tool, Tool::Brush, "ドラッグが終わったら戻る");
}

#[test]
fn releasing_the_key_in_the_middle_of_a_3d_gradient_shape_or_ruler_drag_finishes_the_drag_first() {
    for (command, key, shift, tool) in [
        ("tool.gradient", Key::G, true, Tool::Gradient),
        ("tool.shape", Key::U, false, Tool::Shape),
        ("tool.ruler", Key::U, true, Tool::Ruler),
    ] {
        let (mut h, rect) = cube_view();
        h.state_mut().state.drafting.fill = true;
        h.state_mut()
            .state
            .apply(Action::ToolKeyMode(command, ToolKeyMode::Hold));
        h.run();
        let m = if shift {
            Modifiers::SHIFT
        } else {
            Modifiers::NONE
        };
        h.event(Event::ModifiersChanged(m));
        h.event(Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: m,
        });
        h.step();
        h.step();
        assert_eq!(st(&h).tool, tool);
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.step();
        let (a, b) = (
            screen_of(&h, rect, Vec3::new(-0.3, -0.3, -0.5)),
            screen_of(&h, rect, Vec3::new(0.3, 0.3, -0.5)),
        );
        let steps = st(&h).doc.undo_count();
        press(&h, a, PointerButton::Primary);
        h.step();
        move_to(&h, a + (b - a) * 0.5);
        h.step();
        assert!(
            yolu_app::view3d::draft::dragging(st(&h)),
            "{tool:?}: 3D のドラッグが始まった"
        );
        up(&mut h, key);
        frames(&mut h, 20);
        assert_eq!(st(&h).tool, tool, "{tool:?}: ドラッグの途中は戻さない");
        assert!(yolu_app::view3d::draft::dragging(st(&h)), "{tool:?}");
        move_to(&h, b);
        h.step();
        release(&h, b, PointerButton::Primary);
        h.step();
        h.run();
        assert!(!yolu_app::view3d::draft::dragging(st(&h)), "{tool:?}");
        if tool == Tool::Ruler {
            assert_eq!(crate::common::rulers::total(st(&h)), 1, "定規を置いた");
            assert_eq!(
                st(&h).doc.undo_count(),
                steps + 1,
                "置くのが 1 回の取り消し"
            );
        } else {
            assert_eq!(
                st(&h).doc.undo_count(),
                steps + 1,
                "{tool:?}: 離したときに塗った（取り消し 1 回）: {}",
                st(&h).message
            );
        }
        assert_eq!(st(&h).tool, Tool::Brush, "{tool:?}: 描き終えたら戻る");
    }
}

/// 3D でも、押している間だけ切り替えたグラデーション・図形・定規のペンのドラッグを OS に奪われて離しが補われたら、ドラッグを終えて（図形・定規はやめる、
/// グラデーションは最後の位置で塗る）、キーが離れていれば前のツールに戻る。ツールが戻らないまま残らない。
#[test]
fn a_pen_press_taken_by_the_os_during_a_3d_hold_tool_drag_lets_the_tool_return() {
    for (command, key, shift, tool) in [
        ("tool.gradient", Key::G, true, Tool::Gradient),
        ("tool.shape", Key::U, false, Tool::Shape),
        ("tool.ruler", Key::U, true, Tool::Ruler),
    ] {
        for key_up_first in [true, false] {
            let label = format!("{tool:?} key_up_first={key_up_first}");
            let (mut h, rect) = cube_view();
            h.state_mut().state.drafting.fill = true;
            h.state_mut()
                .state
                .apply(Action::ToolKeyMode(command, ToolKeyMode::Hold));
            h.run();
            let m = if shift {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            };
            h.event(Event::ModifiersChanged(m));
            h.event(Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: m,
            });
            h.step();
            h.step();
            assert_eq!(st(&h).tool, tool, "{label}");
            h.event(Event::ModifiersChanged(Modifiers::NONE));
            h.step();
            let (a, b) = (
                screen_of(&h, rect, Vec3::new(-0.3, -0.3, -0.5)),
                screen_of(&h, rect, Vec3::new(0.3, 0.3, -0.5)),
            );
            for p in [a, a + (b - a) * 0.5, b] {
                h.state().pen().push(pen_at(p, true));
                h.step();
            }
            assert!(
                yolu_app::view3d::draft::dragging(st(&h)),
                "{label}: 3D のペンのドラッグが始まった"
            );
            if key_up_first {
                up(&mut h, key);
                frames(&mut h, 20);
                assert_eq!(st(&h).tool, tool, "{label}: ドラッグの途中は戻さない");
                assert!(yolu_app::view3d::draft::dragging(st(&h)), "{label}");
            }
            let steps = st(&h).doc.undo_count();
            h.state().pen().push_lost(pen_at(b, false));
            h.step();
            h.run();
            assert!(
                !yolu_app::view3d::draft::dragging(st(&h)),
                "{label}: ドラッグは残らない"
            );
            assert!(st(&h).view3d.input.pen_press.is_none(), "{label}");
            assert_eq!(
                crate::common::rulers::total(st(&h)),
                0,
                "{label}: 補った離しでは定規を置かない"
            );
            assert_eq!(
                st(&h).doc.undo_count(),
                steps + usize::from(tool == Tool::Gradient),
                "{label}: グラデーションだけ最後の位置で塗る: {}",
                st(&h).message
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
                up(&mut h, key);
                h.run();
                assert_eq!(st(&h).tool, Tool::Brush, "{label}: キーを離したら戻る");
                assert!(!st(&h).temp_tool.is_active(), "{label}");
            }
        }
    }
}
