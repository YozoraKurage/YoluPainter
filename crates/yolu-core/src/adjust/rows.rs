//! 調整レイヤーの行ごとの合成（[`AdjustKernel::composite_row`]）。各画素は [`AdjustKernel::composite`] と同じバイトになる。
//!
//! 2 段で処理する: (1) 調整した色を求める（値だけで決まる種類は表引き・整数の計算のまま、色相/彩度とカラーバランスは
//! f32 の式をレーンで）、(2) 下とモードと量で混ぜる（`blend` の行の核と同じレーンの式）。段の間は 256 画素ずつの小さい作業の領域で
//! 受け渡す。
//!
//! 色相/彩度とカラーバランスの f32 の式は色の合成（`crate::blend`）と同じ形で、道（AVX2 8 画素・SSE4.1 と NEON 4 画素・スカラー 1 画素）の
//! どれも同じレーンの式を通り、端の画素・スカラーの道・画素ごとの式（`AdjustmentSettings::apply_in`・
//! [`ColorBalance::apply`](super::ColorBalance::apply)）は 1 本のレーン（[`Scalar1`]）で同じ関数を呼ぶ。演算は IEEE の四則・比較・
//! 選択・floor だけなので、道とスレッド数によらず同じバイトになる。
#![cfg_attr(
    not(any(target_arch = "x86_64", target_arch = "aarch64")),
    allow(dead_code, unused_imports, unused_macros, unused_variables, unused_mut)
)]

use super::{AdjustKernel, AdjustmentType, ColorBalance, More};
use crate::blend::{mix_row_at, RowAmount};
use crate::math::simd::{self, clamp01_32, to_byte32, Lanes32, Level, Scalar1};
use crate::types::{BlendMode, ChannelKind, Rgba8};

#[cfg(target_arch = "aarch64")]
use crate::math::simd::Neonx4;
#[cfg(target_arch = "x86_64")]
use crate::math::simd::{Avx2x8, Sse41x4};

/// 段の間に受け渡す画素数。
const CHUNK: usize = 256;

impl AdjustKernel {
    /// 1 行（RGBA）に調整レイヤーを重ねる。量が 0 以下・下が完全に透明な画素はそのまま。
    #[inline]
    pub(crate) fn composite_row(&self, row: &mut [u8], amount: RowAmount<'_>, mode: BlendMode) {
        self.composite_row_at(simd::level(), row, amount, mode)
    }

    pub(crate) fn composite_row_at(
        &self,
        level: Level,
        row: &mut [u8],
        amount: RowAmount<'_>,
        mode: BlendMode,
    ) {
        if amount.mask.is_none() && amount.opacity <= 0.0 {
            return; // 何も変わらない
        }
        let level = level.min(simd::detect());
        let mut adjusted = [0u8; CHUNK * 4];
        for (n, chunk) in row.chunks_mut(CHUNK * 4).enumerate() {
            let adjusted = &mut adjusted[..chunk.len()];
            self.adjust_chunk(level, chunk, adjusted);
            mix_row_at(level, chunk, adjusted, amount.offset(n * CHUNK), mode);
        }
    }

    /// 調整後の色（RGBA、アルファは下のまま）を out へ。色相/彩度・カラーバランスはレーンの式、ほかは 1 画素ずつの式（表引き・整数）。
    fn adjust_chunk(&self, level: Level, src: &[u8], out: &mut [u8]) {
        match self.lane_kind() {
            Some(LaneKind::ColorBalance(balance)) => balance_rows_at(level, &balance, src, out),
            Some(LaneKind::HueSaturation(h)) => hue_rows32_at(level, &h, src, out),
            None => self.adjust_pixels(src, out),
        }
    }

    /// 1 画素ずつ（[`AdjustKernel::composite`] の `adjusted`）。
    fn adjust_pixels(&self, src: &[u8], out: &mut [u8]) {
        for i in 0..src.len() / 4 {
            let below = Rgba8::from_slice(&src[i * 4..]);
            let adjusted = match &self.table {
                Some(t) => Rgba8::new(
                    t[below.r as usize],
                    t[below.g as usize],
                    t[below.b as usize],
                    below.a,
                ),
                None => self.settings.apply_in(self.channel, below),
            };
            out[i * 4..i * 4 + 4].copy_from_slice(&adjusted.to_array());
        }
    }

    /// レーンで計算する種類（浮動小数の式が重いもの）。
    fn lane_kind(&self) -> Option<LaneKind> {
        if self.channel != ChannelKind::Color {
            return None; // 色相/彩度・カラーバランスは色のチャンネルだけ（ほかではレイヤーが適用されない）
        }
        match (&self.settings.kind, &self.settings.more) {
            (AdjustmentType::HueSaturation, _) => Some(LaneKind::HueSaturation(HueSat32::new(
                self.settings.hue,
                self.settings.saturation,
                self.settings.lightness,
            ))),
            (_, More::ColorBalance(c)) if !c.is_neutral() => {
                let (values, preserve) = c.parts();
                Some(LaneKind::ColorBalance(Balance32::new(values, preserve)))
            }
            _ => None,
        }
    }
}

