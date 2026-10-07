//! CLI の試験: .ylp（画面なし）への各命令・保存・確認・まとめて当てる・見本の画像・schema と型の一致・終了コードと日英の文。

mod common;

use common::*;
use serde_json::{json, Value};
use yolu_mcp::tools;
use yolu_ops::value::base64_decode;
use yolu_ops::{command_schema, commands, error_schema, parse_command, reply_schema, ErrorCode};

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

fn layer_id(fx: &Fixture, file: &str, name: &str) -> String {
    let info = fx.ok(&["--file", file, "set.info"]);
    info["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["name"] == name)
        .unwrap_or_else(|| panic!("層 {name}"))["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn reading_commands_describe_the_project() {
    let fx = Fixture::new("read");
    fx.project("a.ylp");
    let doc = fx.ok(&["doc.info", "--file", "a.ylp"]);
    assert_eq!(doc["reply"], "doc");
    assert_eq!(doc["sets"][0]["name"], "Body");
    let set = fx.ok(&["set.info", "--file", "a.ylp", "--pretty"]);
    assert_eq!(set["width"], 64);
    let names: Vec<&str> = set["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Group", "Inner", "Tint", "Base"]);
    let layer = fx.ok(&["layer.get", "--file", "a.ylp", "--layer", "Base"]);
    assert_eq!(layer["reply"], "layer");
    assert_eq!(layer["name"], "Base");
    let history = fx.ok(&["history.info", "--file", "a.ylp"]);
    assert_eq!(history["can_undo"], false);
    let kinds = fx.ok(&["effect.list_kinds", "--file", "a.ylp"]);
    assert!(kinds["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .any(|k| k["id"] == "blur"));
    // pretty は整形、既定は 1 行
    assert!(
        fx.cli(&["doc.info", "--file", "a.ylp", "--pretty"])
            .stdout
            .lines()
            .count()
            > 1
    );
    assert_eq!(
        fx.cli(&["doc.info", "--file", "a.ylp"])
            .stdout
            .lines()
            .count(),
        1
    );
}

#[test]
fn an_edit_without_save_leaves_the_file_unchanged_and_with_save_writes_it() {
    let fx = Fixture::new("save");
    let path = fx.project("a.ylp");
    let before = file_bytes(&path);

    let edited = fx.ok(&[
        "layer.set",
        "--file",
        "a.ylp",
        "--layer",
        "Base",
        "--opacity",
        "0.25",
        "--blend-mode",
        "Multiply",
    ]);
    assert_eq!(edited["reply"], "edited");
    assert_eq!(edited["can_undo"], true);
    assert!(edited.get("saved").is_none());
    assert_eq!(
        file_bytes(&path),
        before,
        "--save が無ければファイルは変わらない"
    );

    let saved = fx.ok(&[
        "layer.set",
        "--file",
        "a.ylp",
        "--layer",
        "Base",
        "--opacity",
        "0.25",
        "--blend-mode",
        "Multiply",
        "--save",
    ]);
    assert_eq!(saved["saved"]["reply"], "saved");
    assert_eq!(saved["saved"]["written"], true);
    assert_ne!(file_bytes(&path), before);
    let layer = fx.ok(&["layer.get", "--file", "a.ylp", "--layer", "Base"]);
    assert_eq!(layer["opacity"], 0.25);
    assert_eq!(layer["blend_mode"], "Multiply");
    // 上書きの前の版は隣の退避に残る（消さない）
    let backups = fx.path("a.ylp-backups~");
    assert!(backups.is_dir() && std::fs::read_dir(backups).unwrap().count() >= 1);
}

#[test]
fn destructive_commands_need_confirm_and_say_so_with_exit_code_4() {
    let fx = Fixture::new("confirm");
    fx.project("a.ylp");
    let (code, error) = fx.fails(&["layer.delete", "--file", "a.ylp", "--layer", "Tint"]);
    assert_eq!(
        (code, error["code"].as_str().unwrap()),
        (4, "confirm_required")
    );
    let (code, _) = fx.fails(&["save", "--file", "a.ylp"]);
    assert_eq!(code, 4, "上書き保存も確認が要る");
    // 確認すれば消せて、取り消せる（命令 1 つ = 取り消し 1 段）
    let batch = r#"[
        {"command":"layer.delete","args":{"layer":"Tint","confirm":true}},
        {"command":"history.info"},
        {"command":"undo"},
        {"command":"set.info"}
    ]"#;
    let out = fx.cli_in(&["batch", "-", "--file", "a.ylp"], batch);
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
    let v: Value = serde_json::from_str(&out.stdout).unwrap();
    let replies = v["replies"].as_array().unwrap();
    assert_eq!(replies[1]["undo_count"], 1);
    assert_eq!(replies[2]["steps"], 1);
    let names: Vec<&str> = replies[3]["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Tint"), "取り消しで戻る: {names:?}");
}

#[test]
fn layer_and_effect_commands_edit_through_flags() {
    let fx = Fixture::new("edit");
    let path = fx.project("a.ylp");
    let base = layer_id(&fx, "a.ylp", "Base");
    let batch = format!(
        "# 層と効果と値を足す\n{}\n{}\n{}\n{}\n",
        json!({"command": "layer.add", "args": {"kind": "fill", "name": "Wash", "fill": {"Color": "#336699"}}}),
        json!({"command": "effect.add", "args": {"layer": base, "kind": "blur", "values": {"radius": 3}}}),
        json!({"command": "mask.add", "args": {"layer": "Tint"}}),
        json!({"command": "layer.set", "args": {"layer": "Wash", "opacity": 0.5, "visible": true}}),
    );
    let out = fx.cli_in(&["batch", "-", "--file", "a.ylp", "--save"], &batch);
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
    let v: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["replies"].as_array().unwrap().len(), 4);
    assert_eq!(v["saved"]["written"], true);
    // 保存した結果を、別の起動で読む
    let layer = fx.ok(&["layer.get", "--file", "a.ylp", "--layer", &base]);
    assert_eq!(layer["effects"][0]["kind"], "blur");
    assert_eq!(layer["effects"][0]["values"]["radius"], 3);
    let wash = fx.ok(&["layer.get", "--file", "a.ylp", "--layer", "Wash"]);
    assert_eq!(wash["opacity"], 0.5);
    assert_eq!(wash["channels"][0]["fill"], "#336699");
    assert!(fx.ok(&["layer.get", "--file", "a.ylp", "--layer", "Tint"])["mask"].is_object());
    assert!(path.is_file());
}

