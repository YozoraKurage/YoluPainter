//! 文書の振る舞い（Unity 版の C# の Core の試験 CoreTests・ClippingTests・ChangeTrackingTests・IntegrationTests・
//! BrushDynamicsTests の M1 の範囲を移したもの。値は C# の試験の期待値そのもの）。
#![allow(clippy::chunks_exact_to_as_chunks)]

use sha2::{Digest, Sha256};
use yolu_core::glam::DVec2;
use yolu_core::{
    BlendMode, BrushSample, BrushSettings, Channel, CoreError, Document, LayerId, Rect, Rgba8,
    RowOrder, TileCoord,
};

const RED: Rgba8 = Rgba8::new(255, 0, 0, 255);

/// C# の試験の Opaque(c): 半径 2・硬さ 1・間隔 0.5・筆圧の割り当てなし。
fn opaque(c: Rgba8) -> BrushSettings {
    BrushSettings {
        radius: 2.0,
        hardness: 1.0,
        spacing: 0.5,
        color: c,
        pressure_size: false,
        pressure_opacity: false,
        ..BrushSettings::default()
    }
}

/// C# の試験の Pixel: ApplyPixel(x, y, 1) を 1 回のストロークで確定する。
fn pixel(doc: &mut Document, layer: LayerId, x: i64, y: i64, c: Rgba8) {
    let mut s = doc.begin_stroke(layer, &opaque(c)).unwrap();
    s.apply_pixel(doc, x, y, 1.0, 1.0).unwrap();
    doc.end_stroke(s).unwrap();
}

fn px(doc: &Document, layer: LayerId, x: u32, y: u32) -> Rgba8 {
    doc.layer(layer)
        .unwrap()
        .pixel(Channel::Color, x, y)
        .unwrap()
}
fn comp(doc: &Document, x: u32, y: u32) -> Rgba8 {
    doc.composite_pixel(Channel::Color, x, y).unwrap()
}
fn all(doc: &Document) -> Vec<u8> {
    doc.composite(doc.bounds()).unwrap()
}
fn small() -> (Document, LayerId) {
    let mut doc = Document::with_tile_size(16, 16, 4).unwrap();
    let l = doc.add_layer("L").unwrap();
    doc.clear_history().unwrap();
    (doc, l)
}

// ───────── CoreTests ─────────

#[test]
fn empty_layers_allocate_nothing() {
    let mut doc = Document::new(4096, 4096).unwrap();
    for i in 0..100 {
        doc.add_layer(&format!("L{i}")).unwrap();
    }
    assert_eq!(doc.allocated_bytes(), 0);
    assert!(doc
        .layers()
        .iter()
        .all(|l| l.surface(Channel::Color).unwrap().tile_count() == 0));
}

#[test]
fn pixel_erase_and_undo() {
    let (mut doc, l) = small();
    pixel(&mut doc, l, 5, 9, RED);
    let surface = doc.layer(l).unwrap().surface(Channel::Color).unwrap();
    assert_eq!((surface.tile_count(), doc.allocated_bytes()), (1, 64));
    let mut s = doc
        .begin_stroke(
            l,
            &BrushSettings {
                erase: true,
                ..opaque(RED)
            },
        )
        .unwrap();
    s.apply_pixel(&mut doc, 5, 9, 1.0, 1.0).unwrap();
    doc.end_stroke(s).unwrap();
    assert_eq!(
        doc.layer(l)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .tile_count(),
        0
    );
    assert!(doc.undo().unwrap());
    assert_eq!(px(&doc, l, 5, 9), RED);
}

#[test]
fn transparent_rgb_survives_a_stroke_and_undo() {
    let (mut doc, l) = small();
    doc.set_pixel(l, 1, 1, Rgba8::new(32, 64, 128, 0)).unwrap();
    pixel(&mut doc, l, 1, 1, Rgba8::new(220, 4, 8, 255));
    doc.undo().unwrap();
    assert_eq!(px(&doc, l, 1, 1), Rgba8::new(32, 64, 128, 0));
}

#[test]
fn stroke_undo_redo_across_tiles() {
    let (mut doc, l) = small();
    let before = all(&doc);
    let brush = BrushSettings {
        color: Rgba8::new(240, 13, 9, 127),
        ..BrushSettings::default()
    };
    let mut s = doc.begin_stroke(l, &brush).unwrap();
    s.add_sample(
        &mut doc,
        BrushSample::new(2.5, 3.5, 1.0, 0.0, DVec2::ZERO).unwrap(),
    )
    .unwrap();
    s.add_sample(
        &mut doc,
        BrushSample::new(12.5, 10.5, 1.0, 1.0, DVec2::ZERO).unwrap(),
    )
    .unwrap();
    doc.end_stroke(s).unwrap();
    let painted = all(&doc);
    assert_ne!(painted, before);
    assert_eq!(doc.undo_count(), 1);
    doc.undo().unwrap();
    assert_eq!(all(&doc), before);
    assert_eq!(doc.allocated_bytes(), 0);
    doc.redo().unwrap();
    assert_eq!(all(&doc), painted);
}

#[test]
fn a_dropped_stroke_changes_nothing() {
    let (mut doc, l) = small();
    pixel(&mut doc, l, 3, 3, Rgba8::new(5, 19, 82, 79));
    doc.clear_history().unwrap();
    let before = all(&doc);
    let mut s = doc.begin_stroke(l, &opaque(RED)).unwrap();
    s.add_point(&mut doc, 3.5, 3.5, 1.0, DVec2::ZERO).unwrap();
    assert_ne!(all(&doc), before); // 描いている途中も合成で見える
    let _ = s; // 札を落とした: 文書は進行中のまま。app はフォーカス喪失などで取り消す
    assert!(doc.has_active_stroke());
    assert!(doc.cancel_active_stroke());
    assert_eq!(all(&doc), before);
    assert!(!doc.has_active_stroke());
    assert_eq!(doc.undo_count(), 0);
}

