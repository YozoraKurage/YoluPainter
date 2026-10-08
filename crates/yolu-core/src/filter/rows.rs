//! フィルターの画素の計算の行の核（SIMD）。各画素・各値は、`pixels.rs` と `mod.rs` の画素ごとの式と**同じバイト**になる。
//!
//! 道は AVX2（4 画素）・SSE4.1 と NEON（2 画素）・スカラー（今までの式）で、`crate::math::simd` の土台で選ぶ。割り算を使う丸め
//! （箱ぼかしの平均・乗算済みから戻す除算）は、f64 の演算が厳密に整数の商の切り捨てと一致する範囲（分子が 2²⁶ 程度まで、商が 2¹⁷ 未満、
//! 商の端からの距離が f64 の丸めの誤差より十分大きい: 箱ぼかしは 0.5/(2r+1) 以上、乗算済みから戻す除算は 1/(2·qa) 以上）で行うので、
//! 整数の除算と同じ値になる（根拠は各関数の注釈と試験）。
#![cfg_attr(
    not(any(target_arch = "x86_64", target_arch = "aarch64")),
    allow(dead_code, unused_imports, unused_macros, unused_variables, unused_mut)
)]

use super::pixels::{hash, lerp, Check};
use super::{area, zeros, Error, Rect, Settings, ValueType};
use crate::math::simd::{self, to_byte, Lanes, Level};

#[cfg(target_arch = "aarch64")]
use crate::math::simd::Neon;
#[cfg(target_arch = "x86_64")]
use crate::math::simd::{Avx2, Sse41};

/// `lerp` のレーン版（結果は 0〜255 の整数）。`as u8` の飽和（NaN は 0）は max・min で作る。
#[inline(always)]
unsafe fn lerp_lanes<V: Lanes>(a: V::F, b: V::F, t: V::F) -> V::F {
    let v = V::floor(V::add(V::add(a, V::mul(V::sub(b, a), t)), V::splat(0.5)));
    V::min(V::max(v, V::splat(0.0)), V::splat(255.0))
}

/// 道ごとの入口を作る: `$lanes::<V>(...)` を、AVX2 と FMA・SSE4.1・NEON を有効にした関数の中で呼び、処理した画素数を返す。
macro_rules! entries {
    ($avx2:ident, $sse41:ident, $neon:ident, $lanes:ident ( $($p:ident : $t:ty),* $(,)? ) $(,)?) => {
        #[cfg(target_arch = "x86_64")]
        #[target_feature(enable = "avx2,fma")]
        unsafe fn $avx2($($p: $t),*) -> usize {
            $lanes::<Avx2>($($p),*)
        }
        #[cfg(target_arch = "x86_64")]
        #[target_feature(enable = "sse4.1")]
        unsafe fn $sse41($($p: $t),*) -> usize {
            $lanes::<Sse41>($($p),*)
        }
        #[cfg(target_arch = "aarch64")]
        #[target_feature(enable = "neon")]
        unsafe fn $neon($($p: $t),*) -> usize {
            $lanes::<Neon>($($p),*)
        }
    };
}

/// 道を選んで入口を呼ぶ（スカラーは 0 画素処理で戻る）。
macro_rules! run {
    ($level:expr, $avx2:ident, $sse41:ident, $neon:ident ( $($a:expr),* )) => {
        match $level.min(simd::detect()) {
            #[cfg(target_arch = "x86_64")]
            // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
            Level::Avx2 => unsafe { $avx2($($a),*) },
            #[cfg(target_arch = "x86_64")]
            // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
            Level::Sse41 => unsafe { $sse41($($a),*) },
            #[cfg(target_arch = "aarch64")]
            // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
            Level::Neon => unsafe { $neon($($a),*) },
            Level::Scalar => 0,
        }
    };
}

// ───────── 元の色と段の結果の混ぜ（強さ） ─────────

