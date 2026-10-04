//! Live Link のスタンドアロンの側（yolu-link-demo の作りを画面に載せたもの）。Unity のエディタの YoluPainter（Live Link の受け口）が
//! つなぎ、シーンのモデルを送ってくる。描いたテクスチャは共有メモリで返し、Unity が本物のマテリアルで見せる。
//!
//! - ファイル ▸ Live Link で待ち受ける（名前は既定で `yolupainter-livelink`、環境変数 `YOLUPAINTER_LINK_NAME` で替えられる）。Unity の
//!   ブリッジが挨拶すると、読める版を取り決めて返す。重ならなければ断り、状態の帯に版の不一致を出す。つなげる Unity は 1 つで、
//!   2 つ目は Busy で断る。
//! - モデル（Model）を受けたら、マテリアルごとにテクスチャセットを結び付け・作り（`sets`）、目を開いていて Unity が Color を見せられる
//!   （流し込み先のある）マテリアルのセットを共有メモリに出して知らせる。描いたら、変わったタイルだけを合成して共有メモリへ書き、
//!   知らせる（チャンネルは今は Color だけ。core の M1 が Color だけなので）。
//! - ポーズ（Pose）・マテリアルの更新（Materials）・モデルを閉じた（ModelClosed）は `AppState.model` に当てる（3D ビューが読む）。
//!
//! スレッド: 待ち受け（来たつながりを受けるだけ。止める合図を 50 ms ごとに見る）と、つながりごとの挨拶と読み・書き。画面のスレッドは
//! フレームの頭で知らせを読み（`poll`）、フレームの終わりに変わったタイルを出す（`publish`）。画面のスレッドはパイプに書かない（書く
//! スレッドへ渡すだけ）ので、Unity が遅くても描く手は止まらない。共有メモリへの書き込みも待たない（タイルごとの seqlock）。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use yolu_protocol::host::PublishedSet;
use yolu_protocol::link::{
    self, accept_with, error_message, negotiate, wrong_direction, LinkError,
};
use yolu_protocol::{
    channel, shm::valid_tile_size, Connection, ErrorCode, Hello, Message, Received, Reject,
    RejectCode, ServerKey, Tile, DEFAULT_LINK_NAME, MAX_TEXTURE_SIZE,
};

use crate::engine::{Channel, Document, RowOrder, TileCoord};
use crate::model::{ModelSource, SceneModel};
use crate::state::AppState;
use crate::lang::Lang;
use crate::view3d::model::ViewError;

/// 挨拶で名乗る名前。
pub const AGENT: &str = concat!("YoluPainter ", env!("CARGO_PKG_VERSION"));

/// 2 つ目の Unity を断る理由。
const BUSY_TEXT: &str =
    "スタンドアロンの YoluPainter はほかの Unity とつながっています（つなげるのは 1 つ）。";

/// つながりの様子。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LinkStatus {
    #[default]
    Off,
    Listening,
    Connected {
        agent: String,
        version: u16,
        session: u64,
    },
    /// 待ち受けられない（同じ名前で別のスタンドアロンが待ち受けている など）。
    Failed(String),
}

/// 知らせの重さ（状態の帯の色）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// 画面が読む Live Link の様子の写し。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkView {
    pub name: String,
    pub status: LinkStatus,
    /// 最後の知らせ（つながった・切れた・断った・版の不一致など）。
    pub notice: Option<(NoticeLevel, String)>,
    /// Unity に出しているテクスチャセット（uid）。
    pub published: Vec<u32>,
    /// これまでに知らせたタイルの数（作り直しの全部を含む）。
    pub tiles_sent: u64,
    /// 最後に版が合わずに断った理由（つながる・やめるまで出す）。
    pub mismatch: Option<String>,
}

impl Default for LinkView {
    fn default() -> Self {
        LinkView {
            name: DEFAULT_LINK_NAME.into(),
            status: LinkStatus::Off,
            notice: None,
            published: Vec::new(),
            tiles_sent: 0,
            mismatch: None,
        }
    }
}

impl LinkView {
    /// 待ち受けているか、つながっている。
    pub fn is_on(&self) -> bool {
        matches!(
            self.status,
            LinkStatus::Listening | LinkStatus::Connected { .. }
        )
    }

    /// 入口のアイコンのツールチップに出す短い文。
    pub fn summary(&self) -> String {
        self.summary_in(Lang::Ja)
    }

