//! マウスと修飾キーの組み合わせの一覧（`keymap::GESTURES` の表から作る）。一覧と実際の入力（3D ビューの回す・パン、ステンシルの移動、選択範囲の
//! 作り方、2D のパン・回転・スポイト）は同じ表を読む。
use crate::lang::Lang;
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
    key_label_with(binding, lang, cfg!(target_os = "macos"))
}

/// `key_label` の、Mac の書き方（Cmd）かを渡せるもの（文書の表は Mac でない書き方で作る）。
pub fn key_label_with(binding: &Binding, lang: Lang, mac: bool) -> String {
    let mut text = String::new();
    if let Some(key) = binding.held {
        text.push_str(key.name());
        text.push('+');
    }
    if binding.modifiers.command {
        text.push_str(if mac { "Cmd+" } else { "Ctrl+" });
    }
    if binding.modifiers.alt {
        text.push_str("Alt+");
    }
    if binding.modifiers.shift {
        text.push_str("Shift+");
    }
    let (ja, en) = match binding.button {
        PointerButton::Primary => ("左ボタン", "Left Button"),
        PointerButton::Secondary => ("右ボタン", "Right Button"),
        PointerButton::Middle => ("中ボタン", "Middle Button"),
        PointerButton::Extra1 => ("戻るボタン", "Back Button"),
        PointerButton::Extra2 => ("進むボタン", "Forward Button"),
    };
    if binding.click {
        text.push_str(&lang.pick(
            format!("{ja}を動かさずに離す"),
            format!("{en} Released without Moving"),
        ));
    } else {
        text.push_str(lang.pick(ja, en));
    }
    text
}
