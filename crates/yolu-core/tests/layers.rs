//! 層の種類（グループ・マスク・塗りつぶし・調整・クリッピング）の振る舞い。C# の GroupTests・MaskTests・AdjustmentTests・
//! ClippingTests の確かめを移したもの（期待値は C# の試験と同じ）。合成のバイトそのものは golden.rs が C# と照らす。

use std::collections::HashSet;

use yolu_core::{
    AdjustmentSettings, BlendMode, BrushSettings, Channel, CoreError, Document, LayerId, LayerKind,
    Rect, Rgba8, RowOrder, TileCoord,
};

fn solid(d: &mut Document, name: &str, c: Rgba8, x0: u32, y0: u32, x1: u32, y1: u32) -> LayerId {
    let id = d.add_layer(name).unwrap();
    for y in y0..y1 {
        for x in x0..x1 {
            d.set_pixel(id, x, y, c).unwrap();
        }
    }
    id
}
fn full(d: &mut Document, name: &str, c: Rgba8) -> LayerId {
    let (w, h) = (d.width(), d.height());
    solid(d, name, c, 0, 0, w, h)
}
fn shape(d: &Document) -> String {
    d.layers()
        .iter()
        .map(|l| match l.parent() {
            None => l.name().to_string(),
            Some(p) => format!("{}<{}", l.name(), d.layer(p).unwrap().name()),
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn at(d: &Document, x: u32, y: u32) -> Rgba8 {
    d.composite_pixel(Channel::Color, x, y).unwrap()
}
/// タイルの経路（領域の合成）が画素ごとの参照の式と同じか、全チャンネルで。
fn assert_tiles_match_reference(d: &Document) {
    for ch in d.channels() {
        let all = d.composite_channel(ch, d.bounds()).unwrap();
        for y in 0..d.height() {
            for x in 0..d.width() {
                let i = ((y * d.width() + x) * 4) as usize;
                assert_eq!(
                    &all[i..i + 4],
                    &d.composite_pixel(ch, x, y).unwrap().to_array(),
                    "{ch:?} ({x},{y})"
                );
            }
        }
    }
}
fn hard(erase: bool) -> BrushSettings {
    BrushSettings {
        radius: 1.0,
        hardness: 1.0,
        color: Rgba8::new(255, 0, 255, 255),
        pressure_size: false,
        pressure_opacity: false,
        erase,
        ..BrushSettings::default()
    }
}
fn mask_pixel(d: &mut Document, layer: LayerId, x: i64, y: i64, erase: bool) {
    let mut s = d.begin_mask_stroke(layer, &hard(erase)).unwrap();
    s.apply_pixel(d, x, y, 1.0, 1.0).unwrap();
    d.end_stroke(s).unwrap();
}
fn changed(d: &Document, ch: Channel, since: u64) -> HashSet<TileCoord> {
    d.changed_tiles(ch, since).unwrap().into_iter().collect()
}

// ───────── グループ ─────────

#[test]
fn groups_keep_their_contents_directly_below_and_every_edit_is_one_undo_step() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let a = d.add_layer("A").unwrap();
    let _b = d.add_layer("B").unwrap();
    let c = d.add_layer("C").unwrap();
    let g = d.add_group("G", None).unwrap();
    d.clear_history().unwrap();
    assert_eq!(d.layer(g).unwrap().blend_mode(), BlendMode::PassThrough);
    assert_eq!(d.layer(g).unwrap().kind(), LayerKind::Group);
    d.move_layer_to(a, Some(g), 0).unwrap();
    assert_eq!(shape(&d), "B C A<G G");
    d.move_layer_to(c, Some(g), 1).unwrap();
    assert_eq!(shape(&d), "B A<G C<G G");
    let names: Vec<String> = d
        .children_of(Some(g))
        .unwrap()
        .iter()
        .map(|i| d.layer(*i).unwrap().name().to_string())
        .collect();
    assert_eq!(names, ["A", "C"]);
    assert_eq!(d.depth_of(c).unwrap(), 1);
    d.move_layer(c, 0).unwrap(); // グループの中で一番下へ
    assert_eq!(shape(&d), "B C<G A<G G");
    d.move_layer(g, 0).unwrap(); // グループを中身ごと一番下へ
    assert_eq!(shape(&d), "C<G A<G G B");
    assert!(
        d.move_layer_to(g, Some(g), 0).is_err(),
        "自分の中へは入れない"
    );
    d.validate_structure().unwrap();
    let steps = d.undo_count();
    assert_eq!(steps, 4);
    for _ in 0..steps {
        d.undo().unwrap();
    }
    assert_eq!(shape(&d), "A B C G");
    for _ in 0..steps {
        d.redo().unwrap();
    }
    assert_eq!(shape(&d), "C<G A<G G B");
}

#[test]
fn new_layers_can_be_placed_directly_above_another_layer_in_its_group() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let a = d.add_layer("A").unwrap();
    let b = d.add_layer("B").unwrap();
    let g = d.group_layers(&[a, b], "G").unwrap();
    d.add_layer("Top").unwrap();
    d.clear_history().unwrap();
    d.add_layer_above("C", Some(a)).unwrap();
    assert_eq!(shape(&d), "A<G C<G B<G G Top");
    d.add_fill_layer("F", &[], Some(g)).unwrap();
    assert_eq!(
        shape(&d),
        "A<G C<G B<G G F Top",
        "グループの上 = 中身の上、兄弟として"
    );
    let adj = d
        .add_adjustment_layer("Adj", AdjustmentSettings::invert(), None, Some(b))
        .unwrap();
    d.add_group("Inner", Some(adj)).unwrap();
    assert_eq!(shape(&d), "A<G C<G B<G Adj<G Inner<G G F Top");
    d.validate_structure().unwrap();
    assert_eq!(d.undo_count(), 4);
    for _ in 0..4 {
        d.undo().unwrap();
    }
    assert_eq!(shape(&d), "A<G B<G G Top");
    assert_eq!(
        d.add_layer_above("X", Some(LayerId(7))),
        Err(CoreError::LayerNotFound)
    );
}

#[test]
fn group_ungroup_and_delete_with_contents() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let a = d.add_layer("A").unwrap();
    let b = d.add_layer("B").unwrap();
    let c = d.add_layer("C").unwrap();
    let top = d.add_layer("Top").unwrap();
    d.clear_history().unwrap();
    let g = d.group_layers(&[c, a], "G").unwrap();
    assert_eq!(
        shape(&d),
        "B A<G C<G G Top",
        "並びを保ち、一番上の対象の所に置く"
    );
    let outer = d.group_layers(&[g, b], "Outer").unwrap();
    assert_eq!(shape(&d), "B<Outer A<G C<G G<Outer Outer Top");
    assert_eq!(d.depth_of(a).unwrap(), 2);
    assert!(d.group_layers(&[a, top], "X").is_err(), "兄弟だけ");
    d.ungroup(outer).unwrap();
    assert_eq!(shape(&d), "B A<G C<G G Top");
    d.remove_layer(g).unwrap();
    assert_eq!(shape(&d), "B Top", "グループを消すと中身も");
    d.undo().unwrap();
    assert_eq!(shape(&d), "B A<G C<G G Top", "1 回の Undo で全部戻る");
    d.undo().unwrap();
    d.undo().unwrap();
    d.undo().unwrap();
    assert_eq!(shape(&d), "A B C Top");
    assert!(d.ungroup(a).is_err());
    // 外したグループの層（ID・設定）は Redo で同じものが戻る
    d.redo().unwrap();
    assert_eq!(d.layer(g).unwrap().name(), "G");
}

