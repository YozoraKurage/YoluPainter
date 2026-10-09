//! 行の核（[`super`]）が、道（スカラー・SSE4.1・AVX2・NEON）によらず、画素ごとの式（`apply_at`）と同じバイトを出すこと。ブラシ・レイヤーの中身・
//! タイルの大きさ・点の列を乱数で振って、同じ入力を画素ごとの式（行の核を使わない）と この CPU が持つ全部の道の行の核で描き、レイヤーの全バイトと
//! ダブの数・変わったかが一致することを確かめる。

use super::*;
use crate::math::simd::forced;
use crate::{
    builtin_tip, Channel, ColorMix, Document, DualBrush, DualBrushMode, LayerLocks, MixMode,
    PaperTexture, SelectionMask, TextureMode,
};
use glam::DVec2;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }
    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
    fn channel(&mut self) -> u8 {
        let r = self.next();
        match r & 7 {
            0 => 0,
            1 => 255,
            _ => (r >> 8) as u8,
        }
    }
    fn alpha(&mut self) -> u8 {
        let r = self.next();
        match r & 7 {
            0 | 1 => 0,
            2 | 3 => 255,
            _ => (r >> 8) as u8,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Ground {
    Empty,
    Uniform,
    Noise,
    Opaque,
}

/// 1 つの事例の入力。
struct Case {
    brush: Brush,
    size: (u32, u32),
    tile: u32,
    ground: Ground,
    points: Vec<(f64, f64, f64)>,
    selection: bool,
    locked: bool,
    parallel: bool,
    /// ストロークの予算を小さくして、途中で断られるようにする。
    tight_budget: bool,
    seed: u64,
}

fn random_case(seed: u64) -> Case {
    let mut r = Rng(seed.wrapping_mul(0x2545F4914F6CDD1D) | 1);
    // 300 は行が STACK_ROW（256）より長いタイル（置き場がヒープの道）
    let tile = r.pick(&[8u32, 16, 24, 32, 37, 64, 300]);
    let size = (r.pick(&[40u32, 77, 96, 330]), r.pick(&[33u32, 64, 90]));
    let radius = r.pick(&[0.6, 1.0, 2.5, 5.0, 9.0, 17.0, 31.0]);
    let mut b = Brush::from(BrushSettings {
        radius,
        hardness: r.pick(&[0.0, 0.25, 0.8, 1.0]),
        spacing: r.pick(&[0.05, 0.15, 0.5, 1.0]),
        opacity: r.pick(&[1.0, 0.6, 0.3]),
        flow: r.pick(&[1.0, 0.5, 0.1]),
        color: Rgba8::new(
            r.channel(),
            r.channel(),
            r.channel(),
            r.pick(&[255, 255, 200, 90]),
        ),
        pressure_size: r.chance(0.5),
        pressure_opacity: r.chance(0.5),
        pressure_flow: r.chance(0.5),
        erase: r.chance(0.2),
        anti_alias: AntiAlias::None,
    });
    b.seed = r.next() as i32;
    if r.chance(0.4) {
        b.tip.angle = r.pick(&[0.0, 30.0, 90.0, 200.0]);
        b.tip.roundness = r.pick(&[1.0, 0.5, 0.2]);
        b.tip.follow_direction = r.chance(0.5);
    }
    if r.chance(0.35) {
        b.tip.image = builtin_tip(r.pick(&[
            "grain",
            "charcoal",
            "bristles",
            "dots",
            "rim",
            "rounded-square",
        ]));
        b.tip.flip_x = r.chance(0.3);
        b.tip.flip_y = r.chance(0.3);
    }
    if r.chance(0.3) {
        b.jitter.size = 0.3;
        b.jitter.angle = 0.4;
        b.jitter.roundness = 0.3;
        b.jitter.scatter = 0.3;
        b.jitter.opacity = 0.2;
        b.jitter.flow = 0.2;
        b.jitter.count = r.pick(&[1u32, 2, 3]);
    }
    if r.chance(0.3) {
        b.texture = Some(PaperTexture {
            scale: r.pick(&[1.0, 2.0, 0.7]),
            mode: if r.chance(0.8) {
                TextureMode::Multiply
            } else {
                TextureMode::Darken
            },
            ..PaperTexture::new(builtin_tip("grain").unwrap(), r.pick(&[0.3, 0.6, 1.0]))
        });
    }
    if r.chance(0.25) {
        b.dual = Some(DualBrush {
            radius: (radius / 3.0).max(0.6),
            spacing: 0.3,
            scatter: 0.4,
            count: 2,
            hardness: 0.4,
            angle: r.pick(&[0.0, 30.0, 100.0]),
            roundness: r.pick(&[1.0, 0.5]),
            tip: if r.chance(0.4) {
                builtin_tip(r.pick(&["grain", "bristles", "rim"]))
            } else {
                None
            },
            mode: r.pick(&[
                DualBrushMode::Multiply,
                DualBrushMode::Darken,
                DualBrushMode::Overlay,
            ]),
        });
    }
    // 効果のブラシ（塗らず、読み元の枠から読んだ色を混ぜる）。消しゴムとは組まない
    if r.chance(0.3) {
        b.base.erase = false;
        b.effect = match r.below(3) {
            0 => BrushEffect::Blur {
                radius: r.pick(&[1u32, 2, 3, 5]),
            },
            1 => BrushEffect::Smudge {
                strength: r.pick(&[0.2, 0.5, 1.0]),
            },
            _ => BrushEffect::Clone {
                offset: DVec2::new(r.pick(&[-9.0, 0.0, 6.5, 14.25]), r.pick(&[-7.0, 3.0, 0.5])),
            },
        };
    }
    // 画素ごとの色: ダブごとの色の変化、色の混ぜ（混ぜる・伸ばす）。消しゴム・効果のブラシとは組まない
    if b.effect.is_paint() && !b.base.erase {
        if r.chance(0.2) {
            b.color.hue = 0.3;
            b.color.brightness = 0.2;
            b.color.per_tip = true;
        }
        if r.chance(0.3) {
            b.mix = ColorMix {
                mode: if r.chance(0.5) {
                    MixMode::Mix
                } else {
                    MixMode::Smear
                },
                paint: r.pick(&[0.0, 0.3, 0.5, 1.0]),
                density: r.pick(&[1.0, 0.7]),
                stretch: r.pick(&[0.0, 0.5, 1.0]),
                ..ColorMix::default()
            };
        }
    }
    if r.chance(0.15) {
        b.assist.taper_in = 5.0;
        b.assist.taper_out = 8.0;
    }
    let n = 3 + r.below(8) as usize;
    let mut points = Vec::new();
    for _ in 0..n {
        points.push((
            r.unit() * (size.0 as f64 + 20.0) - 10.0,
            r.unit() * (size.1 as f64 + 20.0) - 10.0,
            r.unit(),
        ));
    }
    let mut case = Case {
        brush: b,
        size,
        tile,
        ground: r.pick(&[
            Ground::Empty,
            Ground::Uniform,
            Ground::Noise,
            Ground::Opaque,
        ]),
        points,
        selection: r.chance(0.2),
        locked: r.chance(0.15),
        parallel: r.chance(0.25),
        tight_budget: r.chance(0.1),
        seed,
    };
    // アンチエイリアスの段は最後に引く（前の項目の乱数の列を変えない）
    case.brush.base.anti_alias = r.pick(&AntiAlias::ALL);
    case
}

/// レイヤーの全バイト、ダブの数、画素が変わったか、取り消して元に戻ったか。
#[derive(Debug, PartialEq)]
struct Outcome {
    pixels: Vec<u8>,
    stamps: u64,
    changed: bool,
    restored: bool,
    /// 予算などで断られたときの理由（断られたストロークは取り消されて、レイヤーは元のまま）。
    refused: Option<String>,
    /// 同じ点の列を描いて取り消したあとに、レイヤーが元のままか。
    cancel_restored: bool,
}

fn run(case: &Case) -> Option<Outcome> {
    let mut doc = Document::with_tile_size(case.size.0, case.size.1, case.tile).unwrap();
    let layer = doc.add_layer("a").unwrap();
    let ts = case.tile as usize;
    let mut rng = Rng(case.seed ^ 0xABCDEF);
    for ty in 0..case.size.1.div_ceil(case.tile) {
        for tx in 0..case.size.0.div_ceil(case.tile) {
            let mut bytes = vec![0u8; ts * ts * 4];
            for (i, p) in bytes.chunks_exact_mut(4).enumerate() {
                let (x, y) = (
                    tx * case.tile + (i % ts) as u32,
                    ty * case.tile + (i / ts) as u32,
                );
                if x >= case.size.0 || y >= case.size.1 {
                    continue;
                }
                let px = match case.ground {
                    Ground::Empty => [0, 0, 0, 0],
                    Ground::Uniform => [40, 120, 200, 255],
                    Ground::Noise => [rng.channel(), rng.channel(), rng.channel(), rng.alpha()],
                    Ground::Opaque => [rng.channel(), rng.channel(), rng.channel(), 255],
                };
                p.copy_from_slice(&px);
            }
            if !matches!(case.ground, Ground::Empty) {
                doc.import_tile(layer, Channel::Color, TileCoord::new(tx, ty), &bytes)
                    .unwrap();
            }
        }
    }
    doc.clear_history().unwrap();
    if case.selection {
        let (w, h) = (case.size.0 as f64, case.size.1 as f64);
        let mask = SelectionMask::ellipse(&doc, w / 2.0, h / 2.0, w * 0.4, h * 0.35).unwrap();
        doc.set_selection(Some(mask)).unwrap();
    }
    if case.locked {
        doc.set_layer_locks(layer, LayerLocks::TRANSPARENCY)
            .unwrap();
    }
    doc.set_stroke_budget_bytes(if case.tight_budget { 40_000 } else { 256 << 20 })
        .unwrap();
    let read = |doc: &Document| {
        doc.layers()[0].surface(Channel::Color).map_or_else(
            || vec![0; (case.size.0 * case.size.1 * 4) as usize],
            |s| s.to_canvas_bytes(),
        )
    };
    let before = read(&doc);
    let old = set_parallel_dab_pixels(if case.parallel {
        1
    } else {
        PARALLEL_DAB_PIXELS
    });
    // 同じ点の列を 1 つのストロークとして描く。断られたら（予算など）理由を返す。断られたストロークは文書が取り消している
    let draw = |doc: &mut Document, finish: bool| -> Result<Option<crate::StrokeResult>, String> {
        let mut stroke = doc
            .begin_brush_stroke(layer, &case.brush)
            .map_err(|e| format!("{e:?}"))?;
        let mut time = 0.0;
        for &(x, y, p) in &case.points {
            time += 0.01;
            let sample = BrushSample::new(x, y, p, time, DVec2::ZERO).unwrap();
            stroke
                .add_sample(doc, sample)
                .map_err(|e| format!("{e:?}"))?;
        }
        if finish {
            Ok(Some(doc.end_stroke(stroke).map_err(|e| format!("{e:?}"))?))
        } else {
            doc.cancel_stroke(stroke);
            Ok(None)
        }
    };
    // 透明部分のロックのレイヤーへの消しゴムなど、始められない組はどの道でも同じく断られる
    if doc.begin_brush_stroke(layer, &case.brush).is_err() {
        set_parallel_dab_pixels(old);
        return None;
    }
    doc.cancel_active_stroke();
    let outcome = match draw(&mut doc, true) {
        Ok(result) => {
            let result = result.expect("確定した");
            let pixels = read(&doc);
            if result.changed {
                doc.undo().unwrap();
            }
            let restored = read(&doc) == before;
            // 取り消しも同じく元へ戻る
            let _ = draw(&mut doc, false);
            let cancel_restored = read(&doc) == before;
            Outcome {
                pixels,
                stamps: result.stamps,
                changed: result.changed,
                restored,
                refused: None,
                cancel_restored,
            }
        }
        Err(reason) => {
            let pixels = read(&doc);
            Outcome {
                restored: pixels == before,
                cancel_restored: true,
                pixels,
                stamps: 0,
                changed: false,
                refused: Some(reason),
            }
        }
    };
    set_parallel_dab_pixels(old);
    Some(outcome)
}

/// 行の核を使わず画素ごとの式で描く間だけ f を走らせる。
fn per_pixel<R>(f: impl FnOnce() -> R) -> R {
    use std::sync::atomic::Ordering;
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            PER_PIXEL.store(false, Ordering::Relaxed);
        }
    }
    PER_PIXEL.store(true, Ordering::Relaxed);
    let _reset = Reset;
    f()
}

