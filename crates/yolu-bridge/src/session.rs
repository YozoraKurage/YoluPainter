//! つながり 1 つの状態と裏のスレッド。C の関数（ffi）は Unity の主スレッドから呼ばれ、錠を取って状態を読み書きするだけで、
//! ソケットの読み書きは待たない（つなぐ・挨拶・読む・書くは裏のスレッド）。

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use yolu_protocol::compat::refusal_from_reject;
use yolu_protocol::host::now_us;
use yolu_protocol::link::{connect_and_greet_as, wrong_direction};
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
    /// 挨拶が済んだ後の、両側の名乗りと決まった版（つながるまでは None）。
    pub link: Option<LinkInfo>,
    /// プロトコルの版の範囲が合わずに断られたときの、どちらを何版以上にするか（それ以外は None）。
    pub refusal: Option<VersionRefusal>,
    pub events: VecDeque<Event>,
    /// スタンドアロンからの頼み（まだ C# が取り出していないもの。`take_request`）。
    pub requests: VecDeque<Request>,
    pub sets: Vec<SetState>,
    /// 何かが変わるたびに増える（C# は変わっていなければ何もしない）。
    pub serial: u64,
    pub set_revision: u32,
}

/// スタンドアロンからの頼みの項目 1 つ（`MaterialRequest` の項目に、頼みの世代を付けたもの）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub generation: u32,
    pub item: MaterialWant,
}

/// 取り出されない頼みで膨らまない上限（古いものから捨てる。同じマテリアル・同じ頼みは 1 つにまとめる）。
pub const MAX_PENDING_REQUESTS: usize = 4096;

impl State {
    /// 頼みを積む。同じ世代・マテリアル・頼むもの・スロットの前の頼みは置き換える（新しい `have` の印で答える）。
    pub fn push_request(&mut self, generation: u32, item: MaterialWant) {
        self.requests.retain(|r| {
            !(r.generation == generation
                && r.item.material == item.material
                && r.item.wants == item.wants
                && r.item.slot == item.slot)
        });
        if self.requests.len() >= MAX_PENDING_REQUESTS {
            self.requests.pop_front();
        }
        self.requests.push_back(Request { generation, item });
        self.serial += 1;
    }

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

/// 送りの列（書き終えていない枠と、いま書いている枠）に持てる大きさの合計の上限（バイト）。スタンドアロンの受け取りの列
/// （`yolu-app` の `MAX_QUEUED_BYTES`）・ポーズの溜め場と同じ量で、Unity エディターのプロセスの中に、読まない相手のための荷が
/// これ以上溜まらないようにする。1 回の値の送りの絵（64 MiB）は上限の中に収まり、元の絵を読んで待たせる側（C# の 96 MiB の目安）の量より
/// 十分に大きい。辺 8192 の元の絵 1 枚は画素だけで 256 MiB（RGBA8）あり、枠の頭を足すと上限を超える。そういう 1 つの命令（と 512 MiB までの
/// モデル）は、列が空のときだけ積む（`try_enqueue`）ので、ほかの物と並んで積まれることは無い。
/// 守るのは枠の列だけで、ポーズ（メッシュごとに最新 1 つ。送ったモデルの頂点の数に収まる）は別に持つ。
pub const MAX_OUTBOX_BYTES: usize = 256 << 20;

#[cfg(test)]
thread_local! {
    /// 試験用: このスレッドが、積む命令を枠にした回数（断る命令を、枠にして捨てていないことを確かめる）。
    static ENCODED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// 積めない理由（積めたときは `Ok(true)`・印が無い／閉じているときは `Ok(false)`）。
#[derive(Debug)]
pub enum EnqueueError {
    /// 1 つの命令が枠の上限を超える（巨大なモデル）。
    TooLarge(FrameError),
    /// 送りの列が混んでいる（相手が読むのを待っている）。何も積まず、前に積んだものも変えない。少し後に同じ物を送り直せる。
    Busy,
}

impl std::fmt::Display for EnqueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnqueueError::TooLarge(e) => write!(f, "{e}"),
            EnqueueError::Busy => write!(f, "送りの列が混んでいます"),
        }
    }
}

/// 後から来る同じ物に置き換えられる命令の持ち主（世代・マテリアル・スロット）。受け手は、世代とマテリアルの番号でモデルに結び付けて
/// 受けるので、同じ持ち主の新しい命令だけが古いものを置き換える（`superseded`）。
#[derive(Clone, PartialEq, Eq, Debug)]
enum Tag {
    Materials { generation: u32 },
    Values { generation: u32, material: u32 },
    Texture { generation: u32, material: u32, slot: String },
    Original { generation: u32, material: u32, slot: String },
}

impl Tag {
    fn of(message: &Message) -> Option<Tag> {
        match message {
            Message::Materials(u) => Some(Tag::Materials { generation: u.generation }),
            Message::MaterialValues(v) => Some(Tag::Values { generation: v.generation, material: v.material }),
            Message::MaterialTexture(t) => Some(Tag::Texture {
                generation: t.generation,
                material: t.material,
                slot: t.slot.clone(),
            }),
            Message::MaterialOriginal(o) => Some(Tag::Original {
                generation: o.generation,
                material: o.material,
                slot: o.slot.clone(),
            }),
            _ => None,
        }
    }
}

/// 順番待ちの枠 1 つ。
struct Queued {
    frame: Vec<u8>,
    tag: Option<Tag>,
}

/// 新しい命令が置き換える、列の中の古い命令の位置（小さい順）。置き換えるのは、受け手が最後の 1 つだけで同じ状態になるものだけ:
/// - マテリアルの更新: 同じ世代のもの（毎回、全マテリアルの完全な情報）。
/// - スロットの絵: 同じ世代・マテリアル・スロットのもの（受け手は来るたびに絵を取り替える）。
/// - 元の絵: 画素の付いたものが、同じ世代・マテリアル・スロットの古い元の絵（どの様子でも）を置き換える。画素なしの様子（読めない・
///   印が同じ など）は、前の画素に頼ることがあるので何も置き換えない。
/// - マテリアルの値: 同じ世代・マテリアルの古い値と絵を、まとめて置き換える（絵は値が「来る」と言ったスロットだけを受け手が受けるので、
///   値だけを捨てて絵を残さない）。ただし新しい値が「前と同じ」と言うスロットの絵が列の中にあるなら、その絵は古い値のものなので、
///   何も置き換えない（捨てると、受け手はその絵を持たないまま「前と同じ」を受ける）。
///
/// モデル・制御の命令（モデルを閉じる など）は置き換えない。モデルのあとに続く値・絵・元の絵が世代でそのモデルに結び付いているので、
/// モデルを捨てると続きだけが宙に浮く。
fn superseded(frames: &VecDeque<Queued>, message: &Message) -> Vec<usize> {
    let hit = |matches: &dyn Fn(&Tag) -> bool| -> Vec<usize> {
        frames
            .iter()
            .enumerate()
            .filter(|(_, q)| q.tag.as_ref().is_some_and(matches))
            .map(|(i, _)| i)
            .collect()
    };
    match message {
        Message::Materials(u) => hit(&|t| matches!(t, Tag::Materials { generation } if *generation == u.generation)),
        Message::MaterialTexture(t) => hit(&|o| {
            matches!(o, Tag::Texture { generation, material, slot }
                if *generation == t.generation && *material == t.material && *slot == t.slot)
        }),
        Message::MaterialOriginal(o) if o.state == OriginalState::Image => hit(&|t| {
            matches!(t, Tag::Original { generation, material, slot }
                if *generation == o.generation && *material == o.material && *slot == o.slot)
        }),
        Message::MaterialValues(v) => {
            let waiting = |slot: &str| {
                frames.iter().any(|q| {
                    matches!(&q.tag, Some(Tag::Texture { generation, material, slot: s })
                        if *generation == v.generation && *material == v.material && s == slot)
                })
            };
            if v.slots.iter().any(|s| s.state == SlotState::Unchanged && waiting(&s.name)) {
                return Vec::new();
            }
            hit(&|t| match t {
                Tag::Values { generation, material } | Tag::Texture { generation, material, .. } => {
                    *generation == v.generation && *material == v.material
                }
                _ => false,
            })
        }
        _ => Vec::new(),
    }
}

