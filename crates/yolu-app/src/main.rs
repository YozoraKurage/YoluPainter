//! YoluPainter（Rust 版）を起動する。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    // CPU のスレッドの設定は、最初の rayon の利用より前に入れる（変えた値は次の起動から効く）
    yolu_app::settings::apply_thread_setting();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("YoluPainter")
            .with_inner_size([1600.0, 960.0])
            .with_min_inner_size([960.0, 640.0])
            .with_icon(icon()),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "YoluPainter",
        options,
        Box::new(|cc| Ok(Box::new(yolu_app::YoluApp::new(cc)))),
    )
}

/// 窓とタスクバーのアイコン（ロゴ。exe とインストーラーのアイコンは assets/logo/yolupainter.ico）。
fn icon() -> egui::IconData {
    let png = include_bytes!("../assets/logo/yolupainter-256.png");
    let image = image::load_from_memory(png)
        .expect("同梱のロゴ")
        .into_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}
