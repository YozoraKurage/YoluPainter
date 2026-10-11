//! 画面の試験の束: ウィンドウの全体・メニュー・設定・文書の出し入れ・復旧・更新・Live Link の画面・言語（`egui_kittest` の harness。描画は wgpu のソフトの描画）。
//!
//! ウィンドウ（harness）を持つ試験は `common::gpu_thread` の貸し出しで 1 つずつ走る（lavapipe の中で同時に装置を作ると落ちることがあったため）。
//! 画面を使わない試験が同じファイルに混ざっていてもよい（貸し出しを取らないので並んで走る）。ただしウィンドウを作らなくても GPU の装置を作る試験
//! （製品のスレッドで GPU の確認・ベイクをする、`common::canvas_device` など）は、先頭で `common::gpu_thread::lease()` を取る。GPU の装置は `common::shared_gpu` の共用を使う。
//! 新しい試験は、該当する束のフォルダにファイルを足し、この `main.rs` に `mod` を 1 行足す（`headless/bundle_layout.rs` が足し忘れを見つける）。
#[path = "../common/mod.rs"]
mod common;

mod actions_panel;
mod app;
mod chrome;
mod close_jobs;
mod color_window;
mod colorsets_ui;
mod detached;
mod disabled_reasons;
mod fonts;
mod gpu_lost;
mod i18n;
mod layout;
mod library;
mod limits;
mod link_entrance;
mod log_panel;
mod m2;
mod menus;
mod navigator;
mod newproject;
mod numfield;
mod prefs;
mod psd_drop;
mod recovery_ui;
mod rulers_ui;
mod save_close;
mod scroll;
mod settings_window;
mod shelf;
mod shortcut_editor;
mod subtools;
mod titlebar;
mod tool_panels;
mod update;
mod widgets;
