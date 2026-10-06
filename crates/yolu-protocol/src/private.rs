//! 自分（このユーザー）だけが読み書きできるフォルダとファイル。Live Link の鍵・Unix のソケット・共有メモリの画像に使い、同じ PC の
//! ほかのユーザーのプロセスがつなぐことも、描いた画素を読むこともできないようにする。
//!
//! - Unix: フォルダは 0700、ファイルは 0600 で作る（umask に関わらず）。既にあるフォルダ・ファイルは、持ち主が自分（実効 UID）で、
//!   ほかの人の読み書きの印が無いことを確かめ、合わなければ使わない（共有の一時フォルダに、ほかのユーザーが先に同じ名前を作っても乗らない）。
//! - Windows: 親から受け継がない（保護した）DACL に、自分（プロセスのトークンのユーザーの SID）だけを許すエースを 1 つ。作る時の
//!   SECURITY_ATTRIBUTES に入れるので、ほかの許しが付いている瞬間が無い。
//!
//! Live Link のフォルダ（`link_dir`）: Linux は `$XDG_RUNTIME_DIR/yolupainter`（無ければ標準の `/run/user/<UID>/yolupainter`）、
//! どちらも使えなければ `/tmp/yolupainter-<UID>`、さらに `$TMPDIR/yolupainter-<UID>`（ブリッジが探すだけの候補）。macOS は `/tmp/yolupainter-<UID>`
//! （ソケットのパスの上限に収めるため。ユーザーごとに別の持ち主を確かめる）。Windows は `%LOCALAPPDATA%\YoluPainter\LiveLink`。
//! スタンドアロンは使える先頭の候補に置き、ブリッジは鍵のある候補を探す（`find_link_dir`）。環境変数 `TMPDIR` が両者で違っても、
//! `/tmp` の置き場は同じなので見つかる。見つからないのは、`XDG_RUNTIME_DIR` が標準の `/run/user/<UID>` と違う場所を指していて、
//! ブリッジ側が同じ値を持たないとき。

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// Live Link の鍵と（Unix の）ソケットを置くフォルダ（スタンドアロンが使う。無ければ作る）。
pub fn link_dir() -> io::Result<PathBuf> {
    first_private_dir(link_dir_candidates()?)
}

/// 先頭の候補が自分だけのものにできなければ（共有の /tmp にほかのユーザーが同じ名前を先に作った、など）次の候補を使う。
fn first_private_dir(candidates: Vec<PathBuf>) -> io::Result<PathBuf> {
    let mut last = None;
    for dir in candidates {
        match ensure_private_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("Live Link のフォルダが決まりません")))
}

/// ブリッジが使うフォルダ: つなぎ先の名前の鍵とソケットが両方ある候補（スタンドアロンが待ち受けているところ）、無ければ鍵だけがある候補、
/// どちらも無ければスタンドアロンが置くはずの場所。作らない。スタンドアロンとブリッジで環境変数（`XDG_RUNTIME_DIR` など）が違っても、同じフォルダを見つける。
pub fn find_link_dir(name: &str) -> io::Result<PathBuf> {
    pick_link_dir(link_dir_candidates()?, name)
        .ok_or_else(|| io::Error::other("Live Link のフォルダが決まりません"))
}

fn pick_link_dir(candidates: Vec<PathBuf>, name: &str) -> Option<PathBuf> {
    let key = format!("{name}.key");
    let socket = format!("{name}.sock");
    candidates
        .iter()
        .find(|d| d.join(&key).exists() && d.join(&socket).exists())
        .or_else(|| candidates.iter().find(|d| d.join(&key).exists()))
        .or_else(|| candidates.first())
        .cloned()
}

/// フォルダの候補（前から。スタンドアロンは使える先頭に置く）。
#[cfg(unix)]
fn link_dir_candidates() -> io::Result<Vec<PathBuf>> {
    Ok(unix_candidates(
        unsafe { libc::geteuid() },
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::temp_dir(),
        |run| check_private_dir(run).is_ok(),
    ))
}

