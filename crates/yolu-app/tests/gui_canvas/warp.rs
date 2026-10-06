use crate::common;
use egui::{pos2, vec2, Modifiers, Rect};
use yolu_app::{
    canvas::view::CanvasView,
    lang::Lang,
    state::{Action, AppState, StrokeSource, Tool},
    transform::{advanced::Kind, canvas},
};
use yolu_core::{Channel, LiquifyMode, Rgba8};
fn state() -> AppState {
    let mut s = AppState::new(32, 32);
    let id = s.selected_layer.unwrap();
    for y in 4..28 {
        for x in 4..28 {
            s.doc
                .set_pixel(id, x, y, Rgba8::new((x * 7) as u8, (y * 8) as u8, 50, 255))
                .unwrap();
        }
    }
    s.doc.clear_history().unwrap();
    s.apply(Action::SelectTool(Tool::Move));
    s
}
fn view(s: &AppState) -> CanvasView {
    s.view
        .view(Rect::from_min_size(pos2(0., 0.), vec2(512., 512.)), 32, 32)
}
fn pixels(s: &AppState) -> Vec<Rgba8> {
    let id = s.selected_layer.unwrap();
    (0..s.doc.height())
        .flat_map(|y| {
            (0..s.doc.width()).map(move |x| {
                s.doc
                    .layer(id)
                    .unwrap()
                    .pixel(Channel::Color, x, y)
                    .unwrap()
            })
        })
        .collect()
}
#[test]
fn headless_mouse_free_perspective_mesh_commit_cancel_undo() {
    for kind in [Kind::Free, Kind::Perspective, Kind::Mesh] {
        let mut s = state();
        s.transform.advanced.kind = kind;
        let v = view(&s);
        let before = pixels(&s);
        let a = v.to_screen(4., 4.);
        let b = v.to_screen(6., 5.);
        canvas::press(&mut s, &v, a, StrokeSource::Mouse, Modifiers::NONE);
        canvas::moved(&mut s, &v, b, StrokeSource::Mouse, false);
        assert_eq!(pixels(&s), before);
        canvas::cancel(&mut s);
        assert_eq!(pixels(&s), before);
        assert_eq!(s.doc.undo_count(), 0);
        canvas::press(&mut s, &v, a, StrokeSource::Mouse, Modifiers::NONE);
        canvas::release(&mut s, &v, b, StrokeSource::Mouse, false);
        assert_ne!(pixels(&s), before);
        assert_eq!(s.doc.undo_count(), 1);
        s.apply(Action::Undo);
        assert_eq!(pixels(&s), before);
    }
}
#[test]
fn headless_control_corner_and_pen_enter_do_not_double_commit() {
    let mut s = state();
    let v = view(&s);
    let a = v.to_screen(4., 4.);
    let b = v.to_screen(7., 6.);
    canvas::pen_sample(&mut s, &v, a, 7, true, Modifiers::CTRL);
    assert!(s.transform.advanced.draft.is_some());
    canvas::pen_sample(&mut s, &v, b, 7, true, Modifiers::CTRL);
    canvas::commit(&mut s);
    assert_eq!(s.doc.undo_count(), 1);
    canvas::pen_sample(&mut s, &v, b, 7, false, Modifiers::CTRL);
    assert_eq!(s.doc.undo_count(), 1);
}
#[test]
fn headless_focus_tool_and_revision_changes_cancel_safely() {
    let mut s = state();
    s.transform.advanced.kind = Kind::Free;
    let v = view(&s);
    let a = v.to_screen(4., 4.);
    let b = v.to_screen(7., 6.);
    let before = pixels(&s);
    canvas::press(&mut s, &v, a, StrokeSource::Mouse, Modifiers::NONE);
    canvas::moved(&mut s, &v, b, StrokeSource::Mouse, false);
    s.transform_cancel_drag();
    assert_eq!(pixels(&s), before);
    assert!(s.transform.advanced.draft.is_none());
    canvas::press(&mut s, &v, a, StrokeSource::Mouse, Modifiers::NONE);
    canvas::moved(&mut s, &v, b, StrokeSource::Mouse, false);
    s.apply(Action::SelectTool(Tool::Brush));
    assert!(s.transform.advanced.draft.is_none());
    assert_eq!(pixels(&s), before);
}
#[test]
fn headless_liquify_mouse_pen_restore_and_undo() {
    let mut s = state();
    s.apply(Action::SelectTool(Tool::Liquify));
    s.transform.advanced.diameter = 20.;
    let v = view(&s);
    let a = v.to_screen(14., 14.);
    let b = v.to_screen(19., 14.);
    let before = pixels(&s);
    canvas::press(&mut s, &v, a, StrokeSource::Mouse, Modifiers::NONE);
    canvas::release(&mut s, &v, b, StrokeSource::Mouse, false);
    assert_ne!(pixels(&s), before);
    assert_eq!(s.doc.undo_count(), 1);
    let after = pixels(&s);
    s.transform.advanced.mode = LiquifyMode::Restore;
    s.transform.advanced.strength = 1.;
    s.transform.advanced.diameter = 2048.;
    canvas::pen_sample(&mut s, &v, a, 3, true, Modifiers::NONE);
    canvas::pen_sample(&mut s, &v, b, 3, false, Modifiers::NONE);
    assert_ne!(pixels(&s), after);
    assert_eq!(s.doc.undo_count(), 2);
    s.apply(Action::Undo);
    assert_eq!(pixels(&s), after);
    s.apply(Action::Undo);
    assert_eq!(pixels(&s), before);
}
#[test]
fn snapshot_warp_grid_and_options_both_languages() {
    let mut results = egui_kittest::SnapshotResults::new();
    for lang in Lang::ALL {
        let mut s = state();
        s.lang = lang;
        s.transform.advanced.kind = Kind::Mesh;
        let mut ready = false;
        let mut h = common::gpu_thread::builder()
            .with_size(vec2(800., 500.))
            .renderer(common::shared_gpu::renderer())
            .build_ui_state(
                move |ui, s| {
                    if !ready {
                        yolu_app::YoluApp::setup(ui.ctx());
                        ready = true;
                        ui.ctx().request_repaint();
                        return;
                    }
                    let bar = Rect::from_min_size(pos2(0., 0.), vec2(800., 40.));
                    yolu_app::transform::props::options(ui, s, bar, 8.);
                    let v =
                        s.view
                            .view(Rect::from_min_size(pos2(0., 50.), vec2(500., 440.)), 32, 32);
                    if s.transform.advanced.draft.is_none() {
                        canvas::press(
                            s,
                            &v,
                            v.to_screen(16., 16.),
                            StrokeSource::Mouse,
                            Modifiers::NONE,
                        );
                        canvas::moved(s, &v, v.to_screen(18., 19.), StrokeSource::Mouse, false);
                    }
                    canvas::paint_overlay(ui.painter(), &v, s);
                },
                s,
            );
        h.run();
        h.snapshot(if lang == Lang::Ja {
            "warp_grid_ja"
        } else {
            "warp_grid_en"
        });
        results.extend_harness(&mut h);
    }
}

