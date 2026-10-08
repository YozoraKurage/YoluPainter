//! フィルターの行の核が、画素ごとの式（元の実装そのままの写し）と同じバイトを出すことの試験。道ごと（スカラー・SSE4.1・AVX2・NEON）に比べる。
#![allow(clippy::needless_range_loop)]

use super::super::pixels::{box_blur_scalar, mix};
use super::*;
use crate::math::simd::forced;
use crate::math::simd::tests::Rng;

/// この CPU が持つ道（スカラーを含む）。
fn levels() -> Vec<Level> {
    forced::supported()
}

/// SIMD の道だけ。
fn simd_levels() -> Vec<Level> {
    forced::supported()
        .into_iter()
        .filter(|l| *l != Level::Scalar)
        .collect()
}

fn bytes(rng: &mut Rng, n: usize) -> Vec<u8> {
    (0..n).map(|_| rng.byte()).collect()
}

const LENGTHS: [usize; 11] = [0, 1, 2, 3, 4, 5, 7, 8, 9, 31, 100];
const STRENGTHS: [f64; 6] = [1.0, 0.999, 0.7, 0.5, 1.0 / 255.0, 1e-9];

// ───────── 強さの混ぜ・表・ノイズ ─────────

#[test]
fn lerp_rows_match_the_scalar_lerp() {
    let mut rng = Rng(51);
    for &n in &LENGTHS {
        for &t in &STRENGTHS {
            let base = bytes(&mut rng, n * 4);
            let target = bytes(&mut rng, n * 4);
            let mut want = base.clone();
            for i in 0..n {
                for c in 0..3 {
                    want[i * 4 + c] = lerp(base[i * 4 + c], target[i * 4 + c], t);
                }
            }
            for level in levels() {
                let mut row = base.clone();
                lerp_rows_at(level, &mut row, &target, t);
                assert_eq!(row, want, "{level:?} n={n} t={t}");
            }
        }
    }
}

#[test]
fn lut_rows_match_the_scalar_lerp() {
    let mut rng = Rng(52);
    let mut lut = [0u8; 256];
    for l in &mut lut {
        *l = rng.byte();
    }
    lut[0] = 255;
    lut[255] = 0;
    for &n in &LENGTHS {
        for &t in &STRENGTHS {
            let base = bytes(&mut rng, n * 4);
            let mut want = base.clone();
            for i in 0..n {
                for c in 0..3 {
                    let v = base[i * 4 + c];
                    want[i * 4 + c] = lerp(v, lut[v as usize], t);
                }
            }
            for level in levels() {
                let mut row = base.clone();
                lut_row_at(level, &mut row, &lut, t);
                assert_eq!(row, want, "{level:?} n={n} t={t}");
            }
        }
    }
}

/// 元の `point` のノイズの式の写し（独立の基準）。
fn old_noise(p: &mut [u8], x: u32, y: u32, amount: f64, seed: i32, mono: bool, t: f64) {
    let h = hash(hash(hash(seed as u32) ^ x) ^ y);
    for (c, v) in p[..3].iter_mut().enumerate() {
        let u = f64::from(hash(h ^ if mono { 0 } else { c as u32 }) >> 8) / 16777215.0 * 2.0 - 1.0;
        let n = (f64::from(*v) + amount * 127.5 * u + 0.5)
            .floor()
            .clamp(0.0, 255.0) as u8;
        *v = lerp(*v, n, t);
    }
}

#[test]
fn noise_rows_match_the_original_formula() {
    let mut rng = Rng(53);
    for &n in &LENGTHS {
        for &t in &STRENGTHS {
            for (amount, seed, mono) in [
                (0.47, -39, true),
                (0.83, 39, false),
                (1.0, 0, false),
                (0.0, 7, true),
                (0.02, i32::MIN, false),
            ] {
                let base = bytes(&mut rng, n * 4);
                let (x0, y) = (rng.next() as u32 % 5000, rng.next() as u32 % 5000);
                let mut want = base.clone();
                for i in 0..n {
                    old_noise(&mut want[i * 4..], x0 + i as u32, y, amount, seed, mono, t);
                }
                for level in levels() {
                    let mut row = base.clone();
                    noise_row_at(level, &mut row, x0, y, seed, amount, mono, t);
                    assert_eq!(row, want, "{level:?} n={n} t={t} {amount} {seed} {mono}");
                }
            }
        }
    }
}

// ───────── ぼかし・シャープの最後の 1 行 ─────────

/// 乗算済みの u16 × 4 の乱数（アルファの端・割り切れる値・丸めの境を多めに）。
fn premultiplied(rng: &mut Rng, n: usize) -> Vec<u16> {
    let mut q = Vec::with_capacity(n * 4);
    for _ in 0..n {
        let a8 = u32::from(rng.byte());
        let qa = match rng.next() % 9 {
            0 => 0,
            1 => 1,
            2 => 127,
            3 => 128,
            4 => 65025,
            5 => a8 * 255,
            6 => (rng.next() % 65026) as u32,
            _ => a8 * 255 + (rng.next() % 3) as u32,
        };
        for _ in 0..3 {
            let v = match rng.next() % 6 {
                0 => 0,
                1 => qa,
                2 => 65025,
                3 => (qa * u32::from(rng.byte())) / 255,
                _ => (rng.next() % 65026) as u32,
            };
            q.push(v.min(65535) as u16);
        }
        q.push(qa as u16);
    }
    q
}

