//! 3D ビューのグラデーション・図形・定規: 画面の上でドラッグして形を決め、離したときに、カメラから見えている面へ写して描く（core の
//! `cover_screen`。面の向きの弱めと継ぎ目のにじみは 3D のストロークと同じ投影の塗りの切り替え、隠れた所と裏の面は塗らない）。
//! - グラデーション: 見えているテクセルごとに、そのテクセルが写る画面の点で色を決める（2D と同じ式・設定）。
//! - 図形の「塗る」: 画面の形の覆い（縁は 2D の図形の塗りと同じく、1 テクセルを 4 × 4 の点で見た量）を量にして、2D の図形の塗りと
//!   同じ塗り方で塗る。「線で描く」: 輪郭の画面の点の並びを、3D のストローク（`SurfaceStroke`）の入力にする（ブラシの設定・対称が効く。
//!   手ぶれ補正と曲線は切る。筆圧は一定）。
//! - 定規: モデルの面を押して引き、離したときに、選んでいるレイヤーに 3D の定規（モデルの空間。文書の値）を作る（`rulers::edit3d`）。押した点が
//!   モデルの外なら作らない。3D のブラシのストロークの入力の点を寄せるのは `input`（`rulers::edit3d` の寄せ先）。
//!
//! どれも離すまで文書を変えない。Esc・フォーカスを失う・ツールの切り替え・ビューが隠れるで、途中の形を何も描かずに捨てる（離したのを取りこぼした
//! グラデーションは、2D と同じく最後の位置で塗る。ペンの押しを OS に奪われて補った離しも、取りこぼしと同じ）。

use egui::{Modifiers, Painter, Pos2, Rect};
use yolu_core::geometry::{
    cover_screen, pick, ProjectionSettings, ScreenCoverSettings, ScreenCoverage, ScreenShape,
    SurfaceHit, SurfaceInput,
};
use yolu_core::glam::{DVec2, Vec2};

use super::input::{
    camera_view, local, open_stroke, refuse_other_set, screen_point, to_pos, Opened,
};
use crate::drafting::canvas::{fill_region, finish_paint, paint_outline};
use crate::drafting::{endpoints, outline, Figure};
use crate::notice::Source;
use crate::state::{AppState, StrokeSource, Tool};

/// 3D ビューでグラデーション・図形・定規のドラッグの途中か（どれでも true。離す・Esc・フォーカスを失うまで）。
pub fn dragging(app: &AppState) -> bool {
    app.view3d.input.draft.is_some()
}

/// ドラッグで決めるもの。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftKind {
    Gradient,
    Figure,
    Ruler,
}

/// 3D ビューのドラッグの途中（点は表示域の画面の点。左上が原点）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceDraft {
    pub kind: DraftKind,
    pub source: StrokeSource,
    pub start: DVec2,
    pub current: DVec2,
    pub shift: bool,
    pub alt: bool,
    /// 定規を作るときの、押した面の点（モデルの外で押したときは、ドラッグを始めない）。
    pub hit: Option<SurfaceHit>,
}

/// 図形の輪郭を 3D のストロークに流すとき、入力の点の間をこの長さ（画面の点）以下に分ける（ダブの間隔は区間の始まりの面の奥行きで
/// 決まるので、奥行きの変わる長い辺でも間隔が合う）。
const OUTLINE_STEP: f64 = 16.0;

/// 押した（ツールの押し。グラデーション・図形・定規のとき）。
pub(super) fn press(app: &mut AppState, rect: Rect, at: Pos2, source: StrokeSource, shift: bool) {
    let kind = match app.tool {
        Tool::Gradient => DraftKind::Gradient,
        Tool::Shape => DraftKind::Figure,
        Tool::Ruler => DraftKind::Ruler,
        _ => return,
    };
    if app.is_stroking()
        || app.view3d.input.draft.is_some()
        || app.gradient.drag.is_some()
        || app.view3d.model.is_none()
    {
        return;
    }
    match kind {
        DraftKind::Gradient => {
            if let Some(reason) = app.read_only_reason().map(str::to_owned) {
                app.refuse(
                    Source::Gradient,
                    crate::lang::refusals::read_only_set(app.lang, &reason),
                );
                return;
            }
        }
        DraftKind::Figure => {
            if let Err(reason) = crate::region::tools::paint_gate(app) {
                app.refuse(Source::Ruler, reason);
                return;
            }
        }
        DraftKind::Ruler => {}
    }
    // 定規は、モデルの面を押して引く（モデルの外で押したら作らない）
    let hit = if kind == DraftKind::Ruler {
        let Some(model) = app.view3d.model.clone() else {
            return;
        };
        let Some(hit) = pick(&model.geometry, &camera_view(app, rect), local(rect, at)) else {
            return;
        };
        // ブラシと同じく、ほかのテクスチャセットの面からは始めない
        if refuse_other_set(app, &model, &hit) {
            return;
        }
        Some(hit)
    } else {
        None
    };
    let p = screen_point(rect, at);
    app.view3d.input.draft = Some(SurfaceDraft {
        kind,
        source,
        start: p,
        current: p,
        shift,
        alt: false,
        hit,
    });
}

