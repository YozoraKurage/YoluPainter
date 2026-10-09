//! 3D の面と塗りの束: 面への投影・面のストローク・対称・参照の写像・メッシュのマップ・ブラシの参照元・ステンシル・チャンネルの塗り。
//!
//! 1 本の実行ファイルに束ねて、試験のファイルの数だけビルドとリンクが増えないようにしている。束の中の試験は同じプロセスで同時に走る
//! （`RUST_TEST_THREADS` の分だけ）。プロセス全体の状態を変える試験は束に入れず、`tests/` の直下に 1 ファイル 1 本で置く（理由はそのファイルの頭）。
//! 新しい試験は、内容に近い束のフォルダにファイルを置き、この `main.rs` に `mod` を 1 行足す（`edit/bundle_layout.rs` が足し忘れを見つける）。
//! 共通の部品は下で 1 度だけ宣言し、各ファイルは `use crate::<部品>;` で使う。
#[path = "../rayon_support/mod.rs"]
mod rayon_support;

mod anti_alias;
mod brush_sources;
mod cross_symmetry;
mod dab_digest;
mod material;
mod mesh_maps;
mod overlap_priority;
mod stencil;
mod surface_ortho;
mod surface_path_rebind;
mod surface_projection;
mod surface_sampling;
mod surface_screen;
mod surface_stroke;
mod surface_symmetry;
mod surface_tip;
mod uv_topology;
