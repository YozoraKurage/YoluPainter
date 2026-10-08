//! Normal のチャンネルの出力の行の核（SIMD）: 塗った法線の合成を平らな法線へ載せ、Height から作った法線を土台に RNM で重ねる。
//! 各画素は出力の 1 画素の式（`flatten` → Sobel → `rnm` → `encode`、[`super::output_pixel`]）と**同じバイト**になる。
//! 計算は f64 で（高さの差 × 強さ 256 までの微分を f64 の式のまま残す）、演算の順は画素ごとの式と同じ。正規化の平方根・割り算は
//! IEEE の厳密な演算（SSE/AVX の `sqrtpd`・`divpd`）なので同じ double になる。道の選びは `crate::math::simd`。
//! レイヤーの重ね（合成）は f32 の式で、[`super::rows`] にある。
#![cfg_attr(
    not(any(target_arch = "x86_64", target_arch = "aarch64")),
    allow(dead_code, unused_imports, unused_macros, unused_variables, unused_mut)
)]

use super::{height_of, DEGENERATE_LENGTH_SQUARED};
use crate::math::simd::{self, to_byte, Lanes, Level};

#[cfg(target_arch = "aarch64")]
use crate::math::simd::Neon;
#[cfg(target_arch = "x86_64")]
use crate::math::simd::{Avx2, Sse41};

type Vec3<V> = [<V as Lanes>::F; 3];

// ───────── レーンの式 ─────────

#[inline(always)]
unsafe fn normalize_lanes<V: Lanes>(v: Vec3<V>) -> Vec3<V> {
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let l2 = V::add(
        V::add(V::mul(v[0], v[0]), V::mul(v[1], v[1])),
        V::mul(v[2], v[2]),
    );
    let flat = V::lt(l2, V::splat(DEGENERATE_LENGTH_SQUARED));
    let l = V::sqrt(l2);
    [
        V::select(flat, zero, V::div(v[0], l)),
        V::select(flat, zero, V::div(v[1], l)),
        V::select(flat, one, V::div(v[2], l)),
    ]
}

/// 符号化した画素（R・G・B のレーン、0〜255 の整数）の単位ベクトル。
#[inline(always)]
unsafe fn decode_lanes<V: Lanes>(rgb: Vec3<V>) -> Vec3<V> {
    let (two, one) = (V::splat(2.0), V::splat(1.0));
    normalize_lanes::<V>([
        V::sub(V::mul(V::unit(rgb[0]), two), one),
        V::sub(V::mul(V::unit(rgb[1]), two), one),
        V::sub(V::mul(V::unit(rgb[2]), two), one),
    ])
}

/// ベクトルを正規化して符号化する（0〜255 の整数のレーン）。
#[inline(always)]
unsafe fn encode_lanes<V: Lanes>(v: Vec3<V>) -> Vec3<V> {
    let (half, n) = (V::splat(0.5), normalize_lanes::<V>(v));
    [
        to_byte::<V>(V::add(V::mul(n[0], half), half)),
        to_byte::<V>(V::add(V::mul(n[1], half), half)),
        to_byte::<V>(V::add(V::mul(n[2], half), half)),
    ]
}

#[inline(always)]
unsafe fn rnm_lanes<V: Lanes>(b: Vec3<V>, d: Vec3<V>) -> Vec3<V> {
    let (tx, ty, tz) = (b[0], b[1], V::add(b[2], V::splat(1.0)));
    let (ux, uy, uz) = (V::neg(d[0]), V::neg(d[1]), d[2]);
    let degenerate = V::le(tz, V::splat(1e-6));
    let dot = V::add(V::add(V::mul(tx, ux), V::mul(ty, uy)), V::mul(tz, uz));
    let k = V::div(dot, tz);
    [
        V::select(degenerate, b[0], V::sub(V::mul(tx, k), ux)),
        V::select(degenerate, b[1], V::sub(V::mul(ty, k), uy)),
        V::select(degenerate, b[2], V::sub(V::mul(tz, k), uz)),
    ]
}