/// 断らずに積む制御の命令（小さく、相手に状態の終わりを知らせるもの）。
fn is_control(message: &Message) -> bool {
    matches!(
        message,
        Message::ModelClosed { .. } | Message::TextureSetRemoved { .. } | Message::Error(_) | Message::Bye
    )
}

/// 送る順番待ち。ポーズはメッシュごとに一番新しいものだけを残す（遅いスタンドアロンに古いポーズを積まない）。
struct Outbox {
    frames: VecDeque<Queued>,
    /// 積んだ枠のバイトの合計と、いま書いている（列から出したが書き終えていない）枠のバイト。
    queued: usize,
    in_flight: usize,
    /// 積める合計の上限（`MAX_OUTBOX_BYTES`。試験が狭める）。
    limit: usize,
    pose_generation: u32,
    pose: BTreeMap<u32, MeshPose>,
    closing: bool,
    dead: bool,
}

impl Default for Outbox {
    fn default() -> Self {
        Outbox {
            frames: VecDeque::new(),
            queued: 0,
            in_flight: 0,
            limit: MAX_OUTBOX_BYTES,
            pose_generation: 0,
            pose: BTreeMap::new(),
            closing: false,
            dead: false,
        }
    }
}

impl Outbox {
    /// 列に持っている量（積んだ枠と、書いている途中の枠）。
    fn held(&self) -> usize {
        self.queued + self.in_flight
    }

    fn push(&mut self, frame: Vec<u8>, tag: Option<Tag>) {
        self.queued += frame.len();
        self.frames.push_back(Queued { frame, tag });
    }

    /// 古い物を取り除く（`superseded` の位置）。
    fn remove(&mut self, positions: &[usize]) {
        if positions.is_empty() {
            return;
        }
        let mut index = 0;
        let mut freed = 0;
        self.frames.retain(|q| {
            let drop = positions.binary_search(&index).is_ok();
            index += 1;
            if drop {
                freed += q.frame.len();
            }
            !drop
        });
        self.queued -= freed;
    }

    fn clear(&mut self) {
        self.frames.clear();
        self.queued = 0;
    }

    /// 次に書く枠を列から出す（書いている途中の量に数える）。
    fn pop(&mut self) -> Option<Vec<u8>> {
        let q = self.frames.pop_front()?;
        self.queued -= q.frame.len();
        self.in_flight = q.frame.len();
        Some(q.frame)
    }

    /// 残る量（`kept`）の上に、枠 1 つ（`frame_len`）を積んでよいか。残る量が 0 なら、上限より大きい 1 つも積む。
    fn fits(&self, kept: usize, frame_len: usize) -> bool {
        kept == 0 || kept.saturating_add(frame_len) <= self.limit
    }

    /// いま積める大きさの目安。空なら 1 つの命令がどんなに大きくても入る（上限より大きい 1 つの命令が、永久に入れなくならない）。
    fn room(&self) -> u64 {
        if self.held() == 0 {
            u64::MAX
        } else {
            self.limit.saturating_sub(self.held()) as u64
        }
    }
}

/// 組み立て中のモデル（C# が 1 つずつ足して、最後に送る）。
#[derive(Default)]
pub struct Builder {
    pub model: Option<Model>,
    pub pose: Option<Vec<MeshPose>>,
    /// 組み立て中のマテリアルの更新。
    pub materials: Option<Vec<MaterialInfo>>,
    /// 組み立て中のマテリアルの値（`ylb_values_*`）。
    pub values: Option<MaterialValues>,
    /// 最後に送ったモデルの世代と、メッシュごとの頂点の数（ポーズの確かめ）・マテリアルの数（更新の確かめ）。
    pub sent_generation: u32,
    pub sent_vertices: Vec<usize>,
    pub sent_materials: usize,
}

