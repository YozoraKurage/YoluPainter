//! キーとマウスの割り当ての宣言の表。キーボードの割り当て（`bindings`）・マウスと修飾キーの組み合わせ（`GESTURES`）・押している間だけ効くキーや
//! ビューの中のキー（`CONTEXT_KEYS`）を、ここだけに書く。キーの処理（`shell::handle_shortcuts`・3D ビューの回す・パン・ステンシルの移動など）と
//! ショートカットの一覧の窓（`shortcuts`）は、この表を読む。実装の入力判定のソースを文字で読んで一覧を作る方式（ビルドスクリプト）はやめた:
//! 表が実装の一次の資料なので、一覧と実際の入力が食い違わない（食い違いは試験が、表のすべての割り当てを実際の入力へ流して確かめる）。
//! 道具のキーは道具の表（`tools`）の `key` から作る。キーを利用者が替える設定は、この表の上に作る（`bindings` を差し替える）。
//!
//! 順序の決まり: `consume_key` は、書いていない Shift・Alt を気にしない（Shift 付きも修飾なしに当たる）ので、同じキーの割り当ては、修飾の多いほうを
//! 先に判定する。`bindings()` は一覧に出す順、`dispatch` は判定の順（修飾の多いものが先。同じなら表の順）。

use std::sync::OnceLock;

use egui::{InputState, Key, Modifiers, PointerButton};

use crate::clipboard::ClipAction;
use crate::lang::Lang;
use crate::m2::Edit;
use crate::pathtool::PathAction;
use crate::prefs::PrefsAction;
use crate::selection::{SelAction, SelEdit, SelUiOp};
use crate::state::{Action, AppState, Tool};

// ───────── キーボードの割り当て ─────────

/// 割り当てが効く条件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Always,
    /// 選択範囲があるとき。
    HasSelection,
    /// この道具を選んでいるとき。
    Tool(Tool),
    /// Windows のとき（画面から色を取る）。
    Windows,
}

impl When {
    pub fn holds(self, app: &AppState) -> bool {
        match self {
            When::Always => true,
            When::HasSelection => app.doc.selection().is_some(),
            When::Tool(tool) => app.tool == tool,
            When::Windows => cfg!(windows) && !app.is_stroking(),
        }
    }
}

/// 1 つのキーの割り当て。
#[derive(Clone, Debug)]
pub struct KeyBinding {
    pub modifiers: Modifiers,
    pub key: Key,
    pub when: When,
    pub action: Action,
}

fn kb(modifiers: Modifiers, key: Key, action: Action) -> KeyBinding {
    KeyBinding {
        modifiers,
        key,
        when: When::Always,
        action,
    }
}

fn kb_when(modifiers: Modifiers, key: Key, when: When, action: Action) -> KeyBinding {
    KeyBinding {
        modifiers,
        key,
        when,
        action,
    }
}

fn sel_edit(edit: SelEdit) -> Action {
    Action::Sel(SelAction::Edit(edit))
}

/// 道具のキーの文字（「B」「Shift+G」「4」。道具の表の `key`）から、修飾とキーを読む。キーが空・読めなければ None。
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

