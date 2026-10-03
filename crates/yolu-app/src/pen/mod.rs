//! ペンの入力。Windows では Windows Ink（WM_POINTER）を窓のプロシージャで先に受け、筆圧・傾き・消しゴムの端・サイドボタンを
//! 履歴の点ごとに読む（winit も WM_POINTER を受けて egui の Touch に変えるが、筆圧だけで、履歴の点にも最新の筆圧を付ける）。
//! 読んだあとは winit にそのまま渡すので、ペンでボタンを押すなどの画面の操作はいつもどおり egui に届く。
//! ほかの OS では何も入らず、キャンバスはマウスと egui の Touch の筆圧（winit が出せば）で描く。

#[cfg(windows)]
mod win_ink;

use std::sync::{Arc, Mutex};

use crate::engine::Tilt;

/// ペンの 1 点（位置は窓のクライアント領域の物理の画素、左上が原点）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenSample {
    pub pos: [f32; 2],
    /// 0〜1。
    pub pressure: f32,
    pub tilt: Tilt,
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

    /// 点を足す（試験が Windows の窓の代わりに使う）。
    pub fn push(&self, sample: PenSample) {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(sample);
    }
}
