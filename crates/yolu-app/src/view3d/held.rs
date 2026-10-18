//! 3D ビューでマウスが「押している物」の表（2D の `CanvasTool::lost_release` と同じ考え）。
//! ストローク・グラデーション/図形/定規・選択の形・パスのドラッグと矩形・ポーズのギズモ・形のギズモ・点のグラデーションの点。
//! ボタンを離したのを取りこぼしたとき（ウィンドウの外で離したなど）と、本物の離しのとき、`input::handle` はこの表を `ALL` の順に回して、
//! 押している物だけを終える。物を足すときは、`Held`・`ALL` と、`held`・`lost_release`・`release` の match に足す（match は網羅なので、
//! 足し漏れはコンパイルで出る。`input::handle` の if は増やさない）。
//!
//! 今この表を通るのはマウスの押しだけ（`input::handle` の 2 か所とも `StrokeSource::Mouse`）。ペンの離しは `input::pen_sample` が持ち主ごとに終える。
//! `held` が `source` を取るのは、将来ペンの離しも同じ表で終えるため（ペンの分岐は今は通らない）。
//! `Points` は 3D が始めた物（`in_3d`）だけを見る。`Gizmo`（`fillfx.drag`）は 2D と 3D を分けず、`source` だけを見る。

use egui::{Context, Pos2, Rect};

use super::{draft, gizmo, input, select};
use crate::fillfx::gizmo::Source as GizmoSource;
use crate::state::key::Modifiers;
use crate::state::{AppState, StrokeSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Held {
    /// 面のストロークと、範囲・クイックマスクのストローク。
    Stroke,
    /// グラデーション・図形・定規のドラッグ。
    Draft,
    /// 選択の形のドラッグ。
    Select,
    /// パスの点のドラッグと、点を矩形で選ぶドラッグ。
    Path,
    /// ポーズのギズモのドラッグ（マウスでしか始まらない）。
    Pose,
    /// 形のギズモのドラッグ。
    Gizmo,
    /// 点のグラデーションの点のドラッグ（3D ビューで始めた物）。
    Points,
}

impl Held {
    /// 取りこぼしと本物の離しで終える順。
    pub(super) const ALL: [Held; 7] = [
        Held::Stroke,
        Held::Draft,
        Held::Select,
        Held::Path,
        Held::Pose,
        Held::Gizmo,
        Held::Points,
    ];

    /// この入力（`source`）が、これを押している最中か。
    pub(super) fn held(self, app: &AppState, source: StrokeSource) -> bool {
        match self {
            Held::Stroke => app.view3d.view.input.stroke == Some(source),
            Held::Draft => app
                .view3d
                .view
                .input
                .draft
                .is_some_and(|d| d.source == source),
            Held::Select => {
                select::dragging(app)
                    && app
                        .sel
                        .view3d
                        .drag
                        .as_ref()
                        .is_some_and(|d| d.source == source)
            }
            Held::Path => {
                app.path
                    .drag
                    .is_some_and(|d| d.source == source && d.surface)
                    || app
                        .path
                        .rect
                        .is_some_and(|r| r.source == source && r.surface)
            }
            Held::Pose => source == StrokeSource::Mouse && app.view3d.pose.drag.is_some(),
            Held::Gizmo => app
                .fillfx
                .drag
                .as_ref()
                .is_some_and(|d| d.source == gizmo_source(source)),
            Held::Points => app
                .fillfx
                .point_drag
                .as_ref()
                .is_some_and(|d| d.in_3d && d.source == gizmo_source(source)),
        }
    }

    /// ボタンを離したのを取りこぼした（離した位置が分からない）ときの終わらせ方。
    pub(super) fn lost_release(
        self,
        app: &mut AppState,
        rect: Rect,
        source: StrokeSource,
        ctx: &Context,
    ) {
        match self {
            Held::Stroke => input::release(app, ctx),
            // グラデーションは最後の位置で塗り、図形と定規はやめる
            Held::Draft => draft::lost_release(app, rect, source),
            // 何も選ばずにやめる
            Held::Select => select::lost_release(app, source),
            Held::Path => {
                if app
                    .path
                    .drag
                    .is_some_and(|d| d.source == source && d.surface)
                {
                    app.path_finish_drag();
                }
                // 点を矩形で選ぶドラッグは、最後の位置で確定する（2D のツールと同じ）。位置が無ければ捨てる
                if app
                    .path
                    .rect
                    .is_some_and(|r| r.source == source && r.surface)
                {
                    match app.view3d.view.input.last_pointer {
                        Some(at) => crate::pathtool::surface::release(app, rect, at, source),
                        None => app.path.rect = None,
                    }
                }
            }
            Held::Pose => gizmo::release(app, true),
            Held::Gizmo => crate::fillfx::gizmo::release(app, true),
            // 最後の位置で確定する（2D の取りこぼしと、形のギズモの取りこぼしと同じ）
            Held::Points => crate::fillfx::points::release(app, true),
        }
    }

    /// 本物の離しの終わらせ方（離した位置 `at`）。ギズモのドラッグは、呼ぶ側が最後の位置を当ててから回す。
    pub(super) fn release(
        self,
        app: &mut AppState,
        rect: Rect,
        at: Pos2,
        source: StrokeSource,
        modifiers: &Modifiers,
        ctx: &Context,
    ) {
        match self {
            Held::Stroke => input::release(app, ctx),
            Held::Draft => draft::release(app, rect, at, source, modifiers),
            Held::Select => select::release(app, rect, at, source, modifiers),
            Held::Path => crate::pathtool::surface::release(app, rect, at, source),
            Held::Pose => gizmo::release(app, true),
            Held::Gizmo => crate::fillfx::gizmo::release(app, true),
            Held::Points => crate::fillfx::points::release(app, true),
        }
    }
}

fn gizmo_source(source: StrokeSource) -> GizmoSource {
    match source {
        StrokeSource::Mouse => GizmoSource::Mouse,
        StrokeSource::Pen(id) => GizmoSource::Pen(id),
    }
}
