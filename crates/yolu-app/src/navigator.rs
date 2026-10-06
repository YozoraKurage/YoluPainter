//! 2D キャンバスの全体像と表示範囲。表示操作は文書・履歴を変更しない。
use crate::canvas::{
    display::CanvasDisplay,
    view::{CanvasView, ViewState, MAX_ZOOM, MIN_ZOOM},
};
use crate::{
    engine::Channel,
    state::AppState,
    ui::{theme, widgets},
};
use egui::{pos2, vec2, Color32, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions};

#[derive(Clone, Default)]
struct Navigator {
    key: Option<(u128, u64, u64, Channel)>,
    texture: Option<TextureHandle>,
    drag_offset: Option<(f64, f64)>,
}

impl Navigator {
    fn sync(&mut self, ctx: &egui::Context, app: &AppState) {
        let key = (
            app.doc.id(),
            app.doc_epoch,
            app.doc.revision(),
            app.m2.display_channel,
        );
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        self.drag_offset = None;
        self.texture = CanvasDisplay::thumbnail(&app.doc, app.m2.display_channel)
            .ok()
            .map(|image| ctx.load_texture("navigator-image", image, TextureOptions::LINEAR));
    }
}

fn outline(view: &CanvasView, viewport: Rect, miniature: &CanvasView) -> [Pos2; 4] {
    [
        viewport.left_top(),
        viewport.right_top(),
        viewport.right_bottom(),
        viewport.left_bottom(),
    ]
    .map(|p| {
        let (x, y) = view.to_canvas(p);
        miniature.to_screen(x, y)
    })
}

fn center_on(state: &mut ViewState, viewport: Rect, size: (u32, u32), at: (f64, f64)) {
    let p = state.view(viewport, size.0, size.1).to_screen(at.0, at.1);
    let delta = viewport.center() - p;
    if delta.length_sq() > 1e-8 {
        state.pan += delta;
    }
}

