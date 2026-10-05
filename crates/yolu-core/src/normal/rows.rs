//! Normal チャンネルの行の核（SIMD）。各画素は `blend_unchecked`・`clip_onto`・`fade` と出力の 1 画素の式（`flatten` → Sobel → `rnm` →
//! `encode`）と**同じバイト**になる。演算の順は画素ごとの式と同じで、正規化の平方根・割り算は IEEE の厳密な演算（SSE/AVX の
//! `sqrtpd`・`divpd`）なので同じ double になる。道の選びは `crate::math::simd`。
#![cfg_attr(
    not(target_arch = "x86_64"),
    allow(dead_code, unused_imports, unused_macros, unused_variables, unused_mut)
)]

use super::{
    blend_unchecked, clip_onto, fade, height_of, is_detail, Rgba8, DEGENERATE_LENGTH_SQUARED,
};
use crate::blend::RowAmount;
use crate::math::simd::{self, to_byte, Lanes, Level};
use crate::types::BlendMode;

#[cfg(target_arch = "x86_64")]
use crate::math::simd::{Avx2, Sse41};

type Vec3<V> = [<V as Lanes>::F; 3];

/// 読み元の画素の刻み（0 は 1 画素を全部に使う、4 は連続）だけを SIMD で扱う。
#[inline(always)]
fn simd_step(step: usize) -> bool {
    step == 0 || step == 4
}

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

// ───────── 層の重ね・クリッピング・フェード ─────────

/// N 画素の重ね（`blend_unchecked` と同じバイト）。`DETAIL` は Overlay（RNM）。
#[inline(always)]
unsafe fn blend_block<V: Lanes, const DETAIL: bool>(
    dst: [V::F; 4],
    src: [V::F; 4],
    opacity: V::F,
) -> Option<[V::F; 4]> {
    let one = V::splat(1.0);
    let t = V::mul(V::unit(src[3]), opacity);
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

/// N 画素のクリッピング（`clip_onto` と同じバイト）。
#[inline(always)]
unsafe fn clip_block<V: Lanes, const DETAIL: bool>(
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

/// N 画素のフェード（`fade` と同じバイト）。
#[inline(always)]
unsafe fn fade_block<V: Lanes>(backdrop: [V::F; 4], inner: [V::F; 4], amount: V::F) -> [V::F; 4] {
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

#[inline(always)]
unsafe fn blend_row_lanes<V: Lanes, const DETAIL: bool>(
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
        if let Some(out) = blend_block::<V, DETAIL>(dst, src, amount_lanes::<V>(&amount, i)) {
            V::store(&mut res[i * 4..], out);
        }
        i += V::N;
    }
    blend_from_scalar(res, sb, step, amount, mode, i);
}

#[inline(always)]
unsafe fn clip_row_lanes<V: Lanes, const DETAIL: bool>(
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
        if let Some(out) = clip_block::<V, DETAIL>(dst, src, amount_lanes::<V>(&amount, i)) {
            V::store(&mut g[i * 4..], out);
        }
        i += V::N;
    }
    clip_from_scalar(g, cb, step, amount, mode, i);
}

#[inline(always)]
unsafe fn fade_row_lanes<V: Lanes>(res: &mut [u8], inner: &[u8], amount: RowAmount<'_>) {
    let count = res.len() / 4;
    let mut i = 0;
    while i + V::N <= count {
        let out = fade_block::<V>(
            V::load(&res[i * 4..]),
            V::load(&inner[i * 4..]),
            amount_lanes::<V>(&amount, i),
        );
        V::store(&mut res[i * 4..], out);
        i += V::N;
    }
    fade_from_scalar(res, inner, amount, i);
}

/// レーン k に画素 i + k の量。
#[inline(always)]
unsafe fn amount_lanes<V: Lanes>(amount: &RowAmount<'_>, i: usize) -> V::F {
    match amount.mask {
        None => V::splat(amount.opacity),
        Some((m, step, f)) => V::mul(
            V::splat(amount.opacity),
            V::from_fn(|k| f[m[(i + k) * step + 3] as usize]),
        ),
    }
}

// ───────── スカラーの核（端と、SIMD の無い環境） ─────────

#[inline]
fn blend_from_scalar(
    res: &mut [u8],
    sb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
    from: usize,
) {
    for i in from..res.len() / 4 {
        let r = i * 4;
        let v = blend_unchecked(
            Rgba8::from_slice(&res[r..]),
            Rgba8::from_slice(&sb[i * step..]),
            amount.at(i),
            mode,
        );
        res[r..r + 4].copy_from_slice(&v.to_array());
    }
}

#[inline]
fn clip_from_scalar(
    g: &mut [u8],
    cb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
    from: usize,
) {
    for i in from..g.len() / 4 {
        let r = i * 4;
        let v = clip_onto(
            Rgba8::from_slice(&g[r..]),
            Rgba8::from_slice(&cb[i * step..]),
            amount.at(i),
            mode,
        );
        g[r..r + 4].copy_from_slice(&v.to_array());
    }
}

#[inline]
fn fade_from_scalar(res: &mut [u8], inner: &[u8], amount: RowAmount<'_>, from: usize) {
    for i in from..res.len() / 4 {
        let v = fade(
            Rgba8::from_slice(&res[i * 4..]),
            Rgba8::from_slice(&inner[i * 4..]),
            amount.at(i),
        );
        res[i * 4..i * 4 + 4].copy_from_slice(&v.to_array());
    }
}

// ───────── 入口 ─────────

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

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn blend_row_avx2(
    res: &mut [u8],
    sb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    with_detail!(mode, blend_row_lanes::<Avx2>(res, sb, step, amount, mode))
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
    with_detail!(mode, blend_row_lanes::<Sse41>(res, sb, step, amount, mode))
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
    with_detail!(mode, clip_row_lanes::<Avx2>(g, cb, step, amount, mode))
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
    with_detail!(mode, clip_row_lanes::<Sse41>(g, cb, step, amount, mode))
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

/// Normal のチャンネルで、下（res）に上（sb、刻み `step` は 0 か 4）をベクトルとして重ねる。各画素は [`super::blend`] と同じバイト。
#[inline]
pub fn blend_row(res: &mut [u8], sb: &[u8], step: usize, amount: RowAmount<'_>, mode: BlendMode) {
    blend_row_at(simd::level(), res, sb, step, amount, mode)
}

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
        _ => blend_from_scalar(res, sb, step, amount, mode, 0),
    }
}

/// Normal のチャンネルで、クリッピングの下地（g）へクリッピングされた層（cb）を重ねる。各画素は [`super::clip_onto`] と同じバイト。
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
        _ => clip_from_scalar(g, cb, step, amount, mode, 0),
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
        // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
        Level::Avx2 => unsafe { fade_row_avx2(res, inner, amount) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { fade_row_sse41(res, inner, amount) },
        _ => fade_from_scalar(res, inner, amount, 0),
    }
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
        _ => start,
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

/// 合成の行（RGBA）から高さ（R × A / 65025）の行を作る。
pub(super) fn heights_from_rgba(level: Level, src: &[u8], out: &mut [f64]) {
    let from = match level.min(simd::detect()) {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
        Level::Avx2 => unsafe { heights_avx2(src, out) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { heights_sse41(src, out) },
        _ => 0,
    };
    for (x, h) in out.iter_mut().enumerate().skip(from) {
        *h = height_of(src[x * 4], src[x * 4 + 3]);
    }
}

#[cfg(test)]
mod tests;
