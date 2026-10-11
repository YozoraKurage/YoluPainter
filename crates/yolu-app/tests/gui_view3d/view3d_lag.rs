//! 3D ビューで大きなブラシを速く動かしても画面が固まらない（egui_kittest。試しの立方体の上で）: 入力は区間のダブを並べるだけ、塗るのは 1 フレームに
//! 時間の枠まで（最初の 1 つは必ず塗る）。離したあとも同じ枠で塗り続け、塗り終えたら確定する（それまでは描いている最中のまま）。新しい押し・
//! Esc・フォーカスを失う・ビューが隠れるの扱いと、どの切り方でも同じ画素になることを見る。
//! 時間の枠は 0 にして（1 フレーム 1 ダブ）、フレームごとの進みを決めて見る。
use crate::common;
use crate::view3d_brush::{cube_view, press_with, release_with, screen_of};

use common::*;
use egui::{Event, Key, Modifiers, Pos2, Rect};
use egui_kittest::Harness;
use std::time::Duration;
use yolu_app::engine::Tilt;
use yolu_app::pen::PenSample;
use yolu_app::YoluApp;
use yolu_core::glam::Vec3;

type H = Harness<'static, YoluApp>;

/// 試しの立方体の手前の面を横切って折れ曲がる、速い動きの点（画面の点）。
fn path(h: &H, rect: Rect, corners: &[Vec3]) -> Vec<Pos2> {
    corners.iter().map(|c| screen_of(h, rect, *c)).collect()
}

fn first_line() -> [Vec3; 4] {
    [
        Vec3::new(-0.45, 0.3, -0.5),
        Vec3::new(0.45, -0.3, -0.5),
        Vec3::new(0.45, -0.25, -0.5),
        Vec3::new(0.4, -0.2, -0.5),
    ]
}

fn second_line() -> [Vec3; 3] {
    [
        Vec3::new(-0.4, -0.35, -0.5),
        Vec3::new(0.4, 0.35, -0.5),
        Vec3::new(0.35, 0.3, -0.5),
    ]
}

/// 小さなダブをたくさん置く設定で、時間の枠を決めて始める。
fn view(budget: Duration) -> (H, Rect) {
    let (mut h, rect) = cube_view();
    let state = &mut h.state_mut().state;
    state.brush.radius = 3.0;
    state.brush.spacing = 0.2;
    state.view3d.input.paint_budget = Some(budget);
    (h, rect)
}

/// 同じフレームのうちに全部の点を渡す（1 フレームに何回も入力が来る速いペン・マウスの見立て）。
fn move_all(h: &mut H, points: &[Pos2]) {
    for p in points {
        h.input_mut().events.push(Event::PointerMoved(*p));
    }
    h.step();
}

fn queued(h: &H) -> Option<usize> {
    h.state()
        .state
        .view3d
        .input
        .surface
        .as_ref()
        .map(|s| s.queued())
}

fn painted_dabs(h: &H) -> usize {
    h.state()
        .state
        .view3d
        .input
        .surface
        .as_ref()
        .map_or(0, |s| s.stats.dabs + s.stats.missed)
}

fn snapshot(h: &H) -> Vec<u8> {
    let doc = &h.state().state.doc;
    doc.composite(doc.bounds()).unwrap()
}

fn message(h: &H) -> String {
    h.state().state.message.clone()
}

fn stroking(h: &H) -> bool {
    h.state().state.is_stroking()
}

fn released(h: &H) -> bool {
    h.state().state.view3d.input.released
}

/// 離してから、描いている最中でなくなるまでフレームを進める（上限つき）。進んだフレームの数を返す。
fn drain(h: &mut H) -> usize {
    let mut frames = 0;
    while stroking(h) {
        h.step();
        frames += 1;
        assert!(frames < 5000, "確定しない");
    }
    frames
}

