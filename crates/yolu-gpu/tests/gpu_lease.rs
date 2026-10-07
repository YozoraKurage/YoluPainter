//! GPU の貸し出し（`support/gpu_lease.rs`）の確かめ。GPU を使う試験どうしが同じプロセスの中で重ならない（lavapipe の中で装置を同時に
//! 作ると落ちることがあったため）。装置は作らないので、アダプターが無くても走る。
#[path = "support/gpu_lease.rs"]
mod gpu_lease;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// この確かめどうしが、同じ貸し出しを混ぜないように 1 つずつ走らせる。
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

#[test]
fn two_threads_never_hold_the_gpu_at_once() {
    let _serial = serial();
    static HOLDING: AtomicUsize = AtomicUsize::new(0);
    static MOST: AtomicUsize = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                gpu_lease::lease();
                let now = HOLDING.fetch_add(1, Ordering::SeqCst) + 1;
                MOST.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(15));
                HOLDING.fetch_sub(1, Ordering::SeqCst);
                // 放すのはスレッドが終わるとき
            });
        }
    });
    assert_eq!(MOST.load(Ordering::SeqCst), 1);
    assert_eq!(HOLDING.load(Ordering::SeqCst), 0);
}

#[test]
fn a_thread_that_already_holds_the_gpu_can_ask_again_for_a_second_device() {
    let _serial = serial();
    let thread = std::thread::spawn(|| {
        gpu_lease::lease();
        gpu_lease::lease();
        gpu_lease::lease();
    });
    thread
        .join()
        .expect("2 回目からは待たない（自分を待って止まらない）");
}

#[test]
fn a_panicking_test_releases_the_gpu_for_the_next_one() {
    let _serial = serial();
    let failed = std::thread::spawn(|| {
        gpu_lease::lease();
        panic!("GPU を持ったまま落ちる");
    });
    assert!(failed.join().is_err());
    // 次の試験は貸し出しを取れる（落ちた試験の貸し出しは壊れて残らない）
    std::thread::spawn(gpu_lease::lease).join().unwrap();
}
