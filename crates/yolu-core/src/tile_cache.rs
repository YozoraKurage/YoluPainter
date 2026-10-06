//! タイルの中身の持ち主（`TileCell`）と、メモリの上限を超えた分をディスクへ逃がす係。
//!
//! - 層・取り消しの写し・保存の写しは、同じ `Arc<TileCell>` を共有する。cell の中身は作ったら変えない（書くときは中身を取り出すか
//!   複製して、新しい cell を作る）。なのでディスクに書いた写しは、cell が生きている間ずっと正しい。
//! - 中身は「メモリにある」「ディスクにある」「両方」のどれか。読むと、メモリに無ければディスクから読み戻してメモリに置く
//!   （その間は cell の錠を持つ。メモリにあれば読み手どうしは並べて読める）。
//! - 逃がす係は全部の cell の弱い参照を持ち、メモリにある中身の合計が上限を超えると、最後に使った時刻の古い順に、ディスクへ書いて
//!   （まだ書いていなければ）メモリの中身を手放す。今だれかが読んでいる中身（`Arc` の参照が cell の外にもある）は飛ばす。
//!   書くのは裏のスレッド 1 本で、読み手は待たない（逃がす途中の中身はメモリにもあるので、それを読む）。
//! - キャッシュのファイルはプロセスごと・置き場所ごとに 1 つ（`yolupainter-cache-<pid>-<乱数>.bin`）。Unix は開いたらすぐ名前を消し、
//!   Windows は閉じたら消える印（`FILE_FLAG_DELETE_ON_CLOSE`）で開く（落ちても残らない）。中身は生のバイト（圧縮しない）、位置は
//!   タイルの大きさの区画で、空いた区画を使い回す。
//! - ディスクから読めないときは、その cell を読む口が `CoreError::TileUnreadable` を返す（透明のタイルにしない）。

use std::collections::HashMap;
use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, RwLock, Weak};
use std::time::Duration;

use crate::error::CoreError;

/// 設定（アプリが設定と RAM から決めて入れる）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheSettings {
    /// 逃がすか。切っていれば何も逃がさない（もう逃がした分は、読むときにメモリへ戻る）。
    pub enabled: bool,
    /// キャッシュのファイルを置くフォルダ（None は OS の一時フォルダ）。変えたら、次に逃がす分から新しいフォルダのファイルへ書く。
    pub folder: Option<PathBuf>,
    /// メモリに置く中身の上限（バイト）。超えた分を逃がす。
    pub memory_limit: u64,
    /// ディスクに置く中身の上限（バイト）。満杯なら逃がさない。
    pub disk_limit: u64,
}

/// 今の様子（アプリの予算の計算と試験が読む）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStatus {
    /// メモリにある中身の合計。
    pub resident_bytes: u64,
    /// ディスクの区画に置いた中身の合計。
    pub disk_bytes: u64,
    /// ディスクへ書けなかった（満杯・外れた）。設定を入れ直すまで逃がさない（ディスクに置けるのは今の `disk_bytes` まで）。
    pub write_failed: bool,
    /// ディスクから読めなかった回数（プロセスの通し）。
    pub read_failures: u64,
}

/// 今の設定を入れる（同じ値なら何もしない）。入なら裏の書き手を起こす。
pub fn configure(settings: &CacheSettings) {
    global().configure(settings);
}

/// 今の様子。
pub fn status() -> CacheStatus {
    global().status()
}

/// ディスクから読めなかった回数（プロセスの通し。毎フレーム見てよい軽さ）。
pub fn read_failures() -> u64 {
    global().read_failures.load(Relaxed)
}

