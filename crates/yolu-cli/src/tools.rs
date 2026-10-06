//! MCP のツールの定義（名前・題・説明・inputSchema・outputSchema・注釈）を、yolu-ops の命令の一覧から作る。手で書いた表は持たない。
//!
//! 定義は JSON の値で作る（rmcp の型への変換は `mcp`）。`yolupainter-cli schema --tools` も同じ物を出す。
//!
//! - 名前は命令の名前の `.` を `_` にした物（`layer_set`）。
//! - どのツールにも、任意の引数 `file`（画面なしで操作する .ylp）を足す。省くと起動中のアプリが相手。
//! - 命令の説明は英語（AI が読む）で、題は英語と日本語を並べる。
//! - 全部の命令がツール。文書を開くのは普通は `file` だが、`doc_open` は、`file` の組の保存していない編集を捨てて開き直す道
//!   （外でファイルが書き換わって `save` が衝突で断られたあと）として残す。

use serde_json::{json, Map, Value};
use yolu_ops::{commands, CommandSpec};

/// 画面なしで操作する .ylp を指す引数の名前。
pub const FILE_ARG: &str = "file";

const FILE_DESCRIPTION: &str = "Path of a .ylp project to operate on directly, without the YoluPainter app (absolute, or relative to the server's working folder). \
Edits stay in memory until the save tool is called with confirm: true. Omit it to operate on the running YoluPainter, which must have \"Accept external commands\" turned on in its settings.";

/// `doc_open` の説明に足す、このサーバーでの使い方（`file` の組が相手になる）。
const DOC_OPEN_NOTE: &str = " With `file`, pass the same .ylp as `path` to reload that project from disk: its unsaved in-memory edits and undo history are discarded (confirm: true is needed when there are any), \
for example after save was refused with code conflict because the file changed outside. Opening a project the first time needs no call: any tool with `file` opens it.";

/// ツールの説明（命令の説明に、`doc_open` だけこのサーバーでの使い方を足す）。
fn description(spec: &CommandSpec) -> String {
    if spec.name == "doc.open" {
        format!("{}{DOC_OPEN_NOTE}", spec.description.en)
    } else {
        spec.description.en.to_owned()
    }
}

/// ツールの inputSchema（命令の引数の schema に `file` を足した物）。
pub fn input_schema(spec: &CommandSpec) -> Value {
    let mut schema = spec.args_schema();
    if let Value::Object(map) = &mut schema {
        map.entry("type").or_insert_with(|| json!("object"));
        let props = map
            .entry("properties")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(props) = props {
            props.insert(
                FILE_ARG.into(),
                json!({"type": "string", "description": FILE_DESCRIPTION}),
            );
        }
    }
    schema
}

/// ツールの outputSchema。見本の PNG は structuredContent に入れない（画像の content で渡す）ので、`png` の欄を外す。
pub fn output_schema(spec: &CommandSpec) -> Value {
    let mut schema = spec.reply_schema();
    if spec.name == "preview" {
        if let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            props.remove("png");
        }
        if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|r| r != "png");
        }
    }
    schema
}

/// ツール 1 つの定義（MCP の `tools/list` の 1 要素）。
pub fn tool(spec: &CommandSpec) -> Value {
    let mut annotations = json!({
        "title": spec.title.en,
        "readOnlyHint": spec.read_only,
        "idempotentHint": spec.idempotent,
        "openWorldHint": false,
    });
    if !spec.read_only {
        annotations["destructiveHint"] = json!(spec.destructive());
    }
    json!({
        "name": spec.tool_name(),
        "title": format!("{} / {}", spec.title.en, spec.title.ja),
        "description": description(spec),
        "inputSchema": input_schema(spec),
        "outputSchema": output_schema(spec),
        "annotations": annotations,
    })
}

/// 全ツールの定義。
pub fn tools() -> Vec<Value> {
    commands().iter().map(tool).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_a_tool_with_annotations() {
        let list = tools();
        assert_eq!(list.len(), commands().len());
        for t in &list {
            let name = t["name"].as_str().unwrap();
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "{name}"
            );
            assert_eq!(t["inputSchema"]["type"], "object", "{name}");
            assert_eq!(t["outputSchema"]["type"], "object", "{name}");
            assert!(
                t["inputSchema"]["properties"][FILE_ARG].is_object(),
                "{name} に file の引数"
            );
            let a = &t["annotations"];
            assert!(
                a["readOnlyHint"].is_boolean()
                    && a["idempotentHint"].is_boolean()
                    && a["openWorldHint"] == false,
                "{name}"
            );
            assert!(!t["description"].as_str().unwrap().is_empty());
        }
    }

    #[test]
    fn destructive_hints_match_the_danger_marks() {
        for spec in commands().iter() {
            let t = tool(spec);
            let a = &t["annotations"];
            if spec.read_only {
                assert_eq!(a["readOnlyHint"], true);
                assert!(
                    a.get("destructiveHint").is_none(),
                    "{}: 読むだけの道具に壊す印は付けない",
                    spec.name
                );
            } else {
                assert_eq!(a["readOnlyHint"], false);
                assert_eq!(a["destructiveHint"], spec.destructive(), "{}", spec.name);
            }
            // 壊す命令は確認の引数を持つ
            if spec.destructive() {
                assert!(
                    t["inputSchema"]["properties"]["confirm"].is_object(),
                    "{} に confirm",
                    spec.name
                );
            }
        }
        let names: Vec<String> = tools()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect();
        for destructive in ["layer_delete", "mask_delete", "effect_delete", "save"] {
            let t = tools()
                .into_iter()
                .find(|t| t["name"] == destructive)
                .unwrap();
            assert_eq!(
                t["annotations"]["destructiveHint"], true,
                "{destructive} {names:?}"
            );
        }
        let get = tools()
            .into_iter()
            .find(|t| t["name"] == "layer_get")
            .unwrap();
        assert_eq!(get["annotations"]["readOnlyHint"], true);
    }

    #[test]
    fn the_preview_output_does_not_declare_the_image_bytes() {
        let t = tools()
            .into_iter()
            .find(|t| t["name"] == "preview")
            .unwrap();
        assert!(t["outputSchema"]["properties"].get("png").is_none());
        assert!(!t["outputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "png"));
        assert!(t["outputSchema"]["properties"]["width"].is_object());
    }
}
