//! コマンドラインの引数から、実行する動作（命令の JSON・schema・MCP など）を組み立てる。
//!
//! 命令の引数は `--名前 値` で渡し、型は命令の JSON Schema（yolu-ops の型から作った物）に合わせて読む（文字列の欄に `123` を渡しても文字列、
//! 数の欄は数）。ネストした欄は `--values.radius 4` のように `.` でたどるか、`--values '{"radius":4}'` の JSON で渡す。まるごと JSON で
//! 渡すなら、命令の名前のあとに `'{"layer":"Base"}'`・`@ファイル`・`-`（標準入力）を 1 つ置く（後ろの `--名前` が上書きする）。

use std::collections::BTreeSet;

use serde_json::{json, Map, Value};
use yolu_ops::{command_spec, command_spec_by_tool, commands, CommandSpec, Lang, OpError};

/// この CLI が自分で使う名前（命令の引数の名前と重ならない。試験が確かめる）。
pub const RESERVED: &[&str] = &[
    "file", "live", "save", "pretty", "lang", "timeout", "out", "port", "cwd", "args", "help",
    "version", "tools",
];

/// どの動作にも付けられる設定。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Global {
    /// 画面なしで操作する .ylp。
    pub file: Option<String>,
    /// 起動中のアプリへ（`--file` が無いときの既定）。
    pub live: bool,
    /// 成功したら .ylp を上書き保存する（`--file` のときだけ）。
    pub save: bool,
    pub pretty: bool,
    pub lang: Option<Lang>,
    /// 起動中のアプリの返事を待つ秒数。
    pub timeout_secs: Option<f64>,
    /// 見本（`preview`）の PNG をこの道に書き、JSON には道を出す。
    pub out: Option<String>,
    /// 起動中のアプリの番号（アプリの設定「外からの操作を受ける」の番号。既定は `yolu_mcp::DEFAULT_PORT`）。
    pub port: Option<u16>,
    /// 相対パスの起点（既定は今のフォルダ）。
    pub cwd: Option<String>,
}

