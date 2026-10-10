//! 3D ビューの選択ペン・選択消し（egui_kittest。3D ビューのドラッグの入力の道で）: 面に塗って選択範囲へ追加・削除すること（正面から見た板で、2D の選択ペンと同じ量）・
//! Shift（追加）・Ctrl（削除）・ペンの消しゴムの端・筆圧・取り消し 1 回・Esc とツールの切り替えとフォーカスの喪失と補った離し・確定待ち・ツールのキーの一時の
//! 切り替え・読むだけのセットと予算の断り。クイックマスクの 3D のブラシは `parity_symmetry.rs` が見る。
use crate::common::*;
use crate::view3d_brush::{cube_view, press_with, release_with, screen_of};
use crate::view3d_select::{
    amount, at, message, pen_sample, plate_view, selected_count, selection, side_by_side, st, tool,
    H, N,
};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};
use yolu_app::pen::PenSample;
use yolu_app::selection::{SelAction, SelEdit};
use yolu_app::state::Action;
use yolu_app::state::Tool;
use yolu_app::toolkeys::ToolKeyMode;
use yolu_core::glam::Vec3;
use yolu_core::{SelectionCombine, SelectionMask};

/// 細いブラシ（`radius` は 2D のキャンバスでの半径で、文書の画素）。3D の半径はモデルの箱の対角線に対する割合（Unity 版と同じ式）なので、箱が 2 × 2 の
/// 板では、同じ大きさになるよう 1/√2 倍にする（`three_d` が true のとき）。
pub(crate) fn brush(h: &mut H, radius: f32, three_d: bool) {
    let radius = if three_d {
        radius * std::f32::consts::FRAC_1_SQRT_2
    } else {
        radius
    };
    let s = &mut h.state_mut().state;
    s.brush.radius = radius;
    s.brush.hardness = 0.6;
    s.brush.opacity = 1.0;
    s.brush.pressure_size = false;
    s.brush.pressure_opacity = false;
}

/// 板の上の文書の点の列を、画面の点に。
pub(crate) fn on_plate(h: &H, rect: Rect, path: &[(f64, f64)]) -> Vec<Pos2> {
    path.iter().map(|&c| at(h, rect, c)).collect()
}

/// 押す修飾つきで、点をなぞる。
fn drag_mods(h: &mut H, points: &[Pos2], m: Modifiers) {
    if m != Modifiers::NONE {
        h.event(Event::ModifiersChanged(m));
    }
    press_with(h, points[0], m);
    h.step();
    for p in &points[1..] {
        move_to(h, *p);
        h.step();
    }
    release_with(h, *points.last().unwrap(), m);
    h.step();
    if m != Modifiers::NONE {
        h.event(Event::ModifiersChanged(Modifiers::NONE));
    }
    h.run();
}

/// 板の上の文書の点の列をなぞる。
fn stroke(h: &mut H, rect: Rect, path: &[(f64, f64)], m: Modifiers) {
    let pts = on_plate(h, rect, path);
    drag_mods(h, &pts, m);
}

const LINE: [(f64, f64); 4] = [(12.0, 30.0), (24.0, 34.0), (40.0, 30.0), (52.0, 34.0)];
const CROSS: [(f64, f64); 3] = [(32.0, 18.0), (34.0, 32.0), (32.0, 48.0)];

/// 2D のキャンバスの文書の点の、画面の点。
fn canvas_at(h: &H, (x, y): (f64, f64)) -> Pos2 {
    let r = canvas_rect(h);
    let s = st(h);
    s.view
        .view(r, s.doc.width(), s.doc.height())
        .to_screen(x, y)
}

/// 2D のキャンバスで選択ペン（か選択消し）を同じ文書の点の列で引いた選択範囲。
fn two_d(strokes: &[(&[(f64, f64)], Modifiers)], radius: f32) -> Option<SelectionMask> {
    let mut h = app(1000.0, 640.0, N);
    tool(&mut h, Tool::SelectPen);
    brush(&mut h, radius, false);
    for (path, m) in strokes {
        let pts: Vec<Pos2> = path.iter().map(|&c| canvas_at(&h, c)).collect();
        drag_mods(&mut h, &pts, *m);
    }
    selection(&h)
}