#[test]
fn a_commit_without_change_keeps_redo() {
    let (mut doc, l) = small();
    pixel(&mut doc, l, 1, 1, RED);
    doc.undo().unwrap();
    let mut s = doc.begin_stroke(l, &opaque(Rgba8::TRANSPARENT)).unwrap();
    s.apply_pixel(&mut doc, 1, 1, 1.0, 1.0).unwrap();
    assert!(!doc.end_stroke(s).unwrap().changed);
    assert!(doc.can_redo());
    pixel(&mut doc, l, 2, 2, RED);
    assert!(!doc.can_redo());
}

#[test]
fn pressure_opacity_and_flow_keep_straight_alpha() {
    let (mut doc, l) = small();
    let brush = BrushSettings {
        pressure_opacity: true,
        pressure_flow: true,
        ..opaque(RED)
    };
    let mut s = doc.begin_stroke(l, &brush).unwrap();
    s.apply_pixel(&mut doc, 4, 4, 1.0, 0.5).unwrap();
    doc.end_stroke(s).unwrap();
    let c = comp(&doc, 4, 4);
    assert_eq!((c.a, c.r), (64, 255));
}

#[test]
fn soft_brush_falloff_and_zero_pressure() {
    let (mut doc, l) = small();
    let brush = BrushSettings {
        radius: 4.0,
        hardness: 0.0,
        pressure_size: true,
        ..BrushSettings::default()
    };
    let mut s = doc.begin_stroke(l, &brush).unwrap();
    s.add_point(&mut doc, 8.5, 8.5, 0.0, DVec2::ZERO).unwrap();
    doc.end_stroke(s).unwrap();
    assert_eq!(doc.allocated_bytes(), 0);
    let mut s = doc.begin_stroke(l, &brush).unwrap();
    s.add_point(&mut doc, 8.5, 8.5, 1.0, DVec2::ZERO).unwrap();
    doc.end_stroke(s).unwrap();
    assert_eq!(
        (
            px(&doc, l, 8, 8).a,
            px(&doc, l, 10, 8).a,
            px(&doc, l, 12, 8).a
        ),
        (255, 128, 0)
    );
}

#[test]
fn collinear_packets_give_the_same_pixels() {
    let brush = BrushSettings {
        radius: 3.0,
        hardness: 0.3,
        spacing: 0.2,
        color: Rgba8::new(91, 23, 188, 71),
        pressure_opacity: true,
        pressure_size: true,
        ..BrushSettings::default()
    };
    let draw = |samples: &[(f64, f64, f64, f64)]| {
        let mut doc = Document::with_tile_size(64, 64, 8).unwrap();
        let l = doc.add_layer("L").unwrap();
        let mut s = doc.begin_stroke(l, &brush).unwrap();
        for &(x, y, p, t) in samples {
            s.add_sample(&mut doc, BrushSample::new(x, y, p, t, DVec2::ZERO).unwrap())
                .unwrap();
        }
        doc.end_stroke(s).unwrap();
        all(&doc)
    };
    let a = draw(&[(4.5, 8.5, 0.2, 0.0), (52.5, 32.5, 1.0, 1.0)]);
    let b: Vec<_> = (0..=12)
        .map(|i| {
            let i = i as f64;
            (4.5 + 4.0 * i, 8.5 + 2.0 * i, 0.2 + 0.8 * i / 12.0, i / 12.0)
        })
        .collect();
    assert_eq!(a, draw(&b));
}

#[test]
fn duplicate_positions_make_no_extra_dab() {
    let (mut doc, l) = small();
    let mut s = doc
        .begin_stroke(l, &opaque(Rgba8::new(255, 0, 0, 128)))
        .unwrap();
    s.add_sample(
        &mut doc,
        BrushSample::new(4.5, 4.5, 1.0, 0.0, DVec2::ZERO).unwrap(),
    )
    .unwrap();
    s.add_sample(
        &mut doc,
        BrushSample::new(4.5, 4.5, 1.0, 1.0, DVec2::ZERO).unwrap(),
    )
    .unwrap();
    let r = doc.end_stroke(s).unwrap();
    assert_eq!(r.stamps, 1);
    assert_eq!(px(&doc, l, 4, 4).a, 128);
}

#[test]
fn settings_are_frozen_and_edits_wait_for_the_stroke() {
    let (mut doc, l) = small();
    let mut brush = opaque(RED);
    let mut s = doc.begin_stroke(l, &brush).unwrap();
    brush.color = Rgba8::new(0, 0, 255, 255);
    let _ = brush;
    s.apply_pixel(&mut doc, 1, 1, 1.0, 1.0).unwrap();
    assert_eq!(doc.add_layer("x"), Err(CoreError::StrokeActive));
    assert_eq!(doc.undo(), Err(CoreError::StrokeActive));
    assert_eq!(doc.set_pixel(l, 2, 2, RED), Err(CoreError::StrokeActive));
    assert_eq!(
        doc.set_layer_opacity(l, 0.5, false),
        Err(CoreError::StrokeActive)
    );
    assert!(matches!(
        doc.begin_stroke(l, &brush),
        Err(CoreError::StrokeActive)
    ));
    doc.end_stroke(s).unwrap();
    assert_eq!(px(&doc, l, 1, 1), RED);
}

