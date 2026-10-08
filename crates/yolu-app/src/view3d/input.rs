//! 3D ビューの入力（Unity 版の 3D ビューの操作と同じ）:
//! - 左ドラッグで面に描く（ブラシ・消しゴム。ペンの筆圧も）。ほかのテクスチャセットの面からは描き始めない。
//! - 右ドラッグで回す（動かさずに離すとスポイト。ポリゴン塗りつぶしのツールはアイランドのメニュー）。右を押している間の W/A/S/D/Q/E は視点の移動
//!   （Shift で速く）。Alt + 左ドラッグはスナップ回転（軸の向きの 15° 以内に入ったらその向きへ吸い付く）。中ドラッグか Space + 左ドラッグでパン、
//!   ホイールで寄る・引く。Ctrl+Space + 左ドラッグは左右に動かして寄る・引く（動かさずに離すと寄る、Alt を足すと引く）。クローンのブラシでは、
//!   Alt + 左を動かさずに離すと、そこがクローンの元（動かせばスナップ回転）。修飾は押しの始めに持っているもので決める。
//! - ペンはマウスと同じ決まり: サイドボタンを押した接触は右ボタン、Alt・Space・Ctrl+Space を押した接触は左ボタンにそれらを足したもの。
//!   描くのは、修飾もサイドボタンも無いペン先の接触だけ。行き先は触れた最初の点で決めて、離すまで変えない（`pen::PenPress`）。
//! - ぼかし・指先・クローンと 3D の対称（ミラー・放射状）は、面のストロークに通す（core の `SurfaceStrokeOptions`）。
//! - ストロークを取り残さない: 離す・Esc（捨てる）・ウィンドウのフォーカスを失う（そこまでを確定）・ボタンを離したのを取りこぼす で必ず終える。
//!   ストロークの間はカメラもモデルも動かさない（区画の投影の画素を覚えて使うので）。
//! - 速い動き（1 回の入力の区間が長い）でもストロークを捨てない: 面のストロークは、1 回の入力とフレームごとに決まった数までダブを
//!   塗り、残りを持ち越す（`SurfaceStroke::paint_queued`）。持ち越しがあればフレームを続けて頼み、離したら残りを塗ってから確定する。
//!
//! 画面の点はタブの中身の左上からの egui の点。core のカメラも同じ点の大きさで作る（ストロークの間隔は Unity 版と同じく画面の点）。

use egui::{Color32, Event, Key, Modifiers, PointerButton, Pos2, Rect, Stroke, Ui};
use yolu_core::geometry::{
    copy_hits, pick, world_radius, CameraView, Ray, SurfaceCloneSource, SurfaceEffect,
    SurfaceGeometry, SurfaceHit, SurfaceStroke, SurfaceStrokeOptions, SurfaceSymmetrySetup,
    SURFACE_DABS_PER_EVENT,
};
use yolu_core::glam::{Vec2, Vec3};

use super::{gizmo, Nav};
use crate::engine::BrushEffect;
use crate::gesture::{self, ZoomDrag};
use crate::notice::Source;
use crate::pen::{PenPress, PenSample, PressKind};
use crate::state::{AppState, StrokeSource};
use crate::tools::input::Surface;

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

