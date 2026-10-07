//! `yolupainter-cli` の本体。引数を読み（`args`）、命令を .ylp（画面なし）か起動中のアプリへ当て、JSON を返す。
//! `mcp` は、標準入出力の MCP のクライアントを起動中のアプリの受け口へつなぐ中継（`relay`）を動かす。
//!
//! - 返事は標準出力の JSON（`--pretty` で整形）。失敗は標準出力に `{"error": {...}}`（`code`・日英の `message`・`data`）、標準エラーに 1 行の文、
//!   終了コードは `exit_code`（0 成功・1 命令が断った・2 引数の誤り・3 起動中のアプリにつなげない・4 確認が要る）。
//! - `--file x.ylp` は画面なしで開いて 1 回だけ当てる（`--save` で成功したら上書き保存。無ければファイルは変わらない）。`--file` が無ければ起動中のアプリ。
//! - 複数の命令を 1 回の起動でまとめて当てるなら `batch`（途中で失敗したら、保存せずに止まる）。

use std::cell::RefCell;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Map, Value};
use yolu_ops::action::{self, at_index, ActionFile};
use yolu_ops::command::{ActionRunArgs, LayerGetArgs, SaveArgs};
use yolu_ops::refs::{
    no_selection, resolve_relative, substitute_created, uses_selected, Created, SELECTED,
};
use yolu_ops::wire::command_json;
use yolu_ops::{
    command_schema, command_spec, command_spec_by_tool, commands, error_schema, execute,
    parse_command, reply_schema, Command, CommandSpec, Danger, ErrorCode, FileHost, Lang, OpError,
    OpHost, PathPolicy, Reply, COMMAND_VERSION,
};

use crate::args::{self, Action, Global, Invocation};
use crate::live::{self, LiveConfig};
use crate::relay::{self, RelayConfig};
use yolu_mcp::tools;

/// 実行した結果（標準出力・標準エラー・終了コード）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

/// 環境（今のフォルダと、言語の手がかり）。
#[derive(Clone, Debug)]
pub struct Env {
    pub cwd: PathBuf,
    /// 環境変数（`YOLUPAINTER_LANG`・`LC_ALL`・`LC_MESSAGES`・`LANG`）から分かった言語。
    pub lang: Option<Lang>,
}

impl Env {
    pub fn from_process() -> Env {
        Env {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            lang: lang_from_vars(|name| std::env::var(name).ok()),
        }
    }
}

/// 環境変数の言語（`ja`・`en`。`C`・知らない言語は手がかりにしない）。
pub fn lang_from_vars(get: impl Fn(&str) -> Option<String>) -> Option<Lang> {
    Lang::from_vars(get)
}

/// 終了コード。
pub fn exit_code(error: &OpError) -> i32 {
    match error.code {
        ErrorCode::InvalidRequest | ErrorCode::UnknownCommand | ErrorCode::UnsupportedVersion => 2,
        ErrorCode::ConfirmRequired => 4,
        _ if live::is_unreachable(error) => 3,
        _ => 1,
    }
}

fn render(value: &Value, pretty: bool) -> String {
    let mut text = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .expect("JSON");
    text.push('\n');
    text
}

fn ok(value: &Value, pretty: bool) -> Outcome {
    Outcome {
        stdout: render(value, pretty),
        stderr: String::new(),
        code: 0,
    }
}

/// 失敗。標準出力に誤りの JSON、標準エラーに文（言語が分かれば 1 つ、分からなければ日英を並べる）。
pub fn fail(error: &OpError, lang: Option<Lang>, pretty: bool) -> Outcome {
    let message = match lang {
        Some(lang) => error.message.pick(lang).to_owned(),
        None => format!("{} / {}", error.message.ja, error.message.en),
    };
    Outcome {
        stdout: render(&json!({"error": error}), pretty),
        stderr: format!(
            "yolupainter-cli: {message} [{}]\n",
            serde_json::to_value(error.code)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()
        ),
        code: exit_code(error),
    }
}

