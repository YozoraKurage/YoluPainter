//! 復旧の置き場の根（`root`）の下の「プール」。1 回の起動（セッション）ごとに 1 つのフォルダ `<UTC 時刻>-<乱数>` で、
//! 中は世代の置き場（`yolu_io::GenerationStore`）と、印のファイル。
//!
//! - `session.lock`: 動いているあいだ OS の排他ロックを持ち、中に `state=clean|dirty`（保存していない作業の世代がある
//!   か）を書く。正しく閉じると消す。落ちるとロックは OS が手放すが、ファイルは残る → 次の起動が「ロックが取れる印」を
//!   見つけて、落ちたと分かる。別のプロセスが生きていればロックが取れない（使用中。触らない）。
//! - `crashed`: 落ちたと見つけた起動が、`session.lock` の代わりに置く（中は落ちた時の `state=`）。もう 1 度は知らせないが、
//!   一覧には残り、利用者が捨てるまで整理もしない。
//! - 印の無いプールは、正しく閉じたもの。世代は、閉じたプールの合計で設定の数だけ残す（古いものから整理）。
//!
//! 触るのは、名前が `<UTC 時刻>-<乱数>` で印か世代の置き場を持つプールだけ（置き場の根に利用者がほかのものを置いても
//! 消さない。シンボリックリンクは辿らない）。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use yolu_io::{
    generation_time_ms, utc_stamp, GenerationStore, Project, RecoveryInfo, StoreError, INFO_LIMIT,
    INFO_NAME,
};

use super::quota::{self, Limits};
use super::RecoveryError;

pub(crate) const LOCK: &str = "session.lock";
pub(crate) const CRASHED: &str = "crashed";

/// プールの状態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// 別のプロセスが動いている（触らない）。
    Live,
    /// 落ちたと分かっている（`crashed` がある）か、まだ見つけていない（`session.lock` が残っている）。
    Crashed,
    /// 正しく閉じた。
    Closed,
}

/// 一覧の 1 世代。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub pool: PathBuf,
    pub id: String,
    pub time_ms: Option<u64>,
    /// セットの数。
    pub documents: usize,
    /// 元の .ylp のファイル名（無ければプロジェクトの名前。空なら名前なし）。
    pub name: String,
    /// 落ちたプールの世代か。
    pub crashed: bool,
    /// この実行の置き場の世代か。
    pub own: bool,
    /// 読めない理由（壊れた世代）。
    pub problem: Option<String>,
}

/// この実行の置き場（プール）。持っているあいだロックを持つ。`Drop` はロックを手放すだけで印は消さない（落ちた体）。
/// 正しく閉じるには `close_clean`。
pub struct Session {
    dir: PathBuf,
    lock: Option<File>,
    dirty: bool,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("dir", &self.dir)
            .field("dirty", &self.dirty)
            .finish()
    }
}

/// 起動の結果。
pub struct Started {
    pub session: Session,
    /// 前の実行が落ちていて、保存していない作業の世代が残っているプール（今回初めて見つけたもの）。印を片付けられなかった
    /// プール（`skipped`）も、落ちたものとして入る（世代を見せない方へは倒さない）。
    pub crashed: Vec<PathBuf>,
    /// 前の実行の印を片付けられなかった理由（初めの 1 つ。ほかの起動と競って先に片付けられたときは入れない）。
    pub skipped: Option<io::Error>,
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 乱数 64 bit の 16 進 16 桁（プールの名前の後ろ）。
fn unique() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNT.fetch_add(1, Ordering::Relaxed));
    h.write_u32(std::process::id());
    format!("{:016x}", h.finish())
}

