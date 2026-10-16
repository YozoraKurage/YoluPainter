//! ウィンドウで開く・ウィンドウを開かずに撮って PNG に書く（どの入口も同じ道）。

use std::path::Path;

use crate::args::{Options, MAX_POINTS};

/// 実際のウィンドウと同じ補間（テクスチャの補間を GPU のサンプラーに任せる）で、毎回同じ画素になるようディザは切る。
fn render_options() -> eframe::egui_wgpu::RendererOptions {
    eframe::egui_wgpu::RendererOptions {
        predictable_texture_filtering: false,
        ..eframe::egui_wgpu::RendererOptions::PREDICTABLE
    }
}

/// 製品と同じ D3D12 のシェーダーのコンパイラー（Windows）で、kittest の描画器のデバイスを作る。
fn test_renderer() -> egui_kittest::wgpu::WgpuTestRenderer {
    let mut setup = egui_kittest::wgpu::default_wgpu_setup();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
        create
            .instance_descriptor
            .backend_options
            .dx12
            .shader_compiler = yolu_gpu::dx12_compiler();
    }
    egui_kittest::wgpu::WgpuTestRenderer::from_render_state(
        egui_kittest::wgpu::create_render_state(setup, render_options()),
    )
}

/// 撮る前に、アプリが描いた中身の下の端（論理の点）を聞く口。`None` は画面の大きさのまま撮る。
pub trait Fit {
    fn content_bottom(&self) -> Option<f32>;
}

/// ウィンドウで開く。
pub fn open_window<A: eframe::App + 'static>(
    options: &Options,
    build: impl FnOnce(&egui::Context, Option<&eframe::egui_wgpu::RenderState>) -> A + 'static,
) -> Result<(), String> {
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("YoluPainter mock: {}", options.target.label()))
            .with_inner_size(options.size),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: yolu_app::view3d::render::wgpu_configuration(false),
        // 本体と同じく、Linux は X11 で開く
        #[cfg(target_os = "linux")]
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        })),
        ..Default::default()
    };
    eframe::run_native(
        "YoluPainter mock",
        native,
        Box::new(move |cc| Ok(Box::new(build(&cc.egui_ctx, cc.wgpu_render_state.as_ref())))),
    )
    .map_err(|e| e.to_string())
}

/// ウィンドウを開かずに撮って、PNG に書く。`fit` が true なら、描いた中身の下の端まで高さを伸ばして（縮めて）撮り直す。
pub fn write_png<A: eframe::App + Fit + 'static>(
    options: &Options,
    path: &Path,
    fit: bool,
    build: impl FnOnce(&egui::Context, Option<&eframe::egui_wgpu::RenderState>) -> A,
) -> Result<(), String> {
    let mut size = egui::vec2(options.size[0], options.size[1]);
    if fit {
        // 中身の高さが分かるまでは、撮れる最大の高さで描く
        size.y = MAX_POINTS / options.scale;
    }
    let mut harness = egui_kittest::Harness::builder()
        .with_size(size)
        .with_pixels_per_point(options.scale)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(test_renderer())
        .build_eframe(|cc| build(&cc.egui_ctx, cc.wgpu_render_state.as_ref()));
    // 描き直しが止まるまで進める。止まらないパネル（動き続ける物）は、上限まで進めた絵を撮る
    if let Err(e) = harness.try_run() {
        eprintln!("warning: {e}; captured the last frame");
    }
    if fit {
        let bottom = harness
            .state()
            .content_bottom()
            .ok_or("the page did not report its height")?;
        let height = bottom.ceil().max(1.0);
        if height * options.scale > MAX_POINTS {
            return Err(format!(
                "the page is too tall to capture: {height} points at scale {} (each side must be at most {MAX_POINTS}); lower --scale",
                options.scale
            ));
        }
        harness.set_size(egui::vec2(size.x, height));
        if let Err(e) = harness.try_run() {
            eprintln!("warning: {e}; captured the last frame");
        }
    }
    let image = harness.render()?;
    image
        .save(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}
