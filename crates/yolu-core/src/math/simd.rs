//! 画素の計算の SIMD の土台: 実行時の道の選び（[`Level`]）と、f64 の並び（レーン）の演算の型（[`Lanes`]）・f32 の並びの演算の型
//! （[`Lanes32`]。レイヤーの合成の式が使う）。
//!
//! 道は CPU の種類ごとに決まる。
//!
//! - x86_64: AVX2（+ FMA）・SSE4.1・スカラーの 3 つ。`is_x86_feature_detected!` で CPU に合う一番広い道を選ぶ（Windows の配布物
//!   x86_64-pc-windows-msvc でも同じ）。環境変数 `YOLU_SIMD` は `scalar`・`sse41`・`avx2`。
//! - aarch64（Apple Silicon の Mac など）: NEON（f64 が 2 本・f32 が 4 本）・スカラーの 2 つ。NEON は AArch64 の基本の命令なので、実行時の
//!   判定はせず、この CPU の一番広い道は常に NEON。`YOLU_SIMD` は `scalar`・`neon`。
//! - それ以外の CPU: スカラーだけ。
//!
//! 環境変数 `YOLU_SIMD` は広げる向きには効かない（CPU が持つ道より広い値や、別の CPU の道の名前・知らない値は無視して、CPU が持つ一番広い
//! 道のまま）。スカラーの道は SIMD を入れる前と同じ式をそのまま使う。道の型 [`Level`] の変種は、その CPU でビルドされる道だけなので、
//! 別の CPU の道が選ばれることはなく、道ごとの入口を選ぶ `match` は、その CPU の道とスカラーを全部数える（数え漏らすとビルドが通らない）。
//!
//! **結果のバイトは道によらず同じ**。レーンの演算は IEEE の足す・引く・掛ける・割る・平方根と比較・選択だけで、式の演算の順はスカラーの式と
//! 同じにしてある（Rust も LLVM も浮動小数の積和を勝手にまとめないので、スカラーの道と同じ double を経る）。x86_64 の FMA は `b / 255.0` を
//! 割り算なしで出すところだけで使い、その値が割り算と 256 通りの全部で同じ bit であることは試験で確かめる。NEON の型は積和の命令
//! （`vfma`・`vmla`）も、NaN と符号つきの 0 の扱いがスカラーの比較と違う `vmin`・`vmax` も使わず、`min`・`max` は比較と選択で作る。
//!
//! `unsafe` はレーンの演算の中の組み込み関数の呼び出しと、道ごとの入口の呼び出しだけ。[`Lanes`] のメソッドは `unsafe fn` で、それを使う
//! 式の関数（`<V: Lanes>`）も `unsafe fn`。呼び出しの前提（CPU がその命令を持つこと）は型では守れないので、道ごとの入口
//! （`#[target_feature]` 付きの `unsafe fn`）の中からだけ使い、入口を [`level`] の結果でだけ呼ぶことで守る。安全な関数から
//! `Avx2` のメソッドを呼ぶ道は無い（aarch64 の NEON の入口も同じ作りで、`#[target_feature(enable = "neon")]` を付ける）。
#![cfg_attr(
    not(any(target_arch = "x86_64", target_arch = "aarch64")),
    allow(dead_code, unused_imports, unused_macros, unused_variables, unused_mut)
)]

use std::sync::atomic::{AtomicU8, Ordering};

/// 計算の道。大きい方が広い。変種は、その CPU でビルドされる道（スカラーと、x86_64 なら SSE4.1・AVX2、aarch64 なら NEON）だけ。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Level {
    /// 画素ごとのふつうの計算（SIMD を入れる前と同じ式）。
    Scalar = 0,
    /// x86_64 の SSE4.1（f64 が 2 本）。
    #[cfg(target_arch = "x86_64")]
    Sse41 = 1,
    /// x86_64 の AVX2 + FMA（f64 が 4 本）。
    #[cfg(target_arch = "x86_64")]
    Avx2 = 2,
    /// aarch64 の NEON（f64 が 2 本・f32 が 4 本）。
    #[cfg(target_arch = "aarch64")]
    Neon = 1,
}

impl Level {
    /// CPU が持つ道のうち、狭い方から全部（試験と、道の名前の表のため）。
    #[cfg(test)]
    const ALL: &'static [Level] = &[
        Level::Scalar,
        #[cfg(target_arch = "x86_64")]
        Level::Sse41,
        #[cfg(target_arch = "x86_64")]
        Level::Avx2,
        #[cfg(target_arch = "aarch64")]
        Level::Neon,
    ];

    fn from_u8(v: u8) -> Level {
        match v {
            #[cfg(target_arch = "x86_64")]
            2 => Level::Avx2,
            #[cfg(target_arch = "x86_64")]
            1 => Level::Sse41,
            #[cfg(target_arch = "aarch64")]
            1 => Level::Neon,
            _ => Level::Scalar,
        }
    }

    /// 道の名前（環境変数 `YOLU_SIMD` の値と同じ綴り）。
    pub(crate) fn name(self) -> &'static str {
        match self {
            Level::Scalar => "scalar",
            #[cfg(target_arch = "x86_64")]
            Level::Sse41 => "sse41",
            #[cfg(target_arch = "x86_64")]
            Level::Avx2 => "avx2",
            #[cfg(target_arch = "aarch64")]
            Level::Neon => "neon",
        }
    }
}

