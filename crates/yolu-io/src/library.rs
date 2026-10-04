//! 個人のライブラリ（プロジェクトをまたいで使う素材のフォルダ）の読み書き。
//!
//! ライブラリはふつうのフォルダで、PNG（画像）・`.ylsmart`（スマートマテリアル・スマートマスク）・`.ylbrush`・`.ylmaterial` を
//! 拡張子で見分ける。Unity 版の `ResourceLibraryFolder` と同じ作りで、同じフォルダを両方のアプリが指せる（相対パスは `/` 区切りで、
//! `.ylp` の出どころ `library` の `file` と同じ形）。
//!
//! 決まり:
//! - 一覧はフォルダの下を（深さと個数の上限まで）たどる。`.` で始まる名前・`~` で終わる名前（書き込み途中の一時ファイル）・
//!   シンボリックリンク（Windows のジャンクションを含む）・読めない名前（UTF-8 でない・制御文字・`\`・`:`・区切りが 255 UTF-16 単位より長い）は
//!   読み飛ばし、理由を残す。
//!   ライブラリの外のファイルを一覧・読み・書き・消しの対象にしない（相対パスの検査と、途中のシンボリックリンクの拒否）。
//! - 書くときは、同じ中身（同じ種類・同じバイト列）のファイルがあればそれを返し、無ければ、衝突しない名前（大文字小文字を区別しない
//!   環境でも重ならない）で一時ファイルへ書き、書き込みを確かめてから、上書きしない形で名前を付ける。途中で落ちても、本物の名前の
//!   ファイルは書きかけにならず、既にあるファイルを置き換えない。
//! - 同じ利用者の別のプロセスがフォルダを同時に変える競合は防がない（検査と開く間の差し替え）。
use crate::{check, check_budget, Error, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::SystemTime;

/// 一覧に出すファイルの個数の上限（超えた分は読み込まず、`Listing::truncated` で知らせる）。
pub const MAX_ENTRIES: usize = 4096;
/// たどるフォルダの深さの上限（ライブラリの直下を 0 とする）。
pub const MAX_DEPTH: usize = 8;
/// 相対パスの長さの上限（文字数。`.ylp` の出どころの上限と同じ）。
pub const MAX_REL_CHARS: usize = 1024;
/// 名前の 1 つの区切りの長さの上限（UTF-16 単位。NTFS・exFAT の上限で、ext4 などの 255 バイトの上限より広い: 255 バイトに収まる名前は
/// 255 UTF-16 単位にも収まる）。ファイルシステムの上限を超える名前のファイルは作れないので、実在するファイルの名前は、この上限で
/// 断ることがない（バイト数で数えると、NTFS の長い日本語名のファイルを一覧から落とす）。
pub const MAX_COMPONENT_UNITS: usize = 255;
/// 新しく付けるファイル名の、拡張子を除いた長さの上限（文字数。Unity 版の `ResourceLibraryFolder` と同じ 100 文字）。
pub const MAX_STEM_CHARS: usize = 100;
/// 新しく付けるファイル名の、拡張子を除いた長さの上限（UTF-8 のバイト数）。ext4 などは 1 つの名前を 255 バイトまでしか持てない
/// ので、全体からいちばん長い拡張子（`.ylmaterial`）・衝突の番号（` 4294967295`）・予約名の `_` の分を引いた値。日本語（3 バイト）は
/// 77 文字、絵文字（4 バイト）は 58 文字で切れる。
pub const MAX_STEM_BYTES: usize = 255 - ".ylmaterial".len() - " 4294967295".len() - 1;
/// 書き込みと読み込みで、取り消しの旗を見る間隔（バイト）。
const CHUNK: usize = 1 << 20;

/// 断る理由の文言。アプリが文言から短い理由を見分けるので、文言はここにだけ書く。
pub const REFUSAL_ROOT_LINK: &str = "ライブラリのフォルダがシンボリックリンクです";
pub const REFUSAL_ROOT_NOT_FOLDER: &str = "ライブラリの場所がフォルダではありません";
pub const REFUSAL_LINK: &str = "ライブラリの中のシンボリックリンクはたどりません";
pub const REFUSAL_PATH: &str = "ライブラリの相対パスが安全ではありません";
pub const REFUSAL_NOT_FILE: &str = "ライブラリのファイルがふつうのファイルではありません";
pub const REFUSAL_TOO_LARGE: &str = "ライブラリのファイルが大きすぎます";
pub const REFUSAL_EMPTY: &str = "空のファイルは入れられません";
pub const REFUSAL_NO_NAME: &str = "空いている名前が見つかりません";

/// ライブラリのファイルの種類（拡張子から）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// 画像（PNG）。
    Image,
    /// スマートマテリアル・スマートマスク（`.ylsmart`。どちらかはファイルの中を読まないと分からない）。
    Smart,
    Brush,
    Material,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Image, Kind::Smart, Kind::Brush, Kind::Material];

    /// 拡張子（ドットなし。大文字小文字を区別しない）から。
    pub fn of_extension(ext: &str) -> Option<Kind> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "png" => Kind::Image,
            "ylsmart" => Kind::Smart,
            "ylbrush" => Kind::Brush,
            "ylmaterial" => Kind::Material,
            _ => return None,
        })
    }

    /// 書くときの拡張子（ドットなし）。
    pub fn extension(self) -> &'static str {
        match self {
            Kind::Image => "png",
            Kind::Smart => "ylsmart",
            Kind::Brush => "ylbrush",
            Kind::Material => "ylmaterial",
        }
    }

    /// 種類ごとに新しく置くフォルダ（ライブラリの直下）。読むときは場所を問わない。
    pub fn folder(self) -> &'static str {
        match self {
            Kind::Image => "Images",
            Kind::Smart => "Smart",
            Kind::Brush => "Brushes",
            Kind::Material => "Materials",
        }
    }
}

