//! 行（画素の並び）ごとの合成の核と、画素ごとの関数（[`super::blend`] など）が通る式。
//!
//! 道は AVX2（8 画素ずつ）・SSE4.1（4 画素ずつ）・NEON（4 画素ずつ。aarch64）・スカラー（1 画素ずつ）。どの道も同じ f32 の式（[`super::lanes`] と
//! この下の `*_block`）を通り、SIMD の道の端の画素（N で割った余り）・スカラーの道・画素ごとの関数は、1 本のレーン（[`Scalar1`]）で
//! 同じ関数を呼ぶ。演算は IEEE の四則・平方根・floor・比較・選択だけなので、各画素の結果は道とスレッド数によらず同じバイトになる。
//! 近道（上が透明・下が不透明・下が透明・Normal の量 1）は、レーンごとの条件に畳んで同じ結果を選ぶ。
//!
//! 量（不透明度 × マスクの表）は f64 で掛けてから f32 へ丸める（[`RowAmount::at`] を f32 にした値。画素ごとの関数も同じ値を受ける）。

use super::lanes::{blend_rgb, dispatch_mode, is_simple};
use super::MIN_SHORTCUT_ALPHA;
use crate::math::simd::{self, to_byte32 as to_byte, Lanes32, Level, Scalar1};
use crate::types::{BlendMode, Rgba8};

#[cfg(target_arch = "aarch64")]
use crate::math::simd::Neonx4;
#[cfg(target_arch = "x86_64")]
use crate::math::simd::{Avx2x8, Sse41x4};

/// 1 行ぶんの量: 不透明度、またはマスクがあれば 不透明度 × 表（マスクのアルファで引く）。
#[derive(Clone, Copy)]
pub struct RowAmount<'a> {
    pub opacity: f64,
    /// マスクの行（先頭の画素から）・画素の刻み（任意。粗い合成では 4 × 歩幅、1 色のマスクは 0）・アルファ → 量の表。
    /// 読み元 `sb` の刻み（0 か 4 のときだけ SIMD の道に入る）とは独立で、量は画素ごとに表を引いて読む。
    pub mask: Option<(&'a [u8], usize, &'a [f64; 256])>,
}

impl RowAmount<'_> {
    #[inline(always)]
    pub fn at(&self, i: usize) -> f64 {
        match self.mask {
            None => self.opacity,
            Some((m, step, f)) => self.opacity * f[m[i * step + 3] as usize],
        }
    }

    /// 行の途中（画素 `i` から）を先頭とする量。
    pub fn offset(&self, i: usize) -> RowAmount<'_> {
        RowAmount {
            opacity: self.opacity,
            mask: self.mask.map(|(m, step, f)| (&m[i * step..], step, f)),
        }
    }

    /// レーン k に画素 i + k の量（[`Self::at`] を f32 へ丸めた値）。Normal のチャンネルの合成（`crate::normal`）も同じ値を使う。
    #[inline(always)]
    pub(crate) unsafe fn lanes<V: Lanes32>(&self, i: usize) -> V::F {
        match self.mask {
            None => V::splat(self.opacity as f32),
            Some((m, step, f)) => {
                let opacity = self.opacity;
                V::from_fn(|k| (opacity * f[m[(i + k) * step + 3] as usize]) as f32)
            }
        }
    }
}

/// 読み元の画素の刻み（0 は 1 画素を全部に使う、4 は連続）だけを SIMD で扱う。ほかの刻みはスカラーの道に入る。
/// Normal のチャンネルの核（`crate::normal`）も同じ判定を使う。
#[inline(always)]
pub(crate) fn simd_step(step: usize) -> bool {
    let simd = step == 0 || step == 4;
    #[cfg(test)]
    if !simd {
        scalar_steps::note();
    }
    simd
}

/// 試験用: 読み元の刻みが SIMD の道に入れない値で行の核が呼ばれた回数（呼んだスレッドの分だけ）。粗い合成が、歩幅つきの読み元を
/// 詰めてから核へ渡すこと（歩幅つきのまま渡すとスカラーの道に落ちる）の確かめに使う。
#[cfg(test)]
pub(crate) mod scalar_steps {
    use std::cell::Cell;

    thread_local! {
        static CALLS: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) fn note() {
        CALLS.with(|c| c.set(c.get() + 1));
    }

    pub(crate) fn count() -> usize {
        CALLS.with(Cell::get)
    }
}

// ───────── N 画素の式（レーンの核。画素ごとの関数も 1 本のレーンでこれを呼ぶ） ─────────

