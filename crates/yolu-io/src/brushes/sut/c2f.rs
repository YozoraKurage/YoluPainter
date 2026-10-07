//! CLIP STUDIO の独自の入れ物 C2F（素材の `.layer`）から、素材の元の画像を読む。
//!
//! 形（本物の `.sut` 1 つの素材 4 つを調べた。確かな所と推測の所は docs/BRUSH_IMPORT.md）:
//! - 先頭 8 バイトが `\x89C2F\r\n\x1a\n`。続けて PNG に似た塊の並び: 長さ（u32 LE）・種類（4 文字）・中身・CRC32（LE。種類と中身）。
//!   `HEAD`（中身なし）・`dATA` 2 つ・`TAIL`（中身なし）。
//! - 1 つ目の `dATA` は、先頭 2 バイトが `00 00` でなく（`01 00`）、残りは値の偏りがほとんど無い。標準の展開器（zlib・生の deflate・bzip2・
//!   lzma。塊の先頭からと、先頭 2 バイトの後ろから）では開けなかったので、それ以上は調べておらず、読まない。中身は SQLite の最初の数ページに
//!   当たるらしく、ページ数は `(長さ − 2) / 1024` と数える（推測）。
//! - 2 つ目の `dATA` は、先頭 2 バイトが `00 00`、続けて 1024 バイトのページの並び（SQLite のテーブル B 木のページ。文字は UTF-8 か
//!   UTF-16LE）。ページ番号は 1 つ目の塊のページに続けて数える。
//! - 画像の画素は `Offscreen` の `BlockData` の中にある（256×256 のタイルごとの zlib）。表の根のページが読む側（2 つ目の `dATA`）にあれば、SQLite 本体を通さず
//!   B 木を自前で読める（SQLite の最初のページが読めないので、SQLite には載せられない）。
//!
//! 信頼できない入力として、塊の数・行の大きさ・木の深さ・画像の 1 辺に上限を置き、組み立てる行の大きさと展開する画素の数は 1 回の取り込み全体の
//! 合計（[`Work`]）で縛り、壊れていれば [`Refusal`] で断る（パニックしない）。

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::io::Read;

use flate2::read::ZlibDecoder;
use yolu_core::BrushTip;

const SIGNATURE: &[u8] = b"\x89C2F\r\n\x1a\n";

/// 1 ページの大きさ（見た素材すべてで 1024。ページ 1 が読めず、ヘッダーの宣言は確かめられない）。
const PAGE: usize = 1024;

/// 塊の数の上限（見たものは 4 つ）。
const MAX_CHUNKS: usize = 64;

/// B 木の深さの上限（根を 0 として、葉がこの深さまで）。
const MAX_DEPTH: usize = 8;

/// 1 行（オーバーフローを含む）の大きさの上限。
const MAX_PAYLOAD: usize = 64 * 1024 * 1024;

/// 1 回の取り込み（1 つの `.sut`）で組み立てる行の大きさの合計の上限。素材は 256 個まであり、素材ごとの上限だけでは掛け算になるので、
/// 取り込み全体で数える。
const MAX_MATERIALIZED: u64 = 256 * 1024 * 1024;

/// 1 回の取り込みで画像の面に展開する画素の合計の上限（筆先・質感の画素の予算と同じ大きさ。数えるのは別）。素材の面の確保・変換・
/// PNG の符号化は、どのブラシにも使われない素材も含めて、素材を並べるときに前もって行うため、筆先の予算とは別に縛る。
const MAX_PIXELS: u64 = crate::brushes::MAX_DECODED_BYTES;

/// 1 行の列の数の上限。
const MAX_COLUMNS: usize = 1024;

/// 1 つの面のタイルの 1 辺（画素）。
const TILE: u32 = 256;
const TILE_BYTES: usize = (TILE * TILE) as usize;

/// 画像の 1 辺の上限（筆先と同じ）。
const MAX_SIDE: u32 = BrushTip::MAX_SIZE;

const BEGIN_NAME: &str = "BlockDataBeginChunk";
const END_NAME: &str = "BlockDataEndChunk";

/// 読めない理由（試験で区別する。画面には出さない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// 先頭の印が C2F でない。
    NotC2f,
    /// 塊が途中で切れている・長さが合わない。
    Truncated,
    /// 塊の CRC が合わない。
    BadCrc,
    /// 知らない種類の塊・塊の順序の違い。
    UnknownChunk,
    /// 塊が多すぎる。
    TooManyChunks,
    /// 必要なページが読まない側（1 つ目の `dATA`）にある。
    Hidden,
    /// データベースの形が壊れている（ページ・B 木・行・オーバーフロー）。
    Corrupt,
    /// 必要な表が見つからない。
    NoTable,
    /// 同じ名前の表が別の根で見つかって決められない。
    Ambiguous,
    /// 画像のレイヤーが 1 枚に決まらない。
    Layers,
    /// 画素が 1 つも無い。
    Empty,
    /// 面の記述（`Attribute`）の形が違う。
    Attribute,
    /// 対応しない形（色の面・8 bit でない・256 でないタイル・知らない初期色）。
    Unsupported,
    /// 画像が大きすぎる。
    TooLarge,
    /// 1 回の取り込みで素材の C2F に使える仕事（行の組み立て・画素の展開）の合計を超える。
    OverBudget,
    /// タイルの画素（`BlockData`・zlib）が壊れている。
    BadTile,
}

type Result<T> = std::result::Result<T, Refusal>;

/// 読み出した画像。1 画素 1 バイト、行は上から。値は黒の不透明度（0 が空）。
pub(super) struct Plane {
    pub width: u32,
    pub height: u32,
    pub values: Vec<u8>,
}

/// 1 回の取り込み（1 つの `.sut`）の中で、素材の C2F を読むのに使える仕事の残り。素材ごとの上限だけでは、小さな素材を 256 個並べるだけで
/// 仕事が 256 倍になるので、取り込み全体で数える（使い切ったあとの素材は [`Refusal::OverBudget`] で断り、素材の画像は無いものとして扱う）。
pub(super) struct Work {
    materialized: Cell<u64>,
    pixels: Cell<u64>,
}

impl Work {
    pub fn new() -> Work {
        Work::with_limits(MAX_MATERIALIZED, MAX_PIXELS)
    }

    fn with_limits(materialized: u64, pixels: u64) -> Work {
        Work {
            materialized: Cell::new(materialized),
            pixels: Cell::new(pixels),
        }
    }

    fn take(counter: &Cell<u64>, amount: u64) -> Result<()> {
        let left = counter
            .get()
            .checked_sub(amount)
            .ok_or(Refusal::OverBudget)?;
        counter.set(left);
        Ok(())
    }
}

// ---------------- 小さな部品 ----------------

fn be16(b: &[u8], at: usize) -> Option<usize> {
    let v = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([v[0], v[1]]) as usize)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    let v = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([v[0], v[1], v[2], v[3]]))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    let v = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
}