#[test]
fn time_must_not_go_back_and_a_failure_cancels() {
    let (mut doc, l) = small();
    let mut s = doc.begin_stroke(l, &opaque(RED)).unwrap();
    s.add_sample(
        &mut doc,
        BrushSample::new(4.0, 4.0, 1.0, 10.0, DVec2::ZERO).unwrap(),
    )
    .unwrap();
    assert!(doc.allocated_bytes() > 0);
    let r = s.add_sample(
        &mut doc,
        BrushSample::new(5.0, 4.0, 1.0, 9.0, DVec2::ZERO).unwrap(),
    );
    assert!(matches!(r, Err(CoreError::InvalidArgument(_))));
    assert!(!doc.has_active_stroke());
    assert_eq!(doc.allocated_bytes(), 0);
    assert_eq!(doc.end_stroke(s), Err(CoreError::NoActiveStroke));
    // 範囲外の座標・有限でない値も取り消す
    let mut s = doc.begin_stroke(l, &opaque(RED)).unwrap();
    assert!(s.add_point(&mut doc, 2e7, 0.0, 1.0, DVec2::ZERO).is_err());
    assert!(!doc.has_active_stroke());
    let mut s = doc.begin_stroke(l, &opaque(RED)).unwrap();
    assert!(s
        .add_point(&mut doc, f64::NAN, 0.0, 1.0, DVec2::ZERO)
        .is_err());
    assert!(!doc.has_active_stroke());
    // 間違った設定は始める前に断る
    assert!(doc
        .begin_stroke(
            l,
            &BrushSettings {
                spacing: 0.0,
                ..opaque(RED)
            }
        )
        .is_err());
    assert!(doc
        .begin_stroke(
            l,
            &BrushSettings {
                radius: f64::INFINITY,
                ..opaque(RED)
            }
        )
        .is_err());
    assert!(doc
        .begin_stroke(
            l,
            &BrushSettings {
                hardness: 1.5,
                ..opaque(RED)
            }
        )
        .is_err());
}

#[test]
fn history_budget_drops_the_oldest_step() {
    let (mut doc, l) = small();
    doc.set_undo_budget_bytes(150).unwrap();
    pixel(&mut doc, l, 0, 0, RED);
    pixel(&mut doc, l, 8, 8, Rgba8::new(0, 255, 0, 255));
    assert!(doc.history_bytes() <= 150);
    let (count, bytes) = doc.history_trimmed();
    assert!(count == 1 && bytes > 0);
    assert_eq!(doc.undo_count(), 1);
    doc.undo().unwrap();
    assert_eq!((px(&doc, l, 0, 0).a, px(&doc, l, 8, 8).a), (255, 0));
}

#[test]
fn minimum_undo_steps_survive_the_budget() {
    let (mut doc, l) = small();
    doc.set_undo_budget_bytes(1).unwrap();
    doc.set_minimum_undo_steps(2).unwrap();
    pixel(&mut doc, l, 0, 0, RED);
    pixel(&mut doc, l, 8, 8, RED);
    assert_eq!((doc.undo_count(), doc.history_trimmed().0), (2, 0));
    pixel(&mut doc, l, 4, 4, RED);
    assert_eq!(doc.undo_count(), 2);
    assert!(doc.history_trimmed().0 == 1 && doc.history_trimmed().1 > 0);
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(
        (
            px(&doc, l, 4, 4).a,
            px(&doc, l, 8, 8).a,
            px(&doc, l, 0, 0).a
        ),
        (0, 0, 255)
    );
    assert!(!doc.undo().unwrap());
    doc.redo().unwrap();
    doc.redo().unwrap();
    doc.set_minimum_undo_steps(0).unwrap();
    assert_eq!((doc.undo_count(), doc.history_bytes()), (0, 0));
}

#[test]
fn a_strict_budget_keeps_the_pixels() {
    let (mut doc, l) = small();
    doc.set_undo_budget_bytes(1).unwrap();
    pixel(&mut doc, l, 0, 0, RED);
    assert_eq!(doc.history_trimmed().0, 1);
    assert_eq!(doc.history_bytes(), 0);
    assert_eq!(px(&doc, l, 0, 0).a, 255);
}

#[test]
fn layer_order_visibility_and_opacity() {
    let (mut doc, bottom) = small();
    pixel(&mut doc, bottom, 1, 1, RED);
    let top = doc.add_layer("Blue").unwrap();
    pixel(&mut doc, top, 1, 1, Rgba8::new(0, 0, 255, 255));
    doc.set_layer_opacity(top, 0.5, false).unwrap();
    assert_eq!(comp(&doc, 1, 1), Rgba8::new(128, 0, 128, 255));
    doc.set_layer_visible(top, false).unwrap();
    assert_eq!(comp(&doc, 1, 1), RED);
    doc.undo().unwrap();
    doc.move_layer(bottom, 1).unwrap();
    assert_eq!(comp(&doc, 1, 1), RED);
    doc.undo().unwrap();
    assert_eq!(doc.layers()[1].id(), top);
    doc.remove_layer(top).unwrap();
    assert_eq!(doc.layers().len(), 1);
    doc.undo().unwrap();
    assert_eq!(doc.layers()[1].id(), top);
    assert_eq!(doc.layers()[1].name(), "Blue");
    assert_eq!(px(&doc, top, 1, 1), Rgba8::new(0, 0, 255, 255));
}

