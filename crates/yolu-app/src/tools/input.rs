//! ツールの入力の受け口。キャンバスの入力（`canvas`）と 3D ビューの入力（`view3d::input`）は、ツールごとの枝を持たず、ツールの表（`tools::TOOLS`）が
//! 指す受け口を引く。2D は `CanvasTool`（押す・動く・離す・ペン・Esc・Enter・取りこぼした離し・フォーカスを失う）。ドラッグの札を持つツール
//! （移動・変形とゆがみ、図形と定規、グラデーション、パス、選択）がそれぞれ 1 つの実装で、ドラッグを始めた側が離す・Esc・フォーカスの喪失で終える
//! （途中でツールを替えても、始めた側を終わらせる）ので、動き・離す・取りこぼしは、ツールでなくドラッグの持ち主で振り分ける。ブラシ・消しゴム・範囲のツール・
//! スポイトはストロークで描くので `CanvasTool` を持たず、キャンバスの入力が直に扱う。3D は `Surface`（面の上で押したときの行き先）。
//! 各ツールの実際の仕事は、ツールのモジュール（`transform::canvas` など）の関数。ここはその呼び出しを同じ形にそろえるだけ。

use egui::{CursorIcon, Key, Modifiers, Pos2, Rect};

use crate::canvas::view::CanvasView;
use crate::state::{AppState, StrokeSource};

/// 入力の 1 回の前提（押した・動いた・離したときの修飾キーと時刻、キャンバスの表示域、描き直しの番号）。
#[derive(Clone, Copy, Debug)]
pub struct InputCtx {
    pub modifiers: Modifiers,
    pub now: f64,
    pub rect: Rect,
    /// `Context::cumulative_pass_nr`（同じパスの Esc を 2 度数えない）。
    pub pass: u64,
}

/// 2D のキャンバスの入力を受けるツール（ドラッグの札を持つもの）。
pub trait CanvasTool: Sync {
    /// 押した（今のツールが自分のとき）。
    fn press(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        ctx: &InputCtx,
    );
    /// ポインタが動いた（`wants_move` のとき）。
    fn moved(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        ctx: &InputCtx,
    );
    /// 離した（自分のドラッグがあるとき）。
    fn release(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        ctx: &InputCtx,
    );
    /// ペン（Windows Ink）の 1 点。触れた・動いた・離したを、押す・動く・離すにする。
    fn pen(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        id: u32,
        contact: bool,
        ctx: &InputCtx,
    );
    /// ペンの押しを OS に奪われて、離しが補われた（離した位置は最後の点で、本物の離しではない。`AppState::pen_release_lost`）。既定は普通の離し（最後の位置で離す）。
    /// `lost_release` を上書きして、離した位置を使わない終わらせ方にしたツールは、これも同じ終わらせ方に上書きする。
    fn pen_lost(&self, app: &mut AppState, view: &CanvasView, at: Pos2, id: u32, ctx: &InputCtx) {
        self.pen(app, view, at, id, false, ctx);
    }
    /// このペンで押している途中か。
    fn pen_active(&self, app: &AppState, id: u32) -> bool;
    /// 自分のドラッグがあるか（`source` が Some なら、その入力で始めたもの）。
    fn dragging(&self, app: &AppState, source: Option<StrokeSource>) -> bool;
    /// 動きを渡すか（既定は自分のドラッグがあるとき）。
    fn wants_move(&self, app: &AppState) -> bool {
        self.dragging(app, None)
    }
    /// マウスを離したのを取りこぼしたとき（ウィンドウの外で離したなど）の終わらせ方。既定は、最後の位置で離したことにする。
    fn lost_release(&self, app: &mut AppState, view: &CanvasView, at: Pos2, ctx: &InputCtx) {
        self.release(app, view, at, StrokeSource::Mouse, ctx);
    }
    /// Esc: やめたら true。
    fn cancel(&self, app: &mut AppState, ctx: &InputCtx) -> bool;
    /// Esc で、ストロークや表示の回転より先に聞く（あとから聞くツールは false）。
    fn cancel_first(&self) -> bool {
        true
    }
    /// ウィンドウがフォーカスを失った: 途中の操作を取り残さない。
    fn focus_lost(&self, app: &mut AppState);
    /// 毎フレーム（そのフレームの修飾キーを、途中のドラッグへ渡す）。
    fn each_frame(&self, _app: &mut AppState, _ctx: &InputCtx) {}
    /// Enter・Backspace（文字を打っていない・ポップアップが無いとき）。扱ったら true。
    fn key(&self, _app: &mut AppState, _key: Key, _modifiers: Modifiers) -> bool {
        false
    }
    /// ステンシルを動かしている間は押しを渡さないツールか。
    fn respects_stencil(&self) -> bool {
        true
    }
}

