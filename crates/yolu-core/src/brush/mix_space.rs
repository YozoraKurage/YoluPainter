//! 色の混ぜの「知覚的」: 絵の具のように混ざる色の式（混色モード）と、混ざって暗くなる分の補正（輝度の補正）。
//!
//! 標準の混ぜは sRGB の値をそのまま補間する（青と黄は灰色に沈む）。知覚的は、**錐体応答（LMS）の空間で、吸収の積に近い混ぜ**にする。
//! 絵の具は光を吸収して混ざる（混ぜた絵の具が反射する光は、それぞれが反射する光の積に近い）ので、線形の光を錐体の 3 つの帯域
//! （L・M・S）へ移し、帯域ごとに混ぜる。青（S だけを反射）と黄（L・M を反射）を混ぜると、どの帯域も反射が小さくなって暗い緑になり、
//! 白に少量の黄を混ぜると、白の反射はほとんど変わらないので黄が強く出る。RGB の 3 原色の上で同じ混ぜをすると、青の R・G が 0 に
//! 張り付いて緑にならない（錐体の帯域は重なっているので、青にも M の反射が少しあり、緑が残る）。
//!
//! 式（t は後ろの色の重み。`a`・`b` は sRGB の値）:
//! 1. 線形の光（`to_linear` の表）から錐体応答 `x = M·lin + ε` を求める。M は Hunt-Pointer-Estevez の錐体基本（D65 で正規化）に sRGB の
//!    原色を合わせた 3×3、ε = 0.005（0 の応答が積を 0 に潰さない床）。
//! 2. 8 乗根の冪平均 `P = (1 − t)·a^(1/8) + t·b^(1/8)`（指数 1/8 は幾何平均に近く、平方根を 3 回で求められる）。
//! 3. 混ぜた応答 `P^8`。**輝度の補正**は、混ざって暗くなる分の指数を 5 段で変える: `P^(16 − 2·level)` を、重み `s = min(1, min(t, 1 − t) / 0.05)` と、
//!    2 つの色の違い `g = min(1, Δ / 0.16)`（Δ は 3 つの根の差の最大。同じ色どうしは 0）の積 `s·g` だけ `P^8` から寄せる（`level` = 0 で指数
//!    16/8 = 2、4 で 1。両端 t = 0・1 と、同じ色どうしは動かない）。
//! 4. 応答から ε を引いて 0 以上に収め、M の逆行列で線形の光へ、`to_srgb` で sRGB へ。
//!
//! 式は + − × ÷ と sqrt、表引きだけ（OS の数学の関数 `powf`・`exp` を使わない）。だから画素ごとの式とレーンの式、スカラー・SSE4.1・AVX2・NEON の
//! どれでも同じバイトになり、OS（Windows と Linux）にも左右されない。

use crate::generator::{to_linear, to_srgb};

/// 輝度の補正の最大の段（0〜4）。
pub(crate) const MAX_LEVEL: u8 = 4;

/// 応答の床。
const FLOOR: f64 = 0.005;

/// 暗くなる分が全部掛かる 2 つの色の違い（根の差の最大）。青と黄・白と黄で 0.26 ほど、同じ色どうしは 0。
const DIFFERENCE: f64 = 0.16;

/// 暗くなる分が全部掛かる後ろの色の重み（両端からの距離）。荷を打点ごとに混ぜ足していく使い方では、1 回の重みが両端に近いので、小さい重みから効かせる。
const ONSET: f64 = 0.05;

/// 線形の sRGB → 錐体応答（Hunt-Pointer-Estevez、D65 で正規化。XYZ → LMS に sRGB（D65）の XYZ 変換を掛けた行列）。
const TO_LMS: [[f64; 3]; 3] = [
    [0.313_990_216_2, 0.639_512_938_3, 0.046_497_546_2],
    [0.155_372_406_3, 0.757_894_461_6, 0.086_701_418_6],
    [0.017_752_387_0, 0.109_442_094_4, 0.872_569_224_6],
];