/// プールの名前の形（`yyyyMMddTHHmmssfff-<16 進>`）。
pub(crate) fn is_pool_name(name: &str) -> bool {
    name.len() <= 64
        && generation_time_ms(name).is_some()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// 根の下の、プールとして扱うフォルダ（名前の形が合い、印か世代の置き場を持つ。シンボリックリンクは除く）。
pub(crate) fn pools(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().is_some_and(is_pool_name))
        .map(|e| e.path())
        .filter(|p| {
            is_real_dir(p)
                && [LOCK, CRASHED, "current", "generations"]
                    .iter()
                    .any(|n| fs::symlink_metadata(p.join(n)).is_ok())
        })
        .collect();
    dirs.sort();
    dirs
}

/// 根の直下の、プールの名前のフォルダか（UI から来たパスが根の外へ出ないことの確かめ）。
pub(crate) fn check_pool(root: &Path, pool: &Path) -> Result<(), RecoveryError> {
    let ok = pool.parent() == Some(root)
        && pool
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(is_pool_name)
        && is_real_dir(pool);
    if ok {
        Ok(())
    } else {
        Err(RecoveryError::NotPool)
    }
}

/// 別のプロセスが持っているロックか（持たれていなければ、ここで取ってすぐ手放す）。
fn locked_by_someone(path: &Path) -> io::Result<bool> {
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            Ok(false)
        }
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// プールの状態。`session.lock` のロックを調べるので、自分のプールには使わない（自分のロックで「使用中」になる）。
pub(crate) fn kind(pool: &Path) -> Kind {
    let lock = pool.join(LOCK);
    if fs::symlink_metadata(&lock).is_ok() {
        return match locked_by_someone(&lock) {
            Ok(true) => Kind::Live,
            // 開けない（権限など）ものは触らない
            Err(_) => Kind::Live,
            Ok(false) => Kind::Crashed,
        };
    }
    if fs::symlink_metadata(pool.join(CRASHED)).is_ok() {
        Kind::Crashed
    } else {
        Kind::Closed
    }
}

/// 残っている `session.lock`（ロックが取れる = 落ちた）を `crashed` に替える。返すのは、落ちた時に保存していない作業の世代が
/// あったか（読めない印は、あったものとして扱う）。
fn resolve_crashed(pool: &Path) -> io::Result<bool> {
    let lock = pool.join(LOCK);
    let mut text = String::new();
    File::open(&lock)?.take(256).read_to_string(&mut text).ok();
    let dirty = !text.contains("state=clean");
    let mut marker = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(pool.join(CRASHED))?;
    marker.write_all(if dirty {
        b"state=dirty\n"
    } else {
        b"state=clean\n"
    })?;
    marker.sync_all()?;
    drop(marker);
    fs::remove_file(&lock)?;
    Ok(dirty)
}

/// 残っている印の後片付けの結果。
#[derive(Debug)]
enum Settled {
    /// 同時に始めた別の起動が先に片付けた（印か置き場が、読む前に消えた）。何もしない。
    Raced,
    /// 片付けた。保存していない作業の世代があったか。
    Done(bool),
    /// 片付けられなかった（権限・ディスクの空きなど）。印が読めなかったのと同じく、保存していない作業があったものとして扱う。
    Failed(io::Error),
}

fn settle(pool: &Path) -> Settled {
    match resolve_crashed(pool) {
        Ok(dirty) => Settled::Done(dirty),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Settled::Raced,
        Err(e) => Settled::Failed(e),
    }
}

