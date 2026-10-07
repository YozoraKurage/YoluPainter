//! 外からの操作（AI のアシスタントの MCP のクライアント・コマンドラインなど、同じ PC のプログラムからの命令）を受ける、起動中のアプリの側。
//!
//! - 設定「外からの操作を受ける」（既定は切）が入っている間だけ、`http://127.0.0.1:<番号>/mcp`（番号は設定。既定は 17347）で MCP（Streamable HTTP）を
//!   待つ。受け口（hyper と rmcp。127.0.0.1 だけ・Host と Origin の確かめ・同時のつながりの数・本文の上限）は yolu-mcp の `http` で、tokio は
//!   受け口のスレッドの中だけで回す（画面のスレッドに持ち込まない）。ツールの一覧・資料・見本の画像の記憶は yolu-mcp の `server` が持つ。
//! - ツールの呼び出し（命令）は、受け口のスレッドから列（[`QUEUE`] 件）で画面のスレッドへ渡し、画面を起こす。画面のスレッドは `poll` で
//!   1 フレームに [`REQUESTS_PER_FRAME`] まで `AppHost`（`ops_host`）で今のプロジェクトに当てる（1 命令 = 画面の取り消しの 1 段）。返事は
//!   要求ごとの一回きりの受け口（oneshot）で返す。保存は裏のスレッドで動かし、返事は保存が終わったフレームで返す（返事待ちの間も画面は動く）。
//! - 待てない（番号がほかのプログラムに使われている など）ときは、理由を出して待たない（状態の帯の印が「受けられない」になり、理由はツールチップ）。
//!   失敗した番号のままでは毎フレームは試さない（設定を切って入れ直すか、番号を変えると試し直す）。
//! - 切る・番号を変えると、待ちをやめる。列に残っていた要求と返事待ちの保存には「受け付けをやめた」誤りを返し（保存そのものは続く）、走っている
//!   要求の返事を書き終えてから口を閉じる。
//! - 知らせは出しすぎない: 命令ごとのログは出さない。壊す操作（消す・保存・置き換える書き出し）が済んだときだけ短く知らせる。断った要求
//!   （Host・Origin が自分でない・つながりの数が上限）と待てない理由は、続いても 1 度だけ。

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;

use tokio::sync::oneshot;
use yolu_mcp::http::{self, Limits, Observer, Refusal, Running};
use yolu_mcp::{reach, Backend, BackendFuture, YoluMcp};
use yolu_ops::error::{ErrorCode, OpError};
use yolu_ops::meta::{command_spec, Danger};
use yolu_ops::reply::Reply;
use yolu_ops::Command;

use crate::lang::Lang;
use crate::notice::Source;
use crate::ops_host::{AppHost, StartedSave};
use crate::state::AppState;

/// 画面のスレッドが 1 フレームで実行する要求の数（残りは次のフレーム。長い命令が続いても、画面が止まり続けない）。
pub const REQUESTS_PER_FRAME: usize = 8;
/// 受け口のスレッドが画面のスレッドへ渡す要求の列の長さ（同時のつながりは 8 つまでで、1 つのつながりは 1 つずつ頼むので、満ちない）。
pub const QUEUE: usize = 64;

/// 待ち受けの様子。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum OpsStatus {
    /// 受けていない（設定が切）。
    #[default]
    Off,
    /// 待ち受けている。
    Listening,
    /// 開いているつながりの数。
    Connected(usize),
    /// 待ち受けられない（理由。番号がほかのプログラムに使われている など）。
    Failed(String),
}

/// 状態の帯の印の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpsIndicator {
    Waiting,
    Connected,
    Failed,
}

/// 画面が読む、外からの操作の様子の写し。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpsView {
    pub status: OpsStatus,
    /// 待っている番号（待っていなければ 0）。
    pub port: u16,
}

impl OpsView {
    /// 待ち受けているか、つながっている。
    pub fn is_on(&self) -> bool {
        matches!(self.status, OpsStatus::Listening | OpsStatus::Connected(_))
    }

    /// 状態の帯の印（受けていなければ無い）。
    pub fn indicator(&self) -> Option<OpsIndicator> {
        match self.status {
            OpsStatus::Off => None,
            OpsStatus::Listening => Some(OpsIndicator::Waiting),
            OpsStatus::Connected(_) => Some(OpsIndicator::Connected),
            OpsStatus::Failed(_) => Some(OpsIndicator::Failed),
        }
    }

