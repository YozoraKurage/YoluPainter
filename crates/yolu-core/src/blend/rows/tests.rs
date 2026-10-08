//! 合成の式が、道（1 本のレーン・SSE4.1・AVX2・NEON、この CPU が持つもの）によらず同じ bit・同じバイトを出すことの試験。
//! 式そのもの（モードの値）は `blend.rs` の試験と、正解の絵（`tests/golden.rs` など）が見る。
//! 入力は乱数（アルファ 0・255・境目の値を多めに）。
#![allow(clippy::needless_range_loop)]

use super::super::lanes::{self, dispatch_mode};
use super::*;
use crate::math::simd::forced;
use crate::math::simd::on_each_level32;
use crate::math::simd::tests::Rng;
use crate::types::Rgba8;

fn unit(b: usize) -> f32 {
    b as f32 / 255.0
}

// ───────── 分離できるモードの式: 下・上の 256 × 256 の全部の組 ─────────

unsafe fn separable_for_every_byte_pair<V: Lanes32, const MODE: u8>() {
    for d in 0..256usize {
        let mut s = 0usize;
        while s < 256 {
            let dv = V::splat(unit(d));
            let sv = V::from_fn(|k| unit((s + k).min(255)));
            let got = lanes::separable::<V, MODE>(dv, sv);
            for k in 0..V::N {
                let want = lanes::separable::<Scalar1, MODE>(unit(d), unit((s + k).min(255)));
                assert_eq!(
                    V::lane(got, k).to_bits(),
                    want.to_bits(),
                    "{MODE} d={d} s={}",
                    (s + k).min(255)
                );
            }
            s += V::N;
        }
    }
}

unsafe fn all_separable_modes<V: Lanes32>() {
    for mode in BlendMode::LAYER_MODES {
        if mode.is_separable() {
            dispatch_mode!(mode, separable_for_every_byte_pair::<V>());
        }
    }
}

#[test]
fn separable_lanes_match_the_single_lane_for_every_byte_pair() {
    on_each_level32!(all_separable_modes);
}

// ───────── 3 成分のモード: 乱数の組 ─────────

unsafe fn rgb_for_random_triples<V: Lanes32, const MODE: u8>() {
    let mut rng = Rng(u64::from(MODE) + 100);
    let mut next = || rng.unit() as f32;
    for round in 0..40_000 {
        // 成分の一部を等しくする・端の値にする（最大最小の同点・彩度 0・クリップの境）
        let mut d = [next(), next(), next()];
        let mut s = [next(), next(), next()];
        match round % 6 {
            0 => d[1] = d[0],
            1 => s[2] = s[1],
            2 => d = [d[0]; 3],
            3 => s = [s[0]; 3],
            4 => {
                // 8 bit の値だけ（実際に来る入力）
                for v in d.iter_mut().chain(s.iter_mut()) {
                    *v = unit((next() * 255.0) as usize);
                }
            }
            _ => {}
        }
        let dv = [V::splat(d[0]), V::splat(d[1]), V::splat(d[2])];
        // レーンごとに別の上の色を入れる
        let ss: Vec<[f32; 3]> = (0..V::N)
            .map(|k| if k == 0 { s } else { [next(), next(), next()] })
            .collect();
        let sv = [
            V::from_fn(|k| ss[k][0]),
            V::from_fn(|k| ss[k][1]),
            V::from_fn(|k| ss[k][2]),
        ];
        let got = lanes::blend_rgb::<V, MODE>(dv, sv);
        for k in 0..V::N {
            let want = lanes::blend_rgb::<Scalar1, MODE>(d, ss[k]);
            let have = [V::lane(got[0], k), V::lane(got[1], k), V::lane(got[2], k)];
            assert_eq!(
                have.map(f32::to_bits),
                want.map(f32::to_bits),
                "{MODE} d={d:?} s={:?}",
                ss[k]
            );
        }
    }
}

unsafe fn all_rgb_modes<V: Lanes32>() {
    for mode in BlendMode::LAYER_MODES {
        if !mode.is_separable() {
            dispatch_mode!(mode, rgb_for_random_triples::<V>());
        }
    }
}

#[test]
fn rgb_lanes_match_the_single_lane_for_random_triples() {
    on_each_level32!(all_rgb_modes);
}

// ───────── 行の核 ─────────

fn factor_table() -> [f64; 256] {
    let mut factor = [0.0; 256];
    for (h, f) in factor.iter_mut().enumerate() {
        *f = 1.0 - 0.8 * (h as f64 / 255.0);
    }
    factor
}

/// 量: f32 へ丸めると 1 になる値・近道の境（1e-30）の前後・f32 の非正規化数・f32 では 0 になる値も入れる。
const AMOUNTS: [f64; 12] = [
    1.0,
    0.7,
    0.5,
    1.0 / 255.0,
    1e-30,
    1e-31,
    1e-40,
    1e-305,
    0.999999999,
    0.99999,
    0.0,
    1.5,
];

