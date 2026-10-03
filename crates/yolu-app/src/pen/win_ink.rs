//! Windows Ink（WM_POINTER）。窓のプロシージャを差し替え（GWLP_WNDPROC）、ペンの WM_POINTERDOWN・UPDATE・UP で
//! GetPointerPenInfoHistory を読んでから、元のプロシージャ（winit）へ渡す。位置は himetric から画面の画素へ直してから
//! クライアント領域へ（winit と同じ求め方で、整数の画素より細かい）。履歴は新しい順に来るので、古い順に並べ直して詰める。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::Pointer::{
    GetPointerDeviceRects, GetPointerPenInfoHistory, GetPointerType, POINTER_FLAG_INCONTACT,
    POINTER_PEN_INFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, DefWindowProcW, SetWindowLongPtrW, GWLP_WNDPROC, PEN_FLAG_BARREL,
    PEN_FLAG_ERASER, PEN_FLAG_INVERTED, PEN_MASK_PRESSURE, PEN_MASK_TILT_X, PEN_MASK_TILT_Y,
    POINTER_INPUT_TYPE, PT_PEN, WM_NCDESTROY, WM_POINTERDOWN, WM_POINTERUP, WM_POINTERUPDATE,
    WNDPROC,
};

use super::PenSample;
use crate::engine::Tilt;

struct Hooked {
    previous: isize,
    queue: Arc<Mutex<Vec<PenSample>>>,
    ctx: egui::Context,
}

fn table() -> &'static Mutex<HashMap<isize, Hooked>> {
    static TABLE: OnceLock<Mutex<HashMap<isize, Hooked>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 窓のプロシージャを差し替える。同じ窓に二度は差し替えない。差し替えは表のロックを持たずに行う（窓のプロシージャが
/// 同じ表を引くので、もし差し替えの途中にメッセージが来ても止まらないように）。
pub(super) fn hook(hwnd: isize, queue: Arc<Mutex<Vec<PenSample>>>, ctx: egui::Context) -> bool {
    if table()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(&hwnd)
    {
        return true;
    }
    let ours = wndproc as *const () as isize;
    let previous = unsafe { SetWindowLongPtrW(HWND(hwnd as _), GWLP_WNDPROC, ours) };
    if previous == 0 {
        return false;
    }
    table().lock().unwrap_or_else(|e| e.into_inner()).insert(
        hwnd,
        Hooked {
            previous,
            queue,
            ctx,
        },
    );
    true
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let key = hwnd.0 as isize;
    let (previous, queue, ctx) = {
        let table = table().lock().unwrap_or_else(|e| e.into_inner());
        match table.get(&key) {
            Some(h) => (h.previous, h.queue.clone(), h.ctx.clone()),
            // 表に入る前（差し替えた直後）のメッセージは既定の処理へ（捨てない）
            None => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    };
    if matches!(msg, WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP) {
        let id = (wparam.0 & 0xFFFF) as u32;
        let samples = unsafe { read_pen(hwnd, id) };
        if !samples.is_empty() {
            queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend(samples);
            ctx.request_repaint();
        }
    }
    let previous_proc: WNDPROC = unsafe { std::mem::transmute::<isize, WNDPROC>(previous) };
    let result = unsafe { CallWindowProcW(previous_proc, hwnd, msg, wparam, lparam) };
    if msg == WM_NCDESTROY {
        table()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key);
    }
    result
}

/// ペンなら、このメッセージに溜まった履歴の点を古い順に返す（ペンでなければ空）。
unsafe fn read_pen(hwnd: HWND, id: u32) -> Vec<PenSample> {
    let mut kind = POINTER_INPUT_TYPE::default();
    if unsafe { GetPointerType(id, &mut kind) }.is_err() || kind != PT_PEN {
        return Vec::new();
    }
    let mut count = 0u32;
    if unsafe { GetPointerPenInfoHistory(id, &mut count, None) }.is_err() || count == 0 {
        return Vec::new();
    }
    let mut infos = vec![POINTER_PEN_INFO::default(); count as usize];
    if unsafe { GetPointerPenInfoHistory(id, &mut count, Some(infos.as_mut_ptr())) }.is_err() {
        return Vec::new();
    }
    infos.truncate(count as usize);
    let mut out = Vec::with_capacity(infos.len());
    for info in infos.iter().rev() {
        let p = &info.pointerInfo;
        let (mut device, mut display) = (RECT::default(), RECT::default());
        let (x, y) = if unsafe { GetPointerDeviceRects(p.sourceDevice, &mut device, &mut display) }
            .is_ok()
            && device.right > device.left
            && device.bottom > device.top
        {
            let rx = (display.right - display.left) as f64 / (device.right - device.left) as f64;
            let ry = (display.bottom - display.top) as f64 / (device.bottom - device.top) as f64;
            (
                display.left as f64 + p.ptHimetricLocation.x as f64 * rx,
                display.top as f64 + p.ptHimetricLocation.y as f64 * ry,
            )
        } else {
            (p.ptPixelLocation.x as f64, p.ptPixelLocation.y as f64)
        };
        let mut at = POINT {
            x: x.floor() as i32,
            y: y.floor() as i32,
        };
        if !unsafe { ScreenToClient(hwnd, &mut at) }.as_bool() {
            continue;
        }
        let pressure = if info.penMask & PEN_MASK_PRESSURE != 0 {
            (info.pressure as f32 / 1024.0).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let tilt = Tilt {
            x: if info.penMask & PEN_MASK_TILT_X != 0 {
                info.tiltX as f32
            } else {
                0.0
            },
            y: if info.penMask & PEN_MASK_TILT_Y != 0 {
                info.tiltY as f32
            } else {
                0.0
            },
        };
        out.push(PenSample {
            pos: [
                at.x as f32 + x.fract() as f32,
                at.y as f32 + y.fract() as f32,
            ],
            pressure,
            tilt,
            contact: p.pointerFlags.0 & POINTER_FLAG_INCONTACT.0 != 0,
            eraser: info.penFlags & (PEN_FLAG_ERASER | PEN_FLAG_INVERTED) != 0,
            barrel: info.penFlags & PEN_FLAG_BARREL != 0,
            pointer_id: id,
            time_ms: p.dwTime,
        });
    }
    out
}
