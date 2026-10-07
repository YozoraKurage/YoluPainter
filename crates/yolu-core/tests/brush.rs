//! ブラシの振る舞い（Unity 版の C# の Core の試験 BrushTests・BrushDynamicsTests・StrokeAssistTests・StrokeCurveTests・
//! BrushEffectTests のうち、1 つの面へ描くストロークの範囲を移したもの。値は C# の試験の期待値そのもの）と、C# に無い拡張
//! （筆先の反転・紙の質感のモード）の試験。マスクへの効果のブラシはここ、複数チャンネルは material.rs、スレッド数は parallelism.rs、保存の部分はまだ無いので移していない（選択範囲は selection.rs、透明部分のロックは docops.rs の試験）。
//! 束に入れず直下の 1 本: ワーカーの閾値（`yolu_core::brush::set_parallel_dab_pixels`。プロセスで 1 つ）を試験の間だけ 1 にして戻す。束のほかの試験のダブの経路を変え、戻すときに、同じ時に走るほかの試験が決めた値も消す。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::collections::HashSet;
use std::f64::consts::PI;
use std::sync::Arc;

use yolu_core::brush::random::NetRandom;
use yolu_core::brush::{curve, pen_tilt, set_parallel_dab_pixels};
use yolu_core::glam::DVec2;
use yolu_core::{
    builtin_presets, builtin_tip, Brush, BrushEffect, BrushPixel, BrushSample, BrushSettings,
    BrushTip, Channel, ColorDynamics, CoreError, Document, DualBrush, DualBrushMode, LayerId,
    PaperTexture, Rgba8, RowOrder, TextureMode, TipSelection,
};

const BLACK: Rgba8 = Rgba8::new(0, 0, 0, 255);
const INK: Rgba8 = Rgba8::new(200, 60, 30, 255);

fn doc(w: u32, h: u32, tile: u32) -> (Document, LayerId) {
    let mut d = Document::with_tile_size(w, h, tile).unwrap();
    let l = d.add_layer("L").unwrap();
    d.clear_history().unwrap();
    (d, l)
}
fn sample(x: f64, y: f64) -> BrushSample {
    BrushSample::new(x, y, 1.0, 0.0, DVec2::ZERO).unwrap()
}
fn sample_at(x: f64, y: f64, pressure: f64, time: f64) -> BrushSample {
    BrushSample::new(x, y, pressure, time, DVec2::ZERO).unwrap()
}
fn tilted(x: f64, y: f64, tx: f64, ty: f64) -> BrushSample {
    BrushSample::new(x, y, 1.0, 0.0, DVec2::new(tx, ty)).unwrap()
}
/// 線分を segments 等分した点（C# の試験の Line。時刻 0）。
fn line(x0: f64, y0: f64, x1: f64, y1: f64, segments: usize) -> Vec<BrushSample> {
    (0..=segments)
        .map(|i| {
            let t = i as f64 / segments as f64;
            sample(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t)
        })
        .collect()
}
fn paint(d: &mut Document, l: LayerId, brush: &Brush, samples: &[BrushSample]) -> bool {
    paint_in(d, l, Channel::Color, brush, samples)
}
fn paint_in(
    d: &mut Document,
    l: LayerId,
    channel: Channel,
    brush: &Brush,
    samples: &[BrushSample],
) -> bool {
    let mut s = d.begin_brush_stroke_in(l, channel, brush).unwrap();
    for p in samples {
        s.add_sample(d, *p).unwrap();
    }
    d.end_stroke(s).unwrap().changed
}
fn composite(d: &Document, channel: Channel) -> Vec<u8> {
    let mut out = vec![0u8; d.width() as usize * d.height() as usize * 4];
    d.composite_into(channel, d.bounds(), &mut out, RowOrder::BottomUp)
        .unwrap();
    out
}
fn all(d: &Document) -> Vec<u8> {
    d.composite(d.bounds()).unwrap()
}
fn alpha_at(c: &[u8], w: usize, x: usize, y: usize) -> u8 {
    c[(y * w + x) * 4 + 3]
}
fn alphas(c: &[u8]) -> Vec<u8> {
    c.chunks_exact(4).map(|p| p[3]).collect()
}
fn colours(c: &[u8], min_alpha: u8) -> HashSet<(u8, u8, u8)> {
    c.chunks_exact(4)
        .filter(|p| p[3] >= min_alpha && p[3] > 0)
        .map(|p| (p[0], p[1], p[2]))
        .collect()
}

// ───────── BrushTests ─────────

fn canvas(size: u32) -> (Document, LayerId) {
    doc(size, size, 16)
}
fn a(d: &Document, x: u32, y: u32) -> u8 {
    d.composite_pixel(Channel::Color, x, y).unwrap().a
}
/// C# の BrushTests.Fixed: 半径 8・硬さ 1・黒・筆圧の割り当てなし・間隔 0.1。
fn fixed() -> Brush {
    Brush::from(BrushSettings {
        radius: 8.0,
        hardness: 1.0,
        color: BLACK,
        pressure_size: false,
        pressure_opacity: false,
        spacing: 0.1,
        ..BrushSettings::default()
    })
}
/// 時刻 0.01 ずつの点で描いて確定する（C# の BrushTests.Stroke）。
fn stroke(d: &mut Document, l: LayerId, brush: &Brush, points: &[(f64, f64)]) {
    let samples: Vec<BrushSample> = points
        .iter()
        .enumerate()
        .map(|(i, p)| sample_at(p.0, p.1, 1.0, (i + 1) as f64 * 0.01))
        .collect();
    paint(d, l, brush, &samples);
}

#[test]
fn overlapping_dabs_never_exceed_the_strokes_opacity() {
    let (mut d, l) = canvas(64);
    let mut b = fixed();
    b.base.opacity = 0.5;
    stroke(&mut d, l, &b, &[(10.0, 32.0), (54.0, 32.0)]); // 何十個も重なる
    assert_eq!(a(&d, 32, 32), 128, "不透明度 0.5 で止まる");
    stroke(&mut d, l, &b, &[(32.0, 10.0), (32.0, 54.0)]);
    assert_eq!(
        a(&d, 32, 32),
        192,
        "新しいストロークは前の結果の上に重なる: 128/255 + 0.5 × (1 − 128/255)"
    );
}

#[test]
fn low_flow_builds_up_toward_the_opacity_ceiling() {
    let (mut d, l) = canvas(64);
    let mut b = fixed();
    b.base.flow = 0.25;
    b.base.opacity = 0.8;
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    s.apply_pixel(&mut d, 5, 5, 1.0, 1.0).unwrap();
    assert_eq!(a(&d, 5, 5), 51, "0.8 × 0.25");
    s.apply_pixel(&mut d, 5, 5, 1.0, 1.0).unwrap();
    assert_eq!(a(&d, 5, 5), 89, "0.2 + (0.8 − 0.2) × 0.25 = 0.35");
    for _ in 0..60 {
        s.apply_pixel(&mut d, 5, 5, 1.0, 1.0).unwrap();
    }
    assert_eq!(a(&d, 5, 5), 204, "天井の 0.8 に寄る");
    d.end_stroke(s).unwrap();
    assert!(d.undo().unwrap());
    assert_eq!(a(&d, 5, 5), 0);
}

fn upper_half_tip() -> Arc<BrushTip> {
    let mut alpha = vec![0u8; 16 * 16];
    for y in 8..16 {
        for x in 0..16 {
            alpha[y * 16 + x] = 255;
        }
    }
    Arc::new(BrushTip::new("upper half", 16, 16, alpha).unwrap())
}

#[test]
fn a_tip_image_is_placed_upright_and_rotates_counter_clockwise() {
    // 上半分だけ埋まった筆先: 角度 0 では中心より上（Y が大きい側）だけ塗れる。90° 回すと左側になる
    let tip = upper_half_tip();
    let (mut d, l) = canvas(64);
    let mut b = fixed();
    b.tip.image = Some(tip.clone());
    paint(&mut d, l, &b, &[sample(32.0, 32.0)]);
    assert_eq!(a(&d, 32, 36), 255);
    assert_eq!(a(&d, 32, 28), 0);
    let (mut r, rl) = canvas(64);
    b.tip.angle = 90.0;
    paint(&mut r, rl, &b, &[sample(32.0, 32.0)]);
    assert_eq!(a(&r, 28, 32), 255, "90° 回すと埋まった半分が左を向く");
    assert_eq!(a(&r, 36, 32), 0);
}

#[test]
fn roundness_squashes_and_follow_direction_turns_the_tip() {
    let (mut d, l) = canvas(64);
    let mut b = fixed();
    b.tip.roundness = 0.25;
    paint(&mut d, l, &b, &[sample(32.0, 32.0)]);
    assert_eq!(a(&d, 38, 32), 255, "筆先の横の軸に沿っては幅いっぱい");
    assert_eq!(a(&d, 32, 36), 0, "直交する向きは 4 分の 1");
    // 縦に動くストロークで向きに沿わせると、潰れた筆先は縦向きの線になる（横に細い）
    let (mut f, fl) = canvas(64);
    b.tip.follow_direction = true;
    stroke(&mut f, fl, &b, &[(32.0, 10.0), (32.0, 54.0)]);
    assert_eq!(a(&f, 32, 40), 255);
    assert_eq!(a(&f, 36, 40), 0);
}

#[test]
fn jitter_and_scatter_are_deterministic_per_seed_and_stay_in_range() {
    let wild = |seed: i32| {
        let mut b = fixed();
        b.seed = seed;
        b.jitter.scatter = 1.0;
        b.jitter.count = 4;
        b.jitter.size = 0.8;
        b.jitter.angle = 1.0;
        b.jitter.opacity = 0.9;
        b.jitter.flow = 0.5;
        b.base.spacing = 0.5;
        b
    };
    let draw = |seed: i32| {
        let (mut d, l) = canvas(128);
        stroke(&mut d, l, &wild(seed), &[(40.0, 64.0), (88.0, 64.0)]);
        all(&d)
    };
    assert_eq!(draw(7), draw(7), "同じ種なら同じ画素");
    assert_ne!(draw(7), draw(8), "違う種なら散布が違う");
    let pixels = draw(3);
    for y in 0..128usize {
        for x in 0..128usize {
            if pixels[(y * 128 + x) * 4 + 3] > 0 {
                assert!(
                    (40..=88).contains(&y) && (16..=112).contains(&x),
                    "({x},{y}) は散布 + 半径の外"
                );
            }
        }
    }
}

fn stripes() -> Arc<BrushTip> {
    let mut a = vec![0u8; 16];
    for y in 0..4 {
        for x in 0..2 {
            a[y * 4 + x] = 255; // 2 列おきの縞
        }
    }
    Arc::new(BrushTip::new("stripes", 4, 4, a).unwrap())
}

