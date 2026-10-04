//! スマートマテリアルと効果: 置くたびに段・Anchor が新しい ID、大きさ違いでは半径を倍率に合わせ、決まりに収まらなければ断る。
mod attach_support;
use attach_support::*;
use yolu_core::paths::{CanvasPath, CanvasPoint, PathBrush};
use yolu_core::smart::SmartPlacement;
use yolu_core::{
    AnchorPlacement, BrushSettings, Channel, CoreError, Document, EffectSettings, FilterId,
    FilterSpec, FilterTarget, LayerPath,
};

fn blur(doc: &mut Document, layer: yolu_core::LayerId, radius: u32) -> FilterId {
    doc.add_filter(
        layer,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(radius)).channels(&[Channel::Color]),
    )
    .unwrap()
}

fn filter_ids(doc: &Document) -> Vec<FilterId> {
    doc.layers()
        .iter()
        .flat_map(|l| {
            l.filters()
                .iter()
                .chain(l.mask().into_iter().flat_map(|m| m.filters().iter()))
        })
        .map(|e| e.id())
        .collect()
}

#[test]
fn placing_twice_gives_every_stage_and_anchor_its_own_id_and_drops_model_paths() {
    let (mut doc, l) = world();
    let base = l[0];
    blur(&mut doc, base, 3);
    doc.add_layer_mask(base).unwrap();
    doc.add_filter(
        base,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::noise(0.2, 1, true)),
    )
    .unwrap();
    doc.add_anchor(base, AnchorPlacement::Layer, None, None)
        .unwrap();
    doc.add_anchor(base, AnchorPlacement::Mask, None, None)
        .unwrap();
    let material = doc.capture_smart_material(&[base], "効果つき").unwrap();
    assert!(material.layers().iter().all(|l| l.path().is_none()));
    let first = doc
        .place_smart_material(&material, &SmartPlacement::default())
        .unwrap();
    let second = doc
        .place_smart_material(&material, &SmartPlacement::default())
        .unwrap();
    assert_ne!(first.layer_id, second.layer_id);
    let ids = filter_ids(&doc);
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(ids.len(), 6, "元と 2 つの写しに、段が 2 つずつ");
    assert_eq!(unique.len(), ids.len(), "段の ID は重ならない");
    let anchors = doc.anchors();
    let mut anchor_ids: Vec<_> = anchors.iter().map(|a| a.anchor.id()).collect();
    anchor_ids.sort();
    anchor_ids.dedup();
    assert_eq!(anchors.len(), 6);
    assert_eq!(anchor_ids.len(), 6, "Anchor の ID も重ならない");
    // 1 回の Undo で外れる
    doc.undo().unwrap();
    assert_eq!(filter_ids(&doc).len(), 4);
}

