//! x86_64 のレーン: AVX2 + FMA（f64 が 4 本）と SSE4.1（2 本）。
//!
//! 各メソッドは `unsafe fn` で、前提は「CPU がその命令を持つ」こと（[`Lanes`] の `# Safety`）。メソッドの中の `unsafe` ブロックは
//! 組み込み関数の呼び出しだけ。型 [`Avx2`]・[`Sse41`] を使う関数は `#[target_feature(enable = "avx2,fma")]`・
//! `#[target_feature(enable = "sse4.1")]` 付きの `unsafe fn` の入口の中にだけあり、入口は [`super::level`] が返した道でだけ呼ばれる。
//! 画素の読み書きは長さを確かめた固定長の配列へ変換してから行うので、範囲外を読み書きしない。
#![deny(unsafe_op_in_unsafe_fn)]

use core::arch::x86_64::*;

use super::Lanes;

/// 1 / 255 を 2 つの double の和で持つ（上の桁と、その残り）。b × (上 + 残り) を FMA で 1 回だけ丸めると、b / 255.0 を割り算で丸めた値と
/// 0〜255 のすべての b で同じ bit になる（試験 `unit_matches_the_division_for_every_byte`）。
const RECIPROCAL_HIGH: f64 = f64::from_bits(0x3f70_1010_1010_1010);
const RECIPROCAL_LOW: f64 = f64::from_bits(0x3bf0_1010_1010_1010);

/// RGBA の 1 チャンネル（0〜3）を、4 つの画素から i32 の 4 本へ取り出すバイトの並び（0x80 のバイトは 0 になる）。
macro_rules! channel_mask4 {
    ($c:expr) => {
        _mm_setr_epi8(
            $c,
            -1,
            -1,
            -1,
            $c + 4,
            -1,
            -1,
            -1,
            $c + 8,
            -1,
            -1,
            -1,
            $c + 12,
            -1,
            -1,
            -1,
        )
    };
}
macro_rules! channel_mask2 {
    ($c:expr) => {
        _mm_setr_epi8(
            $c,
            -1,
            -1,
            -1,
            $c + 4,
            -1,
            -1,
            -1,
            -1,
            -1,
            -1,
            -1,
            -1,
            -1,
            -1,
            -1,
        )
    };
}

/// u16 の RGBA が 2 画素入った 128 ビットから、チャンネル c を 2 つの i32 にする並び（下位 2 本に入り、上は 0）。
macro_rules! channel_mask_u16 {
    ($c:expr) => {
        _mm_setr_epi8(
            2 * $c,
            2 * $c + 1,
            -1,
            -1,
            8 + 2 * $c,
            9 + 2 * $c,
            -1,
            -1,
            -1,
            -1,
            -1,
            -1,
            -1,
            -1,
            -1,
            -1,
        )
    };
}

/// 2 画素ずつ 2 つの 128 ビット（lo = 画素 0・1、hi = 画素 2・3）から、チャンネルの 4 本の f64 を作る。
#[inline(always)]
unsafe fn u16_channel4(lo: __m128i, hi: __m128i, mask: __m128i) -> __m256d {
    unsafe {
        let a = _mm_shuffle_epi8(lo, mask);
        let b = _mm_shuffle_epi8(hi, mask);
        _mm256_cvtepi32_pd(_mm_unpacklo_epi64(a, b))
    }
}

// ───────── AVX2 + FMA（4 本） ─────────

/// AVX2 と FMA を持つ CPU 用（f64 が 4 本）。
#[derive(Clone, Copy)]
pub(crate) struct Avx2;

impl Lanes for Avx2 {
    const N: usize = 4;
    type F = __m256d;
    type M = __m256d;

    #[inline(always)]
    unsafe fn splat(x: f64) -> __m256d {
        unsafe { _mm256_set1_pd(x) }
    }
    #[inline(always)]
    unsafe fn from_fn(mut f: impl FnMut(usize) -> f64) -> __m256d {
        let (a, b, c, d) = (f(0), f(1), f(2), f(3));
        unsafe { _mm256_setr_pd(a, b, c, d) }
    }
    #[cfg(test)]
    #[inline(always)]
    unsafe fn lane(v: __m256d, k: usize) -> f64 {
        let mut out = [0.0f64; 4];
        unsafe { _mm256_storeu_pd(out.as_mut_ptr(), v) };
        out[k]
    }

