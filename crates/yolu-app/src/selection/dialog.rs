//! 量を聞く小さな窓（拡張・縮小・境界線・ぼかしの半径と、画布の縁を固定するか）。モーダルで、見出しをドラッグして動かせる
//! （窓の骨組みはほかの浮いた窓と同じ `ui::window`）。Enter で適用・Esc で取り消し（文字を打っている間は、その欄に任せる）。
//! 適用は `Action::Sel`（1 回の Undo）を通す。

use egui::{pos2, vec2, Id, Key, Rect};

use super::{SelAction, SelUiOp};
use crate::engine::MAX_MODIFY_RADIUS;
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, SliderSpec};
use crate::ui::window::{self, Spec};

const WIDTH: f32 = 300.0;

fn window_id() -> Id {
    Id::new("yolu.sel-amount")
}

/// 最後に描いた窓の矩形（画面の点。開いていなければ None）。試験が位置を知るために読む。
pub fn last_rect(ctx: &egui::Context) -> Option<Rect> {
    window::last_rect(ctx, window_id())
}

/// 開いていれば窓を描き、押されたものを `Action` として当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    super::saved::show_window(ctx, app);
    let Some(mut dialog) = app.sel.dialog else {
        return;
    };
    let lang = app.lang;
    let has_lock = dialog.kind.uses_edge_lock();
    let height = window::HEADER_HEIGHT
        + 14.0
        + t::SLIDER_ROW_HEIGHT
        + if has_lock { 30.0 } else { 4.0 }
        + 14.0
        + 28.0
        + 14.0;
    // 文字を打っている間のキーは、その欄に任せる
    let keys_free = !ctx.egui_wants_keyboard_input();
    let (enter, esc) = if keys_free {
        ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::Enter),
                i.consume_key(egui::Modifiers::NONE, Key::Escape),
            )
        })
    } else {
        (false, false)
    };
    let spec = Spec {
        title: dialog.kind.name(lang),
        icon: Some("select_all"),
        size: vec2(WIDTH, height),
        modal: true,
        close_label: lang.pick("閉じる", "Close"),
    };
    let id = window_id();
    let mut offset = app.sel.dialog_offset;
    let mut apply = enter;
    let mut cancel = false;
    let closed = window::show(ctx, id, &spec, &mut offset, esc, |ui, frame| {
        let left = frame.body.left() + 14.0;
        let width = frame.body.width() - 28.0;
        let mut y = frame.body.top() + 14.0;
        let slider_rect = Rect::from_min_size(pos2(left, y), vec2(width, t::SLIDER_ROW_HEIGHT));
        let slider = SliderSpec::new(
            lang.pick("半径", "Radius"),
            0.0,
            MAX_MODIFY_RADIUS as f32,
            NumberFormat::int(" px"),
        )
        .tooltip(dialog.kind.tooltip(lang));
        let out = w::slider(
            ui,
            slider_rect,
            id.with("radius"),
            dialog.radius as f32,
            &slider,
        );
        if out.changed {
            dialog.radius = out.value.round().clamp(0.0, MAX_MODIFY_RADIUS as f32) as u32;
        }
        y += t::SLIDER_ROW_HEIGHT + 4.0;
        if has_lock {
            let row = Rect::from_min_size(pos2(left, y), vec2(width, 22.0));
            dialog.edge_lock = w::toggle(
                ui,
                row,
                id.with("edge-lock"),
                lang.pick("端を固定", "Edge lock"),
                dialog.edge_lock,
                Some(lang.pick(
                    "選択範囲が画布の外へ続くものとして扱う（縮小・境界線・ぼかしが画布の端から離れない）",
                    "Treat the selection as continuing past the canvas edge (Shrink, Border and Feather do not pull away from it)",
                )),
                true,
            );
        }
        let buttons_top = frame.rect.bottom() - 14.0 - 28.0;
        let cancel_rect =
            Rect::from_min_size(pos2(left + width - 96.0, buttons_top), vec2(96.0, 28.0));
        let ok_rect = Rect::from_min_size(
            pos2(cancel_rect.left() - 8.0 - 96.0, buttons_top),
            vec2(96.0, 28.0),
        );
        if w::button(ui, ok_rect, id.with("ok"), "OK", true, true, None, None).clicked() {
            apply = true;
        }
        if w::button(
            ui,
            cancel_rect,
            id.with("cancel"),
            lang.pick("キャンセル", "Cancel"),
            false,
            true,
            None,
            None,
        )
        .clicked()
        {
            cancel = true;
        }
    });
    app.sel.dialog_offset = offset;
    app.sel.dialog = Some(dialog);
    if apply {
        app.apply(Action::Sel(SelAction::Ui(SelUiOp::ApplyAmount)));
    } else if closed || cancel {
        app.apply(Action::Sel(SelAction::Ui(SelUiOp::CancelAmount)));
    }
}