#[test]
fn a_material_at_another_size_scales_blur_and_sharpen_radii_and_keeps_only_pixels_of_paths() {
    let (mut source, l) = world();
    let base = l[0];
    blur(&mut source, base, 8);
    source
        .add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::sharpen(3, 1.0, 0)).channels(&[Channel::Color]),
        )
        .unwrap();
    source
        .add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::noise(0.3, 5, true)).channels(&[Channel::Color]),
        )
        .unwrap();
    let path = CanvasPath {
        id: 1,
        channel: Channel::Color,
        brush: PathBrush(BrushSettings {
            radius: 2.0,
            ..BrushSettings::default()
        }),
        points: vec![
            CanvasPoint::new(3.0, 3.0, 1.0).unwrap(),
            CanvasPoint::new(30.0, 20.0, 1.0).unwrap(),
        ],
        material: None,
    };
    source.set_canvas_path(base, path).unwrap();
    let material = source.capture_smart_material(&[base], "大きさ").unwrap();
    assert!(
        material.notes().is_empty(),
        "2D のパスは持ち出せる（モデルの上のパスだけが画素になる）: {:?}",
        material.notes()
    );
    for (w, h, radius, sharpen) in [
        (80, 56, 16, 6),
        (20, 14, 4, 2),
        (40, 28, 8, 3),
        (400, 280, 80, 30),
    ] {
        let mut doc = Document::with_tile_size(w, h, 8).unwrap();
        let placed = doc
            .place_smart_material(&material, &SmartPlacement::default())
            .unwrap();
        let layer = doc.layer(placed.layer_id).unwrap();
        let radii: Vec<u32> = layer
            .filters()
            .iter()
            .filter_map(|e| match e.settings() {
                EffectSettings::Filter(yolu_core::filter::Settings::GaussianBlur { radius }) => {
                    Some(*radius)
                }
                EffectSettings::Filter(yolu_core::filter::Settings::Sharpen { radius, .. }) => {
                    Some(*radius)
                }
                _ => None,
            })
            .collect();
        assert_eq!(radii, vec![radius, sharpen], "{w}×{h}");
        assert_eq!(layer.filters().len(), 3, "雑音は半径が無いのでそのまま");
        // 大きさが違う画布では、パスが画素だけになったことを言う（半径は丸めていないので、半径の知らせは無い）
        if (w, h) == (40, 28) {
            assert!(layer.path().is_some(), "同じ大きさならパスのまま");
            assert!(placed.notes.is_empty(), "{:?}", placed.notes);
        } else {
            assert!(layer.path().is_none(), "{w}×{h}: 点が元の大きさのもの");
            assert_eq!(placed.notes.len(), 1, "{w}×{h}: {:?}", placed.notes);
            assert!(placed.notes[0].contains("画素だけ"), "{:?}", placed.notes);
        }
        assert!(doc.undo().unwrap());
        assert!(doc.layers().is_empty());
    }
    // 半径が最大を超えるなら最大、1 を割るなら 1
    let (mut small, l) = world();
    blur(&mut small, l[0], 1);
    let tiny = small.capture_smart_material(&[l[0]], "小さい").unwrap();
    let mut doc = Document::with_tile_size(20, 14, 8).unwrap();
    let placed = doc
        .place_smart_material(&tiny, &SmartPlacement::default())
        .unwrap();
    assert!(matches!(
        doc.layer(placed.layer_id).unwrap().filters()[0].settings(),
        EffectSettings::Filter(yolu_core::filter::Settings::GaussianBlur { radius: 1 })
    ));
}

#[test]
fn placing_tells_which_radii_were_rounded_to_the_limit_and_which_paths_became_pixels() {
    // 最大を超える半径: ぼかし 200 を 2 倍の画布へ（400 → 256）、シャープ 40 を 3 倍の画布へ（120 → 64）
    let (mut source, l) = world();
    blur(&mut source, l[0], 200);
    source
        .add_filter(
            l[1],
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::sharpen(40, 1.0, 0)).channels(&[Channel::Height]),
        )
        .unwrap();
    let big = source.capture_smart_material(&[l[0]], "広い").unwrap();
    let sharp = source.capture_smart_material(&[l[1]], "鋭い").unwrap();
    let mut doc = Document::with_tile_size(W * 2, H * 2, 8).unwrap();
    let placed = doc
        .place_smart_material(&big, &SmartPlacement::default())
        .unwrap();
    assert_eq!(placed.notes.len(), 1, "{:?}", placed.notes);
    assert!(
        placed.notes[0].contains("256") && placed.notes[0].contains("最大"),
        "{:?}",
        placed.notes
    );
    let mut doc = Document::with_tile_size(W * 3, H * 3, 8).unwrap();
    let placed = doc
        .place_smart_material(&sharp, &SmartPlacement::default())
        .unwrap();
    assert_eq!(placed.notes.len(), 1, "{:?}", placed.notes);
    assert!(
        placed.notes[0].contains("64") && placed.notes[0].contains("最大"),
        "{:?}",
        placed.notes
    );
    // 1 を割る半径は 1 へ
    let (mut small, l) = world();
    blur(&mut small, l[0], 1);
    let tiny = small.capture_smart_material(&[l[0]], "小さい").unwrap();
    let mut doc = Document::with_tile_size(W / 4, H / 4, 8).unwrap();
    let placed = doc
        .place_smart_material(&tiny, &SmartPlacement::default())
        .unwrap();
    assert_eq!(placed.notes.len(), 1, "{:?}", placed.notes);
    assert!(placed.notes[0].contains("最小"), "{:?}", placed.notes);
    // 同じ大きさなら、何も変えないので知らせも無い
    let mut same = Document::with_tile_size(W, H, 8).unwrap();
    let placed = same
        .place_smart_material(&big, &SmartPlacement::default())
        .unwrap();
    assert!(placed.notes.is_empty());
    // スマートマスクも同じ（マスクのフィルターの半径）
    let (mut with_mask, l) = world();
    with_mask.add_layer_mask(l[0]).unwrap();
    with_mask
        .add_filter(
            l[0],
            FilterTarget::Mask,
            FilterSpec::new(EffectSettings::blur(200)),
        )
        .unwrap();
    let mask = with_mask
        .capture_smart_mask(l[0], "ぼかしのマスク")
        .unwrap();
    let mut target = Document::with_tile_size(W * 2, H * 2, 8).unwrap();
    let layer = target.add_layer("先").unwrap();
    let applied = target.apply_smart_mask(&mask, layer, None).unwrap();
    assert_eq!(applied.notes.len(), 1, "{:?}", applied.notes);
    assert!(applied.notes[0].contains("最大"), "{:?}", applied.notes);
}