/// 実行する動作。
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Help,
    Version,
    /// 命令の一覧。
    Commands,
    /// 命令の JSON Schema（`name` があれば 1 つ、`tools` なら MCP のツールの定義）。
    Schema {
        name: Option<String>,
        tools: bool,
    },
    /// MCP サーバー（stdio）。
    Mcp,
    /// 命令を順に実行する（`source` は道・`-`・省略は標準入力）。
    Batch {
        source: Option<String>,
    },
    /// アクションのファイル（`{"format": 1, "name": ..., "commands": [...]}`）を、1 回の取り消しで当てる。
    RunAction {
        path: String,
    },
    /// 命令 1 つ。`args` は命令の引数（JSON の欄名で）。
    Run {
        name: String,
        args: Map<String, Value>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Invocation {
    pub global: Global,
    pub action: Action,
}

fn usage_error(ja: impl Into<String>, en: impl Into<String>) -> OpError {
    OpError::invalid_request(ja, en)
}

/// `--名前` の名前を欄の名前（`_` 区切り）にそろえる。
fn normalize(name: &str) -> String {
    name.replace('-', "_")
}

/// `@道`・`-`（標準入力）の中身を読む関数。
pub type Source<'a> = &'a dyn Fn(&str) -> Result<String, OpError>;

/// 引数を読む。`source` は `@ファイル`・`-` の読み出し。
pub fn parse(tokens: &[String], source: Source<'_>) -> Result<Invocation, OpError> {
    let mut global = Global::default();
    let mut command: Option<String> = None;
    let mut spec: Option<&'static CommandSpec> = None;
    let mut sub_args: Vec<String> = Vec::new();
    let mut map = Map::new();
    let mut positional_json = false;
    let mut json_args: Option<String> = None;
    // `--help`・`--version` は、ほかの引数を全部読んでから（`--lang` が後ろにあっても効くように）答える
    let mut want: Option<Action> = None;
    let mut i = 0;
    let mut options_done = false;
    while i < tokens.len() {
        let token = &tokens[i];
        i += 1;
        if !options_done && token == "--" {
            options_done = true;
            continue;
        }
        let is_option = !options_done && token.starts_with("--") && token.len() > 2;
        if !is_option {
            if command.is_none() {
                command = Some(token.clone());
                spec = lookup(token);
            } else if spec.is_some() {
                if positional_json {
                    return Err(usage_error(
                        format!("引数の JSON が 2 つあります: {token}"),
                        format!("More than one JSON argument: {token}"),
                    ));
                }
                positional_json = true;
                let object = read_json_object(token, source)?;
                for (key, value) in object {
                    map.insert(key, value);
                }
            } else {
                sub_args.push(token.clone());
            }
            continue;
        }
        let body = &token[2..];
        let (raw_name, inline) = match body.split_once('=') {
            Some((n, v)) => (n, Some(v.to_owned())),
            None => (body, None),
        };
        let name = normalize(raw_name);
        let mut take = |what: &str| take_value(tokens, &mut i, inline.clone(), raw_name, what);
        match name.as_str() {
            "file" => global.file = Some(take("a .ylp path")?),
            "live" => global.live = true,
            "save" => global.save = true,
            "pretty" => global.pretty = true,
            "help" => want = Some(Action::Help),
            "version" => want = Some(Action::Version),
            "lang" => {
                let text = take("ja or en")?;
                global.lang = Some(Lang::parse(&text).ok_or_else(|| {
                    usage_error(
                        format!("--lang は ja か en です（{text}）"),
                        format!("--lang is ja or en (got {text})"),
                    )
                })?);
            }
            "timeout" => {
                let text = take("seconds")?;
                let secs: f64 = text.parse().ok().filter(|s: &f64| s.is_finite() && *s > 0.0 && *s <= 3600.0).ok_or_else(|| {
                    usage_error(
                        format!("--timeout は 0 より大きく 3600 以下の秒数です（{text}）"),
                        format!("--timeout is a number of seconds above 0 and at most 3600 (got {text})"),
                    )
                })?;
                global.timeout_secs = Some(secs);
            }
            "out" => global.out = Some(take("a PNG path")?),
            "port" => {
                let text = take("a port number")?;
                let port = text
                    .parse::<u16>()
                    .ok()
                    .filter(|p| yolu_mcp::valid_port(*p))
                    .ok_or_else(|| {
                        usage_error(
                            format!("--port は 1024〜65535 の番号です（{text}）"),
                            format!("--port is a number from 1024 to 65535 (got {text})"),
                        )
                    })?;
                global.port = Some(port);
            }
            "cwd" => global.cwd = Some(take("a folder")?),
            "args" => json_args = Some(take("JSON, @file or -")?),
            "tools" => {
                if command.as_deref() == Some("schema") {
                    sub_args.push("--tools".into());
                } else {
                    return Err(unknown_option(raw_name));
                }
            }
            _ => {
                let Some(spec) = spec else {
                    return Err(match command {
                        None => usage_error(
                            format!("--{raw_name} は命令の名前のあとに置きます（先に命令の名前を書いてください）"),
                            format!("Put --{raw_name} after the command name"),
                        ),
                        Some(_) => unknown_option(raw_name),
                    });
                };
                let schema = spec.args_schema();
                let (target_name, negated) = split_negation(&schema, &name);
                let is_flag = negated
                    || leaf_types(&schema, &schema, target_name).is_some_and(|t| t == ["boolean"]);
                let value_text = if let Some(v) = inline {
                    Some(v)
                } else if is_flag {
                    // 真偽の欄は `--confirm`（真）・`--confirm false`・`--no-confirm`
                    match tokens.get(i).map(|t| t.to_ascii_lowercase()) {
                        Some(t) if t == "true" || t == "false" => {
                            i += 1;
                            Some(t)
                        }
                        _ => None,
                    }
                } else {
                    let v = tokens.get(i).cloned().ok_or_else(|| {
                        usage_error(
                            format!("--{raw_name} に値がありません"),
                            format!("--{raw_name} needs a value"),
                        )
                    })?;
                    i += 1;
                    Some(v)
                };
                set_flag(&mut map, spec, &schema, &name, value_text)?;
            }
        }
    }
    if let Some(action) = want {
        return Ok(Invocation { global, action });
    }
    if let Some(text) = json_args {
        let object = read_json_object(&text, source)?;
        for (key, value) in object {
            map.entry(key).or_insert(value);
        }
    }
    if global.file.is_some() && global.live {
        return Err(usage_error(
            "--file と --live は同時に使えません",
            "--file and --live cannot be used together",
        ));
    }
    if global.save && global.file.is_none() {
        return Err(usage_error(
            "--save は --file で開いた .ylp にだけ使えます（起動中のアプリは、save 命令を使います）",
            "--save works only with a .ylp opened by --file (for the running app use the save command)",
        ));
    }
    let Some(name) = command else {
        return Ok(Invocation {
            global,
            action: Action::Help,
        });
    };
    let has_args = !map.is_empty();
    if matches!(name.as_str(), "mcp" | "commands") && !sub_args.is_empty() {
        return Err(usage_error(
            format!("{name} に余計な引数があります: {}", sub_args.join(" ")),
            format!("{name} takes no arguments: {}", sub_args.join(" ")),
        ));
    }
    let action = match name.as_str() {
        "help" => Action::Help,
        "mcp" => Action::Mcp,
        "commands" => Action::Commands,
        "schema" => {
            let tools = sub_args.iter().any(|a| a == "--tools");
            let names: Vec<&String> = sub_args.iter().filter(|a| *a != "--tools").collect();
            if names.len() > 1 {
                return Err(usage_error(
                    "schema の対象は 1 つです",
                    "schema takes one command name",
                ));
            }
            Action::Schema {
                name: names.first().map(|n| (*n).clone()),
                tools,
            }
        }
        "run-action" | "run_action" => match sub_args.as_slice() {
            [path] => Action::RunAction { path: path.clone() },
            _ => {
                return Err(usage_error(
                    "run-action にはアクションのファイルを 1 つ渡します",
                    "run-action takes one action file",
                ))
            }
        },
        "batch" => {
            if sub_args.len() > 1 {
                return Err(usage_error(
                    "batch の入力は 1 つです",
                    "batch takes one input",
                ));
            }
            Action::Batch {
                source: sub_args.first().cloned(),
            }
        }
        _ => match spec {
            Some(spec) => Action::Run {
                name: spec.name.to_owned(),
                args: map,
            },
            None => return Err(unknown_command(&name)),
        },
    };
    if !matches!(action, Action::Run { .. }) && has_args {
        return Err(usage_error(
            format!("{name} に命令の引数は渡せません"),
            format!("{name} takes no command arguments"),
        ));
    }
    Ok(Invocation { global, action })
}

/// `--名前=値` の値か、次の引数を値として取る。
fn take_value(
    tokens: &[String],
    i: &mut usize,
    inline: Option<String>,
    raw_name: &str,
    what: &str,
) -> Result<String, OpError> {
    if let Some(v) = inline {
        return Ok(v);
    }
    let v = tokens.get(*i).cloned().ok_or_else(|| {
        usage_error(
            format!("--{raw_name} に値がありません"),
            format!("--{raw_name} needs a value ({what})"),
        )
    })?;
    *i += 1;
    Ok(v)
}

fn lookup(token: &str) -> Option<&'static CommandSpec> {
    command_spec(token).or_else(|| command_spec_by_tool(token))
}

