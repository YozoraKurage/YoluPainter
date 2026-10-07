//! 外から文書を操作する命令の型と、画面なしで .ylp を操作するホスト。
//!
//! - 命令（[`Command`]）・返事（[`Reply`]）・誤り（[`OpError`]）は serde の JSON（命令は `{"command": "layer.set", "args": {...}}`）で、
//!   命令の版（[`COMMAND_VERSION`]）を持ち、JSON Schema を型から作れる（[`command_schema`]・[`commands`]）。CLI の命令と MCP のツール
//!   （起動中のアプリへの通信も MCP）は、どれもこの型を通す。
//! - 文書の命令は `yolu_core::Document` の上の関数として [`doc_ops`] の 1 か所にあり、1 つの命令が取り消しの 1 段になる。
//!   プロジェクトの命令（セットの一覧・文書の読み書き・書き出し・保存・見本）は [`OpHost`] に向けて書き、ホストは [`FileHost`]（画面なし）と
//!   起動中のアプリの中のホスト。
//! - 壊す操作（削除・上書き保存・置き換え）は `confirm: true` が無ければ断る（[`Danger`]）。任意のコードを実行する命令は無い。
//!
//! 命令の一覧・版・壊す操作・誤りは `README.md`。

pub mod command;
pub mod describe;
pub mod doc_ops;
pub mod error;
pub mod export;
pub mod file_host;
pub mod host;
pub mod meta;
pub mod path;
pub mod preview;
pub mod refs;
pub mod reply;
pub mod text;
pub mod value;
pub mod wire;

pub use command::{Command, COMMAND_VERSION};
pub use error::{ErrorCode, OpError};
pub use file_host::{newer_version_note, writer, FileHost, FileHostConfig};
pub use host::{execute, ExportJob, OpHost, SaveJob, SetView};
pub use meta::{
    command_schema, command_spec, command_spec_by_tool, commands, error_schema, reply_schema,
    CommandSpec, Danger,
};
pub use path::PathPolicy;
pub use reply::Reply;
pub use text::{Lang, Text};
pub use value::Value;
pub use wire::{parse_command, parse_command_str};

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeExamples;
