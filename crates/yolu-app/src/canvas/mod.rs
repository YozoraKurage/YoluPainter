//! 2D キャンバスのタブ: 合成の絵（`display`）、表示（拡大・パン・回転・反転。写しは `view`）、ブラシのカーソル、入力。
//! 入力はフレームの中の生のイベントを順に見る（1 フレームに来たマウスの移動を全部ストロークの点にする）。ペン（Windows Ink）の
//! 点が来ていれば、そのフレームのストロークはペンの点だけで描き、同じペンから egui が作るマウスの代わりの入力は使わない。
//! ストロークを取り残さない: ボタンを離す・Esc（捨てる）・窓のフォーカスを失う（そこまでを確定）で必ず終える。

pub mod display;
pub mod view;

use egui::{
    pos2, vec2, Color32, CursorIcon, Event, Key, PointerButton, Pos2, Rect, Sense, Stroke, Ui,
};

use self::display::CanvasDisplay;
use self::view::{angle_label, CanvasView, ROTATE_STEP};
use crate::engine::Tilt;
use crate::pen::PenSample;
use crate::state::{AppState, RotateDrag, StrokeSource};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};

/// 見出しの帯の高さ。
pub const HEADER_HEIGHT: f32 = 26.0;
/// ポインタの角度を測らない、表示域の中心からの距離。
const ROTATE_DEAD_ZONE: f32 = 4.0;

/// キャンバスのタブを描く。
pub fn show(ui: &mut Ui, app: &mut AppState, display: &mut CanvasDisplay, pen: &[PenSample]) {
    let full = ui.max_rect();
    let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_HEIGHT));
    let rect = Rect::from_min_max(pos2(full.left(), header.bottom()), full.max);
    let response = ui.interact(rect, ui.id().with("canvas"), Sense::click_and_drag());
    app.canvas_rect = Some(rect);
    ui.advance_cursor_after_rect(full);
    handle_input(ui, app, rect, pen);
    display.sync(ui.ctx(), &app.doc);

    let (w_px, h_px) = (app.doc.width(), app.doc.height());
    let view = app.view.view(rect, w_px, h_px);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, t::CANVAS_BG);
    display.paint(&painter, &view);

    // ブラシのカーソル（回している・回すキーを押している・パンしているあいだは出さない）
    let hover = ui.input(|i| i.pointer.hover_pos());
    let pointer_on_canvas = hover.is_some_and(|p| rect.contains(p)) && response.contains_pointer();
    let busy =
        app.canvas.rotate_key_held || app.canvas.rotating.is_some() || app.canvas.middle_rotating;
    if pointer_on_canvas {
        if busy {
            ui.ctx().set_cursor_icon(CursorIcon::Move);
        } else if app.canvas.panning {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if app.canvas.space_held {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        } else if let Some(p) = hover {
            let radius = (app.brush.radius * view.pixel_size()).max(1.5);
            painter.circle_stroke(p, radius, Stroke::new(3.0, Color32::from_black_alpha(140)));
            painter.circle_stroke(p, radius, Stroke::new(1.2, Color32::from_white_alpha(230)));
            ui.ctx().set_cursor_icon(if radius >= 4.0 {
                CursorIcon::None
            } else {
                CursorIcon::Crosshair
            });
        }
    }
    draw_header(ui, app, header);
}