    /// 印のツールチップ（受けていなければ空）。待っている間は、つなぐ側に渡す URL を添える。
    pub fn tooltip(&self, lang: Lang) -> String {
        let url = yolu_mcp::endpoint(self.port);
        match &self.status {
            OpsStatus::Off => String::new(),
            OpsStatus::Listening => lang.pick(
                format!("外からの操作を待っています（{url}）"),
                format!("Waiting for external commands ({url})"),
            ),
            OpsStatus::Connected(n) => lang.pick(
                format!("外からの操作を受けています（つながり {n}・{url}）"),
                format!("Accepting external commands ({n} connected, {url})"),
            ),
            OpsStatus::Failed(reason) => lang.with_reason(
                lang.pick(
                    "外からの操作を受けられません",
                    "Cannot accept external commands",
                ),
                reason,
            ),
        }
    }
}

/// 受け口のスレッドから画面のスレッドへ渡す要求。
struct Job {
    command: Command,
    reply: oneshot::Sender<Result<Reply, OpError>>,
}

/// 命令を画面のスレッドへ渡す相手（受け口のスレッドで動く）。
struct AppBackend {
    jobs: SyncSender<Job>,
    counters: Arc<Counters>,
    ctx: egui::Context,
}

impl Backend for AppBackend {
    fn run(&self, command: Command) -> BackendFuture {
        let (reply, answer) = oneshot::channel();
        let sent = self.jobs.try_send(Job { command, reply });
        if sent.is_ok() {
            self.counters.sent.fetch_add(1, Ordering::AcqRel);
        }
        // 隠れた窓でも、描き直しの頼みで `logic` が回る
        self.ctx.request_repaint();
        Box::pin(async move {
            match sent {
                Ok(()) => answer.await.unwrap_or_else(|_| Err(reach::stopped())),
                Err(TrySendError::Full(_)) => Err(OpError::new(
                    ErrorCode::Busy,
                    "外からの操作の要求が多すぎます。少し待ってから、もう一度頼みます",
                    "Too many external commands are waiting; wait a moment and try again",
                )),
                Err(TrySendError::Disconnected(_)) => Err(reach::stopped()),
            }
        })
    }
}

/// 受け口の様子（受け口のスレッドが書き、画面のスレッドがフレームの頭で読む）。
#[derive(Default)]
struct Counters {
    open: AtomicUsize,
    /// 列へ渡した要求の数。
    sent: AtomicU64,
    host: AtomicU64,
    origin: AtomicU64,
    busy: AtomicU64,
}

impl Counters {
    fn refusals(&self) -> [(Refusal, u64); 3] {
        [
            (Refusal::Host, self.host.load(Ordering::Acquire)),
            (Refusal::Origin, self.origin.load(Ordering::Acquire)),
            (Refusal::Busy, self.busy.load(Ordering::Acquire)),
        ]
    }
}

struct AppObserver {
    counters: Arc<Counters>,
    ctx: egui::Context,
}

impl Observer for AppObserver {
    fn connections(&self, open: usize) {
        self.counters.open.store(open, Ordering::Release);
        self.ctx.request_repaint();
    }
    fn refused(&self, refusal: Refusal) {
        let counter = match refusal {
            Refusal::Host => &self.counters.host,
            Refusal::Origin => &self.counters.origin,
            Refusal::Busy => &self.counters.busy,
        };
        counter.fetch_add(1, Ordering::AcqRel);
        self.ctx.request_repaint();
    }
}

/// 裏で動いている保存の返事待ち（保存が終わったフレームで返す）。
struct PendingSave {
    reply: oneshot::Sender<Result<Reply, OpError>>,
    command: &'static str,
    started: StartedSave,
}

/// 待ち受けている間の持ち物。
struct Listening {
    /// 落とすと受け口を止める（返事を書き終えて口を閉じるまで待つ）。
    running: Running,
    jobs: Receiver<Job>,
    counters: Arc<Counters>,
    /// 最後に見た、断った数（種類ごと）。
    seen_refusals: [u64; 3],
    /// 列から取り出した要求の数。
    taken: u64,
}

