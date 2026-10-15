//! 最低 1 ピクセル（`Controls::min_one_pixel`）の 2D: 筆圧・入り抜き・フェードで細くなった後のダブの直径に下限 1 画素を掛ける。
//! 切（既定）と、下限が効かない太さのときは今のバイトのまま。直径 0 のダブも 1 画素で残る。間隔は公称の大きさのまま。
//! アンチエイリアスの小さなダブの式は、下限をかけた後の大きさに掛かる（直径 1 画素のブラシで描いたのと同じバイト）。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::glam::DVec2;
use yolu_core::{
    AntiAlias, Brush, BrushSettings, Channel, CoreError, Document, LayerId, LayerLocks, Rgba8,
};

const SIZE: u32 = 64;

fn brush(level: AntiAlias, radius: f64, spacing: f64, on: bool) -> Brush {
    let mut b = Brush::from(BrushSettings {
        radius,
        hardness: 1.0,
        spacing,
        opacity: 1.0,
        flow: 1.0,
        color: Rgba8::new(0, 0, 0, 255),
        pressure_size: true,
        pressure_opacity: false,
        pressure_flow: false,
        erase: false,
        anti_alias: level,
    });
    b.controls.min_one_pixel = on;
    b
}

/// 点の列（x, y, 筆圧）を 1 本のストロークで描いた、アルファ（下の行から、画素の番号 y × SIZE + x）。
fn draw(b: &Brush, points: &[(f64, f64, f64)]) -> Vec<u8> {
    let mut doc = Document::with_tile_size(SIZE, SIZE, 32).unwrap();
    let layer = doc.add_layer("a").unwrap();
    let mut s = doc.begin_brush_stroke(layer, b).unwrap();
    for &(x, y, pressure) in points {
        s.add_point(&mut doc, x, y, pressure, DVec2::ZERO).unwrap();
    }
    doc.end_stroke(s).unwrap();
    doc.layers()[0]
        .surface(Channel::Color)
        .map_or_else(
            || vec![0; (SIZE * SIZE * 4) as usize],
            |s| s.to_canvas_bytes(),
        )
        .chunks_exact(4)
        .map(|p| p[3])
        .collect()
}

fn painted(a: &[u8]) -> usize {
    a.iter().filter(|&&v| v > 0).count()
}

fn line(pressure: f64) -> Vec<(f64, f64, f64)> {
    vec![(10.5, 20.5, pressure), (40.5, 20.5, pressure)]
}

#[test]
fn a_line_drawn_with_almost_no_pressure_is_one_pixel_wide_when_on_and_absent_with_zero_pressure_when_off(
) {
    for pressure in [0.0, 0.02] {
        // 公称の半径 8・間隔 0.0625 は 1 画素おき
        let off = draw(&brush(AntiAlias::None, 8.0, 0.0625, false), &line(pressure));
        if pressure == 0.0 {
            assert_eq!(painted(&off), 0, "切: 筆圧 0 のダブは塗らない");
        }
        let on = draw(&brush(AntiAlias::None, 8.0, 0.0625, true), &line(pressure));
        // 列ごとにちょうど 1 画素（同じ行）。両端の外には塗らない
        let size = SIZE as usize;
        for x in 0..size {
            let column: Vec<usize> = (0..size).filter(|&y| on[y * size + x] > 0).collect();
            if (10..=40).contains(&x) {
                assert_eq!(column, vec![20], "筆圧 {pressure} の x={x}");
            } else {
                assert!(column.is_empty(), "筆圧 {pressure} の x={x}");
            }
        }
    }
}

#[test]
fn the_start_of_a_taper_and_a_faded_out_tail_keep_one_pixel() {
    for (label, edit) in [
        (
            "入り",
            Box::new(|b: &mut Brush| b.assist.taper_in = 20.0) as Box<dyn Fn(&mut Brush)>,
        ),
        (
            "フェード",
            Box::new(|b: &mut Brush| b.controls.fade_size = 4),
        ),
    ] {
        let points = [(10.5, 20.5, 1.0), (40.5, 20.5, 1.0)];
        let mut off = brush(AntiAlias::None, 4.0, 0.125, false);
        edit(&mut off);
        let mut on = off.clone();
        on.controls.min_one_pixel = true;
        let (off, on) = (draw(&off, &points), draw(&on, &points));
        let size = SIZE as usize;
        // 切: 太さが 0 になるダブは塗らない。入: 太さが 0 になる所も 1 画素で続く
        let row_width = |a: &[u8], x: usize| (0..size).filter(|&y| a[y * size + x] > 0).count();
        let thin_off = (10..41).filter(|&x| row_width(&off, x) == 0).count();
        let thin_on = (10..41).filter(|&x| row_width(&on, x) == 0).count();
        assert!(thin_off > 0, "{label}: 切で途切れる所が無い");
        assert_eq!(thin_on, 0, "{label}: 入で途切れた");
        // 入りの下限は 1 画素の幅（太い所は元のまま）
        assert!(painted(&on) > painted(&off), "{label}");
    }
}

#[test]
fn a_brush_thicker_than_one_pixel_paints_the_same_bytes_on_and_off() {
    let points = [(8.0, 9.5, 1.0), (30.25, 40.0, 0.8), (55.0, 20.75, 1.0)];
    for level in [AntiAlias::None, AntiAlias::Medium] {
        let off = draw(&brush(level, 6.0, 0.1, false), &points);
        let on = draw(&brush(level, 6.0, 0.1, true), &points);
        assert!(painted(&off) > 0);
        assert_eq!(off, on, "{level:?}");
    }
}

