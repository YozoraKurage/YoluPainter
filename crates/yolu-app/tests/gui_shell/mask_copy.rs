//! レイヤーマスクのコピー・移動・貼り付けと、白・黒のマスクの追加（メニュー・マスクのサムネイルのドラッグ・Ctrl+V）。
use crate::common;
use crate::common::drive::Drive;

use common::*;
use egui::{pos2, Event, Modifiers, PointerButton, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::clipboard::os::{ClipImage, MemoryClipboard};
use yolu_app::clipboard::ClipAction;
use yolu_app::engine::LayerId;
use yolu_app::m2::{Edit, UiOp};
use yolu_app::notice::Kind;
use yolu_app::state::{Action, AppState};
use yolu_app::ui::menu::Entry;
use yolu_app::YoluApp;
use yolu_core::LayerLocks;

type H = Harness<'static, YoluApp>;

// ───────── ツール ─────────

/// 64 × 64 の文書にレイヤーが `n` 枚（下から順）。履歴は空。
fn layers_of(n: usize) -> (AppState, Vec<LayerId>) {
    let mut s = AppState::new(64, 64);
    for _ in 1..n {
        s.act(Action::NewLayer);
    }
    s.doc_mut().clear_history().unwrap();
    let ids = s.doc().layers().iter().map(|l| l.id()).collect();
    (s, ids)
}

fn hide(s: &AppState, id: LayerId, x: u32, y: u32) -> u8 {
    s.doc()
        .layer(id)
        .unwrap()
        .mask()
        .unwrap()
        .surface()
        .pixel(x, y)
        .unwrap()
        .a
}

fn has_mask(s: &AppState, id: LayerId) -> bool {
    s.doc().layer(id).unwrap().mask().is_some()
}

/// 左半分を見える（白）、右半分を隠す（黒）の外の画像（64 × 64）。
fn half_white() -> ClipImage {
    let mut rgba = Vec::new();
    for _row in 0..64 {
        for col in 0..64 {
            let v = if col < 32 { 255 } else { 0 };
            rgba.extend([v, v, v, 255]);
        }
    }
    ClipImage::new(64, 64, rgba).unwrap()
}

fn entry_of<'a>(entries: &'a [Entry<Action>], label: &str) -> Option<(&'a Action, bool)> {
    entries.iter().find_map(|e| match e {
        Entry::Item {
            label: l,
            action,
            enabled,
            ..
        } if l == label => Some((action, *enabled)),
        _ => None,
    })
}

// ───────── 白・黒のマスクの追加 ─────────

#[test]
fn headless_a_white_and_a_black_mask_are_each_one_undo_and_target_the_mask() {
    let (mut s, ids) = layers_of(2);
    let (a, b) = (ids[0], ids[1]);
    s.act(Action::M2(Edit::AddMask(a)));
    assert_eq!(hide(&s, a, 5, 5), 0, "白は何も隠さない");
    assert_eq!(s.doc().undo_count(), 1);
    s.act(Action::M2(Edit::AddMaskBlack(b)));
    assert_eq!(hide(&s, b, 5, 5), 255, "黒は全体を隠す");
    assert_eq!(hide(&s, b, 63, 63), 255);
    assert_eq!(s.doc().undo_count(), 2, "足すのと隠すのは 1 回の取り消し");
    assert_eq!(s.selected_layer(), Some(b));
    assert!(s.m2().edit_mask, "描く先はマスク");
    s.act(Action::Undo);
    assert!(!has_mask(&s, b), "1 回の取り消しで戻る");
    assert!(has_mask(&s, a));
    // 持っているレイヤーへは断る
    s.act(Action::M2(Edit::AddMaskBlack(a)));
    assert_eq!(s.last_notice().as_ref().map(|n| n.kind), Some(Kind::Error));
    assert_eq!(s.doc().undo_count(), 1);
}

