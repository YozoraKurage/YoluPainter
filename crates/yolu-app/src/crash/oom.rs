//! 確保の失敗の記録用のアロケーター。実行ファイルの `#[global_allocator]` に置く（`main.rs`）。
//!
//! Rust の標準の確保（`Vec`・`Box` など）が失敗すると、`handle_alloc_error` が標準エラーへ書いて abort する。標準エラーの無い窓のアプリでは
//! 何も残らず、Windows の abort は `__fastfail` なので未処理例外のフィルターにも来ない。`std::alloc::set_alloc_error_hook` は不安定版
//! （nightly）の機能で使えないため、システムのアロケーターを包み、確保が null を返したとき（致命的かは分からないので、すぐ）
//! 記録の先へ大きさと呼び出しの番地を書く（`native::allocation_failed`）。普段の確保は、null かの確かめが 1 つ増えるだけ。
use std::alloc::{GlobalAlloc, Layout, System};

pub struct RecordingAlloc;

// SAFETY: 確保・解放・伸ばしはすべて `System` に任せ、null が返ったときだけ、確保しない記録を足す。
unsafe impl GlobalAlloc for RecordingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if ptr.is_null() {
            super::native::allocation_failed(layout.size(), layout.align());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if ptr.is_null() {
            super::native::allocation_failed(layout.size(), layout.align());
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let moved = unsafe { System.realloc(ptr, layout, new_size) };
        if moved.is_null() {
            super::native::allocation_failed(new_size, layout.align());
        }
        moved
    }
}
