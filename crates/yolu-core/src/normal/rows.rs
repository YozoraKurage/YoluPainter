//! Normal のチャンネルの合成の行の核と、画素ごとの関数（[`super::blend`]・[`super::clip_onto`]・[`super::fade`]）が通る式。
//!
//! 計算は f32 で、色の合成（`crate::blend`）と同じ形にしてある: 道は AVX2（8 画素ずつ）・SSE4.1・NEON（4 画素ずつ）・スカラー（1 画素ずつ）で
//! （CPU が持つ物だけ）、どれも同じレーンの式を通り、SIMD の道の端の画素（N で割った余り）・スカラーの道・画素ごとの関数は 1 本のレーン
//! （[`Scalar1`]）で同じ関数を呼ぶ。演算は IEEE の四則・平方根・比較・選択だけ（積和の命令・近似の逆数は使わない）なので、各画素の
//! 結果は道とスレッド数によらず同じバイトになる。量は色の合成と同じく、f64 の不透明度 × マスクの表を f32 へ丸めた値
//! （`RowAmount::lanes`。画素ごとの関数は `amount as f32`）。
//!
//! Height → Normal の出力（`super::output`）は f64 の式のまま。
#![cfg_attr(
    not(any(target_arch = "x86_64", target_arch = "aarch64")),
    allow(dead_code, unused_imports, unused_macros, unused_variables, unused_mut)
)]

use super::is_detail;
use crate::blend::{simd_step, RowAmount};
use crate::math::simd::{self, to_byte32 as to_byte, Lanes32, Level, Scalar1};
use crate::types::{BlendMode, Rgba8};

#[cfg(target_arch = "aarch64")]
use crate::math::simd::Neonx4;
#[cfg(target_arch = "x86_64")]
use crate::math::simd::{Avx2x8, Sse41x4};

type Vec3<V> = [<V as Lanes32>::F; 3];

/// 向きを持たない長さ²（これより短いと平ら）。
const DEGENERATE_LENGTH_SQUARED: f32 = 1e-12;
/// RNM の土台の z + 1 がこれ以下（真内向き）なら、座標系を持たないので土台のまま。
const DEGENERATE_BASE: f32 = 1e-6;

// ───────── レーンの式 ─────────

