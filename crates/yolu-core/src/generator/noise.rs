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
fn lattice(hx: u32, y: i32, z: i32) -> f64 {
    (hash(hash(hx ^ y as u32) ^ z as u32) >> 8) as f64 * (1. / 16777215.)
}
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6. - 15.) + 10.)
}
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}
fn value(x: f64, y: f64, z: f64, seed: u32) -> f64 {
    let (fx, fy, fz) = (x.floor(), y.floor(), z.floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (u, v, w) = (fade(x - fx), fade(y - fy), fade(z - fz));
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
pub(super) fn fractal(mut p: [f64; 3], seeds: [u32; 4]) -> f64 {
    let mut sum = 0.;
    let mut weight = 1.;
    let mut total = 0.;
    for seed in seeds {
        sum += weight * value(p[0], p[1], p[2], seed);
        total += weight;
        p = p.map(|v| v * 2.);
        weight *= 0.5;
    }
    sum / total
}
