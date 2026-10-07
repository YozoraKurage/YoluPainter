//! 手元の HTTP の受け口: 127.0.0.1 だけで待ち、`/mcp` の Streamable HTTP（rmcp）へ渡す。
//!
//! - 待つのは 127.0.0.1（IPv4 の自分）だけ。0.0.0.0・ほかの口では待たない。合言葉は無い（同じ PC のプログラムなら、ほかのアカウントのものでもつなげる）。
//! - rmcp に渡す前に、`Host` が `127.0.0.1:<番号>` か `localhost:<番号>` であること、`Origin` があればそれも同じ 2 つの
//!   `http://` のどちらかであることを確かめ、違えば 403 で断る（ブラウザーの DNS rebinding と、別のサイトのページからの要求を断る）。
//!   rmcp の同じ確かめも同じ一覧で入れておく（二重）。
//! - 同時のつながりは [`MAX_CONNECTIONS`] まで。超えたつながりには 503 を返して閉じる。要求の本文は [`MAX_BODY_BYTES`] まで（rmcp が 413 で断る）。
//!   頭（ヘッダー）が [`HEADER_TIMEOUT`] のうちに届かないつながりは閉じる。
//! - rmcp は状態を持たない形（要求ごとに独立。`Mcp-Session-Id` を出さない）で、返事は JSON（`application/json`）。GET・DELETE は 405。
//! - 止める合図（`CancellationToken`）で、新しいつながりを受けるのをやめ、走っている要求の返事を書き終えるのを少し待ってから閉じる。

use std::convert::Infallible;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdListener};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderMap, CONNECTION, CONTENT_TYPE, HOST, ORIGIN};
use hyper::{Request, Response, StatusCode, Uri};
use hyper_util::rt::{TokioIo, TokioTimer};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::server::YoluMcp;
use crate::PATH;

/// 同時につなげる数。
pub const MAX_CONNECTIONS: usize = 8;
/// 要求の本文の上限（64 MiB）。
pub const MAX_BODY_BYTES: usize = 64 << 20;
/// 要求の頭を待つ長さ。
pub const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// 止めるとき、走っている要求の返事を書き終えるのを待つ長さ。
const GRACE: Duration = Duration::from_millis(500);

/// 受け口の決まり。
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_connections: usize,
    pub max_body_bytes: usize,
    pub header_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_connections: MAX_CONNECTIONS,
            max_body_bytes: MAX_BODY_BYTES,
            header_timeout: HEADER_TIMEOUT,
        }
    }
}

/// 断った理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// `Host` が自分（127.0.0.1・localhost とこの番号）でない。
    Host,
    /// `Origin` があって、自分でない。
    Origin,
    /// つながりの数が上限。
    Busy,
}

/// 受け口の様子を受け取る側（アプリは状態の帯の印と知らせに使う）。受け口のスレッドから呼ばれる。
pub trait Observer: Send + Sync + 'static {
    /// 開いているつながりの数が変わった。
    fn connections(&self, open: usize);
    /// 要求・つながりを断った。
    fn refused(&self, refusal: Refusal);
}

/// 何もしない `Observer`。
pub struct Quiet;

impl Observer for Quiet {
    fn connections(&self, _open: usize) {}
    fn refused(&self, _refusal: Refusal) {}
}

/// 127.0.0.1 の番号で待つ口を開く（番号が使われていれば `AddrInUse`）。0 なら OS が空いた番号を選ぶ（試験）。
pub fn bind(port: u16) -> io::Result<StdListener> {
    let listener = StdListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

/// `Host`・URI の権限の部分（`host:port`）が、自分（`127.0.0.1` か `localhost` と、この番号）か。
fn own_authority(text: &str, port: u16) -> bool {
    let Some((host, p)) = text.rsplit_once(':') else {
        return false;
    };
    p == port.to_string() && (host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost"))
}

/// 1 つだけの頭の値（無い・2 つ以上・文字にできないは None の中の誤り）。
fn single<'a>(
    headers: &'a HeaderMap,
    name: &hyper::header::HeaderName,
) -> Result<Option<&'a str>, ()> {
    let mut values = headers.get_all(name).iter();
    let Some(first) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(());
    }
    first.to_str().map(Some).map_err(|_| ())
}

/// 要求の `Host`（と URI の権限）・`Origin` を確かめる。
pub fn check(headers: &HeaderMap, uri: &Uri, port: u16) -> Result<(), Refusal> {
    let host = single(headers, &HOST).map_err(|()| Refusal::Host)?;
    if !host.is_some_and(|h| own_authority(h, port)) {
        return Err(Refusal::Host);
    }
    if let Some(authority) = uri.authority() {
        if !own_authority(authority.as_str(), port) {
            return Err(Refusal::Host);
        }
    }
    match single(headers, &ORIGIN).map_err(|()| Refusal::Origin)? {
        None => Ok(()),
        Some(origin) => {
            let own = origin
                .get(..7)
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("http://"))
                && own_authority(&origin[7..], port);
            if own {
                Ok(())
            } else {
                Err(Refusal::Origin)
            }
        }
    }
}

