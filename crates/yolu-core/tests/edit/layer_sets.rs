//! レイヤーセット: 既定の形は今の合成と同じバイト、出力の割り当て・複製・削除・名前・並べ替えの 1 回の取り消し、レイヤーセットをまたぐ編集の断り、
//! レイヤーを別のレイヤーセットへ移す、効果（Anchor）とクローンの参照元がレイヤーセットの中だけを読むこと。

use yolu_core::generator::{self, anchor::ReadMode, Settings};
use yolu_core::glam::DVec2;
use yolu_core::Reason;
use yolu_core::{
    AnchorIssueKind, AnchorPlacement, Brush, BrushEffect, BrushSample, BrushSettings, Channel,
    CoreError, Document, EffectSettings, FilterSpec, FilterTarget, LayerId, LayerSetId,
    PressureSwitches, Rgba8, TileCoord,
};

const RED: Rgba8 = Rgba8::new(220, 20, 30, 255);
const BLUE: Rgba8 = Rgba8::new(10, 40, 230, 255);

fn fill(d: &mut Document, id: LayerId, c: Rgba8, x0: u32, y0: u32, x1: u32, y1: u32) {
    for y in y0..y1 {
        for x in x0..x1 {
            d.set_pixel(id, x, y, c).unwrap();
        }
    }
}

fn all(d: &Document, c: Channel) -> Vec<u8> {
    d.composite_channel(c, d.bounds()).unwrap()
}

fn set_all(d: &Document, set: LayerSetId, c: Channel) -> Vec<u8> {
    d.composite_set_channel(set, c, d.bounds()).unwrap()
}

/// 赤の下地と、半分を覆う青のレイヤー（最初のレイヤーセット）。
fn two_layers() -> (Document, LayerId, LayerId) {
    let mut d = Document::with_tile_size(24, 16, 8).unwrap();
    let a = d.add_layer("a").unwrap();
    fill(&mut d, a, RED, 0, 0, 24, 16);
    let b = d.add_layer("b").unwrap();
    fill(&mut d, b, BLUE, 0, 0, 12, 16);
    d.clear_history().unwrap();
    (d, a, b)
}

fn uniform(c: Rgba8, n: usize) -> Vec<u8> {
    c.to_array().repeat(n)
}

#[test]
fn a_new_document_has_one_unnamed_set_that_every_channel_reads() {
    let (d, a, b) = two_layers();
    assert_eq!(d.layer_sets().len(), 1);
    let set = d.layer_sets()[0].id;
    assert!(d.layer_sets()[0].name.is_empty());
    assert!(d.has_default_layer_sets());
    for c in d.channels() {
        assert_eq!(d.channel_output(c).unwrap(), Some(set));
        assert_eq!(all(&d, c), set_all(&d, set, c), "{c:?}");
    }
    assert_eq!(d.layer_set_of(a).unwrap(), set);
    assert_eq!(d.layer_set_of(b).unwrap(), set);
    // 最終の結果は、そのレイヤーセットの合成と同じバイト（タイルの経路・画素ごとの参照・タイルの束）
    let tiles: Vec<_> = d.canvas_tiles().collect();
    let by_tiles = d.composite_tiles(Channel::Color, &tiles).unwrap();
    let by_set = d
        .composite_set_tiles_cancellable(set, Channel::Color, &tiles, None)
        .unwrap();
    for (x, y) in by_tiles.iter().zip(&by_set) {
        assert_eq!(x.pixels, y.pixels);
    }
    assert_eq!(
        d.composite_pixel(Channel::Color, 20, 3).unwrap(),
        d.composite_set_pixel(set, Channel::Color, 20, 3).unwrap()
    );
}

#[test]
fn changing_the_output_changes_the_result_and_one_undo_restores_it() {
    let (mut d, _, _) = two_layers();
    let before = all(&d, Channel::Color);
    let first = d.layer_sets()[0].id;
    let other = d.add_layer_set("影").unwrap();
    // 足しただけでは最終の結果は変わらない
    assert_eq!(all(&d, Channel::Color), before);
    let o = d.add_layer_set("下書き").unwrap();
    d.undo().unwrap();
    assert!(d.layer_sets().iter().all(|s| s.id != o));
    let serial = d.change_serial();
    d.set_channel_output(Channel::Color, Some(other)).unwrap();
    assert_eq!(d.channel_output(Channel::Color).unwrap(), Some(other));
    // 空のレイヤーセットの合成は透明。ほかのチャンネルは前のまま
    assert_eq!(all(&d, Channel::Color), vec![0u8; 24 * 16 * 4]);
    assert_eq!(
        d.channel_output(Channel::Height).unwrap(),
        Some(first),
        "ほかのチャンネルは変えない"
    );
    // 最終の結果がキャンバス全体で変わったことを、変化の記録が言う
    assert_eq!(
        d.changed_tiles(Channel::Color, serial).unwrap().len(),
        d.canvas_tiles().count()
    );
    assert!(!d.has_default_layer_sets());
    d.undo().unwrap();
    assert_eq!(all(&d, Channel::Color), before);
    d.redo().unwrap();
    assert_eq!(all(&d, Channel::Color), vec![0u8; 24 * 16 * 4]);
}

