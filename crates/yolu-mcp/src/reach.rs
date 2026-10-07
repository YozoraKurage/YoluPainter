//! 起動中のアプリにつなげない・返事が来ないことを言う誤り。コマンドラインはこれを終了コード 3 にし（`data.live` が `unreachable`）、
//! MCP の中継は JSON-RPC の誤りの文にする。アプリは、受け付けをやめたときに残っていた要求へ [`stopped`] を返す。

use std::time::Duration;

use serde_json::json;
use yolu_ops::{ErrorCode, OpError};

/// つなげない・つながりが切れた誤り（`data.live == "unreachable"`）。
pub fn unreachable(ja: impl Into<String>, en: impl Into<String>, reason: &str) -> OpError {
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

/// 番号で待っているアプリが無い（起きていない・設定が切・番号が違う）。直し方を言う。
pub fn not_listening(port: u16, detail: &str) -> OpError {
    unreachable(
        format!("起動中の YoluPainter につなげません（{port} 番）。アプリを起動し、設定の「外からの操作を受ける」を入れてください（番号を変えたなら、同じ番号を指定します）"),
        format!("Cannot reach a running YoluPainter on port {port}. Start the app and turn on \"Accept external commands\" in its settings (if you changed the port there, use the same port here)"),
        detail,
    )
}

/// 返事を待っている間に、アプリが外からの操作の受け付けをやめた（設定を切った・アプリを閉じる）。
pub fn stopped() -> OpError {
    unreachable(
        "起動中の YoluPainter が外からの操作の受け付けをやめました（操作が終わったかは分かりません）",
        "The running YoluPainter stopped accepting external commands (it is unknown whether the command ran)",
        "closed",
    )
}

/// アプリの受け口が、つながりの数の上限（[`crate::http::MAX_CONNECTIONS`]）を超えたつながりへ返す HTTP の状態。
pub const BUSY_STATUS: u16 = 503;

/// アプリの受け口のつながりの数が上限で断られた（少し待って頼み直せば通る）。コマンドラインと中継が同じ誤り（`busy`）にする。
pub fn too_many_connections() -> OpError {
    OpError::new(
        ErrorCode::Busy,
        "起動中の YoluPainter へのつながりの数が上限です。少し待ってから、もう一度頼みます",
        "Too many connections to the running YoluPainter; wait a moment and try again",
    )
}

/// 決めた時間のうちに返事が来ない。
pub fn no_reply(timeout: Duration) -> OpError {
    OpError::new(
        ErrorCode::Busy,
        format!("起動中の YoluPainter から {} 秒以内に返事がありません（操作が終わったかは分かりません）", timeout.as_secs_f64()),
        format!("No reply from the running YoluPainter within {} s (it is unknown whether the command ran)", timeout.as_secs_f64()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_busy_status_is_the_one_the_listener_answers_with() {
        assert_eq!(hyper::StatusCode::SERVICE_UNAVAILABLE.as_u16(), BUSY_STATUS);
        let error = too_many_connections();
        assert_eq!(error.code, ErrorCode::Busy);
        assert!(
            !is_unreachable(&error),
            "つながれないのではなく、混んでいる"
        );
    }
}