#[test]
fn a_pass_through_group_composites_like_no_group_and_its_opacity_fades_it() {
    let make = |group: bool| {
        let mut d = Document::with_tile_size(8, 8, 8).unwrap();
        full(&mut d, "Base", Rgba8::new(200, 100, 50, 255));
        let m = solid(&mut d, "Mul", Rgba8::new(128, 255, 64, 200), 2, 2, 7, 7);
        d.set_layer_blend_mode(m, BlendMode::Multiply).unwrap();
        let g = group.then(|| d.group_layers(&[m], "G").unwrap());
        (d, g)
    };
    let (flat, _) = make(false);
    let (mut grouped, g) = make(true);
    let g = g.unwrap();
    assert_eq!(
        grouped.composite(grouped.bounds()).unwrap(),
        flat.composite(flat.bounds()).unwrap()
    );
    grouped.set_layer_opacity(g, 0.5, false).unwrap();
    let below = Rgba8::new(200, 100, 50, 255);
    let full_px = at(&flat, 4, 4);
    let faded = at(&grouped, 4, 4);
    assert!((faded.r as f64 - ((below.r as f64 + full_px.r as f64) / 2.0).round()).abs() <= 1.0);
    assert_eq!(at(&grouped, 0, 0), below, "中身の外は変わらない");
    assert_tiles_match_reference(&grouped);
}

#[test]
fn an_isolated_group_blends_its_contents_only_with_each_other() {
    let mut d = Document::with_tile_size(8, 8, 8).unwrap();
    full(&mut d, "Base", Rgba8::new(200, 100, 50, 255));
    let mul = full(&mut d, "Mul", Rgba8::new(128, 255, 64, 255));
    d.set_layer_blend_mode(mul, BlendMode::Multiply).unwrap();
    let g = d.group_layers(&[mul], "G").unwrap();
    let pass = at(&d, 1, 1);
    assert_eq!(pass, Rgba8::new(100, 100, 13, 255));
    d.set_layer_blend_mode(g, BlendMode::Normal).unwrap();
    assert_eq!(
        at(&d, 1, 1),
        Rgba8::new(128, 255, 64, 255),
        "透明の上の乗算は中身そのもの"
    );
    d.set_layer_blend_mode(g, BlendMode::Multiply).unwrap();
    assert_eq!(at(&d, 1, 1), pass, "グループのモードで結果を重ねる");
    assert_tiles_match_reference(&d);
}

#[test]
fn adjustments_inside_a_group_reach_out_only_when_it_passes_through() {
    let mut d = Document::with_tile_size(8, 8, 8).unwrap();
    full(&mut d, "Base", Rgba8::new(200, 100, 50, 255));
    let inside = solid(&mut d, "Inside", Rgba8::new(10, 20, 30, 255), 0, 0, 4, 8);
    let inv = d
        .add_adjustment_layer("Invert", AdjustmentSettings::invert(), None, None)
        .unwrap();
    let g = d.group_layers(&[inside, inv], "G").unwrap();
    assert_eq!(
        at(&d, 6, 1),
        Rgba8::new(55, 155, 205, 255),
        "通過: 下の層も反転"
    );
    assert_eq!(at(&d, 1, 1), Rgba8::new(245, 235, 225, 255));
    d.set_layer_blend_mode(g, BlendMode::Normal).unwrap();
    assert_eq!(
        at(&d, 6, 1),
        Rgba8::new(200, 100, 50, 255),
        "分離: 中身だけ"
    );
    assert_eq!(at(&d, 1, 1), Rgba8::new(245, 235, 225, 255));
    assert_tiles_match_reference(&d);
}

#[test]
fn clipping_stays_inside_the_group_and_groups_can_clip_and_be_clipped() {
    let mut d = Document::with_tile_size(8, 8, 8).unwrap();
    full(&mut d, "Below", Rgba8::new(0, 0, 255, 255));
    let first = solid(&mut d, "First", Rgba8::new(255, 0, 0, 255), 0, 0, 4, 8);
    d.group_layers(&[first], "G").unwrap();
    d.set_layer_clipping(first, true).unwrap();
    let fi = d.layer_index(first).unwrap();
    assert!(
        !d.is_effectively_clipped(fi),
        "グループの一番下は、グループの下に層があってもクリッピングされない"
    );
    assert_eq!(at(&d, 1, 1), Rgba8::new(255, 0, 0, 255));
    d.set_layer_clipping(first, false).unwrap();
    let clip = full(&mut d, "Clip", Rgba8::new(0, 255, 0, 255));
    d.set_layer_clipping(clip, true).unwrap();
    assert_eq!(
        at(&d, 1, 1),
        Rgba8::new(0, 255, 0, 255),
        "グループの画素の内側"
    );
    assert_eq!(at(&d, 6, 1), Rgba8::new(0, 0, 255, 255), "外には出ない");
    assert_tiles_match_reference(&d);
    // グループ自身を下の層へクリッピングする
    let mut h = Document::with_tile_size(8, 8, 8).unwrap();
    solid(&mut h, "Base", Rgba8::new(0, 0, 255, 255), 0, 0, 8, 4);
    let inner = full(&mut h, "Inner", Rgba8::new(255, 255, 0, 255));
    let cg = h.group_layers(&[inner], "Clipped group").unwrap();
    h.set_layer_clipping(cg, true).unwrap();
    assert_eq!(at(&h, 1, 1), Rgba8::new(255, 255, 0, 255));
    assert_eq!(
        at(&h, 1, 6).a,
        0,
        "クリッピングされたグループは下地の中だけ"
    );
    assert_tiles_match_reference(&h);
}