#[test]
fn no_output_gives_the_channel_default_value() {
    let (mut d, _, _) = two_layers();
    d.set_channel_output(Channel::Roughness, None).unwrap();
    let default = d.channel_info(Channel::Roughness).unwrap().default;
    assert_eq!(all(&d, Channel::Roughness), uniform(default, 24 * 16));
    assert_eq!(
        d.composite_pixel(Channel::Roughness, 5, 5).unwrap(),
        default
    );
    let tiles: Vec<_> = d.canvas_tiles().collect();
    for t in d.composite_tiles(Channel::Roughness, &tiles).unwrap() {
        assert_eq!(t.pixels, uniform(default, t.pixels.len() / 4));
    }
    for t in d
        .composite_coarse_tiles(Channel::Roughness, &tiles, 2)
        .unwrap()
    {
        assert_eq!(t.pixels, uniform(default, t.pixels.len() / 4));
    }
    // 割り当ての無いチャンネルも、レイヤーセットを指せば合成できる
    let first = d.layer_sets()[0].id;
    assert_eq!(
        set_all(&d, first, Channel::Roughness),
        vec![0u8; 24 * 16 * 4]
    );
    // 無いチャンネルは、割り当てが無くても断る
    let gone = Channel::from_index(9).unwrap();
    assert_eq!(
        d.composite_channel(gone, d.bounds()),
        Err(CoreError::ChannelNotFound)
    );
}

#[test]
fn a_duplicated_set_composites_the_same_and_has_new_ids() {
    let (mut d, a, b) = two_layers();
    let first = d.layer_sets()[0].id;
    let group = d.group_layers(&[b], "g").unwrap();
    let steps = d.undo_count();
    let copy = d.duplicate_layer_set(first, Some("案 B")).unwrap();
    assert_eq!(d.undo_count(), steps + 1, "1 回の取り消し");
    assert_eq!(d.layer_sets().len(), 2);
    assert_eq!(d.layer_sets()[1].id, copy);
    assert_eq!(d.layer_sets()[1].name, "案 B");
    for c in d.channels() {
        assert_eq!(set_all(&d, copy, c), set_all(&d, first, c), "{c:?}");
    }
    let copied: Vec<_> = d
        .layers()
        .iter()
        .filter(|l| l.layer_set() == copy)
        .collect();
    assert_eq!(copied.len(), 3);
    assert!(copied.iter().all(|l| ![a, b, group].contains(&l.id())));
    // グループの子は写しのグループに入る
    let g2 = copied.iter().find(|l| l.name() == "g").unwrap().id();
    assert!(copied
        .iter()
        .any(|l| l.name() == "b" && l.parent() == Some(g2)));
    let a2 = copied.iter().find(|l| l.name() == "a").unwrap().id();
    // 1 回の取り消しで、レイヤーセットも写したレイヤーも無くなる
    d.undo().unwrap();
    assert_eq!(d.layer_sets().len(), 1);
    assert_eq!(d.layers().len(), 3);
    d.redo().unwrap();
    assert_eq!(d.layer_sets().len(), 2);
    assert_eq!(d.layers().len(), 6);
    // 写しを描き替えても元は変わらない（タイルは書いたときに分かれる）
    let before = set_all(&d, first, Channel::Color);
    d.set_pixel(a2, 20, 2, BLUE).unwrap();
    assert_eq!(set_all(&d, first, Channel::Color), before);
    assert_ne!(set_all(&d, copy, Channel::Color), before);
}

#[test]
fn a_duplicate_over_the_pixel_budget_is_refused_and_changes_nothing() {
    let (mut d, _, _) = two_layers();
    let first = d.layer_sets()[0].id;
    d.set_source_budget_bytes(d.allocated_bytes() + 16).unwrap();
    let revision = d.revision();
    assert_eq!(
        d.duplicate_layer_set(first, None),
        Err(CoreError::SourceBudgetExceeded)
    );
    assert_eq!(d.layer_sets().len(), 1);
    assert_eq!(d.layers().len(), 2);
    assert_eq!(d.revision(), revision);
    assert!(!d.can_undo());
}

