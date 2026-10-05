//! 大きな .ylp の外側（zip・mimetype・manifest）を、ファイル全体・zip 全体をメモリに置かずに読み書きする。
//!
//! - 読む（[`Package::open`]）: 末尾の中央ディレクトリ（zip64 を含む）から目録を作り、manifest を読み、全エントリを 1 回流して
//!   長さ・CRC-32・SHA-256 を確かめる。小さなエントリ（[`KEEP_IN_MEMORY`] 以下）はそのときメモリに残し、ほかはファイルの中の位置
//!   （[`Blob`]）で持って、要るときに位置から流して読む（読むたびに長さと SHA-256 を確かめ直す）。位置から読むために、開いたファイルの
//!   ハンドルを持ち続ける（パスでは開き直さない。外で改名・移動・削除・別のファイルへの置き換えをされても、開いたときの中身を読む）。
//! - 書く（[`Package::write_to`]）: 2 回に分ける。1 回目は各エントリの長さと SHA-256 を数え（作るエントリは作りながら数える）、manifest を
//!   作る。2 回目に mimetype・manifest・名前の順に流して書く（ローカルヘッダーは中身を書いた後に戻って直す。`YLP-4` は同じ中身の圧縮した
//!   バイト列を写し、大きなエントリは並べて圧縮する）。今の上限（`YLP-3`。
//!   1 エントリ 512 MiB・合計 768 MiB・1000 エントリ）に収まるときは、今の [`crate::Archive::to_bytes`] とバイトまで同じ。収まらないときだけ
//!   `YLP-4` で書き、zip64 は要るとき（4 GiB を超える位置・65535 を超えるエントリ）だけ使う。
//!
//! メモリの上の zip（`.ylsmart`・`.ylbrush`・棚）は今までどおり [`crate::Archive`] が読み書きする。
use crate::{archive, check, check_budget, hash, is_hash, Error, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt,
    fs::File,
    io::{self, BufReader, Cursor, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock, Weak},
};

/// 正本の部分（`document.utpaint.<n>`。正本の版 26）の 1 エントリの上限。
pub const MAX_PART_BYTES: u64 = 256 << 20;
/// ほかの 1 エントリの上限（`YLP-3` と同じ）。
pub const MAX_ONE_ENTRY: u64 = 512 << 20;
/// `YLP-4` のエントリの数の上限（mimetype と manifest を除く）。zip の 16 bit の数に収める（数のための zip64 は、試験が閾値を
/// 下げたときだけ通る）。
pub const MAX_ENTRIES: usize = 65_000;
/// `YLP-3` の上限（今の形。これに収まるプロジェクトは今と同じバイト列で書く）。
pub const CLASSIC_ENTRY_BYTES: u64 = 512 << 20;
pub const CLASSIC_TOTAL_BYTES: u64 = 768 << 20;
pub const CLASSIC_ENTRIES: usize = 1000;
/// 開くとき、確かめながらメモリに残すエントリの大きさ（JSON・選択範囲など）。これより大きなものはファイルの位置で持つ。
pub const KEEP_IN_MEMORY: u64 = 1 << 20;
/// 正本のほかのエントリの合計の上限（`YLP-3` の合計と同じ。正本でないエントリは今の上限のまま）。
pub const OTHER_BYTES: u64 = 768 << 20;
/// 正本の長さを「レイヤーのメモリ」の予算の何倍まで受けるか（一様なタイルは core では 4 バイトだが、正本では全画素を書くため）。
pub const DOCUMENT_FACTOR: u64 = 4;
const MANIFEST: &str = "manifest.sha256";
const YLP_MIME: &str = "application/x-yolupainter";
const YLP_PREFIX: &str = "YOLUPAINTER-YLP-";
/// `YLP-3` の manifest の上限（今と同じ）。
const CLASSIC_MANIFEST: u64 = 1 << 20;
/// `YLP-4` の manifest の上限（65000 行 × 1 行 175 バイトほど）。
const MAX_MANIFEST: u64 = 16 << 20;
/// 中央ディレクトリの上限（エントリ 65002 個 × 名前 96 バイトと拡張に余裕を見る）。
const MAX_CENTRAL: u64 = 32 << 20;

/// 読むときの上限（設定の予算から決める）。
///
/// - 1 エントリ: 正本の部分は [`MAX_PART_BYTES`]、ほかは [`MAX_ONE_ENTRY`]（どの予算にも依らない形の上限）。
/// - セットごとの正本（`document.utpaint` と部分の長さの合計）: `document_bytes`。設定の「レイヤーのメモリ」の予算（`load_source_bytes`）の
///   [`DOCUMENT_FACTOR`] 倍。
/// - 全体: 正本のあるセットの数 × `document_bytes` ＋ `other_bytes`（正本でないエントリは今の 768 MiB）。
/// - エントリの数: [`MAX_ENTRIES`]。名前は今と同じ（96 文字・英数字と `. - _`）。
///
/// `YLP-3` の manifest のファイルは、今の上限（[`CLASSIC_ENTRY_BYTES`]・[`CLASSIC_TOTAL_BYTES`]・[`CLASSIC_ENTRIES`]）で読む。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// セットごとの正本（ヘッダーと部分）の長さの合計の上限。
    pub document_bytes: u64,
    /// 正本でないエントリの長さの合計の上限（全体の上限は、正本の分にこれを足したもの）。
    pub other_bytes: u64,
}
impl Limits {
    /// 設定の「レイヤーのメモリ」の予算（1 つの文書の層の画素に許すバイト数）から。core の既定（256 MiB）を下回らない。
    pub fn from_layer_pixels(layer_pixels: u64) -> Self {
        let pixels = layer_pixels.max(yolu_core::DEFAULT_SOURCE_BUDGET_BYTES);
        Self {
            document_bytes: pixels.saturating_mul(DOCUMENT_FACTOR),
            other_bytes: OTHER_BYTES,
        }
    }
    /// 書いたものを読み直して確かめるときの上限: 予算に依らず、形の上限だけ（自分で書いた大きな文書を、予算で保存できなくしない）。
    pub fn unbounded() -> Self {
        Self {
            document_bytes: u64::MAX,
            other_bytes: u64::MAX,
        }
    }
}
impl Default for Limits {
    fn default() -> Self {
        Self::from_layer_pixels(yolu_core::DEFAULT_SOURCE_BUDGET_BYTES)
    }
}
impl Limits {
    /// エントリ（名前と長さ）が、この上限で読めるか（`YLP-4` の .ylp と復旧の世代の読み手と同じ数え方。超えれば `Error::Budget` で
    /// [`OVER_LAYER_PIXELS_DOCUMENT`] か [`OVER_LAYER_PIXELS_TOTAL`]）。保存した .ylp を、今の予算で開き直せるかの見積もりにも使う。
    pub fn check<'a>(&self, entries: impl IntoIterator<Item = (&'a str, u64)>) -> Result<()> {
        let mut tally = LimitTally::new(self);
        for (name, len) in entries {
            tally.add(name, len)?;
        }
        tally.finish()
    }
}
/// 予算で断る理由（画面の文）。どの予算かだけを言い、数・内部の識別子・上限の導き方は入れない（英語の画面はこの文で見分けて訳す）。
pub const OVER_LAYER_PIXELS_DOCUMENT: &str = "正本が「レイヤーのメモリ」の予算を超えています";
pub const OVER_LAYER_PIXELS_TOTAL: &str = "全体が「レイヤーのメモリ」の予算を超えています";
/// 1 エントリの形の上限（正本の部分 256 MiB・ほか 512 MiB）を超えた。
const ONE_ENTRY_OVER: &str = "1エントリの上限を超えています";
/// 量の上限（[`Limits`]）の数え。.ylp の manifest・復旧の世代・保存の見積もりが同じ数え方を使う。
pub(crate) struct LimitTally<'l> {
    limits: &'l Limits,
    /// セットの置き場（根なら空）→ 正本（ヘッダーと部分）の長さの合計。
    documents: BTreeMap<String, u64>,
    total: u64,
}
impl<'l> LimitTally<'l> {
    pub(crate) fn new(limits: &'l Limits) -> Self {
        Self {
            limits,
            documents: BTreeMap::new(),
            total: 0,
        }
    }
    /// 1 エントリを数える（セットごとの正本は、ここで上限を超えたら断る）。
    pub(crate) fn add(&mut self, name: &str, len: u64) -> Result<()> {
        self.total = self.total.saturating_add(len);
        if let Some(owner) = document_owner(name) {
            let sum = self.documents.entry(owner.to_owned()).or_default();
            *sum = sum.saturating_add(len);
            check_budget(*sum <= self.limits.document_bytes, OVER_LAYER_PIXELS_DOCUMENT)?;
        }
        Ok(())
    }
    /// 全体（正本のあるセットの数 × セットごとの上限 ＋ 正本でないエントリの上限）。
    pub(crate) fn finish(&self) -> Result<()> {
        let allowance = (self.documents.len() as u64)
            .saturating_mul(self.limits.document_bytes)
            .saturating_add(self.limits.other_bytes);
        check_budget(self.total <= allowance, OVER_LAYER_PIXELS_TOTAL)
    }
}

/// 全エントリ（`.ylp` のエントリ名 → 中身）。`mimetype` と manifest は含まない。
pub type Files = BTreeMap<String, Blob>;