// ───────── 出力の行（Height → Normal と塗った法線の重ね） ─────────

/// 高さの行 `row` の、画素 x + dx − 1 から N 個。
#[inline(always)]
unsafe fn height_lanes<V: Lanes>(h: &[f64], row: usize, x: usize, dx: usize) -> V::F {
    V::load_f64(&h[row + x + dx - 1..])
}

/// 出力の 1 画素のレーン版の入力: 塗った法線の合成の画素（なければ平ら）と、高さの 3 行（あれば）。
/// `x` は画素の位置（左端の画素の位置）。高さを読む区間は隣の画素を読むので x ≥ 1 かつ x + N ≤ w − 1 の範囲だけ。
#[inline(always)]
unsafe fn output_block<V: Lanes>(
    normal: Option<&[u8]>,
    heights: Option<&[f64]>,
    w: usize,
    strength: f64,
    x: usize,
    out: &mut [u8],
) {
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let p: Vec3<V> = match normal {
        Some(n) => {
            let px = V::load(&n[x * 4..]);
            let a = V::unit(px[3]);
            let d = decode_lanes::<V>([px[0], px[1], px[2]]);
            let flat = normalize_lanes::<V>([
                V::mul(d[0], a),
                V::mul(d[1], a),
                V::add(V::mul(d[2], a), V::sub(one, a)),
            ]);
            let transparent = V::eq(px[3], zero);
            [
                V::select(transparent, zero, flat[0]),
                V::select(transparent, zero, flat[1]),
                V::select(transparent, one, flat[2]),
            ]
        }
        None => [zero, zero, one],
    };
    let v = match heights {
        Some(h) => {
            let (below, center, above) = (0, w, 2 * w);
            // 0: 左（x − 1）、1: 中（x）、2: 右（x + 1）
            let (a_l, a_c, a_r) = (
                height_lanes::<V>(h, above, x, 0),
                height_lanes::<V>(h, above, x, 1),
                height_lanes::<V>(h, above, x, 2),
            );
            let (c_l, c_r) = (
                height_lanes::<V>(h, center, x, 0),
                height_lanes::<V>(h, center, x, 2),
            );
            let (b_l, b_c, b_r) = (
                height_lanes::<V>(h, below, x, 0),
                height_lanes::<V>(h, below, x, 1),
                height_lanes::<V>(h, below, x, 2),
            );
            let two = V::splat(2.0);
            let eight = V::splat(8.0);
            // (a_r + 2·c_r + b_r − a_l − 2·c_l − b_l) / 8
            let gx = V::div(
                V::sub(
                    V::sub(
                        V::sub(V::add(V::add(a_r, V::mul(two, c_r)), b_r), a_l),
                        V::mul(two, c_l),
                    ),
                    b_l,
                ),
                eight,
            );
            // (a_l + 2·a_c + a_r − b_l − 2·b_c − b_r) / 8
            let gy = V::div(
                V::sub(
                    V::sub(
                        V::sub(V::add(V::add(a_l, V::mul(two, a_c)), a_r), b_l),
                        V::mul(two, b_c),
                    ),
                    b_r,
                ),
                eight,
            );
            let s = V::splat(strength);
            let hv = normalize_lanes::<V>([V::mul(V::neg(s), gx), V::mul(V::neg(s), gy), one]);
            rnm_lanes::<V>(hv, p)
        }
        None => p,
    };
    let c = encode_lanes::<V>(v);
    V::store(&mut out[x * 4..], [c[0], c[1], c[2], V::splat(255.0)]);
}

