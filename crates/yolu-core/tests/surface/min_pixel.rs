//! 最低 1 ピクセル（`Controls::min_one_pixel`）の 3D: 塗るときの半径はブラシの半径（文書の画素）の換算（モデルの単位）に筆圧の大きさを
//! 掛けたもので、下限はその換算で文書の画素の半径 0.5。カメラに正面を向けた平らな四角（UV が文書の全体）に描く。切（既定）は筆圧 0 のダブを
//! 塗らず、入は直径 1 画素のブラシで描いたのと同じ所を塗る。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::geometry::{
    SurfaceGeometry, SurfaceStroke, SurfaceStrokeOptions, SurfaceTriangle, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::{AntiAlias, Brush, BrushSettings, CoreError, Document, LayerId, LayerLocks, Rgba8};

use crate::anti_alias::{bytes, document, front, quad, screen};

fn brush(radius: f64, spacing: f64, pressure_size: bool, on: bool, level: AntiAlias) -> Brush {
    let mut b = Brush::simple(
        BrushSettings {
            radius,
            hardness: 1.0,
            spacing,
            color: Rgba8::new(20, 40, 160, 255),
            anti_alias: level,
            ..BrushSettings::default()
        },
        yolu_core::PressureSwitches {
            size: pressure_size,
            opacity: false,
            flow: false,
        },
    );
    b.controls.min_one_pixel = on;
    b
}

/// 四角に点の列（世界の点と筆圧）を描いた、レイヤーのバイト。
fn draw(b: &Brush, points: &[(f64, f64, f32)]) -> (Vec<u8>, Document, LayerId) {
    draw_on(quad(), b, points)
}

fn draw_on(
    g: std::sync::Arc<SurfaceGeometry>,
    b: &Brush,
    points: &[(f64, f64, f32)],
) -> (Vec<u8>, Document, LayerId) {
    let (mut d, l) = document(1);
    let mut stroke = d.begin_brush_stroke(l, b).unwrap();
    let at = |p: &(f64, f64, f32)| -> Vec2 { screen(p.0, p.1) };
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g,
        front(),
        &b.base,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        at(&points[0]),
        points[0].2,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for p in &points[1..] {
        s.add(&mut d, &mut stroke, at(p), p.2).unwrap();
    }
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    (bytes(&d, l), d, l)
}

fn alpha(bytes: &[u8]) -> Vec<u8> {
    bytes.chunks_exact(4).map(|p| p[3]).collect()
}

fn painted(a: &[u8]) -> usize {
    a.iter().filter(|&&v| v > 0).count()
}

fn line(pressure: f32) -> Vec<(f64, f64, f32)> {
    vec![(20.5, 30.5, pressure), (70.5, 30.5, pressure)]
}

#[test]
fn zero_pressure_paints_nothing_when_off_and_a_thin_line_when_on() {
    // 公称の半径 8・間隔 0.125 の 3D の間隔は 2 点ほど（モデルの単位の換算は四角の対角線 / 文書の幅）
    let off = alpha(&draw(&brush(8.0, 0.125, true, false, AntiAlias::None), &line(0.0)).0);
    assert_eq!(painted(&off), 0, "切: 筆圧 0 のダブは塗らない");
    let on = alpha(&draw(&brush(8.0, 0.125, true, true, AntiAlias::None), &line(0.0)).0);
    assert!(
        painted(&on) >= 10,
        "入: 筆圧 0 でも 1 画素の点が並ぶ: {}",
        painted(&on)
    );
    // 1 画素の幅: 塗った画素が 1 行の中（高さ 1）に収まる
    let w = crate::anti_alias::W as usize;
    let rows: std::collections::BTreeSet<usize> = on
        .iter()
        .enumerate()
        .filter(|(_, &v)| v > 0)
        .map(|(i, _)| i / w)
        .collect();
    assert!(rows.len() <= 2, "{rows:?}");
}

#[test]
fn the_floor_equals_a_brush_whose_diameter_is_one_document_pixel() {
    // 公称の半径 8 の筆圧 0（入）と、半径 0.5 の筆圧なし（直径 1 画素）。3D の間隔は公称の半径に比例するので、間隔は 16 倍にして間隔をそろえる
    let points = [(18.25, 20.5, 0.0f32), (50.5, 41.25, 0.0), (80.0, 28.0, 0.0)];
    for level in [AntiAlias::None, AntiAlias::Medium] {
        let on = alpha(&draw(&brush(8.0, 0.125, true, true, level), &points).0);
        let reference_points: Vec<(f64, f64, f32)> =
            points.iter().map(|&(x, y, _)| (x, y, 1.0)).collect();
        let reference = alpha(&draw(&brush(0.5, 2.0, false, false, level), &reference_points).0);
        assert!(painted(&on) > 0, "{level:?}");
        let differ = on.iter().zip(&reference).filter(|(a, b)| a != b).count();
        let worst = on
            .iter()
            .zip(&reference)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        println!(
            "{level:?}: 塗った {}・直径 1 画素のブラシ {}・違う {differ}・差の最大 {worst}",
            painted(&on),
            painted(&reference)
        );
        assert!(worst <= 2, "{level:?}: 差 {worst}（違う画素 {differ}）");
    }
}