/// 一覧の 1 つ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// ライブラリからの相対パス（`/` 区切り。`.ylp` の出どころの `file` と同じ）。
    pub rel: String,
    pub kind: Kind,
    pub len: u64,
    pub modified: Option<SystemTime>,
}

impl Entry {
    /// 拡張子を除いたファイル名（一覧に出す名前）。
    pub fn name(&self) -> &str {
        let leaf = self.rel.rsplit('/').next().unwrap_or(&self.rel);
        leaf.rsplit_once('.').map_or(leaf, |(stem, _)| stem)
    }
}

/// 一覧から読み飛ばした理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// シンボリックリンク（ジャンクションを含む）。
    Link,
    /// 名前が使えない（UTF-8 でない・制御文字・`\`・`:`・長すぎる）。
    Name,
    /// 深さの上限より下。
    Deep,
    /// 読めなかった（OS の理由）。
    Unreadable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub rel: String,
    pub reason: SkipReason,
}

/// フォルダを読んだ結果。
#[derive(Clone, Debug, Default)]
pub struct Listing {
    /// 名前の順（大文字小文字を区別しない。同じなら元の綴りの順）。
    pub entries: Vec<Entry>,
    pub skipped: Vec<Skipped>,
    /// 個数の上限（`MAX_ENTRIES`）で打ち切った。
    pub truncated: bool,
}

/// 相対パスが安全か: 空でない・長すぎない（全体 `MAX_REL_CHARS` 文字）・`/` で始まらない・`\` と `:` を含まない・区切りが空でも `.` でも
/// `..` でもない・制御文字を含まない・区切りが `MAX_COMPONENT_UNITS` UTF-16 単位以下。Unity 版の `ResourceOrigin.IsLibraryPath` が
/// 見るのは区切りの長さを除く同じ条件で、区切りの上限だけ Rust が足している（上限を超える名前のファイルは、どのファイルシステム
/// にも無いので、実在するファイルの一覧では同じ結果になる。`.ylp` に書かれただけの出どころの文字列は、上限を超えれば断る）。
pub fn is_library_path(rel: &str) -> bool {
    !rel.is_empty()
        && rel.chars().count() <= MAX_REL_CHARS
        && !rel.starts_with('/')
        && !rel.contains('\\')
        && !rel.contains(':')
        && rel.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.encode_utf16().count() <= MAX_COMPONENT_UNITS
                && !part.chars().any(|c| (c as u32) < 32 || c as u32 == 127)
        })
}

