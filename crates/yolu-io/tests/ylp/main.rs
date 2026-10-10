//! .ylp の形式の束: 正本の読み書き・前の版との互換・断り方・C# の書き手との一致・形式の仕様の文書。
//!
//! 1 本の実行ファイルに束ねて、試験のファイルの数だけビルドとリンクが増えないようにしている。束の中の試験は同じプロセスで同時に走る
//! （`RUST_TEST_THREADS` の分だけ）。プロセス全体の状態を変える試験は束に入れず、`tests/` の直下に 1 ファイル 1 本で置く（理由はそのファイルの頭）。
//! 新しい試験は、内容に近い束のフォルダにファイルを置き、この `main.rs` に `mod` を 1 行足す（`ylp/bundle_layout.rs` が足し忘れを見つける）。
//! 共通の部品は下で 1 度だけ宣言し、各ファイルは `use crate::<部品>;` で使う。
#[path = "../legacy_layout/mod.rs"]
mod legacy_layout;

mod adjust_bridge;
mod anti_alias_bridge;
mod bake_priority_bridge;
mod bundle_layout;
mod compatibility;
mod core_bridge;
mod effects_bridge;
mod filters_v28;
mod format_doc;
mod gradient_mixing_bridge;
mod image_generator;
mod layer_locks;
mod look;
mod m2_bridge;
mod manual_id_colors;
mod materials;
mod nesting_native;
mod path_lists_bridge;
mod point_gradient_bridge;
mod procedural_bridge;
mod rejection;
mod rulers_bridge;
mod saved_selections;
mod seams_bridge;
mod selection;
mod text_bridge;