/// N 画素の合成（下 dst に上 src を量 amount で）。値は 0〜255 の整数。何も変わらない組は None。
#[inline(always)]
pub(crate) unsafe fn blend_block<V: Lanes32, const MODE: u8>(
    dst: [V::F; 4],
    src: [V::F; 4],
    amount: V::F,
) -> Option<[V::F; 4]> {
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let (s_a, d_a) = (src[3], dst[3]);
    let sa = V::mul(V::unit(s_a), amount);
    // 上が透明、または量が 0 以下: 下のまま
    let skip = V::or(V::eq(s_a, zero), V::le(sa, zero));
    if V::all(skip) {
        return None;
    }
    // 下が透明なら上の色をそのまま（アルファは量で掛けた値）。Normal で上が不透明・量 1 なら、下が何でも上の色
    let mut copy = V::and(V::eq(d_a, zero), V::ge(sa, V::splat(MIN_SHORTCUT_ALPHA)));
    if is_simple(MODE) {
        copy = V::or(
            copy,
            V::and(V::eq(s_a, V::splat(255.0)), V::eq(amount, one)),
        );
    }
    copy = V::and(copy, V::not(skip));
    let alpha_of_copy = to_byte::<V>(sa);
    let compute = V::not(V::or(skip, copy));
    let mut out_rgb = [dst[0], dst[1], dst[2]];
    let out_a = if V::any(compute) {
        let d = [V::unit(dst[0]), V::unit(dst[1]), V::unit(dst[2])];
        let s = [V::unit(src[0]), V::unit(src[1]), V::unit(src[2])];
        let b = blend_rgb::<V, MODE>(d, s);
        // 下が不透明: a = a_s + (1 − a_s) はちょうど 1、重みは 1 − a_s・0・a_s
        let opaque = V::eq(d_a, V::splat(255.0));
        let t = V::sub(one, sa);
        let mut calc = [zero; 3];
        for c in 0..3 {
            calc[c] = V::add(V::mul(t, d[c]), V::mul(sa, b[c]));
        }
        let mut a_byte = V::splat(255.0);
        if !V::all(V::or(opaque, V::not(compute))) {
            // 下が半透明の画素がある: 一般の式（W3C の source-over の重み）
            let da = V::unit(d_a);
            let a = V::add(sa, V::mul(da, V::sub(one, sa)));
            let wd = V::mul(V::sub(one, sa), da);
            let ws = V::mul(V::sub(one, da), sa);
            let wb = V::mul(da, sa);
            for c in 0..3 {
                let general = V::div(
                    V::add(V::add(V::mul(wd, d[c]), V::mul(ws, s[c])), V::mul(wb, b[c])),
                    a,
                );
                calc[c] = V::select(opaque, calc[c], general);
            }
            a_byte = V::select(opaque, a_byte, to_byte::<V>(a));
        }
        for c in 0..3 {
            out_rgb[c] = V::select(skip, dst[c], V::select(copy, src[c], to_byte::<V>(calc[c])));
        }
        V::select(skip, d_a, V::select(copy, alpha_of_copy, a_byte))
    } else {
        for c in 0..3 {
            out_rgb[c] = V::select(skip, dst[c], src[c]);
        }
        V::select(skip, d_a, alpha_of_copy)
    };
    Some([out_rgb[0], out_rgb[1], out_rgb[2], out_a])
}

/// N 画素のクリッピング（下地 dst へ上 src を量 amount で。下地のアルファは変えない）。
#[inline(always)]
unsafe fn clip_block<V: Lanes32, const MODE: u8>(
    dst: [V::F; 4],
    src: [V::F; 4],
    amount: V::F,
) -> Option<[V::F; 4]> {
    let zero = V::splat(0.0);
    let a = V::mul(V::unit(src[3]), amount);
    // 量 0、または描く下地が無い: 下地のまま
    let skip = V::or(
        V::or(V::eq(src[3], zero), V::eq(dst[3], zero)),
        V::le(a, zero),
    );
    if V::all(skip) {
        return None;
    }
    let d = [V::unit(dst[0]), V::unit(dst[1]), V::unit(dst[2])];
    let s = [V::unit(src[0]), V::unit(src[1]), V::unit(src[2])];
    let b = blend_rgb::<V, MODE>(d, s);
    let mut out = [dst[0], dst[1], dst[2], dst[3]];
    for c in 0..3 {
        let v = to_byte::<V>(V::add(d[c], V::mul(V::sub(b[c], d[c]), a)));
        out[c] = V::select(skip, dst[c], v);
    }
    Some(out)
}

