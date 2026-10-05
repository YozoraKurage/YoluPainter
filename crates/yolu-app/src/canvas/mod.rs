//! 2D キャンバスのタブ: 合成の絵（`display`）、表示（拡大・パン・回転・反転。写しは `view`）、ブラシのカーソル、入力。
//! 入力はフレームの中の生のイベントを順に見る（1 フレームに来たマウスの移動を全部ストロークの点にする）。ペン（Windows Ink）の
//! 点が来ていれば、そのフレームのストロークはペンの点だけで描き、同じペンから egui が作るマウスの代わりの入力は使わない。
//! ストロークを取り残さない: ボタンを離す・Esc（捨てる）・窓のフォーカスを失う（そこまでを確定）で必ず終える。

pub mod display;
pub mod gpu;
pub mod nav;
pub mod view;

use egui::{
    Color32, CursorIcon, Event, Key, Modifiers, PointerButton, Pos2, Rect, Sense, Stroke, Ui,
};

use self::display::CanvasDisplay;
use self::view::{angle_label, CanvasView};
use crate::engine::{BrushSample, Tilt};
use crate::gesture;
use crate::pen::{PenPress, PenSample, PressKind};
use crate::state::{AppState, ShiftHold, StrokeSource};
use crate::tools::input::{CanvasKind, InputCtx};
use crate::ui::theme as t;
use crate::ui::widgets as w;

/// ポインタの角度を測らない、表示域の中心からの距離。
const ROTATE_DEAD_ZONE: f32 = 4.0;
/// Shift で押した点から動いたとみなす画面の距離（点）。これより内側のぶれでは、向きを決めず点も動かさない。縮小して見ていても
/// 画面の 1 画素のぶれが数画素の向きに見えないよう、文書の画素でなく画面の点で測る。
const SHIFT_HOLD_POINTS: f32 = 8.0;

