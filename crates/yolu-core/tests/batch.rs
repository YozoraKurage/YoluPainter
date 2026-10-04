//! `Document::batch`（C# の `PaintDocument.Batch`）: 一続きの編集が 1 回の Undo になる・失敗は積んだ段を戻して何も残さない・何も記録しなければ履歴に
//! 足さない。塗りつぶしの層の画面（デカールを置く）がこの動きに頼る。

use yolu_core::fill_image::{Projection, ProjectionMode};
use yolu_core::{Channel, CoreError, Document, LayerLocks, Rgba8};

fn doc() -> Document {
    Document::new(32, 32).unwrap()
}

#[test]
fn a_batch_of_edits_is_one_undo_step() {
    let mut d = doc();
    let base = d.add_layer("base").unwrap();
    let before_layers = d.layers().len();
    let id = d
        .batch(|d| {
            let id = d.add_fill_layer(
                "fill",
                &[(Channel::Color, Rgba8::new(10, 20, 30, 255))],
                Some(base),
            )?;
            d.set_fill_projection(
                id,
                Projection {
                    mode: ProjectionMode::Planar,
                    ..Projection::default()
                },
                false,
            )?;
            d.set_layer_opacity(id, 0.5, false)?;
            Ok(id)
        })
        .unwrap();
    assert_eq!(d.layers().len(), before_layers + 1);
    assert_eq!(
        d.layer(id).unwrap().projection().mode,
        ProjectionMode::Planar
    );
    assert_eq!(d.layer(id).unwrap().opacity(), 0.5);
    // 1 回の Undo で、層ごと戻る
    assert!(d.undo().unwrap());
    assert_eq!(d.layers().len(), before_layers);
    assert!(d.layer(id).is_none());
    // やり直しで同じ層（同じ ID・設定）が戻る
    assert!(d.redo().unwrap());
    assert_eq!(
        d.layer(id).unwrap().projection().mode,
        ProjectionMode::Planar
    );
    assert_eq!(d.layer(id).unwrap().opacity(), 0.5);
    // 次の Undo はバッチの前の編集（base を足したこと）
    assert!(d.undo().unwrap());
    assert!(d.undo().unwrap());
    assert!(d.layer(base).is_none());
}

#[test]
fn a_failing_batch_changes_nothing_and_leaves_no_step() {
    let mut d = doc();
    let base = d.add_layer("base").unwrap();
    d.set_layer_locks(base, LayerLocks::ALL).unwrap();
    let steps = d.undo_count();
    let layers = d.layers().len();
    let undo_before = d.can_undo();
    // 2 つ目の編集がロックで断られる: 1 つ目（層を足す）も入らない
    let err = d
        .batch(|d| {
            d.add_layer("never")?;
            d.set_layer_opacity(base, 0.3, false)?;
            d.fill(
                base,
                Channel::Color,
                Rgba8::new(1, 2, 3, 255),
                1.0,
                None,
                false,
            )?;
            Ok(())
        })
        .unwrap_err();
    assert!(matches!(err, CoreError::LayerLocked { .. }), "{err:?}");
    assert_eq!(d.layers().len(), layers);
    assert_eq!(d.undo_count(), steps, "積んだ段は戻して残さない");
    assert_eq!(d.can_undo(), undo_before);
    assert_ne!(d.layer(base).unwrap().opacity(), 0.3);
}

#[test]
fn an_empty_batch_adds_no_history() {
    let mut d = doc();
    d.add_layer("a").unwrap();
    d.clear_history().unwrap();
    let r = d.batch(|_| Ok(7)).unwrap();
    assert_eq!(r, 7);
    assert!(!d.can_undo());
}

#[test]
fn a_batch_is_refused_while_a_stroke_runs() {
    let mut d = doc();
    let id = d.add_layer("a").unwrap();
    let stroke = d
        .begin_stroke(id, &yolu_core::BrushSettings::default())
        .unwrap();
    let err = d.batch(|_| Ok(())).unwrap_err();
    assert_eq!(err, CoreError::StrokeActive);
    d.cancel_stroke(stroke);
}