#[test]
fn hidden_groups_and_group_masks_hide_everything_inside() {
    let mut d = Document::with_tile_size(8, 8, 8).unwrap();
    full(&mut d, "Base", Rgba8::new(0, 0, 255, 255));
    let child = full(&mut d, "Child", Rgba8::new(255, 0, 0, 255));
    let g = d.group_layers(&[child], "G").unwrap();
    d.set_layer_visible(g, false).unwrap();
    assert_eq!(at(&d, 1, 1), Rgba8::new(0, 0, 255, 255));
    d.set_layer_visible(g, true).unwrap();
    d.add_layer_mask(g).unwrap();
    d.set_mask_pixel(g, 1, 1, 255).unwrap();
    assert_eq!(
        at(&d, 1, 1),
        Rgba8::new(0, 0, 255, 255),
        "グループのマスクが中身を隠す"
    );
    assert_eq!(at(&d, 2, 2), Rgba8::new(255, 0, 0, 255));
    assert!(
        d.begin_stroke(g, &BrushSettings::default()).is_err(),
        "グループは画素を持たない"
    );
    assert!(d.set_pixel(g, 0, 0, Rgba8::new(1, 2, 3, 4)).is_err());
    assert!(
        d.set_layer_blend_mode(child, BlendMode::PassThrough)
            .is_err(),
        "通過はグループだけ"
    );
    assert_tiles_match_reference(&d);
}

#[test]
fn changing_a_group_invalidates_the_tiles_of_its_contents() {
    let mut d = Document::with_tile_size(32, 32, 8).unwrap();
    solid(&mut d, "Base", Rgba8::new(0, 0, 255, 255), 0, 0, 8, 8);
    let child = solid(&mut d, "Child", Rgba8::new(255, 0, 0, 255), 16, 16, 24, 24);
    let g = d.group_layers(&[child], "G").unwrap();
    let since = d.change_serial();
    d.set_layer_opacity(g, 0.5, false).unwrap();
    let c = changed(&d, Channel::Color, since);
    assert!(c.contains(&TileCoord::new(2, 2)), "中身のタイル");
    assert!(
        !c.contains(&TileCoord::new(0, 0)),
        "下の層のタイルは変わらない"
    );
    // 変わったタイルだけを描き直した画面が、全面の合成と同じ（表示の差分の更新）
    let mut shown = d.composite(d.bounds()).unwrap();
    let mut serial = d.change_serial();
    type Edit = Box<dyn Fn(&mut Document)>;
    let edits: Vec<Edit> = vec![
        Box::new(move |d| d.set_layer_visible(g, false).unwrap()),
        Box::new(move |d| d.set_layer_visible(g, true).unwrap()),
        Box::new(move |d| d.set_layer_blend_mode(g, BlendMode::Screen).unwrap()),
        Box::new(move |d| d.ungroup(g).unwrap()),
        Box::new(|d| {
            d.undo().unwrap();
        }),
        Box::new(move |d| {
            d.add_layer_mask(g).unwrap();
        }),
        Box::new(move |d| {
            d.set_mask_pixel(g, 17, 17, 200).unwrap();
        }),
        Box::new(move |d| d.remove_layer(g).unwrap()),
        Box::new(|d| {
            d.undo().unwrap();
        }),
    ];
    for edit in edits {
        edit(&mut d);
        for coord in d.changed_tiles(Channel::Color, serial).unwrap() {
            let r = d.tile_rect(coord).unwrap();
            let tile = d.composite(r).unwrap();
            for row in 0..r.height {
                let dst = (((r.y + row) * 32 + r.x) * 4) as usize;
                shown[dst..dst + (r.width * 4) as usize].copy_from_slice(
                    &tile[(row * r.width * 4) as usize..((row + 1) * r.width * 4) as usize],
                );
            }
        }
        serial = d.change_serial();
        assert_eq!(shown, d.composite(d.bounds()).unwrap());
    }
}

#[test]
fn broken_nesting_is_refused_for_loaders() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let a = d.add_layer("A").unwrap();
    d.add_layer("B").unwrap();
    let g = d.add_group("G", None).unwrap();
    d.add_layer("Top").unwrap();
    // 正しい: A と B が G の中
    d.set_structure_for_load(&[Some(g), Some(g), None, None])
        .unwrap();
    assert_eq!(shape(&d), "A<G B<G G Top");
    assert_eq!(d.undo_count(), 0, "読み込みは履歴を消す");
    let before = shape(&d);
    for bad in [
        vec![Some(g), Some(g), None, Some(g)],     // 中身がグループの上
        vec![Some(g), None, None, None],           // A と G の間に外の B（続いていない）
        vec![Some(a), None, None, None],           // グループでない層の中
        vec![Some(LayerId(99)), None, None, None], // 無いグループ
        vec![Some(g), Some(g), Some(g), None],     // 自分の中（輪）
        vec![None, None],                          // 数が違う
    ] {
        assert!(d.set_structure_for_load(&bad).is_err(), "{bad:?}");
        assert_eq!(shape(&d), before, "断ったら何も変えない");
    }
}

// ───────── マスク ─────────

const RED: Rgba8 = Rgba8::new(220, 20, 30, 255);

fn full_red_layer() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let l = full(&mut d, "Paint", RED);
    d.clear_history().unwrap();
    (d, l)
}

#[test]
fn mask_hides_and_inverts_according_to_density_and_enabled() {
    let (mut d, l) = full_red_layer();
    d.add_layer_mask(l).unwrap();
    assert_eq!(at(&d, 3, 3).a, 255, "新しいマスクは全部見せる");
    mask_pixel(&mut d, l, 3, 3, false);
    assert_eq!(at(&d, 3, 3).a, 0);
    assert_eq!(at(&d, 10, 10), RED, "見える画素の色はそのまま");
    d.set_layer_mask_density(l, 0.5, false).unwrap();
    assert_eq!(at(&d, 3, 3).a, 128);
    d.set_layer_mask_inverted(l, true).unwrap();
    assert_eq!(at(&d, 3, 3).a, 255);
    assert_eq!(at(&d, 10, 10).a, 128);
    d.set_layer_mask_enabled(l, false).unwrap();
    assert_eq!(at(&d, 3, 3).a, 255);
    assert_eq!(at(&d, 10, 10).a, 255);
    assert_tiles_match_reference(&d);
}