/// エントリの中身。メモリの中・.ylp の中の位置・復旧の置き場の中身のファイル・正本から作るもの のどれか。複製は安い（`Arc`）。
#[derive(Clone)]
pub struct Blob(Repr);
#[derive(Clone)]
enum Repr {
    Mem(Arc<[u8]>),
    Zip(Arc<ZipRef>),
    File(Arc<FileRef>),
    Made(Arc<dyn Made>),
}
/// .ylp の中のエントリの位置（確かめ済みの長さ・CRC・SHA-256 つき）。
pub(crate) struct ZipRef {
    source: Source,
    name: String,
    data: u64,
    packed: u64,
    len: u64,
    method: u16,
    crc: u32,
    sha: String,
}
/// 中身だけを入れたファイル（復旧の置き場の `contents/<SHA-256>.bin` など）。
pub(crate) struct FileRef {
    path: PathBuf,
    name: String,
    len: u64,
    sha: String,
    /// ファイルを置いた場所の持ち主（[`Blob::hold_in`]）。このエントリと、その読み手が生きている間は手放さない（手放すと片付く）。
    keep: Option<Keep>,
}
/// 置いた場所の持ち主（最後の 1 つが手放されたときに片付ける物）。
pub type Keep = Arc<dyn std::any::Any + Send + Sync>;
/// 作るエントリ（core の文書から正本を作るなど）。長さは作る前に分かり、SHA-256 は 1 度作って数える。
pub(crate) trait Made: Send + Sync {
    fn len(&self) -> u64;
    fn sha256(&self) -> Result<String>;
    fn write_to(&self, out: &mut dyn Write) -> Result<()>;
    fn describe(&self) -> String;
}
/// .ylp の置き場（開いたファイルかメモリの中のバイト列）。
#[derive(Clone)]
pub(crate) enum Source {
    File(Arc<OpenFile>),
    Bytes(Arc<[u8]>),
}
trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}
impl Source {
    /// ファイルを開いて置き場にする（読むのはこのハンドルから、位置を指定して。パスは付け替えと手放しの目印で、読むときは使わない）。
    pub(crate) fn open_file(path: &Path) -> io::Result<Self> {
        Ok(Self::File(OpenFile::open(path)?))
    }
    fn open(&self) -> Result<Box<dyn ReadSeek>> {
        Ok(match self {
            Self::File(f) => {
                let file = f.handle()?;
                let len = file.metadata()?.len();
                Box::new(BufReader::with_capacity(1 << 16, At { file, pos: 0, len }))
            }
            Self::Bytes(b) => Box::new(Cursor::new(b.clone())),
        })
    }
    /// 保存で置き換えた後の名前へ付け替える（同じファイルのまま。ハンドルは置換のあとも同じファイルを指す）。
    fn set_path(&self, path: &Path) {
        if let Self::File(f) = self {
            *lock(&f.path) = path.to_path_buf();
        }
    }
}
/// 開いた .ylp のハンドル。ファイルの中の位置から読む（`read_at` / `seek_read`。ファイルのカーソルは使わないので、同時に何本読んでも
/// 互いに動かさない）。標準の開き方（Windows は読み・書き・削除を共有）なので、外の改名・移動・削除・POSIX の置換は妨げず、ハンドルは
/// 開いたときの中身を読み続ける。
pub(crate) struct OpenFile {
    /// 今の名前（保存で置き換えた後は新しい名前へ付け替える）。手放す・掴み直す相手を探す目印で、読むときは使わない。
    path: Mutex<PathBuf>,
    /// ハンドル。`None` は、保存の置換のために手放した（置換できなかった形は掴み直す。置換できたなら、もう開いたときの中身ではない）。
    handle: RwLock<Option<Arc<File>>>,
    /// 開いたときのファイルの長さ（掴み直すとき、同じファイルか見分ける目安。更新時刻は見ない: 同期の道具が中身を変えずに更新時刻だけを
    /// 変えることがある。違うファイルを掴んでも、読むたびの長さ・SHA-256・CRC の確かめで断る）。
    opened: u64,
}
/// 開いているハンドルの一覧（手放す相手を、置換するパスから探す）。生きているものだけ。
static OPEN_FILES: Mutex<Vec<Weak<OpenFile>>> = Mutex::new(Vec::new());
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
impl OpenFile {
    fn open(path: &Path) -> io::Result<Arc<Self>> {
        let file = File::open(path)?;
        let m = file.metadata()?;
        let me = Arc::new(Self {
            path: Mutex::new(path.to_path_buf()),
            handle: RwLock::new(Some(Arc::new(file))),
            opened: m.len(),
        });
        let mut all = lock(&OPEN_FILES);
        all.retain(|w| w.strong_count() > 0);
        all.push(Arc::downgrade(&me));
        Ok(me)
    }
    fn handle(&self) -> Result<Arc<File>> {
        match &*self.handle.read().unwrap_or_else(|e| e.into_inner()) {
            Some(file) => Ok(file.clone()),
            None => Err(Error::SaveConflict(SOURCE_RELEASED.into())),
        }
    }
}
/// `path` を開いている（生きている）ハンドルを手放す。保存の置換が、自分たちの開いたハンドルのせいで通らないファイルシステム（置換の
/// 規則が POSIX でない FAT・exFAT・一部のネットワーク）のための最後の手段。手放したものは、置換に失敗したら [`Released::reacquire`]
/// で掴み直し、置換できたら手放したまま（もう開いたときの中身ではないので、読もうとすると [`SOURCE_RELEASED`] で断る）。
pub(crate) fn release_at(path: &Path) -> Released {
    let mut found = Vec::new();
    for w in lock(&OPEN_FILES).iter() {
        if let Some(f) = w.upgrade() {
            if same_path(&lock(&f.path), path) {
                *f.handle.write().unwrap_or_else(|e| e.into_inner()) = None;
                found.push(f);
            }
        }
    }
    Released(found)
}
/// 手放したハンドル（[`release_at`]）。
pub(crate) struct Released(Vec<Arc<OpenFile>>);
impl Released {
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// 置換に失敗して元のファイルが残ったとき、掴み直す（開いたときと長さが同じファイルだけ。違えば手放したまま）。
    pub(crate) fn reacquire(self) {
        for f in self.0 {
            let path = lock(&f.path).clone();
            let same = File::open(&path)
                .ok()
                .filter(|file| file.metadata().is_ok_and(|m| m.len() == f.opened));
            if let Some(file) = same {
                *f.handle.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(file));
            }
        }
    }
}
/// 同じ名前か（Windows は大文字小文字を区別しない）。パスの書き方の違い（`.` や `..`）までは見ない。
fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}
/// ファイルの中の位置から読む（`Read` + `Seek`）。
struct At {
    file: Arc<File>,
    pos: u64,
    len: u64,
}
impl Read for At {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match read_at(&self.file, buf, self.pos) {
                Ok(n) => {
                    self.pos += n as u64;
                    return Ok(n);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }
}
impl Seek for At {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = pos.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "ファイルの先頭より前へは動かせません"))?;
        Ok(self.pos)
    }
}
#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], pos: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, pos)
}
#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], pos: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, pos)
}