/// 錐体応答 → 線形の sRGB（上の逆行列）。
const FROM_LMS: [[f64; 3]; 3] = [
    [5.472_212_058_4, -4.641_960_098_4, 0.169_637_076_8],
    [-1.125_241_895_5, 2.293_170_938_1, -0.167_895_202_2],
    [0.029_801_651_2, -0.193_180_728_3, 1.163_647_892_8],
];

/// sRGB の値（0〜255。小数でもよい）の線形の光（表の隣り合う 2 つを補間する）。
fn linear_of(v: f64) -> f64 {
    let v = if v > 0.0 { v.min(255.0) } else { 0.0 };
    let i = v.floor();
    let k = i as usize;
    let lo = to_linear(k as u8);
    if k >= 255 {
        return lo;
    }
    lo + (to_linear(k as u8 + 1) - lo) * (v - i)
}

/// 1 つの色の、混ぜの式が読む形（応答の 8 乗根）。ダブの色は 1 回だけ作り、画素ごとには下地の分だけ作る。
pub(crate) fn roots(rgb: [f64; 3]) -> [f64; 3] {
    let lin = [linear_of(rgb[0]), linear_of(rgb[1]), linear_of(rgb[2])];
    std::array::from_fn(|i| {
        let x = TO_LMS[i][0] * lin[0] + TO_LMS[i][1] * lin[1] + TO_LMS[i][2] * lin[2];
        let x = if x > 0.0 { x } else { 0.0 } + FLOOR;
        x.sqrt().sqrt().sqrt()
    })
}

/// 色 `a`（重み 1 − t）と `b`（重み t）の知覚的な混ぜ（sRGB の 0〜255 の実数。丸めは呼び手）。`a_root`・`b_root` は [`roots`]。
/// `level` は輝度の補正（0〜4。5 以上は 4）。t ≤ 0 は `a`、t ≥ 1 は `b` を、混ぜずにそのまま返す。
pub(crate) fn mix_roots(
    a: [f64; 3],
    b: [f64; 3],
    a_root: [f64; 3],
    b_root: [f64; 3],
    t: f64,
    level: u8,
) -> [f64; 3] {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let level = level.min(MAX_LEVEL);
    // 暗くなる分は、後ろの色の重みが両端（0・1）から離れるほど（`ONSET` で全部掛かる）、2 つの色の違い（応答の根の差の最大。同じ色どうしは
    // 混ぜても暗くならない）が大きいほど掛かる
    let delta = (0..3).fold(0.0_f64, |m, i| m.max((a_root[i] - b_root[i]).abs()));
    let s = (t.min(1.0 - t) / ONSET).min(1.0) * (delta / DIFFERENCE).min(1.0);
    let lms: [f64; 3] = std::array::from_fn(|i| {
        let p = (1.0 - t) * a_root[i] + t * b_root[i];
        let p2 = p * p;
        let p4 = p2 * p2;
        let base = p4 * p4;
        // P^(16 − 2·level) = P^8 · P^(8 − 2·level)
        let extra = match level {
            0 => p4 * p4,
            1 => p4 * p2,
            2 => p4,
            3 => p2,
            _ => 1.0,
        };
        let dark = base * extra;
        let m = base + s * (dark - base);
        let m = m - FLOOR;
        if m > 0.0 {
            m
        } else {
            0.0
        }
    });
    std::array::from_fn(|i| {
        to_srgb(FROM_LMS[i][0] * lms[0] + FROM_LMS[i][1] * lms[1] + FROM_LMS[i][2] * lms[2])
    })
}

