//! 保存した所の目印（`Document::mark_saved`・`is_clean`）: Undo・Redo で保存した所へ戻れば同じ中身、戻れなければ違う中身。

use yolu_core::glam::DVec2;
use yolu_core::{BrushSettings, Channel, Document, LayerId, Rgba8, TileCoord};

fn painted(d: &mut Document, layer: LayerId, color: Rgba8) -> bool {
    let brush = BrushSettings {
        radius: 4.0,
        color,
        ..BrushSettings::default()
    };
    let mut s = d.begin_stroke(layer, &brush).unwrap();
    s.add_point(d, 4.0, 4.0, 1.0, DVec2::ZERO).unwrap();
    s.add_point(d, 12.0, 12.0, 1.0, DVec2::ZERO).unwrap();
    d.end_stroke(s).unwrap().changed
}

fn saved_with_layer() -> (Document, LayerId) {
    let mut d = Document::new(32, 32).unwrap();
    let layer = d.add_layer("a").unwrap();
    d.mark_saved();
    (d, layer)
}

#[test]
fn a_new_document_is_clean_and_an_edit_makes_it_unclean_until_undone() {
    let mut d = Document::new(16, 16).unwrap();
    assert!(d.is_clean());
    d.add_layer("a").unwrap();
    assert!(!d.is_clean());
    d.undo().unwrap();
    assert!(d.is_clean(), "保存した所（空の履歴）へ Undo で戻った");
    d.redo().unwrap();
    assert!(!d.is_clean());
}

#[test]
fn a_stroke_that_changes_pixels_is_unclean_and_undo_to_the_saved_step_is_clean() {
    let (mut d, layer) = saved_with_layer();
    assert!(d.is_clean());
    assert!(painted(&mut d, layer, Rgba8::new(200, 30, 30, 255)));
    assert!(!d.is_clean());
    d.undo().unwrap();
    assert!(d.is_clean());
    d.redo().unwrap();
    assert!(!d.is_clean());
}

#[test]
fn a_stroke_that_changes_no_pixels_stays_clean() {
    let (mut d, layer) = saved_with_layer();
    assert!(!painted(&mut d, layer, Rgba8::new(200, 30, 30, 0)));
    assert!(d.is_clean());
}

#[test]
fn a_cancelled_stroke_is_unclean_while_drawing_and_clean_after() {
    let (mut d, layer) = saved_with_layer();
    let brush = BrushSettings {
        radius: 4.0,
        color: Rgba8::new(10, 200, 30, 255),
        ..BrushSettings::default()
    };
    let mut s = d.begin_stroke(layer, &brush).unwrap();
    s.add_point(&mut d, 6.0, 6.0, 1.0, DVec2::ZERO).unwrap();
    assert!(!d.is_clean(), "描いている間");
    d.cancel_stroke(s);
    assert!(d.is_clean());
    assert_eq!(d.undo_count(), 1);
}

#[test]
fn a_new_step_after_undo_forgets_a_saved_step_that_was_redoable() {
    let (mut d, layer) = saved_with_layer();
    painted(&mut d, layer, Rgba8::new(200, 30, 30, 255));
    d.mark_saved();
    assert!(d.is_clean());
    d.undo().unwrap();
    assert!(!d.is_clean());
    d.add_layer("b").unwrap();
    // 保存した所（Redo の側にあった）はもう戻れない: 同じ段数に戻っても同じ中身ではない
    assert!(!d.is_clean());
    d.undo().unwrap();
    assert!(!d.is_clean());
    d.redo().unwrap();
    assert!(!d.is_clean());
}

#[test]
fn changes_outside_the_history_keep_the_document_unclean_until_saved_again() {
    let mut d = Document::with_tile_size(32, 32, 16).unwrap();
    let layer = d.add_layer("a").unwrap();
    d.mark_saved();
    let tile = vec![9u8; (d.tile_size() * d.tile_size() * 4) as usize];
    d.import_tile(layer, Channel::Color, TileCoord::new(0, 0), &tile)
        .unwrap();
    assert!(!d.is_clean());
    d.add_layer("b").unwrap();
    d.undo().unwrap();
    assert!(!d.is_clean(), "履歴を通らない変更は Undo で戻らない");
    d.mark_saved();
    assert!(d.is_clean());
}

#[test]
fn merging_into_the_saved_step_makes_it_unclean_and_unreachable() {
    let (mut d, layer) = saved_with_layer();
    d.set_layer_opacity(layer, 0.7, true).unwrap();
    d.mark_saved();
    d.set_layer_opacity(layer, 0.4, true).unwrap();
    assert!(!d.is_clean());
    d.undo().unwrap();
    assert!(
        !d.is_clean(),
        "まとめを足したので、保存した所の中身はもう無い"
    );
    d.redo().unwrap();
    assert!(!d.is_clean());
}

