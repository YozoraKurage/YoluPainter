//! 色の混ぜ（厚塗りのブラシ）: 混ぜ方が切のブラシは今までと同じ（設定の値は効かない）、混ぜるは下の色を拾って描く色と混ぜる（絵の具の量・濃さ・
//! 色延び・筆圧）、伸ばすは動きの後ろの色を引きずる、下地は今のレイヤーか見えているレイヤーの重なり、並列と直列で同じ画素、選択範囲・透明部分のロック、
//! 2D の対称（左右が鏡像）、3D の面でも効く（離れたアイランドにまたがるダブは塊ごとに下地を凍結）、取消・予算・Undo、色以外のチャンネル・消しゴム・
//! 効果のブラシでは混ぜない。式そのものは `brush/mix.rs` の単体試験と `docs/BRUSH.md`。
//! 束に入れず直下の 1 本: ワーカーの閾値（`yolu_core::brush::set_parallel_dab_pixels`。プロセスで 1 つ）を最大と 0 に切り替えて、直列の経路だけ・ワーカーの経路を通ることを確かめる（このファイルの中では `THRESHOLD` で順に走らせる）。同じプロセスのほかの試験が閾値を変えると外れる。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::sync::{Arc, Mutex};

use yolu_core::geometry::{
    CameraView, OrbitCamera, SurfaceGeometry, SurfaceStroke, SurfaceStrokeOptions, SurfaceTriangle,
};
use yolu_core::glam::{DVec2, Vec2, Vec3};
use yolu_core::material::ChannelPaint;
use yolu_core::{
    Brush, BrushEffect, BrushMappedPixel, BrushPixel, BrushSample, BrushSettings, CanvasSymmetry,
    Channel, ColorMix, CoreError, Document, InputResponse, LayerId, LayerLocks, MixBlur, MixGround,
    MixMode, MixSpace, Rgba8, SelectionMask, SymmetryMode, TileCoord,
};

const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);
const WHITE: Rgba8 = Rgba8::new(255, 255, 255, 255);
const RED: Rgba8 = Rgba8::new(255, 0, 0, 255);

/// ワーカーの閾値を変える試験は、同じ試験ファイルの中で入れ替わらないようにする。
static THRESHOLD: Mutex<()> = Mutex::new(());

struct Restore(i64);
impl Drop for Restore {
    fn drop(&mut self) {
        yolu_core::brush::set_parallel_dab_pixels(self.0);
    }
}

/// キャンバス（タイル 16）。`ground(x, y)` で最初のレイヤーを塗る（画素の座標は左下原点）。
fn canvas(w: u32, h: u32, ts: u32, ground: impl Fn(u32, u32) -> Rgba8) -> (Document, LayerId) {
    let mut d = Document::with_tile_size(w, h, ts).unwrap();
    let l = d.add_layer("L").unwrap();
    fill(&mut d, l, ts, &ground);
    d.clear_history().unwrap();
    (d, l)
}

fn fill(d: &mut Document, l: LayerId, ts: u32, ground: &impl Fn(u32, u32) -> Rgba8) {
    let (w, h) = (d.width(), d.height());
    for ty in 0..h.div_ceil(ts) {
        for tx in 0..w.div_ceil(ts) {
            let mut b = vec![0u8; (ts * ts * 4) as usize];
            let mut any = false;
            for y in 0..ts {
                for x in 0..ts {
                    let (px, py) = (tx * ts + x, ty * ts + y);
                    if px >= w || py >= h {
                        continue;
                    }
                    let c = ground(px, py);
                    any |= c.a != 0;
                    let o = ((y * ts + x) * 4) as usize;
                    b[o..o + 4].copy_from_slice(&c.to_array());
                }
            }
            if any {
                d.import_tile(l, Channel::Color, TileCoord::new(tx, ty), &b)
                    .unwrap();
            }
        }
    }
}

fn settings(radius: f64, color: Rgba8) -> BrushSettings {
    BrushSettings {
        radius,
        hardness: 1.0,
        spacing: 0.1,
        opacity: 1.0,
        flow: 1.0,
        color,
        ..BrushSettings::default()
    }
}

fn mixing(radius: f64, color: Rgba8, mode: MixMode, paint: f64) -> Brush {
    Brush {
        mix: ColorMix {
            mode,
            paint,
            density: 1.0,
            stretch: 0.0,
            ..ColorMix::default()
        },
        ..Brush::simple(settings(radius, color), yolu_core::PressureSwitches::NONE)
    }
}

fn sample(x: f64, y: f64, pressure: f64, time: f64) -> BrushSample {
    BrushSample::new(x, y, pressure, time, DVec2::ZERO).unwrap()
}

/// 点列を引いて確定する。
fn draw(d: &mut Document, l: LayerId, b: &Brush, points: &[(f64, f64, f64)]) {
    let mut s = d.begin_brush_stroke(l, b).unwrap();
    for (i, p) in points.iter().enumerate() {
        s.add_sample(d, sample(p.0, p.1, p.2, i as f64 * 0.01))
            .unwrap();
    }
    d.end_stroke(s).unwrap();
}

fn dot(d: &mut Document, l: LayerId, b: &Brush, x: f64, y: f64, pressure: f64) {
    draw(d, l, b, &[(x, y, pressure)]);
}

fn px(d: &Document, l: LayerId, x: u32, y: u32) -> Rgba8 {
    d.layer(l).unwrap().pixel(Channel::Color, x, y).unwrap()
}

fn bytes(d: &Document, l: LayerId) -> Vec<u8> {
    d.layer(l)
        .unwrap()
        .surface(Channel::Color)
        .map_or_else(Vec::new, |s| s.to_canvas_bytes())
}

fn near(a: Rgba8, b: Rgba8, tolerance: u8) -> bool {
    [(a.r, b.r), (a.g, b.g), (a.b, b.b), (a.a, b.a)]
        .iter()
        .all(|(x, y)| x.abs_diff(*y) <= tolerance)
}

fn line(from: (f64, f64), to: (f64, f64), n: usize) -> Vec<(f64, f64, f64)> {
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            (
                from.0 + (to.0 - from.0) * t,
                from.1 + (to.1 - from.1) * t,
                1.0,
            )
        })
        .collect()
}

// ───────── 切のブラシは今までと同じ ─────────

#[test]
fn a_brush_with_mixing_off_ignores_every_mix_value() {
    let ground = |x: u32, y: u32| Rgba8::new((x * 4) as u8, (y * 4) as u8, 120, 255);
    let path = line((8.0, 20.0), (56.0, 44.0), 24);
    let mut plain = Brush::simple(settings(9.0, RED), yolu_core::PressureSwitches::NONE);
    plain.influences.opacity.pressure.on = true;
    let mut off = plain.clone();
    off.mix = ColorMix {
        mode: MixMode::Off,
        paint: 0.1,
        density: 0.2,
        stretch: 0.9,
        ground: MixGround::Composite,
        ..ColorMix::default()
    };
    off.influences.mix_paint.pressure = yolu_core::InputUse {
        on: true,
        response: InputResponse::new(0.3, vec![]).unwrap(),
    };
    off.influences.mix_density.pressure = yolu_core::InputUse {
        on: true,
        response: InputResponse::new(0.4, vec![]).unwrap(),
    };
    let (mut a, la) = canvas(64, 64, 16, ground);
    let (mut b, lb) = canvas(64, 64, 16, ground);
    draw(&mut a, la, &plain, &path);
    draw(&mut b, lb, &off, &path);
    assert_eq!(bytes(&a, la), bytes(&b, lb));
    let (untouched, lu) = canvas(64, 64, 16, ground);
    assert_ne!(bytes(&a, la), bytes(&untouched, lu), "描けている");
    assert!(plain.validate().is_ok() && off.validate().is_ok());
}

#[test]
fn bad_mix_values_are_refused_before_a_stroke_starts() {
    let (mut d, l) = canvas(32, 32, 16, |_, _| WHITE);
    for field in 0..3 {
        let mut b = mixing(5.0, RED, MixMode::Mix, 0.5);
        match field {
            0 => b.mix.paint = 1.5,
            1 => b.mix.density = -0.1,
            _ => b.mix.stretch = f64::NAN,
        }
        assert!(b.validate().is_err());
        assert!(matches!(
            d.begin_brush_stroke(l, &b),
            Err(CoreError::InvalidArgument(_))
        ));
    }
    assert!(!d.has_active_stroke());
}

// ───────── 混ぜる ─────────

#[test]
fn mixing_blends_the_colour_under_the_brush_by_the_paint_amount() {
    let (mut plain, lp) = canvas(64, 64, 16, |_, _| BLUE);
    dot(
        &mut plain,
        lp,
        &Brush::simple(settings(10.0, RED), yolu_core::PressureSwitches::NONE),
        32.0,
        32.0,
        1.0,
    );
    assert_eq!(px(&plain, lp, 32, 32), RED);

    for (paint, expect) in [
        (1.0, RED),                          // 下の色を拾わない
        (0.5, Rgba8::new(128, 0, 128, 255)), // 半々
        (0.0, BLUE),                         // 下の色だけ（何も足さない）
    ] {
        let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
        dot(
            &mut d,
            l,
            &mixing(10.0, RED, MixMode::Mix, paint),
            32.0,
            32.0,
            1.0,
        );
        assert_eq!(px(&d, l, 32, 32), expect, "paint {paint}");
        // ダブの外は変わらない
        assert_eq!(px(&d, l, 5, 5), BLUE);
    }
}

#[test]
fn the_paint_density_sets_how_much_of_the_mixed_colour_is_laid() {
    let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
    let mut b = mixing(10.0, RED, MixMode::Mix, 1.0);
    b.mix.density = 0.5;
    dot(&mut d, l, &b, 32.0, 32.0, 1.0);
    // 赤を半分だけ置く（天井が 0.5）
    assert!(
        near(px(&d, l, 32, 32), Rgba8::new(128, 0, 128, 255), 1),
        "{:?}",
        px(&d, l, 32, 32)
    );
    // 濃さ 0 は何も置かない
    let (mut z, lz) = canvas(64, 64, 16, |_, _| BLUE);
    let mut b = mixing(10.0, RED, MixMode::Mix, 1.0);
    b.mix.density = 0.0;
    dot(&mut z, lz, &b, 32.0, 32.0, 1.0);
    assert_eq!(px(&z, lz, 32, 32), BLUE);
}

#[test]
fn a_transparent_ground_gives_the_plain_brush_colour_at_any_paint_amount() {
    let (mut plain, lp) = canvas(64, 64, 16, |_, _| Rgba8::TRANSPARENT);
    let path = line((10.0, 30.0), (50.0, 34.0), 20);
    draw(
        &mut plain,
        lp,
        &Brush::simple(
            settings(8.0, Rgba8::new(200, 60, 30, 255)),
            yolu_core::PressureSwitches::NONE,
        ),
        &path,
    );
    for paint in [0.0, 0.3, 1.0] {
        let (mut d, l) = canvas(64, 64, 16, |_, _| Rgba8::TRANSPARENT);
        let mut b = mixing(8.0, Rgba8::new(200, 60, 30, 255), MixMode::Mix, paint);
        b.mix.stretch = 0.7;
        draw(&mut d, l, &b, &path);
        if paint == 0.0 {
            // 量 0 は混ぜる色が無い（透明な下地には何も足さない）
            assert_eq!(bytes(&d, l).iter().filter(|v| **v != 0).count(), 0);
        } else {
            // 薄まらない: 画素の位置も色もアルファも、素のブラシと同じ（丸めの 1 まで）
            let (x, y) = (30, 32);
            assert!(near(px(&d, l, x, y), px(&plain, lp, x, y), 1), "{paint}");
            assert_eq!(px(&d, l, x, y).a, 255);
        }
    }
}

