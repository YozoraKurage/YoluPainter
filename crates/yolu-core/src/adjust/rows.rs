//! 調整の層の行ごとの合成（[`AdjustKernel::composite_row`]）。各画素は [`AdjustKernel::composite`] と同じバイトになる。
//!
//! 2 段で処理する: (1) 調整した色を求める（値だけで決まる種類は表引き・整数の計算のまま、色相/彩度とカラーバランスは f64 の式を
//! レーンで）、(2) 下とモードと量で混ぜる（`blend` の行の核と同じレーンの式）。段の間は 256 画素ずつの小さい作業の領域で受け渡す。
#![cfg_attr(
    not(target_arch = "x86_64"),
    allow(dead_code, unused_imports, unused_macros, unused_variables, unused_mut)
)]

use super::{AdjustKernel, AdjustmentType, More};
use crate::blend::{mix_row_at, RowAmount};
use crate::math::simd::{self, clamp01, to_byte, Lanes, Level};
use crate::types::{BlendMode, ChannelKind};

#[cfg(target_arch = "x86_64")]
use crate::math::simd::{Avx2, Sse41};

/// 段の間に受け渡す画素数。
const CHUNK: usize = 256;

impl AdjustKernel {
    /// 1 行（RGBA）に調整の層を重ねる。量が 0 以下・下が完全に透明な画素はそのまま。
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

    /// 調整後の色（RGBA、アルファは下のまま）を out へ。端数の画素や SIMD の無い環境は 1 画素ずつの式。
    fn adjust_chunk(&self, level: Level, src: &[u8], out: &mut [u8]) {
        let from = match (level, self.lane_kind()) {
            #[cfg(target_arch = "x86_64")]
            // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
            (Level::Avx2, Some(kind)) => unsafe { adjust_lanes_avx2(kind, src, out) },
            #[cfg(target_arch = "x86_64")]
            // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
            (Level::Sse41, Some(kind)) => unsafe { adjust_lanes_sse41(kind, src, out) },
            _ => 0,
        };
        self.adjust_from_scalar(src, out, from);
    }

