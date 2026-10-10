//! 画面の試験の共通のツール（egui_kittest。描画は wgpu のソフトの描画で、コンテナでも回る）。
#![allow(dead_code)]

pub mod canvas_device;
pub mod core_refused;
pub mod fbx;
pub mod gpu_thread;
pub mod livelink;
pub mod rulers;
pub mod shared_gpu;
pub mod tmp;
pub mod viewports;
pub mod wait;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use egui::{pos2, Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::pen::{PenInput, WindowMover};
use yolu_app::state::AppState;
use yolu_app::YoluApp;

/// 試験の「ペン・指でウィンドウを動かす手」（Windows のウィンドウの代わり）: 呼ばれた回数を数え、`answer` が true の間だけ「動かし始めた」と答える。
/// 渡した `StartDrag`・毎フレームの見張りの呼び出しも数える。
pub struct TestMover {
    calls: AtomicUsize,
    answer: AtomicBool,
    handed: AtomicUsize,
    polls: AtomicUsize,
}

impl TestMover {
    pub fn new(answer: bool) -> Arc<TestMover> {
        Arc::new(TestMover {
            calls: AtomicUsize::new(0),
            answer: AtomicBool::new(answer),
            handed: AtomicUsize::new(0),
            polls: AtomicUsize::new(0),
        })
    }

    /// `begin`（ペン・指で動かし始める）を呼ばれた回数。
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// 以後の答えを変える（触れているポインタが分かる・分からない）。
    pub fn answer(&self, touching: bool) {
        self.answer.store(touching, Ordering::SeqCst);
    }

    /// OS の移動の輪に渡した `StartDrag` の数。
    pub fn handed(&self) -> usize {
        self.handed.load(Ordering::SeqCst)
    }

    /// 毎フレームの見張りの呼び出しの数。
    pub fn polls(&self) -> usize {
        self.polls.load(Ordering::SeqCst)
    }
}

impl WindowMover for TestMover {
    fn begin(&self) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.answer.load(Ordering::SeqCst)
    }

    fn start_drag_handed(&self) {
        self.handed.fetch_add(1, Ordering::SeqCst);
    }

    fn poll(&self) {
        self.polls.fetch_add(1, Ordering::SeqCst);
    }
}

/// ペン・指の接触の始まりの入力（winit が `WM_POINTER` から作る Touch。egui の左ボタンの押しの前に付く）。
pub fn touch_start(at: Pos2) -> Event {
    Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(1),
        phase: egui::TouchPhase::Start,
        pos: at,
        force: Some(0.5),
    }
}

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

/// 幅 `width` のウィンドウの既定の並びで、中央の 3D ビューとキャンバスを 1 つの組（キャンバス、3D ビューの順のタブ。キャンバスが前）にした並び。ほかの組・割合は既定のまま。
/// 3D ビューとキャンバスを同時に出すと、どちらも表示域が半分になる。描く・3D を見る試験の座標は、片方だけが広く出ている前提で書いてあるので、
/// 試験のウィンドウ（`app`）はこの並びで立ち上げる。
pub fn tabbed_center_dock(width: f32) -> egui_dock::DockState<yolu_app::Tab> {
    use egui_dock::{TabDestination, TabInsert};
    use yolu_app::Tab;
    let mut dock = yolu_app::app::default_dock_for(width);
    let from = dock.find_tab(&Tab::View3d).expect("3D ビュー");
    let to = dock.find_tab(&Tab::Canvas).expect("キャンバス").node_path();
    dock.move_tab(from, TabDestination::Node(to, TabInsert::Append));
    let canvas = dock
        .find_tab(&Tab::Canvas)
        .expect("キャンバス（動かしたあと）");
    dock.set_active_tab(canvas).expect("前へ");
    dock
}

/// ウィンドウの全体（eframe の App として）。文書は size × size。中央は 1 つの組（`tabbed_center_dock`）。
pub fn app(width: f32, height: f32, size: u32) -> Harness<'static, YoluApp> {
    app_with_renderer(width, height, size, shared_gpu::renderer())
}

/// `app` の、既定の並びのままの版（中央は 3D ビューが左・キャンバスが右）。
pub fn app_default(width: f32, height: f32, size: u32) -> Harness<'static, YoluApp> {
    let mut h = app(width, height, size);
    h.state_mut().dock = yolu_app::app::default_dock_for(width);
    h.run();
    h
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
            let mut app = YoluApp::for_context(
                &cc.egui_ctx,
                AppState::new(size, size),
                PenInput::detached(),
            );
            // 中央は 1 つの組（キャンバスと 3D ビューのタブ）。既定の並び（3D ビューとキャンバスを左右に並べる）を見る試験は `app_default` を使う
            app.dock = tabbed_center_dock(width);
            with_render_state_cpu_canvas(app, cc.wgpu_render_state.as_ref())
        });
    // 焼く場所は CPU に固定（ハードウェアの GPU がある機械でも、試験の結果と画面を揺らさない）。GPU の試験は自分で選ぶ。
    h.state_mut().state.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    h.run();
    remember_right_column(&h);
    h
}

