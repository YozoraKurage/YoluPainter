//! 選択範囲のメニュー（メニューバーの「選択範囲」）と、対称のモードのポップアップ（オプションバーの ▾）。項目は `Action` を返し、
//! 選ばれたあとに閉じてから当てるのは `YoluApp`（ほかのメニューと同じ）。

use super::saved::SavedOp;
use super::symmetry::{mode_name, MODES};
use super::{ModifyKind, SelAction, SelEdit, SelUiOp, SymOp};
use crate::state::{Action, AppState, Tool};
use crate::ui::menu::Entry;

/// 選択の道具（メニューとツールの帯の順）。
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
            .shortcut("Ctrl+A")
            .enabled(free),
        Entry::item(l.pick("選択を解除", "Deselect"), edit(SelEdit::Clear))
            .shortcut("Ctrl+D")
            .enabled(free && any),
        Entry::item(
            l.pick("選択範囲を反転", "Invert Selection"),
            edit(SelEdit::Invert),
        )
        .shortcut("Ctrl+Shift+I")
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
    // 選択範囲を使う操作（選択範囲の下のボタンの帯と同じ。塗る・コピーできる層でなければ押せない）
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
        .shortcut("Delete")
        .enabled(paintable),
    );
    v.push(
        Entry::item(
            l.pick("コピーして新しいレイヤーに", "Copy to a New Layer"),
            edit(SelEdit::ToNewLayer),
        )
        .shortcut("Ctrl+J")
        .enabled(copyable),
    );
    v.push(
        Entry::item(
            l.pick("選択範囲をレイヤーマスクにする", "Make the Selection a Layer Mask"),
            edit(SelEdit::ToMask),
        )
        .enabled(free && any && app.selected_layer.is_some()),
    );
    v.push(
        Entry::item(
            l.pick("クイックマスク", "Quick Mask"),
            Action::Sel(SelAction::Ui(SelUiOp::QuickMask(None))),
        )
        .shortcut("Shift+Q")
        .checked(app.sel.quick)
        .enabled(app.sel.quick || !app.is_stroking()),
    );
    // 覚えた選択範囲の窓（保存も呼び出しもここ）。選択範囲も覚えたものも無ければ開く意味が無い
    v.push(
        Entry::item(
            format!("{}…", super::saved::window_title(l)),
            Action::Sel(SelAction::Saved(SavedOp::OpenWindow)),
        )
        .enabled(any || !app.saved_selections().is_empty()),
    );
    v.push(
        Entry::item(
            l.pick("選択範囲のボタンの帯を表示", "Show the Selection Button Bar"),
            Action::Sel(SelAction::Ui(SelUiOp::Bar(!app.prefs.settings.selection_bar))),
        )
        .checked(app.prefs.settings.selection_bar),
    );
    v.push(Entry::Separator);
    for tool in SELECT_TOOLS {
        v.push(
            Entry::item(tool.name_in(l), Action::SelectTool(tool))
                .shortcut(tool.key())
                .radio(app.tool == tool),
        );
    }
    // ID の色で選択（範囲の道具。焼いた ID マップの色から選択範囲を作る）も選ぶ道具に並べる
    v.push(
        Entry::item(
            Tool::IdSelect.name_in(l),
            Action::SelectTool(Tool::IdSelect),
        )
        .shortcut(Tool::IdSelect.key())
        .radio(app.tool == Tool::IdSelect),
    );
    v
}

/// 対称のモードのポップアップの中身（オプションバーの ▾）。
pub fn symmetry_menu(app: &AppState) -> Vec<Entry<Action>> {
    let l = app.lang;
    let free = !app.is_stroking();
    let sym = &app.sel.symmetry;
    let mut v: Vec<Entry<Action>> = MODES
        .iter()
        .map(|m| {
            Entry::item(
                mode_name(l, *m),
                Action::Sel(SelAction::Symmetry(SymOp::Mode(*m))),
            )
            .radio(sym.mode == *m)
            .enabled(free)
        })
        .collect();
    v.push(Entry::Separator);
    v.push(
        Entry::item(
            l.pick("軸を表示", "Show Axes"),
            Action::Sel(SelAction::Symmetry(SymOp::ShowAxes(!sym.show_axes))),
        )
        .checked(sym.show_axes),
    );
    v
}
