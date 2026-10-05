//! `.sut`（SQLite）を、信頼しない入力として読み取り専用で開き、決まった表だけを決まった問い合わせで読む。
//!
//! - ファイルは一時の場所へ写さず、ファイルへの接続も作らない。メモリ上の読み取り専用のデータベースとして開く
//!   （`sqlite3_deserialize`。ジャーナルも WAL も作られず、元のファイルは開いたままにしない）。
//! - 拡張の読み込みは使わない。SQLite の本体には拡張を読み込む機能がコンパイルされている（`libsqlite3-sys` が有効にする）が、SQL の
//!   `load_extension()` は、有効化の関数を呼ばない限り「not authorized」で断られる。ここは有効化の関数を呼ばない（呼ぶ道具も
//!   `rusqlite` の機能として入れていない）。安全はこの「呼ばない」ことに依るので、試験（`load_extension_is_refused_from_sql`）で見張る。
//!   さらに防御モード・信頼しないスキーマ・トリガーとビューの無効・ATTACH の禁止・文字列と行の長さ・式の深さ・列数の上限を置き、
//!   問い合わせの実行は SQLite の命令の数と経過時間で打ち切る（壊れた・悪意のあるスキーマが、生成列・再帰の問い合わせで止まらなく
//!   なるのを防ぐ）。命令の数は接続全体の累積（合計の上限）、時間は SQL の中で使った時間の合計（PNG の復号など SQL の外の時間は
//!   数えない）。
//! - `Variant` は、読みたい列（`sut.rs` の `KNOWN`。名前はこちらの定数）のうち表に在る通常の列だけを選ぶ。生成列を含めた
//!   `SELECT *` にしない（重い式を持つ生成列を、読まない列まで評価させない）。`MaterialFile` は画像の列と先頭の数列だけ。
//!   ファイルから来た列の名前を SQL に入れるのは `MaterialFile` の数列だけで、引用符を二重にして識別子にする。
//! - 行を読む数と、1 つの値の大きさを制限する（大きな BLOB は大きさだけ知らせて中身を読まない）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::config::DbConfig;
use rusqlite::limits::Limit;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, Statement};

use super::super::error::Fault;
use super::super::short_text;
use super::{c2f, material};

/// SQLite のファイルの先頭の署名。
const SIGNATURE: &[u8] = b"SQLite format 3\0";
/// 署名が先頭にないとき、署名を探す範囲（バイト）。
const SEARCH: usize = 4096;

/// 命令を数える単位（1 万命令ごとに確かめる）と、打ち切る回数（接続全体の累積で合計 5000 万命令。実物の読みは数百万命令以下）と、
/// 打ち切る時間（SQL の中で使った時間の合計）。1 つの関数の呼び出し（巨大な文字列の組み立てなど）の途中では止まらないので、
/// 時間でも切る。
const PROGRESS_OPS: i32 = 10_000;
const PROGRESS_CALLS: u32 = 5_000;
const TIME_LIMIT: Duration = Duration::from_secs(30);

/// 読み取りの上限（命令を確かめた回数・SQL の中の時間）。試験で小さくして踏む。
#[derive(Clone, Copy, Debug)]
pub(super) struct Limits {
    pub calls: u32,
    pub time: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            calls: PROGRESS_CALLS,
            time: TIME_LIMIT,
        }
    }
}

/// 上限の数え方。SQLite の進捗の呼び出し（接続の中）と、問い合わせの出入り（`Scope`）から更新する。
struct Meter {
    limits: Limits,
    base: Instant,
    /// 命令を確かめた回数（累積）。
    calls: AtomicU32,
    /// 終わった問い合わせで使った時間（ナノ秒）。
    spent: AtomicU64,
    /// 今の問い合わせの始まり（`base` からのナノ秒。問い合わせの外では `IDLE`）と、入れ子の深さ。
    since: AtomicU64,
    depth: AtomicU32,
}

const IDLE: u64 = u64::MAX;

impl Meter {
    fn new(limits: Limits) -> Meter {
        Meter {
            limits,
            base: Instant::now(),
            calls: AtomicU32::new(0),
            spent: AtomicU64::new(0),
            since: AtomicU64::new(IDLE),
            depth: AtomicU32::new(0),
        }
    }

