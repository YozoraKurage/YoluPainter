//! 効果の束: フィルター・Generator・Anchor・パス・スマートマテリアルの編集と、レイヤーのロック・レイヤーの操作とのつなぎ目。
//!
//! 1 本の実行ファイルに束ねて、試験のファイルの数だけビルドとリンクが増えないようにしている。束の中の試験は同じプロセスで同時に走る
//! （`RUST_TEST_THREADS` の分だけ）。プロセス全体の状態を変える試験は束に入れず、`tests/` の直下に 1 ファイル 1 本で置く（理由はそのファイルの頭）。
//! 新しい試験は、内容に近い束のフォルダにファイルを置き、この `main.rs` に `mod` を 1 行足す（`edit/bundle_layout.rs` が足し忘れを見つける）。
//! 共通の部品は下で 1 度だけ宣言し、各ファイルは `use crate::<部品>;` で使う。
#[path = "../attach_support/mod.rs"]
mod attach_support;
#[path = "../seam_support/mod.rs"]
mod seam_support;

mod attach_coherence;
mod attach_edit;
mod attach_paths;
mod attach_smart;
mod fill_mip_sharing;
mod fill_points;
mod filter_kinds;
mod generator_kinds;
mod image_stage;
mod island_variation;
mod outputs;
mod path_fill_layers;
mod path_kinds;
mod path_lists;
mod path_tangents;
mod path_tips;
mod procedural_doc;
mod seam_locks;
mod seam_ops;
mod smart;
mod smart_library;
mod uv_seam_filters;
