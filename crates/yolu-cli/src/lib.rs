//! `yolupainter-cli`: YoluPainter の命令（yolu-ops）を、コマンドラインから .ylp（画面なし）か起動中のアプリへ当てる。
//! `yolupainter-cli mcp` は、標準入出力の MCP のクライアントを、起動中のアプリの MCP の受け口（`http://127.0.0.1:<番号>/mcp`）へつなぐ中継。
//!
//! - `args`: コマンドラインの引数の読み（命令の JSON Schema に合わせて型を決める）。
//! - `cli`: 実行・出力（JSON）・終了コード。
//! - `live`: 起動中のアプリへの命令（MCP の `tools/call` を 1 回）。
//! - `relay`: 標準入出力 ⇔ アプリの HTTP の中継（ツールの中身を持たない）。

pub mod args;
pub mod cli;
pub mod live;
pub mod relay;