    #[inline(always)]
    unsafe fn add(a: __m256d, b: __m256d) -> __m256d {
        unsafe { _mm256_add_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn sub(a: __m256d, b: __m256d) -> __m256d {
        unsafe { _mm256_sub_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn mul(a: __m256d, b: __m256d) -> __m256d {
        unsafe { _mm256_mul_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn div(a: __m256d, b: __m256d) -> __m256d {
        unsafe { _mm256_div_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn sqrt(a: __m256d) -> __m256d {
        unsafe { _mm256_sqrt_pd(a) }
    }
    #[inline(always)]
    unsafe fn min(a: __m256d, b: __m256d) -> __m256d {
        unsafe { _mm256_min_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn max(a: __m256d, b: __m256d) -> __m256d {
        unsafe { _mm256_max_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn floor(a: __m256d) -> __m256d {
        unsafe { _mm256_floor_pd(a) }
    }
    #[inline(always)]
    unsafe fn abs(a: __m256d) -> __m256d {
        unsafe { _mm256_andnot_pd(_mm256_set1_pd(-0.0), a) }
    }
    #[inline(always)]
    unsafe fn neg(a: __m256d) -> __m256d {
        unsafe { _mm256_xor_pd(a, _mm256_set1_pd(-0.0)) }
    }

    #[inline(always)]
    unsafe fn lt(a: __m256d, b: __m256d) -> __m256d {
        unsafe { _mm256_cmp_pd::<_CMP_LT_OQ>(a, b) }
    }
    #[inline(always)]
    unsafe fn le(a: __m256d, b: __m256d) -> __m256d {
        unsafe { _mm256_cmp_pd::<_CMP_LE_OQ>(a, b) }
    }
    #[inline(always)]
    unsafe fn gt(a: __m256d, b: __m256d) -> __m256d {
        unsafe { _mm256_cmp_pd::<_CMP_GT_OQ>(a, b) }
    }
    #[inline(always)]
    unsafe fn ge(a: __m256d, b: __m256d) -> __m256d {
        unsafe { _mm256_cmp_pd::<_CMP_GE_OQ>(a, b) }
    }
    #[inline(always)]
    unsafe fn eq(a: __m256d, b: __m256d) -> __m256d {
        unsafe { _mm256_cmp_pd::<_CMP_EQ_OQ>(a, b) }
    }

    #[inline(always)]
    unsafe fn and(a: __m256d, b: __m256d) -> __m256d {
        unsafe { _mm256_and_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn or(a: __m256d, b: __m256d) -> __m256d {
        unsafe { _mm256_or_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn not(a: __m256d) -> __m256d {
        unsafe { _mm256_xor_pd(a, _mm256_castsi256_pd(_mm256_set1_epi64x(-1))) }
    }
    #[inline(always)]
    unsafe fn select(m: __m256d, a: __m256d, b: __m256d) -> __m256d {
        unsafe { _mm256_blendv_pd(b, a, m) }
    }
    #[inline(always)]
    unsafe fn any(m: __m256d) -> bool {
        unsafe { _mm256_movemask_pd(m) != 0 }
    }
    #[inline(always)]
    unsafe fn all(m: __m256d) -> bool {
        unsafe { _mm256_movemask_pd(m) == 0b1111 }
    }

    #[inline(always)]
    unsafe fn unit(b: __m256d) -> __m256d {
        unsafe {
            _mm256_fmadd_pd(
                b,
                _mm256_set1_pd(RECIPROCAL_HIGH),
                _mm256_mul_pd(b, _mm256_set1_pd(RECIPROCAL_LOW)),
            )
        }
    }

    #[inline(always)]
    unsafe fn load(p: &[u8]) -> [__m256d; 4] {
        let a: &[u8; 16] = p[..16].try_into().unwrap();
        unsafe {
            let x = _mm_loadu_si128(a.as_ptr().cast());
            [
                _mm256_cvtepi32_pd(_mm_shuffle_epi8(x, channel_mask4!(0))),
                _mm256_cvtepi32_pd(_mm_shuffle_epi8(x, channel_mask4!(1))),
                _mm256_cvtepi32_pd(_mm_shuffle_epi8(x, channel_mask4!(2))),
                _mm256_cvtepi32_pd(_mm_shuffle_epi8(x, channel_mask4!(3))),
            ]
        }
    }
    #[inline(always)]
    unsafe fn load_f64(p: &[f64]) -> __m256d {
        let a: &[f64; 4] = p[..4].try_into().unwrap();
        unsafe { _mm256_loadu_pd(a.as_ptr()) }
    }
    #[inline(always)]
    unsafe fn store_f64(p: &mut [f64], v: __m256d) {
        let out: &mut [f64; 4] = (&mut p[..4]).try_into().unwrap();
        unsafe { _mm256_storeu_pd(out.as_mut_ptr(), v) }
    }
    #[inline(always)]
    unsafe fn load_u16(p: &[u16]) -> __m256d {
        let a: &[u16; 4] = p[..4].try_into().unwrap();
        unsafe { _mm256_cvtepi32_pd(_mm_cvtepu16_epi32(_mm_loadl_epi64(a.as_ptr().cast()))) }
    }
    #[inline(always)]
    unsafe fn store_u16(p: &mut [u16], v: __m256d) {
        let out: &mut [u16; 4] = (&mut p[..4]).try_into().unwrap();
        unsafe {
            let i = _mm256_cvttpd_epi32(v);
            _mm_storel_epi64(out.as_mut_ptr().cast(), _mm_packus_epi32(i, i));
        }
    }
    #[inline(always)]
    unsafe fn load_u32(p: &[u32]) -> __m256d {
        let a: &[u32; 4] = p[..4].try_into().unwrap();
        unsafe { _mm256_cvtepi32_pd(_mm_loadu_si128(a.as_ptr().cast())) }
    }
    #[inline(always)]
    unsafe fn store_u32(p: &mut [u32], v: __m256d) {
        let out: &mut [u32; 4] = (&mut p[..4]).try_into().unwrap();
        unsafe { _mm_storeu_si128(out.as_mut_ptr().cast(), _mm256_cvttpd_epi32(v)) }
    }
    #[inline(always)]
    unsafe fn load_u16x4(p: &[u16]) -> [__m256d; 4] {
        let a: &[u16; 16] = p[..16].try_into().unwrap();
        unsafe {
            let lo = _mm_loadu_si128(a.as_ptr().cast());
            let hi = _mm_loadu_si128(a.as_ptr().add(8).cast());
            [
                u16_channel4(lo, hi, channel_mask_u16!(0)),
                u16_channel4(lo, hi, channel_mask_u16!(1)),
                u16_channel4(lo, hi, channel_mask_u16!(2)),
                u16_channel4(lo, hi, channel_mask_u16!(3)),
            ]
        }
    }
    #[inline(always)]
    unsafe fn store_u16x4(p: &mut [u16], v: [__m256d; 4]) {
        let out: &mut [u16; 16] = (&mut p[..16]).try_into().unwrap();
        unsafe {
            let r = _mm256_cvttpd_epi32(v[0]);
            let g = _mm256_cvttpd_epi32(v[1]);
            let b = _mm256_cvttpd_epi32(v[2]);
            let a = _mm256_cvttpd_epi32(v[3]);
            // 各 32 ビットに R | G << 16 と B | A << 16。交互に並べると u16 で R G B A R G B A … になる
            let rg = _mm_or_si128(r, _mm_slli_epi32::<16>(g));
            let ba = _mm_or_si128(b, _mm_slli_epi32::<16>(a));
            _mm_storeu_si128(out.as_mut_ptr().cast(), _mm_unpacklo_epi32(rg, ba));
            _mm_storeu_si128(out.as_mut_ptr().add(8).cast(), _mm_unpackhi_epi32(rg, ba));
        }
    }
    #[inline(always)]
    unsafe fn splat_px(px: [u8; 4]) -> [__m256d; 4] {
        unsafe {
            [
                Self::splat(f64::from(px[0])),
                Self::splat(f64::from(px[1])),
                Self::splat(f64::from(px[2])),
                Self::splat(f64::from(px[3])),
            ]
        }
    }
    #[inline(always)]
    unsafe fn store(p: &mut [u8], v: [__m256d; 4]) {
        let out: &mut [u8; 16] = (&mut p[..16]).try_into().unwrap();
        unsafe {
            let r = _mm256_cvttpd_epi32(v[0]);
            let g = _mm256_cvttpd_epi32(v[1]);
            let b = _mm256_cvttpd_epi32(v[2]);
            let a = _mm256_cvttpd_epi32(v[3]);
            let rg = _mm_or_si128(r, _mm_slli_epi32::<8>(g));
            let ba = _mm_or_si128(_mm_slli_epi32::<16>(b), _mm_slli_epi32::<24>(a));
            _mm_storeu_si128(out.as_mut_ptr().cast(), _mm_or_si128(rg, ba));
        }
    }
}

// ───────── SSE4.1（2 本） ─────────

/// SSE4.1 を持つ CPU 用（f64 が 2 本）。
#[derive(Clone, Copy)]
pub(crate) struct Sse41;

impl Lanes for Sse41 {
    const N: usize = 2;
    type F = __m128d;
    type M = __m128d;

    #[inline(always)]
    unsafe fn splat(x: f64) -> __m128d {
        unsafe { _mm_set1_pd(x) }
    }
    #[inline(always)]
    unsafe fn from_fn(mut f: impl FnMut(usize) -> f64) -> __m128d {
        let (a, b) = (f(0), f(1));
        unsafe { _mm_setr_pd(a, b) }
    }
    #[cfg(test)]
    #[inline(always)]
    unsafe fn lane(v: __m128d, k: usize) -> f64 {
        let mut out = [0.0f64; 2];
        unsafe { _mm_storeu_pd(out.as_mut_ptr(), v) };
        out[k]
    }

    #[inline(always)]
    unsafe fn add(a: __m128d, b: __m128d) -> __m128d {
        unsafe { _mm_add_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn sub(a: __m128d, b: __m128d) -> __m128d {
        unsafe { _mm_sub_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn mul(a: __m128d, b: __m128d) -> __m128d {
        unsafe { _mm_mul_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn div(a: __m128d, b: __m128d) -> __m128d {
        unsafe { _mm_div_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn sqrt(a: __m128d) -> __m128d {
        unsafe { _mm_sqrt_pd(a) }
    }
    #[inline(always)]
    unsafe fn min(a: __m128d, b: __m128d) -> __m128d {
        unsafe { _mm_min_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn max(a: __m128d, b: __m128d) -> __m128d {
        unsafe { _mm_max_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn floor(a: __m128d) -> __m128d {
        unsafe { _mm_floor_pd(a) }
    }
    #[inline(always)]
    unsafe fn abs(a: __m128d) -> __m128d {
        unsafe { _mm_andnot_pd(_mm_set1_pd(-0.0), a) }
    }
    #[inline(always)]
    unsafe fn neg(a: __m128d) -> __m128d {
        unsafe { _mm_xor_pd(a, _mm_set1_pd(-0.0)) }
    }

    #[inline(always)]
    unsafe fn lt(a: __m128d, b: __m128d) -> __m128d {
        unsafe { _mm_cmplt_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn le(a: __m128d, b: __m128d) -> __m128d {
        unsafe { _mm_cmple_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn gt(a: __m128d, b: __m128d) -> __m128d {
        unsafe { _mm_cmpgt_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn ge(a: __m128d, b: __m128d) -> __m128d {
        unsafe { _mm_cmpge_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn eq(a: __m128d, b: __m128d) -> __m128d {
        unsafe { _mm_cmpeq_pd(a, b) }
    }

    #[inline(always)]
    unsafe fn and(a: __m128d, b: __m128d) -> __m128d {
        unsafe { _mm_and_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn or(a: __m128d, b: __m128d) -> __m128d {
        unsafe { _mm_or_pd(a, b) }
    }
    #[inline(always)]
    unsafe fn not(a: __m128d) -> __m128d {
        unsafe { _mm_xor_pd(a, _mm_castsi128_pd(_mm_set1_epi64x(-1))) }
    }
    #[inline(always)]
    unsafe fn select(m: __m128d, a: __m128d, b: __m128d) -> __m128d {
        unsafe { _mm_blendv_pd(b, a, m) }
    }
    #[inline(always)]
    unsafe fn any(m: __m128d) -> bool {
        unsafe { _mm_movemask_pd(m) != 0 }
    }
    #[inline(always)]
    unsafe fn all(m: __m128d) -> bool {
        unsafe { _mm_movemask_pd(m) == 0b11 }
    }

    #[inline(always)]
    unsafe fn unit(b: __m128d) -> __m128d {
        unsafe { _mm_div_pd(b, _mm_set1_pd(255.0)) }
    }

    #[inline(always)]
    unsafe fn load(p: &[u8]) -> [__m128d; 4] {
        let a: &[u8; 8] = p[..8].try_into().unwrap();
        unsafe {
            let x = _mm_loadl_epi64(a.as_ptr().cast());
            [
                _mm_cvtepi32_pd(_mm_shuffle_epi8(x, channel_mask2!(0))),
                _mm_cvtepi32_pd(_mm_shuffle_epi8(x, channel_mask2!(1))),
                _mm_cvtepi32_pd(_mm_shuffle_epi8(x, channel_mask2!(2))),
                _mm_cvtepi32_pd(_mm_shuffle_epi8(x, channel_mask2!(3))),
            ]
        }
    }
    #[inline(always)]
    unsafe fn load_f64(p: &[f64]) -> __m128d {
        let a: &[f64; 2] = p[..2].try_into().unwrap();
        unsafe { _mm_loadu_pd(a.as_ptr()) }
    }
    #[inline(always)]
    unsafe fn store_f64(p: &mut [f64], v: __m128d) {
        let out: &mut [f64; 2] = (&mut p[..2]).try_into().unwrap();
        unsafe { _mm_storeu_pd(out.as_mut_ptr(), v) }
    }
    #[inline(always)]
    unsafe fn load_u16(p: &[u16]) -> __m128d {
        let a: &[u16; 2] = p[..2].try_into().unwrap();
        let both = u32::from(a[0]) | (u32::from(a[1]) << 16);
        unsafe { _mm_cvtepi32_pd(_mm_cvtepu16_epi32(_mm_cvtsi32_si128(both as i32))) }
    }
    #[inline(always)]
    unsafe fn store_u16(p: &mut [u16], v: __m128d) {
        let out: &mut [u16; 2] = (&mut p[..2]).try_into().unwrap();
        unsafe {
            let i = _mm_cvttpd_epi32(v);
            let both = _mm_cvtsi128_si32(_mm_packus_epi32(i, i)) as u32;
            out[0] = both as u16;
            out[1] = (both >> 16) as u16;
        }
    }
    #[inline(always)]
    unsafe fn load_u32(p: &[u32]) -> __m128d {
        let a: &[u32; 2] = p[..2].try_into().unwrap();
        unsafe { _mm_cvtepi32_pd(_mm_loadl_epi64(a.as_ptr().cast())) }
    }
    #[inline(always)]
    unsafe fn store_u32(p: &mut [u32], v: __m128d) {
        let out: &mut [u32; 2] = (&mut p[..2]).try_into().unwrap();
        unsafe { _mm_storel_epi64(out.as_mut_ptr().cast(), _mm_cvttpd_epi32(v)) }
    }
    #[inline(always)]
    unsafe fn load_u16x4(p: &[u16]) -> [__m128d; 4] {
        let a: &[u16; 8] = p[..8].try_into().unwrap();
        unsafe {
            let x = _mm_loadu_si128(a.as_ptr().cast());
            [
                _mm_cvtepi32_pd(_mm_shuffle_epi8(x, channel_mask_u16!(0))),
                _mm_cvtepi32_pd(_mm_shuffle_epi8(x, channel_mask_u16!(1))),
                _mm_cvtepi32_pd(_mm_shuffle_epi8(x, channel_mask_u16!(2))),
                _mm_cvtepi32_pd(_mm_shuffle_epi8(x, channel_mask_u16!(3))),
            ]
        }
    }
    #[inline(always)]
    unsafe fn store_u16x4(p: &mut [u16], v: [__m128d; 4]) {
        let out: &mut [u16; 8] = (&mut p[..8]).try_into().unwrap();
        unsafe {
            let r = _mm_cvttpd_epi32(v[0]);
            let g = _mm_cvttpd_epi32(v[1]);
            let b = _mm_cvttpd_epi32(v[2]);
            let a = _mm_cvttpd_epi32(v[3]);
            let rg = _mm_or_si128(r, _mm_slli_epi32::<16>(g));
            let ba = _mm_or_si128(b, _mm_slli_epi32::<16>(a));
            _mm_storeu_si128(out.as_mut_ptr().cast(), _mm_unpacklo_epi32(rg, ba));
        }
    }
    #[inline(always)]
    unsafe fn splat_px(px: [u8; 4]) -> [__m128d; 4] {
        unsafe {
            [
                Self::splat(f64::from(px[0])),
                Self::splat(f64::from(px[1])),
                Self::splat(f64::from(px[2])),
                Self::splat(f64::from(px[3])),
            ]
        }
    }
    #[inline(always)]
    unsafe fn store(p: &mut [u8], v: [__m128d; 4]) {
        let out: &mut [u8; 8] = (&mut p[..8]).try_into().unwrap();
        unsafe {
            let r = _mm_cvttpd_epi32(v[0]);
            let g = _mm_cvttpd_epi32(v[1]);
            let b = _mm_cvttpd_epi32(v[2]);
            let a = _mm_cvttpd_epi32(v[3]);
            let rg = _mm_or_si128(r, _mm_slli_epi32::<8>(g));
            let ba = _mm_or_si128(_mm_slli_epi32::<16>(b), _mm_slli_epi32::<24>(a));
            _mm_storel_epi64(out.as_mut_ptr().cast(), _mm_or_si128(rg, ba));
        }
    }
}
