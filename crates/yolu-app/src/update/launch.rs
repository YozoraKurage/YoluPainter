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
/// macOS は何も落とさない（新しい版を知らせて、リリースのページを開くだけ）ので、置き場を持たない。
pub fn staging_dir() -> Option<PathBuf> {
    staging_for(std::env::consts::OS, |key| {
        std::env::var_os(key).map(PathBuf::from)
    })
}

fn staging_for(os: &str, env: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    let absolute = |key: &str| env(key).filter(|p| p.is_absolute());
    let base = match os {
        "windows" => absolute("LOCALAPPDATA")?,
        "macos" => return None,
        _ => absolute("XDG_CACHE_HOME")
            .or_else(|| absolute("HOME").map(|home| home.join(".cache")))?,
    };
    Some(base.join("YoluPainter").join("updates"))
}

/// この実行ファイルを動かしている、自分以外の起動があるか。インストーラーは、動いている exe を書き換えられず待つので、
/// 別のウィンドウが開いたまま更新を始めると、更新したウィンドウだけが閉じて何も入らない。更新を始める前に、これで断る。
///
/// 見つけ方は、Windows ではプロセスの実行ファイルの完全なパスを比べる（自分のプロセスは除く）。インストーラーが待つ理由
/// （同じ exe のファイルを別のプロセスが使っている）と同じ物差しで、別の場所に入れた版（ポータブル版など）の起動は数えない。
/// 起動ごとに名前付きの印（ミューテックスなど）を持つ方式は採らなかった。印は「ある・無い」しか分からず、数えて自分を除くには
/// 別の仕組み（共有のカウンター・ロックのファイル）が要り、強制終了の後始末が増える。パスの比較は OS が持つ事実を読むだけで、後始末が無い。
/// 見られないプロセス（権限が足りない・終わりかけ）は数えない。一覧を取れなかったときは「いない」とみなす（更新を止め続けない。
/// 取りこぼしても、インストーラーが待ちの上限で諦めたあと、今の exe を起こし直す）。
pub fn another_instance_running() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    runs_elsewhere(&exe, std::process::id(), running_processes())
}

/// `processes`（番号と実行ファイルのパス。パスが分からない物は None）に、自分（`own`）以外で `exe` を動かしている物があるか。
fn runs_elsewhere(
    exe: &Path,
    own: u32,
    processes: impl IntoIterator<Item = (u32, Option<PathBuf>)>,
) -> bool {
    processes
        .into_iter()
        .any(|(id, path)| id != own && path.is_some_and(|path| same_file(exe, &path)))
}

/// 同じファイルを指すパスか。名前が違えばファイルを見ずに断り、同じ名前なら実体のパス（シンボリックリンク・8.3 名などをたどった形）で
/// 比べる。実体が取れないときは、見かけのパスを（Windows は大小を区別せず `\\?\` と `/` を揃えて）比べる。
fn same_file(a: &Path, b: &Path) -> bool {
    if a.file_name().map(file_key) != b.file_name().map(file_key) {
        return false;
    }
    if let (Ok(a), Ok(b)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        return path_key(&a) == path_key(&b);
    }
    path_key(a) == path_key(b)
}

fn file_key(name: &std::ffi::OsStr) -> String {
    let name = name.to_string_lossy();
    if cfg!(windows) {
        name.to_lowercase()
    } else {
        name.into_owned()
    }
}

fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
        text.replace('/', "\\").to_lowercase()
    } else {
        text.into_owned()
    }
}

/// 動いているプロセスの番号と、実行ファイルの完全なパス（取れない物は None）。
#[cfg(windows)]
fn running_processes() -> Vec<(u32, Option<PathBuf>)> {
    use windows::Win32::System::ProcessStatus::EnumProcesses;
    let mut ids = vec![0u32; 1024];
    loop {
        let bytes = (ids.len() * std::mem::size_of::<u32>()) as u32;
        let mut needed = 0u32;
        // SAFETY: ids は bytes バイトぶんの書き込み先で、needed はこの呼び出しの間生きている。
        if unsafe { EnumProcesses(ids.as_mut_ptr(), bytes, &mut needed) }.is_err() {
            return Vec::new();
        }
        if needed < bytes {
            ids.truncate(needed as usize / std::mem::size_of::<u32>());
            break;
        }
        // 一覧が書き先いっぱいなら、まだ続きがあるかもしれない。倍にして取り直す。
        ids.resize(ids.len() * 2, 0);
    }
    ids.into_iter()
        .filter(|id| *id != 0)
        .map(|id| (id, process_image_path(id)))
        .collect()
}

/// プロセスの実行ファイルの完全なパス。開けない（権限・終了済み）・取れないときは None。
#[cfg(windows)]
fn process_image_path(id: u32) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: 開いたハンドルはこの関数の中で必ず閉じる。
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, id) }.ok()?;
    // Win32 のパスの上限（32767 文字）まで取れる大きさ。
    let mut buffer = vec![0u16; 32768];
    let mut length = buffer.len() as u32;
    // SAFETY: buffer は length 文字ぶんの書き込み先。成功すると length は終端を除く文字数になる。
    let queried = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    };
    // SAFETY: 上で開いたハンドル。
    let _ = unsafe { CloseHandle(process) };
    queried.ok()?;
    Some(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..length as usize],
    )))
}

#[cfg(target_os = "linux")]
fn running_processes() -> Vec<(u32, Option<PathBuf>)> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let id: u32 = entry.file_name().to_str()?.parse().ok()?;
            Some((id, std::fs::read_link(entry.path().join("exe")).ok()))
        })
        .collect()
}

