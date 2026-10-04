//! 裏の書き手。実行中の書き込みは 1 つで、その間に来た頼みは最新の 1 つだけを待たせる（古い待ちは捨てる）。主のスレッドは
//! 材料を渡すだけで、正本への詰め直し・ハッシュ・ディスクの書き込みは、ここの別のスレッドが行う。`Fault` は試験用の障害の
//! 注入（段の名前は `GenerationStore` と同じに、書き込みの初めの `snapshot` を足す）。

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use yolu_io::{CommitOptions, Committed, Fault, Files, GenerationStore, RecoveryInfo, StoreError, INFO_NAME};

use super::capture::{build, Capture, Fingerprint};
use super::RecoveryError;

/// 書き込みの頼み。
pub(crate) struct Request {
    pub capture: Capture,
    pub root: PathBuf,
    /// この実行の置き場に残す世代の数。`None` は整理しない（設定を読めず、利用者が選んだ数が分からないとき）。
    pub keep: Option<usize>,
    /// 頼んだときの保存・開くの世代（この世代と違う結果は、保存済みの印に使わない）。
    pub epoch: u64,
}

/// 書き込みの結果。
pub(crate) struct Outcome {
    pub fingerprint: Fingerprint,
    pub epoch: u64,
    pub result: Result<Committed, RecoveryError>,
    pub millis: f64,
}

#[derive(Default)]
struct Inner {
    pending: Option<Request>,
    running: bool,
    results: VecDeque<Outcome>,
    /// 置き場の今の世代の札（次の確定が期待する値）。
    token: Option<String>,
}

/// 窓ごとに 1 つの書き手。
pub(crate) struct Writer {
    shared: Arc<(Mutex<Inner>, Condvar)>,
    fault: Option<Fault>,
    /// 試験用: 書き込みの予算（1 エントリ・合計の上限バイト数）。
    budget: Option<(u64, u64)>,
}

impl Writer {
    pub fn new() -> Self {
        Self {
            shared: Arc::default(),
            fault: None,
            budget: None,
        }
    }

    pub fn set_fault(&mut self, fault: Option<Fault>) {
        self.fault = fault;
    }

    pub fn set_budget(&mut self, budget: Option<(u64, u64)>) {
        self.budget = budget;
    }

    /// 動いていない（実行中も待ちも無い）。
    pub fn is_idle(&self) -> bool {
        let inner = self.shared.0.lock().unwrap();
        !inner.running && inner.pending.is_none()
    }

    /// 頼みを出す。実行中ならその次の 1 つとして待たせる（待っていた古い頼みは捨てる）。
    pub fn submit(&self, request: Request) {
        let (lock, _) = &*self.shared;
        let mut inner = lock.lock().unwrap();
        inner.pending = Some(request);
        if inner.running {
            return;
        }
        inner.running = true;
        let shared = self.shared.clone();
        let (fault, budget) = (self.fault.clone(), self.budget);
        std::thread::spawn(move || run(shared, fault, budget));
    }

    /// 実行中と待ちが全部終わるまで待つ。
    pub fn wait(&self) {
        self.wait_until(None);
    }

    /// 終わるまで、か `deadline` まで待つ。終わったら true（遅いディスクで終了が固まらないように、終わる前は期限を付ける）。
    pub fn wait_until(&self, deadline: Option<Instant>) -> bool {
        let (lock, cvar) = &*self.shared;
        let mut inner = lock.lock().unwrap();
        while inner.running || inner.pending.is_some() {
            match deadline {
                None => inner = cvar.wait(inner).unwrap(),
                Some(d) => {
                    let left = d.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return false;
                    }
                    inner = cvar.wait_timeout(inner, left).unwrap().0;
                }
            }
        }
        true
    }

    pub fn take(&self) -> Option<Outcome> {
        self.shared.0.lock().unwrap().results.pop_front()
    }

    /// 置き場を外から変えた（世代を捨てた）あとの、次の確定が期待する札を読み直す。外の書き手の変更を採る口ではない。
    pub fn resync(&self, store: &GenerationStore) {
        self.shared.0.lock().unwrap().token = store.token().ok();
    }
}

fn run(shared: Arc<(Mutex<Inner>, Condvar)>, fault: Option<Fault>, budget: Option<(u64, u64)>) {
    let (lock, cvar) = &*shared;
    loop {
        let (request, token) = {
            let mut inner = lock.lock().unwrap();
            match inner.pending.take() {
                Some(r) => (r, inner.token.clone()),
                None => {
                    inner.running = false;
                    cvar.notify_all();
                    return;
                }
            }
        };
        let started = Instant::now();
        let fingerprint = request.capture.fingerprint.clone();
        let epoch = request.epoch;
        // 書き込みの中の失敗（パニック）で、書き手が「動いている」まま止まらないようにする（終わるときの待ちが固まる）
        let (result, token) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            write(&request, token.clone(), fault.as_ref(), budget)
        }))
        .unwrap_or((Err(RecoveryError::Panicked), token));
        let outcome = Outcome {
            fingerprint,
            epoch,
            result,
            millis: started.elapsed().as_secs_f64() * 1000.0,
        };
        let mut inner = lock.lock().unwrap();
        inner.token = token;
        inner.results.push_back(outcome);
    }
}

/// 1 回の書き込み。返す札は次の確定が期待する値（確定したら新しい札。確定の後で失敗しても、置き場を読み直して合わせる）。
fn write(
    request: &Request,
    token: Option<String>,
    fault: Option<&Fault>,
    budget: Option<(u64, u64)>,
) -> (Result<Committed, RecoveryError>, Option<String>) {
    // current を置き換えた（確定した）かを、障害の注入より先に見て覚える
    let switched = Arc::new(AtomicBool::new(false));
    let seen = switched.clone();
    let user = fault.cloned();
    let hook: Fault = Arc::new(move |stage: &str| {
        if stage == "after-pointer" {
            seen.store(true, Ordering::SeqCst);
        }
        match &user {
            Some(f) => f(stage),
            None => Ok(()),
        }
    });
    let mut store = GenerationStore::new(&request.root).with_fault(hook);
    if let Some((entry, total)) = budget {
        store = store.with_budget(entry, total);
    }
    let result = (|| -> Result<Committed, RecoveryError> {
        if let Some(f) = fault {
            f("snapshot").map_err(|e| RecoveryError::Store(StoreError::Io(e)))?;
        }
        let capture = &request.capture;
        let project = build(capture)?;
        let mut files: Files = project.original_archive().entries().clone();
        let info = RecoveryInfo {
            title: capture.title.clone(),
            project_path: capture.project_path.clone(),
            project_token: String::new(),
            unchanged: false,
            sets: capture.sets.len(),
        };
        files.insert(INFO_NAME.into(), Arc::from(info.to_bytes()));
        Ok(store.commit(
            &files,
            &CommitOptions {
                expected: token.as_deref(),
                keep: request.keep,
                share: true,
            },
        )?)
    })();
    let token = match &result {
        Ok(committed) => Some(committed.token.clone()),
        // 確定の後で通知を失った場合だけ、置き場を読み直して次回の札を合わせる（外の書き手の確定は採らない）
        Err(_) if switched.load(Ordering::SeqCst) => store.token().ok(),
        Err(_) => token,
    };
    (result, token)
}
