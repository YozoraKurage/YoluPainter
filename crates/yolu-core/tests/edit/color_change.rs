//! 色の変化（`ColorDynamics::tip`・`sub_mix`・`stroke`・`target` と、`Influences` の色の 4 項目）の 2D のストローク: 先端の色相・彩度・明度が
//! 影響元（ランダム・筆圧・傾き）の係数の積で動くこと、サブ描画色混合率、ストロークごとの変化、変化対象、範囲の外の断り。
//! ダブは、互いに重ならない硬い小さな丸を横に並べ、ダブごとの中心の画素の色を読む。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::brush::{hsv_to_rgb, rgb_to_hsv};
use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushSample, BrushSettings, ColorDynamics, ColorItem, ColorTarget, Document,
    InputResponse, InputUse, PressureSwitches, Rgba8, StrokeColorChange, TipColorChange,
};

const WIDTH: u32 = 1024;
const WHITE: Rgba8 = Rgba8::new(255, 255, 255, 255);
const BLACK: Rgba8 = Rgba8::new(0, 0, 0, 255);
/// 色相 0・彩度 0.5・明度 約 0.5。
const BASE: Rgba8 = Rgba8::new(128, 64, 64, 255);

/// 半径 2 の硬い丸（間隔は直径の 2 倍 = 8 画素で、ダブは重ならない）。
fn dots(color: Rgba8) -> Brush {
    Brush::simple(
        BrushSettings {
            radius: 2.0,
            hardness: 1.0,
            spacing: 2.0,
            opacity: 1.0,
            flow: 1.0,
            color,
            ..BrushSettings::default()
        },
        PressureSwitches::NONE,
    )
}

fn used(min: f64) -> InputUse {
    InputUse {
        on: true,
        response: InputResponse::new_signed(min, vec![]).unwrap(),
    }
}

fn with_tip(mut b: Brush, hue: f64, saturation: f64, value: f64) -> Brush {
    b.color.tip = Some(TipColorChange {
        hue,
        saturation,
        value,
    });
    b
}

/// 1 本の線（x = 8 から 8 画素おき、筆圧と傾きは一定）のダブごとの中心の色（左から）。
fn dab_colors(brush: &Brush, pressure: f64, tilt: DVec2) -> Vec<Rgba8> {
    let mut d = Document::with_tile_size(WIDTH, 8, 16).unwrap();
    let l = d.add_layer("L").unwrap();
    d.clear_history().unwrap();
    let mut s = d.begin_brush_stroke(l, brush).unwrap();
    for i in 0..((WIDTH - 16) / 8) {
        let x = 8.0 + 8.0 * i as f64;
        s.add_sample(
            &mut d,
            BrushSample::new(x, 4.0, pressure, 0.0, tilt).unwrap(),
        )
        .unwrap();
    }
    d.end_stroke(s).unwrap();
    let pixels = d.composite(d.bounds()).unwrap();
    let row = &pixels[(3 * WIDTH as usize) * 4..(4 * WIDTH as usize) * 4];
    let mut out = Vec::new();
    let mut x = 0;
    while x < WIDTH as usize {
        if row[x * 4 + 3] == 0 {
            x += 1;
            continue;
        }
        let start = x;
        while x < WIDTH as usize && row[x * 4 + 3] != 0 {
            x += 1;
        }
        let c = ((start + x - 1) / 2) * 4;
        out.push(Rgba8::new(row[c], row[c + 1], row[c + 2], row[c + 3]));
    }
    out
}

fn upright(brush: &Brush) -> Vec<Rgba8> {
    dab_colors(brush, 1.0, DVec2::ZERO)
}

/// 色相（度。基準の色相 0 からの符号つきの差。−180〜180）。
fn hue_degrees(c: Rgba8) -> f64 {
    let (h, _, _) = rgb_to_hsv(c.r as f64 / 255.0, c.g as f64 / 255.0, c.b as f64 / 255.0);
    let d = h * 360.0;
    if d > 180.0 {
        d - 360.0
    } else {
        d
    }
}

fn close(a: u8, b: f64) -> bool {
    (a as f64 - b).abs() <= 1.0
}

