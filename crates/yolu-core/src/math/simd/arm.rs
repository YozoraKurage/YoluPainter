//! aarch64 のレーン: NEON（f64 が 2 本の [`Neon`] と、f32 が 4 本の [`Neonx4`]）。
//!
//! 各メソッドは `unsafe fn` で、前提は「CPU がその命令を持つ」こと（[`Lanes`] の `# Safety`）。NEON は AArch64 の基本の命令で、aarch64 の
//! どの CPU にもあるので、実行時の判定はしない。型を使う関数は、x86_64 の入口と同じ作りで `#[target_feature(enable = "neon")]` 付きの
//! `unsafe fn` の入口の中にだけあり、入口は [`super::level`] が返した道（[`super::Level::Neon`]）でだけ呼ばれる。メソッドの中の
//! `unsafe` ブロックは組み込み関数の呼び出しだけ。画素の読み書きは長さを確かめた固定長の配列へ変換してから行うので、範囲外を読み書きしない。
//!
//! **スカラーと同じ bit にするための決め**（x86_64 の SSE4.1 の型と同じ作り。`Lanes` の説明どおり）:
//! - `min`・`max` は比較（`vclt`・`vcgt`）と選択（`vbsl`）で `if a < b { a } else { b }`・`if a > b { a } else { b }` を作る。
//!   `vmin`・`vmax`・`vminnm`・`vmaxnm` は使わない（NaN と ±0 を返す向きがスカラーの比較と違う）。
//! - 積和を 1 つの命令にまとめない（`vfma`・`vmla` を使わない）。`unit` は割り算（`vdiv`）。FMA で割り算を省く x86_64 の AVX2 の近道は持ち込まない。
//! - `floor` は負の無限大へ丸める命令（`vrndm`）、`sqrt`・`div` は IEEE どおりに丸める `vsqrt`・`vdiv`。`abs`・`neg` は符号のビットだけを変える。
//! - 比較は順序つき（NaN なら偽）で、結果はレーンごとの全ビットが 0 か 1 のマスク。`and`・`or`・`not`・`any`・`all` はマスクのビットで、
//!   `select` はビットの選択なので、選ばれなかった側が NaN でも結果に影響しない。
//! - 整数への変換は切り捨て（`vcvt`）、整数からの変換は正確（読み書きする値は 0〜255・0〜65535・2³¹ 未満の整数）。
//!
//! 並びは little-endian（AArch64 の Linux・macOS・Windows はどれも little-endian）で、RGBA のバイトは R が最下位。
#![deny(unsafe_op_in_unsafe_fn)]
// 引数がレジスタだけの組み込み関数は、NEON を有効にした文脈では安全な関数として呼べる（rustc の版による）。この型の `unsafe` ブロックは
// 版によらず同じ形に揃えてあるので、安全な呼び出しだけを含む場合の警告は出さない
#![allow(unused_unsafe)]

use core::arch::aarch64::*;

use super::{Lanes, Lanes32};

/// RGBA が 2 画素入った 8 バイト（16 バイトの表に 2 回並べた物）から、チャンネル c のバイトを 2 つの u64 のレーンに取り出す `vqtbl1q_u8` の
/// 添字（範囲外の 255 はバイトが 0 になる。little-endian なので、各レーンの最下位のバイトに入り、上は 0）。
const fn byte_channel(c: u8) -> [u8; 16] {
    let mut t = [255u8; 16];
    t[0] = c;
    t[8] = c + 4;
    t
}
/// u16 の RGBA が 2 画素入った 16 バイトから、チャンネル c の u16 を 2 つの u64 のレーンに取り出す添字。
const fn u16_channel(c: u8) -> [u8; 16] {
    let mut t = [255u8; 16];
    t[0] = 2 * c;
    t[1] = 2 * c + 1;
    t[8] = 8 + 2 * c;
    t[9] = 9 + 2 * c;
    t
}
const BYTE_CHANNELS: [[u8; 16]; 4] = [
    byte_channel(0),
    byte_channel(1),
    byte_channel(2),
    byte_channel(3),
];
const U16_CHANNELS: [[u8; 16]; 4] = [
    u16_channel(0),
    u16_channel(1),
    u16_channel(2),
    u16_channel(3),
];