/// 外からの操作を受ける（`YoluApp` が 1 つ持つ）。
pub struct McpServer {
    listening: Option<Listening>,
    /// 待ち受けを試した番号（設定が入っている間、同じ番号で失敗しても毎フレームは試さない。設定を切る・番号を変えると試し直す）。
    tried: Option<u16>,
    /// 待ち受けられなかった理由。
    failure: Option<String>,
    pending: Option<PendingSave>,
    /// 最後に知らせた断りの種類（同じ断りが続いても、知らせは 1 度）。
    last_refusal: Option<Refusal>,
    ctx: Option<egui::Context>,
    /// これまでに実行した要求の数（試験・診断用）。
    handled: u64,
    /// 1 フレームに実行する要求の数（既定は [`REQUESTS_PER_FRAME`]。試験が小さくする）。
    per_frame: usize,
}

impl Default for McpServer {
    fn default() -> Self {
        McpServer::new()
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 待てなかった理由の文。
fn bind_failure(lang: Lang, port: u16, error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::AddrInUse => lang.pick(
            format!("{port} 番はほかのプログラムが使っています"),
            format!("Port {port} is used by another program"),
        ),
        std::io::ErrorKind::PermissionDenied => lang.pick(
            format!("{port} 番は使えません（OS が予約しています）"),
            format!("Port {port} cannot be used (reserved by the system)"),
        ),
        _ => error.to_string(),
    }
}

impl McpServer {
    pub fn new() -> McpServer {
        McpServer {
            listening: None,
            tried: None,
            failure: None,
            pending: None,
            last_refusal: None,
            ctx: None,
            handled: 0,
            per_frame: REQUESTS_PER_FRAME,
        }
    }

    /// 試験用: 1 フレームに実行する要求の数を替える（上限で次のフレームへ回すことを、少ない要求で確かめる）。
    #[doc(hidden)]
    pub fn set_requests_per_frame(&mut self, n: usize) {
        self.per_frame = n.max(1);
    }

    /// 試験用: 列で待っている要求の数（受け口のスレッドが渡し、画面のスレッドがまだ取り出していない数）。
    #[doc(hidden)]
    pub fn waiting(&self) -> u64 {
        self.listening.as_ref().map_or(0, |l| {
            l.counters
                .sent
                .load(Ordering::Acquire)
                .saturating_sub(l.taken)
        })
    }

    /// 待っている番号（待っていなければ None。設定の番号が 0 の試験では、OS が選んだ番号）。
    pub fn port(&self) -> Option<u16> {
        self.listening.as_ref().map(|l| l.running.port())
    }

    /// 待ち受けの様子。
    pub fn status(&self) -> OpsStatus {
        if let Some(reason) = &self.failure {
            return OpsStatus::Failed(reason.clone());
        }
        match &self.listening {
            None => OpsStatus::Off,
            Some(l) => match l.counters.open.load(Ordering::Acquire) {
                0 => OpsStatus::Listening,
                n => OpsStatus::Connected(n),
            },
        }
    }

    /// 画面に写す様子。
    pub fn view(&self) -> OpsView {
        OpsView {
            status: self.status(),
            port: self.port().or(self.tried).unwrap_or(0),
        }
    }

    /// これまでに実行した要求の数（断った・誤りにした要求も数える。試験・診断用）。
    pub fn handled(&self) -> u64 {
        self.handled
    }

    /// 裏で動いている保存の返事を待っているか（試験・診断用）。
    pub fn save_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// 設定（`want` と番号）に合わせて、待ち受けを始める・やめる。番号が変われば待ち直す。始めるのは、その番号で 1 度だけ
    /// （失敗しても毎フレームは試さない）。
    pub fn sync(&mut self, want: bool, port: u16, ctx: &egui::Context, state: &mut AppState) {
        if want {
            if self.tried.is_some_and(|tried| tried != port) {
                self.stop();
            }
            if self.listening.is_none() && self.tried.is_none() {
                self.start(port, ctx, state);
            }
        } else if self.listening.is_some() || self.tried.is_some() || self.failure.is_some() {
            self.stop();
        }
    }