#[test]
fn opacity_drags_coalesce_into_one_step() {
    let (mut doc, l) = small();
    pixel(&mut doc, l, 1, 1, RED);
    let steps = doc.undo_count();
    for v in [0.9, 0.7, 0.5, 0.3] {
        doc.set_layer_opacity(l, v, true).unwrap();
    }
    assert_eq!(doc.undo_count(), steps + 1);
    doc.end_coalescing();
    doc.set_layer_opacity(l, 0.2, true).unwrap();
    assert_eq!(doc.undo_count(), steps + 2);
    doc.undo().unwrap();
    assert_eq!(doc.layer(l).unwrap().opacity(), 0.3);
    doc.undo().unwrap();
    assert_eq!(doc.layer(l).unwrap().opacity(), 1.0);
    doc.redo().unwrap();
    assert_eq!(doc.layer(l).unwrap().opacity(), 0.3);
    // ドラッグを Escape で止める: その段ごと無かったことに
    doc.set_layer_opacity(l, 0.6, true).unwrap();
    doc.set_layer_opacity(l, 0.1, true).unwrap();
    let n = doc.undo_count();
    assert!(doc.cancel_coalescing().unwrap());
    assert_eq!(doc.undo_count(), n - 1);
    assert_eq!(doc.layer(l).unwrap().opacity(), 0.3);
    assert!(!doc.cancel_coalescing().unwrap());
}

#[test]
fn rename_and_refusals() {
    let (mut doc, l) = small();
    doc.set_layer_name(l, "名前").unwrap();
    assert_eq!(doc.layer(l).unwrap().name(), "名前");
    doc.undo().unwrap();
    assert_eq!(doc.layer(l).unwrap().name(), "L");
    assert!(doc.set_layer_blend_mode(l, BlendMode::PassThrough).is_err());
    assert!(doc.set_layer_opacity(l, 1.5, false).is_err());
    assert!(doc.set_layer_opacity(l, f64::NAN, false).is_err());
    assert!(doc.move_layer(l, 3).is_err());
    assert_eq!(
        doc.remove_layer(LayerId(12345)),
        Err(CoreError::LayerNotFound)
    );
    assert!(Document::new(0, 10).is_err() && Document::new(40000, 10).is_err());
    assert!(Document::with_tile_size(10, 10, 2048).is_err());
    assert!(doc.composite(Rect::new(10, 10, 7, 1)).is_err());
    assert!(doc
        .composite_into(
            Channel::Normal,
            doc.bounds(),
            &mut [0u8; 16 * 16 * 4],
            RowOrder::BottomUp
        )
        .is_err());
}

#[test]
fn budgets_refuse_and_cancel_exactly() {
    // 画素の予算: 2 枚目のタイルで断り、ストロークごと取り消す
    let (mut doc, l) = small();
    doc.set_source_budget_bytes(64).unwrap();
    let mut s = doc.begin_stroke(l, &opaque(RED)).unwrap();
    s.apply_pixel(&mut doc, 0, 0, 1.0, 1.0).unwrap();
    assert_eq!(
        s.apply_pixel(&mut doc, 8, 8, 1.0, 1.0),
        Err(CoreError::SourceBudgetExceeded)
    );
    assert!(!doc.has_active_stroke());
    assert_eq!((doc.allocated_bytes(), doc.undo_count()), (0, 0));

    // ストロークの予算: 1 枚目は 64（段）+ 64（覆い 4×4 の float）+ 64（写し）= 192 で入る、2 枚目で断って元へ戻す
    let (mut doc, l) = small();
    pixel(&mut doc, l, 1, 1, Rgba8::new(10, 20, 30, 255));
    doc.clear_history().unwrap();
    let before = all(&doc);
    doc.set_stroke_budget_bytes(192).unwrap();
    let mut s = doc.begin_stroke(l, &opaque(RED)).unwrap();
    s.apply_pixel(&mut doc, 2, 2, 1.0, 1.0).unwrap();
    assert_eq!(doc.active_stroke_stats().unwrap().rollback_bytes, 192);
    assert_eq!(
        s.apply_pixel(&mut doc, 9, 9, 1.0, 1.0),
        Err(CoreError::StrokeBudgetExceeded)
    );
    assert_eq!(all(&doc), before);

    // 一様なタイルを広げるのを断ると、一様なタイルのまま
    let (mut doc, l) = small();
    doc.import_tile(l, Channel::Color, TileCoord::new(0, 0), &[7u8; 64])
        .unwrap();
    assert_eq!(doc.allocated_bytes(), 4);
    doc.set_source_budget_bytes(4).unwrap();
    let mut s = doc.begin_stroke(l, &opaque(RED)).unwrap();
    assert_eq!(
        s.apply_pixel(&mut doc, 1, 1, 1.0, 1.0),
        Err(CoreError::SourceBudgetExceeded)
    );
    assert_eq!(doc.allocated_bytes(), 4);
    assert_eq!(px(&doc, l, 1, 1), Rgba8::new(7, 7, 7, 7));
    assert!(doc.set_source_budget_bytes(3).is_err());
}

// ───────── ClippingTests ─────────

const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);
const CRED: Rgba8 = Rgba8::new(200, 0, 0, 255);
const GREEN: Rgba8 = Rgba8::new(0, 255, 0, 255);