/// 今のツールがクローンのブラシか（元を決められる）。
fn clone_active(app: &AppState) -> bool {
    app.tool.paints()
        && !app.tool.erases()
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
            app.info(
                Source::View3d,
                app.lang
                    .pick("クローンの元を決めました。", "Clone source set."),
            );
        }
        _ => {
            app.refuse(
                Source::View3d,
                app.lang.pick(
                    "今のテクスチャセットの面ではありません。",
                    "Not a surface of the active texture set.",
                ),
            );
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
    // ベイクのウィンドウでアイランドを選んでいる間は、押した面のアイランドを選ぶだけ（ツールを使わない）
    if crate::bake::overlap::press(app, crate::region::tools::Where::Surface(rect), at) {
        return;
    }
    match app.tool.def().surface {
        // スポイトは押した面の値を取るだけ（3D の Alt は回転なので、描くツールの一時的なスポイトは 2D だけ）
        Surface::Pick => {
            crate::eyedrop::pick_surface(app, rect, at);
            return;
        }
        // パスのツールは、押した面の点（掴む・差し込む・足す）。ストロークは持たず、点のドラッグだけが続く
        Surface::Path => {
            crate::pathtool::surface::press(app, rect, at, source);
            return;
        }
        // 範囲のツール（バケツ・ポリゴン塗りつぶし・ID の色で選択）は、点でなく押した面の範囲を使う
        Surface::Region => {
            if app.region.drag.is_none()
                && crate::region::tools::surface_press(app, rect, at, source)
            {
                app.view3d.input.stroke = Some(source);
                app.view3d.input.stroke_points = 0;
            }
            return;
        }
        // 選択・移動と変形・図形・グラデーションなどは 2D のキャンバスだけで使う（3D ビューで描き始めない）
        Surface::Unsupported => {
            app.refuse(
                Source::View3d,
                app.lang.pick(
                    "このツールは 2D のキャンバスで使います",
                    "This tool works on the 2D canvas",
                ),
            );
            return;
        }
        Surface::Paint => {}
    }
    let Some(model) = app.view3d.model.clone() else {
        return;
    };
    let view = camera_view(app, rect);
    let p = local(rect, at);
    let material = app.view3d.material;
    if material < 0 {
        app.refuse(Source::View3d, app.region_missing_reason());
        return;
    }
    if let Some(reason) = app.read_only_reason() {
        let text = crate::lang::refusals::read_only_set(app.lang, reason);
        app.refuse(Source::View3d, text);
        return;
    }
    if let Some(hit) = pick(&model.geometry, &view, p) {
        if hit.material != material {
            let name = model.material_name(hit.material as usize, app.lang);
            app.refuse(
                Source::View3d,
                app.lang.pick(
                    format!("ほかのテクスチャセット（{name}）の面です。"),
                    format!("Surface of another texture set ({name})."),
                ),
            );
            return;
        }
    }
    let Some(layer) = app.selected_layer else {
        app.refuse(
            Source::View3d,
            app.lang
                .pick("描くレイヤーがありません。", "No layer to paint on."),
        );
        return;
    };
    if let Some(reason) = app.paint_blocker() {
        app.refuse(Source::View3d, reason);
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
                    app.refuse(
                        Source::View3d,
                        app.lang.pick("クローンの元がありません", "No clone source"),
                    );
                    return;
                }
            },
        }
    };
    // 3D の対称。ストロークの始めに固める（途中で設定を変えても、このストロークには効かない）
    let symmetry = app.sel.symmetry.surface.setup();
    if symmetry.is_some() && matches!(effect, SurfaceEffect::Smudge | SurfaceEffect::Clone(_)) {
        app.refuse(
            Source::View3d,
            app.lang.pick(
                "指先・クローンでは対称を使えません",
                "Smudge and clone do not work with symmetry",
            ),
        );
        return;
    }
    // ステンシル: 置き場とカメラはストロークの始めに決める（面のテクセルの点を画面へ写して、そこの画像を読む）
    let (stencil_brush, surface_stencil) = match app.surface_stencil(rect) {
        Ok(Some((brush, surface))) => (Some(brush), Some(surface)),
        Ok(None) => (None, None),
        Err(e) => {
            app.notify(
                crate::notice::Kind::of_core(&e),
                Source::View3d,
                app.lang.with_reason(
                    app.lang.pick("描けません", "Cannot paint"),
                    app.lang.core_error(&e),
                ),
            );
            return;
        }
    };
    // 全部入りのブラシ。面のダブは筆先・ゆらぎ・質感・デュアル・フェード・傾き・回転・速さ・手ぶれ補正を受け取らない（効くのは基本の値・色・
    // 色の変化・消しゴム・筆圧・ステンシルと、効果のブラシ・3D の対称）
    let mut stroke = match app.begin_paint_stroke_with(layer, eraser, stencil_brush) {
        Ok(s) => s,
        Err(e) => {
            app.notify(
                crate::notice::Kind::of_core(&e),
                Source::View3d,
                app.lang.with_reason(
                    app.lang.pick("描けません", "Cannot paint"),
                    app.lang.core_error(&e),
                ),
            );
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
            projection: app.view3d.projection,
            projection_memory: None,
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
            // 重なった UV に描いたら、セットごとに 1 度だけ知らせる（片側だけには描けない）
            crate::uv_wireframe::overlap::note_surface(app, rect, at);
        }
        Err(e) => {
            app.doc.cancel_stroke(stroke);
            app.fail(Source::View3d, app.lang.surface_error(&e));
        }
    }
}

