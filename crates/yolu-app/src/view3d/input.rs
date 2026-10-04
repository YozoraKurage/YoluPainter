//! 3D ビューの入力（Unity 版の 3D ビューの操作と同じ）:
//! - 左ドラッグで面に描く（ブラシ・消しゴム。ペンの筆圧も）。ほかのテクスチャセットの面からは描き始めない。
//! - 右ドラッグか Alt + 左ドラッグで回す、中ドラッグか Shift を足したドラッグでパン、ホイールで寄る・引く。
//! - ストロークを取り残さない: 離す・Esc（捨てる）・窓のフォーカスを失う（そこまでを確定）・ボタンを離したのを取りこぼす で必ず終える。
//!   ストロークの間はカメラもモデルも動かさない（遮蔽の結果を覚えて使うので）。
//!
//! 画面の点はタブの中身の左上からの egui の点。core のカメラも同じ点の大きさで作る（ストロークの間隔は Unity 版と同じく画面の点）。

use egui::{Color32, Event, Key, PointerButton, Pos2, Rect, Stroke, Ui};
use yolu_core::geometry::{pick, world_radius, CameraView, SurfaceStroke};
use yolu_core::glam::{Vec2, Vec3};

use super::{gizmo, Nav};
use crate::pen::PenSample;
use crate::state::{AppState, StrokeSource};

fn on_top(ui: &Ui, rect: Rect, p: Pos2) -> bool {
    rect.contains(p)
        && ui
            .ctx()
            .layer_id_at(p)
            .is_none_or(|layer| layer == ui.layer_id())
}

fn local(rect: Rect, p: Pos2) -> Vec2 {
    Vec2::new(p.x - rect.left(), p.y - rect.top())
}

/// 今のカメラを表示域の大きさ（点）で見たもの。
pub fn camera_view(app: &AppState, rect: Rect) -> CameraView {
    app.view3d.camera.view(rect.width(), rect.height())
}

fn begin(
    app: &mut AppState,
    rect: Rect,
    at: Pos2,
    pressure: f32,
    source: StrokeSource,
    eraser: bool,
) {
    // 範囲の道具（バケツ・ポリゴン塗りつぶし・ID の色で選択）は、点でなく押した面の範囲を使う
    if app.tool.is_region() {
        if app.region.drag.is_none() && crate::region::tools::surface_press(app, rect, at, source) {
            app.view3d.input.stroke = Some(source);
            app.view3d.input.stroke_points = 0;
        }
        return;
    }
    if !app.tool.paints() {
        // 選択の道具は 2D のキャンバスだけで使う（3D ビューで描き始めない）
        app.message = app
            .lang
            .pick(
                "この道具は 2D のキャンバスで使います",
                "This tool works on the 2D canvas",
            )
            .into();
        return;
    }
    let Some(model) = app.view3d.model.clone() else {
        return;
    };
    let view = camera_view(app, rect);
    let p = local(rect, at);
    let material = app.view3d.material;
    if material < 0 {
        app.message = app.region_missing_reason();
        return;
    }
    if let Some(reason) = app.read_only_reason() {
        app.message = format!(
            "{}: {reason}",
            app.lang.pick(
                "読むだけのテクスチャセットには描けません",
                "Cannot paint on a read-only texture set"
            )
        );
        return;
    }
    if let Some(hit) = pick(&model.geometry, &view, p) {
        if hit.material != material {
            let name = model.material_name(hit.material as usize, app.lang);
            app.message = app.lang.pick(
                format!("ほかのテクスチャセット（{name}）の面です。"),
                format!("Surface of another texture set ({name})."),
            );
            return;
        }
    }
    let Some(layer) = app.selected_layer else {
        app.message = app.lang.pick("描くレイヤーがありません。", "No layer to paint on.").into();
        return;
    };
    if let Some(reason) = app.paint_blocker() {
        app.message = reason;
        return;
    }
    let settings = app.stroke_settings(eraser);
    // 面のダブは各画素を apply_pixel で塗るので、読み元を凍結する効果のブラシ（ぼかし・指先・クローン）は始める前に断る
    if !settings.erase && !app.m2.brush.effect.is_paint() {
        app.message = app
            .lang
            .pick(
                "3D では効果のブラシ（ぼかし・指先・クローン）は使えません",
                "Effect brushes (blur, smudge, clone) are not available in 3D",
            )
            .into();
        return;
    }
    // ステンシル: 置き場とカメラはストロークの始めに決める（面のテクセルの点を画面へ写して、そこの画像を読む）
    let (stencil_brush, surface_stencil) = match app.surface_stencil(rect) {
        Ok(Some((brush, surface))) => (Some(brush), Some(surface)),
        Ok(None) => (None, None),
        Err(e) => {
            app.message = format!("{}: {e}", app.lang.pick("描けません", "Cannot paint"));
            return;
        }
    };
    // 全部入りのブラシ。面のダブは筆先・ゆらぎ・質感・デュアル・フェード・傾き・回転・速さ・手ぶれ補正を受け取らず（効くのは基本の値・色・
    // 色の変化・消しゴム・筆圧・ステンシル）、効果のブラシも塗れない
    let mut stroke = match app.begin_paint_stroke_with(layer, eraser, stencil_brush) {
        Ok(s) => s,
        Err(e) => {
            app.message = format!("{}: {}", app.lang.pick("描けません", "Cannot paint"), app.lang.core_error(&e));
            return;
        }
    };
    match SurfaceStroke::begin_with_stencil(
        &mut app.doc,
        &mut stroke,
        model.geometry.clone(),
        view,
        &settings,
        Some(material),
        p,
        pressure.clamp(0.0, 1.0),
        surface_stencil,
    ) {
        Ok(s) => {
            app.stroke = Some(stroke);
            app.view3d.input.stroke = Some(source);
            app.view3d.input.surface = Some(s);
            app.view3d.input.stroke_points = 1;
            if !settings.erase {
                app.color.remember();
            }
            app.modified = true;
        }
        Err(e) => {
            app.doc.cancel_stroke(stroke);
            app.message = app.lang.surface_error(&e);
        }
    }
}

