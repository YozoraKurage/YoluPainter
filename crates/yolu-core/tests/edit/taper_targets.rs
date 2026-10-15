//! 入り抜きを掛ける項目（`StrokeAssist::taper_targets`）の 2D: 既定（大きさだけ・最小値 0）は今のバイトのまま（`golden`・`reference` が
//! 作り直さずに通る）。大きさを外して濃さだけに掛けると、線の太さは変わらず、入りの先頭と抜きの終わりで濃さが最小値まで下がる。
//! 抜きの長さの内のダブは、掛ける項目によらず終わりが分かるまで待つ。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::brush::Item;
use yolu_core::glam::DVec2;
use yolu_core::{
    builtin_tip, Brush, BrushSettings, Channel, ColorMix, CoreError, Document, LayerId, MixMode,
    PaperTexture, PressureSwitches, Rgba8, StrokeAssist, TaperTargets, TileCoord,
};

const W: u32 = 128;
const H: u32 = 48;

fn brush(taper_in: f64, taper_out: f64, targets: TaperTargets) -> Brush {
    let mut b = Brush::simple(
        BrushSettings {
            radius: 6.0,
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

fn alpha(doc: &Document) -> Vec<u8> {
    doc.layers()[0]
        .surface(Channel::Color)
        .map_or_else(|| vec![0; (W * H * 4) as usize], |s| s.to_canvas_bytes())
        .chunks_exact(4)
        .map(|p| p[3])
        .collect()
}

/// 横の線（x 10〜110、y 24）。`finish` が偽なら離す前の様子を返す。
fn draw(b: &Brush, finish: bool) -> Vec<u8> {
    let mut doc = Document::with_tile_size(W, H, 32).unwrap();
    let layer = doc.add_layer("a").unwrap();
    let mut s = doc.begin_brush_stroke(layer, b).unwrap();
    for x in [10.5, 60.5, 110.5] {
        s.add_point(&mut doc, x, 24.5, 1.0, DVec2::ZERO).unwrap();
    }
    if !finish {
        return alpha(&doc);
    }
    doc.end_stroke(s).unwrap();
    alpha(&doc)
}

/// 列 x の塗った画素の数（太さ）と、中心の行の濃さ。
fn column(a: &[u8], x: usize) -> (usize, u8) {
    let thickness = (0..H as usize)
        .filter(|&y| a[y * W as usize + x] > 0)
        .count();
    (thickness, a[24 * W as usize + x])
}

fn only(item: Item, min: f64) -> TaperTargets {
    TaperTargets::NONE.with(item, Some(min)).unwrap()
}

#[test]
fn the_default_targets_are_size_only_with_minimum_zero() {
    let t = TaperTargets::default();
    assert!(t.is_default());
    assert_eq!(t.min(Item::Size), Some(0.0));
    for item in Item::ALL.into_iter().skip(1) {
        assert_eq!(t.min(item), None, "{item:?}");
    }
    assert_eq!(StrokeAssist::default().taper_targets, t);
    // 既定は、入り抜きの長さを足しても今までの大きさだけの入り抜き（同じ式・同じバイト）
    let explicit = TaperTargets::NONE.with(Item::Size, Some(0.0)).unwrap();
    assert_eq!(
        draw(&brush(30.0, 30.0, t), true),
        draw(&brush(30.0, 30.0, explicit), true)
    );
    let a = draw(&brush(30.0, 30.0, t), true);
    assert!(column(&a, 12).0 < column(&a, 60).0, "大きさに効く");
}

#[test]
fn opacity_only_taper_keeps_the_thickness_and_fades_the_ends_down_to_the_minimum() {
    let none = draw(&brush(0.0, 0.0, TaperTargets::default()), true);
    for min in [0.0, 0.4] {
        let a = draw(&brush(30.0, 30.0, only(Item::Opacity, min)), true);
        // 太さは線の全体で同じ（大きさに掛けていない）
        assert_eq!(
            column(&a, 12).0,
            column(&none, 12).0,
            "min {min}: 入りの太さ"
        );
        assert_eq!(
            column(&a, 108).0,
            column(&none, 108).0,
            "min {min}: 抜きの太さ"
        );
        // 範囲の外は全濃度、範囲の中は下がり、先頭・終わりで最小値
        assert_eq!(column(&a, 60).1, 255, "min {min}");
        let (head, tail) = (column(&a, 11).1, column(&a, 109).1);
        assert!(head < 255 && tail < 255, "min {min}: {head} {tail}");
        assert!(head >= (255.0 * min) as u8, "min {min}: 先頭 {head}");
        assert!(tail >= (255.0 * min) as u8, "min {min}: 終わり {tail}");
        assert!(column(&a, 11).1 <= column(&a, 20).1 && column(&a, 20).1 < 255);
        if min > 0.0 {
            assert!(head > 255 / 5, "最小値で止まる（0 まで下がらない）: {head}");
        }
    }
}

#[test]
fn a_size_minimum_keeps_the_ends_from_thinning_below_it() {
    let zero = draw(&brush(30.0, 30.0, only(Item::Size, 0.0)), true);
    let half = draw(&brush(30.0, 30.0, only(Item::Size, 0.5)), true);
    let full = draw(&brush(0.0, 0.0, TaperTargets::default()), true);
    let (t0, th, tf) = (
        column(&zero, 11).0,
        column(&half, 11).0,
        column(&full, 11).0,
    );
    assert!(t0 < th && th < tf, "{t0} {th} {tf}");
    assert!(th >= tf / 2 - 1, "最小値 0.5 の半分の太さ: {th} / {tf}");
    assert_eq!(column(&half, 60), column(&full, 60), "範囲の外は同じ");
}

#[test]
fn the_end_waits_for_the_stroke_to_finish_whichever_item_it_tapers() {
    for item in [Item::Opacity, Item::Flow, Item::Hardness, Item::Roundness] {
        let b = brush(0.0, 30.0, only(item, 0.0));
        let during = draw(&b, false);
        assert_eq!(
            column(&during, 105).0,
            0,
            "{item:?}: 抜きの長さの内は離すまで描かない"
        );
        assert!(column(&during, 40).0 > 0, "{item:?}");
        let done = draw(&b, true);
        assert!(column(&done, 105).0 > 0, "{item:?}: 離すと描く");
    }
}

#[test]
fn flow_and_hardness_and_roundness_targets_change_the_ends() {
    let plain = draw(&brush(0.0, 0.0, TaperTargets::default()), true);
    for item in [Item::Flow, Item::Hardness, Item::Roundness] {
        let a = draw(&brush(30.0, 30.0, only(item, 0.2)), true);
        assert_ne!(a, plain, "{item:?}");
        let mid = 24 * W as usize + 60;
        assert_eq!(a[mid], plain[mid], "{item:?}");
    }
}

#[test]
fn a_taper_minimum_outside_zero_to_one_is_refused() {
    for bad in [-0.1, 1.1, f64::NAN] {
        assert!(
            TaperTargets::NONE.with(Item::Flow, Some(bad)).is_err(),
            "{bad}"
        );
    }
    let b = brush(10.0, 0.0, TaperTargets::default());
    assert!(b.validate().is_ok());
    assert!(matches!(
        TaperTargets::NONE.with(Item::Size, Some(2.0)),
        Err(CoreError::InvalidArgument(_))
    ));
}

const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);

/// 一面が `ground` のレイヤー 1 枚の文書（履歴は空）。
fn ground_doc(ground: Option<Rgba8>) -> (Document, LayerId) {
    let mut doc = Document::with_tile_size(W, H, 32).unwrap();
    let layer = doc.add_layer("a").unwrap();
    if let Some(c) = ground {
        for ty in 0..H.div_ceil(32) {
            for tx in 0..W.div_ceil(32) {
                // キャンバスの外の余白は 0 のまま
                let tile: Vec<u8> = (0..32 * 32)
                    .flat_map(|i| {
                        let inside = tx * 32 + i % 32 < W && ty * 32 + i / 32 < H;
                        if inside {
                            c.to_array()
                        } else {
                            [0; 4]
                        }
                    })
                    .collect();
                doc.import_tile(layer, Channel::Color, TileCoord::new(tx, ty), &tile)
                    .unwrap();
            }
        }
    }
    doc.clear_history().unwrap();
    (doc, layer)
}

fn rgba(doc: &Document) -> Vec<u8> {
    doc.layers()[0]
        .surface(Channel::Color)
        .map_or_else(|| vec![0; (W * H * 4) as usize], |s| s.to_canvas_bytes())
}

/// 横の線を引く。`finish` が偽なら離さずに途中の様子を返して、取り消す（文書は引く前に戻る）。
fn stroke_on(doc: &mut Document, layer: LayerId, b: &Brush, finish: bool) -> Vec<u8> {
    let mut s = doc.begin_brush_stroke(layer, b).unwrap();
    for x in [10.5, 60.5, 110.5] {
        s.add_point(doc, x, 24.5, 1.0, DVec2::ZERO).unwrap();
    }
    if finish {
        doc.end_stroke(s).unwrap();
        return rgba(doc);
    }
    let mid = rgba(doc);
    doc.cancel_stroke(s);
    mid
}

#[test]
fn a_stroke_with_a_non_default_taper_target_undoes_redoes_and_cancels_back_to_the_pixels() {
    let targets = [
        only(Item::Opacity, 0.3),
        only(Item::Flow, 0.3),
        only(Item::Hardness, 0.3),
        only(Item::Roundness, 0.3),
        only(Item::Size, 0.5),
    ];
    for t in targets {
        let b = brush(30.0, 30.0, t);
        let (mut doc, layer) = ground_doc(Some(BLUE));
        let before = rgba(&doc);
        let after = stroke_on(&mut doc, layer, &b, true);
        assert_ne!(after, before, "{t:?}: 描けた");
        // 既定の影響先（大きさだけ）とは違う画素になる（影響先が効いている）
        let (mut plain, pl) = ground_doc(Some(BLUE));
        let default_b = brush(30.0, 30.0, TaperTargets::default());
        assert_ne!(after, stroke_on(&mut plain, pl, &default_b, true), "{t:?}");
        assert!(doc.undo().unwrap());
        assert_eq!(rgba(&doc), before, "{t:?}: Undo で元の画素");
        assert!(doc.redo().unwrap());
        assert_eq!(rgba(&doc), after, "{t:?}: Redo で描いた後の画素");
        assert!(doc.undo().unwrap());
        assert_eq!(rgba(&doc), before, "{t:?}: もう一度 Undo");
        // 離さずに取り消す: 途中は描けていて、取り消すと元の画素で、履歴に残らない
        let mid = stroke_on(&mut doc, layer, &b, false);
        assert_ne!(mid, before, "{t:?}: 途中は描けている");
        assert_eq!(rgba(&doc), before, "{t:?}: 取り消しで元の画素");
        assert!(doc.redo().unwrap(), "{t:?}: 取り消しは Redo を消さない");
        assert_eq!(rgba(&doc), after, "{t:?}");
    }
}

/// 色の混ぜと紙の質感を入れた線のブラシ（半径 6、混ぜる量 1、濃さ 1。入り抜きは 30 画素）。
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

/// 画素の列 x の中心の行（y = 24）の RGBA。
fn center(a: &[u8], x: usize) -> [u8; 4] {
    let o = (24 * W as usize + x) * 4;
    [a[o], a[o + 1], a[o + 2], a[o + 3]]
}

/// 入りの先頭（x < 40）・抜きの終わり（x > 80）の範囲の画素と、その外（x 52〜68。ダブの半径の分だけ内側）。
fn ends_and_middle(a: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (mut ends, mut mid) = (Vec::new(), Vec::new());
    for y in 0..H as usize {
        for x in 0..W as usize {
            let o = (y * W as usize + x) * 4;
            let px = &a[o..o + 4];
            if (52..=68).contains(&x) {
                mid.extend_from_slice(px);
            } else if (6..=116).contains(&x) {
                ends.extend_from_slice(px);
            }
        }
    }
    (ends, mid)
}

#[test]
fn tapering_the_paint_amount_the_paint_density_and_the_texture_density_changes_only_the_ends() {
    let draw_on = |b: &Brush| {
        let (mut doc, layer) = ground_doc(Some(BLUE));
        stroke_on(&mut doc, layer, b, true)
    };
    // 入り抜きなし: 混ぜる量 1 は描く色そのまま・紙の質感は全体にかかる
    let plain = draw_on(&textured_mixing_brush(TaperTargets::default(), 0.0));
    let (plain_ends, plain_mid) = ends_and_middle(&plain);
    for item in [Item::MixPaint, Item::MixDensity, Item::TextureDensity] {
        let a = draw_on(&textured_mixing_brush(only(item, 0.0), 30.0));
        let (ends, mid) = ends_and_middle(&a);
        assert_ne!(ends, plain_ends, "{item:?}: 端が変わる");
        assert_eq!(mid, plain_mid, "{item:?}: 範囲の外は同じ");
    }
    // 混ぜる量・濃さは、先頭と終わりで下地の青へ向かい（最小値 0 の先頭は下地のまま）、最小値があれば止まる
    for item in [Item::MixPaint, Item::MixDensity] {
        let full = center(&plain, 60);
        for (min, floor) in [(0.0, 0u8), (0.5, 100u8)] {
            let a = draw_on(&textured_mixing_brush(only(item, min), 30.0));
            assert_eq!(center(&a, 60), full, "{item:?} {min}: 範囲の外");
            let (head, tail) = (center(&a, 11), center(&a, 109));
            assert!(
                head[0] < full[0] && tail[0] < full[0],
                "{item:?} {min}: 端は下地へ寄る {head:?} {tail:?}"
            );
            if min > 0.0 {
                assert!(head[0] > floor, "{item:?} {min}: 最小値で止まる {head:?}");
            }
        }
    }
    // 質感の濃さ: 下地の無いレイヤーで、先頭の覆いの合計が質感を弱めた分だけ変わる
    let alpha_sum = |a: &[u8]| {
        ends_and_middle(a)
            .0
            .chunks_exact(4)
            .map(|p| p[3] as u64)
            .sum()
    };
    let on = |b: &Brush| {
        let (mut doc, layer) = ground_doc(None);
        stroke_on(&mut doc, layer, b, true)
    };
    let textured: u64 = alpha_sum(&on(&textured_mixing_brush(TaperTargets::default(), 0.0)));
    let tapered: u64 = alpha_sum(&on(&textured_mixing_brush(
        only(Item::TextureDensity, 0.0),
        30.0,
    )));
    assert_ne!(tapered, textured, "質感の濃さが端で下がる");
}