/// 2 つの色の知覚的な混ぜ（[`mix_roots`] を、色から根を作って呼ぶ）。
pub(crate) fn mix(a: [f64; 3], b: [f64; 3], t: f64, level: u8) -> [f64; 3] {
    mix_roots(a, b, roots(a), roots(b), t, level)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn byte(v: f64) -> i32 {
        v.round() as i32
    }

    fn rgb(v: [f64; 3]) -> [i32; 3] {
        v.map(byte)
    }

    #[test]
    #[allow(clippy::needless_range_loop)] // 3×3 の積の添字
    fn the_matrices_are_inverses_and_keep_white_white() {
        for i in 0..3 {
            for j in 0..3 {
                let v: f64 = (0..3).map(|k| FROM_LMS[i][k] * TO_LMS[k][j]).sum();
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((v - want).abs() < 1e-8, "{i} {j} {v}");
            }
        }
        let white = mix([255.0; 3], [255.0; 3], 0.5, 0);
        assert_eq!(rgb(white), [255, 255, 255]);
    }

    #[test]
    fn the_ends_are_not_mixed_and_a_same_colour_stays() {
        let (a, b) = ([12.0, 200.0, 99.0], [250.0, 3.0, 128.0]);
        for level in 0..=4 {
            assert_eq!(mix(a, b, 0.0, level), a);
            assert_eq!(mix(a, b, 1.0, level), b);
            // 同じ色どうしはどの重みでも同じ色（暗くならない）
            for t in [0.2, 0.5, 0.9] {
                let m = mix(a, a, t, level);
                for c in 0..3 {
                    assert!((m[c] - a[c]).abs() < 0.6, "{level} {t} {m:?}");
                }
            }
        }
    }

    #[test]
    fn blue_and_yellow_make_a_dark_green_where_the_standard_mix_is_grey() {
        let (blue, yellow) = ([0.0, 0.0, 255.0], [240.0, 208.0, 0.0]);
        let m = mix(blue, yellow, 0.6, 0);
        // 緑が一番強く、R は小さい
        assert!(m[1] > m[0] && m[1] > m[2], "{m:?}");
        assert!(m[0] < 60.0, "{m:?}");
        // 標準の混ぜ（sRGB の補間）は R・G・B が近い灰色
        let std = [144.0, 124.8, 102.0];
        let spread = |c: [f64; 3]| {
            c.iter().cloned().fold(f64::MIN, f64::max) - c.iter().cloned().fold(f64::MAX, f64::min)
        };
        assert!(spread(m) > spread(std) * 1.5, "{m:?}");
    }

    #[test]
    fn a_little_yellow_on_white_is_strongly_yellow() {
        let m = mix([255.0; 3], [240.0, 208.0, 0.0], 0.6, 0);
        // 標準の混ぜの青は 102。知覚的は白の反射がほとんど変わらないので、黄が強く出て青がもっと落ちる
        assert!(m[2] < 70.0, "{m:?}");
        assert!(m[0] > 235.0, "{m:?}");
    }

    #[test]
    fn a_stronger_correction_makes_the_mix_lighter() {
        let (blue, yellow) = ([0.0, 0.0, 255.0], [240.0, 208.0, 0.0]);
        let light = |m: [f64; 3]| m[0] * 0.2126 + m[1] * 0.7152 + m[2] * 0.0722;
        let mut last = -1.0;
        for level in 0..=4 {
            let l = light(mix(blue, yellow, 0.5, level));
            assert!(l > last, "{level} {l} {last}");
            last = l;
        }
        // 段の上限を超えた値は最高と同じ
        assert_eq!(mix(blue, yellow, 0.5, 9), mix(blue, yellow, 0.5, 4));
        // 端の色は補正で動かない
        assert_eq!(mix(blue, yellow, 0.0, 0), mix(blue, yellow, 0.0, 4));
    }

    #[test]
    fn a_heavier_weight_moves_the_mix_towards_that_colour() {
        let (a, b) = ([30.0, 60.0, 200.0], [220.0, 180.0, 20.0]);
        let mut last = f64::MIN;
        for k in 1..10 {
            let m = mix(a, b, f64::from(k) / 10.0, 4);
            assert!(m[0] > last, "{k}");
            last = m[0];
        }
    }

    #[test]
    fn fractional_values_interpolate_the_table() {
        let l = linear_of(100.5);
        assert!(l > to_linear(100) && l < to_linear(101));
        assert_eq!(linear_of(255.0), to_linear(255));
        assert_eq!(linear_of(-3.0), to_linear(0));
        assert_eq!(linear_of(300.0), to_linear(255));
    }
}
