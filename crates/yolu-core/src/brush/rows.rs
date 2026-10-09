//! ダブの画素を行ごとに f32 のレーン（AVX2 は 8 画素・SSE4.1 と NEON は 4 画素・スカラーは 1 画素）で処理する核。ステンシルを使わないブラシ
//! （色を塗る・消す、ダブごとの色・色の混ぜ、読み元の枠から読む効果）は、どの道（スカラーも）でもこの核で描く。選択範囲・透明部分の
//! ロックは、色を塗る・消すだけのブラシなら行の核が受け持ち、画素ごとの色・効果のブラシでは画素ごとの式（`apply_at`）のまま。
//! ステンシル・乗算でない紙の質感も画素ごとの式（[`usable`]）。
//!
//! 計算は f32 で、合成（`crate::blend`）と同じ形にしてある: 道ごとに同じレーンの式を通り、SIMD の道の端の画素（N で割った余り）と、
//! 読む位置がキャンバスの端にかかる効果・混ぜのブロックは、1 本のレーン（[`Scalar1`]）で同じ関数を呼ぶ。演算は IEEE の四則・平方根・floor・
//! 比較・選択だけ（積和の命令・近似の逆数は使わない）で、判断はレーンごと（ブロックの全部・どれかを見るのは計算を省くためだけで、
//! 結果を変えない）なので、各画素の結果は道とスレッド数によらず同じバイトになる。画素ごとの式（`apply_at`）も、ここの式を 1 本の
//! レーンで通る（ステンシルのない同じブラシなら同じバイト）。
//!
//! 画素の仕事を 2 段に分ける。
//! 1. 覆いの行（[`cover_row`]）: 丸（回転・潰しがあっても）と筆先の画像（4 つの texel を画素ごとに集めて双線形）をレーンで測り、
//!    デュアルの合わせ（[`dual_row`]）と紙の質感（乗算、[`texture_row`]）も覆いの行へレーンで掛ける。
//! 2. 当ての行: ストロークの覆い（`wash`、f32）への寄せ・描く前のタイルとの合成（`crate::blend` の Normal とフェードの N 画素の式）・
//!    面への書き込み。色を塗る・消すは [`apply_range`]、指先・クローン・ぼかしは [`effect`]、ダブごとの色・色の混ぜは [`color`]。
//!
//! 画素の中心と筆の中心の x のずれは、行の先頭のずれ（f64 で求めて f32 へ丸める）に画素の番号を足して作る（道によらず同じ値）。
//! 丸の覆いは、距離の 2 乗で確実に硬さの内側・確実に外の画素を平方根と割り算なしで 1・0 に決める（余裕の相対 1e-4 は f32 の丸めの
//! 誤差よりずっと大きいので、正確な式でも同じ 1・0 になる。決めるのはレーンごと）。行は、円か筆先の矩形にかかる区間へ狭める
//! （外側へ余裕を取る。範囲の外は必ず覆い 0 になるので、飛ばしても画素は変わらない）。面のタイルが一様・共有・無いときは、
//! 変わる画素があるブロックの最初の書き込みを `LiveTile::write` で行い（予算の確かめと自分のものにする複製は今までと同じ）、
//! あとはタイルの中へ直接書く。

#![cfg_attr(
    not(any(target_arch = "x86_64", target_arch = "aarch64")),
    allow(dead_code, unused_imports, unused_macros, unused_variables, unused_mut)
)]

use super::*;
use crate::blend::lanes::NORMAL;
use crate::blend::{blend_block, fade_block};
use crate::math::simd::{self, clamp01_32, to_byte32, Lanes32, Level, Scalar1};

#[cfg(target_arch = "aarch64")]
use crate::math::simd::Neonx4;
#[cfg(target_arch = "x86_64")]
use crate::math::simd::{Avx2x8, Sse41x4};
#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

mod color;
mod effect;
use color::apply_color_range;
pub(super) use color::{mixed32, tally32};
use effect::{apply_effect_range, BLUR, CLONE, SMUDGE};
pub(super) use effect::{blur32, mix_effect32, sample32};

/// 行の長さの上限（これを超えるタイルの大きさは行の核を使わない。タイルの大きさは 1〜1024）。
const MAX_ROW: usize = 1024;
/// 行の覆い・紙の質感の値を、スタックに置く行の長さ（これより長い行はヒープ）。
const STACK_ROW: usize = 256;
/// 丸の覆いの近道の余裕（距離の 2 乗に対する相対）。f32 の丸めの誤差（1e-6 より小さい）よりずっと大きい。
const ROUND_MARGIN: f64 = 1e-4;

/// f32 の並び（ストロークの覆い `wash`・覆いの行・デュアルの溜まり）の読み書きと、整数の値のレーンを添字にする変換。
pub(super) trait Slice32: Lanes32 {
    /// 連続する N 個の f32 をレーンへ。
    unsafe fn load_f32(p: &[f32]) -> Self::F;
    /// レーンを連続する N 個の f32 へ。
    unsafe fn store_f32(p: &mut [f32], v: Self::F);
    /// 整数の値（floor の結果）のレーンを i32 へ（先頭の N 個。切り捨て。i32 に収まらない値と NaN は `i32::MIN`）。
    unsafe fn to_i32(v: Self::F, out: &mut [i32; 8]);
}

/// `cvttss2si` と同じ変換（範囲の外と NaN は `i32::MIN`）。
#[inline(always)]
fn truncate_i32(v: f32) -> i32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&v) {
        v as i32
    } else {
        i32::MIN
    }
}

impl Slice32 for Scalar1 {
    #[inline(always)]
    unsafe fn load_f32(p: &[f32]) -> f32 {
        p[0]
    }
    #[inline(always)]
    unsafe fn store_f32(p: &mut [f32], v: f32) {
        p[0] = v;
    }
    #[inline(always)]
    unsafe fn to_i32(v: f32, out: &mut [i32; 8]) {
        out[0] = truncate_i32(v);
    }
}

#[cfg(target_arch = "x86_64")]
impl Slice32 for Avx2x8 {
    #[inline(always)]
    unsafe fn load_f32(p: &[f32]) -> __m256 {
        let q: &[f32; 8] = p[..8].try_into().unwrap();
        unsafe { _mm256_loadu_ps(q.as_ptr()) }
    }
    #[inline(always)]
    unsafe fn store_f32(p: &mut [f32], v: __m256) {
        let q: &mut [f32; 8] = (&mut p[..8]).try_into().unwrap();
        unsafe { _mm256_storeu_ps(q.as_mut_ptr(), v) }
    }
    #[inline(always)]
    unsafe fn to_i32(v: __m256, out: &mut [i32; 8]) {
        unsafe { _mm256_storeu_si256(out.as_mut_ptr().cast(), _mm256_cvttps_epi32(v)) }
    }
}

#[cfg(target_arch = "x86_64")]
impl Slice32 for Sse41x4 {
    #[inline(always)]
    unsafe fn load_f32(p: &[f32]) -> __m128 {
        let q: &[f32; 4] = p[..4].try_into().unwrap();
        unsafe { _mm_loadu_ps(q.as_ptr()) }
    }
    #[inline(always)]
    unsafe fn store_f32(p: &mut [f32], v: __m128) {
        let q: &mut [f32; 4] = (&mut p[..4]).try_into().unwrap();
        unsafe { _mm_storeu_ps(q.as_mut_ptr(), v) }
    }
    #[inline(always)]
    unsafe fn to_i32(v: __m128, out: &mut [i32; 8]) {
        unsafe { _mm_storeu_si128(out.as_mut_ptr().cast(), _mm_cvttps_epi32(v)) }
    }
}

#[cfg(target_arch = "aarch64")]
impl Slice32 for Neonx4 {
    #[inline(always)]
    unsafe fn load_f32(p: &[f32]) -> float32x4_t {
        let q: &[f32; 4] = p[..4].try_into().unwrap();
        unsafe { vld1q_f32(q.as_ptr()) }
    }
    #[inline(always)]
    unsafe fn store_f32(p: &mut [f32], v: float32x4_t) {
        let q: &mut [f32; 4] = (&mut p[..4]).try_into().unwrap();
        unsafe { vst1q_f32(q.as_mut_ptr(), v) }
    }
    /// 範囲の外と NaN を `i32::MIN` にする: `vcvtq_s32_f32` は範囲の外を飽和・NaN を 0 にするので、x86_64 の `cvttps2dq`・
    /// [`truncate_i32`] と同じ値になるように、範囲の確かめ（NaN は偽）で選び直す。範囲の中では切り捨てで同じ値。
    #[inline(always)]
    unsafe fn to_i32(v: float32x4_t, out: &mut [i32; 8]) {
        unsafe {
            let inside = vandq_u32(
                vcgeq_f32(v, vdupq_n_f32(-2_147_483_648.0)),
                vcltq_f32(v, vdupq_n_f32(2_147_483_648.0)),
            );
            let truncated = vbslq_s32(inside, vcvtq_s32_f32(v), vdupq_n_s32(i32::MIN));
            vst1q_s32(out.as_mut_ptr(), truncated);
        }
    }
}

