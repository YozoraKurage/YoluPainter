//! 状態の側から画面へ描き直しを頼む口（egui を知らない）。殻が `egui::Context::request_repaint` を包んで入れ、別のスレッドの仕事は
//! 結果を渡した後に `wake` を呼ぶ。

use std::fmt;
use std::sync::Arc;

/// 描き直しを頼む口。複製は同じ関数を指す。
#[derive(Clone)]
pub struct Waker(Arc<dyn Fn() + Send + Sync>);

impl Waker {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Waker {
        Waker(Arc::new(wake))
    }

    /// 描き直しを頼む（包んだ関数を 1 回呼ぶ）。
    pub fn wake(&self) {
        (self.0)();
    }
}

impl fmt::Debug for Waker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Waker")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn each_wake_calls_the_wrapped_function_once_from_any_clone_and_thread() {
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let waker = Waker::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(count.load(Ordering::SeqCst), 0, "作っただけでは呼ばない");
        let copy = waker.clone();
        waker.wake();
        copy.wake();
        std::thread::spawn(move || copy.wake()).join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 3);
        assert_eq!(format!("{waker:?}"), "Waker");
    }
}
