use super::{
    check,
    ids::{find, union},
    input::{cross, dot, length, sub},
    surface::Surface,
    Result,
};
use std::collections::HashMap;
#[derive(Clone, Copy)]
struct Edge {
    first: usize,
    second: usize,
    count: usize,
    forward: bool,
    inconsistent: bool,
}
#[derive(Clone, Copy)]
struct Segment {
    a: [f64; 3],
    b: [f64; 3],
    phi: f64,
    component: usize,
}
pub(crate) struct Curvature {
    radius: f64,
    cell: f64,
    norm: f64,
    min: [f64; 3],
    segments: Vec<Segment>,
    cells: HashMap<i64, (usize, usize)>,
    pub components: Vec<usize>,
    pub boundary: usize,
    pub non_manifold: usize,
    pub inconsistent: usize,
}
fn cell_key(v: [i64; 3]) -> i64 {
    (v[0] * (1 << 21) + v[1]) * (1 << 21) + v[2]
}
impl Curvature {
    pub fn new(surface: &Surface, radius: f64, remaining: u64) -> Result<Self> {
        let i = surface.input;
        let n = i.triangle_count();
        let tol = (i.diagonal * 1e-6).max(1e-9);
        let mut weld = HashMap::new();
        let mut vertex = vec![];
        for p in i.corners.chunks_exact(3) {
            let key: [i64; 3] =
                std::array::from_fn(|a| (p[a] as f64 / tol).round_ties_even() as i64);
            let next = weld.len();
            vertex.push(*weld.entry(key).or_insert(next));
        }
        let mut edges: HashMap<(usize, usize), Edge> = HashMap::new();
        let mut order = vec![];
        for t in 0..n {
            if !surface.valid[t] {
                continue;
            }
            for e in 0..3 {
                let a = vertex[t * 3 + e];
                let b = vertex[t * 3 + (e + 1) % 3];
                if a == b {
                    continue;
                }
                let key = (a.min(b), a.max(b));
                if let Some(r) = edges.get_mut(&key) {
                    r.count += 1;
                    if r.count == 2 {
                        r.second = t;
                        r.inconsistent = r.forward == (a < b);
                    }
                } else {
                    edges.insert(
                        key,
                        Edge {
                            first: t,
                            second: 0,
                            count: 1,
                            forward: a < b,
                            inconsistent: false,
                        },
                    );
                    order.push(key);
                }
            }
        }
        let corner = |t: usize, id: usize| -> [f64; 3] {
            let c = (0..3).find(|e| vertex[t * 3 + e] == id).unwrap();
            std::array::from_fn(|a| i.corners[t * 9 + c * 3 + a] as f64)
        };
        let mut parent: Vec<_> = (0..n).collect();
        let mut curved = vec![];
        let mut total = 0u64;
        let mut boundary = 0;
        let mut non_manifold = 0;
        let mut inconsistent = 0;
        for key in order {
            let e = edges[&key];
            if e.count == 1 {
                boundary += 1;
                continue;
            }
            if e.count > 2 {
                non_manifold += 1;
                continue;
            }
            if e.inconsistent {
                inconsistent += 1;
                continue;
            }
            union(&mut parent, e.first, e.second);
            let n1 = surface.face[e.first].map(f64::from);
            let n2 = surface.face[e.second].map(f64::from);
            let mut phi = length(cross(n1, n2)).atan2(dot(n1, n2));
            let far = (0..3).find(|c| {
                let id = vertex[e.second * 3 + c];
                id != key.0 && id != key.1
            });
            let Some(far) = far else {
                continue;
            };
            let a = corner(e.first, key.0);
            let b = corner(e.first, key.1);
            let p = std::array::from_fn(|j| i.corners[e.second * 9 + far * 3 + j] as f64);
            if dot(n1, sub(p, a)) > 0. {
                phi = -phi;
            }
            if phi.abs() < 1e-6 {
                continue;
            }
            let pieces = (length(sub(b, a)) / radius).ceil().max(1.) as u64;
            total = total.saturating_add(pieces);
            curved.push((a, b, phi, e.first, pieces));
        }
        check(
            total <= i32::MAX as u64 / 8 && total.saturating_mul(76) <= remaining,
            "曲率の線分が残りのベイク予算を超えます",
        )?;
        let components: Vec<_> = (0..n).map(|t| find(&mut parent, t)).collect();
        let cell = 1.5 * radius;
        let mut raw = Vec::with_capacity(total as usize);
        for (a, b, phi, t, pieces) in curved {
            for j in 0..pieces {
                let t0 = j as f64 / pieces as f64;
                let t1 = (j + 1) as f64 / pieces as f64;
                let aa = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t0);
                let bb = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t1);
                let key = cell_key(std::array::from_fn(|k| {
                    (((aa[k] + bb[k]) * 0.5 - i.min[k]) / cell).floor() as i64
                }));
                raw.push((
                    key,
                    raw.len(),
                    Segment {
                        a: aa,
                        b: bb,
                        phi,
                        component: components[t],
                    },
                ));
            }
        }
        raw.sort_by_key(|(key, index, _)| (*key, *index));
        let mut cells: HashMap<i64, (usize, usize)> = HashMap::new();
        let mut segments = vec![];
        for (key, _, s) in raw {
            let n = segments.len();
            cells.entry(key).or_insert((n, 0)).1 += 1;
            segments.push(s);
        }
        Ok(Self {
            radius,
            cell,
            norm: std::f64::consts::PI / 2. * (16. / 15.) * radius,
            min: i.min,
            segments,
            cells,
            components,
            boundary,
            non_manifold,
            inconsistent,
        })
    }
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }
    pub fn evaluate(&self, p: [f64; 3], component: usize) -> f64 {
        if self.cells.is_empty() {
            return 0.;
        }
        let reach = 1.5 * self.radius;
        let r2 = self.radius * self.radius;
        let inv = 1. / r2;
        let lo: [i64; 3] = std::array::from_fn(|a| {
            (((p[a] - reach - self.min[a]) / self.cell).floor() as i64).max(0)
        });
        let hi: [i64; 3] =
            std::array::from_fn(|a| ((p[a] + reach - self.min[a]) / self.cell).floor() as i64);
        let mut sum = 0.;
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    let Some(&(start, n)) = self.cells.get(&cell_key([x, y, z])) else {
                        continue;
                    };
                    for s in &self.segments[start..start + n] {
                        if s.component != component {
                            continue;
                        }
                        let m = sub(s.a, p);
                        let e = sub(s.b, s.a);
                        let ee = dot(e, e);
                        let me = dot(m, e);
                        let mm = dot(m, m);
                        if ee <= 0. {
                            continue;
                        }
                        let disc = me * me - ee * (mm - r2);
                        if disc <= 0. {
                            continue;
                        }
                        let root = disc.sqrt();
                        let t0 = ((-me - root) / ee).max(0.);
                        let t1 = ((-me + root) / ee).min(1.);
                        if t1 <= t0 {
                            continue;
                        }
                        let a = 1. - mm * inv;
                        let b = -2. * me * inv;
                        let c = -ee * inv;
                        let integral = antiderivative(a, b, c, t1) - antiderivative(a, b, c, t0);
                        sum += s.phi * integral * ee.sqrt();
                    }
                }
            }
        }
        sum / self.norm
    }
}
fn antiderivative(a: f64, b: f64, c: f64, t: f64) -> f64 {
    let t2 = t * t;
    let t3 = t2 * t;
    a * a * t
        + a * b * t2
        + (b * b + 2. * a * c) * t3 / 3.
        + b * c * t2 * t2 / 2.
        + c * c * t3 * t2 / 5.
}