/// フォルダに前のプロセスが残したキャッシュのファイル（`yolupainter-cache-<pid>-<16 桁>.bin`。自分の pid のものは除く）を消す。
/// 消した数。フォルダを読めなければ 0。
pub fn remove_stale_files(folder: &Path) -> usize {
    let own = format!("{FILE_PREFIX}{}-", std::process::id());
    let Ok(entries) = std::fs::read_dir(folder) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if is_cache_file_name(name)
            && !name.starts_with(&own)
            && std::fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// メモリにある中身を、合計が `target` バイト以下になるまで、古い順に今すぐ逃がす（設定の入切によらない。試験と計測の口）。
/// 逃がした cell の数。
#[doc(hidden)]
pub fn evict_now(target: u64) -> usize {
    global().evict_pass(target).evicted
}

/// このプロセスが作ったキャッシュのファイルの道（試験の口。Unix では開いた直後に名前を消しているので、道にはもう何も無い）。
#[doc(hidden)]
pub fn created_files() -> Vec<PathBuf> {
    lock(&global().store).created.clone()
}

const FILE_PREFIX: &str = "yolupainter-cache-";

/// `yolupainter-cache-<数字>-<16 桁の 16 進>.bin` か。
fn is_cache_file_name(name: &str) -> bool {
    let Some(rest) = name
        .strip_prefix(FILE_PREFIX)
        .and_then(|r| r.strip_suffix(".bin"))
    else {
        return false;
    };
    let Some((pid, random)) = rest.split_once('-') else {
        return false;
    };
    !pid.is_empty()
        && pid.bytes().all(|b| b.is_ascii_digit())
        && random.len() == 16
        && random.bytes().all(|b| b.is_ascii_hexdigit())
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ───────── cell ─────────

/// 1 枚のタイルの中身の持ち主。
pub(crate) struct TileCell {
    len: usize,
    /// 最後に使った時刻（`Cache::clock` の値）。
    last_used: AtomicU64,
    state: RwLock<CellState>,
    cache: &'static Cache,
    /// 逃がす係の一覧に入れ、メモリの合計に数えるか（別の予算で持つ効果の評価のキャッシュの中身は入れない。逃がさない）。
    listed: bool,
}

#[derive(Default)]
struct CellState {
    /// メモリの中身。
    mem: Option<Arc<Vec<u8>>>,
    /// ディスクの写し（中身は変えないので、書いた後はずっと正しい）。
    disk: Option<DiskCopy>,
    /// 最後にディスクから読もうとして読めなかった。
    lost: bool,
    /// 試験: ディスクから読むのを必ず失敗させる。
    fail_reads: bool,
}

impl std::fmt::Debug for TileCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.state.read().unwrap_or_else(|e| e.into_inner());
        f.debug_struct("TileCell")
            .field("len", &self.len)
            .field("resident", &s.mem.is_some())
            .field("on_disk", &s.disk.is_some())
            .finish()
    }
}

impl TileCell {
    /// 中身を持つ新しい cell（全体の逃がす係に登録する）。
    pub(crate) fn new(bytes: Vec<u8>) -> Arc<TileCell> {
        Self::new_in(global(), bytes).0
    }

    /// 新しい cell と、その中身を読んでいる参照（読んでいる間は逃がさない）。
    pub(crate) fn new_pinned(bytes: Vec<u8>) -> (Arc<TileCell>, Arc<Vec<u8>>) {
        Self::new_in(global(), bytes)
    }

    /// 逃がさない cell（効果の評価のキャッシュの中身。キャッシュは自分の予算で持ち、作り直せる）。メモリの合計にも数えない。
    pub(crate) fn kept(bytes: Vec<u8>) -> Arc<TileCell> {
        Self::build(global(), bytes, false).0
    }

    fn new_in(cache: &'static Cache, bytes: Vec<u8>) -> (Arc<TileCell>, Arc<Vec<u8>>) {
        Self::build(cache, bytes, true)
    }

    fn build(cache: &'static Cache, bytes: Vec<u8>, listed: bool) -> (Arc<TileCell>, Arc<Vec<u8>>) {
        let len = bytes.len();
        let mem = Arc::new(bytes);
        let cell = Arc::new(TileCell {
            len,
            last_used: AtomicU64::new(cache.clock.load(Relaxed)),
            state: RwLock::new(CellState {
                mem: Some(mem.clone()),
                ..CellState::default()
            }),
            cache,
            listed,
        });
        if listed {
            cache.register(&cell);
            cache.add_resident(len as u64);
        }
        (cell, mem)
    }

    /// 中身のバイト数（メモリに無くても分かる）。
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// 使った時刻を今にする。時計はときどきしか進まないので、書くのは進んだ後の最初の 1 度だけ（読み手どうしが同じ cell の行を
    /// 書き合わない）。
    #[inline]
    fn touch(&self) {
        let now = self.cache.clock.load(Relaxed);
        if self.last_used.load(Relaxed) < now {
            self.last_used.store(now, Relaxed);
        }
    }

    /// 中身を読む（メモリに無ければディスクから読み戻す）。返した参照を持っている間は逃がさない。
    pub(crate) fn bytes(&self) -> Result<Arc<Vec<u8>>, CoreError> {
        self.touch();
        if let Some(m) = &self.state.read().unwrap_or_else(|e| e.into_inner()).mem {
            return Ok(m.clone());
        }
        self.load()
    }

    /// 中身を借りて f に渡す（メモリにあれば参照を増やさない。1 画素を読む口向け）。
    #[inline]
    pub(crate) fn with<R>(&self, f: impl FnOnce(&[u8]) -> R) -> Result<R, CoreError> {
        self.touch();
        {
            let s = self.state.read().unwrap_or_else(|e| e.into_inner());
            if let Some(m) = &s.mem {
                return Ok(f(m));
            }
        }
        let m = self.load()?;
        Ok(f(&m))
    }

