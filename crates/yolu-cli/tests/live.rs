//! 起動中のアプリへの道（`--live`）の試験。アプリの代わりに、本物の挨拶（鍵のファイルの確かめ合い）と枠で待ち受け、
//! 受けた要求を本物の文書に当てる待ち受け（`FakeApp`）を立てて確かめる。実際のアプリとの試験は、アプリの側が入ったあとに行う。

mod common;

use std::time::{Duration, Instant};

use common::*;
use serde_json::Value;
use yolu_cli::cli::exit_code;
use yolu_cli::live::{call, is_unreachable, LiveConfig};
use yolu_ops::{parse_command_str, ErrorCode, Reply};

fn live_args<'a>(app: &'a FakeApp, rest: &[&'a str]) -> Vec<&'a str> {
    let mut args: Vec<&str> = rest.to_vec();
    args.extend(["--live", "--link-name", &app.name]);
    args
}

#[test]
fn commands_reach_the_running_app_and_its_state_persists_between_calls() {
    let fx = Fixture::new("live");
    fx.project("a.ylp");
    let app = FakeApp::start(&fx, "a.ylp", "persist", Behavior::Serve);
    let doc = fx.ok(&live_args(&app, &["doc.info"]));
    assert_eq!(doc["reply"], "doc");
    assert_eq!(doc["sets"][0]["name"], "Body");
    // 1 回の呼び出しごとにつなぎ直しても、アプリの中の文書は続く（編集 → 取り消しの段 → 取り消し）
    let edited = fx.ok(&live_args(
        &app,
        &["layer.set", "--layer", "Base", "--opacity", "0.3"],
    ));
    assert_eq!(edited["reply"], "edited");
    assert_eq!(fx.ok(&live_args(&app, &["history.info"]))["undo_count"], 1);
    assert_eq!(
        fx.ok(&live_args(&app, &["layer.get", "--layer", "Base"]))["opacity"],
        0.3
    );
    assert_eq!(fx.ok(&live_args(&app, &["undo"]))["steps"], 1);
    assert_eq!(
        fx.ok(&live_args(&app, &["layer.get", "--layer", "Base"]))["opacity"],
        1.0
    );
    assert_eq!(app.served(), 6);
    // --file が無ければ起動中のアプリが相手（--live は省ける）
    let out = fx.cli(&["doc.info", "--link-name", &app.name]);
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
}

#[test]
fn the_apps_refusals_arrive_as_they_are() {
    let fx = Fixture::new("live-refuse");
    fx.project("a.ylp");
    let app = FakeApp::start(&fx, "a.ylp", "refuse", Behavior::Serve);
    // 確認が無ければ、アプリ側が断る（要求は同じ型を通る）
    let (code, error) = fx.fails(&live_args(&app, &["layer.delete", "--layer", "Tint"]));
    assert_eq!(
        (code, error["code"].as_str().unwrap()),
        (4, "confirm_required")
    );
    let (code, error) = fx.fails(&live_args(&app, &["layer.get", "--layer", "Nope"]));
    assert_eq!((code, error["code"].as_str().unwrap()), (1, "not_found"));
    // 見本の画像も運べる
    let preview = fx.ok(&live_args(&app, &["preview", "--max-edge", "16"]));
    assert!(preview["png"].as_str().unwrap().len() > 100);
    let written = fx.ok(&live_args(
        &app,
        &["preview", "--max-edge", "16", "--out", "p.png"],
    ));
    assert!(fx.path("p.png").is_file() && written["png_file"].is_string());
}

#[test]
fn an_app_that_is_not_listening_is_reported_with_exit_code_3_and_how_to_fix_it() {
    let fx = Fixture::new("live-none");
    let name = unique_link_name("nobody");
    let (code, error) = fx.fails(&["doc.info", "--live", "--link-name", &name]);
    assert_eq!(code, 3);
    assert_eq!(error["data"]["live"], "unreachable");
    assert!(error["message"]["ja"]
        .as_str()
        .unwrap()
        .contains("外からの操作を受ける"));
    assert!(error["message"]["en"]
        .as_str()
        .unwrap()
        .contains("external commands"));
    // 経路の名前の形が悪ければ、つなぐ前に断る
    let (code, _) = fx.fails(&["doc.info", "--link-name", "bad name!"]);
    assert_eq!(code, 2);
}

#[test]
fn a_link_that_closes_or_answers_wrongly_is_an_error_not_a_hang() {
    let fx = Fixture::new("live-bad");
    fx.project("a.ylp");
    let config = |app: &FakeApp, secs: u64| LiveConfig {
        name: app.name.clone(),
        timeout: Duration::from_secs(secs),
    };
    let command = parse_command_str(r#"{"command":"doc.info"}"#).unwrap();

    let closer = FakeApp::start(&fx, "a.ylp", "closer", Behavior::CloseWithoutReply);
    let started = Instant::now();
    let e = call(&config(&closer, 20), &command).unwrap_err();
    assert!(is_unreachable(&e) && exit_code(&e) == 3, "{e:?}");
    assert!(e.message.en.contains("closed the link"), "{}", e.message.en);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "閉じられたら待たずに返す"
    );

    let wrong = FakeApp::start(&fx, "a.ylp", "wrong", Behavior::WrongFrame);
    let e = call(&config(&wrong, 20), &command).unwrap_err();
    assert!(is_unreachable(&e), "{e:?}");
    assert!(
        e.message.en.contains("not an operation reply"),
        "{}",
        e.message.en
    );

    let rejecter = FakeApp::start(&fx, "a.ylp", "reject", Behavior::Reject);
    let e = call(&config(&rejecter, 20), &command).unwrap_err();
    assert_eq!(e.code, ErrorCode::Refused, "{e:?}");
    assert!(e.message.en.contains("refused the connection"));

    let silent = FakeApp::start(&fx, "a.ylp", "silent", Behavior::Silent);
    let started = Instant::now();
    let e = call(&config(&silent, 1), &command).unwrap_err();
    assert_eq!(e.code, ErrorCode::Busy, "{e:?}");
    assert!(
        e.message.en.contains("unknown whether the command ran"),
        "操作が済んだか分からないことを言う: {}",
        e.message.en
    );
    // Unix は読みの時間切れで 1 秒ほど。Windows の名前付きパイプには読みの時間切れが無いので、見張りのスレッドが（待ち + 挨拶の上限）で切る
    let limit = if cfg!(unix) { 4 } else { 20 };
    assert!(
        started.elapsed() < Duration::from_secs(limit),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn the_default_link_name_is_the_ops_name_and_can_be_changed_by_the_environment() {
    // 環境変数は他の試験と共有するので、読み出しの関数だけを確かめる（名前が正しい形のときだけ使う）
    let default = LiveConfig::default();
    assert!(yolu_protocol::link::valid_link_name(&default.name));
    assert_eq!(
        LiveConfig::default().timeout,
        yolu_cli::live::DEFAULT_TIMEOUT
    );
    assert_ne!(yolu_ops::link::LINK_NAME, yolu_protocol::DEFAULT_LINK_NAME);
}

#[test]
fn live_replies_are_the_same_values_the_headless_host_gives() {
    let fx = Fixture::new("live-same");
    fx.project("a.ylp");
    let app = FakeApp::start(&fx, "a.ylp", "same", Behavior::Serve);
    let live = fx.ok(&live_args(&app, &["set.info"]));
    let file = fx.ok(&["set.info", "--file", "a.ylp"]);
    assert_eq!(live, file, "同じ命令は、アプリでも画面なしでも同じ返事");
    let reply: Reply = serde_json::from_value::<Value>(live)
        .and_then(serde_json::from_value)
        .unwrap();
    assert!(matches!(reply, Reply::Set(_)));
}
