//! 外からの操作（CLI・MCP のクライアントなど、同じ PC の同じユーザーのプログラムからの命令）を受ける、起動中のアプリの側。
//!
//! - 設定「外からの操作を受ける」（既定は切）が入っている間だけ、`yolu-protocol` の手元の経路（Unix のソケット・Windows の名前付きパイプ・鍵のファイルの
//!   確かめ合い）で待ち受ける。名前は Live Link とは別（`yolu_ops::link::LINK_NAME` = `yolupainter-ops`。環境変数 `YOLUPAINTER_OPS_NAME` で替えられる）で、
//!   ソケット・パイプ・鍵のファイルも別。Live Link の鍵ではつなげない。切ると待ち受けをやめ、つながりを閉じる（`Bye` の枠を送ってから閉じる）。
//!   アプリが 2 つ起きているときは、先に待ち受けた方だけが受ける（名前を取れなかった方は理由を出して待ち受けない。Live Link と同じ）。
//! - 挨拶（版の取り決めと鍵）のあとは、`yolu_ops::link` の要求の枠（種類 0x4f50）を読み、返事の枠（0x4f51）を書く。つなげるのは同時に
//!   `MAX_SESSIONS` まで（CLI の 1 回きりのつなぎと、MCP の居続けるつなぎが重なってよい）。
//! - 受けた要求は**画面のスレッド**で実行する（`poll`。1 フレームに `REQUESTS_PER_FRAME` まで）。実行は `AppHost`（`ops_host`）が今のプロジェクトに当て、
//!   1 命令 = 画面の取り消しの 1 段。読みのスレッドは列（`QUEUE` 件）が満杯なら読まない（つなぎ側が詰まるだけで、画面のスレッドは止まらない）。
//!   保存は裏のスレッドで動かし、返事は保存が終わったフレームで返す（返事待ちの間も画面は動く）。
//! - 知らせは出しすぎない: 命令ごとのログは出さない。壊す操作（消す・保存・置き換える書き出し）が済んだときだけ短く知らせる。
//!   待ち受けられない・断ったクライアントがいるときも 1 度だけ。
//! - 切ったあとに、まだ列に残っていた要求は実行しない（つながりを閉じる `Bye` を送ってあるので、返事も返さない）。
//!
//! スレッド: 待ち受け（来たつながりを受けるだけ。止める合図を 50 ms ごとに見る）、つながりごとの挨拶と読み、つながりごとの書き。書くスレッドが
//! 閉じるとき `cancel_reads` で読みを終わらせる（Windows の名前付きパイプは読みの時間切れが無い）。

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use yolu_ops::error::{ErrorCode, OpError};
use yolu_ops::link::{
    decode_request, encode_response, read_frame, Received as FrameReceived, Request, Response,
};
use yolu_ops::meta::{command_spec, Danger};
use yolu_ops::reply::Reply;
use yolu_ops::Command;
use yolu_protocol::frame::Frame;
use yolu_protocol::link::{self, accept_as, LinkError};
use yolu_protocol::{
    AppVersion, Connection, Hello, Identity, Kind, Message, Reject, RejectCode, ServerKey,
};

use crate::lang::Lang;
use crate::ops_host::{AppHost, StartedSave};
use crate::state::AppState;

/// 同時につなげるクライアントの数。
pub const MAX_SESSIONS: usize = 8;
/// 画面のスレッドが 1 フレームで実行する要求の数（残りは次のフレーム。長い命令が続いても、画面が止まり続けない）。
pub const REQUESTS_PER_FRAME: usize = 8;
/// 読みのスレッドが画面のスレッドへ渡す知らせの列の長さ（満杯なら読まない）。
const QUEUE: usize = 64;
/// 読みのスレッドが止める合図を見回る間隔（Unix。Windows は `cancel_reads`）。
const READ_TICK: Duration = Duration::from_millis(100);

const BUSY_TEXT: &str = "外からの操作を受けるつながりの数が上限です。";