/// 引数の先読み（引数を読めなかったときの出力の整形と言語のため）。
fn scan(tokens: &[String]) -> (Option<Lang>, bool) {
    let pretty = tokens.iter().any(|t| t == "--pretty");
    let lang = tokens
        .iter()
        .position(|t| t == "--lang")
        .and_then(|i| tokens.get(i + 1))
        .or_else(|| {
            tokens
                .iter()
                .find_map(|t| t.strip_prefix("--lang=").map(|_| t))
        })
        .and_then(|t| Lang::parse(t.strip_prefix("--lang=").unwrap_or(t)));
    (lang, pretty)
}

/// 引数を実行する。`stdin` は `-`（標準入力）の読み出し。
pub fn run(tokens: &[String], env: &Env, stdin: &mut dyn Read) -> Outcome {
    let stdin = RefCell::new(stdin);
    let cwd = env.cwd.clone();
    let source = |text: &str| -> Result<String, OpError> {
        let mut out = String::new();
        if text == "-" {
            stdin
                .borrow_mut()
                .read_to_string(&mut out)
                .map_err(|e| read_error("標準入力", "standard input", &e))?;
        } else if let Some(path) = text.strip_prefix('@') {
            let path = resolve(&cwd, path);
            out = std::fs::read_to_string(&path).map_err(|e| {
                read_error(&path.display().to_string(), &path.display().to_string(), &e)
            })?;
        } else {
            out = text.to_owned();
        }
        Ok(out)
    };
    let (scan_lang, scan_pretty) = scan(tokens);
    let invocation = match args::parse(tokens, &source) {
        Ok(i) => i,
        Err(e) => return fail(&e, scan_lang.or(env.lang), scan_pretty),
    };
    let Invocation { global, action } = invocation;
    let lang = global.lang.or(env.lang);
    let pretty = global.pretty;
    // `--cwd` は、相対パスの起点（`--file`・書き出し先）を替える
    let env = match &global.cwd {
        Some(folder) => {
            let cwd = resolve(&env.cwd, folder);
            if !cwd.is_dir() {
                let e = OpError::invalid_request(
                    format!("--cwd のフォルダがありません: {}", cwd.display()),
                    format!("The --cwd folder does not exist: {}", cwd.display()),
                );
                return fail(&e, lang, pretty);
            }
            Env {
                cwd,
                lang: env.lang,
            }
        }
        None => env.clone(),
    };
    match execute_action(&global, action, &env, &source) {
        Ok(outcome) => outcome,
        Err(e) => fail(&e, lang, pretty),
    }
}

fn read_error(what_ja: &str, what_en: &str, e: &std::io::Error) -> OpError {
    OpError::invalid_request(
        format!("{what_ja} を読めません: {e}"),
        format!("Cannot read {what_en}: {e}"),
    )
}

fn resolve(cwd: &Path, text: &str) -> PathBuf {
    let p = Path::new(text);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

fn kind_of(spec: &CommandSpec) -> &'static str {
    if spec.read_only {
        "read"
    } else {
        match spec.danger {
            Danger::Safe => "edit",
            Danger::Always => "destructive",
            Danger::WhenReplacing => "replace",
            Danger::PerCommand => "per_command",
        }
    }
}

fn commands_listing() -> Value {
    let list: Vec<Value> = commands()
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "tool": c.tool_name(),
                "kind": kind_of(c),
                "reply": c.reply,
                "title": c.title,
                "description": c.description,
            })
        })
        .collect();
    json!({"version": COMMAND_VERSION, "commands": list})
}

fn schema_output(name: Option<&str>, tools_only: bool) -> Result<Value, OpError> {
    if tools_only {
        return Ok(json!({"tools": tools::tools()}));
    }
    match name {
        None => Ok(json!({
            "version": COMMAND_VERSION,
            "command": command_schema(),
            "reply": reply_schema(),
            "error": error_schema(),
        })),
        Some(name) => {
            let spec = command_spec(name)
                .or_else(|| command_spec_by_tool(name))
                .ok_or_else(|| {
                    OpError::new(
                        ErrorCode::UnknownCommand,
                        format!("知らない命令: {name}"),
                        format!("Unknown command: {name}"),
                    )
                    .with_data(
                        json!({"commands": commands().iter().map(|c| c.name).collect::<Vec<_>>()}),
                    )
                })?;
            Ok(json!({
                "name": spec.name,
                "tool": spec.tool_name(),
                "kind": kind_of(spec),
                "title": spec.title,
                "description": spec.description,
                "args": spec.args_schema(),
                "reply": spec.reply_schema(),
            }))
        }
    }
}