#[test]
fn a_brush_thicker_than_the_floor_paints_the_same_bytes_on_and_off() {
    let points = [(18.25, 20.5, 1.0f32), (50.5, 41.25, 0.9), (80.0, 28.0, 1.0)];
    for level in [AntiAlias::None, AntiAlias::Medium] {
        let off = draw(&brush(6.0, 0.1, true, false, level), &points).0;
        let on = draw(&brush(6.0, 0.1, true, true, level), &points).0;
        assert!(painted(&alpha(&off)) > 0);
        assert_eq!(off, on, "{level:?}");
    }
}

/// 世界の x 0..96・y 0..64（`quad` と同じ大きさ）を左右の 2 枚の四角に分けた形。左は UV が u 0〜0.5（1 テクセルがモデルの単位で 1）、
/// 右は UV が u 0.5〜0.625・v 0〜0.25（12 × 16 テクセルに 48 × 64 を貼るので、1 テクセルがモデルの単位で 4。密度が低い面）。
fn two_densities() -> std::sync::Arc<SurfaceGeometry> {
    let p = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let tri = |a: (f32, f32), b: (f32, f32), c: (f32, f32), uv: &dyn Fn(f32, f32) -> Vec2| {
        SurfaceTriangle::new(
            p(a.0, a.1),
            p(b.0, b.1),
            p(c.0, c.1),
            uv(a.0, a.1),
            uv(b.0, b.1),
            uv(c.0, c.1),
        )
        .with_slot(0, 0, 0)
    };
    let left = |x: f32, y: f32| Vec2::new(x / 96.0, y / 64.0);
    let right = |x: f32, y: f32| Vec2::new(0.5 + (x - 48.0) / 384.0, (y / 64.0) * 0.25);
    std::sync::Arc::new(
        SurfaceGeometry::new(
            vec![
                tri((0.0, 0.0), (0.0, 64.0), (48.0, 0.0), &left),
                tri((48.0, 0.0), (0.0, 64.0), (48.0, 64.0), &left),
                tri((48.0, 0.0), (48.0, 64.0), (96.0, 0.0), &right),
                tri((96.0, 0.0), (48.0, 64.0), (96.0, 64.0), &right),
            ],
            1,
            DEFAULT_WELD_TOLERANCE,
        )
        .unwrap(),
    )
}

/// 左と右の面の、塗った画素の数（左は x < 48、右は 48 <= x。右の面の UV は x 48〜60・y 0〜16 の画素）。
fn painted_in_halves(a: &[u8]) -> (usize, usize) {
    let w = crate::anti_alias::W as usize;
    let count = |range: std::ops::Range<usize>| {
        a.chunks_exact(w)
            .map(|row| row[range.clone()].iter().filter(|&&v| v > 0).count())
            .sum()
    };
    (count(0..48), count(48..w))
}

#[test]
fn a_low_density_face_still_gets_a_texel_painted_when_the_floor_is_on() {
    // 右の面の 12 × 16 テクセルの中ほど（世界の y 34 は v の中ほどの 1 行）を、筆圧 0 の線で端から端まで引く。右のテクセルの中心は
    // 世界の x で 4 おき。モデル全体の換算の下限（半径 0.5 画素 = モデルの単位でおよそ 0.6）では、中心に届かないダブが多い
    let right_line = [(50.0, 34.0, 0.0f32), (94.0, 34.0, 0.0)];
    let on = alpha(
        &draw_on(
            two_densities(),
            &brush(8.0, 0.125, true, true, AntiAlias::None),
            &right_line,
        )
        .0,
    );
    let (left, right) = painted_in_halves(&on);
    assert_eq!(left, 0);
    // 12 テクセルの幅のどのテクセルにも届く（1 行）
    assert!(right >= 11, "右の面の塗った画素 {right}");
    let off = alpha(
        &draw_on(
            two_densities(),
            &brush(8.0, 0.125, true, false, AntiAlias::None),
            &right_line,
        )
        .0,
    );
    assert_eq!(painted(&off), 0, "切: 筆圧 0 のダブは塗らない");
}

