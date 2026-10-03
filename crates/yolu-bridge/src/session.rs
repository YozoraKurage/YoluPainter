//! つながり 1 つの状態と裏のスレッド。C の関数（ffi）は Unity の主スレッドから呼ばれ、錠を取って状態を読み書きするだけで、
//! ソケットの読み書きは待たない（つなぐ・挨拶・読む・書くは裏のスレッド）。

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use yolu_protocol::host::now_us;
use yolu_protocol::link::{connect_and_greet, wrong_direction};
use yolu_protocol::*;

/// つながりの状態（C の関数の返す番号）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(i32)]
pub enum Status {
    Connecting = 0,
    Connected = 1,
    Closed = 2,
    Failed = 3,
}

/// C# へ渡す知らせの種類。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(i32)]
pub enum EventKind {
    Connected = 1,
    Rejected = 2,
    Failed = 3,
    Closed = 4,
    SetAdded = 5,
    SetRemoved = 6,
    /// 相手から来た Error。
    PeerError = 7,
    /// こちらで起きたこと（共有メモリを開けない など）。
    Note = 8,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub kind: EventKind,
    pub set: u32,
    pub code: i32,
    pub text: String,
}

/// テクスチャセットの 1 チャンネル。
pub struct ChannelState {
    pub channel: u8,
    pub image: Option<SharedImageReader>,
    pub dirty: Vec<bool>,
    pub dirty_count: u32,
    /// 一番新しい TilesChanged の書き終えた時刻と、ブリッジが受けた時刻（UNIX のマイクロ秒）。
    pub stamp_us: u64,
    pub received_us: u64,
}

pub struct SetState {
    pub info: TextureSet,
    pub channels: Vec<ChannelState>,
    pub revision: u32,
}

impl SetState {
    pub fn channel_mut(&mut self, channel: u8) -> Option<&mut ChannelState> {
        self.channels.iter_mut().find(|c| c.channel == channel)
    }
    pub fn channel_mask(&self) -> u32 {
        self.channels
            .iter()
            .filter(|c| c.image.is_some())
            .fold(0, |m, c| m | 1 << c.channel)
    }
}

pub struct State {
    pub status: Status,
    pub status_text: String,
    pub welcome: Option<Welcome>,
    pub events: VecDeque<Event>,
    pub sets: Vec<SetState>,
    /// 何かが変わるたびに増える（C# は変わっていなければ何もしない）。
    pub serial: u64,
    pub set_revision: u32,
}

impl State {
    fn push(&mut self, kind: EventKind, set: u32, code: i32, text: String) {
        // 読まれない知らせで膨らまない
        if self.events.len() >= 256 {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            kind,
            set,
            code,
            text,
        });
        self.serial += 1;
    }
    pub fn set_mut(&mut self, set: u32) -> Option<&mut SetState> {
        self.sets.iter_mut().find(|s| s.info.set == set)
    }
}

/// 送る順番待ち。ポーズはメッシュごとに一番新しいものだけを残す（遅いスタンドアロンに古いポーズを積まない）。
#[derive(Default)]
struct Outbox {
    frames: VecDeque<Vec<u8>>,
    pose_generation: u32,
    pose: BTreeMap<u32, MeshPose>,
    closing: bool,
    dead: bool,
}

/// 組み立て中のモデル（C# が 1 つずつ足して、最後に送る）。
#[derive(Default)]
pub struct Builder {
    pub model: Option<Model>,
    pub pose: Option<Vec<MeshPose>>,
    /// 最後に送ったモデルの世代と、メッシュごとの頂点の数（ポーズの確かめ）。
    pub sent_generation: u32,
    pub sent_vertices: Vec<usize>,
}

