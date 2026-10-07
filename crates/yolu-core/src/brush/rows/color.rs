//! 画素ごとの色を持つブラシ（ダブごとの色の変化・色の混ぜ）の行の核。覆いの行（[`super`]）の後に、下地の読み（混ぜ）・混ぜた色・
//! 画素ごとの色の積み・ストロークの覆いへの寄せ・合成・面への書き込みをレーンで行う（`apply_at` の画素ごとの色の道と同じバイト）。
//!
//! 画素ごとの色は R・G・B・A の面（`StrokeTile::paint`）に f32 で持つ。下地の合計（荷の更新の元）は、画素の順に足す今までの
//! 足し算の列を保つため、レーンで積を求めてから 1 画素ずつ f64 へ足す。下地を読めないブロックと、N 画素のウィンドウにできない余りの画素は、
//! 1 本のレーンで同じ式を通る（下地は画素ごとの `MixDab::ground_at`。端でない画素はレーンの読みと同じ値）。

use super::effect::{blur_at_lanes, byte255, load_run};
use super::*;

/// 下地 g と荷を混ぜた色（`MixDab::pixel_color` の f32 の式）: 重み 下地のアルファ × (1 − 絵の具の量)・荷のアルファ × 量で補間し、
/// 0〜255 に四捨五入する。アルファは荷のまま。重みの和が 0 以下のレーンは偽（混ぜる色が無く、塗らない）。
#[inline(always)]
unsafe fn mix_color<V: Lanes32>(mix: &MixDab, g: [V::F; 4]) -> (V::M, [V::F; 4]) {
    unsafe {
        let (zero, one) = (V::splat(0.0), V::splat(1.0));
        let amount = V::splat(mix.paint as f32);
        let wu = V::mul(g[3], V::sub(one, amount));
        let wc = V::mul(V::splat(mix.carry[3] as f32), amount);
        let w = V::add(wu, wc);
        let valid = V::not(V::le(w, zero));
        let mut d = [zero; 4];
        for c in 0..3 {
            let mixed = V::div(
                V::add(V::mul(g[c], wu), V::mul(V::splat(mix.carry[c] as f32), wc)),
                w,
            );
            d[c] = byte255::<V>(mixed);
        }
        d[3] = V::splat(f32::from(super::super::effects::byte255(mix.carry[3])));
        (valid, d)
    }
}

/// 1 画素の混ぜた色（画素ごとの式が使う）。None は混ぜる色が無い。
pub(in crate::brush) fn mixed32(mix: &MixDab, ground: Rgba8) -> Option<Rgba8> {
    // SAFETY: 1 本のレーンは CPU の前提を持たない
    let (valid, d) = unsafe { mix_color::<Scalar1>(mix, lanes_of(ground)) };
    valid.then(|| rgba_of(d))
}

/// 打点の下地の合計に 1 画素を足す（被覆 × 下地のアルファで重みを付ける。積は f32、合計は f64）。
#[inline(always)]
pub(in crate::brush) fn tally32(tally: &mut MixTally, coverage: f32, ground: Rgba8) {
    let weight = coverage * unit32(ground.a);
    tally.cover += f64::from(coverage);
    tally.weight += f64::from(weight);
    tally.rgb[0] += f64::from(weight * f32::from(ground.r));
    tally.rgb[1] += f64::from(weight * f32::from(ground.g));
    tally.rgb[2] += f64::from(weight * f32::from(ground.b));
}

