//! 外からの操作（CLI・MCP のクライアント）を受ける、起動中のアプリの側（`opslive`・`ops_host`）。画面は描かず、アプリを窓の無いフレーム
//! （`tick_hidden`）で回し、本物のソケット（名前は試験ごとに別）でつなぐ。
//!
//! - 設定の入切で待ち受ける・やめる（鍵のファイルも消える）。つないだまま切ると `Bye` で閉じる。2 つ目のアプリは理由を出して待ち受けない。
//! - 命令は画面のスレッドで当たり、1 命令 = 画面の取り消しの 1 段（画面の取り消し・やり直しでそのまま戻る）。
//! - 描いている最中・保存の途中・読むだけのセットは理由つきで断る。`doc.open` は開き替えない。壊す操作は確認が要り、済んだら短く知らせる。
//! - 保存は裏のスレッドで動かし、返事は保存が終わってから。保存したファイルは画面なしのホストで読み戻せる。
use crate::common::{names, tmp, wait::WATCHDOG};

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use yolu_app::engine::DVec2;
use yolu_app::lang::Lang;
use yolu_app::opslive::OpsStatus;
use yolu_app::pen::PenInput;
use yolu_app::state::{Action, AppState};
use yolu_app::YoluApp;
use yolu_ops::link::{
    decode_response, encode_request, read_frame, Received, Request, Response, KIND_REQUEST,
};
use yolu_ops::reply::{Reply, SetState};
use yolu_ops::{parse_command, ErrorCode, FileHost, OpError, OpHost, PathPolicy};
use yolu_protocol::link::{connect_and_greet_as, LinkError};
use yolu_protocol::{AppVersion, Connection, Identity, Kind, Message, RejectCode};

/// 窓の無いアプリ（フレームは `tick_hidden`）。
struct Live {
    ctx: egui::Context,
    app: YoluApp,
}

impl Live {
    /// 外からの操作を受ける設定のアプリ（まだ待ち受けない。最初のフレームで始まる）。
    fn on(tag: &str) -> (Live, String) {
        let name = names::unique_name("ylops", tag);
        let mut live = Live::off(&name);
        live.app.state.prefs.settings.external_ops = true;
        (live, name)
    }

    fn off(name: &str) -> Live {
        let ctx = egui::Context::default();
        let mut app = YoluApp::for_context(&ctx, AppState::new(64, 64), PenInput::detached());
        app.ops_mut().set_name(name).unwrap();
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

    fn connect(&mut self, name: &str) -> Client {
        let client = Client::connect(name);
        self.until("つながり", |l| matches!(l.app.state.ops.status, OpsStatus::Connected(_)).then_some(()));
        client
    }

    /// 命令を送り、返事を待つ（アプリのフレームを回しながら）。
    fn ask(&mut self, client: &mut Client, command: Value) -> Result<Reply, OpError> {
        let id = client.send(command);
        let response = self.until("返事", |_| match client.rx.try_recv() {
            Ok(Got::Response(r)) => Some(r),
            Ok(other) => panic!("返事の代わりに {other:?}"),
            Err(_) => None,
        });
        assert_eq!(response.id, id, "返事は要求の番号に結ばれる");
        response.outcome
    }

    fn ok(&mut self, client: &mut Client, command: Value) -> Reply {
        let name = command["command"].clone();
        self.ask(client, command).unwrap_or_else(|e| panic!("{name} が断られた: {e} {:?}", e.data))
    }

    fn err(&mut self, client: &mut Client, command: Value) -> OpError {
        let name = command["command"].clone();
        self.ask(client, command).expect_err(&format!("{name} は断られるはず"))
    }

    fn layer_names(&self) -> Vec<String> {
        self.app.state.doc.layers().iter().map(|l| l.name().to_owned()).collect()
    }
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)] // 試験の中で数件しか持たない
enum Got {
    Response(Response),
    Bye,
    Closed,
}

/// クライアントの役（CLI・MCP）。返事は別のスレッドで読む（アプリのフレームを回すのは試験のスレッド）。
struct Client {
    conn: Connection,
    rx: Receiver<Got>,
    next_id: u64,
    /// 落とすと、読みのスレッドも終わって、つながりが切れる（読みのスレッドがつながりを持っているので、`conn` を落とすだけでは切れない）。
    done: Arc<AtomicBool>,
}

impl Drop for Client {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
    }
}

impl Client {
    fn connect(name: &str) -> Client {
        Client::try_connect(name).expect("つなげる")
    }

