//! 2D キャンバスのタブ: 合成の絵（`display`）、表示（拡大・パン・回転・反転。写しは `view`）、ブラシのカーソル、入力。
//! 入力はフレームの中の生のイベントを順に見る（1 フレームに来たマウスの移動を全部ストロークの点にする）。ペン（Windows Ink）の
//! 点が来ていれば、そのフレームのストロークはペンの点だけで描き、同じペンから egui が作るマウスの代わりの入力は使わない。
//! ストロークを取り残さない: ボタンを離す・Esc（捨てる）・窓のフォーカスを失う（そこまでを確定）で必ず終える。

pub mod display;
pub mod gpu;
pub mod view;

use egui::{
    Color32, CursorIcon, Event, Key, PointerButton, Pos2, Rect, Sense, Stroke, Ui,
};

use self::display::CanvasDisplay;
use self::view::{angle_label, CanvasView, ROTATE_STEP};
use crate::engine::{BrushSample, Tilt};
use crate::pen::PenSample;
use crate::state::{AppState, RotateDrag, StrokeSource};
use crate::ui::theme as t;
use crate::ui::widgets as w;

/// ポインタの角度を測らない、表示域の中心からの距離。
const ROTATE_DEAD_ZONE: f32 = 4.0;

/// キャンバスのタブを描く。
pub fn show(ui: &mut Ui, app: &mut AppState, display: &mut CanvasDisplay, pen: &[PenSample]) {
    let rect = ui.max_rect();
    let response = ui.interact(rect, ui.id().with("canvas"), Sense::click_and_drag());
    app.canvas_rect = Some(rect);
    app.canvas_drawn = true;
    app.canvas_frame = Some(ui.ctx().cumulative_frame_nr());
    ui.advance_cursor_after_rect(rect);
    handle_input(ui, app, rect, pen);
    display.set_document_epoch(app.doc_epoch);
    display.sync_channel(ui.ctx(), &app.doc, app.m2.display_channel);

    let (w_px, h_px) = (app.doc.width(), app.doc.height());
    let view = app.view.view(rect, w_px, h_px);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, t::CANVAS_BG);
    display.paint(&painter, &view);
    // 選択の縁・ドラッグ中の形・対称の軸
    crate::selection::canvas::paint_overlay(ui.ctx(), &painter, &view, app);
    // 移動・変形の道具: 動かすものの外枠とハンドル（ドラッグ中は変形後の外枠）
    crate::transform::canvas::paint_overlay(&painter, &view, app);
    // 焼いたメッシュマップを見ているとき（読むだけの重ね表示）
    crate::bake::overlay::paint(&painter, app, &view);
    // ステンシル（画面に貼り付いた半透明の画像。T を押しているあいだは枠も）
    crate::stencil::draw_overlay(&painter, &mut app.stencil, rect);

    // ブラシのカーソル（回している・回すキーを押している・パンしている・ステンシルを動かしているあいだは出さない）
    let hover = ui.input(|i| i.pointer.hover_pos());
    let pointer_on_canvas = hover.is_some_and(|p| rect.contains(p)) && response.contains_pointer();
    // 範囲の道具: ポインタの下の範囲の UV の輪郭（回している・パンしているあいだは出さない）
    {
        let navigating = app.canvas.rotate_key_held
            || app.canvas.rotating.is_some()
            || app.canvas.middle_rotating
            || app.canvas.panning
            || app.canvas.space_held;
        let at = hover.filter(|_| pointer_on_canvas && !navigating);
        crate::region::overlay::paint_canvas(&painter, app, &view, at);
    }
    let busy =
        app.canvas.rotate_key_held || app.canvas.rotating.is_some() || app.canvas.middle_rotating;
    if pointer_on_canvas {
        if let Some(icon) = crate::stencil::cursor_icon(&app.stencil) {
            ui.ctx().set_cursor_icon(icon);
        } else if busy {
            ui.ctx().set_cursor_icon(CursorIcon::Move);
        } else if app.canvas.panning {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if app.canvas.space_held {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        } else if app.tool.is_region() || app.tool.is_select() {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        } else if app.tool == crate::state::Tool::Move {
            let icon = crate::transform::canvas::cursor(app, &view, hover);
            ui.ctx().set_cursor_icon(icon);
        } else if let Some(p) = hover {
            let radius = (app.brush.radius * view.pixel_size()).max(1.5);
            painter.circle_stroke(p, radius, Stroke::new(3.0, Color32::from_black_alpha(140)));
            painter.circle_stroke(p, radius, Stroke::new(1.2, Color32::from_white_alpha(230)));
            crate::selection::canvas::paint_mirrored_cursors(&painter, &view, app, p, radius);
            ui.ctx().set_cursor_icon(if radius >= 4.0 {
                CursorIcon::None
            } else {
                CursorIcon::Crosshair
            });
        }
    }
    draw_corner(ui, app, rect);
}

