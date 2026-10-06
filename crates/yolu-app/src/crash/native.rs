//! ネイティブ障害の最小記録。ファイルとヘッダーは平常時に準備する。
use std::{
    fmt::Write as _,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        OnceLock,
    },
};
static OUTPUT: OnceLock<Output> = OnceLock::new();
static ENTERED: AtomicBool = AtomicBool::new(false);
/// 確保の失敗の欄を書いた（ヘッダーは書いてある。落ちたとき、ヘッダーを重ねて書かずに、欄の後ろへ続ける）。
static ALLOC_SEEN: AtomicBool = AtomicBool::new(false);
/// 確保の失敗を書いている最中（記録のための確保の失敗や、別のスレッドの同時の失敗で、重ねて書かない）。
static ALLOC_BUSY: AtomicBool = AtomicBool::new(false);
static ALLOC_FAILURES: AtomicU64 = AtomicU64::new(0);
static IMAGE_BASE: AtomicUsize = AtomicUsize::new(0);
/// 確保の失敗の欄の大きさ（バイト。ヘッダーの直後に固定の大きさで置き、続けて起きた失敗は同じ場所へ上書きする。落ちた詳細はこの後ろ）。
const ALLOC_BLOCK: usize = 1024;
/// 確保の失敗の欄に書く、呼び出しの番地の数。
const ALLOC_FRAMES: usize = 32;
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
    // 確保の失敗の記録で使うものを、平常時に用意する（失敗した確保の中では、確保も読み込みもできない）
    IMAGE_BASE.store(super::image_base().unwrap_or(0), Ordering::Relaxed);
    capture_frames(&mut [0usize; 4]);
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
        // 確保の失敗だけが書いてあって、落ちる処理は入っていない（受け止めて動き続けた失敗。`try_reserve` の断りなど）なら、
        // 落ちた記録にしない
        if ALLOC_SEEN.load(Ordering::Relaxed) && !ENTERED.load(Ordering::Relaxed) {
            let _ = out.file.set_len(0);
        }
        if out.file.metadata().is_ok_and(|m| m.len() == 0) {
            let _ = std::fs::remove_file(&out.path);
        }
    }
}

/// 呼び出しの番地を `frames` へ入れ、入れた数を返す（確保も読み込みもしない。取れなければ 0）。
fn capture_frames(frames: &mut [usize]) -> usize {
    #[cfg(windows)]
    {
        let mut raw = [std::ptr::null_mut::<std::ffi::c_void>(); ALLOC_FRAMES];
        let wanted = frames.len().min(raw.len());
        // SAFETY: 長さ `wanted` までの配列へ書くだけ。
        let count = unsafe {
            windows::Win32::System::Diagnostics::Debug::RtlCaptureStackBackTrace(
                1,
                &mut raw[..wanted],
                None,
            )
        } as usize;
        for (slot, address) in frames.iter_mut().zip(&raw[..count.min(wanted)]) {
            *slot = *address as usize;
        }
        count.min(wanted)
    }
    #[cfg(any(all(target_os = "linux", target_env = "gnu"), target_os = "macos"))]
    {
        let mut raw = [std::ptr::null_mut::<libc::c_void>(); ALLOC_FRAMES];
        let wanted = frames.len().min(raw.len());
        // SAFETY: 長さ `wanted` までの配列へ書くだけ（glibc・Apple の `backtrace`）。最初の呼び出しは共有ライブラリを読み込むことがあるので、
        // `install` が平常時に 1 度呼んでおく。
        let count =
            unsafe { libc::backtrace(raw.as_mut_ptr(), wanted as libc::c_int) }.max(0) as usize;
        for (slot, address) in frames.iter_mut().zip(&raw[..count.min(wanted)]) {
            *slot = *address as usize;
        }
        count.min(wanted)
    }
    #[cfg(not(any(
        windows,
        all(target_os = "linux", target_env = "gnu"),
        target_os = "macos"
    )))]
    {
        let _ = frames;
        0
    }
}