/// 待ち受けの様子。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum OpsStatus {
    /// 受けていない（設定が切）。
    #[default]
    Off,
    /// 待ち受けている。
    Listening,
    /// つながっているクライアントの数。
    Connected(usize),
    /// 待ち受けられない（理由。ほかの YoluPainter が同じ名前で受けている など）。
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

    /// 印のツールチップ（受けていなければ空）。
    pub fn tooltip(&self, lang: Lang) -> String {
        match &self.status {
            OpsStatus::Off => String::new(),
            OpsStatus::Listening => lang
                .pick("外からの操作を待っています", "Waiting for external commands")
                .into(),
            OpsStatus::Connected(n) => lang.pick(
                format!("外からの操作を受けています（つながり {n}）"),
                format!("Accepting external commands ({n} connected)"),
            ),
            OpsStatus::Failed(reason) => lang.pick(
                format!("外からの操作を受けられません: {reason}"),
                format!("Cannot accept external commands: {reason}"),
            ),
        }
    }
}

/// 裏のスレッドからの知らせ。
enum Event {
    Connected {
        session: u64,
        agent: String,
        out: Sender<Out>,
    },
    /// 挨拶で断ったつながり。
    Refused(Refusal),
    Request {
        session: u64,
        request: Request,
    },
    /// つながりが終わった。
    Closed {
        session: u64,
    },
}

/// 挨拶で断った理由の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    /// プロトコルの版が重ならない。
    Version,
    /// つながりの数が上限。
    Busy,
    /// 鍵が無い・合わない。
    Unauthorized,
    /// 挨拶が来ない・壊れている。
    Other,
}

/// 書くスレッドへ渡すもの。
enum Out {
    /// 返事の枠。
    Frame(Vec<u8>),
    /// `Bye` を送って閉じる。
    Bye,
}

struct Listening {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Session {
    #[allow(dead_code)]
    agent: String,
    out: Sender<Out>,
}

/// 裏で動いている保存の返事待ち（保存が終わったフレームで返す）。
struct PendingSave {
    session: u64,
    id: u64,
    command: &'static str,
    started: StartedSave,
}

/// 外からの操作を受ける（`YoluApp` が 1 つ持つ）。
pub struct OpsLink {
    name: String,
    tx: SyncSender<Event>,
    rx: Receiver<Event>,
    listening: Option<Listening>,
    /// 待ち受けを始めようとした（設定が入っている間、失敗しても毎フレーム試さない。設定を切ると戻る）。
    tried: bool,
    /// 待ち受けられなかった理由。
    failure: Option<String>,
    sessions: BTreeMap<u64, Session>,
    session_counter: Arc<AtomicU64>,
    /// つながっている数（待ち受けのスレッドが上限を見る）。
    active: Arc<AtomicUsize>,
    pending: Option<PendingSave>,
    /// 最後に知らせた断りの種類（同じ断りが続いても、知らせは 1 度）。
    last_refusal: Option<Refusal>,
    ctx: Option<egui::Context>,
    /// これまでに実行した要求の数（試験・診断用）。
    handled: u64,
}

impl Default for OpsLink {
    fn default() -> Self {
        OpsLink::new()
    }
}

impl Drop for OpsLink {
    fn drop(&mut self) {
        self.close_sessions();
    }
}

/// 挨拶で名乗る名前（Live Link と同じ名乗りに、通信の種類を足す）。
fn identity() -> Identity {
    Identity::standalone(&format!("{} ops", crate::livelink::AGENT))
        .with_version(AppVersion::parse(env!("CARGO_PKG_VERSION")))
}

impl OpsLink {
    pub fn new() -> OpsLink {
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        let name = std::env::var(yolu_ops::link::LINK_NAME_ENV)
            .ok()
            .filter(|n| link::valid_link_name(n))
            .unwrap_or_else(|| yolu_ops::link::LINK_NAME.to_owned());
        OpsLink {
            name,
            tx,
            rx,
            listening: None,
            tried: false,
            failure: None,
            sessions: BTreeMap::new(),
            session_counter: Arc::new(AtomicU64::new(0)),
            active: Arc::new(AtomicUsize::new(0)),
            pending: None,
            last_refusal: None,
            ctx: None,
            handled: 0,
        }
    }