/// SQLite の可変長整数（最大 9 バイト）。
fn varint(b: &[u8], at: usize) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    for i in 0..8 {
        let byte = *b.get(at.checked_add(i)?)?;
        value = (value << 7) | (byte & 0x7f) as u64;
        if byte & 0x80 == 0 {
            return Some((value, at + i + 1));
        }
    }
    let byte = *b.get(at.checked_add(8)?)?;
    Some(((value << 8) | byte as u64, at + 9))
}

fn utf16be(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
}

// ---------------- 塊 ----------------

/// 塊の並びを検査し、`dATA` の中身を順に返す。
fn chunks(data: &[u8]) -> Result<Vec<&[u8]>> {
    if !data.starts_with(SIGNATURE) {
        return Err(Refusal::NotC2f);
    }
    let mut pos = SIGNATURE.len();
    let mut bodies = Vec::new();
    let mut count = 0usize;
    let mut tail = false;
    while pos < data.len() {
        count += 1;
        if count > MAX_CHUNKS {
            return Err(Refusal::TooManyChunks);
        }
        let len = le32(data, pos).ok_or(Refusal::Truncated)? as usize;
        let kind = data.get(pos + 4..pos + 8).ok_or(Refusal::Truncated)?;
        let body_end = (pos + 8).checked_add(len).ok_or(Refusal::Truncated)?;
        let end = body_end.checked_add(4).ok_or(Refusal::Truncated)?;
        let body = data.get(pos + 8..body_end).ok_or(Refusal::Truncated)?;
        let stored = le32(data, body_end).ok_or(Refusal::Truncated)?;
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(kind);
        hasher.update(body);
        if hasher.finalize() != stored {
            return Err(Refusal::BadCrc);
        }
        match kind {
            b"HEAD" if count == 1 => {}
            b"dATA" if count > 1 => bodies.push(body),
            b"TAIL" if count > 1 => {
                tail = true;
                break;
            }
            _ => return Err(Refusal::UnknownChunk),
        }
        pos = end;
    }
    // 先頭の HEAD が無いと count == 1 の塊が HEAD でなく断られる。TAIL の後ろのバイトは読まない
    if !tail || bodies.is_empty() {
        return Err(Refusal::Truncated);
    }
    Ok(bodies)
}

/// `dATA` の並びをページの並びにする。先頭 2 バイトが `00 00` の塊は 1024 バイトずつのページ。それ以外（標準の展開器で開けなかった塊）は読まず、
/// ページ数だけ数えて場所を空ける。
fn pages<'a>(bodies: &[&'a [u8]]) -> Result<Vec<Option<&'a [u8]>>> {
    let mut out = Vec::new();
    for body in bodies {
        let (flag, rest) = body.split_at_checked(2).ok_or(Refusal::Corrupt)?;
        if flag == [0, 0] {
            if rest.is_empty() || !rest.len().is_multiple_of(PAGE) {
                return Err(Refusal::Corrupt);
            }
            out.extend(
                rest.as_chunks::<PAGE>()
                    .0
                    .iter()
                    .map(|page| Some(page.as_slice())),
            );
        } else {
            let n = rest.len() / PAGE;
            if n == 0 {
                return Err(Refusal::Corrupt);
            }
            out.extend(std::iter::repeat_n(None, n));
        }
    }
    Ok(out)
}

// ---------------- データベース ----------------

#[derive(Debug)]
enum Value {
    Null,
    Int(i64),
    Text(String),
    Blob(Vec<u8>),
    /// 浮動小数点（読まない）。
    Other,
}

impl Value {
    fn int(&self) -> i64 {
        match self {
            Value::Int(v) => *v,
            _ => 0,
        }
    }
}

struct Table {
    root: u32,
    /// 列の名前（小文字）。
    columns: Vec<String>,
}

impl Table {
    fn index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }

    fn int(&self, row: &[Value], name: &str) -> i64 {
        self.index(name)
            .and_then(|i| row.get(i))
            .map_or(0, Value::int)
    }

    fn blob<'r>(&self, row: &'r [Value], name: &str) -> Option<&'r [u8]> {
        match row.get(self.index(name)?)? {
            Value::Blob(b) => Some(b),
            _ => None,
        }
    }
}

/// 葉の 1 つのセル（局所の部分とオーバーフローの先頭）。
struct LeafCell<'a> {
    total: usize,
    local: &'a [u8],
    overflow: u32,
}

struct Db<'a> {
    pages: Vec<Option<&'a [u8]>>,
    /// 文字が UTF-16LE か（表の定義の行から決める）。
    utf16: bool,
    tables: HashMap<String, Vec<Table>>,
    /// 取り込み全体の仕事の残り（行を組み立てるたびに引く）。
    work: &'a Work,
}

