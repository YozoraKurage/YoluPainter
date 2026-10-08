//! Normal のチャンネルの出力の行の核が、画素ごとの式と同じバイトを出すことの試験。道ごと（スカラー・SSE4.1・AVX2・NEON）に比べる。

use super::super::{output_pixel, row, HeightEdgeMode, NormalSettings, NormalYDirection};
use super::*;
use crate::math::simd::forced;
use crate::math::simd::tests::Rng;

const LENGTHS: [usize; 11] = [0, 1, 2, 3, 4, 5, 7, 8, 9, 31, 100];

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
