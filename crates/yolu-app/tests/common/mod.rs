//! 画面の試験の共通のツール（egui_kittest。描画は wgpu のソフトの描画で、コンテナでも回る）。
#![allow(dead_code)]

pub mod canvas_device;
pub mod core_refused;
pub mod fbx;
pub mod gpu_thread;
pub mod livelink;
pub mod shared_gpu;
pub mod tmp;
pub mod viewports;
pub mod wait;

use egui::{pos2, Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::pen::PenInput;
use yolu_app::state::AppState;
use yolu_app::YoluApp;

/// 描画の設定: 実際のウィンドウ（eframe）と同じくテクスチャの補間を GPU のサンプラーに任せる（kittest の既定の「予測できる補間」は
/// シェーダーの中の双線形と端の切り詰めで、Nearest と Repeat が効かず、拡大したキャンバスと市松が実際と違って見える）。
/// ディザは切る（撮るたびに同じ画素になるように）。`wgpu()` より前に渡すこと（後では効かない）。
pub fn render_options() -> eframe::egui_wgpu::RendererOptions {
    eframe::egui_wgpu::RendererOptions {
        predictable_texture_filtering: false,
        ..eframe::egui_wgpu::RendererOptions::PREDICTABLE
    }
}

/// 描画の状態をつなぐ（3D ビューを wgpu で描く）。キャンバスは CPU の表示に固定する（実 GPU の機材でも同じ絵・同じ頁の数に
/// なるように）。ウィンドウを作る試験の builder は、`with_render_state` を直に呼ばずにこれを通すこと。GPU の表示は canvas_gpu.rs が確かめる。
pub fn with_render_state_cpu_canvas(
    app: YoluApp,
    rs: Option<&eframe::egui_wgpu::RenderState>,
) -> YoluApp {
    let mut app = app.with_render_state(rs);
    app.set_canvas_backend(yolu_app::canvas::gpu::CanvasBackend::Cpu);
    app
}

/// ウィンドウの全体（eframe の App として）。文書は size × size。
pub fn app(width: f32, height: f32, size: u32) -> Harness<'static, YoluApp> {
    app_with_renderer(width, height, size, shared_gpu::renderer())
}

/// `app` の、描画器（装置）を選ぶ版（計測が `shared_gpu::renderer_with_adapter_limits` を渡す）。
pub fn app_with_renderer(
    width: f32,
    height: f32,
    size: u32,
    renderer: egui_kittest::wgpu::WgpuTestRenderer,
) -> Harness<'static, YoluApp> {
    let mut h = gpu_thread::builder()
        .with_size(egui::vec2(width, height))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0) // 実際のウィンドウに近い間隔（既定の 0.25 秒ではダブルクリックの間に収まらない）
        .with_max_steps(120)
        .renderer(renderer)
        .build_eframe(move |cc| {
            with_render_state_cpu_canvas(
                YoluApp::for_context(
                    &cc.egui_ctx,
                    AppState::new(size, size),
                    PenInput::detached(),
                ),
                cc.wgpu_render_state.as_ref(),
            )
        });
    // 焼く場所は CPU に固定（ハードウェアの GPU がある機械でも、試験の結果と画面を揺らさない）。GPU の試験は自分で選ぶ。
    h.state_mut().state.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    h.run();
    h
}

pub fn press(h: &Harness<'_, YoluApp>, at: Pos2, button: PointerButton) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
}

