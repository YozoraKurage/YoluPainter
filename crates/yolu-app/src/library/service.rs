//! サムネイルと項目の情報を別のスレッドで作る仕事場。描いている間も画面のスレッドを止めない。
//!
//! 画面は毎フレーム `begin_frame` のあと、いま見えている項目だけを `request` する（札は呼び出し側が決める。同じ札の仕事は 1 つだけ受ける）。
//! 仕事は受けた順に少数のスレッドが 1 つずつ走らせ、できた結果は `poll` で画面のスレッドが受け取る（1 回に受け取る数を絞れる）。
//! スクロールで見えなくなった項目の仕事は、始める前に捨てる（`STALE_FRAMES` フレーム見に来なければ）ので、速く動かしても見えている所が
//! 後回しにならない。捨てた札は、また見えれば、もう一度頼める。仕事の途中は止められないが、`Cancel` を見て切り上げられ、
//! 仕事場を落とす（プロジェクトを替える・パネルを閉じる）と、残りの仕事は始めず、走っている仕事の結果は捨てる。
//! 仕事がパニックしても、スレッドは続き、結果は `None` で返す（札が残り続けて頼み直し続けることはない）。
use std::collections::{HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

/// この数のフレームのあいだ見に来なかった項目の仕事は、始める前に捨てる。
pub const STALE_FRAMES: u64 = 3;

/// 仕事への取り消しの旗（仕事場を落としたときに立つ）。
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    fn set(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// 旗そのもの（ファイルを読み書きする関数へ渡す）。
    pub fn flag(&self) -> &AtomicBool {
        &self.0
    }
}

pub type Work<R> = Box<dyn FnOnce(&Cancel) -> R + Send>;

struct Job<R> {
    key: String,
    work: Work<R>,
}

struct Queue<R> {
    jobs: VecDeque<Job<R>>,
    /// 頼んであって、結果をまだ画面が受け取っていない札（走る前・走っている間・結果の受け取り待ち）と、最後に見に来たフレーム。
    wanted: HashMap<String, u64>,
    frame: u64,
    running: usize,
    held: bool,
    spawned: bool,
}

struct Shared<R> {
    queue: Mutex<Queue<R>>,
    wake: Condvar,
    idle: Condvar,
    cancel: Cancel,
    repaint: Mutex<Option<egui::Context>>,
}

/// 結果 `R` を別のスレッドで作る仕事場。
pub struct Service<R: Send + 'static> {
    shared: Arc<Shared<R>>,
    tx: Sender<(String, Option<R>)>,
    rx: Receiver<(String, Option<R>)>,
    workers: usize,
}

impl<R: Send + 'static> Service<R> {
    /// スレッドは最初の仕事を頼むまで起こさない。
    pub fn new(workers: usize) -> Service<R> {
        let (tx, rx) = channel();
        Service {
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue {
                    jobs: VecDeque::new(),
                    wanted: HashMap::new(),
                    frame: 0,
                    running: 0,
                    held: false,
                    spawned: false,
                }),
                wake: Condvar::new(),
                idle: Condvar::new(),
                cancel: Cancel::default(),
                repaint: Mutex::new(None),
            }),
            tx,
            rx,
            workers: workers.max(1),
        }
    }

    /// できたときに画面を描き直させる口。
    pub fn set_context(&self, ctx: egui::Context) {
        *self.shared.repaint.lock().unwrap() = Some(ctx);
    }

    /// 新しいフレームの始まり（見に来た印の時計を進める）。
    pub fn begin_frame(&self) {
        self.shared.queue.lock().unwrap().frame += 1;
    }

    /// 札 `key` の仕事を頼む（すでに頼んであれば、見に来たことだけを覚える。仕事を作る `make` は、新しく頼むときだけ呼ぶ）。
    /// 新しく頼んだら true。
    pub fn request(&self, key: &str, make: impl FnOnce() -> Work<R>) -> bool {
        let mut q = self.shared.queue.lock().unwrap();
        let frame = q.frame;
        if let Some(seen) = q.wanted.get_mut(key) {
            *seen = frame;
            return false;
        }
        q.wanted.insert(key.to_owned(), frame);
        q.jobs.push_back(Job {
            key: key.to_owned(),
            work: make(),
        });
        let first = !q.spawned;
        q.spawned = true;
        drop(q);
        if first {
            self.spawn();
        }
        self.shared.wake.notify_one();
        true
    }

    fn spawn(&self) {
        for n in 0..self.workers {
            let shared = self.shared.clone();
            let tx = self.tx.clone();
            // スレッドを立てられなくても、画面は動き続ける（札は頼みっぱなしで、結果が来ないだけ）
            let _ = std::thread::Builder::new()
                .name(format!("yolu-thumbnails-{n}"))
                .spawn(move || worker(shared, tx));
        }
    }

    /// できた結果を `max` 個まで受け取る（無ければ空）。仕事がパニックしたものは `None`。
    pub fn poll(&self, max: usize) -> Vec<(String, Option<R>)> {
        let mut out = Vec::new();
        while out.len() < max {
            let Ok(done) = self.rx.try_recv() else { break };
            out.push(done);
        }
        if !out.is_empty() {
            let mut q = self.shared.queue.lock().unwrap();
            for (key, _) in &out {
                q.wanted.remove(key);
            }
        }
        out
    }

    /// 頼んであって、結果をまだ受け取っていない数。
    pub fn pending(&self) -> usize {
        self.shared.queue.lock().unwrap().wanted.len()
    }

    /// 札がまだ頼んであって、結果を受け取っていないか。
    pub fn is_pending(&self, key: &str) -> bool {
        self.shared.queue.lock().unwrap().wanted.contains_key(key)
    }

    /// 試験用: true の間、新しい仕事を始めない（走っている仕事は続く）。
    pub fn hold(&self, hold: bool) {
        self.shared.queue.lock().unwrap().held = hold;
        self.shared.wake.notify_all();
    }

    /// 試験用: 走る前の仕事と走っている仕事がなくなるまで待つ（結果は `poll` で受け取る）。
    pub fn wait_done(&self) {
        let mut q = self.shared.queue.lock().unwrap();
        while !q.jobs.is_empty() || q.running > 0 {
            q = self.shared.idle.wait(q).unwrap();
        }
    }
}

