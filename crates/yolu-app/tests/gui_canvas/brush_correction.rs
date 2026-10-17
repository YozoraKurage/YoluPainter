//! ブラシの詳細の「補正」の区分: 手ぶれ補正の速度による調整・補正タイプ・はらい・後補正（強さ・速度による調整・表示倍率による調整）の行
//! （目・値の変更）と、後補正のブラシでキャンバスに描くと、離した後に線が整い、Ctrl+Z 1 回で描く前に戻ること。
use crate::common;
use crate::common::drive::Drive;

use common::*;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::brushes::{rows, Category, Row};
use yolu_app::engine::{composite_pixel, PostCorrection, StabilizerSpeed};
use yolu_app::lang::Lang;
use yolu_app::m2::UiOp;
use yolu_app::state::Action;
use yolu_app::YoluApp;

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

fn in_panel(r: Rect) -> bool {
    r.left() < 390.0 && r.top() > 200.0 && r.top() < 700.0
}

fn tool_has(h: &H, label: &str) -> bool {
    h.query_all_by_label(label).any(|n| in_panel(n.rect()))
}

/// 詳細の欄の行の左端の目のボタン。
fn eye_of(h: &H, row_label: &str) -> Rect {
    let window = detail_rect(h);
    let row = in_pane(h, row_label);
    h.query_all_by_label("ツールプロパティに表示")
        .map(|n| n.rect())
        .find(|r| window.contains(r.center()) && (r.center().y - row.center().y).abs() < 8.0)
        .unwrap_or_else(|| panic!("{row_label} の目のボタン"))
}

#[test]
fn the_stabilization_page_has_the_correction_rows_and_they_change_the_brush() {
    let mut h = app(1600.0, 1000.0, 128);
    open_detail(&mut h, Category::Stabilization);
    for label in [
        "手ぶれ補正",
        "速度による手ぶれ補正",
        "補正タイプ: ゆっくり描いたときに補正",
        "はらい",
        "後補正",
        "強さ",
        "速度による調整",
        "表示倍率による調整",
        "曲線",
    ] {
        in_pane(&h, label);
    }
    // 速度による手ぶれ補正を入れるまで、補正タイプは触れない。入れると「ゆっくり描いたときに補正」から
    assert!(disabled(&h, "補正タイプ: ゆっくり描いたときに補正"));
    let at = in_pane(&h, "速度による手ぶれ補正").center();
    click(&mut h, at);
    assert_eq!(
        h.m2().brush.assist.stabilizer_speed,
        Some(StabilizerSpeed::SlowOnly)
    );
    assert!(!disabled(&h, "補正タイプ: ゆっくり描いたときに補正"));
    let at = in_pane(&h, "補正タイプ: ゆっくり描いたときに補正").center();
    click(&mut h, at);
    let at = popup_item(&h, "すばやく描いたときに弱く補正").center();
    click(&mut h, at);
    assert_eq!(
        h.m2().brush.assist.stabilizer_speed,
        Some(StabilizerSpeed::WeakenWhenFast)
    );
    // 切ると速度による調整は外れ、補正タイプはまた触れない
    let at = in_pane(&h, "速度による手ぶれ補正").center();
    click(&mut h, at);
    assert_eq!(h.m2().brush.assist.stabilizer_speed, None);
    // 後補正: 入れるまで強さ・速度・表示倍率は触れない。入れたときは強さ 10・両方の調整が入り
    for label in ["強さ", "速度による調整", "表示倍率による調整"] {
        assert!(disabled(&h, label), "{label}");
    }
    let at = in_pane(&h, "後補正").center();
    click(&mut h, at);
    assert_eq!(
        h.m2().brush.assist.post,
        PostCorrection {
            on: true,
            ..PostCorrection::default()
        }
    );
    for label in ["強さ", "速度による調整", "表示倍率による調整"] {
        assert!(!disabled(&h, label), "{label}");
    }
    let at = in_pane(&h, "表示倍率による調整").center();
    click(&mut h, at);
    assert!(!h.m2().brush.assist.post.by_view_scale);
    // 目: 後補正の行をツールプロパティに出す
    assert!(!tool_has(&h, "後補正"));
    let eye = eye_of(&h, "後補正");
    click(&mut h, eye.center());
    assert!(rows::is_shown(&h.m2().brush, Row::PostCorrection));
    assert!(tool_has(&h, "後補正"));
    // 区分の「既定に戻す」は、補正の値を全部戻す（目の印は残す）
    h.m2_mut().brush.assist.sweep = 0.5;
    h.get_by_label("補正を既定に戻す").click();
    h.run();
    let a = h.m2().brush.assist;
    assert!(a.stabilizer_speed.is_none() && a.sweep == 0.0);
    assert_eq!(a.post, PostCorrection::default());
    assert!(rows::is_shown(&h.m2().brush, Row::PostCorrection));
    // どの操作も Undo の段を作らない
    assert!(!h.doc().can_undo());
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

/// 文書の点を、マウスの左ボタンで押したまま順になぞって離す。
fn mouse_stroke(h: &mut H, points: &[(f64, f64)]) {
    let at: Vec<Pos2> = points.iter().map(|(x, y)| point(h, *x, *y)).collect();
    let button = |h: &mut H, p: Pos2, down: bool| {
        h.input_mut().events.push(Event::PointerMoved(p));
        h.input_mut().events.push(Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed: down,
            modifiers: Modifiers::NONE,
        });
        h.step();
    };
    button(h, at[0], true);
    for p in &at[1..] {
        h.input_mut().events.push(Event::PointerMoved(*p));
        h.step();
    }
    button(h, *at.last().unwrap(), false);
    h.run();
}

