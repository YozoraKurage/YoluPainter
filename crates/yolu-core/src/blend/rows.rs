//! 行（画素の並び）ごとの合成の核。各画素の結果は、画素ごとの式（[`super::blend`]・[`super::clip_onto`]・[`super::fade`]・
//! [`super::mix_rgb`]）と**同じバイト**になる。
//!
//! 道は 3 つ: AVX2（4 画素ずつ）・SSE4.1（2 画素ずつ）・スカラー（今までと同じ画素ごとの計算）。SIMD の道は端の画素（N で割った余り）を
//! スカラーの道と同じ関数で処理し、分離できるモードの 256×256 の表は引かずにレーンで式を計算する（表の値は式そのものなので同じ bit）。
//! 近道（上が透明・下が不透明・下が透明・Normal の量 1）は、レーンごとの条件に畳んで同じ結果を選ぶ。
#![cfg_attr(
    not(target_arch = "x86_64"),
    allow(dead_code, unused_imports, unused_macros, unused_variables, unused_mut)
)]

use super::lanes::{blend_rgb, dispatch_mode, is_simple};
use super::{MIN_SHORTCUT_ALPHA, UNIT};
use crate::math::simd::{self, to_byte, Lanes, Level};
use crate::math::to_byte as scalar_to_byte;
use crate::types::BlendMode;

#[cfg(target_arch = "x86_64")]
use crate::math::simd::{Avx2, Sse41};

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

    /// レーン k に画素 i + k の量。
    #[inline(always)]
    unsafe fn lanes<V: Lanes>(&self, i: usize) -> V::F {
        match self.mask {
            None => V::splat(self.opacity),
            Some((m, step, f)) => V::mul(
                V::splat(self.opacity),
                V::from_fn(|k| f[m[(i + k) * step + 3] as usize]),
            ),
        }
    }
}

/// スカラーの道が引く、分離できるモードの 256×256 の表（初めて使うときに作る）。分離できないモードは None。
#[inline]
fn scalar_table(mode: BlendMode) -> Option<&'static [f64]> {
    super::separable_table(mode)
}

/// 読み元の画素の刻み（0 は 1 画素を全部に使う、4 は連続）だけを SIMD で扱う。
#[inline(always)]
fn simd_step(step: usize) -> bool {
    step == 0 || step == 4
}

// ───────── スカラーの核（画素ごとの計算。SIMD の道の端と、SIMD の無い環境で使う） ─────────