/// 対称の写しが塗られなかった理由を知らせる（全部塗れていれば何もしない）。
fn note_symmetry(app: &mut AppState, s: &SurfaceStroke) {
    if let Some(outcome) = s.symmetry_note() {
        app.warn(Source::View3d, app.lang.mirror_note(outcome));
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
                app.warn(Source::View3d, app.lang.mirror_note(outcome));
            }
            crate::uv_wireframe::overlap::note_surface(app, rect, at);
        }
        Err(e) => abandon(app, &e),
    }
}

/// 持ち越したダブを、このフレームの分（入力 1 回と同じ数）だけ塗る。まだ残れば次のフレームを頼む（動かさずに押しているだけでも
/// 塗り進める）。
fn paint_queued(app: &mut AppState, ctx: &egui::Context) {
    let (Some(stroke), Some(surface)) = (app.stroke.as_mut(), app.view3d.input.surface.as_mut())
    else {
        return;
    };
    if surface.queued() == 0 {
        return;
    }
    match surface.paint_queued(&mut app.doc, stroke, SURFACE_DABS_PER_EVENT) {
        Ok(_) => {
            if surface.queued() > 0 {
                ctx.request_repaint();
            }
            if let Some(outcome) = surface.symmetry_note() {
                app.warn(Source::View3d, app.lang.mirror_note(outcome));
            }
        }
        Err(e) => abandon(app, &e),
    }
}

/// 塗れなかった（予算を超えた・ありえない長さの区間など）: 途中まで塗った画素も戻してストロークを取り消す。
fn abandon(app: &mut AppState, e: &yolu_core::geometry::SurfaceStrokeError) {
    if let Some(stroke) = app.stroke.take() {
        app.doc.cancel_stroke(stroke);
    }
    app.view3d.stroke_ended();
    app.fail(Source::View3d, app.lang.surface_error(e));
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
        app.info(
            Source::View3d,
            app.lang
                .pick("ストロークを取り消しました。", "Stroke cancelled."),
        );
    } else {
        let mut surface = surface;
        let last = surface
            .as_mut()
            .map(|s| s.finish(&mut app.doc, &mut stroke));
        match last {
            Some(Err(e)) => {
                app.doc.cancel_stroke(stroke);
                app.fail(Source::View3d, app.lang.surface_error(&e));
            }
            _ => {
                if let Some(note) = surface.as_ref().and_then(|s| s.note) {
                    app.warn(Source::View3d, app.lang.dab_refusal(note));
                }
                if let Some(s) = surface.as_ref() {
                    note_symmetry(app, s);
                    if s.stats.lost > 0 {
                        app.warn(Source::View3d, app.lang.smudge_lost());
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
                    Err(e) => app.notify(
                        crate::notice::Kind::of_core(&e),
                        Source::View3d,
                        app.lang.core_error(&e),
                    ),
                }
            }
        }
    }
    app.view3d.stroke_ended();
}

/// 押しの組み合わせから、ビューを動かす操作（右ボタン・ペンのサイドボタンは回す、中ボタンはパン、左は Ctrl+Space で拡縮・Space でパン・
/// Alt でスナップ回転）。修飾は押しの始めに持っているもの。どれにも当たらなければ None（左は描く）。
fn nav_of(button: PointerButton, m: &Modifiers, space: bool) -> Option<Nav> {
    use crate::keymap::Operation;
    // 組み合わせは `keymap::GESTURES` の表
    match crate::keymap::gesture("view3d", button, m, space)? {
        Operation::Orbit => Some(Nav::Orbit),
        Operation::SnapOrbit => Some(Nav::SnapOrbit),
        Operation::Pan => Some(Nav::Pan),
        Operation::Zoom => Some(Nav::Zoom),
        _ => None,
    }
}

/// ビューを動かす操作を始める（マウスもペンも）。動かさずに離したときの行き先も、ここで決める: クローンのブラシで Alt + 左なら元を決める、
/// 右ボタン（ペンのサイドボタン）ならスポイト（ポリゴン塗りつぶしのツールのときは、アイランドのメニュー）。動かせば、そのまま回す。
#[allow(clippy::too_many_arguments)]
fn nav_press(
    app: &mut AppState,
    rect: Rect,
    source: StrokeSource,
    nav: Nav,
    button: PointerButton,
    pos: Pos2,
    m: &Modifiers,
    space: bool,
) {
    use crate::keymap::Operation;
    app.view3d.input.navigation = Some(super::navigation::Drag::new(
        &app.view3d,
        app.prefs.settings.navigation,
        pos,
    ));
    app.view3d.input.nav = Some((nav, button));
    if nav == Nav::Zoom {
        app.view3d.input.zoom = Some(ZoomDrag::new(pos, m.alt));
    }
    app.view3d.input.clone_press = None;
    app.view3d.input.eyedrop = None;
    match crate::keymap::click_gesture("view3d", button, m, space) {
        Some(Operation::CloneSource) if clone_active(app) => {
            app.view3d.input.clone_press = Some(pos);
        }
        Some(Operation::Pick) if app.tool != crate::state::Tool::PolygonFill => {
            let sample = crate::eyedrop::sample_surface(app, rect, pos);
            app.view3d.input.eyedrop = Some(crate::eyedrop::RightPress {
                source,
                at: pos,
                sample,
            });
        }
        _ => {}
    }
}