/// `table` の 16 バイトから、添字 `channel` が示すバイトを集めて 2 つの u64 にし、f64 にする（u64 は 2⁵³ 未満なので正確）。
#[inline(always)]
unsafe fn gather_f64(table: uint8x16_t, channel: &[u8; 16]) -> float64x2_t {
    unsafe {
        let picked = vqtbl1q_u8(table, vld1q_u8(channel.as_ptr()));
        vcvtq_f64_u64(vreinterpretq_u64_u8(picked))
    }
}

// ───────── NEON（f64 が 2 本） ─────────

/// NEON を持つ CPU 用（f64 が 2 本）。
#[derive(Clone, Copy)]
pub(crate) struct Neon;

impl Lanes for Neon {
    const N: usize = 2;
    type F = float64x2_t;
    type M = uint64x2_t;

    #[inline(always)]
    unsafe fn splat(x: f64) -> float64x2_t {
        unsafe { vdupq_n_f64(x) }
    }
    #[inline(always)]
    unsafe fn from_fn(mut f: impl FnMut(usize) -> f64) -> float64x2_t {
        let both = [f(0), f(1)];
        unsafe { vld1q_f64(both.as_ptr()) }
    }
    #[cfg(test)]
    #[inline(always)]
    unsafe fn lane(v: float64x2_t, k: usize) -> f64 {
        let mut out = [0.0f64; 2];
        unsafe { vst1q_f64(out.as_mut_ptr(), v) };
        out[k]
    }

    #[inline(always)]
    unsafe fn add(a: float64x2_t, b: float64x2_t) -> float64x2_t {
        unsafe { vaddq_f64(a, b) }
    }
    #[inline(always)]
    unsafe fn sub(a: float64x2_t, b: float64x2_t) -> float64x2_t {
        unsafe { vsubq_f64(a, b) }
    }
    #[inline(always)]
    unsafe fn mul(a: float64x2_t, b: float64x2_t) -> float64x2_t {
        unsafe { vmulq_f64(a, b) }
    }
    #[inline(always)]
    unsafe fn div(a: float64x2_t, b: float64x2_t) -> float64x2_t {
        unsafe { vdivq_f64(a, b) }
    }
    #[inline(always)]
    unsafe fn sqrt(a: float64x2_t) -> float64x2_t {
        unsafe { vsqrtq_f64(a) }
    }
    #[inline(always)]
    unsafe fn min(a: float64x2_t, b: float64x2_t) -> float64x2_t {
        unsafe { vbslq_f64(vcltq_f64(a, b), a, b) }
    }
    #[inline(always)]
    unsafe fn max(a: float64x2_t, b: float64x2_t) -> float64x2_t {
        unsafe { vbslq_f64(vcgtq_f64(a, b), a, b) }
    }
    #[inline(always)]
    unsafe fn floor(a: float64x2_t) -> float64x2_t {
        unsafe { vrndmq_f64(a) }
    }
    #[inline(always)]
    unsafe fn abs(a: float64x2_t) -> float64x2_t {
        unsafe { vabsq_f64(a) }
    }
    #[inline(always)]
    unsafe fn neg(a: float64x2_t) -> float64x2_t {
        unsafe { vnegq_f64(a) }
    }

    #[inline(always)]
    unsafe fn lt(a: float64x2_t, b: float64x2_t) -> uint64x2_t {
        unsafe { vcltq_f64(a, b) }
    }
    #[inline(always)]
    unsafe fn le(a: float64x2_t, b: float64x2_t) -> uint64x2_t {
        unsafe { vcleq_f64(a, b) }
    }
    #[inline(always)]
    unsafe fn gt(a: float64x2_t, b: float64x2_t) -> uint64x2_t {
        unsafe { vcgtq_f64(a, b) }
    }
    #[inline(always)]
    unsafe fn ge(a: float64x2_t, b: float64x2_t) -> uint64x2_t {
        unsafe { vcgeq_f64(a, b) }
    }
    #[inline(always)]
    unsafe fn eq(a: float64x2_t, b: float64x2_t) -> uint64x2_t {
        unsafe { vceqq_f64(a, b) }
    }