/// Unix の候補の並べ方（環境を引数にして、試験できるようにする）。
#[cfg(unix)]
fn unix_candidates(
    uid: u32,
    xdg_runtime_dir: Option<PathBuf>,
    temp_dir: PathBuf,
    usable_run_dir: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    fn push(dir: PathBuf, out: &mut Vec<PathBuf>) {
        if !out.contains(&dir) {
            out.push(dir);
        }
    }
    #[cfg(target_os = "linux")]
    {
        // 実行時のフォルダ（RAM の上）。環境変数があればそれ、無くても標準の場所を候補にする。使えるのは、自分のもので、ほかの人が入れないときだけ（決まりでは 0700）
        let mut runs: Vec<PathBuf> = Vec::new();
        runs.extend(xdg_runtime_dir);
        runs.push(PathBuf::from(format!("/run/user/{uid}")));
        for run in runs {
            if run.is_absolute() && usable_run_dir(&run) {
                push(run.join("yolupainter"), &mut out);
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (&xdg_runtime_dir, &usable_run_dir);
    // 固定の /tmp。$TMPDIR はプロセスごとに違い得る（Unity をほかの環境から起動する、など）ので、スタンドアロンとブリッジが同じ場所を
    // 見つけられるよう、/tmp の置き場を先に使う。Mac の一時フォルダ（$TMPDIR）は長く、Unix ソケットのパスの上限（104 バイト）にも収まらない
    push(
        PathBuf::from("/tmp").join(format!("yolupainter-{uid}")),
        &mut out,
    );
    // $TMPDIR の置き場（Mac は使わない）。スタンドアロンが /tmp に置けなかったときの、ブリッジが探すだけの候補
    if !cfg!(target_os = "macos") {
        push(temp_dir.join(format!("yolupainter-{uid}")), &mut out);
    }
    out
}

#[cfg(windows)]
fn link_dir_candidates() -> io::Result<Vec<PathBuf>> {
    link_dir_path().map(|d| vec![d])
}

#[cfg(not(any(unix, windows)))]
fn link_dir_candidates() -> io::Result<Vec<PathBuf>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "この OS では Live Link の自分だけのフォルダを作れません",
    ))
}

#[cfg(windows)]
fn link_dir_path() -> io::Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA がありません"))?;
    let parent = base.join("YoluPainter");
    std::fs::create_dir_all(&parent)?;
    Ok(parent.join("LiveLink"))
}

fn not_private(path: &Path, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{} は自分だけのものではありません（{why}）", path.display()),
    )
}

/// フォルダを自分だけのものにする（無ければ作る。あれば確かめる、Windows では DACL を当て直す）。
pub fn ensure_private_dir(path: &Path) -> io::Result<()> {
    imp::ensure_private_dir(path)
}

/// 自分だけが読み書きできるファイルを新しく作る（既にあれば失敗）。temporary は Windows でディスクへの書き出しを控えさせる印。
pub fn create_private_file(path: &Path, temporary: bool) -> io::Result<File> {
    imp::create_private_file(path, temporary)
}

/// 自分のファイルを読むために開く（Unix では、名前がシンボリックリンクなら辿らずに断る。Windows では書き手が消せるよう共有する）。
pub fn open_private_file(path: &Path) -> io::Result<File> {
    imp::open_private_file(path)
}

/// 開いたファイルが自分だけのものか（Unix: 持ち主が自分で、ほかの人の読み書きの印が無い。Windows: 確かめない ── 置き場が
/// ユーザーのプロファイルの下で、作る側が DACL を付ける）。
pub fn check_private_file(file: &File, path: &Path) -> io::Result<()> {
    imp::check_private_file(file, path)
}

