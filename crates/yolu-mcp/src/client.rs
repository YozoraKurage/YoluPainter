//! 受け口（[`crate::http`]）への小さな客。HTTP/1.1 で、1 回の要求ごとにつなぎ直す（つながりを持ち続けない。アプリを起こし直しても、
//! 設定を切って入れ直しても、次の要求はその時のアプリへつながる）。
//!
//! - [`post`]: JSON-RPC のメッセージ 1 つを `POST /mcp` で送り、返事（JSON か SSE）を受ける。
//! - [`mcp_headers`]: Streamable HTTP の客が付ける頭（`MCP-Protocol-Version`。2026-07-28 からは `Mcp-Method`・`Mcp-Name` も）。
//! - [`messages`]: 返事の本文から JSON-RPC のメッセージを取り出す（`application/json` は 1 つ、`text/event-stream` は `data:` ごと）。
//! - [`reply_of`]: `tools/call` の結果を、yolu-ops の返事（`Reply`）か誤り（`OpError`）に戻す（`server` が作った形の逆）。

use std::io;
use std::net::Ipv4Addr;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::header::{ACCEPT, CONTENT_TYPE, HOST};
use hyper::Request;
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use yolu_ops::value::base64_encode;
use yolu_ops::{CommandSpec, ErrorCode, OpError, Reply};

use crate::PATH;

/// 客が受けられる返事の形（rmcp の受け口は両方を受けられることを求める）。
pub const ACCEPT_BOTH: &str = "application/json, text/event-stream";
/// 頭の名前。
pub const HEADER_PROTOCOL_VERSION: &str = "MCP-Protocol-Version";
pub const HEADER_METHOD: &str = "Mcp-Method";
pub const HEADER_NAME: &str = "Mcp-Name";
/// 要求の `_meta` の版の欄（2026-07-28 の流れ）。
pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
/// `Mcp-Method`・`Mcp-Name` を付ける版（この版から受け口が求める）。
const STANDARD_HEADERS_FROM: &str = "2026-07-28";
/// `initialize` をせずに `tools/call` を 1 回だけ送るとき（コマンドラインの起動中のアプリへの命令）に名乗る版。
pub const ONE_SHOT_VERSION: &str = "2025-11-25";

/// 客の失敗。
#[derive(Debug)]
pub enum ClientError {
    /// つなげない（待っていない・番号が違う）。
    Connect(io::Error),
    /// つながったが、途中で切れた・HTTP として読めない。
    Broken(String),
    /// 決めた時間のうちに返事が来ない。
    Timeout,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Connect(e) => write!(f, "cannot connect: {e}"),
            ClientError::Broken(e) => write!(f, "the connection broke: {e}"),
            ClientError::Timeout => write!(f, "no reply in time"),
        }
    }
}

/// HTTP の返事。
#[derive(Clone, Debug)]
pub struct Exchange {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Bytes,
}

/// `POST /mcp` を 1 回（127.0.0.1 の番号へ）。`headers` は足す頭。
pub async fn post(
    port: u16,
    headers: &[(String, String)],
    body: Vec<u8>,
) -> Result<Exchange, ClientError> {
    let stream = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(ClientError::Connect)?;
    let _ = stream.set_nodelay(true);
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|e| ClientError::Broken(e.to_string()))?;
    let driver = tokio::spawn(connection);
    let mut request = Request::post(PATH)
        .header(HOST, format!("127.0.0.1:{port}"))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, ACCEPT_BOTH);
    for (name, value) in headers {
        request = request.header(name.as_str(), value.as_str());
    }
    let request = request
        .body(Full::new(Bytes::from(body)))
        .map_err(|e| ClientError::Broken(e.to_string()))?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|e| ClientError::Broken(e.to_string()))?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = response
        .into_body()
        .collect()
        .await
        .map_err(|e| ClientError::Broken(e.to_string()))?
        .to_bytes();
    driver.abort();
    Ok(Exchange {
        status,
        content_type,
        body,
    })
}

