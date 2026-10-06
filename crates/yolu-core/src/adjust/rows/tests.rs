//! 調整の層の行の核が、画素ごとの式（`AdjustKernel::composite`・`AdjustmentSettings::composite_in`）と同じバイトを出すことの試験。
#![allow(clippy::needless_range_loop)]

use super::*;
use crate::adjust::{
    AdjustmentSettings, BrightnessContrast, ColorBalance, GradientMap, Posterize, Threshold,
    ToneChannel, ToneCurves,
};
use crate::curve::{Curve, CurvePoint};
use crate::generator::Ramp;
use crate::math::simd::forced;
use crate::math::simd::on_each_level;
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

unsafe fn hue_saturation_for_random_colors<V: Lanes>() {
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
        let grid = lattice();
        let random = (0..40_000).map(|_| (0..V::N * 4).map(|_| rng.byte()).collect::<Vec<u8>>());
        let structured = grid
            .chunks(V::N)
            .filter(|c| c.len() == V::N)
            .map(|c| c.concat());
        for bytes in structured.chain(random.collect::<Vec<_>>()) {
            let px = V::load(&bytes);
            let got =
                hue_saturation::<V>(hue / 360.0, saturation, lightness, [px[0], px[1], px[2]]);
            for k in 0..V::N {
                let c = Rgba8::from_slice(&bytes[k * 4..]);
                let (r, g, b) = settings.hue_saturation_lightness(
                    UNIT[c.r as usize],
                    UNIT[c.g as usize],
                    UNIT[c.b as usize],
                );
                let want = [
                    crate::math::to_byte(r),
                    crate::math::to_byte(g),
                    crate::math::to_byte(b),
                ];
                let have = [
                    V::lane(got[0], k) as u8,
                    V::lane(got[1], k) as u8,
                    V::lane(got[2], k) as u8,
                ];
                assert_eq!(have, want, "{c:?} {hue} {saturation} {lightness}");
            }
        }
    }
}

#[test]
fn hue_saturation_lanes_match_the_scalar_for_random_colors() {
    on_each_level!(hue_saturation_for_random_colors);
}

unsafe fn color_balance_for_random_colors<V: Lanes>() {
    let mut rng = Rng(42);
    for (_, settings) in every_kind() {
        let Some(balance) = settings.color_balance_value().copied() else {
            continue;
        };
        if balance.is_neutral() {
            continue;
        }
        let (values, preserve) = balance.parts();
        let grid = lattice();
        let random = (0..40_000).map(|_| (0..V::N * 4).map(|_| rng.byte()).collect::<Vec<u8>>());
        let structured = grid
            .chunks(V::N)
            .filter(|c| c.len() == V::N)
            .map(|c| c.concat());
        for bytes in structured.chain(random.collect::<Vec<_>>()) {
            let px = V::load(&bytes);
            let got = color_balance::<V>(values, preserve, [px[0], px[1], px[2]]);
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
    on_each_level!(color_balance_for_random_colors);
}