/// 一覧に出す順の、キーボードの割り当て（クリップボードのキーも含む）。
pub fn bindings() -> Vec<KeyBinding> {
    let cmd = Modifiers::COMMAND;
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let none = Modifiers::NONE;
    let shift = Modifiers::SHIFT;
    let mut v = vec![
        kb(cmd_shift, Key::E, Action::M2(Edit::MergeVisible)),
        kb(cmd_shift, Key::G, Action::M2(Edit::UngroupSelected)),
        kb(cmd, Key::E, Action::M2(Edit::MergeDown)),
        // Ctrl+J: 選択範囲があれば、その画素を新しいレイヤーへ（Photoshop の「コピーしたレイヤー」）。無ければレイヤーの複製
        kb_when(
            cmd,
            Key::J,
            When::HasSelection,
            sel_edit(SelEdit::ToNewLayer),
        ),
        kb(cmd, Key::J, Action::M2(Edit::DuplicateSelected)),
        kb(cmd, Key::G, Action::M2(Edit::GroupSelected)),
        kb(cmd_shift, Key::I, sel_edit(SelEdit::Invert)),
        kb(cmd_shift, Key::Z, Action::Redo),
        kb(cmd, Key::A, sel_edit(SelEdit::All)),
        kb(cmd, Key::D, sel_edit(SelEdit::Clear)),
        // 選択範囲があるときだけ: 消去（Delete）
        kb_when(
            none,
            Key::Delete,
            When::HasSelection,
            sel_edit(SelEdit::Erase),
        ),
        kb(cmd, Key::Z, Action::Undo),
        kb(cmd, Key::Y, Action::Redo),
        kb(cmd_shift, Key::N, Action::NewLayer),
        kb(cmd_shift, Key::S, Action::SaveProjectAsDialog),
        kb(cmd, Key::S, Action::SaveProject),
        kb(cmd, Key::O, Action::OpenProjectDialog),
        kb(cmd, Key::N, Action::NewProjectDialog),
        kb(cmd, Key::Num1, Action::ToggleRulerSnap),
        kb(cmd, Key::Num0, Action::FitView),
        kb(cmd, Key::Plus, Action::ZoomIn),
        kb(cmd, Key::Equals, Action::ZoomIn),
        kb(cmd, Key::Minus, Action::ZoomOut),
        kb(cmd, Key::Q, Action::Quit),
        kb(cmd, Key::Comma, Action::Prefs(PrefsAction::Open)),
        kb(shift, Key::R, Action::ResetRotation),
    ];
    // 道具のキー（道具の表のとおり。ツールの帯の並び）
    for tool in Tool::ALL {
        if let Some((modifiers, key)) = parse_tool_key(tool.key()) {
            v.push(kb(modifiers, key, Action::SelectTool(tool)));
        }
    }
    v.extend([
        kb(
            shift,
            Key::Q,
            Action::Sel(SelAction::Ui(SelUiOp::QuickMask(None))),
        ),
        // パスの道具: 選んでいる点（無ければ最後の点）を消す
        kb_when(
            none,
            Key::Delete,
            When::Tool(Tool::Path),
            Action::Path(PathAction::DeleteSelected),
        ),
        kb_when(
            none,
            Key::Backspace,
            When::Tool(Tool::Path),
            Action::Path(PathAction::DeleteSelected),
        ),
        kb(
            none,
            Key::Q,
            Action::Fill(crate::fillfx::FillOp::ToggleHandles),
        ),
        kb(none, Key::X, Action::SwapColors),
        kb(none, Key::D, Action::DefaultColors),
        kb(none, Key::OpenBracket, Action::BrushSmaller),
        kb(none, Key::CloseBracket, Action::BrushLarger),
        kb(none, Key::H, Action::FlipView),
        kb(none, Key::Minus, Action::RotateLeft),
        kb(none, Key::Equals, Action::RotateRight),
    ]);
    // 画面から色を取る（Windows）
    for mode in [
        crate::screen_pick::Mode::HideWindow,
        crate::screen_pick::Mode::Visible,
    ] {
        let modifiers = Modifiers::CTRL
            | Modifiers::ALT
            | if mode == crate::screen_pick::Mode::HideWindow {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            };
        v.push(kb_when(
            modifiers,
            Key::I,
            When::Windows,
            Action::ScreenPick(mode),
        ));
    }
    // クリップボード（コピー・カット・ペースト。入力の受け方は `clipboard::keys` が持つが、割り当てはここ）
    for (modifiers, key, action) in CLIPBOARD_KEYS {
        v.push(kb(modifiers, key, Action::Clip(action)));
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

/// 押している修飾キーの数。
fn modifier_count(m: &Modifiers) -> u8 {
    u8::from(m.shift) + u8::from(m.alt) + u8::from(m.command) + u8::from(m.ctrl)
}

/// 判定の順の、キーボードの割り当て（修飾の多いものが先）。クリップボードは `clipboard::keys` が受けるので入らない。
pub(crate) fn dispatch_order() -> &'static [KeyBinding] {
    static ORDER: OnceLock<Vec<KeyBinding>> = OnceLock::new();
    ORDER.get_or_init(|| {
        let mut v: Vec<KeyBinding> = bindings()
            .into_iter()
            .filter(|b| !matches!(b.action, Action::Clip(_)))
            .collect();
        // 安定な並べ替え（修飾の数が同じなら表の順）。修飾の多いものは、少ないものの修飾を含むので、先に判定する
        v.sort_by_key(|b| std::cmp::Reverse(modifier_count(&b.modifiers)));
        v
    })
}

/// このフレームのキーの操作（効く条件を満たし、押されたもの。押した事象は取り除く）。文字を打っている・メニューを開いている間は呼ばない。
pub fn dispatch(i: &mut InputState, app: &AppState) -> Vec<Action> {
    let mut actions = Vec::new();
    for b in dispatch_order() {
        if b.when.holds(app) && i.consume_key(b.modifiers, b.key) {
            actions.push(b.action.clone());
        }
    }
    actions
}

/// 移動・変形の道具の矢印キー（画面の向きの 1 画素。Shift で 10）。
pub const MOVE_KEYS: [(Key, (f64, f64)); 4] = [
    (Key::ArrowLeft, (-1.0, 0.0)),
    (Key::ArrowRight, (1.0, 0.0)),
    (Key::ArrowUp, (0.0, -1.0)),
    (Key::ArrowDown, (0.0, 1.0)),
];

/// 「表示を右に回す」のもう 1 つの割り当て（キーの位置が配列で違うので文字で見る。JIS の ^ のキー、US の Shift+6）。
pub const ROTATE_RIGHT_TEXT: &str = "^";

// ───────── ビューの中のキー ─────────

/// 2D の表示を回すキー（押しながら左ドラッグ）。
pub const VIEW_ROTATE: Key = Key::R;
/// パン（押しながら左ドラッグ。Ctrl を足すと拡縮）のキー。2D のキャンバスと 3D ビューで同じ。
pub const VIEW_PAN: Key = Key::Space;
/// ステンシルの置き場を動かすキー（押しながらドラッグ）。
pub const STENCIL_MOVE: Key = Key::T;
/// ステンシルを使わないあいだ押すキー。
pub const STENCIL_BYPASS: Key = Key::N;
/// 3D ビューで選んだセットを収めるキー（3D の上で、修飾なし）。
pub const VIEW3D_FRAME: Key = Key::Period;

/// ビューごとのキー（一覧の「2D ビュー」「3D ビュー」「ステンシル」の項目。押しながらマウスを使うものは `mouse`）。
#[derive(Clone, Copy, Debug)]
pub struct ContextKey {
    pub scope: &'static str,
    pub key: Key,
    pub mouse: bool,
    pub ja: &'static str,
    pub en: &'static str,
}

impl ContextKey {
    pub fn label(&self, lang: Lang) -> &'static str {
        lang.pick(self.ja, self.en)
    }
}

