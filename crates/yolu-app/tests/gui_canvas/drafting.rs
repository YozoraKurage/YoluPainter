//! 図形・定規と Shift 直線の回帰。人工の文書と入力だけを使う。
use crate::common;
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::{kittest::Queryable, Harness, SnapshotResults};
use yolu_app::{
    drafting::{self, Figure},
    engine::composite_pixel,
    lang::Lang,
    pen::PenSample,
    rulers::{active::canvas_constraint, RulerAction},
    state::{Action, AppState, StrokeSource, Tool},
    YoluApp,
};
use yolu_core::{glam::DVec2, LayerLocks, Ruler, RulerKind, SelectionMask};

fn rect() -> Rect {
    Rect::from_min_size(Pos2::ZERO, vec2(256.0, 256.0))
}
fn state() -> AppState {
    let mut s = AppState::new(64, 64);
    s.brush.radius = 1.5;
    s.brush.hardness = 1.0;
    s.brush.pressure_size = false;
    s.m2.brush.assist.curve = true;
    s.m2.brush.assist.stabilizer = 12.0;
    s
}
fn alpha(s: &AppState, x: u32, y: u32) -> u8 {
    composite_pixel(&s.doc, x, y)[3]
}

#[test]
fn every_shape_outline_and_fill_is_one_undo() {
    for figure in [Figure::Line, Figure::Rectangle, Figure::Ellipse] {
        for fill in [false, true] {
            let mut s = state();
            s.drafting.figure = figure;
            s.drafting.fill = fill;
            drafting::canvas::paint(&mut s, DVec2::splat(10.0), DVec2::splat(50.0), rect());
            assert!(s.doc.can_undo(), "{figure:?} {fill}: {}", s.message);
            if figure != Figure::Line {
                assert_eq!(alpha(&s, 30, 30) > 0, fill, "{figure:?}");
            }
            let any = (0..64).any(|y| (0..64).any(|x| alpha(&s, x, y) > 0));
            assert!(any);
            s.doc.undo().unwrap();
            assert!(!s.doc.can_undo(), "1 回だけ");
            assert!((0..64).all(|y| (0..64).all(|x| alpha(&s, x, y) == 0)));
            s.doc.redo().unwrap();
            assert!(s.doc.can_undo());
        }
    }
}

#[test]
fn shape_fill_obeys_selection_and_rounded_corners() {
    let mut s = state();
    let selection = SelectionMask::rectangle(&s.doc, 0, 0, 32, 64);
    s.doc.set_selection(Some(selection)).unwrap();
    s.doc.clear_history().unwrap();
    s.drafting.figure = Figure::Rectangle;
    s.drafting.fill = true;
    s.drafting.corner = 10.0;
    drafting::canvas::paint(&mut s, DVec2::splat(10.0), DVec2::splat(50.0), rect());
    assert_eq!(alpha(&s, 10, 10), 0);
    assert!(alpha(&s, 25, 25) > 0);
    assert_eq!(alpha(&s, 40, 25), 0);
    s.doc.undo().unwrap();
    assert!(!s.doc.can_undo());
    assert!(s.doc.selection().is_some());
}

#[test]
fn shapes_refuse_locked_layers_without_history() {
    for fill in [false, true] {
        let mut s = state();
        s.drafting.figure = Figure::Rectangle;
        s.drafting.fill = fill;
        let id = s.selected_layer.unwrap();
        s.doc.set_layer_locks(id, LayerLocks::ALL).unwrap();
        s.doc.clear_history().unwrap();
        drafting::canvas::paint(&mut s, DVec2::splat(10.0), DVec2::splat(50.0), rect());
        assert!(!s.doc.can_undo());
        assert!(!s.doc.has_active_stroke());
        assert!(!s.message.is_empty());
        assert_eq!(alpha(&s, 25, 25), 0);
    }
}

#[test]
fn shapes_refuse_non_raster_and_budget_overflow_atomically() {
    for fill in [false, true] {
        let mut s = state();
        s.drafting.figure = Figure::Rectangle;
        s.drafting.fill = fill;
        let id = s.doc.add_group("Group", None).unwrap();
        s.selected_layer = Some(id);
        s.doc.clear_history().unwrap();
        drafting::canvas::paint(&mut s, DVec2::splat(10.0), DVec2::splat(50.0), rect());
        assert!(!s.doc.can_undo());
        assert!(!s.message.is_empty());
        let mut s = state();
        s.drafting.figure = Figure::Rectangle;
        s.drafting.fill = fill;
        s.doc.set_stroke_budget_bytes(1).unwrap();
        drafting::canvas::paint(&mut s, DVec2::splat(10.0), DVec2::splat(50.0), rect());
        assert!(!s.doc.can_undo(), "{fill}");
        assert!(!s.doc.has_active_stroke());
        assert!((0..64).all(|y| (0..64).all(|x| alpha(&s, x, y) == 0)));
    }
}

#[test]
fn switching_sets_forgets_endpoint_and_each_document_keeps_its_own_rulers() {
    let mut s = state();
    let id = common::rulers::add_canvas(
        &mut s,
        RulerKind::Concentric,
        DVec2::splat(20.0),
        DVec2::splat(40.0),
    );
    let placed = common::rulers::of(&s, s.selected_layer.unwrap());
    assert_eq!(placed.len(), 1);
    s.canvas.previous_end = Some((15.0, 15.0));
    s.add_texture_set().unwrap();
    assert_eq!(s.canvas.previous_end, None);
    assert_eq!(common::rulers::total(&s), 0, "新しいセットには定規が無い");
    assert!(
        s.selected_ruler().is_none(),
        "選んでいた定規は前の文書のもの"
    );
    s.canvas.previous_end = Some((45.0, 45.0));
    s.switch_set(0).unwrap();
    assert_eq!(s.canvas.previous_end, None);
    assert_eq!(common::rulers::of(&s, s.selected_layer.unwrap()), placed);
    assert_eq!(placed[0].id, id);
}