fn diff_stats(a: &SelectionMask, b: &SelectionMask) -> (u32, f64, usize, usize) {
    let (mut worst, mut sum, mut covered_a, mut covered_b) = (0u32, 0u64, 0usize, 0usize);
    for y in 0..N {
        for x in 0..N {
            let (p, q) = (a.amount(x, y) as i32, b.amount(x, y) as i32);
            covered_a += usize::from(p > 0);
            covered_b += usize::from(q > 0);
            let d = (p - q).unsigned_abs();
            worst = worst.max(d);
            sum += u64::from(d);
        }
    }
    let covered = covered_a.max(covered_b).max(1);
    (worst, sum as f64 / covered as f64, covered_a, covered_b)
}

/// 3D の選択範囲が、2D の同じ形の選択ペンの結果と、画素ごとの量でほぼ同じ（縁の画素の量が 1 割ほど違いうる）。
fn assert_like_2d(got: &SelectionMask, want: &SelectionMask, what: &str) {
    let (worst, mean, a, b) = diff_stats(got, want);
    assert!(a > 100 && b > 100, "{what}: {a} {b}");
    assert!(
        (a as f64 - b as f64).abs() <= 0.05 * b as f64,
        "{what}: 選んだ画素の数が違う {a} {b}"
    );
    assert!(
        worst <= 40 && mean <= 6.0,
        "{what}: 最大 {worst} 平均 {mean}"
    );
}

#[test]
fn the_selection_pen_in_3d_adds_like_the_2d_pen_seen_straight_on_and_is_one_undo() {
    for ortho in [true, false] {
        let (mut h, rect) = plate_view(ortho);
        tool(&mut h, Tool::SelectPen);
        brush(&mut h, 4.0, true);
        let pts = on_plate(&h, rect, &LINE);
        drag_mods(&mut h, &pts, Modifiers::NONE);
        assert_eq!(st(&h).doc.undo_count(), 1, "{ortho} {}", message(&h));
        let got = selection(&h).unwrap_or_else(|| panic!("{ortho}: 選べなかった: {}", message(&h)));
        let want = two_d(&[(&LINE, Modifiers::NONE)], 4.0).expect("2D で選べた");
        assert_like_2d(&got, &want, &format!("ortho={ortho}"));
        // 取り消し 1 回で選択なしへ、やり直しで戻る
        key(&h, Key::Z, Modifiers::COMMAND);
        h.run();
        assert!(selection(&h).is_none(), "{ortho}");
        key(&h, Key::Y, Modifiers::COMMAND);
        h.run();
        assert!(selection(&h) == Some(got), "{ortho}");
        assert!(!st(&h).is_stroking());
        assert!(st(&h).view3d.input.cover.is_none() && st(&h).sel.pen.is_none());
        assert!(st(&h).doc.undo_count() == 1);
    }
}

