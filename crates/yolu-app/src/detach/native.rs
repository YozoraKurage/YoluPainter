//! Windows: 外の窓の OS の窓（HWND）を見つけ、主の窓を持ち主にする。
//!
//! eframe は immediate の viewport の窓のハンドルをアプリへ渡さないので、画面のスレッドの最上位の窓のうち、内側の矩形（画素）が viewport の
//! 内側の矩形と合う窓を探す（winit の窓は、どれも作ったスレッドの窓）。持ち主にすると、外の窓は主の窓より前に留まり、主の窓の最小化で
//! 一緒に隠れる（タスクバーには出さない: `ViewportBuilder::with_taskbar(false)`）。同じ矩形に重なった窓が複数あるときは、窓の題名で
//! 1 つに決める（決まらなければ繋がず、次のフレームで探し直す。取り違えると、ほかのウィンドウのペンの点を受けてしまう）。

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumThreadWindows, GetClientRect, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible,
    SetWindowLongPtrW, GWLP_HWNDPARENT,
};

use crate::windowpos::PxRect;

/// 内側の矩形が合うとみなす差（画素。丸めの差）。
const TOLERANCE: i32 = 2;

struct Search {
    client: PxRect,
    skip: Vec<isize>,
    /// 内側の矩形が合った窓（列挙の順）と、その題名。
    found: Vec<(isize, String)>,
}

/// 窓の題名（読めなければ空）。
fn window_title(hwnd: HWND) -> String {
    // SAFETY: この画面のスレッドが作った窓のハンドル。読む長さは、返った長さの中。
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut buffer);
        String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
    }
}

unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `find_window` が渡した `Search` を、列挙の間だけ指す。
    let search = unsafe { &mut *(data.0 as *mut Search) };
    let key = hwnd.0 as isize;
    if search.skip.contains(&key) || !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return BOOL(1);
    }
    let mut rect = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut rect) }.is_err() {
        return BOOL(1);
    }
    let mut origin = POINT { x: 0, y: 0 };
    if !unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool() {
        return BOOL(1);
    }
    let want = search.client;
    let near = |a: i32, b: i32| (a - b).abs() <= TOLERANCE;
    if near(origin.x, want.left)
        && near(origin.y, want.top)
        && near(rect.right - rect.left, want.width())
        && near(rect.bottom - rect.top, want.height())
    {
        search.found.push((key, window_title(hwnd)));
    }
    BOOL(1)
}

/// 画面のスレッドの見えている最上位の窓のうち、内側の矩形（画素）が `client` と合う窓（`skip` の窓を除く）。合う窓が複数あれば、題名が
/// `title` と同じ 1 つ（`detach::pick_window`）。
pub fn find_window(client: PxRect, skip: &[isize], title: &str) -> Option<isize> {
    let mut search = Search {
        client,
        skip: skip.to_vec(),
        found: Vec::new(),
    };
    // SAFETY: 列挙は呼んだスレッドの中で同期して終わり、`search` はその間生きている。
    unsafe {
        let _ = EnumThreadWindows(
            GetCurrentThreadId(),
            Some(visit),
            LPARAM(&mut search as *mut Search as isize),
        );
    }
    super::pick_window(&search.found, title)
}

/// 外の窓の持ち主を主の窓にする。
pub fn set_owner(hwnd: isize, owner: isize) {
    // SAFETY: 2 つとも、この画面のスレッドが作った窓のハンドル。持ち主の付け替えは窓のプロシージャに触れない。
    unsafe {
        SetWindowLongPtrW(HWND(hwnd as _), GWLP_HWNDPARENT, owner);
    }
}
