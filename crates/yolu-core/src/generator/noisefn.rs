//! 手続き型のノイズの基底（値・Perlin・Worley）。+ − × ÷ sqrt floor と整数だけで書き、libm（sin・cos・pow・exp）を使わない
//! （環境が違ってもバイトが変わらない）。`per` が 0 でなければ x・y の格子をその周期で巻く（UV の継ぎ目の無い評価）。
//!
//! 格子の角の値は hash だけで決まる（座標には依らない）ので、同じ格子に入る画素どうしでは使い回せる。基底ごとの `*Cell` が直前の
//! 格子とその角の値を持ち、同じ格子なら hash を引き直さない（違う格子に入れば引き直す）。値は使い回しの有無で変わらない
//! （試験で、使い回さない素直な式とビットを比べる）。
//!
//! SIMD の道（`*_lanes`）は N 画素を 1 組で引く。N 画素が同じ格子に入れば角の値（特徴点）を全部のレーンで共有してレーンで補間し、
//! またぐ組は 1 画素ずつ引く。どちらも 1 画素の式と同じ演算を同じ順に並べたもので、結果のビットは変わらない。
pub(super) use super::noise::hash;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use super::noise::{fade_lanes, lerp_lanes};
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use crate::math::simd::{self, Lanes};

/// 1 つのレイヤーの最大のオクターブ数。
pub const MAX_OCTAVES: u32 = 8;

/// 度の sin・cos（多項式。libm を使わない）。`deg` は有限であること（検査済みの入力）。
pub(crate) fn sin_cos_deg(deg: f64) -> (f64, f64) {
    let q = (deg / 90. + 0.5).floor();
    let r = (deg - 90. * q) * (std::f64::consts::PI / 180.);
    let r2 = r * r;
    let sin = r
        * (1.
            + r2 * (-1. / 6.
                + r2 * (1. / 120.
                    + r2 * (-1. / 5040.
                        + r2 * (1. / 362880.
                            + r2 * (-1. / 39916800.
                                + r2 * (1. / 6227020800.
                                    + r2 * (-1. / 1307674368000.
                                        + r2 * (1. / 355687428096000.)))))))));
    let cos = 1.
        + r2 * (-1. / 2.
            + r2 * (1. / 24.
                + r2 * (-1. / 720.
                    + r2 * (1. / 40320.
                        + r2 * (-1. / 3628800.
                            + r2 * (1. / 479001600.
                                + r2 * (-1. / 87178291200. + r2 * (1. / 20922789888000.))))))));
    match (q as i64).rem_euclid(4) {
        0 => (sin, cos),
        1 => (cos, -sin),
        2 => (-sin, -cos),
        _ => (-cos, sin),
    }
}

#[inline]
pub(super) fn wrap(i: i32, period: i32) -> i32 {
    if period > 0 {
        i.rem_euclid(period)
    } else {
        i
    }
}
const KX: u32 = 0x9e37_79b1;
const KY: u32 = 0x85eb_ca77;
const KZ: u32 = 0xc2b2_ae3d;
#[inline]
pub(super) fn cell_hash(seed: u32, x: i32, y: i32, z: i32) -> u32 {
    let mut h = hash(seed ^ (x as u32).wrapping_mul(KX));
    h = hash(h ^ (y as u32).wrapping_mul(KY));
    hash(h ^ (z as u32).wrapping_mul(KZ))
}
/// ハッシュを [0, 1) の値に。
#[inline]
pub(super) fn unit24(h: u32) -> f64 {
    (h >> 8) as f64 * (1. / 16777216.)
}
/// 整数をハッシュして [0, 1) の値に。
#[inline]
pub(super) fn hash_unit(h: u32) -> f64 {
    unit24(hash(h))
}
#[inline(always)]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6. - 15.) + 10.)
}
#[inline(always)]
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// 格子 `(ix, iy, iz)` の 8 つの角（x が速く、次に y、次に z）の `cell_hash`。x・y は周期 `per` で巻き、z は巻かない。
/// 共通の前半の hash を使い回す（角ごとに 3 回引くと 24 回、ここでは 14 回）。
#[inline(always)]
fn corner_hashes(seed: u32, ix: i32, iy: i32, iz: i32, per: [i32; 2]) -> [u32; 8] {
    let hx = [0, 1].map(|d| hash(seed ^ (wrap(ix + d, per[0]) as u32).wrapping_mul(KX)));
    let ys = [0, 1].map(|d| (wrap(iy + d, per[1]) as u32).wrapping_mul(KY));
    let hxy = [
        [hash(hx[0] ^ ys[0]), hash(hx[0] ^ ys[1])],
        [hash(hx[1] ^ ys[0]), hash(hx[1] ^ ys[1])],
    ];
    let zs = [0, 1].map(|d| ((iz + d) as u32).wrapping_mul(KZ));
    let mut h = [0; 8];
    for dz in 0..2 {
        for dy in 0..2 {
            for dx in 0..2 {
                h[dz * 4 + dy * 2 + dx] = hash(hxy[dx][dy] ^ zs[dz]);
            }
        }
    }
    h
}

/// 直前の格子の整数の座標。
type Key = [i32; 3];