#[test]
fn shift_and_alt_geometry_is_in_document_coordinates() {
    let a = DVec2::new(30.0, 30.0);
    for figure in [Figure::Rectangle, Figure::Ellipse] {
        let (lo, hi) = drafting::endpoints(a, DVec2::new(40.0, 50.0), figure, true, true);
        assert_eq!((lo, hi), (DVec2::splat(10.0), DVec2::splat(50.0)));
    }
    let (lo, hi) = drafting::endpoints(a, DVec2::new(48.0, 44.0), Figure::Line, true, true);
    assert!(((hi - lo).x - (hi - lo).y).abs() < 1e-8);
    assert_eq!((lo + hi) * 0.5, a);
}

#[test]
fn cancelling_a_shape_or_ruler_changes_nothing() {
    let mut s = state();
    for tool in [Tool::Shape, Tool::Ruler] {
        s.tool = tool;
        let view = s.view.view(rect(), 64, 64);
        drafting::canvas::press(
            &mut s,
            &view,
            view.to_screen(10.0, 10.0),
            StrokeSource::Mouse,
            Modifiers::NONE,
        );
        drafting::canvas::moved(
            &mut s,
            &view,
            view.to_screen(50.0, 50.0),
            StrokeSource::Mouse,
            Modifiers::SHIFT,
        );
        assert!(s.is_stroking());
        assert!(s.drafting_cancel());
        assert!(!s.is_stroking());
        assert!(!s.doc.can_undo());
        assert_eq!(common::rulers::total(&s), 0);
    }
}

#[test]
fn ruler_equations_and_perspective_direction_lock() {
    let a = DVec2::new(10.0, 10.0);
    let b = DVec2::new(30.0, 10.0);
    let start = DVec2::new(20.0, 30.0);
    let id = yolu_core::RulerId(1);
    let ruler = |kind: RulerKind, a: DVec2, b: DVec2| Ruler::canvas(id, kind, a, b);
    let constraint = |r: &Ruler, start: DVec2| canvas_constraint(r, start).expect("寄せ先");
    assert_eq!(
        constraint(&ruler(RulerKind::Line, a, b), start).project(DVec2::new(25.0, 44.0)),
        DVec2::new(25.0, 10.0)
    );
    assert_eq!(
        constraint(&ruler(RulerKind::Parallel, a, b), start).project(DVec2::new(25.0, 44.0)),
        DVec2::new(25.0, 30.0)
    );
    let p = constraint(&ruler(RulerKind::Concentric, a, b), start).project(DVec2::new(44.0, 19.0));
    assert!((p.distance(a) - start.distance(a)).abs() < 1e-8);
    let mut r = ruler(
        RulerKind::Perspective,
        DVec2::new(0.0, 20.0),
        DVec2::new(20.0, 0.0),
    );
    r.two_points = true;
    let start = DVec2::new(20.0, 20.0);
    let mut c = constraint(&r, start);
    assert_eq!(c.project(start), start);
    assert_eq!(c.project(DVec2::new(21.0, 10.0)), DVec2::new(20.0, 10.0));
    assert_eq!(c.project(DVec2::new(5.0, 9.0)), DVec2::new(20.0, 9.0));
    r.two_points = false;
    let mut c = constraint(&r, start);
    assert_eq!(c.project(DVec2::new(5.0, 9.0)), DVec2::new(5.0, 20.0));
    // 対称は寄せ先でなく写し
    assert!(canvas_constraint(&ruler(RulerKind::Symmetry, a, b), start).is_none());
}

#[test]
fn ruler_place_move_and_cancel_are_one_undo_step_each_and_cancel_changes_nothing() {
    let mut s = state();
    s.tool = Tool::Ruler;
    let view = s.view.view(rect(), 64, 64);
    let a = view.to_screen(10.0, 10.0);
    let b = view.to_screen(40.0, 40.0);
    let layer = s.selected_layer.unwrap();
    drafting::canvas::press(&mut s, &view, a, StrokeSource::Mouse, Modifiers::NONE);
    drafting::canvas::release(
        &mut s,
        &view,
        b,
        StrokeSource::Mouse,
        Modifiers::NONE,
        rect(),
    );
    let original = common::rulers::of(&s, layer);
    assert_eq!(original.len(), 1, "{}", s.message);
    assert_eq!(s.doc.undo_count(), 1, "作るのは 1 回の取り消し");
    assert!(s.modified);
    assert_eq!(
        s.rulers.selected,
        Some((layer, original[0].id)),
        "作った物を選ぶ"
    );
    // 掴んで動かしている途中でやめても、何も変わらない
    drafting::canvas::press(&mut s, &view, a, StrokeSource::Mouse, Modifiers::NONE);
    drafting::canvas::moved(&mut s, &view, b, StrokeSource::Mouse, Modifiers::NONE);
    assert!(s.is_stroking());
    s.drafting_cancel();
    assert!(!s.is_stroking());
    assert_eq!(common::rulers::of(&s, layer), original);
    assert_eq!(s.doc.undo_count(), 1);
    // 始点のつまみを動かして離す: 始点だけが動き、1 回の取り消し
    drafting::canvas::press(&mut s, &view, a, StrokeSource::Mouse, Modifiers::NONE);
    drafting::canvas::release(
        &mut s,
        &view,
        view.to_screen(15.0, 12.0),
        StrokeSource::Mouse,
        Modifiers::NONE,
        rect(),
    );
    let moved = common::rulers::of(&s, layer);
    let (ma, mb) = yolu_app::rulers::active::canvas_points(&moved[0]).unwrap();
    let (oa, ob) = yolu_app::rulers::active::canvas_points(&original[0]).unwrap();
    assert!((ma - DVec2::new(15.0, 12.0)).length() < 1e-5, "{ma:?}");
    assert_eq!(mb, ob, "終点は動かない");
    assert!((oa - DVec2::new(10.0, 10.0)).length() < 1e-5, "{oa:?}");
    assert_eq!(s.doc.undo_count(), 2);
    s.doc.undo().unwrap();
    assert_eq!(
        common::rulers::of(&s, layer),
        original,
        "取り消しで元の場所へ"
    );
}

