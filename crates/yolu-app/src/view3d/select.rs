//! 3D ビューの選択範囲のツール（長方形選択・楕円形選択・なげなわ・多角形選択・自動選択）。結果は 2D のキャンバスと同じ文書の選択範囲で、
//! 新規・追加・削除・共通（ツールプロパティと修飾キー）・アンチエイリアス・縦横比の固定・中心から・長方形の角の丸めも 2D と同じに効く。
//! - 形（長方形・楕円・なげなわ・多角形）: 画面の上でドラッグ（多角形はクリックで点）して、離したとき（多角形は閉じたとき）に、カメラから見えている面の
//!   テクセルへ写して選択範囲にする（core の `cover_screen`。図形の「塗る」と同じ写し方で、隠れた所と裏の面は選ばず、面の向きの弱めと継ぎ目のにじみは
//!   ブラシの「3D」の切り替えのまま）。縁は、1 テクセルを 4 × 4 の点で見た形の覆い（2D の楕円・多角形の選択範囲と同じ量）。長方形は角を丸めたときだけ
//!   縁が滑らかになり、ほかはアンチエイリアスの設定に従う（2D と同じ）。角の丸めの長さは画面の点。
//! - 自動選択: 押した所の面のテクセルを求め、そのテクセルから 2D と同じ自動選択（許容・隣接・全レイヤー）を走らせる。モデルに当たらなければ何も選ばない。
//!
//! 引いている形は画面の点（表示域の左上が原点）で持ち、離すまで文書を変えない。多角形の点は、視点を動かしても画面の同じ所に残る（定規と同じ）。
//! Esc・フォーカスを失う・ツールの切り替え・文書の切り替えで途中の形を捨てる。離したのを取りこぼしたとき（ウィンドウの外で離した・ペンの押しを OS に
//! 奪われて補った離し）は、離した位置が分からないので、図形・定規と同じく何も選ばずにやめる（2D のキャンバスの選択は最後の位置で選ぶ）。
//! 状態は `AppState::sel.view3d`（`selection::SelState`）が持つので、ツールの切り替えと 2D のキャンバスの Esc も、2D の途中の形と同じ口で捨てる。

use egui::{Modifiers, Painter, PointerButton, Pos2, Rect};
use yolu_core::geometry::ScreenShape;
use yolu_core::glam::DVec2;
use yolu_core::selection::MAX_POLYGON_POINTS;
use yolu_core::SelectionCombine;

use super::draft;
use super::input::{screen_point, to_pos};
use crate::notice::Source;
use crate::region::tools::{refuse_miss_as, surface_point};
use crate::selection::canvas::{
    drag_mode, paint_polygon_draft, paint_shape_outline, CLICK_RADIUS, CLOSE_RADIUS, DOUBLE_CLICK,
};
use crate::selection::{combine_of, shape, SelAction, SelEdit};
use crate::state::{Action, AppState, StrokeSource, Tool};

/// 3D ビューで引いている形（長方形・楕円・なげなわ）。点は表示域の画面の点（左上が原点）。
#[derive(Clone, Debug)]
pub struct Drag {
    pub tool: Tool,
    pub source: StrokeSource,
    pub start: DVec2,
    pub current: DVec2,
    /// なげなわの点（1 点以上離れたものだけ）。
    pub lasso: Vec<DVec2>,
    /// 押した所から動いた最大の距離（クリックかドラッグか。画面の点）。
    pub moved: f64,
    /// 押し始めの修飾（Shift・Ctrl は組み合わせ方、Shift は離すまで縦横比の固定とは別）と、いまの修飾。
    pub pressed_with: Modifiers,
    pub now: Modifiers,
}

/// 3D ビューの選択のツールの途中（引いている形と、打っている多角形の点）。
#[derive(Default)]
pub struct Draft {
    pub drag: Option<Drag>,
    /// 多角形の途中の点（表示域の画面の点）。
    pub polygon: Vec<DVec2>,
    /// 最後に押した時刻と点（ダブルクリックで多角形を閉じる）。
    last_press: Option<(f64, DVec2)>,
    /// このフレームの時刻（秒。`Context` の時計。マウスもペンも同じ時計で、ダブルクリックを見分ける）。
    now: f64,
}