/// 速い線を引いて離し、残りを持ち越した（描いている最中のまま）状態にする。
fn leave_leftover(h: &mut H, rect: Rect, corners: &[Vec3]) {
    let points = path(h, rect, corners);
    press_with(h, points[0], Modifiers::NONE);
    h.step();
    move_all(h, &points[1..]);
    release_with(h, *points.last().unwrap(), Modifiers::NONE);
    h.step();
    assert!(released(h) && stroking(h), "離した: 確定待ち");
    assert!(queued(h).unwrap() > 20, "残りを持ち越している");
    assert_eq!(h.state().state.doc.undo_count(), 0, "まだ確定していない");
}

/// 基準: 時間の枠が十分に長い（1 フレームで全部塗る）ときの、1 本の線の確定後の画素。
fn whole(lines: &[&[Vec3]]) -> (Vec<u8>, usize) {
    let (mut h, rect) = view(Duration::from_secs(600));
    let before = snapshot(&h);
    for corners in lines {
        let points = path(&h, rect, corners);
        press_with(&h, points[0], Modifiers::NONE);
        h.step();
        move_all(&mut h, &points[1..]);
        release_with(&h, *points.last().unwrap(), Modifiers::NONE);
        h.step();
        h.run();
        assert!(!stroking(&h));
    }
    assert!(message(&h).is_empty(), "{}", message(&h));
    assert_ne!(snapshot(&h), before, "線が描けている");
    let undo = h.state().state.doc.undo_count();
    (snapshot(&h), undo)
}

/// 入力では塗らず、1 フレームに 1 ダブずつ塗る。離したあとも描いている最中のまま塗り続け、塗り終えたら 1 本の線として確定する。
/// 画素は、1 フレームで全部塗ったときと同じ。
#[test]
fn a_released_stroke_keeps_painting_one_dab_per_frame_and_commits_once() {
    let (expected, expected_undo) = whole(&[&first_line()]);
    assert_eq!(expected_undo, 1);

    let (mut h, rect) = view(Duration::ZERO);
    let points = path(&h, rect, &first_line());
    press_with(&h, points[0], Modifiers::NONE);
    h.step();
    assert_eq!(painted_dabs(&h), 1, "押した点のダブ");
    // 1 フレームに全部の点が来ても、塗るのは 1 ダブだけ（入力は並べるだけ）
    move_all(&mut h, &points[1..]);
    assert_eq!(
        painted_dabs(&h),
        2,
        "入力の点の数によらず 1 フレーム 1 ダブ"
    );
    let waiting = queued(&h).unwrap();
    assert!(waiting > 20, "持ち越したダブがたくさんある ({waiting})");
    // 離した: 描いている最中のまま、残りを塗り続ける
    release_with(&h, *points.last().unwrap(), Modifiers::NONE);
    h.step();
    assert!(released(&h), "離した印");
    assert!(stroking(&h), "残りがあるあいだは描いている最中");
    assert_eq!(h.state().state.doc.undo_count(), 0, "まだ確定していない");
    assert!(h.state().state.view3d.input.surface.is_some());
    assert!(!h.state().state.can_edit(), "ほかの編集は断る");
    let mut before = painted_dabs(&h);
    let mut left = queued(&h).unwrap();
    assert!(left > 20);
    // マウスを動かしても点は増えない（離したあとの点は受けない）
    h.event(Event::PointerMoved(points[0]));
    h.step();
    let mut frames = 1;
    while stroking(&h) {
        let now = painted_dabs(&h);
        match queued(&h) {
            Some(q) => {
                assert!(q < left, "フレームごとに進む（{q} < {left}）");
                assert_eq!(now, before + 1, "1 フレーム 1 ダブ");
                left = q;
                before = now;
            }
            None => break,
        }
        h.step();
        frames += 1;
        assert!(frames < 5000);
    }
    assert!(frames > 20, "何フレームにも分けて塗った ({frames})");
    assert!(!stroking(&h));
    assert!(h.state().state.view3d.input.surface.is_none());
    assert!(!released(&h), "確定したら印は下ろす");
    assert_eq!(h.state().state.doc.undo_count(), 1, "1 本の線");
    assert!(message(&h).is_empty(), "{}", message(&h));
    assert!(snapshot(&h) == expected, "全部を一度に塗ったときと同じ画素");
}