/// フォルダが自分だけのものか（作らない）。
pub fn check_private_dir(path: &Path) -> io::Result<()> {
    imp::check_private_dir(path)
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

    fn euid() -> u32 {
        unsafe { libc::geteuid() }
    }

    pub fn check_private_dir(path: &Path) -> io::Result<()> {
        let m = std::fs::symlink_metadata(path)?;
        if m.file_type().is_symlink() || !m.is_dir() {
            return Err(not_private(path, "フォルダでない"));
        }
        if m.uid() != euid() {
            return Err(not_private(path, "持ち主が別のユーザー"));
        }
        if m.mode() & 0o077 != 0 {
            return Err(not_private(path, "ほかの人も入れる"));
        }
        Ok(())
    }

    pub fn ensure_private_dir(path: &Path) -> io::Result<()> {
        match std::fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {
                // umask で削られていても 0700 にそろえる
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        check_private_dir(path)
    }

    pub fn create_private_file(path: &Path, _temporary: bool) -> io::Result<File> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        // mode は umask で削られるだけ（広がらない）が、念のため開いた後にも 0600 にそろえる
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }

    pub fn open_private_file(path: &Path) -> io::Result<File> {
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }

    pub fn check_private_file(file: &File, path: &Path) -> io::Result<()> {
        let m = file.metadata()?;
        if !m.is_file() {
            return Err(not_private(path, "普通のファイルでない"));
        }
        if m.uid() != euid() {
            return Err(not_private(path, "持ち主が別のユーザー"));
        }
        if m.mode() & 0o077 != 0 {
            return Err(not_private(path, "ほかの人も読める"));
        }
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    pub use crate::private::win::*;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::Authorization::{SetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_TEMPORARY,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    pub fn check_private_dir(path: &Path) -> io::Result<()> {
        let m = std::fs::symlink_metadata(path)?;
        if m.file_type().is_symlink() || !m.is_dir() {
            return Err(not_private(path, "フォルダでない"));
        }
        Ok(())
    }

    pub fn ensure_private_dir(path: &Path) -> io::Result<()> {
        let sd = OwnedSd::from_sddl(&private_sddl(true)?)?;
        let sa = sd.attributes();
        let w = wide(path);
        if unsafe { CreateDirectoryW(w.as_ptr(), &sa) } == 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(183) {
                // ERROR_ALREADY_EXISTS 以外
                return Err(err);
            }
            // 前からあるフォルダにも、自分だけの保護した DACL を当て直す（持ち主でなければ失敗する）
            let dacl = sd.dacl()?;
            let r = unsafe {
                SetNamedSecurityInfoW(
                    w.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    dacl,
                    std::ptr::null_mut(),
                )
            };
            if r != 0 {
                return Err(io::Error::from_raw_os_error(r as i32));
            }
        }
        check_private_dir(path)
    }

    pub fn create_private_file(path: &Path, temporary: bool) -> io::Result<File> {
        let sd = OwnedSd::from_sddl(&private_sddl(false)?)?;
        let sa = sd.attributes();
        let w = wide(path);
        let handle = unsafe {
            CreateFileW(
                w.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                // 読み・書き・消すのを共有する（相手が開いていても書き手が消せ、最後の写像が閉じたときに消える）
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                &sa,
                CREATE_NEW,
                if temporary {
                    FILE_ATTRIBUTE_TEMPORARY
                } else {
                    FILE_ATTRIBUTE_NORMAL
                },
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let code = unsafe { GetLastError() };
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        Ok(unsafe { File::from_raw_handle(handle as _) })
    }

    pub fn open_private_file(path: &Path) -> io::Result<File> {
        use std::os::windows::fs::OpenOptionsExt;
        // 読み・書き・消すのを共有する（書き手が開かれたままでも消せる）
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(7)
            .open(path)
    }

    pub fn check_private_file(file: &File, path: &Path) -> io::Result<()> {
        // 置き場が自分だけの DACL のフォルダの中で、作る側が自分だけの DACL を付ける。ここでは種類だけ確かめる
        if !file.metadata()?.is_file() {
            return Err(not_private(path, "普通のファイルでない"));
        }
        Ok(())
    }
}

/// 相手のプロセスの持ち主（Windows のアカウントの SID。「S-1-5-21-…」の文字列）が自分と同じか。大文字小文字は区別しない。空の SID は誰とも同じにしない。
pub fn same_account(own: &str, peer: &str) -> bool {
    !own.is_empty() && own.eq_ignore_ascii_case(peer)
}

/// つないだ相手のプロセスの持ち主（OS から取り出した結果 `peer`）と自分の持ち主から、その相手とつながってよいかを決める。別のユーザーの
/// プロセスは断り、持ち主を取り出せない相手も断る（確かめられない相手を信じない）。Windows の名前付きパイプの両側が、挨拶の前
/// （何も送る前・読む前）に呼ぶ。Unix の確かめ（実効 UID）は `link::check_peer`。
pub fn judge_peer_account(own: &str, peer: io::Result<String>) -> io::Result<()> {
    match peer {
        Ok(peer) if same_account(own, &peer) => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "つないだ相手が別のユーザーのプロセスです",
        )),
        Err(e) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("つないだ相手のプロセスの持ち主を確かめられません: {e}"),
        )),
    }
}

