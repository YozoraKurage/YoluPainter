//! 描いている間のパネルの見た目。線を引いている間（マウスでもペンでも、2D のキャンバスでも 3D ビューでも）、キャンバス・3D ビューの外の画素は、
//! 描き始める前の絵のまま変わらない（押せない部品が灰色に変わって、離すと戻る点滅が出ない）。ドックの全タブ・全ツールの欄・プロパティのタブと
//! レイヤーの種類ごとの欄・描き始めたフレームの途中も同じ。押せないこと（操作を受けないこと）は変わらず、描き始める前から押せなかった部品は押せない
//! 見た目のまま、離した後は今の状態の見た目に戻る。部品だけを並べたウィンドウでも、同じ決まりを部品ごとに確かめる。
//! 絵の差は、差のある矩形を失敗の文に出す。`STROKE_LOOK_DUMP=<フォルダ>` を付けると、比べた 2 枚の絵をそのフォルダへ書く。
use crate::common;

use common::*;
use egui::{pos2, vec2, Event, PointerButton, Pos2, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use image::RgbaImage;
use yolu_app::m2::Edit;
use yolu_app::newproject::NpAction;
use yolu_app::pen::PenSample;
use yolu_app::state::{Action, Tool};
use yolu_app::{Tab, YoluApp};

type H = Harness<'static, YoluApp>;

const WIDTH: f32 = 1600.0;
const HEIGHT: f32 = 1000.0;

/// レイヤー（描くレイヤー・塗りつぶしレイヤー）・ブラシのパネル・テクスチャセット・プロパティが写るウィンドウ。描くレイヤーを選んでいる。
fn window() -> H {
    window_sized(HEIGHT)
}

/// `window` の、高さを選べる形（欄の下の方の行までウィンドウに入れるとき）。
fn window_sized(height: f32) -> H {
    let mut h = app(WIDTH, height, 128);
    {
        let s = &mut h.state_mut().state;
        // 2 つ目のテクスチャセット（今のセットになる）に、描くレイヤー・塗りつぶしレイヤー・描くレイヤーを重ねる
        s.apply(Action::Project(NpAction::AddSet));
        s.apply(Action::NewLayer);
        s.apply(Action::M2(Edit::NewFill));
        s.apply(Action::NewLayer);
    }
    h.run();
    // 1 本描いておく（描いた色が最近の色に入る。描き始めに増える物を、描いている間の差と取り違えない）
    let c = canvas_rect(&h).center();
    drag(&mut h, &[c - vec2(40.0, 0.0), c + vec2(40.0, 10.0)]);
    // 知らせ（追加したレイヤー・セット）が消えるまで待つ（出入りの動きを、描いている間の差と取り違えない）
    for _ in 0..((yolu_app::toast::INFO_SECONDS + 1.0) * 60.0) as usize {
        h.step();
    }
    h.run();
    h
}

/// ウィンドウに 3D ビューを開き、試しの立方体を読む。
fn window_3d() -> (H, Rect) {
    let mut h = window();
    h.state_mut().state.view3d.load_demo();
    h.state_mut().state.view3d.camera.yaw = -40.0;
    h.state_mut().state.view3d.camera.pitch = 15.0;
    click_tab(&mut h, Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect)
}

/// 描いている間に変わってよい所: 描いているビュー（キャンバス・3D ビュー）、レイヤーの画素の縮小図とナビゲーターの全体像（描いた点が写る）。
fn live_areas(h: &H, view: Rect) -> Vec<Rect> {
    let mut areas = vec![view];
    for label in [
        "レイヤーの画素",
        "Layer pixels",
        "レイヤーマスク（押すとマスクが対象）",
        "Layer mask (click to target the mask)",
        "ナビゲーター",
        "Navigator",
    ] {
        areas.extend(h.query_all_by_label(label).map(|n| n.rect().expand(3.0)));
    }
    areas
}

fn shot(h: &mut H) -> RgbaImage {
    h.render().expect("描画")
}

/// 絵が止まるまで待って撮る（別のスレッドで作る縮小図などが届くまで。実際の時間で最大 3 秒）。
fn settled_shot(h: &mut H) -> RgbaImage {
    let mut last = shot(h);
    for _ in 0..60 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        h.run();
        let now = shot(h);
        if now == last {
            return now;
        }
        last = now;
    }
    last
}

