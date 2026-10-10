//! 3D ビューの選択ペン: クイックマスクが入っている間、3D ビューのブラシ・消しゴムは、2D のキャンバスと同じく選択ペン・選択消しとして働き、
//! 選択ペンのツール（と選択消し）も同じ道で、面に塗って選択範囲を直す（直径・硬さ・不透明度・筆圧は今のブラシ。1 ストロークが 1 回の取り消し
//! `SelEdit::Shape`）。塗る先は今のテクスチャセットの文書の選択範囲で、面のダブの覆い（投影の塗り。core の `SurfaceCoverStroke`）を UV の画素の
//! 被覆へ積む。被覆・見た目・終わらせ方は 2D の選択ペン（`selection::pen`）のまま（2D のキャンバスが出ていれば、描いている間の重ねも変わる）。
//! レイヤーの画素は変えない。2D の選択ペンと同じく対称は使わない。

use egui::{Pos2, Rect};
use yolu_core::geometry::{CoverPixel, DabRefusal, SurfaceCoverStroke, SurfaceStrokeError};

use super::input::{camera_view, local};
use super::model::ViewModel;
use crate::notice::Source;
use crate::selection::pen::{self, PenError, PenParams};
use crate::state::{AppState, StrokeSource};

/// 3D ビューの選択ペンのストロークの始め（クイックマスクのブラシ・消しゴム `quick`、または選択ペンのツール）。`erase` は消す側か。始めたら 3D の入力に
/// ストロークの印を立てる。始められなければ理由を知らせる。
#[allow(clippy::too_many_arguments)]
pub(super) fn begin(
    app: &mut AppState,
    model: &ViewModel,
    rect: Rect,
    at: Pos2,
    pressure: f32,
    source: StrokeSource,
    erase: bool,
    quick: bool,
) {
    // ほかのビューで選択ペンのストロークが動いている間は始めない（被覆を上書きしない）
    if app.sel.pen.is_some() {
        app.refuse(
            Source::Selection,
            crate::lang::refusals::during_stroke(app.lang),
        );
        return;
    }
    if !pen::begin(app, source, erase, quick) {
        return;
    }
    let params = PenParams::from_brush(&app.brush).cover();
    let begun = SurfaceCoverStroke::begin(
        model.geometry.clone(),
        camera_view(app, rect),
        Some(app.view3d.material),
        params,
        app.view3d.projection,
        app.doc.width(),
        app.doc.height(),
        local(rect, at),
        pressure.clamp(0.0, 1.0),
        room(app),
    );
    match begun {
        Ok((cover, pixels)) => {
            app.view3d.input.cover = Some(cover);
            app.view3d.input.stroke = Some(source);
            app.view3d.input.stroke_points = 1;
            raise(app, &pixels);
        }
        Err(e) => {
            app.sel.pen = None;
            fail(app, &e);
        }
    }
}

/// 3D のクイックマスクのストロークの点。
pub(super) fn add(app: &mut AppState, rect: Rect, at: Pos2, pressure: f32) {
    let room = room(app);
    let Some(cover) = app.view3d.input.cover.as_mut() else {
        return;
    };
    match cover.add(local(rect, at), pressure.clamp(0.0, 1.0), room) {
        Ok(pixels) => {
            app.view3d.input.stroke_points += 1;
            raise(app, &pixels);
        }
        Err(e) => {
            app.sel.pen = None;
            app.view3d.stroke_ended();
            fail(app, &e);
        }
    }
}

/// 3D のクイックマスクのストロークを終える（cancel なら捨てる）。3D のクイックマスクのストロークでなければ false。
pub(super) fn finish(app: &mut AppState, cancel: bool) -> bool {
    if app.view3d.input.cover.take().is_none() {
        return false;
    }
    pen::finish(app, cancel);
    true
}

/// 投影の塗りに使ってよいバイト（選択ペンの作業の予算から、被覆の分を引いた残り）。
fn room(app: &AppState) -> u64 {
    app.sel
        .pen
        .as_ref()
        .map_or(0, |a| a.stroke.budget().saturating_sub(a.stroke.bytes()))
}

/// ダブの量を被覆へ積む。予算を超えたらストロークを捨てて理由を出す。
fn raise(app: &mut AppState, pixels: &[CoverPixel]) {
    let Some(active) = app.sel.pen.as_mut() else {
        return;
    };
    if let Err(e) = active.stroke.raise(pixels) {
        app.sel.pen = None;
        app.view3d.stroke_ended();
        app.fail(Source::Selection, e.text(app.lang));
    }
}

/// 面のストロークの失敗を知らせる（投影の塗りのメモリが足りないのは、2D の選択ペンの予算と同じ文）。
fn fail(app: &mut AppState, e: &SurfaceStrokeError) {
    match e {
        SurfaceStrokeError::Dab(DabRefusal::MemoryBudget) => {
            app.fail(Source::Selection, PenError::TooLarge.text(app.lang))
        }
        e => app.fail(Source::Selection, app.lang.surface_error(e)),
    }
}