/// 下（res）に上（sb）を重ねる（画素 `from` から。各画素は [`super::blend`] と同じバイト）。
#[inline]
fn blend_from_scalar(
    res: &mut [u8],
    sb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
    table: Option<&[f64]>,
    from: usize,
) {
    let simple = mode == BlendMode::Normal || mode == BlendMode::PassThrough;
    let count = res.len() / 4;
    for i in from..count {
        let r = i * 4;
        let s = i * step;
        let s_a = sb[s + 3];
        if s_a == 0 {
            continue; // 上が透明: 下のまま
        }
        let amount = amount.at(i);
        if simple && s_a == 255 && amount == 1.0 {
            res[r..r + 3].copy_from_slice(&sb[s..s + 3]);
            res[r + 3] = 255;
            continue;
        }
        let sa = UNIT[s_a as usize] * amount;
        if sa <= 0.0 {
            continue;
        }
        let d_a = res[r + 3];
        if d_a == 0 && sa >= MIN_SHORTCUT_ALPHA {
            // 下が透明: 重みは 0・a_s・0 で色は (a_s·c)/a_s。積が正規化数なら c から 2 ulp 以内で、丸めると c のバイト
            res[r..r + 3].copy_from_slice(&sb[s..s + 3]);
            res[r + 3] = scalar_to_byte(sa);
            continue;
        }
        let (dr, dg, db) = (
            UNIT[res[r] as usize],
            UNIT[res[r + 1] as usize],
            UNIT[res[r + 2] as usize],
        );
        let (sr, sg, sbb) = (
            UNIT[sb[s] as usize],
            UNIT[sb[s + 1] as usize],
            UNIT[sb[s + 2] as usize],
        );
        let (br, bg, bb) = if simple {
            (sr, sg, sbb)
        } else if let Some(t) = table {
            (
                t[(res[r] as usize) << 8 | sb[s] as usize],
                t[(res[r + 1] as usize) << 8 | sb[s + 1] as usize],
                t[(res[r + 2] as usize) << 8 | sb[s + 2] as usize],
            )
        } else {
            super::blend_rgb(mode, dr, dg, db, sr, sg, sbb)
        };
        let (vr, vg, vb);
        if d_a == 255 {
            // 下が不透明: a = a_s + (1 − a_s) はちょうど 1、重みは 1 − a_s・0・a_s（0 の項と ÷1 は値を変えない）
            let t = 1.0 - sa;
            vr = t * dr + sa * br;
            vg = t * dg + sa * bg;
            vb = t * db + sa * bb;
            res[r + 3] = 255;
        } else {
            let da = UNIT[d_a as usize];
            let a = sa + da * (1.0 - sa);
            let wd = (1.0 - sa) * da;
            let ws = (1.0 - da) * sa;
            let wb = da * sa;
            vr = (wd * dr + ws * sr + wb * br) / a;
            vg = (wd * dg + ws * sg + wb * bg) / a;
            vb = (wd * db + ws * sbb + wb * bb) / a;
            res[r + 3] = scalar_to_byte(a);
        }
        res[r] = scalar_to_byte(vr);
        res[r + 1] = scalar_to_byte(vg);
        res[r + 2] = scalar_to_byte(vb);
    }
}

/// クリッピングの下地（g）へクリッピングされた層（cb）を重ねる（画素 `from` から。各画素は [`super::clip_onto`] と同じバイト）。
#[inline]
fn clip_from_scalar(
    g: &mut [u8],
    cb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
    table: Option<&[f64]>,
    from: usize,
) {
    let simple = mode == BlendMode::Normal || mode == BlendMode::PassThrough;
    let count = g.len() / 4;
    for i in from..count {
        let r = i * 4;
        let s = i * step;
        let c_a = cb[s + 3];
        if c_a == 0 || g[r + 3] == 0 {
            continue; // 量 0、または描く下地が無い
        }
        let a = UNIT[c_a as usize] * amount.at(i);
        if a <= 0.0 {
            continue;
        }
        let (dr, dg, db) = (
            UNIT[g[r] as usize],
            UNIT[g[r + 1] as usize],
            UNIT[g[r + 2] as usize],
        );
        let (br, bg, bb) = if simple {
            (
                UNIT[cb[s] as usize],
                UNIT[cb[s + 1] as usize],
                UNIT[cb[s + 2] as usize],
            )
        } else if let Some(t) = table {
            (
                t[(g[r] as usize) << 8 | cb[s] as usize],
                t[(g[r + 1] as usize) << 8 | cb[s + 1] as usize],
                t[(g[r + 2] as usize) << 8 | cb[s + 2] as usize],
            )
        } else {
            super::blend_rgb(
                mode,
                dr,
                dg,
                db,
                UNIT[cb[s] as usize],
                UNIT[cb[s + 1] as usize],
                UNIT[cb[s + 2] as usize],
            )
        };
        g[r] = scalar_to_byte(dr + (br - dr) * a);
        g[r + 1] = scalar_to_byte(dg + (bg - dg) * a);
        g[r + 2] = scalar_to_byte(db + (bb - db) * a);
    }
}

