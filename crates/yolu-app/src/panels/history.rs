//! 現在のテクスチャセットの履歴。位置は保持された最古の段の直前を 0 とする。

use crate::lang::Lang;
use crate::{
    engine::HistoryKind,
    state::{Action, AppState},
};

pub fn title(kind: HistoryKind, lang: Lang) -> &'static str {
    match kind {
        HistoryKind::Other => lang.t("panels-other-2"),
        HistoryKind::Brush => lang.t("panels-brush-stroke"),
        HistoryKind::Pixels => lang.t("panels-edit-pixels"),
        HistoryKind::AddLayer => lang.t("panels-add-layer"),
        HistoryKind::RemoveLayer => lang.t("panels-remove-layer"),
        HistoryKind::LayerOrder => lang.t("panels-arrange-layers"),
        HistoryKind::LayerProperties => lang.t("panels-edit-layer"),
        HistoryKind::Channel => lang.t("panels-edit-channel"),
        HistoryKind::Fill => lang.t("panels-edit-fill"),
        HistoryKind::AddMask => lang.t("panels-add-mask"),
        HistoryKind::RemoveMask => lang.t("panels-remove-mask"),
        HistoryKind::Mask => lang.t("panels-edit-mask"),
        HistoryKind::Selection => lang.t("panels-change-selection"),
        HistoryKind::AddEffect => lang.t("panels-add-effect"),
        HistoryKind::RemoveEffect => lang.t("panels-remove-effect"),
        HistoryKind::Effect => lang.t("panels-edit-effect"),
        HistoryKind::Anchor => lang.t("panels-edit-anchor"),
        HistoryKind::Path => lang.t("panels-edit-path"),
        HistoryKind::Batch => lang.t("panels-batch-edit"),
        HistoryKind::Look => lang.t("panels-edit-look"),
        HistoryKind::SavedSelections => lang.t("panels-edit-remembered-selections"),
        HistoryKind::Text => lang.t("panels-edit-text"),
        HistoryKind::Rulers => lang.t("panels-edit-ruler"),
        HistoryKind::MoveMask => lang.t("panels-move-mask"),
        HistoryKind::PasteMask => lang.t("panels-paste-mask"),
    }
}

/// ポーズや未確定の多角形の Undo と区別して、文書の保持された位置へ移動する（`Action::HistoryGoTo`）。
pub fn go_to(app: &mut AppState, target: usize) {
    app.apply(Action::HistoryGoTo(target));
}

pub fn show(ui: &mut egui::Ui, app: &mut AppState) {
    let current = app.session.doc.undo_count();
    let count = current + app.session.doc.redo_count();
    // 空の履歴には案内文や初期状態の行を置かない。
    if count == 0 {
        return;
    }
    let mut target = None;
    let row_height = ui.spacing().interact_size.y;
    let area = egui::ScrollArea::vertical().show_rows(ui, row_height, count + 1, |ui, rows| {
        // 描いている間も、行は描き始める前の見た目のまま（押せないことは、下で本当の `can_edit` で守る）
        let can_edit = app.can_edit();
        let shown =
            crate::ui::widgets::look_enabled(ui, ui.make_persistent_id("history.rows"), can_edit);
        ui.add_enabled_ui(shown, |ui| {
            // スライスのイテレーターで先頭を飛ばし、可視範囲だけ読む。全段のコピーは作らない。
            let mut kinds = app.session.doc.history().skip(rows.start.saturating_sub(1));
            for position in rows {
                let label = if position == 0 {
                    app.lang.t("panels-starting-point")
                } else {
                    title(kinds.next().expect("保持された履歴の範囲"), app.lang)
                };
                let text = egui::RichText::new(label);
                let text = if position > current {
                    text.weak()
                } else {
                    text
                };
                // 狭いドックでも折り返さず、show_rows の一行の高さを保つ。
                if ui
                    .add_sized(
                        [ui.available_width(), row_height],
                        egui::Button::selectable(position == current, text).truncate(),
                    )
                    .clicked()
                    && can_edit
                {
                    target = Some(position);
                }
            }
        });
    });
    crate::ui::scroll::pan_area(ui, &area);
    if let Some(position) = target {
        go_to(app, position);
    }
}