    /// 画素 `from` から 1 画素ずつ（[`AdjustKernel::composite`] の `adjusted`）。
    fn adjust_from_scalar(&self, src: &[u8], out: &mut [u8], from: usize) {
        use crate::types::Rgba8;
        for i in from..src.len() / 4 {
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

    /// レーンで計算する種類（f64 の式が重いもの）。
    fn lane_kind(&self) -> Option<LaneKind> {
        if self.channel != ChannelKind::Color {
            return None; // 色相/彩度・カラーバランスは色のチャンネルだけ（ほかでは層が適用されない）
        }
        match (&self.settings.kind, &self.settings.more) {
            (AdjustmentType::HueSaturation, _) => Some(LaneKind::HueSaturation {
                shift: self.settings.hue / 360.0,
                saturation: self.settings.saturation,
                lightness: self.settings.lightness,
            }),
            (_, More::ColorBalance(c)) if !c.is_neutral() => {
                let (values, preserve) = c.parts();
                Some(LaneKind::ColorBalance {
                    values: *values,
                    preserve,
                })
            }
            _ => None,
        }
    }
}

/// カラーバランスを RGBA の並び（src）へ当てた結果を out へ（各画素は `ColorBalance::apply` と同じバイト、アルファはそのまま）。
/// フィルターの段が使う。
pub(crate) fn color_balance_rgba(
    level: Level,
    balance: &crate::adjust::ColorBalance,
    src: &[u8],
    out: &mut [u8],
) {
    use crate::types::Rgba8;
    let kind = if balance.is_neutral() {
        None
    } else {
        let (values, preserve) = balance.parts();
        Some(LaneKind::ColorBalance {
            values: *values,
            preserve,
        })
    };
    let from = match (level.min(simd::detect()), kind) {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
        (Level::Avx2, Some(kind)) => unsafe { adjust_lanes_avx2(kind, src, out) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        (Level::Sse41, Some(kind)) => unsafe { adjust_lanes_sse41(kind, src, out) },
        _ => 0,
    };
    for i in from..src.len() / 4 {
        let v = balance.apply(Rgba8::from_slice(&src[i * 4..]));
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_array());
    }
}

/// レーンで計算する調整の値。
#[derive(Clone, Copy)]
enum LaneKind {
    HueSaturation {
        /// 色相の回転（`hue / 360`）
        shift: f64,
        saturation: f64,
        lightness: f64,
    },
    ColorBalance {
        values: [[f64; 3]; 3],
        preserve: bool,
    },
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn adjust_lanes_avx2(kind: LaneKind, src: &[u8], out: &mut [u8]) -> usize {
    adjust_lanes::<Avx2>(kind, src, out)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn adjust_lanes_sse41(kind: LaneKind, src: &[u8], out: &mut [u8]) -> usize {
    adjust_lanes::<Sse41>(kind, src, out)
}

/// N 画素ずつ調整して out へ。処理した画素数（N の倍数）を返す。
#[inline(always)]
unsafe fn adjust_lanes<V: Lanes>(kind: LaneKind, src: &[u8], out: &mut [u8]) -> usize {
    let count = src.len() / 4;
    let mut i = 0;
    while i + V::N <= count {
        let px = V::load(&src[i * 4..]);
        let rgb = match kind {
            LaneKind::HueSaturation {
                shift,
                saturation,
                lightness,
            } => hue_saturation::<V>(shift, saturation, lightness, [px[0], px[1], px[2]]),
            LaneKind::ColorBalance { values, preserve } => {
                color_balance::<V>(&values, preserve, [px[0], px[1], px[2]])
            }
        };
        V::store(&mut out[i * 4..], [rgb[0], rgb[1], rgb[2], px[3]]);
        i += V::N;
    }
    i
}

#[inline(always)]
unsafe fn hue_to_rgb<V: Lanes>(p: V::F, q: V::F, t: V::F) -> V::F {
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

/// 色相/彩度/明度（`AdjustmentSettings::hue_saturation_lightness` と同じ式）。入力は 0〜255 の整数のレーン、出力は 0〜255 の整数のレーン。
#[inline(always)]
unsafe fn hue_saturation<V: Lanes>(
    shift: f64,
    saturation: f64,
    lightness: f64,
    rgb: [V::F; 3],
) -> [V::F; 3] {
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
    let h = V::select(
        V::eq(mx, r),
        h_red,
        V::select(V::eq(mx, g), h_green, h_blue),
    );
    let h = V::select(chromatic, V::div(h, V::splat(6.0)), zero);
    let h = V::add(h, V::splat(shift));
    let h = V::sub(h, V::floor(h));
    let s = V::max(zero, V::min(one, V::mul(s, V::splat(1.0 + saturation))));
    let l = if lightness >= 0.0 {
        V::add(l, V::mul(V::sub(one, l), V::splat(lightness)))
    } else {
        V::mul(l, V::splat(1.0 + lightness))
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
    let red = V::select(gray, l, hue_to_rgb::<V>(p, q, V::add(h, third)));
    let green = V::select(gray, l, hue_to_rgb::<V>(p, q, h));
    let blue = V::select(gray, l, hue_to_rgb::<V>(p, q, V::sub(h, third)));
    [to_byte::<V>(red), to_byte::<V>(green), to_byte::<V>(blue)]
}

#[inline(always)]
unsafe fn luminance_unit<V: Lanes>(r: V::F, g: V::F, b: V::F) -> V::F {
    V::add(
        V::add(V::mul(V::splat(0.2126), r), V::mul(V::splat(0.7152), g)),
        V::mul(V::splat(0.0722), b),
    )
}

/// カラーバランス（`ColorBalance::apply` と同じ式）。入力・出力は 0〜255 の整数のレーン。
#[inline(always)]
unsafe fn color_balance<V: Lanes>(
    values: &[[f64; 3]; 3],
    preserve: bool,
    rgb: [V::F; 3],
) -> [V::F; 3] {
    let one = V::splat(1.0);
    let (r, g, b) = (V::unit(rgb[0]), V::unit(rgb[1]), V::unit(rgb[2]));
    let y = luminance_unit::<V>(r, g, b);
    let w = [
        V::mul(V::sub(one, y), V::sub(one, y)),
        V::mul(V::mul(V::splat(4.0), y), V::sub(one, y)),
        V::mul(y, y),
    ];
    let mut moved = [r, g, b];
    for (k, c) in moved.iter_mut().enumerate() {
        // `(0..3).map(..).sum::<f64>()` は -0.0 から足し始める
        let mut sum = V::splat(-0.0);
        for (range, weight) in w.iter().enumerate() {
            sum = V::add(sum, V::mul(V::splat(values[range][k]), *weight));
        }
        let delta = V::mul(V::div(sum, V::splat(100.0)), V::splat(0.3));
        *c = V::add(*c, delta);
    }
    if preserve {
        let back = V::sub(
            y,
            luminance_unit::<V>(
                clamp01::<V>(moved[0]),
                clamp01::<V>(moved[1]),
                clamp01::<V>(moved[2]),
            ),
        );
        for c in &mut moved {
            *c = V::add(*c, back);
        }
    }
    [
        to_byte::<V>(clamp01::<V>(moved[0])),
        to_byte::<V>(clamp01::<V>(moved[1])),
        to_byte::<V>(clamp01::<V>(moved[2])),
    ]
}

#[cfg(test)]
mod tests;