pub struct Session {
    state: Mutex<State>,
    outbox: Mutex<Outbox>,
    outbox_cv: Condvar,
    pub builder: Mutex<Builder>,
    stop: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Session {
    /// 裏のスレッドでつなぎ始める（すぐに返る）。
    pub fn start(id: u64, name: String, agent: String) -> Arc<Session> {
        let _ = id;
        let session = Arc::new(Session {
            state: Mutex::new(State {
                status: Status::Connecting,
                status_text: format!("{name} につないでいます"),
                welcome: None,
                events: VecDeque::new(),
                sets: Vec::new(),
                serial: 1,
                set_revision: 0,
            }),
            outbox: Mutex::new(Outbox::default()),
            outbox_cv: Condvar::new(),
            builder: Mutex::new(Builder::default()),
            stop: AtomicBool::new(false),
        });
        let s = session.clone();
        thread::Builder::new()
            .name("yolu-bridge-read".into())
            .spawn(move || s.run(&name, &agent))
            .expect("スレッドを作れない");
        session
    }

    pub fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// 送る（順番待ちに積むだけ）。
    pub fn enqueue(&self, message: &Message) -> bool {
        let mut o = lock(&self.outbox);
        if o.closing || o.dead {
            return false;
        }
        if let Message::Model(m) = message {
            o.pose.clear();
            o.pose_generation = m.generation;
        }
        o.frames.push_back(encode_message(message));
        self.outbox_cv.notify_all();
        true
    }

    /// ポーズを積む（同じメッシュの古い未送信のポーズは置き換える）。
    pub fn enqueue_pose(&self, generation: u32, meshes: Vec<MeshPose>) -> bool {
        let mut o = lock(&self.outbox);
        if o.closing || o.dead {
            return false;
        }
        if o.pose_generation != generation {
            o.pose.clear();
            o.pose_generation = generation;
        }
        for m in meshes {
            o.pose.insert(m.mesh, m);
        }
        self.outbox_cv.notify_all();
        true
    }

    /// 切る: Bye を送って閉じる（読むスレッドは相手が閉じるか、時間切れの見回りで止まる）。
    pub fn close(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let mut o = lock(&self.outbox);
        if !o.closing {
            o.closing = true;
            if !o.dead {
                o.frames.push_back(encode_message(&Message::Bye));
            }
        }
        self.outbox_cv.notify_all();
        drop(o);
        let mut st = self.state();
        if matches!(st.status, Status::Connecting | Status::Connected) {
            st.status = Status::Closed;
            st.status_text = "切りました".into();
            st.serial += 1;
        }
        // 共有メモリの写像はここで手放す（テクスチャは C# が外す）
        st.sets.clear();
    }

    fn fail(&self, status: Status, kind: EventKind, code: i32, text: String) {
        {
            let mut o = lock(&self.outbox);
            o.dead = true;
            o.frames.clear();
            o.pose.clear();
            self.outbox_cv.notify_all();
        }
        let mut st = self.state();
        if st.status != Status::Closed || status == Status::Failed {
            st.status = status;
        }
        st.status_text = text.clone();
        st.sets.clear();
        st.push(kind, 0, code, text);
    }

    fn run(self: Arc<Self>, name: &str, agent: &str) {
        let (conn, mut reader, welcome) = match connect_and_greet(name, agent) {
            Ok(x) => x,
            Err(LinkError::Rejected(r)) => {
                self.fail(
                    Status::Failed,
                    EventKind::Rejected,
                    r.code as i32,
                    format!("スタンドアロンが断りました: {}", r.text),
                );
                return;
            }
            Err(e) => {
                self.fail(
                    Status::Failed,
                    EventKind::Failed,
                    0,
                    format!("{name} につなげません: {e}"),
                );
                return;
            }
        };
        if self.stop.load(Ordering::Relaxed) {
            let _ = conn.send(&Message::Bye);
            return;
        }
        {
            let mut st = self.state();
            st.status = Status::Connected;
            st.status_text = format!(
                "{} とつながりました（プロトコルの版 {}）",
                welcome.agent, welcome.version
            );
            let text = st.status_text.clone();
            st.welcome = Some(welcome);
            st.push(EventKind::Connected, 0, 0, text);
        }
        let writer = {
            let s = self.clone();
            let c = conn.clone();
            thread::Builder::new()
                .name("yolu-bridge-write".into())
                .spawn(move || s.write_loop(c))
                .expect("スレッドを作れない")
        };
        // Linux は時間切れで止める合図を見回る（Windows は時間切れが無いので、相手が閉じたときに止まる）
        reader.set_timeout(Some(Duration::from_millis(200)));
        loop {
            if self.stop.load(Ordering::Relaxed) && lock(&self.outbox).frames.is_empty() {
                // Bye を送り終えた。相手が閉じるのを少し待つ
                reader.set_timeout(Some(Duration::from_millis(500)));
            }
            match reader.next(&conn) {
                Ok(Received::Idle) => {
                    if self.stop.load(Ordering::Relaxed) && lock(&self.outbox).frames.is_empty() {
                        break;
                    }
                }
                Ok(Received::Message(Message::Bye)) => {
                    if !self.stop.load(Ordering::Relaxed) {
                        self.fail(
                            Status::Closed,
                            EventKind::Closed,
                            0,
                            "スタンドアロンがつながりを閉じました".into(),
                        );
                    }
                    break;
                }
                Ok(Received::Message(m)) => {
                    if let Some(reply) = wrong_direction(&m, false) {
                        let _ = conn.send(&reply);
                        continue;
                    }
                    self.handle(m);
                }
                Ok(Received::Unknown(kind)) => self.note(format!(
                    "スタンドアロンから知らない命令が来ました（{}）。ブリッジより新しい版のスタンドアロンかもしれません",
                    yolu_protocol::link::kind_name(kind)
                )),
                Ok(Received::Malformed(kind, e)) => self.note(format!(
                    "スタンドアロンからの {} を読めません: {e}",
                    yolu_protocol::link::kind_name(kind)
                )),
                Err(e) => {
                    if !self.stop.load(Ordering::Relaxed) {
                        self.fail(
                            Status::Closed,
                            EventKind::Closed,
                            0,
                            format!("つながりが切れました: {e}"),
                        );
                    }
                    break;
                }
            }
        }
        {
            let mut o = lock(&self.outbox);
            o.dead = true;
            self.outbox_cv.notify_all();
        }
        let _ = writer.join();
    }

    fn write_loop(&self, conn: Connection) {
        loop {
            let frame = {
                let mut o = lock(&self.outbox);
                loop {
                    if o.dead {
                        return;
                    }
                    if let Some(f) = o.frames.pop_front() {
                        break f;
                    }
                    if !o.pose.is_empty() {
                        let meshes: Vec<MeshPose> =
                            std::mem::take(&mut o.pose).into_values().collect();
                        break encode_message(&Message::Pose(Pose {
                            generation: o.pose_generation,
                            meshes,
                        }));
                    }
                    if o.closing {
                        return;
                    }
                    o = self.outbox_cv.wait(o).unwrap_or_else(|e| e.into_inner());
                }
            };
            if let Err(e) = conn.send_frame(&frame) {
                if !self.stop.load(Ordering::Relaxed) {
                    self.fail(
                        Status::Closed,
                        EventKind::Closed,
                        0,
                        format!("送れません: {e}"),
                    );
                }
                return;
            }
        }
    }

    fn note(&self, text: String) {
        self.state().push(EventKind::Note, 0, 0, text);
    }

    fn handle(&self, message: Message) {
        let mut st = self.state();
        match message {
            Message::TextureSet(info) => {
                let mut notes = Vec::new();
                let tiles = (info.width.div_ceil(info.tile_size)
                    * info.height.div_ceil(info.tile_size)) as usize;
                let channels = info
                    .channels
                    .iter()
                    .map(|c| {
                        let opened = SharedImageReader::open(Path::new(&c.path)).and_then(|img| {
                            let l = img.layout();
                            if l.width != info.width
                                || l.height != info.height
                                || l.tile_size != info.tile_size
                                || img.channel() != c.channel
                                || img.set() != info.set
                            {
                                Err(ShmError::Invalid("知らせと中身の大きさ・チャンネルが違う"))
                            } else {
                                Ok(img)
                            }
                        });
                        let image = match opened {
                            Ok(img) => Some(img),
                            Err(e) => {
                                notes.push(format!(
                                    "テクスチャセット「{}」のチャンネル {} の共有メモリを開けません: {e}",
                                    info.name, c.channel
                                ));
                                None
                            }
                        };
                        // 知らせた時の中身を全部写す（前のつながりで書いたタイルは、もう知らせが来ない）
                        let ok = image.is_some();
                        ChannelState {
                            channel: c.channel,
                            image,
                            dirty: vec![ok; tiles],
                            dirty_count: if ok { tiles as u32 } else { 0 },
                            stamp_us: 0,
                            received_us: now_us(),
                        }
                    })
                    .collect();
                st.set_revision += 1;
                let revision = st.set_revision;
                let set_id = info.set;
                let name = info.name.clone();
                st.sets.retain(|s| s.info.set != set_id);
                st.sets.push(SetState {
                    info,
                    channels,
                    revision,
                });
                st.sets.sort_by_key(|s| s.info.set);
                st.push(EventKind::SetAdded, set_id, 0, name);
                for n in notes {
                    st.push(EventKind::Note, set_id, 0, n);
                }
            }
            Message::TextureSetRemoved { set } => {
                let before = st.sets.len();
                st.sets.retain(|s| s.info.set != set);
                if st.sets.len() != before {
                    st.push(EventKind::SetRemoved, set, 0, String::new());
                }
            }
            Message::TilesChanged(t) => {
                let received = now_us();
                let Some(s) = st.set_mut(t.set) else { return };
                let Some(c) = s.channel_mut(t.channel) else {
                    return;
                };
                let Some(img) = &c.image else { return };
                let l = *img.layout();
                for tile in &t.tiles {
                    if let Some(i) = l.tile_index(tile.x as u32, tile.y as u32) {
                        if !c.dirty[i] {
                            c.dirty[i] = true;
                            c.dirty_count += 1;
                        }
                    }
                }
                c.stamp_us = t.stamp_us;
                c.received_us = received;
                st.serial += 1;
            }
            Message::Error(e) => {
                let text = format!(
                    "スタンドアロンからの誤りの知らせ（{}）: {}",
                    yolu_protocol::link::kind_name(e.kind),
                    e.text
                );
                st.push(EventKind::PeerError, 0, e.code as i32, text);
            }
            // 挨拶はつないだときだけ
            Message::Welcome(_) | Message::Reject(_) => {
                st.push(
                    EventKind::Note,
                    0,
                    0,
                    "挨拶の後に Welcome・Reject が来たので捨てました".into(),
                );
            }
            _ => {}
        }
    }

    pub fn status(&self) -> Status {
        self.state().status
    }
}