fn is_link(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

/// 一覧に出さない名前（`.` で始まる・`~` で終わる）。
fn hidden(name: &str) -> bool {
    name.starts_with('.') || name.ends_with('~')
}

fn cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|c| c.load(Ordering::SeqCst))
}

fn stopped() -> Error {
    Error::Core(yolu_core::CoreError::Cancelled)
}

/// ライブラリの根のフォルダ（無ければ空の一覧。ファイルやシンボリックリンクなら断る）の下を一覧にする。`cancel` が立つと、次のフォルダの
/// 区切りで取り消し（`Core(Cancelled)`）を返す。
pub fn list(root: &Path, cancel: Option<&AtomicBool>) -> Result<Listing> {
    let meta = match fs::symlink_metadata(root) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Listing::default()),
        Err(e) => return Err(e.into()),
    };
    check(!is_link(&meta), REFUSAL_ROOT_LINK)?;
    check(meta.is_dir(), REFUSAL_ROOT_NOT_FOLDER)?;
    let mut out = Listing::default();
    walk(root, "", 0, &mut out, cancel)?;
    out.entries.sort_by(|a, b| {
        a.rel
            .to_lowercase()
            .cmp(&b.rel.to_lowercase())
            .then_with(|| a.rel.cmp(&b.rel))
    });
    Ok(out)
}

fn walk(
    dir: &Path,
    prefix: &str,
    depth: usize,
    out: &mut Listing,
    cancel: Option<&AtomicBool>,
) -> Result<()> {
    if cancelled(cancel) {
        return Err(stopped());
    }
    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            out.skipped.push(Skipped {
                rel: prefix.trim_end_matches('/').to_owned(),
                reason: SkipReason::Unreadable(e.to_string()),
            });
            return Ok(());
        }
    };
    // 順序を OS に任せない（同じフォルダは、いつ読んでも同じ並び・同じ打ち切り位置）
    let mut children: Vec<_> = read.filter_map(|e| e.ok()).collect();
    children.sort_by_key(|e| e.file_name());
    for child in children {
        let Some(name) = child.file_name().to_str().map(str::to_owned) else {
            out.skipped.push(Skipped {
                rel: format!("{prefix}{}", child.file_name().to_string_lossy()),
                reason: SkipReason::Name,
            });
            continue;
        };
        if hidden(&name) {
            continue;
        }
        let rel = format!("{prefix}{name}");
        if !is_library_path(&rel) {
            out.skipped.push(Skipped {
                rel,
                reason: SkipReason::Name,
            });
            continue;
        }
        let meta = match child.metadata() {
            Ok(m) => m,
            Err(e) => {
                out.skipped.push(Skipped {
                    rel,
                    reason: SkipReason::Unreadable(e.to_string()),
                });
                continue;
            }
        };
        if is_link(&meta) {
            out.skipped.push(Skipped {
                rel,
                reason: SkipReason::Link,
            });
        } else if meta.is_dir() {
            if depth >= MAX_DEPTH {
                out.skipped.push(Skipped {
                    rel,
                    reason: SkipReason::Deep,
                });
            } else {
                walk(&child.path(), &format!("{rel}/"), depth + 1, out, cancel)?;
                if out.truncated {
                    return Ok(());
                }
            }
        } else if meta.is_file() {
            let Some(kind) = Path::new(&name)
                .extension()
                .and_then(|e| e.to_str())
                .and_then(Kind::of_extension)
            else {
                continue;
            };
            if out.entries.len() >= MAX_ENTRIES {
                out.truncated = true;
                return Ok(());
            }
            out.entries.push(Entry {
                rel,
                kind,
                len: meta.len(),
                modified: meta.modified().ok(),
            });
        }
    }
    Ok(())
}