impl<R: Send + 'static> Drop for Service<R> {
    fn drop(&mut self) {
        self.shared.cancel.set();
        let mut q = self.shared.queue.lock().unwrap();
        q.jobs.clear();
        drop(q);
        self.shared.wake.notify_all();
    }
}

fn worker<R: Send + 'static>(shared: Arc<Shared<R>>, tx: Sender<(String, Option<R>)>) {
    loop {
        let job = {
            let mut q = shared.queue.lock().unwrap();
            loop {
                if shared.cancel.is_set() {
                    return;
                }
                if !q.held {
                    if let Some(job) = q.jobs.pop_front() {
                        let fresh = q
                            .wanted
                            .get(&job.key)
                            .is_some_and(|seen| seen + STALE_FRAMES >= q.frame);
                        if fresh {
                            q.running += 1;
                            break job;
                        }
                        // 見に来なくなった項目の仕事は捨てる（また見えれば頼み直せる）
                        q.wanted.remove(&job.key);
                        if q.jobs.is_empty() && q.running == 0 {
                            shared.idle.notify_all();
                        }
                        continue;
                    }
                }
                q = shared.wake.wait(q).unwrap();
            }
        };
        let Job { key, work } = job;
        let result = catch_unwind(AssertUnwindSafe(|| work(&shared.cancel))).ok();
        let cancelled = shared.cancel.is_set();
        // 結果を渡してから「走っていない」ことにする（`wait_done` が返ったときには、結果は受け口に入っている）
        if !cancelled {
            let _ = tx.send((key, result));
            if let Some(ctx) = shared.repaint.lock().unwrap().as_ref() {
                ctx.request_repaint();
            }
        }
        {
            let mut q = shared.queue.lock().unwrap();
            q.running -= 1;
            if q.jobs.is_empty() && q.running == 0 {
                shared.idle.notify_all();
            }
        }
        if cancelled {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_for(mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(10), "待ちすぎ");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn collect<R: Send + 'static>(service: &Service<R>, n: usize) -> Vec<(String, Option<R>)> {
        let mut out = Vec::new();
        wait_for(|| {
            out.extend(service.poll(64));
            out.len() >= n
        });
        out
    }

    #[test]
    fn a_job_runs_on_another_thread_and_its_result_is_polled() {
        let service: Service<(String, std::thread::ThreadId)> = Service::new(1);
        let main = std::thread::current().id();
        service.request("a", || {
            Box::new(|_| ("done".to_owned(), std::thread::current().id()))
        });
        let done = collect(&service, 1);
        let (key, result) = &done[0];
        let (text, thread) = result.as_ref().unwrap();
        assert_eq!((key.as_str(), text.as_str()), ("a", "done"));
        assert_ne!(*thread, main);
        assert_eq!(service.pending(), 0);
    }

    #[test]
    fn the_same_key_is_asked_once_until_its_result_is_taken() {
        let service: Service<u32> = Service::new(1);
        let runs = Arc::new(std::sync::atomic::AtomicU32::new(0));
        for _ in 0..5 {
            let runs = runs.clone();
            service.request("k", move || {
                Box::new(move |_| {
                    runs.fetch_add(1, Ordering::SeqCst);
                    7
                })
            });
        }
        service.wait_done();
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        // 結果を受け取る前は、頼み直さない
        assert!(!service.request("k", || Box::new(|_| 8)));
        assert_eq!(service.pending(), 1);
        let done = service.poll(10);
        assert_eq!(done.len(), 1);
        assert_eq!(service.pending(), 0);
        // 受け取ったあとは、頼み直せる
        assert!(service.request("k", || Box::new(|_| 9)));
        assert_eq!(collect(&service, 1)[0].1, Some(9));
    }

    #[test]
    fn the_poll_limit_leaves_the_rest_for_the_next_frame() {
        let service: Service<u32> = Service::new(2);
        for i in 0..6u32 {
            service.request(&i.to_string(), move || Box::new(move |_| i));
        }
        service.wait_done();
        assert_eq!(service.poll(2).len(), 2);
        assert_eq!(service.pending(), 4);
        assert_eq!(service.poll(100).len(), 4);
        assert_eq!(service.pending(), 0);
    }

    #[test]
    fn a_slow_job_does_not_block_the_caller() {
        let service: Service<u32> = Service::new(1);
        let gate = Arc::new(AtomicBool::new(false));
        let waiting = gate.clone();
        let start = Instant::now();
        service.request("slow", move || {
            Box::new(move |_| {
                while !waiting.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                1
            })
        });
        // 頼む・見に行く・受け取るは、仕事が終わるのを待たない
        assert!(service.poll(10).is_empty());
        assert!(service.is_pending("slow"));
        assert!(start.elapsed() < Duration::from_secs(2));
        gate.store(true, Ordering::SeqCst);
        assert_eq!(collect(&service, 1)[0].1, Some(1));
    }

    #[test]
    fn jobs_for_items_nobody_looks_at_any_more_are_dropped_unrun() {
        let service: Service<u32> = Service::new(1);
        service.hold(true);
        let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
        for name in ["gone", "kept"] {
            let ran = ran.clone();
            service.request(name, move || {
                Box::new(move |_| {
                    ran.lock().unwrap().push(name);
                    0
                })
            });
        }
        // スクロールして、「gone」は見に来ない・「kept」は見に来る、というフレームが続く
        for _ in 0..(STALE_FRAMES + 2) {
            service.begin_frame();
            service.request("kept", || unreachable!("頼み済み"));
        }
        service.hold(false);
        service.wait_done();
        let done = service.poll(10);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].0, "kept");
        assert_eq!(*ran.lock().unwrap(), vec!["kept"]);
        // 捨てた札は、また見えれば頼み直せる
        assert!(!service.is_pending("gone"));
        assert!(service.request("gone", || Box::new(|_| 5)));
        assert_eq!(collect(&service, 1)[0].1, Some(5));
    }

    #[test]
    fn a_panicking_job_returns_none_and_the_thread_goes_on() {
        let service: Service<u32> = Service::new(1);
        service.request("boom", || Box::new(|_| panic!("仕事の失敗")));
        service.request("fine", || Box::new(|_| 3));
        let mut done = collect(&service, 2);
        done.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(done[0], ("boom".to_owned(), None));
        assert_eq!(done[1], ("fine".to_owned(), Some(3)));
        assert_eq!(service.pending(), 0);
    }

    #[test]
    fn dropping_the_service_cancels_the_running_job_and_starts_no_more() {
        let service: Service<u32> = Service::new(1);
        let (started_tx, started_rx) = channel();
        let observed = Arc::new(AtomicBool::new(false));
        let seen = observed.clone();
        service.request("first", move || {
            Box::new(move |cancel| {
                started_tx.send(()).unwrap();
                let start = Instant::now();
                while !cancel.is_set() && start.elapsed() < Duration::from_secs(10) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                seen.store(cancel.is_set(), Ordering::SeqCst);
                1
            })
        });
        let second_ran = Arc::new(AtomicBool::new(false));
        let flag = second_ran.clone();
        service.request("second", move || {
            Box::new(move |_| {
                flag.store(true, Ordering::SeqCst);
                2
            })
        });
        started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        drop(service);
        wait_for(|| observed.load(Ordering::SeqCst));
        std::thread::sleep(Duration::from_millis(20));
        assert!(!second_ran.load(Ordering::SeqCst));
    }

    #[test]
    fn a_service_that_was_never_asked_starts_no_thread() {
        let service: Service<u32> = Service::new(4);
        service.begin_frame();
        assert!(service.poll(1).is_empty());
        assert_eq!(service.pending(), 0);
        assert!(!service.shared.queue.lock().unwrap().spawned);
    }
}