#[test]
fn a_random_minimum_of_minus_one_scatters_the_hue_on_both_sides_within_the_value() {
    for sign in [1.0, -1.0] {
        let mut hues = Vec::new();
        for seed in 1..=3 {
            let mut b = with_tip(dots(BASE), 90.0 * sign, 0.0, 0.0);
            b.influences.color_hue.random = used(-1.0);
            b.seed = seed;
            hues.extend(upright(&b).into_iter().map(hue_degrees));
        }
        assert!(hues.len() > 300, "テストの前提: ダブの数 {}", hues.len());
        let (lo, hi) = hues
            .iter()
            .fold((f64::MAX, f64::MIN), |(l, h), &v| (l.min(v), h.max(v)));
        // 8 ビットへ丸めた色から測るので、±90 度に少しの幅を持たせる。符号を逆にしても範囲は同じ
        assert!(lo >= -92.0 && hi <= 92.0, "符号 {sign}: {lo}〜{hi}");
        assert!(
            lo < -70.0 && hi > 70.0,
            "符号 {sign}: 両側に出る {lo}〜{hi}"
        );
    }
}

#[test]
fn a_hue_of_360_with_a_random_minimum_of_minus_one_covers_the_whole_circle() {
    let mut b = with_tip(dots(BASE), 360.0, 0.0, 0.0);
    b.influences.color_hue.random = used(-1.0);
    let mut sectors = [0u32; 6];
    for seed in 1..=3 {
        b.seed = seed;
        for c in upright(&b) {
            let (h, _, _) = rgb_to_hsv(c.r as f64 / 255.0, c.g as f64 / 255.0, c.b as f64 / 255.0);
            sectors[((h * 6.0) as usize).min(5)] += 1;
        }
    }
    assert!(sectors.iter().all(|&n| n > 20), "{sectors:?}");
}

#[test]
fn pressure_with_a_minimum_of_zero_moves_the_value_in_proportion_and_clamps() {
    let value = |v: f64, pressure: f64| {
        let mut b = with_tip(dots(BASE), 0.0, 0.0, v);
        b.influences.color_value.pressure = InputUse::STRAIGHT;
        upright_at(&b, pressure)
    };
    let base_v = 128.0 / 255.0;
    // 筆圧 0 は基準のまま
    assert_eq!(value(0.4, 0.0), BASE);
    assert_eq!(value(-1.0, 0.0), BASE);
    // 間は比例する（R は明度 × 255）
    for p in [0.25, 0.5, 0.75] {
        let c = value(0.4, p);
        assert!(
            close(c.r, 255.0 * (base_v + 0.4 * p).min(1.0)),
            "{p}: {c:?}"
        );
    }
    // 筆圧 1 で頭打ち: +1 は明度 1、−1 は明度 0（黒）。彩度は変えない
    assert_eq!(value(1.0, 1.0).r, 255);
    assert_eq!(value(-1.0, 1.0), BLACK);
    // 向き（符号）が下がる方向にも効く
    let down = value(-0.2, 0.5);
    assert!(close(down.r, 255.0 * (base_v - 0.1)), "{down:?}");
}

fn upright_at(brush: &Brush, pressure: f64) -> Rgba8 {
    let colors = dab_colors(brush, pressure, DVec2::ZERO);
    assert!(colors.windows(2).all(|w| w[0] == w[1]), "ダブの色はそろう");
    colors[0]
}

#[test]
fn saturation_and_value_are_clamped_to_zero_and_one() {
    let mut b = with_tip(dots(BASE), 0.0, 1.0, 0.0);
    b.influences.color_saturation.pressure = InputUse::STRAIGHT;
    // 彩度 0.5 + 1 は 1 で頭打ち: 明度は同じ 128 のまま G・B が 0
    assert_eq!(upright_at(&b, 1.0), Rgba8::new(128, 0, 0, 255));
    let mut b = with_tip(dots(BASE), 0.0, -1.0, 0.0);
    b.influences.color_saturation.pressure = InputUse::STRAIGHT;
    assert_eq!(upright_at(&b, 1.0), Rgba8::new(128, 128, 128, 255));
}

