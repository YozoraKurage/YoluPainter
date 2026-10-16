//! 本物のパネルを 1 つだけ入れたアプリ（`YoluApp` をタブ 1 つのドックとダミーの状態で起こす）。

use std::path::Path;

use yolu_app::app::{Tab, YoluApp};
use yolu_app::pen::PenInput;

use crate::args::Options;
use crate::scene::scene_state;
use crate::shoot::Fit;

impl Fit for YoluApp {
    fn content_bottom(&self) -> Option<f32> {
        None
    }
}

/// アプリ全体をタブ 1 つのドックとダミーの状態で起こす。
pub fn build(
    tab: Tab,
    options: &Options,
    library: &Path,
    ctx: &egui::Context,
    render_state: Option<&eframe::egui_wgpu::RenderState>,
) -> YoluApp {
    let state = scene_state(options.scene, options.lang, library);
    let mut app = YoluApp::for_context(ctx, state, PenInput::detached());
    app.dock = egui_dock::DockState::new(vec![tab]);
    let mut app = app.with_render_state(render_state);
    // 撮るたびに同じ絵になるよう、キャンバスと焼く場所は CPU に固定する
    app.set_canvas_backend(yolu_app::canvas::gpu::CanvasBackend::Cpu);
    app.state.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    app
}
