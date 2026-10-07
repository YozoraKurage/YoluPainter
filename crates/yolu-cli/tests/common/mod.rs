//! 試験の台: 小さな .ylp を作る・命令を CLI の引数で当てる・起動中のアプリの受け口の代わり（本物の MCP の受け口で、本物の文書を操作する）を立てる。
#![allow(dead_code)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use yolu_cli::cli::{run, Env, Outcome};
use yolu_core::{Channel, Document, Rgba8};
use yolu_io::{composite_pngs, DocumentSource, MaterialRef, Project, SaveTarget, SetSpec};
use yolu_mcp::http::{self, Limits, Observer, Refusal, Running};
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
            "yolu-cli-{}-{}-{name}",
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
    pub fn env(&self) -> Env {
        Env {
            cwd: self.dir.clone(),
            lang: None,
        }
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
    /// CLI を引数で走らせる（標準入力は空）。
    pub fn cli(&self, args: &[&str]) -> Outcome {
        self.cli_in(args, "")
    }
    pub fn cli_in(&self, args: &[&str], stdin: &str) -> Outcome {
        let tokens: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        let mut input = stdin.as_bytes();
        run(&tokens, &self.env(), &mut input)
    }
    /// 成功を期待して、標準出力の JSON を返す。
    pub fn ok(&self, args: &[&str]) -> Value {
        let out = self.cli(args);
        assert_eq!(out.code, 0, "{args:?}: {}{}", out.stdout, out.stderr);
        assert!(out.stderr.is_empty(), "{args:?}: {}", out.stderr);
        serde_json::from_str(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?}: {e}: {}", out.stdout))
    }
    /// 失敗を期待して、(終了コード, 標準出力の誤りの JSON) を返す。
    pub fn fails(&self, args: &[&str]) -> (i32, Value) {
        let out = self.cli(args);
        assert_ne!(out.code, 0, "{args:?} が通った: {}", out.stdout);
        let v: Value = serde_json::from_str(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?}: {e}: {}", out.stdout));
        assert!(v["error"]["code"].is_string(), "{args:?}: {v}");
        assert!(v["error"]["message"]["ja"].is_string() && v["error"]["message"]["en"].is_string());
        assert!(
            out.stderr.starts_with("yolupainter-cli: "),
            "{}",
            out.stderr
        );
        (out.code, v["error"].clone())
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

/// 起動中のアプリの代わり。本物の MCP の受け口（127.0.0.1 の空いた番号）で、受けた命令を本物の文書（`FileHost`）に当てて返す。
/// 振る舞いを変えると、HTTP の段で壊れた相手にもなる。
pub struct FakeApp {
    pub port: u16,
    pub host: Arc<Mutex<FileHost>>,
    /// 受けた命令の数。
    pub served: Arc<AtomicU64>,
    /// 開いているつながりの数（受け口が数えた物）。
    pub open: Arc<AtomicUsize>,
    _running: Option<Running>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// 待ち受けの振る舞い。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Behavior {
    /// 命令を実行して返す。
    Serve,
    /// 命令を受けたまま返事をしない（時間切れの試験）。
    Silent,
    /// 要求を読んで、返事を書かずにつながりを閉じる。
    CloseWithoutReply,
    /// MCP でない HTTP の返事をする（別のプログラムが同じ番号で待っている）。
    NotMcp,
    /// つながりの数が上限（503）。
    Busy,
}

struct Run {
    host: Arc<Mutex<FileHost>>,
    served: Arc<AtomicU64>,
    silent: bool,
}

impl Backend for Run {
    fn run(&self, command: Command) -> BackendFuture {
        self.served.fetch_add(1, Ordering::Relaxed);
        let host = self.host.clone();
        let silent = self.silent;
        Box::pin(async move {
            if silent {
                std::future::pending::<()>().await;
            }
            tokio::task::spawn_blocking(move || execute(&mut *host.lock().unwrap(), &command))
                .await
                .expect("命令のスレッドが終わる")
        })
    }
}

impl FakeApp {
    pub fn start(fx: &Fixture, file: &str, behavior: Behavior) -> FakeApp {
        let mut host = FileHost::new(PathPolicy::new(&fx.dir).unwrap());
        host.open(&fx.path(file), true).unwrap();
        FakeApp::start_with(host, behavior)
    }

    pub fn start_with(host: FileHost, behavior: Behavior) -> FakeApp {
        FakeApp::start_on(0, host, behavior)
    }

    /// つながりの数の上限を変えて、命令を実行して返す（上限を小さくして埋める試験）。
    pub fn start_limited(fx: &Fixture, file: &str, limits: Limits) -> FakeApp {
        let mut host = FileHost::new(PathPolicy::new(&fx.dir).unwrap());
        host.open(&fx.path(file), true).unwrap();
        FakeApp::start_with_limits(0, host, Behavior::Serve, limits)
    }

    /// 決めた番号で待つ（0 なら空いた番号）。
    pub fn start_on(port: u16, host: FileHost, behavior: Behavior) -> FakeApp {
        FakeApp::start_with_limits(port, host, behavior, Limits::default())
    }

    fn start_with_limits(port: u16, host: FileHost, behavior: Behavior, limits: Limits) -> FakeApp {
        let host = Arc::new(Mutex::new(host));
        let open = Arc::new(AtomicUsize::new(0));
        let served = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let listener = http::bind(port).expect("待てる");
        let port = listener.local_addr().unwrap().port();
        let (running, thread) = match behavior {
            Behavior::Serve | Behavior::Silent => {
                let backend = Arc::new(Run {
                    host: host.clone(),
                    served: served.clone(),
                    silent: behavior == Behavior::Silent,
                });
                let running = Running::start(
                    listener,
                    YoluMcp::new(backend),
                    limits,
                    Arc::new(Counting { open: open.clone() }),
                )
                .unwrap();
                (Some(running), None)
            }
            _ => {
                let (stop, served) = (stop.clone(), served.clone());
                let thread =
                    std::thread::spawn(move || raw_server(listener, behavior, stop, served));
                (None, Some(thread))
            }
        };
        FakeApp {
            port,
            host,
            served,
            open,
            _running: running,
            stop,
            thread,
        }
    }

    pub fn served(&self) -> u64 {
        self.served.load(Ordering::Relaxed)
    }

    /// `--port` の引数。
    pub fn port_arg(&self) -> String {
        self.port.to_string()
    }
}

/// 開いているつながりの数を覚える `Observer`。
struct Counting {
    open: Arc<AtomicUsize>,
}

impl Observer for Counting {
    fn connections(&self, open: usize) {
        self.open.store(open, Ordering::Relaxed);
    }
    fn refused(&self, _refusal: Refusal) {}
}

/// HTTP の段で壊れた相手（要求を読んで、決めた振る舞いをする）。
fn raw_server(
    listener: std::net::TcpListener,
    behavior: Behavior,
    stop: Arc<std::sync::atomic::AtomicBool>,
    served: Arc<AtomicU64>,
) {
    use std::io::Write;
    while !stop.load(Ordering::Relaxed) {
        let mut stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(_) => break,
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        // 頭と本文を読む（頭の終わりまで読み、Content-Length の分を足す）
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    buffer.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buffer).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let length = text[..end]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if buffer.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
            }
        }
        served.fetch_add(1, Ordering::Relaxed);
        let reply: &[u8] = match behavior {
            Behavior::CloseWithoutReply => b"",
            Behavior::NotMcp => b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
            _ => b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbusy",
        };
        let _ = stream.write_all(reply);
        let _ = stream.flush();
    }
}

impl Drop for FakeApp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// どこも待っていない番号（開いてすぐ閉じた口の番号）。
pub fn unused_port() -> u16 {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.local_addr().unwrap().port()
}

pub fn read_to_end(mut r: impl Read) -> Vec<u8> {
    let mut v = Vec::new();
    r.read_to_end(&mut v).unwrap();
    v
}

pub fn file_bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}
