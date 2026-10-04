//! 効果の行（層の行の下）と、選んだ効果の欄・メニューの画面（egui_kittest）。行を押す・目・上へ・下へ・消す・右クリック・メニュー、
//! 日英の見た目（説明の文を置かない）、スナップショット。文書の操作そのものは `tests/effects.rs`（画面なし）。
mod common;

use common::*;
use egui::{epaint::Shape, pos2, PointerButton, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::SnapshotResults;
use egui_kittest::Harness;
use yolu_app::fx::{FilterKind, FxOp, Selected};
use yolu_app::lang::Lang;
use yolu_app::m2::{Edit, UiOp};
use yolu_app::state::{Action, PopupKind};
use yolu_app::YoluApp;
use yolu_core::generator::Kind;
use yolu_core::{AnchorPlacement, FilterTarget};

fn apply(h: &mut Harness<'_, YoluApp>, action: Action) {
    h.state_mut().state.apply(action);
    h.run();
}

fn fx(h: &mut Harness<'_, YoluApp>, op: FxOp) {
    apply(h, Action::Fx(op));
}

/// 効果の行を並べた文書: 下地の層（ぼかし・階調の反転・アンカー）と、マスクにマップを読む Generator。
fn effect_document(h: &mut Harness<'_, YoluApp>) {
    apply(h, Action::M2(Edit::NewFill));
    let layer = h.state().state.selected_layer.unwrap();
    fx(h, FxOp::AddAnchor { layer, placement: AnchorPlacement::Layer });
    fx(h, FxOp::AddFilter { target: FilterTarget::Content, kind: FilterKind::Blur });
    fx(h, FxOp::AddFilter { target: FilterTarget::Content, kind: FilterKind::Invert });
    apply(h, Action::M2(Edit::AddMask(layer)));
    fx(h, FxOp::AddGenerator { target: FilterTarget::Mask, kind: Kind::EdgeWear });
}

fn selected(h: &Harness<'_, YoluApp>) -> Option<Selected> {
    h.state().state.fx.selected
}

#[test]
fn snapshot_effect_rows_and_the_generator_that_has_no_maps() {
    let mut results = SnapshotResults::new();
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1000.0, 128);
        h.state_mut().state.set_language(lang);
        effect_document(&mut h);
        h.snapshot(format!("fx_rows_generator_{}", lang.pick("ja", "en")));
        results.extend_harness(&mut h);
    }
}

#[test]
fn snapshot_a_selected_filter_and_a_selected_anchor() {
    let mut h = app(1280.0, 1000.0, 128);
    effect_document(&mut h);
    let layer = h.state().state.selected_layer.unwrap();
    let blur = h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap()[0].id();
    fx(&mut h, FxOp::SelectFilter { layer, id: blur });
    h.snapshot("fx_rows_blur_selected");
    let anchor = h.state().state.doc.layer(layer).unwrap().anchor().unwrap().id();
    fx(&mut h, FxOp::SelectAnchor(anchor));
    h.snapshot("fx_rows_anchor_selected");
}

#[test]
fn snapshot_the_add_menu() {
    let mut h = app(1280.0, 1000.0, 128);
    effect_document(&mut h);
    fx(&mut h, FxOp::Deselect);
    h.state_mut().state.property_tab = yolu_app::panels::properties::TAB_ICONS.len() - 1; // レイヤーのタブ（最後）
    h.run();
    h.get_by_label("フィルターを足す").click();
    h.run();
    assert!(matches!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::M2(yolu_app::m2_menu::Popup::AddEffect(FilterTarget::Content)))
    ));
    h.snapshot("fx_add_menu");
}

