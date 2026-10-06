//! 効果のブラシ（指先・クローン・ぼかし）の行の核。覆いの行（[`super`]）の後に、ストロークの覆いへの寄せと、読み元の枠から読んだ色との
//! 混ぜ・面への書き込みをレーンで行う（`apply_at` の効果の道と同じバイト）。
//!
//! 読み元の枠は打点の前に凍結してあるので、画素どうしは独立。読む位置が画布の端にかかる・隣のレーンと連続しない・枠の外のブロックは、
//! 状態に触れる前に画素ごとの式（`apply_at`）へ回す。

use super::*;

/// 効果の種類（[`apply_effect_range`] の定数）。
pub(super) const SMUDGE: u8 = 1;
pub(super) const CLONE: u8 = 2;
pub(super) const BLUR: u8 = 3;

/// 0〜255 に四捨五入して収める（`effects::byte255` のレーンの版）。
#[inline(always)]
pub(super) unsafe fn byte255<V: Lanes>(v: V::F) -> V::F {
    let f = V::floor(V::add(v, V::splat(0.5)));
    V::max(V::splat(0.0), V::min(V::splat(255.0), f))
}

/// 指先・ぼかしの混ぜ方（`effects::mix_effect` のレーンの版）: アルファで重みを付けた補間。
#[inline(always)]
unsafe fn mix_effect_block<V: Lanes>(
    start: [V::F; 4],
    sample: [V::F; 4],
    amount: V::F,
) -> [V::F; 4] {
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

/// 枠の画布の画素 (px, py) から右へ N 画素を RGBA のレーンに（枠の中に全部あるとき）。
#[inline(always)]
pub(super) unsafe fn load_run<V: Lanes>(
    frame: &EffectFrame,
    px: i64,
    py: i64,
) -> Option<[V::F; 4]> {
    let mut bytes = [0u8; 16];
    if frame.run_bytes(px, py, V::N, &mut bytes) {
        Some(V::load(&bytes))
    } else {
        None
    }
}

/// 読み元の枠の双線形（`EffectFrame::sample` と同じ式）の N 画素。読む位置が画布の外・端の丸め・隣のレーンと連続しない・
/// 枠の外のときは None（呼び手が画素ごとの式で描く）。
#[inline(always)]
unsafe fn sample_lanes<V: Lanes>(
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
    let iy = sy.floor() as i64;
    let fy = sy - iy as f64;
    let (cy0, cy1) = (iy.min(h - 1), (iy + 1).min(h - 1));
    let (mut ix, mut fx) = ([0i64; 4], [0.0f64; 4]);
    for k in 0..V::N {
        let sx = (px + k as i64) as f64 + offset.0;
        if sx < 0.0 || sx > (w - 1) as f64 {
            return None;
        }
        ix[k] = sx.floor() as i64;
        fx[k] = sx - ix[k] as f64;
        if ix[k] != ix[0] + k as i64 {
            return None;
        }
    }
    // 右の画素の列が画布の端で丸められないこと
    if ix[0] + V::N as i64 > w - 1 {
        return None;
    }
    let a = load_run::<V>(frame, ix[0], cy0)?;
    let b = load_run::<V>(frame, ix[0] + 1, cy0)?;
    let c = load_run::<V>(frame, ix[0], cy1)?;
    let d = load_run::<V>(frame, ix[0] + 1, cy1)?;
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let fxv = V::from_fn(|k| fx[k]);
    let (omfx, fyv) = (V::sub(one, fxv), V::splat(fy));
    let omfy = V::splat(1.0 - fy);
    let weights = [
        V::mul(omfx, omfy),
        V::mul(fxv, omfy),
        V::mul(omfx, fyv),
        V::mul(fxv, fyv),
    ];
    let (mut r, mut g, mut bl, mut alpha) = (zero, zero, zero, zero);
    for (px, weight) in [a, b, c, d].into_iter().zip(weights) {
        let v = V::mul(px[3], weight);
        r = V::add(r, V::mul(px[0], v));
        g = V::add(g, V::mul(px[1], v));
        bl = V::add(bl, V::mul(px[2], v));
        alpha = V::add(alpha, v);
    }
    let dead = V::le(alpha, zero);
    let blended = [
        byte255::<V>(V::div(r, alpha)),
        byte255::<V>(V::div(g, alpha)),
        byte255::<V>(V::div(bl, alpha)),
        byte255::<V>(alpha),
    ];
    // 小数部がちょうど 0 の画素は、その画素の色そのもの
    let exact = V::and(V::eq(fxv, zero), V::eq(fyv, zero));
    let mut out = [zero; 4];
    for c in 0..4 {
        out[c] = V::select(exact, a[c], V::select(dead, zero, blended[c]));
    }
    Some(out)
}

/// ぼかしの箱の平均（`EffectFrame::blur` と同じ式。箱は画布の端で切る）の N 画素。中心の x は `xs`、y は共通。積分画像の外に出る箱が
/// あるときは None（呼び手が画素ごとの式で描く）。
#[inline(always)]
pub(super) unsafe fn blur_at_lanes<V: Lanes>(
    frame: &EffectFrame,
    canvas: (i64, i64),
    radius: i64,
    xs: [i64; 4],
    y: i64,
) -> Option<[V::F; 4]> {
    let (w, h) = canvas;
    let (y0, y1) = ((y - radius).max(0), (y + radius).min(h - 1) + 1);
    let mut sums = [[0.0f64; 4]; 4];
    let mut areas = [0.0f64; 4];
    for k in 0..V::N {
        let x0 = (xs[k] - radius).max(0);
        let x1 = (xs[k] + radius).min(w - 1) + 1;
        if !frame.integral_covers(x0, y0, x1, y1) {
            return None;
        }
        let box_sums = frame.box_sums(x0, y0, x1, y1);
        for c in 0..4 {
            sums[c][k] = box_sums[c] as f64;
        }
        areas[k] = ((x1 - x0) * (y1 - y0)) as f64;
    }
    let zero = V::splat(0.0);
    let area = V::from_fn(|k| areas[k]);
    let total = V::from_fn(|k| sums[3][k]);
    let dead = V::eq(total, zero);
    let mut out = [zero; 4];
    for c in 0..3 {
        let sum = V::from_fn(|k| sums[c][k]);
        out[c] = V::select(dead, zero, byte255::<V>(V::div(sum, total)));
    }
    out[3] = V::select(dead, zero, byte255::<V>(V::div(total, area)));
    Some(out)
}

/// 効果のブラシ（指先・クローン・ぼかし）の覆いの行 cov の [lo, hi) を当てる（`apply_at` の効果の道と同じ結果）。
/// 読む位置が画布の端にかかるブロックは、画素ごとの式で描く。`(row, x0)` は行の先頭の画素の番号と行の中の最初の画素の位置、
/// `(px_first, py)` は行の最初の画素の画布の座標。
#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn apply_effect_range<V: F32Lanes, const KIND: u8>(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    coord: TileCoord,
    (row, x0): (usize, usize),
    cov: &[f64],
    scale: &Scale<'_>,
    (lo, hi): (usize, usize),
    shape: &DabShape<'_>,
    (px_first, py): (i64, i64),
) -> Result<bool, CoreError> {
    let p = cx.paint;
    let s = p.s;
    let pressure = shape.pressure;
    let frame = p.frame.expect("効果の読み元");
    let canvas = (p.width, p.height);
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let (opacity, flow_k) = (V::splat(s.opacity), V::splat(s.flow));
    let (flow_scale, pressure_opacity, pressure_flow) = (
        V::splat(shape.flow_scale),
        V::splat(pressure.opacity),
        V::splat(pressure.flow),
    );
    let strength = match p.effect {
        EffectKind::Smudge(strength) => Some(V::splat(strength)),
        _ => None,
    };
    let radius = match p.effect {
        EffectKind::Blur(radius) => radius,
        _ => 0,
    };
    let mut changed = false;
    let mut i = lo;
    while i + V::N <= hi {
        let local = row + x0 + i;
        let coverage = V::load_f64(&cov[i..]);
        let opacity_scale = match scale {
            Scale::Const(c) => V::splat(*c),
            Scale::Row(r) => V::load_f64(&r[i..]),
        };
        let ceiling = V::mul(V::mul(opacity, opacity_scale), pressure_opacity);
        let mut flow = V::mul(V::mul(V::mul(coverage, flow_k), flow_scale), pressure_flow);
        if let Some(strength) = strength {
            flow = V::mul(flow, strength);
        }
        let active = V::not(V::or(V::le(flow, zero), V::le(ceiling, zero)));
        if !V::any(active) {
            i += V::N;
            continue;
        }
        // 読む位置を先に確かめる（端にかかるブロックは、状態に触れる前に画素ごとの式へ回す）
        let first = (px_first + i as i64, py);
        let sampled = match KIND {
            BLUR => {
                let xs = [first.0, first.0 + 1, first.0 + 2, first.0 + 3];
                blur_at_lanes::<V>(frame, canvas, radius, xs, first.1)
            }
            _ => sample_lanes::<V>(frame, canvas, (p.offset_x, p.offset_y), first),
        };
        let Some(sampled) = sampled else {
            for k in 0..V::N {
                let opacity_scale = match scale {
                    Scale::Const(c) => *c,
                    Scale::Row(r) => r[i + k],
                };
                changed |= apply_at::<false>(
                    cx,
                    held,
                    live,
                    coord,
                    local + k,
                    cov[i + k],
                    pressure,
                    opacity_scale,
                    shape.flow_scale,
                    None,
                    None,
                )?;
            }
            i += V::N;
            continue;
        };
        let previous = match held.as_ref() {
            Some(st) => V::load_f32(&st.wash[local..]),
            None => zero,
        };
        if held.is_none() {
            *held = Some(new_stroke_tile(cx, live, false)?);
        }
        let st = held.as_mut().expect("直前に作った");
        // 効果のブラシは天井に届いた画素も寄せ続ける（`previous >= ceiling` ならそのまま）
        let accumulated = V::select(
            V::ge(previous, ceiling),
            previous,
            V::add(
                previous,
                V::mul(V::sub(ceiling, previous), V::min(one, flow)),
            ),
        );
        V::store_f32(
            &mut st.wash[local..],
            V::select(active, accumulated, previous),
        );
        let start = match &st.before {
            None => [zero; 4],
            Some(Tile::Uniform(c)) => V::splat_px(c.to_array()),
            Some(Tile::Data(d)) => V::load(&d[local * 4..]),
        };
        let amount = V::min(one, accumulated);
        let mut next = if KIND == CLONE {
            // 選択範囲の量は 1（`fade(.., 1)` は中身そのもの）
            match blend_block::<V, NORMAL>(start, sampled, amount) {
                Some(out) => out,
                None => start,
            }
        } else {
            mix_effect_block::<V>(start, sampled, amount)
        };
        // アルファが 0 になる画素は、描く前の RGB のまま
        let clear = V::eq(next[3], zero);
        for c in 0..3 {
            next[c] = V::select(clear, start[c], next[c]);
        }
        // ぼかしは透明部分に色を広げない
        let writes = if KIND == BLUR {
            V::and(active, V::not(V::eq(start[3], zero)))
        } else {
            active
        };
        let current = load_live::<V>(live, local);
        let out = [
            V::select(writes, next[0], current[0]),
            V::select(writes, next[1], current[1]),
            V::select(writes, next[2], current[2]),
            V::select(writes, next[3], current[3]),
        ];
        changed |= commit::<V>(cx, live, local, out, current)?;
        i += V::N;
    }
    // 端の画素（レーンの数で割った余り）は画素ごとの式で
    while i < hi {
        let local = row + x0 + i;
        let opacity_scale = match scale {
            Scale::Const(c) => *c,
            Scale::Row(r) => r[i],
        };
        changed |= apply_at::<false>(
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