/// 出力の 1 行のレーンで計算できる区間（`from` 以上の画素）を処理し、次の画素の位置を返す。
#[inline(always)]
unsafe fn output_row_lanes<V: Lanes>(
    normal: Option<&[u8]>,
    heights: Option<&[f64]>,
    w: usize,
    strength: f64,
    from: usize,
    out: &mut [u8],
) -> usize {
    let mut x = from;
    // 高さを読む行は、隣（x − 1 と x + N）が行の中にある区間だけ（端は端の規則の画素ごとの式）
    let (first, last) = if heights.is_some() {
        (x.max(1), w.saturating_sub(1))
    } else {
        (x, w)
    };
    x = first;
    while x + V::N <= last {
        output_block::<V>(normal, heights, w, strength, x, out);
        x += V::N;
    }
    x
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn output_row_avx2(
    normal: Option<&[u8]>,
    heights: Option<&[f64]>,
    w: usize,
    strength: f64,
    from: usize,
    out: &mut [u8],
) -> usize {
    output_row_lanes::<Avx2>(normal, heights, w, strength, from, out)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn output_row_sse41(
    normal: Option<&[u8]>,
    heights: Option<&[f64]>,
    w: usize,
    strength: f64,
    from: usize,
    out: &mut [u8],
) -> usize {
    output_row_lanes::<Sse41>(normal, heights, w, strength, from, out)
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn output_row_neon(
    normal: Option<&[u8]>,
    heights: Option<&[f64]>,
    w: usize,
    strength: f64,
    from: usize,
    out: &mut [u8],
) -> usize {
    output_row_lanes::<Neon>(normal, heights, w, strength, from, out)
}

/// 出力の行のうちレーンで処理できる画素を処理して、スカラーで処理する最初の画素の位置を返す（画素 0 は常にスカラー側）。
/// `[start, 次の位置)` が処理済み。高さが無い行は 0 から。
pub(super) fn output_row_at(
    level: Level,
    normal: Option<&[u8]>,
    heights: Option<&[f64]>,
    w: usize,
    strength: f64,
    out: &mut [u8],
) -> (usize, usize) {
    let start = usize::from(heights.is_some());
    let end = match level.min(simd::detect()) {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
        Level::Avx2 => unsafe { output_row_avx2(normal, heights, w, strength, 0, out) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { output_row_sse41(normal, heights, w, strength, 0, out) },
        #[cfg(target_arch = "aarch64")]
        // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
        Level::Neon => unsafe { output_row_neon(normal, heights, w, strength, 0, out) },
        Level::Scalar => start,
    };
    (start, end)
}

// ───────── 高さの読み出し ─────────

#[inline(always)]
unsafe fn heights_lanes<V: Lanes>(src: &[u8], out: &mut [f64]) -> usize {
    let count = out.len();
    let mut i = 0;
    while i + V::N <= count {
        let px = V::load(&src[i * 4..]);
        V::store_f64(
            &mut out[i..],
            V::div(V::mul(px[0], px[3]), V::splat(65025.0)),
        );
        i += V::N;
    }
    i
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn heights_avx2(src: &[u8], out: &mut [f64]) -> usize {
    heights_lanes::<Avx2>(src, out)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn heights_sse41(src: &[u8], out: &mut [f64]) -> usize {
    heights_lanes::<Sse41>(src, out)
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn heights_neon(src: &[u8], out: &mut [f64]) -> usize {
    heights_lanes::<Neon>(src, out)
}

/// 合成の行（RGBA）から高さ（R × A / 65025）の行を作る。
pub(super) fn heights_from_rgba(level: Level, src: &[u8], out: &mut [f64]) {
    let from = match level.min(simd::detect()) {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
        Level::Avx2 => unsafe { heights_avx2(src, out) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { heights_sse41(src, out) },
        #[cfg(target_arch = "aarch64")]
        // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
        Level::Neon => unsafe { heights_neon(src, out) },
        Level::Scalar => 0,
    };
    for (x, h) in out.iter_mut().enumerate().skip(from) {
        *h = height_of(src[x * 4], src[x * 4 + 3]);
    }
}

#[cfg(test)]
mod tests;