/// カラーバランスを RGBA の並び（src）へ当てた結果を out へ（各画素は `ColorBalance::apply` と同じバイト、アルファはそのまま）。
/// フィルターの段が使う。
pub(crate) fn color_balance_rgba(level: Level, balance: &ColorBalance, src: &[u8], out: &mut [u8]) {
    if balance.is_neutral() {
        out.copy_from_slice(src);
        return;
    }
    let (values, preserve) = balance.parts();
    balance_rows_at(level, &Balance32::new(values, preserve), src, out);
}

/// 色相/彩度の値を f32 にした物（行の核と画素ごとの式で同じ値を使う）。
#[derive(Clone, Copy)]
pub(super) struct HueSat32 {
    /// 色相の回転（`hue / 360`）
    shift: f32,
    /// 彩度の倍率（`1 + saturation`）
    scale: f32,
    /// 明度が 0 以上なら白へ寄せる割合（`lightness`）、負なら明度に掛ける倍率（`1 + lightness`）
    light: f32,
    brighten: bool,
}

impl HueSat32 {
    pub(super) fn new(hue: f64, saturation: f64, lightness: f64) -> Self {
        let brighten = lightness >= 0.0;
        HueSat32 {
            shift: (hue / 360.0) as f32,
            scale: (1.0 + saturation) as f32,
            light: if brighten {
                lightness as f32
            } else {
                (1.0 + lightness) as f32
            },
            brighten,
        }
    }
}

/// レーンで計算する調整の値。
#[derive(Clone, Copy)]
enum LaneKind {
    HueSaturation(HueSat32),
    ColorBalance(Balance32),
}

// ───────── 色相/彩度（f32） ─────────

#[inline(always)]
unsafe fn hue_to_rgb32<V: Lanes32>(p: V::F, q: V::F, t: V::F) -> V::F {
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let t = V::select(V::lt(t, zero), V::add(t, one), t);
    let t = V::select(V::gt(t, one), V::sub(t, one), t);
    let rising = V::add(p, V::mul(V::mul(V::sub(q, p), V::splat(6.0)), t));
    let falling = V::add(
        p,
        V::mul(
            V::mul(V::sub(q, p), V::sub(V::splat(2.0 / 3.0), t)),
            V::splat(6.0),
        ),
    );
    V::select(
        V::lt(t, V::splat(1.0 / 6.0)),
        rising,
        V::select(
            V::lt(t, V::splat(0.5)),
            q,
            V::select(V::lt(t, V::splat(2.0 / 3.0)), falling, p),
        ),
    )
}

/// 色相/彩度/明度: RGB → HSL（最大・最小の成分の差が 1e-12 以下なら色相・彩度 0）、色相を回し（0〜1 に折り返す）、彩度に倍率を
/// 掛けて 0〜1 に収め、明度は 0 以上なら白へ・負なら黒へ寄せ、HSL → RGB（彩度 0 なら灰色）。入力・出力は 0〜255 の整数のレーン。
#[inline(always)]
unsafe fn hue_saturation32<V: Lanes32>(h: &HueSat32, rgb: [V::F; 3]) -> [V::F; 3] {
    let (zero, one, half) = (V::splat(0.0), V::splat(1.0), V::splat(0.5));
    let (r, g, b) = (V::unit(rgb[0]), V::unit(rgb[1]), V::unit(rgb[2]));
    // RGB → HSL
    let mx = V::max(r, V::max(g, b));
    let mn = V::min(r, V::min(g, b));
    let l = V::div(V::add(mx, mn), V::splat(2.0));
    let d = V::sub(mx, mn);
    let chromatic = V::gt(d, V::splat(1e-12));
    let s_light = V::div(d, V::sub(V::sub(V::splat(2.0), mx), mn));
    let s_dark = V::div(d, V::add(mx, mn));
    let s = V::select(chromatic, V::select(V::gt(l, half), s_light, s_dark), zero);
    let h_red = V::add(
        V::div(V::sub(g, b), d),
        V::select(V::lt(g, b), V::splat(6.0), zero),
    );
    let h_green = V::add(V::div(V::sub(b, r), d), V::splat(2.0));
    let h_blue = V::add(V::div(V::sub(r, g), d), V::splat(4.0));
    let hue = V::select(
        V::eq(mx, r),
        h_red,
        V::select(V::eq(mx, g), h_green, h_blue),
    );
    let hue = V::select(chromatic, V::div(hue, V::splat(6.0)), zero);
    let hue = V::add(hue, V::splat(h.shift));
    let hue = V::sub(hue, V::floor(hue));
    let s = V::max(zero, V::min(one, V::mul(s, V::splat(h.scale))));
    let l = if h.brighten {
        V::add(l, V::mul(V::sub(one, l), V::splat(h.light)))
    } else {
        V::mul(l, V::splat(h.light))
    };
    // HSL → RGB
    let q = V::select(
        V::lt(l, half),
        V::mul(l, V::add(one, s)),
        V::sub(V::add(l, s), V::mul(l, s)),
    );
    let p = V::sub(V::mul(V::splat(2.0), l), q);
    let third = V::splat(1.0 / 3.0);
    let gray = V::le(s, zero);
    let red = V::select(gray, l, hue_to_rgb32::<V>(p, q, V::add(hue, third)));
    let green = V::select(gray, l, hue_to_rgb32::<V>(p, q, hue));
    let blue = V::select(gray, l, hue_to_rgb32::<V>(p, q, V::sub(hue, third)));
    [
        to_byte32::<V>(red),
        to_byte32::<V>(green),
        to_byte32::<V>(blue),
    ]
}