#[test]
fn removing_a_set_clears_its_outputs_to_the_default_and_the_last_cannot_go() {
    let (mut d, _, _) = two_layers();
    let first = d.layer_sets()[0].id;
    let rough_before = all(&d, Channel::Roughness);
    let other = d.add_layer_set("影").unwrap();
    let shadow = d.add_layer("影のレイヤー").unwrap();
    d.move_layers_to_set(&[shadow], other).unwrap();
    d.set_channel_output(Channel::Color, Some(other)).unwrap();
    d.set_channel_output(Channel::Normal, Some(other)).unwrap();
    d.move_layer_set(other, 0).unwrap();
    assert_eq!(d.layer_sets()[0].id, other);
    let color_before = all(&d, Channel::Color);
    d.clear_history().unwrap();
    // 消したレイヤーセットから出していたチャンネルは、残ったレイヤーセットへ移さず割り当てが無くなる
    d.remove_layer_set(other).unwrap();
    assert_eq!(d.layer_sets().len(), 1);
    assert!(
        d.layer(shadow).is_none(),
        "消したレイヤーセットのレイヤーも消える"
    );
    assert_eq!(d.channel_output(Channel::Color).unwrap(), None);
    assert_eq!(d.channel_output(Channel::Normal).unwrap(), None);
    // 割り当てが無いチャンネルの最終の結果は、そのチャンネルの既定値
    for c in [Channel::Color, Channel::Normal] {
        let default = d.channel_info(c).unwrap().default;
        assert_eq!(
            all(&d, c),
            uniform(default, 24 * 16),
            "{c:?} は既定値で塗った画素"
        );
    }
    // 別のレイヤーセットから出していたチャンネルはそのまま
    assert_eq!(d.channel_output(Channel::Roughness).unwrap(), Some(first));
    assert_eq!(all(&d, Channel::Roughness), rough_before);
    assert_eq!(
        d.remove_layer_set(first),
        Err(CoreError::InvalidArgument(
            Reason::LastLayerSetCannotBeRemoved
        ))
    );
    // 1 回の取り消しで、レイヤーセット・中のレイヤー・割り当てが戻る
    d.undo().unwrap();
    assert_eq!(d.layer_sets()[0].id, other);
    assert_eq!(d.layer_set_of(shadow).unwrap(), other);
    assert_eq!(d.channel_output(Channel::Color).unwrap(), Some(other));
    assert_eq!(d.channel_output(Channel::Normal).unwrap(), Some(other));
    assert_eq!(all(&d, Channel::Color), color_before);
    assert!(!d.can_undo());
}

#[test]
fn renaming_and_reordering_are_one_undo_each() {
    let (mut d, _, _) = two_layers();
    let first = d.layer_sets()[0].id;
    let other = d.add_layer_set("x").unwrap();
    d.rename_layer_set(first, "マテリアル").unwrap();
    assert_eq!(d.layer_sets()[0].name, "マテリアル");
    assert!(d.rename_layer_set(first, "a\nb").is_err());
    d.move_layer_set(other, 0).unwrap();
    let order: Vec<_> = d.layer_sets().iter().map(|s| s.id).collect();
    assert_eq!(order, [other, first]);
    d.undo().unwrap();
    let order: Vec<_> = d.layer_sets().iter().map(|s| s.id).collect();
    assert_eq!(order, [first, other]);
    d.undo().unwrap();
    assert!(d.layer_sets()[0].name.is_empty());
}

#[test]
fn edits_across_sets_are_refused_and_change_nothing() {
    let (mut d, a, b) = two_layers();
    let other = d.add_layer_set("x").unwrap();
    let c = d.add_layer("c").unwrap();
    let g = d.add_group("g", Some(c)).unwrap();
    d.move_layers_to_set(&[c, g], other).unwrap();
    let shape = |d: &Document| {
        d.layers()
            .iter()
            .map(|l| (l.id(), l.parent(), l.layer_set()))
            .collect::<Vec<_>>()
    };
    let before = shape(&d);
    let revision = d.revision();
    // 並べ替え: 別のレイヤーセットのグループへは入れない
    assert!(matches!(
        d.move_layer_to(a, Some(g), 0),
        Err(CoreError::InvalidArgument(_))
    ));
    assert!(matches!(
        d.move_layers(&[a, c], None, 0),
        Err(CoreError::InvalidArgument(_))
    ));
    // グループにまとめる・結合は同じレイヤーセットの中だけ
    assert!(matches!(
        d.group_layers(&[b, c], "x"),
        Err(CoreError::InvalidArgument(_))
    ));
    assert!(matches!(
        d.merge_layers(&[b, c], 2),
        Err(CoreError::InvalidArgument(_))
    ));
    assert_eq!(shape(&d), before);
    assert_eq!(d.revision(), revision);
    // 一番上の段の並べ替えは、同じレイヤーセットの兄弟の中の位置
    d.move_layer(a, 1).unwrap();
    let first = d.layer_sets()[0].id;
    let order: Vec<_> = d
        .layers()
        .iter()
        .filter(|l| l.layer_set() == first)
        .map(|l| l.id())
        .collect();
    assert_eq!(order, [b, a]);
    // 下へ結合は、同じレイヤーセットのすぐ下のレイヤーとだけ（c のすぐ下の並びは b だが、別のレイヤーセット）
    let c2 = d.add_layer_above("c2", Some(c)).unwrap();
    assert_eq!(d.layer_set_of(c2).unwrap(), other);
    d.move_layer(c2, 0).unwrap();
    assert!(d.merge_down_refusal(c2).unwrap().is_some());
}

