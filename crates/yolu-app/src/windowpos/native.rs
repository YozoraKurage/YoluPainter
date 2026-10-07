//! Windows の画面の列挙と、枠を外したウィンドウの最大化（自動で隠すタスクバーを呼び出せるように、その辺を 1 画素空ける）。
//!
//! 最大化の 1 画素: 枠を外したウィンドウは、winit が `WM_NCCALCSIZE` で、最大化中の内側を作業領域いっぱいに直す（枠の分を消すため）。
//! タスクバーを自動で隠していると作業領域は画面全体と同じなので、ウィンドウが画面の端まで覆い、端へマウスを寄せてもタスクバーが出てこない。
//! 知られた手は 3 つあり、一番素直な「最大化中の内側を、自動で隠すタスクバーのある辺だけ 1 画素縮める」を選んだ
//! （最大化の状態を OS に残したまま、winit が内側を作った後に縮められる）。
//! - `WM_GETMINMAXINFO` で最大化の位置と大きさを変える: winit の `WM_NCCALCSIZE` が内側を作業領域で上書きするので効かない。
//! - 最大化を OS に任せず、作業領域より 1 画素小さい矩形を自分で置く: Win+↑・スナップ・ダブルクリックと最大化の状態を失う。
//! - `WM_NCCALCSIZE` で縮める: 最大化の状態は OS のまま。winit の処理（ウィンドウのプロシージャ）より前に付けて、winit が内側を作った後に縮める。
//!   ウィンドウのプロシージャは winit のものを `SetWindowSubclass` で包む（ペンの入力の差し替え `pen::win_ink` とは、どちらを先に付けても連鎖する）。
//!
//! 自動で隠すタスクバーのある辺は、`SHAppBarMessage(ABM_GETAUTOHIDEBAREX)` で画面ごと・辺ごとに調べる。

use std::mem::size_of;
use std::sync::Once;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromRect, HDC, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, RemoveWindowSubclass, SHAppBarMessage, SetWindowSubclass, ABE_BOTTOM,
    ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_GETAUTOHIDEBAREX, APPBARDATA,
};
use windows::Win32::UI::WindowsAndMessaging::{
    IsWindow, IsZoomed, SetWindowPos, MONITORINFOF_PRIMARY, NCCALCSIZE_PARAMS, SPI_SETWORKAREA,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WM_NCCALCSIZE,
    WM_NCDESTROY, WM_SETTINGCHANGE,
};

use super::{maximized_client, Edges, Monitor, PxRect};

/// ウィンドウの差し替えの識別子（任意の値。同じウィンドウに付ける物の中で重ならなければよい）。
const SUBCLASS_ID: usize = 0x594F_4C55;

fn px(rect: RECT) -> PxRect {
    PxRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

fn rect(px: PxRect) -> RECT {
    RECT {
        left: px.left,
        top: px.top,
        right: px.right,
        bottom: px.bottom,
    }
}

/// プロセスを画面ごとの拡大率に対応させる（winit がウィンドウを作る前にかけるのと同じ設定を、列挙の前に 1 度だけ先にかける）。
///
/// 起動の前は、この実行ファイルに DPI の manifest が無いので、プロセスは拡大率に対応していない。そのまま列挙すると、画面の座標と
/// 拡大率が仮想化された値（拡大率は常に 1.0）になり、画素の実際の値が読めない。かといって、呼んだスレッドの文脈を一時的に切り替えて
/// 戻す形にはしない: 戻すときに、元の「非対応」がそのスレッドの明示の値として残りうる。すると、あとで winit がプロセスに対応を入れても、
/// 明示したスレッドの値が優先されて、主のスレッドで作るウィンドウが非対応になりうる（OS がウィンドウをぼかして引き伸ばす）。プロセスにかければ、
/// winit のあとの同じ呼び出しは「すでに設定済み」で失敗するだけで、結果は同じ。すでに設定されている（manifest・互換性の設定）なら変えない。
fn become_per_monitor_aware() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: 引数は定数。プロセスの設定は 1 度しか入らず、入っていれば失敗が返るだけ。
        unsafe {
            // 画面ごとの拡大率（V2）は Windows 10 の 1703 から。無ければ V1 を試す（winit と同じ順）
            if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_err() {
                let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE);
            }
        }
    });
}

/// つながっている画面の一覧（仮想スクリーンの画素。拡大率は画面の実効 DPI を 96 で割った値）。
pub fn monitors() -> Vec<Monitor> {
    become_per_monitor_aware();
    let mut list: Vec<Monitor> = Vec::new();
    // SAFETY: `collect` は `list` を、この呼び出しの間だけ（同じスレッドで同期的に）使う。
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut list as *mut Vec<Monitor> as isize),
        );
    }
    list
}

unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    // SAFETY: `monitors` が渡した、生きている `Vec<Monitor>`。
    let list = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
    if let Some(info) = info(monitor) {
        let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
        // SAFETY: 出力先は自分の変数。
        let scale =
            match unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } {
                Ok(()) if dpi_x > 0 => dpi_x as f32 / 96.0,
                _ => 1.0,
            };
        list.push(Monitor {
            bounds: px(info.rcMonitor),
            work: px(info.rcWork),
            scale,
            primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }
    BOOL(1)
}

