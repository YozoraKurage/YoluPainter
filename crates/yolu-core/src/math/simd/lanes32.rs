//! f32 のレーン（AVX2 は 8 本・SSE4.1 と NEON は 4 本・スカラーは 1 本）の演算。合成の式（`crate::blend`）が使う。
//!
//! スカラーの道も 1 本のレーン [`Scalar1`] として同じトレイトを実装し、行の端の画素・SIMD の無い CPU・画素ごとの関数
//! （`blend::blend` など）も、SIMD の道と**同じ関数・同じ式・同じ順**を通る。演算は IEEE の四則・平方根・floor・比較・選択だけで、
//! どれも正しく丸められるので、道（スカラー・SSE4.1・AVX2・NEON）とスレッド数によらず同じ bit になる。積和の命令（FMA）と近似の逆数・
//! 平方根の逆数の命令は使わない（丸めの回数が道で変わるため）。
//!
//! 呼び出しの前提（CPU がその命令を持つこと）は f64 の [`super::Lanes`] と同じで、道ごとの `#[target_feature]` 付きの入口の
//! 中からだけ呼ぶ。[`Scalar1`] は前提を持たない（どこから呼んでもよい）。
#![allow(clippy::missing_safety_doc)]

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// f32 のレーンの演算。比較は Rust の f32 の比較と同じ（NaN は偽）で、`min`・`max` は `if a < b { a } else { b }`・
/// `if a > b { a } else { b }`（x86 の `minps`・`maxps` と同じ選び方。NEON は比較と選択で作る）。
pub(crate) trait Lanes32: Copy {
    /// レーンの本数。
    const N: usize;
    /// f32 のレーン。
    type F: Copy;
    /// 比較の結果（レーンごとに真か偽）。
    type M: Copy;

    unsafe fn splat(x: f32) -> Self::F;
    /// レーン k に f(k) を入れる（表引きなど、ベクトルの命令にならない値を集める）。k は 0 から順に呼ぶ。
    unsafe fn from_fn(f: impl FnMut(usize) -> f32) -> Self::F;
    /// レーン k の値（試験用）。
    #[cfg(test)]
    unsafe fn lane(v: Self::F, k: usize) -> f32;

    unsafe fn add(a: Self::F, b: Self::F) -> Self::F;
    unsafe fn sub(a: Self::F, b: Self::F) -> Self::F;
    unsafe fn mul(a: Self::F, b: Self::F) -> Self::F;
    unsafe fn div(a: Self::F, b: Self::F) -> Self::F;
    unsafe fn sqrt(a: Self::F) -> Self::F;
    /// `if a < b { a } else { b }`
    unsafe fn min(a: Self::F, b: Self::F) -> Self::F;
    /// `if a > b { a } else { b }`
    unsafe fn max(a: Self::F, b: Self::F) -> Self::F;
    unsafe fn floor(a: Self::F) -> Self::F;
    unsafe fn abs(a: Self::F) -> Self::F;
    /// 符号の反転（`-a`。0 の符号も反転する）。
    unsafe fn neg(a: Self::F) -> Self::F;

    unsafe fn lt(a: Self::F, b: Self::F) -> Self::M;
    unsafe fn le(a: Self::F, b: Self::F) -> Self::M;
    unsafe fn gt(a: Self::F, b: Self::F) -> Self::M;
    unsafe fn ge(a: Self::F, b: Self::F) -> Self::M;
    unsafe fn eq(a: Self::F, b: Self::F) -> Self::M;

    unsafe fn and(a: Self::M, b: Self::M) -> Self::M;
    unsafe fn or(a: Self::M, b: Self::M) -> Self::M;
    unsafe fn not(a: Self::M) -> Self::M;
    /// 真のレーンは a、偽のレーンは b。選ばれなかった方が NaN でも結果に影響しない。
    unsafe fn select(m: Self::M, a: Self::F, b: Self::F) -> Self::F;
    unsafe fn any(m: Self::M) -> bool;
    unsafe fn all(m: Self::M) -> bool;

