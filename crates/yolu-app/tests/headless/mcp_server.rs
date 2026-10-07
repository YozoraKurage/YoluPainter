//! 外からの操作（MCP のクライアント・コマンドライン）を受ける、起動中のアプリの側（`mcp_server`・`ops_host`）。画面は描かず、アプリを
//! 窓の無いフレーム（`tick_hidden`）で回し、本物の HTTP の受け口（127.0.0.1。番号は 0 にして OS に空いた番号を選ばせる）へ、
//! 本物の HTTP の客（`yolu_mcp::client`・コマンドラインの `yolu_cli::live`）でつなぐ。
//!
//! - 設定の入切で待ち受ける・やめる。番号が使われていれば理由を出し、毎フレームは試さない。Host・Origin が違えば断る。
//! - 命令は画面のスレッドで当たり、1 命令 = 画面の取り消しの 1 段（画面の取り消し・やり直しでそのまま戻る）。1 フレームの数に上限。
//! - 描いている最中・保存の途中・読むだけのセットは理由つきで断る。`doc.open` は開き替えない。壊す操作は確認が要り、済んだら短く知らせる。
//! - 保存は裏のスレッドで動かし、返事は保存が終わってから。切ると、待っている要求と返事待ちの保存に「受け付けをやめた」を返す。
use crate::common::{tmp, wait::WATCHDOG};

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use yolu_app::engine::DVec2;
use yolu_app::lang::Lang;
use yolu_app::mcp_server::OpsStatus;
use yolu_app::newproject::NpAction;
use yolu_app::pen::PenInput;
use yolu_app::state::{Action, AppState};
use yolu_app::YoluApp;
use yolu_cli::live::LiveConfig;
use yolu_ops::reply::{Reply, SetState};
use yolu_ops::{parse_command, ErrorCode, FileHost, OpError, OpHost, PathPolicy};

/// 窓の無いアプリ（フレームは `tick_hidden`）。
struct Live {
    ctx: egui::Context,
    app: YoluApp,
}

impl Live {
    /// 外からの操作を受ける設定のアプリ（番号は 0: OS が空いた番号を選ぶ。まだ待ち受けない。最初のフレームで始まる）。
    fn on() -> Live {
        Live::on_with(AppState::new(64, 64))
    }

    /// `on` の、最初の状態を渡せる形。
    fn on_with(state: AppState) -> Live {
        let mut live = Live::off_with(state);
        live.app.state.prefs.settings.external_ops = true;
        live
    }

    fn off() -> Live {
        Live::off_with(AppState::new(64, 64))
    }

    fn off_with(state: AppState) -> Live {
        let ctx = egui::Context::default();
        let mut app = YoluApp::for_context(&ctx, state, PenInput::detached());
        app.state.prefs.settings.external_ops_port = 0;
        Live { ctx, app }
    }

    fn frame(&mut self) {
        let ctx = self.ctx.clone();
        self.app.tick_hidden(&ctx);
    }

