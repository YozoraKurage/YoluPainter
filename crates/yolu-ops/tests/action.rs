//! アクション（命令の列を 1 回の取り消しで当てる）と、相対の指し方（`$selected`・`$created:<n>`）の試験。

mod common;

use common::*;
use serde_json::{json, Value};
use yolu_ops::action::{self, ActionFile, MAX_COMMANDS};
use yolu_ops::refs::{substitute_created, Created};
use yolu_ops::reply::*;
use yolu_ops::{execute_in, parse_command, Command, ErrorCode};

fn commands(values: &[Value]) -> Vec<Command> {
    values.iter().map(|v| parse_command(v).unwrap()).collect()
}

#[test]
fn an_action_applies_every_command_as_one_undo_step_with_created_references() {
    let fx = Fixture::new("action-run");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let before = state(&mut host);
    let before_undo = undo_count(&mut host);
    let list = commands(&[
        json!({"command": "layer.add", "args": {"kind": "paint", "name": "Made", "above": "Base"}}),
        json!({"command": "layer.set", "args": {"layer": "$created:1", "opacity": 0.25, "blend_mode": "Multiply"}}),
        json!({"command": "effect.add", "args": {"layer": "$created:1", "kind": "blur", "values": {"radius": 3}}}),
        json!({"command": "effect.set", "args": {"layer": "$created:1", "effect": "$created:2", "strength": 0.5}}),
        json!({"command": "mask.add", "args": {"layer": "$created:1"}}),
        json!({"command": "layer.add", "args": {"kind": "group", "name": "Holder"}}),
        json!({"command": "layer.move", "args": {"layer": "$created:1", "parent": "$created:3", "index": 0}}),
    ]);
    let done = action::run(&mut host, &list).unwrap();
    assert_eq!(done.steps.len(), list.len());
    assert_eq!(
        done.steps[0].layer.as_deref(),
        Some(layer_id(&mut host, "Made").as_str())
    );
    assert!(!done.unchanged);
    assert_eq!(undo_count(&mut host), before_undo + 1, "全部で 1 段");
    let made = layer_id(&mut host, "Made");
    let Reply::Layer(info) = ok(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": made}}),
    ) else {
        panic!()
    };
    assert_eq!(info.summary.opacity, 0.25);
    assert_eq!(info.summary.blend_mode, "Multiply");
    assert_eq!(info.effects.len(), 1);
    assert_eq!(info.effects[0].strength, 0.5);
    assert!(info.mask.is_some());
    assert_eq!(
        info.summary.parent.as_deref(),
        Some(layer_id(&mut host, "Holder").as_str())
    );
    // 取り消し 1 回で全部戻り、やり直し 1 回で全部また当たる
    let after = state(&mut host);
    ok(&mut host, json!({"command": "undo"}));
    assert_eq!(state(&mut host), before);
    ok(&mut host, json!({"command": "redo"}));
    assert_eq!(state(&mut host), after);
}

