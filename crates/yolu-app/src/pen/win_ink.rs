//! Windows Ink（WM_POINTER）。ウィンドウのプロシージャを差し替え（GWLP_WNDPROC）、ペンの WM_POINTERDOWN・UPDATE・UP で
//! GetPointerPenInfoHistory を読んでから、元のプロシージャ（winit）へ渡す。位置は himetric から画面の画素へ直してから
//! クライアント領域へ（winit と同じ求め方で、整数の画素より細かい）。履歴は新しい順に来るので、古い順に並べ直して詰める。
//!
//! 同じプロシージャが WinTab（`win_tab`）のメッセージ（`WT_PACKET` など）・ウィンドウの前後（`WM_ACTIVATE`）・画面の変更（`WM_DISPLAYCHANGE`）も受けて、
//! ウィンドウごとの `Tab` へ渡す。WinTab が開いている間は、Windows Ink のペンの点を受け口に入れない（winit には、どちらの場合もそのまま渡す）。
//!
//! ペンや指の押しを OS が奪う合図（`WM_POINTERCAPTURECHANGED`・ほかのウィンドウへの `WM_CAPTURECHANGED`・移動の輪の終わりの `WM_EXITSIZEMOVE`）では、
//! 触れていた Windows Ink の押しを離したことにする（補った離しの点を、印つきで受け口へ入れ、押しの持ち主を手放し、egui のポインタの離しとして `WM_LBUTTONUP` を post する。
//! 奪った番号の触れた点は、次の `WM_POINTERDOWN` まで入れない。`capture`）。
//! 帯をペンや指で引いたときは、ウィンドウの位置をポインタの画面の位置に合わせて動かす（`begin_move`。OS の移動の輪に渡さない）。マウスの引きで OS の移動の輪に
//! 渡した `StartDrag` は、輪が始まらないまま 1 秒たったら `WM_EXITSIZEMOVE` を自分のウィンドウへ送って、winit の印を戻す（`DragWatch`）。
//! 受けた合図・補った離し・ウィンドウの移動・見張りの働きは、`YOLU_PEN_LOG` に 1 行ずつ出す。

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::Pointer::{
    GetPointerDeviceRects, GetPointerInfo, GetPointerPenInfoHistory, GetPointerType,
    POINTER_FLAG_INCONTACT, POINTER_INFO, POINTER_PEN_INFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, DefWindowProcW, GetWindowRect, IsZoomed, PostMessageW, SendMessageW,
    SetWindowLongPtrW, SetWindowPos, GWLP_WNDPROC, PEN_FLAG_BARREL, PEN_FLAG_ERASER,
    PEN_FLAG_INVERTED, PEN_MASK_PRESSURE, PEN_MASK_ROTATION, PEN_MASK_TILT_X, PEN_MASK_TILT_Y,
    POINTER_INPUT_TYPE, PT_PEN, PT_TOUCH, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, WM_ACTIVATE,
    WM_CAPTURECHANGED, WM_DISPLAYCHANGE, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_NCDESTROY, WM_POINTERCAPTURECHANGED, WM_POINTERDOWN,
    WM_POINTERUP, WM_POINTERUPDATE, WM_RBUTTONDOWN, WNDPROC,
};

use super::capture::{self, DragWatch, Follow, LastPen, Loss, PenWindowMove, TakenIds};
use super::win_tab::{log_event, Tab};
use super::wintab::{split_position, InkMessage};
use super::{PenSample, WindowMover, WintabNotice};
use crate::engine::Tilt;

/// ウィンドウごとの、ポインタ（ペン・指）の直近の点・ウィンドウを動かす掴み・奪って補った番号・`StartDrag` の見張り（ウィンドウのプロシージャと、
/// 帯を引いた画面のフレームの両方から触る）。
#[derive(Default)]
struct Carry {
    /// 触れている（または最後に触れていた）ポインタの直近の点。Windows Ink の生の点で、押しの持ち主（WinTab か）に依らない。
    last: Option<LastPen>,
    moving: PenWindowMove,
    taken: TakenIds,
    watch: DragWatch,
    /// 見張りが送った `WM_EXITSIZEMOVE` を待っている（本物の輪の終わりではないので、押しを奪う合図に数えない）。
    synthetic_exit: bool,
}

/// Windows Ink の点 1 つと、その画面の位置（仮想スクリーンの物理の画素）。
type Read = (PenSample, [f64; 2]);

/// ポインタ（ペン・指）の 1 点: 番号・触れているか・画面の位置（仮想スクリーンの物理の画素）。
type Point = (u32, bool, [f64; 2]);

