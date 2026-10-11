//! 文書（`docs/GUIDE_KEYS.md` と英語の同じ文書）の「既定の割り当て」の表を、既定の表（`keymap::bindings`・`keymap::GESTURES`）から作る。
//! 文書の `BEGIN`〜`END` の間は手で書かず、ここで作った物と同じであることを試験が確かめる（`YOLU_UPDATE_GUIDE_KEYS=1` で書き直す）。
//! キーの文字は Mac でない書き方（Ctrl）。Mac の読み替えは文書の頭に書く。

use crate::commands::{self, Kind};
use crate::keyconfig;
use crate::keymap::{self, Scope, GESTURES};
use crate::lang::Lang;
use crate::state::AppState;

use super::editor::{command_name, mouse_name, when_label, Section};

/// 作った表を入れる所の始めと終わりの印。
pub const BEGIN: &str = "<!-- keymap:begin -->";
pub const END: &str = "<!-- keymap:end -->";

/// 既定の割り当ての表（区分ごとのキーの表と、マウスの組み合わせの表）。
pub fn tables(lang: Lang) -> String {
    let app = AppState::new_in(32, 32, lang);
    let map = keymap::default_map();
    let (open, close) = lang.pick(("（", "）"), (" (", ")"));
    let mut out = String::new();
    for scope in Scope::ALL {
        let section = Section::of_scope(scope);
        let mut rows: Vec<(String, String)> = Vec::new();
        for group in keyconfig::default_groups()
            .into_iter()
            .filter(|g| g.1 == scope)
        {
            let bindings: Vec<&keymap::KeyBinding> = map
                .rows()
                .iter()
                .filter(|b| (b.command, b.scope) == group)
                .collect();
            if bindings.is_empty() {
                continue;
            }
            let mut name = command_name(&app, group.0);
            if let Some(when) = when_label(lang, bindings[0].when) {
                name = format!("{name}{open}{when}{close}");
            }
            let keys: Vec<String> = bindings
                .iter()
                .map(|b| format!("`{}`", super::label_for(b, false)))
                .collect();
            rows.push((name, keys.join(" / ")));
        }
        if section == Section::Everywhere {
            // 画面の部品の決まったキー（変えられない）
            for c in commands::all() {
                if let Kind::Fixed(key) = c.kind {
                    let name = c.static_label(lang).unwrap_or(c.id).to_owned();
                    let key = format!("`{}`", super::key_text_of(key));
                    if !rows.iter().any(|(n, k)| *n == name && *k == key) {
                        rows.push((name, key));
                    }
                }
            }
        }
        if rows.is_empty() {
            continue;
        }
        out.push_str(&format!("### {}\n\n", section.label(lang)));
        out.push_str(lang.pick(
            "| 操作 | キー |\n|---|---|\n",
            "| Action | Key |\n|---|---|\n",
        ));
        for (name, keys) in rows {
            out.push_str(&format!("| {} | {keys} |\n", escape(&name)));
        }
        out.push('\n');
    }
    out.push_str(&format!("### {}\n\n", lang.pick("マウス", "Mouse")));
    out.push_str(lang.pick(
        "| 操作 | 組み合わせ | 効くモード |\n|---|---|---|\n",
        "| Action | Combination | Mode |\n|---|---|---|\n",
    ));
    for g in GESTURES.iter() {
        let binding = super::gestures::Binding {
            scope: g.scope,
            held: g
                .held
                .and_then(|h| map.rows_of(h).find_map(keymap::KeyBinding::key)),
            modifiers: egui::Modifiers {
                alt: g.alt,
                shift: g.shift,
                ctrl: g.ctrl,
                command: g.ctrl,
                ..egui::Modifiers::NONE
            },
            button: g.button,
            operation: g.operation,
            click: g.click,
        };
        let combo = super::gestures::key_label_with(&binding, lang, false);
        let mode = Section::of_scope(g.mode).label(lang);
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            escape(&mouse_name(lang, g.index)),
            escape(&combo),
            mode
        ));
    }
    out
}

/// 表の升の中の `|` を書き換える。
fn escape(text: &str) -> String {
    text.replace('|', "\\|")
}

/// 文書の印の間を、作った表に置き換えた全文（印が無ければ None）。
pub fn replaced(doc: &str, lang: Lang) -> Option<String> {
    let start = doc.find(BEGIN)? + BEGIN.len();
    let end = doc.find(END)?;
    if end < start {
        return None;
    }
    Some(format!(
        "{}\n\n{}{}",
        &doc[..start],
        tables(lang),
        &doc[end..]
    ))
}
