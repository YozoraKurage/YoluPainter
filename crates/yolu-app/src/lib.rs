//! YoluPainter（Rust 版）の画面。egui（eframe・wgpu）の窓に、Unity 版と同じ配色と部品の見た目で、Substance の並びのドックを置く。
//! 計算は core（`engine` が指す先と、3D の面の計算 `yolu_core::geometry`）に任せ、ここは画面と入力だけ。テクスチャセット（`sets`）、
//! Live Link（`livelink`。Unity から受けたモデルの記録は `model`、3D の形は `view3d`）、.ylp の開く・保存（`project`）も画面の状態との
//! 受け渡しだけ。メッシュマップのベイク（`bake`）・テンプレートの書き出し（`export`）・PSD の読み書き（`psd`）は、長い処理を別の
//! スレッドで走らせ（進み具合と取消つき）、終わったとき状態が変わっていないか確かめてから結果を入れる。浮いた窓は `windows`・`ui::window`。

pub mod app;
pub mod bake;
pub mod canvas;
pub mod engine;
pub mod export;
pub mod lang;
pub mod livelink;
pub mod m2;
pub mod m2_menu;
pub mod matpaint;
pub mod model;
pub mod panels;
pub mod pen;
pub mod project;
pub mod psd;
pub mod region;
pub mod selection;
pub mod sets;
mod settings;
pub mod shelf;
pub mod shell;
pub mod state;
pub mod stencil;
pub mod ui;
pub mod view3d;
pub mod windows;

pub use app::{Tab, YoluApp};