const LENGTHS: [usize; 15] = [0, 1, 2, 3, 4, 5, 7, 8, 9, 16, 17, 31, 33, 100, 515];

fn bytes(rng: &mut Rng, n: usize) -> Vec<u8> {
    (0..n).map(|_| rng.byte()).collect()
}

/// アルファを 0・255・小さい値・乱数にしたバイト列。
fn pixels(rng: &mut Rng, count: usize) -> Vec<u8> {
    let mut v = bytes(rng, count * 4);
    for p in v.chunks_exact_mut(4) {
        p[3] = match rng.next() & 7 {
            0 | 1 => 0,
            2 | 3 => 255,
            4 => 1,
            _ => p[3],
        };
    }
    v
}

fn levels() -> Vec<Level> {
    forced::supported()
}

#[test]
fn blend_rows_match_the_pixel_formula_on_every_level() {
    let mut rng = Rng(21);
    let factor = factor_table();
    for mode in BlendMode::LAYER_MODES
        .into_iter()
        .chain([BlendMode::PassThrough])
    {
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
                        let mut want = below.clone();
                        for i in 0..n {
                            let d = Rgba8::from_slice(&below[i * 4..]);
                            let s = Rgba8::from_slice(&over[i * step..]);
                            want[i * 4..i * 4 + 4].copy_from_slice(
                                &super::super::blend(d, s, a.at(i), mode).to_array(),
                            );
                        }
                        for level in levels() {
                            let mut res = below.clone();
                            blend_row_at(level, &mut res, &over, step, a, mode);
                            assert_eq!(
                                res, want,
                                "{level:?} {mode:?} n={n} amount={amount} step={step} masked={masked}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn clip_rows_match_the_pixel_formula_on_every_level() {
    let mut rng = Rng(22);
    let factor = factor_table();
    for mode in BlendMode::LAYER_MODES
        .into_iter()
        .chain([BlendMode::PassThrough])
    {
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
                        let mut want = below.clone();
                        for i in 0..n {
                            let d = Rgba8::from_slice(&below[i * 4..]);
                            let s = Rgba8::from_slice(&over[i * step..]);
                            want[i * 4..i * 4 + 4].copy_from_slice(
                                &super::super::clip_onto(d, s, a.at(i), mode).to_array(),
                            );
                        }
                        for level in levels() {
                            let mut g = below.clone();
                            clip_row_at(level, &mut g, &over, step, a, mode);
                            assert_eq!(
                                g, want,
                                "{level:?} {mode:?} n={n} amount={amount} step={step} masked={masked}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn fade_rows_match_the_pixel_formula_on_every_level() {
    let mut rng = Rng(23);
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
                    want[i * 4..i * 4 + 4]
                        .copy_from_slice(&super::super::fade(d, s, a.at(i)).to_array());
                }
                for level in levels() {
                    let mut res = below.clone();
                    fade_row_at(level, &mut res, &inner, a);
                    assert_eq!(res, want, "{level:?} n={n} amount={amount} masked={masked}");
                }
            }
        }
    }
}

/// 端の画素（N で割り切れない余り）と、行の外の領域を壊さないこと。
#[test]
fn rows_leave_the_bytes_outside_the_row_alone() {
    let mut rng = Rng(24);
    let a = RowAmount {
        opacity: 0.6,
        mask: None,
    };
    for level in levels() {
        for n in [1usize, 3, 5, 6, 9] {
            let mut res = pixels(&mut rng, n);
            res.extend_from_slice(&[0xAB; 7]);
            let over = pixels(&mut rng, n + 2);
            let mut r1 = res.clone();
            blend_row_at(level, &mut r1[..n * 4], &over, 4, a, BlendMode::Overlay);
            assert_eq!(&r1[n * 4..], &[0xAB; 7], "{level:?} n={n}");
        }
    }
}

/// 全体の道（`blend_row` が `level()` を見て選ぶ）も、固定した道ごとに同じバイトになる。
#[test]
fn the_dispatching_entry_points_follow_the_forced_level() {
    let mut rng = Rng(25);
    let factor = factor_table();
    let n = 70;
    let below = pixels(&mut rng, n);
    let over = pixels(&mut rng, n);
    let mask = pixels(&mut rng, n);
    let a = RowAmount {
        opacity: 0.8,
        mask: Some((&mask, 4, &factor)),
    };
    let mut results = Vec::new();
    for level in levels() {
        let r = forced::with_level(level, || {
            let mut res = below.clone();
            blend_row(&mut res, &over, 4, a, BlendMode::SoftLight);
            let mut g = below.clone();
            clip_row(&mut g, &over, 4, a, BlendMode::Hue);
            let mut f = below.clone();
            fade_row(&mut f, &over, a);
            (res, g, f)
        });
        results.push(r);
    }
    assert!(results.windows(2).all(|w| w[0] == w[1]));
}