#[test]
fn headless_the_layer_menu_offers_white_and_black_masks_and_copy_and_paste() {
    let (mut s, ids) = layers_of(3);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    s.act(Action::M2(Edit::AddMaskBlack(a)));
    // マスクの無いレイヤー: 白・黒の追加があり、コピーは無く、貼り付けはコピーするまで押せない
    let menu = yolu_app::shell::layer_menu(&s, Some(b));
    assert_eq!(
        entry_of(&menu, "白のレイヤーマスクを追加").map(|(a, _)| a),
        Some(&Action::M2(Edit::AddMask(b)))
    );
    assert_eq!(
        entry_of(&menu, "黒のレイヤーマスクを追加").map(|(a, _)| a),
        Some(&Action::M2(Edit::AddMaskBlack(b)))
    );
    assert!(entry_of(&menu, "マスクをコピー").is_none());
    assert_eq!(
        entry_of(&menu, "マスクに貼り付け").map(|(_, on)| on),
        Some(false)
    );
    // マスクのあるレイヤー: コピーが出て、白・黒の追加は出ない
    let menu = yolu_app::shell::layer_menu(&s, Some(a));
    assert_eq!(
        entry_of(&menu, "マスクをコピー"),
        Some((&Action::M2(Edit::CopyMask(a)), true))
    );
    assert!(entry_of(&menu, "白のレイヤーマスクを追加").is_none());
    s.act(Action::M2(Edit::CopyMask(a)));
    assert!(s.clip().mask.is_some());
    assert!(s.message().contains("マスク"), "{}", s.message());
    // コピーしたあとは押せて、複数選んでいれば選んだ全部に貼る
    s.app_mut().select_layers([b, c], c);
    let menu = yolu_app::shell::layer_menu(&s, Some(c));
    assert_eq!(
        entry_of(&menu, "マスクに貼り付け"),
        Some((&Action::M2(Edit::PasteMask(vec![b, c])), true))
    );
    s.app_mut().select_single_layer(c);
    let menu = yolu_app::shell::layer_menu(&s, Some(c));
    assert_eq!(
        entry_of(&menu, "マスクに貼り付け").map(|(a, _)| a),
        Some(&Action::M2(Edit::PasteMask(vec![c])))
    );
}

// ───────── コピーと貼り付け ─────────

#[test]
fn headless_copy_does_not_change_the_document_and_paste_to_several_layers_is_one_undo() {
    let (mut s, ids) = layers_of(3);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    s.act(Action::M2(Edit::AddMaskBlack(a)));
    s.act(Action::M2(Edit::AddMask(c))); // c は白のマスクを持っている（置き換えられる）
    s.doc_mut().clear_history().unwrap();
    s.set_modified(false);
    s.act(Action::M2(Edit::CopyMask(a)));
    assert_eq!(s.doc().undo_count(), 0, "コピーは文書を変えない");
    assert!(!s.modified());
    s.act(Action::M2(Edit::PasteMask(vec![b, c])));
    assert_eq!(s.doc().undo_count(), 1, "複数に貼っても 1 回");
    assert_eq!((hide(&s, b, 9, 9), hide(&s, c, 9, 9)), (255, 255));
    s.act(Action::Undo);
    assert!(!has_mask(&s, b), "マスクの無かったレイヤーは元に戻る");
    assert_eq!(hide(&s, c, 9, 9), 0, "置き換えられたマスクが戻る");
}

#[test]
fn headless_pasting_without_a_copied_mask_is_refused_and_changes_nothing() {
    let (mut s, ids) = layers_of(2);
    s.act(Action::M2(Edit::PasteMask(vec![ids[0]])));
    assert_eq!(
        s.last_notice().as_ref().map(|n| n.kind),
        Some(Kind::Refusal)
    );
    assert!(!has_mask(&s, ids[0]));
    assert_eq!(s.doc().undo_count(), 0);
}

#[test]
fn headless_a_mask_copied_from_another_canvas_size_is_refused() {
    let (mut s, ids) = layers_of(2);
    s.act(Action::M2(Edit::AddMaskBlack(ids[0])));
    s.act(Action::M2(Edit::CopyMask(ids[0])));
    let mut other = AppState::new(32, 32);
    other.clip_mut().mask = s.clip().mask.clone();
    let target = other.selected_layer().unwrap();
    other.act(Action::M2(Edit::PasteMask(vec![target])));
    assert_eq!(
        other.last_notice().as_ref().map(|n| n.kind),
        Some(Kind::Error),
        "{}",
        other.message()
    );
    assert!(!has_mask(&other, target));
    assert_eq!(other.doc().undo_count(), 0);
}