#[test]
fn the_paint_amount_and_density_follow_the_pen_pressure_when_asked() {
    let ink = |pressure_paint: bool, pressure_density: bool| {
        let mut b = mixing(10.0, RED, MixMode::Mix, 1.0);
        b.influences.mix_paint.pressure.on = pressure_paint;
        b.influences.mix_density.pressure.on = pressure_density;
        b
    };
    // 筆圧 0 で量 0（下の色だけ）、筆圧 1 で量 1（赤）
    for (pressure, expect) in [(0.0, BLUE), (1.0, RED)] {
        let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
        dot(&mut d, l, &ink(true, false), 32.0, 32.0, pressure);
        assert_eq!(px(&d, l, 32, 32), expect, "量 {pressure}");
    }
    // 濃さ: 筆圧 0 は置かず、筆圧 1 は置く。最小値があれば筆圧 0 でも置く
    let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
    dot(&mut d, l, &ink(false, true), 32.0, 32.0, 0.0);
    assert_eq!(px(&d, l, 32, 32), BLUE);
    let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
    let mut b = ink(false, true);
    b.influences.mix_density.pressure.response = InputResponse::new(0.5, vec![]).unwrap();
    dot(&mut d, l, &b, 32.0, 32.0, 0.0);
    assert!(near(px(&d, l, 32, 32), Rgba8::new(128, 0, 128, 255), 1));
    // 切っていれば筆圧は使わない（筆圧 0 でも赤）
    let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
    dot(&mut d, l, &ink(false, false), 32.0, 32.0, 0.0);
    assert_eq!(px(&d, l, 32, 32), RED);
}

/// 左半分が青、右半分が白の下地。
fn split(x: u32, _y: u32) -> Rgba8 {
    if x < 32 {
        BLUE
    } else {
        WHITE
    }
}

#[test]
fn the_stretch_drags_the_picked_colour_into_the_next_dabs() {
    let across = line((8.0, 32.0), (56.0, 32.0), 48);
    let run = |stretch: f64| {
        let (mut d, l) = canvas(64, 64, 16, split);
        // 絵の具の量が多いほど、荷は 1 打点ごとに少しずつしか下地へ入れ替わらない（引きずりが長く残る）
        let mut b = mixing(6.0, WHITE, MixMode::Mix, 0.9);
        b.mix.stretch = stretch;
        draw(&mut d, l, &b, &across);
        (px(&d, l, 44, 32), px(&d, l, 55, 32))
    };
    let (none_near, none_far) = run(0.0);
    let (full_near, full_far) = run(1.0);
    // 白い側の境から離れた所: 色延び 0 は毎回白から始めるので白のまま、1 は青い側で拾った色を引きずって青みを持つ
    assert_eq!(none_near, WHITE);
    assert_eq!(none_far, WHITE);
    assert!(
        full_near.r < 235 && full_near.b >= full_near.r,
        "{full_near:?}"
    );
    // 引きずりは先へ行くほど薄れる（荷が白い下地へ入れ替わる）
    assert!(full_far.r >= full_near.r, "{full_near:?} {full_far:?}");
}

#[test]
fn smearing_pulls_the_colour_from_behind_along_the_stroke() {
    let across = line((10.0, 32.0), (54.0, 32.0), 44);
    let run = |mode: MixMode| {
        let (mut d, l) = canvas(64, 64, 16, split);
        let mut b = mixing(8.0, WHITE, mode, 0.0);
        b.mix.stretch = 1.0;
        draw(&mut d, l, &b, &across);
        (d, l)
    };
    // 量 0（下の色だけ）: 混ぜるは同じ画素を読むので、何も変わらない
    let (mixed, lm) = run(MixMode::Mix);
    assert_eq!(px(&mixed, lm, 35, 32), WHITE);
    assert_eq!(px(&mixed, lm, 20, 32), BLUE);
    // 伸ばすは動きの後ろ（青）を白い側へ引きずる
    let (smeared, ls) = run(MixMode::Smear);
    let c = px(&smeared, ls, 35, 32);
    assert!(c.r < 250 && c.b == 255, "{c:?}");
    assert!(px(&smeared, ls, 20, 32).r < 20, "青い側は青いまま");
}

// ───────── 下地 ─────────

#[test]
fn the_composite_ground_reads_the_layers_below() {
    let build = || {
        let mut d = Document::with_tile_size(64, 64, 16).unwrap();
        let below = d.add_layer("below").unwrap();
        fill(&mut d, below, 16, &|_, _| RED);
        let top = d.add_layer("top").unwrap();
        d.clear_history().unwrap();
        (d, top)
    };
    let brush = |ground: MixGround| {
        let mut b = mixing(10.0, Rgba8::new(0, 0, 0, 255), MixMode::Mix, 0.5);
        b.mix.ground = ground;
        b
    };
    // 今のレイヤーだけ: 上のレイヤーは空なので何も拾えず、描く色のまま
    let (mut layer_only, top) = build();
    dot(
        &mut layer_only,
        top,
        &brush(MixGround::Layer),
        32.0,
        32.0,
        1.0,
    );
    assert_eq!(px(&layer_only, top, 32, 32), Rgba8::new(0, 0, 0, 255));
    // 見えているレイヤーの重なり: 下の赤を拾って混ぜる
    let (mut composite, top) = build();
    let b = brush(MixGround::Composite);
    let mut s = composite.begin_brush_stroke(top, &b).unwrap();
    s.use_composite_clone_source(&mut composite).unwrap();
    s.add_sample(&mut composite, sample(32.0, 32.0, 1.0, 0.0))
        .unwrap();
    composite.end_stroke(s).unwrap();
    assert_eq!(px(&composite, top, 32, 32), Rgba8::new(128, 0, 0, 255));
    // 下のレイヤーは変わらない
    let below = composite.layers()[0].id();
    assert_eq!(px(&composite, below, 32, 32), RED);
    // 参照元を凍結しなければ、今のレイヤーから読む（拾うものが無いので描く色）
    let (mut forgot, top) = build();
    dot(&mut forgot, top, &b, 32.0, 32.0, 1.0);
    assert_eq!(px(&forgot, top, 32, 32), Rgba8::new(0, 0, 0, 255));
}

#[test]
fn the_composite_ground_reads_the_picture_frozen_before_the_stroke() {
    // 同じ場所を 2 回通っても、見えているレイヤーの重なりは最初の打点の前の絵のまま（このストロークの描きは読み返さない）
    let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
    let mut b = mixing(8.0, RED, MixMode::Mix, 0.5);
    b.mix.ground = MixGround::Composite;
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    s.use_composite_clone_source(&mut d).unwrap();
    for (i, p) in [(20.0, 32.0), (44.0, 32.0), (20.0, 32.0)]
        .iter()
        .enumerate()
    {
        s.add_sample(&mut d, sample(p.0, p.1, 1.0, i as f64))
            .unwrap();
    }
    d.end_stroke(s).unwrap();
    // 何度重ねても、半々の紫を超えて赤くはならない（今のレイヤーから読むと、重ねるたびに赤へ寄る）
    assert_eq!(px(&d, l, 20, 32), Rgba8::new(128, 0, 128, 255));
}

#[test]
fn mixing_does_not_depend_on_how_often_the_stroke_overlaps_a_pixel() {
    // 混ぜるはストロークを始める前の絵を読む: 同じ場所を何度通っても、行き来しても、1 回だけと同じ混ざり方（重ねるたびに描く色へ寄らない）
    let b = mixing(8.0, RED, MixMode::Mix, 0.5);
    let once = {
        let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
        draw(&mut d, l, &b, &[(20.0, 32.0, 1.0)]);
        px(&d, l, 20, 32)
    };
    let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
    let path = [(20.0, 32.0, 1.0), (44.0, 32.0, 1.0), (20.0, 32.0, 1.0)];
    draw(&mut d, l, &b, &path);
    assert_eq!(px(&d, l, 20, 32), once);
    assert_eq!(once, Rgba8::new(128, 0, 128, 255));
    // 2 本目のストロークは、1 本目が置いた色を下地にする（重ねて塗るほど描く色へ寄る）
    draw(&mut d, l, &b, &[(20.0, 32.0, 1.0)]);
    assert!(px(&d, l, 20, 32).r > once.r, "{:?}", px(&d, l, 20, 32));
}

#[test]
fn smearing_reads_the_layer_as_it_is_while_the_stroke_draws() {
    // 伸ばすは今の面を読む: 先に引きずった色を後の打点が運ぶので、青い側から遠い所にも青が届く
    let across = line((10.0, 32.0), (54.0, 32.0), 44);
    let (mut d, l) = canvas(64, 64, 16, split);
    let mut b = mixing(8.0, WHITE, MixMode::Smear, 0.0);
    b.mix.stretch = 1.0;
    draw(&mut d, l, &b, &across);
    let far = px(&d, l, 40, 32);
    assert!(far.r < 250, "{far:?}");
}

// ───────── 並列と直列 ─────────

fn fingerprint(d: &Document, l: LayerId) -> (usize, usize, Vec<u8>) {
    let s = d.layer(l).unwrap().surface(Channel::Color).unwrap();
    (
        s.tile_count(),
        s.allocated_bytes() as usize,
        s.to_canvas_bytes(),
    )
}

fn big_mixing_brushes() -> Vec<(&'static str, Brush)> {
    let mut mix = mixing(60.0, Rgba8::new(220, 90, 40, 255), MixMode::Mix, 0.5);
    mix.base.hardness = 0.5;
    mix.influences.opacity.pressure.on = true;
    mix.mix.stretch = 0.6;
    mix.influences.mix_paint.pressure.on = true;
    let mut smear = mixing(60.0, Rgba8::new(30, 160, 220, 255), MixMode::Smear, 0.3);
    smear.mix.stretch = 0.8;
    smear.mix.density = 0.8;
    let mut dynamic = mixing(55.0, Rgba8::new(200, 200, 40, 255), MixMode::Mix, 0.4);
    dynamic.seed = 5;
    dynamic.color.hue = 0.3;
    dynamic.color.per_tip = true;
    dynamic.jitter.scatter = 0.2;
    dynamic.jitter.size = 0.3;
    dynamic.mix.stretch = 0.5;
    // 下地混色の拡張: 重ねる・知覚的（輝度の補正 2）・にじむ強さの固定値・筆の初めの絵の具の量
    let mut layer = mixing(60.0, Rgba8::new(40, 200, 120, 255), MixMode::Layer, 0.7);
    layer.mix.stretch = 0.9;
    layer.base.hardness = 0.6;
    let mut perceptual = mixing(58.0, Rgba8::new(240, 208, 0, 255), MixMode::Mix, 0.6);
    perceptual.mix.space = MixSpace::Perceptual;
    perceptual.mix.correction = 2;
    perceptual.mix.stretch = 0.8;
    let mut fixed_blur = mixing(60.0, Rgba8::new(30, 160, 220, 255), MixMode::Smear, 0.3);
    fixed_blur.mix.stretch = 0.7;
    fixed_blur.mix.blur = MixBlur::Fixed(6.0);
    let mut loaded = mixing(60.0, Rgba8::new(220, 90, 40, 255), MixMode::Mix, 0.8);
    loaded.mix.load = 0.3;
    loaded.mix.stretch = 0.9;
    vec![
        ("mix", mix),
        ("smear", smear),
        ("dynamic", dynamic),
        ("layer", layer),
        ("perceptual", perceptual),
        ("fixed blur", fixed_blur),
        ("load", loaded),
    ]
}