/// 調整した色（over、同じ並びの RGBA。アルファは見ない）を下（res）へモードと量で混ぜる（画素 `from` から。量が 0 以下・下が完全に
/// 透明な画素はそのまま、ほかは [`super::mix_rgb`] と同じバイト。アルファは下のまま）。
#[inline]
fn mix_from_scalar(
    res: &mut [u8],
    over: &[u8],
    amount: RowAmount<'_>,
    mode: BlendMode,
    from: usize,
) {
    use crate::types::Rgba8;
    for i in from..res.len() / 4 {
        let a = amount.at(i);
        let below = Rgba8::from_slice(&res[i * 4..]);
        if a <= 0.0 || below.a == 0 {
            continue;
        }
        let v = super::mix_rgb(below, Rgba8::from_slice(&over[i * 4..]), a, mode);
        res[i * 4..i * 4 + 4].copy_from_slice(&v.to_array());
    }
}

/// 通過のグループのフェード（画素 `from` から。各画素は [`super::fade`] と同じバイト）。
#[inline]
fn fade_from_scalar(res: &mut [u8], inner: &[u8], amount: RowAmount<'_>, from: usize) {
    use crate::types::Rgba8;
    for i in from..res.len() / 4 {
        let v = super::fade(
            Rgba8::from_slice(&res[i * 4..]),
            Rgba8::from_slice(&inner[i * 4..]),
            amount.at(i),
        );
        res[i * 4..i * 4 + 4].copy_from_slice(&v.to_array());
    }
}

// ───────── レーンの核 ─────────