thread_local! {
    static RIGHT_X: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// 右の列（レイヤーの組）の左端を覚える。ウィンドウの大きさで右の列の位置が変わるので、試験は固定の座標でなく `rx()` で右の列かどうかを見分ける。
fn remember_right_column(h: &Harness<'_, YoluApp>) {
    if let Some(r) = h.state().tab_rects.get(&yolu_app::Tab::Layers) {
        RIGHT_X.with(|x| x.set(r.left() - 1.0));
    }
}

/// 右の列の左端（直近に作ったウィンドウの、レイヤーの組の左端より 1 点左）。`r.left() > rx()` なら右の列の部品。
pub fn rx() -> f32 {
    RIGHT_X.with(|x| x.get())
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

/// 右のドックの「マテリアル」のパネル（テクスチャセットの見た目）を前に出す。
pub fn open_material(h: &mut Harness<'_, YoluApp>) {
    click_tab(h, yolu_app::Tab::Material);
    h.run();
}

/// ツールプロパティの「塗るチャンネル」の区分（初めは閉じている）を開く。
pub fn open_paint_channels(h: &mut Harness<'_, YoluApp>) {
    h.state_mut()
        .state
        .ui
        .sections
        .insert("paint-channels", true);
    h.run();
}

/// `tab` を `mate` のタブの組へ入れて前に出す（既定の並びで 1 つだけのタブの組に、別のタブを重ねたいとき）。
pub fn put_in_group_of(h: &mut Harness<'_, YoluApp>, tab: yolu_app::Tab, mate: yolu_app::Tab) {
    use egui_dock::{TabDestination, TabInsert};
    let dock = &mut h.state_mut().dock;
    let from = dock.find_tab(&tab).expect("動かすタブ");
    let to = dock.find_tab(&mate).expect("入れる組").node_path();
    dock.move_tab(from, TabDestination::Node(to, TabInsert::Append));
    let moved = dock.find_tab(&tab).expect("動かしたあとのタブ");
    dock.set_active_tab(moved).expect("前へ");
    h.run();
}

/// プロパティの組の中身の矩形（組のタブの帯の下から、ウィンドウの下端まで。右はウィンドウの右）。
pub fn props_body(h: &Harness<'_, YoluApp>) -> Rect {
    let tab = h.state().tab_rects[&yolu_app::Tab::Properties];
    let window = h.ctx.content_rect();
    Rect::from_min_max(pos2(tab.left() - 2.0, tab.bottom()), window.right_bottom())
}

/// プロパティの組の中身の縦の中ほど。組を送って部品を見える所へ置くときの目安。
pub fn props_mid(h: &Harness<'_, YoluApp>) -> f32 {
    props_body(h).center().y
}

/// 右の列の上の組（テクスチャセット・ナビゲーター・チャンネル・アセット）の中身の矩形（組のタブの帯の下から、レイヤーの組のタブの上まで。右はウィンドウの右）。
pub fn top_right_body(h: &Harness<'_, YoluApp>) -> Rect {
    let tabs = &h.state().tab_rects;
    let (first, layers) = (
        tabs[&yolu_app::Tab::TextureSets],
        tabs[&yolu_app::Tab::Layers],
    );
    Rect::from_min_max(
        pos2(first.left() - 2.0, first.bottom()),
        pos2(h.ctx.content_rect().right(), layers.top()),
    )
}

/// レイヤーの組の中身の矩形（組のタブの帯の下から、プロパティの組のタブの上まで。右はウィンドウの右）。
pub fn layers_body(h: &Harness<'_, YoluApp>) -> Rect {
    let tabs = &h.state().tab_rects;
    let (layers, properties) = (
        tabs[&yolu_app::Tab::Layers],
        tabs[&yolu_app::Tab::Properties],
    );
    Rect::from_min_max(
        pos2(layers.left() - 2.0, layers.bottom()),
        pos2(h.ctx.content_rect().right(), properties.top()),
    )
}

/// 右の列（レイヤーの組を含む列）を、中央（キャンバスの組）との分け方の取り分 `fraction`（中央の取り分。大きいほど右の列が狭い）に動かす。既定の右の列はどのウィンドウでも幅が広いので、
/// 狭い列の振る舞い（名前を詰めずに積むなど）を確かめる試験が使う。
pub fn set_center_share(h: &mut Harness<'_, YoluApp>, fraction: f32) {
    use egui_dock::{Node, NodeIndex, Tree};
    fn holds(tree: &Tree<yolu_app::Tab>, at: usize, tab: yolu_app::Tab) -> bool {
        match tree.iter().nth(at) {
            Some(Node::Leaf(leaf)) => leaf.tabs.contains(&tab),
            Some(Node::Vertical(_) | Node::Horizontal(_)) => {
                holds(tree, 2 * at + 1, tab) || holds(tree, 2 * at + 2, tab)
            }
            _ => false,
        }
    }
    let tree = h.state_mut().dock.main_surface_mut();
    let found = (0..tree.iter().count()).find(|&i| {
        matches!(tree.iter().nth(i), Some(Node::Horizontal(_)))
            && holds(tree, 2 * i + 2, yolu_app::Tab::Layers)
            && holds(tree, 2 * i + 1, yolu_app::Tab::Canvas)
    });
    if let Some(i) = found {
        if let Node::Horizontal(split) = &mut tree[NodeIndex(i)] {
            split.fraction = fraction;
        }
    }
    h.run();
}
