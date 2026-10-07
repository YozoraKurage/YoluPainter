//! `yolupainter-cli mcp`: 標準入出力の MCP のクライアント（Claude Desktop の拡張など）を、起動中のアプリの MCP の受け口
//! （`http://127.0.0.1:<番号>/mcp`）へつなぐ中継。1 行 1 メッセージの JSON-RPC を読み、そのまま `POST /mcp` で送り、返事のメッセージを
//! 1 行ずつ書く。ツールの一覧・中身・資料・版の取り決めは持たない（アプリが答えるので、アプリの更新だけで新しくなる）。
//!
//! - Streamable HTTP の客の決まりだけは守る: `initialize` で決まった版を、以後の要求の頭（`MCP-Protocol-Version`）に付ける。
//!   2026-07-28 の流れ（要求ごとの `_meta`）は、その版と `Mcp-Method`・`Mcp-Name` の頭を付ける。
//! - 要求は並べて送る（長い保存の間も、読む要求は先に返る）。同時に送るのは [`IN_FLIGHT`] まで（アプリの受け口の上限を 1 つの中継で埋めない）。
//!   `initialize` だけは、返事を受けてから次を読む（決まった版を次の要求に付けるため）。
//! - アプリにつなげない・返事が無いときは、要求（`id` のあるもの）に理由と直し方を返す: `tools/call` はツールの失敗（`isError` と誤りの JSON。
//!   アプリの誤りと同じ形）、ほかは JSON-RPC の誤り。通知（`id` が無い）は捨てる。
//! - 標準入力が閉じたら、送りかけの要求の返事を書き終えてから終わる。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use yolu_mcp::client::{self, ClientError, META_PROTOCOL_VERSION};
use yolu_mcp::reach;
use yolu_ops::OpError;

/// 同時に送る要求の数。
pub const IN_FLIGHT: usize = 4;

/// 中継の決まり。
#[derive(Clone, Copy, Debug)]
pub struct RelayConfig {
    pub port: u16,
    /// 1 つの要求の返事を待つ長さ。
    pub timeout: Duration,
}

/// 書き出し口（1 行ずつ、混ざらないように）。
type Out<W> = Arc<tokio::sync::Mutex<W>>;

async fn write_line<W: AsyncWrite + Unpin>(out: &Out<W>, message: &Value) {
    let mut line = serde_json::to_vec(message).expect("JSON");
    line.push(b'\n');
    let mut out = out.lock().await;
    let _ = out.write_all(&line).await;
    let _ = out.flush().await;
}

/// 中継できなかった要求への返事。
fn failure(message: &Value, error: &OpError) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    if method == "tools/call" {
        let mut result = json!({
            "content": [{"type": "text", "text": serde_json::to_string(error).expect("JSON")}],
            "isError": true,
        });
        // 2026-07-28 の流れの結果は、種類（resultType）を持つ
        if message
            .pointer("/params/_meta")
            .and_then(|m| m.get(META_PROTOCOL_VERSION))
            .is_some()
        {
            result["resultType"] = json!("complete");
        }
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    } else {
        Some(json!({"jsonrpc": "2.0", "id": id, "error": {
            "code": -32000,
            "message": error.message.en,
            "data": error,
        }}))
    }
}

/// 1 つのメッセージを送り、返事のメッセージを返す（中継できなければ、要求への誤りの返事）。
async fn forward(config: RelayConfig, message: Value, negotiated: Option<String>) -> Vec<Value> {
    let headers = client::mcp_headers(&message, negotiated.as_deref());
    let body = serde_json::to_vec(&message).expect("JSON");
    let outcome =
        tokio::time::timeout(config.timeout, client::post(config.port, &headers, body)).await;
    let error = match outcome {
        Err(_) | Ok(Err(ClientError::Timeout)) => reach::no_reply(config.timeout),
        Ok(Err(ClientError::Connect(e))) => reach::not_listening(config.port, &e.to_string()),
        Ok(Err(ClientError::Broken(e))) => reach::unreachable(
            format!(
                "YoluPainter とのつながりが途中で切れました（操作が終わったかは分かりません）: {e}"
            ),
            format!(
                "The connection to YoluPainter broke (it is unknown whether the command ran): {e}"
            ),
            "closed",
        ),
        // つながりの数が上限（本文は JSON でない文）。直結の客（`live.rs`）と同じ `busy` にして、頼み直しを促す
        Ok(Ok(exchange)) if exchange.status == reach::BUSY_STATUS => reach::too_many_connections(),
        Ok(Ok(exchange)) => {
            match client::messages(&exchange) {
                // 202（通知を受けた）は返事なし。JSON-RPC の誤り（400 など）も、本文のメッセージをそのまま返す
                Ok(messages) if !messages.is_empty() || exchange.status == 202 => return messages,
                Ok(_) | Err(_) => {
                    let body = String::from_utf8_lossy(&exchange.body).into_owned();
                    let status = exchange.status;
                    reach::unreachable(
                    format!("YoluPainter の受け口が要求を受けませんでした（HTTP {status}: {body}）"),
                    format!("The YoluPainter endpoint did not take the request (HTTP {status}: {body})"),
                    "http",
                )
                }
            }
        }
    };
    failure(&message, &error).into_iter().collect()
}

/// 標準入出力の代わりの流れで中継する（試験は流れを差し替える）。入力が閉じたら、送りかけの返事を書いてから返る。
pub async fn run<R, W>(config: RelayConfig, input: R, output: W)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let out: Out<W> = Arc::new(tokio::sync::Mutex::new(output));
    let negotiated: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let permits = Arc::new(Semaphore::new(IN_FLIGHT));
    let mut tasks = JoinSet::new();
    let mut lines = BufReader::new(input).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                write_line(&out, &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("Parse error: {e}")}})).await;
                continue;
            }
        };
        let version = negotiated.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if message.get("method").and_then(Value::as_str) == Some("initialize") {
            // 決まった版を覚えてから次を読む
            let replies = forward(config, message.clone(), version).await;
            for reply in &replies {
                if reply.get("id") == message.get("id") {
                    if let Some(v) = reply
                        .pointer("/result/protocolVersion")
                        .and_then(Value::as_str)
                    {
                        *negotiated.lock().unwrap_or_else(|e| e.into_inner()) = Some(v.to_owned());
                    }
                }
                write_line(&out, reply).await;
            }
            continue;
        }
        let Ok(permit) = permits.clone().acquire_owned().await else {
            break;
        };
        let out = out.clone();
        tasks.spawn(async move {
            let _permit = permit;
            for reply in forward(config, message, version).await {
                write_line(&out, &reply).await;
            }
        });
        // 終わった物を溜めない
        while tasks.try_join_next().is_some() {}
    }
    while tasks.join_next().await.is_some() {}
}

/// 標準入出力で中継する。クライアントが入力を閉じるまで返らない。終了コードを返す。
pub fn serve_stdio(config: RelayConfig) -> i32 {
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
    runtime.block_on(run(config, tokio::io::stdin(), tokio::io::stdout()));
    // 標準入力を待つ読みが残っていても、終わりを待たない
    runtime.shutdown_background();
    0
}
