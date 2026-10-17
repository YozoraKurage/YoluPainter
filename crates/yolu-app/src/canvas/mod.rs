//! 2D キャンバスのタブ: 合成の絵（`display`）、表示（拡大・パン・回転・反転。写しは `view`）、ブラシのカーソル、入力。
//! 入力はフレームの中の生のイベントを順に見る（1 フレームに来たマウスの移動を全部ストロークの点にする）。ペン（Windows Ink）の
//! 点が来ていれば、そのフレームのストロークはペンの点だけで描き、同じペンから egui が作るマウスの代わりの入力は使わない。
//! ストロークを取り残さない: ボタンを離す・Esc（捨てる）・ウィンドウのフォーカスを失う（そこまでを確定）で必ず終える。

pub mod cpu;
pub mod display;
pub mod gpu;
pub mod nav;
pub mod view;

use egui::{Color32, CursorIcon, Event, Key, Pos2, Rect, Sense, Stroke, Ui};

use self::display::CanvasDisplay;
use self::view::{angle_label, CanvasView};
use crate::engine::{BrushEffect, BrushSample, Tilt};
use crate::features::Feature;
use crate::gesture;
use crate::input::{on_top, Item, Owner, Phase, PointerSample, Press, PressCtx, Stream};
use crate::notice::Source;
use crate::pen::PenSample;
use crate::state::key::{Modifiers, PointerButton};
use crate::state::{AppState, ShiftHold, StrokeSource};
use crate::tools::input::{CanvasKind, InputCtx};
use crate::ui::theme as t;
use crate::ui::widgets as w;

/// ポインタの角度を測らない、表示域の中心からの距離。
const ROTATE_DEAD_ZONE: f32 = 4.0;

/// キャンバスのタブを描く。
pub fn show(ui: &mut Ui, app: &mut AppState, display: &mut CanvasDisplay, pen: &[PenSample]) {
    let rect = ui.max_rect();
    let response = ui.interact(rect, ui.id().with("canvas"), Sense::click_and_drag());
    app.ui.canvas_rect = Some(rect);
    app.ui.canvas_drawn = true;
    app.rulers
        .note_pointer(crate::rulers::Place::Canvas, response.contains_pointer());
    // メインウィンドウのフレームの番号（キャンバスを別ウィンドウへ出しても、キーを見るメインウィンドウの番号と比べられるように）
    app.ui.canvas_frame = Some(ui.ctx().cumulative_frame_nr_for(egui::ViewportId::ROOT));
    ui.advance_cursor_after_rect(rect);
    handle_input(
        ui,
        app,
        rect,
        pen,
        gesture::foreign_press(ui.ctx(), &response),
    );
    let (w_px, h_px) = (app.session.doc.width(), app.session.doc.height());
    let view = app.view.view(rect, w_px, h_px);
    display.set_document_epoch(app.session.doc_epoch());
    // 見えている所から上げる（CPU の頁。GPU の道は使わない）
    display.set_viewport(Some(self::cpu::Viewport {
        // 何も見えないとき（表示を画面の外へ動かした）は空の矩形。全部が「見えていない」として、近い順に少しずつ上げる
        visible: view
            .visible_doc_rect(rect)
            .unwrap_or(crate::engine::Rect::new(0, 0, 0, 0)),
        pixel_size: view.pixel_size(),
    }));
    display.sync_channel(ui.ctx(), &app.session.doc, app.m2.display_channel);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, t::CANVAS_BG);
    display.paint(&painter, &view);
    // 選択の縁・ドラッグ中の形・対称の軸
    crate::selection::canvas::paint_overlay(ui.ctx(), &painter, &view, app);
    // 移動・変形のツール: 動かすものの外枠とハンドル（ドラッグ中は変形後の外枠）
    crate::transform::canvas::paint_overlay(&painter, &view, app);
    // グラデーションのツール: ドラッグ中の線
    crate::gradient::canvas::paint_overlay(&painter, &view, app);
    crate::drafting::canvas::paint_overlay(&painter, &view, app);
    // テキストツール: 打っている文字の枠・カーソルと入力欄
    crate::textlayer::canvas::paint_overlay(ui, &painter, &view, app);
    // 選択範囲の下のボタンの帯（描いている間・選択の形を作っている間・表示を動かしている間は出ない）
    crate::selection::bar::show(ui, app, &view, rect);
    // 焼いたメッシュマップを見ているとき（読むだけの重ね表示）
    crate::bake::overlay::paint(&painter, app, &view);
    crate::uv_wireframe::show(ui, app, &view);
    // ステンシル（画面に貼り付いた半透明の画像。Y を押しているあいだは枠も）
    if app.mode.paints() {
        crate::stencil::draw_overlay(&painter, &mut app.stencil, rect);
    }
    // パスのツール: 選んでいるレイヤーの 2D のパスの線と点
    let hover_for_path = ui.input(|i| i.pointer.hover_pos());
    crate::pathtool::canvas::paint_overlay(
        &painter,
        &view,
        app,
        hover_for_path.filter(|p| rect.contains(*p) && response.contains_pointer()),
    );
    // 点のグラデーションの点（編集している間）
    crate::fillfx::points::paint_canvas(
        &painter,
        &view,
        app,
        hover_for_path.filter(|p| rect.contains(*p) && response.contains_pointer()),
    );

    // クローンの元の印
    paint_clone_source(&painter, &view, app);

    // ブラシのカーソル（回している・回すキーを押している・パンしている・ステンシルを動かしているあいだは出さない）
    let hover = ui.input(|i| i.pointer.hover_pos());
    let pointer_on_canvas = hover.is_some_and(|p| rect.contains(p)) && response.contains_pointer();
    // 範囲のツール: ポインタの下の範囲の UV の輪郭（回している・パンしているあいだは出さない）
    {
        let navigating = app.canvas.rotate_key_held
            || app.canvas.rotating.is_some()
            || app.canvas.panning
            || app.canvas.zooming.is_some()
            || app.canvas.space_held;
        let at = hover.filter(|_| pointer_on_canvas && !navigating && app.mode.paints());
        crate::region::overlay::paint_canvas(&painter, app, &view, at);
    }
    let busy = app.canvas.rotate_key_held || app.canvas.rotating.is_some();
    if pointer_on_canvas {
        if let Some(icon) = crate::stencil::cursor_icon(&app.stencil) {
            ui.ctx().set_cursor_icon(icon);
        } else if let Some(press) = app.canvas.eyedrop {
            // 右ボタンでスポイトの途中: 見本の輪とスポイトの絵（押したまま動かすと付いてくる）
            paint_eyedrop_mark(app, &painter, &view, press.at);
            ui.ctx().set_cursor_icon(CursorIcon::None);
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
        } else if !app.mode.paints() {
            // 編集・ポーズのモード: 描かないので、ブラシの円もツールの印も出さない
            ui.ctx().set_cursor_icon(CursorIcon::Default);
        } else if crate::eyedrop::picks(app) {
            // スポイトのツール: ポインタに見本の輪とスポイトの絵（OS の矢印は隠す）
            if let Some(p) = hover {
                paint_eyedrop_mark(app, &painter, &view, p);
            }
            ui.ctx().set_cursor_icon(CursorIcon::None);
        } else if let Some(icon) = app.tool.def().cursor.icon(app, &view, hover) {
            ui.ctx().set_cursor_icon(icon);
        } else if let Some(p) = hover {
            let radius = (app.brush.radius * view.pixel_size()).max(1.5);
            painter.circle_stroke(p, radius, Stroke::new(3.0, Color32::from_black_alpha(140)));
            painter.circle_stroke(p, radius, Stroke::new(1.2, Color32::from_white_alpha(230)));
            let model = app.canvas_model_symmetry();
            crate::selection::canvas::paint_mirrored_cursors(
                &painter,
                &view,
                app,
                model.as_deref(),
                p,
                radius,
            );
            ui.ctx().set_cursor_icon(if radius >= 4.0 {
                CursorIcon::None
            } else {
                CursorIcon::Crosshair
            });
        }
    }
    draw_corner(ui, app, rect);
}

