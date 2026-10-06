//! 合成モードの式（`blend_rgb`・`separable`）の、レーン（SIMD）の版。
//!
//! 演算の順・比較の向き・丸めの位置は、スカラーの式（`super::separable`・`super::blend_rgb`）の 1 つ 1 つに対応させてある（積和へまとめない、
//! `a * b * c` は `(a * b) * c`）。分岐は両方の側を計算して選ぶ。選ばれなかった側が 0 での割り算などで NaN・無限になっても結果に混ざらない。
//! モードはコンパイル時の定数（`MODE`）で、モードごとに 1 つの関数になる。
#![cfg_attr(
    not(target_arch = "x86_64"),
    allow(dead_code, unused_imports, unused_macros, unused_variables, unused_mut)
)]

use super::TIE_MARGIN;
use crate::math::simd::{clamp01, Lanes};
use crate::types::BlendMode;

macro_rules! mode_consts {
    ($($name:ident = $mode:ident),* $(,)?) => {
        $(pub(crate) const $name: u8 = BlendMode::$mode as u8;)*
    };
}
mode_consts! {
    NORMAL = Normal, MULTIPLY = Multiply, SCREEN = Screen, OVERLAY = Overlay, DARKEN = Darken,
    LIGHTEN = Lighten, COLOR_DODGE = ColorDodge, COLOR_BURN = ColorBurn, LINEAR_DODGE = LinearDodge,
    LINEAR_BURN = LinearBurn, HARD_LIGHT = HardLight, SOFT_LIGHT = SoftLight, VIVID_LIGHT = VividLight,
    LINEAR_LIGHT = LinearLight, PIN_LIGHT = PinLight, HARD_MIX = HardMix, DIFFERENCE = Difference,
    EXCLUSION = Exclusion, SUBTRACT = Subtract, DIVIDE = Divide, HUE = Hue, SATURATION = Saturation,
    COLOR = Color, LUMINOSITY = Luminosity, DARKER_COLOR = DarkerColor, LIGHTER_COLOR = LighterColor,
    PASS_THROUGH = PassThrough,
}

/// そのモードが「上の色そのもの」か（Normal・PassThrough）。
pub(crate) const fn is_simple(mode: u8) -> bool {
    mode == NORMAL || mode == PASS_THROUGH
}

/// R・G・B の 3 つのレーン。
pub(crate) type Rgb<V> = [<V as Lanes>::F; 3];

#[inline(always)]
unsafe fn dodge<V: Lanes>(d: V::F, s: V::F) -> V::F {
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let q = V::min(one, V::div(d, V::sub(one, s)));
    V::select(V::le(d, zero), zero, V::select(V::ge(s, one), one, q))
}

#[inline(always)]
unsafe fn burn<V: Lanes>(d: V::F, s: V::F) -> V::F {
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let q = V::sub(one, V::min(one, V::div(V::sub(one, d), s)));
    V::select(V::ge(d, one), one, V::select(V::le(s, zero), zero, q))
}

/// 分離できるモードの 1 成分（`super::separable` と同じ bit）。
#[inline(always)]
pub(crate) unsafe fn separable<V: Lanes, const MODE: u8>(d: V::F, s: V::F) -> V::F {
    let (zero, one, half, two) = (V::splat(0.0), V::splat(1.0), V::splat(0.5), V::splat(2.0));
    let v = match MODE {
        MULTIPLY => V::mul(d, s),
        SCREEN => V::sub(V::add(d, s), V::mul(d, s)),
        OVERLAY => {
            let lo = V::mul(V::mul(two, d), s);
            let hi = V::sub(one, V::mul(V::mul(two, V::sub(one, d)), V::sub(one, s)));
            V::select(V::le(d, half), lo, hi)
        }
        DARKEN => V::min(d, s),
        LIGHTEN => V::max(d, s),
        COLOR_DODGE => dodge::<V>(d, s),
        COLOR_BURN => burn::<V>(d, s),
        LINEAR_DODGE => V::add(d, s),
        LINEAR_BURN => V::sub(V::add(d, s), one),
        HARD_LIGHT => {
            let lo = V::mul(V::mul(two, d), s);
            let hi = V::sub(one, V::mul(V::mul(two, V::sub(one, d)), V::sub(one, s)));
            V::select(V::le(s, half), lo, hi)
        }
        SOFT_LIGHT => {
            let lo = V::sub(
                d,
                V::mul(V::mul(V::sub(one, V::mul(two, s)), d), V::sub(one, d)),
            );
            let hi = V::add(
                d,
                V::mul(V::sub(V::mul(two, s), one), V::sub(V::sqrt(d), d)),
            );
            V::select(V::le(s, half), lo, hi)
        }
        VIVID_LIGHT => {
            let lo = burn::<V>(d, V::mul(two, s));
            let hi = dodge::<V>(d, V::sub(V::mul(two, s), one));
            V::select(V::le(s, half), lo, hi)
        }
        LINEAR_LIGHT => V::sub(V::add(d, V::mul(two, s)), one),
        PIN_LIGHT => {
            let lo = V::min(d, V::mul(two, s));
            let hi = V::max(d, V::sub(V::mul(two, s), one));
            V::select(V::le(s, half), lo, hi)
        }
        HARD_MIX => V::select(
            V::ge(V::add(d, s), V::sub(one, V::splat(TIE_MARGIN))),
            one,
            zero,
        ),
        DIFFERENCE => V::abs(V::sub(d, s)),
        EXCLUSION => V::sub(V::add(d, s), V::mul(V::mul(two, d), s)),
        SUBTRACT => V::sub(d, s),
        DIVIDE => V::select(
            V::le(s, zero),
            V::select(V::le(d, zero), zero, one),
            V::div(d, s),
        ),
        _ => s,
    };
    clamp01::<V>(v)
}

