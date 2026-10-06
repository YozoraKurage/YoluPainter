//! JSON と JSON Schema の試験: 命令・返事・誤りが JSON で往復し、型から作ったスキーマと食い違わない。
//! スキーマの検査器は、schemars が出す形（`$ref`・`type`・`properties`・`required`・`additionalProperties`・`items`・`enum`・`const`・
//! `oneOf`/`anyOf`/`allOf`・`minimum`/`maximum`）だけを見る小さなもの（外の検査器を足さない）。

mod common;

use std::collections::BTreeSet;

use common::*;
use serde_json::{json, Value};
use yolu_ops::wire::command_json;
use yolu_ops::{
    commands, parse_command, Command, Danger, ErrorCode, OpError, Reply, COMMAND_VERSION,
};

// ───────── 小さなスキーマ検査器 ─────────

fn type_matches(kind: &str, value: &Value) -> bool {
    match kind {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_f64().is_some_and(|n| n.fract() == 0.0) && value.is_number(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        other => panic!("検査器が知らない type: {other}"),
    }
}

fn resolve<'a>(root: &'a Value, reference: &str) -> &'a Value {
    let path = reference
        .strip_prefix("#/")
        .unwrap_or_else(|| panic!("{reference}"));
    path.split('/').fold(root, |node, key| {
        node.get(key)
            .unwrap_or_else(|| panic!("{reference} が引けない"))
    })
}

fn check(root: &Value, schema: &Value, value: &Value, at: &str, errors: &mut Vec<String>) {
    let Some(object) = schema.as_object() else {
        if schema == &json!(false) {
            errors.push(format!("{at}: false のスキーマ"));
        }
        return;
    };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        check(root, resolve(root, reference), value, at, errors);
    }
    if let Some(t) = object.get("type") {
        let kinds: Vec<&str> = match t {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().map(|x| x.as_str().unwrap()).collect(),
            _ => panic!(),
        };
        if !kinds.iter().any(|k| type_matches(k, value)) {
            errors.push(format!("{at}: type {kinds:?} に合わない（{value}）"));
            return;
        }
    }
    if let Some(c) = object.get("const") {
        if c != value {
            errors.push(format!("{at}: const {c} と違う（{value}）"));
        }
    }
    if let Some(Value::Array(options)) = object.get("enum") {
        if !options.contains(value) {
            errors.push(format!("{at}: enum に無い（{value}）"));
        }
    }
    if let (Some(min), Some(n)) = (
        object.get("minimum").and_then(Value::as_f64),
        value.as_f64(),
    ) {
        if n < min {
            errors.push(format!("{at}: minimum {min} 未満（{n}）"));
        }
    }
    if let (Some(max), Some(n)) = (
        object.get("maximum").and_then(Value::as_f64),
        value.as_f64(),
    ) {
        if n > max {
            errors.push(format!("{at}: maximum {max} 超え（{n}）"));
        }
    }
    if let Some(map) = value.as_object() {
        let properties = object.get("properties").and_then(Value::as_object);
        if let Some(Value::Array(required)) = object.get("required") {
            for key in required {
                if !map.contains_key(key.as_str().unwrap()) {
                    errors.push(format!("{at}: 必須の {key} が無い"));
                }
            }
        }
        for (key, item) in map {
            let known = properties.and_then(|p| p.get(key));
            match (known, object.get("additionalProperties")) {
                (Some(sub), _) => check(root, sub, item, &format!("{at}.{key}"), errors),
                (None, Some(Value::Bool(false))) => errors.push(format!("{at}: 知らない欄 {key}")),
                (None, Some(extra)) if extra.is_object() => {
                    check(root, extra, item, &format!("{at}.{key}"), errors)
                }
                _ => {}
            }
        }
    }
    if let (Some(items), Some(array)) = (object.get("items"), value.as_array()) {
        for (i, item) in array.iter().enumerate() {
            check(root, items, item, &format!("{at}[{i}]"), errors);
        }
    }
    let branches = |key: &str| -> Option<Vec<Vec<String>>> {
        object.get(key).and_then(Value::as_array).map(|list| {
            list.iter()
                .map(|sub| {
                    let mut e = Vec::new();
                    check(root, sub, value, at, &mut e);
                    e
                })
                .collect()
        })
    };
    if let Some(results) = branches("oneOf") {
        let passed = results.iter().filter(|e| e.is_empty()).count();
        if passed != 1 {
            errors.push(format!(
                "{at}: oneOf のうち {passed} 個に合う（1 個のはず）"
            ));
        }
    }
    if let Some(results) = branches("anyOf") {
        if results.iter().all(|e| !e.is_empty()) {
            errors.push(format!("{at}: anyOf のどれにも合わない（{value}）"));
        }
    }
    if let Some(results) = branches("allOf") {
        for e in results {
            errors.extend(e);
        }
    }
}

