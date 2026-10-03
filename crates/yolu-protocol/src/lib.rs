//! スタンドアロンと Unity のブリッジのあいだの通信の形（Live Link）。
//!
//! - Unity → スタンドアロン: モデル（メッシュの頂点・法線・UV0・三角形・サブメッシュ・マテリアルの組と名前と安定した鍵）、ポーズの変化
//!   （焼いたメッシュの位置）、マテリアルの情報（シェーダーとテクスチャのプロパティの名前、Unity 側が見せられるチャンネル）。
//! - スタンドアロン → Unity: テクスチャセット（マテリアル）ごと・チャンネルごとの画像の「変わったタイル」。画素は共有メモリ（`shm`）、
//!   どのタイルが変わったかは命令で知らせる。
//!
//! 命令はパイプ（`link`。Windows は名前付きパイプ、Linux は Unix のソケット）に枠（`frame`）で流す。版と知らない命令の扱いは `message`。
//! この形を読むのは Rust 同士だけ（Unity の C# はブリッジの C の関数を呼ぶ）なので、C# に同じ読み手は要らない。

// 画素（4 バイト）・数（4 バイト）を chunks_exact で回すのは読みやすさのため（yolu-core と同じ）。
#![allow(clippy::chunks_exact_to_as_chunks)]

pub mod frame;
pub mod host;
pub mod link;
pub mod message;
pub mod shm;
pub mod wire;

pub use frame::{encode_message, Frame, FrameError, FrameReader};
pub use link::{Connection, ConnectionReader, LinkError, Received, DEFAULT_LINK_NAME};
pub use message::*;
pub use shm::{ImageLayout, SharedImageReader, SharedImageWriter, ShmError, TileRead};
