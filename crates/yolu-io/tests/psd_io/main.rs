//! PSD の束: 写しの書き出しと取り込み・焼き込み・調整レイヤー・M2 の層・実物の大きさで流して書く書き出し・C# の正解との照合。
//!
//! 1 本の実行ファイルに束ねて、試験のファイルの数だけビルドとリンクが増えないようにしている。束の中の試験は同じプロセスで同時に走る
//! （`RUST_TEST_THREADS` の分だけ）。プロセス全体の状態を変える試験は束に入れず、`tests/` の直下に 1 ファイル 1 本で置く（理由はそのファイルの頭）。
//! 新しい試験は、内容に近い束のフォルダにファイルを置き、この `main.rs` に `mod` を 1 行足す（`ylp/bundle_layout.rs` が足し忘れを見つける）。
//! 共通の部品は下で 1 度だけ宣言し、各ファイルは `use crate::<部品>;` で使う。
#[path = "../../../yolu-core/tests/attach_support/mod.rs"]
mod attach_support;

mod psd;
mod psd_adjust;
mod psd_bake;
mod psd_golden;
mod psd_import;
mod psd_m2;
mod psd_stream;
