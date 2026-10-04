//! YoluPainter（Rust 版）の画面。egui（eframe・wgpu）の窓に、Unity 版と同じ配色と部品の見た目で、Substance の並びのドックを置く。
//! 計算は core（`engine` が指す先と、3D の面の計算 `yolu_core::geometry`）に任せ、ここは画面と入力だけ。テクスチャセット（`sets`）、
//! Live Link（`livelink`。Unity から受けたモデルの記録は `model`、3D の形は `view3d`）、.ylp の開く・保存（`project`）も画面の状態との
//! 受け渡しだけ。メッシュマップのベイク（`bake`）・テンプレートの書き出し（`export`）・PSD の読み書き（`psd`）は、長い処理を別の
//! スレッドで走らせ（進み具合と取消つき）、終わったとき状態が変わっていないか確かめてから結果を入れる。浮いた窓は `windows`・`ui::window`。
//! ブラシの一覧（組み込みと利用者のブラシ・道具ごとの覚え・見本のストローク・保存）は `brushes`、その画面は左のドックの `panels::brushes` と
//! 詳細の窓 `panels::brush_detail`（欄は `panels::brush_props`）。
//! 落ちても失わない書き置き（変更があると裏のスレッドで復旧用の世代を書き、落ちた次の起動で復旧の窓から開く）は `recovery`。
//! プロジェクトの構成（`newproject`。モデルは別のスレッドで読み、決めたとき 3D ビューに入れる）も画面の状態との受け渡しだけ。
//! 塗りつぶしの層の画像と投影・デカール・ワールドスペースのグラデーションは `fillfx`（欄は `panels::fill_props`、3D ビューの形のギズモは
//! `view3d::shape_gizmo`）、2D のグラデーションの道具は `gradient`。

pub mod app;
pub mod bake;
pub mod brushes;
pub mod canvas;
pub mod clipboard;
pub mod crash;
pub mod engine;
pub mod export;
pub mod fx;
pub mod eyedrop;
pub mod screen_pick;
pub mod fillfx;
pub mod gradient;
pub mod drafting;
pub mod gesture;
pub mod lang;
pub mod layerops;
pub mod livelink;
pub mod m2;
pub mod m2_menu;
pub mod matpaint;
pub mod model;
pub mod newproject;
pub mod panels;
pub mod pathtool;
pub mod pen;
pub mod prefs;
pub mod project;
pub mod psd;
pub mod recovery;
pub mod region;
pub mod selection;
pub mod sets;
pub mod settings;
pub mod colorsets;
pub mod shelf;
pub mod shell;
pub mod state;
pub mod stencil;
pub mod transform;
pub mod ui;
pub mod update;
pub mod view3d;
pub mod windows;

pub use app::{Tab, YoluApp};
