//! GPU の装置（wgpu の Instance・Device）を作る・使う・捨てるのを、同じ試験の実行ファイル（プロセス）の中で 1 つの試験だけに絞る。
//!
//! Vulkan（lavapipe）は、別々のスレッドで装置を同時に作って使うと、プロセスごと SIGSEGV で落ちることがある（libtest は試験ごとに
//! スレッドを分けるので、GPU を使う試験が並ぶと起きる）。試験は、装置を作る前に `lease()` を呼ぶ。ほかのスレッドが持っていれば、その試験が
//! 終わる（スレッドが終わる）まで待つ。同じスレッドで何度呼んでも 1 回分（1 つの試験が装置を何個作ってもよい）。放すのはスレッドが終わるとき。
#![allow(dead_code)]
use std::cell::RefCell;
use std::sync::{Mutex, MutexGuard, PoisonError};

static GPU: Mutex<()> = Mutex::new(());

thread_local! {
    static LEASE: RefCell<Option<MutexGuard<'static, ()>>> = const { RefCell::new(None) };
}

pub fn lease() {
    LEASE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(GPU.lock().unwrap_or_else(PoisonError::into_inner));
        }
    });
}
