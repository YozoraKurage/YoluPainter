//! UV の表示専用の入口と、日英の設定のウィンドウのショートカットの区分。
use crate::common;
use egui::{vec2, Color32, Rect};
use egui_kittest::kittest::Queryable;
use yolu_app::{
    canvas::display::CanvasDisplay,
    lang::Lang,
    state::{Action, AppState},
    YoluApp,
};

#[test]
fn uv_corner_toggles_without_starting_a_stroke_in_both_languages() {
    for lang in Lang::ALL {
        let mut app = AppState::new(64, 64);
        app.lang = lang;
        app.apply(Action::LoadDemoModel);
        app.sync_view3d();
        let mut display = CanvasDisplay::default();
        let mut ready = false;
        let mut h = common::gpu_thread::builder()
            .with_size(vec2(500.0, 400.0))
            .build_ui_state(
                move |ui, app| {
                    if !ready {
                        YoluApp::setup(ui.ctx());
                        ready = true;
                        ui.ctx().request_repaint();
                        return;
                    }
                    yolu_app::canvas::show(ui, app, &mut display, &[]);
                },
                app,
            );
        h.run();
        let label = lang.pick("UV ワイヤーフレーム", "UV Wireframe");
        h.get_by_label(label).click();
        h.run();
        assert!(!h.state().prefs.settings.uv_wireframe);
        assert!(!h.state().is_stroking());
        assert!(!h.state().can_undo());
        h.get_by_label(label).click();
        h.run();
        assert!(h.state().prefs.settings.uv_wireframe);
        assert!(!h.state().can_undo());
    }
}

#[test]
fn shortcut_category_is_localized_and_closes_without_document_changes() {
    for lang in Lang::ALL {
        let mut app = AppState::new(32, 32);
        app.lang = lang;
        app.apply(Action::ShowShortcuts);
        let mut ready = false;
        let mut h = common::gpu_thread::builder()
            .with_size(vec2(850.0, 600.0))
            .build_ui_state(
                move |ui, app| {
                    if !ready {
                        YoluApp::setup(ui.ctx());
                        ready = true;
                        ui.ctx().request_repaint();
                        return;
                    }
                    yolu_app::prefs::show(ui.ctx(), app);
                },
                app,
            );
        h.run();
        assert!(yolu_app::prefs::last_rect(&h.ctx).is_some());
        assert!(h.state().prefs.shows(yolu_app::prefs::Category::Shortcuts));
        // 閉じるは見出しの印
        h.get_by_label(lang.pick("閉じる", "Close")).click();
        h.run();
        assert!(!h.state().prefs.open);
        assert!(!h.state().can_undo());
    }
}

#[test]
fn uv_draws_only_lines_in_the_canvas_clip_and_respects_color_alpha() {
    let mut app = AppState::new(64, 32);
    app.apply(Action::LoadDemoModel);
    app.sync_view3d();
    app.view.angle = 37.0;
    app.view.flip = true;
    app.view.zoom = 2.0;
    app.prefs.settings.uv_wireframe_color = [11, 123, 231, 77];
    let color = Color32::from_rgba_unmultiplied(11, 123, 231, 77);
    let ctx = egui::Context::default();
    YoluApp::setup(&ctx);
    let rect = Rect::from_min_size(egui::pos2(20.0, 30.0), vec2(400.0, 300.0));
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let view = app.view.view(rect, 64, 32);
        yolu_app::uv_wireframe::show(&mut child, &mut app, &view);
    });
    output.textures_delta.clear();
    let mut lines = 0;
    for shape in output.shapes {
        if let egui::Shape::LineSegment { stroke, .. } = shape.shape {
            if stroke.color == color {
                assert_eq!(stroke.width, 1.0);
                assert_eq!(shape.clip_rect, rect);
                lines += 1;
            }
        }
    }
    assert!(lines > 0);
    assert!(!app.can_undo());
}
