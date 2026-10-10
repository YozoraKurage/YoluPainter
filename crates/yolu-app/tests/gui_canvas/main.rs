//! 画面の試験の束: キャンバス・ツール・ブラシ・選択・色・効果（`egui_kittest` の harness。描画は wgpu のソフトの描画）。
//!
//! ウィンドウ（harness）を持つ試験は `common::gpu_thread` の貸し出しで 1 つずつ走る（lavapipe の中で同時に装置を作ると落ちることがあったため）。
//! 画面を使わない試験が同じファイルに混ざっていてもよい（貸し出しを取らないので並んで走る）。ただしウィンドウを作らなくても GPU の装置を作る試験
//! （製品のスレッドで GPU の確認・ベイクをする、`common::canvas_device` など）は、先頭で `common::gpu_thread::lease()` を取る。GPU の装置は `common::shared_gpu` の共用を使う。
//! 新しい試験は、該当する束のフォルダにファイルを足し、この `main.rs` に `mod` を 1 行足す（`headless/bundle_layout.rs` が足し忘れを見つける）。
#[path = "../common/mod.rs"]
mod common;

mod anti_alias_ui;
mod brush_import_ui;
mod brush_menu;
mod brush_mix;
mod brushes;
mod canvas_gpu;
mod clipboard;
mod clipping_button;
mod clone_parity;
mod color_adjust;
mod color_adjust_app;
mod curve_editor;
mod default_layout;
mod distribute;
mod drafting;
mod effects_ui;
mod eyedrop;
mod fillfx_gui;
mod gestures;
mod grunge_picker;
mod history;
mod image_stage_ui;
mod layer_menu;
mod layerops;
mod pathtool;
mod pressure_adjust;
mod ramp_editor;
mod ramp_panel;
mod region;
mod seams_ui;
mod selection;
mod selection_bar;
mod selection_build;
mod stencil;
mod stroke_look;
mod text_tool;
mod tool_keys;
mod tool_layout_ui;
mod view_controls;
mod warp;
