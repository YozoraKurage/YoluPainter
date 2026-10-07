//! 起動中のアプリへの道（`--live`・`--port`）の試験。アプリの代わりに、本物の MCP の受け口（`yolu_mcp::http`）を立て、
//! 受けた命令を本物の文書に当てる待ち受け（`FakeApp`）で確かめる。実際のアプリとの試験は yolu-app の headless の試験にある。

mod common;

use std::time::{Duration, Instant};

use common::*;
use serde_json::Value;
use yolu_cli::cli::exit_code;
use yolu_cli::live::{call, is_unreachable, LiveConfig};
use yolu_ops::{parse_command_str, ErrorCode, Reply};

fn live_args<'a>(port: &'a str, rest: &[&'a str]) -> Vec<&'a str> {
    let mut args: Vec<&str> = rest.to_vec();
    args.extend(["--live", "--port", port]);
    args
}

#[test]
fn commands_reach_the_running_app_and_its_state_persists_between_calls() {
    let fx = Fixture::new("live");
    fx.project("a.ylp");
    let app = FakeApp::start(&fx, "a.ylp", Behavior::Serve);
    let port = app.port_arg();
    let doc = fx.ok(&live_args(&port, &["doc.info"]));
    assert_eq!(doc["reply"], "doc");
    assert_eq!(doc["sets"][0]["name"], "Body");
    // 1 回の呼び出しごとにつなぎ直しても、アプリの中の文書は続く（編集 → 取り消しの段 → 取り消し）
    let edited = fx.ok(&live_args(
        &port,
        &["layer.set", "--layer", "Base", "--opacity", "0.3"],
    ));
    assert_eq!(edited["reply"], "edited");
    assert_eq!(fx.ok(&live_args(&port, &["history.info"]))["undo_count"], 1);
    assert_eq!(
        fx.ok(&live_args(&port, &["layer.get", "--layer", "Base"]))["opacity"],
        0.3
    );
    assert_eq!(fx.ok(&live_args(&port, &["undo"]))["steps"], 1);
    assert_eq!(
        fx.ok(&live_args(&port, &["layer.get", "--layer", "Base"]))["opacity"],
        1.0
    );
    assert_eq!(app.served(), 6);
    // --file が無ければ起動中のアプリが相手（--live は省ける）
    let out = fx.cli(&["doc.info", "--port", &port]);
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
}

#[test]
fn the_apps_refusals_arrive_as_they_are() {
    let fx = Fixture::new("live-refuse");
    fx.project("a.ylp");
    let app = FakeApp::start(&fx, "a.ylp", Behavior::Serve);
    let port = app.port_arg();
    // 確認が無ければ、アプリ側が断る（要求は同じ型を通る）
    let (code, error) = fx.fails(&live_args(&port, &["layer.delete", "--layer", "Tint"]));
    assert_eq!(
        (code, error["code"].as_str().unwrap()),
        (4, "confirm_required")
    );
    let (code, error) = fx.fails(&live_args(&port, &["layer.get", "--layer", "Nope"]));
    assert_eq!((code, error["code"].as_str().unwrap()), (1, "not_found"));
    // 見本の画像も運べる（MCP の画像の content から戻す）
    let preview = fx.ok(&live_args(&port, &["preview", "--max-edge", "16"]));
    assert!(preview["png"].as_str().unwrap().len() > 100);
    let written = fx.ok(&live_args(
        &port,
        &["preview", "--max-edge", "16", "--out", "p.png"],
    ));
    assert!(fx.path("p.png").is_file() && written["png_file"].is_string());
}

#[test]
fn an_app_that_is_not_listening_is_reported_with_exit_code_3_and_how_to_fix_it() {
    let fx = Fixture::new("live-none");
    let port = unused_port().to_string();
    let (code, error) = fx.fails(&["doc.info", "--live", "--port", &port]);
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
    assert!(error["message"]["en"].as_str().unwrap().contains(&port));
}

#[test]
fn a_peer_that_closes_or_answers_wrongly_is_an_error_not_a_hang() {
    let fx = Fixture::new("live-bad");
    fx.project("a.ylp");
    let config = |app: &FakeApp, secs: u64| LiveConfig {
        port: app.port,
        timeout: Duration::from_secs(secs),
    };
    let command = parse_command_str(r#"{"command":"doc.info"}"#).unwrap();

    let closer = FakeApp::start(&fx, "a.ylp", Behavior::CloseWithoutReply);
    let started = Instant::now();
    let e = call(&config(&closer, 20), &command).unwrap_err();
    assert!(is_unreachable(&e) && exit_code(&e) == 3, "{e:?}");
    assert!(
        e.message.en.contains("unknown whether the command ran"),
        "{}",
        e.message.en
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "閉じられたら待たずに返す"
    );

    let other = FakeApp::start(&fx, "a.ylp", Behavior::NotMcp);
    let e = call(&config(&other, 20), &command).unwrap_err();
    assert!(is_unreachable(&e), "{e:?}");
    assert!(
        e.message.en.contains("cannot be read") || e.message.en.contains("not the MCP endpoint"),
        "{}",
        e.message.en
    );

    let busy = FakeApp::start(&fx, "a.ylp", Behavior::Busy);
    let e = call(&config(&busy, 20), &command).unwrap_err();
    assert_eq!(e.code, ErrorCode::Busy, "{e:?}");

    let silent = FakeApp::start(&fx, "a.ylp", Behavior::Silent);
    let started = Instant::now();
    let e = call(&config(&silent, 1), &command).unwrap_err();
    assert_eq!(e.code, ErrorCode::Busy, "{e:?}");
    assert!(
        e.message.en.contains("unknown whether the command ran"),
        "操作が済んだか分からないことを言う: {}",
        e.message.en
    );
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn the_default_port_is_the_apps_default() {
    let default = LiveConfig::default();
    assert_eq!(default.port, yolu_mcp::DEFAULT_PORT);
    assert_eq!(default.timeout, yolu_cli::live::DEFAULT_TIMEOUT);
}

#[test]
fn live_replies_are_the_same_values_the_headless_host_gives() {
    let fx = Fixture::new("live-same");
    fx.project("a.ylp");
    let app = FakeApp::start(&fx, "a.ylp", Behavior::Serve);
    let port = app.port_arg();
    for command in [
        &["set.info"][..],
        &["doc.info"],
        &["layer.get", "--layer", "Base"],
        &["effect.list_kinds"],
    ] {
        let live = fx.ok(&live_args(&port, command));
        let mut on_file = command.to_vec();
        on_file.extend(["--file", "a.ylp"]);
        let file = fx.ok(&on_file);
        if command[0] == "doc.info" {
            // 道は相手（起動中のアプリ・画面なしで開いた物）の言い方
            assert_eq!(live["sets"], file["sets"]);
            continue;
        }
        assert_eq!(
            live, file,
            "{command:?}: 同じ命令は、アプリでも画面なしでも同じ返事"
        );
    }
    let reply: Reply = serde_json::from_value::<Value>(fx.ok(&live_args(&port, &["set.info"])))
        .and_then(serde_json::from_value)
        .unwrap();
    assert!(matches!(reply, Reply::Set(_)));
}

#[test]
fn a_live_batch_resolves_what_it_created_before_sending() {
    let fx = Fixture::new("live-created");
    fx.project("a.ylp");
    let app = FakeApp::start(&fx, "a.ylp", Behavior::Serve);
    let port = app.port_arg();
    let lines = [
        r#"{"command": "layer.add", "args": {"kind": "paint", "name": "Made"}}"#,
        r#"{"command": "layer.set", "args": {"layer": "$created:1", "opacity": 0.4}}"#,
    ]
    .join("\n");
    let out = fx.cli_in(&live_args(&port, &["batch", "-"]), &lines);
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        fx.ok(&live_args(&port, &["layer.get", "--layer", "Made"]))["opacity"],
        0.4
    );
    // アプリは 1 つずつ受けるので、`$created` をそのまま送ると断る（CLI が送る前に替えている）
    let (code, error) = fx.fails(&live_args(&port, &["mask.add", "--layer", "$created:1"]));
    assert_eq!(
        (code, error["code"].as_str().unwrap()),
        (2, "invalid_request")
    );
}

#[test]
fn run_action_reaches_the_running_app_as_one_undo_step() {
    let fx = Fixture::new("live-action");
    fx.project("a.ylp");
    let app = FakeApp::start(&fx, "a.ylp", Behavior::Serve);
    let port = app.port_arg();
    let action = serde_json::json!({"format": 1, "name": "Wash", "commands": [
        {"command": "layer.add", "args": {"kind": "paint", "name": "Made"}},
        {"command": "layer.set", "args": {"layer": "$created:1", "opacity": 0.4}},
        {"command": "mask.add", "args": {"layer": "$created:1"}},
    ]});
    std::fs::write(fx.path("wash.json"), action.to_string()).unwrap();
    let out = fx.ok(&live_args(&port, &["run-action", "wash.json"]));
    assert_eq!(out["action"], "Wash");
    assert_eq!(out["steps"].as_array().unwrap().len(), 3);
    assert_eq!(out["undo_count"], 1);
    assert_eq!(app.served(), 1, "命令 action.run の 1 回で送る");
    let made = fx.ok(&live_args(&port, &["layer.get", "--layer", "Made"]));
    assert_eq!(made["opacity"], 0.4);
    // 取り消し 1 回で全部戻る
    fx.ok(&live_args(&port, &["undo"]));
    let (code, error) = fx.fails(&live_args(&port, &["layer.get", "--layer", "Made"]));
    assert_eq!((code, error["code"].as_str().unwrap()), (1, "not_found"));
    // 途中で断られたら、そこまでの分も戻して何番目かを言う
    let bad = serde_json::json!({"format": 1, "name": "Bad", "commands": [
        {"command": "layer.add", "args": {"kind": "paint", "name": "Made"}},
        {"command": "layer.set", "args": {"layer": "NoSuchLayer", "opacity": 0.4}},
    ]});
    std::fs::write(fx.path("bad.json"), bad.to_string()).unwrap();
    let (code, error) = fx.fails(&live_args(&port, &["run-action", "bad.json"]));
    assert_eq!((code, error["code"].as_str().unwrap()), (1, "not_found"));
    assert_eq!(error["data"]["index"], 1);
    let (code, _) = fx.fails(&live_args(&port, &["layer.get", "--layer", "Made"]));
    assert_eq!(code, 1);
}