fn add(app: &mut AppState, rect: Rect, at: Pos2, pressure: f32) {
    // ポリゴン塗りつぶしのドラッグは、通った範囲を足す
    if app.region.drag.is_some() {
        crate::region::tools::drag_to(app, crate::region::tools::Where::Surface(rect), at);
        return;
    }
    let (Some(stroke), Some(surface)) = (app.stroke.as_mut(), app.view3d.input.surface.as_mut())
    else {
        return;
    };
    match surface.add(
        &mut app.doc,
        stroke,
        local(rect, at),
        pressure.clamp(0.0, 1.0),
    ) {
        Ok(()) => app.view3d.input.stroke_points += 1,
        Err(e) => {
            // 予算を超えた・1 回の入力のダブが多すぎる: 途中まで塗った画素も戻す
            if let Some(stroke) = app.stroke.take() {
                app.doc.cancel_stroke(stroke);
            }
            app.view3d.stroke_ended();
            app.message = app.lang.surface_error(&e);
        }
    }
}

/// 3D のストロークを終える（cancel なら捨てる）。
pub fn finish(app: &mut AppState, cancel: bool) {
    if app.view3d.input.stroke.is_none() {
        return;
    }
    if crate::region::tools::finish_drag(app, cancel) {
        app.view3d.stroke_ended();
        return;
    }
    let surface = app.view3d.input.surface.take();
    let Some(mut stroke) = app.stroke.take() else {
        app.doc.cancel_active_stroke();
        app.view3d.stroke_ended();
        return;
    };
    if cancel {
        app.doc.cancel_stroke(stroke);
        app.message = app.lang.pick("ストロークを取り消しました。", "Stroke cancelled.").into();
    } else {
        let mut surface = surface;
        let last = surface
            .as_mut()
            .map(|s| s.finish(&mut app.doc, &mut stroke));
        match last {
            Some(Err(e)) => {
                app.doc.cancel_stroke(stroke);
                app.message = app.lang.surface_error(&e);
            }
            _ => {
                if let Some(note) = surface.as_ref().and_then(|s| s.note) {
                    app.message = app.lang.dab_refusal(note).into();
                }
                if let Err(e) = app.doc.end_stroke(stroke) {
                    app.message = app.lang.core_error(&e);
                }
            }
        }
    }
    app.view3d.stroke_ended();
}

