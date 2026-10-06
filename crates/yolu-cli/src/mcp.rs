//! MCP サーバー（stdio）。ツールは yolu-ops の命令から作り（`tools`）、相手はツールの引数 `file` で選ぶ
//! （省くと起動中のアプリ。あれば画面なしで、その .ylp を開いて操作する）。
//!
//! - 返事は structuredContent（outputSchema どおりの JSON）と、同じ JSON の text。見本（`preview`）は、PNG を image の content と
//!   resource_link（`yolupainter://preview/<番号>.png`。直近の数枚を覚えていて、`resources/read` で読める）の両方で返す
//!   （画像を表示できないクライアントでも、リンクから取れるように）。
//! - 失敗は `isError: true` の text（誤りの JSON。`code`・日英の `message`・`data`）。壊す操作に `confirm: true` が無ければ `confirm_required`。
//! - 資料: `yolupainter://docs/<名前>`（実行ファイルに埋め込んだ、入れてある版の文書。`.ja`・`.en` で言語を指せる）、
//!   `yolupainter://ops/commands`（命令の一覧と schema）、`yolupainter://ops/effect-kinds`（効果の種類と値の範囲）。
//! - 版: rmcp が 2025-11-25 以前の `initialize` と、2026-07-28 の `server/discover`・要求ごとの `_meta` の両方を受ける。
//! - 任意のコードを実行する道具は無い。ファイルは、`file` の .ylp と、書き出し・保存の命令が指す道だけを扱う（道の決まりは yolu-ops の `PathPolicy`）。

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
    ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::{json, Value};
use yolu_ops::value::base64_encode;
use yolu_ops::{
    command_spec_by_tool, commands, parse_command, Command, ErrorCode, Lang, OpError, Reply,
};

use crate::docs;
use crate::live::{self, LiveConfig};
use crate::session::Sessions;
use crate::tools::{self, FILE_ARG};

/// 見本の PNG を覚えておく枚数と合計の大きさ。
const MAX_PREVIEWS: usize = 8;
const MAX_PREVIEW_BYTES: usize = 64 << 20;

const URI_COMMANDS: &str = "yolupainter://ops/commands";
const URI_EFFECT_KINDS: &str = "yolupainter://ops/effect-kinds";
const URI_PREVIEW_PREFIX: &str = "yolupainter://preview/";

const INSTRUCTIONS: &str = "YoluPainter texture painting. Every tool takes an optional `file` (a .ylp path). \
With `file` the project is opened and edited directly (edits stay in memory until `save` with confirm: true). Without it the tool operates on the running YoluPainter app, \
which must have \"Accept external commands\" turned on in its settings. Destructive tools (deleting, saving over a file, replacing exported files) need `confirm: true`; \
ask the user before passing it. Each editing tool is one undo step. After save_as, the project is the new file: pass the new path as `file` from then on. \
If save is refused with code conflict (the file changed outside), call doc_open with the same path as `file` and `path` and confirm: true to discard the in-memory edits and reload. Read `yolupainter://docs/guide` for the app, `yolupainter://docs/mcp` for these tools, \
and `yolupainter://ops/effect-kinds` for effect kinds and their value ranges. `preview` returns the channel image.";

/// サーバーの設定。
#[derive(Clone, Debug)]
pub struct McpOptions {
    /// 起動中のアプリへの経路。
    pub live: LiveConfig,
    /// `file` の相対パスの起点。
    pub base: PathBuf,
}