#[inline(always)]
unsafe fn lum<V: Lanes>(c: Rgb<V>) -> V::F {
    V::add(
        V::add(V::mul(V::splat(0.3), c[0]), V::mul(V::splat(0.59), c[1])),
        V::mul(V::splat(0.11), c[2]),
    )
}

/// 成分の最大・最小（スカラーの `if g > b { g } else { b }` の順）。
#[inline(always)]
unsafe fn extremes<V: Lanes>(c: Rgb<V>) -> (V::F, V::F) {
    let mx = V::max(c[0], V::max(c[1], c[2]));
    let mn = V::min(c[0], V::min(c[1], c[2]));
    (mx, mn)
}

#[inline(always)]
unsafe fn sat<V: Lanes>(c: Rgb<V>) -> V::F {
    let (mx, mn) = extremes::<V>(c);
    V::sub(mx, mn)
}

/// W3C の SetLum の後に ClipColor と 0〜1 への切り詰め。
#[inline(always)]
unsafe fn set_lum<V: Lanes>(c: Rgb<V>, l: V::F) -> Rgb<V> {
    let (zero, one, eps) = (V::splat(0.0), V::splat(1.0), V::splat(1e-12));
    let delta = V::sub(l, lum::<V>(c));
    let mut k = [
        V::add(c[0], delta),
        V::add(c[1], delta),
        V::add(c[2], delta),
    ];
    let lm = lum::<V>(k);
    let (x, n) = extremes::<V>(k);
    // n・x は下の 2 つの補正の前の値のまま使う（スカラーも同じ）
    let low = V::and(V::lt(n, zero), V::gt(V::sub(lm, n), eps));
    for v in &mut k {
        let adjusted = V::add(lm, V::div(V::mul(V::sub(*v, lm), lm), V::sub(lm, n)));
        *v = V::select(low, adjusted, *v);
    }
    let high = V::and(V::gt(x, one), V::gt(V::sub(x, lm), eps));
    for v in &mut k {
        let adjusted = V::add(
            lm,
            V::div(V::mul(V::sub(*v, lm), V::sub(one, lm)), V::sub(x, lm)),
        );
        *v = V::select(high, adjusted, *v);
    }
    [clamp01::<V>(k[0]), clamp01::<V>(k[1]), clamp01::<V>(k[2])]
}

/// SetSat の 1 成分: 最大なら s、最小なら 0、中間は比を保つ。mx = mn（flat）の画素は 0。
#[inline(always)]
unsafe fn set_sat_channel<V: Lanes>(v: V::F, mn: V::F, mx: V::F, s: V::F, flat: V::M) -> V::F {
    let zero = V::splat(0.0);
    let middle = V::div(V::mul(V::sub(v, mn), s), V::sub(mx, mn));
    let r = V::select(V::eq(v, mx), s, V::select(V::eq(v, mn), zero, middle));
    V::select(flat, zero, r)
}

/// W3C の SetSat: 一番大きい成分を s、一番小さい成分を 0 に、中間は比を保つ。
#[inline(always)]
unsafe fn set_sat<V: Lanes>(c: Rgb<V>, s: V::F) -> Rgb<V> {
    let (mx, mn) = extremes::<V>(c);
    let flat = V::le(V::sub(mx, mn), V::splat(1e-12));
    [
        set_sat_channel::<V>(c[0], mn, mx, s, flat),
        set_sat_channel::<V>(c[1], mn, mx, s, flat),
        set_sat_channel::<V>(c[2], mn, mx, s, flat),
    ]
}

#[inline(always)]
unsafe fn sum3<V: Lanes>(c: Rgb<V>) -> V::F {
    V::add(V::add(c[0], c[1]), c[2])
}