/// キャンバスの入力を受けるツールの種類（表が指す。受け口の実体は `handler`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasKind {
    Transform,
    Drafting,
    Gradient,
    Path,
    Selection,
    Text,
}

impl CanvasKind {
    /// 全部（Esc などで聞く順。Esc は、ストロークと表示の回転より先に `cancel_first` のツール、あとに残りを聞く）。
    pub const ALL: [CanvasKind; 6] = [
        CanvasKind::Drafting,
        CanvasKind::Transform,
        CanvasKind::Path,
        CanvasKind::Gradient,
        CanvasKind::Selection,
        CanvasKind::Text,
    ];

    pub fn handler(self) -> &'static dyn CanvasTool {
        match self {
            CanvasKind::Transform => &TransformInput,
            CanvasKind::Drafting => &DraftingInput,
            CanvasKind::Gradient => &GradientInput,
            CanvasKind::Path => &PathInput,
            CanvasKind::Selection => &SelectionInput,
            CanvasKind::Text => &TextInput,
        }
    }
}

fn same_source(drag: StrokeSource, wanted: Option<StrokeSource>) -> bool {
    wanted.is_none_or(|s| s == drag)
}

// ───────── 移動・変形とゆがみ ─────────

struct TransformInput;

impl CanvasTool for TransformInput {
    fn press(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        ctx: &InputCtx,
    ) {
        // テキストレイヤーのダブルクリックは、テキストツールで打ち直す（離したときに替える）
        if app.tool == crate::state::Tool::Move
            && crate::textlayer::canvas::move_press(app, view, at, source, ctx.now)
        {
            return;
        }
        crate::transform::canvas::press(app, view, at, source, ctx.modifiers);
    }
    fn moved(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        ctx: &InputCtx,
    ) {
        crate::transform::canvas::moved(app, view, at, source, ctx.modifiers.shift);
    }
    fn release(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        ctx: &InputCtx,
    ) {
        if crate::textlayer::canvas::move_release(app, source) {
            return;
        }
        crate::transform::canvas::release(app, view, at, source, ctx.modifiers.shift);
    }
    fn pen(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        id: u32,
        contact: bool,
        ctx: &InputCtx,
    ) {
        crate::transform::canvas::pen_sample(app, view, at, id, contact, ctx.modifiers);
    }
    fn pen_active(&self, app: &AppState, id: u32) -> bool {
        app.transform.pen_down == Some(id)
    }
    fn dragging(&self, app: &AppState, source: Option<StrokeSource>) -> bool {
        app.transform
            .drag
            .as_ref()
            .is_some_and(|d| same_source(d.source, source))
            || app
                .text
                .move_press
                .as_ref()
                .is_some_and(|(_, _, s)| same_source(*s, source))
    }
    fn lost_release(&self, app: &mut AppState, _view: &CanvasView, _at: Pos2, _ctx: &InputCtx) {
        // ダブルクリックの押しは、離したのを受け取れなければ打ち直さない
        app.text.move_press = None;
        // 離した位置が分からないので、最後の位置で確定する
        crate::transform::canvas::commit(app);
    }
    fn cancel(&self, app: &mut AppState, _ctx: &InputCtx) -> bool {
        let double_click = app.text.move_press.take().is_some();
        crate::transform::canvas::cancel(app) || double_click
    }
    fn focus_lost(&self, app: &mut AppState) {
        // 離したのを受け取れないので、何も変えずにやめる
        app.text.move_press = None;
        app.transform_cancel_drag();
    }
    fn key(&self, app: &mut AppState, key: Key, _modifiers: Modifiers) -> bool {
        // ドラッグの途中の Enter: その位置で確定する（あとで離しても、もう何もしない）
        if key == Key::Enter && app.transform.drag.is_some() {
            crate::transform::canvas::commit(app);
            return true;
        }
        false
    }
}

