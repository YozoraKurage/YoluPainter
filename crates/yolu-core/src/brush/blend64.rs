//! ブラシが描く画素の重ね（Normal）とフェードの f64 の式。画素ごとの関数（スカラーの道）と、行の核のレーン
//! （[`crate::math::simd::Lanes`]）の版が同じバイトを出す。
//!
//! 層の合成（[`crate::blend`]）は f32 の式に移ったが、ブラシの画素の計算（覆い・色・効果）は f64 のレーンのままなので、
//! ブラシが使う Normal とフェードだけをここに f64 で持つ。ブラシを f32 に移すときに、この式も合成の式へ寄せる。
#![cfg_attr(
    not(target_arch = "x86_64"),
    allow(dead_code, unused_imports, unused_macros, unused_variables, unused_mut)
)]

use crate::math::simd::{to_byte as lanes_to_byte, Lanes};
use crate::math::{to_byte, UNIT};
use crate::types::Rgba8;

/// これより小さい a·c は非正規化数になり得るので、下が透明のときの近道を使わない。
const MIN_SHORTCUT_ALPHA: f64 = 1e-300;

/// 下（destination）に上（source）を不透明度 opacity で Normal で重ねる。opacity は 0〜1。
#[inline]
pub(crate) fn blend(destination: Rgba8, source: Rgba8, opacity: f64) -> Rgba8 {
    // 不透明な画素を量 1 で重ねると、式の結果は上の画素そのもの
    if source.a == 255 && opacity == 1.0 {
        return source;
    }
    let sa = UNIT[source.a as usize] * opacity;
    let da = UNIT[destination.a as usize];
    if sa <= 0.0 {
        return destination;
    }
    let a = sa + da * (1.0 - sa);
    if a <= 0.0 {
        return Rgba8::TRANSPARENT;
    }
    let (dr, dg, db) = (
        UNIT[destination.r as usize],
        UNIT[destination.g as usize],
        UNIT[destination.b as usize],
    );
    let (sr, sg, sb) = (
        UNIT[source.r as usize],
        UNIT[source.g as usize],
        UNIT[source.b as usize],
    );
    let wd = (1.0 - sa) * da;
    let ws = (1.0 - da) * sa;
    let wb = da * sa;
    Rgba8::new(
        to_byte((wd * dr + ws * sr + wb * sr) / a),
        to_byte((wd * dg + ws * sg + wb * sg) / a),
        to_byte((wd * db + ws * sb + wb * sb) / a),
        to_byte(a),
    )
}

/// 下と中身を amount で混ぜる（プリマルチプライドの補間。透明な側がもう片方を暗くしない）。
#[inline]
pub(crate) fn fade(backdrop: Rgba8, inner: Rgba8, amount: f64) -> Rgba8 {
    if amount >= 1.0 {
        return inner;
    }
    if amount <= 0.0 {
        return backdrop;
    }
    let ba = UNIT[backdrop.a as usize] * (1.0 - amount);
    let ia = UNIT[inner.a as usize] * amount;
    let a = ba + ia;
    if a <= 0.0 {
        return Rgba8::TRANSPARENT;
    }
    Rgba8::new(
        to_byte((UNIT[backdrop.r as usize] * ba + UNIT[inner.r as usize] * ia) / a),
        to_byte((UNIT[backdrop.g as usize] * ba + UNIT[inner.g as usize] * ia) / a),
        to_byte((UNIT[backdrop.b as usize] * ba + UNIT[inner.b as usize] * ia) / a),
        to_byte(a),
    )
}

/// N 画素の Normal の重ね（下 dst に上 src を量 amount で）。レーンごとの結果は [`blend`] の 1 画素と同じバイト。何も変わらない組は None。
#[inline(always)]
pub(crate) unsafe fn blend_block<V: Lanes>(
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
    // 下が透明、または上が不透明・量 1 なら上の色をそのまま（アルファは量で掛けた値）。
    // 下が透明のときの重みは 0・a_s・0 で色は (a_s·c)/a_s。積が正規化数なら c から 2 ulp 以内で、丸めると c のバイト
    let mut copy = V::and(V::eq(d_a, zero), V::ge(sa, V::splat(MIN_SHORTCUT_ALPHA)));
    copy = V::or(
        copy,
        V::and(V::eq(s_a, V::splat(255.0)), V::eq(amount, one)),
    );
    copy = V::and(copy, V::not(skip));
    let alpha_of_copy = lanes_to_byte::<V>(sa);
    let compute = V::not(V::or(skip, copy));
    let mut out_rgb = [dst[0], dst[1], dst[2]];
    let out_a = if V::any(compute) {
        let d = [V::unit(dst[0]), V::unit(dst[1]), V::unit(dst[2])];
        let s = [V::unit(src[0]), V::unit(src[1]), V::unit(src[2])];
        // 下が不透明: a = a_s + (1 − a_s) はちょうど 1、重みは 1 − a_s・0・a_s（0 の項と ÷1 は値を変えない）
        let opaque = V::eq(d_a, V::splat(255.0));
        let t = V::sub(one, sa);
        let mut calc = [zero; 3];
        for c in 0..3 {
            calc[c] = V::add(V::mul(t, d[c]), V::mul(sa, s[c]));
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
                    V::add(V::add(V::mul(wd, d[c]), V::mul(ws, s[c])), V::mul(wb, s[c])),
                    a,
                );
                calc[c] = V::select(opaque, calc[c], general);
            }
            a_byte = V::select(opaque, a_byte, lanes_to_byte::<V>(a));
        }
        for c in 0..3 {
            out_rgb[c] = V::select(
                skip,
                dst[c],
                V::select(copy, src[c], lanes_to_byte::<V>(calc[c])),
            );
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

/// N 画素のフェード（下 backdrop と中身 inner をプリマルチプライドで補間）。レーンごとの結果は [`fade`] と同じバイト。
#[inline(always)]
pub(crate) unsafe fn fade_block<V: Lanes>(
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
        out[c] = V::select(V::le(a, zero), zero, lanes_to_byte::<V>(v));
    }
    out[3] = V::select(V::le(a, zero), zero, lanes_to_byte::<V>(a));
    // 量が 1 以上は中身そのもの、0 以下は下そのまま（スカラーの早い戻り。量 1 以上が先）
    let whole = V::ge(amount, one);
    let none = V::le(amount, zero);
    for c in 0..4 {
        out[c] = V::select(whole, inner[c], V::select(none, backdrop[c], out[c]));
    }
    out
}