/// 命令を当てる相手。
enum Target {
    File(Box<FileHost>),
    Live(LiveConfig),
}

impl Target {
    fn open(global: &Global, env: &Env) -> Result<Target, OpError> {
        match &global.file {
            Some(file) => {
                let policy = PathPolicy::new(&env.cwd).map_err(|e| {
                    OpError::new(
                        ErrorCode::Io,
                        format!("作業のフォルダを扱えません: {e}"),
                        format!("Cannot use the working folder: {e}"),
                    )
                })?;
                let mut host = FileHost::new(policy);
                host.open(&resolve(&env.cwd, file), true)?;
                Ok(Target::File(Box::new(host)))
            }
            None => Ok(Target::Live(live_config(global))),
        }
    }

    fn execute(&mut self, command: &Command) -> Result<Reply, OpError> {
        match self {
            Target::File(host) => execute(host.as_mut(), command),
            Target::Live(config) => live::call(config, command),
        }
    }
}

/// 起動中のアプリへのつなぎ先（`--port`・`--timeout`）。
fn live_config(global: &Global) -> LiveConfig {
    let mut live = LiveConfig::default();
    if let Some(port) = global.port {
        live.port = port;
    }
    if let Some(secs) = global.timeout_secs {
        live.timeout = Duration::from_secs_f64(secs);
    }
    live
}