#[test]
fn the_action_run_command_applies_the_list_as_one_undo_step_and_rolls_back_on_refusal() {
    let fx = Fixture::new("action-command");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let before = state(&mut host);
    let before_undo = undo_count(&mut host);
    // 命令の JSON（CLI・MCP と同じ形）から
    let Reply::Action(done) = ok(
        &mut host,
        json!({"command": "action.run", "args": {"commands": [
            {"command": "layer.add", "args": {"kind": "paint", "name": "Made"}},
            {"v": 1, "command": "effect.add", "args": {"layer": "$created:1", "kind": "invert"}},
            {"command": "layer.set", "args": {"layer": "Base", "opacity": 0.5}},
        ]}}),
    ) else {
        panic!("action の返事")
    };
    assert_eq!(done.steps.len(), 3);
    let made = layer_id(&mut host, "Made");
    assert_eq!(done.steps[0].layer.as_deref(), Some(made.as_str()));
    assert!(done.steps[1].effect.is_some());
    assert!(!done.unchanged);
    assert_eq!(done.undo_count as usize, before_undo + 1);
    assert_eq!(undo_count(&mut host), before_undo + 1, "全部で 1 段");
    ok(&mut host, json!({"command": "undo"}));
    assert_eq!(state(&mut host), before, "取り消し 1 回で全部戻る");
    // 途中で断られたら全部戻し、何番目かを言う
    let e = err(
        &mut host,
        json!({"command": "action.run", "args": {"commands": [
            {"command": "layer.add", "args": {"kind": "paint"}},
            {"command": "layer.set", "args": {"layer": "No such layer", "visible": false}},
        ]}}),
    );
    assert_eq!(e.code, ErrorCode::NotFound);
    let data = e.data.unwrap();
    assert_eq!(
        (data["index"].clone(), data["completed"].clone()),
        (json!(1), json!(1))
    );
    assert_eq!(state(&mut host), before);
    // 入れられない命令（入れ子の action.run も）・読めない命令は、当てる前に断る
    for (item, code) in [
        (
            json!({"command": "action.run", "args": {"commands": [{"command": "mask.add", "args": {"layer": "Base"}}]}}),
            ErrorCode::Unsupported,
        ),
        (
            json!({"command": "save", "args": {"confirm": true}}),
            ErrorCode::Unsupported,
        ),
        (json!({"command": "nope"}), ErrorCode::UnknownCommand),
        (
            json!({"command": "layer.add", "args": {"kind": "nope"}}),
            ErrorCode::InvalidRequest,
        ),
        (
            json!({"command": "layer.delete", "args": {"layer": "Base"}}),
            ErrorCode::ConfirmRequired,
        ),
    ] {
        let e = err(
            &mut host,
            json!({"command": "action.run", "args": {"commands": [
                {"command": "layer.add", "args": {"kind": "paint"}},
                item.clone(),
            ]}}),
        );
        assert_eq!(e.code, code, "{item}");
        assert_eq!(e.data.unwrap()["index"], 1, "{item}");
        assert_eq!(state(&mut host), before, "{item}");
    }
    let e = err(
        &mut host,
        json!({"command": "action.run", "args": {"commands": []}}),
    );
    assert_eq!(e.code, ErrorCode::InvalidRequest);
    let many: Vec<Value> = (0..=MAX_COMMANDS)
        .map(|_| json!({"command": "mask.add", "args": {"layer": "Base"}}))
        .collect();
    let e = err(
        &mut host,
        json!({"command": "action.run", "args": {"commands": many}}),
    );
    assert_eq!(e.code, ErrorCode::Budget);
    assert_eq!(undo_count(&mut host), before_undo);
}

