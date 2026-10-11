//! キーとマウスの割り当ての宣言の表。キーボードの割り当て（`bindings`）・マウスと修飾キーの組み合わせ（`GESTURES`）・ビューの中のキー（`CONTEXT_KEYS`）を、
//! ここだけに書く。割り当ては操作の ID（`commands`）を指し、`Action` はそこから作る。キーの処理（`shell::handle_shortcuts`・3D ビューの回す・パン・
//! ステンシルの移動など）・メニューのキーの文字・ショートカットの一覧のウィンドウ（`shortcuts`）は、この表を読む。実装の入力判定のソースを文字で読んで
//! 一覧を作る方式（ビルドスクリプト）はやめた: 表が実装の一次の資料なので、一覧と実際の入力が食い違わない（食い違いは試験が、表のすべての割り当てを実際の
//! 入力へ流して確かめる）。ツールのキーはツールの表（`tools`）の `key` から作る。キーを利用者が替える設定は、この表の上に作る（`table` の作り方を差し替える）。
//! ツールのキーの動き方（押すと切り替え・押している間だけ・短く押すと切り替え／長押しで押している間だけ）は、表の行ではなく `Keymap::tool_mode` に持ち、
//! `dispatch_fired` が押しと一緒に返す（実行と離しの見張りは `toolkeys`）。
//!
//! 修飾キーは厳密に見る（`modifiers_match`）。Ctrl・Command は egui の `Modifiers::cmd_ctrl_matches` と同じ。文字（A〜Z）・F キー・名前のキー（Tab・Space・
//! Enter・Escape・Backspace・Delete・矢印・Home・End・PageUp・PageDown・Insert）は、Shift と Alt を書いたとおりに見る（書いていない Shift・Alt を押していれば
//! 当てない）。記号のキーと数字のキーは、配列によって Shift や Alt（macOS の Option）を押して打つので、行に書いていない Shift・Alt は見ない
//! （行に書いてあれば押していなければならない）。たとえば US 配列の `+` は Shift+= で Plus として、JIS 配列の `=` は Shift+- で Equals として届き、
//! macOS のドイツ語配列の `[` は Option+5 で届く。AZERTY 配列は上の段の数字を Shift で打つ。判定の順は修飾の多いものが先、同じならキーの段
//! （`Layer`: 場面・モード・どこでも）の上が先、それも同じなら表の順。途中の操作の段の行は、その操作が読む。

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use egui::{Event, InputState, Key, Modifiers, PointerButton};

use crate::clipboard::ClipAction;
use crate::commands;
use crate::lang::Lang;
use crate::state::{Action, AppState, Tool};
use crate::toolkeys::ToolKeyMode;

// ───────── キーボードの割り当て ─────────

/// 割り当てが効く条件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Always,
    /// 選択範囲があるとき。
    HasSelection,
    /// このツールを選んでいるとき。
    Tool(Tool),
    /// Windows のとき（画面から色を取る）。
    Windows,
    /// 点のグラデーションの点を編集していて、点を選んでいるとき。
    PointSelected,
}

impl When {
    pub fn holds(self, app: &AppState) -> bool {
        match self {
            When::Always => true,
            When::HasSelection => app.doc.selection().is_some(),
            When::Tool(tool) => app.tool == tool,
            When::Windows => cfg!(windows) && !app.is_stroking(),
            When::PointSelected => {
                crate::fillfx::points::target(app).is_some() && app.fillfx.point_selected.is_some()
            }
        }
    }
}

/// 割り当てが効く範囲（どのモードで効くか。途中の操作の間だけのキーは、その操作が読む）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// どのモードでも（ファイル・編集・選択範囲・レイヤー・表示・視点）。
    Everywhere,
    /// ペイントのモードだけ（ツール・色・ブラシの大きさ・Q・Shift+Q など）。
    Paint,
    /// 編集のモードだけ。
    Edit,
    /// ポーズのモードだけ。
    Pose,
    /// 途中の操作（3D で右を押している間の視点の移動など）の間だけ。その操作が読み（`take_fly_keys`）、`dispatch` は当てない。
    During,
}

impl Scope {
    /// 今のモードで、この範囲の行が `dispatch` で効くか。
    pub fn holds(self, app: &AppState) -> bool {
        self.holds_in(app.mode)
    }

    /// そのモードで、この範囲の行が `dispatch` で効くか。
    pub fn holds_in(self, mode: crate::mode::EditorMode) -> bool {
        use crate::mode::EditorMode;
        match self {
            Scope::Everywhere => true,
            Scope::Paint => mode == EditorMode::Paint,
            Scope::Edit => mode == EditorMode::Edit,
            Scope::Pose => mode == EditorMode::Pose,
            Scope::During => false,
        }
    }

    /// モードの段の範囲か（ペイント・編集・ポーズ）。
    pub fn is_mode(self) -> bool {
        matches!(self, Scope::Paint | Scope::Edit | Scope::Pose)
    }

    pub const ALL: [Scope; 5] = [
        Scope::Everywhere,
        Scope::Paint,
        Scope::Edit,
        Scope::Pose,
        Scope::During,
    ];

    /// 設定のファイル（keymap.json）の名前。
    pub fn key(self) -> &'static str {
        match self {
            Scope::Everywhere => "everywhere",
            Scope::Paint => "paint",
            Scope::Edit => "edit",
            Scope::Pose => "pose",
            Scope::During => "during",
        }
    }

    pub fn from_key(key: &str) -> Option<Scope> {
        Scope::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// キーの段（上の段ほど先に見る。上の段が受けた入力は、下の段へ落ちない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// 1 途中の操作（右を押している間の視点の移動・開いているパイ）。その操作が先に入力を取る（`take_fly_keys`・`pie`）。
    During,
    /// 2 場面（選択範囲がある・ツール・点を選んでいる など。`When` が `Always` でない行）。
    Context,
    /// 3 モード（ペイント・編集・ポーズ）。
    Mode,
    /// 4 どこでも。
    Everywhere,
}

/// 割り当ての入力。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// キー（修飾キーは厳密に見る）。
    Key { modifiers: Modifiers, key: Key },
    /// 文字の入力（配列でキーの位置が違う文字。JIS の ^ のキー、US の Shift+6）。キーの行と区別して、入力の文字で見る。
    Text(&'static str),
}

/// 1 つのキーの割り当て。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    pub trigger: Trigger,
    /// 操作の ID（`commands`）。
    pub command: &'static str,
    pub scope: Scope,
    pub when: When,
}

impl KeyBinding {
    pub fn modifiers(&self) -> Modifiers {
        match self.trigger {
            Trigger::Key { modifiers, .. } => modifiers,
            Trigger::Text(_) => Modifiers::NONE,
        }
    }

    pub fn key(&self) -> Option<Key> {
        match self.trigger {
            Trigger::Key { key, .. } => Some(key),
            Trigger::Text(_) => None,
        }
    }

    /// この行のキーの段。
    pub fn layer(&self) -> Layer {
        if self.scope == Scope::During {
            Layer::During
        } else if self.when != When::Always {
            Layer::Context
        } else if self.scope.is_mode() {
            Layer::Mode
        } else {
            Layer::Everywhere
        }
    }

    /// この割り当てがキーで実行する `Action`（押しをビューが読む操作・押している間のキーは None）。利用者が作ったパイを開く行（`pie.<ID>`）は、
    /// そのパイを開く。
    pub fn action(&self) -> Option<Action> {
        if let Some(id) = pie_of(self.command) {
            return Some(Action::Pie(crate::pie::PieAction::Open(id.to_owned())));
        }
        commands::find(self.command)
            .and_then(|c| c.action)
            .map(|make| make())
    }

    fn scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    fn when(mut self, when: When) -> Self {
        self.when = when;
        self
    }

    /// ペイントのモードだけの割り当てにする。
    fn paint(self) -> Self {
        self.scope(Scope::Paint)
    }
}

/// 利用者が作ったパイを開く操作の ID（`pie.<パイの ID>`）なら、そのパイの ID。
pub fn pie_of(command: &str) -> Option<&str> {
    command.strip_prefix("pie.")
}

/// 文字を、プロセスの間ずっと使える文字にする（利用者が作ったパイの操作の ID。同じ文字は 1 度だけ確保する）。
pub fn intern(text: &str) -> &'static str {
    static SEEN: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut seen = SEEN
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(found) = seen.get(text) {
        return found;
    }
    let made: &'static str = Box::leak(text.to_owned().into_boxed_str());
    seen.insert(made);
    made
}

/// 2 つの入力が、この環境で同じ押しに当たるか（Mac 以外の Ctrl は、Command の割り当ても Control の割り当ても同じ Ctrl）。
pub fn same_input(a: &Trigger, b: &Trigger) -> bool {
    match (a, b) {
        (Trigger::Text(x), Trigger::Text(y)) => x == y,
        (
            Trigger::Key {
                modifiers: ma,
                key: ka,
            },
            Trigger::Key {
                modifiers: mb,
                key: kb,
            },
        ) => {
            let ctrl = |m: &Modifiers| {
                if cfg!(target_os = "macos") {
                    (m.command || m.mac_cmd, m.ctrl)
                } else {
                    (m.command || m.ctrl || m.mac_cmd, false)
                }
            };
            ka == kb && ctrl(ma) == ctrl(mb) && ma.alt == mb.alt && ma.shift == mb.shift
        }
        _ => false,
    }
}

fn kb(modifiers: Modifiers, key: Key, command: &'static str) -> KeyBinding {
    KeyBinding {
        trigger: Trigger::Key { modifiers, key },
        command,
        scope: Scope::Everywhere,
        when: When::Always,
    }
}

fn kb_text(text: &'static str, command: &'static str) -> KeyBinding {
    KeyBinding {
        trigger: Trigger::Text(text),
        command,
        scope: Scope::Everywhere,
        when: When::Always,
    }
}

/// ツールのキーの文字（「B」「Shift+G」「4」。ツールの表の `key`）から、修飾とキーを読む。キーが空・読めなければ None。
pub fn parse_tool_key(text: &str) -> Option<(Modifiers, Key)> {
    let mut modifiers = Modifiers::NONE;
    let mut rest = text;
    while let Some((head, tail)) = rest.split_once('+') {
        match head {
            "Shift" => modifiers.shift = true,
            "Ctrl" => {
                modifiers.ctrl = true;
                modifiers.command = true;
            }
            "Alt" => modifiers.alt = true,
            _ => return None,
        }
        rest = tail;
    }
    Key::from_name(rest).map(|key| (modifiers, key))
}

/// 移動・変形のツールの矢印キー 1 つ（画面の向きの 1 画素。Shift で 10）。
#[derive(Clone, Copy, Debug)]
pub struct Nudge {
    pub command: &'static str,
    pub key: Key,
    pub shift: bool,
    /// 画面の向き（x は右、y は下）。
    pub direction: (f64, f64),
}

/// 移動・変形のツールの矢印キーの全部（1 画素の 4 つ、Shift で 10 画素の 4 つ）。
pub const NUDGES: [Nudge; 8] = [
    nudge("transform.nudge_left", Key::ArrowLeft, false, (-1.0, 0.0)),
    nudge("transform.nudge_right", Key::ArrowRight, false, (1.0, 0.0)),
    nudge("transform.nudge_up", Key::ArrowUp, false, (0.0, -1.0)),
    nudge("transform.nudge_down", Key::ArrowDown, false, (0.0, 1.0)),
    nudge("transform.nudge_left_10", Key::ArrowLeft, true, (-1.0, 0.0)),
    nudge(
        "transform.nudge_right_10",
        Key::ArrowRight,
        true,
        (1.0, 0.0),
    ),
    nudge("transform.nudge_up_10", Key::ArrowUp, true, (0.0, -1.0)),
    nudge("transform.nudge_down_10", Key::ArrowDown, true, (0.0, 1.0)),
];

const fn nudge(command: &'static str, key: Key, shift: bool, direction: (f64, f64)) -> Nudge {
    Nudge {
        command,
        key,
        shift,
        direction,
    }
}

/// 3D ビューで右ボタンを押している間の視点の移動キー 1 つ（`direction` はカメラの (右, 上, 前) の向き）。
#[derive(Clone, Copy, Debug)]
pub struct FlyKey {
    pub command: &'static str,
    pub key: Key,
    pub direction: [f32; 3],
}

/// 視点の移動キーの全部（W/S は前後、A/D は左右、Q/E は下上）。キーは表の行（`hold_key`）で引く。
pub const FLY_KEYS: [FlyKey; 6] = [
    fly("view3d.fly_forward", Key::W, [0.0, 0.0, 1.0]),
    fly("view3d.fly_back", Key::S, [0.0, 0.0, -1.0]),
    fly("view3d.fly_left", Key::A, [-1.0, 0.0, 0.0]),
    fly("view3d.fly_right", Key::D, [1.0, 0.0, 0.0]),
    fly("view3d.fly_down", Key::Q, [0.0, -1.0, 0.0]),
    fly("view3d.fly_up", Key::E, [0.0, 1.0, 0.0]),
];

const fn fly(command: &'static str, key: Key, direction: [f32; 3]) -> FlyKey {
    FlyKey {
        command,
        key,
        direction,
    }
}