#[test]
fn headless_move_and_copy_to_are_one_undo_and_a_locked_target_is_refused() {
    let (mut s, ids) = layers_of(3);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    s.act(Action::M2(Edit::AddMaskBlack(a)));
    s.doc_mut().clear_history().unwrap();
    s.act(Action::M2(Edit::MoveMask { from: a, to: b }));
    assert!(!has_mask(&s, a) && has_mask(&s, b));
    assert!(!s.m2().edit_mask, "なくなったマスクは描く先でなくなる");
    assert_eq!(s.doc().undo_count(), 1);
    s.act(Action::M2(Edit::CopyMaskTo { from: b, to: c }));
    assert!(has_mask(&s, b) && has_mask(&s, c));
    assert_eq!(s.doc().undo_count(), 2);
    s.act(Action::Undo);
    s.act(Action::Undo);
    assert!(has_mask(&s, a) && !has_mask(&s, b) && !has_mask(&s, c));
    // ロックされた行き先は、何も変えずに理由を言って断る
    s.doc_mut().set_layer_locks(c, LayerLocks::ALL).unwrap();
    s.doc_mut().clear_history().unwrap();
    s.act(Action::M2(Edit::MoveMask { from: a, to: c }));
    assert_eq!(
        s.last_notice().as_ref().map(|n| n.kind),
        Some(Kind::Refusal)
    );
    assert!(has_mask(&s, a) && !has_mask(&s, c));
    assert_eq!(s.doc().undo_count(), 0);
}

#[test]
fn headless_ctrl_v_pastes_into_the_mask_while_the_mask_is_the_target() {
    let (mut s, ids) = layers_of(1);
    let a = ids[0];
    let os = MemoryClipboard::new();
    s.clip_mut().set_os(Box::new(os.clone()));
    os.set(Some(half_white()));
    s.act(Action::M2(Edit::AddMask(a)));
    s.doc_mut().clear_history().unwrap();
    assert!(s.m2().edit_mask);
    s.act(Action::Clip(ClipAction::Paste));
    assert_eq!(s.doc().layers().len(), 1, "新しいレイヤーは作らない");
    assert_eq!(hide(&s, a, 10, 10), 0, "白は見える");
    assert_eq!(hide(&s, a, 50, 10), 255, "黒は隠す");
    assert!(s.m2().edit_mask, "マスクが対象のまま");
    assert_eq!(s.doc().undo_count(), 1);
    s.act(Action::Undo);
    assert_eq!(hide(&s, a, 50, 10), 0);
    // マスクを描いていないときは、今までどおり新しいレイヤー
    s.act(Action::M2Ui(UiOp::EditMask(false)));
    s.act(Action::Clip(ClipAction::Paste));
    assert_eq!(s.doc().layers().len(), 2);
}

// ───────── 画面: サムネイルのドラッグと右クリック ─────────

const MASK_THUMB: &str = "レイヤーマスク（押すとマスクが対象）";

/// マスクのサムネイルの矩形（上の行から）。
fn mask_thumbs(h: &H) -> Vec<Rect> {
    let body = layers_body(h);
    let mut v: Vec<Rect> = h
        .get_all_by_label(MASK_THUMB)
        .map(|n| n.rect())
        .filter(|r| body.contains_rect(*r))
        .collect();
    v.sort_by(|a, b| a.top().total_cmp(&b.top()));
    v
}

fn row_of(h: &H, id: LayerId) -> Rect {
    let name = h.doc().layer(id).unwrap().name().to_owned();
    let body = layers_body(h);
    rect_of(h, &name, |r| body.contains_rect(r) && r.width() > 60.0)
}

/// 2 つのレイヤー（a が下、b が上）。a だけが黒のマスクを持つ。履歴は空。
fn two_layers() -> (H, LayerId, LayerId) {
    let mut h = app(1280.0, 800.0, 64);
    h.act(Action::NewLayer);
    h.run();
    let ids: Vec<LayerId> = h.doc().layers().iter().map(|l| l.id()).collect();
    let (a, b) = (ids[0], ids[1]);
    h.act(Action::M2(Edit::AddMaskBlack(a)));
    h.act(Action::M2Ui(UiOp::EditMask(false)));
    h.doc_mut().clear_history().unwrap();
    h.run();
    (h, a, b)
}

