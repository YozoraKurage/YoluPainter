//! WinTab（`Wintab32.dll`）の殻（Windows だけ）。値の直しは OS に依らない `wintab`。ウィンドウのプロシージャの差し替えは `win_ink` が持ち、WinTab のメッセージと
//! ウィンドウの前後・画面の変更をここへ渡す。
//!
//! - `Wintab32.dll` は実行のときに読む（リンクの時には要らない）。System32 だけから探し（作業フォルダーの偽物を読まない）、プロセスが終わるまで外さない。名前の欄が
//!   1 バイト文字の版（`WTInfoA`・`WTOpenA`・`WTGetA`）を使う。無い・応えないときは、使えない理由を持つ（`Unavailable`）。ドライバーがプロセスの例外フィルター
//!   （落ちた記録の入り口）を差し替えることがあるので、ドライバーを呼ぶ所（読み込み・文脈の開閉・パケットの取得・問い合わせ）は全部、前後で前の物に戻す（`guarded`）。
//! - 文脈はウィンドウごと（メインのウィンドウと、別ウィンドウのそれぞれ）。無効で開き、ウィンドウが前に来たとき有効にして最前面へ（`WTEnable`・`WTOverlap`）、
//!   後ろへ行ったとき無効にする。設定を WinTab から Windows Ink へ替える・ウィンドウが閉じるときは `WTClose`。
//! - パケットは `WT_PACKET` で来た回に、溜まっている分を全部取る（`WTPacketsGet`）。ペンの押しの前（`WM_POINTERDOWN`・マウスのボタンの押し）にも先に取り、押しより前の
//!   パケットが先に受け口へ入るようにする。
//! - ペンの 1 本の押しは、始まりの点が先に来た側（WinTab か Windows Ink）の物にして、離すまで替えない（`wintab::PressOwner`）。後ろにあるウィンドウをペンで押したときのように、
//!   押した瞬間に WinTab のパケットが無ければ Windows Ink の物（そのあと WinTab の点が来ても、その押しの間は入れない）。押しが 2 本になる・始まりが欠けるのを、これで避ける。
//! - ドライバーを呼んでいる間は、状態のロックを持たない（`Tab::with_session`: セッションを取り出して使い、終わったら戻す。呼び出しの中で同じウィンドウにメッセージが送られても、
//!   プロシージャはロックを待たずに、ウィンドウの前後・文脈の閉じ・覆われを処理できる）。
//! - 文脈をドライバーが閉じたとき（`WT_CTXCLOSE`）は、こちらも閉じた状態にして、次の同期で開き直す。最前面でなくなったとき（`WT_CTXOVERLAP` に最前面の印が無い）は、触れたままの点を離す。
//!
//! 環境変数 `YOLU_PEN_LOG` にファイルの名前があるときだけ、受けたパケットの生の値と直した点を 1 行ずつ追記する（実機で確かめるため。画面には何も出さない）。

use std::collections::HashMap;
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::{w, PCSTR};
use windows::Win32::Foundation::{HMODULE, HWND, POINT};
use windows::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter;
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use super::capture::{self, Loss};
use super::wintab::{
    self, batch_positions, map_name, sample_from_packet, Axis, CursorInfo, Device, InkMessage,
    LogContext, OrientationAxes, OutputRange, Owner, PacketLayout, PressOwner, RawPacket, Rect,
    ScreenMap, Touch, Unavailable, CSR_CAPABILITIES, CSR_NAME, CSR_PKTDATA, CSR_TYPE,
    DVC_NPRESSURE, DVC_ORIENTATION, IFC_NDEVICES, IFC_WINTABID, WTI_CURSORS, WTI_DEFSYSCTX,
    WTI_DEVICES, WTI_INTERFACE, WT_CTXCLOSE, WT_CTXOPEN, WT_CTXOVERLAP, WT_CTXUPDATE, WT_DEFBASE,
    WT_INFOCHANGE, WT_PACKET, WT_PROXIMITY,
};
use super::{PenSample, WintabNotice};

// ───────── Wintab32.dll ─────────

type Info = unsafe extern "system" fn(u32, u32, *mut c_void) -> u32;
type Open = unsafe extern "system" fn(HWND, *mut LogContext, i32) -> isize;
type Get = unsafe extern "system" fn(isize, *mut LogContext) -> i32;
type Close = unsafe extern "system" fn(isize) -> i32;
type PacketsGet = unsafe extern "system" fn(isize, i32, *mut c_void) -> i32;
type Flag = unsafe extern "system" fn(isize, i32) -> i32;
type QueueGet = unsafe extern "system" fn(isize) -> i32;
type QueueSet = unsafe extern "system" fn(isize, i32) -> i32;

struct Api {
    info: Info,
    open: Open,
    get: Get,
    close: Close,
    packets_get: PacketsGet,
    enable: Flag,
    overlap: Flag,
    queue_size_get: QueueGet,
    queue_size_set: QueueSet,
}

/// `Wintab32.dll` の関数を読む（プロセスで 1 度。失敗も覚える）。
fn api() -> Result<&'static Api, Unavailable> {
    static API: OnceLock<Result<Api, Unavailable>> = OnceLock::new();
    API.get_or_init(load_api).as_ref().map_err(|e| *e)
}