#[inline(always)]
unsafe fn normalize_lanes<V: Lanes32>(v: Vec3<V>) -> Vec3<V> {
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

/// 符号化した画素（R・G・B のレーン、0〜255 の整数）の単位ベクトル: c / 255 × 2 − 1 を正規化する。
#[inline(always)]
unsafe fn decode_lanes<V: Lanes32>(rgb: Vec3<V>) -> Vec3<V> {
    let (two, one) = (V::splat(2.0), V::splat(1.0));
    normalize_lanes::<V>([
        V::sub(V::mul(V::unit(rgb[0]), two), one),
        V::sub(V::mul(V::unit(rgb[1]), two), one),
        V::sub(V::mul(V::unit(rgb[2]), two), one),
    ])
}

/// ベクトルを正規化して符号化する（0〜255 の整数のレーン。四捨五入は色の合成と同じ）。
#[inline(always)]
unsafe fn encode_lanes<V: Lanes32>(v: Vec3<V>) -> Vec3<V> {
    let (half, n) = (V::splat(0.5), normalize_lanes::<V>(v));
    [
        to_byte::<V>(V::add(V::mul(n[0], half), half)),
        to_byte::<V>(V::add(V::mul(n[1], half), half)),
        to_byte::<V>(V::add(V::mul(n[2], half), half)),
    ]
}

/// Reoriented Normal Mapping: 細部 d を土台 b の向きへ回す（どちらも単位、接空間）。
#[inline(always)]
unsafe fn rnm_lanes<V: Lanes32>(b: Vec3<V>, d: Vec3<V>) -> Vec3<V> {
    let (tx, ty, tz) = (b[0], b[1], V::add(b[2], V::splat(1.0)));
    let (ux, uy, uz) = (V::neg(d[0]), V::neg(d[1]), d[2]);
    let degenerate = V::le(tz, V::splat(DEGENERATE_BASE));
    let dot = V::add(V::add(V::mul(tx, ux), V::mul(ty, uy)), V::mul(tz, uz));
    let k = V::div(dot, tz);
    [
        V::select(degenerate, b[0], V::sub(V::mul(tx, k), ux)),
        V::select(degenerate, b[1], V::sub(V::mul(ty, k), uy)),
        V::select(degenerate, b[2], V::sub(V::mul(tz, k), uz)),
    ]
}

// ───────── N 画素の式（画素ごとの関数も 1 本のレーンでこれを呼ぶ） ─────────

/// N 画素の重ね: t = 上のアルファ × 量、da = 下のアルファで normalize((1 − t)·da·下 + (1 − da)·t·上 + da·t·B(下, 上))、
/// アルファは t + da(1 − t)。`DETAIL` は Overlay（B は RNM）、ほかは B = 上。t ≤ 0 の画素は下のまま。何も変わらない組は None。
#[inline(always)]
unsafe fn blend_block<V: Lanes32, const DETAIL: bool>(
    dst: [V::F; 4],
    src: [V::F; 4],
    amount: V::F,
) -> Option<[V::F; 4]> {
    let one = V::splat(1.0);
    let t = V::mul(V::unit(src[3]), amount);
    let skip = V::le(t, V::splat(0.0));
    if V::all(skip) {
        return None;
    }
    let da = V::unit(dst[3]);
    let b = decode_lanes::<V>([dst[0], dst[1], dst[2]]);
    let s = decode_lanes::<V>([src[0], src[1], src[2]]);
    let c = if DETAIL { rnm_lanes::<V>(b, s) } else { s };
    let wb = V::mul(V::sub(one, t), da);
    let ws = V::mul(V::sub(one, da), t);
    let wc = V::mul(da, t);
    let mut mixed = [wb; 3];
    for k in 0..3 {
        mixed[k] = V::add(V::add(V::mul(wb, b[k]), V::mul(ws, s[k])), V::mul(wc, c[k]));
    }
    let rgb = encode_lanes::<V>(mixed);
    let alpha = to_byte::<V>(V::add(t, V::mul(da, V::sub(one, t))));
    Some([
        V::select(skip, dst[0], rgb[0]),
        V::select(skip, dst[1], rgb[1]),
        V::select(skip, dst[2], rgb[2]),
        V::select(skip, dst[3], alpha),
    ])
}

/// N 画素のクリッピング: normalize((1 − t)·下地 + t·B(下地, 上))、t = 上のアルファ × 量。下地のアルファのまま。t ≤ 0・下地が
/// 透明な画素はそのまま。
#[inline(always)]
unsafe fn clip_block<V: Lanes32, const DETAIL: bool>(
    dst: [V::F; 4],
    src: [V::F; 4],
    amount: V::F,
) -> Option<[V::F; 4]> {
    let zero = V::splat(0.0);
    let t = V::mul(V::unit(src[3]), amount);
    let skip = V::or(V::le(t, zero), V::eq(dst[3], zero));
    if V::all(skip) {
        return None;
    }
    let one = V::splat(1.0);
    let g = decode_lanes::<V>([dst[0], dst[1], dst[2]]);
    let s = decode_lanes::<V>([src[0], src[1], src[2]]);
    let c = if DETAIL { rnm_lanes::<V>(g, s) } else { s };
    let keep = V::sub(one, t);
    let mut mixed = [keep; 3];
    for k in 0..3 {
        mixed[k] = V::add(V::mul(keep, g[k]), V::mul(t, c[k]));
    }
    let rgb = encode_lanes::<V>(mixed);
    Some([
        V::select(skip, dst[0], rgb[0]),
        V::select(skip, dst[1], rgb[1]),
        V::select(skip, dst[2], rgb[2]),
        dst[3],
    ])
}

/// N 画素のフェード: 下と中身のアルファで重みを付けた平均を正規化する。量が 1 以上は中身そのもの、0 以下は下そのまま、
/// 重みの和が 0 以下は透明。
#[inline(always)]
unsafe fn fade_block<V: Lanes32>(backdrop: [V::F; 4], inner: [V::F; 4], amount: V::F) -> [V::F; 4] {
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let ba = V::mul(V::unit(backdrop[3]), V::sub(one, amount));
    let ia = V::mul(V::unit(inner[3]), amount);
    let a = V::add(ba, ia);
    let nothing = V::le(a, zero);
    let b = decode_lanes::<V>([backdrop[0], backdrop[1], backdrop[2]]);
    let i = decode_lanes::<V>([inner[0], inner[1], inner[2]]);
    let mut mixed = [ba; 3];
    for k in 0..3 {
        mixed[k] = V::add(V::mul(ba, b[k]), V::mul(ia, i[k]));
    }
    let rgb = encode_lanes::<V>(mixed);
    let alpha = to_byte::<V>(a);
    let whole = V::ge(amount, one);
    let none = V::le(amount, zero);
    let computed = [rgb[0], rgb[1], rgb[2], alpha];
    let mut out = backdrop;
    for c in 0..4 {
        out[c] = V::select(
            whole,
            inner[c],
            V::select(none, backdrop[c], V::select(nothing, zero, computed[c])),
        );
    }
    out
}

// ───────── 画素ごとの関数（1 本のレーン） ─────────

/// Overlay（RNM）かどうかを定数にして呼ぶ。
macro_rules! with_detail {
    ($mode:expr, $f:ident::<$v:ty>($($arg:expr),* $(,)?)) => {
        if is_detail($mode) {
            $f::<$v, true>($($arg),*)
        } else {
            $f::<$v, false>($($arg),*)
        }
    };
}

#[inline(always)]
fn px(c: Rgba8) -> [f32; 4] {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { Scalar1::splat_px(c.to_array()) }
}

#[inline(always)]
fn rgba(v: [f32; 4]) -> Rgba8 {
    let mut p = [0u8; 4];
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { Scalar1::store(&mut p, v) };
    Rgba8::new(p[0], p[1], p[2], p[3])
}

/// 画素ごとの重ね（[`super::blend`] の中身）。行の核のスカラーの道と同じ関数。
#[inline]
pub(super) fn blend_pixel(d: Rgba8, s: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { with_detail!(mode, blend_block::<Scalar1>(px(d), px(s), amount as f32)) }
        .map_or(d, rgba)
}

/// 画素ごとのクリッピング（[`super::clip_onto`] の中身）。
#[inline]
pub(super) fn clip_pixel(d: Rgba8, s: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { with_detail!(mode, clip_block::<Scalar1>(px(d), px(s), amount as f32)) }
        .map_or(d, rgba)
}

/// 画素ごとのフェード（[`super::fade`] の中身）。
#[inline]
pub(super) fn fade_pixel(d: Rgba8, s: Rgba8, amount: f64) -> Rgba8 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    rgba(unsafe { fade_block::<Scalar1>(px(d), px(s), amount as f32) })
}