/// 1 画素の色相/彩度（`AdjustmentSettings::apply_in` の中身。行の核のスカラーの道と同じ関数）。
pub(super) fn hue_saturation_pixel(hue: f64, saturation: f64, lightness: f64, c: Rgba8) -> Rgba8 {
    let h = HueSat32::new(hue, saturation, lightness);
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    let rgb = unsafe {
        hue_saturation32::<Scalar1>(&h, [f32::from(c.r), f32::from(c.g), f32::from(c.b)])
    };
    Rgba8::new(rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, c.a)
}

#[inline(always)]
unsafe fn hue_rows32_lanes<V: Lanes32>(h: &HueSat32, src: &[u8], out: &mut [u8]) {
    let count = src.len() / 4;
    let mut i = 0;
    if V::N > 1 {
        while i + V::N <= count {
            let px = V::load(&src[i * 4..]);
            let rgb = hue_saturation32::<V>(h, [px[0], px[1], px[2]]);
            V::store(&mut out[i * 4..], [rgb[0], rgb[1], rgb[2], px[3]]);
            i += V::N;
        }
    }
    while i < count {
        let px = Scalar1::load(&src[i * 4..]);
        let rgb = hue_saturation32::<Scalar1>(h, [px[0], px[1], px[2]]);
        Scalar1::store(&mut out[i * 4..], [rgb[0], rgb[1], rgb[2], px[3]]);
        i += 1;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn hue_rows32_avx2(h: &HueSat32, src: &[u8], out: &mut [u8]) {
    hue_rows32_lanes::<Avx2x8>(h, src, out)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn hue_rows32_sse41(h: &HueSat32, src: &[u8], out: &mut [u8]) {
    hue_rows32_lanes::<Sse41x4>(h, src, out)
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn hue_rows32_neon(h: &HueSat32, src: &[u8], out: &mut [u8]) {
    hue_rows32_lanes::<Neonx4>(h, src, out)
}

/// RGBA の並び（src）に色相/彩度を当てて out へ（アルファはそのまま）。全部の画素を処理する。
fn hue_rows32_at(level: Level, h: &HueSat32, src: &[u8], out: &mut [u8]) {
    match level.min(simd::detect()) {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2 を持つ
        Level::Avx2 => unsafe { hue_rows32_avx2(h, src, out) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { hue_rows32_sse41(h, src, out) },
        #[cfg(target_arch = "aarch64")]
        // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
        Level::Neon => unsafe { hue_rows32_neon(h, src, out) },
        // SAFETY: 1 本のレーンは CPU の前提を持たない
        Level::Scalar => unsafe { hue_rows32_lanes::<Scalar1>(h, src, out) },
    }
}

// ───────── カラーバランス（f32） ─────────

/// カラーバランスの値を f32 にした物: 範囲（シャドウ・中間・ハイライト）ごと・成分ごとの値（−100〜100）と、輝度を保つか。
#[derive(Clone, Copy)]
pub(super) struct Balance32 {
    values: [[f32; 3]; 3],
    preserve: bool,
}

impl Balance32 {
    pub(super) fn new(values: &[[f64; 3]; 3], preserve: bool) -> Self {
        Balance32 {
            values: values.map(|r| r.map(|v| v as f32)),
            preserve,
        }
    }
}

/// 0〜1 の浮動小数の輝度（Rec.709。カラーバランスの重みに使う）。
#[inline(always)]
unsafe fn luminance32<V: Lanes32>(r: V::F, g: V::F, b: V::F) -> V::F {
    V::add(
        V::add(V::mul(V::splat(0.2126), r), V::mul(V::splat(0.7152), g)),
        V::mul(V::splat(0.0722), b),
    )
}

/// カラーバランス: 輝度 y（Rec.709）で範囲の重み (1 − y)²・4y(1 − y)・y² を作り、成分ごとに Σ 値 × 重み ÷ 100 × 0.3 を足す。
/// 輝度を保つなら、0〜1 に収めた色の輝度と元の輝度の差を 3 成分へ足し戻す（収める前に）。入力・出力は 0〜255 の整数のレーン。
#[inline(always)]
unsafe fn color_balance32<V: Lanes32>(b: &Balance32, rgb: [V::F; 3]) -> [V::F; 3] {
    let one = V::splat(1.0);
    let (r, g, bl) = (V::unit(rgb[0]), V::unit(rgb[1]), V::unit(rgb[2]));
    let y = luminance32::<V>(r, g, bl);
    let w = [
        V::mul(V::sub(one, y), V::sub(one, y)),
        V::mul(V::mul(V::splat(4.0), y), V::sub(one, y)),
        V::mul(y, y),
    ];
    let mut moved = [r, g, bl];
    for (k, c) in moved.iter_mut().enumerate() {
        let mut sum = V::mul(V::splat(b.values[0][k]), w[0]);
        for (range, weight) in w.iter().enumerate().skip(1) {
            sum = V::add(sum, V::mul(V::splat(b.values[range][k]), *weight));
        }
        let delta = V::mul(
            V::div(sum, V::splat(ColorBalance::RANGE as f32)),
            V::splat(ColorBalance::SCALE as f32),
        );
        *c = V::add(*c, delta);
    }
    if b.preserve {
        let back = V::sub(
            y,
            luminance32::<V>(
                clamp01_32::<V>(moved[0]),
                clamp01_32::<V>(moved[1]),
                clamp01_32::<V>(moved[2]),
            ),
        );
        for c in &mut moved {
            *c = V::add(*c, back);
        }
    }
    [
        to_byte32::<V>(clamp01_32::<V>(moved[0])),
        to_byte32::<V>(clamp01_32::<V>(moved[1])),
        to_byte32::<V>(clamp01_32::<V>(moved[2])),
    ]
}

/// 1 画素のカラーバランス（`ColorBalance::apply` の中身。行の核のスカラーの道と同じ関数）。
pub(super) fn color_balance_pixel(values: &[[f64; 3]; 3], preserve: bool, c: Rgba8) -> Rgba8 {
    let b = Balance32::new(values, preserve);
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    let rgb =
        unsafe { color_balance32::<Scalar1>(&b, [f32::from(c.r), f32::from(c.g), f32::from(c.b)]) };
    Rgba8::new(rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, c.a)
}

#[inline(always)]
unsafe fn balance_rows_lanes<V: Lanes32>(b: &Balance32, src: &[u8], out: &mut [u8]) {
    let count = src.len() / 4;
    let mut i = 0;
    if V::N > 1 {
        while i + V::N <= count {
            let px = V::load(&src[i * 4..]);
            let rgb = color_balance32::<V>(b, [px[0], px[1], px[2]]);
            V::store(&mut out[i * 4..], [rgb[0], rgb[1], rgb[2], px[3]]);
            i += V::N;
        }
    }
    while i < count {
        let px = Scalar1::load(&src[i * 4..]);
        let rgb = color_balance32::<Scalar1>(b, [px[0], px[1], px[2]]);
        Scalar1::store(&mut out[i * 4..], [rgb[0], rgb[1], rgb[2], px[3]]);
        i += 1;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn balance_rows_avx2(b: &Balance32, src: &[u8], out: &mut [u8]) {
    balance_rows_lanes::<Avx2x8>(b, src, out)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn balance_rows_sse41(b: &Balance32, src: &[u8], out: &mut [u8]) {
    balance_rows_lanes::<Sse41x4>(b, src, out)
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn balance_rows_neon(b: &Balance32, src: &[u8], out: &mut [u8]) {
    balance_rows_lanes::<Neonx4>(b, src, out)
}

/// RGBA の並び（src）にカラーバランスを当てて out へ（アルファはそのまま）。全部の画素を処理する。
pub(super) fn balance_rows_at(level: Level, b: &Balance32, src: &[u8], out: &mut [u8]) {
    match level.min(simd::detect()) {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2 を持つ
        Level::Avx2 => unsafe { balance_rows_avx2(b, src, out) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { balance_rows_sse41(b, src, out) },
        #[cfg(target_arch = "aarch64")]
        // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
        Level::Neon => unsafe { balance_rows_neon(b, src, out) },
        // SAFETY: 1 本のレーンは CPU の前提を持たない
        Level::Scalar => unsafe { balance_rows_lanes::<Scalar1>(b, src, out) },
    }
}

#[cfg(test)]
mod tests;