/// 離したあとの残りを塗っている途中で新しく押したら、残りをその場で全部塗って確定してから、新しいストロークを始める（押しを捨てない）。
/// 2 本の線は、それぞれ全部塗ったときと同じ画素の、2 回の Undo。
#[test]
fn a_press_during_the_leftover_finishes_it_and_starts_the_next_stroke() {
    let (expected, expected_undo) = whole(&[&first_line(), &second_line()]);
    assert_eq!(expected_undo, 2);

    let (mut h, rect) = view(Duration::ZERO);
    let first = path(&h, rect, &first_line());
    press_with(&h, first[0], Modifiers::NONE);
    h.step();
    move_all(&mut h, &first[1..]);
    release_with(&h, *first.last().unwrap(), Modifiers::NONE);
    h.step();
    assert!(released(&h) && stroking(&h));
    assert!(queued(&h).unwrap() > 20, "残りを持ち越している");

    let second = path(&h, rect, &second_line());
    press_with(&h, second[0], Modifiers::NONE);
    h.step();
    assert_eq!(
        h.state().state.doc.undo_count(),
        1,
        "前の線は、押した時に確定した"
    );
    assert!(!released(&h), "新しいストローク");
    assert!(
        h.state().state.view3d.input.stroke.is_some(),
        "押しを受けて描き始めた"
    );
    assert_eq!(painted_dabs(&h), 1, "新しいストロークの押した点のダブ");
    move_all(&mut h, &second[1..]);
    release_with(&h, *second.last().unwrap(), Modifiers::NONE);
    h.step();
    drain(&mut h);
    assert_eq!(h.state().state.doc.undo_count(), 2, "2 本の線");
    assert!(message(&h).is_empty(), "{}", message(&h));
    assert!(snapshot(&h) == expected, "それぞれ全部塗ったときと同じ画素");
}

/// 離す前の Esc は、今と同じくストロークごと捨てる（描いた分も戻る。Undo に積まない）。
#[test]
fn escape_before_the_release_still_discards_the_whole_stroke() {
    let (mut h, rect) = view(Duration::ZERO);
    let before = snapshot(&h);
    let points = path(&h, rect, &first_line());
    press_with(&h, points[0], Modifiers::NONE);
    h.step();
    move_all(&mut h, &points[1..]);
    for _ in 0..5 {
        h.step();
    }
    assert!(!released(&h) && stroking(&h));
    assert_ne!(snapshot(&h), before, "途中までは塗れている");
    h.key_press(Key::Escape);
    h.step();
    h.run();
    assert!(!stroking(&h));
    assert!(h.state().state.view3d.input.surface.is_none());
    assert_eq!(h.state().state.doc.undo_count(), 0, "Undo に積まない");
    assert!(snapshot(&h) == before, "塗った分も戻る");
}

/// 離したあと残りを塗っている間（確定待ち）の Esc は、線を捨てない: 残りを塗って確定してから、Esc の普段の意味へ進む。
#[test]
fn escape_after_the_release_keeps_the_line() {
    let (expected, _) = whole(&[&first_line()]);
    let (mut h, rect) = view(Duration::ZERO);
    leave_leftover(&mut h, rect, &first_line());
    h.key_press(Key::Escape);
    h.step();
    h.run();
    assert!(!stroking(&h), "確定した");
    assert!(h.state().state.view3d.input.surface.is_none());
    assert_eq!(h.state().state.doc.undo_count(), 1, "線は残る");
    assert!(snapshot(&h) == expected, "残りを塗ってから確定");
}

/// ウィンドウのフォーカスを失うのは、離したのと同じ: その場で全部は塗らず、残りを時間の枠で塗り続けて確定する。
#[test]
fn losing_focus_is_a_release_and_the_rest_is_painted_in_boxed_frames() {
    let (expected, _) = whole(&[&first_line()]);
    let (mut h, rect) = view(Duration::ZERO);
    let points = path(&h, rect, &first_line());
    press_with(&h, points[0], Modifiers::NONE);
    h.step();
    move_all(&mut h, &points[1..]);
    h.event(Event::WindowFocused(false));
    h.step();
    assert!(released(&h), "離したことになる");
    assert!(stroking(&h), "残りがあるあいだは描いている最中");
    assert!(queued(&h).unwrap() > 20);
    let frames = drain(&mut h);
    assert!(frames > 20, "何フレームにも分けて塗った ({frames})");
    assert_eq!(h.state().state.doc.undo_count(), 1);
    assert!(message(&h).is_empty(), "{}", message(&h));
    assert!(snapshot(&h) == expected, "全部を一度に塗ったときと同じ画素");
}