fn load_api() -> Result<Api, Unavailable> {
    // SAFETY: 例外フィルターの読み出し（None を入れて前の値を受け、すぐ戻す）。
    let filter = unsafe { SetUnhandledExceptionFilter(None) };
    unsafe { SetUnhandledExceptionFilter(filter) };
    // SAFETY: 定数の名前。System32 だけから探す。
    let module = unsafe { LoadLibraryExW(w!("Wintab32.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) };
    let result = match module {
        // SAFETY: 読んだモジュールから、WinTab の仕様の名前と型で関数を取る。
        Ok(module) => unsafe {
            (|| {
                Some(Api {
                    info: symbol(module, b"WTInfoA\0")?,
                    open: symbol(module, b"WTOpenA\0")?,
                    get: symbol(module, b"WTGetA\0")?,
                    close: symbol(module, b"WTClose\0")?,
                    packets_get: symbol(module, b"WTPacketsGet\0")?,
                    enable: symbol(module, b"WTEnable\0")?,
                    overlap: symbol(module, b"WTOverlap\0")?,
                    queue_size_get: symbol(module, b"WTQueueSizeGet\0")?,
                    queue_size_set: symbol(module, b"WTQueueSizeSet\0")?,
                })
            })()
            .ok_or(Unavailable::NoDriver)
        },
        Err(_) => Err(Unavailable::NoLibrary),
    };
    // ドライバーが差し替えたかもしれない例外フィルターを戻す
    unsafe { SetUnhandledExceptionFilter(filter) };
    result
}

/// モジュールの関数を、関数の型 `T`（ポインタと同じ大きさ）で取る。
unsafe fn symbol<T: Copy>(module: HMODULE, name: &'static [u8]) -> Option<T> {
    assert_eq!(size_of::<T>(), size_of::<usize>());
    // SAFETY: 名前は NUL で終わる定数。
    let proc = unsafe { GetProcAddress(module, PCSTR(name.as_ptr())) }?;
    // SAFETY: 関数のポインタを、同じ大きさの関数の型へ。
    Some(unsafe { std::mem::transmute_copy::<_, T>(&proc) })
}

/// `WTInfo` の置き場に足す余り（バイト）。
const INFO_SLACK: usize = 16;

impl Api {
    /// `WTInfo` の答えのバイト列（項目の大きさを先に聞いて、その大きさの置き場で受ける）。無い・答えないなら None。
    fn info_bytes(&self, category: u32, index: u32) -> Option<Vec<u8>> {
        // SAFETY: 出力先が NULL のときは大きさだけが返る。
        let size = unsafe { (self.info)(category, index, null_mut()) } as usize;
        if size == 0 || size > 4096 {
            return None;
        }
        // 聞いた大きさより少し余らせた置き場を渡す（答えが聞いたときより長いドライバーでも溢れない）。答えの長さは返り値
        let mut bytes = vec![0u8; size + INFO_SLACK];
        // SAFETY: 置き場は聞いた大きさ + 余り。
        let got = unsafe { (self.info)(category, index, bytes.as_mut_ptr().cast()) } as usize;
        (got > 0).then(|| {
            bytes.truncate(got.min(bytes.len()));
            bytes
        })
    }

    /// 数 1 つの項目（1・2・4 バイトのどれで答えるドライバーでも読める）。
    fn info_u32(&self, category: u32, index: u32) -> Option<u32> {
        let bytes = self.info_bytes(category, index)?;
        let mut value = [0u8; 4];
        let n = bytes.len().min(4);
        value[..n].copy_from_slice(&bytes[..n]);
        Some(u32::from_le_bytes(value))
    }

    /// 構造体の項目（`T` の大きさに足りない答えは None）。`T` は全部の並びが 4 バイトの整数の構造体。
    fn info_value<T: Copy>(&self, category: u32, index: u32) -> Option<T> {
        let bytes = self.info_bytes(category, index)?;
        (bytes.len() >= size_of::<T>()).then(|| {
            // SAFETY: 長さを確かめた。`T` はどんなビットの並びでも正しい整数だけの構造体。
            unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<T>()) }
        })
    }

    fn info_string(&self, category: u32, index: u32) -> Option<String> {
        let bytes = self.info_bytes(category, index)?;
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }
}

// ───────── 調べるための記録 ─────────

fn log_file() -> Option<&'static Mutex<BufWriter<File>>> {
    static LOG: OnceLock<Option<Mutex<BufWriter<File>>>> = OnceLock::new();
    LOG.get_or_init(|| {
        let path = std::env::var_os("YOLU_PEN_LOG").filter(|p| !p.is_empty())?;
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(|f| Mutex::new(BufWriter::new(f)))
    })
    .as_ref()
}

fn log_enabled() -> bool {
    log_file().is_some()
}

/// 記録に 1 行（先頭は OS の時刻のミリ秒）。
fn log_line(line: &str) {
    if let Some(file) = log_file() {
        let mut w = file.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: 引数無し。
        let _ = writeln!(w, "{} {line}", unsafe { GetTickCount() });
    }
}

