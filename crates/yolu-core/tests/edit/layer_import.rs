//! ほかの文書（PSD を読んだ文書など）のレイヤーを、この文書のレイヤーセットへ入れる口: グループ・マスク・合成の方法がそのまま入って合成が
//! 同じになる、大きさの違いの断り、既存のレイヤーセットの一番上・新しいレイヤーセットへのどちらも 1 回の Undo、タイルの大きさの違い。

use yolu_core::Reason;
use yolu_core::{BlendMode, Channel, CoreError, Document, LayerId, LayerKind, Rgba8};

const RED: Rgba8 = Rgba8::new(220, 20, 30, 255);
const BLUE: Rgba8 = Rgba8::new(10, 40, 230, 255);
const GREEN: Rgba8 = Rgba8::new(30, 200, 60, 180);

fn fill(d: &mut Document, id: LayerId, c: Rgba8, x0: u32, y0: u32, x1: u32, y1: u32) {
    for y in y0..y1 {
        for x in x0..x1 {
            d.set_pixel(id, x, y, c).unwrap();
        }
    }
}

/// 読んだ PSD のような文書: 下地・グループ（乗算の子とマスク付きの子）・その上のクリッピングされたレイヤー。
fn source(tile: u32) -> Document {
    let mut s = Document::with_tile_size(24, 16, tile).unwrap();
    let base = s.add_layer("下地").unwrap();
    fill(&mut s, base, BLUE, 0, 0, 24, 16);
    let a = s.add_layer("乗算").unwrap();
    fill(&mut s, a, GREEN, 4, 0, 20, 16);
    s.set_layer_blend_mode(a, BlendMode::Multiply).unwrap();
    let b = s.add_layer("マスク").unwrap();
    fill(&mut s, b, RED, 0, 0, 12, 8);
    s.add_layer_mask(b).unwrap();
    s.group_layers(&[a, b], "グループ").unwrap();
    for y in 0..16 {
        s.set_mask_pixel(b, 3, y, 255).unwrap();
    }
    let clipped = s.add_layer("クリップ").unwrap();
    fill(&mut s, clipped, RED, 10, 10, 24, 16);
    s.set_layer_clipping(clipped, true).unwrap();
    s.clear_history().unwrap();
    s
}

fn target() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(24, 16, 8).unwrap();
    let a = d.add_layer("今の").unwrap();
    fill(&mut d, a, RED, 0, 0, 24, 16);
    d.clear_history().unwrap();
    (d, a)
}

/// 名前・種類・親の名前・合成の方法・マスクの有無・クリッピング（下から上）。
fn shape(
    d: &Document,
    ids: &[LayerId],
) -> Vec<(String, LayerKind, Option<String>, BlendMode, bool, bool)> {
    ids.iter()
        .map(|id| {
            let l = d.layer(*id).unwrap();
            (
                l.name().to_owned(),
                l.kind(),
                l.parent().map(|p| d.layer(p).unwrap().name().to_owned()),
                l.blend_mode(),
                l.mask().is_some(),
                l.clipping(),
            )
        })
        .collect()
}

#[test]
fn a_new_layer_set_gets_the_groups_masks_and_blends_and_composites_like_the_source() {
    let s = source(8);
    let (mut d, _) = target();
    let (set, ids) = d.insert_layers_as_layer_set(&s, "PSD").unwrap();
    assert_eq!(d.layer_sets().len(), 2);
    assert_eq!(d.layer_sets()[1].name, "PSD");
    assert_eq!(ids.len(), s.layers().len());
    let source_ids: Vec<LayerId> = s.layers().iter().map(|l| l.id()).collect();
    assert_eq!(
        shape(&d, &ids),
        shape(&s, &source_ids),
        "並び・親子・合成・マスク・クリッピングは元のまま"
    );
    for id in &ids {
        assert_eq!(d.layer_set_of(*id).unwrap(), set);
        assert!(!source_ids.contains(id), "ID は新しく振る");
    }
    assert_eq!(
        d.composite_set_channel(set, Channel::Color, d.bounds())
            .unwrap(),
        s.composite_channel(Channel::Color, s.bounds()).unwrap(),
        "入れたレイヤーセットの合成は元の文書の合成と同じ"
    );
    // 1 回の Undo でレイヤーセットごと消え、Redo で戻る
    d.undo().unwrap();
    assert_eq!(d.layer_sets().len(), 1);
    assert_eq!(d.layers().len(), 1);
    assert!(!d.can_undo());
    d.redo().unwrap();
    assert_eq!(d.layer_sets().len(), 2);
    assert_eq!(d.layers().len(), 1 + s.layers().len());
}