/// 2 枚の絵の差のある画素のうち、`holes`（キャンバスなど、変わってよい矩形）の外のものを、近い画素どうしの矩形にまとめて返す。
/// 各行は「矩形: 差のある画素の数（前の色 → 後の色）」。
fn differences(before: &RgbaImage, after: &RgbaImage, holes: &[Rect]) -> Vec<String> {
    assert_eq!(before.dimensions(), after.dimensions());
    const CELL: u32 = 8;
    let (w, h) = before.dimensions();
    let (cols, rows) = (w.div_ceil(CELL), h.div_ceil(CELL));
    let mut cells = vec![0u32; (cols * rows) as usize];
    let mut sample: Vec<Option<([u8; 4], [u8; 4])>> = vec![None; (cols * rows) as usize];
    for y in 0..h {
        for x in 0..w {
            let at = pos2(x as f32 + 0.5, y as f32 + 0.5);
            if holes.iter().any(|hole| hole.contains(at)) {
                continue;
            }
            let (a, b) = (before.get_pixel(x, y).0, after.get_pixel(x, y).0);
            if a != b {
                let i = ((y / CELL) * cols + x / CELL) as usize;
                cells[i] += 1;
                sample[i].get_or_insert((a, b));
            }
        }
    }
    // 隣り合う（斜めも）セルをまとめる
    let mut seen = vec![false; cells.len()];
    let mut out = Vec::new();
    for start in 0..cells.len() {
        if cells[start] == 0 || seen[start] {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1, mut count) = (u32::MAX, u32::MAX, 0, 0, 0);
        let first = sample[start];
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let (cx, cy) = (i as u32 % cols, i as u32 / cols);
            x0 = x0.min(cx * CELL);
            y0 = y0.min(cy * CELL);
            x1 = x1.max((cx + 1) * CELL);
            y1 = y1.max((cy + 1) * CELL);
            count += cells[i];
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (cx as i32 + dx, cy as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= cols as i32 || ny >= rows as i32 {
                        continue;
                    }
                    let n = (ny as u32 * cols + nx as u32) as usize;
                    if cells[n] > 0 && !seen[n] {
                        seen[n] = true;
                        stack.push(n);
                    }
                }
            }
        }
        let (a, b) = first.unwrap();
        out.push(format!(
            "x {x0}..{x1} y {y0}..{y1}: {count} 画素（{a:?} → {b:?}）"
        ));
    }
    out
}

/// 2 枚の絵の、矩形 `r` の中が同じか。
fn same_in(a: &RgbaImage, b: &RgbaImage, r: Rect) -> bool {
    (r.top() as u32..r.bottom() as u32).all(|y| {
        (r.left() as u32..r.right() as u32).all(|x| a.get_pixel(x, y) == b.get_pixel(x, y))
    })
}