#[test]
fn a_secondary_color_mix_without_an_influence_is_constant() {
    let mut b = dots(BASE);
    b.color.secondary = WHITE;
    b.color.sub_mix = 0.5;
    let colors = upright(&b);
    assert!(colors.len() > 100);
    assert!(colors.iter().all(|c| *c == colors[0]), "一定");
    let c = colors[0];
    assert!(close(c.r, 191.5f64.round()), "{c:?}");
    assert!(close(c.g, 159.5f64.round()), "{c:?}");
    assert_eq!((c.b, c.a), (c.g, 255));
    // 1 はサブ描画色そのもの
    b.color.sub_mix = 1.0;
    assert_eq!(upright(&b)[0], WHITE);
}

#[test]
fn a_random_secondary_color_mix_spreads_from_zero_to_the_value() {
    let mut b = dots(BLACK);
    b.color.secondary = WHITE;
    b.color.sub_mix = 1.0;
    b.influences.color_sub_mix.random = used(0.0);
    let mut greys = Vec::new();
    for seed in 1..=3 {
        b.seed = seed;
        greys.extend(upright(&b).into_iter().map(|c| {
            assert!(c.r == c.g && c.g == c.b);
            c.r
        }));
    }
    assert!(greys.iter().min().unwrap() < &30 && greys.iter().max().unwrap() > &225);
    let mean = greys.iter().map(|&g| g as f64).sum::<f64>() / greys.len() as f64;
    assert!((mean - 127.5).abs() < 20.0, "一様な平均 {mean}");
    // 値が 0.4 なら 0〜0.4 × 255
    b.color.sub_mix = 0.4;
    assert!(upright(&b).iter().all(|c| c.r <= 103));
}

#[test]
fn a_stroke_change_is_one_color_per_stroke_and_differs_between_seeds() {
    let mut seen = std::collections::HashSet::new();
    for seed in 1..=8 {
        let mut b = dots(BASE);
        b.color.stroke = Some(StrokeColorChange {
            hue: 90.0,
            saturation: 0.0,
            value: 0.0,
            sub_mix: 0.0,
        });
        b.seed = seed;
        let colors = upright(&b);
        assert!(colors.iter().all(|c| *c == colors[0]), "1 本の中は同じ色");
        assert!(hue_degrees(colors[0]).abs() <= 92.0);
        seen.insert((colors[0].r, colors[0].g, colors[0].b));
    }
    assert!(seen.len() >= 6, "種を変えると色が変わる: {}", seen.len());
}

#[test]
fn the_tip_and_stroke_hue_changes_add_up_past_either_limit() {
    let mut beyond = 0;
    for seed in 1..=8 {
        let mut b = with_tip(dots(BASE), 90.0, 0.0, 0.0);
        b.influences.color_hue.random = used(-1.0);
        b.color.stroke = Some(StrokeColorChange {
            hue: 90.0,
            ..StrokeColorChange::default()
        });
        b.seed = seed;
        beyond += upright(&b)
            .into_iter()
            .filter(|c| hue_degrees(*c).abs() > 95.0)
            .count();
    }
    assert!(beyond > 20, "足し算で単独の上限（90 度）を超える: {beyond}");
}

#[test]
fn a_secondary_only_target_leaves_the_color_but_the_secondary_mix_still_applies() {
    let mut b = with_tip(dots(BASE), 90.0, 0.5, 0.5);
    b.influences.color_hue.random = used(-1.0);
    b.color.stroke = Some(StrokeColorChange {
        hue: 90.0,
        saturation: 0.5,
        value: 0.5,
        sub_mix: 0.5,
    });
    for target in [ColorTarget::Main, ColorTarget::MainAndSub] {
        b.color.target = target;
        let colors = upright(&b);
        assert!(colors.iter().any(|c| *c != BASE), "{target:?} は当てる");
    }
    b.color.target = ColorTarget::Sub;
    assert!(upright(&b).iter().all(|c| *c == BASE), "色は変わらない");
    b.color.secondary = WHITE;
    b.color.sub_mix = 1.0;
    assert!(upright(&b).iter().all(|c| *c == WHITE), "混合率は効く");
}