fn validate(root: &Value, schema: &Value, value: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    check(root, schema, value, "$", &mut errors);
    errors
}

// ───────── 命令の見本（1 命令に 1 つ以上。正規の形: 既定の値の欄は省く） ─────────

fn samples() -> Vec<Value> {
    vec![
        json!({"command": "doc.info", "args": {}}),
        json!({"command": "doc.open", "args": {"path": "a.ylp", "confirm": true}}),
        json!({"command": "set.info", "args": {"set": "Body"}}),
        json!({"command": "layer.get", "args": {"layer": "Base"}}),
        json!({"command": "layer.add", "args": {"kind": "fill", "name": "F", "above": "Base", "fill": {"Color": "#ff8000"}}}),
        json!({"command": "layer.add", "args": {"kind": "adjustment", "adjustment": {"kind": "levels", "values": {"gamma": 2, "x": true, "y": "z"}}, "channels": ["Color"]}}),
        json!({"command": "layer.delete", "args": {"layer": "Base", "confirm": true}}),
        json!({"command": "layer.move", "args": {"layer": "Base", "parent": "Group", "index": 0}}),
        json!({"command": "layer.move", "args": {"layer": "Base", "to_root": true}}),
        json!({"command": "layer.set", "args": {
            "layer": "Base", "name": "N", "visible": false, "opacity": 0.5, "blend_mode": "Multiply", "clipping": true,
            "locks": {"pixels": true, "all": false}, "channels": {"Roughness": true},
            "fill": {"Color": "#102030", "Roughness": null},
            "adjustment": {"kind": "hue_saturation", "values": {"hue": 30}}
        }}),
        json!({"command": "mask.add", "args": {"layer": "Base"}}),
        json!({"command": "mask.delete", "args": {"layer": "Base", "confirm": true}}),
        json!({"command": "mask.set", "args": {"layer": "Base", "enabled": false, "inverted": true, "density": 0.25}}),
        json!({"command": "effect.get", "args": {"layer": "Base", "effect": "0123456789abcdef0123456789abcdef"}}),
        json!({"command": "effect.add", "args": {
            "layer": "Base", "target": "mask", "kind": "blur", "values": {"radius": 3}, "strength": 0.5, "enabled": false, "index": 0
        }}),
        json!({"command": "effect.set", "args": {
            "layer": "Base", "effect": "0123456789abcdef0123456789abcdef", "kind": "sharpen", "values": {"amount": 2.5},
            "channels": ["Color"], "strength": 1.0, "enabled": true, "index": 1
        }}),
        json!({"command": "effect.delete", "args": {"layer": "Base", "effect": "0123456789abcdef0123456789abcdef", "confirm": true}}),
        json!({"command": "effect.list_kinds", "args": {}}),
        json!({"command": "history.info", "args": {"set": "Body"}}),
        json!({"command": "undo", "args": {"steps": 3}}),
        json!({"command": "redo", "args": {}}),
        json!({"command": "preview", "args": {"set": "Body", "channel": "Normal", "max_edge": 256}}),
        json!({"command": "export.channels", "args": {"channels": ["Color"], "dir": "out", "name": "n", "confirm": true}}),
        json!({"command": "export.textures", "args": {"template": "liltoon", "dir": "out"}}),
        json!({"command": "export.psd", "args": {"path": "a.psd", "channel": "Color", "mode": "flat", "confirm": true}}),
        json!({"command": "save", "args": {"confirm": true}}),
        json!({"command": "save_as", "args": {"path": "b.ylp"}}),
    ]
}