fn busy_ground(x: u32, y: u32) -> Rgba8 {
    let v = (x * 7 + y * 13) % 256;
    if x > 450 {
        Rgba8::TRANSPARENT
    } else {
        Rgba8::new(v as u8, (255 - v) as u8, ((x / 3 + y / 5) % 256) as u8, 255)
    }
}

fn big_path() -> Vec<BrushSample> {
    (0..=50)
        .map(|i| {
            let t = i as f64 / 50.0;
            BrushSample::new(
                60.0 + 480.0 * t,
                180.0 + 50.0 * (t * 6.0).sin(),
                0.3 + 0.7 * (t * 3.0).sin().abs(),
                i as f64 * 0.01,
                DVec2::ZERO,
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn large_mixing_dabs_give_the_same_pixels_on_the_serial_and_the_parallel_paths() {
    let _guard = THRESHOLD.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = Restore(yolu_core::brush::set_parallel_dab_pixels(i64::MAX));
    for (name, brush) in big_mixing_brushes() {
        let run = |threshold: i64, threads: usize, composite: bool| {
            yolu_core::brush::set_parallel_dab_pixels(threshold);
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            pool.install(|| {
                let (mut d, l) = canvas(600, 360, 64, busy_ground);
                let mut b = brush.clone();
                if composite {
                    b.mix.ground = MixGround::Composite;
                }
                let mut s = d.begin_brush_stroke(l, &b).unwrap();
                if composite {
                    s.use_composite_clone_source(&mut d).unwrap();
                }
                for p in big_path() {
                    s.add_sample(&mut d, p).unwrap();
                }
                let parallel = d.active_stroke_stats().unwrap().parallel_dabs;
                d.end_stroke(s).unwrap();
                (fingerprint(&d, l), parallel)
            })
        };
        for composite in [false, true] {
            let (serial, serial_dabs) = run(i64::MAX, 1, composite);
            assert_eq!(serial_dabs, 0, "{name}: 閾値が大きいときは直列の経路");
            let original = canvas(600, 360, 64, busy_ground);
            assert_ne!(
                serial.2,
                bytes(&original.0, original.1),
                "{name}: 描けている"
            );
            for threads in [2, 3, 8] {
                let (parallel, parallel_dabs) = run(0, threads, composite);
                assert!(parallel_dabs > 0, "{name}: ワーカーの経路を通る");
                assert!(
                    parallel == serial,
                    "{name} composite {composite} threads {threads}: 経路で変わった"
                );
            }
        }
    }
}

// ───────── 取消・予算・Undo ─────────

#[test]
fn cancelling_a_mixing_stroke_restores_the_exact_pixels_and_undo_redo_round_trips() {
    let ground = |x: u32, y: u32| Rgba8::new((x * 3) as u8, (y * 3) as u8, 90, 255);
    let (mut d, l) = canvas(64, 64, 16, ground);
    let before = bytes(&d, l);
    let b = mixing(9.0, RED, MixMode::Mix, 0.4);
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    for (i, p) in line((8.0, 20.0), (56.0, 44.0), 12).iter().enumerate() {
        s.add_sample(&mut d, sample(p.0, p.1, 1.0, i as f64 * 0.01))
            .unwrap();
    }
    assert_ne!(bytes(&d, l), before, "途中は描けている");
    d.cancel_stroke(s);
    assert_eq!(bytes(&d, l), before);
    assert!(!d.can_undo());
    // 確定 → Undo → Redo
    draw(&mut d, l, &b, &line((8.0, 20.0), (56.0, 44.0), 12));
    let after = bytes(&d, l);
    assert_ne!(after, before);
    d.undo().unwrap();
    assert_eq!(bytes(&d, l), before);
    d.redo().unwrap();
    assert_eq!(bytes(&d, l), after);
}

#[test]
fn the_pixels_read_for_mixing_count_against_the_stroke_budget() {
    let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
    let before = bytes(&d, l);
    let b = mixing(10.0, RED, MixMode::Mix, 0.5);
    // 読む枠（21 × 21 画素 × 4 バイト = 1764）が予算に入らない: 最初のダブでストロークごと断る
    d.set_stroke_budget_bytes(1000).unwrap();
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    let err = s
        .add_sample(&mut d, sample(32.0, 32.0, 1.0, 0.0))
        .unwrap_err();
    assert_eq!(err, CoreError::StrokeBudgetExceeded);
    assert!(!d.has_active_stroke());
    assert_eq!(bytes(&d, l), before);
    assert_eq!(d.undo_count(), 0);
    // 予算を戻せば同じ入力が通る
    d.set_stroke_budget_bytes(64 << 20).unwrap();
    dot(&mut d, l, &b, 32.0, 32.0, 1.0);
    assert_ne!(bytes(&d, l), before);
    // 伸ばすは箱の積分画像の分も数える（混ぜるより多く要る）
    let smear = mixing(10.0, RED, MixMode::Smear, 0.5);
    let need = |brush: &Brush| {
        let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
        let mut s = d.begin_brush_stroke(l, brush).unwrap();
        s.add_sample(&mut d, sample(32.0, 32.0, 1.0, 0.0)).unwrap();
        let held = d.active_stroke_stats().unwrap().rollback_bytes;
        d.cancel_stroke(s);
        held
    };
    let mut budget = 1000u64;
    // 混ぜるが通る最小に近い予算で、伸ばす（枠が大きい）は断る
    let mix_need = need(&b);
    budget = budget.max(mix_need + 21 * 21 * 4 + 64);
    let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
    d.set_stroke_budget_bytes(budget).unwrap();
    let mut s = d.begin_brush_stroke(l, &smear).unwrap();
    let r = (0..5)
        .try_for_each(|i| s.add_sample(&mut d, sample(30.0 + i as f64 * 2.0, 32.0, 1.0, i as f64)));
    assert_eq!(r, Err(CoreError::StrokeBudgetExceeded));
    assert!(!d.has_active_stroke());
}

// ───────── 選択範囲・透明部分のロック ─────────

/// キャンバスを左から 16 画素ずつの 4 つのタイル（x 0..15・16..31・32..47・48..63）に分けた選択範囲: 量は左から 255・128・0・0。
fn banded_selection(d: &Document) -> SelectionMask {
    let band = |amount: u8| vec![amount; 16 * 16];
    SelectionMask::from_amount_tiles(
        d.width(),
        d.height(),
        16,
        [
            (TileCoord::new(0, 0), band(255)),
            (TileCoord::new(1, 0), band(128)),
        ],
    )
    .unwrap()
}

#[test]
fn a_selection_cuts_the_mixed_colour_by_its_amount_and_leaves_the_rest_alone() {
    // 全選択（255）の画素は混ざった色になり、半分選ばれた画素は描く前の画素から半分だけそこへ寄り、選択外は 1 バイトも変わらない
    for mode in [MixMode::Mix, MixMode::Smear, MixMode::Layer] {
        let (mut d, l) = canvas(64, 16, 16, |_, _| BLUE);
        d.set_selection(Some(banded_selection(&d))).unwrap();
        d.clear_history().unwrap();
        let before = bytes(&d, l);
        let b = mixing(30.0, RED, mode, 0.5);
        dot(&mut d, l, &b, 32.0, 8.0, 1.0);
        let mixed = Rgba8::new(128, 0, 128, 255);
        assert_eq!(px(&d, l, 8, 8), mixed, "{mode:?} 全選択");
        assert_eq!(
            px(&d, l, 24, 8),
            yolu_core::blend::fade(BLUE, mixed, 128.0 / 255.0),
            "{mode:?} 半分選ばれた画素"
        );
        for x in 32..64 {
            assert_eq!(px(&d, l, x, 8), BLUE, "{mode:?} 選択外 x={x}");
        }
        // 選択外の画素は、バイト単位で元のまま（選択外のタイルは写しも取らない）
        let after = bytes(&d, l);
        for y in 0..16usize {
            assert_eq!(
                after[(y * 64 + 32) * 4..(y * 64 + 64) * 4],
                before[(y * 64 + 32) * 4..(y * 64 + 64) * 4],
                "{mode:?} 行 {y}"
            );
        }
        // 取消・Undo は選択範囲があっても元へ戻る
        d.undo().unwrap();
        assert_eq!(bytes(&d, l), before, "{mode:?}");
    }
}

/// 幅 64 のキャンバスの左半分（x 0..31）の画素。
fn left_half(d: &Document, l: LayerId) -> Vec<Rgba8> {
    (0..d.height())
        .flat_map(|y| (0..32).map(move |x| (x, y)))
        .map(|(x, y)| px(d, l, x, y))
        .collect()
}

#[test]
fn the_brush_does_not_pick_up_the_ground_of_pixels_it_cannot_paint() {
    // 混ぜるは、選択外の画素は塗らないので荷にも入らない: 選択外の下地の色を変えても、選択内の結果は 1 バイトも変わらない
    let inside = |outside: Rgba8, mode: MixMode| {
        let ground = move |x: u32, _y: u32| if x < 32 { BLUE } else { outside };
        let (mut d, l) = canvas(64, 16, 16, ground);
        d.set_selection(Some(banded_selection(&d))).unwrap();
        d.clear_history().unwrap();
        let mut b = mixing(5.0, RED, mode, 0.5);
        b.mix.stretch = 1.0;
        // 選択の縁をまたいで、選択内へ引いていく（縁の打点は選択外の下地も覆い、後の打点が荷を運ぶ）
        draw(&mut d, l, &b, &line((36.0, 8.0), (4.0, 8.0), 32));
        left_half(&d, l)
    };
    // （混ぜるだけ。伸ばすは動きの後ろの色を読むので、ぼかしや指先と同じく、選択外の色も読む）
    let with_green = inside(Rgba8::new(0, 255, 0, 255), MixMode::Mix);
    let with_white = inside(WHITE, MixMode::Mix);
    assert!(
        with_green == with_white,
        "選択内の結果が選択外の下地で変わった"
    );
    // 対照: 選択が無ければ、縁をまたぐ打点は隣の下地の色を拾うので、結果が変わる
    let free = |outside: Rgba8| {
        let ground = move |x: u32, _y: u32| if x < 32 { BLUE } else { outside };
        let (mut d, l) = canvas(64, 16, 16, ground);
        let mut b = mixing(5.0, RED, MixMode::Mix, 0.5);
        b.mix.stretch = 1.0;
        draw(&mut d, l, &b, &line((36.0, 8.0), (4.0, 8.0), 32));
        left_half(&d, l)
    };
    assert_ne!(free(Rgba8::new(0, 255, 0, 255)), free(WHITE));
}

#[test]
fn a_transparency_lock_keeps_the_transparent_pixels_and_mixes_the_opaque_ones() {
    // 透明部分のロック: 透明な画素は RGB もアルファも守り（下に隠れた RGB も）、不透明な画素はアルファを保ったまま混ざる
    let hidden = Rgba8::new(9, 8, 7, 0);
    for mode in [MixMode::Mix, MixMode::Smear, MixMode::Layer] {
        let ground = move |x: u32, _y: u32| if x < 8 { BLUE } else { hidden };
        let (mut d, l) = canvas(16, 16, 16, ground);
        d.set_layer_locks(l, LayerLocks::TRANSPARENCY).unwrap();
        d.clear_history().unwrap();
        let before = bytes(&d, l);
        dot(&mut d, l, &mixing(6.0, RED, mode, 0.5), 8.0, 8.0, 1.0);
        assert_eq!(px(&d, l, 5, 8), Rgba8::new(128, 0, 128, 255), "{mode:?}");
        for x in 8..14 {
            assert_eq!(px(&d, l, x, 8), hidden, "{mode:?} 透明な画素 x={x}");
        }
        let after = bytes(&d, l);
        for y in 0..16usize {
            assert_eq!(
                after[(y * 16 + 8) * 4..(y * 16 + 16) * 4],
                before[(y * 16 + 8) * 4..(y * 16 + 16) * 4],
                "{mode:?} 行 {y}"
            );
        }
        d.undo().unwrap();
        assert_eq!(bytes(&d, l), before, "{mode:?}");
    }
}

// ───────── 対称 ─────────

/// 縦の軸（x = 幅の半分）の左右で同じ色になる下地（x の鏡像の画素が同じ色）。
fn mirrored_ground(w: u32) -> impl Fn(u32, u32) -> Rgba8 {
    move |x, y| {
        let d = (x as i32 * 2 + 1 - w as i32).unsigned_abs();
        Rgba8::new((d * 3).min(255) as u8, (y * 4).min(255) as u8, 90, 255)
    }
}

/// 縦の対称の左右が鏡像か（画素 x と w − 1 − x が同じ）。
fn is_mirror_image(d: &Document, l: LayerId) -> bool {
    let w = d.width();
    (0..d.height()).all(|y| (0..w / 2).all(|x| px(d, l, x, y) == px(d, l, w - 1 - x, y)))
}

#[test]
fn a_mirror_symmetric_mix_or_smear_gives_a_mirror_image() {
    // 写しは元と動きの向きが逆なので、伸ばすは動きの向きを使わず（同じ画素の周りの箱だけ）、混ぜるも伸ばすも左右が鏡像になる
    let w = 64u32;
    for mode in [MixMode::Mix, MixMode::Smear] {
        let (mut d, l) = canvas(w, 32, 16, mirrored_ground(w));
        let before = bytes(&d, l);
        let mut b = mixing(6.0, RED, mode, 0.6);
        b.mix.stretch = 0.8;
        b.symmetry =
            CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(w as f64 / 2.0, 16.0), 2)
                .unwrap();
        // 左の半分だけを、右向きに引く（元の動きは右向き、写しは左向き）
        draw(&mut d, l, &b, &line((6.0, 14.0), (24.0, 18.0), 24));
        assert_ne!(bytes(&d, l), before, "{mode:?} 描けている");
        assert_ne!(
            px(&d, l, 20, 16),
            px(&d, l, 20, 0),
            "{mode:?} 左が塗れている"
        );
        assert!(is_mirror_image(&d, l), "{mode:?} 左右が鏡像");
        // 取消・Undo
        d.undo().unwrap();
        assert_eq!(bytes(&d, l), before, "{mode:?}");
    }
}

#[test]
fn symmetric_copies_far_apart_read_only_their_own_regions() {
    // 対称の写しがキャンバスの両端にあるとき、外接の箱（キャンバスの幅いっぱい）を枠にせず、写しごとの小さな範囲を読む:
    // 外接の箱なら要る量よりずっと小さい予算でも、ストロークは取り消されず、両側が混ざる
    let (w, h) = (4096u32, 32u32);
    for mode in [MixMode::Mix, MixMode::Smear] {
        let ground = |_x: u32, _y: u32| BLUE;
        let (mut d, l) = canvas(w, h, 16, ground);
        let before = bytes(&d, l);
        let mut b = mixing(4.0, RED, mode, 0.5);
        b.symmetry =
            CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(w as f64 / 2.0, 16.0), 2)
                .unwrap();
        // 外接の箱の枠だけで 4072 × 9 × 4 = 約 147 KB（伸ばすは積分画像の分も）になる。それより小さい予算（写しの画素の置き場と、写しごとの
        // 枠とタイルの写しが入る大きさ）
        d.set_stroke_budget_bytes(60_000).unwrap();
        let mut s = d.begin_brush_stroke(l, &b).unwrap();
        s.add_sample(&mut d, sample(12.0, 16.0, 1.0, 0.0)).unwrap();
        assert!(d.has_active_stroke(), "{mode:?} 取り消されていない");
        d.end_stroke(s).unwrap();
        // 下地は一様な青なので、混ぜるも伸ばすも（箱の平均も青）同じ色
        let mixed = Rgba8::new(128, 0, 128, 255);
        assert_eq!(px(&d, l, 12, 16), mixed, "{mode:?} 元");
        assert_eq!(px(&d, l, w - 1 - 12, 16), mixed, "{mode:?} 写し");
        assert_eq!(px(&d, l, w / 2, 16), BLUE, "{mode:?} 間は触れない");
        d.undo().unwrap();
        assert_eq!(bytes(&d, l), before, "{mode:?}");
    }
}

// ───────── 離れた塊を持つ面のダブ ─────────

#[test]
fn a_surface_dab_over_two_distant_islands_freezes_only_the_pixels_it_reads() {
    // UV の離れた 2 つのアイランドにまたがるダブ（継ぎ目のダブ）: 外接の箱（キャンバスの大半）を枠にすると予算を超えてストロークごと取り消されるが、
    // 塊ごとに凍結するので、小さな予算でも取り消されず、両方のアイランドが混ざる。ダブの外は触れない
    let size = 512u32;
    let island = |x0: i64, y0: i64| -> Vec<BrushPixel> {
        (0..8)
            .flat_map(|dy| {
                (0..8).map(move |dx| BrushPixel {
                    x: x0 + dx,
                    y: y0 + dy,
                    coverage: 1.0,
                })
            })
            .collect()
    };
    let mut pixels = island(20, 20);
    pixels.extend(island(480, 480));
    for mode in [MixMode::Mix, MixMode::Smear] {
        let (mut d, l) = canvas(size, size, 16, |_, _| BLUE);
        let before = bytes(&d, l);
        // 外接の箱の枠は 468 × 468 × 4 = 約 876 KB。それより十分小さい予算
        d.set_stroke_budget_bytes(120_000).unwrap();
        let b = mixing(3.0, RED, mode, 0.5);
        let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
        stroke
            .apply_dab(&mut d, &pixels, DVec2::new(24.0, 24.0), 1.0)
            .unwrap();
        assert!(d.has_active_stroke(), "{mode:?} 取り消されていない");
        let mixed = Rgba8::new(128, 0, 128, 255);
        d.end_stroke(stroke).unwrap();
        assert_eq!(px(&d, l, 23, 23), mixed, "{mode:?} 1 つ目のアイランド");
        assert_eq!(px(&d, l, 483, 483), mixed, "{mode:?} 2 つ目のアイランド");
        assert_eq!(px(&d, l, 28, 28), BLUE, "{mode:?} アイランドの外");
        assert_eq!(px(&d, l, 250, 250), BLUE, "{mode:?} 間");
        d.undo().unwrap();
        assert_eq!(bytes(&d, l), before, "{mode:?}");
    }
}

#[test]
fn distant_islands_in_one_dab_come_out_as_if_painted_separately() {
    // 1 つのダブの画素が離れたアイランドにまたがっても、アイランドごとに別のストロークの最初のダブとして塗ったのと同じ画素（塊ごとの枠は、そのアイランドの下地だけを
    // 読む。荷は最初のダブなので描く色、箱の大きさは一番大きい塊から）
    let island = |x0: i64, y0: i64| -> Vec<BrushPixel> {
        (0..9)
            .flat_map(|dy| {
                (0..9).map(move |dx| BrushPixel {
                    x: x0 + dx,
                    y: y0 + dy,
                    coverage: 0.8,
                })
            })
            .collect()
    };
    let (first, second) = (island(20, 20), island(200, 150));
    let both: Vec<BrushPixel> = first.iter().chain(&second).copied().collect();
    let ground = |x: u32, y: u32| Rgba8::new((x * 7 % 256) as u8, (y * 11 % 256) as u8, 60, 255);
    for mode in [MixMode::Mix, MixMode::Smear] {
        let mut brush = mixing(4.0, RED, mode, 0.3);
        brush.mix.stretch = 0.5;
        let dab = |pixels: &[BrushPixel]| {
            let (mut d, l) = canvas(256, 192, 16, ground);
            let mut s = d.begin_brush_stroke(l, &brush).unwrap();
            s.apply_dab(&mut d, pixels, DVec2::new(24.0, 24.0), 1.0)
                .unwrap();
            d.end_stroke(s).unwrap();
            (d, l)
        };
        let (together, lt) = dab(&both);
        let (a, la) = dab(&first);
        let (b, lb) = dab(&second);
        for p in &both {
            let (x, y) = (p.x as u32, p.y as u32);
            let alone = if first.contains(p) {
                px(&a, la, x, y)
            } else {
                px(&b, lb, x, y)
            };
            assert_eq!(px(&together, lt, x, y), alone, "{mode:?} ({x}, {y})");
        }
    }
}

#[test]
fn a_smearing_dab_over_touching_tiles_reads_the_ground_before_it_paints() {
    // タイルをまたぐ 1 つながりのダブは 1 つの塊（1 つの枠）: 伸ばすの箱の平均は、タイルの境で隣の画素が塗られる前の下地を読む
    // （境の左の列を先に塗って、右の列がそれを読み返すことはない）
    let ground = |x: u32, _y: u32| if x < 16 { BLUE } else { WHITE };
    let (mut d, l) = canvas(32, 16, 16, ground);
    let pixels: Vec<BrushPixel> = (12..20)
        .flat_map(|x| {
            (4..12).map(move |y| BrushPixel {
                x,
                y,
                coverage: 1.0,
            })
        })
        .collect();
    // 量 0: 下の色の平均だけを置く
    let brush = mixing(4.0, RED, MixMode::Smear, 0.0);
    let mut s = d.begin_brush_stroke(l, &brush).unwrap();
    s.apply_dab(&mut d, &pixels, DVec2::new(16.0, 8.0), 1.0)
        .unwrap();
    d.end_stroke(s).unwrap();
    // 境の右 (16, 8): 凍結した下地の 3 × 3 の平均 = (青 3 + 白 6) / 9
    let at = px(&d, l, 16, 8);
    assert!(near(at, Rgba8::new(170, 170, 255, 255), 1), "{at:?}");
    // 境の左 (15, 8): (青 6 + 白 3) / 9
    let at = px(&d, l, 15, 8);
    assert!(near(at, Rgba8::new(85, 85, 255, 255), 1), "{at:?}");
}

// ───────── 混ぜないもの ─────────

#[test]
fn erasers_effects_data_channels_and_masks_do_not_mix() {
    let ground = |x: u32, y: u32| Rgba8::new((x * 3) as u8, (y * 3) as u8, 90, 255);
    let path = line((10.0, 24.0), (54.0, 40.0), 16);
    // 消しゴム
    let eraser = |mix: bool| {
        let (mut d, l) = canvas(64, 64, 16, ground);
        let mut b = Brush::from(BrushSettings {
            erase: true,
            ..settings(9.0, RED)
        });
        if mix {
            b.mix = ColorMix {
                mode: MixMode::Mix,
                paint: 0.2,
                ..ColorMix::default()
            };
        }
        draw(&mut d, l, &b, &path);
        bytes(&d, l)
    };
    assert_eq!(eraser(true), eraser(false));
    // 効果のブラシ（ぼかし・指先）
    for effect in [BrushEffect::BLUR, BrushEffect::SMUDGE] {
        let run = |mix: bool| {
            let (mut d, l) = canvas(64, 64, 16, ground);
            let mut b = Brush {
                effect,
                ..Brush::simple(settings(9.0, RED), yolu_core::PressureSwitches::NONE)
            };
            if mix {
                b.mix = ColorMix {
                    mode: MixMode::Smear,
                    paint: 0.2,
                    ..ColorMix::default()
                };
            }
            draw(&mut d, l, &b, &path);
            bytes(&d, l)
        };
        assert_eq!(run(true), run(false), "{effect:?}");
    }
    // データのチャンネル（Roughness）とマスク
    let roughness = |mix: bool| {
        let mut d = Document::with_tile_size(64, 64, 16).unwrap();
        let l = d.add_layer("L").unwrap();
        fill(&mut d, l, 16, &ground);
        d.set_channel_pixel(l, Channel::Roughness, 5, 5, Rgba8::new(40, 40, 40, 255))
            .unwrap();
        let mut b = Brush::simple(
            settings(9.0, Rgba8::new(200, 200, 200, 255)),
            yolu_core::PressureSwitches::NONE,
        );
        if mix {
            b.mix = ColorMix {
                mode: MixMode::Mix,
                paint: 0.2,
                ..ColorMix::default()
            };
        }
        let mut s = d.begin_brush_stroke_in(l, Channel::Roughness, &b).unwrap();
        for (i, p) in path.iter().enumerate() {
            s.add_sample(&mut d, sample(p.0, p.1, 1.0, i as f64 * 0.01))
                .unwrap();
        }
        d.end_stroke(s).unwrap();
        d.layer(l)
            .unwrap()
            .surface(Channel::Roughness)
            .unwrap()
            .to_canvas_bytes()
    };
    assert_eq!(roughness(true), roughness(false));
    let mask = |mix: bool| {
        let (mut d, l) = canvas(64, 64, 16, ground);
        d.add_layer_mask(l).unwrap();
        let mut b = Brush::simple(settings(9.0, RED), yolu_core::PressureSwitches::NONE);
        if mix {
            b.mix = ColorMix {
                mode: MixMode::Mix,
                paint: 0.2,
                ..ColorMix::default()
            };
        }
        let mut s = d.begin_brush_mask_stroke(l, &b).unwrap();
        for (i, p) in path.iter().enumerate() {
            s.add_sample(&mut d, sample(p.0, p.1, 1.0, i as f64 * 0.01))
                .unwrap();
        }
        d.end_stroke(s).unwrap();
        d.layer(l)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .to_canvas_bytes()
    };
    assert_eq!(mask(true), mask(false));
}

#[test]
fn a_mixing_brush_is_refused_by_the_pixel_by_pixel_entry() {
    let (mut d, l) = canvas(32, 32, 16, |_, _| BLUE);
    let b = mixing(6.0, RED, MixMode::Mix, 0.5);
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    let err = s.apply_pixel(&mut d, 10, 10, 1.0, 1.0).unwrap_err();
    assert!(matches!(err, CoreError::Unsupported(_)), "{err:?}");
    assert!(!d.has_active_stroke(), "断ったらストロークは取り消される");
}

// ───────── 3D の面 ─────────

const W: u32 = 32;
const H: u32 = 16;

fn plate() -> Arc<SurfaceGeometry> {
    let p = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let uv = |x: f32, y: f32| Vec2::new(0.1 + 0.8 * x, 0.1 + 0.8 * y);
    let t = |a, b, c, ua, ub, uc| SurfaceTriangle::new(a, b, c, ua, ub, uc);
    Arc::new(
        SurfaceGeometry::new(
            vec![
                t(
                    p(0.0, 0.0),
                    p(1.0, 0.0),
                    p(1.0, 1.0),
                    uv(0.0, 0.0),
                    uv(1.0, 0.0),
                    uv(1.0, 1.0),
                ),
                t(
                    p(0.0, 0.0),
                    p(1.0, 1.0),
                    p(0.0, 1.0),
                    uv(0.0, 0.0),
                    uv(1.0, 1.0),
                    uv(0.0, 1.0),
                ),
            ],
            1,
            0.000_001,
        )
        .unwrap(),
    )
}

fn front(g: &SurfaceGeometry) -> CameraView {
    let mut cam = OrbitCamera::framing(&g.bounds());
    cam.yaw = 180.0;
    cam.pitch = 0.0;
    cam.view(400.0, 200.0)
}

/// 板の真ん中を 1 回押して離す。
fn surface_dot(brush: &Brush, ground: Rgba8) -> (Document, LayerId) {
    let g = plate();
    let view = front(&g);
    let mut d = Document::with_tile_size(W, H, 8).unwrap();
    let l = d.add_layer("paint").unwrap();
    fill(&mut d, l, 8, &|_, _| ground);
    d.clear_history().unwrap();
    let mut stroke = d.begin_brush_stroke(l, brush).unwrap();
    let at = view.to_screen(Vec3::new(0.5, 0.5, 0.0)).unwrap();
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g,
        view,
        &brush.base,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        at,
        1.0,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    (d, l)
}

#[test]
fn mixing_works_on_a_3d_surface_dab_too() {
    let (plain, lp) = surface_dot(
        &Brush::simple(settings(3.0, RED), yolu_core::PressureSwitches::NONE),
        BLUE,
    );
    let (mixed, lm) = surface_dot(&mixing(3.0, RED, MixMode::Mix, 0.5), BLUE);
    let centre = (W / 2, H / 2);
    assert_eq!(px(&plain, lp, centre.0, centre.1), RED);
    assert_eq!(
        px(&mixed, lm, centre.0, centre.1),
        Rgba8::new(128, 0, 128, 255)
    );
    // 伸ばすも面のダブで効く（ずれは使わず、同じ画素の周りの箱）。一様な下地なら下地の色を拾う
    let (smeared, ls) = surface_dot(&mixing(3.0, RED, MixMode::Smear, 0.0), BLUE);
    assert_eq!(px(&smeared, ls, centre.0, centre.1), BLUE);
    // 混ぜる面でも Undo で戻る
    let (mut d, l) = surface_dot(&mixing(3.0, RED, MixMode::Mix, 0.5), BLUE);
    assert_ne!(px(&d, l, centre.0, centre.1), BLUE);
    d.undo().unwrap();
    assert_eq!(px(&d, l, centre.0, centre.1), BLUE);
}

#[test]
fn a_transparent_3d_surface_gets_the_plain_colour() {
    let (plain, lp) = surface_dot(
        &Brush::simple(settings(3.0, RED), yolu_core::PressureSwitches::NONE),
        Rgba8::TRANSPARENT,
    );
    let (mixed, lm) = surface_dot(&mixing(3.0, RED, MixMode::Mix, 0.5), Rgba8::TRANSPARENT);
    assert_eq!(bytes(&plain, lp), bytes(&mixed, lm));
}

/// 板の上を、世界の x が `from` から `to` まで（y は真ん中）引く。
fn surface_drag(
    brush: &Brush,
    ground: impl Fn(u32, u32) -> Rgba8,
    from: f32,
    to: f32,
) -> (Document, LayerId) {
    let g = plate();
    let view = front(&g);
    let mut d = Document::with_tile_size(W, H, 8).unwrap();
    let l = d.add_layer("paint").unwrap();
    fill(&mut d, l, 8, &ground);
    d.clear_history().unwrap();
    let mut stroke = d.begin_brush_stroke(l, brush).unwrap();
    let a = view.to_screen(Vec3::new(from, 0.5, 0.0)).unwrap();
    let b = view.to_screen(Vec3::new(to, 0.5, 0.0)).unwrap();
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g,
        view,
        &brush.base,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        a,
        1.0,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for i in 1..=16 {
        s.add(&mut d, &mut stroke, a + (b - a) * (i as f32 / 16.0), 1.0)
            .unwrap();
    }
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    (d, l)
}

fn split_16(x: u32, _y: u32) -> Rgba8 {
    if x < 16 {
        BLUE
    } else {
        WHITE
    }
}

#[test]
fn smearing_on_a_3d_surface_drags_the_colour_along_the_stroke() {
    let brush = |mode: MixMode| {
        let mut b = mixing(2.0, WHITE, mode, 0.0);
        b.mix.stretch = 1.0;
        b
    };
    // 量 0（下の色だけ）: 混ぜるは同じ画素を読むので何も変わらない。伸ばすは動きの後ろの青を白い側へ運ぶ
    let (mixed, lm) = surface_drag(&brush(MixMode::Mix), split_16, 0.15, 0.85);
    let (smeared, ls) = surface_drag(&brush(MixMode::Smear), split_16, 0.15, 0.85);
    let row = H / 2;
    let beyond: Vec<Rgba8> = (16..24).map(|x| px(&smeared, ls, x, row)).collect();
    assert!(
        (16..24).all(|x| px(&mixed, lm, x, row) == WHITE),
        "混ぜるは何も変えない"
    );
    assert!(
        beyond.iter().any(|c| c.r < 240),
        "伸ばすは青を白い側へ運ぶ: {beyond:?}"
    );
    // 取り消すと元へ
    let (mut d, l) = surface_drag(&brush(MixMode::Smear), split_16, 0.15, 0.85);
    d.undo().unwrap();
    assert_eq!(px(&d, l, 18, row), WHITE);
}

#[test]
fn a_3d_smear_with_symmetry_falls_back_to_the_pixel_neighbourhood_and_still_paints() {
    // 対称と組むときは、写しごとの読み元が要る指先の読み方ができないので、動きの向きを使わない（同じ画素の周りの箱）。塗れる（予算の拒否は
    // a_3d_mix_or_smear_over_the_stroke_budget_cancels_the_whole_stroke）
    let g = plate();
    let view = front(&g);
    let mut d = Document::with_tile_size(W, H, 8).unwrap();
    let l = d.add_layer("paint").unwrap();
    fill(&mut d, l, 8, &|_, _| BLUE);
    d.clear_history().unwrap();
    let mut b = mixing(3.0, RED, MixMode::Smear, 0.5);
    b.mix.stretch = 0.5;
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let at = view.to_screen(Vec3::new(0.5, 0.5, 0.0)).unwrap();
    let symmetry = yolu_core::geometry::SurfaceSymmetrySetup {
        mirror: Some(yolu_core::geometry::MirrorPlane::from_model(
            Vec3::new(0.5, 0.0, 0.0),
            yolu_core::glam::Quat::IDENTITY,
            yolu_core::geometry::SymmetryAxis::X,
            0.0,
        )),
        radial: None,
        ignore_visibility: false,
    };
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g,
        view,
        &b.base,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        at,
        1.0,
        SurfaceStrokeOptions {
            symmetry: Some(symmetry),
            ..SurfaceStrokeOptions::default()
        },
    )
    .unwrap();
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    assert_ne!(px(&d, l, W / 2, H / 2), BLUE);
}

/// 板の真ん中を 1 回押して離す（ストロークの予算つき。projection は投影の塗りに使ってよいバイトで、None なら文書の予算から）。終わりまで
/// 通ったら Ok、どこかで断られたら、そのエラーと、ストロークを取り消した後の文書。
fn surface_dot_within(
    brush: &Brush,
    mirror: bool,
    budget: Option<u64>,
    projection: Option<u64>,
) -> (
    Document,
    LayerId,
    Result<(), yolu_core::geometry::SurfaceStrokeError>,
) {
    let g = plate();
    let view = front(&g);
    let mut d = Document::with_tile_size(W, H, 8).unwrap();
    let l = d.add_layer("paint").unwrap();
    fill(&mut d, l, 8, &|_, _| BLUE);
    d.clear_history().unwrap();
    if let Some(b) = budget {
        d.set_stroke_budget_bytes(b).unwrap();
    }
    let mut stroke = d.begin_brush_stroke(l, brush).unwrap();
    let at = view.to_screen(Vec3::new(0.5, 0.5, 0.0)).unwrap();
    let symmetry = mirror.then(|| yolu_core::geometry::SurfaceSymmetrySetup {
        mirror: Some(yolu_core::geometry::MirrorPlane::from_model(
            Vec3::new(0.5, 0.0, 0.0),
            yolu_core::glam::Quat::IDENTITY,
            yolu_core::geometry::SymmetryAxis::X,
            0.0,
        )),
        radial: None,
        ignore_visibility: false,
    });
    let result = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g,
        view,
        &brush.base,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        at,
        1.0,
        SurfaceStrokeOptions {
            symmetry,
            projection_memory: projection,
            ..SurfaceStrokeOptions::default()
        },
    )
    .and_then(|mut s| s.finish(&mut d, &mut stroke));
    match &result {
        // 通った: 確定する
        Ok(_) => {
            d.end_stroke(stroke).unwrap();
        }
        // 断られた: 文書が取り消し済みでなければ、呼び手（3D のビュー）がここで取り消す
        Err(_) => d.cancel_stroke(stroke),
    }
    (d, l, result)
}