fn unknown_option(name: &str) -> OpError {
    usage_error(
        format!("知らないオプション: --{name}"),
        format!("Unknown option: --{name}"),
    )
}

fn unknown_command(name: &str) -> OpError {
    let names: Vec<&str> = commands().iter().map(|c| c.name).collect();
    OpError::new(
        yolu_ops::ErrorCode::UnknownCommand,
        format!("知らない命令: {name}（一覧は `yolupainter-cli commands`）"),
        format!("Unknown command: {name} (list them with `yolupainter-cli commands`)"),
    )
    .with_data(json!({"commands": names}))
}

fn read_json_object(text: &str, source: Source<'_>) -> Result<Map<String, Value>, OpError> {
    let body = if text == "-" || text.starts_with('@') {
        source(text)?
    } else {
        text.to_owned()
    };
    match serde_json::from_str::<Value>(&body) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(usage_error(
            "引数の JSON はオブジェクト（{...}）です",
            "The JSON arguments must be an object ({...})",
        )),
        Err(e) => Err(usage_error(
            format!("引数の JSON を読めません: {e}"),
            format!("Cannot read the JSON arguments: {e}"),
        )),
    }
}

// ───────── 型に合わせた値の読み ─────────

/// `$ref`（`#/$defs/名前`）をたどる。
fn follow<'a>(root: &'a Value, mut node: &'a Value) -> &'a Value {
    for _ in 0..16 {
        let Some(reference) = node.get("$ref").and_then(Value::as_str) else {
            break;
        };
        let Some(target) = reference.strip_prefix('#').and_then(|p| root.pointer(p)) else {
            break;
        };
        node = target;
    }
    node
}