/// 残りを塗っている途中で 3D のタブが隠れたら、塗り進められないので、その場で塗り終えて確定する（取り残さない）。
#[test]
fn hiding_the_3d_view_during_the_leftover_finishes_the_stroke() {
    let (expected, _) = whole(&[&first_line()]);
    let (mut h, rect) = view(Duration::ZERO);
    let points = path(&h, rect, &first_line());
    press_with(&h, points[0], Modifiers::NONE);
    h.step();
    move_all(&mut h, &points[1..]);
    release_with(&h, *points.last().unwrap(), Modifiers::NONE);
    h.step();
    assert!(released(&h) && stroking(&h));
    click_tab(&mut h, yolu_app::Tab::Canvas);
    h.run();
    assert!(!stroking(&h), "隠れたら確定する");
    assert_eq!(h.state().state.doc.undo_count(), 1);
    assert!(snapshot(&h) == expected, "全部塗ってから確定");
}

/// 1 回のフレームに塗る時間が普通の長さ（既定）でも、同じ線は同じ画素の 1 本になる（フレームの区切りによらない）。
#[test]
fn the_default_time_box_paints_the_same_stroke() {
    let (expected, _) = whole(&[&first_line()]);
    let (mut h, rect) = cube_view();
    {
        let state = &mut h.state_mut().state;
        state.brush.radius = 3.0;
        state.brush.spacing = 0.2;
        assert!(state.view3d.input.paint_budget.is_none(), "既定の枠");
    }
    let points = path(&h, rect, &first_line());
    press_with(&h, points[0], Modifiers::NONE);
    h.step();
    move_all(&mut h, &points[1..]);
    release_with(&h, *points.last().unwrap(), Modifiers::NONE);
    h.step();
    drain(&mut h);
    assert_eq!(h.state().state.doc.undo_count(), 1);
    assert!(snapshot(&h) == expected);
}

/// 離した直後の Ctrl+Z は断られず、3D の線が 1 回で戻る（先に残りを塗って確定してから、取り消しを通す）。やり直しで同じ線が戻る。
#[test]
fn ctrl_z_right_after_the_release_undoes_the_whole_line_in_one_step() {
    let (expected, _) = whole(&[&first_line()]);
    let (mut h, rect) = view(Duration::ZERO);
    let before = snapshot(&h);
    leave_leftover(&mut h, rect, &first_line());
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.step();
    h.run();
    assert!(!stroking(&h));
    assert_eq!(h.state().state.doc.undo_count(), 0, "1 回で戻った");
    assert!(snapshot(&h) == before, "線が消えた");
    assert!(h.state().state.doc.can_redo());
    assert!(
        !message(&h).contains("描いている"),
        "断りの文は出ない: {}",
        message(&h)
    );
    h.state_mut().state.apply(yolu_app::state::Action::Redo);
    assert!(snapshot(&h) == expected, "やり直しで、全部を塗った線が戻る");
}

/// 保存・モードの切り替え・外からの操作（MCP/CLI の入り口）も、先に確定してから通す（断らない）。
#[test]
fn saving_switching_the_mode_and_outside_operations_first_finish_the_leftover() {
    for what in ["save", "mode", "outside"] {
        let (mut h, rect) = view(Duration::ZERO);
        leave_leftover(&mut h, rect, &first_line());
        let state = &mut h.state_mut().state;
        match what {
            "save" => state.apply(yolu_app::state::Action::SaveProject),
            "mode" => {
                assert!(
                    state.set_mode(yolu_app::mode::EditorMode::Edit),
                    "切り替わる"
                );
                assert_eq!(state.mode, yolu_app::mode::EditorMode::Edit);
            }
            _ => {
                let _ = yolu_app::ops_host::AppHost::new(state);
            }
        }
        assert!(!stroking(&h), "{what}: 先に確定した");
        assert_eq!(h.state().state.doc.undo_count(), 1, "{what}");
        assert!(
            !message(&h).contains("描いている"),
            "{what}: 断りの文は出ない: {}",
            message(&h)
        );
    }
}

