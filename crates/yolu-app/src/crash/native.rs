//! ネイティブ障害の最小記録。ファイルとヘッダーは平常時に準備する。
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, OnceLock},
};
#[cfg(windows)]
use std::sync::atomic::Ordering;
static OUTPUT: OnceLock<Output> = OnceLock::new();
static ENTERED: AtomicBool = AtomicBool::new(false);
struct Output {
    file: File,
    path: PathBuf,
    header: Vec<u8>,
    #[cfg(windows)]
    private_terms: Vec<Vec<u16>>,
}

pub fn install(dir: &Path, header: &str) {
    let path = dir.join(format!("crash-{}.log", super::stamp()));
    let Ok(file) = OpenOptions::new().write(true).create_new(true).open(&path) else {
        return;
    };
    // 動いている間は排他ロックを持つ。強制終了などで取り残された長さ 0 の記録先は、ロックが空いていることで見分けて消す。
    let _ = file.try_lock();
    sweep(dir, &path);
    let output = Output {
        file,
        path,
        header: format!("{header}Kind: Native crash\n").into_bytes(),
        #[cfg(windows)]
        private_terms: super::Redactor::environment()
            .users
            .iter()
            .map(|s| s.encode_utf16().collect())
            .collect(),
    };
    if OUTPUT.set(output).is_err() {
        return;
    }
    super::prune(dir, "crash-", super::CRASH_KEEP);
    #[cfg(windows)]
    // SAFETY: プロセス終了まで生きる関数を登録する。異常から実行を再開しない。
    unsafe {
        windows::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter(Some(filter));
    }
    #[cfg(target_os = "linux")]
    signals::install();
}