/// 一覧を取れない OS は「いない」とみなす（更新の入れ方がインストーラーになるのは Windows だけ）。
#[cfg(not(any(windows, target_os = "linux")))]
fn running_processes() -> Vec<(u32, Option<PathBuf>)> {
    Vec::new()
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

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/update-launch-tests")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn another_instance_is_a_process_other_than_this_one_running_the_same_exe() {
        let dir = temp_dir("peers");
        let exe = dir.join("yolupainter.exe");
        let copy = dir.join("portable").join("yolupainter.exe");
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"exe").unwrap();
        std::fs::write(&copy, b"exe").unwrap();
        let elsewhere = dir.join("other.exe");
        std::fs::write(&elsewhere, b"other").unwrap();
        let own = 100;
        let run = |list: Vec<(u32, Option<PathBuf>)>| runs_elsewhere(&exe, own, list);
        // 自分だけ・一覧が空・パスが取れない・別の exe・別の場所の同じ名前（ポータブル版）は、別の起動に数えない
        assert!(!run(vec![]));
        assert!(!run(vec![(own, Some(exe.clone()))]));
        assert!(!run(vec![(7, None)]));
        assert!(!run(vec![(7, Some(elsewhere.clone()))]));
        assert!(!run(vec![(7, Some(copy.clone()))]));
        // 自分のほかに、同じ exe を動かしている物が 1 つでもあれば、別の起動がある
        assert!(run(vec![(own, Some(exe.clone())), (7, Some(exe.clone()))]));
        assert!(run(vec![
            (7, None),
            (8, Some(elsewhere)),
            (9, Some(exe.clone()))
        ]));
        // 同じファイルを指す別の書き方（. や .. を通る）も同じ exe
        let roundabout = dir.join("portable").join("..").join("yolupainter.exe");
        assert!(run(vec![(7, Some(roundabout))]));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_are_compared_without_case_and_separator_differences() {
        // 実体の無いパスでも、見かけの違い（大小・区切り・\\?\）だけなら同じとみなす
        let a = Path::new(r"C:\Users\me\AppData\Local\Programs\YoluPainter\yolupainter.exe");
        assert!(same_file(
            a,
            Path::new(r"c:/users/ME/appdata/local/programs/yolupainter/YOLUPAINTER.EXE")
        ));
        assert!(same_file(
            a,
            Path::new(r"\\?\C:\Users\me\AppData\Local\Programs\YoluPainter\yolupainter.exe")
        ));
        assert!(!same_file(
            a,
            Path::new(r"C:\Users\me\Desktop\yolupainter.exe")
        ));
    }

    /// 本物の別プロセスを立てて、プロセスの一覧から見つかり、終われば見つからなくなることを確かめる（Linux は /proc、Windows は
    /// EnumProcesses・QueryFullProcessImageNameW。それ以外の OS は一覧を取らないので確かめない）。
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn a_real_second_process_of_the_same_exe_is_found_until_it_ends() {
        let dir = temp_dir("real");
        let exe = dir.join(if cfg!(windows) {
            "sleeper.exe"
        } else {
            "sleeper"
        });
        // 試験の実行ファイル自身を別の名前の別のパスに置いて動かす（同じ場所に置けなければ写す）。
        let this = std::env::current_exe().unwrap();
        if std::fs::hard_link(&this, &exe).is_err() {
            std::fs::copy(&this, &exe).unwrap();
        }
        let mut child = std::process::Command::new(&exe)
            .args([
                "--exact",
                "update::launch::tests::sleeper_for_the_real_process_test",
                "--nocapture",
            ])
            .env("YOLU_SLEEPER", "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let own = std::process::id();
        // 起動してパスが取れるようになるまで少し待つ
        let found = (0..100).any(|_| {
            if runs_elsewhere(&exe, own, running_processes()) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            false
        });
        child.kill().unwrap();
        child.wait().unwrap();
        let gone = (0..100).any(|_| {
            if !runs_elsewhere(&exe, own, running_processes()) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            false
        });
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(found, "動いている別のプロセスが見つからない");
        assert!(gone, "終わったプロセスが見つかり続ける");
    }

    /// 上の試験が別プロセスとして立てる側。環境変数が無いときは何もせず終わる（通常の試験の一覧では何も起こさない）。
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn sleeper_for_the_real_process_test() {
        if std::env::var_os("YOLU_SLEEPER").is_some() {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    }

    #[test]
    fn the_staging_folder_is_per_user_and_named_for_the_uninstaller() {
        if let Some(dir) = staging_dir() {
            assert!(dir.ends_with("YoluPainter/updates") || dir.ends_with(r"YoluPainter\updates"));
            assert!(dir.is_absolute());
        }
    }

    #[test]
    fn macos_keeps_no_staging_folder_because_it_downloads_nothing() {
        let home = |key: &str| (key == "HOME").then(|| PathBuf::from("/Users/u"));
        assert_eq!(staging_for("macos", home), None);
        // ほかの OS は今までどおり（Linux は XDG か ~/.cache）
        assert_eq!(
            staging_for("linux", home),
            Some(PathBuf::from("/Users/u/.cache/YoluPainter/updates"))
        );
        let xdg = |key: &str| (key == "XDG_CACHE_HOME").then(|| PathBuf::from("/var/cache/u"));
        assert_eq!(
            staging_for("linux", xdg),
            Some(PathBuf::from("/var/cache/u/YoluPainter/updates"))
        );
        // 相対の環境変数は使わない
        let relative = |key: &str| (key == "XDG_CACHE_HOME").then(|| PathBuf::from("cache"));
        assert_eq!(staging_for("linux", relative), None);
    }
}