/// 列ごとのアルファの重心の、となりの列との差の平均（まっすぐなら 0 に近い）。
fn wobble(h: &H, from: u32, to: u32) -> f64 {
    let doc = h.doc();
    let centre = |x: u32| {
        let (mut s, mut n) = (0.0, 0.0);
        for y in 0..doc.height() {
            let a = f64::from(composite_pixel(doc, x, y)[3]);
            s += a * f64::from(y);
            n += a;
        }
        s / n
    };
    let c: Vec<f64> = (from..to).map(centre).collect();
    c.windows(2).map(|p| (p[1] - p[0]).abs()).sum::<f64>() / (c.len() - 1) as f64
}

fn post_app(post: bool) -> H {
    let mut h = app(1280.0, 800.0, 128);
    let s = h.app_mut();
    s.brush_mut().radius = 2.0;
    s.brush_mut().hardness = 1.0;
    s.brush_mut().pressure.size = false;
    s.m2_mut().brush.assist.post = PostCorrection {
        on: post,
        strength: 10.0,
        by_speed: false,
        by_view_scale: false,
        ..PostCorrection::default()
    };
    h
}

#[test]
fn a_post_corrected_line_is_straightened_on_release_and_one_undo_removes_it() {
    // 揺れ（上下 0.6 画素）のある横の線
    let points: Vec<(f64, f64)> = (0..=80)
        .map(|i| {
            let y = 64.0 + if (i / 2) % 2 == 0 { 0.6 } else { -0.6 };
            (20.0 + i as f64, y)
        })
        .collect();
    let mut plain = post_app(false);
    mouse_stroke(&mut plain, &points);
    let mut h = post_app(true);
    let steps = h.doc().undo_count();
    mouse_stroke(&mut h, &points);
    assert!(!h.app().is_stroking());
    assert_eq!(h.doc().undo_count(), steps + 1, "描き直しは 1 回の Undo");
    let (fixed, raw) = (wobble(&h, 30, 90), wobble(&plain, 30, 90));
    assert!(fixed < raw / 2.0, "{fixed} < {raw}");
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    let doc = h.doc();
    assert!(
        (0..doc.width()).all(|x| (0..doc.height()).all(|y| composite_pixel(doc, x, y)[3] == 0)),
        "Ctrl+Z 1 回で描く前"
    );
}

#[test]
fn snapshot_brush_detail_stabilization() {
    for (lang, name) in [
        (Lang::Ja, "brushes_detail_stabilization"),
        (Lang::En, "brushes_detail_stabilization_english"),
    ] {
        let mut h = app(1600.0, 1000.0, 128);
        h.act(Action::M2Ui(UiOp::Language(lang)));
        h.run();
        let a = &mut h.m2_mut().brush.assist;
        a.stabilizer = 12.0;
        a.stabilizer_speed = Some(StabilizerSpeed::SlowOnly);
        a.sweep = 0.4;
        a.post = PostCorrection {
            on: true,
            ..PostCorrection::default()
        };
        open_detail(&mut h, Category::Stabilization);
        let rect = detail_rect(&h);
        crate::common::shot::clean_region(&mut h, name, rect);
    }
}