    /// つなぎ先の名前（待ち受けていないときだけ替えられる。試験で重ならない名前にする）。
    pub fn set_name(&mut self, name: &str) -> Result<(), String> {
        if self.listening.is_some() {
            return Err("待ち受けている間は名前を替えません。".into());
        }
        if !link::valid_link_name(name) {
            return Err("つなぎ先の名前は 1〜64 文字の英数字と . _ - です。".into());
        }
        self.name = name.to_owned();
        Ok(())
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// 待ち受けの様子。
    pub fn status(&self) -> OpsStatus {
        if let Some(reason) = &self.failure {
            return OpsStatus::Failed(reason.clone());
        }
        match (&self.listening, self.sessions.len()) {
            (None, _) => OpsStatus::Off,
            (Some(_), 0) => OpsStatus::Listening,
            (Some(_), n) => OpsStatus::Connected(n),
        }
    }

    /// 画面に写す様子。
    pub fn view(&self) -> OpsView {
        OpsView { status: self.status() }
    }

    /// これまでに実行した要求の数（断った・誤りにした要求も数える。試験・診断用）。
    pub fn handled(&self) -> u64 {
        self.handled
    }

    /// 裏で動いている保存の返事を待っているか（試験・診断用）。
    pub fn save_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// 設定（`want`）に合わせて、待ち受けを始める・やめる。始めるのは、入ったあとに 1 度だけ（失敗しても毎フレームは試さない）。
    pub fn sync(&mut self, want: bool, ctx: &egui::Context, state: &mut AppState) {
        if want {
            if self.listening.is_none() && !self.tried {
                self.start(ctx, state);
            }
        } else if self.listening.is_some() || self.tried || self.failure.is_some() {
            self.stop();
        }
    }

    /// 待ち受けを始める。名前を取れなければ、理由を出して待ち受けない。
    pub fn start(&mut self, ctx: &egui::Context, state: &mut AppState) {
        if self.listening.is_some() {
            return;
        }
        self.tried = true;
        self.failure = None;
        let listener = match link::Server::bind(&self.name, true) {
            Ok(l) => l,
            Err(e) => {
                let lang = state.lang;
                let reason = if e.kind() == std::io::ErrorKind::AddrInUse {
                    lang.pick(
                        "ほかの YoluPainter がすでに受けています",
                        "Another YoluPainter is already accepting them",
                    )
                    .to_owned()
                } else {
                    e.to_string()
                };
                state.message = OpsView { status: OpsStatus::Failed(reason.clone()) }.tooltip(lang);
                self.failure = Some(reason);
                return;
            }
        };
        self.ctx = Some(ctx.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            let counter = self.session_counter.clone();
            let active = self.active.clone();
            let key = listener.key();
            thread::Builder::new()
                .name("yolu-ops-listen".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok(stream) => {
                                let session = counter.fetch_add(1, Ordering::Relaxed) + 1;
                                let (tx, ctx, stop) = (tx.clone(), ctx.clone(), stop.clone());
                                let (key, active) = (key.clone(), active.clone());
                                let _ = thread::Builder::new()
                                    .name(format!("yolu-ops-{session}"))
                                    .spawn(move || serve(stream, session, tx, ctx, stop, key, active));
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(50))
                            }
                            Err(_) => thread::sleep(Duration::from_millis(200)),
                        }
                    }
                })
                .expect("スレッドを作れる")
        };
        self.listening = Some(Listening { stop, thread: Some(thread) });
    }

    /// 待ち受けをやめ、つながりを閉じる（`Bye` を送る）。
    pub fn stop(&mut self) {
        self.listening = None; // 落とすと待ち受けのスレッドが止まり、鍵のファイルを消す
        self.close_sessions();
        self.tried = false;
        self.failure = None;
        self.last_refusal = None;
    }

    /// つながっているクライアントへ `Bye` を送って閉じる。
    fn close_sessions(&mut self) {
        for (_, session) in std::mem::take(&mut self.sessions) {
            let _ = session.out.send(Out::Bye);
        }
    }

    /// 裏のスレッドからの知らせを読み、受けた要求を実行する（フレームの頭で）。
    pub fn poll(&mut self, state: &mut AppState) {
        self.finish_pending_save(state);
        let mut ran = 0;
        loop {
            if ran >= REQUESTS_PER_FRAME {
                // 残りは次のフレーム（長い命令が続いても、画面が止まり続けない）
                if let Some(ctx) = &self.ctx {
                    ctx.request_repaint();
                }
                break;
            }
            let Ok(event) = self.rx.try_recv() else { break };
            match event {
                Event::Connected { session, agent, out } => {
                    if self.listening.is_none() {
                        // やめた後に来た
                        let _ = out.send(Out::Bye);
                        continue;
                    }
                    self.sessions.insert(session, Session { agent, out });
                    self.last_refusal = None;
                }
                Event::Refused(kind) => {
                    if self.last_refusal != Some(kind) {
                        self.last_refusal = Some(kind);
                        state.message = refusal_text(state.lang, kind);
                    }
                }
                Event::Request { session, request } => {
                    ran += 1;
                    self.handle(session, request, state);
                }
                Event::Closed { session } => {
                    self.sessions.remove(&session);
                }
            }
        }
        self.finish_pending_save(state);
    }

    fn respond(&self, session: u64, response: &Response) {
        if let Some(s) = self.sessions.get(&session) {
            let _ = s.out.send(Out::Frame(encode_response(response)));
        }
    }

    /// 要求を画面のスレッドで実行して返す。保存は裏で始めて、返事は保存が終わってから。
    fn handle(&mut self, session: u64, request: Request, state: &mut AppState) {
        // 閉じた後に残っていた要求。`stop` は待ち受けをやめると同時に `sessions` を空にし、やめたあとに届いたつながりは入れない
        // （`poll` の `Connected`）ので、受けていない間にここを通る要求は無い。返事も返さない（相手には `Bye` を送ってある）
        if !self.sessions.contains_key(&session) {
            return;
        }
        self.handled += 1;
        // 前の保存の結果が出ていれば、先に返す（次の保存の頼みが、前の結果を消さない）
        self.finish_pending_save(state);
        let Request { id, command } = request;
        let (result, started) = {
            let mut host = AppHost::new(state);
            // `execute` は途中の panic を `internal` の誤りにして返す（文書の編集は積んだ段を戻してから）。受け止めて動き続ける panic なので、落ちた記録にしない
            let result = crate::crash::handled(std::panic::AssertUnwindSafe(|| yolu_ops::execute(&mut host, &command)))
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
            self.pending = Some(PendingSave { session, id, command: command.name(), started });
            self.finish_pending_save(state);
            return;
        }
        match result {
            Ok(reply) => {
                if breaks_something(&command, &reply) {
                    announce(state, command.name());
                }
                self.respond(session, &Response::ok(id, reply));
            }
            Err(error) => self.respond(session, &Response::err(id, error)),
        }
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
                    Response::ok(pending.id, AppHost::saved_reply(state, &pending.started, facts))
                }
                Err(text) => Response::err(
                    pending.id,
                    OpError::new(ErrorCode::Io, text.clone(), text)
                        .with_data(serde_json::json!({"path": outcome.path.display().to_string()})),
                ),
            },
            None => Response::err(
                pending.id,
                OpError::new(
                    ErrorCode::Internal,
                    "保存の結果を受け取れませんでした",
                    "The result of the save could not be received",
                ),
            ),
        };
        self.respond(pending.session, &response);
    }
}

