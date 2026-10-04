//! 利用者ごとのアプリ設定。文書・試験の AppState とは独立して読み書きする。
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

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

pub(crate) fn load(path: &Path) -> io::Result<Lang> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Lang::default()),
        Err(e) => return Err(e),
    };
    let mut text = String::new();
    file.take(4097).read_to_string(&mut text)?;
    if text.len() > 4096 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "settings too large"));
    }
    match text.trim() {
        "language=ja" => Ok(Lang::Ja),
        "language=en" => Ok(Lang::En),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "invalid language setting")),
    }
}

pub(crate) fn save(path: &Path, lang: Lang) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "settings directory missing"))?;
    std::fs::create_dir_all(parent)?;
    let pending = path.with_extension(format!("{}.pending", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&pending)?;
    let result = (|| {
        file.write_all(lang.pick(b"language=ja\n", b"language=en\n"))?;
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
    #[test]
    fn language_survives_restart_and_failed_replace_preserves_settings() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/settings-tests").join(std::process::id().to_string());
        let path = dir.join("settings.conf");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&path).unwrap(), Lang::Ja);
        for lang in [Lang::En, Lang::Ja, Lang::En] {
            save(&path, lang).unwrap();
            assert_eq!(load(&path).unwrap(), lang);
        }
        let pending = path.with_extension(format!("{}.pending", std::process::id()));
        std::fs::write(&pending, "busy").unwrap();
        assert!(save(&path, Lang::Ja).is_err());
        assert_eq!(load(&path).unwrap(), Lang::En);
        std::fs::write(&path, "language=unknown").unwrap();
        assert_eq!(load(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
        std::fs::write(&path, vec![b'a'; 4097]).unwrap();
        assert!(load(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