    /// つなぐ（挨拶で断られれば `LinkError::Rejected`）。
    fn try_connect(name: &str) -> Result<Client, LinkError> {
        let identity = Identity::standalone("試験のクライアント").with_version(Some(AppVersion::new(0, 4, 0)));
        let (conn, mut reader, _) = connect_and_greet_as(name, &identity)?;
        let (tx, rx) = mpsc::channel();
        let done = Arc::new(AtomicBool::new(false));
        let stop = done.clone();
        reader.set_timeout(Some(Duration::from_millis(50)));
        std::thread::spawn(move || loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let (frames, stream) = reader.raw();
            let mut stream = stream;
            match read_frame(frames, &mut stream) {
                Ok(Received::Frame(frame)) if frame.kind == Kind::Bye as u16 => {
                    let _ = tx.send(Got::Bye);
                }
                Ok(Received::Frame(frame)) => match decode_response(&frame) {
                    Ok(response) => {
                        let _ = tx.send(Got::Response(response));
                    }
                    Err(e) => panic!("返事を読めない: {e}"),
                },
                Ok(Received::Idle) => {}
                Ok(Received::Closed) | Err(_) => {
                    let _ = tx.send(Got::Closed);
                    break;
                }
            }
        });
        Ok(Client { conn, rx, next_id: 1, done })
    }

    fn send(&mut self, command: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let request = Request { id, command: parse_command(&command).expect("命令の形") };
        self.conn.send_frame(&encode_request(&request).unwrap()).expect("送れる");
        id
    }

    /// 礼儀正しく抜ける（`Bye` の枠を送ってから切る）。
    fn leave(self) {
        let _ = self.conn.send(&Message::Bye);
    }

    /// 閉じられたこと（`Bye` の枠のあと、つながりが切れる）を待つ。
    fn wait_closed(&self, live: &mut Live) {
        let mut said_bye = false;
        live.until("閉じる", |_| loop {
            match self.rx.try_recv() {
                Ok(Got::Bye) => said_bye = true,
                Ok(Got::Closed) => return Some(()),
                Ok(Got::Response(r)) => panic!("閉じる前に返事が来た: {r:?}"),
                Err(_) => return None,
            }
        });
        assert!(said_bye, "閉じる前に Bye を送る");
    }
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

// ───────── 設定の入切 ─────────

#[test]
fn the_setting_starts_and_stops_listening_and_removes_the_key_file() {
    let name = names::unique_name("ylops", "toggle");
    let mut live = Live::off(&name);
    // 既定は切: 待ち受けない（つなげない・鍵も無い）
    assert!(!live.app.state.prefs.settings.external_ops);
    live.settle();
    assert_eq!(live.app.state.ops.status, OpsStatus::Off);
    assert!(yolu_protocol::link::connect(&name).is_err());
    let key = yolu_protocol::auth::key_path(&name).unwrap();
    assert!(!key.exists());
    // 入れると待ち受ける
    live.app.state.prefs.settings.external_ops = true;
    live.frame();
    assert_eq!(live.app.state.ops.status, OpsStatus::Listening);
    assert!(live.app.state.ops.is_on());
    assert!(key.exists(), "鍵のファイルは待ち受けている間だけ");
    assert!(!live.app.state.ops.tooltip(Lang::Ja).is_empty() && !live.app.state.ops.tooltip(Lang::En).is_empty());
    let mut client = live.connect(&name);
    assert_eq!(live.app.state.ops.status, OpsStatus::Connected(1));
    // 命令が通る
    let reply = live.ok(&mut client, json!({"command": "doc.info"}));
    let Reply::Doc(doc) = reply else { panic!("doc.info の返事ではない") };
    assert_eq!(doc.sets.len(), 1);
    assert_eq!(doc.path, "", "まだファイルが無い文書");
    // つないだまま切ると、Bye を送って閉じる。待ち受けもやめ、鍵も消える
    live.app.state.prefs.settings.external_ops = false;
    live.frame();
    assert_eq!(live.app.state.ops.status, OpsStatus::Off);
    client.wait_closed(&mut live);
    assert!(!key.exists());
    assert!(yolu_protocol::link::connect(&name).is_err());
    // もう一度入れると、また受ける（新しい鍵で）
    live.app.state.prefs.settings.external_ops = true;
    live.frame();
    assert_eq!(live.app.state.ops.status, OpsStatus::Listening);
    let mut again = live.connect(&name);
    assert!(matches!(live.ok(&mut again, json!({"command": "doc.info"})), Reply::Doc(_)));
}