/// 0.5.0 のフィルター・ジェネレーターが一覧に出て、フラグで値つきで足し、保存して別の起動で読める。
#[test]
fn the_new_filters_and_generators_are_listed_and_added_through_flags() {
    let fx = Fixture::new("new-kinds");
    fx.project("a.ylp");
    let kinds = fx.ok(&["effect.list_kinds", "--file", "a.ylp"]);
    let ids: Vec<&str> = kinds["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["id"].as_str().unwrap())
        .collect();
    for id in [
        "slope_blur",
        "morphology",
        "glow",
        "pattern",
        "light",
        "mask_builder",
    ] {
        assert!(ids.contains(&id), "{id}: {ids:?}");
    }
    let out = fx.cli(&[
        "effect.add",
        "--file",
        "a.ylp",
        "--layer",
        "Base",
        "--kind",
        "glow",
        "--values.radius",
        "12",
        "--channels",
        "Color",
        "--save",
    ]);
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
    let out = fx.cli(&[
        "effect.add",
        "--file",
        "a.ylp",
        "--layer",
        "Base",
        "--kind",
        "pattern",
        "--values.shape",
        "checker",
        "--save",
    ]);
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
    let layer = fx.ok(&["layer.get", "--file", "a.ylp", "--layer", "Base"]);
    let effects = layer["effects"].as_array().unwrap();
    assert!(effects
        .iter()
        .any(|e| e["kind"] == "glow" && e["values"]["radius"] == 12));
    assert!(effects
        .iter()
        .any(|e| e["kind"] == "pattern" && e["values"]["shape"] == "checker"));
}

#[test]
fn a_failing_batch_stops_names_the_command_and_saves_nothing() {
    let fx = Fixture::new("batch-fail");
    let path = fx.project("a.ylp");
    let before = file_bytes(&path);
    let lines = format!(
        "{}\n{}\n{}\n",
        json!({"command": "layer.set", "args": {"layer": "Base", "opacity": 0.1}}),
        json!({"command": "layer.get", "args": {"layer": "NoSuchLayer"}}),
        json!({"command": "layer.set", "args": {"layer": "Base", "opacity": 0.9}}),
    );
    let out = fx.cli_in(&["batch", "-", "--file", "a.ylp", "--save"], &lines);
    assert_eq!(out.code, 1);
    let v: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "not_found");
    assert_eq!(v["error"]["data"]["index"], 1);
    assert_eq!(v["error"]["data"]["command"], "layer.get");
    assert_eq!(v["error"]["data"]["completed"], 1);
    assert_eq!(file_bytes(&path), before, "失敗したまとめは保存しない");
    // 読めない入力・空の入力は、実行の前に断る
    assert_eq!(
        fx.cli_in(&["batch", "-", "--file", "a.ylp"], "{ not json")
            .code,
        2
    );
    assert_eq!(
        fx.cli_in(&["batch", "-", "--file", "a.ylp"], "# コメントだけ\n")
            .code,
        2
    );
    let bad = fx.cli_in(
        &["batch", "-", "--file", "a.ylp"],
        &json!([{"command": "nope"}]).to_string(),
    );
    assert_eq!(bad.code, 2);
}