/// マスクのサムネイルを押し、`to` まで動かして、`release_modifiers` を押しながら離す。
fn drag_mask(h: &mut H, from: egui::Pos2, to: egui::Pos2, release_modifiers: Modifiers) {
    press(h, from, PointerButton::Primary);
    h.step();
    move_to(h, offset(from, 0.0, 10.0));
    h.step();
    move_to(h, to);
    h.step();
    h.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: release_modifiers,
    });
    h.step();
    h.run();
}

#[test]
fn dragging_the_mask_thumbnail_to_another_layer_moves_the_mask_in_one_undo() {
    let (mut h, a, b) = two_layers();
    let order: Vec<LayerId> = h.doc().layers().iter().map(|l| l.id()).collect();
    let thumb = mask_thumbs(&h)[0];
    let row = row_of(&h, b);
    // ドラッグ中は、落とす先の行が分かる
    press(&h, thumb.center(), PointerButton::Primary);
    h.step();
    move_to(&h, offset(thumb.center(), 0.0, -10.0));
    h.step();
    move_to(&h, pos2(row.left() + 40.0, row.center().y));
    h.step();
    assert_eq!(
        h.ui().mask_drag.map(|d| (d.from, d.target)),
        Some((a, Some(b)))
    );
    assert!(
        h.ui().layer_drag.is_none(),
        "並べ替えのドラッグにはならない"
    );
    release(
        &h,
        pos2(row.left() + 40.0, row.center().y),
        PointerButton::Primary,
    );
    h.step();
    h.run();
    assert!(h.ui().mask_drag.is_none());
    assert!(
        h.doc().layer(a).unwrap().mask().is_none(),
        "元からは無くなる"
    );
    assert!(h.doc().layer(b).unwrap().mask().is_some(), "行き先に付く");
    let now: Vec<LayerId> = h.doc().layers().iter().map(|l| l.id()).collect();
    assert_eq!(now, order, "レイヤーの並びは変わらない");
    assert_eq!(h.doc().undo_count(), 1);
    h.act(Action::Undo);
    h.run();
    assert!(h.doc().layer(a).unwrap().mask().is_some());
    assert!(h.doc().layer(b).unwrap().mask().is_none());
}

#[test]
fn releasing_with_alt_copies_the_mask_instead_of_moving_it() {
    let (mut h, a, b) = two_layers();
    let thumb = mask_thumbs(&h)[0];
    let row = row_of(&h, b);
    drag_mask(
        &mut h,
        thumb.center(),
        pos2(row.left() + 40.0, row.center().y),
        Modifiers::ALT,
    );
    assert!(
        h.doc().layer(a).unwrap().mask().is_some(),
        "元のマスクは残る"
    );
    assert!(h.doc().layer(b).unwrap().mask().is_some());
    assert_eq!(h.doc().undo_count(), 1, "コピーも 1 回の取り消し");
    h.act(Action::Undo);
    h.run();
    assert!(h.doc().layer(a).unwrap().mask().is_some());
    assert!(h.doc().layer(b).unwrap().mask().is_none());
}

#[test]
fn dropping_a_mask_on_its_own_row_or_outside_the_list_does_nothing() {
    let (mut h, a, b) = two_layers();
    let thumb = mask_thumbs(&h)[0];
    let own = row_of(&h, a);
    drag_mask(
        &mut h,
        thumb.center(),
        pos2(own.left() + 60.0, own.center().y),
        Modifiers::NONE,
    );
    assert!(h.doc().layer(a).unwrap().mask().is_some());
    assert!(h.doc().layer(b).unwrap().mask().is_none());
    assert_eq!(h.doc().undo_count(), 0);
    // 一覧の外（キャンバスの上）で離す
    let outside = canvas_rect(&h).center();
    drag_mask(&mut h, thumb.center(), outside, Modifiers::NONE);
    assert!(h.doc().layer(a).unwrap().mask().is_some());
    assert!(h.doc().layer(b).unwrap().mask().is_none());
    assert_eq!(h.doc().undo_count(), 0);
}