/// 一覧に出す順の、キーボードの割り当て（クリップボードのキー・押している間のキー・ビューが読むキーも含む）。
/// 1 つの操作に行が 2 つ以上あるとき、最初の行が「主の行」（メニューのキーの文字になる）。
pub fn bindings() -> Vec<KeyBinding> {
    let cmd = Modifiers::COMMAND;
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let none = Modifiers::NONE;
    let shift = Modifiers::SHIFT;
    let mut v = vec![
        // 結合は画素を変えるので、ペイントのモードだけ（メニューからはどのモードでも）
        kb(cmd_shift, Key::E, "layer.merge_visible").paint(),
        kb(cmd_shift, Key::G, "layer.ungroup"),
        kb(cmd, Key::E, "layer.merge_down").paint(),
        // Ctrl+J: 選択範囲があれば、その画素を新しいレイヤーへ（Photoshop の「コピーしたレイヤー」）。無ければレイヤーの複製
        kb(cmd, Key::J, "selection.to_new_layer").when(When::HasSelection),
        kb(cmd, Key::J, "layer.duplicate"),
        kb(cmd, Key::G, "layer.group"),
        kb(cmd_shift, Key::I, "selection.invert"),
        kb(cmd_shift, Key::Z, "edit.redo"),
        kb(cmd, Key::A, "selection.all"),
        kb(cmd, Key::D, "selection.deselect"),
        // 点のグラデーション: 選んでいる点を消す。選択範囲の消去・パスの点の削除と同じキーなので、それらより前に置く（修飾の数が同じ割り当ては
        // 表の順に判定され、先に当たったものがキーを取る。点を選んでいる間は、Delete・Backspace の相手は点）
        kb(none, Key::Delete, "fill.delete_point")
            .when(When::PointSelected)
            .paint(),
        kb(none, Key::Backspace, "fill.delete_point")
            .when(When::PointSelected)
            .paint(),
        // 選択範囲があるときだけ: 消去（Delete）。キーからの画素の変更なので、ペイントのモードだけ（メニューからはどのモードでも）
        kb(none, Key::Delete, "selection.erase")
            .when(When::HasSelection)
            .paint(),
        kb(cmd, Key::Z, "edit.undo"),
        kb(cmd, Key::Y, "edit.redo"),
        kb(cmd_shift, Key::N, "layer.new"),
        kb(cmd_shift, Key::S, "file.save_as"),
        kb(cmd, Key::S, "file.save"),
        kb(cmd, Key::O, "file.open"),
        kb(cmd, Key::N, "file.new_project"),
        kb(cmd, Key::Num1, "view.ruler_snap"),
        kb(cmd, Key::Num2, "view.special_ruler_snap"),
        kb(cmd, Key::Num4, "view.switch_special_ruler"),
        kb(cmd, Key::Num0, "view.fit"),
        kb(cmd, Key::Plus, "view.zoom_in"),
        kb(cmd, Key::Equals, "view.zoom_in"),
        kb(cmd, Key::Minus, "view.zoom_out"),
        kb(cmd, Key::Q, "app.quit"),
        kb(cmd, Key::Comma, "app.settings"),
        kb(shift, Key::R, "view.reset_rotation"),
        // モードのパイ（どのモードでも。macOS でも Command ではなく Control）
        kb(Modifiers::CTRL, Key::Tab, "mode.pie"),
    ];
    // ツールのキー（ツールの表のとおり。ツールの帯の並び）
    for tool in Tool::ALL {
        if let Some((modifiers, key)) = parse_tool_key(tool.key()) {
            v.push(kb(modifiers, key, commands::tool_command(tool)).paint());
        }
    }
    v.extend([
        kb(shift, Key::Q, "selection.quick_mask").paint(),
        // パスのツール: 選んでいる点（無ければ最後の点）を消す
        kb(none, Key::Delete, "path.delete_point")
            .when(When::Tool(Tool::Path))
            .paint(),
        kb(none, Key::Backspace, "path.delete_point")
            .when(When::Tool(Tool::Path))
            .paint(),
        // パスのツール: パスの編集を抜ける（次の点は新しいパスを始める）
        kb(none, Key::Enter, "path.finish")
            .when(When::Tool(Tool::Path))
            .paint(),
        kb(none, Key::Q, "fill.toggle_handles").paint(),
        kb(none, Key::X, "color.swap").paint(),
        kb(none, Key::D, "color.default").paint(),
        kb(none, Key::OpenBracket, "brush.smaller").paint(),
        kb(none, Key::CloseBracket, "brush.larger").paint(),
        kb(none, Key::H, "view.flip"),
        kb(none, Key::Minus, "view.rotate_left"),
        // 表示を右に回す: 主の行は文字の ^（メニューの文字）。JIS の ^ のキーと US の Shift+6 は配列でキーの位置が違うので、文字で見る
        kb_text("^", "view.rotate_right"),
        kb(none, Key::Equals, "view.rotate_right"),
    ]);
    // 画面から色を取る（Windows）
    for (command, shifted) in [
        ("color.pick_screen_hidden", true),
        ("color.pick_screen", false),
    ] {
        let modifiers = Modifiers::CTRL
            | Modifiers::ALT
            | if shifted {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            };
        v.push(kb(modifiers, Key::I, command).when(When::Windows).paint());
    }
    // クリップボード（コピー・カット・ペースト。入力の受け方は `clipboard::keys` が持つが、割り当てはここ）。画素を変えるカット・ペーストは
    // ペイントのモードだけ（受け口は `shell::handle_shortcuts` が同じ決まりで止める）
    for (modifiers, key, action) in CLIPBOARD_KEYS {
        let row = kb(modifiers, key, clip_command(action));
        v.push(if changes_pixels(action) {
            row.paint()
        } else {
            row
        });
    }
    // 押している間だけ効くキー（Space: パン（Ctrl を加えると拡縮）、Y: ステンシルの置き場、N: ステンシルの一時解除）。
    // 押している間に見る修飾の条件は、読む側が持つ。`view.rotate_hold`（R を押しながらの 2D の回転）は、既定では割り当てない
    // （Alt + 左ドラッグと重なるので。操作は残してあり、割り当てれば読む側が受ける）
    v.extend([
        kb(none, Key::Space, "view.pan_hold"),
        kb(none, Key::Y, "stencil.transform_hold").paint(),
        kb(none, Key::N, "stencil.bypass_hold").paint(),
    ]);
    // 3D ビューで右ボタン（ペンのサイドボタン）を押している間の視点の移動（前後・左右・下上。Shift で速く。読み方は `FLY_KEYS`）。
    // キーの段 1「途中の操作」: 右を押している間だけ、ペイントのツールのキーより先に取る
    for fly in FLY_KEYS {
        v.push(kb(none, fly.key, fly.command).scope(Scope::During));
    }
    // 3D ビューで選んだセットを収める（3D の上で、修飾なし。ビューが読む）
    v.push(kb(none, Key::Period, "view3d.frame_selected"));
    // 編集・ポーズのモードの段: G/R/S・Alt+G/R/S（位置・回転・大きさを戻す）・Shift+Tab（スナップを常にする・しない）
    let alt = Modifiers::ALT;
    for scope in [Scope::Edit, Scope::Pose] {
        v.extend([
            kb(none, Key::G, "object.grab").scope(scope),
            kb(none, Key::R, "object.rotate").scope(scope),
            kb(none, Key::S, "object.scale").scope(scope),
            kb(alt, Key::G, "object.reset_position").scope(scope),
            kb(alt, Key::R, "object.reset_rotation").scope(scope),
            kb(alt, Key::S, "object.reset_scale").scope(scope),
            kb(shift, Key::Tab, "object.snap_toggle").scope(scope),
        ]);
    }
    // 編集のモードだけ: H・Alt+H（選んだ物の印を隠す・全部出す）・Delete（選んだ物を削除）・Q（選んだ形の取っ手を隠す・出す）
    v.extend([
        kb(none, Key::H, "object.hide").scope(Scope::Edit),
        kb(alt, Key::H, "object.reveal").scope(Scope::Edit),
        kb(none, Key::Delete, "object.delete").scope(Scope::Edit),
        kb(none, Key::Q, "fill.toggle_handles").scope(Scope::Edit),
    ]);
    // 移動・変形のツールの矢印キー（移動・変形のツールを選んでいるとき。ビューが読む）
    for n in NUDGES {
        let modifiers = if n.shift { shift } else { none };
        v.push(
            kb(modifiers, n.key, n.command)
                .when(When::Tool(Tool::Move))
                .paint(),
        );
    }
    v
}

/// クリップボードのキー（Shift 付きが先。Paste の押しは `ClipState::v_handled` で離しを 1 回見送る）。
pub const CLIPBOARD_KEYS: [(Modifiers, Key, ClipAction); 4] = [
    (
        Modifiers {
            alt: false,
            ctrl: false,
            shift: true,
            mac_cmd: false,
            command: true,
        },
        Key::C,
        ClipAction::CopyMerged,
    ),
    (Modifiers::COMMAND, Key::C, ClipAction::Copy),
    (Modifiers::COMMAND, Key::X, ClipAction::Cut),
    (Modifiers::COMMAND, Key::V, ClipAction::Paste),
];

/// キーからのクリップボードの操作が画素を変えるか（カット・ペースト。ペイントのモードだけで効く）。
pub fn changes_pixels(action: ClipAction) -> bool {
    matches!(action, ClipAction::Cut | ClipAction::Paste)
}

/// クリップボードの操作の ID。
fn clip_command(action: ClipAction) -> &'static str {
    match action {
        ClipAction::CopyMerged => "clip.copy_merged",
        ClipAction::Copy => "clip.copy",
        ClipAction::Cut => "clip.cut",
        ClipAction::Paste => "clip.paste",
    }
}

/// 押している修飾キーの数。
fn modifier_count(m: &Modifiers) -> u8 {
    u8::from(m.shift) + u8::from(m.alt) + u8::from(m.command) + u8::from(m.ctrl)
}

/// 判定の順の、キーボードの割り当て（修飾の多いものが先、同じならキーの段の上（場面・モード・どこでも）が先、それも同じなら表の順）。
/// `Action` を持たない行（押している間のキー・ビューが読むキー）とクリップボード（`clipboard::keys` が受ける）は入らない。
/// 修飾の数を段より先に見るのは、記号と数字のキーが書いていない Shift・Alt を見ないため（モードの段の `-` が、どこでもの段の Shift+- を取らない）。
fn dispatch_order_of(rows: &[KeyBinding]) -> Vec<KeyBinding> {
    let mut v: Vec<KeyBinding> = rows
        .iter()
        .filter(|b| !matches!(b.action(), None | Some(Action::Clip(_))))
        .copied()
        .collect();
    // 安定な並べ替え（修飾の数と段が同じなら表の順）
    v.sort_by_key(|b| (std::cmp::Reverse(modifier_count(&b.modifiers())), b.layer()));
    v
}

/// 今効いている割り当て: 表（一覧に出す順）と判定の順、マウスの組み合わせを 1 つにしたもの。作り直して丸ごと差し替える（`install`）ので、表と
/// 判定の順が食い違わない。
#[derive(Debug)]
pub struct Keymap {
    rows: Vec<KeyBinding>,
    order: Vec<KeyBinding>,
    gestures: Vec<Gesture>,
    /// 既定（押すと切り替え）から変えたツールのキーの動き方（操作の ID ごと）。
    tool_modes: Vec<(&'static str, ToolKeyMode)>,
}

impl Keymap {
    /// キーの表から（マウスの組み合わせは既定）。
    pub fn new(rows: Vec<KeyBinding>) -> Keymap {
        Self::with_gestures(rows, GESTURES.to_vec())
    }

    /// キーの表とマウスの組み合わせから。組み合わせは、条件の多いもの（押しながらのキー・修飾の多いもの）を先に見る（少ないものに隠れない）。
    pub fn with_gestures(rows: Vec<KeyBinding>, mut gestures: Vec<Gesture>) -> Keymap {
        let order = dispatch_order_of(&rows);
        gestures.sort_by_key(|g| {
            std::cmp::Reverse((
                g.held.is_some(),
                u8::from(g.alt) + u8::from(g.shift) + u8::from(g.ctrl),
            ))
        });
        Keymap {
            rows,
            order,
            gestures,
            tool_modes: Vec::new(),
        }
    }

    /// ツールのキーの動き方（既定から変えたもの）を入れる。
    pub fn with_tool_modes(mut self, modes: Vec<(&'static str, ToolKeyMode)>) -> Keymap {
        self.tool_modes = modes;
        self
    }

    /// ツールを選ぶ操作のキーの動き方（変えていなければ、押すと切り替え）。
    pub fn tool_mode(&self, command: &str) -> ToolKeyMode {
        self.tool_modes
            .iter()
            .find(|(id, _)| *id == command)
            .map_or(ToolKeyMode::Tap, |(_, mode)| *mode)
    }

    /// マウスの組み合わせ（判定の順。外したものは入らない）。
    pub fn gestures(&self) -> &[Gesture] {
        &self.gestures
    }

    /// 表の行（一覧に出す順。クリップボード・押している間のキー・ビューが読むキーも含む）。
    pub fn rows(&self) -> &[KeyBinding] {
        &self.rows
    }

    /// 判定の順の行（`dispatch` が見る）。
    pub fn order(&self) -> &[KeyBinding] {
        &self.order
    }

    /// この操作の行（表の順）。
    pub fn rows_of<'a>(&'a self, command: &'a str) -> impl Iterator<Item = &'a KeyBinding> {
        self.rows.iter().filter(move |b| b.command == command)
    }
}

fn slot() -> &'static RwLock<Arc<Keymap>> {
    static SLOT: OnceLock<RwLock<Arc<Keymap>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(Arc::new(Keymap::new(bindings()))))
}

thread_local! {
    /// このスレッドで効かせる割り当て（アプリがフレームの初めと、ショートカットの設定を変えたときに入れる）。入れていなければ既定の表。
    /// アプリの状態ごとに持ち、プロセスで 1 つの表を書き換えないので、同じプロセスの別のアプリ（試験の並び）の割り当てに混ざらない。
    static INSTALLED: RefCell<Option<Arc<Keymap>>> = const { RefCell::new(None) };
}

