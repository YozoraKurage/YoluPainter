//! キーの一覧のウィンドウ（読むだけ）と、メニュー・ツールチップのキーの文字。割り当ては `keymap` の表（操作は `commands` の ID）で、一覧・メニューの文字・
//! 実際のキーの処理は同じ表を読む。名前はメニューと共有する。
pub mod gestures;

use crate::{
    commands,
    keymap::{self, Trigger, When},
    lang::Lang,
    m2::Edit,
    pathtool::PathAction,
    selection::{SelAction, SelEdit},
    state::{Action, AppState},
    ui::menu::Entry,
    windows::{ListSpec, Row},
};
use egui::{Key, Modifiers, Vec2};

#[derive(Default)]
pub struct ShortcutWindow {
    pub open: bool,
    offset: Vec2,
    scroll: f32,
}

/// キーの割り当て 1 つ（`keymap` の表のもの）。
pub type Binding = keymap::KeyBinding;

/// 一覧に出すキーの割り当て（今効いている表のもの。この環境で効くもの。Windows だけのキーは Windows でだけ）。`Action` を持たない行（押している間のキー・
/// ビューが読むキー）は、ビューごとのキーの節に出るので入れない。
pub fn bindings() -> Vec<Binding> {
    keymap::current()
        .rows()
        .iter()
        .filter(|b| b.when != When::Windows || cfg!(windows))
        .filter(|b| b.action().is_some())
        .copied()
        .collect()
}