pub struct Session {
    state: Mutex<State>,
    outbox: Mutex<Outbox>,
    outbox_cv: Condvar,
    pub builder: Mutex<Builder>,
    stop: AtomicBool,
    /// 1 つの命令の中身の上限（バイト。既定は枠の上限 `MAX_PAYLOAD`。試験が小さくして、巨大な命令を作らずに断りの流れを確かめる）。
    payload_limit: AtomicUsize,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Session {
    /// 裏のスレッドでつなぎ始める（すぐに返る）。
    pub fn start(id: u64, name: String, identity: Identity) -> Arc<Session> {
        let _ = id;
        let session = Session::build(&name);
        let s = session.clone();
        thread::Builder::new()
            .name("yolu-bridge-read".into())
            .spawn(move || s.run(&name, &identity))
            .expect("スレッドを作れない");
        session
    }

    /// つなぎ始める前の器（つなぐスレッドは `start` が起こす。試験は、つながった状態を直接作るためにこれだけを使う）。
    fn build(name: &str) -> Arc<Session> {
        Arc::new(Session {
            state: Mutex::new(State {
                status: Status::Connecting,
                status_text: format!("{name} につないでいます"),
                welcome: None,
                link: None,
                refusal: None,
                events: VecDeque::new(),
                requests: VecDeque::new(),
                sets: Vec::new(),
                serial: 1,
                set_revision: 0,
            }),
            outbox: Mutex::new(Outbox::default()),
            outbox_cv: Condvar::new(),
            builder: Mutex::new(Builder::default()),
            stop: AtomicBool::new(false),
            payload_limit: AtomicUsize::new(frame::MAX_PAYLOAD),
        })
    }

    /// 1 つの命令の中身の上限（バイト）。
    pub fn payload_limit(&self) -> usize {
        self.payload_limit.load(Ordering::Relaxed)
    }

    /// 上限を狭める（`MAX_PAYLOAD` を超えては広げない。試験用）。
    pub fn set_payload_limit(&self, limit: usize) {
        self.payload_limit
            .store(limit.min(frame::MAX_PAYLOAD), Ordering::Relaxed);
    }

    pub fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// このつながりで使える機能（双方の印の共通部分。つながるまでは 0）。
    pub fn common_features(&self) -> u64 {
        lock(&self.state).link.as_ref().map_or(0, LinkInfo::common_features)
    }

    /// まだ書き終えていない枠のバイトの合計（順番待ちに積んだものと、いま書いている途中のもの）。
    pub fn pending_bytes(&self) -> u64 {
        lock(&self.outbox).held() as u64
    }

    /// いま積める大きさの目安（バイト）。`MAX_OUTBOX_BYTES` から、積んでいる量を引いたもの。何も積んでいなければ、1 つの命令が上限より大きくても
    /// 入るので `u64::MAX`。C# は、大きな絵を読む前にこれで足りるかを見て、足りなければ読まずに待つ。
    pub fn send_room(&self) -> u64 {
        lock(&self.outbox).room()
    }

    /// 積める大きさの合計の上限を決める（試験用。既定は `MAX_OUTBOX_BYTES`）。
    pub fn set_outbox_limit(&self, bytes: usize) {
        lock(&self.outbox).limit = bytes;
    }

    /// 命令を送ってよいか（命令が要る機能の印が、相手にも立っているか。`need_of` は命令の種類ごとに要る印の決め方で、実際の送り口は
    /// `Kind::required_feature`、試験は印の要る表を差し込む）。積む口（`enqueue`・`enqueue_pose`）は、必ずこれで確かめてから積む。
    fn accepts_with(&self, message: &Message, need_of: impl Fn(Kind) -> u64) -> bool {
        yolu_protocol::compat::accepts_with(self.common_features(), message, need_of)
    }

    /// 送る（順番待ちに積むだけ）。印の要る命令で、相手に印が無ければ積まない（false）。枠の上限を超える命令・列が混んでいるときも積まない（false。
    /// 理由を知りたい口は `try_enqueue`）。
    pub fn enqueue(&self, message: &Message) -> bool {
        self.enqueue_with(message, Kind::required_feature)
    }

    fn enqueue_with(&self, message: &Message, need_of: impl Fn(Kind) -> u64) -> bool {
        matches!(self.try_enqueue_with(message, need_of), Ok(true))
    }

    /// `enqueue` の、積めない理由を返す形（`Ok(false)` は印が無い・閉じている。`Err` は枠の上限を超える・列が混んでいる）。
    ///
    /// 列の合計が上限（`MAX_OUTBOX_BYTES`）を超えるときは、`superseded` の古い物を置き換えても収まらなければ `EnqueueError::Busy` で断る
    /// （断るときは何も変えない）。例外: 列が空なら、上限より大きい 1 つの命令（512 MiB までのモデル・256 MiB の元の絵）も積む。
    /// 制御の命令（モデルを閉じる など）は小さく、終わりを知らせるので、混んでいても積む。
    /// モデルは、混んでいて断るときも「大きすぎる」で断るときも、枠にする前に大きさで決める（送り直しのたびに、巨大なモデルを枠にして捨てない）。
    pub fn try_enqueue(&self, message: &Message) -> Result<bool, EnqueueError> {
        self.try_enqueue_with(message, Kind::required_feature)
    }

    fn try_enqueue_with(
        &self,
        message: &Message,
        need_of: impl Fn(Kind) -> u64,
    ) -> Result<bool, EnqueueError> {
        if !self.accepts_with(message, need_of) {
            return Ok(false);
        }
        // モデルは置き換えられず（`superseded` は何も返さない）、枠の大きさは枠にしなくても分かる。入らないなら、枠にする前に断る
        // （C# は混んでいる間、同じモデルを 0.5 秒おきに送り直す。そのたびに数百 MiB を枠にして捨てると、エディターの主のスレッドが止まり、
        // メモリが上限を超えて跳ねる）。枠にしたあとの確かめ（下）は、そのあいだに列が変わった分のために残す
        if let Message::Model(m) = message {
            let payload = usize::try_from(m.payload_len()).unwrap_or(usize::MAX);
            if payload > self.payload_limit().min(frame::MAX_PAYLOAD) {
                return Err(EnqueueError::TooLarge(FrameError::TooLarge(payload)));
            }
            let o = lock(&self.outbox);
            if o.closing || o.dead {
                return Ok(false);
            }
            if !o.fits(o.held(), frame::HEADER_LEN.saturating_add(payload)) {
                return Err(EnqueueError::Busy);
            }
        }
        // 枠にしてから錠を取る（大きな命令を書く間、送り口の錠を持たない）
        #[cfg(test)]
        ENCODED.with(|c| c.set(c.get() + 1));
        let frame = try_encode_message_within(message, self.payload_limit())
            .map_err(EnqueueError::TooLarge)?;
        let mut o = lock(&self.outbox);
        if o.closing || o.dead {
            return Ok(false);
        }
        let old = superseded(&o.frames, message);
        let freed: usize = old.iter().map(|&i| o.frames[i].frame.len()).sum();
        let kept = o.held() - freed;
        if !is_control(message) && !o.fits(kept, frame.len()) {
            return Err(EnqueueError::Busy);
        }
        o.remove(&old);
        if let Message::Model(m) = message {
            o.pose.clear();
            o.pose_generation = m.generation;
        }
        o.push(frame, Tag::of(message));
        self.outbox_cv.notify_all();
        Ok(true)
    }

    /// ポーズを積む（同じメッシュの古い未送信のポーズは置き換える）。ポーズの命令が印を要るなら、相手に印が無ければ積まない（false）。
    pub fn enqueue_pose(&self, generation: u32, meshes: Vec<MeshPose>) -> bool {
        self.enqueue_pose_with(generation, meshes, Kind::required_feature)
    }

    fn enqueue_pose_with(
        &self,
        generation: u32,
        meshes: Vec<MeshPose>,
        need_of: impl Fn(Kind) -> u64,
    ) -> bool {
        if !yolu_protocol::compat::satisfies(self.common_features(), need_of(Kind::Pose)) {
            return false;
        }
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
                o.push(encode_message(&Message::Bye), None);
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
        // つながりは終わった: 相手の版・使える機能は、つながっている間だけの答え
        st.link = None;
        // 共有メモリの写像はここで手放す（テクスチャは C# が外す）
        st.sets.clear();
    }

    fn fail(&self, status: Status, kind: EventKind, code: i32, text: String) {
        {
            let mut o = lock(&self.outbox);
            o.dead = true;
            o.clear();
            o.pose.clear();
            self.outbox_cv.notify_all();
        }
        let mut st = self.state();
        if st.status != Status::Closed || status == Status::Failed {
            st.status = status;
        }
        st.status_text = text.clone();
        st.sets.clear();
        // つながりは終わった: 相手の版・使える機能は、つながっている間だけの答え（閉じたあとも版のずれの印が残らない）
        st.link = None;
        st.push(kind, 0, code, text);
    }

    fn run(self: Arc<Self>, name: &str, identity: &Identity) {
        let (conn, mut reader, welcome) = match connect_and_greet_as(name, identity) {
            Ok(x) => x,
            Err(LinkError::Rejected(r)) => {
                self.state().refusal = refusal_from_reject(Product::Standalone, &r);
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
            st.link = conn.link_info().cloned();
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
            if self.stop.load(Ordering::Relaxed) && lock(&self.outbox).held() == 0 {
                // Bye を送り終えた。相手が閉じるのを少し待つ
                reader.set_timeout(Some(Duration::from_millis(500)));
            }
            match reader.next(&conn) {
                Ok(Received::Idle) => {
                    if self.stop.load(Ordering::Relaxed) && lock(&self.outbox).held() == 0 {
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
                    if let Some(f) = o.pop() {
                        break f;
                    }
                    if !o.pose.is_empty() {
                        let meshes: Vec<MeshPose> =
                            std::mem::take(&mut o.pose).into_values().collect();
                        match try_encode_message_within(
                            &Message::Pose(Pose {
                                generation: o.pose_generation,
                                meshes,
                            }),
                            self.payload_limit(),
                        ) {
                            Ok(frame) => {
                                o.in_flight = frame.len();
                                break frame;
                            }
                            // 枠の上限を超えるポーズは送らない（つながりは保つ。次の変化で新しいポーズを送る）
                            Err(e) => {
                                drop(o);
                                self.note(format!("ポーズを送れません: {e}"));
                                o = lock(&self.outbox);
                                continue;
                            }
                        }
                    }
                    if o.closing {
                        return;
                    }
                    o = self.outbox_cv.wait(o).unwrap_or_else(|e| e.into_inner());
                }
            };
            let sent = conn.send_frame(&frame);
            lock(&self.outbox).in_flight = 0;
            if let Err(e) = sent {
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
            Message::MaterialRequest(r) => {
                for item in r.items {
                    st.push_request(r.generation, item);
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_protocol::compat::PeerInfo;

    const MARK_A: u64 = 1 << 40;
    const MARK_B: u64 = 1 << 41;

    /// 試験用の印の要る表（今の命令は印を要らないので、試験が差し込む）: マテリアルは A、ポーズは B、モデルを閉じるのは A と B。
    fn need_of(kind: Kind) -> u64 {
        match kind {
            Kind::Materials => MARK_A,
            Kind::Pose => MARK_B,
            Kind::ModelClosed => MARK_A | MARK_B,
            _ => 0,
        }
    }

    /// 挨拶が済んだ状態の器（双方の印だけを決める。ソケットは張らない）。
    fn linked(own: u64, peer: u64) -> Arc<Session> {
        let session = Session::build("試験");
        {
            let mut st = session.state();
            st.status = Status::Connected;
            st.link = Some(LinkInfo {
                protocol: PROTOCOL_VERSION,
                own: Identity::unity("試験のブリッジ").with_features(own),
                peer: PeerInfo {
                    agent: "試験のスタンドアロン".into(),
                    versions: None,
                    features: peer,
                },
            });
        }
        session
    }

    fn materials() -> Message {
        Message::Materials(MaterialsUpdate {
            generation: 1,
            materials: Vec::new(),
        })
    }

    fn poses() -> Vec<MeshPose> {
        vec![MeshPose {
            mesh: 0,
            positions: vec![[0.0; 3]],
            normals: Vec::new(),
        }]
    }

    /// 積んだ枠の数とポーズのメッシュの数。
    fn queued(session: &Session) -> (usize, usize) {
        let o = lock(&session.outbox);
        (o.frames.len(), o.pose.len())
    }

    #[test]
    fn a_command_that_needs_a_mark_is_queued_only_when_both_sides_have_it() {
        let closed = Message::ModelClosed { generation: 1 };
        // 双方に A だけ: A を要るマテリアルは積む。A と B を要るものは積まない（B は相手に無い）
        let s = linked(MARK_A | MARK_B, MARK_A | 1 << 50);
        assert_eq!(s.common_features(), MARK_A);
        assert!(s.accepts_with(&materials(), need_of));
        assert!(!s.accepts_with(&closed, need_of));
        assert!(s.enqueue_with(&materials(), need_of));
        assert!(!s.enqueue_with(&closed, need_of));
        // 印の要らない命令はいつも積む
        assert!(s.enqueue_with(&Message::TextureSetRemoved { set: 1 }, need_of));
        assert_eq!(queued(&s), (2, 0), "積んだのはマテリアルと印の要らない命令");

        // 自分にだけ印がある（相手の印が無い）: 送らない
        let s = linked(MARK_A | MARK_B, 0);
        assert_eq!(s.common_features(), 0);
        assert!(!s.enqueue_with(&materials(), need_of));
        assert!(!s.enqueue_with(&closed, need_of));
        assert_eq!(queued(&s), (0, 0));

        // 双方に A と B: 全部積む
        let s = linked(MARK_A | MARK_B, MARK_A | MARK_B);
        assert!(s.enqueue_with(&materials(), need_of));
        assert!(s.enqueue_with(&closed, need_of));
        assert_eq!(queued(&s), (2, 0));
    }

    #[test]
    fn a_pose_goes_through_the_same_gate() {
        // ポーズは B を要る（試験の表）: 相手に B が無ければ積まず、世代も動かさない
        let s = linked(MARK_A | MARK_B, MARK_A);
        assert!(!s.enqueue_pose_with(3, poses(), need_of));
        assert_eq!(queued(&s), (0, 0));
        assert_eq!(lock(&s.outbox).pose_generation, 0);
        // B があれば積む
        let s = linked(MARK_B, MARK_B);
        assert!(s.enqueue_pose_with(3, poses(), need_of));
        assert_eq!(queued(&s), (0, 1));
        // 今の命令の表（印の要らない）では、相手の印が無くても積む（今までどおり）
        let s = linked(0, 0);
        assert!(s.enqueue_pose(1, poses()));
        assert!(s.enqueue(&materials()));
        assert_eq!(queued(&s), (1, 1));
    }

    #[test]
    fn nothing_is_queued_before_the_link_is_made() {
        // 挨拶の前（link が無い）は共通の印が 0: 印を要る命令は積まない
        let s = Session::build("試験");
        assert_eq!(s.common_features(), 0);
        assert!(!s.enqueue_with(&materials(), need_of));
        assert!(!s.enqueue_pose_with(1, poses(), need_of));
        assert!(s.enqueue_with(&Message::TextureSetRemoved { set: 1 }, need_of));
    }

    #[test]
    fn a_command_over_the_payload_limit_is_not_queued_and_says_why() {
        let s = linked(0, 0);
        assert_eq!(s.payload_limit(), frame::MAX_PAYLOAD);
        s.set_payload_limit(usize::MAX);
        assert_eq!(s.payload_limit(), frame::MAX_PAYLOAD, "枠の上限より広げない");
        s.set_payload_limit(64);
        let big = Message::Error(ErrorMessage {
            code: ErrorCode::Other,
            kind: 0,
            text: "x".repeat(200),
        });
        // 積めない理由が分かる（パニックしない）。印の無い・閉じているときの false とは別
        assert!(matches!(
            s.try_enqueue(&big),
            Err(EnqueueError::TooLarge(FrameError::TooLarge(n))) if n > 200
        ));
        assert!(!s.enqueue(&big));
        assert_eq!(queued(&s), (0, 0));
        // 上限に収まる命令は積める
        assert!(s.try_enqueue(&Message::TextureSetRemoved { set: 1 }).unwrap());
        assert_eq!(queued(&s), (1, 0));
        // 上限を戻せば同じ命令を積める
        s.set_payload_limit(frame::MAX_PAYLOAD);
        assert!(s.enqueue(&big));
    }

    #[test]
    fn requests_from_the_standalone_are_merged_and_bounded() {
        let s = linked(0, 0);
        let mut st = s.state();
        let before = st.serial;
        st.push_request(3, MaterialWant::original(1, "_MainTex", 7));
        st.push_request(3, MaterialWant::values(1));
        // 同じ世代・マテリアル・頼むもの・スロットは 1 つにまとまり、あとの印が残る（順番はあとへ）
        st.push_request(3, MaterialWant::original(1, "_MainTex", 9));
        assert_eq!(st.requests.len(), 2);
        assert_eq!(st.requests.back().unwrap().item.have, 9);
        // 世代が違えば別の頼み
        st.push_request(4, MaterialWant::original(1, "_MainTex", 9));
        assert_eq!(st.requests.len(), 3);
        assert!(st.serial > before, "C# が気づく");
        // 取り出されない頼みで膨らまない（古いものから捨てる）
        for m in 0..(MAX_PENDING_REQUESTS as u32 + 10) {
            st.push_request(5, MaterialWant::values(m + 100));
        }
        assert_eq!(st.requests.len(), MAX_PENDING_REQUESTS);
        assert_eq!(st.requests.back().unwrap().item.material, MAX_PENDING_REQUESTS as u32 + 109);
        assert_eq!(st.requests.front().unwrap().item.material, 110);
    }

    #[test]
    fn the_link_answers_end_with_the_link() {
        // つながりが終わる（相手が閉じた・失敗した）と、相手の版・使える機能の答えは消える
        for (status, kind) in [
            (Status::Closed, EventKind::Closed),
            (Status::Failed, EventKind::Failed),
        ] {
            let s = linked(MARK_A, MARK_A);
            assert_eq!(s.common_features(), MARK_A);
            s.fail(status, kind, 0, "終わり".into());
            assert!(s.state().link.is_none(), "{status:?}");
            assert_eq!(s.common_features(), 0, "{status:?}");
        }
        let s = linked(MARK_A, MARK_A);
        s.close();
        assert!(s.state().link.is_none());
        assert_eq!(s.status(), Status::Closed);
    }

    // ───────── 送りの列の上限と置き換え ─────────

    /// 値・絵・元の絵を送れる（双方に印がある）つながり。
    fn open() -> Arc<Session> {
        let all = yolu_protocol::feature::MATERIAL_VALUES
            | yolu_protocol::feature::ORIGINAL_TEXTURES
            | yolu_protocol::feature::MATERIAL_REQUEST;
        linked(all, all)
    }

    fn model(generation: u32, name_len: usize) -> Message {
        Message::Model(Model {
            generation,
            name: "m".repeat(name_len),
            materials: Vec::new(),
            meshes: Vec::new(),
        })
    }

    /// 頂点が `vertices` 個のメッシュ 1 つだけのモデル（枠が大きい。名前の長さには上限があるので、大きさは頂点で作る）。
    fn heavy_model(generation: u32, vertices: usize) -> Message {
        Message::Model(Model {
            generation,
            name: "m".into(),
            materials: Vec::new(),
            meshes: vec![MeshData {
                key: "0".into(),
                name: "mesh".into(),
                skinned: false,
                positions: vec![[0.0; 3]; vertices],
                normals: Vec::new(),
                uv0: Vec::new(),
                submeshes: Vec::new(),
            }],
        })
    }

    /// 画素が `bytes` 以上（4 の倍数に切り上げ）の絵（1 行）。
    fn texture(generation: u32, material: u32, slot: &str, bytes: usize, fill: u8) -> Message {
        let width = bytes.div_ceil(4).max(1);
        Message::MaterialTexture(MaterialTexture {
            generation,
            material,
            slot: slot.into(),
            width: width as u32,
            height: 1,
            srgb: false,
            pixels: vec![fill; width * 4],
        })
    }

    fn original(generation: u32, material: u32, slot: &str, state: OriginalState, bytes: usize, fill: u8) -> Message {
        let width = bytes.div_ceil(4).max(1);
        Message::MaterialOriginal(MaterialOriginal {
            generation,
            material,
            slot: slot.into(),
            state,
            read: OriginalRead::File,
            compressed: false,
            width: width as u32,
            height: 1,
            srgb: true,
            pixels: if state == OriginalState::Image { vec![fill; width * 4] } else { Vec::new() },
            stamp: 7,
        })
    }

    fn values(generation: u32, material: u32, shader: &str, slots: &[(&str, SlotState)]) -> Message {
        Message::MaterialValues(MaterialValues {
            generation,
            material,
            kind: ValuesKind::LilToon,
            shader: shader.into(),
            source: String::new(),
            properties: Vec::new(),
            keywords: Vec::new(),
            slots: slots
                .iter()
                .map(|(name, state)| SlotTexture {
                    name: (*name).into(),
                    state: *state,
                    width: 1,
                    height: 1,
                })
                .collect(),
        })
    }

    fn materials_for(generation: u32, name: &str) -> Message {
        Message::Materials(MaterialsUpdate {
            generation,
            materials: vec![MaterialInfo {
                key: MaterialKey::Material {
                    name: name.into(),
                    asset: None,
                },
                shader: String::new(),
                textures: Vec::new(),
                routes: Vec::new(),
            }],
        })
    }

    /// 列に積んである命令を、積んだ順に読み戻す。
    fn queue(session: &Session) -> Vec<Message> {
        let o = lock(&session.outbox);
        o.frames
            .iter()
            .map(|q| {
                let kind = u16::from_le_bytes([q.frame[4], q.frame[5]]);
                Message::decode(kind, &q.frame[frame::HEADER_LEN..]).expect("積んだ枠は読める")
            })
            .collect()
    }

    fn kinds(session: &Session) -> Vec<Kind> {
        queue(session).iter().map(Message::kind).collect()
    }

    fn frame_len(message: &Message) -> usize {
        encode_message(message).len()
    }

    fn busy(result: Result<bool, EnqueueError>) -> bool {
        matches!(result, Err(EnqueueError::Busy))
    }

    #[test]
    fn the_queue_stops_at_the_limit_and_refuses_what_cannot_be_replaced() {
        let s = open();
        let one = model(1, 400);
        let n = frame_len(&one);
        s.set_outbox_limit(n * 2 + n / 2);
        assert!(s.try_enqueue(&one).unwrap());
        assert!(s.try_enqueue(&model(2, 400)).unwrap());
        assert_eq!(s.pending_bytes() as usize, 2 * n);
        // 3 つ目は上限を超える: 断る。何も変わらない（積んだ順も、量も）
        assert!(busy(s.try_enqueue(&model(3, 400))));
        assert!(busy(s.try_enqueue(&texture(1, 0, "_a", n / 2 + 1, 1))));
        assert_eq!(s.pending_bytes() as usize, 2 * n);
        assert_eq!(kinds(&s), vec![Kind::Model, Kind::Model]);
        // 積める量の目安は、上限から積んだ量を引いたもの
        assert_eq!(s.send_room() as usize, n / 2);
        // 制御の命令は、混んでいても積む（モデルを閉じる知らせが断られて、相手に閉じたことが伝わらないままにならない）
        assert!(s.try_enqueue(&Message::ModelClosed { generation: 2 }).unwrap());
        assert!(s.try_enqueue(&Message::TextureSetRemoved { set: 1 }).unwrap());
        assert_eq!(kinds(&s), vec![Kind::Model, Kind::Model, Kind::ModelClosed, Kind::TextureSetRemoved]);
        // 上限を上げれば（または相手が読んで空きができれば）同じ命令を積める
        s.set_outbox_limit(n * 10);
        assert!(s.try_enqueue(&model(3, 400)).unwrap());
    }

    /// このスレッドが、積む命令を枠にした回数（`try_enqueue` が枠にしてから断った分も数える）。
    fn encoded() -> usize {
        ENCODED.with(|c| c.get())
    }

    #[test]
    fn a_model_that_cannot_be_queued_is_refused_without_being_encoded() {
        // 混んでいる間、C# は 0.5 秒おきに同じモデルを送り直す。そのたびに数百 MiB を枠にして（中身と枠の 2 つの写し）捨てると、
        // エディターの主のスレッドが止まり、メモリが上限を超えて跳ねる。モデルは置き換えられず、大きさは枠にしなくても分かるので、
        // 入らないなら枠にする前に断る
        let s = open();
        let big = heavy_model(1, 300_000);
        assert!(s.try_enqueue(&texture(1, 0, "_a", 1000, 1)).unwrap());
        let held = s.pending_bytes() as usize;
        s.set_outbox_limit(held + 10);
        let before = encoded();
        for _ in 0..10 {
            assert!(busy(s.try_enqueue(&big)));
        }
        assert_eq!(encoded(), before, "断るモデルを枠にしていない");
        assert_eq!(s.pending_bytes() as usize, held, "何も変わらない");
        assert_eq!(kinds(&s), vec![Kind::MaterialTexture]);
        // 入るようになれば、1 回だけ枠にして積む
        s.set_outbox_limit(held + frame_len(&big) + 10);
        let before = encoded();
        assert!(s.try_enqueue(&big).unwrap());
        assert_eq!(encoded(), before + 1);
        assert_eq!(kinds(&s), vec![Kind::MaterialTexture, Kind::Model]);
    }

    #[test]
    fn a_model_over_the_message_limit_or_for_a_closed_link_is_refused_without_being_encoded() {
        let s = open();
        let big = heavy_model(1, 100_000);
        assert!(s.try_enqueue(&texture(1, 0, "_a", 1000, 1)).unwrap());
        s.set_outbox_limit(s.pending_bytes() as usize + 10);
        s.set_payload_limit(1 << 19);
        assert!(frame_len(&big) > 1 << 19);
        let before = encoded();
        // 1 つの命令の上限を超える大きさは、混んでいても「混んでいる」ではなく「大きすぎる」（送り直しても入らない）
        assert!(matches!(s.try_enqueue(&big), Err(EnqueueError::TooLarge(_))));
        assert_eq!(encoded(), before);
        // 閉じたつながりには積まない（混みの断りより先に）
        s.set_payload_limit(frame::MAX_PAYLOAD);
        s.close();
        assert!(!s.try_enqueue(&big).unwrap());
        assert_eq!(encoded(), before);
    }

    #[test]
    fn an_empty_queue_takes_one_command_larger_than_the_limit() {
        // 上限より大きい 1 つの命令（512 MiB までのモデル・256 MiB の元の絵）も、列が空なら積む。入らないままにならない
        let s = open();
        s.set_outbox_limit(100);
        assert_eq!(s.send_room(), u64::MAX, "空なら、どんなに大きくても入る");
        let big = model(1, 1000);
        assert!(s.try_enqueue(&big).unwrap());
        // 積んだあとは満杯: 次は積まない
        assert_eq!(s.send_room(), 0);
        assert!(busy(s.try_enqueue(&model(2, 10))));
        assert!(busy(s.try_enqueue(&texture(1, 0, "_a", 10, 1))));
    }

    #[test]
    fn a_frame_being_written_counts_until_it_is_written() {
        // 列から出して書いている途中の枠も、メモリに残っているので量に数える（書き終えるまで、次の大きな物を積ませない）
        let s = open();
        let one = model(1, 400);
        let n = frame_len(&one);
        s.set_outbox_limit(n + n / 2);
        assert!(s.try_enqueue(&one).unwrap());
        {
            let mut o = lock(&s.outbox);
            let frame = o.pop().expect("積んだ枠");
            assert_eq!(frame.len(), n);
        }
        assert_eq!(s.pending_bytes() as usize, n, "書いている途中の枠");
        assert!(busy(s.try_enqueue(&model(2, 400))));
        lock(&s.outbox).in_flight = 0; // 書き終えた
        assert_eq!(s.pending_bytes(), 0);
        assert!(s.try_enqueue(&model(2, 400)).unwrap());
    }

    #[test]
    fn a_newer_texture_of_the_same_slot_replaces_the_older_one_in_the_queue() {
        let s = open();
        assert!(s.try_enqueue(&texture(1, 0, "_a", 100, 1)).unwrap());
        assert!(s.try_enqueue(&texture(1, 0, "_b", 100, 2)).unwrap());
        assert!(s.try_enqueue(&texture(1, 1, "_a", 100, 3)).unwrap());
        assert!(s.try_enqueue(&texture(2, 0, "_a", 100, 4)).unwrap());
        let before = s.pending_bytes();
        // 同じ世代・マテリアル・スロットの新しい絵: 古いのを外して、最後に積む（ほかのスロット・マテリアル・世代は残る）
        assert!(s.try_enqueue(&texture(1, 0, "_a", 100, 9)).unwrap());
        assert_eq!(s.pending_bytes(), before, "置き換えで増えない");
        let q = queue(&s);
        assert_eq!(q.len(), 4);
        let order: Vec<(u32, u32, String, u8)> = q
            .iter()
            .map(|m| match m {
                Message::MaterialTexture(t) => (t.generation, t.material, t.slot.clone(), t.pixels[0]),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            order,
            vec![
                (1, 0, "_b".into(), 2),
                (1, 1, "_a".into(), 3),
                (2, 0, "_a".into(), 4),
                (1, 0, "_a".into(), 9),
            ]
        );
    }

    #[test]
    fn a_replacement_that_frees_the_room_is_taken_even_when_the_queue_is_full() {
        // 満杯でも、同じ物を置き換える新しい絵は（差し引きで収まるので）積む。置き換えられない大きな物は断る
        let s = open();
        let first = texture(1, 0, "_a", 1000, 1);
        let n = frame_len(&first);
        s.set_outbox_limit(n + 10);
        assert!(s.try_enqueue(&first).unwrap());
        assert!(busy(s.try_enqueue(&texture(1, 0, "_b", 1000, 2))), "別のスロットは置き換えられない");
        assert!(s.try_enqueue(&texture(1, 0, "_a", 1000, 3)).unwrap(), "同じスロットは置き換わる");
        assert_eq!(s.pending_bytes() as usize, n);
        match &queue(&s)[0] {
            Message::MaterialTexture(t) => assert_eq!(t.pixels[0], 3, "最新が残る"),
            other => panic!("{other:?}"),
        }
        // 断られたものは、前に積んだものを消さない
        assert!(busy(s.try_enqueue(&texture(1, 0, "_b", 1000, 2))));
        assert_eq!(queue(&s).len(), 1);
    }

    #[test]
    fn values_fit_a_full_queue_only_by_replacing_the_pictures_of_their_own_material() {
        // 満杯の列にある物が、同じ世代・マテリアルの送っていない絵なら、新しい値（「値なし」も）は、その絵を外した分で収まって積まれる
        // （受け手は、値が「来る」と言った絵だけを値のあとに受ける。値で知らせていない絵は、値のあとでは断られる）。元の絵・別のマテリアルの絵は、
        // 値に置き換えられないので、値は断られ、前に積んだ物は変わらない
        let none = |material: u32| {
            Message::MaterialValues(MaterialValues {
                generation: 1,
                material,
                kind: ValuesKind::None,
                shader: String::new(),
                source: String::new(),
                properties: Vec::new(),
                keywords: Vec::new(),
                slots: Vec::new(),
            })
        };
        let full_of = |ballast: &Message| {
            let s = open();
            s.set_outbox_limit(frame_len(ballast) + 1);
            assert!(s.try_enqueue(ballast).unwrap());
            s
        };
        // 同じマテリアルの絵: 外して、値が積まれる
        let s = full_of(&texture(1, 0, "_fill", 1000, 1));
        assert!(s.try_enqueue(&none(0)).unwrap(), "置き換えて空いた分で収まる");
        assert_eq!(kinds(&s), vec![Kind::MaterialValues]);
        // 別のマテリアルの絵・同じマテリアルの元の絵: 置き換えられないので断る
        for ballast in [texture(1, 1, "_fill", 1000, 1), original(1, 0, "_fill", OriginalState::Image, 1000, 1)] {
            let s = full_of(&ballast);
            let held = s.pending_bytes();
            assert!(busy(s.try_enqueue(&none(0))), "{:?} は値に置き換えられない", ballast.kind());
            assert_eq!(kinds(&s), vec![ballast.kind()], "断られても、前に積んだ物は残る");
            assert_eq!(s.pending_bytes(), held);
        }
    }

    #[test]
    fn an_original_with_pixels_replaces_older_originals_of_the_slot_but_a_status_does_not() {
        let s = open();
        assert!(s.try_enqueue(&original(1, 0, "_MainTex", OriginalState::Image, 100, 1)).unwrap());
        assert!(s.try_enqueue(&original(1, 1, "_MainTex", OriginalState::Image, 100, 2)).unwrap());
        // 画素の付いた新しい元の絵は、同じ世代・マテリアル・スロットの古いものを置き換える
        assert!(s.try_enqueue(&original(1, 0, "_MainTex", OriginalState::Image, 120, 3)).unwrap());
        let q = queue(&s);
        assert_eq!(q.len(), 2);
        // 画素なしの様子（印が同じ・読めない）は何も置き換えない（前の画素に頼ることがあるので、積んだ順も保つ）
        assert!(s.try_enqueue(&original(1, 0, "_MainTex", OriginalState::Cached, 0, 0)).unwrap());
        assert!(s.try_enqueue(&original(1, 1, "_MainTex", OriginalState::Unreadable, 0, 0)).unwrap());
        assert_eq!(queue(&s).len(), 4);
        // 画素の付いた絵は、前の画素なしの様子も置き換える（絵そのものを持つので、前の様子に頼らない）
        assert!(s.try_enqueue(&original(1, 0, "_MainTex", OriginalState::Image, 90, 4)).unwrap());
        let q = queue(&s);
        let shape: Vec<(u32, OriginalState)> = q
            .iter()
            .map(|m| match m {
                Message::MaterialOriginal(o) => (o.material, o.state),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                (1, OriginalState::Image),
                (1, OriginalState::Unreadable),
                (0, OriginalState::Image),
            ]
        );
        // 別の世代は置き換えない
        assert!(s.try_enqueue(&original(2, 0, "_MainTex", OriginalState::Image, 10, 5)).unwrap());
        assert_eq!(queue(&s).len(), 4);
    }

    #[test]
    fn a_newer_material_update_of_the_generation_replaces_the_older_one() {
        let s = open();
        assert!(s.try_enqueue(&model(1, 10)).unwrap());
        assert!(s.try_enqueue(&materials_for(1, "古い")).unwrap());
        assert!(s.try_enqueue(&materials_for(2, "別の世代")).unwrap());
        assert!(s.try_enqueue(&materials_for(1, "新しい")).unwrap());
        // モデルは残り、同じ世代の古い更新だけが無くなる。新しいものはモデルのあと（最後）に積まれる
        let names: Vec<String> = queue(&s)
            .iter()
            .filter_map(|m| match m {
                Message::Materials(u) => match &u.materials[0].key {
                    MaterialKey::Material { name, .. } => Some(name.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["別の世代".to_string(), "新しい".to_string()]);
        assert_eq!(kinds(&s)[0], Kind::Model);
    }

    #[test]
    fn a_model_is_never_replaced() {
        // モデルは置き換えない: 世代でモデルに結び付いた値・絵・元の絵が後ろに続くので、途中だけ捨てると、続きが宙に浮く
        let s = open();
        assert!(s.try_enqueue(&model(1, 10)).unwrap());
        assert!(s.try_enqueue(&values(1, 0, "A", &[])).unwrap());
        assert!(s.try_enqueue(&model(2, 10)).unwrap());
        assert_eq!(kinds(&s), vec![Kind::Model, Kind::MaterialValues, Kind::Model]);
    }

    #[test]
    fn newer_values_replace_the_older_values_and_pictures_of_the_material() {
        let s = open();
        assert!(s.try_enqueue(&model(1, 10)).unwrap());
        // マテリアル 0: 値（絵が来る）と絵 2 枚。マテリアル 1: 値と絵 1 枚
        assert!(s.try_enqueue(&values(1, 0, "A", &[("_a", SlotState::Follows), ("_b", SlotState::Follows)])).unwrap());
        assert!(s.try_enqueue(&texture(1, 0, "_a", 50, 1)).unwrap());
        assert!(s.try_enqueue(&texture(1, 0, "_b", 50, 2)).unwrap());
        assert!(s.try_enqueue(&values(1, 1, "B", &[("_a", SlotState::Follows)])).unwrap());
        assert!(s.try_enqueue(&texture(1, 1, "_a", 50, 3)).unwrap());
        // マテリアル 0 の新しい値（絵は 1 枚だけ来る、もう 1 つは入っていない）: 古い値と絵を全部外す。マテリアル 1 は残る
        assert!(s.try_enqueue(&values(1, 0, "A2", &[("_a", SlotState::Follows), ("_b", SlotState::Empty)])).unwrap());
        let q = queue(&s);
        let shape: Vec<String> = q
            .iter()
            .map(|m| match m {
                Message::Model(_) => "model".to_string(),
                Message::MaterialValues(v) => format!("values {} {}", v.material, v.shader),
                Message::MaterialTexture(t) => format!("texture {} {}", t.material, t.slot),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(shape, vec!["model", "values 1 B", "texture 1 _a", "values 0 A2"]);
        // その後に来る絵は、新しい値の後ろに付く
        assert!(s.try_enqueue(&texture(1, 0, "_a", 50, 4)).unwrap());
        assert_eq!(kinds(&s), vec![Kind::Model, Kind::MaterialValues, Kind::MaterialTexture, Kind::MaterialValues, Kind::MaterialTexture]);
    }

    #[test]
    fn values_that_say_unchanged_do_not_drop_the_picture_they_rely_on() {
        // 古い値で「絵が来る」と言った絵が列の中にある。新しい値が「前と同じ」と言うなら、その絵を捨てると、受け手は絵を持たないまま
        // 「前と同じ」を受ける。だから何も置き換えず、積んだ順のまま全部を残す
        let s = open();
        assert!(s.try_enqueue(&values(1, 0, "A", &[("_a", SlotState::Follows)])).unwrap());
        assert!(s.try_enqueue(&texture(1, 0, "_a", 50, 1)).unwrap());
        assert!(s.try_enqueue(&values(1, 0, "A2", &[("_a", SlotState::Unchanged)])).unwrap());
        assert_eq!(kinds(&s), vec![Kind::MaterialValues, Kind::MaterialTexture, Kind::MaterialValues]);
        // 列の中に絵が無い（もう相手に渡った）スロットの「前と同じ」は、古い値を置き換えてよい
        let s = open();
        assert!(s.try_enqueue(&values(1, 0, "A", &[("_a", SlotState::Unchanged)])).unwrap());
        assert!(s.try_enqueue(&values(1, 0, "A2", &[("_a", SlotState::Unchanged)])).unwrap());
        assert_eq!(kinds(&s), vec![Kind::MaterialValues]);
        match &queue(&s)[0] {
            Message::MaterialValues(v) => assert_eq!(v.shader, "A2"),
            other => panic!("{other:?}"),
        }
        // 絵を待たせているスロットと、前と同じと言うスロットが別なら、絵ごと置き換える
        let s = open();
        assert!(s.try_enqueue(&values(1, 0, "A", &[("_a", SlotState::Follows), ("_b", SlotState::Unchanged)])).unwrap());
        assert!(s.try_enqueue(&texture(1, 0, "_a", 50, 1)).unwrap());
        assert!(s.try_enqueue(&values(1, 0, "A2", &[("_a", SlotState::Follows), ("_b", SlotState::Unchanged)])).unwrap());
        assert_eq!(kinds(&s), vec![Kind::MaterialValues]);
        // 「値なし」も古い値と絵を置き換える
        let none = Message::MaterialValues(MaterialValues {
            generation: 1,
            material: 0,
            kind: ValuesKind::None,
            shader: String::new(),
            source: String::new(),
            properties: Vec::new(),
            keywords: Vec::new(),
            slots: Vec::new(),
        });
        assert!(s.try_enqueue(&none).unwrap());
        assert_eq!(kinds(&s), vec![Kind::MaterialValues]);
    }

    #[test]
    fn the_queue_never_passes_the_limit_whatever_is_sent() {
        // どんな順に送っても、積んだ量は（空の列へ入れた上限より大きい 1 つを除いて）上限を超えない
        let s = open();
        let limit = 5_000;
        s.set_outbox_limit(limit);
        let mut accepted = 0;
        let mut refused = 0;
        for i in 0..400u32 {
            let message = match i % 7 {
                0 => texture(1, i % 3, if i % 2 == 0 { "_a" } else { "_b" }, 700 + (i as usize % 5) * 100, i as u8),
                1 => original(1, i % 4, "_MainTex", OriginalState::Image, 900, i as u8),
                2 => values(1, i % 3, "S", &[("_a", SlotState::Follows)]),
                3 => materials_for(1, "m"),
                4 => model(i, 300 + (i as usize % 4) * 100),
                5 => original(1, i % 4, "_MainTex", OriginalState::Cached, 0, 0),
                _ => Message::ModelClosed { generation: i },
            };
            let control = matches!(message, Message::ModelClosed { .. });
            match s.try_enqueue(&message) {
                Ok(true) => accepted += 1,
                Err(EnqueueError::Busy) => {
                    refused += 1;
                    assert!(!control);
                }
                other => panic!("{other:?}"),
            }
            // 制御の命令は小さい（上限をはみ出すのは、その分だけ）
            let controls = queue(&s).iter().filter(|m| matches!(m, Message::ModelClosed { .. })).count();
            assert!(
                s.pending_bytes() as usize <= limit + controls * frame_len(&Message::ModelClosed { generation: 0 }),
                "{} バイト（上限 {limit}）",
                s.pending_bytes()
            );
            if i % 11 == 10 {
                // 相手が少し読む
                let mut o = lock(&s.outbox);
                o.pop();
                o.in_flight = 0;
            }
        }
        assert!(accepted > 0 && refused > 0, "積めたもの {accepted}・断ったもの {refused}");
    }

    #[test]
    fn a_closing_or_dead_queue_takes_nothing_and_close_keeps_the_byte_count_right() {
        let s = open();
        assert!(s.try_enqueue(&texture(1, 0, "_a", 100, 1)).unwrap());
        s.close();
        // Bye が積まれ、閉じたあとは何も積まない（混んでいる、ではなく積まない）
        assert!(!s.try_enqueue(&texture(1, 0, "_b", 100, 1)).unwrap());
        let held = s.pending_bytes() as usize;
        assert_eq!(held, frame_len(&texture(1, 0, "_a", 100, 1)) + frame_len(&Message::Bye));
        let s = open();
        assert!(s.try_enqueue(&texture(1, 0, "_a", 100, 1)).unwrap());
        s.fail(Status::Closed, EventKind::Closed, 0, "終わり".into());
        assert_eq!(s.pending_bytes(), 0, "失敗で列は空になる");
        assert!(!s.try_enqueue(&texture(1, 0, "_a", 100, 1)).unwrap());
    }
}