fn mouse(h: &mut Harness<'_, YoluApp>, p: Pos2, down: bool, m: Modifiers) {
    h.input_mut().events.push(Event::ModifiersChanged(m));
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: down,
        modifiers: m,
    });
    h.step();
}
fn point(h: &Harness<'_, YoluApp>, x: f64, y: f64) -> Pos2 {
    let s = &h.state().state;
    s.view
        .view(s.ui.canvas_rect.unwrap(), s.doc.width(), s.doc.height())
        .to_screen(x, y)
}
fn pen(h: &mut Harness<'_, YoluApp>, p: Pos2, contact: bool, pressure: f32, m: Modifiers) {
    h.input_mut().events.push(Event::ModifiersChanged(m));
    h.state().pen().push(PenSample {
        pos: [p.x, p.y],
        pressure,
        tilt: Default::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 5,
        time_ms: 100,
    });
    // Windows Ink と同時に来る代替マウス入力も二重描画しない。
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: contact,
        modifiers: m,
    });
    h.step();
}

#[test]
fn mouse_and_pen_shift_click_join_in_one_undo_after_view_rotation() {
    for is_pen in [false, true] {
        let mut h = common::app(1280.0, 800.0, 64);
        h.state_mut().state.brush.radius = 1.5;
        let a = point(&h, 10.0, 10.0);
        if is_pen {
            pen(&mut h, a, true, 0.8, Modifiers::NONE);
            pen(&mut h, a, false, 0.0, Modifiers::NONE);
        } else {
            mouse(&mut h, a, true, Modifiers::NONE);
            mouse(&mut h, a, false, Modifiers::NONE);
        }
        h.state_mut().state.view.angle = 37.0;
        h.state_mut().state.view.flip = true;
        h.run();
        let b = point(&h, 50.0, 50.0);
        if is_pen {
            pen(&mut h, b, true, 0.6, Modifiers::SHIFT);
            pen(&mut h, b, false, 0.0, Modifiers::SHIFT);
        } else {
            mouse(&mut h, b, true, Modifiers::SHIFT);
            mouse(&mut h, b, false, Modifiers::SHIFT);
        }
        assert!(alpha(&h.state().state, 30, 30) > 0, "{is_pen}");
        h.state_mut().state.doc.undo().unwrap();
        assert_eq!(alpha(&h.state().state, 30, 30), 0);
        assert!(alpha(&h.state().state, 10, 10) > 0);
        h.state_mut().state.doc.undo().unwrap();
        assert!(!h.state().state.doc.can_undo());
    }
}

#[test]
fn shape_mouse_shift_alt_release_and_escape() {
    let mut h = common::app(1280.0, 800.0, 64);
    {
        let s = &mut h.state_mut().state;
        s.tool = Tool::Shape;
        s.drafting.figure = Figure::Rectangle;
        s.drafting.fill = true;
    }
    let a = point(&h, 30.0, 30.0);
    let b = point(&h, 40.0, 50.0);
    let m = Modifiers::SHIFT | Modifiers::ALT;
    // 押しの始めの Alt は表示を回す組み合わせなので、Alt は押したあとに足す（図形の「中心から」）
    mouse(&mut h, a, true, Modifiers::SHIFT);
    mouse(&mut h, b, false, m);
    assert!(alpha(&h.state().state, 15, 15) > 0);
    h.state_mut().state.doc.undo().unwrap();
    mouse(&mut h, a, true, Modifiers::NONE);
    h.input_mut().events.push(Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    mouse(&mut h, b, false, Modifiers::NONE);
    assert!(!h.state().state.doc.can_undo());
}

#[test]
fn ruler_snaps_a_stroke_started_near_it_and_ctrl_one_and_two_toggle_the_snaps() {
    let mut h = common::app(1280.0, 800.0, 64);
    {
        let s = &mut h.state_mut().state;
        s.brush.radius = 1.0;
        common::rulers::line(s, (0.0, 20.0), (64.0, 20.0));
        assert!(s.rulers.snap_ruler, "既定は入");
    }
    // 定規のすぐそば（1 画素）で押して動かすと、定規の上に描く
    let a = point(&h, 10.0, 21.0);
    let b = point(&h, 50.0, 40.0);
    mouse(&mut h, a, true, Modifiers::NONE);
    h.input_mut().events.push(Event::PointerMoved(b));
    h.step();
    mouse(&mut h, b, false, Modifiers::NONE);
    assert!(alpha(&h.state().state, 30, 20) > 0);
    assert_eq!(alpha(&h.state().state, 30, 33), 0);
    // 切ると寄らない
    h.state_mut()
        .state
        .apply(Action::Ruler(RulerAction::ToggleSnapRuler));
    assert!(!h.state().state.rulers.snap_ruler);
    for (key, special) in [(Key::Num1, false), (Key::Num2, true)] {
        h.input_mut()
            .events
            .push(Event::ModifiersChanged(Modifiers::COMMAND));
        h.input_mut().events.push(Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        });
        h.step();
        let r = &h.state().state.rulers;
        if special {
            assert!(!r.snap_special, "Ctrl+2 で切れる");
        } else {
            assert!(r.snap_ruler, "Ctrl+1 で入る");
        }
    }
}

#[test]
fn ruler_tool_options_and_overlays_in_both_languages() {
    let mut snapshots = SnapshotResults::new();
    for lang in Lang::ALL {
        let mut s = state();
        s.lang = lang;
        s.tool = Tool::Ruler;
        let mut h = common::gpu_thread::builder()
            .with_size(vec2(920.0, 300.0))
            .renderer(common::shared_gpu::renderer())
            .build_ui_state(
                |ui, s: &mut AppState| {
                    YoluApp::setup(ui.ctx());
                    let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(920.0, 40.0));
                    yolu_app::rulers::tool::options(ui, s, r, 8.0);
                    let r = Rect::from_min_max(pos2(0.0, 45.0), pos2(920.0, 300.0));
                    let view = s.view.view(r, 64, 64);
                    drafting::canvas::paint_overlay(&ui.painter_at(r), &view, s);
                },
                s,
            );
        for (i, kind) in [
            RulerKind::Line,
            RulerKind::Parallel,
            RulerKind::Concentric,
            RulerKind::Perspective,
            RulerKind::Symmetry,
        ]
        .into_iter()
        .enumerate()
        {
            let s = h.state_mut();
            let layer = s.selected_layer.unwrap();
            let old: Vec<_> = common::rulers::of(s, layer).iter().map(|r| r.id).collect();
            if !old.is_empty() {
                s.apply(Action::Ruler(RulerAction::Delete {
                    owner: layer,
                    ids: old,
                }));
            }
            s.rulers.kind = kind;
            let mut r = Ruler::canvas(
                yolu_core::RulerId(1),
                kind,
                DVec2::new(20.0, 20.0),
                DVec2::new(45.0, 40.0),
            );
            r.two_points = kind == RulerKind::Perspective;
            if kind == RulerKind::Symmetry {
                r.lines = 6;
            }
            common::rulers::add(s, r);
            h.run();
            h.snapshot(format!(
                "rulers_tool_{}_{i}",
                if lang == Lang::Ja { "ja" } else { "en" }
            ));
        }
        snapshots.extend_harness(&mut h);
    }
}