/// モードの合成色 B(下, 上)。成分は 0〜1（`super::blend_rgb` と同じ bit）。
#[inline(always)]
pub(crate) unsafe fn blend_rgb<V: Lanes, const MODE: u8>(d: Rgb<V>, s: Rgb<V>) -> Rgb<V> {
    match MODE {
        NORMAL | PASS_THROUGH => s,
        HUE => set_lum::<V>(set_sat::<V>(s, sat::<V>(d)), lum::<V>(d)),
        SATURATION => set_lum::<V>(set_sat::<V>(d, sat::<V>(s)), lum::<V>(d)),
        COLOR => set_lum::<V>(s, lum::<V>(d)),
        LUMINOSITY => set_lum::<V>(d, lum::<V>(s)),
        DARKER_COLOR | LIGHTER_COLOR => {
            let tie = V::splat(TIE_MARGIN);
            let take_over = if MODE == DARKER_COLOR {
                V::lt(sum3::<V>(s), V::sub(sum3::<V>(d), tie))
            } else {
                V::gt(sum3::<V>(s), V::add(sum3::<V>(d), tie))
            };
            [
                V::select(take_over, s[0], d[0]),
                V::select(take_over, s[1], d[1]),
                V::select(take_over, s[2], d[2]),
            ]
        }
        _ => [
            separable::<V, MODE>(d[0], s[0]),
            separable::<V, MODE>(d[1], s[1]),
            separable::<V, MODE>(d[2], s[2]),
        ],
    }
}

/// モードを定数にして `$f::<$v, MODE>(...)` を呼ぶ。
macro_rules! dispatch_mode {
    ($mode:expr, $f:ident::<$v:ty>($($arg:expr),* $(,)?)) => {
        match $mode {
            $crate::types::BlendMode::Normal => $f::<$v, { $crate::blend::lanes::NORMAL }>($($arg),*),
            $crate::types::BlendMode::Multiply => $f::<$v, { $crate::blend::lanes::MULTIPLY }>($($arg),*),
            $crate::types::BlendMode::Screen => $f::<$v, { $crate::blend::lanes::SCREEN }>($($arg),*),
            $crate::types::BlendMode::Overlay => $f::<$v, { $crate::blend::lanes::OVERLAY }>($($arg),*),
            $crate::types::BlendMode::Darken => $f::<$v, { $crate::blend::lanes::DARKEN }>($($arg),*),
            $crate::types::BlendMode::Lighten => $f::<$v, { $crate::blend::lanes::LIGHTEN }>($($arg),*),
            $crate::types::BlendMode::ColorDodge => $f::<$v, { $crate::blend::lanes::COLOR_DODGE }>($($arg),*),
            $crate::types::BlendMode::ColorBurn => $f::<$v, { $crate::blend::lanes::COLOR_BURN }>($($arg),*),
            $crate::types::BlendMode::LinearDodge => $f::<$v, { $crate::blend::lanes::LINEAR_DODGE }>($($arg),*),
            $crate::types::BlendMode::LinearBurn => $f::<$v, { $crate::blend::lanes::LINEAR_BURN }>($($arg),*),
            $crate::types::BlendMode::HardLight => $f::<$v, { $crate::blend::lanes::HARD_LIGHT }>($($arg),*),
            $crate::types::BlendMode::SoftLight => $f::<$v, { $crate::blend::lanes::SOFT_LIGHT }>($($arg),*),
            $crate::types::BlendMode::VividLight => $f::<$v, { $crate::blend::lanes::VIVID_LIGHT }>($($arg),*),
            $crate::types::BlendMode::LinearLight => $f::<$v, { $crate::blend::lanes::LINEAR_LIGHT }>($($arg),*),
            $crate::types::BlendMode::PinLight => $f::<$v, { $crate::blend::lanes::PIN_LIGHT }>($($arg),*),
            $crate::types::BlendMode::HardMix => $f::<$v, { $crate::blend::lanes::HARD_MIX }>($($arg),*),
            $crate::types::BlendMode::Difference => $f::<$v, { $crate::blend::lanes::DIFFERENCE }>($($arg),*),
            $crate::types::BlendMode::Exclusion => $f::<$v, { $crate::blend::lanes::EXCLUSION }>($($arg),*),
            $crate::types::BlendMode::Subtract => $f::<$v, { $crate::blend::lanes::SUBTRACT }>($($arg),*),
            $crate::types::BlendMode::Divide => $f::<$v, { $crate::blend::lanes::DIVIDE }>($($arg),*),
            $crate::types::BlendMode::Hue => $f::<$v, { $crate::blend::lanes::HUE }>($($arg),*),
            $crate::types::BlendMode::Saturation => $f::<$v, { $crate::blend::lanes::SATURATION }>($($arg),*),
            $crate::types::BlendMode::Color => $f::<$v, { $crate::blend::lanes::COLOR }>($($arg),*),
            $crate::types::BlendMode::Luminosity => $f::<$v, { $crate::blend::lanes::LUMINOSITY }>($($arg),*),
            $crate::types::BlendMode::DarkerColor => $f::<$v, { $crate::blend::lanes::DARKER_COLOR }>($($arg),*),
            $crate::types::BlendMode::LighterColor => $f::<$v, { $crate::blend::lanes::LIGHTER_COLOR }>($($arg),*),
            $crate::types::BlendMode::PassThrough => $f::<$v, { $crate::blend::lanes::PASS_THROUGH }>($($arg),*),
        }
    };
}
pub(crate) use dispatch_mode;