#[test]
fn mask_strokes_change_only_the_mask_and_undo_exactly() {
    let (mut d, l) = full_red_layer();
    d.add_layer_mask(l).unwrap();
    d.clear_history().unwrap();
    let color_before = d
        .layer(l)
        .unwrap()
        .surface(Channel::Color)
        .unwrap()
        .to_canvas_bytes();
    mask_pixel(&mut d, l, 5, 5, false);
    let mask = d.layer(l).unwrap().mask().unwrap();
    assert_eq!(mask.surface().tile_count(), 1);
    assert_eq!(
        mask.surface().pixel(5, 5).unwrap(),
        Rgba8::new(0, 0, 0, 255),
        "ブラシの色は使わず、隠す量だけ"
    );
    assert_eq!(
        d.layer(l)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .to_canvas_bytes(),
        color_before
    );
    assert!(d.undo().unwrap());
    assert_eq!(
        d.layer(l).unwrap().mask().unwrap().surface().tile_count(),
        0
    );
    assert_eq!(at(&d, 5, 5).a, 255);
    assert!(d.redo().unwrap());
    assert_eq!(at(&d, 5, 5).a, 0);
    mask_pixel(&mut d, l, 5, 5, true);
    assert_eq!(at(&d, 5, 5).a, 255, "マスクを消すと見せる");
    let mut s = d.begin_mask_stroke(l, &hard(false)).unwrap();
    s.apply_pixel(&mut d, 9, 9, 1.0, 1.0).unwrap();
    d.cancel_stroke(s);
    assert_eq!(at(&d, 9, 9).a, 255, "取り消したストロークは何も残さない");
}

#[test]
fn adding_and_removing_a_mask_is_undoable() {
    let (mut d, l) = full_red_layer();
    d.add_layer_mask(l).unwrap();
    mask_pixel(&mut d, l, 2, 2, false);
    d.set_layer_mask_density(l, 0.75, false).unwrap();
    d.remove_layer_mask(l).unwrap();
    assert!(d.layer(l).unwrap().mask().is_none());
    assert_eq!(at(&d, 2, 2).a, 255);
    d.undo().unwrap();
    let m = d.layer(l).unwrap().mask().unwrap();
    assert_eq!(m.density(), 0.75);
    assert_eq!(at(&d, 2, 2).a, 64);
    assert!(d.add_layer_mask(l).is_err(), "マスクは 1 つ");
    d.undo().unwrap();
    d.undo().unwrap();
    d.undo().unwrap();
    assert!(d.layer(l).unwrap().mask().is_none());
    assert!(d.begin_mask_stroke(l, &hard(false)).is_err());
}

#[test]
fn mask_pixels_count_against_the_source_budget() {
    let (mut d, l) = full_red_layer();
    d.add_layer_mask(l).unwrap();
    let before = d.allocated_bytes();
    mask_pixel(&mut d, l, 1, 1, false);
    assert_eq!(d.allocated_bytes(), before + 8 * 8 * 4);
    d.set_source_budget_bytes(d.allocated_bytes()).unwrap(); // もう 1 タイルも増やせない
    let mut s = d.begin_mask_stroke(l, &hard(false)).unwrap();
    assert_eq!(
        s.apply_pixel(&mut d, 12, 12, 1.0, 1.0),
        Err(CoreError::SourceBudgetExceeded)
    );
    assert!(!d.has_active_stroke(), "断ったストロークは取り消してある");
    assert_eq!(at(&d, 12, 12).a, 255);
    // マスクを外して予算を下げると、Undo で付け直すのは断る（何も変えない）
    d.remove_layer_mask(l).unwrap();
    d.set_source_budget_bytes(d.allocated_bytes()).unwrap();
    assert_eq!(d.undo(), Err(CoreError::SourceBudgetExceeded));
    assert!(d.layer(l).unwrap().mask().is_none());
    assert_eq!(
        d.undo_count(),
        3,
        "断った段は残る（マスクを足す・描く・外す）"
    );
}

#[test]
fn mask_changes_are_tracked_for_every_channel_the_layer_has() {
    let mut d = Document::with_tile_size(32, 32, 16).unwrap();
    let l = d.add_layer("A").unwrap();
    d.set_pixel(l, 1, 1, RED).unwrap();
    d.set_channel_pixel(l, Channel::Roughness, 20, 20, RED)
        .unwrap();
    d.add_layer_mask(l).unwrap();
    d.clear_history().unwrap();
    let since = d.change_serial();
    mask_pixel(&mut d, l, 20, 3, false);
    for ch in [Channel::Color, Channel::Roughness] {
        assert!(
            changed(&d, ch, since).contains(&TileCoord::new(1, 0)),
            "{ch:?}"
        );
    }
    assert!(
        changed(&d, Channel::Height, since).is_empty(),
        "面の無いチャンネルは変わらない"
    );
    // グループのマスクは全チャンネル
    let g = d.group_layers(&[l], "G").unwrap();
    d.add_layer_mask(g).unwrap();
    let since = d.change_serial();
    d.set_mask_pixel(g, 30, 30, 9).unwrap();
    for ch in Channel::ALL {
        assert!(
            changed(&d, ch, since).contains(&TileCoord::new(1, 1)),
            "{ch:?}"
        );
    }
}

// ───────── 調整・塗りつぶし ─────────

fn one_pixel(c: Rgba8) -> (Document, LayerId) {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let l = d.add_layer("Paint").unwrap();
    d.set_pixel(l, 2, 2, c).unwrap();
    d.clear_history().unwrap();
    (d, l)
}

#[test]
fn an_adjustment_changes_colour_below_but_never_alpha() {
    let (mut d, _) = one_pixel(Rgba8::new(100, 50, 25, 128));
    d.add_adjustment_layer("Invert", AdjustmentSettings::invert(), None, None)
        .unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(155, 205, 230, 128));
    assert_eq!(at(&d, 9, 9), Rgba8::TRANSPARENT, "透明はそのまま");
}

#[test]
fn opacity_mask_and_blend_mode_mix_the_adjustment_back() {
    let (mut d, _) = one_pixel(Rgba8::new(100, 100, 100, 255));
    let adj = d
        .add_adjustment_layer("Invert", AdjustmentSettings::invert(), None, None)
        .unwrap();
    d.set_layer_opacity(adj, 0.5, false).unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(128, 128, 128, 255));
    d.set_layer_opacity(adj, 1.0, false).unwrap();
    d.set_layer_blend_mode(adj, BlendMode::Multiply).unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(61, 61, 61, 255));
    d.set_layer_blend_mode(adj, BlendMode::Normal).unwrap();
    d.add_layer_mask(adj).unwrap();
    let brush = BrushSettings {
        radius: 1.0,
        hardness: 1.0,
        pressure_size: false,
        pressure_opacity: false,
        ..BrushSettings::default()
    };
    let mut s = d.begin_mask_stroke(adj, &brush).unwrap();
    s.apply_pixel(&mut d, 2, 2, 1.0, 1.0).unwrap();
    d.end_stroke(s).unwrap();
    assert_eq!(
        at(&d, 2, 2),
        Rgba8::new(100, 100, 100, 255),
        "マスクが調整を隠す"
    );
}

