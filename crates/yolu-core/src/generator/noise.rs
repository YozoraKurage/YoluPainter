//! C# の種類に重ねる 4 オクターブの値ノイズ。整数の hash と f64 の + − × ÷ floor だけで、環境が違っても同じ値になる。
//! 格子点の値は hash だけで決まるので、同じ格子の中の画素どうしでは使い回せる（[`Cache`]）。値は使い回しの有無で変わらない。
//! SIMD の道では N 画素を 1 組で引く（[`fractal_lanes`]）。N 画素が同じ格子に入ればその角の値を全部のレーンで共有し、またぐ組は
//! 1 画素ずつ引く。レーンの演算は 1 画素の式と同じ順で同じ IEEE の演算だけなので、結果のビットは道によらず同じ。
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use crate::math::simd::Lanes;
pub(super) fn hash(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846ca68b);
    h ^= h >> 16;
    h
}
pub(super) fn seeds(seed: i32) -> [u32; 4] {
    let seed = hash(seed as u32 ^ 0x9e3779b9);
    std::array::from_fn(|o| hash(seed.wrapping_add((o as u32).wrapping_mul(0x85ebca6b))))
}
#[inline(always)]
fn lattice(hx: u32, y: i32, z: i32) -> f64 {
    (hash(hash(hx ^ y as u32) ^ z as u32) >> 8) as f64 * (1. / 16777215.)
}
#[inline(always)]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6. - 15.) + 10.)
}
#[inline(always)]
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}
/// 格子 `(ix, iy, iz)` の 8 つの角の値。並びは x が速く、次に y、次に z（`[c000, c100, c010, c110, c001, c101, c011, c111]`）。
/// `with_z1` が偽なら z + 1 の側（後ろの 4 つ）は 0 のまま（z の補間の重みが 0 のときは使わない）。
#[inline(always)]
fn corners(seed: u32, ix: i32, iy: i32, iz: i32, with_z1: bool) -> [f64; 8] {
    let hx0 = hash(seed ^ ix as u32);
    let hx1 = hash(seed ^ ix.wrapping_add(1) as u32);
    let mut c = [0.; 8];
    c[0] = lattice(hx0, iy, iz);
    c[1] = lattice(hx1, iy, iz);
    c[2] = lattice(hx0, iy + 1, iz);
    c[3] = lattice(hx1, iy + 1, iz);
    if with_z1 {
        c[4] = lattice(hx0, iy, iz + 1);
        c[5] = lattice(hx1, iy, iz + 1);
        c[6] = lattice(hx0, iy + 1, iz + 1);
        c[7] = lattice(hx1, iy + 1, iz + 1);
    }
    c
}
/// 1 オクターブぶんの直前の格子（その角の値）。
#[derive(Clone, Copy)]
struct Cell {
    key: [i32; 3],
    c: [f64; 8],
    /// 角が入っているか。
    valid: bool,
    /// z + 1 の側の角まで入っているか。
    z1: bool,
}
impl Cell {
    const EMPTY: Cell = Cell {
        key: [0; 3],
        c: [0.; 8],
        valid: false,
        z1: false,
    };
}
/// 行をたどるあいだの、オクターブごとの直前の格子。UV の空間や位置のマップがなめらかなら、隣の画素は同じ格子に入るので、
/// hash を引き直さずに済む（違う格子に入れば引き直す）。行（または別の入力）ごとに `new` で作り直す。
pub(super) struct Cache {
    cells: [Cell; 4],
}
impl Cache {
    pub(super) const fn new() -> Self {
        Self {
            cells: [Cell::EMPTY; 4],
        }
    }
}
#[inline(always)]
fn value_in(p: [f64; 3], seed: u32, cell: &mut Cell) -> f64 {
    let (fx, fy, fz) = (p[0].floor(), p[1].floor(), p[2].floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (u, v, w) = (fade(p[0] - fx), fade(p[1] - fy), fade(p[2] - fz));
    // w が 0 なら最後の補間は `a + (b - a) * 0 = a`（角の値は有限）なので、z + 1 の側は引かない
    let need_z1 = w != 0.;
    if !(cell.valid && cell.key == [ix, iy, iz] && (cell.z1 || !need_z1)) {
        cell.c = corners(seed, ix, iy, iz, need_z1);
        cell.key = [ix, iy, iz];
        cell.valid = true;
        cell.z1 = need_z1;
    }
    let c = &cell.c;
    let c00 = lerp(c[0], c[1], u);
    let c10 = lerp(c[2], c[3], u);
    let low = lerp(c00, c10, v);
    if !need_z1 {
        return low;
    }
    let c01 = lerp(c[4], c[5], u);
    let c11 = lerp(c[6], c[7], u);
    lerp(low, lerp(c01, c11, v), w)
}
/// 4 オクターブの値ノイズ（0..1）。`cache` は同じ行・同じ入力のあいだ使い回す。
#[inline]
pub(super) fn fractal(mut p: [f64; 3], seeds: [u32; 4], cache: &mut Cache) -> f64 {
    let mut sum = 0.;
    let mut weight = 1.;
    let mut total = 0.;
    for (seed, cell) in seeds.into_iter().zip(cache.cells.iter_mut()) {
        sum += weight * value_in(p, seed, cell);
        total += weight;
        p = p.map(|v| v * 2.);
        weight *= 0.5;
    }
    sum / total
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) unsafe fn fade_lanes<V: Lanes>(t: V::F) -> V::F {
    let t3 = V::mul(V::mul(t, t), t);
    let inner = V::add(
        V::mul(t, V::sub(V::mul(t, V::splat(6.)), V::splat(15.))),
        V::splat(10.),
    );
    V::mul(t3, inner)
}
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) unsafe fn lerp_lanes<V: Lanes>(a: V::F, b: V::F, t: V::F) -> V::F {
    V::add(a, V::mul(V::sub(b, a), t))
}
/// 全部のレーンの `floor` が同じなら、その格子の整数の座標（`floor` が違えば格子は違う。大きすぎて整数に収まらない座標で、違う `floor` が
/// 同じ整数になる場合も「違う」と見て 1 画素ずつ引くだけで、値は変わらない）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn same_key<V: Lanes>(floors: &[[f64; 4]; 3]) -> Option<[i32; 3]> {
    for lane in 1..V::N {
        if floors[0][lane] != floors[0][0]
            || floors[1][lane] != floors[1][0]
            || floors[2][lane] != floors[2][0]
        {
            return None;
        }
    }
    Some([
        floors[0][0] as i32,
        floors[1][0] as i32,
        floors[2][0] as i32,
    ])
}
/// N 画素の値ノイズ。N 画素が同じ格子に入るなら角の値を共有してレーンで補間し、またぐなら 1 画素ずつ引く（どちらも 1 画素の式と同じ値）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn value_lanes<V: Lanes>(p: [V::F; 3], seed: u32, cell: &mut Cell) -> V::F {
    let (fx, fy, fz) = (V::floor(p[0]), V::floor(p[1]), V::floor(p[2]));
    let (u, v, w) = (
        fade_lanes::<V>(V::sub(p[0], fx)),
        fade_lanes::<V>(V::sub(p[1], fy)),
        fade_lanes::<V>(V::sub(p[2], fz)),
    );
    let mut floors = [[0.; 4]; 3];
    V::store_f64(&mut floors[0], fx);
    V::store_f64(&mut floors[1], fy);
    V::store_f64(&mut floors[2], fz);
    let Some(key) = same_key::<V>(&floors) else {
        // 格子をまたぐ組: 1 画素ずつ（隣の格子の覚えに入れ替わるが、値は同じ）
        let mut at = [[0.; 4]; 3];
        for (axis, lanes) in at.iter_mut().enumerate() {
            V::store_f64(lanes, p[axis]);
        }
        let mut out = [0.; 4];
        for (k, o) in out.iter_mut().enumerate().take(V::N) {
            *o = value_in([at[0][k], at[1][k], at[2][k]], seed, cell);
        }
        return V::load_f64(&out);
    };
    // w が全部 0 なら最後の補間は `a + (b - a) * 0 = a`（角の値は有限）なので、z + 1 の側は引かない
    let need_z1 = !V::all(V::eq(w, V::splat(0.)));
    if !(cell.valid && cell.key == key && (cell.z1 || !need_z1)) {
        cell.c = corners(seed, key[0], key[1], key[2], need_z1);
        cell.key = key;
        cell.valid = true;
        cell.z1 = need_z1;
    }
    let mut c = [V::splat(0.); 8];
    for (lane, value) in c.iter_mut().zip(cell.c) {
        *lane = V::splat(value);
    }
    let x00 = lerp_lanes::<V>(c[0], c[1], u);
    let x10 = lerp_lanes::<V>(c[2], c[3], u);
    let low = lerp_lanes::<V>(x00, x10, v);
    if !need_z1 {
        return low;
    }
    let x01 = lerp_lanes::<V>(c[4], c[5], u);
    let x11 = lerp_lanes::<V>(c[6], c[7], u);
    lerp_lanes::<V>(low, lerp_lanes::<V>(x01, x11, v), w)
}
/// N 画素の 4 オクターブの値ノイズ（`fractal` と同じ値）。
///
/// # Safety
/// `V` の命令を持つ CPU で、その命令を有効にした `#[target_feature]` 付きの入口の中から呼ぶ。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) unsafe fn fractal_lanes<V: Lanes>(
    mut p: [V::F; 3],
    seeds: [u32; 4],
    cache: &mut Cache,
) -> V::F {
    let mut sum = V::splat(0.);
    let mut weight = 1.;
    let mut total = 0.;
    let two = V::splat(2.);
    for (seed, cell) in seeds.into_iter().zip(cache.cells.iter_mut()) {
        sum = V::add(
            sum,
            V::mul(V::splat(weight), value_lanes::<V>(p, seed, cell)),
        );
        total += weight;
        p = [V::mul(p[0], two), V::mul(p[1], two), V::mul(p[2], two)];
        weight *= 0.5;
    }
    V::div(sum, V::splat(total))
}

