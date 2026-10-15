//! 入り抜きを掛ける項目（`StrokeAssist::taper_targets`）の 3D: 大きさを外して濃さだけに掛けると、面の線の太さは変わらず、入りの先頭と
//! 抜きの終わりで濃さが最小値まで下がり、大きさの最小値は先頭・終わりの太さを下から押さえる。抜きの長さの内のダブは、掛ける項目によらず
//! 離すまで待つ。既定（大きさだけ・最小値 0）は今のバイトのまま（`surface_tip` の入り抜きの試験と `golden` が作り直さずに通る）。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::brush::Item;
use yolu_core::geometry::{SurfaceStroke, SurfaceStrokeOptions};
use yolu_core::{Brush, BrushSettings, PressureSwitches, Rgba8, StrokeAssist, TaperTargets};

use crate::anti_alias::{bytes, document, front, quad, screen, H, W};

fn brush(taper_in: f64, taper_out: f64, targets: TaperTargets) -> Brush {
    let mut b = Brush::simple(
        BrushSettings {
            radius: 5.0,
            hardness: 1.0,
            spacing: 0.1,
            opacity: 1.0,
            flow: 1.0,
            color: Rgba8::new(0, 0, 0, 255),
            ..BrushSettings::default()
        },
        PressureSwitches {
            size: false,
            opacity: false,
            flow: false,
        },
    );
    b.assist = StrokeAssist {
        taper_in,
        taper_out,
        taper_targets: targets,
        ..StrokeAssist::default()
    };
    b
}

/// 横の線（世界の x 10〜86、y 30）の、アルファ。`finish` が偽なら、離す前の様子。
fn draw(b: &Brush, finish: bool) -> Vec<u8> {
    let g = quad();
    let (mut d, l) = document(1);
    let mut stroke = d.begin_brush_stroke(l, b).unwrap();
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g,
        front(),
        &b.base,
        Some(0),
        screen(10.0, 30.0),
        1.0,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for x in [30.0, 50.0, 70.0, 86.0] {
        s.add(&mut d, &mut stroke, screen(x, 30.0), 1.0).unwrap();
        s.paint_queued(&mut d, &mut stroke, usize::MAX).unwrap();
    }
    if finish {
        s.finish(&mut d, &mut stroke).unwrap();
    }
    let out = bytes(&d, l);
    if finish {
        d.end_stroke(stroke).unwrap();
    } else {
        d.cancel_stroke(stroke);
    }
    out.chunks_exact(4).map(|p| p[3]).collect()
}

/// 列 x の塗った画素の数（太さ）と、線の中心の行の濃さ。
fn column(a: &[u8], x: usize) -> (usize, u8) {
    let w = W as usize;
    let thickness = (0..H as usize).filter(|&y| a[y * w + x] > 0).count();
    // 線の中心の行は、列の中で一番濃い画素
    let row = (0..H as usize).max_by_key(|&y| a[y * w + x]).unwrap_or(0);
    (thickness, a[row * w + x])
}

fn only(item: Item, min: f64) -> TaperTargets {
    TaperTargets::NONE.with(item, Some(min)).unwrap()
}

#[test]
fn opacity_only_taper_keeps_the_3d_thickness_and_fades_the_ends_down_to_the_minimum() {
    let none = draw(&brush(0.0, 0.0, TaperTargets::default()), true);
    for min in [0.0, 0.4] {
        let a = draw(&brush(30.0, 30.0, only(Item::Opacity, min)), true);
        assert_eq!(
            column(&a, 12).0,
            column(&none, 12).0,
            "min {min}: 入りの太さ"
        );
        assert_eq!(
            column(&a, 84).0,
            column(&none, 84).0,
            "min {min}: 抜きの太さ"
        );
        assert_eq!(column(&a, 48).1, 255, "min {min}: 範囲の外");
        let (head, tail) = (column(&a, 11).1, column(&a, 85).1);
        assert!(head < 255 && tail < 255, "min {min}: {head} {tail}");
        assert!(head >= (255.0 * min) as u8 && tail >= (255.0 * min) as u8);
        if min > 0.0 {
            assert!(
                head > 255 / 5 && tail > 255 / 5,
                "最小値で止まる: {head} {tail}"
            );
        }
    }
}

#[test]
fn a_size_minimum_keeps_the_3d_ends_from_thinning_below_it() {
    let zero = draw(&brush(30.0, 30.0, only(Item::Size, 0.0)), true);
    let half = draw(&brush(30.0, 30.0, only(Item::Size, 0.5)), true);
    let full = draw(&brush(0.0, 0.0, TaperTargets::default()), true);
    let (t0, th, tf) = (
        column(&zero, 12).0,
        column(&half, 12).0,
        column(&full, 12).0,
    );
    assert!(t0 < th && th < tf, "{t0} {th} {tf}");
    assert_eq!(column(&half, 48), column(&full, 48), "範囲の外は同じ");
}

#[test]
fn the_3d_end_waits_for_the_stroke_to_finish_whichever_item_it_tapers() {
    for item in [Item::Opacity, Item::Flow, Item::Hardness, Item::Roundness] {
        let b = brush(0.0, 30.0, only(item, 0.0));
        let during = draw(&b, false);
        assert_eq!(
            column(&during, 84).0,
            0,
            "{item:?}: 抜きの長さの内は離すまで描かない"
        );
        assert!(column(&during, 30).0 > 0, "{item:?}");
        assert!(column(&draw(&b, true), 84).0 > 0, "{item:?}: 離すと描く");
    }
}