// ───────── 図形と定規 ─────────

struct DraftingInput;

impl CanvasTool for DraftingInput {
    fn press(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        ctx: &InputCtx,
    ) {
        crate::drafting::canvas::press(app, view, at, source, ctx.modifiers);
    }
    fn moved(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        ctx: &InputCtx,
    ) {
        crate::drafting::canvas::moved(app, view, at, source, ctx.modifiers);
    }
    fn release(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        ctx: &InputCtx,
    ) {
        crate::drafting::canvas::release(app, view, at, source, ctx.modifiers, ctx.rect);
    }
    fn pen(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        id: u32,
        contact: bool,
        ctx: &InputCtx,
    ) {
        crate::drafting::canvas::pen_sample(app, view, at, id, contact, ctx.modifiers, ctx.rect);
    }
    fn pen_active(&self, app: &AppState, id: u32) -> bool {
        app.drafting.pen_down == Some(id)
    }
    fn each_frame(&self, app: &mut AppState, ctx: &InputCtx) {
        // 押しているあいだの Shift・Alt は、動かさなくても形に効く
        if let Some(drag) = app.drafting.drag.as_mut() {
            drag.shift = ctx.modifiers.shift;
            drag.alt = ctx.modifiers.alt;
        }
        if let Some(drag) = app.rulers.drag.as_mut() {
            drag.shift = ctx.modifiers.shift;
        }
    }
    fn dragging(&self, app: &AppState, source: Option<StrokeSource>) -> bool {
        app.drafting
            .drag
            .is_some_and(|d| same_source(d.source, source))
            || app
                .rulers
                .drag
                .as_ref()
                .is_some_and(|d| same_source(d.source, source))
    }
    fn lost_release(&self, app: &mut AppState, _view: &CanvasView, _at: Pos2, _ctx: &InputCtx) {
        // 図形は離した位置が不明なら取消し、画素を変更しない
        app.drafting_cancel();
    }
    fn pen_lost(
        &self,
        app: &mut AppState,
        _view: &CanvasView,
        _at: Pos2,
        id: u32,
        _ctx: &InputCtx,
    ) {
        // ペンの押しも、マウスの取りこぼしと同じく取り消す（補った離しの位置は、本物の離しの位置ではない）
        if app.drafting.pen_down == Some(id) {
            app.drafting_cancel();
            app.drafting.pen_down = None;
        }
    }
    fn cancel(&self, app: &mut AppState, _ctx: &InputCtx) -> bool {
        // 図形と定規は離すまで画素・定規を変更しない
        app.drafting_cancel()
    }
    fn focus_lost(&self, app: &mut AppState) {
        app.drafting_cancel();
        app.drafting.pen_down = None;
    }
}

// ───────── グラデーション ─────────

struct GradientInput;

impl CanvasTool for GradientInput {
    fn press(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        _ctx: &InputCtx,
    ) {
        crate::gradient::canvas::press(app, view, at, source);
    }
    fn moved(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        _ctx: &InputCtx,
    ) {
        crate::gradient::canvas::moved(app, view, at, source);
    }
    fn release(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        _ctx: &InputCtx,
    ) {
        crate::gradient::canvas::release(app, view, at, source);
    }
    fn pen(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        id: u32,
        contact: bool,
        _ctx: &InputCtx,
    ) {
        crate::gradient::canvas::pen_sample(app, view, at, id, contact);
    }
    fn pen_active(&self, app: &AppState, id: u32) -> bool {
        app.gradient.pen_down == Some(id)
    }
    fn dragging(&self, app: &AppState, source: Option<StrokeSource>) -> bool {
        app.gradient
            .drag
            .as_ref()
            .is_some_and(|d| same_source(d.source, source))
    }
    fn cancel(&self, app: &mut AppState, _ctx: &InputCtx) -> bool {
        crate::gradient::canvas::cancel(app)
    }
    fn cancel_first(&self) -> bool {
        false
    }
    fn focus_lost(&self, app: &mut AppState) {
        // 離したのを受け取れないので、何も塗らずにやめる
        app.gradient_cancel_drag();
    }
}

// ───────── パス ─────────

struct PathInput;