    /// 待ち受けを始める。待てなければ、理由を出して待たない。
    pub fn start(&mut self, port: u16, ctx: &egui::Context, state: &mut AppState) {
        if self.listening.is_some() {
            return;
        }
        self.tried = Some(port);
        self.failure = None;
        self.ctx = Some(ctx.clone());
        let lang = state.lang;
        let listener = match http::bind(port) {
            Ok(l) => l,
            Err(e) => return self.fail(bind_failure(lang, port, &e), state),
        };
        let (jobs_tx, jobs) = mpsc::sync_channel(QUEUE);
        let counters = Arc::new(Counters::default());
        let backend = Arc::new(AppBackend {
            jobs: jobs_tx,
            counters: counters.clone(),
            ctx: ctx.clone(),
        });
        let observer = Arc::new(AppObserver {
            counters: counters.clone(),
            ctx: ctx.clone(),
        });
        let running =
            match Running::start(listener, YoluMcp::new(backend), Limits::default(), observer) {
                Ok(r) => r,
                Err(e) => return self.fail(e.to_string(), state),
            };
        self.listening = Some(Listening {
            running,
            jobs,
            counters,
            seen_refusals: [0; 3],
            taken: 0,
        });
    }

    fn fail(&mut self, reason: String, state: &mut AppState) {
        let text = OpsView {
            status: OpsStatus::Failed(reason.clone()),
            port: 0,
        }
        .tooltip(state.lang);
        state.fail(Source::Ops, text);
        self.failure = Some(reason);
    }

    /// 待ち受けをやめる。列に残っていた要求と返事待ちの保存には「受け付けをやめた」誤りを返し、受け口を止める（口を閉じるまで待つ）。
    pub fn stop(&mut self) {
        if let Some(listening) = self.listening.take() {
            while let Ok(job) = listening.jobs.try_recv() {
                let _ = job.reply.send(Err(reach::stopped()));
            }
            if let Some(pending) = self.pending.take() {
                let _ = pending.reply.send(Err(reach::stopped()));
            }
            // 列の受け手を先に落とす（このあとに届く要求は、送れずに「受け付けをやめた」になる）。受け口を止めて、口を閉じるまで待つ
            let Listening { running, jobs, .. } = listening;
            drop(jobs);
            drop(running);
        }
        self.tried = None;
        self.failure = None;
        self.last_refusal = None;
    }

    /// 受け口のスレッドからの要求を実行し、断りを知らせる（フレームの頭で）。
    pub fn poll(&mut self, state: &mut AppState) {
        self.finish_pending_save(state);
        self.note_refusals(state);
        let mut ran = 0;
        while let Some(listening) = &mut self.listening {
            if ran >= self.per_frame {
                // 残りは次のフレーム（長い命令が続いても、画面が止まり続けない）
                if let Some(ctx) = &self.ctx {
                    ctx.request_repaint();
                }
                break;
            }
            let Ok(job) = listening.jobs.try_recv() else {
                break;
            };
            listening.taken += 1;
            ran += 1;
            self.handle(job, state);
        }
        self.finish_pending_save(state);
    }

    /// 断った要求があれば、種類が変わったときだけ短く知らせる。
    fn note_refusals(&mut self, state: &mut AppState) {
        let Some(listening) = &mut self.listening else {
            return;
        };
        for (index, (kind, count)) in listening.counters.refusals().into_iter().enumerate() {
            if count != listening.seen_refusals[index] {
                listening.seen_refusals[index] = count;
                if self.last_refusal != Some(kind) {
                    self.last_refusal = Some(kind);
                    // 断りはログに残さない（短く知らせるだけ）
                    let text = refusal_text(state.lang, kind);
                    state.refuse(Source::Ops, text);
                }
            }
        }
    }

