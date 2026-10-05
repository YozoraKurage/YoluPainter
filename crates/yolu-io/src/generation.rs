//! 復旧用の世代の置き場（Unity 版 `GenerationStore` と同じ契約・同じディスクの形。互換の射程は下）。
//!
//! 1 つの置き場（`root`）に、確かめ済みの世代を積む。世代は 1 度作ったら書き換えず、`current`（今の世代の名前を書いた小さな
//! ファイル）を**最後に**置き換えて確定する。途中で落ちても、`current` が指す世代は前のまま読める。
//!
//! ```text
//! <root>/current                  今の世代の名前（最後に置き換える）
//! <root>/previous                 1 つ前の世代の名前
//! <root>/generations/<世代>/manifest.sha256
//! <root>/generations/<世代>/<名前>      （manifest の見出しが -1 の世代だけ。中身を世代の中に持つ）
//! <root>/contents/<SHA-256>.bin   （見出しが -2 の世代の中身。同じ中身は世代をまたいで 1 つを共有する）
//! <root>/.save.lock               書き込みの排他（OS のロック。落ちて残っても邪魔にならない）
//! <root>/.staging-<世代>/         作っている途中の世代（落ちて残ったものは `reclaim_abandoned` が片付ける）
//! ```
//!
//! 世代の名前は `yyyyMMddTHHmmssfff-<乱数 32 桁>`（UTC）で、名前の順が時刻の順。manifest は `DOTPAINT-MANIFEST-1|2` の見出しと
//! `<SHA-256> <長さ> <名前>` の行。名前は `.ylp` のエントリ名（`sets/<ID>/…`・`resources/…`・根）と同じ範囲に限る。
//!
//! Unity 版との互換の射程: ディスクの形（`current`・`previous`・manifest・`contents/`）は同じで、Unity 版が書いた世代はこの置き場が
//! 読める（実際の C# の出力を fixture にして試験する）。逆は一部だけ: この置き場は `.ylp` のエントリ名を通すので、`sets/<ID>/` の
//! 下に入れ子の名前（開いた `.ylp` の合成の PNG `sets/<ID>/composite/Color.png` など）を含む世代を書くが、Unity 版の
//! `ValidateName` はその名前を「Unsafe generation filename」で断る（Unity 版が読める世代は、入れ子の名前を含まないもの）。
//! 実際に Unity 版に読ませた結果は `tests/fixtures/rust-generation.unity.txt`。
//!
//! 保証の射程: 確定の前に世代の中身を読み直してハッシュを確かめ、確定の直前にも `current` が期待した世代のままか確かめる。
//! ファイルは `sync_all` で書き出す。ディレクトリの fsync と電源断の耐久性は OS に依り、約束しない。ロックはこの道具どうしの
//! 排他で、ロックしない外部の書き手はハッシュで見つける（止めない）。

use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::package::{part_number, Blob, Limits, MAX_ONE_ENTRY, MAX_PART_BYTES};
use crate::{hash, is_hash, valid_id};

const FLAT: &str = "DOTPAINT-MANIFEST-1";
const SHARED: &str = "DOTPAINT-MANIFEST-2";
const NATIVE: &str = "document.utpaint";
const MANIFEST: &str = "manifest.sha256";
/// manifest の大きさの上限。
const MANIFEST_LIMIT: u64 = 1024 * 1024;

/// 世代の全エントリ（`.ylp` のエントリ名 → 中身。中身はメモリか、置き場のファイル・.ylp の中の位置・正本から作るもの）。
pub type Files = crate::package::Files;

/// 書く前の空きの確かめ。これから**新しく**書くバイト数（共有の中身で置き場にもうあるものは含めない。manifest・ポインタの小さな
/// ものも含めない）を渡され、書けないなら `LowSpace` を返す。確定の前、中身を書き始める前に 1 回だけ呼ばれ、Err なら何も
/// 書かずに `StoreError::LowSpace` で断る（`current` は前のまま）。
pub type SpaceGuard = Arc<dyn Fn(u64) -> Result<(), LowSpace> + Send + Sync>;

/// 空きが足りなくて書かなかった理由（バイト数）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LowSpace {
    /// 置き場のあるボリュームの空き。
    pub available: u64,
    /// 今回新しく書くバイト数。
    pub needed: u64,
    /// 書いたあとも空けておく量。
    pub reserve: u64,
}

/// 試験用の障害注入。段の名前（`file:<名前>`・`verified`・`generation-renamed`・`before-pointer`・`after-pointer`・
/// `prune-generation:<世代>`・`prune-content:<札>`）で呼ばれ、Err を返すとその段で失敗する。整理の段の失敗は確定した保存を
/// 取り消さず、握りつぶす。
pub type Fault = Arc<dyn Fn(&str) -> io::Result<()> + Send + Sync>;

/// 世代の置き場の失敗。画面は種類から短い理由を作り、`Display` は日本語の診断。
#[derive(Debug)]
pub enum StoreError {
    /// 確定した世代が無い（`current` が無い）。
    NoGeneration,
    /// 期待の印なしの確定で、置き場にもう世代がある（黙って上書きしない）。
    AlreadyExists,
    /// 期待した世代から変わっている（外の書き手・手作業・壊れ。理由を添える）。
    Changed(String),
    /// 別の書き手が置き場のロックを持っている。
    Busy,
    /// 世代・中身が壊れている・不正（理由）。
    Corrupt(String),
    /// 予算・上限を超えた。
    Budget(String),
    /// 呼び出しの誤り（保持数・空の世代など）。
    InvalidArgument(&'static str),
    /// ディスクの空きが少なく、書かなかった（`SpaceGuard` の断り）。
    LowSpace(LowSpace),
    Io(io::Error),
}
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoGeneration => f.write_str("確定した世代がありません"),
            Self::AlreadyExists => {
                f.write_str("置き場に既に世代があります。黙って上書きしません")
            }
            Self::Changed(why) => write!(f, "置き場の世代が外で変わりました（{why}）"),
            Self::Busy => f.write_str("別の書き込みが置き場を使っています"),
            Self::Corrupt(why) | Self::Budget(why) => f.write_str(why),
            Self::InvalidArgument(what) => write!(f, "不正な指定です: {what}"),
            Self::LowSpace(low) => write!(
                f,
                "ディスクの空きが少ないので書きませんでした（空き {} バイト、新しく書く量 {}、残す量 {}）",
                low.available, low.needed, low.reserve
            ),
            Self::Io(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}
impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
type R<T> = Result<T, StoreError>;
fn corrupt<T>(why: impl Into<String>) -> R<T> {
    Err(StoreError::Corrupt(why.into()))
}

/// 読んだ世代（全エントリ検証済み）。
#[derive(Clone, Debug)]
pub struct Generation {
    pub id: String,
    /// 世代と manifest の札（外の書き手の検出に使う）。
    pub token: String,
    pub files: Files,
}

/// 確定した世代（中身は持たない。大きなバイト列を持ち越さない）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Committed {
    pub id: String,
    pub token: String,
    /// 今回新しく書いた中身のバイト数（manifest・ポインタは含めない）。
    pub written_bytes: u64,
    /// 前の世代と共有して書かなかったエントリの数。
    pub reused_files: usize,
}

