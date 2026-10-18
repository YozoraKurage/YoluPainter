//! 入り抜きを掛ける項目（`StrokeAssist::taper_targets`）の 3D: 大きさを外して濃さだけに掛けると、面の線の太さは変わらず、入りの先頭と
//! 抜きの終わりで濃さが最小値まで下がり、大きさの最小値は先頭・終わりの太さを下から押さえる。抜きの長さの内のダブは、掛ける項目によらず
//! 離すまで待つ。既定（大きさだけ・最小値 0）は今のバイトのまま（`surface_tip` の入り抜きの試験と `golden` が作り直さずに通る）。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::brush::Item;
use yolu_core::geometry::{SurfaceStroke, SurfaceStrokeOptions};
use yolu_core::{
    builtin_tip, Brush, BrushSettings, Channel, ColorMix, Document, LayerId, MixMode, PaperTexture,
    PressureSwitches, Rgba8, StrokeAssist, TaperTargets, TileCoord,
};

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
        Some(&yolu_core::geometry::MaterialSet::one(0)),
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

const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);

/// 一面が `ground` のレイヤー 1 枚の文書（履歴は空）。
fn ground_doc(ground: Option<Rgba8>) -> (Document, LayerId) {
    let (mut d, l) = document(1);
    if let Some(c) = ground {
        let tile: Vec<u8> = (0..32 * 32).flat_map(|_| c.to_array()).collect();
        for ty in 0..H.div_ceil(32) {
            for tx in 0..W.div_ceil(32) {
                d.import_tile(l, Channel::Color, TileCoord::new(tx, ty), &tile)
                    .unwrap();
            }
        }
    }
    d.clear_history().unwrap();
    (d, l)
}

