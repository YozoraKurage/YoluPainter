//! キー・修飾キー・マウスのボタンの値の型。状態とキーの設定がこれを持つ（画面の部品のクレートを知らない）。
//! 変種・欄・名前の文字は egui 0.36 のものと同じ（キーの設定のファイルはキーの名前で読み書きする）。入力の事象から変える所は
//! `keymap::key_egui`。

/// キーの変種・名前・ほかに読む名前の表から、`Key` と `ALL`・`name`・`from_name` を作る。
macro_rules! keys {
    ($($variant:ident => $name:literal $(| $alias:literal)*,)*) => {
        /// キーボードのキー。
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
        pub enum Key {
            $($variant,)*
        }

        impl Key {
            /// 全部のキー（宣言の順）。
            pub const ALL: &'static [Key] = &[$(Key::$variant,)*];

            /// 英語の名前（「A」「Space」「Down」）。
            pub fn name(self) -> &'static str {
                match self {
                    $(Key::$variant => $name,)*
                }
            }

            /// 名前から（`name` の文字のほか、記号 1 文字と、ブラウザ・OS の呼び名も読む）。
            pub fn from_name(key: &str) -> Option<Key> {
                Some(match key {
                    $($name $(| $alias)* => Key::$variant,)*
                    _ => return None,
                })
            }
        }
    };
}

keys! {
    ArrowDown => "Down" | "⏷" | "ArrowDown",
    ArrowLeft => "Left" | "⏴" | "ArrowLeft",
    ArrowRight => "Right" | "⏵" | "ArrowRight",
    ArrowUp => "Up" | "⏶" | "ArrowUp",
    Escape => "Escape" | "Esc",
    Tab => "Tab",
    Backspace => "Backspace",
    Enter => "Enter" | "Return" | "NumpadEnter",
    Space => "Space" | " ",
    Insert => "Insert" | "Help",
    Delete => "Delete",
    Home => "Home",
    End => "End",
    PageUp => "PageUp",
    PageDown => "PageDown",
    Copy => "Copy",
    Cut => "Cut",
    Paste => "Paste",
    Colon => "Colon" | ":",
    Comma => "Comma" | "," | "NumpadComma",
    Backslash => "Backslash" | "\\",
    Slash => "Slash" | "/" | "NumpadDivide",
    Pipe => "Pipe" | "|",
    Questionmark => "Questionmark" | "?",
    Exclamationmark => "Exclamationmark" | "!",
    OpenBracket => "OpenBracket" | "[" | "BracketLeft",
    CloseBracket => "CloseBracket" | "]" | "BracketRight",
    OpenCurlyBracket => "OpenCurlyBracket" | "{",
    CloseCurlyBracket => "CloseCurlyBracket" | "}",
    Backtick => "Backtick" | "`" | "Backquote" | "Grave",
    Minus => "Minus" | "-" | "−" | "NumpadSubtract",
    Period => "Period" | "." | "NumpadDecimal",
    Plus => "Plus" | "+" | "NumpadAdd",
    Equals => "Equals" | "=" | "Equal" | "NumpadEqual",
    Semicolon => "Semicolon" | ";",
    Quote => "Quote" | "'",
    Num0 => "0" | "Digit0" | "Numpad0",
    Num1 => "1" | "Digit1" | "Numpad1",
    Num2 => "2" | "Digit2" | "Numpad2",
    Num3 => "3" | "Digit3" | "Numpad3",
    Num4 => "4" | "Digit4" | "Numpad4",
    Num5 => "5" | "Digit5" | "Numpad5",
    Num6 => "6" | "Digit6" | "Numpad6",
    Num7 => "7" | "Digit7" | "Numpad7",
    Num8 => "8" | "Digit8" | "Numpad8",
    Num9 => "9" | "Digit9" | "Numpad9",
    A => "A" | "a" | "KeyA",
    B => "B" | "b" | "KeyB",
    C => "C" | "c" | "KeyC",
    D => "D" | "d" | "KeyD",
    E => "E" | "e" | "KeyE",
    F => "F" | "f" | "KeyF",
    G => "G" | "g" | "KeyG",
    H => "H" | "h" | "KeyH",
    I => "I" | "i" | "KeyI",
    J => "J" | "j" | "KeyJ",
    K => "K" | "k" | "KeyK",
    L => "L" | "l" | "KeyL",
    M => "M" | "m" | "KeyM",
    N => "N" | "n" | "KeyN",
    O => "O" | "o" | "KeyO",
    P => "P" | "p" | "KeyP",
    Q => "Q" | "q" | "KeyQ",
    R => "R" | "r" | "KeyR",
    S => "S" | "s" | "KeyS",
    T => "T" | "t" | "KeyT",
    U => "U" | "u" | "KeyU",
    V => "V" | "v" | "KeyV",
    W => "W" | "w" | "KeyW",
    X => "X" | "x" | "KeyX",
    Y => "Y" | "y" | "KeyY",
    Z => "Z" | "z" | "KeyZ",
    F1 => "F1",
    F2 => "F2",
    F3 => "F3",
    F4 => "F4",
    F5 => "F5",
    F6 => "F6",
    F7 => "F7",
    F8 => "F8",
    F9 => "F9",
    F10 => "F10",
    F11 => "F11",
    F12 => "F12",
    F13 => "F13",
    F14 => "F14",
    F15 => "F15",
    F16 => "F16",
    F17 => "F17",
    F18 => "F18",
    F19 => "F19",
    F20 => "F20",
    F21 => "F21",
    F22 => "F22",
    F23 => "F23",
    F24 => "F24",
    F25 => "F25",
    F26 => "F26",
    F27 => "F27",
    F28 => "F28",
    F29 => "F29",
    F30 => "F30",
    F31 => "F31",
    F32 => "F32",
    F33 => "F33",
    F34 => "F34",
    F35 => "F35",
    BrowserBack => "BrowserBack",
    ShiftLeft => "ShiftLeft",
    ShiftRight => "ShiftRight",
    ControlLeft => "ControlLeft",
    ControlRight => "ControlRight",
    AltLeft => "AltLeft",
    AltRight => "AltRight",
    SuperLeft => "SuperLeft" | "MetaLeft" | "OSLeft",
    SuperRight => "SuperRight" | "MetaRight" | "OSRight",
    IntlBackslash => "IntlBackslash",
}

