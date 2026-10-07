//! ブラシの取り込みの束: 同梱の筆先・GBR・GIH・VBR・ABR・PAT・PNG・CLIP STUDIO（`.sut`・C2F）と、信頼できないファイル。
//!
//! 1 本の実行ファイルに束ねて、試験のファイルの数だけビルドとリンクが増えないようにしている。束の中の試験は同じプロセスで同時に走る
//! （`RUST_TEST_THREADS` の分だけ）。プロセス全体の状態を変える試験は束に入れず、`tests/` の直下に 1 ファイル 1 本で置く（理由はそのファイルの頭）。
//! 新しい試験は、内容に近い束のフォルダにファイルを置き、この `main.rs` に `mod` を 1 行足す（`ylp/bundle_layout.rs` が足し忘れを見つける）。
//! 共通の部品は下で 1 度だけ宣言し、各ファイルは `use crate::<部品>;` で使う。
#[path = "../brush_files/mod.rs"]
mod brush_files;

mod brush_bundled;
mod brush_clipstudio;
mod brush_golden;
mod brush_import;
mod brush_import_hostile;
mod brush_import_sut;
mod brush_import_sut_layer;
mod brush_import_sut_real;
