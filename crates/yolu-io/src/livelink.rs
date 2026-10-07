//! Live Link の相手の文書（根の `livelink.json`）。スタンドアロン版が .ylp の状態として残し、開き直すと Unity なしで同じモデル
//! （FBX の並び・レンダラーとマテリアルの結び）とポーズになる。
//!
//! - 中身は Live Link の頼みと同じ形（`format` 1・`kind` "open" の JSON。形の正本は Rust のリポジトリの `docs/LIVELINK.md`）で、ポーズは
//!   今のポーズ（骨の値・BlendShape の重み）に、マテリアルの値は除いて（値は各セットの `look.json` の `received`）書く。ここは大きさと
//!   JSON のオブジェクトであることと版だけを確かめ、中身はアプリが読む（読めなければエントリはバイト列のまま残す）。
//! - 正本の版も .ylp の形式（7・8）も変えない状態のエントリ（`view.json`・`pose.json` と同じ扱い）。スタンドアロン 0.4.x は知らない
//!   エントリとして知らせ、開いて保存し直してもバイト列のまま残す。Unity 版は知らせて保存で落とす（失うのは相手の記録とポーズだけ）。

use serde_json::Value;

use crate::{check, check_budget, Result};

/// エントリの名前（根）。
pub const ENTRY: &str = "livelink.json";
/// 版。
pub const FORMAT: i64 = 1;
/// エントリの大きさの上限（頼みの大きさの上限と同じ）。
pub const MAX_BYTES: usize = 16 * 1024 * 1024;

/// 確かめる（大きさ・JSON のオブジェクト・`format` が 1）。
pub fn validate(bytes: &[u8]) -> Result<()> {
    check_budget(
        bytes.len() <= MAX_BYTES,
        "livelink.json のバイト予算超過です",
    )?;
    let root: Value = serde_json::from_slice(bytes)?;
    let format = root.get("format").and_then(Value::as_i64);
    check(
        root.is_object(),
        "livelink.json がオブジェクトではありません",
    )?;
    check(
        format == Some(FORMAT),
        "livelink.json の形式はこの版では読めません",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_format_1_object_within_the_budget_is_accepted() {
        assert!(validate(br#"{"format":1,"kind":"open"}"#).is_ok());
        assert!(validate(br#"{"format":2}"#).is_err());
        assert!(validate(br#"[1]"#).is_err());
        assert!(validate(b"{").is_err());
        let big = format!("{{\"format\":1,\"x\":\"{}\"}}", "a".repeat(MAX_BYTES));
        assert!(validate(big.as_bytes()).is_err());
    }
}