/// この CPU で使える一番広い道（環境変数は見ない）。
pub(crate) fn detect() -> Level {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
            return Level::Avx2;
        }
        if is_x86_feature_detected!("sse4.1") {
            return Level::Sse41;
        }
    }
    // NEON は AArch64 の基本の命令（ARMv8-A の浮動小数点・SIMD）で、Rust の aarch64 のターゲット（macOS・Linux・Windows）は
    // 既定で有効にしている。持たない CPU は無いので、実行時の判定はしない
    #[cfg(target_arch = "aarch64")]
    {
        Level::Neon
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        Level::Scalar
    }
}

/// 指定（環境変数 `YOLU_SIMD` の値）に従って、CPU が持つ道 `detected` を下げる（広げる向きには効かない）。この CPU の道の名前
/// （x86_64 は `sse41`・`avx2`、aarch64 は `neon`）と `scalar` だけを受け、別の CPU の道の名前・知らない値は無視する。
fn clamp_to_request(request: Option<&str>, detected: Level) -> Level {
    let wanted = match request {
        Some("scalar") => Level::Scalar,
        #[cfg(target_arch = "x86_64")]
        Some("sse41") => Level::Sse41,
        #[cfg(target_arch = "x86_64")]
        Some("avx2") => Level::Avx2,
        #[cfg(target_arch = "aarch64")]
        Some("neon") => Level::Neon,
        _ => return detected,
    };
    wanted.min(detected)
}

fn from_environment(detected: Level) -> Level {
    clamp_to_request(std::env::var("YOLU_SIMD").ok().as_deref(), detected)
}

const UNSET: u8 = u8::MAX;
static CHOSEN: AtomicU8 = AtomicU8::new(UNSET);

/// 使う道（初めて呼ぶときに 1 回だけ決める）。
#[inline]
pub(crate) fn level() -> Level {
    #[cfg(test)]
    if let Some(forced) = forced::get() {
        return forced;
    }
    match CHOSEN.load(Ordering::Relaxed) {
        UNSET => {
            let chosen = from_environment(detect());
            CHOSEN.store(chosen as u8, Ordering::Relaxed);
            chosen
        }
        v => Level::from_u8(v),
    }
}

/// 試験で道を固定するための口。固定中も、ほかの試験はどの道でも同じバイトを出すので邪魔にならない（固定する試験どうしだけ鍵で直列にする）。
#[cfg(test)]
pub(crate) mod forced {
    use super::*;
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());
    static FORCED: AtomicU8 = AtomicU8::new(UNSET);

    pub(super) fn get() -> Option<Level> {
        match FORCED.load(Ordering::SeqCst) {
            UNSET => None,
            v => Some(Level::from_u8(v)),
        }
    }

    /// この CPU が持つ道を、狭い方から。「どの道でも同じバイト」の試験はこの一覧の全部をスカラーと比べるので、一覧にこの CPU の
    /// 一番広い道（aarch64 なら NEON）が入っていることをここで確かめる（入っていないと、SIMD の道を比べないまま通ってしまう）。
    pub(crate) fn supported() -> Vec<Level> {
        let levels: Vec<Level> = Level::ALL
            .iter()
            .copied()
            .filter(|l| *l <= detect())
            .collect();
        assert_eq!(levels.first(), Some(&Level::Scalar));
        assert_eq!(levels.last(), Some(&detect()));
        #[cfg(target_arch = "aarch64")]
        assert_eq!(levels, [Level::Scalar, Level::Neon]);
        levels
    }

    /// `level()` が `l` を返す間だけ `f` を走らせる。
    pub(crate) fn with_level<R>(l: Level, f: impl FnOnce() -> R) -> R {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(l <= detect(), "この CPU は {l:?} を持たない");
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                FORCED.store(UNSET, Ordering::SeqCst);
            }
        }
        FORCED.store(l as u8, Ordering::SeqCst);
        let _reset = Reset;
        f()
    }
}

