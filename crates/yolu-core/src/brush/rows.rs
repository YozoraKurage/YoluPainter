//! ダブの画素を行ごとに（レーン: AVX2 は 4 画素・SSE4.1 は 2 画素）処理する核。ステンシルを使わないブラシ（色を塗る・消す、ダブごとの色・
//! 色の混ぜ、読み元の枠から読む効果）の、`dab_tile_with` の画素ごとの式（スカラーの道）と**同じバイト**を出す。選択範囲・透明部分のロックは、
//! 色を塗る・消すだけのブラシなら行の核が受け持ち、画素ごとの色・効果のブラシでは画素ごとの式のまま。ステンシル・乗算でない紙の質感・
//! 面のダブの写像も画素ごとの式のまま（[`usable`]）。
//!
//! 画素の仕事を 2 段に分ける。
//! 1. 覆いの行（[`cover_row`]）: 丸（回転・潰しがあっても）はレーンで測り、筆先の画像は 4 つの texel を画素ごとに集めて双線形をレーンで
//!    計算する。デュアルの合わせと紙の質感（乗算）も覆いの行へレーンで掛ける。
//! 2. 当ての行: ストロークの覆い（`wash`）への寄せ・描く前のタイルとの合成・面への書き込みをレーンで行う。色を塗る・消すは
//!    [`apply_range`]、指先・クローン・ぼかしは [`effect`]、ダブごとの色・色の混ぜは [`color`]。合成は
//!    [`crate::blend::blend_block`]（合成の行の核と同じ式）で、近道（上が不透明で量 1・下が透明）も同じ条件で選ぶ。
//!
//! 演算の順はスカラーの式と同じ（積和へまとめない、`a * b * c` は `(a * b) * c`）。丸の覆いは、距離の 2 乗で確実に硬さの内側・確実に
//! 外の画素を、平方根と割り算なしで決める（余裕 1e-9 は丸めの誤差よりずっと大きい）。行は、円か筆先の矩形にかかる区間へ狭める
//! （外側へ余裕を取る。範囲の外は必ず覆い 0 になるので、飛ばしても画素は変わらない）。端の画素（レーンの数で割った余り）と、読む位置が
//! 画布の端にかかる効果のブロックは、スカラーの `apply_at` に渡す。面のタイルが一様・共有・無いときは、変わる画素があるブロックの
//! 最初の書き込みを `LiveTile::write` で行い（予算の確かめと自分のものにする複製は今までと同じ）、あとはタイルの中へ直接書く。

#![cfg_attr(
    not(target_arch = "x86_64"),
    allow(dead_code, unused_imports, unused_macros, unused_variables, unused_mut)
)]

use super::*;
use crate::blend::lanes::NORMAL;
use crate::blend::{blend_block, fade_block};
use crate::math::simd::{self, to_byte, Lanes, Level};

#[cfg(target_arch = "x86_64")]
use crate::math::simd::{Avx2, Sse41};
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

mod color;
mod effect;
use color::apply_color_range;
use effect::{apply_effect_range, BLUR, CLONE, SMUDGE};

/// 行の長さの上限（これを超えるタイルの大きさは行の核を使わない。タイルの大きさは 1〜1024）。
const MAX_ROW: usize = 1024;
/// 行の覆い・紙の質感の値を、スタックに置く行の長さ（これより長い行はヒープ）。
const STACK_ROW: usize = 256;

/// f32 の並び（ストロークの覆い `wash`）を f64 のレーンへ読み書きする。f32 → f64 は厳密で、f64 → f32 は `as f32` と同じ最近接丸め。
trait F32Lanes: Lanes {
    /// 連続する N 個の f32 をレーンへ。
    unsafe fn load_f32(p: &[f32]) -> Self::F;
    /// レーンを連続する N 個の f32 へ。
    unsafe fn store_f32(p: &mut [f32], v: Self::F);
    /// f32 に丸めた値（`as f32` と同じ最近接丸め）を f64 のレーンで返す。
    unsafe fn round_f32(v: Self::F) -> Self::F;
}

#[cfg(target_arch = "x86_64")]
impl F32Lanes for Avx2 {
    #[inline(always)]
    unsafe fn load_f32(p: &[f32]) -> __m256d {
        let q: &[f32; 4] = p[..4].try_into().unwrap();
        unsafe { _mm256_cvtps_pd(_mm_loadu_ps(q.as_ptr())) }
    }
    #[inline(always)]
    unsafe fn store_f32(p: &mut [f32], v: __m256d) {
        let q: &mut [f32; 4] = (&mut p[..4]).try_into().unwrap();
        unsafe { _mm_storeu_ps(q.as_mut_ptr(), _mm256_cvtpd_ps(v)) }
    }
    #[inline(always)]
    unsafe fn round_f32(v: __m256d) -> __m256d {
        unsafe { _mm256_cvtps_pd(_mm256_cvtpd_ps(v)) }
    }
}

#[cfg(target_arch = "x86_64")]
impl F32Lanes for Sse41 {
    #[inline(always)]
    unsafe fn load_f32(p: &[f32]) -> __m128d {
        let q: &[f32; 2] = p[..2].try_into().unwrap();
        unsafe { _mm_cvtps_pd(_mm_castsi128_ps(_mm_loadl_epi64(q.as_ptr().cast()))) }
    }
    #[inline(always)]
    unsafe fn store_f32(p: &mut [f32], v: __m128d) {
        let q: &mut [f32; 2] = (&mut p[..2]).try_into().unwrap();
        unsafe { _mm_storel_epi64(q.as_mut_ptr().cast(), _mm_castps_si128(_mm_cvtpd_ps(v))) }
    }
    #[inline(always)]
    unsafe fn round_f32(v: __m128d) -> __m128d {
        unsafe { _mm_cvtps_pd(_mm_cvtpd_ps(v)) }
    }
}