#[test]
fn every_level_paints_the_same_bytes_as_the_per_pixel_formula() {
    let levels = forced::supported();
    let mut checked = 0;
    let mut refusals = 0;
    for seed in 0..300 {
        let case = random_case(seed);
        let reference = forced::with_level(Level::Scalar, || per_pixel(|| run(&case)));
        let Some(reference) = reference else {
            for &level in &levels {
                assert!(
                    forced::with_level(level, || run(&case)).is_none(),
                    "{level:?} seed {seed}"
                );
            }
            continue;
        };
        assert!(
            reference.restored && reference.cancel_restored,
            "取り消しで戻る seed {seed}"
        );
        refusals += usize::from(reference.refused.is_some());
        for &level in &levels {
            let got = forced::with_level(level, || run(&case)).expect("同じ入力は同じく始められる");
            assert_eq!(got.stamps, reference.stamps, "{level:?} seed {seed}");
            assert_eq!(got.changed, reference.changed, "{level:?} seed {seed}");
            assert_eq!(got.refused, reference.refused, "{level:?} seed {seed}");
            assert!(got.restored && got.cancel_restored, "{level:?} seed {seed}");
            if got.pixels != reference.pixels {
                let at = got
                    .pixels
                    .iter()
                    .zip(&reference.pixels)
                    .position(|(a, b)| a != b)
                    .unwrap();
                panic!(
                    "{level:?} seed {seed}: 画素 ({}, {}) チャンネル {} で {} ≠ {}",
                    (at / 4) as u32 % case.size.0,
                    (at / 4) as u32 / case.size.0,
                    at % 4,
                    got.pixels[at],
                    reference.pixels[at]
                );
            }
            checked += 1;
        }
    }
    assert!(checked > 0 || levels == [Level::Scalar]);
    assert!(refusals > 0, "予算で断られる事例も通る");
}

