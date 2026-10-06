//! 別のスレッドの仕事の共通の部品: 結果の受け口と取り消しの旗（`Worker`）。
//!
//! `Worker` にしない仕事:
//! - `update/mod.rs` の `Job`: 取り消しの旗を持たない（ダウンロードの取り消しは通信の口 `Link` が持ち、確かめとダウンロードで作りが別）。
//! - `project/save.rs` の `Job`: 保存は途中で止めない（取り消しの旗が無い）。進み具合を画面と共有の `Shared` で持つ。
//! - `library/service.rs` の `Job<R>`: 待ち行列の 1 つの要素（スレッドを仕事ごとに作らず、取り消しの旗も列で 1 つ）。

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;

/// 仕事の中から見る取り消しの旗。
#[derive(Clone, Debug)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// 取り消しを頼まれたか。
    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// 旗そのもの（取り消しを見る core・io の関数へ渡す）。
    pub fn flag(&self) -> &AtomicBool {
        &self.0
    }
}

/// 受け口を 1 回見た結果。
#[derive(Debug)]
pub enum Polled<M> {
    /// まだ何も届いていない。
    Empty,
    Message(M),
    /// 仕事が知らせずに止まった（送り口が捨てられた。スレッドの異常など）。
    Lost,
}

/// 別のスレッドで走る仕事の、画面の側の口（結果の受け口と取り消しの旗）。
pub struct Worker<M> {
    rx: Receiver<M>,
    cancel: Arc<AtomicBool>,
    cancel_on_drop: bool,
}

impl<M: Send + 'static> Worker<M> {
    /// 名前を付けたスレッドで `work` を始める。スレッドを作れなければ誤りを返す（仕事は始まらない）。
    pub fn spawn(
        name: &str,
        work: impl FnOnce(Sender<M>, Cancel) + Send + 'static,
    ) -> io::Result<Worker<M>> {
        Self::spawn_on(
            std::thread::Builder::new().name(name.into()),
            Arc::default(),
            work,
        )
    }

    /// スレッドの作り方（名前・スタックの大きさ）と、取り消しの旗を渡して始める（旗をほかの所にも登録しておく仕事のため）。
    pub fn spawn_on(
        builder: std::thread::Builder,
        cancel: Arc<AtomicBool>,
        work: impl FnOnce(Sender<M>, Cancel) + Send + 'static,
    ) -> io::Result<Worker<M>> {
        let (tx, rx) = channel();
        let flag = Cancel(cancel.clone());
        builder.spawn(move || work(tx, flag))?;
        Ok(Worker {
            rx,
            cancel,
            cancel_on_drop: false,
        })
    }

    /// スレッドの無い受け口と、その送り口（試験用: 終わらない仕事を置く。送り口を持っているあいだは走っているまま）。
    #[doc(hidden)]
    pub fn parked() -> (Worker<M>, Sender<M>) {
        let (tx, rx) = channel();
        (
            Worker {
                rx,
                cancel: Arc::default(),
                cancel_on_drop: false,
            },
            tx,
        )
    }
}

impl<M> Worker<M> {
    /// 受け口を捨てたら、取り消しの旗も立てる（結果の行き先が無くなる仕事。走り続けても受ける者がいない）。
    pub fn cancel_on_drop(mut self) -> Self {
        self.cancel_on_drop = true;
        self
    }

    /// 取り消しを頼む（止まるのは仕事が次に旗を見たとき）。
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_canceled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// 届いた物を 1 つ受ける（待たない）。
    pub fn poll(&self) -> Polled<M> {
        match self.rx.try_recv() {
            Ok(m) => Polled::Message(m),
            Err(TryRecvError::Empty) => Polled::Empty,
            Err(TryRecvError::Disconnected) => Polled::Lost,
        }
    }

    /// 届くまで、`timeout` まで待つ（試験の待ち）。待ちきれなければ `Empty`。
    #[doc(hidden)]
    pub fn wait(&self, timeout: std::time::Duration) -> Polled<M> {
        match self.rx.recv_timeout(timeout) {
            Ok(m) => Polled::Message(m),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Polled::Empty,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Polled::Lost,
        }
    }
}

impl<M> Drop for Worker<M> {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_worker_delivers_its_messages_then_reports_lost_when_the_thread_ends() {
        let w = Worker::spawn("yolu-test-worker", |tx, _| {
            tx.send(1).unwrap();
            tx.send(2).unwrap();
        })
        .unwrap();
        assert!(matches!(
            w.wait(Duration::from_secs(10)),
            Polled::Message(1)
        ));
        assert!(matches!(
            w.wait(Duration::from_secs(10)),
            Polled::Message(2)
        ));
        assert!(matches!(w.wait(Duration::from_secs(10)), Polled::Lost));
        assert!(matches!(w.poll(), Polled::Lost));
    }

    #[test]
    fn cancel_reaches_the_work_and_drop_raises_the_flag_only_when_chosen() {
        let (seen_tx, seen_rx) = channel();
        let w: Worker<()> = Worker::spawn("yolu-test-cancel", move |_tx, cancel| {
            while !cancel.is_set() {
                std::thread::sleep(Duration::from_millis(1));
            }
            seen_tx.send(cancel.flag().load(Ordering::Relaxed)).unwrap();
        })
        .unwrap();
        assert!(!w.is_canceled());
        assert!(matches!(w.poll(), Polled::Empty));
        w.cancel();
        assert!(w.is_canceled());
        assert!(seen_rx.recv_timeout(Duration::from_secs(10)).unwrap());

        // 捨てても、選ばなければ旗は立たない
        let flag = Arc::new(AtomicBool::new(false));
        let plain: Worker<()> =
            Worker::spawn_on(std::thread::Builder::new(), flag.clone(), |_tx, _cancel| {}).unwrap();
        drop(plain);
        assert!(!flag.load(Ordering::Relaxed));
        let chosen: Worker<()> =
            Worker::spawn_on(std::thread::Builder::new(), flag.clone(), |_tx, _cancel| {})
                .unwrap()
                .cancel_on_drop();
        drop(chosen);
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn a_parked_worker_stays_empty_until_its_sender_sends_or_goes() {
        let (w, tx) = Worker::<u8>::parked();
        assert!(matches!(w.poll(), Polled::Empty));
        tx.send(7).unwrap();
        assert!(matches!(w.poll(), Polled::Message(7)));
        drop(tx);
        assert!(matches!(w.poll(), Polled::Lost));
    }
}