fn log_flush() {
    if let Some(file) = log_file() {
        let _ = file.lock().unwrap_or_else(|e| e.into_inner()).flush();
    }
}

/// WinTab のメッセージ（動きを伴わない物）の記録。
fn log_message(what: &str, offset: u32, lparam: isize) {
    if log_enabled() {
        log_line(&format!("{what} offset={offset} lparam=0x{lparam:x}"));
        log_flush();
    }
}

/// 記録に 1 行（`YOLU_PEN_LOG` があるときだけ。行は記録が有効なときにだけ作る）。
pub(super) fn log_event(line: impl FnOnce() -> String) {
    if log_enabled() {
        log_line(&line());
        log_flush();
    }
}

/// Windows Ink の点の記録（WinTab と見比べるため）。
pub(super) fn log_ink(samples: &[PenSample]) {
    if log_enabled() {
        for s in samples {
            log_line(&format!("ink {}", wintab::log_sample(s, None, "ink")));
        }
        log_flush();
    }
}

// ───────── 画面 ─────────

/// 仮想デスクトップ全体。
fn desktop_rect() -> Option<Rect> {
    // SAFETY: 引数は定数。
    let (x, y, w, h) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    Some(Rect::new(x, y, w, h)).filter(Rect::is_valid)
}

fn monitor_rects() -> Vec<Rect> {
    crate::windowpos::monitors()
        .iter()
        .map(|m| {
            Rect::new(
                m.bounds.left,
                m.bounds.top,
                m.bounds.right - m.bounds.left,
                m.bounds.bottom - m.bounds.top,
            )
        })
        .collect()
}

/// OS のカーソルの位置（物理の画素）。
fn cursor_position() -> Option<[f64; 2]> {
    let mut at = POINT::default();
    // SAFETY: 出力先は自分の変数。
    unsafe { GetCursorPos(&mut at) }
        .ok()
        .map(|_| [at.x as f64, at.y as f64])
}

/// ドライバーが文脈に書いた画面の範囲（広さの無い値は使わない）。
fn driver_rect(lc: &LogContext) -> Option<Rect> {
    Some(Rect::new(
        lc.sys_org[0],
        lc.sys_org[1],
        lc.sys_ext[0],
        lc.sys_ext[1],
    ))
    .filter(Rect::is_valid)
}

/// 画面への写し先の候補。相対モード（`lcSysMode` が 0 でない）は、パケットの位置を画面へ写せないので、候補を試さず、点の位置はいつもカーソル。
fn screen_map(lc: &LogContext) -> ScreenMap {
    if lc.sys_mode != 0 {
        return ScreenMap::none();
    }
    ScreenMap::new(driver_rect(lc), &monitor_rects(), desktop_rect())
}

// ───────── 開いた文脈 ─────────

/// 開いた文脈 1 つ。
struct Session {
    api: &'static Api,
    hctx: isize,
    msg_base: u32,
    range: OutputRange,
    device: Device,
    map: ScreenMap,
    cursors: HashMap<u32, CursorInfo>,
    touch: Touch,
    /// ドライバーが出すパケットのバイト列の並び（開いた文脈の `lcPktData`）。
    layout: PacketLayout,
    /// `WTPacketsGet` の受け皿（文脈の待ち行列の大きさ × 最大のパケットのバイト数）。
    buffer: Vec<u8>,
}

/// 待ち行列の大きさの候補（大きい方から。ドライバーが受けられた最初の大きさにする）。
const QUEUE_SIZES: [i32; 6] = [1024, 512, 256, 128, 64, 32];
/// 1 回に取る点の上限（異常なドライバーで終わらなくならないように）。
const MAX_BATCH: usize = 1 << 16;
/// 同じ文脈をドライバーが続けて閉じたとき、この時間のうちなら、次にウィンドウが前に来るまで開き直さない（開いては閉じる繰り返しにならないように）。
const REOPEN_AFTER: Duration = Duration::from_secs(5);

/// ドライバーを呼ぶ間、例外フィルターを守る（ドライバーが差し替えることがあり、差し替わると落ちた記録が残らなくなる）。
fn guarded<T>(f: impl FnOnce() -> T) -> T {
    // SAFETY: 例外フィルターの読み出しと書き戻し。
    let filter = unsafe { SetUnhandledExceptionFilter(None) };
    unsafe { SetUnhandledExceptionFilter(filter) };
    let out = f();
    unsafe { SetUnhandledExceptionFilter(filter) };
    out
}

fn open_session(hwnd: HWND) -> Result<Session, Unavailable> {
    let api = api()?;
    guarded(|| open_inner(api, hwnd))
}

