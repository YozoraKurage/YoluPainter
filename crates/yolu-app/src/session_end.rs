//! OS のログオフ・シャットダウンが保存の途中に来たとき、保存が終わるまで待ってもらう（Windows だけ。ほかの OS は何もしない）。
//!
//! winit は `WM_QUERYENDSESSION` を窓の閉じる頼みにしないので、窓の閉じる・終了メニューと違い、何もしなければ保存の途中でも OS が
//! アプリを終わらせてしまう。保存の間だけ、OS に待ってもらう理由（`ShutdownBlockReasonCreate`）を出し、`WM_QUERYENDSESSION` には
//! 「まだ終われない」（FALSE）と答える。保存が終われば理由を消す。保存していないとき（理由が無いとき）の `WM_QUERYENDSESSION` と、ほかの
//! メッセージは、元の窓のプロシージャ（ペンの入力の差し替え・winit）へそのまま渡す。
//!
//! 保存の仕事そのものは止めない（検証した一時ファイルからの 1 回の置換で確定する）ので、待たずに強制的に終わらされても、保存先は前の
//! 中身のままか新しい中身で、途中の物は残らない（次の保存が一時ファイルを片付ける）。

/// 保存の間か（窓のプロシージャが読む。`set_saving` が書く）。
#[cfg(windows)]
static SAVING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 窓に繋ぐ（Windows の eframe の窓だけ。ほかの OS と窓の無い試験では何もしない）。繋げたら true。
pub fn attach(cc: &eframe::CreationContext<'_>) -> bool {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = cc.window_handle() {
            if let RawWindowHandle::Win32(h) = handle.as_raw() {
                return win::hook(h.hwnd.get());
            }
        }
    }
    let _ = cc;
    false
}

/// 保存の仕事の有無を知らせる（毎フレーム。変わったときだけ OS へ伝える）。`reason` は OS が出す「待っている理由」（利用者の言語）。
pub fn set_saving(saving: bool, reason: &str) {
    #[cfg(windows)]
    win::set_saving(saving, reason);
    #[cfg(not(windows))]
    let _ = (saving, reason);
}

#[cfg(windows)]
mod win {
    use std::sync::atomic::{AtomicIsize, Ordering};

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, SetWindowLongPtrW, GWLP_WNDPROC, WM_NCDESTROY, WM_QUERYENDSESSION, WNDPROC,
    };

    use super::SAVING;

    /// 差し替えた窓と、差し替える前のプロシージャ（0 は差し替えていない）。窓は 1 つだけ。
    static WINDOW: AtomicIsize = AtomicIsize::new(0);
    static PREVIOUS: AtomicIsize = AtomicIsize::new(0);

    pub(super) fn hook(hwnd: isize) -> bool {
        if WINDOW.load(Ordering::Acquire) == hwnd {
            return true;
        }
        let ours = wndproc as *const () as isize;
        // 先に前のプロシージャを置く（差し替えた直後のメッセージが、前のプロシージャへ渡せるように）。差し替えられなければ戻す
        let previous = unsafe { SetWindowLongPtrW(HWND(hwnd as _), GWLP_WNDPROC, ours) };
        if previous == 0 {
            return false;
        }
        PREVIOUS.store(previous, Ordering::Release);
        WINDOW.store(hwnd, Ordering::Release);
        true
    }

    pub(super) fn set_saving(saving: bool, reason: &str) {
        let hwnd = WINDOW.load(Ordering::Acquire);
        if hwnd == 0 || SAVING.swap(saving, Ordering::AcqRel) == saving {
            return;
        }
        let hwnd = HWND(hwnd as _);
        if saving {
            let text: Vec<u16> = reason.encode_utf16().chain(std::iter::once(0)).collect();
            // 理由を出せなくても、保存は続く（OS が待ってくれないだけ）
            let _ = unsafe { ShutdownBlockReasonCreate(hwnd, PCWSTR(text.as_ptr())) };
        } else {
            let _ = unsafe { ShutdownBlockReasonDestroy(hwnd) };
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // 保存の途中の終了の問い合わせには「まだ終われない」と答える（OS は出した理由を見せて待つ）
        if msg == WM_QUERYENDSESSION && SAVING.load(Ordering::Acquire) {
            return LRESULT(0);
        }
        let previous = PREVIOUS.load(Ordering::Acquire);
        if previous == 0 {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        let previous_proc: WNDPROC = unsafe { std::mem::transmute::<isize, WNDPROC>(previous) };
        let result = unsafe { CallWindowProcW(previous_proc, hwnd, msg, wparam, lparam) };
        if msg == WM_NCDESTROY {
            SAVING.store(false, Ordering::Release);
            WINDOW.store(0, Ordering::Release);
        }
        result
    }
}