fn execute_action(
    global: &Global,
    action: Action,
    env: &Env,
    source: args::Source<'_>,
) -> Result<Outcome, OpError> {
    let pretty = global.pretty;
    match action {
        Action::Help => Ok(Outcome {
            stdout: usage(global.lang.or(env.lang)).to_owned(),
            stderr: String::new(),
            code: 0,
        }),
        Action::Version => Ok(Outcome {
            stdout: format!(
                "yolupainter-cli {} (command version {COMMAND_VERSION})\n",
                env!("CARGO_PKG_VERSION")
            ),
            stderr: String::new(),
            code: 0,
        }),
        Action::Commands => Ok(ok(&commands_listing(), pretty)),
        Action::Schema { name, tools } => Ok(ok(&schema_output(name.as_deref(), tools)?, pretty)),
        Action::Mcp => {
            if global.file.is_some() || global.save || global.out.is_some() {
                return Err(OpError::invalid_request(
                    "mcp は起動中のアプリへの中継です。--file・--save・--out は使えません",
                    "mcp relays to the running app; --file, --save and --out cannot be used with it",
                ));
            }
            let live = live_config(global);
            let code = relay::serve_stdio(RelayConfig {
                port: live.port,
                timeout: live.timeout,
            });
            Ok(Outcome {
                stdout: String::new(),
                stderr: String::new(),
                code,
            })
        }
        Action::Run { name, args } => {
            let command = parse_command(&json!({"command": name, "args": Value::Object(args)}))?;
            let mut target = Target::open(global, env)?;
            let reply = target.execute(&command)?;
            let mut value = finish_reply(&reply, global, env)?;
            if global.save {
                let saved = save(&mut target)?;
                if let Value::Object(map) = &mut value {
                    map.insert("saved".into(), saved);
                }
            }
            Ok(ok(&value, pretty))
        }
        Action::Batch { source: input } => {
            if global.out.is_some() {
                return Err(OpError::invalid_request(
                    "--out は batch には使えません",
                    "--out cannot be used with batch",
                ));
            }
            let text = source(input.as_deref().unwrap_or("-"))?;
            let commands = parse_batch(&text)?;
            let mut target = Target::open(global, env)?;
            let mut replies: Vec<Value> = Vec::new();
            // `$created:<n>` は、この batch で作ったレイヤー・効果（送る前に ID へ替える）
            let mut created = Created::default();
            // 起動中のアプリへは 1 つずつ送るので、`$selected` を相手に任せると命令ごとに引き直される（途中の削除で選びが隣へ移る）。
            // アクションと同じに、始めた時の 1 つを先に聞いて ID に決める
            let live = matches!(target, Target::Live(_));
            let selected = if live {
                start_selected(&mut target, &commands)?
            } else {
                Vec::new()
            };
            for (index, command) in commands.iter().enumerate() {
                let set = command.set().map(str::to_owned);
                let resolved = if live {
                    resolve_relative(command, Some(&created), &mut || {
                        selected
                            .iter()
                            .find(|(chosen_set, _)| *chosen_set == set)
                            .map(|(_, id)| id.clone())
                            .ok_or_else(|| no_selection(None))
                    })
                    .map(|command_resolved| command_resolved.unwrap_or_else(|| command.clone()))
                } else {
                    substitute_created(command, &created)
                };
                let reply = resolved
                    .and_then(|command| {
                        let reply = target.execute(&command)?;
                        created.note(&command, &reply);
                        Ok(reply)
                    })
                    .map_err(|e| at_index(e, index, command, replies.len()))?;
                replies.push(finish_reply(
                    &reply,
                    &Global {
                        out: None,
                        ..global.clone()
                    },
                    env,
                )?);
            }
            let mut output = Map::new();
            output.insert("replies".into(), Value::Array(replies));
            if global.save {
                output.insert("saved".into(), save(&mut target)?);
            }
            Ok(ok(&Value::Object(output), pretty))
        }
        Action::RunAction { path } => {
            if global.out.is_some() {
                return Err(OpError::invalid_request(
                    "--out は run-action には使えません",
                    "--out cannot be used with run-action",
                ));
            }
            let file = read_action(&resolve(&env.cwd, &path))?;
            let mut target = Target::open(global, env)?;
            // 画面なしの .ylp にも起動中のアプリにも、命令 `action.run` の 1 回で当てる（全部で取り消しの 1 段。途中で断れば全部戻る）
            let command = Command::ActionRun(ActionRunArgs {
                commands: file.commands.iter().map(command_json).collect(),
            });
            let reply = target.execute(&command)?;
            if !matches!(reply, Reply::Action(_)) {
                return Err(OpError::new(
                    ErrorCode::Internal,
                    format!("action.run の返事が {} でした", reply.name()),
                    format!("action.run replied with {}", reply.name()),
                ));
            }
            // 返事（`reply: "action"`・`set`・`steps`・`undo_count` …）に、アクションの名前を添える
            let mut output = Map::new();
            output.insert("action".into(), json!(file.name));
            if let Value::Object(fields) = serde_json::to_value(&reply).expect("JSON") {
                output.extend(fields);
            }
            if global.save {
                output.insert("saved".into(), save(&mut target)?);
            }
            Ok(ok(&Value::Object(output), pretty))
        }
    }
}

/// 起動中のアプリへ 1 つずつ送る batch の、始めた時の `$selected`（命令の `set` ごと）。`$selected` を使う命令があるときだけ、
/// アプリに `layer.get` で聞く（読むだけで何も変えない）。選んでいるレイヤーが無ければ、何も変える前に、最初に使う命令の番号つきで断る。
fn start_selected(
    target: &mut Target,
    commands: &[Command],
) -> Result<Vec<(Option<String>, String)>, OpError> {
    let mut chosen: Vec<(Option<String>, String)> = Vec::new();
    for (index, command) in commands.iter().enumerate() {
        if !uses_selected(command) {
            continue;
        }
        let set = command.set().map(str::to_owned);
        if chosen.iter().any(|(known, _)| *known == set) {
            continue;
        }
        let ask = Command::LayerGet(LayerGetArgs {
            set: set.clone(),
            layer: SELECTED.to_owned(),
        });
        let id = match target.execute(&ask) {
            Ok(Reply::Layer(info)) => info.summary.id,
            Ok(other) => {
                return Err(OpError::new(
                    ErrorCode::Internal,
                    format!("layer.get の返事が {} でした", other.name()),
                    format!("layer.get replied with {}", other.name()),
                ))
            }
            Err(e) => return Err(at_index(e, index, command, 0)),
        };
        chosen.push((set, id));
    }
    Ok(chosen)
}