/// [`post`] を、その場で作った実行基盤で待つ（同期の呼び手: コマンドラインの命令・試験）。
pub fn post_blocking(
    port: u16,
    headers: &[(String, String)],
    body: Vec<u8>,
    timeout: std::time::Duration,
) -> Result<Exchange, ClientError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| ClientError::Broken(format!("cannot start the runtime: {e}")))?;
    runtime.block_on(async {
        tokio::time::timeout(timeout, post(port, headers, body))
            .await
            .unwrap_or(Err(ClientError::Timeout))
    })
}

/// 頭の値に入れられない文字（制御文字・ASCII の外・前後の空白）を含むか。含めば `=?base64?…?=` で包む（MCP の SEP-2243 の決まり）。
fn header_value(text: &str) -> String {
    let needs = text.starts_with([' ', '\t'])
        || text.ends_with([' ', '\t'])
        || text.chars().any(|c| !(' '..='~').contains(&c))
        || (text.starts_with("=?base64?") && text.ends_with("?="));
    if needs {
        format!("=?base64?{}?=", base64_encode(text.as_bytes()))
    } else {
        text.to_owned()
    }
}

/// JSON-RPC のメッセージに付ける頭。版は、メッセージの `_meta` にあればそれ、無ければ `negotiated`（`initialize` で決まった版）。
pub fn mcp_headers(message: &Value, negotiated: Option<&str>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let version = message
        .pointer("/params/_meta")
        .and_then(|m| m.get(META_PROTOCOL_VERSION))
        .and_then(Value::as_str)
        .or(negotiated);
    let is_initialize = message.get("method").and_then(Value::as_str) == Some("initialize");
    // initialize は版を本文で言う（頭を付けるなら本文と同じ版でなければならないので、付けない）
    if is_initialize {
        return out;
    }
    let Some(version) = version else {
        return out;
    };
    out.push((HEADER_PROTOCOL_VERSION.to_owned(), version.to_owned()));
    if version >= STANDARD_HEADERS_FROM {
        if let Some(method) = message.get("method").and_then(Value::as_str) {
            out.push((HEADER_METHOD.to_owned(), header_value(method)));
            let key = match method {
                "tools/call" | "prompts/get" => Some("name"),
                "resources/read" | "resources/subscribe" | "resources/unsubscribe" => Some("uri"),
                "tasks/get" | "tasks/update" | "tasks/cancel" => Some("taskId"),
                _ => None,
            };
            if let Some(name) = key
                .and_then(|k| message.get("params").and_then(|p| p.get(k)))
                .and_then(Value::as_str)
            {
                out.push((HEADER_NAME.to_owned(), header_value(name)));
            }
        }
    }
    out
}

/// 返事の本文から JSON-RPC のメッセージを取り出す。
pub fn messages(exchange: &Exchange) -> Result<Vec<Value>, String> {
    let text = std::str::from_utf8(&exchange.body).map_err(|e| e.to_string())?;
    let kind = exchange.content_type.as_deref().unwrap_or("");
    if kind.starts_with("text/event-stream") {
        let mut out = Vec::new();
        let mut data: Vec<&str> = Vec::new();
        let flush = |data: &mut Vec<&str>, out: &mut Vec<Value>| -> Result<(), String> {
            if !data.is_empty() {
                let joined = data.join("\n");
                data.clear();
                if !joined.trim().is_empty() {
                    out.push(serde_json::from_str(&joined).map_err(|e| e.to_string())?);
                }
            }
            Ok(())
        };
        for line in text.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.is_empty() {
                flush(&mut data, &mut out)?;
            } else if let Some(rest) = line.strip_prefix("data:") {
                data.push(rest.strip_prefix(' ').unwrap_or(rest));
            }
        }
        flush(&mut data, &mut out)?;
        Ok(out)
    } else if text.trim().is_empty() {
        Ok(Vec::new())
    } else {
        Ok(vec![serde_json::from_str(text).map_err(|e| e.to_string())?])
    }
}

/// `tools/call` の要求。
pub fn tool_call(id: u64, tool: &str, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": tool, "arguments": arguments}})
}