    /// b / 255（割り算。b は 0〜255 の整数の値）。
    unsafe fn unit(b: Self::F) -> Self::F;

    /// 連続する N 画素の RGBA（4N バイト以上のスライスの先頭）を、R・G・B・A のレーンへ（値は 0〜255 の整数）。
    unsafe fn load(p: &[u8]) -> [Self::F; 4];
    /// 1 つの画素をすべてのレーンへ。
    unsafe fn splat_px(px: [u8; 4]) -> [Self::F; 4];
    /// R・G・B・A のレーン（0〜255 の整数の値）を連続する N 画素の RGBA（4N バイト以上のスライスの先頭）へ。
    unsafe fn store(p: &mut [u8], v: [Self::F; 4]);
}

/// 0〜1 の値を 0〜255 の整数の値へ（floor(v × 255 + 0.5) を 0〜255 に収める。NaN は 0）。
#[inline(always)]
pub(crate) unsafe fn to_byte32<V: Lanes32>(value: V::F) -> V::F {
    let v = V::add(V::mul(value, V::splat(255.0)), V::splat(0.5));
    // `max(v, 0)` は v ≤ 0 と NaN を 0 にし、`min(.., 255)` は 255 以上を 255 にする。正の v < 255 の floor は切り捨て
    V::floor(V::min(V::max(v, V::splat(0.0)), V::splat(255.0)))
}

/// `if v < 0 { 0 } else if v > 1 { 1 } else { v }`
#[inline(always)]
pub(crate) unsafe fn clamp01_32<V: Lanes32>(v: V::F) -> V::F {
    V::select(
        V::lt(v, V::splat(0.0)),
        V::splat(0.0),
        V::select(V::gt(v, V::splat(1.0)), V::splat(1.0), v),
    )
}

// ───────── スカラー（1 本） ─────────

/// 1 本のレーン（スカラーの道・行の端・画素ごとの関数）。CPU の前提を持たない。
#[derive(Clone, Copy)]
pub(crate) struct Scalar1;

impl Lanes32 for Scalar1 {
    const N: usize = 1;
    type F = f32;
    type M = bool;
    #[inline(always)]
    unsafe fn splat(x: f32) -> f32 {
        x
    }
    #[inline(always)]
    unsafe fn from_fn(mut f: impl FnMut(usize) -> f32) -> f32 {
        f(0)
    }
    #[cfg(test)]
    unsafe fn lane(v: f32, _k: usize) -> f32 {
        v
    }
    #[inline(always)]
    unsafe fn add(a: f32, b: f32) -> f32 {
        a + b
    }
    #[inline(always)]
    unsafe fn sub(a: f32, b: f32) -> f32 {
        a - b
    }
    #[inline(always)]
    unsafe fn mul(a: f32, b: f32) -> f32 {
        a * b
    }
    #[inline(always)]
    unsafe fn div(a: f32, b: f32) -> f32 {
        a / b
    }
    #[inline(always)]
    unsafe fn sqrt(a: f32) -> f32 {
        a.sqrt()
    }
    #[inline(always)]
    unsafe fn min(a: f32, b: f32) -> f32 {
        if a < b {
            a
        } else {
            b
        }
    }
    #[inline(always)]
    unsafe fn max(a: f32, b: f32) -> f32 {
        if a > b {
            a
        } else {
            b
        }
    }
    #[inline(always)]
    unsafe fn floor(a: f32) -> f32 {
        a.floor()
    }
    #[inline(always)]
    unsafe fn abs(a: f32) -> f32 {
        a.abs()
    }
    #[inline(always)]
    unsafe fn neg(a: f32) -> f32 {
        -a
    }
    #[inline(always)]
    unsafe fn lt(a: f32, b: f32) -> bool {
        a < b
    }
    #[inline(always)]
    unsafe fn le(a: f32, b: f32) -> bool {
        a <= b
    }
    #[inline(always)]
    unsafe fn gt(a: f32, b: f32) -> bool {
        a > b
    }
    #[inline(always)]
    unsafe fn ge(a: f32, b: f32) -> bool {
        a >= b
    }
    #[inline(always)]
    unsafe fn eq(a: f32, b: f32) -> bool {
        a == b
    }
    #[inline(always)]
    unsafe fn and(a: bool, b: bool) -> bool {
        a & b
    }
    #[inline(always)]
    unsafe fn or(a: bool, b: bool) -> bool {
        a | b
    }
    #[inline(always)]
    unsafe fn not(a: bool) -> bool {
        !a
    }
    #[inline(always)]
    unsafe fn select(m: bool, a: f32, b: f32) -> f32 {
        if m {
            a
        } else {
            b
        }
    }
    #[inline(always)]
    unsafe fn any(m: bool) -> bool {
        m
    }
    #[inline(always)]
    unsafe fn all(m: bool) -> bool {
        m
    }
    #[inline(always)]
    unsafe fn unit(b: f32) -> f32 {
        b / 255.0
    }
    #[inline(always)]
    unsafe fn load(p: &[u8]) -> [f32; 4] {
        [
            f32::from(p[0]),
            f32::from(p[1]),
            f32::from(p[2]),
            f32::from(p[3]),
        ]
    }
    #[inline(always)]
    unsafe fn splat_px(px: [u8; 4]) -> [f32; 4] {
        [
            f32::from(px[0]),
            f32::from(px[1]),
            f32::from(px[2]),
            f32::from(px[3]),
        ]
    }
    #[inline(always)]
    unsafe fn store(p: &mut [u8], v: [f32; 4]) {
        // 値は 0〜255 の整数（to_byte32 の結果か、読んだバイトそのもの）
        p[0] = v[0] as u8;
        p[1] = v[1] as u8;
        p[2] = v[2] as u8;
        p[3] = v[3] as u8;
    }
}