/// 16×16・タイル 8: 背景は全部青、下地は (2,2) の 1 画素、クリッピングされた層は (2,2) と (9,9)。
fn clip_stack(base_color: Rgba8, clip_color: Rgba8) -> (Document, LayerId, LayerId, LayerId) {
    let mut doc = Document::with_tile_size(16, 16, 8).unwrap();
    let bg = doc.add_layer("bg").unwrap();
    for ty in 0..2 {
        for tx in 0..2 {
            let tile: Vec<u8> = BLUE.to_array().repeat(64);
            doc.import_tile(bg, Channel::Color, TileCoord::new(tx, ty), &tile)
                .unwrap();
        }
    }
    let base = doc.add_layer("base").unwrap();
    doc.set_pixel(base, 2, 2, base_color).unwrap();
    let clip = doc.add_layer("clip").unwrap();
    doc.set_pixel(clip, 2, 2, clip_color).unwrap();
    doc.set_pixel(clip, 9, 9, clip_color).unwrap();
    doc.set_layer_clipping(clip, true).unwrap();
    doc.clear_history().unwrap();
    (doc, bg, base, clip)
}

#[test]
fn clipped_layers_draw_only_inside_the_base() {
    let (mut doc, _, _, clip) = clip_stack(CRED, GREEN);
    assert_eq!((comp(&doc, 2, 2), comp(&doc, 9, 9)), (GREEN, BLUE));
    doc.set_layer_clipping(clip, false).unwrap();
    assert_eq!(comp(&doc, 9, 9), GREEN);
    doc.undo().unwrap();
    assert_eq!(comp(&doc, 9, 9), BLUE);

    let (doc, ..) = clip_stack(Rgba8::new(200, 0, 0, 128), GREEN);
    let want = yolu_core::blend::blend(BLUE, Rgba8::new(0, 255, 0, 128), 1.0, BlendMode::Normal);
    assert_eq!(want, Rgba8::new(0, 128, 127, 255));
    assert_eq!(comp(&doc, 2, 2), want);

    let (mut doc, _, _, clip) = clip_stack(CRED, Rgba8::new(128, 128, 128, 255));
    doc.set_layer_blend_mode(clip, BlendMode::Multiply).unwrap();
    assert_eq!(comp(&doc, 2, 2), Rgba8::new(100, 0, 0, 255));
}

#[test]
fn base_opacity_visibility_and_clip_opacity() {
    let (mut doc, _, base, clip) = clip_stack(CRED, GREEN);
    doc.set_layer_opacity(base, 0.5, false).unwrap();
    assert_eq!(
        comp(&doc, 2, 2),
        yolu_core::blend::blend(BLUE, GREEN, 0.5, BlendMode::Normal)
    );
    doc.set_layer_opacity(base, 1.0, false).unwrap();
    doc.set_layer_visible(base, false).unwrap();
    assert_eq!(comp(&doc, 2, 2), BLUE);
    doc.set_layer_visible(base, true).unwrap();
    doc.set_layer_opacity(clip, 0.5, false).unwrap();
    assert_eq!(comp(&doc, 2, 2), Rgba8::new(100, 128, 0, 255));
}

#[test]
fn several_clipped_layers_and_the_bottom_mark() {
    let (mut doc, bg, _, _) = clip_stack(CRED, GREEN);
    let second = doc.add_layer("second").unwrap();
    doc.set_pixel(second, 2, 2, Rgba8::new(255, 255, 255, 128))
        .unwrap();
    doc.set_pixel(second, 9, 9, Rgba8::new(255, 255, 255, 255))
        .unwrap();
    doc.set_layer_clipping(second, true).unwrap();
    assert_eq!(
        (comp(&doc, 2, 2), comp(&doc, 9, 9)),
        (Rgba8::new(128, 255, 128, 255), BLUE)
    );
    doc.set_layer_clipping(bg, true).unwrap();
    assert!(!doc.is_effectively_clipped(0));
    assert_eq!(comp(&doc, 5, 5), BLUE);
}

#[test]
fn moving_the_base_reports_the_clipped_tiles() {
    let (mut doc, _, base, _) = clip_stack(CRED, GREEN);
    let since = doc.change_serial();
    doc.move_layer(base, 0).unwrap();
    assert!(doc
        .changed_tiles(Channel::Color, since)
        .unwrap()
        .contains(&TileCoord::new(1, 1)));
    assert_eq!(comp(&doc, 9, 9), GREEN);
}

// ───────── ChangeTrackingTests ─────────

fn dot(doc: &mut Document, layer: LayerId, x: i64, y: i64) {
    let brush = BrushSettings {
        radius: 2.0,
        hardness: 1.0,
        color: Rgba8::new(9, 80, 200, 255),
        pressure_size: false,
        pressure_opacity: false,
        ..BrushSettings::default()
    };
    let mut s = doc.begin_stroke(layer, &brush).unwrap();
    s.apply_pixel(doc, x, y, 1.0, 1.0).unwrap();
    doc.end_stroke(s).unwrap();
}
fn tiles(v: &[(u32, u32)]) -> Vec<TileCoord> {
    let mut t: Vec<_> = v.iter().map(|&(x, y)| TileCoord::new(x, y)).collect();
    t.sort();
    t
}