#[test]
fn hue_saturation_cannot_target_scalar_channels() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let hs = AdjustmentSettings::hue_saturation(30.0, 0.0, 0.0).unwrap();
    assert!(d
        .add_adjustment_layer("bad", hs, Some(&[Channel::Roughness]), None)
        .is_err());
    let hsl = d.add_adjustment_layer("hsl", hs, None, None).unwrap();
    assert_eq!(
        d.layer(hsl).unwrap().enabled_channels(),
        vec![Channel::Color, Channel::Emission]
    );
    assert!(d.set_channel_enabled(hsl, Channel::Height, true).is_err());
    let levels = AdjustmentSettings::levels(0.0, 1.0, 2.0, 0.0, 1.0).unwrap();
    let lv = d
        .add_adjustment_layer("levels", levels, None, None)
        .unwrap();
    assert!(d.layer(lv).unwrap().is_channel_enabled(Channel::Roughness));
    let hs10 = AdjustmentSettings::hue_saturation(10.0, 0.0, 0.0).unwrap();
    assert!(
        d.set_adjustment(lv, hs10, false).is_err(),
        "Roughness が有効"
    );
    for ch in [
        Channel::Roughness,
        Channel::Metallic,
        Channel::Height,
        Channel::Normal,
    ] {
        d.set_channel_enabled(lv, ch, false).unwrap();
    }
    d.set_adjustment(lv, hs10, false).unwrap();
    assert!(d.begin_stroke(lv, &BrushSettings::default()).is_err());
    assert!(
        d.set_adjustment(lv, hs10, false).is_ok(),
        "同じ値は何もしない"
    );
}

#[test]
fn adjustment_changes_are_undoable_and_cover_the_canvas() {
    let (mut d, _) = one_pixel(Rgba8::new(100, 100, 100, 255));
    let adj = d
        .add_adjustment_layer(
            "L",
            AdjustmentSettings::levels(0.0, 1.0, 1.0, 0.0, 1.0).unwrap(),
            None,
            None,
        )
        .unwrap();
    d.clear_history().unwrap();
    let since = d.change_serial();
    d.set_adjustment(
        adj,
        AdjustmentSettings::levels(0.0, 1.0, 1.0, 0.0, 0.5).unwrap(),
        false,
    )
    .unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(50, 50, 50, 255));
    assert_eq!(
        changed(&d, Channel::Color, since).len(),
        4,
        "調整は画布の全タイル"
    );
    d.undo().unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(100, 100, 100, 255));
}

#[test]
fn slider_edits_coalesce_into_one_undo_step_until_the_run_ends() {
    let (mut d, l) = one_pixel(Rgba8::new(100, 100, 100, 255));
    for o in [0.9, 0.7, 0.5] {
        d.set_layer_opacity(l, o, true).unwrap();
    }
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!(d.layer(l).unwrap().opacity(), 1.0);
    d.redo().unwrap();
    assert_eq!(d.layer(l).unwrap().opacity(), 0.5);
    d.set_layer_opacity(l, 0.4, true).unwrap();
    assert_eq!(d.undo_count(), 2, "Undo・Redo でまとめが終わる");
    d.end_coalescing();
    d.set_layer_opacity(l, 0.3, true).unwrap();
    assert_eq!(d.undo_count(), 3);
    d.set_layer_name(l, "x").unwrap();
    d.set_layer_opacity(l, 0.2, true).unwrap();
    assert_eq!(d.undo_count(), 5, "ほかの編集でまとめが終わる");
    d.set_layer_opacity(l, 0.1, false).unwrap();
    d.set_layer_opacity(l, 0.05, false).unwrap();
    assert_eq!(d.undo_count(), 7);
    d.add_layer_mask(l).unwrap();
    d.set_layer_mask_density(l, 0.5, true).unwrap();
    d.set_layer_mask_density(l, 0.25, true).unwrap();
    d.set_layer_opacity(l, 0.6, true).unwrap();
    assert_eq!(d.undo_count(), 10, "別の対象は新しいまとめ");
    d.undo().unwrap();
    d.undo().unwrap();
    assert_eq!(d.layer(l).unwrap().mask().unwrap().density(), 1.0);
}

#[test]
fn fill_layers_cover_the_canvas_and_their_values_are_undoable() {
    let mut d = Document::with_tile_size(20, 12, 8).unwrap();
    let f = d
        .add_fill_layer("F", &[(Channel::Color, Rgba8::new(10, 20, 30, 255))], None)
        .unwrap();
    assert_eq!(at(&d, 19, 11), Rgba8::new(10, 20, 30, 255));
    assert_eq!(d.layer(f).unwrap().enabled_channels(), vec![Channel::Color]);
    assert!(
        d.begin_stroke(f, &BrushSettings::default()).is_err(),
        "塗りつぶしには描けない"
    );
    assert!(d
        .import_tile(f, Channel::Color, TileCoord::new(0, 0), &[0; 256])
        .is_err());
    let since = d.change_serial();
    d.set_fill_value(
        f,
        Channel::Roughness,
        Some(Rgba8::new(77, 77, 77, 255)),
        false,
    )
    .unwrap();
    assert!(
        d.layer(f).unwrap().is_channel_enabled(Channel::Roughness),
        "値を置くと有効に"
    );
    assert_eq!(
        changed(&d, Channel::Roughness, since).len(),
        6,
        "そのチャンネルの全タイル"
    );
    assert!(changed(&d, Channel::Color, since).is_empty());
    assert_eq!(
        d.composite_pixel(Channel::Roughness, 3, 3).unwrap(),
        Rgba8::new(77, 77, 77, 255)
    );
    // ドラッグはまとまる
    for v in [10u8, 20, 30] {
        d.set_fill_value(f, Channel::Color, Some(Rgba8::new(v, v, v, 255)), true)
            .unwrap();
    }
    assert_eq!(d.undo_count(), 3, "足す・Roughness・まとめたドラッグ");
    d.undo().unwrap();
    assert_eq!(at(&d, 0, 0), Rgba8::new(10, 20, 30, 255));
    d.undo().unwrap();
    assert!(
        !d.layer(f).unwrap().is_channel_enabled(Channel::Roughness),
        "取り消しで元の無効へ"
    );
    assert_eq!(d.layer(f).unwrap().fill_value(Channel::Roughness), None);
    // 値を消すと何も出さない（有効は保つ）
    d.set_fill_value(f, Channel::Color, None, false).unwrap();
    assert_eq!(at(&d, 0, 0), Rgba8::TRANSPARENT);
    assert!(d.layer(f).unwrap().is_channel_enabled(Channel::Color));
    let r = d.add_layer("R").unwrap();
    assert!(
        d.set_fill_value(r, Channel::Color, None, false).is_err(),
        "塗りつぶしだけ"
    );
}

