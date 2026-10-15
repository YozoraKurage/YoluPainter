//! 初めの設定（既定）の束: プロパティの欄の初めのタブ・スポイトの対象・知らせの位置・保存せずに閉じるときの問い・ブラシサイズの段・最近開いたプロジェクト。
//! 試験は運転の口（`common::drive::Drive`）を通す。
use crate::common;

use common::drive::Drive;
use yolu_app::panels::properties::{tab_labels, TAB_LAYER, TAB_ORDER, TAB_STENCIL};
use yolu_app::state::{AppState, Tool};

#[test]
fn the_properties_panel_starts_on_the_layer_tab_which_is_the_left_one() {
    let mut h = AppState::new(32, 32);
    assert_eq!(h.ui().property_tab, TAB_LAYER);
    assert_eq!(TAB_ORDER[0], TAB_LAYER, "左のタブがレイヤー");
    assert_eq!(TAB_ORDER[1], TAB_STENCIL);
    h.set_lang(yolu_app::lang::Lang::Ja);
    assert_eq!(tab_labels(h.app()), ["レイヤー", "ステンシル"]);
    h.set_lang(yolu_app::lang::Lang::En);
    assert_eq!(tab_labels(h.app()), ["Layer", "Stencil"]);
}

#[test]
fn the_properties_tab_strip_shows_layer_on_the_left_and_shows_the_layer_body_first() {
    let mut h = common::app(1600.0, 900.0, 256);
    h.run();
    let props = h.state().tab_rects[&yolu_app::Tab::Properties];
    let in_props = |r: egui::Rect| r.left() > props.left() - 2.0 && r.top() > props.top();
    let layer = common::rect_of(&h, "レイヤー", in_props);
    let stencil = common::rect_of(&h, "ステンシル", in_props);
    assert!(layer.center().x < stencil.center().x);
    assert_eq!(h.ui().property_tab, TAB_LAYER);
    // 番号の意味（ステンシル = 0・レイヤー = 1）は並びを替えても変えない
    assert_eq!((TAB_STENCIL, TAB_LAYER), (0, 1));
    common::click(&mut h, stencil.center());
    assert_eq!(h.ui().property_tab, TAB_STENCIL);
}

#[test]
fn the_eyedropper_starts_on_all_layers_and_the_selected_layer_row_turns_it_off() {
    use yolu_app::subtool::{Key, SubToolAction};
    let mut h = AppState::new(32, 32);
    assert!(h.eyedrop().all_layers);
    let list = h.app().subtool_list(Tool::Eyedropper).unwrap();
    assert_eq!(list.current(), Key::Builtin("all"));
    assert!(!h
        .app()
        .subtool_is_modified(Tool::Eyedropper, list.current()));
    h.act(yolu_app::state::Action::SubTool(SubToolAction::Select(
        Tool::Eyedropper,
        Key::Builtin("layer"),
    )));
    assert!(!h.eyedrop().all_layers);
    h.act(yolu_app::state::Action::SubTool(SubToolAction::Select(
        Tool::Eyedropper,
        Key::Builtin("all"),
    )));
    assert!(h.eyedrop().all_layers);
}

// ───────── 知らせ（トースト）は状態の帯の左の端 ─────────

/// 描いた文字（`Shape::Text`）のうち、条件に合う文の矩形（画面の点）。
fn text_rects(
    h: &egui_kittest::Harness<'_, yolu_app::YoluApp>,
    pick: impl Fn(&str) -> bool,
) -> Vec<egui::Rect> {
    fn find(shape: &egui::Shape, pick: &dyn Fn(&str) -> bool, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| find(s, pick, out)),
            egui::Shape::Text(t) if pick(&t.galley.job.text) => {
                out.push(t.galley.rect.translate(t.pos.to_vec2()))
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in &h.output().shapes {
        find(&s.shape, &pick, &mut out);
    }
    out
}

fn with_usage(h: &mut egui_kittest::Harness<'static, yolu_app::YoluApp>) {
    h.usage_mut().build = Some("0.1.0 · a1b2c3d".into());
    h.usage_mut().process = Some(812 * 1024 * 1024);
    h.usage_mut().gpu = Some(1536 * 1024 * 1024);
    h.usage_mut().sampled_at = Some(1.0e9);
}

#[test]
fn the_toast_is_one_line_in_the_status_bar_from_its_left_edge_and_stops_before_the_right_items() {
    use yolu_app::notice::Source;
    let mut h = common::app(1000.0, 700.0, 128);
    h.set_lang(yolu_app::lang::Lang::Ja);
    with_usage(&mut h);
    h.event(egui::Event::PointerGone);
    let screen = h.ctx.content_rect();
    let bar_top = screen.bottom() - yolu_app::ui::theme::STATUS_BAR_HEIGHT;

    // 短い文: 帯の左の端に、そのまま 1 行
    h.app_mut()
        .info(Source::Project, "新しいプロジェクトを作りました。");
    h.run();
    let short = text_rects(&h, |s| s == "新しいプロジェクトを作りました。");
    assert_eq!(short.len(), 1);
    assert!(
        short[0].left() < 40.0 && short[0].top() >= bar_top && short[0].bottom() <= screen.bottom(),
        "{:?}",
        short[0]
    );

    // 右の項目の左端（これより左が知らせの置き場）
    let painter = h.ctx.layer_painter(egui::LayerId::background());
    let bar = egui::Rect::from_min_max(egui::pos2(screen.left(), bar_top), screen.max);
    let layout = yolu_app::shell::status_layout(&painter, h.app(), bar);
    assert!(!layout.items.is_empty() && layout.free_right < screen.right() - 40.0);

    // 長い文: 右の項目に重ならない幅で切り、「…」を付ける。全文はホバーのツールチップ
    let full = "とても長い知らせの文です。".repeat(40);
    h.app_mut().info(Source::Project, full.as_str());
    h.run();
    let cut = text_rects(&h, |s| s.starts_with("とても長い") && s.ends_with('…'));
    assert_eq!(cut.len(), 1, "切った 1 行");
    assert!(
        cut[0].left() < 40.0 && cut[0].right() <= layout.free_right + 0.5,
        "{:?} / {}",
        cut[0],
        layout.free_right
    );
    assert!(cut[0].top() >= bar_top, "1 行（帯の中）");
    assert!(
        text_rects(&h, |s| s == full).is_empty(),
        "全文は、ホバーするまで出ない"
    );
    h.event(egui::Event::PointerMoved(cut[0].center()));
    for _ in 0..60 {
        h.step();
    }
    assert_eq!(
        text_rects(&h, |s| s == full).len(),
        1,
        "ホバーで全文のツールチップ"
    );
}

#[test]
fn a_toast_that_fits_has_no_tooltip_and_the_text_readers_see_the_full_sentence() {
    use yolu_app::notice::Source;
    let mut h = common::app(1000.0, 700.0, 128);
    h.event(egui::Event::PointerGone);
    h.app_mut()
        .info(Source::Project, "新しいプロジェクトを作りました。");
    h.run();
    assert_eq!(
        h.message(),
        "新しいプロジェクトを作りました。",
        "試験が読む口は今のまま"
    );
    assert_eq!(
        yolu_app::shell::status_text(h.app()),
        "新しいプロジェクトを作りました。"
    );
    let at = text_rects(&h, |s| s == "新しいプロジェクトを作りました。")[0].center();
    h.event(egui::Event::PointerMoved(at));
    h.run();
    assert_eq!(
        text_rects(&h, |s| s == "新しいプロジェクトを作りました。").len(),
        1,
        "ツールチップは重ねない"
    );
}