// ───────── 画素の式（N 画素の式と、画素ごとの式 `apply_at` が通る 1 本のレーンの版） ─────────

#[inline(always)]
pub(super) fn lanes_of(c: Rgba8) -> [f32; 4] {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { Scalar1::splat_px(c.to_array()) }
}

#[inline(always)]
pub(super) fn rgba_of(v: [f32; 4]) -> Rgba8 {
    let mut p = [0u8; 4];
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { Scalar1::store(&mut p, v) };
    Rgba8::new(p[0], p[1], p[2], p[3])
}

/// `if a < b { a } else { b }`（レーンの `min` と同じ）。
#[inline(always)]
pub(super) fn min32(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

/// ストロークの覆いを天井へ流量の割合だけ寄せる（前の値 + (天井 − 前の値) × min(1, 流量)）。
#[inline(always)]
unsafe fn accumulate<V: Lanes32>(previous: V::F, ceiling: V::F, flow: V::F) -> V::F {
    unsafe {
        V::add(
            previous,
            V::mul(V::sub(ceiling, previous), V::min(V::splat(1.0), flow)),
        )
    }
}

/// 1 画素の [`accumulate`]。
#[inline(always)]
pub(super) fn accumulate32(previous: f32, ceiling: f32, flow: f32) -> f32 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { accumulate::<Scalar1>(previous, ceiling, flow) }
}

/// 下 start に描く色 color を量 amount で Normal で重ねる（`crate::blend` の Normal の N 画素の式。何も変わらなければ start）。
#[inline(always)]
unsafe fn normal_block<V: Lanes32>(start: [V::F; 4], color: [V::F; 4], amount: V::F) -> [V::F; 4] {
    unsafe { blend_block::<V, NORMAL>(start, color, amount).unwrap_or(start) }
}

/// 1 画素の [`normal_block`]。
#[inline(always)]
pub(super) fn normal32(start: Rgba8, color: Rgba8, amount: f32) -> Rgba8 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    rgba_of(unsafe { normal_block::<Scalar1>(lanes_of(start), lanes_of(color), amount) })
}

/// 1 画素のフェード（`crate::blend::fade` の N 画素の式を量 f32 で）。
#[inline(always)]
pub(super) fn fade32(backdrop: Rgba8, inner: Rgba8, amount: f32) -> Rgba8 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    rgba_of(unsafe { fade_block::<Scalar1>(lanes_of(backdrop), lanes_of(inner), amount) })
}

/// 消しゴム: アルファを to_byte(start.a / 255 × (1 − 覆い × 描く色のアルファ / 255)) に。0 になったら透明（RGB も 0）。
#[inline(always)]
unsafe fn erase_block<V: Lanes32>(start: [V::F; 4], accumulated: V::F, alpha: V::F) -> [V::F; 4] {
    unsafe {
        let (zero, one) = (V::splat(0.0), V::splat(1.0));
        let a = V::mul(
            V::unit(start[3]),
            V::sub(one, V::div(V::mul(accumulated, alpha), V::splat(255.0))),
        );
        let a = to_byte32::<V>(a);
        let gone = V::eq(a, zero);
        [
            V::select(gone, zero, start[0]),
            V::select(gone, zero, start[1]),
            V::select(gone, zero, start[2]),
            a,
        ]
    }
}

/// 1 画素の [`erase_block`]。
#[inline(always)]
pub(super) fn erase32(start: Rgba8, accumulated: f32, alpha: u8) -> Rgba8 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    rgba_of(unsafe { erase_block::<Scalar1>(lanes_of(start), accumulated, f32::from(alpha)) })
}

/// 透明部分のロック（`document::locks::paint_keeping_alpha` と同じ形の f32 の式）: アルファは描く前のまま、色だけを描く色の
/// アルファ × 量の割合で寄せる。描く前が透明か、割合が 0 以下の画素はそのまま。
#[inline(always)]
unsafe fn keeping_block<V: Lanes32>(start: [V::F; 4], paint: [V::F; 4], amount: V::F) -> [V::F; 4] {
    unsafe {
        let (zero, one) = (V::splat(0.0), V::splat(1.0));
        let a = V::min(V::div(V::mul(amount, paint[3]), V::splat(255.0)), one);
        let skip = V::or(V::eq(start[3], zero), V::le(a, zero));
        let mut out = start;
        for c in 0..3 {
            let s = V::unit(start[c]);
            let p = V::unit(paint[c]);
            let mixed = to_byte32::<V>(V::add(s, V::mul(V::sub(p, s), a)));
            out[c] = V::select(skip, start[c], mixed);
        }
        out
    }
}

/// 1 画素の [`keeping_block`]。
#[inline(always)]
pub(super) fn keeping32(start: Rgba8, paint: Rgba8, amount: f32) -> Rgba8 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    rgba_of(unsafe { keeping_block::<Scalar1>(lanes_of(start), lanes_of(paint), amount) })
}

/// 画素ごとの色の面の 1 成分の積み: 最初の打点（前の覆いが 0 以下）は色そのもの、あとは流量の割合 w だけ寄せる。
#[inline(always)]
unsafe fn plane_step<V: Lanes32>(old: V::F, unit: V::F, w: V::F, fresh: V::M) -> V::F {
    unsafe { V::select(fresh, unit, V::add(old, V::mul(V::sub(unit, old), w))) }
}

/// 1 画素の [`plane_step`]。
#[inline(always)]
pub(super) fn plane_step32(old: f32, unit: f32, w: f32, fresh: bool) -> f32 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { plane_step::<Scalar1>(old, unit, w, fresh) }
}

/// 0〜1 の値を 0〜255 の整数へ（`to_byte32` の 1 画素）。
#[inline(always)]
pub(super) fn byte32(v: f32) -> u8 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { to_byte32::<Scalar1>(v) as u8 }
}

/// b / 255（レーンの `unit` と同じ割り算）。
#[inline(always)]
pub(super) fn unit32(b: u8) -> f32 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { Scalar1::unit(f32::from(b)) }
}

// ───────── ブロックの並べ方 ─────────

/// 行の [lo, hi) を N 画素ずつ並べる。N 画素に満たない余り [i, hi) は、行の置き場（長さ n）に収まるなら、余りを含む N 画素のウィンドウ
/// （始まり t = min(i, n − N)）を 1 つ、ウィンドウの中の [i, hi) だけを生かして（[`window`]）通す。ウィンドウの中の生かさない画素は、前の
/// ブロックがもう描いたか区間の外の画素で、覆いを 0 にして何も変えない（読み直した値をそのまま書き戻すだけ）。置き場が N 画素より
/// 短い行は、余りを 1 本のレーンで描く。
#[derive(Clone, Copy)]
struct Block {
    /// ブロックの最初の画素（行の置き場の中の位置）。
    at: usize,
    /// 生かす画素の範囲（ウィンドウのとき）。
    live: Option<(usize, usize)>,
}

/// 画素 i から先の次のブロック（無ければ None。N 画素のウィンドウにできない余りは呼び手が 1 本のレーンで描く）。
#[inline(always)]
fn next_block(i: usize, hi: usize, n: usize, lanes: usize) -> Option<Block> {
    if i + lanes <= hi {
        Some(Block { at: i, live: None })
    } else if i < hi && lanes > 1 && n >= lanes {
        Some(Block {
            at: i.min(n - lanes),
            live: Some((i, hi)),
        })
    } else {
        None
    }
}

/// ウィンドウの中の生かす画素（[from, to)）のレーン。
#[inline(always)]
unsafe fn window<V: Lanes32>(at: usize, (from, to): (usize, usize)) -> V::M {
    unsafe {
        let index = V::from_fn(|k| (at + k) as f32);
        V::and(
            V::ge(index, V::splat(from as f32)),
            V::lt(index, V::splat(to as f32)),
        )
    }
}

/// ブロックの覆い（ウィンドウなら生かさない画素を 0 に）。
#[inline(always)]
unsafe fn block_coverage<V: Slice32>(cov: &[f32], b: Block) -> V::F {
    unsafe {
        let c = V::load_f32(&cov[b.at..]);
        match b.live {
            None => c,
            Some(live) => V::select(window::<V>(b.at, live), c, V::splat(0.0)),
        }
    }
}