// ───────── クリッピング ─────────

const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);
const RED2: Rgba8 = Rgba8::new(200, 0, 0, 255);
const GREEN: Rgba8 = Rgba8::new(0, 255, 0, 255);

fn stack(base_px: Rgba8, clipped_px: Rgba8) -> (Document, LayerId, LayerId, LayerId) {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let bg = full(&mut d, "Background", BLUE);
    let b = d.add_layer("Base").unwrap();
    d.set_pixel(b, 2, 2, base_px).unwrap();
    let c = d.add_layer("Clipped").unwrap();
    d.set_pixel(c, 2, 2, clipped_px).unwrap();
    d.set_pixel(c, 9, 9, clipped_px).unwrap();
    d.set_layer_clipping(c, true).unwrap();
    d.clear_history().unwrap();
    (d, bg, b, c)
}

#[test]
fn a_clipped_layer_shows_only_inside_the_base() {
    let (mut d, _, _, c) = stack(RED2, GREEN);
    assert_eq!(at(&d, 2, 2), GREEN);
    assert_eq!(at(&d, 9, 9), BLUE);
    d.set_layer_clipping(c, false).unwrap();
    assert_eq!(at(&d, 9, 9), GREEN);
    d.undo().unwrap();
    assert_eq!(at(&d, 9, 9), BLUE);
}

#[test]
fn the_group_keeps_the_bases_alpha_like_photoshop() {
    let (d, _, _, _) = stack(Rgba8::new(200, 0, 0, 128), GREEN);
    assert_eq!(at(&d, 2, 2), Rgba8::new(0, 128, 127, 255));
}

#[test]
fn clipped_blend_modes_see_only_the_base() {
    let (mut d, _, _, c) = stack(RED2, Rgba8::new(128, 128, 128, 255));
    d.set_layer_blend_mode(c, BlendMode::Multiply).unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(100, 0, 0, 255));
}

#[test]
fn the_bases_opacity_mask_and_visibility_apply_to_the_whole_group() {
    let (mut d, _, b, c) = stack(RED2, GREEN);
    d.set_layer_opacity(b, 0.5, false).unwrap();
    assert_eq!(
        at(&d, 2, 2),
        yolu_core::blend::blend(BLUE, GREEN, 0.5, BlendMode::Normal)
    );
    d.set_layer_opacity(b, 1.0, false).unwrap();
    d.add_layer_mask(b).unwrap();
    mask_pixel(&mut d, b, 2, 2, false);
    assert_eq!(at(&d, 2, 2), BLUE, "下地のマスクはクリッピングも隠す");
    d.set_layer_mask_enabled(b, false).unwrap();
    d.set_layer_visible(b, false).unwrap();
    assert_eq!(at(&d, 2, 2), BLUE, "見えない下地はクリッピングも隠す");
    d.set_layer_visible(b, true).unwrap();
    d.set_layer_opacity(c, 0.5, false).unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(100, 128, 0, 255));
}

#[test]
fn consecutive_clipped_layers_share_the_base_and_the_bottom_layer_cannot_clip() {
    let (mut d, bg, _, _) = stack(RED2, GREEN);
    let second = d.add_layer("Second").unwrap();
    d.set_pixel(second, 2, 2, Rgba8::new(255, 255, 255, 128))
        .unwrap();
    d.set_pixel(second, 9, 9, Rgba8::new(255, 255, 255, 255))
        .unwrap();
    d.set_layer_clipping(second, true).unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(128, 255, 128, 255));
    assert_eq!(at(&d, 9, 9), BLUE);
    d.set_layer_clipping(bg, true).unwrap();
    assert!(!d.is_effectively_clipped(0));
    assert_eq!(at(&d, 5, 5), BLUE);
}

#[test]
fn clipped_adjustments_and_fills_affect_only_the_base() {
    let (mut d, _, _, c) = stack(RED2, GREEN);
    d.set_layer_visible(c, false).unwrap();
    let inv = d
        .add_adjustment_layer("Invert", AdjustmentSettings::invert(), None, None)
        .unwrap();
    d.set_layer_clipping(inv, true).unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(55, 255, 255, 255));
    assert_eq!(at(&d, 9, 9), BLUE);
    d.set_layer_visible(inv, false).unwrap();
    let fill = d
        .add_fill_layer(
            "Tint",
            &[(Channel::Color, Rgba8::new(255, 255, 0, 255))],
            None,
        )
        .unwrap();
    d.set_layer_clipping(fill, true).unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(255, 255, 0, 255));
    assert_eq!(at(&d, 9, 9), BLUE);
}

#[test]
fn layers_clipped_to_an_adjustment_have_nothing_to_clip_to() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let bg = d.add_layer("bg").unwrap();
    d.set_pixel(bg, 1, 1, RED2).unwrap();
    d.add_adjustment_layer(
        "Levels",
        AdjustmentSettings::levels(0.0, 1.0, 1.0, 0.0, 0.5).unwrap(),
        None,
        None,
    )
    .unwrap();
    let c = d.add_layer("c").unwrap();
    d.set_pixel(c, 1, 1, GREEN).unwrap();
    d.set_layer_clipping(c, true).unwrap();
    assert_eq!(at(&d, 1, 1), Rgba8::new(100, 0, 0, 255));
}

#[test]
fn moving_the_base_marks_the_clipped_layers_tiles() {
    let (mut d, _, b, _) = stack(RED2, GREEN);
    let since = d.change_serial();
    d.move_layer(b, 0).unwrap(); // 背景が下地になる: 緑は (9,9) にも出る
    assert!(changed(&d, Channel::Color, since).contains(&TileCoord::new(1, 1)));
    assert_eq!(at(&d, 9, 9), GREEN);
}

// ───────── 合成の経路 ─────────