/// 前の実行の後片付けをして、この実行のプールを作る。落ちた実行のプール（印が残っている）は `crashed` に替え、保存して
/// いない作業の世代があるものを返す。閉じたプールは `keep` の数に整理する（`None` は整理しない: 利用者が選んだ数が分からない
/// とき）。続けて、ディスクの上限（`limits`。`None` は整理しない: 利用者が選んだ量が分からないとき）を超えていれば古い世代から消す。
pub fn start(
    root: &Path,
    keep: Option<usize>,
    limits: Option<&Limits>,
) -> Result<Started, RecoveryError> {
    fs::create_dir_all(root)?;
    let mut crashed = Vec::new();
    let mut skipped: Option<io::Error> = None;
    for pool in pools(root) {
        if fs::symlink_metadata(pool.join(LOCK)).is_err() {
            continue;
        }
        if locked_by_someone(&pool.join(LOCK)).unwrap_or(true) {
            continue; // 別のプロセスが動いている
        }
        let dirty = match settle(&pool) {
            Settled::Raced => continue,
            Settled::Done(dirty) => dirty,
            Settled::Failed(e) => {
                skipped.get_or_insert(e);
                true
            }
        };
        let has_data = GenerationStore::new(&pool)
            .list()
            .map(|g| !g.is_empty())
            .unwrap_or(true);
        if !has_data {
            let _ = fs::remove_dir_all(&pool);
        } else if dirty {
            crashed.push(pool);
        }
    }
    sweep_held(root);
    if let Some(keep) = keep {
        let _ = sweep(root, keep, None);
    }
    if let Some(limits) = limits {
        quota::enforce(root, limits, None, now_ms());
    }
    let dir = root.join(format!("{}-{}", utc_stamp(now_ms()), unique()));
    fs::create_dir(&dir)?;
    let mut lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(dir.join(LOCK))?;
    lock.try_lock().map_err(|e| match e {
        std::fs::TryLockError::WouldBlock => RecoveryError::Store(StoreError::Busy),
        std::fs::TryLockError::Error(e) => RecoveryError::Io(e),
    })?;
    lock.write_all(b"state=clean\n")?;
    lock.flush()?;
    Ok(Started {
        session: Session {
            dir,
            lock: Some(lock),
            dirty: false,
        },
        crashed,
        skipped,
    })
}

impl Session {
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn store(&self) -> GenerationStore {
        GenerationStore::new(&self.dir)
    }
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    /// 保存していない作業の世代があるかを印に書く（変わったときだけ書く）。
    pub fn mark(&mut self, dirty: bool) -> io::Result<()> {
        if self.dirty == dirty {
            return Ok(());
        }
        let Some(lock) = self.lock.as_mut() else {
            return Ok(());
        };
        lock.set_len(0)?;
        lock.seek(io::SeekFrom::Start(0))?;
        lock.write_all(if dirty {
            b"state=dirty\n"
        } else {
            b"state=clean\n"
        })?;
        lock.flush()?;
        self.dirty = dirty;
        Ok(())
    }
    /// 正しく閉じる: 印を消す（世代は残す。無ければプールごと消す）。閉じたプールを `keep` の数に整理する（`None` は整理しない）。
    /// ディスクの上限（`limits`）の整理では、この実行の最新の世代を守る（閉じたあとの最初の起動から、ほかの閉じた世代と同じ扱い）。
    pub fn close_clean(mut self, root: &Path, keep: Option<usize>, limits: Option<&Limits>) {
        drop(self.lock.take());
        let _ = fs::remove_file(self.dir.join(LOCK));
        // 復旧から開いたプロジェクトの置き直した中身（閉じたあとは使わない）
        let _ = fs::remove_dir_all(held_root(&self.dir));
        let empty = GenerationStore::new(&self.dir)
            .list()
            .map(|g| g.is_empty())
            .unwrap_or(false);
        if empty {
            let _ = fs::remove_dir_all(&self.dir);
        }
        if let Some(keep) = keep {
            let _ = sweep(root, keep, None);
        }
        if let Some(limits) = limits {
            quota::enforce(root, limits, Some(&self.dir), now_ms());
        }
    }
}