impl Draft {
    /// 途中の形と点を全部捨てる。何かあったか。
    pub fn cancel(&mut self) -> bool {
        let any = self.drag.is_some() || !self.polygon.is_empty();
        self.drag = None;
        self.drop_polygon();
        any
    }

    /// 打っていた多角形の点を捨てる（2D のキャンバスで多角形を打ち始めたとき）。
    pub fn drop_polygon(&mut self) {
        self.polygon.clear();
        self.last_press = None;
    }

    /// 多角形の最後の点を消す。点があったか。
    pub fn remove_last_point(&mut self) -> bool {
        let had = self.polygon.pop().is_some();
        if self.polygon.is_empty() {
            self.last_press = None;
        }
        had
    }
}

/// 3D ビューで形を引いている途中か（離す・Esc・フォーカスを失うまで。多角形の点を打っている間は含めない）。
pub fn dragging(app: &AppState) -> bool {
    app.sel.view3d.drag.is_some()
}

/// このフレームの時刻を覚える（フレームの初めに。多角形のダブルクリックを、マウスもペンも同じ時計で見る）。
pub(super) fn frame(app: &mut AppState, now: f64) {
    app.sel.view3d.now = now;
}

/// 押した（選択のツールの押し）。
pub(super) fn press(
    app: &mut AppState,
    rect: Rect,
    at: Pos2,
    source: StrokeSource,
    modifiers: Modifiers,
) {
    if app.is_stroking()
        || app.view3d.model.is_none()
        || app.sel.drag.is_some()
        || app.sel.view3d.drag.is_some()
    {
        return;
    }
    if let Some(reason) = app.read_only_reason().map(str::to_owned) {
        app.refuse(
            Source::Selection,
            crate::lang::refusals::read_only_set(app.lang, &reason),
        );
        return;
    }
    let p = screen_point(rect, at);
    match app.tool {
        Tool::Wand => wand(app, rect, at, modifiers),
        Tool::Polygon => polygon_press(app, rect, p, modifiers),
        tool @ (Tool::SelectRect | Tool::SelectEllipse | Tool::Lasso) => {
            app.sel.view3d.drag = Some(Drag {
                tool,
                source,
                start: p,
                current: p,
                lasso: vec![p],
                moved: 0.0,
                pressed_with: modifiers,
                now: modifiers,
            });
        }
        _ => {}
    }
}

/// 自動選択: 押した所の面のテクセルから、2D と同じ自動選択を走らせる。
fn wand(app: &mut AppState, rect: Rect, at: Pos2, modifiers: Modifiers) {
    match surface_point(app, rect, at) {
        Ok((x, y)) => {
            let mode = combine_of(app.sel.combine, PointerButton::Primary, modifiers);
            // 3D ビューでは対称定規の写しを使わない（種は押した面の点だけ）
            app.apply(Action::Sel(SelAction::Edit(SelEdit::Wand {
                seeds: vec![(x.floor() as u32, y.floor() as u32)],
                mode,
            })));
        }
        Err(miss) => refuse_miss_as(app, miss, Source::Selection),
    }
}

/// 多角形の点を打つ（始めの点の近く・ダブルクリックで閉じる）。
fn polygon_press(app: &mut AppState, rect: Rect, p: DVec2, modifiers: Modifiers) {
    // 2D のキャンバスで打っていた途中の点は捨てる（途中の多角形は 1 つ）
    app.sel.polygon.clear();
    app.sel.polygon_hover = None;
    let draft = &mut app.sel.view3d;
    let now = draft.now;
    let double = draft.last_press.is_some_and(|(t, q)| {
        now - t <= DOUBLE_CLICK && q.distance(p) <= f64::from(CLICK_RADIUS) * 2.0
    });
    draft.last_press = Some((now, p));
    let n = draft.polygon.len();
    if n >= 3 {
        let near_first = draft.polygon[0].distance(p) <= f64::from(CLOSE_RADIUS);
        if double || near_first {
            finish_polygon(app, rect, modifiers);
            return;
        }
    } else if double && n > 0 {
        return; // 点が足りないままのダブルクリックは、同じ所に点を重ねない
    }
    if draft.polygon.len() < MAX_POLYGON_POINTS {
        draft.polygon.push(p);
    }
}