#[test]
fn rows_select_toggle_move_and_remove_from_the_panel() {
    let mut h = app(1280.0, 1000.0, 128);
    apply(&mut h, Action::M2(Edit::NewFill));
    let layer = h.state().state.selected_layer.unwrap();
    fx(&mut h, FxOp::AddFilter { target: FilterTarget::Content, kind: FilterKind::Blur });
    fx(&mut h, FxOp::AddFilter { target: FilterTarget::Content, kind: FilterKind::Invert });
    let ids: Vec<_> = h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap().iter().map(|e| e.id()).collect();
    // 層の行を押すと、選んでいた効果は外れる
    let layer_name = h.state().state.doc.layer(layer).unwrap().name().to_owned();
    h.get_by_label(&layer_name).click();
    h.run();
    assert_eq!(selected(&h), None);
    // 効果の行を押すと選ぶ（その層のまま）
    let blur_label = "ぼかし（ガウス）  4 px";
    h.get_by_label(blur_label).click();
    h.run();
    assert_eq!(selected(&h), Some(Selected::Filter { layer, id: ids[0] }));
    // 上へ（後から掛かる）: ぼかしが 1 つ上の段になる。1 回の Undo
    let steps = h.state().state.doc.undo_count();
    h.get_by_label("上へ（後から掛かる）").click();
    h.run();
    let order: Vec<_> = h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap().iter().map(|e| e.id()).collect();
    assert_eq!(order, vec![ids[1], ids[0]]);
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    // 目で無効にする
    h.get_all_by_label("フィルターを無効にする").next().unwrap().click();
    h.run();
    assert!(!h.state().state.doc.find_filter(ids[0]).unwrap().1.enabled(), "一番上の行（ぼかし）の目で無効になる");
    // 消す
    h.get_by_label("フィルターを削除").click();
    h.run();
    assert_eq!(h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap().len(), 1);
    assert_eq!(selected(&h), None, "消した行の選びは外れる");
    // Undo（Ctrl+Z）で戻る
    key(&h, egui::Key::Z, egui::Modifiers::COMMAND);
    h.run();
    assert_eq!(h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap().len(), 2);
}

#[test]
fn the_context_menu_of_a_row_acts_on_that_row() {
    let mut h = app(1280.0, 1000.0, 128);
    apply(&mut h, Action::M2(Edit::NewFill));
    let layer = h.state().state.selected_layer.unwrap();
    fx(&mut h, FxOp::AddFilter { target: FilterTarget::Content, kind: FilterKind::Blur });
    let id = h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap()[0].id();
    fx(&mut h, FxOp::Deselect);
    let at = h.get_by_label("ぼかし（ガウス）  4 px").rect().center();
    press(&h, at, PointerButton::Secondary);
    h.step();
    release(&h, at, PointerButton::Secondary);
    h.run();
    assert_eq!(selected(&h), Some(Selected::Filter { layer, id }), "右クリックで選ぶ");
    assert!(matches!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::M2(yolu_app::m2_menu::Popup::EffectContext))
    ));
    // 行の消すボタンと同じ名前なので、メニューの項目（幅のある矩形）を選ぶ
    let item = rect_of(&h, "フィルターを削除", |r| r.width() > 80.0);
    click(&mut h, item.center());
    assert!(h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap().is_empty());
}

#[test]
fn a_filter_is_added_from_the_properties_button_and_the_filter_menu() {
    let mut h = app(1280.0, 1000.0, 128);
    let layer = h.state().state.selected_layer.unwrap();
    h.state_mut().state.property_tab = yolu_app::panels::properties::TAB_ICONS.len() - 1; // レイヤーのタブ（最後）
    h.run();
    h.get_by_label("フィルターを足す").click();
    h.run();
    let item = popup_item(&h, "シャープ");
    click(&mut h, item.center());
    assert_eq!(h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap().len(), 1);
    // メニューバーの「フィルター」から
    let title = menu_title(&h, "フィルター");
    click(&mut h, title.center());
    assert!(matches!(h.state().state.popup.as_ref().map(|p| p.kind), Some(PopupKind::MenuBar(4))));
    let item = popup_item(&h, "階調の反転");
    click(&mut h, item.center());
    assert_eq!(h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap().len(), 2);
}

#[test]
fn dragging_a_slider_in_the_effect_panel_is_one_undo_step() {
    let mut h = app(1280.0, 1000.0, 128);
    let layer = h.state().state.selected_layer.unwrap();
    fx(&mut h, FxOp::AddFilter { target: FilterTarget::Content, kind: FilterKind::Blur });
    let id = h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap()[0].id();
    let steps = h.state().state.doc.undo_count();
    let slider = rect_of(&h, "半径", |r| r.left() > 1000.0);
    let y = slider.center().y + 8.0;
    drag(&mut h, &[pos2(slider.left() + 30.0, y), pos2(slider.left() + 80.0, y), pos2(slider.left() + 120.0, y)]);
    let radius = match h.state().state.doc.find_filter(id).unwrap().1.settings() {
        yolu_core::EffectSettings::Filter(yolu_core::filter::Settings::GaussianBlur { radius }) => *radius,
        other => panic!("{other:?}"),
    };
    assert_ne!(radius, 4, "つまみで半径が変わる");
    assert_eq!(h.state().state.doc.undo_count(), steps + 1, "ドラッグは 1 回の Undo");
    let _ = UiOp::EditMask(false);
}


