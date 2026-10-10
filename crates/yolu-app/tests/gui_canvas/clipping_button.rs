//! クリッピングのボタン（レイヤーの欄の下の帯。CLIP STUDIO と同じ「下のレイヤーでクリッピング」の切り替え）: 押すと 1 回の Undo で入り切りし、
//! 押している状態が分かり、押せないレイヤー（一番下・グループの中で下が無い）は理由つきで無効、プロパティのレイヤーの欄にクリッピングの項目は無い。
use crate::common;

use common::*;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::lang::Lang;
use yolu_app::panels::layers::{clipping_name, clipping_state, clipping_tooltip};
use yolu_app::state::Action;
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

fn name() -> &'static str {
    clipping_name(Lang::Ja)
}

fn new_layer(h: &mut H) {
    h.state_mut().state.apply(Action::NewLayer);
    h.run();
}

fn is_disabled(h: &H, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

#[test]
fn the_button_clips_the_selected_layer_to_the_one_below_and_back_with_one_undo_each() {
    let mut h = app(1280.0, 800.0, 128);
    new_layer(&mut h);
    let top = h.state().state.selected_layer.unwrap();
    assert!(!h.state().state.doc.layer(top).unwrap().clipping());
    assert!(!is_disabled(&h, name()));
    let steps = h.state().state.doc.undo_count();
    h.get_by_label(name()).click();
    h.run();
    let doc = &h.state().state.doc;
    assert!(
        doc.layer(top).unwrap().clipping(),
        "押すとクリッピングが入る"
    );
    assert!(
        doc.is_effectively_clipped(doc.layer_index(top).unwrap()),
        "下のレイヤーがあるので効く"
    );
    assert_eq!(doc.undo_count(), steps + 1, "1 回の Undo");
    assert_eq!(clipping_state(&h.state().state), (true, None));
    h.snapshot("layers_clipping_button");
    // もう一度押すと外れる
    h.get_by_label(name()).click();
    h.run();
    assert!(!h.state().state.doc.layer(top).unwrap().clipping());
    assert_eq!(h.state().state.doc.undo_count(), steps + 2);
    // Undo で入り直し、もう 1 回で外れる
    h.state_mut().state.apply(Action::Undo);
    assert!(h.state().state.doc.layer(top).unwrap().clipping());
    h.state_mut().state.apply(Action::Undo);
    assert!(!h.state().state.doc.layer(top).unwrap().clipping());
    assert_eq!(h.state().state.doc.undo_count(), steps);
}

#[test]
fn the_bottom_layer_cannot_be_clipped_and_the_tooltip_says_why() {
    let mut h = app(1280.0, 800.0, 128);
    // 1 枚だけ: 一番下
    assert_eq!(
        clipping_state(&h.state().state),
        (false, Some("一番下のレイヤーです"))
    );
    let tip = format!("{}（一番下のレイヤーです）", name());
    assert!(is_disabled(&h, &tip), "押せない");
    let steps = h.state().state.doc.undo_count();
    h.get_by_label(&tip).click();
    h.run();
    assert_eq!(h.state().state.doc.undo_count(), steps);
    // 上に足すと、一番下のほうは相変わらず無効、上のほうは有効
    new_layer(&mut h);
    assert!(!is_disabled(&h, name()));
    let bottom = h.state().state.doc.layers()[0].id();
    h.state_mut().state.selected_layer = Some(bottom);
    h.run();
    assert!(is_disabled(&h, &tip));
}

#[test]
fn the_bottom_layer_of_a_group_cannot_be_clipped_but_the_one_above_it_can() {
    let mut h = app(1280.0, 800.0, 128);
    new_layer(&mut h);
    let a = h.state().state.selected_layer.unwrap();
    h.state_mut()
        .state
        .apply(Action::M2(yolu_app::m2::Edit::GroupSelected));
    h.run();
    let group = h.state().state.selected_layer.unwrap();
    assert_ne!(group, a);
    // グループの中の 1 枚: 下に兄弟が無いので無効（グループの外の下のレイヤーには付けられない）
    h.state_mut().state.selected_layer = Some(a);
    h.run();
    let tip = format!("{}（グループの中で一番下のレイヤーです）", name());
    assert!(is_disabled(&h, &tip));
    // 兄弟を足すと、その上のレイヤーは有効
    h.state_mut().state.apply(Action::NewLayer);
    h.run();
    let b = h.state().state.selected_layer.unwrap();
    assert_eq!(h.state().state.doc.layer(b).unwrap().parent(), Some(group));
    assert!(!is_disabled(&h, name()));
    h.get_by_label(name()).click();
    h.run();
    assert!(h.state().state.doc.layer(b).unwrap().clipping());
}

#[test]
fn a_flag_left_on_a_layer_with_nothing_below_can_still_be_cleared_from_the_button() {
    let mut h = app(1280.0, 800.0, 128);
    new_layer(&mut h);
    let bottom = h.state().state.doc.layers()[0].id();
    // 一番下のレイヤーの印は効かない（`is_effectively_clipped` は偽）が、印そのものは外せる
    h.state_mut()
        .state
        .doc
        .set_layer_clipping(bottom, true)
        .unwrap();
    h.state_mut().state.selected_layer = Some(bottom);
    h.run();
    assert_eq!(clipping_state(&h.state().state), (true, None));
    assert!(!is_disabled(&h, name()));
    h.get_by_label(name()).click();
    h.run();
    assert!(!h.state().state.doc.layer(bottom).unwrap().clipping());
}

#[test]
fn the_layer_properties_have_no_clipping_row_and_the_button_is_in_the_layer_panel() {
    let mut h = app(1600.0, 900.0, 128);
    new_layer(&mut h);
    // プロパティの「レイヤー」のタブ（2 つ目。同じ名前のドックのタブの下にある）
    let props = h.state().tab_rects[&yolu_app::Tab::Properties];
    let tab = rect_of(&h, "レイヤー", |r| {
        r.top() > props.bottom() && r.top() < props.bottom() + 40.0
    });
    click(&mut h, tab.center());
    assert_eq!(h.state().state.ui.property_tab, 1);
    assert!(
        h.query_all_by_label("クリッピング").next().is_none(),
        "プロパティのレイヤーの欄にクリッピングの項目は無い"
    );
    // ボタンはレイヤーの欄（右の列、レイヤーのタブの下）にある
    let layers_tab = h.state().tab_rects[&yolu_app::Tab::Layers];
    let properties_tab = h.state().tab_rects[&yolu_app::Tab::Properties];
    let button = h.get_by_label(name()).rect();
    assert!(
        button.top() > layers_tab.bottom() && button.bottom() < properties_tab.top(),
        "{button:?}"
    );
}

#[test]
fn the_button_is_disabled_while_stroking() {
    let mut h = app(1280.0, 800.0, 128);
    new_layer(&mut h);
    let c = canvas_rect(&h).center();
    press(&h, c, egui::PointerButton::Primary);
    h.step();
    assert!(h.state().state.is_stroking());
    assert!(is_disabled(&h, name()), "描いている間は押せない");
    release(&h, c, egui::PointerButton::Primary);
    h.run();
    assert!(!is_disabled(&h, name()));
}

#[test]
fn the_button_speaks_english_with_the_same_reasons() {
    let mut h = app(1280.0, 800.0, 128);
    h.state_mut().state.lang = Lang::En;
    h.run();
    let en = clipping_name(Lang::En);
    assert_eq!(en, "Clip to the Layer Below");
    let tip = clipping_tooltip(Lang::En, Some("Bottom layer"));
    assert_eq!(tip, "Clip to the Layer Below (Bottom layer)");
    assert!(is_disabled(&h, &tip), "英語でも理由つきで無効");
    new_layer(&mut h);
    assert!(!is_disabled(&h, en));
}