impl Blob {
    pub(crate) fn made(made: Arc<dyn Made>) -> Self {
        Self(Repr::Made(made))
    }
    /// 中身のファイル（長さと SHA-256 は呼び手が確かめたもの）。
    pub(crate) fn file(path: PathBuf, name: &str, len: u64, sha: String) -> Self {
        Self(Repr::File(Arc::new(FileRef {
            path,
            name: name.into(),
            len,
            sha,
            keep: None,
        })))
    }
    /// 中身を `dir` の中のファイル（`<SHA-256>.bin`）に置き、そこから読むエントリにする。今の置き場（復旧の世代・開いた .ylp）が後で
    /// 消えても読めるように（復旧の世代から開いたプロジェクト）。中身のファイルは、まずハードリンクにし（写さない。置き場の中身は書き換え
    /// られず、消されてもリンクが中身を残す）、できなければ（別のボリューム・リンクの無いファイルシステム）流して写す（長さと SHA-256 を
    /// 確かめながら。メモリに全部を持たない）。メモリの中身はそのまま返す。`keep` は `dir` の持ち主で、このエントリと読み手が生きている
    /// 間は手放さない。読むたびに長さと SHA-256 を確かめるのは、ほかのファイルのエントリと同じ。
    pub fn hold_in(&self, dir: &Path, keep: &Keep) -> Result<Self> {
        if self.in_memory().is_some() {
            return Ok(self.clone());
        }
        let (len, sha, name) = (self.len(), self.sha256()?, self.label());
        check(is_hash(&sha), format!("中身の札が不正です: {name}"))?;
        let target = dir.join(format!("{sha}.bin"));
        if std::fs::symlink_metadata(&target).is_err() {
            let linked = match &self.0 {
                Repr::File(f) => std::fs::hard_link(&f.path, &target).is_ok(),
                _ => false,
            };
            if !linked {
                let pending = dir.join(format!("{sha}.pending"));
                let copied = (|| -> Result<()> {
                    let mut out = io::BufWriter::with_capacity(1 << 20, File::create(&pending)?);
                    self.write_to(&mut out)?;
                    out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
                    std::fs::rename(&pending, &target)?;
                    Ok(())
                })();
                if copied.is_err() {
                    let _ = std::fs::remove_file(&pending);
                }
                copied?;
            }
        }
        Ok(Self(Repr::File(Arc::new(FileRef {
            path: target,
            name,
            len,
            sha,
            keep: Some(keep.clone()),
        }))))
    }
    /// 診断のための名前（エントリ名。メモリと作るものは種類）。
    fn label(&self) -> String {
        match &self.0 {
            Repr::Mem(_) => "メモリ".into(),
            Repr::Zip(z) => z.name.clone(),
            Repr::File(f) => f.name.clone(),
            Repr::Made(m) => m.describe(),
        }
    }
    /// バイト数。
    pub fn len(&self) -> u64 {
        match &self.0 {
            Repr::Mem(b) => b.len() as u64,
            Repr::Zip(z) => z.len,
            Repr::File(f) => f.len,
            Repr::Made(m) => m.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// 中身の SHA-256（16 進 64 桁）。作るエントリは、初めて聞かれたときに 1 度作って数える。
    pub fn sha256(&self) -> Result<String> {
        match &self.0 {
            Repr::Mem(b) => Ok(hash(b)),
            Repr::Zip(z) => Ok(z.sha.clone()),
            Repr::File(f) => Ok(f.sha.clone()),
            Repr::Made(m) => m.sha256(),
        }
    }
    /// 同じ中身の持ち方か（複製したものどうし）。中身を読まずに、変わっていないエントリを見分ける。
    pub(crate) fn same(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Repr::Mem(a), Repr::Mem(b)) => Arc::ptr_eq(a, b),
            (Repr::Zip(a), Repr::Zip(b)) => Arc::ptr_eq(a, b),
            (Repr::File(a), Repr::File(b)) => Arc::ptr_eq(a, b),
            (Repr::Made(a), Repr::Made(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    /// メモリの中の中身（メモリに無ければ None）。
    pub fn in_memory(&self) -> Option<&[u8]> {
        match &self.0 {
            Repr::Mem(b) => Some(b),
            _ => None,
        }
    }
    /// 中身をメモリに読む（小さなエントリのため。正本の部分など大きなものは `reader`・`write_to` で流す）。ファイルのものは長さと
    /// SHA-256 を確かめ直す。
    pub fn bytes(&self) -> Result<Arc<[u8]>> {
        if let Repr::Mem(b) = &self.0 {
            return Ok(b.clone());
        }
        check_budget(
            self.len() <= MAX_ONE_ENTRY,
            "エントリが大きすぎてメモリに読めません",
        )?;
        let mut out = Vec::with_capacity(self.len() as usize);
        self.write_to(&mut out)?;
        Ok(Arc::from(out))
    }
    /// 中身を流して読む（終わりまで読むと、長さと SHA-256・CRC を確かめる。合わなければ `InvalidData` の io エラー）。作るエントリは
    /// 流して読めない（`write_to` を使う）。
    pub fn reader(&self) -> Result<Box<dyn Read + Send>> {
        match &self.0 {
            Repr::Mem(b) => Ok(Box::new(Cursor::new(b.clone()))),
            Repr::Zip(z) => z.reader(),
            Repr::File(f) => {
                let file = File::open(&f.path)?;
                Ok(Box::new(Kept {
                    inner: Verify::new(
                        BufReader::with_capacity(1 << 16, file),
                        f.name.clone(),
                        f.len,
                        &f.sha,
                        None,
                    ),
                    _keep: f.keep.clone(),
                }))
            }
            Repr::Made(m) => Err(Error::InvalidData(format!(
                "作るエントリは流して読めません: {}",
                m.describe()
            ))),
        }
    }
    /// 中身を `out` へ書く（ファイルのものは確かめながら）。
    pub fn write_to(&self, out: &mut dyn Write) -> Result<()> {
        match &self.0 {
            Repr::Mem(b) => Ok(out.write_all(b)?),
            Repr::Made(m) => m.write_to(out),
            _ => {
                let mut r = self.reader()?;
                copy(&mut r, out)
            }
        }
    }
    /// .ylp の中の位置で持つエントリの置き場の名前を、保存で置き換えた後の名前へ付け替える（同じファイルのまま。読むのは開いたハンドルで、
    /// 置換のあとも同じファイルを指すので、向け直さない。エントリは同じ `Arc` のままなので、`same` で変わっていないと見分けられ、
    /// 正本の骨組みを読み直さない）。
    pub(crate) fn note_path(&self, path: &Path) {
        if let Repr::Zip(z) = &self.0 {
            z.source.set_path(path);
        }
    }
}
/// 開いた .ylp が、保存の置換のために手放された（置換の規則が POSIX でないファイルシステムで、自分の保存が置き換えた。古い版の写しは
/// もう読めない。保存した後のプロジェクトを使う）。
pub const SOURCE_RELEASED: &str = "開いた .ylp は保存で置き換えられたため、この写しの中身はもう読めません";
impl ZipRef {
    fn reader(&self) -> Result<Box<dyn Read + Send>> {
        let mut f = self.source.open()?;
        f.seek(SeekFrom::Start(self.data))?;
        let raw = f.take(self.packed);
        let crc = Some(self.crc);
        Ok(if self.method == 8 {
            Box::new(Verify::new(
                Inflate::new(raw, self.packed),
                self.name.clone(),
                self.len,
                &self.sha,
                crc,
            ))
        } else {
            Box::new(Verify::new(raw, self.name.clone(), self.len, &self.sha, crc))
        })
    }
}
/// 置いた場所の持ち主を、読み終えるまで持つ読み手（エントリを先に手放しても、読んでいるファイルを片付けない）。
struct Kept<R: Read> {
    inner: R,
    _keep: Option<Keep>,
}
impl<R: Read> Read for Kept<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}
/// 流して展開し、使い切った圧縮のバイト数が宣言どおりかを終わりで確かめる。
struct Inflate<R: Read> {
    d: flate2::bufread::DeflateDecoder<BufReader<io::Take<R>>>,
    packed: u64,
}
impl<R: Read> Inflate<R> {
    fn new(r: io::Take<R>, packed: u64) -> Self {
        Self {
            d: flate2::bufread::DeflateDecoder::new(BufReader::with_capacity(1 << 16, r)),
            packed,
        }
    }
}
impl<R: Read> Read for Inflate<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.d.read(buf).map_err(|e| invalid(format!("Deflateが壊れています（{e}）")))?;
        if n == 0 && !buf.is_empty() && self.d.total_in() != self.packed {
            return Err(invalid("Deflateの長さが一致しません"));
        }
        Ok(n)
    }
}
fn invalid(why: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.into())
}
/// 長さ・SHA-256・CRC を数えながら読み、終わりで確かめる。宣言より長い中身は、超えたところで断る。
struct Verify<R: Read> {
    inner: R,
    name: String,
    remaining: u64,
    sha: Sha256,
    /// 確かめる SHA-256（空なら確かめない。manifest そのものなど、札の無いエントリ）。
    expected: String,
    crc: Option<(crc32fast::Hasher, u32)>,
    done: bool,
}
impl<R: Read> Verify<R> {
    fn new(inner: R, name: String, len: u64, sha: &str, crc: Option<u32>) -> Self {
        Self {
            inner,
            name,
            remaining: len,
            sha: Sha256::new(),
            expected: sha.to_owned(),
            crc: crc.map(|c| (crc32fast::Hasher::new(), c)),
            done: false,
        }
    }
}
impl<R: Read> Read for Verify<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        // 宣言の長さより 1 バイトだけ多く読めるようにして、長すぎる中身を見つける
        let want = buf.len().min(self.remaining.saturating_add(1).min(usize::MAX as u64) as usize);
        let n = self.inner.read(&mut buf[..want])?;
        if n as u64 > self.remaining {
            return Err(invalid(format!("宣言より長い中身です: {}", self.name)));
        }
        self.remaining -= n as u64;
        self.sha.update(&buf[..n]);
        if let Some((h, _)) = &mut self.crc {
            h.update(&buf[..n]);
        }
        if n == 0 {
            self.done = true;
            if self.remaining != 0 {
                return Err(invalid(format!("中身が途中で切れています: {}", self.name)));
            }
            let digest = format!("{:x}", std::mem::take(&mut self.sha).finalize());
            if !self.expected.is_empty() && digest != self.expected {
                return Err(invalid(format!("SHA-256または長さが一致しません: {}", self.name)));
            }
            if let Some((h, want)) = self.crc.take() {
                if h.finalize() != want {
                    return Err(invalid(format!("CRCまたは長さが一致しません: {}", self.name)));
                }
            }
        }
        Ok(n)
    }
}
/// 読み手から書き手へ流す。読みの `InvalidData` は壊れたファイルの種類にする。
pub(crate) fn copy(r: &mut dyn Read, w: &mut dyn Write) -> Result<()> {
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io_error(e)),
        };
        w.write_all(&buf[..n])?;
    }
}
/// io のエラーを、壊れたデータ（`InvalidData`）と入出力の失敗に言い分ける。
pub(crate) fn io_error(e: io::Error) -> Error {
    if e.kind() == io::ErrorKind::InvalidData {
        Error::InvalidData(e.to_string())
    } else {
        Error::Io(e)
    }
}
impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::Mem(b) => write!(f, "Blob(メモリ {} バイト)", b.len()),
            Repr::Zip(z) => write!(f, "Blob({} の {} バイト)", z.name, z.len),
            Repr::File(x) => write!(f, "Blob({} {} バイト)", x.name, x.len),
            Repr::Made(m) => write!(f, "Blob(作る {})", m.describe()),
        }
    }
}
/// 中身が同じか（両方メモリならバイト列、ほかは長さと SHA-256）。
impl PartialEq for Blob {
    fn eq(&self, other: &Self) -> bool {
        match (self.in_memory(), other.in_memory()) {
            (Some(a), Some(b)) => a == b,
            _ => {
                self.len() == other.len()
                    && matches!((self.sha256(), other.sha256()), (Ok(a), Ok(b)) if a == b)
            }
        }
    }
}
impl Eq for Blob {}
impl From<Arc<[u8]>> for Blob {
    fn from(b: Arc<[u8]>) -> Self {
        Self(Repr::Mem(b))
    }
}
impl From<Vec<u8>> for Blob {
    fn from(b: Vec<u8>) -> Self {
        Self(Repr::Mem(Arc::from(b)))
    }
}
impl From<&[u8]> for Blob {
    fn from(b: &[u8]) -> Self {
        Self(Repr::Mem(Arc::from(b)))
    }
}
impl<const N: usize> From<[u8; N]> for Blob {
    fn from(b: [u8; N]) -> Self {
        Self(Repr::Mem(Arc::from(&b[..])))
    }
}

/// 正本の部分の名前か（`document.utpaint.<1 から始まる番号>`。根かセットの下）。
pub(crate) fn part_number(name: &str) -> Option<u32> {
    let leaf = archive::split_set(name).map_or(name, |(_, leaf)| leaf);
    let n = leaf.strip_prefix("document.utpaint.")?;
    (!n.is_empty() && !n.starts_with('0') && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit()))
        .then(|| n.parse().ok())
        .flatten()
}
/// 正本のヘッダーか部分の名前なら、そのセットの置き場（根なら空）。
fn document_owner(name: &str) -> Option<&str> {
    let (owner, leaf) = archive::split_set(name).unwrap_or(("", name));
    (leaf == "document.utpaint" || part_number(name).is_some()).then_some(owner)
}
/// 分けない正本（部分を持たない `document.utpaint`）か。
fn is_whole_document<V>(name: &str, entries: &BTreeMap<String, V>) -> bool {
    let (prefix, leaf) = match archive::split_set(name) {
        Some((id, leaf)) => (format!("sets/{id}/"), leaf),
        None => (String::new(), name),
    };
    leaf == "document.utpaint" && !entries.contains_key(&format!("{prefix}document.utpaint.1"))
}
/// 1 エントリの上限（正本の部分は小さい）。
fn entry_limit(name: &str) -> u64 {
    if part_number(name).is_some() {
        MAX_PART_BYTES
    } else {
        MAX_ONE_ENTRY
    }
}