// ───────── AVX2（8 本） ─────────

/// AVX2 の f32 のレーン（8 本）。`avx2` を有効にした入口の中からだけ呼ぶ。
#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
pub(crate) struct Avx2x8;

#[cfg(target_arch = "x86_64")]
impl Lanes32 for Avx2x8 {
    const N: usize = 8;
    type F = __m256;
    type M = __m256;
    #[inline(always)]
    unsafe fn splat(x: f32) -> __m256 {
        _mm256_set1_ps(x)
    }
    #[inline(always)]
    unsafe fn from_fn(mut f: impl FnMut(usize) -> f32) -> __m256 {
        let v = [f(0), f(1), f(2), f(3), f(4), f(5), f(6), f(7)];
        _mm256_loadu_ps(v.as_ptr())
    }
    #[cfg(test)]
    unsafe fn lane(v: __m256, k: usize) -> f32 {
        let mut a = [0.0f32; 8];
        _mm256_storeu_ps(a.as_mut_ptr(), v);
        a[k]
    }
    #[inline(always)]
    unsafe fn add(a: __m256, b: __m256) -> __m256 {
        _mm256_add_ps(a, b)
    }
    #[inline(always)]
    unsafe fn sub(a: __m256, b: __m256) -> __m256 {
        _mm256_sub_ps(a, b)
    }
    #[inline(always)]
    unsafe fn mul(a: __m256, b: __m256) -> __m256 {
        _mm256_mul_ps(a, b)
    }
    #[inline(always)]
    unsafe fn div(a: __m256, b: __m256) -> __m256 {
        _mm256_div_ps(a, b)
    }
    #[inline(always)]
    unsafe fn sqrt(a: __m256) -> __m256 {
        _mm256_sqrt_ps(a)
    }
    #[inline(always)]
    unsafe fn min(a: __m256, b: __m256) -> __m256 {
        _mm256_min_ps(a, b)
    }
    #[inline(always)]
    unsafe fn max(a: __m256, b: __m256) -> __m256 {
        _mm256_max_ps(a, b)
    }
    #[inline(always)]
    unsafe fn floor(a: __m256) -> __m256 {
        _mm256_floor_ps(a)
    }
    #[inline(always)]
    unsafe fn abs(a: __m256) -> __m256 {
        _mm256_andnot_ps(_mm256_set1_ps(-0.0), a)
    }
    #[inline(always)]
    unsafe fn neg(a: __m256) -> __m256 {
        _mm256_xor_ps(_mm256_set1_ps(-0.0), a)
    }
    #[inline(always)]
    unsafe fn lt(a: __m256, b: __m256) -> __m256 {
        _mm256_cmp_ps::<_CMP_LT_OQ>(a, b)
    }
    #[inline(always)]
    unsafe fn le(a: __m256, b: __m256) -> __m256 {
        _mm256_cmp_ps::<_CMP_LE_OQ>(a, b)
    }
    #[inline(always)]
    unsafe fn gt(a: __m256, b: __m256) -> __m256 {
        _mm256_cmp_ps::<_CMP_GT_OQ>(a, b)
    }
    #[inline(always)]
    unsafe fn ge(a: __m256, b: __m256) -> __m256 {
        _mm256_cmp_ps::<_CMP_GE_OQ>(a, b)
    }
    #[inline(always)]
    unsafe fn eq(a: __m256, b: __m256) -> __m256 {
        _mm256_cmp_ps::<_CMP_EQ_OQ>(a, b)
    }
    #[inline(always)]
    unsafe fn and(a: __m256, b: __m256) -> __m256 {
        _mm256_and_ps(a, b)
    }
    #[inline(always)]
    unsafe fn or(a: __m256, b: __m256) -> __m256 {
        _mm256_or_ps(a, b)
    }
    #[inline(always)]
    unsafe fn not(a: __m256) -> __m256 {
        _mm256_xor_ps(a, _mm256_castsi256_ps(_mm256_set1_epi32(-1)))
    }
    #[inline(always)]
    unsafe fn select(m: __m256, a: __m256, b: __m256) -> __m256 {
        _mm256_blendv_ps(b, a, m)
    }
    #[inline(always)]
    unsafe fn any(m: __m256) -> bool {
        _mm256_movemask_ps(m) != 0
    }
    #[inline(always)]
    unsafe fn all(m: __m256) -> bool {
        _mm256_movemask_ps(m) == 0xFF
    }
    #[inline(always)]
    unsafe fn unit(b: __m256) -> __m256 {
        _mm256_div_ps(b, _mm256_set1_ps(255.0))
    }
    #[inline(always)]
    unsafe fn load(p: &[u8]) -> [__m256; 4] {
        let a: &[u8; 32] = p[..32].try_into().unwrap();
        let x = _mm256_loadu_si256(a.as_ptr().cast());
        let m = _mm256_set1_epi32(0xFF);
        [
            _mm256_cvtepi32_ps(_mm256_and_si256(x, m)),
            _mm256_cvtepi32_ps(_mm256_and_si256(_mm256_srli_epi32::<8>(x), m)),
            _mm256_cvtepi32_ps(_mm256_and_si256(_mm256_srli_epi32::<16>(x), m)),
            _mm256_cvtepi32_ps(_mm256_srli_epi32::<24>(x)),
        ]
    }
    #[inline(always)]
    unsafe fn splat_px(px: [u8; 4]) -> [__m256; 4] {
        [
            _mm256_set1_ps(f32::from(px[0])),
            _mm256_set1_ps(f32::from(px[1])),
            _mm256_set1_ps(f32::from(px[2])),
            _mm256_set1_ps(f32::from(px[3])),
        ]
    }
    #[inline(always)]
    unsafe fn store(p: &mut [u8], v: [__m256; 4]) {
        let out: &mut [u8; 32] = (&mut p[..32]).try_into().unwrap();
        // 値は 0〜255 の整数なので、切り捨ての変換で同じ値になる
        let r = _mm256_cvttps_epi32(v[0]);
        let g = _mm256_cvttps_epi32(v[1]);
        let b = _mm256_cvttps_epi32(v[2]);
        let a = _mm256_cvttps_epi32(v[3]);
        let rg = _mm256_or_si256(r, _mm256_slli_epi32::<8>(g));
        let ba = _mm256_or_si256(_mm256_slli_epi32::<16>(b), _mm256_slli_epi32::<24>(a));
        _mm256_storeu_si256(out.as_mut_ptr().cast(), _mm256_or_si256(rg, ba));
    }
}

