//! 画面を描かない試験の束（ウィンドウ・GPU の装置を作らない。CPU だけの文書・保存・取り込み・Live Link の通信・ソースの検査）。
//!
//! 1 本の実行ファイルに束ねて、試験の数だけリンクが増えないようにしている。ここの試験は同時に走ってよい（`RUST_TEST_THREADS` の分だけ並ぶ）。
//! ウィンドウ・`egui_kittest` の harness・wgpu の装置を使う試験は `gui_*` の束に置く（`common::gpu_thread::builder` を通す。貸し出しで 1 つずつ走る）。
//! プロセス全体の状態（rayon の全体のプール・ウィンドウの貸し出し・環境変数）を変える・数える試験は、束に入れず `tests/` の直下に 1 ファイル 1 本の
//! 実行ファイルで置く（`threads.rs`・`window_lease.rs`・`windowpos.rs`）。新しい試験は、該当する束のフォルダにファイルを足し、この `main.rs` に `mod` を 1 行足す（`bundle_layout` が足し忘れを見つける）。
#[path = "../common/mod.rs"]
mod common;

mod automation;
mod bigdoc;
mod brush_import;
mod brush_list;
mod bucket;
mod bundle_layout;
mod colorsets;
mod dialog_parent;
mod dialog_places;
mod disk_cache;
mod effects;
mod fillfx;
mod frame_pacing;
mod ftl_keys;
mod guide_keys;
mod island_variation;
mod layer_sets;
mod liltoon_io;
mod livelink_files;
mod mcp_server;
mod new_layer_position;
mod no_developer_words;
mod no_instruction_text;
mod no_literal_pick;
mod notice_rules;
mod overlap_uv;
mod pose_saved;
mod pose_takes;
mod procedural;
mod recovery;
mod reference_lines;
mod save_background;
mod saved_selections;
mod symmetry_seeds;
mod tmp_cleanup;
mod tool_layout;
mod uv_topology;

/// 画面の外枠（`YoluApp`）を直に持つテスト（ウィンドウを作らずフレームを回す）も、AppState の欄を運転の口で読み書きする。
impl common::drive::Drive for yolu_app::YoluApp {
    fn app(&self) -> &yolu_app::state::AppState {
        &self.state
    }
    fn app_mut(&mut self) -> &mut yolu_app::state::AppState {
        &mut self.state
    }
}
