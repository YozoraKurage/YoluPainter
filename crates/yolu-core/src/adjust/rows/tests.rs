//! 調整レイヤーの行の核が、画素ごとの式（`AdjustKernel::composite`・`AdjustmentSettings::composite_in`）と同じバイトを出すことの試験。
#![allow(clippy::needless_range_loop)]

use super::*;
use crate::adjust::{
    AdjustmentSettings, BrightnessContrast, ColorBalance, GradientMap, Posterize, Threshold,
    ToneChannel, ToneCurves,
};
use crate::curve::{Curve, CurvePoint};
use crate::generator::Ramp;
use crate::math::simd::forced;
use crate::math::simd::tests::Rng;
use crate::math::UNIT;
use crate::types::Rgba8;

fn every_kind() -> Vec<(&'static str, AdjustmentSettings)> {
    let curve = Curve::new(vec![
        CurvePoint { x: 0.0, y: 0.1 },
        CurvePoint { x: 0.4, y: 0.6 },
        CurvePoint { x: 1.0, y: 0.9 },
    ])
    .unwrap();
    vec![
        ("invert", AdjustmentSettings::invert()),
        (
            "levels",
            AdjustmentSettings::levels(0.1, 0.9, 1.4, 0.05, 0.95).unwrap(),
        ),
        (
            "levels-identity",
            AdjustmentSettings::levels(0.0, 1.0, 1.0, 0.0, 1.0).unwrap(),
        ),
        (
            "hue-saturation",
            AdjustmentSettings::hue_saturation(40.0, 0.3, 0.1).unwrap(),
        ),
        (
            "hue-saturation-negative",
            AdjustmentSettings::hue_saturation(-120.0, -0.6, -0.4).unwrap(),
        ),
        (
            "hue-saturation-extreme",
            AdjustmentSettings::hue_saturation(180.0, 1.0, 1.0).unwrap(),
        ),
        (
            "hue-saturation-zero",
            AdjustmentSettings::hue_saturation(0.0, 0.0, 0.0).unwrap(),
        ),
        (
            "gradient-map",
            AdjustmentSettings::gradient_map(GradientMap::new(Ramp::default(), false)),
        ),
        (
            "tone-curve",
            AdjustmentSettings::tone_curve(
                ToneCurves::identity().with_curve(ToneChannel::Composite, curve.clone()),
            ),
        ),
        (
            "tone-curve-red",
            AdjustmentSettings::tone_curve(
                ToneCurves::identity().with_curve(ToneChannel::Red, curve),
            ),
        ),
        (
            "color-balance",
            AdjustmentSettings::color_balance(
                ColorBalance::new(
                    [10.0, -20.0, 30.0],
                    [40.0, 0.0, -30.0],
                    [0.0, 20.0, 50.0],
                    true,
                )
                .unwrap(),
            ),
        ),
        (
            "color-balance-free",
            AdjustmentSettings::color_balance(
                ColorBalance::new(
                    [100.0, 100.0, -100.0],
                    [-100.0, 60.0, 100.0],
                    [100.0, -100.0, 100.0],
                    false,
                )
                .unwrap(),
            ),
        ),
        (
            "color-balance-neutral",
            AdjustmentSettings::color_balance(ColorBalance::neutral()),
        ),
        (
            "brightness-contrast",
            AdjustmentSettings::brightness_contrast(BrightnessContrast::new(30.0, 20.0).unwrap()),
        ),
        (
            "threshold",
            AdjustmentSettings::threshold(Threshold::new(128).unwrap()),
        ),
        (
            "posterize",
            AdjustmentSettings::posterize(Posterize::new(5).unwrap()),
        ),
        (
            "posterize-255",
            AdjustmentSettings::posterize(Posterize::new(255).unwrap()),
        ),
    ]
}

fn pixels(rng: &mut Rng, count: usize) -> Vec<u8> {
    let mut v: Vec<u8> = (0..count * 4).map(|_| rng.byte()).collect();
    for p in v.chunks_exact_mut(4) {
        p[3] = match rng.next() & 7 {
            0 | 1 => 0,
            2 | 3 => 255,
            4 => 1,
            _ => p[3],
        };
        // 灰色・同点の成分を混ぜる（彩度 0 の分岐）
        match rng.next() % 11 {
            0 => {
                p[1] = p[0];
                p[2] = p[0];
            }
            1 => p[2] = p[1],
            _ => {}
        }
    }
    v
}

