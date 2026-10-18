use yolu_core::{
    BlendMode, BrushSettings, Channel, CoreError, Document, HistoryKind as K, LayerId, Rgba8,
};

fn history(d: &Document) -> Vec<K> {
    d.history().collect()
}

#[test]
fn kinds_follow_chronological_order_through_undo_redo_and_branching() {
    let mut d = Document::new(16, 16).unwrap();
    let id = d.add_layer("test").unwrap();
    d.set_layer_opacity(id, 0.5, false).unwrap();
    let mut s = d.begin_stroke(id, &BrushSettings::default()).unwrap();
    s.apply_pixel(&mut d, 0, 0, 1.0, 1.0).unwrap();
    d.end_stroke(s).unwrap();
    let expected = vec![K::AddLayer, K::LayerProperties, K::Brush];
    assert_eq!(history(&d), expected);
    let bytes = d.history_bytes();
    for position in (0..3).rev() {
        assert!(d.undo().unwrap());
        assert_eq!(d.undo_count(), position);
        assert_eq!(history(&d), expected);
        assert_eq!(d.history_bytes(), bytes);
    }
    for position in 1..=3 {
        assert!(d.redo().unwrap());
        assert_eq!(d.undo_count(), position);
        assert_eq!(history(&d), expected);
    }
    d.undo().unwrap();
    d.add_layer("branch").unwrap();
    assert_eq!(
        history(&d),
        vec![K::AddLayer, K::LayerProperties, K::AddLayer]
    );
    assert_eq!(d.redo_count(), 0);
}

#[test]
fn batch_is_one_kind_and_preserves_cost() {
    let mut d = Document::new(16, 16).unwrap();
    d.batch(|d| {
        d.add_layer("a")?;
        d.add_layer("b")?;
        Ok(())
    })
    .unwrap();
    assert_eq!(history(&d), vec![K::Batch]);
    assert_eq!(d.history_bytes(), 256);
    d.undo().unwrap();
    assert!(d.layers().is_empty());
    assert_eq!(history(&d), vec![K::Batch]);
    d.redo().unwrap();
    assert_eq!(d.layers().len(), 2);
}

#[test]
fn coalescing_keeps_one_kind_and_original_cost() {
    let mut d = Document::new(16, 16).unwrap();
    let id = d.add_layer("a").unwrap();
    d.set_layer_opacity(id, 0.7, true).unwrap();
    let bytes = d.history_bytes();
    d.set_layer_opacity(id, 0.3, true).unwrap();
    assert_eq!(history(&d), vec![K::AddLayer, K::LayerProperties]);
    assert_eq!(bytes, d.history_bytes());
    d.undo().unwrap();
    assert_eq!(d.layer(id).unwrap().opacity(), 1.0);
    d.redo().unwrap();
    assert_eq!(d.layer(id).unwrap().opacity(), 0.3);
}

#[test]
fn eviction_keeps_kinds_aligned_and_clear_removes_them() {
    let mut d = Document::new(16, 16).unwrap();
    let id = d.add_layer("a").unwrap();
    d.set_layer_opacity(id, 0.5, false).unwrap();
    let property_cost = d.history_bytes() - 128;
    d.set_undo_budget_bytes(property_cost).unwrap();
    assert_eq!(history(&d), vec![K::LayerProperties]);
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!(history(&d), vec![K::LayerProperties]);
    d.set_undo_budget_bytes(0).unwrap();
    assert!(history(&d).is_empty());
    d.add_layer("b").unwrap();
    d.clear_history().unwrap();
    assert!(history(&d).is_empty());
}

#[test]
fn failed_batch_restores_kinds_and_redo() {
    let mut d = Document::new(16, 16).unwrap();
    d.add_layer("a").unwrap();
    d.undo().unwrap();
    let bytes = d.history_bytes();
    let result: Result<(), CoreError> = d.batch(|d| {
        d.add_layer("b")?;
        Err(CoreError::BatchActive)
    });
    assert!(result.is_err());
    assert_eq!(history(&d), vec![K::AddLayer]);
    assert_eq!(d.redo_count(), 1);
    assert_eq!(d.history_bytes(), bytes);
}

#[test]
fn effect_add_edit_remove_and_layer_remove_are_distinct() {
    use yolu_core::{Channel, EffectSettings, FilterSpec, FilterTarget};
    let mut d = Document::new(16, 16).unwrap();
    let id = d.add_layer("a").unwrap();
    let effect = d
        .add_filter(
            id,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(1)).channels(&[Channel::Color]),
        )
        .unwrap();
    d.set_filter_strength(id, effect, 0.5, true).unwrap();
    d.set_filter_strength(id, effect, 0.3, true).unwrap();
    d.remove_filter(id, effect).unwrap();
    d.remove_layer(id).unwrap();
    let expected = vec![
        K::AddLayer,
        K::AddEffect,
        K::Effect,
        K::RemoveEffect,
        K::RemoveLayer,
    ];
    assert_eq!(history(&d), expected);
    while d.undo().unwrap() {
        assert_eq!(history(&d), expected);
    }
    while d.redo().unwrap() {
        assert_eq!(history(&d), expected);
    }
}

#[test]
fn empty_batch_and_cancelled_coalescing_leave_no_extra_kinds() {
    let mut d = Document::new(16, 16).unwrap();
    let id = d.add_layer("a").unwrap();
    d.set_layer_opacity(id, 0.4, true).unwrap();
    d.cancel_coalescing().unwrap();
    assert_eq!(history(&d), vec![K::AddLayer]);
    d.undo().unwrap();
    d.batch(|_| Ok(())).unwrap();
    assert_eq!(history(&d), vec![K::AddLayer]);
    assert_eq!(d.redo_count(), 1);
    assert_eq!(K::default(), K::Other);
}