/// manifest と全エントリを検証した .ylp（中身はメモリかファイルの位置）。
#[derive(Clone, Debug)]
pub struct Package {
    pub(crate) files: Files,
    /// manifest の版（1〜4）。書くときは、今の上限に収まれば `min(版, 3)`、収まらなければ 4。
    pub(crate) level: u32,
    /// 読んだときの manifest（作ったものは書くときに作る）。
    manifest: Option<Arc<[u8]>>,
    /// 開くときに確かめながら読んだ、分けない正本の骨組み（エントリ名 → 骨組みか読めなかった理由）。同じ中身を 2 度流さないため。
    pub(crate) skeletons: BTreeMap<String, std::result::Result<Arc<crate::NativeDocument>, String>>,
    /// エントリではないが、圧縮したバイト列をそのまま写せる元（置き換えたセットの前の正本の部分など。.ylp の中の位置）。
    pub(crate) donors: Vec<Blob>,
}
impl Package {
    /// エントリと版から（名前と、正本があることを確かめる。manifest は書くときに作る）。
    pub(crate) fn build(files: Files, level: u32) -> Result<Self> {
        check_budget(files.len() <= MAX_ENTRIES, "エントリの数の上限を超えています")?;
        for (name, blob) in &files {
            archive::name_check(name, level.min(3), YLP_PREFIX)?;
            check(
                name != "mimetype" && name != MANIFEST,
                "予約されたエントリ名です",
            )?;
            check_budget(blob.len() <= entry_limit(name), ONE_ENTRY_OVER)?;
        }
        archive::complete_ylp(files.keys(), level.min(3))?;
        Ok(Self {
            files,
            level,
            manifest: None,
            skeletons: BTreeMap::new(),
            donors: Vec::new(),
        })
    }
    pub fn entries(&self) -> &Files {
        &self.files
    }
    /// manifest の版（1〜4）。
    pub fn manifest_version(&self) -> u32 {
        self.level
    }
    /// 読んだときの manifest（作ったプロジェクトは None。書くときに作る）。
    pub fn read_manifest(&self) -> Option<&[u8]> {
        self.manifest.as_deref()
    }
    /// メモリの中のバイト列から読む。エントリはメモリに持つ。
    pub fn read_bytes(bytes: &[u8], limits: &Limits) -> Result<Self> {
        let source = Source::Bytes(Arc::from(bytes));
        let mut p = read(&source, limits, u64::MAX)?;
        // メモリから読んだものは、全部をメモリに持つ（試験・小さなファイル）
        for blob in p.files.values_mut() {
            if blob.in_memory().is_none() {
                *blob = Blob::from(blob.bytes()?);
            }
        }
        Ok(p)
    }
    /// ファイルから流して読む（全エントリを確かめる。小さなものだけメモリに残す）。
    pub fn open(path: &Path, limits: &Limits) -> Result<Self> {
        Self::open_keeping(path, limits, Thresholds::current().keep_in_memory)
    }
    /// `open` の、メモリに残すエントリの大きさ（[`Thresholds::keep_in_memory`]）を渡す形。別のスレッドから呼ぶ側が、そのスレッドで取った
    /// 閾値を渡す（閾値はスレッドごとに変えられるので、別のスレッドの `open` は呼んだスレッドの閾値を見ない）。
    pub(crate) fn open_keeping(path: &Path, limits: &Limits, keep: u64) -> Result<Self> {
        read(&Source::open_file(path)?, limits, keep)
    }
    /// 保存で置き換えた後の名前へ、エントリの置き場の名前を付け替える（[`Blob::note_path`]）。
    pub(crate) fn note_path(&self, path: &Path) {
        for blob in self.files.values() {
            blob.note_path(path);
        }
    }

    /// 書く形を決める（1 回目: 全エントリの長さと SHA-256 を数え、manifest を作る）。
    ///
    /// エントリごとの数え（作るエントリは作りながら数える）は互いに独立なので、エントリごとに並べて数える。結果は 1 つずつ数えるのと同じで、
    /// 断る理由も同じ（名前の順に見て、はじめに断ったエントリの理由。それより後ろのエントリは、断ったあとに数えない）。同じ正本から作る
    /// エントリ（版 26 のヘッダーと部分）は、正本の側が 1 回の流しでまとめて数える（`Made::sha256`）ので、その分は並ばない。
    pub(crate) fn plan(&self) -> Result<Plan> {
        let t = Thresholds::current();
        let entries: Vec<(&String, &Blob)> = self.files.iter().collect();
        let counted = first_failure_in_order(&entries, |(name, blob)| {
            let len = blob.len();
            check_budget(len <= entry_limit(name), ONE_ENTRY_OVER)?;
            Ok((blob.sha256()?, len))
        })?;
        let mut lines = Vec::with_capacity(self.files.len());
        let mut total = 0u64;
        let mut fits = self.files.len() <= t.classic_entries;
        for ((name, _), (sha, len)) in entries.iter().zip(counted) {
            total = total.saturating_add(len);
            fits &= len <= t.classic_entry_bytes;
            lines.push((sha, len, (*name).clone()));
        }
        fits &= total <= t.classic_total_bytes;
        let level = if fits { self.level.min(3) } else { 4 };
        let mut text = format!("{YLP_PREFIX}{level}\n");
        for (sha, len, name) in &lines {
            text.push_str(&format!("{sha} {len} {name}\n"));
        }
        Ok(Plan {
            level,
            manifest: Arc::from(text.into_bytes()),
            thresholds: t,
        })
    }
    /// 2 回目: `out` へ書く。今の上限に収まる（`YLP-3` 以前）ときは今の `Archive::to_bytes` と同じバイト列。
    pub(crate) fn write_with(&self, plan: &Plan, out: &mut dyn Output) -> Result<()> {
        let donors = if plan.level >= 4 {
            self.donor_map()
        } else {
            Default::default()
        };
        let mut w = ZipWriter {
            out,
            central: Vec::new(),
            count: 0,
            zip64: plan.level >= 4,
            options: &plan.thresholds,
            donors: &donors,
        };
        w.entry("mimetype", &Blob::from(YLP_MIME.as_bytes()))?;
        w.entry(MANIFEST, &Blob::from(plan.manifest.clone()))?;
        for (name, blob) in &self.files {
            w.entry(name, blob)?;
        }
        w.finish()
    }
    /// 圧縮したバイト列を写せるエントリ（.ylp の中の位置で持つエントリと、置き換えて外したエントリ）。SHA-256 で引く。
    fn donor_map(&self) -> std::collections::HashMap<String, Arc<ZipRef>> {
        self.files
            .values()
            .chain(&self.donors)
            .filter_map(|b| match &b.0 {
                Repr::Zip(z) => Some((z.sha.clone(), z.clone())),
                _ => None,
            })
            .filter(|(sha, _)| !sha.is_empty())
            .collect()
    }
    /// 置き換えて外すエントリを、圧縮したバイト列を写せる元として覚える（.ylp の中の位置で持つものだけ。同じ中身は 1 つ）。
    pub(crate) fn remember_donors<'a>(&mut self, removed: impl Iterator<Item = &'a Blob>) {
        for b in removed {
            if matches!(b.0, Repr::Zip(_)) && !self.donors.iter().any(|d| d.same(b)) {
                self.donors.push(b.clone());
            }
        }
    }
    /// メモリの中へ書く（試験・小さなプロジェクト）。
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let plan = self.plan()?;
        let mut out = Cursor::new(Vec::new());
        self.write_with(&plan, &mut out)?;
        Ok(out.into_inner())
    }
}
/// `items` の各要素に `work` を、並べて（rayon）かける。どれかが断れば、並びの順にいちばん前の断りを返す（1 つずつかけたときと同じ理由）。
/// 断ったより後ろの要素は、まだ始めていなければ始めない（断った前のものは、断りの理由の候補なので最後までかける）。
/// 並べる仕事は別のスレッドで動くので、スレッドごとの閾値（[`Thresholds::scoped`]）は見えない。呼ぶ側が先に値を取っておく。
pub(crate) fn first_failure_in_order<T: Sync, R: Send>(
    items: &[T],
    work: impl Fn(&T) -> Result<R> + Sync,
) -> Result<Vec<R>> {
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let failed = AtomicUsize::new(usize::MAX);
    let results: Vec<Option<Result<R>>> = items
        .par_iter()
        .enumerate()
        .map(|(index, item)| {
            if index > failed.load(Ordering::Relaxed) {
                return None;
            }
            let done = work(item);
            if done.is_err() {
                failed.fetch_min(index, Ordering::Relaxed);
            }
            Some(done)
        })
        .collect();
    let mut out = Vec::with_capacity(items.len());
    for done in results {
        // 後ろのものを飛ばしたのは、それより前に断りがあったときだけ（先にその断りに行き着く）
        out.push(done.expect("断りより前は飛ばさない")?);
    }
    Ok(out)
}

/// 書く形（1 回目の結果）。
pub(crate) struct Plan {
    pub level: u32,
    pub manifest: Arc<[u8]>,
    pub thresholds: Thresholds,
}

