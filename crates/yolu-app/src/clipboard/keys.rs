//! クリップボードのキー（Ctrl+C・Ctrl+X・Ctrl+V・Ctrl+Shift+C）。
//!
//! egui-winit は Ctrl+C / Ctrl+X / Ctrl+V を `Key` の押下にせず `Event::Copy` / `Event::Cut` / `Event::Paste(文字)` に置き換える
//! （Ctrl+Shift+C も `Copy`。OS のクリップボードに文字が無ければ `Paste` も出ない。画像だけのクリップボードでは、V を押しても
//! 押下の事象が全く来ない）。V を離した事象は `Key` として来るので、Ctrl を押したまま V を離したときも貼る。egui_kittest・Web など
//! は `Key` の押下で来る。どの経路でも 1 回の押下は 1 回の操作になるよう、`Paste` / `Key` の押下を処理したら V の離しを 1 回だけ
//! 見送る（`ClipState::v_handled`）。

use egui::{Event, InputState, Key};

use super::{ClipAction, ClipState};
use crate::state::Action;

/// 今のフレームのクリップボードの操作（文字を打っている間・メニューを開いている間は呼ばない）。受け取った事象は取り除く。
pub fn shortcut_actions(i: &mut InputState, clip: &mut ClipState) -> Vec<Action> {
    let mut out = Vec::new();
    let mut push = |a: ClipAction| out.push(Action::Clip(a));
    // キーの割り当ては `keymap::CLIPBOARD_KEYS`（Shift 付きが先。consume_key は書いていない Shift を気にしない）
    for (modifiers, key, action) in crate::keymap::CLIPBOARD_KEYS {
        if i.consume_key(modifiers, key) {
            push(action);
            if action == ClipAction::Paste {
                clip.v_handled = true;
            }
        }
    }
    // egui-winit の置き換え後の事象と、V の離し。Shift は事象の並びの中の、その時点の修飾（フレームの終わりでは離している
    // こともある）で見る: フレームの初めは前のフレームの終わりの修飾、途中は `ModifiersChanged` と `Key` の修飾で変わる
    let mut modifiers = clip.last_modifiers;
    i.events.retain(|event| match event {
        Event::ModifiersChanged(m) => {
            modifiers = *m;
            true
        }
        Event::Copy => {
            push(if modifiers.shift {
                ClipAction::CopyMerged
            } else {
                ClipAction::Copy
            });
            false
        }
        Event::Cut => {
            push(ClipAction::Cut);
            false
        }
        Event::Paste(_) => {
            push(ClipAction::Paste);
            clip.v_handled = true;
            false
        }
        Event::Key {
            key,
            pressed,
            modifiers: key_modifiers,
            ..
        } => {
            modifiers = *key_modifiers;
            if *key == Key::V && !*pressed {
                if clip.v_handled {
                    clip.v_handled = false;
                } else if key_modifiers.command {
                    push(ClipAction::Paste);
                }
            }
            true
        }
        _ => true,
    });
    clip.last_modifiers = i.modifiers;
    out
}

/// キーを取らない状態（文字を打っている・メニューを開いている）でも、V の離しだけは見て、処理済みの印を片づける
/// （印が残ると、次の画像だけのクリップボードの貼り付けを 1 回見逃す）。
pub fn observe_blocked(i: &InputState, clip: &mut ClipState) {
    if i.events.iter().any(|e| {
        matches!(
            e,
            Event::Key {
                key: Key::V,
                pressed: false,
                ..
            }
        )
    }) {
        clip.v_handled = false;
    }
    clip.last_modifiers = i.modifiers;
}