/// 値のノイズの、直前の格子の角の値。
#[derive(Clone, Copy)]
pub(super) struct ValueCell {
    valid: bool,
    key: Key,
    c: [f64; 8],
}
impl ValueCell {
    pub(super) const EMPTY: Self = Self {
        valid: false,
        key: [0; 3],
        c: [0.; 8],
    };
}
/// 値のノイズ。0..1。
pub(super) fn value3(p: [f64; 3], seed: u32, per: [i32; 2], cell: &mut ValueCell) -> f64 {
    let (fx, fy, fz) = (p[0].floor(), p[1].floor(), p[2].floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (u, v, w) = (fade(p[0] - fx), fade(p[1] - fy), fade(p[2] - fz));
    if !(cell.valid && cell.key == [ix, iy, iz]) {
        cell.c = corner_hashes(seed, ix, iy, iz, per).map(unit24);
        cell.key = [ix, iy, iz];
        cell.valid = true;
    }
    let c = &cell.c;
    let x00 = lerp(c[0], c[1], u);
    let x10 = lerp(c[2], c[3], u);
    let x01 = lerp(c[4], c[5], u);
    let x11 = lerp(c[6], c[7], u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// 勾配の向き（`(h >> 20) & 15`）ごとの、使う 2 つの座標（0 = x、1 = y、2 = z）とその符号。`s1 * a + s2 * b` は元の `a + b`・`a - b`・
/// `-a - b` と同じ値（符号の反転と、引き算＝符号を反転した足し算は IEEE で厳密）。
const GRAD: [(usize, f64, usize, f64); 16] = [
    (0, 1., 1, 1.),   // x + y
    (1, 1., 0, -1.),  // y - x
    (0, 1., 1, -1.),  // x - y
    (0, -1., 1, -1.), // -x - y
    (0, 1., 2, 1.),   // x + z
    (2, 1., 0, -1.),  // z - x
    (0, 1., 2, -1.),  // x - z
    (0, -1., 2, -1.), // -x - z
    (1, 1., 2, 1.),   // y + z
    (2, 1., 1, -1.),  // z - y
    (1, 1., 2, -1.),  // y - z
    (1, -1., 2, -1.), // -y - z
    (0, 1., 1, 1.),   // 12: x + y
    (1, 1., 0, -1.),  // 13: y - x
    (2, 1., 1, -1.),  // 14: z - y
    (1, -1., 2, -1.), // 15: -y - z
];
/// 改良版の Perlin 勾配ノイズの値域（おおよそ ±1）を、0..1 の中央に広げる倍率。
const PERLIN_SPREAD: f64 = 1.25;

/// 勾配のノイズの、直前の格子の角の勾配の向き。
#[derive(Clone, Copy)]
pub(super) struct PerlinCell {
    valid: bool,
    key: Key,
    sel: [u8; 8],
}
impl PerlinCell {
    pub(super) const EMPTY: Self = Self {
        valid: false,
        key: [0; 3],
        sel: [0; 8],
    };
}
/// 勾配（Perlin）ノイズ。0..1（0.5 が中央）。
pub(super) fn perlin3(p: [f64; 3], seed: u32, per: [i32; 2], cell: &mut PerlinCell) -> f64 {
    let (fx, fy, fz) = (p[0].floor(), p[1].floor(), p[2].floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (x, y, z) = (p[0] - fx, p[1] - fy, p[2] - fz);
    let (u, v, w) = (fade(x), fade(y), fade(z));
    if !(cell.valid && cell.key == [ix, iy, iz]) {
        cell.sel = corner_hashes(seed, ix, iy, iz, per).map(|h| ((h >> 20) & 15) as u8);
        cell.key = [ix, iy, iz];
        cell.valid = true;
    }
    let g = |k: usize| {
        let (dx, dy, dz) = ((k & 1) as f64, ((k >> 1) & 1) as f64, (k >> 2) as f64);
        let c = [x - dx, y - dy, z - dz];
        let (i1, s1, i2, s2) = GRAD[cell.sel[k] as usize];
        s1 * c[i1] + s2 * c[i2]
    };
    let x00 = lerp(g(0), g(1), u);
    let x10 = lerp(g(2), g(3), u);
    let x01 = lerp(g(4), g(5), u);
    let x11 = lerp(g(6), g(7), u);
    let n = lerp(lerp(x00, x10, v), lerp(x01, x11, v), w);
    (0.5 + 0.5 * n * PERLIN_SPREAD).clamp(0., 1.)
}

/// 最も近い特徴点までの距離（`f1`）・2 番目（`f2`）・最も近いセルの ID・その特徴点の位置。
#[derive(Clone, Copy)]
pub(super) struct Cells {
    pub f1: f64,
    pub f2: f64,
    pub id: u32,
    pub point: [f64; 3],
}
/// セルの特徴点（位置と、そのセルのハッシュ）。
#[derive(Clone, Copy)]
struct Point {
    q: [f64; 3],
    h: u32,
}
/// Worley ノイズの、直前の格子とその周りの 27 セルの特徴点（z・y・x の順の走査と同じ並び）。
#[derive(Clone, Copy)]
pub(super) struct WorleyCell {
    valid: bool,
    key: Key,
    points: [Point; 27],
}
impl WorleyCell {
    pub(super) const EMPTY: Self = Self {
        valid: false,
        key: [0; 3],
        points: [Point { q: [0.; 3], h: 0 }; 27],
    };
}
fn fill_points(seed: u32, ix: i32, iy: i32, iz: i32, per: [i32; 2]) -> [Point; 27] {
    let hx = [-1, 0, 1].map(|d| hash(seed ^ (wrap(ix + d, per[0]) as u32).wrapping_mul(KX)));
    let ys = [-1, 0, 1].map(|d| (wrap(iy + d, per[1]) as u32).wrapping_mul(KY));
    let mut points = [Point { q: [0.; 3], h: 0 }; 27];
    for dz in -1..=1i32 {
        let zk = ((iz + dz) as u32).wrapping_mul(KZ);
        for dy in -1..=1i32 {
            for dx in -1..=1i32 {
                let hxy = hash(hx[(dx + 1) as usize] ^ ys[(dy + 1) as usize]);
                let h = hash(hxy ^ zk);
                points[((dz + 1) * 9 + (dy + 1) * 3 + (dx + 1)) as usize] = Point {
                    q: [
                        (ix + dx) as f64 + unit24(h),
                        (iy + dy) as f64 + unit24(hash(h ^ 0x68bc_21eb)),
                        (iz + dz) as f64 + unit24(hash(h ^ 0x02e5_be93)),
                    ],
                    h,
                };
            }
        }
    }
    points
}
/// Worley（セル）ノイズ。セルごとに 1 つの特徴点を置く（隣の 27 セルを調べる）。
pub(super) fn worley3(p: [f64; 3], seed: u32, per: [i32; 2], cell: &mut WorleyCell) -> Cells {
    let (fx, fy, fz) = (p[0].floor(), p[1].floor(), p[2].floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    if !(cell.valid && cell.key == [ix, iy, iz]) {
        cell.points = fill_points(seed, ix, iy, iz, per);
        cell.key = [ix, iy, iz];
        cell.valid = true;
    }
    let (mut d1, mut d2) = (f64::MAX, f64::MAX);
    let mut id = 0;
    let mut point = [0.; 3];
    for pt in &cell.points {
        let q = pt.q;
        let (a, b, c) = (q[0] - p[0], q[1] - p[1], q[2] - p[2]);
        let d = a * a + b * b + c * c;
        if d < d1 {
            d2 = d1;
            d1 = d;
            id = pt.h;
            point = q;
        } else if d < d2 {
            d2 = d;
        }
    }
    Cells {
        f1: d1.sqrt(),
        f2: d2.sqrt(),
        id,
        point,
    }
}

// ───────── SIMD の道（N 画素を 1 組で） ─────────

/// N 画素が全部同じ格子に入るなら、その格子の整数の座標（`floor` が違えば格子は違う。大きすぎて整数に収まらない座標で、違う `floor` が
/// 同じ整数になる場合も「違う」と見て 1 画素ずつ引くだけで、値は変わらない）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn same_cell<V: Lanes>(fx: V::F, fy: V::F, fz: V::F) -> Option<Key> {
    let mut floors = [[0.; 4]; 3];
    V::store_f64(&mut floors[0], fx);
    V::store_f64(&mut floors[1], fy);
    V::store_f64(&mut floors[2], fz);
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
/// N 画素を 1 画素ずつの式 `f` で引く（格子をまたぐ組）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn per_lane<V: Lanes>(p: [V::F; 3], mut f: impl FnMut([f64; 3]) -> f64) -> V::F {
    let mut at = [[0.; 4]; 3];
    for (axis, lanes) in at.iter_mut().enumerate() {
        V::store_f64(lanes, p[axis]);
    }
    let mut out = [0.; 4];
    for (k, o) in out.iter_mut().enumerate().take(V::N) {
        *o = f([at[0][k], at[1][k], at[2][k]]);
    }
    V::load_f64(&out)
}

/// N 画素の値のノイズ（`value3` と同じ値）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) unsafe fn value3_lanes<V: Lanes>(
    p: [V::F; 3],
    seed: u32,
    per: [i32; 2],
    cell: &mut ValueCell,
) -> V::F {
    let (fx, fy, fz) = (V::floor(p[0]), V::floor(p[1]), V::floor(p[2]));
    let Some(key) = same_cell::<V>(fx, fy, fz) else {
        return per_lane::<V>(p, |q| value3(q, seed, per, cell));
    };
    let (u, v, w) = (
        fade_lanes::<V>(V::sub(p[0], fx)),
        fade_lanes::<V>(V::sub(p[1], fy)),
        fade_lanes::<V>(V::sub(p[2], fz)),
    );
    if !(cell.valid && cell.key == key) {
        cell.c = corner_hashes(seed, key[0], key[1], key[2], per).map(unit24);
        cell.key = key;
        cell.valid = true;
    }
    let mut c = [V::splat(0.); 8];
    for (lane, value) in c.iter_mut().zip(cell.c) {
        *lane = V::splat(value);
    }
    let x00 = lerp_lanes::<V>(c[0], c[1], u);
    let x10 = lerp_lanes::<V>(c[2], c[3], u);
    let x01 = lerp_lanes::<V>(c[4], c[5], u);
    let x11 = lerp_lanes::<V>(c[6], c[7], u);
    lerp_lanes::<V>(
        lerp_lanes::<V>(x00, x10, v),
        lerp_lanes::<V>(x01, x11, v),
        w,
    )
}

/// N 画素の勾配（Perlin）ノイズ（`perlin3` と同じ値）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) unsafe fn perlin3_lanes<V: Lanes>(
    p: [V::F; 3],
    seed: u32,
    per: [i32; 2],
    cell: &mut PerlinCell,
) -> V::F {
    let (fx, fy, fz) = (V::floor(p[0]), V::floor(p[1]), V::floor(p[2]));
    let Some(key) = same_cell::<V>(fx, fy, fz) else {
        return per_lane::<V>(p, |q| perlin3(q, seed, per, cell));
    };
    let (x, y, z) = (V::sub(p[0], fx), V::sub(p[1], fy), V::sub(p[2], fz));
    let (u, v, w) = (fade_lanes::<V>(x), fade_lanes::<V>(y), fade_lanes::<V>(z));
    if !(cell.valid && cell.key == key) {
        cell.sel = corner_hashes(seed, key[0], key[1], key[2], per).map(|h| ((h >> 20) & 15) as u8);
        cell.key = key;
        cell.valid = true;
    }
    let one = V::splat(1.);
    // 角 (dx, dy, dz) からの距離（`x - dx`。dx = 0 は `x - 0.0 = x` なのでそのまま）
    let (x1, y1, z1) = (V::sub(x, one), V::sub(y, one), V::sub(z, one));
    let mut g = [V::splat(0.); 8];
    for (k, out) in g.iter_mut().enumerate() {
        let c = [
            if k & 1 == 0 { x } else { x1 },
            if k & 2 == 0 { y } else { y1 },
            if k & 4 == 0 { z } else { z1 },
        ];
        let (i1, s1, i2, s2) = GRAD[cell.sel[k] as usize];
        *out = V::add(V::mul(V::splat(s1), c[i1]), V::mul(V::splat(s2), c[i2]));
    }
    let x00 = lerp_lanes::<V>(g[0], g[1], u);
    let x10 = lerp_lanes::<V>(g[2], g[3], u);
    let x01 = lerp_lanes::<V>(g[4], g[5], u);
    let x11 = lerp_lanes::<V>(g[6], g[7], u);
    let n = lerp_lanes::<V>(
        lerp_lanes::<V>(x00, x10, v),
        lerp_lanes::<V>(x01, x11, v),
        w,
    );
    simd::clamp01::<V>(V::add(
        V::splat(0.5),
        V::mul(V::mul(V::splat(0.5), n), V::splat(PERLIN_SPREAD)),
    ))
}

/// `Cells` の N 画素ぶん。`id` は整数の値を持つ f64（`u32` を厳密に表せる）。
#[derive(Clone, Copy)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) struct CellsLanes<V: Lanes> {
    pub f1: V::F,
    pub f2: V::F,
    pub id: V::F,
    pub point: [V::F; 3],
}
/// 27 点から N 画素それぞれの、最寄りの距離²・2 番目の距離²・最寄りの点の番号（f64 で持つ）。点を走査順の 3 つの連続した範囲に分けて
/// 独立に走査し（更新が前の点に依る列が 3 本に割れて、並んで進む）、走査順の早い範囲から順に畳む。畳み方は、最寄りは小さい方、同じ距離なら
/// 早い範囲の点、2 番目は 4 つの値（各範囲の最寄りと 2 番目）の 2 番目に小さい値なので、1 本の列で `d < d1` の順に更新した結果と同じになる。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn nearest_lanes<V: Lanes>(points: &[Point; 27], p: [V::F; 3]) -> (V::F, V::F, V::F) {
    let start = (V::splat(f64::MAX), V::splat(f64::MAX), V::splat(0.));
    let mut parts = [start; 3];
    for step in 0..9 {
        for (part, acc) in parts.iter_mut().enumerate() {
            let k = part * 9 + step;
            let pt = &points[k];
            let (a, b, c) = (
                V::sub(V::splat(pt.q[0]), p[0]),
                V::sub(V::splat(pt.q[1]), p[1]),
                V::sub(V::splat(pt.q[2]), p[2]),
            );
            let d = V::add(V::add(V::mul(a, a), V::mul(b, b)), V::mul(c, c));
            let (below_first, below_second) = (V::lt(d, acc.0), V::lt(d, acc.1));
            // `if d < d1 { d2 = d1; d1 = d; 勝者 = このセル } else if d < d2 { d2 = d }`
            acc.1 = V::select(below_first, acc.0, V::select(below_second, d, acc.1));
            acc.0 = V::select(below_first, d, acc.0);
            acc.2 = V::select(below_first, V::splat(k as f64), acc.2);
        }
    }
    let mut merged = parts[0];
    for later in &parts[1..] {
        let later_first = V::lt(later.0, merged.0);
        merged = (
            V::select(later_first, later.0, merged.0),
            V::select(
                later_first,
                V::min(merged.0, later.1),
                V::min(merged.1, later.0),
            ),
            V::select(later_first, later.2, merged.2),
        );
    }
    merged
}
/// `nearest_lanes` の距離だけ版（勝者の点を持ち回らない）。最寄りは `min`、2 番目は `min(d2, max(d1, d))` で更新する（`d < d1` なら d1、
/// `d < d2` なら d、それ以外は d2 になるので、勝者を選ぶ版と同じ値）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn nearest_distances_lanes<V: Lanes>(points: &[Point; 27], p: [V::F; 3]) -> (V::F, V::F) {
    let start = (V::splat(f64::MAX), V::splat(f64::MAX));
    let mut parts = [start; 3];
    for step in 0..9 {
        for (part, acc) in parts.iter_mut().enumerate() {
            let pt = &points[part * 9 + step];
            let (a, b, c) = (
                V::sub(V::splat(pt.q[0]), p[0]),
                V::sub(V::splat(pt.q[1]), p[1]),
                V::sub(V::splat(pt.q[2]), p[2]),
            );
            let d = V::add(V::add(V::mul(a, a), V::mul(b, b)), V::mul(c, c));
            acc.1 = V::min(acc.1, V::max(acc.0, d));
            acc.0 = V::min(acc.0, d);
        }
    }
    let mut merged = parts[0];
    for later in &parts[1..] {
        let later_first = V::lt(later.0, merged.0);
        merged = (
            V::select(later_first, later.0, merged.0),
            V::select(
                later_first,
                V::min(merged.0, later.1),
                V::min(merged.1, later.0),
            ),
        );
    }
    merged
}
/// N 画素の Worley ノイズの最寄りと 2 番目の距離だけ（`worley3` の `f1`・`f2` と同じ値）。ID・点が要らない呼び手用で、その分だけ速い。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) unsafe fn worley_distances_lanes<V: Lanes>(
    p: [V::F; 3],
    seed: u32,
    per: [i32; 2],
    cell: &mut WorleyCell,
) -> (V::F, V::F) {
    let (fx, fy, fz) = (V::floor(p[0]), V::floor(p[1]), V::floor(p[2]));
    let Some(key) = same_cell::<V>(fx, fy, fz) else {
        let mut at = [[0.; 4]; 3];
        for (axis, lanes) in at.iter_mut().enumerate() {
            V::store_f64(lanes, p[axis]);
        }
        let mut out = [[0.; 4]; 2];
        for k in 0..V::N {
            let c = worley3([at[0][k], at[1][k], at[2][k]], seed, per, cell);
            out[0][k] = c.f1;
            out[1][k] = c.f2;
        }
        return (V::load_f64(&out[0]), V::load_f64(&out[1]));
    };
    if !(cell.valid && cell.key == key) {
        cell.points = fill_points(seed, key[0], key[1], key[2], per);
        cell.key = key;
        cell.valid = true;
    }
    let (d1, d2) = nearest_distances_lanes::<V>(&cell.points, p);
    (V::sqrt(d1), V::sqrt(d2))
}
/// N 画素の Worley ノイズ（`worley3` と同じ値・同じ ID・同じ点。同じ距離のセルは元の走査の順で先のものが勝つ）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) unsafe fn worley3_lanes<V: Lanes>(
    p: [V::F; 3],
    seed: u32,
    per: [i32; 2],
    cell: &mut WorleyCell,
) -> CellsLanes<V> {
    let (fx, fy, fz) = (V::floor(p[0]), V::floor(p[1]), V::floor(p[2]));
    let Some(key) = same_cell::<V>(fx, fy, fz) else {
        // 格子をまたぐ組: 1 画素ずつ
        let mut at = [[0.; 4]; 3];
        for (axis, lanes) in at.iter_mut().enumerate() {
            V::store_f64(lanes, p[axis]);
        }
        let mut out = [[0.; 4]; 6];
        for k in 0..V::N {
            let c = worley3([at[0][k], at[1][k], at[2][k]], seed, per, cell);
            out[0][k] = c.f1;
            out[1][k] = c.f2;
            out[2][k] = f64::from(c.id);
            out[3][k] = c.point[0];
            out[4][k] = c.point[1];
            out[5][k] = c.point[2];
        }
        return CellsLanes {
            f1: V::load_f64(&out[0]),
            f2: V::load_f64(&out[1]),
            id: V::load_f64(&out[2]),
            point: [
                V::load_f64(&out[3]),
                V::load_f64(&out[4]),
                V::load_f64(&out[5]),
            ],
        };
    };
    if !(cell.valid && cell.key == key) {
        cell.points = fill_points(seed, key[0], key[1], key[2], per);
        cell.key = key;
        cell.valid = true;
    }
    let (d1, d2, winner) = nearest_lanes::<V>(&cell.points, p);
    let mut which = [0.; 4];
    V::store_f64(&mut which, winner);
    let points = &cell.points;
    CellsLanes {
        f1: V::sqrt(d1),
        f2: V::sqrt(d2),
        id: V::from_fn(|k| f64::from(points[which[k] as usize].h)),
        point: [
            V::from_fn(|k| points[which[k] as usize].q[0]),
            V::from_fn(|k| points[which[k] as usize].q[1]),
            V::from_fn(|k| points[which[k] as usize].q[2]),
        ],
    }
}