    /// ディスクから読み戻してメモリに置く（cell の錠を持ったまま）。
    fn load(&self) -> Result<Arc<Vec<u8>>, CoreError> {
        let mut s = self.state.write().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = &s.mem {
            return Ok(m.clone());
        }
        let read = match (&s.disk, s.fail_reads) {
            (Some(copy), false) => copy.read(self.len),
            _ => Err(io::Error::other("読めない区画")),
        };
        match read {
            Ok(bytes) => {
                let m = Arc::new(bytes);
                s.mem = Some(m.clone());
                s.lost = false;
                drop(s);
                self.cache.add_resident(self.len as u64);
                Ok(m)
            }
            Err(_) => {
                s.lost = true;
                drop(s);
                self.cache.read_failures.fetch_add(1, Relaxed);
                Err(CoreError::TileUnreadable)
            }
        }
    }

    /// 書くための中身。この cell をほかが持っていなければ中身を取り出し（複製しない）、共有していれば複製する
    /// （今までの写しで書く決まりと同じ結果）。`pinned` はこの cell から読んだ中身。
    pub(crate) fn detach(cell: Arc<TileCell>, pinned: Arc<Vec<u8>>) -> Vec<u8> {
        if Arc::strong_count(&cell) == 1 {
            let mut s = cell.state.write().unwrap_or_else(|e| e.into_inner());
            if let Some(m) = s.mem.take() {
                drop(s);
                drop(m);
                if cell.listed {
                    cell.cache.sub_resident(cell.len as u64);
                }
            }
        }
        drop(cell);
        Arc::try_unwrap(pinned).unwrap_or_else(|a| (*a).clone())
    }

    /// 書くための中身（`detach` の、まだ読んでいない cell の形）。
    #[cfg(test)]
    pub(crate) fn into_vec(cell: Arc<TileCell>) -> Result<Vec<u8>, CoreError> {
        let pinned = cell.bytes()?;
        Ok(Self::detach(cell, pinned))
    }

    /// ディスクから読めなかったことがあるか（読み直して読めたら消える）。
    pub(crate) fn is_lost(&self) -> bool {
        self.state.read().unwrap_or_else(|e| e.into_inner()).lost
    }

    /// メモリにあるか・ディスクにあるか（試験と計測の口）。
    pub(crate) fn residency(&self) -> (bool, bool) {
        let s = self.state.read().unwrap_or_else(|e| e.into_inner());
        (s.mem.is_some(), s.disk.is_some())
    }

    /// 試験: これから先、ディスクから読むのを失敗させる。
    pub(crate) fn fail_reads_for_test(&self) {
        self.state
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .fail_reads = true;
    }

    /// 今すぐ逃がす（読んでいる最中なら何もしない）。逃がしたら true。
    pub(crate) fn evict(&self) -> bool {
        self.cache.evict_cell(self, true).unwrap_or(false)
    }

    /// 2 つが同じ cell か。
    #[inline]
    pub(crate) fn ptr_eq(a: &Arc<TileCell>, b: &Arc<TileCell>) -> bool {
        Arc::ptr_eq(a, b)
    }
}

impl Drop for TileCell {
    fn drop(&mut self) {
        let s = self.state.get_mut().unwrap_or_else(|e| e.into_inner());
        if s.mem.take().is_some() && self.listed {
            self.cache.sub_resident(self.len as u64);
        }
        if let Some(copy) = s.disk.take() {
            self.cache.release_slot(copy.slot, self.len);
        }
    }
}

// ───────── 逃がす係 ─────────

const SHARDS: usize = 16;

/// 入のときに時計を進める間隔（最後に使った時刻の細かさ。cell の時刻を書くのは、この間に 1 度まで）。
const CLOCK_TICK: Duration = Duration::from_millis(250);

/// 弱い参照の一覧のかたまり（スレッドごとに散らして、登録の錠を取り合わない）。
#[derive(Default)]
struct Shard {
    cells: Vec<Weak<TileCell>>,
    /// この数に届いたら死んだ参照を掃除する（掃除の後の数の 2 倍。登録 1 回あたりの手間を一定にする）。
    sweep_at: usize,
}

/// ディスクの区画の位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    file: usize,
    offset: u64,
}

/// cell のディスクの写し（区画と、そのファイル。読むときに係の錠を取らない）。
struct DiskCopy {
    slot: Slot,
    file: Arc<File>,
}

impl DiskCopy {
    fn read(&self, len: usize) -> io::Result<Vec<u8>> {
        let mut bytes = vec![0u8; len];
        read_at(&self.file, &mut bytes, self.slot.offset)?;
        Ok(bytes)
    }
}

struct CacheFile {
    file: Arc<File>,
    /// ファイルの末尾（新しい区画はここに足す）。
    end: u64,
    /// 空いた区画（大きさごと）。
    free: HashMap<usize, Vec<u64>>,
    /// 使っている区画の数。
    live: u64,
}

struct Store {
    folder: PathBuf,
    disk_limit: u64,
    /// 使っている区画のバイト数の合計。
    used: u64,
    files: Vec<Option<CacheFile>>,
    /// 新しい区画を置くファイル（今のフォルダの物）。
    current: Option<usize>,
    /// 書けなかった（満杯・外れた）。設定を入れ直すまで逃がさない。
    failed: bool,
    /// 作ったファイルの道（試験の口）。
    created: Vec<PathBuf>,
    /// 前の残りを掃除したフォルダ。
    swept: Vec<PathBuf>,
}

