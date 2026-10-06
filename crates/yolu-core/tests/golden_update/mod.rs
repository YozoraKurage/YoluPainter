//! 正解の撮り直し（`YOLU_GOLDEN_UPDATE=1`）の共通の道具。撮り直しでは、違った正解を今の出力で書き直し、比べの失敗にしない。
//! 撮り直したら差分（git diff）を見て、意図した変化だけかを確かめる。正解のファイルの見出しの行（# …）はそのまま残す。
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 撮り直しの間か。
pub fn updating() -> bool {
    std::env::var_os("YOLU_GOLDEN_UPDATE").is_some()
}

/// この crate の `tests` のフォルダ。
pub fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// 同じ試験の実行ファイルの中で、並んで走る試験が同じファイルを書き直しても壊れないように。
static FILES: Mutex<()> = Mutex::new(());

/// 行のファイル `path` の、鍵 `key`（行の先頭、次が空白）の行を `line` に置き換える。
pub fn replace_line(path: &Path, key: &str, line: &str) {
    let _held = FILES.lock().unwrap_or_else(|e| e.into_inner());
    let text = std::fs::read_to_string(path).unwrap();
    let prefix = format!("{key} ");
    let mut found = 0;
    let lines: Vec<&str> = text
        .lines()
        .map(|l| {
            if !l.starts_with('#') && l.starts_with(&prefix) {
                found += 1;
                line
            } else {
                l
            }
        })
        .collect();
    assert_eq!(found, 1, "{} に鍵 {key} の行が 1 つではない", path.display());
    std::fs::write(path, lines.join("\n") + "\n").unwrap();
}