    #[inline(always)]
    unsafe fn and(a: uint64x2_t, b: uint64x2_t) -> uint64x2_t {
        unsafe { vandq_u64(a, b) }
    }
    #[inline(always)]
    unsafe fn or(a: uint64x2_t, b: uint64x2_t) -> uint64x2_t {
        unsafe { vorrq_u64(a, b) }
    }
    #[inline(always)]
    unsafe fn not(a: uint64x2_t) -> uint64x2_t {
        unsafe { veorq_u64(a, vdupq_n_u64(u64::MAX)) }
    }
    #[inline(always)]
    unsafe fn select(m: uint64x2_t, a: float64x2_t, b: float64x2_t) -> float64x2_t {
        unsafe { vbslq_f64(m, a, b) }
    }
    #[inline(always)]
    unsafe fn any(m: uint64x2_t) -> bool {
        // マスクはレーンごとに全ビットが 0 か 1 なので、32 ビットごとに見ても同じ
        unsafe { vmaxvq_u32(vreinterpretq_u32_u64(m)) != 0 }
    }
    #[inline(always)]
    unsafe fn all(m: uint64x2_t) -> bool {
        unsafe { vminvq_u32(vreinterpretq_u32_u64(m)) == u32::MAX }
    }

    #[inline(always)]
    unsafe fn unit(b: float64x2_t) -> float64x2_t {
        unsafe { vdivq_f64(b, vdupq_n_f64(255.0)) }
    }

    #[inline(always)]
    unsafe fn load(p: &[u8]) -> [float64x2_t; 4] {
        let a: &[u8; 8] = p[..8].try_into().unwrap();
        unsafe {
            let x = vld1_u8(a.as_ptr());
            // 添字は 0〜7 しか使わないので、表の後ろ半分は前半と同じ物でよい
            let table = vcombine_u8(x, x);
            [
                gather_f64(table, &BYTE_CHANNELS[0]),
                gather_f64(table, &BYTE_CHANNELS[1]),
                gather_f64(table, &BYTE_CHANNELS[2]),
                gather_f64(table, &BYTE_CHANNELS[3]),
            ]
        }
    }
    #[inline(always)]
    unsafe fn load_f64(p: &[f64]) -> float64x2_t {
        let a: &[f64; 2] = p[..2].try_into().unwrap();
        unsafe { vld1q_f64(a.as_ptr()) }
    }
    #[inline(always)]
    unsafe fn store_f64(p: &mut [f64], v: float64x2_t) {
        let out: &mut [f64; 2] = (&mut p[..2]).try_into().unwrap();
        unsafe { vst1q_f64(out.as_mut_ptr(), v) }
    }
    #[inline(always)]
    unsafe fn load_u16(p: &[u16]) -> float64x2_t {
        let a: &[u16; 2] = p[..2].try_into().unwrap();
        let both = u32::from(a[0]) | (u32::from(a[1]) << 16);
        unsafe {
            let wide = vmovl_u16(vreinterpret_u16_u32(vdup_n_u32(both)));
            vcvtq_f64_u64(vmovl_u32(vget_low_u32(wide)))
        }
    }
    #[inline(always)]
    unsafe fn store_u16(p: &mut [u16], v: float64x2_t) {
        let out: &mut [u16; 2] = (&mut p[..2]).try_into().unwrap();
        unsafe {
            let i = vqmovn_u64(vcvtq_u64_f64(v));
            let packed = vqmovn_u32(vcombine_u32(i, i));
            let both = vget_lane_u32::<0>(vreinterpret_u32_u16(packed));
            out[0] = both as u16;
            out[1] = (both >> 16) as u16;
        }
    }
    #[inline(always)]
    unsafe fn load_u32(p: &[u32]) -> float64x2_t {
        let a: &[u32; 2] = p[..2].try_into().unwrap();
        unsafe { vcvtq_f64_u64(vmovl_u32(vld1_u32(a.as_ptr()))) }
    }
    #[inline(always)]
    unsafe fn store_u32(p: &mut [u32], v: float64x2_t) {
        let out: &mut [u32; 2] = (&mut p[..2]).try_into().unwrap();
        unsafe { vst1_u32(out.as_mut_ptr(), vqmovn_u64(vcvtq_u64_f64(v))) }
    }
    #[inline(always)]
    unsafe fn load_u16x4(p: &[u16]) -> [float64x2_t; 4] {
        let a: &[u16; 8] = p[..8].try_into().unwrap();
        unsafe {
            let table = vreinterpretq_u8_u16(vld1q_u16(a.as_ptr()));
            [
                gather_f64(table, &U16_CHANNELS[0]),
                gather_f64(table, &U16_CHANNELS[1]),
                gather_f64(table, &U16_CHANNELS[2]),
                gather_f64(table, &U16_CHANNELS[3]),
            ]
        }
    }
    #[inline(always)]
    unsafe fn store_u16x4(p: &mut [u16], v: [float64x2_t; 4]) {
        let out: &mut [u16; 8] = (&mut p[..8]).try_into().unwrap();
        unsafe {
            let r = vqmovn_u64(vcvtq_u64_f64(v[0]));
            let g = vqmovn_u64(vcvtq_u64_f64(v[1]));
            let b = vqmovn_u64(vcvtq_u64_f64(v[2]));
            let a = vqmovn_u64(vcvtq_u64_f64(v[3]));
            // 各 32 ビットに R | G << 16 と B | A << 16（画素 0・1）。交互に並べると u16 で R G B A R G B A になる
            let rg = vorr_u32(r, vshl_n_u32::<16>(g));
            let ba = vorr_u32(b, vshl_n_u32::<16>(a));
            let px = vcombine_u32(vzip1_u32(rg, ba), vzip2_u32(rg, ba));
            vst1q_u16(out.as_mut_ptr(), vreinterpretq_u16_u32(px));
        }
    }
    #[inline(always)]
    unsafe fn splat_px(px: [u8; 4]) -> [float64x2_t; 4] {
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
    unsafe fn store(p: &mut [u8], v: [float64x2_t; 4]) {
        let out: &mut [u8; 8] = (&mut p[..8]).try_into().unwrap();
        unsafe {
            let r = vqmovn_u64(vcvtq_u64_f64(v[0]));
            let g = vqmovn_u64(vcvtq_u64_f64(v[1]));
            let b = vqmovn_u64(vcvtq_u64_f64(v[2]));
            let a = vqmovn_u64(vcvtq_u64_f64(v[3]));
            let rg = vorr_u32(r, vshl_n_u32::<8>(g));
            let ba = vorr_u32(vshl_n_u32::<16>(b), vshl_n_u32::<24>(a));
            vst1_u8(out.as_mut_ptr(), vreinterpret_u8_u32(vorr_u32(rg, ba)));
        }
    }
}

// ───────── NEON（f32 が 4 本） ─────────

/// NEON を持つ CPU 用（f32 が 4 本）。
#[derive(Clone, Copy)]
pub(crate) struct Neonx4;

impl Lanes32 for Neonx4 {
    const N: usize = 4;
    type F = float32x4_t;
    type M = uint32x4_t;

