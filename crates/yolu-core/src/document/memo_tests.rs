//! 描いている間の下の覚え（`composite::Memo`）が、描く層の外の変化で捨てられること。
//! 描いている間は層の属性・並び・画素の編集はどれも断られるので、外の変化の道は変化の記録（`Journal::mark`）への直の印で作る
//! （入力のマップを替える・Live Link のように、断られずに合成を変える道の代わり）。

use super::*;
use crate::glam::DVec2;

fn painted() -> (Document, LayerId, LayerId) {
    let mut d = Document::with_tile_size(48, 32, 16).unwrap();
    let below = d.add_layer("below").unwrap();
    let above = d.add_layer("above").unwrap();
    for y in 0..32 {
        for x in 0..48 {
            d.set_pixel(below, x, y, Rgba8::new(40, 90, 200, 255)).unwrap();
        }
    }
    d.set_pixel(above, 3, 3, Rgba8::new(255, 0, 0, 255)).unwrap();
    d.clear_history().unwrap();
    (d, below, above)
}

fn dab(d: &mut Document, layer: LayerId) -> Stroke {
    let brush = BrushSettings {
        radius: 4.0,
        hardness: 1.0,
        color: Rgba8::new(255, 255, 0, 255),
        pressure_size: false,
        pressure_opacity: false,
        pressure_flow: false,
        ..BrushSettings::default()
    };
    let mut s = d.begin_stroke(layer, &brush).unwrap();
    s.add_point(d, 20.0, 12.0, 1.0, DVec2::ZERO).unwrap();
    s
}

#[test]
fn the_stroke_own_writes_keep_the_memory_but_any_other_mark_drops_it() {
    let (mut d, below, _above) = painted();
    let s = dab(&mut d, below);
    let _ = d.composite_channel(Channel::Color, d.bounds()).unwrap();
    let kept = d.composite_memo_stats();
    assert!(kept.tiles > 0, "手を付けたタイルを覚えた");
    // 描き続ける（自分の面への書き込みは、覚えを捨てない）
    let mut s = s;
    s.add_point(&mut d, 22.0, 13.0, 1.0, DVec2::ZERO).unwrap();
    let _ = d.composite_channel(Channel::Color, d.bounds()).unwrap();
    let after = d.composite_memo_stats();
    assert!(after.hits > kept.hits, "覚えから続けた");
    assert!(after.tiles >= kept.tiles);
    // ほかの変化（外の印）で、次の合成は覚えを作り直す
    let before = d.composite_memo_stats().built;
    d.journal.mark(Channel::Color, TileCoord::new(1, 0));
    let _ = d.composite_channel(Channel::Color, d.bounds()).unwrap();
    assert!(d.composite_memo_stats().built > before, "外の印のあとは作り直す");
    d.end_stroke(s).unwrap();
    assert_eq!(d.composite_memo_stats().tiles, 0, "確定したら手放す");
}

#[test]
fn cancelling_the_stroke_drops_the_memory_and_its_bytes_stay_in_the_budget() {
    let (mut d, below, _) = painted();
    d.set_stroke_budget_bytes(1 << 20).unwrap();
    let s = dab(&mut d, below);
    let _ = d.composite_channel(Channel::Color, d.bounds()).unwrap();
    let st = d.composite_memo_stats();
    assert!(st.tiles > 0);
    // 覚えのバイト数は、ストロークの予算から巻き戻し用の写しを引いた残りに収まる
    let rollback = d.active_stroke_stats().unwrap().rollback_bytes;
    assert!(st.bytes + rollback <= 1 << 20);
    d.cancel_stroke(s);
    assert_eq!(d.composite_memo_stats().tiles, 0);
}