#[test]
fn a_paper_texture_modulates_coverage() {
    let (mut d, l) = canvas(64);
    let mut b = fixed();
    b.texture = Some(PaperTexture::new(stripes(), 1.0));
    stroke(&mut d, l, &b, &[(16.0, 32.0), (48.0, 32.0)]);
    assert_eq!(a(&d, 32, 32), 255, "x=32 は埋まった縞");
    assert_eq!(a(&d, 34, 32), 0, "x=34 は空の縞");
    let (mut h, hl) = canvas(64);
    b.texture = Some(PaperTexture::new(stripes(), 0.5));
    stroke(&mut h, hl, &b, &[(16.0, 32.0), (48.0, 32.0)]);
    assert_eq!(
        a(&h, 34, 32),
        128,
        "深さ 0.5 なら空の縞は、何個重なっても半分で止まる"
    );
}

#[test]
fn the_round_tip_is_unchanged_by_the_new_engine() {
    // 角度 0・丸さ 1・ゆらぎ無しの丸ブラシは、1 回のダブが従来の式（硬さの smoothstep）そのもの
    let (mut d, l) = canvas(64);
    let mut b = fixed();
    b.base.hardness = 0.5;
    b.base.radius = 10.0;
    paint(&mut d, l, &b, &[sample(32.3, 31.7)]);
    for y in 20..44u32 {
        for x in 20..44u32 {
            let (dx, dy) = (x as f64 + 0.5 - 32.3, y as f64 + 0.5 - 31.7);
            let dist = (dx * dx + dy * dy).sqrt() / 10.0;
            let mut expected = 0.0;
            if dist <= 1.0 {
                expected = 1.0;
                if dist > 0.5 {
                    let t = (1.0 - dist) / 0.5;
                    expected = t * t * (3.0 - 2.0 * t);
                }
            }
            assert_eq!(
                a(&d, x, y),
                (expected * 255.0 + 0.5).floor() as u8,
                "({x},{y})"
            );
        }
    }
}

#[test]
fn brush_settings_validate_their_input() {
    // C# BrushTests.BrushTipsValidateTheirInput の設定の部分
    let mut b = fixed();
    b.jitter.count = 0;
    assert!(b.validate().is_err());
    let mut b = fixed();
    b.tip.roundness = 0.0;
    assert!(b.validate().is_err());
    let mut b = fixed();
    b.jitter.size = 1.5;
    assert!(b.validate().is_err());
    let t = BrushTip::new("copy", 2, 2, vec![0, 255, 255, 0]).unwrap();
    assert_eq!(t.sample(-0.1, 0.5), 0.0);
    assert!(
        (t.sample_tiled(-1.5, 2.5) - t.sample_tiled(0.5, 0.5)).abs() < 1e-9,
        "並べた読みは回る"
    );
}

#[test]
fn every_builtin_brush_paints_deterministically() {
    let presets = builtin_presets();
    assert!(presets.len() >= 12);
    let ids: HashSet<_> = presets.iter().map(|p| p.id).collect();
    assert_eq!(ids.len(), presets.len(), "名前は重ならない");
    for preset in &presets {
        let draw = || {
            let (mut d, l) = canvas(128);
            let mut b = preset.brush.clone();
            b.base.color = BLACK;
            if b.base.erase {
                for y in 0..128 {
                    for x in 0..128 {
                        d.set_pixel(l, x, y, BLACK).unwrap();
                    }
                }
                d.clear_history().unwrap();
            }
            paint(
                &mut d,
                l,
                &b,
                &[
                    sample_at(30.0, 60.0, 0.8, 0.0),
                    sample_at(70.0, 70.0, 1.0, 0.01),
                    sample_at(100.0, 62.0, 0.6, 0.02),
                ],
            );
            all(&d)
        };
        let first = draw();
        assert_eq!(first, draw(), "{} は決まった結果", preset.id);
        let a = alphas(&first);
        let mark = if preset.brush.base.erase {
            a.iter().any(|&v| v < 255)
        } else {
            a.iter().any(|&v| v > 0)
        };
        assert!(mark, "{} は跡を残す", preset.id);
        preset.brush.validate().unwrap();
    }
    assert!(Arc::ptr_eq(
        &builtin_tip("grain").unwrap(),
        &builtin_tip("grain").unwrap()
    ));
    assert!(builtin_tip("missing").is_none());
    let chalk = presets.iter().find(|p| p.id == "chalk").unwrap();
    assert_eq!(chalk.brush.base.radius, 18.0);
}

// ───────── BrushDynamicsTests ─────────

/// C# の BrushDynamicsTests.Hard: 硬さ 1・間隔 0.25・INK・筆圧の割り当てなし。
fn hard(radius: f64) -> Brush {
    Brush::from(BrushSettings {
        radius,
        hardness: 1.0,
        spacing: 0.25,
        color: INK,
        pressure_size: false,
        pressure_opacity: false,
        ..BrushSettings::default()
    })
}
/// 96×64・タイル 32 の文書にストローク 1 本を描いた合成（C# の BrushDynamicsTests.Draw）。
fn draw_in(brush: &Brush, channel: Channel, samples: &[BrushSample]) -> Vec<u8> {
    let (mut d, l) = doc(96, 64, 32);
    paint_in(&mut d, l, channel, brush, samples);
    composite(&d, channel)
}
fn draw(brush: &Brush, samples: &[BrushSample]) -> Vec<u8> {
    draw_in(brush, Channel::Color, samples)
}
fn a96(c: &[u8], x: usize, y: usize) -> u8 {
    alpha_at(c, 96, x, y)
}

#[test]
fn default_brush_has_no_dynamics_and_paints_like_the_m1_settings() {
    let b = Brush::default();
    assert!(!b.color.is_active());
    assert!(b.dual.is_none() && b.texture.is_none());
    assert!(b.color.per_tip);
    let c = b.controls;
    assert_eq!([c.fade_size, c.fade_opacity, c.fade_flow], [0, 0, 0]);
    assert!(!(c.tilt_size || c.tilt_opacity || c.tilt_flow || c.tilt_angle));
    // M1 の口（BrushSettings）と全部入りの口（Brush::from）は同じ画素
    let s = BrushSettings {
        radius: 9.0,
        hardness: 0.3,
        spacing: 0.1,
        flow: 0.5,
        pressure_flow: true,
        color: INK,
        ..BrushSettings::default()
    };
    let path: Vec<BrushSample> = (0..12)
        .map(|i| {
            sample_at(
                8.0 + i as f64 * 7.0,
                30.0 + (i as f64).sin() * 9.0,
                0.3 + 0.06 * i as f64,
                0.0,
            )
        })
        .collect();
    let (mut d1, l1) = doc(96, 64, 32);
    let mut s1 = d1.begin_stroke(l1, &s).unwrap();
    for p in &path {
        s1.add_sample(&mut d1, *p).unwrap();
    }
    d1.end_stroke(s1).unwrap();
    assert_eq!(all(&d1), draw(&Brush::from(s), &path));
}

#[test]
fn colour_jitters_stay_in_their_ranges() {
    let mut random = NetRandom::new(1);
    // 色相 ±0.1 × 180° = ±18°。純赤からなら R = 255、片方は 0、もう片方は 255 × 6 × 0.05 = 76.5 → 77 まで
    let hue = ColorDynamics {
        hue: 0.1,
        ..ColorDynamics::default()
    };
    let reds: Vec<Rgba8> = (0..500)
        .map(|_| hue.next(Rgba8::new(255, 0, 0, 255), &mut random))
        .collect();
    assert!(reds
        .iter()
        .all(|c| c.r == 255 && c.g.min(c.b) == 0 && c.g.max(c.b) <= 77 && c.a == 255));
    assert!(
        reds.iter().any(|c| c.g > 40) && reds.iter().any(|c| c.b > 40),
        "円の両方の向き"
    );
    // 明るさ ±0.2: 灰 128（v = 0.502）は 0.302〜0.702 → 77〜179、灰のまま
    let bright = ColorDynamics {
        brightness: 0.2,
        ..ColorDynamics::default()
    };
    let greys: Vec<Rgba8> = (0..500)
        .map(|_| bright.next(Rgba8::new(128, 128, 128, 255), &mut random))
        .collect();
    assert!(greys
        .iter()
        .all(|c| c.r == c.g && c.g == c.b && (77..=179).contains(&c.r)));
    assert!(greys.iter().map(|c| c.r).min().unwrap() < 90);
    assert!(greys.iter().map(|c| c.r).max().unwrap() > 165);
    // 描画色/背景色 0.5: 黒から白へ半分まで
    let mix = ColorDynamics {
        secondary: Rgba8::new(255, 255, 255, 255),
        foreground_background: 0.5,
        ..ColorDynamics::default()
    };
    let mixed: Vec<Rgba8> = (0..500)
        .map(|_| mix.next(Rgba8::new(0, 0, 0, 255), &mut random))
        .collect();
    assert!(mixed.iter().all(|c| c.r == c.g && c.g == c.b && c.r <= 128));
    assert!(mixed.iter().map(|c| c.r).max().unwrap() > 110);
    // 純度: (255,128,128) の彩度 0.498 → +0.5 で 0.749 → (255,64,64)。−0.5 で純赤は (255,128,128)
    let purity = |p: f64| ColorDynamics {
        purity: p,
        ..ColorDynamics::default()
    };
    assert_eq!(
        purity(0.5).next(Rgba8::new(255, 128, 128, 255), &mut random),
        Rgba8::new(255, 64, 64, 255)
    );
    assert_eq!(
        purity(-0.5).next(Rgba8::new(255, 0, 0, 90), &mut random),
        Rgba8::new(255, 128, 128, 90),
        "アルファは保つ"
    );
    // 0 の項目は乱数を引かない
    let (mut x, mut y) = (NetRandom::new(5), NetRandom::new(5));
    purity(0.3).next(Rgba8::new(255, 255, 255, 255), &mut x);
    assert_eq!(x.next_double(), y.next_double());
}

#[test]
fn per_tip_colours_are_deterministic_per_seed_and_never_move_the_dabs() {
    let path = line(8.0, 30.0, 88.0, 34.0, 8);
    let jittery = |seed: i32| {
        let mut b = hard(5.0);
        b.base.spacing = 0.3;
        b.jitter.scatter = 0.5;
        b.jitter.count = 2;
        b.jitter.size = 0.4;
        b.seed = seed;
        b.color.hue = 1.0;
        b.color.brightness = 0.3;
        b
    };
    let first = draw(&jittery(3), &path);
    assert_eq!(draw(&jittery(3), &path), first, "同じ種なら同じ画素");
    assert_ne!(draw(&jittery(4), &path), first);
    assert!(colours(&first, 255).len() > 5, "ダブごとに新しい色");
    let mut plain = jittery(3);
    plain.color.hue = 0.0;
    plain.color.brightness = 0.0;
    assert_eq!(
        alphas(&first),
        alphas(&draw(&plain, &path)),
        "色の乱数は別の列: ダブの位置は動かない"
    );
}

