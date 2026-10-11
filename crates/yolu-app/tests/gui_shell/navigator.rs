use crate::common;
use egui::{pos2, vec2, Event, Modifiers, PointerButton, Rect};
use egui_kittest::{kittest::Queryable, Harness};
use yolu_app::{app::Tab, lang::Lang, state::AppState, YoluApp};

fn harness(lang: Lang) -> Harness<'static, AppState> {
    let mut app = AppState::new(256, 128);
    app.lang = lang;
    let layer = app.doc.layers()[0].id();
    for y in 70..110 {
        for x in 30..100 {
            app.doc
                .set_pixel(layer, x, y, yolu_app::engine::Rgba8::new(220, 90, 40, 255))
                .unwrap();
        }
    }
    app.ui.canvas_rect = Some(Rect::from_min_size(pos2(400.0, 20.0), vec2(600.0, 400.0)));
    app.view.zoom = 3.0;
    app.view.angle = 30.0;
    let mut ready = false;
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(300.0, 350.0))
        .renderer(common::shared_gpu::renderer())
        .build_ui_state(
            move |ui, app| {
                if !ready {
                    YoluApp::setup(ui.ctx());
                    ready = true;
                    ui.ctx().request_repaint();
                    return;
                }
                yolu_app::navigator::show(ui, app);
            },
            app,
        );
    h.run();
    h
}

fn pointer(h: &mut Harness<'_, AppState>, p: egui::Pos2, down: bool) {
    h.event(Event::PointerMoved(p));
    h.event(Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: down,
        modifiers: Modifiers::NONE,
    });
    h.run();
}

#[test]
fn navigator_buttons_and_languages() {
    for lang in [Lang::Ja, Lang::En] {
        let mut h = harness(lang);
        let revision = h.state().doc.revision();
        let undo = h.state().doc.undo_count();
        let redo = h.state().doc.redo_count();
        h.get_by_label(lang.pick("左右反転", "Flip horizontally"))
            .click();
        h.run();
        assert!(h.state().view.flip);
        h.get_by_label(lang.pick("回転を戻す", "Reset rotation"))
            .click();
        h.run();
        assert_eq!(h.state().view.angle, 0.0);
        h.get_by_label("100%").click();
        h.run();
        let app = h.state();
        assert!(
            (app.view
                .view(app.ui.canvas_rect.unwrap(), 256, 128)
                .pixel_size()
                - 1.0)
                .abs()
                < 1e-5
        );
        h.get_by_label(lang.pick("全体を表示", "Fit canvas"))
            .click();
        h.run();
        assert_eq!(h.state().view.zoom, 1.0);
        assert_eq!(h.state().view.pan, egui::Vec2::ZERO);
        assert_eq!(h.state().doc.revision(), revision);
        assert_eq!(h.state().doc.undo_count(), undo);
        assert_eq!(h.state().doc.redo_count(), redo);
        assert_eq!(
            Tab::Navigator.title_in(lang),
            lang.pick("ナビゲーター", "Navigator")
        );
    }
}

#[test]
fn navigator_click_and_drag_move_the_view() {
    let mut h = harness(Lang::En);
    let area = h.get_by_label("Navigator").rect();
    let mini = yolu_app::canvas::view::ViewState::default().view(area.shrink(4.0), 256, 128);
    let p = mini.to_screen(20.0, 20.0);
    pointer(&mut h, p, true);
    pointer(&mut h, p, false);
    let app = h.state();
    let viewport = app.ui.canvas_rect.unwrap();
    let c = app
        .view
        .view(viewport, 256, 128)
        .to_canvas(viewport.center());
    assert!(
        (c.0 - 20.0).abs() < 0.001 && (c.1 - 20.0).abs() < 0.001,
        "{c:?}"
    );
    let start = mini.to_screen(25.0, 23.0);
    pointer(&mut h, start, true);
    h.event(Event::PointerMoved(start + vec2(20.0, 10.0)));
    h.run();
    pointer(&mut h, start + vec2(20.0, 10.0), false);
    let c2 = h
        .state()
        .view
        .view(viewport, 256, 128)
        .to_canvas(viewport.center());
    let delta = mini.to_canvas(start + vec2(20.0, 10.0));
    assert!((c2.0 - c.0 - (delta.0 - 25.0)).abs() < 0.001);
    assert!((c2.1 - c.1 - (delta.1 - 23.0)).abs() < 0.001);
}

#[test]
fn navigator_panel_snapshot_japanese() {
    let mut h = harness(Lang::Ja);
    h.state_mut().view.flip = true;
    h.run();
    h.snapshot("navigator_ja");
}

#[test]
fn navigator_panel_snapshot_english() {
    let mut h = harness(Lang::En);
    h.state_mut().view.flip = true;
    h.run();
    h.snapshot("navigator_en");
}

