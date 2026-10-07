//! アクション: 記録した命令の列（ファイルは `{"format": 1, "name": "...", "commands": [...]}`。命令は CLI・MCP と同じ JSON）と、その列を
//! 1 つのテクスチャセットの文書へ **1 回の取り消し**で当てる実行（[`run`]。命令 `action.run` も同じ実行で、MCP のツール `action_run` になる）。
//!
//! - 入れられる命令は、レイヤー・マスク・効果を変える命令だけ（[`ACTION_COMMANDS`]・[`check_allowed`]）。読む・見本・書き出し・保存・取り消し・
//!   `action.run` は入れない（取り消しはまとめの中ではできず、書き出し・保存はファイルを変えて戻せない）。
//! - 全部を core の `Document::batch` の 1 回で当てる（命令ごとの段にしない）。途中の命令が断ったら、そこまでに当てた分も戻して文書を元のままにし、
//!   何番目の命令が・なぜかを誤りの `data`（`index` は 0 から・`command`・`completed`）で返す。
//! - 相手の指し方は `$selected`（始めた時に選んでいるレイヤー。1 つに決める）・`$created:<n>`（この実行で n 番目に作ったレイヤー・効果）が使える
//!   （[`crate::refs`]）。
//! - 上限: 1 つのアクションの命令は [`MAX_COMMANDS`] まで、ファイルは [`MAX_FILE_BYTES`] まで、名前は [`MAX_NAME_CHARS`] 文字まで。

use schemars::{json_schema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use yolu_core::CoreError;

use crate::command::{ActionRunArgs, Command};
use crate::doc_ops;
use crate::error::{ErrorCode, OpError};
use crate::host::{check_confirm, guard, OpHost};
use crate::refs::{no_selection, resolve_relative, uses_selected, Created};
use crate::reply::{Edited, Reply};
use crate::wire::parse_command;

pub use crate::reply::{ActionDone, ActionStep};

/// アクションのファイルの形式の版。
pub const ACTION_FORMAT: u32 = 1;
/// 1 つのアクションの命令の上限。
pub const MAX_COMMANDS: usize = 1000;
/// アクションのファイルの大きさの上限（読む前に確かめる）。
pub const MAX_FILE_BYTES: u64 = 4 << 20;
/// アクションの名前の上限（文字）。
pub const MAX_NAME_CHARS: usize = 100;
/// アクションに入れられる命令（レイヤー・マスク・効果を変える命令）。
pub const ACTION_COMMANDS: [&str; 10] = [
    "layer.add",
    "layer.delete",
    "layer.move",
    "layer.set",
    "mask.add",
    "mask.delete",
    "mask.set",
    "effect.add",
    "effect.set",
    "effect.delete",
];

/// アクションのファイルの中身。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionFile {
    pub format: u32,
    pub name: String,
    pub commands: Vec<Command>,
}

impl ActionFile {
    pub fn new(name: impl Into<String>, commands: Vec<Command>) -> ActionFile {
        ActionFile {
            format: ACTION_FORMAT,
            name: name.into(),
            commands,
        }
    }

    /// ファイルに書く文（整えた JSON。末尾に改行）。
    pub fn to_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("アクションは JSON にできる");
        text.push('\n');
        text
    }
}

/// `action.run` の `commands` のスキーマ（入れられる命令の名前の表。引数は各命令のツールのスキーマ）。`Command` の型を入れ子にすると、
/// スキーマが自分を指す形になる（それを読めない MCP のクライアントがあっても困らないよう、名前の表と object に留める）。
pub fn commands_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "array",
        "minItems": 1,
        "maxItems": MAX_COMMANDS,
        "items": {
            "type": "object",
            "properties": {
                "command": {"type": "string", "enum": ACTION_COMMANDS},
                "args": {"type": "object"},
                "v": {"type": "integer"}
            },
            "required": ["command"],
            "additionalProperties": false
        }
    })
}

/// アクションの名前の検査（1〜[`MAX_NAME_CHARS`] 文字・制御文字なし・前後の空白なし）。
pub fn check_name(name: &str) -> Result<(), OpError> {
    let chars = name.chars().count();
    if chars == 0
        || chars > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
        || name.trim() != name
    {
        return Err(OpError::invalid_value(
            format!("アクションの名前は 1〜{MAX_NAME_CHARS} 文字で、制御文字と前後の空白を含められません"),
            format!("An action name has 1 to {MAX_NAME_CHARS} characters, no control characters and no leading or trailing spaces"),
        ));
    }
    Ok(())
}

/// アクションに入れられる命令か（[`ACTION_COMMANDS`]）。
pub fn check_allowed(command: &Command) -> Result<(), OpError> {
    if ACTION_COMMANDS.contains(&command.name()) {
        return Ok(());
    }
    Err(OpError::new(
        ErrorCode::Unsupported,
        format!(
            "{} はアクションに入れられません（入れられるのは、レイヤー・マスク・効果を変える命令だけです）",
            command.name()
        ),
        format!(
            "{} cannot be part of an action (only commands that change layers, masks and effects can)",
            command.name()
        ),
    ))
}