#[test]
fn per_stroke_colour_is_one_colour_and_per_tip_colours_respect_the_opacity_ceiling() {
    let path = line(8.0, 30.0, 88.0, 34.0, 8);
    let mut once = hard(5.0);
    once.base.hardness = 0.4;
    once.color.hue = 0.8;
    once.color.per_tip = false;
    once.seed = 9;
    let c = draw(&once, &path);
    let set = colours(&c, 1);
    assert_eq!(
        set.len(),
        1,
        "塗った画素（柔らかい縁も）はストロークの 1 色"
    );
    assert_ne!(*set.iter().next().unwrap(), (200, 60, 30));
    let mut half = hard(5.0);
    half.base.opacity = 0.5;
    half.base.spacing = 0.1;
    half.color.hue = 1.0;
    let h = draw(&half, &path);
    assert_eq!(
        *alphas(&h).iter().max().unwrap(),
        128,
        "重なったダブは色を変えるが天井は越えない"
    );
    assert!(colours(&h, 128).len() > 5);
}

#[test]
fn colour_dynamics_are_ignored_on_data_channels_and_erasing() {
    let path = line(8.0, 30.0, 88.0, 34.0, 6);
    let mut dynamic = hard(5.0);
    dynamic.base.color = Rgba8::new(180, 180, 180, 255);
    dynamic.color.hue = 1.0;
    dynamic.color.foreground_background = 1.0;
    dynamic.color.secondary = Rgba8::new(10, 250, 10, 255);
    dynamic.color.purity = 0.5;
    let mut plain = hard(5.0);
    plain.base.color = dynamic.base.color;
    for ch in [Channel::Roughness, Channel::Metallic, Channel::Height] {
        assert_eq!(
            draw_in(&dynamic, ch, &path),
            draw_in(&plain, ch, &path),
            "{ch:?} はデータ: 値そのものを塗る"
        );
    }
    assert_ne!(
        draw_in(&dynamic, Channel::Emission, &path),
        draw_in(&plain, Channel::Emission, &path),
        "Emission は色"
    );
    let erased = |b: &Brush| {
        let (mut d, l) = doc(96, 64, 32);
        paint(&mut d, l, &hard(12.0), &path);
        paint(&mut d, l, b, &path);
        all(&d)
    };
    let mut erase = hard(5.0);
    erase.base.erase = true;
    let mut erase_dynamic = erase.clone();
    erase_dynamic.color.hue = 1.0;
    erase_dynamic.color.foreground_background = 1.0;
    assert_eq!(erased(&erase_dynamic), erased(&erase));
}

#[test]
fn the_surface_brush_paints_the_strokes_one_colour() {
    let (mut d, l) = doc(96, 64, 32);
    let mut b = hard(6.0);
    b.color.hue = 1.0;
    b.seed = 2;
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    for x in 10..40 {
        s.apply_pixel(&mut d, x, 20, if x % 2 == 0 { 1.0 } else { 0.5 }, 1.0)
            .unwrap();
    }
    d.end_stroke(s).unwrap();
    assert_eq!(
        colours(&all(&d), 1).len(),
        1,
        "面のダブには筆先の境が無いので、色はストロークに 1 回"
    );
}

#[test]
fn a_hard_dual_tip_multiplies_the_main_tip() {
    let mut main = hard(10.0);
    main.dual = Some(DualBrush {
        radius: 4.0,
        hardness: 1.0,
        ..DualBrush::default()
    });
    let dot = [sample(20.0, 20.0)];
    assert_eq!(
        draw(&main, &dot),
        draw(&hard(4.0), &dot),
        "硬い 10 px × 硬い 4 px = 4 px の点"
    );
    // 柔らかい 2 つ目（硬さ 0、半径 4）: 中心から 2 px で smoothstep(0.5) = 0.5 → アルファ 128
    let mut soft = hard(10.0);
    soft.dual = Some(DualBrush {
        radius: 4.0,
        hardness: 0.0,
        ..DualBrush::default()
    });
    let c = draw(&soft, &[sample(20.5, 20.5)]);
    assert_eq!(a96(&c, 20, 20), 255);
    assert_eq!(a96(&c, 22, 20), 128);
    assert_eq!(a96(&c, 25, 20), 0);
    assert_eq!(c[(20 * 96 + 22) * 4], 200, "色は変わらない");
}

#[test]
fn dual_brush_is_deterministic_and_independent_of_the_input_rate() {
    let s = |seed: i32| {
        let mut b = hard(6.0);
        b.seed = seed;
        b.dual = Some(DualBrush {
            radius: 5.0,
            hardness: 0.5,
            spacing: 0.37,
            scatter: 0.8,
            count: 2,
            mode: DualBrushMode::Multiply,
            ..DualBrush::default()
        });
        b
    };
    let sparse = draw(&s(1), &[sample(8.0, 32.0), sample(88.0, 32.0)]);
    let dense = draw(&s(1), &line(8.0, 32.0, 88.0, 32.0, 50));
    assert_eq!(
        dense, sparse,
        "主のダブが見る 2 つ目のダブは線の長さで決まり、入力の区切り方によらない"
    );
    assert_ne!(
        draw(&s(2), &[sample(8.0, 32.0), sample(88.0, 32.0)]),
        sparse
    );
    let mut plain = hard(6.0);
    plain.seed = 1;
    let count = |c: &[u8]| alphas(c).iter().filter(|&&v| v > 0).count();
    assert!(count(&sparse) < count(&draw(&plain, &[sample(8.0, 32.0), sample(88.0, 32.0)])));
}

#[test]
fn fade_shrinks_the_size_over_the_given_stamps() {
    // 半径 8・間隔 4 px: 0 番目 x=10 r=8、1 番目 14 r=6、2 番目 18 r=4、3 番目 22 r=2、4 番目から 0 → 行 32 の右端は 23
    let mut b = hard(8.0);
    b.controls.fade_size = 4;
    let (mut d, l) = doc(96, 64, 32);
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    s.add_sample(&mut d, sample(10.0, 32.0)).unwrap();
    s.add_sample(&mut d, sample(60.0, 32.0)).unwrap();
    let r = d.end_stroke(s).unwrap();
    let c = all(&d);
    assert_eq!((0..96).filter(|&x| a96(&c, x, 32) > 0).max(), Some(23));
    assert_eq!(
        (0..64).filter(|&y| a96(&c, 22, y) > 0).count(),
        4,
        "x = 22 に届くのは 2 px のダブだけ"
    );
    assert_eq!(r.stamps, 13, "消えた描点も数える（0〜48 を 4 px ごと）");
}

#[test]
fn fade_opacity_and_flow_halve_the_second_stamp() {
    // 半径 4・間隔 8 px: x=10, 18, 26。2 段のフェードで 1 番目は半分（128）、2 番目は 0
    for flow in [false, true] {
        let mut b = hard(4.0);
        b.base.spacing = 1.0;
        if flow {
            b.controls.fade_flow = 2;
        } else {
            b.controls.fade_opacity = 2;
        }
        let c = draw(&b, &[sample(10.0, 32.0), sample(40.0, 32.0)]);
        assert_eq!(
            (
                a96(&c, 10, 32),
                a96(&c, 18, 32),
                a96(&c, 14, 32),
                a96(&c, 26, 32)
            ),
            (255, 128, 128, 0),
            "{}",
            if flow { "流量" } else { "不透明度" }
        );
    }
}

#[test]
fn pen_tilt_is_measured_from_upright_and_samples_are_clamped() {
    assert!(
        (pen_tilt::amount(PI / 4.0, -PI / 4.0) - 2f64.sqrt().atan() / (PI / 2.0)).abs() < 1e-12,
        "tan²θ = tan²θx + tan²θy"
    );
    assert!((pen_tilt::azimuth(-PI / 4.0, 0.0) - PI).abs() < 1e-12);
    let s = BrushSample::new(1.0, 2.0, 1.0, 0.0, DVec2::new(5.0, -5.0)).unwrap();
    assert_eq!(s.tilt, DVec2::new(PI / 2.0, -PI / 2.0), "±90° に収める");
    assert!(BrushSample::new(1.0, 2.0, 1.0, 0.0, DVec2::new(f64::NAN, 0.0)).is_err());
}

#[test]
fn tilt_scales_size_and_opacity_and_turns_the_tip() {
    let mut t = hard(8.0);
    t.controls.tilt_size = true;
    assert_eq!(
        draw(&t, &[tilted(20.0, 20.0, PI / 4.0, 0.0)]),
        draw(&hard(4.0), &[sample(20.0, 20.0)]),
        "直立から 45° で大きさは半分"
    );
    assert_eq!(
        draw(&t, &[sample(20.0, 20.0)]),
        draw(&hard(8.0), &[sample(20.0, 20.0)]),
        "傾きの無い入力（マウス）は何も変えない"
    );
    let mut flat = hard(8.0);
    flat.controls.tilt_opacity = true;
    assert!(
        alphas(&draw(&flat, &[tilted(20.0, 20.0, PI / 2.0, 0.0)]))
            .iter()
            .all(|&v| v == 0),
        "寝かせきったペンは塗らない"
    );
    let mut turn = hard(8.0);
    turn.tip.roundness = 0.25;
    turn.controls.tilt_angle = true;
    let mut vertical = hard(8.0);
    vertical.tip.roundness = 0.25;
    vertical.tip.angle = 90.0;
    assert_eq!(
        draw(&turn, &[tilted(30.0, 30.0, 0.0, PI / 4.0)]),
        draw(&vertical, &[sample(30.0, 30.0)]),
        "+Y へ倒すと筆先は 90° 回る"
    );
}

fn every_dynamic() -> Brush {
    let mut s = hard(6.0);
    s.color.hue = 1.0;
    s.color.foreground_background = 0.5;
    s.controls.fade_opacity = 30;
    s.controls.tilt_size = true;
    s.dual = Some(DualBrush {
        radius: 3.0,
        hardness: 0.5,
        scatter: 1.0,
        count: 2,
        mode: DualBrushMode::ColorBurn,
        ..DualBrush::default()
    });
    s
}

#[test]
fn undo_and_cancel_restore_exact_pixels_with_every_dynamic() {
    let s = every_dynamic();
    let (mut d, l) = doc(96, 64, 32);
    paint(&mut d, l, &hard(10.0), &line(5.0, 10.0, 90.0, 50.0, 4));
    let before = all(&d);
    let path: Vec<BrushSample> = line(8.0, 50.0, 88.0, 12.0, 9)
        .iter()
        .map(|p| BrushSample::new(p.x, p.y, 1.0, 0.0, DVec2::new(0.3, 0.2)).unwrap())
        .collect();
    let mut st = d.begin_brush_stroke(l, &s).unwrap();
    for p in &path {
        st.add_sample(&mut d, *p).unwrap();
    }
    d.cancel_stroke(st);
    assert_eq!(all(&d), before, "取消");
    assert!(paint(&mut d, l, &s, &path));
    let after = all(&d);
    assert_ne!(after, before);
    assert!(d.undo().unwrap());
    assert_eq!(all(&d), before);
    assert!(d.redo().unwrap());
    assert_eq!(all(&d), after);
}