#[test]
fn blur_finish_rows_match_the_original_formula() {
    let mut rng = Rng(54);
    for &n in &LENGTHS {
        for &t in &STRENGTHS {
            let input: Vec<u8> = {
                let mut v = bytes(&mut rng, n * 4);
                for p in v.chunks_exact_mut(4) {
                    p[3] = match rng.next() & 3 {
                        0 => 0,
                        1 => 255,
                        _ => p[3],
                    };
                }
                v
            };
            let q = premultiplied(&mut rng, n);
            // 元の実装: 逆乗算 → mix
            let mut want = vec![0u8; n * 4];
            for i in 0..n {
                let inp: [u8; 4] = input[i * 4..i * 4 + 4].try_into().unwrap();
                let qa = u32::from(q[i * 4 + 3]);
                let mut f = inp;
                let a = ((qa + 127) / 255) as u8;
                if a == 0 {
                    f[3] = 0;
                } else {
                    for c in 0..3 {
                        f[c] = ((2 * u32::from(q[i * 4 + c]) * 255 + qa) / (2 * qa)).min(255) as u8;
                    }
                    f[3] = a;
                }
                want[i * 4..i * 4 + 4].copy_from_slice(&mix(inp, f, t, false));
            }
            let settings = Settings::GaussianBlur { radius: 3 };
            for level in levels() {
                for ty in [ValueType::Color, ValueType::Scalar, ValueType::Mask] {
                    let mut out = vec![0u8; n * 4];
                    assert!(finish_row_at(level, &settings, ty, t, &input, &q, &mut out));
                    assert_eq!(out, want, "{level:?} n={n} t={t}");
                }
            }
        }
    }
}

#[test]
fn tangent_normal_blur_is_left_to_the_pixel_path() {
    let settings = Settings::GaussianBlur { radius: 3 };
    for level in levels() {
        let mut out = vec![0u8; 16];
        assert!(!finish_row_at(
            level,
            &settings,
            ValueType::TangentNormal,
            1.0,
            &[0; 16],
            &[0; 16],
            &mut out
        ));
    }
}

#[test]
fn sharpen_finish_rows_match_the_original_formula() {
    let mut rng = Rng(55);
    for &n in &LENGTHS {
        for &t in &STRENGTHS {
            for (amount, threshold) in [(1.7, 12u32), (0.0, 0), (5.0, 255), (3.3, 1), (0.5, 100)] {
                let input: Vec<u8> = {
                    let mut v = bytes(&mut rng, n * 4);
                    for p in v.chunks_exact_mut(4) {
                        p[3] = match rng.next() & 3 {
                            0 => 0,
                            1 => 255,
                            _ => p[3],
                        };
                    }
                    v
                };
                let q = premultiplied(&mut rng, n);
                let mut want = vec![0u8; n * 4];
                for i in 0..n {
                    let inp: [u8; 4] = input[i * 4..i * 4 + 4].try_into().unwrap();
                    let qa = u32::from(q[i * 4 + 3]);
                    let mut f = inp;
                    if inp[3] != 0 && qa != 0 {
                        for c in 0..3 {
                            let diff =
                                f64::from(inp[c]) - f64::from(q[i * 4 + c]) * 255.0 / f64::from(qa);
                            if diff.abs() >= f64::from(threshold) {
                                f[c] = (f64::from(inp[c]) + amount * diff + 0.5)
                                    .floor()
                                    .clamp(0.0, 255.0) as u8;
                            }
                        }
                    }
                    for c in 0..3 {
                        f[c] = lerp(inp[c], f[c], t);
                    }
                    want[i * 4..i * 4 + 4].copy_from_slice(&f);
                }
                let settings = Settings::Sharpen {
                    radius: 3,
                    amount,
                    threshold,
                };
                for level in levels() {
                    let mut out = vec![0u8; n * 4];
                    assert!(finish_row_at(
                        level,
                        &settings,
                        ValueType::Color,
                        t,
                        &input,
                        &q,
                        &mut out
                    ));
                    assert_eq!(out, want, "{level:?} n={n} t={t} {amount} {threshold}");
                }
            }
        }
    }
}

// ───────── 箱ぼかし ─────────