/// 確定のしかた。
#[derive(Clone, Debug, Default)]
pub struct CommitOptions<'a> {
    /// 期待する今の世代の札。`None` は「置き場にまだ世代が無いこと」を求める。
    pub expected: Option<&'a str>,
    /// 残す世代の数（2 以上）。確定の後に、`current`・`previous` を守って超えた分の古い世代を消す。`None` は消さない。
    pub keep: Option<usize>,
    /// 同じ中身を世代の間で共有する（見出し -2）。しなければ世代の中に全部を書く（見出し -1）。
    pub share: bool,
}

/// 一覧の 1 世代の軽い情報（中身のハッシュは読まない。長さと存在だけ確かめる）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationInfo {
    pub id: String,
    /// 世代の名前から読んだ作った時刻（UTC の Unix ミリ秒）。
    pub time_ms: Option<u64>,
    /// 中身を共有する世代（見出し -2）か。
    pub shared: bool,
    /// 文書（`document.utpaint`）の数 = テクスチャセットの数。
    pub documents: usize,
    pub entries: usize,
    pub bytes: u64,
    pub is_current: bool,
    /// 読めない・壊れている理由（読み飛ばす世代）。
    pub problem: Option<String>,
}

/// 世代 1 つがディスクで占めるもの（`GenerationStore::footprint`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationFootprint {
    pub id: String,
    /// 世代の名前から読んだ作った時刻（UTC の Unix ミリ秒）。
    pub time_ms: Option<u64>,
    /// 世代のフォルダの中のバイト数。中身を世代の中に持つ世代（見出し -1）は中身ぜんぶ、共有の世代（見出し -2）は manifest だけ。
    pub own_bytes: u64,
    /// 共有の中身（札と長さ。同じ札は 1 つにまとめる）。この世代を消しても、ほかの世代が使う中身は消えない。
    pub contents: Vec<(String, u64)>,
    /// 読めない・壊れている理由。manifest を読めない世代は、使う中身が分からないので `contents` は空。
    pub problem: Option<String>,
}

/// 置き場がディスクで占めるもの。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Footprint {
    /// 世代（新しい順）。
    pub generations: Vec<GenerationFootprint>,
    /// 置き場のフォルダ全体のバイト数（作りかけ・どの世代も使わない中身・manifest・ポインタ・ロックのファイルを含む）。
    pub total_bytes: u64,
}

/// 復旧用の世代の置き場。
#[derive(Clone)]
pub struct GenerationStore {
    root: PathBuf,
    fault: Option<Fault>,
    /// 書き込みの予算（1 エントリ・合計の上限バイト数）。`None` は形の上限だけ（1 エントリは正本の部分 256 MiB・ほか 512 MiB）。
    budget: Option<(u64, u64)>,
    /// 読み書きの量の上限（設定の予算から。`None` は形の上限だけ）。
    limits: Option<Limits>,
    /// この道具が流して確かめた共有の中身と、そのときの長さ・更新時刻（同じなら、次の確定でハッシュを数え直さない。複製の間で共有）。
    good: Arc<std::sync::Mutex<std::collections::HashMap<PathBuf, (u64, SystemTime)>>>,
    /// 書く前の空きの確かめ。`None` は確かめない。
    space: Option<SpaceGuard>,
}
impl std::fmt::Debug for GenerationStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenerationStore")
            .field("root", &self.root)
            .field("fault", &self.fault.is_some())
            .field("budget", &self.budget)
            .field("limits", &self.limits)
            .field("space_guard", &self.space.is_some())
            .finish()
    }
}

struct Entry {
    hash: String,
    len: u64,
    name: String,
}
struct Manifest {
    shared: bool,
    entries: Vec<Entry>,
}

static NEXT: AtomicU64 = AtomicU64::new(0);
static LAST_MS: AtomicU64 = AtomicU64::new(0);