/// 縁のアンチエイリアスの段ごとに、硬い丸・細い線・1 画素より小さいダブ・回して潰した丸・小さな画像の筆先・デュアルの丸を、どの道でも
/// 画素ごとの式と同じバイトで描く（乱数の事例とは別に、段のある形を必ず通す）。
#[test]
fn anti_aliased_shapes_paint_the_same_bytes_on_every_level() {
    let levels = forced::supported();
    let shapes: Vec<Brush> = {
        let base = |radius: f64, hardness: f64| {
            Brush::from(BrushSettings {
                radius,
                hardness,
                spacing: 0.2,
                color: Rgba8::new(200, 30, 60, 255),
                ..BrushSettings::default()
            })
        };
        let mut ellipse = base(9.0, 1.0);
        ellipse.tip.roundness = 0.2;
        ellipse.tip.angle = 37.0;
        let mut thin = base(4.0, 1.0);
        thin.tip.roundness = 0.05;
        thin.tip.angle = 120.0;
        let mut tip = base(0.4, 1.0);
        tip.tip.image = builtin_tip("rounded-square");
        let mut dual = base(7.0, 0.9);
        dual.dual = Some(DualBrush {
            radius: 1.2,
            hardness: 1.0,
            roundness: 0.5,
            angle: 20.0,
            ..DualBrush::default()
        });
        let needle = |radius: f64, roundness: f64, angle: f64| {
            let mut b = base(radius, 1.0);
            b.tip.roundness = roundness;
            b.tip.angle = angle;
            b
        };
        vec![
            base(6.0, 1.0),
            base(1.3, 0.95),
            base(0.3, 1.0),
            ellipse,
            thin,
            tip,
            dual,
            needle(6.0, 0.05, 15.0),
            needle(3.0, 0.1, 45.0),
            needle(12.0, 0.04, 70.0),
        ]
    };
    for (i, shape) in shapes.into_iter().enumerate() {
        for level in AntiAlias::ALL {
            let mut brush = shape.clone();
            brush.base.anti_alias = level;
            let case = Case {
                brush,
                size: (77, 64),
                tile: 16,
                ground: Ground::Noise,
                points: vec![
                    (5.3, 7.1, 0.3),
                    (40.7, 33.2, 1.0),
                    (70.25, 12.5, 0.6),
                    (30.0, 58.0, 0.1),
                ],
                selection: false,
                locked: false,
                parallel: i % 2 == 0,
                tight_budget: false,
                seed: i as u64,
            };
            let reference =
                forced::with_level(Level::Scalar, || per_pixel(|| run(&case))).expect("始められる");
            assert!(reference.changed, "{i} {level:?}");
            for &l in &levels {
                let got = forced::with_level(l, || run(&case)).expect("始められる");
                assert_eq!(got.pixels, reference.pixels, "形 {i} {level:?} 道 {l:?}");
            }
        }
    }
}

