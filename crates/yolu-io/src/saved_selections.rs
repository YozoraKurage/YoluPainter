//! 名前を付けて残した選択範囲（セットの下の `selections.json` と `selection-<SHA-256>.bin`）。.ylp の形式 8 の中身。
//!
//! - 並びと名前は `sets/<ID>/selections.json`（索引）、選択範囲の中身は `sets/<ID>/selection-<中身の印>.bin`（今の選択範囲の
//!   `selection.bin` と同じ `YLSL` の版 1。[`crate::Selection`]）。中身の印は、選択範囲のバイト列の SHA-256 の先頭 128 bit（小文字の
//!   16 進 32 桁。エントリの名前が `sets/<ID>/` を含めて 96 文字までなので、全部の桁は入らない）。同じ中身は 1 つのエントリを共有する。
//!   名前の決まりは外側の決まり（英数字と `. - _`、96 文字まで。フォルダなし）のままなので、外側の版（`YOLUPAINTER-YLP-3`）は変わらない。
//! - 索引の形（UTF-8 の JSON のオブジェクト、256 KiB まで）:
//!   ```json
//!   { "format": 1, "selections": [ { "name": "前髪", "content": "<中身の印（SHA-256 の先頭の小文字の 16 進 32 桁）>" } ] }
//!   ```
//!   `selections` は並びの順（0〜[`MAX_SAVED`] 個）。`name` は前後の空白がなく、1〜[`MAX_NAME_CHARS`] 文字、制御文字なし、並びの中で
//!   重ならない。知らないキーは読み飛ばし、`format` が 1 でないもの（新しい版）は読まない。
//! - 読み手は、壊れた項目だけを飛ばし（理由は [`SkipReason`]）、読める項目は読む。索引そのものが読めなければ全部を飛ばす。
//!   飛ばしたエントリはファイルにバイト列のまま残る（残した選択範囲を書き換えるまで）。選択範囲と文書の大きさが違うものは断る
//!   （画布の大きさを変える操作は、書く前に残した選択範囲も新しい大きさへ作り直す）。
//! - 書き手は、決まりに合わない並びを書かずに断り、書き直すときは前の索引と `selection-*.bin` をセットごと全部置き換える
//!   （使われなくなった中身を残さない）。同じ並びはいつも同じバイト列になる。

use crate::{
    check, hash, package::Blob, Error, Result, Selection, MAX_ENTRY_BYTES,
};
use serde_json::Value;

/// 索引のエントリ名（セットの下）。
pub const INDEX: &str = "selections.json";
/// 1 つのセットに残せる数（読み手が読む数・書き手が断る数）。
pub const MAX_SAVED: usize = yolu_core::MAX_SAVED_SELECTIONS;
/// 名前の長さの上限（文字数）。
pub const MAX_NAME_CHARS: usize = yolu_core::MAX_SAVED_NAME_CHARS;
/// 索引の大きさの上限。
pub const MAX_INDEX_BYTES: usize = 256 * 1024;
/// 索引の版。
pub const FORMAT: i64 = 1;

const CONTENT_PREFIX: &str = "selection-";
const CONTENT_SUFFIX: &str = ".bin";
/// 中身の印の桁数（SHA-256 の先頭の 16 進）。
const CONTENT_ID_LEN: usize = 32;

/// 中身の印（SHA-256 の先頭 32 桁）か。
fn is_content_id(s: &str) -> bool {
    s.len() == CONTENT_ID_LEN && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 選択範囲のバイト列の中身の印。
fn content_id(bytes: &[u8]) -> String {
    hash(bytes)[..CONTENT_ID_LEN].to_owned()
}

/// 名前つきの選択範囲 1 つ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedSelection {
    pub name: String,
    pub selection: Selection,
}

