//! 手続き型のノイズの基底（値・Perlin・Worley）。+ − × ÷ sqrt floor と整数だけで書き、libm（sin・cos・pow・exp）を使わない
//! （環境が違ってもバイトが変わらない）。`per` が 0 でなければ x・y の格子をその周期で巻く（UV の継ぎ目の無い評価）。
pub(super) use super::noise::hash;

/// 1 つの層の最大のオクターブ数。
pub const MAX_OCTAVES: u32 = 8;

/// 度の sin・cos（多項式。libm を使わない）。`deg` は有限であること（検査済みの入力）。
pub(super) fn sin_cos_deg(deg: f64) -> (f64, f64) {
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
#[inline]
pub(super) fn cell_hash(seed: u32, x: i32, y: i32, z: i32) -> u32 {
    let mut h = hash(seed ^ (x as u32).wrapping_mul(0x9e37_79b1));
    h = hash(h ^ (y as u32).wrapping_mul(0x85eb_ca77));
    hash(h ^ (z as u32).wrapping_mul(0xc2b2_ae3d))
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
#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6. - 15.) + 10.)
}
#[inline]
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}
#[inline]
fn corner(seed: u32, x: i32, y: i32, z: i32, per: [i32; 2]) -> u32 {
    cell_hash(seed, wrap(x, per[0]), wrap(y, per[1]), z)
}

/// 値のノイズ。0..1。
pub(super) fn value3(p: [f64; 3], seed: u32, per: [i32; 2]) -> f64 {
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

#[inline]
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
/// 改良版の Perlin 勾配ノイズの値域（おおよそ ±1）を、0..1 の中央に広げる倍率。
const PERLIN_SPREAD: f64 = 1.25;

/// 勾配（Perlin）ノイズ。0..1（0.5 が中央）。
pub(super) fn perlin3(p: [f64; 3], seed: u32, per: [i32; 2]) -> f64 {
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

/// 最も近い特徴点までの距離（`f1`）・2 番目（`f2`）・最も近いセルの ID・その特徴点の位置。
#[derive(Clone, Copy)]
pub(super) struct Cells {
    pub f1: f64,
    pub f2: f64,
    pub id: u32,
    pub point: [f64; 3],
}
/// Worley（セル）ノイズ。セルごとに 1 つの特徴点を置く（隣の 27 セルを調べる）。
pub(super) fn worley3(p: [f64; 3], seed: u32, per: [i32; 2]) -> Cells {
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

#[cfg(test)]
mod tests {
    use super::*;

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
                for f in [value3, perlin3] {
                    // 小数部は足し算の丸めで 1 ULP ずれ得るので、ビットでなく近さで比べる
                    assert!((f(p, seed, per) - f(q, seed, per)).abs() < 1e-9);
                }
                let (a, b) = (worley3(p, seed, per), worley3(q, seed, per));
                assert!((a.f1 - b.f1).abs() < 1e-9 && (a.f2 - b.f2).abs() < 1e-9);
            }
        }
    }
    #[test]
    fn ranges_are_unit() {
        for i in 0..2000 {
            let p = [i as f64 * 0.173, i as f64 * 0.311 - 40., i as f64 * 0.093];
            for v in [value3(p, 9, [0, 0]), perlin3(p, 9, [0, 0])] {
                assert!((0. ..=1.).contains(&v), "{v}");
            }
            let c = worley3(p, 9, [0, 0]);
            assert!(c.f1 <= c.f2 && c.f1 >= 0.);
        }
    }
}