impl<'a> Db<'a> {
    fn open(pages: Vec<Option<&'a [u8]>>, work: &'a Work) -> Result<Db<'a>> {
        let mut db = Db {
            pages,
            utf16: false,
            tables: HashMap::new(),
            work,
        };
        db.scan_schema()?;
        Ok(db)
    }

    fn page(&self, no: u32) -> Result<&'a [u8]> {
        match self.pages.get((no as usize).wrapping_sub(1)) {
            Some(Some(page)) => Ok(page),
            Some(None) => Err(Refusal::Hidden),
            None => Err(Refusal::Corrupt),
        }
    }

    /// 葉ページのセルの位置から、セルの局所の部分を取る。
    fn leaf_cell(page: &'a [u8], at: usize) -> Option<LeafCell<'a>> {
        let (total, p) = varint(page, at)?;
        // 行の番号は読まない（参照は列の `MainId`）
        let (_, p) = varint(page, p)?;
        let total = usize::try_from(total).ok()?;
        if total > MAX_PAYLOAD {
            return None;
        }
        // 局所の大きさ（SQLite の規則。U = 1024）
        let usable = PAGE;
        let max_local = usable - 35;
        let min_local = ((usable - 12) * 32 / 255) - 23;
        let (local_len, overflow) = if total <= max_local {
            (total, 0)
        } else {
            let k = min_local + ((total - min_local) % (usable - 4));
            let local = if k <= max_local { k } else { min_local };
            (local, be32(page, p.checked_add(local)?)?)
        };
        let local = page.get(p..p.checked_add(local_len)?)?;
        Some(LeafCell {
            total,
            local,
            overflow,
        })
    }

    /// セルの全体の中身（オーバーフローをたどる）。
    fn payload(&self, cell: &LeafCell<'a>) -> Result<Vec<u8>> {
        Work::take(&self.work.materialized, cell.total as u64)?;
        let mut out = Vec::with_capacity(cell.total);
        out.extend_from_slice(cell.local);
        let mut next = cell.overflow;
        // 1 つのオーバーフローページが運ぶのは 1020 バイト。たどる回数を中身の大きさに合わせて縛る（輪になっていても止まる）
        let mut hops = cell.total / (PAGE - 4) + 2;
        while out.len() < cell.total {
            if next == 0 || hops == 0 {
                return Err(Refusal::Corrupt);
            }
            hops -= 1;
            let page = self.page(next)?;
            let take = (cell.total - out.len()).min(PAGE - 4);
            out.extend_from_slice(&page[4..4 + take]);
            next = be32(page, 0).ok_or(Refusal::Corrupt)?;
        }
        Ok(out)
    }

    /// 表の定義の行（`sqlite_master`）を、読める葉ページから拾う。最初のページは読めないので、根からは辿らない。
    fn scan_schema(&mut self) -> Result<()> {
        let mut seen_utf8 = false;
        let mut seen_utf16 = false;
        let mut found: Vec<(String, Table)> = Vec::new();
        for page in self.pages.iter().flatten().copied() {
            if page[0] != 0x0d {
                continue;
            }
            let Some(n) = be16(page, 3) else { continue };
            if 8 + 2 * n > PAGE {
                continue;
            }
            for i in 0..n {
                let Some(at) = be16(page, 8 + 2 * i) else {
                    continue;
                };
                let Some(cell) = Self::leaf_cell(page, at) else {
                    continue;
                };
                // 局所の部分の先頭の型だけで絞る: (text, text, text, int, text) で、最初が 5 文字（"table"）
                let Some(types) = record_types(cell.local) else {
                    continue;
                };
                if types.len() != 5
                    || !(types[0] == 23 || types[0] == 33)
                    || !matches!(types[3], 1..=6 | 8 | 9)
                    || types[4] < 13
                    || types[4] % 2 == 0
                {
                    continue;
                }
                let utf16 = types[0] == 33;
                let Ok(payload) = self.payload(&cell) else {
                    continue;
                };
                let Ok(values) = decode_record(&payload, utf16) else {
                    continue;
                };
                let [Value::Text(kind), Value::Text(name), _, Value::Int(root), Value::Text(sql)] =
                    values.as_slice()
                else {
                    continue;
                };
                if kind != "table" || !starts_with_ignore_case(sql, "CREATE TABLE") {
                    continue;
                }
                let Ok(root) = u32::try_from(*root) else {
                    continue;
                };
                if utf16 {
                    seen_utf16 = true;
                } else {
                    seen_utf8 = true;
                }
                found.push((
                    name.to_ascii_lowercase(),
                    Table {
                        root,
                        columns: columns_of(sql),
                    },
                ));
            }
        }
        // 文字の形が混ざる（表の定義の行どうしで違う）ものは、どちらで読むか決められない
        if seen_utf8 && seen_utf16 {
            return Err(Refusal::Corrupt);
        }
        self.utf16 = seen_utf16;
        for (name, table) in found {
            let list = self.tables.entry(name).or_default();
            // 同じ行が別のページに写って残っていることがある（根が同じなら同じ表）
            if !list.iter().any(|t| t.root == table.root) {
                list.push(table);
            }
        }
        Ok(())
    }

    fn table(&self, name: &str) -> Result<&Table> {
        match self.tables.get(&name.to_ascii_lowercase()) {
            None => Err(Refusal::NoTable),
            Some(list) if list.len() == 1 => Ok(&list[0]),
            Some(_) => Err(Refusal::Ambiguous),
        }
    }

    /// 根から葉ページを順に集める（輪・深すぎる木・知らない種類のページは断る）。
    fn leaves(&self, root: u32) -> Result<Vec<u32>> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        self.walk(root, 0, &mut seen, &mut out)?;
        Ok(out)
    }

    fn walk(
        &self,
        no: u32,
        depth: usize,
        seen: &mut HashSet<u32>,
        out: &mut Vec<u32>,
    ) -> Result<()> {
        if depth > MAX_DEPTH || !seen.insert(no) {
            return Err(Refusal::Corrupt);
        }
        let page = self.page(no)?;
        match page[0] {
            0x0d => out.push(no),
            0x05 => {
                let n = be16(page, 3).ok_or(Refusal::Corrupt)?;
                if 12 + 2 * n > PAGE {
                    return Err(Refusal::Corrupt);
                }
                for i in 0..n {
                    let at = be16(page, 12 + 2 * i).ok_or(Refusal::Corrupt)?;
                    let child = be32(page, at).ok_or(Refusal::Corrupt)?;
                    self.walk(child, depth + 1, seen, out)?;
                }
                let right = be32(page, 8).ok_or(Refusal::Corrupt)?;
                self.walk(right, depth + 1, seen, out)?;
            }
            _ => return Err(Refusal::Corrupt),
        }
        Ok(())
    }

    /// 表の行のうち `keep` が受け入れたもの（行は列の並びの値）。
    fn select(
        &self,
        table: &Table,
        mut keep: impl FnMut(&[Value]) -> bool,
    ) -> Result<Vec<Vec<Value>>> {
        let mut out = Vec::new();
        for no in self.leaves(table.root)? {
            let page = self.page(no)?;
            let n = be16(page, 3).ok_or(Refusal::Corrupt)?;
            if 8 + 2 * n > PAGE {
                return Err(Refusal::Corrupt);
            }
            for i in 0..n {
                let at = be16(page, 8 + 2 * i).ok_or(Refusal::Corrupt)?;
                let cell = Self::leaf_cell(page, at).ok_or(Refusal::Corrupt)?;
                let payload = self.payload(&cell)?;
                let row = decode_record(&payload, self.utf16)?;
                if keep(&row) {
                    out.push(row);
                }
            }
        }
        Ok(out)
    }
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len()
        && text.is_char_boundary(prefix.len())
        && text[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// `CREATE TABLE 名前(列, 列, ...)` から列の名前（小文字）を順に取る。制約の行は数えない。
fn columns_of(sql: &str) -> Vec<String> {
    let (Some(open), Some(close)) = (sql.find('('), sql.rfind(')')) else {
        return Vec::new();
    };
    if close <= open {
        return Vec::new();
    }
    let inner = &sql[open + 1..close];
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    parts
        .into_iter()
        .filter_map(|part| {
            // 先頭の語（`UNIQUE(x)` のように括弧が続くものは括弧の前まで）
            let first = part.split_whitespace().next()?.split('(').next()?;
            let name = first.trim_matches(|c| matches!(c, '"' | '`' | '[' | ']' | '\''));
            let upper = name.to_ascii_uppercase();
            if matches!(
                upper.as_str(),
                "PRIMARY" | "UNIQUE" | "CHECK" | "FOREIGN" | "CONSTRAINT"
            ) {
                return None;
            }
            Some(name.to_ascii_lowercase())
        })
        .take(MAX_COLUMNS)
        .collect()
}

/// 行の局所の部分の先頭から、列の型（シリアル型）の並びを読む。ヘッダーが局所の部分に入らなければ None。
fn record_types(local: &[u8]) -> Option<Vec<u64>> {
    let (header, mut p) = varint(local, 0)?;
    let header = usize::try_from(header).ok()?;
    if header > local.len() || header > 8 * MAX_COLUMNS {
        return None;
    }
    let mut types = Vec::new();
    while p < header {
        let (t, next) = varint(local, p)?;
        types.push(t);
        p = next;
        if types.len() > MAX_COLUMNS {
            return None;
        }
    }
    (p == header).then_some(types)
}

fn decode_record(payload: &[u8], utf16: bool) -> Result<Vec<Value>> {
    let types = record_types(payload).ok_or(Refusal::Corrupt)?;
    let (header, _) = varint(payload, 0).ok_or(Refusal::Corrupt)?;
    let mut at = header as usize;
    let mut out = Vec::with_capacity(types.len());
    for t in types {
        let size = match t {
            0 | 8 | 9 => 0,
            1 => 1,
            2 => 2,
            3 => 3,
            4 => 4,
            5 => 6,
            6 | 7 => 8,
            10 | 11 => return Err(Refusal::Corrupt),
            t => usize::try_from((t - 12) / 2).map_err(|_| Refusal::Corrupt)?,
        };
        let bytes = payload
            .get(at..at.checked_add(size).ok_or(Refusal::Corrupt)?)
            .ok_or(Refusal::Corrupt)?;
        at += size;
        out.push(match t {
            0 => Value::Null,
            8 => Value::Int(0),
            9 => Value::Int(1),
            1..=6 => {
                // 符号つきのビッグエンディアン
                let mut v: i64 = if bytes[0] & 0x80 != 0 { -1 } else { 0 };
                for b in bytes {
                    v = (v << 8) | *b as i64;
                }
                Value::Int(v)
            }
            7 => Value::Other,
            t if t % 2 == 0 => Value::Blob(bytes.to_vec()),
            _ => Value::Text(if utf16 {
                let units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .collect();
                String::from_utf16_lossy(&units)
            } else {
                String::from_utf8_lossy(bytes).into_owned()
            }),
        });
    }
    Ok(out)
}

// ---------------- 面（Offscreen） ----------------

struct Geometry {
    width: u32,
    height: u32,
    tiles_x: u32,
    tiles_y: u32,
}

/// 名前つきの区画（u32 BE の文字数・UTF-16BE の名前・中身）の中身。
fn section<'a>(attr: &'a [u8], at: usize, len: usize, name: &str) -> Option<&'a [u8]> {
    let body = attr.get(at..at.checked_add(len)?)?;
    let n = be32(body, 0)? as usize;
    if n != name.encode_utf16().count() {
        return None;
    }
    let end = 4usize.checked_add(n.checked_mul(2)?)?;
    (body.get(4..end)? == utf16be(name)).then(|| &body[end..])
}