/// N 画素の合成（下 dst に上 src を量 amount で）。レーンごとの結果は `blend_from_scalar` の 1 画素と同じバイト。
#[inline(always)]
pub(crate) unsafe fn blend_block<V: Lanes, const MODE: u8>(
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

#[inline(always)]
unsafe fn blend_row_lanes<V: Lanes, const MODE: u8>(
    res: &mut [u8],
    sb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    let count = res.len() / 4;
    let uniform = if step == 0 {
        Some(V::splat_px(sb[..4].try_into().unwrap()))
    } else {
        None
    };
    let mut i = 0;
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
    blend_from_scalar(res, sb, step, amount, mode, None, i);
}

/// N 画素のクリッピング（下地 dst へ上 src を量 amount で。下地のアルファは変えない）。
#[inline(always)]
unsafe fn clip_block<V: Lanes, const MODE: u8>(
    dst: [V::F; 4],
    src: [V::F; 4],
    amount: V::F,
) -> Option<[V::F; 4]> {
    let zero = V::splat(0.0);
    let a = V::mul(V::unit(src[3]), amount);
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

#[inline(always)]
unsafe fn clip_row_lanes<V: Lanes, const MODE: u8>(
    g: &mut [u8],
    cb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    let count = g.len() / 4;
    let uniform = if step == 0 {
        Some(V::splat_px(cb[..4].try_into().unwrap()))
    } else {
        None
    };
    let mut i = 0;
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
    clip_from_scalar(g, cb, step, amount, mode, None, i);
}

/// N 画素の調整の合成（下 dst と調整後の色 over をモードと量で。アルファは下のまま）。
#[inline(always)]
unsafe fn mix_block<V: Lanes, const MODE: u8>(
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

#[inline(always)]
unsafe fn mix_row_lanes<V: Lanes, const MODE: u8>(
    res: &mut [u8],
    over: &[u8],
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    let count = res.len() / 4;
    let mut i = 0;
    while i + V::N <= count {
        let dst = V::load(&res[i * 4..]);
        let src = V::load(&over[i * 4..]);
        if let Some(out) = mix_block::<V, MODE>(dst, src, amount.lanes::<V>(i)) {
            V::store(&mut res[i * 4..], out);
        }
        i += V::N;
    }
    mix_from_scalar(res, over, amount, mode, i);
}

/// N 画素のフェード（下 backdrop と中身 inner をプリマルチプライドで補間）。
#[inline(always)]
pub(crate) unsafe fn fade_block<V: Lanes>(backdrop: [V::F; 4], inner: [V::F; 4], amount: V::F) -> [V::F; 4] {
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
    // 量が 1 以上は中身そのもの、0 以下は下そのまま（スカラーの早い戻り。量 1 以上が先）
    let whole = V::ge(amount, one);
    let none = V::le(amount, zero);
    for c in 0..4 {
        out[c] = V::select(whole, inner[c], V::select(none, backdrop[c], out[c]));
    }
    out
}

#[inline(always)]
unsafe fn fade_row_lanes<V: Lanes>(res: &mut [u8], inner: &[u8], amount: RowAmount<'_>) {
    let count = res.len() / 4;
    let mut i = 0;
    while i + V::N <= count {
        let out = fade_block::<V>(
            V::load(&res[i * 4..]),
            V::load(&inner[i * 4..]),
            amount.lanes::<V>(i),
        );
        V::store(&mut res[i * 4..], out);
        i += V::N;
    }
    fade_from_scalar(res, inner, amount, i);
}

// ───────── 入口 ─────────

/// 下（res）に上（sb、刻み `step` は 0 か 4）を重ねる。各画素は [`super::blend`] と同じバイト。
/// 分離できるモードの 256×256 の表（`separable_table`）は、スカラーの道だけが引く（SIMD の道は式をレーンで計算するので作らない）。
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
        // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
        Level::Avx2 => unsafe { blend_row_avx2(res, sb, step, amount, mode) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { blend_row_sse41(res, sb, step, amount, mode) },
        _ => blend_from_scalar(res, sb, step, amount, mode, scalar_table(mode), 0),
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
    dispatch_mode!(mode, blend_row_lanes::<Avx2>(res, sb, step, amount, mode))
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
    dispatch_mode!(mode, blend_row_lanes::<Sse41>(res, sb, step, amount, mode))
}

/// クリッピングの下地（g）へクリッピングされた層（cb、刻みは 0 か 4）を重ねる。各画素は [`super::clip_onto`] と同じバイト。
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
        // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
        Level::Avx2 => unsafe { clip_row_avx2(g, cb, step, amount, mode) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { clip_row_sse41(g, cb, step, amount, mode) },
        _ => clip_from_scalar(g, cb, step, amount, mode, scalar_table(mode), 0),
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
    dispatch_mode!(mode, clip_row_lanes::<Avx2>(g, cb, step, amount, mode))
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
    dispatch_mode!(mode, clip_row_lanes::<Sse41>(g, cb, step, amount, mode))
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
        // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
        Level::Avx2 => unsafe { mix_row_avx2(res, over, amount, mode) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { mix_row_sse41(res, over, amount, mode) },
        _ => mix_from_scalar(res, over, amount, mode, 0),
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn mix_row_avx2(res: &mut [u8], over: &[u8], amount: RowAmount<'_>, mode: BlendMode) {
    dispatch_mode!(mode, mix_row_lanes::<Avx2>(res, over, amount, mode))
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn mix_row_sse41(res: &mut [u8], over: &[u8], amount: RowAmount<'_>, mode: BlendMode) {
    dispatch_mode!(mode, mix_row_lanes::<Sse41>(res, over, amount, mode))
}

/// 通過のグループのフェード: res（下）と inner（中身、同じ並び）を量で補間して res へ。各画素は [`super::fade`] と同じバイト。
#[inline]
pub fn fade_row(res: &mut [u8], inner: &[u8], amount: RowAmount<'_>) {
    fade_row_at(simd::level(), res, inner, amount)
}

pub(crate) fn fade_row_at(level: Level, res: &mut [u8], inner: &[u8], amount: RowAmount<'_>) {
    match level.min(simd::detect()) {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
        Level::Avx2 => unsafe { fade_row_avx2(res, inner, amount) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { fade_row_sse41(res, inner, amount) },
        _ => fade_from_scalar(res, inner, amount, 0),
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn fade_row_avx2(res: &mut [u8], inner: &[u8], amount: RowAmount<'_>) {
    fade_row_lanes::<Avx2>(res, inner, amount)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn fade_row_sse41(res: &mut [u8], inner: &[u8], amount: RowAmount<'_>) {
    fade_row_lanes::<Sse41>(res, inner, amount)
}

#[cfg(test)]
mod tests;