/// 一覧に出すビューのキー。押しながらの組み合わせで一覧に出るもの（3D の Space・ステンシルの T）は、`GESTURES` の側に出る。
pub const CONTEXT_KEYS: [ContextKey; 9] = [
    ContextKey {
        scope: "canvas",
        key: VIEW_ROTATE,
        mouse: true,
        ja: "回転",
        en: "Rotate",
    },
    ContextKey {
        scope: "canvas",
        key: VIEW_PAN,
        mouse: true,
        ja: "パン / Ctrl: ズーム",
        en: "Pan / Ctrl: Zoom",
    },
    ContextKey {
        scope: "canvas",
        key: Key::Escape,
        mouse: false,
        ja: "操作をキャンセル",
        en: "Cancel Operation",
    },
    ContextKey {
        scope: "canvas",
        key: Key::Enter,
        mouse: false,
        ja: "変形・多角形選択を確定",
        en: "Confirm Transform / Polygon Selection",
    },
    ContextKey {
        scope: "canvas",
        key: Key::Backspace,
        mouse: false,
        ja: "多角形選択の最後の点を削除",
        en: "Delete Last Polygon Selection Point",
    },
    ContextKey {
        scope: "view3d",
        key: Key::Escape,
        mouse: false,
        ja: "操作をキャンセル",
        en: "Cancel Operation",
    },
    ContextKey {
        scope: "view3d",
        key: VIEW3D_FRAME,
        mouse: false,
        ja: "選んだセットを収める",
        en: "Frame Selected Set",
    },
    ContextKey {
        scope: "stencil",
        key: STENCIL_BYPASS,
        mouse: false,
        ja: "ステンシルを一時解除",
        en: "Bypass Stencil",
    },
    ContextKey {
        scope: "stencil",
        key: Key::Escape,
        mouse: false,
        ja: "操作をキャンセル",
        en: "Cancel Operation",
    },
];

// ───────── マウスと修飾キーの組み合わせ ─────────

