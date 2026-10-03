//! 画面の試験の共通の道具（egui_kittest。描画は wgpu のソフトの描画で、コンテナでも回る）。
#![allow(dead_code)]

use egui::{pos2, Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::pen::PenInput;
use yolu_app::state::AppState;
use yolu_app::YoluApp;

/// 描画の設定: 実際の窓（eframe）と同じくテクスチャの補間を GPU のサンプラーに任せる（kittest の既定の「予測できる補間」は
/// シェーダーの中の双線形と端の切り詰めで、Nearest と Repeat が効かず、拡大したキャンバスと市松が実際と違って見える）。
/// ディザは切る（撮るたびに同じ画素になるように）。`wgpu()` より前に渡すこと（後では効かない）。
pub fn render_options() -> eframe::egui_wgpu::RendererOptions {
    eframe::egui_wgpu::RendererOptions {
        predictable_texture_filtering: false,
        ..eframe::egui_wgpu::RendererOptions::PREDICTABLE
    }
}

/// 窓の全体（eframe の App として）。文書は size × size。
pub fn app(width: f32, height: f32, size: u32) -> Harness<'static, YoluApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(width, height))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0) // 実際の窓に近い間隔（既定の 0.25 秒ではダブルクリックの間に収まらない）
        .with_max_steps(120)
        .with_render_options(render_options())
        .wgpu()
        .build_eframe(move |cc| {
            YoluApp::for_context(
                &cc.egui_ctx,
                AppState::new(size, size),
                PenInput::detached(),
            )
            .with_render_state(cc.wgpu_render_state.as_ref())
        });
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

/// 同じ名前の部品のうち、条件に合うもの（例: 右の列の中）の矩形。
pub fn rect_of(h: &Harness<'_, YoluApp>, label: &str, pick: impl Fn(Rect) -> bool) -> Rect {
    use egui_kittest::kittest::Queryable;
    let rects: Vec<Rect> = h.get_all_by_label(label).map(|n| n.rect()).collect();
    *rects
        .iter()
        .find(|r| pick(**r))
        .unwrap_or_else(|| panic!("{label}: {rects:?}"))
}

/// メニューバーの見出し（同じ名前のドックのタブより上にある）。
pub fn menu_title(h: &Harness<'_, YoluApp>, label: &str) -> Rect {
    rect_of(h, label, |r| r.top() < 24.0)
}

/// 開いているポップアップの中の項目。
pub fn popup_item(h: &Harness<'_, YoluApp>, label: &str) -> Rect {
    let body = h
        .state()
        .state
        .popup
        .as_ref()
        .expect("popup open")
        .state
        .rect;
    rect_of(h, label, |r| body.contains(r.center()))
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

/// ドックのタブのボタンを押す。
pub fn click_tab(h: &mut Harness<'_, YoluApp>, tab: yolu_app::Tab) {
    let at = h.state().tab_rects.get(&tab).expect("tab shown").center();
    click(h, at);
}