impl Store {
    fn new() -> Store {
        Store {
            folder: std::env::temp_dir(),
            disk_limit: 0,
            used: 0,
            files: Vec::new(),
            current: None,
            failed: false,
            created: Vec::new(),
            swept: Vec::new(),
        }
    }

    /// 区画を 1 つ取る。満杯・書けなくなった後なら None、ファイルを作れなければ誤り。
    fn allocate(&mut self, len: usize) -> io::Result<Option<(Slot, Arc<File>)>> {
        if self.failed || self.used.saturating_add(len as u64) > self.disk_limit {
            return Ok(None);
        }
        let index = match self.current {
            Some(i) => i,
            None => {
                if !self.swept.contains(&self.folder) {
                    remove_stale_files(&self.folder);
                    self.swept.push(self.folder.clone());
                }
                let (file, path) = open_cache_file(&self.folder)?;
                self.created.push(path);
                let entry = CacheFile {
                    file: Arc::new(file),
                    end: 0,
                    free: HashMap::new(),
                    live: 0,
                };
                let i = match self.files.iter().position(Option::is_none) {
                    Some(i) => {
                        self.files[i] = Some(entry);
                        i
                    }
                    None => {
                        self.files.push(Some(entry));
                        self.files.len() - 1
                    }
                };
                self.current = Some(i);
                i
            }
        };
        let f = self.files[index].as_mut().expect("今のファイル");
        let offset = match f.free.get_mut(&len).and_then(Vec::pop) {
            Some(o) => o,
            None => {
                let o = f.end;
                f.end += len as u64;
                o
            }
        };
        f.live += 1;
        self.used += len as u64;
        Ok(Some((
            Slot {
                file: index,
                offset,
            },
            f.file.clone(),
        )))
    }

    fn release(&mut self, slot: Slot, len: usize) {
        self.used = self.used.saturating_sub(len as u64);
        let current = self.current;
        let Some(f) = self.files.get_mut(slot.file).and_then(Option::as_mut) else {
            return;
        };
        f.live = f.live.saturating_sub(1);
        if f.live == 0 && current != Some(slot.file) {
            // 今のフォルダでないファイルは、使う区画が無くなったら閉じる（閉じれば消える）
            self.files[slot.file] = None;
        } else {
            f.free.entry(len).or_default().push(slot.offset);
        }
    }
}

/// 逃がす作業 1 回の結果。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Pass {
    pub evicted: usize,
    /// ディスクが満杯・書けなかった cell があった（その cell は残し、ディスクに写しのある cell だけを続けて手放した）。
    pub full: bool,
}

pub(crate) struct Cache {
    resident: AtomicU64,
    /// 時計（入の間は `CLOCK_TICK` ごとと、逃がす作業の回ごとに 1 進む）。cell の最後に使った時刻はこの値。
    clock: AtomicU64,
    /// メモリの上限（切なら u64::MAX）。
    limit: AtomicU64,
    shards: Vec<Mutex<Shard>>,
    next_shard: AtomicUsize,
    store: Mutex<Store>,
    /// 逃がす作業は 1 度に 1 つ。
    pass: Mutex<()>,
    read_failures: AtomicU64,
    /// 裏の書き手を起こす印。
    wake: (Mutex<bool>, Condvar),
    /// 起こすのを頼んで、まだ書き手が受けていない。
    requested: AtomicBool,
    /// 裏の書き手がいるか（全体の係だけ）。
    worker: AtomicBool,
    settings: Mutex<Option<CacheSettings>>,
}

fn global() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Cache::new)
}

thread_local! {
    static SHARD: std::cell::Cell<usize> = const { std::cell::Cell::new(usize::MAX) };
}

impl Cache {
    fn new() -> Cache {
        Cache {
            resident: AtomicU64::new(0),
            clock: AtomicU64::new(1),
            limit: AtomicU64::new(u64::MAX),
            shards: (0..SHARDS).map(|_| Mutex::default()).collect(),
            next_shard: AtomicUsize::new(0),
            store: Mutex::new(Store::new()),
            pass: Mutex::new(()),
            read_failures: AtomicU64::new(0),
            wake: (Mutex::new(false), Condvar::new()),
            requested: AtomicBool::new(false),
            worker: AtomicBool::new(false),
            settings: Mutex::new(None),
        }
    }

    fn configure(&'static self, settings: &CacheSettings) {
        {
            let mut last = lock(&self.settings);
            if last.as_ref() == Some(settings) {
                return;
            }
            *last = Some(settings.clone());
        }
        {
            let mut store = lock(&self.store);
            store.disk_limit = settings.disk_limit;
            let folder = settings.folder.clone().unwrap_or_else(std::env::temp_dir);
            if store.folder != folder {
                store.folder = folder;
                store.current = None;
                // 前のフォルダのファイルは、使う区画が無ければ閉じる
                for f in store.files.iter_mut() {
                    if f.as_ref().is_some_and(|f| f.live == 0) {
                        *f = None;
                    }
                }
            }
            store.failed = false;
        }
        let limit = if settings.enabled {
            settings.memory_limit
        } else {
            u64::MAX
        };
        self.limit.store(limit, Relaxed);
        if settings.enabled {
            self.start_worker();
            if self.resident.load(Relaxed) > limit {
                self.request();
            }
        }
    }

