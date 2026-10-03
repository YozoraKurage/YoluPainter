//! YoluPainter（Rust 版）の画面。egui（eframe・wgpu）の窓に、Unity 版と同じ配色と部品の見た目で、Substance の並びのドックを置く。
//! 計算は core（`engine` が指す先。今は app の中の仮の core）に任せ、ここは画面と入力だけ。

pub mod app;
pub mod canvas;
pub mod engine;
pub mod panels;
pub mod pen;
pub mod shell;
pub mod state;
pub mod ui;

pub use app::{Tab, YoluApp};