#[test]
fn navigator_thumbnail_matches_gpu_canvas_composite() {
    use yolu_app::canvas::{display::CanvasDisplay, gpu::CanvasBackend};
    use yolu_app::engine::{Channel, Rect as DocRect, Rgba8};
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(960.0, 720.0))
        .renderer(common::shared_gpu::renderer())
        .build_eframe(|cc| {
            let mut state = AppState::new(32, 32);
            let layer = state.doc.layers()[0].id();
            state
                .doc
                .set_pixel(layer, 3, 25, Rgba8::new(30, 140, 210, 130))
                .unwrap();
            let mut app =
                YoluApp::for_context(&cc.egui_ctx, state, yolu_app::pen::PenInput::detached())
                    .with_render_state(cc.wgpu_render_state.as_ref());
            app.set_canvas_backend(CanvasBackend::Gpu);
            app
        });
    // 中央は 1 つの組（`common::app` と同じ並び）
    h.state_mut().dock = common::tabbed_center_dock(960.0);
    h.run();
    let thumb = CanvasDisplay::thumbnail(&h.state().state.doc, Channel::Color).unwrap();
    let gpu = h
        .state_mut()
        .read_canvas_gpu_display(DocRect::new(0, 0, 32, 32))
        .unwrap();
    for y in 0..32 {
        for x in 0..32 {
            for c in 0..4 {
                assert!(
                    thumb.pixels[(31 - y) * 32 + x][c].abs_diff(gpu[(y * 32 + x) * 4 + c]) <= 1
                );
            }
        }
    }
    h.state_mut().set_canvas_backend(CanvasBackend::Cpu);
    h.run();
    assert_eq!(
        thumb,
        CanvasDisplay::thumbnail(&h.state().state.doc, Channel::Color).unwrap()
    );
}

#[test]
fn navigator_sliders_change_zoom_and_rotation() {
    let mut h = harness(Lang::En);
    for label in ["Zoom", "Rotation"] {
        let r = h
            .query_all_by_label(label)
            .map(|n| n.rect())
            .find(|r| r.width() > 80.0)
            .expect("スライダー");
        let before = h.state().view;
        let p = pos2(r.left() + r.width() * 0.8, r.center().y);
        pointer(&mut h, p, true);
        pointer(&mut h, p, false);
        if label == "Zoom" {
            assert_ne!(h.state().view.zoom, before.zoom);
        } else {
            assert_ne!(h.state().view.angle, before.angle);
        }
    }
}

fn type_zoom(h: &mut Harness<'_, AppState>, lang: Lang, text: &str) {
    let role = egui::accesskit::Role::SpinButton;
    h.get_by_role_and_label(role, lang.pick("拡大率", "Zoom"))
        .focus();
    h.run();
    assert!(h
        .get_by_role_and_label(role, lang.pick("拡大率", "Zoom"))
        .is_focused());
    h.event_modifiers(
        Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        },
        Modifiers::COMMAND,
    );
    h.step();
    h.event(Event::Text(text.into()));
    h.step();
    h.event(Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
}

#[test]
fn review_zoom_numbers_are_percentages_in_both_languages() {
    for lang in [Lang::Ja, Lang::En] {
        for text in ["100", "100%", "50%"] {
            let mut h = harness(lang);
            type_zoom(&mut h, lang, text);
            let app = h.state();
            let expected = if text == "50%" { 0.5 } else { 1.0 };
            let actual = app
                .view
                .view(app.ui.canvas_rect.unwrap(), 256, 128)
                .pixel_size();
            assert!(
                (actual - expected).abs() < 1e-4,
                "{lang:?} {text}: {actual}"
            );
        }
    }
}

#[test]
fn review_click_with_press_and_release_in_one_frame_moves_the_view() {
    let mut h = harness(Lang::En);
    let area = h.get_by_label("Navigator").rect();
    let mini = yolu_app::canvas::view::ViewState::default().view(area.shrink(4.0), 256, 128);
    let p = mini.to_screen(20.0, 20.0);
    h.input_mut().events.push(Event::PointerMoved(p));
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    h.run();
    let app = h.state();
    let rect = app.ui.canvas_rect.unwrap();
    let c = app.view.view(rect, 256, 128).to_canvas(rect.center());
    assert!(
        (c.0 - 20.0).abs() < 0.001 && (c.1 - 20.0).abs() < 0.001,
        "{c:?}"
    );
}

#[test]
fn review_actual_size_then_zoom_keeps_the_requested_direction() {
    use yolu_app::canvas::view::ViewState;
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0));
    for size in [8192, 8] {
        for factor in [1.25, 0.8] {
            let mut view = ViewState::default();
            view.actual_size(rect, size, size);
            let anchor = pos2(230.0, 170.0);
            let before = view.view(rect, size, size).to_canvas(anchor);
            view.zoom_to(view.zoom * factor, Some(anchor), rect);
            let after = view.view(rect, size, size);
            assert!(
                (after.pixel_size() - factor).abs() < 1e-5,
                "{size}: {}",
                after.pixel_size()
            );
            let at = after.to_canvas(anchor);
            assert!((at.0 - before.0).abs() < 0.001 && (at.1 - before.1).abs() < 0.001);
        }
    }
}

#[test]
fn review_actual_size_uses_the_shared_zoom_limits_at_document_extremes() {
    use yolu_app::canvas::view::{ViewState, MAX_ZOOM, MIN_ZOOM};
    for (size, viewport) in [(32768, 1.0), (1, 32768.0)] {
        let rect = Rect::from_min_size(egui::Pos2::ZERO, vec2(viewport, viewport));
        for factor in [1.25, 0.8] {
            let mut view = ViewState::default();
            view.actual_size(rect, size, size);
            assert!((MIN_ZOOM..=MAX_ZOOM).contains(&view.zoom));
            assert!((view.view(rect, size, size).pixel_size() - 1.0).abs() < 1e-6);
            view.zoom_to(view.zoom * factor, None, rect);
            // 極端に広い表示域では矩形端のf32丸めを含むため、倍率そのものを照合する。
            assert!((view.zoom * viewport / size as f32 - factor).abs() < 1e-6);
        }
    }
}