// ───────── 道の選び ─────────

/// このタイルを行の核で描けるなら、使う道を返す。紙の質感が乗算か無く、ステンシルを使わないこと。そして、色を塗る・消す
/// （選択範囲・透明部分のロックがあってもよい）か、選択範囲・ロックを使わない画素ごとの色（ダブごとの色・色の混ぜ）か読み元の枠から
/// 読む効果（ぼかし・指先・クローン）で、面のダブの写像でないこと。
///
/// 道は 1 度だけ読み、呼び手は返った道をそのまま [`dab_tile`] に渡す（判断と実行で別々に読むと、その間に道が変わりうる。試験は
/// 道を切り替えるので、並んで走る別の試験のストロークで起こりうる）。
pub(super) fn usable(cx: &PixelContext<'_>, s: &DabShape<'_>) -> Option<Level> {
    #[cfg(test)]
    if per_pixel_forced() {
        return None;
    }
    eligible(
        cx.paint,
        s,
        matches!(cx.selected, Selected::Everywhere),
        cx.tile_size,
    )
    .then(simd::level)
}

/// 試験だけ: 行の核を使わず、画素ごとの式（`apply_at`）で描く（行の核と画素ごとの式が同じバイトになることを比べる）。
#[cfg(test)]
static PER_PIXEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
fn per_pixel_forced() -> bool {
    PER_PIXEL.load(std::sync::atomic::Ordering::Relaxed)
}

/// [`usable`] と [`cost_per_pixel`] が共有する、行の核を使えるかの判定。
fn eligible(p: &Paint<'_>, s: &DabShape<'_>, no_selection: bool, tile_size: usize) -> bool {
    let kind_ok = p.stencil.is_none()
        && p.mapped.is_none()
        && match p.effect {
            // 混ぜるブラシは下地の枠が要る
            EffectKind::Paint => p.mix.is_none() || p.frame.is_some(),
            _ => !p.tip_colors && p.mix.is_none() && p.frame.is_some(),
        };
    kind_ok
        && tile_size <= MAX_ROW
        && (is_plain(p) || (!p.keep_alpha && no_selection))
        && s.texture.is_none_or(|t| t.mode.as_blend().is_none())
}

/// 色を塗る・消すだけ（効果・画素ごとの色なし）のブラシか。選択範囲と透明部分のロックは、このブラシだけ行の核が受け持つ。
fn is_plain(p: &Paint<'_>) -> bool {
    p.effect == EffectKind::Paint && !p.tip_colors
}