/// 通過のグループの上に、Metallic の面を持たないクリッピングのレイヤー（面を持つとグループが分離になり、合成が変わる）。
fn clipped_over_pass_through_group() -> (Document, LayerId) {
    let mut d = Document::new(32, 32).unwrap();
    let bg = d.add_layer("bg").unwrap();
    let child = d.add_layer("child").unwrap();
    for y in 0..32 {
        for x in 0..32 {
            d.set_channel_pixel(bg, Channel::Metallic, x, y, Rgba8::new(200, 180, 160, 180))
                .unwrap();
            d.set_channel_pixel(child, Channel::Metallic, x, y, Rgba8::new(90, 40, 10, 120))
                .unwrap();
        }
    }
    d.set_layer_blend_mode(child, BlendMode::Multiply).unwrap();
    d.set_layer_opacity(child, 0.6, false).unwrap();
    d.group_layers(&[child], "g").unwrap();
    let clip = d.add_layer("clip").unwrap();
    d.set_layer_clipping(clip, true).unwrap();
    d.clear_history().unwrap();
    (d, clip)
}

fn metallic(d: &Document) -> Vec<u8> {
    d.composite_channel(Channel::Metallic, d.bounds()).unwrap()
}

fn line(d: &mut Document, layer: LayerId, color: Rgba8) -> bool {
    let brush = BrushSettings {
        radius: 4.0,
        color,
        ..BrushSettings::default()
    };
    let mut s = d.begin_stroke_in(layer, Channel::Metallic, &brush).unwrap();
    s.add_point(d, 4.0, 4.0, 1.0, yolu_core::glam::DVec2::ZERO)
        .unwrap();
    s.add_point(d, 26.0, 20.0, 1.0, yolu_core::glam::DVec2::ZERO)
        .unwrap();
    d.end_stroke(s).unwrap().changed
}

/// 画素を変えないストローク（アルファ 0 の色）は、始めに作った面を残さず、段も積まず、合成も変えない。
#[test]
fn a_stroke_that_changes_no_pixels_leaves_no_new_surface() {
    let (mut d, clip) = clipped_over_pass_through_group();
    let before = metallic(&d);
    assert!(!line(&mut d, clip, Rgba8::new(49, 156, 239, 0)));
    assert_eq!(d.undo_count(), 0);
    let l = d.layer(clip).unwrap();
    assert!(l.surface(Channel::Metallic).is_none());
    assert!(!l.is_channel_enabled(Channel::Metallic));
    assert_eq!(metallic(&d), before);
}

/// 面を作ったストロークの取り消しは、面を外してチャンネルを無効に戻し、合成をバイトまで戻す。やり直しは描いた後と同じ。
#[test]
fn undoing_a_stroke_that_created_its_surface_restores_the_composite() {
    let (mut d, clip) = clipped_over_pass_through_group();
    let before = metallic(&d);
    assert!(line(&mut d, clip, Rgba8::new(240, 240, 240, 255)));
    assert_eq!(d.undo_count(), 1);
    let drawn = metallic(&d);
    assert_ne!(drawn, before);
    assert!(d.undo().unwrap());
    let l = d.layer(clip).unwrap();
    assert!(l.surface(Channel::Metallic).is_none());
    assert!(!l.is_channel_enabled(Channel::Metallic));
    assert_eq!(metallic(&d), before);
    assert!(d.redo().unwrap());
    assert!(d.layer(clip).unwrap().is_channel_enabled(Channel::Metallic));
    assert_eq!(metallic(&d), drawn);
}

/// やり直しが予算に断られたら、やり直しで作り直した面も有効も残さず、段は Redo に残る（断った後は何も変わっていない）。
#[test]
fn a_redo_refused_by_the_budget_leaves_no_surface_behind() {
    let (mut d, clip) = clipped_over_pass_through_group();
    let before = metallic(&d);
    assert!(line(&mut d, clip, Rgba8::new(240, 240, 240, 255)));
    assert!(d.undo().unwrap());
    // 今の画素の量ちょうどにする: 線のタイルを戻せない
    d.set_source_budget_bytes(d.allocated_bytes()).unwrap();
    let revision = d.revision();
    assert!(d.redo().is_err());
    let l = d.layer(clip).unwrap();
    assert!(l.surface(Channel::Metallic).is_none());
    assert!(!l.is_channel_enabled(Channel::Metallic));
    assert_eq!((d.undo_count(), d.redo_count()), (0, 1));
    assert_eq!(d.revision(), revision);
    assert_eq!(metallic(&d), before);
    // 予算を戻せば、同じ段をやり直せる
    d.set_source_budget_bytes(u64::MAX / 2).unwrap();
    assert!(d.redo().unwrap());
    assert!(d.layer(clip).unwrap().is_channel_enabled(Channel::Metallic));
}

/// 面を作ったストロークを取り消して（キャンセル）も、面は残らない。
#[test]
fn cancelling_a_stroke_that_created_its_surface_removes_it() {
    let (mut d, clip) = clipped_over_pass_through_group();
    let before = metallic(&d);
    let brush = BrushSettings {
        radius: 4.0,
        ..BrushSettings::default()
    };
    let mut s = d.begin_stroke_in(clip, Channel::Metallic, &brush).unwrap();
    s.add_point(&mut d, 4.0, 4.0, 1.0, yolu_core::glam::DVec2::ZERO)
        .unwrap();
    d.cancel_stroke(s);
    assert!(d.layer(clip).unwrap().surface(Channel::Metallic).is_none());
    assert_eq!(d.undo_count(), 0);
    assert_eq!(metallic(&d), before);
}
