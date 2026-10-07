//! MCP サーバーの試験。本物の HTTP の受け口（127.0.0.1 の空いた番号）を、画面なしのホスト（起動中のアプリの代わり）を相手に立て、
//! 本物の HTTP の客で話す。2025-11-25 の `initialize` の流れと、2026-07-28 の `server/discover`・要求ごとの `_meta` の流れの両方、
//! ツールの一覧・呼び出し・画像の返事・資料の一覧と読み・誤りの形・structuredContent が出力の schema に合うことを確かめる。

mod common;

use common::*;
use serde_json::{json, Value};
use yolu_ops::value::base64_decode;
use yolu_ops::{command_spec_by_tool, commands};

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

fn served(fx: &Fixture) -> Served {
    fx.project("a.ylp");
    Served::start(fx.host("a.ylp"))
}

#[test]
fn the_initialize_flow_negotiates_2025_11_25_and_lists_the_tools() {
    let fx = Fixture::new("init");
    let server = served(&fx);
    let (mut mcp, init) = Mcp::legacy(server.port);
    assert_eq!(init["protocolVersion"], "2025-11-25");
    assert!(
        init["capabilities"]["tools"].is_object() && init["capabilities"]["resources"].is_object()
    );
    assert_eq!(init["serverInfo"]["name"], "yolupainter");
    assert!(init["instructions"]
        .as_str()
        .unwrap()
        .contains("confirm: true"));

    let list = mcp.result("tools/list", json!({}));
    let tools = list["tools"].as_array().unwrap();
    assert_eq!(tools.len(), commands().len(), "命令は全部ツール");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for spec in commands() {
        assert!(
            names.contains(&spec.tool_name().as_str()),
            "{} がツールに無い",
            spec.name
        );
    }
    // doc_open は、アプリが文書を開き替えないことが説明に入る（壊す印つき）
    let doc_open = tools.iter().find(|t| t["name"] == "doc_open").unwrap();
    assert!(doc_open["description"]
        .as_str()
        .unwrap()
        .contains("does not switch documents"));
    assert_eq!(doc_open["annotations"]["destructiveHint"], true);
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert!(
            tool["title"].as_str().unwrap().contains(" / "),
            "{name}: 題は英語と日本語"
        );
        assert!(tool["description"].as_str().unwrap().len() > 10, "{name}");
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
        assert!(tool["inputSchema"]["properties"].is_object(), "{name}");
        assert!(
            tool["inputSchema"]["properties"].get("file").is_none(),
            "{name}: ファイルを直に開く引数は無い"
        );
        assert_eq!(tool["outputSchema"]["type"], "object", "{name}");
        let a = &tool["annotations"];
        assert!(
            a["readOnlyHint"].is_boolean()
                && a["idempotentHint"].is_boolean()
                && a["openWorldHint"] == false,
            "{name}: {a}"
        );
    }
    let get = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap();
    assert_eq!(get("layer_get")["annotations"]["readOnlyHint"], true);
    assert_eq!(get("layer_delete")["annotations"]["destructiveHint"], true);
    assert_eq!(get("layer_set")["annotations"]["destructiveHint"], false);
    assert_eq!(get("save")["annotations"]["destructiveHint"], true);
    // 一覧は相手（アプリ）に聞かない
    assert_eq!(server.ran(), 0);
}