/// `anyOf`・`oneOf` の「null 以外が 1 つだけ」を、その 1 つとして見る（`Option<T>`）。
fn effective<'a>(root: &'a Value, node: &'a Value) -> &'a Value {
    let node = follow(root, node);
    for key in ["anyOf", "oneOf"] {
        if let Some(members) = node.get(key).and_then(Value::as_array) {
            let others: Vec<&Value> = members
                .iter()
                .map(|m| follow(root, m))
                .filter(|m| m.get("type").and_then(Value::as_str) != Some("null"))
                .collect();
            if others.len() == 1 {
                return effective(root, others[0]);
            }
        }
    }
    node
}

/// 欄が取れる JSON の型（`string`・`boolean`・`integer`・`number`・`array`・`object`・`null`）。
fn node_types(root: &Value, node: &Value) -> BTreeSet<String> {
    let node = follow(root, node);
    let mut out = BTreeSet::new();
    match node.get("type") {
        Some(Value::String(t)) => {
            out.insert(t.clone());
        }
        Some(Value::Array(ts)) => {
            out.extend(ts.iter().filter_map(Value::as_str).map(str::to_owned))
        }
        _ => {}
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(members) = node.get(key).and_then(Value::as_array) {
            for m in members {
                out.extend(node_types(root, m));
            }
        }
    }
    if node.get("enum").is_some() || node.get("const").is_some() {
        out.insert("string".into());
    }
    out
}

/// 命令の引数の欄 `path`（`.` 区切り）の schema。`additionalProperties` の値もたどる。
fn field_schema<'a>(root: &'a Value, schema: &'a Value, path: &str) -> Option<&'a Value> {
    let mut node = schema;
    for segment in path.split('.') {
        let here = effective(root, node);
        if let Some(next) = here.get("properties").and_then(|p| p.get(segment)) {
            node = next;
        } else {
            node = here.get("additionalProperties").filter(|v| v.is_object())?;
        }
    }
    Some(node)
}

/// 欄の型（null を除いた並び。schema に無い欄は None）。
fn leaf_types(root: &Value, schema: &Value, path: &str) -> Option<Vec<String>> {
    let node = field_schema(root, schema, path)?;
    let mut types: Vec<String> = node_types(root, node)
        .into_iter()
        .filter(|t| t != "null")
        .collect();
    types.sort();
    Some(types)
}

fn is_json_number(text: &str) -> Option<Value> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(t)
        .ok()
        .filter(Value::is_number)
}