#[test]
fn strokes_undo_redo_and_cancel_report_their_tiles() {
    let mut doc = Document::with_tile_size(64, 64, 16).unwrap();
    let l = doc.add_layer("L").unwrap();
    let since = doc.change_serial();
    dot(&mut doc, l, 5, 5);
    dot(&mut doc, l, 40, 50);
    assert_eq!(
        doc.changed_tiles(Channel::Color, since).unwrap(),
        tiles(&[(0, 0), (2, 3)])
    );
    let now = doc.change_serial();
    assert!(doc.changed_tiles(Channel::Color, now).unwrap().is_empty());
    doc.undo().unwrap();
    assert_eq!(
        doc.changed_tiles(Channel::Color, now).unwrap(),
        tiles(&[(2, 3)])
    );
    let now = doc.change_serial();
    doc.redo().unwrap();
    assert_eq!(
        doc.changed_tiles(Channel::Color, now).unwrap(),
        tiles(&[(2, 3)])
    );
    let now = doc.change_serial();
    let mut s = doc.begin_stroke(l, &opaque(RED)).unwrap();
    s.apply_pixel(&mut doc, 20, 20, 1.0, 1.0).unwrap();
    doc.cancel_stroke(s);
    assert_eq!(
        doc.changed_tiles(Channel::Color, now).unwrap(),
        tiles(&[(1, 1)])
    );
    assert!(doc
        .changed_tiles(Channel::Color, doc.change_serial() + 1)
        .is_none());
}

#[test]
fn layer_edits_report_the_layer_tiles() {
    let mut doc = Document::with_tile_size(64, 64, 16).unwrap();
    let a = doc.add_layer("A").unwrap();
    let b = doc.add_layer("B").unwrap();
    dot(&mut doc, a, 3, 3);
    dot(&mut doc, b, 40, 40);
    let check = |doc: &mut Document, edit: &dyn Fn(&mut Document), want: &[(u32, u32)]| {
        let since = doc.change_serial();
        edit(doc);
        assert_eq!(
            doc.changed_tiles(Channel::Color, since).unwrap(),
            tiles(want)
        );
    };
    check(
        &mut doc,
        &|d| d.set_layer_visible(a, false).unwrap(),
        &[(0, 0)],
    );
    check(
        &mut doc,
        &|d| d.set_layer_opacity(a, 0.5, false).unwrap(),
        &[(0, 0)],
    );
    check(
        &mut doc,
        &|d| d.set_layer_blend_mode(b, BlendMode::Screen).unwrap(),
        &[(2, 2)],
    );
    check(&mut doc, &|d| d.move_layer(a, 1).unwrap(), &[(0, 0)]);
    check(
        &mut doc,
        &|d| {
            d.add_layer("C").unwrap();
        },
        &[],
    );
    check(&mut doc, &|d| d.remove_layer(b).unwrap(), &[(2, 2)]);
    check(
        &mut doc,
        &|d| {
            d.undo().unwrap();
        },
        &[(2, 2)],
    );
    check(
        &mut doc,
        &|d| {
            d.set_pixel(a, 9, 9, RED).unwrap();
        },
        &[(0, 0)],
    );
    check(&mut doc, &|d| d.set_layer_name(a, "renamed").unwrap(), &[]);
}

#[test]
fn a_clipping_mark_that_starts_applying_reports_where_the_composite_changes() {
    let mut doc = Document::with_tile_size(64, 32, 16).unwrap();
    let b = doc.add_layer("B").unwrap();
    dot(&mut doc, b, 20, 5);
    let a = doc.add_layer("A").unwrap();
    dot(&mut doc, a, 3, 3);
    doc.set_layer_clipping(a, true).unwrap();
    let steps: [&dyn Fn(&mut Document); 3] = [
        &|d| d.move_layer(b, 1).unwrap(),
        &|d| {
            d.undo().unwrap();
        },
        &|d| {
            d.redo().unwrap();
        },
    ];
    for step in steps {
        let before = all(&doc);
        let since = doc.change_serial();
        step(&mut doc);
        let after = all(&doc);
        assert_ne!(before, after);
        let changed = doc.changed_tiles(Channel::Color, since).unwrap();
        for (i, (p, q)) in before
            .chunks_exact(4)
            .zip(after.chunks_exact(4))
            .enumerate()
        {
            if p != q {
                let (x, y) = ((i % 64) as u32, (i / 64) as u32);
                assert!(
                    changed.contains(&TileCoord::new(x / 16, y / 16)),
                    "({x},{y}) が記録に無い"
                );
            }
        }
    }
}

// ───────── IntegrationTests ─────────

#[test]
fn a_dab_on_a_tile_corner_of_a_4k_document() {
    let mut doc = Document::new(4096, 4096).unwrap();
    let l = doc.add_layer("L").unwrap();
    let mut s = doc
        .begin_stroke(
            l,
            &BrushSettings {
                radius: 4.0,
                ..BrushSettings::default()
            },
        )
        .unwrap();
    s.add_point(&mut doc, 2048.0, 2048.0, 1.0, DVec2::ZERO)
        .unwrap();
    doc.end_stroke(s).unwrap();
    assert!(doc.allocated_bytes() <= 4 * 128 * 128 * 4);
    assert!(px(&doc, l, 2048, 2048).a > 0 && px(&doc, l, 2047, 2047).a > 0);
}

#[test]
fn undo_redo_and_cancel_restore_exact_bytes() {
    let mut doc = Document::with_tile_size(64, 64, 16).unwrap();
    let l = doc.add_layer("L").unwrap();
    let layer_bytes = |d: &Document| {
        d.layer(l)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .to_canvas_bytes()
    };
    let before = layer_bytes(&doc);
    let brush = BrushSettings {
        radius: 8.0,
        color: Rgba8::new(90, 10, 130, 128),
        ..BrushSettings::default()
    };
    let mut s = doc.begin_stroke(l, &brush).unwrap();
    s.add_point(&mut doc, 30.0, 30.0, 1.0, DVec2::ZERO).unwrap();
    s.add_point(&mut doc, 40.0, 40.0, 1.0, DVec2::ZERO).unwrap();
    doc.end_stroke(s).unwrap();
    let painted = layer_bytes(&doc);
    doc.undo().unwrap();
    assert_eq!(layer_bytes(&doc), before);
    doc.redo().unwrap();
    assert_eq!(layer_bytes(&doc), painted);
    let mut s = doc.begin_stroke(l, &BrushSettings::default()).unwrap();
    s.add_point(&mut doc, 10.0, 10.0, 1.0, DVec2::ZERO).unwrap();
    doc.cancel_stroke(s);
    assert_eq!(layer_bytes(&doc), painted);
}