#[test]
fn the_stateless_2026_07_28_flow_discovers_lists_and_calls_without_initialize() {
    let fx = Fixture::new("2026");
    let server = served(&fx);
    let mut mcp = Mcp::stateless(server.port);
    let discover = mcp.result("server/discover", json!({}));
    let versions: Vec<&str> = discover["supportedVersions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        versions.contains(&"2026-07-28") && versions.contains(&"2025-11-25"),
        "{versions:?}"
    );
    assert!(discover["capabilities"]["tools"].is_object());
    assert_eq!(
        discover["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "yolupainter"
    );
    let list = mcp.result("tools/list", json!({}));
    assert_eq!(list["resultType"], "complete");
    assert!(
        list["ttlMs"].is_u64() && list["cacheScope"].is_string(),
        "一覧に保持の手がかり: {list}"
    );
    assert_eq!(list["tools"].as_array().unwrap().len(), commands().len());
    let called = mcp.call("doc_info", json!({}));
    assert_eq!(called["resultType"], "complete");
    assert_eq!(called["isError"], false);
    assert_eq!(called["structuredContent"]["sets"][0]["name"], "Body");
    // `_meta` の無い 2026 の要求は断る
    let mut bare = Mcp::new(server.port);
    bare.version = Some("2026-07-28".into());
    let response = bare.request("server/discover", json!({}));
    assert!(response["error"].is_object(), "{response}");
    // 資料も同じ流れで読める
    let read = mcp.result("resources/read", json!({"uri": "yolupainter://docs/guide"}));
    assert!(read["contents"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("# "));
}

#[test]
fn tools_edit_with_undo_confirmation_and_errors_in_both_languages() {
    let fx = Fixture::new("edit");
    let server = served(&fx);
    let (mut mcp, _) = Mcp::legacy(server.port);

    let info = mcp.call("doc_info", json!({}));
    assert_eq!(info["isError"], false);
    assert_eq!(info["structuredContent"]["sets"][0]["name"], "Body");
    assert_eq!(
        text_json(&info),
        info["structuredContent"],
        "text は同じ JSON"
    );
    let edited = mcp.call("layer_set", json!({"layer": "Base", "opacity": 0.4}));
    assert_eq!(edited["isError"], false);
    assert_eq!(edited["structuredContent"]["can_undo"], true);
    let history = mcp.call("history_info", json!({}));
    assert_eq!(history["structuredContent"]["undo_count"], 1);

    // 壊す操作は、確認が無ければ断る（理由つき）
    let refused = mcp.call("layer_delete", json!({"layer": "Tint"}));
    assert_eq!(refused["isError"], true);
    assert!(refused.get("structuredContent").is_none());
    let error = text_json(&refused);
    assert_eq!(error["code"], "confirm_required");
    assert!(error["message"]["ja"].is_string() && error["message"]["en"].is_string());
    let deleted = mcp.call("layer_delete", json!({"layer": "Tint", "confirm": true}));
    assert_eq!(deleted["isError"], false);
    assert_eq!(mcp.call("undo", json!({}))["structuredContent"]["steps"], 1);
    let refused = mcp.call("save", json!({}));
    assert_eq!(text_json(&refused)["code"], "confirm_required");
    let saved = mcp.call("save", json!({"confirm": true}));
    assert_eq!(saved["structuredContent"]["written"], true);

    // 誤りは、ツールの失敗（isError）で、理由が日英で読める
    let missing = mcp.call("layer_get", json!({"layer": "Nope"}));
    assert_eq!(
        (
            missing["isError"].clone(),
            text_json(&missing)["code"].clone()
        ),
        (json!(true), json!("not_found"))
    );
    let typo = mcp.call("layer_get", json!({"layer": "Base", "bogus": 1}));
    assert_eq!(
        (typo["isError"].clone(), text_json(&typo)["code"].clone()),
        (json!(true), json!("invalid_request"))
    );
    // ファイルを直に開く引数は無い（知らない欄として断る）
    let file = mcp.call("doc_info", json!({"file": "a.ylp"}));
    assert_eq!(text_json(&file)["code"], "invalid_request");
    let outside = mcp.call("export_channels", json!({"dir": "../outside"}));
    assert_eq!(text_json(&outside)["code"], "path_refused");
    // 知らないツールは、JSON-RPC の誤り
    let unknown = mcp.request(
        "tools/call",
        json!({"name": "no_such_tool", "arguments": {}}),
    );
    assert!(unknown["error"].is_object());
}

#[test]
fn preview_returns_an_image_and_a_link_to_the_same_png() {
    let fx = Fixture::new("preview");
    let server = served(&fx);
    let (mut mcp, _) = Mcp::legacy(server.port);
    let result = mcp.call("preview", json!({"max_edge": 24}));
    assert_eq!(result["isError"], false);
    let content = result["content"].as_array().unwrap();
    let types: Vec<&str> = content
        .iter()
        .map(|c| c["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, ["text", "image", "resource_link"]);
    let image = &content[1];
    assert_eq!(image["mimeType"], "image/png");
    let png = base64_decode(image["data"].as_str().unwrap()).unwrap();
    assert!(png.starts_with(PNG_MAGIC));
    let link = &content[2];
    assert_eq!(link["mimeType"], "image/png");
    assert_eq!(link["size"], png.len());
    let uri = link["uri"].as_str().unwrap().to_owned();
    assert!(uri.starts_with("yolupainter://preview/") && uri.ends_with(".png"));
    // 構造化した結果には、画像のバイトを入れない（outputSchema と合う）
    let structured = &result["structuredContent"];
    assert_eq!(structured["width"], 24);
    assert!(structured.get("png").is_none());
    assert!(
        !content[0]["text"].as_str().unwrap().contains("\"png\""),
        "text にも画像のバイトを入れない"
    );
    // リンクを読むと同じ PNG（要求ごとに別の HTTP のつながりでも、受け口が覚えている）
    let read = mcp.result("resources/read", json!({"uri": uri}));
    assert_eq!(read["contents"][0]["mimeType"], "image/png");
    assert_eq!(
        base64_decode(read["contents"][0]["blob"].as_str().unwrap()).unwrap(),
        png
    );
    let listed = mcp.result("resources/list", json!({}));
    assert!(listed["resources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["uri"] == uri.as_str()));
    let gone = mcp.request(
        "resources/read",
        json!({"uri": "yolupainter://preview/9999.png"}),
    );
    assert!(gone["error"].is_object());
    // 客の側で、結果を命令の返事に戻せる（コマンドラインが使う）
    let spec = command_spec_by_tool("preview").unwrap();
    let reply = yolu_mcp::client::reply_of(spec, &result).unwrap();
    let yolu_ops::Reply::Preview(info) = reply else {
        panic!("preview の返事ではない")
    };
    assert_eq!(info.png.0, png);
}

#[test]
fn resources_serve_the_bundled_docs_the_command_list_and_the_effect_kinds() {
    let fx = Fixture::new("resources");
    let server = served(&fx);
    let (mut mcp, _) = Mcp::legacy(server.port);
    let listed = mcp.result("resources/list", json!({}));
    let resources = listed["resources"].as_array().unwrap();
    let uris: Vec<&str> = resources
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    for expected in [
        "yolupainter://docs/guide",
        "yolupainter://docs/guide.ja",
        "yolupainter://docs/cli",
        "yolupainter://docs/mcp",
        "yolupainter://docs/psd",
        "yolupainter://ops/commands",
        "yolupainter://ops/effect-kinds",
    ] {
        assert!(
            uris.contains(&expected),
            "{expected} が資料に無い: {uris:?}"
        );
    }
    for r in resources {
        assert!(r["name"].is_string() && r["mimeType"].is_string(), "{r}");
    }
    let guide = mcp.result("resources/read", json!({"uri": "yolupainter://docs/guide"}));
    assert_eq!(guide["contents"][0]["mimeType"], "text/markdown");
    assert_eq!(
        guide["contents"][0]["text"].as_str().unwrap(),
        include_str!("../../../docs/en/GUIDE.md"),
        "入れてある版の文書そのまま"
    );
    let ja = mcp.result(
        "resources/read",
        json!({"uri": "yolupainter://docs/guide.ja"}),
    );
    assert_eq!(
        ja["contents"][0]["text"].as_str().unwrap(),
        include_str!("../../../docs/GUIDE.md")
    );
    let kinds = mcp.result(
        "resources/read",
        json!({"uri": "yolupainter://ops/effect-kinds"}),
    );
    let kinds: Value =
        serde_json::from_str(kinds["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert!(kinds["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .any(|k| k["id"] == "blur" && k["params"].is_array()));
    let listing = mcp.result(
        "resources/read",
        json!({"uri": "yolupainter://ops/commands"}),
    );
    let listing: Value =
        serde_json::from_str(listing["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        listing["commands"].as_array().unwrap().len(),
        commands().len()
    );
    assert!(listing["commands"][0]["args_schema"].is_object());
    for bad in [
        "yolupainter://docs/none",
        "yolupainter://docs/guide.fr",
        "yolupainter://docs/../guide",
        "yolupainter://other",
        "file:///etc/passwd",
    ] {
        assert!(
            mcp.request("resources/read", json!({"uri": bad}))["error"].is_object(),
            "{bad}"
        );
    }
    assert_eq!(server.ran(), 0, "資料はアプリに聞かない");
}

// ───────── structuredContent は outputSchema に合う ─────────

fn violations(root: &Value, schema: &Value, value: &Value, path: &str, out: &mut Vec<String>) {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let target = reference
            .strip_prefix('#')
            .and_then(|p| root.pointer(p))
            .unwrap_or_else(|| panic!("{reference} を引けない"));
        return violations(root, target, value, path, out);
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(members) = schema.get(key).and_then(Value::as_array) {
            let matched = members.iter().any(|m| {
                let mut inner = Vec::new();
                violations(root, m, value, path, &mut inner);
                inner.is_empty()
            });
            if !matched {
                out.push(format!("{path}: {key} のどれにも合わない: {value}"));
            }
            return;
        }
    }
    if let Some(members) = schema.get("allOf").and_then(Value::as_array) {
        for m in members {
            violations(root, m, value, path, out);
        }
    }
    if let Some(expected) = schema.get("const") {
        if expected != value {
            out.push(format!("{path}: {expected} のはずが {value}"));
        }
        return;
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        if !options.contains(value) {
            out.push(format!("{path}: 選択肢に無い {value}"));
        }
    }
    if let Some(ty) = schema.get("type") {
        let types: Vec<&str> = match ty {
            Value::String(t) => vec![t.as_str()],
            Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        let fits = |t: &&str| match *t {
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            "number" => value.is_number(),
            "integer" => {
                value.is_i64() || value.is_u64() || value.as_f64().is_some_and(|f| f.fract() == 0.0)
            }
            "array" => value.is_array(),
            "object" => value.is_object(),
            _ => true,
        };
        if !types.iter().any(fits) {
            out.push(format!("{path}: 型 {types:?} に合わない {value}"));
            return;
        }
    }
    if let Some(map) = value.as_object() {
        for r in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !map.contains_key(r) {
                out.push(format!("{path}: 必須の {r} が無い"));
            }
        }
        let props = schema.get("properties").and_then(Value::as_object);
        for (k, v) in map {
            match props.and_then(|p| p.get(k)) {
                Some(s) => violations(root, s, v, &format!("{path}.{k}"), out),
                None => match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => out.push(format!("{path}: 知らない欄 {k}")),
                    Some(extra) if extra.is_object() => {
                        violations(root, extra, v, &format!("{path}.{k}"), out)
                    }
                    _ => {}
                },
            }
        }
    }
    if let (Some(items), Some(list)) = (schema.get("items"), value.as_array()) {
        for (i, v) in list.iter().enumerate() {
            violations(root, items, v, &format!("{path}[{i}]"), out);
        }
    }
}

#[test]
fn structured_results_conform_to_the_output_schemas() {
    let fx = Fixture::new("schema");
    let server = served(&fx);
    let (mut mcp, _) = Mcp::legacy(server.port);
    let list = mcp.result("tools/list", json!({}));
    let schemas: std::collections::BTreeMap<String, Value> = list["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_owned(),
                t["outputSchema"].clone(),
            )
        })
        .collect();
    let path = fx.path("a.ylp").display().to_string();
    let steps: Vec<(&str, Value)> = vec![
        ("doc_info", json!({})),
        ("doc_open", json!({"path": path, "confirm": true})),
        ("set_info", json!({})),
        ("layer_get", json!({"layer": "Base"})),
        (
            "layer_add",
            json!({"kind": "fill", "name": "Wash", "fill": {"Color": "#336699"}}),
        ),
        (
            "layer_add",
            json!({"kind": "adjustment", "name": "Inv", "adjustment": {"kind": "invert"}}),
        ),
        ("layer_get", json!({"layer": "Inv"})),
        (
            "layer_set",
            json!({"layer": "Wash", "opacity": 0.5, "locks": {"pixels": true}}),
        ),
        ("layer_set", json!({"layer": "Wash", "opacity": 0.5})),
        ("layer_move", json!({"layer": "Wash", "index": 0})),
        ("mask_add", json!({"layer": "Tint"})),
        ("mask_set", json!({"layer": "Tint", "density": 0.5})),
        (
            "effect_add",
            json!({"layer": "Base", "kind": "blur", "values": {"radius": 2}}),
        ),
        ("effect_get", json!({"layer": "Base"})),
        ("layer_get", json!({"layer": "Base"})),
        ("layer_get", json!({"layer": "Tint"})),
        ("effect_list_kinds", json!({})),
        ("history_info", json!({})),
        ("undo", json!({})),
        ("redo", json!({})),
        ("preview", json!({"max_edge": 16})),
        (
            "export_channels",
            json!({"dir": "out", "channels": ["Color"], "confirm": true}),
        ),
        ("export_textures", json!({"dir": "tex", "confirm": true})),
        (
            "export_psd",
            json!({"path": "x.psd", "mode": "flat", "confirm": true}),
        ),
        ("save", json!({"confirm": true})),
        ("save_as", json!({"path": "copy.ylp", "confirm": true})),
        ("mask_delete", json!({"layer": "Tint", "confirm": true})),
        ("layer_delete", json!({"layer": "Wash", "confirm": true})),
    ];
    let mut seen = std::collections::BTreeSet::new();
    for (tool, args) in steps {
        let result = mcp.call(tool, args.clone());
        assert_eq!(result["isError"], false, "{tool} {args}: {result}");
        let schema = &schemas[tool];
        let mut problems = Vec::new();
        violations(
            schema,
            schema,
            &result["structuredContent"],
            "$",
            &mut problems,
        );
        assert!(
            problems.is_empty(),
            "{tool} の structuredContent が outputSchema に合わない: {problems:#?}\n{}",
            result["structuredContent"]
        );
        // 客は、どの結果も命令の返事に戻せる
        let spec = command_spec_by_tool(tool).unwrap();
        yolu_mcp::client::reply_of(spec, &result)
            .unwrap_or_else(|e| panic!("{tool} の結果を返事に戻せない: {e:?}"));
        seen.insert(tool);
    }
    assert!(seen.len() >= 20, "{seen:?}");
}

#[test]
fn the_validator_used_here_does_catch_a_wrong_shape() {
    let schema = json!({"type": "object", "required": ["a"], "properties": {"a": {"type": "integer"}}, "additionalProperties": false});
    for bad in [
        json!({}),
        json!({"a": "x"}),
        json!({"a": 1, "b": 2}),
        json!([1]),
    ] {
        let mut out = Vec::new();
        violations(&schema, &schema, &bad, "$", &mut out);
        assert!(!out.is_empty(), "{bad}");
    }
    let mut out = Vec::new();
    violations(&schema, &schema, &json!({"a": 3}), "$", &mut out);
    assert!(out.is_empty(), "{out:?}");
}
