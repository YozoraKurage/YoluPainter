//! 散布効果の 3D: カメラに正面を向けた平らな四角に、2D と同じブラシ・同じ点の列で描いた散布の粒子の列が、2D と同じになること
//! （同じ設定・同じ入力の点の列なら、2D の描点と 3D の面のダブが同じ散布の乱数の列から同じ粒子を作る）。粒子は小さく硬い丸で、
//! 塗った画素の並びを比べる（3D は画面から面への写しで画素の縁が少し動くので、違う画素の数が少ないことを見る）。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::geometry::{SurfaceStroke, SurfaceStrokeOptions};
use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushSettings, InputResponse, InputUse, PressureSwitches, Rgba8, Spray, SprayDirection,
};

use crate::anti_alias::{bytes, document, front, quad, screen, W};

const ROW: f64 = 30.0;

/// 半径 8 の硬い丸に、粒子サイズ 3 の散布を入れたもの（間隔は直径の 2 倍で、描点の円は重ならない）。
fn sprayed() -> Brush {
    let mut b = Brush::simple(
        BrushSettings {
            radius: 8.0,
            hardness: 1.0,
            spacing: 2.0,
            opacity: 1.0,
            flow: 1.0,
            color: Rgba8::new(128, 64, 64, 255),
            ..BrushSettings::default()
        },
        PressureSwitches::NONE,
    );
    b.spray = Some(Spray {
        size: 3.0,
        density: 6.0,
        bias: 0.3,
        direction: SprayDirection::Line,
        random_angle: Some(0.5),
        ..Spray::default()
    });
    b.influences.spray_size.random = InputUse {
        on: true,
        response: InputResponse::new(0.4, vec![]).unwrap(),
    };
    b.seed = 11;
    b
}

fn points() -> Vec<(f64, f64)> {
    (0..3).map(|i| (16.0 + 32.0 * i as f64, ROW)).collect()
}

fn draw_2d(brush: &Brush) -> Vec<u8> {
    draw_2d_at(brush, 1.0)
}

fn draw_2d_at(brush: &Brush, pressure: f64) -> Vec<u8> {
    let (mut d, l) = document(1);
    let mut s = d.begin_brush_stroke(l, brush).unwrap();
    for (x, y) in points() {
        s.add_point(&mut d, x, y, pressure, DVec2::ZERO).unwrap();
    }
    d.end_stroke(s).unwrap();
    bytes(&d, l)
}

fn draw_3d(brush: &Brush) -> Vec<u8> {
    draw_3d_at(brush, 1.0)
}

fn draw_3d_at(brush: &Brush, pressure: f64) -> Vec<u8> {
    let g = quad();
    let mut b = brush.clone();
    // ブラシの半径と粒子サイズ（どちらも文書の画素）を、四角の上で 2D と同じ大きさになるように直す
    let scale = W as f64 / g.brush_scale() as f64;
    b.base.radius *= scale;
    if let Some(s) = b.spray.as_mut() {
        s.size *= scale;
    }
    let (mut d, l) = document(1);
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let (x0, y0) = points()[0];
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g.clone(),
        front(),
        &b.base,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        screen(x0, y0),
        pressure as f32,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for &(x, y) in &points()[1..] {
        s.add(&mut d, &mut stroke, screen(x, y), pressure as f32)
            .unwrap();
    }
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    bytes(&d, l)
}

/// 塗った画素の印。
fn painted(pixels: &[u8]) -> Vec<bool> {
    pixels.chunks_exact(4).map(|p| p[3] != 0).collect()
}

fn differing(a: &[bool], b: &[bool]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

#[test]
fn the_spray_particles_match_between_2d_and_3d() {
    let brush = sprayed();
    let a = painted(&draw_2d(&brush));
    let b = painted(&draw_3d(&brush));
    let count = a.iter().filter(|p| **p).count();
    assert!(count > 60, "テストの前提: 粒子が塗れる {count}");
    let same = differing(&a, &b);
    // 違う種の 2D（別の粒子の列）は、ずっと多く違う
    let mut other = brush.clone();
    other.seed = 12;
    let unlike = differing(&a, &painted(&draw_2d(&other)));
    assert!(
        same * 4 < unlike && same * 8 < count,
        "2D と 3D の違い {same}・別の種の違い {unlike}・塗った画素 {count}"
    );
}

#[test]
fn the_spray_particles_match_between_2d_and_3d_with_pressure_linked_size_and_center_direction() {
    let mut brush = sprayed();
    brush.spray = Some(Spray {
        size: 5.0,
        link_size: true,
        direction: SprayDirection::Center,
        random_angle: None,
        ..brush.spray.unwrap()
    });
    brush.influences.size.pressure = InputUse {
        on: true,
        response: InputResponse::new(0.2, vec![]).unwrap(),
    };
    let a = painted(&draw_2d_at(&brush, 0.7));
    let b = painted(&draw_3d_at(&brush, 0.7));
    let count = a.iter().filter(|p| **p).count();
    assert!(count > 40, "テストの前提: 粒子が塗れる {count}");
    // 筆圧 1 とは別の塗りになる（筆圧と連動の換算が効いている）
    let full = painted(&draw_2d_at(&brush, 1.0));
    assert!(differing(&a, &full) > 20, "筆圧 0.7 と 1 が同じ");
    let same = differing(&a, &b);
    let mut other = brush.clone();
    other.seed = 12;
    let unlike = differing(&a, &painted(&draw_2d_at(&other, 0.7)));
    assert!(
        same * 4 < unlike && same * 8 < count,
        "2D と 3D の違い {same}・別の種の違い {unlike}・塗った画素 {count}"
    );
}