fn open_inner(api: &'static Api, hwnd: HWND) -> Result<Session, Unavailable> {
    // WinTab のサービスが使えるか（0 は、ドライバーが応えない・タブレットが繋がっていない）
    // SAFETY: 出力先が NULL のときは大きさだけが返る。
    if unsafe { (api.info)(0, 0, null_mut()) } == 0 {
        let interface = unsafe { (api.info)(WTI_INTERFACE, IFC_WINTABID, null_mut()) };
        return Err(if interface > 0 {
            Unavailable::NoTablet
        } else {
            Unavailable::NoDriver
        });
    }
    let devices = api.info_u32(WTI_INTERFACE, IFC_NDEVICES);
    if devices == Some(0) {
        return Err(Unavailable::NoTablet);
    }
    let default: LogContext = api
        .info_value(WTI_DEFSYSCTX, 0)
        .ok_or(Unavailable::NoDriver)?;
    let mut lc = LogContext::ours(&default);
    // 無効で開く（有効にするのは、ウィンドウが前に来たとき）
    // SAFETY: 名前と文脈を入れた構造体を渡す。
    let hctx = unsafe { (api.open)(hwnd, &mut lc, 0) };
    if hctx == 0 {
        return Err(Unavailable::NoContext);
    }
    // 待ち行列を大きくする（受けられない大きさは、小さい方へ。全部だめなら元に戻し、それもだめなら文脈ごと諦める）
    // SAFETY: 開いた文脈のハンドル。
    let original = unsafe { (api.queue_size_get)(hctx) };
    let mut queued = QUEUE_SIZES
        .iter()
        .copied()
        .find(|&n| unsafe { (api.queue_size_set)(hctx, n) } != 0);
    if queued.is_none() && original > 0 && unsafe { (api.queue_size_set)(hctx, original) } != 0 {
        queued = Some(original);
    }
    if queued.is_none() {
        unsafe { (api.close)(hctx) };
        return Err(Unavailable::NoContext);
    }
    // ドライバーが直した値を読み直す（パケットの並び・出力の範囲・画面の範囲）
    let mut got = lc;
    // SAFETY: 文脈の構造体の置き場を渡す。
    if unsafe { (api.get)(hctx, &mut got) } == 0 {
        got = lc;
    }
    // パケットの並びがこちらの読める形で、位置を含み、絶対の値（相対のボタン・筆圧ではない）でなければ、この文脈は読めない
    let layout = PacketLayout::new(got.pkt_data).filter(|l| l.has_position());
    let Some(layout) = layout.filter(|_| got.pkt_mode == 0) else {
        if log_enabled() {
            log_line(&format!(
                "context unusable pkt_data=0x{:x} pkt_mode=0x{:x}",
                got.pkt_data, got.pkt_mode
            ));
            log_flush();
        }
        unsafe { (api.close)(hctx) };
        return Err(Unavailable::NoContext);
    };
    let size = (unsafe { (api.queue_size_get)(hctx) }).max(16) as usize;
    let device_index = match devices {
        Some(n) if got.device >= n => 0,
        _ => got.device,
    };
    let pressure = api
        .info_value::<Axis>(WTI_DEVICES + device_index, DVC_NPRESSURE)
        .filter(|a| wintab::normalize_pressure(0, a).is_some());
    let orientation = api
        .info_value::<[Axis; 3]>(WTI_DEVICES + device_index, DVC_ORIENTATION)
        .map_or(Device::NONE.orientation, |axes| {
            OrientationAxes::from_axes(&axes)
        });
    let session = Session {
        api,
        hctx,
        msg_base: if got.msg_base == 0 {
            WT_DEFBASE
        } else {
            got.msg_base
        },
        range: OutputRange::of(&got),
        device: Device {
            pressure,
            orientation,
        },
        map: screen_map(&got),
        cursors: HashMap::new(),
        touch: Touch::default(),
        layout,
        buffer: vec![0; size * PacketLayout::MAX_SIZE],
    };
    if log_enabled() {
        log_line(&format!(
            "context open hctx={hctx} devices={devices:?} queue={size} msg_base=0x{:x} device={} options=0x{:x} pkt_data=0x{:x} pkt_mode=0x{:x} \
             in_org={:?} in_ext={:?} out_org={:?} out_ext={:?} sys_mode={} sys_org={:?} sys_ext={:?} pressure={:?} orientation={:?}",
            session.msg_base,
            got.device,
            got.options,
            got.pkt_data,
            got.pkt_mode,
            got.in_org,
            got.in_ext,
            got.out_org,
            got.out_ext,
            got.sys_mode,
            got.sys_org,
            got.sys_ext,
            session.device.pressure,
            session.device.orientation,
        ));
        log_line(&format!(
            "screens desktop={:?} monitors={:?}",
            desktop_rect(),
            monitor_rects()
        ));
        log_flush();
    }
    Ok(session)
}

/// 1 回の `pump` で分かったこと（記録の 1 行に出す）。
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct PumpInfo {
    /// 取ったパケットの数。
    pub packets: usize,
    /// 触れた点があったか。
    pub contact: bool,
}