#[test]
fn new_layers_go_into_the_set_of_the_layer_below_or_the_first_set() {
    let (mut d, a, _) = two_layers();
    let first = d.layer_sets()[0].id;
    let other = d.add_layer_set("x").unwrap();
    let c = d.add_layer("c").unwrap();
    d.move_layers_to_set(&[c], other).unwrap();
    let top = d.add_layer("top").unwrap();
    assert_eq!(d.layer_set_of(top).unwrap(), first);
    let above_c = d.add_layer_above("above c", Some(c)).unwrap();
    assert_eq!(d.layer_set_of(above_c).unwrap(), other);
    let fill = d
        .add_fill_layer("fill", &[(Channel::Color, RED)], Some(above_c))
        .unwrap();
    assert_eq!(d.layer_set_of(fill).unwrap(), other);
    let above_a = d.add_group("group", Some(a)).unwrap();
    assert_eq!(d.layer_set_of(above_a).unwrap(), first);
}

#[test]
fn moving_a_group_to_another_set_takes_its_contents_to_the_top() {
    let (mut d, a, b) = two_layers();
    let first = d.layer_sets()[0].id;
    let before = set_all(&d, first, Channel::Color);
    let other = d.add_layer_set("x").unwrap();
    let base = d.add_layer("base").unwrap();
    d.move_layers_to_set(&[base], other).unwrap();
    let g = d.group_layers(&[b], "g").unwrap();
    d.clear_history().unwrap();
    d.move_layers_to_set(&[g], other).unwrap();
    assert_eq!(d.layer_set_of(g).unwrap(), other);
    assert_eq!(d.layer_set_of(b).unwrap(), other);
    assert_eq!(d.layer(b).unwrap().parent(), Some(g));
    // 移した先の一番上
    let top: Vec<_> = d
        .layers()
        .iter()
        .filter(|l| l.layer_set() == other && l.parent().is_none())
        .map(|l| l.id())
        .collect();
    assert_eq!(top, [base, g]);
    assert_eq!(
        d.layers().iter().filter(|l| l.layer_set() == first).count(),
        1
    );
    // 元のレイヤーセットには a だけが残る
    let mut only_a = Document::with_tile_size(24, 16, 8).unwrap();
    let a2 = only_a.add_layer("a").unwrap();
    fill(&mut only_a, a2, RED, 0, 0, 24, 16);
    assert_eq!(
        set_all(&d, first, Channel::Color),
        all(&only_a, Channel::Color)
    );
    assert!(d.layer(a).is_some());
    d.undo().unwrap();
    assert_eq!(d.layer_set_of(g).unwrap(), first);
    assert_eq!(d.layer_set_of(b).unwrap(), first);
    assert_eq!(set_all(&d, first, Channel::Color), before);
}

fn anchor_generator() -> EffectSettings {
    let mut g = Settings::new(generator::Kind::Anchor);
    g.blend = generator::Blend::Replace;
    EffectSettings::generator(g)
}

#[test]
fn anchors_are_read_only_inside_the_layer_set() {
    let (mut d, a, b) = two_layers();
    let anchor = d
        .add_anchor(a, AnchorPlacement::Layer, Some("下地"), None)
        .unwrap();
    let reader = d
        .add_filter(
            b,
            FilterTarget::Content,
            FilterSpec::new(anchor_generator()).channels(&[Channel::Color]),
        )
        .unwrap();
    d.set_generator_anchor(
        b,
        reader,
        Some(anchor),
        Channel::Color,
        ReadMode::Value,
        false,
    )
    .unwrap();
    assert!(d.anchor_issues().is_empty());
    let first = d.layer_sets()[0].id;
    let reading = set_all(&d, first, Channel::Color);
    // 下地を別のレイヤーセットへ移すと、読む段はその Anchor を「無い」として入力のまま通す
    let other = d.add_layer_set("x").unwrap();
    d.move_layers_to_set(&[a], other).unwrap();
    let issues = d.anchor_issues();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].kind, AnchorIssueKind::Missing);
    assert_eq!(
        d.anchor_reference_refusal(b, Some(anchor)).unwrap(),
        Some(AnchorIssueKind::Missing)
    );
    assert!(d.anchors_readable_from(b).unwrap().is_empty());
    assert!(d
        .set_generator_anchor(
            b,
            reader,
            Some(anchor),
            Channel::Color,
            ReadMode::Value,
            false
        )
        .is_err());
    // b だけの合成（入力のまま通す段は b の画素のまま）
    let mut alone = Document::with_tile_size(24, 16, 8).unwrap();
    let b2 = alone.add_layer("b").unwrap();
    fill(&mut alone, b2, BLUE, 0, 0, 12, 16);
    assert_eq!(
        set_all(&d, first, Channel::Color),
        all(&alone, Channel::Color)
    );
    d.undo().unwrap();
    assert!(d.anchor_issues().is_empty());
    assert_eq!(set_all(&d, first, Channel::Color), reading);
}