/// rmcp の確かめにも入れる、自分の `Host` と `Origin` の一覧。
fn own_hosts(port: u16) -> Vec<String> {
    vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")]
}

fn own_origins(port: u16) -> Vec<String> {
    vec![
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ]
}

type Body = BoxBody<Bytes, Infallible>;
type Mcp = StreamableHttpService<YoluMcp, NeverSessionManager>;

fn plain(status: StatusCode, text: &'static str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from_static(text.as_bytes())).boxed())
        .expect("固定の返事は組める")
}

async fn route(
    request: Request<Incoming>,
    port: u16,
    mcp: Mcp,
    observer: Arc<dyn Observer>,
) -> Response<Body> {
    if let Err(refusal) = check(request.headers(), request.uri(), port) {
        observer.refused(refusal);
        return plain(
            StatusCode::FORBIDDEN,
            match refusal {
                Refusal::Origin => "Forbidden: the Origin is not this server",
                _ => "Forbidden: the Host is not this server",
            },
        );
    }
    if request.uri().path() != PATH {
        return plain(StatusCode::NOT_FOUND, "Not Found");
    }
    mcp.handle(request).await
}

/// 開いているつながりの数（落とすと減らして知らせる）。
struct Open {
    count: Arc<AtomicUsize>,
    observer: Arc<dyn Observer>,
}

impl Open {
    fn new(count: Arc<AtomicUsize>, observer: Arc<dyn Observer>) -> Open {
        let n = count.fetch_add(1, Ordering::AcqRel) + 1;
        observer.connections(n);
        Open { count, observer }
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        let n = self.count.fetch_sub(1, Ordering::AcqRel) - 1;
        self.observer.connections(n);
    }
}

