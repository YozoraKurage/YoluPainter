//! 3D ビューの入力（Unity 版の 3D ビューの操作と同じ）:
//! - 左ドラッグで面に描く（ブラシ・消しゴム。ペンの筆圧も）。ほかのテクスチャセットの面からは描き始めない。
//! - 右ドラッグか Alt + 左ドラッグで回す、中ドラッグか Shift を足したドラッグでパン、ホイールで寄る・引く。クローンのブラシでは、Alt + 左を
//!   動かさずに離すと、そこがクローンの元（動かせば回す）。
//! - ぼかし・指先・クローンと 3D の対称（ミラー・放射状）は、面のストロークに通す（core の `SurfaceStrokeOptions`）。
//! - ストロークを取り残さない: 離す・Esc（捨てる）・窓のフォーカスを失う（そこまでを確定）・ボタンを離したのを取りこぼす で必ず終える。
//!   ストロークの間はカメラもモデルも動かさない（遮蔽の結果を覚えて使うので）。
//!
//! 画面の点はタブの中身の左上からの egui の点。core のカメラも同じ点の大きさで作る（ストロークの間隔は Unity 版と同じく画面の点）。

use egui::{Color32, Event, Key, PointerButton, Pos2, Rect, Stroke, Ui};
use yolu_core::geometry::{
    copy_hits, pick, world_radius, CameraView, Ray, SurfaceCloneSource, SurfaceEffect,
    SurfaceGeometry, SurfaceHit, SurfaceStroke, SurfaceStrokeOptions, SurfaceSymmetrySetup,
};
use yolu_core::glam::{Vec2, Vec3};

use super::{gizmo, Nav};
use crate::engine::BrushEffect;
use crate::pen::PenSample;
use crate::state::{AppState, StrokeSource, Tool};

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

/// クリックとみなす、押してから離すまでに動いてよい距離（画面の点）。
const CLICK_DISTANCE: f32 = 4.0;

/// 今の道具がクローンのブラシか（元を決められる）。
fn clone_active(app: &AppState) -> bool {
    app.tool.paints()
        && app.tool != Tool::Eraser
        && matches!(app.m2.brush.effect, BrushEffect::Clone { .. })
}

/// 画面の点の下の面を、クローンの元にする（描くテクスチャセットの面だけ）。
fn set_clone_source(app: &mut AppState, rect: Rect, at: Pos2) {
    let Some(model) = app.view3d.model.clone() else {
        return;
    };
    let view = camera_view(app, rect);
    match pick(&model.geometry, &view, local(rect, at)) {
        Some(hit) if hit.material == app.view3d.material => {
            app.view3d.clone.set_source(hit);
            app.message = app
                .lang
                .pick("クローンの元を決めました。", "Clone source set.")
                .into();
        }
        _ => {
            app.message = app
                .lang
                .pick(
                    "今のテクスチャセットの面ではありません。",
                    "Not a surface of the active texture set.",
                )
                .into();
        }
    }
}