/// クローンの元の印（十字）。描いていないあいだは、決めた元の点（クローンのブラシのときだけ）。描いている間は、今写している元の点
/// （描く点 + offset）に動く。
fn paint_clone_source(painter: &egui::Painter, view: &CanvasView, app: &AppState) {
    let at = if app.canvas.stroke.is_some() {
        match (app.canvas.clone_offset, app.canvas.current_end) {
            (Some(offset), Some((x, y))) => Some((x + offset.x, y + offset.y)),
            _ => None,
        }
    } else if crate::clone_source::active(app) {
        app.clone.canvas_source_for(app.session.doc.id())
    } else {
        None
    };
    if let Some((x, y)) = at {
        crate::clone_source::paint_mark(painter, view.to_screen(x, y));
    }
}

/// スポイトの印（今の色｜ポインタの下の色の輪とスポイトの絵）を `at` に描く。ポインタの下の色は、同じ画素・同じ文書なら読み直さない。
fn paint_eyedrop_mark(app: &mut AppState, painter: &egui::Painter, view: &CanvasView, at: Pos2) {
    let sample = crate::eyedrop::sample_canvas(app, view, at);
    crate::eyedrop_mark::paint(painter, at, crate::eyedrop::current_swatch(app), sample);
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
                format!("{}: {reason}", lang.t("canvas-read-only-tip")),
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
                crate::shortcuts::named_with_keys(
                    lang,
                    &lang.f(
                        "canvas-the-view-is-rotated-click-tip",
                        &[("angle", &angle_label(app.view.angle))],
                    ),
                    &[crate::shortcuts::key_in("view.reset_rotation", app.mode)],
                ),
            )
            .enabled(enabled),
        );
        actions.push(Some(crate::state::Action::ResetRotation));
    }
    if app.view.flip {
        items.push(w::CornerIcon::new("flip", "flip", flip_tip(lang, app.mode)).enabled(enabled));
        actions.push(Some(crate::state::Action::FlipView));
    }
    // 焼いたメッシュマップを重ねて見ている: 名前（押し込まれた見た目。押すとやめる）
    if let Some(name) = crate::bake::overlay::view_name(app) {
        items.push(
            w::CornerIcon::new(
                "meshmap",
                "visibility",
                format!("{}: {name}", lang.t("canvas-mesh-map-tip")),
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

/// スポイトのツールは押した所の値を取るだけで、始めない。ストロークかドラッグを始めたら true。
#[allow(clippy::too_many_arguments)]
fn begin_any(
    app: &mut AppState,
    view: &CanvasView,
    p: Pos2,
    source: StrokeSource,
    eraser: bool,
    rect: Rect,
    shift: bool,
) -> bool {
    if crate::eyedrop::picks(app) {
        crate::eyedrop::pick_canvas(app, view, p);
        return false;
    }
    // ベイクのウィンドウでアイランドを選んでいる間は、押した所のアイランドを選ぶだけ（ツールを使わない）
    if crate::bake::overlap::press(app, crate::region::tools::Where::Canvas(view), p) {
        return false;
    }
    // 点のグラデーションの点を編集している間: 点を掴む・追加する（描かない）
    let points_source = match source {
        StrokeSource::Mouse => crate::fillfx::gizmo::Source::Mouse,
        StrokeSource::Pen(id) => crate::fillfx::gizmo::Source::Pen(id),
    };
    if crate::fillfx::points::canvas_press(app, view, p, points_source) {
        if crate::fillfx::points::dragging(app) {
            app.canvas.stroke = Some(source);
            app.canvas.stroke_points = 0;
        }
        return crate::fillfx::points::dragging(app);
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
    let first = stroke_start(app, view, p, shift);
    // 手ぶれ補正と曲線を切るのは、定規にスナップした決まった道だけ（3D ビューと同じ）。Shift の直線で始めたストロークも、続けてなぞる所には効かせる
    let snapped = app
        .canvas_ruler_constraint(view, app.session.selected_layer, p)
        .is_some();
    begin_stroke(app, source, eraser, rect, snapped, first)
}

/// ストロークの最初の点（文書の座標。`first_point` が最初に足す点と同じ）。Shift で前の終点があれば、そこから線を引くので前の終点、なければ
/// 押した点。定規のスナップを当てる。
fn stroke_start(app: &AppState, view: &CanvasView, p: Pos2, shift: bool) -> (f64, f64) {
    let pressed = view.to_canvas(p);
    let first = if shift && app.tool.paints() {
        app.canvas.previous_end.unwrap_or(pressed)
    } else {
        pressed
    };
    if let Some(mut constraint) = app.canvas_ruler_constraint(view, app.session.selected_layer, p) {
        let at = constraint.project(yolu_core::glam::DVec2::new(first.0, first.1));
        return (at.x, at.y);
    }
    first
}

/// 2D のストロークを始める。`first` は、ストロークの最初の点（文書の座標。クローンの元から offset を決めるのに使う）。
fn begin_stroke(
    app: &mut AppState,
    source: StrokeSource,
    eraser: bool,
    rect: Rect,
    guided: bool,
    first: (f64, f64),
) -> bool {
    // 確かめの順は `stroke_gate` の頭（3D ビューの `view3d::input::begin` と同じ）
    if let Err(reason) = crate::stroke_gate::read_only(app) {
        app.refuse(Source::Brush, reason);
        return false;
    }
    // クイックマスクが入っていれば、ブラシ・消しゴムは選択ペン・選択消しとして働く
    if let Some(began) = crate::selection::quick::begin(app, source, eraser) {
        return began;
    }
    let layer = match crate::stroke_gate::layer(app) {
        Ok(id) => id,
        Err(reason) => {
            app.refuse(Source::Brush, reason);
            return false;
        }
    };
    let settings = app.stroke_settings(eraser);
    if let Err(reason) = crate::stroke_gate::symmetry_effect(
        app,
        crate::rulers::Place::Canvas,
        layer,
        settings.erase,
    ) {
        app.refuse(Source::Brush, reason);
        return false;
    }
    // ステンシルの置き場はストロークの始めに決める（ストロークの間は変えない）
    let stencil = match app.canvas_stencil(rect) {
        Ok(s) => s,
        Err(e) => {
            app.notify(
                crate::notice::Kind::of_core(&e),
                Source::Brush,
                crate::stroke_gate::cannot_paint(app.lang, &app.lang.core_error(&e)),
            );
            return false;
        }
    };
    // クローン: 元を決めていれば、ストロークの始めの点で offset を決めてブラシに入れる（揃える・揃えないは `CloneState::canvas_offset`）。
    // 決めていなければ、ブラシの offset のまま写す
    let cloning = !eraser && crate::clone_source::active(app);
    let restore = cloning.then(|| (app.clone.clone(), app.m2.brush.effect));
    if let (true, BrushEffect::Clone { offset: current }) = (cloning, app.m2.brush.effect) {
        if let Some(offset) = app
            .clone
            .canvas_offset(app.session.doc.id(), first, current)
        {
            app.m2.brush.effect = BrushEffect::Clone { offset };
        }
    }
    let result = if guided {
        app.begin_guided_canvas_stroke(layer, eraser, stencil)
    } else {
        app.begin_canvas_stroke(layer, eraser, stencil)
    };
    if result.is_err() {
        // 始められなかったときは、決めた offset も戻す（次のストロークの始めで決め直す）
        if let Some((clone, effect)) = restore {
            app.clone = clone;
            app.m2.brush.effect = effect;
        }
    }
    match result {
        Ok(stroke) => {
            app.canvas.clone_offset = match app.m2.brush.effect {
                BrushEffect::Clone { offset } if cloning => Some(offset),
                _ => None,
            };
            app.session.stroke = Some(stroke);
            app.canvas.stroke = Some(source);
            app.canvas.stroke_points = 0;
            app.canvas.stroke_time = None;
            app.canvas.current_end = None;
            app.canvas.shift_hold = None;
            app.canvas.ruler_constraint = None;
            if !settings.erase {
                app.color.remember();
            }
            app.session.mark_content_modified();
            true
        }
        Err(e) => {
            app.notify(
                crate::notice::Kind::of_core(&e),
                Source::Brush,
                crate::stroke_gate::cannot_paint(app.lang, &app.lang.core_error(&e)),
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
    if app.region.leftover_drag.is_some() {
        crate::region::bucket::drag(app, view, p);
        return;
    }
    if app.region.drag.is_some() {
        crate::region::tools::drag_to(app, crate::region::tools::Where::Canvas(view), p);
        return;
    }
    if app.fillfx.point_drag.as_ref().is_some_and(|d| !d.in_3d) {
        crate::fillfx::points::canvas_drag(app, view, p);
        return;
    }
    if crate::selection::quick::add_point(app, view, p, pressure) {
        return;
    }
    let (mut x, mut y) = view.to_canvas(p);
    if let Some(mut hold) = app.canvas.shift_hold {
        // 押した点のぶれ（画面の点で数画素まで）は、向きも点も動かさない。固定する押しは、超えて動いた向きを 45° 刻みで固定する。
        // 前の終点からの線は押した点で終わっているので、ぶれを超えて動いたら、続きは普通に描く
        let (ox, oy) = hold.origin;
        let near = view.to_screen(ox, oy).distance(p) < crate::state::SHIFT_HOLD_POINTS as f32;
        match hold.constrain((x, y), near) {
            Some(point) => {
                (x, y) = point;
                app.canvas.shift_hold = Some(hold);
            }
            None => app.canvas.shift_hold = None,
        }
    }
    if let Some(constraint) = app.canvas.ruler_constraint.as_mut() {
        let point = constraint.project(yolu_core::glam::DVec2::new(x, y));
        (x, y) = (point.x, point.y);
    }
    let Some(stroke) = app.session.stroke.as_mut() else {
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
        Ok(s) => stroke.add_sample(&mut app.session.doc, s),
        Err(e) => {
            app.session.doc.cancel_active_stroke();
            Err(e)
        }
    };
    match result {
        Ok(()) => {
            app.canvas.current_end = Some((x, y));
            app.canvas.stroke_time = Some(time);
            app.canvas.stroke_points += 1;
            // 重なった UV に描いたら、セットごとに 1 度だけ知らせる（片側だけには描けない）
            if app.tool.paints() {
                let radius = app.brush.radius as f64;
                app.note_overlap_canvas(x, y, radius);
            }
        }
        Err(e) => {
            // core は失敗したストロークを取り消してから返す（予算を超えたなど）。札を手放して知らせる
            app.session.stroke = None;
            app.canvas.stroke = None;
            app.notify(
                crate::notice::Kind::of_core(&e),
                Source::Canvas,
                app.lang.core_error(&e),
            );
        }
    }
}

/// ストロークをその場で終える（cancel なら捨てる）。3D ビューのストロークは、持ち越したダブを全部塗ってから確定する。
pub fn finish_stroke(app: &mut AppState, cancel: bool) {
    finish_stroke_with(app, cancel, None);
}

/// ウィンドウのフォーカスを失ったとき、ストロークをそこまでで終える。3D ビューのストロークは、離したのと同じく、残りを時間の枠で塗ってから
/// 確定する（`view3d::input::release`。ctx はその間のフレームを頼む）。
fn release_stroke(app: &mut AppState, ctx: &egui::Context) {
    finish_stroke_with(app, false, Some(ctx));
}

fn finish_stroke_with(app: &mut AppState, cancel: bool, release: Option<&egui::Context>) {
    app.canvas.stroke = None;
    let cloned = app.canvas.clone_offset.take();
    app.canvas.shift_hold = None;
    app.canvas.ruler_constraint = None;
    let endpoint = app.canvas.current_end.take();
    // 3D ビューのストロークは 3D ビューの終わらせ方で（持ち越したダブと最後の区間を塗ってから確定する。クイックマスクも）
    if app.view3d.input.surface.is_some() || app.view3d.input.cover.is_some() {
        match release {
            Some(ctx) if !cancel => crate::view3d::input::release(app, ctx),
            _ => crate::view3d::input::finish(app, cancel),
        }
        return;
    }
    if crate::region::tools::finish_drag(app, cancel) {
        return;
    }
    if app.fillfx.point_drag.as_ref().is_some_and(|d| !d.in_3d) {
        crate::fillfx::points::release(app, !cancel);
        return;
    }
    if crate::selection::quick::finish(app, cancel) {
        return;
    }
    let Some(stroke) = app.session.stroke.take() else {
        // 札を失っていても、core に進行中のストロークが残っていれば取り消す（取り残さない）
        app.session.doc.cancel_active_stroke();
        return;
    };
    if cancel {
        app.session.doc.cancel_stroke(stroke);
        app.info(Source::Canvas, app.lang.t("canvas-stroke-cancelled"));
    } else {
        match app.session.doc.end_stroke(stroke) {
            Err(e) => app.notify(
                crate::notice::Kind::of_core(&e),
                Source::Canvas,
                app.lang.core_error(&e),
            ),
            Ok(result) => {
                // 3D の対称の写しが見つからなかったダブがあれば（確定のときに描いた待ちのダブも）、3D ビューと同じく知らせる
                if let Some(outcome) = result.copy_note {
                    app.warn(Source::Canvas, app.lang.mirror_note(outcome));
                }
                if endpoint.is_some() {
                    app.canvas.previous_end = endpoint;
                }
                // 揃えるクローンは、画素を変えて確定したストロークの offset で続ける（3D の先の基準と同じ）
                if let (Some(offset), true) = (cloned, result.changed) {
                    app.clone.canvas_stroke_kept(app.session.doc.id(), offset);
                }
            }
        }
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
    if app.session.stroke.is_some() {
        app.canvas.ruler_constraint =
            app.canvas_ruler_constraint(view, app.session.selected_layer, p);
    }
    if shift && app.tool.paints() && app.session.stroke.is_some() {
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
        if has_previous {
            // 直線は押した点まで遅れずにまっすぐ届かせる（手ぶれ補正の遅れを詰め、曲線の待たせている区間も描き切って、そこを角にする。
            // 続けてなぞる所には補正と曲線が効く。3D ビューと同じ）
            end_line(app);
        }
        app.canvas.shift_hold = Some(ShiftHold::new(view.to_canvas(p), has_previous));
    } else {
        add_point(app, view, p, pressure, tilt, rotation, time);
    }
}

/// 直線の終わり: 手ぶれ補正の筆を、最後に足した点まで描き、待たせている曲線の区間も描き切る（ストロークは続く）。失敗したら core が取り消した
/// ストロークの札を手放して知らせる（`add_point` と同じ）。
fn end_line(app: &mut AppState) {
    let Some(stroke) = app.session.stroke.as_mut() else {
        return;
    };
    if let Err(e) = stroke.end_line(&mut app.session.doc) {
        app.session.stroke = None;
        app.canvas.stroke = None;
        app.notify(
            crate::notice::Kind::of_core(&e),
            Source::Canvas,
            app.lang.core_error(&e),
        );
    }
}

/// このフレームの入力の前提（押しを始めてよいか・修飾キー・時刻）。
struct Frame {
    /// 押しを始めてはいけない（ポップアップ・ウィンドウ・ドックのタブの見出しをつかんでいる・押しがほかの部品のもの）。
    no_press: bool,
    modifiers: Modifiers,
    now: f64,
}

/// ペンの 1 点（`PointerSample::pen`）。触れた最初の点で行き先を決め（`press_kind`）、離すまで変えない。Shift は直線、Ctrl とサイドボタンは描かない。
fn pen_sample(ui: &Ui, app: &mut AppState, rect: Rect, s: &PointerSample, frame: &Frame) {
    let StrokeSource::Pen(id) = s.source else {
        return;
    };
    crate::selection::canvas::note_pen(app, id, s.pressure, s.eraser);
    let p = s.pos;
    let (w_px, h_px) = (app.session.doc.width(), app.session.doc.height());
    let view = app.view.view(rect, w_px, h_px);
    let source = s.source;
    let press = match s.phase {
        // 浮いているだけ・ほかのペン（別の ID）の押しが続いている間は、この点を使わない
        Phase::Idle { .. } => return,
        Phase::Move | Phase::Release { .. } => match app.canvas.pen_press {
            Some(press) => press,
            None => return,
        },
        Phase::Press => {
            // 3D ビューで離した後の残りを塗っている途中（確定待ち）の押し: 先に確定してから、この押しを受ける
            if !frame.no_press && on_top(ui, rect, p) {
                crate::view3d::input::settle(app);
            }
            let kind = press_kind(ui, app, rect, p, s, frame);
            crate::pen::log::event(|| {
                crate::pen::log::press_line(
                    crate::pen::log::Site {
                        view: "canvas",
                        src: "pen",
                        frame: ui.ctx().cumulative_frame_nr(),
                        pen_frame: true,
                    },
                    &format!("{kind:?}"),
                    p,
                    &format!("{:?}", s.button),
                    false,
                )
            });
            if kind == Owner::Tool && app.mode.paints() {
                app.rulers.last_drew = Some(crate::rulers::Place::Canvas);
            }
            let button = s.button;
            app.canvas.pen_press = Some(Press {
                source,
                owner: kind,
                button,
                last: p,
            });
            match kind {
                Owner::Ignored => {}
                Owner::View => {
                    let start = nav::start_of(app, button, &frame.modifiers);
                    nav::start(app, rect, p, button, start, &frame.modifiers);
                    let click = nav::click_of(app, button, &frame.modifiers);
                    nav::note_click(app, p, button, click);
                }
                Owner::Eyedrop => {
                    crate::eyedrop::right_begin(app, source, p, button);
                }
                Owner::Tool if drives_pen(app) => {
                    app.sel.press_button = button;
                    drive_pen(app, &view, p, id, true, frame);
                }
                Owner::Tool => {
                    if begin_any(app, &view, p, source, s.eraser, rect, frame.modifiers.shift) {
                        first_point(
                            app,
                            &view,
                            p,
                            s.pressure,
                            s.tilt,
                            s.rotation,
                            s.time,
                            frame.modifiers.shift,
                        );
                    }
                }
            }
            return;
        }
    };
    if let Phase::Release { lost } = s.phase {
        // OS に押しを奪われて補った離し（本物の離しではない）は、マウスの取りこぼしと同じに扱う: 離しの操作（クリックの拡縮・クローンの元）はしない、スポイトは色を取らない、
        // 離した位置が要るツールは取りやめる。ストロークは今までどおり、そこまでを終える
        match press.owner {
            Owner::Ignored => {}
            Owner::View if lost => nav::cancel(app),
            Owner::View => nav::released(app, rect, p, s.button),
            Owner::Eyedrop => {
                let inside = !lost && on_top(ui, rect, p);
                crate::eyedrop::right_end(app, &view, source, p, inside);
            }
            Owner::Tool => {
                if app.canvas.stroke == Some(source) {
                    finish_stroke(app, false);
                }
                if lost {
                    lost_pen(app, &view, p, id, frame);
                } else {
                    drive_pen(app, &view, p, id, false, frame);
                }
            }
        }
        app.canvas.pen_press = None;
    } else {
        match press.owner {
            Owner::Ignored => {}
            Owner::View => {
                crate::pen::log::event(|| {
                    crate::pen::log::move_line(
                        crate::pen::log::Site {
                            view: "canvas",
                            src: "pen",
                            frame: ui.ctx().cumulative_frame_nr(),
                            pen_frame: true,
                        },
                        p,
                        press.last,
                        nav_name(app),
                        true,
                    )
                });
                nav::moved(app, rect, p, press.last, frame.modifiers.shift)
            }
            Owner::Eyedrop => crate::eyedrop::right_move(app, source, p),
            Owner::Tool => {
                // 描いている・選択の形を作っているときだけ続ける（ツールを途中で替えても、始めた側を終わらせる）
                if app.canvas.stroke == Some(source) {
                    add_point(app, &view, p, s.pressure, s.tilt, s.rotation, s.time);
                } else {
                    drive_pen(app, &view, p, id, true, frame);
                }
            }
        }
        app.canvas.pen_press = Some(Press { last: p, ..press });
    }
}

/// ペンを押す・動く・離すとして渡すツール（ドラッグの札を持つツール。ツールの表の `canvas`）。
fn drives_pen(app: &AppState) -> bool {
    app.tool.def().canvas.is_some()
}

/// ペンの点を渡すときの入力の前提。
fn pen_input_ctx(app: &AppState, frame: &Frame) -> InputCtx {
    InputCtx {
        modifiers: frame.modifiers,
        now: frame.now,
        rect: app.ui.canvas_rect.unwrap_or(Rect::NOTHING),
        pass: 0,
    }
}

/// ペンの押しを OS に奪われて離しが補われたとき、そのペンで押している途中のツールの終わらせ方（`CanvasTool::pen_lost`）。
fn lost_pen(app: &mut AppState, view: &CanvasView, p: Pos2, id: u32, frame: &Frame) {
    let ctx = pen_input_ctx(app, frame);
    for kind in CanvasKind::ALL {
        let handler = kind.handler();
        if handler.pen_active(app, id) {
            handler.pen_lost(app, view, p, id, &ctx);
        }
    }
}

/// ドラッグの札を持つツール（選択・移動と変形・グラデーション・図形と定規・パス）のペン（触れる・動く・離すを、押す・動く・離すにする）。押しの始めは今のツールへ、
/// 続きと離すは始めた側へ（途中でツールを替えても、始めた側を終わらせる）。
fn drive_pen(
    app: &mut AppState,
    view: &CanvasView,
    p: Pos2,
    id: u32,
    contact: bool,
    frame: &Frame,
) {
    let ctx = pen_input_ctx(app, frame);
    let starting = contact
        && !CanvasKind::ALL
            .iter()
            .any(|k| k.handler().pen_active(app, id));
    let current = app.tool.def().canvas;
    for kind in CanvasKind::ALL {
        let handler = kind.handler();
        if handler.pen_active(app, id) || (starting && current == Some(kind)) {
            handler.pen(app, view, p, id, contact, &ctx);
        }
    }
}

/// ペンが触れた最初の点の行き先（`input::owner`。マウスの押しと同じ表）。描くツールの Ctrl は描かない。
fn press_kind(
    ui: &Ui,
    app: &AppState,
    rect: Rect,
    p: Pos2,
    s: &PointerSample,
    frame: &Frame,
) -> Owner {
    let holds_off = gesture::pen_holds_off(app.tool.paints(), &frame.modifiers.into());
    crate::input::owner(&press_ctx(
        app,
        s.button,
        &frame.modifiers,
        frame.no_press,
        on_top(ui, rect, p),
        holds_off,
    ))
}

/// 2D のキャンバスの押しの事情（マウスもペンも。サイドボタンは右ボタン）。実際のボタン・修飾・押しながらのキーで、ドラッグの操作と離しの操作を
/// 引く（`nav::start_of`・`nav::click_of`。選択のツールの選択の行は、左以外のボタンでもツールのボタン）。編集・ポーズのモードは見るだけ。
/// `busy`（押しを受けない間）は、ボタンを押している最中の物だけ: ストローク（`has_active_stroke`・`canvas.stroke`）・移動と変形・パス・図形・
/// 定規のドラッグ・範囲の離した後の残りのドラッグ（`leftover_drag`）・3D ビューのストローク・表示を回している・パンしている・拡縮している。
/// 範囲の塗りの裏の仕事（`region.job`。ボタンを離した後も続く）は入れない: 入れると、塗りの間の押しがどのツールでも黙って捨てられる
/// （バケツは `refuse_busy` が断りを出す）。3D の `busy`（ビューを動かしている・ストローク・図形・選択の形）との違いは、2D のツールのドラッグ
/// （移動と変形・パス・定規など）を含むこと: 2D は前から `is_stroking` の間を `begin_any` が断っていて、その範囲を変えないため。
fn press_ctx(
    app: &AppState,
    button: PointerButton,
    m: &Modifiers,
    blocked: bool,
    on_top: bool,
    holds_off: bool,
) -> PressCtx {
    let start = nav::start_of(app, button, m);
    PressCtx {
        blocked,
        on_top,
        stencil: app.stencil.handling(),
        busy: app.session.doc.has_active_stroke()
            || crate::features::stroking_except(app, "region")
            || app.region.leftover_drag.is_some()
            || moving_view(app),
        view_op: start.moves_view(),
        click_op: nav::click_of(app, button, m).is_some(),
        eyedrop: start == nav::Start::Pick,
        tool: matches!(start, nav::Start::Tool | nav::Start::Select(_)),
        paints: app.mode.paints(),
        holds_off,
    }
}

/// 表示を回している・パンしている・拡縮している最中か（マウスもペンも）。
fn moving_view(app: &AppState) -> bool {
    app.canvas.rotating.is_some() || app.canvas.panning || app.canvas.zooming.is_some()
}

/// 記録に出す、今動かしている表示の操作の名前（無ければ "none"）。
fn nav_name(app: &AppState) -> &'static str {
    if app.canvas.rotating.is_some() {
        "rotate"
    } else if app.canvas.panning {
        "pan"
    } else if app.canvas.zooming.is_some() {
        "zoom"
    } else {
        "none"
    }
}

/// マウスのツールの押しのボタン（選択の行を左ボタン以外にしていれば、そのボタン。ツールの押しが無ければ左ボタン）。
fn tool_button(app: &AppState) -> PointerButton {
    app.canvas
        .mouse_press
        .filter(|p| p.owner == Owner::Tool)
        .map_or(PointerButton::Primary, |p| p.button)
}

/// このボタンを今押しているか（無ければ偽）。
fn button_down(ui: &Ui, button: Option<PointerButton>) -> bool {
    button.is_some_and(|b| ui.input(|i| i.pointer.button_down(b.into())))
}

/// 文字の入力欄が前のフレームにあったかを覚える場所。
fn typing_id() -> egui::Id {
    egui::Id::new("yolu.canvas.typing")
}

/// 選択範囲を持っていて、Esc を使うものが無いか（Esc で選択を解除してよいか）。Esc を自分の操作に使うものを優先する: メニューなどの
/// ポップアップ・確認のウィンドウ・開いている浮いたウィンドウ・つまみやドラッグの途中（ボタンを押している間）・塗りつぶしの仕事・色の名前の変更。
/// キャンバスより前に描く部品やフレームの頭の処理が、このフレームの Esc でもうやめた（状態がもう空になっている）ものは、
/// `note_escape_taken` の印で見る。
/// 文字の入力中と、描く・形を作る・移動と変形・グラデーション・図形・パスなどの途中は、呼ぶ側が先に見る（やめるものがあればそちらが先）。
fn escape_is_free(app: &AppState, ctx: &egui::Context) -> bool {
    app.session.doc.selection().is_some()
        && app.ui.popup.is_none()
        && !app.ui.popup_was_open
        && app.sel.dialog.is_none()
        && !crate::windows::modal_open(app)
        && !crate::ui::window::any_open(ctx)
        && !crate::ui::window::escape_taken(ctx)
        && !ctx.input(|i| i.pointer.any_down())
        && app.region.job.is_none()
        && app.colorsets.rename.is_none()
        && app.colorsets.dragging.is_none()
        && app.toolset.ui.drag.is_none()
}

fn handle_input(ui: &mut Ui, app: &mut AppState, rect: Rect, pen: &[PenSample], foreign: bool) {
    let (w_px, h_px) = (app.session.doc.width(), app.session.doc.height());
    let ctx = ui.ctx().clone();
    let typing = ctx.egui_wants_keyboard_input();
    // 文字の入力欄は Esc を、入力をやめるのに使う。欄がこのフレームのこれより前に Esc で手放したときも、1 つ前のフレームで入力中だったかで分かる
    let typed_last = ctx.data_mut(|d| d.get_temp::<bool>(typing_id()).unwrap_or(false));
    ctx.data_mut(|d| d.insert_temp(typing_id(), typing));
    let (now, frame_dt) = ctx.input(|i| (i.time, i.unstable_dt as f64));
    // 保存していない変更の問いが開いている間も止める（別ウィンドウのキャンバスは問いの覆いの外で、入力を直に読むため）
    let blocked = crate::windows::pointer_blocked(app);
    let (events, modifiers, r_down, space_down) = ui.input(|i| {
        (
            i.events.clone(),
            Modifiers::from(i.modifiers),
            crate::keymap::hold_down(i, "view.rotate_hold"),
            crate::keymap::hold_down(i, "view.pan_hold"),
        )
    });
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

    // ペン（Windows Ink）の点が先、egui のイベントが後。ペンの押し（触れてから離すまで）は、ペンの点だけで扱い、同じ押しが egui のポインタの押しとして
    // 二重に来ても使わない（`pen_frame`）
    let stream = Stream::new(pen, &events, app.canvas.pen_press.is_some(), now, frame_dt);
    let pen_frame = stream.pen_frame;
    let ppp = ctx.pixels_per_point();
    if crate::pen::log::enabled() && (pen_frame || nav_name(app) != "none") {
        crate::pen::log::event(|| {
            crate::pen::log::frame_line(
                "canvas",
                ctx.cumulative_frame_nr(),
                pen_frame,
                app.canvas.pen_press.is_some(),
                pen.len(),
                nav_name(app),
            )
        });
    }
    for item in stream {
        let (event, time) = match item {
            Item::Pen(s) => {
                let lost = app.pen_release_lost(s);
                let s = PointerSample::pen(s, ppp, app.canvas.pen_press, lost, modifiers);
                pen_sample(ui, app, rect, &s, &frame);
                continue;
            }
            Item::Event { event, time } => (event, time),
        };
        // Y を押しているあいだのドラッグはステンシルの置き場を動かす（描かない・回さない・パンしない）
        let over = match event {
            Event::PointerButton { pos, .. } => on_top(ui, rect, *pos),
            _ => false,
        };
        // 編集・ポーズのモードでは描かないので、ステンシルも使わない（3D と同じ）
        if app.mode.paints() && crate::stencil::handle_event(app, event, rect, over, &modifiers) {
            continue;
        }
        // マウスの押し・離し・動き（ペンの点と同じ形。ペンの押しの間の印 `pen_frame` は点が持つ）
        if let Some(s) = PointerSample::mouse(
            event,
            time,
            app.canvas.touch_pressure.unwrap_or(1.0),
            modifiers,
            pen_frame,
        ) {
            let pen_frame = s.pen_frame;
            match s.phase {
                Phase::Press | Phase::Release { .. } => {
                    let (pos, button) = (s.pos, &s.button);
                    if s.phase == Phase::Press {
                        // ポリゴン塗りつぶしの右クリック: アイランドの優先・焼かないのメニュー（開いているアイランドのメニューの外の右クリックは、
                        // 重なった次のアイランドのメニュー。開いているメニューの受け皿が上にあるので、そのときはメニューの本体の外かだけを見る）
                        let menu = *button == PointerButton::Secondary
                            && app.ui.popup.as_ref().is_some_and(|p| {
                                matches!(
                                    p.kind,
                                    crate::state::PopupKind::BakeIsland { map: false, .. }
                                ) && rect.contains(pos)
                                    && !p.state.rect.contains(pos)
                            });
                        if *button == PointerButton::Secondary
                            && !pen_frame
                            && (menu || (!frame.no_press && on_top(ui, rect, pos)))
                        {
                            let view = app.view.view(rect, w_px, h_px);
                            crate::bake::overlap::menu_press(
                                app,
                                crate::region::tools::Where::Canvas(&view),
                                pos,
                            );
                        }
                        // ペンの押しは、ペンの点が持つ（これは同じ押しの egui のポインタの代わりの入力）。スポイトの途中は、ほかのボタンで始めない
                        if pen_frame {
                            crate::pen::log::event(|| {
                                crate::pen::log::press_line(
                                    crate::pen::log::Site {
                                        view: "canvas",
                                        src: "mouse",
                                        frame: ctx.cumulative_frame_nr(),
                                        pen_frame: true,
                                    },
                                    "none",
                                    pos,
                                    &format!("{button:?}"),
                                    true,
                                )
                            });
                        }
                        if menu
                            || frame.no_press
                            || !on_top(ui, rect, pos)
                            || pen_frame
                            || app.canvas.eyedrop.is_some()
                        {
                            continue;
                        }
                        // 3D ビューで離した後の残りを塗っている途中（確定待ち）の押し: 先に確定してから、この押しを受ける
                        crate::view3d::input::settle(app);
                        // 押した瞬間に、実際のボタン・修飾・押しながらのキーで、ドラッグの操作と離しの操作を別々に引き、押しの持ち主を決める
                        // （`press_ctx`。ペンと同じ表）
                        let m = s.modifiers;
                        let start = nav::start_of(app, *button, &m);
                        let click = nav::click_of(app, *button, &m);
                        crate::pen::log::event(|| {
                            crate::pen::log::press_line(
                                crate::pen::log::Site {
                                    view: "canvas",
                                    src: "mouse",
                                    frame: ctx.cumulative_frame_nr(),
                                    pen_frame: false,
                                },
                                &crate::pen::log::mouse_kind("start", start, click.is_some()),
                                pos,
                                &format!("{button:?}"),
                                false,
                            )
                        });
                        let owner = crate::input::owner(&press_ctx(
                            app,
                            *button,
                            &m,
                            frame.no_press,
                            true,
                            false,
                        ));
                        match owner {
                            Owner::View => {
                                nav::start(app, rect, pos, *button, start, &m);
                            }
                            Owner::Eyedrop => {
                                // ほかのボタンを押している間（左ボタンのドラッグの途中など）は始めない
                                let others_down = ui.input(|i| {
                                    [
                                        PointerButton::Primary,
                                        PointerButton::Secondary,
                                        PointerButton::Middle,
                                    ]
                                    .into_iter()
                                    .any(|b| b != *button && i.pointer.button_down(b.into()))
                                });
                                if !others_down {
                                    crate::eyedrop::right_begin(
                                        app,
                                        StrokeSource::Mouse,
                                        pos,
                                        *button,
                                    );
                                }
                            }
                            Owner::Tool | Owner::Ignored => {}
                        }
                        // 離しの操作の控えはビューの押しだけ、描いた場所の控えはツールの押しだけ（ペンと同じ。無視した押しは何も書かない）
                        if owner == Owner::View {
                            nav::note_click(app, pos, *button, click);
                        }
                        if owner == Owner::Tool && app.mode.paints() {
                            app.rulers.last_drew = Some(crate::rulers::Place::Canvas);
                        }
                        let press = Press {
                            source: StrokeSource::Mouse,
                            owner,
                            button: *button,
                            last: pos,
                        };
                        if owner != Owner::Tool {
                            // ビューの押し・スポイト・何もしない（編集・ポーズのモードの 2D のキャンバスは見るだけ。描かない・選択範囲も作らない）
                        } else if let Some(kind) = app.tool.def().canvas {
                            // ドラッグの札を持つツール（選択・移動と変形・グラデーション・図形と定規・パス）。選択の行の組み合わせ方は、押したボタンで引く
                            app.sel.press_button = *button;
                            app.canvas.mouse_press = Some(press);
                            let view = app.view.view(rect, w_px, h_px);
                            let ctx = InputCtx {
                                modifiers: m,
                                now,
                                rect,
                                pass: ctx.cumulative_pass_nr(),
                            };
                            kind.handler()
                                .press(app, &view, pos, StrokeSource::Mouse, &ctx);
                        } else {
                            let view = app.view.view(rect, w_px, h_px);
                            let shift = m.shift;
                            if begin_any(app, &view, pos, StrokeSource::Mouse, false, rect, shift) {
                                app.canvas.mouse_press = Some(press);
                                let view = app.view.view(rect, w_px, h_px);
                                first_point(
                                    app, &view, pos, s.pressure, s.tilt, s.rotation, s.time, shift,
                                );
                            }
                        }
                    } else {
                        // 離した: ドラッグを始めた側が終わらせる（ツールを替えていても）。そのボタンが始めた物だけを終える
                        if tool_button(app) == *button {
                            let ctx = InputCtx {
                                modifiers: s.modifiers,
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
                            app.canvas.mouse_press = None;
                        }
                        if !pen_frame {
                            nav::released(app, rect, pos, *button);
                        }
                        if app
                            .canvas
                            .eyedrop
                            .is_some_and(|p| p.button == *button && p.source == StrokeSource::Mouse)
                        {
                            let view = app.view.view(rect, w_px, h_px);
                            // キャンバスの表示域の中（上に別の物が無い所）で離したときだけ取る。外で離したら、見えていない画素の色は取らずに取りやめる
                            let inside = on_top(ui, rect, pos);
                            crate::eyedrop::right_end(app, &view, StrokeSource::Mouse, pos, inside);
                        }
                        if *button == PointerButton::Secondary {
                            let view = app.view.view(rect, w_px, h_px);
                            crate::bake::overlap::menu_release(
                                app,
                                &ctx,
                                crate::region::tools::Where::Canvas(&view),
                                pos,
                            );
                        }
                    }
                    app.canvas.last_pointer = Some(pos);
                }
                Phase::Move => {
                    let pos = s.pos;
                    let previous = app.canvas.last_pointer.unwrap_or(pos);
                    if !pen_frame {
                        let move_ctx = InputCtx {
                            modifiers: s.modifiers,
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
                        add_point(app, &view, pos, s.pressure, s.tilt, s.rotation, s.time);
                    }
                    // ペンの押しの回す・パン・拡縮は、ペンの点が動かす
                    if crate::pen::log::enabled() && nav_name(app) != "none" {
                        crate::pen::log::event(|| {
                            crate::pen::log::move_line(
                                crate::pen::log::Site {
                                    view: "canvas",
                                    src: "mouse",
                                    frame: ctx.cumulative_frame_nr(),
                                    pen_frame,
                                },
                                pos,
                                previous,
                                nav_name(app),
                                !pen_frame,
                            )
                        });
                    }
                    if !pen_frame {
                        nav::moved(app, rect, pos, previous, s.modifiers.shift);
                        crate::eyedrop::right_move(app, StrokeSource::Mouse, pos);
                    }
                    app.canvas.last_pointer = Some(pos);
                }
                Phase::Idle { .. } => {}
            }
            continue;
        }
        match event {
            Event::Touch { force, phase, .. } => app.note_touch(*force, *phase),
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
            // メニュー・パイ・ダイアログが開いている間の Esc は、それを閉じる（下のキャンバスの操作はやめない）
            Event::Key {
                key: Key::Escape,
                pressed: true,
                ..
            } if !blocked => {
                // 3D ビューで離した後の残りを塗っている間（確定待ち）の Esc は、線を捨てない: 確定してから、Esc の普段の意味へ進む
                crate::view3d::input::settle(app);
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
                if app.canvas.eyedrop.take().is_some() {
                    // 右ボタンのスポイトを取りやめた（色は変えない）
                } else if cancelled(app, true) {
                    // 図形と定規は離すまで画素・定規を変更しない。移動・変形のドラッグは何も変えずにやめた。
                    // パスの点のドラッグを捨てた（ドラッグが無ければ選んだ点を外した）
                } else if app.is_stroking() {
                    finish_stroke(app, true);
                } else if let Some(drag) = app.canvas.rotating.take() {
                    app.view.angle = drag.start_angle;
                    app.view.pan = drag.start_pan;
                    // 動かさずに離すクローンの元の指定も取りやめる（3D と同じ）
                    app.canvas.clone_press = None;
                } else if !cancelled(app, false)
                    && !typing
                    && !typed_last
                    && escape_is_free(app, ui.ctx())
                {
                    // やめるものが無かった: 選択範囲があれば解除（Ctrl+D と同じ。1 回の取り消し）
                    app.apply(crate::state::Action::Sel(
                        crate::selection::SelAction::Edit(crate::selection::SelEdit::Clear),
                    ));
                }
            }
            Event::Key {
                key: key @ (Key::Enter | Key::Backspace),
                pressed: true,
                modifiers: m,
                ..
            } if !typing && !blocked => {
                // ドラッグの途中の Enter（移動・変形）、多角形の確定・最後の点（選択）
                for kind in CanvasKind::ALL {
                    if kind.handler().key(app, (*key).into(), (*m).into()) {
                        break;
                    }
                }
            }
            Event::WindowFocused(false) => {
                // フォーカスを失ったら、そこまでを確定する（離したのを受け取れないので）。選択の途中の形は捨てる。移動と変形・グラデーション・図形は
                // 何も変えずにやめる
                release_stroke(app, ui.ctx());
                for kind in CanvasKind::ALL {
                    kind.handler().focus_lost(app);
                }
                app.canvas.pen_press = None;
                app.canvas.eyedrop = None;
                nav::cancel(app);
                app.canvas.rotate_key_held = false;
                app.forget_touch();
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
    // ボタンを離したのを取りこぼしたとき（ウィンドウの外で離したなど）も、押していなければ終える。ストローク・ドラッグの札を持つツールのドラッグは、
    // 最後の位置で終える（ツールごとの終わらせ方は受け口が決める）
    // ツールの押しのボタン（選択の行を左ボタン以外にしていれば、そのボタン）
    let tool_button = tool_button(app);
    let released = !ui.input(|i| i.pointer.button_down(tool_button.into()))
        && !events
            .iter()
            .any(|e| matches!(e, Event::PointerButton { pressed: true, .. }));
    if released {
        app.canvas.mouse_press = None;
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
    // スポイトのボタンを離したのを取りこぼしたとき（ウィンドウの外で離したなど）は、取りやめる（3D と同じ。離した所が分からないので、色は取らない）
    let lost_pick = app
        .canvas
        .eyedrop
        .filter(|press| press.source == StrokeSource::Mouse)
        .is_some_and(|press| {
            !ui.input(|i| i.pointer.button_down(press.button.into()))
                && !events.iter().any(|e| {
                    matches!(
                        e,
                        Event::PointerButton {
                            button,
                            pressed: true,
                            ..
                        } if PointerButton::from(*button) == press.button
                    )
                })
        });
    if lost_pick {
        let view = app.view.view(rect, w_px, h_px);
        let at = app.canvas.last_pointer.unwrap_or(rect.center());
        crate::eyedrop::right_end(app, &view, StrokeSource::Mouse, at, false);
    }
    // 表示を動かしている押しのボタンを離したのを取りこぼしたときも終える。ペンが回す・拡縮している間は、egui のポインタが押していなくても
    // 続ける（ペンが離したときに終える）
    if !button_down(ui, app.canvas.nav_button) && !nav::pen_driven(app) {
        app.canvas.rotating = None;
        app.canvas.zooming = None;
        app.canvas.panning = false;
        app.canvas.nav_button = None;
    }
    // 離したのを取りこぼした押しは、クローンの元にしない（離した所が分からない。3D と同じ）
    if !button_down(ui, app.canvas.clone_press.map(|(_, b)| b)) && !nav::pen_driven(app) {
        app.canvas.clone_press = None;
    }
    crate::stencil::settle(app, ui.input(|i| i.pointer.any_down()));
}

/// 左右反転の印のツールチップ（キーは今のモードの今の割り当て）。
pub fn flip_tip(lang: crate::lang::Lang, mode: crate::mode::EditorMode) -> String {
    crate::shortcuts::named_with_keys(
        lang,
        lang.t("canvas-the-view-is-mirrored-click-tip"),
        &[crate::shortcuts::key_in("view.flip", mode)],
    )
}

/// 機能の表の行（キャンバスのストロークの最中か。文書を替えたら、前の文書の座標で覚えた点を捨てる）。
pub(crate) const FEATURE: Feature = Feature {
    stroking: Some(|app| app.canvas.stroke.is_some()),
    document_replaced: Some(|app| {
        let canvas = &mut app.canvas;
        canvas.previous_end = None;
        canvas.current_end = None;
        canvas.shift_hold = None;
        canvas.ruler_constraint = None;
        canvas.clone_press = None;
        canvas.clone_offset = None;
    }),
    ..Feature::new("canvas")
};

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
            true,
            (0.0, 0.0)
        ));
        first_point(&mut app, &view, a, 0.7, Tilt::default(), None, 1.0, false);
        finish_stroke(&mut app, false);
        let before = app.session.doc.revision();
        assert!(begin_stroke(
            &mut app,
            StrokeSource::Mouse,
            false,
            rect,
            true,
            (0.0, 0.0)
        ));
        first_point(&mut app, &view, b, 0.7, Tilt::default(), None, 2.0, true);
        assert_eq!(app.canvas.stroke_points, 2);
        finish_stroke(&mut app, false);
        let end = app.canvas.previous_end.unwrap();
        assert!((end.0 - 40.0).abs() < 1e-4 && (end.1 - 40.0).abs() < 1e-4);
        assert_ne!(before, app.session.doc.revision());
        app.session.doc.undo().unwrap();
        app.session.doc.undo().unwrap();
        assert!(!app.session.doc.can_undo());
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
            true,
            (0.0, 0.0)
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
        assert!(!app.session.doc.can_undo());
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
            true,
            (0.0, 0.0)
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
        assert_eq!(app.session.doc.undo_count(), 1);
        // 保持を超えて動かしたら、続きは向きを固定せずに普通に描く
        assert!(begin_stroke(
            &mut app,
            StrokeSource::Mouse,
            false,
            rect,
            true,
            (0.0, 0.0)
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
        assert_eq!(app.session.doc.undo_count(), 1);
    }
}