/// ポインタ・ペンが動いた（`previous` は前の位置）。
fn nav_move(app: &mut AppState, rect: Rect, pos: Pos2, previous: Pos2) {
    if app
        .view3d
        .input
        .clone_press
        .is_some_and(|start| start.distance(pos) > CLICK_DISTANCE)
    {
        app.view3d.input.clone_press = None; // 動かした: 回すだけ
    }
    if app
        .view3d
        .input
        .eyedrop
        .is_some_and(|press| press.at.distance(pos) > CLICK_DISTANCE)
    {
        app.view3d.input.eyedrop = None; // 動かした: 回すだけ（スポイトにしない）
    }
    let Some((nav, _)) = app.view3d.input.nav else {
        return;
    };
    let d = pos - previous;
    match nav {
        Nav::Orbit | Nav::SnapOrbit | Nav::Pan => {
            super::navigation::move_by(app, rect, nav, d.x, d.y)
        }
        Nav::Zoom => {
            if let Some(mut zoom) = app.view3d.input.zoom {
                let dx = zoom.moved_to(pos);
                // 動かさずに離せば寄る（クリック）なので、少しの揺れでは動かさない
                if !zoom.is_click() {
                    super::navigation::move_by(
                        app,
                        rect,
                        Nav::Zoom,
                        dx / gesture::POINTS_PER_NOTCH,
                        0.0,
                    );
                }
                app.view3d.input.zoom = Some(zoom);
            }
        }
    }
}

/// ボタン（ペンの押し）を離した。`button` が動かしていた操作のものなら終える。動かさずに離した拡縮は寄る（Alt を押して押していたら引く）。
/// 左を動かさずに離したクローンの元の指定は、ここで決める。
fn nav_release(app: &mut AppState, rect: Rect, pos: Pos2, button: PointerButton) {
    if app.view3d.input.nav.is_some_and(|(_, b)| b == button) {
        if let Some(zoom) = app.view3d.input.zoom.take() {
            if zoom.is_click() {
                let sign = if zoom.out { -1.0 } else { 1.0 };
                super::navigation::move_by(
                    app,
                    rect,
                    Nav::Zoom,
                    sign * gesture::CLICK_NOTCHES,
                    0.0,
                );
            }
        }
        app.view3d.input.nav = None;
        app.view3d.input.navigation = None;
    }
    if button == PointerButton::Primary {
        if let Some(start) = app.view3d.input.clone_press.take() {
            if start.distance(pos) <= CLICK_DISTANCE {
                set_clone_source(app, rect, start);
            }
        }
    }
    // 右ボタンを動かさずに離した: 離した所の面の値を取る
    if button == PointerButton::Secondary {
        if let Some(press) = app.view3d.input.eyedrop.take() {
            if press.at.distance(pos) <= CLICK_DISTANCE {
                crate::eyedrop::pick_surface(app, rect, pos);
            }
        }
    }
}

/// ビューを動かす操作の途中を全部やめる。
fn nav_cancel(app: &mut AppState) {
    app.view3d.input.navigation = None;
    app.view3d.input.nav = None;
    app.view3d.input.zoom = None;
    app.view3d.input.clone_press = None;
    app.view3d.input.eyedrop = None;
}

/// このフレームの入力の前提。
struct Frame {
    /// 押しを始めてはいけない（ポップアップ・設定のパネル・ドックのタブの見出しをつかんでいる・押しがほかの部品のもの）。
    press_blocked: bool,
    modifiers: Modifiers,
    space: bool,
}

