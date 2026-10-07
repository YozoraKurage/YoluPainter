//! 物理ピクセルの仮想デスクトップを 1 枚取得し、同じ座標の枠なしウィンドウに表示する。
use super::native_input::Pointer;
use super::*;
use ::windows::{
    core::w,
    Win32::{
        Foundation::*,
        Graphics::{Dwm::DwmFlush, Gdi::*},
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::Cell;

pub(super) fn pick(frame: &eframe::Frame, mode: Mode) -> Result<Option<[u8; 3]>, Failure> {
    let handle = frame.window_handle().map_err(|_| Failure::Overlay)?;
    let RawWindowHandle::Win32(raw) = handle.as_raw() else {
        return Err(Failure::Overlay);
    };
    // eframe の DPI 状態は保存し、セッションが終わったら戻す。
    let dpi = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if dpi.0.is_null() {
        return Err(Failure::Overlay);
    }
    struct Dpi(DPI_AWARENESS_CONTEXT);
    impl Drop for Dpi {
        fn drop(&mut self) {
            unsafe {
                SetThreadDpiAwarenessContext(self.0);
            }
        }
    }
    let _dpi = Dpi(dpi);
    let mut desktop = Native {
        owner: HWND(raw.hwnd.get() as *mut _),
        hidden: false,
    };
    session(&mut desktop, mode)
}

struct Native {
    owner: HWND,
    hidden: bool,
}
impl Desktop for Native {
    fn hide(&mut self) {
        self.hidden = true;
        unsafe {
            let _ = ShowWindow(self.owner, SW_HIDE);
        }
    }
    fn settle(&mut self) -> Result<(), Failure> {
        // 非表示がコンポジターへ反映されてから撮る。固定時間の sleep に頼らない。
        unsafe { DwmFlush().map_err(|_| Failure::Capture) }
    }
    fn capture(&mut self) -> Result<Snapshot, Failure> {
        unsafe { capture() }
    }
    fn select(&mut self, image: &Snapshot) -> Result<Option<[i32; 2]>, Failure> {
        unsafe {
            select(
                image,
                HINSTANCE(GetWindowLongPtrW(self.owner, GWLP_HINSTANCE) as *mut _),
            )
        }
    }
    fn restore(&mut self) {
        unsafe {
            if self.hidden {
                let _ = ShowWindow(self.owner, SW_SHOW);
                self.hidden = false;
            }
            let _ = SetForegroundWindow(self.owner);
        }
    }
}

fn bitmap_info(size: [i32; 2]) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size[0],
            biHeight: -size[1],
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

struct ScreenDc(HDC);
impl Drop for ScreenDc {
    fn drop(&mut self) {
        unsafe {
            ReleaseDC(None, self.0);
        }
    }
}
struct MemoryDc(HDC);
impl Drop for MemoryDc {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}
struct Bitmap(HBITMAP);
impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

unsafe fn capture() -> Result<Snapshot, Failure> {
    unsafe {
        let origin = [
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
        ];
        let size = [
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        ];
        let length = Snapshot::byte_len(size)?;
        let screen = ScreenDc(GetDC(None));
        if screen.0 .0.is_null() {
            return Err(Failure::Capture);
        }
        let memory = MemoryDc(CreateCompatibleDC(Some(screen.0)));
        if memory.0 .0.is_null() {
            return Err(Failure::Capture);
        }
        let info = bitmap_info(size);
        let mut bits = std::ptr::null_mut();
        let bitmap = Bitmap(
            CreateDIBSection(Some(screen.0), &info, DIB_RGB_COLORS, &mut bits, None, 0)
                .map_err(|_| Failure::Capture)?,
        );
        if bits.is_null() {
            return Err(Failure::Capture);
        }
        let old = SelectObject(memory.0, bitmap.0.into());
        if old.0.is_null() || old.0 as isize == -1 {
            return Err(Failure::Capture);
        }
        let result = BitBlt(
            memory.0,
            0,
            0,
            size[0],
            size[1],
            Some(screen.0),
            origin[0],
            origin[1],
            SRCCOPY | CAPTUREBLT,
        );
        let flushed = GdiFlush().as_bool();
        SelectObject(memory.0, old);
        result.map_err(|_| Failure::Capture)?;
        if !flushed {
            return Err(Failure::Capture);
        }
        let mut bgra = Vec::new();
        bgra.try_reserve_exact(length)
            .map_err(|_| Failure::Budget)?;
        bgra.extend_from_slice(std::slice::from_raw_parts(bits.cast::<u8>(), length));
        Ok(Snapshot { origin, size, bgra })
    }
}

struct Overlay<'a> {
    image: &'a Snapshot,
    point: Cell<[i32; 2]>,
    selection: Cell<Selection>,
    armed: Cell<bool>,
}
impl Overlay<'_> {
    fn input(&self, event: PickInput) {
        let mut selection = self.selection.get();
        selection.input(event);
        self.selection.set(selection);
    }
}
struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            SetWindowLongPtrW(self.0, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.0);
        }
    }
}