    fn until<T>(&mut self, what: &str, mut found: impl FnMut(&mut Live) -> Option<T>) -> T {
        let deadline = Instant::now() + WATCHDOG;
        loop {
            self.frame();
            if let Some(value) = found(self) {
                return value;
            }
            assert!(
                Instant::now() < deadline,
                "{what} を待ったが来ない: 状態={:?}, 知らせ={}",
                self.app.state.ops.status,
                self.app.state.message
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// 何も起きないことを見るために、フレームを少し進める。
    fn settle(&mut self) {
        for _ in 0..30 {
            self.frame();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// 待っている番号（待ち受けていなければ落ちる）。
    fn port(&self) -> u16 {
        self.app.ops().port().expect("待ち受けている")
    }

    /// 命令を裏のスレッドから送る（返事は、アプリのフレームを回しながら受ける）。
    fn send(&self, command: Value) -> Receiver<Result<Reply, OpError>> {
        send_to(self.port(), command)
    }

    /// 命令を送り、返事を待つ（アプリのフレームを回しながら）。
    fn ask(&mut self, command: Value) -> Result<Reply, OpError> {
        let rx = self.send(command);
        self.until("返事", |_| rx.try_recv().ok())
    }

    fn ok(&mut self, command: Value) -> Reply {
        let name = command["command"].clone();
        self.ask(command)
            .unwrap_or_else(|e| panic!("{name} が断られた: {e} {:?}", e.data))
    }

    fn err(&mut self, command: Value) -> OpError {
        let name = command["command"].clone();
        self.ask(command)
            .expect_err(&format!("{name} は断られるはず"))
    }
}

/// コマンドラインの客（`yolu_cli::live`: MCP の `tools/call` を 1 回）で、裏のスレッドから送る。
fn send_to(port: u16, command: Value) -> Receiver<Result<Reply, OpError>> {
    let command = parse_command(&command).expect("命令の形");
    let config = LiveConfig {
        port,
        timeout: Duration::from_secs(60),
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(yolu_cli::live::call(&config, &command));
    });
    rx
}

/// 画面のスレッドを通らない MCP の要求（`initialize`・`tools/list`）を、本物の HTTP の客で送る。
fn mcp(port: u16, message: Value, version: Option<&str>) -> (u16, Vec<Value>) {
    let headers = yolu_mcp::client::mcp_headers(&message, version);
    let exchange = yolu_mcp::client::post_blocking(
        port,
        &headers,
        serde_json::to_vec(&message).unwrap(),
        Duration::from_secs(60),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let messages = yolu_mcp::client::messages(&exchange).unwrap_or_default();
    (exchange.status, messages)
}

/// 生の HTTP の要求を送り、状態の番号を返す。
fn raw_status(port: u16, host: &str, origin: Option<&str>) -> u16 {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "raw", "version": "1"}}}).to_string();
    let origin = origin
        .map(|o| format!("Origin: {o}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {host}\r\n{origin}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut text = String::new();
    let _ = stream.read_to_string(&mut text);
    text.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("状態の行が無い: {text}"))
}

fn layer_count(live: &Live) -> usize {
    live.app.state.doc.layers().len()
}

fn first_set(reply: Reply) -> yolu_ops::reply::SetInfo {
    match reply {
        Reply::Set(s) => s,
        other => panic!("set.info の返事ではない: {other:?}"),
    }
}

fn edited(reply: Reply) -> yolu_ops::reply::Edited {
    match reply {
        Reply::Edited(e) => e,
        other => panic!("edited の返事ではない: {other:?}"),
    }
}

// ───────── 設定の入切と番号 ─────────

#[test]
fn the_setting_starts_and_stops_listening_on_the_loopback() {
    let mut live = Live::off();
    // 既定は切: 待ち受けない
    assert!(!live.app.state.prefs.settings.external_ops);
    live.settle();
    assert_eq!(live.app.state.ops.status, OpsStatus::Off);
    assert!(live.app.ops().port().is_none());
    // 入れると待ち受ける（127.0.0.1 だけ）
    live.app.state.prefs.settings.external_ops = true;
    live.frame();
    assert_eq!(live.app.state.ops.status, OpsStatus::Listening);
    assert!(live.app.state.ops.is_on());
    let port = live.port();
    assert_eq!(live.app.state.ops.port, port);
    for lang in [Lang::Ja, Lang::En] {
        let tip = live.app.state.ops.tooltip(lang);
        assert!(
            tip.contains(&format!("http://127.0.0.1:{port}/mcp")),
            "つなぐ側に渡す URL が印のツールチップにある: {tip}"
        );
    }
    // MCP の initialize と一覧は、画面のスレッドを通らずに答える
    let (status, messages) = mcp(
        port,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}),
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(messages[0]["result"]["serverInfo"]["name"], "yolupainter");
    let (_, messages) = mcp(
        port,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
        Some("2025-11-25"),
    );
    assert_eq!(
        messages[0]["result"]["tools"].as_array().unwrap().len(),
        yolu_ops::commands().len()
    );
    // 命令が通る
    let Reply::Doc(doc) = live.ok(json!({"command": "doc.info"})) else {
        panic!("doc.info の返事ではない")
    };
    assert_eq!(doc.sets.len(), 1);
    assert_eq!(doc.path, "", "まだファイルが無い文書");
    // 切ると、待ち受けをやめる（つなげない。同じ番号はすぐに空く）
    live.app.state.prefs.settings.external_ops = false;
    live.frame();
    assert_eq!(live.app.state.ops.status, OpsStatus::Off);
    assert!(live.app.state.ops.tooltip(Lang::Ja).is_empty());
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    // もう一度入れると、また受ける
    live.app.state.prefs.settings.external_ops = true;
    live.frame();
    assert_eq!(live.app.state.ops.status, OpsStatus::Listening);
    assert!(matches!(
        live.ok(json!({"command": "doc.info"})),
        Reply::Doc(_)
    ));
}

#[test]
fn a_port_in_use_is_reported_once_and_not_retried_every_frame() {
    // ほかのプログラム（ここでは試験の口）が番号を使っている
    let taken = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = taken.local_addr().unwrap().port();
    let mut live = Live::on();
    live.app.state.prefs.settings.external_ops_port = port;
    live.frame();
    let OpsStatus::Failed(reason) = live.app.state.ops.status.clone() else {
        panic!("待ち受けない: {:?}", live.app.state.ops.status)
    };
    assert!(reason.contains(&port.to_string()), "{reason}");
    assert!(
        reason.contains("ほかのプログラム"),
        "使われている理由: {reason}"
    );
    assert!(
        live.app.state.ops.tooltip(Lang::Ja).contains(&reason),
        "理由が印のツールチップに出る"
    );
    assert!(!live.app.state.message.is_empty(), "理由を知らせる");
    // 毎フレームは試さない（口が空いても、設定を変えるまで失敗のまま）
    drop(taken);
    live.app.state.message.clear();
    live.settle();
    assert!(matches!(live.app.state.ops.status, OpsStatus::Failed(_)));
    assert!(live.app.state.message.is_empty(), "続けて知らせない");
    // 番号を変えると試し直す
    live.app.state.prefs.settings.external_ops_port = 0;
    live.frame();
    assert_eq!(live.app.state.ops.status, OpsStatus::Listening);
    // 切って入れ直しても試し直す
    live.app.state.prefs.settings.external_ops = false;
    live.frame();
    assert_eq!(live.app.state.ops.status, OpsStatus::Off);
}

#[test]
fn changing_the_port_moves_the_listener() {
    let mut live = Live::on();
    live.frame();
    let first = live.port();
    // 空いた番号を 1 つ探して、そこへ移す
    let free = {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        l.local_addr().unwrap().port()
    };
    live.app.state.prefs.settings.external_ops_port = free;
    live.frame();
    assert_eq!(live.app.state.ops.status, OpsStatus::Listening);
    assert_eq!(live.port(), free);
    assert!(
        TcpStream::connect(("127.0.0.1", first)).is_err(),
        "前の番号では待たない"
    );
    assert!(matches!(
        live.ok(json!({"command": "doc.info"})),
        Reply::Doc(_)
    ));
}

#[test]
fn a_foreign_host_or_origin_is_refused_and_said_once() {
    let mut live = Live::on();
    live.frame();
    let port = live.port();
    let own = format!("127.0.0.1:{port}");
    assert_eq!(raw_status(port, &own, None), 200);
    live.app.state.message.clear();
    // DNS rebinding（ほかの名前が 127.0.0.1 を指す）
    assert_eq!(raw_status(port, &format!("evil.example:{port}"), None), 403);
    live.until("断った知らせ", |l| {
        (!l.app.state.message.is_empty()).then_some(())
    });
    assert_eq!(
        live.app.state.message,
        "外からの操作: 宛先がこの PC でない要求を断りました。"
    );
    // 同じ断りが続いても、知らせは 1 度
    live.app.state.message.clear();
    assert_eq!(raw_status(port, &format!("evil.example:{port}"), None), 403);
    live.settle();
    assert!(
        live.app.state.message.is_empty(),
        "{}",
        live.app.state.message
    );
    // ウェブページから（Host は自分でも、Origin が違う）
    assert_eq!(raw_status(port, &own, Some("http://evil.example")), 403);
    live.until("断った知らせ", |l| {
        (!l.app.state.message.is_empty()).then_some(())
    });
    assert_eq!(
        live.app.state.message,
        "外からの操作: ウェブページからの要求を断りました。"
    );
    assert_eq!(
        live.app.ops().handled(),
        0,
        "断った要求は画面のスレッドへ来ない"
    );
}

// ───────── 命令は画面の取り消しの 1 段 ─────────

#[test]
fn one_command_is_one_step_of_the_screens_undo_and_redo() {
    let mut live = Live::on();
    live.frame();
    let before = layer_count(&live);
    let undo_before = live.app.state.doc.undo_count();
    let reply = edited(
        live.ok(json!({"command": "layer.add", "args": {"kind": "paint", "name": "外から"}})),
    );
    assert_eq!(layer_count(&live), before + 1);
    assert_eq!(live.app.state.doc.undo_count(), undo_before + 1);
    assert!(reply.can_undo);
    assert!(live.app.state.modified, "変更ありの印が付く");
    let undo_mid = live.app.state.doc.undo_count();
    live.ok(
        json!({"command": "layer.set", "args": {"layer": "外から", "opacity": 0.5, "visible": false, "name": "外から 2"}}),
    );
    assert_eq!(live.app.state.doc.undo_count(), undo_mid + 1);
    let layer = live
        .app
        .state
        .doc
        .layers()
        .iter()
        .find(|l| l.name() == "外から 2")
        .expect("名前が変わった");
    assert!((layer.opacity() - 0.5).abs() < 1e-9 && !layer.visible());
    live.app.state.apply(Action::Undo);
    let layer = live
        .app
        .state
        .doc
        .layers()
        .iter()
        .find(|l| l.name() == "外から")
        .expect("名前が戻った");
    assert!((layer.opacity() - 1.0).abs() < 1e-9 && layer.visible());
    live.app.state.apply(Action::Undo);
    assert_eq!(layer_count(&live), before);
    live.app.state.apply(Action::Redo);
    assert_eq!(layer_count(&live), before + 1);
    let undone = live.ok(json!({"command": "undo"}));
    assert!(matches!(undone, Reply::Undone(_)));
    assert_eq!(layer_count(&live), before);
    live.ok(json!({"command": "redo"}));
    assert_eq!(layer_count(&live), before + 1);
}

#[test]
fn deleting_a_selected_layer_keeps_the_screens_selection_valid() {
    let mut live = Live::on();
    live.frame();
    live.ok(json!({"command": "layer.add", "args": {"kind": "paint", "name": "選ぶ"}}));
    let id = live
        .app
        .state
        .doc
        .layers()
        .iter()
        .find(|l| l.name() == "選ぶ")
        .unwrap()
        .id();
    live.app.state.selected_layer = Some(id);
    live.ok(json!({"command": "layer.delete", "args": {"layer": "選ぶ", "confirm": true}}));
    let selected = live.app.state.selected_layer.expect("選び直される");
    assert!(
        live.app.state.doc.layer(selected).is_some(),
        "消した層を選んだままにしない"
    );
}

#[test]
fn reads_describe_the_open_project_and_the_current_set() {
    let mut live = Live::on();
    live.frame();
    let info = first_set(live.ok(json!({"command": "set.info"})));
    assert_eq!((info.width, info.height), (64, 64));
    assert_eq!(info.state, SetState::Editable);
    assert_eq!(info.layers.len(), layer_count(&live));
    let by_name = first_set(live.ok(json!({"command": "set.info", "args": {"set": info.name}})));
    assert_eq!(by_name.id, info.id);
    let e = live.err(json!({"command": "set.info", "args": {"set": "無いセット"}}));
    assert_eq!(e.code, ErrorCode::NotFound);
    assert!(e.data.unwrap()["sets"].is_array());
    // 見本は今の合成から（MCP の画像の content で運び、客で PNG に戻す）
    let Reply::Preview(preview) = live.ok(json!({"command": "preview", "args": {"max_edge": 16}}))
    else {
        panic!("preview の返事ではない")
    };
    assert_eq!(
        (preview.width, preview.height, preview.source_width),
        (16, 16, 64)
    );
    assert_eq!(&preview.png.0[1..4], b"PNG");
    assert!(!live.app.state.modified);
}

// ───────── 断る ─────────

#[test]
fn edits_are_refused_while_drawing_and_reads_still_answer() {
    let mut live = Live::on();
    live.frame();
    let layer = live.app.state.doc.layers().last().unwrap().id();
    let brush = live.app.state.stroke_settings(false);
    let mut stroke = live.app.state.doc.begin_stroke(layer, &brush).unwrap();
    stroke
        .add_point(&mut live.app.state.doc, 10.0, 10.0, 1.0, DVec2::ZERO)
        .unwrap();
    assert!(live.app.state.is_stroking());
    let before = layer_count(&live);
    for command in [
        json!({"command": "layer.add", "args": {"kind": "paint"}}),
        json!({"command": "undo"}),
        json!({"command": "preview"}),
    ] {
        let e = live.err(command.clone());
        assert_eq!(e.code, ErrorCode::Busy, "{command}");
        assert!(!e.message.ja.is_empty() && !e.message.en.is_empty());
    }
    assert_eq!(layer_count(&live), before, "断った命令は何も変えない");
    assert!(matches!(
        live.ok(json!({"command": "set.info"})),
        Reply::Set(_)
    ));
    live.app.state.doc.end_stroke(stroke).unwrap();
    live.ok(json!({"command": "layer.add", "args": {"kind": "paint"}}));
    assert_eq!(layer_count(&live), before + 1);
}

#[test]
fn a_read_only_set_refuses_edits_with_the_screens_reason() {
    let mut live = Live::on();
    live.app.state.sets.get_mut(0).unwrap().read_only = Some("編集できない中身があります".into());
    live.frame();
    let e = live.err(json!({"command": "layer.add", "args": {"kind": "paint"}}));
    assert_eq!(e.code, ErrorCode::ReadOnly);
    assert!(e.message.ja.contains("編集できない中身があります"));
    let info = first_set(live.ok(json!({"command": "set.info"})));
    assert_eq!(info.state, SetState::ReadOnly);
    let Reply::Doc(doc) = live.ok(json!({"command": "doc.info"})) else {
        panic!()
    };
    assert_eq!(doc.sets[0].state, SetState::ReadOnly);
    assert!(doc.sets[0].reason.is_some());
}

#[test]
fn destructive_commands_need_confirmation_and_say_so_briefly_when_done() {
    let mut live = Live::on();
    live.frame();
    live.ok(json!({"command": "layer.add", "args": {"kind": "paint", "name": "消す"}}));
    let before = layer_count(&live);
    live.app.state.message.clear();
    let e = live.err(json!({"command": "layer.delete", "args": {"layer": "消す"}}));
    assert_eq!(e.code, ErrorCode::ConfirmRequired);
    assert_eq!(layer_count(&live), before);
    assert!(live.app.state.message.is_empty(), "断ったときは知らせない");
    live.ok(json!({"command": "layer.delete", "args": {"layer": "消す", "confirm": true}}));
    assert_eq!(layer_count(&live), before - 1);
    assert_eq!(live.app.state.message, "外からの操作: レイヤーを消す");
    live.app.state.message.clear();
    live.ok(json!({"command": "layer.add", "args": {"kind": "paint"}}));
    live.ok(json!({"command": "set.info"}));
    assert!(
        live.app.state.message.is_empty(),
        "{}",
        live.app.state.message
    );
}

#[test]
fn opening_another_document_is_refused_and_the_same_file_returns_the_current_one() {
    let dir = tmp::test_dir("mcpserver-open");
    let file = dir.join("work.ylp");
    let mut live = Live::on();
    live.app.state.apply(Action::SaveProjectAs(file.clone()));
    live.app.state.wait_save();
    live.frame();
    let e = live.err(json!({"command": "doc.open", "args": {"path": dir.join("other.ylp").display().to_string()}}));
    assert_eq!(e.code, ErrorCode::Unsupported);
    let Reply::Doc(doc) =
        live.ok(json!({"command": "doc.open", "args": {"path": file.display().to_string()}}))
    else {
        panic!()
    };
    assert_eq!(PathBuf::from(&doc.path), file);
}

// ───────── 1 フレームの上限・同時の上限 ─────────

#[test]
fn a_frame_runs_at_most_its_share_of_waiting_requests() {
    let mut live = Live::on();
    live.app.ops_mut().set_requests_per_frame(2);
    live.frame();
    let before = layer_count(&live);
    let port = live.port();
    let replies: Vec<_> = (0..5)
        .map(|n| send_to(port, json!({"command": "layer.add", "args": {"kind": "paint", "name": format!("並び {n}")}})))
        .collect();
    // 5 つとも列に入るまで、フレームを回さずに待つ
    let deadline = Instant::now() + WATCHDOG;
    while live.app.ops().waiting() < 5 {
        assert!(
            Instant::now() < deadline,
            "列に入らない: {}",
            live.app.ops().waiting()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let handled = live.app.ops().handled();
    live.frame();
    assert_eq!(
        live.app.ops().handled(),
        handled + 2,
        "1 フレームに 2 つまで"
    );
    assert_eq!(layer_count(&live), before + 2);
    live.frame();
    assert_eq!(live.app.ops().handled(), handled + 4);
    live.frame();
    assert_eq!(live.app.ops().handled(), handled + 5, "残りは次のフレーム");
    for rx in replies {
        let reply = live.until("返事", |_| rx.try_recv().ok());
        assert!(matches!(reply, Ok(Reply::Edited(_))), "{reply:?}");
    }
}

#[test]
fn connections_beyond_eight_are_refused_and_a_closed_ones_place_is_given_back() {
    let mut live = Live::on();
    live.frame();
    let port = live.port();
    let held: Vec<TcpStream> = (0..8)
        .map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap())
        .collect();
    live.until("8 つのつながり", |l| {
        (l.app.state.ops.status == OpsStatus::Connected(8)).then_some(())
    });
    assert!(live.app.state.ops.tooltip(Lang::Ja).contains("つながり 8"));
    live.app.state.message.clear();
    // 9 つ目は 503（コマンドラインは Busy の誤り）
    let e = live.err(json!({"command": "doc.info"}));
    assert_eq!(e.code, ErrorCode::Busy, "{e:?}");
    live.until("上限の知らせ", |l| {
        (!l.app.state.message.is_empty()).then_some(())
    });
    assert_eq!(
        live.app.state.message,
        "外からの操作: つながりの数が上限のため断りました。"
    );
    // 1 つ閉じると、その分が戻る
    drop(held);
    live.until("閉じたつながりを忘れる", |l| {
        (l.app.state.ops.status == OpsStatus::Listening).then_some(())
    });
    assert!(matches!(
        live.ok(json!({"command": "doc.info"})),
        Reply::Doc(_)
    ));
}

// ───────── 保存 ─────────

#[test]
fn saving_runs_in_the_background_and_the_reply_comes_when_it_is_done() {
    let dir = tmp::test_dir("mcpserver-save");
    let file = dir.join("work.ylp");
    let mut live = Live::on();
    live.app.state.save.background = true;
    live.app.state.apply(Action::SaveProjectAs(file.clone()));
    live.app.state.wait_save();
    live.frame();
    live.ok(json!({"command": "layer.add", "args": {"kind": "paint", "name": "保存する層"}}));
    let e = live.err(json!({"command": "save"}));
    assert_eq!(e.code, ErrorCode::ConfirmRequired);
    // 保存の仕事を止めておき、返事が保存の終わりまで来ないこと・その間の編集と保存が断られることを見る
    let hold = live.app.state.save.hold_next();
    let rx = live.send(json!({"command": "save", "args": {"confirm": true}}));
    live.until("保存の開始", |l| l.app.state.is_saving().then_some(()));
    assert!(live.app.ops().save_pending());
    live.settle();
    assert!(rx.try_recv().is_err(), "保存が終わるまで返事は来ない");
    let e = live.err(json!({"command": "layer.add", "args": {"kind": "paint"}}));
    assert_eq!(e.code, ErrorCode::Busy, "保存の途中の編集は断る");
    let e = live.err(json!({"command": "save", "args": {"confirm": true}}));
    assert_eq!(e.code, ErrorCode::Busy, "保存の途中の 2 回目の保存は断る");
    assert!(
        matches!(live.ok(json!({"command": "set.info"})), Reply::Set(_)),
        "読む命令は保存の途中でも答える"
    );
    hold.release();
    let reply = live.until("保存の返事", |_| rx.try_recv().ok());
    let Reply::Saved(saved) = reply.expect("保存できた") else {
        panic!("saved の返事ではない")
    };
    assert!(saved.written);
    assert_eq!(PathBuf::from(&saved.path), file);
    assert_eq!(saved.sets_written.len(), 1);
    assert!(saved.backup.is_some(), "上書きは前の版を退避に残す");
    assert!(!live.app.ops().save_pending());
    assert!(!live.app.state.modified, "保存したので変更ありの印が消える");
    let mut host = FileHost::new(PathPolicy::new(&dir).unwrap());
    host.open(&file, true).unwrap();
    let names = host
        .with_document(None, |doc| {
            doc.layers()
                .iter()
                .map(|l| l.name().to_owned())
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert!(names.iter().any(|n| n == "保存する層"), "{names:?}");
    assert_eq!(live.app.state.message, "外からの操作: 保存");
    live.app.state.message.clear();
    let Reply::Saved(again) = live.ok(json!({"command": "save", "args": {"confirm": true}})) else {
        panic!()
    };
    assert!(!again.written);
    live.settle();
    assert!(
        live.app.state.message.is_empty(),
        "書かなかった保存は知らせない: {}",
        live.app.state.message
    );
}

#[test]
fn saving_through_the_link_says_which_never_saved_unreadable_set_was_left_out() {
    let dir = tmp::test_dir("mcpserver-leftout");
    let cache = tmp::test_dir("mcpserver-leftout-cache");
    // タイルの大きさに合う大きさの文書（`fill` はタイルごと画素を入れる）
    let mut live = Live::on_with(AppState::new_in(256, 128, Lang::Ja));
    live.frame();
    live.app.state.apply(Action::Project(NpAction::AddSet));
    assert_eq!(live.app.state.sets.len(), 2, "{}", live.app.state.message);
    for i in 0..2 {
        crate::disk_cache::fill(live.app.state.set_doc_mut(i), 70 + i as u64);
    }
    live.app.state.modified = true;
    // 2 つ目のセットだけが読めなくなる（まだ 1 度も保存していない）
    crate::disk_cache::make_unreadable(&live.app.state, 1, &cache);
    live.app.state.check_tile_cache();
    let left = live.app.state.sets.get(1).unwrap().name.clone();
    let file = dir.join("left.ylp");
    let Reply::Saved(saved) =
        live.ok(json!({"command": "save_as", "args": {"path": file.display().to_string()}}))
    else {
        panic!()
    };
    assert!(saved.written && file.exists());
    assert_eq!(saved.sets_written.len(), 1);
    assert!(
        saved.notes.iter().any(|n| n.ja
            == format!("テクスチャセット「{left}」は読めないため、保存に入れていません")
            && n.en
                == format!(
                    "Texture set \"{left}\" could not be read and was left out of the save"
                )),
        "{:?}",
        saved.notes
    );
    // 保存したファイルには、読めたセットだけが入っている
    let project = yolu_io::Project::read(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(project.sets().len(), 1);
    assert_eq!(project.sets()[0].id, live.app.state.sets.get(0).unwrap().id);
}

#[test]
fn saving_a_new_document_needs_a_destination_and_replacing_a_file_needs_confirmation() {
    let dir = tmp::test_dir("mcpserver-saveas");
    let mut live = Live::on();
    live.frame();
    let e = live.err(json!({"command": "save", "args": {"confirm": true}}));
    assert_eq!(e.code, ErrorCode::InvalidValue);
    let e = live.err(
        json!({"command": "save_as", "args": {"path": dir.join("x.png").display().to_string()}}),
    );
    assert_eq!(e.code, ErrorCode::PathRefused);
    let file = dir.join("new.ylp");
    let Reply::Saved(saved) =
        live.ok(json!({"command": "save_as", "args": {"path": file.display().to_string()}}))
    else {
        panic!()
    };
    assert!(saved.written && file.exists());
    assert!(saved.backup.is_none(), "新しいファイルには退避が無い");
    let Reply::Doc(doc) = live.ok(json!({"command": "doc.info"})) else {
        panic!()
    };
    assert_eq!(
        PathBuf::from(&doc.path),
        file,
        "保存した先が今のファイルになる"
    );
    assert!(!doc.unsaved);
    let other = dir.join("other.ylp");
    std::fs::copy(&file, &other).unwrap();
    let e = live.err(json!({"command": "save_as", "args": {"path": other.display().to_string()}}));
    assert_eq!(e.code, ErrorCode::ConfirmRequired);
    live.app.state.message.clear();
    live.ok(json!({"command": "save_as", "args": {"path": other.display().to_string(), "confirm": true}}));
    assert_eq!(
        live.app.state.message, "外からの操作: 名前を付けて保存",
        "置き換えたときだけ知らせる"
    );
}

#[test]
fn exports_write_the_current_sets_composite() {
    let dir = tmp::test_dir("mcpserver-export");
    let mut live = Live::on();
    live.frame();
    let out = dir.join("out");
    let Reply::Exported(exported) = live.ok(
        json!({"command": "export.channels", "args": {"dir": out.display().to_string(), "channels": ["Color"], "name": "tex"}}),
    ) else {
        panic!("exported の返事ではない")
    };
    assert_eq!(exported.files.len(), 1);
    let written = PathBuf::from(&exported.files[0].path);
    assert!(written.exists() && written.starts_with(&out), "{written:?}");
    assert_eq!(
        (exported.files[0].width, exported.files[0].height),
        (64, 64)
    );
    let e = live.err(
        json!({"command": "export.channels", "args": {"dir": out.display().to_string(), "channels": ["Color"], "name": "tex"}}),
    );
    assert_eq!(e.code, ErrorCode::ConfirmRequired);
}

// ───────── 切る ─────────

#[test]
fn turning_off_answers_the_waiting_requests_without_running_them() {
    let mut live = Live::on();
    live.frame();
    let before = layer_count(&live);
    let rx = live.send(json!({"command": "layer.add", "args": {"kind": "paint"}}));
    // フレームを回さずに列に入るまで待ち、切る
    let deadline = Instant::now() + WATCHDOG;
    while live.app.ops().waiting() < 1 {
        assert!(Instant::now() < deadline, "列に入らない");
        std::thread::sleep(Duration::from_millis(5));
    }
    live.app.state.prefs.settings.external_ops = false;
    live.frame();
    let reply = live.until("切ったあとの返事", |_| rx.try_recv().ok());
    let e = reply.expect_err("実行せずに断る");
    assert!(yolu_cli::live::is_unreachable(&e), "{e:?}");
    assert!(
        e.message.ja.contains("受け付けをやめました"),
        "{}",
        e.message.ja
    );
    live.settle();
    assert_eq!(layer_count(&live), before, "切ったあとの要求は実行しない");
}

#[test]
fn turning_off_while_a_save_is_pending_answers_it_and_the_save_still_finishes() {
    let dir = tmp::test_dir("mcpserver-off");
    let file = dir.join("work.ylp");
    let mut live = Live::on();
    live.app.state.save.background = true;
    live.app.state.apply(Action::SaveProjectAs(file.clone()));
    live.app.state.wait_save();
    live.frame();
    live.ok(json!({"command": "layer.add", "args": {"kind": "paint", "name": "切る前"}}));
    let hold = live.app.state.save.hold_next();
    let rx = live.send(json!({"command": "save", "args": {"confirm": true}}));
    live.until("保存の開始", |l| l.app.state.is_saving().then_some(()));
    live.app.state.prefs.settings.external_ops = false;
    live.frame();
    assert_eq!(live.app.state.ops.status, OpsStatus::Off);
    let reply = live.until("切ったあとの客の結果", |_| rx.try_recv().ok());
    let error = reply.expect_err("返事は来ず、受け付けをやめた誤りになる");
    assert!(yolu_cli::live::is_unreachable(&error), "{error:?}");
    assert!(
        error.message.ja.contains("受け付けをやめました"),
        "{}",
        error.message.ja
    );
    hold.release();
    live.app.state.wait_save();
    assert!(!live.app.state.modified, "保存そのものは続いて終わる");
}

// ───────── 起動 ─────────

#[test]
fn a_saved_setting_makes_the_app_listen_at_startup_and_an_old_settings_file_does_not() {
    let dir = tmp::test_dir("mcpserver-settings");
    let settings = dir.join("settings.conf");
    let free = {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        l.local_addr().unwrap().port()
    };
    std::fs::write(
        &settings,
        format!("language=ja\nexternal_ops=on\nexternal_ops_port={free}\n"),
    )
    .unwrap();
    let ctx = egui::Context::default();
    let mut app =
        YoluApp::for_context_with_settings(&ctx, Some(settings.clone()), PenInput::detached());
    assert!(app.state.prefs.settings.external_ops);
    app.tick_hidden(&ctx);
    assert_eq!(app.state.ops.status, OpsStatus::Listening);
    assert_eq!(app.ops().port(), Some(free));
    drop(app);
    assert!(
        TcpStream::connect(("127.0.0.1", free)).is_err(),
        "閉じると待ちをやめる"
    );
    // この項目を知らない古い設定のファイルは切（待ち受けない）
    std::fs::write(&settings, "language=ja\nlivelink_on_startup=off\n").unwrap();
    let mut old = YoluApp::for_context_with_settings(&ctx, Some(settings), PenInput::detached());
    assert!(!old.state.prefs.settings.external_ops);
    assert_eq!(
        old.state.prefs.settings.external_ops_port,
        yolu_mcp::DEFAULT_PORT
    );
    old.tick_hidden(&ctx);
    assert_eq!(old.state.ops.status, OpsStatus::Off);
}