fn assert_same_outside(before: &RgbaImage, after: &RgbaImage, holes: &[Rect], what: &str) {
    let diffs = differences(before, after, holes);
    if let Ok(dir) = std::env::var("STROKE_LOOK_DUMP") {
        let name: String = what.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        before.save(format!("{dir}/{name}-before.png")).unwrap();
        after.save(format!("{dir}/{name}-after.png")).unwrap();
    }
    assert!(
        diffs.is_empty(),
        "{what}: 描いている間にビューの外の絵が変わった（{} か所）\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}

fn sample(at: Pos2, contact: bool, pressure: f32) -> PenSample {
    PenSample {
        pos: [at.x, at.y],
        pressure,
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 5,
        time_ms: 0,
    }
}

/// ペンの点 1 つ（`emulate` なら、winit が同じウィンドウのメッセージから作る egui の入力も同じフレームへ入れる）。
fn pen_frame(h: &mut H, point: PenSample, events: Vec<Event>, emulate: bool) {
    h.state().pen().push(point);
    if emulate {
        h.input_mut().events.extend(events);
    }
    h.step();
}

fn pen_down(h: &mut H, at: Pos2, pressure: f32, emulate: bool) {
    pen_frame(
        h,
        sample(at, false, 0.0),
        vec![Event::PointerMoved(at)],
        emulate,
    );
    pen_frame(
        h,
        sample(at, true, pressure),
        vec![
            Event::Touch {
                device_id: egui::TouchDeviceId(0),
                id: egui::TouchId(5),
                phase: egui::TouchPhase::Start,
                pos: at,
                force: Some(pressure),
            },
            Event::PointerMoved(at),
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        emulate,
    );
}

fn pen_to(h: &mut H, at: Pos2, pressure: f32, emulate: bool) {
    pen_frame(
        h,
        sample(at, true, pressure),
        vec![
            Event::Touch {
                device_id: egui::TouchDeviceId(0),
                id: egui::TouchId(5),
                phase: egui::TouchPhase::Move,
                pos: at,
                force: Some(pressure),
            },
            Event::PointerMoved(at),
        ],
        emulate,
    );
}

fn pen_up(h: &mut H, at: Pos2, emulate: bool) {
    pen_frame(
        h,
        sample(at, false, 0.0),
        vec![
            Event::Touch {
                device_id: egui::TouchDeviceId(0),
                id: egui::TouchId(5),
                phase: egui::TouchPhase::End,
                pos: at,
                force: None,
            },
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
            Event::PointerGone,
        ],
        emulate,
    );
    h.run();
}

fn stroking(h: &H) -> bool {
    h.state().state.is_stroking()
}

/// 描く前の絵を撮り、`start` で線を引き始めて（離さない）、もう 1 枚撮る。ビュー `view` の外が同じことを確かめる。
fn assert_panels_unchanged(h: &mut H, view: Rect, what: &str, mut start: impl FnMut(&mut H)) {
    // ポインタはビューの上（部品にかからない所）に置いてから撮る
    move_to(h, view.center());
    h.run();
    let before = settled_shot(h);
    let holes = live_areas(h, view);
    start(h);
    assert!(stroking(h), "{what}: 描き始めた");
    let during = shot(h);
    assert_same_outside(&before, &during, &holes, &format!("{what}（描いている間）"));
}

fn mouse_stroke(h: &mut H, c: Pos2) -> Pos2 {
    press(h, c, PointerButton::Primary);
    h.step();
    move_to(h, c + vec2(30.0, 10.0));
    h.step();
    move_to(h, c + vec2(60.0, 20.0));
    h.step();
    c + vec2(60.0, 20.0)
}

#[test]
fn the_panels_keep_their_look_while_the_mouse_paints_on_the_canvas() {
    let mut h = window();
    let canvas = canvas_rect(&h);
    let mut end = Pos2::ZERO;
    assert_panels_unchanged(&mut h, canvas, "マウス・2D", |h| {
        end = mouse_stroke(h, canvas.center());
    });
    release(&h, end, PointerButton::Primary);
    h.run();
}

#[test]
fn the_panels_keep_their_look_while_the_pen_paints_on_the_canvas() {
    for emulate in [false, true] {
        let mut h = window();
        let canvas = canvas_rect(&h);
        let c = canvas.center();
        assert_panels_unchanged(&mut h, canvas, "ペン・2D", |h| {
            pen_down(h, c, 0.6, emulate);
            pen_to(h, c + vec2(30.0, 10.0), 0.8, emulate);
            pen_to(h, c + vec2(60.0, 20.0), 0.3, emulate);
        });
        pen_up(&mut h, c + vec2(60.0, 20.0), emulate);
        assert!(!stroking(&h));
    }
}

#[test]
fn the_panels_keep_their_look_while_the_mouse_paints_in_the_3d_view() {
    let (mut h, rect) = window_3d();
    let mut end = Pos2::ZERO;
    assert_panels_unchanged(&mut h, rect, "マウス・3D", |h| {
        press(h, rect.center(), PointerButton::Primary);
        h.step();
        move_to(h, rect.center() + vec2(20.0, 8.0));
        h.step();
        move_to(h, rect.center() + vec2(40.0, 16.0));
        h.step();
        end = rect.center() + vec2(40.0, 16.0);
    });
    release(&h, end, PointerButton::Primary);
    h.run();
}

#[test]
fn the_panels_keep_their_look_while_the_pen_paints_in_the_3d_view() {
    let (mut h, rect) = window_3d();
    let c = rect.center();
    assert_panels_unchanged(&mut h, rect, "ペン・3D", |h| {
        pen_down(h, c, 0.7, true);
        pen_to(h, c + vec2(20.0, 8.0), 0.9, true);
        pen_to(h, c + vec2(40.0, 16.0), 0.4, true);
    });
    pen_up(&mut h, c + vec2(40.0, 16.0), true);
}

/// ドックのどのタブを前にしていても、描いている間の見た目は変わらない。
#[test]
fn every_dock_tab_keeps_its_look_while_stroking() {
    let mut h = window();
    // ナビゲーターは既定の並びでは閉じているので、「ウィンドウ」のメニューと同じ操作で開く
    h.state_mut()
        .state
        .apply(Action::Dock(yolu_app::detach::DockOp::Show(Tab::Navigator)));
    h.run();
    let canvas = canvas_rect(&h);
    for tab in [
        Tab::Assets,
        Tab::Channels,
        Tab::ColorSets,
        Tab::Navigator,
        Tab::Log,
        Tab::History,
        Tab::SubTools,
        Tab::Color,
        Tab::TextureSets,
        Tab::Layers,
        Tab::Properties,
        Tab::Material,
    ] {
        click_tab(&mut h, tab);
        h.run();
        let mut end = Pos2::ZERO;
        assert_panels_unchanged(&mut h, canvas, &format!("タブ {tab:?}"), |h| {
            end = mouse_stroke(h, canvas.center());
        });
        release(&h, end, PointerButton::Primary);
        h.run();
        assert!(!stroking(&h));
    }
}

/// プロパティのどのタブ（ステンシル・レイヤー）でも、レイヤーに描いてもマスクに描いても、描いている間の見た目は変わらない。
#[test]
fn every_property_tab_keeps_its_look_while_stroking() {
    for mask in [false, true] {
        let mut h = window();
        if mask {
            let id = h
                .state()
                .state
                .selected_layer
                .expect("レイヤーを選んでいる");
            h.state_mut().state.apply(Action::M2(Edit::AddMask(id)));
            h.state_mut().state.m2.edit_mask = true;
            h.run();
        }
        let canvas = canvas_rect(&h);
        for tab in 0..yolu_app::panels::properties::TAB_ICONS.len() {
            h.state_mut().state.ui.property_tab = tab;
            h.run();
            let mut end = Pos2::ZERO;
            assert_panels_unchanged(
                &mut h,
                canvas,
                &format!("プロパティのタブ {tab}（マスク {mask}）"),
                |h| end = mouse_stroke(h, canvas.center()),
            );
            release(&h, end, PointerButton::Primary);
            h.run();
            assert!(!stroking(&h));
        }
    }
}

/// ツールごとに、オプションバー・左のパネルの見た目が変わらない（描き始めるツールだけ。ほかは描き始めないので何も確かめない）。
#[test]
fn every_tool_keeps_its_look_while_stroking() {
    let mut h = window();
    let canvas = canvas_rect(&h);
    let mut tried = Vec::new();
    for tool in Tool::ALL {
        h.state_mut().state.apply(Action::SelectTool(tool));
        h.run();
        move_to(&h, canvas.center());
        h.run();
        let before = shot(&mut h);
        let holes = live_areas(&h, canvas);
        let end = mouse_stroke(&mut h, canvas.center());
        if stroking(&h) {
            let during = shot(&mut h);
            assert_same_outside(&before, &during, &holes, &format!("ツール {}", tool.id()));
            tried.push(tool.id());
        }
        release(&h, end, PointerButton::Primary);
        h.run();
        // 押したままの途中の状態を捨てる（次のツールへ）
        key(&h, egui::Key::Escape, egui::Modifiers::NONE);
        h.run();
        assert!(!stroking(&h), "{}: 離しても描き終わらない", tool.id());
    }
    assert!(tried.len() >= 3, "描き始めたツールが少ない: {tried:?}");
}

/// どのツールの欄（オプションバー・左のパネル）も、描いている間は見た目が変わらない。マウスの押しで描き始めないツール（バケツ・選択・パスなど）
/// の欄も確かめるため、文書の側でストロークを始める（`is_stroking` が真になるのは、押しで始めた描きでも文書の描きでも同じ）。
#[test]
fn every_tools_panels_keep_their_look_while_a_document_stroke_is_active() {
    let mut h = window();
    let canvas = canvas_rect(&h);
    for tool in Tool::ALL {
        h.state_mut().state.apply(Action::SelectTool(tool));
        h.run();
        move_to(&h, canvas.center());
        h.run();
        let before = settled_shot(&mut h);
        let holes = live_areas(&h, canvas);
        {
            let s = &mut h.state_mut().state;
            let layer = s.selected_layer.expect("レイヤーを選んでいる");
            let brush = s.stroke_settings(false);
            // 札は捨てても、文書のストロークは進行中のまま（`cancel_active_stroke` で取り消す）
            let _ = s.doc.begin_stroke(layer, &brush).unwrap();
        }
        assert!(stroking(&h), "{}: 文書のストロークが進行中", tool.id());
        h.step();
        h.step();
        let during = shot(&mut h);
        assert_same_outside(
            &before,
            &during,
            &holes,
            &format!("ツールの欄 {}", tool.id()),
        );
        assert!(h.state_mut().state.doc.cancel_active_stroke());
        h.run();
        assert!(!stroking(&h));
    }
}

/// レイヤーの種類・プロパティのタブごとの欄（ステンシルの画像・lilToon の節・塗りつぶしレイヤー・調整・グループ）も、描いている間は見た目が変わらない。
/// 欄が出ているレイヤーとは別の、描けるレイヤーに、文書の側でストロークを始める。
#[test]
fn every_property_context_keeps_its_look_while_a_document_stroke_is_active() {
    use yolu_app::m2::AdjustmentKind;
    use yolu_app::stencil::StencilOp;
    let dir = common::tmp::test_dir("stroke-look");
    let png = dir.join("stencil.png");
    image::RgbaImage::from_fn(32, 32, |x, _| {
        image::Rgba(if x < 16 {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 255]
        })
    })
    .save(&png)
    .unwrap();
    // 高いウィンドウ（プロパティの欄の下の方の行まで描く）
    let mut h = window_sized(3200.0);
    {
        let s = &mut h.state_mut().state;
        s.apply(Action::Stencil(StencilOp::Load(png)));
        // lilToon の節を全部開く
        for key in [
            "look.main",
            "look.base",
            "look.lighting",
            "look.shadow",
            "look.rim",
            "look.rimshade",
            "look.emission",
            "look.normal",
            "look.outline",
            "look.reflection",
            "look.specular",
            "look.matcap",
            "look.glitter",
            "look.backlight",
            "look.distancefade",
            "look.alpha",
            "look.decal",
            "look.uv",
            "look.paint",
            "look.remap",
            "look.follow",
            "look.plane",
        ] {
            s.ui.sections.insert(key, true);
        }
    }
    h.run();
    let raster = h
        .state()
        .state
        .selected_layer
        .expect("レイヤーを選んでいる");
    // 描けるレイヤーのほかに、塗りつぶし・調整・グループのレイヤーを足す（足したレイヤーが選ばれる）
    let mut others = Vec::new();
    {
        let s = &mut h.state_mut().state;
        s.apply(Action::M2(Edit::NewFill));
        others.push(("塗りつぶし", s.selected_layer.unwrap()));
        s.apply(Action::M2(Edit::NewAdjustment(AdjustmentKind::Invert)));
        others.push(("調整", s.selected_layer.unwrap()));
        s.apply(Action::M2(Edit::NewGroup));
        others.push(("グループ", s.selected_layer.unwrap()));
    }
    h.run();
    let canvas = canvas_rect(&h);
    let mut cases = vec![
        ("描けるレイヤー", raster, 0),
        ("描けるレイヤー", raster, 1),
        ("描けるレイヤー", raster, 2),
    ];
    for (name, id) in others {
        cases.push((name, id, 2));
    }
    for (n, (name, id, tab)) in cases.into_iter().enumerate() {
        {
            let s = &mut h.state_mut().state;
            s.selected_layer = Some(id);
            s.ui.property_tab = tab;
        }
        h.run();
        move_to(&h, canvas.center());
        h.run();
        let before = settled_shot(&mut h);
        let holes = live_areas(&h, canvas);
        {
            let s = &mut h.state_mut().state;
            let brush = s.stroke_settings(false);
            // 札は捨てても、文書のストロークは進行中のまま（`cancel_active_stroke` で取り消す）
            let _ = s.doc.begin_stroke(raster, &brush).unwrap();
        }
        assert!(stroking(&h));
        h.step();
        h.step();
        let during = shot(&mut h);
        assert_same_outside(
            &before,
            &during,
            &holes,
            &format!("context{n} {name} tab{tab}"),
        );
        assert!(h.state_mut().state.doc.cancel_active_stroke());
        h.run();
        assert!(!stroking(&h));
    }
}

/// 描いている間にパネルのボタンを押しても、操作は走らない（押す判定は描き始める前のまま。見た目だけ保つ）。
#[test]
fn panel_buttons_do_nothing_while_stroking() {
    let mut h = window();
    let canvas = canvas_rect(&h);
    let c = canvas.center();
    let layers_before = h.state().state.doc.layers().len();
    // ペンで描き始め（egui のポインタは使わない）、マウスでレイヤーの追加のボタンを押す
    pen_down(&mut h, c, 0.6, false);
    pen_to(&mut h, c + vec2(30.0, 10.0), 0.8, false);
    assert!(stroking(&h));
    let add = h.get_by_label("新規レイヤー");
    assert!(add.accesskit_node().is_disabled(), "描いている間は押せない");
    let at = add.rect().center();
    click(&mut h, at);
    pen_to(&mut h, c + vec2(40.0, 12.0), 0.8, false);
    assert!(stroking(&h), "押しても描きは止まらない");
    assert_eq!(h.state().state.doc.layers().len(), layers_before);
    pen_up(&mut h, c + vec2(40.0, 12.0), false);
    // 描き終えれば同じボタンが押せる（対照）
    assert!(!h
        .get_by_label("新規レイヤー")
        .accesskit_node()
        .is_disabled());
    let at = h.get_by_label("新規レイヤー").rect().center();
    click(&mut h, at);
    assert_eq!(h.state().state.doc.layers().len(), layers_before + 1);
}

/// 描いている間に今の状態が変わっても（読むだけのセットになるなど）、レイヤーの操作の帯は描き始める前の見た目のまま。離したあとは、今の状態の見た目になる。
#[test]
fn after_the_release_the_layer_buttons_follow_the_state_again() {
    // 読むだけの状態から始めたウィンドウの、削除のボタンの絵（押せない見た目の見本）
    let (reference, button) = {
        let mut h = window();
        h.state_mut().state.sets.get_mut(1).unwrap().read_only = Some("試験".into());
        h.run();
        move_to(&h, canvas_rect(&h).center());
        h.run();
        let button = h.get_by_label("レイヤーを削除").rect();
        (shot(&mut h), button)
    };
    let mut h = window();
    let canvas = canvas_rect(&h);
    move_to(&h, canvas.center());
    h.run();
    let before = shot(&mut h);
    assert_eq!(h.get_by_label("レイヤーを削除").rect(), button);
    let end = mouse_stroke(&mut h, canvas.center());
    assert!(stroking(&h));
    // 描いている間に読むだけになる
    h.state_mut().state.sets.get_mut(1).unwrap().read_only = Some("試験".into());
    h.step();
    let during = shot(&mut h);
    release(&h, end, PointerButton::Primary);
    h.run();
    move_to(&h, canvas.center());
    h.run();
    let after = shot(&mut h);
    let same_in = |a: &RgbaImage, b: &RgbaImage| same_in(a, b, button.expand(2.0));
    assert!(
        same_in(&before, &during),
        "描いている間は、描き始める前の見た目"
    );
    assert!(
        !same_in(&before, &reference),
        "読むだけの見た目と、描き始める前の見た目は違う（確かめが空振りしていない）"
    );
    assert!(same_in(&reference, &after), "離した後は、今の状態の見た目");
}

// ───────── 部品だけの試験（描いている間の見た目の決まりを、部品ごとに） ─────────

/// 部品を並べた見本。`stroking` は描いている間、`base` は描いていないときに押せるか（描いている間は、呼ぶ側が `!stroking` を掛けて押せなくする）。
#[derive(Default)]
struct Parts {
    ready: bool,
    stroking: bool,
    /// このフレームの途中の、この行から描き始める（描き始めたフレームは、上の部品を描いたあとにキャンバスが描き始める形）。
    begins_at: Option<usize>,
    base: [bool; 3],
    /// 描いている間に初めて出る部品を出すか。
    show_late: bool,
    clicks: [u32; 3],
    on: bool,
    value: f32,
    number: f64,
    scoped_value: f32,
}

const PARTS_ROWS: usize = 12;

fn part_row(origin: Pos2, i: usize) -> Rect {
    Rect::from_min_size(origin + vec2(8.0, 8.0 + 34.0 * i as f32), vec2(300.0, 28.0))
}

fn draw_parts(ui: &mut egui::Ui, g: &mut Parts) {
    use yolu_app::ui::numfield::{number_field, NumSpec};
    use yolu_app::ui::widgets as w;
    if !g.ready {
        // 書体は次のフレームから効く（このフレームで太字の書体を使うと egui が止まる）ので、初めのフレームは準備だけ
        YoluApp::setup(ui.ctx());
        g.ready = true;
        ui.ctx().request_repaint();
        return;
    }
    w::begin_stroke_frame(ui.ctx(), g.stroking);
    let area = Rect::from_min_size(
        ui.max_rect().min,
        vec2(320.0, 8.0 + 34.0 * PARTS_ROWS as f32),
    );
    ui.allocate_rect(area, egui::Sense::hover());
    ui.painter()
        .rect_filled(area, 0.0, yolu_app::ui::theme::PANEL_BG);
    let row = |i| part_row(area.min, i);
    // 部品の行ごとに、いま描いているかを見る（`begins_at` の行から、描いている）
    let begin = g.begins_at;
    let mut now = g.stroking;
    macro_rules! free {
        ($i:expr) => {{
            if begin == Some($i) && !now {
                now = true;
                w::update_stroke_hold(ui.ctx(), true);
            }
            !now
        }};
    }
    for i in 0..3 {
        let free = free!(i);
        if w::button(
            ui,
            row(i),
            ("part.button", i),
            "OK",
            i == 2,
            g.base[i] && free,
            None,
            None,
        )
        .clicked()
        {
            g.clicks[i] += 1;
        }
    }
    let free = free!(3);
    w::icon_button(
        ui,
        Rect::from_min_size(row(3).min, vec2(28.0, 28.0)),
        "part.icon",
        "layers",
        "アイコン",
        false,
        g.base[0] && free,
        16.0,
    );
    let free = free!(4);
    let spec =
        w::SliderSpec::new("値", 0.0, 100.0, w::NumberFormat::int("%")).enabled(g.base[0] && free);
    let out = w::slider(ui, row(4), "part.slider", g.value, &spec);
    if out.changed {
        g.value = out.value;
    }
    let free = free!(5);
    let spec = w::SliderSpec::new("二行", 0.0, 100.0, w::NumberFormat::int("%"))
        .enabled(g.base[1] && free);
    w::slider(
        ui,
        Rect::from_min_size(row(5).min, vec2(300.0, 34.0)),
        "part.slider2",
        40.0,
        &spec,
    );
    let free = free!(6);
    g.on = w::toggle(
        ui,
        row(6),
        "part.toggle",
        "チェック",
        g.on,
        None,
        g.base[0] && free,
    );
    let free = free!(7);
    w::dropdown(
        ui,
        row(7),
        "part.dropdown",
        Some("選び"),
        "通常",
        None,
        g.base[0] && free,
        80.0,
    );
    let free = free!(8);
    let out = number_field(
        ui,
        row(8),
        "part.number",
        "X",
        g.number,
        &NumSpec::new(-100.0, 100.0, 0.5, 1),
        None,
        None,
        g.base[0] && free,
    );
    if out.changed {
        g.number = out.value;
    }
    let free = free!(9);
    w::color_swatch(
        ui,
        row(9),
        "part.swatch",
        [0.8, 0.3, 0.2, 1.0],
        "色",
        g.base[0] && free,
    );
    // `ui.add_enabled_ui` の代わりの入れ物（中の部品は自分の `enabled` を持たず、入れ物の押せるかに従う）
    let free = free!(10);
    w::enabled_scope(ui, "part.scope", g.base[0] && free, |ui| {
        let spec = w::SliderSpec::new("入れ物", 0.0, 100.0, w::NumberFormat::int("%"));
        let out = w::slider(ui, row(10), "part.scoped", g.scoped_value, &spec);
        if out.changed {
            g.scoped_value = out.value;
        }
    });
    let free = free!(11);
    if g.show_late {
        w::button(
            ui,
            row(11),
            "part.late",
            "OK",
            false,
            g.base[0] && free,
            None,
            None,
        );
    }
    if now {
        // 描き始めたフレームのあとは、フレームの頭から描いている
        g.stroking = true;
        g.begins_at = None;
    }
}

fn parts(base: [bool; 3]) -> Harness<'static, Parts> {
    let state = Parts {
        base,
        value: 30.0,
        number: 5.0,
        scoped_value: 50.0,
        ..Default::default()
    };
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(340.0, 8.0 + 34.0 * PARTS_ROWS as f32 + 20.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(common::shared_gpu::renderer())
        .build_ui_state(draw_parts, state);
    h.run();
    h
}

fn parts_shot(h: &mut Harness<'_, Parts>) -> RgbaImage {
    // ポインタは部品にかけずに撮る（hover の見た目を混ぜない）
    h.event(Event::PointerGone);
    h.run();
    h.render().expect("描画")
}

fn parts_origin(h: &Harness<'_, Parts>) -> Pos2 {
    // 選びの箱（左の名前の幅 80 の右）から、見本の左上を逆算する
    h.get_by_label("選び: 通常").rect().min - vec2(80.0 + 8.0, 8.0 + 34.0 * 7.0)
}

fn parts_click(h: &mut Harness<'_, Parts>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run();
}

fn parts_drag(h: &mut Harness<'_, Parts>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    h.event(Event::PointerMoved(to));
    h.step();
    h.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run();
}

/// 描いている間は、押せる部品も押せない部品も描き始める前の見た目のまま。押せないことは変わらず、離せば元どおり押せる。
#[test]
fn parts_keep_their_look_and_stay_inert_while_stroking() {
    let mut h = parts([true, false, true]);
    let origin = parts_origin(&h);
    let idle = parts_shot(&mut h);
    // 描いていないときは押せる（対照）
    parts_click(&mut h, part_row(origin, 0).center());
    assert_eq!(h.state().clicks, [1, 0, 0]);
    let before = parts_shot(&mut h);
    assert_eq!(
        differences(&idle, &before, &[]),
        Vec::<String>::new(),
        "押した後に離れれば元の絵"
    );

    h.state_mut().stroking = true;
    h.step();
    let during = parts_shot(&mut h);
    assert_eq!(
        differences(&before, &during, &[]),
        Vec::<String>::new(),
        "描いている間は、押せる部品も押せない部品も描き始める前の見た目"
    );
    // 描いている間は押せない: 押しても何も起きず、押した見た目にもならない
    for i in 0..3 {
        parts_click(&mut h, part_row(origin, i).center());
    }
    assert_eq!(
        h.state().clicks,
        [1, 0, 0],
        "描いている間はボタンが押せない"
    );
    let r = part_row(origin, 4);
    parts_drag(
        &mut h,
        r.left_center() + vec2(10.0, 0.0),
        r.right_center() - vec2(10.0, 0.0),
    );
    assert_eq!(h.state().value, 30.0, "描いている間はスライダーが動かない");
    parts_click(&mut h, part_row(origin, 6).left_center() + vec2(8.0, 0.0));
    assert!(!h.state().on, "描いている間はチェックが替わらない");
    let r = part_row(origin, 8);
    parts_drag(&mut h, r.center(), r.center() + vec2(40.0, 0.0));
    assert_eq!(h.state().number, 5.0, "描いている間は数値の欄が動かない");
    let r = part_row(origin, 10);
    parts_drag(
        &mut h,
        r.left_center() + vec2(10.0, 0.0),
        r.right_center() - vec2(10.0, 0.0),
    );
    assert_eq!(
        h.state().scoped_value,
        50.0,
        "描いている間は入れ物の中が動かない"
    );
    let after_pokes = parts_shot(&mut h);
    assert_eq!(
        differences(&before, &after_pokes, &[]),
        Vec::<String>::new(),
        "押そうとしても見た目は変わらない"
    );

    // 描いている間に初めて出た部品は、そのときの本当の見た目（押せない）で出る
    h.state_mut().show_late = true;
    let with_late = parts_shot(&mut h);
    let late = part_row(origin, 11);
    let disabled_twin = part_row(origin, 1);
    for dy in 0..late.height() as u32 {
        for x in late.left() as u32..late.right() as u32 {
            assert_eq!(
                with_late.get_pixel(x, late.top() as u32 + dy),
                with_late.get_pixel(x, disabled_twin.top() as u32 + dy),
                "描いている間に初めて出たボタンは、押せない見た目 ({x}, {dy})"
            );
        }
    }
    h.state_mut().show_late = false;

    // 離すと元どおり押せる
    h.state_mut().stroking = false;
    h.run();
    let released = parts_shot(&mut h);
    assert_eq!(
        differences(&before, &released, &[]),
        Vec::<String>::new(),
        "離した後も同じ見た目"
    );
    parts_click(&mut h, part_row(origin, 0).center());
    assert_eq!(h.state().clicks, [2, 0, 0], "離せば押せる");
    parts_drag(
        &mut h,
        r.left_center() + vec2(10.0, 0.0),
        r.right_center() - vec2(10.0, 0.0),
    );
    assert_ne!(h.state().scoped_value, 50.0, "離せば入れ物の中も動く");
}

/// 描いている間に今の状態が変わっても、見た目は描き始める前のまま。離した次のフレームから、今の状態の見た目になる。
#[test]
fn parts_follow_the_state_again_after_the_release() {
    let mut h = parts([true, false, true]);
    let before = parts_shot(&mut h);
    // 押せない見た目の見本（部品 0 と同じ部品が、押せないときの絵）
    let mut disabled = parts([false, false, true]);
    let disabled_shot = parts_shot(&mut disabled);
    assert!(
        !differences(&before, &disabled_shot, &[]).is_empty(),
        "押せる・押せないで見た目が違う"
    );

    h.state_mut().stroking = true;
    h.step();
    // 描いている間に、部品 0 は押せないことになる
    h.state_mut().base[0] = false;
    let during = parts_shot(&mut h);
    assert_eq!(
        differences(&before, &during, &[]),
        Vec::<String>::new(),
        "描いている間は、描き始める前の見た目のまま"
    );
    h.state_mut().stroking = false;
    h.run();
    let after = parts_shot(&mut h);
    assert_eq!(
        differences(&disabled_shot, &after, &[]),
        Vec::<String>::new(),
        "離したら、今の状態（押せない）の見た目"
    );
}

/// 描き始めたフレームのうちに、あとから描く部品も、描き始める前の見た目を使う（途中から下の部品だけ灰色にならない）。次のフレーム以降も同じ。
#[test]
fn a_stroke_that_begins_mid_frame_does_not_gray_the_parts_drawn_after_it() {
    let mut h = parts([true, true, true]);
    let before = parts_shot(&mut h);
    // 1 フレームの途中（行 4）で描き始める: 上の部品は描いていないとき、下の部品は描き始めたあとに描く
    h.state_mut().begins_at = Some(4);
    h.step();
    let began = h.render().expect("描画");
    assert_eq!(
        differences(&before, &began, &[]),
        Vec::<String>::new(),
        "描き始めたフレーム"
    );
    assert!(h.state().stroking, "次のフレームは頭から描いている");
    let during = parts_shot(&mut h);
    assert_eq!(
        differences(&before, &during, &[]),
        Vec::<String>::new(),
        "描き始めたあとのフレーム"
    );
    // 描き終えて、すぐ次の線を、また途中から描き始めても同じ（途中のフレームの部品の見た目は、描き始める前として使わない）
    h.state_mut().stroking = false;
    h.step();
    h.state_mut().begins_at = Some(2);
    h.step();
    let again = h.render().expect("描画");
    assert_eq!(
        differences(&before, &again, &[]),
        Vec::<String>::new(),
        "続けて描き始めたフレーム"
    );
}