#[test]
fn per_tip_colours_and_dual_coverage_count_toward_the_stroke_budget() {
    // 1 タイル（32²）の変更前は空で 0 バイト。覆い 4 B/画素 → 64 + 4096。ダブごとの色は 16 B/画素、デュアルは別に 4 B/画素
    let dot = sample(10.0, 10.0);
    let (mut d, l) = doc(96, 64, 32);
    d.set_stroke_budget_bytes(64 + 32 * 32 * 4 + 100).unwrap();
    paint(&mut d, l, &hard(4.0), &[dot]);
    assert!(d.can_undo(), "普通の点は収まる");
    let painted = all(&d);
    let tip_colours = Brush {
        color: ColorDynamics {
            hue: 1.0,
            ..ColorDynamics::default()
        },
        ..Brush::from(BrushSettings {
            radius: 4.0,
            ..BrushSettings::default()
        })
    };
    let dual = Brush {
        dual: Some(DualBrush {
            radius: 3.0,
            ..DualBrush::default()
        }),
        ..Brush::from(BrushSettings {
            radius: 4.0,
            ..BrushSettings::default()
        })
    };
    for b in [tip_colours, dual] {
        let mut s = d.begin_brush_stroke(l, &b).unwrap();
        assert_eq!(
            s.add_sample(&mut d, dot),
            Err(CoreError::StrokeBudgetExceeded)
        );
        assert!(!d.has_active_stroke(), "ストロークは自分で取り消した");
        assert_eq!(all(&d), painted);
    }
    assert!(d.undo().unwrap());
    assert!(!d.can_undo(), "履歴には普通の点だけ");
}

#[test]
fn invalid_dynamics_are_refused_before_any_pixel_changes() {
    type Change = Box<dyn Fn(&mut Brush)>;
    let bad: Vec<Change> = vec![
        Box::new(|b| b.color.hue = 1.5),
        Box::new(|b| b.color.saturation = -0.1),
        Box::new(|b| b.color.brightness = f64::NAN),
        Box::new(|b| b.color.foreground_background = 2.0),
        Box::new(|b| b.color.purity = -1.01),
        Box::new(|b| b.controls.fade_flow = yolu_core::brush::MAX_FADE + 1),
        Box::new(|b| {
            b.dual = Some(DualBrush {
                radius: 0.0,
                ..DualBrush::default()
            })
        }),
        Box::new(|b| {
            b.dual = Some(DualBrush {
                count: 17,
                ..DualBrush::default()
            })
        }),
        Box::new(|b| {
            b.dual = Some(DualBrush {
                spacing: 0.0,
                ..DualBrush::default()
            })
        }),
        Box::new(|b| {
            b.dual = Some(DualBrush {
                roundness: 0.0,
                ..DualBrush::default()
            })
        }),
        Box::new(|b| {
            b.dual = Some(DualBrush {
                hardness: f64::INFINITY,
                ..DualBrush::default()
            })
        }),
        Box::new(|b| b.texture = Some(PaperTexture::new(stripes(), 1.5))),
        Box::new(|b| {
            b.texture = Some(PaperTexture {
                scale: 0.01,
                ..PaperTexture::new(stripes(), 0.5)
            })
        }),
        Box::new(|b| b.tip.images = vec![stripes(); 257]),
        Box::new(|b| b.jitter.scatter = 11.0),
        Box::new(|b| b.jitter.count = 17),
    ];
    let (mut d, l) = doc(96, 64, 32);
    let before = all(&d);
    for change in &bad {
        let mut b = hard(6.0);
        change(&mut b);
        assert!(matches!(
            d.begin_brush_stroke(l, &b),
            Err(CoreError::InvalidArgument(_))
        ));
    }
    assert_eq!(all(&d), before);
    let s = d.begin_brush_stroke(l, &hard(6.0)).unwrap(); // 断った後も新しいストロークを始められる
    d.cancel_stroke(s);
}

#[test]
fn the_stroke_freezes_its_brush() {
    let mut b = hard(6.0);
    b.dual = Some(DualBrush {
        radius: 3.0,
        ..DualBrush::default()
    });
    b.color.hue = 0.2;
    let path = line(10.0, 30.0, 80.0, 30.0, 5);
    let expected = draw(&b, &path);
    let (mut d, l) = doc(96, 64, 32);
    let mut st = d.begin_brush_stroke(l, &b).unwrap();
    b.dual.as_mut().unwrap().radius = 20.0;
    b.color.hue = 1.0;
    for p in &path {
        st.add_sample(&mut d, *p).unwrap();
    }
    d.end_stroke(st).unwrap();
    assert_eq!(
        all(&d),
        expected,
        "描いている間にブラシを変えても、そのストロークには効かない"
    );
}

// ───────── StrokeAssistTests ─────────

fn assist_hard(radius: f64) -> Brush {
    Brush::from(BrushSettings {
        radius,
        hardness: 1.0,
        spacing: 0.1,
        color: Rgba8::new(0, 0, 0, 255),
        pressure_size: false,
        pressure_opacity: false,
        ..BrushSettings::default()
    })
}
fn assist_draw(b: &Brush, points: &[(f64, f64)]) -> Vec<u8> {
    let (mut d, l) = doc(128, 64, 16);
    let samples: Vec<BrushSample> = points.iter().map(|p| sample(p.0, p.1)).collect();
    paint(&mut d, l, b, &samples);
    all(&d)
}
fn a128(c: &[u8], x: usize, y: usize) -> u8 {
    alpha_at(c, 128, x, y)
}
/// 列 x で塗られた画素の数（線の太さ）。
fn thickness(c: &[u8], x: usize) -> usize {
    (0..64).filter(|&y| a128(c, x, y) > 0).count()
}

#[test]
fn zero_assist_means_the_stroke_is_unchanged() {
    let path = [(10.0, 30.0), (40.0, 35.0), (80.0, 20.0)];
    let plain = assist_draw(&assist_hard(4.0), &path);
    let mut s = assist_hard(4.0);
    s.assist.stabilizer = 0.0;
    s.assist.taper_in = 0.0;
    s.assist.taper_out = 0.0;
    assert_eq!(assist_draw(&s, &path), plain);
}

#[test]
fn the_stabilizer_smooths_jitter_and_finishes_at_the_last_point() {
    let zigzag: Vec<(f64, f64)> = (0..50)
        .map(|i| {
            (
                10.0 + i as f64 * 2.0,
                32.0 + if i % 2 == 0 { 4.0 } else { -4.0 },
            )
        })
        .collect();
    let raw = assist_draw(&assist_hard(2.0), &zigzag);
    let mut s = assist_hard(2.0);
    s.assist.stabilizer = 12.0;
    let smooth = assist_draw(&s, &zigzag);
    let rows = |c: &[u8]| {
        (0..64)
            .filter(|&y| (30..90).any(|x| a128(c, x, y) > 0))
            .count()
    };
    assert!(rows(&smooth) < rows(&raw), "糸より短い揺れはならされる");
    assert!(
        a128(&smooth, 108, 28) as u32
            + a128(&smooth, 108, 36) as u32
            + a128(&smooth, 108, 32) as u32
            > 0,
        "最後の入力の点まで描く"
    );
}

#[test]
fn the_stabilizer_does_not_depend_on_the_input_rate() {
    let mut s = assist_hard(4.0);
    s.assist.stabilizer = 8.0;
    let sparse = assist_draw(&s, &[(10.0, 30.0), (60.0, 30.0), (110.0, 30.0)]);
    let dense: Vec<(f64, f64)> = (0..101).map(|i| (10.0 + i as f64, 30.0)).collect();
    assert_eq!(
        assist_draw(&s, &dense),
        sparse,
        "まっすぐな線は入力の頻度によらず同じ"
    );
}

#[test]
fn tapers_thin_both_ends_and_the_end_waits_for_the_stroke_to_finish() {
    let mut s = assist_hard(6.0);
    s.assist.taper_in = 30.0;
    s.assist.taper_out = 30.0;
    let (mut d, l) = doc(128, 64, 16);
    let mut st = d.begin_brush_stroke(l, &s).unwrap();
    st.add_sample(&mut d, sample(10.0, 32.0)).unwrap();
    st.add_sample(&mut d, sample(110.0, 32.0)).unwrap();
    let during = all(&d);
    assert_eq!(
        a128(&during, 100, 32),
        0,
        "最後の抜きの長さは終わりが分かるまで待つ"
    );
    assert!(a128(&during, 60, 32) > 0);
    d.end_stroke(st).unwrap();
    let c = all(&d);
    assert!(thickness(&c, 14) < thickness(&c, 60), "入り");
    assert!(thickness(&c, 106) < thickness(&c, 60), "抜き");
    assert!(thickness(&c, 100) > 0, "確定で終わりを描く");
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert!(all(&d).iter().all(|&b| b == 0));
}

#[test]
fn cancelling_with_held_back_dabs_leaves_nothing() {
    let mut s = assist_hard(4.0);
    s.assist.taper_out = 40.0;
    s.assist.stabilizer = 5.0;
    let (mut d, l) = doc(128, 64, 16);
    let mut st = d.begin_brush_stroke(l, &s).unwrap();
    st.add_sample(&mut d, sample(10.0, 32.0)).unwrap();
    st.add_sample(&mut d, sample(100.0, 32.0)).unwrap();
    d.cancel_stroke(st);
    assert!(all(&d).iter().all(|&b| b == 0));
    assert_eq!(d.undo_count(), 0);
    assert!(!d.has_active_stroke());
}

#[test]
fn invalid_assist_values_are_refused_and_time_still_has_to_increase() {
    for bad in [
        |b: &mut Brush| b.assist.stabilizer = -1.0,
        |b: &mut Brush| b.assist.taper_in = f64::NAN,
        |b: &mut Brush| b.assist.taper_out = yolu_core::brush::MAX_STROKE_ASSIST + 1.0,
    ] {
        let mut s = assist_hard(4.0);
        bad(&mut s);
        assert!(s.validate().is_err());
    }
    let mut ok = assist_hard(4.0);
    ok.assist.stabilizer = 3.0;
    ok.assist.taper_in = 5.0;
    ok.assist.taper_out = 7.0;
    let (mut d, l) = doc(128, 64, 16);
    let mut st = d.begin_brush_stroke(l, &ok).unwrap();
    st.add_sample(&mut d, sample_at(5.0, 5.0, 1.0, 2.0))
        .unwrap();
    assert!(
        st.add_sample(&mut d, sample_at(9.0, 9.0, 1.0, 1.0))
            .is_err(),
        "手ぶれ補正でも時刻は増えなければならない"
    );
    assert!(!d.has_active_stroke());
}

// ───────── StrokeCurveTests ─────────

const CX: f64 = 64.3;
const CY: f64 = 63.7;
const R: f64 = 40.0;