/// パスのツールへ、修飾キー（取っ手の Alt・Ctrl）と時刻（点のダブルクリック）を渡す。
fn path_input(app: &mut AppState, ctx: &InputCtx) {
    app.path.input = crate::pathtool::PathInputState {
        alt: ctx.modifiers.alt,
        ctrl: ctx.modifiers.command,
        shift: ctx.modifiers.shift,
        now: Some(ctx.now),
    };
}

impl CanvasTool for PathInput {
    fn press(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        ctx: &InputCtx,
    ) {
        path_input(app, ctx);
        crate::pathtool::canvas::press(app, view, at, source);
    }
    fn moved(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        ctx: &InputCtx,
    ) {
        path_input(app, ctx);
        crate::pathtool::canvas::moved(app, view, at, source);
    }
    fn release(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        ctx: &InputCtx,
    ) {
        path_input(app, ctx);
        crate::pathtool::canvas::release(app, view, at, source);
    }
    fn pen(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        id: u32,
        contact: bool,
        ctx: &InputCtx,
    ) {
        path_input(app, ctx);
        crate::pathtool::canvas::pen_sample(app, view, at, id, contact);
    }
    fn pen_active(&self, app: &AppState, id: u32) -> bool {
        // 3D のビューが触れているペンは、2D の入力では扱わない
        app.path.pen_down.is_some_and(|d| d.id == id && !d.surface)
    }
    fn dragging(&self, app: &AppState, source: Option<StrokeSource>) -> bool {
        // 2D のキャンバスで始めたドラッグだけ（3D の面のドラッグは `pathtool::surface`）。点を矩形で選ぶドラッグも
        app.path
            .drag
            .is_some_and(|d| !d.surface && same_source(d.source, source))
            || app
                .path
                .rect
                .is_some_and(|r| !r.surface && same_source(r.source, source))
    }
    fn lost_release(&self, app: &mut AppState, view: &CanvasView, _at: Pos2, _ctx: &InputCtx) {
        // 離したのを取りこぼしたら、最後の位置で確定する
        app.path_finish_drag();
        if app.path.rect.is_some_and(|r| !r.surface) {
            crate::pathtool::canvas::finish_rect(app, view);
        }
    }
    fn cancel(&self, app: &mut AppState, ctx: &InputCtx) -> bool {
        // 点のドラッグを捨てる（ドラッグが無ければ選んだ点を外す）
        app.path_cancel(ctx.pass)
    }
    fn focus_lost(&self, app: &mut AppState) {
        app.path_finish_drag();
        app.path.rect = None;
    }
}

// ───────── 選択 ─────────

struct SelectionInput;

impl CanvasTool for SelectionInput {
    fn press(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        ctx: &InputCtx,
    ) {
        crate::selection::canvas::press(app, view, at, source, ctx.modifiers, ctx.now);
    }
    fn moved(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        _ctx: &InputCtx,
    ) {
        crate::selection::canvas::moved(app, view, at, source);
    }
    fn release(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        ctx: &InputCtx,
    ) {
        crate::selection::canvas::release(app, view, at, source, ctx.modifiers);
    }
    fn pen(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        id: u32,
        contact: bool,
        ctx: &InputCtx,
    ) {
        crate::selection::canvas::pen_sample(app, view, at, id, contact, ctx.modifiers, ctx.now);
    }
    fn pen_active(&self, app: &AppState, id: u32) -> bool {
        app.sel.pen_down == Some(id)
    }
    fn dragging(&self, app: &AppState, source: Option<StrokeSource>) -> bool {
        app.sel
            .drag
            .as_ref()
            .is_some_and(|d| same_source(d.source, source))
    }
    fn wants_move(&self, app: &AppState) -> bool {
        // 多角形のゴムの線はドラッグが無くても動きを見る
        app.tool.is_select()
    }
    fn cancel(&self, app: &mut AppState, _ctx: &InputCtx) -> bool {
        crate::selection::canvas::cancel(app)
    }
    fn cancel_first(&self) -> bool {
        false
    }
    fn focus_lost(&self, app: &mut AppState) {
        // 選択の途中の形は捨てる
        app.sel.cancel_drafts();
    }
    fn key(&self, app: &mut AppState, key: Key, modifiers: Modifiers) -> bool {
        if app.tool != crate::state::Tool::Polygon {
            return false;
        }
        match key {
            Key::Enter => crate::selection::canvas::finish_polygon(app, modifiers),
            Key::Backspace => crate::selection::canvas::remove_last_point(app),
            _ => return false,
        }
        true
    }
    fn respects_stencil(&self) -> bool {
        false
    }
}

