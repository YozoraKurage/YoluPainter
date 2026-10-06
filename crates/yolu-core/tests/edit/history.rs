use yolu_core::{BrushSettings, CoreError, Document, HistoryKind as K};

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
