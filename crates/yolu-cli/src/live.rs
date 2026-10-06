//! 起動中のアプリへ命令を送る。経路は yolu-protocol の手元の経路（Live Link とは別の名前 `yolupainter-ops`・別の鍵のファイル）で、
//! つないだ直後の挨拶（鍵の HMAC の確かめ合い・版の取り決め）は Live Link と同じ。そのあと、yolu-ops の枠（要求 `0x4f50`・返事 `0x4f51`）を流す。
//!
//! 1 回の呼び出しごとにつなぎ直す（つながりを持ち続けない）。アプリが再起動しても、設定を切って入れ直しても、次の呼び出しはその時の
//! アプリへつながる。つなげない理由（アプリが起きていない・設定が切・鍵が合わない）は、直し方を言う誤りにする。

use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::json;
use yolu_ops::link::{decode_response, encode_request, read_frame, Received, Request, KIND_RESPONSE, LINK_NAME};
use yolu_ops::{Command, ErrorCode, OpError, Reply};
use yolu_protocol::link::{connect_and_greet_within, LinkError, HANDSHAKE_TIMEOUT};
use yolu_protocol::{Identity, Kind};

/// 経路の名前を環境変数で替えるときの名前（アプリの受け口と同じ）。
pub use yolu_ops::link::LINK_NAME_ENV;
/// 返事を待つ長さの既定。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// つなぎ先。
#[derive(Clone, Debug)]
pub struct LiveConfig {
    pub name: String,
    pub timeout: Duration,
}

