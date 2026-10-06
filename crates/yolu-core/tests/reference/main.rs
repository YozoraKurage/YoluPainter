//! 正解との照合の束: 実 C# Core が書いた正解（`tests/golden/`）・収録したハッシュ（`generator-index.txt`・`procedural-index.txt`）と、
//! 全バイトで比べる試験。正解の撮り直し（`YOLU_GOLDEN_UPDATE=1`）の道具 `golden_update` は束で 1 つ（書き直しの鍵を束の中で共有する）。
//!
//! 1 本の実行ファイルに束ねて、試験のファイルの数だけビルドとリンクが増えないようにしている。束の中の試験は同じプロセスで同時に走る
//! （`RUST_TEST_THREADS` の分だけ）。プロセス全体の状態を変える試験は束に入れず、`tests/` の直下に 1 ファイル 1 本で置く（理由はそのファイルの頭）。
//! 新しい試験は、内容に近い束のフォルダにファイルを置き、この `main.rs` に `mod` を 1 行足す（`edit/bundle_layout.rs` が足し忘れを見つける）。
//! 共通の部品は下で 1 度だけ宣言し、各ファイルは `use crate::<部品>;` で使う。
#[path = "../attach_support/mod.rs"]
mod attach_support;
#[path = "../support/fill_cases.rs"]
mod fill_cases;
#[path = "../filter_support/mod.rs"]
mod filter_support;
#[path = "../generator_support/mod.rs"]
mod generator_support;
#[path = "../golden_update/mod.rs"]
mod golden_update;
#[path = "../seam_support/mod.rs"]
mod seam_support;

mod clipboard_golden;
mod docops_golden;
mod export_golden;
mod fill_image;
mod filter;
mod generator;
mod generator_rows;
mod material_golden;
mod procedural;
mod seam_golden;
mod surface_golden;