/// 表示域の右上の隅に重ねる小さなアイコン（見出しの帯は置かない）。読むだけのセットの鍵・表示の回転・左右反転・重ねて見ている
/// メッシュマップだけが、今の状態のとき出る。文字は無く、名前と理由はツールチップ。
fn draw_corner(ui: &mut Ui, app: &mut AppState, view: Rect) {
    let lang = app.lang;
    let enabled = !app.is_stroking();
    let mut items = Vec::new();
    // 押したとき当てる操作（状態を見せるだけの印は None）
    let mut actions: Vec<Option<crate::state::Action>> = Vec::new();
    // 読むだけのセット: 鍵（理由はツールチップ）
    if let Some(reason) = app.read_only_reason() {
        items.push(
            w::CornerIcon::new(
                "readonly",
                "lock",
                format!("{}: {reason}", lang.pick("読むだけ", "Read-only")),
            )
            .indicator()
            .color(t::WARNING),
        );
        actions.push(None);
    }
    if app.view.angle != 0.0 {
        items.push(
            w::CornerIcon::new(
                "angle",
                "rotate_90_degrees_cw",
                lang.pick(
                    format!("表示を回しています（{}）。押すと回転を戻します（Shift+R）", angle_label(app.view.angle)),
                    format!("The view is rotated ({}). Click to reset the rotation (Shift+R)", angle_label(app.view.angle)),
                ),
            )
            .enabled(enabled),
        );
        actions.push(Some(crate::state::Action::ResetRotation));
    }
    if app.view.flip {
        items.push(
            w::CornerIcon::new(
                "flip",
                "flip",
                lang.pick(
                    "表示を左右反転しています。押すと戻します（H）",
                    "The view is mirrored. Click to restore it (H)",
                ),
            )
            .enabled(enabled),
        );
        actions.push(Some(crate::state::Action::FlipView));
    }
    // 焼いたメッシュマップを重ねて見ている: 名前（押し込まれた見た目。押すとやめる）
    if let Some(name) = crate::bake::overlay::view_name(app) {
        items.push(
            w::CornerIcon::new(
                "meshmap",
                "visibility",
                format!("{}: {name}", lang.pick("メッシュマップ", "Mesh Map")),
            )
            .selected(true),
        );
        actions.push(Some(crate::state::Action::Bake(
            crate::bake::BakeAction::View(crate::bake::MeshMapView::None),
        )));
    }
    if let Some(action) = w::corner_icons(ui, "canvas", view, &items)
        .clicked
        .and_then(|i| actions.swap_remove(i))
    {
        app.apply(action);
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

/// 押した点で始める: ブラシ・消しゴムはストローク、バケツ・ポリゴン塗りつぶし・ID の色で選択は範囲の道具（`region`）。
/// ストロークかドラッグを始めたら true。
fn begin_any(
    app: &mut AppState,
    view: &CanvasView,
    p: Pos2,
    source: StrokeSource,
    eraser: bool,
    rect: Rect,
) -> bool {
    if app.tool.is_region() {
        if app.region.drag.is_some() {
            return false;
        }
        let began = crate::region::tools::canvas_press(app, view, p, source);
        if began {
            app.canvas.stroke = Some(source);
            app.canvas.stroke_points = 0;
        }
        return began;
    }
    begin_stroke(app, source, eraser, rect)
}

fn begin_stroke(app: &mut AppState, source: StrokeSource, eraser: bool, rect: Rect) -> bool {
    if let Some(reason) = app.read_only_reason() {
        app.message = format!(
            "{}: {reason}",
            app.lang.pick("読むだけのテクスチャセットには描けません", "Cannot paint on a read-only texture set")
        );
        return false;
    }
    let Some(layer) = app.selected_layer else {
        app.message = app.lang.pick("描くレイヤーがありません。", "No layer to paint on.").into();
        return false;
    };
    if let Some(reason) = app.paint_blocker() {
        app.message = reason;
        return false;
    }
    let settings = app.stroke_settings(eraser);
    // ステンシルの置き場はストロークの始めに決める（ストロークの間は変えない）
    let stencil = match app.canvas_stencil(rect) {
        Ok(s) => s,
        Err(e) => {
            app.message = format!("{}: {}", app.lang.pick("描けません", "Cannot paint"), app.lang.core_error(&e));
            return false;
        }
    };
    match app.begin_canvas_stroke(layer, eraser, stencil) {
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
            app.message = format!(
                "{}: {}",
                app.lang.pick("描けません", "Cannot paint"),
                crate::matpaint::refusal_text(app.lang, &e)
            );
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
    // ポリゴン塗りつぶしのドラッグは、点でなく通った範囲を足す
    if app.region.drag.is_some() {
        crate::region::tools::drag_to(app, crate::region::tools::Where::Canvas(view), p);
        return;
    }
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
            app.message = app.lang.core_error(&e);
        }
    }
}

/// ストロークを終える（cancel なら捨てる）。
pub fn finish_stroke(app: &mut AppState, cancel: bool) {
    app.canvas.stroke = None;
    if crate::region::tools::finish_drag(app, cancel) {
        return;
    }
    let Some(stroke) = app.stroke.take() else {
        // 札を失っていても、core に進行中のストロークが残っていれば取り消す（取り残さない）
        app.doc.cancel_active_stroke();
        return;
    };
    if cancel {
        app.doc.cancel_stroke(stroke);
        app.message = app.lang.pick("ストロークを取り消しました。", "Stroke cancelled.").into();
    } else if let Err(e) = app.doc.end_stroke(stroke) {
        app.message = app.lang.core_error(&e);
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
    let blocked = app.popup.is_some() || app.popup_was_open || app.sel.dialog.is_some();
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
    app.region.modifiers = modifiers;
    app.canvas.rotate_key_held = r_down && !typing && !modifiers.any() && !blocked;
    app.canvas.space_held = space_down && !typing && !blocked;

    // ペン（Windows Ink）。点があればこのフレームのストロークはペンだけで描く。
    let pen_frame = !pen.is_empty() || matches!(app.canvas.stroke, Some(StrokeSource::Pen(_)));
    for s in pen {
        let p = s.pos_points(ppp);
        let view = app.view.view(rect, w_px, h_px);
        // 押した瞬間に終わるツール（バケツ・ID の色で選択）をこのペンで押している間は、次の点で押し直さない（離したら印を下ろす）
        if app.canvas.pen_once == Some(s.pointer_id) {
            if !s.contact {
                app.canvas.pen_once = None;
            }
            continue;
        }
        // 描いている最中のペンは、道具を選択へ替えても従来の match で終わらせる（離したのを受け取れず取り残さない）
        let pen_stroke_running = matches!(app.canvas.stroke, Some(StrokeSource::Pen(_)));
        if app.tool == crate::state::Tool::Move && !pen_stroke_running {
            // 移動・変形の道具: 触れる・動く・離すを、押す・動く・離すにする（ペンで描かない）
            let usable = !blocked
                && !app.canvas.rotate_key_held
                && !app.canvas.space_held
                && !app.stencil.handling()
                && (app.transform.pen_down.is_some() || on_top(ui, rect, p));
            if usable || !s.contact {
                crate::transform::canvas::pen_sample(app, &view, p, s.pointer_id, s.contact, modifiers);
            }
            continue;
        }
        if app.tool.is_select() && !pen_stroke_running {
            // 選択の道具: 触れる・動く・離すを、押す・動く・離すにする
            let usable = !blocked
                && !app.canvas.rotate_key_held
                && !app.canvas.space_held
                && !app.stencil.handling()
                && (app.sel.pen_down.is_some() || on_top(ui, rect, p));
            if usable || !s.contact {
                crate::selection::canvas::pen_sample(
                    app,
                    &view,
                    p,
                    s.pointer_id,
                    s.contact,
                    modifiers,
                    now,
                );
            }
            continue;
        }
        match app.canvas.stroke {
            None if s.contact
                && !blocked
                && on_top(ui, rect, p)
                && !app.canvas.rotate_key_held
                && !app.canvas.space_held
                && !app.stencil.handling() =>
            {
                if begin_any(app, &view, p, StrokeSource::Pen(s.pointer_id), s.eraser, rect) {
                    add_point(
                        app,
                        &view,
                        p,
                        s.pressure,
                        s.tilt,
                        s.rotation,
                        s.time_ms as f64 / 1000.0,
                    );
                } else if app.tool.is_one_shot() {
                    app.canvas.pen_once = Some(s.pointer_id);
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
        // T を押しているあいだのドラッグはステンシルの置き場を動かす（描かない・回さない・パンしない）
        let over = match event {
            Event::PointerButton { pos, .. } => on_top(ui, rect, *pos),
            _ => false,
        };
        if crate::stencil::handle_event(app, event, rect, over, modifiers.shift) {
            continue;
        }
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
                modifiers: event_modifiers,
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
                        } else if app.tool == crate::state::Tool::Move {
                            if !pen_frame && !app.stencil.handling() {
                                let view = app.view.view(rect, w_px, h_px);
                                crate::transform::canvas::press(
                                    app,
                                    &view,
                                    pos,
                                    StrokeSource::Mouse,
                                    *event_modifiers,
                                );
                            }
                        } else if app.tool.is_select() {
                            if !pen_frame {
                                let view = app.view.view(rect, w_px, h_px);
                                crate::selection::canvas::press(
                                    app,
                                    &view,
                                    pos,
                                    StrokeSource::Mouse,
                                    *event_modifiers,
                                    now,
                                );
                            }
                        } else if !pen_frame
                            && app.canvas.stroke.is_none()
                            && !app.stencil.handling()
                            && {
                                let view = app.view.view(rect, w_px, h_px);
                                begin_any(app, &view, pos, StrokeSource::Mouse, false, rect)
                            }
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
                        if app
                            .transform
                            .drag
                            .as_ref()
                            .is_some_and(|d| d.source == StrokeSource::Mouse)
                        {
                            let view = app.view.view(rect, w_px, h_px);
                            crate::transform::canvas::release(
                                app,
                                &view,
                                pos,
                                StrokeSource::Mouse,
                                event_modifiers.shift,
                            );
                        }
                        if app.sel.drag.is_some() {
                            let view = app.view.view(rect, w_px, h_px);
                            crate::selection::canvas::release(
                                app,
                                &view,
                                pos,
                                StrokeSource::Mouse,
                                *event_modifiers,
                            );
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
                if app.tool.is_select() && !pen_frame {
                    let view = app.view.view(rect, w_px, h_px);
                    crate::selection::canvas::moved(app, &view, pos, StrokeSource::Mouse);
                }
                if app.transform.drag.is_some() && !pen_frame {
                    let view = app.view.view(rect, w_px, h_px);
                    crate::transform::canvas::moved(
                        app,
                        &view,
                        pos,
                        StrokeSource::Mouse,
                        modifiers.shift,
                    );
                }
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
                if crate::transform::canvas::cancel(app) {
                    // 移動・変形のドラッグは何も変えずにやめた
                } else if app.is_stroking() {
                    finish_stroke(app, true);
                } else if let Some(drag) = app.canvas.rotating.take() {
                    app.view.angle = drag.start_angle;
                    app.view.pan = drag.start_pan;
                } else {
                    crate::selection::canvas::cancel(app);
                }
            }
            Event::Key {
                key: Key::Enter,
                pressed: true,
                ..
            } if app.transform.drag.is_some() && !typing && !blocked => {
                // ドラッグの途中の Enter: その位置で確定する（あとで離しても、もう何もしない）
                crate::transform::canvas::commit(app);
            }
            Event::Key {
                key: Key::Enter,
                pressed: true,
                modifiers: event_modifiers,
                ..
            } if app.tool == crate::state::Tool::Polygon && !typing && !blocked => {
                crate::selection::canvas::finish_polygon(app, *event_modifiers);
            }
            Event::Key {
                key: Key::Backspace,
                pressed: true,
                ..
            } if app.tool == crate::state::Tool::Polygon && !typing && !blocked => {
                crate::selection::canvas::remove_last_point(app);
            }
            Event::WindowFocused(false) => {
                // フォーカスを失ったら、そこまでを確定する（離したのを受け取れないので）。選択の途中の形は捨てる
                finish_stroke(app, false);
                app.canvas.pen_once = None;
                app.sel.cancel_drafts();
                app.transform_cancel_drag(); // 離したのを受け取れないので、移動・変形は何も変えずにやめる
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
    // 選択の形のドラッグも、離したのを取りこぼしたら、最後の位置で確定する
    if app
        .sel
        .drag
        .as_ref()
        .is_some_and(|d| d.source == StrokeSource::Mouse)
        && !ui.input(|i| i.pointer.primary_down())
        && !events
            .iter()
            .any(|e| matches!(e, Event::PointerButton { pressed: true, .. }))
    {
        let view = app.view.view(rect, w_px, h_px);
        let at = app.canvas.last_pointer.unwrap_or(rect.center());
        crate::selection::canvas::release(app, &view, at, StrokeSource::Mouse, modifiers);
    }
    // 移動・変形のドラッグも、離したのを取りこぼしたら、最後の位置で確定する
    if app
        .transform
        .drag
        .as_ref()
        .is_some_and(|d| d.source == StrokeSource::Mouse)
        && !ui.input(|i| i.pointer.primary_down())
        && !events
            .iter()
            .any(|e| matches!(e, Event::PointerButton { pressed: true, .. }))
    {
        crate::transform::canvas::commit(app);
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        app.canvas.rotating = None;
    }
    crate::stencil::settle(app, ui.input(|i| i.pointer.any_down()));
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
