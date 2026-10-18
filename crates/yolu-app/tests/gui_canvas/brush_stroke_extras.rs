//! ブラシの詳細の、インクの色混ぜの拡張（混色モード・輝度の補正・にじむ強さと連動・描き始めの絵の具の量）・ストロークの連続吹き付けとリボン・
//! 水彩境界・はみ出し防止の行（値の変更・使えない形・区分の既定に戻す）と、連続吹き付けのブラシでペン・マウスを止めて押している間に
//! フレームごとに点が足されること。
use crate::common;
use crate::common::drive::{Drive, DriveApp};

use common::*;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::brushes::Category;
use yolu_app::engine::{composite_pixel, MixBlur, MixMode, MixSpace};
use yolu_app::lang::Lang;
use yolu_app::m2::UiOp;
use yolu_app::pen::PenSample;
use yolu_app::state::Action;
use yolu_app::YoluApp;
use yolu_core::{builtin_tip, ExpandMode, ReferenceLines, Spray, WaterEdge};

type H = Harness<'static, YoluApp>;

fn open_detail(h: &mut H, category: Category) {
    let ui = &mut h.brushes_mut().ui;
    ui.detail.open = true;
    ui.detail.category = category;
    ui.detail.scroll = 0.0;
    h.run();
}

fn detail_rect(h: &H) -> Rect {
    yolu_app::ui::window::last_rect(&h.ctx, yolu_app::panels::brush_detail::id())
        .expect("ブラシの詳細のウィンドウを描いている")
}

fn in_pane_node<'a>(h: &'a H, label: &'a str) -> egui_kittest::Node<'a> {
    let window = detail_rect(h);
    h.query_all_by_label(label)
        .find(|n| window.contains(n.rect().center()) && n.rect().left() > window.left() + 168.0)
        .unwrap_or_else(|| panic!("{label}"))
}

fn in_pane(h: &H, label: &str) -> Rect {
    in_pane_node(h, label).rect()
}

fn disabled(h: &H, label: &str) -> bool {
    in_pane_node(h, label).accesskit_node().is_disabled()
}

fn press(h: &mut H, label: &str) {
    let at = in_pane(h, label).center();
    click(h, at);
}

fn reset(h: &mut H, category: Category) {
    let name = category.name(Lang::Ja);
    h.get_by_label(&format!("{name}を既定に戻す")).click();
    h.run();
}

#[test]
fn the_ink_page_has_the_mixing_extension_rows() {
    let mut h = app(1600.0, 1000.0, 128);
    open_detail(&mut h, Category::Ink);
    // 混ぜ方がなしの間は、拡張の行は出ない
    let window = detail_rect(&h);
    for label in [
        "混色モード: 標準",
        "輝度の補正: なし",
        "にじむ強さ",
        "にじむ強さの連動",
        "初めの絵の具の量",
    ] {
        assert!(
            !h.query_all_by_label(label)
                .any(|n| window.contains(n.rect().center())),
            "{label}"
        );
    }
    h.m2_mut().brush.mix.mode = MixMode::Smear;
    h.run();
    // 輝度の補正は知覚的のときだけ、固定のにじむ強さは連動を切ったときだけ
    assert!(!disabled(&h, "混色モード: 標準") && disabled(&h, "輝度の補正: なし"));
    assert!(disabled(&h, "にじむ強さ") && !disabled(&h, "にじむ強さの連動"));
    press(&mut h, "混色モード: 標準");
    let at = popup_item(&h, "知覚的").center();
    click(&mut h, at);
    assert_eq!(h.m2().brush.mix.space, MixSpace::Perceptual);
    press(&mut h, "輝度の補正: なし");
    let at = popup_item(&h, "高").center();
    click(&mut h, at);
    assert_eq!(h.m2().brush.mix.correction, 3);
    press(&mut h, "にじむ強さの連動");
    assert_eq!(h.m2().brush.mix.blur, MixBlur::Fixed(2.0));
    assert!(!disabled(&h, "にじむ強さ"));
    press(&mut h, "にじむ強さの連動");
    assert_eq!(h.m2().brush.mix.blur, MixBlur::Linked);
    // 伸ばす以外では、にじむ強さの 2 行は触れない
    h.m2_mut().brush.mix.mode = MixMode::Mix;
    h.run();
    assert!(disabled(&h, "にじむ強さの連動") && !disabled(&h, "初めの絵の具の量"));
    reset(&mut h, Category::Ink);
    assert_eq!(h.m2().brush.mix, Default::default());
    assert!(!h.doc().can_undo());
}