#[test]
fn a_stack_that_does_not_fit_the_destination_is_refused_before_anything_changes() {
    let (mut source, l) = world();
    for _ in 0..3 {
        blur(&mut source, l[0], 100);
    }
    let material = source.capture_smart_material(&[l[0]], "広い").unwrap();
    // 3 倍の大きさへ: 半径が 300 になり、合計 900 > 512
    let mut doc = Document::with_tile_size(W * 3, H * 3, 8).unwrap();
    let before = (doc.layers().len(), doc.undo_count());
    let r = doc.place_smart_material(&material, &SmartPlacement::default());
    assert!(matches!(r, Err(CoreError::Unsupported(_))), "{r:?}");
    assert_eq!((doc.layers().len(), doc.undo_count()), before);
    // 同じ大きさなら置ける
    let mut same = Document::with_tile_size(W, H, 8).unwrap();
    same.place_smart_material(&material, &SmartPlacement::default())
        .unwrap();
}

#[test]
fn a_surface_path_is_dropped_when_a_material_is_captured() {
    use yolu_core::paths::{fingerprint, PathPoint, SurfacePath};
    let (mut doc, _) = world();
    let layer = doc.add_layer("面").unwrap();
    doc.set_channel_enabled(layer, Channel::Height, true)
        .unwrap();
    let g = {
        use yolu_core::geometry::{SurfaceGeometry, SurfaceTriangle, DEFAULT_WELD_TOLERANCE};
        use yolu_core::glam::{Vec2, Vec3};
        SurfaceGeometry::new(
            vec![SurfaceTriangle::new(
                Vec3::ZERO,
                Vec3::X,
                Vec3::new(1.0, 1.0, 0.0),
                Vec2::ZERO,
                Vec2::X,
                Vec2::ONE,
            )],
            1,
            DEFAULT_WELD_TOLERANCE,
        )
        .unwrap()
    };
    let path = SurfacePath {
        id: 3,
        channel: Channel::Height,
        brush: PathBrush(BrushSettings {
            radius: 0.1,
            ..BrushSettings::default()
        }),
        points: vec![PathPoint::new(0, 0.3, 0.2, 1.0).unwrap()],
        model_fingerprint: fingerprint(&g),
        material: None,
    };
    let drawn = yolu_core::paths::render_surface(
        &path,
        &g,
        &yolu_core::paths::Options {
            width: W,
            height: H,
            tile_size: 8,
            ..Default::default()
        },
    )
    .unwrap();
    doc.set_path(layer, LayerPath::Surface(path), drawn.channels)
        .unwrap();
    let material = doc.capture_smart_material(&[layer], "面のパス").unwrap();
    assert!(
        material.layers()[0].path().is_none(),
        "モデルの三角形に結び付いたパスは持ち出さない"
    );
    assert_eq!(material.notes().len(), 1, "{:?}", material.notes());
    assert!(
        material.notes()[0].contains("面") && material.notes()[0].contains("画素だけ"),
        "どの層のパスが画素だけになったかを言う: {:?}",
        material.notes()
    );
    assert!(
        material.layers()[0]
            .surface(Channel::Height)
            .unwrap()
            .tile_count()
            > 0,
        "画素は残る"
    );
}

