//! Live Link で、読むスレッドから画面のスレッドへ渡す命令の列の、積んでいる量の上限。
//! 画面のスレッドがフレームを回していない間（隠れた窓・重い処理の最中）も、読むスレッドは Unity の命令を受けて列へ積む。上限が無いと、
//! モデル・テクスチャ・元の絵のような大きな命令が、その間の分だけ無制限に溜まる（ポーズは `livelink_pose` が最新だけを持つので別）。
//!
//! 決まり:
//! - 積んでいる命令の大きさの合計（[`message_bytes`]）が上限 [`MAX_QUEUED_BYTES`] に達したら、読むスレッドは次の命令を**読まずに待つ**。
//!   読まれない分は、ソケットとパイプが詰まって Unity 側の書くスレッドが待つので、命令は捨てず、順番も変わらない。画面のスレッドが取り出して量が
//!   上限を下回ったら、読むのを続ける。積んだ量が上限を超えるのは、上限の手前で読んだ 1 つの命令の分まで（その 1 つが上限より大きくても読む。
//!   待ち続けて、巨大な 1 つのモデルが永久に入れなくならない）。
//! - 古い同じ種類の命令を捨てる形にしないのは、モデルのあとに続く元の絵・値・テクスチャが、世代と材質の番号でそのモデルに結び付いていて、
//!   途中だけを捨てると、足りない物を Unity に頼み直す道と世代の食い違いの手当てが要るため。待たせれば、何も失わず順番も変わらない。
//!   失うのは、待っているあいだの Unity からの新しい命令が（画面のスレッドが取り出すまで）届かないことだけ。
//! - 保証の射程: この上限で守れるのは、スタンドアロン側のメモリ。読むのを止めたあいだに Unity が送る命令は、ブリッジの送りの列（`yolu-bridge` の
//!   `Session` の送り待ち）に溜まるが、そちらにも同じ大きさの上限（`MAX_OUTBOX_BYTES`）がある。ブリッジは、列の中の最新だけが意味を持つ命令（同じスロットの絵・
//!   値・マテリアルの更新・画素の付いた元の絵）を新しいもので置き換え、置き換えられない命令（モデルなど）が上限を超えるときは「混んでいる」と断って、
//!   Unity が少し後に送り直す。読む側が戻れば、列の中の命令は積んだ順に届く。
//! - 待っているあいだに、つながりが終わった（今のつながりでなくなった）・列の持ち主が落ちたときは、待ちをやめて読むスレッドを終える。

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use yolu_protocol::{Message, Pose};

/// 積んでおく命令の大きさの合計の上限（バイト）。ポーズの溜め場（`livelink_pose::MAX_POSE_BYTES`）と同じ量。
pub const MAX_QUEUED_BYTES: usize = 256 << 20;

/// 待ちの間に「まだ待つ必要があるか」を見直す間隔。
const RECHECK: Duration = Duration::from_millis(100);

#[derive(Default)]
struct State {
    bytes: usize,
    closed: bool,
}

/// 列に積んでいる量の帳簿（読むスレッドが足し、画面のスレッドが引く）。
pub struct Backlog {
    state: Mutex<State>,
    room: Condvar,
    limit: Mutex<usize>,
}

impl Backlog {
    pub fn new() -> Arc<Backlog> {
        Backlog::with_limit(MAX_QUEUED_BYTES)
    }

