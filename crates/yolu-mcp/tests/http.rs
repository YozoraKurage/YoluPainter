//! 受け口の HTTP の決まり: 127.0.0.1 だけ・Host と Origin の確かめ・道・同時のつながりの上限・本文の上限・止めると閉じる。
//! 本物の受け口に、生の HTTP（頭を自由に書く）と本物の客で当てる。

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use common::*;
use serde_json::json;
use yolu_mcp::http::{Limits, Refusal};

fn served(fx: &Fixture, limits: Limits) -> Served {
    fx.project("a.ylp");
    Served::start_with(fx.host("a.ylp"), limits)
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "{what} を待ったが来ない");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_foreign_host_or_origin_is_refused_before_the_mcp_layer() {
    let fx = Fixture::new("guard");
    let server = served(&fx, Limits::default());
    let port = server.port;
    let own = format!("127.0.0.1:{port}");
    let local = format!("localhost:{port}");
    // 自分の Host（Origin が無い・自分の Origin）は通る
    assert_eq!(raw_status(port, &raw_request(port, &own, None)), 200);
    assert_eq!(raw_status(port, &raw_request(port, &local, None)), 200);
    let own_origin = format!("http://localhost:{port}");
    assert_eq!(
        raw_status(port, &raw_request(port, &own, Some(&own_origin))),
        200
    );
    // DNS rebinding（ほかの名前が 127.0.0.1 を指す）: Host が違うと断る
    for host in [
        format!("evil.example:{port}"),
        "127.0.0.1".to_owned(),
        format!("127.0.0.1:{}", port.wrapping_add(1)),
    ] {
        assert_eq!(
            raw_status(port, &raw_request(port, &host, None)),
            403,
            "{host}"
        );
    }
    // 別のサイトのページからの要求: Origin が違うと断る（Host が自分でも）
    for origin in ["http://evil.example", "null", "https://localhost:1"] {
        assert_eq!(
            raw_status(port, &raw_request(port, &own, Some(origin))),
            403,
            "{origin}"
        );
    }
    let refused = server.seen.refused.lock().unwrap().clone();
    assert!(refused.contains(&Refusal::Host) && refused.contains(&Refusal::Origin));
    assert_eq!(server.ran(), 0, "断った要求はアプリへ行かない");
}

#[test]
fn only_post_on_the_mcp_path_is_answered() {
    let fx = Fixture::new("paths");
    let server = served(&fx, Limits::default());
    let port = server.port;
    let request = raw_request(port, &format!("127.0.0.1:{port}"), None);
    assert_eq!(
        raw_status(port, &request.replacen("/mcp", "/other", 1)),
        404
    );
    let get = format!(
        "GET /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n"
    );
    assert_eq!(
        raw_status(port, &get),
        405,
        "状態を持たないので GET の流れは無い"
    );
    let delete =
        format!("DELETE /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    assert_eq!(raw_status(port, &delete), 405);
}

#[test]
fn the_listener_is_on_the_ipv4_loopback_only() {
    let listener = yolu_mcp::http::bind(0).unwrap();
    assert_eq!(
        listener.local_addr().unwrap().ip(),
        std::net::Ipv4Addr::LOCALHOST,
        "待つのは 127.0.0.1 だけ（0.0.0.0 にしない）"
    );
}

#[test]
fn connections_beyond_the_limit_get_503_and_a_closed_ones_place_is_given_back() {
    let fx = Fixture::new("limit");
    let limits = Limits {
        max_connections: 3,
        ..Limits::default()
    };
    let server = served(&fx, limits);
    let port = server.port;
    // 何も送らずに開けたつながり 3 つで、上限まで埋まる
    let mut held: Vec<TcpStream> = (0..3)
        .map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap())
        .collect();
    wait_until("3 つのつながり", || {
        server.seen.open.load(Ordering::Relaxed) == 3
    });
    // 4 つ目は 503
    assert_eq!(
        raw_status(port, &raw_request(port, &format!("127.0.0.1:{port}"), None)),
        503
    );
    assert!(server.seen.refused.lock().unwrap().contains(&Refusal::Busy));
    assert_eq!(
        server.seen.open.load(Ordering::Relaxed),
        3,
        "断ったつながりは数えない"
    );
    // 1 つ閉じると、その分が戻る
    drop(held.pop());
    wait_until("閉じたつながりを忘れる", || {
        server.seen.open.load(Ordering::Relaxed) == 2
    });
    assert_eq!(
        raw_status(port, &raw_request(port, &format!("127.0.0.1:{port}"), None)),
        200
    );
}

#[test]
fn a_body_over_the_limit_is_refused() {
    let fx = Fixture::new("body");
    let limits = Limits {
        max_body_bytes: 1024,
        ..Limits::default()
    };
    let server = served(&fx, limits);
    let port = server.port;
    let big = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "doc_info", "arguments": {}, "pad": "x".repeat(4096)}}).to_string();
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2025-11-25\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{big}",
        big.len()
    );
    assert_eq!(raw_status(port, &request), 413);
    assert_eq!(server.ran(), 0);
}

#[test]
fn headers_that_never_finish_are_cut_off() {
    let fx = Fixture::new("slow");
    let limits = Limits {
        header_timeout: Duration::from_millis(300),
        ..Limits::default()
    };
    let server = served(&fx, limits);
    let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    stream
        .write_all(b"POST /mcp HTTP/1.1\r\nHost: 127.0.0.1")
        .unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let started = Instant::now();
    let mut buffer = Vec::new();
    let _ = stream.read_to_end(&mut buffer);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "頭が届かないつながりは閉じる: {:?}",
        started.elapsed()
    );
    wait_until("閉じたつながりを忘れる", || {
        server.seen.open.load(Ordering::Relaxed) == 0
    });
}

#[test]
fn stopping_closes_the_listener_and_the_open_connections() {
    let fx = Fixture::new("stop");
    let mut server = served(&fx, Limits::default());
    let port = server.port;
    let (mut mcp, _) = Mcp::legacy(port);
    assert_eq!(mcp.call("doc_info", json!({}))["isError"], false);
    let mut idle = TcpStream::connect(("127.0.0.1", port)).unwrap();
    wait_until("待っているつながり", || {
        server.seen.open.load(Ordering::Relaxed) >= 1
    });
    let started = Instant::now();
    server.stop();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "止めるのに待たない"
    );
    // 待ちをやめた: つなげない（同じ番号をすぐに開き直せる）
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    let again = yolu_mcp::http::bind(port).expect("止めた番号はすぐに使える");
    drop(again);
    // 開いていたつながりも閉じた
    idle.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut buffer = [0u8; 16];
    assert!(matches!(idle.read(&mut buffer), Ok(0) | Err(_)));
}