impl GenerationStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            fault: None,
            budget: None,
            limits: None,
            space: None,
            good: Arc::default(),
        }
    }
    /// この道具が確かめた後、長さと更新時刻の変わっていない共有の中身か（ハッシュを数え直さない。保証の射程: 更新時刻を変えずに
    /// 同じ長さで書き換える外の書き手は、ここでは見つけない。開くとき（`load`）はいつも全部を数える）。
    fn known_good(&self, path: &Path, len: u64) -> bool {
        let Ok(meta) = fs::symlink_metadata(path) else {
            return false;
        };
        let Ok(modified) = meta.modified() else {
            return false;
        };
        meta.is_file()
            && meta.len() == len
            && self
                .good
                .lock()
                .is_ok_and(|g| g.get(path) == Some(&(len, modified)))
    }
    fn remember_good(&self, path: &Path) {
        if let Ok(meta) = fs::symlink_metadata(path) {
            if let (Ok(modified), Ok(mut g)) = (meta.modified(), self.good.lock()) {
                g.insert(path.to_path_buf(), (meta.len(), modified));
            }
        }
    }
    /// 読み書きの量の上限を設定の予算から決める（`Limits`。セットごとの正本は「レイヤーのメモリ」の予算の 4 倍、全体は正本の数 ×
    /// それ ＋ 768 MiB）。超える世代は書かず、読まない（理由に予算の名前を添える）。
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = Some(limits);
        self
    }
    /// 書く前の空きの確かめを付ける（`SpaceGuard`）。確かめは新しく書くバイト数を見るので、前の世代と同じ中身だけの確定は
    /// 0 バイトで確かめる。
    pub fn with_space_guard(mut self, guard: SpaceGuard) -> Self {
        self.space = Some(guard);
        self
    }
    /// 書き込みの予算を小さくする（1 エントリの上限・合計の上限、バイト数）。既定の上限より大きくはできない。試験が、大きな領域を
    /// 確保せずに予算の断りを確かめるための口。
    pub fn with_budget(mut self, entry_bytes: u64, total_bytes: u64) -> Self {
        self.budget = Some((entry_bytes.min(MAX_ONE_ENTRY), total_bytes));
        self
    }
    /// 試験用の障害注入（`Fault`）を付ける。
    pub fn with_fault(mut self, fault: Fault) -> Self {
        self.fault = Some(fault);
        self
    }
    pub fn set_fault(&mut self, fault: Option<Fault>) {
        self.fault = fault;
    }
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn hit(&self, stage: &str) -> R<()> {
        match &self.fault {
            Some(f) => f(stage).map_err(StoreError::Io),
            None => Ok(()),
        }
    }
    fn generations_dir(&self) -> PathBuf {
        self.root.join("generations")
    }
    fn content_path(&self, hash: &str) -> PathBuf {
        self.root.join("contents").join(format!("{hash}.bin"))
    }

    /// `current` が指す世代を、全エントリを確かめて読む。
    pub fn load(&self) -> R<Generation> {
        let id = self.current_id()?.ok_or(StoreError::NoGeneration)?;
        self.read_generation(&id, true)
    }
    /// 名前で選んだ世代を、全エントリを確かめて読む（`current` でなくてよい）。
    pub fn load_generation(&self, id: &str) -> R<Generation> {
        validate_generation(id)?;
        self.read_generation(id, true)
    }
    /// 今の世代の札（`current` と manifest だけを読む。中身は確かめない）。
    pub fn token(&self) -> R<String> {
        let id = self.current_id()?.ok_or(StoreError::NoGeneration)?;
        let manifest = self.read_manifest_bytes(&id)?;
        Ok(token_of(&id, &manifest))
    }
    /// `current` が札と違う（読めない場合も含む）。
    pub fn has_external_change(&self, token: &str) -> bool {
        self.token().map(|t| t != token).unwrap_or(true)
    }

    fn current_id(&self) -> R<Option<String>> {
        read_pointer(&self.root.join("current"))
    }
    fn read_manifest_bytes(&self, id: &str) -> R<Vec<u8>> {
        let path = self.generations_dir().join(id).join(MANIFEST);
        match fs::symlink_metadata(&path) {
            Ok(_) => read_bounded(&path, MANIFEST_LIMIT),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                corrupt("世代に manifest がありません")
            }
            Err(e) => Err(e.into()),
        }
    }
    /// 世代の全エントリを長さとハッシュまで流して確かめる。`keep_bytes` なら、エントリを返す（小さなものはメモリに、ほかは置き場の
    /// ファイルを指す）。
    fn read_generation(&self, id: &str, keep_bytes: bool) -> R<Generation> {
        let manifest_bytes = self.read_manifest_bytes(id)?;
        let manifest = parse_manifest(&manifest_bytes)?;
        if keep_bytes {
            self.check_limits(manifest.entries.iter().map(|e| (e.name.as_str(), e.len)))?;
        }
        let dir = self.generations_dir().join(id);
        let mut files = Files::new();
        for entry in &manifest.entries {
            let small = self.read_entry(&dir, &manifest, entry, keep_bytes)?;
            if keep_bytes {
                let blob = match small {
                    Some(bytes) => Blob::from(bytes),
                    None => Blob::file(
                        self.entry_path(&dir, &manifest, entry),
                        &entry.name,
                        entry.len,
                        entry.hash.clone(),
                    ),
                };
                files.insert(entry.name.clone(), blob);
            }
        }
        Ok(Generation {
            id: id.to_owned(),
            token: token_of(id, &manifest_bytes),
            files,
        })
    }
    /// 1 エントリを長さとハッシュを流して確かめる（メモリに全部を読まない）。`keep` なら、小さな中身（1 MiB まで）を返す。
    fn read_entry(
        &self,
        dir: &Path,
        manifest: &Manifest,
        entry: &Entry,
        keep: bool,
    ) -> R<Option<Vec<u8>>> {
        let path = self.entry_path(dir, manifest, entry);
        match fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() && m.len() == entry.len => {}
            Ok(_) => return corrupt(format!("世代の長さが合いません: {}", entry.name)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return corrupt(format!("世代の中身がありません: {}", entry.name))
            }
            Err(e) => return Err(e.into()),
        }
        if !keep && manifest.shared && self.known_good(&path, entry.len) {
            return Ok(None);
        }
        let small = keep && entry.len <= crate::package::Thresholds::current().keep_in_memory;
        let (digest, bytes) = digest_file(&path, entry.len, small)?;
        if manifest.shared && digest == entry.hash {
            self.remember_good(&path);
        }
        if digest != entry.hash {
            return corrupt(format!("世代のハッシュが合いません: {}", entry.name));
        }
        Ok(bytes)
    }
    /// 量の上限（`Limits`。設定の予算から）を確かめる。
    fn check_limits<'a>(&self, entries: impl Iterator<Item = (&'a str, u64)>) -> R<()> {
        let Some(limits) = &self.limits else {
            return Ok(());
        };
        // .ylp の読み手と同じ数え方・同じ理由の文（どの予算かだけ。画面の英語はこの文で見分けて訳す）
        limits.check(entries).map_err(|e| match e {
            crate::Error::Budget(why) => StoreError::Budget(why),
            other => StoreError::Corrupt(other.to_string()),
        })
    }
    fn entry_path(&self, dir: &Path, manifest: &Manifest, entry: &Entry) -> PathBuf {
        if manifest.shared {
            self.content_path(&entry.hash)
        } else {
            entry.name.split('/').fold(dir.to_path_buf(), |p, c| p.join(c))
        }
    }

    /// 世代 1 つ（`id`。`None` は `current`）の中の小さなエントリだけを、長さとハッシュを確かめて読む（一覧用。全体の検証は
    /// `load`）。無ければ `None`。`max` を超えるものは断る。
    pub fn read_file(&self, id: Option<&str>, name: &str, max: u64) -> R<Option<Vec<u8>>> {
        validate_name(name)?;
        let id = match id {
            Some(id) => {
                validate_generation(id)?;
                id.to_owned()
            }
            None => self.current_id()?.ok_or(StoreError::NoGeneration)?,
        };
        let manifest_bytes = self.read_manifest_bytes(&id)?;
        let manifest = parse_manifest(&manifest_bytes)?;
        let Some(entry) = manifest.entries.iter().find(|e| e.name == name) else {
            return Ok(None);
        };
        if entry.len > max {
            return Err(StoreError::Budget(
                "復旧の情報が予算を超えています".into(),
            ));
        }
        let dir = self.generations_dir().join(&id);
        let path = self.entry_path(&dir, &manifest, entry);
        let (digest, bytes) = digest_file(&path, entry.len, true)?;
        if digest != entry.hash {
            return corrupt(format!("世代のハッシュが合いません: {}", entry.name));
        }
        Ok(bytes)
    }

    /// 世代を 1 つ確定する。作る → 確かめる → 世代の名前へ改名 → `previous` → `current` を**最後に**置き換える。失敗したら
    /// `current` は前のまま。確定の後の整理（`keep`）の失敗は握りつぶし、次の確定でやり直す。
    pub fn commit(&self, files: &Files, options: &CommitOptions<'_>) -> R<Committed> {
        if options.keep.is_some_and(|k| k < 2) {
            return Err(StoreError::InvalidArgument(
                "残す世代は 2 つ以上（今と 1 つ前）",
            ));
        }
        if files.is_empty() || !has_native(files.keys().map(String::as_str)) {
            return Err(StoreError::InvalidArgument("完全な正本が要ります"));
        }
        let (entry_max, total_max) = self.budget.unwrap_or((MAX_ONE_ENTRY, u64::MAX));
        let mut total = 0u64;
        for (name, data) in files {
            validate_name(name)?;
            if data.len() > entry_max.min(entry_limit(name)) {
                return Err(StoreError::Budget("エントリの予算を超えています".into()));
            }
            total = total.saturating_add(data.len());
        }
        if total > total_max {
            return Err(StoreError::Budget(
                "書き置きが作業の予算を超えています".into(),
            ));
        }
        self.check_limits(files.iter().map(|(n, b)| (n.as_str(), b.len())))?;
        fs::create_dir_all(&self.root)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(".save.lock"))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(StoreError::Busy),
            Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
        }
        let result = self.commit_locked(files, options);
        let _ = lock.unlock();
        result
    }

    fn commit_locked(&self, files: &Files, options: &CommitOptions<'_>) -> R<Committed> {
        self.check_expected(options.expected, true)?;
        let id = self.next_generation_id()?;
        let staging = self.root.join(format!(".staging-{id}"));
        let generations = self.generations_dir();
        fs::create_dir_all(&staging)?;
        fs::create_dir_all(&generations)?;
        let mut renamed = false;
        let result = self.write_generation(files, options, &id, &staging, &generations, &mut renamed);
        if result.is_err() && !renamed {
            // 確定していない作りかけだけを片付ける（前の世代・current には触れない）。残すと、整理が共有の中身を消せなくなる。
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn write_generation(
        &self,
        files: &Files,
        options: &CommitOptions<'_>,
        id: &str,
        staging: &Path,
        generations: &Path,
        renamed: &mut bool,
    ) -> R<Committed> {
        let share = options.share;
        let mut written = 0u64;
        let mut reused = 0usize;
        let mut verified: HashSet<String> = HashSet::new();
        // 札は 1 度だけ計算する（空きの確かめも、書く段も、同じ札を使う）。正本から作るエントリは、ここで 1 度作って数える
        let digests: Vec<String> = files
            .values()
            .map(|d| d.sha256().map_err(|e| StoreError::Corrupt(e.to_string())))
            .collect::<R<_>>()?;
        if let Some(guard) = &self.space {
            let mut needed = 0u64;
            let mut counted: HashSet<&str> = HashSet::new();
            for (data, digest) in files.values().zip(&digests) {
                let is_new = if share {
                    // 同じ中身が 2 つの名前にあっても 1 度しか書かない。置き場にあるものは書かない
                    counted.insert(digest.as_str())
                        && fs::symlink_metadata(self.content_path(digest))
                            .is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
                } else {
                    true
                };
                if is_new {
                    needed += data.len();
                }
            }
            guard(needed).map_err(StoreError::LowSpace)?;
        }
        let mut manifest = String::from(if share { SHARED } else { FLAT });
        manifest.push('\n');
        for ((name, data), digest) in files.iter().zip(digests) {
            if share {
                let path = self.content_path(&digest);
                fs::create_dir_all(path.parent().expect("contents の下"))?;
                match fs::symlink_metadata(&path) {
                    Ok(_) => {
                        let changed = || {
                            StoreError::Corrupt(
                                "共有の中身が、この道具の外で変わっています".into(),
                            )
                        };
                        if fs::symlink_metadata(&path)?.len() != data.len() {
                            return Err(changed());
                        }
                        if !self.known_good(&path, data.len()) {
                            if digest_file(&path, data.len(), false)?.0 != digest {
                                return Err(changed());
                            }
                            self.remember_good(&path);
                        }
                        verified.insert(digest.clone());
                        reused += 1;
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        let pending = staging.join(format!("{digest}.pending"));
                        write_blob_durable(&pending, data)?;
                        fs::rename(&pending, &path)?;
                        written += data.len();
                    }
                    Err(e) => return Err(e.into()),
                }
            } else {
                let path = name.split('/').fold(staging.to_path_buf(), |p, c| p.join(c));
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                write_blob_durable(&path, data)?;
                written += data.len();
            }
            manifest.push_str(&format!("{digest} {} {name}\n", data.len()));
            self.hit(&format!("file:{name}"))?;
        }
        let manifest_bytes = manifest.into_bytes();
        write_durable(&staging.join(MANIFEST), &manifest_bytes)?;
        // 確定の前に、書いたものを読み直して確かめる（この確定の中で確かめた共有の中身は読み直さない）
        let parsed = parse_manifest(&manifest_bytes)?;
        for entry in &parsed.entries {
            if verified.contains(&entry.hash) {
                continue;
            }
            self.read_entry(staging, &parsed, entry, false)?;
            if parsed.shared {
                self.remember_good(&self.content_path(&entry.hash));
            }
        }
        self.hit("verified")?;
        let committed_dir = generations.join(id);
        fs::rename(staging, &committed_dir)?;
        *renamed = true;
        self.hit("generation-renamed")?;
        // 置き換える直前に、current が期待した世代のままか（外の書き手が割り込んでいないか）もう一度見る
        self.check_expected(options.expected, false)?;
        let next = self.root.join(format!(".current-{}", random_hex()));
        write_durable(&next, format!("{id}\n").as_bytes())?;
        let switched = (|| -> R<()> {
            self.hit("before-pointer")?;
            let current = self.root.join("current");
            if let Some(old) = read_pointer(&current)? {
                // 1 つ前の世代の名前を残してから current を替える（current の置き換えは必ず最後）
                let keep_previous = self.root.join(format!(".previous-{}", random_hex()));
                write_durable(&keep_previous, format!("{old}\n").as_bytes())?;
                fs::rename(&keep_previous, self.root.join("previous"))?;
            }
            // 削除してから移動する代替手順は使わない。置換の失敗時は前の current が残る。
            fs::rename(&next, &current)?;
            Ok(())
        })();
        if let Err(e) = switched {
            let _ = fs::remove_file(&next);
            return Err(e);
        }
        let token = token_of(id, &manifest_bytes);
        self.hit("after-pointer")?;
        if let Some(keep) = options.keep {
            // 確定した後の整理。失敗しても新しい current は取り消さない
            let _ = self.prune(keep);
        }
        Ok(Committed {
            id: id.to_owned(),
            token,
            written_bytes: written,
            reused_files: reused,
        })
    }

    /// `expected` の確かめ。`full` なら `current` の全エントリのハッシュまで確かめ、そうでなければ `current` と manifest の札だけ。
    fn check_expected(&self, expected: Option<&str>, full: bool) -> R<()> {
        let current = self.current_id();
        match expected {
            None => match current {
                Ok(Some(_)) => Err(StoreError::AlreadyExists),
                Ok(None) => Ok(()),
                Err(e) => Err(e),
            },
            Some(want) => {
                let changed = |why: String| StoreError::Changed(why);
                let id = match current {
                    Ok(Some(id)) => id,
                    Ok(None) => return Err(changed("current がありません".into())),
                    Err(e) => return Err(changed(e.to_string())),
                };
                let token = if full {
                    self.read_generation(&id, false).map(|g| g.token)
                } else {
                    self.read_manifest_bytes(&id).map(|m| token_of(&id, &m))
                };
                match token {
                    Ok(t) if t == want => Ok(()),
                    Ok(_) => Err(changed("札が違います".into())),
                    // ハッシュ・長さの不一致など、外の改変で読めないのも「外で変わった」の一種
                    Err(StoreError::Io(e)) => Err(StoreError::Io(e)),
                    Err(e) => Err(changed(e.to_string())),
                }
            }
        }
    }

    /// 世代の名前。時刻のミリ秒は、置き場の中で必ず増える（同じミリ秒の 2 回の確定・時計の巻き戻りでも名前の順を保つ）。
    fn next_generation_id(&self) -> R<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let newest = self
            .current_id()
            .ok()
            .flatten()
            .and_then(|c| generation_time_ms(&c))
            .unwrap_or(0);
        let mut ms = now.max(newest + 1);
        loop {
            let last = LAST_MS.load(Ordering::Relaxed);
            ms = ms.max(last + 1);
            if LAST_MS
                .compare_exchange(last, ms, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }
        Ok(format!("{}-{}", utc_stamp(ms), random_hex()))
    }

    /// 世代の一覧（新しい順）。中身のハッシュは読まず、manifest と各エントリの存在・長さだけを見る。壊れた世代は
    /// `problem` に理由を付けて残す（読み飛ばすのは呼ぶ側）。置き場がまだ無ければ空。
    pub fn list(&self) -> R<Vec<GenerationInfo>> {
        let dir = self.generations_dir();
        let mut names = Vec::new();
        match fs::read_dir(&dir) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if validate_generation(&name).is_ok()
                        && entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    {
                        names.push(name);
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        }
        names.sort_by(|a, b| b.cmp(a));
        let current = self.current_id().ok().flatten();
        Ok(names
            .into_iter()
            .map(|id| {
                let mut info = GenerationInfo {
                    time_ms: generation_time_ms(&id),
                    shared: false,
                    documents: 0,
                    entries: 0,
                    bytes: 0,
                    is_current: current.as_deref() == Some(id.as_str()),
                    problem: None,
                    id,
                };
                if let Err(e) = self.fill_info(&mut info) {
                    info.problem = Some(e.to_string());
                }
                info
            })
            .collect())
    }
    fn fill_info(&self, info: &mut GenerationInfo) -> R<()> {
        let manifest = parse_manifest(&self.read_manifest_bytes(&info.id)?)?;
        let dir = self.generations_dir().join(&info.id);
        info.shared = manifest.shared;
        info.entries = manifest.entries.len();
        info.documents = manifest
            .entries
            .iter()
            .filter(|e| is_native_name(&e.name))
            .count();
        for entry in &manifest.entries {
            let path = self.entry_path(&dir, &manifest, entry);
            match fs::symlink_metadata(&path) {
                Ok(m) if m.is_file() && m.len() == entry.len => info.bytes += entry.len,
                Ok(_) => return corrupt(format!("世代の長さが合いません: {}", entry.name)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    return corrupt(format!("世代の中身がありません: {}", entry.name))
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// 置き場のフォルダ全体のバイト数（ファイルの長さの合計。作りかけ・どの世代も使わない中身・ポインタ・ロックのファイルを含む。
    /// シンボリックリンクは辿らない）。
    pub fn disk_bytes(&self) -> u64 {
        tree_bytes(&self.root)
    }

    /// 置き場がディスクで占めるものを数える（中身のハッシュは読まない。manifest とファイルの長さだけ）。置き場がまだ無ければ空。
    /// 消す順を決める側が、世代ごとの「消すと空く量」（自分だけが使う中身）を出せるよう、共有の中身の札と長さを世代ごとに返す。
    pub fn footprint(&self) -> R<Footprint> {
        let dir = self.generations_dir();
        let mut names = Vec::new();
        match fs::read_dir(&dir) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if validate_generation(&name).is_ok()
                        && entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    {
                        names.push(name);
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        names.sort_by(|a, b| b.cmp(a));
        let generations = names
            .into_iter()
            .map(|id| {
                let own_bytes = tree_bytes(&dir.join(&id));
                let parsed = self
                    .read_manifest_bytes(&id)
                    .and_then(|bytes| parse_manifest(&bytes));
                let (contents, problem) = match parsed {
                    Ok(manifest) if manifest.shared => {
                        let mut seen = HashSet::new();
                        let contents = manifest
                            .entries
                            .iter()
                            .filter(|e| seen.insert(e.hash.clone()))
                            .map(|e| (e.hash.clone(), e.len))
                            .collect();
                        (contents, None)
                    }
                    Ok(_) => (Vec::new(), None),
                    Err(e) => (Vec::new(), Some(e.to_string())),
                };
                GenerationFootprint {
                    time_ms: generation_time_ms(&id),
                    own_bytes,
                    contents,
                    problem,
                    id,
                }
            })
            .collect();
        Ok(Footprint {
            generations,
            total_bytes: tree_bytes(&self.root),
        })
    }

    /// 世代を 1 つ捨てる。`current`・`previous` がその世代を指していれば、残った一番新しい世代へ付け替えてから消す（残りが
    /// 無ければポインタも消す）。共有の中身は、どの世代も使わなくなったものだけ消す。
    pub fn remove_generation(&self, id: &str) -> R<()> {
        validate_generation(id)?;
        let dir = self.generations_dir().join(id);
        if fs::symlink_metadata(&dir).is_err() {
            return Ok(());
        }
        fs::create_dir_all(&self.root)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(".save.lock"))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(StoreError::Busy),
            Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
        }
        let result = self.remove_locked(id, &dir);
        let _ = lock.unlock();
        result
    }
    fn remove_locked(&self, id: &str, dir: &Path) -> R<()> {
        let mut remaining: Vec<String> = fs::read_dir(self.generations_dir())?
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != id && validate_generation(n).is_ok())
            .collect();
        remaining.sort_by(|a, b| b.cmp(a));
        // 付け替えが先、消すのが後（途中で落ちても、指す先の無いポインタを残さない）
        for pointer in ["current", "previous"] {
            let path = self.root.join(pointer);
            if read_pointer(&path)?.as_deref() != Some(id) {
                continue;
            }
            match remaining.first() {
                Some(next) => {
                    let pending = self.root.join(format!(".{pointer}-{}", random_hex()));
                    write_durable(&pending, format!("{next}\n").as_bytes())?;
                    fs::rename(&pending, &path)?;
                }
                None => fs::remove_file(&path)?,
            }
        }
        // manifest を先に消す（途中で落ちた世代は、壊れた世代として読み飛ばされる）
        let _ = fs::remove_file(dir.join(MANIFEST));
        fs::remove_dir_all(dir)?;
        let _ = self.prune_contents();
        Ok(())
    }

    /// 落ちた書き込みの残りを片付ける。`.save.lock` を取れたとき（この置き場へ書いている道具が今は無いとき）だけ、世代にならなかった
    /// `.staging-*` フォルダ（作りかけの世代。`<札>.pending` を含む）を消し、そのあと、どの世代も使わない共有の中身を消す。
    /// 書き込みは確定の間ずっとロックを持つので、ロックが取れて残っている `.staging-*` は、落ちた・パニックした書き込みの残り。
    /// 世代（`current`・`previous` が指すものを含む）には触れない。ロックを持たれていれば `Busy`。返すのは減ったバイト数。
    /// 残りが 1 つでもあると `prune_contents` は何も消せないので、世代を消して量を減らす側は、先にこれを呼ぶ。
    pub fn reclaim_abandoned(&self) -> R<u64> {
        if !self.root.is_dir() {
            return Ok(0);
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(".save.lock"))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(StoreError::Busy),
            Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
        }
        let before = self.disk_bytes();
        let result = self.reclaim_locked();
        let _ = lock.unlock();
        result?;
        Ok(before.saturating_sub(self.disk_bytes()))
    }
    fn reclaim_locked(&self) -> R<()> {
        for entry in fs::read_dir(&self.root)?.filter_map(|e| e.ok()) {
            if !entry.file_name().to_string_lossy().starts_with(".staging-") {
                continue;
            }
            // 本物のフォルダだけ（シンボリックリンクは辿らない）
            if fs::symlink_metadata(entry.path()).is_ok_and(|m| m.is_dir()) {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
        self.prune_contents()
    }

    /// 超えた分の古い世代を消す。守るのは `current`・`previous` と、新しい順に数が `keep` になるまで。
    fn prune(&self, keep: usize) -> R<()> {
        let mut protected = BTreeSet::new();
        for pointer in ["current", "previous"] {
            if let Some(id) = read_pointer(&self.root.join(pointer))? {
                protected.insert(id);
            }
        }
        // 世代の名前の形のフォルダだけを数える（`.DS_Store`・`desktop.ini`・手で置いたものは、`list` と同じく読み飛ばし、
        // 整理を止めない・消さない）
        let mut candidates: Vec<String> = fs::read_dir(self.generations_dir())?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| validate_generation(n).is_ok() && generation_time_ms(n).is_some())
            .collect();
        candidates.sort_by(|a, b| b.cmp(a));
        let mut retained = protected;
        for name in &candidates {
            if retained.len() < keep {
                retained.insert(name.clone());
            }
        }
        for name in candidates.iter().filter(|n| !retained.contains(*n)) {
            if self.hit(&format!("prune-generation:{name}")).is_err() {
                continue;
            }
            let dir = self.generations_dir().join(name);
            let _ = fs::remove_file(dir.join(MANIFEST));
            let _ = fs::remove_dir_all(dir);
        }
        self.prune_contents()
    }

    /// どの世代（作りかけを含む）も使わない共有の中身を消す。manifest が無い・読めない世代があれば、何も消さない。
    fn prune_contents(&self) -> R<()> {
        let contents = self.root.join("contents");
        if !contents.is_dir() {
            return Ok(());
        }
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Ok(entries) = fs::read_dir(self.generations_dir()) {
            dirs.extend(entries.filter_map(|e| e.ok()).map(|e| e.path()));
        }
        for entry in fs::read_dir(&self.root)?.filter_map(|e| e.ok()) {
            if entry.file_name().to_string_lossy().starts_with(".staging-") {
                dirs.push(entry.path());
            }
        }
        let mut referenced: HashSet<String> = HashSet::new();
        for dir in dirs {
            let Ok(bytes) = read_bounded(&dir.join(MANIFEST), MANIFEST_LIMIT) else {
                return Ok(());
            };
            let Ok(text) = std::str::from_utf8(&bytes) else {
                return Ok(());
            };
            let mut lines = text.split('\n');
            match lines.next() {
                Some(FLAT) => continue,
                Some(SHARED) => {}
                _ => return Ok(()),
            }
            for line in lines.filter(|l| !l.is_empty()) {
                let parts: Vec<&str> = line.split(' ').collect();
                if parts.len() != 3 || !is_hash(parts[0]) {
                    return Ok(());
                }
                referenced.insert(parts[0].to_owned());
            }
        }
        for entry in fs::read_dir(&contents)?.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("bin") {
                continue;
            }
            let Some(digest) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !is_hash(digest) || referenced.contains(digest) {
                continue;
            }
            if self.hit(&format!("prune-content:{digest}")).is_err() {
                continue;
            }
            let _ = fs::remove_file(&path);
        }
        Ok(())
    }
}

/// フォルダの中のファイルの長さの合計（シンボリックリンクは辿らず、読めないものは数えない）。
fn tree_bytes(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }
    total
}

fn token_of(id: &str, manifest: &[u8]) -> String {
    format!("{id}:{}", hash(manifest))
}

/// ポインタのファイル（世代の名前 + 改行）。無ければ `None`。
fn read_pointer(path: &Path) -> R<Option<String>> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let bytes = read_bounded(path, 256)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| StoreError::Corrupt("ポインタが UTF-8 ではありません".into()))?;
    let id = text.trim();
    validate_generation(id)?;
    Ok(Some(id.to_owned()))
}