fn draw_header(ui: &mut Ui, app: &mut AppState, bar: Rect) {
    let p = ui.painter().clone();
    w::fill(&p, bar, t::PANEL_HEADER);
    w::hline(&p, bar.left(), bar.right(), bar.bottom() - 1.0, t::BORDER);
    let left = bar.left() + 8.0;
    let label = format!(
        "2D · {} · カラー  {}%",
        app.sets.current().name,
        (app.view.zoom * 100.0).round() as i32
    );
    let width = w::text_width(&p, &label, t::LABEL_DIM).min((bar.width() - 16.0).max(0.0));
    let shown = w::fit(&p, &label, width, t::LABEL_DIM);
    w::text(
        &p,
        Rect::from_min_size(pos2(left, bar.top()), vec2(width, bar.height())),
        &shown,
        t::LABEL_DIM,
        Align::Left,
    );
    let mut x = left + width + 8.0;
    let enabled = !app.is_stroking();
    // 読むだけのセット: 鍵と「読むだけ」（理由はツールチップ）
    if let Some(reason) = app.read_only_reason().map(str::to_owned) {
        let text = "読むだけ";
        let bw = 20.0 + w::text_width(&p, text, t::LABEL_DIM) + 8.0;
        if x + bw <= bar.right() {
            let r = Rect::from_min_size(pos2(x, bar.top() + 2.0), vec2(bw, 22.0));
            w::rounded(&p, r, t::CONTROL_BG, 3.0);
            w::icon(
                &p,
                Rect::from_min_size(r.min, vec2(20.0, r.height())),
                "lock",
                t::WARNING,
                14.0,
            );
            w::text(
                &p,
                Rect::from_min_max(pos2(r.left() + 20.0, r.top()), r.max),
                text,
                t::LABEL_DIM.with_color(t::WARNING),
                Align::Left,
            );
            ui.interact(r, ui.id().with("canvas.readonly"), egui::Sense::hover())
                .on_hover_text(reason);
            x += bw + 4.0;
        }
    }
    if app.view.angle != 0.0 {
        let text = angle_label(app.view.angle);
        let bw = 20.0 + w::text_width(&p, &text, t::LABEL_DIM) + 6.0;
        if x + bw <= bar.right() {
            let r = Rect::from_min_size(pos2(x, bar.top() + 2.0), vec2(bw, 22.0));
            if w::button(
                ui,
                r,
                "canvas.angle",
                &text,
                false,
                enabled,
                Some("表示を回しています。押すと回転を戻します（Shift+R）。"),
                Some("rotate_90_degrees_cw"),
            )
            .clicked()
            {
                app.apply(crate::state::Action::ResetRotation);
            }
            x += bw + 4.0;
        }
    }
    if app.view.flip && x + 24.0 <= bar.right() {
        let r = Rect::from_min_size(pos2(x, bar.top() + 2.0), vec2(24.0, 22.0));
        if w::icon_button(
            ui,
            r,
            "canvas.flip",
            "flip",
            "表示を左右反転しています。押すと戻します（H）。",
            true,
            enabled,
            16.0,
        )
        .clicked()
        {
            app.apply(crate::state::Action::FlipView);
        }
    }
}

fn pointer_angle(rect: Rect, p: Pos2) -> f32 {
    let d = p - rect.center();
    d.y.atan2(d.x).to_degrees()
}

fn delta_angle(from: f32, to: f32) -> f32 {
    let d = (to - from) % 360.0;
    if d > 180.0 {
        d - 360.0
    } else if d < -180.0 {
        d + 360.0
    } else {
        d
    }
}

/// この点で一番上にあるのがキャンバスの層か（ポップアップ・浮いた窓が上にあれば描かない）。
fn on_top(ui: &Ui, rect: Rect, p: Pos2) -> bool {
    rect.contains(p)
        && ui
            .ctx()
            .layer_id_at(p)
            .is_none_or(|layer| layer == ui.layer_id())
}

fn begin_stroke(app: &mut AppState, source: StrokeSource, eraser: bool) -> bool {
    if let Some(reason) = app.read_only_reason() {
        app.message = format!("読むだけのテクスチャセットには描けません: {reason}");
        return false;
    }
    let Some(layer) = app.selected_layer else {
        app.message = "描くレイヤーがありません。".into();
        return false;
    };
    let settings = app.stroke_settings(eraser);
    match app.doc.begin_stroke(layer, &settings) {
        Ok(stroke) => {
            app.stroke = Some(stroke);
            app.canvas.stroke = Some(source);
            app.canvas.stroke_points = 0;
            if !settings.erase {
                app.color.remember();
            }
            app.modified = true;
            true
        }
        Err(e) => {
            app.message = format!("描けません: {e}");
            false
        }
    }
}