/// 相対パスをライブラリの中の実際のパスにする。相対パスが安全でない・途中にシンボリックリンクがある（根を含む）ときは断る
/// （ライブラリの外を指させない）。ファイルが無くてもよい（作る前の検査にも使う）。
pub fn resolve(root: &Path, rel: &str) -> Result<PathBuf> {
    check(is_library_path(rel), REFUSAL_PATH)?;
    match fs::symlink_metadata(root) {
        Ok(m) => check(!is_link(&m), REFUSAL_ROOT_LINK)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut current = root.to_path_buf();
    for part in rel.split('/') {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(m) => check(!is_link(&m), REFUSAL_LINK)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(current)
}

/// 中身の SHA-256（小文字の 16 進）。
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// 開いたファイルの長さが `limit` を超えていれば、読む前に予算超過で断る。ファイルでない（フォルダ・デバイス）ものも断る。
fn open_checked(path: &Path, limit: u64) -> Result<(fs::File, u64)> {
    let file = fs::File::open(path)?;
    let meta = file.metadata()?;
    check(meta.is_file(), REFUSAL_NOT_FILE)?;
    check_budget(meta.len() <= limit, REFUSAL_TOO_LARGE)?;
    Ok((file, meta.len()))
}

/// ライブラリのファイルを読む。長さが `limit` を超えるときは読む前に断る（開いたファイルで確かめるので、確かめた後に伸びても
/// `limit` を超えて読まない）。`cancel` が立つと、読み込みの区切りで取り消す。
pub fn read(root: &Path, rel: &str, limit: u64, cancel: Option<&AtomicBool>) -> Result<Vec<u8>> {
    let path = resolve(root, rel)?;
    let (mut file, len) = open_checked(&path, limit)?;
    let mut out = Vec::with_capacity(len as usize);
    let mut buf = vec![0u8; CHUNK.min(len as usize).max(1)];
    loop {
        if cancelled(cancel) {
            return Err(stopped());
        }
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        check_budget(out.len() as u64 + n as u64 <= limit, REFUSAL_TOO_LARGE)?;
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

/// ファイルの SHA-256 を、全部を読み込まずに求める（`limit` と `cancel` は `read` と同じ）。長さも返す。
pub fn hash_file(
    root: &Path,
    rel: &str,
    limit: u64,
    cancel: Option<&AtomicBool>,
) -> Result<(String, u64)> {
    let path = resolve(root, rel)?;
    let (mut file, len) = open_checked(&path, limit)?;
    let mut sha = Sha256::new();
    let mut buf = vec![0u8; CHUNK.min(len as usize).max(1)];
    let mut total = 0u64;
    loop {
        if cancelled(cancel) {
            return Err(stopped());
        }
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        check_budget(total <= limit, REFUSAL_TOO_LARGE)?;
        sha.update(&buf[..n]);
    }
    Ok((format!("{:x}", sha.finalize()), total))
}

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 素材の名前から、ファイル名の（拡張子を除いた）部分を作る: ファイルシステムが断る文字は `_`、前後の空白・ドットを除き、
/// `MAX_STEM_CHARS` 文字・`MAX_STEM_BYTES` バイトまで（文字の途中では切らない）。Windows の予約名（CON・NUL など）は先頭に `_` を足す。空になれば `Asset`。
pub fn safe_stem(name: &str) -> String {
    let replaced: String = name
        .trim()
        .chars()
        .map(|c| {
            if (c as u32) < 32 || c as u32 == 127 || "\\/:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = replaced.trim_matches(['.', ' ']);
    let mut stem = String::new();
    for c in trimmed.chars().take(MAX_STEM_CHARS) {
        if stem.len() + c.len_utf8() > MAX_STEM_BYTES {
            break;
        }
        stem.push(c);
    }
    stem = stem.trim_end_matches(['.', ' ']).to_owned();
    if stem.is_empty() {
        return "Asset".into();
    }
    let head = stem.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&head.as_str()) {
        stem.insert(0, '_');
    }
    stem
}

/// `add` の結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Added {
    /// ライブラリの中の相対パス（既にあった場合はそのファイルの）。
    pub rel: String,
    /// 同じ中身のファイルが既にあって、書かなかった。
    pub existed: bool,
    pub sha256: String,
    pub len: u64,
}

/// `entry` の中身が `bytes` と同じか（長さが同じものだけ、読んで比べる）。
fn same_bytes(root: &Path, entry: &Entry, bytes: &[u8], cancel: Option<&AtomicBool>) -> bool {
    if entry.len != bytes.len() as u64 {
        return false;
    }
    let Ok(path) = resolve(root, &entry.rel) else {
        return false;
    };
    let Ok((mut file, _)) = open_checked(&path, bytes.len() as u64) else {
        return false;
    };
    let mut buf = vec![0u8; CHUNK.min(bytes.len()).max(1)];
    let mut at = 0usize;
    loop {
        if cancelled(cancel) {
            return false;
        }
        let Ok(n) = file.read(&mut buf) else {
            return false;
        };
        if n == 0 {
            return at == bytes.len();
        }
        if at + n > bytes.len() || buf[..n] != bytes[at..at + n] {
            return false;
        }
        at += n;
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        ".{:x}-{:x}-{:x}.tmp~",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// フォルダを（途中のシンボリックリンクを断って）作る。
fn make_dirs(root: &Path, dir: &str) -> Result<PathBuf> {
    if dir.is_empty() {
        resolve_root(root)?;
        fs::create_dir_all(root)?;
        return Ok(root.to_path_buf());
    }
    let path = resolve(root, dir)?;
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn resolve_root(root: &Path) -> Result<()> {
    if let Ok(m) = fs::symlink_metadata(root) {
        check(!is_link(&m), REFUSAL_ROOT_LINK)?;
    }
    Ok(())
}

/// `dir`（ライブラリの直下からの相対。空ならライブラリの直下）の中で、まだ無い名前（大文字小文字を区別しない）を `stem` に番号を
/// 足して決める。
fn unique_name(path: &Path, stem: &str, ext: &str) -> String {
    let taken: std::collections::HashSet<String> = fs::read_dir(path)
        .map(|r| {
            r.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_str().map(str::to_lowercase))
                .collect()
        })
        .unwrap_or_default();
    let name = format!("{stem}.{ext}");
    if !taken.contains(&name.to_lowercase()) {
        return name;
    }
    (2u32..)
        .map(|n| format!("{stem} {n}.{ext}"))
        .find(|n| !taken.contains(&n.to_lowercase()))
        .expect("無限の並び")
}

/// 素材のバイト列をライブラリへ入れる（`dir` はライブラリの直下からの相対のフォルダ。空なら直下）。同じ種類で同じバイト列のファイルが
/// （どのフォルダにも）既にあれば何も書かずにそれを返す。無ければ `name` から作った名前（`safe_stem`。重なれば番号を足す）で、
/// 一時ファイルへ書いて確かめてから、上書きしない形で名前を付ける。`cancel` が立つと、書き込みの区切りで一時ファイルを消して
/// 取り消す（本物の名前のファイルは作らない）。
pub fn add(
    root: &Path,
    dir: &str,
    name: &str,
    kind: Kind,
    bytes: &[u8],
    cancel: Option<&AtomicBool>,
) -> Result<Added> {
    add_with(root, dir, name, kind, bytes, cancel, &Faults::default())
}

/// 書き込みの区切りの口（一時ファイルのパス・書き終えた区切りの数）。
#[doc(hidden)]
pub type ChunkHook<'a> = &'a dyn Fn(&Path, usize);

/// 試験用の口（`add_with`）。書き込みの途中・名前を付ける段で、ふつうは起きない道を通すために使う。
#[doc(hidden)]
#[derive(Default)]
pub struct Faults<'a> {
    /// 一時ファイルへ 1 区切り（`CHUNK` バイト）書くたびに、一時ファイルのパスと、書き終えた区切りの数を渡して呼ぶ
    /// （取り消しの旗を書き込みの途中で立てる・書きかけの一時ファイルを見る）。
    pub after_chunk: Option<ChunkHook<'a>>,
    /// true なら、ハードリンクを作れないファイルシステムの再現（名前を付ける段が付け替えの枝へ進む）。
    pub no_hard_link: bool,
}

/// `add` と同じ。`faults` で、書き込みの途中の取り消しとハードリンクの無いファイルシステムを再現する（試験用）。
#[doc(hidden)]
pub fn add_with(
    root: &Path,
    dir: &str,
    name: &str,
    kind: Kind,
    bytes: &[u8],
    cancel: Option<&AtomicBool>,
    faults: &Faults<'_>,
) -> Result<Added> {
    check(dir.is_empty() || is_library_path(dir), REFUSAL_PATH)?;
    check(!bytes.is_empty(), REFUSAL_EMPTY)?;
    resolve_root(root)?;
    let sha256 = sha256_hex(bytes);
    let len = bytes.len() as u64;
    let existing = list(root, cancel)?;
    for entry in existing.entries.iter().filter(|e| e.kind == kind) {
        if same_bytes(root, entry, bytes, cancel) {
            return Ok(Added {
                rel: entry.rel.clone(),
                existed: true,
                sha256,
                len,
            });
        }
        if cancelled(cancel) {
            return Err(stopped());
        }
    }
    let folder = make_dirs(root, dir)?;
    let ext = kind.extension();
    let stem = safe_stem(name);
    let temp = folder.join(temp_name());
    let written = write_synced(&temp, bytes, cancel, faults.after_chunk);
    if let Err(e) = written {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    // 名前を付ける: 上書きしないハードリンクで確定する。他のプロセスが同じ名前を先に作っていたら、次の番号で取り直す。
    // ハードリンクを持たないファイルシステムでは、名前が空いているのを確かめてから付け替える。
    let result = (|| -> Result<String> {
        for _ in 0..64 {
            let file = unique_name(&folder, &stem, ext);
            let target = folder.join(&file);
            let linked = if faults.no_hard_link {
                Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
            } else {
                fs::hard_link(&temp, &target)
            };
            match linked {
                Ok(()) => return Ok(file),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => {
                    if fs::symlink_metadata(&target).is_ok() {
                        continue;
                    }
                    fs::rename(&temp, &target)?;
                    return Ok(file);
                }
            }
        }
        Err(Error::InvalidData(REFUSAL_NO_NAME.into()))
    })();
    let _ = fs::remove_file(&temp);
    let file = result?;
    let rel = if dir.is_empty() {
        file
    } else {
        format!("{dir}/{file}")
    };
    Ok(Added {
        rel,
        existed: false,
        sha256,
        len,
    })
}

/// 一時ファイルへ書いて、ディスクへ確定する（既にあれば失敗。`cancel` を区切りごとに見る。`after_chunk` は試験用）。
fn write_synced(
    path: &Path,
    bytes: &[u8],
    cancel: Option<&AtomicBool>,
    after_chunk: Option<ChunkHook<'_>>,
) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    for (n, chunk) in bytes.chunks(CHUNK).enumerate() {
        if cancelled(cancel) {
            return Err(stopped());
        }
        file.write_all(chunk)?;
        if let Some(hook) = after_chunk {
            hook(path, n + 1);
        }
    }
    // 最後の区切りのあとに旗が立っても、名前を付ける前に取り消せる
    if cancelled(cancel) {
        return Err(stopped());
    }
    file.sync_all()?;
    Ok(())
}

/// ライブラリのファイルを消す（ふつうのファイルだけ。フォルダとシンボリックリンクは消さない。空になったフォルダは残す）。
pub fn remove(root: &Path, rel: &str) -> Result<()> {
    let path = resolve(root, rel)?;
    let meta = fs::symlink_metadata(&path)?;
    check(meta.is_file(), REFUSAL_NOT_FILE)?;
    fs::remove_file(&path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_paths_follow_the_ylp_origin_rule() {
        for ok in ["a.png", "Images/a b.png", "木目/柄.ylsmart", "a/b/c.png"] {
            assert!(is_library_path(ok), "{ok}");
        }
        for bad in [
            "",
            "/a.png",
            "a//b.png",
            "a/./b.png",
            "../a.png",
            "a/..",
            "a\\b.png",
            "C:a.png",
            "a/b:c.png",
            "a\u{0}b.png",
            "a/\u{7f}.png",
            "a/b/",
        ] {
            assert!(!is_library_path(bad), "{bad:?}");
        }
        // 長さの上限は全体の文字数（区切りは 255 UTF-16 単位まで）
        let long = |chars: usize| {
            let mut text = String::new();
            while text.chars().count() < chars {
                if !text.is_empty() {
                    text.push('/');
                }
                text.push_str(&"a".repeat((chars - text.chars().count()).min(200)));
            }
            text
        };
        assert!(is_library_path(&long(MAX_REL_CHARS)));
        assert!(!is_library_path(&long(MAX_REL_CHARS + 1)));
        assert!(is_library_path(&"a".repeat(MAX_COMPONENT_UNITS)));
        assert!(!is_library_path(&"a".repeat(MAX_COMPONENT_UNITS + 1)));
    }

    #[test]
    fn stems_lose_what_a_path_cannot_hold() {
        assert_eq!(safe_stem("a/b:c"), "a_b_c");
        assert_eq!(safe_stem("  "), "Asset");
        assert_eq!(safe_stem("..."), "Asset");
        assert_eq!(safe_stem("木目"), "木目");
        assert_eq!(safe_stem(" x. "), "x");
        assert_eq!(safe_stem("con"), "_con");
        assert_eq!(safe_stem("NUL.txt"), "_NUL.txt");
        assert_eq!(safe_stem("com1x"), "com1x");
        assert_eq!(safe_stem(&"a".repeat(300)).chars().count(), MAX_STEM_CHARS);
        // 文字数の上限より先にバイト数の上限が効く名前は、文字の途中で切らない
        let japanese = safe_stem(&"あ".repeat(300));
        assert!(japanese.len() <= MAX_STEM_BYTES && japanese.chars().all(|c| c == 'あ'));
        assert_eq!(japanese.chars().count(), MAX_STEM_BYTES / 3);
        let emoji = safe_stem(&"😀".repeat(300));
        assert_eq!(emoji.chars().count(), MAX_STEM_BYTES / 4);
        // 予約名の `_` を足しても、バイト数の上限に収まる
        assert!(safe_stem(&format!("con.{}", "a".repeat(300))).len() <= MAX_STEM_BYTES + 1);
    }

    #[test]
    fn kinds_follow_the_extension_in_any_case() {
        assert_eq!(Kind::of_extension("PNG"), Some(Kind::Image));
        assert_eq!(Kind::of_extension("YlSmart"), Some(Kind::Smart));
        assert_eq!(Kind::of_extension("jpg"), None);
        for k in Kind::ALL {
            assert_eq!(Kind::of_extension(k.extension()), Some(k));
        }
    }

    #[test]
    fn an_entry_name_drops_the_folder_and_the_extension() {
        let e = Entry {
            rel: "Smart/Rusty.v2.ylsmart".into(),
            kind: Kind::Smart,
            len: 1,
            modified: None,
        };
        assert_eq!(e.name(), "Rusty.v2");
    }
}