/// このダブの 1 画素あたりの時間の見積もり（ナノ秒。ワーカーで描くかの判断の重み）。行の核を使えない（ステンシル・紙の質感が乗算
/// 以外・面のダブの写像・画素ごとの色や効果のブラシの選択範囲・透明部分のロック）ときと、道がスカラーのときは画素ごとの式の 30。
/// SIMD の道で行の核を使えるときは、硬い丸 1・柔らかい丸と筆先の画像 3（デュアルは +1、紙の質感は +4）・効果と画素ごとの色 14。
/// 1 スレッドの計測（f64 の核で、硬い丸 1.9・柔らかい丸 3.7・筆先 3.1・質感 7・デュアル 4・指先 12・混ぜる 17 ナノ秒 / 画素）より、
/// 直列の方がわずかに速い側へ寄せてある（ワーカーを使う損の方が大きいため）。
pub(super) fn cost_per_pixel(
    paint: &Paint<'_>,
    s: &DabShape<'_>,
    no_selection: bool,
    tile_size: usize,
) -> i64 {
    if simd::level() == Level::Scalar || !eligible(paint, s, no_selection, tile_size) {
        return SCALAR_PIXEL_NANOS;
    }
    if paint.effect != EffectKind::Paint || paint.tip_colors {
        return 14;
    }
    let mut cost = if s.tip.is_some() || s.hardness < 1.0 || !s.edge.is_off() {
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

/// 行の核で 1 枚のタイルのダブの画素を描く（`level` は [`usable`] が返した道）。
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
    match level.min(simd::detect()) {
        // 箱の平均を読むブラシは、AVX2 の CPU でも SSE4.1 の 4 本で描く（どの道も同じバイト。測ると 4 本の方が速かった）
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2（と SSE4.1）を持つ
        Level::Avx2 if box_average(cx.paint) => unsafe {
            tile_sse41(cx, held, live, dual, s, coord, xs, ys)
        },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2 を持つ
        Level::Avx2 => unsafe { tile_avx2(cx, held, live, dual, s, coord, xs, ys) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { tile_sse41(cx, held, live, dual, s, coord, xs, ys) },
        #[cfg(target_arch = "aarch64")]
        // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
        Level::Neon => unsafe { tile_neon(cx, held, live, dual, s, coord, xs, ys) },
        // SAFETY: 1 本のレーンは CPU の前提を持たない
        Level::Scalar => unsafe { tile::<Scalar1>(cx, held, live, dual, s, coord, xs, ys) },
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
    unsafe { tile::<Avx2x8>(cx, held, live, dual, s, coord, xs, ys) }
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
    unsafe { tile::<Sse41x4>(cx, held, live, dual, s, coord, xs, ys) }
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[allow(clippy::too_many_arguments)]
unsafe fn tile_neon(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dual: Option<&[f32]>,
    s: &DabShape<'_>,
    coord: TileCoord,
    xs: (i64, i64),
    ys: (i64, i64),
) -> Result<bool, CoreError> {
    unsafe { tile::<Neonx4>(cx, held, live, dual, s, coord, xs, ys) }
}

/// 箱の平均を画素ごとに読むブラシ（ぼかし・色の混ぜの伸ばす）か。AVX2 の CPU でも SSE4.1 の 4 本で描くかの判断にだけ使う。
#[cfg(target_arch = "x86_64")]
fn box_average(p: &Paint<'_>) -> bool {
    matches!(p.effect, EffectKind::Blur(_)) || p.mix.is_some_and(|m| m.mode == MixMode::Smear)
}

/// 紙の質感の行ごとの値（乗算の紙の質感があるとき、画素ごとの不透明度の係数）。
#[derive(Clone, Copy)]
pub(super) enum Scale<'a> {
    /// ダブで 1 つ（質感なし）。
    Const(f32),
    Row(&'a [f32]),
}

impl Scale<'_> {
    #[inline(always)]
    unsafe fn lanes<V: Slice32>(&self, i: usize) -> V::F {
        match self {
            Scale::Const(c) => unsafe { V::splat(*c) },
            Scale::Row(r) => unsafe { V::load_f32(&r[i..]) },
        }
    }
}

/// ダブの形の f32 の値（タイルごとに 1 回作る）。
pub(super) struct Shape32<'a> {
    radius: f32,
    hardness: f32,
    /// 1 − 硬さ（f64 で求めて丸めた値）
    inner: f32,
    cos: f32,
    sin: f32,
    nsin: f32,
    /// 半径 × 潰し
    squash: f32,
    aspect_x: f32,
    aspect_y: f32,
    /// 近道の閾値（回転・潰しの無い丸は距離の 2 乗、そのほかは規格化した距離の 2 乗に対して）
    inside: f32,
    outside: f32,
    inv_radius2: f32,
    inv_squash2: f32,
    plain: bool,
    flip_x: bool,
    flip_y: bool,
    tip: Option<&'a BrushTip>,
    /// 丸の縁のアンチエイリアス（None は今の式）。
    aa: Option<Aa32>,
    /// 画像の筆先の小さなダブの濃さ（1 は掛けない）。
    density: f32,
}

/// 丸の縁のアンチエイリアスの f32 の値（[`super::edge`] の式。ダブで 1 つ）。
#[derive(Clone, Copy)]
struct Aa32 {
    /// 帯の幅（画素）。
    band: f32,
    /// 1 − 硬さ（ぼかしの幅、規格化した単位）と、今の式で覆いが半分になる距離。
    soft: f32,
    mid: f32,
    density: f32,
    /// 帯がぼかしの幅以下の画素は今の式にする（濃さ 1 のダブ）。
    exact_soft: bool,
    /// 真円の帯（規格化した単位）と、帯の幅・外の端（ダブで 1 つ）。
    plain_band: f32,
    plain_width: f32,
    plain_outer: f32,
    /// 潰した丸の、中心での勾配（規格化した単位 / 画素。短い軸の向き）。
    max_gradient: f32,
    /// 潰した丸の、覆いが半分になる楕円の外接の箱の半分の幅（画素。回した座標の u・v の向き。帯の式の距離を下から押さえる）。
    box_u: f32,
    box_v: f32,
}

impl<'a> Shape32<'a> {
    pub(super) fn new(s: &DabShape<'a>) -> Self {
        let radius = s.radius as f32;
        let hardness = s.hardness as f32;
        let squash = (s.radius * s.roundness) as f32;
        let (r, h, q) = (f64::from(radius), f64::from(hardness), f64::from(squash));
        let (mut inside, mut outside) = if s.plain {
            (
                h * r * (h * r) * (1.0 - ROUND_MARGIN),
                r * r * (1.0 + ROUND_MARGIN),
            )
        } else {
            (h * h * (1.0 - ROUND_MARGIN), 1.0 + ROUND_MARGIN)
        };
        let mut aa = None;
        let mut density = 1.0f32;
        if !s.edge.is_off() {
            match s.tip {
                Some(_) => density = s.edge.density as f32,
                None => {
                    let soft = 1.0 - s.hardness;
                    let band = s.edge.band;
                    let plain_band = band / s.radius;
                    let max_gradient = 1.0 / (s.radius * s.roundness);
                    // 帯がいちばん広い向きでもぼかしの幅以下で、濃さ 1 のダブは今の式のまま
                    if s.edge.density != 1.0 || band * max_gradient > soft {
                        let (inner, outer) = s.round_bounds();
                        let (inner2, outer2) = (
                            inner * inner * (1.0 - ROUND_MARGIN),
                            outer * outer * (1.0 + ROUND_MARGIN),
                        );
                        (inside, outside) = if s.plain {
                            (
                                inner2 * (s.radius * s.radius),
                                outer2 * (s.radius * s.radius),
                            )
                        } else {
                            (inner2, outer2)
                        };
                        let width = if plain_band > soft { plain_band } else { soft };
                        let mid = 1.0 - soft * 0.5;
                        let half = plain_band * 0.5;
                        let m = if half > mid { half } else { mid };
                        aa = Some(Aa32 {
                            band: band as f32,
                            soft: soft as f32,
                            mid: mid as f32,
                            density: s.edge.density as f32,
                            exact_soft: s.edge.density == 1.0,
                            plain_band: plain_band as f32,
                            plain_width: width as f32,
                            plain_outer: (m + width * 0.5) as f32,
                            max_gradient: max_gradient as f32,
                            box_u: (s.radius * mid) as f32,
                            box_v: (s.radius * s.roundness * mid) as f32,
                        });
                    }
                }
            }
        }
        Shape32 {
            radius,
            hardness,
            inner: (1.0 - s.hardness) as f32,
            cos: s.cos as f32,
            sin: s.sin as f32,
            nsin: (-s.sin) as f32,
            squash,
            aspect_x: s.aspect_x as f32,
            aspect_y: s.aspect_y as f32,
            inside: inside as f32,
            outside: outside as f32,
            inv_radius2: (1.0 / (r * r)) as f32,
            inv_squash2: (1.0 / (q * q)) as f32,
            plain: s.plain,
            flip_x: s.flip_x,
            flip_y: s.flip_y,
            tip: s.tip,
            aa,
            density,
        }
    }
}

/// ストロークとダブの、ブロックに共通の f32 の値（タイルごとに 1 回作る。画素ごとの式も同じ丸め方で作る）。
pub(super) struct Dab32 {
    opacity: f32,
    flow: f32,
    flow_scale: f32,
    pressure_opacity: f32,
    pressure_flow: f32,
    /// 色の混ぜの濃さ（天井への倍率）
    density: Option<f32>,
    /// 指先の強さ（流量への倍率）
    strength: Option<f32>,
}

impl Dab32 {
    fn new(p: &Paint<'_>, s: &DabShape<'_>) -> Self {
        Dab32 {
            opacity: p.s.opacity as f32,
            flow: p.s.flow as f32,
            flow_scale: s.flow_scale as f32,
            pressure_opacity: s.pressure.opacity as f32,
            pressure_flow: s.pressure.flow as f32,
            density: p.mix.map(|m| m.density as f32),
            strength: match p.effect {
                EffectKind::Smudge(strength) => Some(strength as f32),
                _ => None,
            },
        }
    }

    /// 天井（不透明度 × 係数 × 筆圧、色の混ぜなら × 濃さ）。
    #[inline(always)]
    unsafe fn ceiling<V: Lanes32>(&self, opacity_scale: V::F) -> V::F {
        unsafe {
            let c = V::mul(
                V::mul(V::splat(self.opacity), opacity_scale),
                V::splat(self.pressure_opacity),
            );
            match self.density {
                Some(d) => V::mul(c, V::splat(d)),
                None => c,
            }
        }
    }

    /// 流量（覆い × 流量 × 係数 × 筆圧、指先なら × 強さ）。
    #[inline(always)]
    unsafe fn flow<V: Lanes32>(&self, coverage: V::F) -> V::F {
        unsafe {
            let f = V::mul(
                V::mul(
                    V::mul(coverage, V::splat(self.flow)),
                    V::splat(self.flow_scale),
                ),
                V::splat(self.pressure_flow),
            );
            match self.strength {
                Some(k) => V::mul(f, V::splat(k)),
                None => f,
            }
        }
    }
}

/// 画素ごとの式（`apply_at`）の天井と流量（[`Dab32`] と同じ値・同じ順の f32 の式）。`density`・`smudge` は色の混ぜの濃さと
/// 指先の強さ。
pub(super) fn ceiling_and_flow32(
    p: &Paint<'_>,
    pressure: PressureScale,
    (opacity_scale, flow_scale): (f32, f32),
    coverage: f32,
    density: Option<f64>,
    smudge: Option<f64>,
) -> (f32, f32) {
    let d = Dab32 {
        opacity: p.s.opacity as f32,
        flow: p.s.flow as f32,
        flow_scale,
        pressure_opacity: pressure.opacity as f32,
        pressure_flow: pressure.flow as f32,
        density: density.map(|v| v as f32),
        strength: smudge.map(|v| v as f32),
    };
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe {
        (
            d.ceiling::<Scalar1>(opacity_scale),
            d.flow::<Scalar1>(coverage),
        )
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
unsafe fn tile<V: Slice32>(
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
            let (mut cov, mut scale) = ([0.0f32; $len], [0.0f32; $len]);
            unsafe {
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
            }
        }};
    }
    if n <= 32 {
        with_buffers!(32)
    } else if n <= 128 {
        with_buffers!(128)
    } else if n <= STACK_ROW {
        with_buffers!(STACK_ROW)
    } else {
        let (mut cov, mut scale) = (vec![0.0f32; n], vec![0.0f32; n]);
        unsafe { rows::<V>(cx, held, live, dual, s, coord, xs, ys, &mut cov, &mut scale) }
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
unsafe fn rows<V: Slice32>(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dual: Option<&[f32]>,
    s: &DabShape<'_>,
    coord: TileCoord,
    xs: (i64, i64),
    ys: (i64, i64),
    cov: &mut [f32],
    scale: &mut [f32],
) -> Result<bool, CoreError> {
    let ts = cx.tile_size as i64;
    let (ox, oy) = (coord.x as i64 * ts, coord.y as i64 * ts);
    let shape = Shape32::new(s);
    let dab = Dab32::new(cx.paint, s);
    let opacity_scale = s.opacity_scale as f32;
    let mut changed = false;
    for py in ys.0..=ys.1 {
        let row = ((py - oy) * ts) as usize;
        let x0 = (xs.0 - ox) as usize;
        let (lo, hi) = unsafe { cover_row::<V>(s, &shape, py, xs, cov) };
        if lo >= hi {
            continue;
        }
        // デュアル・質感は覆いの行へ掛ける
        if let Some(mode) = s.dual {
            unsafe { dual_row::<V>(mode, dual, cov, lo, hi, row + x0) };
        }
        let with_texture = s.texture.is_some();
        if let Some(tex) = s.texture {
            unsafe { texture_row::<V>(tex, opacity_scale, py, xs.0, cov, scale, lo, hi) };
        }
        let scale = if with_texture {
            Scale::Row(&scale[..])
        } else {
            Scale::Const(opacity_scale)
        };
        let place = (row, x0);
        changed |= unsafe {
            match cx.paint.effect {
                EffectKind::Paint if cx.paint.tip_colors => apply_color_range::<V>(
                    cx,
                    held,
                    live,
                    &dab,
                    place,
                    cov,
                    scale,
                    (lo, hi),
                    (xs.0, py),
                )?,
                EffectKind::Paint => {
                    apply_range::<V>(cx, held, live, &dab, place, cov, scale, (lo, hi))?
                }
                EffectKind::Smudge(_) => apply_effect_range::<V, SMUDGE>(
                    cx,
                    held,
                    live,
                    &dab,
                    place,
                    cov,
                    scale,
                    (lo, hi),
                    (xs.0, py),
                )?,
                EffectKind::Clone => apply_effect_range::<V, CLONE>(
                    cx,
                    held,
                    live,
                    &dab,
                    place,
                    cov,
                    scale,
                    (lo, hi),
                    (xs.0, py),
                )?,
                EffectKind::Blur(_) => apply_effect_range::<V, BLUR>(
                    cx,
                    held,
                    live,
                    &dab,
                    place,
                    cov,
                    scale,
                    (lo, hi),
                    (xs.0, py),
                )?,
            }
        };
    }
    Ok(changed)
}

// ───────── 覆いの行 ─────────

/// N 画素の覆い（0 以下は塗らない）。dx・dy は画素の中心と筆の中心のずれ。`live` が偽のレーン（ウィンドウの生かさない画素）の値は
/// 使わない（筆先の画像の画素を読まない）。
#[inline(always)]
unsafe fn cover_block<V: Slice32>(c: &Shape32<'_>, dx: V::F, dy: V::F, live: Option<V::M>) -> V::F {
    unsafe {
        let Some(tip) = c.tip else {
            return round_block::<V>(c, dx, dy);
        };
        let (zero, one, half) = (V::splat(0.0), V::splat(1.0), V::splat(0.5));
        let (cos, sin, nsin) = (V::splat(c.cos), V::splat(c.sin), V::splat(c.nsin));
        let mut u = V::div(V::add(V::mul(cos, dx), V::mul(sin, dy)), V::splat(c.radius));
        let mut v = V::div(
            V::add(V::mul(nsin, dx), V::mul(cos, dy)),
            V::splat(c.squash),
        );
        if c.flip_x {
            u = V::neg(u);
        }
        if c.flip_y {
            v = V::neg(v);
        }
        let tu = V::mul(V::add(V::div(u, V::splat(c.aspect_x)), one), half);
        let tv = V::mul(V::add(V::div(v, V::splat(c.aspect_y)), one), half);
        // [0, 1]² の外は 0
        let outside = V::or(
            V::or(V::lt(tu, zero), V::lt(tv, zero)),
            V::or(V::gt(tu, one), V::gt(tv, one)),
        );
        let (w, h) = (tip.width() as i32, tip.height() as i32);
        let x = V::sub(V::mul(tu, V::splat(w as f32)), half);
        let y = V::sub(V::mul(tv, V::splat(h as f32)), half);
        let (x0, y0) = (V::floor(x), V::floor(y));
        let (fx, fy) = (V::sub(x, x0), V::sub(y, y0));
        let (mut xi, mut yi) = ([0i32; 8], [0i32; 8]);
        V::to_i32(x0, &mut xi);
        V::to_i32(y0, &mut yi);
        let alpha = tip.alpha();
        // 読む画素: 生かすレーンで、[0, 1]² の中（外のレーンの値は 0 にするので読まない）
        let mut skip = [0.0f32; 8];
        let skipped = match live {
            Some(live) => V::or(outside, V::not(live)),
            None => outside,
        };
        V::store_f32(&mut skip, V::select(skipped, one, zero));
        // 4 つの texel（画像の外の texel は 0）。全部画像の中なら範囲の確かめなしで読む
        let (mut ta, mut tb, mut tc, mut td) = ([0.0f32; 8], [0.0f32; 8], [0.0f32; 8], [0.0f32; 8]);
        let stride = w as usize;
        let texel = |x: i32, y: i32| {
            if x < 0 || y < 0 || x >= w || y >= h {
                0.0
            } else {
                f32::from(alpha[(y * w + x) as usize])
            }
        };
        for k in 0..V::N {
            if skip[k] != 0.0 {
                continue;
            }
            let (x, y) = (xi[k], yi[k]);
            if x >= 0 && x < w - 1 && y >= 0 && y < h - 1 {
                let at = y as usize * stride + x as usize;
                ta[k] = f32::from(alpha[at]);
                tb[k] = f32::from(alpha[at + 1]);
                tc[k] = f32::from(alpha[at + stride]);
                td[k] = f32::from(alpha[at + stride + 1]);
            } else {
                ta[k] = texel(x, y);
                tb[k] = texel(x.wrapping_add(1), y);
                tc[k] = texel(x, y.wrapping_add(1));
                td[k] = texel(x.wrapping_add(1), y.wrapping_add(1));
            }
        }
        let (a, b, cc, d) = (
            V::load_f32(&ta),
            V::load_f32(&tb),
            V::load_f32(&tc),
            V::load_f32(&td),
        );
        let (omfx, omfy) = (V::sub(one, fx), V::sub(one, fy));
        let top = V::add(V::mul(a, omfx), V::mul(b, fx));
        let bottom = V::add(V::mul(cc, omfx), V::mul(d, fx));
        let mut value = V::div(
            V::add(V::mul(top, omfy), V::mul(bottom, fy)),
            V::splat(255.0),
        );
        if c.density != 1.0 {
            value = V::mul(value, V::splat(c.density));
        }
        V::select(outside, zero, value)
    }
}

/// 丸の覆い: 規格化した距離 d が 1 を超えれば 0、硬さ以下なら 1、その間は smoothstep（t = (1 − d) / (1 − 硬さ)、t²(3 − 2t)）。
#[inline(always)]
unsafe fn round_block<V: Slice32>(c: &Shape32<'_>, dx: V::F, dy: V::F) -> V::F {
    unsafe {
        let (zero, one) = (V::splat(0.0), V::splat(1.0));
        let (cos, sin, nsin) = (V::splat(c.cos), V::splat(c.sin), V::splat(c.nsin));
        // 規格化した距離の 2 乗の近似（回転・潰しの無い丸は距離の 2 乗そのもの）で、確実に内側・外側の画素を決める
        let (q, ru, rv, approx) = if c.plain {
            let q = V::add(V::mul(dx, dx), V::mul(dy, dy));
            (q, zero, zero, q)
        } else {
            let ru = V::add(V::mul(cos, dx), V::mul(sin, dy));
            let rv = V::add(V::mul(nsin, dx), V::mul(cos, dy));
            let approx = V::add(
                V::mul(V::mul(ru, ru), V::splat(c.inv_radius2)),
                V::mul(V::mul(rv, rv), V::splat(c.inv_squash2)),
            );
            (zero, ru, rv, approx)
        };
        let inside = V::lt(approx, V::splat(c.inside));
        let outside = V::gt(approx, V::splat(c.outside));
        let full = match &c.aa {
            Some(aa) => V::splat(aa.density),
            None => one,
        };
        let sure = V::select(inside, full, zero);
        let decided = V::or(inside, outside);
        if V::all(decided) {
            return sure;
        }
        if let Some(aa) = &c.aa {
            let (d, floored, band) = if c.plain {
                let d = V::div(V::sqrt(q), V::splat(c.radius));
                (d, d, V::splat(aa.plain_band))
            } else {
                let u = V::div(ru, V::splat(c.radius));
                let v = V::div(rv, V::splat(c.squash));
                let uu = V::mul(u, u);
                let vv = V::mul(v, v);
                let d = V::sqrt(V::add(uu, vv));
                // 規格化した距離の勾配（画素あたり）: √(u²/r² + v²/q²) / d。中心は短い軸の向きの値
                let g = V::div(
                    V::sqrt(V::add(
                        V::mul(uu, V::splat(c.inv_radius2)),
                        V::mul(vv, V::splat(c.inv_squash2)),
                    )),
                    d,
                );
                let g = V::select(V::gt(d, zero), g, V::splat(aa.max_gradient));
                // 帯の式の距離を、半分の楕円の外接の箱の外の距離で下から押さえる（細い楕円の斜めで帯が外へ伸びない）
                let du = V::sub(V::abs(ru), V::splat(aa.box_u));
                let dv = V::sub(V::abs(rv), V::splat(aa.box_v));
                let outside = V::max(V::max(du, dv), zero);
                let floored = V::max(d, V::add(V::splat(aa.mid), V::mul(g, outside)));
                (d, floored, V::mul(V::splat(aa.band), g))
            };
            let exact = aa_block::<V>(c, aa, d, floored, band);
            return V::select(decided, sure, exact);
        }
        let d = if c.plain {
            V::div(V::sqrt(q), V::splat(c.radius))
        } else {
            let u = V::div(ru, V::splat(c.radius));
            let v = V::div(rv, V::splat(c.squash));
            V::sqrt(V::add(V::mul(u, u), V::mul(v, v)))
        };
        let t = V::div(V::sub(one, d), V::splat(c.inner));
        let smooth = V::mul(
            V::mul(t, t),
            V::sub(V::splat(3.0), V::mul(V::splat(2.0), t)),
        );
        let exact = V::select(
            V::gt(d, one),
            zero,
            V::select(V::gt(d, V::splat(c.hardness)), smooth, one),
        );
        V::select(decided, sure, exact)
    }
}

/// 丸の縁のアンチエイリアスの覆い（[`super::edge::cover64`] の f32 のレーンの版）: 規格化した距離 d、帯 band（規格化した単位）。
/// 帯の幅は max(ぼかしの幅, 帯)、帯の中心は今の式で覆いが半分になる距離（帯の半分より近ければ帯の半分）。濃さ 1 のダブで帯が
/// ぼかしの幅以下の画素は今の式。
#[inline(always)]
unsafe fn aa_block<V: Slice32>(
    c: &Shape32<'_>,
    aa: &Aa32,
    d: V::F,
    floored: V::F,
    band: V::F,
) -> V::F {
    unsafe {
        let (zero, one) = (V::splat(0.0), V::splat(1.0));
        let soft = V::splat(aa.soft);
        let (width, outer) = if c.plain {
            (V::splat(aa.plain_width), V::splat(aa.plain_outer))
        } else {
            let width = V::max(soft, band);
            let m = V::max(V::splat(aa.mid), V::mul(band, V::splat(0.5)));
            (width, V::add(m, V::mul(width, V::splat(0.5))))
        };
        let t = V::min(V::div(V::sub(outer, floored), width), one);
        let smooth = V::mul(
            V::mul(
                V::mul(t, t),
                V::sub(V::splat(3.0), V::mul(V::splat(2.0), t)),
            ),
            V::splat(aa.density),
        );
        let cover = V::select(V::ge(floored, outer), zero, smooth);
        if !aa.exact_soft {
            return cover;
        }
        // 帯がぼかしの幅以下の画素は今の式
        let t = V::div(V::sub(one, d), V::splat(c.inner));
        let old = V::mul(
            V::mul(t, t),
            V::sub(V::splat(3.0), V::mul(V::splat(2.0), t)),
        );
        let old = V::select(
            V::gt(d, one),
            zero,
            V::select(V::gt(d, V::splat(c.hardness)), old, one),
        );
        V::select(V::gt(band, soft), cover, old)
    }
}

/// 1 画素の覆い（[`cover_block`] の 1 本のレーン。SIMD の道の端の画素も通る）。
#[inline(never)]
pub(super) fn cover_one(c: &Shape32<'_>, dx: f32, dy: f32) -> f32 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { cover_block::<Scalar1>(c, dx, dy, None) }
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
pub(super) unsafe fn cover_row<V: Slice32>(
    s: &DabShape<'_>,
    c: &Shape32<'_>,
    py: i64,
    xs: (i64, i64),
    cov: &mut [f32],
) -> (usize, usize) {
    let n = (xs.1 - xs.0 + 1) as usize;
    let dy64 = (py as f64 + 0.5) - s.y;
    let (mut lo, mut hi) = (0usize, n);
    // 塗る可能性のある区間（それより外の画素は必ず覆い 0）に狭める。余裕は、外れた判定の丸めを吸う分
    let span = match s.tip {
        // 丸の外の画素は d > 1 で塗らない。行が円（半径 radius。潰した丸は半径が小さいだけ）にかかる区間。回転・潰しのときは外接の
        // 半径 radius の円で足りる
        None => {
            let reach = if s.edge.is_off() {
                s.radius * s.roundness.max(1.0)
            } else {
                s.reach()
            } + 1.5;
            if dy64.abs() > reach {
                return (0, 0);
            }
            let half = (reach * reach - dy64 * dy64).max(0.0).sqrt() + 1.5;
            Some((s.x - 0.5 - half, s.x - 0.5 + half))
        }
        Some(_) => tip_span(s, dy64),
    };
    if let Some((first, last)) = span {
        let (first, last) = (first.ceil(), last.floor());
        lo = ((first - xs.0 as f64).max(0.0)) as usize;
        hi = (((last - xs.0 as f64) + 1.0).min(n as f64).max(0.0)) as usize;
        if lo >= hi {
            return (0, 0);
        }
    }
    // 画素 i の中心のずれは、行の先頭のずれ base に i を足した値（整数の足し算は f32 で正確なので、道によらず同じ値）
    let base = (xs.0 as f64 + 0.5 - s.x) as f32;
    let dy = dy64 as f32;
    let mut i = lo;
    // 1 本のレーンの道もこのループ（余りの 1 画素ずつの式は SIMD の道の短い行だけ）
    {
        unsafe {
            let (basev, dyv) = (V::splat(base), V::splat(dy));
            while let Some(b) = next_block(i, hi, n, V::N) {
                let index = V::from_fn(|k| (b.at + k) as f32);
                let dx = V::add(basev, index);
                let v = match b.live {
                    None => cover_block::<V>(c, dx, dyv, None),
                    // ウィンドウでは生かす画素だけを書く（前のブロックが書いた画素はそのまま）
                    Some(live) => {
                        let live = window::<V>(b.at, live);
                        let v = cover_block::<V>(c, dx, dyv, Some(live));
                        V::select(live, v, V::load_f32(&cov[b.at..]))
                    }
                };
                V::store_f32(&mut cov[b.at..], v);
                i = if b.live.is_some() { hi } else { i + V::N };
            }
        }
    }
    while i < hi {
        cov[i] = cover_one(c, base + i as f32, dy);
        i += 1;
    }
    (lo, hi)
}

/// デュアルブラシの合わせ（`DualBrushMode::combine` の f32 のレーンの版。主が 0 以下なら 0）。
#[inline(always)]
unsafe fn combine<V: Lanes32>(mode: DualBrushMode, main: V::F, dual: V::F) -> V::F {
    unsafe {
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
        V::select(V::le(main, zero), zero, clamp01_32::<V>(r))
    }
}

/// 1 画素の [`combine`]。
#[inline(always)]
pub(super) fn combine32(mode: DualBrushMode, main: f32, dual: f32) -> f32 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { combine::<Scalar1>(mode, main, dual) }
}

/// 覆いの行 [lo, hi) へデュアルの溜まり（このタイルの `cells`。画素 `base + i`）を合わせる。
#[inline(always)]
pub(super) unsafe fn dual_row<V: Slice32>(
    mode: DualBrushMode,
    cells: Option<&[f32]>,
    cov: &mut [f32],
    lo: usize,
    hi: usize,
    base: usize,
) {
    let mut i = lo;
    // 1 本のレーンの道もこのループ（余りの 1 画素ずつの式は SIMD の道の短い行だけ）
    {
        unsafe {
            while let Some(b) = next_block(i, hi, cov.len(), V::N) {
                let main = V::load_f32(&cov[b.at..]);
                let dual = match cells {
                    Some(c) => V::load_f32(&c[base + b.at..]),
                    None => V::splat(0.0),
                };
                let combined = combine::<V>(mode, main, dual);
                // ウィンドウでは、生かす画素だけを合わせる（前のブロックが合わせた画素に 2 度掛けない）
                let v = match b.live {
                    None => combined,
                    Some(live) => V::select(window::<V>(b.at, live), combined, main),
                };
                V::store_f32(&mut cov[b.at..], v);
                i = if b.live.is_some() { hi } else { i + V::N };
            }
        }
    }
    while i < hi {
        cov[i] = combine32(mode, cov[i], cells.map_or(0.0, |c| c[base + i]));
        i += 1;
    }
}

/// 紙の質感の行の読み（行の y の側と、筆先の画像の大きさ・倍率）。
pub(super) struct Grain<'a> {
    alpha: &'a [u8],
    width: i32,
    row0: usize,
    row1: usize,
    fy: f32,
    scale: f32,
}

impl<'a> Grain<'a> {
    pub(super) fn new(tex: &'a PaperTexture, py: i64) -> Self {
        let r = tex.image.tiled_row((py as f64 + 0.5) / tex.scale);
        Grain {
            alpha: tex.image.alpha(),
            width: tex.image.width() as i32,
            row0: r.row0,
            row1: r.row1,
            fy: r.fy as f32,
            scale: tex.scale as f32,
        }
    }
}

/// 並べた紙の質感の N 画素（キャンバスの画素 px の中心 (px + 0.5) / 倍率 の双線形、0〜1）。
#[inline(always)]
unsafe fn grain_block<V: Slice32>(g: &Grain<'_>, px: V::F) -> V::F {
    unsafe {
        let (one, half) = (V::splat(1.0), V::splat(0.5));
        let x = V::sub(V::div(V::add(px, half), V::splat(g.scale)), half);
        let x0 = V::floor(x);
        let fx = V::sub(x, x0);
        let mut xi = [0i32; 8];
        V::to_i32(x0, &mut xi);
        // 並べた画像の列（x0 を w で割った余りと、その次の列）。レーンどうしの x0 の差は小さいので、最初のレーンの余りに差を足して、
        // [0, w) を出たときだけ剰余を取る
        let w = g.width;
        let (mut wx0, mut wx1) = ([0usize; 8], [0usize; 8]);
        let first_x = xi[0];
        let first_wrapped = first_x.rem_euclid(w);
        for k in 0..V::N {
            let mut v = first_wrapped.wrapping_add(xi[k].wrapping_sub(first_x));
            if v < 0 || v >= w {
                v = xi[k].rem_euclid(w);
            }
            wx0[k] = v as usize;
            wx1[k] = if v + 1 == w { 0 } else { v as usize + 1 };
        }
        let al = g.alpha;
        let a = V::from_fn(|k| f32::from(al[g.row0 + wx0[k]]));
        let b = V::from_fn(|k| f32::from(al[g.row0 + wx1[k]]));
        let c = V::from_fn(|k| f32::from(al[g.row1 + wx0[k]]));
        let d = V::from_fn(|k| f32::from(al[g.row1 + wx1[k]]));
        let fy = V::splat(g.fy);
        let (omfx, omfy) = (V::sub(one, fx), V::sub(one, fy));
        let top = V::add(V::mul(a, omfx), V::mul(b, fx));
        let bottom = V::add(V::mul(c, omfx), V::mul(d, fx));
        V::div(
            V::add(V::mul(top, omfy), V::mul(bottom, fy)),
            V::splat(255.0),
        )
    }
}

/// 1 画素の紙の質感（[`grain_block`] の 1 本のレーン。乗算以外の合わせ方の紙の質感が、画素ごとの式で読む）。
pub(super) fn grain_one(g: &Grain<'_>, px: i64) -> f32 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { grain_block::<Scalar1>(g, px as f32) }
}

/// 乗算の紙の質感の 1 ブロック: 不透明度の係数 opacity_scale × (1 − 深さ × (1 − 質感))。
#[inline(always)]
unsafe fn texture_scale<V: Slice32>(g: &Grain<'_>, depth: f32, base: f32, px: V::F) -> V::F {
    unsafe {
        let one = V::splat(1.0);
        let grain = grain_block::<V>(g, px);
        V::mul(
            V::splat(base),
            V::sub(one, V::mul(V::splat(depth), V::sub(one, grain))),
        )
    }
}

/// 画素ごとの式の、乗算の紙の質感の係数（[`texture_row`] の 1 画素と同じ値）。
pub(super) fn texture_scale_one(g: &Grain<'_>, depth: f32, base: f32, px: i64) -> f32 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { texture_scale::<Scalar1>(g, depth, base, px as f32) }
}

/// 乗算の紙の質感: 覆いの行 [lo, hi) の画素ごとの不透明度の係数 `scale` を作り、係数が 0 以下の画素の覆いを 0 にする。
#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn texture_row<V: Slice32>(
    tex: &PaperTexture,
    opacity_scale: f32,
    py: i64,
    x_first: i64,
    cov: &mut [f32],
    scale: &mut [f32],
    lo: usize,
    hi: usize,
) {
    let g = Grain::new(tex, py);
    let depth = tex.depth as f32;
    let mut i = lo;
    // 1 本のレーンの道もこのループ（余りの 1 画素ずつの式は SIMD の道の短い行だけ）
    {
        unsafe {
            let zero = V::splat(0.0);
            // ウィンドウで前のブロックと重なる画素は、同じ係数を書き直し、0 にした覆いをもう一度 0 にするだけ（同じ結果）
            while let Some(b) = next_block(i, hi, cov.len(), V::N) {
                // 画素の座標は整数なので f32 で正確（道によらず同じ値）
                let px = V::from_fn(|k| (x_first + (b.at + k) as i64) as f32);
                let ceiling_scale = texture_scale::<V>(&g, depth, opacity_scale, px);
                V::store_f32(&mut scale[b.at..], ceiling_scale);
                let main = V::load_f32(&cov[b.at..]);
                V::store_f32(
                    &mut cov[b.at..],
                    V::select(V::le(ceiling_scale, zero), zero, main),
                );
                i = if b.live.is_some() { hi } else { i + V::N };
            }
        }
    }
    while i < hi {
        let ceiling_scale = texture_scale_one(&g, depth, opacity_scale, x_first + i as i64);
        scale[i] = ceiling_scale;
        if ceiling_scale <= 0.0 {
            cov[i] = 0.0;
        }
        i += 1;
    }
}

// ───────── 当ての行 ─────────

/// タイルの画素 index から N 画素（面のタイルの今の値）。
#[inline(always)]
unsafe fn load_live<V: Lanes32>(live: &LiveTile, pixel: usize) -> [V::F; 4] {
    unsafe {
        match live {
            LiveTile::Absent => [V::splat(0.0); 4],
            LiveTile::Uniform(c) => V::splat_px(c.to_array()),
            LiveTile::Shared(_, d) => V::load(&d[pixel * 4..]),
            LiveTile::Owned(v) => V::load(&v[pixel * 4..]),
        }
    }
}

/// 描く前の画素から N 画素。
#[inline(always)]
unsafe fn load_before<V: Lanes32>(st: &StrokeTile, pixel: usize) -> [V::F; 4] {
    unsafe {
        match &st.before_px {
            None => [V::splat(0.0); 4],
            Some(Pixels::Uniform(c)) => V::splat_px(c.to_array()),
            Some(Pixels::Data(d)) => V::load(&d[pixel * 4..]),
        }
    }
}

/// 面のタイルの画素 local から N 画素へ out を書く（今の値 current と違う画素があるときだけ）。違う画素があれば true。
#[inline(always)]
unsafe fn commit<V: Lanes32>(
    cx: &mut PixelContext<'_>,
    live: &mut LiveTile,
    local: usize,
    out: [V::F; 4],
    current: [V::F; 4],
) -> Result<bool, CoreError> {
    unsafe {
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
                // 一様・共有・無いタイルは、最初に変わる画素の書き込みで自分のものにする（予算の確かめも今までと同じ）。
                // 変わらない画素（ウィンドウの生かさない画素を含む）は書かない
                let (mut bytes, mut was) = ([0u8; 32], [0u8; 32]);
                V::store(&mut bytes, out);
                V::store(&mut was, current);
                for k in 0..V::N {
                    if bytes[k * 4..k * 4 + 4] == was[k * 4..k * 4 + 4] {
                        continue;
                    }
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
}

/// 覆いの行 cov の [lo, hi) を、ストロークの覆いと面へ当てる（色を塗る/消すだけのブラシ。`apply_at` の同じ道と同じ結果）。
/// N 画素ずつのあと、余りの画素は 1 本のレーンで同じ式を通る。
#[inline(always)]
#[allow(clippy::too_many_arguments)]
unsafe fn apply_range<V: Slice32>(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dab: &Dab32,
    place: (usize, usize),
    cov: &[f32],
    scale: Scale<'_>,
    (lo, hi): (usize, usize),
) -> Result<bool, CoreError> {
    let (changed, i) =
        unsafe { paint_blocks::<V>(cx, held, live, dab, place, cov, scale, (lo, hi))? };
    if i < hi {
        return Ok(changed | paint_rest(cx, held, live, dab, place, cov, scale, (i, hi))?);
    }
    Ok(changed)
}

/// [`apply_range`] の余りの画素（1 本のレーン。どの道の余りもこの 1 つの関数）。
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn paint_rest(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dab: &Dab32,
    place: (usize, usize),
    cov: &[f32],
    scale: Scale<'_>,
    range: (usize, usize),
) -> Result<bool, CoreError> {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    unsafe { paint_blocks::<Scalar1>(cx, held, live, dab, place, cov, scale, range).map(|r| r.0) }
}

/// [lo, hi) を N 画素ずつ当て、変わったかと、止まった画素（余りの先頭）を返す。
#[inline(always)]
#[allow(clippy::too_many_arguments)]
unsafe fn paint_blocks<V: Slice32>(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dab: &Dab32,
    (row, x0): (usize, usize),
    cov: &[f32],
    scale: Scale<'_>,
    (lo, hi): (usize, usize),
) -> Result<(bool, usize), CoreError> {
    unsafe {
        let p = cx.paint;
        let s = p.s;
        let mut changed = false;
        let (zero, one) = (V::splat(0.0), V::splat(1.0));
        let color = V::splat_px(p.stroke_color.to_array());
        let erase_alpha = V::splat(f32::from(s.color.a));
        // 選択範囲の量（このタイルの画素ごと。無ければ 1）と、透明部分のロック
        let amounts = match cx.selected {
            Selected::Tile(a) => Some(a),
            _ => None,
        };
        let keep_alpha = p.keep_alpha;
        let mut i = lo;
        let n = if V::N > 1 { cov.len() } else { 0 };
        while let Some(b) = next_block(i, hi, n, V::N) {
            i = if b.live.is_some() { hi } else { i + V::N };
            let local = row + x0 + b.at;
            let coverage = block_coverage::<V>(cov, b);
            let ceiling = dab.ceiling::<V>(scale.lanes::<V>(b.at));
            let flow = dab.flow::<V>(coverage);
            // flow <= 0 || ceiling <= 0 は塗らない（NaN は塗る側。画素ごとの式の `if flow <= 0.0 || ceiling <= 0.0` と同じ）
            let mut active = V::not(V::or(V::le(flow, zero), V::le(ceiling, zero)));
            // 選択されていない画素・透明部分のロックの透明な画素は何もしない（写しも取らない）
            let selected = match amounts {
                None => one,
                Some(Amounts::Uniform(v)) => V::unit(V::splat(f32::from(*v))),
                Some(Amounts::Data(d)) => V::unit(V::from_fn(|k| f32::from(d[local + k]))),
            };
            if amounts.is_some() {
                active = V::and(active, V::not(V::le(selected, zero)));
            }
            if keep_alpha {
                let now = load_live::<V>(live, local);
                active = V::and(active, V::not(V::eq(now[3], zero)));
            }
            if !V::any(active) {
                continue;
            }
            let previous = match held.as_ref() {
                Some(st) => {
                    let w = V::load_f32(&st.wash[local..]);
                    // ストロークの覆いが天井に届いた画素は何もしない
                    active = V::and(active, V::not(V::ge(w, ceiling)));
                    w
                }
                None => zero,
            };
            if !V::any(active) {
                continue;
            }
            if held.is_none() {
                *held = Some(new_stroke_tile(cx, live, true)?);
            }
            let st = held.as_mut().expect("直前に作った");
            let accumulated = accumulate::<V>(previous, ceiling, flow);
            V::store_f32(
                &mut st.wash[local..],
                V::select(active, accumulated, previous),
            );
            let start = load_before::<V>(st, local);
            let amount = V::min(one, accumulated);
            let next = if s.erase {
                erase_block::<V>(start, accumulated, erase_alpha)
            } else if keep_alpha {
                // アルファは描く前のまま。色だけを（描く色のアルファ × 量）の割合で寄せる
                keeping_block::<V>(start, color, V::mul(amount, selected))
            } else {
                normal_block::<V>(start, color, amount)
            };
            // 半分だけ選ばれた画素は、描く前の画素から選ばれた量だけ寄せる
            let part = V::lt(selected, one);
            let next = if amounts.is_some() && !keep_alpha && V::any(part) {
                let faded = fade_block::<V>(start, next, selected);
                [
                    V::select(part, faded[0], next[0]),
                    V::select(part, faded[1], next[1]),
                    V::select(part, faded[2], next[2]),
                    V::select(part, faded[3], next[3]),
                ]
            } else {
                next
            };
            let current = load_live::<V>(live, local);
            let out = [
                V::select(active, next[0], current[0]),
                V::select(active, next[1], current[1]),
                V::select(active, next[2], current[2]),
                V::select(active, next[3], current[3]),
            ];
            changed |= commit::<V>(cx, live, local, out, current)?;
        }
        Ok((changed, i))
    }
}

// ───────── デュアルブラシの溜まり ─────────

/// デュアルブラシの 2 つ目の筆先のダブの、1 枚のタイル分の溜まり（`dual_dab_at` のタイルのループ）をレーンで行う。溜まりの置き場
/// （`cells`）は、覆いのある画素を最初に見たときに `alloc`（予算の確かめつき）で作る。画素ごとに自分の覆いと今の値の大きい方を
/// 取るだけなので、行や画素の順は結果を変えない。
pub(super) fn dual_tile(
    shape: &DualShape<'_>,
    xs: (i64, i64),
    ys: (i64, i64),
    origin: (i64, i64, usize),
    cells: &mut Option<Vec<f32>>,
    alloc: &mut dyn FnMut() -> Result<Vec<f32>, CoreError>,
) -> Result<(), CoreError> {
    match simd::level() {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level() は detect() 以下なので、AVX2 を持つ
        Level::Avx2 => unsafe { dual_avx2(shape, xs, ys, origin, cells, alloc) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level() は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { dual_sse41(shape, xs, ys, origin, cells, alloc) },
        #[cfg(target_arch = "aarch64")]
        // SAFETY: level() は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
        Level::Neon => unsafe { dual_neon(shape, xs, ys, origin, cells, alloc) },
        // SAFETY: 1 本のレーンは CPU の前提を持たない
        Level::Scalar => unsafe { dual_rows::<Scalar1>(shape, xs, ys, origin, cells, alloc) },
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
    unsafe { dual_rows::<Avx2x8>(shape, xs, ys, origin, cells, alloc) }
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
    unsafe { dual_rows::<Sse41x4>(shape, xs, ys, origin, cells, alloc) }
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn dual_neon(
    shape: &DualShape<'_>,
    xs: (i64, i64),
    ys: (i64, i64),
    origin: (i64, i64, usize),
    cells: &mut Option<Vec<f32>>,
    alloc: &mut dyn FnMut() -> Result<Vec<f32>, CoreError>,
) -> Result<(), CoreError> {
    unsafe { dual_rows::<Neonx4>(shape, xs, ys, origin, cells, alloc) }
}

/// デュアルの 2 つ目の筆先の形を、主のダブの形（丸も回転の式で測る = `plain` でない。反転・紙の質感・デュアルは無い）として。
pub(super) fn dual_cover_shape<'a>(shape: &DualShape<'a>) -> DabShape<'a> {
    DabShape {
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
        edge: shape.edge,
    }
}

#[inline(always)]
unsafe fn dual_rows<V: Slice32>(
    shape: &DualShape<'_>,
    xs: (i64, i64),
    ys: (i64, i64),
    (ox, oy, ts): (i64, i64, usize),
    cells: &mut Option<Vec<f32>>,
    alloc: &mut dyn FnMut() -> Result<Vec<f32>, CoreError>,
) -> Result<(), CoreError> {
    let cover = dual_cover_shape(shape);
    let c = Shape32::new(&cover);
    let n = (xs.1 - xs.0 + 1) as usize;
    let mut stack = [0.0f32; STACK_ROW];
    let mut heap = Vec::new();
    let cov: &mut [f32] = if n <= STACK_ROW {
        &mut stack[..n]
    } else {
        heap.resize(n, 0.0);
        &mut heap
    };
    let x0 = (xs.0 - ox) as usize;
    for py in ys.0..=ys.1 {
        let (lo, hi) = unsafe { cover_row::<V>(&cover, &c, py, xs, cov) };
        if lo >= hi {
            continue;
        }
        if cells.is_none() {
            if !cov[lo..hi].iter().any(|&c| c > 0.0) {
                continue;
            }
            *cells = Some(alloc()?);
        }
        let cells = cells.as_mut().expect("直前に作った");
        let row = ((py - oy) * ts as i64) as usize + x0;
        let mut i = lo;
        // 1 本のレーンの道もこのループ（余りの 1 画素ずつの式は SIMD の道の短い行だけ）
        {
            unsafe {
                // 大きい方を取るだけなので、ウィンドウで前のブロックと重なる画素をもう一度通しても同じ
                while let Some(b) = next_block(i, hi, n, V::N) {
                    let coverage = block_coverage::<V>(cov, b);
                    let current = V::load_f32(&cells[row + b.at..]);
                    V::store_f32(
                        &mut cells[row + b.at..],
                        V::select(V::gt(coverage, current), coverage, current),
                    );
                    i = if b.live.is_some() { hi } else { i + V::N };
                }
            }
        }
        while i < hi {
            if cov[i] > cells[row + i] {
                cells[row + i] = cov[i];
            }
            i += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