/// 道を固定する別のスレッドが、ストロークの途中で道を切り替えても落ちない（判断と実行で道を別々に読むと、その間にスカラーの道へ
/// 変わって、行の核の入口が使えない道に当たる）。固定を切り替えるのは試験だけだが、ほかの試験のストロークと並んで走るので起こりうる。
#[test]
fn a_level_switched_by_another_thread_does_not_break_a_stroke() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Stop<'a>(&'a AtomicBool);
    impl Drop for Stop<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let levels = forced::supported();
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !stop.load(Ordering::Relaxed) {
                for &level in &levels {
                    forced::with_level(level, std::thread::yield_now);
                }
            }
        });
        let _stop = Stop(&stop);
        for seed in 0..60 {
            let _ = run(&random_case(seed));
        }
    });
}

/// 整数の値のレーンを添字にする変換（`Slice32::to_i32`）は、どの道でも `truncate_i32`（x86_64 の `cvttps2dq` と同じ）と同じ値になる。
/// 範囲の外・NaN・無限大は `i32::MIN`、範囲の中は 0 へ切り捨て（NEON の `vcvtq_s32_f32` は範囲の外を飽和、NaN を 0 にするので、
/// 揃え直してあることの確かめ）。
#[test]
fn to_i32_follows_the_truncating_conversion_on_every_level() {
    use crate::math::simd::on_each_level32;
    unsafe fn check<V: Slice32>() {
        let values = [
            0.0f32,
            -0.0,
            0.5,
            -0.5,
            1.9999,
            -1.9999,
            255.5,
            -255.5,
            8_388_607.5,
            -8_388_607.5,
            2_147_483_520.0,
            2_147_483_648.0,
            -2_147_483_648.0,
            -2_147_483_904.0,
            1e30,
            -1e30,
            f32::MAX,
            f32::MIN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            f32::from_bits(0xFFC0_0000),
            f32::from_bits(0x7F80_0001),
            f32::MIN_POSITIVE,
        ];
        for start in 0..values.len() {
            let v = V::from_fn(|k| values[(start + k) % values.len()]);
            let mut out = [0x5A5A_5A5Ai32; 8];
            V::to_i32(v, &mut out);
            for k in 0..V::N {
                let x = values[(start + k) % values.len()];
                assert_eq!(out[k], truncate_i32(x), "{x}");
            }
            assert!(
                out[V::N..].iter().all(|&o| o == 0x5A5A_5A5A),
                "先頭の N 個より後ろは書かない"
            );
        }
    }
    on_each_level32!(check);
}