#[test]
fn dropping_a_mask_on_a_locked_layer_is_refused_with_its_reason() {
    let (mut h, a, b) = two_layers();
    h.doc_mut().set_layer_locks(b, LayerLocks::ALL).unwrap();
    h.doc_mut().clear_history().unwrap();
    h.run();
    let thumb = mask_thumbs(&h)[0];
    let row = row_of(&h, b);
    drag_mask(
        &mut h,
        thumb.center(),
        pos2(row.left() + 40.0, row.center().y),
        Modifiers::NONE,
    );
    assert!(h.doc().layer(a).unwrap().mask().is_some());
    assert!(h.doc().layer(b).unwrap().mask().is_none());
    assert_eq!(h.doc().undo_count(), 0);
    assert_eq!(
        h.last_notice().as_ref().map(|n| n.kind),
        Some(Kind::Refusal)
    );
}

#[test]
fn copy_mask_and_paste_to_mask_work_from_the_layer_context_menu() {
    let (mut h, a, b) = two_layers();
    let name_a = h.doc().layer(a).unwrap().name().to_owned();
    let name_b = h.doc().layer(b).unwrap().name().to_owned();
    choose_in_layer_menu(&mut h, &name_a, "マスクをコピー");
    assert!(h.clip().mask.is_some());
    choose_in_layer_menu(&mut h, &name_b, "マスクに貼り付け");
    assert!(h.doc().layer(b).unwrap().mask().is_some());
    assert!(
        h.doc().layer(a).unwrap().mask().is_some(),
        "コピーなので元も残る"
    );
    assert_eq!(h.doc().undo_count(), 1);
}

#[test]
fn the_layer_context_menu_adds_a_black_mask() {
    let (mut h, _a, b) = two_layers();
    let name_b = h.doc().layer(b).unwrap().name().to_owned();
    choose_in_layer_menu(&mut h, &name_b, "黒のレイヤーマスクを追加");
    assert_eq!(
        h.doc()
            .layer(b)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(3, 3)
            .unwrap()
            .a,
        255
    );
    assert!(h.m2().edit_mask);
    assert_eq!(h.doc().undo_count(), 1);
}

#[test]
fn the_layer_properties_has_a_white_and_a_black_mask_button() {
    let mut h = app(1280.0, 800.0, 64);
    let id = h.selected_layer().unwrap();
    // 上の見出しを閉じて、マスクの見出しをウィンドウの中に入れる
    for key in ["layer", "locks", "layer-channels"] {
        h.ui_mut().sections.insert(key, false);
    }
    h.run();
    let node = h.get_by_label("白のマスク");
    node.click();
    h.run();
    assert_eq!(
        h.doc()
            .layer(id)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(3, 3)
            .unwrap()
            .a,
        0
    );
    h.act(Action::Undo);
    h.run();
    h.get_by_label("黒のマスク").click();
    h.run();
    assert_eq!(
        h.doc()
            .layer(id)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(3, 3)
            .unwrap()
            .a,
        255
    );
    assert_eq!(h.doc().undo_count(), 1, "黒も 1 回の取り消し");
}