#[test]
fn canvas_events_enter_escape_and_liquify_pen_are_routed() {
    let mut h = common::app(1000., 700., 32);
    h.state_mut().state = state();
    h.state_mut().state.transform.advanced.kind = Kind::Free;
    h.run();
    let rect = common::canvas_rect(&h);
    let v = h.state().state.view.view(rect, 32, 32);
    let a = v.to_screen(4., 4.);
    let b = v.to_screen(7., 6.);
    let before = pixels(&h.state().state);
    common::press(&h, a, egui::PointerButton::Primary);
    h.step();
    common::move_to(&h, b);
    h.step();
    assert!(h.state().state.transform.advanced.draft.is_some());
    common::key(&h, egui::Key::Escape, Modifiers::NONE);
    h.step();
    common::release(&h, b, egui::PointerButton::Primary);
    h.run();
    assert_eq!(pixels(&h.state().state), before);
    assert_eq!(h.state().state.doc.undo_count(), 0);
    common::press(&h, a, egui::PointerButton::Primary);
    h.step();
    common::move_to(&h, b);
    h.step();
    common::key(&h, egui::Key::Enter, Modifiers::NONE);
    h.step();
    assert_eq!(h.state().state.doc.undo_count(), 1);
    common::release(&h, b, egui::PointerButton::Primary);
    h.run();
    assert_eq!(h.state().state.doc.undo_count(), 1);
    h.state_mut().state.apply(Action::Undo);
    h.state_mut().state.apply(Action::SelectTool(Tool::Liquify));
    h.state_mut().state.transform.advanced.diameter = 20.;
    h.run();
    for (pos, contact) in [
        (v.to_screen(14., 14.), true),
        (v.to_screen(19., 14.), true),
        (v.to_screen(19., 14.), false),
    ] {
        h.state().pen().push(yolu_app::pen::PenSample {
            pos: [pos.x, pos.y],
            pressure: 0.5,
            tilt: Default::default(),
            rotation: None,
            contact,
            eraser: false,
            barrel: false,
            pointer_id: 9,
            time_ms: 0,
        });
        h.step();
    }
    h.run();
    assert_eq!(h.state().state.doc.undo_count(), 1);
    assert_ne!(pixels(&h.state().state), before);
    assert!(h.state().state.canvas.stroke.is_none());
}
#[test]
fn headless_warped_pixels_save_and_open_without_new_format() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/warp-project-tests")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("warp.ylp");
    let mut s = state();
    s.transform.advanced.kind = Kind::Mesh;
    let v = view(&s);
    canvas::press(
        &mut s,
        &v,
        v.to_screen(16., 16.),
        StrokeSource::Mouse,
        Modifiers::NONE,
    );
    canvas::release(
        &mut s,
        &v,
        v.to_screen(18., 19.),
        StrokeSource::Mouse,
        false,
    );
    let expected = pixels(&s);
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(path.exists(), "{}", s.message);
    let mut opened = AppState::new(32, 32);
    opened.apply(Action::OpenProject(path));
    assert_eq!(pixels(&opened), expected);
}

