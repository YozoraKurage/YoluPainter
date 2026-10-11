//! 更新の通信（HTTPS の GET だけ）。`yolu_update::Transport` の実装で、署名・SHA-256・大きさの確かめは yolu-update が持つ。
//! ここは「HTTPS で取る・上限を超えたら止める・取り消せる・時間を切る」だけを守る。
//!
//! - Windows は WinHTTP（OS の証明書の保管庫・プロキシの設定・TLS を使う。新しい通信の部品を足さない）。
//! - それ以外（Linux）は `curl`（プロトコルを https に固定し、設定ファイルを読ませない）。無ければ通信できず、更新の確認が失敗として出る。
//!
//! 常に `https://` が必要。試験だけが、同じ機械の `http://127.0.0.1` を許す口（`for_loopback_test`）を使う。
//!
//! 転送（リダイレクト）: curl は 5 回までで、転送先も https だけ。WinHTTP は 5 回までに絞る設定を試み、設定できない環境では
//! OS の既定（10 回）のまま進める。https から http への転送は WinHTTP の既定が断る。どちらも、止まらない転送の連なりは失敗にする
//! （試験は同じ機械の http の相手で見る。https から http への転送を、TLS の相手で通して確かめる試験は無い）。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use yolu_update::{Error, Transport};

/// 通信の途中経過（別のスレッドが読む）と取消。
#[derive(Clone, Default)]
pub struct Link {
    pub cancel: Arc<AtomicBool>,
    /// 今の取得で読んだバイト数。
    pub progress: Arc<AtomicU64>,
}

impl Link {
    pub fn is_canceled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// 取り消されたときの失敗の文（呼ぶ側は `Link::is_canceled` で見分け、この文は見せない）。
const CANCELED: &str = "取り消しました";
const USER_AGENT: &str = concat!("YoluPainter/", env!("CARGO_PKG_VERSION"));
const CHUNK: usize = 64 * 1024;

pub struct HttpTransport {
    link: Link,
    loopback_http: bool,
}

impl HttpTransport {
    pub fn new(link: Link) -> HttpTransport {
        HttpTransport {
            link,
            loopback_http: false,
        }
    }

