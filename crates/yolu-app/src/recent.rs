//! 最近開いたプロジェクトの一覧（ファイルのメニューの「最近開いたプロジェクト」）。最大 `MAX` 件・新しい順。
//!
//! 置き場は、設定のファイル（`settings.conf`）と同じフォルダの `recent.conf`（端末ごと）。1 行目が見出し（`yolupainter-recent 1`）、あとは 1 行に
//! 1 つの絶対パス。読み書きは `userfiles` の部品を通す（大きさを先に確かめて読む・一時ファイルへ書いて読み戻し、1 回の置換で確定する）。
//!
//! 読めない・壊れたファイル（UTF-8 でない・大きすぎる・見出しが違う）は、空の一覧として始める。そのファイルは捨てず、次に一覧を書くとき
//! （開く・保存した後）に、先に `recent.conf.broken` へ移してから新しく書く（上書きで失わない）。新しいバージョンの見出しのファイルも同じ。
//! 書けなければ（権限・ディスク）、開いている間だけ覚える。テストの状態（置き場なし）は、ファイルに触れない。

use std::path::{Path, PathBuf};

use crate::userfiles::{self, FileError};

/// 一覧の最大の件数。
pub const MAX: usize = 10;
/// ファイルの見出し（バージョンつき）。
const HEADER: &str = "yolupainter-recent 1";
/// 読む行の数の上限（見出しの次から。書くのは `MAX` 行だけなので、余裕を見て）。
const MAX_LINES: usize = 100;
/// 読むファイルの大きさの上限（10 件 × パス 1 本に十分）。
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// 最近開いたプロジェクト。
#[derive(Clone, Debug, Default)]
pub struct Recent {
    items: Vec<PathBuf>,
    /// 残すファイル（None なら、このアプリを開いている間だけ覚える）。
    path: Option<PathBuf>,
    /// 置き場にあるファイルを読めなかった（次に書く前に、別の名前へ移す）。
    unreadable: bool,
}

impl Recent {
    /// 設定のフォルダの `recent.conf`（設定のフォルダが分からなければ None）。
    pub fn default_path() -> Option<PathBuf> {
        crate::settings::path().map(|p| p.with_file_name("recent.conf"))
    }