#[test]
fn headless_perspective_mirrors_the_other_corner_of_the_same_edge() {
    let mut s = state();
    s.transform.advanced.kind = Kind::Perspective;
    let v = view(&s);
    canvas::press(
        &mut s,
        &v,
        v.to_screen(4., 4.),
        StrokeSource::Mouse,
        Modifiers::NONE,
    );
    canvas::moved(&mut s, &v, v.to_screen(6., 5.), StrokeSource::Mouse, false);
    let yolu_core::Warp::Projective(h) = &s.transform.advanced.draft.as_ref().unwrap().warp else {
        panic!("射影")
    };
    for (from, to) in [
        ((4., 4.), (6., 5.)),
        ((28., 4.), (26., 5.)),
        ((28., 28.), (28., 28.)),
        ((4., 28.), (4., 28.)),
    ] {
        let p = h.apply(from.0, from.1);
        assert!((p.0 - to.0).abs() < 1e-6 && (p.1 - to.1).abs() < 1e-6);
    }
}

#[test]
fn review_liquify_restore_after_all_visible_pixels_leave_canvas() {
    let mut s = state();
    s.apply(Action::SelectTool(Tool::Liquify));
    s.transform.advanced.diameter = 2048.;
    s.transform.advanced.strength = 1.;
    let v = view(&s);
    let before = pixels(&s);
    let start = v.to_screen(16.5, 16.5);
    let outside = v.to_screen(100.5, 16.5);
    canvas::press(&mut s, &v, start, StrokeSource::Mouse, Modifiers::NONE);
    canvas::release(&mut s, &v, outside, StrokeSource::Mouse, false);
    assert!(s.transform_bounds_cached().is_none());
    assert_eq!(s.doc.undo_count(), 1);
    let empty = pixels(&s);
    assert!(empty.iter().all(|p| p.a == 0));
    s.transform.advanced.mode = LiquifyMode::Restore;
    canvas::press(&mut s, &v, start, StrokeSource::Mouse, Modifiers::NONE);
    assert!(
        s.transform.advanced.draft.is_some(),
        "可視画素がなくても復元を開始できる: {}",
        s.message
    );
    canvas::release(&mut s, &v, start, StrokeSource::Mouse, false);
    assert!(s.transform_bounds_cached().is_some());
    let restored = pixels(&s);
    assert_eq!(restored[16 * 32 + 16], before[16 * 32 + 16]);
    assert_eq!(s.doc.undo_count(), 2);
    s.apply(Action::Undo);
    assert_eq!(pixels(&s), empty);
    s.apply(Action::Undo);
    assert_eq!(pixels(&s), before);
}