#[test]
fn a_3d_mix_or_smear_over_the_stroke_budget_cancels_the_whole_stroke() {
    // 3D の面の混ぜる（面のダブの枠）・伸ばす（写像されたダブの見積もりと読み元の図）・対称付きの伸ばす（箱の積分画像つきの枠）は、
    // 投影の画素が入っても、混ぜる見積もりか読み元の図がストロークの予算を超えたらストロークごと取り消す: ストロークは残らず、画素も履歴も元のまま。
    // 同じストロークは、予算が足りれば通る
    use yolu_core::geometry::{SamplingError, SurfaceStrokeError};
    for (name, brush, mirror, why) in [
        (
            "混ぜる",
            mixing(3.0, RED, MixMode::Mix, 0.5),
            false,
            SurfaceStrokeError::Core(CoreError::StrokeBudgetExceeded),
        ),
        (
            "伸ばす",
            mixing(3.0, RED, MixMode::Smear, 0.5),
            false,
            SurfaceStrokeError::Sampling(SamplingError::ChartBudget),
        ),
        (
            "対称付きの伸ばす",
            mixing(3.0, RED, MixMode::Smear, 0.5),
            true,
            SurfaceStrokeError::Core(CoreError::StrokeBudgetExceeded),
        ),
    ] {
        let (open, lo, ok) = surface_dot_within(&brush, mirror, None, None);
        assert_eq!(ok, Ok(()), "{name}");
        assert_ne!(
            px(&open, lo, W / 2, H / 2),
            BLUE,
            "{name}: 予算が足りれば塗れる"
        );
        let (d, l, result) = surface_dot_within(&brush, mirror, Some(200), Some(64 << 20));
        assert_eq!(result, Err(why), "{name}");
        assert!(!d.has_active_stroke(), "{name}");
        assert_eq!(d.undo_count(), 0, "{name}");
        assert!(
            (0..H).all(|y| (0..W).all(|x| px(&d, l, x, y) == BLUE)),
            "{name}: 画素は元のまま"
        );
    }
}

