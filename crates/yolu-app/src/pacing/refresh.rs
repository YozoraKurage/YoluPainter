//! Windows: ウィンドウのあるモニターのリフレッシュレート（Hz）。`MonitorFromWindow` → `GetMonitorInfoW`（モニターの装置名）→
//! `EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS)` の `dmDisplayFrequency`。モニターをまたいだ・表示の設定が変わったときは、
//! 呼び手（`YoluApp::read_refresh_rate`）が 1 秒に 1 度読み直すので、追いつくまで最大 1 秒かかる。

use std::mem::size_of;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    EnumDisplaySettingsW, GetMonitorInfoW, MonitorFromWindow, DEVMODEW, ENUM_CURRENT_SETTINGS,
    MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};

/// ウィンドウ（HWND の値）のあるモニターの、今のリフレッシュレート。読めなければ `None`（OS が「既定」と答えたときも）。
pub fn monitor_refresh_hz(hwnd: isize) -> Option<f64> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let mut mode = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    // SAFETY: `cbSize`・`dmSize` を入れた構造体を渡す（Win32 の呼び方どおり）。MONITORINFOEXW は MONITORINFO の先頭に同じ並びで続けた物。
    // 装置名は GetMonitorInfoW が入れた、終端つきの UTF-16。
    unsafe {
        let monitor = MonitorFromWindow(HWND(hwnd as *mut _), MONITOR_DEFAULTTONEAREST);
        if !GetMonitorInfoW(
            monitor,
            (&mut info as *mut MONITORINFOEXW).cast::<MONITORINFO>(),
        )
        .as_bool()
        {
            return None;
        }
        if !EnumDisplaySettingsW(
            PCWSTR(info.szDevice.as_ptr()),
            ENUM_CURRENT_SETTINGS,
            &mut mode,
        )
        .as_bool()
        {
            return None;
        }
    }
    super::refresh_hz_from_display_frequency(mode.dmDisplayFrequency)
}

/// 複数のウィンドウのあるモニターのうち、いちばん速いリフレッシュレート（読めたものだけ。どれも読めなければ `None`）。
pub fn fastest_refresh_hz(hwnds: &[isize]) -> Option<f64> {
    super::fastest(hwnds.iter().map(|&hwnd| monitor_refresh_hz(hwnd)))
}