/// N 画素の度の sin・cos（`sin_cos_deg` と同じ値）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) unsafe fn sin_cos_deg_lanes<V: Lanes>(deg: V::F) -> (V::F, V::F) {
    let q = V::floor(V::add(V::div(deg, V::splat(90.)), V::splat(0.5)));
    let r = V::mul(
        V::sub(deg, V::mul(V::splat(90.), q)),
        V::splat(std::f64::consts::PI / 180.),
    );
    let r2 = V::mul(r, r);
    // 多項式（ホーナー法。スカラーの式と同じ順）
    let mut sin = V::splat(1. / 355687428096000.);
    for c in [
        -1. / 1307674368000.,
        1. / 6227020800.,
        -1. / 39916800.,
        1. / 362880.,
        -1. / 5040.,
        1. / 120.,
        -1. / 6.,
        1.,
    ] {
        sin = V::add(V::splat(c), V::mul(r2, sin));
    }
    let sin = V::mul(r, sin);
    let mut cos = V::splat(1. / 20922789888000.);
    for c in [
        -1. / 87178291200.,
        1. / 479001600.,
        -1. / 3628800.,
        1. / 40320.,
        -1. / 720.,
        1. / 24.,
        -1. / 2.,
        1.,
    ] {
        cos = V::add(V::splat(c), V::mul(r2, cos));
    }
    // 象限 `q mod 4`（q は整数の値）
    let quarter = V::sub(q, V::mul(V::splat(4.), V::floor(V::mul(q, V::splat(0.25)))));
    let (is1, is2, is3) = (
        V::eq(quarter, V::splat(1.)),
        V::eq(quarter, V::splat(2.)),
        V::eq(quarter, V::splat(3.)),
    );
    let (neg_sin, neg_cos) = (V::neg(sin), V::neg(cos));
    (
        V::select(
            is3,
            neg_cos,
            V::select(is2, neg_sin, V::select(is1, cos, sin)),
        ),
        V::select(
            is3,
            sin,
            V::select(is2, neg_cos, V::select(is1, neg_sin, cos)),
        ),
    )
}
/// N 画素のハッシュ（整数の値を持つ f64）を [0, 1) の値に（`unit24` と同じ値）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) unsafe fn unit24_lanes<V: Lanes>(h: V::F) -> V::F {
    V::mul(
        V::floor(V::mul(h, V::splat(1. / 256.))),
        V::splat(1. / 16777216.),
    )
}