#[test]
fn a_3d_mix_whose_projected_texels_do_not_fit_cancels_the_stroke() {
    // 投影の画素が 1 回の操作のメモリに入らないダブは、混ぜる見積もりの前に断り、2D と同じくストロークごと取り消す（画素も履歴も元のまま）
    for (name, brush, mirror) in [
        ("混ぜる", mixing(3.0, RED, MixMode::Mix, 0.5), false),
        ("伸ばす", mixing(3.0, RED, MixMode::Smear, 0.5), false),
        (
            "対称付きの伸ばす",
            mixing(3.0, RED, MixMode::Smear, 0.5),
            true,
        ),
    ] {
        let (d, l, result) = surface_dot_within(&brush, mirror, Some(200), None);
        assert_eq!(
            result,
            Err(yolu_core::geometry::SurfaceStrokeError::Dab(
                yolu_core::geometry::DabRefusal::MemoryBudget
            )),
            "{name}"
        );
        assert!(!d.has_active_stroke(), "{name}");
        assert_eq!(d.undo_count(), 0, "{name}");
        assert!(
            (0..H).all(|y| (0..W).all(|x| px(&d, l, x, y) == BLUE)),
            "{name}: 画素は元のまま"
        );
    }
}

#[test]
fn a_mapped_smear_dab_counts_its_plan_and_its_pixels_against_the_stroke_budget() {
    // 3D の伸ばす（写像されたダブ）は、呼び手の参照の計画のバイトと、画素ごとの見積もりをストロークの予算に数える
    let pixels = |n: i64| -> Vec<BrushMappedPixel> {
        (0..n)
            .map(|i| {
                let pixel = BrushPixel {
                    x: 8 + i % 40,
                    y: 8 + i / 40,
                    coverage: 1.0,
                };
                BrushMappedPixel::single(pixel, pixel.x, pixel.y)
            })
            .collect()
    };
    let smear = mixing(4.0, RED, MixMode::Smear, 0.5);
    // (画素の数, 計画のバイト, 予算): 超えるものと、収まるもの
    for (n, plan, budget, ok) in [
        (80, 1_000_000, 100_000, false), // 計画が予算を超える
        (80, 0, 100_000, true),
        (1600, 0, 100_000, false), // 画素ごとの見積もり（160 バイト余り × 画素）が予算を超える
    ] {
        let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
        let before = bytes(&d, l);
        d.set_stroke_budget_bytes(budget).unwrap();
        let mut s = d.begin_brush_stroke(l, &smear).unwrap();
        let r = s.apply_mapped_dab(&mut d, &pixels(n), 1.0, plan, None);
        if ok {
            assert!(r.is_ok(), "{n} {plan}: {r:?}");
            assert!(d.has_active_stroke());
            d.end_stroke(s).unwrap();
            assert_ne!(bytes(&d, l), before);
        } else {
            assert_eq!(r, Err(CoreError::StrokeBudgetExceeded), "{n} {plan}");
            assert!(!d.has_active_stroke());
            assert_eq!(bytes(&d, l), before);
            assert_eq!(d.undo_count(), 0);
        }
    }
}