/// 書く形を決める閾値。既定は本物の値で、試験が小さくして（[`Thresholds::scoped`]）、小さな文書で大きな形の道（`YLP-4`・zip64・
/// 正本の分割）を全部通す口。読み手の上限（[`Limits`]）は変えない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thresholds {
    /// `YLP-3` で書ける 1 エントリ・合計・エントリの数（これに収まるときだけ今の形で書く）。
    pub classic_entry_bytes: u64,
    pub classic_total_bytes: u64,
    pub classic_entries: usize,
    /// この位置以上（4 GiB 未満でも）のローカルヘッダーは、中央ディレクトリから zip64 の拡張で指す。`YLP-4` のときだけ効く。
    pub zip64_offset_at: u64,
    /// エントリ（mimetype・manifest を含む）がこの数を超えたら zip64 の終端を書く。`YLP-4` のときだけ効く。
    pub zip64_count_above: u64,
    /// 正本（中の版の並び）がこれを超えたら、版 26 で部分に分ける。
    pub split_above: u64,
    /// 1 つの部分の上限（タイル 1 枚の値は分けない）。
    pub part_bytes: u64,
    /// 小さな層をまとめる大きさ: 今の部分も次の層もこれに満たなければ、層の始まりで区切らずに続ける。
    pub part_min: u64,
    /// ファイルを開くとき、メモリに残すエントリの大きさ（これより大きなものはファイルの位置で持つ）。
    pub keep_in_memory: u64,
}
impl Thresholds {
    pub const REAL: Self = Self {
        classic_entry_bytes: CLASSIC_ENTRY_BYTES,
        classic_total_bytes: CLASSIC_TOTAL_BYTES,
        classic_entries: CLASSIC_ENTRIES,
        zip64_offset_at: 0xFFFF_FFFF,
        zip64_count_above: 0xFFFF,
        split_above: CLASSIC_ENTRY_BYTES,
        part_bytes: MAX_PART_BYTES,
        part_min: 16 << 20,
        keep_in_memory: KEEP_IN_MEMORY,
    };
    /// このスレッドの今の閾値（試験が `scoped` で変えていなければ本物の値）。
    pub fn current() -> Self {
        SCOPED.with(|c| c.get()).unwrap_or(Self::REAL)
    }
    /// `f` の間だけ、このスレッドの閾値を変える（試験の口。上限を本物より大きくはできない）。
    pub fn scoped<T>(self, f: impl FnOnce() -> T) -> T {
        let t = Self {
            classic_entry_bytes: self.classic_entry_bytes.min(CLASSIC_ENTRY_BYTES),
            classic_total_bytes: self.classic_total_bytes.min(CLASSIC_TOTAL_BYTES),
            classic_entries: self.classic_entries.min(CLASSIC_ENTRIES),
            zip64_offset_at: self.zip64_offset_at.min(0xFFFF_FFFF),
            zip64_count_above: self.zip64_count_above.min(0xFFFF),
            split_above: self.split_above.min(CLASSIC_ENTRY_BYTES),
            part_bytes: self.part_bytes.clamp(1, MAX_PART_BYTES),
            part_min: self.part_min,
            keep_in_memory: self.keep_in_memory.min(KEEP_IN_MEMORY),
        };
        struct Restore(Option<Thresholds>);
        impl Drop for Restore {
            fn drop(&mut self) {
                SCOPED.with(|c| c.set(self.0));
            }
        }
        let _restore = Restore(SCOPED.with(|c| c.replace(Some(t))));
        f()
    }
}
impl Default for Thresholds {
    fn default() -> Self {
        Self::REAL
    }
}
thread_local! {
    static SCOPED: std::cell::Cell<Option<Thresholds>> = const { std::cell::Cell::new(None) };
}

/// 書く先（後から戻ってローカルヘッダーを直し、最後に余りを切る）。
pub(crate) trait Output: Write + Seek {
    fn truncate_at(&mut self, len: u64) -> io::Result<()>;
}
impl Output for Cursor<Vec<u8>> {
    fn truncate_at(&mut self, len: u64) -> io::Result<()> {
        self.get_mut().truncate(len as usize);
        Ok(())
    }
}
impl Output for io::BufWriter<File> {
    fn truncate_at(&mut self, len: u64) -> io::Result<()> {
        self.flush()?;
        self.get_ref().set_len(len)
    }
}

struct ZipWriter<'a> {
    out: &'a mut dyn Output,
    central: Vec<u8>,
    count: u64,
    zip64: bool,
    options: &'a Thresholds,
    /// 圧縮したバイト列をそのまま写せるエントリ（SHA-256 → .ylp の中の位置）。`YLP-4` のときだけ使う。
    donors: &'a std::collections::HashMap<String, Arc<ZipRef>>,
}
/// 1 つのエントリを、1 MiB ずつのかたまりに分けて別々のスレッドで圧縮し、順に並べて 1 つの Deflate の流れにする（pigz と同じ形: かたまりは
/// 前のかたまりの終わりの 32 KiB を辞書にして圧縮し、終わりでないかたまりは Sync の flush でバイトの境目に揃える。最後のかたまりだけが
/// 最後のブロックの印を持つ）。持つのは、動いているかたまりの数（スレッドの数の 2 倍）ぶんの入力と出力だけ。どの読み手にも普通の Deflate の
/// 流れとして読める。出力はスレッドの数に依らず同じ（かたまりの大きさと水準で決まる）。今の形（`YLP-3`）には使わない（今の書き手と
/// バイトまで同じにするため）。
struct ParallelDeflate;
/// 圧縮するかたまり（番号・中身・辞書にする前のかたまり・最後か）。
type Job = (usize, Arc<Vec<u8>>, Option<Arc<Vec<u8>>>, bool);
impl ParallelDeflate {
    const BLOCK: usize = 1 << 20;
    const DICT: usize = 32 << 10;
    /// 書いた（CRC、元の長さ、圧縮した長さ）。
    fn run(blob: &Blob, out: &mut dyn Write) -> Result<(u32, u64, u64)> {
        use std::sync::mpsc::{channel, sync_channel};
        let threads = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .clamp(1, 8);
        std::thread::scope(|scope| -> Result<(u32, u64, u64)> {
            let (job_tx, job_rx) =
                sync_channel::<Job>(threads * 2);
            let job_rx = Arc::new(std::sync::Mutex::new(job_rx));
            let (done_tx, done_rx) = channel::<(usize, io::Result<Vec<u8>>)>();
            for _ in 0..threads {
                let (jobs, done) = (job_rx.clone(), done_tx.clone());
                scope.spawn(move || loop {
                    let job = jobs.lock().map(|j| j.recv());
                    let Ok(Ok((index, block, dict, last))) = job else {
                        return;
                    };
                    let tail = dict.as_deref().map(|d| &d[d.len().saturating_sub(Self::DICT)..]);
                    if done.send((index, Self::block(&block, tail, last))).is_err() {
                        return;
                    }
                });
            }
            drop(done_tx);
            let mut feed = Feed {
                jobs: Some(job_tx),
                done: done_rx,
                out,
                held: None,
                current: Vec::with_capacity(Self::BLOCK),
                previous: None,
                sent: 0,
                written: 0,
                waiting: BTreeMap::new(),
                packed: 0,
                crc: crc32fast::Hasher::new(),
                len: 0,
            };
            let wrote = blob.write_to(&mut feed);
            let finished = wrote.and_then(|()| feed.finish());
            // 失敗しても、スレッドが待たずに終わるように仕事の口を閉じる
            feed.jobs = None;
            finished?;
            Ok((feed.crc.clone().finalize(), feed.len, feed.packed))
        })
    }
    /// 1 つのかたまりを圧縮する（生の Deflate。最後でなければ Sync の flush でバイトの境目に揃える）。
    fn block(input: &[u8], dict: Option<&[u8]>, last: bool) -> io::Result<Vec<u8>> {
        use flate2::{Compress, Compression, FlushCompress, Status};
        let mut c = Compress::new(Compression::default(), false);
        if let Some(d) = dict {
            c.set_dictionary(d).map_err(io::Error::other)?;
        }
        let mut out = Vec::with_capacity(input.len() / 2 + 1024);
        let flush = if last {
            FlushCompress::Finish
        } else {
            FlushCompress::Sync
        };
        loop {
            if out.capacity() - out.len() < 64 << 10 {
                out.reserve(out.capacity().max(64 << 10));
            }
            let at = c.total_in() as usize;
            let status = c.compress_vec(&input[at..], &mut out, flush).map_err(io::Error::other)?;
            let consumed = c.total_in() as usize == input.len();
            let room = out.capacity() > out.len();
            match status {
                Status::StreamEnd => return Ok(out),
                _ if !last && consumed && room => return Ok(out),
                _ => {}
            }
        }
    }
}
/// `ParallelDeflate` へかたまりを渡し、できた順に並べて書く書き先。
struct Feed<'o> {
    jobs: Option<std::sync::mpsc::SyncSender<Job>>,
    done: std::sync::mpsc::Receiver<(usize, io::Result<Vec<u8>>)>,
    out: &'o mut dyn Write,
    /// 渡すのを待っているかたまり（次のかたまりが来るか、終わりまで、最後かが分からない）。
    held: Option<Arc<Vec<u8>>>,
    current: Vec<u8>,
    previous: Option<Arc<Vec<u8>>>,
    sent: usize,
    written: usize,
    waiting: BTreeMap<usize, Vec<u8>>,
    packed: u64,
    crc: crc32fast::Hasher,
    len: u64,
}
impl Feed<'_> {
    fn send(&mut self, block: Arc<Vec<u8>>, last: bool) -> io::Result<()> {
        let dict = self.previous.replace(block.clone());
        let jobs = self.jobs.as_ref().ok_or_else(|| io::Error::other("圧縮が止まりました"))?;
        jobs.send((self.sent, block, dict, last))
            .map_err(|_| io::Error::other("圧縮のスレッドが止まりました"))?;
        self.sent += 1;
        self.drain(false)
    }
    /// できたかたまりを順に書く（`all` なら、渡した全部ができるまで待つ）。
    fn drain(&mut self, all: bool) -> io::Result<()> {
        loop {
            while let Some(bytes) = self.waiting.remove(&self.written) {
                self.out.write_all(&bytes)?;
                self.packed += bytes.len() as u64;
                self.written += 1;
            }
            if self.written == self.sent {
                return Ok(());
            }
            let next = if all {
                self.done.recv().map_err(|_| io::Error::other("圧縮のスレッドが止まりました"))?
            } else {
                match self.done.try_recv() {
                    Ok(x) => x,
                    Err(_) => return Ok(()),
                }
            };
            self.waiting.insert(next.0, next.1?);
        }
    }
    fn push_block(&mut self) -> io::Result<()> {
        let block = Arc::new(std::mem::replace(&mut self.current, Vec::with_capacity(ParallelDeflate::BLOCK)));
        if let Some(previous) = self.held.replace(block) {
            self.send(previous, false)?;
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        if !self.current.is_empty() || self.held.is_none() {
            self.push_block()?;
        }
        if let Some(last) = self.held.take() {
            self.send(last, true)?;
        }
        self.drain(true)?;
        Ok(())
    }
}
impl Write for Feed<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let room = ParallelDeflate::BLOCK - self.current.len();
        let n = buf.len().min(room);
        self.current.extend_from_slice(&buf[..n]);
        self.crc.update(&buf[..n]);
        self.len += n as u64;
        if self.current.len() == ParallelDeflate::BLOCK {
            self.push_block()?;
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
/// はじめの 1 MiB を試しに圧縮して、縮むか（97% 未満になるか）。
fn compresses(blob: &Blob) -> Result<bool> {
    const SAMPLE: usize = 1 << 20;
    struct Prefix(Vec<u8>);
    impl Write for Prefix {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let room = SAMPLE - self.0.len();
            if room == 0 {
                return Err(io::Error::other("試しの分を読み終えた"));
            }
            let n = buf.len().min(room);
            self.0.extend_from_slice(&buf[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut prefix = Prefix(Vec::with_capacity(SAMPLE));
    match blob.write_to(&mut prefix) {
        Ok(()) => {}
        // 試しの分で止めた（ほかの失敗は、このあと全部を書くときに出る）
        Err(_) if prefix.0.len() == SAMPLE => {}
        Err(e) => return Err(e),
    }
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&prefix.0)?;
    let packed = encoder.finish()?;
    Ok((packed.len() as u64) * 100 < (prefix.0.len() as u64) * 97)
}
/// 書いたバイト数と CRC を数えながら書く。
struct Counting<'a> {
    out: &'a mut dyn Write,
    crc: crc32fast::Hasher,
    len: u64,
}
impl Write for Counting<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.out.write(buf)?;
        self.crc.update(&buf[..n]);
        self.len += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}
/// 書いたバイト数だけを数える。
struct Tally<'a> {
    out: &'a mut dyn Write,
    len: u64,
}
impl Write for Tally<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.out.write(buf)?;
        self.len += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}
