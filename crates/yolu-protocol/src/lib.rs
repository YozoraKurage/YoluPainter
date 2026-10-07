//! モデルの形と、Live Link のファイルの受け渡し。
//!
//! - `model`: メッシュ・マテリアルの組・ポーズの型（`yolu_protocol::Model` など）。アプリの中で 3D ビューとテクスチャセットの結びつけが
//!   共有する。通信の形ではない。
//! - `files`: Live Link の受け渡し。Unity が頼みの JSON を `inbox/` に置き、スタンドアロンが拾う。書き出したら返事の JSON を `outbox/` に
//!   置き、Unity が拾う。送るのはファイルの道と小さな値だけで、スタンドアロンが FBX と絵のファイルを自分で読む。
//! - `private`: 自分だけが読み書きできるフォルダとファイル（受け渡しのフォルダが使う）。

pub mod files;
pub mod model;
pub mod private;

pub use model::*;
