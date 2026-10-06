//! MCP サーバーの試験。本物の `yolupainter-cli mcp` を起こし、標準入出力（1 行 1 メッセージの JSON-RPC）で話す。
//! 2025-11-25 の `initialize` の流れと、2026-07-28 の `server/discover`・要求ごとの `_meta` の流れの両方、ツールの一覧・呼び出し・画像の返事・
//! 資料の一覧と読み、`file` の組、起動中のアプリ（代わりの待ち受け）への道を確かめる。

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use common::*;
use serde_json::{json, Value};
use yolu_ops::value::base64_decode;
use yolu_ops::{commands, parse_command_str};

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Value>,
    next: u64,
    /// 2026-07-28 の流れ（要求ごとの `_meta`）か。
    stateless: bool,
}

impl Mcp {
    fn start(fx: &Fixture, link_name: &str) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_yolupainter-cli"))
            .arg("mcp")
            .current_dir(&fx.dir)
            .env(yolu_ops::link::LINK_NAME_ENV, link_name)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let value: Value = serde_json::from_str(&line)
                    .unwrap_or_else(|e| panic!("標準出力に JSON でない行: {e}: {line}"));
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        Mcp {
            child,
            stdin: Some(stdin),
            rx,
            next: 0,
            stateless: false,
        }
    }

    /// 2025-11-25 の `initialize` で始める。
    fn legacy(fx: &Fixture, link_name: &str) -> (Mcp, Value) {
        let mut mcp = Mcp::start(fx, link_name);
        let init = mcp.result(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test-client", "version": "1"}}),
        );
        mcp.notify("notifications/initialized", json!({}));
        (mcp, init)
    }

    fn send(&mut self, value: Value) {
        let stdin = self.stdin.as_mut().expect("標準入力がまだ開いている");
        writeln!(stdin, "{value}").unwrap();
        stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn meta(&self) -> Value {
        json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/clientInfo": {"name": "test-client", "version": "1"},
        })
    }

    /// 要求を送り、返事（result か error を含む JSON-RPC の応答）を返す。
    fn request(&mut self, method: &str, mut params: Value) -> Value {
        if self.stateless {
            params["_meta"] = self.meta();
        }
        self.next += 1;
        let id = self.next;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let message = self
                .rx
                .recv_timeout(Duration::from_secs(60))
                .unwrap_or_else(|e| panic!("{method} の返事が来ない: {e}"));
            if message["id"] == json!(id) {
                return message;
            }
        }
    }

    fn result(&mut self, method: &str, params: Value) -> Value {
        let response = self.request(method, params);
        assert!(response.get("error").is_none(), "{method}: {response}");
        response["result"].clone()
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.result("tools/call", json!({"name": tool, "arguments": arguments}))
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn text_json(result: &Value) -> Value {
    let text = result["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{result}"));
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

fn nobody() -> String {
    unique_link_name("mcp-nobody")
}

#[test]
fn the_initialize_flow_negotiates_2025_11_25_and_lists_the_tools() {
    let fx = Fixture::new("mcp-init");
    let (mut mcp, init) = Mcp::legacy(&fx, &nobody());
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
    // doc_open は、組の保存していない編集を捨てて開き直す道（使い方が説明に入る。壊す印つき）
    let doc_open = tools.iter().find(|t| t["name"] == "doc_open").unwrap();
    assert!(doc_open["description"]
        .as_str()
        .unwrap()
        .contains("same .ylp as `path`"));
    assert_eq!(doc_open["annotations"]["destructiveHint"], true);
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert!(
            tool["title"].as_str().unwrap().contains(" / "),
            "{name}: 題は英語と日本語"
        );
        assert!(tool["description"].as_str().unwrap().len() > 10, "{name}");
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
        assert_eq!(tool["outputSchema"]["type"], "object", "{name}");
        assert!(
            tool["inputSchema"]["properties"]["file"].is_object(),
            "{name}"
        );
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
}

#[test]
fn the_stateless_2026_07_28_flow_discovers_lists_and_calls_without_initialize() {
    let fx = Fixture::new("mcp-2026");
    fx.project("a.ylp");
    let mut mcp = Mcp::start(&fx, &nobody());
    mcp.stateless = true;
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
    assert!(discover["instructions"]
        .as_str()
        .unwrap()
        .contains("confirm: true"));
    let list = mcp.result("tools/list", json!({}));
    assert_eq!(list["resultType"], "complete");
    assert!(
        list["ttlMs"].is_u64() && list["cacheScope"].is_string(),
        "一覧に保持の手がかり: {list}"
    );
    assert_eq!(list["tools"].as_array().unwrap().len(), commands().len());
    let called = mcp.call("doc_info", json!({"file": "a.ylp"}));
    assert_eq!(called["resultType"], "complete");
    assert_eq!(called["isError"], false);
    assert_eq!(called["structuredContent"]["sets"][0]["name"], "Body");
    // `_meta` の無い要求は、2026 の流れでは断る
    let mut bare = Mcp::start(&fx, &nobody());
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
fn tools_edit_a_file_with_undo_confirmation_and_an_explicit_save() {
    let fx = Fixture::new("mcp-edit");
    let path = fx.project("a.ylp");
    let before = file_bytes(&path);
    let (mut mcp, _) = Mcp::legacy(&fx, &nobody());

    let info = mcp.call("doc_info", json!({"file": "a.ylp"}));
    assert_eq!(info["isError"], false);
    assert_eq!(info["structuredContent"]["sets"][0]["name"], "Body");
    assert_eq!(
        text_json(&info),
        info["structuredContent"],
        "text は同じ JSON"
    );

    let edited = mcp.call(
        "layer_set",
        json!({"file": "a.ylp", "layer": "Base", "opacity": 0.4}),
    );
    assert_eq!(edited["isError"], false);
    assert_eq!(edited["structuredContent"]["can_undo"], true);
    // 同じ .ylp への次の呼び出しは、メモリの中の編集を引き継ぐ
    let history = mcp.call("history_info", json!({"file": "a.ylp"}));
    assert_eq!(history["structuredContent"]["undo_count"], 1);
    assert_eq!(
        file_bytes(&path),
        before,
        "保存するまでファイルは変わらない"
    );

    // 壊す操作は、確認が無ければ断る（理由つき）
    let refused = mcp.call("layer_delete", json!({"file": "a.ylp", "layer": "Tint"}));
    assert_eq!(refused["isError"], true);
    assert!(refused.get("structuredContent").is_none());
    let error = text_json(&refused);
    assert_eq!(error["code"], "confirm_required");
    assert!(error["message"]["ja"].is_string() && error["message"]["en"].is_string());
    let deleted = mcp.call(
        "layer_delete",
        json!({"file": "a.ylp", "layer": "Tint", "confirm": true}),
    );
    assert_eq!(deleted["isError"], false);
    assert_eq!(
        mcp.call("undo", json!({"file": "a.ylp"}))["structuredContent"]["steps"],
        1
    );

    // 保存も確認が要り、保存したあとは別の起動から読める
    let refused = mcp.call("save", json!({"file": "a.ylp"}));
    assert_eq!(
        (
            refused["isError"].clone(),
            text_json(&refused)["code"].clone()
        ),
        (json!(true), json!("confirm_required"))
    );
    let saved = mcp.call("save", json!({"file": "a.ylp", "confirm": true}));
    assert_eq!(saved["isError"], false);
    assert_eq!(saved["structuredContent"]["written"], true);
    assert_ne!(file_bytes(&path), before);
    assert_eq!(
        fx.ok(&["layer.get", "--file", "a.ylp", "--layer", "Base"])["opacity"],
        0.4
    );
    // 保存したあとの同じ .ylp は、自分で書いたのを「外の書き換え」と見ず、取り消しの段を残す
    assert_eq!(
        mcp.call("history_info", json!({"file": "a.ylp"}))["structuredContent"]["can_redo"],
        true
    );

    // 誤りは、ツールの失敗（isError）で、理由が日英で読める
    let missing = mcp.call("layer_get", json!({"file": "a.ylp", "layer": "Nope"}));
    assert_eq!(
        (
            missing["isError"].clone(),
            text_json(&missing)["code"].clone()
        ),
        (json!(true), json!("not_found"))
    );
    let typo = mcp.call(
        "layer_get",
        json!({"file": "a.ylp", "layer": "Base", "bogus": 1}),
    );
    assert_eq!(
        (typo["isError"].clone(), text_json(&typo)["code"].clone()),
        (json!(true), json!("invalid_request"))
    );
    let not_a_string = mcp.call("layer_get", json!({"file": 5, "layer": "Base"}));
    assert_eq!(text_json(&not_a_string)["code"], "invalid_request");
    let absent = mcp.call("doc_info", json!({"file": "missing.ylp"}));
    assert_eq!(absent["isError"], true);
    let outside = mcp.call(
        "export_channels",
        json!({"file": "a.ylp", "dir": "../outside"}),
    );
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
    let fx = Fixture::new("mcp-preview");
    fx.project("a.ylp");
    let (mut mcp, _) = Mcp::legacy(&fx, &nobody());
    let result = mcp.call("preview", json!({"file": "a.ylp", "max_edge": 24}));
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
    // リンクを読むと同じ PNG
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
    // 覚えていない番号は見つからない
    let gone = mcp.request(
        "resources/read",
        json!({"uri": "yolupainter://preview/9999.png"}),
    );
    assert!(gone["error"].is_object());
}

#[test]
fn resources_serve_the_bundled_docs_the_command_list_and_the_effect_kinds() {
    let fx = Fixture::new("mcp-resources");
    let (mut mcp, _) = Mcp::legacy(&fx, &nobody());
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
    let psd = mcp.result("resources/read", json!({"uri": "yolupainter://docs/psd"}));
    assert!(psd["contents"][0]["text"].as_str().unwrap().contains("PSD"));
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
}

#[test]
fn without_file_the_tools_go_to_the_running_app_or_say_why_not() {
    let fx = Fixture::new("mcp-live");
    fx.project("a.ylp");
    // アプリが受けていない: 直し方を言う失敗
    let (mut mcp, _) = Mcp::legacy(&fx, &nobody());
    let result = mcp.call("doc_info", json!({}));
    assert_eq!(result["isError"], true);
    let error = text_json(&result);
    assert_eq!(error["data"]["live"], "unreachable");
    assert!(error["message"]["en"]
        .as_str()
        .unwrap()
        .contains("external commands"));
    drop(mcp);
    // 受けているアプリ（代わりの待ち受け）がある: 同じツールがそちらへ当たる
    let app = FakeApp::start(&fx, "a.ylp", "mcp-app", Behavior::Serve);
    let (mut mcp, _) = Mcp::legacy(&fx, &app.name);
    let info = mcp.call("set_info", json!({}));
    assert_eq!(info["isError"], false);
    assert_eq!(info["structuredContent"]["name"], "Body");
    let edited = mcp.call("layer_set", json!({"layer": "Base", "opacity": 0.2}));
    assert_eq!(edited["isError"], false);
    assert_eq!(
        mcp.call("history_info", json!({}))["structuredContent"]["undo_count"],
        1
    );
    assert_eq!(app.served(), 3);
    // アプリの壊す操作の確認も同じ
    let refused = mcp.call("layer_delete", json!({"layer": "Tint"}));
    assert_eq!(text_json(&refused)["code"], "confirm_required");
    // `file` を付ければ、アプリを通さず、その .ylp を直接開く（アプリの文書は変わらない）
    let direct = mcp.call("layer_get", json!({"file": "a.ylp", "layer": "Base"}));
    assert_eq!(direct["structuredContent"]["opacity"], 1.0);
    assert_eq!(app.served(), 4, "file 付きの呼び出しはアプリへ行かない");
}

#[test]
fn the_open_files_are_reloaded_when_changed_outside_and_capped_when_all_have_edits() {
    let fx = Fixture::new("mcp-sessions");
    fx.project("a.ylp");
    let (mut mcp, _) = Mcp::legacy(&fx, &nobody());
    assert_eq!(
        mcp.call("layer_get", json!({"file": "a.ylp", "layer": "Base"}))["structuredContent"]
            ["opacity"],
        1.0
    );
    // 編集が無いあいだに外で書き換わったら、次の呼び出しで開き直す（古い中身を読ませない）
    std::thread::sleep(Duration::from_millis(30));
    fx.ok(&[
        "layer.set",
        "--file",
        "a.ylp",
        "--layer",
        "Base",
        "--opacity",
        "0.6",
        "--save",
    ]);
    assert_eq!(
        mcp.call("layer_get", json!({"file": "a.ylp", "layer": "Base"}))["structuredContent"]
            ["opacity"],
        0.6
    );
    // 編集があるときは開き直さず、保存は外の書き換えを衝突として断る
    mcp.call(
        "layer_set",
        json!({"file": "a.ylp", "layer": "Base", "opacity": 0.1}),
    );
    std::thread::sleep(Duration::from_millis(30));
    fx.ok(&[
        "layer.set",
        "--file",
        "a.ylp",
        "--layer",
        "Base",
        "--opacity",
        "0.7",
        "--save",
    ]);
    assert_eq!(
        mcp.call("layer_get", json!({"file": "a.ylp", "layer": "Base"}))["structuredContent"]
            ["opacity"],
        0.1,
        "編集中の中身は残る"
    );
    let conflict = mcp.call("save", json!({"file": "a.ylp", "confirm": true}));
    assert_eq!(
        (
            conflict["isError"].clone(),
            text_json(&conflict)["code"].clone()
        ),
        (json!(true), json!("conflict"))
    );

    // 開く数の上限: 全部が保存していない編集を持つなら、9 つ目は断る。保存すれば別のを閉じて開ける
    for n in 0..8 {
        let name = format!("many{n}.ylp");
        fx.project(&name);
        let edited = mcp.call(
            "layer_set",
            json!({"file": name, "layer": "Base", "opacity": 0.5}),
        );
        // a.ylp が編集中なので、8 つ目（a を含めて 9 つ目）で断られる
        if n < 7 {
            assert_eq!(edited["isError"], false, "{n}");
        } else {
            assert_eq!(edited["isError"], true, "{n}");
            assert_eq!(text_json(&edited)["code"], "budget");
        }
    }
    fx.project("fresh.ylp");
    let saved = mcp.call("save", json!({"file": "many0.ylp", "confirm": true}));
    assert_eq!(saved["isError"], false);
    assert_eq!(
        mcp.call("doc_info", json!({"file": "fresh.ylp"}))["isError"],
        false,
        "保存した 1 つを閉じて開ける"
    );
}

/// ディスクの .ylp の Base の不透明度。
fn disk_opacity(fx: &Fixture, file: &str) -> Value {
    fx.ok(&["layer.get", "--file", file, "--layer", "Base"])["opacity"].clone()
}

fn opacity_of(mcp: &mut Mcp, file: &str) -> Value {
    mcp.call("layer_get", json!({"file": file, "layer": "Base"}))["structuredContent"]["opacity"]
        .clone()
}

#[test]
fn save_as_moves_the_open_project_to_the_new_file_and_the_old_path_stays_the_old_file() {
    let fx = Fixture::new("mcp-save-as");
    fx.project("a.ylp");
    let (mut mcp, _) = Mcp::legacy(&fx, &nobody());
    mcp.call(
        "layer_set",
        json!({"file": "a.ylp", "layer": "Base", "opacity": 0.3}),
    );
    let saved = mcp.call(
        "save_as",
        json!({"file": "a.ylp", "path": "copy.ylp", "confirm": true}),
    );
    assert_eq!(saved["isError"], false, "{saved}");
    assert!(saved["structuredContent"]["path"]
        .as_str()
        .unwrap()
        .ends_with("copy.ylp"));
    // 新しい道は編集した文書そのもの、元の道は元のファイルを開き直した別の物
    assert_eq!(opacity_of(&mut mcp, "copy.ylp"), 0.3);
    assert_eq!(opacity_of(&mut mcp, "a.ylp"), 1.0);
    assert_eq!(disk_opacity(&fx, "a.ylp"), 1.0, "元のファイルは変わらない");
    // 新しい道で編集・保存すると copy.ylp だけが変わる（外の書き換えの見張りも copy.ylp を見る: 衝突しない）
    mcp.call(
        "layer_set",
        json!({"file": "copy.ylp", "layer": "Base", "opacity": 0.6}),
    );
    let saved = mcp.call("save", json!({"file": "copy.ylp", "confirm": true}));
    assert_eq!(saved["isError"], false, "{saved}");
    assert_eq!(
        (disk_opacity(&fx, "copy.ylp"), disk_opacity(&fx, "a.ylp")),
        (json!(0.6), json!(1.0))
    );
    // 元の道で編集・保存すると a.ylp だけが変わる（copy.ylp の文書とは別の組）
    mcp.call(
        "layer_set",
        json!({"file": "a.ylp", "layer": "Base", "opacity": 0.2}),
    );
    let saved = mcp.call("save", json!({"file": "a.ylp", "confirm": true}));
    assert_eq!(saved["isError"], false, "{saved}");
    assert_eq!(
        (disk_opacity(&fx, "copy.ylp"), disk_opacity(&fx, "a.ylp")),
        (json!(0.6), json!(0.2))
    );
    assert_eq!(opacity_of(&mut mcp, "copy.ylp"), 0.6);
    // copy.ylp を外で書き換えると、新しい道の組が（編集が無ければ）それを開き直す
    std::thread::sleep(Duration::from_millis(30));
    fx.ok(&[
        "layer.set",
        "--file",
        "copy.ylp",
        "--layer",
        "Base",
        "--opacity",
        "0.9",
        "--save",
    ]);
    assert_eq!(opacity_of(&mut mcp, "copy.ylp"), 0.9);
}

#[test]
fn save_as_over_a_file_another_path_holds_merges_when_clean_and_refuses_when_edited() {
    let fx = Fixture::new("mcp-save-as-over");
    for name in ["a.ylp", "b.ylp", "d.ylp", "e.ylp"] {
        fx.project(name);
    }
    let (mut mcp, _) = Mcp::legacy(&fx, &nobody());
    // b は編集せずに開いている。a の編集を b へ名前を付けて保存すると、b の組は 1 つにまとまる
    assert_eq!(opacity_of(&mut mcp, "b.ylp"), 1.0);
    mcp.call(
        "layer_set",
        json!({"file": "a.ylp", "layer": "Base", "opacity": 0.3}),
    );
    let saved = mcp.call(
        "save_as",
        json!({"file": "a.ylp", "path": "b.ylp", "confirm": true}),
    );
    assert_eq!(saved["isError"], false, "{saved}");
    assert_eq!(opacity_of(&mut mcp, "b.ylp"), 0.3);
    assert_eq!(opacity_of(&mut mcp, "a.ylp"), 1.0);
    mcp.call(
        "layer_set",
        json!({"file": "b.ylp", "layer": "Base", "opacity": 0.5}),
    );
    assert_eq!(
        mcp.call("save", json!({"file": "b.ylp", "confirm": true}))["isError"],
        false
    );
    assert_eq!(
        (disk_opacity(&fx, "a.ylp"), disk_opacity(&fx, "b.ylp")),
        (json!(1.0), json!(0.5)),
        "b の独立した 2 つ目の組が無いので、編集は分かれない"
    );

    // e は保存していない編集つき。d の編集を e へ名前を付けて保存しようとすると、書く前に断る（どちらの編集も、ファイルも残る）
    mcp.call(
        "layer_set",
        json!({"file": "e.ylp", "layer": "Base", "opacity": 0.9}),
    );
    mcp.call(
        "layer_set",
        json!({"file": "d.ylp", "layer": "Base", "opacity": 0.4}),
    );
    let refused = mcp.call(
        "save_as",
        json!({"file": "d.ylp", "path": "e.ylp", "confirm": true}),
    );
    assert_eq!(
        (
            refused["isError"].clone(),
            text_json(&refused)["code"].clone()
        ),
        (json!(true), json!("conflict")),
        "{refused}"
    );
    assert_eq!(opacity_of(&mut mcp, "e.ylp"), 0.9);
    assert_eq!(opacity_of(&mut mcp, "d.ylp"), 0.4);
    assert_eq!(disk_opacity(&fx, "e.ylp"), 1.0);
    // e を保存すれば、d の保存先にできる
    mcp.call("save", json!({"file": "e.ylp", "confirm": true}));
    let saved = mcp.call(
        "save_as",
        json!({"file": "d.ylp", "path": "e.ylp", "confirm": true}),
    );
    assert_eq!(saved["isError"], false, "{saved}");
    assert_eq!(disk_opacity(&fx, "e.ylp"), 0.4);
}

#[test]
fn a_refused_save_is_recovered_by_doc_open_which_discards_the_edits_of_that_file() {
    let fx = Fixture::new("mcp-reopen");
    fx.project("a.ylp");
    fx.project("b.ylp");
    let (mut mcp, _) = Mcp::legacy(&fx, &nobody());
    mcp.call(
        "layer_set",
        json!({"file": "a.ylp", "layer": "Base", "opacity": 0.1}),
    );
    std::thread::sleep(Duration::from_millis(30));
    fx.ok(&[
        "layer.set",
        "--file",
        "a.ylp",
        "--layer",
        "Base",
        "--opacity",
        "0.7",
        "--save",
    ]);
    let conflict = mcp.call("save", json!({"file": "a.ylp", "confirm": true}));
    assert_eq!(text_json(&conflict)["code"], "conflict");
    // 確認が無ければ断り、編集は残る
    let refused = mcp.call("doc_open", json!({"file": "a.ylp", "path": "a.ylp"}));
    assert_eq!(
        (
            refused["isError"].clone(),
            text_json(&refused)["code"].clone()
        ),
        (json!(true), json!("confirm_required"))
    );
    assert_eq!(opacity_of(&mut mcp, "a.ylp"), 0.1);
    // confirm: true で、編集を捨てて外の書き換えを開き直す
    let reopened = mcp.call(
        "doc_open",
        json!({"file": "a.ylp", "path": "a.ylp", "confirm": true}),
    );
    assert_eq!(reopened["isError"], false, "{reopened}");
    assert_eq!(reopened["structuredContent"]["unsaved"], false);
    assert_eq!(opacity_of(&mut mcp, "a.ylp"), 0.7);
    // 開き直した組は、そのまま編集・保存できる（衝突しない）
    mcp.call(
        "layer_set",
        json!({"file": "a.ylp", "layer": "Base", "opacity": 0.2}),
    );
    let saved = mcp.call("save", json!({"file": "a.ylp", "confirm": true}));
    assert_eq!(saved["isError"], false, "{saved}");
    assert_eq!(disk_opacity(&fx, "a.ylp"), 0.2);
    // 別のファイルを指す doc_open は、その組を別のファイルへ付け替える（元の道は元のファイルを開き直す）
    let moved = mcp.call(
        "doc_open",
        json!({"file": "a.ylp", "path": "b.ylp", "confirm": true}),
    );
    assert_eq!(moved["isError"], false, "{moved}");
    mcp.call(
        "layer_set",
        json!({"file": "b.ylp", "layer": "Base", "opacity": 0.4}),
    );
    assert_eq!(
        mcp.call("save", json!({"file": "b.ylp", "confirm": true}))["isError"],
        false
    );
    assert_eq!(
        (disk_opacity(&fx, "a.ylp"), disk_opacity(&fx, "b.ylp")),
        (json!(0.2), json!(0.4))
    );
    assert_eq!(opacity_of(&mut mcp, "a.ylp"), 0.2);
}

#[test]
fn the_server_ends_quietly_when_the_client_closes_the_input() {
    let fx = Fixture::new("mcp-eof");
    let (mut mcp, _) = Mcp::legacy(&fx, &nobody());
    let _ = mcp.result("tools/list", json!({}));
    drop(mcp.stdin.take());
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = mcp.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "標準入力が閉じても終わらない"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "{status:?}");
}

#[test]
fn the_command_json_the_tools_accept_is_what_the_cli_accepts() {
    // MCP の引数（`file` を除く）は、命令の JSON の `args` と同じ。`doc_info` の空の引数でも通る
    let fx = Fixture::new("mcp-same");
    fx.project("a.ylp");
    let (mut mcp, _) = Mcp::legacy(&fx, &nobody());
    let via_mcp = mcp.call("set_info", json!({"file": "a.ylp"}))["structuredContent"].clone();
    let via_cli = fx.ok(&["set.info", "--file", "a.ylp"]);
    let mut via_cli_payload = via_cli.clone();
    via_cli_payload.as_object_mut().unwrap().remove("reply");
    assert_eq!(via_mcp, via_cli_payload);
    parse_command_str(r#"{"command":"set.info","args":{}}"#).unwrap();
}

// ───────── structuredContent は outputSchema に合う ─────────

/// JSON Schema の、この試験に要る部分だけの検査（type・enum・const・required・properties・additionalProperties false・items・anyOf・oneOf・allOf・$ref）。
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
    let fx = Fixture::new("mcp-schema");
    fx.project("a.ylp");
    let (mut mcp, _) = Mcp::legacy(&fx, &nobody());
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
    let f = "a.ylp";
    let steps: Vec<(&str, Value)> = vec![
        ("doc_info", json!({})),
        ("doc_open", json!({"path": "a.ylp", "confirm": true})),
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
    for (tool, mut args) in steps {
        args["file"] = json!(f);
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
        seen.insert(tool);
    }
    // 効果の種類の返事は、全部の種類で schema に合う（型・範囲・選択肢の欄を含む）
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
