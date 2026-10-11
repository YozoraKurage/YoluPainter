//! YoluPainter（Rust 版）の画面。egui（eframe・wgpu）のウィンドウに、Unity 版と同じ配色と部品の見た目で、Substance の並びのドックを置く。
//! 外からの操作（MCP のクライアント・コマンドラインの命令。設定で入れたときだけ受ける）は、127.0.0.1 の HTTP の受け口と要求の列が `mcp_server`、
//! 今のプロジェクトへ当てるホストが `ops_host`。
//! 計算は core（`engine` が指す先と、3D の面の計算 `yolu_core::geometry`）に任せ、ここは画面と入力だけ。テクスチャセット（`sets`）、
//! Live Link（`livelink`。Unity から受けたモデルの記録は `model`、3D の形は `view3d`）、.ylp の開く・保存（`project`）も画面の状態との
//! 受け渡しだけ。メッシュマップのベイク（`bake`）・テンプレートの書き出し（`export`）・PSD の読み書き（`psd`）は、長い処理を別の
//! スレッドで走らせ（進み具合と取消つき）、終わったとき状態が変わっていないか確かめてから結果を入れる。浮いたウィンドウは `windows`・`ui::window`。
//! ブラシの一覧（組み込みと利用者のブラシ・ツールごとの覚え・見本のストローク・保存）は `brushes`、その画面は左のドックの `panels::brushes` と
//! 詳細のウィンドウ `panels::brush_detail`（欄は `panels::brush_props`）。
//! 落ちても失わない書き置き（変更があると裏のスレッドで復旧用の世代を書き、落ちた次の起動で復旧のウィンドウから開く）は `recovery`。
//! プロジェクトの構成（`newproject`。モデルは別のスレッドで読み、決めたとき 3D ビューに入れる）も画面の状態との受け渡しだけ。
//! 塗りつぶしレイヤーの画像と投影・デカール・グラデーションデカールは `fillfx`（欄は `panels::fill_props`、3D ビューの形のギズモは
//! `view3d::shape_gizmo`）、2D のグラデーションのツールは `gradient`。

pub mod app;
pub mod automation;
pub mod bake;
pub mod brushes;
pub mod canvas;
pub mod clipboard;
pub mod clone_source;
pub mod colorsets;
pub mod commands;
pub mod crash;
pub mod detach;
pub mod dialog;
pub mod distribute;
pub mod drafting;
pub mod engine;
pub mod export;
pub mod eyedrop;
pub mod eyedrop_mark;
pub mod fillfx;
pub mod fx;
pub mod gesture;
pub mod gpu_memory;
pub mod gpu_watch;
pub mod gradient;
pub mod jobs;
pub mod keyconfig;
pub mod keymap;
pub mod lang;
pub mod layermenu;
pub mod layerops;
pub mod layout;
pub mod library;
pub mod livelink;
pub mod look;
pub mod m2;
pub mod m2_menu;
pub mod matpaint;
pub mod mcp_server;
pub mod mode;
pub mod model;
pub mod newproject;
pub mod notice;
pub mod objects;
pub mod ops_host;
pub mod pacing;
pub mod panels;
pub mod pathtool;
pub mod pen;
pub mod pie;
pub mod prefs;
pub mod project;
pub mod psd;
pub mod psd_export;
pub mod psd_import;
pub mod rampsets;
pub mod recovery;
pub mod region;
pub mod rulers;
pub mod screen_pick;
pub mod selection;
pub mod session_end;
pub mod sets;
pub mod settings;
pub mod shelf;
pub mod shell;
pub mod shortcuts;
pub mod state;
pub mod stencil;
pub mod subtool;
pub mod textlayer;
pub mod titlebar;
pub mod toast;
pub mod toolkeys;
pub mod tools;
pub mod toolset;
pub mod transform;
pub mod ui;
pub mod update;
pub mod usage;
pub mod userfiles;
pub mod uv_wireframe;
pub mod view3d;
pub mod windowpos;
pub mod windows;

pub use app::{Tab, YoluApp};

pub mod navigator;
