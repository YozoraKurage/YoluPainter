//! YoluPainter の MCP サーバーの中身と、手元の HTTP の受け口・客。
//!
//! - `tools`: ツールの定義（名前・題・説明・inputSchema・outputSchema・注釈）を yolu-ops の命令の一覧から作る。
//! - `docs`: 使う人向けの文書（`docs/`）を埋め込み、資料 `yolupainter://docs/<名前>` にする。
//! - `server`: rmcp の `ServerHandler`（ツールの一覧と実行・資料・見本の画像）。命令を実行する相手は [`Backend`] で受ける
//!   （起動中のアプリは画面のスレッドへ渡し、試験は画面なしのホストへ当てる）。
//! - `http`: 127.0.0.1 だけで待つ Streamable HTTP の受け口（hyper の HTTP/1.1。rmcp に渡す前に Host・Origin を確かめ、
//!   同時のつながりの数を絞る）。
//! - `client`: 同じ受け口への小さな客（コマンドラインの中継・起動中のアプリへの命令・試験）。
//! - `reach`: 起動中のアプリにつなげない・返事が来ないことを言う誤り（コマンドラインの終了コード 3）。

pub mod client;
pub mod docs;
pub mod http;
pub mod reach;
pub mod server;
pub mod tools;

pub use server::{Backend, BackendFuture, YoluMcp};

/// 既定の番号。IANA の登録で割り当てが無く、OS が送り元に使う一時の番号の範囲（Linux の既定 32768〜60999、Windows の既定 49152〜65535）の外。
pub const DEFAULT_PORT: u16 = 17347;
/// 設定で選べる番号の範囲（1024 より下は OS の特権が要る）。
pub const MIN_PORT: u16 = 1024;
/// MCP の受け口の道。
pub const PATH: &str = "/mcp";

/// つなぐ側に渡す URL（`http://127.0.0.1:<番号>/mcp`）。
pub fn endpoint(port: u16) -> String {
    format!("http://127.0.0.1:{port}{PATH}")
}

/// 番号として使えるか（1024〜65535）。
pub fn valid_port(port: u16) -> bool {
    port >= MIN_PORT
}