// ───────── 行（V で N 画素ずつ、端は 1 本のレーン） ─────────

#[inline(always)]
unsafe fn blend_row_lanes<V: Lanes32, const DETAIL: bool>(
    res: &mut [u8],
    sb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
) {
    let count = res.len() / 4;
    let mut i = 0;
    if V::N > 1 {
        let uniform = if step == 0 {
            Some(V::splat_px(sb[..4].try_into().unwrap()))
        } else {
            None
        };
        while i + V::N <= count {
            let src = match uniform {
                Some(u) => u,
                None => V::load(&sb[i * 4..]),
            };
            let dst = V::load(&res[i * 4..]);
            if let Some(out) = blend_block::<V, DETAIL>(dst, src, amount.lanes::<V>(i)) {
                V::store(&mut res[i * 4..], out);
            }
            i += V::N;
        }
    }
    while i < count {
        let src = Scalar1::load(&sb[i * step..]);
        let dst = Scalar1::load(&res[i * 4..]);
        if let Some(out) = blend_block::<Scalar1, DETAIL>(dst, src, amount.lanes::<Scalar1>(i)) {
            Scalar1::store(&mut res[i * 4..], out);
        }
        i += 1;
    }
}

#[inline(always)]
unsafe fn clip_row_lanes<V: Lanes32, const DETAIL: bool>(
    g: &mut [u8],
    cb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
) {
    let count = g.len() / 4;
    let mut i = 0;
    if V::N > 1 {
        let uniform = if step == 0 {
            Some(V::splat_px(cb[..4].try_into().unwrap()))
        } else {
            None
        };
        while i + V::N <= count {
            let src = match uniform {
                Some(u) => u,
                None => V::load(&cb[i * 4..]),
            };
            let dst = V::load(&g[i * 4..]);
            if let Some(out) = clip_block::<V, DETAIL>(dst, src, amount.lanes::<V>(i)) {
                V::store(&mut g[i * 4..], out);
            }
            i += V::N;
        }
    }
    while i < count {
        let src = Scalar1::load(&cb[i * step..]);
        let dst = Scalar1::load(&g[i * 4..]);
        if let Some(out) = clip_block::<Scalar1, DETAIL>(dst, src, amount.lanes::<Scalar1>(i)) {
            Scalar1::store(&mut g[i * 4..], out);
        }
        i += 1;
    }
}