/// マスクにフィルターと Anchor を持つ層（土台）と、マスクの無い層（中・上）。
fn mask_world() -> (Document, Vec<yolu_core::LayerId>) {
    let (mut doc, l) = world();
    let base = l[0];
    doc.add_layer_mask(base).unwrap();
    mask_stroke(&mut doc, base, 10.0);
    doc.add_filter(
        base,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::blur(4)),
    )
    .unwrap();
    doc.add_anchor(base, AnchorPlacement::Mask, None, None)
        .unwrap();
    (doc, l)
}

fn mask_stroke(doc: &mut Document, layer: yolu_core::LayerId, x: f64) {
    let mut s = doc
        .begin_mask_stroke(layer, &BrushSettings::default())
        .unwrap();
    for (dx, dy) in [(0.0, 5.0), (8.0, 10.0), (16.0, 14.0)] {
        s.add_point(doc, x + dx, dy, 1.0, yolu_core::glam::DVec2::ZERO)
            .unwrap();
    }
    doc.end_stroke(s).unwrap();
}

#[test]
fn a_smart_mask_gets_its_own_stage_and_anchor_ids_every_time_it_is_placed() {
    let (mut doc, l) = mask_world();
    let (base, mid, top) = (l[0], l[1], l[2]);
    let mask = doc.capture_smart_mask(base, "ぼかしのマスク").unwrap();
    let before = filter_ids(&doc);
    doc.apply_smart_mask(&mask, mid, None).unwrap();
    doc.apply_smart_mask(&mask, top, None).unwrap();
    // 元の層へ戻す（マスクを置き換える）
    doc.apply_smart_mask(&mask, base, None).unwrap();
    let ids = filter_ids(&doc);
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(ids.len(), 3, "マスクのフィルターは層ごとに 1 つ");
    assert_eq!(unique.len(), ids.len(), "段の ID は重ならない: {ids:?}");
    assert!(
        !ids.contains(&before[0]),
        "元の層へ戻しても、元と同じ ID にはならない"
    );
    let anchors: Vec<_> = doc.anchors().iter().map(|a| a.anchor.id()).collect();
    let mut unique = anchors.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(anchors.len(), 3);
    assert_eq!(unique.len(), anchors.len(), "Anchor の ID も重ならない");
    // 同じ ID の段を探すと、それを持つ層が見つかる（先頭の 1 つに化けない）
    for id in &ids {
        assert!(doc.find_filter(*id).is_some());
    }
    // 1 回ずつの Undo で戻る
    for _ in 0..3 {
        assert!(doc.undo().unwrap());
    }
    assert_eq!(filter_ids(&doc), before);
}

#[test]
fn replacing_a_mask_with_a_smart_mask_never_shows_a_stale_result() {
    let (mut doc, l) = mask_world();
    let base = l[0];
    let captured = doc.capture_smart_mask(base, "捕まえたマスク").unwrap();
    let colors = [Channel::Color, Channel::Height];
    let cached_equals_fresh = |doc: &Document, what: &str| {
        let now: Vec<_> = colors.iter().map(|c| whole(doc, *c)).collect();
        doc.release_effect_cache();
        let fresh: Vec<_> = colors.iter().map(|c| whole(doc, *c)).collect();
        assert!(now == fresh, "{what}: キャッシュの合成が評価し直しと違う");
    };
    cached_equals_fresh(&doc, "最初");
    // 捕まえた後にマスクを塗り足し、捕まえたマスクを置き直す（画素だけが入れ替わる）
    mask_stroke(&mut doc, base, 22.0);
    cached_equals_fresh(&doc, "塗り足した後");
    let touched = whole(&doc, Channel::Color);
    doc.apply_smart_mask(&captured, base, None).unwrap();
    cached_equals_fresh(&doc, "置き直した後");
    assert_ne!(
        whole(&doc, Channel::Color),
        touched,
        "マスクの画素が替わったので合成が変わる"
    );
    assert!(doc.undo().unwrap());
    cached_equals_fresh(&doc, "Undo");
    assert_eq!(whole(&doc, Channel::Color), touched);
    assert!(doc.redo().unwrap());
    cached_equals_fresh(&doc, "Redo");
    // 変化の記録: 入れ替えの後の変化は、塗った所を含む
    let since = doc.change_serial();
    assert!(doc.undo().unwrap());
    assert!(
        !doc.changed_tiles(Channel::Color, since).unwrap().is_empty(),
        "マスクの入れ替えを戻すと、変わったタイルが記録される"
    );
}