    #[inline(always)]
    unsafe fn splat(x: f32) -> float32x4_t {
        unsafe { vdupq_n_f32(x) }
    }
    #[inline(always)]
    unsafe fn from_fn(mut f: impl FnMut(usize) -> f32) -> float32x4_t {
        let all = [f(0), f(1), f(2), f(3)];
        unsafe { vld1q_f32(all.as_ptr()) }
    }
    #[cfg(test)]
    unsafe fn lane(v: float32x4_t, k: usize) -> f32 {
        let mut out = [0.0f32; 4];
        unsafe { vst1q_f32(out.as_mut_ptr(), v) };
        out[k]
    }

    #[inline(always)]
    unsafe fn add(a: float32x4_t, b: float32x4_t) -> float32x4_t {
        unsafe { vaddq_f32(a, b) }
    }
    #[inline(always)]
    unsafe fn sub(a: float32x4_t, b: float32x4_t) -> float32x4_t {
        unsafe { vsubq_f32(a, b) }
    }
    #[inline(always)]
    unsafe fn mul(a: float32x4_t, b: float32x4_t) -> float32x4_t {
        unsafe { vmulq_f32(a, b) }
    }
    #[inline(always)]
    unsafe fn div(a: float32x4_t, b: float32x4_t) -> float32x4_t {
        unsafe { vdivq_f32(a, b) }
    }
    #[inline(always)]
    unsafe fn sqrt(a: float32x4_t) -> float32x4_t {
        unsafe { vsqrtq_f32(a) }
    }
    #[inline(always)]
    unsafe fn min(a: float32x4_t, b: float32x4_t) -> float32x4_t {
        unsafe { vbslq_f32(vcltq_f32(a, b), a, b) }
    }
    #[inline(always)]
    unsafe fn max(a: float32x4_t, b: float32x4_t) -> float32x4_t {
        unsafe { vbslq_f32(vcgtq_f32(a, b), a, b) }
    }
    #[inline(always)]
    unsafe fn floor(a: float32x4_t) -> float32x4_t {
        unsafe { vrndmq_f32(a) }
    }
    #[inline(always)]
    unsafe fn abs(a: float32x4_t) -> float32x4_t {
        unsafe { vabsq_f32(a) }
    }
    #[inline(always)]
    unsafe fn neg(a: float32x4_t) -> float32x4_t {
        unsafe { vnegq_f32(a) }
    }
    #[inline(always)]
    unsafe fn lt(a: float32x4_t, b: float32x4_t) -> uint32x4_t {
        unsafe { vcltq_f32(a, b) }
    }
    #[inline(always)]
    unsafe fn le(a: float32x4_t, b: float32x4_t) -> uint32x4_t {
        unsafe { vcleq_f32(a, b) }
    }
    #[inline(always)]
    unsafe fn gt(a: float32x4_t, b: float32x4_t) -> uint32x4_t {
        unsafe { vcgtq_f32(a, b) }
    }
    #[inline(always)]
    unsafe fn ge(a: float32x4_t, b: float32x4_t) -> uint32x4_t {
        unsafe { vcgeq_f32(a, b) }
    }
    #[inline(always)]
    unsafe fn eq(a: float32x4_t, b: float32x4_t) -> uint32x4_t {
        unsafe { vceqq_f32(a, b) }
    }
    #[inline(always)]
    unsafe fn and(a: uint32x4_t, b: uint32x4_t) -> uint32x4_t {
        unsafe { vandq_u32(a, b) }
    }
    #[inline(always)]
    unsafe fn or(a: uint32x4_t, b: uint32x4_t) -> uint32x4_t {
        unsafe { vorrq_u32(a, b) }
    }
    #[inline(always)]
    unsafe fn not(a: uint32x4_t) -> uint32x4_t {
        unsafe { vmvnq_u32(a) }
    }
    #[inline(always)]
    unsafe fn select(m: uint32x4_t, a: float32x4_t, b: float32x4_t) -> float32x4_t {
        unsafe { vbslq_f32(m, a, b) }
    }
    #[inline(always)]
    unsafe fn any(m: uint32x4_t) -> bool {
        unsafe { vmaxvq_u32(m) != 0 }
    }
    #[inline(always)]
    unsafe fn all(m: uint32x4_t) -> bool {
        unsafe { vminvq_u32(m) == u32::MAX }
    }
    #[inline(always)]
    unsafe fn unit(b: float32x4_t) -> float32x4_t {
        unsafe { vdivq_f32(b, vdupq_n_f32(255.0)) }
    }
    #[inline(always)]
    unsafe fn load(p: &[u8]) -> [float32x4_t; 4] {
        let a: &[u8; 16] = p[..16].try_into().unwrap();
        unsafe {
            let x = vreinterpretq_u32_u8(vld1q_u8(a.as_ptr()));
            let m = vdupq_n_u32(0xFF);
            [
                vcvtq_f32_u32(vandq_u32(x, m)),
                vcvtq_f32_u32(vandq_u32(vshrq_n_u32::<8>(x), m)),
                vcvtq_f32_u32(vandq_u32(vshrq_n_u32::<16>(x), m)),
                vcvtq_f32_u32(vshrq_n_u32::<24>(x)),
            ]
        }
    }
    #[inline(always)]
    unsafe fn splat_px(px: [u8; 4]) -> [float32x4_t; 4] {
        unsafe {
            [
                vdupq_n_f32(f32::from(px[0])),
                vdupq_n_f32(f32::from(px[1])),
                vdupq_n_f32(f32::from(px[2])),
                vdupq_n_f32(f32::from(px[3])),
            ]
        }
    }
    #[inline(always)]
    unsafe fn store(p: &mut [u8], v: [float32x4_t; 4]) {
        let out: &mut [u8; 16] = (&mut p[..16]).try_into().unwrap();
        unsafe {
            // 値は 0〜255 の整数なので、切り捨ての変換で同じ値になる
            let r = vcvtq_u32_f32(v[0]);
            let g = vcvtq_u32_f32(v[1]);
            let b = vcvtq_u32_f32(v[2]);
            let a = vcvtq_u32_f32(v[3]);
            let rg = vorrq_u32(r, vshlq_n_u32::<8>(g));
            let ba = vorrq_u32(vshlq_n_u32::<16>(b), vshlq_n_u32::<24>(a));
            vst1q_u8(out.as_mut_ptr(), vreinterpretq_u8_u32(vorrq_u32(rg, ba)));
        }
    }
}