#[test]
fn a_tilt_influence_scales_the_change_by_the_upright_amount() {
    let mut b = with_tip(dots(BASE), 0.0, 0.0, 0.4);
    b.influences.color_value.tilt = InputUse::STRAIGHT;
    let up = dab_colors(&b, 1.0, DVec2::ZERO)[0];
    assert!(close(up.r, 255.0 * (128.0 / 255.0 + 0.4)), "{up:?}");
    let lean = dab_colors(&b, 1.0, DVec2::new(0.6, 0.2))[0];
    assert!(lean.r < up.r && lean.r >= BASE.r, "{lean:?}");
    // 傾きを使わない設定では傾けても同じ
    let mut plain = with_tip(dots(BASE), 0.0, 0.0, 0.4);
    plain.influences.color_value.pressure = InputUse::OFF;
    assert_eq!(
        dab_colors(&plain, 1.0, DVec2::new(0.6, 0.2))[0],
        dab_colors(&plain, 1.0, DVec2::ZERO)[0]
    );
}

#[test]
fn an_unused_change_leaves_the_photoshop_style_colors_and_the_bytes_alone() {
    let ps = ColorDynamics {
        secondary: WHITE,
        foreground_background: 0.4,
        hue: 0.3,
        brightness: 0.2,
        per_tip: true,
        ..ColorDynamics::default()
    };
    let mut plain = dots(BASE);
    plain.color = ps;
    plain.seed = 5;
    let expected = upright(&plain);
    // 影響元だけ入れて変化は切のブラシ: 乱数を 1 つも引かない
    let mut off = plain.clone();
    off.influences.color_hue.random = used(-1.0);
    off.influences.color_sub_mix.pressure = InputUse::STRAIGHT;
    assert_eq!(upright(&off), expected);
    // 変化が値 0 で動いていても（ダブごとの経路・乱数の列を使う）、Photoshop の形の色の列は変わらない
    let mut zero = plain.clone();
    zero.color.tip = Some(TipColorChange::default());
    zero.influences.color_hue.random = used(-1.0);
    zero.influences.color_value.random = used(-1.0);
    assert_eq!(upright(&zero), expected);
    assert!(
        expected.windows(2).any(|w| w[0] != w[1]),
        "テストの前提: ゆらぐ"
    );
}

#[test]
fn a_constant_change_applies_on_top_of_each_photoshop_style_color() {
    let mut b = dots(BASE);
    b.color = ColorDynamics {
        secondary: WHITE,
        foreground_background: 0.5,
        hue: 0.3,
        per_tip: true,
        ..ColorDynamics::default()
    };
    b.seed = 3;
    let plain = upright(&b);
    assert!(
        plain.windows(2).any(|w| w[0] != w[1]),
        "テストの前提: ゆらぐ"
    );
    // 一定のサブ描画色混合率 1: どのダブもサブ描画色（Photoshop の形の色は上書きされる）
    b.color.sub_mix = 1.0;
    assert!(upright(&b).iter().all(|c| *c == WHITE));
    // ストロークごとの変化だけなら、ダブごとの Photoshop の形の色のゆらぎが残り、全部に同じ変化量が足される
    b.color.sub_mix = 0.0;
    b.color.stroke = Some(StrokeColorChange {
        value: 0.3,
        ..StrokeColorChange::default()
    });
    let shifted = upright(&b);
    assert_eq!(shifted.len(), plain.len());
    assert!(shifted.windows(2).any(|w| w[0] != w[1]));
    assert_ne!(shifted, plain, "変化量が足される");
}

#[test]
fn out_of_range_and_non_finite_changes_are_refused() {
    let tip = |hue, saturation, value| {
        let mut b = dots(BASE);
        b.color.tip = Some(TipColorChange {
            hue,
            saturation,
            value,
        });
        b
    };
    let stroke = |hue, saturation, value, sub_mix| {
        let mut b = dots(BASE);
        b.color.stroke = Some(StrokeColorChange {
            hue,
            saturation,
            value,
            sub_mix,
        });
        b
    };
    assert!(tip(360.0, 1.0, -1.0).validate().is_ok());
    assert!(stroke(180.0, 1.0, 1.0, 1.0).validate().is_ok());
    let mut sub = dots(BASE);
    sub.color.sub_mix = 1.0;
    assert!(sub.validate().is_ok());
    let bad = [
        tip(360.1, 0.0, 0.0),
        tip(-361.0, 0.0, 0.0),
        tip(0.0, 1.01, 0.0),
        tip(0.0, 0.0, -1.01),
        tip(f64::NAN, 0.0, 0.0),
        tip(0.0, f64::INFINITY, 0.0),
        stroke(180.5, 0.0, 0.0, 0.0),
        stroke(-1.0, 0.0, 0.0, 0.0),
        stroke(0.0, 1.5, 0.0, 0.0),
        stroke(0.0, 0.0, 0.0, f64::NAN),
        {
            let mut b = dots(BASE);
            b.color.sub_mix = 1.5;
            b
        },
        {
            let mut b = dots(BASE);
            b.color.sub_mix = -0.1;
            b
        },
    ];
    for (i, b) in bad.iter().enumerate() {
        assert!(b.validate().is_err(), "{i}");
    }
}