#[test]
fn a_refused_command_rolls_back_everything_before_it_and_says_which() {
    let fx = Fixture::new("action-fail");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let before = state(&mut host);
    let before_undo = undo_count(&mut host);
    let list = commands(&[
        json!({"command": "layer.add", "args": {"kind": "paint", "name": "Made"}}),
        json!({"command": "layer.set", "args": {"layer": "Base", "opacity": 0.5}}),
        json!({"command": "layer.set", "args": {"layer": "No such layer", "visible": false}}),
        json!({"command": "layer.set", "args": {"layer": "Base", "visible": false}}),
    ]);
    let e = action::run(&mut host, &list).unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    let data = e.data.unwrap();
    assert_eq!(data["index"], 2);
    assert_eq!(data["command"], "layer.set");
    assert_eq!(data["completed"], 2);
    assert_eq!(state(&mut host), before, "断った命令の前の分も戻る");
    assert_eq!(undo_count(&mut host), before_undo, "取り消しの段を残さない");
    // 作った物の番号の外・種類の違い
    let e = action::run(
        &mut host,
        &commands(&[
            json!({"command": "layer.add", "args": {"kind": "paint"}}),
            json!({"command": "effect.set", "args": {"layer": "Base", "effect": "$created:1", "strength": 0.5}}),
        ]),
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidValue, "{}", e.message.en);
    assert_eq!(e.data.unwrap()["index"], 1);
    let e = action::run(
        &mut host,
        &commands(&[
            json!({"command": "layer.set", "args": {"layer": "$created:2", "visible": false}}),
        ]),
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert_eq!(state(&mut host), before);
}

#[test]
fn an_action_refuses_commands_it_cannot_hold_before_changing_anything() {
    let fx = Fixture::new("action-check");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let before = state(&mut host);
    for (value, code) in [
        (
            json!({"command": "save", "args": {"confirm": true}}),
            ErrorCode::Unsupported,
        ),
        (json!({"command": "undo"}), ErrorCode::Unsupported),
        (
            json!({"command": "export.channels", "args": {"dir": "out"}}),
            ErrorCode::Unsupported,
        ),
        (json!({"command": "set.info"}), ErrorCode::Unsupported),
        (
            json!({"command": "layer.delete", "args": {"layer": "Base"}}),
            ErrorCode::ConfirmRequired,
        ),
    ] {
        let list = commands(&[
            json!({"command": "layer.add", "args": {"kind": "paint"}}),
            value.clone(),
        ]);
        let e = action::run(&mut host, &list).unwrap_err();
        assert_eq!(e.code, code, "{value}");
        assert_eq!(e.data.unwrap()["index"], 1, "{value}");
        assert_eq!(state(&mut host), before, "{value}");
    }
    // 1 つのテクスチャセットだけ
    let list = commands(&[
        json!({"command": "layer.add", "args": {"kind": "paint", "set": "Body"}}),
        json!({"command": "layer.add", "args": {"kind": "paint", "set": "Other"}}),
    ]);
    assert_eq!(
        action::run(&mut host, &list).unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        action::run(&mut host, &[]).unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    let many = vec![
        commands(&[json!({"command": "mask.add", "args": {"layer": "Base"}})])[0]
            .clone();
        MAX_COMMANDS + 1
    ];
    assert_eq!(
        action::run(&mut host, &many).unwrap_err().code,
        ErrorCode::Budget
    );
    assert_eq!(state(&mut host), before);
}

#[test]
fn selected_is_refused_without_a_screen_and_created_only_inside_a_run() {
    let fx = Fixture::new("action-relative");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    // .ylp には選んでいたレイヤーが入っていない
    let e = err(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "$selected", "visible": false}}),
    );
    assert_eq!(e.code, ErrorCode::NotFound);
    assert!(e.message.ja.contains(".ylp"), "{}", e.message.ja);
    let e = action::run(
        &mut host,
        &commands(&[
            json!({"command": "layer.add", "args": {"kind": "paint"}}),
            json!({"command": "mask.add", "args": {"layer": "$selected"}}),
        ]),
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert_eq!(e.data.unwrap()["index"], 1);
    // 1 つだけの命令では、作った物の番号は使えない
    let e = err(
        &mut host,
        json!({"command": "mask.add", "args": {"layer": "$created:1"}}),
    );
    assert_eq!(e.code, ErrorCode::InvalidRequest);
    for bad in ["$created:0", "$created:x", "$created:", "$created:+1"] {
        let e = err(
            &mut host,
            json!({"command": "mask.add", "args": {"layer": bad}}),
        );
        assert_eq!(e.code, ErrorCode::InvalidValue, "{bad}");
    }
    let e = err(
        &mut host,
        json!({"command": "effect.delete", "args": {"layer": "Base", "effect": "$selected", "confirm": true}}),
    );
    assert_eq!(e.code, ErrorCode::InvalidValue);
    // `$` で始まるほかの名前は名前
    let e = err(
        &mut host,
        json!({"command": "mask.add", "args": {"layer": "$other"}}),
    );
    assert_eq!(e.code, ErrorCode::NotFound);
    assert_eq!(e.data.unwrap()["name"], "$other");
}

