//! 利用者ごとのアプリ設定。文書・試験の AppState とは独立して読み書きする。
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use yolu_io::{BackupKeep, MAX_BACKUPS_TO_KEEP};

use crate::lang::Lang;

pub(crate) fn path() -> Option<PathBuf> {
    config_path(std::env::consts::OS, |key| std::env::var_os(key).map(PathBuf::from))
}

fn config_path(os: &str, env: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    let absolute = |key| env(key).filter(|p| p.is_absolute());
    let base = match os {
        "windows" => absolute("APPDATA"),
        "macos" => absolute("HOME").map(|p| p.join("Library/Application Support")),
        _ => absolute("XDG_CONFIG_HOME")
            .or_else(|| absolute("HOME").map(|p| p.join(".config"))),
    }?;
    Some(base.join("YoluPainter").join("settings.conf"))
}

/// 利用者ごとの設定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    pub lang: Lang,
    /// 上書き保存で置き換えた前の版（退避）をいくつ残すか。
    pub backups: BackupKeep,
}

impl Default for Settings {
    fn default() -> Self {
        Self { lang: Lang::default(), backups: BackupKeep::All }
    }
}

/// 読んだときに既定へ戻したもの（画面は種類から短い理由を作る）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Problem {
    /// ファイルを読めない（読み込みの失敗・大きすぎる・`キー=値` の形ではない行）。設定は全部既定。
    Unreadable,
    /// 知らない言語（書いてあった値）。
    Language(String),
    /// 退避を残す数が `all` でも 0〜上限の数でもない（書いてあった値）。すべて残す。
    Backups(String),
}

impl Problem {
    /// 状態の帯の短い文。
    pub(crate) fn text(&self, lang: Lang) -> String {
        match self {
            Self::Unreadable => lang.pick("設定を読めません。", "Cannot read the settings.").into(),
            Self::Language(_) => lang.pick("言語の設定を読めません。", "Cannot read the language setting.").into(),
            Self::Backups(value) => {
                let shown: String = value.chars().take(12).collect();
                lang.pick(
                    format!("退避を残す数の設定が正しくありません（{shown}）。すべて残します。"),
                    format!("Invalid Backups to Keep setting ({shown}); keeping all."),
                )
            }
        }
    }
}

/// 設定のファイルを読む。無ければ既定。`キー=値` を 1 行ずつで、知らないキーは読み飛ばす。値が正しくない項目は既定へ戻して
/// `Problem` を返す（ファイルは、設定を変えて書き直すまで触らない）。
pub(crate) fn load(path: &Path) -> (Settings, Vec<Problem>) {
    match read(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (Settings::default(), Vec::new()),
        Err(_) => (Settings::default(), vec![Problem::Unreadable]),
    }
}

fn read(path: &Path) -> io::Result<String> {
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take(4097).read_to_string(&mut text)?;
    if text.len() > 4096 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "settings too large"));
    }
    Ok(text)
}

fn parse(text: &str) -> (Settings, Vec<Problem>) {
    let mut settings = Settings::default();
    let mut problems = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Some((key, value)) = line.split_once('=') else {
            return (Settings::default(), vec![Problem::Unreadable]);
        };
        let value = value.trim();
        match key.trim() {
            "language" => match value {
                "ja" => settings.lang = Lang::Ja,
                "en" => settings.lang = Lang::En,
                _ => problems.push(Problem::Language(value.to_owned())),
            },
            "backups" => match parse_backups(value) {
                Some(keep) => settings.backups = keep,
                None => problems.push(Problem::Backups(value.to_owned())),
            },
            _ => {}
        }
    }
    (settings, problems)
}

/// `all`（すべて残す）か、0〜上限の数。
fn parse_backups(value: &str) -> Option<BackupKeep> {
    if value == "all" {
        return Some(BackupKeep::All);
    }
    let n: u32 = value.parse().ok()?;
    (n <= MAX_BACKUPS_TO_KEEP).then_some(BackupKeep::Count(n))
}

/// 書く内容。言語は常に、退避は既定（すべて残す）でなければ。既定のときは書かないので、退避を使わない間は今までと同じ中身。
fn render(settings: &Settings) -> String {
    let mut text = format!("language={}\n", settings.lang.pick("ja", "en"));
    match settings.backups {
        BackupKeep::All => {}
        BackupKeep::Count(n) => text += &format!("backups={}\n", n.min(MAX_BACKUPS_TO_KEEP)),
    }
    text
}