fn factor_table() -> [f64; 256] {
    let mut factor = [0.0; 256];
    for (h, f) in factor.iter_mut().enumerate() {
        *f = 1.0 - 0.8 * UNIT[h];
    }
    factor
}

#[test]
fn adjustment_rows_match_the_pixel_formula_on_every_level() {
    let mut rng = Rng(31);
    let factor = factor_table();
    let amounts = [1.0, 0.7, 0.25, 1e-305, 0.0, 1.5];
    // 長さ: N で割り切れない・256 画素ずつの段を跨ぐ
    let lengths = [0usize, 1, 3, 4, 5, 9, 63, 256, 257, 515];
    let modes = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::ColorDodge,
        BlendMode::Divide,
        BlendMode::HardMix,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
        BlendMode::DarkerColor,
        BlendMode::PassThrough,
    ];
    for (name, settings) in every_kind() {
        for channel in [ChannelKind::Color, ChannelKind::Scalar] {
            let kernel = settings.kernel(channel);
            for &mode in &modes {
                for &n in &lengths {
                    for &amount in &amounts {
                        let below = pixels(&mut rng, n);
                        let mask = pixels(&mut rng, n);
                        for masked in [false, true] {
                            let a = RowAmount {
                                opacity: amount,
                                mask: masked.then_some((&mask[..], 4, &factor)),
                            };
                            let mut want = below.clone();
                            for i in 0..n {
                                let d = Rgba8::from_slice(&below[i * 4..]);
                                let v = kernel.composite(d, a.at(i), mode);
                                want[i * 4..i * 4 + 4].copy_from_slice(&v.to_array());
                                // 公開の画素の式とも同じ
                                assert_eq!(
                                    v,
                                    settings.composite_in(channel, d, a.at(i), mode),
                                    "{name} kernel"
                                );
                            }
                            for level in forced::supported() {
                                let mut res = below.clone();
                                kernel.composite_row_at(level, &mut res, a, mode);
                                assert_eq!(
                                    res, want,
                                    "{name} {channel:?} {level:?} {mode:?} n={n} amount={amount} masked={masked}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

/// 端と同点（成分が等しい・最大最小が重なる）を含む、色の格子（16³）。
fn lattice() -> Vec<[u8; 4]> {
    const V: [u8; 16] = [
        0, 1, 2, 17, 34, 51, 85, 127, 128, 153, 170, 204, 238, 253, 254, 255,
    ];
    let mut out = Vec::new();
    for &r in &V {
        for &g in &V {
            for &b in &V {
                out.push([r, g, b, 255]);
            }
        }
    }
    out
}

unsafe fn hue_saturation_for_random_colors<V: Lanes32>() {
    let mut rng = Rng(41);
    let params = [
        (40.0, 0.3, 0.1),
        (-120.0, -0.6, -0.4),
        (180.0, 1.0, 1.0),
        (-180.0, 1.0, -1.0),
        (0.0, 0.0, 0.0),
        (91.5, -1.0, 0.0),
        (12.0, 0.55, 0.999),
    ];
    for (hue, saturation, lightness) in params {
        let settings = AdjustmentSettings::hue_saturation(hue, saturation, lightness).unwrap();
        let h = HueSat32::new(hue, saturation, lightness);
        let grid = lattice();
        let random = (0..40_000).map(|_| (0..V::N * 4).map(|_| rng.byte()).collect::<Vec<u8>>());
        let structured = grid
            .chunks(V::N)
            .filter(|c| c.len() == V::N)
            .map(|c| c.concat());
        for bytes in structured.chain(random.collect::<Vec<_>>()) {
            let px = V::load(&bytes);
            let got = hue_saturation32::<V>(&h, [px[0], px[1], px[2]]);
            for k in 0..V::N {
                let c = Rgba8::from_slice(&bytes[k * 4..]);
                let want = settings.apply(c);
                let have = [
                    V::lane(got[0], k) as u8,
                    V::lane(got[1], k) as u8,
                    V::lane(got[2], k) as u8,
                ];
                assert_eq!(
                    have,
                    [want.r, want.g, want.b],
                    "{c:?} {hue} {saturation} {lightness}"
                );
            }
        }
    }
}

#[test]
fn hue_saturation_lanes_match_the_scalar_for_random_colors() {
    crate::math::simd::on_each_level32!(hue_saturation_for_random_colors);
}

unsafe fn color_balance_for_random_colors<V: Lanes32>() {
    let mut rng = Rng(42);
    for (_, settings) in every_kind() {
        let Some(balance) = settings.color_balance_value().copied() else {
            continue;
        };
        if balance.is_neutral() {
            continue;
        }
        let (values, preserve) = balance.parts();
        let b = Balance32::new(values, preserve);
        let grid = lattice();
        let random = (0..40_000).map(|_| (0..V::N * 4).map(|_| rng.byte()).collect::<Vec<u8>>());
        let structured = grid
            .chunks(V::N)
            .filter(|c| c.len() == V::N)
            .map(|c| c.concat());
        for bytes in structured.chain(random.collect::<Vec<_>>()) {
            let px = V::load(&bytes);
            let got = color_balance32::<V>(&b, [px[0], px[1], px[2]]);
            for k in 0..V::N {
                let c = Rgba8::from_slice(&bytes[k * 4..]);
                let want = balance.apply(c);
                let have = [
                    V::lane(got[0], k) as u8,
                    V::lane(got[1], k) as u8,
                    V::lane(got[2], k) as u8,
                ];
                assert_eq!(have, [want.r, want.g, want.b], "{c:?}");
            }
        }
    }
}

#[test]
fn color_balance_lanes_match_the_scalar_for_random_colors() {
    crate::math::simd::on_each_level32!(color_balance_for_random_colors);
}

/// f32 にする前の f64 のカラーバランスの式（比べるためだけに残す）。
fn color_balance_f64(values: &[[f64; 3]; 3], preserve: bool, c: Rgba8) -> Rgba8 {
    use crate::math::{clamp01, to_byte};
    let lum = |r: f64, g: f64, b: f64| 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let (r, g, b) = (UNIT[c.r as usize], UNIT[c.g as usize], UNIT[c.b as usize]);
    let y = lum(r, g, b);
    let w = [(1.0 - y) * (1.0 - y), 4.0 * y * (1.0 - y), y * y];
    let mut d = [0.0; 3];
    for (k, delta) in d.iter_mut().enumerate() {
        let sum: f64 = (0..3).map(|range| values[range][k] * w[range]).sum();
        *delta = sum / 100.0 * 0.3;
    }
    let (mut r2, mut g2, mut b2) = (r + d[0], g + d[1], b + d[2]);
    if preserve {
        let back = y - lum(clamp01(r2), clamp01(g2), clamp01(b2));
        r2 += back;
        g2 += back;
        b2 += back;
    }
    Rgba8::new(
        to_byte(clamp01(r2)),
        to_byte(clamp01(g2)),
        to_byte(clamp01(b2)),
        c.a,
    )
}

/// f32 の式と f64 の式の差: RGB の立方体の 3 刻み（86³）と灰色の近く（成分の差 ±3 以内）で、どれも 1 段以内。違うバイトの割合は
/// `--nocapture` で出す。
#[test]
fn color_balance_stays_within_one_step_of_the_f64_formula() {
    let mut colors = Vec::new();
    for r in (0..=255u32).step_by(3) {
        for g in (0..=255u32).step_by(3) {
            for b in (0..=255u32).step_by(3) {
                colors.push(Rgba8::new(r as u8, g as u8, b as u8, 255));
            }
        }
    }
    for v in 0..=255i32 {
        for dg in -3..=3i32 {
            for db in -3..=3i32 {
                let (g, b) = (v + dg, v + db);
                if (0..=255).contains(&g) && (0..=255).contains(&b) {
                    colors.push(Rgba8::new(v as u8, g as u8, b as u8, 255));
                }
            }
        }
    }
    for (name, settings) in every_kind() {
        let Some(balance) = settings.color_balance_value().copied() else {
            continue;
        };
        let (values, preserve) = balance.parts();
        let (mut worst, mut differ) = (0u8, 0usize);
        for &c in &colors {
            let got = balance.apply(c);
            let want = if balance.is_neutral() {
                c
            } else {
                color_balance_f64(values, preserve, c)
            };
            for (a, b) in got.to_array().into_iter().zip(want.to_array()) {
                worst = worst.max(a.abs_diff(b));
                differ += usize::from(a != b);
            }
        }
        let total = colors.len() * 3;
        println!(
            "{name}: 最大 {worst} 段・違うバイト {differ} / {total}（{:.4}%）",
            differ as f64 * 100.0 / total as f64
        );
        assert!(worst <= 1, "{name}: 最大 {worst} 段");
    }
}

/// f32 にする前の f64 の色相/彩度の式（比べるためだけに残す）。
fn hue_saturation_f64(hue: f64, saturation: f64, lightness: f64, c: [u8; 3]) -> [u8; 3] {
    use crate::math::to_byte;
    fn hue_to_rgb(p: f64, q: f64, mut t: f64) -> f64 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    }
    let (r, g, b) = (
        UNIT[c[0] as usize],
        UNIT[c[1] as usize],
        UNIT[c[2] as usize],
    );
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let mut l = (mx + mn) / 2.0;
    let (mut h, mut s) = (0.0, 0.0);
    let d = mx - mn;
    if d > 1e-12 {
        s = if l > 0.5 {
            d / (2.0 - mx - mn)
        } else {
            d / (mx + mn)
        };
        h = if mx == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if mx == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        h /= 6.0;
    }
    h += hue / 360.0;
    h -= h.floor();
    s = (s * (1.0 + saturation)).clamp(0.0, 1.0);
    l = if lightness >= 0.0 {
        l + (1.0 - l) * lightness
    } else {
        l * (1.0 + lightness)
    };
    if s <= 0.0 {
        return [to_byte(l); 3];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    [h + 1.0 / 3.0, h, h - 1.0 / 3.0].map(|t| to_byte(hue_to_rgb(p, q, t)))
}

/// f32 の式と f64 の式の差: RGB の立方体の 3 刻み（86³）と灰色の近く（成分の差 ±3 以内）× 色相 7・彩度 5・明度 4 の 140 設定で、
/// どれも 1 段以内。違うバイト・画素の割合は `--nocapture` で出す。
#[test]
fn hue_saturation_stays_within_one_step_of_the_f64_formula() {
    let mut colors = Vec::new();
    for r in (0..=255u32).step_by(3) {
        for g in (0..=255u32).step_by(3) {
            for b in (0..=255u32).step_by(3) {
                colors.push([r as u8, g as u8, b as u8]);
            }
        }
    }
    for v in 0..=255i32 {
        for dg in -3..=3i32 {
            for db in -3..=3i32 {
                let (g, b) = (v + dg, v + db);
                if (0..=255).contains(&g) && (0..=255).contains(&b) {
                    colors.push([v as u8, g as u8, b as u8]);
                }
            }
        }
    }
    let (mut worst, mut differ, mut total, mut pixels_differ, mut pixels) =
        (0u8, 0u64, 0u64, 0u64, 0u64);
    for hue in [-180.0, -120.0, -45.0, 0.0, 30.0, 90.0, 180.0] {
        for saturation in [-1.0, -0.5, 0.0, 0.4, 1.0] {
            for lightness in [-0.6, 0.0, 0.25, 0.8] {
                let settings =
                    AdjustmentSettings::hue_saturation(hue, saturation, lightness).unwrap();
                for c in &colors {
                    let want = hue_saturation_f64(hue, saturation, lightness, *c);
                    let got = settings.apply(Rgba8::new(c[0], c[1], c[2], 255));
                    let mut any = false;
                    for (a, b) in [got.r, got.g, got.b].into_iter().zip(want) {
                        let d = a.abs_diff(b);
                        worst = worst.max(d);
                        differ += u64::from(d != 0);
                        any |= d != 0;
                        total += 1;
                    }
                    pixels_differ += u64::from(any);
                    pixels += 1;
                }
            }
        }
    }
    println!(
        "色相/彩度: 最大 {worst} 段・違うバイト {differ} / {total}（{:.3}%）・違う画素 {pixels_differ} / {pixels}（{:.3}%）",
        differ as f64 * 100.0 / total as f64,
        pixels_differ as f64 * 100.0 / pixels as f64
    );
    assert!(worst <= 1, "最大 {worst} 段");
}
