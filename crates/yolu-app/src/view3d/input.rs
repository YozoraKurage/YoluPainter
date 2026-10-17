//! 3D ビューの入力（Unity 版の 3D ビューの操作と同じ）:
//! - 左ドラッグで面に描く（ブラシ・消しゴム。ペンの筆圧も。指・ペンの Touch の力はマウスの道の筆圧に）。ほかのテクスチャセットの面からは描き始めない。
//!   Shift + 押しは、前のストロークの終点（面の点で覚えて、今の画面へ写す）から押した点までの直線（前が無ければ、押した点から向きを 45° 刻みで固定）。
//!   Ctrl を押した接触は、ペンは描かず、マウスは描く（2D のキャンバスと同じ）。
//! - 右ドラッグで回す（動かさずに離すとスポイト。ポリゴン塗りつぶしのツールはアイランドのメニュー）。右を押している間の W/A/S/D/Q/E は視点の移動
//!   （Shift で速く）。Alt + 右ドラッグはスナップ回転（ドラッグした向きへ、90° 隣の軸の視点に切り替える。Alt + 左は描き始めない）。中ドラッグか Space + 左ドラッグでパン、
//!   ホイールで寄る・引く。Ctrl+Space + 左ドラッグは左右に動かして寄る・引く（動かさずに離すと寄る、Alt を足すと引く）。クローンのブラシでは、
//!   Alt + 左を動かさずに離すと、そこがクローンの元（動かせば何もしない）。修飾は押しの始めに持っているもので決める。
//! - ペンはマウスと同じ決まり: サイドボタンを押した接触は右ボタン、Alt・Space・Ctrl+Space を押した接触は左ボタンにそれらを足したもの。
//!   描くのは、修飾もサイドボタンも無いペン先の接触だけ。行き先は触れた最初の点で決めて（`input::owner`。マウスの押しも同じ表）、離すまで変えない（`input::Press`）。
//! - ぼかし・指先・クローンと 3D の対称（ミラー・放射状）と 2D の対称（UV の平面。3D の写しの後に当てる）は、面のストロークに通す
//!   （core の `SurfaceStrokeOptions`）。
//! - 選択ペン・選択消しのツールと、クイックマスクが入っている間のブラシ・消しゴムは、面に塗って選択範囲を直す（2D のキャンバスと同じ。`quick`）。
//! - 長方形選択・楕円形選択・なげなわ・多角形選択・自動選択は、グラデーション・図形と同じく画面の上で引いて（自動選択は押して）、見えている面の
//!   テクセルの選択範囲にする（`select`。離したのを取りこぼしたときは何も選ばずにやめる）。
//! - ストロークを取り残さない: 離す・Esc（捨てる）・ウィンドウのフォーカスを失う（そこまでを確定）・ボタンを離したのを取りこぼす で必ず終える。
//!   ストロークの間はカメラもモデルも動かさない（区画の投影の画素を覚えて使うので）。
//! - 速い動き（1 回の入力の区間が長い）でもストロークを捨てず、画面を固めない: 面のストロークは、入力ではダブを並べるだけにして、塗るのは
//!   1 フレームに 1 回、時間の枠まで（`SurfaceStroke::paint_queued_until`。最初の 1 つは必ず塗る）。枠は、溜まった仕事の見込みで 8〜50 ms
//!   の間に決める（`pacing`。遅れを約 0.3 秒に収める）。残りは次のフレームに持ち越し、フレームを頼み続ける。離したあとも同じ枠で塗り続け
//!   （`release`。離したあとは最大の枠）、塗り終えたら確定する。
//! - 離したあと塗り終えるまでは「確定待ち」: 利用者は離した時点で描き終えたと思っているので、次の操作（押し・キー・ホイール・Esc・取り消しや
//!   保存などの操作・閉じる・ビューが隠れる・GPU を失う）が来たら、先に残りを全部塗って確定してから（`settle`・`finish`）、その操作をそのまま通す。
//!   離す前の Esc は、今までどおり線ごと捨てる。
//!
//! 画面の点はタブの中身の左上からの egui の点。core のカメラも同じ点の大きさで作る（ストロークの間隔は Unity 版と同じく画面の点）。

use std::time::Instant;

use egui::{Color32, Event, Key, Pos2, Rect, Stroke, Ui};
use yolu_core::geometry::{
    copy_hits, pick, world_radius, CameraView, Ray, SurfaceCloneSource, SurfaceEffect,
    SurfaceGeometry, SurfaceHit, SurfaceInput, SurfaceStroke, SurfaceStrokeOptions,
    SurfaceSymmetrySetup,
};
use yolu_core::glam::{DVec2, Vec2, Vec3};

use super::held::Held;
use super::model::ViewModel;
use super::{gizmo, pacing, Nav};
use crate::engine::{BrushEffect, BrushSettings, Tilt};
use crate::features::Feature;
use crate::gesture::{self, ZoomDrag};
use crate::input::{on_top, Item, Owner, Phase, PointerSample, Press, PressCtx, Stream};
use crate::notice::Source;
use crate::pen::PenSample;
use crate::state::key::{Modifiers, PointerButton};
use crate::state::{AppState, ShiftHold, StrokeSource, SHIFT_HOLD_POINTS};
use crate::tools::input::Surface;

pub(super) fn local(rect: Rect, p: Pos2) -> Vec2 {
    Vec2::new(p.x - rect.left(), p.y - rect.top())
}

/// ストロークの点に添えるペンの傾き・軸の回転と時刻（マウスは傾き・回転なし）。
#[derive(Clone, Copy, Default)]
struct PenState {
    tilt: Tilt,
    /// 度、画面で時計回り（回転を送れないペン・マウスは None）。
    rotation: Option<f32>,
    /// 秒。
    time: f64,
}

impl PenState {
    /// 点の傾き・回転・時刻（マウスの点は傾き・回転なし）。
    fn of(s: &PointerSample) -> PenState {
        PenState {
            tilt: s.tilt,
            rotation: s.rotation,
            time: s.time,
        }
    }

    /// core の入力の点: 傾きは画面の右・上へ倒れる向きを正のラジアンに（ペンの値は右・下が正）、回転は画面で反時計回りのラジアンに
    /// （2D のキャンバスを回さずに見たときと同じ向き）。
    fn input(self, at: Vec2, pressure: f32) -> SurfaceInput {
        const MAX: f64 = std::f64::consts::FRAC_PI_2 - 1e-6;
        let radians = |degrees: f32| (degrees as f64).to_radians().clamp(-MAX, MAX);
        SurfaceInput {
            tilt: DVec2::new(radians(self.tilt.x), -radians(self.tilt.y)),
            rotation: self.rotation.map_or(0.0, |d| -(d as f64).to_radians()),
            time: self.time,
            ..SurfaceInput::new(at, pressure.clamp(0.0, 1.0))
        }
    }
}

/// 今のカメラを表示域の大きさ（点）で見たもの。
pub fn camera_view(app: &AppState, rect: Rect) -> CameraView {
    app.view3d.view.camera.view(rect.width(), rect.height())
}

/// クリックとみなす、押してから離すまでに動いてよい距離（画面の点）。
const CLICK_DISTANCE: f32 = gesture::CLICK_MOVE;

/// Shift の直線で、前の終点（今の画面へ写した点）から押した点までに許す長さ（画面の点）。これを超える（カメラのすぐ脇の面など、ありえない遠さへ
/// 写る）ときは、前の終点が無いものとして押した点から始める。
const MAX_LINE: f32 = 8192.0;

/// 画面の点の下の面を、クローンの元にする（描くテクスチャセットの面だけ）。
fn set_clone_source(app: &mut AppState, rect: Rect, at: Pos2) {
    let Some(model) = app.view3d.model.clone() else {
        return;
    };
    let view = camera_view(app, rect);
    match pick(&model.geometry, &view, local(rect, at)) {
        Some(hit) if hit.material == app.view3d.material => {
            app.clone.set_source(hit);
            app.info(Source::View3d, app.lang.t("view3d-clone-source-set"));
        }
        _ => {
            app.refuse(Source::View3d, app.lang.t("view3d-not-a-surface-of-the"));
        }
    }
}