/// 命令の失敗に、何番目の命令か（0 から）・命令の名前・それまでに済んだ数を添える（CLI の batch と同じ欄）。
pub fn at_index(mut error: OpError, index: usize, command: &Command, completed: usize) -> OpError {
    let mut data = match error.data.take() {
        Some(Value::Object(map)) => map,
        Some(other) => Map::from_iter([("detail".to_owned(), other)]),
        None => Map::new(),
    };
    data.insert("index".into(), json!(index));
    data.insert("command".into(), json!(command.name()));
    data.insert("completed".into(), json!(completed));
    error.with_data(Value::Object(data))
}

/// アクションのファイルの文を読む。形式の版・名前・命令の数と形・入れられる命令かを確かめる（1 つでも違えば全体を断る）。
pub fn parse_action(text: &str) -> Result<ActionFile, OpError> {
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(too_large());
    }
    let value: Value = serde_json::from_str(text).map_err(|e| {
        OpError::new(
            ErrorCode::InvalidProject,
            format!("アクションの JSON を読めません（{e}）"),
            format!("Cannot read the action JSON ({e})"),
        )
    })?;
    let Value::Object(map) = value else {
        return Err(malformed(
            "オブジェクトではありません",
            "it is not an object",
        ));
    };
    if let Some(key) = map
        .keys()
        .find(|k| !matches!(k.as_str(), "format" | "name" | "commands"))
    {
        return Err(malformed(
            &format!("知らない欄「{key}」があります"),
            &format!("unknown field \"{key}\""),
        ));
    }
    match map.get("format").and_then(Value::as_u64) {
        Some(f) if f == u64::from(ACTION_FORMAT) => {}
        Some(f) => {
            return Err(OpError::new(
                ErrorCode::UnsupportedVersion,
                format!("アクションの形式 {f} は読めません（この版が読むのは {ACTION_FORMAT}）"),
                format!("Action format {f} is not supported (this version reads {ACTION_FORMAT})"),
            )
            .with_data(json!({"format": f, "supported": ACTION_FORMAT})))
        }
        None => return Err(malformed("format がありません", "no \"format\"")),
    }
    let name = map
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("name がありません", "no \"name\""))?
        .to_owned();
    check_name(&name)?;
    let Some(Value::Array(items)) = map.get("commands") else {
        return Err(malformed("commands がありません", "no \"commands\" array"));
    };
    let commands = parse_list(items)?;
    Ok(ActionFile {
        format: ACTION_FORMAT,
        name,
        commands,
    })
}

/// 命令の列の JSON を読む（数の上限・命令の形・入れられる命令か。1 つでも違えば全体を断り、`data.index` に何番目か）。
pub fn parse_list(items: &[Value]) -> Result<Vec<Command>, OpError> {
    if items.len() > MAX_COMMANDS {
        return Err(too_many(items.len()));
    }
    let mut commands = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let command = parse_command(item).map_err(|e| index_only(e, index))?;
        check_allowed(&command).map_err(|e| at_index(e, index, &command, 0))?;
        commands.push(command);
    }
    Ok(commands)
}

fn index_only(mut error: OpError, index: usize) -> OpError {
    let mut data = match error.data.take() {
        Some(Value::Object(map)) => map,
        Some(other) => Map::from_iter([("detail".to_owned(), other)]),
        None => Map::new(),
    };
    data.insert("index".into(), json!(index));
    error.with_data(Value::Object(data))
}

fn malformed(ja: &str, en: &str) -> OpError {
    OpError::new(
        ErrorCode::InvalidProject,
        format!("アクションのファイルの形が違います（{ja}）"),
        format!("The action file is malformed ({en})"),
    )
}

fn too_large() -> OpError {
    OpError::new(
        ErrorCode::Budget,
        format!(
            "アクションのファイルが大きすぎます（{} MiB まで）",
            MAX_FILE_BYTES >> 20
        ),
        format!(
            "The action file is too large (at most {} MiB)",
            MAX_FILE_BYTES >> 20
        ),
    )
}

fn too_many(count: usize) -> OpError {
    OpError::new(
        ErrorCode::Budget,
        format!("アクションの命令は {MAX_COMMANDS} 個までです"),
        format!("An action has at most {MAX_COMMANDS} commands"),
    )
    .with_data(json!({"count": count, "max": MAX_COMMANDS}))
}