/// ポインタ・ペンが動いた（この入力で始めたドラッグだけ）。
pub(super) fn moved(app: &mut AppState, rect: Rect, at: Pos2, source: StrokeSource, m: &Modifiers) {
    if let Some(d) = app
        .view3d
        .input
        .draft
        .as_mut()
        .filter(|d| d.source == source)
    {
        d.current = screen_point(rect, at);
        d.shift = m.shift;
        d.alt = m.alt;
    }
}

/// 毎フレーム: 押しているあいだの Shift・Alt は、動かさなくても形に効く（2D と同じ）。
pub(super) fn modifiers(app: &mut AppState, m: &Modifiers) {
    if let Some(d) = app.view3d.input.draft.as_mut() {
        d.shift = m.shift;
        d.alt = m.alt;
    }
}

/// 離した: 決めた形を描く（この入力で始めたドラッグだけ）。
pub(super) fn release(
    app: &mut AppState,
    rect: Rect,
    at: Pos2,
    source: StrokeSource,
    m: &Modifiers,
) {
    if app.view3d.input.draft.is_none_or(|d| d.source != source) {
        return;
    }
    moved(app, rect, at, source, m);
    if let Some(d) = app.view3d.input.draft.take() {
        apply(app, rect, d);
    }
}

/// 離したのを取りこぼした（ウィンドウの外で離した・ペンの押しを OS に奪われて補った離し（`AppState::pen_release_lost`）など）: グラデーションは最後の位置で
/// 離したことにし、図形と定規は何も変えずにやめる（2D と同じ。2D のグラデーションの補った離しも最後の位置で塗る）。この入力（`source`）で始めたドラッグだけ。
pub(super) fn lost_release(app: &mut AppState, rect: Rect, source: StrokeSource) {
    if app.view3d.input.draft.is_none_or(|d| d.source != source) {
        return;
    }
    if let Some(d) = app.view3d.input.draft.take() {
        if d.kind == DraftKind::Gradient {
            apply(app, rect, d);
        }
    }
}

/// Esc: 途中の形を捨てる（グラデーションは知らせる。2D と同じ）。何かあったか。
pub(super) fn cancel(app: &mut AppState) -> bool {
    match app.view3d.input.draft.map(|d| d.kind) {
        None => false,
        Some(DraftKind::Gradient) => crate::gradient::canvas::cancel(app),
        Some(_) => app.drafting_cancel(),
    }
}

/// ドラッグで決めた形を描く。
fn apply(app: &mut AppState, rect: Rect, d: SurfaceDraft) {
    match d.kind {
        DraftKind::Gradient => gradient(app, rect, d.start, d.current),
        DraftKind::Figure => {
            if d.start.distance(d.current) > 0.01 {
                let (a, b) = endpoints(d.start, d.current, app.drafting.figure, d.shift, d.alt);
                figure(app, rect, a, b);
            }
        }
        DraftKind::Ruler => crate::rulers::edit3d::create(app, rect, &d),
    }
}

/// 画面の形を、今のテクスチャセットの見えている面のテクセルへ写す（1 回の操作の予算で）。面の向きの弱めと継ぎ目のにじみはブラシの「3D」の
/// 切り替えのまま、隠れた所と裏の面は、切り替えによらず塗らない（形は画面の全体まで広がるので、見ていない面をまとめて変えない）。写せなければ
/// 理由を知らせて None。
pub(super) fn cover(
    app: &mut AppState,
    rect: Rect,
    shape: ScreenShape<'_>,
    positions: bool,
    source: Source,
) -> Option<ScreenCoverage> {
    let model = app.view3d.model.clone()?;
    let material = app.view3d.material;
    if material < 0 {
        app.refuse(source, app.region_missing_reason());
        return None;
    }
    let view = camera_view(app, rect);
    let settings = ScreenCoverSettings {
        material: Some(material),
        projection: ProjectionSettings {
            paint_hidden: false,
            paint_backfaces: false,
            ..app.view3d.projection
        },
        shape,
        positions,
        room: app.doc.stroke_budget_bytes(),
    };
    match cover_screen(&app.doc, model.geometry.clone(), &view, &settings) {
        Ok(c) => Some(c),
        Err(e) => {
            app.fail(source, app.lang.surface_error(&e));
            None
        }
    }
}