    fn now(&self) -> u64 {
        self.base.elapsed().as_nanos().min(u64::MAX as u128 - 1) as u64
    }

    /// 進捗の呼び出しから: 打ち切るか。
    fn over(&self) -> bool {
        let calls = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
        let since = self.since.load(Ordering::Relaxed);
        let running = if since == IDLE {
            0
        } else {
            self.now().saturating_sub(since)
        };
        let spent = self.spent.load(Ordering::Relaxed).saturating_add(running);
        calls > self.limits.calls || u128::from(spent) > self.limits.time.as_nanos()
    }

    /// 問い合わせの間、時間を数える（入れ子は最も外側の出入りだけ）。
    fn scope(&self) -> Scope<'_> {
        if self.depth.fetch_add(1, Ordering::Relaxed) == 0 {
            self.since.store(self.now(), Ordering::Relaxed);
        }
        Scope(self)
    }
}

struct Scope<'a>(&'a Meter);

impl Drop for Scope<'_> {
    fn drop(&mut self) {
        let meter = self.0;
        if meter.depth.fetch_sub(1, Ordering::Relaxed) == 1 {
            let since = meter.since.swap(IDLE, Ordering::Relaxed);
            if since != IDLE {
                meter
                    .spent
                    .fetch_add(meter.now().saturating_sub(since), Ordering::Relaxed);
            }
        }
    }
}

/// スキーマの項目（表・索引・ビューなど）の数の上限。実物の .sut は十数個。
const MAX_SCHEMA_OBJECTS: i64 = 1024;

/// 読むブラシの数・見る素材の数（`Node` の行は全部を見て、読まなかったブラシの数を数える。作業は命令の数と時間の上限が縛る）。
pub(super) const MAX_BRUSHES: usize = 256;
pub(super) const MAX_MATERIALS: usize = 256;

/// 列 1 つの値の大きさの上限（`Variant`）。これを超える BLOB と文字列は中身を読まない。
const MAX_CELL_BYTES: usize = 256 * 1024;
const MAX_TEXT_CHARS: usize = 256;
/// 読む列の数の上限（`Variant`・`MaterialFile`）。
const MAX_COLUMNS: usize = 4096;

/// 読んだ値 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Cell {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
    /// 大きすぎて読まなかった BLOB・文字列。
    Oversized,
}

impl Cell {
    fn read(value: ValueRef<'_>) -> Cell {
        match value {
            ValueRef::Null => Cell::Null,
            ValueRef::Integer(i) => Cell::Int(i),
            ValueRef::Real(r) => Cell::Real(r),
            ValueRef::Text(t) => {
                if t.len() > MAX_CELL_BYTES {
                    Cell::Oversized
                } else {
                    Cell::Text(short_text(&String::from_utf8_lossy(t), MAX_TEXT_CHARS))
                }
            }
            ValueRef::Blob(b) => {
                if b.len() > MAX_CELL_BYTES {
                    Cell::Oversized
                } else {
                    Cell::Blob(b.to_vec())
                }
            }
        }
    }

    /// 数として読めるなら数（有限のものだけ）。真偽は 0 / 0 以外。
    pub fn number(&self) -> Option<f64> {
        match self {
            Cell::Int(i) => Some(*i as f64),
            Cell::Real(r) if r.is_finite() => Some(*r),
            _ => None,
        }
    }
}

/// `Variant` の 1 行（列の名前は小文字）。
#[derive(Debug, Default)]
pub(super) struct Row {
    cells: HashMap<String, Cell>,
}

