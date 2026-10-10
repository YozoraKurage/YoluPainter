//! 選択範囲のメニュー（メニューバーの「選択範囲」）。項目は `Action` を返し、
//! 選ばれたあとに閉じてから当てるのは `YoluApp`（ほかのメニューと同じ）。

use super::saved::SavedOp;
use super::{ModifyKind, SelAction, SelEdit, SelUiOp};
use crate::state::{Action, AppState, Tool};
use crate::ui::menu::Entry;

/// 選択のツール（メニューとツールの帯の順）。
pub const SELECT_TOOLS: [Tool; 6] = [
    Tool::SelectRect,
    Tool::SelectEllipse,
    Tool::Lasso,
    Tool::Polygon,
    Tool::Wand,
    Tool::SelectPen,
];

/// メニューバーの「選択範囲」の中身。
pub fn select_menu(app: &AppState) -> Vec<Entry<Action>> {
    let l = app.lang;
    let free = !app.is_stroking() && app.read_only_reason().is_none();
    let any = app.doc.selection().is_some();
    let edit = |e: SelEdit| Action::Sel(SelAction::Edit(e));
    let mut v = vec![
        Entry::item(l.pick("すべてを選択", "Select All"), edit(SelEdit::All))
            .command_key("selection.all")
            .enabled(free),
        Entry::item(l.pick("選択を解除", "Deselect"), edit(SelEdit::Clear))
            .command_key("selection.deselect")
            .enabled(free && any),
        Entry::item(
            l.pick("選択範囲を反転", "Invert Selection"),
            edit(SelEdit::Invert),
        )
        .command_key("selection.invert")
        .enabled(free && any),
        Entry::Separator,
    ];
    for kind in ModifyKind::ALL {
        let item = if kind.uses_radius() {
            Entry::item(
                format!("{}…", kind.name(l)),
                Action::Sel(SelAction::Ui(SelUiOp::OpenAmount(kind))),
            )
        } else {
            Entry::item(
                kind.name(l),
                edit(SelEdit::Modify {
                    kind,
                    radius: 0,
                    edge_lock: false,
                }),
            )
        };
        v.push(item.enabled(free && any));
    }
    v.push(Entry::Separator);
    // 選択範囲を使う操作（選択範囲の下のボタンの帯と同じ。塗る・コピーできるレイヤーでなければ押せない）
    let paintable = free && any && app.paint_blocker().is_none();
    let copyable = free
        && any
        && app
            .selected_layer
            .and_then(|id| app.doc.layer(id))
            .is_some_and(|layer| {
                (app.m2.edit_mask && layer.mask().is_some())
                    || matches!(
                        layer.kind(),
                        crate::engine::LayerKind::Raster | crate::engine::LayerKind::Fill
                    )
            });
    v.push(
        Entry::item(
            l.pick("描画色で塗りつぶす", "Fill with the Paint Color"),
            edit(SelEdit::Fill),
        )
        .enabled(paintable),
    );
    v.push(
        Entry::item(
            l.pick("選択範囲を消去", "Erase Selection"),
            edit(SelEdit::Erase),
        )
        .command_key("selection.erase")
        .enabled(paintable),
    );
    v.push(
        Entry::item(
            l.pick("コピーして新しいレイヤーに", "Copy to a New Layer"),
            edit(SelEdit::ToNewLayer),
        )
        .command_key("selection.to_new_layer")
        .enabled(copyable),
    );
    v.push(
        Entry::item(
            l.pick(
                "選択範囲をレイヤーマスクにする",
                "Make the Selection a Layer Mask",
            ),
            edit(SelEdit::ToMask),
        )
        .enabled(free && any && app.selected_layer.is_some()),
    );
    v.push(
        Entry::item(
            l.pick("クイックマスク", "Quick Mask"),
            Action::Sel(SelAction::Ui(SelUiOp::QuickMask(None))),
        )
        .command_key("selection.quick_mask")
        .checked(app.sel.quick)
        .enabled(app.sel.quick || !app.is_stroking()),
    );
    // 覚えた選択範囲のウィンドウ（保存も呼び出しもここ）。選択範囲も覚えたものも無ければ開く意味が無い
    v.push(
        Entry::item(
            format!("{}…", super::saved::window_title(l)),
            Action::Sel(SelAction::Saved(SavedOp::OpenWindow)),
        )
        .enabled(any || !app.saved_selections().is_empty()),
    );
    v.push(
        Entry::item(
            l.pick(
                "選択範囲のボタンの帯を表示",
                "Show the Selection Button Bar",
            ),
            Action::Sel(SelAction::Ui(SelUiOp::Bar(
                !app.prefs.settings.selection_bar,
            ))),
        )
        .checked(app.prefs.settings.selection_bar),
    );
    v.push(Entry::Separator);
    for tool in SELECT_TOOLS {
        v.push(
            Entry::item(tool.name_in(l), Action::SelectTool(tool))
                .command_key(crate::commands::tool_command(tool))
                .radio(app.tool == tool),
        );
    }
    // ID の色で選択（範囲のツール。焼いた ID マップの色から選択範囲を作る）も選ぶツールに並べる
    v.push(
        Entry::item(
            Tool::IdSelect.name_in(l),
            Action::SelectTool(Tool::IdSelect),
        )
        .command_key(crate::commands::tool_command(Tool::IdSelect))
        .radio(app.tool == Tool::IdSelect),
    );
    v
}