/// 入れ子・マスク・調整・塗りつぶし・クリッピングの混ざった文書。
fn mixed() -> Document {
    let mut d = Document::with_tile_size(53, 41, 16).unwrap();
    let mut seed = 0x1234_5678u64;
    let mut next = move || {
        seed = seed.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let mut ids = Vec::new();
    for l in 0..8 {
        let id = d.add_layer(&format!("L{l}")).unwrap();
        for _ in 0..500 {
            let r = next();
            let c = Rgba8::new(r as u8, (r >> 8) as u8, (r >> 16) as u8, (r >> 24) as u8);
            let (x, y) = ((r >> 32) as u32 % 53, (r >> 48) as u32 % 41);
            d.set_pixel(id, x, y, c).unwrap();
            d.set_channel_pixel(id, Channel::Normal, (x + 7) % 53, y, c)
                .unwrap();
        }
        d.set_layer_blend_mode(id, BlendMode::LAYER_MODES[(l * 5) % 26])
            .unwrap();
        d.set_layer_opacity(id, 0.35 + 0.08 * l as f64, false)
            .unwrap();
        ids.push(id);
    }
    d.set_layer_clipping(ids[3], true).unwrap();
    d.set_layer_clipping(ids[6], true).unwrap();
    d.add_layer_mask(ids[2]).unwrap();
    for i in 0..300u32 {
        d.set_mask_pixel(ids[2], (i * 7) % 53, (i * 13) % 41, (i * 37 % 256) as u8)
            .unwrap();
    }
    let g = d.group_layers(&[ids[1], ids[2], ids[3]], "G").unwrap();
    d.set_layer_opacity(g, 0.6, false).unwrap();
    let inner = d.group_layers(&[ids[5], ids[6]], "Inner").unwrap();
    d.set_layer_blend_mode(inner, BlendMode::Overlay).unwrap();
    d.add_adjustment_layer(
        "lv",
        AdjustmentSettings::levels(0.1, 0.9, 1.3, 0.0, 1.0).unwrap(),
        None,
        Some(ids[4]),
    )
    .unwrap();
    d.add_fill_layer(
        "f",
        &[(Channel::Color, Rgba8::new(30, 90, 200, 70))],
        Some(inner),
    )
    .unwrap();
    d.clear_history().unwrap();
    d
}

#[test]
fn the_bytes_do_not_depend_on_the_thread_count_or_row_order() {
    let d = mixed();
    for ch in [Channel::Color, Channel::Normal] {
        let reference = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(|| d.composite_channel(ch, d.bounds()).unwrap());
        for threads in [2, 3, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            assert_eq!(
                pool.install(|| d.composite_channel(ch, d.bounds()).unwrap()),
                reference,
                "{ch:?} {threads}"
            );
        }
        // 上から下の並び = 下から上の行を逆に
        let mut top_down = vec![0u8; reference.len()];
        d.composite_into(ch, d.bounds(), &mut top_down, RowOrder::TopDown)
            .unwrap();
        let row = d.width() as usize * 4;
        let flipped: Vec<u8> = reference.chunks(row).rev().flatten().copied().collect();
        assert_eq!(top_down, flipped);
        // 一部の矩形 = 全体の切り抜き
        let r = Rect::new(7, 5, 30, 21);
        let part = d.composite_channel(ch, r).unwrap();
        for y in 0..r.height as usize {
            let src = ((r.y as usize + y) * d.width() as usize + r.x as usize) * 4;
            assert_eq!(
                &part[y * r.width as usize * 4..(y + 1) * r.width as usize * 4],
                &reference[src..src + r.width as usize * 4]
            );
        }
    }
    assert_tiles_match_reference(&d);
}

#[test]
fn duplicating_copies_a_layer_or_a_group_with_its_contents_above_the_original() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    full(&mut d, "Base", Rgba8::new(0, 0, 255, 255));
    let a = solid(&mut d, "A", Rgba8::new(200, 10, 10, 180), 2, 2, 12, 12);
    d.set_channel_pixel(a, Channel::Height, 3, 3, Rgba8::new(9, 9, 9, 255))
        .unwrap();
    d.add_layer_mask(a).unwrap();
    d.set_mask_pixel(a, 4, 4, 200).unwrap();
    let b = solid(&mut d, "B", Rgba8::new(10, 200, 10, 200), 6, 6, 16, 16);
    let g = d.group_layers(&[a, b], "G").unwrap();
    d.set_layer_blend_mode(g, BlendMode::Multiply).unwrap();
    d.clear_history().unwrap();
    let bytes = d.allocated_bytes();
    let copy = d.duplicate_layer(g, Some("G copy")).unwrap();
    assert_eq!(shape(&d), "Base A<G B<G G A<G copy B<G copy G copy");
    assert_ne!(copy, g);
    let c = d.layer(copy).unwrap();
    assert_eq!(c.blend_mode(), BlendMode::Multiply);
    assert_eq!(
        d.allocated_bytes(),
        bytes * 2 - d.layers()[0].allocated_bytes(),
        "写しも予算に数える"
    );
    // 写しの中の層は別の ID で、画素・マスク・チャンネルは同じ
    let inner = d.children_of(Some(copy)).unwrap();
    let ca = d.layer(inner[0]).unwrap();
    assert_ne!(ca.id(), a);
    assert_eq!(
        ca.pixel(Channel::Height, 3, 3).unwrap(),
        Rgba8::new(9, 9, 9, 255)
    );
    assert_eq!(ca.mask().unwrap().surface().pixel(4, 4).unwrap().a, 200);
    // 写しへ描いても元は変わらない（タイルの共有は書くときに分かれる）
    d.set_pixel(inner[0], 5, 5, Rgba8::new(1, 2, 3, 255))
        .unwrap();
    assert_eq!(
        d.layer(a).unwrap().pixel(Channel::Color, 5, 5).unwrap(),
        Rgba8::new(200, 10, 10, 180)
    );
    let mut e = Document::with_tile_size(16, 16, 8).unwrap();
    let x = full(&mut e, "X", RED);
    e.clear_history().unwrap();
    let y = e.duplicate_layer(x, None).unwrap();
    assert_eq!(shape(&e), "X X");
    assert_eq!(e.undo_count(), 1);
    e.undo().unwrap();
    assert!(e.layer(y).is_none());
    e.set_source_budget_bytes(e.allocated_bytes()).unwrap();
    assert_eq!(
        e.duplicate_layer(x, None),
        Err(CoreError::SourceBudgetExceeded)
    );
    assert_eq!(shape(&e), "X");
}