// ───────── 文字 ─────────

struct TextInput;

impl CanvasTool for TextInput {
    fn press(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        source: StrokeSource,
        _ctx: &InputCtx,
    ) {
        crate::textlayer::canvas::press(app, view, at, source);
    }
    fn moved(
        &self,
        _app: &mut AppState,
        _view: &CanvasView,
        _at: Pos2,
        _source: StrokeSource,
        _ctx: &InputCtx,
    ) {
    }
    fn release(
        &self,
        app: &mut AppState,
        _view: &CanvasView,
        _at: Pos2,
        source: StrokeSource,
        _ctx: &InputCtx,
    ) {
        crate::textlayer::canvas::release(app, source);
    }
    fn pen(
        &self,
        app: &mut AppState,
        view: &CanvasView,
        at: Pos2,
        _id: u32,
        contact: bool,
        _ctx: &InputCtx,
    ) {
        if contact && app.text.press.is_none() {
            crate::textlayer::canvas::press(app, view, at, StrokeSource::Mouse);
        } else if !contact {
            crate::textlayer::canvas::release(app, StrokeSource::Mouse);
        }
    }
    fn pen_active(&self, app: &AppState, _id: u32) -> bool {
        app.text.press.is_some()
    }
    fn dragging(&self, app: &AppState, source: Option<StrokeSource>) -> bool {
        crate::textlayer::canvas::dragging(app, source)
    }
    fn cancel(&self, app: &mut AppState, _ctx: &InputCtx) -> bool {
        // 押しの途中だけ（打っている文字の Esc は入力欄が受けて、打ち終わる）
        app.text.press.take().is_some()
    }
    fn cancel_first(&self) -> bool {
        false
    }
    fn focus_lost(&self, app: &mut AppState) {
        app.text.press = None;
    }
}

// ───────── 3D ビューの面 ─────────

/// 3D ビューで面を押したときの行き先。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// 面にストロークで描く（ブラシ・消しゴム）。
    Paint,
    /// 押した面の値を取る（スポイト）。
    Pick,
    /// 押した面の範囲を使う（バケツ・ポリゴン塗りつぶし・ID の色で選択）。
    Region,
    /// 面に点を置く・掴む（パス）。
    Path,
    /// 面のダブの覆いを選択範囲に積む（選択ペン・選択消し。`view3d::quick` の、クイックマスクのブラシと同じ道）。
    Cover,
    /// 2D のキャンバスだけのツール（移動と変形・ゆがみ・テキスト）。
    Unsupported,
    /// 画面の上で引いて、見えている面へ写す（グラデーション・図形・定規は描き、長方形選択・楕円形選択・なげなわ・多角形選択・自動選択は選択範囲にする。
    /// `view3d::draft`・`view3d::select`）。
    Screen,
}

// ───────── ポインタの形 ─────────

/// キャンバスの上のポインタの形の決め方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    /// ブラシの大きさの輪（小さいときは十字）。
    Brush,
    /// 十字。
    Crosshair,
    /// 動かすものの上の形（移動・変形とゆがみ）。
    Transform,
    /// 点の上は掴む手、曲線の上は足す形（パス）。
    Path,
    /// 文字を打つ形（文字）。
    Text,
}

impl Cursor {
    /// 表が決める形（`None` はブラシの輪を描く）。
    pub fn icon(
        self,
        app: &mut AppState,
        view: &CanvasView,
        hover: Option<Pos2>,
    ) -> Option<CursorIcon> {
        match self {
            Cursor::Brush => None,
            Cursor::Crosshair => Some(CursorIcon::Crosshair),
            Cursor::Transform => Some(crate::transform::canvas::cursor(app, view, hover)),
            Cursor::Path => Some(match hover {
                Some(p) => crate::pathtool::canvas::cursor_icon(app, view, p),
                None => CursorIcon::Crosshair,
            }),
            Cursor::Text => Some(CursorIcon::Text),
        }
    }
}