/// 入力を当てる（rect はタブの中身の表示域）。
pub fn handle(ui: &mut Ui, app: &mut AppState, rect: Rect, pen: &[PenSample]) {
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    // ストロークの札をほか（キャンバスの Esc・フォーカスを失ったとき）が手放したら、こちらも終える
    if app.view3d.input.stroke.is_some() && app.stroke.is_none() && app.region.drag.is_none() {
        app.view3d.stroke_ended();
    }
    if app.view3d.model.is_none() {
        app.view3d.input.nav = None;
        return;
    }
    let blocked = app.popup.is_some() || app.popup_was_open;
    app.region.modifiers = ui.input(|i| i.modifiers);
    let events = ui.input(|i| i.events.clone());
    // ポーズのモードでは描かない（左ボタンはギズモと骨を選ぶ。ペンの点は描くのに使わない）
    let pose_mode = app.view3d.pose.mode;
    // ポーズのモードの間はペンの点を見ないので、押している印も持ち越さない（離したのを見落とした印が次の押しを止めない）
    if pose_mode {
        app.view3d.input.pen_once = None;
    }
    let pen: &[PenSample] = if pose_mode { &[] } else { pen };
    let (snap, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
    // ギズモのドラッグは、1 フレームに何度ポインタが動いても、最後の位置を 1 回だけ当てる（1 回ごとにスキニング・refit・
    // モデルの組み直しが走るので、高いポーリングのマウスやペンでは、途中の位置は描かれずに捨てられるだけ）。ボタンを離す・Esc・
    // フォーカスを失うの前には、そこまでの位置を当ててから終える
    let mut drag_at: Option<Pos2> = None;
    let flush = |app: &mut AppState, drag_at: &mut Option<Pos2>| {
        if let Some(at) = drag_at.take() {
            gizmo::drag_to(app, rect, at, snap);
        }
    };

    // ペン（Windows Ink）。点があればこのフレームのストロークはペンだけで描く
    let pen_frame =
        !pen.is_empty() || matches!(app.view3d.input.stroke, Some(StrokeSource::Pen(_)));
    for s in pen {
        let p = s.pos_points(ppp);
        // 押した瞬間に終わるツール（バケツ・ID の色で選択）をこのペンで押している間は、次の点で押し直さない（離したら印を下ろす）
        if app.view3d.input.pen_once == Some(s.pointer_id) {
            if !s.contact {
                app.view3d.input.pen_once = None;
            }
            continue;
        }
        match app.view3d.input.stroke {
            None if s.contact
                && !blocked
                && on_top(ui, rect, p)
                && app.view3d.input.nav.is_none()
                && !app.stencil.handling() =>
            {
                begin(
                    app,
                    rect,
                    p,
                    s.pressure,
                    StrokeSource::Pen(s.pointer_id),
                    s.eraser,
                );
                if app.tool.is_one_shot() {
                    app.view3d.input.pen_once = Some(s.pointer_id);
                }
            }
            Some(StrokeSource::Pen(id)) if id == s.pointer_id && s.contact => {
                add(app, rect, p, s.pressure)
            }
            Some(StrokeSource::Pen(id)) if id == s.pointer_id && !s.contact => finish(app, false),
            _ => {}
        }
    }

    for event in &events {
        // T を押しているあいだのドラッグはステンシルの置き場を動かす（描かない・回さない・パンしない。ポーズのモードでは描かない）
        let over = match event {
            Event::PointerButton { pos, .. } => on_top(ui, rect, *pos),
            _ => false,
        };
        if !pose_mode && crate::stencil::handle_event(app, event, rect, over, shift) {
            continue;
        }
        match event {
            Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: m,
            } => {
                let pos = *pos;
                if *pressed {
                    if blocked || !on_top(ui, rect, pos) || app.view3d.input.stroke.is_some() {
                        continue;
                    }
                    let nav = match button {
                        PointerButton::Secondary => {
                            Some(if m.shift { Nav::Pan } else { Nav::Orbit })
                        }
                        PointerButton::Middle => Some(Nav::Pan),
                        PointerButton::Primary if m.alt => {
                            Some(if m.shift { Nav::Pan } else { Nav::Orbit })
                        }
                        _ => None,
                    };
                    if let Some(nav) = nav {
                        app.view3d.input.nav = Some((nav, *button));
                    } else if *button == PointerButton::Primary
                        && !pen_frame
                        && app.view3d.input.nav.is_none()
                    {
                        if pose_mode {
                            if app.view3d.pose.drag.is_none() {
                                gizmo::press(app, rect, pos);
                            }
                        } else if !app.stencil.handling() {
                            begin(app, rect, pos, 1.0, StrokeSource::Mouse, false);
                        }
                    }
                } else {
                    if app.view3d.input.nav.is_some_and(|(_, b)| b == *button) {
                        app.view3d.input.nav = None;
                    }
                    if *button == PointerButton::Primary
                        && app.view3d.input.stroke == Some(StrokeSource::Mouse)
                    {
                        finish(app, false);
                    }
                    if *button == PointerButton::Primary {
                        flush(app, &mut drag_at);
                        gizmo::release(app, true);
                    }
                }
                app.view3d.input.last_pointer = Some(pos);
            }
            Event::PointerMoved(pos) => {
                let pos = *pos;
                let previous = app.view3d.input.last_pointer.unwrap_or(pos);
                if app.view3d.input.stroke == Some(StrokeSource::Mouse) && !pen_frame {
                    add(app, rect, pos, 1.0);
                }
                if app.view3d.pose.drag.is_some() {
                    drag_at = Some(pos);
                }
                if let Some((nav, _)) = app.view3d.input.nav {
                    let d = pos - previous;
                    match nav {
                        Nav::Orbit => app.view3d.camera.orbit(d.x, d.y),
                        Nav::Pan => app.view3d.camera.pan(d.x, d.y, rect.height()),
                    }
                }
                app.view3d.input.last_pointer = Some(pos);
            }
            Event::MouseWheel { unit, delta, .. } => {
                let Some(p) = ui.input(|i| i.pointer.hover_pos()) else {
                    continue;
                };
                if blocked || !on_top(ui, rect, p) || app.view3d.input.stroke.is_some() {
                    continue;
                }
                let notches = match unit {
                    egui::MouseWheelUnit::Point => delta.y / 40.0,
                    egui::MouseWheelUnit::Line => delta.y,
                    egui::MouseWheelUnit::Page => delta.y * 3.0,
                };
                app.view3d.camera.zoom(notches);
            }
            Event::Key {
                key: Key::Escape,
                pressed: true,
                ..
            } => {
                if app.view3d.input.stroke.is_some() {
                    finish(app, true);
                }
                // ギズモのドラッグは始まりのポーズへ戻す（それまでの位置は当てない）
                drag_at = None;
                gizmo::release(app, false);
                app.view3d.input.nav = None;
            }
            Event::WindowFocused(false) => {
                // フォーカスを失ったら、そこまでを確定する（離したのを受け取れないので）
                finish(app, false);
                app.view3d.input.pen_once = None;
                flush(app, &mut drag_at);
                gizmo::release(app, true);
                app.view3d.input.nav = None;
            }
            _ => {}
        }
    }
    flush(app, &mut drag_at);
    // ボタンを離したのを取りこぼしたとき（窓の外で離したなど）も、押していなければ終える
    let (primary, any_down) = ui.input(|i| (i.pointer.primary_down(), i.pointer.any_down()));
    if app.view3d.input.stroke == Some(StrokeSource::Mouse)
        && !primary
        && !events
            .iter()
            .any(|e| matches!(e, Event::PointerButton { pressed: true, .. }))
    {
        finish(app, false);
    }
    if app.view3d.pose.drag.is_some()
        && !primary
        && !events
            .iter()
            .any(|e| matches!(e, Event::PointerButton { pressed: true, .. }))
    {
        gizmo::release(app, true);
    }
    if !any_down {
        app.view3d.input.nav = None;
    }
    crate::stencil::settle(app, any_down);
}

