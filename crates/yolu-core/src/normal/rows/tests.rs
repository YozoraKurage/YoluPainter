//! Normal チャンネルの合成の行の核が、画素ごとの式と同じバイトを出すことの試験。道ごと（スカラー・SSE4.1・AVX2・NEON）に比べる。
#![allow(clippy::needless_range_loop)]

use super::super::{blend_unchecked, clip_onto, fade};
use super::*;
use crate::math::simd::forced;
use crate::math::simd::tests::Rng;
use crate::math::UNIT;

const AMOUNTS: [f64; 8] = [
    1.0,
    0.7,
    0.5,
    1.0 / 255.0,
    1e-305,
    f64::from_bits(1),
    0.0,
    1.5,
];
const LENGTHS: [usize; 11] = [0, 1, 2, 3, 4, 5, 7, 8, 9, 31, 100];
const MODES: [BlendMode; 5] = [
    BlendMode::Normal,
    BlendMode::Overlay,
    BlendMode::Multiply,
    BlendMode::PassThrough,
    BlendMode::Hue,
];

fn factor_table() -> [f64; 256] {
    let mut factor = [0.0; 256];
    for (h, f) in factor.iter_mut().enumerate() {
        *f = 1.0 - 0.8 * UNIT[h];
    }
    factor
}

/// 法線の画素: 乱数に、平ら・真逆・ゼロベクトル（長さの境）・軸を混ぜる。アルファは 0・255・小さい値・乱数。
fn pixels(rng: &mut Rng, count: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(count * 4);
    for _ in 0..count {
        let rgb: [u8; 3] = match rng.next() % 10 {
            0 => [128, 128, 255],
            1 => [128, 128, 0],
            2 => [0, 0, 0],
            3 => [255, 255, 255],
            4 => [127, 127, 127],
            5 => [128, 128, 128],
            _ => [rng.byte(), rng.byte(), rng.byte()],
        };
        let a = match rng.next() & 7 {
            0 | 1 => 0,
            2 | 3 => 255,
            4 => 1,
            _ => rng.byte(),
        };
        v.extend_from_slice(&[rgb[0], rgb[1], rgb[2], a]);
    }
    v
}