fn sample_names() -> BTreeSet<String> {
    samples()
        .iter()
        .map(|s| s["command"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn every_command_has_a_sample_a_spec_and_a_schema_entry_of_the_same_name() {
    let spec_names: BTreeSet<String> = commands().iter().map(|c| c.name.to_owned()).collect();
    assert_eq!(
        spec_names.len(),
        commands().len(),
        "命令の名前が重なっている"
    );
    assert_eq!(
        sample_names(),
        spec_names,
        "見本のない命令・表に無い命令がある"
    );
    let schema = yolu_ops::command_schema();
    let schema_names: BTreeSet<String> = schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            v["properties"]["command"]["const"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        schema_names, spec_names,
        "型（スキーマ）と命令の表の名前が合わない"
    );
    assert_eq!(spec_names.len(), 25);
}

#[test]
fn commands_round_trip_through_json_and_name_themselves_the_same() {
    for sample in samples() {
        let command =
            parse_command(&sample).unwrap_or_else(|e| panic!("{sample}: {}", e.message.en));
        assert_eq!(command.name(), sample["command"].as_str().unwrap());
        let mut again = command_json(&command);
        assert_eq!(again["v"], json!(COMMAND_VERSION));
        again.as_object_mut().unwrap().remove("v");
        assert_eq!(
            again,
            sample,
            "{}: 書き戻すと見本と同じ（既定の値の欄は出さない）",
            command.name()
        );
        // 版つきの JSON も読め、同じ命令になる
        assert_eq!(parse_command(&command_json(&command)).unwrap(), command);
        // serde の素の形（命令の名前と引数）でも往復する
        let plain: Command = serde_json::from_value(sample.clone()).unwrap();
        assert_eq!(plain, command);
    }
}

#[test]
fn samples_fit_the_whole_command_schema_and_their_own_arguments_schema() {
    let whole = yolu_ops::command_schema();
    for sample in samples() {
        let name = sample["command"].as_str().unwrap();
        let errors = validate(&whole, &whole, &sample);
        assert!(errors.is_empty(), "{name}: {errors:?}");
        let spec = yolu_ops::command_spec(name).unwrap();
        let args = spec.args_schema();
        assert_eq!(
            args["type"], "object",
            "{name}: MCP の inputSchema は object"
        );
        let errors = validate(&args, &args, &sample["args"]);
        assert!(errors.is_empty(), "{name}: {errors:?}");
    }
}

#[test]
fn the_schema_rejects_what_the_types_reject() {
    let whole = yolu_ops::command_schema();
    for sample in samples() {
        let name = sample["command"].as_str().unwrap();
        // 知らない欄
        let mut extra = sample.clone();
        extra["args"]
            .as_object_mut()
            .unwrap()
            .insert("bogus".into(), json!(1));
        assert!(
            !validate(&whole, &whole, &extra).is_empty(),
            "{name}: 知らない欄を通した"
        );
        assert!(parse_command(&extra).is_err(), "{name}");
        // 知らない命令の名前
        let mut renamed = sample.clone();
        renamed["command"] = json!("nope");
        assert!(!validate(&whole, &whole, &renamed).is_empty(), "{name}");
        // args が無い
        let mut bare = sample.clone();
        bare.as_object_mut().unwrap().remove("args");
        assert!(
            !validate(&whole, &whole, &bare).is_empty(),
            "{name}（スキーマは args を要る。読み口は args を省ける）"
        );
    }
    // 必須の欄・型・列挙
    let get = yolu_ops::command_spec("layer.get").unwrap().args_schema();
    assert!(!validate(&get, &get, &json!({})).is_empty());
    assert!(!validate(&get, &get, &json!({"layer": 3})).is_empty());
    let add = yolu_ops::command_spec("layer.add").unwrap().args_schema();
    assert!(!validate(&add, &add, &json!({"kind": "sparkle"})).is_empty());
    assert!(validate(&add, &add, &json!({"kind": "group"})).is_empty());
    let set = yolu_ops::command_spec("layer.set").unwrap().args_schema();
    assert!(!validate(&set, &set, &json!({"layer": "a", "opacity": "x"})).is_empty());
    assert!(!validate(
        &set,
        &set,
        &json!({"layer": "a", "channels": {"Color": "yes"}})
    )
    .is_empty());
    let eff = yolu_ops::command_spec("effect.add").unwrap().args_schema();
    assert!(!validate(
        &eff,
        &eff,
        &json!({"layer": "a", "kind": "blur", "values": {"radius": [1]}})
    )
    .is_empty());
    assert!(!validate(
        &eff,
        &eff,
        &json!({"layer": "a", "kind": "blur", "values": {"radius": null}})
    )
    .is_empty());
    assert!(!validate(
        &eff,
        &eff,
        &json!({"layer": "a", "kind": "blur", "target": "everywhere"})
    )
    .is_empty());
}

#[test]
fn a_command_has_a_confirm_argument_exactly_when_it_can_be_destructive() {
    let always: BTreeSet<&str> = ["layer.delete", "mask.delete", "effect.delete", "save"].into();
    let replacing: BTreeSet<&str> = [
        "doc.open",
        "export.channels",
        "export.textures",
        "export.psd",
        "save_as",
    ]
    .into();
    for spec in commands() {
        let args = spec.args_schema();
        let has_confirm = args["properties"].get("confirm").is_some();
        match spec.danger {
            Danger::Always => assert!(always.contains(spec.name), "{}", spec.name),
            Danger::WhenReplacing => assert!(replacing.contains(spec.name), "{}", spec.name),
            Danger::Safe => assert!(
                !always.contains(spec.name) && !replacing.contains(spec.name),
                "{}",
                spec.name
            ),
        }
        assert_eq!(
            has_confirm,
            spec.danger != Danger::Safe,
            "{}: confirm の欄と壊す印が合わない",
            spec.name
        );
        assert_eq!(spec.destructive(), has_confirm, "{}", spec.name);
        // 読むだけの命令は壊さない
        if spec.read_only {
            assert_eq!(spec.danger, Danger::Safe, "{}", spec.name);
        }
        assert!(
            !spec.title.ja.is_empty()
                && !spec.title.en.is_empty()
                && !spec.description.ja.is_empty()
                && !spec.description.en.is_empty(),
            "{}",
            spec.name
        );
        // 壊す命令の説明は、確認が要ることを言う
        if spec.danger == Danger::Always {
            assert!(
                spec.description.en.contains("confirm: true")
                    && spec.description.ja.contains("confirm: true"),
                "{}",
                spec.name
            );
        }
    }
    for name in always.iter().chain(replacing.iter()) {
        assert!(yolu_ops::command_spec(name).is_some(), "{name}");
    }
    // 読むだけの命令
    let read_only: BTreeSet<&str> = commands()
        .iter()
        .filter(|c| c.read_only)
        .map(|c| c.name)
        .collect();
    let expected: BTreeSet<&str> = [
        "doc.info",
        "set.info",
        "layer.get",
        "effect.get",
        "effect.list_kinds",
        "history.info",
        "preview",
    ]
    .into();
    assert_eq!(read_only, expected);
}

// ───────── 返事 ─────────

/// 全部の命令を 1 度ずつ当てて、返事を集める。
fn every_reply() -> Vec<(String, Reply)> {
    let fx = Fixture::new("every-reply");
    fx.project("a.ylp");
    fx.project("b.ylp");
    let mut host = fx.host("a.ylp");
    let base = layer_id(&mut host, "Base");
    let mut out: Vec<(String, Reply)> = Vec::new();
    fn run_one(out: &mut Vec<(String, Reply)>, host: &mut yolu_ops::FileHost, command: Value) {
        let name = command["command"].as_str().unwrap().to_owned();
        let reply = ok(host, command);
        out.push((name, reply));
    }
    run_one(&mut out, &mut host, json!({"command": "doc.info"}));
    run_one(&mut out, &mut host, json!({"command": "set.info"}));
    run_one(
        &mut out,
        &mut host,
        json!({"command": "layer.get", "args": {"layer": base}}),
    );
    run_one(&mut out, &mut host, json!({"command": "effect.list_kinds"}));
    run_one(
        &mut out,
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "adjustment", "name": "Adj", "adjustment": {"kind": "levels"}}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Adj", "opacity": 0.5}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "layer.move", "args": {"layer": "Adj", "index": 0}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "mask.add", "args": {"layer": base}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "mask.set", "args": {"layer": base, "density": 0.5}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "effect.add", "args": {"layer": base, "kind": "blur"}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "effect.add", "args": {"layer": base, "target": "mask", "kind": "noise"}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "effect.get", "args": {"layer": base}}),
    );
    let Reply::Effects(list) = out.last().unwrap().1.clone() else {
        panic!()
    };
    let blur = list.effects[0].id.clone();
    run_one(
        &mut out,
        &mut host,
        json!({"command": "effect.set", "args": {"layer": base, "effect": blur, "values": {"radius": 7}}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "layer.get", "args": {"layer": base}}),
    );
    run_one(&mut out, &mut host, json!({"command": "history.info"}));
    run_one(
        &mut out,
        &mut host,
        json!({"command": "undo", "args": {"steps": 2}}),
    );
    run_one(&mut out, &mut host, json!({"command": "redo"}));
    run_one(
        &mut out,
        &mut host,
        json!({"command": "effect.delete", "args": {"layer": base, "effect": blur, "confirm": true}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "mask.delete", "args": {"layer": base, "confirm": true}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "layer.delete", "args": {"layer": "Adj", "confirm": true}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "preview", "args": {"max_edge": 16}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "export.channels", "args": {"dir": "o1"}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "export.textures", "args": {"dir": "o2"}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "export.psd", "args": {"path": "o3/a.psd"}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "save_as", "args": {"path": "c.ylp"}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "group"}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "save", "args": {"confirm": true}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "save", "args": {"confirm": true}}),
    );
    run_one(
        &mut out,
        &mut host,
        json!({"command": "doc.open", "args": {"path": "b.ylp"}}),
    );
    out
}

#[test]
fn every_reply_round_trips_and_fits_the_reply_schemas() {
    let whole = yolu_ops::reply_schema();
    let replies = every_reply();
    let names: BTreeSet<String> = replies.iter().map(|(n, _)| n.clone()).collect();
    for spec in commands() {
        assert!(
            names.contains(spec.name),
            "{} の返事を集められていない",
            spec.name
        );
    }
    let kinds: BTreeSet<&str> = replies.iter().map(|(_, r)| r.name()).collect();
    for expected in [
        "doc", "set", "layer", "effects", "kinds", "history", "preview", "edited", "undone",
        "exported", "saved",
    ] {
        assert!(kinds.contains(expected), "{expected}");
    }
    for (command, reply) in &replies {
        let json = serde_json::to_value(reply).unwrap();
        assert_eq!(json["reply"], reply.name(), "{command}");
        let back: Reply = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(&back, reply, "{command}: JSON の往復");
        let errors = validate(&whole, &whole, &json);
        assert!(errors.is_empty(), "{command}: {errors:?}");
        // 命令ごとの返事のスキーマ（MCP の outputSchema）は、返事の中身（印なし）に合う
        let spec = yolu_ops::command_spec(command).unwrap();
        assert_eq!(spec.reply, reply.name(), "{command}: 表の返事の種類");
        let payload_schema = spec.reply_schema();
        let payload = reply.payload();
        assert!(payload.get("reply").is_none());
        let errors = validate(&payload_schema, &payload_schema, &payload);
        assert!(errors.is_empty(), "{command}: {errors:?}");
    }
    // 返事のスキーマは、知らない印・足りない欄を断る
    assert!(!validate(&whole, &whole, &json!({"reply": "nope"})).is_empty());
    assert!(!validate(
        &whole,
        &whole,
        &json!({"reply": "history", "undo_count": 1})
    )
    .is_empty());
}

// ───────── 誤り ─────────

fn every_code() -> Vec<ErrorCode> {
    use ErrorCode::*;
    // 変種を増やすと、この match が通らなくなる（試験の一覧に足す）
    let all = [
        InvalidRequest,
        UnknownCommand,
        UnsupportedVersion,
        NoDocument,
        NotFound,
        Ambiguous,
        InvalidValue,
        ReadOnly,
        Unsupported,
        ConfirmRequired,
        PathRefused,
        Budget,
        Refused,
        Busy,
        Conflict,
        InvalidProject,
        Io,
        Cancelled,
        Internal,
    ];
    for code in all {
        match code {
            InvalidRequest | UnknownCommand | UnsupportedVersion | NoDocument | NotFound
            | Ambiguous | InvalidValue | ReadOnly | Unsupported | ConfirmRequired | PathRefused
            | Budget | Refused | Busy | Conflict | InvalidProject | Io | Cancelled | Internal => {}
        }
    }
    all.to_vec()
}

#[test]
fn errors_round_trip_and_the_schema_lists_exactly_the_codes() {
    let schema = yolu_ops::error_schema();
    // 変種に説明があると、schemars は const の列（oneOf）にする。説明が無ければ enum の列
    let code_schema = &schema["$defs"]["ErrorCode"];
    let in_schema: BTreeSet<String> = match code_schema.get("enum").and_then(Value::as_array) {
        Some(list) => list
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect(),
        None => code_schema["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["const"].as_str().unwrap().to_owned())
            .collect(),
    };
    let in_code: BTreeSet<String> = every_code()
        .into_iter()
        .map(|c| {
            serde_json::to_value(c)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(in_schema, in_code);
    for code in every_code() {
        let error = OpError::new(code, "理由", "reason").with_data(json!({"x": [1, 2]}));
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(
            serde_json::from_value::<OpError>(json.clone()).unwrap(),
            error
        );
        assert!(validate(&schema, &schema, &json).is_empty(), "{code:?}");
        let bare = OpError::new(code, "理由", "reason");
        let json = serde_json::to_value(&bare).unwrap();
        assert!(json.get("data").is_none());
        assert!(validate(&schema, &schema, &json).is_empty());
    }
    // 日英の文を欠いた誤りは通らない
    assert!(!validate(
        &schema,
        &schema,
        &json!({"code": "io", "message": {"ja": "x"}})
    )
    .is_empty());
    assert!(!validate(
        &schema,
        &schema,
        &json!({"code": "boom", "message": {"ja": "x", "en": "y"}})
    )
    .is_empty());
}

#[test]
fn real_errors_carry_both_languages_and_fit_the_error_schema() {
    let fx = Fixture::new("real-errors");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let schema = yolu_ops::error_schema();
    for command in [
        json!({"command": "layer.get", "args": {"layer": "x"}}),
        json!({"command": "layer.delete", "args": {"layer": "Base"}}),
        json!({"command": "effect.add", "args": {"layer": "Base", "kind": "blur", "values": {"radius": 999}}}),
        json!({"command": "effect.add", "args": {"layer": "Base", "kind": "nope"}}),
        json!({"command": "doc.open", "args": {"path": "../x.ylp"}}),
        json!({"command": "layer.explode"}),
        json!({"v": 9, "command": "doc.info"}),
        json!({"command": "undo", "args": {"steps": 0}}),
        json!({"command": "layer.set", "args": {"layer": "Base", "opacity": 7}}),
    ] {
        let e = err(&mut host, command.clone());
        assert!(
            !e.message.ja.is_empty() && !e.message.en.is_empty(),
            "{command}"
        );
        assert!(
            !e.message.ja.is_ascii(),
            "{command}: 日本語の文: {}",
            e.message.ja
        );
        assert!(
            e.message.en.is_ascii(),
            "{command}: 英語の文は英語（診断の日本語は data へ）: {}",
            e.message.en
        );
        let json = serde_json::to_value(&e).unwrap();
        assert!(validate(&schema, &schema, &json).is_empty(), "{command}");
        assert_eq!(serde_json::from_value::<OpError>(json).unwrap(), e);
    }
}

// ───────── 効果の種類の一覧 ─────────

#[test]
fn every_effect_kind_and_parameter_has_a_description_in_both_languages() {
    let fx = Fixture::new("kinds");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let Reply::Kinds(kinds) = ok(&mut host, json!({"command": "effect.list_kinds"})) else {
        panic!()
    };
    assert_eq!(
        kinds.kinds.len(),
        yolu_core::effects::catalog::kinds().len()
    );
    for kind in &kinds.kinds {
        assert!(
            !kind.title.ja.is_empty() && !kind.title.en.is_empty(),
            "{}",
            kind.id
        );
        assert!(
            !kind.description.ja.is_empty() && !kind.description.en.is_empty(),
            "{}",
            kind.id
        );
        assert!(!kind.used_as.is_empty());
        for p in &kind.params {
            assert!(
                !p.description.ja.is_empty() && !p.description.en.is_empty(),
                "{}.{}",
                kind.id,
                p.name
            );
            match p.kind {
                yolu_ops::reply::ParamKindName::Integer
                | yolu_ops::reply::ParamKindName::Number => {
                    assert!(
                        p.min.is_some() && p.max.is_some() && p.min < p.max,
                        "{}.{}",
                        kind.id,
                        p.name
                    );
                }
                yolu_ops::reply::ParamKindName::Choice => {
                    assert!(p.options.len() >= 2, "{}.{}", kind.id, p.name)
                }
                yolu_ops::reply::ParamKindName::Boolean => {
                    assert!(p.min.is_none() && p.options.is_empty())
                }
            }
        }
        // 足せない種類は欄を持たず、値で変えられない中身を挙げる
        assert_eq!(
            kind.addable,
            !kind.params.is_empty() || matches!(kind.id.as_str(), "invert" | "normalize"),
            "{}",
            kind.id
        );
        if !kind.addable {
            assert!(!kind.opaque.is_empty(), "{}", kind.id);
        }
    }
    // 範囲は core の検査と同じ定義（表）から
    let blur = kinds.kinds.iter().find(|k| k.id == "blur").unwrap();
    assert_eq!(
        (blur.params[0].min, blur.params[0].max),
        (Some(1.0), Some(256.0))
    );
}

#[test]
fn tool_names_are_unique_and_use_only_characters_the_ai_apis_allow() {
    let mut seen = BTreeSet::new();
    for spec in commands() {
        let tool = spec.tool_name();
        assert!(!tool.is_empty() && tool.len() <= 64, "{tool}");
        assert!(
            tool.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
            "{tool}"
        );
        assert!(
            seen.insert(tool.clone()),
            "ツールの名前が重なっている: {tool}"
        );
        assert_eq!(
            yolu_ops::command_spec_by_tool(&tool).unwrap().name,
            spec.name
        );
    }
    assert_eq!(
        yolu_ops::command_spec_by_tool("layer_set").unwrap().name,
        "layer.set"
    );
    assert!(yolu_ops::command_spec_by_tool("layer.set").is_none());
}