impl Default for LiveConfig {
    fn default() -> Self {
        LiveConfig {
            name: std::env::var(LINK_NAME_ENV)
                .ok()
                .filter(|n| yolu_protocol::link::valid_link_name(n))
                .unwrap_or_else(|| LINK_NAME.to_owned()),
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// つなげない・つながりが切れたことを知らせる誤り（CLI の終了コード 3。`data.live` が `unreachable`）。
fn unreachable(ja: impl Into<String>, en: impl Into<String>, reason: &str) -> OpError {
    OpError::new(ErrorCode::Io, ja, en).with_data(json!({"live": "unreachable", "reason": reason}))
}

/// 起動中のアプリにつなげないときの誤りか。
pub fn is_unreachable(error: &OpError) -> bool {
    error
        .data
        .as_ref()
        .and_then(|d| d.get("live"))
        .and_then(|v| v.as_str())
        == Some("unreachable")
}

fn not_listening(detail: &str) -> OpError {
    unreachable(
        "起動中の YoluPainter につなげません。アプリを起動し、設定の「外からの操作を受ける」を入れてください（画面なしで .ylp を操作するなら file を指定します）",
        "Cannot reach a running YoluPainter. Start the app and turn on \"Accept external commands\" in its settings (or pass file to operate on a .ylp without the app)",
        detail,
    )
}

fn map_link_error(error: LinkError) -> OpError {
    match error {
        LinkError::Io(e) => match e.kind() {
            std::io::ErrorKind::NotFound
            | std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::BrokenPipe => not_listening(&e.to_string()),
            _ => unreachable(
                format!("起動中の YoluPainter との通信に失敗しました: {e}"),
                format!("Communication with the running YoluPainter failed: {e}"),
                &e.to_string(),
            ),
        },
        LinkError::Rejected(reject) => OpError::new(
            ErrorCode::Refused,
            format!("起動中の YoluPainter が接続を断りました: {}", reject.text),
            format!(
                "The running YoluPainter refused the connection: {}",
                reject.text
            ),
        )
        .with_data(json!({"reason": reject.text})),
        LinkError::Untrusted(why) => unreachable(
            format!("つないだ相手を信用できません（{why}）"),
            format!("The peer on the link cannot be trusted ({why})"),
            &why,
        ),
        other => unreachable(
            format!("起動中の YoluPainter との通信に失敗しました: {other}"),
            format!("Communication with the running YoluPainter failed: {other}"),
            &other.to_string(),
        ),
    }
}

/// 要求を 1 つ送って返事を待つ（つなぎ直す）。時間切れの待ちは別のスレッドで見張る（Windows の名前付きパイプには読みの時間切れが無い）。
pub fn call(config: &LiveConfig, command: &Command) -> Result<Reply, OpError> {
    let (tx, rx) = mpsc::channel();
    let name = config.name.clone();
    let timeout = config.timeout;
    let command_name = command.name();
    let command = command.clone();
    let worker = std::thread::Builder::new()
        .name("yolupainter-ops-call".into())
        .spawn(move || {
            let _ = tx.send(exchange(&name, &command, timeout));
        })
        .map_err(|e| {
            OpError::new(
                ErrorCode::Io,
                format!("通信のスレッドを作れません: {e}"),
                format!("Cannot start the link thread: {e}"),
            )
        })?;
    match rx.recv_timeout(timeout + HANDSHAKE_TIMEOUT) {
        Ok(result) => {
            let _ = worker.join();
            result
        }
        // 見張りの時間が来ても、読んでいるスレッドは相手が閉じるまで残る（強く止める道は無い）。待たずに返す
        Err(_) => Err(OpError::new(
            ErrorCode::Busy,
            format!("起動中の YoluPainter から {} 秒以内に返事がありません（操作が終わったかは分かりません）", timeout.as_secs()),
            format!("No reply from the running YoluPainter within {} s (it is unknown whether the command ran)", timeout.as_secs()),
        )
        .with_data(json!({"command": command_name}))),
    }
}

fn exchange(name: &str, command: &Command, timeout: Duration) -> Result<Reply, OpError> {
    let identity = Identity::unity("yolupainter-cli");
    let (connection, mut reader, _welcome) =
        connect_and_greet_within(name, &identity, HANDSHAKE_TIMEOUT).map_err(map_link_error)?;
    let id = 1;
    let frame = encode_request(&Request {
        id,
        command: command.clone(),
    })?;
    connection
        .send_frame(&frame)
        .map_err(|e| map_link_error(LinkError::Io(e)))?;
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(OpError::new(
                ErrorCode::Busy,
                "起動中の YoluPainter の返事を待つ時間が尽きました（操作が終わったかは分かりません）",
                "Ran out of time waiting for the running YoluPainter (it is unknown whether the command ran)",
            ));
        }
        reader.set_timeout(Some(left.min(Duration::from_millis(500))));
        let (frames, stream) = reader.raw();
        let mut stream = stream;
        let received = read_frame(frames, &mut stream).map_err(|e| {
            unreachable(
                format!("YoluPainter とのつながりが途中で切れました（操作が終わったかは分かりません）: {}", e.message.ja),
                format!("The link to YoluPainter broke (it is unknown whether the command ran): {}", e.message.en),
                "closed",
            )
        })?;
        match received {
            Received::Frame(frame) if frame.kind == Kind::Bye as u16 => {
                // 受け口を切った（設定「外からの操作を受ける」を切った・アプリを閉じる）。返事は来ない
                return Err(unreachable(
                    "起動中の YoluPainter が外からの操作の受け付けをやめました（操作が終わったかは分かりません）",
                    "The running YoluPainter stopped accepting external commands (it is unknown whether the command ran)",
                    "closed",
                ));
            }
            Received::Frame(frame) => {
                if frame.kind != KIND_RESPONSE {
                    return Err(unreachable(
                        format!("起動中の YoluPainter が操作の返事でない枠を返しました（種類 {:#x}）。アプリが古い版かもしれません", frame.kind),
                        format!("The running YoluPainter sent a frame that is not an operation reply (kind {:#x}); the app may be an older version", frame.kind),
                        "unexpected frame",
                    ));
                }
                let response = decode_response(&frame)?;
                if response.id != id {
                    continue;
                }
                return response.outcome;
            }
            Received::Idle => continue,
            Received::Closed => {
                return Err(unreachable(
                    "返事の前に YoluPainter がつながりを閉じました（操作が終わったかは分かりません）",
                    "YoluPainter closed the link before replying (it is unknown whether the command ran)",
                    "closed",
                ))
            }
        }
    }
}