/// マウスの組み合わせで決まる操作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    /// 3D ビューの視点を回す。
    Orbit,
    Pan,
    Zoom,
    /// 2D キャンバスの表示を回す。
    Rotate,
    /// 描く道具で色を取る（Alt）。
    Pick,
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
    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            Self::Orbit => lang.pick("回転", "Orbit"),
            Self::Pan => lang.pick("パン", "Pan"),
            Self::Zoom => lang.pick("ズーム", "Zoom"),
            Self::Rotate => lang.pick("回転", "Rotate"),
            Self::Pick => Tool::Eyedropper.name_in(lang),
            Self::SelectionAdd => lang.pick("選択範囲に足す", "Add to Selection"),
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
/// `held` はあるとき押しているキー。
#[derive(Clone, Copy, Debug)]
pub struct Gesture {
    /// 一覧のまとまり（選択範囲の道具の組み合わせ方は「selection」で、一覧では 2D ビューに並ぶ）。
    pub scope: &'static str,
    pub held: Option<Key>,
    pub button: PointerButton,
    pub alt: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub operation: Operation,
    /// この組み合わせで操作を始める（偽なら、始めたあとの修飾の効き方で、始め方の判定には使わない）。
    pub starts: bool,
}

const fn gesture_of(
    scope: &'static str,
    held: Option<Key>,
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
    }
}

use PointerButton::{Middle, Primary, Secondary};

/// マウスと修飾キーの組み合わせの全部。
pub const GESTURES: [Gesture; 19] = [
    // 2D キャンバス: 中ボタンのパンと、Shift で回転。左ボタンの Alt は描く道具のスポイト
    gesture_of(
        "canvas",
        None,
        Middle,
        (false, true, false),
        Operation::Rotate,
    ),
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
        Operation::Pick,
    ),
    // 選択範囲の道具の作り方: Shift で足す・Ctrl で引く・両方で重ねる
    gesture_of(
        "selection",
        None,
        Primary,
        (false, true, true),
        Operation::SelectionIntersect,
    ),
    gesture_of(
        "selection",
        None,
        Primary,
        (false, true, false),
        Operation::SelectionAdd,
    ),
    gesture_of(
        "selection",
        None,
        Primary,
        (false, false, true),
        Operation::SelectionSubtract,
    ),
    // 3D ビュー: 右ボタンで回す（Shift でパン）・中ボタンでパン・Space + 左でパン（Ctrl を足すと拡縮）・Alt + 左で回す（Shift を足すとパン）
    gesture_of(
        "view3d",
        None,
        Secondary,
        (false, true, false),
        Operation::Pan,
    ),
    gesture_of(
        "view3d",
        None,
        Secondary,
        (false, false, false),
        Operation::Orbit,
    ),
    gesture_of(
        "view3d",
        None,
        Middle,
        (false, false, false),
        Operation::Pan,
    ),
    gesture_of(
        "view3d",
        Some(VIEW_PAN),
        Primary,
        (false, false, true),
        Operation::Zoom,
    ),
    gesture_of(
        "view3d",
        Some(VIEW_PAN),
        Primary,
        (false, false, false),
        Operation::Pan,
    ),
    gesture_of("view3d", None, Primary, (true, true, false), Operation::Pan),
    gesture_of(
        "view3d",
        None,
        Primary,
        (true, false, false),
        Operation::Orbit,
    ),
    // ステンシル（T を押しながら）: 左で回す・中か Ctrl + 左で動かす・右か Alt + 左で大きさ
    gesture_of(
        "stencil",
        Some(STENCIL_MOVE),
        Middle,
        (false, false, false),
        Operation::MoveStencil,
    ),
    gesture_of(
        "stencil",
        Some(STENCIL_MOVE),
        Primary,
        (false, false, true),
        Operation::MoveStencil,
    ),
    gesture_of(
        "stencil",
        Some(STENCIL_MOVE),
        Secondary,
        (false, false, false),
        Operation::ScaleStencil,
    ),
    gesture_of(
        "stencil",
        Some(STENCIL_MOVE),
        Primary,
        (true, false, false),
        Operation::ScaleStencil,
    ),
    gesture_of(
        "stencil",
        Some(STENCIL_MOVE),
        Primary,
        (false, false, false),
        Operation::RotateStencil,
    ),
    // 回している間の Shift は 15° 刻み（`StencilState::update_drag`）。押す順は問わないので、回す組み合わせに Shift を足した形で載せる
    Gesture {
        scope: "stencil",
        held: Some(STENCIL_MOVE),
        button: Primary,
        alt: false,
        shift: true,
        ctrl: false,
        operation: Operation::SnapStencilRotation,
        starts: false,
    },
];

/// Ctrl（Mac の Command も）か。
fn ctrl(m: &Modifiers) -> bool {
    m.ctrl || m.command
}