impl ZipWriter<'_> {
    fn entry(&mut self, name: &str, blob: &Blob) -> Result<()> {
        let offset = self.out.stream_position()?;
        let try_deflate = name != "mimetype" && !name.ends_with(".png");
        // `YLP-4` は、同じ中身の圧縮したバイト列がファイルにあれば、そのまま写す（変わらないセット・変わらない層の部分を圧縮し直さない。
        // 書いた後の読み直しで長さ・CRC・SHA-256 を確かめる）。今の形（`YLP-3`）は今の書き手とバイトまで同じにするため、いつも圧縮し直す
        if self.zip64 {
            let raw = match &blob.0 {
                Repr::Zip(z) => Some(z.clone()),
                _ => blob
                    .sha256()
                    .ok()
                    .and_then(|sha| self.donors.get(&sha))
                    .filter(|z| z.len == blob.len())
                    .cloned(),
            };
            if let Some(z) = raw {
                return self.raw_entry(name, offset, &z);
            }
        }
        // ローカルヘッダー（方式・CRC・長さは後で直す）
        let mut head = Vec::with_capacity(30 + name.len());
        put32(&mut head, 0x04034b50);
        for n in [20, 0, 0, 0, 33] {
            put16(&mut head, n)
        }
        for _ in 0..3 {
            put32(&mut head, 0)
        }
        put16(&mut head, name.len() as u16);
        put16(&mut head, 0);
        head.extend(name.as_bytes());
        self.out.write_all(&head)?;
        let data = offset + head.len() as u64;
        let len = blob.len();
        let (mut method, mut crc, mut packed) = (0u16, 0u32, 0u64);
        // `YLP-4` の大きなエントリは、はじめの 1 MiB を試しに圧縮して、縮まなければ圧縮しない（縮まない画素を全部圧縮してから捨てない）
        let worth = !self.zip64 || len < 4 << 20 || compresses(blob)?;
        if try_deflate && worth && self.zip64 && len >= 4 << 20 {
            // `YLP-4` の大きなエントリは、1 MiB ずつに分けて並べて圧縮する（1 つの Deflate の流れのまま。下の `ParallelDeflate`）
            let (c, n, p) = ParallelDeflate::run(blob, &mut *self.out)?;
            check(n == len, format!("書いた長さが一致しません: {name}"))?;
            if p > 0 && p < len {
                (method, crc, packed) = (8, c, p);
            }
        } else if try_deflate && worth {
            let mut tally = Tally {
                out: &mut *self.out,
                len: 0,
            };
            let mut encoder =
                flate2::write::DeflateEncoder::new(&mut tally, flate2::Compression::default());
            let mut counting = Counting {
                out: &mut encoder,
                crc: crc32fast::Hasher::new(),
                len: 0,
            };
            blob.write_to(&mut counting)?;
            let (c, n) = (counting.crc.finalize(), counting.len);
            encoder.finish()?;
            check(n == len, format!("書いた長さが一致しません: {name}"))?;
            if tally.len > 0 && tally.len < len {
                (method, crc, packed) = (8, c, tally.len);
            }
        }
        if method == 0 {
            // 縮まない（今の書き手と同じく、圧縮しないで入れる）。圧縮した分は上書きし、余りは最後に切る
            self.out.seek(SeekFrom::Start(data))?;
            let mut counting = Counting {
                out: &mut *self.out,
                crc: crc32fast::Hasher::new(),
                len: 0,
            };
            blob.write_to(&mut counting)?;
            check(counting.len == len, format!("書いた長さが一致しません: {name}"))?;
            crc = counting.crc.finalize();
            packed = len;
        }
        let end = data + packed;
        check_budget(
            len <= 0xFFFF_FFFE && packed <= 0xFFFF_FFFE,
            "1エントリが4 GiBを超えています",
        )?;
        self.out.seek(SeekFrom::Start(offset + 8))?;
        let mut patch = Vec::with_capacity(14);
        put16(&mut patch, method);
        put16(&mut patch, 0);
        put16(&mut patch, 33);
        put32(&mut patch, crc);
        put32(&mut patch, packed as u32);
        put32(&mut patch, len as u32);
        self.out.write_all(&patch)?;
        self.out.seek(SeekFrom::Start(end))?;
        self.central_entry(name, offset, method, crc, packed, len)
    }
    /// 中央ディレクトリの 1 エントリ（位置が閾値以上なら zip64 の拡張で指す）。
    fn central_entry(
        &mut self,
        name: &str,
        offset: u64,
        method: u16,
        crc: u32,
        packed: u64,
        len: u64,
    ) -> Result<()> {
        let far = self.zip64 && offset >= self.options.zip64_offset_at;
        let c = &mut self.central;
        put32(c, 0x02014b50);
        for n in [if far { 45 } else { 20 }, if far { 45 } else { 20 }, 0, method, 0, 33] {
            put16(c, n)
        }
        for n in [crc, packed as u32, len as u32] {
            put32(c, n)
        }
        for n in [name.len() as u16, if far { 12 } else { 0 }, 0, 0, 0] {
            put16(c, n)
        }
        put32(c, 0);
        put32(c, if far { 0xFFFF_FFFF } else { offset as u32 });
        check(
            far || offset <= 0xFFFF_FFFE,
            "zip64なしでは4 GiBを超える位置を指せません",
        )?;
        c.extend(name.as_bytes());
        if far {
            put16(c, 1);
            put16(c, 8);
            c.extend(offset.to_le_bytes());
        }
        self.count += 1;
        Ok(())
    }
    /// 圧縮したバイト列をそのまま写す（方式・CRC・長さは元のエントリのまま）。
    fn raw_entry(&mut self, name: &str, offset: u64, z: &ZipRef) -> Result<()> {
        let mut head = Vec::with_capacity(30 + name.len());
        put32(&mut head, 0x04034b50);
        for n in [20, 0, z.method, 0, 33] {
            put16(&mut head, n)
        }
        put32(&mut head, z.crc);
        put32(&mut head, z.packed as u32);
        put32(&mut head, z.len as u32);
        put16(&mut head, name.len() as u16);
        put16(&mut head, 0);
        head.extend(name.as_bytes());
        self.out.write_all(&head)?;
        let mut f = z.source.open()?;
        f.seek(SeekFrom::Start(z.data))?;
        let mut packed = f.take(z.packed);
        let n = io::copy(&mut packed, &mut *self.out)?;
        check(n == z.packed, format!("写す元が途中で切れています: {name}"))?;
        self.central_entry(name, offset, z.method, z.crc, z.packed, z.len)
    }
    fn finish(self) -> Result<()> {
        let start = self.out.stream_position()?;
        self.out.write_all(&self.central)?;
        let size = self.central.len() as u64;
        let end_of_central = start + size;
        let big = self.zip64
            && (self.count > self.options.zip64_count_above
                || start >= self.options.zip64_offset_at
                || size >= self.options.zip64_offset_at);
        check(
            big || (self.count <= 0xFFFF && end_of_central <= 0xFFFF_FFFE),
            "zip64なしでは書けない大きさです",
        )?;
        let mut tail = Vec::new();
        if big {
            // zip64 の終端の記録と、その位置を指す目印
            put32(&mut tail, 0x06064b50);
            tail.extend(44u64.to_le_bytes());
            put16(&mut tail, 45);
            put16(&mut tail, 45);
            put32(&mut tail, 0);
            put32(&mut tail, 0);
            tail.extend(self.count.to_le_bytes());
            tail.extend(self.count.to_le_bytes());
            tail.extend(size.to_le_bytes());
            tail.extend(start.to_le_bytes());
            put32(&mut tail, 0x07064b50);
            put32(&mut tail, 0);
            tail.extend(end_of_central.to_le_bytes());
            put32(&mut tail, 1);
        }
        put32(&mut tail, 0x06054b50);
        let count16 = if big { 0xFFFF } else { self.count as u16 };
        for n in [0, 0, count16, count16] {
            put16(&mut tail, n)
        }
        put32(&mut tail, if big { 0xFFFF_FFFF } else { size as u32 });
        put32(&mut tail, if big { 0xFFFF_FFFF } else { start as u32 });
        put16(&mut tail, 0);
        self.out.write_all(&tail)?;
        let length = self.out.stream_position()?;
        self.out.truncate_at(length)?;
        self.out.flush()?;
        Ok(())
    }
}