/// 既定の割り当て（ツールの表とこのファイルの表から作ったもの）。
pub fn default_map() -> Arc<Keymap> {
    slot()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// 今効いている割り当て（このスレッドに入れた表。無ければ既定の表）。
pub fn current() -> Arc<Keymap> {
    INSTALLED
        .with(|m| m.borrow().clone())
        .unwrap_or_else(default_map)
}

/// このスレッドで効かせる割り当てを入れる（ショートカットの設定が作った表）。
pub fn install(map: Arc<Keymap>) {
    INSTALLED.with(|m| *m.borrow_mut() = Some(map));
}

/// 効かせる割り当てを丸ごと差し替える（このスレッドで。表と判定の順を作り直す。マウスの組み合わせは既定）。
pub fn replace(rows: Vec<KeyBinding>) {
    install(Arc::new(Keymap::new(rows)));
}

/// 修飾キーを書いたとおりに見るキーか（文字・F キー・名前のキー）。記号と数字のキーは、配列によって Shift や Option を押して打つので、見ない。
pub fn modifiers_are_exact(key: Key) -> bool {
    use Key::*;
    matches!(
        key,
        A | B
            | C
            | D
            | E
            | F
            | G
            | H
            | I
            | J
            | K
            | L
            | M
            | N
            | O
            | P
            | Q
            | R
            | S
            | T
            | U
            | V
            | W
            | X
            | Y
            | Z
            | F1
            | F2
            | F3
            | F4
            | F5
            | F6
            | F7
            | F8
            | F9
            | F10
            | F11
            | F12
            | F13
            | F14
            | F15
            | F16
            | F17
            | F18
            | F19
            | F20
            | F21
            | F22
            | F23
            | F24
            | F25
            | F26
            | F27
            | F28
            | F29
            | F30
            | F31
            | F32
            | F33
            | F34
            | F35
            | Tab
            | Space
            | Enter
            | Escape
            | Backspace
            | Delete
            | ArrowUp
            | ArrowDown
            | ArrowLeft
            | ArrowRight
            | Home
            | End
            | PageUp
            | PageDown
            | Insert
    )
}

/// 押した修飾キーが、割り当ての修飾に当たるか。Ctrl・Command は `Modifiers::cmd_ctrl_matches`（Command の割り当ては Ctrl でも Mac の Command でも当たる）。
/// `modifiers_are_exact` のキーは Shift・Alt を書いたとおり（書いていない Shift・Alt を押していれば当たらない）、記号と数字のキーは、書いた Shift・Alt が
/// 押されていれば当たる（書いていない Shift・Alt は見ない）。
pub fn modifiers_match(pressed: &Modifiers, pattern: Modifiers, key: Key) -> bool {
    if !pressed.cmd_ctrl_matches(pattern) {
        return false;
    }
    if modifiers_are_exact(key) {
        pressed.alt == pattern.alt && pressed.shift == pattern.shift
    } else {
        (!pattern.alt || pressed.alt) && (!pattern.shift || pressed.shift)
    }
}

/// このフレームにキーを押した事象があれば取り除いて true（繰り返しも含む。修飾キーは `modifiers_match`）。
pub fn consume_key(i: &mut InputState, modifiers: Modifiers, key: Key) -> bool {
    take_key(i, modifiers, key).0
}

/// このフレームのキーの押しを取り除き、（当たったか, 繰り返しでない押しがあったか）を返す。
fn take_key(i: &mut InputState, modifiers: Modifiers, key: Key) -> (bool, bool) {
    let (mut found, mut fresh) = (false, false);
    i.events.retain(|event| {
        let Event::Key {
            key: pressed_key,
            modifiers: pressed_modifiers,
            pressed: true,
            repeat,
            ..
        } = event
        else {
            return true;
        };
        let is_match = *pressed_key == key && modifiers_match(pressed_modifiers, modifiers, key);
        found |= is_match;
        fresh |= is_match && !*repeat;
        !is_match
    });
    (found, fresh)
}

/// 操作の割り当て（効く範囲と条件を満たすもの）のどれかを押したか（押した事象は取り除く）。ビューが押しを読む操作（3D の `.`・矢印）が使う。
pub fn consume_command(i: &mut InputState, app: &AppState, command: &str) -> bool {
    let map = current();
    let mut found = false;
    for b in map.rows_of(command) {
        if let Trigger::Key { modifiers, key } = b.trigger {
            if b.scope.holds(app) && b.when.holds(app) && consume_key(i, modifiers, key) {
                found = true;
            }
        }
    }
    found
}

/// 押している間だけ効く操作のキー（表の最初のキーの行。割り当てが無ければ None）。
pub fn hold_key(command: &str) -> Option<Key> {
    current().rows_of(command).find_map(KeyBinding::key)
}

/// 押している間だけ効く操作のキーを今押しているか。
pub fn hold_down(i: &InputState, command: &str) -> bool {
    hold_key(command).is_some_and(|key| i.key_down(key))
}

/// 視点の移動キーの押し（繰り返しを含む。Shift 以外の修飾キーが無いもの。Ctrl を押した Ctrl+S などは残す）を取り除く。右ボタンを押している
/// 3D ビューの間（`flying`）は、W/A/S/D/Q/E をキーの表に渡さない（ツールの切り替え・初期設定の色・塗りつぶしの取っ手に行かない）。
/// `held` は、移動の間に押されていた移動キー: 右ボタンを先に離しても、そのキーが押されたままの間は、OS のキーの繰り返し（`repeat` の押し）が
/// 続くので、キーを離すまで取り除き続ける。離して押し直せば、今までどおり表へ届く。
pub fn take_fly_keys(i: &mut InputState, held: &mut Vec<Key>, flying: bool) {
    let keys: Vec<Key> = FLY_KEYS
        .iter()
        .filter_map(|fly| hold_key(fly.command))
        .collect();
    if flying {
        // 右ボタンを押す前から押していたキーも、移動の間は押されている
        for key in &keys {
            if i.key_down(*key) && !held.contains(key) {
                held.push(*key);
            }
        }
    }
    if !flying && held.is_empty() {
        return;
    }
    i.events.retain(|event| match event {
        Event::Key {
            key,
            modifiers,
            pressed,
            ..
        } if keys.contains(key) => {
            if !*pressed {
                // 離した: 次の押しは新しい押し
                held.retain(|k| k != key);
                return true;
            }
            let plain = !modifiers.command && !modifiers.ctrl && !modifiers.alt;
            if plain && (flying || held.contains(key)) {
                if !held.contains(key) {
                    held.push(*key);
                }
                false
            } else {
                true
            }
        }
        _ => true,
    });
    held.retain(|key| i.key_down(*key));
}

/// 操作のキー（表の最初のキーの行。画面の部品の決まった働きは、その定義のキー）。
pub fn key_of(command: &str) -> Option<Key> {
    hold_key(command).or_else(|| match commands::find(command)?.kind {
        commands::Kind::Fixed(key) => Some(key),
        _ => None,
    })
}

/// 操作の主の行（表で最初の行。メニューのキーの文字になる）。
pub fn primary(command: &str) -> Option<KeyBinding> {
    current().rows_of(command).next().copied()
}

/// そのモードで、この行のキーが操作を実行するか: 範囲がそのモードで効き、判定の順で先に同じキーを取る別の操作の行（場面の行は場面に
/// よるので数えない）が無い。メニュー・ツールチップに出すキーの文字を、今のモードの表から引くのに使う。
pub fn effective_in(map: &Keymap, row: &KeyBinding, mode: crate::mode::EditorMode) -> bool {
    if !row.scope.holds_in(mode) {
        return false;
    }
    for b in map.order() {
        if b == row {
            return true;
        }
        if b.trigger == row.trigger
            && b.command != row.command
            && b.when == When::Always
            && b.scope.holds_in(mode)
        {
            return false;
        }
    }
    true
}

/// キーの繰り返しの押しでも実行する操作か（パイを開く操作・切り替えなどは偽。利用者が作ったパイを開く操作も偽）。
pub fn repeats(command: &str) -> bool {
    pie_of(command).is_none() && commands::find(command).is_none_or(|c| c.repeats)
}

/// このフレームのキーの操作（効く範囲と条件を満たし、押されたもの。押した事象は取り除く）。文字を打っている・メニューを開いている間は呼ばない。
/// 同じ操作の行が 2 つ同時に当たっても（JIS 配列の ^ のキーは `Key::Equals` と文字の `^` が両方届く）、1 回だけ実行する。
pub fn dispatch(i: &mut InputState, app: &AppState) -> Vec<Action> {
    dispatch_with(&current(), i, app)
}

/// `dispatch` を、与えた割り当てで（試験が今効いている表を差し替えずに確かめる）。
pub fn dispatch_with(map: &Keymap, i: &mut InputState, app: &AppState) -> Vec<Action> {
    dispatch_fired(map, i, app)
        .into_iter()
        .map(|f| f.action)
        .collect()
}

/// キーで起きた操作 1 つ（`dispatch_fired`）。
#[derive(Clone, Debug, PartialEq)]
pub struct Fired {
    pub action: Action,
    /// 操作の ID。
    pub command: &'static str,
    /// 押したキー（文字の入力は None。離しを見られない）。
    pub key: Option<Key>,
    /// ツールを選ぶ操作のキーの動き方（ほかの操作は `Tap`）。`Tap` 以外は、繰り返しの押しを受けない。
    pub mode: ToolKeyMode,
}

/// `dispatch_with` の、押したキーと動き方も返す版（ツールのキーの「押している間だけ」が、離しを見るキーを知るのに使う）。
pub fn dispatch_fired(map: &Keymap, i: &mut InputState, app: &AppState) -> Vec<Fired> {
    let mut fired = Vec::new();
    let mut done: Vec<&'static str> = Vec::new();
    for b in map.order() {
        if !(b.scope.holds(app) && b.when.holds(app)) {
            continue;
        }
        let (hit, fresh) = match b.trigger {
            Trigger::Key { modifiers, key } => take_key(i, modifiers, key),
            // 文字の入力は取り除かない（文字を打つ部品は、キーの処理の前に止めてある）
            Trigger::Text(text) => {
                let hit = i
                    .events
                    .iter()
                    .any(|e| matches!(e, Event::Text(s) if s == text));
                (hit, hit)
            }
        };
        let mode = map.tool_mode(b.command);
        // 繰り返しの押しを受けない操作（パイを開く・押している間だけのツールのキー）は、繰り返しだけなら事象を取り除いて何もしない
        let hit = hit && (fresh || (mode == ToolKeyMode::Tap && repeats(b.command)));
        // 事象は当たった行ごとに取り除き、操作は 1 回目だけ実行する
        if hit && !done.contains(&b.command) {
            done.push(b.command);
            if let Some(action) = b.action() {
                fired.push(Fired {
                    action,
                    command: b.command,
                    key: b.key(),
                    mode,
                });
            }
        }
    }
    fired
}

// ───────── ビューの中のキー ─────────

/// ビューごとのキー（一覧の「2D ビュー」「3D ビュー」「ステンシル」の項目。押しながらマウスを使うものは `mouse`）。名前とキーは操作（`commands`）のもの。
#[derive(Clone, Copy, Debug)]
pub struct ContextKey {
    pub scope: &'static str,
    pub command: &'static str,
    pub mouse: bool,
}

impl ContextKey {
    pub fn label(&self, lang: Lang) -> &'static str {
        commands::find(self.command)
            .and_then(|c| c.static_label(lang))
            .unwrap_or("")
    }

    pub fn key(&self) -> Option<Key> {
        key_of(self.command)
    }
}

const fn context(scope: &'static str, command: &'static str, mouse: bool) -> ContextKey {
    ContextKey {
        scope,
        command,
        mouse,
    }
}

/// 一覧に出すビューのキー。押しながらの組み合わせで一覧に出るもの（3D の Space・ステンシルの Y）は、`GESTURES` の側に出る。
pub const CONTEXT_KEYS: [ContextKey; 14] = [
    context("canvas", "view.pan_hold", true),
    context("canvas", "canvas.cancel", false),
    context("canvas", "canvas.confirm", false),
    context("canvas", "canvas.remove_last_point", false),
    context("view3d", "view3d.cancel", false),
    context("view3d", "view3d.frame_selected", false),
    context("view3d", "view3d.fly_forward", false),
    context("view3d", "view3d.fly_back", false),
    context("view3d", "view3d.fly_left", false),
    context("view3d", "view3d.fly_right", false),
    context("view3d", "view3d.fly_down", false),
    context("view3d", "view3d.fly_up", false),
    context("stencil", "stencil.bypass_hold", false),
    context("stencil", "stencil.cancel", false),
];

// ───────── マウスと修飾キーの組み合わせ ─────────

/// マウスの組み合わせで決まる操作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    /// 3D ビューの視点を回す。
    Orbit,
    Pan,
    Zoom,
    /// 2D キャンバスの表示を回す（15° 刻み。Shift で自由）。
    Rotate,
    /// 色を取る（2D は右ボタンを押す。3D は右ボタンを動かさずに離す）。
    Pick,
    /// 3D ビューの視点を回す。軸の向き（正面・背面・右・左・上・下）の 15° 以内に入ったらその向きへ吸い付く。
    SnapOrbit,
    /// クローンのブラシで、クローンの元を決める（2D・3D とも Alt + 左を動かさずに離す。動かせば、2D は表示を回し、3D はスナップ回転）。
    CloneSource,
    SelectionAdd,
    SelectionSubtract,
    SelectionIntersect,
    MoveStencil,
    RotateStencil,
    ScaleStencil,
    /// ステンシルの回転を 15° 刻みにする（回す間の Shift）。
    SnapStencilRotation,
}

