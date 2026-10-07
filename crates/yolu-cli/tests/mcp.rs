//! `yolupainter-cli mcp`（標準入出力 ⇔ 起動中のアプリの MCP の受け口の中継）の試験。本物の実行ファイルを起こし、標準入出力
//! （1 行 1 メッセージの JSON-RPC）で話す。相手は本物の MCP の受け口（`FakeApp`。受けた命令を本物の文書に当てる）。
//! 2025-11-25 の `initialize` の流れと、2026-07-28 の `server/discover`・要求ごとの `_meta` の流れが、アプリまで届いて戻ること、
//! アプリにつなげないときの返事、入力が閉じたら終わることを確かめる。

mod common;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use common::*;
use serde_json::{json, Value};
use yolu_mcp::http::Limits;
use yolu_ops::commands;
use yolu_ops::value::base64_decode;

struct Relay {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Value>,
    next: u64,
    /// 2026-07-28 の流れ（要求ごとの `_meta`）か。
    stateless: bool,
}

impl Relay {
    fn start(port: u16) -> Relay {
        let mut child = Command::new(env!("CARGO_BIN_EXE_yolupainter-cli"))
            .args(["mcp", "--port", &port.to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let value: Value = serde_json::from_str(&line)
                    .unwrap_or_else(|e| panic!("標準出力に JSON でない行: {e}: {line}"));
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        Relay {
            child,
            stdin: Some(stdin),
            rx,
            next: 0,
            stateless: false,
        }
    }

    /// 2025-11-25 の `initialize` で始める。
    fn legacy(port: u16) -> (Relay, Value) {
        let mut relay = Relay::start(port);
        let init = relay.result(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test-client", "version": "1"}}),
        );
        relay.notify("notifications/initialized", json!({}));
        (relay, init)
    }

    fn send(&mut self, value: Value) {
        let stdin = self.stdin.as_mut().expect("標準入力がまだ開いている");
        writeln!(stdin, "{value}").unwrap();
        stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn request(&mut self, method: &str, mut params: Value) -> Value {
        if self.stateless {
            params["_meta"] = json!({
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": {"name": "test-client", "version": "1"},
            });
        }
        self.next += 1;
        let id = self.next;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let message = self
                .rx
                .recv_timeout(Duration::from_secs(60))
                .unwrap_or_else(|e| panic!("{method} の返事が来ない: {e}"));
            if message["id"] == json!(id) {
                return message;
            }
        }
    }

    fn result(&mut self, method: &str, params: Value) -> Value {
        let response = self.request(method, params);
        assert!(response.get("error").is_none(), "{method}: {response}");
        response["result"].clone()
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.result("tools/call", json!({"name": tool, "arguments": arguments}))
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn text_json(result: &Value) -> Value {
    let text = result["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{result}"));
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

#[test]
fn the_initialize_flow_reaches_the_app_and_comes_back() {
    let fx = Fixture::new("relay-init");
    fx.project("a.ylp");
    let app = FakeApp::start(&fx, "a.ylp", Behavior::Serve);
    let (mut relay, init) = Relay::legacy(app.port);
    // 版の取り決め・名乗り・手引きは、アプリ（受け口）が答えた物
    assert_eq!(init["protocolVersion"], "2025-11-25");
    assert_eq!(init["serverInfo"]["name"], "yolupainter");
    assert!(init["capabilities"]["tools"].is_object());
    let list = relay.result("tools/list", json!({}));
    assert_eq!(
        list["tools"].as_array().unwrap().len(),
        commands().len(),
        "ツールの一覧はアプリが持つ"
    );
    let edited = relay.call("layer_set", json!({"layer": "Base", "opacity": 0.2}));
    assert_eq!(edited["isError"], false);
    assert_eq!(
        relay.call("history_info", json!({}))["structuredContent"]["undo_count"],
        1
    );
    assert_eq!(app.served(), 2);
    let refused = relay.call("layer_delete", json!({"layer": "Tint"}));
    assert_eq!(text_json(&refused)["code"], "confirm_required");
    // 画像も通る
    let preview = relay.call("preview", json!({"max_edge": 16}));
    let png = base64_decode(preview["content"][1]["data"].as_str().unwrap()).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    let uri = preview["content"][2]["uri"].as_str().unwrap().to_owned();
    let read = relay.result("resources/read", json!({"uri": uri}));
    assert_eq!(
        base64_decode(read["contents"][0]["blob"].as_str().unwrap()).unwrap(),
        png
    );
    // 知らないツールは、アプリの JSON-RPC の誤りがそのまま来る
    let unknown = relay.request(
        "tools/call",
        json!({"name": "no_such_tool", "arguments": {}}),
    );
    assert!(unknown["error"].is_object(), "{unknown}");
}

#[test]
fn the_stateless_2026_07_28_flow_reaches_the_app() {
    let fx = Fixture::new("relay-2026");
    fx.project("a.ylp");
    let app = FakeApp::start(&fx, "a.ylp", Behavior::Serve);
    let mut relay = Relay::start(app.port);
    relay.stateless = true;
    let discover = relay.result("server/discover", json!({}));
    assert!(discover["supportedVersions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "2026-07-28"));
    let list = relay.result("tools/list", json!({}));
    assert_eq!(list["resultType"], "complete");
    let called = relay.call("set_info", json!({}));
    assert_eq!(called["resultType"], "complete");
    assert_eq!(called["structuredContent"]["name"], "Body");
    let read = relay.result("resources/read", json!({"uri": "yolupainter://docs/mcp"}));
    assert!(read["contents"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("# "));
}

#[test]
fn without_a_listening_app_requests_say_why_and_how_to_fix_it() {
    let port = unused_port();
    let mut relay = Relay::start(port);
    // 版の取り決めもアプリがするので、initialize は誤り（直し方の文つき）
    let init = relay.request(
        "initialize",
        json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}),
    );
    let message = init["error"]["message"].as_str().unwrap();
    assert!(message.contains("Accept external commands"), "{message}");
    assert_eq!(init["error"]["data"]["data"]["live"], "unreachable");
    // ツールの呼び出しは、ツールの失敗（アプリの誤りと同じ形）
    let called = relay.call("doc_info", json!({}));
    assert_eq!(called["isError"], true);
    let error = text_json(&called);
    assert_eq!(error["data"]["live"], "unreachable");
    assert!(error["message"]["ja"]
        .as_str()
        .unwrap()
        .contains("外からの操作を受ける"));
    // 通知には返事をしない（次の要求の返事が先に来る）
    relay.notify("notifications/initialized", json!({}));
    let list = relay.request("tools/list", json!({}));
    assert!(list["error"].is_object());
    // あとからアプリが受け始めれば、同じ中継がつながる（つながりを持ち続けない）
    let fx = Fixture::new("relay-late");
    fx.project("a.ylp");
    let mut host = yolu_ops::FileHost::new(yolu_ops::PathPolicy::new(&fx.dir).unwrap());
    yolu_ops::OpHost::open(&mut host, &fx.path("a.ylp"), true).unwrap();
    let listener = yolu_mcp::http::bind(port);
    if let Ok(listener) = listener {
        drop(listener);
        let late = FakeApp::start_on(port, host, Behavior::Serve);
        let called = relay.call("doc_info", json!({}));
        assert_eq!(called["isError"], false, "{called}");
        assert_eq!(late.served(), 1);
    }
}

#[test]
fn the_relay_ends_quietly_when_the_client_closes_the_input() {
    let fx = Fixture::new("relay-eof");
    fx.project("a.ylp");
    let app = FakeApp::start(&fx, "a.ylp", Behavior::Serve);
    let (mut relay, _) = Relay::legacy(app.port);
    let _ = relay.result("tools/list", json!({}));
    drop(relay.stdin.take());
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = relay.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "標準入力が閉じても終わらない"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "{status:?}");
}

#[test]
fn a_line_that_is_not_json_gets_a_parse_error_and_the_relay_goes_on() {
    let fx = Fixture::new("relay-parse");
    fx.project("a.ylp");
    let app = FakeApp::start(&fx, "a.ylp", Behavior::Serve);
    let (mut relay, _) = Relay::legacy(app.port);
    let stdin = relay.stdin.as_mut().unwrap();
    writeln!(stdin, "{{not json").unwrap();
    stdin.flush().unwrap();
    let error = relay.rx.recv_timeout(Duration::from_secs(60)).unwrap();
    assert_eq!(error["error"]["code"], -32700);
    assert_eq!(relay.call("doc_info", json!({}))["isError"], false);
}

#[test]
fn an_app_at_its_connection_limit_is_busy_not_unreachable() {
    let fx = Fixture::new("relay-busy");
    fx.project("a.ylp");
    // つながり 1 つの上限を、何も送らない 1 つで埋める
    let app = FakeApp::start_limited(
        &fx,
        "a.ylp",
        Limits {
            max_connections: 1,
            ..Limits::default()
        },
    );
    let held = TcpStream::connect(("127.0.0.1", app.port)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.open.load(Ordering::Relaxed) != 1 {
        assert!(Instant::now() < deadline, "埋めたつながりを数えない");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut relay = Relay::start(app.port);
    // tools/call: ツールの失敗で、code が busy（直結の客と同じ。つなげない io ではない）
    let called = relay.call("doc_info", json!({}));
    assert_eq!(called["isError"], true, "{called}");
    let error = text_json(&called);
    assert_eq!(error["code"], "busy", "{error}");
    assert!(
        error["data"]["live"].is_null(),
        "つなげないのではない: {error}"
    );
    assert_eq!(app.served(), 0, "断られた要求はアプリの命令にならない");
    // ほかの要求は JSON-RPC の誤りに、同じ誤りが入る
    let listed = relay.request("tools/list", json!({}));
    assert_eq!(listed["error"]["data"]["code"], "busy", "{listed}");
    // 空きが戻れば、同じ中継がそのまま通る
    drop(held);
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.open.load(Ordering::Relaxed) != 0 {
        assert!(Instant::now() < deadline, "閉じたつながりを忘れない");
        std::thread::sleep(Duration::from_millis(5));
    }
    let called = relay.call("doc_info", json!({}));
    assert_eq!(called["isError"], false, "{called}");
    assert_eq!(app.served(), 1);
}