struct Hooked {
    previous: isize,
    queue: Arc<Mutex<Vec<PenSample>>>,
    ctx: egui::Context,
    tab: Arc<Tab>,
    carry: Arc<Mutex<Carry>>,
}

fn table() -> &'static Mutex<HashMap<isize, Hooked>> {
    static TABLE: OnceLock<Mutex<HashMap<isize, Hooked>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// ウィンドウのプロシージャを差し替える。同じウィンドウに二度は差し替えない。差し替えは表のロックを持たずに行う（ウィンドウのプロシージャが
/// 同じ表を引くので、もし差し替えの途中にメッセージが来ても止まらないように）。
pub(super) fn hook(
    hwnd: isize,
    queue: Arc<Mutex<Vec<PenSample>>>,
    lost: Arc<Mutex<Vec<u32>>>,
    ctx: egui::Context,
    wintab: Arc<AtomicBool>,
    notice: Arc<Mutex<WintabNotice>>,
) -> bool {
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
    let tab = Arc::new(Tab::new(
        hwnd,
        queue.clone(),
        lost,
        ctx.clone(),
        wintab,
        notice,
    ));
    table().lock().unwrap_or_else(|e| e.into_inner()).insert(
        hwnd,
        Hooked {
            previous,
            queue,
            ctx,
            tab,
            carry: Arc::new(Mutex::new(Carry::default())),
        },
    );
    true
}

/// ウィンドウの `Tab` を、表から取る（表のロックは短く）。
fn tab_of(hwnd: isize) -> Option<Arc<Tab>> {
    table()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&hwnd)
        .map(|h| h.tab.clone())
}

/// ウィンドウの `Carry` を、表から取る（表のロックは短く）。
fn carry_of(hwnd: isize) -> Option<Arc<Mutex<Carry>>> {
    table()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&hwnd)
        .map(|h| h.carry.clone())
}

/// 設定の札に合わせて、ウィンドウの WinTab を開く・閉じる。
pub(super) fn sync(hwnd: isize) {
    if let Some(tab) = tab_of(hwnd) {
        tab.sync();
    }
}

/// ウィンドウの WinTab が開いているか。
pub(super) fn wintab_open(hwnd: isize) -> bool {
    tab_of(hwnd).is_some_and(|tab| tab.is_open())
}