fn unreadable(detail: impl std::fmt::Display) -> OpError {
    OpError::new(
        ErrorCode::Internal,
        format!("YoluPainter の返事を読めません: {detail}"),
        format!("The reply of YoluPainter cannot be read: {detail}"),
    )
}

/// `tools/call` の結果（JSON-RPC の `result`）を、命令の返事か誤りに戻す。成功は structuredContent に返事の種類（`reply`）を足し、
/// 見本は画像の content を `png` に戻す。失敗（`isError`）は text の誤りの JSON。
pub fn reply_of(spec: &CommandSpec, result: &Value) -> Result<Reply, OpError> {
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        let text = content
            .iter()
            .find_map(|c| c.get("text").and_then(Value::as_str))
            .unwrap_or_default();
        return Err(serde_json::from_str::<OpError>(text)
            .unwrap_or_else(|_| OpError::new(ErrorCode::Internal, text, text)));
    }
    let mut payload = result
        .get("structuredContent")
        .cloned()
        .ok_or_else(|| unreadable("structuredContent is missing"))?;
    let Value::Object(map) = &mut payload else {
        return Err(unreadable("structuredContent is not an object"));
    };
    map.insert("reply".into(), json!(spec.reply));
    if spec.name == "preview" {
        let image = content
            .iter()
            .find(|c| c.get("type").and_then(Value::as_str) == Some("image"))
            .and_then(|c| c.get("data"))
            .cloned()
            .ok_or_else(|| unreadable("the preview has no image"))?;
        map.insert("png".into(), image);
    }
    serde_json::from_value(payload).map_err(unreadable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_follow_the_version_of_each_message() {
        let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25"}});
        assert!(
            mcp_headers(&init, None).is_empty(),
            "initialize は本文で版を言う"
        );
        let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}});
        assert!(
            mcp_headers(&list, None).is_empty(),
            "版が分からなければ付けない"
        );
        assert_eq!(
            mcp_headers(&list, Some("2025-11-25")),
            [(HEADER_PROTOCOL_VERSION.to_owned(), "2025-11-25".to_owned())]
        );
        let call = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
            "name": "layer_get", "arguments": {}, "_meta": {META_PROTOCOL_VERSION: "2026-07-28"}}});
        assert_eq!(
            mcp_headers(&call, Some("2025-11-25")),
            [
                (HEADER_PROTOCOL_VERSION.to_owned(), "2026-07-28".to_owned()),
                (HEADER_METHOD.to_owned(), "tools/call".to_owned()),
                (HEADER_NAME.to_owned(), "layer_get".to_owned()),
            ],
            "_meta の版が先"
        );
        let read = json!({"jsonrpc": "2.0", "id": 4, "method": "resources/read", "params": {"uri": "yolupainter://docs/ガイド"}});
        let headers = mcp_headers(&read, Some("2026-07-28"));
        assert_eq!(headers[2].0, HEADER_NAME);
        assert!(
            headers[2].1.starts_with("=?base64?"),
            "ASCII の外は base64 で包む"
        );
    }

    #[test]
    fn json_and_event_stream_bodies_give_their_messages() {
        let json_reply = Exchange {
            status: 200,
            content_type: Some("application/json".into()),
            body: Bytes::from_static(br#"{"jsonrpc":"2.0","id":1,"result":{}}"#),
        };
        assert_eq!(
            messages(&json_reply).unwrap(),
            [json!({"jsonrpc":"2.0","id":1,"result":{}})]
        );
        let sse = Exchange {
            status: 200,
            content_type: Some("text/event-stream".into()),
            body: Bytes::from_static(
                b"retry: 3000\r\n\r\nevent: message\r\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"n\"}\r\n\r\ndata: {\"jsonrpc\":\"2.0\",\r\ndata: \"id\":2,\"result\":{}}\n\n",
            ),
        };
        assert_eq!(
            messages(&sse).unwrap(),
            [
                json!({"jsonrpc":"2.0","method":"n"}),
                json!({"jsonrpc":"2.0","id":2,"result":{}})
            ]
        );
        let accepted = Exchange {
            status: 202,
            content_type: None,
            body: Bytes::new(),
        };
        assert!(messages(&accepted).unwrap().is_empty());
    }
}
