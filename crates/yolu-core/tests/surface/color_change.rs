//! 色の変化の 3D: カメラに正面を向けた平らな四角に、2D と同じブラシ・同じ点の列で描いた硬い小さな丸の、ダブごとの色の列が 2D と同じになること
//! （同じ設定・同じ入力の点の列なら、2D の描点と 3D の面のダブが同じ乱数の列から同じ色を引く）。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::geometry::{SurfaceStroke, SurfaceStrokeOptions};
use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushSettings, ColorDynamics, ColorTarget, InputResponse, InputUse, PressureSwitches,
    Rgba8, StrokeColorChange, TipColorChange,
};

use crate::anti_alias::{bytes, document, front, quad, screen, W};

const BASE: Rgba8 = Rgba8::new(128, 64, 64, 255);
const ROW: usize = 30;

/// 半径 2 の硬い丸（間隔 8 画素）。
fn dots() -> Brush {
    Brush::simple(
        BrushSettings {
            radius: 2.0,
            hardness: 1.0,
            spacing: 2.0,
            opacity: 1.0,
            flow: 1.0,
            color: BASE,
            ..BrushSettings::default()
        },
        PressureSwitches::NONE,
    )
}

fn random(min: f64) -> InputUse {
    InputUse {
        on: true,
        response: InputResponse::new_signed(min, vec![]).unwrap(),
    }
}

fn points() -> Vec<(f64, f64)> {
    (0..10)
        .map(|i| (6.0 + 8.0 * i as f64, ROW as f64))
        .collect()
}

fn draw_2d(brush: &Brush) -> Vec<u8> {
    let (mut d, l) = document(1);
    let mut s = d.begin_brush_stroke(l, brush).unwrap();
    for (x, y) in points() {
        s.add_point(&mut d, x, y, 1.0, DVec2::ZERO).unwrap();
    }
    d.end_stroke(s).unwrap();
    bytes(&d, l)
}

fn draw_3d(brush: &Brush) -> Vec<u8> {
    let g = quad();
    let mut b = brush.clone();
    b.base.radius *= W as f64 / g.brush_scale() as f64;
    let (mut d, l) = document(1);
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let (x0, y0) = points()[0];
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g.clone(),
        front(),
        &b.base,
        Some(0),
        screen(x0, y0),
        1.0,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for &(x, y) in &points()[1..] {
        s.add(&mut d, &mut stroke, screen(x, y), 1.0).unwrap();
    }
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    bytes(&d, l)
}

/// ROW の行の、塗った範囲ごとの中心の色。
fn centers(pixels: &[u8]) -> Vec<[u8; 4]> {
    let row = &pixels[ROW * W as usize * 4..(ROW + 1) * W as usize * 4];
    let mut out = Vec::new();
    let mut x = 0;
    while x < W as usize {
        if row[x * 4 + 3] == 0 {
            x += 1;
            continue;
        }
        let start = x;
        while x < W as usize && row[x * 4 + 3] != 0 {
            x += 1;
        }
        let c = ((start + x - 1) / 2) * 4;
        out.push([row[c], row[c + 1], row[c + 2], row[c + 3]]);
    }
    out
}

fn same_colors_in_2d_and_3d(name: &str, brush: &Brush) {
    let a = centers(&draw_2d(brush));
    let b = centers(&draw_3d(brush));
    assert_eq!(a.len(), 10, "{name}: テストの前提: ダブの数 {a:?}");
    assert_eq!(a.len(), b.len(), "{name}: ダブの数");
    for (i, (p, q)) in a.iter().zip(&b).enumerate() {
        let d = p.iter().zip(q).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        assert!(d <= 1, "{name}: {i} 番目 2D {p:?}・3D {q:?}");
    }
}

#[test]
fn the_dab_colors_of_a_change_match_between_2d_and_3d() {
    let mut tip = dots();
    tip.color.tip = Some(TipColorChange {
        hue: 120.0,
        saturation: 0.3,
        value: -0.2,
    });
    tip.influences.color_hue.random = random(-1.0);
    tip.influences.color_saturation.random = random(-1.0);
    tip.seed = 4;
    let colors = centers(&draw_2d(&tip));
    assert!(
        colors.windows(2).filter(|w| w[0] != w[1]).count() >= 6,
        "テストの前提: 色が散る {colors:?}"
    );
    same_colors_in_2d_and_3d("先端のランダム", &tip);

    let mut sub = dots();
    sub.color.secondary = Rgba8::new(20, 200, 90, 255);
    sub.color.sub_mix = 0.9;
    sub.influences.color_sub_mix.random = random(0.0);
    sub.color.stroke = Some(StrokeColorChange {
        hue: 60.0,
        saturation: 0.2,
        value: 0.2,
        sub_mix: 0.1,
    });
    sub.seed = 9;
    same_colors_in_2d_and_3d("サブとストロークごと", &sub);

    // Photoshop の形の色のゆらぎ（描点ごと）と、先端・サブの変化を一緒に使う
    let mut both = tip.clone();
    both.color = ColorDynamics {
        secondary: Rgba8::new(240, 240, 20, 255),
        foreground_background: 0.5,
        hue: 0.3,
        per_tip: true,
        tip: both.color.tip,
        sub_mix: 0.3,
        target: ColorTarget::MainAndSub,
        ..ColorDynamics::default()
    };
    both.influences.color_sub_mix.random = random(0.0);
    same_colors_in_2d_and_3d("描点ごとの色のゆらぎと一緒", &both);
}