/// 壊す印のある命令か（`Danger::Always`）。
fn command_breaks(name: &str) -> bool {
    command_spec(name).is_some_and(|s| s.danger == Danger::Always)
}

/// 済んだ命令が、壊す操作だったか: 消す・上書き保存（いつも壊す印。ただし何も書かなかったものは除く）、置き換えたファイルのある書き出し。
fn breaks_something(command: &Command, reply: &Reply) -> bool {
    // 何も書かなかった上書き保存（編集が無い）は、壊す操作が済んだことにならない
    if matches!(reply, Reply::Saved(s) if !s.written) {
        return false;
    }
    command_breaks(command.name())
        || matches!(reply, Reply::Exported(e) if e.files.iter().any(|f| f.replaced))
}

/// 壊す操作が済んだときの短い知らせ（命令の名前。誰が・何を、は書かない）。
fn announce(state: &mut AppState, command: &str) {
    let Some(spec) = command_spec(command) else { return };
    let lang = state.lang;
    let title = spec.title.pick(match lang {
        Lang::Ja => yolu_ops::Lang::Ja,
        Lang::En => yolu_ops::Lang::En,
    });
    state.message = lang.pick(format!("外からの操作: {title}"), format!("External command: {title}"));
}

fn refusal_text(lang: Lang, kind: Refusal) -> String {
    match kind {
        Refusal::Version => lang
            .pick(
                "外からの操作: 版の合わないクライアントを断りました。",
                "External command client refused: versions do not match.",
            )
            .into(),
        Refusal::Busy => lang
            .pick(
                "外からの操作: つながりの数が上限のため断りました。",
                "External command client refused: too many connections.",
            )
            .into(),
        Refusal::Unauthorized => lang
            .pick(
                "外からの操作: 鍵の合わないクライアントを断りました。",
                "External command client refused: wrong key.",
            )
            .into(),
        Refusal::Other => lang
            .pick(
                "外からの操作: 挨拶できないクライアントを断りました。",
                "External command client refused: no valid greeting.",
            )
            .into(),
    }
}

