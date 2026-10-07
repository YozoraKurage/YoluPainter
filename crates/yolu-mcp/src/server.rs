//! MCP サーバー（rmcp の `ServerHandler`）。ツールは yolu-ops の命令から作り（`tools`）、命令は [`Backend`] へ渡して実行する
//! （起動中のアプリでは画面のスレッド。相手はいつもアプリが開いている文書）。
//!
//! - 返事は structuredContent（outputSchema どおりの JSON）と、同じ JSON の text。見本（`preview`）は、PNG を image の content と
//!   resource_link（`yolupainter://preview/<番号>.png`。直近の数枚を覚えていて、`resources/read` で読める）の両方で返す
//!   （画像を表示できないクライアントでも、リンクから取れるように）。
//! - 失敗は `isError: true` の text（誤りの JSON。`code`・日英の `message`・`data`）。壊す操作に `confirm: true` が無ければ `confirm_required`。
//! - 資料: `yolupainter://docs/<名前>`（実行ファイルに埋め込んだ、入れてある版の文書。`.ja`・`.en` で言語を指せる）、
//!   `yolupainter://ops/commands`（命令の一覧と schema）、`yolupainter://ops/effect-kinds`（効果の種類と値の範囲）。
//! - 版: rmcp が 2025-11-25 以前の `initialize` と、2026-07-28 の `server/discover`・要求ごとの `_meta` の両方を受ける。
//! - 任意のコードを実行する道具は無い。ファイルは、書き出し・保存の命令が指す道だけを扱う（道の決まりは yolu-ops の `PathPolicy` と相手のホスト）。

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
    ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::{json, Value};
use yolu_ops::value::base64_encode;
use yolu_ops::{command_spec_by_tool, commands, parse_command, Command, Lang, OpError, Reply};

use crate::docs;
use crate::tools;

/// 見本の PNG を覚えておく枚数と合計の大きさ。
const MAX_PREVIEWS: usize = 8;
const MAX_PREVIEW_BYTES: usize = 64 << 20;

pub const URI_COMMANDS: &str = "yolupainter://ops/commands";
pub const URI_EFFECT_KINDS: &str = "yolupainter://ops/effect-kinds";
pub const URI_PREVIEW_PREFIX: &str = "yolupainter://preview/";

const INSTRUCTIONS: &str = "YoluPainter texture painting. The tools operate on the project open in the running YoluPainter app \
(its \"Accept external commands\" setting is on, or this server would not answer). Destructive tools (deleting, saving over a file, replacing exported files) need `confirm: true`; \
ask the user before passing it. Each editing tool is one step of the app's undo history, and the user sees every change in the app. \
Tools are refused with code busy while the user is drawing or a save is running: wait and retry. Read `yolupainter://docs/guide` for the app, `yolupainter://docs/mcp` for these tools, \
and `yolupainter://ops/effect-kinds` for effect kinds and their value ranges. `preview` returns the channel image.";

/// 命令を実行する相手の返事（非同期。画面のスレッドが答えるまで待つ）。
pub type BackendFuture = Pin<Box<dyn Future<Output = Result<Reply, OpError>> + Send>>;

/// 命令を実行する相手（起動中のアプリ・試験の画面なしのホスト）。1 つの命令は取り消しの 1 段。
pub trait Backend: Send + Sync + 'static {
    fn run(&self, command: Command) -> BackendFuture;
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
    backend: Arc<dyn Backend>,
    tools: Vec<Tool>,
    previews: Mutex<Previews>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// MCP サーバー（`ServerHandler`）。複製は同じ相手と同じ見本の記憶を持つ（要求ごとに複製を作る HTTP の受け口でも、見本のリンクが続く）。
#[derive(Clone)]
pub struct YoluMcp {
    shared: Arc<Shared>,
}

impl YoluMcp {
    pub fn new(backend: Arc<dyn Backend>) -> YoluMcp {
        let tools = tools::tools()
            .into_iter()
            .map(|t| serde_json::from_value::<Tool>(t).expect("ツールの定義は MCP の Tool の形"))
            .collect();
        YoluMcp {
            shared: Arc::new(Shared {
                backend,
                tools,
                previews: Mutex::new(Previews::default()),
            }),
        }
    }
}

impl Shared {
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
        let args = request.arguments.unwrap_or_default();
        let command = match parse_command(&json!({"command": spec.name, "args": args})) {
            Ok(c) => c,
            Err(e) => return Ok(error_result(&e).into()),
        };
        let result = match self.shared.backend.run(command).await {
            Ok(reply) => self.shared.success(reply),
            Err(e) => error_result(&e),
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
