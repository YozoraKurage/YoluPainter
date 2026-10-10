//! 文書の編集の束: レイヤー・レイヤーの操作・選択範囲・コピーとペースト・履歴・チャンネル・色調補正・書き出し・2D の合成。
//!
//! 1 本の実行ファイルに束ねて、試験のファイルの数だけビルドとリンクが増えないようにしている。束の中の試験は同じプロセスで同時に走る
//! （`RUST_TEST_THREADS` の分だけ）。プロセス全体の状態を変える試験は束に入れず、`tests/` の直下に 1 ファイル 1 本で置く（理由はそのファイルの頭）。
//! 新しい試験は、内容に近い束のフォルダにファイルを置き、この `main.rs` に `mod` を 1 行足す（`edit/bundle_layout.rs` が足し忘れを見つける）。
//! 共通の部品は下で 1 度だけ宣言し、各ファイルは `use crate::<部品>;` で使う。
#[path = "../comp2d_support/mod.rs"]
mod comp2d_support;

mod adjust_kinds;
mod anti_alias;
mod batch;
mod bundle_layout;
mod channels;
mod clipboard;
mod comp2d;
mod docops;
mod export;
mod history;
mod import_tiles;
mod layers;
mod merge_compare;
mod nesting;
mod ramp_mixing;
mod rulers;
mod selection;
mod simd_document;
mod text_layer;
mod warp;