/// 通しの確かめ: 片方に黒のマスクを足す（全部隠れる）→ 白く塗った所だけ見える → Alt を押しながらもう片方へ落とすと両方に同じマスクがある
/// → Ctrl+Z 1 回で元に戻る。
#[test]
fn a_black_mask_painted_white_is_copied_with_alt_and_one_ctrl_z_takes_it_back() {
    let mut h = app(1280.0, 800.0, 64);
    h.act(Action::NewLayer);
    h.run();
    let ids: Vec<LayerId> = h.doc().layers().iter().map(|l| l.id()).collect();
    let (a, b) = (ids[0], ids[1]);
    for y in 10..30 {
        for x in 10..30 {
            h.doc_mut()
                .set_pixel(a, x, y, yolu_app::engine::Rgba8::new(255, 0, 0, 255))
                .unwrap();
        }
    }
    h.doc_mut().clear_history().unwrap();
    let alpha = |h: &H, x, y| {
        h.doc()
            .composite_pixel(yolu_app::engine::Channel::Color, x, y)
            .unwrap()
            .a
    };
    assert_eq!(alpha(&h, 15, 15), 255);
    let name_a = h.doc().layer(a).unwrap().name().to_owned();
    choose_in_layer_menu(&mut h, &name_a, "黒のレイヤーマスクを追加");
    assert_eq!(alpha(&h, 15, 15), 0, "全部隠れる");
    // 白く塗った所（隠す量を戻した所）だけ見える
    let region = yolu_app::engine::SelectionMask::rectangle(h.doc(), 12, 12, 18, 18);
    h.doc_mut().fill_mask(a, 1.0, Some(&region), true).unwrap();
    h.run();
    assert_eq!(alpha(&h, 15, 15), 255);
    assert_eq!(alpha(&h, 11, 11), 0);
    // サムネイルを Alt を押しながらもう片方へ落とす
    let steps = h.doc().undo_count();
    let thumb = mask_thumbs(&h)[0];
    let row = row_of(&h, b);
    drag_mask(
        &mut h,
        thumb.center(),
        pos2(row.left() + 40.0, row.center().y),
        Modifiers::ALT,
    );
    let bytes = |h: &H, id| {
        h.doc()
            .layer(id)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .canvas_bytes()
            .unwrap()
    };
    assert_eq!(bytes(&h, a), bytes(&h, b), "両方に同じマスク");
    assert_eq!(h.doc().undo_count(), steps + 1);
    key(&h, egui::Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(h.doc().layer(b).unwrap().mask().is_none(), "1 回で元に戻る");
    assert_eq!(alpha(&h, 15, 15), 255);
    assert_eq!(h.doc().undo_count(), steps);
}

// ───────── スクリーンショット ─────────

/// マスクのサムネイルをつかんで、b の行の上まで動かした途中（離さない）。
fn hold_mask_over_the_other_row(h: &mut H, b: LayerId, alt: bool) {
    let thumb = mask_thumbs(h)[0];
    let row = row_of(h, b);
    press(h, thumb.center(), PointerButton::Primary);
    h.step();
    move_to(h, offset(thumb.center(), 0.0, -10.0));
    h.step();
    if alt {
        h.event(Event::ModifiersChanged(Modifiers::ALT));
    }
    move_to(h, pos2(row.left() + 60.0, row.center().y));
    h.step();
    h.step();
}

#[test]
fn snapshot_dragging_a_mask_over_a_layer_row() {
    let (mut h, _a, b) = two_layers();
    hold_mask_over_the_other_row(&mut h, b, false);
    let panel = super::shots::group(&h, yolu_app::Tab::Layers);
    common::shot::region(&mut h, "mask_drag_move", panel);
}

#[test]
fn snapshot_dragging_a_mask_with_alt_shows_the_copy_badge() {
    let (mut h, _a, b) = two_layers();
    hold_mask_over_the_other_row(&mut h, b, true);
    let panel = super::shots::group(&h, yolu_app::Tab::Layers);
    common::shot::region(&mut h, "mask_drag_copy", panel);
}

#[test]
fn snapshot_the_layer_menu_with_a_mask_in_both_languages() {
    for (lang, name) in [
        (yolu_app::lang::Lang::Ja, "mask_layer_menu"),
        (yolu_app::lang::Lang::En, "mask_layer_menu_en"),
    ] {
        let (mut h, a, _b) = two_layers();
        h.set_lang(lang);
        h.act(Action::M2(Edit::CopyMask(a)));
        h.run();
        let layer_name = h.doc().layer(a).unwrap().name().to_owned();
        open_layer_menu(&mut h, &layer_name);
        let rect = common::shot::popup(&h);
        common::shot::clean_region(&mut h, name, rect);
    }
}

#[test]
fn snapshot_the_mask_buttons_in_the_layer_properties_in_english() {
    let mut h = app(1280.0, 1000.0, 256);
    h.set_lang(yolu_app::lang::Lang::En);
    // 上の見出しを閉じて、マスクの見出しをスクリーンショットの中に入れる
    for key in ["layer", "locks", "layer-channels"] {
        h.ui_mut().sections.insert(key, false);
    }
    h.run();
    let panel = super::shots::group(&h, yolu_app::Tab::Properties);
    common::shot::region(&mut h, "mask_buttons_en", panel);
}
