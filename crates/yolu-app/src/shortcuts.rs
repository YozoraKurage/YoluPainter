//! キーの一覧のウィンドウ（読むだけ）と、メニュー・ツールチップのキーの文字。割り当ては `keymap` の表（操作は `commands` の ID）で、一覧・メニューの文字・
//! 実際のキーの処理は同じ表を読む。名前は操作の表（`commands`）が持ち、メニューもそれを使う。
pub mod editor;
pub mod gestures;
pub mod guide;

use crate::{
    commands,
    keymap::{self, Trigger, When},
    lang::Lang,
    mode::EditorMode,
    state::key::{Key, Modifiers},
    state::Action,
};

pub use editor::ShortcutWindow;

/// キーの割り当て 1 つ（`keymap` の表のもの）。
pub type Binding = keymap::KeyBinding;

/// 一覧に出すキーの割り当て（今効いている表のもの。この環境で効くもの。Windows だけのキーは Windows でだけ）。`Action` を持たない行（押している間のキー・
/// ビューが読むキー）は、ビューごとのキーの節に出るので入れない。
pub fn bindings() -> Vec<Binding> {
    let mut out: Vec<Binding> = Vec::new();
    for b in keymap::current().rows() {
        if (b.when == When::Windows && !cfg!(windows)) || b.action().is_none() {
            continue;
        }
        // 同じ操作・同じ入力の行（編集とポーズの段に同じキーで置いた行）は 1 行にする
        if out
            .iter()
            .any(|o| o.command == b.command && o.trigger == b.trigger)
        {
            continue;
        }
        out.push(*b);
    }
    out
}

/// 移動・変形のツールの矢印キー（1 画素の 4 つ。表の行のキー）。
pub fn movement_keys() -> [Key; 4] {
    let small: Vec<&keymap::Nudge> = keymap::NUDGES.iter().filter(|n| !n.shift).collect();
    std::array::from_fn(|i| keymap::key_of(small[i].command).unwrap_or(small[i].key))
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
        Key::Period => ".",
        Key::Slash => "/",
        Key::Backslash => "\\",
        Key::Semicolon => ";",
        Key::Escape => "Esc",
        _ => key.name(),
    }
}

/// キーの文字（文書・一覧の書き方。記号は記号で、Esc は Esc）。
pub fn key_text_of(key: Key) -> &'static str {
    key_name(key)
}
/// 割り当ての文字（「Ctrl+A」。macOS は Command を「Cmd+」と書く）。文字の入力の行はその文字。
pub fn key_label(binding: &Binding) -> String {
    label_for(binding, cfg!(target_os = "macos"))
}

pub(crate) fn label_for(binding: &Binding, mac: bool) -> String {
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

thread_local! {
    /// メニューを作っている間のモード（`menu_mode` が決め、`menu_key` が読む）。既定はペイント。
    static MENU_MODE: std::cell::Cell<EditorMode> = const { std::cell::Cell::new(EditorMode::Paint) };
}

/// メニューを作る間、キーの文字をそのモードの表から引く（戻すと前のモードへ）。
pub struct MenuMode(EditorMode);

impl Drop for MenuMode {
    fn drop(&mut self) {
        MENU_MODE.with(|m| m.set(self.0));
    }
}

/// この後 `MenuMode` を捨てるまで、`menu_key` は `mode` で効くキーだけを出す（メニューを作る口が使う）。
pub fn menu_mode(mode: EditorMode) -> MenuMode {
    MenuMode(MENU_MODE.with(|m| m.replace(mode)))
}

/// メニューの項目に添えるキーの文字（操作の ID の主の行。表で最初の行。`MENU_ALL_ROWS` の操作は全部の行を「 / 」でつなぐ。割り当てが無ければ None）。
/// メニューの文字は手で書かず、ここから作る。メニューを作っている間は、そのモードで効く行だけ（`menu_mode`）。
pub fn menu_key(command: &str) -> Option<String> {
    menu_key_in(command, MENU_MODE.with(|m| m.get()))
}

/// そのモードで効く行からの、キーの文字（ほかのモードの行・同じキーをほかの操作が先に取る行は出さない）。
pub fn menu_key_in(command: &str, mode: EditorMode) -> Option<String> {
    menu_key_with(command, mode, cfg!(target_os = "macos"))
}

fn menu_key_with(command: &str, mode: EditorMode, mac: bool) -> Option<String> {
    let map = keymap::current();
    let mut rows = map
        .rows_of(command)
        .filter(|b| keymap::effective_in(&map, b, mode));
    if MENU_ALL_ROWS.contains(&command) {
        let rows: Vec<String> = rows.map(|b| label_for(b, mac)).collect();
        return (!rows.is_empty()).then(|| rows.join(" / "));
    }
    rows.next().map(|b| label_for(b, mac))
}

/// 操作の、そのモードで効くキーの文字（主の行。無ければ None）。ツールチップに添えるキーは、ここから引く（設定で変えたキーに付いてくる）。
pub fn key_in(command: &str, mode: EditorMode) -> Option<String> {
    menu_key_in(command, mode)
}

/// 名前にキーを添える（「選択を解除（Ctrl+D / Esc）」。英語は「Deselect (Ctrl+D / Esc)」）。キーが無ければ名前だけ。
pub fn named_with_keys(lang: Lang, name: &str, keys: &[Option<String>]) -> String {
    let keys: Vec<&str> = keys.iter().flatten().map(String::as_str).collect();
    if keys.is_empty() {
        return name.to_owned();
    }
    let keys = keys.join(" / ");
    lang.f("shortcuts-text", &[("name", &name), ("keys", &keys)])
}

/// ツールのキーの文字（ペイントのモードの表から。設定で外していれば空）。
pub fn tool_key(tool: crate::state::Tool) -> String {
    menu_key_in(commands::tool_command(tool), EditorMode::Paint).unwrap_or_default()
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
        Some(key) => lang.f("shortcuts-text-tip", &[("name", &name), ("key", &key)]),
        None => name.to_owned(),
    }
}

#[cfg(test)]
mod tests;