/// 持ち主のいない長さ 0 の記録先（前の起動が panic・強制終了・電源断で消せなかったもの）を消す。
/// 動いている別の起動の記録先は、その起動がロックを持っているので消さない。ロックを使えない場所では、持ち主が分からないので消さない。
pub fn sweep(dir: &Path, own: &Path) {
    for path in super::files(dir, "crash-") {
        if path == own || !super::is_empty(&path) {
            continue;
        }
        let Ok(file) = File::open(&path) else {
            continue;
        };
        if file.try_lock().is_ok() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

pub fn reserved(path: &Path) -> bool {
    OUTPUT.get().is_some_and(|out| out.path == path)
}

pub fn cleanup() {
    if let Some(out) = OUTPUT.get() {
        if out.file.metadata().is_ok_and(|m| m.len() == 0) {
            let _ = std::fs::remove_file(&out.path);
        }
    }
}

/// Linux のシグナル。std が SIGSEGV・SIGBUS に置いているスタック溢れの検出（sigaltstack の上で動く）を壊さないよう、
/// `sigaction` で SA_ONSTACK を付けて置き、前の動作を控えて、記録のあとでそれへ引き継ぐ。
#[cfg(target_os = "linux")]
mod signals {
    use super::{ENTERED, OUTPUT};
    use libc::{c_int, c_void, siginfo_t};
    use std::sync::{atomic::Ordering, OnceLock};

    const SIGNALS: [c_int; 5] = [
        libc::SIGILL,
        libc::SIGABRT,
        libc::SIGBUS,
        libc::SIGFPE,
        libc::SIGSEGV,
    ];
    /// 置く前の動作。置き終えたあとで入る（それより前に届いたシグナルは既定の動作へ進む）。
    static PREVIOUS: OnceLock<Vec<(c_int, libc::sigaction)>> = OnceLock::new();

    pub fn install() {
        let mut previous = Vec::new();
        for sig in SIGNALS {
            // SAFETY: 0 で埋めた sigaction に、自分の関数・SA_SIGINFO・SA_ONSTACK を入れて登録する。
            // 登録関数は async-signal-safe な処理（write・signal・raise）と前の動作の呼び出しだけをする。
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = handler as *const () as usize;
                action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigemptyset(&mut action.sa_mask);
                let mut old: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(sig, &action, &mut old) == 0 {
                    previous.push((sig, old));
                }
            }
        }
        let _ = PREVIOUS.set(previous);
    }

    extern "C" fn handler(sig: c_int, info: *mut siginfo_t, context: *mut c_void) {
        use std::os::fd::AsRawFd;
        if !ENTERED.swap(true, Ordering::Relaxed) {
            if let Some(out) = OUTPUT.get() {
                let detail: &[u8] = match sig {
                    libc::SIGILL => b"Signal: SIGILL\n",
                    libc::SIGABRT => b"Signal: SIGABRT\n",
                    libc::SIGBUS => b"Signal: SIGBUS\n",
                    libc::SIGFPE => b"Signal: SIGFPE\n",
                    _ => b"Signal: SIGSEGV\n",
                };
                // SAFETY: fd は閉じず、スライスもプロセス終了まで有効。割り当て・ロック・Rust の I/O は使わない。
                unsafe {
                    libc::write(
                        out.file.as_raw_fd(),
                        out.header.as_ptr().cast(),
                        out.header.len(),
                    );
                    libc::write(out.file.as_raw_fd(), detail.as_ptr().cast(), detail.len());
                }
            }
        }
        // 前の動作（std のスタック溢れの検出など）へ引き継ぐ。戻ってきたら既定の動作で終わる。
        if let Some((_, old)) = PREVIOUS
            .get()
            .and_then(|all| all.iter().find(|(s, _)| *s == sig))
        {
            let function = old.sa_sigaction;
            if function != libc::SIG_DFL && function != libc::SIG_IGN {
                // SAFETY: sa_sigaction は前の持ち主が登録した関数で、SA_SIGINFO の有無で型が決まる。
                unsafe {
                    if old.sa_flags & libc::SA_SIGINFO != 0 {
                        let f: extern "C" fn(c_int, *mut siginfo_t, *mut c_void) =
                            std::mem::transmute(function);
                        f(sig, info, context);
                    } else {
                        let f: extern "C" fn(c_int) = std::mem::transmute(function);
                        f(sig);
                    }
                }
            }
        }
        // SAFETY: 既定動作へ戻して同じシグナルを再送する。障害後のアプリは継続しない。
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
    }
}

#[cfg(windows)]
unsafe extern "system" fn filter(
    info: *const windows::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS,
) -> i32 {
    use std::io::Write;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::HMODULE,
            System::LibraryLoader::{
                GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            },
        },
    };
    if ENTERED.swap(true, Ordering::Relaxed) || info.is_null() {
        return 0;
    }
    if let Some(out) = OUTPUT.get() {
        let mut file = &out.file;
        let _ = file.write_all(&out.header);
        // SAFETY: OS がフィルターの呼び出し中だけ渡す構造体。null は読み取らない。
        let record = unsafe { (*info).ExceptionRecord };
        if !record.is_null() {
            let record = unsafe { &*record };
            let _ = writeln!(
                file,
                "Exception: 0x{:08x}\nAddress: {:p}",
                record.ExceptionCode.0 as u32, record.ExceptionAddress
            );
            let mut module = HMODULE::default();
            // SAFETY: FROM_ADDRESS では文字列ではなく命令アドレスを受け取る。
            if unsafe {
                GetModuleHandleExW(
                    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                        | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                    PCWSTR(record.ExceptionAddress.cast()),
                    &mut module,
                )
            }
            .is_ok()
            {
                let mut name = [0u16; 1024];
                let len = unsafe { GetModuleFileNameW(Some(module), &mut name) } as usize;
                let _ = writeln!(file, "Module base: {:p}", module.0);
                if len > 0 && len < name.len() {
                    let start = name[..len]
                        .iter()
                        .rposition(|c| *c == 92 || *c == 47)
                        .map_or(0, |i| i + 1);
                    // ヒープを使わず、パスとユーザー名を除く。モジュールは exe / dll だけ名前を残す。
                    let name = &name[start..len];
                    let fold = |c: u16| if (65..=90).contains(&c) { c + 32 } else { c };
                    let private = out.private_terms.iter().any(|term| {
                        !term.is_empty()
                            && name.windows(term.len()).any(|part| {
                                part.iter().zip(term).all(|(a, b)| fold(*a) == fold(*b))
                            })
                    });
                    let known_extension = [b".dll", b".exe"].iter().any(|ext| {
                        name.len() >= 4
                            && name[name.len() - 4..]
                                .iter()
                                .zip(ext.iter())
                                .all(|(a, b)| fold(*a) == *b as u16)
                    });
                    let _ = file.write_all(b"Module: ");
                    let private_extension = [
                        ".ylp", ".ylbrush", ".ylsmart", ".fbx", ".psd", ".psb", ".obj", ".gltf",
                        ".glb", ".png", ".jpg", ".jpeg", ".tga", ".exr", ".hdr", ".kra", ".abr",
                        ".blend",
                    ]
                    .iter()
                    .any(|ext| {
                        name.windows(ext.len()).any(|part| {
                            part.iter()
                                .zip(ext.bytes())
                                .all(|(a, b)| fold(*a) == b as u16)
                        })
                    });
                    if private || private_extension || !known_extension {
                        let _ = file.write_all(b"[module]");
                    } else {
                        for c in char::decode_utf16(name.iter().copied()) {
                            let _ = write!(file, "{}", c.unwrap_or(char::REPLACEMENT_CHARACTER));
                        }
                    }
                    let _ = file.write_all(b"\n");
                }
            }
        }
        let _ = file.sync_data();
    }
    0 // EXCEPTION_CONTINUE_SEARCH: OS の既定処理へ渡す。
}
