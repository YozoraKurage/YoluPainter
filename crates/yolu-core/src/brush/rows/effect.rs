//! 効果のブラシ（指先・クローン・ぼかし）の行の核。覆いの行（[`super`]）の後に、ストロークの覆いへの寄せと、読み元の枠から読んだ色との
//! 混ぜ・面への書き込みをレーンで行う（`apply_at` の効果の道と同じバイト）。
//!
//! 読み元の枠は打点の前に凍結してあるので、画素どうしは独立。読む位置がキャンバスの端にかかる・隣のレーンと連続しない・枠の外のブロックと、
//! N 画素のウィンドウにできない余りの画素は、1 本のレーンで同じ式を通る（読みは画素ごとの [`sample32`]・[`blur32`]。端でない画素はレーンの読みと同じ値）。

use super::*;

/// 効果の種類（[`apply_effect_range`] の定数）。
pub(super) const SMUDGE: u8 = 1;
pub(super) const CLONE: u8 = 2;
pub(super) const BLUR: u8 = 3;

/// 0〜255 に四捨五入して収める（`effects::byte255` の f32 のレーンの版）。
#[inline(always)]
pub(super) unsafe fn byte255<V: Lanes32>(v: V::F) -> V::F {
    unsafe {
        let f = V::floor(V::add(v, V::splat(0.5)));
        V::max(V::splat(0.0), V::min(V::splat(255.0), f))
    }
}

/// 指先・ぼかしの混ぜ方（`effects::mix_effect` の f32 の式）: アルファで重みを付けた補間。重みの和が 0 以下なら描く前の色のまま
/// アルファ 0。
#[inline(always)]
unsafe fn mix_effect_block<V: Lanes32>(
    start: [V::F; 4],
    sample: [V::F; 4],
    amount: V::F,
) -> [V::F; 4] {
    unsafe {
        let (zero, one) = (V::splat(0.0), V::splat(1.0));
        let a = V::mul(start[3], V::sub(one, amount));
        let b = V::mul(sample[3], amount);
        let alpha = V::add(a, b);
        let dead = V::le(alpha, zero);
        let mut out = [zero; 4];
        for c in 0..3 {
            let v = V::div(V::add(V::mul(start[c], a), V::mul(sample[c], b)), alpha);
            out[c] = V::select(dead, start[c], byte255::<V>(v));
        }
        out[3] = V::select(dead, zero, byte255::<V>(alpha));
        out
    }
}

/// 1 画素の [`mix_effect_block`]（画素ごとの式が使う）。
pub(in crate::brush) fn mix_effect32(start: Rgba8, sample: Rgba8, amount: f32) -> Rgba8 {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    rgba_of(unsafe { mix_effect_block::<Scalar1>(lanes_of(start), lanes_of(sample), amount) })
}

/// 枠のキャンバスの画素 (px, py) から右へ N 画素を RGBA のレーンに（枠の中に全部あるとき）。
#[inline(always)]
pub(super) unsafe fn load_run<V: Lanes32>(
    frame: &EffectFrame,
    px: i64,
    py: i64,
) -> Option<[V::F; 4]> {
    let mut bytes = [0u8; 32];
    if frame.run_bytes(px, py, V::N, &mut bytes) {
        Some(unsafe { V::load(&bytes) })
    } else {
        None
    }
}

/// 4 つの画素（左下・右下・左上・右上）の双線形（プリマルチプライド。`EffectFrame::sample` の f32 の式）。小数部がちょうど 0 の
/// 画素は左下の画素そのもの、重みの和が 0 以下なら透明。
#[inline(always)]
unsafe fn bilinear_block<V: Lanes32>(px: [[V::F; 4]; 4], fx: V::F, fy: V::F) -> [V::F; 4] {
    unsafe {
        let (zero, one) = (V::splat(0.0), V::splat(1.0));
        let (omfx, omfy) = (V::sub(one, fx), V::sub(one, fy));
        let weights = [
            V::mul(omfx, omfy),
            V::mul(fx, omfy),
            V::mul(omfx, fy),
            V::mul(fx, fy),
        ];
        let (mut r, mut g, mut b, mut alpha) = (zero, zero, zero, zero);
        for (p, weight) in px.into_iter().zip(weights) {
            let v = V::mul(p[3], weight);
            r = V::add(r, V::mul(p[0], v));
            g = V::add(g, V::mul(p[1], v));
            b = V::add(b, V::mul(p[2], v));
            alpha = V::add(alpha, v);
        }
        let dead = V::le(alpha, zero);
        let blended = [
            byte255::<V>(V::div(r, alpha)),
            byte255::<V>(V::div(g, alpha)),
            byte255::<V>(V::div(b, alpha)),
            byte255::<V>(alpha),
        ];
        let exact = V::and(V::eq(fx, zero), V::eq(fy, zero));
        let mut out = [zero; 4];
        for c in 0..4 {
            out[c] = V::select(exact, px[0][c], V::select(dead, zero, blended[c]));
        }
        out
    }
}