fn clone_brush() -> Brush {
    Brush {
        effect: BrushEffect::Clone {
            offset: DVec2::new(-2.0, 0.0),
        },
        ..Brush::simple(
            BrushSettings {
                radius: 3.0,
                hardness: 1.0,
                ..BrushSettings::default()
            },
            PressureSwitches::NONE,
        )
    }
}

/// target へ、合成を参照元にしたクローンで 1 点描いて、target の画素が変わったか。
fn clone_once(d: &mut Document, target: LayerId) -> bool {
    let mut stroke = d.begin_brush_stroke(target, &clone_brush()).unwrap();
    stroke.use_composite_clone_source(d).unwrap();
    stroke
        .add_sample(
            d,
            BrushSample::new(18.0, 8.0, 1.0, 0.0, DVec2::ZERO).unwrap(),
        )
        .unwrap();
    d.end_stroke(stroke).unwrap().changed
}

#[test]
fn the_clone_source_reads_only_the_set_of_the_painted_layer() {
    // 同じレイヤーセットなら、下の赤を写す
    let (mut d, _, _) = two_layers();
    let target = d.add_layer("target").unwrap();
    assert!(clone_once(&mut d, target));
    // 別のレイヤーセット（空）のレイヤーへは、赤も青も写らない（出力の割り当てをそこへ向けても同じ）
    let (mut d, _, _) = two_layers();
    let other = d.add_layer_set("x").unwrap();
    let target = d.add_layer("target").unwrap();
    d.move_layers_to_set(&[target], other).unwrap();
    assert!(!clone_once(&mut d, target));
    d.set_channel_output(Channel::Color, Some(other)).unwrap();
    assert!(!clone_once(&mut d, target));
    assert_eq!(set_all(&d, other, Channel::Color), vec![0u8; 24 * 16 * 4]);
}

#[test]
fn merging_visible_layers_keeps_the_other_sets() {
    let (mut d, _, _) = two_layers();
    let first = d.layer_sets()[0].id;
    let before = set_all(&d, first, Channel::Color);
    let other = d.add_layer_set("x").unwrap();
    let c = d.add_layer("c").unwrap();
    fill(&mut d, c, BLUE, 0, 0, 24, 16);
    d.move_layers_to_set(&[c], other).unwrap();
    let report = d.merge_visible_in(first, "merged", 2).unwrap();
    assert_eq!(report.max_visible_difference, 0);
    assert_eq!(d.layer_set_of(report.result_id).unwrap(), first);
    assert_eq!(d.layer_set_of(c).unwrap(), other);
    assert_eq!(set_all(&d, first, Channel::Color), before);
    assert_eq!(
        d.layers().iter().filter(|l| l.layer_set() == first).count(),
        1
    );
}

#[test]
fn loading_layer_sets_checks_ids_routes_and_group_members() {
    let (mut d, a, b) = two_layers();
    let g = d.group_layers(&[b], "g").unwrap();
    let s1 = LayerSetId(11);
    let s2 = LayerSetId(22);
    let sets = |names: &[(LayerSetId, &str)]| {
        names
            .iter()
            .map(|(id, n)| yolu_core::LayerSetInfo {
                id: *id,
                name: (*n).to_owned(),
            })
            .collect::<Vec<_>>()
    };
    let order: Vec<LayerId> = d.layers().iter().map(|l| l.id()).collect();
    assert_eq!(order, [a, b, g]);
    // グループの子が親と違うレイヤーセット・在らないレイヤーセット・重なる ID・同じチャンネル 2 度は断る
    for (s, route, members) in [
        (sets(&[(s1, ""), (s2, "x")]), vec![], vec![s1, s1, s2]),
        (sets(&[(s1, "")]), vec![], vec![s1, s2, s2]),
        (sets(&[(s1, ""), (s1, "x")]), vec![], vec![s1, s1, s1]),
        (sets(&[]), vec![], vec![]),
        (
            sets(&[(s1, "")]),
            vec![(Channel::Color, Some(s2))],
            vec![s1, s1, s1],
        ),
        (
            sets(&[(s1, "")]),
            vec![(Channel::Color, None), (Channel::Color, Some(s1))],
            vec![s1, s1, s1],
        ),
    ] {
        assert!(d.set_layer_sets_for_load(s, &route, &members).is_err());
        assert_eq!(d.layer_sets().len(), 1, "断ったら何も変えない");
    }
    d.set_layer_sets_for_load(
        sets(&[(s1, "マテリアル"), (s2, "")]),
        &[(Channel::Color, Some(s2)), (Channel::Height, None)],
        &[s1, s2, s2],
    )
    .unwrap();
    assert_eq!(d.layer_set_of(a).unwrap(), s1);
    assert_eq!(d.layer_set_of(g).unwrap(), s2);
    assert_eq!(d.channel_output(Channel::Color).unwrap(), Some(s2));
    assert_eq!(d.channel_output(Channel::Height).unwrap(), None);
    assert_eq!(d.channel_output(Channel::Metallic).unwrap(), Some(s1));
    d.validate_structure().unwrap();
}