fn words(bytes: &[u8]) -> Option<Vec<u32>> {
    bytes.len().is_multiple_of(4).then(|| {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_be_bytes(*c))
            .collect()
    })
}

/// 面の記述 `Attribute`: 16・3 つの区画の長さ・`Parameter`・`InitColor`・`BlockSize`。
fn geometry(attr: &[u8]) -> Result<Geometry> {
    let head = |i: usize| {
        be32(attr, i * 4)
            .map(|v| v as usize)
            .ok_or(Refusal::Attribute)
    };
    if head(0)? != 16 {
        return Err(Refusal::Attribute);
    }
    let (param_len, init_len, size_len) = (head(1)?, head(2)?, head(3)?);
    let total = 16usize
        .checked_add(param_len)
        .and_then(|v| v.checked_add(init_len))
        .and_then(|v| v.checked_add(size_len))
        .ok_or(Refusal::Attribute)?;
    if total != attr.len() {
        return Err(Refusal::Attribute);
    }
    let param = section(attr, 16, param_len, "Parameter")
        .and_then(words)
        .ok_or(Refusal::Attribute)?;
    let init = section(attr, 16 + param_len, init_len, "InitColor")
        .and_then(words)
        .ok_or(Refusal::Attribute)?;
    section(attr, 16 + param_len + init_len, size_len, "BlockSize").ok_or(Refusal::Attribute)?;
    if param.len() < 18 || init.len() < 5 {
        return Err(Refusal::Attribute);
    }
    let (width, height, tiles_x, tiles_y) = (param[0], param[1], param[2], param[3]);
    if width < 1 || height < 1 {
        return Err(Refusal::Attribute);
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(Refusal::TooLarge);
    }
    if tiles_x != width.div_ceil(TILE) || tiles_y != height.div_ceil(TILE) {
        return Err(Refusal::Attribute);
    }
    // 面の数は 7 番目（灰色 1・色 5）、タイルの大きさは 14・15 番目、ビット数は 16・17 番目。空のタイルの値（初期色）は 0 だけ
    if param[7] != 1
        || param[14] != TILE
        || param[15] != TILE
        || param[16] != 8
        || param[17] != 8
        || init[1..4] != [0, 0, 0]
    {
        return Err(Refusal::Unsupported);
    }
    Ok(Geometry {
        width,
        height,
        tiles_x,
        tiles_y,
    })
}

/// `BlockData`: タイルごとの塊（u32 BE の全長・`BlockDataBeginChunk`・u32×5・（画素があれば）u32 BE の長さと［u32 LE の zlib の長さ・zlib］・
/// `BlockDataEndChunk`）の並びと、その後ろの `BlockStatus` など。画素が 1 つも無ければ None。
fn read_tiles(data: &[u8], geo: &Geometry) -> Result<Option<Plane>> {
    let begin = {
        let mut v = (BEGIN_NAME.len() as u32).to_be_bytes().to_vec();
        v.extend(utf16be(BEGIN_NAME));
        v
    };
    let end = {
        let mut v = (END_NAME.len() as u32).to_be_bytes().to_vec();
        v.extend(utf16be(END_NAME));
        v
    };
    let tiles = (geo.tiles_x * geo.tiles_y) as usize;
    let mut values = vec![0u8; geo.width as usize * geo.height as usize];
    let mut done = vec![false; tiles];
    let mut count = 0usize;
    let mut any = false;
    let mut pos = 0usize;
    // 塊の並びは、BlockDataBeginChunk でなくなるところ（BlockStatus など）まで
    while data.get(pos + 4..pos + 4 + begin.len()) == Some(&begin[..]) {
        let total = be32(data, pos).ok_or(Refusal::BadTile)? as usize;
        let block = data
            .get(pos..pos.checked_add(total).ok_or(Refusal::BadTile)?)
            .ok_or(Refusal::BadTile)?;
        pos += total;
        count += 1;
        if count > tiles {
            return Err(Refusal::BadTile);
        }
        let mut p = 4 + begin.len();
        let field = |i: usize| be32(block, p + i * 4).ok_or(Refusal::BadTile);
        let (index, bytes, tw, th, has) = (field(0)?, field(1)?, field(2)?, field(3)?, field(4)?);
        p += 20;
        if index as usize >= tiles
            || std::mem::replace(&mut done[index as usize], true)
            || bytes as usize != TILE_BYTES
            || tw != TILE
            || th != TILE
            || has > 1
        {
            return Err(Refusal::BadTile);
        }
        if has == 1 {
            let stored = be32(block, p).ok_or(Refusal::BadTile)? as usize;
            let zlen = le32(block, p + 4).ok_or(Refusal::BadTile)? as usize;
            if stored != zlen.checked_add(4).ok_or(Refusal::BadTile)? {
                return Err(Refusal::BadTile);
            }
            let stream = block
                .get(p + 8..(p + 8).checked_add(zlen).ok_or(Refusal::BadTile)?)
                .ok_or(Refusal::BadTile)?;
            p += 4 + stored;
            let mut tile = Vec::with_capacity(TILE_BYTES);
            // 余りが 1 バイトでも出れば長すぎる
            ZlibDecoder::new(stream)
                .take(TILE_BYTES as u64 + 1)
                .read_to_end(&mut tile)
                .map_err(|_| Refusal::BadTile)?;
            if tile.len() != TILE_BYTES {
                return Err(Refusal::BadTile);
            }
            let (x0, y0) = (
                (index % geo.tiles_x) as usize * TILE as usize,
                (index / geo.tiles_x) as usize * TILE as usize,
            );
            let width = geo.width as usize;
            let copy = (geo.width as usize - x0).min(TILE as usize);
            for row in 0..(geo.height as usize - y0).min(TILE as usize) {
                let at = (y0 + row) * width + x0;
                values[at..at + copy].copy_from_slice(&tile[row * TILE as usize..][..copy]);
            }
            any = true;
        }
        // 塊の終わりの印で、塊の長さと過不足なく合う
        let tail = block.get(p..).ok_or(Refusal::BadTile)?;
        if tail != &end[..] {
            return Err(Refusal::BadTile);
        }
    }
    if count != tiles {
        return Err(Refusal::BadTile);
    }
    Ok(any.then_some(Plane {
        width: geo.width,
        height: geo.height,
        values,
    }))
}