/// 画素の座標 (x, y)（整数で画素そのもの）の双線形（`EffectFrame::sample` の f32 の式。キャンバスの端の外は端の画素を延ばす）。
/// 位置の整数部と小数部は f64 で求め、小数部を f32 へ丸める（レーンの読みと同じ値）。
pub(in crate::brush) fn sample32(frame: &EffectFrame, x: f64, y: f64, w: i64, h: i64) -> Rgba8 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = ((x - ix) as f32, (y - iy) as f32);
    let (ix, iy) = (ix as i64, iy as i64);
    let at = |px: i64, py: i64| lanes_of(frame.pixel(px.min(w - 1), py.min(h - 1)));
    let px = [
        at(ix, iy),
        at(ix + 1, iy),
        at(ix, iy + 1),
        at(ix + 1, iy + 1),
    ];
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    rgba_of(unsafe { bilinear_block::<Scalar1>(px, fx, fy) })
}

/// 読み元の枠の双線形の N 画素（[`sample32`] と同じ式）。読む位置がキャンバスの外・端の丸め・隣のレーンと連続しない・枠の外のときは None
/// （呼び手が 1 画素ずつ描く）。
#[inline(always)]
unsafe fn sample_lanes<V: Lanes32>(
    frame: &EffectFrame,
    canvas: (i64, i64),
    offset: (f64, f64),
    first: (i64, i64),
) -> Option<[V::F; 4]> {
    let (w, h) = canvas;
    let (px, py) = first;
    let sy = py as f64 + offset.1;
    if sy < 0.0 || sy > (h - 1) as f64 {
        return None;
    }
    let iy = sy.floor();
    let fy = (sy - iy) as f32;
    let iy = iy as i64;
    let (cy0, cy1) = (iy.min(h - 1), (iy + 1).min(h - 1));
    let (mut ix, mut fx) = ([0i64; 8], [0.0f32; 8]);
    for k in 0..V::N {
        let sx = (px + k as i64) as f64 + offset.0;
        if sx < 0.0 || sx > (w - 1) as f64 {
            return None;
        }
        let f = sx.floor();
        ix[k] = f as i64;
        fx[k] = (sx - f) as f32;
        if ix[k] != ix[0] + k as i64 {
            return None;
        }
    }
    // 右の画素の列がキャンバスの端で丸められないこと
    if ix[0] + V::N as i64 > w - 1 {
        return None;
    }
    let a = unsafe { load_run::<V>(frame, ix[0], cy0)? };
    let b = unsafe { load_run::<V>(frame, ix[0] + 1, cy0)? };
    let c = unsafe { load_run::<V>(frame, ix[0], cy1)? };
    let d = unsafe { load_run::<V>(frame, ix[0] + 1, cy1)? };
    unsafe {
        let fxv = V::from_fn(|k| fx[k]);
        Some(bilinear_block::<V>([a, b, c, d], fxv, V::splat(fy)))
    }
}

/// ぼかしの箱の平均（f32）: プリマルチプライドの和 ÷ アルファの和を 0〜255 に四捨五入、アルファは アルファの和 ÷ 画素数。
/// アルファの和が 0 なら透明。和と画素数は整数を f32 へ丸めた値。
#[inline(always)]
unsafe fn blur_block<V: Lanes32>(sums: [V::F; 4], area: V::F) -> [V::F; 4] {
    unsafe {
        let zero = V::splat(0.0);
        let total = sums[3];
        let dead = V::eq(total, zero);
        let mut out = [zero; 4];
        for c in 0..3 {
            out[c] = V::select(dead, zero, byte255::<V>(V::div(sums[c], total)));
        }
        out[3] = V::select(dead, zero, byte255::<V>(V::div(total, area)));
        out
    }
}

/// (x, y) を中心に半径 radius の箱の平均（キャンバス w × h の中だけ。[`blur_block`] の 1 画素）。
pub(in crate::brush) fn blur32(
    frame: &EffectFrame,
    x: i64,
    y: i64,
    radius: i64,
    w: i64,
    h: i64,
) -> Rgba8 {
    let (sums, area) = frame.blur_box(x, y, radius, w, h);
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    rgba_of(unsafe { blur_block::<Scalar1>(sums.map(|v| v as f32), area as f32) })
}