fn soft(curve: bool) -> Brush {
    let mut b = Brush::from(BrushSettings {
        radius: 4.0,
        hardness: 0.0,
        spacing: 0.05,
        color: Rgba8::new(0, 0, 0, 255),
        pressure_size: false,
        pressure_opacity: false,
        ..BrushSettings::default()
    });
    b.assist.curve = curve;
    b
}

/// 中心 (CX, CY)・半径 R の円の 3/4 の弧を、1 周 n 等分の角度の点で描く。
fn arc(n: usize, b: &Brush) -> Vec<u8> {
    let (mut d, l) = doc(128, 128, 32);
    let samples: Vec<BrushSample> = (0..=n * 3 / 4)
        .map(|i| {
            let t = 2.0 * PI * i as f64 / n as f64;
            sample_at(CX + R * t.cos(), CY + R * t.sin(), 1.0, i as f64)
        })
        .collect();
    paint(&mut d, l, b, &samples);
    all(&d)
}

/// 線の中心の円からのずれ（C# の StrokeCurveTests.CenterLine）: 角度の区切りごとに塗った画素の中心からの距離を濃さで平均し、
/// 最初と最後の区間を除いた範囲での（円からの最大のずれ、最大と最小の差）。
fn center_line(rgba: &[u8], width: usize, height: usize, n: usize) -> (f64, f64) {
    let bins = 240;
    let (mut sum, mut weight) = (vec![0.0; bins], vec![0.0; bins]);
    for y in 0..height {
        for x in 0..width {
            let a = rgba[(y * width + x) * 4 + 3] as f64;
            if a == 0.0 {
                continue;
            }
            let (dx, dy) = (x as f64 + 0.5 - CX, y as f64 + 0.5 - CY);
            let mut angle = dy.atan2(dx);
            if angle < 0.0 {
                angle += 2.0 * PI;
            }
            let b = (angle / (2.0 * PI) * bins as f64) as usize % bins;
            sum[b] += (dx * dx + dy * dy).sqrt() * a;
            weight[b] += a;
        }
    }
    let step = 2.0 * PI / n as f64;
    let end = (n * 3 / 4) as f64 * step;
    let (mut deviation, mut low, mut high) = (0.0f64, f64::MAX, 0.0f64);
    for b in 0..bins {
        let angle = (b as f64 + 0.5) / bins as f64 * 2.0 * PI;
        if weight[b] == 0.0 || angle <= step || angle >= end - step {
            continue;
        }
        let r = sum[b] / weight[b];
        deviation = deviation.max((r - R).abs());
        low = low.min(r);
        high = high.max(r);
    }
    (deviation, high - low)
}

#[test]
fn the_curve_through_sparse_points_of_a_circle_stays_on_it() {
    for n in [8usize, 10, 12] {
        let p = |i: i32| {
            let t = 2.0 * PI * i as f64 / n as f64;
            (R * t.cos(), R * t.sin())
        };
        let sag = R * (1.0 - (PI / n as f64).cos());
        let ((x0, y0), (x1, y1), (x2, y2), (x3, y3)) = (p(-1), p(0), p(1), p(2));
        let mut worst = 0.0f64;
        for k in 0..=200 {
            let (x, y) = curve::point(x0, y0, x1, y1, x2, y2, x3, y3, k as f64 / 200.0);
            worst = worst.max(((x * x + y * y).sqrt() - R).abs());
        }
        assert!(
            worst < sag / 8.0,
            "n = {n}: 曲線は弦よりずっと円に近い（{worst}）"
        );
    }
    // 重なった前後の点でも数にならない値は出さない
    let (x, y) = curve::point(5.0, 5.0, 5.0, 5.0, 9.0, 5.0, 9.0, 5.0, 0.5);
    assert!(!x.is_nan() && !y.is_nan() && (5.0..=9.0).contains(&x));
}

#[test]
fn a_sparse_circle_is_drawn_round_on_the_canvas() {
    for n in [8usize, 10, 12] {
        let straight = center_line(&arc(n, &soft(false)), 128, 128, n);
        let curved = center_line(&arc(n, &soft(true)), 128, 128, n);
        // C# の実測（線の中心の最大のずれ）: n = 8 で 3.15 → 0.58 px、10 で 2.12 → 0.64、12 で 1.94 → 0.59
        assert!(
            curved.0 < 1.0,
            "n = {n}: 曲線の線は円に沿う（{}）",
            curved.0
        );
        assert!(
            curved.0 < straight.0 / 2.0,
            "n = {n}: 直線は角を削る（{}）",
            straight.0
        );
    }
}

#[test]
fn the_stabilizer_on_sparse_points_no_longer_makes_corners() {
    let mut s = soft(false);
    s.assist.stabilizer = 6.0;
    let straight = center_line(&arc(8, &s), 128, 128, 8);
    let mut s = soft(true);
    s.assist.stabilizer = 6.0;
    let curved = center_line(&arc(8, &s), 128, 128, 8);
    assert!(
        curved.1 < straight.1 * 0.6,
        "直線 {}・曲線 {}（C# の実測 3.07 → 1.43 px）",
        straight.1,
        curved.1
    );
}

#[test]
fn the_segment_to_the_newest_point_waits_and_commit_draws_it() {
    let (mut d, l) = doc(128, 64, 16);
    let mut s = soft(true);
    s.base.hardness = 1.0;
    s.base.radius = 3.0;
    let mut st = d.begin_brush_stroke(l, &s).unwrap();
    st.add_sample(&mut d, sample(10.0, 32.0)).unwrap();
    st.add_sample(&mut d, sample_at(60.0, 40.0, 1.0, 1.0))
        .unwrap();
    st.add_sample(&mut d, sample_at(110.0, 32.0, 1.0, 2.0))
        .unwrap();
    let during = all(&d);
    assert!(a128(&during, 30, 36) > 0, "次の点が分かった区間は描く");
    assert!(
        (0..64).all(|y| a128(&during, 100, y) == 0),
        "最新の点への区間は待つ"
    );
    d.end_stroke(st).unwrap();
    let c = all(&d);
    assert!(
        (0..64).any(|y| a128(&c, 100, y) > 0),
        "確定で最後の区間を描く"
    );
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert!(
        all(&d).iter().all(|&b| b == 0),
        "1 回の Undo で曲線全体が消える"
    );
    d.redo().unwrap();
    assert_eq!(all(&d), c);
}

#[test]
fn cancelling_with_a_held_segment_leaves_nothing() {
    let (mut d, l) = doc(128, 64, 16);
    let mut s = soft(true);
    s.assist.taper_out = 20.0;
    s.assist.stabilizer = 3.0;
    let mut st = d.begin_brush_stroke(l, &s).unwrap();
    st.add_sample(&mut d, sample(10.0, 32.0)).unwrap();
    st.add_sample(&mut d, sample_at(60.0, 40.0, 1.0, 1.0))
        .unwrap();
    st.add_sample(&mut d, sample_at(110.0, 20.0, 1.0, 2.0))
        .unwrap();
    d.cancel_stroke(st);
    assert!(all(&d).iter().all(|&b| b == 0));
    assert_eq!(d.undo_count(), 0);
    assert!(!d.has_active_stroke());
}

#[test]
fn on_a_line_the_curve_draws_the_same_pixels_as_straight_segments() {
    for dual in [false, true] {
        let draw = |curve: bool| {
            let (mut d, l) = doc(128, 64, 16);
            let mut b = Brush::from(BrushSettings {
                radius: 5.0,
                hardness: 0.6,
                spacing: 0.1,
                color: Rgba8::new(0, 0, 0, 255),
                ..BrushSettings::default()
            });
            b.assist.taper_in = 20.0;
            b.assist.taper_out = 25.0;
            b.assist.curve = curve;
            // デュアルの間隔（1.842 px）は主の間隔（1 px）とこの長さの中で重ならない（C# の試験の注と同じ）
            if dual {
                b.dual = Some(DualBrush {
                    radius: 3.07,
                    spacing: 0.3,
                    scatter: 0.5,
                    count: 2,
                    ..DualBrush::default()
                });
            }
            let mut st = d.begin_brush_stroke(l, &b).unwrap();
            for i in 0..=5 {
                st.add_sample(
                    &mut d,
                    sample_at(
                        10.0 + 20.0 * i as f64,
                        32.0,
                        0.3 + 0.14 * i as f64,
                        i as f64,
                    ),
                )
                .unwrap();
            }
            let r = d.end_stroke(st).unwrap();
            (all(&d), r.stamps)
        };
        let (straight, a) = draw(false);
        let (curved, b) = draw(true);
        assert_eq!(b, a, "曲線に沿っても間隔は同じに溜まる");
        assert!(
            curved == straight,
            "筆圧・入り抜き{}は同じ線の長さに従う",
            if dual { "・デュアル" } else { "" }
        );
    }
}

#[test]
fn tapers_shape_a_curved_stroke_and_the_end_waits_for_commit() {
    let (mut d, l) = doc(128, 128, 32);
    let mut s = soft(true);
    s.base.hardness = 1.0;
    s.base.radius = 5.0;
    s.assist.taper_in = 25.0;
    s.assist.taper_out = 25.0;
    let n = 8;
    let at = |i: usize| {
        let t = 2.0 * PI * i as f64 / n as f64;
        (CX + R * t.cos(), CY + R * t.sin())
    };
    let mut st = d.begin_brush_stroke(l, &s).unwrap();
    for i in 0..=6 {
        let (x, y) = at(i);
        st.add_sample(&mut d, sample_at(x, y, 1.0, i as f64))
            .unwrap();
    }
    let during = all(&d);
    let ex = (CX + R * (2.0 * PI * 6.0 / n as f64).cos()) as usize;
    assert_eq!(alpha_at(&during, 128, ex, CY as usize), 0);
    d.end_stroke(st).unwrap();
    let c = all(&d);
    let across = |angle: f64| {
        let mut count = 0;
        let mut r = R - 10.0;
        while r <= R + 10.0 {
            let x = (CX + r * angle.cos()) as usize;
            let y = (CY + r * angle.sin()) as usize;
            if alpha_at(&c, 128, x, y) > 0 {
                count += 1;
            }
            r += 0.25;
        }
        count
    };
    let end = 2.0 * PI * 6.0 / n as f64;
    assert!(across(0.15) < across(end / 2.0), "入り");
    assert!(across(end - 0.15) < across(end / 2.0), "抜き");
    assert!(across(end - 0.3) > 0, "確定で終わりを描く");
    assert_eq!(d.undo_count(), 1);
}