#[test]
fn a_layer_can_be_added_to_an_empty_set_and_goes_to_its_top() {
    let (mut d, a, b) = two_layers();
    let first = d.layer_sets()[0].id;
    let other = d.add_layer_set("影のマスク").unwrap();
    d.clear_history().unwrap();
    let before = set_all(&d, first, Channel::Color);
    // 空のレイヤーセットへ足す（`add_layer_above(.., None)` だと最初のレイヤーセットへ入る）
    let x = d.add_layer_in_set("x", other, None).unwrap();
    assert_eq!(d.layer_set_of(x).unwrap(), other);
    assert_eq!(d.undo_count(), 1, "1 回の取り消し");
    assert_eq!(d.history().last(), Some(yolu_core::HistoryKind::AddLayer));
    assert_eq!(
        d.layers().last().map(|l| l.id()),
        Some(x),
        "並びの一番上（ほかのレイヤーセットの合成は変わらない）"
    );
    assert_eq!(set_all(&d, first, Channel::Color), before);
    let y = d.add_layer_in_set("y", other, Some(x)).unwrap();
    assert_eq!(d.layer_set_of(y).unwrap(), other);
    assert_eq!(d.layer_index(y), d.layer_index(x).map(|i| i + 1));
    // above を渡さなければ、そのレイヤーセットの一番上
    let z = d.add_layer_in_set("z", first, None).unwrap();
    assert_eq!(d.layer_set_of(z).unwrap(), first);
    assert_eq!(d.children_in_set(first, None).unwrap(), [a, b, z]);
    assert_eq!(d.children_in_set(other, None).unwrap(), [x, y]);
    // 別のレイヤーセットのレイヤーの上へは断り、何も変えない
    let steps = d.undo_count();
    let revision = d.revision();
    assert_eq!(
        d.add_layer_in_set("w", other, Some(a)),
        Err(CoreError::InvalidArgument(Reason::AboveInOtherLayerSet))
    );
    let gone = LayerSetId(0x7777);
    assert_eq!(
        d.add_layer_in_set("w", gone, None),
        Err(CoreError::InvalidArgument(Reason::LayerSetDoesNotExist))
    );
    assert_eq!(d.undo_count(), steps);
    assert_eq!(d.revision(), revision);
    d.undo().unwrap();
    d.undo().unwrap();
    d.undo().unwrap();
    assert!(d.children_in_set(other, None).unwrap().is_empty());
}

#[test]
fn children_are_listed_per_set_so_positions_count_inside_one_set() {
    let (mut d, a, b) = two_layers();
    let first = d.layer_sets()[0].id;
    let other = d.add_layer_set("x").unwrap();
    let c = d.add_layer_in_set("c", other, None).unwrap();
    let e = d.add_layer_in_set("e", other, None).unwrap();
    // 全部のレイヤーセットの一番上の段は、`children_of(None)` が返す
    assert_eq!(d.children_of(None).unwrap(), [a, b, c, e]);
    assert_eq!(d.children_in_set(first, None).unwrap(), [a, b]);
    assert_eq!(d.children_in_set(other, None).unwrap(), [c, e]);
    // 位置はレイヤーセットの中で数える（`move_layer_to` と同じ数え方）
    d.move_layer_to(e, None, 0).unwrap();
    assert_eq!(d.children_in_set(other, None).unwrap(), [e, c]);
    assert_eq!(d.children_in_set(first, None).unwrap(), [a, b]);
    // グループの子: 同じレイヤーセットから引ける。別のレイヤーセットから引くと空
    let g = d.add_group("g", Some(c)).unwrap();
    d.move_layer_to(c, Some(g), 0).unwrap();
    assert_eq!(d.children_in_set(other, Some(g)).unwrap(), [c]);
    assert!(d.children_in_set(first, Some(g)).unwrap().is_empty());
    assert_eq!(d.children_of(Some(g)).unwrap(), [c]);
    assert_eq!(
        d.children_in_set(LayerSetId(0x7777), None),
        Err(CoreError::InvalidArgument(Reason::LayerSetDoesNotExist))
    );
    assert_eq!(
        d.children_in_set(first, Some(a)),
        Err(CoreError::InvalidArgument(Reason::NotGroup))
    );
}