/// 3D の面にストロークを始める（ブラシ・消しゴム・効果のブラシ）。`shift` は Shift を押した押し（直線）。
#[allow(clippy::too_many_arguments)]
fn begin(
    app: &mut AppState,
    rect: Rect,
    at: Pos2,
    pressure: f32,
    source: StrokeSource,
    eraser: bool,
    pen: PenState,
    modifiers: Modifiers,
) {
    let shift = modifiers.shift;
    // ベイクのウィンドウでアイランドを選んでいる間は、押した面のアイランドを選ぶだけ（ツールを使わない）
    if crate::bake::overlap::press(app, crate::region::tools::Where::Surface(rect), at) {
        return;
    }
    // 編集・ポーズのモードでは描かない（ツールを使わない）
    if !app.mode.paints() {
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
                && app.region.leftover_drag.is_none()
                && crate::region::tools::surface_press(app, rect, at, source)
            {
                app.view3d.view.input.stroke = Some(source);
                app.view3d.view.input.stroke_points = 0;
            }
            return;
        }
        // グラデーション・図形・定規は、画面の上で引いて、離したときに見えている面へ写す。選択のツール（長方形・楕円・なげなわ・多角形・自動選択）は、
        // 同じ写し方で選択範囲にする
        Surface::Screen => {
            if app.tool.is_select() {
                super::select::press(app, rect, at, source, modifiers);
            } else {
                super::draft::press(app, rect, at, source, shift);
            }
            return;
        }
        // 選択ペン・選択消し: 面のダブの覆いを選択範囲に積む（下の、クイックマスクのブラシ・消しゴムと同じ道。描くレイヤーは要らない）
        Surface::Cover => {}
        // 移動と変形・ゆがみ・テキストは 2D のキャンバスだけで使う（3D ビューで描き始めない）
        Surface::Unsupported => {
            app.refuse(Source::View3d, app.lang.t("view3d-this-tool-works-on-the"));
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
    // 確かめの順は `stroke_gate` の頭（2D のキャンバスの `begin_stroke` と同じ。3D だけの前提が先）。選択ペンのツールの断りは、2D の選択ペンと同じ
    // 選択範囲の出どころ
    let refused_as = if app.tool.def().surface == Surface::Cover {
        Source::Selection
    } else {
        Source::Brush
    };
    if let Err(reason) = crate::stroke_gate::read_only(app) {
        app.refuse(refused_as, reason);
        return;
    }
    let pressed = pick(&model.geometry, &view, p);
    if let Some(hit) = &pressed {
        if refuse_other_set(app, refused_as, &model, hit) {
            return;
        }
    }
    // 選択ペンのツールは、押したときの Shift（追加）・Ctrl（削除）・ペンの消しゴムの端で、追加か削除かが決まる（2D のキャンバスと同じ）。クイックマスクが入って
    // いれば、ブラシ・消しゴムも選択ペン・選択消しとして働く（2D のキャンバスと同じ。選択範囲の UV の画素へ）
    if app.tool.def().surface == Surface::Cover {
        let erase = crate::selection::pen::erases(app.sel.pen_erase || eraser, modifiers);
        super::quick::begin(app, &model, rect, at, pressure, source, erase, false);
        return;
    }
    if app.sel.quick {
        let erase = eraser || app.tool.erases() || app.m2.erase_mode;
        super::quick::begin(app, &model, rect, at, pressure, source, erase, true);
        return;
    }
    let layer = match crate::stroke_gate::layer(app) {
        Ok(id) => id,
        Err(reason) => {
            app.refuse(Source::Brush, reason);
            return;
        }
    };
    let erase = app.stroke_settings(eraser).erase;
    // Shift: 前のストロークの終点を今の画面へ写した点から、押した点までを直線で描く。前の終点が無い（今のモデルの面でない・カメラの後ろ・
    // ありえない遠さ）ときは、押した点から向きを固定する
    let line = shift && app.tool.paints();
    let from = line
        .then(|| previous_screen(app, &model.geometry, &view, rect, at))
        .flatten();
    let first = from.unwrap_or(at);
    // 定規のスナップ: 押した点で寄せ先を決めて凍結し（ストロークの間は変えない）、最初の点も寄せる（2D と同じ式。決まった道なので、
    // 手ぶれ補正と曲線は切る）。直線は 2D と同じく、押した点が線の近く（`SNAP_POINTS`）のときだけ寄せる。平行線・同心円・パースは押した面の点から
    let press = screen_point(rect, at);
    let mut constraint = app.view3d_ruler_constraint(&view, Some(layer), press, pressed.as_ref());
    let first = match constraint.as_mut() {
        Some(c) => to_pos(rect, c.project(screen_point(rect, first))),
        None => first,
    };
    let Some(Opened {
        stroke,
        surface: s,
        symmetry,
    }) = open_stroke(
        app,
        Source::Brush,
        &model,
        rect,
        view,
        layer,
        eraser,
        constraint.is_some(),
        pen.input(local(rect, first), pressure),
    )
    else {
        return;
    };
    app.session.stroke = Some(stroke);
    app.view3d.view.input.stroke = Some(source);
    app.view3d.view.input.symmetry = symmetry;
    app.view3d.view.input.ruler_constraint = constraint;
    note_symmetry(app, &s);
    app.view3d.view.input.surface = Some(s);
    app.view3d.view.input.stroke_points = 1;
    remember_point(app, first, at, pressure, pen);
    app.view3d.view.input.last_hit = pick(&model.geometry, &view, local(rect, first));
    app.view3d.view.input.shift_hold = None;
    if !erase {
        app.color.remember();
    }
    app.session.mark_content_modified();
    // 重なった UV に描いたら、セットごとに 1 度だけ知らせる（片側だけには描けない）
    crate::uv_wireframe::overlap::note_surface(app, rect, at);
    if line {
        if from.is_some() {
            // 前の終点から押した点までの線（押した点も 1 つの入力）。線は押した点まで遅れずにまっすぐ届かせる（手ぶれ補正の遅れを詰め、曲線の
            // 待たせている区間も並べ切って、そこを角にする。続けてなぞる所には補正と曲線が効く。2D のキャンバスと同じ）
            add(app, rect, at, pressure, pen);
            let closed = app.view3d.view.input.surface.as_mut().map(|s| s.end_line());
            if let Some(Err(e)) = closed {
                abandon(app, &e);
            }
        }
        // 押した点のぶれを抑える。前の終点が無ければ、最初に動いた向きを 45° 刻みで固定する
        if app.view3d.view.input.stroke.is_some() {
            app.view3d.view.input.shift_hold =
                Some(ShiftHold::new((at.x as f64, at.y as f64), from.is_some()));
        }
    }
}

/// 押した面が今のテクスチャセットのものでなければ、その断りを `source` の出どころで出して true（面に描く・面に定規を作る押しが、ほかのテクスチャセットの
/// 面から始めない）。
pub(super) fn refuse_other_set(
    app: &mut AppState,
    source: Source,
    model: &ViewModel,
    hit: &yolu_core::geometry::SurfaceHit,
) -> bool {
    if hit.material == app.view3d.material {
        return false;
    }
    let name = model.material_name(hit.material as usize, app.lang);
    app.refuse(source, crate::stroke_gate::other_set_face(app.lang, &name));
    true
}

/// 表示域の画面の点（定規の点の空間。表示域の左上が原点）。
pub(super) fn screen_point(rect: Rect, p: Pos2) -> DVec2 {
    DVec2::new((p.x - rect.left()) as f64, (p.y - rect.top()) as f64)
}

/// 表示域の画面の点を、egui の点へ。
pub(super) fn to_pos(rect: Rect, p: DVec2) -> Pos2 {
    Pos2::new(rect.left() + p.x as f32, rect.top() + p.y as f32)
}

/// 面のストロークを始めた物: 文書のストロークの札・面のストローク・固めた 3D の対称。
pub(super) struct Opened {
    pub stroke: crate::engine::Stroke,
    pub surface: SurfaceStroke,
    pub symmetry: Option<SurfaceSymmetrySetup>,
}

/// 面のストロークを始める: 効果（消しゴムは色を塗る側。クローンは元の面の点が要る）・3D と 2D の対称・ステンシル・全部入りのブラシ
/// （筆先・ゆらぎ・質感・デュアル・入り抜き・手ぶれ補正・曲線も、2D と同じ式で面のダブに効く）で、文書のストロークと面のストロークを始める。
/// `guided` なら手ぶれ補正と曲線を切る（定規のスナップ・図形の輪郭のような決まった道。2D と同じ）。始められなければ理由を `source` の出どころで
/// 知らせて None（確かめの順は `stroke_gate` の頭の 6）。
#[allow(clippy::too_many_arguments)]
pub(super) fn open_stroke(
    app: &mut AppState,
    source: Source,
    model: &ViewModel,
    rect: Rect,
    view: CameraView,
    layer: crate::engine::LayerId,
    eraser: bool,
    guided: bool,
    input: SurfaceInput,
) -> Option<Opened> {
    let settings = app.stroke_settings(eraser);
    // 効果のブラシ（消しゴムは色を塗る側）。クローンは元の面の点が要る
    let effect = if settings.erase {
        SurfaceEffect::Paint
    } else {
        match app.m2.brush.effect {
            BrushEffect::Paint => SurfaceEffect::Paint,
            BrushEffect::Blur { .. } => SurfaceEffect::Blur,
            BrushEffect::Smudge { .. } => SurfaceEffect::Smudge,
            BrushEffect::Clone { .. } => match app.clone.source_for(&model.geometry) {
                Some(source) => SurfaceEffect::Clone(SurfaceCloneSource {
                    source,
                    destination: app.clone.destination_for(&model.geometry),
                }),
                None => {
                    app.refuse(source, app.lang.t("view3d-no-clone-source"));
                    return None;
                }
            },
        }
    };
    if let Err(reason) = crate::stroke_gate::symmetry_effect(
        app,
        crate::rulers::Place::View3d,
        layer,
        settings.erase,
    ) {
        app.refuse(source, reason);
        return None;
    }
    // 3D の対称と 2D の対称（UV の平面。3D の写しの後に当てる）。ストロークの始めに固める（途中で設定を変えても、このストロークには効かない）
    let (symmetry, canvas_symmetry) = {
        let active = app.rulers_active_for(crate::rulers::Place::View3d, Some(layer));
        (active.surface_symmetry, active.canvas_symmetry)
    };
    // ステンシル: 置き場とカメラはストロークの始めに決める（面のテクセルの点を画面へ写して、そこの画像を読む）
    let (stencil_brush, surface_stencil) = match app.surface_stencil(rect) {
        Ok(Some((brush, surface))) => (Some(brush), Some(surface)),
        Ok(None) => (None, None),
        Err(e) => {
            app.notify(
                crate::notice::Kind::of_core(&e),
                source,
                crate::stroke_gate::cannot_paint(app.lang, &app.lang.core_error(&e)),
            );
            return None;
        }
    };
    // 画面上のサイズで指定のブラシは、始めの点の下の面で、文書の画素 1（ブラシの半径の単位）が画面で何点かで半径を固める
    let scale = screen_points_per_pixel(app, model, &view, input.at);
    let mut brush = app.with_stroke_scale(scale, |app| app.stroke_brush(eraser));
    let settings = BrushSettings {
        radius: brush.base.radius,
        ..settings
    };
    brush.stencil = stencil_brush;
    if guided {
        brush.assist.stabilizer = 0.0;
        brush.assist.curve = false;
        brush.assist.post.on = false;
    }
    let mut stroke = match app.begin_stroke_with(layer, &brush) {
        Ok(s) => s,
        Err(e) => {
            app.notify(
                crate::notice::Kind::of_core(&e),
                source,
                crate::stroke_gate::cannot_paint(app.lang, &app.lang.core_error(&e)),
            );
            return None;
        }
    };
    let material = app.view3d.material;
    match SurfaceStroke::begin_input(
        &mut app.session.doc,
        &mut stroke,
        model.geometry.clone(),
        view,
        &settings,
        Some(material),
        input,
        SurfaceStrokeOptions {
            stencil: surface_stencil,
            symmetry,
            effect,
            projection: app.view3d.projection,
            projection_memory: None,
            canvas_symmetry,
        },
    ) {
        Ok(surface) => Some(Opened {
            stroke,
            surface,
            symmetry,
        }),
        Err(e) => {
            app.session.doc.cancel_stroke(stroke);
            app.fail(source, app.lang.surface_error(&e));
            None
        }
    }
}

/// 押した点の下の面で、文書の画素 1（ブラシの半径の単位）が画面で何点か（見つからない・使えない値は 1）。画面上のサイズで指定のブラシが、
/// ストロークの始めに半径を固めるのに使う（2D のキャンバスは表示の倍率）。
fn screen_points_per_pixel(
    app: &AppState,
    model: &ViewModel,
    view: &CameraView,
    at: yolu_core::glam::Vec2,
) -> f64 {
    match pick(&model.geometry, view, at) {
        Some(hit) => points_per_pixel_at(app, model, view, hit.position),
        None => 1.0,
    }
}

/// [`screen_points_per_pixel`] の、面の点（モデルの空間）が分かっているとき。
fn points_per_pixel_at(
    app: &AppState,
    model: &ViewModel,
    view: &CameraView,
    position: Vec3,
) -> f64 {
    let one = world_radius(&model.geometry, 1.0, app.session.doc.width());
    let points = f64::from(view.world_radius_to_screen(position, one));
    if points.is_finite() && points > 0.0 {
        points
    } else {
        1.0
    }
}

/// 対称の写しが塗られなかった理由を知らせる（全部塗れていれば何もしない）。
fn note_symmetry(app: &mut AppState, s: &SurfaceStroke) {
    if let Some(outcome) = s.symmetry_note() {
        app.warn(Source::View3d, app.lang.mirror_note(outcome));
    }
}

/// 前のストロークの終点を、今のカメラで画面へ写した点（Shift の直線の始め）。覚えていない・今のモデルの面でない（世代が違う）・カメラの後ろ・
/// 押した点 `to` から `MAX_LINE` より遠いときは None。画面の外・裏の面でも、画面の点として返す（塗りは、今の投影の決まりで見える面だけ）。
fn previous_screen(
    app: &AppState,
    geometry: &SurfaceGeometry,
    view: &CameraView,
    rect: Rect,
    to: Pos2,
) -> Option<Pos2> {
    let end = app.view3d.view.input.previous_end?;
    if end.revision != geometry.revision() || end.triangle as usize >= geometry.triangle_count() {
        return None;
    }
    let s = view.to_screen(end.position)?;
    let p = Pos2::new(rect.left() + s.x, rect.top() + s.y);
    (p.x.is_finite() && p.y.is_finite() && p.distance(to) <= MAX_LINE).then_some(p)
}

/// Shift で始めたストロークの点に、押した点のぶれの抑えと向きの固定を当てる（2D の `add_point` と同じ決まり）。
fn shifted(app: &mut AppState, at: Pos2) -> Pos2 {
    let Some(mut hold) = app.view3d.view.input.shift_hold else {
        return at;
    };
    let origin = Pos2::new(hold.origin.0 as f32, hold.origin.1 as f32);
    let near = origin.distance(at) < SHIFT_HOLD_POINTS as f32;
    match hold.constrain((at.x as f64, at.y as f64), near) {
        Some((x, y)) => {
            app.view3d.view.input.shift_hold = Some(hold);
            Pos2::new(x as f32, y as f32)
        }
        None => {
            app.view3d.view.input.shift_hold = None;
            at
        }
    }
}

fn add(app: &mut AppState, rect: Rect, at: Pos2, pressure: f32, pen: PenState) {
    // ポリゴン塗りつぶしのドラッグは、通った範囲を足す
    if app.region.drag.is_some() {
        crate::region::tools::drag_to(app, crate::region::tools::Where::Surface(rect), at);
        return;
    }
    // バケツの近い色の取り残しのドラッグは、通った面の UV の画素を足す
    if app.region.leftover_drag.is_some() {
        crate::region::bucket::drag_surface(app, rect, at);
        return;
    }
    // クイックマスクのストロークは、選択範囲の被覆へ
    if app.view3d.view.input.cover.is_some() {
        super::quick::add(app, rect, at, pressure);
        return;
    }
    // 離した後（残りを塗っている間）の点は受けない
    if app.view3d.view.input.surface.is_none() || app.view3d.view.input.released {
        return;
    }
    let raw = at;
    let at = shifted(app, at);
    // 定規のスナップ（ストロークの始めに凍結した寄せ先）
    let at = match app.view3d.view.input.ruler_constraint.as_mut() {
        Some(c) => to_pos(rect, c.project(screen_point(rect, at))),
        None => at,
    };
    // 面に当たった最後の点（確定したら、次の Shift の直線の始め）。ストロークの間はカメラもモデルも動かないので、塗りと同じ当たり
    let hit = app
        .view3d
        .model
        .as_ref()
        .and_then(|m| pick(&m.geometry, &camera_view(app, rect), local(rect, at)));
    let (Some(stroke), Some(surface)) = (
        app.session.stroke.as_mut(),
        app.view3d.view.input.surface.as_mut(),
    ) else {
        return;
    };
    match surface.add_input(
        &mut app.session.doc,
        stroke,
        pen.input(local(rect, at), pressure),
    ) {
        Ok(()) => {
            app.view3d.view.input.stroke_points += 1;
            if hit.is_some() {
                app.view3d.view.input.last_hit = hit;
            }
            if let Some(outcome) = surface.symmetry_note() {
                app.warn(Source::View3d, app.lang.mirror_note(outcome));
            }
            remember_point(app, at, raw, pressure, pen);
            crate::uv_wireframe::overlap::note_surface(app, rect, at);
        }
        Err(e) => abandon(app, &e),
    }
}

/// 描いているストロークの最後の点 `at`（Shift のぶれの抑え・向きの固定、定規を当てた後）を覚える。連続吹き付けは、点が来ないフレームに、
/// 当てる前の点 `raw`（2D と同じ。渡すときにまた当てる）と筆圧・傾き・回転を渡す（`crate::input::held`）。
fn remember_point(app: &mut AppState, at: Pos2, raw: Pos2, pressure: f32, pen: PenState) {
    let input = &mut app.view3d.view.input;
    input.last_point = Some(at);
    input.held = Some(crate::input::HeldPoint::new(
        raw,
        pressure,
        pen.tilt,
        pen.rotation,
        pen.time,
    ));
}

/// 連続吹き付けのブラシで描いている間: このフレームに新しい点が来なかったら、最後の点の位置・筆圧・傾き・回転で今の時刻の点を 1 つ渡し
/// （2D と同じ。`crate::input::held`）、次のフレームを頼む。離した後・ほかのブラシは何もしない。
fn continue_held(app: &mut AppState, ctx: &egui::Context, rect: Rect) {
    let input = &app.view3d.view.input;
    if input.surface.is_none() || input.released || app.session.stroke.is_none() {
        app.view3d.view.input.held = None;
        return;
    }
    let Some(held) = input.held else {
        return;
    };
    let continuous = app
        .session
        .stroke
        .as_ref()
        .and_then(|s| s.brush(&app.session.doc).ok())
        .is_some_and(|b| b.continuous_active());
    if !continuous {
        return;
    }
    ctx.request_repaint();
    let now = ctx.input(|i| i.time);
    if let Some(time) = held.time_at(now) {
        let pen = PenState {
            tilt: held.tilt,
            rotation: held.rotation,
            time,
        };
        add(app, rect, held.pos, held.pressure, pen);
    }
    if let Some(h) = app.view3d.view.input.held.as_mut() {
        h.received = Some(now);
    }
}

/// フレームごとのストロークの進め: 連続吹き付けの点を渡し（[`continue_held`]）、持ち越したダブを塗る（[`paint_queued`]）。
fn advance_stroke(app: &mut AppState, ctx: &egui::Context, rect: Rect) {
    continue_held(app, ctx, rect);
    paint_queued(app, ctx);
}

/// 持ち越したダブを、このフレームの分（時間の枠まで）塗る。まだ残れば次のフレームを頼む（動かさずに押しているだけでも塗り進める）。
/// 離したあとで塗り終えたら、文書のストロークを確定する。
fn paint_queued(app: &mut AppState, ctx: &egui::Context) {
    // egui が 1 フレームに 2 パス回すときの、捨てるほうのパスでは塗らない（最後のパスが塗る）
    if ctx.will_discard() {
        return;
    }
    let released = app.view3d.view.input.released;
    let fixed_budget = app.view3d.view.input.paint_budget;
    let mut clock = app.view3d.view.input.dab_clock;
    let (Some(stroke), Some(surface)) = (
        app.session.stroke.as_mut(),
        app.view3d.view.input.surface.as_mut(),
    ) else {
        return;
    };
    if surface.queued() == 0 {
        if released {
            finish(app, false);
        }
        return;
    }
    // 塗り＋表示の同期の枠のうち、塗りに使う時間
    let budget = fixed_budget.unwrap_or_else(|| {
        clock.paint_budget(pacing::frame_budget(
            clock.backlog(surface.queued()),
            released,
        ))
    });
    let started = Instant::now();
    match surface.paint_queued_until(&mut app.session.doc, stroke, started + budget) {
        Ok(painted) => {
            clock.record(painted, started.elapsed());
            app.view3d.view.input.dab_clock = clock;
            app.view3d.view.input.frame_dabs = painted;
            let more = surface.queued() > 0;
            let note = surface.symmetry_note();
            if more {
                ctx.request_repaint();
            }
            if let Some(outcome) = note {
                app.warn(Source::View3d, app.lang.mirror_note(outcome));
            }
            if released && !more {
                finish(app, false);
            }
        }
        Err(e) => abandon(app, &e),
    }
}

/// 塗れなかった（予算を超えた・ありえない長さの区間など）: 途中まで塗った画素も戻してストロークを取り消す。
fn abandon(app: &mut AppState, e: &yolu_core::geometry::SurfaceStrokeError) {
    if let Some(stroke) = app.session.stroke.take() {
        app.session.doc.cancel_stroke(stroke);
    }
    app.view3d.stroke_ended();
    app.fail(Source::View3d, app.lang.surface_error(e));
}

/// 離した（マウス・ペンを離した、ウィンドウのフォーカスを失った）: 面のストロークは、残りのダブを塗らずに並べて持ち越し、フレームごとに
/// 時間の枠で塗って、塗り終えたら確定する（`paint_queued`。それまでは描いている最中のまま、新しい点は受けない）。残りが無ければ、
/// 同じフレームのうちに確定する。面のストローク以外（範囲・クイックマスク）はその場で確定する。
pub fn release(app: &mut AppState, ctx: &egui::Context) {
    if app.view3d.view.input.stroke.is_none() || app.view3d.view.input.released {
        return;
    }
    let Some(surface) = app
        .view3d
        .view
        .input
        .surface
        .as_mut()
        .filter(|_| app.session.stroke.is_some())
    else {
        finish(app, false);
        return;
    };
    match surface.end_input() {
        Ok(()) => {
            app.view3d.view.input.released = true;
            ctx.request_repaint();
        }
        Err(e) => abandon(app, &e),
    }
}

/// 離した後で残りを塗っているストロークを、その場で（残りを全部塗って）確定する。新しい押し・閉じる・ビューが隠れるなど、残りを待てない所が
/// 呼ぶ。そうでなければ何もしない。
pub fn settle(app: &mut AppState) {
    if app.view3d.view.input.released {
        finish(app, false);
    }
}

/// 3D のストロークを、その場で終える（cancel なら捨てる）。持ち越したダブが残っていれば全部塗ってから確定する。
pub fn finish(app: &mut AppState, cancel: bool) {
    if app.view3d.view.input.stroke.is_none() {
        return;
    }
    if crate::region::tools::finish_drag(app, cancel) {
        app.view3d.stroke_ended();
        return;
    }
    if super::quick::finish(app, cancel) {
        app.view3d.stroke_ended();
        return;
    }
    let surface = app.view3d.view.input.surface.take();
    let Some(mut stroke) = app.session.stroke.take() else {
        app.session.doc.cancel_active_stroke();
        app.view3d.stroke_ended();
        return;
    };
    if cancel {
        app.session.doc.cancel_stroke(stroke);
        app.info(Source::View3d, app.lang.t("view3d-stroke-cancelled"));
    } else {
        let mut surface = surface;
        let last = surface
            .as_mut()
            .map(|s| s.finish(&mut app.session.doc, &mut stroke));
        match last {
            Some(Err(e)) => {
                app.session.doc.cancel_stroke(stroke);
                app.fail(Source::View3d, app.lang.surface_error(&e));
            }
            _ => {
                if let Some(s) = surface.as_ref() {
                    note_symmetry(app, s);
                    if s.stats.lost > 0 {
                        app.warn(Source::View3d, app.lang.smudge_lost());
                    }
                }
                // 揃えるクローンは、変わったストロークの先の基準を次のストロークへ渡す
                let destination = surface.as_ref().and_then(|s| s.clone_destination());
                match app.session.doc.end_stroke(stroke) {
                    Ok(result) => {
                        if let (true, Some(d)) = (result.changed, destination) {
                            app.clone.destination = Some(d);
                        }
                        remember_end(app);
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

/// 確定したストロークの終点（面に当たった最後の入力の面の点）を覚える（次の Shift の直線の始め）。一度も面に当たらなかったストロークの後は、
/// 前の終点が無いものとして扱う（古い終点から遠い線を引かない）。
fn remember_end(app: &mut AppState) {
    app.view3d.view.input.previous_end = app.view3d.view.input.last_hit;
}

/// 押しの組み合わせから、ビューを動かす操作（右ボタン・ペンのサイドボタンは回す（Alt を足すとスナップ回転）、中ボタンはパン、左は
/// Ctrl+Space で拡縮・Space でパン）。修飾は押しの始めに持っているもの。どれにも当たらなければ None（左は描く）。
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

/// ビューを動かす操作を始める（マウスもペンも）。動かせば、そのまま回す・パン・拡縮。
fn nav_press(app: &mut AppState, nav: Nav, button: PointerButton, pos: Pos2, m: &Modifiers) {
    app.view3d.view.input.navigation = Some(super::navigation::Drag::new(
        &app.view3d,
        app.prefs.settings.navigation,
        pos,
    ));
    app.view3d.view.input.nav = Some((nav, button));
    if nav == Nav::Zoom {
        app.view3d.view.input.zoom = Some(ZoomDrag::new(pos, m.alt));
    }
}

/// この押しを動かさずに離したときの操作（組み合わせの表の離しの行。クローンの元はクローンのブラシのとき、スポイトはポリゴン塗りつぶしの右ボタン
/// （アイランドのメニュー）でないとき）。ドラッグの操作の有無によらず引く。
fn click_of(
    app: &AppState,
    button: PointerButton,
    m: &Modifiers,
    space: bool,
) -> Option<crate::keymap::Operation> {
    use crate::keymap::Operation;
    match crate::keymap::click_gesture("view3d", button, m, space)? {
        Operation::CloneSource if crate::clone_source::active(app) => Some(Operation::CloneSource),
        Operation::Pick
            if !(button == PointerButton::Secondary && app.right_opens_island_menu()) =>
        {
            Some(Operation::Pick)
        }
        _ => None,
    }
}

/// 離しの操作を覚える（動かさずに離したら行う。動かしたら捨てる）。スポイトは押した所の見本を読んでおく。
fn note_click(
    app: &mut AppState,
    rect: Rect,
    source: StrokeSource,
    button: PointerButton,
    pos: Pos2,
    click: Option<crate::keymap::Operation>,
) {
    use crate::keymap::Operation;
    app.view3d.view.input.clone_press = None;
    app.view3d.view.input.eyedrop = None;
    match click {
        Some(Operation::CloneSource) => app.view3d.view.input.clone_press = Some((pos, button)),
        Some(Operation::Pick) => {
            let sample = crate::eyedrop::sample_surface(app, rect, pos);
            app.view3d.view.input.eyedrop = Some(crate::eyedrop::RightPress {
                source,
                at: pos,
                sample,
                button,
            });
        }
        _ => {}
    }
}

/// 記録に出す、今動かしているビューの操作の名前（無ければ "none"）。
fn nav_name(app: &AppState) -> String {
    app.view3d
        .view
        .input
        .nav
        .map_or_else(|| "none".to_string(), |(nav, _)| format!("{nav:?}"))
}

/// ポインタ・ペンが動いた（`previous` は前の位置）。
fn nav_move(app: &mut AppState, rect: Rect, pos: Pos2, previous: Pos2) {
    if app
        .view3d
        .view
        .input
        .clone_press
        .is_some_and(|(start, _)| start.distance(pos) > CLICK_DISTANCE)
    {
        app.view3d.view.input.clone_press = None; // 動かした: ドラッグの操作だけ
    }
    if app
        .view3d
        .view
        .input
        .eyedrop
        .is_some_and(|press| press.at.distance(pos) > CLICK_DISTANCE)
    {
        app.view3d.view.input.eyedrop = None; // 動かした: 回すだけ（スポイトにしない）
    }
    let Some((nav, _)) = app.view3d.view.input.nav else {
        return;
    };
    let d = match nav {
        // 回すドラッグは、押した所から `CLICK_DISTANCE` を超えるまで回さない（超えたら、押した所からの動きを全部当てる。パン・拡縮は今までどおり）
        Nav::Orbit | Nav::SnapOrbit => {
            let Some(drag) = app.view3d.view.input.navigation.as_mut() else {
                return;
            };
            match drag.rotation_delta(pos, previous, CLICK_DISTANCE) {
                Some(d) => d,
                None => return,
            }
        }
        Nav::Pan | Nav::Zoom => pos - previous,
    };
    match nav {
        Nav::Orbit | Nav::SnapOrbit | Nav::Pan => {
            super::navigation::move_by(app, rect, nav, d.x, d.y)
        }
        Nav::Zoom => {
            if let Some(mut zoom) = app.view3d.view.input.zoom {
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
                app.view3d.view.input.zoom = Some(zoom);
            }
        }
    }
}

/// ボタン（ペンの押し）を離した。`button` が動かしていた操作のものなら終える。動かさずに離した拡縮は寄る（Alt を押して押していたら引く）。
/// 左を動かさずに離したクローンの元の指定は、ここで決める。
fn nav_release(app: &mut AppState, rect: Rect, pos: Pos2, button: PointerButton) {
    if app.view3d.view.input.nav.is_some_and(|(_, b)| b == button) {
        if let Some(zoom) = app.view3d.view.input.zoom.take() {
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
        app.view3d.view.input.nav = None;
        app.view3d.view.input.navigation = None;
    }
    // 離しの操作は、押したボタンを離したときだけ（動かさずに離したら行う）
    if app
        .view3d
        .view
        .input
        .clone_press
        .is_some_and(|(_, b)| b == button)
    {
        if let Some((start, _)) = app.view3d.view.input.clone_press.take() {
            if start.distance(pos) <= CLICK_DISTANCE {
                set_clone_source(app, rect, start);
            }
        }
    }
    if app
        .view3d
        .view
        .input
        .eyedrop
        .is_some_and(|p| p.button == button)
    {
        if let Some(press) = app.view3d.view.input.eyedrop.take() {
            if press.at.distance(pos) <= CLICK_DISTANCE {
                crate::eyedrop::pick_surface(app, rect, pos);
            }
        }
    }
}

/// ビューを動かす操作の途中を全部やめる。
fn nav_cancel(app: &mut AppState) {
    app.view3d.view.input.navigation = None;
    app.view3d.view.input.nav = None;
    app.view3d.view.input.zoom = None;
    app.view3d.view.input.clone_press = None;
    app.view3d.view.input.eyedrop = None;
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
    s: &PointerSample,
    frame: &Frame,
    drag_at: &mut Option<Pos2>,
) {
    let StrokeSource::Pen(id) = s.source else {
        return;
    };
    let p = s.pos;
    let source = s.source;
    // ツールの帯のドラッグで始めた G/R/S（このペン）: 点を渡し、離したら G/R/S が決める
    if app
        .objects
        .transform
        .as_ref()
        .is_some_and(|t| t.drag == Some(crate::fillfx::gizmo::Source::Pen(id)))
    {
        if s.phase.contact() {
            app.objects.pen_at = Some(p);
        } else {
            app.objects.pen_at = Some(p);
            app.objects.pen_lifted = true;
            app.view3d.view.input.pen_press = None;
        }
        return;
    }
    if app
        .fillfx
        .drag
        .as_ref()
        .is_some_and(|d| d.source == crate::fillfx::gizmo::Source::Pen(id))
    {
        if s.phase.contact() {
            *drag_at = Some(p);
        } else {
            if let Some(at) = drag_at.take() {
                let m = &frame.modifiers;
                crate::fillfx::gizmo::drag_to(app, rect, at, m.shift, m.command);
            }
            crate::fillfx::gizmo::release(app, true);
            app.view3d.view.input.pen_press = None;
        }
        return;
    }
    if app
        .fillfx
        .point_drag
        .as_ref()
        .is_some_and(|d| d.in_3d && d.source == crate::fillfx::gizmo::Source::Pen(id))
    {
        if s.phase.contact() {
            *drag_at = Some(p);
        } else {
            if let Some(at) = drag_at.take() {
                crate::fillfx::points::drag_to(app, rect, at);
            }
            crate::fillfx::points::release(app, true);
            app.view3d.view.input.pen_press = None;
        }
        return;
    }
    let press = match s.phase {
        // 浮いているだけ・ほかのペン（別の ID）の押しが続いている間は、この点を使わない
        Phase::Idle { .. } => return,
        Phase::Move | Phase::Release { .. } => match app.view3d.view.input.pen_press {
            Some(press) => press,
            None => return,
        },
        Phase::Press => {
            // 離した後の残りを塗っている途中の押し: 残りを塗って確定してから、この押しを受ける
            if !frame.press_blocked && on_top(ui, rect, p) {
                settle(app);
            }
            let kind = press_kind(ui, app, rect, p, s, frame);
            crate::pen::log::event(|| {
                crate::pen::log::press_line(
                    crate::pen::log::Site {
                        view: "view3d",
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
            // ツールの押しが入った 3D ビューを覚える（「スナップする特殊定規の切り替え」が、ポインタがどちらにも乗っていないときに使う。2D と同じ）
            if kind == Owner::Tool && app.mode.paints() {
                app.rulers.last_drew = Some(crate::rulers::Place::View3d);
            }
            let button = s.button;
            app.view3d.view.input.pen_press = Some(Press {
                source,
                owner: kind,
                button,
                last: p,
            });
            match kind {
                // 3D のサイドボタンは右ボタンとして `View`（スポイトの印は `nav_press` が持つ）
                Owner::Ignored | Owner::Eyedrop => {}
                Owner::View => {
                    if let Some(nav) = nav_of(button, &frame.modifiers, frame.space) {
                        nav_press(app, nav, button, p, &frame.modifiers);
                    }
                    let click = click_of(app, button, &frame.modifiers, frame.space);
                    note_click(app, rect, source, button, p, click);
                    // サイドボタン（右ボタン）を動かさずに離したら、ポリゴン塗りつぶしのアイランドのメニュー（ほかのツールはスポイト。`nav_press`）
                    if button == PointerButton::Secondary && !frame.modifiers.any() && !frame.space
                    {
                        crate::bake::overlap::menu_press(
                            app,
                            crate::region::tools::Where::Surface(rect),
                            p,
                        );
                    }
                }
                Owner::Tool => {
                    // 選択の行の組み合わせ方は、押したボタンで引く（2D と同じ）
                    app.sel.press_button = button;
                    if app.mode == crate::mode::EditorMode::Edit {
                        // 編集のモード: 点の印で選ぶ・選んだ形の取っ手を掴む（描かない）。移動・回転・拡縮のツールは G/R/S のドラッグ
                        crate::objects::press(app, rect, p, crate::fillfx::gizmo::Source::Pen(id));
                        crate::objects::transform::start_drag(ui.ctx(), app);
                    } else if crate::fillfx::gizmo::press(
                        app,
                        rect,
                        p,
                        crate::fillfx::gizmo::Source::Pen(id),
                    ) || crate::fillfx::points::press(
                        app,
                        rect,
                        p,
                        crate::fillfx::gizmo::Source::Pen(id),
                    ) {
                        // 形のギズモのハンドルの上・点の編集: 描かずにドラッグを始める
                    } else if app.mode.paints() && app.tool.def().surface == Surface::Path {
                        // パスのツール: 押す・動く・離すを、点を足す・掴む・動かすにする
                        crate::pathtool::surface::pen_sample(app, rect, p, id, true, true);
                    } else {
                        begin(
                            app,
                            rect,
                            p,
                            s.pressure,
                            source,
                            s.eraser,
                            PenState::of(s),
                            frame.modifiers,
                        );
                    }
                }
            }
            return;
        }
    };
    if s.phase == Phase::Move {
        match press.owner {
            Owner::Ignored | Owner::Eyedrop => {}
            Owner::View => {
                crate::pen::log::event(|| {
                    crate::pen::log::move_line(
                        crate::pen::log::Site {
                            view: "view3d",
                            src: "pen",
                            frame: ui.ctx().cumulative_frame_nr(),
                            pen_frame: true,
                        },
                        p,
                        press.last,
                        &nav_name(app),
                        true,
                    )
                });
                nav_move(app, rect, p, press.last)
            }
            Owner::Tool => {
                if app.view3d.view.input.stroke == Some(source) {
                    add(app, rect, p, s.pressure, PenState::of(s));
                } else if app.path.pen_in(true) {
                    crate::pathtool::surface::pen_sample(app, rect, p, id, true, false);
                } else {
                    super::draft::moved(app, rect, p, source, &frame.modifiers);
                    super::select::moved(app, rect, p, source, &frame.modifiers);
                }
            }
        }
        app.view3d.view.input.pen_press = Some(Press { last: p, ..press });
    } else {
        // OS に押しを奪われて補った離し（本物の離しではない。`AppState::pen_release_lost`）は、マウスの取りこぼしと同じに扱う: ビューの操作の離しの操作
        // （クリックの拡縮・クローンの元・スポイト・メニュー）は出さず、途中をやめる。グラデーション・図形・定規は `draft::lost_release`、選択の形は何も選ばずにやめる。
        // ストロークと、ギズモ・パスの離しは、今までどおり最後の位置で終える
        let lost = s.phase == Phase::Release { lost: true };
        match press.owner {
            Owner::Ignored | Owner::Eyedrop => {}
            Owner::View if lost => nav_cancel(app),
            Owner::View => {
                // このペンが始めた物のボタン（ドラッグの操作か、離しの操作）
                let held = &app.view3d.view.input;
                let button = held
                    .nav
                    .map(|(_, b)| b)
                    .or(held.clone_press.map(|(_, b)| b))
                    .or(held.eyedrop.map(|e| e.button));
                if let Some(button) = button {
                    nav_release(app, rect, p, button);
                }
                crate::bake::overlap::menu_release(
                    app,
                    ui.ctx(),
                    crate::region::tools::Where::Surface(rect),
                    p,
                );
            }
            Owner::Tool => {
                if app.view3d.view.input.stroke == Some(source) {
                    release(app, ui.ctx());
                }
                if lost {
                    // 補った離しの位置は本物の離しの位置ではない: グラデーションは最後の位置で塗り、図形と定規はやめる（マウスの取りこぼしと同じ）
                    super::draft::lost_release(app, rect, source);
                    super::select::lost_release(app, source);
                } else {
                    super::draft::release(app, rect, p, source, &frame.modifiers);
                    super::select::release(app, rect, p, source, &frame.modifiers);
                }
                if app.path.pen_in(true) {
                    crate::pathtool::surface::pen_sample(app, rect, p, id, false, false);
                }
            }
        }
        app.view3d.view.input.pen_press = None;
    }
}

/// ペンが触れた最初の点の行き先（`input::owner`。マウスの押しと同じ表）。描くツールの Ctrl は描かない（Shift は直線）。
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
        frame.space,
        frame.press_blocked,
        on_top(ui, rect, p),
        holds_off,
    ))
}

/// 3D ビューの押しの事情（マウスもペンも）。ドラッグの操作（`nav_of`）か離しの操作（`click_of`）があればビューの押し。ツールのボタンは左ボタンと、
/// 選択のツールの選択の行を割り当てたボタン（2D の `nav::start_of` と同じ表）。ビューを動かしている・描いている・図形と選択の形を引いている間は、
/// 押しを受けない。Alt を持った押しは、ツールの押しにならない。編集のモードの物の選びとポーズのモードのギズモも左ボタンのツールの押しに当たるので、モードではツールの押しを断らない
/// （描くかは `begin` が決める）。
fn press_ctx(
    app: &AppState,
    button: PointerButton,
    m: &Modifiers,
    space: bool,
    blocked: bool,
    on_top: bool,
    holds_off: bool,
) -> PressCtx {
    let select_row = app.mode.paints()
        && app.tool.is_select()
        && crate::selection::combine_for(button, m).is_some();
    PressCtx {
        blocked,
        on_top,
        stencil: app.stencil.handling(),
        busy: app.view3d.view.input.nav.is_some()
            || app.view3d.view.input.stroke.is_some()
            || app.view3d.view.input.draft.is_some()
            || super::select::dragging(app),
        view_op: nav_of(button, m, space).is_some(),
        click_op: click_of(app, button, m, space).is_some(),
        eyedrop: false,
        // Alt を持って押した接触は、視点の操作の組み合わせ（Alt + 右）か離しの操作（Alt + 左のクローンの元）でなければ、描き始めない
        tool: (button == PointerButton::Primary || select_row) && !m.alt,
        paints: true,
        holds_off,
    }
}

/// マウスのツールの押しのボタン（選択の行を左ボタン以外にしていれば、そのボタン。ツールの押しが無ければ左ボタン）。
fn tool_button(app: &AppState) -> PointerButton {
    app.view3d
        .view
        .input
        .mouse_press
        .filter(|p| p.owner == Owner::Tool)
        .map_or(PointerButton::Primary, |p| p.button)
}

/// Esc: 離したあと残りを塗っている間は確定し、パスの点のドラッグ・ストローク・グラデーション／図形／定規のドラッグ・選びの途中・ギズモのドラッグを
/// 捨てる（ギズモは始まりの位置へ戻す。それまでの位置は当てない）。
fn escape(app: &mut AppState, ctx: &egui::Context) {
    // 離したあと残りを塗っている間（確定待ち）の Esc は、線を捨てない: 確定してから、Esc の普段の意味へ進む
    settle(app);
    // パスの点のドラッグを捨てる（無ければ選んだ点を外す）
    let path_esc = app.path_cancel(ctx.cumulative_pass_nr());
    if app.view3d.view.input.stroke.is_some() && !path_esc {
        finish(app, true);
        // この Esc はストロークを捨てるのに使った（先に描く 3D ビューのあとで、キャンバスが同じ Esc で、前からある選択範囲まで解除しない）
        crate::ui::window::note_escape_taken(ctx);
    }
    // グラデーション・図形・定規のドラッグは何も描かずに捨てる。選択の形と多角形の点も、何も選ばずに捨てる
    super::draft::cancel(app);
    super::select::cancel(app, ctx);
    super::hide::input::cancel(app);
    // ギズモのドラッグは始まりのポーズへ戻す（それまでの位置は当てない）。形のギズモもドラッグの前へ戻す
    gizmo::release(app, false);
    crate::fillfx::gizmo::release(app, false);
    crate::fillfx::points::release(app, false);
    nav_cancel(app);
}

/// 入力を当てる（rect はタブの中身の表示域。`foreign` はこの押しが egui でほかの部品のものか）。
pub fn handle(ui: &mut Ui, app: &mut AppState, rect: Rect, pen: &[PenSample], foreign: bool) {
    let ctx = ui.ctx().clone();
    // ストロークの札をほか（キャンバスの Esc・フォーカスを失ったとき）が手放したら、こちらも終える。選択ペンとクイックマスクの
    // ストロークは `app.stroke` でなく `app.sel.pen` が持つので、3D の被覆（`input.cover`）が残り、選択ペンのストロークが生きている
    // 間は、札が無くても続ける（被覆と選択ペンのどちらかが先に無くなれば、ここで終える）
    let quick_live = app.view3d.view.input.cover.is_some() && app.sel.pen.is_some();
    if app.view3d.view.input.stroke.is_some()
        && app.session.stroke.is_none()
        && app.region.drag.is_none()
        && app.region.leftover_drag.is_none()
        && !quick_live
    {
        app.view3d.stroke_ended();
    }
    if app.view3d.model.is_none() {
        nav_cancel(app);
        app.view3d.view.input.pen_press = None;
        app.view3d.view.input.mouse_press = None;
        app.view3d.view.input.draft = None;
        super::select::focus_lost(app);
        // Touch の終わり・取りやめとフォーカスを失ったのは、モデルが無くても読む（古い力を次のマウスの押しに持ち越さない）
        let ended = ui.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    Event::Touch {
                        phase: egui::TouchPhase::End | egui::TouchPhase::Cancel,
                        ..
                    } | Event::WindowFocused(false)
                )
            })
        });
        if ended {
            app.forget_touch();
        }
        return;
    }
    // 保存していない変更の問いが開いている間も止める（別ウィンドウの 3D ビューは問いの覆いの外で、入力を直に読むため）
    let blocked = crate::windows::pointer_blocked(app);
    app.region.modifiers = ui.input(|i| i.modifiers.into());
    // 設定のパネルを開いているあいだの押しは、パネルの外でも 3D に使わない（パネルは外の押しで閉じる。その押しが描き始め・回し始めに
    // ならないように。このフレームの押しで閉じるときも、パネルはこの後に描くので開いている）。ホイールは使える。ドックのタブの見出しを
    // つかんでいる間・離した直後と、押しがほかの部品のものであるときも、描き始めも回し始めもしない
    let press_blocked =
        blocked || app.view3d.view.display.settings_open || app.dock_grabbed() || foreign;
    super::navigation::shortcut(ui, app, rect, foreign);
    // 右ボタンを押している間の W/A/S/D/Q/E は、視点の移動
    super::navigation::fly(ui, app);
    let (events, now, frame_dt) = ui.input(|i| (i.events.clone(), i.time, i.unstable_dt as f64));
    super::select::frame(app, now);
    // ポーズのモードでは描かない（左ボタンはギズモとボーンを選ぶ。ペンの点は描くのに使わない）。ハイドのモードも描かない（左ボタンは
    // ボーンの丸と UV アイランドを選ぶ。ペンは egui のポインタとして同じ道を通る）
    let pose_mode = app.mode == crate::mode::EditorMode::Pose;
    let hide_mode = app.mode == crate::mode::EditorMode::Hide;
    // ポーズ・ハイドのモードの間はペンの点を見ないので、押している印も持ち越さない（離したのを見落とした印が次の押しを止めない）
    if pose_mode || hide_mode {
        app.view3d.view.input.pen_press = None;
    }
    let pen: &[PenSample] = if pose_mode || hide_mode { &[] } else { pen };
    let (snap, shift, modifiers) =
        ui.input(|i| (i.modifiers.command, i.modifiers.shift, i.modifiers.into()));
    // 図形のドラッグの Shift・Alt は、動かさなくても形に効く
    super::draft::modifiers(app, &modifiers);
    super::select::modifiers(app, &modifiers);
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

    // ペン（Windows Ink）の点が先、egui のイベントが後。ペンの押し（触れてから離すまで）は、ペンの点だけで扱い、同じ押しが egui のポインタの押しとして
    // 二重に来ても使わない（`pen_frame`）。マウスの点の時刻（筆の速さ）は、2D のキャンバスと同じく、前のフレームからの時間をこのフレームのマウスの点の数で
    // 等分する（`Stream`）
    let stream = Stream::new(
        pen,
        &events,
        app.view3d.view.input.pen_press.is_some(),
        now,
        frame_dt,
    );
    let pen_frame = stream.pen_frame;
    let ppp = ui.ctx().pixels_per_point();
    if crate::pen::log::enabled() && (pen_frame || app.view3d.view.input.nav.is_some()) {
        crate::pen::log::event(|| {
            crate::pen::log::frame_line(
                "view3d",
                ui.ctx().cumulative_frame_nr(),
                pen_frame,
                app.view3d.view.input.pen_press.is_some(),
                pen.len(),
                &nav_name(app),
            )
        });
    }
    let frame = Frame {
        press_blocked,
        modifiers,
        space,
    };
    for item in stream {
        let (event, time) = match item {
            Item::Pen(s) => {
                let lost = app.pen_release_lost(s);
                let s =
                    PointerSample::pen(s, ppp, app.view3d.view.input.pen_press, lost, modifiers);
                pen_sample(ui, app, rect, &s, &frame, &mut drag_at);
                continue;
            }
            Item::Event { event, time } => (event, time),
        };
        // Y を押しているあいだのドラッグはステンシルの置き場を動かす（描かない・回さない・パンしない。編集・ポーズのモードでは描かないので、ステンシルも使わない）
        let over = match event {
            Event::PointerButton { pos, .. } => on_top(ui, rect, *pos),
            _ => false,
        };
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
                    let (pos, button, m) = (s.pos, &s.button, &s.modifiers);
                    if s.phase == Phase::Press {
                        if press_blocked || !on_top(ui, rect, pos) {
                            continue;
                        }
                        // 離した後の残りを塗っている途中の押し: 残りを塗って確定してから、この押しを受ける（押しを捨てない）
                        settle(app);
                        let c = press_ctx(app, *button, m, space, press_blocked, true, false);
                        if pen_frame {
                            // ペンの押しは、ペンの点が持つ（これは同じ押しの egui のポインタの代わりの入力）
                            crate::pen::log::event(|| {
                                crate::pen::log::press_line(
                                    crate::pen::log::Site {
                                        view: "view3d",
                                        src: "mouse",
                                        frame: ui.ctx().cumulative_frame_nr(),
                                        pen_frame: true,
                                    },
                                    "none",
                                    pos,
                                    &format!("{button:?}"),
                                    true,
                                )
                            });
                            continue;
                        }
                        // 押した瞬間に、実際のボタン・修飾・押しながらのキーで、ドラッグの操作と離しの操作を別々に引き、押しの持ち主を決める
                        // （`press_ctx`。ペンと同じ表）。ドラッグの操作があれば始め、離しの操作は覚えて、動かさずに離したら行う（動いたら捨てる）
                        let nav = nav_of(*button, m, space);
                        let click = click_of(app, *button, m, space);
                        crate::pen::log::event(|| {
                            crate::pen::log::press_line(
                                crate::pen::log::Site {
                                    view: "view3d",
                                    src: "mouse",
                                    frame: ui.ctx().cumulative_frame_nr(),
                                    pen_frame: false,
                                },
                                &crate::pen::log::mouse_kind("nav", nav, click.is_some()),
                                pos,
                                &format!("{button:?}"),
                                false,
                            )
                        });
                        let owner = crate::input::owner(&c);
                        if owner == Owner::View {
                            if let Some(nav) = nav {
                                nav_press(app, nav, *button, pos, m);
                            }
                            note_click(app, rect, StrokeSource::Mouse, *button, pos, click);
                        }
                        // 右ボタンを動かさずに離したら、ポリゴン塗りつぶしのアイランドのメニュー（ほかのツールはスポイト。動かせば回すだけ）
                        if !c.busy && *button == PointerButton::Secondary && !m.any() && !space {
                            crate::bake::overlap::menu_press(
                                app,
                                crate::region::tools::Where::Surface(rect),
                                pos,
                            );
                        }
                        if owner == Owner::Tool && app.mode.paints() {
                            app.rulers.last_drew = Some(crate::rulers::Place::View3d);
                        }
                        if owner == Owner::Tool {
                            app.view3d.view.input.mouse_press = Some(Press {
                                source: StrokeSource::Mouse,
                                owner,
                                button: *button,
                                last: pos,
                            });
                            // 選択の行の組み合わせ方は、押したボタンで引く（2D と同じ）
                            app.sel.press_button = *button;
                            if hide_mode {
                                // ハイドのモード: ボーンの丸・UV アイランドを選ぶ（四角で選ぶツールは離すまで引く）
                                super::hide::input::press(app, rect, pos, m);
                            } else if pose_mode {
                                if app.view3d.pose.drag.is_none() {
                                    gizmo::press(app, rect, pos);
                                    // 移動・回転・拡縮のツール: 輪の外で、選んだボーンの面から左ドラッグを始めたら、その種類の G/R/S
                                    if let Some(kind) = app.edit_tool.transform() {
                                        let selected = app
                                            .view3d
                                            .pose
                                            .session
                                            .as_ref()
                                            .and_then(|s| s.selected);
                                        if app.view3d.pose.drag.is_none()
                                            && selected.is_some()
                                            && gizmo::bone_at(app, rect, pos) == selected
                                        {
                                            app.objects.drag_request = Some((
                                                kind,
                                                pos,
                                                crate::fillfx::gizmo::Source::Mouse,
                                            ));
                                            crate::objects::transform::start_drag(ui.ctx(), app);
                                        }
                                    }
                                }
                            } else if app.mode == crate::mode::EditorMode::Edit {
                                // 編集のモード: 点の印で選ぶ・選んだ形の取っ手を掴む（描かない）。移動・回転・拡縮のツールは G/R/S のドラッグ
                                crate::objects::press(
                                    app,
                                    rect,
                                    pos,
                                    crate::fillfx::gizmo::Source::Mouse,
                                );
                                crate::objects::transform::start_drag(ui.ctx(), app);
                            } else {
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
                                    begin(
                                        app,
                                        rect,
                                        pos,
                                        s.pressure,
                                        StrokeSource::Mouse,
                                        false,
                                        PenState::of(&s),
                                        *m,
                                    );
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
                        // ツールの押しのボタン（選択の行を左ボタン以外にしていれば、そのボタン）を離したら、ツールが終える
                        if *button == tool_button(app) {
                            // ギズモのドラッグは、1 フレームの最後の位置を当ててから終える
                            flush(app, &mut drag_at);
                            // ハイドのモードの四角の選びを終える
                            super::hide::input::release(app, rect, pos);
                            for held in Held::ALL {
                                if held.held(app, StrokeSource::Mouse) {
                                    held.release(app, rect, pos, StrokeSource::Mouse, m, &ctx);
                                }
                            }
                            app.view3d.view.input.mouse_press = None;
                        }
                    }
                    app.view3d.view.input.last_pointer = Some(pos);
                }
                Phase::Move => {
                    let pos = s.pos;
                    let previous = app.view3d.view.input.last_pointer.unwrap_or(pos);
                    if app.view3d.view.input.stroke == Some(StrokeSource::Mouse) && !pen_frame {
                        add(app, rect, pos, s.pressure, PenState::of(&s));
                    }
                    if !pen_frame {
                        super::hide::input::moved(app, pos);
                        crate::pathtool::surface::moved(app, rect, pos, StrokeSource::Mouse);
                        super::draft::moved(app, rect, pos, StrokeSource::Mouse, &s.modifiers);
                        super::select::moved(app, rect, pos, StrokeSource::Mouse, &s.modifiers);
                    }
                    if app.view3d.pose.drag.is_some()
                        || app
                            .fillfx
                            .drag
                            .as_ref()
                            .is_some_and(|d| d.source == crate::fillfx::gizmo::Source::Mouse)
                        || app.fillfx.point_drag.as_ref().is_some_and(|d| {
                            d.in_3d && d.source == crate::fillfx::gizmo::Source::Mouse
                        })
                    {
                        drag_at = Some(pos);
                    }
                    // ペンの押しの回す・パン・拡縮は、ペンの点が動かす
                    if crate::pen::log::enabled() && app.view3d.view.input.nav.is_some() {
                        crate::pen::log::event(|| {
                            crate::pen::log::move_line(
                                crate::pen::log::Site {
                                    view: "view3d",
                                    src: "mouse",
                                    frame: ui.ctx().cumulative_frame_nr(),
                                    pen_frame,
                                },
                                pos,
                                previous,
                                &nav_name(app),
                                !pen_frame,
                            )
                        });
                    }
                    if !pen_frame {
                        nav_move(app, rect, pos, previous);
                    }
                    app.view3d.view.input.last_pointer = Some(pos);
                }
                Phase::Idle { .. } => {}
            }
            continue;
        }
        match event {
            // 指・ペンの Touch の力は、2D のキャンバスと同じ全体の調整を通して、マウスの道の筆圧にする
            Event::Touch { force, phase, .. } => app.note_touch(*force, *phase),
            Event::MouseWheel { unit, delta, .. } => {
                let Some(p) = ui.input(|i| i.pointer.hover_pos()) else {
                    continue;
                };
                // 離した後の残りを塗っている途中（確定待ち）のホイールは、確定してから受ける
                if !blocked && on_top(ui, rect, p) {
                    settle(app);
                }
                if blocked
                    || !on_top(ui, rect, p)
                    || app.view3d.view.input.stroke.is_some()
                    || app.view3d.view.input.draft.is_some()
                    || super::select::dragging(app)
                {
                    continue;
                }
                let notches = match unit {
                    egui::MouseWheelUnit::Point => delta.y / 40.0,
                    egui::MouseWheelUnit::Line => delta.y,
                    egui::MouseWheelUnit::Page => delta.y * 3.0,
                };
                super::navigation::wheel(app, rect, p, notches);
            }
            // メニュー・パイが開いている間の Esc は、それを閉じる（下の 3D の操作はやめない）
            Event::Key {
                key: Key::Escape,
                pressed: true,
                ..
            } if !blocked => {
                // 形のギズモ・点のドラッグ用に溜めた位置は当てずに捨てる
                drag_at = None;
                escape(app, &ctx);
            }
            // 多角形選択の Enter（閉じる）・Backspace（最後の点を消す）。文字を打っているときは使わない
            Event::Key {
                key: key @ (Key::Enter | Key::Backspace),
                pressed: true,
                modifiers: key_modifiers,
                ..
            } if !blocked && !ctx.egui_wants_keyboard_input() => {
                super::select::key(app, rect, (*key).into(), (*key_modifiers).into());
            }
            Event::WindowFocused(false) => {
                // フォーカスを失ったら、そこまでを確定する（離したのを受け取れないので。離したのと同じく、残りは時間の枠で塗ってから）
                release(app, &ctx);
                // グラデーション・図形・定規のドラッグは、離したのを受け取れないので何も描かずに捨てる。選択の形と多角形の点も同じ
                app.view3d.view.input.draft = None;
                super::select::focus_lost(app);
                super::hide::input::cancel(app);
                app.path_finish_drag();
                // 点を矩形で選ぶドラッグは、離したのを受け取れないので捨てる（古い始点が次の離しで効かないように）
                if app.path.rect.is_some_and(|r| r.surface) {
                    app.path.rect = None;
                }
                app.view3d.view.input.pen_press = None;
                app.view3d.view.input.mouse_press = None;
                flush(app, &mut drag_at);
                gizmo::release(app, true);
                // 形のギズモ・点のドラッグは離したのを受け取れないので、ドラッグの前に戻す（履歴にも残さない）
                drag_at = None;
                crate::fillfx::gizmo::release(app, false);
                crate::fillfx::points::release(app, false);
                nav_cancel(app);
                app.forget_touch();
            }
            _ => {}
        }
    }
    flush(app, &mut drag_at);
    // ボタンを離したのを取りこぼしたとき（ウィンドウの外で離したなど）も、押していなければ離したことにする
    // ツールの押しのボタン（選択の行を左ボタン以外にしていれば、そのボタン）
    let tool_button = tool_button(app);
    let (tool_down, any_down) = ui.input(|i| {
        (
            i.pointer.button_down(tool_button.into()),
            i.pointer.any_down(),
        )
    });
    let missed = !tool_down
        && !events
            .iter()
            .any(|e| matches!(e, Event::PointerButton { pressed: true, .. }));
    if missed {
        for held in Held::ALL {
            if held.held(app, StrokeSource::Mouse) {
                held.lost_release(app, rect, StrokeSource::Mouse, &ctx);
            }
        }
        // ツールの押しのボタンを離したのを取りこぼしたら、押しの控えも捨てる（次の押しのボタンを引き違えない）
        app.view3d.view.input.mouse_press = None;
        // 離したのを取りこぼした四角の選びは、何も選ばずに捨てる
        super::hide::input::cancel(app);
    }
    // 取りこぼしたストロークも、このフレームのうちに塗り始める
    advance_stroke(app, &ctx, rect);
    // ペンが回している・パンしている・寄っている間は、egui のポインタが押していなくても続ける（ペンが離したときに終える）
    let pen_press = app.view3d.view.input.pen_press;
    let pen_driven = pen_press.is_some_and(|p| p.owner == Owner::View);
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
fn seen_from_camera(geometry: &SurfaceGeometry, view: &CameraView, point: &SurfaceHit) -> bool {
    let sight = view.sight(point.position);
    let (to, distance) = (sight.to, sight.distance);
    if distance <= 0.0 || point.normal.dot(-to) <= 0.0 {
        return false;
    }
    let epsilon = (geometry.bounds().size().length() * 1e-5).max(1e-7);
    match geometry.raycast(
        Ray::new(sight.origin, to / distance),
        false,
        distance + epsilon,
    ) {
        None => true,
        Some(first) => first.triangle == point.triangle || first.distance >= distance - epsilon,
    }
}

/// 今効く 3D の対称（描いているあいだはそのストロークに固めたもの）。
fn active_symmetry(app: &AppState) -> Option<SurfaceSymmetrySetup> {
    if app.view3d.view.input.stroke.is_some() {
        app.view3d.view.input.symmetry
    } else {
        app.rulers_active(crate::rulers::Place::View3d)
            .surface_symmetry
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
    // 画面上のサイズで指定のブラシは、設定の大きさが画面の点（ポインタの下の面で、文書の画素へ直す）
    let radius_px = if app.m2.brush.controls.size_by_screen {
        app.brush.radius as f64 / points_per_pixel_at(app, model, &view, hit.position)
    } else {
        app.brush.radius as f64
    };
    let radius = world_radius(&model.geometry, radius_px, app.session.doc.width());
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
    // 写しのカーソル（描いている最中は、3D のストローク以外では出さない。クイックマスクは対称を使わない）
    if app.view3d.material != hit.material
        || app.sel.quick
        || app.tool.def().surface == Surface::Cover
    {
        return true;
    }
    let copies = match active_symmetry(app) {
        Some(sym) => copy_hits(
            &model.geometry,
            &hit,
            sym.mirror.as_ref(),
            sym.radial.as_ref(),
            radius,
        ),
        None => Vec::new(),
    };
    for copy in &copies {
        let alpha = if seen_from_camera(&model.geometry, &view, copy) {
            242
        } else {
            100
        };
        if let Some(points) = ring_points(&view, rect, copy.position, copy.normal, radius) {
            draw_ring(&painter, points, symmetry_color(alpha), alpha / 2);
        }
    }
    canvas_copy_rings(app, &painter, &view, rect, &hit, &copies, radius);
    true
}

/// 2D の対称の写しのカーソル: 元と 3D の写しの面の点の UV を、キャンバスの上で 2D の対称で写し、写した UV の下の面（今のテクスチャ
/// セットの UV の格子で引く。作ってあるときだけ）に水色の円を出す（カメラから見えない所は薄く）。円の大きさは、写した元と写し先の
/// テクセルの細かさの比で直す（2D の写しは UV の上で同じ大きさなので、面の上では写し先のテクセルの大きさに合う）。
fn canvas_copy_rings(
    app: &AppState,
    painter: &egui::Painter,
    view: &CameraView,
    rect: Rect,
    hit: &SurfaceHit,
    copies: &[SurfaceHit],
    radius: f32,
) {
    let canvas = match app.view3d.view.input.stroke {
        Some(_) if app.view3d.view.input.surface.is_none() => return,
        _ => app.canvas_symmetry(),
    };
    let (Some(grid), Ok(transforms)) = (app.cached_region_grid(), canvas.transforms()) else {
        return;
    };
    if !canvas.enabled() {
        return;
    }
    let geometry = grid.geometry();
    let size = DVec2::new(
        app.session.doc.width() as f64,
        app.session.doc.height() as f64,
    );
    let (doc_w, doc_h) = (
        app.session.doc.width() as i32,
        app.session.doc.height() as i32,
    );
    let reach = app.brush.radius as f64;
    for h in std::iter::once(hit).chain(copies) {
        let (x, y) = (h.uv.x as f64 * size.x, h.uv.y as f64 * size.y);
        let from = geometry
            .triangles()
            .get(h.triangle as usize)
            .map_or(0.0, |t| t.texel_size(doc_w, doc_h));
        for t in transforms.iter().skip(1) {
            let (qx, qy) = t.map(x, y);
            let Some((index, w)) = grid.locate(DVec2::new(qx, qy), size, reach) else {
                continue;
            };
            let tri = &geometry.triangles()[index as usize];
            let to = tri.texel_size(doc_w, doc_h);
            let ring = if from > 0.0 && to > 0.0 {
                radius * to / from
            } else {
                radius
            };
            let copy = SurfaceHit {
                position: tri.a * w.x + tri.b * w.y + tri.c * w.z,
                normal: tri.normal(),
                triangle: index,
                ..*h
            };
            let alpha = if seen_from_camera(geometry, view, &copy) {
                242
            } else {
                100
            };
            if let Some(points) = ring_points(view, rect, copy.position, copy.normal, ring) {
                draw_ring(painter, points, symmetry_color(alpha), alpha / 2);
            }
        }
    }
}

/// 3D ビューの上に、3D の定規（対称の面と軸を含む）とクローンの元の印を描く（絵の上、カーソルの下）。
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
    // 3D の定規（対称の面と軸を含む）
    crate::rulers::draw3d::paint_shown(&painter, app, rect);
    // クローンの元（十字）
    if crate::clone_source::active(app) {
        if let Some(source) = app.clone.source_for(&model.geometry) {
            if let Some(p) = to_screen(source.position) {
                crate::clone_source::paint_mark(&painter, p);
            }
        }
    }
}

/// 機能の表の行（3D ビューの面のストローク・図形のドラッグの最中か。グラデーションのドラッグは描いている最中に数えない。
/// 文書を替えたら、前の文書の面で覚えた点を捨てる）。
pub(crate) const FEATURE: Feature = Feature {
    stroking: Some(|app| {
        app.view3d.view.input.cover.is_some()
            || app
                .view3d
                .view
                .input
                .draft
                .is_some_and(|d| d.kind != crate::view3d::draft::DraftKind::Gradient)
    }),
    document_replaced: Some(|app| app.view3d.view.input.previous_end = None),
    ..Feature::new("view3d.input")
};
