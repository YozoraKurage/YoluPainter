//! ペンの入力。Windows では Windows Ink（WM_POINTER）を窓のプロシージャで先に受け、筆圧・傾き・消しゴムの端・サイドボタンを
//! 履歴の点ごとに読む（winit も WM_POINTER を受けて egui の Touch に変えるが、筆圧だけで、履歴の点にも最新の筆圧を付ける）。
//! 読んだあとは winit にそのまま渡すので、ペンでボタンを押すなどの画面の操作はいつもどおり egui に届く。
//! ほかの OS では何も入らず、キャンバスはマウスと egui の Touch の筆圧（winit が出せば）で描く。
//!
//! ペンの押し（触れてから離すまで）の行き先は、触れた最初の点で決めて離すまで変えない（`PenPress`）: 描く道具の押し・ビューを動かす操作
//! （回す・パン・拡縮。サイドボタン・Alt・Space・Ctrl+Space・R）・何もしない（押した所がビューの外や別の部品）。描くのは、修飾もサイドボタンも
//! 無いペン先の接触だけ。winit はペンを egui のポインタ（左ボタン）にも変えて同じ押しを二重に届けるので、ペンの点が持つ押しの間は、
//! egui のポインタの押しをビューが使わない。サイドボタンを押した接触は、egui の部品にも右ボタンとして届ける（`ButtonMap`）。

pub mod adjust;
#[cfg(windows)]
mod win_ink;
pub mod window;

use std::sync::{Arc, Mutex};

use crate::engine::Tilt;

/// ペンの押しの行き先。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressKind {
    /// 道具の押し（ブラシ・消しゴム・範囲の道具・選択の道具）。始められなかった押し（読むだけ・Alt の予約など）も、離すまでここに留まる。
    Tool,
    /// ビューを動かす操作（2D は回す・パン・拡縮、3D は回す・パン・拡縮・クローンの元）。
    View,
    /// 何もしない（押した所がビューの外・別の部品の上・ポップアップの下、サイドボタンの 2D、修飾を押した描く道具）。
    Ignored,
}

/// ビューが持つ、ペンの今の押し（触れてから離すまで）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenPress {
    pub id: u32,
    pub kind: PressKind,
    /// このペンの前の点（ビューを動かす量を求める）。
    pub last: egui::Pos2,
}

/// ペンの 1 点（位置は窓のクライアント領域の物理の画素、左上が原点）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenSample {
    pub pos: [f32; 2],
    /// 0〜1。
    pub pressure: f32,
    pub tilt: Tilt,
    /// ペンの軸まわりの回転（度、時計回り 0〜360）。回転を送れないペンは None（0° を送るペンの Some(0.0) とは別。
    /// core は回転が 0 でないときだけ「情報あり」と見なすので、None は core へ回転を渡さない）。
    pub rotation: Option<f32>,
    /// 紙に触れている。
    pub contact: bool,
    /// 消しゴムの端（裏返して近づけている、または触れている）。
    pub eraser: bool,
    /// サイドボタン。
    pub barrel: bool,
    pub pointer_id: u32,
    /// OS の時刻（ミリ秒）。
    pub time_ms: u32,
}

impl PenSample {
    /// egui の点（論理の座標）。
    pub fn pos_points(&self, pixels_per_point: f32) -> egui::Pos2 {
        egui::pos2(
            self.pos[0] / pixels_per_point,
            self.pos[1] / pixels_per_point,
        )
    }
}

/// ペンの点の受け口（窓のプロシージャが詰め、画面のフレームが取り出す）。
#[derive(Clone, Default)]
pub struct PenInput {
    queue: Arc<Mutex<Vec<PenSample>>>,
    hooked: bool,
}

impl PenInput {
    /// 窓に繋がない受け口（試験と、Windows 以外）。
    pub fn detached() -> PenInput {
        PenInput::default()
    }

    /// eframe の窓に繋ぐ（Windows だけ。ほかの OS と窓の無い試験では何もしない）。
    pub fn attach(cc: &eframe::CreationContext<'_>) -> PenInput {
        let input = PenInput::detached();
        #[cfg(windows)]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = cc.window_handle() {
                if let RawWindowHandle::Win32(h) = handle.as_raw() {
                    let hooked =
                        win_ink::hook(h.hwnd.get(), input.queue.clone(), cc.egui_ctx.clone());
                    return PenInput { hooked, ..input };
                }
            }
        }
        let _ = cc;
        input
    }

    /// 窓に繋がっている（Windows Ink の点が来る）か。
    pub fn is_hooked(&self) -> bool {
        self.hooked
    }

    /// 溜まった点を取り出す（古い順）。
    pub fn drain(&self) -> Vec<PenSample> {
        std::mem::take(&mut *self.queue.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// 溜まった点を取り出さずに見る（古い順。egui の入力を直す `ButtonMap` が、このフレームのサイドボタンを読む）。
    pub fn peek(&self) -> Vec<PenSample> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 点を足す（試験が Windows の窓の代わりに使う）。
    pub fn push(&self, sample: PenSample) {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(sample);
    }
}

/// egui のポインタ（winit がペンの Touch から作る左ボタンの代わりの入力）のうち、サイドボタンを押したペンの接触を右ボタンに直す。
/// 押しの始めのサイドボタンの状態で、離すまで決める（途中でサイドボタンを離しても、押した右ボタンを右ボタンのまま離す）。
/// ペンの点が来ていないフレームのポインタ（マウス・指）は直さない。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonMap {
    secondary: bool,
}

impl ButtonMap {
    /// events を直す。`samples` は同じフレームに届いたペンの点（古い順）。
    pub fn remap(&mut self, samples: &[PenSample], events: &mut [egui::Event]) {
        for event in events {
            let egui::Event::PointerButton {
                button, pressed, ..
            } = event
            else {
                continue;
            };
            if *button != egui::PointerButton::Primary {
                continue;
            }
            if *pressed {
                // 押しの始めの点のサイドボタン（最初の接触の点。無ければペンの押しではない）
                self.secondary = samples.iter().find(|s| s.contact).is_some_and(|s| s.barrel);
            }
            if self.secondary {
                *button = egui::PointerButton::Secondary;
            }
            if !*pressed {
                self.secondary = false;
            }
        }
    }

    /// 右ボタンの押しの途中か（試験用）。
    pub fn is_secondary(&self) -> bool {
        self.secondary
    }
}