impl Session {
    /// 溜まっている分を全部取る（古い順）。
    fn read_all(&mut self) -> Vec<RawPacket> {
        let mut all = Vec::new();
        let size = self.layout.size();
        // 頼む数は、受け皿の大きさ ÷ 最大のパケットの大きさ（ドライバーが並びを変えても溢れない）
        let cap = self.buffer.len() / PacketLayout::MAX_SIZE;
        let (api, hctx) = (self.api, self.hctx);
        loop {
            let buffer = self.buffer.as_mut_ptr();
            // SAFETY: 受け皿は `cap` 個の最大のパケット分の大きさ。
            let n = guarded(|| unsafe { (api.packets_get)(hctx, cap as i32, buffer.cast()) });
            let n = (n.max(0) as usize).min(cap);
            all.extend(
                self.buffer[..n * size]
                    .chunks_exact(size)
                    .filter_map(|bytes| self.layout.read(bytes)),
            );
            if n < cap || all.len() >= MAX_BATCH {
                break;
            }
        }
        all
    }

    /// 待ち行列を捨てる（ペンが文脈から出たとき。溜まった古い点が、次に近づいたときに混ざらないように）。
    fn flush(&mut self) {
        let (api, hctx) = (self.api, self.hctx);
        // SAFETY: 受け皿が NULL のときは、点を捨てるだけ。
        guarded(|| unsafe { (api.packets_get)(hctx, 1 << 16, null_mut()) });
    }

    fn cursor_info(&mut self, cursor: u32) -> CursorInfo {
        let api = self.api;
        *self.cursors.entry(cursor).or_insert_with(|| {
            let category = WTI_CURSORS + cursor;
            guarded(|| {
                let capabilities = api.info_u32(category, CSR_CAPABILITIES);
                let packet_data = api.info_u32(category, CSR_PKTDATA);
                let info = CursorInfo::from_info(capabilities, packet_data);
                if log_enabled() {
                    log_line(&format!(
                        "cursor {cursor} name={:?} type={:?} capabilities={capabilities:?} packet_data={packet_data:?} -> {info:?}",
                        api.info_string(category, CSR_NAME),
                        api.info_u32(category, CSR_TYPE),
                    ));
                }
                info
            })
        })
    }

    /// 画面の範囲をドライバーが直した値で読み直し、写し先の候補を作り直す（画面の構成・割り当てが変わったとき）。
    fn refresh(&mut self, foreground: bool) {
        let mut lc = LogContext::zeroed();
        let (api, hctx) = (self.api, self.hctx);
        // SAFETY: 文脈の構造体の置き場を渡す。
        if guarded(|| unsafe { (api.get)(hctx, &mut lc) }) != 0 {
            self.range = OutputRange::of(&lc);
            self.map = screen_map(&lc);
        } else {
            self.map.forget();
        }
        if log_enabled() {
            log_line(&format!(
                "refresh foreground={foreground} range={:?} sys_mode={} sys_org={:?} sys_ext={:?} monitors={:?}",
                self.range,
                lc.sys_mode,
                lc.sys_org,
                lc.sys_ext,
                monitor_rects()
            ));
        }
    }

    /// 溜まっているパケットを全部取って、点にする。押しの持ち主（`owner`）が許す点だけを返す。
    fn pump(&mut self, hwnd: HWND, owner: &Mutex<PressOwner>) -> (PumpInfo, Vec<PenSample>) {
        let packets = self.read_all();
        if packets.is_empty() {
            return (PumpInfo::default(), Vec::new());
        }
        let cursor = cursor_position();
        let positions = batch_positions(
            &packets,
            &self.range,
            &mut self.map,
            cursor,
            self.touch.is_touching(),
        );
        let source = map_name(self.map.current().map(|(k, _)| k));
        // SAFETY: 引数無し。
        let received = unsafe { GetTickCount() };
        let log = log_enabled();
        let mut info = PumpInfo {
            packets: packets.len(),
            contact: false,
        };
        // 点にする（カーソルの種類の問い合わせなど、ドライバーを呼ぶ所は、押しの持ち主のロックを取る前に済ませる）
        let mut built = Vec::with_capacity(packets.len());
        for (packet, screen) in packets.iter().zip(&positions) {
            if log {
                log_line(&wintab::log_packet(packet));
            }
            let Some(pos) = screen.and_then(|p| super::win_ink::screen_to_client(hwnd, p)) else {
                if log {
                    log_line("smp skipped (no position)");
                }
                continue;
            };
            let cursor_info = self.cursor_info(packet.cursor);
            let sample = sample_from_packet(packet, pos, &self.device, &cursor_info, received);
            info.contact |= sample.contact;
            built.push((sample, *screen));
        }
        // 入れるのは、押しの持ち主が WinTab のとき（持ち主がいないとき）の点だけ
        let mut out = Vec::with_capacity(built.len());
        let mut owner = owner.lock().unwrap_or_else(|e| e.into_inner());
        for (sample, screen) in built {
            if !owner.admit_tab(sample.contact) {
                if log {
                    log_line(&format!(
                        "{} dropped (owner=ink)",
                        wintab::log_sample(&sample, screen, &source)
                    ));
                }
                continue;
            }
            self.touch.note(&sample);
            if log {
                log_line(&wintab::log_sample(&sample, screen, &source));
            }
            out.push(sample);
        }
        (info, out)
    }
}

// ───────── ウィンドウごとの状態 ─────────

