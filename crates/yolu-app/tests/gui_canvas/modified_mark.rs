//! 変更の印（メニューバーの右の「•」）: 画素の変わらないストローク・取り消したストロークでは立たず、変えたストロークでは立ち、
//! Undo で保存した所へ戻れば消える。文書の外の変更の印は、Undo で保存した所へ戻っても残る。
use crate::common;
use crate::common::drive::Drive;

use common::*;
use egui::{Key, Modifiers};
use yolu_app::state::{Action, Tool};

#[test]
fn a_stroke_marks_the_project_and_undo_back_to_the_saved_point_clears_the_mark() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    assert!(!h.modified());
    drag(&mut h, &[c, offset(c, 40.0, 0.0)]);
    assert_eq!(h.doc().undo_count(), 1);
    assert!(h.modified(), "描いた");
    h.act(Action::Undo);
    assert!(!h.modified(), "保存した所（開いたとき）へ Undo で戻った");
    h.act(Action::Redo);
    assert!(h.modified());
}

#[test]
fn undo_to_a_point_saved_after_a_stroke_clears_the_mark_and_a_further_edit_sets_it() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    drag(&mut h, &[c, offset(c, 40.0, 0.0)]);
    h.set_modified(false); // 保存した
    assert!(!h.modified());
    drag(&mut h, &[offset(c, 0.0, 30.0), offset(c, 40.0, 30.0)]);
    assert!(h.modified());
    h.act(Action::Undo);
    assert!(!h.modified(), "保存した所へ戻った");
    h.act(Action::Undo);
    assert!(h.modified(), "保存した所より前");
}

#[test]
fn a_stroke_that_leaves_every_pixel_alone_does_not_mark_the_project() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    // 何も描いていない透明な所を消しゴムでなぞっても、画素は変わらない
    h.act(Action::SelectTool(Tool::Eraser));
    drag(&mut h, &[c, offset(c, 40.0, 0.0)]);
    assert_eq!(h.doc().undo_count(), 0, "段を積まない");
    assert!(!h.modified());
}

#[test]
fn a_cancelled_stroke_does_not_mark_the_project_but_marks_it_while_drawing() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    press(&h, c, egui::PointerButton::Primary);
    h.step();
    move_to(&h, offset(c, 30.0, 0.0));
    h.step();
    assert!(h.doc().has_active_stroke());
    assert!(h.modified(), "描いている間");
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    release(&h, offset(c, 30.0, 0.0), egui::PointerButton::Primary);
    h.run();
    assert!(!h.doc().has_active_stroke());
    assert_eq!(h.doc().undo_count(), 0);
    assert!(!h.modified(), "取り消した");
}

#[test]
fn a_mark_for_a_change_outside_the_document_stays_after_undo_to_the_saved_point() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    drag(&mut h, &[c, offset(c, 40.0, 0.0)]);
    h.set_modified(true); // 文書の外の変更（セットの名前・ポーズなど）
    h.act(Action::Undo);
    assert!(h.modified());
}

#[test]
fn layer_edits_mark_the_project_and_undo_clears_it() {
    let mut h = app(1280.0, 800.0, 64);
    let layer = h.selected_layer().unwrap();
    for (name, action) in [
        ("追加", Action::NewLayer),
        ("表示", Action::ToggleVisible(layer)),
        (
            "合成モード",
            Action::SetBlend(layer, yolu_app::engine::BlendMode::Multiply),
        ),
    ] {
        assert!(!h.modified(), "{name}の前");
        h.act(action);
        assert!(h.modified(), "{name}");
        h.act(Action::Undo);
        assert!(!h.modified(), "{name}を Undo で戻した");
    }
    let before = h.doc().revision();
    h.act(Action::RenameLayer(layer, "別の名前".into()));
    assert_ne!(h.doc().revision(), before);
    assert!(h.modified(), "名前を変えた");
    h.act(Action::Undo);
    assert!(!h.modified());
    let current = h.doc().layer(layer).unwrap().name().to_owned();
    h.act(Action::RenameLayer(layer, current));
    assert!(!h.modified(), "同じ名前への変更では立たない");
}

#[test]
fn switching_texture_sets_carries_the_unsaved_edit_over_and_only_saving_clears_it() {
    let mut h = app(1280.0, 800.0, 256);
    let first = h.sets().iter().next().unwrap().uid;
    h.app_mut().add_texture_set().unwrap();
    let second = h.sets().iter().nth(1).unwrap().uid;
    h.act(Action::SelectSet(first));
    h.set_modified(false); // 保存した（セットの追加の印を下ろす）
    assert!(!h.modified());
    let c = canvas_rect(&h).center();
    drag(&mut h, &[c, offset(c, 40.0, 0.0)]);
    assert!(h.modified(), "最初のセットで描いた");
    h.act(Action::SelectSet(second));
    assert!(h.modified(), "セットを替えても、保存していない編集は残る");
    h.act(Action::SelectSet(first));
    h.act(Action::Undo);
    assert!(
        h.modified(),
        "戻って Undo しても、替えた間に外れた編集の印は残る"
    );
    h.set_modified(false); // 保存した
    assert!(!h.modified());
}
