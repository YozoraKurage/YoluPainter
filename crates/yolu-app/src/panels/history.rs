//! 現在のテクスチャセットの履歴。位置は保持された最古の段の直前を 0 とする。

use crate::lang::{refusals, Lang};
use crate::notice::{Kind, Source};
use crate::{engine::HistoryKind, state::AppState};

pub fn title(kind: HistoryKind, lang: Lang) -> &'static str {
    match kind {
        HistoryKind::Other => lang.pick("その他", "Other"),
        HistoryKind::Brush => lang.pick("ブラシで描く", "Brush stroke"),
        HistoryKind::Pixels => lang.pick("画素を変える", "Edit pixels"),
        HistoryKind::AddLayer => lang.pick("レイヤーを追加", "Add layer"),
        HistoryKind::RemoveLayer => lang.pick("レイヤーを消す", "Remove layer"),
        HistoryKind::LayerOrder => lang.pick("レイヤーを並べ替える", "Arrange layers"),
        HistoryKind::LayerProperties => lang.pick("レイヤーを変える", "Edit layer"),
        HistoryKind::Channel => lang.pick("チャンネルを変える", "Edit channel"),
        HistoryKind::Fill => lang.pick("塗りつぶしを変える", "Edit fill"),
        HistoryKind::AddMask => lang.pick("マスクを追加", "Add mask"),
        HistoryKind::RemoveMask => lang.pick("マスクを消す", "Remove mask"),
        HistoryKind::Mask => lang.pick("マスクを変える", "Edit mask"),
        HistoryKind::Selection => lang.pick("選択範囲を変える", "Change selection"),
        HistoryKind::AddEffect => lang.pick("効果を追加", "Add effect"),
        HistoryKind::RemoveEffect => lang.pick("効果を消す", "Remove effect"),
        HistoryKind::Effect => lang.pick("効果を変える", "Edit effect"),
        HistoryKind::Anchor => lang.pick("アンカーを変える", "Edit anchor"),
        HistoryKind::Path => lang.pick("パスを変える", "Edit path"),
        HistoryKind::Batch => lang.pick("まとめて編集", "Batch edit"),
        HistoryKind::Look => lang.pick("見た目を変える", "Edit look"),
        HistoryKind::SavedSelections => {
            lang.pick("覚えた選択範囲を変える", "Edit remembered selections")
        }
        HistoryKind::Text => lang.pick("テキストを変える", "Edit Text"),
        HistoryKind::Rulers => lang.pick("定規を変える", "Edit Ruler"),
    }
}

/// ポーズや未確定の多角形の Undo と区別して、文書の保持された位置へ移動する。
pub fn go_to(app: &mut AppState, target: usize) {
    if app.is_stroking() {
        app.refuse(Source::Edit, refusals::during_stroke(app.lang));
        return;
    }
    if let Some(reason) = app.read_only_reason() {
        let text = refusals::read_only_set(app.lang, reason);
        app.refuse(Source::Edit, text);
        return;
    }
    if target > app.doc.undo_count() + app.doc.redo_count() {
        return;
    }
    while target != app.doc.undo_count() {
        let result = if target < app.doc.undo_count() {
            app.doc.undo()
        } else {
            app.doc.redo()
        };
        match result {
            Ok(true) => app.modified = true,
            Ok(false) => break,
            Err(e) => {
                app.notify(Kind::of_core(&e), Source::Edit, app.lang.core_error(&e));
                break;
            }
        }
    }
    app.ensure_selection();
}

pub fn show(ui: &mut egui::Ui, app: &mut AppState) {
    let current = app.doc.undo_count();
    let count = current + app.doc.redo_count();
    // 空の履歴には案内文や初期状態の行を置かない。
    if count == 0 {
        return;
    }
    let mut target = None;
    let row_height = ui.spacing().interact_size.y;
    egui::ScrollArea::vertical().show_rows(ui, row_height, count + 1, |ui, rows| {
        // 描いている間も、行は描き始める前の見た目のまま（押せないことは、下で本当の `can_edit` で守る）
        let can_edit = app.can_edit();
        let shown =
            crate::ui::widgets::look_enabled(ui, ui.make_persistent_id("history.rows"), can_edit);
        ui.add_enabled_ui(shown, |ui| {
            // スライスのイテレーターで先頭を飛ばし、可視範囲だけ読む。全段のコピーは作らない。
            let mut kinds = app.doc.history().skip(rows.start.saturating_sub(1));
            for position in rows {
                let label = if position == 0 {
                    app.lang.pick("開始位置", "Starting point")
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
    if let Some(position) = target {
        go_to(app, position);
    }
}
