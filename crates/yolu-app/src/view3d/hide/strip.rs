//! ハイドのモードの左のツールの帯（UV アイランドを選ぶ・四角で選ぶ、すべて選択・選択を解除、選んだ面を隠す・すべて表示）と、オプション
//! バーの今の選ぶツールのアイコン。ツールチップは名前とキーだけ（キーはハイドのモードの表から引く）。

use egui::{pos2, vec2, Rect, Ui};

use super::{HideAction, HideTool};
use crate::mode::EditorMode;
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets as w;

/// 操作の名前に、ハイドのモードで効くキーを添える。
pub fn named(app: &AppState, name: &str, command: &str) -> String {
    crate::shortcuts::named_with_keys(
        app.lang,
        name,
        &[crate::shortcuts::key_in(command, EditorMode::Hide)],
    )
}

/// 選んだ UV アイランドがあるか（隠す・選択を解除を押せる）。
fn has_selection(app: &AppState) -> bool {
    !app.view3d.view.hide.selected.is_empty()
}

/// 左の帯（`bottom` は使える下の端。`free` は描いていない間で、押せるかを決める）。
pub fn show(ui: &mut Ui, app: &mut AppState, r: Rect, bottom: f32, free: bool) {
    let lang = app.lang;
    let p = ui.painter().clone();
    let button =
        |y: f32| Rect::from_min_size(pos2(r.left() + 5.0, y), vec2(r.width() - 10.0, 32.0));
    let mut y = r.top() + 6.0;
    let mut action = None;
    for tool in HideTool::ALL {
        if button(y).bottom() > bottom {
            return;
        }
        let tool_id = tool.icon().trim_start_matches("tools/");
        if w::tool_button(
            ui,
            button(y),
            tool_id,
            tool.name(lang),
            app.view3d.hiding.tool == tool,
        )
        .clicked()
        {
            action = Some(HideAction::Tool(tool));
        }
        y += 34.0;
    }
    let selecting = has_selection(app);
    let items: [(&str, &str, &str, HideAction, bool); 4] = [
        (
            "select_all",
            lang.t("hide-select-all"),
            "hide.select_all",
            HideAction::SelectAll,
            true,
        ),
        (
            "deselect",
            lang.t("hide-deselect"),
            "hide.deselect",
            HideAction::Deselect,
            selecting,
        ),
        (
            "visibility_off",
            lang.t("hide-hide-selected-faces"),
            "hide.hide_selected",
            HideAction::HideSelected,
            selecting && free,
        ),
        (
            "visibility",
            lang.t("hide-show-all"),
            "hide.show_all",
            HideAction::ShowAll,
            free,
        ),
    ];
    for (k, (icon, name, command, act, enabled)) in items.into_iter().enumerate() {
        if k % 2 == 0 {
            y += 4.0;
            w::hline(&p, r.left() + 8.0, r.right() - 8.0, y - 3.0, t::SEPARATOR);
        }
        if button(y).bottom() > bottom {
            break;
        }
        let tip = named(app, name, command);
        if w::icon_button(
            ui,
            button(y),
            ("hide.strip", k),
            icon,
            &tip,
            false,
            enabled,
            20.0,
        )
        .clicked()
        {
            action = Some(act);
        }
        y += 34.0;
    }
    if let Some(a) = action {
        app.apply(Action::Hide(a));
    }
}

/// オプションバー（今の選ぶツールのアイコン）。
pub fn options(ui: &mut Ui, app: &AppState, bar: Rect, x: f32) {
    let p = ui.painter();
    w::icon(
        p,
        Rect::from_min_size(pos2(x, bar.top()), vec2(22.0, bar.height())),
        app.view3d.hiding.tool.icon(),
        t::TEXT,
        20.0,
    );
}