/// 命令の `set` が、全部同じか省いてあるか（アクションは 1 つのテクスチャセットに当てる）。同じならそのセット（全部省けば None）。
fn one_set(commands: &[Command]) -> Result<Option<String>, OpError> {
    let mut chosen: Option<&str> = None;
    for (index, command) in commands.iter().enumerate() {
        if let Some(set) = command.set() {
            match chosen {
                Some(first) if first != set => return Err(at_index(
                    OpError::invalid_request(
                        "アクションは 1 つのテクスチャセットに当てます（set が命令ごとに違います）",
                        "An action works on one texture set (the commands name different sets)",
                    ),
                    index,
                    command,
                    0,
                )),
                _ => chosen = Some(set),
            }
        }
    }
    Ok(chosen.map(str::to_owned))
}

/// 命令の列を、1 つのテクスチャセット（命令の `set`。省けば今のセット）の文書へ、1 回の取り消しで当てる。
///
/// 当てる前に、全部の命令が入れられる物か・壊す命令に `confirm: true` があるかを確かめる（1 つでも違えば何も変えない）。`$selected` は
/// 始めた時に選んでいるレイヤーの 1 つに決める。途中の命令が断ったら、それまでに当てた分も戻し、`data` に何番目か（`index`、0 から）を添えて返す。
pub fn run(host: &mut dyn OpHost, commands: &[Command]) -> Result<ActionDone, OpError> {
    guard("action", || run_checked(host, commands))
}

/// 命令 `action.run`（`execute` の振り分けから。途中の panic は外の `guard` が受ける）: 命令の列を読んでから [`run`] と同じに当てる。
pub(crate) fn run_args(host: &mut dyn OpHost, args: &ActionRunArgs) -> Result<ActionDone, OpError> {
    let commands = parse_list(&args.commands)?;
    run_checked(host, &commands)
}

fn run_checked(host: &mut dyn OpHost, commands: &[Command]) -> Result<ActionDone, OpError> {
    if commands.is_empty() {
        return Err(OpError::invalid_request(
            "アクションに命令がありません",
            "The action has no commands",
        ));
    }
    if commands.len() > MAX_COMMANDS {
        return Err(too_many(commands.len()));
    }
    for (index, command) in commands.iter().enumerate() {
        check_allowed(command)
            .and_then(|()| check_confirm(command))
            .map_err(|e| at_index(e, index, command, 0))?;
    }
    let set = one_set(commands)?;
    let selected = match commands.iter().position(uses_selected) {
        Some(index) => Some(
            host.selected_layer(set.as_deref())
                .map_err(|e| at_index(e, index, &commands[index], 0))?,
        ),
        None => None,
    };
    let mut steps: Vec<ActionStep> = Vec::new();
    let mut failure: Option<OpError> = None;
    let mut set_id = String::new();
    let reply = host.write_set(set.as_deref(), &mut |facts, doc| {
        steps.clear();
        failure = None;
        set_id = facts.id.to_owned();
        let before = doc.undo_count();
        let outcome = doc.batch(|d| {
            let mut created = Created::default();
            for (index, command) in commands.iter().enumerate() {
                let step = resolve_relative(command, Some(&created), &mut || {
                    selected.clone().ok_or_else(|| no_selection(None))
                })
                .and_then(|resolved| {
                    let command = resolved.unwrap_or_else(|| command.clone());
                    doc_ops::write(facts, d, &command).map(|reply| (command, reply))
                });
                match step {
                    Ok((command, reply)) => {
                        created.note(&command, &reply);
                        steps.push(step_of(&reply));
                    }
                    Err(e) => {
                        failure = Some(at_index(e, index, command, steps.len()));
                        // まとめを戻すための印（呼び手には `failure` を返す）
                        return Err(CoreError::Cancelled);
                    }
                }
            }
            Ok(())
        });
        match outcome {
            Ok(()) => Ok(Reply::Edited(Edited {
                set: facts.id.to_owned(),
                layer: None,
                effect: None,
                unchanged: doc.undo_count() == before,
                undo_count: doc.undo_count() as u32,
                can_undo: doc.can_undo(),
                // 命令ごとの知らせは steps が持つ
                notes: Vec::new(),
            })),
            Err(e) => Err(failure.take().unwrap_or_else(|| OpError::from_core(&e))),
        }
    })?;
    let Reply::Edited(edited) = reply else {
        unreachable!("アクションの返事は Edited");
    };
    Ok(ActionDone {
        set: if set_id.is_empty() {
            edited.set
        } else {
            set_id
        },
        steps,
        unchanged: edited.unchanged,
        undo_count: edited.undo_count,
        can_undo: edited.can_undo,
    })
}

/// 1 つの命令の返事から、その命令がしたこと（入れられる命令の返事は、どれも `Edited`）。
fn step_of(reply: &Reply) -> ActionStep {
    match reply {
        Reply::Edited(e) => ActionStep {
            layer: e.layer.clone(),
            effect: e.effect.clone(),
            unchanged: e.unchanged,
            notes: e.notes.clone(),
        },
        _ => ActionStep::default(),
    }
}
