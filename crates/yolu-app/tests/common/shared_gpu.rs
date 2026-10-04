//! GPU 接続だけを共有する試験用の準備。画面・入力・描画資源は呼び出すたびに分離する。

use std::sync::{Arc, OnceLock};

use eframe::egui_wgpu::{RenderState, Renderer};
use egui_kittest::{wgpu::WgpuTestRenderer, Harness};
use yolu_app::{pen::PenInput, state::AppState, YoluApp};

/// アダプター・デバイス・キューは共用し、テクスチャ番号と描画コールバックの置き場は毎回作り直す。
/// 初期化用の状態は描画に使わず、各画面の renderer を共有しない。
pub fn renderer() -> WgpuTestRenderer {
    static CONNECTION: OnceLock<RenderState> = OnceLock::new();
    let connection = CONNECTION.get_or_init(|| {
        egui_kittest::wgpu::create_render_state(
            egui_kittest::wgpu::default_wgpu_setup(),
            crate::common::render_options(),
        )
    });
    let mut state = connection.clone();
    state.renderer = Arc::new(egui::mutex::RwLock::new(Renderer::new(
        &state.device,
        state.target_format,
        crate::common::render_options(),
    )));
    WgpuTestRenderer::from_render_state(state)
}

/// common::app と同じ窓・文書・CPU キャンバスを、共用の GPU 接続に載せる。
pub fn app(width: f32, height: f32, size: u32) -> Harness<'static, YoluApp> {
    let mut h = crate::common::gpu_thread::builder()
        .with_size(egui::vec2(width, height))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(renderer())
        .build_eframe(move |cc| {
            crate::common::with_render_state_cpu_canvas(
                YoluApp::for_context(
                    &cc.egui_ctx,
                    AppState::new(size, size),
                    PenInput::detached(),
                ),
                cc.wgpu_render_state.as_ref(),
            )
        });
    h.state_mut().state.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    h.run();
    h
}
