//! ペンの入力。Windows では Windows Ink（WM_POINTER）をウィンドウのプロシージャで先に受け、筆圧・傾き・消しゴムの端・サイドボタンを
//! 履歴の点ごとに読む（winit も WM_POINTER を受けて egui の Touch に変えるが、筆圧だけで、履歴の点にも最新の筆圧を付ける）。
//! 読んだあとは winit にそのまま渡すので、ペンでボタンを押すなどの画面の操作はいつもどおり egui に届く。
//! macOS では、アプリの NSEvent のローカルの監視（`mac_tablet`）で、Wacom・XP-Pen などのドライバーが標準の NSEvent に載せるタブレットの点（筆圧・傾き・消しゴムの端・
//! サイドボタン）を読む（試し。設定で切れる）。winit は macOS のタブレットの筆圧を捨て、マウスの左ボタンにして渡すだけなので、読んだあとのイベントは返して、
//! winit とアプリがいつもどおり受ける。値の直しは `tablet`（OS に依らない）。
//! ほかの OS では何も入らず、キャンバスはマウスと egui の Touch の筆圧（winit が出せば）で描く。
//!
//! ペンの押し（触れてから離すまで）の行き先は、触れた最初の点で決めて離すまで変えない（`PenPress`）: 描くツールの押し・ビューを動かす操作
//! （回す・パン・拡縮。Alt・Space・Ctrl+Space、3D ではサイドボタン）・スポイト（2D のサイドボタン）・何もしない（押した所がビューの外や別の部品）。
//! 描くのは、修飾もサイドボタンも無いペン先の接触だけ。winit はペンを egui のポインタ（左ボタン）にも変えて同じ押しを二重に届けるので、ペンの点が持つ押しの間は、
//! egui のポインタの押しをビューが使わない。サイドボタンを押した接触は、egui の部品にも右ボタンとして届ける（`ButtonMap`）。

pub mod adjust;
#[cfg(target_os = "macos")]
mod mac_tablet;
pub mod tablet;
#[cfg(windows)]
mod win_ink;
pub mod window;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::engine::Tilt;

/// ペンの押しの行き先。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressKind {
    /// ツールの押し（ブラシ・消しゴム・範囲のツール・選択のツール）。始められなかった押し（読むだけ・Alt の予約など）も、離すまでここに留まる。
    Tool,
    /// ビューを動かす操作（2D は回す・パン・拡縮、3D は回す・パン・拡縮・クローンの元）。
    View,
    /// 2D のサイドボタン（右ボタンと同じ）: 押した所の色を見本にして、離して取るスポイト。
    Eyedrop,
    /// 何もしない（押した所がビューの外・別の部品の上・ポップアップの下、ポリゴン塗りつぶしの 2D のサイドボタン、修飾を押した描くツール）。
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

/// ペンの 1 点（位置はウィンドウのクライアント領域の物理の画素、左上が原点）。
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

/// ペンの点の受け口（ウィンドウのプロシージャが詰め、画面のフレームが取り出す）。
#[derive(Clone, Default)]
pub struct PenInput {
    queue: Arc<Mutex<Vec<PenSample>>>,
    hooked: bool,
    /// macOS のタブレットの点を詰めるか（設定「タブレットの筆圧（試し）」。別ウィンドウの受け口と同じ札を共有する）。ほかの OS では使わない。
    tablet: Arc<AtomicBool>,
}

impl PenInput {
    /// ウィンドウに繋がない受け口（試験と、Windows 以外）。
    pub fn detached() -> PenInput {
        PenInput::default()
    }