/// 値の文字列を、欄の型に合わせて JSON の値にする。
fn coerce(
    text: &str,
    types: &Option<Vec<String>>,
    accepts_null: bool,
    field: &str,
) -> Result<Value, OpError> {
    let bad = |what: &str, what_en: &str| {
        usage_error(
            format!("--{field} は{what}です（{text}）"),
            format!("--{field} must be {what_en} (got {text})"),
        )
    };
    if accepts_null && text == "null" {
        return Ok(Value::Null);
    }
    let Some(types) = types else {
        // schema に無い所（値の中身など）は、JSON として読めればその値、読めなければ文字列
        return Ok(serde_json::from_str::<Value>(text)
            .ok()
            .filter(|v| !v.is_object() && !v.is_array())
            .unwrap_or_else(|| Value::String(text.to_owned())));
    };
    let kinds: Vec<&str> = types.iter().map(String::as_str).collect();
    match kinds.as_slice() {
        ["string"] => Ok(Value::String(text.to_owned())),
        ["boolean"] => match text.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(Value::Bool(true)),
            "false" | "0" | "no" | "off" => Ok(Value::Bool(false)),
            _ => Err(bad("true か false", "true or false")),
        },
        ["integer"] => text
            .trim()
            .parse::<i64>()
            .map(|n| json!(n))
            .map_err(|_| bad("整数", "an integer")),
        ["integer", "number"] | ["number"] => {
            is_json_number(text).ok_or_else(|| bad("数", "a number"))
        }
        ["array"] => {
            let t = text.trim();
            if t.starts_with('[') {
                serde_json::from_str::<Value>(t)
                    .ok()
                    .filter(Value::is_array)
                    .ok_or_else(|| bad("JSON の配列", "a JSON array"))
            } else {
                Ok(Value::Array(
                    t.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|s| Value::String(s.to_owned()))
                        .collect(),
                ))
            }
        }
        ["object"] => serde_json::from_str::<Value>(text.trim())
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| bad("JSON のオブジェクト", "a JSON object")),
        _ => {
            // 数・真偽・文字列のどれでもよい欄（効果の値）
            let lower = text.to_ascii_lowercase();
            if lower == "true" || lower == "false" {
                return Ok(Value::Bool(lower == "true"));
            }
            Ok(is_json_number(text).unwrap_or_else(|| Value::String(text.to_owned())))
        }
    }
}

/// `--no-confirm` のように、真偽の欄を偽にする書き方か（欄の名前そのものが `no_` で始まるときは、欄の名前を優先する）。
fn split_negation<'a>(schema: &Value, name: &'a str) -> (&'a str, bool) {
    match name.strip_prefix("no_") {
        Some(rest)
            if field_schema(schema, schema, name).is_none()
                && field_schema(schema, schema, rest).is_some() =>
        {
            (rest, true)
        }
        _ => (name, false),
    }
}

/// `--名前 値` を、命令の引数の対応表へ入れる。
fn set_flag(
    map: &mut Map<String, Value>,
    spec: &CommandSpec,
    schema: &Value,
    name: &str,
    value: Option<String>,
) -> Result<(), OpError> {
    let (name, negated) = split_negation(schema, name);
    let first = name.split('.').next().unwrap_or(name);
    if field_schema(schema, schema, first).is_none() {
        let known: Vec<&str> = effective(schema, schema)
            .get("properties")
            .and_then(Value::as_object)
            .map(|p| p.keys().map(String::as_str).collect())
            .unwrap_or_default();
        return Err(usage_error(
            format!(
                "{} に --{name} という引数はありません（使える引数: {}）",
                spec.name,
                known.join("・")
            ),
            format!(
                "{} has no --{name} argument (available: {})",
                spec.name,
                known.join(", ")
            ),
        )
        .with_data(json!({"command": spec.name, "arguments": known})));
    }
    let node = field_schema(schema, schema, name);
    let types = leaf_types(schema, schema, name);
    let accepts_null = node.is_some_and(|n| node_types(schema, n).contains("null"));
    let value = match (negated, value) {
        (true, None) => Value::Bool(false),
        (true, Some(_)) => {
            return Err(usage_error(
                format!("--no-{name} に値は付けません"),
                format!("--no-{name} takes no value"),
            ))
        }
        (false, None) => Value::Bool(true),
        (false, Some(text)) => coerce(&text, &types, accepts_null, name)?,
    };
    let is_array = types.as_deref() == Some(&["array".to_owned()]);
    insert_path(map, name, value, is_array)
}