/// 閉じる頼みの状態（セッションを取り出して使っている間に閉じる頼みが来たとき、使い終わった所で閉じる）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Closing {
    #[default]
    No,
    /// こちらから閉じる（`WTClose` を呼ぶ）。
    Ask,
    /// ドライバーが閉じた（もう無いので、`WTClose` は呼ばない）。
    Driver,
}

#[derive(Default)]
struct State {
    /// この選びの間に、もう開こうとしたか（開けなかったとき、フレームごとに繰り返さない。ウィンドウが前に来たときなどに、また試す）。
    tried: bool,
    session: Option<Session>,
    closing: Closing,
    /// ドライバーが文脈を閉じた最後の時刻。
    driver_closed_at: Option<Instant>,
}

/// ウィンドウ 1 つの WinTab（`win_ink` の表に入る）。
///
/// ドライバーを呼んでいる間は、状態のロックを持たない。セッションを取り出して使い、終わったら戻す（`with_session`）: 呼び出しの中でウィンドウにメッセージが送られても、
/// プロシージャは同じロックを待たずに、ウィンドウの前後・文脈の閉じ・覆われを処理できる。文脈のハンドルと開いている印は、ロックの外の値（`hctx`・`open`）で読む。
pub(super) struct Tab {
    hwnd: isize,
    queue: Arc<Mutex<Vec<PenSample>>>,
    /// 補った離しのポインタの番号（`queue` と同じ錠の中で足す。`PenInput::drain_with_lost`）。
    lost: Arc<Mutex<Vec<u32>>>,
    ctx: egui::Context,
    /// 設定で WinTab を選んでいるか。
    wanted: Arc<AtomicBool>,
    notice: Arc<Mutex<WintabNotice>>,
    /// 文脈が開いている間 true（Windows Ink の点を受け口へ入れるかを、押しの持ち主で決める目印）。
    open: AtomicBool,
    /// 開いた文脈のメッセージの基準（`lcMsgBase`）。
    msg_base: AtomicU32,
    /// 開いた文脈のハンドル（開いていなければ 0）。
    hctx: AtomicIsize,
    state: Mutex<State>,
    /// 触れている Windows Ink の最後の点（WinTab へ替えるとき、離しの点を作る）。
    ink: Mutex<Touch>,
    /// 今のペンの押しの持ち主。
    owner: Mutex<PressOwner>,
}