/// キャンバスのタブを描く。
pub fn show(ui: &mut Ui, app: &mut AppState, display: &mut CanvasDisplay, pen: &[PenSample]) {
    let rect = ui.max_rect();
    let response = ui.interact(rect, ui.id().with("canvas"), Sense::click_and_drag());
    app.canvas_rect = Some(rect);
    app.canvas_drawn = true;
    app.canvas_frame = Some(ui.ctx().cumulative_frame_nr());
    ui.advance_cursor_after_rect(rect);
    handle_input(
        ui,
        app,
        rect,
        pen,
        gesture::foreign_press(ui.ctx(), &response),
    );
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
    // グラデーションの道具: ドラッグ中の線
    crate::gradient::canvas::paint_overlay(&painter, &view, app);
    crate::drafting::canvas::paint_overlay(&painter, &view, app);
    // 選択範囲の下のボタンの帯（描いている間・選択の形を作っている間・表示を動かしている間は出ない）
    crate::selection::bar::show(ui, app, &view, rect);
    // 焼いたメッシュマップを見ているとき（読むだけの重ね表示）
    crate::bake::overlay::paint(&painter, app, &view);
    crate::uv_wireframe::show(ui, app, &view);
    // ステンシル（画面に貼り付いた半透明の画像。T を押しているあいだは枠も）
    crate::stencil::draw_overlay(&painter, &mut app.stencil, rect);
    // パスの道具: 選んでいる層の 2D のパスの線と点
    let hover_for_path = ui.input(|i| i.pointer.hover_pos());
    crate::pathtool::canvas::paint_overlay(
        &painter,
        &view,
        app,
        hover_for_path.filter(|p| rect.contains(*p) && response.contains_pointer()),
    );

    // ブラシのカーソル（回している・回すキーを押している・パンしている・ステンシルを動かしているあいだは出さない）
    let hover = ui.input(|i| i.pointer.hover_pos());
    let pointer_on_canvas = hover.is_some_and(|p| rect.contains(p)) && response.contains_pointer();
    // 範囲の道具: ポインタの下の範囲の UV の輪郭（回している・パンしているあいだは出さない）
    {
        let navigating = app.canvas.rotate_key_held
            || app.canvas.rotating.is_some()
            || app.canvas.middle_rotating
            || app.canvas.panning
            || app.canvas.zooming.is_some()
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
        } else if let Some(zoom) = app.canvas.zooming {
            ui.ctx().set_cursor_icon(zoom_cursor(zoom.out));
        } else if app.canvas.space_held && ui.input(|i| gesture::zoom_chord(&i.modifiers, true)) {
            // Ctrl+Space を押している: 虫めがね（Alt も押していれば縮小）
            ui.ctx()
                .set_cursor_icon(zoom_cursor(ui.input(|i| i.modifiers.alt)));
        } else if app.canvas.space_held {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        } else if crate::eyedrop::picks(app, crate::keymap::picks(&ui.input(|i| i.modifiers))) {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        } else if let Some(icon) = app.tool.def().cursor.icon(app, &view, hover) {
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
                    format!(
                        "表示を回しています（{}）。押すと回転を戻します（Shift+R）",
                        angle_label(app.view.angle)
                    ),
                    format!(
                        "The view is rotated ({}). Click to reset the rotation (Shift+R)",
                        angle_label(app.view.angle)
                    ),
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

/// Ctrl+Space の虫めがねのポインタ（縮小は Alt）。
pub fn zoom_cursor(out: bool) -> CursorIcon {
    if out {
        CursorIcon::ZoomOut
    } else {
        CursorIcon::ZoomIn
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

/// スポイト（Alt を押した描く道具も）は押した所の値を取るだけで、始めない。ストロークかドラッグを始めたら true。
#[allow(clippy::too_many_arguments)]
fn begin_any(
    app: &mut AppState,
    view: &CanvasView,
    p: Pos2,
    source: StrokeSource,
    eraser: bool,
    rect: Rect,
    pick: bool,
    shift: bool,
) -> bool {
    if crate::eyedrop::picks(app, pick) {
        crate::eyedrop::pick_canvas(app, view, p);
        return false;
    }
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
    begin_stroke(
        app,
        source,
        eraser,
        rect,
        shift || (app.drafting.snap && app.ruler().is_some()),
    )
}

fn begin_stroke(
    app: &mut AppState,
    source: StrokeSource,
    eraser: bool,
    rect: Rect,
    guided: bool,
) -> bool {
    if let Some(reason) = app.read_only_reason() {
        app.message = format!(
            "{}: {reason}",
            app.lang.pick(
                "読むだけのテクスチャセットには描けません",
                "Cannot paint on a read-only texture set"
            )
        );
        return false;
    }
    // クイックマスクが入っていれば、ブラシ・消しゴムは選択ペン・選択消しとして働く
    if let Some(began) = crate::selection::quick::begin(app, source, eraser) {
        return began;
    }
    let Some(layer) = app.selected_layer else {
        app.message = app
            .lang
            .pick("描くレイヤーがありません。", "No layer to paint on.")
            .into();
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
            app.message = format!(
                "{}: {}",
                app.lang.pick("描けません", "Cannot paint"),
                app.lang.core_error(&e)
            );
            return false;
        }
    };
    let result = if guided {
        app.begin_guided_canvas_stroke(layer, eraser, stencil)
    } else {
        app.begin_canvas_stroke(layer, eraser, stencil)
    };
    match result {
        Ok(stroke) => {
            app.stroke = Some(stroke);
            app.canvas.stroke = Some(source);
            app.canvas.stroke_points = 0;
            app.canvas.stroke_time = None;
            app.canvas.current_end = None;
            app.canvas.shift_hold = None;
            app.canvas.ruler_constraint = None;
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
    if app.region.leftover_drag.is_some() { crate::region::bucket::drag(app, view, p); return; }
    if app.region.drag.is_some() {
        crate::region::tools::drag_to(app, crate::region::tools::Where::Canvas(view), p);
        return;
    }
    if crate::selection::quick::add_point(app, view, p, pressure) {
        return;
    }
    let (mut x, mut y) = view.to_canvas(p);
    if let Some(mut hold) = app.canvas.shift_hold {
        let (ox, oy) = hold.origin;
        if hold.direction.is_none() && view.to_screen(ox, oy).distance(p) < SHIFT_HOLD_POINTS {
            // 押した点のぶれ（画面の点で数画素まで）は、向きも点も動かさない
            (x, y) = (ox, oy);
        } else if hold.locks {
            let (dx, dy) = (x - ox, y - oy);
            let (ux, uy) = *hold.direction.get_or_insert_with(|| {
                let angle = (dy.atan2(dx) / std::f64::consts::FRAC_PI_4).round()
                    * std::f64::consts::FRAC_PI_4;
                (angle.cos(), angle.sin())
            });
            let length = dx * ux + dy * uy;
            (x, y) = (ox + length * ux, oy + length * uy);
            app.canvas.shift_hold = Some(hold);
        } else {
            // 前の終点からの線は押した点で終わっている。ぶれを超えて動いたら、続きは普通に描く
            app.canvas.shift_hold = None;
        }
    }
    if let Some(constraint) = app.canvas.ruler_constraint.as_mut() {
        let point = constraint.project(yolu_core::glam::DVec2::new(x, y));
        (x, y) = (point.x, point.y);
    }
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
            app.canvas.current_end = Some((x, y));
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
    app.canvas.shift_hold = None;
    app.canvas.ruler_constraint = None;
    let endpoint = app.canvas.current_end.take();
    if crate::region::tools::finish_drag(app, cancel) {
        return;
    }
    if crate::selection::quick::finish(app, cancel) {
        return;
    }
    let Some(stroke) = app.stroke.take() else {
        // 札を失っていても、core に進行中のストロークが残っていれば取り消す（取り残さない）
        app.doc.cancel_active_stroke();
        return;
    };
    if cancel {
        app.doc.cancel_stroke(stroke);
        app.message = app
            .lang
            .pick("ストロークを取り消しました。", "Stroke cancelled.")
            .into();
    } else if let Err(e) = app.doc.end_stroke(stroke) {
        app.message = app.lang.core_error(&e);
    } else if endpoint.is_some() {
        app.canvas.previous_end = endpoint;
    }
}

/// 最初の点と、Shift のクリックから前の終点をつなぐ線。同じ筆圧で既存ストロークへ渡す。
#[allow(clippy::too_many_arguments)]
fn first_point(
    app: &mut AppState,
    view: &CanvasView,
    p: Pos2,
    pressure: f32,
    tilt: Tilt,
    rotation: Option<f32>,
    time: f64,
    shift: bool,
) {
    if app.drafting.snap && app.stroke.is_some() {
        let (x, y) = view.to_canvas(p);
        app.canvas.ruler_constraint = app
            .ruler()
            .map(|r| r.constraint(yolu_core::glam::DVec2::new(x, y)));
    }
    if shift && app.tool.paints() && app.stroke.is_some() {
        let has_previous = app.canvas.previous_end.is_some();
        if let Some((x, y)) = app.canvas.previous_end {
            add_point(
                app,
                view,
                view.to_screen(x, y),
                pressure,
                tilt,
                rotation,
                time,
            );
        }
        add_point(app, view, p, pressure, tilt, rotation, time);
        app.canvas.shift_hold = Some(ShiftHold {
            origin: view.to_canvas(p),
            locks: !has_previous,
            direction: None,
        });
    } else {
        add_point(app, view, p, pressure, tilt, rotation, time);
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

/// このフレームの入力の前提（押しを始めてよいか・修飾キー・時刻）。
struct Frame {
    /// 押しを始めてはいけない（ポップアップ・窓・ドックのタブの見出しをつかんでいる・押しがほかの部品のもの）。
    no_press: bool,
    modifiers: Modifiers,
    now: f64,
}

/// ペンの 1 点。触れた最初の点で行き先を決め（`press_kind`）、離すまで変えない。Shift は直線、Ctrl とサイドボタンは描かない。
fn pen_sample(ui: &Ui, app: &mut AppState, rect: Rect, s: &PenSample, frame: &Frame) {
    crate::selection::canvas::note_pen(app, s.pointer_id, s.pressure, s.eraser);
    let p = s.pos_points(ui.ctx().pixels_per_point());
    let (w_px, h_px) = (app.doc.width(), app.doc.height());
    let view = app.view.view(rect, w_px, h_px);
    let source = StrokeSource::Pen(s.pointer_id);
    let press = match app.canvas.pen_press {
        Some(press) if press.id == s.pointer_id => press,
        // ほかのペン（別の ID）の押しが続いている間は、この点を使わない
        Some(_) => return,
        None if s.contact => {
            let kind = press_kind(ui, app, rect, p, s, frame);
            app.canvas.pen_press = Some(PenPress {
                id: s.pointer_id,
                kind,
                last: p,
            });
            match kind {
                PressKind::Ignored => {}
                PressKind::View => {
                    nav::press(app, rect, p, &frame.modifiers);
                }
                PressKind::Tool if drives_pen(app) => {
                    drive_pen(app, &view, p, s.pointer_id, true, frame);
                }
                PressKind::Tool => {
                    if begin_any(
                        app,
                        &view,
                        p,
                        source,
                        s.eraser,
                        rect,
                        crate::keymap::picks(&frame.modifiers),
                        frame.modifiers.shift,
                    ) {
                        first_point(
                            app,
                            &view,
                            p,
                            s.pressure,
                            s.tilt,
                            s.rotation,
                            s.time_ms as f64 / 1000.0,
                            frame.modifiers.shift,
                        );
                    }
                }
            }
            return;
        }
        // 浮いているだけ
        None => return,
    };
    if s.contact {
        match press.kind {
            PressKind::Ignored => {}
            PressKind::View => nav::moved(app, rect, p, press.last, frame.modifiers.shift),
            PressKind::Tool => {
                // 描いている・選択の形を作っているときだけ続ける（道具を途中で替えても、始めた側を終わらせる）
                if app.canvas.stroke == Some(source) {
                    add_point(
                        app,
                        &view,
                        p,
                        s.pressure,
                        s.tilt,
                        s.rotation,
                        s.time_ms as f64 / 1000.0,
                    );
                } else {
                    drive_pen(app, &view, p, s.pointer_id, true, frame);
                }
            }
        }
        app.canvas.pen_press = Some(PenPress { last: p, ..press });
    } else {
        match press.kind {
            PressKind::Ignored => {}
            PressKind::View => nav::released(app, rect),
            PressKind::Tool => {
                if app.canvas.stroke == Some(source) {
                    finish_stroke(app, false);
                }
                drive_pen(app, &view, p, s.pointer_id, false, frame);
            }
        }
        app.canvas.pen_press = None;
    }
}

/// ペンを押す・動く・離すとして渡す道具（ドラッグの札を持つ道具。道具の表の `canvas`）。
fn drives_pen(app: &AppState) -> bool {
    app.tool.def().canvas.is_some()
}

/// ドラッグの札を持つ道具（選択・移動と変形・グラデーション・図形と定規・パス）のペン（触れる・動く・離すを、押す・動く・離すにする）。押しの始めは今の道具へ、
/// 続きと離すは始めた側へ（途中で道具を替えても、始めた側を終わらせる）。
fn drive_pen(
    app: &mut AppState,
    view: &CanvasView,
    p: Pos2,
    id: u32,
    contact: bool,
    frame: &Frame,
) {
    let ctx = InputCtx {
        modifiers: frame.modifiers,
        now: frame.now,
        rect: app.canvas_rect.unwrap_or(Rect::NOTHING),
        pass: 0,
    };
    let starting = contact && !CanvasKind::ALL.iter().any(|k| k.handler().pen_active(app, id));
    let current = app.tool.def().canvas;
    for kind in CanvasKind::ALL {
        let handler = kind.handler();
        if handler.pen_active(app, id) || (starting && current == Some(kind)) {
            handler.pen(app, view, p, id, contact, &ctx);
        }
    }
}

/// ペンが触れた最初の点の行き先。ビューを動かす（R・Space・Ctrl+Space）・何もしない（押した所が別の部品・ステンシルを動かしている間・
/// サイドボタン・Ctrl を押したブラシと消しゴム）・道具。Alt を押した道具は道具のまま（`begin_any` が、Alt のスポイトとして値を取って、
/// 描き始めない）。ステンシルを動かす押しは、同じ押しの egui のポインタの代わりの入力をステンシルが取るので、ビューを動かす判定より先に
/// 手放す（マウスの押しと同じく、ステンシルだけが動く）。
fn press_kind(
    ui: &Ui,
    app: &AppState,
    rect: Rect,
    p: Pos2,
    s: &PenSample,
    frame: &Frame,
) -> PressKind {
    if frame.no_press || !on_top(ui, rect, p) || app.stencil.handling() {
        return PressKind::Ignored;
    }
    if (app.canvas.rotate_key_held || app.canvas.space_held) && !app.is_stroking() {
        return PressKind::View;
    }
    // 2D には右ボタンの操作が無い。ペンのサイドボタンは、描かない
    if s.barrel {
        return PressKind::Ignored;
    }
    let m = &frame.modifiers;
    if app.tool.paints() && gesture::ctrl(m) {
        return PressKind::Ignored;
    }
    PressKind::Tool
}

/// 文字の入力欄が前のフレームにあったかを覚える場所。
fn typing_id() -> egui::Id {
    egui::Id::new("yolu.canvas.typing")
}

/// 選択範囲を持っていて、Esc を使うものが無いか（Esc で選択を解除してよいか）。Esc を自分の操作に使うものを優先する: メニューなどの
/// ポップアップ・確かめの窓・開いている浮いた窓・つまみやドラッグの途中（ボタンを押している間）・塗りつぶしの仕事・色の名前の変更。
/// キャンバスより前に描く部品やフレームの頭の処理が、このフレームの Esc でもうやめた（状態がもう空になっている）ものは、
/// `note_escape_taken` の印で見る。
/// 文字の入力中と、描く・形を作る・移動と変形・グラデーション・図形・パスなどの途中は、呼ぶ側が先に見る（やめるものがあればそちらが先）。
fn escape_is_free(app: &AppState, ctx: &egui::Context) -> bool {
    app.doc.selection().is_some()
        && app.popup.is_none()
        && !app.popup_was_open
        && app.sel.dialog.is_none()
        && !crate::windows::modal_open(app)
        && !crate::ui::window::any_open(ctx)
        && !crate::ui::window::escape_taken(ctx)
        && !ctx.input(|i| i.pointer.any_down())
        && app.region.job.is_none()
        && app.colorsets.rename.is_none()
        && app.colorsets.dragging.is_none()
        && app.brushes.ui.drag.is_none()
}

fn handle_input(ui: &mut Ui, app: &mut AppState, rect: Rect, pen: &[PenSample], foreign: bool) {
    let (w_px, h_px) = (app.doc.width(), app.doc.height());
    let ctx = ui.ctx().clone();
    let typing = ctx.egui_wants_keyboard_input();
    // 文字の入力欄は Esc を、入力をやめるのに使う。欄がこのフレームのこれより前に Esc で手放したときも、1 つ前のフレームで入力中だったかで分かる
    let typed_last = ctx.data_mut(|d| d.get_temp::<bool>(typing_id()).unwrap_or(false));
    ctx.data_mut(|d| d.insert_temp(typing_id(), typing));
    let (now, frame_dt) = ctx.input(|i| (i.time, i.unstable_dt as f64));
    let blocked = app.popup.is_some() || app.popup_was_open || app.sel.dialog.is_some();
    let (events, modifiers, r_down, space_down) = ui.input(|i| {
        (
            i.events.clone(),
            i.modifiers,
            i.key_down(crate::keymap::VIEW_ROTATE),
            i.key_down(crate::keymap::VIEW_PAN),
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
    // 押しを始めてよいか: ポップアップのほか、ドックのタブの見出しをつかんでいる間・離した直後と、押しがほかの部品（分け目・隅のアイコン）の
    // ものであるときも、描き始めも回し始めもしない
    let frame = Frame {
        no_press: blocked || app.dock_grabbed() || foreign,
        modifiers,
        now,
    };

    // ペン（Windows Ink）。ペンの押し（触れてから離すまで）は、ペンの点だけで扱い、同じ押しが egui のポインタの押しとして二重に来ても使わない。
    let pen_frame = app.canvas.pen_press.is_some() || pen.iter().any(|s| s.contact);
    for s in pen {
        pen_sample(ui, app, rect, s, &frame);
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
                    // 指・ペンの Touch の力も、ペンの点と同じ全体の調整を通す（マウスは 1 のまま）
                    app.canvas.touch_pressure = Some(app.adjust_pressure(f.clamp(0.0, 1.0)));
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
                        if frame.no_press || !on_top(ui, rect, pos) {
                            continue;
                        }
                        if pen_frame {
                            // ペンの押しは、ペンの点が持つ（これは同じ押しの egui のポインタの代わりの入力）
                            continue;
                        }
                        if !app.is_stroking() && nav::press(app, rect, pos, event_modifiers) {
                            // R・Space・Ctrl+Space を押しながらの左ドラッグ: 回す・パン・拡縮
                        } else if let Some(kind) = app.tool.def().canvas {
                            // ドラッグの札を持つ道具（選択・移動と変形・グラデーション・図形と定規・パス）
                            let handler = kind.handler();
                            if !handler.respects_stencil() || !app.stencil.handling() {
                                let view = app.view.view(rect, w_px, h_px);
                                let ctx = InputCtx {
                                    modifiers: *event_modifiers,
                                    now,
                                    rect,
                                    pass: ctx.cumulative_pass_nr(),
                                };
                                handler.press(app, &view, pos, StrokeSource::Mouse, &ctx);
                            }
                        } else if app.canvas.stroke.is_none() && !app.stencil.handling() && {
                            let view = app.view.view(rect, w_px, h_px);
                            begin_any(
                                app,
                                &view,
                                pos,
                                StrokeSource::Mouse,
                                false,
                                rect,
                                crate::keymap::picks(event_modifiers),
                                event_modifiers.shift,
                            )
                        } {
                            let view = app.view.view(rect, w_px, h_px);
                            first_point(
                                app,
                                &view,
                                pos,
                                app.canvas.touch_pressure.unwrap_or(1.0),
                                Tilt::default(),
                                None,
                                time,
                                event_modifiers.shift,
                            );
                        }
                    }
                    (PointerButton::Primary, false) => {
                        // 離した: ドラッグを始めた側が終わらせる（道具を替えていても）
                        let ctx = InputCtx {
                            modifiers: *event_modifiers,
                            now,
                            rect,
                            pass: ctx.cumulative_pass_nr(),
                        };
                        for kind in CanvasKind::ALL {
                            let handler = kind.handler();
                            if handler.dragging(app, Some(StrokeSource::Mouse)) {
                                let view = app.view.view(rect, w_px, h_px);
                                handler.release(app, &view, pos, StrokeSource::Mouse, &ctx);
                            }
                        }
                        if app.canvas.stroke == Some(StrokeSource::Mouse) {
                            finish_stroke(app, false);
                        }
                        if !pen_frame {
                            nav::released(app, rect);
                        }
                    }
                    (PointerButton::Middle, true) => {
                        if !frame.no_press && !pen_frame && on_top(ui, rect, pos) {
                            // 中ボタン: パン、Shift を足すと回転（`keymap::GESTURES`）
                            if crate::keymap::gesture("canvas", PointerButton::Middle, &modifiers, false)
                                == Some(crate::keymap::Operation::Rotate)
                            {
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
                if !pen_frame {
                    let move_ctx = InputCtx {
                        modifiers,
                        now,
                        rect,
                        pass: ctx.cumulative_pass_nr(),
                    };
                    for kind in CanvasKind::ALL {
                        let handler = kind.handler();
                        if handler.wants_move(app) {
                            let view = app.view.view(rect, w_px, h_px);
                            handler.moved(app, &view, pos, StrokeSource::Mouse, &move_ctx);
                        }
                    }
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
                // ペンの押しの回す・パン・拡縮は、ペンの点が動かす
                if !pen_frame {
                    nav::moved(app, rect, pos, previous, modifiers.shift);
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
                let ctx = InputCtx {
                    modifiers,
                    now,
                    rect,
                    pass: ctx.cumulative_pass_nr(),
                };
                // 図形・移動と変形・パスは、ストロークや表示の回転より先に。グラデーション・選択は、それらがなければ
                let cancelled = |app: &mut AppState, first: bool| {
                    CanvasKind::ALL
                        .iter()
                        .filter(|k| k.handler().cancel_first() == first)
                        .any(|k| k.handler().cancel(app, &ctx))
                };
                if cancelled(app, true) {
                    // 図形と定規は離すまで画素・定規を変更しない。移動・変形のドラッグは何も変えずにやめた。
                    // パスの点のドラッグを捨てた（ドラッグが無ければ選んだ点を外した）
                } else if app.is_stroking() {
                    finish_stroke(app, true);
                } else if let Some(drag) = app.canvas.rotating.take() {
                    app.view.angle = drag.start_angle;
                    app.view.pan = drag.start_pan;
                } else if !cancelled(app, false) && !typing && !typed_last && escape_is_free(app, ui.ctx()) {
                    // やめるものが無かった: 選択範囲があれば解除（Ctrl+D と同じ。1 回の取り消し）
                    app.apply(crate::state::Action::Sel(crate::selection::SelAction::Edit(
                        crate::selection::SelEdit::Clear,
                    )));
                }
            }
            Event::Key {
                key: key @ (Key::Enter | Key::Backspace),
                pressed: true,
                modifiers: event_modifiers,
                ..
            } if !typing && !blocked => {
                // ドラッグの途中の Enter（移動・変形）、多角形の確定・最後の点（選択）
                for kind in CanvasKind::ALL {
                    if kind.handler().key(app, *key, *event_modifiers) {
                        break;
                    }
                }
            }
            Event::WindowFocused(false) => {
                // フォーカスを失ったら、そこまでを確定する（離したのを受け取れないので）。選択の途中の形は捨てる。移動と変形・グラデーション・図形は
                // 何も変えずにやめる
                finish_stroke(app, false);
                for kind in CanvasKind::ALL {
                    kind.handler().focus_lost(app);
                }
                app.canvas.pen_press = None;
                nav::cancel(app);
                app.canvas.rotate_key_held = false;
            }
            _ => {}
        }
    }
    let frame_ctx = InputCtx {
        modifiers,
        now,
        rect,
        pass: ctx.cumulative_pass_nr(),
    };
    for kind in CanvasKind::ALL {
        kind.handler().each_frame(app, &frame_ctx);
    }
    // ボタンを離したのを取りこぼしたとき（窓の外で離したなど）も、押していなければ終える。ストローク・ドラッグの札を持つ道具のドラッグは、
    // 最後の位置で終える（道具ごとの終わらせ方は受け口が決める）
    let released = !ui.input(|i| i.pointer.primary_down())
        && !events
            .iter()
            .any(|e| matches!(e, Event::PointerButton { pressed: true, .. }));
    if released {
        if app.canvas.stroke == Some(StrokeSource::Mouse) {
            finish_stroke(app, false);
        }
        for kind in CanvasKind::ALL {
            let handler = kind.handler();
            if handler.dragging(app, Some(StrokeSource::Mouse)) {
                let view = app.view.view(rect, w_px, h_px);
                let at = app.canvas.last_pointer.unwrap_or(rect.center());
                handler.lost_release(app, &view, at, &frame_ctx);
            }
        }
    }
    // ペンが回す・拡縮している間は、egui のポインタが押していなくても続ける（ペンが離したときに終える）
    if !ui.input(|i| i.pointer.primary_down()) && !nav::pen_driven(app) {
        app.canvas.rotating = None;
        app.canvas.zooming = None;
    }
    crate::stencil::settle(app, ui.input(|i| i.pointer.any_down()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_line_is_one_undo_and_remembers_document_coordinates() {
        let mut app = AppState::new(64, 64);
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(256.0, 256.0));
        app.view.angle = 35.0;
        app.view.flip = true;
        let view = app.view.view(rect, 64, 64);
        app.brush.radius = 2.0;
        let a = view.to_screen(10.0, 10.0);
        let b = view.to_screen(40.0, 40.0);
        assert!(begin_stroke(
            &mut app,
            StrokeSource::Mouse,
            false,
            rect,
            true
        ));
        first_point(&mut app, &view, a, 0.7, Tilt::default(), None, 1.0, false);
        finish_stroke(&mut app, false);
        let before = app.doc.revision();
        assert!(begin_stroke(
            &mut app,
            StrokeSource::Mouse,
            false,
            rect,
            true
        ));
        first_point(&mut app, &view, b, 0.7, Tilt::default(), None, 2.0, true);
        assert_eq!(app.canvas.stroke_points, 2);
        finish_stroke(&mut app, false);
        let end = app.canvas.previous_end.unwrap();
        assert!((end.0 - 40.0).abs() < 1e-4 && (end.1 - 40.0).abs() < 1e-4);
        assert_ne!(before, app.doc.revision());
        app.doc.undo().unwrap();
        app.doc.undo().unwrap();
        assert!(!app.doc.can_undo());
        app.document_replaced();
        assert_eq!(app.canvas.previous_end, None);
    }

    /// 縮小して見ている（画面の 1 点が文書の 2 画素）文書と枠。
    fn zoomed_out() -> (AppState, Rect) {
        let app = AppState::new(512, 512);
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(256.0, 256.0));
        (app, rect)
    }

    #[test]
    fn shift_drag_locks_direction_after_the_hold() {
        let (mut app, rect) = zoomed_out();
        let view = app.view.view(rect, 512, 512);
        assert!(begin_stroke(
            &mut app,
            StrokeSource::Mouse,
            false,
            rect,
            true
        ));
        first_point(
            &mut app,
            &view,
            view.to_screen(100.0, 100.0),
            1.0,
            Tilt::default(),
            None,
            1.0,
            true,
        );
        // 画面の 1 点（文書の 2 画素）のぶれでは、向きも点も動かない
        add_point(
            &mut app,
            &view,
            view.to_screen(102.0, 101.0),
            1.0,
            Tilt::default(),
            None,
            1.1,
        );
        assert_eq!(app.canvas.current_end, Some((100.0, 100.0)));
        assert_eq!(app.canvas.shift_hold.unwrap().direction, None);
        // 保持を超えた最初の動きが 45 度刻みの向きを決める（ほぼ水平なので水平）
        add_point(
            &mut app,
            &view,
            view.to_screen(200.0, 120.0),
            1.0,
            Tilt::default(),
            None,
            1.2,
        );
        let end = app.canvas.current_end.unwrap();
        assert!(
            (end.0 - 200.0).abs() < 1e-3 && (end.1 - 100.0).abs() < 1e-3,
            "{end:?}"
        );
        add_point(
            &mut app,
            &view,
            view.to_screen(210.0, 300.0),
            1.0,
            Tilt::default(),
            None,
            1.3,
        );
        let end = app.canvas.current_end.unwrap();
        assert!((end.1 - 100.0).abs() < 1e-3, "{end:?}");
        finish_stroke(&mut app, true);
        assert_eq!(app.canvas.previous_end, None);
        assert!(!app.doc.can_undo());
    }

    #[test]
    fn shift_click_from_previous_end_stays_at_the_click_despite_jitter_in_a_zoomed_out_view() {
        let (mut app, rect) = zoomed_out();
        let view = app.view.view(rect, 512, 512);
        app.canvas.previous_end = Some((20.0, 20.0));
        assert!(begin_stroke(
            &mut app,
            StrokeSource::Mouse,
            false,
            rect,
            true
        ));
        first_point(
            &mut app,
            &view,
            view.to_screen(100.0, 60.0),
            1.0,
            Tilt::default(),
            None,
            1.0,
            true,
        );
        assert_eq!(app.canvas.stroke_points, 2);
        assert!(!app.canvas.shift_hold.unwrap().locks);
        // 画面の数点以内のぶれ（文書では十数画素）。終点も向きも動かない
        for (i, (x, y)) in [(103.0, 62.0), (96.0, 57.0), (108.0, 66.0), (92.0, 54.0)]
            .into_iter()
            .enumerate()
        {
            add_point(
                &mut app,
                &view,
                view.to_screen(x, y),
                1.0,
                Tilt::default(),
                None,
                1.1 + i as f64 * 0.1,
            );
            assert_eq!(app.canvas.current_end, Some((100.0, 60.0)), "{x},{y}");
        }
        finish_stroke(&mut app, false);
        let end = app.canvas.previous_end.unwrap();
        assert!(
            (end.0 - 100.0).abs() < 1e-3 && (end.1 - 60.0).abs() < 1e-3,
            "{end:?}"
        );
        assert_eq!(app.doc.undo_count(), 1);
        // 保持を超えて動かしたら、続きは向きを固定せずに普通に描く
        assert!(begin_stroke(
            &mut app,
            StrokeSource::Mouse,
            false,
            rect,
            true
        ));
        first_point(
            &mut app,
            &view,
            view.to_screen(200.0, 200.0),
            1.0,
            Tilt::default(),
            None,
            2.0,
            true,
        );
        add_point(
            &mut app,
            &view,
            view.to_screen(300.0, 260.0),
            1.0,
            Tilt::default(),
            None,
            2.1,
        );
        add_point(
            &mut app,
            &view,
            view.to_screen(330.0, 400.0),
            1.0,
            Tilt::default(),
            None,
            2.2,
        );
        let end = app.canvas.current_end.unwrap();
        assert!(
            (end.0 - 330.0).abs() < 1e-3 && (end.1 - 400.0).abs() < 1e-3,
            "{end:?}"
        );
        // 取り消したストロークの終点は覚えない（前の終点のまま）
        finish_stroke(&mut app, true);
        let kept = app.canvas.previous_end.unwrap();
        assert!(
            (kept.0 - 100.0).abs() < 1e-3 && (kept.1 - 60.0).abs() < 1e-3,
            "{kept:?}"
        );
        assert_eq!(app.doc.undo_count(), 1);
    }

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