impl Operation {
    /// 全部の組み合わせ。
    pub const ALL: [Operation; 14] = [
        Self::Orbit,
        Self::Pan,
        Self::Zoom,
        Self::Rotate,
        Self::Pick,
        Self::SnapOrbit,
        Self::CloneSource,
        Self::SelectionAdd,
        Self::SelectionSubtract,
        Self::SelectionIntersect,
        Self::MoveStencil,
        Self::RotateStencil,
        Self::ScaleStencil,
        Self::SnapStencilRotation,
    ];

    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            Self::Orbit => lang.pick("回転", "Orbit"),
            Self::Pan => lang.pick("パン", "Pan"),
            Self::Zoom => lang.pick("ズーム", "Zoom"),
            Self::Rotate => lang.pick("回転", "Rotate"),
            Self::Pick => Tool::Eyedropper.name_in(lang),
            Self::SnapOrbit => lang.pick("スナップ回転", "Snap Orbit"),
            Self::CloneSource => lang.pick("クローンの元を決める", "Set Clone Source"),
            Self::SelectionAdd => lang.pick("選択範囲に追加", "Add to Selection"),
            Self::SelectionSubtract => lang.pick("選択範囲から引く", "Subtract from Selection"),
            Self::SelectionIntersect => lang.pick("選択範囲と重ねる", "Intersect with Selection"),
            Self::MoveStencil => lang.pick("ステンシルの移動", "Move Stencil"),
            Self::RotateStencil => lang.pick("ステンシルの回転", "Rotate Stencil"),
            Self::ScaleStencil => lang.pick("ステンシルの拡縮", "Scale Stencil"),
            Self::SnapStencilRotation => lang.pick(
                "ステンシルの回転を 15° 刻みに",
                "Snap Stencil Rotation to 15°",
            ),
        }
    }
}

/// マウスの組み合わせ 1 つ。`alt`・`shift`・`ctrl` は押していなければならない修飾（ほかの修飾は気にしない。上から順に最初に当たったものが効く）、
/// `held` はあるとき押している間のキーの操作の ID（`Kind::Hold`。キーは `hold_key` で引く）。
/// 修飾は押しの始め（ボタンを押した瞬間）に持っているもので決める。始めたあとに押した修飾は、始めた操作の中の修飾（`starts: false` の行）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gesture {
    /// 一覧のまとまり（選択範囲のツールの組み合わせ方は「selection」で、一覧では 2D ビューに並ぶ）。
    pub scope: &'static str,
    pub held: Option<&'static str>,
    pub button: PointerButton,
    pub alt: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub operation: Operation,
    /// この組み合わせで操作を始める（偽なら、始めたあとの修飾の効き方で、始め方の判定には使わない）。
    pub starts: bool,
    /// 動かさずに離したときの操作（真なら、同じ組み合わせで始めた操作を、押した所から動かさずに離したときの行き先）。
    pub click: bool,
    /// 効くモード（キーの段。ペイントの組み合わせ（クローンの元・選択範囲の作り方・ステンシル）はペイントのモードだけ）。一覧と設定の画面が読み、
    /// 受け口も同じ決まりで止める（クローンの元は `clone_source::active`、選択範囲は 2D のツールの押し、ステンシルは `stencil::update_keys`）。
    pub mode: Scope,
    /// 既定の表（`GESTURES`）の番号（ショートカットの設定が、変えた組み合わせをこの番号で覚える）。
    pub index: u8,
}

impl Gesture {
    /// 今のモードで、この組み合わせが効くか。
    pub fn holds(&self, app: &AppState) -> bool {
        self.mode.holds(app)
    }
}

/// ペイントのモードだけの組み合わせにする。
const fn in_paint(mut g: Gesture) -> Gesture {
    g.mode = Scope::Paint;
    g
}

const fn gesture_of(
    scope: &'static str,
    held: Option<&'static str>,
    button: PointerButton,
    (alt, shift, ctrl): (bool, bool, bool),
    operation: Operation,
) -> Gesture {
    Gesture {
        scope,
        held,
        button,
        alt,
        shift,
        ctrl,
        operation,
        starts: true,
        click: false,
        mode: Scope::Everywhere,
        index: 0,
    }
}

/// 同じ組み合わせで始めた操作を、動かさずに離したときの操作の行。
const fn click_of(
    scope: &'static str,
    button: PointerButton,
    (alt, shift, ctrl): (bool, bool, bool),
    operation: Operation,
) -> Gesture {
    Gesture {
        scope,
        held: None,
        button,
        alt,
        shift,
        ctrl,
        operation,
        starts: false,
        click: true,
        mode: Scope::Everywhere,
        index: 0,
    }
}

use PointerButton::{Middle, Primary, Secondary};

/// マウスと修飾キーの組み合わせの全部。
pub const GESTURES: [Gesture; 20] = numbered([
    // 2D キャンバス: 中ボタンでパン、Alt + 左ドラッグで表示を回す（15° 刻み。Shift で自由。動かさずに離すとクローンの元）、右ボタンを押すとスポイト
    gesture_of(
        "canvas",
        None,
        Middle,
        (false, false, false),
        Operation::Pan,
    ),
    gesture_of(
        "canvas",
        None,
        Primary,
        (true, false, false),
        Operation::Rotate,
    ),
    in_paint(click_of(
        "canvas",
        Primary,
        (true, false, false),
        Operation::CloneSource,
    )),
    gesture_of(
        "canvas",
        None,
        Secondary,
        (false, false, false),
        Operation::Pick,
    ),
    // 選択範囲のツールの作り方: Shift で足す・Ctrl で引く・両方で重ねる
    in_paint(gesture_of(
        "selection",
        None,
        Primary,
        (false, true, true),
        Operation::SelectionIntersect,
    )),
    in_paint(gesture_of(
        "selection",
        None,
        Primary,
        (false, true, false),
        Operation::SelectionAdd,
    )),
    in_paint(gesture_of(
        "selection",
        None,
        Primary,
        (false, false, true),
        Operation::SelectionSubtract,
    )),
    // 3D ビュー: 右ボタンで回す（動かさずに離すとスポイト）・中ボタンでパン・Space + 左でパン（Ctrl を足すと拡縮）・
    // Alt + 左でスナップ回転（動かさずに離すとクローンの元）
    gesture_of(
        "view3d",
        None,
        Secondary,
        (false, false, false),
        Operation::Orbit,
    ),
    click_of("view3d", Secondary, (false, false, false), Operation::Pick),
    gesture_of(
        "view3d",
        None,
        Middle,
        (false, false, false),
        Operation::Pan,
    ),
    gesture_of(
        "view3d",
        Some("view.pan_hold"),
        Primary,
        (false, false, true),
        Operation::Zoom,
    ),
    gesture_of(
        "view3d",
        Some("view.pan_hold"),
        Primary,
        (false, false, false),
        Operation::Pan,
    ),
    gesture_of(
        "view3d",
        None,
        Primary,
        (true, false, false),
        Operation::SnapOrbit,
    ),
    in_paint(click_of(
        "view3d",
        Primary,
        (true, false, false),
        Operation::CloneSource,
    )),
    // ステンシル（Y を押しながら）: 左で回す・中か Ctrl + 左で動かす・右か Alt + 左で大きさ
    in_paint(gesture_of(
        "stencil",
        Some("stencil.transform_hold"),
        Middle,
        (false, false, false),
        Operation::MoveStencil,
    )),
    in_paint(gesture_of(
        "stencil",
        Some("stencil.transform_hold"),
        Primary,
        (false, false, true),
        Operation::MoveStencil,
    )),
    in_paint(gesture_of(
        "stencil",
        Some("stencil.transform_hold"),
        Secondary,
        (false, false, false),
        Operation::ScaleStencil,
    )),
    in_paint(gesture_of(
        "stencil",
        Some("stencil.transform_hold"),
        Primary,
        (true, false, false),
        Operation::ScaleStencil,
    )),
    in_paint(gesture_of(
        "stencil",
        Some("stencil.transform_hold"),
        Primary,
        (false, false, false),
        Operation::RotateStencil,
    )),
    // 回している間の Shift は 15° 刻み（`StencilState::update_drag`）。押す順は問わないので、回す組み合わせに Shift を足した形で載せる
    Gesture {
        scope: "stencil",
        held: Some("stencil.transform_hold"),
        button: Primary,
        alt: false,
        shift: true,
        ctrl: false,
        operation: Operation::SnapStencilRotation,
        starts: false,
        click: false,
        mode: Scope::Paint,
        index: 0,
    },
]);

/// 表の並びの番号を付ける。
const fn numbered<const N: usize>(mut gestures: [Gesture; N]) -> [Gesture; N] {
    let mut i = 0;
    while i < N {
        gestures[i].index = i as u8;
        i += 1;
    }
    gestures
}

/// Ctrl（Mac の Command も）か。
fn ctrl(m: &Modifiers) -> bool {
    m.ctrl || m.command
}

/// この押しに当たる操作（`held` は、その範囲の押しながらのキーを押しているか）。表の上から順に最初に当たったもの。
pub fn gesture(scope: &str, button: PointerButton, m: &Modifiers, held: bool) -> Option<Operation> {
    gesture_where(scope, button, m, held, |_| true)
}

/// `gesture` の、引く操作を `wanted` に絞ったもの（2D の視点の行は、スポイトの行の条件の多い組み合わせに隠れない）。
pub fn gesture_where(
    scope: &str,
    button: PointerButton,
    m: &Modifiers,
    held: bool,
    wanted: impl Fn(Operation) -> bool,
) -> Option<Operation> {
    current()
        .gestures()
        .iter()
        .filter(|g| g.starts && g.scope == scope && wanted(g.operation))
        .find(|g| {
            g.button == button
                && (g.held.is_none() || held)
                && (!g.alt || m.alt)
                && (!g.shift || m.shift)
                && (!g.ctrl || ctrl(m))
        })
        .map(|g| g.operation)
}

/// 始めたあとに効く修飾の行（ステンシルの回転を 15° 刻みにする Shift など）の修飾を、今押しているか（行を外していれば、修飾が無い行なら偽）。
pub fn modifier_held(scope: &str, operation: Operation, m: &Modifiers) -> bool {
    current().gestures().iter().any(|g| {
        g.scope == scope
            && g.operation == operation
            && !g.starts
            && !g.click
            && (g.alt || g.shift || g.ctrl)
            && (!g.alt || m.alt)
            && (!g.shift || m.shift)
            && (!g.ctrl || ctrl(m))
    })
}

/// この押しが始める操作を、修飾も押しながらのキーも書いたとおりに（書いていない Shift・Alt・Ctrl・押しながらのキーがあれば当てない）引く。
/// 2D の右ボタンのスポイトのように、修飾を足した押しでは始めない操作が使う（`gesture` は書いていない修飾を気にしない）。
pub fn gesture_exact(
    scope: &str,
    button: PointerButton,
    m: &Modifiers,
    held: bool,
) -> Option<Operation> {
    exact(scope, button, m, held, |g| g.starts)
}

/// この押しを動かさずに離したときの操作（押しの始めの修飾で決める。始める組み合わせと違い、修飾は書いたとおりに（書いていない Shift・Alt・Ctrl・
/// 押しながらのキーがあれば当てない）見る: Shift を押した右クリックがスポイトになったりしない。無ければ None）。
pub fn click_gesture(
    scope: &str,
    button: PointerButton,
    m: &Modifiers,
    held: bool,
) -> Option<Operation> {
    exact(scope, button, m, held, |g| g.click)
}