    pub fn summary_in(&self, lang: Lang) -> String {
        match &self.status {
            LinkStatus::Off => lang.pick("Live Link: 切っています", "Live Link: Off").into(),
            LinkStatus::Listening if self.mismatch.is_some() => lang
                .pick(
                    "Live Link: 版の合わない Unity を断りました",
                    "Live Link: Version mismatch",
                )
                .into(),
            LinkStatus::Listening => lang.pick(
                format!("Live Link: Unity を待っています（{}）", self.name),
                format!("Live Link: Waiting for Unity ({})", self.name),
            ),
            LinkStatus::Connected { version, .. } => lang.pick(
                format!(
                    "Live Link: Unity とつながっています（版 {version}・セット {}）",
                    self.published.len()
                ),
                format!(
                    "Live Link: Connected (v{version} · {} sets)",
                    self.published.len()
                ),
            ),
            LinkStatus::Failed(_) => lang
                .pick("Live Link: 待ち受けられません", "Live Link: Unavailable")
                .into(),
        }
    }

    /// 窓の先頭に出す状態の名前（名前だけ）。
    pub fn state_label(&self, lang: Lang) -> &'static str {
        match &self.status {
            LinkStatus::Off => lang.pick("切断", "Off"),
            LinkStatus::Listening if self.mismatch.is_some() => {
                lang.pick("版が合いません", "Version mismatch")
            }
            LinkStatus::Listening => lang.pick("待機中", "Waiting"),
            LinkStatus::Connected { .. } => lang.pick("接続中", "Connected"),
            LinkStatus::Failed(_) => lang.pick("待ち受けられません", "Unavailable"),
        }
    }

    /// 入口のアイコンの印の様子。
    pub fn indicator(&self) -> LinkIndicator {
        match &self.status {
            LinkStatus::Off => LinkIndicator::Off,
            LinkStatus::Failed(_) => LinkIndicator::Failed,
            LinkStatus::Listening if self.mismatch.is_some() => LinkIndicator::Mismatch,
            LinkStatus::Listening => LinkIndicator::Waiting,
            LinkStatus::Connected { .. } => match self.notice {
                Some((NoticeLevel::Error, _)) => LinkIndicator::Mismatch,
                _ => LinkIndicator::Connected,
            },
        }
    }

    /// つながっている Unity の名前（挨拶の名乗りから。「(Unity 2022.3.22f1)」の形なら版の名前だけ）。つながっていなければ None。
    pub fn unity_name(&self) -> Option<String> {
        let LinkStatus::Connected { agent, .. } = &self.status else {
            return None;
        };
        let version = agent
            .find("(Unity ")
            .and_then(|at| {
                let rest = &agent[at + 1..];
                rest.find(')').map(|end| rest[..end].to_owned())
            })
            .filter(|name| !name.trim().is_empty());
        Some(version.unwrap_or_else(|| agent.clone()))
    }

    /// アイコンのツールチップ: 状態の文と、理由があれば（待ち受けられない・版が合わない・最後の知らせが誤り）その理由。
    pub fn tooltip(&self, lang: Lang) -> String {
        let summary = self.summary_in(lang);
        match (&self.status, &self.mismatch, &self.notice) {
            (LinkStatus::Failed(e), _, _) => format!("{summary}\n{e}"),
            (_, Some(m), _) => format!("{summary}\n{m}"),
            (_, _, Some((NoticeLevel::Error, n))) => format!("{summary}\n{n}"),
            _ => summary,
        }
    }
}

/// 入口のアイコンの印の様子（切断は灰・待機中は薄い色・接続は緑・版の不一致は警告の色・待ち受けられないは赤）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkIndicator {
    Off,
    Waiting,
    Connected,
    Mismatch,
    Failed,
}

/// 始める・やめるの頼み（メニューから。`YoluApp` が当てる）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkRequest {
    Start,
    Stop,
}

/// 裏のスレッドからの知らせ。
enum Event {
    Connected {
        session: u64,
        hello: Hello,
        version: u16,
        out: Sender<Out>,
    },
    /// 版が合わないので断った。
    Refused {
        text: String,
    },
    /// ほかの Unity とつながっているので断った。
    Busy {
        agent: String,
    },
    /// 鍵が無い・合わない挨拶を断った（古いブリッジ・別の鍵・別のユーザーのつなぎ）。
    Unauthorized {
        text: String,
    },
    HandshakeFailed(String),
    Message {
        session: u64,
        message: Message,
    },
    Unknown {
        session: u64,
        kind: u16,
    },
    Malformed {
        session: u64,
        kind: u16,
        text: String,
    },
    /// つながりが終わった（None は Unity の Bye）。
    Closed {
        session: u64,
        reason: Option<String>,
    },
}