    /// ファイルから読む。無ければ空から始める（書くときに作る）。読めないファイルは空の一覧で始め、書き直す前に移す。
    pub fn load(path: Option<PathBuf>) -> Recent {
        let Some(file) = path else {
            return Recent::default();
        };
        match userfiles::read_checked(&file, MAX_FILE_BYTES) {
            Ok(text) => match parse(&text) {
                Some(items) => Recent {
                    items,
                    path: Some(file),
                    unreadable: false,
                },
                None => Recent {
                    items: Vec::new(),
                    path: Some(file),
                    unreadable: true,
                },
            },
            Err(FileError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Recent {
                items: Vec::new(),
                path: Some(file),
                unreadable: false,
            },
            Err(_) => Recent {
                items: Vec::new(),
                path: Some(file),
                unreadable: true,
            },
        }
    }

    /// 新しい順の一覧。
    pub fn items(&self) -> &[PathBuf] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 開いた・保存したファイルを先頭へ（同じファイルは前へ動かす。`MAX` を超えた古い物は落とす）。覚えられないパス（1 行に書けない）は覚えない。
    pub fn add(&mut self, file: &Path) {
        let file = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
        if !usable(&file) {
            return;
        }
        self.items.retain(|p| !userfiles::same_file(p, &file));
        self.items.insert(0, file);
        self.items.truncate(MAX);
        self.save();
    }

    /// 一覧から外す（無くなったファイル）。
    pub fn remove(&mut self, file: &Path) {
        let before = self.items.len();
        self.items.retain(|p| !userfiles::same_file(p, file));
        if self.items.len() != before {
            self.save();
        }
    }

    /// 一覧を消す。
    pub fn clear(&mut self) {
        if !self.items.is_empty() {
            self.items.clear();
            self.save();
        }
    }

    fn save(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        if self.unreadable {
            // 読めなかったファイルは捨てず、別の名前へ移してから新しく書く（移せなければ、書かずに開いている間だけ覚える）
            if path.exists() {
                let Some(dir) = path.parent() else {
                    return;
                };
                let name = (1..)
                    .map(|n| match n {
                        1 => "recent.conf.broken".to_owned(),
                        n => format!("recent.conf.broken-{n}"),
                    })
                    .find(|n| !dir.join(n).exists())
                    .expect("名前は尽きない");
                if std::fs::rename(&path, dir.join(name)).is_err() {
                    return;
                }
            }
            self.unreadable = false;
        }
        let text = encode(&self.items);
        // 書けなくても、開いている間は覚えている
        let _ = userfiles::write_text(&path, &text, MAX_FILE_BYTES, |read| read == text);
    }
}

/// ファイルの文にする。
fn encode(items: &[PathBuf]) -> String {
    let mut text = format!("{HEADER}\n");
    for item in items {
        if let Some(line) = item.to_str() {
            text.push_str(line);
            text.push('\n');
        }
    }
    text
}

/// 文を読む。見出しが違えば None（読めないファイル）。読めない行（絶対パスでない）は読み飛ばし、同じファイルは落とす。`MAX` 件がそろったら
/// 残りの行は読まない（起動を止めないため。同じファイルかを調べるたびにディスクへ尋ねる）。
fn parse(text: &str) -> Option<Vec<PathBuf>> {
    parse_with(text, userfiles::same_file)
}

/// `parse` の本体（同じファイルかの調べ方を受ける）。調べる行は、`MAX` 件がそろうまでと、`MAX_LINES` 行までに限る（このアプリが書くのは
/// `MAX` 行だけ。同じパスが何千行も続く手編集のファイルでも、調べる数は上限で止まる）。
fn parse_with(text: &str, mut same: impl FnMut(&Path, &Path) -> bool) -> Option<Vec<PathBuf>> {
    let mut lines = text.lines();
    if lines.next()?.trim_end() != HEADER {
        return None;
    }
    let mut items: Vec<PathBuf> = Vec::new();
    for line in lines.take(MAX_LINES) {
        if items.len() >= MAX {
            break;
        }
        let path = PathBuf::from(line.trim_end_matches('\r'));
        if usable(&path) && !items.iter().any(|p| same(p, &path)) {
            items.push(path);
        }
    }
    Some(items)
}

/// 覚えてよいパスか（絶対パスで、1 行の UTF-8 に書ける）。
fn usable(path: &Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|s| !s.is_empty() && !s.contains(['\n', '\r']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_absolute_one_line_paths_are_remembered() {
        assert!(usable(Path::new(&std::env::temp_dir().join("a.ylp"))));
        assert!(!usable(Path::new("a.ylp")));
        assert!(!usable(&std::env::temp_dir().join("a\nb.ylp")));
        assert_eq!(parse("まちがい\nx"), None);
        assert_eq!(parse(&format!("{HEADER}\nrelative.ylp\n")), Some(vec![]));
    }

    /// 同じファイルかの調べの数を数えながら読む。
    fn counted(text: &str) -> (Vec<PathBuf>, usize) {
        let mut calls = 0;
        let items = parse_with(text, |a, b| {
            calls += 1;
            a == b
        })
        .unwrap();
        (items, calls)
    }

    #[test]
    fn a_file_with_thousands_of_lines_checks_only_a_bounded_number_of_files() {
        let dir = std::env::temp_dir();
        // 違うパスが数千行: 10 件がそろったところで止まる（調べは 0+1+…+9 回）
        let mut text = format!("{HEADER}\n");
        for n in 0..5000 {
            text.push_str(&format!("{}\n", dir.join(format!("p{n}.ylp")).display()));
        }
        let (items, calls) = counted(&text);
        assert_eq!(items.len(), MAX);
        assert_eq!(items[0], dir.join("p0.ylp"));
        assert_eq!(items[MAX - 1], dir.join(format!("p{}.ylp", MAX - 1)));
        assert_eq!(calls, MAX * (MAX - 1) / 2);
        // 同じパスが数千行: 件数がそろわなくても、読む行の上限で止まる
        let mut text = format!("{HEADER}\n");
        for _ in 0..5000 {
            text.push_str(&format!("{}\n", dir.join("same.ylp").display()));
        }
        let (items, calls) = counted(&text);
        assert_eq!(items, vec![dir.join("same.ylp")]);
        assert!(calls <= MAX_LINES, "{calls}");
        // 読めない行が数千行続いても、調べは起きない
        let mut text = format!("{HEADER}\n");
        for _ in 0..5000 {
            text.push_str("relative.ylp\n");
        }
        assert_eq!(counted(&text), (vec![], 0));
    }
}