#[test]
fn layers_go_on_top_of_an_existing_set_in_one_undo_and_a_clip_with_nothing_below_stays_unclipped() {
    let mut s = Document::with_tile_size(24, 16, 8).unwrap();
    let low = s.add_layer("下が無いクリップ").unwrap();
    fill(&mut s, low, BLUE, 0, 0, 8, 8);
    s.set_layer_clipping(low, true).unwrap();
    let top = s.add_layer("上のクリップ").unwrap();
    fill(&mut s, top, GREEN, 0, 0, 8, 8);
    s.set_layer_clipping(top, true).unwrap();
    s.clear_history().unwrap();
    let (mut d, existing) = target();
    let set = d.layer_sets()[0].id;
    let before = d.composite_channel(Channel::Color, d.bounds()).unwrap();
    let ids = d.insert_layers_from(&s, set).unwrap();
    // 一番上の段の一番上（今のレイヤーの上）
    let order: Vec<LayerId> = d.layers().iter().map(|l| l.id()).collect();
    assert_eq!(order, vec![existing, ids[0], ids[1]]);
    // 元の一番下は、印があっても下地（何にもクリッピングされない）: 印を外して、今のレイヤーへクリッピングしない。上のクリップは元のとおり下へ
    assert!(!d.layer(ids[0]).unwrap().clipping());
    assert!(d.layer(ids[1]).unwrap().clipping());
    // 見え方: 今のレイヤーの上に、元のとおり（下地の上に、それにクリッピングした上のレイヤー）
    let mut e = Document::with_tile_size(24, 16, 8).unwrap();
    let red = e.add_layer("今の").unwrap();
    fill(&mut e, red, RED, 0, 0, 24, 16);
    let base = e.add_layer("下地").unwrap();
    fill(&mut e, base, BLUE, 0, 0, 8, 8);
    let clip = e.add_layer("クリップ").unwrap();
    fill(&mut e, clip, GREEN, 0, 0, 8, 8);
    e.set_layer_clipping(clip, true).unwrap();
    assert_eq!(
        d.composite_channel(Channel::Color, d.bounds()).unwrap(),
        e.composite_channel(Channel::Color, e.bounds()).unwrap()
    );
    d.undo().unwrap();
    assert_eq!(d.layers().len(), 1);
    assert_eq!(
        d.composite_channel(Channel::Color, d.bounds()).unwrap(),
        before
    );
    assert!(!d.can_undo(), "1 回の Undo");
}

#[test]
fn a_source_of_another_size_is_refused_and_changes_nothing() {
    let mut s = Document::with_tile_size(16, 16, 8).unwrap();
    s.add_layer("a").unwrap();
    let (mut d, _) = target();
    let set = d.layer_sets()[0].id;
    let revision = d.revision();
    for result in [
        d.insert_layers_from(&s, set).map(|_| ()),
        d.insert_layers_as_layer_set(&s, "PSD").map(|_| ()),
    ] {
        assert_eq!(
            result,
            Err(CoreError::InvalidArgument(
                Reason::LayersSizeDiffersFromCanvas
            ))
        );
    }
    assert_eq!(d.layers().len(), 1);
    assert_eq!(d.layer_sets().len(), 1);
    assert_eq!(d.revision(), revision);
    assert!(!d.can_undo());
}

#[test]
fn a_source_with_another_tile_size_is_retiled_with_the_same_pixels() {
    let s = source(16);
    let (mut d, _) = target();
    let (set, _) = d.insert_layers_as_layer_set(&s, "PSD").unwrap();
    assert_eq!(
        d.composite_set_channel(set, Channel::Color, d.bounds())
            .unwrap(),
        s.composite_channel(Channel::Color, s.bounds()).unwrap()
    );
}

#[test]
fn inserting_over_the_pixel_budget_is_refused_and_changes_nothing() {
    let s = source(8);
    let (mut d, _) = target();
    d.set_source_budget_bytes(d.allocated_bytes()).unwrap();
    let set = d.layer_sets()[0].id;
    assert_eq!(
        d.insert_layers_from(&s, set),
        Err(CoreError::SourceBudgetExceeded)
    );
    assert_eq!(d.layers().len(), 1);
    assert!(!d.can_undo());
}