#[test]
fn an_oversized_curved_segment_is_refused_without_partial_edits() {
    for at_commit in [false, true] {
        let (mut d, l) = doc(64, 64, 16);
        let mut b = Brush::from(BrushSettings {
            radius: 0.05,
            spacing: 0.1,
            color: Rgba8::new(0, 0, 0, 255),
            ..BrushSettings::default()
        });
        b.assist.curve = true;
        let mut st = d.begin_brush_stroke(l, &b).unwrap();
        st.add_sample(&mut d, sample(20.0, 20.5)).unwrap();
        st.add_sample(&mut d, sample_at(30.0, 20.5, 1.0, 1.0))
            .unwrap();
        st.add_sample(&mut d, sample_at(5_000_000.0, 20.5, 1.0, 2.0))
            .unwrap(); // まだ待っている区間
        if at_commit {
            assert!(d.end_stroke(st).is_err());
        } else {
            assert!(st
                .add_sample(&mut d, sample_at(5_000_000.0, 40.0, 1.0, 3.0))
                .is_err());
        }
        assert!(!d.has_active_stroke());
        assert!(
            all(&d).iter().all(|&b| b == 0),
            "断ったストロークは画素を残さない"
        );
        assert_eq!(d.undo_count(), 0);
    }
}

// ───────── BrushEffectTests ─────────

const W: usize = 19;
const H: usize = 13;

/// 19×13 のレイヤーに決まった模様（アルファ 0・120・255 が混ざる）を置いた文書（C# の BrushEffectTests.Make の Color）。
fn effect_doc(tile: u32, w: usize, h: usize) -> (Document, LayerId) {
    let (mut d, l) = doc(w as u32, h as u32, tile);
    for y in 0..h {
        for x in 0..w {
            let a = if (x + y) % 5 == 0 {
                0
            } else if (x + y) % 3 == 0 {
                120
            } else {
                255
            };
            let c = Rgba8::new(
                ((x * 31) % 256) as u8,
                ((y * 47) % 256) as u8,
                ((x * 19 + y * 7) % 256) as u8,
                a,
            );
            d.set_pixel(l, x as u32, y as u32, c).unwrap();
        }
    }
    d.clear_history().unwrap();
    (d, l)
}
fn read(d: &Document, l: LayerId) -> Vec<Rgba8> {
    let layer = d.layer(l).unwrap();
    (0..d.width() * d.height())
        .map(|i| {
            layer
                .pixel(Channel::Color, i % d.width(), i / d.width())
                .unwrap()
        })
        .collect()
}
fn effect_brush(effect: BrushEffect) -> Brush {
    Brush {
        effect,
        ..Brush::from(BrushSettings {
            radius: 3.0,
            hardness: 0.6,
            spacing: 0.2,
            opacity: 0.7,
            flow: 0.6,
            pressure_size: false,
            pressure_opacity: true,
            pressure_flow: true,
            ..BrushSettings::default()
        })
    }
}
const BLUR: BrushEffect = BrushEffect::Blur { radius: 2 };
const SMUDGE: BrushEffect = BrushEffect::Smudge { strength: 0.6 };
const CLONE: BrushEffect = BrushEffect::Clone {
    offset: DVec2::new(-2.25, 0.5),
};
fn b255(v: f64) -> u8 {
    (v + 0.5).floor().clamp(0.0, 255.0) as u8
}
// 独立した参照: 全画面の配列、ウィンドウの中を直接足すぼかし、4 点の補間。core のフィルター・合成を呼ばない。
fn weighted(p: &[Rgba8], weights: &[f64]) -> Rgba8 {
    let (mut a, mut r, mut g, mut b) = (0.0, 0.0, 0.0, 0.0);
    for (c, w) in p.iter().zip(weights) {
        let v = c.a as f64 * w;
        a += v;
        r += c.r as f64 * v;
        g += c.g as f64 * v;
        b += c.b as f64 * v;
    }
    if a == 0.0 {
        Rgba8::TRANSPARENT
    } else {
        Rgba8::new(b255(r / a), b255(g / a), b255(b / a), b255(a))
    }
}
fn ref_blur(image: &[Rgba8], x: usize, y: usize, radius: usize) -> Rgba8 {
    let mut p = Vec::new();
    for yy in y.saturating_sub(radius)..=(y + radius).min(H - 1) {
        for xx in x.saturating_sub(radius)..=(x + radius).min(W - 1) {
            p.push(image[yy * W + xx]);
        }
    }
    let w = vec![1.0 / p.len() as f64; p.len()];
    weighted(&p, &w)
}
fn ref_bilinear(image: &[Rgba8], x: f64, y: f64) -> Rgba8 {
    let (ix, iy) = (x.floor() as usize, y.floor() as usize);
    let (fx, fy) = (x - ix as f64, y - iy as f64);
    let at = |xx: usize, yy: usize| image[yy.min(H - 1) * W + xx.min(W - 1)];
    if fx == 0.0 && fy == 0.0 {
        return at(ix, iy);
    }
    weighted(
        &[
            at(ix, iy),
            at(ix + 1, iy),
            at(ix, iy + 1),
            at(ix + 1, iy + 1),
        ],
        &[
            (1.0 - fx) * (1.0 - fy),
            fx * (1.0 - fy),
            (1.0 - fx) * fy,
            fx * fy,
        ],
    )
}
fn ref_mix(start: Rgba8, source: Rgba8, amount: f64, clone: bool) -> Rgba8 {
    let sa = source.a as f64 / 255.0 * amount;
    let ba = start.a as f64 / 255.0;
    let u = if clone {
        ba * (1.0 - sa)
    } else {
        ba * (1.0 - amount)
    };
    let a = u + sa;
    if a <= 0.0 || b255(a * 255.0) == 0 {
        return Rgba8::new(start.r, start.g, start.b, 0);
    }
    Rgba8::new(
        b255((start.r as f64 * u + source.r as f64 * sa) / a),
        b255((start.g as f64 * u + source.g as f64 * sa) / a),
        b255((start.b as f64 * u + source.b as f64 * sa) / a),
        b255(a * 255.0),
    )
}
#[allow(clippy::too_many_arguments)]
fn reference_dab(
    original: &[Rgba8],
    live: &mut Vec<Rgba8>,
    wash: &mut [f32],
    effect: BrushEffect,
    pixels: &[BrushPixel],
    (dx, dy): (f64, f64),
    pressure: f64,
) {
    let (opacity, flow_setting) = (0.7, 0.6);
    let frame = live.clone();
    let mut next = live.clone();
    for p in pixels {
        let i = p.y as usize * W + p.x as usize;
        let ceiling = opacity * pressure;
        let strength = if let BrushEffect::Smudge { strength } = effect {
            strength
        } else {
            1.0
        };
        let flow = p.coverage * flow_setting * pressure * strength;
        let w = wash[i] as f64;
        let a = if w >= ceiling {
            w
        } else {
            w + (ceiling - w) * flow.min(1.0)
        };
        wash[i] = a as f32;
        let source = match effect {
            BrushEffect::Blur { radius } => {
                if original[i].a == 0 {
                    continue;
                }
                ref_blur(&frame, p.x as usize, p.y as usize, radius as usize)
            }
            _ => {
                let (x, y) = (p.x as f64 + dx, p.y as f64 + dy);
                if x < 0.0 || y < 0.0 || x > (W - 1) as f64 || y > (H - 1) as f64 {
                    continue;
                }
                ref_bilinear(
                    if matches!(effect, BrushEffect::Clone { .. }) {
                        original
                    } else {
                        &frame
                    },
                    x,
                    y,
                )
            }
        };
        next[i] = ref_mix(
            original[i],
            source,
            a,
            matches!(effect, BrushEffect::Clone { .. }),
        );
    }
    *live = next;
}
fn effect_pixels(step: usize) -> Vec<BrushPixel> {
    (0..W * H)
        .map(|i| BrushPixel {
            x: (i % W) as i64,
            y: (i / W) as i64,
            coverage: 0.2 + ((i + step) % 7) as f64 / 10.0,
        })
        .collect()
}

#[test]
fn effect_dabs_equal_an_independent_premultiplied_reference() {
    for effect in [BLUR, SMUDGE, CLONE] {
        let (mut d, l) = effect_doc(16, W, H);
        let original = read(&d, l);
        let mut expected = original.clone();
        let mut wash = vec![0.0f32; W * H];
        let mut st = d.begin_brush_stroke(l, &effect_brush(effect)).unwrap();
        if effect == SMUDGE {
            assert!(
                !st.apply_dab(&mut d, &effect_pixels(0), DVec2::new(5.0, 5.0), 1.0)
                    .unwrap(),
                "最初は位置を覚えるだけ"
            );
        }
        for n in 0..3 {
            let pixels = effect_pixels(n);
            let offset = match effect {
                BrushEffect::Smudge { .. } => (-1.25, -0.5),
                BrushEffect::Clone { offset } => (offset.x, offset.y),
                _ => (0.0, 0.0),
            };
            reference_dab(
                &original,
                &mut expected,
                &mut wash,
                effect,
                &pixels,
                offset,
                0.8,
            );
            st.apply_dab(
                &mut d,
                &pixels,
                DVec2::new(6.25 + n as f64 * 1.25, 5.5 + n as f64 * 0.5),
                0.8,
            )
            .unwrap();
            assert_eq!(read(&d, l), expected, "{effect:?} ダブ {n}");
        }
        assert!(d.end_stroke(st).unwrap().changed);
        assert_eq!(d.undo_count(), 1);
        d.undo().unwrap();
        assert_eq!(read(&d, l), original);
        d.redo().unwrap();
        assert_eq!(read(&d, l), expected);
    }
}

/// 試験の間だけワーカーの閾値を変える（どの値でも画素は同じ。経路の選び方だけが変わる）。
struct Threshold(i64);
impl Threshold {
    fn set(pixels: i64) -> Threshold {
        Threshold(set_parallel_dab_pixels(pixels))
    }
}
impl Drop for Threshold {
    fn drop(&mut self) {
        set_parallel_dab_pixels(self.0);
    }
}

#[test]
fn sampling_and_dabs_are_identical_across_tile_sizes_and_thread_counts() {
    let _t = Threshold::set(1);
    for effect in [BLUR, SMUDGE, CLONE, BrushEffect::Paint] {
        let mut expected: Option<Vec<Rgba8>> = None;
        for tile in [4u32, 16, 32] {
            for threads in [1usize, 4] {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .unwrap();
                let pixels = pool.install(|| {
                    let (mut d, l) = effect_doc(tile, 75, 57);
                    let mut b = effect_brush(effect);
                    b.base.radius = 22.0;
                    b.base.spacing = 0.08;
                    if effect == BrushEffect::Paint {
                        // 色を塗るブラシは、ダイナミクスを全部入れて同じことを確かめる
                        b = every_dynamic();
                        b.base.radius = 22.0;
                        b.texture = Some(PaperTexture::new(builtin_tip("grain").unwrap(), 0.6));
                        b.tip.image = Some(builtin_tip("noisy-disc").unwrap());
                        b.jitter.angle = 0.5;
                    }
                    paint(
                        &mut d,
                        l,
                        &b,
                        &[
                            sample_at(25.5, 28.5, 1.0, 0.0),
                            sample_at(44.5, 31.5, 1.0, 1.0),
                        ],
                    );
                    read(&d, l)
                });
                match &expected {
                    None => expected = Some(pixels),
                    Some(e) => assert!(*e == pixels, "{effect:?} タイル {tile} スレッド {threads}"),
                }
            }
        }
    }
}