/// このタイルを行の核で描けるなら、使う道（`Avx2` か `Sse41`）を返す。道がスカラーでなく、紙の質感が乗算か無く、ステンシルを使わないこと。
/// そして、色を塗る・消す（選択範囲・透明部分のロックがあってもよい）か、選択範囲・ロックを使わない画素ごとの色（ダブごとの色・色の混ぜ）
/// か読み元の枠から読む効果（ぼかし・指先・クローン）で、面のダブの写像でないこと。
///
/// 道は 1 度だけ読み、呼び手は返った道をそのまま [`dab_tile`] に渡す（判断と実行で別々に読むと、その間に道が変わったとき
/// [`dab_tile`] がスカラーの道に当たる。試験は道を切り替えるので、並んで走る別の試験のストロークで起こりうる）。
pub(super) fn usable(cx: &PixelContext<'_>, s: &DabShape<'_>) -> Option<Level> {
    let level = simd::level();
    eligible(
        cx.paint,
        s,
        matches!(cx.selected, Selected::Everywhere),
        cx.tile_size,
        level,
    )
    .then_some(level)
}

/// [`usable`] と [`cost_per_pixel`] が共有する、行の核を使えるかの判定（`level` は呼び手が読んだ道）。
fn eligible(
    p: &Paint<'_>,
    s: &DabShape<'_>,
    no_selection: bool,
    tile_size: usize,
    level: Level,
) -> bool {
    let kind_ok = p.stencil.is_none()
        && p.mapped.is_none()
        && match p.effect {
            // 混ぜるブラシは下地の枠が要る
            EffectKind::Paint => p.mix.is_none() || p.frame.is_some(),
            _ => !p.tip_colors && p.mix.is_none() && p.frame.is_some(),
        };
    kind_ok
        && level != Level::Scalar
        && tile_size <= MAX_ROW
        && (is_plain(p) || (!p.keep_alpha && no_selection))
        && s.texture.is_none_or(|t| t.mode.as_blend().is_none())
}

/// 色を塗る・消すだけ（効果・画素ごとの色なし）のブラシか。選択範囲と透明部分のロックは、このブラシだけ行の核が受け持つ。
fn is_plain(p: &Paint<'_>) -> bool {
    p.effect == EffectKind::Paint && !p.tip_colors
}

/// このダブの 1 画素あたりの時間の見積もり（ナノ秒。ワーカーで描くかの判断の重み）。行の核を使えない（スカラーの道・ステンシル・
/// 紙の質感が乗算以外・面のダブの写像・画素ごとの色や効果のブラシの選択範囲・透明部分のロック）ときは画素ごとの式の 30。使えるときは、硬い丸 1・柔らかい丸と
/// 筆先の画像 3（デュアルは +1、紙の質感は +4）・効果と画素ごとの色 14。1 スレッドの計測（硬い丸 1.9・柔らかい丸 3.7・筆先 3.1・
/// 質感 7・デュアル 4・指先 12・混ぜる 17 ナノ秒 / 画素）より、直列の方がわずかに速い側へ寄せてある（ワーカーを使う損の方が大きいため）。
pub(super) fn cost_per_pixel(
    paint: &Paint<'_>,
    s: &DabShape<'_>,
    no_selection: bool,
    tile_size: usize,
) -> i64 {
    if !eligible(paint, s, no_selection, tile_size, simd::level()) {
        return SCALAR_PIXEL_NANOS;
    }
    if paint.effect != EffectKind::Paint || paint.tip_colors {
        return 14;
    }
    let mut cost = if s.tip.is_some() || s.hardness < 1.0 {
        3
    } else {
        1
    };
    if s.dual.is_some() {
        cost += 1;
    }
    if s.texture.is_some() {
        cost += 4;
    }
    cost
}

