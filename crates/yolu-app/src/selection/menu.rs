//! 選択範囲のメニュー（メニューバーの「選択範囲」）と、対称のモードのポップアップ（オプションバーの ▾）。項目は `Action` を返し、
//! 選ばれたあとに閉じてから当てるのは `YoluApp`（ほかのメニューと同じ）。

use super::symmetry::{mode_name, MODES};
use super::{ModifyKind, SelAction, SelEdit, SelUiOp, SymOp};
use crate::state::{Action, AppState, Tool};
use crate::ui::menu::Entry;

/// 選択の道具（メニューとツールの帯の順）。
pub const SELECT_TOOLS: [Tool; 5] = [
    Tool::SelectRect,
    Tool::SelectEllipse,
    Tool::Lasso,
    Tool::Polygon,
    Tool::Wand,
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
    for tool in SELECT_TOOLS {
        v.push(
            Entry::item(tool.name_in(l), Action::SelectTool(tool))
                .shortcut(tool.key())
                .radio(app.tool == tool),
        );
    }
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