#[test]
fn ctrl_the_option_and_the_pen_eraser_end_make_it_a_selection_eraser_and_shift_adds_again() {
    // 追加してから、Ctrl を押して押した 2 本目で削る（2D と同じ量）
    let (mut h, rect) = plate_view(true);
    tool(&mut h, Tool::SelectPen);
    brush(&mut h, 4.0, true);
    stroke(&mut h, rect, &LINE, Modifiers::NONE);
    stroke(&mut h, rect, &CROSS, Modifiers::COMMAND);
    assert_eq!(st(&h).doc.undo_count(), 2, "{}", message(&h));
    let want = two_d(
        &[(&LINE, Modifiers::NONE), (&CROSS, Modifiers::COMMAND)],
        4.0,
    )
    .unwrap();
    assert_like_2d(&selection(&h).unwrap(), &want, "Ctrl で削る");
    // 削った所の真ん中は空く（追加した線の上を横切る所）
    assert_eq!(amount(&h, 33, 32), 0, "削った");
    // オプション（選択消し）が入っていれば、押すだけで削る。Shift を押して押せば追加する
    let (mut h, rect) = plate_view(true);
    tool(&mut h, Tool::SelectPen);
    brush(&mut h, 4.0, true);
    stroke(&mut h, rect, &LINE, Modifiers::NONE);
    h.state_mut().state.sel.pen_erase = true;
    stroke(&mut h, rect, &CROSS, Modifiers::NONE);
    assert_eq!(amount(&h, 33, 32), 0, "選択消しのオプションで削る");
    stroke(&mut h, rect, &CROSS, Modifiers::SHIFT);
    assert_eq!(amount(&h, 33, 24), 255, "Shift を押せば追加する");
    assert_eq!(st(&h).doc.undo_count(), 3, "{}", message(&h));
    // ペンの消しゴムの端: 削る
    let (mut h, rect) = plate_view(true);
    tool(&mut h, Tool::SelectPen);
    brush(&mut h, 4.0, true);
    stroke(&mut h, rect, &LINE, Modifiers::NONE);
    let cross = on_plate(&h, rect, &CROSS);
    for (i, p) in cross.iter().enumerate() {
        h.state().pen().push(PenSample {
            eraser: true,
            ..pen_sample(*p, true, i as u32 * 10)
        });
        h.step();
    }
    h.state().pen().push(PenSample {
        eraser: true,
        ..pen_sample(cross[2], false, 40)
    });
    h.step();
    h.run();
    assert_eq!(amount(&h, 33, 32), 0, "消しゴムの端で削る: {}", message(&h));
    assert_eq!(st(&h).doc.undo_count(), 2);
}

#[test]
fn the_pen_pressure_scales_the_amount_and_the_3d_symmetry_is_not_used() {
    let (mut h, rect) = plate_view(true);
    tool(&mut h, Tool::SelectPen);
    brush(&mut h, 4.0, true);
    h.state_mut().state.brush.pressure_opacity = true;
    // 3D の対称は選択ペンに使わない（2D の選択ペンと同じ）
    crate::common::rulers::mirror_3d(
        &mut h.state_mut().state,
        yolu_core::glam::DVec3::ZERO,
        yolu_core::glam::DVec3::X,
    );
    let pts = on_plate(&h, rect, &LINE);
    for (i, p) in pts.iter().enumerate() {
        h.state().pen().push(pen_sample(*p, true, i as u32 * 10)); // 筆圧 0.6
        h.step();
    }
    h.state().pen().push(pen_sample(pts[3], false, 50));
    h.step();
    h.run();
    let m = selection(&h).expect("選べた");
    let top = (0..N)
        .flat_map(|y| (0..N).map(move |x| (x, y)))
        .map(|(x, y)| m.amount(x, y))
        .max()
        .unwrap();
    assert!((148..=158).contains(&top), "筆圧 0.6 の量は約 153: {top}");
    let total = selected_count(&h, N);
    let (mut h2, rect2) = plate_view(true);
    tool(&mut h2, Tool::SelectPen);
    brush(&mut h2, 4.0, true);
    h2.state_mut().state.brush.pressure_opacity = true;
    let pts2 = on_plate(&h2, rect2, &LINE);
    for (i, p) in pts2.iter().enumerate() {
        h2.state().pen().push(pen_sample(*p, true, i as u32 * 10));
        h2.step();
    }
    h2.state().pen().push(pen_sample(pts2[3], false, 50));
    h2.step();
    h2.run();
    assert_eq!(
        total,
        selected_count(&h2, N),
        "対称を入れても入れなくても同じ"
    );
}

// ───────── 取りやめ・取り残し・確定待ち・ツールのキー ─────────