/// ぼかしの箱の平均（[`blur32`] と同じ式）の N 画素。中心の x は `xs`（左から順に大きくなるか同じ）、y は共通。箱はキャンバスの端で
/// 切るので、レーンの箱の左右の端も左から順に並び、全部の箱が積分画像の中にあるかは、最初の箱の左と最後の箱の右で決まる。
/// 外に出る箱があるときは None（呼び手が 1 画素ずつ描く）。
#[inline(always)]
pub(super) unsafe fn blur_at_lanes<V: Lanes32>(
    frame: &EffectFrame,
    canvas: (i64, i64),
    radius: i64,
    xs: &[i64; 8],
    y: i64,
) -> Option<[V::F; 4]> {
    let (w, h) = canvas;
    let (y0, y1) = ((y - radius).max(0), (y + radius).min(h - 1) + 1);
    let left = |x: i64| (x - radius).max(0);
    let right = |x: i64| (x + radius).min(w - 1) + 1;
    if !frame.integral_covers(left(xs[0]), y0, right(xs[V::N - 1]), y1) {
        return None;
    }
    let (top, bottom) = frame.integral_rows(y0, y1);
    let (mut sums, mut areas) = ([[0.0f32; 8]; 4], [0.0f32; 8]);
    for k in 0..V::N {
        let (x0, x1) = (left(xs[k]), right(xs[k]));
        let (l, r) = (frame.integral_column(x0), frame.integral_column(x1));
        for c in 0..4 {
            sums[c][k] = (bottom[r + c] - bottom[l + c] - top[r + c] + top[l + c]) as f32;
        }
        areas[k] = ((x1 - x0) * (y1 - y0)) as f32;
    }
    unsafe {
        Some(blur_block::<V>(
            sums.map(|s| V::from_fn(|k| s[k])),
            V::from_fn(|k| areas[k]),
        ))
    }
}

/// 効果のブラシの N 画素: ストロークの覆いを寄せ（天井に届いた画素もそのまま寄せ続ける）、読んだ色 sampled と混ぜて書く。
/// readable が偽のレーン（読む位置がキャンバスの外）は、覆いだけ寄せて画素は書かない。ぼかしは透明部分に色を広げない。
#[inline(always)]
#[allow(clippy::too_many_arguments)]
unsafe fn effect_block<V: Slice32, const KIND: u8>(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dab: &Dab32,
    local: usize,
    coverage: V::F,
    opacity_scale: V::F,
    (sampled, readable): ([V::F; 4], V::M),
) -> Result<bool, CoreError> {
    unsafe {
        let (zero, one) = (V::splat(0.0), V::splat(1.0));
        let ceiling = dab.ceiling::<V>(opacity_scale);
        let flow = dab.flow::<V>(coverage);
        let active = V::not(V::or(V::le(flow, zero), V::le(ceiling, zero)));
        if !V::any(active) {
            return Ok(false);
        }
        let previous = match held.as_ref() {
            Some(st) => V::load_f32(&st.wash[local..]),
            None => zero,
        };
        if held.is_none() {
            *held = Some(new_stroke_tile(cx, live, false)?);
        }
        let st = held.as_mut().expect("直前に作った");
        let accumulated = V::select(
            V::ge(previous, ceiling),
            previous,
            accumulate::<V>(previous, ceiling, flow),
        );
        V::store_f32(
            &mut st.wash[local..],
            V::select(active, accumulated, previous),
        );
        let start = load_before::<V>(st, local);
        let amount = V::min(one, accumulated);
        let mut next = if KIND == CLONE {
            // 選択範囲の量は 1（フェードの量 1 は中身そのもの）
            normal_block::<V>(start, sampled, amount)
        } else {
            mix_effect_block::<V>(start, sampled, amount)
        };
        // アルファが 0 になる画素は、描く前の RGB のまま
        let clear = V::eq(next[3], zero);
        for c in 0..3 {
            next[c] = V::select(clear, start[c], next[c]);
        }
        let mut writes = V::and(active, readable);
        if KIND == BLUR {
            writes = V::and(writes, V::not(V::eq(start[3], zero)));
        }
        let current = load_live::<V>(live, local);
        let out = [
            V::select(writes, next[0], current[0]),
            V::select(writes, next[1], current[1]),
            V::select(writes, next[2], current[2]),
            V::select(writes, next[3], current[3]),
        ];
        commit::<V>(cx, live, local, out, current)
    }
}