/// つながりの数を、スレッドが終わるときに戻す。
struct Slot {
    active: Arc<AtomicUsize>,
    held: Cell<bool>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if self.held.get() {
            self.active.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// 要求の枠の `id`（読めなければ 0。読めない要求にも、返事を結べる番号で断る）。
fn request_id(frame: &Frame) -> u64 {
    serde_json::from_slice::<serde_json::Value>(&frame.payload)
        .ok()
        .and_then(|v| v.get("id").and_then(serde_json::Value::as_u64))
        .unwrap_or(0)
}

/// つながり 1 つ: 挨拶（版の取り決めと鍵。つなげる数に上限）、書くスレッドを立て、このスレッドで要求を読み続ける。
fn serve(
    stream: interprocess::local_socket::Stream,
    session: u64,
    tx: SyncSender<Event>,
    ctx: egui::Context,
    stop: Arc<AtomicBool>,
    key: Arc<ServerKey>,
    active: Arc<AtomicUsize>,
) {
    let wake = |event: Event| {
        let sent = tx.send(event).is_ok();
        ctx.request_repaint();
        sent
    };
    let slot = Slot { active: active.clone(), held: Cell::new(false) };
    // 数は、挨拶（鍵と版）が済んでから取る。挨拶を送らない接続や鍵の合わない接続が枠を塞がない
    let claim = |_: &Hello| -> Result<(), Reject> {
        if active.fetch_add(1, Ordering::AcqRel) >= MAX_SESSIONS {
            active.fetch_sub(1, Ordering::AcqRel);
            return Err(Reject::plain(RejectCode::Busy, BUSY_TEXT));
        }
        slot.held.set(true);
        Ok(())
    };
    let (conn, mut reader, hello) =
        match accept_as(stream, &identity(), session, &key, link::HANDSHAKE_TIMEOUT, &claim) {
            Ok(x) => x,
            Err(LinkError::Rejected(r)) => {
                let kind = match r.code {
                    RejectCode::Busy => Refusal::Busy,
                    RejectCode::Unauthorized => Refusal::Unauthorized,
                    RejectCode::VersionMismatch => Refusal::Version,
                    _ => Refusal::Other,
                };
                wake(Event::Refused(kind));
                return;
            }
            Err(_) => {
                wake(Event::Refused(Refusal::Other));
                return;
            }
        };
    let (out_tx, out_rx) = mpsc::channel::<Out>();
    {
        let writer = conn.clone();
        let spawned = thread::Builder::new()
            .name(format!("yolu-ops-{session}-write"))
            .spawn(move || write_loop(writer, out_rx));
        if spawned.is_err() {
            return;
        }
    }
    if !wake(Event::Connected { session, agent: hello.agent.clone(), out: out_tx.clone() }) {
        return;
    }
    reader.set_timeout(Some(READ_TICK));
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let (frames, stream) = reader.raw();
        let mut stream = stream;
        match read_frame(frames, &mut stream) {
            Ok(FrameReceived::Frame(frame)) => {
                if frame.kind == Kind::Bye as u16 {
                    break; // 相手が礼儀正しく抜けた
                }
                match decode_request(&frame) {
                    Ok(request) => {
                        // 満杯なら、画面のスレッドが取り出すまで待つ（つなぎ側が詰まるだけ）
                        if !wake(Event::Request { session, request }) {
                            break;
                        }
                    }
                    Err(error) => {
                        // 読めない要求は、画面のスレッドを通さず、ここで断る
                        let response = Response::err(request_id(&frame), error);
                        let _ = out_tx.send(Out::Frame(encode_response(&response)));
                    }
                }
            }
            Ok(FrameReceived::Idle) => {}
            Ok(FrameReceived::Closed) | Err(_) => break,
        }
    }
    let _ = wake(Event::Closed { session });
    // 書くスレッドは、送り手（このスレッドと画面のスレッドの控え）がなくなれば終わる
    drop(out_tx);
}

fn write_loop(conn: Connection, rx: Receiver<Out>) {
    while let Ok(out) = rx.recv() {
        match out {
            Out::Frame(bytes) => {
                if conn.send_frame(&bytes).is_err() {
                    break;
                }
            }
            Out::Bye => {
                let _ = conn.send(&Message::Bye);
                break;
            }
        }
    }
    // 読みのスレッドが（時間切れの無い Windows でも）終われるように、待っている読みを終わらせる
    conn.cancel_reads();
}
