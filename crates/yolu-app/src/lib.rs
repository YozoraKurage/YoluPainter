//! YoluPainter（Rust 版）の画面。egui（eframe・wgpu）の窓に、Unity 版と同じ配色と部品の見た目で、Substance の並びのドックを置く。
//! 計算は core（`engine` が指す先と、3D の面の計算 `yolu_core::geometry`）に任せ、ここは画面と入力だけ。テクスチャセット（`sets`）、
//! Live Link（`livelink`。Unity から受けたモデルの記録は `model`、3D の形は `view3d`）、.ylp の開く・保存（`project`）も画面の状態との
//! 受け渡しだけ。

pub mod app;
pub mod canvas;
pub mod engine;
pub mod lang;
pub mod livelink;
pub mod m2;
pub mod m2_menu;
pub mod model;
pub mod panels;
pub mod pen;
pub mod project;
pub mod sets;
pub mod shell;
pub mod state;
pub mod ui;
pub mod view3d;

pub use app::{Tab, YoluApp};