/// 1 画素ずつ呼ぶ呼び手用の 4 オクターブの値ノイズ（値は `fractal` と同じ）。格子の覚えはスレッドごとに持ち、同じ `seeds` の呼びどうしで
/// 使い回すので、画素を隣へ進める呼びでは同じ格子の中の hash を引き直さない。`seeds` が変われば覚えを捨てる。
pub(super) fn fractal_pixel(p: [f64; 3], seeds: [u32; 4]) -> f64 {
    use std::cell::RefCell;
    thread_local! {
        static STATE: RefCell<([u32; 4], Cache)> = const { RefCell::new(([0; 4], Cache::new())) };
    }
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.0 != seeds {
            *state = (seeds, Cache::new());
        }
        fractal(p, seeds, &mut state.1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 引き直しの無い素直な式（格子の値を使い回さない）。
    fn plain(p: [f64; 3], seed: u32) -> f64 {
        let (fx, fy, fz) = (p[0].floor(), p[1].floor(), p[2].floor());
        let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
        let (u, v, w) = (fade(p[0] - fx), fade(p[1] - fy), fade(p[2] - fz));
        let hx0 = hash(seed ^ ix as u32);
        let hx1 = hash(seed ^ ix.wrapping_add(1) as u32);
        let c00 = lerp(lattice(hx0, iy, iz), lattice(hx1, iy, iz), u);
        let c10 = lerp(lattice(hx0, iy + 1, iz), lattice(hx1, iy + 1, iz), u);
        let c01 = lerp(lattice(hx0, iy, iz + 1), lattice(hx1, iy, iz + 1), u);
        let c11 = lerp(
            lattice(hx0, iy + 1, iz + 1),
            lattice(hx1, iy + 1, iz + 1),
            u,
        );
        lerp(lerp(c00, c10, v), lerp(c01, c11, v), w)
    }
    fn plain_fractal(mut p: [f64; 3], seeds: [u32; 4]) -> f64 {
        let (mut sum, mut weight, mut total) = (0., 1., 0.);
        for seed in seeds {
            sum += weight * plain(p, seed);
            total += weight;
            p = p.map(|v| v * 2.);
            weight *= 0.5;
        }
        sum / total
    }

    /// 同じ格子に入る画素・別の格子に入る画素・z が整数（補間の重み 0）の画素の混ざった並びで、使い回しの有無によらず同じビットになる。
    #[test]
    fn cached_noise_equals_the_plain_formula_bit_for_bit() {
        let seeds = seeds(19381);
        let mut cache = Cache::new();
        let mut state = 7u32;
        let mut next = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f64 / (1u32 << 24) as f64
        };
        let mut checked = 0;
        // 隣り合う画素（ほとんど同じ格子）と、とびとびの画素（ほとんど別の格子）、z が 0 の面（UV の空間）
        for case in 0..3 {
            for i in 0..4000 {
                let t = i as f64;
                let p = match case {
                    0 => [t * 0.013 - 8., 5.3 + t * 0.002, -1.7 + t * 0.0007],
                    1 => [next() * 40. - 20., next() * 40. - 20., next() * 40. - 20.],
                    _ => [t * 0.021, 3.0 + (i / 500) as f64 * 0.37, 0.],
                };
                assert_eq!(
                    fractal(p, seeds, &mut cache).to_bits(),
                    plain_fractal(p, seeds).to_bits(),
                    "{p:?}"
                );
                assert_eq!(
                    fractal_pixel(p, seeds).to_bits(),
                    plain_fractal(p, seeds).to_bits(),
                    "{p:?}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 12000);
    }

    /// z の重みが 0 の画素のあとに、同じ格子で重みが 0 でない画素が来ても、z + 1 の側を引き直して同じ値になる。
    #[test]
    fn a_cell_filled_without_the_upper_corners_is_completed_when_needed() {
        let seeds = seeds(5);
        let mut cache = Cache::new();
        for z in [0., 0.5, 0., 0.25, 0.0001] {
            let p = [3.3, 4.4, z];
            assert_eq!(
                fractal(p, seeds, &mut cache).to_bits(),
                plain_fractal(p, seeds).to_bits(),
                "{p:?}"
            );
        }
    }

    /// SIMD の道（AVX2・SSE4.1・NEON）の N 画素ぶんの値は、1 画素の式とビットまで同じ。同じ格子に入る組・またぐ組・z が整数の組・負の座標・
    /// 大きな座標を通る。
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[test]
    fn lane_noise_equals_the_plain_formula_on_every_simd_level() {
        #[allow(clippy::needless_range_loop)]
        unsafe fn check<V: Lanes>() {
            let mut state = 0x9e37_79b9u32;
            let mut next = move || {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) as f64 / (1u32 << 24) as f64
            };
            for seed in [0u32, 19381, 0xdead_beef] {
                let seeds = seeds(seed as i32);
                let mut cache = Cache::new();
                for case in 0..5 {
                    for i in 0..1200 {
                        let t = i as f64;
                        let mut at = [[0.; 4]; 3];
                        for k in 0..V::N {
                            let t = t * V::N as f64 + k as f64;
                            let x = match case {
                                0 => t * 0.0011 - 3.,
                                1 => next() * 60. - 30.,
                                2 => t * 0.013,
                                3 => -50. + t * 0.0021,
                                _ => (t * 0.07).floor() * 0.5 + (k as f64) * 1e-9,
                            };
                            let y = match case {
                                0 => 5.3 + t * 0.0004,
                                1 => next() * 60. - 30.,
                                2 => 3.0 + (i / 400) as f64 * 0.37,
                                3 => 7.7,
                                _ => 2.5,
                            };
                            let z = match case {
                                0 => -1.7 + t * 0.0001,
                                1 => next() * 60. - 30.,
                                2 | 4 => 0.,
                                _ => 1.25,
                            };
                            for (axis, v) in [x, y, z].into_iter().enumerate() {
                                at[axis][k] = v;
                            }
                        }
                        let p = [
                            V::load_f64(&at[0]),
                            V::load_f64(&at[1]),
                            V::load_f64(&at[2]),
                        ];
                        let mut got = [0.; 4];
                        V::store_f64(&mut got, fractal_lanes::<V>(p, seeds, &mut cache));
                        for k in 0..V::N {
                            let want = plain_fractal([at[0][k], at[1][k], at[2][k]], seeds);
                            assert_eq!(
                                got[k].to_bits(),
                                want.to_bits(),
                                "case {case} lane {k} {:?}",
                                [at[0][k], at[1][k], at[2][k]]
                            );
                        }
                    }
                }
            }
        }
        crate::math::simd::on_each_level!(check);
    }
}
