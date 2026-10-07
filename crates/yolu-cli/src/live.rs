//! 起動中のアプリへ命令を送る: アプリの MCP の受け口（`http://127.0.0.1:<番号>/mcp`）へ、`tools/call` を 1 回。
//!
//! 受け口は状態を持たない（要求ごとに独立）ので、`initialize` はせず、版の頭（`MCP-Protocol-Version`）を付けた要求 1 つで足りる。
//! 1 回の呼び出しごとにつなぎ直す（アプリが起動し直しても、設定を切って入れ直しても、次の呼び出しはその時のアプリへつながる）。
//! つなげない理由（アプリが起きていない・設定が切・番号が違う）は、直し方を言う誤りにする（終了コード 3）。

use std::time::Duration;

use serde_json::{json, Value};
use yolu_mcp::client::{self, ClientError, Exchange, HEADER_PROTOCOL_VERSION, ONE_SHOT_VERSION};
use yolu_mcp::reach;
use yolu_ops::wire::command_json;
use yolu_ops::{command_spec, Command, CommandSpec, ErrorCode, OpError, Reply};

pub use yolu_mcp::reach::is_unreachable;
/// 返事を待つ長さの既定。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// つなぎ先。
#[derive(Clone, Debug)]
pub struct LiveConfig {
    /// アプリの設定「外からの操作を受ける」の番号。
    pub port: u16,
    pub timeout: Duration,
}

impl Default for LiveConfig {
    fn default() -> Self {
        LiveConfig {
            port: yolu_mcp::DEFAULT_PORT,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// 要求を 1 つ送って返事を待つ。
pub fn call(config: &LiveConfig, command: &Command) -> Result<Reply, OpError> {
    let spec = command_spec(command.name()).expect("命令は一覧にある");
    let arguments = command_json(command)
        .get("args")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let message = client::tool_call(1, &spec.tool_name(), arguments);
    let headers = [(
        HEADER_PROTOCOL_VERSION.to_owned(),
        ONE_SHOT_VERSION.to_owned(),
    )];
    let body = serde_json::to_vec(&message).expect("JSON");
    let port = config.port;
    match client::post_blocking(port, &headers, body, config.timeout) {
        Err(ClientError::Timeout) => {
            Err(reach::no_reply(config.timeout).with_data(json!({"command": command.name()})))
        }
        Err(ClientError::Connect(e)) => Err(reach::not_listening(port, &e.to_string())),
        Err(ClientError::Broken(e)) => Err(reach::unreachable(
            format!(
                "YoluPainter とのつながりが途中で切れました（操作が終わったかは分かりません）: {e}"
            ),
            format!(
                "The connection to YoluPainter broke (it is unknown whether the command ran): {e}"
            ),
            "closed",
        )),
        Ok(exchange) => interpret(spec, &exchange, port),
    }
}

/// HTTP の返事を、命令の返事か誤りにする。
fn interpret(spec: &CommandSpec, exchange: &Exchange, port: u16) -> Result<Reply, OpError> {
    match exchange.status {
        200 => {}
        reach::BUSY_STATUS => return Err(reach::too_many_connections()),
        status => {
            let body = String::from_utf8_lossy(&exchange.body);
            return Err(reach::unreachable(
                format!("{port} 番で待っているのが YoluPainter の MCP の受け口ではありません（HTTP {status}: {body}）"),
                format!("What listens on port {port} is not the MCP endpoint of YoluPainter (HTTP {status}: {body})"),
                "http",
            ));
        }
    }
    let unreadable = |detail: &str| {
        reach::unreachable(
            format!("YoluPainter の返事を読めません: {detail}"),
            format!("The reply of YoluPainter cannot be read: {detail}"),
            "unreadable",
        )
    };
    let messages = client::messages(exchange).map_err(|e| unreadable(&e))?;
    let response = messages
        .into_iter()
        .find(|m| m.get("id") == Some(&json!(1)))
        .ok_or_else(|| unreadable("no response to the request"))?;
    if let Some(error) = response.get("error") {
        let text = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        return Err(OpError::new(
            ErrorCode::InvalidRequest,
            format!("起動中の YoluPainter が要求を断りました: {text}"),
            format!("The running YoluPainter refused the request: {text}"),
        )
        .with_data(error.clone()));
    }
    let result = response
        .get("result")
        .ok_or_else(|| unreadable("no result"))?;
    client::reply_of(spec, result)
}
