//! 起動中のアプリへの通信の枠: 要求（`Request`）と返事（`Response`）の JSON と、手元の経路で運ぶ枠（yolu-protocol の `YLNK` の枠）の
//! 種類。**ここは枠の型と読み書きだけ**で、経路そのもの（アプリの中の待ち受け・CLI のつなぎ・鍵の確かめ合い）は別に実装する。
//!
//! - 名前の決まり: 経路は yolu-protocol の手元の経路（Unix のソケット・Windows の名前付きパイプ・鍵のファイルの HMAC の確かめ合い）を、
//!   Live Link とは**別の名前**（[`LINK_NAME`]）で使う。名前が違えば、ソケット・パイプも鍵のファイルも別で、Live Link の鍵では入れない。
//! - 要求は `{"v":1,"id":7,"command":"layer.set","args":{...}}`、返事は `{"v":1,"id":7,"ok":true,"reply":{...}}` か
//!   `{"v":1,"id":7,"ok":false,"error":{...}}`。`id` は返事を要求に結ぶ番号（呼び手が決める）。`v` は命令の版で、違えば断る。
//! - 枠の中身は 1 つの JSON。大きさの上限は [`MAX_PAYLOAD`]（見本の画像が載る。これを超える返事は送らず、誤りにする）。

use std::io::{Read, Write};

use serde_json::{json, Value};
use yolu_protocol::frame::{try_encode_frame, Fill, Frame, FrameError, FrameReader};

use crate::command::{Command, COMMAND_VERSION};
use crate::error::{ErrorCode, OpError};
use crate::reply::Reply;
use crate::wire::{check_version, parse_command};

/// 手元の経路の名前（Live Link の `yolu_protocol::DEFAULT_LINK_NAME` とは別）。
pub const LINK_NAME: &str = "yolupainter-ops";
/// 経路の名前を替える環境変数（アプリの受け口と CLI・MCP が同じ名前を読む。試験・2 つのアプリを並べるとき）。
pub const LINK_NAME_ENV: &str = "YOLUPAINTER_OPS_NAME";
/// 枠の種類: 要求。
pub const KIND_REQUEST: u16 = 0x4f50;
/// 枠の種類: 返事。
pub const KIND_RESPONSE: u16 = 0x4f51;
/// 枠の中身の上限（64 MiB）。
pub const MAX_PAYLOAD: usize = 64 << 20;

/// 要求。
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub id: u64,
    pub command: Command,
}

/// 返事。成功は `Reply`、失敗は `OpError`。
#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    pub id: u64,
    pub outcome: Result<Reply, OpError>,
}

impl Request {
    pub fn to_json(&self) -> Value {
        let mut value = crate::wire::command_json(&self.command);
        if let Value::Object(map) = &mut value {
            map.insert("id".into(), json!(self.id));
        }
        value
    }
    pub fn from_json(value: &Value) -> Result<Request, OpError> {
        let id = value.get("id").and_then(Value::as_u64).ok_or_else(|| {
            OpError::invalid_request(
                "id（要求の番号）がありません",
                "`id` (the request number) is missing",
            )
        })?;
        // 要求の枠は版を必ず持つ（命令の JSON は省けるが、通信では取り違えを避ける）
        if value.get("v").is_none() {
            return Err(OpError::new(
                ErrorCode::UnsupportedVersion,
                "要求に版（v）がありません",
                "The request has no version (`v`)",
            ));
        }
        Ok(Request {
            id,
            command: parse_command(value)?,
        })
    }
}

impl Response {
    pub fn ok(id: u64, reply: Reply) -> Response {
        Response {
            id,
            outcome: Ok(reply),
        }
    }
    pub fn err(id: u64, error: OpError) -> Response {
        Response {
            id,
            outcome: Err(error),
        }
    }
    pub fn to_json(&self) -> Value {
        match &self.outcome {
            Ok(reply) => json!({"v": COMMAND_VERSION, "id": self.id, "ok": true, "reply": reply}),
            Err(error) => json!({"v": COMMAND_VERSION, "id": self.id, "ok": false, "error": error}),
        }
    }
    pub fn from_json(value: &Value) -> Result<Response, OpError> {
        check_version(Some(value.get("v").ok_or_else(|| {
            OpError::invalid_request(
                "返事に版（v）がありません",
                "The response has no version (`v`)",
            )
        })?))?;
        let bad = |what: &str| {
            OpError::invalid_request(
                format!("返事の {what} が正しくありません"),
                format!("The response's {what} is invalid"),
            )
        };
        let id = value
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| bad("id"))?;
        let outcome = match value.get("ok").and_then(Value::as_bool) {
            Some(true) => Ok(serde_json::from_value(
                value.get("reply").cloned().ok_or_else(|| bad("reply"))?,
            )
            .map_err(|e| {
                OpError::invalid_request(
                    format!("返事の reply が正しくありません: {e}"),
                    format!("The response's reply is invalid: {e}"),
                )
            })?),
            Some(false) => Err(serde_json::from_value(
                value.get("error").cloned().ok_or_else(|| bad("error"))?,
            )
            .map_err(|e| {
                OpError::invalid_request(
                    format!("返事の error が正しくありません: {e}"),
                    format!("The response's error is invalid: {e}"),
                )
            })?),
            None => return Err(bad("ok")),
        };
        Ok(Response { id, outcome })
    }
}