fn parse_manifest(bytes: &[u8]) -> R<Manifest> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| StoreError::Corrupt("manifest が UTF-8 ではありません".into()))?;
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() < 2 || (lines[0] != FLAT && lines[0] != SHARED) {
        return corrupt("未対応の manifest です");
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut names: HashSet<&str> = HashSet::new();
    let mut total = 0u64;
    for line in &lines[1..] {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split(' ').collect();
        let len = parts.get(1).and_then(|s| s.parse::<u64>().ok());
        let (true, Some(len)) = (parts.len() == 3 && is_hash(parts[0]), len) else {
            return corrupt("manifest の行が不正です");
        };
        if len > entry_limit(parts[2]) {
            return corrupt("manifest の行が不正です");
        }
        validate_name(parts[2])?;
        if !names.insert(parts[2]) {
            return corrupt("manifest にエントリが重複しています");
        }
        total = total.saturating_add(len);
        entries.push(Entry {
            hash: parts[0].to_owned(),
            len,
            name: parts[2].to_owned(),
        });
    }
    if !has_native(entries.iter().map(|e| e.name.as_str())) {
        return corrupt("世代に正本がありません");
    }
    Ok(Manifest {
        shared: lines[0] == SHARED,
        entries,
    })
}

fn is_native_name(name: &str) -> bool {
    name == NATIVE
        || name
            .strip_prefix("sets/")
            .and_then(|s| s.split_once('/'))
            .is_some_and(|(id, leaf)| valid_id(id) && leaf == NATIVE)
}
fn has_native<'a>(mut names: impl Iterator<Item = &'a str>) -> bool {
    names.any(is_native_name)
}