    /// eframe のウィンドウに繋ぐ（Windows と macOS だけ。ほかの OS とウィンドウの無い試験では何もしない）。
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
        #[cfg(target_os = "macos")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = cc.window_handle() {
                if let RawWindowHandle::AppKit(h) = handle.as_raw() {
                    // 設定を読むのはこのあと（`YoluApp::with_settings` が札を合わせる）。それまでは既定の入
                    input.tablet.store(true, Ordering::Relaxed);
                    let hooked = mac_tablet::hook_view(
                        h.ns_view,
                        input.queue.clone(),
                        cc.egui_ctx.clone(),
                        input.tablet.clone(),
                    );
                    return PenInput { hooked, ..input };
                }
            }
        }
        let _ = cc;
        input
    }

    /// ウィンドウのハンドル（HWND の値）に繋ぐ（Windows の別ウィンドウ。eframe は子ウィンドウのハンドルを渡さないので、`detach` が見つけたウィンドウ）。
    #[cfg(windows)]
    pub fn attach_hwnd(hwnd: isize, ctx: &egui::Context) -> PenInput {
        let input = PenInput::detached();
        let hooked = win_ink::hook(hwnd, input.queue.clone(), ctx.clone());
        PenInput { hooked, ..input }
    }

    /// 題名で見つけた macOS の別ウィンドウに繋ぐ（eframe は別ウィンドウの NSWindow を渡さないので、アプリの全ウィンドウから、題名が合うまだ繋いでいない 1 つを探す。
    /// ビューポートにフォーカスがある `focused` ときは、キーウィンドウを先に見る）。見つからない・複数ある ときは None（次のフレームで探し直す）。
    /// タブレットの入切の札は `main` と共有する。
    #[cfg(target_os = "macos")]
    pub fn attach_titled(
        title: &str,
        focused: bool,
        ctx: &egui::Context,
        main: &PenInput,
    ) -> Option<PenInput> {
        let input = PenInput {
            tablet: main.tablet.clone(),
            ..PenInput::detached()
        };
        mac_tablet::hook_titled(
            title,
            focused,
            input.queue.clone(),
            ctx.clone(),
            input.tablet.clone(),
        )
        .then_some(PenInput {
            hooked: true,
            ..input
        })
    }

    /// ウィンドウに繋がっている（Windows Ink の点、または macOS のタブレットの点が来る）か。
    pub fn is_hooked(&self) -> bool {
        self.hooked && (cfg!(not(target_os = "macos")) || self.tablet.load(Ordering::Relaxed))
    }

    /// ウィンドウに繋いである（macOS の別ウィンドウが、繋ぎ終わったかを見る。設定で切ってあっても繋いである）。
    #[cfg(target_os = "macos")]
    pub(crate) fn is_window_hooked(&self) -> bool {
        self.hooked
    }

    /// macOS のタブレットの点を読むか（設定「タブレットの筆圧（試し）」）を合わせる。切ったとき、触れたままのペンは離したことにする
    /// （離しの点が来なくなって、ペンの押しがキャンバスに残らないように）。ほかの OS では何もしない。
    pub fn set_tablet(&self, on: bool) {
        if self.tablet.swap(on, Ordering::Relaxed) != on && !on {
            #[cfg(target_os = "macos")]
            mac_tablet::release_touching();
        }
    }

    /// 札の今の値（試験用）。
    pub fn tablet_on(&self) -> bool {
        self.tablet.load(Ordering::Relaxed)
    }

    /// 溜まった点を取り出す（古い順）。
    pub fn drain(&self) -> Vec<PenSample> {
        std::mem::take(&mut *self.queue.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// 溜まった点を取り出さずに見る（古い順。egui の入力を直す `ButtonMap` が、このフレームのサイドボタンを読む）。
    pub fn peek(&self) -> Vec<PenSample> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 点を足す（試験が Windows のウィンドウの代わりに使う）。
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

#[cfg(test)]
mod tests {
    use super::*;

    /// タブレットの入切の札は、複製（別ウィンドウの受け口が持つ）と共有する。ウィンドウに繋がっていない受け口は、札によらず繋がっていない。
    #[test]
    fn the_tablet_switch_is_shared_by_clones_and_an_input_without_a_window_is_never_hooked() {
        let main = PenInput::detached();
        let copy = main.clone();
        assert!(!main.tablet_on());
        main.set_tablet(true);
        assert!(copy.tablet_on());
        assert!(!main.is_hooked(), "ウィンドウに繋がっていない");
        copy.set_tablet(false);
        assert!(!main.tablet_on());
        // 切っても、溜まった点は取り出せる（捨てない）
        main.push(PenSample {
            pos: [1.0, 2.0],
            pressure: 0.5,
            tilt: Tilt::default(),
            rotation: None,
            contact: true,
            eraser: false,
            barrel: false,
            pointer_id: 0,
            time_ms: 0,
        });
        copy.set_tablet(true);
        copy.set_tablet(false);
        assert_eq!(main.drain().len(), 1);
    }
}
