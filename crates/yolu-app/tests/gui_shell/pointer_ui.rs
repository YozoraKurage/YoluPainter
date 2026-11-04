//! ポインターレイヤーの画面: レイヤーの右クリックの「ポインターレイヤーを作成」、追加のメニューの「新規ポインターレイヤー」、プロパティの指す先と読む元。
use crate::common;
use crate::common::drive::Drive;

use common::*;
use egui_kittest::kittest::Queryable;
use yolu_app::engine::LayerKind;
use yolu_core::PointerTarget;

#[test]
fn the_layer_context_menu_creates_a_pointer_to_that_layer_in_one_undo() {
    let mut h = app(1280.0, 800.0, 64);
    let target = h.selected_layer().unwrap();
    let name = h.doc().layer(target).unwrap().name().to_owned();
    h.doc_mut().clear_history().unwrap();
    h.run();
    choose_in_layer_menu(&mut h, &name, "ポインターレイヤーを作成");
    h.run();
    let id = h.selected_layer().unwrap();
    let layer = h.doc().layer(id).unwrap();
    assert_eq!(layer.kind(), LayerKind::Pointer);
    assert_eq!(
        layer.pointer().unwrap().target(),
        PointerTarget::Layer(target)
    );
    assert_eq!(h.doc().undo_count(), 1);
}

#[test]
fn the_add_menu_creates_a_pointer_and_the_properties_show_its_target_and_sources() {
    let mut h = app(1280.0, 800.0, 64);
    let name = h
        .doc()
        .layer(h.selected_layer().unwrap())
        .unwrap()
        .name()
        .to_owned();
    choose_in_layer_menu(&mut h, &name, "新規ポインターレイヤー");
    h.run();
    let id = h.selected_layer().unwrap();
    assert_eq!(h.doc().layer(id).unwrap().kind(), LayerKind::Pointer);
    // 上の見出しを閉じて、ポインターの欄をウィンドウの中に入れる
    for key in ["layer", "locks"] {
        h.ui_mut().sections.insert(key, false);
    }
    h.run();
    assert!(
        h.query_by_label_contains("指す先:").is_some(),
        "指す先のドロップダウン"
    );
    assert!(
        h.query_by_label_contains("カラー: カラー").is_some(),
        "読む元の行（既定は同じチャンネル）"
    );
}

/// 2 つ目のテクスチャセット（長い名前）と、グループの中のレイヤーのある文書で、ほかのセットを指すポインターレイヤーを選んだ形にする。
fn pointer_to_another_set(
    h: &mut egui_kittest::Harness<'_, yolu_app::YoluApp>,
    lang: yolu_app::lang::Lang,
) {
    use yolu_app::fx::pointers::{set_key, PointerOp};
    use yolu_app::fx::FxOp;
    use yolu_app::m2::UiOp;
    use yolu_app::state::Action;
    h.act(Action::M2Ui(UiOp::Language(lang)));
    h.run();
    h.act(Action::M2(yolu_app::m2::Edit::GroupSelected));
    let app = h.app_mut();
    let uid = app.add_texture_set().unwrap();
    app.rename_set(
        uid,
        "とても長い名前のテクスチャセット Very Long Texture Set Name",
    )
    .unwrap();
    let index = app.session.sets.index_of(uid).unwrap();
    let key = yolu_core::ExternalSet {
        texture_set: set_key(&app.session.sets.get(index).unwrap().id).unwrap(),
        layer_set: app.set_doc(index).layer_sets()[0].id.0,
    };
    app.switch_set(0).unwrap();
    h.act(Action::Fx(FxOp::Pointer(PointerOp::New(None))));
    let id = h.selected_layer().unwrap();
    h.act(Action::Fx(FxOp::Pointer(PointerOp::Target {
        layer: id,
        target: PointerTarget::External(key),
    })));
    // 上の見出しを閉じて、ポインターの欄をウィンドウの中に入れる
    for section in ["layer", "locks"] {
        h.ui_mut().sections.insert(section, false);
    }
    h.run();
}

#[test]
fn snapshots_of_the_pointer_properties_and_the_open_target_list_in_both_languages() {
    use yolu_app::lang::Lang;
    use yolu_app::Tab;
    let mut results = egui_kittest::SnapshotResults::new();
    for (lang, name) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let mut h = app(1500.0, 1300.0, 64);
        pointer_to_another_set(&mut h, lang);
        let rect = common::shot::group(&h, Tab::Properties);
        common::shot::clean_region(&mut h, &format!("pointer_properties_{name}"), rect);
        // 指す先の一覧を開く（節の見出し・字下げ・押せない行・長いセット名）
        let label = lang.pick("指す先:", "Target:");
        let at = h
            .query_by_label_contains(label)
            .expect("指す先の行")
            .rect()
            .center();
        click(&mut h, at);
        h.run();
        let rect = common::shot::with_popup(&h, rect);
        common::shot::region(&mut h, &format!("pointer_target_list_{name}"), rect);
        results.extend_harness(&mut h);
    }
}
