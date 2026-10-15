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
    assert!(!h.app().subtool_is_modified(Tool::Eyedropper, list.current()));
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