pub fn release(h: &Harness<'_, YoluApp>, at: Pos2, button: PointerButton) {
    h.event(Event::PointerButton {
        pos: at,
        button,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
}

pub fn move_to(h: &Harness<'_, YoluApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
}

/// ポインタを `at` に置いたまま、ツールチップが出る時間（既定 0.5 秒。1 フレーム 1/60 秒）より長く待つ。ツールチップの文字は
/// アクセシビリティの木に出るので、`query_by_label` で読める。
pub fn hover_and_wait(h: &mut Harness<'_, YoluApp>, at: Pos2) {
    move_to(h, at);
    for _ in 0..60 {
        h.step();
    }
}

/// 左ボタンで points をなぞる（1 点ごとに 1 フレーム）。
pub fn drag(h: &mut Harness<'_, YoluApp>, points: &[Pos2]) {
    press(h, points[0], PointerButton::Primary);
    h.step();
    for p in &points[1..] {
        move_to(h, *p);
        h.step();
    }
    release(h, *points.last().unwrap(), PointerButton::Primary);
    h.step();
    h.run();
}

pub fn click(h: &mut Harness<'_, YoluApp>, at: Pos2) {
    press(h, at, PointerButton::Primary);
    h.step();
    release(h, at, PointerButton::Primary);
    h.run();
}

/// 今のキャンバスの表示域（キャンバスのタブの中の、見出しの下）。
pub fn canvas_rect(h: &Harness<'_, YoluApp>) -> Rect {
    h.state().canvas_view_rect().expect("canvas drawn")
}

pub fn center(r: Rect) -> Pos2 {
    r.center()
}

pub fn offset(p: Pos2, dx: f32, dy: f32) -> Pos2 {
    pos2(p.x + dx, p.y + dy)
}

/// 画面の点の下の、文書の合成の画素。
pub fn canvas_pixel(h: &Harness<'_, YoluApp>, p: Pos2) -> [u8; 4] {
    let app = h.state();
    let r = canvas_rect(h);
    let v = app
        .state
        .view
        .view(r, app.state.doc.width(), app.state.doc.height());
    let (x, y) = v.to_canvas(p);
    yolu_app::engine::composite_pixel(
        &app.state.doc,
        x.floor().max(0.0) as u32,
        y.floor().max(0.0) as u32,
    )
}

/// 3D ビューの右上の軸の印の矩形（描いていなければ None）。絵の画素を数える試験は、この中を数えない（3D の絵の上に重ねた印）。
pub fn view3d_axes_rect(h: &Harness<'_, YoluApp>) -> Option<Rect> {
    use egui_kittest::kittest::Queryable;
    h.query_by_label("視点の軸")
        .or_else(|| h.query_by_label("View axes"))
        .map(|n| n.rect().expand(2.0))
}

/// 同じ名前の部品のうち、条件に合うもの（例: 右の列の中）の矩形。
pub fn rect_of(h: &Harness<'_, YoluApp>, label: &str, pick: impl Fn(Rect) -> bool) -> Rect {
    use egui_kittest::kittest::Queryable;
    let rects: Vec<Rect> = h.get_all_by_label(label).map(|n| n.rect()).collect();
    *rects
        .iter()
        .find(|r| pick(**r))
        .unwrap_or_else(|| panic!("{label}: {rects:?}"))
}

/// オプションバー（メニューバーの下の帯）の部品。同じ値は左のドックのツールプロパティにも出るので、名前だけでは 2 つに当たる。
pub fn bar_rect(h: &Harness<'_, YoluApp>, label: &str) -> Rect {
    rect_of(h, label, |r| r.top() > 24.0 && r.bottom() < 62.0)
}

/// 左のドックのサブツールのパネルの中（一覧・ツールプロパティ・ブラシサイズ）の部品。
pub fn dock_rect(h: &Harness<'_, YoluApp>, label: &str) -> Rect {
    rect_of(h, label, |r| {
        r.left() < 390.0 && r.top() > 62.0 && r.bottom() < 700.0
    })
}

/// メニューバーの見出し（同じ名前のドックのタブより上にある）。
pub fn menu_title(h: &Harness<'_, YoluApp>, label: &str) -> Rect {
    rect_of(h, label, |r| r.top() < 24.0)
}

/// 開いているポップアップの中の項目（行の矩形が本体にすっぽり入るもの。同じ名前の下の部品が、本体の端をまたいで重なっても取り違えない）。
pub fn popup_item(h: &Harness<'_, YoluApp>, label: &str) -> Rect {
    let body = h
        .state()
        .state
        .popup
        .as_ref()
        .expect("popup open")
        .state
        .rect;
    rect_of(h, label, |r| body.contains_rect(r))
}

pub fn key(h: &Harness<'_, YoluApp>, key: egui::Key, modifiers: Modifiers) {
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    });
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers,
    });
}

/// ドックのタブのボタンを押す（外へ出したウィンドウのタブも。試験のウィンドウでは、別ウィンドウはメインウィンドウの中の egui のウィンドウ）。
pub fn click_tab(h: &mut Harness<'_, YoluApp>, tab: yolu_app::Tab) {
    let app = h.state();
    let at = app
        .tab_rects
        .get(&tab)
        .or_else(|| {
            app.detached
                .windows
                .iter()
                .find_map(|w| w.tab_rects.get(&tab))
        })
        .expect("tab shown")
        .center();
    click(h, at);
}

/// 画面の文言に、使い方の説明・開発用の数が混じっていない（名前・状態・短い理由だけ。説明はツールチップ）。
pub fn assert_plain(what: &str, text: &str) {
    const HOW_TO: &[&str] = &[
        "してください",
        "ください",
        "クリック",
        "ドラッグして",
        "押して",
        "タップ",
        "選んで",
        "入力して",
        "click",
        "drag ",
        "press ",
        "please",
        "choose ",
        "select a",
        "to add",
        "you can",
        "tap ",
    ];
    const DEV: &[&str] = &["MiB", "KiB", "GiB", "三角形", "triangles", "バイト"];
    let lower = text.to_lowercase();
    for word in HOW_TO.iter().chain(DEV) {
        assert!(
            !lower.contains(&word.to_lowercase()),
            "{what}: 使い方・開発用の語「{word}」: {text}"
        );
    }
    assert!(
        text.chars().count() <= 70,
        "{what}: 長い（{} 字）: {text}",
        text.chars().count()
    );
}

pub fn has_japanese(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}')
    })
}