#[test]
fn removals_count_their_pixels_against_the_history_budget() {
    let mut d = Document::with_tile_size(64, 64, 32).unwrap();
    let a = d.add_layer("A").unwrap();
    for y in 0..64 {
        for x in 0..64 {
            d.set_pixel(a, x, y, Rgba8::new(x as u8, y as u8, 7, 255))
                .unwrap();
        }
    }
    d.add_layer_mask(a).unwrap();
    for y in 0..32 {
        for x in 0..32 {
            d.set_mask_pixel(a, x, y, (x * 8) as u8).unwrap();
        }
    }
    d.clear_history().unwrap();
    let color = d
        .layer(a)
        .unwrap()
        .surface(Channel::Color)
        .unwrap()
        .allocated_bytes();
    let mask = d
        .layer(a)
        .unwrap()
        .mask()
        .unwrap()
        .surface()
        .allocated_bytes();
    assert_eq!((color, mask), (4 * 32 * 32 * 4, 32 * 32 * 4));
    d.remove_layer_mask(a).unwrap();
    assert_eq!(
        d.history_bytes(),
        64 + mask,
        "マスクを外す段はマスクの画素を持つ"
    );
    d.undo().unwrap();
    d.redo().unwrap();
    d.undo().unwrap();
    d.clear_history().unwrap();
    d.remove_layer(a).unwrap();
    assert_eq!(
        d.history_bytes(),
        128 + color + mask,
        "層を消す段は層の画素を持つ"
    );
    // 予算を下げると古い段から落とす（守る段の数だけは残す）
    d.set_minimum_undo_steps(1).unwrap();
    d.set_undo_budget_bytes(1000).unwrap();
    assert_eq!(d.undo_count(), 1, "守る 1 段は予算を超えても残す");
    d.add_layer("B").unwrap();
    assert_eq!(d.undo_count(), 1, "新しい段が入ると古い大きな段は落ちる");
    assert_eq!(d.history_trimmed().0, 1);
    assert!(d.history_bytes() <= 1000);
}

#[test]
fn persistent_ids_carry_the_group_structure() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let a = d.add_layer("A").unwrap();
    let b = d.add_layer("B").unwrap();
    d.group_layers(&[a, b], "G").unwrap();
    d.add_layer("Top").unwrap();
    let before = shape(&d);
    let ids: Vec<LayerId> = (1..=4).map(|i| LayerId(1000 + i)).collect();
    let d = d.with_persistent_ids(77, &ids).unwrap();
    assert_eq!(shape(&d), before);
    assert_eq!(d.layers()[0].parent(), Some(LayerId(1003)));
    d.validate_structure().unwrap();
}

#[test]
fn clipping_onto_a_group_switches_it_to_isolated_and_is_recorded() {
    // 通過のグループにクリッピングの層が入ると、グループは分離で合成する: 中身のタイルが変わる
    let mut d = Document::with_tile_size(8, 8, 4).unwrap();
    let b = d.add_layer("b").unwrap();
    d.set_pixel(b, 0, 0, Rgba8::new(200, 100, 50, 255)).unwrap();
    let g = d.add_group("g", None).unwrap();
    let inv = d
        .add_adjustment_layer("inv", AdjustmentSettings::invert(), None, None)
        .unwrap();
    d.move_layer_to(inv, Some(g), 0).unwrap();
    let c = d.add_layer("c").unwrap();
    let mut shown = d.composite(d.bounds()).unwrap();
    let mut serial = d.change_serial();
    type Edit = Box<dyn Fn(&mut Document)>;
    let edits: Vec<Edit> = vec![
        Box::new(move |d| d.set_layer_clipping(c, true).unwrap()),
        Box::new(move |d| d.set_layer_visible(c, false).unwrap()),
        Box::new(move |d| d.set_layer_visible(c, true).unwrap()),
        Box::new(|d| {
            d.undo().unwrap();
        }),
        Box::new(|d| {
            d.undo().unwrap();
        }),
        Box::new(|d| {
            d.undo().unwrap();
        }),
        Box::new(|d| {
            d.redo().unwrap();
        }),
        Box::new(move |d| d.remove_layer(c).unwrap()),
        Box::new(|d| {
            d.undo().unwrap();
        }),
        Box::new(move |d| {
            d.set_channel_pixel(c, Channel::Height, 1, 1, Rgba8::new(1, 1, 1, 255))
                .unwrap();
        }),
    ];
    for (k, edit) in edits.into_iter().enumerate() {
        edit(&mut d);
        for coord in d.changed_tiles(Channel::Color, serial).unwrap() {
            let r = d.tile_rect(coord).unwrap();
            let tile = d.composite(r).unwrap();
            for row in 0..r.height {
                let dst = (((r.y + row) * 8 + r.x) * 4) as usize;
                shown[dst..dst + (r.width * 4) as usize].copy_from_slice(
                    &tile[(row * r.width * 4) as usize..((row + 1) * r.width * 4) as usize],
                );
            }
        }
        serial = d.change_serial();
        assert_eq!(shown, d.composite(d.bounds()).unwrap(), "編集 {k}");
    }
}

#[test]
fn ungrouping_holds_the_groups_pixels_in_the_history_and_budget() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let a = full(&mut d, "A", RED);
    let g = d.group_layers(&[a], "G").unwrap();
    d.add_layer_mask(g).unwrap();
    for x in 0..8 {
        d.set_mask_pixel(g, x, 0, (x * 30) as u8).unwrap();
    }
    let mask = d.layer(g).unwrap().allocated_bytes();
    assert_eq!(mask, 8 * 8 * 4);
    d.ungroup(g).unwrap();
    assert_eq!(
        d.history_bytes(),
        128 + mask,
        "外したグループのマスクの画素を数える"
    );
    d.set_source_budget_bytes(d.allocated_bytes()).unwrap();
    assert_eq!(
        d.undo(),
        Err(CoreError::SourceBudgetExceeded),
        "戻すと予算を超える"
    );
    assert!(d.layer(g).is_none());
    d.set_source_budget_bytes(1 << 30).unwrap();
    d.undo().unwrap();
    assert_eq!(
        d.layer(g)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(7, 0)
            .unwrap()
            .a,
        210
    );
}

#[test]
fn a_plain_stack_is_raster_layers_without_groups_masks_or_channel_blends() {
    let mut d = Document::with_tile_size(8, 8, 8).unwrap();
    let a = d.add_layer("a").unwrap();
    assert!(d.is_plain_stack());
    d.add_layer_mask(a).unwrap();
    assert!(!d.is_plain_stack());
    d.undo().unwrap();
    d.set_channel_opacity(a, Channel::Color, Some(0.5), false)
        .unwrap();
    assert!(!d.is_plain_stack());
    d.undo().unwrap();
    d.group_layers(&[a], "G").unwrap();
    assert!(!d.is_plain_stack());
    d.undo().unwrap();
    d.add_fill_layer("f", &[], None).unwrap();
    assert!(!d.is_plain_stack());
}