// ───────── BrushDynamicsTests（Unity 版が固定している SHA-256） ─────────

#[test]
fn the_round_brush_matches_the_hash_pinned_by_the_unity_tests() {
    let mut doc = Document::with_tile_size(96, 64, 32).unwrap();
    let l = doc.add_layer("L").unwrap();
    let brush = BrushSettings {
        radius: 6.0,
        hardness: 1.0,
        spacing: 0.2,
        opacity: 0.8,
        flow: 0.5,
        color: Rgba8::new(200, 60, 30, 255),
        ..BrushSettings::default()
    };
    let mut s = doc.begin_stroke(l, &brush).unwrap();
    let (x0, y0, x1, y1, n, p0, p1) = (8.0, 30.0, 88.0, 34.0, 23, 0.3, 1.0);
    for i in 0..=n {
        let t = i as f64 / n as f64;
        let sample = BrushSample::new(
            x0 + (x1 - x0) * t,
            y0 + (y1 - y0) * t + 6.0 * (t * 6.0).sin(),
            p0 + (p1 - p0) * t,
            i as f64 * 0.01,
            DVec2::ZERO,
        )
        .unwrap();
        s.add_sample(&mut doc, sample).unwrap();
    }
    doc.end_stroke(s).unwrap();
    let hash: String = Sha256::digest(all(&doc))
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        hash,
        "ac5e5edac86c317850a9bcfba4e9aa55a24db80fdd45bf1fc74016768860b587"
    );
}

// ───────── 合成の口 ─────────

#[test]
fn top_down_rows_are_the_bottom_up_rows_reversed() {
    let mut doc = Document::with_tile_size(300, 200, 64).unwrap();
    let l = doc.add_layer("L").unwrap();
    let mut s = doc
        .begin_stroke(
            l,
            &BrushSettings {
                radius: 30.0,
                ..BrushSettings::default()
            },
        )
        .unwrap();
    s.add_point(&mut doc, 10.0, 10.0, 1.0, DVec2::ZERO).unwrap();
    s.add_point(&mut doc, 290.0, 190.0, 0.4, DVec2::ZERO)
        .unwrap();
    doc.end_stroke(s).unwrap();
    let rect = Rect::new(17, 23, 250, 150);
    let up = doc.composite(rect).unwrap();
    let mut down = vec![0u8; up.len()];
    doc.composite_into(Channel::Color, rect, &mut down, RowOrder::TopDown)
        .unwrap();
    let row = 250 * 4;
    for r in 0..150 {
        assert_eq!(
            &up[r * row..(r + 1) * row],
            &down[(149 - r) * row..(150 - r) * row]
        );
    }
    // 部分の矩形 = 全体の該当部分
    let whole = all(&doc);
    for r in 0..150usize {
        let src = ((23 + r) * 300 + 17) * 4;
        assert_eq!(&up[r * row..(r + 1) * row], &whole[src..src + row]);
    }
    assert!(doc.composite(Rect::new(0, 0, 0, 0)).unwrap().is_empty());
}

#[test]
fn compositing_does_not_depend_on_the_thread_count() {
    let mut doc = Document::new(1024, 768).unwrap();
    for (i, mode) in [
        BlendMode::Normal,
        BlendMode::Overlay,
        BlendMode::Hue,
        BlendMode::ColorDodge,
    ]
    .into_iter()
    .enumerate()
    {
        let l = doc.add_layer("L").unwrap();
        doc.set_layer_blend_mode(l, mode).unwrap();
        doc.set_layer_clipping(l, i == 2).unwrap();
        let mut s = doc
            .begin_stroke(
                l,
                &BrushSettings {
                    radius: 60.0 + 10.0 * i as f64,
                    color: Rgba8::new(40 * i as u8, 200, 90, 180),
                    ..BrushSettings::default()
                },
            )
            .unwrap();
        for k in 0..20 {
            let k = k as f64;
            s.add_point(
                &mut doc,
                50.0 + 45.0 * k,
                100.0 + 25.0 * k + 60.0 * i as f64,
                0.3 + k / 30.0,
                DVec2::ZERO,
            )
            .unwrap();
        }
        doc.end_stroke(s).unwrap();
    }
    let many = all(&doc);
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
        .install(|| all(&doc));
    let three = rayon::ThreadPoolBuilder::new()
        .num_threads(3)
        .build()
        .unwrap()
        .install(|| all(&doc));
    assert_eq!(many, one);
    assert_eq!(many, three);
}

