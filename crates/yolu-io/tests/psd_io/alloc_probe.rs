//! この束の試験が、ある処理の間にこのスレッドが持った作業メモリの最大（確保して手放していない量のピーク）を測る道具。
//! `#[global_allocator]` はプロセス全体に効くが、数えるのは `peak_live_bytes` の中のスレッドだけで、ほかの試験の結果には触れない。
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

fn grow(by: isize) {
    let _ = ON.try_with(|on| {
        if on.get() {
            LIVE.with(|live| {
                live.set(live.get() + by);
                PEAK.with(|peak| peak.set(peak.get().max(live.get())));
            })
        }
    });
}

pub struct Probe;
// SAFETY: 確保そのものは System にそのまま渡し、量を数えるだけ。
unsafe impl GlobalAlloc for Probe {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grow(layout.size() as isize);
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        grow(layout.size() as isize);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        grow(new_size as isize - layout.size() as isize);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        grow(-(layout.size() as isize));
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Probe = Probe;

/// `f` を実行し、その間にこのスレッドが持った作業メモリの最大のバイト数（始めからの増え）を返す。
pub fn peak_live_bytes<T>(f: impl FnOnce() -> T) -> (T, usize) {
    ON.with(|on| on.set(true));
    LIVE.with(|live| live.set(0));
    PEAK.with(|peak| peak.set(0));
    let value = f();
    ON.with(|on| on.set(false));
    (value, PEAK.with(|peak| peak.get()).max(0) as usize)
}

/// 探りが生きていること（確保を捉え、手放した分を引き、測りの外を数えない）の陽性対照。これが無いと、探りが壊れても「小さい」の断言がすべて素通りで通る。
#[test]
fn the_probe_sees_live_allocations_only_inside_the_measure() {
    use std::hint::black_box;
    const BIG: usize = 4 << 20;
    let (v, peak) = peak_live_bytes(|| black_box(vec![1u8; BIG]));
    assert_eq!(v.len(), BIG);
    assert!(peak >= BIG, "{peak}");
    // 確保してすぐ手放した 2 つは、同時には持たない
    let (_, peak) = peak_live_bytes(|| {
        drop(black_box(vec![1u8; BIG]));
        drop(black_box(vec![1u8; BIG]));
    });
    assert!((BIG..2 * BIG).contains(&peak), "{peak}");
    // 測りの前に持ったものは数えない
    let (_, peak) = peak_live_bytes(|| black_box(vec![1u8; 1024]));
    assert!(peak < BIG, "{peak}");
}