#[inline(always)]
unsafe fn fade_row_lanes<V: Lanes32>(res: &mut [u8], inner: &[u8], amount: RowAmount<'_>) {
    let count = res.len() / 4;
    let mut i = 0;
    if V::N > 1 {
        while i + V::N <= count {
            let out = fade_block::<V>(
                V::load(&res[i * 4..]),
                V::load(&inner[i * 4..]),
                amount.lanes::<V>(i),
            );
            V::store(&mut res[i * 4..], out);
            i += V::N;
        }
    }
    while i < count {
        let out = fade_block::<Scalar1>(
            Scalar1::load(&res[i * 4..]),
            Scalar1::load(&inner[i * 4..]),
            amount.lanes::<Scalar1>(i),
        );
        Scalar1::store(&mut res[i * 4..], out);
        i += 1;
    }
}

// ───────── 入口 ─────────

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn blend_row_avx2(
    res: &mut [u8],
    sb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    with_detail!(mode, blend_row_lanes::<Avx2x8>(res, sb, step, amount))
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn blend_row_sse41(
    res: &mut [u8],
    sb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    with_detail!(mode, blend_row_lanes::<Sse41x4>(res, sb, step, amount))
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn blend_row_neon(
    res: &mut [u8],
    sb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    with_detail!(mode, blend_row_lanes::<Neonx4>(res, sb, step, amount))
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn clip_row_avx2(
    g: &mut [u8],
    cb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    with_detail!(mode, clip_row_lanes::<Avx2x8>(g, cb, step, amount))
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn clip_row_sse41(
    g: &mut [u8],
    cb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    with_detail!(mode, clip_row_lanes::<Sse41x4>(g, cb, step, amount))
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn clip_row_neon(
    g: &mut [u8],
    cb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    with_detail!(mode, clip_row_lanes::<Neonx4>(g, cb, step, amount))
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn fade_row_avx2(res: &mut [u8], inner: &[u8], amount: RowAmount<'_>) {
    fade_row_lanes::<Avx2x8>(res, inner, amount)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn fade_row_sse41(res: &mut [u8], inner: &[u8], amount: RowAmount<'_>) {
    fade_row_lanes::<Sse41x4>(res, inner, amount)
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn fade_row_neon(res: &mut [u8], inner: &[u8], amount: RowAmount<'_>) {
    fade_row_lanes::<Neonx4>(res, inner, amount)
}

/// Normal のチャンネルで、下（res）に上（sb、刻み `step`。0 は 1 画素を全部に、4 は連続、ほかはスカラーの道）をベクトルとして
/// 重ねる。各画素は [`super::blend`] と同じバイト。
#[inline]
pub fn blend_row(res: &mut [u8], sb: &[u8], step: usize, amount: RowAmount<'_>, mode: BlendMode) {
    blend_row_at(simd::level(), res, sb, step, amount, mode)
}

/// [`blend_row`] の、道を指定する形（CPU が持たない道を指定されたら、持つ一番広い道に下げる）。
pub(crate) fn blend_row_at(
    level: Level,
    res: &mut [u8],
    sb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    let level = if simd_step(step) {
        level.min(simd::detect())
    } else {
        Level::Scalar
    };
    match level {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2 を持つ
        Level::Avx2 => unsafe { blend_row_avx2(res, sb, step, amount, mode) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { blend_row_sse41(res, sb, step, amount, mode) },
        #[cfg(target_arch = "aarch64")]
        // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
        Level::Neon => unsafe { blend_row_neon(res, sb, step, amount, mode) },
        // SAFETY: 1 本のレーンは CPU の前提を持たない
        Level::Scalar => unsafe {
            with_detail!(mode, blend_row_lanes::<Scalar1>(res, sb, step, amount))
        },
    }
}

/// Normal のチャンネルで、クリッピングの下地（g）へクリッピングされたレイヤー（cb、刻みは [`blend_row`] と同じ）を重ねる。
/// 各画素は [`super::clip_onto`] と同じバイト。
#[inline]
pub fn clip_row(g: &mut [u8], cb: &[u8], step: usize, amount: RowAmount<'_>, mode: BlendMode) {
    clip_row_at(simd::level(), g, cb, step, amount, mode)
}

pub(crate) fn clip_row_at(
    level: Level,
    g: &mut [u8],
    cb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    let level = if simd_step(step) {
        level.min(simd::detect())
    } else {
        Level::Scalar
    };
    match level {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2 を持つ
        Level::Avx2 => unsafe { clip_row_avx2(g, cb, step, amount, mode) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { clip_row_sse41(g, cb, step, amount, mode) },
        #[cfg(target_arch = "aarch64")]
        // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
        Level::Neon => unsafe { clip_row_neon(g, cb, step, amount, mode) },
        // SAFETY: 1 本のレーンは CPU の前提を持たない
        Level::Scalar => unsafe {
            with_detail!(mode, clip_row_lanes::<Scalar1>(g, cb, step, amount))
        },
    }
}

/// Normal のチャンネルの通過のグループのフェード（res の下と inner を量で）。各画素は [`super::fade`] と同じバイト。
#[inline]
pub fn fade_row(res: &mut [u8], inner: &[u8], amount: RowAmount<'_>) {
    fade_row_at(simd::level(), res, inner, amount)
}

pub(crate) fn fade_row_at(level: Level, res: &mut [u8], inner: &[u8], amount: RowAmount<'_>) {
    match level.min(simd::detect()) {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2 を持つ
        Level::Avx2 => unsafe { fade_row_avx2(res, inner, amount) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { fade_row_sse41(res, inner, amount) },
        #[cfg(target_arch = "aarch64")]
        // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
        Level::Neon => unsafe { fade_row_neon(res, inner, amount) },
        // SAFETY: 1 本のレーンは CPU の前提を持たない
        Level::Scalar => unsafe { fade_row_lanes::<Scalar1>(res, inner, amount) },
    }
}

#[cfg(test)]
mod tests;