fn begin(
    app: &mut AppState,
    rect: Rect,
    at: Pos2,
    pressure: f32,
    source: StrokeSource,
    eraser: bool,
) {
    // スポイトは押した面の値を取るだけ（3D の Alt は回転なので、描く道具の一時的なスポイトは 2D だけ）
    if app.tool == crate::state::Tool::Eyedropper {
        crate::eyedrop::pick_surface(app, rect, at);
        return;
    }
    // パスの道具は、押した面の点（掴む・差し込む・足す）。ストロークは持たず、点のドラッグだけが続く
    if app.tool.is_path() {
        crate::pathtool::surface::press(app, rect, at, source);
        return;
    }
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
    // 効果のブラシ（消しゴムは色を塗る側）。クローンは元の面の点が要る
    let effect = if settings.erase {
        SurfaceEffect::Paint
    } else {
        match app.m2.brush.effect {
            BrushEffect::Paint => SurfaceEffect::Paint,
            BrushEffect::Blur { .. } => SurfaceEffect::Blur,
            BrushEffect::Smudge { .. } => SurfaceEffect::Smudge,
            BrushEffect::Clone { .. } => match app.view3d.clone.source_for(&model.geometry) {
                Some(source) => SurfaceEffect::Clone(SurfaceCloneSource {
                    source,
                    destination: app.view3d.clone.destination_for(&model.geometry),
                }),
                None => {
                    app.message = app
                        .lang
                        .pick("クローンの元がありません", "No clone source")
                        .into();
                    return;
                }
            },
        }
    };
    // 3D の対称。ストロークの始めに固める（途中で設定を変えても、このストロークには効かない）
    let symmetry = app.sel.symmetry.surface.setup();
    if symmetry.is_some() && matches!(effect, SurfaceEffect::Smudge | SurfaceEffect::Clone(_)) {
        app.message = app
            .lang
            .pick(
                "指先・クローンでは対称を使えません",
                "Smudge and clone do not work with symmetry",
            )
            .into();
        return;
    }
    // ステンシル: 置き場とカメラはストロークの始めに決める（面のテクセルの点を画面へ写して、そこの画像を読む）
    let (stencil_brush, surface_stencil) = match app.surface_stencil(rect) {
        Ok(Some((brush, surface))) => (Some(brush), Some(surface)),
        Ok(None) => (None, None),
        Err(e) => {
            app.message = format!("{}: {}", app.lang.pick("描けません", "Cannot paint"), app.lang.core_error(&e));
            return;
        }
    };
    // 全部入りのブラシ。面のダブは筆先・ゆらぎ・質感・デュアル・フェード・傾き・回転・速さ・手ぶれ補正を受け取らない（効くのは基本の値・色・
    // 色の変化・消しゴム・筆圧・ステンシルと、効果のブラシ・3D の対称）
    let mut stroke = match app.begin_paint_stroke_with(layer, eraser, stencil_brush) {
        Ok(s) => s,
        Err(e) => {
            app.message = format!("{}: {}", app.lang.pick("描けません", "Cannot paint"), app.lang.core_error(&e));
            return;
        }
    };
    match SurfaceStroke::begin_with_options(
        &mut app.doc,
        &mut stroke,
        model.geometry.clone(),
        view,
        &settings,
        Some(material),
        p,
        pressure.clamp(0.0, 1.0),
        SurfaceStrokeOptions {
            stencil: surface_stencil,
            symmetry,
            effect,
        },
    ) {
        Ok(s) => {
            app.stroke = Some(stroke);
            app.view3d.input.stroke = Some(source);
            app.view3d.input.symmetry = symmetry;
            note_symmetry(app, &s);
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

/// 対称の写しが塗られなかった理由を知らせる（全部塗れていれば何もしない）。
fn note_symmetry(app: &mut AppState, s: &SurfaceStroke) {
    if let Some(outcome) = s.symmetry_note() {
        app.message = app.lang.mirror_note(outcome).into();
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
        Ok(()) => {
            app.view3d.input.stroke_points += 1;
            if let Some(outcome) = surface.symmetry_note() {
                app.message = app.lang.mirror_note(outcome).into();
            }
        }
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
                if let Some(s) = surface.as_ref() {
                    note_symmetry(app, s);
                    if s.stats.lost > 0 {
                        app.message = app.lang.smudge_lost().into();
                    }
                }
                // 揃えるクローンは、変わったストロークの先の基準を次のストロークへ渡す
                let destination = surface.as_ref().and_then(|s| s.clone_destination());
                match app.doc.end_stroke(stroke) {
                    Ok(result) => {
                        if let (true, Some(d)) = (result.changed, destination) {
                            app.view3d.clone.destination = Some(d);
                        }
                    }
                    Err(e) => app.message = app.lang.core_error(&e),
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
    // 設定のパネルを開いているあいだの押しは、パネルの外でも 3D に使わない（パネルは外の押しで閉じる。その押しが描き始め・回し始めに
    // ならないように。このフレームの押しで閉じるときも、パネルはこの後に描くので開いている）。ホイールは使える
    let press_blocked = blocked || app.view3d.display.settings_open;
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
            // 塗りつぶしの形のギズモ（Shift で両側、Ctrl で刻み）
            crate::fillfx::gizmo::drag_to(app, rect, at, shift, snap);
        }
    };

    // ペン（Windows Ink）。点があればこのフレームのストロークはペンだけで描く
    let pen_frame =
        !pen.is_empty() || matches!(app.view3d.input.stroke, Some(StrokeSource::Pen(_)));
    for s in pen {
        let p = s.pos_points(ppp);
        // 形のギズモをこのペンで掴んでいる間は、その点でドラッグを進め、離したら確定する
        if app
            .fillfx
            .drag
            .as_ref()
            .is_some_and(|d| d.source == crate::fillfx::gizmo::Source::Pen(s.pointer_id))
        {
            if s.contact {
                drag_at = Some(p);
            } else {
                flush(app, &mut drag_at);
                crate::fillfx::gizmo::release(app, true);
            }
            continue;
        }
        // 押した瞬間に終わるツール（バケツ・ID の色で選択）をこのペンで押している間は、次の点で押し直さない（離したら印を下ろす）
        if app.view3d.input.pen_once == Some(s.pointer_id) {
            if !s.contact {
                app.view3d.input.pen_once = None;
            }
            continue;
        }
        // パスの道具: 触れる・動く・離すを、押す・動く・離すにする
        if app.tool.is_path() && app.view3d.input.stroke.is_none() {
            let usable = !press_blocked
                && on_top(ui, rect, p)
                && app.view3d.input.nav.is_none()
                && !app.stencil.handling();
            if usable || app.path.pen_in(true) || !s.contact {
                crate::pathtool::surface::pen_sample(app, rect, p, s.pointer_id, s.contact, usable);
            }
            continue;
        }
        match app.view3d.input.stroke {
            None if s.contact
                && !press_blocked
                && on_top(ui, rect, p)
                && app.view3d.input.nav.is_none()
                && !app.stencil.handling() =>
            {
                // 形のギズモのハンドルの上なら、描かずにドラッグを始める
                if !pose_mode
                    && crate::fillfx::gizmo::press(
                        app,
                        rect,
                        p,
                        crate::fillfx::gizmo::Source::Pen(s.pointer_id),
                    )
                {
                    continue;
                }
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
                    if press_blocked || !on_top(ui, rect, pos) || app.view3d.input.stroke.is_some()
                    {
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
                        // クローンのブラシでは、Alt + 左を動かさずに離すと元を決める（動かせば、そのまま回す）
                        if *button == PointerButton::Primary
                            && m.alt
                            && !m.shift
                            && clone_active(app)
                        {
                            app.view3d.input.clone_press = Some(pos);
                        }
                    } else if *button == PointerButton::Primary
                        && !pen_frame
                        && app.view3d.input.nav.is_none()
                    {
                        if pose_mode {
                            if app.view3d.pose.drag.is_none() {
                                gizmo::press(app, rect, pos);
                            }
                        } else if !app.stencil.handling() {
                            // 形のギズモのハンドルの上なら、描かずにドラッグを始める
                            if !crate::fillfx::gizmo::press(
                                app,
                                rect,
                                pos,
                                crate::fillfx::gizmo::Source::Mouse,
                            ) {
                                begin(app, rect, pos, 1.0, StrokeSource::Mouse, false);
                            }
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
                        crate::pathtool::surface::release(app, rect, pos, StrokeSource::Mouse);
                        if let Some(start) = app.view3d.input.clone_press.take() {
                            if start.distance(pos) <= CLICK_DISTANCE {
                                set_clone_source(app, rect, start);
                            }
                        }
                    }
                    if *button == PointerButton::Primary {
                        flush(app, &mut drag_at);
                        gizmo::release(app, true);
                        if app.fillfx.drag.as_ref().is_some_and(|d| d.source == crate::fillfx::gizmo::Source::Mouse) {
                            crate::fillfx::gizmo::release(app, true);
                        }
                    }
                }
                app.view3d.input.last_pointer = Some(pos);
            }
            Event::PointerMoved(pos) => {
                let pos = *pos;
                let previous = app.view3d.input.last_pointer.unwrap_or(pos);
                if app
                    .view3d
                    .input
                    .clone_press
                    .is_some_and(|start| start.distance(pos) > CLICK_DISTANCE)
                {
                    app.view3d.input.clone_press = None; // 動かした: 回すだけ
                }
                if app.view3d.input.stroke == Some(StrokeSource::Mouse) && !pen_frame {
                    add(app, rect, pos, 1.0);
                }
                if !pen_frame {
                    crate::pathtool::surface::moved(app, rect, pos, StrokeSource::Mouse);
                }
                if app.view3d.pose.drag.is_some()
                    || app.fillfx.drag.as_ref().is_some_and(|d| d.source == crate::fillfx::gizmo::Source::Mouse)
                {
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
                // パスの点のドラッグを捨てる（無ければ選んだ点を外す）
                let path_esc = app.path_cancel(ctx.cumulative_pass_nr());
                if app.view3d.input.stroke.is_some() && !path_esc {
                    finish(app, true);
                }
                // ギズモのドラッグは始まりのポーズへ戻す（それまでの位置は当てない）。形のギズモもドラッグの前へ戻す
                drag_at = None;
                gizmo::release(app, false);
                // ペンで掴んでいた形のギズモは、ペンが触れたままの次の点で掴み直さず・描き始めない（離すまで待つ）
                if let Some(crate::fillfx::gizmo::Source::Pen(id)) =
                    app.fillfx.drag.as_ref().map(|d| d.source)
                {
                    app.view3d.input.pen_once = Some(id);
                }
                crate::fillfx::gizmo::release(app, false);
                app.view3d.input.nav = None;
            }
            Event::WindowFocused(false) => {
                // フォーカスを失ったら、そこまでを確定する（離したのを受け取れないので）
                finish(app, false);
                app.path_finish_drag();
                app.view3d.input.pen_once = None;
                flush(app, &mut drag_at);
                gizmo::release(app, true);
                // 形のギズモは離したのを受け取れないので、ドラッグの前に戻す（履歴にも残さない）
                drag_at = None;
                crate::fillfx::gizmo::release(app, false);
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
    if app.path.drag.is_some_and(|d| d.source == StrokeSource::Mouse && d.surface)
        && !primary
        && !events
            .iter()
            .any(|e| matches!(e, Event::PointerButton { pressed: true, .. }))
    {
        app.path_finish_drag();
    }
    if app.view3d.pose.drag.is_some()
        && !primary
        && !events
            .iter()
            .any(|e| matches!(e, Event::PointerButton { pressed: true, .. }))
    {
        gizmo::release(app, true);
    }
    if app
        .fillfx
        .drag
        .as_ref()
        .is_some_and(|d| d.source == crate::fillfx::gizmo::Source::Mouse)
        && !primary
        && !events
            .iter()
            .any(|e| matches!(e, Event::PointerButton { pressed: true, .. }))
    {
        crate::fillfx::gizmo::release(app, true);
    }
    if !any_down {
        app.view3d.input.nav = None;
    }
    crate::stencil::settle(app, any_down);
}

/// 面の点のまわりの、ブラシの半径の円（面の接平面の円を画面へ写した楕円）の頂点。画面の外・カメラの後ろにかかれば None。
fn ring_points(
    view: &CameraView,
    rect: Rect,
    position: Vec3,
    normal: Vec3,
    radius: f32,
) -> Option<Vec<Pos2>> {
    // 接平面の 2 つの軸
    let helper = if normal.y.abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let u = normal.cross(helper).normalize_or_zero();
    let v = normal.cross(u);
    let screen_radius = view.world_radius_to_screen(position, radius);
    let segments = ((screen_radius * 0.8).ceil() as usize).clamp(24, 96);
    let mut points = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let a = i as f32 / segments as f32 * std::f32::consts::TAU;
        let p = position + (u * a.cos() + v * a.sin()) * radius;
        let s = view.to_screen(p)?;
        points.push(Pos2::new(rect.left() + s.x, rect.top() + s.y));
    }
    Some(points)
}

/// 二重の線の円（外が黒、中が色）。
fn draw_ring(painter: &egui::Painter, points: Vec<Pos2>, color: Color32, black_alpha: u8) {
    painter.add(egui::Shape::line(
        points.clone(),
        Stroke::new(3.0, Color32::from_black_alpha(black_alpha)),
    ));
    painter.add(egui::Shape::line(points, Stroke::new(1.2, color)));
}

/// 対称の線の色（2D の軸と同じ水色）。
fn symmetry_color(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(115, 209, 255, alpha)
}

/// 面の上の点がカメラから見えるか（表向きで、手前に別の面が無い）。
fn seen_from_camera(geometry: &SurfaceGeometry, camera: Vec3, point: &SurfaceHit) -> bool {
    let to = point.position - camera;
    let distance = to.length();
    if distance <= 0.0 || point.normal.dot(-to) <= 0.0 {
        return false;
    }
    let epsilon = (geometry.bounds().size().length() * 1e-5).max(1e-7);
    match geometry.raycast(Ray::new(camera, to / distance), false, distance + epsilon) {
        None => true,
        Some(first) => first.triangle == point.triangle || first.distance >= distance - epsilon,
    }
}

/// 今効く 3D の対称（描いているあいだはそのストロークに固めたもの）。
fn active_symmetry(app: &AppState) -> Option<SurfaceSymmetrySetup> {
    if app.view3d.input.stroke.is_some() {
        app.view3d.input.symmetry
    } else {
        app.sel.symmetry.surface.setup()
    }
}

/// ブラシのカーソル: ポインタの下の面の、ブラシの半径の円（面の接平面の円を画面へ写した楕円）。白と黒の二重の線。対称の写しの
/// 面の上にも水色の円を出す（カメラから見えない所は薄く）。面に当たらなければ描かずに false。
pub fn draw_cursor(ui: &Ui, app: &AppState, rect: Rect, pointer: Pos2) -> bool {
    let Some(model) = &app.view3d.model else {
        return false;
    };
    let view = camera_view(app, rect);
    let Some(hit) = pick(&model.geometry, &view, local(rect, pointer)) else {
        return false;
    };
    let radius = world_radius(&model.geometry, app.brush.radius as f64, app.doc.width());
    let Some(points) = ring_points(&view, rect, hit.position, hit.normal, radius) else {
        return false;
    };
    let painter = ui.painter_at(rect);
    draw_ring(&painter, points, Color32::from_white_alpha(230), 140);
    // 写しのカーソル（描いている最中は、3D のストローク以外では出さない）
    let Some(sym) = active_symmetry(app) else {
        return true;
    };
    if app.view3d.material != hit.material {
        return true;
    }
    for copy in copy_hits(
        &model.geometry,
        &hit,
        sym.mirror.as_ref(),
        sym.radial.as_ref(),
        radius,
    ) {
        let alpha = if seen_from_camera(&model.geometry, view.position, &copy) {
            242
        } else {
            100
        };
        if let Some(points) = ring_points(&view, rect, copy.position, copy.normal, radius) {
            draw_ring(&painter, points, symmetry_color(alpha), alpha / 2);
        }
    }
    true
}

/// 線 1 本（外が黒の細い影、中が水色）。画面の外の端点は、点どうしを結ぶだけ（クリップは painter が行う）。
fn draw_line(painter: &egui::Painter, a: Pos2, b: Pos2) {
    painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(90)));
    painter.line_segment([a, b], Stroke::new(1.5, symmetry_color(204)));
}

/// 3D ビューの上に、対称の面（ミラーの四角）と放射状の軸、クローンの元の印を描く（絵の上、カーソルの下）。
pub fn draw_overlays(ui: &Ui, app: &AppState, rect: Rect) {
    let Some(model) = &app.view3d.model else {
        return;
    };
    let painter = ui.painter_at(rect);
    let view = camera_view(app, rect);
    let to_screen = |p: Vec3| {
        view.to_screen(p)
            .map(|s| Pos2::new(rect.left() + s.x, rect.top() + s.y))
    };
    if app.sel.symmetry.surface.show_plane {
        if let Some(sym) = active_symmetry(app) {
            let bounds = model.geometry.bounds();
            let reach = (bounds.extents.max_element() * 1.25).max(1e-4);
            if let Some(plane) = sym.mirror {
                // 箱の中心を面へ落とした点のまわりの四角
                let center = bounds.center - plane.normal * plane.signed_distance(bounds.center);
                let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(a, b)| {
                    to_screen(center + plane.axis_u * (a * reach) + plane.axis_v * (b * reach))
                });
                if corners.iter().all(|c| c.is_some()) {
                    let c: Vec<Pos2> = corners.iter().flatten().copied().collect();
                    painter.add(egui::Shape::convex_polygon(
                        c.clone(),
                        symmetry_color(26),
                        Stroke::NONE,
                    ));
                    for i in 0..4 {
                        draw_line(&painter, c[i], c[(i + 1) % 4]);
                    }
                }
            }
            if let Some(radial) = sym.radial {
                let along = radial.axis * reach;
                if let (Some(a), Some(b)) = (
                    to_screen(radial.origin - along),
                    to_screen(radial.origin + along),
                ) {
                    draw_line(&painter, a, b);
                }
            }
        }
    }
    // クローンの元（十字）
    if clone_active(app) {
        if let Some(source) = app.view3d.clone.source_for(&model.geometry) {
            if let Some(p) = to_screen(source.position) {
                let accent = crate::ui::theme::ACCENT;
                painter.line_segment(
                    [p - egui::vec2(7.0, 0.0), p + egui::vec2(7.0, 0.0)],
                    Stroke::new(3.0, Color32::from_black_alpha(160)),
                );
                painter.line_segment(
                    [p - egui::vec2(0.0, 7.0), p + egui::vec2(0.0, 7.0)],
                    Stroke::new(3.0, Color32::from_black_alpha(160)),
                );
                painter.line_segment(
                    [p - egui::vec2(6.0, 0.0), p + egui::vec2(6.0, 0.0)],
                    Stroke::new(1.5, accent),
                );
                painter.line_segment(
                    [p - egui::vec2(0.0, 6.0), p + egui::vec2(0.0, 6.0)],
                    Stroke::new(1.5, accent),
                );
            }
        }
    }
}