#[test]
fn the_spacing_follows_the_nominal_size_not_the_floored_size() {
    // 公称の半径 8・間隔 0.125 は 2 画素おき。下限の 1 画素のダブは、その間隔のまま（詰めない）で並ぶ。画素の中心に置いた半径 0.5 の
    // 硬い丸は、その画素だけを塗る
    let on = draw(&brush(AntiAlias::None, 8.0, 0.125, true), &line(0.0));
    let size = SIZE as usize;
    let xs: Vec<usize> = (0..size).filter(|&x| on[20 * size + x] > 0).collect();
    let expected: Vec<usize> = (10..=40).step_by(2).collect();
    assert_eq!(xs, expected);
}

#[test]
fn the_anti_alias_small_dab_rule_applies_to_the_floored_size() {
    // 下限が効いた太さ 0 のダブは、直径 1 画素のブラシで描いたのと同じバイト（縁の広げと濃さは下限の後の大きさに掛かる）。
    // 公称の半径 8・間隔 0.125 と、半径 0.5・間隔 2 は、どちらも 2 画素おき
    let points = |pressure: f64| {
        [
            (10.25, 20.75, pressure),
            (26.5, 33.0, pressure),
            (48.0, 12.5, pressure),
        ]
    };
    for level in [AntiAlias::Weak, AntiAlias::Medium, AntiAlias::Strong] {
        let on = draw(&brush(level, 8.0, 0.125, true), &points(0.0));
        let mut one_pixel = brush(level, 0.5, 2.0, false);
        one_pixel.base.pressure_size = false;
        let reference = draw(&one_pixel, &points(1.0));
        assert!(painted(&on) > 0, "{level:?}");
        assert_eq!(on, reference, "{level:?}");
    }
}

/// 太い線（切のブラシ）を 1 本引いた文書（Undo・取消・ロックの試験で、元に戻る先の画素を空にしないため）。
fn document_with_a_base_line() -> (Document, LayerId) {
    let mut doc = Document::with_tile_size(SIZE, SIZE, 32).unwrap();
    let layer = doc.add_layer("a").unwrap();
    let mut base = brush(AntiAlias::None, 3.0, 0.1, false);
    base.base.color = Rgba8::new(200, 30, 30, 255);
    let mut s = doc.begin_brush_stroke(layer, &base).unwrap();
    for (x, y) in [(8.5, 8.5), (50.5, 8.5)] {
        s.add_point(&mut doc, x, y, 1.0, DVec2::ZERO).unwrap();
    }
    doc.end_stroke(s).unwrap();
    (doc, layer)
}

fn canvas(doc: &Document) -> Vec<u8> {
    doc.layers()[0]
        .surface(Channel::Color)
        .map_or_else(Vec::new, |s| s.to_canvas_bytes())
}

#[test]
fn undo_and_redo_restore_a_zero_pressure_floored_stroke_and_a_cancel_leaves_nothing() {
    let (mut doc, layer) = document_with_a_base_line();
    let before = canvas(&doc);
    let undo_before = doc.undo_count();
    let floored = brush(AntiAlias::None, 8.0, 0.0625, true);
    let mut s = doc.begin_brush_stroke(layer, &floored).unwrap();
    for &(x, y, pressure) in &line(0.0) {
        s.add_point(&mut doc, x, y + 10.0, pressure, DVec2::ZERO)
            .unwrap();
    }
    doc.end_stroke(s).unwrap();
    let after = canvas(&doc);
    assert_ne!(before, after, "筆圧 0 の下限のストロークが画素を変える");
    assert_eq!(
        doc.undo_count(),
        undo_before + 1,
        "1 本のストローク = 1 つの Undo"
    );
    assert!(doc.undo().unwrap());
    assert_eq!(canvas(&doc), before, "Undo で描く前に戻る");
    assert!(doc.redo().unwrap());
    assert_eq!(canvas(&doc), after, "Redo で描いた後に戻る");

    // 描いている途中の取消: 何も残らず、Undo の履歴も増えない
    let (mut doc, layer) = document_with_a_base_line();
    let before = canvas(&doc);
    let undo_before = doc.undo_count();
    let mut s = doc.begin_brush_stroke(layer, &floored).unwrap();
    for &(x, y, pressure) in &line(0.0) {
        s.add_point(&mut doc, x, y + 10.0, pressure, DVec2::ZERO)
            .unwrap();
    }
    doc.cancel_stroke(s);
    assert_eq!(canvas(&doc), before);
    assert_eq!(doc.undo_count(), undo_before);
}

#[test]
fn a_floored_stroke_on_a_locked_layer_is_refused_and_changes_nothing() {
    for lock in [LayerLocks::PIXELS, LayerLocks::ALL] {
        let (mut doc, layer) = document_with_a_base_line();
        doc.set_layer_locks(layer, lock).unwrap();
        let before = canvas(&doc);
        let (undo, redo) = (doc.undo_count(), doc.redo_count());
        let floored = brush(AntiAlias::None, 8.0, 0.0625, true);
        let refused = doc.begin_brush_stroke(layer, &floored);
        assert!(
            matches!(refused, Err(CoreError::LayerLocked { .. })),
            "{lock:?}"
        );
        assert_eq!(canvas(&doc), before, "{lock:?}");
        assert_eq!(
            (doc.undo_count(), doc.redo_count()),
            (undo, redo),
            "{lock:?}"
        );
    }
}