/// 2D の値のノイズ（`value3` の z = 0 の面と同じ値）と、その勾配（格子の単位。x・y の偏微分）。フィルターのスロープぼかし・ゆがみが
/// 読む向きを決める。格子を巻かない。
pub(crate) fn value2_gradient(x: f64, y: f64, seed: u32) -> (f64, [f64; 2]) {
    let (fx, fy) = (x.floor(), y.floor());
    let (ix, iy) = (fx as i32, fy as i32);
    let (tx, ty) = (x - fx, y - fy);
    let (u, v) = (fade(tx), fade(ty));
    let c = |dx: i32, dy: i32| unit24(cell_hash(seed, ix + dx, iy + dy, 0));
    let (c00, c10, c01, c11) = (c(0, 0), c(1, 0), c(0, 1), c(1, 1));
    let x0 = lerp(c00, c10, u);
    let x1 = lerp(c01, c11, u);
    // fade(t) = 6t⁵ − 15t⁴ + 10t³ の微分 30t²(t − 1)²
    let dfade = |t: f64| 30. * t * t * (t - 1.) * (t - 1.);
    let dx = dfade(tx) * lerp(c10 - c00, c11 - c01, v);
    let dy = dfade(ty) * (x1 - x0);
    (lerp(x0, x1, v), [dx, dy])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 使い回しの無い素直な式（格子の値を使い回さない。この最適化の前の実装そのまま）。
    mod plain {
        use super::*;
        #[inline]
        fn corner(seed: u32, x: i32, y: i32, z: i32, per: [i32; 2]) -> u32 {
            cell_hash(seed, wrap(x, per[0]), wrap(y, per[1]), z)
        }
        pub fn value3(p: [f64; 3], seed: u32, per: [i32; 2]) -> f64 {
            let (fx, fy, fz) = (p[0].floor(), p[1].floor(), p[2].floor());
            let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
            let (u, v, w) = (fade(p[0] - fx), fade(p[1] - fy), fade(p[2] - fz));
            let c = |dx, dy, dz| unit24(corner(seed, ix + dx, iy + dy, iz + dz, per));
            let x00 = lerp(c(0, 0, 0), c(1, 0, 0), u);
            let x10 = lerp(c(0, 1, 0), c(1, 1, 0), u);
            let x01 = lerp(c(0, 0, 1), c(1, 0, 1), u);
            let x11 = lerp(c(0, 1, 1), c(1, 1, 1), u);
            lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
        }
        fn grad(h: u32, x: f64, y: f64, z: f64) -> f64 {
            match (h >> 20) & 15 {
                0 | 12 => x + y,
                1 | 13 => y - x,
                2 => x - y,
                3 => -x - y,
                4 => x + z,
                5 => z - x,
                6 => x - z,
                7 => -x - z,
                8 => y + z,
                9 => z - y,
                10 => y - z,
                11 => -y - z,
                14 => z - y,
                _ => -y - z,
            }
        }
        pub fn perlin3(p: [f64; 3], seed: u32, per: [i32; 2]) -> f64 {
            let (fx, fy, fz) = (p[0].floor(), p[1].floor(), p[2].floor());
            let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
            let (x, y, z) = (p[0] - fx, p[1] - fy, p[2] - fz);
            let (u, v, w) = (fade(x), fade(y), fade(z));
            let g = |dx: i32, dy: i32, dz: i32| {
                grad(
                    corner(seed, ix + dx, iy + dy, iz + dz, per),
                    x - dx as f64,
                    y - dy as f64,
                    z - dz as f64,
                )
            };
            let x00 = lerp(g(0, 0, 0), g(1, 0, 0), u);
            let x10 = lerp(g(0, 1, 0), g(1, 1, 0), u);
            let x01 = lerp(g(0, 0, 1), g(1, 0, 1), u);
            let x11 = lerp(g(0, 1, 1), g(1, 1, 1), u);
            let n = lerp(lerp(x00, x10, v), lerp(x01, x11, v), w);
            (0.5 + 0.5 * n * PERLIN_SPREAD).clamp(0., 1.)
        }
        pub fn worley3(p: [f64; 3], seed: u32, per: [i32; 2]) -> Cells {
            let (fx, fy, fz) = (p[0].floor(), p[1].floor(), p[2].floor());
            let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
            let (mut d1, mut d2) = (f64::MAX, f64::MAX);
            let mut id = 0;
            let mut point = [0.; 3];
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (cx, cy, cz) = (ix + dx, iy + dy, iz + dz);
                        let h = corner(seed, cx, cy, cz, per);
                        let q = [
                            cx as f64 + unit24(h),
                            cy as f64 + unit24(hash(h ^ 0x68bc_21eb)),
                            cz as f64 + unit24(hash(h ^ 0x02e5_be93)),
                        ];
                        let (a, b, c) = (q[0] - p[0], q[1] - p[1], q[2] - p[2]);
                        let d = a * a + b * b + c * c;
                        if d < d1 {
                            d2 = d1;
                            d1 = d;
                            id = h;
                            point = q;
                        } else if d < d2 {
                            d2 = d;
                        }
                    }
                }
            }
            Cells {
                f1: d1.sqrt(),
                f2: d2.sqrt(),
                id,
                point,
            }
        }
    }

    #[test]
    fn value2_gradient_is_the_z0_value_noise_and_its_slope() {
        for (i, seed) in [0u32, 7, 0xdead_beef].into_iter().enumerate() {
            for k in 0..200 {
                let x = -13.7 + k as f64 * 0.173 + i as f64;
                let y = 5.1 - k as f64 * 0.091;
                let (n, g) = value2_gradient(x, y, seed);
                assert_eq!(
                    n.to_bits(),
                    plain::value3([x, y, 0.], seed, [0, 0]).to_bits()
                );
                // 勾配は差分と合う
                let e = 1e-6;
                let nx = (plain::value3([x + e, y, 0.], seed, [0, 0])
                    - plain::value3([x - e, y, 0.], seed, [0, 0]))
                    / (2. * e);
                let ny = (plain::value3([x, y + e, 0.], seed, [0, 0])
                    - plain::value3([x, y - e, 0.], seed, [0, 0]))
                    / (2. * e);
                assert!(
                    (g[0] - nx).abs() < 1e-5 && (g[1] - ny).abs() < 1e-5,
                    "{x} {y}"
                );
            }
        }
    }

    #[test]
    fn sin_cos_matches_the_library_closely() {
        let mut worst: f64 = 0.;
        for i in -3600..=3600 {
            let deg = i as f64 * 0.1;
            let (s, c) = sin_cos_deg(deg);
            let (rs, rc) = (deg.to_radians()).sin_cos();
            worst = worst.max((s - rs).abs()).max((c - rc).abs());
        }
        assert!(worst < 1e-13, "{worst}");
        assert_eq!(sin_cos_deg(0.), (0., 1.));
        assert_eq!(sin_cos_deg(90.).0, 1.);
    }
    #[test]
    fn periodic_noise_tiles() {
        let per = [5, 3];
        for seed in [1u32, 77] {
            for i in 0..20 {
                let p = [i as f64 * 0.37 + 0.11, i as f64 * 0.21 + 0.4, 0.];
                let q = [p[0] + 5., p[1] + 3., 0.];
                // 小数部は足し算の丸めで 1 ULP ずれ得るので、ビットでなく近さで比べる
                let (mut va, mut vb) = (ValueCell::EMPTY, ValueCell::EMPTY);
                assert!(
                    (value3(p, seed, per, &mut va) - value3(q, seed, per, &mut vb)).abs() < 1e-9
                );
                let (mut pa, mut pb) = (PerlinCell::EMPTY, PerlinCell::EMPTY);
                assert!(
                    (perlin3(p, seed, per, &mut pa) - perlin3(q, seed, per, &mut pb)).abs() < 1e-9
                );
                let (mut wa, mut wb) = (WorleyCell::EMPTY, WorleyCell::EMPTY);
                let (a, b) = (
                    worley3(p, seed, per, &mut wa),
                    worley3(q, seed, per, &mut wb),
                );
                assert!((a.f1 - b.f1).abs() < 1e-9 && (a.f2 - b.f2).abs() < 1e-9);
            }
        }
    }
    #[test]
    fn ranges_are_unit() {
        let (mut va, mut pa, mut wa) = (ValueCell::EMPTY, PerlinCell::EMPTY, WorleyCell::EMPTY);
        for i in 0..2000 {
            let p = [i as f64 * 0.173, i as f64 * 0.311 - 40., i as f64 * 0.093];
            for v in [
                value3(p, 9, [0, 0], &mut va),
                perlin3(p, 9, [0, 0], &mut pa),
            ] {
                assert!((0. ..=1.).contains(&v), "{v}");
            }
            let c = worley3(p, 9, [0, 0], &mut wa);
            assert!(c.f1 <= c.f2 && c.f1 >= 0.);
        }
    }

    /// 同じ格子に入る画素の並び（使い回しが効く）・とびとびの画素（引き直す）・UV の周期つき・負の座標で、使い回しの有無によらず
    /// 素直な式と同じビットになる。
    #[test]
    fn cached_primitives_equal_the_plain_formulas_bit_for_bit() {
        let mut state = 0x2545_f491u32;
        let mut next = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f64 / (1u32 << 24) as f64
        };
        let mut checked = 0;
        for per in [[0, 0], [7, 5], [64, 1]] {
            for seed in [0u32, 0xdead_beef, 12345] {
                let (mut va, mut pa, mut wa) =
                    (ValueCell::EMPTY, PerlinCell::EMPTY, WorleyCell::EMPTY);
                for case in 0..3 {
                    for i in 0..1500 {
                        let t = i as f64;
                        let p = match case {
                            0 => [t * 0.013 - 8., 5.3 + t * 0.002, -1.7 + t * 0.0007],
                            1 => [next() * 40. - 20., next() * 40. - 20., next() * 40. - 20.],
                            _ => [t * 0.021, 3.0 + (i / 500) as f64 * 0.37, 0.25],
                        };
                        assert_eq!(
                            value3(p, seed, per, &mut va).to_bits(),
                            plain::value3(p, seed, per).to_bits(),
                            "value {p:?} {per:?}"
                        );
                        assert_eq!(
                            perlin3(p, seed, per, &mut pa).to_bits(),
                            plain::perlin3(p, seed, per).to_bits(),
                            "perlin {p:?} {per:?}"
                        );
                        let (a, b) = (worley3(p, seed, per, &mut wa), plain::worley3(p, seed, per));
                        assert_eq!(a.f1.to_bits(), b.f1.to_bits(), "worley f1 {p:?} {per:?}");
                        assert_eq!(a.f2.to_bits(), b.f2.to_bits(), "worley f2 {p:?} {per:?}");
                        assert_eq!(a.id, b.id, "worley id {p:?} {per:?}");
                        assert_eq!(
                            a.point.map(f64::to_bits),
                            b.point.map(f64::to_bits),
                            "worley point {p:?} {per:?}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 3 * 3 * 3 * 1500);
    }

    /// SIMD の道（AVX2・SSE4.1・NEON）の N 画素ぶんの値・Worley の ID と点・sin/cos・ハッシュの [0, 1) は、1 画素の式とビットまで同じ。
    /// 同じ格子に入る組・またぐ組・UV の周期つき・負の座標を通る。
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[test]
    fn lane_primitives_equal_the_plain_formulas_on_every_simd_level() {
        #[allow(clippy::needless_range_loop)]
        unsafe fn check<V: Lanes>() {
            let mut state = 0x1357_9bdfu32;
            let mut next = move || {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) as f64 / (1u32 << 24) as f64
            };
            for per in [[0, 0], [7, 5]] {
                for seed in [0u32, 0xdead_beef] {
                    let (mut va, mut pa, mut wa) =
                        (ValueCell::EMPTY, PerlinCell::EMPTY, WorleyCell::EMPTY);
                    for case in 0..4 {
                        for i in 0..800 {
                            let mut at = [[0.; 4]; 3];
                            for k in 0..V::N {
                                let t = (i * V::N + k) as f64;
                                let x = match case {
                                    0 => t * 0.0013 - 3.,
                                    1 => next() * 40. - 20.,
                                    2 => t * 0.017,
                                    _ => -30. + t * 0.0031,
                                };
                                let y = match case {
                                    0 => 5.3 + t * 0.0005,
                                    1 => next() * 40. - 20.,
                                    2 => 3. + (i / 300) as f64 * 0.41,
                                    _ => 7.7,
                                };
                                let z = match case {
                                    0 => -1.7 + t * 0.0001,
                                    1 => next() * 40. - 20.,
                                    _ => 0.25,
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
                            let mut got = [[0.; 4]; 8];
                            V::store_f64(&mut got[0], value3_lanes::<V>(p, seed, per, &mut va));
                            V::store_f64(&mut got[1], perlin3_lanes::<V>(p, seed, per, &mut pa));
                            let c = worley3_lanes::<V>(p, seed, per, &mut wa);
                            V::store_f64(&mut got[2], c.f1);
                            V::store_f64(&mut got[3], c.f2);
                            V::store_f64(&mut got[4], c.id);
                            V::store_f64(&mut got[5], c.point[0]);
                            V::store_f64(&mut got[6], c.point[1]);
                            V::store_f64(&mut got[7], c.point[2]);
                            for k in 0..V::N {
                                let q = [at[0][k], at[1][k], at[2][k]];
                                let w = plain::worley3(q, seed, per);
                                let want = [
                                    plain::value3(q, seed, per),
                                    plain::perlin3(q, seed, per),
                                    w.f1,
                                    w.f2,
                                    f64::from(w.id),
                                    w.point[0],
                                    w.point[1],
                                    w.point[2],
                                ];
                                for (which, (g, e)) in got.iter().zip(want).enumerate() {
                                    assert_eq!(
                                        g[k].to_bits(),
                                        e.to_bits(),
                                        "case {case} lane {k} 種類 {which} {q:?} {per:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
            // sin・cos と unit24 は値の並びを直接比べる（度は −400〜800、ハッシュは 32 ビットの全域から）
            let mut degs = Vec::new();
            for i in 0..4000 {
                degs.push(-400. + i as f64 * 0.3);
                degs.push(next() * 180.);
            }
            degs.extend([0., 90., 180., 270., 360., -90., 45.5, 719.99]);
            for chunk in degs.chunks(V::N) {
                if chunk.len() < V::N {
                    continue;
                }
                let (s, c) = sin_cos_deg_lanes::<V>(V::load_f64(chunk));
                let (mut ss, mut cc) = ([0.; 4], [0.; 4]);
                V::store_f64(&mut ss, s);
                V::store_f64(&mut cc, c);
                for k in 0..V::N {
                    let (ws, wc) = sin_cos_deg(chunk[k]);
                    assert_eq!(ss[k].to_bits(), ws.to_bits(), "sin {}", chunk[k]);
                    assert_eq!(cc[k].to_bits(), wc.to_bits(), "cos {}", chunk[k]);
                }
            }
            for i in 0..5000u32 {
                let hs: Vec<u32> = (0..V::N)
                    .map(|k| match i % 3 {
                        0 => hash(i * 7 + k as u32),
                        1 => u32::MAX - i * 13 - k as u32,
                        _ => (i << 8) + k as u32,
                    })
                    .collect();
                let lanes: Vec<f64> = hs.iter().map(|h| f64::from(*h)).collect();
                let mut got = [0.; 4];
                V::store_f64(&mut got, unit24_lanes::<V>(V::load_f64(&lanes)));
                for k in 0..V::N {
                    assert_eq!(
                        got[k].to_bits(),
                        unit24(hs[k]).to_bits(),
                        "unit24 {}",
                        hs[k]
                    );
                }
            }
        }
        crate::math::simd::on_each_level!(check);
    }

    /// 27 点の探索を 3 つの範囲に割って畳んでも、1 本の列で走査した結果と同じ（同じ距離が走査の順で先の点に勝つ・2 番目が重なる場合を含む）。
    /// 座標を粗い格子の上に置いて、同じ距離・同じ最寄りを大量に作る。
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[test]
    fn partitioned_nearest_search_equals_the_sequential_scan_including_ties() {
        #[allow(clippy::needless_range_loop)]
        unsafe fn check<V: Lanes>() {
            let mut state = 0x7777_1234u32;
            let mut next = move |n: u32| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) % n
            };
            let mut ties = 0;
            for _ in 0..4000 {
                let coarse = 2 + next(4);
                let grid = |v: u32| f64::from(v) / f64::from(coarse);
                let mut points = [Point { q: [0.; 3], h: 0 }; 27];
                for (k, pt) in points.iter_mut().enumerate() {
                    pt.q = [
                        grid(next(coarse * 3)) - 1.,
                        grid(next(coarse * 3)) - 1.,
                        grid(next(coarse * 3)) - 1.,
                    ];
                    pt.h = k as u32;
                }
                let mut at = [[0.; 4]; 3];
                for k in 0..V::N {
                    for axis in 0..3 {
                        at[axis][k] = grid(next(coarse * 2));
                    }
                }
                let p = [
                    V::load_f64(&at[0]),
                    V::load_f64(&at[1]),
                    V::load_f64(&at[2]),
                ];
                let (d1, d2, winner) = nearest_lanes::<V>(&points, p);
                let (only1, only2) = nearest_distances_lanes::<V>(&points, p);
                let (mut a, mut b, mut w) = ([0.; 4], [0.; 4], [0.; 4]);
                let (mut c, mut e) = ([0.; 4], [0.; 4]);
                V::store_f64(&mut a, d1);
                V::store_f64(&mut b, d2);
                V::store_f64(&mut w, winner);
                V::store_f64(&mut c, only1);
                V::store_f64(&mut e, only2);
                for k in 0..V::N {
                    // 元の走査（1 本の列）
                    let (mut e1, mut e2, mut ew) = (f64::MAX, f64::MAX, 0usize);
                    let mut same = 0;
                    for (j, pt) in points.iter().enumerate() {
                        let (x, y, z) =
                            (pt.q[0] - at[0][k], pt.q[1] - at[1][k], pt.q[2] - at[2][k]);
                        let d = x * x + y * y + z * z;
                        if d == e1 {
                            same += 1;
                        }
                        if d < e1 {
                            e2 = e1;
                            e1 = d;
                            ew = j;
                        } else if d < e2 {
                            e2 = d;
                        }
                    }
                    ties += same;
                    assert_eq!(a[k].to_bits(), e1.to_bits(), "最寄り");
                    assert_eq!(b[k].to_bits(), e2.to_bits(), "2 番目");
                    assert_eq!(c[k].to_bits(), e1.to_bits(), "最寄り（距離だけ）");
                    assert_eq!(e[k].to_bits(), e2.to_bits(), "2 番目（距離だけ）");
                    assert_eq!(w[k] as usize, ew, "勝者");
                }
            }
            assert!(ties > 500, "同じ距離の場合が足りない: {ties}");
        }
        crate::math::simd::on_each_level!(check);
    }
}