fn input_limit_stroke(extra: bool, lang: Lang) -> (AppState, Vec<Rgba8>, u64, u64) {
    let mut s = AppState::new(2, 2);
    s.lang = lang;
    let id = s.selected_layer.unwrap();
    s.doc
        .set_pixel(id, 0, 0, Rgba8::new(80, 90, 100, 255))
        .unwrap();
    s.doc.clear_history().unwrap();
    s.apply(Action::SelectTool(Tool::Liquify));
    s.transform.advanced.diameter = 20.;
    s.transform.advanced.strength = 0.;
    let v = s
        .view
        .view(Rect::from_min_size(pos2(0., 0.), vec2(200., 200.)), 2, 2);
    let start = v.to_screen(0.5, 0.5);
    let other = v.to_screen(1.5, 0.5);
    let before = pixels(&s);
    let revision = s.doc.revision();
    let serial = s.doc.change_serial();
    canvas::press(&mut s, &v, start, StrokeSource::Mouse, Modifiers::NONE);
    for i in 1..=16_384 {
        if i == 16_384 {
            s.transform.advanced.strength = 1.;
        }
        canvas::moved(
            &mut s,
            &v,
            if i % 2 == 0 { start } else { other },
            StrokeSource::Mouse,
            false,
        );
    }
    let yolu_core::Warp::Liquify(dabs) = &s.transform.advanced.draft.as_ref().unwrap().warp else {
        panic!("ゆがみ")
    };
    assert_eq!(dabs.len(), 16_384);
    if extra {
        canvas::moved(&mut s, &v, other, StrokeSource::Mouse, false);
    }
    canvas::release(&mut s, &v, start, StrokeSource::Mouse, false);
    (s, before, revision, serial)
}
#[test]
fn review_liquify_input_at_limit_commits_one_undo() {
    let (mut s, before, _, _) = input_limit_stroke(false, Lang::Ja);
    assert_eq!(s.doc.undo_count(), 1);
    assert_ne!(pixels(&s), before);
    s.apply(Action::Undo);
    assert_eq!(pixels(&s), before);
}
#[test]
fn review_liquify_input_over_limit_refuses_whole_stroke() {
    for lang in Lang::ALL {
        let (s, before, revision, serial) = input_limit_stroke(true, lang);
        assert_eq!(
            s.doc.undo_count(),
            0,
            "上限超過では途中までのUndoを作らない"
        );
        assert_eq!(pixels(&s), before);
        assert_eq!(s.doc.revision(), revision);
        assert_eq!(s.doc.change_serial(), serial);
        assert_eq!(
            s.message,
            lang.pick("ゆがみの入力数が上限です", "Liquify input limit reached")
        );
        assert!(s.transform.advanced.draft.is_none());
        assert!(s.transform.drag.is_none());
    }
}
#[test]
fn review_liquify_limit_includes_previously_committed_session_inputs() {
    let (mut s, _, _, _) = input_limit_stroke(false, Lang::En);
    let before = pixels(&s);
    let revision = s.doc.revision();
    let serial = s.doc.change_serial();
    let v = s
        .view
        .view(Rect::from_min_size(pos2(0., 0.), vec2(200., 200.)), 2, 2);
    // 戻すの押下だけでも描点を1個追加する。開始時の上限検査も通ること。
    s.transform.advanced.mode = LiquifyMode::Restore;
    let at = v.to_screen(0.5, 0.5);
    canvas::press(&mut s, &v, at, StrokeSource::Mouse, Modifiers::NONE);
    canvas::release(&mut s, &v, at, StrokeSource::Mouse, false);
    assert_eq!(pixels(&s), before);
    assert_eq!(s.doc.undo_count(), 1);
    assert_eq!(s.doc.revision(), revision);
    assert_eq!(s.doc.change_serial(), serial);
    assert_eq!(s.message, "Liquify input limit reached");
}
