//! egui の入力の型と、状態が持つキーの型（`state::key`）の間の変換。egui の事象・入力を読む所が `.into()` で使う。

use crate::state::key::{Key, Modifiers, PointerButton};

/// 状態のキーの番号（宣言の順）から egui のキー。egui の `Key::ALL` は宣言の順ではないので、宣言の番号の所に置き直す。
const EGUI_KEYS: [egui::Key; Key::ALL.len()] = {
    let mut table = [egui::Key::A; Key::ALL.len()];
    let mut i = 0;
    while i < egui::Key::ALL.len() {
        let key = egui::Key::ALL[i];
        table[key as usize] = key;
        i += 1;
    }
    table
};

const _: () = assert!(egui::Key::ALL.len() == Key::ALL.len());

impl From<egui::Key> for Key {
    fn from(key: egui::Key) -> Key {
        Key::ALL[key as usize]
    }
}

impl From<Key> for egui::Key {
    fn from(key: Key) -> egui::Key {
        EGUI_KEYS[key as usize]
    }
}

impl From<egui::Modifiers> for Modifiers {
    fn from(m: egui::Modifiers) -> Modifiers {
        Modifiers {
            alt: m.alt,
            ctrl: m.ctrl,
            shift: m.shift,
            mac_cmd: m.mac_cmd,
            command: m.command,
        }
    }
}

impl From<Modifiers> for egui::Modifiers {
    fn from(m: Modifiers) -> egui::Modifiers {
        egui::Modifiers {
            alt: m.alt,
            ctrl: m.ctrl,
            shift: m.shift,
            mac_cmd: m.mac_cmd,
            command: m.command,
        }
    }
}

impl From<egui::PointerButton> for PointerButton {
    fn from(button: egui::PointerButton) -> PointerButton {
        match button {
            egui::PointerButton::Primary => PointerButton::Primary,
            egui::PointerButton::Secondary => PointerButton::Secondary,
            egui::PointerButton::Middle => PointerButton::Middle,
            egui::PointerButton::Extra1 => PointerButton::Extra1,
            egui::PointerButton::Extra2 => PointerButton::Extra2,
        }
    }
}

impl From<PointerButton> for egui::PointerButton {
    fn from(button: PointerButton) -> egui::PointerButton {
        match button {
            PointerButton::Primary => egui::PointerButton::Primary,
            PointerButton::Secondary => egui::PointerButton::Secondary,
            PointerButton::Middle => egui::PointerButton::Middle,
            PointerButton::Extra1 => egui::PointerButton::Extra1,
            PointerButton::Extra2 => egui::PointerButton::Extra2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_egui_key_converts_both_ways_with_the_same_name_order_and_spellings() {
        assert_eq!(Key::ALL.len(), egui::Key::ALL.len());
        for &egui_key in egui::Key::ALL {
            let key = Key::from(egui_key);
            assert_eq!(egui::Key::from(key), egui_key);
            assert_eq!(key.name(), egui_key.name(), "{egui_key:?}");
            assert_eq!(format!("{key:?}"), format!("{egui_key:?}"));
            // 名前と記号の両方を、egui と同じキーに読む
            for text in [egui_key.name(), egui_key.symbol_or_name()] {
                assert_eq!(Key::from_name(text), Some(key), "{text:?}");
            }
            for &other in egui::Key::ALL {
                assert_eq!(key.cmp(&Key::from(other)), egui_key.cmp(&other));
            }
        }
        for &key in Key::ALL {
            assert_eq!(Key::from(egui::Key::from(key)), key);
        }
    }

    #[test]
    fn every_spelling_the_key_table_reads_is_read_the_same_way_by_egui() {
        // 表の名前のほかに読む文字（ブラウザ・OS の呼び名・小文字）も、egui と同じキーになる
        let spellings = "⏷ ArrowDown ⏴ ArrowLeft ⏵ ArrowRight ⏶ ArrowUp Esc Return NumpadEnter Help : , \
            NumpadComma \\ / NumpadDivide | ? ! [ BracketLeft ] BracketRight { } ` Backquote Grave - − \
            NumpadSubtract . NumpadDecimal + NumpadAdd = Equal NumpadEqual ; ' MetaLeft OSLeft MetaRight \
            OSRight Shift ctrl F36 Numpad space"
            .split_whitespace()
            .chain([" ", ""]);
        for text in spellings {
            assert_eq!(
                Key::from_name(text),
                egui::Key::from_name(text).map(Key::from),
                "{text:?}"
            );
        }
        let digits = ('0'..='9').flat_map(|d| [format!("Digit{d}"), format!("Numpad{d}")]);
        let letters =
            ('a'..='z').flat_map(|c| [c.to_string(), format!("Key{}", c.to_ascii_uppercase())]);
        for text in digits.chain(letters) {
            assert_eq!(
                Key::from_name(&text),
                egui::Key::from_name(&text).map(Key::from),
                "{text:?}"
            );
        }
    }

    #[test]
    fn all_32_modifier_combinations_convert_both_ways_and_answer_like_egui() {
        let all: Vec<egui::Modifiers> = (0u8..32)
            .map(|bits| egui::Modifiers {
                alt: bits & 1 != 0,
                ctrl: bits & 2 != 0,
                shift: bits & 4 != 0,
                mac_cmd: bits & 8 != 0,
                command: bits & 16 != 0,
            })
            .collect();
        for &e in &all {
            let m = Modifiers::from(e);
            assert_eq!(egui::Modifiers::from(m), e);
            assert_eq!(format!("{m:?}"), format!("{e:?}"));
            assert_eq!(m.is_none(), e.is_none(), "{e:?}");
            assert_eq!(m.any(), e.any(), "{e:?}");
            for &q in &all {
                let other = Modifiers::from(q);
                assert_eq!(m.contains(other), e.contains(q), "{e:?} {q:?}");
                assert_eq!(
                    m.cmd_ctrl_matches(other),
                    e.cmd_ctrl_matches(q),
                    "{e:?} {q:?}"
                );
                assert_eq!(Modifiers::from(e | q), m | other, "{e:?} {q:?}");
                assert_eq!(m == other, e == q);
            }
        }
        for (mine, theirs) in [
            (Modifiers::NONE, egui::Modifiers::NONE),
            (Modifiers::ALT, egui::Modifiers::ALT),
            (Modifiers::CTRL, egui::Modifiers::CTRL),
            (Modifiers::SHIFT, egui::Modifiers::SHIFT),
            (Modifiers::MAC_CMD, egui::Modifiers::MAC_CMD),
            (Modifiers::COMMAND, egui::Modifiers::COMMAND),
            (Modifiers::default(), egui::Modifiers::default()),
        ] {
            assert_eq!(egui::Modifiers::from(mine), theirs);
        }
    }

    #[test]
    fn the_five_pointer_buttons_convert_both_ways() {
        use egui::PointerButton as E;
        for e in [E::Primary, E::Secondary, E::Middle, E::Extra1, E::Extra2] {
            let b = PointerButton::from(e);
            assert_eq!(E::from(b), e);
            assert_eq!(format!("{b:?}"), format!("{e:?}"));
            assert_eq!(b as usize, e as usize);
        }
    }
}