    fn status(&self) -> CacheStatus {
        let store = lock(&self.store);
        CacheStatus {
            resident_bytes: self.resident.load(Relaxed),
            disk_bytes: store.used,
            write_failed: store.failed,
            read_failures: self.read_failures.load(Relaxed),
        }
    }

    fn register(&self, cell: &Arc<TileCell>) {
        let shard = SHARD.with(|s| {
            if s.get() == usize::MAX {
                s.set(self.next_shard.fetch_add(1, Relaxed) % SHARDS);
            }
            s.get()
        });
        let mut s = lock(&self.shards[shard]);
        if s.cells.len() >= s.sweep_at {
            s.cells.retain(|w| w.strong_count() > 0);
            s.sweep_at = (s.cells.len() * 2).max(1024);
        }
        s.cells.push(Arc::downgrade(cell));
    }

    fn add_resident(&self, bytes: u64) {
        let now = self.resident.fetch_add(bytes, Relaxed) + bytes;
        if now > self.limit.load(Relaxed) {
            self.request();
        }
    }

    fn sub_resident(&self, bytes: u64) {
        self.resident.fetch_sub(bytes, Relaxed);
    }

    fn request(&self) {
        // 書き手がいない・もう頼んである（起こすまで待つ）なら錠を取らない
        if !self.worker.load(Relaxed) || self.requested.swap(true, Relaxed) {
            return;
        }
        let (flag, cv) = &self.wake;
        let mut f = lock(flag);
        if !*f {
            *f = true;
            cv.notify_one();
        }
    }

