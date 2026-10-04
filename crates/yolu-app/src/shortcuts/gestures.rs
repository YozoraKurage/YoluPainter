//! 実装の入力判定から、修飾キー・ボタン・保持キーの組み合わせを列挙する。
use crate::{
    gesture, lang::Lang, selection::combine_of, stencil::DragKind, view3d::Nav, windows::Row,
};
use egui::{Key, Modifiers, PointerButton};
use std::sync::OnceLock;
use yolu_core::selection::SelectionCombine;

include!(concat!(env!("OUT_DIR"), "/shortcut_gestures.rs"));

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
            Self::Pick => crate::state::Tool::Eyedropper.name_in(lang),
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Binding {
    pub scope: &'static str,
    pub held: Option<Key>,
    pub modifiers: Modifiers,
    pub button: PointerButton,
    pub operation: Operation,
}

fn modifiers(bits: u8) -> Modifiers {
    Modifiers {
        alt: bits & 1 != 0,
        shift: bits & 2 != 0,
        ctrl: bits & 4 != 0,
        command: bits & 4 != 0,
        ..Modifiers::NONE
    }
}

/// 2D キャンバスの押しの組み合わせ。中ボタンは Shift で回転・そのほかはパン（`canvas/mod.rs` の中ボタンの処理）、
/// 左ボタンの Alt は描く道具のスポイト（`eyedrop::picks`）。
fn canvas_operation(button: PointerButton, m: &Modifiers) -> Option<Operation> {
    match button {
        PointerButton::Middle => Some(if m.shift {
            Operation::Rotate
        } else {
            Operation::Pan
        }),
        PointerButton::Primary if m.alt => Some(Operation::Pick),
        _ => None,
    }
}

fn operation(scope: &str, button: PointerButton, bits: u8, held: bool) -> Option<Operation> {
    let m = modifiers(bits);
    match scope {
        "view3d" => navigation(button, &m, held).map(|nav| match nav {
            Nav::Orbit => Operation::Orbit,
            Nav::Pan => Operation::Pan,
            Nav::Zoom => Operation::Zoom,
        }),
        "canvas" if !held => canvas_operation(button, &m),
        // 選択範囲の道具の組み合わせ方は、実装の `combine_of`（Shift で足す・Ctrl で引く・両方で重ねる）をそのまま使う。
        "selection" if !held && button == PointerButton::Primary => {
            match combine_of(SelectionCombine::Replace, m) {
                SelectionCombine::Replace => None,
                SelectionCombine::Add => Some(Operation::SelectionAdd),
                SelectionCombine::Subtract => Some(Operation::SelectionSubtract),
                SelectionCombine::Intersect => Some(Operation::SelectionIntersect),
            }
        }
        "stencil" if held => stencil(button, &m).map(|kind| match kind {
            DragKind::Move => Operation::MoveStencil,
            DragKind::Rotate => Operation::RotateStencil,
            DragKind::Scale => Operation::ScaleStencil,
        }),
        _ => None,
    }
}

pub fn bindings() -> &'static [Binding] {
    static BINDINGS: OnceLock<Vec<Binding>> = OnceLock::new();
    BINDINGS.get_or_init(|| {
        let mut out = Vec::new();
        for scope in ["canvas", "selection", "view3d", "stencil"] {
            for button in [
                PointerButton::Primary,
                PointerButton::Middle,
                PointerButton::Secondary,
            ] {
                for held in [false, true] {
                    for bits in 0..8 {
                        let Some(op) = operation(scope, button, bits, held) else {
                            continue;
                        };
                        // 結果が同じ不要な修飾の組を落とす。Alt+Shift のパンのように結果が変わるものは残す。
                        let redundant = (0..bits).any(|less| {
                            less & bits == less && operation(scope, button, less, held) == Some(op)
                        }) || (held
                            && operation(scope, button, bits, false) == Some(op));
                        if redundant {
                            continue;
                        }
                        out.push(Binding {
                            scope,
                            held: held.then(|| {
                                if scope == "stencil" {
                                    stencil_key()
                                } else {
                                    space_key()
                                }
                            }),
                            modifiers: modifiers(bits),
                            button,
                            operation: op,
                        });
                    }
                }
            }
        }
        // 回している間の Shift は 15° 刻み（`StencilState::update_drag`）。押す順は問わないので、回す組み合わせに Shift を足した形で載せる。
        out.push(Binding {
            scope: "stencil",
            held: Some(stencil_key()),
            modifiers: Modifiers {
                shift: true,
                ..Modifiers::NONE
            },
            button: PointerButton::Primary,
            operation: Operation::SnapStencilRotation,
        });
        out
    })
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
    text.push_str(match binding.button {
        PointerButton::Primary => lang.pick("左ボタン", "Left Button"),
        PointerButton::Middle => lang.pick("中ボタン", "Middle Button"),
        PointerButton::Secondary => lang.pick("右ボタン", "Right Button"),
        _ => unreachable!("一覧は3つのボタンを列挙する"),
    });
    text
}

/// 一覧で見せるまとまり。選択範囲の道具の組み合わせは 2D ビューに並べる。
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