// ---------------- 素材の画像 ----------------

/// 素材の `.layer`（C2F）から、素材の元の画像を読む。
///
/// 画像の面は、非フォルダのレイヤー（ちょうど 1 枚）の「元の画像のミップマップ」（なければ描画用のミップマップ）をたどる:
/// `Layer.ResizableOriginalMipmap`（`LayerRenderMipmap`）→ `Mipmap.BaseMipmapInfo` → `MipmapInfo.Offscreen` → `Offscreen`。
/// 参照は各表の `MainId`（行の番号ではない）。
pub(super) fn read(data: &[u8], work: &Work) -> Result<Plane> {
    let bodies = chunks(data)?;
    let db = Db::open(pages(&bodies)?, work)?;
    let layer_table = db.table("Layer")?;
    let layers = db.select(layer_table, |row| {
        let folder =
            layer_table.int(row, "layerfolder") != 0 || layer_table.int(row, "layertype") == 256;
        !folder
    })?;
    let [layer] = layers.as_slice() else {
        return Err(Refusal::Layers);
    };
    let mut starts = Vec::new();
    for name in ["resizableoriginalmipmap", "layerrendermipmap"] {
        let id = layer_table.int(layer, name);
        if id > 0 && !starts.contains(&id) {
            starts.push(id);
        }
    }
    if starts.is_empty() {
        return Err(Refusal::Layers);
    }
    let mipmaps = db.table("Mipmap")?;
    let infos = db.table("MipmapInfo")?;
    let offscreens = db.table("Offscreen")?;
    let one = |table: &Table, id: i64| -> Result<Option<Vec<Value>>> {
        let mut rows = db.select(table, |row| table.int(row, "mainid") == id)?;
        match rows.len() {
            0 => Ok(None),
            1 => Ok(rows.pop()),
            _ => Err(Refusal::Ambiguous),
        }
    };
    for start in starts {
        let Some(mipmap) = one(mipmaps, start)? else {
            continue;
        };
        let Some(info) = one(infos, mipmaps.int(&mipmap, "basemipmapinfo"))? else {
            continue;
        };
        let Some(offscreen) = one(offscreens, infos.int(&info, "offscreen"))? else {
            continue;
        };
        let attribute = offscreens
            .blob(&offscreen, "attribute")
            .ok_or(Refusal::Attribute)?;
        let block = offscreens
            .blob(&offscreen, "blockdata")
            .ok_or(Refusal::BadTile)?;
        let geo = geometry(attribute)?;
        // 面を確保する前に、画素の数を取り込み全体の分から引く（画素の無い面でも引く。確保の前に断るため）
        Work::take(&work.pixels, geo.width as u64 * geo.height as u64)?;
        if let Some(plane) = read_tiles(block, &geo)? {
            return Ok(plane);
        }
    }
    Err(Refusal::Empty)
}

