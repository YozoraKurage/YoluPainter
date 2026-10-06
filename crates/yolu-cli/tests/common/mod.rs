//! 試験の台: 小さな .ylp を作る・命令を CLI の引数で当てる・起動中のアプリの待ち受けの代わり（本物の文書を操作する）を立てる。
#![allow(dead_code)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use yolu_cli::cli::{run, Env, Outcome};
use yolu_core::{Channel, Document, Rgba8};
use yolu_io::{composite_pngs, DocumentSource, MaterialRef, Project, SaveTarget, SetSpec};
use yolu_ops::link::{
    decode_request, encode_response, read_frame, Received, Response, KIND_REQUEST,
};
use yolu_ops::{execute, FileHost, OpHost, PathPolicy};
use yolu_protocol::link::{accept_as, Server, HANDSHAKE_TIMEOUT};
use yolu_protocol::{Identity, Reject};

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

/// 経路の名前（試験ごとに別。実際のアプリの `yolupainter-ops` と重ならない）。
pub fn unique_link_name(tag: &str) -> String {
    format!(
        "ylp-cli-test-{}-{}-{tag}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// 起動中のアプリの代わり。本物の挨拶（鍵の確かめ合い）で待ち受け、受けた要求を本物の文書（`FileHost`）に当てて返す。
pub struct FakeApp {
    pub name: String,
    pub host: Arc<Mutex<FileHost>>,
    /// 受けた要求の数。
    pub served: Arc<AtomicU64>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// 待ち受けの振る舞い。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Behavior {
    /// 要求を実行して返す。
    Serve,
    /// 挨拶のあと、返事を返さずにつながりを閉じる。
    CloseWithoutReply,
    /// 返事を返さずに待つ（時間切れの試験）。
    Silent,
    /// 要求に、操作の返事ではない枠を返す。
    WrongFrame,
    /// 挨拶で断る（鍵と版は合っていて、受けてよいかの確かめで断る）。
    Reject,
}

impl FakeApp {
    pub fn start(fx: &Fixture, file: &str, tag: &str, behavior: Behavior) -> FakeApp {
        let name = unique_link_name(tag);
        let mut host = FileHost::new(PathPolicy::new(&fx.dir).unwrap());
        host.open(&fx.path(file), true).unwrap();
        FakeApp::start_with(name, host, behavior)
    }

    pub fn start_with(name: String, host: FileHost, behavior: Behavior) -> FakeApp {
        let server = Server::bind(&name, true).expect("待ち受けを始められる");
        let host = Arc::new(Mutex::new(host));
        let served = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread = {
            let (host, served, stop) = (host.clone(), served.clone(), stop.clone());
            std::thread::spawn(move || {
                let key = server.key();
                while !stop.load(Ordering::Relaxed) {
                    let stream = match server.accept() {
                        Ok(s) => s,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        Err(_) => break,
                    };
                    let (host, served, stop) = (host.clone(), served.clone(), stop.clone());
                    let key = key.clone();
                    std::thread::spawn(move || {
                        let claim = |_: &yolu_protocol::Hello| -> Result<(), Reject> {
                            if behavior == Behavior::Reject {
                                Err(Reject::plain(yolu_protocol::RejectCode::Busy, "試験の断り"))
                            } else {
                                Ok(())
                            }
                        };
                        let Ok((connection, mut reader, _hello)) = accept_as(
                            stream,
                            &Identity::standalone("fake-app"),
                            1,
                            &key,
                            HANDSHAKE_TIMEOUT,
                            &claim,
                        ) else {
                            return;
                        };
                        loop {
                            if stop.load(Ordering::Relaxed) {
                                return;
                            }
                            reader.set_timeout(Some(Duration::from_millis(100)));
                            let (frames, stream) = reader.raw();
                            let mut stream = stream;
                            match read_frame(frames, &mut stream) {
                                Ok(Received::Frame(frame)) => {
                                    assert_eq!(frame.kind, KIND_REQUEST);
                                    served.fetch_add(1, Ordering::Relaxed);
                                    let request = decode_request(&frame).expect("要求は読める");
                                    match behavior {
                                        Behavior::CloseWithoutReply => return,
                                        Behavior::Silent => {
                                            while !stop.load(Ordering::Relaxed) {
                                                std::thread::sleep(Duration::from_millis(20));
                                            }
                                            return;
                                        }
                                        Behavior::WrongFrame => {
                                            let _ = connection.send_raw(0x0007, b"x");
                                            return;
                                        }
                                        _ => {}
                                    }
                                    let outcome = {
                                        let mut host = host.lock().unwrap();
                                        execute(&mut *host, &request.command)
                                    };
                                    let bytes = encode_response(&Response {
                                        id: request.id,
                                        outcome,
                                    });
                                    if connection.send_frame(&bytes).is_err() {
                                        return;
                                    }
                                }
                                Ok(Received::Idle) => {}
                                Ok(Received::Closed) | Err(_) => return,
                            }
                        }
                    });
                }
            })
        };
        FakeApp {
            name,
            host,
            served,
            stop,
            thread: Some(thread),
        }
    }

    pub fn served(&self) -> u64 {
        self.served.load(Ordering::Relaxed)
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

pub fn read_to_end(mut r: impl Read) -> Vec<u8> {
    let mut v = Vec::new();
    r.read_to_end(&mut v).unwrap();
    v
}

pub fn file_bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}
