//! 復旧の設定（書き置きの間隔・終わったストロークの数・残す世代の数・置き場）。言語の設定（`settings.conf`）とは別の
//! `recovery.conf`（同じフォルダ）に、`キー=値` を 1 行ずつ書く。範囲の外の値は、Unity 版の設定と同じく既定へ戻し、
//! 理由を返す（ファイルは、保存し直すまで触らない）。

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// 書き置きの間隔（秒）の範囲。
pub const INTERVAL_RANGE: (u32, u32) = (5, 600);
/// 残す世代の数の範囲（今と 1 つ前を含むので 2 以上）。
pub const KEEP_RANGE: (u32, u32) = (2, 1000);
/// ストロークの数の上限（0 は数では書かない）。
pub const MAX_STROKES: u32 = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoverySettings {
    /// 変更があってから（連続して描いているときは前の書き置きから）、書くまでの時間。
    pub interval_seconds: u32,
    /// 前の書き置きから、終わったストロークがこの数に達したら、時間を待たずに書く。0 は数では書かない。
    pub strokes_between: u32,
    /// 閉じた置き場に残す世代の数、と、この実行の置き場で残す世代の数。
    pub generations_to_keep: u32,
    /// 置き場。無ければ設定のフォルダの下の `recovery`。
    pub directory: Option<PathBuf>,
}

impl Default for RecoverySettings {
    fn default() -> Self {
        Self {
            interval_seconds: 15,
            strokes_between: 10,
            generations_to_keep: 3,
            directory: None,
        }
    }
}

/// 読み書きの失敗の理由（種類と OS の番号。画面が言語に合わせた短い文にする）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoReason {
    pub kind: io::ErrorKind,
    pub os: Option<i32>,
}
impl IoReason {
    pub fn of(error: &io::Error) -> Self {
        Self {
            kind: error.kind(),
            os: error.raw_os_error(),
        }
    }
    pub(crate) fn to_error(&self) -> io::Error {
        match self.os {
            Some(code) => io::Error::from_raw_os_error(code),
            None => io::Error::from(self.kind),
        }
    }
}

/// 起動で気づいたこと（設定のファイルの読み込み・前の実行の後片付け。画面は種類から短い文を作る）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// キーの値が範囲の外（キー、書いてあった値、範囲）。既定へ戻した。
    OutOfRange {
        key: &'static str,
        value: String,
        range: (u32, u32),
    },
    /// 置き場が絶対パスではない。既定へ戻した。
    RelativeDirectory(String),
    /// 設定のファイルを読めない（権限・大きすぎる・UTF-8 でないなど）。既定の間隔で動くが、利用者が選んだ世代の数・置き場は
    /// 分からないので、世代は整理せず、設定のファイルにも書かない。
    Unreadable(IoReason),
    /// 前の実行の印（`session.lock`）を片付けられなかった（権限・ディスクの空きなど）。その世代は落ちた実行のものとして一覧に残る。
    PreviousRun(IoReason),
}

impl RecoverySettings {
    /// 設定のファイル（言語の設定と同じフォルダの `recovery.conf`）。
    pub(crate) fn path() -> Option<PathBuf> {
        crate::settings::path().map(|p| p.with_file_name("recovery.conf"))
    }

    /// 置き場の根。設定の `directory` か、設定のファイル（`conf`）と同じフォルダの下の `recovery`。どちらも決まらなければ None
    /// （復旧を使わない）。
    pub(crate) fn root_beside(&self, conf: Option<&Path>) -> Option<PathBuf> {
        self.directory
            .clone()
            .or_else(|| conf.and_then(|p| p.parent().map(|d| d.join("recovery"))))
    }

