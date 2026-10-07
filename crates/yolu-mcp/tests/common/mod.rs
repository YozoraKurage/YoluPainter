//! 試験の台: 小さな .ylp を作る・本物の HTTP の受け口を、画面なしのホスト（`FileHost`）を相手に立てる・本物の HTTP の客で話す。
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use yolu_core::{Channel, Document, Rgba8};
use yolu_io::{composite_pngs, DocumentSource, MaterialRef, Project, SaveTarget, SetSpec};
use yolu_mcp::client::{self, Exchange};
use yolu_mcp::http::{self, Limits, Observer, Refusal};
use yolu_mcp::{Backend, BackendFuture, YoluMcp};
use yolu_ops::{execute, Command, FileHost, OpHost, PathPolicy};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// 試験ごとの作業のフォルダ（落とすと消す）。
pub struct Fixture {
    pub dir: PathBuf,
}

impl Fixture {
    pub fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "yolu-mcp-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Fixture { dir }
    }
    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }
    /// 1 セットの .ylp（Base・Tint・Group の中の Inner）を書く。
    pub fn project(&self, file: &str) -> PathBuf {
        let path = self.path(file);
        let doc = sample_document();
        let composites = composite_pngs(&doc).unwrap();
        let name = "Body".to_owned();
        let spec = SetSpec {
            id: "11111111-1111-4111-8111-111111111111".into(),
            name: name.clone(),
            material: MaterialRef::Material { name, asset: None },
            document: Some(DocumentSource::from_core(Arc::new(doc)).unwrap()),
            composites,
        };
        let project =
            Project::create(yolu_ops::writer(), std::slice::from_ref(&spec), &spec.id).unwrap();
        SaveTarget::create(&path).unwrap().save(&project).unwrap();
        path
    }
    /// その .ylp を開いた画面なしのホスト（起動中のアプリの代わり）。
    pub fn host(&self, file: &str) -> FileHost {
        let mut host = FileHost::new(PathPolicy::new(&self.dir).unwrap());
        host.open(&self.path(file), true).unwrap();
        host
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 64×48 の文書: 下から Base（画素あり）・Tint（半透明の塗りつぶし）・Inner を入れた Group。
pub fn sample_document() -> Document {
    let mut doc = Document::new(64, 48).unwrap();
    let base = doc.add_layer("Base").unwrap();
    for y in 0..48 {
        for x in 0..64 {
            let c = Rgba8::new(
                (x * 4) as u8,
                (y * 5) as u8,
                90,
                if (x + y) % 7 == 0 { 0 } else { 255 },
            );
            doc.set_pixel(base, x, y, c).unwrap();
        }
    }
    doc.add_fill_layer(
        "Tint",
        &[(Channel::Color, Rgba8::new(40, 80, 160, 128))],
        None,
    )
    .unwrap();
    let inner = doc.add_layer("Inner").unwrap();
    doc.set_pixel(inner, 3, 4, Rgba8::new(255, 0, 0, 255))
        .unwrap();
    doc.group_layers(&[inner], "Group").unwrap();
    doc.clear_history().unwrap();
    doc
}

/// 画面なしのホストを相手にする `Backend`（命令は裏のスレッドで実行する）。
pub struct FileBackend {
    pub host: Arc<Mutex<FileHost>>,
    pub ran: Arc<AtomicU64>,
}

impl Backend for FileBackend {
    fn run(&self, command: Command) -> BackendFuture {
        let host = self.host.clone();
        self.ran.fetch_add(1, Ordering::Relaxed);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || execute(&mut *host.lock().unwrap(), &command))
                .await
                .expect("命令のスレッドが終わる")
        })
    }
}

/// 受け口の様子を数える。
#[derive(Default)]
pub struct Seen {
    pub open: AtomicUsize,
    pub refused: Mutex<Vec<Refusal>>,
}

impl Observer for Seen {
    fn connections(&self, open: usize) {
        self.open.store(open, Ordering::Relaxed);
    }
    fn refused(&self, refusal: Refusal) {
        self.refused.lock().unwrap().push(refusal);
    }
}

/// 本物の受け口（127.0.0.1 の空いた番号）を、裏のスレッドの tokio で動かす（`http::Running`）。落とすと止める。
pub struct Served {
    pub port: u16,
    pub host: Arc<Mutex<FileHost>>,
    pub ran: Arc<AtomicU64>,
    pub seen: Arc<Seen>,
    running: http::Running,
}

impl Served {
    pub fn start(host: FileHost) -> Served {
        Served::start_with(host, Limits::default())
    }