impl Row {
    pub fn get(&self, name: &str) -> Option<&Cell> {
        self.cells.get(name)
    }
    pub fn number(&self, name: &str) -> Option<f64> {
        self.get(name).and_then(Cell::number)
    }
    pub fn first_number(&self, names: &[&str]) -> Option<f64> {
        names.iter().find_map(|n| self.number(n))
    }
    pub fn first_blob(&self, names: &[&str]) -> Option<&[u8]> {
        names.iter().find_map(|n| match self.get(n) {
            Some(Cell::Blob(b)) if !b.is_empty() => Some(b.as_slice()),
            _ => None,
        })
    }
    /// 大きすぎて読まなかった値がある列か。
    pub fn is_oversized(&self, names: &[&str]) -> bool {
        names
            .iter()
            .any(|n| matches!(self.get(n), Some(Cell::Oversized)))
    }
    /// 0 でない数（旗が立っている）。
    pub fn on(&self, names: &[&str]) -> bool {
        self.first_number(names).is_some_and(|v| v != 0.0)
    }
}

#[cfg(test)]
impl Row {
    /// 試験用: 列を 1 つ足した行。
    pub fn with(mut self, name: &str, cell: Cell) -> Row {
        self.cells.insert(name.to_string(), cell);
        self
    }
}

/// `Node` の 1 行。
#[derive(Debug)]
pub(super) struct Node {
    pub name: String,
    /// 現在の設定の `Variant` の番号と、既定の設定の番号（0 は無い）。
    pub variant: i64,
    pub init_variant: i64,
}

/// `MaterialFile` の 1 行から取り出したもの。
pub(super) struct Material {
    /// 行の順（`_PW_ID` が数ならその値）。
    pub order: i64,
    /// 参照と突き合わせるための文字列（行の文字列の列。小文字・場所と拡張子を落とす前のもの）。
    pub texts: Vec<String>,
    pub image: Option<material::Image>,
    /// 画像が CLIP STUDIO 独自の入れ物にだけあって読めない（使える PNG が無い）。
    pub proprietary: bool,
    /// 筆先の素材か質感の素材か（素材の中の情報から分かるときだけ）。
    pub kind: Option<material::Kind>,
}

pub(super) struct Database {
    conn: Connection,
    meter: Arc<Meter>,
}

/// 署名の位置（先頭か、先頭の少し後ろ）。
fn locate(bytes: &[u8]) -> Option<usize> {
    let window = &bytes[..bytes.len().min(SEARCH + SIGNATURE.len())];
    window.windows(SIGNATURE.len()).position(|w| w == SIGNATURE)
}

fn limits_or_not_database(error: &rusqlite::Error) -> Fault {
    match error {
        rusqlite::Error::SqliteFailure(e, _)
            if e.code == rusqlite::ErrorCode::OperationInterrupted =>
        {
            Fault::SutLimits
        }
        _ => Fault::SutNotDatabase,
    }
}

impl Database {
    /// メモリ上のファイルの中身から開く。SQLite でなければ `SutNotDatabase`。
    pub fn open(bytes: &[u8]) -> Result<Database, Fault> {
        Database::open_with(bytes, Limits::default())
    }