/// 画素ごとの色の N 画素。ground は混ぜるブラシの下地の読み（無ければダブの色）。
#[inline(always)]
#[allow(clippy::too_many_arguments)]
unsafe fn color_block<V: Slice32>(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dab: &Dab32,
    local: usize,
    coverage: V::F,
    opacity_scale: V::F,
    ground: Option<[V::F; 4]>,
) -> Result<bool, CoreError> {
    unsafe {
        let p = cx.paint;
        let plane = cx.tile_size * cx.tile_size;
        let (zero, one) = (V::splat(0.0), V::splat(1.0));
        // 画素の色 d（0〜255）と、混ぜる色が無い（重みが 0 以下）画素の除外
        let (valid, d) = match (p.mix, ground) {
            (Some(mix), Some(g)) => {
                // 打点の下地の合計に足す（被覆 × 下地のアルファで重みを付ける。足す順は画素の順）
                let weight = V::mul(coverage, V::unit(g[3]));
                let mut cov = [0.0f32; 8];
                let (mut ws, mut wr, mut wg, mut wb) = ([0.0f32; 8], [0.0; 8], [0.0; 8], [0.0; 8]);
                V::store_f32(&mut cov, coverage);
                V::store_f32(&mut ws, weight);
                V::store_f32(&mut wr, V::mul(weight, g[0]));
                V::store_f32(&mut wg, V::mul(weight, g[1]));
                V::store_f32(&mut wb, V::mul(weight, g[2]));
                for k in 0..V::N {
                    if cov[k] > 0.0 {
                        cx.tally.cover += f64::from(cov[k]);
                        cx.tally.weight += f64::from(ws[k]);
                        cx.tally.rgb[0] += f64::from(wr[k]);
                        cx.tally.rgb[1] += f64::from(wg[k]);
                        cx.tally.rgb[2] += f64::from(wb[k]);
                    }
                }
                mix_color::<V>(&mix, g)
            }
            _ => (V::eq(zero, zero), V::splat_px(p.dab_color.to_array())),
        };
        let ceiling = dab.ceiling::<V>(opacity_scale);
        let flow = dab.flow::<V>(coverage);
        let active = V::and(
            valid,
            V::not(V::or(V::le(flow, zero), V::le(ceiling, zero))),
        );
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
        // 天井に届いた画素も、色は寄せ続ける（`previous >= ceiling` なら覆いはそのまま）
        let accumulated = V::select(
            V::ge(previous, ceiling),
            previous,
            accumulate::<V>(previous, ceiling, flow),
        );
        V::store_f32(
            &mut st.wash[local..],
            V::select(active, accumulated, previous),
        );
        // 画素ごとの色の積み（最初の打点はその色、あとは流量の割合だけ寄せる）
        let w = V::min(one, flow);
        let fresh = V::le(previous, zero);
        let paint = st.paint.as_mut().expect("ダブごとの色");
        let mut color = [zero; 4];
        for c in 0..4 {
            let at = c * plane + local;
            let old = V::load_f32(&paint[at..]);
            let value = plane_step::<V>(old, V::unit(d[c]), w, fresh);
            V::store_f32(&mut paint[at..], V::select(active, value, old));
            color[c] = to_byte32::<V>(value);
        }
        let start = load_before::<V>(st, local);
        let next = normal_block::<V>(start, color, V::min(one, accumulated));
        let current = load_live::<V>(live, local);
        let out = [
            V::select(active, next[0], current[0]),
            V::select(active, next[1], current[1]),
            V::select(active, next[2], current[2]),
            V::select(active, next[3], current[3]),
        ];
        commit::<V>(cx, live, local, out, current)
    }
}

/// 画素ごとの色のブラシの覆いの行 cov の [lo, hi) を当てる。
#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn apply_color_range<V: Slice32>(
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
    let canvas = (p.width, p.height);
    let mut changed = false;
    let mut i = lo;
    let n = if V::N > 1 { cov.len() } else { 0 };
    while let Some(b) = next_block(i, hi, n, V::N) {
        // このブロックが受け持つ画素（ウィンドウなら生かす画素）
        let part = b.live.unwrap_or((i, i + V::N));
        i = part.1;
        let local = row + x0 + b.at;
        let coverage = unsafe { block_coverage::<V>(cov, b) };
        if !unsafe { V::any(V::gt(coverage, V::splat(0.0))) } {
            continue;
        }
        let first = (px_first + b.at as i64, py);
        // 下地（混ぜるブラシ）。読めないブロックは、状態に触れる前に 1 画素ずつへ回す
        let ground = match p.mix {
            None => Some(None),
            Some(mix) => {
                let frame = p.frame.expect("混ぜの下地");
                let g = unsafe {
                    if mix.mode == MixMode::Smear {
                        let xs: [i64; 8] = std::array::from_fn(|k| {
                            (first.0 + k as i64 + mix.shift.0).clamp(0, canvas.0 - 1)
                        });
                        let y = (first.1 + mix.shift.1).clamp(0, canvas.1 - 1);
                        blur_at_lanes::<V>(frame, canvas, mix.blur, &xs, y)
                    } else {
                        load_run::<V>(frame, first.0, first.1)
                    }
                };
                g.map(Some)
            }
        };
        changed |= match ground {
            Some(ground) => unsafe {
                color_block::<V>(
                    cx,
                    held,
                    live,
                    dab,
                    local,
                    coverage,
                    scale.lanes::<V>(b.at),
                    ground,
                )?
            },
            None => color_rest(
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
        changed |= color_rest(
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

/// 画素ごとの色のブラシの [lo, hi) を 1 画素ずつ（1 本のレーン）。下地は画素ごとの `MixDab::ground_at`。
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn color_rest(
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
    let mut changed = false;
    for (i, &coverage) in cov.iter().enumerate().take(hi).skip(lo) {
        if coverage <= 0.0 {
            continue;
        }
        let ground = p.mix.map(|mix| {
            let frame = p.frame.expect("混ぜの下地");
            lanes_of(mix.ground_at(frame, px_first + i as i64, py, p.width, p.height))
        });
        // SAFETY: 1 本のレーンは CPU の前提を持たない
        changed |= unsafe {
            color_block::<Scalar1>(
                cx,
                held,
                live,
                dab,
                row + x0 + i,
                coverage,
                scale.lanes::<Scalar1>(i),
                ground,
            )?
        };
    }
    Ok(changed)
}