/// ペンの 1 点。触れた最初の点で行き先を決め（`press_kind`）、離すまで変えない。ペンで描くのは、修飾もサイドボタンも無いペン先の接触だけ。
/// 形のギズモをこのペンで掴んでいる間は、点を `drag_at` に溜め（1 フレームに 1 回当てる）、離したら確定する。
fn pen_sample(
    ui: &Ui,
    app: &mut AppState,
    rect: Rect,
    s: &PenSample,
    frame: &Frame,
    drag_at: &mut Option<Pos2>,
) {
    let p = s.pos_points(ui.ctx().pixels_per_point());
    let source = StrokeSource::Pen(s.pointer_id);
    if app
        .fillfx
        .drag
        .as_ref()
        .is_some_and(|d| d.source == crate::fillfx::gizmo::Source::Pen(s.pointer_id))
    {
        if s.contact {
            *drag_at = Some(p);
        } else {
            if let Some(at) = drag_at.take() {
                let m = &frame.modifiers;
                crate::fillfx::gizmo::drag_to(app, rect, at, m.shift, m.command);
            }
            crate::fillfx::gizmo::release(app, true);
            app.view3d.input.pen_press = None;
        }
        return;
    }
    if app
        .fillfx
        .point_drag
        .as_ref()
        .is_some_and(|d| d.in_3d && d.source == crate::fillfx::gizmo::Source::Pen(s.pointer_id))
    {
        if s.contact {
            *drag_at = Some(p);
        } else {
            if let Some(at) = drag_at.take() {
                crate::fillfx::points::drag_to(app, rect, at);
            }
            crate::fillfx::points::release(app, true);
            app.view3d.input.pen_press = None;
        }
        return;
    }
    let press = match app.view3d.input.pen_press {
        Some(press) if press.id == s.pointer_id => press,
        // ほかのペン（別の ID）の押しが続いている間は、この点を使わない
        Some(_) => return,
        None if s.contact => {
            let kind = press_kind(ui, app, rect, p, s, frame);
            app.view3d.input.pen_press = Some(PenPress {
                id: s.pointer_id,
                kind,
                last: p,
            });
            match kind {
                // 3D のサイドボタンは右ボタンとして `View`（スポイトの印は `nav_press` が持つ）
                PressKind::Ignored | PressKind::Eyedrop => {}
                PressKind::View => {
                    let button = if s.barrel {
                        PointerButton::Secondary
                    } else {
                        PointerButton::Primary
                    };
                    if let Some(nav) = nav_of(button, &frame.modifiers, frame.space) {
                        nav_press(
                            app,
                            rect,
                            source,
                            nav,
                            button,
                            p,
                            &frame.modifiers,
                            frame.space,
                        );
                    }
                    // サイドボタン（右ボタン）を動かさずに離したら、ポリゴン塗りつぶしのアイランドのメニュー（ほかのツールはスポイト。`nav_press`）
                    if s.barrel && !frame.modifiers.any() && !frame.space {
                        crate::bake::overlap::menu_press(
                            app,
                            crate::region::tools::Where::Surface(rect),
                            p,
                        );
                    }
                }
                PressKind::Tool => {
                    if crate::fillfx::gizmo::press(
                        app,
                        rect,
                        p,
                        crate::fillfx::gizmo::Source::Pen(s.pointer_id),
                    ) || crate::fillfx::points::press(
                        app,
                        rect,
                        p,
                        crate::fillfx::gizmo::Source::Pen(s.pointer_id),
                    ) {
                        // 形のギズモのハンドルの上・点の編集: 描かずにドラッグを始める
                    } else if app.tool.def().surface == Surface::Path {
                        // パスのツール: 押す・動く・離すを、点を足す・掴む・動かすにする
                        crate::pathtool::surface::pen_sample(
                            app,
                            rect,
                            p,
                            s.pointer_id,
                            true,
                            true,
                        );
                    } else {
                        begin(app, rect, p, s.pressure, source, s.eraser);
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
            PressKind::Ignored | PressKind::Eyedrop => {}
            PressKind::View => nav_move(app, rect, p, press.last),
            PressKind::Tool => {
                if app.view3d.input.stroke == Some(source) {
                    add(app, rect, p, s.pressure);
                } else if app.path.pen_in(true) {
                    crate::pathtool::surface::pen_sample(app, rect, p, s.pointer_id, true, false);
                }
            }
        }
        app.view3d.input.pen_press = Some(PenPress { last: p, ..press });
    } else {
        match press.kind {
            PressKind::Ignored | PressKind::Eyedrop => {}
            PressKind::View => {
                if let Some((_, button)) = app.view3d.input.nav {
                    nav_release(app, rect, p, button);
                }
                crate::bake::overlap::menu_release(
                    app,
                    ui.ctx(),
                    crate::region::tools::Where::Surface(rect),
                    p,
                );
            }
            PressKind::Tool => {
                if app.view3d.input.stroke == Some(source) {
                    finish(app, false);
                }
                if app.path.pen_in(true) {
                    crate::pathtool::surface::pen_sample(app, rect, p, s.pointer_id, false, false);
                }
            }
        }
        app.view3d.input.pen_press = None;
    }
}

/// ペンが触れた最初の点の行き先。ビューを動かす（サイドボタン・Alt・Space・Ctrl+Space）・何もしない（押した所が別の部品・ビューを動かして
/// いる最中・ステンシルを動かしている間・修飾を押したブラシと消しゴム）・ツール。ステンシルを動かす押しは、同じ押しの egui のポインタの
/// 代わりの入力をステンシルが取るので、ビューを動かす判定より先に手放す（マウスの押しと同じく、ステンシルだけが動く）。
fn press_kind(
    ui: &Ui,
    app: &AppState,
    rect: Rect,
    p: Pos2,
    s: &PenSample,
    frame: &Frame,
) -> PressKind {
    if frame.press_blocked
        || !on_top(ui, rect, p)
        || app.view3d.input.nav.is_some()
        || app.view3d.input.stroke.is_some()
        || app.stencil.handling()
    {
        return PressKind::Ignored;
    }
    let button = if s.barrel {
        PointerButton::Secondary
    } else {
        PointerButton::Primary
    };
    if nav_of(button, &frame.modifiers, frame.space).is_some() {
        return PressKind::View;
    }
    let m = &frame.modifiers;
    if app.tool.paints() && (m.shift || gesture::ctrl(m)) {
        return PressKind::Ignored;
    }
    PressKind::Tool
}

/// 入力を当てる（rect はタブの中身の表示域。`foreign` はこの押しが egui でほかの部品のものか）。
pub fn handle(ui: &mut Ui, app: &mut AppState, rect: Rect, pen: &[PenSample], foreign: bool) {
    let ctx = ui.ctx().clone();
    // ストロークの札をほか（キャンバスの Esc・フォーカスを失ったとき）が手放したら、こちらも終える
    if app.view3d.input.stroke.is_some() && app.stroke.is_none() && app.region.drag.is_none() {
        app.view3d.stroke_ended();
    }
    if app.view3d.model.is_none() {
        nav_cancel(app);
        app.view3d.input.pen_press = None;
        return;
    }
    let blocked = app.popup.is_some() || app.ui.popup_was_open;
    app.region.modifiers = ui.input(|i| i.modifiers);
    // 設定のパネルを開いているあいだの押しは、パネルの外でも 3D に使わない（パネルは外の押しで閉じる。その押しが描き始め・回し始めに
    // ならないように。このフレームの押しで閉じるときも、パネルはこの後に描くので開いている）。ホイールは使える。ドックのタブの見出しを
    // つかんでいる間・離した直後と、押しがほかの部品のものであるときも、描き始めも回し始めもしない
    let press_blocked =
        blocked || app.view3d.display.settings_open || app.dock_grabbed() || foreign;
    super::navigation::shortcut(ui, app, rect, foreign);
    // 右ボタンを押している間の W/A/S/D/Q/E は、視点の移動
    super::navigation::fly(ui, app);
    let events = ui.input(|i| i.events.clone());
    // ポーズのモードでは描かない（左ボタンはギズモと骨を選ぶ。ペンの点は描くのに使わない）
    let pose_mode = app.view3d.pose.mode;
    // ポーズのモードの間はペンの点を見ないので、押している印も持ち越さない（離したのを見落とした印が次の押しを止めない）
    if pose_mode {
        app.view3d.input.pen_press = None;
    }
    let pen: &[PenSample] = if pose_mode { &[] } else { pen };
    let (snap, shift, modifiers) =
        ui.input(|i| (i.modifiers.command, i.modifiers.shift, i.modifiers));
    // パスのツールの取っ手のドラッグ（Alt で折る・Ctrl で両方を伸ばす）と、点のダブルクリック
    app.path.input = crate::pathtool::PathInputState {
        alt: modifiers.alt,
        ctrl: modifiers.command,
        shift: modifiers.shift,
        now: Some(ui.input(|i| i.time)),
    };
    let space = ui.input(|i| crate::keymap::hold_down(i, "view.pan_hold"))
        && !ctx.egui_wants_keyboard_input();
    // ギズモのドラッグは、1 フレームに何度ポインタが動いても、最後の位置を 1 回だけ当てる（1 回ごとにスキニング・refit・
    // モデルの組み直しが走るので、高いポーリングのマウスやペンでは、途中の位置は描かれずに捨てられるだけ）。ボタンを離す・Esc・
    // フォーカスを失うの前には、そこまでの位置を当ててから終える
    let mut drag_at: Option<Pos2> = None;
    let flush = |app: &mut AppState, drag_at: &mut Option<Pos2>| {
        if let Some(at) = drag_at.take() {
            gizmo::drag_to(app, rect, at, snap);
            // 塗りつぶしの形のギズモ（Shift で両側、Ctrl で刻み）
            crate::fillfx::gizmo::drag_to(app, rect, at, shift, snap);
            // 点のグラデーションの点
            crate::fillfx::points::drag_to(app, rect, at);
        }
    };

    // ペン（Windows Ink）。ペンの押し（触れてから離すまで）は、ペンの点だけで扱い、同じ押しが egui のポインタの押しとして二重に来ても使わない
    let pen_frame = app.view3d.input.pen_press.is_some() || pen.iter().any(|s| s.contact);
    let frame = Frame {
        press_blocked,
        modifiers,
        space,
    };
    for s in pen {
        pen_sample(ui, app, rect, s, &frame, &mut drag_at);
    }

    for event in &events {
        // Y を押しているあいだのドラッグはステンシルの置き場を動かす（描かない・回さない・パンしない。ポーズのモードでは描かない）
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
                    if pen_frame {
                        // ペンの押しは、ペンの点が持つ（これは同じ押しの egui のポインタの代わりの入力）
                        continue;
                    }
                    if let Some(nav) = nav_of(*button, m, space) {
                        nav_press(app, rect, StrokeSource::Mouse, nav, *button, pos, m, space);
                        // 右ボタンを動かさずに離したら、ポリゴン塗りつぶしのアイランドのメニュー（ほかのツールはスポイト。動かせば回すだけ）
                        if *button == PointerButton::Secondary && !m.any() && !space {
                            crate::bake::overlap::menu_press(
                                app,
                                crate::region::tools::Where::Surface(rect),
                                pos,
                            );
                        }
                    } else if *button == PointerButton::Primary && app.view3d.input.nav.is_none() {
                        if pose_mode {
                            if app.view3d.pose.drag.is_none() {
                                gizmo::press(app, rect, pos);
                            }
                        } else if !app.stencil.handling() {
                            // 形のギズモのハンドルの上・点のグラデーションの点を編集している間は、描かずにドラッグを始める
                            if !crate::fillfx::gizmo::press(
                                app,
                                rect,
                                pos,
                                crate::fillfx::gizmo::Source::Mouse,
                            ) && !crate::fillfx::points::press(
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
                    // ペンの押しの回す・パン・拡縮は、ペンの点が終える
                    if !pen_frame {
                        nav_release(app, rect, pos, *button);
                        if *button == PointerButton::Secondary {
                            crate::bake::overlap::menu_release(
                                app,
                                &ctx,
                                crate::region::tools::Where::Surface(rect),
                                pos,
                            );
                        }
                    }
                    if *button == PointerButton::Primary
                        && app.view3d.input.stroke == Some(StrokeSource::Mouse)
                    {
                        finish(app, false);
                    }
                    if *button == PointerButton::Primary {
                        crate::pathtool::surface::release(app, rect, pos, StrokeSource::Mouse);
                        flush(app, &mut drag_at);
                        gizmo::release(app, true);
                        if app
                            .fillfx
                            .drag
                            .as_ref()
                            .is_some_and(|d| d.source == crate::fillfx::gizmo::Source::Mouse)
                        {
                            crate::fillfx::gizmo::release(app, true);
                        }
                        if app
                            .fillfx
                            .point_drag
                            .as_ref()
                            .is_some_and(|d| d.source == crate::fillfx::gizmo::Source::Mouse)
                        {
                            crate::fillfx::points::release(app, true);
                        }
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
                if !pen_frame {
                    crate::pathtool::surface::moved(app, rect, pos, StrokeSource::Mouse);
                }
                if app.view3d.pose.drag.is_some()
                    || app
                        .fillfx
                        .drag
                        .as_ref()
                        .is_some_and(|d| d.source == crate::fillfx::gizmo::Source::Mouse)
                    || app
                        .fillfx
                        .point_drag
                        .as_ref()
                        .is_some_and(|d| d.in_3d && d.source == crate::fillfx::gizmo::Source::Mouse)
                {
                    drag_at = Some(pos);
                }
                // ペンの押しの回す・パン・拡縮は、ペンの点が動かす
                if !pen_frame {
                    nav_move(app, rect, pos, previous);
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
                super::navigation::wheel(app, rect, p, notches);
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
                crate::fillfx::gizmo::release(app, false);
                crate::fillfx::points::release(app, false);
                nav_cancel(app);
            }
            Event::WindowFocused(false) => {
                // フォーカスを失ったら、そこまでを確定する（離したのを受け取れないので）
                finish(app, false);
                app.path_finish_drag();
                // 点を矩形で選ぶドラッグは、離したのを受け取れないので捨てる（古い始点が次の離しで効かないように）
                if app.path.rect.is_some_and(|r| r.surface) {
                    app.path.rect = None;
                }
                app.view3d.input.pen_press = None;
                flush(app, &mut drag_at);
                gizmo::release(app, true);
                // 形のギズモ・点のドラッグは離したのを受け取れないので、ドラッグの前に戻す（履歴にも残さない）
                drag_at = None;
                crate::fillfx::gizmo::release(app, false);
                crate::fillfx::points::release(app, false);
                nav_cancel(app);
            }
            _ => {}
        }
    }
    flush(app, &mut drag_at);
    paint_queued(app, &ctx);
    // ボタンを離したのを取りこぼしたとき（ウィンドウの外で離したなど）も、押していなければ終える
    let (primary, any_down) = ui.input(|i| (i.pointer.primary_down(), i.pointer.any_down()));
    if app.view3d.input.stroke == Some(StrokeSource::Mouse)
        && !primary
        && !events
            .iter()
            .any(|e| matches!(e, Event::PointerButton { pressed: true, .. }))
    {
        finish(app, false);
    }
    if app
        .path
        .drag
        .is_some_and(|d| d.source == StrokeSource::Mouse && d.surface)
        && !primary
        && !events
            .iter()
            .any(|e| matches!(e, Event::PointerButton { pressed: true, .. }))
    {
        app.path_finish_drag();
    }
    // 点を矩形で選ぶドラッグも、取りこぼしたら最後の位置で確定する（2D のツールと同じ）。位置が無ければ捨てる
    if app
        .path
        .rect
        .is_some_and(|r| r.source == StrokeSource::Mouse && r.surface)
        && !primary
        && !events
            .iter()
            .any(|e| matches!(e, Event::PointerButton { pressed: true, .. }))
    {
        match app.view3d.input.last_pointer {
            Some(at) => crate::pathtool::surface::release(app, rect, at, StrokeSource::Mouse),
            None => app.path.rect = None,
        }
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
    // ペンが回している・パンしている・寄っている間は、egui のポインタが押していなくても続ける（ペンが離したときに終える）
    let pen_driven = app
        .view3d
        .input
        .pen_press
        .is_some_and(|p| p.kind == PressKind::View);
    if !any_down && !pen_driven {
        nav_cancel(app);
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

/// 画面の円の頂点（中心 center・半径 radius は画面の点）。
fn circle_points(center: Pos2, radius: f32) -> Vec<Pos2> {
    let segments = ((radius * 0.8).ceil() as usize).clamp(24, 96);
    (0..=segments)
        .map(|i| {
            let a = i as f32 / segments as f32 * std::f32::consts::TAU;
            center + egui::vec2(a.cos(), a.sin()) * radius
        })
        .collect()
}

/// ブラシのカーソル: ポインタのまわりの、塗る画面の円（半径はポインタの下の面の奥行きで、ブラシの半径を画面へ直したもの）。白と黒の
/// 二重の線。対称の写しの面の上にも水色の円を出す（カメラから見えない所は薄く）。面に当たらなければ描かずに false。
pub fn draw_cursor(ui: &Ui, app: &AppState, rect: Rect, pointer: Pos2) -> bool {
    let Some(model) = &app.view3d.model else {
        return false;
    };
    let view = camera_view(app, rect);
    let Some(hit) = pick(&model.geometry, &view, local(rect, pointer)) else {
        return false;
    };
    let radius = world_radius(&model.geometry, app.brush.radius as f64, app.doc.width());
    let screen_radius = view.world_radius_to_screen(hit.position, radius);
    if !(screen_radius > 0.0 && screen_radius.is_finite()) {
        return false;
    }
    let painter = ui.painter_at(rect);
    draw_ring(
        &painter,
        circle_points(pointer, screen_radius),
        Color32::from_white_alpha(230),
        140,
    );
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