/// N 画素の調整の合成（下 dst と調整後の色 over をモードと量で。アルファは下のまま。量が 0 以下・下が透明な画素はそのまま）。
#[inline(always)]
unsafe fn mix_block<V: Lanes32, const MODE: u8>(
    dst: [V::F; 4],
    over: [V::F; 4],
    amount: V::F,
) -> Option<[V::F; 4]> {
    let zero = V::splat(0.0);
    let skip = V::or(V::le(amount, zero), V::eq(dst[3], zero));
    if V::all(skip) {
        return None;
    }
    let d = [V::unit(dst[0]), V::unit(dst[1]), V::unit(dst[2])];
    let s = [V::unit(over[0]), V::unit(over[1]), V::unit(over[2])];
    let b = blend_rgb::<V, MODE>(d, s);
    let mut out = dst;
    for c in 0..3 {
        let v = to_byte::<V>(V::add(d[c], V::mul(V::sub(b[c], d[c]), amount)));
        out[c] = V::select(skip, dst[c], v);
    }
    Some(out)
}

/// N 画素のフェード（下 backdrop と中身 inner をプリマルチプライドで補間。透明な側がもう片方を暗くしない）。
#[inline(always)]
pub(crate) unsafe fn fade_block<V: Lanes32>(
    backdrop: [V::F; 4],
    inner: [V::F; 4],
    amount: V::F,
) -> [V::F; 4] {
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let ba = V::mul(V::unit(backdrop[3]), V::sub(one, amount));
    let ia = V::mul(V::unit(inner[3]), amount);
    let a = V::add(ba, ia);
    let mut out = [zero; 4];
    for c in 0..3 {
        let v = V::div(
            V::add(
                V::mul(V::unit(backdrop[c]), ba),
                V::mul(V::unit(inner[c]), ia),
            ),
            a,
        );
        out[c] = V::select(V::le(a, zero), zero, to_byte::<V>(v));
    }
    out[3] = V::select(V::le(a, zero), zero, to_byte::<V>(a));
    // 量が 1 以上は中身そのもの、0 以下は下そのまま（量 1 以上が先）
    let whole = V::ge(amount, one);
    let none = V::le(amount, zero);
    for c in 0..4 {
        out[c] = V::select(whole, inner[c], V::select(none, backdrop[c], out[c]));
    }
    out
}

// ───────── 画素ごとの関数（1 本のレーン） ─────────

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

#[inline(always)]
unsafe fn blend_one<V: Lanes32, const MODE: u8>(d: Rgba8, s: Rgba8, amount: f32) -> Rgba8 {
    blend_block::<Scalar1, MODE>(px(d), px(s), amount).map_or(d, rgba)
}
#[inline(always)]
unsafe fn clip_one<V: Lanes32, const MODE: u8>(d: Rgba8, s: Rgba8, amount: f32) -> Rgba8 {
    clip_block::<Scalar1, MODE>(px(d), px(s), amount).map_or(d, rgba)
}
#[inline(always)]
unsafe fn mix_one<V: Lanes32, const MODE: u8>(d: Rgba8, s: Rgba8, amount: f32) -> Rgba8 {
    mix_block::<Scalar1, MODE>(px(d), px(s), amount).map_or(d, rgba)
}

/// 画素ごとの合成（[`super::blend`] の中身）。行の核のスカラーの道と同じ関数。
pub(crate) fn blend_pixel(d: Rgba8, s: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { dispatch_mode!(mode, blend_one::<Scalar1>(d, s, amount as f32)) }
}
/// 画素ごとのクリッピング（[`super::clip_onto`] の中身）。
pub(crate) fn clip_pixel(d: Rgba8, s: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { dispatch_mode!(mode, clip_one::<Scalar1>(d, s, amount as f32)) }
}
/// 画素ごとの調整の合成（[`super::mix_rgb`] の中身）。
pub(crate) fn mix_pixel(d: Rgba8, s: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { dispatch_mode!(mode, mix_one::<Scalar1>(d, s, amount as f32)) }
}
/// 画素ごとのフェード（[`super::fade`] の中身）。
pub(crate) fn fade_pixel(d: Rgba8, s: Rgba8, amount: f64) -> Rgba8 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    rgba(unsafe { fade_block::<Scalar1>(px(d), px(s), amount as f32) })
}

// ───────── 行（V で N 画素ずつ、端は 1 本のレーン） ─────────

#[inline(always)]
unsafe fn blend_row_lanes<V: Lanes32, const MODE: u8>(
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
            if let Some(out) = blend_block::<V, MODE>(dst, src, amount.lanes::<V>(i)) {
                V::store(&mut res[i * 4..], out);
            }
            i += V::N;
        }
    }
    while i < count {
        let src = Scalar1::load(&sb[i * step..]);
        let dst = Scalar1::load(&res[i * 4..]);
        if let Some(out) = blend_block::<Scalar1, MODE>(dst, src, amount.lanes::<Scalar1>(i)) {
            Scalar1::store(&mut res[i * 4..], out);
        }
        i += 1;
    }
}