    /// ファイルを読む。無ければ既定。読めなければ Err（呼ぶ側は `Problem::Unreadable` で知らせ、世代を整理せず、ファイルを
    /// 上書きしない）。範囲の外は既定へ戻して `Problem` を返す。
    pub fn load(path: &Path) -> io::Result<(Self, Vec<Problem>)> {
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((Self::default(), vec![])),
            Err(e) => return Err(e),
        };
        let mut text = String::new();
        file.take(4097).read_to_string(&mut text)?;
        if text.len() > 4096 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "settings too large"));
        }
        Ok(Self::parse(&text))
    }

    pub(crate) fn parse(text: &str) -> (Self, Vec<Problem>) {
        let mut settings = Self::default();
        let mut problems = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            let mut number = |name: &'static str, range: (u32, u32), slot: &mut u32| match value
                .parse::<u32>()
            {
                Ok(n) if (range.0..=range.1).contains(&n) => *slot = n,
                _ => problems.push(Problem::OutOfRange {
                    key: name,
                    value: value.to_owned(),
                    range,
                }),
            };
            match key {
                "interval" => number("interval", INTERVAL_RANGE, &mut settings.interval_seconds),
                "generations" => number("generations", KEEP_RANGE, &mut settings.generations_to_keep),
                "strokes" => number("strokes", (0, MAX_STROKES), &mut settings.strokes_between),
                "directory" if !value.is_empty() => {
                    if Path::new(value).is_absolute() {
                        settings.directory = Some(PathBuf::from(value));
                    } else {
                        problems.push(Problem::RelativeDirectory(value.to_owned()));
                    }
                }
                _ => {}
            }
        }
        (settings, problems)
    }

    /// 書く（検証した一時ファイルから 1 回の置き換え。書けなければ元のまま）。
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "settings directory missing"))?;
        std::fs::create_dir_all(parent)?;
        let pending = path.with_extension(format!("{}.pending", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)?;
        let mut text = format!(
            "interval={}\nstrokes={}\ngenerations={}\n",
            self.interval_seconds, self.strokes_between, self.generations_to_keep
        );
        if let Some(dir) = &self.directory {
            text.push_str(&format!("directory={}\n", dir.display()));
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_unity_settings() {
        let d = RecoverySettings::default();
        assert_eq!((d.interval_seconds, d.generations_to_keep), (15, 3));
        assert!(d.directory.is_none());
    }

    #[test]
    fn values_out_of_range_fall_back_to_the_default_with_a_reason() {
        let (s, problems) = RecoverySettings::parse(
            "interval=3\ngenerations=1\nstrokes=5000\ndirectory=relative/dir\nunknown=1\nnonsense\n",
        );
        assert_eq!(s, RecoverySettings::default());
        assert_eq!(problems.len(), 4);
        assert!(matches!(
            problems[0],
            Problem::OutOfRange { key: "interval", range: (5, 600), .. }
        ));
        assert_eq!(problems[3], Problem::RelativeDirectory("relative/dir".into()));
        let (s, problems) = RecoverySettings::parse("interval=600\ngenerations=1000\nstrokes=0\n");
        assert!(problems.is_empty());
        assert_eq!(
            (s.interval_seconds, s.generations_to_keep, s.strokes_between),
            (600, 1000, 0)
        );
        let (s, _) = RecoverySettings::parse("interval=abc\ninterval=7");
        assert_eq!(s.interval_seconds, 7, "壊れた行のあとの正しい行は読む");
    }

    #[test]
    fn settings_survive_a_save_and_a_failed_replace_keeps_the_file() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/recovery-settings-tests")
            .join(std::process::id().to_string());
        let path = dir.join("recovery.conf");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(RecoverySettings::load(&path).unwrap().0, RecoverySettings::default());
        let custom = RecoverySettings {
            interval_seconds: 30,
            strokes_between: 0,
            generations_to_keep: 7,
            directory: Some(dir.join("置き場")),
        };
        custom.save(&path).unwrap();
        let (loaded, problems) = RecoverySettings::load(&path).unwrap();
        assert_eq!(loaded, custom);
        assert!(problems.is_empty());
        let pending = path.with_extension(format!("{}.pending", std::process::id()));
        std::fs::write(&pending, "busy").unwrap();
        assert!(RecoverySettings::default().save(&path).is_err());
        assert_eq!(RecoverySettings::load(&path).unwrap().0, custom);
        std::fs::write(&path, vec![b'a'; 4097]).unwrap();
        assert!(RecoverySettings::load(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
