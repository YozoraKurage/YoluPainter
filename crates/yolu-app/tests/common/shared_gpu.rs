//! GPU 接続（アダプター・デバイス・キュー）を、同じ試験の実行ファイル（プロセス）の中で 1 つだけ作って共有する。画面・入力・描画資源
//! （`Renderer`・テクスチャ・3D の絵）は呼び出すたびに分離する。
//!
//! 画面ごとに接続を作り直すと、そのたびに 3D ビューの面のシェーダーとパイプラインを Vulkan（lavapipe）が最初から組む（測った例: 1 画面で約 0.7 秒。
//! 同じ接続の 2 回目からは約 0.1 秒）。共有しても結果の絵は変わらない（`Renderer` を作り直すので、egui のテクスチャの番号・描画コールバックの
//! 置き場は画面ごとに別。スナップショットは全件同じ）。ウィンドウを持つ試験は `gpu_thread` の貸し出しで 1 つずつ走るので、接続を同時に使うのは 1 つの試験だけ。

use std::sync::{Arc, OnceLock};

use eframe::egui_wgpu::{RenderState, Renderer, RendererOptions};
use egui_kittest::wgpu::WgpuTestRenderer;

/// アダプター・デバイス・キューは共用し、テクスチャ番号と描画コールバックの置き場は毎回作り直す。
/// 初期化用の状態は描画に使わず、各画面の renderer を共有しない。描画の設定は `common::render_options()`（実際のウィンドウと同じ補間）。
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

/// 計測用: 装置を、アダプターが持つ上限で作る（`renderer` は egui の既定の上限で作る。egui-wgpu は OpenGL のとき WebGL2 の上限（storage の
/// 入れ物も compute も無い）で装置を作るので、OpenGL ではキャンバスの GPU の表示などの compute を使う道が動かない。製品の装置の作り方は
/// 変えず、OpenGL の道で GPU の仕事を測るときだけ使う）。プロセスに 1 つ。
pub fn renderer_with_adapter_limits() -> WgpuTestRenderer {
    static CONNECTION: OnceLock<RenderState> = OnceLock::new();
    let connection = CONNECTION.get_or_init(|| {
        let mut setup = egui_kittest::wgpu::default_wgpu_setup();
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
            let base = create.device_descriptor.clone();
            create.device_descriptor = Arc::new(move |adapter| {
                let mut descriptor = base(adapter);
                descriptor.required_limits = adapter.limits();
                descriptor
            });
        }
        egui_kittest::wgpu::create_render_state(setup, crate::common::render_options())
    });
    let mut state = connection.clone();
    state.renderer = Arc::new(egui::mutex::RwLock::new(Renderer::new(
        &state.device,
        state.target_format,
        crate::common::render_options(),
    )));
    WgpuTestRenderer::from_render_state(state)
}
