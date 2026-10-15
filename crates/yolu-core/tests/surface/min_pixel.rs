//! 最低 1 ピクセル（`Controls::min_one_pixel`）の 3D: 塗るときの半径はブラシの半径（文書の画素）の換算（モデルの単位）に筆圧の大きさを
//! 掛けたもので、下限はその換算で文書の画素の半径 0.5。カメラに正面を向けた平らな四角（UV が文書の全体）に描く。切（既定）は筆圧 0 のダブを
//! 塗らず、入は直径 1 画素のブラシで描いたのと同じ所を塗る。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::geometry::{SurfaceGeometry, SurfaceStroke, SurfaceStrokeOptions};
use yolu_core::glam::Vec2;
use yolu_core::{AntiAlias, Brush, BrushSettings, Document, LayerId, Rgba8};

use crate::anti_alias::{bytes, document, front, quad, screen};

fn brush(radius: f64, spacing: f64, pressure_size: bool, on: bool, level: AntiAlias) -> Brush {
    let mut b = Brush::from(BrushSettings {
        radius,
        hardness: 1.0,
        spacing,
        color: Rgba8::new(20, 40, 160, 255),
        pressure_size,
        pressure_opacity: false,
        anti_alias: level,
        ..BrushSettings::default()
    });
    b.controls.min_one_pixel = on;
    b
}

/// 四角に点の列（世界の点と筆圧）を描いた、レイヤーのバイト。
fn draw(b: &Brush, points: &[(f64, f64, f32)]) -> (Vec<u8>, Document, LayerId) {
    let g: std::sync::Arc<SurfaceGeometry> = quad();
    let (mut d, l) = document(1);
    let mut stroke = d.begin_brush_stroke(l, b).unwrap();
    let at = |p: &(f64, f64, f32)| -> Vec2 { screen(p.0, p.1) };
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g,
        front(),
        &b.base,
        Some(0),
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
