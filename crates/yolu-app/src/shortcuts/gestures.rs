//! マウスと修飾キーの組み合わせの一覧（`keymap::GESTURES` の表から作る）。一覧と実際の入力（3D ビューの回す・パン、ステンシルの移動、選択範囲の
//! 作り方、2D のパン・回転・スポイト）は同じ表を読む。
use crate::{lang::Lang, windows::Row};
use egui::{Key, Modifiers, PointerButton};

pub use crate::keymap::Operation;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Binding {
    pub scope: &'static str,
    pub held: Option<Key>,
    pub modifiers: Modifiers,
    pub button: PointerButton,
    pub operation: Operation,
    /// 動かさずに離したときの操作（「右クリック」のように、ボタンを押したまま動かす操作とは別の行）。
    pub click: bool,
}

/// マウスの組み合わせの全部（押している間のキーは、キーの表の今の割り当て）。
pub fn bindings() -> Vec<Binding> {
    crate::keymap::GESTURES
        .iter()
        .map(|g| Binding {
            scope: g.scope,
            held: g.held.and_then(crate::keymap::hold_key),
            modifiers: Modifiers {
                alt: g.alt,
                shift: g.shift,
                ctrl: g.ctrl,
                command: g.ctrl,
                ..Modifiers::NONE
            },
            button: g.button,
            operation: g.operation,
            click: g.click,
        })
        .collect()
}

pub fn key_label(binding: &Binding, lang: Lang) -> String {
    let mut text = String::new();
    if let Some(key) = binding.held {
        text.push_str(key.name());
        text.push('+');
    }
    if binding.modifiers.command {
        text.push_str(if cfg!(target_os = "macos") {
            "Cmd+"
        } else {
            "Ctrl+"
        });
    }
    if binding.modifiers.alt {
        text.push_str("Alt+");
    }
    if binding.modifiers.shift {
        text.push_str("Shift+");
    }
    text.push_str(match (binding.button, binding.click) {
        (PointerButton::Primary, false) => lang.pick("左ボタン", "Left Button"),
        (PointerButton::Primary, true) => lang.pick("左クリック", "Left Click"),
        (PointerButton::Middle, _) => lang.pick("中ボタン", "Middle Button"),
        (PointerButton::Secondary, false) => lang.pick("右ボタン", "Right Button"),
        (PointerButton::Secondary, true) => lang.pick("右クリック", "Right Click"),
        _ => unreachable!("一覧は3つのボタンを列挙する"),
    });
    text
}

/// 一覧で見せるまとまり。選択範囲のツールの組み合わせは 2D ビューに並べる。
fn listed_under(scope: &str) -> &str {
    if scope == "selection" {
        "canvas"
    } else {
        scope
    }
}

pub fn rows(scope: &str, lang: Lang) -> Vec<Row> {
    bindings()
        .iter()
        .filter(|b| listed_under(b.scope) == scope)
        .map(|b| Row {
            left: b.operation.label(lang).into(),
            middle: String::new(),
            right: key_label(b, lang),
            warning: false,
        })
        .collect()
}