impl McpOptions {
    /// 今のフォルダを起点にした既定の設定。
    pub fn from_env() -> McpOptions {
        McpOptions {
            live: LiveConfig::default(),
            base: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

#[derive(Default)]
struct Previews {
    next: u64,
    items: VecDeque<(u64, Arc<Vec<u8>>)>,
    bytes: usize,
}

impl Previews {
    fn add(&mut self, png: Vec<u8>) -> u64 {
        self.next += 1;
        let id = self.next;
        self.bytes += png.len();
        self.items.push_back((id, Arc::new(png)));
        while self.items.len() > MAX_PREVIEWS
            || (self.bytes > MAX_PREVIEW_BYTES && self.items.len() > 1)
        {
            if let Some((_, old)) = self.items.pop_front() {
                self.bytes -= old.len();
            }
        }
        id
    }
    fn get(&self, id: u64) -> Option<Arc<Vec<u8>>> {
        self.items
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, p)| p.clone())
    }
}

struct Shared {
    options: McpOptions,
    tools: Vec<Tool>,
    sessions: Mutex<Sessions>,
    previews: Mutex<Previews>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// MCP サーバー（`ServerHandler`）。
#[derive(Clone)]
pub struct YoluMcp {
    shared: Arc<Shared>,
}

impl YoluMcp {
    pub fn new(options: McpOptions) -> YoluMcp {
        let tools = tools::tools()
            .into_iter()
            .map(|t| serde_json::from_value::<Tool>(t).expect("ツールの定義は MCP の Tool の形"))
            .collect();
        YoluMcp {
            shared: Arc::new(Shared {
                options,
                tools,
                sessions: Mutex::new(Sessions::new()),
                previews: Mutex::new(Previews::default()),
            }),
        }
    }
}

impl Shared {
    /// 命令を相手（`file` の .ylp か、起動中のアプリ）へ当てる。
    fn run(&self, file: Option<&str>, command: &Command) -> Result<Reply, OpError> {
        match file {
            Some(file) => lock(&self.sessions).run(&self.options.base, file, command),
            None => live::call(&self.options.live, command),
        }
    }

    fn success(&self, reply: Reply) -> CallToolResult {
        let mut payload = reply.payload();
        match reply {
            Reply::Preview(info) => {
                if let Some(map) = payload.as_object_mut() {
                    map.remove("png");
                }
                let png = info.png.0;
                let encoded = base64_encode(&png);
                let size = png.len() as u64;
                let id = lock(&self.previews).add(png);
                let mut result = CallToolResult::structured(payload);
                result
                    .content
                    .push(ContentBlock::image(encoded, "image/png"));
                let link = Resource::new(
                    format!("{URI_PREVIEW_PREFIX}{id}.png"),
                    format!("preview-{id}.png"),
                )
                .with_title(format!("{} {}x{}", info.channel, info.width, info.height))
                .with_mime_type("image/png")
                .with_size(size);
                result.content.push(ContentBlock::resource_link(link));
                result
            }
            _ => CallToolResult::structured(payload),
        }
    }
}

/// 失敗を、ツールの失敗の結果（`isError`）にする。中身は誤りの JSON（`code`・日英の `message`・`data`）。
pub fn error_result(error: &OpError) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(
        serde_json::to_string(error).expect("誤りは JSON にできる"),
    )])
}

fn docs_resources() -> Vec<Resource> {
    let mut out = Vec::new();
    for doc in docs::all() {
        let (text, lang) = doc.default_text();
        let title = docs::title_of(text);
        let lang_name = lang.pick("Japanese", "English");
        out.push(
            Resource::new(format!("{}{}", docs::URI_PREFIX, doc.name), doc.name)
                .with_title(title)
                .with_description(format!("YoluPainter user documentation ({lang_name})"))
                .with_mime_type("text/markdown")
                .with_size(text.len() as u64),
        );
        if lang == Lang::En {
            if let Some(ja) = doc.ja {
                out.push(
                    Resource::new(
                        format!("{}{}.ja", docs::URI_PREFIX, doc.name),
                        format!("{}.ja", doc.name),
                    )
                    .with_title(docs::title_of(ja))
                    .with_description("YoluPainter user documentation (Japanese)")
                    .with_mime_type("text/markdown")
                    .with_size(ja.len() as u64),
                );
            }
        }
    }
    out
}

fn commands_listing() -> Value {
    let list: Vec<Value> = commands()
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "tool": c.tool_name(),
                "title": c.title,
                "description": c.description,
                "read_only": c.read_only,
                "destructive": c.destructive(),
                "idempotent": c.idempotent,
                "reply": c.reply,
                "args_schema": c.args_schema(),
                "reply_schema": tools::output_schema(c),
            })
        })
        .collect();
    json!({"version": yolu_ops::COMMAND_VERSION, "commands": list})
}

fn kinds_listing() -> Value {
    serde_json::to_value(yolu_ops::doc_ops::kinds_info()).expect("効果の種類は JSON にできる")
}