/// 多角形を閉じて選択範囲にする（Enter・始めの点・ダブルクリック）。3 点に満たなければ捨てる。
pub(super) fn finish_polygon(app: &mut AppState, rect: Rect, modifiers: Modifiers) {
    let points = std::mem::take(&mut app.sel.view3d.polygon);
    app.sel.view3d.last_press = None;
    if points.is_empty() {
        return;
    }
    if points.len() < 3 {
        crate::selection::canvas::refuse_too_few_points(app);
        return;
    }
    let mode = combine_of(app.sel.combine, PointerButton::Primary, modifiers);
    select_shape(app, rect, ScreenShape::Polygon { points: &points }, mode);
}

/// Enter（多角形を閉じる）・Backspace（最後の点を消す）。扱ったか。
pub(super) fn key(app: &mut AppState, rect: Rect, key: egui::Key, modifiers: Modifiers) -> bool {
    if app.tool != Tool::Polygon || app.sel.view3d.polygon.is_empty() {
        return false;
    }
    match key {
        egui::Key::Enter => finish_polygon(app, rect, modifiers),
        egui::Key::Backspace => {
            app.sel.view3d.remove_last_point();
        }
        _ => return false,
    }
    true
}

/// ポインタ・ペンが動いた（この入力で始めたドラッグだけ）。
pub(super) fn moved(app: &mut AppState, rect: Rect, at: Pos2, source: StrokeSource, m: &Modifiers) {
    let p = screen_point(rect, at);
    let Some(d) = app.sel.view3d.drag.as_mut().filter(|d| d.source == source) else {
        return;
    };
    d.current = p;
    d.now = *m;
    d.moved = d.moved.max(p.distance(d.start));
    if d.tool == Tool::Lasso
        && d.lasso.len() < MAX_POLYGON_POINTS
        && d.lasso.last().is_none_or(|l| l.distance(p) >= 1.0)
    {
        d.lasso.push(p);
    }
}

/// 毎フレーム: 押しているあいだの Shift・Alt は、動かさなくても形に効く（2D と同じ）。
pub(super) fn modifiers(app: &mut AppState, m: &Modifiers) {
    if let Some(d) = app.sel.view3d.drag.as_mut() {
        d.now = *m;
    }
}

/// 離した: 形を選択範囲にする（クリックなら解除。この入力で始めたドラッグだけ）。
pub(super) fn release(
    app: &mut AppState,
    rect: Rect,
    at: Pos2,
    source: StrokeSource,
    m: &Modifiers,
) {
    if app
        .sel
        .view3d
        .drag
        .as_ref()
        .is_none_or(|d| d.source != source)
    {
        return;
    }
    moved(app, rect, at, source, m);
    let Some(mut drag) = app.sel.view3d.drag.take() else {
        return;
    };
    if drag.tool == Tool::Lasso && drag.lasso.len() < MAX_POLYGON_POINTS {
        drag.lasso.push(drag.current);
    }
    commit(app, rect, drag);
}

/// 離したのを取りこぼした（ウィンドウの外で離した・ペンの押しを OS に奪われて補った離し）: 何も選ばずにやめる。この入力で始めたドラッグだけ。
pub(super) fn lost_release(app: &mut AppState, source: StrokeSource) {
    if app
        .sel
        .view3d
        .drag
        .as_ref()
        .is_some_and(|d| d.source == source)
    {
        app.sel.view3d.drag = None;
    }
}

/// フォーカスを失った: 離したのを受け取れないので、途中の形と点を何も選ばずに捨てる。
pub(super) fn focus_lost(app: &mut AppState) {
    app.sel.view3d.cancel();
}