unsafe fn select(image: &Snapshot, instance: HINSTANCE) -> Result<Option<[i32; 2]>, Failure> {
    unsafe {
        // クラスはプロセスの間同じプロシージャを持つ。既登録なら atom = 0 でもそのまま使える。
        let class = w!("YoluPainter.ScreenPick");
        let wc = WNDCLASSW {
            hInstance: instance,
            lpfnWndProc: Some(window_proc),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_CROSS).map_err(|_| Failure::Overlay)?,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor).map_err(|_| Failure::Overlay)?;
        let overlay = Box::new(Overlay {
            image,
            point: Cell::new([cursor.x, cursor.y]),
            selection: Cell::new(Selection::default()),
            armed: Cell::new(false),
        });
        let window = Window(
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class,
                w!(""),
                WS_POPUP,
                image.origin[0],
                image.origin[1],
                image.size[0],
                image.size[1],
                None,
                None,
                Some(instance),
                None,
            )
            .map_err(|_| Failure::Overlay)?,
        );
        SetWindowLongPtrW(
            window.0,
            GWLP_USERDATA,
            (&*overlay as *const Overlay<'_>) as isize,
        );
        // SetWindowPos は最大化と違い、複数画面の負座標も縮めずに覆う。
        SetWindowPos(
            window.0,
            Some(HWND_TOPMOST),
            image.origin[0],
            image.origin[1],
            image.size[0],
            image.size[1],
            SWP_SHOWWINDOW,
        )
        .map_err(|_| Failure::Overlay)?;
        let _ = SetForegroundWindow(window.0);
        if GetForegroundWindow() != window.0 {
            return Err(Failure::Overlay);
        }
        overlay.armed.set(true);
        let _ = InvalidateRect(Some(window.0), None, false);
        while !overlay.selection.get().done {
            let mut msg = MSG::default();
            let result = GetMessageW(&mut msg, Some(window.0), 0, 0).0;
            if result == -1 {
                return Err(Failure::Overlay);
            }
            if result == 0 {
                PostQuitMessage(msg.wParam.0 as i32);
                break;
            }
            // 所有者の入力・タイマーは保留する。eframe をネストして再入しない。
            if msg.hwnd == window.0 {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        // window は overlay より先に破棄される。画像へのポインターを残さない。
        Ok(overlay.selection.get().point)
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Overlay<'_>;
        if pointer.is_null() {
            return DefWindowProcW(hwnd, message, wp, lp);
        }
        let overlay = &*pointer;
        let input = native_input::decode(message, wp.0, lp.0, &NativePointer(hwnd));
        let result = if let Some(input) = input {
            overlay.input(input);
            LRESULT(0)
        } else {
            match message {
                WM_PAINT => {
                    let mut paint = PAINTSTRUCT::default();
                    let dc = BeginPaint(hwnd, &mut paint);
                    draw(dc, overlay);
                    let _ = EndPaint(hwnd, &paint);
                    LRESULT(0)
                }
                WM_ERASEBKGND => LRESULT(1),
                WM_MOUSEMOVE => {
                    if let Some(point) = NativePointer(hwnd).cursor_position() {
                        overlay.point.set(point);
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    LRESULT(0)
                }
                WM_CLOSE | WM_RBUTTONDOWN => {
                    overlay.input(PickInput::Cancel);
                    LRESULT(0)
                }
                WM_ACTIVATE if overlay.armed.get() && wp.0 & 0xffff == WA_INACTIVE as usize => {
                    overlay.input(PickInput::Cancel);
                    LRESULT(0)
                }
                WM_DISPLAYCHANGE => {
                    overlay.input(PickInput::Cancel);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, message, wp, lp),
            }
        };
        if overlay.selection.get().done {
            // 同期のフォーカス喪失通知でも GetMessage を起こしてウィンドウを戻す。
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        }
        result
    }
}

unsafe fn draw(dc: HDC, overlay: &Overlay<'_>) {
    unsafe {
        let image = overlay.image;
        let point = overlay.point.get();
        let info = bitmap_info(image.size);
        StretchDIBits(
            dc,
            0,
            0,
            image.size[0],
            image.size[1],
            0,
            0,
            image.size[0],
            image.size[1],
            Some(image.bgra.as_ptr().cast()),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let p = POINT {
            x: point[0],
            y: point[1],
        };
        if !GetMonitorInfoW(MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST), &mut monitor).as_bool() {
            return;
        }
        let r = monitor.rcMonitor;
        let origin = loupe_origin(point, [r.left, r.top, r.right, r.bottom], [132, 156]);
        let [x, y] = [origin[0] - image.origin[0], origin[1] - image.origin[1]];
        // 11×11 画素を 12 倍。各セルは取得画像そのものから読むので、端で余白があっても中心はずれない。
        for row in 0..11 {
            for column in 0..11 {
                let rgb = image
                    .sample([point[0] + column - 5, point[1] + row - 5])
                    .unwrap_or([0; 3]);
                let rect = RECT {
                    left: x + column * 12,
                    top: y + row * 12,
                    right: x + (column + 1) * 12,
                    bottom: y + (row + 1) * 12,
                };
                fill(dc, &rect, rgb);
            }
        }
        let center = RECT {
            left: x + 60,
            top: y + 60,
            right: x + 72,
            bottom: y + 72,
        };
        let brush = CreateSolidBrush(COLORREF(0x00ffffff));
        FrameRect(dc, &center, brush);
        let _ = DeleteObject(brush.into());
        let rgb = image.sample(point).unwrap_or([0; 3]);
        fill(
            dc,
            &RECT {
                left: x,
                top: y + 132,
                right: x + 132,
                bottom: y + 156,
            },
            rgb,
        );
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(
            dc,
            COLORREF(
                if u32::from(rgb[0]) * 299 + u32::from(rgb[1]) * 587 + u32::from(rgb[2]) * 114
                    > 128000
                {
                    0
                } else {
                    0xffffff
                },
            ),
        );
        let text: Vec<u16> = format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
            .encode_utf16()
            .collect();
        let _ = TextOutW(dc, x + 8, y + 136, &text);
    }
}
unsafe fn fill(dc: HDC, rect: &RECT, rgb: [u8; 3]) {
    unsafe {
        let brush = CreateSolidBrush(COLORREF(
            u32::from(rgb[0]) | (u32::from(rgb[1]) << 8) | (u32::from(rgb[2]) << 16),
        ));
        FillRect(dc, rect, brush);
        let _ = DeleteObject(brush.into());
    }
}

struct NativePointer(HWND);
impl Pointer for NativePointer {
    fn cursor_position(&self) -> Option<[i32; 2]> {
        let mut point = POINT::default();
        unsafe {
            GetCursorPos(&mut point).ok()?;
        }
        Some([point.x, point.y])
    }
    fn client_to_screen(&self, point: [i32; 2]) -> Option<[i32; 2]> {
        let mut point = POINT {
            x: point[0],
            y: point[1],
        };
        unsafe {
            ClientToScreen(self.0, &mut point)
                .as_bool()
                .then_some([point.x, point.y])
        }
    }
}