#[test]
fn the_stroke_page_has_continuous_spraying_and_ribbon() {
    let mut h = app(1600.0, 1000.0, 128);
    open_detail(&mut h, Category::Stroke);
    // 連続吹き付け: 入れる・切る。後補正が入っている間は触れない
    press(&mut h, "連続吹き付け");
    assert!(h.m2().brush.tip.continuous);
    h.m2_mut().brush.assist.post.on = true;
    h.run();
    assert!(disabled(&h, "連続吹き付け"));
    h.m2_mut().brush.assist.post.on = false;
    h.run();
    press(&mut h, "連続吹き付け");
    assert!(!h.m2().brush.tip.continuous);
    // リボン: 円形の筆先・散布効果の間は触れない
    h.m2_mut().brush.tip.image = None;
    h.m2_mut().brush.tip.images.clear();
    h.run();
    assert!(disabled(&h, "リボン"));
    h.m2_mut().brush.tip.image = builtin_tip("charcoal");
    h.run();
    press(&mut h, "リボン");
    assert!(h.m2().brush.tip.ribbon);
    h.m2_mut().brush.spray = Some(Spray::default());
    h.run();
    assert!(disabled(&h, "リボン"));
    h.m2_mut().brush.spray = None;
    h.m2_mut().brush.tip.continuous = true;
    h.run();
    // 筆先の区分を戻しても、ストロークの区分の連続吹き付け・リボンは残る。ストロークの区分を戻すと切
    open_detail(&mut h, Category::Tip);
    reset(&mut h, Category::Tip);
    assert!(h.m2().brush.tip.continuous && h.m2().brush.tip.ribbon);
    open_detail(&mut h, Category::Stroke);
    reset(&mut h, Category::Stroke);
    assert!(!h.m2().brush.tip.continuous && !h.m2().brush.tip.ribbon);
}

#[test]
fn the_watercolor_edge_page_turns_the_edge_on_and_edits_it() {
    let mut h = app(1600.0, 1000.0, 128);
    open_detail(&mut h, Category::WaterEdge);
    for label in [
        "水彩境界の幅",
        "透明度影響",
        "明度影響",
        "ドラッグ後に処理",
        "ぼかし幅",
    ] {
        assert!(disabled(&h, label), "{label}");
    }
    press(&mut h, "水彩境界");
    assert_eq!(h.m2().brush.water_edge, Some(WaterEdge::default()));
    for label in ["水彩境界の幅", "透明度影響", "明度影響", "ドラッグ後に処理"]
    {
        assert!(!disabled(&h, label), "{label}");
    }
    // ぼかし幅はドラッグ後に処理のときだけ
    assert!(disabled(&h, "ぼかし幅"));
    press(&mut h, "ドラッグ後に処理");
    assert!(h.m2().brush.water_edge.unwrap().after_stroke);
    assert!(!disabled(&h, "ぼかし幅"));
    // 幅のスライダーの右の端を押すと、上限
    let width = in_pane(&h, "水彩境界の幅");
    click(&mut h, Pos2::new(width.right() - 2.0, width.center().y));
    assert!(h.m2().brush.water_edge.unwrap().width > WaterEdge::default().width);
    // 消しゴムでは効かない
    h.set_tool(yolu_app::state::Tool::Eraser);
    h.run();
    assert!(disabled(&h, "水彩境界"));
    h.set_tool(yolu_app::state::Tool::Brush);
    h.run();
    reset(&mut h, Category::WaterEdge);
    assert_eq!(h.m2().brush.water_edge, None);
}