fn exact(
    scope: &str,
    button: PointerButton,
    m: &Modifiers,
    held: bool,
    wanted: impl Fn(&Gesture) -> bool,
) -> Option<Operation> {
    current()
        .gestures()
        .iter()
        .filter(|g| wanted(g) && g.scope == scope)
        .find(|g| {
            g.button == button
                && g.held.is_some() == held
                && g.alt == m.alt
                && g.shift == m.shift
                && g.ctrl == ctrl(m)
        })
        .map(|g| g.operation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gesture_combination_has_a_distinct_meaning_in_its_scope() {
        for g in &GESTURES {
            // 同じ範囲・ボタン・押しながらのキー・修飾の組み合わせは 1 つだけ
            let twins = GESTURES
                .iter()
                .filter(|o| {
                    o.scope == g.scope
                        && o.held == g.held
                        && o.button == g.button
                        && (o.starts, o.click) == (g.starts, g.click)
                        && (o.alt, o.shift, o.ctrl) == (g.alt, g.shift, g.ctrl)
                })
                .count();
            assert_eq!(twins, 1, "{g:?}");
        }
    }

    #[test]
    fn a_gesture_is_never_hidden_by_an_earlier_one() {
        for (i, a) in GESTURES.iter().enumerate() {
            for b in &GESTURES[i + 1..] {
                if !(a.starts && b.starts) || a.scope != b.scope || a.button != b.button {
                    continue;
                }
                // a の条件が b の条件に含まれるなら、b に当たる入力は先に a に当たる（b が隠れる）
                let hidden = (!a.alt || b.alt)
                    && (!a.shift || b.shift)
                    && (!a.ctrl || b.ctrl)
                    && (a.held.is_none() || a.held == b.held);
                assert!(!hidden, "{b:?} が先の {a:?} に隠れる");
            }
        }
    }

    #[test]
    fn the_view_gestures_resolve_the_way_the_input_code_reads_them() {
        let none = Modifiers::NONE;
        let alt = Modifiers::ALT;
        let shift = Modifiers::SHIFT;
        let alt_shift = Modifiers::ALT | Modifiers::SHIFT;
        let ctrl = Modifiers::CTRL;
        // 3D: 右で回す・中でパン・Space + 左でパン（Ctrl を足すと拡縮）・Alt + 左でスナップ回転
        assert_eq!(
            gesture("view3d", Secondary, &none, false),
            Some(Operation::Orbit)
        );
        assert_eq!(
            gesture("view3d", Middle, &none, false),
            Some(Operation::Pan)
        );
        assert_eq!(gesture("view3d", Primary, &none, false), None);
        assert_eq!(
            gesture("view3d", Primary, &none, true),
            Some(Operation::Pan)
        );
        assert_eq!(
            gesture("view3d", Primary, &ctrl, true),
            Some(Operation::Zoom)
        );
        assert_eq!(gesture("view3d", Primary, &ctrl, false), None);
        assert_eq!(
            gesture("view3d", Primary, &alt, false),
            Some(Operation::SnapOrbit)
        );
        // 外した組み合わせ: Shift + 右は、パンではなく回す（Shift は気にしない）。Alt + Shift + 左も、パンではなくスナップ回転
        assert_eq!(
            gesture("view3d", Secondary, &shift, false),
            Some(Operation::Orbit)
        );
        assert_eq!(
            gesture("view3d", Primary, &alt_shift, false),
            Some(Operation::SnapOrbit)
        );
        // Space を押していても、右・中ボタンは同じ
        assert_eq!(
            gesture("view3d", Secondary, &none, true),
            Some(Operation::Orbit)
        );
        // 動かさずに離したとき: 右はスポイト、Alt + 左はクローンの元。修飾は書いたとおり（Shift を足すとどちらでもない）
        assert_eq!(
            click_gesture("view3d", Secondary, &none, false),
            Some(Operation::Pick)
        );
        assert_eq!(
            click_gesture("view3d", Primary, &alt, false),
            Some(Operation::CloneSource)
        );
        assert_eq!(click_gesture("view3d", Secondary, &shift, false), None);
        assert_eq!(click_gesture("view3d", Primary, &alt_shift, false), None);
        assert_eq!(click_gesture("view3d", Secondary, &ctrl, false), None);
        assert_eq!(click_gesture("view3d", Secondary, &none, true), None);
        assert_eq!(click_gesture("view3d", Primary, &alt, true), None);
        assert_eq!(click_gesture("view3d", Primary, &none, false), None);
        assert_eq!(click_gesture("view3d", Middle, &none, false), None);
        // 2D の右ボタンのスポイトは、修飾も書いたとおり（Shift・Alt・Ctrl・Space を足すと始めない）
        assert_eq!(
            gesture_exact("canvas", Secondary, &none, false),
            Some(Operation::Pick)
        );
        assert_eq!(gesture_exact("canvas", Secondary, &shift, false), None);
        assert_eq!(gesture_exact("canvas", Secondary, &alt, false), None);
        assert_eq!(gesture_exact("canvas", Secondary, &ctrl, false), None);
        assert_eq!(gesture_exact("canvas", Secondary, &none, true), None);
        assert_eq!(
            gesture_exact("canvas", Primary, &alt, false),
            Some(Operation::Rotate)
        );
        assert_eq!(gesture_exact("canvas", Primary, &alt_shift, false), None);
        // 2D: 中でパン・Alt + 左で回す・右でスポイト。Shift + 中は回さない（パン）
        assert_eq!(
            gesture("canvas", Middle, &none, false),
            Some(Operation::Pan)
        );
        assert_eq!(
            gesture("canvas", Middle, &shift, false),
            Some(Operation::Pan)
        );
        assert_eq!(
            gesture("canvas", Primary, &alt, false),
            Some(Operation::Rotate)
        );
        assert_eq!(gesture("canvas", Primary, &none, false), None);
        assert_eq!(
            gesture("canvas", Secondary, &none, false),
            Some(Operation::Pick)
        );
        // 2D の Alt + 左は、描くツールのスポイトではない（どの組み合わせも Pick を左ボタンに割り当てない）
        assert!(GESTURES
            .iter()
            .all(|g| !(g.operation == Operation::Pick && g.button == Primary)));
        // 選択範囲の作り方
        assert_eq!(gesture("selection", Primary, &none, false), None);
        assert_eq!(
            gesture("selection", Primary, &shift, false),
            Some(Operation::SelectionAdd)
        );
        assert_eq!(
            gesture("selection", Primary, &ctrl, false),
            Some(Operation::SelectionSubtract)
        );
        assert_eq!(
            gesture("selection", Primary, &(ctrl | shift), false),
            Some(Operation::SelectionIntersect)
        );
        // ステンシル
        assert_eq!(
            gesture("stencil", Primary, &none, true),
            Some(Operation::RotateStencil)
        );
        assert_eq!(
            gesture("stencil", Primary, &shift, true),
            Some(Operation::RotateStencil),
            "Shift は始め方を変えない"
        );
        assert_eq!(
            gesture("stencil", Primary, &ctrl, true),
            Some(Operation::MoveStencil)
        );
        assert_eq!(
            gesture("stencil", Primary, &alt, true),
            Some(Operation::ScaleStencil)
        );
        assert_eq!(
            gesture("stencil", Middle, &none, true),
            Some(Operation::MoveStencil)
        );
        assert_eq!(
            gesture("stencil", Secondary, &none, true),
            Some(Operation::ScaleStencil)
        );
    }

    /// 1 つのキーの押しを、実際の判定（`dispatch`）へ流す。
    fn dispatched(app: &AppState, key: Key, modifiers: Modifiers) -> Vec<Action> {
        let ctx = egui::Context::default();
        let mut got = Vec::new();
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
        let mut output = ctx.run_ui(input, |ui| {
            ui.input_mut(|i| got = dispatch(i, app));
        });
        output.textures_delta.clear();
        got
    }

    #[test]
    fn modifiers_are_exact_for_letters_function_keys_and_named_keys_but_not_for_symbols_and_digits()
    {
        for key in Key::ALL {
            let name = key.name();
            let letter = name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic();
            let function = name.len() >= 2
                && name.starts_with('F')
                && name[1..].bytes().all(|b| b.is_ascii_digit());
            let named = matches!(
                key,
                Key::Tab
                    | Key::Space
                    | Key::Enter
                    | Key::Escape
                    | Key::Backspace
                    | Key::Delete
                    | Key::ArrowUp
                    | Key::ArrowDown
                    | Key::ArrowLeft
                    | Key::ArrowRight
                    | Key::Home
                    | Key::End
                    | Key::PageUp
                    | Key::PageDown
                    | Key::Insert
            );
            assert_eq!(
                modifiers_are_exact(*key),
                letter || function || named,
                "{key:?}"
            );
        }
        // 数字は上の段を Shift で打つ配列（AZERTY）があるので、記号と同じ扱い
        for key in [
            Key::Num0,
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
            Key::Minus,
            Key::Equals,
            Key::Plus,
            Key::Comma,
            Key::Period,
            Key::Slash,
            Key::Backslash,
            Key::Semicolon,
            Key::Colon,
            Key::Quote,
            Key::Backtick,
            Key::OpenBracket,
            Key::CloseBracket,
            Key::Pipe,
            Key::Questionmark,
            Key::Exclamationmark,
        ] {
            assert!(!modifiers_are_exact(key), "{key:?}");
        }
    }

    #[test]
    fn modifiers_match_exactly_for_letters_and_by_what_is_written_for_symbols_and_digits() {
        let none = Modifiers::NONE;
        // 書いていない Shift・Alt を押していれば、文字のキーは当たらない
        assert!(modifiers_match(&none, none, Key::B));
        assert!(!modifiers_match(&Modifiers::SHIFT, none, Key::B));
        assert!(!modifiers_match(&Modifiers::ALT, none, Key::B));
        assert!(modifiers_match(&Modifiers::SHIFT, Modifiers::SHIFT, Key::G));
        assert!(!modifiers_match(&none, Modifiers::SHIFT, Key::G));
        // 書いていない Ctrl は当たらない・Command の割り当ては Ctrl でも Mac の Command でも当たる
        assert!(!modifiers_match(&Modifiers::CTRL, none, Key::B));
        assert!(modifiers_match(
            &(Modifiers::CTRL | Modifiers::COMMAND),
            Modifiers::COMMAND,
            Key::S
        ));
        assert!(modifiers_match(
            &(Modifiers::MAC_CMD | Modifiers::COMMAND),
            Modifiers::COMMAND,
            Key::S
        ));
        // 記号と数字のキーは、書いていない Shift・Alt を押していても当たる
        for key in [Key::Equals, Key::Minus, Key::OpenBracket, Key::Num4] {
            assert!(modifiers_match(&Modifiers::SHIFT, none, key), "{key:?}");
            assert!(modifiers_match(&Modifiers::ALT, none, key), "{key:?}");
            assert!(
                modifiers_match(&(Modifiers::ALT | Modifiers::SHIFT), none, key),
                "{key:?}"
            );
        }
        // 行に書いてあれば、押していなければならない
        assert!(!modifiers_match(&none, Modifiers::ALT, Key::Equals));
        assert!(modifiers_match(
            &Modifiers::ALT,
            Modifiers::ALT,
            Key::Equals
        ));
        assert!(!modifiers_match(&none, Modifiers::SHIFT, Key::Num4));
        // Ctrl の扱いは記号・数字でも変わらない（Ctrl+0 は Ctrl を押したときだけ）
        assert!(modifiers_match(
            &(Modifiers::COMMAND | Modifiers::SHIFT),
            Modifiers::COMMAND,
            Key::Plus
        ));
        assert!(modifiers_match(
            &(Modifiers::CTRL | Modifiers::COMMAND),
            Modifiers::COMMAND,
            Key::Num0
        ));
        assert!(!modifiers_match(&none, Modifiers::COMMAND, Key::Num0));
        assert!(!modifiers_match(&Modifiers::CTRL, none, Key::Num0));
        // 名前のキー（矢印・Delete など）は Shift を書いたとおりに見る
        assert!(!modifiers_match(&Modifiers::SHIFT, none, Key::ArrowLeft));
        assert!(modifiers_match(
            &Modifiers::SHIFT,
            Modifiers::SHIFT,
            Key::ArrowLeft
        ));
        assert!(!modifiers_match(&Modifiers::SHIFT, none, Key::Delete));
        assert!(!modifiers_match(&Modifiers::ALT, none, Key::Delete));
    }

    #[test]
    fn extra_shift_or_alt_no_longer_reaches_a_plain_letter_key() {
        let app = AppState::new(32, 32);
        let brush = vec![Action::SelectTool(Tool::Brush)];
        assert_eq!(dispatched(&app, Key::B, Modifiers::NONE), brush);
        assert_eq!(dispatched(&app, Key::B, Modifiers::SHIFT), vec![]);
        assert_eq!(dispatched(&app, Key::B, Modifiers::ALT), vec![]);
        assert_eq!(
            dispatched(&app, Key::B, Modifiers::ALT | Modifiers::SHIFT),
            vec![]
        );
        // Alt+G はバケツでもグラデーションでもない・Shift+G はグラデーション
        assert_eq!(dispatched(&app, Key::G, Modifiers::ALT), vec![]);
        assert_eq!(
            dispatched(&app, Key::G, Modifiers::NONE),
            vec![Action::SelectTool(Tool::Fill)]
        );
        assert_eq!(
            dispatched(&app, Key::G, Modifiers::SHIFT),
            vec![Action::SelectTool(Tool::Gradient)]
        );
        // 修飾なしのキー X・D・[ も同じ
        assert_eq!(dispatched(&app, Key::X, Modifiers::SHIFT), vec![]);
        assert_eq!(dispatched(&app, Key::D, Modifiers::ALT), vec![]);
        // Ctrl 付きの割り当ては Alt が増えると当たらない・Ctrl+Shift+Z はやり直し
        assert_eq!(
            dispatched(&app, Key::Z, Modifiers::COMMAND),
            vec![Action::Undo]
        );
        assert_eq!(
            dispatched(&app, Key::Z, Modifiers::COMMAND | Modifiers::ALT),
            vec![]
        );
        assert_eq!(
            dispatched(&app, Key::Z, Modifiers::COMMAND | Modifiers::SHIFT),
            vec![Action::Redo]
        );
    }

    #[test]
    fn symbol_and_digit_keys_do_not_look_at_the_shift_or_option_a_layout_needs_to_type_them() {
        let app = AppState::new(32, 32);
        // US 配列の Ctrl+Shift+= は、論理キー + の Plus に Shift が付いて届く。Equals で届く配列もある。どちらもズームイン
        for key in [Key::Plus, Key::Equals] {
            assert_eq!(
                dispatched(&app, key, Modifiers::COMMAND | Modifiers::SHIFT),
                vec![Action::ZoomIn],
                "{key:?}"
            );
            assert_eq!(
                dispatched(&app, key, Modifiers::COMMAND),
                vec![Action::ZoomIn],
                "{key:?}"
            );
        }
        // = を Shift で打つ配列（JIS の Shift+-、ドイツ語の Shift+0）は Equals に Shift が付いて届く。表示を右に回す
        assert_eq!(
            dispatched(&app, Key::Equals, Modifiers::SHIFT),
            vec![Action::RotateRight]
        );
        assert_eq!(
            dispatched(&app, Key::Minus, Modifiers::SHIFT),
            vec![Action::RotateLeft]
        );
        // US 配列の Shift+= は論理キー + の Plus で届く。Ctrl の無い Plus の割り当ては無いので、何も起きない
        assert_eq!(dispatched(&app, Key::Plus, Modifiers::SHIFT), vec![]);
        // macOS のドイツ語配列の [ ] は Option+5・Option+6 で届く（Option の効果が論理キーに入る）。ブラシの大きさが替わる
        assert_eq!(
            dispatched(&app, Key::OpenBracket, Modifiers::ALT),
            vec![Action::BrushSmaller]
        );
        assert_eq!(
            dispatched(&app, Key::CloseBracket, Modifiers::ALT),
            vec![Action::BrushLarger]
        );
        assert_eq!(
            dispatched(&app, Key::Equals, Modifiers::ALT),
            vec![Action::RotateRight]
        );
        // AZERTY 配列は上の段の数字を Shift で打つ。ポリゴン塗りつぶし（4）が効く
        assert_eq!(
            dispatched(&app, Key::Num4, Modifiers::NONE),
            vec![Action::SelectTool(Tool::PolygonFill)]
        );
        assert_eq!(
            dispatched(&app, Key::Num4, Modifiers::SHIFT),
            vec![Action::SelectTool(Tool::PolygonFill)]
        );
        // Ctrl 付きの数字の割り当ては今までどおり（Ctrl を押したときだけ）
        assert_eq!(
            dispatched(&app, Key::Num0, Modifiers::COMMAND),
            vec![Action::FitView]
        );
        assert_eq!(
            dispatched(&app, Key::Num1, Modifiers::COMMAND),
            vec![Action::Ruler(crate::rulers::RulerAction::ToggleSnapRuler)]
        );
        assert_eq!(dispatched(&app, Key::Num0, Modifiers::NONE), vec![]);
        // 文字のキーは Option・Shift を加えると効かない（記号・数字とは違う）
        assert_eq!(dispatched(&app, Key::X, Modifiers::ALT), vec![]);
    }

    /// 与えた事象を `dispatch` に流して、実行する操作と、判定のあとに残った事象を返す。
    fn dispatched_events(app: &AppState, events: Vec<Event>) -> (Vec<Action>, Vec<Event>) {
        let ctx = egui::Context::default();
        let mut got = Vec::new();
        let mut left = Vec::new();
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            ui.input_mut(|i| {
                got = dispatch(i, app);
                left = i.events.clone();
            });
        });
        output.textures_delta.clear();
        (got, left)
    }

    #[test]
    fn one_operation_runs_once_per_frame_even_when_two_of_its_rows_match() {
        let app = AppState::new(32, 32);
        let key = |key, modifiers| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        // JIS 配列の ^ のキーは、キーの Equals と文字の ^ が両方届く。表示を右に回すのは 1 回だけ
        let (got, left) = dispatched_events(
            &app,
            vec![key(Key::Equals, Modifiers::NONE), Event::Text("^".into())],
        );
        assert_eq!(got, vec![Action::RotateRight]);
        // 操作を捨てても、当たった事象は取り除いたまま（キーの事象は残らない）
        assert!(
            left.iter().all(|e| !matches!(e, Event::Key { .. })),
            "{left:?}"
        );
        // 文字だけ・キーだけでも 1 回
        let (got, _) = dispatched_events(&app, vec![Event::Text("^".into())]);
        assert_eq!(got, vec![Action::RotateRight]);
        let (got, _) = dispatched_events(&app, vec![key(Key::Equals, Modifiers::NONE)]);
        assert_eq!(got, vec![Action::RotateRight]);
        // やり直しの 2 つのキーを同じフレームで押しても 1 回
        let (got, left) = dispatched_events(
            &app,
            vec![
                key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT),
                key(Key::Y, Modifiers::COMMAND),
            ],
        );
        assert_eq!(got, vec![Action::Redo]);
        assert!(
            left.iter().all(|e| !matches!(e, Event::Key { .. })),
            "{left:?}"
        );
        // 別の操作は、同じフレームで両方実行する
        let (got, _) = dispatched_events(
            &app,
            vec![key(Key::X, Modifiers::NONE), key(Key::D, Modifiers::NONE)],
        );
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn the_caret_text_rotates_right_like_the_equals_key() {
        let app = AppState::new(32, 32);
        let ctx = egui::Context::default();
        let mut got = Vec::new();
        let input = egui::RawInput {
            events: vec![Event::Text("^".into())],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            ui.input_mut(|i| got = dispatch(i, &app));
        });
        output.textures_delta.clear();
        assert_eq!(got, vec![Action::RotateRight]);
        // 別の文字は当たらない
        let mut got = Vec::new();
        let input = egui::RawInput {
            events: vec![Event::Text("6".into())],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            ui.input_mut(|i| got = dispatch(i, &app));
        });
        output.textures_delta.clear();
        assert!(got.is_empty());
    }

    #[test]
    fn consume_key_removes_the_matching_events_only() {
        let ctx = egui::Context::default();
        let key_event = |key, modifiers| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let input = egui::RawInput {
            events: vec![
                key_event(Key::A, Modifiers::SHIFT),
                key_event(Key::A, Modifiers::NONE),
                key_event(Key::A, Modifiers::NONE),
            ],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            ui.input_mut(|i| {
                assert!(consume_key(i, Modifiers::NONE, Key::A));
                // Shift 付きの 1 つだけが残る
                assert_eq!(
                    i.events
                        .iter()
                        .filter(|e| matches!(e, Event::Key { .. }))
                        .count(),
                    1
                );
                assert!(!consume_key(i, Modifiers::NONE, Key::A));
                assert!(consume_key(i, Modifiers::SHIFT, Key::A));
            });
        });
        output.textures_delta.clear();
    }

    #[test]
    fn rows_are_judged_with_more_modifiers_first_and_keep_the_table_order_otherwise() {
        let map = current();
        let order = map.order();
        for pair in order.windows(2) {
            assert!(
                modifier_count(&pair[0].modifiers()) >= modifier_count(&pair[1].modifiers()),
                "{:?} が {:?} より先",
                pair[0],
                pair[1]
            );
        }
        // 同じ修飾の行は表の順（点を選んでいる間の Delete は、選択範囲の消去・パスの点の削除より先）
        let at = |command: &str, when: When| {
            order
                .iter()
                .position(|b| {
                    b.command == command && b.when == when && b.key() == Some(Key::Delete)
                })
                .unwrap_or_else(|| panic!("{command}"))
        };
        assert!(
            at("fill.delete_point", When::PointSelected)
                < at("selection.erase", When::HasSelection)
        );
        assert!(
            at("fill.delete_point", When::PointSelected)
                < at("path.delete_point", When::Tool(Tool::Path))
        );
    }

    #[test]
    fn tool_keys_come_from_the_tool_table_and_every_tool_with_a_key_has_one_binding() {
        let all = bindings();
        for tool in Tool::ALL {
            let found: Vec<_> = all
                .iter()
                .filter(|b| b.command == commands::tool_command(tool))
                .collect();
            if tool.key().is_empty() {
                assert!(found.is_empty(), "{tool:?}");
                continue;
            }
            assert_eq!(found.len(), 1, "{tool:?}");
            let (m, key) = parse_tool_key(tool.key()).expect("読める");
            assert_eq!(
                found[0].trigger,
                Trigger::Key { modifiers: m, key },
                "{tool:?}"
            );
        }
        assert_eq!(parse_tool_key("Shift+G"), Some((Modifiers::SHIFT, Key::G)));
        assert_eq!(parse_tool_key("4"), Some((Modifiers::NONE, Key::Num4)));
        assert_eq!(parse_tool_key(""), None);
        assert_eq!(parse_tool_key("Meta+G"), None);
    }

    #[test]
    fn the_same_input_is_never_bound_twice_under_the_same_condition_and_scope() {
        let all = bindings();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                if a.trigger == b.trigger && a.when == b.when && a.scope == b.scope {
                    panic!(
                        "重なった割り当て: {:?} {:?} {:?}: {} / {}",
                        a.trigger, a.when, a.scope, a.command, b.command
                    );
                }
            }
        }
    }

    #[test]
    fn rows_belong_to_the_scope_the_design_gives_them() {
        for b in bindings() {
            // キーからの画素の変更（選択範囲の消去・カット・ペースト・結合）もペイントだけ
            let paint = b.command.starts_with("tool.")
                || b.command.starts_with("color.")
                || b.command.starts_with("brush.")
                || b.command.starts_with("stencil.")
                || b.command.starts_with("path.")
                || b.command.starts_with("transform.nudge_")
                || matches!(
                    b.command,
                    "fill.toggle_handles"
                        | "fill.delete_point"
                        | "selection.quick_mask"
                        | "selection.erase"
                        | "clip.cut"
                        | "clip.paste"
                        | "layer.merge_down"
                        | "layer.merge_visible"
                );
            let during = b.command.starts_with("view3d.fly_");
            // 編集・ポーズの段の行（同じ操作をそれぞれの段に置く）。Q は、ペイントと編集の両方の段
            if b.command.starts_with("object.")
                || (b.command == "fill.toggle_handles" && b.scope == Scope::Edit)
            {
                let both = !matches!(
                    b.command,
                    "object.hide" | "object.reveal" | "object.delete" | "fill.toggle_handles"
                );
                assert!(
                    b.scope == Scope::Edit || (both && b.scope == Scope::Pose),
                    "{} {:?}",
                    b.command,
                    b.scope
                );
                continue;
            }
            assert_eq!(
                b.scope,
                if during {
                    Scope::During
                } else if paint {
                    Scope::Paint
                } else {
                    Scope::Everywhere
                },
                "{}",
                b.command
            );
        }
        // G/R/S・Alt+G/R/S・Shift+Tab は編集とポーズの両方の段、H・Alt+H・Delete は編集だけ
        for (command, scopes) in [
            ("object.grab", &[Scope::Edit, Scope::Pose][..]),
            ("object.rotate", &[Scope::Edit, Scope::Pose]),
            ("object.scale", &[Scope::Edit, Scope::Pose]),
            ("object.reset_position", &[Scope::Edit, Scope::Pose]),
            ("object.reset_rotation", &[Scope::Edit, Scope::Pose]),
            ("object.reset_scale", &[Scope::Edit, Scope::Pose]),
            ("object.snap_toggle", &[Scope::Edit, Scope::Pose]),
            ("object.hide", &[Scope::Edit]),
            ("object.reveal", &[Scope::Edit]),
            ("object.delete", &[Scope::Edit]),
            ("fill.toggle_handles", &[Scope::Paint, Scope::Edit]),
        ] {
            let got: Vec<Scope> = current().rows_of(command).map(|b| b.scope).collect();
            assert_eq!(got, scopes, "{command}");
        }
        // 3D の視点の移動は、キーの段 1「途中の操作」（右を押している間だけ）。押している間の Space と 3D の . は、どのモードでも。
        // Y・N（ステンシル）はペイントだけ
        for fly in FLY_KEYS {
            assert_eq!(
                primary(fly.command).map(|b| b.scope),
                Some(Scope::During),
                "{}",
                fly.command
            );
        }
        assert_eq!(
            primary("view.pan_hold").map(|b| b.scope),
            Some(Scope::Everywhere)
        );
        assert_eq!(
            primary("view3d.frame_selected").map(|b| b.scope),
            Some(Scope::Everywhere)
        );
        assert_eq!(
            primary("stencil.transform_hold").map(|b| b.scope),
            Some(Scope::Paint)
        );
        assert_eq!(
            primary("stencil.bypass_hold").map(|b| b.scope),
            Some(Scope::Paint)
        );
        // モードのパイ（Ctrl+Tab）はどのモードでも。macOS でも Command ではなく Control
        let pie = primary("mode.pie").expect("モードのパイの行");
        assert_eq!(pie.scope, Scope::Everywhere);
        assert_eq!(
            pie.trigger,
            Trigger::Key {
                modifiers: Modifiers::CTRL,
                key: Key::Tab
            }
        );
        // 範囲はモードで決まる。途中の操作の行は `dispatch` では効かない（その操作が読む）
        use crate::mode::EditorMode;
        let mut app = AppState::new(32, 32);
        for mode in EditorMode::ALL {
            app.mode = mode;
            assert!(Scope::Everywhere.holds(&app));
            assert_eq!(Scope::Paint.holds(&app), mode == EditorMode::Paint);
            assert_eq!(Scope::Edit.holds(&app), mode == EditorMode::Edit);
            assert_eq!(Scope::Pose.holds(&app), mode == EditorMode::Pose);
            assert!(!Scope::During.holds(&app));
        }
    }

    #[test]
    fn every_row_belongs_to_one_key_layer() {
        for b in bindings() {
            let expected = if b.scope == Scope::During {
                Layer::During
            } else if b.when != When::Always {
                Layer::Context
            } else if matches!(b.scope, Scope::Paint | Scope::Edit | Scope::Pose) {
                Layer::Mode
            } else {
                Layer::Everywhere
            };
            assert_eq!(b.layer(), expected, "{}", b.command);
        }
        // 段の順: 途中の操作 → 場面 → モード → どこでも
        assert!(Layer::During < Layer::Context);
        assert!(Layer::Context < Layer::Mode);
        assert!(Layer::Mode < Layer::Everywhere);
        // 途中の操作の行は判定の順に入らない（その操作が読む）
        assert!(current().order().iter().all(|b| b.layer() != Layer::During));
        assert_eq!(
            bindings()
                .iter()
                .filter(|b| b.layer() == Layer::During)
                .map(|b| b.command)
                .collect::<Vec<_>>(),
            FLY_KEYS.iter().map(|f| f.command).collect::<Vec<_>>()
        );
    }

    /// 1 つのキーの押しを、与えた割り当て（`dispatch_with`）へ流す。
    fn dispatched_with(
        map: &Keymap,
        app: &AppState,
        key: Key,
        modifiers: Modifiers,
    ) -> Vec<Action> {
        let ctx = egui::Context::default();
        let mut got = Vec::new();
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
        let mut output = ctx.run_ui(input, |ui| {
            ui.input_mut(|i| got = dispatch_with(map, i, app));
        });
        output.textures_delta.clear();
        got
    }

    #[test]
    fn paint_keys_work_only_in_paint_mode_and_everywhere_keys_work_in_every_mode() {
        use crate::mode::EditorMode;
        let mut app = AppState::new(32, 32);
        app.apply(Action::Pose(crate::view3d::pose::PoseAction::LoadFigure));
        let map = current();
        for mode in EditorMode::ALL {
            assert!(app.set_mode(mode), "{mode:?}: {}", app.message);
            let paints = mode == EditorMode::Paint;
            // ペイントの段: ツールのキー・X・D・[ ]
            for (key, action) in [
                (Key::E, Action::SelectTool(Tool::Eraser)),
                (Key::X, Action::SwapColors),
                (Key::D, Action::DefaultColors),
                (Key::OpenBracket, Action::BrushSmaller),
            ] {
                assert_eq!(
                    dispatched_with(&map, &app, key, Modifiers::NONE),
                    if paints { vec![action] } else { vec![] },
                    "{mode:?} {key:?}"
                );
            }
            // どこでもの段: 保存・取り消し・モードのパイ
            assert_eq!(
                dispatched_with(&map, &app, Key::S, Modifiers::COMMAND),
                vec![Action::SaveProject],
                "{mode:?}"
            );
            assert_eq!(
                dispatched_with(&map, &app, Key::Z, Modifiers::COMMAND),
                vec![Action::Undo],
                "{mode:?}"
            );
            assert_eq!(
                dispatched_with(&map, &app, Key::Tab, Modifiers::CTRL),
                vec![Action::Pie(crate::pie::PieAction::Open("mode".into()))],
                "{mode:?}"
            );
            // Ctrl に Shift を加えた Tab・Ctrl の無い Tab は、モードのパイではない
            assert!(
                dispatched_with(&map, &app, Key::Tab, Modifiers::CTRL | Modifiers::SHIFT)
                    .is_empty()
            );
            assert!(dispatched_with(&map, &app, Key::Tab, Modifiers::NONE).is_empty());
        }
    }

    #[test]
    fn the_mode_pie_opens_on_the_real_control_key_of_each_platform_but_not_on_a_repeat() {
        let app = AppState::new(32, 32);
        let map = current();
        let open = vec![Action::Pie(crate::pie::PieAction::Open("mode".into()))];
        // Windows・Linux の Ctrl は ctrl と command の両方が立って届く。macOS の Control は ctrl だけ
        let ctrl = Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        };
        let mac_control = Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        };
        assert_eq!(dispatched_with(&map, &app, Key::Tab, ctrl), open);
        assert_eq!(dispatched_with(&map, &app, Key::Tab, mac_control), open);
        // macOS の Command+Tab（OS のアプリの切り替え）は当たらない
        let mac_command = Modifiers {
            mac_cmd: true,
            command: true,
            ..Modifiers::NONE
        };
        assert!(dispatched_with(&map, &app, Key::Tab, mac_command).is_empty());
        // 押したままのキーの繰り返しでは開かない（事象は取り除く）。ほかの操作の繰り返しは今までどおり効く。egui は、押したままのキーの
        // 2 回目からの押しを繰り返しにするので、同じ入れ物で押してから続ける
        let ctx = egui::Context::default();
        let frame = |key, modifiers, repeat| {
            let mut got = Vec::new();
            let mut left = Vec::new();
            let input = egui::RawInput {
                events: vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat,
                    modifiers,
                }],
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                ui.input_mut(|i| {
                    got = dispatch(i, &app);
                    left = i.events.clone();
                });
            });
            output.textures_delta.clear();
            (got, left)
        };
        assert_eq!(frame(Key::Tab, ctrl, false).0, open);
        for _ in 0..3 {
            let (got, left) = frame(Key::Tab, ctrl, true);
            assert!(got.is_empty(), "{got:?}");
            assert!(left.iter().all(|e| !matches!(e, Event::Key { .. })));
        }
        assert_eq!(
            frame(Key::OpenBracket, Modifiers::NONE, false).0,
            vec![Action::BrushSmaller]
        );
        assert_eq!(
            frame(Key::OpenBracket, Modifiers::NONE, true).0,
            vec![Action::BrushSmaller]
        );
        assert!(!commands::find("mode.pie").unwrap().repeats);
        assert!(!commands::find("view3d.pie").unwrap().repeats);
        // 繰り返しを受けないのは、パイを開く操作・G/R/S を始める操作（ポップアップを開く操作。閉じたあと、押したままのキーで開き直さない）と、
        // 切り替え・消す操作（押したままで出たり消えたり、続けて消したりしない）、ベイクのウィンドウを開く操作（開き直してウィンドウの状態を戻さない）
        let once: Vec<&str> = commands::all()
            .iter()
            .filter(|c| !c.repeats)
            .map(|c| c.id)
            .collect();
        assert_eq!(
            once,
            [
                "bake.open",
                "fill.toggle_handles",
                "mode.pie",
                "view3d.pie",
                "object.grab",
                "object.rotate",
                "object.scale",
                "object.snap_toggle",
                "object.hide",
                "object.reveal",
                "object.delete",
                "view3d.ortho",
            ]
        );
    }

    #[test]
    fn paint_combinations_belong_to_paint_mode_and_the_view_ones_to_every_mode() {
        use crate::mode::EditorMode;
        for g in &GESTURES {
            let paint = matches!(
                g.operation,
                Operation::CloneSource
                    | Operation::SelectionAdd
                    | Operation::SelectionSubtract
                    | Operation::SelectionIntersect
                    | Operation::MoveStencil
                    | Operation::RotateStencil
                    | Operation::ScaleStencil
                    | Operation::SnapStencilRotation
            );
            assert_eq!(
                g.mode,
                if paint {
                    Scope::Paint
                } else {
                    Scope::Everywhere
                },
                "{g:?}"
            );
        }
        // 2D と 3D のクローンの元（Alt＋左を動かさずに離す）は、ペイントのモードだけ
        let clone: Vec<&Gesture> = GESTURES
            .iter()
            .filter(|g| g.operation == Operation::CloneSource)
            .collect();
        assert_eq!(
            clone.iter().map(|g| g.scope).collect::<Vec<_>>(),
            ["canvas", "view3d"]
        );
        let mut app = AppState::new(32, 32);
        app.m2.brush.effect = crate::engine::BrushEffect::Clone {
            offset: Default::default(),
        };
        for mode in [EditorMode::Paint, EditorMode::Edit] {
            app.set_mode(mode);
            for g in &clone {
                assert_eq!(g.holds(&app), mode.paints(), "{mode:?} {}", g.scope);
            }
            // 受け口も同じ決まり
            assert_eq!(crate::clone_source::active(&app), mode.paints(), "{mode:?}");
        }
    }

    #[test]
    fn a_mode_row_takes_its_key_before_an_everywhere_row_and_only_in_its_mode() {
        use crate::mode::EditorMode;
        use crate::objects::{Kind as K, ObjectAction as O};
        // 編集の段の H（印を隠す）は、どこでもの段の H（表示を左右反転）より先。ポーズの段には H が無いので、どこでもの段へ落ちる
        let map = current();
        let mut app = AppState::new(32, 32);
        app.apply(Action::Pose(crate::view3d::pose::PoseAction::LoadFigure));
        let h = |app: &AppState| dispatched_with(&map, app, Key::H, Modifiers::NONE);
        assert_eq!(
            h(&app),
            vec![Action::FlipView],
            "ペイントのモードはどこでもの段"
        );
        assert!(app.set_mode(EditorMode::Edit));
        assert_eq!(
            h(&app),
            vec![Action::Object(O::Hide)],
            "編集のモードの段が先に取り、どこでもの段へは落ちない"
        );
        assert!(app.set_mode(EditorMode::Pose));
        assert_eq!(h(&app), vec![Action::FlipView]);
        // G・R・S は、ペイントではツール（G はバケツ・S は選択ペン）、編集・ポーズでは G/R/S
        let key = |app: &AppState, k| dispatched_with(&map, app, k, Modifiers::NONE);
        for mode in [EditorMode::Edit, EditorMode::Pose] {
            assert!(app.set_mode(mode));
            assert_eq!(
                key(&app, Key::G),
                vec![Action::Object(O::Transform(K::Grab))]
            );
            assert_eq!(
                key(&app, Key::R),
                vec![Action::Object(O::Transform(K::Rotate))]
            );
            assert_eq!(
                key(&app, Key::S),
                vec![Action::Object(O::Transform(K::Scale))]
            );
            assert_eq!(
                dispatched_with(&map, &app, Key::G, Modifiers::ALT),
                vec![Action::Object(O::Reset(K::Grab))]
            );
            assert_eq!(
                dispatched_with(&map, &app, Key::Tab, Modifiers::SHIFT),
                vec![Action::Object(O::ToggleSnap)]
            );
        }
        assert!(app.set_mode(EditorMode::Paint));
        assert_eq!(key(&app, Key::G), vec![Action::SelectTool(Tool::Fill)]);
        assert_eq!(key(&app, Key::S), vec![Action::SelectTool(Tool::SelectPen)]);
        assert!(key(&app, Key::R).is_empty());
        assert!(dispatched_with(&map, &app, Key::Tab, Modifiers::SHIFT).is_empty());
        // 判定の順: 修飾の数が同じなら、場面 → モード → どこでも
        let order = map.order();
        let at = |command: &str| order.iter().position(|b| b.command == command).unwrap();
        assert!(at("object.hide") < at("view.flip"));
        assert!(at("selection.to_new_layer") < at("layer.duplicate"));
    }

    #[test]
    fn delete_and_the_pixel_keys_follow_the_mode() {
        use crate::mode::EditorMode;
        use crate::objects::ObjectAction as O;
        let map = current();
        let mut app = AppState::new(32, 32);
        app.apply(Action::Sel(crate::selection::SelAction::Edit(
            crate::selection::SelEdit::All,
        )));
        let delete = |app: &AppState| dispatched_with(&map, app, Key::Delete, Modifiers::NONE);
        let erase = Action::Sel(crate::selection::SelAction::Edit(
            crate::selection::SelEdit::Erase,
        ));
        // ペイント: 選択範囲の中身を消す。編集: 選んだ物を消す（選択範囲の中身は消さない）。ポーズ: 何もしない
        assert_eq!(delete(&app), vec![erase]);
        assert!(app.set_mode(EditorMode::Edit));
        assert_eq!(delete(&app), vec![Action::Object(O::Delete)]);
        app.apply(Action::Pose(crate::view3d::pose::PoseAction::LoadFigure));
        assert!(app.set_mode(EditorMode::Pose));
        assert!(delete(&app).is_empty());
        // Q（取っ手を隠す・出す）は、ペイントと編集の段
        assert!(app.set_mode(EditorMode::Edit));
        let q = Action::Fill(crate::fillfx::FillOp::ToggleHandles);
        assert_eq!(
            dispatched_with(&map, &app, Key::Q, Modifiers::NONE),
            vec![q.clone()]
        );
        assert!(app.set_mode(EditorMode::Pose));
        assert!(dispatched_with(&map, &app, Key::Q, Modifiers::NONE).is_empty());
        // 画素を変えるクリップボードのキーはペイントだけ
        assert!(changes_pixels(ClipAction::Cut) && changes_pixels(ClipAction::Paste));
        assert!(!changes_pixels(ClipAction::Copy) && !changes_pixels(ClipAction::CopyMerged));
        // 結合（Ctrl+E・Ctrl+Shift+E）もペイントだけ。複製（Ctrl+J）はどのモードでも
        let cmd = Modifiers::COMMAND;
        for mode in [EditorMode::Edit, EditorMode::Pose] {
            assert!(app.set_mode(mode));
            assert!(
                dispatched_with(&map, &app, Key::E, cmd).is_empty(),
                "{mode:?}"
            );
            assert!(dispatched_with(&map, &app, Key::E, cmd | Modifiers::SHIFT).is_empty());
            assert!(!dispatched_with(&map, &app, Key::J, cmd).is_empty());
        }
        assert!(app.set_mode(EditorMode::Paint));
        assert_eq!(
            dispatched_with(&map, &app, Key::E, cmd),
            vec![Action::M2(crate::m2::Edit::MergeDown)]
        );
    }

    #[test]
    fn a_pose_row_takes_its_key_before_an_everywhere_row_of_the_same_key() {
        use crate::mode::EditorMode;
        // 今の表でポーズの段とどこでもの段に同じキーは無いので、表を作って確かめる（H: どこでもは表示を左右反転、ポーズは初期設定の色）
        let map = Keymap::new(vec![
            kb(Modifiers::NONE, Key::H, "view.flip"),
            kb(Modifiers::NONE, Key::H, "color.default").scope(Scope::Pose),
        ]);
        let mut app = AppState::new(32, 32);
        app.apply(Action::Pose(crate::view3d::pose::PoseAction::LoadFigure));
        let h = |app: &AppState| dispatched_with(&map, app, Key::H, Modifiers::NONE);
        assert_eq!(h(&app), vec![Action::FlipView]);
        assert!(app.set_mode(EditorMode::Pose));
        assert_eq!(h(&app), vec![Action::DefaultColors], "ポーズの段が先に取る");
        assert!(app.set_mode(EditorMode::Edit));
        assert_eq!(h(&app), vec![Action::FlipView]);
        // メニューのキーの文字も同じ決まり: ポーズのモードでは、表示を左右反転に H を添えない
        let row = |command: &str| *map.rows_of(command).next().unwrap();
        assert!(effective_in(&map, &row("view.flip"), EditorMode::Paint));
        assert!(!effective_in(&map, &row("view.flip"), EditorMode::Pose));
        assert!(effective_in(&map, &row("color.default"), EditorMode::Pose));
        assert!(!effective_in(
            &map,
            &row("color.default"),
            EditorMode::Paint
        ));
    }

    #[test]
    fn menu_keys_show_only_the_keys_that_work_in_the_current_mode() {
        use crate::mode::EditorMode::{Edit, Paint, Pose};
        use crate::shortcuts::menu_key_in;
        assert_eq!(menu_key_in("view.flip", Paint).as_deref(), Some("H"));
        assert_eq!(menu_key_in("view.flip", Edit), None, "編集の H は印を隠す");
        assert_eq!(menu_key_in("view.flip", Pose).as_deref(), Some("H"));
        for command in [
            "selection.erase",
            "layer.merge_down",
            "clip.cut",
            "clip.paste",
        ] {
            assert!(menu_key_in(command, Paint).is_some(), "{command}");
            assert_eq!(menu_key_in(command, Edit), None, "{command}");
        }
        assert_eq!(
            menu_key_in("fill.toggle_handles", Edit).as_deref(),
            Some("Q")
        );
        assert!(menu_key_in("layer.duplicate", Edit).is_some());
        // メニューを作る間だけ、そのモードの表で引く
        {
            let _mode = crate::shortcuts::menu_mode(Edit);
            assert_eq!(crate::shortcuts::menu_key("view.flip"), None);
        }
        assert_eq!(
            crate::shortcuts::menu_key("view.flip").as_deref(),
            Some("H")
        );
        // 表示のメニューの「表示を左右反転」: 編集のモードでは H を添えない
        let mut app = AppState::new(32, 32);
        let flip_key = |app: &AppState| {
            (0..8)
                .find_map(|i| {
                    let entries = crate::shell::menu_entries(app, i);
                    crate::ui::menu::leaves(&entries)
                        .into_iter()
                        .find_map(|e| match e {
                            crate::ui::menu::Entry::Item {
                                action: Action::FlipView,
                                shortcut,
                                ..
                            } => Some(shortcut.clone()),
                            _ => None,
                        })
                })
                .expect("表示を左右反転の項目")
        };
        assert_eq!(flip_key(&app).as_deref(), Some("H"));
        assert!(app.set_mode(Edit));
        assert_eq!(flip_key(&app), None);
    }

    #[test]
    fn the_keys_read_by_the_views_come_from_the_table() {
        // R を押しながらの 2D の回転は、既定では割り当てが無い（操作は残してある）
        assert_eq!(hold_key("view.rotate_hold"), None);
        assert_eq!(hold_key("view.pan_hold"), Some(Key::Space));
        assert_eq!(hold_key("stencil.transform_hold"), Some(Key::Y));
        assert_eq!(hold_key("stencil.bypass_hold"), Some(Key::N));
        assert_eq!(hold_key("nothing.here"), None);
        assert_eq!(key_of("view3d.frame_selected"), Some(Key::Period));
        assert_eq!(key_of("canvas.cancel"), Some(Key::Escape));
        assert_eq!(key_of("canvas.confirm"), Some(Key::Enter));
        assert_eq!(key_of("canvas.remove_last_point"), Some(Key::Backspace));
        assert_eq!(key_of("view3d.cancel"), Some(Key::Escape));
        assert_eq!(key_of("stencil.cancel"), Some(Key::Escape));
        // 文字の行は、キーを持たない
        assert_eq!(
            primary("view.rotate_right").map(|b| b.trigger),
            Some(Trigger::Text("^"))
        );
        assert_eq!(hold_key("view.rotate_right"), Some(Key::Equals));
        // 矢印は 1 画素の 4 つと Shift の 4 つ
        assert_eq!(NUDGES.len(), 8);
        for n in NUDGES {
            let row = primary(n.command).unwrap_or_else(|| panic!("{}", n.command));
            let modifiers = if n.shift {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            };
            assert_eq!(
                row.trigger,
                Trigger::Key {
                    modifiers,
                    key: n.key
                }
            );
            assert_eq!(row.when, When::Tool(Tool::Move));
        }
        // 押している間のキーを持つ操作の組み合わせの表示は、そのキーの操作の ID を指す
        for g in &GESTURES {
            if let Some(held) = g.held {
                assert!(hold_key(held).is_some(), "{held}");
            }
        }
    }

    #[test]
    fn consume_command_reads_the_table_and_respects_the_condition() {
        let mut app = AppState::new(32, 32);
        let ctx = egui::Context::default();
        let press = |key, modifiers| egui::RawInput {
            events: vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        };
        // 矢印は移動・変形のツールのときだけ
        app.tool = Tool::Brush;
        let mut hit = true;
        let mut output = ctx.run_ui(press(Key::ArrowLeft, Modifiers::NONE), |ui| {
            hit = ui.input_mut(|i| consume_command(i, &app, "transform.nudge_left"));
        });
        output.textures_delta.clear();
        assert!(!hit);
        app.tool = Tool::Move;
        let mut output = ctx.run_ui(press(Key::ArrowLeft, Modifiers::NONE), |ui| {
            hit = ui.input_mut(|i| consume_command(i, &app, "transform.nudge_left"));
        });
        output.textures_delta.clear();
        assert!(hit);
        // Shift+矢印は 10 画素の行にだけ当たる
        let mut small = true;
        let mut big = false;
        let mut output = ctx.run_ui(press(Key::ArrowLeft, Modifiers::SHIFT), |ui| {
            small = ui.input_mut(|i| consume_command(i, &app, "transform.nudge_left"));
            big = ui.input_mut(|i| consume_command(i, &app, "transform.nudge_left_10"));
        });
        output.textures_delta.clear();
        assert!(!small && big);
    }

    #[test]
    fn a_rebuilt_keymap_keeps_its_rows_and_its_judging_order_together() {
        let default = Keymap::new(bindings());
        assert_eq!(default.rows(), bindings().as_slice());
        // 判定の順は、行のうち Action を持つもの（クリップボードを除く）を、修飾の多い順に並べたもの
        let judged = default
            .rows()
            .iter()
            .filter(|b| !matches!(b.action(), None | Some(Action::Clip(_))))
            .count();
        assert_eq!(default.order().len(), judged);
        // 行を減らして作り直すと、判定の順も一緒に変わる
        let only_save: Vec<KeyBinding> = bindings()
            .into_iter()
            .filter(|b| b.command == "file.save")
            .collect();
        assert_eq!(only_save.len(), 1);
        let small = Keymap::new(only_save);
        assert_eq!(small.rows().len(), 1);
        assert_eq!(small.order().len(), 1);
        assert_eq!(small.rows_of("file.save").count(), 1);
        assert_eq!(small.rows_of("file.open").count(), 0);
        // 今効いている割り当てを同じ内容で差し替えても、表と判定の順は変わらない（ほかの試験と並んで走ってよい）
        let before = current();
        replace(bindings());
        let after = current();
        assert!(!Arc::ptr_eq(&before, &after));
        assert_eq!(before.rows(), after.rows());
        assert_eq!(before.order(), after.order());
    }

    #[test]
    fn the_fly_keys_are_rows_of_the_table_and_point_the_way_their_names_say() {
        let expected = [
            ("view3d.fly_forward", Key::W, [0.0, 0.0, 1.0]),
            ("view3d.fly_back", Key::S, [0.0, 0.0, -1.0]),
            ("view3d.fly_left", Key::A, [-1.0, 0.0, 0.0]),
            ("view3d.fly_right", Key::D, [1.0, 0.0, 0.0]),
            ("view3d.fly_down", Key::Q, [0.0, -1.0, 0.0]),
            ("view3d.fly_up", Key::E, [0.0, 1.0, 0.0]),
        ];
        assert_eq!(FLY_KEYS.len(), expected.len());
        for (command, key, direction) in expected {
            assert_eq!(hold_key(command), Some(key), "{command}");
            let fly = FLY_KEYS
                .iter()
                .find(|f| f.command == command)
                .unwrap_or_else(|| panic!("{command}"));
            assert_eq!((fly.key, fly.direction), (key, direction), "{command}");
            assert_eq!(
                commands::find(command).map(|c| c.kind),
                Some(commands::Kind::Hold),
                "{command}"
            );
            let row = primary(command).expect("表の行");
            assert_eq!(
                row.trigger,
                Trigger::Key {
                    modifiers: Modifiers::NONE,
                    key
                }
            );
        }
    }

    #[test]
    fn taking_the_fly_keys_removes_only_plain_and_shifted_presses_of_those_keys() {
        let key = |key, modifiers, pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        let ctx = egui::Context::default();
        let mut left = Vec::new();
        let input = egui::RawInput {
            events: vec![
                key(Key::W, Modifiers::NONE, true),
                key(Key::S, Modifiers::SHIFT, true),
                key(Key::A, Modifiers::COMMAND, true),
                key(Key::D, Modifiers::ALT, true),
                key(Key::Q, Modifiers::NONE, true),
                key(Key::E, Modifiers::NONE, false),
                key(Key::X, Modifiers::NONE, true),
                Event::Text("w".into()),
            ],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            ui.input_mut(|i| {
                take_fly_keys(i, &mut Vec::new(), true);
                left = i.events.clone();
            });
        });
        output.textures_delta.clear();
        // 修飾なし・Shift だけの押し（W・S・Q）は取り除く。Ctrl・Alt を足したもの・離した事象・ほかのキー・文字の入力は残す
        let keys: Vec<(Key, bool)> = left
            .iter()
            .filter_map(|e| match e {
                Event::Key { key, pressed, .. } => Some((*key, *pressed)),
                _ => None,
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                (Key::A, true),
                (Key::D, true),
                (Key::E, false),
                (Key::X, true)
            ]
        );
        assert!(left.iter().any(|e| matches!(e, Event::Text(t) if t == "w")));
    }

    #[test]
    fn a_fly_key_still_held_after_the_flight_stays_off_the_table_until_it_is_released_and_pressed_again(
    ) {
        let key = |key, modifiers, pressed, repeat| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat,
            modifiers,
        };
        let ctx = egui::Context::default();
        let mut held: Vec<Key> = Vec::new();
        let frame = |events: Vec<Event>, flying: bool, held: &mut Vec<Key>| -> Vec<(Key, bool)> {
            let mut left = Vec::new();
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                ui.input_mut(|i| {
                    take_fly_keys(i, held, flying);
                    left = i
                        .events
                        .iter()
                        .filter_map(|e| match e {
                            Event::Key { key, pressed, .. } => Some((*key, *pressed)),
                            _ => None,
                        })
                        .collect();
                });
            });
            output.textures_delta.clear();
            left
        };
        // 移動の間（右ボタンを押している）に W・D・Shift+Q を押す: 取り除く
        let left = frame(
            vec![
                key(Key::W, Modifiers::NONE, true, false),
                key(Key::D, Modifiers::NONE, true, false),
                key(Key::Q, Modifiers::SHIFT, true, false),
            ],
            true,
            &mut held,
        );
        assert!(left.is_empty());
        assert_eq!(held.len(), 3);
        // 右ボタンを先に離しても、押したままの間のキーの繰り返しは取り除く（W・D・Shift+Q）
        let left = frame(
            vec![
                key(Key::W, Modifiers::NONE, true, true),
                key(Key::D, Modifiers::NONE, true, true),
                key(Key::Q, Modifiers::SHIFT, true, true),
            ],
            false,
            &mut held,
        );
        assert!(left.is_empty(), "{left:?}");
        // 押していなかったキー（X）と、Ctrl を足した W は届く
        let left = frame(
            vec![
                key(Key::X, Modifiers::NONE, true, false),
                key(Key::W, Modifiers::COMMAND, true, true),
            ],
            false,
            &mut held,
        );
        assert_eq!(left, vec![(Key::X, true), (Key::W, true)]);
        // W を離す: 離した事象は届き、W だけ「押したまま」の扱いが終わる
        let left = frame(
            vec![key(Key::W, Modifiers::NONE, false, false)],
            false,
            &mut held,
        );
        assert_eq!(left, vec![(Key::W, false)]);
        assert!(!held.contains(&Key::W) && held.contains(&Key::D) && held.contains(&Key::Q));
        // 押し直した W は、表へ届く。D・Q は押したままなので、まだ届かない
        let left = frame(
            vec![
                key(Key::W, Modifiers::NONE, true, false),
                key(Key::D, Modifiers::NONE, true, true),
            ],
            false,
            &mut held,
        );
        assert_eq!(left, vec![(Key::W, true)]);
        // 全部離したら、何も覚えていない
        frame(
            vec![
                key(Key::W, Modifiers::NONE, false, false),
                key(Key::D, Modifiers::NONE, false, false),
                key(Key::Q, Modifiers::SHIFT, false, false),
            ],
            false,
            &mut held,
        );
        assert!(held.is_empty());
    }

    #[test]
    fn a_fly_key_pressed_before_the_right_button_is_held_counts_as_held_for_the_flight() {
        let ctx = egui::Context::default();
        let mut held: Vec<Key> = Vec::new();
        let run = |events: Vec<Event>, flying: bool, held: &mut Vec<Key>| {
            let mut count = 0;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.input_mut(|i| {
                        take_fly_keys(i, held, flying);
                        count = i
                            .events
                            .iter()
                            .filter(|e| matches!(e, Event::Key { pressed: true, .. }))
                            .count();
                    });
                },
            );
            output.textures_delta.clear();
            count
        };
        let press = |repeat| Event::Key {
            key: Key::W,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: Modifiers::NONE,
        };
        // 右ボタンを押す前に W を押していた（表へ届く）
        assert_eq!(run(vec![press(false)], false, &mut held), 1);
        // 右ボタンを押した（移動の間）: 繰り返しは届かず、右を離したあとも W を離すまで届かない
        assert_eq!(run(vec![press(true)], true, &mut held), 0);
        assert_eq!(run(vec![press(true)], false, &mut held), 0);
    }

    #[test]
    fn every_clipboard_key_has_its_own_command_id() {
        for (modifiers, key, action) in CLIPBOARD_KEYS {
            let row = bindings()
                .into_iter()
                .find(|b| b.trigger == Trigger::Key { modifiers, key })
                .expect("クリップボードの行");
            assert_eq!(row.action(), Some(Action::Clip(action)));
            assert_eq!(row.command, clip_command(action));
        }
    }
}
