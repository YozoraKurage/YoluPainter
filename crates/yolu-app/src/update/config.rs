//! 「起動時に更新を確かめる」の選択の保存。設定のフォルダの `update.conf`（1 行 `check_on_startup=on|off`）。
//! 言語の設定（`settings.conf`）とは別のファイルにする: 旧い版が `settings.conf` の知らない行を「言語の設定が読めない」と
//! 扱うので、同じファイルに足すと、旧い版へ戻したときに言語の知らせが出てしまう。
//! 無い・読めない選択は「まだ聞いていない」として扱い、聞くまで通信しない。

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// 起動時に更新を確かめるか。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preference {
    /// まだ聞いていない（聞くまで、通信しない）。
    #[default]
    Unset,
    On,
    Off,
}

/// 設定ファイル（`settings.conf`）と同じフォルダの `update.conf`。
pub fn path_for(settings: &Path) -> Option<PathBuf> {
    Some(settings.parent()?.join("update.conf"))
}

pub fn load(path: &Path) -> io::Result<Preference> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Preference::Unset),
        Err(e) => return Err(e),
    };
    let mut text = String::new();
    file.take(1025).read_to_string(&mut text)?;
    if text.len() > 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "update.conf too large",
        ));
    }
    match text.trim() {
        "check_on_startup=on" => Ok(Preference::On),
        "check_on_startup=off" => Ok(Preference::Off),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid update setting",
        )),
    }
}

/// 一時ファイルに書いてから 1 回の rename で置く（書きかけの設定を残さない）。
pub fn save(path: &Path, on: bool) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "settings directory missing"))?;
    std::fs::create_dir_all(parent)?;
    let pending = path.with_extension(format!("{}.pending", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)?;
    let result = (|| {
        file.write_all(if on {
            b"check_on_startup=on\n"
        } else {
            b"check_on_startup=off\n"
        })?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&pending, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&pending);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/update-config-tests")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn sits_beside_the_settings_file() {
        assert_eq!(
            path_for(Path::new("/a/YoluPainter/settings.conf")).unwrap(),
            Path::new("/a/YoluPainter/update.conf")
        );
    }

    #[test]
    fn choice_survives_restart_and_is_unset_until_made() {
        let dir = scratch("roundtrip");
        let path = dir.join("update.conf");
        assert_eq!(load(&path).unwrap(), Preference::Unset);
        save(&path, true).unwrap();
        assert_eq!(load(&path).unwrap(), Preference::On);
        save(&path, false).unwrap();
        assert_eq!(load(&path).unwrap(), Preference::Off);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unreadable_choices_are_errors_and_a_failed_save_keeps_the_old_one() {
        let dir = scratch("broken");
        let path = dir.join("update.conf");
        save(&path, true).unwrap();
        // 書きかけの一時ファイルが道をふさいでいる: 保存は失敗し、前の選択が残る。
        let pending = path.with_extension(format!("{}.pending", std::process::id()));
        std::fs::write(&pending, "busy").unwrap();
        assert!(save(&path, false).is_err());
        assert_eq!(load(&path).unwrap(), Preference::On);
        std::fs::remove_file(&pending).unwrap();
        for bad in [
            "check_on_startup=maybe",
            "language=ja",
            "",
            "check_on_startup=on\ncheck_on_startup=off",
        ] {
            std::fs::write(&path, bad).unwrap();
            assert_eq!(
                load(&path).unwrap_err().kind(),
                io::ErrorKind::InvalidData,
                "{bad:?}"
            );
        }
        std::fs::write(&path, vec![b'a'; 2000]).unwrap();
        assert!(load(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
