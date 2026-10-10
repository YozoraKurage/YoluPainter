//! 画面の試験の束: 3D ビュー・マテリアルの見た目（lilToon を含む）・ポーズ・テクスチャセット・出力（`egui_kittest` の harness。描画は wgpu のソフトの描画）。
//!
//! ウィンドウ（harness）を持つ試験は `common::gpu_thread` の貸し出しで 1 つずつ走る（lavapipe の中で同時に装置を作ると落ちることがあったため）。
//! 画面を使わない試験が同じファイルに混ざっていてもよい（貸し出しを取らないので並んで走る）。ただしウィンドウを作らなくても GPU の装置を作る試験
//! （製品のスレッドで GPU の確認・ベイクをする、`common::canvas_device` など）は、先頭で `common::gpu_thread::lease()` を取る。GPU の装置は `common::shared_gpu` の共用を使う。
//! 新しい試験は、該当する束のフォルダにファイルを足し、この `main.rs` に `mod` を 1 行足す（`headless/bundle_layout.rs` が足し忘れを見つける）。
#[path = "../common/mod.rs"]
mod common;

mod gpu_memory;
mod liltoon;
mod liltoon_ortho;
mod liltoon_panel;
mod liltoon_reference;
mod modes;
mod normalset;
mod objects;
mod outputs;
mod overlap_uv_ui;
mod parity_input;
mod parity_review;
mod parity_symmetry;
mod parity_views;
mod pose;
mod pose_takes;
mod pose_ui;
mod sets;
mod tool_keys_3d;
mod view3d;
mod view3d_aniso;
mod view3d_axes;
mod view3d_brush;
mod view3d_draft;
mod view3d_fx;
mod view3d_lag;
mod view3d_look;
mod view3d_navigation;
mod view3d_padding;
mod view3d_path_rect;
mod view3d_rulers;
mod view3d_rulers_edit;
mod view3d_select;
mod view3d_select_overlay;
mod view3d_select_pen;
mod view3d_sets;