impl Tab {
    pub(super) fn new(
        hwnd: isize,
        queue: Arc<Mutex<Vec<PenSample>>>,
        lost: Arc<Mutex<Vec<u32>>>,
        ctx: egui::Context,
        wanted: Arc<AtomicBool>,
        notice: Arc<Mutex<WintabNotice>>,
    ) -> Tab {
        Tab {
            hwnd,
            queue,
            lost,
            ctx,
            wanted,
            notice,
            open: AtomicBool::new(false),
            msg_base: AtomicU32::new(WT_DEFBASE),
            hctx: AtomicIsize::new(0),
            state: Mutex::new(State::default()),
            ink: Mutex::new(Touch::default()),
            owner: Mutex::new(PressOwner::default()),
        }
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut c_void)
    }

    /// WinTab が開いている（Windows Ink の点は、押しの持ち主が Windows Ink のものだけ入れる）。
    pub(super) fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    fn push(&self, samples: Vec<PenSample>) {
        if samples.is_empty() {
            return;
        }
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(samples);
        self.ctx.request_repaint();
    }

    /// 送った Windows Ink の点を覚える（WinTab へ替えるとき、触れたままなら離す）。
    pub(super) fn note_ink(&self, samples: &[PenSample]) {
        let mut ink = self.ink.lock().unwrap_or_else(|e| e.into_inner());
        for s in samples {
            ink.note(s);
        }
        drop(ink);
        log_ink(samples);
    }

    /// 触れている Windows Ink のペンの最後の点（触れていなければ None）。
    pub(super) fn touching(&self) -> Option<PenSample> {
        self.ink
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .touching()
    }

    /// 押しを奪う合図 `loss` が触れている Windows Ink の押しを奪ったなら、その離しを（補った離しの印つきで）受け口へ入れ、押しの持ち主（Windows Ink）を手放す
    /// （`capture::seize`）。
    pub(super) fn seize(&self, loss: Loss) -> Option<PenSample> {
        let lift = {
            let mut ink = self.ink.lock().unwrap_or_else(|e| e.into_inner());
            let mut owner = self.owner.lock().unwrap_or_else(|e| e.into_inner());
            capture::seize(&mut ink, &mut owner, loss)
        }?;
        super::push_lost(&self.queue, &self.lost, lift);
        self.ctx.request_repaint();
        Some(lift)
    }

    /// Windows Ink のペンの点のうち、押しの持ち主が許す物（`wintab::route_ink`）。押しの始まりでは、先に WinTab のパケットを取って持ち主を決める。
    pub(super) fn route_ink(&self, message: InkMessage, samples: Vec<PenSample>) -> Vec<PenSample> {
        let pumped = (message == InkMessage::Down).then(|| self.pump());
        let kept = {
            let mut owner = self.owner.lock().unwrap_or_else(|e| e.into_inner());
            wintab::route_ink(&mut owner, message, samples)
        };
        if let Some(info) = pumped {
            self.log_press("POINTERDOWN", info);
        }
        kept
    }

    /// 押しの始まりのメッセージ（マウスのボタンの押し）の前に、WinTab のパケットを先に取る。
    pub(super) fn pump_before(&self, message: &str) {
        let info = self.pump();
        self.log_press(message, info);
    }

    fn log_press(&self, message: &str, info: PumpInfo) {
        if log_enabled() {
            let owner = self
                .owner
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .current();
            log_line(&wintab::log_press(
                owner,
                message,
                info.packets,
                info.contact,
            ));
            log_flush();
        }
    }

    /// セッションを取り出して `f` を呼び、戻す。取り出している間はロックを持たない。セッションが無い（開いていない・ほかで使っている）ときは None。
    fn with_session<R>(&self, f: impl FnOnce(&mut Session) -> R) -> Option<R> {
        let mut session = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .session
            .take()?;
        let out = f(&mut session);
        self.put_back(session);
        Some(out)
    }

    /// 使い終わったセッションを戻す。使っている間に閉じる頼み（`close`・ドライバーの閉じ）が来ていれば、戻さずに閉じる。
    fn put_back(&self, session: Session) {
        let closing = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if self.is_open() && state.closing == Closing::No && state.session.is_none() {
                state.session = Some(session);
                return;
            }
            std::mem::take(&mut state.closing)
        };
        self.finish_close(session, closing != Closing::Driver);
    }

    /// 取り出したセッションを閉じる: 触れたままの点の離しを入れ、`call_close` なら `WTClose`。
    fn finish_close(&self, mut session: Session, call_close: bool) {
        self.push(session.touch.release().into_iter().collect());
        let (api, hctx) = (session.api, session.hctx);
        if call_close {
            // SAFETY: 開いた文脈のハンドル（このあと使わない）。
            guarded(|| unsafe { (api.close)(hctx) });
        }
        if log_enabled() {
            log_line(&format!("context close hctx={hctx} wtclose={call_close}"));
            log_flush();
        }
    }

    /// 札に合わせて開く・閉じる。
    pub(super) fn sync(&self) {
        if self.wanted.load(Ordering::Relaxed) {
            if self.is_open() {
                return;
            }
            let first = {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                !std::mem::replace(&mut state.tried, true)
            };
            if first {
                self.try_open();
            }
        } else {
            self.state.lock().unwrap_or_else(|e| e.into_inner()).tried = false;
            if self.is_open() {
                self.close();
            }
        }
    }

    fn try_open(&self) {
        match open_session(self.hwnd()) {
            Ok(session) => {
                let hctx = session.hctx;
                let api = session.api;
                self.msg_base.store(session.msg_base, Ordering::Relaxed);
                self.hctx.store(hctx, Ordering::Release);
                {
                    let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    state.session = Some(session);
                    state.closing = Closing::No;
                }
                self.open.store(true, Ordering::Release);
                self.owner.lock().unwrap_or_else(|e| e.into_inner()).reset();
                // 触れたままの Windows Ink のペンは離したことにする（以後の押しは、始まった側の物）
                let lift = self.ink.lock().unwrap_or_else(|e| e.into_inner()).release();
                self.push(lift.into_iter().collect());
                // このウィンドウが前にあるなら、有効にして最前面へ。そうでなければ、前に来たとき（WM_ACTIVATE）
                guarded(|| unsafe {
                    if self.is_foreground() {
                        (api.enable)(hctx, 1);
                        (api.overlap)(hctx, 1);
                    }
                });
            }
            Err(why) => {
                self.notice
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .report(why);
                self.ctx.request_repaint();
                if log_enabled() {
                    log_line(&format!("context unavailable: {why:?}"));
                    log_flush();
                }
            }
        }
    }

    fn is_foreground(&self) -> bool {
        // SAFETY: 引数無し。
        unsafe { GetForegroundWindow() == self.hwnd() }
    }

    /// 文脈を閉じる。触れたままの点は離し、押しの持ち主は手放す。
    pub(super) fn close(&self) {
        let was_open = self.is_open();
        let taken = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let session = state.session.take();
            if session.is_none() && was_open {
                // セッションを使っている最中（入れ子）。使い終わった所で閉じる
                state.closing = Closing::Ask;
            }
            session
        };
        self.open.store(false, Ordering::Release);
        self.hctx.store(0, Ordering::Release);
        self.owner.lock().unwrap_or_else(|e| e.into_inner()).reset();
        if let Some(session) = taken {
            self.finish_close(session, true);
        }
    }

    /// ドライバーが文脈を閉じた（ドライバーの再起動・タブレットの取り外しなど）。こちらも閉じた状態にして、触れたままの点は離す。
    /// 次の `sync` で開き直す（続けて閉じられたときは、次にウィンドウが前に来るまで待つ）。
    fn on_driver_close(&self) {
        let now = Instant::now();
        let taken = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let session = state.session.take();
            if session.is_none() {
                state.closing = Closing::Driver;
            }
            let quick = state
                .driver_closed_at
                .is_some_and(|t| now.duration_since(t) < REOPEN_AFTER);
            state.driver_closed_at = Some(now);
            state.tried = quick;
            session
        };
        self.open.store(false, Ordering::Release);
        self.hctx.store(0, Ordering::Release);
        self.owner.lock().unwrap_or_else(|e| e.into_inner()).reset();
        log_message("context closed by the driver", WT_CTXCLOSE, 0);
        if let Some(session) = taken {
            self.finish_close(session, false);
        }
        self.ctx.request_repaint();
    }

    /// ウィンドウが前に来た（有効にして最前面へ）・後ろへ行った（触れたままの点を離して、無効にする）。
    pub(super) fn on_activate(&self, active: bool) {
        if !self.is_open() {
            // 開けなかった選びは、ウィンドウが前に来たときにもう一度試す
            if active && self.wanted.load(Ordering::Relaxed) {
                self.state.lock().unwrap_or_else(|e| e.into_inner()).tried = false;
                self.sync();
            }
            return;
        }
        let Some((api, hctx)) = self.api_and_context() else {
            return;
        };
        if active {
            guarded(|| unsafe {
                (api.enable)(hctx, 1);
                (api.overlap)(hctx, 1);
            });
            self.refresh();
        } else {
            self.release_touch();
            guarded(|| unsafe { (api.enable)(hctx, 0) });
        }
    }

    /// 画面の構成が変わった。
    pub(super) fn on_display_change(&self) {
        if self.is_open() {
            self.refresh();
        }
    }

    fn api_and_context(&self) -> Option<(&'static Api, isize)> {
        let hctx = self.hctx.load(Ordering::Acquire);
        if hctx == 0 {
            return None;
        }
        Some((api().ok()?, hctx))
    }

    fn refresh(&self) {
        let foreground = self.is_foreground();
        self.with_session(|session| session.refresh(foreground));
    }

    /// 触れているペンの離しの点を受け口へ。押しの持ち主が WinTab なら手放す（離しが来ないまま止まる所で呼ぶ）。
    fn release_touch(&self) {
        let lift = self
            .with_session(|session| session.touch.release())
            .flatten();
        {
            let mut owner = self.owner.lock().unwrap_or_else(|e| e.into_inner());
            if owner.current() == Some(Owner::WinTab) {
                owner.reset();
            }
        }
        self.push(lift.into_iter().collect());
    }

    /// ウィンドウへ来たメッセージが、開いている文脈の WinTab のメッセージなら、`lcMsgBase` からの差。
    pub(super) fn wt_offset(&self, msg: u32) -> Option<u32> {
        if !self.is_open() {
            return None;
        }
        wintab::message_offset(msg, self.msg_base.load(Ordering::Relaxed))
    }

    /// WinTab のメッセージ。`wparam`・`lparam` はメッセージのもの。
    pub(super) fn on_message(&self, offset: u32, wparam: usize, lparam: isize) {
        let Some((_, hctx)) = self.api_and_context() else {
            return;
        };
        let mine = wparam as isize == hctx;
        match offset {
            WT_PACKET if lparam == hctx => {
                self.pump();
            }
            WT_PROXIMITY if mine => {
                let entering = (lparam & 0xFFFF) != 0;
                if log_enabled() {
                    log_line(&format!(
                        "proximity entering={entering} lparam=0x{lparam:x}"
                    ));
                    log_flush();
                }
                if entering {
                    self.refresh();
                } else {
                    // 文脈から出た: 残りを取り、溜まった点を捨て、触れたままなら離す
                    self.pump();
                    self.with_session(Session::flush);
                    self.release_touch();
                }
            }
            // ドライバーが文脈を閉じた
            WT_CTXCLOSE if mine => self.on_driver_close(),
            // 最前面でなくなった（別のウィンドウ・別のアプリの文脈が上になった・無効にされた）: パケットが来なくなるので、触れたままなら離す
            WT_CTXOVERLAP if mine => {
                log_message("context overlap", offset, lparam);
                if !wintab::overlap_on_top(lparam) {
                    self.release_touch();
                }
            }
            WT_INFOCHANGE => log_message("info change", offset, lparam),
            WT_CTXOPEN | WT_CTXUPDATE => log_message("context message", offset, lparam),
            _ => {}
        }
    }

    /// 溜まっているパケットを全部取って、点にして受け口へ入れる（押しの持ち主が許す点だけ）。
    pub(super) fn pump(&self) -> PumpInfo {
        let hwnd = self.hwnd();
        let Some((info, out)) = self.with_session(|session| session.pump(hwnd, &self.owner)) else {
            return PumpInfo::default();
        };
        log_flush();
        self.push(out);
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Wintab32.dll` の読み込みは、ドライバーの有無にかかわらず落ちずに、どれかの結果を返し、結果は変わらない（無い機械では NoLibrary か NoDriver）。
    #[test]
    fn loading_the_driver_library_never_panics_and_gives_the_same_answer_every_time() {
        let first = api().map(|a| a as *const Api);
        let second = api().map(|a| a as *const Api);
        assert_eq!(first, second);
        if let Err(why) = first {
            assert!(matches!(
                why,
                Unavailable::NoLibrary | Unavailable::NoDriver
            ));
        }
    }
}
