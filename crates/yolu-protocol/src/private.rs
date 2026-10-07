//! 自分（このユーザー）だけが読み書きできるフォルダとファイル。Live Link の受け渡しのフォルダ（`files`）に使い、同じ PC のほかの
//! ユーザーが頼みや返事を置くことも、読むこともできないようにする。
//!
//! - Unix: フォルダは 0700、ファイルは 0600 で作る（umask に関わらず）。既にあるフォルダ・ファイルは、持ち主が自分（実効 UID）で、
//!   ほかの人の読み書きの印が無いことを確かめ、合わなければ使わない（共有の一時フォルダに、ほかのユーザーが先に同じ名前を作っても乗らない）。
//! - Windows: 親から受け継がない（保護した）DACL に、自分（プロセスのトークンのユーザーの SID）だけを許すエースを 1 つ。作る時の
//!   SECURITY_ATTRIBUTES に入れるので、ほかの許しが付いている瞬間が無い。

use std::fs::File;
use std::io;
use std::path::Path;

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
}

#[cfg(windows)]
mod imp {
    use super::win::{private_sddl, OwnedSd};
    use super::*;
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
}

/// Windows のセキュリティの手伝い（自分の SID と、自分だけを許す SDDL）。
#[cfg(windows)]
mod win {
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

    /// パスのもの（ファイル・フォルダ）の DACL を SDDL の文字列で（試験用）。
    #[cfg(test)]
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

    /// Wine の上で動いているか（Wine は DACL をファイルやフォルダに保存せず、既定の許しを返す。DACL の試験は本物の Windows だけで確かめる）。
    #[cfg(test)]
    pub fn is_wine() -> bool {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
        let ntdll: Vec<u16> = "ntdll.dll".encode_utf16().chain(Some(0)).collect();
        unsafe {
            let module = GetModuleHandleW(ntdll.as_ptr());
            !module.is_null()
                && GetProcAddress(module, c"wine_get_version".as_ptr().cast()).is_some()
        }
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
    use std::path::PathBuf;

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
        let f = d.join("a.json");
        let file = create_private_file(&f, false).unwrap();
        assert_eq!(file.metadata().unwrap().mode() & 0o777, 0o600);
        assert!(
            create_private_file(&f, false).is_err(),
            "新しいファイルだけを作る"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_dir_others_can_open_or_a_link_to_one_is_refused() {
        let d = scratch("open");
        std::fs::create_dir(&d).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        let e = ensure_private_dir(&d).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
        // 名前の付いたリンクも使わない（ほかの人の場所へ向けられる）
        let link = scratch("link");
        std::os::unix::fs::symlink(&d, &link).unwrap();
        assert!(ensure_private_dir(&link).is_err());
        std::fs::remove_file(&link).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::path::PathBuf;

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
        let f = d.join("a.json");
        let file = create_private_file(&f, false).unwrap();
        assert_only_me(&win::dacl_sddl(f.to_str().unwrap()).unwrap(), "ファイル");
        assert!(
            create_private_file(&f, false).is_err(),
            "新しいファイルだけを作る"
        );
        drop(file);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_live_link_folder_and_its_boxes_have_only_my_sid_in_the_dacl() {
        let root = scratch("folder");
        let folder = crate::files::Folder::open(&root).unwrap();
        for (dir, what) in [
            (folder.root().to_path_buf(), "Live Link のフォルダ"),
            (folder.inbox(), "inbox"),
            (folder.claimed(), "claimed"),
            (folder.outbox(), "outbox"),
        ] {
            assert_only_me(&win::dacl_sddl(dir.to_str().unwrap()).unwrap(), what);
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
