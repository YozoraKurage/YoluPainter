//! `yolupainter-cli`: YoluPainter の命令（yolu-ops）を、コマンドラインと MCP サーバー（stdio）から、
//! .ylp（画面なし）か起動中のアプリへ当てる。
//!
//! - `args`: コマンドラインの引数の読み（命令の JSON Schema に合わせて型を決める）。
//! - `cli`: 実行・出力（JSON）・終了コード。
//! - `live`: 起動中のアプリへの経路（yolu-protocol の手元の経路と yolu-ops の枠）。
//! - `mcp`・`tools`・`session`・`docs`: MCP サーバー（ツールは命令から作る・`file` の .ylp の組・埋め込んだ文書の資料）。

pub mod args;
pub mod cli;
pub mod docs;
pub mod live;
pub mod mcp;
pub mod session;
pub mod tools;