/// 行の核で 1 枚のタイルのダブの画素を描く（`level` は [`usable`] が返した道。結果は `dab_tile_with` と同じ）。
#[allow(clippy::too_many_arguments)]
pub(super) fn dab_tile(
    level: Level,
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dual: Option<&[f32]>,
    s: &DabShape<'_>,
    coord: TileCoord,
    xs: (i64, i64),
    ys: (i64, i64),
) -> Result<bool, CoreError> {
    match level {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level() は detect() 以下なので、AVX2 と FMA を持つ
        Level::Avx2 => unsafe { tile_avx2(cx, held, live, dual, s, coord, xs, ys) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level() は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { tile_sse41(cx, held, live, dual, s, coord, xs, ys) },
        _ => unreachable!("usable() はスカラーの道を返さない"),
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(clippy::too_many_arguments)]
unsafe fn tile_avx2(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dual: Option<&[f32]>,
    s: &DabShape<'_>,
    coord: TileCoord,
    xs: (i64, i64),
    ys: (i64, i64),
) -> Result<bool, CoreError> {
    unsafe { tile::<Avx2>(cx, held, live, dual, s, coord, xs, ys) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
#[allow(clippy::too_many_arguments)]
unsafe fn tile_sse41(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dual: Option<&[f32]>,
    s: &DabShape<'_>,
    coord: TileCoord,
    xs: (i64, i64),
    ys: (i64, i64),
) -> Result<bool, CoreError> {
    unsafe { tile::<Sse41>(cx, held, live, dual, s, coord, xs, ys) }
}

/// 紙の質感の行ごとの値（乗算の紙の質感があるとき、画素ごとの不透明度の係数）。
enum Scale<'a> {
    /// ダブで 1 つ（質感なし）。
    Const(f64),
    Row(&'a [f64]),
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
unsafe fn tile<V: F32Lanes>(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dual: Option<&[f32]>,
    s: &DabShape<'_>,
    coord: TileCoord,
    xs: (i64, i64),
    ys: (i64, i64),
) -> Result<bool, CoreError> {
    // 覆いと紙の質感の行の置き場。小さなダブでゼロ埋めの費用が目立たないよう、行の長さで置き場の大きさを選ぶ
    let n = (xs.1 - xs.0 + 1) as usize;
    macro_rules! with_buffers {
        ($len:expr) => {{
            let (mut cov, mut scale) = ([0.0f64; $len], [0.0f64; $len]);
            rows::<V>(
                cx,
                held,
                live,
                dual,
                s,
                coord,
                xs,
                ys,
                &mut cov[..n],
                &mut scale[..n],
            )
        }};
    }
    if n <= 32 {
        with_buffers!(32)
    } else if n <= 128 {
        with_buffers!(128)
    } else if n <= STACK_ROW {
        with_buffers!(STACK_ROW)
    } else {
        let (mut cov, mut scale) = (vec![0.0f64; n], vec![0.0f64; n]);
        rows::<V>(cx, held, live, dual, s, coord, xs, ys, &mut cov, &mut scale)
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
unsafe fn rows<V: F32Lanes>(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dual: Option<&[f32]>,
    s: &DabShape<'_>,
    coord: TileCoord,
    xs: (i64, i64),
    ys: (i64, i64),
    cov: &mut [f64],
    scale: &mut [f64],
) -> Result<bool, CoreError> {
    let ts = cx.tile_size as i64;
    let (ox, oy) = (coord.x as i64 * ts, coord.y as i64 * ts);
    let mut changed = false;
    for py in ys.0..=ys.1 {
        let row = ((py - oy) * ts) as usize;
        let x0 = (xs.0 - ox) as usize;
        let (lo, hi) = unsafe { cover_row::<V>(s, py, xs, cov) };
        if lo >= hi {
            continue;
        }
        // デュアル・質感は覆いの行へ掛ける（式はスカラーの `dab_tile_with` と同じ）
        if let Some(mode) = s.dual {
            unsafe { dual_row::<V>(mode, dual, cov, lo, hi, row + x0) };
        }
        let with_texture = s.texture.is_some();
        if let Some(tex) = s.texture {
            unsafe { texture_row::<V>(s, tex, py, xs.0, cov, scale, lo, hi) };
        }
        let scale = if with_texture {
            Scale::Row(&scale[..])
        } else {
            Scale::Const(s.opacity_scale)
        };
        let place = (row, x0);
        changed |= unsafe {
            match cx.paint.effect {
                EffectKind::Paint if cx.paint.tip_colors => apply_color_range::<V>(
                    cx,
                    held,
                    live,
                    coord,
                    place,
                    cov,
                    &scale,
                    (lo, hi),
                    s,
                    (xs.0, py),
                )?,
                EffectKind::Paint => {
                    apply_range::<V>(cx, held, live, coord, row, x0, cov, &scale, lo, hi, s)?
                }
                EffectKind::Smudge(_) => apply_effect_range::<V, SMUDGE>(
                    cx,
                    held,
                    live,
                    coord,
                    place,
                    cov,
                    &scale,
                    (lo, hi),
                    s,
                    (xs.0, py),
                )?,
                EffectKind::Clone => apply_effect_range::<V, CLONE>(
                    cx,
                    held,
                    live,
                    coord,
                    place,
                    cov,
                    &scale,
                    (lo, hi),
                    s,
                    (xs.0, py),
                )?,
                EffectKind::Blur(_) => apply_effect_range::<V, BLUR>(
                    cx,
                    held,
                    live,
                    coord,
                    place,
                    cov,
                    &scale,
                    (lo, hi),
                    s,
                    (xs.0, py),
                )?,
            }
        };
    }
    Ok(changed)
}

/// 画素 px の覆い（0 以下は塗らない。`dab_tile_with` の丸と筆先の画像の式そのもの）。
#[inline(always)]
fn cover_pixel(s: &DabShape<'_>, dx: f64, dy: f64) -> f64 {
    match s.tip {
        None => {
            let d = if s.plain {
                (dx * dx + dy * dy).sqrt() / s.radius
            } else {
                let u = (s.cos * dx + s.sin * dy) / s.radius;
                let v = (-s.sin * dx + s.cos * dy) / (s.radius * s.roundness);
                (u * u + v * v).sqrt()
            };
            if d > 1.0 {
                return 0.0;
            }
            let mut coverage = 1.0;
            if d > s.hardness {
                let t = (1.0 - d) / (1.0 - s.hardness);
                coverage = t * t * (3.0 - 2.0 * t);
            }
            coverage
        }
        Some(tip) => {
            let mut u = (s.cos * dx + s.sin * dy) / s.radius;
            let mut v = (-s.sin * dx + s.cos * dy) / (s.radius * s.roundness);
            if s.flip_x {
                u = -u;
            }
            if s.flip_y {
                v = -v;
            }
            tip.sample((u / s.aspect_x + 1.0) * 0.5, (v / s.aspect_y + 1.0) * 0.5)
        }
    }
}

/// 筆先の画像の画素（範囲の外は 0。`BrushTip` の双線形の読みの texel と同じ）。
#[inline(always)]
fn tip_texel(alpha: &[u8], w: i32, h: i32, x: i32, y: i32) -> f64 {
    if x < 0 || y < 0 || x >= w || y >= h {
        0.0
    } else {
        alpha[(y * w + x) as usize] as f64
    }
}

/// 筆先の画像の矩形（回転・潰し・反転をかけた後、画像は規格化した座標の |u| ≤ aspect_x・|v| ≤ aspect_y）が、行（中心からの縦のずれ dy）と
/// 交わる画素の x の範囲（画素の座標の実数。余裕を足してある）。行が矩形と交わらなければ空の範囲。どの画素も範囲の外なら必ず
/// 画像の外（覆い 0）。係数がほぼ 0 の向き（回転がちょうど 90 度のときの cos など）は、その向きでは狭めない。
fn tip_span(s: &DabShape<'_>, dy: f64) -> Option<(f64, f64)> {
    // |a * dx + b| ≤ reach を満たす dx の範囲（None は制限なし、Some(空) は満たさない）
    fn range(a: f64, b: f64, reach: f64) -> Option<(f64, f64)> {
        if a.abs() < 1e-9 {
            return if b.abs() <= reach + 1e-3 {
                None
            } else {
                Some((1.0, 0.0))
            };
        }
        let (p, q) = ((-reach - b) / a, (reach - b) / a);
        Some(if p < q { (p, q) } else { (q, p) })
    }
    let reach_u = s.aspect_x * s.radius;
    let reach_v = s.aspect_y * s.radius * s.roundness;
    let by_u = range(s.cos, s.sin * dy, reach_u);
    let by_v = range(-s.sin, s.cos * dy, reach_v);
    let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
    for (a, b) in [by_u, by_v].into_iter().flatten() {
        lo = lo.max(a);
        hi = hi.min(b);
    }
    if lo == f64::NEG_INFINITY && hi == f64::INFINITY {
        return None;
    }
    // 余裕: 2 画素（と、範囲の大きさの 1e-9 倍）。範囲が空（lo > hi）でも余裕を足して比べる
    let margin = 2.0 + 1e-9 * (lo.abs().min(1e15) + hi.abs().min(1e15));
    let (lo, hi) = (lo - margin, hi + margin);
    if lo > hi {
        return Some((1.0, 0.0));
    }
    Some((s.x - 0.5 + lo, s.x - 0.5 + hi))
}

/// 行 py の覆いを cov[0..n) に入れ、塗る可能性のある区間 [lo, hi) を返す（区間の外の cov は読まない）。
#[inline(always)]
unsafe fn cover_row<V: Lanes>(
    s: &DabShape<'_>,
    py: i64,
    xs: (i64, i64),
    cov: &mut [f64],
) -> (usize, usize) {
    let n = (xs.1 - xs.0 + 1) as usize;
    let dy = (py as f64 + 0.5) - s.y;
    let (mut lo, mut hi) = (0usize, n);
    // 塗る可能性のある区間（それより外の画素は必ず覆い 0）に狭める。余裕は、外れた判定の丸めを吸う分
    let span = match s.tip {
        // 丸の外の画素は d > 1 で塗らない。行が円（半径 radius。潰した丸は半径が小さいだけ）にかかる区間。回転・潰しのときは外接の
        // 半径 radius の円で足りる
        None => {
            let reach = s.radius * s.roundness.max(1.0) + 1.5;
            if dy.abs() > reach {
                return (0, 0);
            }
            let half = (reach * reach - dy * dy).max(0.0).sqrt() + 1.5;
            Some((s.x - 0.5 - half, s.x - 0.5 + half))
        }
        Some(_) => tip_span(s, dy),
    };
    if let Some((first, last)) = span {
        let (first, last) = (first.ceil(), last.floor());
        lo = ((first - xs.0 as f64).max(0.0)) as usize;
        hi = (((last - xs.0 as f64) + 1.0).min(n as f64).max(0.0)) as usize;
        if lo >= hi {
            return (0, 0);
        }
    }
    let mut i = lo;
    let half = V::splat(0.5);
    let (sx, dyv) = (V::splat(s.x), V::splat(dy));
    let (radius, hardness) = (V::splat(s.radius), V::splat(s.hardness));
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let (two, three) = (V::splat(2.0), V::splat(3.0));
    let inner = V::splat(1.0 - s.hardness);
    let (cos, sin, nsin) = (V::splat(s.cos), V::splat(s.sin), V::splat(-s.sin));
    let squash = V::splat(s.radius * s.roundness);
    let mut pxv = V::add(
        V::splat((xs.0 + lo as i64) as f64),
        V::from_fn(|k| k as f64),
    );
    let step = V::splat(V::N as f64);
    match s.tip {
        None => {
            // 距離の 2 乗で、確実に硬さの内側（覆い 1）・確実に外（覆い 0）の画素を、平方根と割り算なしで決める。余裕の 1e-9 は d の丸めの誤差
            // （5e-16 ほど）よりずっと大きいので、この 2 つに入る画素の `d` は、スカラーの式でも `d <= hardness`・`d > 1` になる。
            // 縁（硬さから 1 まで）にかかる画素のあるブロックだけ、`d` と滑らかな縁を正確に測る
            let inner_limit = V::splat(s.hardness * s.hardness * (1.0 - 1e-9));
            let outer_limit = V::splat(1.0 + 1e-9);
            let (inv_radius2, inv_squash2) = (
                V::splat(1.0 / (s.radius * s.radius)),
                V::splat(1.0 / (s.radius * s.roundness * s.radius * s.roundness)),
            );
            while i + V::N <= hi {
                let dx = V::sub(V::add(pxv, half), sx);
                // 規格化した距離の 2 乗（近似でよい。判定の余裕が誤差を吸う）
                let approx = if s.plain {
                    V::mul(V::add(V::mul(dx, dx), V::mul(dyv, dyv)), inv_radius2)
                } else {
                    let ru = V::add(V::mul(cos, dx), V::mul(sin, dyv));
                    let rv = V::add(V::mul(nsin, dx), V::mul(cos, dyv));
                    V::add(
                        V::mul(V::mul(ru, ru), inv_radius2),
                        V::mul(V::mul(rv, rv), inv_squash2),
                    )
                };
                let inside = V::lt(approx, inner_limit);
                let outside = V::gt(approx, outer_limit);
                if V::all(V::or(inside, outside)) {
                    V::store_f64(&mut cov[i..], V::select(inside, one, zero));
                } else {
                    let d = if s.plain {
                        V::div(V::sqrt(V::add(V::mul(dx, dx), V::mul(dyv, dyv))), radius)
                    } else {
                        let u = V::div(V::add(V::mul(cos, dx), V::mul(sin, dyv)), radius);
                        let v = V::div(V::add(V::mul(nsin, dx), V::mul(cos, dyv)), squash);
                        V::sqrt(V::add(V::mul(u, u), V::mul(v, v)))
                    };
                    let t = V::div(V::sub(one, d), inner);
                    let smooth = V::mul(V::mul(t, t), V::sub(three, V::mul(two, t)));
                    let c = V::select(
                        V::gt(d, one),
                        zero,
                        V::select(V::gt(d, hardness), smooth, one),
                    );
                    V::store_f64(&mut cov[i..], c);
                }
                i += V::N;
                pxv = V::add(pxv, step);
            }
        }
        Some(tip) => {
            let (w, h) = (tip.width() as i32, tip.height() as i32);
            let alpha = tip.alpha();
            let (wf, hf) = (V::splat(w as f64), V::splat(h as f64));
            let (aspect_x, aspect_y) = (V::splat(s.aspect_x), V::splat(s.aspect_y));
            let m255 = V::splat(255.0);
            while i + V::N <= hi {
                let dx = V::sub(V::add(pxv, half), sx);
                let mut u = V::div(V::add(V::mul(cos, dx), V::mul(sin, dyv)), radius);
                let mut v = V::div(V::add(V::mul(nsin, dx), V::mul(cos, dyv)), squash);
                if s.flip_x {
                    u = V::neg(u);
                }
                if s.flip_y {
                    v = V::neg(v);
                }
                let tu = V::mul(V::add(V::div(u, aspect_x), one), half);
                let tv = V::mul(V::add(V::div(v, aspect_y), one), half);
                // [0, 1]² の外は 0
                let outside = V::or(
                    V::or(V::lt(tu, zero), V::lt(tv, zero)),
                    V::or(V::gt(tu, one), V::gt(tv, one)),
                );
                let x = V::sub(V::mul(tu, wf), half);
                let y = V::sub(V::mul(tv, hf), half);
                let (x0, y0) = (V::floor(x), V::floor(y));
                let (fx, fy) = (V::sub(x, x0), V::sub(y, y0));
                let (mut xi, mut yi) = ([0.0f64; 4], [0.0f64; 4]);
                V::store_f64(&mut xi, x0);
                V::store_f64(&mut yi, y0);
                // 4 つの texel が全部画像の中なら、範囲の確かめなしで読む
                let interior = V::all(V::and(
                    V::and(V::ge(x0, zero), V::lt(V::add(x0, one), wf)),
                    V::and(V::ge(y0, zero), V::lt(V::add(y0, one), hf)),
                ));
                let (a, b, c, d);
                if interior {
                    let at = |k: usize| yi[k] as usize * w as usize + xi[k] as usize;
                    let stride = w as usize;
                    a = V::from_fn(|k| alpha[at(k)] as f64);
                    b = V::from_fn(|k| alpha[at(k) + 1] as f64);
                    c = V::from_fn(|k| alpha[at(k) + stride] as f64);
                    d = V::from_fn(|k| alpha[at(k) + stride + 1] as f64);
                } else {
                    let at = |dxi: i32, dyi: i32, k: usize| {
                        tip_texel(alpha, w, h, xi[k] as i32 + dxi, yi[k] as i32 + dyi)
                    };
                    a = V::from_fn(|k| at(0, 0, k));
                    b = V::from_fn(|k| at(1, 0, k));
                    c = V::from_fn(|k| at(0, 1, k));
                    d = V::from_fn(|k| at(1, 1, k));
                }
                let (omfx, omfy) = (V::sub(one, fx), V::sub(one, fy));
                let top = V::add(V::mul(a, omfx), V::mul(b, fx));
                let bottom = V::add(V::mul(c, omfx), V::mul(d, fx));
                let value = V::div(V::add(V::mul(top, omfy), V::mul(bottom, fy)), m255);
                V::store_f64(&mut cov[i..], V::select(outside, zero, value));
                i += V::N;
                pxv = V::add(pxv, step);
            }
        }
    }
    while i < hi {
        let px = xs.0 + i as i64;
        cov[i] = cover_pixel(s, (px as f64 + 0.5) - s.x, dy);
        i += 1;
    }
    (lo, hi)
}

/// デュアルブラシの合わせ（`DualBrushMode::combine` のレーンの版。主が 0 以下なら 0）。
#[inline(always)]
unsafe fn combine<V: Lanes>(mode: DualBrushMode, main: V::F, dual: V::F) -> V::F {
    let (zero, one, half, two) = (V::splat(0.0), V::splat(1.0), V::splat(0.5), V::splat(2.0));
    let r = match mode {
        DualBrushMode::Multiply => V::mul(main, dual),
        DualBrushMode::Darken => V::min(main, dual),
        DualBrushMode::Overlay => {
            let lo = V::mul(V::mul(two, main), dual);
            let hi = V::sub(
                one,
                V::mul(V::mul(two, V::sub(one, main)), V::sub(one, dual)),
            );
            V::select(V::lt(main, half), lo, hi)
        }
        DualBrushMode::ColorDodge => V::select(
            V::ge(dual, one),
            one,
            V::min(one, V::div(main, V::sub(one, dual))),
        ),
        DualBrushMode::ColorBurn => V::select(
            V::ge(main, one),
            one,
            V::select(
                V::le(dual, zero),
                zero,
                V::sub(one, V::min(one, V::div(V::sub(one, main), dual))),
            ),
        ),
        DualBrushMode::LinearBurn => V::sub(V::add(main, dual), one),
        DualBrushMode::HardMix => V::select(V::ge(V::add(main, dual), one), one, zero),
        DualBrushMode::Subtract => V::sub(main, dual),
    };
    V::select(V::le(main, zero), zero, simd::clamp01::<V>(r))
}

/// 覆いの行 [lo, hi) へデュアルの溜まり（このタイルの `cells`。画素 `base + i`）を合わせる。
#[inline(always)]
unsafe fn dual_row<V: F32Lanes>(
    mode: DualBrushMode,
    cells: Option<&[f32]>,
    cov: &mut [f64],
    lo: usize,
    hi: usize,
    base: usize,
) {
    let mut i = lo;
    while i + V::N <= hi {
        let main = V::load_f64(&cov[i..]);
        let dual = match cells {
            Some(c) => V::load_f32(&c[base + i..]),
            None => V::splat(0.0),
        };
        V::store_f64(&mut cov[i..], combine::<V>(mode, main, dual));
        i += V::N;
    }
    while i < hi {
        if cov[i] > 0.0 {
            let cell = cells.map_or(0.0, |c| c[base + i] as f64);
            cov[i] = mode.combine(cov[i], cell);
        }
        i += 1;
    }
}

/// 乗算の紙の質感: 覆いの行 [lo, hi) の画素ごとの不透明度の係数 `scale` を作り、係数が 0 以下の画素の覆いを 0 にする。
#[inline(always)]
#[allow(clippy::too_many_arguments)]
unsafe fn texture_row<V: Lanes>(
    s: &DabShape<'_>,
    tex: &PaperTexture,
    py: i64,
    x_first: i64,
    cov: &mut [f64],
    scale: &mut [f64],
    lo: usize,
    hi: usize,
) {
    let grain_row = tex.image.tiled_row((py as f64 + 0.5) / tex.scale);
    let (w, alpha) = (tex.image.width() as i32, tex.image.alpha());
    let (zero, one, half) = (V::splat(0.0), V::splat(1.0), V::splat(0.5));
    let (texture_scale, depth, base) = (
        V::splat(tex.scale),
        V::splat(tex.depth),
        V::splat(s.opacity_scale),
    );
    let (fy, m255) = (V::splat(grain_row.fy), V::splat(255.0));
    let omfy = V::sub(one, fy);
    let mut pxv = V::add(
        V::splat((x_first + lo as i64) as f64),
        V::from_fn(|k| k as f64),
    );
    let mut i = lo;
    while i + V::N <= hi {
        let x = V::sub(V::div(V::add(pxv, half), texture_scale), half);
        let x0 = V::floor(x);
        let fx = V::sub(x, x0);
        let mut xi = [0.0f64; 4];
        V::store_f64(&mut xi, x0);
        // 並べた画像の列（x0 を w で割った余りと、その次の列）。レーンどうしの x0 の差は小さいので、最初のレーンの余りに差を足して、
        // [0, w) を出たときだけ剰余を取る
        let (mut wx0, mut wx1) = ([0usize; 4], [0usize; 4]);
        let first_x = xi[0] as i32;
        let first_wrapped = first_x.rem_euclid(w);
        for k in 0..V::N {
            let mut v = first_wrapped + (xi[k] as i32 - first_x);
            if v < 0 || v >= w {
                v = (xi[k] as i32).rem_euclid(w);
            }
            wx0[k] = v as usize;
            wx1[k] = if v + 1 == w { 0 } else { v as usize + 1 };
        }
        let a = V::from_fn(|k| alpha[grain_row.row0 + wx0[k]] as f64);
        let b = V::from_fn(|k| alpha[grain_row.row0 + wx1[k]] as f64);
        let c = V::from_fn(|k| alpha[grain_row.row1 + wx0[k]] as f64);
        let d = V::from_fn(|k| alpha[grain_row.row1 + wx1[k]] as f64);
        let omfx = V::sub(one, fx);
        let top = V::add(V::mul(a, omfx), V::mul(b, fx));
        let bottom = V::add(V::mul(c, omfx), V::mul(d, fx));
        let grain = V::div(V::add(V::mul(top, omfy), V::mul(bottom, fy)), m255);
        // ceiling_scale = opacity_scale * (1 - depth * (1 - grain))
        let ceiling_scale = V::mul(base, V::sub(one, V::mul(depth, V::sub(one, grain))));
        V::store_f64(&mut scale[i..], ceiling_scale);
        let main = V::load_f64(&cov[i..]);
        V::store_f64(
            &mut cov[i..],
            V::select(V::le(ceiling_scale, zero), zero, main),
        );
        i += V::N;
        pxv = V::add(pxv, V::splat(V::N as f64));
    }
    while i < hi {
        if cov[i] > 0.0 {
            let px = x_first + i as i64;
            let grain = tex
                .image
                .sample_tiled_in(&grain_row, (px as f64 + 0.5) / tex.scale);
            let mut ceiling_scale = s.opacity_scale;
            ceiling_scale *= 1.0 - tex.depth * (1.0 - grain);
            if ceiling_scale <= 0.0 {
                cov[i] = 0.0;
            }
            scale[i] = ceiling_scale;
        }
        i += 1;
    }
}

/// タイルの画素 index から N 画素（面のタイルの今の値）。
#[inline(always)]
unsafe fn load_live<V: Lanes>(live: &LiveTile, pixel: usize) -> [V::F; 4] {
    unsafe {
        match live {
            LiveTile::Absent => [V::splat(0.0); 4],
            LiveTile::Uniform(c) => V::splat_px(c.to_array()),
            LiveTile::Shared(d) => V::load(&d[pixel * 4..]),
            LiveTile::Owned(v) => V::load(&v[pixel * 4..]),
        }
    }
}

/// 透明部分のロック（`document::locks::paint_keeping_alpha` のレーンの版）: アルファは描く前のまま、色だけを描く色のアルファ × 量の
/// 割合で寄せる。描く前が透明か、割合が 0 以下の画素はそのまま。
#[inline(always)]
unsafe fn keeping_block<V: Lanes>(start: [V::F; 4], paint: [u8; 4], amount: V::F) -> [V::F; 4] {
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let a = V::min(
        V::div(V::mul(amount, V::splat(paint[3] as f64)), V::splat(255.0)),
        one,
    );
    let skip = V::or(V::eq(start[3], zero), V::le(a, zero));
    let mut out = [start[0], start[1], start[2], start[3]];
    for c in 0..3 {
        let s = V::unit(start[c]);
        let p = V::unit(V::splat(paint[c] as f64));
        let mixed = to_byte::<V>(V::add(s, V::mul(V::sub(p, s), a)));
        out[c] = V::select(skip, start[c], mixed);
    }
    out
}

/// 面のタイルの画素 local から N 画素へ out を書く（今の値 current と違う画素があるときだけ）。違う画素があれば true。
#[inline(always)]
unsafe fn commit<V: Lanes>(
    cx: &mut PixelContext<'_>,
    live: &mut LiveTile,
    local: usize,
    out: [V::F; 4],
    current: [V::F; 4],
) -> Result<bool, CoreError> {
    let differs = V::or(
        V::or(
            V::not(V::eq(out[0], current[0])),
            V::not(V::eq(out[1], current[1])),
        ),
        V::or(
            V::not(V::eq(out[2], current[2])),
            V::not(V::eq(out[3], current[3])),
        ),
    );
    if !V::any(differs) {
        return Ok(false);
    }
    let ts = cx.tile_size;
    match live {
        LiveTile::Owned(v) => V::store(&mut v[local * 4..], out),
        _ => {
            // 一様・共有・無いタイルは、最初に変わる画素の書き込みで自分のものにする（予算の確かめも今までと同じ）
            let mut bytes = [0u8; 16];
            V::store(&mut bytes, out);
            for k in 0..V::N {
                let c = Rgba8::from_slice(&bytes[k * 4..k * 4 + 4]);
                live.write(
                    (local + k) * 4,
                    c,
                    cx.allocated,
                    ts * ts * 4,
                    cx.budgets.growth,
                )?;
            }
        }
    }
    Ok(true)
}

/// 覆いの行 cov の [lo, hi) を、ストロークの覆いと面へ当てる（`apply_at` の色を塗る/消すだけの道と同じ結果）。
#[inline(always)]
#[allow(clippy::too_many_arguments)]
unsafe fn apply_range<V: F32Lanes>(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    coord: TileCoord,
    row: usize,
    x0: usize,
    cov: &[f64],
    scale: &Scale<'_>,
    lo: usize,
    hi: usize,
    shape: &DabShape<'_>,
) -> Result<bool, CoreError> {
    let p = cx.paint;
    let s = p.s;
    let pressure = shape.pressure;
    let mut changed = false;
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let (opacity, flow_k) = (V::splat(s.opacity), V::splat(s.flow));
    let (flow_scale, pressure_opacity, pressure_flow) = (
        V::splat(shape.flow_scale),
        V::splat(pressure.opacity),
        V::splat(pressure.flow),
    );
    let color = p.stroke_color.to_array();
    let erase_alpha = V::splat(s.color.a as f64);
    // 選択範囲の量（このタイルの画素ごと。無ければ 1）と、透明部分のロック
    let amounts = match cx.selected {
        Selected::Tile(a) => Some(a),
        _ => None,
    };
    let keep_alpha = p.keep_alpha;
    let mut i = lo;
    while i + V::N <= hi {
        let local = row + x0 + i;
        let coverage = unsafe { V::load_f64(&cov[i..]) };
        let opacity_scale = match scale {
            Scale::Const(c) => V::splat(*c),
            Scale::Row(r) => unsafe { V::load_f64(&r[i..]) },
        };
        let ceiling = V::mul(V::mul(opacity, opacity_scale), pressure_opacity);
        let flow = V::mul(V::mul(V::mul(coverage, flow_k), flow_scale), pressure_flow);
        // flow <= 0 || ceiling <= 0 は塗らない（NaN は塗る側。スカラーの `if flow <= 0.0 || ceiling <= 0.0` と同じ）
        let mut active = V::not(V::or(V::le(flow, zero), V::le(ceiling, zero)));
        // 選択されていない画素・透明部分のロックの透明な画素は何もしない（写しも取らない）
        let selected = match amounts {
            None => one,
            Some(Amounts::Uniform(v)) => V::splat(*v as f64 / 255.0),
            Some(Amounts::Data(d)) => V::unit(V::from_fn(|k| d[local + k] as f64)),
        };
        if amounts.is_some() {
            active = V::and(active, V::not(V::le(selected, zero)));
        }
        if keep_alpha {
            let now = unsafe { load_live::<V>(live, local) };
            active = V::and(active, V::not(V::eq(now[3], zero)));
        }
        if !V::any(active) {
            i += V::N;
            continue;
        }
        let previous = match held.as_ref() {
            Some(st) => {
                let w = unsafe { V::load_f32(&st.wash[local..]) };
                // ストロークの覆いが天井に届いた画素は何もしない
                active = V::and(active, V::not(V::ge(w, ceiling)));
                w
            }
            None => zero,
        };
        if !V::any(active) {
            i += V::N;
            continue;
        }
        if held.is_none() {
            *held = Some(new_stroke_tile(cx, live, true)?);
        }
        let st = held.as_mut().expect("直前に作った");
        let accumulated = V::add(
            previous,
            V::mul(V::sub(ceiling, previous), V::min(one, flow)),
        );
        unsafe {
            V::store_f32(
                &mut st.wash[local..],
                V::select(active, accumulated, previous),
            );
        }
        let start = match &st.before {
            None => [zero; 4],
            Some(Tile::Uniform(c)) => unsafe { V::splat_px(c.to_array()) },
            Some(Tile::Data(d)) => unsafe { V::load(&d[local * 4..]) },
        };
        let next = if s.erase {
            // alpha = to_byte(start.a / 255 * (1 - accumulated * color.a / 255))、0 なら透明（RGB も 0）
            let a = V::mul(
                V::unit(start[3]),
                V::sub(
                    one,
                    V::div(V::mul(accumulated, erase_alpha), V::splat(255.0)),
                ),
            );
            let a = unsafe { to_byte::<V>(a) };
            let gone = V::eq(a, zero);
            [
                V::select(gone, zero, start[0]),
                V::select(gone, zero, start[1]),
                V::select(gone, zero, start[2]),
                a,
            ]
        } else if keep_alpha {
            // アルファは描く前のまま。色だけを（描く色のアルファ × 量）の割合で寄せる
            unsafe { keeping_block::<V>(start, color, V::mul(V::min(one, accumulated), selected)) }
        } else {
            let src = unsafe { V::splat_px(color) };
            match unsafe { blend_block::<V, NORMAL>(start, src, V::min(one, accumulated)) } {
                Some(out) => out,
                None => start,
            }
        };
        // 半分だけ選ばれた画素は、描く前の画素から選ばれた量だけ寄せる
        let next = if amounts.is_some() && !keep_alpha && V::any(V::lt(selected, one)) {
            let faded = unsafe { fade_block::<V>(start, next, selected) };
            [
                V::select(V::lt(selected, one), faded[0], next[0]),
                V::select(V::lt(selected, one), faded[1], next[1]),
                V::select(V::lt(selected, one), faded[2], next[2]),
                V::select(V::lt(selected, one), faded[3], next[3]),
            ]
        } else {
            next
        };
        let current = unsafe { load_live::<V>(live, local) };
        let out = [
            V::select(active, next[0], current[0]),
            V::select(active, next[1], current[1]),
            V::select(active, next[2], current[2]),
            V::select(active, next[3], current[3]),
        ];
        changed |= unsafe { commit::<V>(cx, live, local, out, current)? };
        i += V::N;
    }
    // 端の画素（レーンの数で割った余り）は画素ごとの式で
    while i < hi {
        let local = row + x0 + i;
        let opacity_scale = match scale {
            Scale::Const(c) => *c,
            Scale::Row(r) => r[i],
        };
        changed |= apply_at::<true>(
            cx,
            held,
            live,
            coord,
            local,
            cov[i],
            pressure,
            opacity_scale,
            shape.flow_scale,
            None,
            None,
        )?;
        i += 1;
    }
    Ok(changed)
}

/// デュアルブラシの 2 つ目の筆先のダブの、1 枚のタイル分の溜まり（`dual_dab_at` のタイルのループ）をレーンで行う。道がスカラーなら None
/// （呼び手が画素ごとに）。溜まりの置き場（`cells`）は、覆いのある画素を最初に見たときに `alloc`（予算の確かめつき）で作る。
/// 画素ごとに自分の覆いと今の値の大きい方を取るだけなので、行や画素の順は結果を変えない。
pub(super) fn dual_tile(
    shape: &DualShape<'_>,
    xs: (i64, i64),
    ys: (i64, i64),
    origin: (i64, i64, usize),
    cells: &mut Option<Vec<f32>>,
    alloc: &mut dyn FnMut() -> Result<Vec<f32>, CoreError>,
) -> Option<Result<(), CoreError>> {
    if origin.2 > MAX_ROW {
        return None;
    }
    match simd::level() {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level() は detect() 以下なので、AVX2 と FMA を持つ
        Level::Avx2 => Some(unsafe { dual_avx2(shape, xs, ys, origin, cells, alloc) }),
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level() は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => Some(unsafe { dual_sse41(shape, xs, ys, origin, cells, alloc) }),
        _ => None,
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn dual_avx2(
    shape: &DualShape<'_>,
    xs: (i64, i64),
    ys: (i64, i64),
    origin: (i64, i64, usize),
    cells: &mut Option<Vec<f32>>,
    alloc: &mut dyn FnMut() -> Result<Vec<f32>, CoreError>,
) -> Result<(), CoreError> {
    unsafe { dual_rows::<Avx2>(shape, xs, ys, origin, cells, alloc) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn dual_sse41(
    shape: &DualShape<'_>,
    xs: (i64, i64),
    ys: (i64, i64),
    origin: (i64, i64, usize),
    cells: &mut Option<Vec<f32>>,
    alloc: &mut dyn FnMut() -> Result<Vec<f32>, CoreError>,
) -> Result<(), CoreError> {
    unsafe { dual_rows::<Sse41>(shape, xs, ys, origin, cells, alloc) }
}

#[inline(always)]
unsafe fn dual_rows<V: F32Lanes>(
    shape: &DualShape<'_>,
    xs: (i64, i64),
    ys: (i64, i64),
    (ox, oy, ts): (i64, i64, usize),
    cells: &mut Option<Vec<f32>>,
    alloc: &mut dyn FnMut() -> Result<Vec<f32>, CoreError>,
) -> Result<(), CoreError> {
    // 覆いの行は、主のダブの覆いの行と同じ式（丸も回転の式で測る = `plain` でない）。反転・紙の質感・デュアルは無い
    let cover = DabShape {
        x: shape.x,
        y: shape.y,
        radius: shape.radius,
        cos: shape.cos,
        sin: shape.sin,
        roundness: shape.roundness,
        aspect_x: shape.aspect_x,
        aspect_y: shape.aspect_y,
        hardness: shape.hardness,
        pressure: PressureScale {
            opacity: 1.0,
            flow: 1.0,
            mix_paint: 1.0,
            mix_density: 1.0,
        },
        opacity_scale: 1.0,
        flow_scale: 1.0,
        tip: shape.tip,
        plain: false,
        flip_x: false,
        flip_y: false,
        texture: None,
        dual: None,
    };
    let n = (xs.1 - xs.0 + 1) as usize;
    let mut stack = [0.0f64; STACK_ROW];
    let mut heap = Vec::new();
    let cov: &mut [f64] = if n <= STACK_ROW {
        &mut stack[..n]
    } else {
        heap.resize(n, 0.0);
        &mut heap
    };
    let x0 = (xs.0 - ox) as usize;
    for py in ys.0..=ys.1 {
        let (lo, hi) = cover_row::<V>(&cover, py, xs, cov);
        if lo >= hi {
            continue;
        }
        if cells.is_none() {
            if !cov[lo..hi].iter().any(|&c| c > 0.0) {
                continue;
            }
            *cells = Some(alloc()?);
        }
        let c = cells.as_mut().expect("直前に作った");
        let row = ((py - oy) * ts as i64) as usize + x0;
        let mut i = lo;
        while i + V::N <= hi {
            let coverage = V::load_f64(&cov[i..]);
            let current = V::load_f32(&c[row + i..]);
            V::store_f32(
                &mut c[row + i..],
                V::select(V::gt(coverage, current), coverage, current),
            );
            i += V::N;
        }
        while i < hi {
            if cov[i] > c[row + i] as f64 {
                c[row + i] = cov[i] as f32;
            }
            i += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