#[inline(always)]
unsafe fn lerp_rows_lanes<V: Lanes>(row: &mut [u8], target: &[u8], t: f64) -> usize {
    let count = row.len() / 4;
    let t = V::splat(t);
    let mut i = 0;
    while i + V::N <= count {
        let a = V::load(&row[i * 4..]);
        let b = V::load(&target[i * 4..]);
        let out = [
            lerp_lanes::<V>(a[0], b[0], t),
            lerp_lanes::<V>(a[1], b[1], t),
            lerp_lanes::<V>(a[2], b[2], t),
            a[3],
        ];
        V::store(&mut row[i * 4..], out);
        i += V::N;
    }
    i
}
entries!(
    lerp_rows_avx2,
    lerp_rows_sse41,
    lerp_rows_neon,
    lerp_rows_lanes(row: &mut [u8], target: &[u8], t: f64)
);

/// 行の RGB を、段の結果（target、同じ並びの RGBA）へ強さ t で混ぜる（`lerp`。アルファは変えない）。
pub(super) fn lerp_rows_at(level: Level, row: &mut [u8], target: &[u8], t: f64) {
    if t >= 1.0 {
        // lerp(a, b, 1) = floor(a + (b − a) + 0.5) = b
        for (p, q) in row.chunks_exact_mut(4).zip(target.chunks_exact(4)) {
            p[..3].copy_from_slice(&q[..3]);
        }
        return;
    }
    let from = run!(
        level,
        lerp_rows_avx2,
        lerp_rows_sse41,
        lerp_rows_neon(row, target, t)
    );
    for i in from..row.len() / 4 {
        for c in 0..3 {
            row[i * 4 + c] = lerp(row[i * 4 + c], target[i * 4 + c], t);
        }
    }
}

/// 値だけで決まる段（反転・レベル・Normalize の 256 の表）: 行の RGB を `lerp(v, lut[v], t)` にする。
#[inline(always)]
unsafe fn lut_row_lanes<V: Lanes>(row: &mut [u8], lut: &[u8; 256], t: f64) -> usize {
    let count = row.len() / 4;
    let t = V::splat(t);
    let mut i = 0;
    while i + V::N <= count {
        let a = V::load(&row[i * 4..]);
        let mut out = a;
        for c in 0..3 {
            let b = V::from_fn(|k| f64::from(lut[row[(i + k) * 4 + c] as usize]));
            out[c] = lerp_lanes::<V>(a[c], b, t);
        }
        V::store(&mut row[i * 4..], out);
        i += V::N;
    }
    i
}
entries!(
    lut_row_avx2,
    lut_row_sse41,
    lut_row_neon,
    lut_row_lanes(row: &mut [u8], lut: &[u8; 256], t: f64)
);

/// 行の RGB を `lerp(v, lut[v], t)` にする。
pub(super) fn lut_row_at(level: Level, row: &mut [u8], lut: &[u8; 256], t: f64) {
    if t >= 1.0 {
        for p in row.chunks_exact_mut(4) {
            for v in &mut p[..3] {
                *v = lut[*v as usize];
            }
        }
        return;
    }
    let from = run!(
        level,
        lut_row_avx2,
        lut_row_sse41,
        lut_row_neon(row, lut, t)
    );
    for p in row[from * 4..].chunks_exact_mut(4) {
        for v in &mut p[..3] {
            *v = lerp(*v, lut[*v as usize], t);
        }
    }
}

// ───────── ノイズ ─────────

/// 画素 (x, y) の乱数の基（`pixels::hash` を 3 回）。
#[inline(always)]
fn noise_seed(seed: i32, x: u32, y: u32) -> u32 {
    hash(hash(hash(seed as u32) ^ x) ^ y)
}

/// チャンネル c の乱数を −1〜1 の値に（スカラーの式の `u`）。
#[inline(always)]
fn noise_unit(h: u32, channel_key: u32) -> f64 {
    f64::from(hash(h ^ channel_key) >> 8) / 16777215.0 * 2.0 - 1.0
}

