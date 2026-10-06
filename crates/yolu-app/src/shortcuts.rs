//! キーの一覧の窓（読むだけ）。割り当ては `keymap` の表で、一覧と実際のキーの処理は同じ表を読む。名前はメニューと共有する。
pub mod gestures;

use crate::{
    keymap::{self, When},
    lang::Lang,
    m2::Edit,
    pathtool::PathAction,
    selection::{SelAction, SelEdit},
    state::{Action, AppState},
    ui::menu::Entry,
    windows::{ListSpec, Row},
};
use egui::{Key, Vec2};

#[derive(Default)]
pub struct ShortcutWindow {
    pub open: bool,
    offset: Vec2,
    scroll: f32,
}

/// キーの割り当て 1 つ（`keymap` の表のもの）。
pub type Binding = keymap::KeyBinding;

/// 一覧に出すキーの割り当て（この環境で効くもの。Windows だけのキーは Windows でだけ）。
pub fn bindings() -> Vec<Binding> {
    keymap::bindings()
        .into_iter()
        .filter(|b| b.when != When::Windows || cfg!(windows))
        .collect()
}

/// 移動・変形の道具の矢印キー。
pub fn movement_keys() -> [Key; 4] {
    keymap::MOVE_KEYS.map(|(key, _)| key)
}

/// 「表示を右に回す」のもう 1 つの割り当て（文字で見る）。
pub fn rotation_text() -> &'static str {
    keymap::ROTATE_RIGHT_TEXT
}

pub fn menu_entry(lang: Lang) -> Entry<Action> {
    Entry::item(
        lang.pick("ショートカット", "Keyboard Shortcuts"),
        Action::ShowShortcuts,
    )
}

fn key_name(key: Key) -> &'static str {
    match key {
        Key::Num0 => "0",
        Key::Num4 => "4",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::ArrowLeft => "←",
        Key::ArrowRight => "→",
        Key::ArrowUp => "↑",
        Key::ArrowDown => "↓",
        Key::Comma => ",",
        Key::Plus => "+",
        Key::Equals => "=",
        Key::Minus => "-",
        _ => key.name(),
    }
}
pub fn key_label(binding: &Binding) -> String {
    let m = binding.modifiers;
    let mut text = String::new();
    if m.command {
        text.push_str(if cfg!(target_os = "macos") {
            "Cmd+"
        } else {
            "Ctrl+"
        });
    } else if m.ctrl {
        // Control そのもの（Windows の画面から色を取るキーは Ctrl+Alt+I。`command` を立てずに `ctrl` だけで書く）
        text.push_str("Ctrl+");
    }
    if m.alt {
        text.push_str("Alt+");
    }
    if m.shift {
        text.push_str("Shift+");
    }
    text.push_str(key_name(binding.key));
    text
}

/// 操作に割り当てたキーの文字（「Ctrl+A」。表の中でいちばん先に判定される割り当て。無ければ None）。
/// ツールチップに書くキーは、文字を直に書かずここから引く（キーの割り当ては `keymap` の表だけが持つ）。
pub fn shortcut_text(action: &Action) -> Option<String> {
    keymap::dispatch_order()
        .iter()
        .find(|b| b.action == *action)
        .map(key_label)
}

/// ツールチップの名前にキーを添える（「すべてを選択（Ctrl+A）」・英語は「Select All (Ctrl+A)」）。割り当てが無ければ名前だけ。
pub fn tip_with_key(lang: Lang, name: &str, action: &Action) -> String {
    match shortcut_text(action) {
        Some(key) => lang.pick(format!("{name}（{key}）"), format!("{name} ({key})")),
        None => name.to_owned(),
    }
}