#[test]
fn the_reference_lines_page_turns_the_guard_on_and_edits_it() {
    let mut h = app(1600.0, 1000.0, 128);
    open_detail(&mut h, Category::ReferenceLines);
    for label in [
        "色の誤差",
        "領域拡縮",
        "領域拡縮の量",
        "領域拡縮の方法: 最も濃いピクセルまで",
    ] {
        assert!(disabled(&h, label), "{label}");
    }
    press(&mut h, "参照レイヤーの線をはみ出さない");
    assert!(h.m2().brush.reference_lines.enabled);
    assert!(!disabled(&h, "色の誤差") && !disabled(&h, "領域拡縮"));
    assert!(disabled(&h, "領域拡縮の量"));
    press(&mut h, "領域拡縮");
    assert!(h.m2().brush.reference_lines.expand);
    press(&mut h, "領域拡縮の方法: 最も濃いピクセルまで");
    let at = popup_item(&h, "丸く").center();
    click(&mut h, at);
    assert_eq!(h.m2().brush.reference_lines.mode, ExpandMode::Round);
    reset(&mut h, Category::ReferenceLines);
    assert_eq!(h.m2().brush.reference_lines, ReferenceLines::default());
}

fn point(h: &H, x: f64, y: f64) -> Pos2 {
    let s = h.app();
    s.view()
        .view(
            s.ui().canvas_rect.unwrap(),
            s.doc().width(),
            s.doc().height(),
        )
        .to_screen(x, y)
}

fn pen(h: &mut H, p: Pos2, contact: bool, time_ms: u32) {
    h.push_pen(PenSample {
        pos: [p.x, p.y],
        pressure: if contact { 0.8 } else { 0.0 },
        tilt: Default::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 5,
        time_ms,
    });
    // ペンと同時に来る代わりのマウスの入力（二重に描かない）
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: contact,
        modifiers: Modifiers::NONE,
    });
    h.step();
}

fn mouse(h: &mut H, p: Pos2, down: bool) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: down,
        modifiers: Modifiers::NONE,
    });
    h.step();
}

fn continuous_app(on: bool) -> H {
    let mut h = app(1280.0, 800.0, 128);
    let s = h.app_mut();
    s.brush_mut().radius = 6.0;
    s.brush_mut().hardness = 0.5;
    s.brush_mut().flow = 0.1;
    s.brush_mut().pressure.size = false;
    s.m2_mut().brush.tip.continuous = on;
    h
}

/// 押して `frames` フレームのあいだ動かさずに押し、離す（ペンか、マウス）。離した後の、押した点のアルファと、受けた点の数。
fn hold(on: bool, use_pen: bool, frames: usize) -> (u8, usize) {
    let mut h = continuous_app(on);
    let at = point(&h, 64.0, 64.0);
    if use_pen {
        pen(&mut h, at, true, 1000);
    } else {
        mouse(&mut h, at, true);
    }
    for _ in 0..frames {
        h.step();
    }
    let points = h.canvas().stroke_points;
    if use_pen {
        pen(&mut h, at, false, 1000 + 17 * frames as u32);
    } else {
        mouse(&mut h, at, false);
    }
    h.run();
    assert!(!h.app().is_stroking());
    (composite_pixel(h.doc(), 64, 64)[3], points)
}

#[test]
fn holding_a_pen_still_adds_a_point_each_frame_and_darkens_the_spot() {
    let (dark, points) = hold(true, true, 30);
    let (plain, plain_points) = hold(false, true, 30);
    // 動かさない間も、フレームごとに同じ点を 1 つ渡す（切のブラシは渡さない）
    assert_eq!(plain_points, 1);
    assert!(points >= 25, "{points}");
    assert!(dark > plain, "{dark} > {plain}");
}

#[test]
fn escape_focus_loss_and_release_stop_the_held_points_and_the_frame_requests() {
    for how in ["escape", "focus lost", "release"] {
        let mut h = continuous_app(true);
        let at = point(&h, 64.0, 64.0);
        pen(&mut h, at, true, 1000);
        for _ in 0..10 {
            h.step();
        }
        assert!(
            h.canvas().stroke_points >= 8,
            "{how}: テストの前提: 点が足されている"
        );
        match how {
            "escape" => key(&h, egui::Key::Escape, Modifiers::NONE),
            "focus lost" => h.event(Event::WindowFocused(false)),
            _ => pen(&mut h, at, false, 1200),
        }
        // 次のフレームを頼み続けていれば、ここで終わらない（`run` は頼みが止まるまで進める）
        h.run();
        assert!(!h.app().is_stroking(), "{how}");
        assert!(h.canvas().held.is_none(), "{how}: 最後の点を手放す");
        let points = h.canvas().stroke_points;
        let doc = h.doc().undo_count();
        for _ in 0..20 {
            h.step();
        }
        assert_eq!(h.canvas().stroke_points, points, "{how}: 点が増えない");
        assert_eq!(h.doc().undo_count(), doc, "{how}");
    }
}

