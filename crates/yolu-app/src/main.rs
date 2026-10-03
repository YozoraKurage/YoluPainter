//! YoluPainter（Rust 版）を起動する。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("YoluPainter")
            .with_inner_size([1600.0, 960.0])
            .with_min_inner_size([960.0, 640.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "YoluPainter",
        options,
        Box::new(|cc| Ok(Box::new(yolu_app::YoluApp::new(cc)))),
    )
}
