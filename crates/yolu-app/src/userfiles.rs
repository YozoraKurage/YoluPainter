//! 利用者のファイル（設定のフォルダに置くプリセット・一覧）の置き場で共通の部品: 上限つきの読み込み・読み戻して確かめる置換・番号の
//! ファイルの読み込み・消す・重ならない名前・変更ありの判定。形式（行の形・誤りの種類）はそれぞれのモジュールが持つ。

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// 読み書きの失敗のうち、形式によらない物（それぞれの誤りの種類へ写す）。
#[derive(Debug)]
pub enum FileError {
    Io(io::Error),
    /// 上限より大きい。
    TooLarge,
    /// UTF-8 の文でない。
    NotText,
    /// 書いたファイルを読み戻したら、書いた中身と違った。
    Mismatch,
}

impl From<io::Error> for FileError {
    fn from(e: io::Error) -> Self {
        FileError::Io(e)
    }
}

/// 上限つきで文として読む（上限 + 1 バイトまで読んで、超えれば `TooLarge`。UTF-8 でなければ読み込みの失敗 `Io`）。
pub fn read_text(path: &Path, limit: u64) -> Result<String, FileError> {
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take(limit + 1).read_to_string(&mut text)?;
    if text.len() as u64 > limit {
        return Err(FileError::TooLarge);
    }
    Ok(text)
}

/// 大きさを先に確かめてから文として読む（上限を超えれば `TooLarge`、UTF-8 でなければ `NotText`）。
pub fn read_checked(path: &Path, limit: u64) -> Result<String, FileError> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > limit {
        return Err(FileError::TooLarge);
    }
    let bytes = std::fs::read(path)?;
    String::from_utf8(bytes).map_err(|_| FileError::NotText)
}

/// 文を `path` へ置く（フォルダが無ければ作り、一時ファイルへ書いて読み戻しを `verify` で確かめ、1 回の置換で確定。
/// `yolu_io::atomic`）。上限を超える文は書かない。
pub fn write_text(
    path: &Path,
    text: &str,
    limit: u64,
    verify: impl Fn(&str) -> bool,
) -> Result<(), FileError> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    if text.len() as u64 > limit {
        return Err(FileError::TooLarge);
    }
    let check = |read: &[u8]| std::str::from_utf8(read).is_ok_and(&verify);
    let opts = yolu_io::atomic::ReplaceOptions {
        limit: Some(limit),
        verify: Some(&check),
        create_dirs: false,
    };
    yolu_io::atomic::replace_with(path, &opts, |f| f.write_all(text.as_bytes())).map_err(|e| {
        match yolu_io::atomic::rejected(&e) {
            Some(yolu_io::atomic::Rejected::TooLarge) => FileError::TooLarge,
            Some(yolu_io::atomic::Rejected::Mismatch) => FileError::Mismatch,
            None => FileError::Io(e),
        }
    })
}

/// ファイルを消す（もう無ければ、消せたのと同じ）。
pub fn remove(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// 番号の名前のファイルを読んだ結果（番号の順）。
pub struct Loaded<T, E> {
    /// 読めた中身と番号。
    pub items: Vec<(u32, T)>,
    /// 読めなかったファイルの名前と理由。
    pub problems: Vec<(String, E)>,
    /// 次に使う番号（読めなかったファイルの番号も避ける）。
    pub next_id: u32,
}

/// フォルダの番号の名前のファイル（`file_id` が番号を返す物）を番号の順に読む。`max` 個を超えた分と、名前（`name_of`）が先の物と
/// 重なる物は読み飛ばして理由を残す。無いフォルダは空。
pub fn load_numbered<T, E>(
    dir: &Path,
    file_id: impl Fn(&Path) -> Option<u32>,
    max: usize,
    read: impl Fn(&Path) -> Result<T, E>,
    name_of: impl Fn(&T) -> &str,
    too_many: impl Fn() -> E,
    duplicate: impl Fn() -> E,
) -> Loaded<T, E> {
    let mut files: Vec<(u32, PathBuf)> = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
        for entry in read.flatten() {
            let path = entry.path();
            if let Some(id) = file_id(&path) {
                files.push((id, path));
            }
        }
    }
    files.sort_by_key(|(id, _)| *id);
    let mut loaded = Loaded {
        items: Vec::new(),
        problems: Vec::new(),
        next_id: 0,
    };
    let mut max_id = 0;
    for (id, path) in files {
        max_id = max_id.max(id);
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if loaded.items.len() >= max {
            loaded.problems.push((file, too_many()));
            continue;
        }
        match read(&path) {
            Ok(item) => {
                if loaded
                    .items
                    .iter()
                    .any(|(_, p)| name_of(p) == name_of(&item))
                {
                    loaded.problems.push((file, duplicate()));
                } else {
                    loaded.items.push((id, item));
                }
            }
            Err(error) => loaded.problems.push((file, error)),
        }
    }
    loaded.next_id = max_id + 1;
    loaded
}