/// ブラシのカーソル: ポインタの下の面の、ブラシの半径の円（面の接平面の円を画面へ写した楕円）。白と黒の二重の線。
/// 面に当たらなければ描かずに false。
pub fn draw_cursor(ui: &Ui, app: &AppState, rect: Rect, pointer: Pos2) -> bool {
    let Some(model) = &app.view3d.model else {
        return false;
    };
    let view = camera_view(app, rect);
    let Some(hit) = pick(&model.geometry, &view, local(rect, pointer)) else {
        return false;
    };
    let radius = world_radius(&model.geometry, app.brush.radius as f64, app.doc.width());
    // 接平面の 2 つの軸
    let n = hit.normal;
    let helper = if n.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let u = n.cross(helper).normalize_or_zero();
    let v = n.cross(u);
    let screen_radius = view.world_radius_to_screen(hit.position, radius);
    let segments = ((screen_radius * 0.8).ceil() as usize).clamp(24, 96);
    let mut points = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let a = i as f32 / segments as f32 * std::f32::consts::TAU;
        let p = hit.position + (u * a.cos() + v * a.sin()) * radius;
        match view.to_screen(p) {
            Some(s) => points.push(Pos2::new(rect.left() + s.x, rect.top() + s.y)),
            None => return false,
        }
    }
    let painter = ui.painter_at(rect);
    painter.add(egui::Shape::line(
        points.clone(),
        Stroke::new(3.0, Color32::from_black_alpha(140)),
    ));
    painter.add(egui::Shape::line(
        points,
        Stroke::new(1.2, Color32::from_white_alpha(230)),
    ));
    true
}