#[test]
fn pen_shape_cancel_stays_cancelled_until_lift_and_focus_loss_discards_preview() {
    let mut h = common::app(1280.0, 800.0, 64);
    h.state_mut().state.tool = Tool::Shape;
    h.state_mut().state.drafting.figure = Figure::Ellipse;
    h.state_mut().state.drafting.fill = true;
    let a = point(&h, 10.0, 10.0);
    let b = point(&h, 50.0, 50.0);
    pen(&mut h, a, true, 1.0, Modifiers::NONE);
    h.input_mut().events.push(Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    pen(&mut h, b, true, 1.0, Modifiers::NONE);
    assert!(h.state().state.drafting.drag.is_none());
    pen(&mut h, b, false, 0.0, Modifiers::NONE);
    assert!(!h.state().state.doc.can_undo());
    pen(&mut h, a, true, 1.0, Modifiers::NONE);
    pen(&mut h, b, true, 1.0, Modifiers::NONE);
    pen(&mut h, b, false, 0.0, Modifiers::NONE);
    assert!(alpha(&h.state().state, 30, 30) > 0);
    h.state_mut().state.doc.undo().unwrap();
    mouse(&mut h, a, true, Modifiers::NONE);
    h.input_mut().events.push(Event::WindowFocused(false));
    h.step();
    assert!(h.state().state.drafting.drag.is_none());
    assert!(!h.state().state.doc.can_undo());
}

/// OS に押しを奪われて補った離し（`PenInput::push_lost`）。egui には、補ったときに post されるマウスの離し（winit が egui のポインタの離しにする）も届く。
fn pen_lost(h: &mut Harness<'_, YoluApp>, p: Pos2) {
    h.state().pen().push_lost(PenSample {
        pos: [p.x, p.y],
        pressure: 0.0,
        tilt: Default::default(),
        rotation: None,
        contact: false,
        eraser: false,
        barrel: false,
        pointer_id: 5,
        time_ms: 100,
    });
    h.input_mut().events.push(Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
}

/// 図形の途中でペンの押しを OS に奪われて離しが補われたら、離した位置が不明なので取りやめる（マウスの取りこぼしと同じ。画素を変えない）。本物の離しは確定する。
#[test]
fn a_shape_pen_press_taken_by_the_os_is_cancelled_while_a_real_release_commits_it() {
    let mut h = common::app(1280.0, 800.0, 64);
    h.state_mut().state.tool = Tool::Shape;
    h.state_mut().state.drafting.figure = Figure::Ellipse;
    h.state_mut().state.drafting.fill = true;
    let a = point(&h, 10.0, 10.0);
    let b = point(&h, 50.0, 50.0);
    // 本物の離し: 確定する
    pen(&mut h, a, true, 1.0, Modifiers::NONE);
    pen(&mut h, b, true, 1.0, Modifiers::NONE);
    assert!(h.state().state.drafting.drag.is_some());
    pen(&mut h, b, false, 0.0, Modifiers::NONE);
    assert!(alpha(&h.state().state, 30, 30) > 0, "本物の離しは確定する");
    h.state_mut().state.doc.undo().unwrap();
    assert_eq!(alpha(&h.state().state, 30, 30), 0);
    // 奪われて補った離し: 取りやめる
    pen(&mut h, a, true, 1.0, Modifiers::NONE);
    pen(&mut h, b, true, 1.0, Modifiers::NONE);
    pen_lost(&mut h, b);
    let s = &h.state().state;
    assert!(s.drafting.drag.is_none(), "途中の図形は残らない");
    assert!(
        s.drafting.pen_down.is_none(),
        "このペンの押しの印も残らない"
    );
    assert_eq!(alpha(s, 30, 30), 0, "補った離しで確定しない");
    assert!(!s.doc.can_undo(), "履歴にも残さない");
    // 次の押しは新しい押しとして始まり、確定する
    pen(&mut h, a, true, 1.0, Modifiers::NONE);
    pen(&mut h, b, true, 1.0, Modifiers::NONE);
    pen(&mut h, b, false, 0.0, Modifiers::NONE);
    assert!(alpha(&h.state().state, 30, 30) > 0);
}

#[test]
fn shape_and_ruler_shortcuts_select_their_tools() {
    let mut h = common::app(1280.0, 800.0, 64);
    for (m, tool) in [
        (Modifiers::SHIFT, Tool::Ruler),
        (Modifiers::NONE, Tool::Shape),
    ] {
        h.input_mut().events.push(Event::ModifiersChanged(m));
        h.input_mut().events.push(Event::Key {
            key: Key::U,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: m,
        });
        h.step();
        assert_eq!(h.state().state.tool, tool);
        h.input_mut().events.push(Event::Key {
            key: Key::U,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: m,
        });
        h.step();
    }
}

#[test]
fn ctrl_click_still_paints_with_the_mouse_and_the_pen_still_ignores_ctrl() {
    // マウスの Ctrl+クリックは、これまでどおり描く（この変更は Shift の直線だけ）。ペンの Ctrl は従来どおり描かない
    let mut h = common::app(1280.0, 800.0, 64);
    h.state_mut().state.tool = Tool::Brush;
    let p = point(&h, 30.0, 30.0);
    pen(&mut h, p, true, 1.0, Modifiers::CTRL);
    pen(&mut h, p, false, 0.0, Modifiers::CTRL);
    assert!(!h.state().state.doc.can_undo());
    mouse(&mut h, p, true, Modifiers::CTRL);
    mouse(&mut h, p, false, Modifiers::CTRL);
    assert!(h.state().state.doc.can_undo());
    assert!(alpha(&h.state().state, 30, 30) > 0);
}

#[test]
fn shapes_and_rulers_roundtrip_through_a_saved_project() {
    let dir = std::env::temp_dir().join(format!("yolu-drafting-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    crate::common::tmp::clean_up_after_test(&dir);
    let path = dir.join("shapes.ylp");
    let mut s = state();
    s.drafting.figure = Figure::Ellipse;
    s.drafting.fill = true;
    drafting::canvas::paint(&mut s, DVec2::splat(10.0), DVec2::splat(50.0), rect());
    let expected = s.doc.composite(s.doc.bounds()).unwrap();
    let layer = s.selected_layer.unwrap();
    common::rulers::add_canvas(&mut s, RulerKind::Parallel, DVec2::ZERO, DVec2::X);
    common::rulers::symmetry_2d(&mut s, (32.0, 32.0), (3.0, 4.0), 6, false);
    let placed = common::rulers::of(&s, layer);
    assert_eq!(placed.len(), 2);
    s.canvas.previous_end = Some((50.0, 50.0));
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(path.is_file(), "{}", s.message);
    s.apply(Action::OpenProject(path));
    assert_eq!(s.doc.composite(s.doc.bounds()).unwrap(), expected);
    assert!(s.canvas.previous_end.is_none());
    // 定規は文書の値なので、開き直しても同じ（ID・種類・点・本数・印・表示の範囲）
    let layer = s.selected_layer.unwrap();
    assert_eq!(common::rulers::of(&s, layer), placed);
    assert!(
        s.rulers.selected.is_none(),
        "選びは画面の状態で、保存しない"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn shift_pen_line_uses_endpoint_pressure_instead_of_previous_pressure() {
    let mut h = common::app(1280.0, 800.0, 64);
    {
        let s = &mut h.state_mut().state;
        s.brush.radius = 5.0;
        s.brush.pressure_size = false;
        s.brush.pressure_opacity = true;
        s.brush.hardness = 1.0;
    }
    let a = point(&h, 10.0, 30.0);
    let b = point(&h, 50.0, 30.0);
    pen(&mut h, a, true, 1.0, Modifiers::NONE);
    pen(&mut h, a, false, 0.0, Modifiers::NONE);
    pen(&mut h, b, true, 0.4, Modifiers::SHIFT);
    pen(&mut h, b, false, 0.0, Modifiers::SHIFT);
    let s = &h.state().state;
    let alphas: Vec<_> = (20..=40).map(|x| alpha(s, x, 30)).collect();
    assert!(alphas.iter().all(|a| *a > 0 && *a < 200), "{alphas:?}");
    assert!(
        *alphas.iter().max().unwrap() - *alphas.iter().min().unwrap() <= 1,
        "{alphas:?}"
    );
}

#[test]
fn shift_eraser_and_effect_brush_use_the_same_atomic_stroke() {
    use yolu_app::engine::BrushEffect;
    for effect in [None, Some(BrushEffect::BLUR)] {
        let mut h = common::app(1280.0, 800.0, 64);
        {
            let s = &mut h.state_mut().state;
            s.drafting.figure = Figure::Rectangle;
            s.drafting.fill = true;
            drafting::canvas::paint(s, DVec2::new(8.0, 8.0), DVec2::new(32.0, 56.0), rect());
            s.doc.clear_history().unwrap();
            s.brush.radius = 4.0;
            s.brush.pressure_size = false;
            s.canvas.previous_end = Some((12.0, 30.0));
            if let Some(effect) = effect {
                s.tool = Tool::Brush;
                s.m2.brush.effect = effect;
            } else {
                s.tool = Tool::Eraser;
            }
        }
        let before = h
            .state()
            .state
            .doc
            .composite(h.state().state.doc.bounds())
            .unwrap();
        let b = point(&h, 50.0, 30.0);
        mouse(&mut h, b, true, Modifiers::SHIFT);
        mouse(&mut h, b, false, Modifiers::SHIFT);
        let s = &mut h.state_mut().state;
        assert_eq!(s.doc.undo_count(), 1, "{}", s.message);
        assert_ne!(s.doc.composite(s.doc.bounds()).unwrap(), before);
        s.doc.undo().unwrap();
        assert_eq!(s.doc.composite(s.doc.bounds()).unwrap(), before);
    }
}

#[test]
fn tool_strip_and_snap_buttons_fit_the_minimum_window_in_both_languages() {
    for lang in Lang::ALL {
        let mut h = common::app(960.0, 640.0, 64);
        h.state_mut().state.lang = lang;
        h.run();
        // ウィンドウの最小の大きさ（960x640）で、ツールの帯の全部のボタンが帯の中に収まる
        let (top, bottom) = (24.0 + 36.0, 640.0 - 22.0);
        for tool in Tool::ALL {
            // キーの無いツール（ゆがみ）は名前だけ
            let label = match (lang, tool.key()) {
                (_, "") => tool.name_in(lang).to_string(),
                (Lang::Ja, key) => format!("{}（{key}）", tool.name_in(lang)),
                (Lang::En, key) => format!("{} ({key})", tool.name_in(lang)),
            };
            let r = h.get_by_label(&label).rect();
            assert!(
                r.top() >= top && r.bottom() <= bottom,
                "{lang:?} {label}: {r:?}"
            );
        }
        // ブラシ・図形・定規のオプションバーも、いちばん長い並び（長方形の角の丸み・2 点パース）で、スナップの 2 つのボタンがウィンドウの中に収まる
        let snaps = [
            lang.pick("定規にスナップ（Ctrl+1）", "Snap to Ruler (Ctrl+1)"),
            lang.pick(
                "特殊定規にスナップ（Ctrl+2）",
                "Snap to Special Ruler (Ctrl+2)",
            ),
        ];
        for (tool, ruler_kind) in [
            (Tool::Brush, RulerKind::Line),
            (Tool::Shape, RulerKind::Line),
            (Tool::Ruler, RulerKind::Perspective),
            (Tool::Ruler, RulerKind::Symmetry),
        ] {
            let s = &mut h.state_mut().state;
            s.tool = tool;
            s.drafting.figure = Figure::Rectangle;
            s.rulers.kind = ruler_kind;
            s.rulers.two_points = true;
            h.run();
            for snap in snaps {
                let r = h.get_by_label(snap).rect();
                assert!(
                    r.right() <= 960.0,
                    "{lang:?} {tool:?} {ruler_kind:?}: {r:?}"
                );
            }
        }
    }
}

fn same_pixels(a: &AppState, b: &AppState) -> bool {
    a.doc.composite(a.doc.bounds()).unwrap() == b.doc.composite(b.doc.bounds()).unwrap()
}

/// 手ぶれ補正と曲線補間を入れたブラシ（`state()`）と、どちらも入れないブラシで、同じ図形を描く。
fn figure_with(smoothing: bool, figure: Figure, corner: f32) -> AppState {
    let mut s = state();
    s.m2.brush.assist.curve = smoothing;
    s.m2.brush.assist.stabilizer = if smoothing { 12.0 } else { 0.0 };
    s.drafting.figure = figure;
    s.drafting.corner = corner;
    drafting::canvas::paint(
        &mut s,
        DVec2::new(10.0, 12.0),
        DVec2::new(50.0, 40.0),
        rect(),
    );
    s
}

#[test]
fn figure_outlines_keep_corners_and_edge_ends_despite_stabilizer_and_curve() {
    type Points = &'static [(u32, u32)];
    let cases: [(Figure, f32, Points, Points); 4] = [
        // 直線: 両端と中点
        (
            Figure::Line,
            0.0,
            &[(10, 12), (30, 26), (50, 40)],
            &[(10, 40), (50, 12)],
        ),
        // 長方形: 四隅と辺の中点。内側と外側は塗らない
        (
            Figure::Rectangle,
            0.0,
            &[
                (10, 12),
                (50, 12),
                (10, 40),
                (50, 40),
                (30, 12),
                (30, 40),
                (10, 26),
                (50, 26),
            ],
            &[(30, 26), (4, 4), (56, 46)],
        ),
        // 角丸: 辺の端（丸めの始まり）まで届く。角そのものは丸めて塗らない
        (
            Figure::Rectangle,
            8.0,
            &[
                (18, 12),
                (42, 12),
                (18, 40),
                (42, 40),
                (10, 20),
                (50, 20),
                (10, 32),
                (50, 32),
            ],
            &[(10, 12), (50, 40), (30, 26)],
        ),
        // 楕円: 上下左右の端
        (
            Figure::Ellipse,
            0.0,
            &[(50, 26), (10, 26), (30, 40), (30, 12)],
            &[(30, 26), (10, 12), (50, 40)],
        ),
    ];
    for (figure, corner, painted, clear) in cases {
        let smoothed = figure_with(true, figure, corner);
        let plain = figure_with(false, figure, corner);
        assert!(same_pixels(&smoothed, &plain), "{figure:?} {corner}");
        for (x, y) in painted {
            assert!(
                alpha(&smoothed, *x, *y) > 0,
                "{figure:?} {corner}: ({x},{y}) が塗られていない"
            );
        }
        for (x, y) in clear {
            assert_eq!(
                alpha(&smoothed, *x, *y),
                0,
                "{figure:?} {corner}: ({x},{y})"
            );
        }
    }
}

#[test]
fn figure_strokes_ignore_speed_controls_whatever_the_vertex_spacing() {
    // 頂点ごとの時刻で速さを決めない。速さの制御（サイズ・不透明度・流量）を最大に入れても、制御の無いブラシと同じ画素になる
    for (figure, corner) in [
        (Figure::Line, 0.0),
        (Figure::Rectangle, 0.0),
        (Figure::Rectangle, 8.0),
        (Figure::Ellipse, 0.0),
    ] {
        let plain = figure_with(false, figure, corner);
        let mut fast = state();
        let c = &mut fast.m2.brush.controls;
        (c.speed_size, c.speed_opacity, c.speed_flow, c.speed_max) = (true, true, true, 100.0);
        fast.drafting.figure = figure;
        fast.drafting.corner = corner;
        drafting::canvas::paint(
            &mut fast,
            DVec2::new(10.0, 12.0),
            DVec2::new(50.0, 40.0),
            rect(),
        );
        assert!(
            alpha(&fast, 30, 12) > 0 || alpha(&fast, 30, 26) > 0,
            "{figure:?}"
        );
        assert!(same_pixels(&fast, &plain), "{figure:?} {corner}");
    }
}

/// 入力の経路（マウス・ペン）で、図形のツールでなくブラシの 1 ストロークを (a → b) と動かして離す。
fn stroke(h: &mut Harness<'_, YoluApp>, is_pen: bool, a: Pos2, b: Pos2, m: Modifiers) {
    if is_pen {
        pen(h, a, true, 1.0, m);
        pen(h, b, true, 1.0, m);
        pen(h, b, false, 0.0, m);
    } else {
        mouse(h, a, true, m);
        h.input_mut().events.push(Event::PointerMoved(b));
        h.step();
        mouse(h, b, false, m);
    }
}

/// 選んでいるレイヤーに定規を置いた画面（スナップは 2 つとも既定の入）。
fn app_with_ruler(ruler: Ruler) -> Harness<'static, YoluApp> {
    let mut h = common::app(1280.0, 800.0, 64);
    let s = &mut h.state_mut().state;
    s.brush.radius = 1.5;
    s.brush.hardness = 1.0;
    s.brush.pressure_size = false;
    common::rulers::add(s, ruler);
    h
}

fn ruler_of(kind: RulerKind, a: (f64, f64), b: (f64, f64), two_points: bool) -> Ruler {
    let mut r = Ruler::canvas(
        yolu_core::RulerId(1),
        kind,
        DVec2::new(a.0, a.1),
        DVec2::new(b.0, b.1),
    );
    r.two_points = two_points;
    r
}

#[test]
fn shift_line_end_is_painted_exactly_with_stabilizer_and_curve_on() {
    for is_pen in [false, true] {
        let build = |smoothing: bool| {
            let mut h = common::app(1280.0, 800.0, 64);
            {
                let s = &mut h.state_mut().state;
                s.brush.radius = 1.5;
                s.brush.hardness = 1.0;
                s.brush.pressure_size = false;
                s.m2.brush.assist.curve = smoothing;
                s.m2.brush.assist.stabilizer = if smoothing { 12.0 } else { 0.0 };
                // 速さの制御を入れても、直線の 2 点は同じ時刻なので細く・薄くならない
                if smoothing {
                    let c = &mut s.m2.brush.controls;
                    (c.speed_size, c.speed_opacity, c.speed_max) = (true, true, 100.0);
                }
            }
            let a = point(&h, 10.0, 12.0);
            let b = point(&h, 50.0, 40.0);
            stroke(&mut h, is_pen, a, a, Modifiers::NONE);
            stroke(&mut h, is_pen, b, b, Modifiers::SHIFT);
            h
        };
        let smoothed = build(true);
        let plain = build(false);
        let (s, p) = (&smoothed.state().state, &plain.state().state);
        assert!(same_pixels(s, p), "{is_pen}");
        for (x, y) in [(10, 12), (30, 26), (50, 40)] {
            assert!(alpha(s, x, y) > 0, "{is_pen}: ({x},{y})");
        }
        assert_eq!(alpha(s, 10, 40), 0);
        assert_eq!(alpha(s, 50, 12), 0);
    }
}

#[test]
fn ruler_wins_over_shift_for_click_lines_and_direction_locks() {
    for is_pen in [false, true] {
        // 前の終点からの Shift クリックは、定規の上へ寄せた 2 点を結ぶ（押した点は定規のそば）
        let mut h = app_with_ruler(ruler_of(RulerKind::Line, (0.0, 20.0), (64.0, 20.0), false));
        h.state_mut().state.rulers.snap_ruler = false;
        let a = point(&h, 10.0, 40.0);
        stroke(&mut h, is_pen, a, a, Modifiers::NONE);
        h.state_mut().state.rulers.snap_ruler = true;
        let b = point(&h, 50.0, 21.0);
        stroke(&mut h, is_pen, b, b, Modifiers::SHIFT);
        let s = &h.state().state;
        assert!(alpha(s, 30, 20) > 0, "{is_pen}");
        assert!(alpha(s, 50, 20) > 0, "{is_pen}");
        assert_eq!(
            alpha(s, 30, 42),
            0,
            "寄せずに結んだ線の上は塗らない {is_pen}"
        );
        assert_eq!(alpha(s, 30, 32), 0, "{is_pen}");

        // Shift で始めたドラッグの水平への固定より、定規の寄せ先（斜め）が優先される（押した点は定規のそば）
        let mut h = app_with_ruler(ruler_of(RulerKind::Line, (0.0, 0.0), (64.0, 64.0), false));
        let (a, b) = (point(&h, 30.0, 31.0), point(&h, 50.0, 32.0));
        stroke(&mut h, is_pen, a, b, Modifiers::SHIFT);
        let s = &h.state().state;
        assert!(alpha(s, 35, 35) > 0, "{is_pen}");
        assert!(alpha(s, 40, 40) > 0, "{is_pen}");
        assert_eq!(
            alpha(s, 45, 31),
            0,
            "水平に固定した線の上は塗らない {is_pen}"
        );
    }
}

#[test]
fn a_straight_ruler_pulls_only_within_26_points_of_where_the_stroke_starts() {
    // 画面で 26 点の内で押すと寄り、外で押すと寄らない（定規が何本あっても、近い所で描き始めた線だけが寄る）
    let h = app_with_ruler(ruler_of(RulerKind::Line, (0.0, 20.0), (64.0, 20.0), false));
    let per_canvas_px = point(&h, 1.0, 0.0).distance(point(&h, 0.0, 0.0)) as f64;
    let near = 25.0 / per_canvas_px;
    let far = 27.0 / per_canvas_px;
    for (offset, snapped) in [(near, true), (-near, true), (far, false), (-far, false)] {
        let mut h = app_with_ruler(ruler_of(RulerKind::Line, (0.0, 20.0), (64.0, 20.0), false));
        let (a, b) = (
            point(&h, 10.0, 20.0 + offset),
            point(&h, 50.0, 20.0 + offset + 12.0),
        );
        stroke(&mut h, false, a, b, Modifiers::NONE);
        let s = &h.state().state;
        assert_eq!(alpha(s, 30, 20) > 0, snapped, "{offset}");
        // 寄らなかった線は押した点から動かした先まで素直に描く
        assert_eq!(
            alpha(s, 30, (20.0 + offset + 6.0).round() as u32) > 0,
            !snapped,
            "{offset}"
        );
    }
}

#[test]
fn only_the_nearest_of_several_straight_rulers_pulls_the_stroke() {
    let mut h = common::app(1280.0, 800.0, 64);
    {
        let s = &mut h.state_mut().state;
        s.brush.radius = 1.0;
        s.brush.hardness = 1.0;
        s.brush.pressure_size = false;
        common::rulers::line(s, (0.0, 20.0), (64.0, 20.0));
        common::rulers::line(s, (0.0, 22.0), (64.0, 22.0));
    }
    // y = 22 に近い所で押す: 22 の線へ寄る
    let (a, b) = (point(&h, 10.0, 22.4), point(&h, 50.0, 40.0));
    stroke(&mut h, false, a, b, Modifiers::NONE);
    let s = &h.state().state;
    assert!(alpha(s, 30, 22) > 0);
    assert_eq!(alpha(s, 30, 20), 0);
}

#[test]
fn a_special_ruler_pulls_whatever_the_distance_and_off_switches_it_off() {
    // 平行線は、押した点を通る定規の向きの線へ（定規から遠くても寄る）。「特殊定規にスナップ」を切ると寄らない
    for special in [true, false] {
        let mut h = app_with_ruler(ruler_of(
            RulerKind::Parallel,
            (0.0, 5.0),
            (64.0, 5.0),
            false,
        ));
        h.state_mut().state.rulers.snap_special = special;
        let (a, b) = (point(&h, 10.0, 40.0), point(&h, 50.0, 55.0));
        stroke(&mut h, false, a, b, Modifiers::NONE);
        let s = &h.state().state;
        assert_eq!(alpha(s, 30, 40) > 0, special, "{special}");
        assert_eq!(alpha(s, 30, 47) > 0, !special, "{special}");
    }
}

#[test]
fn every_ruler_kind_snaps_mouse_and_pen_strokes_through_the_input_path() {
    for is_pen in [false, true] {
        // 直線定規（押した点は定規のそば）と平行線
        for (kind, start_y, on_line, off_line) in [
            (RulerKind::Line, 21.0, (30, 20), (30, 33)),
            (RulerKind::Parallel, 25.0, (30, 25), (30, 33)),
        ] {
            let mut h = app_with_ruler(ruler_of(kind, (0.0, 20.0), (64.0, 20.0), false));
            let (a, b) = (point(&h, 10.0, start_y), point(&h, 50.0, 40.0));
            stroke(&mut h, is_pen, a, b, Modifiers::NONE);
            let s = &h.state().state;
            assert!(alpha(s, on_line.0, on_line.1) > 0, "{kind:?} {is_pen}");
            assert_eq!(alpha(s, off_line.0, off_line.1), 0, "{kind:?} {is_pen}");
        }
        // 同心円: 押した点を通る円の上（中心 (32, 32)、押した点の半径 15）
        let mut h = app_with_ruler(ruler_of(
            RulerKind::Concentric,
            (32.0, 32.0),
            (40.0, 32.0),
            false,
        ));
        let (a, b) = (point(&h, 47.0, 32.0), point(&h, 32.0, 62.0));
        stroke(&mut h, is_pen, a, b, Modifiers::NONE);
        let s = &h.state().state;
        assert!(alpha(s, 47, 32) > 0, "{is_pen}");
        assert!(alpha(s, 32, 47) > 0, "円の上へ寄せた終点 {is_pen}");
        assert_eq!(alpha(s, 32, 61), 0, "動かした先は塗らない {is_pen}");
        // パース 1 点: 押した点と消失点 (0, 20) を結ぶ線の上
        let mut h = app_with_ruler(ruler_of(
            RulerKind::Perspective,
            (0.0, 20.0),
            (60.0, 0.0),
            false,
        ));
        let (a, b) = (point(&h, 20.0, 40.0), point(&h, 35.0, 50.0));
        stroke(&mut h, is_pen, a, b, Modifiers::NONE);
        let s = &h.state().state;
        assert!(alpha(s, 32, 52) > 0, "{is_pen}");
        assert_eq!(alpha(s, 35, 50), 0, "{is_pen}");
        // パース 2 点: 最初の動きに近い消失点 (60, 0) の側を選ぶ
        let mut h = app_with_ruler(ruler_of(
            RulerKind::Perspective,
            (0.0, 20.0),
            (60.0, 0.0),
            true,
        ));
        let (a, b) = (point(&h, 20.0, 40.0), point(&h, 35.0, 30.0));
        stroke(&mut h, is_pen, a, b, Modifiers::NONE);
        let s = &h.state().state;
        assert!(alpha(s, 32, 27) > 0, "{is_pen}");
        assert_eq!(alpha(s, 35, 30), 0, "{is_pen}");
    }
}

#[test]
fn a_shape_drag_with_the_pen_is_dropped_when_the_canvas_is_hidden() {
    use yolu_app::Tab;
    let mut h = common::app(1280.0, 800.0, 64);
    {
        let s = &mut h.state_mut().state;
        s.tool = Tool::Shape;
        s.drafting.figure = Figure::Rectangle;
        s.drafting.fill = true;
    }
    let (a, b) = (point(&h, 10.0, 10.0), point(&h, 50.0, 50.0));
    pen(&mut h, a, true, 1.0, Modifiers::NONE);
    pen(&mut h, b, true, 1.0, Modifiers::NONE);
    assert!(h.state().state.drafting.drag.is_some());
    // 触れたまま別のタブへ（キャンバスは隠れ、ペンの離れを受け取れない）
    common::click_tab(&mut h, Tab::View3d);
    h.run();
    assert!(!h.state().state.ui.canvas_visible);
    assert!(
        h.state().state.drafting.drag.is_none(),
        "図形の途中を捨てた"
    );
    assert!(
        h.state().state.drafting.pen_down.is_none(),
        "ペンの印も捨てた"
    );
    pen(&mut h, b, false, 0.0, Modifiers::NONE);
    common::click_tab(&mut h, Tab::Canvas);
    h.run();
    assert!(h.state().state.ui.canvas_visible);
    assert!(
        !h.state().state.doc.can_undo(),
        "隠れているあいだに何も塗らない"
    );
    // 戻したあとの次の押しは、新しい押しとして描ける
    let (a, b) = (point(&h, 10.0, 10.0), point(&h, 50.0, 50.0));
    pen(&mut h, a, true, 1.0, Modifiers::NONE);
    pen(&mut h, b, true, 1.0, Modifiers::NONE);
    pen(&mut h, b, false, 0.0, Modifiers::NONE);
    assert!(alpha(&h.state().state, 30, 30) > 0);
}

#[test]
fn a_ruler_without_length_is_not_placed_or_collapsed() {
    let mut s = state();
    s.tool = Tool::Ruler;
    let view = s.view.view(rect(), 64, 64);
    let a = view.to_screen(10.0, 10.0);
    // 動かさずにクリックしただけでは置かない（向きが X に落ちて、意図しない水平の寄せ先になる）
    for kind in [RulerKind::Line, RulerKind::Parallel, RulerKind::Symmetry] {
        s.rulers.kind = kind;
        drafting::canvas::press(&mut s, &view, a, StrokeSource::Mouse, Modifiers::NONE);
        drafting::canvas::release(
            &mut s,
            &view,
            a,
            StrokeSource::Mouse,
            Modifiers::NONE,
            rect(),
        );
        assert_eq!(common::rulers::total(&s), 0, "{kind:?}");
    }
    assert!(!s.doc.can_undo());
    // 置いた定規の端点を、もう一方の端点に重ねても潰さない（元の定規のまま。取り消しの段も増えない）
    s.rulers.kind = RulerKind::Line;
    drafting::canvas::press(&mut s, &view, a, StrokeSource::Mouse, Modifiers::NONE);
    drafting::canvas::release(
        &mut s,
        &view,
        view.to_screen(40.0, 10.0),
        StrokeSource::Mouse,
        Modifiers::NONE,
        rect(),
    );
    let layer = s.selected_layer.unwrap();
    let placed = common::rulers::of(&s, layer);
    assert_eq!(placed.len(), 1);
    drafting::canvas::press(
        &mut s,
        &view,
        view.to_screen(40.0, 10.0),
        StrokeSource::Mouse,
        Modifiers::NONE,
    );
    drafting::canvas::release(
        &mut s,
        &view,
        a,
        StrokeSource::Mouse,
        Modifiers::NONE,
        rect(),
    );
    assert_eq!(common::rulers::of(&s, layer), placed);
    assert_eq!(s.doc.undo_count(), 1);
}