#[test]
fn a_material_stroke_mixes_the_colour_channel_and_paints_the_data_channels_plainly() {
    // 複数チャンネルのストローク（マテリアル）でも、色のチャンネルだけが混ざり、Roughness などは同じダブを普通に塗る。
    // 混ぜるは面のダブ（まとめて渡す）、伸ばすは写像されたダブで、どちらも通る
    for mode in [MixMode::Mix, MixMode::Smear] {
        let g = plate();
        let view = front(&g);
        let mut d = Document::with_tile_size(W, H, 8).unwrap();
        let l = d.add_layer("paint").unwrap();
        fill(&mut d, l, 8, &split_16);
        d.clear_history().unwrap();
        let mut brush = mixing(2.0, WHITE, mode, 0.0);
        brush.mix.stretch = 1.0;
        let channels = [
            ChannelPaint::new(Channel::Color, WHITE),
            ChannelPaint::new(Channel::Roughness, Rgba8::new(200, 200, 200, 255)),
        ];
        let mut stroke = d.begin_material_brush_stroke(l, &channels, &brush).unwrap();
        let a = view.to_screen(Vec3::new(0.15, 0.5, 0.0)).unwrap();
        let b = view.to_screen(Vec3::new(0.85, 0.5, 0.0)).unwrap();
        let mut s = SurfaceStroke::begin_with_options(
            &mut d,
            &mut stroke,
            g,
            view,
            &brush.base,
            Some(&yolu_core::geometry::MaterialSet::one(0)),
            a,
            1.0,
            SurfaceStrokeOptions::default(),
        )
        .unwrap();
        for i in 1..=16 {
            s.add(&mut d, &mut stroke, a + (b - a) * (i as f32 / 16.0), 1.0)
                .unwrap();
        }
        s.finish(&mut d, &mut stroke).unwrap();
        d.end_stroke(stroke).unwrap();
        let row = H / 2;
        // Roughness は普通に塗られている（混ぜず、描く値のまま）
        let rough = d
            .layer(l)
            .unwrap()
            .pixel(Channel::Roughness, 10, row)
            .unwrap();
        assert_eq!(rough, Rgba8::new(200, 200, 200, 255), "{mode:?}");
        // 色は混ざる（量 0 = 下の色だけ: 青い側は青のまま、白い側へは伸ばすだけが青を運ぶ）
        assert_eq!(px(&d, l, 8, row), BLUE, "{mode:?}");
        let beyond = (16..24).any(|x| px(&d, l, x, row).r < 240);
        assert_eq!(beyond, mode == MixMode::Smear, "{mode:?}");
    }
}

#[test]
fn a_2d_material_stroke_takes_the_composite_ground_for_every_channel() {
    // 複数チャンネルのストロークで「全レイヤーから」: 参照元は全部のチャンネルの面が受ける（混ぜが効かないデータのチャンネルも）。
    // 色のチャンネルは下のレイヤーの赤を拾って混ぜ、Roughness は混ぜずに描く値のまま
    let mut d = Document::with_tile_size(64, 64, 16).unwrap();
    let below = d.add_layer("below").unwrap();
    fill(&mut d, below, 16, &|_, _| RED);
    let top = d.add_layer("top").unwrap();
    d.clear_history().unwrap();
    let mut brush = mixing(10.0, Rgba8::new(0, 0, 0, 255), MixMode::Mix, 0.5);
    brush.mix.ground = MixGround::Composite;
    let channels = [
        ChannelPaint::new(Channel::Color, Rgba8::new(0, 0, 0, 255)),
        ChannelPaint::new(Channel::Roughness, Rgba8::new(90, 90, 90, 255)),
    ];
    let mut stroke = d
        .begin_material_brush_stroke(top, &channels, &brush)
        .unwrap();
    stroke.use_composite_clone_source(&mut d).unwrap();
    stroke
        .add_sample(&mut d, sample(32.0, 32.0, 1.0, 0.0))
        .unwrap();
    d.end_stroke(stroke).unwrap();
    assert_eq!(px(&d, top, 32, 32), Rgba8::new(128, 0, 0, 255));
    assert_eq!(
        d.layer(top)
            .unwrap()
            .pixel(Channel::Roughness, 32, 32)
            .unwrap(),
        Rgba8::new(90, 90, 90, 255)
    );
}

// ───────── 下地混色の拡張（重ねる・知覚的・輝度の補正・にじむ強さ・筆の初めの絵の具の量） ─────────

const YELLOW: Rgba8 = Rgba8::new(240, 208, 0, 255);

#[test]
fn layering_reads_the_current_surface_so_an_overlap_moves_towards_the_brush_colour() {
    // 同じ場所へ 2 つの打点が重なる。混ぜるは始める前の絵（青）を読むので、何度重なっても半々。重ねるは今の面を読むので、2 つ目の打点は 1 つ目の
    // 置いた紫を読んで混ぜ、さらに赤へ寄る
    let path = [(20.0, 32.0, 1.0), (26.0, 32.0, 1.0)];
    let run = |mode: MixMode| {
        let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
        let mut b = mixing(8.0, RED, mode, 0.5);
        b.mix.stretch = 0.0;
        draw(&mut d, l, &b, &path);
        (d, l)
    };
    let (mixed, lm) = run(MixMode::Mix);
    let (layered, ll) = run(MixMode::Layer);
    // 2 つの打点に覆われる所（どちらの円にも入る）
    assert_eq!(px(&mixed, lm, 23, 32), Rgba8::new(128, 0, 128, 255));
    let both = px(&layered, ll, 23, 32);
    assert!(both.r > 150 && both.b < 110, "{both:?}");
    // 1 つ目の打点だけに覆われる所（円の左の端）は、どちらも同じ（下の青と半々）
    assert_eq!(px(&layered, ll, 12, 32), px(&mixed, lm, 12, 32));
    assert_eq!(px(&layered, ll, 12, 32), Rgba8::new(128, 0, 128, 255));
    // 下地の外は変わらない
    assert_eq!(px(&layered, ll, 5, 5), BLUE);
}

