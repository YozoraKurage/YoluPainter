//! 更新の設定の保存。設定のフォルダの `update.conf`（1 行 1 項目）:
//!
//! ```text
//! check_on_startup=on|off   起動時に更新を確かめるか（まだ聞いていなければ、この行は無い）
//! use_beta=on               試験版も更新の候補にするか（入のときだけ書く。切は行が無いのと同じ）
//! ```
//!
//! 旧い版の `check_on_startup` 1 行だけのファイルは、そのまま読める（試験版は切）。旧い版が試験版の行つきのファイルを読むと、
//! 設定が読めない扱いになり、初回の問いがもう一度出る（通信はしない）。
//! 言語の設定（`settings.conf`）とは別のファイルにする: 旧い版が `settings.conf` の知らない行を「言語の設定が読めない」と
//! 扱うので、同じファイルに足すと、旧い版へ戻したときに言語の知らせが出てしまう。
//! 無い・読めない選択は「まだ聞いていない」（試験版は切）として扱い、聞くまで通信しない。
//! 試験版の行を別のファイルにしなかったのは、アンインストーラーが消すファイルを名前で挙げている（`installer/yolupainter.nsi`・
//! `docs/INSTALL.md` の表）ため。増やすと、その 3 か所と試験を揃える必要がある。

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

/// `update.conf` に保存する値の全部。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stored {
    pub check: Preference,
    /// 試験版も更新の候補にする（設定「試験版を使う」）。
    pub beta: bool,
}

fn invalid(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

/// 無ければ既定（まだ聞いていない・試験版は切）。読めなければ `InvalidData`（呼び出し側は既定として扱う）。
pub fn load(path: &Path) -> io::Result<Stored> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Stored::default()),
        Err(e) => return Err(e),
    };
    let mut text = String::new();
    file.take(1025).read_to_string(&mut text)?;
    if text.len() > 1024 {
        return Err(invalid("update.conf too large"));
    }
    let (mut check, mut beta) = (None, None);
    let mut lines = 0;
    for line in text.trim().lines() {
        lines += 1;
        match line.trim() {
            "check_on_startup=on" if check.is_none() => check = Some(Preference::On),
            "check_on_startup=off" if check.is_none() => check = Some(Preference::Off),
            "use_beta=on" if beta.is_none() => beta = Some(true),
            "use_beta=off" if beta.is_none() => beta = Some(false),
            _ => return Err(invalid("invalid update setting")),
        }
    }
    if lines == 0 {
        return Err(invalid("invalid update setting"));
    }
    Ok(Stored {
        check: check.unwrap_or(Preference::Unset),
        beta: beta.unwrap_or(false),
    })
}

/// 一時ファイルに書いてから 1 回の rename で置く（書きかけの設定を残さない）。
/// 何も設定していない（まだ聞いていない・試験版は切）ときは、ファイルを置かない（あれば消す）。
pub fn save(path: &Path, stored: &Stored) -> io::Result<()> {
    let mut text = String::new();
    match stored.check {
        Preference::On => text.push_str("check_on_startup=on\n"),
        Preference::Off => text.push_str("check_on_startup=off\n"),
        Preference::Unset => {}
    }
    if stored.beta {
        text.push_str("use_beta=on\n");
    }
    if text.is_empty() {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
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
        file.write_all(text.as_bytes())?;
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

    fn checked(check: Preference) -> Stored {
        Stored { check, beta: false }
    }

    #[test]
    fn choice_survives_restart_and_is_unset_until_made() {
        let dir = scratch("roundtrip");
        let path = dir.join("update.conf");
        assert_eq!(load(&path).unwrap(), Stored::default());
        save(&path, &checked(Preference::On)).unwrap();
        assert_eq!(load(&path).unwrap(), checked(Preference::On));
        save(&path, &checked(Preference::Off)).unwrap();
        assert_eq!(load(&path).unwrap(), checked(Preference::Off));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_old_one_line_file_is_read_as_it_was_and_the_beta_line_is_written_only_when_on() {
        let dir = scratch("compat");
        let path = dir.join("update.conf");
        std::fs::create_dir_all(&dir).unwrap();
        // 旧い版が書いた 1 行は、そのまま読める（試験版は切）
        for (text, want) in [
            ("check_on_startup=on\n", Preference::On),
            ("check_on_startup=off", Preference::Off),
            ("  check_on_startup=on  \r\n", Preference::On),
        ] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(load(&path).unwrap(), checked(want), "{text:?}");
        }
        // 試験版を切のまま保存すると、旧い版と同じ 1 行のまま（旧い版が読める）
        save(&path, &checked(Preference::On)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "check_on_startup=on\n"
        );
        // 入のときだけ 2 行目が付く。保存し直しても、もう一方の値を落とさない
        let both = Stored {
            check: Preference::On,
            beta: true,
        };
        save(&path, &both).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "check_on_startup=on\nuse_beta=on\n"
        );
        assert_eq!(load(&path).unwrap(), both);
        // 順番は問わない・切の行も読める
        for text in [
            "use_beta=on\ncheck_on_startup=on\n",
            "check_on_startup=on\nuse_beta=on",
        ] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(load(&path).unwrap(), both, "{text:?}");
        }
        std::fs::write(&path, "check_on_startup=off\nuse_beta=off\n").unwrap();
        assert_eq!(load(&path).unwrap(), checked(Preference::Off));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_beta_setting_alone_keeps_the_startup_question_unasked_and_nothing_set_leaves_no_file() {
        let dir = scratch("beta-alone");
        let path = dir.join("update.conf");
        // まだ聞いていない人が試験版だけを入れる: 問いはまだ聞いていないまま
        let beta_only = Stored {
            check: Preference::Unset,
            beta: true,
        };
        save(&path, &beta_only).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "use_beta=on\n");
        assert_eq!(load(&path).unwrap(), beta_only);
        // 何も設定していない状態に戻すと、ファイルは無くなる（無いファイルも、消えていても失敗にしない）
        save(&path, &Stored::default()).unwrap();
        assert!(!path.exists());
        save(&path, &Stored::default()).unwrap();
        assert_eq!(load(&path).unwrap(), Stored::default());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unreadable_choices_are_errors_and_a_failed_save_keeps_the_old_one() {
        let dir = scratch("broken");
        let path = dir.join("update.conf");
        save(&path, &checked(Preference::On)).unwrap();
        // 書きかけの一時ファイルが道をふさいでいる: 保存は失敗し、前の選択が残る。
        let pending = path.with_extension(format!("{}.pending", std::process::id()));
        std::fs::write(&pending, "busy").unwrap();
        assert!(save(&path, &checked(Preference::Off)).is_err());
        assert_eq!(load(&path).unwrap(), checked(Preference::On));
        std::fs::remove_file(&pending).unwrap();
        for bad in [
            "check_on_startup=maybe",
            "language=ja",
            "",
            "\n",
            "check_on_startup=on\ncheck_on_startup=off",
            "use_beta=on\nuse_beta=off",
            "use_beta=yes",
            "check_on_startup=on\nunknown=1",
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