/// f64 のレーン（AVX2 は 4 本、SSE4.1・NEON は 2 本）の演算。メソッドはどれも `#[inline(always)]` で、`#[target_feature]` 付きの入口の中で
/// 1 つの関数に畳まれる。比較は Rust の f64 の比較と同じ（NaN は偽）で、`min`・`max` は `if a < b { a } else { b }`・`if a > b { a } else { b }`。
///
/// レーンの演算を含む式の中にクロージャを置かない。クロージャは `#[target_feature]` を引き継がず、インライン化されないと AVX2 の命令が SSE に
/// 割られて 3 倍ほど遅くなる（`from_fn` に渡す表引きのように、レーンの演算を含まないものは構わない）。
///
/// # Safety
///
/// メソッドはすべて `unsafe fn`。実装の型（[`Avx2`]・[`Sse41`]・aarch64 の `Neon`）は誰でも名指しできるので、「CPU がその命令を持つ」ことは
/// 型では守れない。呼ぶ側が、その命令（AVX2 は `avx2` と `fma`、SSE4.1 は `sse4.1`、NEON は `neon`）を持つ CPU の上で走っていることを保つ。
/// 具体的には、道ごとの入口（同じ命令を有効にした `#[target_feature]` 付きの `unsafe fn`）の中からだけ呼び、その入口は [`level`] の結果
/// （[`detect`] 以下）で選んだ道でだけ呼ぶ（NEON は aarch64 のどの CPU にもあるので、aarch64 でビルドした物の中ならどこからでも呼べるが、入口を
/// 通す作りは x86_64 と揃える）。メモリの安全はこの前提とは別で、画素の読み書きはどれも長さを確かめてから行い、足りなければ panic する。
pub(crate) trait Lanes: Copy {
    /// レーンの本数。
    const N: usize;
    /// f64 のレーン。
    type F: Copy;
    /// 比較の結果（レーンごとに真か偽）。
    type M: Copy;

    unsafe fn splat(x: f64) -> Self::F;
    /// レーン k に f(k) を入れる（表引きなど、ベクトルの命令にならない値を集める）。k は 0 から順に呼ぶ。
    unsafe fn from_fn(f: impl FnMut(usize) -> f64) -> Self::F;
    /// レーン k の値（試験用）。
    #[cfg(test)]
    unsafe fn lane(v: Self::F, k: usize) -> f64;

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
    #[cfg_attr(not(test), allow(dead_code))]
    unsafe fn or(a: Self::M, b: Self::M) -> Self::M;
    unsafe fn not(a: Self::M) -> Self::M;
    /// 真のレーンは a、偽のレーンは b。選ばれなかった方が NaN でも結果に影響しない。
    unsafe fn select(m: Self::M, a: Self::F, b: Self::F) -> Self::F;
    unsafe fn any(m: Self::M) -> bool;
    unsafe fn all(m: Self::M) -> bool;

    /// b / 255.0（b は 0〜255 の整数の値）。`UNIT[b]` と同じ bit。
    unsafe fn unit(b: Self::F) -> Self::F;

    /// 連続する N 画素の RGBA（4N バイト以上のスライスの先頭）を、R・G・B・A のレーンへ（値は 0〜255 の整数）。
    unsafe fn load(p: &[u8]) -> [Self::F; 4];
    /// 連続する N 個の f64 をレーンへ。
    unsafe fn load_f64(p: &[f64]) -> Self::F;
    /// レーンを連続する N 個の f64 へ。
    unsafe fn store_f64(p: &mut [f64], v: Self::F);
    /// 連続する N 個の u16 をレーンへ（値は整数）。
    unsafe fn load_u16(p: &[u16]) -> Self::F;
    /// レーン（0〜65535 の整数の値）を連続する N 個の u16 へ。
    unsafe fn store_u16(p: &mut [u16], v: Self::F);
    /// 連続する N 個の u32（2³¹ 未満）をレーンへ（値は整数）。
    unsafe fn load_u32(p: &[u32]) -> Self::F;
    /// レーン（0〜2³¹ 未満の整数の値）を連続する N 個の u32 へ。
    unsafe fn store_u32(p: &mut [u32], v: Self::F);
    /// 連続する N 画素の u16 の 4 チャンネル（4N 個以上のスライスの先頭、R・G・B・A の順）を、チャンネルごとのレーンへ。
    unsafe fn load_u16x4(p: &[u16]) -> [Self::F; 4];
    /// チャンネルごとのレーン（0〜65535 の整数の値）を、連続する N 画素の u16 の 4 チャンネル（4N 個以上のスライスの先頭、R・G・B・A の順）へ。
    unsafe fn store_u16x4(p: &mut [u16], v: [Self::F; 4]);
    /// 1 つの画素をすべてのレーンへ。
    #[cfg_attr(not(test), allow(dead_code))]
    unsafe fn splat_px(px: [u8; 4]) -> [Self::F; 4];
    /// R・G・B・A のレーン（0〜255 の整数の値）を連続する N 画素の RGBA（4N バイト以上のスライスの先頭）へ。
    unsafe fn store(p: &mut [u8], v: [Self::F; 4]);
}