/// 2 つのレイヤーセットに、別々のタイルへ描いたレイヤーを 1 つずつ置く（タイルは 8 画素で 3×2 枚）。
fn two_sets_on_different_tiles() -> (Document, LayerSetId, LayerSetId) {
    let mut d = Document::with_tile_size(24, 16, 8).unwrap();
    let first = d.layer_sets()[0].id;
    let a = d.add_layer_in_set("a", first, None).unwrap();
    fill(&mut d, a, RED, 0, 0, 8, 8);
    let other = d.add_layer_set("影").unwrap();
    let b = d.add_layer_in_set("b", other, None).unwrap();
    fill(&mut d, b, BLUE, 16, 8, 24, 16);
    d.clear_history().unwrap();
    (d, first, other)
}

#[test]
fn an_output_change_marks_only_the_tiles_the_two_sets_have_pixels_in() {
    let (mut d, first, other) = two_sets_on_different_tiles();
    let (a_tile, b_tile) = (TileCoord::new(0, 0), TileCoord::new(2, 1));
    let before = all(&d, Channel::Color);
    let serial = d.change_serial();
    d.set_channel_output(Channel::Color, Some(other)).unwrap();
    // 割り当てが変わったことで最終の結果が変わるのは、どちらかのレイヤーセットに画素のあるタイルだけ
    assert_eq!(
        d.changed_tiles(Channel::Color, serial).unwrap(),
        [a_tile, b_tile]
    );
    // ほかのチャンネルは印を付けない
    assert!(d
        .changed_tiles(Channel::Roughness, serial)
        .unwrap()
        .is_empty());
    // 印のないタイルの最終の結果は、割り当てを変えても変わらない（どちらも透明）
    let after = all(&d, Channel::Color);
    for tile in d.canvas_tiles() {
        if [a_tile, b_tile].contains(&tile) {
            continue;
        }
        let r = d.tile_rect(tile).unwrap();
        for y in r.y..r.y + r.height {
            for x in r.x..r.x + r.width {
                let i = ((y * 24 + x) * 4) as usize;
                assert_eq!(before[i..i + 4], after[i..i + 4], "{tile:?} ({x}, {y})");
            }
        }
    }
    // 取り消しも同じ範囲
    let serial = d.change_serial();
    d.undo().unwrap();
    assert_eq!(
        d.changed_tiles(Channel::Color, serial).unwrap(),
        [a_tile, b_tile]
    );
    assert_eq!(all(&d, Channel::Color), before);
    assert_eq!(d.channel_output(Channel::Color).unwrap(), Some(first));
}

#[test]
fn an_output_change_to_or_from_none_marks_the_whole_canvas_because_the_default_fills_it() {
    let (mut d, _, other) = two_sets_on_different_tiles();
    let serial = d.change_serial();
    d.set_channel_output(Channel::Roughness, None).unwrap();
    assert_eq!(
        d.changed_tiles(Channel::Roughness, serial).unwrap().len(),
        d.canvas_tiles().count()
    );
    let serial = d.change_serial();
    d.set_channel_output(Channel::Roughness, Some(other))
        .unwrap();
    assert_eq!(
        d.changed_tiles(Channel::Roughness, serial).unwrap().len(),
        d.canvas_tiles().count()
    );
}

#[test]
fn a_fill_layer_in_a_set_marks_the_whole_canvas_when_the_output_moves_to_it() {
    let (mut d, first, other) = two_sets_on_different_tiles();
    d.add_fill_layer(
        "fill",
        &[(Channel::Color, RED)],
        d.children_in_set(other, None).unwrap().first().copied(),
    )
    .unwrap();
    d.clear_history().unwrap();
    let serial = d.change_serial();
    d.set_channel_output(Channel::Color, Some(other)).unwrap();
    assert_eq!(
        d.changed_tiles(Channel::Color, serial).unwrap().len(),
        d.canvas_tiles().count()
    );
    let _ = first;
}

#[test]
fn layer_set_edits_are_named_in_the_history() {
    let (mut d, a, _) = two_layers();
    let first = d.layer_sets()[0].id;
    let last = |d: &Document| d.history().last();
    let other = d.add_layer_set("x").unwrap();
    assert_eq!(last(&d), Some(yolu_core::HistoryKind::LayerSets));
    d.rename_layer_set(other, "y").unwrap();
    assert_eq!(last(&d), Some(yolu_core::HistoryKind::LayerSets));
    d.move_layer_set(other, 0).unwrap();
    assert_eq!(last(&d), Some(yolu_core::HistoryKind::LayerSets));
    d.set_channel_output(Channel::Color, Some(other)).unwrap();
    assert_eq!(last(&d), Some(yolu_core::HistoryKind::LayerSets));
    d.move_layers_to_set(&[a], other).unwrap();
    assert_eq!(last(&d), Some(yolu_core::HistoryKind::LayerSets));
    let copy = d.duplicate_layer_set(first, None).unwrap();
    assert_eq!(last(&d), Some(yolu_core::HistoryKind::LayerSets));
    d.remove_layer_set(copy).unwrap();
    assert_eq!(last(&d), Some(yolu_core::HistoryKind::LayerSets));
}