/// 世代に入れてよいエントリの名前（`.ylp` のエントリ名と同じ範囲。`/` で区切った 4 つまで、1 つ 80 文字まで、英数字と `.-_`）。
fn validate_name(name: &str) -> R<()> {
    let bad = || StoreError::Corrupt("世代に入れられないエントリ名です".into());
    if name.is_empty() || name.len() > 96 {
        return Err(bad());
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() > 4 || parts.last() == Some(&MANIFEST) {
        return Err(bad());
    }
    for part in parts {
        if part.is_empty()
            || part.len() > 80
            || part.starts_with('.')
            || part.contains("..")
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        {
            return Err(bad());
        }
    }
    Ok(())
}
fn validate_generation(id: &str) -> R<()> {
    if id.is_empty()
        || id.len() > 80
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return corrupt("不正な世代の名前です");
    }
    Ok(())
}

fn read_bounded(path: &Path, max: u64) -> R<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() {
        return corrupt("通常のファイルではありません");
    }
    if meta.len() > max {
        return Err(StoreError::Budget("ファイルが許す大きさを超えています".into()));
    }
    let mut buf = Vec::with_capacity(meta.len() as usize);
    File::open(path)?.take(max + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 != meta.len() {
        return Err(StoreError::Io(io::Error::other(
            "読み込み中にファイルが変わりました",
        )));
    }
    Ok(buf)
}