/// アクションのファイルを読む（大きさを先に確かめる）。
fn read_action(path: &Path) -> Result<ActionFile, OpError> {
    let display = path.display().to_string();
    let meta = std::fs::metadata(path).map_err(|e| read_error(&display, &display, &e))?;
    if meta.len() > action::MAX_FILE_BYTES {
        return Err(OpError::new(
            ErrorCode::Budget,
            format!(
                "アクションのファイルが大きすぎます（{} MiB まで）: {display}",
                action::MAX_FILE_BYTES >> 20
            ),
            format!(
                "The action file is too large (at most {} MiB): {display}",
                action::MAX_FILE_BYTES >> 20
            ),
        ));
    }
    let text = std::fs::read_to_string(path).map_err(|e| read_error(&display, &display, &e))?;
    action::parse_action(&text)
}

fn save(target: &mut Target) -> Result<Value, OpError> {
    let reply = target.execute(&Command::Save(SaveArgs { confirm: true }))?;
    Ok(serde_json::to_value(&reply).expect("JSON"))
}

/// 返事を JSON にする。`--out` があれば、見本の PNG をその道へ書いて JSON には道を出す。
fn finish_reply(reply: &Reply, global: &Global, env: &Env) -> Result<Value, OpError> {
    let mut value = serde_json::to_value(reply).expect("返事は JSON にできる");
    match (&global.out, reply) {
        (Some(out), Reply::Preview(info)) => {
            let path = resolve(&env.cwd, out);
            write_png(&path, &info.png.0)?;
            if let Value::Object(map) = &mut value {
                map.remove("png");
                map.insert("png_file".into(), json!(path.display().to_string()));
            }
        }
        (Some(_), _) => {
            return Err(OpError::invalid_request(
                "--out は preview の PNG を書くときだけ使えます",
                "--out is only for writing the PNG of preview",
            ))
        }
        _ => {}
    }
    Ok(value)
}

/// 隣の一時ファイルに書いてから置き換える（途中で止まっても、書きかけの PNG を残さない）。
fn write_png(path: &Path, bytes: &[u8]) -> Result<(), OpError> {
    let io = |e: &std::io::Error| {
        OpError::new(
            ErrorCode::Io,
            format!("{} へ書けません: {e}", path.display()),
            format!("Cannot write {}: {e}", path.display()),
        )
    };
    let temporary = path.with_extension(format!("png-{}.tmp", std::process::id()));
    std::fs::write(&temporary, bytes).map_err(|e| io(&e))?;
    std::fs::rename(&temporary, path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        io(&e)
    })
}

/// `batch` の入力。JSON の配列、または 1 行 1 命令（空行と `#` で始まる行は読み飛ばす）。
pub fn parse_batch(text: &str) -> Result<Vec<Command>, OpError> {
    let trimmed = text.trim_start();
    let values: Vec<Value> = if trimmed.starts_with('[') {
        match serde_json::from_str::<Value>(trimmed) {
            Ok(Value::Array(items)) => items,
            Ok(_) => unreachable!("[ で始まる JSON は配列"),
            Err(e) => {
                return Err(OpError::invalid_request(
                    format!("batch の JSON を読めません: {e}"),
                    format!("Cannot read the batch JSON: {e}"),
                ))
            }
        }
    } else {
        let mut items = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            items.push(serde_json::from_str::<Value>(line).map_err(|e| {
                OpError::invalid_request(
                    format!("batch の {} 行目を読めません: {e}", n + 1),
                    format!("Cannot read line {} of the batch: {e}", n + 1),
                )
            })?);
        }
        items
    };
    if values.is_empty() {
        return Err(OpError::invalid_request(
            "batch に命令がありません",
            "The batch has no commands",
        ));
    }
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            parse_command(v).map_err(|mut e| {
                e.message.ja = format!("batch の {} 番目: {}", i + 1, e.message.ja);
                e.message.en = format!("batch item {}: {}", i + 1, e.message.en);
                e
            })
        })
        .collect()
}