fn add_point(app: &mut AppState, view: &CanvasView, p: Pos2, pressure: f32, tilt: Tilt) {
    let (x, y) = view.to_canvas(p);
    let Some(stroke) = app.stroke.as_mut() else {
        return;
    };
    match stroke.add_point(
        &mut app.doc,
        x,
        y,
        pressure.clamp(0.0, 1.0) as f64,
        tilt.radians(),
    ) {
        Ok(()) => app.canvas.stroke_points += 1,
        Err(e) => {
            // core は失敗したストロークを取り消してから返す（予算を超えたなど）。札を手放して知らせる
            app.stroke = None;
            app.canvas.stroke = None;
            app.message = e.to_string();
        }
    }
}

/// ストロークを終える（cancel なら捨てる）。
pub fn finish_stroke(app: &mut AppState, cancel: bool) {
    app.canvas.stroke = None;
    let Some(stroke) = app.stroke.take() else {
        // 札を失っていても、core に進行中のストロークが残っていれば取り消す（取り残さない）
        app.doc.cancel_active_stroke();
        return;
    };
    if cancel {
        app.doc.cancel_stroke(stroke);
        app.message = "ストロークを取り消しました。".into();
    } else if let Err(e) = app.doc.end_stroke(stroke) {
        app.message = e.to_string();
    }
}