#[test]
fn effect_budgets_and_invalid_inputs_cancel_without_an_undo() {
    for effect in [BLUR, SMUDGE, CLONE] {
        let (mut d, l) = effect_doc(16, W, H);
        let saved = read(&d, l);
        d.set_stroke_budget_bytes(10).unwrap();
        let mut st = d.begin_brush_stroke(l, &effect_brush(effect)).unwrap();
        let r = st
            .add_sample(&mut d, sample_at(5.0, 5.0, 1.0, 0.0))
            .and_then(|_| st.add_sample(&mut d, sample_at(12.0, 7.0, 1.0, 1.0)));
        assert_eq!(r, Err(CoreError::StrokeBudgetExceeded));
        assert!(!d.has_active_stroke());
        assert_eq!(read(&d, l), saved);
        assert_eq!(d.undo_count(), 0);
        d.set_stroke_budget_bytes(4 << 20).unwrap();
        let mut st = d.begin_brush_stroke(l, &effect_brush(effect)).unwrap();
        st.add_sample(&mut d, sample_at(5.0, 5.0, 1.0, 0.0))
            .unwrap();
        st.add_sample(&mut d, sample_at(12.0, 7.0, 1.0, 1.0))
            .unwrap();
        let bad = [BrushPixel {
            x: 5,
            y: 5,
            coverage: 2.0,
        }];
        assert!(st
            .apply_dab(&mut d, &bad, DVec2::new(5.0, 5.0), 1.0)
            .is_err());
        assert!(!d.has_active_stroke());
        assert_eq!(read(&d, l), saved);
        assert_eq!(d.undo_count(), 0);
    }
}

#[test]
fn invalid_effect_parameters_are_refused_before_any_mutation() {
    let (mut d, l) = effect_doc(16, W, H);
    let saved = read(&d, l);
    let erase_blur = Brush {
        effect: BrushEffect::BLUR,
        ..Brush::from(BrushSettings {
            erase: true,
            ..BrushSettings::default()
        })
    };
    for b in [
        effect_brush(BrushEffect::Blur { radius: 0 }),
        effect_brush(BrushEffect::Blur { radius: 65 }),
        effect_brush(BrushEffect::Smudge { strength: f64::NAN }),
        effect_brush(BrushEffect::Smudge { strength: 1.5 }),
        effect_brush(BrushEffect::Clone {
            offset: DVec2::new(f64::INFINITY, 0.0),
        }),
        effect_brush(BrushEffect::Clone {
            offset: DVec2::new(0.0, 2e7),
        }),
        erase_blur,
    ] {
        assert!(matches!(
            d.begin_brush_stroke(l, &b),
            Err(CoreError::InvalidArgument(_))
        ));
    }
    assert_eq!(read(&d, l), saved);
}

#[test]
fn effect_brushes_refuse_single_pixels() {
    let (mut d, l) = effect_doc(16, W, H);
    let saved = read(&d, l);
    let mut st = d.begin_brush_stroke(l, &effect_brush(BLUR)).unwrap();
    assert!(matches!(
        st.apply_pixel(&mut d, 3, 3, 1.0, 1.0),
        Err(CoreError::Unsupported(_))
    ));
    assert!(!d.has_active_stroke());
    assert_eq!(read(&d, l), saved);
}

#[test]
fn clone_reads_the_stroke_start_when_source_and_destination_overlap() {
    let (mut d, l) = doc(10, 1, 4);
    for x in 0..10 {
        d.set_pixel(l, x, 0, Rgba8::new((x * 20) as u8, 0, 0, 255))
            .unwrap();
    }
    let original = read(&d, l);
    d.clear_history().unwrap();
    let b = Brush {
        effect: BrushEffect::Clone {
            offset: DVec2::new(-1.0, 0.0),
        },
        ..Brush::from(BrushSettings {
            radius: 0.5,
            hardness: 1.0,
            spacing: 1.0,
            pressure_size: false,
            pressure_opacity: false,
            ..BrushSettings::default()
        })
    };
    paint(
        &mut d,
        l,
        &b,
        &[sample_at(1.5, 0.5, 1.0, 0.0), sample_at(8.5, 0.5, 1.0, 1.0)],
    );
    let now = read(&d, l);
    for x in 1..9 {
        assert_eq!(now[x], original[x - 1], "書いた画素から写し直さない");
    }
}

#[test]
fn blur_leaves_zero_alpha_rgb_and_ignores_its_colour_in_the_average() {
    let (mut d, l) = doc(3, 1, 2);
    d.set_pixel(l, 0, 0, Rgba8::new(255, 17, 99, 0)).unwrap();
    d.set_pixel(l, 1, 0, Rgba8::new(10, 20, 30, 255)).unwrap();
    d.set_pixel(l, 2, 0, Rgba8::new(10, 20, 30, 255)).unwrap();
    d.clear_history().unwrap();
    let b = Brush {
        effect: BrushEffect::Blur { radius: 1 },
        ..Brush::default()
    };
    let mut st = d.begin_brush_stroke(l, &b).unwrap();
    st.apply_dab(
        &mut d,
        &[
            BrushPixel {
                x: 0,
                y: 0,
                coverage: 1.0,
            },
            BrushPixel {
                x: 1,
                y: 0,
                coverage: 1.0,
            },
        ],
        DVec2::new(1.0, 0.0),
        1.0,
    )
    .unwrap();
    d.end_stroke(st).unwrap();
    let p = read(&d, l);
    assert_eq!(p[0], Rgba8::new(255, 17, 99, 0));
    assert_eq!(p[1], Rgba8::new(10, 20, 30, 170));
}

#[test]
fn smudge_reset_forgets_the_previous_position() {
    let (mut d, l) = effect_doc(16, W, H);
    let mut st = d.begin_brush_stroke(l, &effect_brush(SMUDGE)).unwrap();
    let px = effect_pixels(1);
    assert!(!st
        .apply_dab(&mut d, &px, DVec2::new(5.0, 5.0), 1.0)
        .unwrap());
    assert!(st
        .apply_dab(&mut d, &px, DVec2::new(7.0, 6.0), 1.0)
        .unwrap());
    st.reset_effect_direction(&mut d).unwrap();
    assert!(
        !st.apply_dab(&mut d, &px, DVec2::new(12.0, 3.0), 1.0)
            .unwrap(),
        "継ぎ目をまたいだ後の最初のダブは位置を覚えるだけ"
    );
    d.end_stroke(st).unwrap();
}

/// C# の MaskPixelsUseTheSameEffectAndKeepOneUndo: ぼかし・指先・クローンをマスクへ描くと、色ではなく同じ画素演算が効き
/// （同じ画素を Color へ描いた結果と同じバイト）、Undo は 1 回でマスクだけを戻し、Redo で同じ結果に戻る。
#[test]
fn mask_pixels_use_the_same_effect_and_keep_one_undo() {
    for effect in [BLUR, SMUDGE, CLONE] {
        let (mut d, l) = effect_doc(16, W, H);
        d.add_layer_mask(l).unwrap();
        for y in 0..H {
            for x in 0..W {
                let hide = ((x * 29 + y * 17) % 256) as u8;
                d.set_mask_pixel(l, x as u32, y as u32, hide).unwrap();
                d.set_pixel(l, x as u32, y as u32, Rgba8::new(0, 0, 0, hide))
                    .unwrap();
            }
        }
        d.clear_history().unwrap();
        let mask_bytes = |d: &Document| {
            d.layer(l)
                .unwrap()
                .mask()
                .unwrap()
                .surface()
                .to_canvas_bytes()
        };
        let color_bytes = |d: &Document| {
            d.layer(l)
                .unwrap()
                .surface(Channel::Color)
                .unwrap()
                .to_canvas_bytes()
        };
        let (mask_before, color_before) = (mask_bytes(&d), color_bytes(&d));
        assert_eq!(mask_before, color_before);
        let b = effect_brush(effect);
        let samples = [
            sample_at(5.0, 5.0, 1.0, 0.0),
            sample_at(12.0, 7.0, 1.0, 1.0),
        ];
        let mut st = d.begin_brush_mask_stroke(l, &b).unwrap();
        for p in samples {
            st.add_sample(&mut d, p).unwrap();
        }
        assert!(d.end_stroke(st).unwrap().changed, "{effect:?}");
        let expected = mask_bytes(&d);
        assert_ne!(expected, mask_before, "{effect:?} はマスクを変える");
        assert_eq!(color_bytes(&d), color_before, "マスクの描画は色に触れない");
        assert_eq!(d.undo_count(), 1, "{effect:?}");
        assert!(d.undo().unwrap());
        assert_eq!(mask_bytes(&d), mask_before, "{effect:?}");
        assert_eq!(d.undo_count(), 0);
        assert!(d.redo().unwrap());
        assert_eq!(mask_bytes(&d), expected, "{effect:?}");
        // 同じ画素を Color へ描いた結果と同じ
        paint(&mut d, l, &b, &samples);
        assert_eq!(
            color_bytes(&d),
            expected,
            "{effect:?} マスクにも同じ画素演算が効く"
        );
    }
}

/// マスクへの効果のストロークも、予算の拒否・取消・不正な入力で、元のマスクのバイトと履歴へ戻す。
#[test]
fn effect_strokes_on_a_mask_cancel_and_refuse_back_to_the_exact_mask() {
    for effect in [BLUR, SMUDGE, CLONE] {
        let (mut d, l) = effect_doc(16, W, H);
        d.add_layer_mask(l).unwrap();
        for y in 0..H {
            for x in 0..W {
                d.set_mask_pixel(l, x as u32, y as u32, ((x * 29 + y * 17) % 256) as u8)
                    .unwrap();
            }
        }
        d.clear_history().unwrap();
        let mask = |d: &Document| {
            d.layer(l)
                .unwrap()
                .mask()
                .unwrap()
                .surface()
                .to_canvas_bytes()
        };
        let saved = mask(&d);
        let run = |d: &mut Document| {
            let mut st = d.begin_brush_mask_stroke(l, &effect_brush(effect)).unwrap();
            let r = st
                .add_sample(d, sample_at(5.0, 5.0, 1.0, 0.0))
                .and_then(|_| st.add_sample(d, sample_at(12.0, 7.0, 1.0, 1.0)));
            (st, r)
        };
        // Escape（取消）
        let (st, r) = run(&mut d);
        r.unwrap();
        assert_ne!(mask(&d), saved);
        d.cancel_stroke(st);
        assert_eq!((mask(&d), d.undo_count()), (saved.clone(), 0), "{effect:?}");
        // ストロークの予算
        d.set_stroke_budget_bytes(10).unwrap();
        let (_, r) = run(&mut d);
        assert_eq!(r, Err(CoreError::StrokeBudgetExceeded), "{effect:?}");
        assert!(!d.has_active_stroke());
        assert_eq!((mask(&d), d.undo_count()), (saved.clone(), 0), "{effect:?}");
        d.set_stroke_budget_bytes(4 << 20).unwrap();
        // 不正な入力
        let (mut st, r) = run(&mut d);
        r.unwrap();
        let bad = [BrushPixel {
            x: 5,
            y: 5,
            coverage: 2.0,
        }];
        assert!(st
            .apply_dab(&mut d, &bad, DVec2::new(5.0, 5.0), 1.0)
            .is_err());
        assert!(!d.has_active_stroke());
        assert_eq!((mask(&d), d.undo_count()), (saved.clone(), 0), "{effect:?}");
    }
}

