//! macOS のタブレット（Wacom・XP-Pen などのドライバー）の点を、アプリの NSEvent のローカルの監視で読む。値の直しは `tablet`（OS に依らない純粋な部分）。ここは
//! Objective-C の呼び出しだけの薄い殻。
//!
//! 監視は `NSEvent::addLocalMonitorForEventsMatchingMask:handler:` で、アプリへ届くイベントを winit より先に見る（Windows Ink がウィンドウのプロシージャで
//! WM_POINTER を先に受けるのと同じ位置）。読むだけで、イベントはそのまま返すので、winit とアプリがいつもどおり受ける。winit を直す・ビューを差し替える形にしなかったのは、
//! winit のビューの中身（クラス）に触れず、winit の版を上げても壊れないようにするため。
//!
//! 監視は 1 つだけ（最初にウィンドウを繋いだとき）。繋いだウィンドウ（メインウィンドウと別ウィンドウ）の NSWindow を弱い参照で持ち、そのウィンドウへ届いたイベントだけを
//! ウィンドウごとの受け口に詰める。ウィンドウが閉じれば参照は切れ、表から消える。ペンの近づいた・離れたはウィンドウに結ばれない（ウィンドウを持たないことがある）ので、
//! どのウィンドウのイベントでも状態を更新する。
//!
//! 監視は、ウィンドウの枠のドラッグなど入れ子の追跡の間のイベントを見られず、その間の左ボタンの離しも届かないことがある（Apple の文書）。左ボタンを押した記憶は、移動と
//! OS の今のボタンの状態で消す（`tablet`）。
//!
//! 全部、画面のスレッド（macOS のメインスレッド）で動く。表は thread_local。パニックは `crash::handled` で受け止め（落ちた記録にしない）、イベントは必ず返す。

use std::cell::RefCell;
use std::ffi::c_void;
use std::panic::AssertUnwindSafe;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSEvent, NSEventMask, NSPointingDeviceType, NSView, NSWindow};

use super::tablet::{self, Input, Point, TabletState};
use super::PenSample;

/// 監視が見るイベント: 左・右・その他のボタンの押し・ドラッグ・離しと移動、`tabletPoint`、`tabletProximity`。
fn mask() -> NSEventMask {
    NSEventMask::LeftMouseDown
        | NSEventMask::LeftMouseUp
        | NSEventMask::LeftMouseDragged
        | NSEventMask::RightMouseDown
        | NSEventMask::RightMouseUp
        | NSEventMask::RightMouseDragged
        | NSEventMask::OtherMouseDown
        | NSEventMask::OtherMouseUp
        | NSEventMask::OtherMouseDragged
        | NSEventMask::MouseMoved
        | NSEventMask::TabletPoint
        | NSEventMask::TabletProximity
}

struct Entry {
    id: usize,
    window: Weak<NSWindow>,
    queue: Arc<Mutex<Vec<PenSample>>>,
    ctx: egui::Context,
    enabled: Arc<AtomicBool>,
}

impl Entry {
    fn push(&self, sample: PenSample) {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(sample);
        self.ctx.request_repaint();
    }

    fn is(&self, window: &NSWindow) -> bool {
        self.window
            .load()
            .is_some_and(|w| std::ptr::eq(Retained::as_ptr(&w), window))
    }
}

#[derive(Default)]
struct Registry {
    entries: Vec<Entry>,
    state: TabletState,
    /// 入れた監視（アプリが終わるまで外さない）。
    monitor: Option<Retained<AnyObject>>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

impl Registry {
    /// 閉じたウィンドウの行を消す。
    fn prune(&mut self) {
        self.entries.retain(|e| e.window.load().is_some());
    }

    fn push_to(&self, owner: usize, sample: PenSample) {
        if let Some(entry) = self.entries.iter().find(|e| e.id == owner) {
            entry.push(sample);
        }
    }