    pub fn start_with(host: FileHost, limits: Limits) -> Served {
        let host = Arc::new(Mutex::new(host));
        let ran = Arc::new(AtomicU64::new(0));
        let backend = Arc::new(FileBackend {
            host: host.clone(),
            ran: ran.clone(),
        });
        let listener = http::bind(0).expect("待てる");
        let seen = Arc::new(Seen::default());
        let running =
            http::Running::start(listener, YoluMcp::new(backend), limits, seen.clone()).unwrap();
        Served {
            port: running.port(),
            host,
            ran,
            seen,
            running,
        }
    }

    pub fn ran(&self) -> u64 {
        self.ran.load(Ordering::Relaxed)
    }

    /// 止めて、口を閉じるまで待つ。
    pub fn stop(&mut self) {
        self.running.stop();
    }
}

/// 試験の客の実行基盤。
pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// 1 つの JSON-RPC のメッセージを送り、HTTP の返事を受ける。
pub fn post(port: u16, message: &Value, negotiated: Option<&str>) -> Exchange {
    let headers = client::mcp_headers(message, negotiated);
    runtime()
        .block_on(async {
            tokio::time::timeout(
                Duration::from_secs(60),
                client::post(port, &headers, serde_json::to_vec(message).unwrap()),
            )
            .await
        })
        .expect("60 秒のうちに返事")
        .unwrap_or_else(|e| panic!("{e}"))
}

/// MCP の客（Streamable HTTP）。2025-11-25 の流れは `initialize` で版を決めて頭に付け、2026-07-28 の流れは要求ごとの `_meta`。
pub struct Mcp {
    pub port: u16,
    pub version: Option<String>,
    next: u64,
    /// 2026-07-28 の流れ（要求ごとの `_meta`）か。
    pub stateless: bool,
}

impl Mcp {
    pub fn new(port: u16) -> Mcp {
        Mcp {
            port,
            version: None,
            next: 0,
            stateless: false,
        }
    }

    /// 2025-11-25 の `initialize` で始める。
    pub fn legacy(port: u16) -> (Mcp, Value) {
        let mut mcp = Mcp::new(port);
        let init = mcp.result(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test-client", "version": "1"}}),
        );
        mcp.version = Some(init["protocolVersion"].as_str().unwrap().to_owned());
        mcp.notify("notifications/initialized", json!({}));
        (mcp, init)
    }

    pub fn stateless(port: u16) -> Mcp {
        let mut mcp = Mcp::new(port);
        mcp.stateless = true;
        mcp
    }

    fn meta() -> Value {
        json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/clientInfo": {"name": "test-client", "version": "1"},
        })
    }

    pub fn notify(&mut self, method: &str, params: Value) {
        let message = json!({"jsonrpc": "2.0", "method": method, "params": params});
        let exchange = post(self.port, &message, self.version.as_deref());
        assert_eq!(exchange.status, 202, "{method}: {:?}", exchange.body);
    }

    /// 要求を送り、返事（result か error を含む JSON-RPC の応答）を返す。HTTP の誤りは、`error` に状態の番号と本文を入れて返す。
    pub fn request(&mut self, method: &str, mut params: Value) -> Value {
        if self.stateless {
            params["_meta"] = Mcp::meta();
        }
        self.next += 1;
        let id = self.next;
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let exchange = post(self.port, &message, self.version.as_deref());
        let messages = client::messages(&exchange).unwrap_or_default();
        match messages.into_iter().find(|m| m["id"] == json!(id)) {
            Some(m) => m,
            None => {
                json!({"error": {"http": exchange.status, "body": String::from_utf8_lossy(&exchange.body)}})
            }
        }
    }

    pub fn result(&mut self, method: &str, params: Value) -> Value {
        let response = self.request(method, params);
        assert!(response.get("error").is_none(), "{method}: {response}");
        response["result"].clone()
    }

    pub fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.result("tools/call", json!({"name": tool, "arguments": arguments}))
    }
}

pub fn text_json(result: &Value) -> Value {
    let text = result["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{result}"));
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

/// 生の HTTP の要求（頭を自由に書く。Host・Origin の試験）を送り、状態の番号を返す。
pub fn raw_status(port: u16, request: &str) -> u16 {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buffer.extend_from_slice(&chunk[..n]);
                if buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(e) => panic!("返事を読めない: {e}"),
        }
    }
    let text = String::from_utf8_lossy(&buffer);
    text.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("状態の行が無い: {text}"))
}

/// 要求の生の形（`initialize` の JSON を本文に）。
pub fn raw_request(port: u16, host: &str, origin: Option<&str>) -> String {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "raw", "version": "1"}}}).to_string();
    let origin = origin
        .map(|o| format!("Origin: {o}\r\n"))
        .unwrap_or_default();
    let _ = port;
    format!(
        "POST /mcp HTTP/1.1\r\nHost: {host}\r\n{origin}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