#[test]
fn a_layering_stroke_cancels_and_undoes_like_a_mixing_stroke() {
    let ground = |x: u32, y: u32| Rgba8::new((x * 3) as u8, (y * 3) as u8, 90, 255);
    type Edit = Box<dyn Fn(&mut Brush)>;
    let variants: Vec<(&str, Edit)> = vec![
        (
            "layer",
            Box::new(|b: &mut Brush| b.mix.mode = MixMode::Layer),
        ),
        (
            "perceptual",
            Box::new(|b: &mut Brush| b.mix.space = MixSpace::Perceptual),
        ),
        (
            "fixed blur",
            Box::new(|b: &mut Brush| {
                b.mix.mode = MixMode::Smear;
                b.mix.blur = MixBlur::Fixed(3.0);
            }),
        ),
        ("load", Box::new(|b: &mut Brush| b.mix.load = 0.3)),
    ];
    for (name, apply) in variants {
        let (mut d, l) = canvas(64, 64, 16, ground);
        let before = bytes(&d, l);
        let mut b = mixing(9.0, RED, MixMode::Mix, 0.4);
        apply(&mut b);
        let mut s = d.begin_brush_stroke(l, &b).unwrap();
        for (i, p) in line((8.0, 20.0), (56.0, 44.0), 12).iter().enumerate() {
            s.add_sample(&mut d, sample(p.0, p.1, 1.0, i as f64 * 0.01))
                .unwrap();
        }
        assert_ne!(bytes(&d, l), before, "{name}: 途中は描けている");
        d.cancel_stroke(s);
        assert_eq!(bytes(&d, l), before, "{name}");
        draw(&mut d, l, &b, &line((8.0, 20.0), (56.0, 44.0), 12));
        let after = bytes(&d, l);
        assert_ne!(after, before, "{name}");
        d.undo().unwrap();
        assert_eq!(bytes(&d, l), before, "{name}");
        d.redo().unwrap();
        assert_eq!(bytes(&d, l), after, "{name}");
    }
}

#[test]
fn the_perceptual_space_turns_blue_and_yellow_green_where_the_standard_mix_is_grey() {
    let run = |space: MixSpace| {
        let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
        let mut b = mixing(10.0, YELLOW, MixMode::Mix, 0.6);
        b.mix.space = space;
        dot(&mut d, l, &b, 32.0, 32.0, 1.0);
        px(&d, l, 32, 32)
    };
    let standard = run(MixSpace::Standard);
    // 標準は sRGB の補間: 青 0.4 と黄 0.6 を値のまま混ぜる
    assert_eq!(standard, Rgba8::new(144, 125, 102, 255));
    let perceptual = run(MixSpace::Perceptual);
    assert!(
        perceptual.g > perceptual.r && perceptual.g > perceptual.b,
        "{perceptual:?}"
    );
    assert!(perceptual.r < 60, "{perceptual:?}");
    // 絵の具量 1 は描く色そのもの、0 は下の色そのまま（空間によらない）
    for (paint, expect) in [(1.0, YELLOW), (0.0, BLUE)] {
        let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
        let mut b = mixing(10.0, YELLOW, MixMode::Mix, paint);
        b.mix.space = MixSpace::Perceptual;
        dot(&mut d, l, &b, 32.0, 32.0, 1.0);
        assert_eq!(px(&d, l, 32, 32), expect, "paint {paint}");
    }
}

#[test]
fn a_stronger_luminance_correction_makes_the_perceptual_mix_lighter() {
    let light = |level: u8| {
        let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
        let mut b = mixing(10.0, YELLOW, MixMode::Mix, 0.6);
        b.mix.space = MixSpace::Perceptual;
        b.mix.correction = level;
        b.mix.stretch = 0.0;
        dot(&mut d, l, &b, 32.0, 32.0, 1.0);
        let c = px(&d, l, 32, 32);
        0.2126 * f64::from(c.r) + 0.7152 * f64::from(c.g) + 0.0722 * f64::from(c.b)
    };
    let steps: Vec<f64> = (0..=4).map(light).collect();
    assert!(steps.windows(2).all(|w| w[1] > w[0]), "{steps:?}");
    // 標準の空間では、輝度の補正は効かない
    let std_light = |level: u8| {
        let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
        let mut b = mixing(10.0, YELLOW, MixMode::Mix, 0.6);
        b.mix.correction = level;
        dot(&mut d, l, &b, 32.0, 32.0, 1.0);
        px(&d, l, 32, 32)
    };
    assert_eq!(std_light(0), std_light(4));
}

#[test]
fn the_perceptual_space_carries_the_load_in_the_same_space_and_keeps_the_carry_from_darkening() {
    // 荷の拾いと引きずりも知覚的な空間で混ぜる。白い下地の上を引くと、黄は白と混ざり続けても暗くならない（拾いに輝度の補正は掛けない）
    let path = line((8.0, 32.0), (56.0, 32.0), 60);
    let (mut d, l) = canvas(64, 64, 16, |_, _| WHITE);
    let mut b = mixing(6.0, YELLOW, MixMode::Mix, 0.6);
    b.mix.space = MixSpace::Perceptual;
    b.mix.correction = 0;
    b.mix.stretch = 0.9;
    draw(&mut d, l, &b, &path);
    let (early, late) = (px(&d, l, 16, 32), px(&d, l, 52, 32));
    let lum =
        |c: Rgba8| 0.2126 * f64::from(c.r) + 0.7152 * f64::from(c.g) + 0.0722 * f64::from(c.b);
    assert!(lum(late) > lum(early) * 0.8, "{early:?} {late:?}");
}

#[test]
fn the_blur_strength_sets_the_smear_box_and_automatic_follows_the_brush_size() {
    // 左が青・右が白の境の上で、伸ばす。箱が大きいほど境の色が遠くまで混ざる
    let across = line((10.0, 32.0), (54.0, 32.0), 44);
    let run = |radius: f64, blur: MixBlur| {
        let (mut d, l) = canvas(64, 64, 16, split);
        let mut b = mixing(radius, WHITE, MixMode::Smear, 0.4);
        b.mix.stretch = 0.3;
        b.mix.blur = blur;
        draw(&mut d, l, &b, &across);
        bytes(&d, l)
    };
    let small = run(8.0, MixBlur::Fixed(1.0));
    let large = run(8.0, MixBlur::Fixed(8.0));
    assert_ne!(small, large);
    // 自動設定は半径の 1 割（半径 20 なら 2 画素）= 同じ半径の固定値
    assert_eq!(run(20.0, MixBlur::Auto), run(20.0, MixBlur::Fixed(2.0)));
    assert_ne!(run(20.0, MixBlur::Auto), run(20.0, MixBlur::Fixed(7.0)));
    // 半径が小さければ 1 画素（下限）
    assert_eq!(run(4.0, MixBlur::Auto), run(4.0, MixBlur::Fixed(1.0)));
    // 大きさに連動して広い形は半径の 2 割（半径 20 なら 4 画素、半径 60 なら上限の 8 画素、半径 2 なら下限の 1 画素）
    assert_eq!(run(20.0, MixBlur::Linked), run(20.0, MixBlur::Fixed(4.0)));
    assert_ne!(run(20.0, MixBlur::Linked), run(20.0, MixBlur::Auto));
    assert_eq!(run(60.0, MixBlur::Linked), run(60.0, MixBlur::Fixed(8.0)));
    assert_eq!(run(2.0, MixBlur::Linked), run(2.0, MixBlur::Fixed(1.0)));
    // 混ぜる・重ねるには箱が無いので、固定値は結果を変えない
    for mode in [MixMode::Mix, MixMode::Layer] {
        let with = |blur: MixBlur| {
            let (mut d, l) = canvas(64, 64, 16, split);
            let mut b = mixing(8.0, RED, mode, 0.5);
            b.mix.blur = blur;
            draw(&mut d, l, &b, &across);
            bytes(&d, l)
        };
        assert_eq!(with(MixBlur::Auto), with(MixBlur::Fixed(8.0)), "{mode:?}");
    }
    // 固定値の箱が大きくても、タイルの境を越えて読む（キャンバスの端でも読み出しがはみ出さない）
    let (mut d, l) = canvas(64, 64, 16, split);
    let mut b = mixing(6.0, WHITE, MixMode::Smear, 0.5);
    b.mix.blur = MixBlur::Fixed(8.0);
    draw(&mut d, l, &b, &line((2.0, 2.0), (61.0, 61.0), 40));
}

#[test]
fn the_initial_load_makes_the_first_dab_pale_and_the_next_dabs_bring_the_brush_colour() {
    // 筆の初めの絵の具の量 0.25（絵の具量 1・色延び 0）: 最初の打点の荷は、描く色 1/4 と下の青 3/4。次の打点からは描く色だけ
    let path = [(16.0, 32.0, 1.0), (44.0, 32.0, 1.0)];
    let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
    let mut b = mixing(6.0, RED, MixMode::Mix, 1.0);
    b.mix.stretch = 0.0;
    b.mix.load = 0.25;
    draw(&mut d, l, &b, &path);
    // 最初の打点だけに覆われる所（円の左の端）が薄い。それより先は、あとの打点が描く色で塗り直す
    let first = px(&d, l, 10, 32);
    assert!(near(first, Rgba8::new(64, 0, 191, 255), 1), "{first:?}");
    assert_eq!(px(&d, l, 16, 32), RED);
    assert_eq!(px(&d, l, 44, 32), RED);
    // 既定（1）は描く色そのもの。初めの絵の具の量が効くのは最初の打点だけ
    let (mut plain, lp) = canvas(64, 64, 16, |_, _| BLUE);
    let mut b = mixing(6.0, RED, MixMode::Mix, 1.0);
    b.mix.stretch = 0.0;
    draw(&mut plain, lp, &b, &path);
    assert_eq!(px(&plain, lp, 10, 32), RED);
    // 描く色の荷は、筆圧と混ぜ方（重ねる）でも最初の打点だけ薄い
    let (mut layered, ll) = canvas(64, 64, 16, |_, _| BLUE);
    let mut b = mixing(6.0, RED, MixMode::Layer, 1.0);
    b.mix.stretch = 0.0;
    b.mix.load = 0.25;
    draw(&mut layered, ll, &b, &path);
    assert!(near(px(&layered, ll, 10, 32), first, 1));
    assert_eq!(px(&layered, ll, 44, 32), RED);
}

#[test]
fn the_new_mix_values_are_checked_before_a_stroke_starts() {
    type Edit = Box<dyn Fn(&mut ColorMix)>;
    let bad: Vec<(&str, Edit)> = vec![
        ("load low", Box::new(|m: &mut ColorMix| m.load = -0.1)),
        ("load high", Box::new(|m: &mut ColorMix| m.load = 1.1)),
        ("load nan", Box::new(|m: &mut ColorMix| m.load = f64::NAN)),
        ("correction", Box::new(|m: &mut ColorMix| m.correction = 5)),
        (
            "blur low",
            Box::new(|m: &mut ColorMix| m.blur = MixBlur::Fixed(-1.0)),
        ),
        (
            "blur high",
            Box::new(|m: &mut ColorMix| m.blur = MixBlur::Fixed(9.0)),
        ),
        (
            "blur nan",
            Box::new(|m: &mut ColorMix| m.blur = MixBlur::Fixed(f64::NAN)),
        ),
    ];
    for (name, apply) in bad {
        let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
        let mut b = mixing(8.0, RED, MixMode::Smear, 0.5);
        apply(&mut b.mix);
        assert!(d.begin_brush_stroke(l, &b).is_err(), "{name}");
        assert!(!d.has_active_stroke(), "{name}");
    }
    // 端の値は通る
    let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
    let mut b = mixing(8.0, RED, MixMode::Smear, 0.5);
    b.mix.load = 0.0;
    b.mix.correction = 4;
    b.mix.blur = MixBlur::Fixed(8.0);
    dot(&mut d, l, &b, 32.0, 32.0, 1.0);
}