fn info(monitor: HMONITOR) -> Option<MONITORINFO> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `cbSize` を入れた MONITORINFO を渡す（Win32 の呼び方どおり）。
    unsafe { GetMonitorInfoW(monitor, &mut info) }
        .as_bool()
        .then_some(info)
}

/// ウィンドウのプロシージャを包んで、最大化の 1 画素を空ける（実際のウィンドウだけ。ウィンドウのハンドルが取れなければ何もしない）。
pub fn install(cc: &eframe::CreationContext<'_>) {
    let Ok(handle) = cc.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(raw) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(raw.hwnd.get() as *mut _);
    // SAFETY: 自分のウィンドウ（このスレッドが作った）に、このモジュールの関数を付ける。同じ識別子で再び付けても参照値が替わるだけ。
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, 0);
    }
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    match message {
        WM_NCCALCSIZE if wparam.0 != 0 && lparam.0 != 0 => {
            // winit（この先）が、最大化中の内側を作業領域に直す。その後で縮める
            // SAFETY: 次のウィンドウのプロシージャへ、同じ引数をそのまま渡す。
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            // SAFETY: wparam が真のとき、lparam は NCCALCSIZE_PARAMS を指す（Win32 の決まり）。
            let params = unsafe { &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS) };
            if unsafe { IsZoomed(hwnd) }.as_bool() {
                if let Some(client) = leave_autohide_gap(params.rgrc[0]) {
                    params.rgrc[0] = client;
                }
            }
            result
        }
        WM_SETTINGCHANGE if wparam.0 == SPI_SETWORKAREA.0 as usize => {
            // 自動で隠すかどうかを切り替えると作業領域が変わる。最大化中なら、内側の求め直しを頼む
            // SAFETY: 次のウィンドウのプロシージャへ、同じ引数をそのまま渡す。
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            // SAFETY: 自分のウィンドウへの枠の再計算の頼み（位置・大きさ・重なりは変えない）。
            unsafe {
                if IsZoomed(hwnd).as_bool() {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        0,
                        0,
                        SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            result
        }
        WM_NCDESTROY => {
            // SAFETY: 自分が付けた物を外してから、次へ渡す。
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
        }
        // SAFETY: 次のウィンドウのプロシージャへ、同じ引数をそのまま渡す。
        _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    }
}

/// winit が作った最大化の内側（作業領域と一致するとき）から、自動で隠すタスクバーのある辺を 1 画素縮めた矩形。縮める辺が無い・
/// winit の作った内側でない（枠つきのウィンドウ。OS が自分で 1 画素を残している）なら None。
fn leave_autohide_gap(client: RECT) -> Option<RECT> {
    // SAFETY: 読むだけ。
    let monitor = unsafe { MonitorFromRect(&client, MONITOR_DEFAULTTONEAREST) };
    let info = info(monitor)?;
    if px(client) != px(info.rcWork) {
        return None;
    }
    let edges = autohide_edges(info.rcMonitor);
    edges
        .any()
        .then(|| rect(maximized_client(px(info.rcWork), edges)))
}

/// 画面 `monitor` の、自動で隠すタスクバー（アプリバー）がある辺。
fn autohide_edges(monitor: RECT) -> Edges {
    let has = |edge: u32| {
        let mut data = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            uEdge: edge,
            rc: monitor,
            ..Default::default()
        };
        // SAFETY: `cbSize` と画面の矩形を入れた APPBARDATA を渡す。戻り値はその辺のアプリバーのウィンドウ（無ければ 0）。
        let bar = unsafe { SHAppBarMessage(ABM_GETAUTOHIDEBAREX, &mut data) };
        bar != 0 && unsafe { IsWindow(Some(HWND(bar as *mut _))) }.as_bool()
    };
    Edges {
        left: has(ABE_LEFT),
        top: has(ABE_TOP),
        right: has(ABE_RIGHT),
        bottom: has(ABE_BOTTOM),
    }
}

/// `install` と同じ包みを、ウィンドウのハンドル（HWND の値）で付ける（別ウィンドウ。eframe は子ウィンドウのハンドルを渡さないので、`detach` が見つけたウィンドウ）。
/// 付けた時にもう最大化していれば、内側の求め直しを頼む（次の `WM_NCCALCSIZE` から 1 画素が空く）。
pub fn install_hwnd(hwnd: isize) {
    let hwnd = HWND(hwnd as *mut _);
    // SAFETY: この画面のスレッドが作ったウィンドウ（`detach::native::find_window` が同じスレッドのウィンドウから見つけた）に、このモジュールの関数を付ける。
    // 同じ識別子で再び付けても参照値が替わるだけ。枠の再計算の頼みは位置・大きさ・重なりを変えない。
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return;
        }
        let _ = SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, 0);
        if IsZoomed(hwnd).as_bool() {
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}
