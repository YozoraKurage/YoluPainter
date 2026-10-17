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
mod clip_isolation;
mod clipboard;
mod color_change;
mod comp2d;
mod docops;
mod dual_brush;
mod dual_tips;
mod export;
mod history;
mod import_tiles;
mod influences;
mod layer_import;
mod layer_sets;
mod layers;
mod mask_copy;
mod merge_compare;
mod min_pixel;
mod nesting;
mod ramp_mixing;
mod reference_lines;
mod rulers;
mod saved_mark;
mod selection;
mod simd_document;
mod spray;
mod stroke_correction;
mod taper_start;
mod taper_targets;
mod text_layer;
mod tip_rules;
mod view_scale;
mod warp;
mod water_edge;