/// 割り算の置き換え（合計 + r) / (2r + 1) = floor((合計 + r + 0.5) × (1 / (2r + 1)))` が、元の実装の `(合計 + r) × inv >> 40` と、
/// 取りうる合計（0〜(2r+1)×65535）のすべて（半径 1〜16）・商の端の前後（それ以外の半径）で同じ値。
#[test]
fn the_average_by_multiplication_is_the_integer_quotient() {
    for r in 1..=256u32 {
        let d = u64::from(2 * r) + 1;
        let inv = ((1u64 << 40) + u64::from(2 * r)) / d;
        let rcp = 1.0 / (f64::from(2 * r) + 1.0);
        let bias = f64::from(r) + 0.5;
        let check = |sum: u32| {
            let old = ((u64::from(sum + r) * inv) >> 40) as u32;
            let new = ((f64::from(sum) + bias) * rcp).floor() as u32;
            assert_eq!(old, new, "r={r} sum={sum}");
            assert_eq!(old, (sum + r) / (2 * r + 1), "r={r} sum={sum}（整数の商）");
        };
        let max = (2 * r + 1) * 65535;
        if r <= 16 {
            for sum in 0..=max {
                check(sum);
            }
        } else {
            let step = 1 + r as usize % 7;
            for k in (0..=65535u32).step_by(step) {
                for dx in [0u32, 1, 2, d as u32 - 2, d as u32 - 1] {
                    let base = k * (2 * r + 1);
                    // 商が k になる合計の両端
                    for sum in [(base + dx).saturating_sub(r), base + dx + (2 * r) / 2] {
                        if sum <= max {
                            check(sum);
                        }
                    }
                }
            }
            check(max);
            check(max - 1);
            check(0);
        }
    }
}

fn grow(r: Rect, m: u32, w: u32, h: u32) -> Rect {
    let x = r.x.saturating_sub(m);
    let y = r.y.saturating_sub(m);
    Rect::new(
        x,
        y,
        (r.x + r.width).saturating_add(m).min(w) - x,
        (r.y + r.height).saturating_add(m).min(h) - y,
    )
}

#[test]
fn box_blur_matches_the_scalar_implementation() {
    let mut rng = Rng(56);
    let ok: Check<'_> = &|| Ok(());
    for round in 0..120 {
        let w = 1 + (rng.next() % 37) as u32;
        let h = 1 + (rng.next() % 29) as u32;
        let r = match round % 6 {
            0 => 1,
            1 => 2,
            2 => 3,
            3 => 8,
            4 => 1 + (rng.next() % 40) as u32,
            _ => 256,
        };
        let bx = (rng.next() % u64::from(w)) as u32;
        let by = (rng.next() % u64::from(h)) as u32;
        let b = Rect::new(
            bx,
            by,
            1 + (rng.next() % u64::from(w - bx)) as u32,
            1 + (rng.next() % u64::from(h - by)) as u32,
        );
        let a = grow(b, r, w, h);
        let src: Vec<u16> = (0..area(a) * 4)
            .map(|_| match rng.next() % 8 {
                0 => 0,
                1 => 65025,
                2 => 65535,
                _ => (rng.next() % 65026) as u16,
            })
            .collect();
        let want = box_blur_scalar(&src, a, b, r, w, h, ok).unwrap();
        for level in simd_levels() {
            let got = box_blur_at(level, &src, a, b, r, w, h, ok)
                .expect("SIMD の道がある")
                .unwrap();
            assert_eq!(got, want, "{level:?} {w}x{h} r={r} a={a:?} b={b:?}");
        }
        assert!(box_blur_at(Level::Scalar, &src, a, b, r, w, h, ok).is_none());
    }
}

#[test]
fn box_blur_stops_on_cancel() {
    let (w, h) = (20u32, 20u32);
    let b = Rect::new(0, 0, w, h);
    let src = vec![1000u16; (w * h * 4) as usize];
    let cancel: Check<'_> = &|| Err(Error::Cancelled);
    for level in simd_levels() {
        let got = box_blur_at(level, &src, b, b, 2, w, h, cancel).unwrap();
        assert!(matches!(got, Err(Error::Cancelled)));
    }
}

#[test]
fn premultiply_matches_the_scalar() {
    let mut rng = Rng(57);
    for &n in &LENGTHS {
        let src: Vec<u8> = {
            let mut v = bytes(&mut rng, n * 4);
            for p in v.chunks_exact_mut(4) {
                p[3] = match rng.next() & 3 {
                    0 => 0,
                    1 => 255,
                    _ => p[3],
                };
            }
            v
        };
        let mut want = vec![0u16; n * 4];
        for i in 0..n {
            let a = u16::from(src[i * 4 + 3]);
            for c in 0..3 {
                want[i * 4 + c] = u16::from(src[i * 4 + c]) * a;
            }
            want[i * 4 + 3] = a * 255;
        }
        for level in levels() {
            let mut out = vec![0xFFFFu16; n * 4 + 2];
            premultiply_at(level, &src, &mut out[..n * 4]);
            assert_eq!(&out[..n * 4], &want[..], "{level:?} n={n}");
            assert_eq!(&out[n * 4..], &[0xFFFF; 2], "{level:?}");
        }
    }
}