    pub fn with_limit(limit: usize) -> Arc<Backlog> {
        Arc::new(Backlog {
            state: Mutex::new(State::default()),
            room: Condvar::new(),
            limit: Mutex::new(limit),
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 上限を変える（試験用）。
    pub fn set_limit(&self, limit: usize) {
        *self.limit.lock().unwrap_or_else(|e| e.into_inner()) = limit;
        self.room.notify_all();
    }

    pub fn limit(&self) -> usize {
        *self.limit.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 積んでいる量。
    pub fn queued(&self) -> usize {
        self.state().bytes
    }

    /// 次の命令を読む前に呼ぶ。積んでいる量が上限に達していれば、画面のスレッドが取り出して下回るまで待つ。
    /// `keep` は待つあいだ定期的に見て、false（つながりが終わった）なら待ちをやめる。読んでよければ true。
    pub fn wait_for_room(&self, keep: impl Fn() -> bool) -> bool {
        let mut state = self.state();
        loop {
            if state.closed {
                return false;
            }
            if state.bytes < self.limit() {
                return true;
            }
            if !keep() {
                return false;
            }
            state = self
                .room
                .wait_timeout(state, RECHECK)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// 命令を列へ積む前に、その大きさを足す。
    pub fn add(&self, bytes: usize) {
        let mut state = self.state();
        state.bytes = state.bytes.saturating_add(bytes);
    }

    /// 画面のスレッドが命令を列から取り出したので、その大きさを引く（待っている読むスレッドを起こす）。
    pub fn release(&self, bytes: usize) {
        let mut state = self.state();
        state.bytes = state.bytes.saturating_sub(bytes);
        drop(state);
        self.room.notify_all();
    }

    /// 列の持ち主が落ちた: 待っている読むスレッドを終わらせる。
    pub fn close(&self) {
        self.state().closed = true;
        self.room.notify_all();
    }
}

/// 命令がメモリで占める大きさの見積もり（バイト）。大きくなる種類（モデル・テクスチャ・元の絵・ポーズ）は中身の量、
/// 小さい種類は一定の量。
pub fn message_bytes(message: &Message) -> usize {
    const SMALL: usize = 1024;
    match message {
        Message::Model(model) => usize::try_from(model.payload_len()).unwrap_or(usize::MAX),
        Message::Pose(pose) => pose_bytes(pose),
        Message::MaterialTexture(t) => t.pixels.len().saturating_add(SMALL),
        Message::MaterialOriginal(o) => o.pixels.len().saturating_add(SMALL),
        _ => SMALL,
    }
}

/// ポーズの大きさ（位置と法線のバイト）。
pub fn pose_bytes(pose: &Pose) -> usize {
    pose.meshes
        .iter()
        .map(|m| (m.positions.len() + m.normals.len()) * 12)
        .fold(0usize, usize::saturating_add)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Instant;

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "待ちすぎ: {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_reader_waits_at_the_limit_and_resumes_when_the_screen_takes_commands_out() {
        let backlog = Backlog::with_limit(100);
        assert!(backlog.wait_for_room(|| true));
        backlog.add(60);
        assert!(backlog.wait_for_room(|| true), "上限の手前なら読む");
        backlog.add(60);
        assert_eq!(backlog.queued(), 120, "手前で読んだ 1 つの分は、上限を超えて積める");
        let waiting = Arc::new(AtomicBool::new(true));
        let thread = {
            let (backlog, waiting) = (backlog.clone(), waiting.clone());
            std::thread::spawn(move || {
                let ok = backlog.wait_for_room(|| true);
                waiting.store(false, Ordering::SeqCst);
                ok
            })
        };
        std::thread::sleep(Duration::from_millis(150));
        assert!(waiting.load(Ordering::SeqCst), "上限に達している間は読まない");
        backlog.release(60);
        assert_eq!(backlog.queued(), 60);
        wait_for("読むのが再開する", || !waiting.load(Ordering::SeqCst));
        assert!(thread.join().unwrap());
    }

    #[test]
    fn one_command_larger_than_the_limit_is_still_read_when_the_queue_is_empty() {
        let backlog = Backlog::with_limit(10);
        assert!(backlog.wait_for_room(|| true));
        backlog.add(1000);
        let started = Instant::now();
        let blocked = Arc::new(AtomicBool::new(false));
        let thread = {
            let (backlog, blocked) = (backlog.clone(), blocked.clone());
            std::thread::spawn(move || {
                let ok = backlog.wait_for_room(|| true);
                blocked.store(true, Ordering::SeqCst);
                ok
            })
        };
        std::thread::sleep(Duration::from_millis(100));
        assert!(!blocked.load(Ordering::SeqCst));
        backlog.release(1000);
        assert!(thread.join().unwrap());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn the_wait_ends_when_the_link_ends_or_the_owner_is_gone() {
        let backlog = Backlog::with_limit(10);
        backlog.add(10);
        let alive = Arc::new(AtomicBool::new(true));
        let thread = {
            let (backlog, alive) = (backlog.clone(), alive.clone());
            std::thread::spawn(move || backlog.wait_for_room(|| alive.load(Ordering::SeqCst)))
        };
        std::thread::sleep(Duration::from_millis(50));
        alive.store(false, Ordering::SeqCst);
        assert!(!thread.join().unwrap(), "つながりが終われば、待たずに読むスレッドを終える");

        let thread = {
            let backlog = backlog.clone();
            std::thread::spawn(move || backlog.wait_for_room(|| true))
        };
        std::thread::sleep(Duration::from_millis(50));
        backlog.close();
        assert!(!thread.join().unwrap(), "持ち主が落ちれば、待ちを解く");
        assert!(!backlog.wait_for_room(|| true), "閉じたあとは読まない");
    }

    #[test]
    fn releasing_more_than_was_added_never_underflows() {
        let backlog = Backlog::with_limit(10);
        backlog.add(5);
        backlog.release(50);
        assert_eq!(backlog.queued(), 0);
        let counter = AtomicUsize::new(0);
        assert!(backlog.wait_for_room(|| {
            counter.fetch_add(1, Ordering::SeqCst);
            true
        }));
        assert_eq!(counter.load(Ordering::SeqCst), 0, "余裕があれば確かめも要らない");
    }

    #[test]
    fn big_commands_are_counted_by_their_content_and_small_ones_by_a_constant() {
        use yolu_protocol::{MaterialTexture, MeshPose};
        let texture = Message::MaterialTexture(MaterialTexture {
            generation: 1,
            material: 0,
            slot: "_MainTex".into(),
            width: 512,
            height: 512,
            srgb: true,
            pixels: vec![0; 512 * 512 * 4],
        });
        assert!(message_bytes(&texture) >= 512 * 512 * 4);
        let pose = Message::Pose(Pose {
            generation: 1,
            meshes: vec![MeshPose {
                mesh: 0,
                positions: vec![[0.0; 3]; 1000],
                normals: vec![[0.0; 3]; 1000],
            }],
        });
        assert_eq!(message_bytes(&pose), 2000 * 12);
        assert!(message_bytes(&Message::Bye) > 0);
        assert!(message_bytes(&Message::Bye) < 1 << 16);
    }
}