/// ノイズの 1 画素のチャンネルの結果（スカラー）。
#[inline(always)]
fn noise_channel(v: u8, amount: f64, u: f64, t: f64) -> u8 {
    let n = (f64::from(v) + amount * 127.5 * u + 0.5)
        .floor()
        .clamp(0.0, 255.0) as u8;
    lerp(v, n, t)
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
unsafe fn noise_row_lanes<V: Lanes>(
    row: &mut [u8],
    x0: u32,
    y: u32,
    seed: i32,
    amount: f64,
    monochrome: bool,
    t: f64,
) -> usize {
    let count = row.len() / 4;
    let (tv, scale) = (V::splat(t), V::splat(amount * 127.5));
    let half = V::splat(0.5);
    let mut i = 0;
    while i + V::N <= count {
        let a = V::load(&row[i * 4..]);
        // 乱数は整数の計算なので画素ごとに。値を f64 のレーンへ集めてから式を計算する
        let mut bases = [0u32; 4];
        for (k, b) in bases.iter_mut().enumerate().take(V::N) {
            *b = noise_seed(seed, x0.wrapping_add((i + k) as u32), y);
        }
        let mut out = a;
        for c in 0..3 {
            let key = if monochrome { 0 } else { c as u32 };
            let raw = V::from_fn(|k| f64::from(hash(bases[k] ^ key) >> 8));
            let u = V::sub(
                V::mul(V::div(raw, V::splat(16777215.0)), V::splat(2.0)),
                V::splat(1.0),
            );
            let n = V::floor(V::add(V::add(a[c], V::mul(scale, u)), half));
            let n = V::min(V::max(n, V::splat(0.0)), V::splat(255.0));
            out[c] = lerp_lanes::<V>(a[c], n, tv);
        }
        V::store(&mut row[i * 4..], out);
        i += V::N;
    }
    i
}
entries!(
    noise_row_avx2,
    noise_row_sse41,
    noise_row_neon,
    noise_row_lanes(
        row: &mut [u8],
        x0: u32,
        y: u32,
        seed: i32,
        amount: f64,
        monochrome: bool,
        t: f64,
    )
);

/// 行（左端のキャンバス座標 x0、行 y）にノイズを足す。
#[allow(clippy::too_many_arguments)]
pub(super) fn noise_row_at(
    level: Level,
    row: &mut [u8],
    x0: u32,
    y: u32,
    seed: i32,
    amount: f64,
    monochrome: bool,
    t: f64,
) {
    let from = run!(
        level,
        noise_row_avx2,
        noise_row_sse41,
        noise_row_neon(row, x0, y, seed, amount, monochrome, t)
    );
    for (x, p) in row.chunks_exact_mut(4).enumerate().skip(from) {
        let h = noise_seed(seed, x0.wrapping_add(x as u32), y);
        for (c, v) in p[..3].iter_mut().enumerate() {
            let u = noise_unit(h, if monochrome { 0 } else { c as u32 });
            *v = noise_channel(*v, amount, u, t);
        }
    }
}

// ───────── 乗算済みへの変換（ぼかしの前） ─────────

#[inline(always)]
unsafe fn premultiply_lanes<V: Lanes>(src: &[u8], out: &mut [u16]) -> usize {
    let count = src.len() / 4;
    let mut i = 0;
    while i + V::N <= count {
        let px = V::load(&src[i * 4..]);
        let a = px[3];
        V::store_u16x4(
            &mut out[i * 4..],
            [
                V::mul(px[0], a),
                V::mul(px[1], a),
                V::mul(px[2], a),
                V::mul(a, V::splat(255.0)),
            ],
        );
        i += V::N;
    }
    i
}
entries!(
    premultiply_avx2,
    premultiply_sse41,
    premultiply_neon,
    premultiply_lanes(src: &[u8], out: &mut [u16])
);

/// 画素（straight RGBA）を乗算済み（R・G・B に A を掛け、A は A × 255。u16 × 4）にする。
pub(super) fn premultiply_at(level: Level, src: &[u8], out: &mut [u16]) {
    let from = run!(
        level,
        premultiply_avx2,
        premultiply_sse41,
        premultiply_neon(src, out)
    );
    for (p, b) in out[from * 4..]
        .chunks_exact_mut(4)
        .zip(src[from * 4..].chunks_exact(4))
    {
        let a = u16::from(b[3]);
        for c in 0..3 {
            p[c] = u16::from(b[c]) * a;
        }
        p[3] = a * 255;
    }
}

// ───────── ぼかし・シャープの最後の 1 行 ─────────

/// 乗算済みの値（q: u16 × 4、並びは出力の矩形と同じ）から straight へ戻した色の 1 画素（ぼかし、接空間法線でない型）。
#[inline(always)]
fn unpremultiply_pixel(input: [u8; 4], q: &[u16]) -> [u8; 4] {
    let qa = u32::from(q[3]);
    let mut f = input;
    let a = ((qa + 127) / 255) as u8;
    if a == 0 {
        f[3] = 0;
    } else {
        for c in 0..3 {
            f[c] = ((2 * u32::from(q[c]) * 255 + qa) / (2 * qa)).min(255) as u8;
        }
        f[3] = a;
    }
    f
}

/// ぼかしの 1 画素の結果（強さ t での混ぜを含む）。
pub(super) fn blur_pixel(input: [u8; 4], q: &[u16], t: f64) -> [u8; 4] {
    super::pixels::mix(input, unpremultiply_pixel(input, q), t, false)
}

/// シャープの 1 画素の結果（強さ t での混ぜを含む）。
pub(super) fn sharpen_pixel(
    input: [u8; 4],
    q: &[u16],
    amount: f64,
    threshold: u32,
    t: f64,
) -> [u8; 4] {
    let qa = u32::from(q[3]);
    let mut f = input;
    if input[3] != 0 && qa != 0 {
        for c in 0..3 {
            let diff = f64::from(input[c]) - f64::from(q[c]) * 255.0 / f64::from(qa);
            if diff.abs() >= f64::from(threshold) {
                f[c] = (f64::from(input[c]) + amount * diff + 0.5)
                    .floor()
                    .clamp(0.0, 255.0) as u8;
            }
        }
    }
    for c in 0..3 {
        f[c] = lerp(input[c], f[c], t);
    }
    f
}

/// `pixels::mix`（接空間法線でない）のレーン版。0 < t < 1。
#[inline(always)]
unsafe fn mix_lanes<V: Lanes>(input: [V::F; 4], output: [V::F; 4], t: f64) -> [V::F; 4] {
    let zero = V::splat(0.0);
    let tv = V::splat(t);
    let same_alpha = V::eq(input[3], output[3]);
    let ba = V::mul(V::unit(input[3]), V::splat(1.0 - t));
    let ia = V::mul(V::unit(output[3]), tv);
    let a = V::add(ba, ia);
    let nothing = V::le(a, zero);
    let mut out = input;
    for c in 0..3 {
        let blended = to_byte::<V>(V::div(
            V::add(
                V::mul(V::unit(input[c]), ba),
                V::mul(V::unit(output[c]), ia),
            ),
            a,
        ));
        out[c] = V::select(
            same_alpha,
            lerp_lanes::<V>(input[c], output[c], tv),
            V::select(nothing, zero, blended),
        );
    }
    out[3] = V::select(
        same_alpha,
        input[3],
        V::select(nothing, zero, to_byte::<V>(a)),
    );
    out
}

#[inline(always)]
unsafe fn blur_finish_lanes<V: Lanes>(input: &[u8], q: &[u16], out: &mut [u8], t: f64) -> usize {
    let count = input.len() / 4;
    let zero = V::splat(0.0);
    let mut i = 0;
    while i + V::N <= count {
        let inp = V::load(&input[i * 4..]);
        let qv = V::load_u16x4(&q[i * 4..]);
        let qa = qv[3];
        // a = (qa + 127) / 255（整数の商。余りが 0 なら割り算が厳密、0 でなければ商の端から 1/255 以上離れるので f64 の割り算と切り捨てで同じ）
        let a = V::floor(V::div(V::add(qa, V::splat(127.0)), V::splat(255.0)));
        let transparent = V::eq(a, zero);
        let d2 = V::mul(V::splat(2.0), qa);
        let mut f = [inp[0], inp[1], inp[2], a];
        for c in 0..3 {
            // (2·q·255 + qa) / (2·qa) の商（分子は 3.4 千万未満。余りが 0 なら割り算が厳密、0 でなければ商の端から 1/(2·qa) 以上
            // （qa ≤ 65025 で 1/130050）離れ、f64 の丸めの誤差（商 ≤ 256 で 3e-14 以下）よりずっと大きいので同じ）
            let n = V::add(V::mul(V::mul(V::splat(2.0), qv[c]), V::splat(255.0)), qa);
            let v = V::min(V::floor(V::div(n, d2)), V::splat(255.0));
            f[c] = V::select(transparent, inp[c], v);
        }
        let res = if t >= 1.0 {
            f
        } else if t <= 0.0 {
            inp
        } else {
            mix_lanes::<V>(inp, f, t)
        };
        V::store(&mut out[i * 4..], res);
        i += V::N;
    }
    i
}
entries!(
    blur_finish_avx2,
    blur_finish_sse41,
    blur_finish_neon,
    blur_finish_lanes(input: &[u8], q: &[u16], out: &mut [u8], t: f64)
);

#[inline(always)]
#[allow(clippy::too_many_arguments)]
unsafe fn sharpen_finish_lanes<V: Lanes>(
    input: &[u8],
    q: &[u16],
    out: &mut [u8],
    amount: f64,
    threshold: u32,
    t: f64,
) -> usize {
    let count = input.len() / 4;
    let zero = V::splat(0.0);
    let (amount, threshold, tv) = (
        V::splat(amount),
        V::splat(f64::from(threshold)),
        V::splat(t),
    );
    let mut i = 0;
    while i + V::N <= count {
        let inp = V::load(&input[i * 4..]);
        let qv = V::load_u16x4(&q[i * 4..]);
        let qa = qv[3];
        let valid = V::and(V::not(V::eq(inp[3], zero)), V::not(V::eq(qa, zero)));
        let mut res = inp;
        for c in 0..3 {
            let blurred = V::div(V::mul(qv[c], V::splat(255.0)), qa);
            let diff = V::sub(inp[c], blurred);
            let hit = V::and(valid, V::ge(V::abs(diff), threshold));
            let sharp = V::floor(V::add(V::add(inp[c], V::mul(amount, diff)), V::splat(0.5)));
            let sharp = V::min(V::max(sharp, zero), V::splat(255.0));
            let f = V::select(hit, sharp, inp[c]);
            res[c] = lerp_lanes::<V>(inp[c], f, tv);
        }
        V::store(&mut out[i * 4..], res);
        i += V::N;
    }
    i
}
entries!(
    sharpen_finish_avx2,
    sharpen_finish_sse41,
    sharpen_finish_neon,
    sharpen_finish_lanes(
        input: &[u8],
        q: &[u16],
        out: &mut [u8],
        amount: f64,
        threshold: u32,
        t: f64,
    )
);

/// ぼかし・シャープの出力の 1 行。input は入力の同じ位置の行（画素の並びは q と同じ）、q は乗算済みの u16 × 4。
/// この行をここで処理したなら true、1 画素ずつの道が要るなら（接空間法線のぼかしは再正規化がある）false。
pub(super) fn finish_row_at(
    level: Level,
    settings: &Settings,
    value_type: ValueType,
    strength: f64,
    input: &[u8],
    q: &[u16],
    out: &mut [u8],
) -> bool {
    let count = input.len() / 4;
    let from = match *settings {
        Settings::Sharpen {
            amount, threshold, ..
        } => {
            let from = run!(
                level,
                sharpen_finish_avx2,
                sharpen_finish_sse41,
                sharpen_finish_neon(input, q, out, amount, threshold, strength)
            );
            for i in from..count {
                let px = sharpen_pixel(
                    input[i * 4..i * 4 + 4].try_into().unwrap(),
                    &q[i * 4..],
                    amount,
                    threshold,
                    strength,
                );
                out[i * 4..i * 4 + 4].copy_from_slice(&px);
            }
            return true;
        }
        _ if value_type == ValueType::TangentNormal => return false,
        _ => run!(
            level,
            blur_finish_avx2,
            blur_finish_sse41,
            blur_finish_neon(input, q, out, strength)
        ),
    };
    for i in from..count {
        let px = blur_pixel(
            input[i * 4..i * 4 + 4].try_into().unwrap(),
            &q[i * 4..],
            strength,
        );
        out[i * 4..i * 4 + 4].copy_from_slice(&px);
    }
    true
}

// ───────── 箱ぼかし ─────────

fn edge(v: i64, n: u32) -> u32 {
    v.clamp(0, i64::from(n) - 1) as u32
}

/// 箱ぼかし 1 回（横・縦）。平均は整数の商 `(合計 + r) / (2r + 1)`（割る数 D = 2r + 1）で、合計は 2²⁶ 未満。
/// 合計 + r を D で割った余りを k（0〜D−1）とすると `(合計 + r + 0.5) / D = 商 + (k + 0.5) / D` なので、商の端（前後の整数）からの距離は
/// 0.5/D 以上（D = 513 で 1/1026）。`floor((合計 + r + 0.5) × (1/(2r+1)))` の f64 の丸めの誤差（1.5e-11 以下）はこの余白よりずっと小さいので、
/// 整数の商と同じ値になる（元の実装の `(合計 + r) × inv >> 40` も同じ商。試験で全部の半径・端の値を確かめる）。
/// 評価した範囲は半径 r ≤ 256（D ≤ 513）。実際の 1 回の半径は、GaussianBlur の上限 256 を 3 回に分けた `ceil(256/3) = 86` 以下で D は 173 以下。
#[inline(always)]
unsafe fn box_blur_lanes<V: Lanes>(
    src: &[u16],
    a: Rect,
    b: Rect,
    r: u32,
    w: u32,
    h: u32,
    check: Check<'_>,
) -> Result<Vec<u16>, Error> {
    let aw = a.width as usize;
    let bw = b.width as usize;
    let mut mid = zeros::<u16>(a.height as usize * bw * 4)?;
    let mut dst = zeros::<u16>(area(b) * 4)?;
    let rcp = V::splat(1.0 / (f64::from(2 * r) + 1.0));
    let bias = V::splat(f64::from(r) + 0.5);
    // ウィンドウの出入りの画素の位置（端でクランプ済み）は行によらないので先に表にする
    let init: Vec<usize> = (-i64::from(r)..=i64::from(r))
        .map(|d| (edge(i64::from(b.x) + d, w) - a.x) as usize)
        .collect();
    let leave: Vec<usize> = (0..bw.saturating_sub(1))
        .map(|x| (edge(i64::from(b.x) + x as i64 - i64::from(r), w) - a.x) as usize)
        .collect();
    let enter: Vec<usize> = (0..bw.saturating_sub(1))
        .map(|x| (edge(i64::from(b.x) + x as i64 + i64::from(r) + 1, w) - a.x) as usize)
        .collect();
    for y in 0..a.height as usize {
        check()?;
        let src_row = y * aw * 4;
        let mid_row = y * bw * 4;
        for g in (0..4).step_by(V::N) {
            let mut sum = V::splat(0.0);
            for &ix in &init {
                sum = V::add(sum, V::load_u16(&src[src_row + ix * 4 + g..]));
            }
            for x in 0..bw {
                let avg = V::floor(V::mul(V::add(sum, bias), rcp));
                V::store_u16(&mut mid[mid_row + x * 4 + g..], avg);
                if x + 1 == bw {
                    break;
                }
                let e = V::load_u16(&src[src_row + enter[x] * 4 + g..]);
                let l = V::load_u16(&src[src_row + leave[x] * 4 + g..]);
                sum = V::add(sum, V::sub(e, l));
            }
        }
    }
    let width4 = bw * 4;
    let mut sums = zeros::<u32>(width4)?;
    for d in -(i64::from(r))..=i64::from(r) {
        let row = (edge(i64::from(b.y) + d, h) - a.y) as usize * width4;
        for (s, m) in sums.iter_mut().zip(&mid[row..row + width4]) {
            *s += u32::from(*m);
        }
    }
    for y in 0..b.height as usize {
        check()?;
        let row = y * width4;
        for j in (0..width4).step_by(V::N) {
            let avg = V::floor(V::mul(V::add(V::load_u32(&sums[j..]), bias), rcp));
            V::store_u16(&mut dst[row + j..], avg);
        }
        if y + 1 == b.height as usize {
            break;
        }
        let leave = (edge(i64::from(b.y) + y as i64 - i64::from(r), h) - a.y) as usize * width4;
        let enter = (edge(i64::from(b.y) + y as i64 + i64::from(r) + 1, h) - a.y) as usize * width4;
        for j in (0..width4).step_by(V::N) {
            let e = V::load_u16(&mid[enter + j..]);
            let l = V::load_u16(&mid[leave + j..]);
            let s = V::add(V::load_u32(&sums[j..]), V::sub(e, l));
            V::store_u32(&mut sums[j..], s);
        }
    }
    Ok(dst)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn box_blur_avx2(
    src: &[u16],
    a: Rect,
    b: Rect,
    r: u32,
    w: u32,
    h: u32,
    check: Check<'_>,
) -> Result<Vec<u16>, Error> {
    box_blur_lanes::<Avx2>(src, a, b, r, w, h, check)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn box_blur_sse41(
    src: &[u16],
    a: Rect,
    b: Rect,
    r: u32,
    w: u32,
    h: u32,
    check: Check<'_>,
) -> Result<Vec<u16>, Error> {
    box_blur_lanes::<Sse41>(src, a, b, r, w, h, check)
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn box_blur_neon(
    src: &[u16],
    a: Rect,
    b: Rect,
    r: u32,
    w: u32,
    h: u32,
    check: Check<'_>,
) -> Result<Vec<u16>, Error> {
    box_blur_lanes::<Neon>(src, a, b, r, w, h, check)
}

/// 箱ぼかしの SIMD の道（スカラーの道なら None）。
#[allow(clippy::too_many_arguments)]
pub(super) fn box_blur_at(
    level: Level,
    src: &[u16],
    a: Rect,
    b: Rect,
    r: u32,
    w: u32,
    h: u32,
    check: Check<'_>,
) -> Option<Result<Vec<u16>, Error>> {
    match level.min(simd::detect()) {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
        Level::Avx2 => Some(unsafe { box_blur_avx2(src, a, b, r, w, h, check) }),
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => Some(unsafe { box_blur_sse41(src, a, b, r, w, h, check) }),
        #[cfg(target_arch = "aarch64")]
        // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
        Level::Neon => Some(unsafe { box_blur_neon(src, a, b, r, w, h, check) }),
        Level::Scalar => None,
    }
}

#[cfg(test)]
mod tests;
