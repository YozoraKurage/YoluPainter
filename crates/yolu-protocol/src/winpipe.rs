//! Windows の名前付きパイプの、つなぎ方と挨拶の見張り（Windows だけ）。
//!
//! - **つなぎ方**: パイプを開くとき、なりすましの段（SQOS）を「匿名」（`SECURITY_ANONYMOUS`）に決める。決めないと既定は「なりすまし可」で、
//!   先にパイプの名前を取った別のプロセスが、つないだこちらのアカウントでなりすませる（読み込んだ後に `ImpersonateNamedPipeClient`）。匿名なら、
//!   つないだ相手はこちらが誰かも分からず、なりすませない。interprocess の `Stream::connect` は開く旗を選べないので、`std::fs::OpenOptions` で開き、
//!   `Stream` に包む。包めなかったときだけ、今までの `Stream::connect` に戻る（つなげないことを選ばない）。
//! - **持ち主の確かめ**: つないだ相手のプロセス（パイプのサーバー・クライアントの番号）の持ち主が自分のアカウントか。SQOS を決める前から、挨拶の前
//!   （何も送らず・読まず）に確かめるので、別のアカウントのなりすましの待ち受けには、そもそも何も渡さない。
//! - **挨拶の時間切れ**: Windows の名前付きパイプには読みの時間切れが無い。時間が来たら待っている読みを `CancelIoEx` で取り消す見張り。

use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsHandle, AsRawHandle, OwnedHandle};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use interprocess::local_socket::Stream;
use interprocess::os::windows::named_pipe::local_socket::Stream as PipeStream;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OVERLAPPED, SECURITY_ANONYMOUS};
use windows_sys::Win32::System::IO::CancelIoEx;

/// パイプが忙しい（空いているサーバー側の口が今は無い）ときの Win32 のエラー。
const ERROR_PIPE_BUSY: i32 = 231;

/// 忙しいパイプを待つ長さの上限。
const BUSY_WAIT: Duration = Duration::from_secs(2);

/// つなぎ方の失敗。
pub(crate) enum Connect {
    /// パイプを開けなかった（相手がいない・権限が無い・忙しいまま）。
    Os(io::Error),
    /// 開けたが `Stream` に包めなかった（今までのつなぎ方に戻る）。
    Wrap,
}

/// パイプ（`\\.\pipe\<pipe>`）を、なりすましの段を匿名にして開き、`Stream` に包む。
pub(crate) fn connect_anonymous(pipe: &str) -> Result<Stream, Connect> {
    let path = format!(r"\\.\pipe\{pipe}");
    let started = Instant::now();
    let file = loop {
        let mut options = std::fs::OpenOptions::new();
        options
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_OVERLAPPED)
            // SECURITY_SQOS_PRESENT は std が自動で付ける。値は SECURITY_ANONYMOUS（0）
            .security_qos_flags(SECURITY_ANONYMOUS);
        match options.open(&path) {
            Ok(f) => break f,
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && started.elapsed() < BUSY_WAIT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(Connect::Os(e)),
        }
    };
    PipeStream::try_from(OwnedHandle::from(file))
        .map(Stream::from)
        .map_err(|e| {
            // 診断（マージしない）: 包めずに開き直すと、相手には閉じた 1 本目と 2 本目のつながりが来る
            eprintln!(
                "YOLU_PIPE_DIAG: 口を包めず開き直す（{:?}、{:?}）",
                e.details, e.cause
            );
            Connect::Wrap
        })
}

/// 挨拶の見張りが取り消す先の、パイプの口の番号（`Stream` が生きている間だけ使う）。
pub(crate) fn raw_of(stream: &Stream) -> usize {
    match stream {
        Stream::NamedPipe(pipe) => pipe.as_handle().as_raw_handle() as usize,
    }
}

/// 口（`raw_of`）で待っている読みを取り消す（取り消されたスレッドの読みは誤りで返る。取り消すものが無ければ何もしない）。口が生きている間だけ呼ぶ。
pub(crate) fn cancel_reads(raw: usize) {
    unsafe { CancelIoEx(raw as HANDLE, std::ptr::null()) };
}

/// つないだ相手のプロセスの番号（サーバーの側ならクライアント、クライアントの側ならサーバー）。
pub(crate) fn peer_pid(stream: &Stream) -> io::Result<u32> {
    use interprocess::local_socket::traits::StreamCommon;
    stream
        .peer_creds()?
        .pid()
        .ok_or_else(|| io::Error::other("相手のプロセスの番号が分かりません"))
}

/// つないだ相手のプロセスの持ち主が自分のアカウントか（`private::judge_peer_account`）。
pub(crate) fn check_owner(stream: &Stream) -> io::Result<()> {
    let own = crate::private::win::user_sid()?;
    let peer = peer_pid(stream).and_then(crate::private::win::process_user_sid);
    crate::private::judge_peer_account(&own, peer)
}

/// 挨拶の見張り: 時間が来たら、待っている読みを取り消す。`finish` で止める（止めた後は、口に何もしない）。
pub(crate) struct Watchdog {
    state: Arc<(Mutex<State>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct State {
    done: bool,
    fired: bool,
}

impl Watchdog {
    /// `raw` の口（`raw_of`）の読みを、`timeout` 後に取り消す見張りを始める。呼び手は、口を閉じる前に `finish` する。
    pub(crate) fn start(raw: usize, timeout: Duration) -> Watchdog {
        let state = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let shared = state.clone();
        let thread = std::thread::Builder::new()
            .name("yolu-link-handshake-watchdog".into())
            .spawn(move || {
                let (lock, cv) = &*shared;
                let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
                let deadline = Instant::now() + timeout;
                while !guard.done {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        // 錠を持ったまま取り消す: `finish` は錠が取れてから `done` を立てるので、口を閉じた後に取り消すことはない
                        guard.fired = true;
                        unsafe { CancelIoEx(raw as HANDLE, std::ptr::null()) };
                        return;
                    }
                    guard = cv
                        .wait_timeout(guard, left)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
            })
            .ok();
        Watchdog { state, thread }
    }

    /// 見張りを止める。時間が来て取り消したなら true。
    pub(crate) fn finish(mut self) -> bool {
        self.stop()
    }

    fn stop(&mut self) -> bool {
        let (lock, cv) = &*self.state;
        {
            let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            guard.done = true;
            cv.notify_all();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        lock.lock().unwrap_or_else(|e| e.into_inner()).fired
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop();
    }
}