    /// `open`（読み取りの上限を指定する）。
    pub fn open_with(bytes: &[u8], limits: Limits) -> Result<Database, Fault> {
        let start = locate(bytes).ok_or(Fault::SutNotDatabase)?;
        let data = &bytes[start..];
        let mut conn = Connection::open_in_memory().map_err(|_| Fault::SutNotDatabase)?;
        let meter = Arc::new(Meter::new(limits));
        let configured = (|| -> rusqlite::Result<()> {
            // 文字列・BLOB・行の大きさ、SQL の長さ、式の深さ、列、命令数、複合の SELECT、関数の引数、ATTACH、LIKE の長さ、変数、トリガーの深さ
            for (limit, value) in [
                (Limit::SQLITE_LIMIT_LENGTH, 64 * 1024 * 1024),
                // 表の定義（スキーマの CREATE 文）もこの長さで読むので、数百列の `Variant`（数十 KB）が入る大きさにする
                (Limit::SQLITE_LIMIT_SQL_LENGTH, 4 * 1024 * 1024),
                (Limit::SQLITE_LIMIT_COLUMN, MAX_COLUMNS as i32),
                (Limit::SQLITE_LIMIT_EXPR_DEPTH, 64),
                (Limit::SQLITE_LIMIT_COMPOUND_SELECT, 4),
                (Limit::SQLITE_LIMIT_VDBE_OP, 100_000),
                (Limit::SQLITE_LIMIT_FUNCTION_ARG, 8),
                (Limit::SQLITE_LIMIT_ATTACHED, 0),
                (Limit::SQLITE_LIMIT_LIKE_PATTERN_LENGTH, 64),
                (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 8),
                (Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 1),
            ] {
                conn.set_limit(limit, value)?;
            }
            conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
            conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
            conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)?;
            conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false)?;
            conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, false)?;
            conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER, false)?;
            let counting = Arc::clone(&meter);
            conn.progress_handler(PROGRESS_OPS, Some(move || counting.over()))?;
            Ok(())
        })();
        configured.map_err(|_| Fault::SutNotDatabase)?;
        // 先頭 100 バイトのヘッダーだけ写して直す: 書き込みの記録（WAL）の形式を示す印（18・19 バイト目）が 2 のファイルは、メモリから
        // 開けない。ファイルはチェックポイントの済んだ 1 つのファイルなので、ふつうのファイルの形式（1）として読む。
        let split = data.len().min(100);
        let mut head = data[..split].to_vec();
        if head.len() == 100 && head[18] == 2 && head[19] == 2 {
            head[18] = 1;
            head[19] = 1;
        }
        let source = std::io::Read::chain(std::io::Cursor::new(head), &data[split..]);
        conn.deserialize_read_exact("main", source, data.len(), true)
            .map_err(|_| Fault::SutNotDatabase)?;
        let db = Database { conn, meter };
        // 署名があっても中身が SQLite でなければ、最初の問い合わせで分かる。表・索引などの数が多すぎるファイルは断る
        let objects = {
            let _scope = db.meter.scope();
            db.conn
                .query_row("SELECT count(*) FROM sqlite_master", [], |r| {
                    r.get::<_, i64>(0)
                })
                .map_err(|e| limits_or_not_database(&e))?
        };
        if objects > MAX_SCHEMA_OBJECTS {
            return Err(Fault::SutLimits);
        }
        Ok(db)
    }

    /// 通常の表として在るか（仮想表・ビューは無いものとする）。
    pub fn has_table(&self, name: &str) -> Result<bool, Fault> {
        let _scope = self.meter.scope();
        match self.conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1 COLLATE NOCASE",
            [name],
            |r| r.get::<_, Option<String>>(0),
        ) {
            Ok(sql) => Ok(!sql
                .unwrap_or_default()
                .trim_start()
                .to_ascii_uppercase()
                .starts_with("CREATE VIRTUAL")),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
            Err(e) => Err(limits_or_not_database(&e)),
        }
    }

    /// `Node` から、ブラシ（現在の設定か既定の設定の番号を持つ行）を上限まで。2 つ目は上限を超えて読まなかったブラシの数
    /// （行は全部を見て数える。ブラシでない行は数えない）。
    pub fn nodes(&self) -> Result<(Vec<Node>, usize), Fault> {
        if !self.has_table("Node")? {
            return Err(Fault::SutNoNodeTable);
        }
        let _scope = self.meter.scope();
        let mut stmt = self
            .conn
            .prepare("SELECT NodeName, NodeVariantId, NodeInitVariantId FROM Node")
            .map_err(|_| Fault::SutNoNodeTable)?;
        let mut rows = stmt.query([]).map_err(|e| limits_or_not_database(&e))?;
        let mut nodes = Vec::new();
        let mut skipped = 0usize;
        loop {
            let row = match rows.next() {
                Ok(Some(row)) => row,
                Ok(None) => break,
                Err(e) => return Err(limits_or_not_database(&e)),
            };
            // 番号は整数のほか、実数・数字の文字列でも持たれうる
            let int = |i: usize| match row.get_ref(i) {
                Ok(ValueRef::Integer(v)) => v,
                Ok(ValueRef::Real(v)) if v.is_finite() => v as i64,
                Ok(ValueRef::Text(t)) => std::str::from_utf8(t)
                    .ok()
                    .and_then(|s| s.trim().parse::<i64>().ok())
                    .unwrap_or(0),
                _ => 0,
            };
            let (variant, init_variant) = (int(1), int(2));
            if variant == 0 && init_variant == 0 {
                continue;
            }
            if nodes.len() >= MAX_BRUSHES {
                skipped += 1;
                continue;
            }
            let name = match row.get_ref(0) {
                Ok(ValueRef::Text(t) | ValueRef::Blob(t)) => {
                    short_text(&String::from_utf8_lossy(t), 128)
                }
                _ => String::new(),
            };
            nodes.push(Node {
                name,
                variant,
                init_variant,
            });
        }
        Ok((nodes, skipped))
    }

    /// 表の列（`PRAGMA table_xinfo`。生成列・隠れた列も出る）。`pragma` はこのファイルの定数の文。
    fn columns(&self, pragma: &str) -> Result<Vec<Column>, Fault> {
        let _scope = self.meter.scope();
        let mut stmt = self
            .conn
            .prepare(pragma)
            .map_err(|e| limits_or_not_database(&e))?;
        let mut rows = stmt.query([]).map_err(|e| limits_or_not_database(&e))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(|e| limits_or_not_database(&e))? {
            if out.len() >= MAX_COLUMNS {
                break;
            }
            let name = match row.get_ref(1) {
                Ok(ValueRef::Text(t)) => String::from_utf8_lossy(t).into_owned(),
                _ => continue,
            };
            let declared = match row.get_ref(2) {
                Ok(ValueRef::Text(t)) => String::from_utf8_lossy(t).to_ascii_uppercase(),
                _ => String::new(),
            };
            let hidden = match row.get_ref(6) {
                Ok(ValueRef::Integer(h)) => h,
                _ => 0,
            };
            out.push(Column {
                name,
                declared,
                hidden,
            });
        }
        Ok(out)
    }

    /// `Variant` が在るなら、番号から行を引く道具を返す（表か番号の列 `VariantID` が無ければ None）。行には `known`（小文字の列の
    /// 名前）のうち表に在る通常の列だけを入れる。番号は `VariantID` だけで引く（行の連番 `_PW_ID` は、別の行の設定を黙って読む
    /// 恐れがあるので代わりにしない）。
    pub fn variants(&self, known: &[&str]) -> Result<Option<Variants<'_>>, Fault> {
        if !self.has_table("Variant")? {
            return Ok(None);
        }
        let columns = self.columns("PRAGMA table_xinfo(Variant)")?;
        let normal = |wanted: &str| {
            columns
                .iter()
                .any(|c| c.hidden == 0 && c.name.eq_ignore_ascii_case(wanted))
        };
        if !normal("VariantID") {
            return Ok(None);
        }
        let key = "VariantID";
        let mut select: Vec<&str> = known.iter().copied().filter(|k| normal(k)).collect();
        select.dedup();
        if select.is_empty() {
            select.push(key);
        }
        let list: Vec<String> = select.iter().map(|c| format!("\"{c}\"")).collect();
        let sql = format!(
            "SELECT {} FROM \"Variant\" WHERE \"{key}\" = ?1 LIMIT 1",
            list.join(", ")
        );
        let stmt = {
            let _scope = self.meter.scope();
            self.conn
                .prepare(&sql)
                .map_err(|e| limits_or_not_database(&e))?
        };
        Ok(Some(Variants {
            stmt,
            meter: &self.meter,
        }))
    }

    /// `MaterialFile` の素材（上限まで）。2 つ目は上限を超えて読まなかった行があるか。表が無ければ空。
    /// 取り出した画像の合計は `png_budget` から引く（足りなければ `Fault::SutLimits`）。素材の C2F を読む仕事は `work`（取り込み全体で
    /// 共有）から引き、使い切ったあとの素材は画像が無いものとして扱う。
    pub fn materials(
        &self,
        png_budget: &mut u64,
        work: &c2f::Work,
    ) -> Result<(Vec<Material>, bool), Fault> {
        if !self.has_table("MaterialFile")? {
            return Ok((Vec::new(), false));
        }
        let columns = self.columns("PRAGMA table_xinfo(MaterialFile)")?;
        // 画像の列・番号の列・素材の名前などの文字の列（先頭の数列）。引用符を二重にして識別子として入れる
        let mut chosen: Vec<&Column> = Vec::new();
        for c in columns
            .iter()
            .filter(|c| c.hidden == 0 && c.name.len() <= 64)
        {
            let lower = c.name.to_ascii_lowercase();
            if lower == "filedata" || lower == "_pw_id" {
                chosen.push(c);
            }
        }
        let mut others = 0;
        for c in columns
            .iter()
            .filter(|c| c.hidden == 0 && c.name.len() <= 64)
        {
            let lower = c.name.to_ascii_lowercase();
            if lower != "filedata"
                && lower != "_pw_id"
                && !c.declared.contains("BLOB")
                && others < 6
            {
                chosen.push(c);
                others += 1;
            }
        }
        if chosen.is_empty() {
            return Ok((Vec::new(), false));
        }
        let list: Vec<String> = chosen
            .iter()
            .map(|c| format!("\"{}\"", c.name.replace('"', "\"\"")))
            .collect();
        let sql = format!("SELECT {} FROM \"MaterialFile\" LIMIT ?1", list.join(", "));
        let _scope = self.meter.scope();
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| limits_or_not_database(&e))?;
        let names: Vec<String> = chosen.iter().map(|c| c.name.to_ascii_lowercase()).collect();
        let mut rows = stmt
            .query([(MAX_MATERIALS + 1) as i64])
            .map_err(|e| limits_or_not_database(&e))?;
        let mut out = Vec::new();
        let mut more = false;
        let mut index = 0i64;
        loop {
            let row = match rows.next() {
                Ok(Some(row)) => row,
                Ok(None) => break,
                Err(e) => return Err(limits_or_not_database(&e)),
            };
            if out.len() >= MAX_MATERIALS {
                more = true;
                break;
            }
            let mut order = index;
            let mut texts = Vec::new();
            let mut image = None;
            let mut proprietary = false;
            let mut kind = None;
            for (i, name) in names.iter().enumerate() {
                let Ok(value) = row.get_ref(i) else {
                    continue;
                };
                match (name.as_str(), value) {
                    ("filedata", ValueRef::Blob(blob)) => {
                        image = material::extract(blob, work);
                        proprietary = image.is_none() && material::has_proprietary_image(blob);
                        kind = material::kind(blob);
                    }
                    ("_pw_id", ValueRef::Integer(id)) => order = id,
                    (_, ValueRef::Text(t)) if texts.len() < 8 => {
                        let text = short_text(&String::from_utf8_lossy(t), 512);
                        if !text.is_empty() {
                            texts.push(text);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(found) = &image {
                let size = found.png.len() as u64;
                if size > *png_budget {
                    return Err(Fault::SutLimits);
                }
                *png_budget -= size;
            }
            out.push(Material {
                order,
                texts,
                image,
                proprietary,
                kind,
            });
            index += 1;
        }
        out.sort_by_key(|m| m.order);
        Ok((out, more))
    }
}

/// `table_xinfo` の 1 列。
struct Column {
    name: String,
    /// 宣言の型（大文字）。
    declared: String,
    /// 0 通常、1 仮想表の隠れた列、2・3 生成列。
    hidden: i64,
}

/// `Variant` を番号で引く（準備した 1 つの文を使い回す）。
pub(super) struct Variants<'conn> {
    stmt: Statement<'conn>,
    meter: &'conn Meter,
}

impl Variants<'_> {
    /// その番号の行。無ければ None。
    pub fn get(&mut self, id: i64) -> Result<Option<Row>, Fault> {
        let _scope = self.meter.scope();
        let mut rows = self
            .stmt
            .query([id])
            .map_err(|e| limits_or_not_database(&e))?;
        let row = match rows.next() {
            Ok(Some(row)) => row,
            Ok(None) => return Ok(None),
            Err(e) => return Err(limits_or_not_database(&e)),
        };
        let names: Vec<String> = row
            .as_ref()
            .column_names()
            .iter()
            .take(MAX_COLUMNS)
            .map(|n| n.to_ascii_lowercase())
            .collect();
        let mut cells = HashMap::new();
        for (i, name) in names.into_iter().enumerate() {
            if let Ok(value) = row.get_ref(i) {
                cells.insert(name, Cell::read(value));
            }
        }
        Ok(Some(Row { cells }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Variant` に番号だけの行が `rows` 個ある（番号の列に索引は無いので、引くたびに全部を見る）データベースの中身。
    fn scan_file(rows: u32) -> Vec<u8> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE Variant (VariantID INTEGER, BrushSize REAL);
             WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < {rows})
             INSERT INTO Variant SELECT x, 10 FROM c;"
        ))
        .unwrap();
        conn.serialize("main").unwrap().to_vec()
    }

    fn open(rows: u32, limits: Limits) -> Database {
        Database::open_with(&scan_file(rows), limits).unwrap()
    }

    #[test]
    fn a_scan_that_runs_past_the_instruction_limit_is_refused_as_a_limit() {
        // 2 万行の全件の走査は 6 万命令（確かめる単位で 6 回）。上限 3 回では、引いている途中で打ち切られる
        let db = open(
            20_000,
            Limits {
                calls: 3,
                time: Duration::from_secs(3600),
            },
        );
        let mut variants = db.variants(&["brushsize"]).unwrap().unwrap();
        assert_eq!(variants.get(999_999).err(), Some(Fault::SutLimits));
        // 上限の範囲なら引ける
        let db = open(
            20_000,
            Limits {
                calls: 1000,
                time: Duration::from_secs(3600),
            },
        );
        let mut variants = db.variants(&["brushsize"]).unwrap().unwrap();
        assert!(variants.get(19_999).unwrap().is_some());
    }

    #[test]
    fn the_instruction_limit_is_a_total_over_all_queries() {
        // 1 回の走査は 6 回ぶん。上限 20 回なら、最初の数回は引けて、積み重なって打ち切られる
        let db = open(
            20_000,
            Limits {
                calls: 20,
                time: Duration::from_secs(3600),
            },
        );
        let mut variants = db.variants(&["brushsize"]).unwrap().unwrap();
        assert!(
            variants.get(999_999).unwrap().is_none(),
            "1 回ぶんは上限の中"
        );
        let mut outcome = Ok(None);
        for _ in 0..10 {
            outcome = variants.get(999_999);
            if outcome.is_err() {
                break;
            }
        }
        assert_eq!(outcome.err(), Some(Fault::SutLimits));
    }

    #[test]
    fn a_scan_that_runs_past_the_time_limit_is_refused_as_a_limit() {
        let db = open(
            20_000,
            Limits {
                calls: u32::MAX,
                time: Duration::ZERO,
            },
        );
        let mut variants = db.variants(&["brushsize"]).unwrap().unwrap();
        assert_eq!(variants.get(999_999).err(), Some(Fault::SutLimits));
    }

    #[test]
    fn time_spent_outside_the_database_does_not_count() {
        // 開いてから SQL の外で時間が過ぎても（PNG の復号など）、そのあとの問い合わせは時間の上限に数えない
        let db = open(
            20_000,
            Limits {
                calls: u32::MAX,
                time: Duration::from_millis(300),
            },
        );
        let mut variants = db.variants(&["brushsize"]).unwrap().unwrap();
        std::thread::sleep(Duration::from_millis(600));
        assert!(variants.get(19_999).unwrap().is_some());
        std::thread::sleep(Duration::from_millis(600));
        assert!(variants.get(1).unwrap().is_some());
    }

    #[test]
    fn load_extension_is_refused_from_sql() {
        // SQLite 本体には拡張を読む機能が入っているが、有効化の関数を呼ばないので SQL からは使えない
        let db = open(10, Limits::default());
        let error = db
            .conn
            .query_row("SELECT load_extension('/nonexistent/extension')", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_err();
        assert!(
            error.to_string().contains("not authorized"),
            "拡張を開こうとしている: {error}"
        );
        // 引数が 2 つの形も同じ
        assert!(db
            .conn
            .query_row(
                "SELECT load_extension('/nonexistent/extension', 'entry')",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap_err()
            .to_string()
            .contains("not authorized"));
    }
}