#[test]
fn the_per_face_floor_does_not_change_a_stroke_thicker_than_a_texel_or_the_dense_face() {
    // 切と入が同じバイト: 太い線（下限が効かない）。左の密な面の筆圧 0 の線は、右の面の下限が左へ効かないので左だけ塗る
    let thick = [(10.0, 30.0, 1.0f32), (84.0, 40.0, 1.0)];
    let off = draw_on(
        two_densities(),
        &brush(6.0, 0.1, true, false, AntiAlias::None),
        &thick,
    )
    .0;
    let on = draw_on(
        two_densities(),
        &brush(6.0, 0.1, true, true, AntiAlias::None),
        &thick,
    )
    .0;
    assert!(painted(&alpha(&off)) > 0);
    assert_eq!(off, on);
    let left_line = [(4.5, 20.5, 0.0f32), (44.5, 20.5, 0.0)];
    let on = alpha(
        &draw_on(
            two_densities(),
            &brush(8.0, 0.125, true, true, AntiAlias::None),
            &left_line,
        )
        .0,
    );
    let (left, right) = painted_in_halves(&on);
    assert!(left >= 20, "{left}");
    assert_eq!(right, 0);
}

fn floored() -> Brush {
    brush(8.0, 0.125, true, true, AntiAlias::None)
}

/// 太い線を 1 本引いた文書（元に戻る先の画素を空にしないため）。
fn document_with_a_base_line() -> (Document, LayerId) {
    let (mut d, l) = document(1);
    let g = quad();
    let mut base = brush(6.0, 0.1, false, false, AntiAlias::None);
    base.base.color = Rgba8::new(200, 30, 30, 255);
    let mut stroke = d.begin_brush_stroke(l, &base).unwrap();
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g,
        front(),
        &base.base,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        screen(10.5, 10.5),
        1.0,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    s.add(&mut d, &mut stroke, screen(80.5, 10.5), 1.0).unwrap();
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    (d, l)
}

/// 筆圧 0 の下限の線を引く。`finish` までを、`end` が真なら `end_stroke` で確定し、偽なら `cancel_stroke` で取り消す。
fn floored_stroke(d: &mut Document, l: LayerId, g: std::sync::Arc<SurfaceGeometry>, end: bool) {
    let b = floored();
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let mut s = SurfaceStroke::begin_with_options(
        d,
        &mut stroke,
        g,
        front(),
        &b.base,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        screen(20.5, 30.5),
        0.0,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    s.add(d, &mut stroke, screen(70.5, 30.5), 0.0).unwrap();
    if end {
        s.finish(d, &mut stroke).unwrap();
        d.end_stroke(stroke).unwrap();
    } else {
        d.cancel_stroke(stroke);
    }
}

#[test]
fn undo_and_redo_restore_a_floored_surface_stroke_and_a_cancel_leaves_nothing() {
    let (mut d, l) = document_with_a_base_line();
    let before = bytes(&d, l);
    let undo = d.undo_count();
    floored_stroke(&mut d, l, quad(), true);
    let after = bytes(&d, l);
    assert_ne!(before, after, "筆圧 0 の下限のストロークが画素を変える");
    assert_eq!(d.undo_count(), undo + 1, "1 本のストローク = 1 つの Undo");
    assert!(d.undo().unwrap());
    assert_eq!(bytes(&d, l), before, "Undo で描く前に戻る");
    assert!(d.redo().unwrap());
    assert_eq!(bytes(&d, l), after, "Redo で描いた後に戻る");

    let (mut d, l) = document_with_a_base_line();
    let before = bytes(&d, l);
    let undo = d.undo_count();
    floored_stroke(&mut d, l, quad(), false);
    assert_eq!(bytes(&d, l), before, "描いている途中の取消で何も残らない");
    assert_eq!(d.undo_count(), undo);
}

#[test]
fn a_floored_surface_stroke_on_a_locked_layer_is_refused_and_changes_nothing() {
    for lock in [LayerLocks::PIXELS, LayerLocks::ALL] {
        let (mut d, l) = document_with_a_base_line();
        d.set_layer_locks(l, lock).unwrap();
        let before = bytes(&d, l);
        let (undo, redo) = (d.undo_count(), d.redo_count());
        let refused = d.begin_brush_stroke(l, &floored());
        assert!(
            matches!(refused, Err(CoreError::LayerLocked { .. })),
            "{lock:?}"
        );
        assert_eq!(bytes(&d, l), before, "{lock:?}");
        assert_eq!((d.undo_count(), d.redo_count()), (undo, redo), "{lock:?}");
    }
}