/// 効果のブラシ（指先・クローン・ぼかし）の覆いの行 cov の [lo, hi) を当てる（`apply_at` の効果の道と同じ結果）。
/// `(row, x0)` は行の先頭の画素の番号と行の中の最初の画素の位置、`(px_first, py)` は行の最初の画素のキャンバスの座標。
#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn apply_effect_range<V: Slice32, const KIND: u8>(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dab: &Dab32,
    (row, x0): (usize, usize),
    cov: &[f32],
    scale: Scale<'_>,
    (lo, hi): (usize, usize),
    (px_first, py): (i64, i64),
) -> Result<bool, CoreError> {
    let p = cx.paint;
    let frame = p.frame.expect("効果の読み元");
    let canvas = (p.width, p.height);
    let radius = match p.effect {
        EffectKind::Blur(radius) => radius,
        _ => 0,
    };
    let mut changed = false;
    let mut i = lo;
    let n = if V::N > 1 { cov.len() } else { 0 };
    while let Some(b) = next_block(i, hi, n, V::N) {
        // このブロックが受け持つ画素（ウィンドウなら生かす画素）
        let part = b.live.unwrap_or((i, i + V::N));
        i = part.1;
        let local = row + x0 + b.at;
        let (coverage, opacity_scale) =
            unsafe { (block_coverage::<V>(cov, b), scale.lanes::<V>(b.at)) };
        // 塗る画素の無いブロックは読まない
        let any = unsafe {
            let zero = V::splat(0.0);
            let ceiling = dab.ceiling::<V>(opacity_scale);
            let flow = dab.flow::<V>(coverage);
            V::any(V::not(V::or(V::le(flow, zero), V::le(ceiling, zero))))
        };
        if !any {
            continue;
        }
        let first = (px_first + b.at as i64, py);
        let sampled = unsafe {
            match KIND {
                BLUR => {
                    let xs: [i64; 8] = std::array::from_fn(|k| first.0 + k as i64);
                    blur_at_lanes::<V>(frame, canvas, radius, &xs, first.1)
                }
                _ => sample_lanes::<V>(frame, canvas, (p.offset_x, p.offset_y), first),
            }
        };
        changed |= match sampled {
            Some(sampled) => unsafe {
                let all = V::eq(V::splat(0.0), V::splat(0.0));
                effect_block::<V, KIND>(
                    cx,
                    held,
                    live,
                    dab,
                    local,
                    coverage,
                    opacity_scale,
                    (sampled, all),
                )?
            },
            // 読む位置が端にかかるブロックは、1 画素ずつ
            None => effect_rest::<KIND>(
                cx,
                held,
                live,
                dab,
                (row, x0),
                cov,
                scale,
                part,
                (px_first, py),
            )?,
        };
    }
    if i < hi {
        changed |= effect_rest::<KIND>(
            cx,
            held,
            live,
            dab,
            (row, x0),
            cov,
            scale,
            (i, hi),
            (px_first, py),
        )?;
    }
    Ok(changed)
}

/// 効果のブラシの [lo, hi) を 1 画素ずつ（1 本のレーン）。読みは画素ごと（[`sample32`]・[`blur32`]）。
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn effect_rest<const KIND: u8>(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dab: &Dab32,
    (row, x0): (usize, usize),
    cov: &[f32],
    scale: Scale<'_>,
    (lo, hi): (usize, usize),
    (px_first, py): (i64, i64),
) -> Result<bool, CoreError> {
    let p = cx.paint;
    let frame = p.frame.expect("効果の読み元");
    let (w, h) = (p.width, p.height);
    let mut changed = false;
    for (i, &coverage) in cov.iter().enumerate().take(hi).skip(lo) {
        // SAFETY: 1 本のレーンは CPU の前提を持たない
        let opacity_scale = unsafe { scale.lanes::<Scalar1>(i) };
        // SAFETY: 同上
        let (ceiling, flow) = unsafe {
            (
                dab.ceiling::<Scalar1>(opacity_scale),
                dab.flow::<Scalar1>(coverage),
            )
        };
        if flow <= 0.0 || ceiling <= 0.0 {
            continue;
        }
        let px = px_first + i as i64;
        let sampled = match p.effect {
            EffectKind::Blur(radius) => Some(blur32(frame, px, py, radius, w, h)),
            _ => {
                let (sx, sy) = (px as f64 + p.offset_x, py as f64 + p.offset_y);
                (sx >= 0.0 && sy >= 0.0 && sx <= (w - 1) as f64 && sy <= (h - 1) as f64)
                    .then(|| sample32(frame, sx, sy, w, h))
            }
        };
        let readable = sampled.is_some();
        let sampled = lanes_of(sampled.unwrap_or(Rgba8::TRANSPARENT));
        // SAFETY: 同上
        changed |= unsafe {
            effect_block::<Scalar1, KIND>(
                cx,
                held,
                live,
                dab,
                row + x0 + i,
                coverage,
                opacity_scale,
                (sampled, readable),
            )?
        };
    }
    Ok(changed)
}
