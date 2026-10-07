//! プロジェクトの置き場の束: 大きな .ylp・復旧の世代・ライブラリ・棚・配布用の写し・保存の並列・新しいプロジェクト。
//!
//! 1 本の実行ファイルに束ねて、試験のファイルの数だけビルドとリンクが増えないようにしている。束の中の試験は同じプロセスで同時に走る
//! （`RUST_TEST_THREADS` の分だけ）。プロセス全体の状態を変える試験は束に入れず、`tests/` の直下に 1 ファイル 1 本で置く（理由はそのファイルの頭）。
//! 新しい試験は、内容に近い束のフォルダにファイルを置き、この `main.rs` に `mod` を 1 行足す（`ylp/bundle_layout.rs` が足し忘れを見つける）。
//! 共通の部品は下で 1 度だけ宣言し、各ファイルは `use crate::<部品>;` で使う。
mod bigdoc;
mod distribution;
mod generation;
mod library;
mod project_edit;
mod save_parallel;
mod shelf;
mod smart_resample;