/// 新しいファイルにエントリを流して書いて `sync_all` する（あるファイルは上書きしない）。
fn write_blob_durable(path: &Path, blob: &Blob) -> R<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut out = io::BufWriter::with_capacity(1 << 20, file);
    blob.write_to(&mut out).map_err(|e| match e {
        crate::Error::Io(e) => StoreError::Io(e),
        other => StoreError::Corrupt(other.to_string()),
    })?;
    let file = out.into_inner().map_err(|e| e.into_error())?;
    file.sync_all()?;
    Ok(())
}
/// ファイルの SHA-256 を流して数える（長さが `len` と違えば断る）。`keep` なら中身も返す（小さなもの）。
fn digest_file(path: &Path, len: u64, keep: bool) -> R<(String, Option<Vec<u8>>)> {
    use sha2::{Digest, Sha256};
    let mut f = io::BufReader::with_capacity(1 << 20, File::open(path)?);
    let mut sha = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut kept = keep.then(|| Vec::with_capacity(len.min(1 << 20) as usize));
    let mut n_total = 0u64;
    loop {
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        n_total += n as u64;
        if n_total > len {
            return Err(StoreError::Io(io::Error::other(
                "読み込み中にファイルが変わりました",
            )));
        }
        sha.update(&buf[..n]);
        if let Some(k) = &mut kept {
            k.extend_from_slice(&buf[..n]);
        }
    }
    if n_total != len {
        return Err(StoreError::Io(io::Error::other(
            "読み込み中にファイルが変わりました",
        )));
    }
    Ok((format!("{:x}", sha.finalize()), kept))
}
/// 1 エントリの上限（正本の部分は小さい）。
fn entry_limit(name: &str) -> u64 {
    if part_number(name).is_some() {
        MAX_PART_BYTES
    } else {
        MAX_ONE_ENTRY
    }
}
/// 新しいファイルに書いて `sync_all` する（あるファイルは上書きしない）。
fn write_durable(path: &Path, bytes: &[u8]) -> R<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// 乱数 128 bit の 16 進 32 桁（世代の名前・一時ファイルの名前）。
fn random_hex() -> String {
    use std::hash::{BuildHasher, Hasher};
    let state = std::collections::hash_map::RandomState::new();
    let count = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut a = state.build_hasher();
    a.write_u64(count);
    a.write_u32(std::process::id());
    let mut b = state.build_hasher();
    b.write_u64(!count);
    b.write_u8(0x5a);
    format!("{:016x}{:016x}", a.finish(), b.finish())
}

