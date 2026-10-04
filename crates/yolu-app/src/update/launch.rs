//! OS ごとの口: インストールした Windows かの判定、落としたインストーラーの置き場と起動、リリースのページを開く。

use std::io;
use std::path::{Path, PathBuf};

/// この実行ファイルが、インストーラーで入れた物か。インストーラーは実行ファイルの隣に `uninstall.exe` を置く
/// （zip を展開しただけの物には無い）。zip の物は自分でインストーラーを走らせない（別の場所へもう 1 つ入ってしまう）。
pub fn is_installed_copy(exe: &Path) -> bool {
    exe.parent()
        .is_some_and(|dir| dir.join("uninstall.exe").is_file())
}

/// 落としたインストーラーを置く利用者ごとのフォルダ（ほかの利用者が差し替えられない場所）。
/// アンインストーラーもここを片付ける（installer/yolupainter.nsi の `updates`）。
pub fn staging_dir() -> Option<PathBuf> {
    let absolute = |key: &str| {
        std::env::var_os(key)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let base = if cfg!(windows) {
        absolute("LOCALAPPDATA")?
    } else {
        absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|home| home.join(".cache")))?
    };
    Some(base.join("YoluPainter").join("updates"))
}

/// 開く URL は https の、見える ASCII だけ（OS の「開く」へ渡すので、変な文字を通さない）。
fn checked_url(url: &str) -> io::Result<&str> {
    if url.starts_with("https://") && url.bytes().all(|b| b.is_ascii_graphic()) {
        Ok(url)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not an https URL",
        ))
    }
}

/// インストーラーを無音で走らせる（`/S`）。入れ終わったらアプリを起こし直す（`/RUN`）。アプリが閉じるのを、インストーラーが待つ。
/// 呼んだアプリが終わっても続くよう、切り離して起動する。
#[cfg(windows)]
pub fn run_installer(path: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    Command::new(path)
        .args(["/S", "/RUN"])
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

#[cfg(not(windows))]
pub fn run_installer(_path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the installer is for Windows",
    ))
}

#[cfg(windows)]
pub fn open_page(url: &str) -> io::Result<()> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let url: Vec<u16> = checked_url(url)?
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: 文字列は NUL 終端の UTF-16 で、この呼び出しの間生きている。
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // 32 より大きければ成功（ShellExecute の決まり）。
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(windows))]
pub fn open_page(url: &str) -> io::Result<()> {
    use std::process::{Command, Stdio};
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut child = Command::new(opener)
        .arg(checked_url(url)?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // 終わりを受けておく（ゾンビを残さない）。
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_urls_are_opened() {
        assert!(checked_url("https://github.com/o/r/releases/tag/v1.0.0").is_ok());
        for bad in [
            "http://github.com",
            "file:///etc/passwd",
            "calc.exe",
            "https://a b",
            "https://日本",
            "",
        ] {
            assert!(checked_url(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn an_installed_copy_is_the_one_with_an_uninstaller_beside_it() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/update-launch-tests")
            .join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("yolupainter.exe");
        std::fs::write(&exe, b"exe").unwrap();
        assert!(!is_installed_copy(&exe));
        std::fs::write(dir.join("uninstall.exe"), b"u").unwrap();
        assert!(is_installed_copy(&exe));
        assert!(!is_installed_copy(Path::new("yolupainter.exe")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_staging_folder_is_per_user_and_named_for_the_uninstaller() {
        if let Some(dir) = staging_dir() {
            assert!(dir.ends_with("YoluPainter/updates") || dir.ends_with(r"YoluPainter\updates"));
            assert!(dir.is_absolute());
        }
    }
}