fn handle_input(ui: &mut Ui, app: &mut AppState, rect: Rect, pen: &[PenSample]) {
    let (w_px, h_px) = (app.doc.width(), app.doc.height());
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    let typing = ctx.egui_wants_keyboard_input();
    let blocked = app.popup.is_some() || app.popup_was_open;
    let (events, modifiers, r_down, space_down) = ui.input(|i| {
        (
            i.events.clone(),
            i.modifiers,
            i.key_down(Key::R),
            i.key_down(Key::Space),
        )
    });
    app.canvas.rotate_key_held = r_down && !typing && !modifiers.any() && !blocked;
    app.canvas.space_held = space_down && !typing && !blocked;

    // ペン（Windows Ink）。点があればこのフレームのストロークはペンだけで描く。
    let pen_frame = !pen.is_empty() || matches!(app.canvas.stroke, Some(StrokeSource::Pen(_)));
    for s in pen {
        let p = s.pos_points(ppp);
        let view = app.view.view(rect, w_px, h_px);
        match app.canvas.stroke {
            None if s.contact
                && !blocked
                && on_top(ui, rect, p)
                && !app.canvas.rotate_key_held
                && !app.canvas.space_held =>
            {
                if begin_stroke(app, StrokeSource::Pen(s.pointer_id), s.eraser) {
                    add_point(app, &view, p, s.pressure, s.tilt);
                }
            }
            Some(StrokeSource::Pen(id)) if id == s.pointer_id && s.contact => {
                add_point(app, &view, p, s.pressure, s.tilt)
            }
            Some(StrokeSource::Pen(id)) if id == s.pointer_id && !s.contact => {
                finish_stroke(app, false)
            }
            _ => {}
        }
    }

    for event in &events {
        match event {
            Event::Touch { force, phase, .. } => {
                if let Some(f) = force {
                    app.canvas.touch_pressure = Some(f.clamp(0.0, 1.0));
                }
                if matches!(phase, egui::TouchPhase::End | egui::TouchPhase::Cancel) {
                    app.canvas.touch_pressure = None;
                }
            }
            Event::PointerButton {
                pos,
                button,
                pressed,
                ..
            } => {
                let pos = *pos;
                match (button, pressed) {
                    (PointerButton::Primary, true) => {
                        if blocked || !on_top(ui, rect, pos) {
                            continue;
                        }
                        if app.canvas.rotate_key_held && !app.is_stroking() {
                            app.canvas.rotating = Some(RotateDrag {
                                start_angle: app.view.angle,
                                start_pan: app.view.pan,
                                swept: 0.0,
                                last_pointer_angle: pointer_angle(rect, pos),
                            });
                        } else if app.canvas.space_held && !app.is_stroking() {
                            app.canvas.panning = true;
                        } else if !pen_frame
                            && app.canvas.stroke.is_none()
                            && begin_stroke(app, StrokeSource::Mouse, false)
                        {
                            let view = app.view.view(rect, w_px, h_px);
                            add_point(
                                app,
                                &view,
                                pos,
                                app.canvas.touch_pressure.unwrap_or(1.0),
                                Tilt::default(),
                            );
                        }
                    }
                    (PointerButton::Primary, false) => {
                        if app.canvas.stroke == Some(StrokeSource::Mouse) {
                            finish_stroke(app, false);
                        }
                        app.canvas.rotating = None;
                        if !app.canvas.middle_rotating {
                            app.canvas.panning = false;
                        }
                    }
                    (PointerButton::Middle, true) => {
                        if !blocked && on_top(ui, rect, pos) {
                            if modifiers.shift {
                                app.canvas.middle_rotating = !app.is_stroking();
                            } else {
                                app.canvas.panning = true;
                            }
                        }
                    }
                    (PointerButton::Middle, false) => {
                        app.canvas.panning = false;
                        app.canvas.middle_rotating = false;
                    }
                    _ => {}
                }
                app.canvas.last_pointer = Some(pos);
            }
            Event::PointerMoved(pos) => {
                let pos = *pos;
                let previous = app.canvas.last_pointer.unwrap_or(pos);
                if app.canvas.stroke == Some(StrokeSource::Mouse) && !pen_frame {
                    let view = app.view.view(rect, w_px, h_px);
                    add_point(
                        app,
                        &view,
                        pos,
                        app.canvas.touch_pressure.unwrap_or(1.0),
                        Tilt::default(),
                    );
                }
                if let Some(mut drag) = app.canvas.rotating {
                    if (pos - rect.center()).length() >= ROTATE_DEAD_ZONE {
                        let a = pointer_angle(rect, pos);
                        drag.swept += delta_angle(drag.last_pointer_angle, a);
                        drag.last_pointer_angle = a;
                        let mut swept = drag.swept;
                        if modifiers.shift {
                            swept = ((drag.start_angle + swept) / ROTATE_STEP).round()
                                * ROTATE_STEP
                                - drag.start_angle;
                        }
                        app.view
                            .rotate_from(drag.start_angle, drag.start_pan, swept);
                        app.canvas.rotating = Some(drag);
                    }
                } else if app.canvas.middle_rotating {
                    if (previous - rect.center()).length() >= ROTATE_DEAD_ZONE
                        && (pos - rect.center()).length() >= ROTATE_DEAD_ZONE
                    {
                        app.view.rotate_by(delta_angle(
                            pointer_angle(rect, previous),
                            pointer_angle(rect, pos),
                        ));
                    }
                } else if app.canvas.panning {
                    app.view.pan += pos - previous;
                }
                app.canvas.last_pointer = Some(pos);
            }
            Event::MouseWheel { unit, delta, .. } => {
                let Some(p) = ui.input(|i| i.pointer.hover_pos()) else {
                    continue;
                };
                if blocked || !on_top(ui, rect, p) {
                    continue;
                }
                let notches = match unit {
                    egui::MouseWheelUnit::Point => delta.y / 40.0,
                    egui::MouseWheelUnit::Line => delta.y,
                    egui::MouseWheelUnit::Page => delta.y * 3.0,
                };
                // 1 目盛りで約 1.23 倍（Unity 版の exp(0.07 × 3)）。ポインタの下の画素は動かない
                app.view
                    .zoom_to(app.view.zoom * (notches * 0.21).exp(), Some(p), rect);
            }
            Event::Key {
                key: Key::Escape,
                pressed: true,
                ..
            } => {
                if app.is_stroking() {
                    finish_stroke(app, true);
                } else if let Some(drag) = app.canvas.rotating.take() {
                    app.view.angle = drag.start_angle;
                    app.view.pan = drag.start_pan;
                }
            }
            Event::WindowFocused(false) => {
                // フォーカスを失ったら、そこまでを確定する（離したのを受け取れないので）
                finish_stroke(app, false);
                app.canvas.rotating = None;
                app.canvas.panning = false;
                app.canvas.middle_rotating = false;
                app.canvas.rotate_key_held = false;
            }
            _ => {}
        }
    }
    // ボタンを離したのを取りこぼしたとき（窓の外で離したなど）も、押していなければ終える
    if app.canvas.stroke == Some(StrokeSource::Mouse)
        && !ui.input(|i| i.pointer.primary_down())
        && !events
            .iter()
            .any(|e| matches!(e, Event::PointerButton { pressed: true, .. }))
    {
        finish_stroke(app, false);
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        app.canvas.rotating = None;
    }
}