#[test]
fn escape_and_switching_tools_drop_the_stroke_and_focus_loss_and_a_lost_release_end_it_as_the_3d_brush_does(
) {
    for way in 0..4 {
        let (mut h, rect) = plate_view(true);
        tool(&mut h, Tool::SelectPen);
        brush(&mut h, 4.0, true);
        let pts = on_plate(&h, rect, &LINE);
        press(&h, pts[0], PointerButton::Primary);
        h.step();
        for p in &pts[1..3] {
            move_to(&h, *p);
            h.step();
        }
        assert!(st(&h).view3d.input.cover.is_some(), "{way}: 描いている");
        match way {
            0 => key(&h, Key::Escape, Modifiers::NONE),
            1 => h.state_mut().state.apply(Action::SelectTool(Tool::Brush)),
            2 => h.event(Event::WindowFocused(false)),
            _ => {
                // OS に押しを奪われて補った離し（ペンの押しは別の道）: 下のペンの試験で見る。ここは離すだけ
                release(&h, pts[2], PointerButton::Primary);
            }
        }
        h.step();
        h.run();
        assert!(!st(&h).is_stroking(), "{way}");
        assert!(
            st(&h).view3d.input.cover.is_none() && st(&h).sel.pen.is_none(),
            "{way}"
        );
        match way {
            // 捨てる: 何も選ばず、取り消しに積まない
            0 | 1 => {
                assert!(selection(&h).is_none(), "{way}");
                assert_eq!(st(&h).doc.undo_count(), 0, "{way}");
            }
            // そこまでを確定する（3D のブラシと同じ）
            _ => {
                assert!(selection(&h).is_some(), "{way}: そこまでを選ぶ");
                assert_eq!(st(&h).doc.undo_count(), 1, "{way}");
            }
        }
        release(&h, pts[3], PointerButton::Primary);
        h.run();
    }
}

#[test]
fn a_pen_press_taken_by_the_os_ends_the_selection_pen_stroke_at_the_last_point() {
    let (mut h, rect) = plate_view(true);
    tool(&mut h, Tool::SelectPen);
    brush(&mut h, 4.0, true);
    let pts = on_plate(&h, rect, &LINE);
    for (i, p) in pts.iter().enumerate() {
        h.state().pen().push(pen_sample(*p, true, i as u32 * 10));
        h.step();
    }
    assert!(st(&h).view3d.input.cover.is_some());
    h.state().pen().push_lost(pen_sample(pts[3], false, 50));
    h.step();
    h.run();
    assert!(st(&h).view3d.input.cover.is_none() && st(&h).sel.pen.is_none());
    assert!(st(&h).view3d.input.pen_press.is_none());
    assert!(selection(&h).is_some(), "最後の位置までを選ぶ");
    assert_eq!(st(&h).doc.undo_count(), 1);
    // 次の押しは新しいストローク
    stroke(&mut h, rect, &CROSS, Modifiers::NONE);
    assert_eq!(st(&h).doc.undo_count(), 2);
}

#[test]
fn a_selection_pen_press_during_the_leftover_of_a_released_brush_stroke_finishes_it_first() {
    let (mut h, rect) = cube_view();
    {
        let s = &mut h.state_mut().state;
        s.brush.radius = 3.0;
        s.brush.spacing = 0.2;
        s.view3d.input.paint_budget = Some(std::time::Duration::ZERO);
        s.color.set_main([1.0, 0.0, 0.0, 1.0]);
    }
    let (a, b) = (
        screen_of(&h, rect, Vec3::new(-0.4, -0.35, -0.5)),
        screen_of(&h, rect, Vec3::new(0.4, 0.35, -0.5)),
    );
    press_with(&h, a, Modifiers::NONE);
    h.step();
    h.input_mut().events.push(Event::PointerMoved(b));
    h.step();
    release_with(&h, b, Modifiers::NONE);
    h.step();
    assert!(
        st(&h).view3d.input.released && st(&h).is_stroking(),
        "確定待ち"
    );
    // ツールは直に替えて、押しが先に確定させる
    h.state_mut().state.tool = Tool::SelectPen;
    let (c, d) = (
        screen_of(&h, rect, Vec3::new(-0.2, -0.2, -0.5)),
        screen_of(&h, rect, Vec3::new(0.2, 0.2, -0.5)),
    );
    drag_mods(&mut h, &[c, (c + d.to_vec2()) * 0.5, d], Modifiers::NONE);
    assert!(!st(&h).is_stroking(), "{}", message(&h));
    assert_eq!(
        st(&h).doc.undo_count(),
        2,
        "線の確定と選択範囲、それぞれ 1 回: {}",
        message(&h)
    );
    assert!(selection(&h).is_some());
}