/// 3D ビューのホイールも、確定してから受ける（視点が寄る）。
#[test]
fn the_wheel_right_after_the_release_finishes_the_line_and_zooms() {
    let (mut h, rect) = view(Duration::ZERO);
    leave_leftover(&mut h, rect, &first_line());
    let distance = h.state().state.view3d.camera.distance;
    h.event(Event::PointerMoved(rect.center()));
    h.step();
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, 1.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    h.step();
    h.run();
    assert!(!stroking(&h), "確定した");
    assert_eq!(h.state().state.doc.undo_count(), 1);
    assert_ne!(
        h.state().state.view3d.camera.distance,
        distance,
        "ホイールが効いた"
    );
}

/// ツールのキーの「押している間だけ」も、離した直後から効く（先に確定してから、一時の切り替えを始める）。
#[test]
fn a_hold_tool_key_right_after_the_release_switches_temporarily() {
    use yolu_app::state::{Action, Tool};
    use yolu_app::toolkeys::ToolKeyMode;
    let (mut h, rect) = view(Duration::ZERO);
    h.state_mut()
        .state
        .apply(Action::ToolKeyMode("tool.eraser", ToolKeyMode::Hold));
    h.run();
    let previous = h.state().state.tool;
    leave_leftover(&mut h, rect, &first_line());
    h.event(Event::Key {
        key: Key::E,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    assert!(!stroking(&h), "確定した");
    assert_eq!(h.state().state.doc.undo_count(), 1);
    assert_eq!(h.state().state.tool, Tool::Eraser, "消しゴムになった");
    assert!(h.state().state.temp_tool.is_active(), "一時の切り替え");
    h.event(Event::Key {
        key: Key::E,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.run();
    assert_eq!(h.state().state.tool, previous, "離すと戻る");
}

/// 3D ビューの外（パネル・メニューのある所）を押しても、描いている最中の判定で黙って効かなくならないよう、押しの前に確定する
/// （パネルの部品は `is_stroking` を見て何もしないものがある）。ポインタを動かすだけでは確定しない。
#[test]
fn a_press_anywhere_in_the_window_finishes_the_leftover_but_only_moving_does_not() {
    let (mut h, rect) = view(Duration::ZERO);
    leave_leftover(&mut h, rect, &first_line());
    let outside = egui::pos2(6.0, 6.0);
    assert!(!rect.contains(outside), "3D ビューの外");
    h.event(Event::PointerMoved(outside));
    h.step();
    assert!(released(&h) && stroking(&h), "動かすだけでは確定しない");
    h.event(Event::PointerButton {
        pos: outside,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    assert!(!stroking(&h), "押したら、先に確定した");
    assert_eq!(h.state().state.doc.undo_count(), 1);
}

/// 閉じる流れは、保存していない変更を聞く前に確定する。
#[test]
fn closing_finishes_the_leftover_before_asking_about_unsaved_changes() {
    use std::cell::Cell;
    use std::rc::Rc;
    let (mut h, rect) = view(Duration::ZERO);
    leave_leftover(&mut h, rect, &first_line());
    let seen: Rc<Cell<Option<(bool, usize)>>> = Rc::default();
    let record = seen.clone();
    h.state_mut().answer_close_question(move |state| {
        record.set(Some((state.is_stroking(), state.doc.undo_count())));
        false
    });
    h.state_mut().state.quit = true;
    h.step();
    assert_eq!(seen.get(), Some((false, 1)), "聞くときには確定している");
}

/// 2D のキャンバスと 3D ビューを並べているとき、3D の離した後の残りを塗っている間にキャンバスを押しても、押しは受けられる
/// （先に 3D の線を確定してから、2D の線を引く。Undo は 2 回分）。
#[test]
fn a_press_on_the_2d_canvas_finishes_the_3d_leftover_and_draws() {
    use egui_dock::{DockState, NodeIndex};
    let (mut h, _) = view(Duration::ZERO);
    let mut dock = DockState::new(vec![yolu_app::Tab::Canvas]);
    dock.main_surface_mut()
        .split_right(NodeIndex::root(), 0.5, vec![yolu_app::Tab::View3d]);
    h.state_mut().dock = dock;
    h.run();
    let rect = h.state().view3d_rect().expect("3D も出ている");
    leave_leftover(&mut h, rect, &first_line());
    let at = canvas_rect(&h).center();
    drag(&mut h, &[at, at + egui::vec2(40.0, 0.0)]);
    assert!(!stroking(&h));
    assert_eq!(h.state().state.doc.undo_count(), 2, "3D の線と 2D の線");
    assert!(
        canvas_pixel(&h, at + egui::vec2(20.0, 0.0))[3] > 0,
        "2D の線が描かれた"
    );
}

/// その場で確定させる呼び手（GPU を失ったとき・閉じるときなど）は、残りを全部塗ってから確定する。塗り残しを作らない。
#[test]
fn finishing_in_place_paints_the_whole_rest_before_committing() {
    let (expected, _) = whole(&[&first_line()]);
    let (mut h, rect) = view(Duration::ZERO);
    let points = path(&h, rect, &first_line());
    press_with(&h, points[0], Modifiers::NONE);
    h.step();
    move_all(&mut h, &points[1..]);
    release_with(&h, *points.last().unwrap(), Modifiers::NONE);
    h.step();
    assert!(released(&h) && stroking(&h));
    assert!(queued(&h).unwrap() > 20);
    yolu_app::canvas::finish_stroke(&mut h.state_mut().state, false);
    assert!(!stroking(&h), "その場で確定した");
    assert_eq!(h.state().state.doc.undo_count(), 1);
    assert!(snapshot(&h) == expected, "残りを全部塗ってから確定");
    assert!(message(&h).is_empty(), "{}", message(&h));
}

/// ペンを離すのも、離したのと同じ: 1 フレームに全部の点が来ても塗るのは 1 ダブだけ、離したあとは描いている最中のまま時間の枠で塗り続け、
/// 塗り終えたら確定する。画素は、時間の枠が十分に長い（1 フレームで全部塗る）ペンの線と同じ。
#[test]
fn lifting_the_pen_is_a_release_and_the_rest_is_painted_in_boxed_frames() {
    let draw = |budget: Duration| -> (Vec<u8>, usize) {
        let (mut h, rect) = view(budget);
        let points = path(&h, rect, &first_line());
        let sample = |at: Pos2, contact: bool, time_ms: u32| PenSample {
            pos: [at.x, at.y],
            pressure: if contact { 1.0 } else { 0.0 },
            tilt: Tilt::default(),
            rotation: None,
            contact,
            eraser: false,
            barrel: false,
            pointer_id: 5,
            time_ms,
        };
        h.state().pen().push(sample(points[0], true, 0));
        h.step();
        for (i, p) in points[1..].iter().enumerate() {
            h.state().pen().push(sample(*p, true, 10 * (i as u32 + 1)));
        }
        h.step();
        h.state()
            .pen()
            .push(sample(*points.last().unwrap(), false, 100));
        h.step();
        let mut frames = 0;
        if budget == Duration::ZERO {
            assert!(released(&h) && stroking(&h), "ペンを離した");
            assert!(queued(&h).unwrap() > 20, "残りを持ち越している");
            assert_eq!(h.state().state.doc.undo_count(), 0, "まだ確定していない");
            frames = drain(&mut h);
        }
        h.run();
        assert!(!stroking(&h));
        assert_eq!(h.state().state.doc.undo_count(), 1, "1 本の線");
        assert!(message(&h).is_empty(), "{}", message(&h));
        (snapshot(&h), frames)
    };
    let (whole, _) = draw(Duration::from_secs(600));
    let (boxed, frames) = draw(Duration::ZERO);
    assert!(frames > 20, "何フレームにも分けて塗った ({frames})");
    assert!(boxed == whole, "全部を一度に塗ったときと同じ画素");
}