/// 押している修飾キー。
#[derive(Clone, Copy, Default, Hash, PartialEq, Eq)]
pub struct Modifiers {
    /// Alt（Mac の Option）。
    pub alt: bool,
    /// Control。
    pub ctrl: bool,
    pub shift: bool,
    /// Mac の Command。ほかの OS では常に偽。
    pub mac_cmd: bool,
    /// Mac は Command、ほかの OS は Ctrl（OS をまたいだ割り当てはこちらを見る）。
    pub command: bool,
}

impl std::fmt::Debug for Modifiers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_none() {
            return write!(f, "Modifiers::NONE");
        }
        let mut debug = f.debug_struct("Modifiers");
        for (name, on) in [
            ("alt", self.alt),
            ("ctrl", self.ctrl),
            ("shift", self.shift),
            ("mac_cmd", self.mac_cmd),
            ("command", self.command),
        ] {
            if on {
                debug.field(name, &true);
            }
        }
        debug.finish()
    }
}

impl Modifiers {
    pub const NONE: Self = Self {
        alt: false,
        ctrl: false,
        shift: false,
        mac_cmd: false,
        command: false,
    };
    pub const ALT: Self = Self {
        alt: true,
        ..Self::NONE
    };
    pub const CTRL: Self = Self {
        ctrl: true,
        ..Self::NONE
    };
    pub const SHIFT: Self = Self {
        shift: true,
        ..Self::NONE
    };
    pub const MAC_CMD: Self = Self {
        mac_cmd: true,
        ..Self::NONE
    };
    pub const COMMAND: Self = Self {
        command: true,
        ..Self::NONE
    };

    pub fn is_none(&self) -> bool {
        *self == Self::NONE
    }

    pub fn any(&self) -> bool {
        !self.is_none()
    }

    /// Ctrl・Command だけを見て、押した修飾（`self`）が割り当ての修飾（`pattern`）に当たるか（Alt・Shift は見ない）。
    /// Command の割り当ては Ctrl でも Mac の Command でも当たり、Mac の Command の割り当ては Control も書いたとおりに見る。
    pub fn cmd_ctrl_matches(&self, pattern: Self) -> bool {
        if pattern.mac_cmd {
            return self.mac_cmd && pattern.ctrl == self.ctrl;
        }
        if !pattern.ctrl && !pattern.command {
            return !self.ctrl && !self.command;
        }
        (!pattern.ctrl || self.ctrl) && (!pattern.command || self.command)
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Self;

    /// 両方のどちらかで押している修飾。
    fn bitor(self, rhs: Self) -> Self {
        Self {
            alt: self.alt | rhs.alt,
            ctrl: self.ctrl | rhs.ctrl,
            shift: self.shift | rhs.shift,
            mac_cmd: self.mac_cmd | rhs.mac_cmd,
            command: self.command | rhs.command,
        }
    }
}

/// マウスのボタン。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerButton {
    /// 左。
    Primary,
    /// 右。
    Secondary,
    /// 中（ホイールの押し）。
    Middle,
    /// 戻る。
    Extra1,
    /// 進む。
    Extra2,
}