/// 待つ口から受けて、止める合図まで答え続ける。返るのは、つながりを全部閉じたあと。
pub async fn serve(
    listener: StdListener,
    mcp: YoluMcp,
    limits: Limits,
    observer: Arc<dyn Observer>,
    cancel: CancellationToken,
) -> io::Result<()> {
    let port = listener.local_addr()?.port();
    let listener = tokio::net::TcpListener::from_std(listener)?;
    // rmcp の止める合図は、走っている要求の返事を書き終えたあとに出す（先に出すと、返事を待っている要求が 500 になる）
    let finished = CancellationToken::new();
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_allowed_hosts(own_hosts(port))
        .with_allowed_origins(own_origins(port))
        .with_max_request_body_bytes(limits.max_body_bytes)
        .with_cancellation_token(finished.clone());
    let service: Mcp = StreamableHttpService::new(
        move || Ok(mcp.clone()),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    let open = Arc::new(AtomicUsize::new(0));
    let mut connections = JoinSet::new();
    // 上限を超えて断るつながり（止めるときは待たずに捨てる）
    let mut refusals = JoinSet::new();
    loop {
        let stream = tokio::select! {
            _ = cancel.cancelled() => break,
            // 終わったつながりの後始末（JoinSet に溜めない）
            Some(_) = connections.join_next(), if !connections.is_empty() => continue,
            Some(_) = refusals.join_next(), if !refusals.is_empty() => continue,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                // 手の数の上限などの一時の失敗。少し待って受け直す
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
        };
        let _ = stream.set_nodelay(true);
        let io = TokioIo::new(stream);
        if open.load(Ordering::Acquire) >= limits.max_connections {
            observer.refused(Refusal::Busy);
            refusals.spawn(refuse_busy(io, limits.header_timeout));
            continue;
        }
        let guard = Open::new(open.clone(), observer.clone());
        let (service, observer, cancel) = (service.clone(), observer.clone(), cancel.clone());
        connections.spawn(async move {
            let _guard = guard;
            let handler = hyper::service::service_fn(move |request: Request<Incoming>| {
                let (service, observer) = (service.clone(), observer.clone());
                async move { Ok::<_, Infallible>(route(request, port, service, observer).await) }
            });
            let mut builder = hyper::server::conn::http1::Builder::new();
            builder
                .timer(TokioTimer::new())
                .header_read_timeout(limits.header_timeout)
                .keep_alive(true);
            let connection = builder.serve_connection(io, handler);
            tokio::pin!(connection);
            tokio::select! {
                _ = connection.as_mut() => {}
                _ = cancel.cancelled() => {
                    connection.as_mut().graceful_shutdown();
                    let _ = tokio::time::timeout(GRACE, connection.as_mut()).await;
                }
            }
        });
    }
    drop(listener);
    drop(refusals);
    // 走っている要求の返事を書き終える（各つながりが GRACE で切る）
    while connections.join_next().await.is_some() {}
    finished.cancel();
    Ok(())
}

/// 受け口を裏のスレッドで動かす（tokio はそのスレッドの中だけで回す）。落とす・[`Running::stop`] で止める（走っている要求の返事を
/// 書き終えるのを少し待ち、口を閉じてから返る。止めたあとは同じ番号をすぐに開き直せる）。
pub struct Running {
    port: u16,
    cancel: CancellationToken,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    /// `bind` で開いた口で待ち始める。
    pub fn start(
        listener: StdListener,
        mcp: YoluMcp,
        limits: Limits,
        observer: Arc<dyn Observer>,
    ) -> io::Result<Running> {
        let port = listener.local_addr()?.port();
        let cancel = CancellationToken::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let thread = {
            let cancel = cancel.clone();
            std::thread::Builder::new()
                .name("yolu-mcp-http".into())
                .spawn(move || {
                    let _ = runtime.block_on(serve(listener, mcp, limits, observer, cancel));
                    // 返事を書き終えたあとに残った仕事（rmcp が要求ごとに立てる物）は待たない
                    runtime.shutdown_background();
                })?
        };
        Ok(Running {
            port,
            cancel,
            thread: Some(thread),
        })
    }

    /// 待っている番号。
    pub fn port(&self) -> u16 {
        self.port
    }

    /// 止めて、口を閉じるまで待つ。
    pub fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 上限を超えたつながり: 要求を読んで 503 を返し、閉じる。
async fn refuse_busy(io: TokioIo<tokio::net::TcpStream>, header_timeout: Duration) {
    let handler = hyper::service::service_fn(|_request: Request<Incoming>| async {
        let mut response = plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "Service Unavailable: too many connections",
        );
        response
            .headers_mut()
            .insert(CONNECTION, hyper::header::HeaderValue::from_static("close"));
        Ok::<_, Infallible>(response)
    });
    let mut builder = hyper::server::conn::http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(header_timeout)
        .keep_alive(false);
    let _ = tokio::time::timeout(
        header_timeout + GRACE,
        builder.serve_connection(io, handler),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::header::HeaderValue;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                hyper::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    fn path() -> Uri {
        Uri::from_static("/mcp")
    }

    #[test]
    fn only_this_servers_host_and_origin_pass() {
        let port = 17347;
        for host in ["127.0.0.1:17347", "localhost:17347", "LocalHost:17347"] {
            assert_eq!(
                check(&headers(&[("host", host)]), &path(), port),
                Ok(()),
                "{host}"
            );
        }
        for host in [
            "127.0.0.1",
            "localhost",
            "127.0.0.1:17348",
            "evil.example:17347",
            "localhost.evil.example:17347",
            "[::1]:17347",
            "0.0.0.0:17347",
            "127.0.0.2:17347",
            "localhost:017347",
        ] {
            assert_eq!(
                check(&headers(&[("host", host)]), &path(), port),
                Err(Refusal::Host),
                "{host}"
            );
        }
        assert_eq!(
            check(&headers(&[]), &path(), port),
            Err(Refusal::Host),
            "Host が無い"
        );
        assert_eq!(
            check(
                &headers(&[("host", "127.0.0.1:17347"), ("host", "evil.example:17347")]),
                &path(),
                port
            ),
            Err(Refusal::Host),
            "Host が 2 つ"
        );
        // 絶対の形の URI の権限も確かめる
        let absolute = Uri::from_static("http://evil.example:17347/mcp");
        assert_eq!(
            check(&headers(&[("host", "127.0.0.1:17347")]), &absolute, port),
            Err(Refusal::Host)
        );
        for origin in [
            "http://127.0.0.1:17347",
            "http://localhost:17347",
            "HTTP://LOCALHOST:17347",
        ] {
            assert_eq!(
                check(
                    &headers(&[("host", "localhost:17347"), ("origin", origin)]),
                    &path(),
                    port
                ),
                Ok(()),
                "{origin}"
            );
        }
        for origin in [
            "null",
            "https://localhost:17347",
            "http://evil.example",
            "http://evil.example:17347",
            "http://localhost:3000",
            "http://localhost",
            "http://127.0.0.1:17347/",
            "",
        ] {
            assert_eq!(
                check(
                    &headers(&[("host", "localhost:17347"), ("origin", origin)]),
                    &path(),
                    port
                ),
                Err(Refusal::Origin),
                "{origin}"
            );
        }
    }
}