/// 試験用の C2F の組み立て（統合試験 `tests/brushes/brush_import_sut_layer.rs` と同じもの）。
#[cfg(test)]
#[allow(dead_code)]
#[path = "../../../tests/brush_files/c2f.rs"]
mod fixture;

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = (body.len() as u32).to_le_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        let mut crc = crc32fast::Hasher::new();
        crc.update(kind);
        crc.update(body);
        out.extend_from_slice(&crc.finalize().to_le_bytes());
        out
    }

    fn container(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut out = SIGNATURE.to_vec();
        for c in chunks {
            out.extend_from_slice(c);
        }
        out
    }

    #[test]
    fn the_container_is_checked_chunk_by_chunk() {
        let ok = container(&[
            chunk(b"HEAD", b""),
            chunk(b"dATA", b"\x01\x00abc"),
            chunk(b"dATA", b"\x00\x00def"),
            chunk(b"TAIL", b""),
        ]);
        let bodies = chunks(&ok).unwrap();
        assert_eq!(bodies, [&b"\x01\x00abc"[..], &b"\x00\x00def"[..]]);
        // 印
        assert_eq!(chunks(b"\x89PNG\r\n\x1a\n").err(), Some(Refusal::NotC2f));
        assert_eq!(chunks(&[]).err(), Some(Refusal::NotC2f));
        // 切れている（長さが残りを超える・CRC が足りない・TAIL が無い）
        let cut = &ok[..ok.len() - 3];
        assert_eq!(chunks(cut).err(), Some(Refusal::Truncated));
        let no_tail = container(&[chunk(b"HEAD", b""), chunk(b"dATA", b"\x00\x00x")]);
        assert_eq!(chunks(&no_tail).err(), Some(Refusal::Truncated));
        let mut huge = container(&[chunk(b"HEAD", b"")]);
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        huge.extend_from_slice(b"dATA");
        assert_eq!(chunks(&huge).err(), Some(Refusal::Truncated));
        // CRC
        let mut bad = ok.clone();
        let at = SIGNATURE.len() + 12 + 8 + 1;
        bad[at] ^= 1;
        assert_eq!(chunks(&bad).err(), Some(Refusal::BadCrc));
        // 知らない塊・HEAD の位置
        let unknown = container(&[
            chunk(b"HEAD", b""),
            chunk(b"zzzz", b"?"),
            chunk(b"dATA", b"\x00\x00x"),
            chunk(b"TAIL", b""),
        ]);
        assert_eq!(chunks(&unknown).err(), Some(Refusal::UnknownChunk));
        let headless = container(&[chunk(b"dATA", b"\x00\x00x"), chunk(b"TAIL", b"")]);
        assert_eq!(chunks(&headless).err(), Some(Refusal::UnknownChunk));
        let tail_first = container(&[chunk(b"TAIL", b""), chunk(b"HEAD", b"")]);
        assert_eq!(chunks(&tail_first).err(), Some(Refusal::UnknownChunk));
        // 塊が多すぎる
        let mut many = vec![chunk(b"HEAD", b"")];
        many.extend((0..MAX_CHUNKS).map(|_| chunk(b"dATA", b"\x00\x00x")));
        assert_eq!(
            chunks(&container(&many)).err(),
            Some(Refusal::TooManyChunks)
        );
        // TAIL の後ろのバイトは読まない
        let mut trailing = ok.clone();
        trailing.extend_from_slice(b"garbage after the tail");
        assert!(chunks(&trailing).is_ok());
    }

    #[test]
    fn pages_are_counted_from_the_chunks_and_the_unreadable_ones_keep_their_places() {
        let page = vec![7u8; PAGE];
        let mut plain = vec![0u8, 0];
        plain.extend(&page);
        plain.extend(&page);
        let mut hidden = vec![1u8, 0];
        hidden.extend(vec![9u8; 3 * PAGE + 8]);
        let all = pages(&[&hidden, &plain]).unwrap();
        assert_eq!(all.len(), 5);
        assert!(all[..3].iter().all(Option::is_none));
        assert!(all[3..].iter().all(|p| p.is_some_and(|p| p == &page[..])));
        // 読めない側が 1 ページに満たない・読める側が 1024 の倍数でない・空
        assert!(pages(&[&[1, 0, 1, 2, 3]]).is_err());
        assert!(pages(&[&plain[..plain.len() - 1]]).is_err());
        assert!(pages(&[&[0, 0]]).is_err());
        assert!(pages(&[&[0]]).is_err());
    }

    #[test]
    fn varints_and_records_decode_with_signs_and_every_serial_type() {
        assert_eq!(varint(&[0x7f], 0), Some((127, 1)));
        assert_eq!(varint(&[0x81, 0x00], 0), Some((128, 2)));
        assert_eq!(varint(&[0xff; 9], 0), Some((u64::MAX, 9)));
        assert_eq!(varint(&[0x81], 0), None, "途中で切れている");
        // 型 0（NULL）・1（i8 = -2）・2（i16 = 200）・8・9（定数 0・1）・7（浮動小数点）・13+2n（文字）・12+2n（BLOB）
        let mut record = vec![9u8, 0, 1, 2, 8, 9, 7, 19, 16];
        record.extend([0xfe]);
        record.extend(200i16.to_be_bytes());
        record.extend(1.5f64.to_be_bytes());
        record.extend(b"abc");
        record.extend([1, 2]);
        let values = decode_record(&record, false).unwrap();
        assert!(matches!(values[0], Value::Null));
        assert_eq!(values[1].int(), -2);
        assert_eq!(values[2].int(), 200);
        assert_eq!((values[3].int(), values[4].int()), (0, 1));
        assert!(matches!(values[5], Value::Other));
        assert!(matches!(&values[6], Value::Text(t) if t == "abc"));
        assert!(matches!(&values[7], Value::Blob(b) if b == &[1, 2]));
        // 予約された型・本体が足りない・ヘッダーが本体より長い
        assert_eq!(decode_record(&[2, 10], false).err(), Some(Refusal::Corrupt));
        assert_eq!(decode_record(&[2, 1], false).err(), Some(Refusal::Corrupt));
        assert_eq!(decode_record(&[9, 1], false).err(), Some(Refusal::Corrupt));
        // UTF-16LE
        let text = decode_record(&[2, 21, b'h', 0, b'i', 0], true).unwrap();
        assert!(matches!(&text[0], Value::Text(t) if t == "hi"));
    }

    #[test]
    fn column_names_come_from_the_create_statement_without_constraints() {
        let sql = "CREATE TABLE Layer(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, \"MainId\" INTEGER DEFAULT NULL, [LayerName] TEXT DEFAULT (1+2), PRIMARY KEY(_PW_ID, MainId), UNIQUE(MainId))";
        assert_eq!(columns_of(sql), ["_pw_id", "mainid", "layername"]);
        assert!(columns_of("CREATE TABLE x").is_empty());
        assert!(columns_of(")(").is_empty());
    }
}

/// 信頼できない入力としての上限と、壊れ方ごとの断り（理由まで確かめる）。ファイルの組み立ては統合試験と共通。
#[cfg(test)]
mod limits {
    use super::fixture::{self, Layer, RawBlock};
    use super::*;

    const MIB: usize = 1024 * 1024;

    fn refusal(layer: &Layer) -> Option<Refusal> {
        read(&layer.c2f(), &Work::new()).err()
    }

    /// 対照: 壊す前（同じ組み立て）は読める。
    fn reads(layer: &Layer) {
        let plane = read(&layer.c2f(), &Work::new()).unwrap_or_else(|r| panic!("読めない: {r:?}"));
        assert_eq!((plane.width, plane.height), (layer.width, layer.height));
        assert_eq!(plane.values, layer.values);
    }

    #[test]
    fn a_tree_one_level_deeper_than_the_limit_is_refused() {
        // 葉が深さ 8（内部ページが 8 段）までは読める。9 段になると断る
        let mut layer = Layer::new(40, 30);
        layer.extra_depth = MAX_DEPTH - 1;
        reads(&layer);
        layer.extra_depth = MAX_DEPTH;
        assert_eq!(refusal(&layer), Some(Refusal::Corrupt));
        layer.extra_depth = 40;
        assert_eq!(refusal(&layer), Some(Refusal::Corrupt));
    }

    #[test]
    fn a_row_that_claims_more_than_the_row_limit_is_refused_without_building_it() {
        // 上限は 64 MiB（行の先頭の分を足して超えるものを断る）
        let mut layer = Layer::new(40, 30);
        layer.claims = vec![60 * MIB];
        reads(&layer);
        // 小さなファイルが上限を超える大きさを名乗る。組み立てる前に断る
        layer.claims = vec![64 * MIB];
        let file = layer.c2f();
        assert!(file.len() < MIB, "ファイルは小さいまま");
        assert_eq!(refusal(&layer), Some(Refusal::Corrupt));
        layer.claims = vec![usize::MAX / 4];
        assert_eq!(refusal(&layer), Some(Refusal::Corrupt));
    }

    #[test]
    fn rows_that_add_up_beyond_the_work_limit_are_refused() {
        // 1 行は上限の内（56 MiB）でも、合計が 256 MiB を超える
        let mut layer = Layer::new(40, 30);
        layer.claims = vec![56 * MIB, 56 * MIB];
        reads(&layer);
        layer.claims = vec![56 * MIB; 5];
        assert_eq!(refusal(&layer), Some(Refusal::OverBudget));
    }