/// 項目を飛ばした理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// 索引が読めない（壊れた JSON・型の違い・新しい版。すべての項目を飛ばす。詳細は診断の文）。
    Index(String),
    /// 項目の形が正しくない（名前の決まりに合わない・中身の印が不正）。
    Item(String),
    /// 索引にある中身のエントリがファイルに無い。
    MissingContent,
    /// 中身が選択範囲として読めない（`YLSL` の検査・索引の印と違う。診断の文）。
    UnreadableContent(String),
    /// 中身は選択範囲として読めるが、今の文書と大きさ（幅・高さ・タイルの大きさ）が違う（文書の大きさを変えたあとの古いもの）。壊れたファイルではない。
    WrongSize,
    /// 名前が前の項目と重なる（先のものを読んだ）。
    DuplicateName,
    /// 数の上限（[`MAX_SAVED`]）を超えた項目。
    TooMany,
}

/// 飛ばした項目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// 索引の中の位置（0 から。索引が読めなければ 0）。
    pub index: usize,
    /// 読めた名前（読めなければ None）。
    pub name: Option<String>,
    pub reason: SkipReason,
}

/// 読んだ結果（読めた項目と、飛ばした項目）。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SavedSelections {
    pub items: Vec<SavedSelection>,
    pub skipped: Vec<Skipped>,
}

/// セットの下のエントリを葉の名前から引く口（無ければ None）。
pub(crate) type Entry<'a> = dyn Fn(&str) -> Option<Result<std::sync::Arc<[u8]>>> + 'a;

/// 選択範囲の中身のエントリ名の葉（`selection-<64 桁の 16 進>.bin`）か。
pub(crate) fn is_content_leaf(leaf: &str) -> bool {
    leaf.strip_prefix(CONTENT_PREFIX)
        .and_then(|rest| rest.strip_suffix(CONTENT_SUFFIX))
        .is_some_and(is_content_id)
}

/// セットの下のこの機能のエントリ（索引か中身）の葉か。
pub(crate) fn is_entry_leaf(leaf: &str) -> bool {
    leaf == INDEX || is_content_leaf(leaf)
}

fn content_leaf(content: &str) -> String {
    format!("{CONTENT_PREFIX}{content}{CONTENT_SUFFIX}")
}

/// 名前が決まりに合うか（前後の空白がなく、1〜上限の文字数、制御文字なし）。
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(char::is_control)
}

/// `prefix`（`sets/<ID>/`）の下から読む。`entry` は葉の名前から中身を引く口、`size` は文書の（幅・高さ・タイルの大きさ）。
pub(crate) fn read(
    entry: &Entry<'_>,
    size: (i32, i32, i32),
) -> SavedSelections {
    let mut out = SavedSelections::default();
    let Some(index) = entry(INDEX) else {
        return out;
    };
    let index_failed = |why: String| SavedSelections {
        items: Vec::new(),
        skipped: vec![Skipped {
            index: 0,
            name: None,
            reason: SkipReason::Index(why),
        }],
    };
    let bytes = match index {
        Ok(b) => b,
        Err(e) => return index_failed(e.to_string()),
    };
    let root = match parse_index(&bytes) {
        Ok(v) => v,
        Err(e) => return index_failed(e.to_string()),
    };
    let list = root["selections"].as_array().cloned().unwrap_or_default();
    for (i, item) in list.iter().enumerate() {
        let name = item.get("name").and_then(Value::as_str).map(str::to_owned);
        let skip = |reason| Skipped {
            index: i,
            name: name.clone(),
            reason,
        };
        if out.items.len() >= MAX_SAVED {
            out.skipped.push(skip(SkipReason::TooMany));
            continue;
        }
        let Some(name_ok) = name.clone().filter(|n| valid_name(n)) else {
            out.skipped.push(skip(SkipReason::Item("name".into())));
            continue;
        };
        let content = match item.get("content").and_then(Value::as_str) {
            Some(c) if is_content_id(c) => c,
            _ => {
                out.skipped.push(skip(SkipReason::Item("content".into())));
                continue;
            }
        };
        if out.items.iter().any(|s| s.name == name_ok) {
            out.skipped.push(skip(SkipReason::DuplicateName));
            continue;
        }
        let bytes = match entry(&content_leaf(content)) {
            None => {
                out.skipped.push(skip(SkipReason::MissingContent));
                continue;
            }
            Some(Err(e)) => {
                out.skipped
                    .push(skip(SkipReason::UnreadableContent(e.to_string())));
                continue;
            }
            Some(Ok(b)) => b,
        };
        if content_id(&bytes) != content {
            out.skipped.push(skip(SkipReason::UnreadableContent(
                "中身の印が索引と違います".into(),
            )));
            continue;
        }
        // 大きさだけが違うもの（文書の大きさを変えたあとの古いもの）は、壊れたファイルとは言い分ける
        if header_size(&bytes).is_some_and(|found| found != size) {
            out.skipped.push(skip(SkipReason::WrongSize));
            continue;
        }
        match Selection::read_sized(&bytes, size) {
            Ok(selection) => out.items.push(SavedSelection {
                name: name_ok,
                selection,
            }),
            Err(e) => out
                .skipped
                .push(skip(SkipReason::UnreadableContent(e.to_string()))),
        }
    }
    out
}