/// 閉じたプールの世代の合計を、新しいものから `keep` に整理する（超えた古い世代を捨て、空になったプールを消す）。落ちた
/// プール・動いているプール・`own` は触らない。
pub fn sweep(root: &Path, keep: usize, own: Option<&Path>) -> Result<(), RecoveryError> {
    let mut all: Vec<(String, PathBuf)> = Vec::new();
    for pool in pools(root) {
        if Some(pool.as_path()) == own || kind(&pool) != Kind::Closed {
            continue;
        }
        match GenerationStore::new(&pool).list() {
            Ok(list) if list.is_empty() => {
                let _ = fs::remove_dir_all(&pool);
            }
            Ok(list) => all.extend(list.into_iter().map(|g| (g.id, pool.clone()))),
            Err(_) => {}
        }
    }
    all.sort_by(|a, b| b.0.cmp(&a.0));
    let mut touched: Vec<PathBuf> = Vec::new();
    for (id, pool) in all.into_iter().skip(keep) {
        if GenerationStore::new(&pool).remove_generation(&id).is_ok() && !touched.contains(&pool) {
            touched.push(pool);
        }
    }
    for pool in touched {
        if GenerationStore::new(&pool)
            .list()
            .is_ok_and(|g| g.is_empty())
        {
            let _ = fs::remove_dir_all(&pool);
        }
    }
    Ok(())
}

/// 根の下の世代の一覧（新しい順）。動いている別のプロセスのプールは除く。`own` はこの実行のプール。
pub fn list(root: &Path, own: Option<&Path>) -> Vec<Row> {
    let mut rows = Vec::new();
    for pool in pools(root) {
        let is_own = Some(pool.as_path()) == own;
        let state = if is_own { Kind::Closed } else { kind(&pool) };
        if state == Kind::Live {
            continue;
        }
        let store = GenerationStore::new(&pool);
        let Ok(generations) = store.list() else {
            continue;
        };
        for g in generations {
            let info = store
                .read_file(Some(&g.id), INFO_NAME, INFO_LIMIT)
                .ok()
                .flatten()
                .and_then(|b| RecoveryInfo::from_bytes(&b).ok())
                .unwrap_or_default();
            let name = match info.project_path.rsplit(['/', '\\']).next() {
                Some(file) if !file.is_empty() => file.to_owned(),
                _ => info.title.clone(),
            };
            rows.push(Row {
                pool: pool.clone(),
                time_ms: g.time_ms,
                documents: g.documents,
                name,
                crashed: state == Kind::Crashed,
                own: is_own,
                problem: g.problem,
                id: g.id,
            });
        }
    }
    rows.sort_by(|a, b| b.id.cmp(&a.id));
    rows
}

/// 世代を開く用に読む（全エントリを確かめる。上限は設定の「レイヤーのメモリ」の予算から）。一覧用の情報は外す。メモリに読まなかった
/// エントリ（正本・PSD の原本などの大きなもの）は、置き場の世代の外（この実行の [`Held`]）へ置き直してから読む: 開いたあとで、その世代が
/// 整理・破棄されても（復旧の窓の「破棄」・保持数・ディスクの上限・ほかのウィンドウの整理）、読むだけのセット（core で扱えない中身・
/// 予算超過・効果の入力がそろわない）の正本と、描いていないセットのエントリを保存できるように。置き直しはハードリンクで、できなければ
/// 流して写す（メモリに全部を持たない）。置き直せなければ、開かずに理由を返す（後で保存できなくなる開き方をしない）。
pub fn load(
    root: &Path,
    pool: &Path,
    id: &str,
    limits: yolu_io::Limits,
    own: &Path,
) -> Result<(Project, RecoveryInfo), RecoveryError> {
    check_pool(root, pool)?;
    let generation = GenerationStore::new(pool)
        .with_limits(limits)
        .load_generation(id)?;
    let mut files = generation.files;
    if files.values().any(|b| b.in_memory().is_none()) {
        let held = Held::create(own)?;
        let keep: yolu_io::Keep = held.clone();
        for blob in files.values_mut() {
            *blob = blob.hold_in(&held.0, &keep)?;
        }
    }
    let info = files
        .remove(INFO_NAME)
        .and_then(|b| b.bytes().ok())
        .and_then(|b| RecoveryInfo::from_bytes(&b).ok())
        .unwrap_or_default();
    Ok((Project::from_entries(files)?, info))
}