    #[test]
    fn the_work_is_shared_by_every_material_of_one_import() {
        // 1 つなら読める C2F（合計 112 MiB）でも、同じ取り込みの 3 つ目は残りが足りない
        let mut layer = Layer::new(40, 30);
        layer.claims = vec![56 * MIB, 56 * MIB];
        let file = layer.c2f();
        let work = Work::new();
        assert!(read(&file, &work).is_ok());
        assert!(read(&file, &work).is_ok());
        assert_eq!(read(&file, &work).err(), Some(Refusal::OverBudget));
        assert!(
            read(&file, &Work::new()).is_ok(),
            "新しい取り込みは新しい予算"
        );
    }

    #[test]
    fn pixels_are_charged_for_the_whole_import_before_the_face_is_allocated() {
        // 2048×2048 を名乗って画素が 1 つも無い素材を 256 個並べても、確保・展開するのは予算の分だけ。画素が無くても引く
        let mut layer = Layer::new(2048, 2048);
        layer.all_empty = true;
        let file = layer.c2f();
        assert!(file.len() < MIB);
        let work = Work::new();
        let (mut empty, mut over) = (0, 0);
        for _ in 0..256 {
            match read(&file, &work).err() {
                Some(Refusal::Empty) => empty += 1,
                Some(Refusal::OverBudget) => over += 1,
                other => panic!("{other:?}"),
            }
        }
        // 元の面が空だと描画用の面も見るので、1 つの素材で 2 面ぶん引く
        assert_eq!(empty, 32, "256 Mi 画素 ÷ (2 面 × 2048²)");
        assert_eq!(empty + over, 256);
        // 画素のある面でも同じ。上限を小さくして、ちょうど入る数だけ読める
        let layer = Layer::new(300, 270);
        let file = layer.c2f();
        let work = Work::with_limits(MAX_MATERIALIZED, 2 * 300 * 270);
        assert!(read(&file, &work).is_ok());
        assert!(read(&file, &work).is_ok());
        assert_eq!(read(&file, &work).err(), Some(Refusal::OverBudget));
        assert_eq!(work.pixels.get(), 0);
    }

    #[test]
    fn a_table_defined_twice_with_different_roots_is_ambiguous() {
        let mut layer = Layer::new(40, 30);
        layer.second_layer_table = true;
        assert_eq!(refusal(&layer), Some(Refusal::Ambiguous));
        // 同じ行が別のページに写って残っているだけ（根が同じ）なら、同じ表
        let mut copy = Layer::new(40, 30);
        copy.repeated_layer_table = true;
        reads(&copy);
    }

    #[test]
    fn utf8_and_utf16_table_definitions_cannot_be_mixed() {
        for utf16 in [false, true] {
            let mut layer = Layer::new(40, 30);
            layer.utf16 = utf16;
            reads(&layer);
            layer.mixed_text = true;
            assert_eq!(refusal(&layer), Some(Refusal::Corrupt), "utf16={utf16}");
        }
    }

    /// 元の面の `BlockData` を、タイルの塊の並び（300×20 は横 2 枚）に差し替える。
    fn with_blocks(blocks: &[Vec<u8>]) -> Layer {
        let mut layer = Layer::new(300, 20);
        layer.blocks = Some(fixture::assemble_blocks(blocks));
        layer
    }

    fn tile_pair() -> (Vec<u8>, Vec<u8>) {
        let layer = Layer::new(300, 20);
        let tiles = fixture::tiles_of(300, 20, &layer.values);
        (tiles[0].clone().unwrap(), tiles[1].clone().unwrap())
    }

    #[test]
    fn tile_blocks_that_do_not_add_up_are_refused() {
        let (t0, t1) = tile_pair();
        let good = |i: u32| RawBlock::new(i, Some(if i == 0 { &t0 } else { &t1 }));
        let bad = |layer: Layer| assert_eq!(refusal(&layer), Some(Refusal::BadTile));
        // 対照: 正しい並びは読める（同じ画素）
        let mut control = with_blocks(&[good(0).build(), good(1).build()]);
        let plane = read(&control.c2f(), &Work::new()).unwrap();
        assert_eq!(plane.values, control.values);
        control.blocks = None;
        reads(&control);

        // 番号が重なる・数が足りない・多い・番号が範囲の外
        bad(with_blocks(&[good(0).build(), good(0).build()]));
        bad(with_blocks(&[good(1).build(), good(1).build()]));
        bad(with_blocks(&[good(0).build()]));
        bad(with_blocks(&[
            good(0).build(),
            good(1).build(),
            good(1).build(),
        ]));
        bad(with_blocks(&[good(0).build(), good(5).build()]));
        bad(with_blocks(&[good(0).build(), good(u32::MAX).build()]));
        bad(with_blocks(&[]));
        // タイルの大きさの欄が 65536・256・256 でない・画素の有無の欄が 0 でも 1 でもない
        for edit in [
            |b: &mut RawBlock| b.bytes = 65535,
            |b: &mut RawBlock| b.bytes = 0,
            |b: &mut RawBlock| b.tile_w = 128,
            |b: &mut RawBlock| b.tile_h = 255,
            |b: &mut RawBlock| b.has = 2,
            |b: &mut RawBlock| {
                b.has = 2;
                b.stream = None;
            },
        ] {
            let mut b = good(1);
            edit(&mut b);
            bad(with_blocks(&[good(0).build(), b.build()]));
        }
        // 長さの欄が zlib の長さ + 4 でない
        for delta in [-4i64, -1, 1, 4] {
            let mut b = good(1);
            b.stored_delta = delta;
            bad(with_blocks(&[good(0).build(), b.build()]));
        }
        // zlib が展開すると 65536 バイトに足りない・余る・壊れている
        for len in [0usize, 65535, 65537, 70000] {
            let mut b = good(1);
            b.stream = Some(fixture::zlib(&vec![7u8; len]));
            bad(with_blocks(&[good(0).build(), b.build()]));
        }
        let mut b = good(1);
        let stream = b.stream.as_mut().unwrap();
        let mid = stream.len() / 2;
        stream[mid] ^= 0xff;
        bad(with_blocks(&[good(0).build(), b.build()]));
        // 塊の終わりの印が違う・後ろに余りがある
        let mut b = good(1);
        b.end = b"x".to_vec();
        bad(with_blocks(&[good(0).build(), b.build()]));
        let mut b = good(1);
        b.end.extend([0, 0, 0, 0]);
        bad(with_blocks(&[good(0).build(), b.build()]));
        // 塊の全長の欄がデータより長い・短い
        for total in [u32::MAX, 10 * 1024 * 1024, 8, 0] {
            let mut data = good(1).build();
            data[..4].copy_from_slice(&total.to_be_bytes());
            bad(with_blocks(&[good(0).build(), data]));
        }
    }

