//! 実際の shell の割り当てからビルド時に生成する、読むだけのキー一覧。
pub mod gestures;

use crate::{
    clipboard::ClipAction,
    lang::Lang,
    m2::Edit,
    pathtool::PathAction,
    selection::{SelAction, SelEdit},
    state::{Action, AppState, Tool},
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

#[derive(Clone, Debug)]
pub struct Binding {
    pub modifiers: Modifiers,
    pub key: Key,
    pub action: Action,
}
fn sel_edit(edit: SelEdit) -> Action {
    Action::Sel(SelAction::Edit(edit))
}
include!(concat!(env!("OUT_DIR"), "/shortcut_catalog.rs"));

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

/// 名前はメニューを共有する。メニューに無いキーだけここで名前を持つ。
pub fn action_label(app: &AppState, action: &Action) -> Option<String> {
    if let Action::SelectTool(tool) = action {
        return Some(tool.name_in(app.lang).into());
    }
    for index in 0..6 {
        for entry in crate::shell::menu_entries(app, index) {
            if let Entry::Item {
                label,
                action: candidate,
                ..
            } = entry
            {
                if &candidate == action {
                    return Some(label);
                }
            }
        }
    }
    let l = app.lang;
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

fn context_label(lang: Lang, scope: &str, key: Key) -> Option<&'static str> {
    Some(match (scope, key) {
        ("canvas", Key::R) => lang.pick("回転", "Rotate"),
        ("canvas" | "view3d", Key::Space) => lang.pick("パン / Ctrl: ズーム", "Pan / Ctrl: Zoom"),
        ("canvas" | "view3d" | "stencil", Key::Escape) => {
            lang.pick("操作をキャンセル", "Cancel Operation")
        }
        ("canvas", Key::Enter) => lang.pick(
            "変形・多角形選択を確定",
            "Confirm Transform / Polygon Selection",
        ),
        ("canvas", Key::Backspace) => lang.pick(
            "多角形選択の最後の点を削除",
            "Delete Last Polygon Selection Point",
        ),
        ("stencil", Key::T) => lang.pick("ステンシルの変形", "Transform Stencil"),
        ("stencil", Key::N) => lang.pick("ステンシルを一時解除", "Bypass Stencil"),
        _ => return None,
    })
}

fn context_rows(lang: Lang) -> Vec<Row> {
    let keys = context_keys();
    let mut rows = Vec::new();
    for (scope, title) in [
        ("canvas", lang.pick("2D ビュー", "2D View")),
        ("view3d", lang.pick("3D ビュー", "3D View")),
        ("stencil", lang.pick("ステンシル", "Stencil")),
    ] {
        rows.push(Row::text(title, false));
        for (_, key) in keys.iter().filter(|(context, _)| *context == scope) {
            if gestures::bindings()
                .iter()
                .any(|b| b.scope == scope && b.held == Some(*key))
            {
                continue;
            }
            let mouse = matches!(
                (scope, key),
                ("canvas", Key::R | Key::Space) | ("view3d", Key::Space) | ("stencil", Key::T)
            );
            rows.push(Row {
                left: context_label(lang, scope, *key)
                    .unwrap_or_else(|| lang.pick("未登録の操作", "Unlisted Action"))
                    .into(),
                middle: String::new(),
                right: format!(
                    "{}{}",
                    key_name(*key),
                    if mouse {
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
    #[test]
    fn every_generated_binding_has_a_localized_name() {
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
            for (scope, key) in context_keys() {
                assert!(
                    context_label(lang, scope, key).is_some(),
                    "{scope}: {key:?}"
                );
            }
            assert!(rows(&app).len() > bindings().len());
        }
    }
    #[test]
    fn generated_tool_bindings_match_real_keyboard_dispatch() {
        for binding in bindings() {
            let Action::SelectTool(tool) = binding.action else {
                continue;
            };
            let ctx = egui::Context::default();
            let mut app = AppState::new(32, 32);
            app.tool = if tool == Tool::Brush {
                Tool::Eraser
            } else {
                Tool::Brush
            };
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key: binding.key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: binding.modifiers,
                }],
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                crate::shell::handle_shortcuts(ui.ctx(), &mut app)
            });
            output.textures_delta.clear();
            assert_eq!(app.tool, tool, "{}", key_label(&binding));
            assert_eq!(tool.key(), key_label(&binding));
        }
    }
    #[test]
    fn generated_clipboard_bindings_match_real_dispatch() {
        for binding in bindings() {
            if !matches!(binding.action, Action::Clip(_)) {
                continue;
            }
            let ctx = egui::Context::default();
            let mut app = AppState::new(32, 32);
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
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
    fn source_catalog_covers_all_direct_calls_and_conditional_keys() {
        let source = include_str!("shell.rs");
        let handle = source
            .split("pub fn handle_shortcuts(")
            .nth(1)
            .unwrap()
            .split("pub fn options_bar(")
            .next()
            .unwrap();
        let calls = handle.matches("        key(").count();
        let clipboard = include_str!("clipboard/keys.rs")
            .matches("if i.consume_key(")
            .count();
        assert_eq!(bindings().len(), calls + clipboard);
        assert_eq!(
            handle.matches("Key::").count(),
            calls + movement_keys().len(),
            "一覧に入らない直接のキー処理が増えています"
        );
        assert!(bindings()
            .iter()
            .any(|b| b.action == sel_edit(SelEdit::ToNewLayer)));
        assert!(bindings()
            .iter()
            .any(|b| b.action == Action::Path(PathAction::DeleteSelected)));
        assert_eq!(movement_keys().len(), 4);
        assert!(handle.contains(&format!("s == {:?}", rotation_text())));
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
}