#[test]
fn a_second_app_with_the_same_name_does_not_listen_and_says_why() {
    let (mut first, name) = Live::on("twice");
    first.frame();
    assert_eq!(first.app.state.ops.status, OpsStatus::Listening);
    let mut second = Live::off(&name);
    second.app.state.prefs.settings.external_ops = true;
    second.frame();
    let OpsStatus::Failed(reason) = second.app.state.ops.status.clone() else {
        panic!("2 つ目は待ち受けない: {:?}", second.app.state.ops.status)
    };
    assert!(!reason.is_empty());
    assert!(second.app.state.ops.tooltip(Lang::Ja).contains(&reason), "理由が印のツールチップに出る");
    assert!(!second.app.state.message.is_empty(), "理由を知らせる");
    // 先に待ち受けている方は影響を受けず、受け続ける
    assert_eq!(first.app.state.ops.status, OpsStatus::Listening);
    let mut client = first.connect(&name);
    assert!(matches!(first.ok(&mut client, json!({"command": "doc.info"})), Reply::Doc(_)));
    // 毎フレームは試さない（設定を切って入れ直すまで、失敗のまま）
    second.settle();
    assert!(matches!(second.app.state.ops.status, OpsStatus::Failed(_)));
    second.app.state.prefs.settings.external_ops = false;
    second.frame();
    assert_eq!(second.app.state.ops.status, OpsStatus::Off);
}

// ───────── 命令は画面の取り消しの 1 段 ─────────

#[test]
fn one_command_is_one_step_of_the_screens_undo_and_redo() {
    let (mut live, name) = Live::on("undo");
    live.frame();
    let mut client = live.connect(&name);
    let before = layer_count(&live);
    let undo_before = live.app.state.doc.undo_count();
    // 層を足す: 1 命令 = 1 段
    let reply = edited(live.ok(&mut client, json!({"command": "layer.add", "args": {"kind": "paint", "name": "外から"}})));
    assert_eq!(layer_count(&live), before + 1);
    assert_eq!(live.app.state.doc.undo_count(), undo_before + 1);
    assert!(reply.can_undo);
    assert!(live.app.state.modified, "変更ありの印が付く");
    // 値を複数まとめて変える命令も 1 段
    let undo_mid = live.app.state.doc.undo_count();
    live.ok(
        &mut client,
        json!({"command": "layer.set", "args": {"layer": "外から", "opacity": 0.5, "visible": false, "name": "外から 2"}}),
    );
    assert_eq!(live.app.state.doc.undo_count(), undo_mid + 1);
    let layer = live.app.state.doc.layers().iter().find(|l| l.name() == "外から 2").expect("名前が変わった");
    assert!((layer.opacity() - 0.5).abs() < 1e-9 && !layer.visible());
    // 画面の取り消し（Ctrl+Z と同じ道）で 1 段ずつ戻る
    live.app.state.apply(Action::Undo);
    let layer = live.app.state.doc.layers().iter().find(|l| l.name() == "外から").expect("名前が戻った");
    assert!((layer.opacity() - 1.0).abs() < 1e-9 && layer.visible());
    live.app.state.apply(Action::Undo);
    assert_eq!(layer_count(&live), before);
    // やり直しも画面と同じ
    live.app.state.apply(Action::Redo);
    assert_eq!(layer_count(&live), before + 1);
    // 外からの undo は画面の取り消しの並びを 1 段戻す
    let undone = live.ok(&mut client, json!({"command": "undo"}));
    assert!(matches!(undone, Reply::Undone(_)));
    assert_eq!(layer_count(&live), before);
    live.ok(&mut client, json!({"command": "redo"}));
    assert_eq!(layer_count(&live), before + 1);
}

#[test]
fn deleting_a_selected_layer_keeps_the_screens_selection_valid() {
    let (mut live, name) = Live::on("selection");
    live.frame();
    let mut client = live.connect(&name);
    live.ok(&mut client, json!({"command": "layer.add", "args": {"kind": "paint", "name": "選ぶ"}}));
    let id = live.app.state.doc.layers().iter().find(|l| l.name() == "選ぶ").unwrap().id();
    live.app.state.selected_layer = Some(id);
    live.ok(&mut client, json!({"command": "layer.delete", "args": {"layer": "選ぶ", "confirm": true}}));
    let selected = live.app.state.selected_layer.expect("選び直される");
    assert!(live.app.state.doc.layer(selected).is_some(), "消した層を選んだままにしない");
}