#[test]
fn json_arguments_come_from_a_file_or_inline() {
    let fx = Fixture::new("json-args");
    fx.project("a.ylp");
    std::fs::write(fx.path("args.json"), r#"{"layer":"Base"}"#).unwrap();
    assert_eq!(
        fx.ok(&["layer.get", "@args.json", "--file", "a.ylp"])["name"],
        "Base"
    );
    assert_eq!(
        fx.ok(&["layer.get", r#"{"layer":"Tint"}"#, "--file", "a.ylp"])["name"],
        "Tint"
    );
    assert_eq!(
        fx.cli_in(
            &["layer.get", "-", "--file", "a.ylp"],
            r#"{"layer":"Inner"}"#
        )
        .code,
        0
    );
    let (code, _) = fx.fails(&["layer.get", "@missing.json", "--file", "a.ylp"]);
    assert_eq!(code, 2);
}

#[test]
fn preview_embeds_the_png_or_writes_it_with_out() {
    let fx = Fixture::new("preview");
    fx.project("a.ylp");
    let inline = fx.ok(&["preview", "--file", "a.ylp", "--max-edge", "32"]);
    assert_eq!(inline["reply"], "preview");
    assert_eq!(inline["width"], 32);
    let png = base64_decode(inline["png"].as_str().unwrap()).unwrap();
    assert!(png.starts_with(PNG_MAGIC));

    let out = fx.ok(&[
        "preview",
        "--file",
        "a.ylp",
        "--max-edge",
        "32",
        "--out",
        "shot.png",
    ]);
    assert!(out.get("png").is_none());
    let written = file_bytes(&fx.path("shot.png"));
    assert_eq!(written, png, "--out の PNG は埋め込みと同じ画素");
    assert_eq!(out["png_file"], fx.path("shot.png").display().to_string());
    // 道具のほかの命令に --out は付けない
    let (code, _) = fx.fails(&["doc.info", "--file", "a.ylp", "--out", "x.png"]);
    assert_eq!(code, 2);
    // 画像の大きさの範囲は yolu-ops が断る
    let (code, error) = fx.fails(&["preview", "--file", "a.ylp", "--max-edge", "0"]);
    assert_eq!(
        (code, error["code"].as_str().unwrap()),
        (1, "invalid_value")
    );
}

#[test]
fn export_commands_write_files_and_ask_before_replacing() {
    let fx = Fixture::new("export");
    fx.project("a.ylp");
    let first = fx.ok(&[
        "export.channels",
        "--file",
        "a.ylp",
        "--dir",
        "out",
        "--channels",
        "Color",
    ]);
    assert_eq!(first["reply"], "exported");
    let file = first["files"][0]["path"].as_str().unwrap().to_owned();
    assert!(std::path::Path::new(&file).is_file());
    let (code, error) = fx.fails(&[
        "export.channels",
        "--file",
        "a.ylp",
        "--dir",
        "out",
        "--channels",
        "Color",
    ]);
    assert_eq!(
        (code, error["code"].as_str().unwrap()),
        (4, "confirm_required")
    );
    fx.ok(&[
        "export.channels",
        "--file",
        "a.ylp",
        "--dir",
        "out",
        "--channels",
        "Color",
        "--confirm",
    ]);
    let psd = fx.ok(&[
        "export.psd",
        "--file",
        "a.ylp",
        "--path",
        "work.psd",
        "--mode",
        "flat",
    ]);
    assert_eq!(psd["reply"], "exported");
    assert!(fx.path("work.psd").is_file());
    // 作業のフォルダの外へ出る道は断る
    let (code, error) = fx.fails(&["export.channels", "--file", "a.ylp", "--dir", "../outside"]);
    assert_eq!((code, error["code"].as_str().unwrap()), (1, "path_refused"));
}

#[test]
fn the_commands_listing_matches_the_library() {
    let fx = Fixture::new("commands");
    let listing = fx.ok(&["commands"]);
    let names: Vec<&str> = listing["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    let expected: Vec<&str> = commands().iter().map(|c| c.name).collect();
    assert_eq!(names, expected);
    let kind = |name: &str| {
        listing["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap()["kind"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(kind("layer.get"), "read");
    assert_eq!(kind("layer.set"), "edit");
    assert_eq!(kind("layer.delete"), "destructive");
    assert_eq!(kind("export.psd"), "replace");
    assert_eq!(kind("save"), "destructive");
    for c in listing["commands"].as_array().unwrap() {
        assert!(c["title"]["ja"].is_string() && c["title"]["en"].is_string());
    }
}

#[test]
fn schema_output_is_the_type_derived_schema() {
    let fx = Fixture::new("schema");
    let all = fx.ok(&["schema"]);
    assert_eq!(all["command"], command_schema());
    assert_eq!(all["reply"], reply_schema());
    assert_eq!(all["error"], error_schema());
    assert_eq!(all["version"], yolu_ops::COMMAND_VERSION);
    for spec in commands() {
        let one = fx.ok(&["schema", spec.name]);
        assert_eq!(one["args"], spec.args_schema(), "{}", spec.name);
        assert_eq!(one["reply"], spec.reply_schema(), "{}", spec.name);
        assert_eq!(fx.ok(&["schema", &spec.tool_name()])["name"], spec.name);
    }
    assert_eq!(
        fx.ok(&["schema", "--tools"])["tools"],
        Value::Array(tools::tools())
    );
    let (code, error) = fx.fails(&["schema", "no.such"]);
    assert_eq!(
        (code, error["code"].as_str().unwrap()),
        (2, "unknown_command")
    );
}

/// schema の `type` に合う、最小の値（必須の欄を満たすための見本）。
fn dummy(root: &Value, node: &Value) -> Value {
    let node = match node
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix('#'))
        .and_then(|p| root.pointer(p))
    {
        Some(target) => target,
        None => node,
    };
    if let Some(value) = node.get("const") {
        return value.clone();
    }
    if let Some(first) = node
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|e| e.first())
    {
        return first.clone();
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(members) = node.get(key).and_then(Value::as_array) {
            if let Some(m) = members
                .iter()
                .find(|m| m.get("type").and_then(Value::as_str) != Some("null"))
            {
                return dummy(root, m);
            }
        }
    }
    let ty = match node.get("type") {
        Some(Value::String(t)) => t.as_str(),
        Some(Value::Array(ts)) => ts
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .unwrap_or("null"),
        _ => "object",
    };
    match ty {
        "string" => json!("x"),
        "boolean" => json!(true),
        "integer" => json!(1),
        "number" => json!(0.5),
        "array" => json!([]),
        "null" => Value::Null,
        _ => {
            let mut map = serde_json::Map::new();
            for r in node
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                map.insert(r.to_owned(), dummy(root, &node["properties"][r]));
            }
            Value::Object(map)
        }
    }
}

#[test]
fn schemas_agree_with_the_types_on_required_and_unknown_fields() {
    for spec in commands() {
        let schema = spec.args_schema();
        let args = dummy(&schema, &schema);
        // 必須の欄だけを満たす引数は、型に通る
        let ok = parse_command(&json!({"command": spec.name, "args": args}));
        assert!(
            ok.is_ok(),
            "{}: {args} が通らない: {:?}",
            spec.name,
            ok.err()
        );
        // 必須の欄を 1 つ欠くと断る
        for required in schema["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let mut missing = args.clone();
            missing.as_object_mut().unwrap().remove(required);
            let e = parse_command(&json!({"command": spec.name, "args": missing})).unwrap_err();
            assert_eq!(
                e.code,
                ErrorCode::InvalidRequest,
                "{} の {required}",
                spec.name
            );
        }
        // schema が追加の欄を許さないなら、型も許さない（綴りの間違いを黙って流さない）
        assert_eq!(schema["additionalProperties"], false, "{}", spec.name);
        let mut extra = args.clone();
        extra
            .as_object_mut()
            .unwrap()
            .insert("typo_field".into(), json!(1));
        assert!(
            parse_command(&json!({"command": spec.name, "args": extra})).is_err(),
            "{}",
            spec.name
        );
    }
}

#[test]
fn errors_have_exit_codes_and_both_languages() {
    let fx = Fixture::new("errors");
    fx.project("a.ylp");
    let (code, error) = fx.fails(&["layer.get", "--file", "a.ylp", "--layer", "Nope"]);
    assert_eq!((code, error["code"].as_str().unwrap()), (1, "not_found"));
    let (code, error) = fx.fails(&["no.such.command"]);
    assert_eq!(
        (code, error["code"].as_str().unwrap()),
        (2, "unknown_command")
    );
    let (code, error) = fx.fails(&[
        "layer.set",
        "--file",
        "a.ylp",
        "--layer",
        "Base",
        "--opacity",
        "2",
    ]);
    assert_eq!(
        (code, error["code"].as_str().unwrap()),
        (1, "invalid_value")
    );
    let (code, _) = fx.fails(&["doc.info", "--file", "missing.ylp"]);
    assert_eq!(code, 1);
    // 言語: 分からなければ日英を並べ、選べば 1 つ
    let both = fx.cli(&["layer.get", "--file", "a.ylp", "--layer", "Nope"]);
    assert!(
        both.stderr.contains("見つかりません") && both.stderr.contains("No layer"),
        "{}",
        both.stderr
    );
    let ja = fx.cli(&[
        "layer.get",
        "--file",
        "a.ylp",
        "--layer",
        "Nope",
        "--lang",
        "ja",
    ]);
    assert!(
        ja.stderr.contains("見つかりません") && !ja.stderr.contains("No layer"),
        "{}",
        ja.stderr
    );
    let en = fx.cli(&[
        "layer.get",
        "--file",
        "a.ylp",
        "--layer",
        "Nope",
        "--lang=en",
    ]);
    assert!(
        en.stderr.contains("No layer") && !en.stderr.contains("見つかりません"),
        "{}",
        en.stderr
    );
    // 引数の読みの失敗でも、言語を守る
    let usage = fx.cli(&["--lang", "en", "layer.set", "--bogus", "1"]);
    assert_eq!(usage.code, 2);
    assert!(
        usage.stderr.contains("no --bogus argument") && !usage.stderr.contains("という引数"),
        "{}",
        usage.stderr
    );
    assert_eq!(usage_stdout_code(&usage.stdout), "invalid_request");
}

fn usage_stdout_code(stdout: &str) -> String {
    serde_json::from_str::<Value>(stdout).unwrap()["error"]["code"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn help_version_and_the_environment_language() {
    let fx = Fixture::new("help");
    let help = fx.cli(&["--help"]);
    assert_eq!(help.code, 0);
    assert!(help.stdout.contains("yolupainter-cli") && help.stdout.contains("--file"));
    let ja = fx.cli(&["--help", "--lang", "ja"]);
    assert!(ja.stdout.contains("使い方"));
    assert_eq!(fx.cli(&[]).stdout, help.stdout, "引数なしは使い方");
    let version = fx.cli(&["--version"]);
    assert!(version
        .stdout
        .starts_with(&format!("yolupainter-cli {}", env!("CARGO_PKG_VERSION"))));
    use yolu_cli::cli::lang_from_vars;
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |k: &str| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        }
    };
    assert_eq!(
        lang_from_vars(env(&[("LANG", "ja_JP.UTF-8")])),
        Some(yolu_ops::Lang::Ja)
    );
    assert_eq!(
        lang_from_vars(env(&[("LC_ALL", "en_US.UTF-8"), ("LANG", "ja_JP.UTF-8")])),
        Some(yolu_ops::Lang::En)
    );
    assert_eq!(lang_from_vars(env(&[("LANG", "C.UTF-8")])), None);
    assert_eq!(
        lang_from_vars(env(&[("YOLUPAINTER_LANG", "ja"), ("LANG", "en_US")])),
        Some(yolu_ops::Lang::Ja)
    );
    assert_eq!(lang_from_vars(env(&[])), None);
}

#[test]
fn the_real_binary_runs_a_command_and_reports_the_exit_code() {
    let fx = Fixture::new("binary");
    fx.project("a.ylp");
    let exe = env!("CARGO_BIN_EXE_yolupainter-cli");
    let out = std::process::Command::new(exe)
        .args([
            "layer.get",
            "--file",
            "a.ylp",
            "--layer",
            "Base",
            "--lang",
            "en",
        ])
        .current_dir(&fx.dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["name"], "Base");
    let out = std::process::Command::new(exe)
        .args(["layer.delete", "--file", "a.ylp", "--layer", "Base"])
        .current_dir(&fx.dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&out.stderr).contains("confirm_required"));
    // 標準入力の JSON
    let mut child = std::process::Command::new(exe)
        .args(["layer.get", "-", "--file", "a.ylp"])
        .current_dir(&fx.dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"layer":"Tint"}"#)
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["name"],
        "Tint"
    );
}

#[test]
fn cwd_changes_the_start_of_relative_paths() {
    let fx = Fixture::new("cwd");
    std::fs::create_dir_all(fx.path("work")).unwrap();
    fx.project("work/a.ylp");
    // 今のフォルダには a.ylp が無いが、--cwd work なら見つかり、書き出し先の相対パスも work の下になる
    assert_eq!(fx.fails(&["doc.info", "--file", "a.ylp"]).0, 1);
    assert_eq!(
        fx.ok(&["doc.info", "--file", "a.ylp", "--cwd", "work"])["reply"],
        "doc"
    );
    fx.ok(&[
        "export.channels",
        "--file",
        "a.ylp",
        "--cwd",
        "work",
        "--dir",
        "out",
        "--channels",
        "Color",
    ]);
    assert!(fx.path("work/out").is_dir() && !fx.path("out").exists());
    let (code, _) = fx.fails(&["doc.info", "--file", "a.ylp", "--cwd", "nowhere"]);
    assert_eq!(code, 2);
}

#[test]
fn mcp_refuses_the_options_that_choose_a_file() {
    let fx = Fixture::new("mcp-options");
    for args in [["mcp", "--file", "a.ylp"], ["mcp", "--out", "x.png"]] {
        let (code, error) = fx.fails(&args);
        assert_eq!(
            (code, error["code"].as_str().unwrap()),
            (2, "invalid_request"),
            "{args:?}"
        );
    }
    assert_eq!(fx.fails(&["mcp", "--save"]).0, 2);
}

#[test]
fn an_unusable_port_is_refused_by_mcp_as_by_the_commands() {
    // 1024 より下・番号でない物は、つなぐ前に断る（「アプリが待ち受けていない」という誤った直し方にしない）
    let fx = Fixture::new("port");
    for port in ["0", "80", "1023", "65536", "x", ""] {
        for args in [
            vec!["mcp", "--port", port],
            vec!["doc.info", "--port", port],
        ] {
            let (code, error) = fx.fails(&args);
            assert_eq!(
                (code, error["code"].as_str().unwrap()),
                (2, "invalid_request"),
                "{args:?}"
            );
        }
    }
}
