//! グランジのプリセット。ノイズの層（[`Layer`]）としきい値（smoothstep）・三角波の組み合わせで、値は 1 が「汚れ・傷などがある」。
//! 式は + − × ÷ sqrt floor と整数だけ。座標 `b` は基本のセル（`Procedural::scale`）が 1 の単位。
use super::{
    noisefn::{cell_hash, hash_unit, sin_cos_deg, unit24, wrap},
    procedural::{FractalMode::*, GrungePreset, Layer, NoiseBasis::*, Plan},
};

#[inline]
fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}
/// 整数で 1、半分で 0 の三角波。
#[inline]
fn tri(x: f64) -> f64 {
    ((x - x.floor()) * 2. - 1.).abs()
}

pub(super) fn eval(p: &Plan, b: [f64; 3], preset: GrungePreset) -> f64 {
    match preset {
        GrungePreset::Stain => stain(p, b),
        GrungePreset::Rust => rust(p, b),
        GrungePreset::Scratches => scratches(p, b),
        GrungePreset::Dust => dust(p, b),
        GrungePreset::Fingerprints => fingerprints(p, b),
        GrungePreset::Weave => weave(p, b),
        GrungePreset::Cracks => cracks(p, b),
        GrungePreset::Splatter => splatter(p, b),
        GrungePreset::Peeling => peeling(p, b),
        GrungePreset::WoodGrain => wood_grain(p, b),
        GrungePreset::Pebbles => pebbles(p, b),
    }
}

fn stain(p: &Plan, b: [f64; 3]) -> f64 {
    let a = p.layer(b, &Layer::new(Value, Fbm, 5, 0.7, 0));
    let c = p.layer(b, &Layer::new(Perlin, Fbm, 4, 2.2, 1));
    let g = p.layer(b, &Layer::new(Value, Fbm, 2, 24., 2));
    smooth(0.38, 0.78, a * 0.6 + c * 0.4) * (0.7 + 0.6 * g)
}

fn rust(p: &Plan, b: [f64; 3]) -> f64 {
    let patch = p.layer(b, &Layer::new(Perlin, Fbm, 5, 0.8, 0));
    let blot = p.layer(b, &Layer::new(Perlin, Ridged, 4, 4., 1));
    let pits = 1. - smooth(0., 0.2, p.layer(b, &Layer::new(Worley, Fbm, 1, 12., 2)));
    let spread = smooth(0.42, 0.60, patch);
    let halo = smooth(0.32, 0.46, patch) * 0.3 * blot;
    (spread * (0.45 + 0.55 * blot) + 0.4 * spread * pits).max(halo)
}

/// 傷の筋: セルごとに向き・長さ・太さが乱数の線分（端が細る）。2D の模様（位置ではトライプラナー）。`len` は枠の単位（セルの間隔が 1）の
/// 半分の長さ、`width` は基本のセルの単位の太さ。
fn segments(p: &Plan, b: [f64; 3], freq: f64, slot: u32, chance: f64, len: f64, width: f64) -> f64 {
    let f = p.frame(b, [freq, freq, 0.], slot, 0);
    let (ix, iy) = (f.p[0].floor() as i32, f.p[1].floor() as i32);
    let mut best: f64 = 0.;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (cx, cy) = (ix + dx, iy + dy);
            let h = cell_hash(f.seed, wrap(cx, f.per[0]), wrap(cy, f.per[1]), 5);
            if hash_unit(h ^ 0x0a5b_3c91) >= chance {
                continue;
            }
            let center = [
                cx as f64 + unit24(h),
                cy as f64 + hash_unit(h ^ 0x68bc_21eb),
            ];
            let (s, c) = sin_cos_deg(hash_unit(h ^ 0x02e5_be93) * 180.);
            let half = len * (0.4 + 0.6 * hash_unit(h ^ 0x9e37_79b1));
            let w = width * freq * (0.6 + 0.8 * hash_unit(h ^ 0x85eb_ca6b));
            let (rx, ry) = (f.p[0] - center[0], f.p[1] - center[1]);
            let (along, perp) = ((c * rx + s * ry).abs(), c * ry - s * rx);
            let dist = if along <= half {
                perp.abs()
            } else {
                ((along - half) * (along - half) + perp * perp).sqrt()
            };
            let taper = 1. - smooth(half * 0.5, half, along);
            let v = (1. - smooth(w * 0.4, w, dist)) * (0.5 + 0.5 * taper);
            best = best.max(v * (0.55 + 0.45 * hash_unit(h ^ 0x27d4_eb2f)));
        }
    }
    best
}
fn scratches(p: &Plan, b: [f64; 3]) -> f64 {
    let wear = smooth(0.25, 0.6, p.layer(b, &Layer::new(Perlin, Fbm, 3, 0.8, 9)));
    let long = segments(p, b, 1., 0, 0.7, 0.9, 0.016);
    let mid = segments(p, b, 2.3, 1, 0.75, 0.8, 0.009);
    let short = segments(p, b, 5., 2, 0.7, 0.6, 0.005);
    long.max(mid).max(short) * (0.35 + 0.65 * wear)
}

fn dust(p: &Plan, b: [f64; 3]) -> f64 {
    let fine = p.layer(b, &Layer::new(Value, Fbm, 3, 18., 0));
    let m = p.layer(b, &Layer::new(Perlin, Fbm, 4, 1.2, 1));
    let cover = smooth(0.30, 0.65, m);
    cover * (0.3 + 0.7 * smooth(0.5, 0.85, fine))
}