/// Esc: 途中の形と点を捨てる。何かあったか。捨てたら、この Esc は使ったことにする（先に描く 3D ビューのあとで、キャンバスが同じ Esc で、前からある選択範囲まで
/// 解除しない）。
pub(super) fn cancel(app: &mut AppState, ctx: &egui::Context) -> bool {
    let any = app.sel.view3d.cancel();
    if any {
        crate::ui::window::note_escape_taken(ctx);
        app.info(
            Source::Selection,
            app.lang
                .pick("選択の途中をやめました。", "Selection cancelled."),
        );
    }
    any
}

/// 引いた形を選択範囲にする。
fn commit(app: &mut AppState, rect: Rect, drag: Drag) {
    let click = drag.moved < f64::from(CLICK_RADIUS);
    let mode = drag_mode(
        app.sel.combine,
        drag.tool,
        PointerButton::Primary,
        drag.pressed_with,
        drag.now,
    );
    if click {
        // 動かさずに離したら解除（新規のとき。追加・削除・共通では何もしない）
        if mode == SelectionCombine::Replace && app.doc.selection().is_some() {
            app.apply(Action::Sel(SelAction::Edit(SelEdit::Clear)));
        }
        return;
    }
    let c = shape::Constraint::of(&app.sel, drag.tool, drag.pressed_with, drag.now);
    let (a, b) = shape::drag_corners(
        (drag.start.x, drag.start.y),
        (drag.current.x, drag.current.y),
        c.square,
        c.center,
    );
    let (a, b) = (DVec2::new(a.0, a.1), DVec2::new(b.0, b.1));
    match drag.tool {
        Tool::SelectRect => {
            let corner = f64::from(app.sel.corner_radius);
            select_shape(app, rect, ScreenShape::Rectangle { a, b, corner }, mode)
        }
        Tool::SelectEllipse => select_shape(app, rect, ScreenShape::Ellipse { a, b }, mode),
        Tool::Lasso => select_shape(
            app,
            rect,
            ScreenShape::Polygon {
                points: &drag.lasso,
            },
            mode,
        ),
        _ => {}
    }
}

/// 画面の形を見えている面のテクセルへ写し、今の選択範囲と `mode` で組み合わせて文書に置く（1 回の取り消し）。縁の滑らかさは 2D と同じ:
/// 角を丸めない長方形は常に縁が 0 か 255 だけ、ほかはアンチエイリアスの設定に従う。
fn select_shape(app: &mut AppState, rect: Rect, shape: ScreenShape<'_>, mode: SelectionCombine) {
    let Some(coverage) = draft::cover(app, rect, shape, false, Source::Selection) else {
        return;
    };
    let plain_rectangle = matches!(shape, ScreenShape::Rectangle { corner, .. } if corner <= 0.0);
    let mask = if plain_rectangle {
        coverage.mask().sharpen()
    } else {
        app.edge_of(coverage.mask().clone())
    };
    app.apply(Action::Sel(SelAction::Edit(SelEdit::Shape { mask, mode })));
}

/// 3D ビューの上に、引いている形と打っている多角形の途中を描く（2D のキャンバスと同じ見た目）。`hover` はポインタ（この表示域の上にあるとき）。
pub fn paint_overlay(painter: &Painter, app: &AppState, rect: Rect, hover: Option<Pos2>) {
    let screen = |p: (f64, f64)| to_pos(rect, DVec2::new(p.0, p.1));
    if let Some(d) = &app.sel.view3d.drag {
        if d.moved >= f64::from(CLICK_RADIUS) {
            let c = shape::Constraint::of(&app.sel, d.tool, d.pressed_with, d.now);
            let (a, b) = shape::drag_corners(
                (d.start.x, d.start.y),
                (d.current.x, d.current.y),
                c.square,
                c.center,
            );
            let lasso: Vec<(f64, f64)> = d.lasso.iter().map(|p| (p.x, p.y)).collect();
            paint_shape_outline(
                painter,
                d.tool,
                (a, b),
                &lasso,
                f64::from(app.sel.corner_radius),
                &screen,
            );
        }
    }
    let polygon = &app.sel.view3d.polygon;
    if app.tool == Tool::Polygon && !polygon.is_empty() {
        let placed: Vec<Pos2> = polygon.iter().map(|p| to_pos(rect, *p)).collect();
        paint_polygon_draft(painter, &placed, hover);
    }
}