#[test]
fn reads_describe_the_open_project_and_the_current_set() {
    let (mut live, name) = Live::on("reads");
    live.frame();
    let mut client = live.connect(&name);
    let info = first_set(live.ok(&mut client, json!({"command": "set.info"})));
    assert_eq!((info.width, info.height), (64, 64));
    assert_eq!(info.state, SetState::Editable);
    assert_eq!(info.layers.len(), layer_count(&live));
    // セットの名前でも ID でも指せる。知らない名前は一覧つきで断る
    let by_name = first_set(live.ok(&mut client, json!({"command": "set.info", "args": {"set": info.name}})));
    assert_eq!(by_name.id, info.id);
    let e = live.err(&mut client, json!({"command": "set.info", "args": {"set": "無いセット"}}));
    assert_eq!(e.code, ErrorCode::NotFound);
    assert!(e.data.unwrap()["sets"].is_array());
    // 見本は今の合成から（PNG の大きさが最大の辺に収まる）
    let Reply::Preview(preview) = live.ok(&mut client, json!({"command": "preview", "args": {"max_edge": 16}})) else {
        panic!("preview の返事ではない")
    };
    assert_eq!((preview.width, preview.height, preview.source_width), (16, 16, 64));
    assert_eq!(&preview.png.0[1..4], b"PNG");
    // 読む命令は何も変えない
    assert!(!live.app.state.modified);
}

// ───────── 断る ─────────

#[test]
fn edits_are_refused_while_drawing_and_reads_still_answer() {
    let (mut live, name) = Live::on("drawing");
    live.frame();
    let mut client = live.connect(&name);
    let layer = live.app.state.doc.layers().last().unwrap().id();
    let brush = live.app.state.stroke_settings(false);
    let mut stroke = live.app.state.doc.begin_stroke(layer, &brush).unwrap();
    stroke.add_point(&mut live.app.state.doc, 10.0, 10.0, 1.0, DVec2::ZERO).unwrap();
    assert!(live.app.state.is_stroking());
    let before = layer_count(&live);
    for command in [
        json!({"command": "layer.add", "args": {"kind": "paint"}}),
        json!({"command": "undo"}),
        json!({"command": "preview"}),
    ] {
        let e = live.err(&mut client, command.clone());
        assert_eq!(e.code, ErrorCode::Busy, "{command}");
        assert!(!e.message.ja.is_empty() && !e.message.en.is_empty());
    }
    assert_eq!(layer_count(&live), before, "断った命令は何も変えない");
    // 読むだけの命令は答える
    assert!(matches!(live.ok(&mut client, json!({"command": "set.info"})), Reply::Set(_)));
    // 終われば通る
    live.app.state.doc.end_stroke(stroke).unwrap();
    live.ok(&mut client, json!({"command": "layer.add", "args": {"kind": "paint"}}));
    assert_eq!(layer_count(&live), before + 1);
}

#[test]
fn a_read_only_set_refuses_edits_with_the_screens_reason() {
    let (mut live, name) = Live::on("readonly");
    live.app.state.sets.get_mut(0).unwrap().read_only = Some("編集できない中身があります".into());
    live.frame();
    let mut client = live.connect(&name);
    let e = live.err(&mut client, json!({"command": "layer.add", "args": {"kind": "paint"}}));
    assert_eq!(e.code, ErrorCode::ReadOnly);
    assert!(e.message.ja.contains("編集できない中身があります"));
    let info = first_set(live.ok(&mut client, json!({"command": "set.info"})));
    assert_eq!(info.state, SetState::ReadOnly);
    let Reply::Doc(doc) = live.ok(&mut client, json!({"command": "doc.info"})) else { panic!() };
    assert_eq!(doc.sets[0].state, SetState::ReadOnly);
    assert!(doc.sets[0].reason.is_some());
}

#[test]
fn destructive_commands_need_confirmation_and_say_so_briefly_when_done() {
    let (mut live, name) = Live::on("confirm");
    live.frame();
    let mut client = live.connect(&name);
    live.ok(&mut client, json!({"command": "layer.add", "args": {"kind": "paint", "name": "消す"}}));
    let before = layer_count(&live);
    live.app.state.message.clear();
    let e = live.err(&mut client, json!({"command": "layer.delete", "args": {"layer": "消す"}}));
    assert_eq!(e.code, ErrorCode::ConfirmRequired);
    assert_eq!(layer_count(&live), before);
    assert!(live.app.state.message.is_empty(), "断ったときは知らせない");
    live.ok(&mut client, json!({"command": "layer.delete", "args": {"layer": "消す", "confirm": true}}));
    assert_eq!(layer_count(&live), before - 1);
    assert_eq!(live.app.state.message, "外からの操作: レイヤーを消す");
    // 命令ごとのログは出さない: 壊さない命令は知らせない
    live.app.state.message.clear();
    live.ok(&mut client, json!({"command": "layer.add", "args": {"kind": "paint"}}));
    live.ok(&mut client, json!({"command": "set.info"}));
    assert!(live.app.state.message.is_empty(), "{}", live.app.state.message);
}

