//! 入り抜きを掛ける項目（`StrokeAssist::taper_targets`）の 2D: 既定（大きさだけ・最小値 0）は今のバイトのまま（`golden`・`reference` が
//! 作り直さずに通る）。大きさを外して濃さだけに掛けると、線の太さは変わらず、入りの先頭と抜きの終わりで濃さが最小値まで下がる。
//! 抜きの長さの内のダブは、掛ける項目によらず終わりが分かるまで待つ。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::brush::Item;
use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushSettings, Channel, CoreError, Document, PressureSwitches, Rgba8, StrokeAssist,
    TaperTargets,
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