    /// 試験用: 同じ機械の `http://127.0.0.1`・`http://localhost` も許す（証明書つきの相手を用意せずに、通信の部分を動かすため）。
    #[doc(hidden)]
    pub fn for_loopback_test(link: Link) -> HttpTransport {
        HttpTransport {
            link,
            loopback_http: true,
        }
    }
}

impl Transport for HttpTransport {
    fn get(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, Error> {
        let url = Url::parse(url, self.loopback_http)?;
        self.link.progress.store(0, Ordering::Relaxed);
        let mut body = Body {
            bytes: Vec::new(),
            max: max_bytes,
            link: &self.link,
        };
        platform::fetch(&url, &mut body)?;
        Ok(body.bytes)
    }
}

/// 読んだ中身を貯める。上限を超えたら止め、取消を見る。
struct Body<'a> {
    bytes: Vec<u8>,
    max: usize,
    link: &'a Link,
}

impl Body<'_> {
    fn push(&mut self, chunk: &[u8]) -> Result<(), Error> {
        if self.link.is_canceled() {
            return Err(Error(CANCELED.into()));
        }
        if self.bytes.len() + chunk.len() > self.max {
            return Err(Error("大きさの上限を超えました".into()));
        }
        self.bytes.extend_from_slice(chunk);
        self.link
            .progress
            .store(self.bytes.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    /// 上限を超える長さを名乗る相手は、読む前に断る（Windows。curl は `--max-filesize` を使わず、読みながら止める）。
    #[cfg(windows)]
    fn announce(&self, length: u64) -> Result<(), Error> {
        if length > self.max as u64 {
            return Err(Error("大きさの上限を超えます".into()));
        }
        Ok(())
    }
}

/// 取得先の URL（検査済み）。
#[derive(Debug, PartialEq, Eq)]
struct Url {
    text: String,
    secure: bool,
    host: String,
    port: u16,
    /// パスと問い合わせ（`/` から）。
    target: String,
}

impl Url {
    fn parse(text: &str, loopback_http: bool) -> Result<Url, Error> {
        let fail = |message: &str| Err(Error(message.into()));
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_graphic()) {
            return fail("URL の文字が不正です");
        }
        let (secure, rest) = if let Some(rest) = text.strip_prefix("https://") {
            (true, rest)
        } else if let (true, Some(rest)) = (loopback_http, text.strip_prefix("http://")) {
            (false, rest)
        } else {
            return fail("HTTPS が必要です");
        };
        let rest = rest.split('#').next().unwrap_or_default();
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        let (authority, target) = rest.split_at(end);
        let target = if target.starts_with('/') {
            target.to_owned()
        } else {
            format!("/{target}")
        };
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => match port.parse::<u16>() {
                Ok(port) if port != 0 => (host, port),
                _ => return fail("ポートが不正です"),
            },
            None => (authority, if secure { 443 } else { 80 }),
        };
        let host_ok = !host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
        if !host_ok {
            return fail("ホスト名が不正です");
        }
        if !secure && host != "127.0.0.1" && host != "localhost" {
            return fail("HTTPS が必要です");
        }
        Ok(Url {
            text: text.to_owned(),
            secure,
            host: host.to_owned(),
            port,
            target,
        })
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    use windows::core::PCWSTR;
    use windows::Win32::Networking::WinHttp::*;

    use super::{Body, Error, Url, CHUNK, USER_AGENT};

    /// 閉じ忘れないための包み。
    struct Handle(*mut c_void);

    impl Handle {
        fn open(raw: *mut c_void, what: &str) -> Result<Handle, Error> {
            if raw.is_null() {
                Err(Error(format!("通信を始められません（{what}）")))
            } else {
                Ok(Handle(raw))
            }
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: `open` が WinHTTP から受けた有効な口で、ここでしか閉じない。
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn failed(what: &str, error: windows::core::Error) -> Error {
        Error(format!("通信に失敗しました（{what}: {error}）"))
    }

    pub fn fetch(url: &Url, body: &mut Body<'_>) -> Result<(), Error> {
        let agent = wide(USER_AGENT);
        let host = wide(&url.host);
        let target = wide(&url.target);
        // SAFETY: WinHTTP の口は `Handle` が閉じる。渡す文字列は、この関数が終わるまで生きている NUL 終端の UTF-16。
        unsafe {
            // 自動のプロキシ（Windows 8.1 以降）。使えない環境では OS の既定のプロキシへ。
            let mut raw = WinHttpOpen(
                PCWSTR(agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            );
            if raw.is_null() {
                raw = WinHttpOpen(
                    PCWSTR(agent.as_ptr()),
                    WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                    PCWSTR::null(),
                    PCWSTR::null(),
                    0,
                );
            }
            let session = Handle::open(raw, "セッション")?;
            // 名前の解決・接続・送信・受信（ミリ秒）。止まったままにしない。
            WinHttpSetTimeouts(session.0, 10_000, 10_000, 30_000, 15_000)
                .map_err(|e| failed("時間切れの設定", e))?;
            let connection = Handle::open(
                WinHttpConnect(session.0, PCWSTR(host.as_ptr()), url.port, 0),
                "接続",
            )?;
            let flags = if url.secure {
                WINHTTP_FLAG_SECURE
            } else {
                WINHTTP_OPEN_REQUEST_FLAGS(0)
            };
            let request = Handle::open(
                WinHttpOpenRequest(
                    connection.0,
                    windows::core::w!("GET"),
                    PCWSTR(target.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    std::ptr::null(),
                    flags,
                ),
                "要求",
            )?;
            // 転送の上限を 5 回に絞り（既定は 10 回）、HTTPS から HTTP へは行かないと明示する（これは既定どおり）。
            // 設定できない環境（Wine など）では既定のまま進めるので、保証は「OS の既定の回数まで」。
            let _ = WinHttpSetOption(
                Some(request.0 as *const c_void),
                WINHTTP_OPTION_REDIRECT_POLICY,
                Some(&WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP.to_ne_bytes()),
            );
            let _ = WinHttpSetOption(
                Some(request.0 as *const c_void),
                WINHTTP_OPTION_MAX_HTTP_AUTOMATIC_REDIRECTS,
                Some(&5u32.to_ne_bytes()),
            );
            WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(|e| failed("送信", e))?;
            WinHttpReceiveResponse(request.0, std::ptr::null_mut())
                .map_err(|e| failed("受信", e))?;

            let mut status = 0u32;
            let mut size = std::mem::size_of::<u32>() as u32;
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut status as *mut u32 as *mut c_void),
                &mut size,
                std::ptr::null_mut(),
            )
            .map_err(|e| failed("状態", e))?;
            if status != 200 {
                return Err(Error(format!("サーバーが {status} を返しました")));
            }
            // 長さを名乗る相手は、読む前に上限と照らす（名乗らない転送では、読みながら止める）。
            let mut length = 0u32;
            let mut size = std::mem::size_of::<u32>() as u32;
            if WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut length as *mut u32 as *mut c_void),
                &mut size,
                std::ptr::null_mut(),
            )
            .is_ok()
            {
                body.announce(u64::from(length))?;
            }
            let mut chunk = vec![0u8; CHUNK];
            loop {
                if body.link.is_canceled() {
                    return Err(Error(super::CANCELED.into()));
                }
                let mut read = 0u32;
                WinHttpReadData(
                    request.0,
                    chunk.as_mut_ptr() as *mut c_void,
                    chunk.len() as u32,
                    &mut read,
                )
                .map_err(|e| failed("読み込み", e))?;
                if read == 0 {
                    return Ok(());
                }
                body.push(&chunk[..read as usize])?;
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::time::Duration;

    use super::{Body, Error, Url, CHUNK, USER_AGENT};

    /// curl に渡す引数。-q: 利用者の .curlrc を読まない。転送は 5 回まで。プロトコルは https だけ（転送先も）。
    pub(super) fn curl_args(url: &Url, limit_seconds: u32) -> Vec<String> {
        let mut args: Vec<String> = [
            "-q",
            "--silent",
            "--show-error",
            "--fail",
            "--location",
            "--max-redirs",
            "5",
            "--connect-timeout",
            "10",
            "--max-time",
        ]
        .map(String::from)
        .to_vec();
        args.extend([
            limit_seconds.to_string(),
            "--user-agent".into(),
            USER_AGENT.into(),
        ]);
        let more: &[&str] = if url.secure {
            &["--proto", "=https", "--proto-redir", "=https", "--tlsv1.2"]
        } else {
            // 試験用の同じ機械への http（`Url::parse` が許したときだけ）
            &[
                "--proto",
                "=http,https",
                "--proto-redir",
                "=http,https",
                "--noproxy",
                "*",
            ]
        };
        args.extend(more.iter().map(|a| (*a).to_owned()));
        args.extend(["--output", "-", "--", url.text.as_str()].map(String::from));
        args
    }

    pub fn fetch(url: &Url, body: &mut Body<'_>) -> Result<(), Error> {
        // 更新情報（小さい）は短く、大きい取得は長く待つ。
        let limit = if body.max <= 1024 * 1024 { 30 } else { 900 };
        let mut command = Command::new("curl");
        command.args(curl_args(url, limit));
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|e| Error(format!("curl を起動できません（{e}）")))?;
        let mut stdout = child.stdout.take().expect("piped");
        // 読むのは別のスレッド（読み込みの待ちの間も取消を見られるように）。
        let (sender, receiver) = channel::<Vec<u8>>();
        let reader = std::thread::spawn(move || {
            let mut chunk = vec![0u8; CHUNK];
            while let Ok(read) = stdout.read(&mut chunk) {
                if read == 0 || sender.send(chunk[..read].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut outcome = Ok(());
        loop {
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(chunk) => {
                    if let Err(e) = body.push(&chunk) {
                        outcome = Err(e);
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if body.link.is_canceled() {
                        outcome = Err(Error(super::CANCELED.into()));
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        if outcome.is_err() {
            let _ = child.kill();
        }
        let status = child.wait();
        drop(receiver);
        let _ = reader.join();
        outcome?;
        match status {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(Error(format!(
                "curl が失敗しました（終了コード {}）",
                status.code().map_or("なし".to_owned(), |c| c.to_string())
            ))),
            Err(e) => Err(Error(format!("curl を待てません（{e}）"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    fn parse(text: &str) -> Result<Url, Error> {
        Url::parse(text, false)
    }

    #[test]
    fn only_https_urls_with_plain_hosts_are_taken() {
        let url = parse("https://github.com/o/r/releases/latest/download/u.json").unwrap();
        assert_eq!(
            (url.host.as_str(), url.port, url.secure),
            ("github.com", 443, true)
        );
        assert_eq!(url.target, "/o/r/releases/latest/download/u.json");
        assert_eq!(
            parse("https://example.org:8443/a?b=1#frag").unwrap().target,
            "/a?b=1"
        );
        assert_eq!(parse("https://example.org").unwrap().target, "/");
        assert_eq!(parse("https://example.org?x=1").unwrap().target, "/?x=1");
        for bad in [
            "",
            "http://github.com/x",
            "ftp://github.com/x",
            "file:///etc/passwd",
            "//github.com/x",
            "https://",
            "https://user@github.com/x",
            "https://github.com:0/x",
            "https://github.com:99999/x",
            "https://git hub.com/x",
            "https://github.com/a b",
            "https://github.com/日本",
            "https://[::1]/x",
            "https://exa_mple.org/x",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn plain_http_is_only_for_this_machine_and_only_when_a_test_asks() {
        assert!(Url::parse("http://127.0.0.1:8080/x", true).is_ok());
        assert!(Url::parse("http://localhost:8080/x", true).is_ok());
        assert!(Url::parse("http://github.com/x", true).is_err());
        assert!(Url::parse("http://127.0.0.1:8080/x", false).is_err());
        // 本番の口は、同じ機械でも http を断る。
        let transport = HttpTransport::new(Link::default());
        assert!(transport.get("http://127.0.0.1:1/x", 10).is_err());
    }

    /// 1 回の接続に決まった返事を返す小さなサーバー（要求の 1 行目を記録する）。
    struct Server {
        port: u16,
        requests: std::sync::mpsc::Receiver<String>,
    }

    type Responder = Box<dyn Fn(&str, &mut TcpStream) + Send>;

    fn serve(connections: usize, responder: Responder) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sender, requests) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for _ in 0..connections {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        break;
                    }
                    head.push(byte[0]);
                }
                let line = String::from_utf8_lossy(&head)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let _ = sender.send(line.clone());
                responder(&line, &mut stream);
            }
        });
        Server { port, requests }
    }

    fn respond(stream: &mut TcpStream, status: &str, headers: &str, body: &[u8]) {
        let head = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body);
    }

    fn get(port: u16, path: &str, max: usize, link: Link) -> Result<Vec<u8>, Error> {
        HttpTransport::for_loopback_test(link).get(&format!("http://127.0.0.1:{port}{path}"), max)
    }

    #[test]
    fn a_plain_get_returns_the_body_and_reports_progress() {
        let server = serve(
            1,
            Box::new(|_, s| {
                respond(
                    s,
                    "200 OK",
                    "Content-Type: application/json\r\n",
                    b"{\"a\":1}",
                )
            }),
        );
        let link = Link::default();
        let body = get(server.port, "/updater-v1.json", 100, link.clone()).unwrap();
        assert_eq!(body, b"{\"a\":1}");
        assert_eq!(link.progress.load(Ordering::Relaxed), 7);
        let line = server
            .requests
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(line.starts_with("GET /updater-v1.json HTTP/1."), "{line}");
    }

    #[test]
    fn redirects_are_followed_like_github_release_downloads() {
        let server = serve(
            2,
            Box::new(|line, s| {
                if line.starts_with("GET /latest ") {
                    respond(s, "302 Found", "Location: /real\r\n", b"");
                } else {
                    respond(s, "200 OK", "", b"final");
                }
            }),
        );
        assert_eq!(
            get(server.port, "/latest", 100, Link::default()).unwrap(),
            b"final"
        );
    }

    #[test]
    fn endless_redirects_are_a_failure_not_a_loop() {
        // 自分へ転送し続ける相手。上限（curl は 5 回、WinHTTP は 5 回か既定の 10 回）で止まり、失敗にする。
        // 用意した接続の数より前に止まったこと（相手が尽きたからではないこと）を、受けた要求の数で見る。
        const PREPARED: usize = 14;
        let server = serve(
            PREPARED,
            Box::new(|line, s| {
                let n: usize = line
                    .split(['/', ' '])
                    .find_map(|part| part.parse().ok())
                    .unwrap_or(0);
                respond(
                    s,
                    "302 Found",
                    &format!("Location: /hop/{}\r\n", n + 1),
                    b"",
                );
            }),
        );
        let result = get(server.port, "/hop/0", 100, Link::default());
        assert!(result.is_err());
        let seen = server.requests.try_iter().count();
        assert!(
            (6..=11).contains(&seen),
            "上限を超えて転送を追った、または追わなかった: {seen} 回"
        );
        // 使わなかった接続を空け、相手のスレッドを終わらせる。
        for _ in seen..PREPARED {
            drop(TcpStream::connect(("127.0.0.1", server.port)));
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn curl_follows_five_redirects_at_most_and_only_to_https() {
        let args = |text: &str, loopback: bool| {
            platform::curl_args(&Url::parse(text, loopback).unwrap(), 30)
        };
        let pair = |args: &[String], key: &str| {
            let at = args.iter().position(|a| a == key).unwrap();
            args[at + 1].clone()
        };
        let secure = args(
            "https://github.com/o/r/releases/latest/download/u.json",
            false,
        );
        assert_eq!(pair(&secure, "--max-redirs"), "5");
        assert_eq!(pair(&secure, "--proto"), "=https");
        assert_eq!(pair(&secure, "--proto-redir"), "=https");
        assert!(
            secure.contains(&"-q".to_owned()),
            "利用者の .curlrc を読まない"
        );
        assert_eq!(
            secure.last().unwrap(),
            "https://github.com/o/r/releases/latest/download/u.json"
        );
        // 試験用の同じ機械への http だけが、http を許す（本番の URL は https しか通らない）
        let local = args("http://127.0.0.1:8080/x", true);
        assert_eq!(pair(&local, "--max-redirs"), "5");
        assert_eq!(pair(&local, "--proto-redir"), "=http,https");
    }

    /// curl に渡す引数の全体を固定する。オプションを足す（`--data`・`-H`・`-b`・`-T`・`-K`・`--url`・`--referer`・`--cookie` など）と落ちる:
    /// アプリの名前と版（User-Agent）のほかに、利用者を識別する情報は付けない約束（INSTALL）と、tools/network-watch.py の許す形が、この全体に基づく。
    #[cfg(not(windows))]
    #[test]
    fn curl_arguments_are_exactly_these() {
        let url =
            "https://github.com/YozoraKurage/YoluPainter/releases/latest/download/updater-v1.json";
        let expect = |limit: &str| -> Vec<String> {
            [
                "-q",
                "--silent",
                "--show-error",
                "--fail",
                "--location",
                "--max-redirs",
                "5",
                "--connect-timeout",
                "10",
                "--max-time",
                limit,
                "--user-agent",
                &format!("YoluPainter/{}", env!("CARGO_PKG_VERSION")),
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--tlsv1.2",
                "--output",
                "-",
                "--",
                url,
            ]
            .map(String::from)
            .to_vec()
        };
        let parsed = Url::parse(url, false).unwrap();
        assert_eq!(platform::curl_args(&parsed, 30), expect("30"));
        assert_eq!(
            platform::curl_args(&parsed, 900),
            expect("900"),
            "大きい取得は待つ時間だけが違う"
        );
        // URL は引数の最後の 1 つだけで、問い合わせ（?）や識別子を足す口は無い
        assert_eq!(platform::curl_args(&parsed, 30).last().unwrap(), url);
        assert!(!url.contains('?'));
    }

    #[test]
    fn error_statuses_are_failures() {
        for status in ["404 Not Found", "500 Internal Server Error"] {
            let server = serve(1, Box::new(move |_, s| respond(s, status, "", b"nope")));
            assert!(
                get(server.port, "/x", 100, Link::default()).is_err(),
                "{status}"
            );
        }
    }

    #[test]
    fn a_body_over_the_limit_is_refused_whether_announced_or_streamed() {
        // 長さを名乗る
        let server = serve(1, Box::new(|_, s| respond(s, "200 OK", "", &[b'x'; 64])));
        assert!(get(server.port, "/x", 63, Link::default()).is_err());
        // ちょうど上限は通る
        let server = serve(1, Box::new(|_, s| respond(s, "200 OK", "", &[b'x'; 64])));
        assert_eq!(
            get(server.port, "/x", 64, Link::default()).unwrap().len(),
            64
        );
        // 名乗らずに流す（chunked）: 読みながら止める
        let server = serve(
            1,
            Box::new(|_, s| {
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                );
                for _ in 0..50 {
                    if s.write_all(b"40\r\n").is_err()
                        || s.write_all(&[b'y'; 64]).is_err()
                        || s.write_all(b"\r\n").is_err()
                    {
                        return;
                    }
                }
                let _ = s.write_all(b"0\r\n\r\n");
            }),
        );
        assert!(get(server.port, "/x", 1000, Link::default()).is_err());
    }

    #[test]
    fn canceling_stops_a_stalled_transfer_promptly() {
        let server = serve(
            1,
            Box::new(|_, s| {
                // 名乗った長さの途中で止まる（切るまで黙る）
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\nabc",
                );
                std::thread::sleep(Duration::from_secs(20));
            }),
        );
        let link = Link::default();
        let flag = link.cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            flag.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let result = get(server.port, "/slow", 2000, link.clone());
        assert!(result.is_err());
        assert!(link.is_canceled());
        // curl（Linux）は読み込みの待ちの間も取消を見る。WinHTTP の読み込みは待ちの間は抜けられず、
        // 受信の時間切れ（15 秒）で抜ける。どちらも、止まった相手を待ち続けない。
        let limit = if cfg!(windows) { 25 } else { 5 };
        assert!(
            started.elapsed() < Duration::from_secs(limit),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn nothing_listening_is_a_failure_not_a_hang() {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let started = Instant::now();
        assert!(get(port, "/x", 100, Link::default()).is_err());
        assert!(started.elapsed() < Duration::from_secs(15));
    }
}