    /// 要求を画面のスレッドで実行して返す。保存は裏で始めて、返事は保存が終わってから。
    fn handle(&mut self, job: Job, state: &mut AppState) {
        self.handled += 1;
        // 前の保存の結果が出ていれば、先に返す（次の保存の頼みが、前の結果を消さない）
        self.finish_pending_save(state);
        let Job { command, reply } = job;
        let (result, started) = {
            let mut host = AppHost::new(state);
            // `execute` は途中の panic を `internal` の誤りにして返す（文書の編集は積んだ段を戻してから）。受け止めて動き続ける panic なので、落ちた記録にしない
            let result = crate::crash::handled(std::panic::AssertUnwindSafe(|| {
                yolu_ops::execute(&mut host, &command)
            }))
            .unwrap_or_else(|_| {
                Err(OpError::new(
                    ErrorCode::Internal,
                    "命令の途中で想定していない失敗が起きました",
                    "The command stopped because of an unexpected failure",
                ))
            });
            (result, host.take_started_save())
        };
        if let Some(started) = started {
            self.pending = Some(PendingSave {
                reply,
                command: command.name(),
                started,
            });
            self.finish_pending_save(state);
            return;
        }
        if result.is_ok() {
            // 通った要求があれば、次の断りはまた知らせる
            self.last_refusal = None;
        }
        if let Ok(done) = &result {
            if breaks_something(&command, done) {
                announce(state, command.name());
            }
        }
        // 相手がもう待っていない（つながりが切れた）なら、返事は捨てる
        let _ = reply.send(result);
    }

    /// 裏の保存が終わっていれば、返事を返す（結果は `SaveState::take_outcome`）。
    fn finish_pending_save(&mut self, state: &mut AppState) {
        if self.pending.is_none() {
            return;
        }
        let outcome = state.save.take_outcome();
        if outcome.is_none() && state.is_saving() {
            return;
        }
        let pending = self.pending.take().expect("上で見た");
        let response = match outcome {
            Some(outcome) => match outcome.result {
                Ok(facts) => {
                    if command_breaks(pending.command) || pending.started.replaced {
                        announce(state, pending.command);
                    }
                    Ok(AppHost::saved_reply(state, &pending.started, facts))
                }
                Err(text) => Err(OpError::new(ErrorCode::Io, text.clone(), text)
                    .with_data(serde_json::json!({"path": outcome.path.display().to_string()}))),
            },
            None => Err(OpError::new(
                ErrorCode::Internal,
                "保存の結果を受け取れませんでした",
                "The result of the save could not be received",
            )),
        };
        let _ = pending.reply.send(response);
    }
}

/// 壊す印のある命令か（`Danger::Always`）。
fn command_breaks(name: &str) -> bool {
    command_spec(name).is_some_and(|s| s.danger == Danger::Always)
}

/// 済んだ命令が、壊す操作だったか: 消す・上書き保存（いつも壊す印。ただし何も書かなかったものは除く）、置き換えたファイルのある書き出し、
/// 消す命令を含み何かを変えたアクション（`action.run`）。
fn breaks_something(command: &Command, reply: &Reply) -> bool {
    // 何も書かなかった上書き保存（編集が無い）は、壊す操作が済んだことにならない
    if matches!(reply, Reply::Saved(s) if !s.written) {
        return false;
    }
    if let (Command::ActionRun(args), Reply::Action(done)) = (command, reply) {
        return !done.unchanged
            && args.commands.iter().any(|c| {
                c.get("command")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(command_breaks)
            });
    }
    command_breaks(command.name())
        || matches!(reply, Reply::Exported(e) if e.files.iter().any(|f| f.replaced))
}

/// 壊す操作が済んだときの短い知らせ（命令の名前。誰が・何を、は書かない）。
fn announce(state: &mut AppState, command: &str) {
    let Some(spec) = command_spec(command) else {
        return;
    };
    let lang = state.lang;
    let title = spec.title.pick(match lang {
        Lang::Ja => yolu_ops::Lang::Ja,
        Lang::En => yolu_ops::Lang::En,
    });
    state.info(
        Source::Ops,
        lang.pick(
            format!("外からの操作: {title}"),
            format!("External command: {title}"),
        ),
    );
}

fn refusal_text(lang: Lang, kind: Refusal) -> String {
    match kind {
        Refusal::Host => lang
            .pick(
                "外からの操作: 宛先がこの PC でない要求を断りました。",
                "External command refused: the request was not addressed to this PC.",
            )
            .into(),
        Refusal::Origin => lang
            .pick(
                "外からの操作: ウェブページからの要求を断りました。",
                "External command refused: the request came from a web page.",
            )
            .into(),
        Refusal::Busy => lang
            .pick(
                "外からの操作: つながりの数が上限のため断りました。",
                "External command client refused: too many connections.",
            )
            .into(),
    }
}