#[inline(always)]
unsafe fn clip_row_lanes<V: Lanes32, const MODE: u8>(
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
            if let Some(out) = clip_block::<V, MODE>(dst, src, amount.lanes::<V>(i)) {
                V::store(&mut g[i * 4..], out);
            }
            i += V::N;
        }
    }
    while i < count {
        let src = Scalar1::load(&cb[i * step..]);
        let dst = Scalar1::load(&g[i * 4..]);
        if let Some(out) = clip_block::<Scalar1, MODE>(dst, src, amount.lanes::<Scalar1>(i)) {
            Scalar1::store(&mut g[i * 4..], out);
        }
        i += 1;
    }
}

#[inline(always)]
unsafe fn mix_row_lanes<V: Lanes32, const MODE: u8>(
    res: &mut [u8],
    over: &[u8],
    amount: RowAmount<'_>,
) {
    let count = res.len() / 4;
    let mut i = 0;
    if V::N > 1 {
        while i + V::N <= count {
            let dst = V::load(&res[i * 4..]);
            let src = V::load(&over[i * 4..]);
            if let Some(out) = mix_block::<V, MODE>(dst, src, amount.lanes::<V>(i)) {
                V::store(&mut res[i * 4..], out);
            }
            i += V::N;
        }
    }
    while i < count {
        let dst = Scalar1::load(&res[i * 4..]);
        let src = Scalar1::load(&over[i * 4..]);
        if let Some(out) = mix_block::<Scalar1, MODE>(dst, src, amount.lanes::<Scalar1>(i)) {
            Scalar1::store(&mut res[i * 4..], out);
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

/// 下（res）に上（sb、刻み `step`。0 は 1 画素を全部に、4 は連続、ほかはスカラーの道）を重ねる。各画素は [`super::blend`] と同じバイト。
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
            dispatch_mode!(mode, blend_row_lanes::<Scalar1>(res, sb, step, amount))
        },
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn blend_row_avx2(
    res: &mut [u8],
    sb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    dispatch_mode!(mode, blend_row_lanes::<Avx2x8>(res, sb, step, amount))
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
    dispatch_mode!(mode, blend_row_lanes::<Sse41x4>(res, sb, step, amount))
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
    dispatch_mode!(mode, blend_row_lanes::<Neonx4>(res, sb, step, amount))
}

/// クリッピングの下地（g）へクリッピングされたレイヤー（cb、刻みは [`blend_row`] と同じ）を重ねる。各画素は [`super::clip_onto`] と同じバイト。
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
            dispatch_mode!(mode, clip_row_lanes::<Scalar1>(g, cb, step, amount))
        },
    }
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
    dispatch_mode!(mode, clip_row_lanes::<Avx2x8>(g, cb, step, amount))
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
    dispatch_mode!(mode, clip_row_lanes::<Sse41x4>(g, cb, step, amount))
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
    dispatch_mode!(mode, clip_row_lanes::<Neonx4>(g, cb, step, amount))
}

/// 調整した色（over、res と同じ並びの RGBA。アルファは見ない）を、下（res）へモードと量で混ぜる。各画素は [`super::mix_rgb`] と
/// 同じバイト（量が 0 以下・下が完全に透明な画素はそのまま）。
pub(crate) fn mix_row_at(
    level: Level,
    res: &mut [u8],
    over: &[u8],
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    match level.min(simd::detect()) {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2 を持つ
        Level::Avx2 => unsafe { mix_row_avx2(res, over, amount, mode) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { mix_row_sse41(res, over, amount, mode) },
        #[cfg(target_arch = "aarch64")]
        // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
        Level::Neon => unsafe { mix_row_neon(res, over, amount, mode) },
        // SAFETY: 1 本のレーンは CPU の前提を持たない
        Level::Scalar => unsafe {
            dispatch_mode!(mode, mix_row_lanes::<Scalar1>(res, over, amount))
        },
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn mix_row_avx2(res: &mut [u8], over: &[u8], amount: RowAmount<'_>, mode: BlendMode) {
    dispatch_mode!(mode, mix_row_lanes::<Avx2x8>(res, over, amount))
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn mix_row_sse41(res: &mut [u8], over: &[u8], amount: RowAmount<'_>, mode: BlendMode) {
    dispatch_mode!(mode, mix_row_lanes::<Sse41x4>(res, over, amount))
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn mix_row_neon(res: &mut [u8], over: &[u8], amount: RowAmount<'_>, mode: BlendMode) {
    dispatch_mode!(mode, mix_row_lanes::<Neonx4>(res, over, amount))
}

/// 通過のグループのフェード: res（下）と inner（中身、同じ並び）を量で補間して res へ。各画素は [`super::fade`] と同じバイト。
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

#[cfg(test)]
mod tests;