#[test]
fn merging_after_the_saved_step_still_allows_undo_back_to_it() {
    let (mut d, layer) = saved_with_layer();
    d.set_layer_opacity(layer, 0.7, false).unwrap();
    d.mark_saved();
    d.end_coalescing();
    d.set_layer_opacity(layer, 0.5, true).unwrap();
    d.set_layer_opacity(layer, 0.3, true).unwrap();
    assert!(!d.is_clean());
    d.undo().unwrap();
    assert!(d.is_clean());
}

#[test]
fn trimming_old_steps_keeps_the_saved_step_reachable_until_it_is_dropped() {
    let mut d = Document::new(16, 16).unwrap();
    let layer = d.add_layer("a").unwrap();
    d.mark_saved(); // 位置 1
    d.set_layer_opacity(layer, 0.5, false).unwrap();
    let step = d.history_bytes() - 128;
    // 最も古い段（レイヤーの追加）だけを落とす: 保存した所は履歴の底になって、まだ戻れる
    d.set_undo_budget_bytes(step).unwrap();
    assert_eq!(d.undo_count(), 1);
    assert!(!d.is_clean());
    d.undo().unwrap();
    assert!(d.is_clean());
    d.redo().unwrap();
    // 予算の中に次の段が入らず、保存した所の底まで落ちると、戻れない
    d.set_layer_opacity(layer, 0.4, false).unwrap();
    d.undo().unwrap();
    assert!(!d.is_clean(), "保存した所の段は予算で落ちた");
}

#[test]
fn clearing_the_history_keeps_a_saved_document_clean_and_an_unsaved_one_unclean() {
    let (mut d, _) = saved_with_layer();
    d.clear_history().unwrap();
    assert!(d.is_clean());
    d.add_layer("b").unwrap();
    d.clear_history().unwrap();
    assert!(!d.is_clean());
}

#[test]
fn a_batch_is_one_step_and_undo_returns_to_the_saved_step() {
    let (mut d, layer) = saved_with_layer();
    d.batch(|d| {
        d.set_layer_opacity(layer, 0.5, false)?;
        d.add_layer("b")?;
        Ok(())
    })
    .unwrap();
    assert!(!d.is_clean());
    d.undo().unwrap();
    assert!(d.is_clean(), "まとめは 1 段で、Undo 1 回で保存した所へ戻る");
    d.redo().unwrap();
    assert!(!d.is_clean());
}

#[test]
fn a_batch_after_undo_forgets_the_saved_step_on_the_redo_side() {
    let (mut d, layer) = saved_with_layer();
    d.set_layer_opacity(layer, 0.5, false).unwrap();
    d.mark_saved();
    d.undo().unwrap();
    d.batch(|d| {
        d.set_layer_opacity(layer, 0.2, false)?;
        d.add_layer("b")?;
        Ok(())
    })
    .unwrap();
    assert!(!d.is_clean());
    d.undo().unwrap();
    assert!(!d.is_clean(), "保存した所は Redo の側で、まとめで捨てた");
    d.redo().unwrap();
    assert!(!d.is_clean());
}

#[test]
fn a_rolled_back_batch_stays_unclean_until_saved_again() {
    let (mut d, _) = saved_with_layer();
    let failed = d.batch(|d| {
        d.add_layer("b")?;
        d.add_layer("c")?;
        Err::<(), _>(yolu_core::CoreError::NoActiveStroke)
    });
    assert!(failed.is_err());
    assert_eq!(d.undo_count(), 1, "履歴は始める前と同じ");
    assert!(
        !d.is_clean(),
        "巻き戻しは履歴を通らない変更で、保存し直すまで違う中身"
    );
    d.add_layer("d").unwrap();
    d.undo().unwrap();
    assert!(!d.is_clean());
    d.mark_saved();
    assert!(d.is_clean());
}

#[test]
fn discarding_the_last_step_is_unclean_even_back_at_the_saved_position() {
    let (mut d, _) = saved_with_layer();
    d.add_layer("b").unwrap();
    d.mark_saved();
    assert!(d.discard_last_step().unwrap());
    assert!(!d.is_clean());
    // 同じ段数まで積み直しても、保存した段とは別の物
    d.add_layer("c").unwrap();
    assert!(!d.is_clean());
    d.mark_saved();
    assert!(d.is_clean());
}

#[test]
fn cancelling_a_coalesced_step_is_unclean_even_back_at_the_saved_position() {
    let (mut d, layer) = saved_with_layer();
    d.set_layer_opacity(layer, 0.7, true).unwrap();
    d.mark_saved();
    assert!(d.is_clean());
    assert!(d.cancel_coalescing().unwrap());
    assert!(!d.is_clean());
    d.set_layer_opacity(layer, 0.3, false).unwrap();
    assert!(!d.is_clean(), "同じ段数でも、保存した段とは別の物");
    d.undo().unwrap();
    assert!(!d.is_clean());
}