#[test]
fn holding_the_mouse_still_also_sprays() {
    let (dark, points) = hold(true, false, 20);
    let (plain, _) = hold(false, false, 20);
    assert!(points >= 15, "{points}");
    assert!(dark > plain, "{dark} > {plain}");
}

#[test]
fn snapshot_brush_continuous_2d() {
    // 左から右への線の中ほどで、ペンを 1 秒（60 フレーム）止める
    let mut h = continuous_app(true);
    let mut t = 1000u32;
    for i in 0..=20 {
        let p = point(&h, 16.0 + i as f64 * 2.0, 64.0);
        pen(&mut h, p, true, t);
        t += 17;
    }
    for _ in 0..60 {
        h.step();
        t += 17;
    }
    for i in 21..=48 {
        let p = point(&h, 16.0 + i as f64 * 2.0, 64.0);
        pen(&mut h, p, true, t);
        t += 17;
    }
    let p = point(&h, 112.0, 64.0);
    pen(&mut h, p, false, t);
    h.run();
    let rect = h.ui().canvas_rect.unwrap();
    crate::common::shot::clean_region(&mut h, "brush_continuous_2d", rect);
}

/// 斜めの縞の筆先（96 × 24。線に沿って続けて写すと、縞が切れずにつながる）。
fn stripes() -> std::sync::Arc<yolu_core::BrushTip> {
    let (w, h) = (96u32, 24u32);
    let alpha = (0..h)
        .flat_map(|y| (0..w).map(move |x| if ((x + y) / 6) % 2 == 0 { 255 } else { 50 }))
        .collect();
    std::sync::Arc::new(yolu_core::BrushTip::new("stripes", w, h, alpha).unwrap())
}

#[test]
fn snapshot_brush_ribbon_2d() {
    let mut h = app(1280.0, 800.0, 128);
    let s = h.app_mut();
    s.brush_mut().radius = 10.0;
    s.brush_mut().spacing = 0.1;
    s.brush_mut().pressure.size = false;
    let b = &mut s.m2_mut().brush;
    b.tip.images = vec![stripes(), builtin_tip("charcoal").unwrap()];
    b.tip.selection = yolu_core::TipSelection::Sequential;
    b.tip.ribbon = true;
    let at: Vec<Pos2> = (0..=40)
        .map(|i| {
            let t = i as f64 / 40.0;
            point(&h, 14.0 + 100.0 * t, 64.0 + 30.0 * (t * 6.0).sin())
        })
        .collect();
    mouse(&mut h, at[0], true);
    for p in &at[1..] {
        h.input_mut().events.push(Event::PointerMoved(*p));
        h.step();
    }
    mouse(&mut h, *at.last().unwrap(), false);
    h.run();
    let rect = h.ui().canvas_rect.unwrap();
    crate::common::shot::clean_region(&mut h, "brush_ribbon_2d", rect);
}

#[test]
fn snapshot_brush_detail_new_pages() {
    for (lang, suffix) in [(Lang::Ja, ""), (Lang::En, "_english")] {
        let mut h = app(1600.0, 1000.0, 128);
        h.act(Action::M2Ui(UiOp::Language(lang)));
        h.run();
        let b = &mut h.m2_mut().brush;
        b.mix.mode = MixMode::Smear;
        b.mix.space = MixSpace::Perceptual;
        b.mix.correction = 2;
        b.tip.image = builtin_tip("charcoal");
        b.tip.ribbon = true;
        b.tip.continuous = true;
        b.water_edge = Some(WaterEdge {
            after_stroke: true,
            ..WaterEdge::default()
        });
        b.reference_lines = ReferenceLines {
            enabled: true,
            expand: true,
            ..ReferenceLines::default()
        };
        for (category, name) in [
            (Category::Ink, "brushes_detail_ink_extension"),
            (Category::Stroke, "brushes_detail_stroke_extension"),
            (Category::WaterEdge, "brushes_detail_watercolor_edge"),
            (Category::ReferenceLines, "brushes_detail_reference_lines"),
        ] {
            open_detail(&mut h, category);
            let rect = detail_rect(&h);
            crate::common::shot::clean_region(&mut h, &format!("{name}{suffix}"), rect);
        }
    }
}