/// 移動・変形のツールの矢印キー（1 画素の 4 つ。表の行のキー）。
pub fn movement_keys() -> [Key; 4] {
    let small: Vec<&keymap::Nudge> = keymap::NUDGES.iter().filter(|n| !n.shift).collect();
    std::array::from_fn(|i| keymap::key_of(small[i].command).unwrap_or(small[i].key))
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
/// 割り当ての文字（「Ctrl+A」。macOS は Command を「Cmd+」と書く）。文字の入力の行はその文字。
pub fn key_label(binding: &Binding) -> String {
    label_for(binding, cfg!(target_os = "macos"))
}

fn label_for(binding: &Binding, mac: bool) -> String {
    let (m, key) = match binding.trigger {
        Trigger::Text(text) => return text.to_owned(),
        Trigger::Key { modifiers, key } => (modifiers, key),
    };
    key_text(&m, key, mac)
}

fn key_text(m: &Modifiers, key: Key, mac: bool) -> String {
    let mut text = String::new();
    if m.command {
        text.push_str(if mac { "Cmd+" } else { "Ctrl+" });
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
    text.push_str(key_name(key));
    text
}

/// メニューに主の行だけでなく全部の行を並べる操作（やり直しは、Ctrl+Shift+Z と Ctrl+Y のどちらも押せることをメニューが示す）。
const MENU_ALL_ROWS: [&str; 1] = ["edit.redo"];

/// メニューの項目に添えるキーの文字（操作の ID の主の行。表で最初の行。`MENU_ALL_ROWS` の操作は全部の行を「 / 」でつなぐ。割り当てが無ければ None）。
/// メニューの文字は手で書かず、ここから作る。
pub fn menu_key(command: &str) -> Option<String> {
    menu_key_with(command, cfg!(target_os = "macos"))
}

fn menu_key_with(command: &str, mac: bool) -> Option<String> {
    if MENU_ALL_ROWS.contains(&command) {
        let rows: Vec<String> = keymap::current()
            .rows_of(command)
            .map(|b| label_for(b, mac))
            .collect();
        return (!rows.is_empty()).then(|| rows.join(" / "));
    }
    keymap::primary(command).map(|b| label_for(&b, mac))
}

/// 操作に割り当てたキーの文字（「Ctrl+A」。主の行だけ。無ければ None）。
/// ツールチップに書くキーは、文字を直に書かずここから引く（キーの割り当ては `keymap` の表だけが持つ）。
pub fn shortcut_text(action: &Action) -> Option<String> {
    commands::for_action(action)
        .and_then(|c| keymap::primary(c.id))
        .map(|b| key_label(&b))
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
            Action::Fill(crate::fillfx::FillOp::DeletePoint) => {
                l.pick("グラデーションの点を削除", "Delete Gradient Point")
            }
            Action::Path(PathAction::DeleteSelected) => {
                l.pick("パスの点を削除", "Delete Path Point")
            }
            Action::Path(PathAction::SelectPath(None)) => {
                l.pick("パスの編集を終える", "Finish Path")
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
        l.pick("ツール", "Tools"),
        l.pick("編集", "Edit"),
        l.pick("レイヤー", "Layer"),
        l.pick("選択範囲", "Selection"),
        l.pick("表示", "View"),
        l.pick("ファイル", "File"),
    ];
    let mut all = Vec::new();
    let listed: Vec<(Binding, Action)> = bindings()
        .into_iter()
        .filter_map(|b| b.action().map(|action| (b, action)))
        .collect();
    for (g, title) in groups.iter().enumerate() {
        all.push(Row::text(*title, false));
        let in_group = || listed.iter().filter(|(_, action)| group(action) == g);
        for (binding, action) in in_group().filter(|(b, _)| b.key().is_some()) {
            all.push(Row {
                left: action_label(app, action)
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
        // 文字の入力で見る割り当て（キーの位置が配列で違う文字）は、キーの行のあとに別の行で出す
        for (binding, _) in in_group().filter(|(b, _)| b.key().is_none()) {
            all.push(Row {
                left: text_row_label(app, binding),
                middle: String::new(),
                right: key_label(binding),
                warning: false,
            });
        }
    }
    all.extend(context_rows(l));
    all
}

/// 文字の入力で見る割り当ての行の名前（キーの行と並べたとき、どの操作のもう 1 つの割り当てかが分かる名前）。
fn text_row_label(app: &AppState, binding: &Binding) -> String {
    match binding.command {
        "view.rotate_right" => app.lang.pick("右に回転", "Rotate Right").into(),
        _ => binding
            .action()
            .and_then(|action| action_label(app, &action))
            .unwrap_or_default(),
    }
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
            let Some(key) = context.key() else {
                continue;
            };
            rows.push(Row {
                left: context.label(lang).into(),
                middle: String::new(),
                right: format!(
                    "{}{}",
                    key_name(key),
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
    use egui::Event;

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
                let action = binding.action().expect("一覧の行は Action を持つ");
                let label = action_label(&app, &action)
                    .unwrap_or_else(|| panic!("名前がありません: {action:?}"));
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
                assert!(context.key().is_some(), "{}", context.command);
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

    /// 割り当てが効く条件を満たした状態（ツールは、ツールの割り当てなら押す前と違うもの）。
    fn state_for(binding: &Binding) -> AppState {
        let mut app = AppState::new(32, 32);
        app.tool = match (binding.when, binding.action()) {
            (When::Tool(tool), _) => tool,
            (_, Some(Action::SelectTool(Tool::Brush))) => Tool::Eraser,
            _ => Tool::Brush,
        };
        if binding.when == When::HasSelection {
            app.apply(Action::Sel(SelAction::Edit(SelEdit::All)));
        }
        if binding.when == When::PointSelected {
            app.apply(Action::M2(Edit::NewFill));
            let layer = app.selected_layer.expect("足したレイヤー");
            app.apply(Action::Fill(crate::fillfx::FillOp::AddPoints {
                layer,
                channel: yolu_core::Channel::Color,
            }));
            app.apply(Action::Fill(crate::fillfx::FillOp::SelectPoint(Some(0))));
        }
        app
    }

    /// 割り当ての入力そのもの（キーの行はキーの押し、文字の行は文字の入力）。
    fn input_for(binding: &Binding) -> egui::RawInput {
        let events = match binding.trigger {
            Trigger::Key { modifiers, key } => vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            Trigger::Text(text) => vec![Event::Text(text.to_owned())],
        };
        egui::RawInput {
            events,
            ..Default::default()
        }
    }

    #[test]
    fn every_listed_binding_is_judged_to_its_own_action_and_tool_keys_switch_tools() {
        for binding in bindings() {
            let action = binding.action().expect("一覧の行は Action を持つ");
            if matches!(action, Action::Clip(_)) {
                continue; // クリップボードは次の試験
            }
            // 実際の判定の順（修飾の多いものが先）で、この押しは、この割り当ての操作だけに当たる（同じキーの別の割り当てに横取りされない）
            let app = state_for(&binding);
            let ctx = egui::Context::default();
            let mut got = Vec::new();
            let mut output = ctx.run_ui(input_for(&binding), |ui| {
                ui.input_mut(|i| got = keymap::dispatch(i, &app));
            });
            output.textures_delta.clear();
            assert_eq!(got, vec![action.clone()], "{}", key_label(&binding));
            // ツールのキーは、実際のキーの処理でツールが替わり、一覧の文字はツールの表のキーと同じ
            if let Action::SelectTool(tool) = action {
                let mut app = state_for(&binding);
                let Trigger::Key { modifiers, key } = binding.trigger else {
                    panic!("ツールのキーは文字の行ではない");
                };
                press(&mut app, key, modifiers);
                assert_eq!(app.tool, tool, "{}", key_label(&binding));
                assert_eq!(tool.key(), key_label(&binding));
            }
        }
    }

    /// 点のグラデーションの点を編集していて、点を選んでいる状態（Delete・Backspace が点を消す割り当ての条件）。
    fn with_point_selected() -> AppState {
        let delete = bindings()
            .into_iter()
            .find(|b| b.when == When::PointSelected)
            .expect("点を消す割り当て");
        state_for(&delete)
    }

    fn dispatched(app: &AppState, key: Key) -> Vec<Action> {
        let ctx = egui::Context::default();
        let mut got = Vec::new();
        let input = egui::RawInput {
            events: vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            ui.input_mut(|i| got = keymap::dispatch(i, app));
        });
        output.textures_delta.clear();
        got
    }

    #[test]
    fn a_selected_gradient_point_takes_delete_and_backspace_before_the_selection_and_the_path_tool()
    {
        let point = Action::Fill(crate::fillfx::FillOp::DeletePoint);
        // 選択範囲があっても、Delete は点を消す（選択範囲の消去に取られない）
        let mut app = with_point_selected();
        app.apply(Action::Sel(SelAction::Edit(SelEdit::All)));
        assert!(app.doc.selection().is_some());
        assert_eq!(dispatched(&app, Key::Delete), vec![point.clone()]);
        assert_eq!(dispatched(&app, Key::Backspace), vec![point.clone()]);
        // パスのツールでも、点を選んでいる間は点を消す（パスの点の削除に取られない）
        app.tool = Tool::Path;
        assert_eq!(dispatched(&app, Key::Delete), vec![point.clone()]);
        assert_eq!(dispatched(&app, Key::Backspace), vec![point]);
        // 点を選んでいなければ、今までどおり選択範囲の消去・パスの点の削除
        app.apply(Action::Fill(crate::fillfx::FillOp::SelectPoint(None)));
        assert_eq!(dispatched(&app, Key::Delete), vec![sel_edit_erase()]);
        assert_eq!(
            dispatched(&app, Key::Backspace),
            vec![Action::Path(PathAction::DeleteSelected)]
        );
    }

    fn sel_edit_erase() -> Action {
        Action::Sel(SelAction::Edit(SelEdit::Erase))
    }

    #[test]
    fn clipboard_bindings_match_the_real_clipboard_dispatch() {
        for binding in bindings() {
            let Some(Action::Clip(_)) = binding.action() else {
                continue;
            };
            let ctx = egui::Context::default();
            let mut app = AppState::new(32, 32);
            let mut found = Vec::new();
            let mut output = ctx.run_ui(input_for(&binding), |ui| {
                ui.input_mut(|i| found = crate::clipboard::keys::shortcut_actions(i, &mut app.clip))
            });
            output.textures_delta.clear();
            assert_eq!(Some(found[0].clone()), binding.action());
            assert_eq!(found.len(), 1);
        }
    }

    #[test]
    fn the_table_is_the_only_place_that_names_a_shortcut_key() {
        // 文字・数字・記号のキー（ショートカット）を `consume_key` で直に読む所は、割り当ての表だけ。ウィンドウの Enter・Escape・Tab などの
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

    /// `src` の（試験を除く）ソースの全部（相対の道と本文）。
    fn sources() -> Vec<(String, String)> {
        fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, String)>) {
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
                    let text = std::fs::read_to_string(&path).unwrap();
                    let code = text.split("#[cfg(test)]").next().unwrap_or("").to_owned();
                    out.push((name, code));
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut out = Vec::new();
        walk(&root, &root, &mut out);
        out
    }

    #[test]
    fn no_menu_text_is_hand_written_in_the_source() {
        // メニューの項目のキーの文字は、`.command_key(ID)`（表から作る）だけ。`.shortcut("…")` の直書きは試験の中を除いて無い
        let mut found = Vec::new();
        for (name, code) in sources() {
            if code.contains(".shortcut(\"") {
                found.push(name);
            }
        }
        assert!(
            found.is_empty(),
            "メニューのキーの文字を手で書いている: {found:?}"
        );
    }

    #[test]
    fn views_read_their_keys_from_the_table_and_not_from_constants() {
        // 押している間のキー・3D の . ・矢印・^ の文字は、表から ID で引く（定数を直に読まない）
        let banned = [
            "keymap::VIEW_PAN",
            "keymap::VIEW_ROTATE",
            "keymap::STENCIL_MOVE",
            "keymap::STENCIL_BYPASS",
            "keymap::VIEW3D_FRAME",
            "keymap::MOVE_KEYS",
            "keymap::ROTATE_RIGHT_TEXT",
        ];
        let mut found = Vec::new();
        for (name, code) in sources() {
            for word in banned {
                if code.contains(word) {
                    found.push(format!("{name}: {word}"));
                }
            }
        }
        assert!(
            found.is_empty(),
            "表を通さずキーの定数を読んでいる: {found:?}"
        );
        // 読む所が表から引いていること
        let all = sources();
        let code = |name: &str| {
            &all.iter()
                .find(|(n, _)| n == name)
                .unwrap_or_else(|| panic!("{name}"))
                .1
        };
        assert!(code("view3d/navigation.rs").contains("\"view3d.frame_selected\""));
        assert!(code("shell.rs").contains("keymap::NUDGES"));
        assert!(code("stencil/input.rs").contains("\"stencil.transform_hold\""));
        assert!(code("stencil/input.rs").contains("\"stencil.bypass_hold\""));
        assert!(code("canvas/mod.rs").contains("\"view.rotate_hold\""));
        assert!(code("canvas/mod.rs").contains("\"view.pan_hold\""));
        assert!(code("view3d/input.rs").contains("\"view.pan_hold\""));
        assert!(code("panels/view3d.rs").contains("\"view.pan_hold\""));
        assert!(code("bake/uvmap.rs").contains("\"view.pan_hold\""));
    }

    #[test]
    fn movement_keys_and_the_caret_row_come_from_the_table() {
        assert_eq!(
            movement_keys(),
            [
                Key::ArrowLeft,
                Key::ArrowRight,
                Key::ArrowUp,
                Key::ArrowDown
            ]
        );
        let caret = keymap::primary("view.rotate_right").expect("行");
        assert_eq!(caret.trigger, Trigger::Text("^"));
        let mut app = AppState::new(32, 32);
        app.lang = Lang::En;
        let all = rows(&app);
        assert!(all.iter().any(|r| r.left.contains("Move (1 px / 10 px)")));
        assert!(all
            .iter()
            .any(|r| r.left == "Rotate Right" && r.right == "^"));
        assert!(all
            .iter()
            .any(|r| r.left == "Rotate View Right" && r.right == "="));
        // 一覧に出すキーの割り当てには、押している間のキーとビューが読むキーは入らない（ビューごとのキーの節に出る）
        assert!(bindings().iter().all(|b| b.action().is_some()));
        // ツールのキーはツールの表のとおり、ツールの帯の並びで一覧に出る
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
    fn the_listed_rows_keep_their_groups_names_and_keys() {
        // 一覧のウィンドウの中身（行の並び・名前・キーの文字）は、表を ID に替えても変わらない
        let mut app = AppState::new(32, 32);
        app.lang = Lang::En;
        let all = rows(&app);
        let text = |r: &Row| format!("{} | {}", r.left, r.right);
        let find = |title: &str| {
            all.iter()
                .position(|r| r.left == title && r.right.is_empty())
                .unwrap_or_else(|| panic!("見出し {title}"))
        };
        let section = |title: &str, next: &str| -> Vec<String> {
            all[find(title) + 1..find(next)].iter().map(text).collect()
        };
        assert_eq!(
            section("View", "File"),
            [
                "Delete Gradient Point | Delete",
                "Delete Gradient Point | Backspace",
                "Fit to Screen | Ctrl+0",
                "Zoom In | Ctrl++",
                "Zoom In | Ctrl+=",
                "Zoom Out | Ctrl+-",
                "Reset Rotation | Shift+R",
                "Projection Handles | Q",
                "Flip View | H",
                "Rotate View Left | -",
                "Rotate View Right | =",
                "Rotate Right | ^",
            ]
        );
        assert_eq!(
            section("Selection", "View"),
            [
                "Copy to a New Layer | Ctrl+J",
                "Invert Selection | Ctrl+Shift+I",
                "Select All | Ctrl+A",
                "Deselect | Ctrl+D",
                "Erase Selection | Delete",
                "Quick Mask | Shift+Q",
            ]
        );
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
    fn the_screen_pick_keys_read_as_their_documented_text() {
        // Windows だけの割り当て（この環境の一覧には出ないので、表から直に見る）。Ctrl は Command ではなく Control そのもの
        let listed: Vec<Binding> = keymap::bindings()
            .into_iter()
            .filter(|b| b.command.starts_with("color.pick_screen"))
            .collect();
        assert_eq!(listed.len(), 2);
        for b in listed {
            let expected = match b.command {
                "color.pick_screen" => "Ctrl+Alt+I",
                "color.pick_screen_hidden" => "Ctrl+Alt+Shift+I",
                other => panic!("{other}"),
            };
            assert_eq!(key_label(&b), expected);
            assert_eq!(label_for(&b, true), expected, "macOS でも Control のまま");
            assert_eq!(b.when, When::Windows);
        }
        assert!(
            bindings()
                .iter()
                .all(|b| !matches!(b.action(), Some(Action::ScreenPick(_))))
                || cfg!(windows)
        );
        // 修飾の判定は、Windows と同じ押し方（Ctrl は Command としても届く）で当たる。Shift の有無は区別する
        let ctrl_alt = Modifiers::CTRL | Modifiers::COMMAND | Modifiers::ALT;
        assert!(keymap::modifiers_match(
            &ctrl_alt,
            Modifiers::CTRL | Modifiers::ALT,
            Key::I
        ));
        assert!(!keymap::modifiers_match(
            &(ctrl_alt | Modifiers::SHIFT),
            Modifiers::CTRL | Modifiers::ALT,
            Key::I
        ));
        assert!(keymap::modifiers_match(
            &(ctrl_alt | Modifiers::SHIFT),
            Modifiers::CTRL | Modifiers::ALT | Modifiers::SHIFT,
            Key::I
        ));
        // Windows の実際の判定でも Alt+Ctrl+I は画面の色を取る
        if cfg!(windows) {
            let app = AppState::new(32, 32);
            let ctx = egui::Context::default();
            let mut got = Vec::new();
            let m = ctrl_alt;
            let input = egui::RawInput {
                events: vec![Event::Key {
                    key: Key::I,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: m,
                }],
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                ui.input_mut(|i| got = keymap::dispatch(i, &app));
            });
            output.textures_delta.clear();
            assert_eq!(
                got,
                vec![Action::ScreenPick(crate::screen_pick::Mode::Visible)]
            );
        }
    }

    #[test]
    fn selection_bindings_are_listed_with_their_conditions() {
        let all = bindings();
        let has = |action: Action| all.iter().any(|b| b.action() == Some(action.clone()));
        assert!(has(Action::Sel(SelAction::Edit(SelEdit::ToNewLayer))));
        assert!(has(Action::Path(PathAction::DeleteSelected)));
        assert!(has(Action::Clip(ClipAction::Paste)));
    }

    #[test]
    fn a_tool_key_with_an_extra_shift_or_alt_changes_nothing_in_the_real_key_handling() {
        let mut app = AppState::new(32, 32);
        app.tool = Tool::Eraser;
        // Shift+B・Alt+B・Alt+G ではツールが替わらない（以前は書いていない Shift・Alt を気にせず、B・G に当たった）
        press(&mut app, Key::B, Modifiers::SHIFT);
        assert_eq!(app.tool, Tool::Eraser);
        press(&mut app, Key::B, Modifiers::ALT);
        assert_eq!(app.tool, Tool::Eraser);
        press(&mut app, Key::G, Modifiers::ALT);
        assert_eq!(app.tool, Tool::Eraser);
        // 書いたとおりの組み合わせは、今までどおり
        press(&mut app, Key::G, Modifiers::SHIFT);
        assert_eq!(app.tool, Tool::Gradient);
        press(&mut app, Key::G, Modifiers::NONE);
        assert_eq!(app.tool, Tool::Fill);
        press(&mut app, Key::B, Modifiers::NONE);
        assert_eq!(app.tool, Tool::Brush);
        // X・D は Shift を加えると入れ替えない
        let main = app.color.main;
        press(&mut app, Key::X, Modifiers::SHIFT);
        assert_eq!(app.color.main, main);
        press(&mut app, Key::X, Modifiers::NONE);
        assert_ne!(app.color.main, main);
        // 記号のキーは Shift を加えても効く（= を Shift で打つ JIS 配列の Shift+- は、Equals に Shift が付いて届き、表示を右に回す）
        let angle = app.view.angle;
        press(&mut app, Key::Equals, Modifiers::SHIFT);
        assert_ne!(app.view.angle, angle);
        // 数字のキーも（上の段を Shift で打つ AZERTY 配列）。ポリゴン塗りつぶしに替わる
        press(&mut app, Key::Num4, Modifiers::SHIFT);
        assert_eq!(app.tool, Tool::PolygonFill);
        // macOS で Option を押して打つ [ は、ブラシを小さくする
        let radius = app.brush.radius;
        press(&mut app, Key::OpenBracket, Modifiers::ALT);
        assert!(app.brush.radius < radius);
    }

    #[test]
    fn the_key_text_follows_the_platform_command_key() {
        let save = keymap::primary("file.save").expect("行");
        assert_eq!(label_for(&save, false), "Ctrl+S");
        assert_eq!(label_for(&save, true), "Cmd+S");
        let redo = keymap::primary("edit.redo").expect("行");
        assert_eq!(label_for(&redo, true), "Cmd+Shift+Z");
        // メニューの文字は、この環境の書き方（macOS は Cmd）で表の主の行から作る
        let command = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        assert_eq!(
            menu_key("file.save_as").as_deref(),
            Some(format!("{command}+Shift+S").as_str())
        );
        assert_eq!(
            menu_key("tool.gradient").as_deref(),
            Some("Shift+G"),
            "ツールのキーは Shift+G のまま"
        );
        assert_eq!(menu_key("view.rotate_right").as_deref(), Some("^"));
        // やり直しだけは、メニューに全部の行を並べる（ツールチップの文字は主の行だけ）
        assert_eq!(
            menu_key("edit.redo").as_deref(),
            Some(format!("{command}+Shift+Z / {command}+Y").as_str())
        );
        assert_eq!(
            shortcut_text(&Action::Redo).as_deref(),
            Some(format!("{command}+Shift+Z").as_str())
        );
        assert_eq!(
            menu_key("view.zoom_in").as_deref(),
            Some(format!("{command}++").as_str())
        );
        // macOS の書き方（Cmd）は、メニューの文字でも同じ
        assert_eq!(menu_key_with("file.save", true).as_deref(), Some("Cmd+S"));
        assert_eq!(
            menu_key_with("edit.redo", true).as_deref(),
            Some("Cmd+Shift+Z / Cmd+Y")
        );
        assert_eq!(
            menu_key_with("view.zoom_in", true).as_deref(),
            Some("Cmd++")
        );
        assert_eq!(menu_key_with("tool.brush", true).as_deref(), Some("B"));
        assert_eq!(menu_key("tool.liquify"), None);
        assert_eq!(menu_key("no.such.command"), None);
    }

    /// 全部のメニューの項目（入れ子も）。
    fn all_menu_items(app: &AppState) -> Vec<(String, Action, Option<String>)> {
        let mut out = Vec::new();
        let mut menus: Vec<Vec<Entry<Action>>> = (0..=crate::shell::HELP_MENU)
            .map(|i| crate::shell::menu_entries(app, i))
            .collect();
        menus.push(crate::selection::menu::select_menu(app));
        menus.push(crate::shell::layer_menu(app, app.selected_layer));
        for entries in &menus {
            for entry in crate::ui::menu::leaves(entries) {
                if let Entry::Item {
                    label,
                    action,
                    shortcut,
                    ..
                } = entry
                {
                    out.push((label.clone(), action.clone(), shortcut.clone()));
                }
            }
        }
        out
    }

    /// 表から作る前に手で書いていたメニューのキーの文字（macOS 以外）。（メニュー, 日本語, 英語, キー）。メニューは 0 ファイル・1 編集・2 レイヤー・3 選択範囲・5 表示。
    /// 今のメニューの文字が表（主の行）から作るものと同じことは、この固定の表で確かめる（同じ表どうしを比べない）。
    #[cfg(not(target_os = "macos"))]
    const HAND_WRITTEN_MENU_KEYS: [(usize, &str, &str, &str); 51] = [
        // ファイルのメニュー
        (0, "新規プロジェクト…", "New Project…", "Ctrl+N"),
        (0, "開く…", "Open…", "Ctrl+O"),
        (0, "保存", "Save", "Ctrl+S"),
        (0, "別名で保存…", "Save As…", "Ctrl+Shift+S"),
        (0, "終了", "Quit", "Ctrl+Q"),
        // 編集のメニュー
        (1, "取り消し", "Undo", "Ctrl+Z"),
        (1, "やり直し", "Redo", "Ctrl+Shift+Z / Ctrl+Y"),
        (1, "カット", "Cut", "Ctrl+X"),
        (1, "コピー", "Copy", "Ctrl+C"),
        (1, "結合してコピー", "Copy Merged", "Ctrl+Shift+C"),
        (1, "ペースト", "Paste", "Ctrl+V"),
        (1, "ブラシ", "Brush", "B"),
        (1, "消しゴム", "Eraser", "E"),
        (1, "バケツ", "Fill", "G"),
        (1, "図形", "Shape", "U"),
        (1, "定規", "Ruler", "Shift+U"),
        (1, "グラデーション", "Gradient", "Shift+G"),
        (1, "ポリゴン塗りつぶし", "Polygon Fill", "4"),
        (1, "移動・変形", "Move / Transform", "V"),
        (1, "スポイト", "Eyedropper", "I"),
        (1, "パス", "Path", "P"),
        (1, "テキスト", "Text", "T"),
        (
            1,
            "メインとサブの色を入れ替え",
            "Swap Main and Sub Colors",
            "X",
        ),
        (1, "初期設定の色", "Default Colors", "D"),
        (1, "設定…", "Settings…", "Ctrl+,"),
        // レイヤーのメニュー
        (2, "新規レイヤー", "New Layer", "Ctrl+Shift+N"),
        (2, "レイヤーをグループ化", "Group Layers", "Ctrl+G"),
        (2, "複製", "Duplicate", "Ctrl+J"),
        (2, "下のレイヤーと結合", "Merge Down", "Ctrl+E"),
        (2, "表示レイヤーを結合", "Merge Visible", "Ctrl+Shift+E"),
        // 選択範囲のメニュー
        (3, "すべてを選択", "Select All", "Ctrl+A"),
        (3, "選択を解除", "Deselect", "Ctrl+D"),
        (3, "選択範囲を反転", "Invert Selection", "Ctrl+Shift+I"),
        (3, "選択範囲を消去", "Erase Selection", "Delete"),
        (
            3,
            "コピーして新しいレイヤーに",
            "Copy to a New Layer",
            "Ctrl+J",
        ),
        (3, "クイックマスク", "Quick Mask", "Shift+Q"),
        (3, "長方形選択", "Rectangle Select", "M"),
        (3, "楕円形選択", "Ellipse Select", "Shift+M"),
        (3, "なげなわ", "Lasso", "L"),
        (3, "多角形選択", "Polygon Select", "Shift+L"),
        (3, "自動選択", "Magic Wand", "W"),
        (3, "選択ペン", "Selection Pen", "S"),
        (3, "ID の色で選択", "ID Color Select", "Shift+W"),
        // 表示のメニュー
        (5, "ズームイン", "Zoom In", "Ctrl++"),
        (5, "ズームアウト", "Zoom Out", "Ctrl+-"),
        (5, "画面に合わせる", "Fit to Screen", "Ctrl+0"),
        (5, "表示を左に回す", "Rotate View Left", "-"),
        (5, "表示を右に回す", "Rotate View Right", "^"),
        (5, "回転を戻す", "Reset Rotation", "Shift+R"),
        (5, "定規にスナップ", "Snap to Ruler", "Ctrl+1"),
        (5, "表示を左右反転", "Flip View", "H"),
    ];

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_menu_keys_equal_the_hand_written_ones_the_menus_had_before() {
        for lang in Lang::ALL {
            // 0: レイヤーを 1 つ選んでいる、1: 選択範囲がある、2: グループを選んでいる
            for state in 0..3 {
                let mut app = AppState::new_in(32, 32, lang);
                app.apply(Action::NewLayer);
                match state {
                    1 => app.apply(Action::Sel(SelAction::Edit(SelEdit::All))),
                    2 => app.apply(Action::M2(Edit::GroupSelected)),
                    _ => {}
                }
                let mut expected: Vec<(usize, String, String)> = HAND_WRITTEN_MENU_KEYS
                    .iter()
                    .map(|(menu, ja, en, key)| {
                        (*menu, lang.pick(*ja, *en).to_owned(), (*key).to_owned())
                    })
                    .collect();
                let name = |ja: &'static str, en: &'static str| lang.pick(ja, en);
                if state == 1 {
                    // 選択範囲があるあいだの Ctrl+J は「コピーして新しいレイヤー」なので、複製には出さない
                    expected.retain(|(menu, label, _)| {
                        !(*menu == 2 && label == name("複製", "Duplicate"))
                    });
                }
                if state == 2 {
                    for (menu, label, key) in &mut expected {
                        if *menu != 2 {
                            continue;
                        }
                        if *label == name("レイヤーをグループ化", "Group Layers") {
                            *label = name("グループ解除", "Ungroup").to_owned();
                            *key = "Ctrl+Shift+G".to_owned();
                        } else if *label == name("下のレイヤーと結合", "Merge Down") {
                            *label = name("グループを結合", "Merge Group").to_owned();
                        }
                    }
                }
                let mut got = Vec::new();
                for menu in [0, 1, 2, 3, 5] {
                    for entry in crate::ui::menu::leaves(&crate::shell::menu_entries(&app, menu)) {
                        if let Entry::Item {
                            label,
                            shortcut: Some(key),
                            ..
                        } = entry
                        {
                            got.push((menu, label.clone(), key.clone()));
                        }
                    }
                }
                assert_eq!(got, expected, "{lang:?} state {state}");
            }
        }
    }

    #[test]
    fn the_menu_key_text_is_the_primary_row_of_the_operations_own_command() {
        for lang in Lang::ALL {
            // 選択範囲が無いときと、あるとき
            for selected in [false, true] {
                let mut app = AppState::new_in(32, 32, lang);
                app.apply(Action::NewLayer);
                if selected {
                    app.apply(Action::Sel(SelAction::Edit(SelEdit::All)));
                }
                for (label, action, shortcut) in all_menu_items(&app) {
                    let Some(command) = commands::for_action(&action) else {
                        // 操作の一覧に無い Action（グループ解除は押したレイヤーの ID で別に）はキーを出す理由が無い
                        if !matches!(action, Action::M2(Edit::Ungroup(_))) {
                            assert_eq!(shortcut, None, "{lang:?} {label}");
                        }
                        continue;
                    };
                    let expected = menu_key(command.id);
                    if shortcut.is_some() {
                        assert_eq!(shortcut, expected, "{lang:?} {label} ({})", command.id);
                    } else {
                        // キーの文字が無い項目は、割り当てが無い（ゆがみ）か、選択範囲があるあいだの複製（Ctrl+J は別の操作のもの）
                        let ok =
                            expected.is_none() || (selected && command.id == "layer.duplicate");
                        assert!(ok, "{lang:?} {label}: {:?} が出ていない", expected);
                    }
                }
            }
        }
    }

    #[test]
    fn ungroup_shows_its_key_only_for_the_layer_that_is_selected() {
        let mut app = AppState::new(32, 32);
        app.apply(Action::M2(Edit::GroupSelected));
        let group = app.selected_layer.expect("グループ");
        assert!(app.doc.layer(group).is_some_and(|l| l.is_group()));
        let ungroup = |app: &AppState, id| {
            crate::shell::layer_menu(app, Some(id))
                .into_iter()
                .find_map(|e| match e {
                    Entry::Item {
                        action: Action::M2(Edit::Ungroup(_)),
                        shortcut,
                        ..
                    } => Some(shortcut),
                    _ => None,
                })
                .expect("グループ解除の項目")
        };
        // 右クリックしたグループが選んだレイヤーと同じなら、キー（選んだレイヤーへの操作）を出す
        assert_eq!(ungroup(&app, group), menu_key("layer.ungroup"));
        assert!(ungroup(&app, group).is_some());
        // 別のレイヤーを選んでいるときは出さない（キーを押すと選んだレイヤーのほうが動くため）
        app.apply(Action::NewLayer);
        assert_ne!(app.selected_layer, Some(group));
        assert_eq!(ungroup(&app, group), None);
    }
}
