//! YoluPainter（Rust 版）を起動する。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 確保の失敗（メモリ不足）を、落ちる前に記録の先へ書く（`crash::oom`）。
#[global_allocator]
static ALLOCATOR: yolu_app::crash::RecordingAlloc = yolu_app::crash::RecordingAlloc;

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
    // ウィンドウの面の同期（垂直同期を待つか）は、ウィンドウを作るときに決まる。設定のファイルの値を、eframe を作る前に読む
    let vsync = yolu_app::settings::startup_vsync();
    yolu_app::crash::note(if vsync {
        "present mode: AutoVsync"
    } else {
        "present mode: AutoNoVsync"
    });
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("YoluPainter")
        .with_inner_size([1600.0, 960.0])
        .with_min_inner_size(yolu_app::layout::MIN_SIZE)
        .with_icon(icon());
    // Windows だけ OS のタイトルバーを外す（最小化・最大化・閉じるは、メニューの帯の右端に自前で置く）
    if yolu_app::titlebar::CUSTOM_FRAME {
        viewport = viewport.with_decorations(false);
    }
    // 前に終わったときのウィンドウの大きさと位置（設定のフォルダの layout.json）。画面を列挙できる OS（Windows）では、画面ごとの拡大率・
    // 作業領域と突き合わせて置き場所を決める（初回は主の画面の中央。記録の画面が無ければ主の画面の中央）。ウィンドウの位置と最大化は、
    // ウィンドウができてから最初のフレームで合わせる（`windowpos`）ので、作るときは大きさだけ渡す
    let record = yolu_app::layout::saved_window();
    if let Some(place) = yolu_app::windowpos::startup(record.as_ref()) {
        viewport = viewport.with_inner_size(place.size_points());
        yolu_app::windowpos::remember(place);
    } else if let Some(window) = record {
        // 画面を列挙できない OS: 記録をそのまま戻す。位置が今のどの画面にも見えなければ、位置は戻さず大きさだけ戻す
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
        // 3D ビューのアンチエイリアスに 2× と 8× を選べるよう、機材が持つ形式の機能を装置へ足す。面の同期は設定「垂直同期」（既定は待たない）
        wgpu_options: yolu_app::view3d::render::wgpu_configuration(vsync),
        // Linux: ウィンドウは X11 で開く（Wayland の机では XWayland の上）。Wayland は浮かせるウィンドウの位置を決められず、ウィンドウの外へ出す欄・位置の覚えが
        // 効かないため。X11 が無い机（XWayland の無い Wayland）では起動できない
        #[cfg(target_os = "linux")]
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        })),
        ..Default::default()
    };
    eframe::run_native(
        "YoluPainter",
        options,
        Box::new(|cc| {
            // Windows: ファイルのウィンドウの親になるメインウィンドウを預け、枠を外したウィンドウの最大化を自動で隠すタスクバーに合わせる
            yolu_app::dialog::set_owner(cc);
            yolu_app::windowpos::install(cc);
            Ok(Box::new(yolu_app::YoluApp::new(cc)))
        }),
    )
}

/// ウィンドウとタスクバーのアイコン（ロゴ。exe とインストーラーのアイコンは assets/logo/yolupainter.ico）。
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