/// 選択範囲のバイト列の頭（`YLSL` と版 1）から、大きさ（幅・高さ・タイルの大きさ）だけを読む。頭が読めなければ None。
fn header_size(b: &[u8]) -> Option<(i32, i32, i32)> {
    if b.len() < 20 || &b[..4] != b"YLSL" {
        return None;
    }
    let int = |at: usize| i32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    (int(4) == 1).then(|| (int(8), int(12), int(16)))
}

/// 索引を読む（形の検査。項目の検査は `read`）。
fn parse_index(bytes: &[u8]) -> Result<Value> {
    check(
        bytes.len() <= MAX_INDEX_BYTES,
        "残した選択範囲の索引が大きすぎます",
    )?;
    let value: Value = serde_json::from_slice(bytes)?;
    check(value.is_object(), "残した選択範囲の索引がオブジェクトではありません")?;
    let format = value.get("format").and_then(Value::as_i64);
    check(
        format == Some(FORMAT),
        "残した選択範囲の索引の版が未対応です",
    )?;
    check(
        value.get("selections").is_some_and(Value::is_array),
        "残した選択範囲の索引に selections がありません",
    )?;
    Ok(value)
}

/// 書く決まりを確かめる（数・名前・重なり・文書との大きさ）。
pub(crate) fn validate(items: &[SavedSelection], size: (i32, i32, i32)) -> Result<()> {
    check(
        items.len() <= MAX_SAVED,
        format!("残せる選択範囲は {MAX_SAVED} 個までです"),
    )?;
    for (i, s) in items.iter().enumerate() {
        check(
            valid_name(&s.name),
            format!("残した選択範囲の名前が決まりに合いません: {}", s.name),
        )?;
        check(
            !items[..i].iter().any(|t| t.name == s.name),
            format!("残した選択範囲の名前が重なっています: {}", s.name),
        )?;
        check(
            (s.selection.width(), s.selection.height(), s.selection.tile_size()) == size,
            format!("残した選択範囲と正本の大きさが一致しません: {}", s.name),
        )?;
    }
    Ok(())
}

/// 書くエントリ（葉の名前 → 中身。空の並びは空）。同じ中身は 1 つ。
pub(crate) fn entries(items: &[SavedSelection]) -> Result<Vec<(String, Blob)>> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let mut out: Vec<(String, Blob)> = Vec::new();
    let mut list = Vec::with_capacity(items.len());
    for s in items {
        let bytes = s.selection.to_bytes();
        if bytes.len() > MAX_ENTRY_BYTES {
            return Err(Error::Budget(format!(
                "残した選択範囲が大きすぎます: {}",
                s.name
            )));
        }
        let content = content_id(&bytes);
        list.push(serde_json::json!({ "name": s.name, "content": content }));
        let leaf = content_leaf(&content);
        if !out.iter().any(|(n, _)| *n == leaf) {
            out.push((leaf, Blob::from(bytes)));
        }
    }
    let index = serde_json::json!({ "format": FORMAT, "selections": list });
    out.push((INDEX.into(), Blob::from(serde_json::to_vec(&index)?)));
    Ok(out)
}