/// 書くスレッドへ渡すもの。
enum Out {
    Message(Message),
    /// Bye を送って閉じる。
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

struct Active {
    session: u64,
    out: Sender<Out>,
}

impl Active {
    fn send(&self, message: Message) {
        let _ = self.out.send(Out::Message(message));
    }
}

/// Unity に出しているテクスチャセット 1 つ。
struct Published {
    set: PublishedSet,
    /// 書いた文書（開き直しで替われば全部を書き直す）。
    doc_id: u128,
    /// 最後に書いた後の文書の変化の通し番号。
    since: u64,
}

/// Live Link（`YoluApp` が 1 つ持つ）。
pub struct LiveLink {
    name: String,
    status: LinkStatus,
    notice: Option<(NoticeLevel, String)>,
    mismatch: Option<String>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    listening: Option<Listening>,
    /// つながっている Unity のつながりの番号（0 は無し）。待ち受けのスレッドが 2 つ目を断るのに使う。
    active_session: Arc<AtomicU64>,
    sessions: Arc<AtomicU64>,
    active: Option<Active>,
    published: BTreeMap<u32, Published>,
    /// 共有メモリを作れなかったセット（次のモデルまで作り直さない。毎フレーム試さない）。
    failed: BTreeSet<u32>,
    failed_for_model: u64,
    tiles_sent: u64,
}

impl Default for LiveLink {
    fn default() -> Self {
        LiveLink::new()
    }
}

impl Drop for LiveLink {
    fn drop(&mut self) {
        if let Some(a) = self.active.take() {
            let _ = a.out.send(Out::Bye);
        }
    }
}

impl LiveLink {
    pub fn new() -> LiveLink {
        let (tx, rx) = mpsc::channel();
        let name = std::env::var("YOLUPAINTER_LINK_NAME")
            .ok()
            .filter(|n| link::valid_link_name(n))
            .unwrap_or_else(|| DEFAULT_LINK_NAME.to_owned());
        LiveLink {
            name,
            status: LinkStatus::Off,
            notice: None,
            mismatch: None,
            tx,
            rx,
            listening: None,
            active_session: Arc::new(AtomicU64::new(0)),
            sessions: Arc::new(AtomicU64::new(0)),
            active: None,
            published: BTreeMap::new(),
            failed: BTreeSet::new(),
            failed_for_model: 0,
            tiles_sent: 0,
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

    pub fn status(&self) -> &LinkStatus {
        &self.status
    }

    /// 画面に写す様子。
    pub fn view(&self) -> LinkView {
        LinkView {
            name: self.name.clone(),
            status: self.status.clone(),
            notice: self.notice.clone(),
            published: self.published.keys().copied().collect(),
            tiles_sent: self.tiles_sent,
            mismatch: self.mismatch.clone(),
        }
    }

    fn notify(&mut self, level: NoticeLevel, text: String, state: &mut AppState) {
        state.message = text.clone();
        self.notice = Some((level, text));
    }

    /// 頼みを当てる。
    pub fn request(&mut self, request: LinkRequest, ctx: &egui::Context, state: &mut AppState) {
        match request {
            LinkRequest::Start => self.start(ctx, state),
            LinkRequest::Stop => self.stop(state),
        }
    }

    /// 待ち受けを始める。
    pub fn start(&mut self, ctx: &egui::Context, state: &mut AppState) {
        if self.listening.is_some() {
            return;
        }
        let listener = match link::Server::bind(&self.name, true) {
            Ok(l) => l,
            Err(e) => {
                let text = state.lang.pick(format!("Live Link を「{}」で待ち受けられません: {e}", self.name), format!("Live Link unavailable at “{}”: {e}", self.name));
                self.status = LinkStatus::Failed(text.clone());
                self.notify(NoticeLevel::Error, text, state);
                return;
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            let active = self.active_session.clone();
            let sessions = self.sessions.clone();
            let key = listener.key();
            thread::Builder::new()
                .name("yolu-livelink-listen".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok(stream) => {
                                let session = sessions.fetch_add(1, Ordering::Relaxed) + 1;
                                let (tx, ctx, active) = (tx.clone(), ctx.clone(), active.clone());
                                let key = key.clone();
                                let _ = thread::Builder::new()
                                    .name(format!("yolu-livelink-{session}"))
                                    .spawn(move || serve(stream, session, tx, ctx, active, key));
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
        self.listening = Some(Listening {
            stop,
            thread: Some(thread),
        });
        self.status = LinkStatus::Listening;
        let text = state.lang.pick(format!("Live Link: Unity を待っています（{}）。", self.name), format!("Live Link: Waiting for Unity ({}).", self.name));
        self.notify(NoticeLevel::Info, text, state);
    }

    /// 待ち受けをやめ、つながっていれば切る（Bye を送る）。出したテクスチャセットの共有メモリは片付ける。
    pub fn stop(&mut self, state: &mut AppState) {
        self.listening = None; // 落とすと待ち受けのスレッドが止まる
        let was_connected = self.active.is_some();
        self.disconnect(state);
        self.status = LinkStatus::Off;
        self.mismatch = None;
        let text = if was_connected {
            state.lang.pick("Live Link を切りました。", "Live Link disconnected.")
        } else {
            state.lang.pick("Live Link の待ち受けをやめました。", "Live Link stopped.")
        };
        self.notify(NoticeLevel::Info, text.into(), state);
    }

    fn disconnect(&mut self, state: &mut AppState) {
        if let Some(a) = self.active.take() {
            let _ = a.out.send(Out::Bye);
            let _ = self.active_session.compare_exchange(
                a.session,
                0,
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
            if let Some(m) = state.model.as_mut() {
                if m.source == (ModelSource::LiveLink { session: a.session }) {
                    m.live = false;
                }
            }
        }
        self.published.clear();
        self.failed.clear();
    }

    fn current_session(&self) -> Option<u64> {
        self.active.as_ref().map(|a| a.session)
    }

    /// 裏のスレッドからの知らせを読み、状態に当てる（フレームの頭で）。
    pub fn poll(&mut self, state: &mut AppState) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Connected {
                    session,
                    hello,
                    version,
                    out,
                } => {
                    if self.listening.is_none() || self.active.is_some() {
                        // やめた後・つながっている間に来た（待ち受けのスレッドは 2 つ目を断るので、普通は来ない）
                        let _ = out.send(Out::Bye);
                        continue;
                    }
                    self.active = Some(Active { session, out });
                    self.status = LinkStatus::Connected {
                        agent: hello.agent.clone(),
                        version,
                        session,
                    };
                    self.mismatch = None;
                    let text = state.lang.pick(format!(
                        "Live Link: Unity とつながりました（{}・プロトコルの版 {version}）。",
                        hello.agent
                    ), format!(
                        "Live Link: Connected ({} · protocol v{version}).",
                        hello.agent
                    ));
                    self.notify(NoticeLevel::Info, text, state);
                }
                Event::Refused { text } => {
                    // 理由の全文（版の範囲）は状態の帯のツールチップに出す
                    self.mismatch = Some(text);
                    self.notify(
                        NoticeLevel::Error,
                        state.lang.pick("Live Link: 版の合わない Unity を断りました。", "Live Link: Version mismatch.").into(),
                        state,
                    );
                }
                Event::Busy { agent } => {
                    let text = state.lang.pick(format!("Live Link: 2 つ目の Unity（{agent}）を断りました。"), format!("Live Link: Second Unity connection refused ({agent})."));
                    self.notify(NoticeLevel::Warning, text, state);
                }
                Event::Unauthorized { text } => {
                    // 理由は鍵の断りの文（古いブリッジ・別の鍵など）。つながっている Unity には影響しない
                    let text = state.lang.pick(
                        format!("Live Link: 鍵の合わない接続を断りました（{text}）。"),
                        format!("Live Link: Connection refused, key mismatch ({text})."),
                    );
                    self.notify(NoticeLevel::Warning, text, state);
                }
                Event::HandshakeFailed(e) => {
                    let text = state.lang.pick(format!("Live Link: つなぎ始めで失敗しました: {e}"), format!("Live Link: Handshake failed: {e}"));
                    self.notify(NoticeLevel::Warning, text, state);
                }
                Event::Message { session, message } => {
                    if Some(session) == self.current_session() {
                        self.handle(message, state);
                    }
                }
                Event::Unknown { session, kind } => {
                    if Some(session) == self.current_session() {
                        let text = state.lang.pick(format!(
                            "Live Link: Unity からの知らない命令（種類 0x{kind:04x}）を断りました。"
                        ), format!(
                            "Live Link: Unknown Unity command (0x{kind:04x})."
                        ));
                        self.notify(NoticeLevel::Warning, text, state);
                    }
                }
                Event::Malformed {
                    session,
                    kind,
                    text,
                } => {
                    if Some(session) == self.current_session() {
                        let text = state.lang.pick(format!(
                            "Live Link: Unity からの命令（{}）を読めません: {text}",
                            link::kind_name(kind)
                        ), format!(
                            "Live Link: Invalid Unity command ({}): {text}",
                            link::kind_name(kind)
                        ));
                        self.notify(NoticeLevel::Warning, text, state);
                    }
                }
                Event::Closed { session, reason } => {
                    if Some(session) != self.current_session() {
                        continue;
                    }
                    self.disconnect(state);
                    self.status = if self.listening.is_some() {
                        LinkStatus::Listening
                    } else {
                        LinkStatus::Off
                    };
                    let (level, text) = match reason {
                        None => (
                            NoticeLevel::Info,
                            state.lang.pick("Live Link: Unity が切りました。", "Live Link: Unity disconnected.").to_owned(),
                        ),
                        Some(e) => (
                            NoticeLevel::Warning,
                            state.lang.pick(format!("Live Link: Unity とのつながりが切れました: {e}"), format!("Live Link: Connection lost: {e}")),
                        ),
                    };
                    self.notify(level, text, state);
                }
            }
        }
    }

    /// Unity へ返す誤りの返事。表示の言語に依らず、プロトコルの診断として日本語の文に固定する
    /// （画面に出る知らせは `state.lang` で作る。返事を受ける Unity の側が、自分の言語で扱う）。
    fn reply_error(&self, code: ErrorCode, kind: u16, text: String) {
        if let Some(a) = &self.active {
            a.send(error_message(code, kind, text));
        }
    }

    fn handle(&mut self, message: Message, state: &mut AppState) {
        let Some(session) = self.current_session() else {
            return;
        };
        if let Some(reply) = wrong_direction(&message, true) {
            if let Some(a) = &self.active {
                a.send(reply);
            }
            return;
        }
        let ours = |m: &SceneModel| m.source == ModelSource::LiveLink { session };
        match message {
            Message::Model(model) => {
                // 同じつながりの 2 つ目以降のモデル（Unity が送り直した）は、3D ビューを前へ出し直さない
                let first = !state.model.as_ref().is_some_and(ours);
                let (report, shape) = state.receive_link_model(&model, session);
                self.failed.clear();
                let mut text = state.lang.pick(
                    format!("Live Link: モデル「{}」を受けました。", model.name),
                    format!("Live Link: Received the model “{}”.", model.name),
                );
                if shape.is_ok() && first {
                    // 届いたモデルは 3D ビューに出す（キャンバスが前にあれば 3D ビューのタブを前へ）
                    state.view3d.pose.focus = true;
                }
                if let Err(e) = shape {
                    let e = state.lang.view_error(&e);
                    text += &state.lang.pick(format!(" 3D ビューには出せません: {e}。"), format!(" Unavailable in 3D View: {e}."));
                }
                if !report.created.is_empty() {
                    text += &state.lang.pick(format!(" 新しいテクスチャセット: {}。", report.created.join("・")), format!(" New texture sets: {}.", report.created.join(", ")));
                }
                if !report.unmatched.is_empty() {
                    text += &state.lang.pick(format!(" モデルに無いセット: {}。", report.unmatched.join("・")), format!(" Sets not in this model: {}.", report.unmatched.join(", ")));
                }
                self.notify(NoticeLevel::Info, text, state);
            }
            Message::Pose(pose) => {
                // 3D ビューの形に当てる（描いている最中なら、終わってから）。合わないポーズは何も変えずに断る
                let result = if state.model.as_ref().is_some_and(ours) {
                    state.receive_link_pose(&pose)
                } else {
                    Err(ViewError::NoLinkModel)
                };
                if let Err(e) = result {
                    self.reply_error(ErrorCode::Refused, 0x0011, Lang::Ja.view_error(&e));
                }
            }
            Message::Materials(update) => match state.model.as_mut().filter(|m| ours(m)) {
                Some(m) => match m.apply_materials(&update) {
                    Ok(keys_changed) => {
                        if keys_changed {
                            state.bind_model();
                        }
                    }
                    Err(e) => self.reply_error(ErrorCode::Refused, 0x0012, e),
                },
                None => self.reply_error(
                    ErrorCode::Refused,
                    0x0012,
                    "モデルを受ける前のマテリアルの更新は使えません".into(),
                ),
            },
            Message::ModelClosed { generation } => {
                if state.model.as_ref().is_some_and(ours) && state.close_link_model(generation) {
                    self.notify(
                        NoticeLevel::Info,
                        state.lang.pick("Live Link: Unity がモデルを閉じました。", "Live Link: Unity closed the model.").into(),
                        state,
                    );
                }
            }
            Message::Error(e) => {
                let text = state.lang.pick(format!(
                    "Live Link: Unity からの誤りの知らせ（{}）: {}",
                    link::kind_name(e.kind),
                    e.text
                ), format!(
                    "Live Link: Unity error ({}): {}",
                    link::kind_name(e.kind),
                    e.text
                ));
                self.notify(NoticeLevel::Warning, text, state);
            }
            Message::Hello(_) => self.reply_error(
                ErrorCode::UnexpectedCommand,
                0x0001,
                "挨拶はつないだときだけです".into(),
            ),
            // Bye は読むスレッドが Closed にする。向きの違う命令は上で断った
            _ => {}
        }
    }

    /// 変わったタイルを共有メモリに書いて知らせる（フレームの終わりに）。出すセットの増減・世代の変化もここで合わせる。
    pub fn publish(&mut self, state: &mut AppState) {
        let Some(session) = self.current_session() else {
            return;
        };
        let model = state
            .model
            .as_ref()
            .filter(|m| m.live && m.source == ModelSource::LiveLink { session });
        let Some(model) = model else {
            let gone: Vec<u32> = std::mem::take(&mut self.published).into_keys().collect();
            if let Some(a) = &self.active {
                for uid in gone {
                    a.send(Message::TextureSetRemoved { set: uid });
                }
            }
            return;
        };
        if self.failed_for_model != model.revision {
            self.failed.clear();
            self.failed_for_model = model.revision;
        }
        let generation = model.generation;
        // 出すセット: 目が開いていて、マテリアルに付いていて、Unity が Color を見せられる
        let wanted: Vec<(usize, u32, u32)> = state
            .sets
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let m = s.bound?;
                let info = model.materials.get(m as usize)?;
                (s.visible && info.routes.iter().any(|r| r.channel == channel::COLOR))
                    .then_some((i, s.uid, m))
            })
            .collect();
        let gone: Vec<u32> = self
            .published
            .keys()
            .copied()
            .filter(|uid| !wanted.iter().any(|w| w.1 == *uid))
            .collect();
        let mut out = Vec::new();
        for uid in gone {
            self.published.remove(&uid);
            out.push(Message::TextureSetRemoved { set: uid });
        }
        let mut notes = Vec::new();
        for (index, uid, material) in wanted {
            let doc = state.set_doc(index);
            let name = state
                .sets
                .get(index)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            let fits = self.published.get(&uid).is_some_and(|p| {
                p.set.width() == doc.width()
                    && p.set.height() == doc.height()
                    && p.set.tile_size() == doc.tile_size()
            });
            if !fits {
                self.published.remove(&uid);
                if self.failed.contains(&uid) {
                    continue;
                }
                match create(session, uid, generation, material, &name, doc, state.lang) {
                    Ok((p, tiles)) => {
                        out.push(p.set.announce());
                        self.tiles_sent += tiles as u64;
                        self.published.insert(uid, p);
                    }
                    Err(e) => {
                        self.failed.insert(uid);
                        notes.push(state.lang.pick(format!(
                            "Live Link: テクスチャセット「{name}」を Unity に出せません: {e}"
                        ), format!(
                            "Live Link: Cannot publish texture set “{name}”: {e}"
                        )));
                    }
                }
                continue;
            }
            let p = self.published.get_mut(&uid).expect("上で確かめた");
            match write_changes(p, doc, state.lang) {
                Ok(tiles) if !tiles.is_empty() => {
                    self.tiles_sent += tiles.len() as u64;
                    out.extend(p.set.tiles_changed(channel::COLOR, &tiles));
                }
                Ok(_) => {}
                Err(e) => notes.push(state.lang.pick(format!(
                    "Live Link: テクスチャセット「{name}」の共有メモリに書けません: {e}"
                ), format!(
                    "Live Link: Cannot update texture set “{name}”: {e}"
                ))),
            }
            // 世代・マテリアルの番号・名前が変わったら知らせ直す（ブリッジは知らせを受けると全部のタイルを読み直すので、中身を書いた後に）
            if p.set.generation != generation || p.set.material != material || p.set.name != name {
                p.set.generation = generation;
                p.set.material = material;
                p.set.name = name;
                out.push(p.set.announce());
            }
        }
        if let Some(active) = &self.active {
            for m in out {
                active.send(m);
            }
        }
        for n in notes {
            self.notify(NoticeLevel::Error, n, state);
        }
    }
}

/// 文書の全部を合成して共有メモリに書いた、新しい出しもの。返すのは書いたタイルの数。
#[allow(clippy::too_many_arguments)]
fn create(
    session: u64,
    uid: u32,
    generation: u32,
    material: u32,
    name: &str,
    doc: &Document,
    lang: Lang,
) -> Result<(Published, usize), String> {
    if doc.width() > MAX_TEXTURE_SIZE || doc.height() > MAX_TEXTURE_SIZE {
        return Err(lang.pick(format!(
            "大きさ {}×{} は Unity のテクスチャの上限 {MAX_TEXTURE_SIZE} を超えます",
            doc.width(),
            doc.height()
        ), format!(
            "Texture size {}×{} exceeds the Unity limit ({MAX_TEXTURE_SIZE})",
            doc.width(),
            doc.height()
        )));
    }
    if !valid_tile_size(doc.tile_size()) {
        return Err(lang.pick(format!(
            "タイルの大きさ {} は共有メモリで使えません（16〜1024 の 2 の冪）",
            doc.tile_size()
        ), format!(
            "Invalid shared tile size {} (power of two, 16–1024)",
            doc.tile_size()
        )));
    }
    let set = PublishedSet::create(
        session,
        uid,
        generation,
        material,
        name,
        doc.width(),
        doc.height(),
        doc.tile_size(),
        &[channel::COLOR],
    )
    .map_err(|e| e.to_string())?;
    let mut p = Published {
        set,
        doc_id: doc.id(),
        since: doc.change_serial(),
    };
    let ts = doc.tile_size();
    let all: Vec<TileCoord> = (0..doc.height().div_ceil(ts))
        .flat_map(|y| (0..doc.width().div_ceil(ts)).map(move |x| TileCoord::new(x, y)))
        .collect();
    let n = write_tiles(&mut p, doc, &all, lang)?.len();
    Ok((p, n))
}

/// 前に書いた後に変わったタイルを書く（文書が替わっていれば全部）。返すのは書いたタイル。
fn write_changes(p: &mut Published, doc: &Document, lang: Lang) -> Result<Vec<Tile>, String> {
    let ts = doc.tile_size();
    let coords = if p.doc_id != doc.id() {
        None
    } else {
        doc.changed_tiles(Channel::Color, p.since)
    };
    let coords = coords.unwrap_or_else(|| {
        (0..doc.height().div_ceil(ts))
            .flat_map(|y| (0..doc.width().div_ceil(ts)).map(move |x| TileCoord::new(x, y)))
            .collect()
    });
    p.doc_id = doc.id();
    p.since = doc.change_serial();
    write_tiles(p, doc, &coords, lang)
}

fn write_tiles(
    p: &mut Published,
    doc: &Document,
    coords: &[TileCoord],
    lang: Lang,
) -> Result<Vec<Tile>, String> {
    let ts = doc.tile_size() as usize;
    let mut buf = Vec::new();
    let mut tiles = Vec::with_capacity(coords.len());
    let img = p
        .set
        .image_mut(channel::COLOR)
        .ok_or(lang.pick("Color の共有メモリがありません", "Color shared memory not found"))?;
    for c in coords {
        let Some(rect) = doc.tile_rect(*c) else {
            continue;
        };
        if rect.width == 0 || rect.height == 0 {
            continue;
        }
        buf.resize(rect.width as usize * rect.height as usize * 4, 0);
        doc.composite_into(Channel::Color, rect, &mut buf, RowOrder::BottomUp)
            .map_err(|e| lang.core_error(&e))?;
        let row = rect.width as usize * 4;
        img.write_tile(c.x, c.y, |slot| {
            for r in 0..rect.height as usize {
                slot[r * ts * 4..r * ts * 4 + row].copy_from_slice(&buf[r * row..(r + 1) * row]);
            }
        })
        .map_err(|e| e.to_string())?;
        tiles.push(Tile {
            x: c.x as u16,
            y: c.y as u16,
        });
    }
    Ok(tiles)
}

/// つながり 1 つ: 挨拶（版の取り決め。2 つ目の Unity は断る）、書くスレッドを立て、このスレッドで読み続ける。
fn serve(
    stream: interprocess::local_socket::Stream,
    session: u64,
    tx: Sender<Event>,
    ctx: egui::Context,
    active: Arc<AtomicU64>,
    key: Arc<ServerKey>,
) {
    let wake = |e: Event| {
        let _ = tx.send(e);
        ctx.request_repaint();
    };
    let release = || {
        let _ = active.compare_exchange(session, 0, Ordering::AcqRel, Ordering::Relaxed);
    };
    // 「つなげる Unity は 1 つ」の枠は、挨拶（鍵と版）が済んでから取る。挨拶を送らない接続や鍵の合わない接続が枠を塞がず、
    // つながっていることも、鍵を知っている相手にしか教えない。
    let busy_agent = std::cell::RefCell::new(String::new());
    let claim = |hello: &Hello| {
        active
            .compare_exchange(0, session, Ordering::AcqRel, Ordering::Relaxed)
            .map(|_| ())
            .map_err(|_| {
                *busy_agent.borrow_mut() = hello.agent.clone();
                Reject {
                    code: RejectCode::Busy,
                    text: BUSY_TEXT.to_owned(),
                }
            })
    };
    let (conn, mut reader, hello) = match accept_with(
        stream,
        AGENT,
        session,
        &key,
        link::HANDSHAKE_TIMEOUT,
        &claim,
    ) {
        Ok(x) => x,
        Err(LinkError::Rejected(r)) if r.code == RejectCode::Busy => {
            wake(Event::Busy {
                agent: busy_agent.take(),
            });
            return;
        }
        Err(LinkError::Rejected(r)) if r.code == RejectCode::Unauthorized => {
            release();
            wake(Event::Unauthorized { text: r.text });
            return;
        }
        Err(LinkError::Rejected(r)) => {
            release();
            wake(Event::Refused { text: r.text });
            return;
        }
        Err(e) => {
            release();
            wake(Event::HandshakeFailed(e.to_string()));
            return;
        }
    };
    let version = negotiate(&hello).unwrap_or(yolu_protocol::PROTOCOL_VERSION);
    let (out_tx, out_rx) = mpsc::channel::<Out>();
    let writer = conn.clone();
    let _ = thread::Builder::new()
        .name(format!("yolu-livelink-{session}-write"))
        .spawn(move || write_loop(writer, out_rx));
    wake(Event::Connected {
        session,
        hello,
        version,
        out: out_tx,
    });
    loop {
        match reader.next(&conn) {
            Ok(Received::Message(Message::Bye)) => {
                // 自分から抜けて閉じる（Windows は受けの時間切れが無いので、待ち合わない）
                release();
                wake(Event::Closed {
                    session,
                    reason: None,
                });
                break;
            }
            Ok(Received::Message(message)) => wake(Event::Message { session, message }),
            Ok(Received::Unknown(kind)) => wake(Event::Unknown { session, kind }),
            Ok(Received::Malformed(kind, e)) => wake(Event::Malformed {
                session,
                kind,
                text: e.to_string(),
            }),
            Ok(Received::Idle) => {}
            Err(e) => {
                release();
                wake(Event::Closed {
                    session,
                    reason: Some(e.to_string()),
                });
                break;
            }
        }
    }
}

fn write_loop(conn: Connection, rx: Receiver<Out>) {
    while let Ok(out) = rx.recv() {
        match out {
            Out::Message(m) => {
                if conn.send(&m).is_err() {
                    break;
                }
            }
            Out::Bye => {
                let _ = conn.send(&Message::Bye);
                break;
            }
        }
    }
}
