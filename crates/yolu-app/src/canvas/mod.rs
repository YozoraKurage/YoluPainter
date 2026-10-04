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
use crate::engine::{BrushSample, Tilt};
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
    display.sync_channel(ui.ctx(), &app.doc, app.m2.display_channel);

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
        "2D · {} · {}  {}%",
        app.sets.current().name,
        crate::m2::channel_name(app.lang, &app.doc, app.m2.display_channel),
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
    if let Some(reason) = app.paint_blocker() {
        app.message = reason;
        return false;
    }
    let settings = app.stroke_settings(eraser);
    match app.begin_paint_stroke(layer, eraser) {
        Ok(stroke) => {
            app.stroke = Some(stroke);
            app.canvas.stroke = Some(source);
            app.canvas.stroke_points = 0;
            app.canvas.stroke_time = None;
            if !settings.erase {
                app.color.remember();
            }
            app.modified = true;
            true
        }
        Err(e) => {
            app.message = format!("{}: {e}", app.lang.pick("描けません", "Cannot paint"));
            false
        }
    }
}

/// ペン・マウスの 1 点（時刻は秒。速さの制御に使う。戻さない）。傾きとペンの回転は表示の回転・反転を直してキャンバスの向きで渡す。
/// 回転の情報が無い入力（マウス・タッチ・回転を送れないペン）は None で、core にも回転を渡さない（表示の向きで 0 でなくならない）。
fn add_point(
    app: &mut AppState,
    view: &CanvasView,
    p: Pos2,
    pressure: f32,
    tilt: Tilt,
    rotation: Option<f32>,
    time: f64,
) {
    let (x, y) = view.to_canvas(p);
    let Some(stroke) = app.stroke.as_mut() else {
        return;
    };
    let time = time.max(app.canvas.stroke_time.unwrap_or(f64::NEG_INFINITY));
    let sample = BrushSample::new(
        x,
        y,
        pressure.clamp(0.0, 1.0) as f64,
        time,
        view.tilt_to_canvas(tilt),
    )
    .and_then(|s| match rotation {
        Some(degrees) => s.with_rotation(view.rotation_to_canvas(degrees)),
        None => Ok(s),
    });
    let result = match sample {
        Ok(s) => stroke.add_sample(&mut app.doc, s),
        Err(e) => {
            app.doc.cancel_active_stroke();
            Err(e)
        }
    };
    match result {
        Ok(()) => {
            app.canvas.stroke_time = Some(time);
            app.canvas.stroke_points += 1;
        }
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

/// マウスで描く点になりうるイベント（押す・動く）。
fn is_mouse_sample_event(event: &Event) -> bool {
    matches!(
        event,
        Event::PointerMoved(_)
            | Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            }
    )
}

/// マウスの点の時刻。egui のイベントには時刻が無く、1 フレームの全イベントに `now` を付けると、時刻が進まない点では core が速さを
/// 前の値のままにするので、1 フレームに N 個のイベントがあれば速さが本当の約 1/N になる。そこで前のフレームから `now` までを
/// そのフレームのイベントの数で等分し、単調に増える時刻を付ける（最後のイベントが `now`）。長く止まったあとの最初のフレームで
/// 速さが極端に遅く見えないよう、間隔は `MAX_FRAME_GAP` までに抑える。
struct MouseClock {
    now: f64,
    start: f64,
    step: f64,
    index: usize,
}

impl MouseClock {
    const MAX_FRAME_GAP: f64 = 0.1;

    fn new(now: f64, frame_dt: f64, events: usize) -> MouseClock {
        let dt = frame_dt.clamp(0.0, Self::MAX_FRAME_GAP);
        MouseClock {
            now,
            start: now - dt,
            step: dt / events.max(1) as f64,
            index: 0,
        }
    }

    fn next(&mut self) -> f64 {
        self.index += 1;
        (self.start + self.step * self.index as f64).min(self.now)
    }
}

fn handle_input(ui: &mut Ui, app: &mut AppState, rect: Rect, pen: &[PenSample]) {
    let (w_px, h_px) = (app.doc.width(), app.doc.height());
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    let typing = ctx.egui_wants_keyboard_input();
    let (now, frame_dt) = ctx.input(|i| (i.time, i.unstable_dt as f64));
    let blocked = app.popup.is_some() || app.popup_was_open;
    let (events, modifiers, r_down, space_down) = ui.input(|i| {
        (
            i.events.clone(),
            i.modifiers,
            i.key_down(Key::R),
            i.key_down(Key::Space),
        )
    });
    let mut clock = MouseClock::new(
        now,
        frame_dt,
        events.iter().filter(|e| is_mouse_sample_event(e)).count(),
    );
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
                    add_point(
                        app,
                        &view,
                        p,
                        s.pressure,
                        s.tilt,
                        s.rotation,
                        s.time_ms as f64 / 1000.0,
                    );
                }
            }
            Some(StrokeSource::Pen(id)) if id == s.pointer_id && s.contact => add_point(
                app,
                &view,
                p,
                s.pressure,
                s.tilt,
                s.rotation,
                s.time_ms as f64 / 1000.0,
            ),
            Some(StrokeSource::Pen(id)) if id == s.pointer_id && !s.contact => {
                finish_stroke(app, false)
            }
            _ => {}
        }
    }

    for event in &events {
        // 描く点になりうるイベントごとに 1 つずつ進める（描かなくても進める。数えたときと同じ数になる）
        let time = if is_mouse_sample_event(event) {
            clock.next()
        } else {
            now
        };
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
                                None,
                                time,
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
                        None,
                        time,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_times_spread_over_the_frame_and_end_at_now() {
        let mut clock = MouseClock::new(10.0, 0.016, 4);
        let times: Vec<f64> = (0..4).map(|_| clock.next()).collect();
        assert!(times.windows(2).all(|w| w[1] > w[0]), "{times:?}");
        assert!((times[0] - 9.988).abs() < 1e-9, "{times:?}");
        assert!((times[3] - 10.0).abs() < 1e-9, "{times:?}");
    }

    #[test]
    fn a_long_pause_does_not_make_the_first_frame_look_slow() {
        let mut clock = MouseClock::new(100.0, 30.0, 2);
        let (a, b) = (clock.next(), clock.next());
        assert!(b - a <= MouseClock::MAX_FRAME_GAP, "{a} {b}");
        assert!((b - 100.0).abs() < 1e-9);
    }

    #[test]
    fn no_events_or_a_negative_gap_stay_at_now() {
        let mut clock = MouseClock::new(5.0, -1.0, 0);
        assert_eq!(clock.next(), 5.0);
        assert_eq!(clock.next(), 5.0, "数えた数より多く呼んでも now を越えない");
    }
}