#[test]
fn big_dabs_give_the_same_bytes_with_any_thread_count() {
    let run = || {
        let mut doc = Document::with_tile_size(400, 300, 64).unwrap();
        let l = doc.add_layer("L").unwrap();
        // 全画素・一様・無いタイルの混ざった層
        for (i, coord) in [(0, 0), (1, 1), (2, 2), (3, 1), (4, 3)].iter().enumerate() {
            let mut bytes = vec![0u8; 64 * 64 * 4];
            for (k, b) in bytes.iter_mut().enumerate() {
                *b = if i % 2 == 0 {
                    (k * 7 + i * 31) as u8
                } else {
                    (100 + i) as u8
                };
            }
            doc.import_tile(l, Channel::Color, TileCoord::new(coord.0, coord.1), &bytes)
                .unwrap();
        }
        let brush = BrushSettings {
            radius: 90.0,
            hardness: 0.4,
            spacing: 0.05,
            flow: 0.3,
            color: Rgba8::new(30, 200, 120, 220),
            pressure_flow: true,
            ..BrushSettings::default()
        };
        let since = doc.change_serial();
        let mut s = doc.begin_stroke(l, &brush).unwrap();
        for k in 0..25 {
            let k = k as f64;
            s.add_point(
                &mut doc,
                40.0 + 13.0 * k,
                50.0 + 8.0 * k + 20.0 * (k * 0.7).sin(),
                0.4 + k / 40.0,
                DVec2::ZERO,
            )
            .unwrap();
        }
        let stats = doc.active_stroke_stats().unwrap();
        doc.end_stroke(s).unwrap();
        let mut e = doc
            .begin_stroke(
                l,
                &BrushSettings {
                    erase: true,
                    radius: 70.0,
                    ..brush
                },
            )
            .unwrap();
        e.add_point(&mut doc, 380.0, 20.0, 1.0, DVec2::ZERO)
            .unwrap();
        e.add_point(&mut doc, 20.0, 280.0, 0.6, DVec2::ZERO)
            .unwrap();
        doc.end_stroke(e).unwrap();
        let bytes = doc
            .layer(l)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .to_canvas_bytes();
        (
            bytes,
            stats,
            doc.changed_tiles(Channel::Color, since).unwrap(),
            doc.allocated_bytes(),
            doc.history_bytes(),
        )
    };
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
        .install(run);
    let four = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap()
        .install(run);
    let many = run();
    assert_eq!(one.1.parallel_dabs, 0);
    assert!(four.1.parallel_dabs > 0 && many.1.parallel_dabs > 0);
    for other in [&four, &many] {
        assert!(one.0 == other.0, "画素がスレッドの数で変わった");
        assert_eq!(
            (one.1.stamps, one.1.tiles, one.1.rollback_bytes),
            (other.1.stamps, other.1.tiles, other.1.rollback_bytes)
        );
        assert_eq!((&one.2, one.3, one.4), (&other.2, other.3, other.4));
    }
}

#[test]
fn the_document_can_move_between_threads_and_be_read_from_several() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Document>();
    send_sync::<yolu_core::Stroke>();
    let mut doc = Document::with_tile_size(64, 64, 16).unwrap();
    let l = doc.add_layer("L").unwrap();
    dot(&mut doc, l, 5, 5);
    let pixels = std::thread::spawn(move || all(&doc)).join().unwrap();
    assert_eq!(pixels.len(), 64 * 64 * 4);
}

#[test]
fn dabs_are_reported_while_the_stroke_is_still_open() {
    // 画面は描いている途中も変わったタイルだけを描き直す（C# の SurfaceRevisionTests: 確定の前のダブもタイルの変化に入る）
    let mut doc = Document::with_tile_size(64, 64, 16).unwrap();
    let l = doc.add_layer("L").unwrap();
    let mut s = doc.begin_stroke(l, &opaque(RED)).unwrap();
    let since = doc.change_serial();
    let revision = doc.revision();
    s.add_point(&mut doc, 8.0, 8.0, 1.0, DVec2::ZERO).unwrap();
    assert_eq!(
        doc.changed_tiles(Channel::Color, since).unwrap(),
        tiles(&[(0, 0)])
    );
    assert!(doc.revision() > revision);
    let since = doc.change_serial();
    s.add_point(&mut doc, 40.0, 8.0, 1.0, DVec2::ZERO).unwrap();
    assert_eq!(
        doc.changed_tiles(Channel::Color, since).unwrap(),
        tiles(&[(0, 0), (1, 0), (2, 0)])
    );
    let since = doc.change_serial();
    doc.end_stroke(s).unwrap();
    assert!(doc.changed_tiles(Channel::Color, since).unwrap().is_empty()); // 確定は画素を変えない
}

#[test]
fn changed_tiles_can_be_recomposited_one_by_one() {
    let mut doc = Document::with_tile_size(100, 70, 32).unwrap();
    let l = doc.add_layer("L").unwrap();
    let mut screen = all(&doc);
    let since = doc.change_serial();
    let mut s = doc
        .begin_stroke(
            l,
            &BrushSettings {
                radius: 12.0,
                ..opaque(RED)
            },
        )
        .unwrap();
    s.add_point(&mut doc, 5.0, 60.0, 1.0, DVec2::ZERO).unwrap();
    s.add_point(&mut doc, 95.0, 10.0, 1.0, DVec2::ZERO).unwrap();
    doc.end_stroke(s).unwrap();
    for coord in doc.changed_tiles(Channel::Color, since).unwrap() {
        let r = doc.tile_rect(coord).unwrap();
        let part = doc.composite(r).unwrap();
        for row in 0..r.height as usize {
            let dst = ((r.y as usize + row) * 100 + r.x as usize) * 4;
            screen[dst..dst + r.width as usize * 4]
                .copy_from_slice(&part[row * r.width as usize * 4..][..r.width as usize * 4]);
        }
    }
    assert_eq!(screen, all(&doc));
    assert_eq!(
        doc.tile_rect(TileCoord::new(3, 2)),
        Some(Rect::new(96, 64, 4, 6))
    );
    assert_eq!(doc.tile_rect(TileCoord::new(4, 0)), None);
}