    fn start_worker(&'static self) {
        if self.worker.swap(true, Relaxed) {
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("yolu-tile-cache".into())
            .spawn(move || self.run_worker());
        if spawned.is_err() {
            self.worker.store(false, Relaxed);
        }
    }

    fn run_worker(&'static self) {
        loop {
            {
                let (flag, cv) = &self.wake;
                let mut f = lock(flag);
                while !*f {
                    // 頼まれるまでの間も時計を進める（使った時刻が、逃がす回の無い間も古い・新しいを分ける）
                    let (next, waited) = cv
                        .wait_timeout(f, CLOCK_TICK)
                        .unwrap_or_else(|e| e.into_inner());
                    f = next;
                    if waited.timed_out() {
                        self.clock.fetch_add(1, Relaxed);
                    }
                }
                *f = false;
                self.requested.store(false, Relaxed);
            }
            let limit = self.limit.load(Relaxed);
            if self.resident.load(Relaxed) <= limit {
                continue;
            }
            // 上限より少し下まで逃がす（上限のすぐ上で毎回少しずつ走らせない）
            let pass = self.evict_pass(limit - limit / 16);
            if pass.full || self.resident.load(Relaxed) > limit {
                // 満杯・全部読まれている: しばらく待ってから（登録のたびに一覧を全部見ない）
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }

    /// メモリにある中身を、合計が target 以下になるまで古い順に逃がす。
    fn evict_pass(&self, target: u64) -> Pass {
        let _one = lock(&self.pass);
        let mut pass = Pass::default();
        if self.resident.load(Relaxed) <= target {
            return pass;
        }
        // これより後に使った cell は、この回の候補より新しい
        self.clock.fetch_add(1, Relaxed);
        let mut candidates: Vec<(u64, Weak<TileCell>)> = Vec::new();
        for shard in &self.shards {
            let mut s = lock(shard);
            s.cells.retain(|w| w.strong_count() > 0);
            s.sweep_at = (s.cells.len() * 2).max(1024);
            for w in &s.cells {
                let Some(cell) = w.upgrade() else {
                    continue;
                };
                let resident = cell
                    .state
                    .try_read()
                    .is_ok_and(|st| st.mem.as_ref().is_some_and(|m| Arc::strong_count(m) == 1));
                if resident {
                    candidates.push((cell.last_used.load(Relaxed), w.clone()));
                }
            }
        }
        candidates.sort_by_key(|(t, _)| *t);
        // 1 度書けなかったら、この回はもう書かない。ディスクに写しのある cell（読み戻した後でメモリにも残っているもの）は
        // 書かずに手放せるので、書けない cell は飛ばして続ける（先頭に書けない古い cell があるだけで、手放せるものまで残さない）
        let mut can_write = true;
        for (_, w) in candidates {
            if self.resident.load(Relaxed) <= target {
                break;
            }
            let Some(cell) = w.upgrade() else {
                continue;
            };
            match self.evict_cell(&cell, can_write) {
                Ok(true) => pass.evicted += 1,
                Ok(false) => {}
                Err(()) => {
                    pass.full = true;
                    can_write = false;
                }
            }
        }
        pass
    }

    /// 1 つの cell の中身をディスクへ書いて（まだなら）メモリから手放す。読んでいる最中・中身が無いなら Ok(false)。
    /// ディスクが満杯・書けなければ Err。`can_write` が false なら書かない（書く要る cell は Ok(false) で残し、写しのある cell だけ手放す）。
    fn evict_cell(&self, cell: &TileCell, can_write: bool) -> Result<bool, ()> {
        if !cell.listed {
            return Ok(false);
        }
        let (bytes, write) = {
            let s = cell.state.read().unwrap_or_else(|e| e.into_inner());
            match &s.mem {
                Some(m) if Arc::strong_count(m) == 1 => (m.clone(), s.disk.is_none()),
                _ => return Ok(false),
            }
        };
        if write && !can_write {
            return Ok(false);
        }
        if write {
            let allocated = lock(&self.store).allocate(cell.len);
            let (slot, file) = match allocated {
                Ok(Some(s)) => s,
                Ok(None) => return Err(()),
                Err(_) => {
                    lock(&self.store).failed = true;
                    return Err(());
                }
            };
            if write_at(&file, &bytes, slot.offset).is_err() {
                let mut store = lock(&self.store);
                store.release(slot, cell.len);
                store.failed = true;
                return Err(());
            }
            let mut s = cell.state.write().unwrap_or_else(|e| e.into_inner());
            if s.disk.is_none() && s.mem.is_some() {
                s.disk = Some(DiskCopy { slot, file });
            } else {
                // 書いている間に中身を取り出された（書く側が持っていった）
                drop(s);
                lock(&self.store).release(slot, cell.len);
                return Ok(false);
            }
        }
        drop(bytes);
        let mut s = cell.state.write().unwrap_or_else(|e| e.into_inner());
        let free = s.disk.is_some() && s.mem.as_ref().is_some_and(|m| Arc::strong_count(m) == 1);
        if !free {
            return Ok(false);
        }
        s.mem = None;
        drop(s);
        self.sub_resident(cell.len as u64);
        Ok(true)
    }

    fn release_slot(&self, slot: Slot, len: usize) {
        lock(&self.store).release(slot, len);
    }
}

/// フォルダに新しいキャッシュのファイルを作る（Unix は作ったらすぐ名前を消す。Windows は閉じたら消える印で開く）。
fn open_cache_file(folder: &Path) -> io::Result<(File, PathBuf)> {
    let mut last = io::Error::other("キャッシュのファイルを作れない");
    for attempt in 0..8u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(attempt);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        let path = folder.join(format!(
            "{FILE_PREFIX}{}-{:016x}.bin",
            std::process::id(),
            h.finish()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            /// 閉じたら（プロセスが落ちても）OS が消す。開くには消す権利（DELETE）も要る。
            const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
            const GENERIC_READ: u32 = 0x8000_0000;
            const GENERIC_WRITE: u32 = 0x4000_0000;
            const DELETE: u32 = 0x0001_0000;
            options
                .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
                .custom_flags(FILE_FLAG_DELETE_ON_CLOSE);
        }
        match options.open(&path) {
            Ok(file) => {
                #[cfg(unix)]
                {
                    // 開いたままなら読み書きでき、閉じれば（落ちても）消える
                    let _ = std::fs::remove_file(&path);
                }
                return Ok((file, path));
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, offset)
}

#[cfg(unix)]
fn write_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buf.len() {
        let n = file.seek_read(&mut buf[done..], offset + done as u64)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        done += n;
    }
    Ok(())
}

#[cfg(windows)]
fn write_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buf.len() {
        let n = file.seek_write(&buf[done..], offset + done as u64)?;
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        done += n;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 試験ごとの係（全体の係と分ける。裏の書き手は持たず、逃がすのは試験が呼ぶ）。
    fn cache(disk_limit: u64) -> &'static Cache {
        let c: &'static Cache = Box::leak(Box::new(Cache::new()));
        lock(&c.store).disk_limit = disk_limit;
        c
    }

    fn cell(c: &'static Cache, value: u8) -> Arc<TileCell> {
        TileCell::new_in(c, vec![value; 64]).0
    }

    #[test]
    fn evicted_contents_come_back_from_disk_byte_for_byte() {
        let c = cache(1 << 20);
        let a = cell(c, 7);
        let mut pattern: Vec<u8> = (0..64).map(|i| (i * 13 + 5) as u8).collect();
        let b = TileCell::new_in(c, pattern.clone()).0;
        assert_eq!(c.resident.load(Relaxed), 128);
        assert_eq!(c.evict_pass(0).evicted, 2);
        assert_eq!(c.resident.load(Relaxed), 0);
        assert_eq!(
            (a.residency(), b.residency()),
            ((false, true), (false, true))
        );
        assert_eq!(*b.bytes().unwrap(), pattern);
        assert_eq!(
            b.residency(),
            (true, true),
            "読み戻した後もディスクの写しは残す"
        );
        assert_eq!(*a.bytes().unwrap(), vec![7u8; 64]);
        assert_eq!(c.resident.load(Relaxed), 128);
        // 2 度目は書かずに手放すだけ（中身は変えないので写しは正しい）
        let used = lock(&c.store).used;
        assert_eq!(c.evict_pass(0).evicted, 2);
        assert_eq!(lock(&c.store).used, used);
        pattern[0] ^= 0xFF;
        assert_ne!(*b.bytes().unwrap(), pattern);
    }

    #[test]
    fn contents_being_read_are_not_evicted() {
        let c = cache(1 << 20);
        let a = cell(c, 1);
        let held = a.bytes().unwrap();
        assert_eq!(c.evict_pass(0).evicted, 0, "読んでいる間は逃がさない");
        assert_eq!(a.residency(), (true, false));
        drop(held);
        assert_eq!(c.evict_pass(0).evicted, 1);
    }

    #[test]
    fn the_oldest_contents_go_first_and_eviction_stops_at_the_target() {
        let c = cache(1 << 20);
        let cells: Vec<_> = (0..4).map(|i| cell(c, i)).collect();
        // 0 と 2 を今使う: 1 と 3 が古い
        cells[0].bytes().unwrap();
        cells[2].bytes().unwrap();
        c.clock.fetch_add(1, Relaxed);
        cells[0].bytes().unwrap();
        cells[2].bytes().unwrap();
        assert_eq!(c.evict_pass(128).evicted, 2);
        let resident: Vec<bool> = cells.iter().map(|x| x.residency().0).collect();
        assert_eq!(resident, vec![true, false, true, false]);
    }

    #[test]
    fn a_full_disk_stops_eviction_and_keeps_the_contents_in_memory() {
        let c = cache(64 * 2);
        let cells: Vec<_> = (0..3).map(|i| cell(c, i)).collect();
        let pass = c.evict_pass(0);
        assert_eq!(
            pass,
            Pass {
                evicted: 2,
                full: true
            }
        );
        assert_eq!(c.resident.load(Relaxed), 64);
        for (i, x) in cells.iter().enumerate() {
            assert_eq!(*x.bytes().unwrap(), vec![i as u8; 64]);
        }
        // 区画を手放せば次は書ける
        drop(cells);
        assert_eq!(lock(&c.store).used, 0);
        let d = cell(c, 9);
        assert_eq!(c.evict_pass(0).evicted, 1);
        assert_eq!(*d.bytes().unwrap(), vec![9u8; 64]);
    }

    #[test]
    fn after_the_disk_is_full_cells_read_back_are_still_released() {
        let c = cache(64 * 2);
        let cells: Vec<_> = (0..3).map(|i| cell(c, i)).collect();
        // 先に使った 0・1 がディスクへ出て、最後の 2 は満杯で書けずメモリに残る
        assert_eq!(
            c.evict_pass(0),
            Pass {
                evicted: 2,
                full: true
            }
        );
        assert_eq!(c.resident.load(Relaxed), 64);
        // 読み戻すと 0・1 はメモリとディスクの両方に（新しく使った）。書けない 2 が一番古い候補になる
        for (i, x) in cells.iter().enumerate().take(2) {
            assert_eq!(*x.bytes().unwrap(), vec![i as u8; 64]);
        }
        assert_eq!(c.resident.load(Relaxed), 192);
        let used = lock(&c.store).used;
        // 2 は書けないので飛ばす。それでも写しのある 0・1 は書かずに手放す（満杯で先頭が止まって、残さない）
        assert_eq!(
            c.evict_pass(0),
            Pass {
                evicted: 2,
                full: true
            }
        );
        assert_eq!(c.resident.load(Relaxed), 64);
        assert_eq!(lock(&c.store).used, used, "書き直さない");
        let state: Vec<_> = cells.iter().map(|x| x.residency()).collect();
        assert_eq!(state, vec![(false, true), (false, true), (true, false)]);
        for (i, x) in cells.iter().enumerate() {
            assert_eq!(*x.bytes().unwrap(), vec![i as u8; 64]);
        }
    }

    #[test]
    fn freed_slots_are_reused_and_dropped_cells_release_memory() {
        let c = cache(1 << 20);
        let a = cell(c, 1);
        let b = cell(c, 2);
        c.evict_pass(0);
        let end = lock(&c.store).files[0].as_ref().unwrap().end;
        assert_eq!(end, 128);
        drop(a);
        let e = cell(c, 3);
        c.evict_pass(0);
        assert_eq!(
            lock(&c.store).files[0].as_ref().unwrap().end,
            128,
            "空いた区画を使い回す"
        );
        assert_eq!(*e.bytes().unwrap(), vec![3u8; 64]);
        assert_eq!(*b.bytes().unwrap(), vec![2u8; 64]);
        drop((b, e));
        assert_eq!(c.resident.load(Relaxed), 0);
        assert_eq!(lock(&c.store).used, 0);
    }

    #[test]
    fn an_unreadable_slot_is_an_error_not_a_blank_tile() {
        let c = cache(1 << 20);
        let a = cell(c, 5);
        c.evict_pass(0);
        a.fail_reads_for_test();
        assert_eq!(a.bytes(), Err(CoreError::TileUnreadable));
        assert_eq!(a.with(|b| b[0]), Err(CoreError::TileUnreadable));
        assert!(a.is_lost());
        assert_eq!(c.read_failures.load(Relaxed), 2);
    }

    #[test]
    fn writing_a_shared_cell_copies_and_an_unshared_one_moves() {
        let c = cache(1 << 20);
        let (a, pinned) = TileCell::new_in(c, vec![4u8; 64]);
        let ptr = pinned.as_ptr();
        let other = a.clone();
        let copied = TileCell::detach(a, pinned);
        assert_ne!(copied.as_ptr(), ptr, "共有していれば複製");
        assert_eq!(*other.bytes().unwrap(), vec![4u8; 64]);
        let pinned = other.bytes().unwrap();
        let ptr = pinned.as_ptr();
        let moved = TileCell::detach(other, pinned);
        assert_eq!(moved.as_ptr(), ptr, "自分だけなら取り出す");
        // 逃がした cell も、書くときは読み戻して取り出す
        let d = TileCell::new_in(c, vec![8u8; 64]).0;
        c.evict_pass(0);
        assert_eq!(TileCell::into_vec(d).unwrap(), vec![8u8; 64]);
        drop((copied, moved));
        assert_eq!(c.resident.load(Relaxed), 0);
        assert_eq!(lock(&c.store).used, 0);
    }

    #[test]
    fn kept_cells_are_neither_counted_nor_evicted() {
        let c = cache(1 << 20);
        let kept = TileCell::build(c, vec![6u8; 64], false).0;
        let listed = cell(c, 7);
        assert_eq!(c.resident.load(Relaxed), 64);
        assert!(!kept.evict());
        assert_eq!(c.evict_pass(0).evicted, 1);
        assert_eq!(kept.residency(), (true, false));
        drop((kept, listed));
        assert_eq!(c.resident.load(Relaxed), 0);
    }

    #[test]
    fn only_cache_files_from_other_processes_are_removed() {
        let dir = std::env::temp_dir().join(format!(
            "yolu-cache-stale-{}-{:x}",
            std::process::id(),
            std::collections::hash_map::RandomState::new()
                .build_hasher()
                .finish()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let own = format!("{FILE_PREFIX}{}-0123456789abcdef.bin", std::process::id());
        for name in [
            "yolupainter-cache-1-0123456789abcdef.bin",
            "yolupainter-cache-999999999-fedcba9876543210.bin",
            own.as_str(),
            "yolupainter-cache-1-short.bin",
            "yolupainter-cache-x-0123456789abcdef.bin",
            "other.bin",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        assert_eq!(remove_stale_files(&dir), 2);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        let mut want = vec![
            own,
            "other.bin".to_string(),
            "yolupainter-cache-1-short.bin".to_string(),
            "yolupainter-cache-x-0123456789abcdef.bin".to_string(),
        ];
        want.sort();
        assert_eq!(left, want);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_cache_file_has_no_name_while_it_is_open() {
        let c = cache(1 << 20);
        let a = cell(c, 1);
        c.evict_pass(0);
        let created = lock(&c.store).created.clone();
        assert_eq!(created.len(), 1);
        assert!(!created[0].exists(), "開いたらすぐ名前を消す");
        assert_eq!(*a.bytes().unwrap(), vec![1u8; 64]);
    }

    #[cfg(windows)]
    #[test]
    fn the_cache_file_disappears_when_it_is_closed() {
        let dir = std::env::temp_dir().join(format!("yolu-cache-close-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (file, path) = open_cache_file(&dir).unwrap();
        write_at(&file, &[1, 2, 3, 4], 0).unwrap();
        assert!(path.exists(), "開いている間は名前がある");
        drop(file);
        assert!(!path.exists(), "閉じたら消える");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_new_folder_takes_new_slots_and_the_old_file_closes_when_empty() {
        let c = cache(1 << 20);
        let a = cell(c, 1);
        c.evict_pass(0);
        {
            let mut store = lock(&c.store);
            store.folder = std::env::temp_dir().join(".");
            store.current = None;
        }
        let b = cell(c, 2);
        c.evict_pass(0);
        assert_eq!(lock(&c.store).files.iter().flatten().count(), 2);
        assert_eq!(*a.bytes().unwrap(), vec![1u8; 64]);
        drop(a);
        assert_eq!(
            lock(&c.store).files.iter().flatten().count(),
            1,
            "古いファイルは閉じる"
        );
        assert_eq!(*b.bytes().unwrap(), vec![2u8; 64]);
    }
}