#[test]
fn a_named_batch_is_one_history_step_of_the_chosen_kind_and_rolls_back_on_failure() {
    let (mut d, _, _) = two_layers();
    let steps = d.undo_count();
    let (set, layer) = d
        .batch_as(yolu_core::HistoryKind::LayerSets, |d| {
            let set = d.add_layer_set("x")?;
            let layer = d.add_layer_in_set("l", set, None)?;
            Ok((set, layer))
        })
        .unwrap();
    assert_eq!(d.undo_count(), steps + 1, "足した 2 つの段が 1 段になる");
    assert_eq!(d.history().last(), Some(yolu_core::HistoryKind::LayerSets));
    assert_eq!(d.layer_set_of(layer).unwrap(), set);
    d.undo().unwrap();
    assert_eq!(d.layer_sets().len(), 1);
    assert!(d.layer(layer).is_none());
    d.redo().unwrap();
    assert_eq!(d.layer_sets().len(), 2);
    // 途中で断られたら、済んだ分も戻す（履歴は変わらない）
    let revision_steps = d.undo_count();
    let result: Result<(), CoreError> = d.batch_as(yolu_core::HistoryKind::LayerSets, |d| {
        d.add_layer_set("y")?;
        d.add_layer_in_set("m", LayerSetId(0x7777), None)?;
        Ok(())
    });
    assert_eq!(
        result,
        Err(CoreError::InvalidArgument(Reason::LayerSetDoesNotExist))
    );
    assert_eq!(d.undo_count(), revision_steps);
    assert_eq!(d.layer_sets().len(), 2, "足したレイヤーセットも戻る");
    // 段が 1 つだけなら、その段の種類のまま
    d.batch_as(yolu_core::HistoryKind::LayerSets, |d| {
        d.add_layer_in_set("only", set, None)
    })
    .unwrap();
    assert_eq!(d.history().last(), Some(yolu_core::HistoryKind::AddLayer));
}

#[test]
fn a_smart_material_placed_at_the_top_goes_into_the_chosen_set() {
    use yolu_core::smart::SmartPlacement;
    let (mut d, a, _) = two_layers();
    let first = d.layer_sets()[0].id;
    let other = d.add_layer_set("影のマスク").unwrap();
    let x = d.add_layer_in_set("x", other, None).unwrap();
    let material = d.capture_smart_material(&[a], "素材").unwrap();
    let in_set = |d: &Document, set| d.layers().iter().filter(|l| l.layer_set() == set).count();
    d.clear_history().unwrap();
    let (first_before, other_before) = (in_set(&d, first), in_set(&d, other));
    // set を渡せば、一番上の段のそのレイヤーセットの中の位置（x のすぐ上）に入る
    let placed = d
        .place_smart_material(
            &material,
            &SmartPlacement {
                set: Some(other),
                position: Some(1),
                ..SmartPlacement::default()
            },
        )
        .unwrap();
    assert_eq!(d.layer_set_of(placed.layer_id).unwrap(), other);
    assert_eq!(in_set(&d, first), first_before);
    assert_eq!(in_set(&d, other), other_before + 1);
    assert_eq!(
        d.children_in_set(other, None).unwrap(),
        [x, placed.layer_id]
    );
    assert_eq!(d.undo_count(), 1);
    // set が無ければ今までどおり最初のレイヤーセット
    let default = d
        .place_smart_material(&material, &SmartPlacement::default())
        .unwrap();
    assert_eq!(d.layer_set_of(default.layer_id).unwrap(), first);
    // グループの中へは、そのグループのレイヤーセットでなければ断り、何も変えない
    let group = d.add_group("g", Some(x)).unwrap();
    let steps = d.undo_count();
    assert_eq!(
        d.place_smart_material(
            &material,
            &SmartPlacement {
                parent: Some(group),
                set: Some(first),
                ..SmartPlacement::default()
            },
        )
        .err(),
        Some(CoreError::InvalidArgument(Reason::IntoGroupOfOtherLayerSet))
    );
    assert_eq!(
        d.place_smart_material(
            &material,
            &SmartPlacement {
                set: Some(LayerSetId(0x7777)),
                ..SmartPlacement::default()
            },
        )
        .err(),
        Some(CoreError::InvalidArgument(Reason::LayerSetDoesNotExist))
    );
    assert_eq!(d.undo_count(), steps);
}