/// 名前はメニューを共有する。メニューに無いキーだけここで名前を持つ。
pub fn action_label(app: &AppState, action: &Action) -> Option<String> {
    if let Action::SelectTool(tool) = action {
        return Some(tool.name_in(app.lang).into());
    }
    for index in 0..6 {
        let entries = crate::shell::menu_entries(app, index);
        for entry in crate::ui::menu::leaves(&entries) {
            if let Entry::Item {
                label,
                action: candidate,
                ..
            } = entry
            {
                if candidate == action {
                    return Some(label.clone());
                }
            }
        }
    }
    let l = app.lang;
    if let Action::ScreenPick(mode) = action {
        return Some(mode.label(l).into());
    }
    Some(
        match action {
            Action::M2(Edit::UngroupSelected) => l.pick("グループ解除", "Ungroup"),
            Action::M2(Edit::GroupSelected) => l.pick("レイヤーをグループ化", "Group Layers"),
            Action::M2(Edit::DuplicateSelected) => l.pick("レイヤーを複製", "Duplicate Layer"),
            Action::M2(Edit::MergeDown) => l.pick("レイヤーを結合", "Merge Layers"),
            Action::M2(Edit::MergeVisible) => l.pick("表示レイヤーを結合", "Merge Visible"),
            Action::BrushSmaller => l.pick("ブラシを小さく", "Smaller Brush"),
            Action::BrushLarger => l.pick("ブラシを大きく", "Larger Brush"),
            Action::DefaultColors => l.pick("既定の色", "Default Colors"),
            Action::Fill(crate::fillfx::FillOp::ToggleHandles) => {
                l.pick("投影のハンドル", "Projection Handles")
            }
            Action::Path(PathAction::DeleteSelected) => {
                l.pick("パスの点を削除", "Delete Path Point")
            }
            Action::Sel(SelAction::Edit(SelEdit::ToNewLayer)) => {
                l.pick("選択した画素を新しいレイヤーへ", "Selection to New Layer")
            }
            _ => return None,
        }
        .into(),
    )
}

fn group(action: &Action) -> usize {
    match action {
        Action::SelectTool(_)
        | Action::BrushSmaller
        | Action::BrushLarger
        | Action::SwapColors
        | Action::DefaultColors
        | Action::Path(_) => 0,
        Action::M2(_) | Action::NewLayer => 2,
        Action::Sel(_) => 3,
        Action::FitView
        | Action::ZoomIn
        | Action::ZoomOut
        | Action::ResetRotation
        | Action::FlipView
        | Action::RotateLeft
        | Action::RotateRight
        | Action::Fill(_) => 4,
        Action::SaveProject
        | Action::SaveProjectAsDialog
        | Action::OpenProjectDialog
        | Action::NewProjectDialog
        | Action::Quit => 5,
        _ => 1,
    }
}

pub fn rows(app: &AppState) -> Vec<Row> {
    let l = app.lang;
    let groups = [
        l.pick("道具", "Tools"),
        l.pick("編集", "Edit"),
        l.pick("レイヤー", "Layer"),
        l.pick("選択範囲", "Selection"),
        l.pick("表示", "View"),
        l.pick("ファイル", "File"),
    ];
    let mut all = Vec::new();
    let bindings = bindings();
    for (g, title) in groups.iter().enumerate() {
        all.push(Row::text(*title, false));
        for binding in bindings.iter().filter(|b| group(&b.action) == g) {
            all.push(Row {
                left: action_label(app, &binding.action)
                    .unwrap_or_else(|| l.pick("未登録の操作", "Unlisted Action").into()),
                middle: String::new(),
                right: key_label(binding),
                warning: false,
            });
        }
        if g == 0 {
            let arrows = movement_keys().map(key_name).join(" / ");
            all.push(Row {
                left: l.pick("移動（1 px / 10 px）", "Move (1 px / 10 px)").into(),
                middle: String::new(),
                right: format!("{arrows} / Shift"),
                warning: false,
            });
        }
        if g == 4 {
            all.push(Row {
                left: l.pick("右に回転", "Rotate Right").into(),
                middle: String::new(),
                right: rotation_text().into(),
                warning: false,
            });
        }
    }
    all.extend(context_rows(l));
    all
}

fn context_rows(lang: Lang) -> Vec<Row> {
    let mut rows = Vec::new();
    for (scope, title) in [
        ("canvas", lang.pick("2D ビュー", "2D View")),
        ("view3d", lang.pick("3D ビュー", "3D View")),
        ("stencil", lang.pick("ステンシル", "Stencil")),
    ] {
        rows.push(Row::text(title, false));
        for context in keymap::CONTEXT_KEYS.iter().filter(|c| c.scope == scope) {
            rows.push(Row {
                left: context.label(lang).into(),
                middle: String::new(),
                right: format!(
                    "{}{}",
                    key_name(context.key),
                    if context.mouse {
                        lang.pick(" + マウス", " + Mouse")
                    } else {
                        ""
                    }
                ),
                warning: false,
            });
        }
        rows.extend(gestures::rows(scope, lang));
    }
    rows
}

pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.shortcuts.open {
        return;
    }
    let spec = ListSpec {
        id: "shortcuts",
        title: app.lang.pick("ショートカット", "Keyboard Shortcuts").into(),
        icon: "tune",
        modal: false,
        width: 620.0,
        summary: None,
        rows: rows(app),
        buttons: Vec::new(),
        close_label: app.lang.pick("閉じる", "Close").into(),
    };
    if crate::windows::show_list(
        ctx,
        &spec,
        &mut app.shortcuts.offset,
        &mut app.shortcuts.scroll,
    )
    .is_some()
    {
        app.shortcuts.open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::ClipAction;
    use crate::state::Tool;
    use egui::{Event, Modifiers};

    /// 1 つのキーの押しを、実際のキーの処理（`shell::handle_shortcuts`）へ流す。
    fn press(app: &mut AppState, key: Key, modifiers: Modifiers) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            events: vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| crate::shell::handle_shortcuts(ui.ctx(), app));
        output.textures_delta.clear();
    }

    #[test]
    fn every_binding_has_a_localized_name_and_every_context_key_a_label() {
        let mut app = AppState::new(32, 32);
        for lang in Lang::ALL {
            app.lang = lang;
            app.selected_layer = None;
            for binding in bindings() {
                let label = action_label(&app, &binding.action)
                    .unwrap_or_else(|| panic!("名前がありません: {:?}", binding.action));
                assert!(!label.is_empty());
                if lang == Lang::En {
                    assert!(
                        !label
                            .chars()
                            .any(|c| ('\u{3000}'..='\u{9fff}').contains(&c)),
                        "{label}"
                    );
                }
            }
            for context in keymap::CONTEXT_KEYS {
                assert!(!context.label(lang).is_empty());
                if lang == Lang::En {
                    assert!(!context
                        .label(lang)
                        .chars()
                        .any(|c| ('\u{3000}'..='\u{9fff}').contains(&c)));
                }
            }
            assert!(rows(&app).len() > bindings().len());
        }
    }

    /// 割り当てが効く条件を満たした状態（道具は、道具の割り当てなら押す前と違うもの）。
    fn state_for(binding: &Binding) -> AppState {
        let mut app = AppState::new(32, 32);
        app.tool = match (binding.when, &binding.action) {
            (When::Tool(tool), _) => tool,
            (_, Action::SelectTool(Tool::Brush)) => Tool::Eraser,
            _ => Tool::Brush,
        };
        if binding.when == When::HasSelection {
            app.apply(Action::Sel(SelAction::Edit(SelEdit::All)));
        }
        app
    }

    #[test]
    fn every_listed_binding_is_judged_to_its_own_action_and_tool_keys_switch_tools() {
        for binding in bindings() {
            if matches!(binding.action, Action::Clip(_)) {
                continue; // クリップボードは次の試験
            }
            // 実際の判定の順（修飾の多いものが先）で、この押しは、この割り当ての操作だけに当たる（同じキーの別の割り当てに横取りされない）
            let app = state_for(&binding);
            let ctx = egui::Context::default();
            let mut got = Vec::new();
            let input = egui::RawInput {
                events: vec![Event::Key {
                    key: binding.key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: binding.modifiers,
                }],
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                ui.input_mut(|i| got = keymap::dispatch(i, &app));
            });
            output.textures_delta.clear();
            assert_eq!(got, vec![binding.action.clone()], "{}", key_label(&binding));
            // 道具のキーは、実際のキーの処理で道具が替わり、一覧の文字は道具の表のキーと同じ
            if let Action::SelectTool(tool) = binding.action {
                let mut app = state_for(&binding);
                press(&mut app, binding.key, binding.modifiers);
                assert_eq!(app.tool, tool, "{}", key_label(&binding));
                assert_eq!(tool.key(), key_label(&binding));
            }
        }
    }

    #[test]
    fn clipboard_bindings_match_the_real_clipboard_dispatch() {
        for binding in bindings() {
            let Action::Clip(_) = binding.action else {
                continue;
            };
            let ctx = egui::Context::default();
            let mut app = AppState::new(32, 32);
            let input = egui::RawInput {
                events: vec![Event::Key {
                    key: binding.key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: binding.modifiers,
                }],
                ..Default::default()
            };
            let mut found = Vec::new();
            let mut output = ctx.run_ui(input, |ui| {
                ui.input_mut(|i| found = crate::clipboard::keys::shortcut_actions(i, &mut app.clip))
            });
            output.textures_delta.clear();
            assert_eq!(found, vec![binding.action]);
        }
    }

    #[test]
    fn the_table_is_the_only_place_that_names_a_shortcut_key() {
        // 文字・数字・記号のキー（ショートカット）を `consume_key` で直に読む所は、割り当ての表だけ。窓の Enter・Escape・Tab などの
        // 操作のキーは、その部品が持つ。ここに足すときは、表に足してから使う
        const UI_KEYS: [&str; 12] = [
            "Enter",
            "Escape",
            "Tab",
            "Space",
            "Backspace",
            "Delete",
            "ArrowUp",
            "ArrowDown",
            "ArrowLeft",
            "ArrowRight",
            "Home",
            "End",
        ];
        fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, root, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let name = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    if name == "keymap.rs" || name == "clipboard/keys.rs" {
                        continue; // 表と、表の割り当てを使う受け口
                    }
                    let text = std::fs::read_to_string(&path).unwrap();
                    let code = text.split("#[cfg(test)]").next().unwrap_or("");
                    for call in code.split(".consume_key(").skip(1) {
                        let args = call.split(')').next().unwrap_or("");
                        if let Some(key) = args.split("Key::").nth(1) {
                            let key = key
                                .split(|c: char| !c.is_ascii_alphanumeric())
                                .next()
                                .unwrap_or("");
                            if !UI_KEYS.contains(&key) {
                                out.push(format!("{name}: Key::{key}"));
                            }
                        }
                    }
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        walk(&root, &root, &mut found);
        assert!(
            found.is_empty(),
            "ショートカットのキーを表の外で直に読んでいる: {found:?}"
        );
    }

    #[test]
    fn movement_keys_and_rotation_text_come_from_the_table() {
        assert_eq!(movement_keys().len(), 4);
        assert_eq!(rotation_text(), "^");
        let mut app = AppState::new(32, 32);
        app.lang = Lang::En;
        let all = rows(&app);
        assert!(all.iter().any(|r| r.left.contains("Move (1 px / 10 px)")));
        assert!(all.iter().any(|r| r.right == "^"));
        // 道具のキーは道具の表のとおり、ツールの帯の並びで一覧に出る
        let mut last = 0;
        for tool in Tool::ALL.iter().filter(|t| !t.key().is_empty()) {
            let at = all
                .iter()
                .position(|r| r.left == tool.name_in(Lang::En) && r.right == tool.key())
                .unwrap_or_else(|| panic!("{tool:?} のキーが一覧にない"));
            assert!(at > last, "{tool:?}: ツールの帯の並び");
            last = at;
        }
    }

    #[test]
    fn menu_and_window_do_not_change_the_document() {
        let mut app = AppState::new(32, 32);
        assert!(crate::shell::menu_entries(&app, crate::shell::HELP_MENU)
            .iter()
            .any(|entry| matches!(
                entry,
                Entry::Item {
                    action: Action::ShowShortcuts,
                    ..
                }
            )));
        let epoch = app.doc_epoch;
        app.apply(Action::ShowShortcuts);
        assert!(app.shortcuts.open);
        assert_eq!(epoch, app.doc_epoch);
        assert!(!app.can_undo());
    }

    #[test]
    fn the_screen_pick_keys_read_as_their_menu_text() {
        // Windows だけの割り当て（この環境の一覧には出ないので、表から直に見る）。メニューの項目のキーの文字と同じ
        let listed: Vec<Binding> = keymap::bindings()
            .into_iter()
            .filter(|b| matches!(b.action, Action::ScreenPick(_)))
            .collect();
        assert_eq!(listed.len(), 2);
        for b in listed {
            let Action::ScreenPick(mode) = b.action else {
                unreachable!()
            };
            assert_eq!(key_label(&b), mode.shortcut());
            assert_eq!(b.when, When::Windows);
        }
        assert!(
            bindings()
                .iter()
                .all(|b| !matches!(b.action, Action::ScreenPick(_)))
                || cfg!(windows)
        );
    }

    #[test]
    fn selection_bindings_are_listed_with_their_conditions() {
        let all = bindings();
        assert!(all
            .iter()
            .any(|b| b.action == Action::Sel(SelAction::Edit(SelEdit::ToNewLayer))));
        assert!(all
            .iter()
            .any(|b| b.action == Action::Path(PathAction::DeleteSelected)));
        assert!(all
            .iter()
            .any(|b| b.action == Action::Clip(ClipAction::Paste)));
    }
}