// ───────── SSE4.1（4 本） ─────────

/// SSE4.1 の f32 のレーン（4 本）。`sse4.1` を有効にした入口の中からだけ呼ぶ。
#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
pub(crate) struct Sse41x4;

#[cfg(target_arch = "x86_64")]
impl Lanes32 for Sse41x4 {
    const N: usize = 4;
    type F = __m128;
    type M = __m128;
    #[inline(always)]
    unsafe fn splat(x: f32) -> __m128 {
        _mm_set1_ps(x)
    }
    #[inline(always)]
    unsafe fn from_fn(mut f: impl FnMut(usize) -> f32) -> __m128 {
        let v = [f(0), f(1), f(2), f(3)];
        _mm_loadu_ps(v.as_ptr())
    }
    #[cfg(test)]
    unsafe fn lane(v: __m128, k: usize) -> f32 {
        let mut a = [0.0f32; 4];
        _mm_storeu_ps(a.as_mut_ptr(), v);
        a[k]
    }
    #[inline(always)]
    unsafe fn add(a: __m128, b: __m128) -> __m128 {
        _mm_add_ps(a, b)
    }
    #[inline(always)]
    unsafe fn sub(a: __m128, b: __m128) -> __m128 {
        _mm_sub_ps(a, b)
    }
    #[inline(always)]
    unsafe fn mul(a: __m128, b: __m128) -> __m128 {
        _mm_mul_ps(a, b)
    }
    #[inline(always)]
    unsafe fn div(a: __m128, b: __m128) -> __m128 {
        _mm_div_ps(a, b)
    }
    #[inline(always)]
    unsafe fn sqrt(a: __m128) -> __m128 {
        _mm_sqrt_ps(a)
    }
    #[inline(always)]
    unsafe fn min(a: __m128, b: __m128) -> __m128 {
        _mm_min_ps(a, b)
    }
    #[inline(always)]
    unsafe fn max(a: __m128, b: __m128) -> __m128 {
        _mm_max_ps(a, b)
    }
    #[inline(always)]
    unsafe fn floor(a: __m128) -> __m128 {
        _mm_floor_ps(a)
    }
    #[inline(always)]
    unsafe fn abs(a: __m128) -> __m128 {
        _mm_andnot_ps(_mm_set1_ps(-0.0), a)
    }
    #[inline(always)]
    unsafe fn neg(a: __m128) -> __m128 {
        _mm_xor_ps(_mm_set1_ps(-0.0), a)
    }
    #[inline(always)]
    unsafe fn lt(a: __m128, b: __m128) -> __m128 {
        _mm_cmplt_ps(a, b)
    }
    #[inline(always)]
    unsafe fn le(a: __m128, b: __m128) -> __m128 {
        _mm_cmple_ps(a, b)
    }
    #[inline(always)]
    unsafe fn gt(a: __m128, b: __m128) -> __m128 {
        _mm_cmpgt_ps(a, b)
    }
    #[inline(always)]
    unsafe fn ge(a: __m128, b: __m128) -> __m128 {
        _mm_cmpge_ps(a, b)
    }
    #[inline(always)]
    unsafe fn eq(a: __m128, b: __m128) -> __m128 {
        _mm_cmpeq_ps(a, b)
    }
    #[inline(always)]
    unsafe fn and(a: __m128, b: __m128) -> __m128 {
        _mm_and_ps(a, b)
    }
    #[inline(always)]
    unsafe fn or(a: __m128, b: __m128) -> __m128 {
        _mm_or_ps(a, b)
    }
    #[inline(always)]
    unsafe fn not(a: __m128) -> __m128 {
        _mm_xor_ps(a, _mm_castsi128_ps(_mm_set1_epi32(-1)))
    }
    #[inline(always)]
    unsafe fn select(m: __m128, a: __m128, b: __m128) -> __m128 {
        _mm_blendv_ps(b, a, m)
    }
    #[inline(always)]
    unsafe fn any(m: __m128) -> bool {
        _mm_movemask_ps(m) != 0
    }
    #[inline(always)]
    unsafe fn all(m: __m128) -> bool {
        _mm_movemask_ps(m) == 0xF
    }
    #[inline(always)]
    unsafe fn unit(b: __m128) -> __m128 {
        _mm_div_ps(b, _mm_set1_ps(255.0))
    }
    #[inline(always)]
    unsafe fn load(p: &[u8]) -> [__m128; 4] {
        let a: &[u8; 16] = p[..16].try_into().unwrap();
        let x = _mm_loadu_si128(a.as_ptr().cast());
        let m = _mm_set1_epi32(0xFF);
        [
            _mm_cvtepi32_ps(_mm_and_si128(x, m)),
            _mm_cvtepi32_ps(_mm_and_si128(_mm_srli_epi32::<8>(x), m)),
            _mm_cvtepi32_ps(_mm_and_si128(_mm_srli_epi32::<16>(x), m)),
            _mm_cvtepi32_ps(_mm_srli_epi32::<24>(x)),
        ]
    }
    #[inline(always)]
    unsafe fn splat_px(px: [u8; 4]) -> [__m128; 4] {
        [
            _mm_set1_ps(f32::from(px[0])),
            _mm_set1_ps(f32::from(px[1])),
            _mm_set1_ps(f32::from(px[2])),
            _mm_set1_ps(f32::from(px[3])),
        ]
    }
    #[inline(always)]
    unsafe fn store(p: &mut [u8], v: [__m128; 4]) {
        let out: &mut [u8; 16] = (&mut p[..16]).try_into().unwrap();
        let r = _mm_cvttps_epi32(v[0]);
        let g = _mm_cvttps_epi32(v[1]);
        let b = _mm_cvttps_epi32(v[2]);
        let a = _mm_cvttps_epi32(v[3]);
        let rg = _mm_or_si128(r, _mm_slli_epi32::<8>(g));
        let ba = _mm_or_si128(_mm_slli_epi32::<16>(b), _mm_slli_epi32::<24>(a));
        _mm_storeu_si128(out.as_mut_ptr().cast(), _mm_or_si128(rg, ba));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::simd::tests::Rng;

    /// 2 つの値の組（8 bit の値・その割り算・乱数・端の値）の並び。
    fn operands() -> Vec<(f32, f32)> {
        let mut rng = Rng(32);
        let mut v = Vec::new();
        for a in (0..256).step_by(3) {
            for b in (0..256).step_by(5) {
                v.push((a as f32 / 255.0, b as f32 / 255.0));
                v.push((a as f32, b as f32 / 255.0));
            }
        }
        let edges = [
            0.0f32,
            -0.0,
            1.0,
            0.5,
            1e-30,
            f32::MIN_POSITIVE,
            1e-40,
            255.0,
            254.5,
            -1.0,
            f32::INFINITY,
            f32::NAN,
            // 前提（0〜255 の整数・0〜1）の外: 負の値、255 や 2³¹ を超える値、整数に丸めたとき端になる値、f32 の端、NaN の別の形
            f32::NEG_INFINITY,
            -255.0,
            256.0,
            65536.0,
            2_147_483_520.0,
            2_147_483_648.0,
            -2_147_483_648.0,
            -2_147_483_904.0,
            8_388_607.5,
            8_388_608.0,
            -8_388_607.5,
            f32::MAX,
            f32::MIN,
            -f32::MIN_POSITIVE,
            f32::from_bits(0xFFC0_0000),
            f32::from_bits(0x7FC0_5EED),
            f32::from_bits(0x7F80_0001),
        ];
        for &a in &edges {
            for &b in &edges {
                v.push((a, b));
            }
        }
        for _ in 0..20_000 {
            v.push((rng.unit() as f32 * 4.0 - 2.0, rng.unit() as f32 * 4.0 - 2.0));
        }
        v
    }

    /// 演算 1 つずつを、道（V）と 1 本のレーンで比べる（bit まで同じ。NaN は NaN どうし）。
    #[allow(clippy::needless_range_loop)]
    unsafe fn every_op_matches_the_scalar<V: Lanes32>() {
        let ops = operands();
        let same = |x: f32, y: f32| x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan());
        for chunk in ops.chunks(V::N) {
            if chunk.len() < V::N {
                break;
            }
            let a = V::from_fn(|k| chunk[k].0);
            let b = V::from_fn(|k| chunk[k].1);
            let pick = |m: V::M| V::select(m, V::splat(1.0), V::splat(0.0));
            let lanes: [(&str, V::F); 18] = [
                ("add", V::add(a, b)),
                ("sub", V::sub(a, b)),
                ("mul", V::mul(a, b)),
                ("div", V::div(a, b)),
                ("sqrt", V::sqrt(a)),
                ("min", V::min(a, b)),
                ("max", V::max(a, b)),
                ("floor", V::floor(a)),
                ("abs", V::abs(a)),
                ("neg", V::neg(a)),
                ("lt", pick(V::lt(a, b))),
                ("le", pick(V::le(a, b))),
                ("gt", pick(V::gt(a, b))),
                ("ge", pick(V::ge(a, b))),
                ("eq", pick(V::eq(a, b))),
                ("unit", V::unit(V::floor(V::abs(a)))),
                ("to_byte", to_byte32::<V>(a)),
                ("clamp01", clamp01_32::<V>(a)),
            ];
            for k in 0..V::N {
                let (x, y) = chunk[k];
                let s = |m: bool| if m { 1.0 } else { 0.0 };
                let want: [f32; 18] = [
                    Scalar1::add(x, y),
                    Scalar1::sub(x, y),
                    Scalar1::mul(x, y),
                    Scalar1::div(x, y),
                    Scalar1::sqrt(x),
                    Scalar1::min(x, y),
                    Scalar1::max(x, y),
                    Scalar1::floor(x),
                    Scalar1::abs(x),
                    Scalar1::neg(x),
                    s(Scalar1::lt(x, y)),
                    s(Scalar1::le(x, y)),
                    s(Scalar1::gt(x, y)),
                    s(Scalar1::ge(x, y)),
                    s(Scalar1::eq(x, y)),
                    Scalar1::unit(Scalar1::floor(Scalar1::abs(x))),
                    to_byte32::<Scalar1>(x),
                    clamp01_32::<Scalar1>(x),
                ];
                for (i, (name, got)) in lanes.iter().enumerate() {
                    let got = V::lane(*got, k);
                    assert!(same(got, want[i]), "{name}({x}, {y}): {got} != {}", want[i]);
                }
            }
        }
    }

    #[test]
    fn every_lane_op_matches_the_single_lane_on_every_level() {
        crate::math::simd::on_each_level32!(every_op_matches_the_scalar);
    }

    unsafe fn pixels_round_trip<V: Lanes32>() {
        let mut rng = Rng(33);
        for _ in 0..2_000 {
            let bytes: Vec<u8> = (0..V::N * 4).map(|_| rng.byte()).collect();
            let mut out = vec![0u8; V::N * 4];
            V::store(&mut out, V::load(&bytes));
            assert_eq!(out, bytes);
            let px = [bytes[0], bytes[1], bytes[2], bytes[3]];
            let mut out = vec![0u8; V::N * 4];
            V::store(&mut out, V::splat_px(px));
            assert!(out.chunks_exact(4).all(|c| c == px));
        }
    }

    #[test]
    fn pixels_load_and_store_unchanged_on_every_level() {
        crate::math::simd::on_each_level32!(pixels_round_trip);
    }
}
