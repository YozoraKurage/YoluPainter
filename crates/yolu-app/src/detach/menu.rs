//! メニューの「ウィンドウ」と、ドックのタブの右クリックの項目。

use super::DockOp;
use crate::layout::HIDEABLE;
use crate::state::{Action, AppState};
use crate::ui::menu::Entry;
use crate::Tab;

/// 「ウィンドウ」に並べるパネル（上から）。名前はタブの名前と同じ。
pub const PANELS: [Tab; 17] = [
    Tab::SubTools,
    Tab::ToolProperties,
    Tab::BrushSize,
    Tab::Assets,
    Tab::Channels,
    Tab::Color,
    Tab::ColorSets,
    Tab::TextureSets,
    Tab::Navigator,
    Tab::Layers,
    Tab::Log,
    Tab::Actions,
    Tab::Properties,
    Tab::Material,
    Tab::History,
    Tab::Canvas,
    Tab::View3d,
];

/// 「ウィンドウ」の中身: パネルの一覧（開いているパネルにチェック。押すと前に出す・閉じていれば開く。`HIDEABLE` のナビゲーターとアクションは、開いていれば
/// 押すと閉じる）と、パネルの並びを戻す。
pub fn window_entries(app: &AppState) -> Vec<Entry<Action>> {
    let l = app.lang;
    let mut entries: Vec<Entry<Action>> = PANELS
        .iter()
        .map(|&tab| {
            let open = app.ui.panels.open.contains(&tab);
            let op = if open && HIDEABLE.contains(&tab) {
                DockOp::Hide(tab)
            } else {
                DockOp::Show(tab)
            };
            Entry::item(tab.title_in(l), Action::Dock(op)).checked(open)
        })
        .collect();
    entries.push(Entry::Separator);
    entries.push(Entry::item(
        l.pick("パネルの並びを戻す", "Reset Panel Layout"),
        Action::ResetLayout,
    ));
    entries
}

/// タブの右クリック: メインウィンドウのタブは「別ウィンドウで開く」。別ウィンドウのタブは「ドックに戻す」と、ウィンドウにほかのタブもあれば「別ウィンドウで開く」。
pub fn tab_entries(app: &AppState, tab: Tab) -> Vec<Entry<Action>> {
    let l = app.lang;
    let open = Entry::item(
        l.pick("別ウィンドウで開く", "Open in New Window"),
        Action::Dock(DockOp::Detach(tab)),
    );
    if !app.ui.panels.outside.contains(&tab) {
        return vec![open];
    }
    let back = Entry::item(
        l.pick("ドックに戻す", "Return to Dock"),
        Action::Dock(DockOp::Return(tab)),
    );
    if app.ui.panels.alone.contains(&tab) {
        vec![back]
    } else {
        vec![open, back]
    }
}