    #[test]
    fn faces_that_are_not_what_the_reader_knows_are_refused_with_their_reasons() {
        let mut layer = Layer::new(30, 30);
        layer.tile_param = 128;
        assert_eq!(refusal(&layer), Some(Refusal::Unsupported));
        let mut layer = Layer::new(30, 30);
        layer.planes = 5;
        assert_eq!(refusal(&layer), Some(Refusal::Unsupported));
        let mut layer = Layer::new(30, 30);
        layer.width = 2049;
        layer.height = 1;
        layer.values = vec![1; 2049];
        assert_eq!(refusal(&layer), Some(Refusal::TooLarge));
        let mut layer = Layer::new(30, 30);
        layer.all_empty = true;
        assert_eq!(refusal(&layer), Some(Refusal::Empty));
        for count in [0, 2] {
            let mut layer = Layer::new(30, 30);
            layer.layers = count;
            assert_eq!(refusal(&layer), Some(Refusal::Layers), "{count}");
        }
    }
}

/// 本物の `.sut`（試験の外。環境変数 `YOLU_REAL_SUT` のパス）で、読んだ画素の意味を確かめる。素材の中には CLIP STUDIO 自身が作った見本
/// （`MaterialThumbnail`。300×300 の PNG）があるので、それと突き合わせる: 見本は「黒のインクに不透明度」の絵で、読んだ画素 v（0 が空）が
/// その不透明度なら、見本の不透明度の画素ごとの並びと一致する。向き（上下・左右）と値の向き（v か 255 − v か）も、これで確かめる。
/// 見本が無い素材（`CanvasPreview` の素材）では確かめられない。ファイルも画像もリポジトリに入れず、名前や画像は表示しない。
#[cfg(test)]
mod real {
    use super::*;

    /// 見本（RGBA）から、不透明度と、不透明な所の色の平均（黒なら 0）。
    fn thumbnail(png_bytes: &[u8]) -> (u32, u32, Vec<u8>, f64) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes))
            .read_info()
            .expect("見本の PNG");
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).expect("見本の PNG");
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        let pixels = &buf[..info.buffer_size()];
        let alpha: Vec<u8> = pixels.chunks(4).map(|px| px[3]).collect();
        let (mut sum, mut weight) = (0.0, 0.0);
        for px in pixels.chunks(4) {
            sum += px[0] as f64 * px[3] as f64;
            weight += px[3] as f64;
        }
        (info.width, info.height, alpha, sum / weight.max(1.0))
    }

    /// 面を `w`×`h` に面積平均で縮める。
    fn shrink(plane: &Plane, w: u32, h: u32, flip: (bool, bool), invert: bool) -> Vec<f64> {
        let mut out = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            let y = if flip.1 { h - 1 - y } else { y };
            let (y0, y1) = (
                y as u64 * plane.height as u64 / h as u64,
                ((y as u64 + 1) * plane.height as u64).div_ceil(h as u64),
            );
            for x in 0..w {
                let x = if flip.0 { w - 1 - x } else { x };
                let (x0, x1) = (
                    x as u64 * plane.width as u64 / w as u64,
                    ((x as u64 + 1) * plane.width as u64).div_ceil(w as u64),
                );
                let (mut sum, mut n) = (0.0, 0.0);
                for sy in y0..y1.min(plane.height as u64) {
                    for sx in x0..x1.min(plane.width as u64) {
                        let v = plane.values[(sy * plane.width as u64 + sx) as usize] as f64;
                        sum += if invert { 255.0 - v } else { v };
                        n += 1.0;
                    }
                }
                out.push(sum / n);
            }
        }
        out
    }

    fn correlation(a: &[f64], b: &[f64]) -> f64 {
        let n = a.len() as f64;
        let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
        let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
        for (x, y) in a.iter().zip(b) {
            cov += (x - ma) * (y - mb);
            va += (x - ma) * (x - ma);
            vb += (y - mb) * (y - mb);
        }
        cov / (va.sqrt() * vb.sqrt()).max(1e-12)
    }

    #[test]
    #[ignore = "本物の .sut が要る（YOLU_REAL_SUT）"]
    fn the_pixels_are_the_black_ink_opacity_the_material_thumbnail_shows() {
        let path = std::env::var("YOLU_REAL_SUT").expect("YOLU_REAL_SUT に .sut のパスを入れる");
        let bytes = std::fs::read(path).expect("読める");
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.deserialize_read_exact("main", std::io::Cursor::new(&bytes[..]), bytes.len(), true)
            .expect("SQLite");
        let mut stmt = conn
            .prepare("SELECT FileData FROM MaterialFile ORDER BY _PW_ID")
            .unwrap();
        let blobs: Vec<Vec<u8>> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let mut checked = 0;
        for blob in &blobs {
            for entry in crate::brushes::sut::tar::entries(blob)
                .iter()
                .filter(|e| e.name.ends_with(".layer"))
            {
                let work = Work::new();
                let plane = read(entry.data, &work).expect("素材の画像が読める");
                let db = Db::open(pages(&chunks(entry.data).unwrap()).unwrap(), &work).unwrap();
                let Some(table) = db.tables.get("materialthumbnail").map(|l| &l[0]) else {
                    println!(
                        "見本の無い素材（{}×{}）は確かめられない",
                        plane.width, plane.height
                    );
                    continue;
                };
                let rows = db.select(table, |_| true).unwrap();
                let Some(Value::Blob(png_bytes)) = table
                    .index("thumbnailimage")
                    .and_then(|i| rows.first()?.get(i))
                else {
                    panic!("見本の画像の列が無い");
                };
                let (w, h, alpha, ink) = thumbnail(png_bytes);
                let alpha: Vec<f64> = alpha.into_iter().map(f64::from).collect();
                let against =
                    |flip, invert| correlation(&shrink(&plane, w, h, flip, invert), &alpha);
                let direct = against((false, false), false);
                let upside_down = against((false, true), false);
                let mirrored = against((true, false), false);
                let inverted = against((false, false), true);
                let mean_plane =
                    plane.values.iter().map(|&v| v as f64).sum::<f64>() / plane.values.len() as f64;
                let mean_alpha = alpha.iter().sum::<f64>() / alpha.len() as f64;
                println!(
                    "素材 {}×{}: 見本との相関 {direct:.4}（上下を逆にすると {upside_down:.4}、左右を逆にすると {mirrored:.4}、255 − v だと {inverted:.4}）、平均 {mean_plane:.2} と {mean_alpha:.2}、見本のインクの色 {ink:.1}",
                    plane.width, plane.height
                );
                assert!(
                    direct > 0.5,
                    "画素が見本の不透明度と並びで合わない: {direct}"
                );
                assert!(
                    direct > upside_down && direct > mirrored,
                    "向きを取り違えたほうが合う: {direct} と {upside_down}・{mirrored}"
                );
                assert!(inverted < 0.0, "255 − v の向きは逆の相関になる: {inverted}");
                assert!(
                    (mean_plane - mean_alpha).abs() < 2.0,
                    "平均が合わない: {mean_plane} と {mean_alpha}"
                );
                assert!(ink < 8.0, "見本のインクが黒でない: {ink}");
                checked += 1;
            }
        }
        assert!(checked > 0, "見本のある素材が 1 つも無い");
    }
}
