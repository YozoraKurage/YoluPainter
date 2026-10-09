//! ブラシの縁のアンチエイリアスの欄: アプリの既定と組み込みの硬いブラシは 中、形状のカテゴリの選びで設定が替わり、キャンバスに描く線（実際の
//! 入力の道）の縁が変わる。パスのブラシの欄の選び。日英の絵。
#![allow(clippy::chunks_exact_to_as_chunks)]
use crate::common;

use common::*;
use egui::{Event, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::brushes::{builtin, BrushAction, BrushKey, Category};
use yolu_app::engine::{AntiAlias, Channel};
use yolu_app::lang::Lang;
use yolu_app::m2::UiOp;
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

fn st(h: &H) -> &AppState {
    &h.state().state
}

fn open_detail(h: &mut H, category: Category) {
    let ui = &mut h.state_mut().state.brushes.ui;
    ui.detail.open = true;
    ui.detail.category = category;
    ui.detail.scroll = 0.0;
    h.run();
}

fn close_detail(h: &mut H) {
    h.state_mut().state.brushes.ui.detail.open = false;
    h.run();
}

fn detail_rect(h: &H) -> Rect {
    yolu_app::ui::window::last_rect(&h.ctx, yolu_app::panels::brush_detail::id())
        .expect("ブラシの詳細のウィンドウを描いている")
}

fn shot(h: &mut H, rect: Rect, name: &str) {
    h.event(Event::PointerGone);
    h.step();
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        rect.left().floor() as u32,
        rect.top().floor() as u32,
        rect.width().ceil() as u32,
        rect.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

/// 選んでいるレイヤーの、縁の帯の画素（0 と 255 の間のアルファ）の数と、塗った画素の数。
fn edge_pixels(h: &H) -> (usize, usize) {
    let s = st(h);
    let layer = s.selected_layer.unwrap();
    let bytes = s
        .doc
        .layer(layer)
        .unwrap()
        .surface(Channel::Color)
        .map_or_else(Vec::new, |surface| surface.to_canvas_bytes());
    let alpha = bytes.chunks_exact(4).map(|p| p[3]);
    let partial = alpha.clone().filter(|&a| a > 0 && a < 255).count();
    (partial, alpha.filter(|&a| a > 0).count())
}

#[test]
fn the_app_default_and_hard_builtin_brushes_are_medium_and_soft_ones_are_none() {
    let h = app(1280.0, 800.0, 128);
    assert_eq!(st(&h).brush.anti_alias, AntiAlias::Medium);
    for id in [
        "standard",
        "hard-round",
        "ink-pen",
        "pencil",
        "gouache",
        "standard-eraser",
        "hard-eraser",
    ] {
        let b = builtin::find(id).unwrap();
        assert_eq!(b.brush.base.anti_alias, AntiAlias::Medium, "{id}");
    }
    for id in [
        "soft-round",
        "airbrush",
        "soft-eraser",
        "marker",
        "chalk",
        "mixer",
        "blur",
        "splatter",
    ] {
        let b = builtin::find(id).unwrap();
        assert_eq!(b.brush.base.anti_alias, AntiAlias::None, "{id}");
    }
    // 起動直後の設定は「標準」と同じ（変更の印が付かない）
    assert!(!st(&h).brush_is_modified(BrushKey::Builtin("standard")));
}

#[test]
fn choosing_a_level_changes_the_edge_of_a_stroke_drawn_on_the_canvas() {
    let mut h = app(1600.0, 1000.0, 256);
    {
        let b = &mut h.state_mut().state.brush;
        b.radius = 7.0;
        b.hardness = 1.0;
        b.pressure_size = false;
        b.pressure_opacity = false;
    }
    h.run();
    let mut results = Vec::new();
    for (now, pick, level) in [
        ("中", "なし", AntiAlias::None),
        ("なし", "強", AntiAlias::Strong),
        ("強", "弱", AntiAlias::Weak),
    ] {
        // 形状のカテゴリの「アンチエイリアス」の箱から選ぶ
        open_detail(&mut h, Category::Shape);
        h.get_by_label(&format!("アンチエイリアス: {now}")).click();
        h.run();
        let at = popup_item(&h, pick).center();
        click(&mut h, at);
        assert_eq!(st(&h).brush.anti_alias, level);
        close_detail(&mut h);
        // キャンバスに斜めの線を描く（ペンの入力の道）
        let c = canvas_rect(&h).center();
        drag(
            &mut h,
            &[
                offset(c, -60.0, -25.0),
                offset(c, -10.0, 3.0),
                offset(c, 55.0, 30.0),
            ],
        );
        results.push(edge_pixels(&h));
        h.state_mut().state.apply(Action::Undo);
        h.run();
        assert_eq!(edge_pixels(&h).1, 0, "取り消しで消える");
    }
    // なし: 硬い丸の縁は 2 値。弱・強: 縁に帯が付き、強いほど広い
    assert!(results[0].1 > 0);
    assert_eq!(results[0].0, 0, "{results:?}");
    assert!(
        results[2].0 > 0 && results[1].0 > results[2].0,
        "{results:?}"
    );
    // 選んだ段はブラシの変更（元に戻すで組み込みの値へ）
    assert!(st(&h).brush_is_modified(BrushKey::Builtin("standard")));
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Revert(BrushKey::Builtin(
            "standard",
        ))));
    h.run();
    assert_eq!(st(&h).brush.anti_alias, AntiAlias::Medium);
}

#[test]
fn the_shape_reset_restores_medium() {
    let mut h = app(1600.0, 1000.0, 128);
    h.state_mut().state.brush.anti_alias = AntiAlias::Strong;
    open_detail(&mut h, Category::Shape);
    h.get_by_label("形状を既定に戻す").click();
    h.run();
    assert_eq!(st(&h).brush.anti_alias, AntiAlias::Medium);
}

#[test]
fn the_path_brush_has_its_own_anti_alias_choice() {
    let mut h = app(1600.0, 1000.0, 128);
    h.state_mut().state.apply(Action::SelectTool(Tool::Path));
    h.run();
    // パスが無いときは、次に作るパスが取る今のブラシの値を替える
    h.get_by_label("アンチエイリアス: 中").click();
    h.run();
    let at = popup_item(&h, "強").center();
    click(&mut h, at);
    assert_eq!(st(&h).brush.anti_alias, AntiAlias::Strong);
}

/// 日本語の欄は `brushes_detail_shape`（brushes.rs）が撮る。
#[test]
fn snapshot_brush_detail_shape_english() {
    let mut h = app(1600.0, 1000.0, 128);
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    open_detail(&mut h, Category::Shape);
    // 英語の画面に日本語が無い
    h.get_by_label("Anti-aliasing: Medium");
    let rect = detail_rect(&h);
    shot(&mut h, rect, "brushes_detail_shape_english");
}
