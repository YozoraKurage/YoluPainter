//! 描画試験の窓の生成・使用・破棄を、プロセス中で生き続ける同じスレッドへ集める。
use std::any::Any;
use std::cell::RefCell;
use std::sync::{mpsc, Once, OnceLock};

thread_local! {
    /// このスレッドで最後に起きた panic の場所。ワーカーの出力捕捉先は最初の呼び手に属するので、
    /// libtest の panic フック行（場所つき）は後続の試験には届かない。そのためフックで場所だけ控える。
    static LAST_PANIC_AT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// 既存のフックの前に、panic の場所を控えるだけの層を 1 度だけ重ねる。既存のフックの出力は変えない。
fn record_panic_locations() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Some(at) = info.location() {
                let at = format!("{}:{}:{}", at.file(), at.line(), at.column());
                let _ = LAST_PANIC_AT.try_with(|slot| {
                    if let Ok(mut slot) = slot.try_borrow_mut() {
                        *slot = Some(at);
                    }
                });
            }
            previous(info);
        }));
    });
}

/// ワーカーで落ちた試験の panic。呼び手の試験へ戻すときに場所も運ぶ。
pub struct Failure {
    pub payload: Box<dyn Any + Send>,
    pub location: Option<String>,
}

impl Failure {
    /// 失敗の本文と、落ちた場所（ファイル:行:列）。
    pub fn describe(&self) -> String {
        let message = self
            .payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| self.payload.downcast_ref::<&str>().copied())
            .unwrap_or("文字列以外の panic");
        match &self.location {
            Some(at) => format!("{message}（{at}）"),
            None => message.to_string(),
        }
    }
}

/// ワーカーで試験を実行し、落ちたら panic の中身と場所を返す。
pub fn run_checked(test: fn()) -> Result<(), Failure> {
    type Job = Box<dyn FnOnce() + Send>;
    static WORKER: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        record_panic_locations();
        let (sender, receiver) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("描画試験".into())
            .spawn(move || {
                for job in receiver {
                    job();
                }
            })
            .expect("描画試験スレッドを作る");
        sender
    });
    let (sender, receiver) = mpsc::sync_channel(1);
    worker
        .send(Box::new(move || {
            LAST_PANIC_AT.with(|slot| slot.borrow_mut().take());
            // 失敗を呼び出した libtest の試験へ戻し、後続の試験も実行できるようにする。
            let result = std::panic::catch_unwind(test).map_err(|payload| Failure {
                payload,
                location: LAST_PANIC_AT.with(|slot| slot.borrow_mut().take()),
            });
            let _ = sender.send(result);
        }))
        .expect("描画試験スレッドが生きている");
    receiver.recv().expect("描画試験の結果を受け取る")
}

pub fn run(test: fn()) {
    if let Err(failure) = run_checked(test) {
        // ワーカーの出力捕捉先は最初の呼び手に属するので、失敗の本文と場所は今の試験にも残す。
        eprintln!("描画試験に失敗: {}", failure.describe());
        std::panic::resume_unwind(failure.payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callers_use_the_same_thread() {
        static THREAD: OnceLock<std::thread::ThreadId> = OnceLock::new();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    let caller = std::thread::current().id();
                    run(|| {
                        let current = std::thread::current().id();
                        assert_eq!(*THREAD.get_or_init(|| current), current);
                    });
                    assert_ne!(*THREAD.get().unwrap(), caller);
                });
            }
        });
    }

    #[test]
    fn a_failure_returns_to_the_caller_and_the_next_test_runs() {
        let failure = std::panic::catch_unwind(|| run(|| panic!("試験の失敗")));
        let payload = failure.expect_err("失敗を成功として扱わない");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"試験の失敗"));
        run(|| {});
    }

    #[test]
    fn a_failure_carries_the_place_it_panicked_at() {
        let line = line!() + 1;
        let failure = run_checked(|| panic!("場所を見る")).expect_err("失敗を成功として扱わない");
        let at = failure.location.as_deref().expect("落ちた場所が残る");
        assert!(at.contains("gpu_thread.rs"), "ファイル名: {at}");
        assert!(at.contains(&format!(":{line}:")), "行 {line}: {at}");
        let described = failure.describe();
        assert!(described.contains("場所を見る") && described.contains(at), "{described}");
        // 次の試験の場所に前の失敗が混ざらない。
        assert!(run_checked(|| {}).is_ok());
        let line = line!() + 1;
        let second = run_checked(|| panic!("二度目")).expect_err("失敗");
        assert!(second.location.unwrap().contains(&format!(":{line}:")));
    }
}