/// 利用者の物の名前を、先の物と重ならないようにする（重なれば ` 2`・` 3`… を付ける）。
pub fn unused_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|name| !taken(name))
        .expect("名前は尽きない")
}

/// 一覧の項目が元と違うか。今の項目（`current`）は今の設定（`live`）と元（`baseline`）を比べ、ほかの項目は覚えている変更（`edited`）で
/// 見る。項目が無ければ false。
pub fn is_modified<T: PartialEq>(entry: Option<(&T, bool)>, current: bool, live: &T) -> bool {
    match entry {
        Some((baseline, _)) if current => live != baseline,
        Some((_, edited)) => edited,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("yolu-app-userfiles-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn text_is_written_checked_and_read_within_the_limit() {
        let d = dir("text");
        let path = d.join("sub/a.txt");
        write_text(&path, "name=a\n", 64, |read| read == "name=a\n").unwrap();
        assert_eq!(read_text(&path, 64).unwrap(), "name=a\n");
        assert_eq!(read_checked(&path, 64).unwrap(), "name=a\n");
        assert!(matches!(read_text(&path, 3), Err(FileError::TooLarge)));
        assert!(matches!(read_checked(&path, 3), Err(FileError::TooLarge)));
        assert!(matches!(
            write_text(&path, "name=b\n", 64, |_| false),
            Err(FileError::Mismatch)
        ));
        assert!(matches!(
            write_text(&path, "name=long\n", 4, |_| true),
            Err(FileError::TooLarge)
        ));
        assert_eq!(read_text(&path, 64).unwrap(), "name=a\n", "前のまま");
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(matches!(read_checked(&path, 64), Err(FileError::NotText)));
        assert!(matches!(read_text(&path, 64), Err(FileError::Io(_))));
        remove(&path).unwrap();
        remove(&path).unwrap();
        assert_eq!(std::fs::read_dir(d.join("sub")).unwrap().count(), 0);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn numbered_files_load_in_order_skipping_extra_and_duplicate_names() {
        let d = dir("numbered");
        std::fs::create_dir_all(&d).unwrap();
        for (id, name) in [(3, "b"), (1, "a"), (7, "a"), (9, "c"), (12, "bad")] {
            std::fs::write(d.join(format!("p-{id}.txt")), name).unwrap();
        }
        std::fs::write(d.join("other.txt"), "x").unwrap();
        let id_of = |p: &Path| {
            p.file_name()?
                .to_str()?
                .strip_prefix("p-")?
                .strip_suffix(".txt")?
                .parse()
                .ok()
        };
        let read = |p: &Path| {
            let text = std::fs::read_to_string(p).unwrap();
            if text == "bad" {
                Err("bad")
            } else {
                Ok(text)
            }
        };
        let loaded = load_numbered(&d, id_of, 3, read, |t| t.as_str(), || "many", || "dup");
        assert_eq!(
            loaded.items,
            [(1, "a".to_owned()), (3, "b".into()), (9, "c".into())]
        );
        assert_eq!(
            loaded.problems,
            [("p-7.txt".to_owned(), "dup"), ("p-12.txt".into(), "many")]
        );
        assert_eq!(loaded.next_id, 13);
        let empty = load_numbered(
            &d.join("none"),
            id_of,
            3,
            read,
            |t| t.as_str(),
            || "",
            || "",
        );
        assert!(empty.items.is_empty() && empty.problems.is_empty());
        assert_eq!(empty.next_id, 1);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn names_get_a_number_and_modified_follows_the_current_entry() {
        assert_eq!(unused_name("筆", |_| false), "筆");
        assert_eq!(unused_name("筆", |n| n == "筆" || n == "筆 2"), "筆 3");
        assert!(is_modified(Some((&1, false)), true, &2));
        assert!(!is_modified(Some((&1, true)), true, &1));
        assert!(is_modified(Some((&1, true)), false, &1));
        assert!(!is_modified(Some((&1, false)), false, &2));
        assert!(!is_modified::<i32>(None, true, &2));
    }
}
