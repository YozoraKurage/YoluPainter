//! Normal チャンネルの行の核が、画素ごとの式と同じバイトを出すことの試験。道ごと（スカラー・SSE4.1・AVX2）に比べる。
#![allow(clippy::needless_range_loop)]

use super::super::{output_pixel, row, HeightEdgeMode, NormalSettings, NormalYDirection};
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

/// 高さ（0〜1）: 端・段・乱数。
fn heights(rng: &mut Rng, count: usize) -> Vec<f64> {
    (0..count)
        .map(|_| match rng.next() % 6 {
            0 => 0.0,
            1 => 1.0,
            2 => f64::from(rng.byte()) / 255.0,
            3 => height_of(rng.byte(), rng.byte()),
            _ => rng.unit(),
        })
        .collect()
}

#[test]
fn output_rows_match_the_pixel_formula_on_every_level() {
    let mut rng = Rng(63);
    for w in [1usize, 2, 3, 4, 5, 6, 7, 8, 9, 17, 33, 64] {
        for edges in [HeightEdgeMode::Clamp, HeightEdgeMode::Wrap] {
            for strength in [4.0, -4.0, 0.5, 256.0, 0.0] {
                let settings =
                    NormalSettings::new(true, strength, edges, NormalYDirection::OpenGL).unwrap();
                for (with_normal, with_heights) in
                    [(true, true), (true, false), (false, true), (false, false)]
                {
                    let normal = pixels(&mut rng, w);
                    let hs = heights(&mut rng, 3 * w);
                    let n = with_normal.then_some(&normal[..]);
                    let h = with_heights.then_some(&hs[..]);
                    let mut want = vec![0u8; w * 4];
                    let wrap = edges == HeightEdgeMode::Wrap;
                    for x in 0..w {
                        output_pixel(x, n, h, w, wrap, strength, &mut want);
                    }
                    for level in forced::supported() {
                        let mut out = vec![0xEEu8; w * 4];
                        forced::with_level(level, || row(n, h, w, &settings, &mut out));
                        assert_eq!(
                            out, want,
                            "{level:?} w={w} {edges:?} strength={strength} normal={with_normal} heights={with_heights}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn heights_from_rgba_matches_height_of() {
    let mut rng = Rng(64);
    for &n in &LENGTHS {
        let src = pixels(&mut rng, n);
        let want: Vec<f64> = (0..n)
            .map(|x| height_of(src[x * 4], src[x * 4 + 3]))
            .collect();
        for level in forced::supported() {
            let mut out = vec![-1.0; n];
            heights_from_rgba(level, &src, &mut out);
            assert_eq!(
                out.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                want.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                "{level:?} n={n}"
            );
        }
    }
    // 取りうる (R, A) の全部
    let all: Vec<u8> = (0..=255u32)
        .flat_map(|r| (0..=255u32).flat_map(move |a| [r as u8, 0, 0, a as u8]))
        .collect();
    let want: Vec<f64> = (0..65536)
        .map(|x| height_of(all[x * 4], all[x * 4 + 3]))
        .collect();
    for level in forced::supported() {
        let mut out = vec![-1.0; 65536];
        heights_from_rgba(level, &all, &mut out);
        assert!(
            out.iter()
                .zip(&want)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "{level:?}"
        );
    }
}
