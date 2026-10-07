//! MCP のツールの定義（名前・題・説明・inputSchema・outputSchema・注釈）を、yolu-ops の命令の一覧から作る。手で書いた表は持たない。
//!
//! 定義は JSON の値で作る（rmcp の型への変換は `server`）。`yolupainter-cli schema --tools` も同じ物を出す。
//!
//! - 名前は命令の名前の `.` を `_` にした物（`layer_set`）。
//! - 引数は命令の引数そのもの。相手はいつも起動中のアプリ（開いている文書）。
//! - 命令の説明は英語（AI が読む）で、題は英語と日本語を並べる。

use serde_json::{json, Value};
use yolu_ops::{commands, CommandSpec};

/// `doc_open` の説明に足す、MCP での相手（起動中のアプリは文書を開き替えない）。
const DOC_OPEN_NOTE: &str = " Through MCP the target is the running YoluPainter app, which does not switch documents: \
pass the path of the file the app has open to get that document; any other path is refused with code unsupported.";

/// ツールの説明（命令の説明に、`doc_open` だけ MCP での振る舞いを足す）。
fn description(spec: &CommandSpec) -> String {
    if spec.name == "doc.open" {
        format!("{}{DOC_OPEN_NOTE}", spec.description.en)
    } else {
        spec.description.en.to_owned()
    }
}

/// ツールの inputSchema（命令の引数の schema。型と欄の表が無ければ足す: 引数の無い命令も、空の欄の表を持つ object にする）。
pub fn input_schema(spec: &CommandSpec) -> Value {
    let mut schema = spec.args_schema();
    if let Value::Object(map) = &mut schema {
        map.entry("type").or_insert_with(|| json!("object"));
        map.entry("properties").or_insert_with(|| json!({}));
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
                t["inputSchema"]["properties"].get("file").is_none(),
                "{name}: ファイルを直に開く引数は無い"
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
