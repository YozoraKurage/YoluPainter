//! クイックマスク（Photoshop のクイックマスク）。文書の選択範囲そのものを、赤い半透明の重ねとして見せる表示の切り替えで、別のマスクや
//! 一時レイヤーを持たない。入っている間、ブラシは選択ペン・消しゴムは選択消しとして働き（直径・硬さ・不透明度・筆圧は今のブラシのもの）、
//! 1 ストロークが 1 回の Undo（`SelEdit::Shape`）。切ると縁の点線の表示に戻り、選択範囲はそのまま残る。取り消し・やり直しも文書のものが
//! そのまま効く。縁の点線は入っている間は出さない（赤の重ねが選択範囲）。
//!
//! キャンバスの入力（`canvas/mod.rs`）が、ブラシ・消しゴムのストロークの始め・点・終わりで `begin`・`add_point`・`finish` を呼ぶ。
//! クイックマスクが入っていなければ何もせず、これまでの描き方のまま。

use egui::{Painter, Pos2};

use super::overlay::Tint;
use super::pen;
use crate::canvas::view::CanvasView;
use crate::engine::SelectionMask;
use crate::notice::Source;
use crate::state::{AppState, StrokeSource};

/// 赤い重ね（満量の画素の濃さ）。
pub const TINT: Tint = Tint {
    rgb: [255, 48, 48],
    alpha: 0.5,
};

impl AppState {
    /// クイックマスクを入れる・切る（None は切り替え）。描いている間は断る。
    pub fn quick_mask(&mut self, on: Option<bool>) {
        let want = on.unwrap_or(!self.sel.quick);
        if want == self.sel.quick {
            return;
        }
        if self.is_stroking() {
            self.refuse(
                Source::Selection,
                crate::lang::refusals::during_stroke(self.lang),
            );
            return;
        }
        self.sel.quick = want;
        if !want {
            self.sel.quick_overlay.clear();
        }
        self.info(
            Source::Selection,
            if want {
                self.lang.pick("クイックマスク", "Quick Mask")
            } else {
                self.lang
                    .pick("クイックマスクを終えました。", "Quick Mask off.")
            },
        );
    }
}

/// ブラシ・消しゴムのストロークの始め。クイックマスクが入っていなければ None（これまでどおり描く）、入っていれば始められたか。
pub fn begin(app: &mut AppState, source: StrokeSource, eraser: bool) -> Option<bool> {
    if !app.sel.quick {
        return None;
    }
    // ほかのビューで選択ペンのストロークが動いている間は始めない（被覆を上書きしない）
    if app.sel.pen.is_some() {
        app.refuse(
            Source::Selection,
            crate::lang::refusals::during_stroke(app.lang),
        );
        return Some(false);
    }
    let erase = eraser || app.tool.erases();
    if !pen::begin(app, source, erase, true) {
        return Some(false);
    }
    app.canvas.stroke = Some(source);
    app.canvas.stroke_points = 0;
    app.canvas.stroke_time = None;
    Some(true)
}

/// ストロークの点。クイックマスクのストロークなら受け取って true（描くストロークには渡さない）。
pub fn add_point(app: &mut AppState, view: &CanvasView, pos: Pos2, pressure: f32) -> bool {
    if !app.sel.pen.as_ref().is_some_and(|a| a.quick) {
        return false;
    }
    if pen::add_point(app, view, pos, pressure) {
        app.canvas.stroke_points += 1;
    }
    true
}

/// ストロークの終わり（cancel なら捨てる）。クイックマスクのストロークなら終えて true。
pub fn finish(app: &mut AppState, cancel: bool) -> bool {
    if !app.sel.pen.as_ref().is_some_and(|a| a.quick) {
        return false;
    }
    pen::finish(app, cancel);
    true
}

/// 赤い重ねを描く（描いている途中は、その見た目）。選択範囲が空なら何も重ねない。
pub fn paint(painter: &Painter, view: &CanvasView, app: &mut AppState) {
    pen::sync(app);
    let live = app.sel.pen.as_ref().filter(|a| a.quick);
    let mask = live
        .and_then(|a| a.stroke.preview().cloned())
        .or_else(|| app.doc.selection().cloned())
        .unwrap_or_else(|| SelectionMask::none(&app.doc));
    // 変わったタイルは、ここで受け取って空にする（3D ビューも `sync` を呼ぶので、先に呼んだほうの分も溜まっている）
    let hint = app
        .sel
        .pen
        .as_mut()
        .filter(|a| a.quick)
        .and_then(|a| a.stroke.take_synced());
    app.sel
        .quick_overlay
        .paint(painter, view, &mask, TINT, hint.as_deref(), "quick-mask");
}