#[test]
fn normal_rows_match_the_pixel_formulas_on_every_level() {
    let mut rng = Rng(61);
    let factor = factor_table();
    for &mode in &MODES {
        for &n in &LENGTHS {
            for &amount in &AMOUNTS {
                for step in [4usize, 0] {
                    let below = pixels(&mut rng, n);
                    let over = pixels(&mut rng, if step == 0 { 1 } else { n });
                    let mask = pixels(&mut rng, n);
                    for masked in [false, true] {
                        let a = RowAmount {
                            opacity: amount,
                            mask: masked.then_some((&mask[..], 4, &factor)),
                        };
                        let (mut want_b, mut want_c) = (below.clone(), below.clone());
                        for i in 0..n {
                            let d = Rgba8::from_slice(&below[i * 4..]);
                            let s = Rgba8::from_slice(&over[i * step..]);
                            want_b[i * 4..i * 4 + 4]
                                .copy_from_slice(&blend_unchecked(d, s, a.at(i), mode).to_array());
                            want_c[i * 4..i * 4 + 4]
                                .copy_from_slice(&clip_onto(d, s, a.at(i), mode).to_array());
                        }
                        for level in forced::supported() {
                            let mut res = below.clone();
                            blend_row_at(level, &mut res, &over, step, a, mode);
                            assert_eq!(
                                res, want_b,
                                "blend {level:?} {mode:?} n={n} amount={amount} step={step} masked={masked}"
                            );
                            let mut g = below.clone();
                            clip_row_at(level, &mut g, &over, step, a, mode);
                            assert_eq!(
                                g, want_c,
                                "clip {level:?} {mode:?} n={n} amount={amount} step={step} masked={masked}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn normal_fade_rows_match_the_pixel_formula_on_every_level() {
    let mut rng = Rng(62);
    let factor = factor_table();
    for &n in &LENGTHS {
        for &amount in &AMOUNTS {
            let below = pixels(&mut rng, n);
            let inner = pixels(&mut rng, n);
            let mask = pixels(&mut rng, n);
            for masked in [false, true] {
                let a = RowAmount {
                    opacity: amount,
                    mask: masked.then_some((&mask[..], 4, &factor)),
                };
                let mut want = below.clone();
                for i in 0..n {
                    let d = Rgba8::from_slice(&below[i * 4..]);
                    let s = Rgba8::from_slice(&inner[i * 4..]);
                    want[i * 4..i * 4 + 4].copy_from_slice(&fade(d, s, a.at(i)).to_array());
                }
                for level in forced::supported() {
                    let mut res = below.clone();
                    fade_row_at(level, &mut res, &inner, a);
                    assert_eq!(res, want, "{level:?} n={n} amount={amount} masked={masked}");
                }
            }
        }
    }
}

// ───────── f64 の式との差（式を f32 にした変化の大きさ） ─────────

/// f32 にする前の f64 の重ねの式（比べるためだけに残す）。
fn blend_f64(below: Rgba8, over: Rgba8, opacity: f64, mode: BlendMode) -> Rgba8 {
    use super::super::{decode, encode, rnm};
    use crate::math::to_byte;
    let t = UNIT[over.a as usize] * opacity;
    let da = UNIT[below.a as usize];
    if t <= 0.0 {
        return below;
    }
    let b = decode(below);
    let s = decode(over);
    let c = if is_detail(mode) { rnm(b, s) } else { s };
    let wb = (1.0 - t) * da;
    let ws = (1.0 - da) * t;
    let wc = da * t;
    encode(
        wb * b.0 + ws * s.0 + wc * c.0,
        wb * b.1 + ws * s.1 + wc * c.1,
        wb * b.2 + ws * s.2 + wc * c.2,
        to_byte(t + da * (1.0 - t)),
    )
}

fn clip_f64(group: Rgba8, clipped: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
    use super::super::{decode, encode, rnm};
    let t = UNIT[clipped.a as usize] * amount;
    if t <= 0.0 || group.a == 0 {
        return group;
    }
    let g = decode(group);
    let s = decode(clipped);
    let c = if is_detail(mode) { rnm(g, s) } else { s };
    encode(
        (1.0 - t) * g.0 + t * c.0,
        (1.0 - t) * g.1 + t * c.1,
        (1.0 - t) * g.2 + t * c.2,
        group.a,
    )
}

fn fade_f64(backdrop: Rgba8, inner: Rgba8, amount: f64) -> Rgba8 {
    use super::super::{decode, encode};
    use crate::math::to_byte;
    if amount >= 1.0 {
        return inner;
    }
    if amount <= 0.0 {
        return backdrop;
    }
    let ba = UNIT[backdrop.a as usize] * (1.0 - amount);
    let ia = UNIT[inner.a as usize] * amount;
    let a = ba + ia;
    if a <= 0.0 {
        return Rgba8::TRANSPARENT;
    }
    let b = decode(backdrop);
    let i = decode(inner);
    encode(
        ba * b.0 + ia * i.0,
        ba * b.1 + ia * i.1,
        ba * b.2 + ia * i.2,
        to_byte(a),
    )
}

/// 1 回の重ね・クリッピング・フェードの、f64 の式との差。乱数の画素（平ら・真逆・軸を混ぜる）で、どれも 1 段以内。
/// 違うバイトの割合は `--nocapture` で出す。
#[test]
fn one_composite_stays_within_one_step_of_the_f64_formula() {
    let mut rng = Rng(65);
    let n = 1 << 16;
    for (name, opacity) in [("1", 1.0), ("0.6", 0.6), ("0.35", 0.35), ("乱数", -1.0)] {
        for mode in [BlendMode::Normal, BlendMode::Overlay] {
            let (mut worst, mut differ, mut total) = (0u8, 0usize, 0usize);
            for _ in 0..4 {
                let below = pixels(&mut rng, n);
                let over = pixels(&mut rng, n);
                for i in 0..n {
                    let d = Rgba8::from_slice(&below[i * 4..]);
                    let s = Rgba8::from_slice(&over[i * 4..]);
                    let amount = if opacity < 0.0 { rng.unit() } else { opacity };
                    let pairs = [
                        (
                            blend_unchecked(d, s, amount, mode),
                            blend_f64(d, s, amount, mode),
                        ),
                        (clip_onto(d, s, amount, mode), clip_f64(d, s, amount, mode)),
                        (fade(d, s, amount), fade_f64(d, s, amount)),
                    ];
                    for (got, want) in pairs {
                        for (a, b) in got.to_array().into_iter().zip(want.to_array()) {
                            let diff = a.abs_diff(b);
                            worst = worst.max(diff);
                            differ += usize::from(diff != 0);
                            total += 1;
                        }
                    }
                }
            }
            println!(
                "{mode:?} 量 {name}: 最大 {worst} 段・違うバイト {differ} / {total}（{:.4}%）",
                differ as f64 * 100.0 / total as f64
            );
            assert!(worst <= 1, "{mode:?} 量 {name}: 最大 {worst} 段");
        }
    }
}