// ───────── レーンの上の共通の式 ─────────

/// 0〜1 の値を 0〜255 の整数の値へ（`math::to_byte` と同じ式。NaN は 0）。
#[inline(always)]
pub(crate) unsafe fn to_byte<V: Lanes>(value: V::F) -> V::F {
    let v = V::add(V::mul(value, V::splat(255.0)), V::splat(0.5));
    // `max(v, 0)` は v ≤ 0 と NaN を 0 にし、`min(.., 255)` は 255 以上を 255 にする。正の v < 255 の floor は切り捨て
    V::floor(V::min(V::max(v, V::splat(0.0)), V::splat(255.0)))
}

/// `if v < 0 { 0 } else if v > 1 { 1 } else { v }`
#[inline(always)]
pub(crate) unsafe fn clamp01<V: Lanes>(v: V::F) -> V::F {
    V::select(
        V::lt(v, V::splat(0.0)),
        V::splat(0.0),
        V::select(V::gt(v, V::splat(1.0)), V::splat(1.0), v),
    )
}

#[cfg(target_arch = "x86_64")]
mod x86;
#[cfg(target_arch = "x86_64")]
pub(crate) use x86::{Avx2, Sse41};

#[cfg(target_arch = "aarch64")]
mod arm;
#[cfg(target_arch = "aarch64")]
pub(crate) use arm::{Neon, Neonx4};

mod lanes32;
pub(crate) use lanes32::{clamp01_32, to_byte32, Lanes32, Scalar1};
#[cfg(target_arch = "x86_64")]
pub(crate) use lanes32::{Avx2x8, Sse41x4};

/// 試験用: [`on_each_level`] の f32 のレーン（[`Lanes32`]）の版。1 本のレーン（[`Scalar1`]）と、この CPU が持つ SIMD の道
/// （AVX2・SSE4.1・NEON）ごとに、その命令を有効にした入口の中で `f::<V>()` を走らせる。
#[cfg(test)]
macro_rules! on_each_level32 {
    ($f:ident) => {{
        // SAFETY: 1 本のレーンは CPU の前提を持たない
        unsafe { $f::<$crate::math::simd::Scalar1>() }
        #[cfg(target_arch = "x86_64")]
        {
            #[target_feature(enable = "avx2,fma")]
            unsafe fn avx2() {
                $f::<$crate::math::simd::Avx2x8>()
            }
            #[target_feature(enable = "sse4.1")]
            unsafe fn sse41() {
                $f::<$crate::math::simd::Sse41x4>()
            }
            if $crate::math::simd::detect() >= $crate::math::simd::Level::Avx2 {
                // SAFETY: avx2 と fma を持つことを確かめた
                unsafe { avx2() }
            }
            if $crate::math::simd::detect() >= $crate::math::simd::Level::Sse41 {
                // SAFETY: sse4.1 を持つことを確かめた
                unsafe { sse41() }
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            #[target_feature(enable = "neon")]
            unsafe fn neon() {
                $f::<$crate::math::simd::Neonx4>()
            }
            if $crate::math::simd::detect() >= $crate::math::simd::Level::Neon {
                // SAFETY: neon を持つことを確かめた
                unsafe { neon() }
            }
        }
    }};
}
#[cfg(test)]
pub(crate) use on_each_level32;

/// 試験用: ジェネリックな関数 `f::<V>()` を、この CPU が持つ SIMD の道（AVX2・SSE4.1・NEON）ごとに、その命令を有効にした入口の中で走らせる。
/// スカラーの道は SIMD の型を持たないので、呼び出し側が別に確かめる。
#[cfg(test)]
macro_rules! on_each_level {
    ($f:ident) => {{
        #[cfg(target_arch = "x86_64")]
        {
            #[target_feature(enable = "avx2,fma")]
            unsafe fn avx2() {
                $f::<$crate::math::simd::Avx2>()
            }
            #[target_feature(enable = "sse4.1")]
            unsafe fn sse41() {
                $f::<$crate::math::simd::Sse41>()
            }
            if $crate::math::simd::detect() >= $crate::math::simd::Level::Avx2 {
                // SAFETY: avx2 と fma を持つことを確かめた
                unsafe { avx2() }
            }
            if $crate::math::simd::detect() >= $crate::math::simd::Level::Sse41 {
                // SAFETY: sse4.1 を持つことを確かめた
                unsafe { sse41() }
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            #[target_feature(enable = "neon")]
            unsafe fn neon() {
                $f::<$crate::math::simd::Neon>()
            }
            if $crate::math::simd::detect() >= $crate::math::simd::Level::Neon {
                // SAFETY: neon を持つことを確かめた
                unsafe { neon() }
            }
        }
    }};
}
#[cfg(test)]
pub(crate) use on_each_level;

#[cfg(test)]
pub(crate) mod tests;
