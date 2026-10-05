//! YoluPainter（Rust 版）を起動する。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    yolu_app::crash::install();
    let result = yolu_app::crash::guard(start, yolu_app::crash::failure_dialog);
    if let Err(error) = &result {
        yolu_app::crash::startup_failure(&error.to_string());
    }
    yolu_app::crash::cleanup();
    result
}

fn start() -> eframe::Result {
    // CPU のスレッドの設定は、最初の rayon の利用より前に入れる（変えた値は次の起動から効く）
    yolu_app::settings::apply_thread_setting();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("YoluPainter")
        .with_inner_size([1600.0, 960.0])
        .with_min_inner_size(yolu_app::layout::MIN_SIZE)
        .with_icon(icon());
    // Windows だけ OS のタイトルバーを外す（最小化・最大化・閉じるは、メニューの帯の右端に自前で置く）
    if yolu_app::titlebar::CUSTOM_FRAME {
        viewport = viewport.with_decorations(false);
    }
    // 前に終わったときの窓の大きさと位置（設定のフォルダの layout.json）。位置が今のどの画面にも見えなければ、位置は戻さず大きさだけ戻す
    if let Some(window) = yolu_app::layout::saved_window() {
        viewport = viewport.with_inner_size(window.size);
        if yolu_app::layout::is_visible_on_a_monitor(&window) {
            viewport = viewport.with_position(window.position);
        }
        if window.maximized {
            viewport = viewport.with_maximized(true);
        }
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "YoluPainter",
        options,
        Box::new(|cc| {
            // Windows: ファイルの窓の親になる主の窓を預ける
            yolu_app::dialog::set_owner(cc);
            Ok(Box::new(yolu_app::YoluApp::new(cc)))
        }),
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