    fn handle(&mut self, event: &NSEvent) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let event_type = event.r#type().0 as u64;
        let Some(input) = tablet::classify(event_type, || event.subtype().0) else {
            return;
        };
        match input {
            Input::Proximity => {
                let entering = event.isEnteringProximity();
                let eraser = entering && event.pointingDeviceType() == NSPointingDeviceType::Eraser;
                if let Some((owner, sample)) =
                    self.state
                        .proximity(event.deviceID() as u64, entering, eraser)
                {
                    self.push_to(owner, sample);
                }
            }
            Input::Point(kind) => {
                let Some(window) = event.window(mtm) else {
                    return;
                };
                self.prune();
                let Some(entry) = self.entries.iter().find(|e| e.is(&window)) else {
                    return;
                };
                if !entry.enabled.load(Ordering::Relaxed) {
                    return;
                }
                let Some(view) = window.contentView() else {
                    return;
                };
                // ビューの中の位置（点）。数でなければ、マウスの今の位置（画面の座標）から取り直す。それでも数でなければ、その点は捨てる（`TabletState::point`）。
                // 原点は左下にそろえる（反転したビューは、上からの位置を下からにする）
                let mut local = view.convertPoint_fromView(event.locationInWindow(), None);
                if !(local.x.is_finite() && local.y.is_finite()) {
                    local = view.convertPoint_fromView(
                        window.convertPointFromScreen(NSEvent::mouseLocation()),
                        None,
                    );
                }
                let height = view.bounds().size.height;
                let y = if view.isFlipped() {
                    height - local.y
                } else {
                    local.y
                };
                let tilt = event.tilt();
                let point = Point {
                    kind,
                    location: [local.x, y],
                    height,
                    scale: window.backingScaleFactor(),
                    pressure: event.pressure(),
                    tilt: [tilt.x as f32, tilt.y as f32],
                    rotation: event.rotation(),
                    buttons: event.buttonMask().0 as u64,
                    left_pressed: NSEvent::pressedMouseButtons() & 1 != 0,
                    device: event.deviceID() as u64,
                    timestamp: event.timestamp(),
                };
                let owner = entry.id;
                if let Some(sample) = self.state.point(owner, &point) {
                    entry.push(sample);
                }
            }
        }
    }

    /// 監視を入れる（1 回だけ）。
    fn install(&mut self) -> bool {
        if self.monitor.is_some() {
            return true;
        }
        let block = RcBlock::new(monitor);
        // SAFETY: ブロックの戻りは、受けたイベントのポインタをそのまま返す（有効）。画面のスレッドで呼ぶ（`register` が確かめた）。
        let monitor =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask(), &block) };
        self.monitor = monitor;
        self.monitor.is_some()
    }
}

/// 監視のブロック。イベントは読むだけで、そのまま返す。途中で panic しても、イベントは返す（Objective-C の側へ巻き戻さない。受け止めた panic は落ちた記録にしない）。
fn monitor(event: NonNull<NSEvent>) -> *mut NSEvent {
    let _ = crate::crash::handled(AssertUnwindSafe(|| {
        // SAFETY: 監視に渡されるイベントは、呼び出しの間生きている。
        let event = unsafe { event.as_ref() };
        // 表が（スレッドの終わりで）もう無いとき、呼び出しの中でまた監視が呼ばれたとき（無いはず）は、そのイベントを読まない
        let _ = REGISTRY.try_with(|registry| {
            if let Ok(mut registry) = registry.try_borrow_mut() {
                registry.handle(event);
            }
        });
    }));
    event.as_ptr()
}

fn register(
    window: &NSWindow,
    queue: Arc<Mutex<Vec<PenSample>>>,
    ctx: egui::Context,
    enabled: Arc<AtomicBool>,
) -> bool {
    let hooked = REGISTRY.try_with(|registry| {
        let Ok(mut registry) = registry.try_borrow_mut() else {
            return false;
        };
        registry.prune();
        if registry.entries.iter().any(|e| e.is(window)) {
            return true;
        }
        if !registry.install() {
            return false;
        }
        registry.entries.push(Entry {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            window: Weak::new(window),
            queue,
            ctx,
            enabled,
        });
        true
    });
    hooked.unwrap_or(false)
}

/// メインウィンドウのビュー（`RawWindowHandle::AppKit` の `ns_view`）のウィンドウを繋ぐ。
pub(super) fn hook_view(
    ns_view: NonNull<c_void>,
    queue: Arc<Mutex<Vec<PenSample>>>,
    ctx: egui::Context,
    enabled: Arc<AtomicBool>,
) -> bool {
    if MainThreadMarker::new().is_none() {
        return false;
    }
    // SAFETY: eframe が渡したウィンドウのビュー。画面のスレッドで、ウィンドウが生きている間だけ読む。
    let view: &NSView = unsafe { ns_view.cast::<NSView>().as_ref() };
    let Some(window) = view.window() else {
        return false;
    };
    register(&window, queue, ctx, enabled)
}