#[test]
fn a_negative_input_minimum_is_only_for_the_hue_saturation_and_value() {
    // 色相・彩度・明度の応えは −1〜1、ほかの項目とサブ描画色混合率は 0〜1
    for item in ColorItem::ALL {
        let mut b = dots(BASE);
        b.influences.color_mut(item).random = used(-0.5);
        assert_eq!(b.validate().is_ok(), item.signed(), "{item:?}");
    }
    let mut b = dots(BASE);
    b.influences.size.random = used(-0.5);
    assert!(b.validate().is_err());
    b.influences.size.random = used(0.5);
    assert!(b.validate().is_ok());
    // 応えの範囲の外は、作るときに断る
    assert!(InputResponse::new_signed(-1.5, vec![]).is_err());
    assert!(InputResponse::new_signed(f64::NAN, vec![]).is_err());
    assert!(InputResponse::new(-0.5, vec![]).is_err());
    let mut b = dots(BASE);
    b.influences.color_hue.random = used(-1.0);
    assert!(b.validate().is_ok());
}

#[test]
fn the_color_math_matches_the_textbook_conversion() {
    // 基準の明度 0.5・彩度 0.5 を 90 度回すと、赤から黄緑へ（色相 0.25）
    let b = with_tip(dots(BASE), 90.0, 0.0, 0.0);
    let c = upright_at(&b, 1.0);
    let (h, s, v) = rgb_to_hsv(128.0 / 255.0, 64.0 / 255.0, 64.0 / 255.0);
    let (r, g, bl) = hsv_to_rgb(h + 0.25, s, v);
    assert_eq!(
        c,
        Rgba8::new(
            (r * 255.0).round() as u8,
            (g * 255.0).round() as u8,
            (bl * 255.0).round() as u8,
            255
        )
    );
}

#[test]
fn undo_and_redo_of_a_stroke_with_a_random_color_change_restore_the_exact_bytes() {
    let mut b = with_tip(dots(BASE), 90.0, 0.4, 0.4);
    b.color.secondary = WHITE;
    b.color.sub_mix = 0.6;
    b.influences.color_hue.random = used(-1.0);
    b.influences.color_value.random = used(-1.0);
    b.influences.color_sub_mix.random = used(0.0);
    b.seed = 7;
    let mut d = Document::with_tile_size(WIDTH, 8, 16).unwrap();
    let l = d.add_layer("L").unwrap();
    d.clear_history().unwrap();
    let before = d.composite(d.bounds()).unwrap();
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    for i in 0..((WIDTH - 16) / 8) {
        let x = 8.0 + 8.0 * i as f64;
        s.add_sample(
            &mut d,
            BrushSample::new(x, 4.0, 1.0, 0.0, DVec2::ZERO).unwrap(),
        )
        .unwrap();
    }
    d.end_stroke(s).unwrap();
    let drawn = d.composite(d.bounds()).unwrap();
    assert_ne!(drawn, before, "テストの前提: 描けている");
    // 色がダブごとにゆらいでいる（元に戻すだけで済む 1 色の線ではない）
    let colors: std::collections::HashSet<_> =
        drawn.chunks_exact(4).filter(|p| p[3] != 0).collect();
    assert!(colors.len() > 20, "テストの前提: 色が散る {}", colors.len());
    assert!(d.undo().unwrap());
    assert_eq!(d.composite(d.bounds()).unwrap(), before, "Undo で描く前");
    assert!(d.redo().unwrap());
    assert_eq!(d.composite(d.bounds()).unwrap(), drawn, "Redo で描いた後");
}