/// 画面の始点 a から終点 b へのグラデーション（2D と同じ断り・設定・式。色は見えているテクセルが写る画面の点で決める）。
fn gradient(app: &mut AppState, rect: Rect, a: DVec2, b: DVec2) {
    if app.is_stroking() {
        app.refuse(
            Source::Gradient,
            crate::lang::refusals::during_stroke(app.lang),
        );
        return;
    }
    if a.distance(b) < AppState::GRADIENT_CLICK {
        return;
    }
    if let Some(reason) = app.read_only_reason().map(str::to_owned) {
        app.refuse(
            Source::Gradient,
            crate::lang::refusals::read_only_set(app.lang, &reason),
        );
        return;
    }
    let Some((id, masked)) = app.gradient_target() else {
        return;
    };
    let Some(coverage) = cover(app, rect, ScreenShape::View, true, Source::Gradient) else {
        return;
    };
    app.gradient_fill(id, masked, (a, b), Some(coverage.mask()), &|x, y| {
        coverage.screen(x, y)
    });
}

/// 向かい合う角 a・b の図形を描く（塗るか、輪郭を 3D のストロークで）。
fn figure(app: &mut AppState, rect: Rect, a: DVec2, b: DVec2) {
    if app.is_stroking() {
        return;
    }
    let layer = match crate::region::tools::paint_gate(app) {
        Ok(id) => id,
        Err(e) => {
            app.refuse(Source::Ruler, e);
            return;
        }
    };
    let figure = app.drafting.figure;
    let corner = app.drafting.corner as f64;
    let before = app.doc.revision();
    app.doc.end_coalescing();
    if app.drafting.fill && figure != Figure::Line {
        let shape = match figure {
            Figure::Rectangle => ScreenShape::Rectangle { a, b, corner },
            _ => ScreenShape::Ellipse { a, b },
        };
        let Some(coverage) = cover(app, rect, shape, false, Source::Ruler) else {
            return;
        };
        let result = fill_region(app, layer, coverage.mask()).map(|_| ());
        finish_paint(app, result, before);
    } else {
        outline_stroke(app, rect, layer, &outline(figure, a, b, corner), before);
    }
}

/// 輪郭の画面の点の並びを、3D のストロークの入力にして描く（1 回の Undo。塗れなければストロークごと取り消す）。
fn outline_stroke(
    app: &mut AppState,
    rect: Rect,
    layer: crate::engine::LayerId,
    points: &[DVec2],
    before: u64,
) {
    let (Some(model), Some(&first)) = (app.view3d.model.clone(), points.first()) else {
        return;
    };
    if app.view3d.material < 0 {
        app.refuse(Source::Ruler, app.region_missing_reason());
        return;
    }
    let view = camera_view(app, rect);
    // 筆圧は一定、時刻は同じ（2D の図形と同じく、速さの制御が頂点の間隔で効かない）
    let input = |p: DVec2| SurfaceInput::new(Vec2::new(p.x as f32, p.y as f32), 1.0);
    let Some(Opened {
        mut stroke,
        mut surface,
        ..
    }) = open_stroke(app, &model, rect, view, layer, false, true, input(first))
    else {
        return;
    };
    let mut result = Ok(());
    let mut last = first;
    'points: for &p in &points[1..] {
        let n = ((p - last).length() / OUTLINE_STEP).ceil().max(1.0) as usize;
        for i in 1..=n {
            let q = last + (p - last) * (i as f64 / n as f64);
            if let Err(e) = surface.add_input(&mut app.doc, &mut stroke, input(q)) {
                result = Err(e);
                break 'points;
            }
        }
        last = p;
    }
    if result.is_ok() {
        result = surface.finish(&mut app.doc, &mut stroke);
    }
    let ended = match result {
        Err(e) => {
            app.doc.cancel_stroke(stroke);
            app.fail(Source::Ruler, app.lang.surface_error(&e));
            Ok(())
        }
        Ok(()) => {
            if let Some(outcome) = surface.symmetry_note() {
                app.warn(Source::View3d, app.lang.mirror_note(outcome));
            }
            app.doc.end_stroke(stroke).map(|_| ())
        }
    };
    finish_paint(app, ended, before);
}

/// 3D ビューの上に、ドラッグの途中の形（グラデーションの線・図形の輪郭・引いている定規）を描く（2D と同じ見た目）。作った定規は `rulers::draw3d`。
pub fn paint_overlay(painter: &Painter, app: &AppState, rect: Rect) {
    let screen = |p: DVec2| to_pos(rect, p);
    let draft = app.view3d.input.draft;
    match draft {
        Some(d) if d.kind == DraftKind::Gradient => {
            crate::gradient::canvas::paint_line(painter, screen(d.start), screen(d.current));
        }
        Some(d) if d.kind == DraftKind::Figure => {
            let (a, b) = endpoints(d.start, d.current, app.drafting.figure, d.shift, d.alt);
            let points = outline(app.drafting.figure, a, b, app.drafting.corner as f64)
                .into_iter()
                .map(screen)
                .collect();
            paint_outline(painter, points);
        }
        Some(d) if d.kind == DraftKind::Ruler => {
            if let Some(ruler) = crate::rulers::edit3d::build(app, rect, &d) {
                crate::rulers::draw3d::paint_ruler(
                    painter,
                    app,
                    rect,
                    &ruler,
                    crate::rulers::draw3d::Look::Draft,
                );
            }
        }
        _ => {}
    }
}