const USAGE_EN: &str = "\
yolupainter-cli - operate YoluPainter projects from the command line, and relay MCP to the running app

Usage:
  yolupainter-cli <command> [--name value ...] [--file project.ylp [--save]] [--pretty]
  yolupainter-cli batch [file|-] --file project.ylp [--save]
  yolupainter-cli run-action <action.json> [--file project.ylp [--save]]
  yolupainter-cli commands          list every command
  yolupainter-cli schema [command]  JSON Schema of the commands (--tools: MCP tool definitions)
  yolupainter-cli mcp               relay MCP on stdio to the running app (http://127.0.0.1:<port>/mcp)

Commands are named like layer.set (or layer_set). Pass arguments as flags (--layer Base --opacity 0.5,
--values.radius 4, --confirm) or as one JSON object ('{\"layer\":\"Base\"}', @file.json, or - for stdin).

Target:
  --file <x.ylp>   open the project without the app; edits are lost unless --save is given
  --save           save the .ylp in place after the command succeeded (with --file)
  (no --file)      the running YoluPainter (Settings: Accept external commands)
  --port <number>  the port set in the app (default 17347)
Options:
  --pretty  --lang ja|en  --timeout <seconds>  --out <png>  --cwd <folder>  --version  --help

The reply is JSON on stdout. On failure stdout has {\"error\": ...}, stderr one line, and the exit code is
1 refused, 2 bad arguments, 3 the app cannot be reached, 4 confirmation needed (--confirm).
";

const USAGE_JA: &str = "\
yolupainter-cli - YoluPainter のプロジェクトをコマンドラインから操作し、MCP を起動中のアプリへ中継する

使い方:
  yolupainter-cli <命令> [--名前 値 ...] [--file project.ylp [--save]] [--pretty]
  yolupainter-cli batch [ファイル|-] --file project.ylp [--save]
  yolupainter-cli run-action <アクション.json> [--file project.ylp [--save]]
  yolupainter-cli commands          命令の一覧
  yolupainter-cli schema [命令]     命令の JSON Schema（--tools は MCP のツールの定義）
  yolupainter-cli mcp               標準入出力の MCP を起動中のアプリ（http://127.0.0.1:<番号>/mcp）へ中継する

命令の名前は layer.set（か layer_set）の形です。引数は --layer Base --opacity 0.5・--values.radius 4・--confirm のように渡すか、
JSON のオブジェクト 1 つ（'{\"layer\":\"Base\"}'・@file.json・- は標準入力）で渡します。

相手:
  --file <x.ylp>   アプリなしでプロジェクトを開く（--save が無ければ、編集はファイルに入りません）
  --save           命令が成功したら .ylp に上書き保存する（--file のとき）
  （--file なし）  起動中の YoluPainter（設定「外からの操作を受ける」を入れておく）
  --port <番号>    アプリの設定の番号（既定は 17347）
オプション:
  --pretty  --lang ja|en  --timeout <秒>  --out <png>  --cwd <フォルダ>  --version  --help

返事は標準出力の JSON です。失敗は標準出力に {\"error\": ...}、標準エラーに 1 行、終了コードは
1 命令が断った・2 引数の誤り・3 起動中のアプリにつなげない・4 確認が要る（--confirm）です。
";

fn usage(lang: Option<Lang>) -> &'static str {
    match lang {
        Some(Lang::Ja) => USAGE_JA,
        _ => USAGE_EN,
    }
}

/// 標準入力（読むたびに錠を取る）。`stdin().lock()` を持ち続けると、MCP の中継の標準入力の読み（tokio が同じ錠を使う）が止まる。
struct LazyStdin;

impl Read for LazyStdin {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::io::stdin().read(buf)
    }
}

/// 実行ファイルの入口。標準出力・標準エラーへ書き、終了コードを返す。
pub fn main_with(tokens: Vec<String>) -> i32 {
    use std::io::Write;
    let env = Env::from_process();
    let outcome = run(&tokens, &env, &mut LazyStdin);
    let _ = std::io::stdout().write_all(outcome.stdout.as_bytes());
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().write_all(outcome.stderr.as_bytes());
    outcome.code
}