/// この押しに当たる操作（`held` は、その範囲の押しながらのキーを押しているか）。表の上から順に最初に当たったもの。
pub fn gesture(scope: &str, button: PointerButton, m: &Modifiers, held: bool) -> Option<Operation> {
    GESTURES
        .iter()
        .filter(|g| g.starts && g.scope == scope)
        .find(|g| {
            g.button == button
                && (g.held.is_none() || held)
                && (!g.alt || m.alt)
                && (!g.shift || m.shift)
                && (!g.ctrl || ctrl(m))
        })
        .map(|g| g.operation)
}

/// 描く道具の左ボタンが、この修飾でスポイトになるか（2D だけ）。
pub fn picks(m: &Modifiers) -> bool {
    gesture("canvas", Primary, m, false) == Some(Operation::Pick)
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
    fn the_view_gestures_resolve_the_way_the_input_code_used_to() {
        let none = Modifiers::NONE;
        let alt = Modifiers::ALT;
        let shift = Modifiers::SHIFT;
        let alt_shift = Modifiers::ALT | Modifiers::SHIFT;
        let ctrl = Modifiers::CTRL;
        // 3D
        assert_eq!(
            gesture("view3d", Secondary, &none, false),
            Some(Operation::Orbit)
        );
        assert_eq!(
            gesture("view3d", Secondary, &shift, false),
            Some(Operation::Pan)
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
            Some(Operation::Orbit)
        );
        assert_eq!(
            gesture("view3d", Primary, &alt_shift, false),
            Some(Operation::Pan)
        );
        // Space を押していても、右・中ボタンは同じ
        assert_eq!(
            gesture("view3d", Secondary, &none, true),
            Some(Operation::Orbit)
        );
        // 2D
        assert_eq!(
            gesture("canvas", Middle, &none, false),
            Some(Operation::Pan)
        );
        assert_eq!(
            gesture("canvas", Middle, &shift, false),
            Some(Operation::Rotate)
        );
        assert!(picks(&alt) && !picks(&none) && !picks(&shift));
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

    #[test]
    fn more_specific_keys_are_judged_before_the_ones_that_would_swallow_them() {
        let order = dispatch_order();
        let includes = |big: &Modifiers, small: &Modifiers| {
            (!small.shift || big.shift)
                && (!small.alt || big.alt)
                && (!small.command || big.command)
                && (!small.ctrl || big.ctrl)
        };
        for (i, a) in order.iter().enumerate() {
            for b in &order[i + 1..] {
                if a.key != b.key {
                    continue;
                }
                // 先に判定される a は、書いていない修飾を気にしないことがある。b が a の修飾を全部含む（b のほうが細かい）のに a が先だと、
                // b の押しは a に横取りされる（a が、b と同じ条件でしか効かない場合を除く）
                let swallowed = includes(&b.modifiers, &a.modifiers)
                    && modifier_count(&b.modifiers) > modifier_count(&a.modifiers);
                assert!(
                    !swallowed || a.when != When::Always,
                    "{:?}+{:?} が先の {:?}+{:?} に横取りされる",
                    b.modifiers,
                    b.key,
                    a.modifiers,
                    a.key
                );
            }
        }
    }

    #[test]
    fn tool_keys_come_from_the_tool_table_and_every_tool_with_a_key_has_one_binding() {
        let all = bindings();
        for tool in Tool::ALL {
            let found: Vec<_> = all
                .iter()
                .filter(|b| b.action == Action::SelectTool(tool))
                .collect();
            if tool.key().is_empty() {
                assert!(found.is_empty(), "{tool:?}");
                continue;
            }
            assert_eq!(found.len(), 1, "{tool:?}");
            let (m, key) = parse_tool_key(tool.key()).expect("読める");
            assert_eq!((found[0].modifiers, found[0].key), (m, key), "{tool:?}");
        }
        assert_eq!(parse_tool_key("Shift+G"), Some((Modifiers::SHIFT, Key::G)));
        assert_eq!(parse_tool_key("4"), Some((Modifiers::NONE, Key::Num4)));
        assert_eq!(parse_tool_key(""), None);
        assert_eq!(parse_tool_key("Meta+G"), None);
    }

    #[test]
    fn the_same_key_and_modifiers_are_never_bound_twice_under_the_same_condition() {
        let all = bindings();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                if a.key == b.key && a.modifiers == b.modifiers && a.when == b.when {
                    panic!(
                        "重なった割り当て: {:?}+{:?} {:?} / {:?}",
                        a.modifiers, a.key, a.action, b.action
                    );
                }
            }
        }
    }
}
