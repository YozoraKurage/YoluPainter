//! OS のファイルの窓・確かめの窓は、主の窓を親にして出す口（`dialog::file`・`dialog::message`）から作る。`rfd::FileDialog::new()`・
//! `rfd::MessageDialog::new()` を直に呼ぶと、親の無い窓が主の窓の後ろに回る（Windows）。ソースの中で直に呼んでよいのは、
//! 口そのもの（`dialog.rs`）と、窓の無い起動でも出すクラッシュの知らせ（`crash/`）だけ。
//! 例外として、終了の確かめ（`app.rs` の `confirm_close`）はまだ直に呼んでいる。ここを `dialog::message()` へ置き換えれば例外は
//! 要らなくなる（置き換えても、この試験は通る）。
use std::path::{Path, PathBuf};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `text` の中で `needle` が現れる位置の、含まれる関数の名前（`fn name` の最後に見えたもの）。
fn enclosing_fn(text: &str, at: usize) -> String {
    let head = &text[..at];
    head.rfind("fn ")
        .map(|i| head[i + 3..].split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("").to_owned())
        .unwrap_or_default()
}

#[test]
fn file_and_message_dialogs_come_from_the_parented_helpers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    assert!(files.len() > 50, "ソースを集められていない");
    let mut direct = Vec::new();
    for file in files {
        let relative = file.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        if relative == "dialog.rs" || relative.starts_with("crash/") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        for needle in ["rfd::FileDialog::new()", "rfd::MessageDialog::new()"] {
            for (at, _) in text.match_indices(needle) {
                direct.push((relative.clone(), enclosing_fn(&text, at)));
            }
        }
    }
    let allowed = [("app.rs".to_owned(), "confirm_close".to_owned())];
    let unexpected: Vec<_> = direct.iter().filter(|d| !allowed.contains(d)).collect();
    assert!(unexpected.is_empty(), "親なしの窓を直に作っている（`crate::dialog::file()`・`message()` を使う）: {unexpected:?}");
}