/// 画面の位置（f64。物理の画素）をクライアント領域の位置にする。整数より細かい端数を残す（負の位置でも、端数は 0 以上）。
pub(super) fn screen_to_client(hwnd: HWND, screen: [f64; 2]) -> Option<[f32; 2]> {
    let (x, y) = (split_position(screen[0])?, split_position(screen[1])?);
    let mut at = POINT { x: x.0, y: y.0 };
    // SAFETY: 出力先は自分の変数。
    if !unsafe { ScreenToClient(hwnd, &mut at) }.as_bool() {
        return None;
    }
    Some([at.x as f32 + x.1, at.y as f32 + y.1])
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let key = hwnd.0 as isize;
    let (previous, queue, ctx, tab, carry) = {
        let table = table().lock().unwrap_or_else(|e| e.into_inner());
        match table.get(&key) {
            Some(h) => (
                h.previous,
                h.queue.clone(),
                h.ctx.clone(),
                h.tab.clone(),
                h.carry.clone(),
            ),
            // 表に入る前（差し替えた直後）のメッセージは既定の処理へ（捨てない）
            None => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    };
    match msg {
        WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
            let id = (wparam.0 & 0xFFFF) as u32;
            let down = msg == WM_POINTERDOWN;
            let reads = unsafe { read_pen(hwnd, id) };
            if reads.is_empty() {
                // 指（PT_TOUCH）: 受け口には入れない。帯を引いてウィンドウを動かす掴みのために、位置だけ追う
                if let Some(point) = unsafe { read_finger(id) } {
                    follow_pointer(hwnd, &carry, &[point], down);
                }
            } else {
                // 奪って離しを補った番号の触れた点は、次の触れた最初のメッセージまで入れない（補った離しのあとで押しが新しく始まり、ストロークが割れないように）
                let reads: Vec<Read> = {
                    let mut carry = carry.lock().unwrap_or_else(|e| e.into_inner());
                    reads
                        .into_iter()
                        .filter(|(s, _)| carry.taken.admit(down, s))
                        .collect()
                };
                if !reads.is_empty() {
                    let points: Vec<Point> = reads
                        .iter()
                        .map(|(s, screen)| (s.pointer_id, s.contact, *screen))
                        .collect();
                    follow_pointer(hwnd, &carry, &points, down);
                    let samples: Vec<PenSample> = reads.iter().map(|(s, _)| *s).collect();
                    // WinTab が開いている間は、ペンの 1 本の押しごとに、Windows Ink と WinTab のどちらの点を入れるか決める（`wintab::PressOwner`）。
                    // 押しの始まりでは、先に WinTab のパケットを取ってから決める（取った点は、この Windows Ink の点より先に受け口へ入る）
                    let samples = if tab.is_open() {
                        let message = match msg {
                            WM_POINTERDOWN => InkMessage::Down,
                            WM_POINTERUP => InkMessage::Up,
                            _ => InkMessage::Update,
                        };
                        tab.route_ink(message, samples)
                    } else {
                        samples
                    };
                    if !samples.is_empty() {
                        tab.note_ink(&samples);
                        queue
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .extend(samples);
                        ctx.request_repaint();
                    }
                }
            }
        }
        // ペンがマウスとして押すときも、WinTab のパケットを先に取る
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            if tab.is_open() {
                tab.pump_before(match msg {
                    WM_LBUTTONDOWN => "LBUTTONDOWN",
                    WM_RBUTTONDOWN => "RBUTTONDOWN",
                    _ => "MBUTTONDOWN",
                });
            }
        }
        // OS がペンの押しを奪う合図: 触れていた押しを離したことにする（離しの点が届かないまま、受け口・押しの持ち主・ビュー・egui の押しが残らないように）
        WM_POINTERCAPTURECHANGED => {
            let id = (wparam.0 & 0xFFFF) as u32;
            let kept = lparam.0 == hwnd.0 as isize;
            seized(hwnd, &tab, &carry, Loss::Pointer { id, kept });
        }
        WM_CAPTURECHANGED => {
            let to = lparam.0;
            let elsewhere = to != 0 && to != hwnd.0 as isize;
            seized(hwnd, &tab, &carry, Loss::Mouse { elsewhere });
        }
        WM_ENTERSIZEMOVE => {
            carry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .watch
                .entered();
            log_event(|| "size-move enter".to_string());
        }
        WM_EXITSIZEMOVE => {
            let synthetic = {
                let mut carry = carry.lock().unwrap_or_else(|e| e.into_inner());
                carry.watch.exited();
                std::mem::take(&mut carry.synthetic_exit)
            };
            if synthetic {
                // 見張りが送った物: winit が「動かしている最中」の印を戻すためで、ペンや指の押しを奪う合図ではない
                log_event(|| "size-move exit (sent by the watch)".to_string());
            } else {
                seized(hwnd, &tab, &carry, Loss::ExitMove);
            }
        }
        WM_ACTIVATE => tab.on_activate((wparam.0 & 0xFFFF) != 0),
        WM_DISPLAYCHANGE => tab.on_display_change(),
        WM_NCDESTROY => {
            carry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .moving
                .cancel();
            tab.close()
        }
        _ => {
            if let Some(offset) = tab.wt_offset(msg) {
                tab.on_message(offset, wparam.0, lparam.0);
            }
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

/// 押しを奪われた合図を受けた: ウィンドウを動かしていたら止め、触れていた Windows Ink の押しを離したことにする。
/// 離しの点は補った離しの印つきで受け口へ入れ、その番号を「奪った」と覚える（次の触れた最初のメッセージまで、その番号の触れた点を入れない）。
/// egui のポインタの押し（winit がペンや指の接触から作った左ボタン）の離しとして `WM_LBUTTONUP` を post する（winit が移動の輪の終わりに自分で post するのと同じ道。
/// ここで呼び出さず post するのは、メッセージの処理の途中で winit の処理を入れ子にしないため）。post するかは、受け口の触れている記録でなく
/// Windows Ink の生の点（`Carry::last`）で決める: egui の押しは winit の Touch から来るので、WinTab が押しの持ち主で受け口の記録が空でも、egui の押しは残る。
fn seized(hwnd: HWND, tab: &Tab, carry: &Mutex<Carry>, loss: Loss) {
    let (was_moving, egui_down) = {
        let mut carry = carry.lock().unwrap_or_else(|e| e.into_inner());
        let was_moving = carry.moving.lose(loss);
        let egui_down = match carry.last {
            Some(last) if last.contact && loss.takes_pointer(last.pointer) => {
                carry.last = Some(LastPen {
                    contact: false,
                    ..last
                });
                Some(last)
            }
            _ => None,
        };
        (was_moving, egui_down)
    };
    let touching = tab.touching();
    let lift = tab.seize(loss);
    if let Some(lift) = &lift {
        carry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .taken
            .take(lift.pointer_id);
    }
    // egui の押しを離すための位置（クライアント領域）。補った離しの位置、無ければ Windows Ink の生の点の位置
    let release_at = lift
        .map(|lift| lift.pos)
        .or_else(|| egui_down.and_then(|last| screen_to_client(hwnd, last.screen)));
    // マウスの捕まえの変更は、普通のマウスの離しのたびに来る。ペンが触れている・ウィンドウを動かしている・ほかのウィンドウが捕まえたときだけ記録する
    let worth = !matches!(loss, Loss::Mouse { elsewhere: false })
        || touching.is_some()
        || was_moving
        || egui_down.is_some();
    if worth {
        log_event(|| {
            let line = capture::log_loss(
                loss,
                touching.as_ref(),
                lift.is_some(),
                release_at.is_some(),
            );
            if was_moving {
                format!("{line} window-move=cancelled")
            } else {
                line
            }
        });
    }
    if let Some(pos) = release_at {
        let at = ((pos[1] as i32 as u16 as u32) << 16) | (pos[0] as i32 as u16 as u32);
        // SAFETY: 自分のウィンドウへ、マウスの左ボタンの離し（位置はクライアント領域の画素）を post するだけ。
        let _ = unsafe { PostMessageW(Some(hwnd), WM_LBUTTONUP, WPARAM(0), LPARAM(at as isize)) };
    }
}

/// ポインタ（ペン・指）の点を受けた: 直近の点を覚え、ウィンドウを動かしている最中なら、その画面の位置に合わせてウィンドウを動かす（離しの点で終える）。
/// `down` は触れた最初のメッセージ（`WM_POINTERDOWN`）: 新しい押しの始まりなので、前の押しの掴みが残っていても捨てる（離しが届かなかった押しで、次の接触がウィンドウを動かさないように）。
fn follow_pointer(hwnd: HWND, carry: &Mutex<Carry>, points: &[Point], down: bool) {
    let mut target = None;
    let mut ended = false;
    {
        let mut carry = carry.lock().unwrap_or_else(|e| e.into_inner());
        if down && carry.moving.cancel() {
            log_event(|| "window-move cancelled (new down)".to_string());
        }
        // 掴んでいる間だけ、ウィンドウの今の大きさを読む（別の拡大率の画面へまたぐと変わる）
        let size = if carry.moving.is_active() {
            window_rect(hwnd).map_or([0, 0], |r| [r.right - r.left, r.bottom - r.top])
        } else {
            [0, 0]
        };
        for &(pointer, contact, screen) in points {
            carry.last = Some(LastPen {
                pointer,
                contact,
                screen,
            });
            match carry.moving.follow(pointer, contact, screen, size) {
                Follow::To(to) => target = Some(to),
                Follow::Ended => ended = true,
                Follow::Idle => {}
            }
        }
    }
    if let Some([x, y]) = target {
        // SAFETY: 自分のウィンドウの位置だけを変える（大きさ・重なり・前面は変えない）。ロックは持たない（位置を変えると別のメッセージが入れ子で来る）。
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                None,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
    }
    if ended {
        log_event(|| "window-move end (pointer up)".to_string());
    }
}

/// ウィンドウの外枠の矩形（仮想スクリーンの物理の画素）。
fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    // SAFETY: 出力先は自分の変数。
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok().map(|_| rect)
}

/// 帯をペンや指で引いたときの、ウィンドウを動かす手と、`StartDrag` の見張り（`PenInput::begin_window_move` など）。
struct NativeMover {
    hwnd: isize,
}

impl WindowMover for NativeMover {
    fn begin(&self) -> bool {
        begin_move(self.hwnd)
    }

    fn start_drag_handed(&self) {
        if let Some(carry) = carry_of(self.hwnd) {
            carry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .watch
                .handed(Instant::now());
        }
        log_event(|| "os-move handed to the OS".to_string());
    }

    fn poll(&self) {
        let Some(carry) = carry_of(self.hwnd) else {
            return;
        };
        let stuck = {
            let mut carry = carry.lock().unwrap_or_else(|e| e.into_inner());
            let stuck = carry.watch.stuck(Instant::now());
            carry.synthetic_exit |= stuck;
            stuck
        };
        if stuck {
            log_event(|| {
                "os-move stuck (no WM_ENTERSIZEMOVE): sending WM_EXITSIZEMOVE".to_string()
            });
            // SAFETY: 自分のウィンドウへ、移動の輪の終わりを送る（winit が「動かしている最中」の印を戻す）。ロックは持たない。
            let _ = unsafe {
                SendMessageW(
                    HWND(self.hwnd as _),
                    WM_EXITSIZEMOVE,
                    Some(WPARAM(0)),
                    Some(LPARAM(0)),
                )
            };
        }
    }
}

/// ウィンドウ `hwnd` をペンや指で動かす手。
pub(super) fn mover(hwnd: isize) -> Arc<dyn WindowMover> {
    Arc::new(NativeMover { hwnd })
}

/// 触れているポインタ（ペン・指）の最後の点から、そのポインタでウィンドウを動かし始める（古くても、最後の位置から掴む。帯を引き始めた押しが
/// ペン・指のものだと分かっているときだけ呼ばれる）。最大化中は動かさずに true（OS の移動の輪に渡さない）。触れているポインタが無い・ウィンドウの位置が読めないなら false。
fn begin_move(hwnd: isize) -> bool {
    let Some(carry) = carry_of(hwnd) else {
        return false;
    };
    let h = HWND(hwnd as _);
    let source = carry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .last
        .and_then(|last| last.source());
    let Some((pointer, pen)) = source else {
        log_event(|| "window-move not-started (no touching pointer)".to_string());
        return false;
    };
    // SAFETY: 読むだけ。
    if unsafe { IsZoomed(h) }.as_bool() {
        log_event(|| "window-move not-started (maximized)".to_string());
        return true;
    }
    let Some(rect) = window_rect(h) else {
        return false;
    };
    let size = [rect.right - rect.left, rect.bottom - rect.top];
    carry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .moving
        .begin(pointer, pen, [rect.left, rect.top], size);
    log_event(|| {
        format!(
            "window-move begin id={pointer} pointer=({:.1},{:.1}) window=({},{}) size=({},{})",
            pen[0], pen[1], rect.left, rect.top, size[0], size[1]
        )
    });
    true
}

/// 指（`PT_TOUCH`）なら、そのメッセージの点（番号・触れているか・画面の位置）。指でなければ None。
unsafe fn read_finger(id: u32) -> Option<Point> {
    let mut kind = POINTER_INPUT_TYPE::default();
    if unsafe { GetPointerType(id, &mut kind) }.is_err() || kind != PT_TOUCH {
        return None;
    }
    let mut info = POINTER_INFO::default();
    unsafe { GetPointerInfo(id, &mut info) }.ok()?;
    Some((
        id,
        info.pointerFlags.0 & POINTER_FLAG_INCONTACT.0 != 0,
        [
            f64::from(info.ptPixelLocation.x),
            f64::from(info.ptPixelLocation.y),
        ],
    ))
}

/// ペンなら、このメッセージに溜まった履歴の点を、画面の位置つきで古い順に返す（ペンでなければ空）。
unsafe fn read_pen(hwnd: HWND, id: u32) -> Vec<Read> {
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
        let (x, y): (f64, f64) =
            if unsafe { GetPointerDeviceRects(p.sourceDevice, &mut device, &mut display) }.is_ok()
                && device.right > device.left
                && device.bottom > device.top
            {
                let rx =
                    (display.right - display.left) as f64 / (device.right - device.left) as f64;
                let ry =
                    (display.bottom - display.top) as f64 / (device.bottom - device.top) as f64;
                (
                    display.left as f64 + p.ptHimetricLocation.x as f64 * rx,
                    display.top as f64 + p.ptHimetricLocation.y as f64 * ry,
                )
            } else {
                (p.ptPixelLocation.x as f64, p.ptPixelLocation.y as f64)
            };
        let Some(pos) = screen_to_client(hwnd, [x, y]) else {
            continue;
        };
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
        let sample = PenSample {
            pos,
            pressure,
            tilt,
            rotation: (info.penMask & PEN_MASK_ROTATION != 0)
                .then_some((info.rotation % 360) as f32),
            contact: p.pointerFlags.0 & POINTER_FLAG_INCONTACT.0 != 0,
            eraser: info.penFlags & (PEN_FLAG_ERASER | PEN_FLAG_INVERTED) != 0,
            barrel: info.penFlags & PEN_FLAG_BARREL != 0,
            pointer_id: id,
            time_ms: p.dwTime,
        };
        out.push((sample, [x, y]));
    }
    out
}