// ───────── C# に無い拡張 ─────────

#[test]
fn flipping_mirrors_the_tip_image() {
    let tip = upper_half_tip();
    let draw_with = |flip_x: bool, flip_y: bool, angle: f64| {
        let (mut d, l) = canvas(64);
        let mut b = fixed();
        b.tip.image = Some(tip.clone());
        b.tip.flip_x = flip_x;
        b.tip.flip_y = flip_y;
        b.tip.angle = angle;
        paint(&mut d, l, &b, &[sample(32.0, 32.0)]);
        d
    };
    let d = draw_with(false, true, 0.0);
    assert_eq!(a(&d, 32, 28), 255, "上下の反転で埋まった半分が下に");
    assert_eq!(a(&d, 32, 36), 0);
    let d = draw_with(true, false, 0.0);
    assert_eq!(
        (a(&d, 32, 36), a(&d, 32, 28)),
        (255, 0),
        "左右の反転は上半分の筆先を変えない"
    );
    // 反転は回す前の筆先にかかる: 上下に反転して 90° 回すと右を向く
    let d = draw_with(false, true, 90.0);
    assert_eq!((a(&d, 36, 32), a(&d, 28, 32)), (255, 0));
    // 丸い筆先には効かない
    let (mut r1, l1) = canvas(64);
    let (mut r2, l2) = canvas(64);
    let mut b = fixed();
    b.tip.roundness = 0.5;
    b.tip.angle = 20.0;
    paint(&mut r1, l1, &b, &[sample(32.0, 32.0)]);
    b.tip.flip_x = true;
    b.tip.flip_y = true;
    paint(&mut r2, l2, &b, &[sample(32.0, 32.0)]);
    assert_eq!(all(&r1), all(&r2));
}

#[test]
fn texture_modes_follow_the_dual_brush_formulas_on_the_ceiling() {
    // 縞: x mod 4 が 0・1 は白（1）、2・3 は黒（0）。不透明度 0.6・深さ 1 の硬い線を引き、白い縞（x=32）と黒い縞（x=34）を見る
    let draw_mode = |mode: TextureMode, depth: f64| {
        let (mut d, l) = canvas(64);
        let mut b = fixed();
        b.base.opacity = 0.6;
        b.texture = Some(PaperTexture {
            mode,
            ..PaperTexture::new(stripes(), depth)
        });
        stroke(&mut d, l, &b, &[(16.0, 32.0), (48.0, 32.0)]);
        (a(&d, 32, 32), a(&d, 34, 32), all(&d))
    };
    let c = |v: f64| (v * 255.0 + 0.5).floor() as u8;
    assert_eq!(draw_mode(TextureMode::Multiply, 1.0).0, c(0.6));
    assert_eq!(draw_mode(TextureMode::Multiply, 1.0).1, 0);
    // 減算: c − g。白は 0、黒は c
    let s = draw_mode(TextureMode::Subtract, 1.0);
    assert_eq!((s.0, s.1), (0, c(0.6)), "減算は黒い所ほど塗れる");
    // 比較（暗）: min(c, g)
    let s = draw_mode(TextureMode::Darken, 1.0);
    assert_eq!((s.0, s.1), (c(0.6), 0));
    // ハードミックス: c + g ≥ 1 なら 1
    let s = draw_mode(TextureMode::HardMix, 1.0);
    assert_eq!((s.0, s.1), (255, 0));
    // 覆い焼き: c / (1 − g)。白なら 1、黒なら c
    let s = draw_mode(TextureMode::ColorDodge, 1.0);
    assert_eq!((s.0, s.1), (255, c(0.6)));
    // 深さ 0.5 は元の天井との中ほど（減算の白: 0.6 + (0 − 0.6) × 0.5 = 0.3）
    assert_eq!(draw_mode(TextureMode::Subtract, 0.5).0, c(0.3));
    // 深さ 0 はどのモードでも質感なしと同じ
    let (mut d, l) = canvas(64);
    let mut b = fixed();
    b.base.opacity = 0.6;
    stroke(&mut d, l, &b, &[(16.0, 32.0), (48.0, 32.0)]);
    let plain = all(&d);
    for m in TextureMode::ALL {
        assert_eq!(draw_mode(m, 0.0).2, plain, "{m:?}");
    }
}

#[test]
fn the_tip_list_cycles_in_order() {
    // 順に: 上半分 → 下半分（反転した画像）→ 上半分 …。間隔を半径の 4 倍にして重ならないダブで確かめる
    let up = upper_half_tip();
    let mut down_alpha = up.alpha().to_vec();
    down_alpha.reverse();
    let down = Arc::new(BrushTip::new("lower half", 16, 16, down_alpha).unwrap());
    let (mut d, l) = doc(96, 32, 32);
    let mut b = fixed();
    b.base.radius = 4.0;
    b.base.spacing = 1.5; // 12 px ごと
    b.tip.images = vec![up.clone(), down.clone()];
    b.tip.selection = TipSelection::Sequential;
    stroke(&mut d, l, &b, &[(10.0, 16.0), (82.0, 16.0)]);
    for (k, x) in (10..=82).step_by(12).enumerate() {
        let upper = a(&d, x, 18) > 0;
        let lower = a(&d, x, 13) > 0;
        assert_eq!(
            (upper, lower),
            (k % 2 == 0, k % 2 == 1),
            "{k} 番目のダブ（x = {x}）"
        );
    }
}

#[test]
fn pen_rotation_turns_the_tip_only_when_asked() {
    let rotated = |on: bool| {
        let mut b = hard(8.0);
        b.tip.roundness = 0.25;
        b.controls.rotation_angle = on;
        let s = sample(30.0, 30.0).with_rotation(PI / 2.0).unwrap();
        draw(&b, &[s])
    };
    let mut vertical = hard(8.0);
    vertical.tip.roundness = 0.25;
    vertical.tip.angle = 90.0;
    assert_eq!(
        rotated(true),
        draw(&vertical, &[sample(30.0, 30.0)]),
        "軸を 90° 回すと筆先も 90° 回る"
    );
    let mut flat = hard(8.0);
    flat.tip.roundness = 0.25;
    assert_eq!(
        rotated(false),
        draw(&flat, &[sample(30.0, 30.0)]),
        "切っていれば回転は使わない"
    );
    assert!(sample(1.0, 1.0).with_rotation(f64::NAN).is_err());
}

#[test]
fn pen_rotation_is_interpolated_the_short_way_round() {
    // 上半分の筆先を、回転 +170° から −170° へ動かす: 間のダブは 180°（下向き）を通る（0° を通る長い回り道ではない）
    let tip = upper_half_tip();
    let mut b = hard(8.0);
    b.tip.image = Some(tip);
    b.base.spacing = 4.0; // 64 px ごと: x = 10 と 74 の 2 つのダブ
    b.controls.rotation_angle = true;
    let deg = |d: f64| d.to_radians();
    let (mut d, l) = doc(96, 64, 32);
    let mut st = d.begin_brush_stroke(l, &b).unwrap();
    st.add_sample(
        &mut d,
        sample(10.0, 32.0).with_rotation(deg(170.0)).unwrap(),
    )
    .unwrap();
    st.add_sample(
        &mut d,
        sample(74.0, 32.0).with_rotation(deg(-170.0)).unwrap(),
    )
    .unwrap();
    d.end_stroke(st).unwrap();
    let c = all(&d);
    // 2 つ目のダブ（x = 74）は −170°: 埋まった半分はほぼ下
    assert!(a96(&c, 74, 28) > 0 && a96(&c, 74, 36) == 0);
    // 補間の確かめ: 間隔を詰めて真ん中（x = 42）のダブを見る
    let mut b2 = b.clone();
    b2.base.spacing = 1.0; // 16 px ごと: 10, 26, 42, 58, 74
    let (mut d2, l2) = doc(96, 64, 32);
    let mut st = d2.begin_brush_stroke(l2, &b2).unwrap();
    st.add_sample(
        &mut d2,
        sample(10.0, 32.0).with_rotation(deg(170.0)).unwrap(),
    )
    .unwrap();
    st.add_sample(
        &mut d2,
        sample(74.0, 32.0).with_rotation(deg(-170.0)).unwrap(),
    )
    .unwrap();
    d2.end_stroke(st).unwrap();
    let c2 = all(&d2);
    assert!(a96(&c2, 42, 28) > 0, "真ん中は 180°（下向き）");
    assert_eq!(a96(&c2, 42, 37), 0, "0°（上向き）を通る回り道ではない");
}

#[test]
fn speed_thins_fast_segments_and_needs_time() {
    let mut b = hard(6.0);
    b.base.spacing = 0.1;
    b.controls.speed_size = true;
    b.controls.speed_max = 1000.0;
    let column = |c: &[u8], x: usize| (0..64).filter(|&y| a96(c, x, y) > 0).count();
    // 左の区間は 40 px を 0.4 秒（100 px/s）、右の区間は 40 px を 0.05 秒（800 px/s）
    let timed = [
        sample_at(8.0, 32.0, 1.0, 0.0),
        sample_at(48.0, 32.0, 1.0, 0.4),
        sample_at(88.0, 32.0, 1.0, 0.45),
    ];
    let c = draw(&b, &timed);
    assert!(
        column(&c, 70) < column(&c, 40),
        "速い区間ほど細い（{} / {}）",
        column(&c, 70),
        column(&c, 40)
    );
    // 時刻が進まない入力では速さは 0 のまま（何も変えない）
    let untimed = [sample(8.0, 32.0), sample(48.0, 32.0), sample(88.0, 32.0)];
    let mut plain = b.clone();
    plain.controls.speed_size = false;
    assert_eq!(draw(&b, &untimed), draw(&plain, &untimed));
    // 切っていれば時刻があっても同じ
    assert_eq!(draw(&plain, &timed), draw(&plain, &untimed));
    // 速さの上限は 0 より大きい
    let mut bad = b.clone();
    bad.controls.speed_max = 0.0;
    assert!(bad.validate().is_err());
    // 不透明度と流量も（速い区間ほど薄い）
    let mut op = hard(6.0);
    op.base.spacing = 0.1;
    op.controls.speed_opacity = true;
    op.controls.speed_max = 1000.0;
    let c = draw(&op, &timed);
    assert!(a96(&c, 70, 32) < a96(&c, 30, 32));
}