/// 固定の大きさの配列へ書く `fmt::Write`（あふれた分は捨てる）。確保しない。
struct Stack<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl std::fmt::Write for Stack<'_> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let room = self.buf.len() - self.len;
        let take = text.len().min(room);
        self.buf[self.len..self.len + take].copy_from_slice(&text.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}

/// 位置を指定して書く（ファイルの位置を使わない。確保しない）。
fn write_at(file: &File, offset: u64, bytes: &[u8]) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        let _ = file.write_all_at(bytes, offset);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < bytes.len() {
            match file.seek_write(&bytes[done..], offset + done as u64) {
                Ok(0) | Err(_) => break,
                Ok(n) => done += n,
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, offset, bytes);
    }
}

/// 確保が失敗した（アロケーターが null を返した。`oom::RecordingAlloc`）ときに、大きさと呼び出しの番地を記録の先へ書く。
/// 失敗した確保が致命的か（Rust の標準の確保は失敗すると `handle_alloc_error` で止まる。Windows では `__fastfail` で、フィルターに来ない）は
/// ここでは分からないので、すぐ書く。`try_reserve` の断りのように受け止めて動き続けた失敗は、`cleanup`（正常な終わり）が落ちた記録にしない。
/// 書くのは確保も読み込みもしない道（固定の配列・位置指定の書き込み）だけ。
pub fn allocation_failed(size: usize, align: usize) {
    let Some(out) = OUTPUT.get() else { return };
    if ENTERED.load(Ordering::Relaxed) || ALLOC_BUSY.swap(true, Ordering::Acquire) {
        return;
    }
    let failures = ALLOC_FAILURES.fetch_add(1, Ordering::Relaxed) + 1;
    let mut frames = [0usize; ALLOC_FRAMES];
    let count = capture_frames(&mut frames);
    let mut block = [b' '; ALLOC_BLOCK];
    {
        let mut text = Stack {
            buf: &mut block[..ALLOC_BLOCK - 1],
            len: 0,
        };
        let _ = writeln!(
            text,
            "Allocation failed: {size} bytes (align {align}), failure #{failures}"
        );
        let _ = writeln!(
            text,
            "Image base: 0x{:x}",
            IMAGE_BASE.load(Ordering::Relaxed)
        );
        let _ = writeln!(text, "Frames:");
        for address in &frames[..count] {
            let _ = writeln!(text, "0x{address:x}");
        }
    }
    block[ALLOC_BLOCK - 1] = b'\n';
    if !ALLOC_SEEN.swap(true, Ordering::Relaxed) {
        write_at(&out.file, 0, &out.header);
    }
    write_at(&out.file, out.header.len() as u64, &block);
    ALLOC_BUSY.store(false, Ordering::Release);
}

/// Linux のシグナル。std が SIGSEGV・SIGBUS に置いているスタック溢れの検出（sigaltstack の上で動く）を壊さないよう、
/// `sigaction` で SA_ONSTACK を付けて置き、前の動作を控えて、記録のあとでそれへ引き継ぐ。
#[cfg(target_os = "linux")]
mod signals {
    use super::{ALLOC_BLOCK, ALLOC_SEEN, ENTERED, OUTPUT};
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
                    if ALLOC_SEEN.load(Ordering::Relaxed) {
                        // 確保の失敗の欄が先に書いてある: ヘッダーは重ねず、欄の後ろへ続ける
                        libc::lseek(
                            out.file.as_raw_fd(),
                            (out.header.len() + ALLOC_BLOCK) as libc::off_t,
                            libc::SEEK_SET,
                        );
                    } else {
                        libc::write(
                            out.file.as_raw_fd(),
                            out.header.as_ptr().cast(),
                            out.header.len(),
                        );
                    }
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
        use std::io::{Seek, SeekFrom};
        let mut file = &out.file;
        if ALLOC_SEEN.load(Ordering::Relaxed) {
            // 確保の失敗の欄が先に書いてある: ヘッダーは重ねず、欄の後ろへ続ける
            let _ = file.seek(SeekFrom::Start((out.header.len() + ALLOC_BLOCK) as u64));
        } else {
            let _ = file.write_all(&out.header);
        }
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
