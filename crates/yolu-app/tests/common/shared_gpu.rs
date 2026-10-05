//! GPU 接続（アダプター・デバイス・キュー）を、同じ試験の実行ファイル（プロセス）の中で 1 つだけ作って共有する。画面・入力・描画資源
//! （`Renderer`・テクスチャ・3D の絵）は呼び出すたびに分離する。
//!
//! 画面ごとに接続を作り直すと、そのたびに 3D ビューの面のシェーダーとパイプラインを Vulkan（lavapipe）が最初から組む（測った例: 1 画面で約 0.7 秒。
//! 同じ接続の 2 回目からは約 0.1 秒）。共有しても結果の絵は変わらない（`Renderer` を作り直すので、egui のテクスチャの番号・描画コールバックの
//! 置き場は画面ごとに別。スナップショットは全件同じ）。窓を持つ試験は `gpu_thread` の貸し出しで 1 つずつ走るので、接続を同時に使うのは 1 つの試験だけ。

use std::sync::{Arc, OnceLock};

use eframe::egui_wgpu::{RenderState, Renderer, RendererOptions};
use egui_kittest::wgpu::WgpuTestRenderer;

/// アダプター・デバイス・キューは共用し、テクスチャ番号と描画コールバックの置き場は毎回作り直す。
/// 初期化用の状態は描画に使わず、各画面の renderer を共有しない。描画の設定は `common::render_options()`（実際の窓と同じ補間）。
pub fn renderer() -> WgpuTestRenderer {
    renderer_with(crate::common::render_options())
}

/// `renderer` の、描画の設定を指定する版（kittest の既定の「予測できる補間」は `RendererOptions::PREDICTABLE`）。
pub fn renderer_with(options: RendererOptions) -> WgpuTestRenderer {
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
        options,
    )));
    WgpuTestRenderer::from_render_state(state)
}