#[test]
fn opening_another_document_is_refused_and_the_same_file_returns_the_current_one() {
    let dir = tmp::test_dir("opslive-open");
    let file = dir.join("work.ylp");
    let (mut live, name) = Live::on("open");
    live.app.state.apply(Action::SaveProjectAs(file.clone()));
    live.app.state.wait_save();
    assert!(file.exists());
    live.frame();
    let mut client = live.connect(&name);
    let e = live.err(&mut client, json!({"command": "doc.open", "args": {"path": dir.join("other.ylp").display().to_string()}}));
    assert_eq!(e.code, ErrorCode::Unsupported);
    let Reply::Doc(doc) = live.ok(&mut client, json!({"command": "doc.open", "args": {"path": file.display().to_string()}})) else {
        panic!()
    };
    assert_eq!(PathBuf::from(&doc.path), file);
}

#[test]
fn a_malformed_request_is_refused_with_its_number_and_the_link_stays_usable() {
    let (mut live, name) = Live::on("malformed");
    live.frame();
    let mut client = live.connect(&name);
    // 壊れた JSON（番号を読めないので 0）
    client.conn.send_raw(KIND_REQUEST, b"{not json").unwrap();
    let got = live.until("断り", |_| client.rx.try_recv().ok());
    let Got::Response(response) = got else { panic!("{got:?}") };
    assert_eq!(response.id, 0);
    assert_eq!(response.outcome.unwrap_err().code, ErrorCode::InvalidRequest);
    // 知らない命令（番号は読める）
    client
        .conn
        .send_raw(KIND_REQUEST, br#"{"v":1,"id":41,"command":"no.such"}"#)
        .unwrap();
    let got = live.until("断り", |_| client.rx.try_recv().ok());
    let Got::Response(response) = got else { panic!("{got:?}") };
    assert_eq!(response.id, 41);
    assert_eq!(response.outcome.unwrap_err().code, ErrorCode::UnknownCommand);
    // つながりは使える
    assert!(matches!(live.ok(&mut client, json!({"command": "doc.info"})), Reply::Doc(_)));
}

// ───────── 保存 ─────────

#[test]
fn saving_runs_in_the_background_and_the_reply_comes_when_it_is_done() {
    let dir = tmp::test_dir("opslive-save");
    let file = dir.join("work.ylp");
    let (mut live, name) = Live::on("save");
    live.app.state.save.background = true;
    live.app.state.apply(Action::SaveProjectAs(file.clone()));
    live.app.state.wait_save();
    live.frame();
    let mut client = live.connect(&name);
    // 保存のあとに編集すると、上書き保存が書く
    live.ok(&mut client, json!({"command": "layer.add", "args": {"kind": "paint", "name": "保存する層"}}));
    // 確認が無ければ断る
    let e = live.err(&mut client, json!({"command": "save"}));
    assert_eq!(e.code, ErrorCode::ConfirmRequired);
    // 保存の仕事を止めておき、返事が保存の終わりまで来ないこと・その間の編集と保存が断られることを見る
    let hold = live.app.state.save.hold_next();
    let id = client.send(json!({"command": "save", "args": {"confirm": true}}));
    live.until("保存の開始", |l| l.app.state.is_saving().then_some(()));
    assert!(live.app.ops().save_pending());
    live.settle();
    assert!(client.rx.try_recv().is_err(), "保存が終わるまで返事は来ない");
    let e = live.err(&mut client, json!({"command": "layer.add", "args": {"kind": "paint"}}));
    assert_eq!(e.code, ErrorCode::Busy, "保存の途中の編集は断る");
    let e = live.err(&mut client, json!({"command": "save", "args": {"confirm": true}}));
    assert_eq!(e.code, ErrorCode::Busy, "保存の途中の 2 回目の保存は断る");
    assert!(matches!(live.ok(&mut client, json!({"command": "set.info"})), Reply::Set(_)), "読む命令は保存の途中でも答える");
    hold.release();
    let response = live.until("保存の返事", |_| match client.rx.try_recv() {
        Ok(Got::Response(r)) => Some(r),
        Ok(other) => panic!("{other:?}"),
        Err(_) => None,
    });
    assert_eq!(response.id, id);
    let Reply::Saved(saved) = response.outcome.expect("保存できた") else { panic!("saved の返事ではない") };
    assert!(saved.written);
    assert_eq!(PathBuf::from(&saved.path), file);
    assert_eq!(saved.sets_written.len(), 1);
    assert!(saved.backup.is_some(), "上書きは前の版を退避に残す");
    assert!(!live.app.ops().save_pending());
    assert!(!live.app.state.modified, "保存したので変更ありの印が消える");
    // 保存したファイルを画面なしのホストで読み戻すと、外から足した層がある
    let mut host = FileHost::new(PathPolicy::new(&dir).unwrap());
    host.open(&file, true).unwrap();
    let names = host
        .with_document(None, |doc| doc.layers().iter().map(|l| l.name().to_owned()).collect::<Vec<_>>())
        .unwrap();
    assert!(names.iter().any(|n| n == "保存する層"), "{names:?}");
    // 書いた上書き保存は短く知らせる
    assert_eq!(live.app.state.message, "外からの操作: 保存");
    // 何も変えていなければ、もう一度保存しても書かない。何も壊していないので知らせない
    live.app.state.message.clear();
    let Reply::Saved(again) = live.ok(&mut client, json!({"command": "save", "args": {"confirm": true}})) else { panic!() };
    assert!(!again.written);
    live.settle();
    assert!(live.app.state.message.is_empty(), "書かなかった保存は知らせない: {}", live.app.state.message);
}

#[test]
fn saving_a_new_document_needs_a_destination_and_replacing_a_file_needs_confirmation() {
    let dir = tmp::test_dir("opslive-saveas");
    let (mut live, name) = Live::on("saveas");
    live.frame();
    let mut client = live.connect(&name);
    // まだファイルが無い文書の上書き保存は、保存先を言って断る
    let e = live.err(&mut client, json!({"command": "save", "args": {"confirm": true}}));
    assert_eq!(e.code, ErrorCode::InvalidValue);
    // 名前は .ylp で終わる
    let e = live.err(&mut client, json!({"command": "save_as", "args": {"path": dir.join("x.png").display().to_string()}}));
    assert_eq!(e.code, ErrorCode::PathRefused);
    // 新しいファイルへ（同期の保存でも、返事は返る）
    let file = dir.join("new.ylp");
    let Reply::Saved(saved) = live.ok(&mut client, json!({"command": "save_as", "args": {"path": file.display().to_string()}})) else {
        panic!()
    };
    assert!(saved.written && file.exists());
    assert!(saved.backup.is_none(), "新しいファイルには退避が無い");
    let Reply::Doc(doc) = live.ok(&mut client, json!({"command": "doc.info"})) else { panic!() };
    assert_eq!(PathBuf::from(&doc.path), file, "保存した先が今のファイルになる");
    assert!(!doc.unsaved);
    // もうあるファイルへの保存は、確認が要る
    let other = dir.join("other.ylp");
    std::fs::copy(&file, &other).unwrap();
    let e = live.err(&mut client, json!({"command": "save_as", "args": {"path": other.display().to_string()}}));
    assert_eq!(e.code, ErrorCode::ConfirmRequired);
    live.app.state.message.clear();
    live.ok(&mut client, json!({"command": "save_as", "args": {"path": other.display().to_string(), "confirm": true}}));
    assert_eq!(live.app.state.message, "外からの操作: 名前を付けて保存", "置き換えたときだけ知らせる");
}

#[test]
fn exports_write_the_current_sets_composite() {
    let dir = tmp::test_dir("opslive-export");
    let (mut live, name) = Live::on("export");
    live.frame();
    let mut client = live.connect(&name);
    let out = dir.join("out");
    let Reply::Exported(exported) = live.ok(
        &mut client,
        json!({"command": "export.channels", "args": {"dir": out.display().to_string(), "channels": ["Color"], "name": "tex"}}),
    ) else {
        panic!("exported の返事ではない")
    };
    assert_eq!(exported.files.len(), 1);
    let written = PathBuf::from(&exported.files[0].path);
    assert!(written.exists() && written.starts_with(&out), "{written:?}");
    assert_eq!((exported.files[0].width, exported.files[0].height), (64, 64));
    // もうあるファイルへの書き出しは確認が要る
    let e = live.err(
        &mut client,
        json!({"command": "export.channels", "args": {"dir": out.display().to_string(), "channels": ["Color"], "name": "tex"}}),
    );
    assert_eq!(e.code, ErrorCode::ConfirmRequired);
}

// ───────── つながり ─────────

#[test]
fn several_clients_share_the_link_and_a_closed_one_is_forgotten() {
    let (mut live, name) = Live::on("several");
    live.frame();
    let mut a = live.connect(&name);
    let mut b = Client::connect(&name);
    live.until("2 つ目のつながり", |l| (l.app.state.ops.status == OpsStatus::Connected(2)).then_some(()));
    // 同じ画面のスレッドで、順に実行される
    live.ok(&mut a, json!({"command": "layer.add", "args": {"kind": "paint", "name": "A"}}));
    live.ok(&mut b, json!({"command": "layer.add", "args": {"kind": "paint", "name": "B"}}));
    let names = live.layer_names();
    assert!(names.iter().any(|n| n == "A") && names.iter().any(|n| n == "B"));
    // 何も言わずに切れたつながり（プログラムが落ちた）を忘れる
    drop(a);
    live.until("閉じたつながりを忘れる", |l| (l.app.state.ops.status == OpsStatus::Connected(1)).then_some(()));
    assert!(matches!(live.ok(&mut b, json!({"command": "doc.info"})), Reply::Doc(_)));
    // 礼儀正しく抜けたつながり（Bye を送る）も忘れる
    b.leave();
    live.until("抜けたつながりを忘れる", |l| (l.app.state.ops.status == OpsStatus::Listening).then_some(()));
}

/// 数は挨拶（鍵と版）が済んでから取る。何も言わないつなぎは枠を取らず、9 つ目は挨拶で `Busy` と断られ（知らせは 1 度だけ）、閉じたつながりの分は戻る。
#[test]
fn the_ninth_client_is_refused_as_busy_and_a_closed_ones_place_is_given_back() {
    let (mut live, name) = Live::on("limit");
    live.frame();
    let mut clients = Vec::new();
    for n in 1..=7 {
        clients.push(Client::connect(&name));
        live.until("つながり", |l| (l.app.state.ops.status == OpsStatus::Connected(n)).then_some(()));
    }
    // 挨拶を送らないつなぎは、数に入らない（8 つ目がつなげる）
    let silent = yolu_protocol::link::connect(&name).expect("つなげる");
    clients.push(Client::connect(&name));
    live.until("8 つ目のつながり", |l| (l.app.state.ops.status == OpsStatus::Connected(8)).then_some(()));
    drop(silent);
    live.until("挨拶の無いつなぎを断った知らせ", |l| {
        (l.app.state.message == "外からの操作: 挨拶できないクライアントを断りました。").then_some(())
    });
    // 9 つ目は挨拶で断られる。知らせは 1 度だけ。つながっている 8 つは影響を受けない
    live.app.state.message.clear();
    let busy = |name: &str| match Client::try_connect(name) {
        Err(LinkError::Rejected(r)) => r.code,
        Err(e) => panic!("断りの代わりに誤り: {e}"),
        Ok(_) => panic!("9 つ目はつなげない"),
    };
    assert_eq!(busy(&name), RejectCode::Busy);
    live.until("上限の知らせ", |l| (!l.app.state.message.is_empty()).then_some(()));
    assert_eq!(live.app.state.message, "外からの操作: つながりの数が上限のため断りました。");
    live.app.state.message.clear();
    assert_eq!(busy(&name), RejectCode::Busy);
    live.settle();
    assert!(live.app.state.message.is_empty(), "同じ断りが続いても知らせは 1 度: {}", live.app.state.message);
    assert_eq!(live.app.state.ops.status, OpsStatus::Connected(8));
    assert!(matches!(live.ok(&mut clients[0], json!({"command": "doc.info"})), Reply::Doc(_)));
    // 1 つ閉じると、その分が戻って、またつなげる（閉じた知らせとつなぎ側の枠の戻しは別なので、少し待って繰り返す）
    drop(clients.remove(0));
    live.until("閉じたつながりを忘れる", |l| (l.app.state.ops.status == OpsStatus::Connected(7)).then_some(()));
    let mut again = live.until("枠が戻ってつなげる", |_| Client::try_connect(&name).ok());
    live.until("つなぎ直し", |l| (l.app.state.ops.status == OpsStatus::Connected(8)).then_some(()));
    assert!(matches!(live.ok(&mut again, json!({"command": "doc.info"})), Reply::Doc(_)));
}

#[test]
fn requests_left_on_a_connection_after_the_setting_is_turned_off_are_not_executed() {
    let (mut live, name) = Live::on("afteroff");
    live.frame();
    let mut client = live.connect(&name);
    let before = layer_count(&live);
    // 切ったあとに、まだ残っているつながりから要求が来ても実行しない
    live.app.state.prefs.settings.external_ops = false;
    client.send(json!({"command": "layer.add", "args": {"kind": "paint"}}));
    live.frame();
    live.frame();
    client.wait_closed(&mut live);
    live.settle();
    assert_eq!(layer_count(&live), before, "切ったあとの要求は実行しない");
}

// ───────── 起動 ─────────

#[test]
fn a_saved_setting_makes_the_app_listen_at_startup_and_an_old_settings_file_does_not() {
    let dir = tmp::test_dir("opslive-settings");
    let settings = dir.join("settings.conf");
    // 入れた設定を書いたファイルで起動すると、最初のフレームで待ち受ける
    std::fs::write(&settings, "language=ja\nexternal_ops=on\n").unwrap();
    let name = names::unique_name("ylops", "startup");
    let ctx = egui::Context::default();
    let mut app = YoluApp::for_context_with_settings(&ctx, Some(settings.clone()), PenInput::detached());
    app.ops_mut().set_name(&name).unwrap();
    assert!(app.state.prefs.settings.external_ops);
    app.tick_hidden(&ctx);
    assert_eq!(app.state.ops.status, OpsStatus::Listening);
    drop(app);
    // この項目を知らない古い設定のファイルは切（待ち受けない）
    std::fs::write(&settings, "language=ja\nlivelink_on_startup=off\n").unwrap();
    let name = names::unique_name("ylops", "startup-old");
    let mut old = YoluApp::for_context_with_settings(&ctx, Some(settings), PenInput::detached());
    old.ops_mut().set_name(&name).unwrap();
    assert!(!old.state.prefs.settings.external_ops);
    old.tick_hidden(&ctx);
    assert_eq!(old.state.ops.status, OpsStatus::Off);
    assert!(yolu_protocol::link::connect(&name).is_err());
}

/// コマンドラインの客（`yolu_cli::live`。`yolupainter-cli` の起動中のアプリへの道と、MCP の `file` を省いた呼び出しが使う）が、本物の受け口と
/// 話せる: 1 回の呼び出しごとにつなぎ、命令が画面の文書に当たり、読む命令が答える。返事を待つ間に受け口を切ると、アプリは返事の代わりに
/// `Bye` を送って閉じ、客はそれを「受け付けをやめた」（つなげない誤り）として返す（返事でない枠として、古い版のアプリと取り違えない）。
#[test]
fn the_command_line_client_talks_to_the_running_app_and_reads_its_bye() {
    let dir = tmp::test_dir("opslive-cli");
    let file = dir.join("work.ylp");
    let (mut live, name) = Live::on("cli");
    live.app.state.save.background = true;
    live.app.state.apply(Action::SaveProjectAs(file.clone()));
    live.app.state.wait_save();
    live.frame();
    let config = yolu_cli::live::LiveConfig { name, timeout: Duration::from_secs(60) };
    let ask = |command: Value| {
        let config = config.clone();
        let command = parse_command(&command).expect("命令の形");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(yolu_cli::live::call(&config, &command));
        });
        rx
    };
    let rx = ask(json!({"command": "layer.add", "args": {"kind": "paint", "name": "CLI の層"}}));
    let reply = live.until("CLI の編集の返事", |_| rx.try_recv().ok());
    assert!(matches!(reply, Ok(Reply::Edited(_))), "{reply:?}");
    assert!(live.layer_names().iter().any(|n| n == "CLI の層"), "画面の文書に当たる: {:?}", live.layer_names());
    let rx = ask(json!({"command": "doc.info"}));
    let reply = live.until("CLI の読む返事", |_| rx.try_recv().ok());
    let Ok(Reply::Doc(doc)) = reply else { panic!("doc.info の返事ではない: {reply:?}") };
    assert_eq!(PathBuf::from(&doc.path), file);
    // 保存を止めておき、返事を待っている間に受け口を切る
    let hold = live.app.state.save.hold_next();
    let rx = ask(json!({"command": "save", "args": {"confirm": true}}));
    live.until("保存の開始", |l| l.app.state.is_saving().then_some(()));
    live.app.state.prefs.settings.external_ops = false;
    live.frame();
    assert_eq!(live.app.state.ops.status, OpsStatus::Off);
    let reply = live.until("切ったあとの客の結果", |_| rx.try_recv().ok());
    let error = reply.expect_err("返事は来ず、つなげない誤りになる");
    assert!(yolu_cli::live::is_unreachable(&error), "{error:?}");
    assert!(error.message.ja.contains("受け付けをやめました"), "{}", error.message.ja);
    hold.release();
    live.app.state.wait_save();
}
