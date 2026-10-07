//! 命令の JSON を読む入口。版の確かめ・知らない命令と引数の誤りの言い分けをここで行い、型の `Deserialize` の生の誤りを見せない。
//!
//! 命令の JSON は `{"v": 1, "command": "layer.set", "args": {...}}`。`v`（命令の版）は省ける（今の版とみなす）。`args` も省ける（空）。

use serde_json::{json, Value};

use crate::command::{Command, COMMAND_VERSION};
use crate::error::{ErrorCode, OpError};
use crate::meta::commands;

/// 版の欄を確かめる。無ければ今の版。数でない・合わない版は断る。
pub fn check_version(value: Option<&Value>) -> Result<(), OpError> {
    let Some(v) = value else { return Ok(()) };
    match v.as_u64() {
        Some(n) if n == u64::from(COMMAND_VERSION) => Ok(()),
        _ => Err(OpError::new(
            ErrorCode::UnsupportedVersion,
            format!("命令の版 {v} には対応していません（対応: {COMMAND_VERSION}）"),
            format!("Command version {v} is not supported (supported: {COMMAND_VERSION})"),
        )
        .with_data(json!({"got": v, "supported": [COMMAND_VERSION]}))),
    }
}

/// JSON の値（`v`・`command`・`args`）から命令へ。
pub fn parse_command(value: &Value) -> Result<Command, OpError> {
    let Some(object) = value.as_object() else {
        return Err(OpError::invalid_request(
            "命令は JSON のオブジェクトです",
            "A command is a JSON object",
        ));
    };
    check_version(object.get("v"))?;
    let name = object
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            OpError::invalid_request(
                "command（命令の名前）がありません",
                "`command` (the command name) is missing",
            )
        })?;
    if !commands().iter().any(|c| c.name == name) {
        return Err(OpError::new(
            ErrorCode::UnknownCommand,
            format!("知らない命令: {name}"),
            format!("Unknown command: {name}"),
        )
        .with_data(json!({"commands": commands().iter().map(|c| c.name).collect::<Vec<_>>()})));
    }
    let args = object.get("args").cloned().unwrap_or_else(|| json!({}));
    let tagged = json!({"command": name, "args": args});
    serde_json::from_value::<Command>(tagged).map_err(|e| {
        OpError::invalid_request(
            format!("{name} の引数が正しくありません: {e}"),
            format!("Invalid arguments for {name}: {e}"),
        )
        .with_data(json!({"command": name}))
    })
}

/// JSON の文字列から命令へ。
pub fn parse_command_str(text: &str) -> Result<Command, OpError> {
    let value: Value = serde_json::from_str(text).map_err(|e| {
        OpError::invalid_request(
            format!("JSON を読めません: {e}"),
            format!("Cannot read the JSON: {e}"),
        )
    })?;
    parse_command(&value)
}

/// 命令の JSON（版つき）。
pub fn command_json(command: &Command) -> Value {
    let mut value = serde_json::to_value(command).expect("命令は JSON にできる");
    if let Value::Object(map) = &mut value {
        map.insert("v".into(), json!(COMMAND_VERSION));
    }
    value
}