#[test]
fn releasing_the_hold_key_in_the_middle_of_a_3d_selection_pen_stroke_finishes_the_stroke_first() {
    let (mut h, rect) = plate_view(true);
    brush(&mut h, 4.0, true);
    h.state_mut()
        .state
        .apply(Action::ToolKeyMode("tool.select_pen", ToolKeyMode::Hold));
    h.run();
    let key_event = |h: &H, pressed: bool| {
        h.event(Event::Key {
            key: Key::S,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        })
    };
    key_event(&h, true);
    h.step();
    h.step();
    assert_eq!(st(&h).tool, Tool::SelectPen);
    let pts = on_plate(&h, rect, &LINE);
    press(&h, pts[0], PointerButton::Primary);
    h.step();
    move_to(&h, pts[1]);
    h.step();
    assert!(st(&h).view3d.input.cover.is_some());
    key_event(&h, false);
    for _ in 0..20 {
        h.step();
    }
    assert_eq!(st(&h).tool, Tool::SelectPen, "描いている途中は戻さない");
    for p in &pts[2..] {
        move_to(&h, *p);
        h.step();
    }
    release(&h, pts[3], PointerButton::Primary);
    h.step();
    h.run();
    assert!(selection(&h).is_some(), "{}", message(&h));
    assert_eq!(st(&h).doc.undo_count(), 1);
    assert_eq!(st(&h).tool, Tool::Brush, "描き終えたら戻る");
}

#[test]
fn a_read_only_set_and_a_small_work_budget_refuse_and_change_nothing() {
    let (mut h, rect) = plate_view(true);
    tool(&mut h, Tool::SelectPen);
    brush(&mut h, 4.0, true);
    h.state_mut().state.sets.get_mut(0).unwrap().read_only = Some("読めない中身".into());
    let pts = on_plate(&h, rect, &LINE);
    drag_mods(&mut h, &pts, Modifiers::NONE);
    assert!(message(&h).contains("読めない中身"), "{}", message(&h));
    assert!(selection(&h).is_none());
    assert!(st(&h).sel.pen.is_none() && st(&h).view3d.input.cover.is_none());
    // 作業の予算（被覆のタイルの分）に入らない
    let (mut h, rect) = plate_view(true);
    tool(&mut h, Tool::SelectPen);
    brush(&mut h, 4.0, true);
    h.state_mut().state.sel.pen_budget = 1000;
    let pts = on_plate(&h, rect, &LINE);
    drag_mods(&mut h, &pts, Modifiers::NONE);
    assert!(!message(&h).is_empty());
    assert!(selection(&h).is_none(), "{}", message(&h));
    assert_eq!(st(&h).doc.undo_count(), 0);
    assert!(!st(&h).is_stroking());
}