fn fingerprints(p: &Plan, b: [f64; 3]) -> f64 {
    let f = p.frame(b, [0.6, 0.6, 0.], 20, 0);
    let c = p.cells(&f);
    let d = [f.p[0] - c.point[0], f.p[1] - c.point[1]];
    let (s, co) = sin_cos_deg(unit24(c.id) * 180.);
    let dx = co * d[0] + s * d[1];
    let dy = (co * d[1] - s * d[0]) * 1.3;
    let r = (dx * dx + dy * dy).sqrt();
    let n1 = p.layer(b, &Layer::new(Perlin, Fbm, 3, 3., 1)) - 0.5;
    let n2 = p.layer(b, &Layer::new(Perlin, Fbm, 2, 4., 2)) - 0.5;
    let vis = smooth(0.25, 0.6, p.layer(b, &Layer::new(Perlin, Fbm, 3, 2., 3)));
    let ridge = smooth(0.30, 0.55, tri(r * 11. + n1 * 2.4));
    let patch = 1. - smooth(0.22, 0.46, r + 0.1 * n2);
    ridge * patch * (0.5 + 0.5 * vis)
}

fn weave(p: &Plan, b: [f64; 3]) -> f64 {
    // 1 つの基本のセルに 6 本（偶数。UV では周期が奇数にならず、市松の向きが巻く）
    const T: f64 = 6.;
    let (x, y) = (b[0] * T, b[1] * T);
    let (jx, jy) = (x.floor(), y.floor());
    let (fx, fy) = (x - jx, y - jy);
    let (jx, jy) = (jx as i32, jy as i32);
    let per = p.period(T as i32);
    let prof = |f: f64| 1. - (2. * f - 1.) * (2. * f - 1.);
    let over_vertical = (jx + jy).rem_euclid(2) == 0;
    let (over, thread) = if over_vertical {
        (prof(fx), p.lattice(21, jx, 0, per))
    } else {
        (prof(fy), p.lattice(22, 0, jy, per))
    };
    let fiber = p.layer(b, &Layer::new(Value, Fbm, 2, T * 5., 0));
    (0.3 + 0.7 * over) * (0.8 + 0.2 * thread) * (0.88 + 0.24 * fiber)
}

fn cracks(p: &Plan, b: [f64; 3]) -> f64 {
    let w = |slot| (p.layer(b, &Layer::new(Perlin, Fbm, 2, 1.5, slot)) - 0.5) * 0.45;
    let wb = [
        b[0] + w(0),
        b[1] + w(1),
        if p.is_uv() { b[2] } else { b[2] + w(2) },
    ];
    let c = p.cells(&p.frame(wb, [1.2; 3], 3, 0));
    let width = 0.012 + 0.02 * p.layer(b, &Layer::new(Perlin, Fbm, 2, 3., 4));
    let main = 1. - smooth(width * 0.4, width * 1.2, c.f2 - c.f1);
    let c2 = p.cells(&p.frame(wb, [3.4; 3], 5, 0));
    let fine = (1. - smooth(0.01, 0.03, c2.f2 - c2.f1))
        * smooth(0.4, 0.6, p.layer(b, &Layer::new(Perlin, Fbm, 2, 2., 6)));
    main.max(0.6 * fine)
}

fn drops(p: &Plan, b: [f64; 3], freq: f64, slot: u32, biggest: f64) -> f64 {
    let c = p.cells(&p.frame(b, [freq; 3], slot, 0));
    let (r1, r2) = (unit24(c.id), hash_unit(c.id ^ 0x7f4a_7c15));
    if r2 < 0.45 {
        return 0.;
    }
    let radius = 0.04 + biggest * r1 * r1;
    1. - smooth(radius * 0.75, radius, c.f1)
}
fn splatter(p: &Plan, b: [f64; 3]) -> f64 {
    let big = drops(p, b, 1., 0, 0.32);
    let mid = drops(p, b, 3.1, 1, 0.3);
    let small = drops(p, b, 9., 2, 0.28) * 0.9;
    big.max(mid).max(small)
}

/// 塗装の剥げ: 剥げの場の値（大きいほど剥げる）。既定のレベル（`GrungePreset::default_levels`）が境目を作り、レベルを動かすと
/// 剥げの割合が変わる。
fn peeling(p: &Plan, b: [f64; 3]) -> f64 {
    let base = p.layer(b, &Layer::new(Perlin, Fbm, 5, 1., 0));
    let chip = p.layer(b, &Layer::new(Perlin, Ridged, 3, 5., 1));
    base + 0.12 * (chip - 0.5)
}

fn wood_grain(p: &Plan, b: [f64; 3]) -> f64 {
    // 年輪: 位置の空間では局所の y 軸のまわり、UV では x 方向の縞（周期が整数の本数になる）
    let r = if p.is_uv() {
        b[0] * 4.
    } else {
        (b[0] * b[0] + b[2] * b[2] * 0.85).sqrt() * 2.5
    };
    let warp = (p.layer(b, &Layer::new(Perlin, Fbm, 3, 0.45, 0)) - 0.5) * 1.6
        + (p.layer(b, &Layer::new(Value, Fbm, 2, 6., 1)) - 0.5) * 0.25;
    let rings = r + warp;
    let f = rings - rings.floor();
    let ring = if f < 0.78 { f / 0.78 } else { (1. - f) / 0.22 };
    let fiber = p.layer(b, &Layer::new(Value, Fbm, 3, 1., 2).aniso([9., 0.5, 9.]));
    (0.2 + 0.8 * ring) * (0.8 + 0.4 * fiber)
}

fn pebbles(p: &Plan, b: [f64; 3]) -> f64 {
    let c = p.cells(&p.frame(b, [1.; 3], 0, 0));
    let border = smooth(0., 0.14, c.f2 - c.f1);
    let dome = 1. - smooth(0., 0.6, c.f1);
    border * (0.5 + 0.5 * dome) * (0.82 + 0.18 * unit24(c.id))
}