/// 復旧から開いたプロジェクトの、置き場の世代の外へ置き直した中身のフォルダ（`<根>/<この実行のプール>.held~/<番号>`）。中身を指す
/// エントリ（`Blob`）が持ち、最後の 1 つ（とその読み手）が手放されると消える（ほかのプロジェクトを開いた・保存した .ylp を元にした
/// あと）。正しく閉じるときは `close_clean` が、落ちて残ったものは次の起動の `start` が片付ける（動いているプロセスのものには触らない）。
/// プールの名前の形ではない（`.` と `~` を含む）ので、プールの一覧・整理・ディスクの量の数えには入らない。
pub(crate) struct Held(PathBuf);

impl Held {
    fn create(own: &Path) -> io::Result<std::sync::Arc<Self>> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = held_root(own);
        fs::create_dir_all(&parent)?;
        loop {
            let dir = parent.join(NEXT.fetch_add(1, Ordering::Relaxed).to_string());
            match fs::create_dir(&dir) {
                Ok(()) => return Ok(std::sync::Arc::new(Self(dir))),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// この実行のプール（`own`）の、置き直した中身の置き場。
pub(crate) fn held_root(own: &Path) -> PathBuf {
    let name = own
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    own.with_file_name(format!("{name}{HELD_SUFFIX}"))
}

const HELD_SUFFIX: &str = ".held~";

/// 落ちた実行・閉じた実行が残した、置き直した中身の置き場を消す（持ち主のプールが別のプロセスで動いているものは残す）。
fn sweep_held(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(owner) = name.strip_suffix(HELD_SUFFIX) else {
            continue;
        };
        let path = entry.path();
        if is_pool_name(owner) && is_real_dir(&path) && kind(&root.join(owner)) != Kind::Live {
            let _ = fs::remove_dir_all(&path);
        }
    }
}

/// 世代を 1 つ捨てる。プールが空になれば（この実行のプール以外は）プールごと消す。
pub fn discard(
    root: &Path,
    pool: &Path,
    id: &str,
    own: Option<&Path>,
) -> Result<(), RecoveryError> {
    check_pool(root, pool)?;
    if Some(pool) != own && kind(pool) == Kind::Live {
        return Err(RecoveryError::Store(StoreError::Busy));
    }
    let store = GenerationStore::new(pool);
    store.remove_generation(id)?;
    if Some(pool) != own && store.list().is_ok_and(|g| g.is_empty()) {
        let _ = fs::remove_dir_all(pool);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yolu-pool-{tag}-{}-{}",
            std::process::id(),
            unique()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_marker_that_vanished_is_a_race_and_one_that_cannot_be_settled_is_a_failure() {
        let dir = temp("settle");
        // 読む前に消えた（同時に始めた別の起動が片付けた）: 競り
        assert!(matches!(settle(&dir), Settled::Raced));
        assert!(matches!(settle(&dir.join("missing")), Settled::Raced));
        // 片付けられる
        fs::write(dir.join(LOCK), "state=clean\n").unwrap();
        assert!(matches!(settle(&dir), Settled::Done(false)));
        assert!(dir.join(CRASHED).is_file() && !dir.join(LOCK).exists());
        // `crashed` を書けない（同じ名前のフォルダがある）: 競りではなく失敗
        let blocked = temp("settle-blocked");
        fs::write(blocked.join(LOCK), "state=dirty\n").unwrap();
        fs::create_dir(blocked.join(CRASHED)).unwrap();
        match settle(&blocked) {
            Settled::Failed(e) => assert_ne!(e.kind(), io::ErrorKind::NotFound),
            other => panic!("失敗のはず: {other:?}"),
        }
        assert!(
            blocked.join(LOCK).exists(),
            "片付けられなかった印は残る（次の起動がやり直す）"
        );
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(blocked);
    }
}