/// アプリの見えているウィンドウのうち、題名が `title`（そのビューポートに今付いている題名）の、まだ繋いでいない 1 つを繋ぐ（別ウィンドウ。複数あれば繋がない）。
/// ビューポートにフォーカスがある（`focused`）ときは、キーウィンドウが題名も合って繋いでいなければ、それを繋ぐ（同じ題名が重なっていても決まる）。
pub(super) fn hook_titled(
    title: &str,
    focused: bool,
    queue: Arc<Mutex<Vec<PenSample>>>,
    ctx: egui::Context,
    enabled: Arc<AtomicBool>,
) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    let found = REGISTRY
        .try_with(|registry| {
            let Ok(mut registry) = registry.try_borrow_mut() else {
                return None;
            };
            registry.prune();
            let fits = |w: &NSWindow| {
                w.isVisible()
                    && w.title().to_string() == title
                    && !registry.entries.iter().any(|e| e.is(w))
            };
            if focused {
                if let Some(key) = app.keyWindow().filter(|w| fits(w)) {
                    return Some(key);
                }
            }
            let windows = app.windows();
            let mut same = windows.iter().filter(|w| fits(w));
            match (same.next(), same.next()) {
                (Some(window), None) => Some(window),
                _ => None,
            }
        })
        .ok()
        .flatten();
    match found {
        Some(window) => register(&window, queue, ctx, enabled),
        None => false,
    }
}

/// 触れている全部のペンを離したことにする（機能を切ったとき）。
pub(super) fn release_touching() {
    let _ = REGISTRY.try_with(|registry| {
        if let Ok(mut registry) = registry.try_borrow_mut() {
            for (owner, sample) in registry.state.release_all() {
                registry.push_to(owner, sample);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pen::tablet::ns;

    /// 純粋な部分が持つ NSEvent の数値が、objc2-app-kit の定数と同じこと。
    #[test]
    fn the_event_numbers_match_appkit() {
        use objc2_app_kit::{NSEventSubtype, NSEventType};
        assert_eq!(ns::LEFT_MOUSE_DOWN, NSEventType::LeftMouseDown.0 as u64);
        assert_eq!(ns::LEFT_MOUSE_UP, NSEventType::LeftMouseUp.0 as u64);
        assert_eq!(ns::RIGHT_MOUSE_DOWN, NSEventType::RightMouseDown.0 as u64);
        assert_eq!(ns::RIGHT_MOUSE_UP, NSEventType::RightMouseUp.0 as u64);
        assert_eq!(ns::MOUSE_MOVED, NSEventType::MouseMoved.0 as u64);
        assert_eq!(
            ns::LEFT_MOUSE_DRAGGED,
            NSEventType::LeftMouseDragged.0 as u64
        );
        assert_eq!(
            ns::RIGHT_MOUSE_DRAGGED,
            NSEventType::RightMouseDragged.0 as u64
        );
        assert_eq!(ns::TABLET_POINT, NSEventType::TabletPoint.0 as u64);
        assert_eq!(ns::TABLET_PROXIMITY, NSEventType::TabletProximity.0 as u64);
        assert_eq!(ns::OTHER_MOUSE_DOWN, NSEventType::OtherMouseDown.0 as u64);
        assert_eq!(ns::OTHER_MOUSE_UP, NSEventType::OtherMouseUp.0 as u64);
        assert_eq!(
            ns::OTHER_MOUSE_DRAGGED,
            NSEventType::OtherMouseDragged.0 as u64
        );
        assert_eq!(ns::SUBTYPE_TABLET_POINT, NSEventSubtype::TabletPoint.0);
        assert_eq!(
            ns::SUBTYPE_TABLET_PROXIMITY,
            NSEventSubtype::TabletProximity.0
        );
    }

    /// ペン先・サイドボタンのビットが同じこと。
    #[test]
    fn the_button_mask_bits_match_appkit() {
        use objc2_app_kit::NSEventButtonMask;
        assert_eq!(tablet::MASK_TIP, NSEventButtonMask::PenTip.0 as u64);
        assert_eq!(
            tablet::MASK_LOWER_SIDE,
            NSEventButtonMask::PenLowerSide.0 as u64
        );
        assert_eq!(
            tablet::MASK_UPPER_SIDE,
            NSEventButtonMask::PenUpperSide.0 as u64
        );
    }
}