#[test]
fn the_quick_mask_brush_still_works_beside_the_selection_pen() {
    // クイックマスクが入っていても、選択ペンのツールは選択ペンとして塗る（クイックマスクのブラシではない）
    let (mut h, rect) = plate_view(true);
    brush(&mut h, 4.0, true);
    h.state_mut()
        .state
        .apply(Action::Sel(yolu_app::selection::SelAction::Ui(
            yolu_app::selection::SelUiOp::QuickMask(Some(true)),
        )));
    tool(&mut h, Tool::SelectPen);
    stroke(&mut h, rect, &LINE, Modifiers::NONE);
    assert!(selection(&h).is_some(), "{}", message(&h));
    assert_eq!(st(&h).doc.undo_count(), 1);
    // ブラシへ替えれば、クイックマスクのブラシとして塗る（追加）
    tool(&mut h, Tool::Brush);
    stroke(&mut h, rect, &CROSS, Modifiers::NONE);
    assert_eq!(st(&h).doc.undo_count(), 2, "{}", message(&h));
    assert!(amount(&h, 33, 32) > 0);
}

// ───────── 並べた画面: 2D の重ね・Esc ─────────

pub(crate) const WINDING: [(f64, f64); 6] = [
    (12.0, 30.0),
    (20.0, 32.0),
    (28.0, 30.0),
    (36.0, 34.0),
    (44.0, 30.0),
    (52.0, 32.0),
];

pub(crate) fn select_rect(h: &mut H, (x0, y0, x1, y1): (i64, i64, i64, i64)) {
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Edit(SelEdit::Rect {
            x0,
            y0,
            x1,
            y1,
            mode: SelectionCombine::Replace,
        })));
    h.run();
}

/// 選択ペン・ブラシのストロークを、マウスかペンで引いている途中の Esc は、ストロークだけを捨て、前からある選択範囲を解除しない
/// （3D ビューが先に描かれる並びでも、キャンバスが同じ Esc で解除しない）。次の Esc は、やめるものが無いので解除する。
#[test]
fn escape_in_the_middle_of_a_3d_stroke_keeps_the_selection_that_was_there() {
    for (canvas_first, t, mouse) in [
        (false, Tool::SelectPen, false),
        (true, Tool::SelectPen, false),
        (false, Tool::SelectPen, true),
        (true, Tool::SelectPen, true),
        (false, Tool::Brush, false),
        (true, Tool::Brush, false),
        (false, Tool::Brush, true),
        (true, Tool::Brush, true),
    ] {
        let label = format!("canvas_first={canvas_first} {t:?} mouse={mouse}");
        let (mut h, rect) = side_by_side(canvas_first);
        select_rect(&mut h, (2, 2, 62, 62));
        let kept = selection(&h);
        assert!(kept.is_some());
        let steps = st(&h).doc.undo_count();
        tool(&mut h, t);
        brush(&mut h, 3.0, false);
        let pts = on_plate(&h, rect, &WINDING);
        if mouse {
            press(&h, pts[0], PointerButton::Primary);
            h.step();
            for p in &pts[1..3] {
                move_to(&h, *p);
                h.step();
            }
        } else {
            for (i, p) in pts.iter().take(3).enumerate() {
                h.state().pen().push(pen_sample(*p, true, i as u32 * 10));
                h.step();
            }
        }
        assert!(
            st(&h).view3d.input.stroke.is_some(),
            "{label}: ストロークの途中"
        );
        key(&h, Key::Escape, Modifiers::NONE);
        h.step();
        assert!(
            selection(&h) == kept,
            "{label}: 同じ Esc で前からある選択範囲まで解除しない"
        );
        assert!(st(&h).view3d.input.stroke.is_none(), "{label}: 捨てた");
        if mouse {
            release(&h, pts[2], PointerButton::Primary);
        } else {
            h.state().pen().push(pen_sample(pts[2], false, 100));
        }
        h.step();
        h.run();
        assert!(selection(&h) == kept, "{label}: 離しても変わらない");
        assert_eq!(
            st(&h).doc.undo_count(),
            steps,
            "{label}: 取り消しに積まない"
        );
        // 次の Esc: やめるものが無いので、キャンバスが選択範囲を解除する
        tool(&mut h, Tool::SelectRect);
        key(&h, Key::Escape, Modifiers::NONE);
        h.step();
        h.run();
        assert!(selection(&h).is_none(), "{label}: 次の Esc で解除");
    }
}