/// 描いた文字（アイコンの頭文字は除く）の一覧。
fn shown_texts(h: &Harness<'_, YoluApp>) -> Vec<(String, Rect)> {
    fn walk(shape: &Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            Shape::Text(text) => {
                out.push((text.galley.job.text.clone(), Rect::from_min_size(text.pos, text.galley.size())));
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, &mut out);
    }
    out
}

fn has_japanese(text: &str) -> bool {
    text.chars().any(|c| matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}'))
}

#[test]
fn the_effect_screens_have_no_instruction_text_and_the_english_one_no_japanese() {
    for lang in Lang::ALL {
        for select in 0..3 {
            let mut h = app(1280.0, 1000.0, 128);
            h.state_mut().state.set_language(lang);
            effect_document(&mut h);
            let layer = h.state().state.selected_layer.unwrap();
            let stage = h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap()[0].id();
            match select {
                0 => {}
                1 => fx(&mut h, FxOp::SelectFilter { layer, id: stage }),
                _ => {
                    let anchor = h.state().state.doc.layer(layer).unwrap().anchor().unwrap().id();
                    fx(&mut h, FxOp::SelectAnchor(anchor));
                }
            }
            // 開いたメニューも
            let texts = shown_texts(&h);
            // 層の一覧とプロパティの欄（右の列。状態の帯の知らせは状態なので除く）
            for (text, rect) in texts.iter().filter(|(_, r)| r.left() > 1050.0 && r.top() > 280.0 && r.bottom() < 976.0) {
                assert!(!text.contains('。') && !text.ends_with('.'), "{lang:?}/{select}: 文の形の文字 {text:?}");
                assert!(text.chars().count() <= 40, "{lang:?}/{select}: 長い文字 {text:?} {rect:?}");
            }
            if lang == Lang::En {
                for (text, _) in &texts {
                    assert!(!has_japanese(text), "英語の画面に日本語 {text:?}");
                }
            }
        }
    }
}

#[test]
fn the_add_menu_in_english_has_no_japanese_and_in_japanese_every_name_is_localised() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1000.0, 128);
        h.state_mut().state.set_language(lang);
        h.state_mut().state.property_tab = yolu_app::panels::properties::TAB_ICONS.len() - 1;
        h.run();
        h.get_by_label(lang.pick("フィルターを足す", "Add Filter")).click();
        h.run();
        for (text, _) in shown_texts(&h) {
            if lang == Lang::En {
                assert!(!has_japanese(&text), "{text:?}");
            }
        }
        let names: Vec<String> = yolu_app::fx::menu::add_entries(&h.state().state, FilterTarget::Content)
            .iter()
            .filter_map(|e| match e {
                yolu_app::ui::menu::Entry::Item { label, .. } => Some(label.clone()),
                _ => None,
            })
            .collect();
        assert!(names.len() >= 15, "{names:?}");
    }
}

#[test]
fn a_generator_row_without_maps_has_a_mark_whose_tooltip_gives_the_reason() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1000.0, 128);
        h.state_mut().state.set_language(lang);
        effect_document(&mut h);
        let label = format!("{}  {}", lang.pick("エッジの摩耗", "Edge Wear"), lang.pick("乗算", "Multiply"));
        let row = h.get_by_label(&label).rect();
        // 選んでいる行には上へ・下へ・消すの 3 つのボタンがあり、その左に印
        let at = pos2(row.right() - 4.0 - 60.0 - 10.0, row.center().y);
        move_to(&h, at);
        for _ in 0..90 {
            h.step();
        }
        let reason = lang.pick("Curvature のマップがありません", "No Curvature map");
        let texts = shown_texts(&h);
        // 欄の警告の行にも同じ理由が出ているので、ポインタの近く（行のすぐ下）に出た文字を見る
        assert!(
            texts.iter().any(|(t, r)| t.contains(reason) && r.top() > row.top() && r.top() < row.top() + 90.0),
            "{lang:?}: {texts:?}"
        );
    }
}