fn frame_error(e: &FrameError) -> OpError {
    OpError::new(
        ErrorCode::Io,
        format!("通信の枠を扱えません: {e}"),
        format!("The link frame cannot be handled: {e}"),
    )
}

fn encode(kind: u16, value: &Value) -> Result<Vec<u8>, OpError> {
    let payload = serde_json::to_vec(value).expect("JSON は文字列にできる");
    if payload.len() > MAX_PAYLOAD {
        return Err(OpError::new(
            ErrorCode::Budget,
            format!(
                "通信の中身が大きすぎます（{} MiB。上限 {} MiB）",
                payload.len().div_ceil(1 << 20),
                MAX_PAYLOAD >> 20
            ),
            format!(
                "The message is too large ({} MiB; the limit is {} MiB)",
                payload.len().div_ceil(1 << 20),
                MAX_PAYLOAD >> 20
            ),
        ));
    }
    try_encode_frame(kind, 0, &payload).map_err(|e| frame_error(&e))
}

/// 要求を枠（頭とバイト列）にする。
pub fn encode_request(request: &Request) -> Result<Vec<u8>, OpError> {
    encode(KIND_REQUEST, &request.to_json())
}

/// 返事を枠にする。上限を超える返事は、送れない理由の誤りの返事に替える（相手に区切りを失わせない）。
pub fn encode_response(response: &Response) -> Vec<u8> {
    match encode(KIND_RESPONSE, &response.to_json()) {
        Ok(bytes) => bytes,
        Err(error) => encode(KIND_RESPONSE, &Response::err(response.id, error).to_json())
            .expect("小さな誤りの返事は上限に収まる"),
    }
}

fn decode_json(frame: &Frame, kind: u16, what: &str) -> Result<Value, OpError> {
    if frame.kind != kind {
        return Err(OpError::invalid_request(
            format!("{what}の枠ではありません（種類 {:#x}）", frame.kind),
            format!("Not a {what} frame (kind {:#x})", frame.kind),
        ));
    }
    if frame.payload.len() > MAX_PAYLOAD {
        return Err(OpError::new(
            ErrorCode::Budget,
            "通信の中身が大きすぎます",
            "The message is too large",
        ));
    }
    serde_json::from_slice(&frame.payload).map_err(|e| {
        OpError::invalid_request(
            format!("{what}の JSON を読めません: {e}"),
            format!("Cannot read the {what} JSON: {e}"),
        )
    })
}

pub fn decode_request(frame: &Frame) -> Result<Request, OpError> {
    Request::from_json(&decode_json(frame, KIND_REQUEST, "要求")?)
}

pub fn decode_response(frame: &Frame) -> Result<Response, OpError> {
    Response::from_json(&decode_json(frame, KIND_RESPONSE, "返事")?)
}

/// [`read_frame`] の結果。
#[derive(Debug, PartialEq, Eq)]
pub enum Received {
    /// 枠が 1 つそろった。
    Frame(Frame),
    /// 読みの時間切れ（`WouldBlock`・`TimedOut`・`Interrupted`）で、枠がそろわなかった。つながりは続いていて、読んだ分は
    /// `FrameReader` に残るので、同じ `FrameReader` でもう一度呼ぶ。
    Idle,
    /// 枠の切れ目で、相手がきれいに閉じた（読みかけのバイトは無い）。
    Closed,
}

/// 流れ（ソケット・パイプ）から枠を 1 つ読む。時間切れ（[`Received::Idle`]）と、枠の切れ目での閉じ（[`Received::Closed`]）は別に返す。
/// 枠の途中で相手が閉じたら、要求が消えたことを黙って流さず、誤りにする。合言葉・長さが正しくない流れも誤り。
pub fn read_frame(reader: &mut FrameReader, stream: &mut impl Read) -> Result<Received, OpError> {
    loop {
        if let Some(frame) = reader.next_frame().map_err(|e| frame_error(&e))? {
            return Ok(Received::Frame(frame));
        }
        match reader.fill(stream).map_err(|e| frame_error(&e))? {
            Fill::Read(_) => {}
            Fill::Idle => return Ok(Received::Idle),
            Fill::Closed if reader.pending() == 0 => return Ok(Received::Closed),
            Fill::Closed => {
                let got = reader.pending();
                return Err(OpError::new(
                    ErrorCode::Io,
                    format!("枠の途中で相手が閉じました（{got} バイトを受けたところ）"),
                    format!(
                        "The peer closed the link in the middle of a frame ({got} bytes received)"
                    ),
                ));
            }
        }
    }
}

/// 枠を流れへ書く。
pub fn write_bytes(stream: &mut impl Write, bytes: &[u8]) -> Result<(), OpError> {
    stream
        .write_all(bytes)
        .and_then(|()| stream.flush())
        .map_err(|e| {
            OpError::new(
                ErrorCode::Io,
                format!("通信へ書けません: {e}"),
                format!("Cannot write to the link: {e}"),
            )
        })
}