/// 独立したタブ。キャッシュとドラッグの状態は egui の一時状態に置く。
pub fn show(ui: &mut egui::Ui, app: &mut AppState) {
    let id = ui.make_persistent_id("navigator-state");
    let mut nav = ui
        .ctx()
        .data_mut(|d| d.get_temp::<Navigator>(id))
        .unwrap_or_default();
    nav.sync(ui.ctx(), app);
    let width = ui.available_width().max(1.0);
    let height = (ui.available_height() - 92.0).clamp(32.0, 256.0);
    let (area, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Image,
            true,
            app.lang.pick("ナビゲーター", "Navigator"),
        )
    });
    let size = (app.doc.width(), app.doc.height());
    let miniature = ViewState::default().view(area.shrink(4.0), size.0, size.1);
    let image = miniature.image;
    let painter = ui.painter_at(area);
    // 透明部分にも位置が分かるよう、キャンバスと同じ市松を重ねる。
    for y in 0..(image.height() / 8.0).ceil() as usize {
        for x in 0..(image.width() / 8.0).ceil() as usize {
            let r = Rect::from_min_size(
                image.min + vec2(x as f32 * 8.0, y as f32 * 8.0),
                vec2(8.0, 8.0),
            )
            .intersect(image);
            painter.rect_filled(
                r,
                0.0,
                if (x + y) % 2 == 0 {
                    theme::CHECKER_LIGHT
                } else {
                    theme::CHECKER_DARK
                },
            );
        }
    }
    if let Some(texture) = &nav.texture {
        painter.image(
            texture.id(),
            image,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    if let Some(viewport) = app.ui.canvas_rect.filter(|r| r.is_positive()) {
        let view = app.view.view(viewport, size.0, size.1);
        if (response.is_pointer_button_down_on() || response.clicked())
            && !app.is_stroking()
            && !app.dock_grabbed()
        {
            if let Some(p) = response.interact_pointer_pos() {
                let at = miniature.to_canvas(p);
                let offset = *nav.drag_offset.get_or_insert_with(|| {
                    if viewport.contains(view.to_screen(at.0, at.1)) {
                        let center = view.to_canvas(viewport.center());
                        (center.0 - at.0, center.1 - at.1)
                    } else {
                        (0.0, 0.0)
                    }
                });
                let previous = app.view;
                center_on(
                    &mut app.view,
                    viewport,
                    size,
                    (at.0 + offset.0, at.1 + offset.1),
                );
                if app.view != previous {
                    ui.ctx().request_repaint();
                }
            }
        } else {
            nav.drag_offset = None;
        }
        let corners = outline(
            &app.view.view(viewport, size.0, size.1),
            viewport,
            &miniature,
        );
        painter.add(egui::Shape::closed_line(
            corners.to_vec(),
            Stroke::new(3.0, Color32::BLACK),
        ));
        painter.add(egui::Shape::closed_line(
            corners.to_vec(),
            Stroke::new(1.0, theme::ACCENT),
        ));
    }
    ui.add_enabled_ui(app.ui.canvas_rect.is_some() && !app.is_stroking(), |ui| {
        let viewport = app.ui.canvas_rect.unwrap_or(area);
        let fit = (viewport.width() / size.0 as f32).min(viewport.height() / size.1 as f32);
        let mut zoom = app.view.zoom;
        if ui
            .add(
                egui::Slider::new(&mut zoom, MIN_ZOOM..=MAX_ZOOM)
                    .logarithmic(true)
                    .clamping(egui::SliderClamping::Edits)
                    .text(app.lang.pick("拡大率", "Zoom"))
                    .custom_formatter(|v, _| format!("{:.0}%", v * fit as f64 * 100.0))
                    .custom_parser(|text| {
                        let text = text.trim();
                        let percent = text
                            .strip_suffix('%')
                            .unwrap_or(text)
                            .trim()
                            .parse::<f64>()
                            .ok()?;
                        let zoom = percent / (fit as f64 * 100.0);
                        zoom.is_finite().then_some(zoom)
                    }),
            )
            .changed()
        {
            app.view.zoom_to(zoom, None, viewport);
            ui.ctx().request_repaint();
        }
        let mut angle = app.view.angle;
        if ui
            .add(
                egui::Slider::new(&mut angle, -180.0..=180.0)
                    .suffix("°")
                    .text(app.lang.pick("回転", "Rotation")),
            )
            .changed()
        {
            app.view.set_angle(angle);
            ui.ctx().request_repaint();
        }
        ui.horizontal(|ui| {
            for (i, icon, label) in [
                (0, "flip", app.lang.pick("左右反転", "Flip horizontally")),
                (
                    1,
                    "arrow_maximize",
                    app.lang.pick("全体を表示", "Fit canvas"),
                ),
                (2, "target", "100%"),
                (
                    3,
                    "restart_alt",
                    app.lang.pick("回転を戻す", "Reset rotation"),
                ),
            ] {
                let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                if widgets::icon_button(ui, r, i, icon, label, i == 0 && app.view.flip, true, 18.0)
                    .clicked()
                {
                    match i {
                        0 => app.view.flip_horizontally(),
                        1 => app.view.fit(),
                        2 => app.view.actual_size(viewport, size.0, size.1),
                        _ => app.view.set_angle(0.0),
                    }
                    ui.ctx().request_repaint();
                }
            }
        });
    });
    ui.ctx().data_mut(|d| d.insert_temp(id, nav));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Document, Rgba8};

    #[test]
    fn navigator_outline_and_movement_follow_every_view_transform() {
        let viewport = Rect::from_min_size(pos2(30.0, 50.0), vec2(600.0, 400.0));
        let miniature = ViewState::default().view(
            Rect::from_min_size(Pos2::ZERO, vec2(200.0, 100.0)),
            800,
            400,
        );
        for zoom in [0.5, 1.0, 3.0] {
            for angle in [0.0, 30.0, 90.0, -150.0] {
                for flip in [false, true] {
                    let mut state = ViewState {
                        zoom,
                        angle,
                        flip,
                        pan: vec2(37.0, -64.0),
                    };
                    let view = state.view(viewport, 800, 400);
                    let points = outline(&view, viewport, &miniature);
                    for (p, expected) in points.into_iter().zip([
                        viewport.left_top(),
                        viewport.right_top(),
                        viewport.right_bottom(),
                        viewport.left_bottom(),
                    ]) {
                        let at = miniature.to_canvas(p);
                        assert!(view.to_screen(at.0, at.1).distance(expected) < 0.001);
                    }
                    center_on(&mut state, viewport, (800, 400), (125.0, 70.0));
                    assert!(
                        state
                            .view(viewport, 800, 400)
                            .to_screen(125.0, 70.0)
                            .distance(viewport.center())
                            < 0.001
                    );
                }
            }
        }
    }

    #[test]
    fn navigator_cache_changes_only_with_document_revision_epoch_or_channel() {
        let ctx = egui::Context::default();
        let mut app = AppState::new(32, 32);
        let mut nav = Navigator::default();
        nav.sync(&ctx, &app);
        let first = nav.texture.as_ref().unwrap().id();
        app.view.rotate_by(33.0);
        app.view.zoom = 3.0;
        nav.sync(&ctx, &app);
        assert_eq!(nav.texture.as_ref().unwrap().id(), first);
        app.doc.add_layer("test").unwrap();
        nav.sync(&ctx, &app);
        let revised = nav.texture.as_ref().unwrap().id();
        assert_ne!(first, revised);
        app.doc_epoch += 1;
        nav.sync(&ctx, &app);
        assert_ne!(nav.texture.as_ref().unwrap().id(), revised);
        app.m2.display_channel = Channel::Normal;
        nav.sync(&ctx, &app);
        assert_eq!(nav.key.unwrap().3, Channel::Normal);
    }

    #[test]
    fn navigator_thumbnail_is_bounded_top_down_and_alpha_averaged() {
        let mut doc = Document::new(512, 2).unwrap();
        let layer = doc.add_layer("test").unwrap();
        doc.set_pixel(layer, 0, 1, Rgba8::new(255, 0, 0, 255))
            .unwrap();
        doc.set_pixel(layer, 1, 1, Rgba8::new(0, 0, 255, 0))
            .unwrap();
        let image = CanvasDisplay::thumbnail(&doc, Channel::Color).unwrap();
        assert_eq!(image.size, [256, 1]);
        assert_eq!(
            image.pixels[0],
            Color32::from_rgba_premultiplied(64, 0, 0, 64)
        );
        let mut doc = Document::new(2, 2).unwrap();
        let layer = doc.add_layer("test").unwrap();
        doc.set_pixel(layer, 0, 1, Rgba8::new(255, 0, 0, 255))
            .unwrap();
        let image = CanvasDisplay::thumbnail(&doc, Channel::Color).unwrap();
        assert_eq!(image.pixels[0], Color32::RED);
        assert_eq!(image.pixels[2], Color32::TRANSPARENT);
    }

    #[test]
    fn review_cache_pixels_follow_save_open_undo_redo_and_cancel() {
        let ctx = egui::Context::default();
        let mut app = AppState::new(8, 8);
        let mut nav = Navigator::default();
        let read = |nav: &mut Navigator, app: &AppState| {
            nav.sync(&ctx, app);
            let id = nav.texture.as_ref().unwrap().id();
            let mut delta = ctx.tex_manager().write().take_delta();
            let image = delta
                .set
                .iter()
                .find(|(key, _)| **key == id)
                .expect("画像を更新")
                .1;
            let egui::ImageData::Color(image) = image[0].image.clone();
            let pixel = image.pixels[(7 - 5) * 8 + 3];
            delta.clear();
            pixel
        };
        assert_eq!(read(&mut nav, &app), Color32::TRANSPARENT);
        let layer = app.doc.layers()[0].id();
        app.doc
            .fill(
                layer,
                Channel::Color,
                Rgba8::new(255, 0, 0, 255),
                1.0,
                None,
                false,
            )
            .unwrap();
        assert_eq!(read(&mut nav, &app), Color32::RED);
        assert!(app.doc.undo().unwrap());
        assert_eq!(read(&mut nav, &app), Color32::TRANSPARENT);
        assert!(app.doc.redo().unwrap());
        assert_eq!(read(&mut nav, &app), Color32::RED);
        let undo = app.doc.undo_count();
        app.doc.set_layer_opacity(layer, 0.0, true).unwrap();
        assert_eq!(read(&mut nav, &app), Color32::TRANSPARENT);
        assert!(app.doc.cancel_coalescing().unwrap());
        assert_eq!(read(&mut nav, &app), Color32::RED);
        assert_eq!(app.doc.undo_count(), undo);
        let dir = std::env::temp_dir().join(format!("navigator-review-{}", app.doc.id()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("navigator.ylp");
        crate::project::save_from(&mut app, &path);
        assert!(path.is_file(), "{}", app.message);
        app.doc
            .fill(
                layer,
                Channel::Color,
                Rgba8::new(0, 0, 255, 255),
                1.0,
                None,
                false,
            )
            .unwrap();
        assert_eq!(read(&mut nav, &app), Color32::BLUE);
        let epoch = app.doc_epoch;
        crate::project::open_into(&mut app, &path);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
        assert!(app.doc_epoch > epoch);
        assert_eq!(read(&mut nav, &app), Color32::RED);
        assert_eq!(app.doc.undo_count(), 0);
        assert_eq!(app.doc.redo_count(), 0);
        app.view.rotate_by(30.0);
        app.view.flip_horizontally();
        app.view
            .actual_size(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0)), 8, 8);
        nav.sync(&ctx, &app);
        let mut delta = ctx.tex_manager().write().take_delta();
        assert!(delta.set.is_empty());
        delta.clear();
        assert_eq!(app.doc.undo_count(), 0);
        assert_eq!(app.doc.redo_count(), 0);
    }

    #[test]
    fn navigator_actual_size_preserves_the_center() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0));
        let mut state = ViewState {
            zoom: 3.0,
            pan: vec2(12.0, 48.0),
            angle: 30.0,
            flip: true,
        };
        let before = state.view(rect, 8192, 8192).to_canvas(rect.center());
        state.actual_size(rect, 8192, 8192);
        let view = state.view(rect, 8192, 8192);
        assert!((view.pixel_size() - 1.0).abs() < 1e-6);
        let after = view.to_canvas(rect.center());
        assert!((before.0 - after.0).abs() < 0.001 && (before.1 - after.1).abs() < 0.001);
    }
}