/// 横の線を面に引く。`finish` が偽なら離さずに途中の様子を返して、取り消す（文書は引く前に戻る）。
fn stroke_on(d: &mut Document, l: LayerId, b: &Brush, finish: bool) -> Vec<u8> {
    let mut stroke = d.begin_brush_stroke(l, b).unwrap();
    let mut s = SurfaceStroke::begin_with_options(
        d,
        &mut stroke,
        quad(),
        front(),
        &b.base,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        screen(10.0, 30.0),
        1.0,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for x in [30.0, 50.0, 70.0, 86.0] {
        s.add(d, &mut stroke, screen(x, 30.0), 1.0).unwrap();
        s.paint_queued(d, &mut stroke, usize::MAX).unwrap();
    }
    if finish {
        s.finish(d, &mut stroke).unwrap();
        d.end_stroke(stroke).unwrap();
        return bytes(d, l);
    }
    let mid = bytes(d, l);
    d.cancel_stroke(stroke);
    mid
}

#[test]
fn a_3d_stroke_with_a_non_default_taper_target_undoes_redoes_and_cancels_back_to_the_pixels() {
    let targets = [
        only(Item::Opacity, 0.3),
        only(Item::Flow, 0.3),
        only(Item::Hardness, 0.3),
        only(Item::Roundness, 0.3),
        only(Item::Size, 0.5),
    ];
    for t in targets {
        let b = brush(30.0, 30.0, t);
        let (mut d, l) = ground_doc(Some(BLUE));
        let before = bytes(&d, l);
        let after = stroke_on(&mut d, l, &b, true);
        assert_ne!(after, before, "{t:?}: 描けた");
        let (mut plain, pl) = ground_doc(Some(BLUE));
        let default_b = brush(30.0, 30.0, TaperTargets::default());
        assert_ne!(after, stroke_on(&mut plain, pl, &default_b, true), "{t:?}");
        assert!(d.undo().unwrap());
        assert_eq!(bytes(&d, l), before, "{t:?}: Undo で元の画素");
        assert!(d.redo().unwrap());
        assert_eq!(bytes(&d, l), after, "{t:?}: Redo で描いた後の画素");
        assert!(d.undo().unwrap());
        assert_eq!(bytes(&d, l), before, "{t:?}: もう一度 Undo");
        // 離さずに取り消す: 途中は描けていて、取り消すと元の画素で、Redo は残る
        let mid = stroke_on(&mut d, l, &b, false);
        assert_ne!(mid, before, "{t:?}: 途中は描けている");
        assert_eq!(bytes(&d, l), before, "{t:?}: 取り消しで元の画素");
        assert!(d.redo().unwrap(), "{t:?}: 取り消しは Redo を消さない");
        assert_eq!(bytes(&d, l), after, "{t:?}");
    }
}

/// 色の混ぜと紙の質感を入れた線のブラシ（混ぜる量 1、濃さ 1。描く色は赤）。
fn textured_mixing_brush(targets: TaperTargets, taper: f64) -> Brush {
    let mut b = brush(taper, taper, targets);
    b.base.color = Rgba8::new(255, 0, 0, 255);
    b.mix = ColorMix {
        mode: MixMode::Mix,
        paint: 1.0,
        density: 1.0,
        stretch: 0.0,
        ..ColorMix::default()
    };
    let mut t = PaperTexture::new(builtin_tip("grain").unwrap(), 1.0);
    t.scale = 2.0;
    b.texture = Some(t);
    b
}

/// 入りの先頭・抜きの終わりの範囲（x < 40・x > 56）の画素と、その外（x 46〜50。入りは x 40、抜きは x 56 から先が範囲の外で、ダブの半径の分だけ内側）。
fn ends_and_middle(a: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (mut ends, mut mid) = (Vec::new(), Vec::new());
    for y in 0..H as usize {
        for x in 0..W as usize {
            let o = (y * W as usize + x) * 4;
            let px = &a[o..o + 4];
            if (46..=50).contains(&x) {
                mid.extend_from_slice(px);
            } else if (6..=90).contains(&x) {
                ends.extend_from_slice(px);
            }
        }
    }
    (ends, mid)
}

/// 列 x の一番赤い画素（線の中心の行。下地は青なので、アルファでは選べない）。
fn center(a: &[u8], x: usize) -> [u8; 4] {
    let row = (0..H as usize)
        .max_by_key(|&y| a[(y * W as usize + x) * 4])
        .unwrap_or(0);
    let o = (row * W as usize + x) * 4;
    [a[o], a[o + 1], a[o + 2], a[o + 3]]
}

#[test]
fn tapering_the_paint_amount_the_paint_density_and_the_texture_density_changes_only_the_3d_ends() {
    let draw_on = |b: &Brush, ground: Option<Rgba8>| {
        let (mut d, l) = ground_doc(ground);
        stroke_on(&mut d, l, b, true)
    };
    let plain = draw_on(
        &textured_mixing_brush(TaperTargets::default(), 0.0),
        Some(BLUE),
    );
    let (plain_ends, plain_mid) = ends_and_middle(&plain);
    for item in [Item::MixPaint, Item::MixDensity, Item::TextureDensity] {
        let a = draw_on(&textured_mixing_brush(only(item, 0.0), 30.0), Some(BLUE));
        let (ends, mid) = ends_and_middle(&a);
        assert_ne!(ends, plain_ends, "{item:?}: 端が変わる");
        assert_eq!(mid, plain_mid, "{item:?}: 範囲の外は同じ");
    }
    // 混ぜる量・濃さは、先頭と終わりで下地の青へ向かい、最小値があれば止まる
    for item in [Item::MixPaint, Item::MixDensity] {
        let full = center(&plain, 48);
        for (min, floor) in [(0.0, 0u8), (0.5, 100u8)] {
            let a = draw_on(&textured_mixing_brush(only(item, min), 30.0), Some(BLUE));
            assert_eq!(center(&a, 48), full, "{item:?} {min}: 範囲の外");
            let (head, tail) = (center(&a, 11), center(&a, 85));
            assert!(
                head[0] < full[0] && tail[0] < full[0],
                "{item:?} {min}: 端は下地へ寄る {head:?} {tail:?}"
            );
            if min > 0.0 {
                assert!(head[0] > floor, "{item:?} {min}: 最小値で止まる {head:?}");
            }
        }
    }
    // 質感の濃さ: 下地の無いレイヤーでも、端の覆いが質感を弱めた分だけ変わる
    let alpha_sum = |a: &[u8]| -> u64 {
        ends_and_middle(a)
            .0
            .chunks_exact(4)
            .map(|p| p[3] as u64)
            .sum()
    };
    let textured = alpha_sum(&draw_on(
        &textured_mixing_brush(TaperTargets::default(), 0.0),
        None,
    ));
    let tapered = alpha_sum(&draw_on(
        &textured_mixing_brush(only(Item::TextureDensity, 0.0), 30.0),
        None,
    ));
    assert_ne!(tapered, textured, "質感の濃さが端で下がる");
}