impl ServerHandler for YoluMcp {
    fn get_info(&self) -> ServerConfig {
        let mut implementation = Implementation::new("yolupainter", env!("CARGO_PKG_VERSION"));
        implementation.title = Some("YoluPainter".to_owned());
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(implementation)
        .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(self.shared.tools.clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let Some(spec) = command_spec_by_tool(&request.name) else {
            return Err(McpError::invalid_params(
                format!("Unknown tool: {}", request.name),
                None,
            ));
        };
        let mut args = request.arguments.unwrap_or_default();
        let file = match args.remove(FILE_ARG) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s),
            Some(_) => {
                let e = OpError::new(
                    ErrorCode::InvalidRequest,
                    "file は .ylp の道（文字列）です",
                    "`file` must be a string path of a .ylp",
                );
                return Ok(error_result(&e).into());
            }
        };
        let command = match parse_command(&json!({"command": spec.name, "args": args})) {
            Ok(c) => c,
            Err(e) => return Ok(error_result(&e).into()),
        };
        let shared = self.shared.clone();
        let outcome =
            tokio::task::spawn_blocking(move || shared.run(file.as_deref(), &command)).await;
        let result = match outcome {
            Ok(Ok(reply)) => self.shared.success(reply),
            Ok(Err(e)) => error_result(&e),
            Err(join) => error_result(&OpError::new(
                ErrorCode::Internal,
                format!("命令の途中で想定していない失敗が起きました: {join}"),
                format!("The command stopped because of an unexpected failure: {join}"),
            )),
        };
        Ok(result.into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let mut resources = docs_resources();
        resources.push(
            Resource::new(URI_COMMANDS, "commands")
                .with_title("YoluPainter commands")
                .with_description("Every command (= MCP tool) with its title, description, danger mark and JSON Schemas")
                .with_mime_type("application/json"),
        );
        resources.push(
            Resource::new(URI_EFFECT_KINDS, "effect-kinds")
                .with_title("Effect kinds")
                .with_description("Effect kinds (filters and generators) with the name, type, range and default of every parameter (the result of effect_list_kinds)")
                .with_mime_type("application/json"),
        );
        for (id, png) in lock(&self.shared.previews).items.iter() {
            resources.push(
                Resource::new(
                    format!("{URI_PREVIEW_PREFIX}{id}.png"),
                    format!("preview-{id}.png"),
                )
                .with_title("Preview image")
                .with_mime_type("image/png")
                .with_size(png.len() as u64),
            );
        }
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let uri = request.uri;
        let not_found = || {
            McpError::resource_not_found(
                format!("Unknown resource: {uri}"),
                Some(json!({"uri": uri})),
            )
        };
        let contents = if let Some(name) = uri.strip_prefix(docs::URI_PREFIX) {
            let (_, _, text) = docs::find(name).ok_or_else(not_found)?;
            ResourceContents::text(text, &uri).with_mime_type("text/markdown")
        } else if uri == URI_COMMANDS {
            ResourceContents::text(
                serde_json::to_string(&commands_listing()).expect("JSON"),
                &uri,
            )
            .with_mime_type("application/json")
        } else if uri == URI_EFFECT_KINDS {
            ResourceContents::text(serde_json::to_string(&kinds_listing()).expect("JSON"), &uri)
                .with_mime_type("application/json")
        } else if let Some(id) = uri
            .strip_prefix(URI_PREVIEW_PREFIX)
            .and_then(|r| r.strip_suffix(".png"))
            .and_then(|n| n.parse::<u64>().ok())
        {
            let png = lock(&self.shared.previews).get(id).ok_or_else(not_found)?;
            ResourceContents::blob(base64_encode(&png), &uri).with_mime_type("image/png")
        } else {
            return Err(not_found());
        };
        Ok(ReadResourceResult::new(vec![contents]).into())
    }
}

/// stdio で MCP サーバーを動かす。クライアントが閉じるまで返らない。終了コードを返す。
pub fn serve_stdio(options: McpOptions) -> i32 {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("yolupainter-cli: cannot start the runtime: {e}");
            return 1;
        }
    };
    let code = runtime.block_on(async {
        let service = match YoluMcp::new(options).serve(rmcp::transport::stdio()).await {
            Ok(service) => service,
            Err(e) => {
                eprintln!("yolupainter-cli: the MCP server could not start: {e}");
                return 0;
            }
        };
        match service.waiting().await {
            Ok(_) => 0,
            Err(e) => {
                eprintln!("yolupainter-cli: the MCP server stopped: {e}");
                1
            }
        }
    });
    // 標準入力を待つ読みが残っていても、終わりを待たない
    runtime.shutdown_background();
    code
}