#[test]
fn a_batch_counts_what_it_created_across_commands() {
    let fx = Fixture::new("batch-created");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let mut created = Created::default();
    for value in [
        json!({"command": "layer.add", "args": {"kind": "paint", "name": "One"}}),
        json!({"command": "effect.add", "args": {"layer": "$created:1", "kind": "invert"}}),
        json!({"command": "effect.set", "args": {"layer": "$created:1", "effect": "$created:2", "enabled": false}}),
    ] {
        let command = parse_command(&value).unwrap();
        let reply = execute_in(&mut host, &command, Some(&created)).unwrap();
        created.note(&command, &reply);
    }
    assert_eq!(created.len(), 2);
    let Reply::Layer(info) = ok(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": "One"}}),
    ) else {
        panic!()
    };
    assert!(!info.effects[0].enabled);
    // 送る前に `$created` だけを替え、`$selected` は残す（起動中のアプリへ 1 つずつ送る batch）
    let command = parse_command(
        &json!({"command": "layer.move", "args": {"layer": "$created:1", "parent": "$selected"}}),
    )
    .unwrap();
    let Command::LayerMove(moved) = substitute_created(&command, &created).unwrap() else {
        panic!()
    };
    assert_eq!(moved.layer, info.summary.id);
    assert_eq!(moved.parent.as_deref(), Some("$selected"));
}

#[test]
fn action_files_round_trip_and_bad_files_are_refused_with_a_reason() {
    let list = commands(&[
        json!({"command": "layer.add", "args": {"kind": "fill", "name": "Wash", "fill": {"Color": "#336699"}}}),
        json!({"command": "layer.delete", "args": {"layer": "$created:1", "confirm": true}}),
    ]);
    let file = ActionFile::new("Wash", list.clone());
    let text = file.to_text();
    let read = action::parse_action(&text).unwrap();
    assert_eq!(read, file);
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["format"], 1);
    assert_eq!(value["commands"][0]["command"], "layer.add");
    let bad = |v: Value| action::parse_action(&v.to_string()).unwrap_err();
    assert_eq!(
        bad(json!({"format": 2, "name": "x", "commands": []})).code,
        ErrorCode::UnsupportedVersion
    );
    assert_eq!(
        bad(json!({"format": 1, "name": "x", "commands": [], "extra": 1})).code,
        ErrorCode::InvalidProject
    );
    assert_eq!(
        bad(json!({"format": 1, "name": "", "commands": []})).code,
        ErrorCode::InvalidValue
    );
    assert_eq!(
        bad(json!({"format": 1, "name": " x", "commands": []})).code,
        ErrorCode::InvalidValue
    );
    let e = bad(
        json!({"format": 1, "name": "x", "commands": [{"command": "save", "args": {"confirm": true}}]}),
    );
    assert_eq!(e.code, ErrorCode::Unsupported);
    assert_eq!(e.data.unwrap()["index"], 0);
    let e = bad(
        json!({"format": 1, "name": "x", "commands": [{"command": "layer.add", "args": {"kind": "paint"}}, {"command": "nope"}]}),
    );
    assert_eq!(e.code, ErrorCode::UnknownCommand);
    assert_eq!(e.data.unwrap()["index"], 1);
    let too_many: Vec<Value> = (0..=MAX_COMMANDS)
        .map(|_| json!({"command": "mask.add", "args": {"layer": "Base"}}))
        .collect();
    assert_eq!(
        bad(json!({"format": 1, "name": "x", "commands": too_many})).code,
        ErrorCode::Budget
    );
    assert_eq!(
        action::parse_action("{not json").unwrap_err().code,
        ErrorCode::InvalidProject
    );
    assert_eq!(
        action::parse_action(&" ".repeat((action::MAX_FILE_BYTES + 1) as usize))
            .unwrap_err()
            .code,
        ErrorCode::Budget
    );
}