/// `a.b.c` の道へ入れる（途中のオブジェクトは作る。配列の欄に同じ名前を繰り返したら足す）。
fn insert_path(
    map: &mut Map<String, Value>,
    path: &str,
    value: Value,
    append: bool,
) -> Result<(), OpError> {
    let segments: Vec<&str> = path.split('.').collect();
    let mut node = map;
    for segment in &segments[..segments.len() - 1] {
        let entry = node
            .entry((*segment).to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        node = entry.as_object_mut().ok_or_else(|| {
            usage_error(
                format!("--{path} の途中の {segment} がオブジェクトではありません"),
                format!("--{path}: {segment} is not an object"),
            )
        })?;
    }
    let last = segments[segments.len() - 1];
    match (node.get_mut(last), value) {
        (Some(Value::Array(existing)), Value::Array(more)) if append => existing.extend(more),
        (Some(Value::Object(existing)), Value::Object(more)) => existing.extend(more),
        (_, value) => {
            node.insert(last.to_owned(), value);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_source(_: &str) -> Result<String, OpError> {
        Err(usage_error("読めません", "cannot read"))
    }

    fn run(tokens: &[&str]) -> Result<Invocation, OpError> {
        let tokens: Vec<String> = tokens.iter().map(|t| (*t).to_owned()).collect();
        parse(&tokens, &no_source)
    }

    fn args_of(tokens: &[&str]) -> Value {
        match run(tokens).unwrap().action {
            Action::Run { args, .. } => Value::Object(args),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn flags_follow_the_schema_types() {
        // 数の欄は数、文字列の欄は数字でも文字列、真偽の欄は値なしで真
        let v = args_of(&[
            "layer.set",
            "--layer",
            "123",
            "--opacity",
            "0.5",
            "--visible",
        ]);
        assert_eq!(v, json!({"layer": "123", "opacity": 0.5, "visible": true}));
        let v = args_of(&[
            "layer.set",
            "--layer",
            "A",
            "--visible",
            "false",
            "--clipping=true",
        ]);
        assert_eq!(v, json!({"layer": "A", "visible": false, "clipping": true}));
        let v = args_of(&["layer.delete", "--layer", "A", "--confirm"]);
        assert_eq!(v, json!({"layer": "A", "confirm": true}));
        let v = args_of(&["layer.delete", "--layer", "A", "--no-confirm"]);
        assert_eq!(v, json!({"layer": "A", "confirm": false}));
    }

    #[test]
    fn nested_fields_use_dots_and_values_guess_their_type() {
        let v = args_of(&[
            "effect.add",
            "--layer",
            "A",
            "--kind",
            "blur",
            "--values.radius",
            "4",
            "--values.monochrome",
            "true",
            "--values.preset",
            "rust",
        ]);
        assert_eq!(
            v,
            json!({"layer": "A", "kind": "blur", "values": {"radius": 4, "monochrome": true, "preset": "rust"}})
        );
        let v = args_of(&[
            "layer.add",
            "--kind",
            "fill",
            "--fill.Color",
            "#ff0000",
            "--fill.Roughness",
            "#808080",
        ]);
        assert_eq!(
            v["fill"],
            json!({"Color": "#ff0000", "Roughness": "#808080"})
        );
        // JSON の塊でも渡せて、点の道と混ぜられる
        let v = args_of(&[
            "effect.add",
            "--layer",
            "A",
            "--kind",
            "blur",
            "--values",
            r#"{"radius": 2}"#,
            "--values.seed",
            "7",
        ]);
        assert_eq!(v["values"], json!({"radius": 2, "seed": 7}));
        // 色の文字列を取る欄の null（値を外す）
        let v = args_of(&["layer.set", "--layer", "A", "--fill.Color", "null"]);
        assert_eq!(v["fill"], json!({"Color": null}));
    }

    #[test]
    fn arrays_take_repeats_commas_or_json() {
        let v = args_of(&[
            "export.channels",
            "--dir",
            "out",
            "--channels",
            "Color",
            "--channels",
            "Normal,Height",
        ]);
        assert_eq!(v["channels"], json!(["Color", "Normal", "Height"]));
        let v = args_of(&[
            "export.channels",
            "--dir",
            "out",
            "--channels",
            r#"["Color"]"#,
        ]);
        assert_eq!(v["channels"], json!(["Color"]));
    }

    #[test]
    fn json_positional_and_flags_merge_with_flags_winning() {
        let v = args_of(&[
            "layer.set",
            r#"{"layer":"A","opacity":0.1}"#,
            "--opacity",
            "0.9",
        ]);
        assert_eq!(v, json!({"layer": "A", "opacity": 0.9}));
        let v = args_of(&[
            "layer.set",
            "--args",
            r#"{"layer":"A","opacity":0.1}"#,
            "--opacity",
            "0.9",
        ]);
        assert_eq!(v, json!({"layer": "A", "opacity": 0.9}));
    }

    #[test]
    fn tool_style_names_and_dashes_are_accepted() {
        let inv = run(&["layer_set", "--layer", "A", "--blend-mode", "Multiply"]).unwrap();
        assert!(
            matches!(&inv.action, Action::Run { name, args } if name == "layer.set" && args["blend_mode"] == "Multiply")
        );
    }

    #[test]
    fn mistakes_are_refused_with_a_reason() {
        for bad in [
            vec!["layer.set", "--layr", "A"],
            vec!["layer.set", "--opacity", "high", "--layer", "A"],
            vec!["layer.set", "--visible", "maybe"],
            vec!["layer.set", "--layer"],
            vec!["no.such.command"],
            vec!["--layer", "A"],
            vec!["layer.set", "{}", "{}"],
            vec!["layer.set", "[1]"],
            vec!["--file", "a.ylp", "--live", "doc.info"],
            vec!["--save", "doc.info"],
            vec!["--lang", "fr", "doc.info"],
            vec!["--timeout", "0", "doc.info"],
            vec!["commands", "extra"],
            vec!["mcp", "--file"],
            vec!["--port", "80", "doc.info"],
            vec!["--port", "abc", "doc.info"],
        ] {
            let e = run(&bad).unwrap_err();
            assert!(
                matches!(
                    e.code,
                    yolu_ops::ErrorCode::InvalidRequest | yolu_ops::ErrorCode::UnknownCommand
                ),
                "{bad:?}: {e:?}"
            );
            assert!(
                !e.message.ja.is_empty() && !e.message.en.is_empty(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn globals_work_before_and_after_the_command() {
        let a = run(&["--file", "a.ylp", "--pretty", "doc.info"]).unwrap();
        let b = run(&["doc.info", "--file", "a.ylp", "--pretty"]).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.global.file.as_deref(), Some("a.ylp"));
        assert!(a.global.pretty);
        assert_eq!(run(&["--version"]).unwrap().action, Action::Version);
        assert_eq!(run(&[]).unwrap().action, Action::Help);
        assert_eq!(
            run(&["schema", "layer.set"]).unwrap().action,
            Action::Schema {
                name: Some("layer.set".into()),
                tools: false
            }
        );
        assert_eq!(
            run(&["schema", "--tools"]).unwrap().action,
            Action::Schema {
                name: None,
                tools: true
            }
        );
        assert_eq!(
            run(&["batch", "-"]).unwrap().action,
            Action::Batch {
                source: Some("-".into())
            }
        );
        assert_eq!(run(&["mcp"]).unwrap().action, Action::Mcp);
    }

    #[test]
    fn no_command_argument_uses_a_reserved_name() {
        for spec in commands() {
            let schema = spec.args_schema();
            let props = effective(&schema, &schema)
                .get("properties")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for key in props.keys() {
                assert!(
                    !RESERVED.contains(&key.as_str()),
                    "{} の引数 {key} が CLI の予約名と重なる",
                    spec.name
                );
            }
        }
    }
}
