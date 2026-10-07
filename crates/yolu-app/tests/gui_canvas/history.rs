use crate::common;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use yolu_app::{
    engine::{BrushSettings, Document, HistoryKind},
    lang::Lang,
    panels::history::{go_to, show, title},
    sets::MaterialRef,
    state::AppState,
    Tab,
};

#[test]
fn rows_move_back_and_forward_in_both_languages() {
    for lang in [Lang::Ja, Lang::En] {
        let mut app = AppState::new_in(16, 16, lang);
        let id = app.doc.add_layer("private layer name").unwrap();
        app.doc.set_layer_opacity(id, 0.4, false).unwrap();
        let mut h = common::gpu_thread::builder().build_ui_state(show, app);
        h.run();
        let earlier = h.get_by_label(title(HistoryKind::AddLayer, lang));
        let later = h.get_by_label(title(HistoryKind::LayerProperties, lang));
        assert!(earlier.rect().top() < later.rect().top());
        assert_eq!(
            later.accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        h.get_by_label(title(HistoryKind::AddLayer, lang)).click();
        h.run();
        assert_eq!(h.state().doc.undo_count(), 1);
        assert_eq!(
            h.get_by_label(title(HistoryKind::AddLayer, lang))
                .accesskit_node()
                .toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        assert_eq!(h.state().doc.layer(id).unwrap().opacity(), 1.0);
        h.get_by_label(title(HistoryKind::LayerProperties, lang))
            .click();
        h.run();
        assert_eq!(h.state().doc.undo_count(), 2);
        assert_eq!(h.state().doc.layer(id).unwrap().opacity(), 0.4);
        h.get_by_label(lang.pick("開始位置", "Starting point"))
            .click();
        h.run();
        assert_eq!(h.state().doc.undo_count(), 0);
        assert!(h.state().modified);
        assert!(h.query_by_label("private layer name").is_none());
    }
}

#[test]
fn stroke_and_read_only_set_refuse_history_navigation() {
    let mut app = AppState::new(16, 16);
    let id = app.doc.add_layer("a").unwrap();
    let s = app.doc.begin_stroke(id, &BrushSettings::default()).unwrap();
    go_to(&mut app, 0);
    assert_eq!(app.doc.undo_count(), 1);
    app.doc.cancel_stroke(s);
    app.sets.get_mut(0).unwrap().read_only = Some("test".into());
    go_to(&mut app, 0);
    assert_eq!(app.doc.undo_count(), 1);
    assert!(!app.modified);
}

#[test]
fn switching_sets_uses_each_documents_history() {
    let mut app = AppState::new(16, 16);
    app.doc.add_layer("a").unwrap();
    let mut second = Document::new(16, 16).unwrap();
    let id = second.add_layer("b").unwrap();
    second.set_layer_opacity(id, 0.5, false).unwrap();
    let index = app.sets.push(
        "second".into(),
        "second".into(),
        false,
        MaterialRef::Unassigned,
        None,
        second,
    );
    app.switch_set(index).unwrap();
    assert_eq!(
        app.doc.history().collect::<Vec<_>>(),
        vec![HistoryKind::AddLayer, HistoryKind::LayerProperties]
    );
    go_to(&mut app, 0);
    app.switch_set(0).unwrap();
    assert_eq!(
        app.doc.history().collect::<Vec<_>>(),
        vec![HistoryKind::AddLayer]
    );
    assert_eq!(app.doc.undo_count(), 1);
    app.switch_set(index).unwrap();
    assert_eq!(app.doc.undo_count(), 0);
    assert_eq!(app.doc.redo_count(), 2);
}

#[test]
fn tab_is_available_and_invalid_position_is_ignored() {
    assert_eq!(Tab::History.title_in(Lang::Ja), "ヒストリー");
    assert_eq!(Tab::History.title_in(Lang::En), "History");
    assert!(yolu_app::app::default_dock()
        .find_tab(&Tab::History)
        .is_some());
    let mut app = AppState::new(16, 16);
    app.doc.add_layer("a").unwrap();
    go_to(&mut app, 2);
    assert_eq!(app.doc.undo_count(), 1);
    assert!(!app.modified);
}

#[test]
fn empty_and_evicted_history_do_not_show_discarded_steps() {
    let mut h = common::gpu_thread::builder().build_ui_state(show, AppState::new(16, 16));
    h.run();
    assert!(h.query_by_label("開始位置").is_none());
    let id = h.state_mut().doc.add_layer("a").unwrap();
    h.state_mut().doc.set_layer_opacity(id, 0.5, false).unwrap();
    let cost = h.state().doc.history_bytes() - 128;
    h.state_mut().doc.set_undo_budget_bytes(cost).unwrap();
    h.run();
    assert!(h.query_by_label("レイヤーを追加").is_none());
    h.get_by_label("レイヤーを変える");
    h.get_by_label("開始位置").click();
    h.run();
    assert_eq!(h.state().doc.layer(id).unwrap().opacity(), 1.0);
}

#[test]
fn history_rows_are_disabled_during_a_stroke() {
    let mut app = AppState::new(16, 16);
    let id = app.doc.add_layer("a").unwrap();
    let s = app.doc.begin_stroke(id, &BrushSettings::default()).unwrap();
    let mut h = common::gpu_thread::builder().build_ui_state(show, app);
    h.run();
    assert!(h.get_by_label("開始位置").accesskit_node().is_disabled());
    assert!(h
        .get_by_label("レイヤーを追加")
        .accesskit_node()
        .is_disabled());
    h.state_mut().doc.cancel_stroke(s);
    h.run();
    assert!(!h.get_by_label("開始位置").accesskit_node().is_disabled());
}

#[test]
fn long_history_only_builds_visible_rows_and_keeps_absolute_positions() {
    let mut app = AppState::new_in(16, 16, Lang::En);
    let base = app.doc.layers()[0].id();
    for i in 0..5_000 {
        app.doc
            .set_layer_opacity(base, if i % 2 == 0 { 0.3 } else { 0.7 }, false)
            .unwrap();
    }
    let marker = app.doc.add_layer("marker").unwrap();
    for i in 0..5_000 {
        app.doc
            .set_layer_opacity(base, if i % 2 == 0 { 0.3 } else { 0.7 }, false)
            .unwrap();
    }
    app.doc.remove_layer(marker).unwrap();
    assert_eq!(app.doc.undo_count(), 10_002);
    let mut h = common::gpu_thread::builder()
        .with_size(egui::vec2(300.0, 240.0))
        .with_max_steps(120)
        .build_ui_state(
            move |ui, state| {
                let row_pitch = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
                show(ui, state);
                // スクロール量は本番と同じスタイルの一行の高さから求める。
                ui.ctx()
                    .data_mut(|d| d.insert_temp(egui::Id::new("history_test_pitch"), row_pitch));
            },
            app,
        );
    h.run();
    assert!(
        h.query_all_by_label("Edit layer").count() <= 20,
        "画面外の履歴の行を生成している"
    );
    assert!(h.query_by_label("Remove layer").is_none());
    let pitch = h.ctx.data(|d| {
        d.get_temp::<f32>(egui::Id::new("history_test_pitch"))
            .unwrap()
    });
    let scroll = |h: &Harness<'_, AppState>, delta| {
        h.event(egui::Event::PointerMoved(egui::pos2(100.0, 100.0)));
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            delta: egui::vec2(0.0, delta),
            modifiers: egui::Modifiers::NONE,
        });
    };
    scroll(&h, -pitch * 5_000.0);
    h.run();
    assert!(h.query_all_by_label("Edit layer").count() <= 20);
    h.get_by_label("Add layer").click();
    h.run();
    assert_eq!(h.state().doc.undo_count(), 5_001);
    assert_eq!(
        h.get_by_label("Add layer").accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    let marker_y = h.get_by_label("Add layer").rect().top();
    h.query_all_by_label("Edit layer")
        .find(|n| n.rect().top() > marker_y)
        .unwrap()
        .click();
    h.run();
    assert_eq!(h.state().doc.undo_count(), 5_002);
    scroll(&h, -1_000_000.0);
    h.run();
    h.get_by_label("Remove layer").click();
    h.run();
    assert_eq!(h.state().doc.undo_count(), 10_002);
    assert_eq!(
        h.get_by_label("Remove layer").accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    scroll(&h, 1_000_000.0);
    h.run();
    h.get_by_label("Starting point").click();
    h.run();
    assert_eq!(h.state().doc.undo_count(), 0);
}