/// Windows のセキュリティの手伝い（自分の SID と、自分だけを許す SDDL）。
#[cfg(windows)]
pub mod win {
    use std::io;
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACL, PSECURITY_DESCRIPTOR,
        SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// このプロセスのユーザーの SID（"S-1-5-21-…"）。
    pub fn user_sid() -> io::Result<String> {
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(io::Error::last_os_error());
            }
            let sid = token_user_sid(token);
            CloseHandle(token);
            sid
        }
    }

    /// そのプロセス番号のプロセスのユーザーの SID。プロセスを開けない（もう無い・権限が足りない）・トークンを読めないときは失敗する
    /// （呼び手は、確かめられない相手を信じない）。照会だけの権限（PROCESS_QUERY_LIMITED_INFORMATION）で開く。
    pub fn process_user_sid(pid: u32) -> io::Result<String> {
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut token: HANDLE = std::ptr::null_mut();
            let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token);
            let error = (opened == 0).then(io::Error::last_os_error);
            CloseHandle(process);
            if let Some(e) = error {
                return Err(e);
            }
            let sid = token_user_sid(token);
            CloseHandle(token);
            sid
        }
    }

    /// トークンのユーザーの SID の文字列。
    unsafe fn token_user_sid(token: HANDLE) -> io::Result<String> {
        let mut len = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut s) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut n = 0;
        while *s.add(n) != 0 {
            n += 1;
        }
        let out = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
        LocalFree(s.cast());
        Ok(out)
    }

    /// パスのもの（ファイル・フォルダ・`\\\\.\\pipe\\名前` の名前付きパイプ）の DACL を SDDL の文字列で（試験用）。
    pub fn dacl_sddl(path: &str) -> io::Result<String> {
        use windows_sys::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW,
            SE_FILE_OBJECT,
        };
        use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            let r = GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut sd,
            );
            if r != 0 {
                return Err(io::Error::from_raw_os_error(r as i32));
            }
            let mut text: *mut u16 = std::ptr::null_mut();
            let ok = ConvertSecurityDescriptorToStringSecurityDescriptorW(
                sd,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                std::ptr::null_mut(),
            );
            if ok == 0 {
                let e = io::Error::last_os_error();
                LocalFree(sd);
                return Err(e);
            }
            let mut n = 0;
            while *text.add(n) != 0 {
                n += 1;
            }
            let out = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
            LocalFree(text.cast());
            LocalFree(sd);
            Ok(out)
        }
    }

    /// その番号のプロセスが今も動いているか（見つからなければ動いていない。開けない（権限が足りない）なら、あるものとする）。
    pub fn process_alive(pid: u32) -> bool {
        use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return std::io::Error::last_os_error().raw_os_error()
                    == Some(ERROR_ACCESS_DENIED as i32);
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
            ok == 0 || code == STILL_ACTIVE as u32
        }
    }

    /// Wine の上で動いているか（Wine は DACL をファイルやフォルダに保存せず、既定の許しを返す。DACL の試験は本物の Windows だけで確かめる）。
    pub fn is_wine() -> bool {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
        let ntdll: Vec<u16> = "ntdll.dll".encode_utf16().chain(Some(0)).collect();
        unsafe {
            let module = GetModuleHandleW(ntdll.as_ptr());
            !module.is_null()
                && GetProcAddress(module, c"wine_get_version".as_ptr().cast()).is_some()
        }
    }

    /// ユーザーごとに変わる短い印（SID の SHA-256 の先頭 4 バイトの 16 進）。名前付きパイプの名前に入れて、同じ PC の別のユーザーと重ならないようにする。
    pub fn user_tag() -> io::Result<String> {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(user_sid()?.as_bytes());
        Ok(hash[..4].iter().map(|b| format!("{b:02x}")).collect())
    }

    /// 自分だけに全部を許す、保護した（受け継がない）DACL の SDDL。inherit はフォルダの中へ受け継がせる印。
    pub fn private_sddl(inherit: bool) -> io::Result<String> {
        let sid = user_sid()?;
        Ok(if inherit {
            format!("D:P(A;OICI;FA;;;{sid})")
        } else {
            format!("D:P(A;;GA;;;{sid})")
        })
    }

    /// SDDL から作ったセキュリティ記述子（LocalFree で返す）。
    pub struct OwnedSd(PSECURITY_DESCRIPTOR);

    impl OwnedSd {
        pub fn from_sddl(sddl: &str) -> io::Result<OwnedSd> {
            let w: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
            let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            let ok = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    w.as_ptr(),
                    SDDL_REVISION_1,
                    &mut sd,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(OwnedSd(sd))
        }

        /// 作る関数へ渡す SECURITY_ATTRIBUTES（この値より長く使わない）。
        pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
            SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: self.0,
                bInheritHandle: 0,
            }
        }

        /// 中の DACL（この値より長く使わない）。
        pub fn dacl(&self) -> io::Result<*mut ACL> {
            let mut present = 0;
            let mut defaulted = 0;
            let mut dacl: *mut ACL = std::ptr::null_mut();
            if unsafe { GetSecurityDescriptorDacl(self.0, &mut present, &mut dacl, &mut defaulted) }
                == 0
                || present == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(dacl)
        }
    }

    impl Drop for OwnedSd {
        fn drop(&mut self) {
            unsafe { LocalFree(self.0) };
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ylp-private-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_private_dir_and_file_are_only_for_the_user() {
        let d = scratch("ok");
        ensure_private_dir(&d).unwrap();
        assert_eq!(std::fs::metadata(&d).unwrap().mode() & 0o777, 0o700);
        // 2 度目は確かめるだけ
        ensure_private_dir(&d).unwrap();
        let f = d.join("a.key");
        let file = create_private_file(&f, false).unwrap();
        assert_eq!(file.metadata().unwrap().mode() & 0o777, 0o600);
        check_private_file(&file, &f).unwrap();
        assert!(
            create_private_file(&f, false).is_err(),
            "新しいファイルだけを作る"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_dir_or_file_others_can_open_is_refused() {
        let d = scratch("open");
        std::fs::create_dir(&d).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        let e = ensure_private_dir(&d).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
        let f = d.join("shared.key");
        std::fs::write(&f, b"x").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        let file = File::open(&f).unwrap();
        assert!(check_private_file(&file, &f).is_err());
        // 名前の付いたリンクも使わない（ほかの人の場所へ向けられる）
        let link = scratch("link");
        std::os::unix::fs::symlink(&d, &link).unwrap();
        assert!(ensure_private_dir(&link).is_err());
        std::fs::remove_file(&link).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_link_dir_is_private() {
        let d = link_dir().unwrap();
        check_private_dir(&d).unwrap();
    }

    /// TMPDIR が違うスタンドアロンとブリッジでも、/run/user が無い環境で置いた先頭の候補を、ブリッジが探す。
    #[test]
    fn the_candidates_share_a_fixed_tmp_place_whatever_tmpdir_is() {
        let fixed = PathBuf::from("/tmp/yolupainter-1000");
        let standalone = unix_candidates(1000, None, PathBuf::from("/var/tmp/a"), |_| false);
        let bridge = unix_candidates(1000, None, PathBuf::from("/var/tmp/b"), |_| false);
        assert_eq!(standalone[0], fixed, "スタンドアロンは /tmp の固定の置き場");
        assert!(bridge.contains(&fixed), "ブリッジも探す");
        if cfg!(not(target_os = "macos")) {
            assert!(standalone.contains(&PathBuf::from("/var/tmp/a/yolupainter-1000")));
        }
        #[cfg(target_os = "linux")]
        {
            // 使える実行時のフォルダが先（環境変数、標準の場所の順）。重なる候補は 1 つ
            let c = unix_candidates(1000, Some("/xdg".into()), PathBuf::from("/tmp"), |_| true);
            assert_eq!(
                c,
                vec![
                    PathBuf::from("/xdg/yolupainter"),
                    PathBuf::from("/run/user/1000/yolupainter"),
                    fixed.clone()
                ]
            );
            // 使えない実行時のフォルダは入れない
            let c = unix_candidates(1000, Some("/xdg".into()), PathBuf::from("/tmp"), |_| false);
            assert_eq!(c, vec![fixed]);
        }
    }

    /// 先頭の候補が自分だけのものにならなくても、次の候補が使われる（ブリッジは全部の候補から探す）。
    #[test]
    fn the_standalone_uses_the_next_candidate_and_the_bridge_finds_the_key_anywhere() {
        let squatted = scratch("squat");
        std::fs::create_dir(&squatted).unwrap();
        std::fs::set_permissions(&squatted, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mine = scratch("mine");
        let got = first_private_dir(vec![squatted.clone(), mine.clone()]).unwrap();
        assert_eq!(got, mine);
        assert!(first_private_dir(vec![squatted.clone()]).is_err());
        assert!(first_private_dir(vec![]).is_err());
        // ブリッジ: 鍵とソケットが両方ある所、無ければ鍵のある所、どちらも無ければ先頭
        let other = scratch("other");
        ensure_private_dir(&other).unwrap();
        let all = vec![squatted.clone(), other.clone(), mine.clone()];
        assert_eq!(pick_link_dir(all.clone(), "n"), Some(squatted.clone()));
        std::fs::write(mine.join("n.key"), b"k").unwrap();
        assert_eq!(pick_link_dir(all.clone(), "n"), Some(mine.clone()));
        std::fs::write(other.join("n.key"), b"k").unwrap();
        std::fs::write(other.join("n.sock"), b"s").unwrap();
        assert_eq!(pick_link_dir(all, "n"), Some(other.clone()));
        assert_eq!(pick_link_dir(vec![], "n"), None);
        for d in [&squatted, &mine, &other] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ylp-private-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// DACL が「受け継がない保護」で、許しが自分の SID の 1 つだけ（Everyone・Users・Administrators などが付いていない）。
    fn assert_only_me(sddl: &str, what: &str) {
        if win::is_wine() {
            eprintln!("Wine は DACL を保存しないので、{what} の DACL は確かめません（{sddl}）");
            return;
        }
        let sid = win::user_sid().unwrap();
        assert!(
            sddl.starts_with("D:P"),
            "{what}: 受け継ぎを止めていない {sddl}"
        );
        assert_eq!(
            sddl.matches("(A;").count(),
            1,
            "{what}: 許しが 1 つでない {sddl}"
        );
        assert_eq!(
            sddl.matches('(').count(),
            1,
            "{what}: 拒否などが混ざる {sddl}"
        );
        // SDDL は既知の SID を短い別名で書く。組み込みの Administrator（RID 500。CI の Windows の実行ユーザー）は「LA」
        let alias = sid.ends_with("-500").then_some("LA");
        assert!(
            [Some(sid.as_str()), alias]
                .into_iter()
                .flatten()
                .any(|me| sddl.contains(&format!(";;;{me})"))),
            "{what}: 自分でない {sddl}"
        );
    }

    #[test]
    fn a_private_dir_and_file_have_only_my_sid_in_the_dacl() {
        let d = scratch("acl");
        ensure_private_dir(&d).unwrap();
        assert_only_me(&win::dacl_sddl(d.to_str().unwrap()).unwrap(), "フォルダ");
        // 2 度目（前からあるフォルダ）も当て直して確かめる
        ensure_private_dir(&d).unwrap();
        let f = d.join("a.key");
        let file = create_private_file(&f, false).unwrap();
        assert_only_me(&win::dacl_sddl(f.to_str().unwrap()).unwrap(), "ファイル");
        check_private_file(&file, &f).unwrap();
        assert!(
            create_private_file(&f, false).is_err(),
            "新しいファイルだけを作る"
        );
        drop(file);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_link_dir_and_the_named_pipe_are_only_for_me() {
        assert_only_me(
            &win::dacl_sddl(link_dir().unwrap().to_str().unwrap()).unwrap(),
            "Live Link のフォルダ",
        );
        let name = format!("ylp-acl-{}", std::process::id());
        let _server = crate::link::Server::bind(&name, false).unwrap();
        if win::is_wine() {
            return;
        }
        let pipe = format!(r"\\.\pipe\{}", crate::link::pipe_name(&name).unwrap());
        assert_only_me(&win::dacl_sddl(&pipe).unwrap(), "名前付きパイプ");
    }
}

#[cfg(test)]
mod account_tests {
    use super::*;

    const ME: &str = "S-1-5-21-1111-2222-3333-1001";

    #[test]
    fn only_the_same_account_is_accepted_and_the_case_does_not_matter() {
        assert!(same_account(ME, ME));
        assert!(same_account(ME, &ME.to_ascii_lowercase()));
        assert!(
            !same_account(ME, "S-1-5-21-1111-2222-3333-1002"),
            "別のユーザー"
        );
        assert!(!same_account(ME, "S-1-5-18"), "SYSTEM も別のアカウント");
        assert!(!same_account(ME, ""));
        assert!(!same_account("", ""), "空の SID は誰とも同じにしない");
    }

    #[test]
    fn a_peer_that_is_another_account_or_cannot_be_checked_is_refused() {
        assert!(judge_peer_account(ME, Ok(ME.to_owned())).is_ok());
        let other = judge_peer_account(ME, Ok("S-1-5-21-1111-2222-3333-1002".into())).unwrap_err();
        assert_eq!(other.kind(), io::ErrorKind::PermissionDenied);
        assert!(other.to_string().contains("別のユーザー"), "{other}");
        // 持ち主を取り出せない（プロセスを開けない・もう無い）相手は信じない。取り出せなかった理由を添える
        let unknown =
            judge_peer_account(ME, Err(io::Error::other("アクセスが拒否されました"))).unwrap_err();
        assert_eq!(unknown.kind(), io::ErrorKind::PermissionDenied);
        assert!(
            unknown.to_string().contains("確かめられません")
                && unknown.to_string().contains("アクセスが拒否"),
            "{unknown}"
        );
        // 自分の SID が空（取れていない）なら、誰も通さない
        assert!(judge_peer_account("", Ok(String::new())).is_err());
    }
}