/// FNV-1a（64 ビット）。画素の列を 1 つの値にして固定する。
fn fnv(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// 標準の混ぜる・伸ばすの線（量・濃さ・色延び・硬さを変え、2D と縦の対称）の画素のハッシュ。新しい値（混色モード・輝度の補正・にじむ強さ・
/// 初めの量）には触らない（既定のまま）。
fn standard_mix_line_hashes() -> Vec<(String, u64)> {
    let w = 64u32;
    let ground = |x: u32, y: u32| Rgba8::new((x * 3) as u8, (y * 3) as u8, 90, 255);
    let path = line((8.0, 20.0), (56.0, 44.0), 24);
    let values = [
        (0.2, 1.0, 0.0, 1.0),
        (0.4, 1.0, 0.5, 1.0),
        (0.7, 0.6, 0.9, 1.0),
        (0.5, 0.8, 0.3, 0.5),
    ];
    let mut out = Vec::new();
    for mode in [MixMode::Mix, MixMode::Smear] {
        for (paint, density, stretch, hardness) in values {
            for mirror in [false, true] {
                let (mut d, l) = canvas(w, w, 16, ground);
                let mut b = mixing(8.0, RED, mode, paint);
                b.mix.density = density;
                b.mix.stretch = stretch;
                b.base.hardness = hardness;
                if mirror {
                    b.symmetry = CanvasSymmetry::new(
                        SymmetryMode::Vertical,
                        DVec2::new(f64::from(w) / 2.0, 32.0),
                        2,
                    )
                    .unwrap();
                }
                draw(&mut d, l, &b, &path);
                out.push((
                    format!("{mode:?} {paint} {density} {stretch} {hardness} {mirror}"),
                    fnv(&bytes(&d, l)),
                ));
            }
        }
    }
    out
}

#[test]
fn the_standard_mix_and_smear_lines_keep_the_bytes_from_before_the_new_values() {
    // 値は、新しい値（混色モード・輝度の補正・にじむ強さ・初めの量）を足す前のコードで同じ線を描いて求めた
    const BEFORE: [u64; 16] = [
        0x52802ae685125267,
        0x565e599f6cab3700,
        0x891f85001cede6fc,
        0xee250713d485ad09,
        0x8d9eb1ea654da8ca,
        0x58303de8e1c72f5d,
        0xbf0dda25bfd4cf11,
        0x1cfde0fed7adfb8d,
        0xcee7dabe36c32e7b,
        0x6534ffff4a8ec675,
        0x50fc23539037e8cf,
        0x8ddfc939fdcdb8b5,
        0xc0fb4485b264ec4e,
        0xfbfa85b4f5ba4ab5,
        0xdddd9da8c386d140,
        0xa3347882dddcc727,
    ];
    let now = standard_mix_line_hashes();
    let all = now
        .iter()
        .map(|(name, h)| format!("{name}: {h:#018x}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(now.len(), BEFORE.len(), "{all}");
    for ((name, h), want) in now.iter().zip(BEFORE) {
        assert_eq!(*h, want, "{name}\n{all}");
    }
    // 混ぜ方が切のブラシは、新しい値が何であっても今までと同じ（効かない）
    let path = line((8.0, 20.0), (56.0, 44.0), 24);
    let ground = |x: u32, y: u32| Rgba8::new((x * 3) as u8, (y * 3) as u8, 90, 255);
    let off = |new: bool| {
        let (mut d, l) = canvas(64, 64, 16, ground);
        let mut b = Brush::simple(settings(8.0, RED), yolu_core::PressureSwitches::NONE);
        if new {
            b.mix.space = MixSpace::Perceptual;
            b.mix.correction = 3;
            b.mix.blur = MixBlur::Fixed(5.0);
            b.mix.load = 0.2;
        }
        draw(&mut d, l, &b, &path);
        bytes(&d, l)
    };
    assert_eq!(off(false), off(true));
}

#[test]
fn the_new_mixes_work_on_a_3d_surface_dab_and_symmetry_too() {
    let centre = (W / 2, H / 2);
    // 面のダブ: 重ねる・知覚的・初めの絵の具の量
    let (layered, ll) = surface_dot(&mixing(3.0, RED, MixMode::Layer, 0.5), BLUE);
    assert_eq!(
        px(&layered, ll, centre.0, centre.1),
        Rgba8::new(128, 0, 128, 255)
    );
    let mut perceptual = mixing(3.0, YELLOW, MixMode::Mix, 0.6);
    perceptual.mix.space = MixSpace::Perceptual;
    let (p, lp) = surface_dot(&perceptual, BLUE);
    let c = px(&p, lp, centre.0, centre.1);
    assert!(c.g > c.r && c.g > c.b, "{c:?}");
    let mut loaded = mixing(3.0, RED, MixMode::Mix, 1.0);
    loaded.mix.load = 0.5;
    let (q, lq) = surface_dot(&loaded, BLUE);
    assert!(near(
        px(&q, lq, centre.0, centre.1),
        Rgba8::new(128, 0, 128, 255),
        1
    ));
    // 面の伸ばしでも、固定の箱を持てる（動きの向きの読み方）
    let mut b = mixing(2.0, WHITE, MixMode::Smear, 0.0);
    b.mix.stretch = 1.0;
    b.mix.blur = MixBlur::Fixed(4.0);
    let (smeared, ls) = surface_drag(&b, split_16, 0.15, 0.85);
    let row = H / 2;
    assert!((16..24).any(|x| px(&smeared, ls, x, row).r < 240));
    // 左右対称（2D）: 重ねる・知覚的でも鏡像
    let mirrored = |build: &dyn Fn(&mut Brush)| {
        let w = 64u32;
        let (mut d, l) = canvas(w, 32, 16, mirrored_ground(w));
        let mut b = mixing(6.0, YELLOW, MixMode::Mix, 0.5);
        build(&mut b);
        b.symmetry =
            CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(w as f64 / 2.0, 16.0), 2)
                .unwrap();
        draw(&mut d, l, &b, &line((6.0, 14.0), (24.0, 18.0), 24));
        is_mirror_image(&d, l)
    };
    assert!(mirrored(&|b| b.mix.mode = MixMode::Layer));
    assert!(mirrored(&|b| b.mix.space = MixSpace::Perceptual));
}

#[test]
fn the_new_variants_count_the_read_pixels_against_the_stroke_budget_like_the_old_ones() {
    // 重ねる・知覚的・初めの絵の具の量は、混ぜると同じ大きさの枠を読む。伸ばすは箱の積分画像の分が要り、固定の箱が大きいほど枠も広がる。
    // 枠が予算に入らなければ、どの変種も最初のダブでストロークごと断る
    let need = |brush: &Brush| {
        let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
        let mut s = d.begin_brush_stroke(l, brush).unwrap();
        s.add_sample(&mut d, sample(32.0, 32.0, 1.0, 0.0)).unwrap();
        let held = d.active_stroke_stats().unwrap().rollback_bytes;
        d.cancel_stroke(s);
        held
    };
    let mix = mixing(10.0, RED, MixMode::Mix, 0.5);
    let mut layer = mix.clone();
    layer.mix.mode = MixMode::Layer;
    let mut perceptual = mix.clone();
    perceptual.mix.space = MixSpace::Perceptual;
    let mut loaded = mix.clone();
    loaded.mix.load = 0.3;
    assert_eq!(need(&layer), need(&mix));
    assert_eq!(need(&perceptual), need(&mix));
    assert_eq!(need(&loaded), need(&mix));
    let mut large = mixing(10.0, RED, MixMode::Smear, 0.5);
    large.mix.blur = MixBlur::Fixed(8.0);
    // 予算が枠に入らなければ、どの変種もストロークごと断り、レイヤーは元のまま
    for (name, brush) in [
        ("layer", &layer),
        ("perceptual", &perceptual),
        ("load", &loaded),
        ("large blur", &large),
    ] {
        let (mut d, l) = canvas(64, 64, 16, |_, _| BLUE);
        let before = bytes(&d, l);
        d.set_stroke_budget_bytes(1000).unwrap();
        let mut s = d.begin_brush_stroke(l, brush).unwrap();
        let err = s
            .add_sample(&mut d, sample(32.0, 32.0, 1.0, 0.0))
            .unwrap_err();
        assert_eq!(err, CoreError::StrokeBudgetExceeded, "{name}");
        assert!(!d.has_active_stroke(), "{name}");
        assert_eq!(bytes(&d, l), before, "{name}");
        assert_eq!(d.undo_count(), 0, "{name}");
        d.set_stroke_budget_bytes(64 << 20).unwrap();
        dot(&mut d, l, brush, 32.0, 32.0, 1.0);
        assert_ne!(bytes(&d, l), before, "{name}: 予算を戻せば同じ入力が通る");
    }
}

#[test]
fn a_perceptual_mix_obeys_the_selection_and_the_transparency_lock_like_the_standard_one() {
    // 選択範囲: 全選択の画素は知覚的に混ざった色、半分選ばれた画素は描く前の画素から半分だけそこへ寄り、選択外は変わらない
    let (mut d, l) = canvas(64, 16, 16, |_, _| BLUE);
    d.set_selection(Some(banded_selection(&d))).unwrap();
    d.clear_history().unwrap();
    let before = bytes(&d, l);
    let mut b = mixing(30.0, YELLOW, MixMode::Mix, 0.6);
    b.mix.space = MixSpace::Perceptual;
    dot(&mut d, l, &b, 32.0, 8.0, 1.0);
    let mixed = px(&d, l, 8, 8);
    assert!(mixed.g > mixed.r && mixed.g > mixed.b, "{mixed:?}");
    assert_eq!(
        px(&d, l, 24, 8),
        yolu_core::blend::fade(BLUE, mixed, 128.0 / 255.0)
    );
    for x in 32..64 {
        assert_eq!(px(&d, l, x, 8), BLUE, "選択外 x={x}");
    }
    d.undo().unwrap();
    assert_eq!(bytes(&d, l), before);
    // 透明部分のロック: 透明な画素は RGB もアルファも守り、不透明な画素は知覚的に混ざる
    let hidden = Rgba8::new(9, 8, 7, 0);
    let ground = move |x: u32, _y: u32| if x < 8 { BLUE } else { hidden };
    let (mut d, l) = canvas(16, 16, 16, ground);
    d.set_layer_locks(l, LayerLocks::TRANSPARENCY).unwrap();
    d.clear_history().unwrap();
    let mut b = mixing(6.0, YELLOW, MixMode::Mix, 0.6);
    b.mix.space = MixSpace::Perceptual;
    dot(&mut d, l, &b, 8.0, 8.0, 1.0);
    let c = px(&d, l, 5, 8);
    assert!(c.a == 255 && c.g > c.r, "{c:?}");
    for x in 8..14 {
        assert_eq!(px(&d, l, x, 8), hidden, "透明な画素 x={x}");
    }
}