/// UTC の Unix ミリ秒 → `yyyyMMddTHHmmssfff`。
pub fn utc_stamp(ms: u64) -> String {
    let secs = ms / 1000;
    let (days, rest) = ((secs / 86400) as i64, secs % 86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}{:03}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60,
        ms % 1000
    )
}

/// 世代の名前から作った時刻（UTC の Unix ミリ秒）。読めなければ `None`。
pub fn generation_time_ms(id: &str) -> Option<u64> {
    let stamp = id.split('-').next()?;
    let b = stamp.as_bytes();
    if b.len() != 18 || b[8] != b'T' || !b.iter().enumerate().all(|(i, c)| i == 8 || c.is_ascii_digit())
    {
        return None;
    }
    let n = |r: std::ops::Range<usize>| stamp[r].parse::<u64>().ok();
    let (y, m, d) = (n(0..4)? as i64, n(4..6)? as u32, n(6..8)? as u32);
    let (h, mi, s, ms) = (n(9..11)?, n(11..13)?, n(13..15)?, n(15..18)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let days = days_from_civil(y, m, d);
    if days < 0 {
        return None;
    }
    Some(((days as u64 * 86400 + h * 3600 + mi * 60 + s) * 1000) + ms)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 世代に添える一覧用の情報のエントリ名（`.ylp` のエントリではない。開くときは外す）。
pub const INFO_NAME: &str = "recovery.json";
/// 一覧用の情報の大きさの上限。
pub const INFO_LIMIT: u64 = 64 * 1024;

/// 世代に添える `recovery.json`（一覧に出す名前など）。Unity 版の `RecoveryCatalog.Info`（title・projectPath・projectToken・
/// unchanged）と同じキーに、セットの数（`sets`）を足した。読むときは足りないキーを既定で補い、知らないキーは無視する。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecoveryInfo {
    pub title: String,
    /// 元の `.ylp` のパス（無ければ空）。
    pub project_path: String,
    pub project_token: String,
    /// 書いた時点で保存済みの `.ylp` と同じだったか。
    pub unchanged: bool,
    pub sets: usize,
}
impl RecoveryInfo {
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::json!({
            "title": self.title,
            "projectPath": self.project_path,
            "projectToken": self.project_token,
            "unchanged": self.unchanged,
            "sets": self.sets,
        })
        .to_string()
        .into_bytes()
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, StoreError> {
        if bytes.len() as u64 > INFO_LIMIT {
            return Err(StoreError::Budget("復旧の情報が予算を超えています".into()));
        }
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|_| StoreError::Corrupt("復旧の情報を読めません".into()))?;
        let text = |key: &str| value.get(key).and_then(|v| v.as_str()).unwrap_or("").to_owned();
        if !value.is_object() {
            return corrupt("復旧の情報を読めません");
        }
        Ok(Self {
            title: text("title"),
            project_path: text("projectPath"),
            project_token: text("projectToken"),
            unchanged: value.get("unchanged").and_then(|v| v.as_bool()).unwrap_or(false),
            sets: value.get("sets").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        })
    }
}
