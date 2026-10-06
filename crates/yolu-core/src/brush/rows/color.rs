//! 画素ごとの色を持つブラシ（ダブごとの色の変化・色の混ぜ）の行の核。覆いの行（[`super`]）の後に、下地の読み（混ぜ）・混ぜた色・
//! 画素ごとの色の積み・ストロークの覆いへの寄せ・合成・面への書き込みをレーンで行う（`apply_at` の画素ごとの色の道と同じバイト）。
//!
//! 画素ごとの色は R・G・B・A の面（`StrokeTile::paint`）に f32 で持つ。f32 の足し算・割り算は、f64 で計算して f32 に丸めた値と
//! 同じ（仮数が 53 ≥ 2 × 24 + 2 桁あるので二重丸めが結果を変えない）。下地の合計（荷の更新の元）は、画素の順に足す今までの
//! 足し算の列を保つため、レーンで積を求めてからスカラーで 1 画素ずつ足す。

use super::effect::{blur_at_lanes, byte255, load_run};
use super::*;
use crate::math::simd::to_byte;

/// 画素ごとの色のブラシの覆いの行 cov の [lo, hi) を当てる。下地を読めないブロックは画素ごとの式で描く。
#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn apply_color_range<V: F32Lanes>(
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
    let ts = cx.tile_size;
    let plane = ts * ts;
    let canvas = (p.width, p.height);
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let (opacity, flow_k) = (V::splat(s.opacity), V::splat(s.flow));
    let (flow_scale, pressure_opacity, pressure_flow) = (
        V::splat(shape.flow_scale),
        V::splat(pressure.opacity),
        V::splat(pressure.flow),
    );
    let density = p.mix.map(|m| V::splat(m.density));
    let dab = p.dab_color.to_array();
    let mut changed = false;
    let mut i = lo;
    while i + V::N <= hi {
        let local = row + x0 + i;
        let coverage = V::load_f64(&cov[i..]);
        if !V::any(V::gt(coverage, zero)) {
            i += V::N;
            continue;
        }
        let first = (px_first + i as i64, py);
        // 下地（混ぜるブラシ）。読めないブロックは、状態に触れる前に画素ごとの式へ回す
        let ground = match p.mix {
            None => Some(None),
            Some(mix) => {
                let frame = p.frame.expect("混ぜの下地");
                let g = if mix.mode == MixMode::Smear {
                    let xs: [i64; 4] = std::array::from_fn(|k| {
                        (first.0 + k as i64 + mix.shift.0).clamp(0, canvas.0 - 1)
                    });
                    let y = (first.1 + mix.shift.1).clamp(0, canvas.1 - 1);
                    blur_at_lanes::<V>(frame, canvas, mix.blur, xs, y)
                } else {
                    load_run::<V>(frame, first.0, first.1)
                };
                g.map(Some)
            }
        };
        let Some(ground) = ground else {
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
        // 画素の色 d（0〜255）と、混ぜる色が無い（重みが 0 以下）画素の除外
        let mut valid = V::eq(zero, zero);
        let d = match (p.mix, ground) {
            (Some(mix), Some(g)) => {
                // 打点の下地の合計に足す（被覆 × 下地のアルファで重みを付ける。足す順は画素の順）
                let weight = V::mul(coverage, V::unit(g[3]));
                let (mut ws, mut wr, mut wg, mut wb) = ([0.0f64; 4], [0.0; 4], [0.0; 4], [0.0; 4]);
                V::store_f64(&mut ws, weight);
                V::store_f64(&mut wr, V::mul(weight, g[0]));
                V::store_f64(&mut wg, V::mul(weight, g[1]));
                V::store_f64(&mut wb, V::mul(weight, g[2]));
                for k in 0..V::N {
                    if cov[i + k] > 0.0 {
                        cx.tally.cover += cov[i + k];
                        cx.tally.weight += ws[k];
                        cx.tally.rgb[0] += wr[k];
                        cx.tally.rgb[1] += wg[k];
                        cx.tally.rgb[2] += wb[k];
                    }
                }
                // 混ぜた色: 下地と荷を、絵の具の量でアルファ重みの補間（アルファは荷のまま）
                let amount = V::splat(mix.paint);
                let wu = V::mul(g[3], V::sub(one, amount));
                let wc = V::splat(mix.carry[3] * mix.paint);
                let w = V::add(wu, wc);
                valid = V::not(V::le(w, zero));
                let mut d = [zero; 4];
                for c in 0..3 {
                    let mixed = V::div(
                        V::add(V::mul(g[c], wu), V::mul(V::splat(mix.carry[c]), wc)),
                        w,
                    );
                    d[c] = byte255::<V>(mixed);
                }
                d[3] = V::splat(super::super::effects::byte255(mix.carry[3]) as f64);
                d
            }
            _ => [
                V::splat(dab[0] as f64),
                V::splat(dab[1] as f64),
                V::splat(dab[2] as f64),
                V::splat(dab[3] as f64),
            ],
        };
        let opacity_scale = match scale {
            Scale::Const(c) => V::splat(*c),
            Scale::Row(r) => V::load_f64(&r[i..]),
        };
        let mut ceiling = V::mul(V::mul(opacity, opacity_scale), pressure_opacity);
        if let Some(density) = density {
            ceiling = V::mul(ceiling, density);
        }
        let flow = V::mul(V::mul(V::mul(coverage, flow_k), flow_scale), pressure_flow);
        let active = V::and(
            valid,
            V::not(V::or(V::le(flow, zero), V::le(ceiling, zero))),
        );
        if !V::any(active) {
            i += V::N;
            continue;
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
            V::add(
                previous,
                V::mul(V::sub(ceiling, previous), V::min(one, flow)),
            ),
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
            let unit = V::unit(d[c]);
            let first_value = V::round_f32(unit);
            let step = V::round_f32(V::mul(V::sub(unit, old), w));
            let next_value = V::round_f32(V::add(old, step));
            let value = V::select(fresh, first_value, next_value);
            V::store_f32(&mut paint[at..], V::select(active, value, old));
            color[c] = to_byte::<V>(value);
        }
        let start = match &st.before {
            None => [zero; 4],
            Some(Tile::Uniform(c)) => V::splat_px(c.to_array()),
            Some(Tile::Data(d)) => V::load(&d[local * 4..]),
        };
        let next = match blend_block::<V>(start, color, V::min(one, accumulated)) {
            Some(out) => out,
            None => start,
        };
        let current = load_live::<V>(live, local);
        let out = [
            V::select(active, next[0], current[0]),
            V::select(active, next[1], current[1]),
            V::select(active, next[2], current[2]),
            V::select(active, next[3], current[3]),
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