pub(crate) fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "settings directory missing"))?;
    std::fs::create_dir_all(parent)?;
    let pending = path.with_extension(format!("{}.pending", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&pending)?;
    let result = (|| {
        file.write_all(render(settings).as_bytes())?;
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
    #[test]
    fn config_directories_follow_each_os_and_reject_relative_xdg() {
        let base = std::env::current_dir().unwrap();
        let home = base.join("example");
        let env = |key: &str| match key {
            "HOME" => Some(home.clone()),
            "APPDATA" => Some(base.join("roaming")),
            "XDG_CONFIG_HOME" => Some(PathBuf::from("relative")),
            _ => None,
        };
        assert_eq!(config_path("linux", env).unwrap(), home.join(".config/YoluPainter/settings.conf"));
        assert_eq!(config_path("macos", env).unwrap(), home.join("Library/Application Support/YoluPainter/settings.conf"));
        assert_eq!(config_path("windows", env).unwrap(), base.join("roaming/YoluPainter/settings.conf"));
        assert!(config_path("linux", |_| None).is_none());
        assert_eq!(config_path("linux", |_| Some(base.join("config"))).unwrap(), base.join("config/YoluPainter/settings.conf"));
    }
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/settings-tests").join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }
    fn settings(lang: Lang, backups: BackupKeep) -> Settings {
        Settings { lang, backups }
    }
    #[test]
    fn language_survives_restart_and_failed_replace_preserves_settings() {
        let dir = temp_dir("language");
        let path = dir.join("settings.conf");
        assert_eq!(load(&path), (Settings::default(), vec![]));
        assert_eq!(Settings::default().lang, Lang::Ja);
        for lang in [Lang::En, Lang::Ja, Lang::En] {
            save(&path, &settings(lang, BackupKeep::All)).unwrap();
            assert_eq!(load(&path), (settings(lang, BackupKeep::All), vec![]));
        }
        let pending = path.with_extension(format!("{}.pending", std::process::id()));
        std::fs::write(&pending, "busy").unwrap();
        assert!(save(&path, &settings(Lang::Ja, BackupKeep::All)).is_err());
        assert_eq!(load(&path).0.lang, Lang::En);
        std::fs::write(&path, "language=unknown").unwrap();
        assert_eq!(load(&path), (Settings::default(), vec![Problem::Language("unknown".into())]));
        std::fs::write(&path, vec![b'a'; 4097]).unwrap();
        assert_eq!(load(&path), (Settings::default(), vec![Problem::Unreadable]));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn backups_to_keep_is_written_only_when_it_is_not_the_default_and_restored() {
        let dir = temp_dir("backups");
        let path = dir.join("settings.conf");
        // 既定（すべて残す）は書かない。今までのファイルと同じ中身
        save(&path, &settings(Lang::En, BackupKeep::All)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n");
        for keep in [BackupKeep::Count(0), BackupKeep::Count(1), BackupKeep::Count(37), BackupKeep::Count(MAX_BACKUPS_TO_KEEP)] {
            save(&path, &settings(Lang::Ja, keep)).unwrap();
            assert_eq!(load(&path), (settings(Lang::Ja, keep), vec![]), "{keep:?}");
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=ja\nbackups=1000\n");
        // 選び直して「すべて」に戻すと、行は消える
        save(&path, &settings(Lang::Ja, BackupKeep::All)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=ja\n");
        // 上限を超えて渡されても、書くのは上限
        save(&path, &settings(Lang::Ja, BackupKeep::Count(5000))).unwrap();
        assert_eq!(load(&path).0.backups, BackupKeep::Count(MAX_BACKUPS_TO_KEEP));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn broken_values_fall_back_to_the_default_with_a_reason_and_the_rest_is_kept() {
        let (read, problems) = parse("language=en\nbackups=many\n");
        assert_eq!(read, settings(Lang::En, BackupKeep::All));
        assert_eq!(problems, [Problem::Backups("many".into())]);
        // 範囲の外・負・小数・空・上限の次
        for bad in ["-1", "1001", "5000", "2.5", "", "ALL", "0x10", "99999999999999999999"] {
            let (read, problems) = parse(&format!("language=ja\nbackups={bad}\n"));
            assert_eq!(read.backups, BackupKeep::All, "{bad:?}");
            assert_eq!(problems, [Problem::Backups(bad.into())], "{bad:?}");
        }
        // 両方が壊れていれば両方の理由。書いていない項目は既定で、理由は出さない
        let (read, problems) = parse("language=fr\nbackups=-3\n");
        assert_eq!(read, Settings::default());
        assert_eq!(problems, [Problem::Language("fr".into()), Problem::Backups("-3".into())]);
        assert_eq!(parse("backups=7\n"), (settings(Lang::Ja, BackupKeep::Count(7)), vec![]));
        assert_eq!(parse("language=en\nbackups=all\n"), (settings(Lang::En, BackupKeep::All), vec![]));
        // 空白・空行・知らないキー（読み飛ばす。新しい版が足した項目で壊れない）・後ろの行が勝つ
        assert_eq!(parse("\n  language = en \n future=1\n backups = 12 \nbackups=13\n"), (settings(Lang::En, BackupKeep::Count(13)), vec![]));
        // `キー=値` ではない行は、読めないファイル（全部既定）
        assert_eq!(parse("language=en\njunk\n"), (Settings::default(), vec![Problem::Unreadable]));
    }
    #[test]
    fn problems_have_short_reasons_in_both_languages_without_the_other_one() {
        for lang in Lang::ALL {
            let texts: Vec<String> = [Problem::Unreadable, Problem::Language("x".into()), Problem::Backups("5000".into())].iter().map(|p| p.text(lang)).collect();
            for (i, a) in texts.iter().enumerate() {
                assert_eq!(a.is_ascii(), lang == Lang::En, "{lang:?} {a}");
                assert!(texts.iter().skip(i + 1).all(|b| a != b));
            }
            assert!(texts[2].contains("5000"));
        }
        // 長い値は切って、帯を溢れさせない
        let long = Problem::Backups("9".repeat(500)).text(Lang::En);
        assert!(long.len() < 100, "{long}");
    }
}