// ───────── 読む ─────────

/// 中央ディレクトリの 1 エントリ（ローカルヘッダーまで確かめたもの）。
struct Listed {
    name: String,
    method: u16,
    crc: u32,
    packed: u64,
    len: u64,
    data: u64,
}

fn read(source: &Source, limits: &Limits, keep: u64) -> Result<Package> {
    let mut f = source.open()?;
    let file_len = f.seek(SeekFrom::End(0))?;
    let mime = YLP_MIME;
    // 先頭は無圧縮の mimetype
    let mut head = vec![0u8; 38 + mime.len()];
    f.seek(SeekFrom::Start(0))?;
    let ok = file_len >= head.len() as u64 && f.read_exact(&mut head).is_ok();
    check(
        ok && head[0..4] == *b"PK\x03\x04"
            && le16(&head, 8) == 0
            && le32(&head, 18) as usize == mime.len()
            && le32(&head, 22) as usize == mime.len()
            && le16(&head, 26) == 8
            && le16(&head, 28) == 0
            && &head[30..38] == b"mimetype"
            && &head[38..] == mime.as_bytes(),
        "先頭に無圧縮のmimetypeがありません",
    )?;
    // 終端（後ろ 65557 バイトの中から探す）
    let tail_len = file_len.min(65557);
    let mut tail = vec![0u8; tail_len as usize];
    f.seek(SeekFrom::Start(file_len - tail_len))?;
    f.read_exact(&mut tail)?;
    let at = (tail.len().saturating_sub(65557)..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| {
            tail.get(i..i + 4) == Some(b"PK\x05\x06")
                && i + 22 <= tail.len()
                && i + 22 + le16(&tail, i + 20) as usize == tail.len()
        })
        .ok_or_else(|| Error::InvalidData("ZIP終端がありません".into()))?;
    let eocd = file_len - tail_len + at as u64;
    let e = &tail[at..];
    check(
        le16(e, 4) == 0 && le16(e, 6) == 0,
        "分割ZIPは未対応です",
    )?;
    let (count16, disk_count16, size32, start32) = (le16(e, 10), le16(e, 8), le32(e, 12), le32(e, 16));
    let wide = count16 == 0xFFFF || size32 == 0xFFFF_FFFF || start32 == 0xFFFF_FFFF;
    let (count, central_size, central_start, central_end) = if wide {
        // zip64 の終端: 目印（終端の直前 20 バイト）と記録（目印の直前 56 バイト）
        check(eocd >= 20 + 56, "zip64の終端がありません")?;
        let mut loc = [0u8; 20];
        f.seek(SeekFrom::Start(eocd - 20))?;
        f.read_exact(&mut loc)?;
        let record_at = le64(&loc, 8);
        check(
            le32(&loc, 0) == 0x07064b50 && le32(&loc, 4) == 0 && le32(&loc, 16) == 1
                && record_at.checked_add(56) == Some(eocd - 20),
            "zip64の終端の目印が不正です",
        )?;
        let mut rec = [0u8; 56];
        f.seek(SeekFrom::Start(record_at))?;
        f.read_exact(&mut rec)?;
        let n = le64(&rec, 24);
        let (size, start) = (le64(&rec, 40), le64(&rec, 48));
        check(
            le32(&rec, 0) == 0x06064b50
                && le64(&rec, 4) == 44
                && le16(&rec, 14) <= 45
                && le32(&rec, 16) == 0
                && le32(&rec, 20) == 0
                && le64(&rec, 32) == n
                && (count16 == 0xFFFF || u64::from(count16) == n)
                && u64::from(disk_count16) == u64::from(count16)
                && (size32 == 0xFFFF_FFFF || u64::from(size32) == size)
                && (start32 == 0xFFFF_FFFF || u64::from(start32) == start),
            "zip64の終端の記録が不正です",
        )?;
        (n, size, start, record_at)
    } else {
        check(disk_count16 == count16, "ZIPエントリ数が不正です")?;
        (u64::from(count16), u64::from(size32), u64::from(start32), eocd)
    };
    check(
        (3..=MAX_ENTRIES as u64 + 2).contains(&count),
        "ZIPエントリ数が不正です",
    )?;
    check(
        central_start.checked_add(central_size) == Some(central_end),
        "中央ディレクトリが不正です",
    )?;
    check_budget(central_size <= MAX_CENTRAL, "中央ディレクトリの予算超過です")?;
    let mut cd = vec![0u8; central_size as usize];
    f.seek(SeekFrom::Start(central_start))?;
    f.read_exact(&mut cd)?;
    let mut listed: Vec<Listed> = Vec::with_capacity(count as usize);
    let mut at = 0usize;
    let mut previous_end = 0u64;
    let mut any_zip64 = wide;
    let mut local = vec![0u8; 30];
    for index in 0..count {
        let c = slice(&cd, at, 46)?;
        check(le32(c, 0) == 0x02014b50, "中央ディレクトリが壊れています")?;
        let needed = le16(c, 6);
        let flags = le16(c, 8);
        let method = le16(c, 10);
        let crc = le32(c, 16);
        let (packed32, len32) = (le32(c, 20), le32(c, 24));
        let nl = le16(c, 28) as usize;
        let xl = le16(c, 30) as usize;
        let cl = le16(c, 32) as usize;
        let disk = le16(c, 34);
        let offset32 = le32(c, 42);
        let allowed = if method == 8 { 0x080e } else { 0x0808 };
        check(
            disk == 0 && flags & !allowed == 0 && (method == 0 || method == 8),
            "暗号化・未知の圧縮方式・分割ZIPは未対応です",
        )?;
        let name = std::str::from_utf8(slice(&cd, at + 46, nl)?)
            .map_err(|_| Error::InvalidData("ZIP名がUTF-8ではありません".into()))?
            .to_string();
        let (packed, len, offset, z) = wide_values(
            slice(&cd, at + 46 + nl, xl)?,
            [len32, packed32, offset32],
        )?;
        check(
            needed <= 20 || (needed <= 45 && (z || wide)),
            "ZIP64または新しいZIP機能は未対応です",
        )?;
        any_zip64 |= z;
        check(
            (index != 0 || offset == 0) && offset >= previous_end && offset < central_start,
            "ZIPエントリの重なりまたは並びが不正です",
        )?;
        f.seek(SeekFrom::Start(offset))?;
        f.read_exact(&mut local)
            .map_err(|_| Error::InvalidData("ZIPが途中で切れています".into()))?;
        let lnl = le16(&local, 26) as usize;
        let lxl = le16(&local, 28) as usize;
        let mut rest = vec![0u8; lnl + lxl];
        f.read_exact(&mut rest)
            .map_err(|_| Error::InvalidData("ZIPが途中で切れています".into()))?;
        check(
            le32(&local, 0) == 0x04034b50
                && (le16(&local, 4) <= 20 || (le16(&local, 4) <= 45 && (z || wide)))
                && le16(&local, 6) == flags
                && le16(&local, 8) == method
                && lnl == nl
                && &rest[..nl] == name.as_bytes(),
            "ZIPのローカルヘッダーが一致しません",
        )?;
        let (lp, ll, _, lz) = wide_values(&rest[nl..], [le32(&local, 22), le32(&local, 18), 0])?;
        any_zip64 |= lz;
        if flags & 8 == 0 {
            check(
                le32(&local, 14) == crc && lp == packed && ll == len,
                "ZIPの長さ・CRC宣言が一致しません",
            )?;
        }
        let start = offset + 30 + nl as u64 + lxl as u64;
        let finish = start
            .checked_add(packed)
            .ok_or_else(|| Error::Budget("長さが過大です".into()))?;
        check(
            finish <= central_start,
            "ZIPエントリが中央ディレクトリに重なっています",
        )?;
        previous_end = finish;
        if flags & 8 != 0 {
            let mut d = [0u8; 24];
            f.seek(SeekFrom::Start(finish))?;
            let n = (central_start - finish).min(24) as usize;
            f.read_exact(&mut d[..n])?;
            let signed = n >= 4 && le32(&d, 0) == 0x08074b50;
            let base = if signed { 4 } else { 0 };
            let wide_sizes = z || lz;
            let size_len = if wide_sizes { 8 } else { 4 };
            let need = base + 4 + 2 * size_len;
            check(
                need <= n
                    && le32(&d, base) == crc
                    && read_n(&d, base + 4, size_len) == packed
                    && read_n(&d, base + 4 + size_len, size_len) == len,
                "ZIPのデータ記述子が一致しません",
            )?;
            previous_end = finish + need as u64;
        }
        let limit = if name == MANIFEST {
            MAX_MANIFEST
        } else if name == "mimetype" {
            256
        } else {
            entry_limit(&name)
        };
        check_budget(len <= limit, "展開の予算超過です")?;
        if method == 0 {
            check(packed == len, "無圧縮ZIPの長さが不正です")?;
        }
        check(
            index != 0 || name == "mimetype",
            "先頭エントリがmimetypeではありません",
        )?;
        listed.push(Listed {
            name,
            method,
            crc,
            packed,
            len,
            data: start,
        });
        at += 46 + nl + xl + cl;
    }
    check(at == cd.len(), "中央ディレクトリの長さが一致しません")?;
    drop(cd);
    let mut by_name: BTreeMap<&str, &Listed> = BTreeMap::new();
    for l in &listed {
        check(
            by_name.insert(l.name.as_str(), l).is_none(),
            "ZIPに重複エントリがあります",
        )?;
    }
    let small = |l: &Listed| -> Result<Vec<u8>> {
        let blob = zip_blob(source, l, None);
        let mut out = Vec::new();
        let mut r = blob.reader()?;
        copy(&mut r, &mut out)?;
        Ok(out)
    };
    check(
        by_name
            .get("mimetype")
            .map(|l| small(l))
            .transpose()?
            .is_some_and(|v| v == mime.as_bytes()),
        "MIMEが一致しません",
    )?;
    let manifest_entry = by_name
        .get(MANIFEST)
        .ok_or_else(|| Error::InvalidData("manifestがありません".into()))?;
    let manifest = small(manifest_entry)?;
    let text = std::str::from_utf8(&manifest)
        .map_err(|_| Error::InvalidData("manifestがUTF-8ではありません".into()))?;
    let mut lines = text.split('\n');
    let head_line = lines.next().unwrap_or("");
    let level = head_line
        .strip_prefix(YLP_PREFIX)
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(|| Error::InvalidData("未知のmanifestです".into()))?;
    check(
        (1..=4).contains(&level) && head_line == format!("{YLP_PREFIX}{level}"),
        format!("manifest {head_line} は未対応です。対応上限は4です"),
    )?;
    let classic = level <= 3;
    if classic {
        // 今の形は今の上限で読む（zip64 は使えない）
        check(!any_zip64, "ZIP64は未対応です")?;
        check(
            listed.len() <= CLASSIC_ENTRIES + 2,
            "ZIPエントリ数が不正です",
        )?;
        check_budget(
            manifest_entry.len <= CLASSIC_MANIFEST,
            "展開の予算超過です",
        )?;
    }
    let mut entries: BTreeMap<String, (u64, String)> = BTreeMap::new();
    let mut total = 0u64;
    let mut limit_tally = LimitTally::new(limits);
    for line in lines.filter(|s| !s.is_empty()) {
        let parts: Vec<_> = line.split(' ').collect();
        check(parts.len() == 3, "manifestの行が不正です")?;
        let (digest, name) = (parts[0], parts[2]);
        archive::name_check(name, level.min(3), YLP_PREFIX)?;
        check(
            is_hash(digest) && !parts[1].is_empty() && parts[1].bytes().all(|b| b.is_ascii_digit()),
            "manifestのハッシュまたは長さが不正です",
        )?;
        let len = parts[1]
            .parse::<u64>()
            .map_err(|_| Error::Budget("長さが過大です".into()))?;
        total = total
            .checked_add(len)
            .ok_or_else(|| Error::Budget("長さが過大です".into()))?;
        if classic {
            check_budget(
                len <= CLASSIC_ENTRY_BYTES && total <= CLASSIC_TOTAL_BYTES,
                "manifestの予算超過です",
            )?;
        } else {
            check_budget(len <= entry_limit(name), ONE_ENTRY_OVER)?;
            limit_tally.add(name, len)?;
        }
        check(
            name != MANIFEST && name != "mimetype",
            "予約されたエントリ名です",
        )?;
        check(
            entries.insert(name.to_owned(), (len, digest.to_owned())).is_none(),
            "manifestに重複があります",
        )?;
    }
    if !classic {
        check_budget(entries.len() <= MAX_ENTRIES, "エントリの数の上限を超えています")?;
        limit_tally.finish()?;
    }
    // 行の形・名前・予算は全部確かめてから、エントリと突き合わせる（予算超過をほかの食い違いより先に言い分ける）。manifest に
    // 無いエントリは展開しない
    check(
        by_name.len() == entries.len() + 2
            && entries.keys().all(|n| by_name.contains_key(n.as_str())),
        "manifestにないエントリがあります",
    )?;
    // エントリごとの確かめ（流して長さ・CRC・SHA-256 を数える）は互いに独立なので、エントリごとに並べる。結果は 1 つずつ確かめるのと同じで、
    // 断る理由も同じ（名前の順にいちばん前に断ったエントリの理由）
    let items: Vec<(&String, &(u64, String))> = entries.iter().collect();
    let verified = first_failure_in_order(&items, |&(name, (len, digest))| -> Result<Verified> {
        let l = by_name
            .get(name.as_str())
            .ok_or_else(|| Error::InvalidData(format!("エントリがありません: {name}")))?;
        check(
            l.len == *len,
            format!("SHA-256または長さが一致しません: {name}"),
        )?;
        let blob = zip_blob(source, l, Some(digest.clone()));
        // 1 回流して確かめる（小さなものはそのときメモリに残す）
        let mut r = blob.reader()?;
        if *len > keep && is_whole_document(name, &entries) {
            // 分けない大きな正本は、確かめながら骨組みも読む（開くときに同じ中身を 2 度流さない）
            let parsed = {
                let mut src = crate::native::StreamSource::new(&mut r);
                crate::native::Parse::begin(&mut src, None, crate::native::Keep::Skeleton).and_then(
                    |mut parse| {
                        while parse.next_layer()?.is_some() {}
                        parse.finish()
                    },
                )
            };
            // 読み残し（骨組みの読みが途中で断ったとき）も終わりまで流して、長さ・CRC・SHA-256 を確かめる
            copy(&mut r, &mut io::sink())?;
            Ok(Verified {
                blob,
                skeleton: Some(parsed.map(Arc::new).map_err(|e| e.to_string())),
            })
        } else if *len <= keep {
            // 宣言の長さで先に確保しない（中身が宣言より短い壊れたファイルで大きく確保しない）
            let mut v = Vec::with_capacity((*len).min(1 << 16) as usize);
            copy(&mut r, &mut v)?;
            Ok(Verified {
                blob: Blob::from(v),
                skeleton: None,
            })
        } else {
            copy(&mut r, &mut io::sink())?;
            Ok(Verified { blob, skeleton: None })
        }
    })?;
    let mut files = Files::new();
    let mut skeletons = BTreeMap::new();
    for ((name, _), done) in items.iter().zip(verified) {
        if let Some(skeleton) = done.skeleton {
            skeletons.insert((*name).clone(), skeleton);
        }
        files.insert((*name).clone(), done.blob);
    }
    archive::complete_ylp(files.keys(), level.min(3))?;
    Ok(Package {
        files,
        level,
        manifest: Some(Arc::from(manifest)),
        skeletons,
        donors: Vec::new(),
    })
}
/// 1 エントリの確かめの結果（メモリに残す中身か位置、分けない大きな正本なら読んだ骨組み）。
struct Verified {
    blob: Blob,
    skeleton: Option<std::result::Result<Arc<crate::NativeDocument>, String>>,
}
fn zip_blob(source: &Source, l: &Listed, sha: Option<String>) -> Blob {
    Blob(Repr::Zip(Arc::new(ZipRef {
        source: source.clone(),
        name: l.name.clone(),
        data: l.data,
        packed: l.packed,
        len: l.len,
        method: l.method,
        crc: l.crc,
        // SHA-256 の分からない読み（mimetype・manifest）は、確かめない札
        sha: sha.unwrap_or_default(),
    })))
}
/// 拡張フィールドを確かめ、zip64 の拡張があれば 32 bit の欄の代わりの値を読む（`[展開後, 圧縮後, 位置]` の順。0xFFFFFFFF の欄だけ）。
/// 返すのは（圧縮後, 展開後, 位置, zip64 の拡張があったか）。
fn wide_values(extra: &[u8], narrow: [u32; 3]) -> Result<(u64, u64, u64, bool)> {
    let mut values = narrow.map(u64::from);
    let mut found = false;
    let mut at = 0;
    while at < extra.len() {
        let kind = le16(slice(extra, at, 2)?, 0);
        let len = le16(slice(extra, at + 2, 2)?, 0) as usize;
        let body = slice(extra, at + 4, len)?;
        if kind == 1 {
            check(!found, "zip64の拡張が重複しています")?;
            found = true;
            let mut k = 0;
            for (i, v) in values.iter_mut().enumerate() {
                if narrow[i] == 0xFFFF_FFFF {
                    check(k + 8 <= body.len(), "zip64の拡張が短すぎます")?;
                    *v = le64(body, k);
                    k += 8;
                }
            }
            check(k == body.len(), "zip64の拡張の長さが一致しません")?;
        }
        at += 4 + len;
    }
    check(
        found || narrow.iter().all(|&n| n != 0xFFFF_FFFF),
        "zip64の拡張がありません",
    )?;
    Ok((values[1], values[0], values[2], found))
}
fn slice(b: &[u8], at: usize, n: usize) -> Result<&[u8]> {
    b.get(
        at..at
            .checked_add(n)
            .ok_or_else(|| Error::Budget("長さが過大です".into()))?,
    )
    .ok_or_else(|| Error::InvalidData("ZIPが途中で切れています".into()))
}
fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn le64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
fn read_n(b: &[u8], at: usize, n: usize) -> u64 {
    if n == 8 {
        le64(b, at)
    } else {
        u64::from(le32(b, at))
    }
}
fn put16(b: &mut Vec<u8>, v: u16) {
    b.extend(v.to_le_bytes())
}
fn put32(b: &mut Vec<u8>, v: u32) {
    b.extend(v.to_le_bytes())
}

/// ファイル全体の SHA-256 と長さを流して数える（保存の印）。
pub(crate) fn file_digest(path: &Path) -> Result<(String, u64)> {
    let mut f = BufReader::with_capacity(1 << 20, File::open(path)?);
    let mut sha = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut len = 0u64;
    loop {
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        sha.update(&buf[..n]);
        len += n as u64;
    }
    Ok((format!("{:x}", sha.finalize()), len))
}

#[cfg(test)]
#[path = "package_tests.rs"]
mod tests;
