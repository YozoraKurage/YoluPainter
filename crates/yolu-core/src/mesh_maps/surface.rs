use super::{
    input::{cross, dot, length, sub},
    MeshBakeInput,
};
use std::collections::HashMap;
pub(crate) fn interpolate(values: &[f32], t: usize, u: f64, v: f64) -> [f64; 3] {
    let k = t * 9;
    let w = 1. - u - v;
    std::array::from_fn(|a| {
        w * values[k + a] as f64 + u * values[k + 3 + a] as f64 + v * values[k + 6 + a] as f64
    })
}
pub(crate) struct Surface<'a> {
    pub input: &'a MeshBakeInput,
    pub face: Vec<[f32; 3]>,
    pub valid: Vec<bool>,
}
impl<'a> Surface<'a> {
    pub fn new(input: &'a MeshBakeInput) -> Self {
        let mut face = vec![];
        let mut valid = vec![];
        for t in 0..input.triangle_count() {
            let (normal, has_area) = face_normal(&input.corners, t);
            valid.push(has_area);
            face.push(normal);
        }
        Self { input, face, valid }
    }
    pub fn point(&self, t: usize, u: f64, v: f64) -> ([f64; 3], [f64; 3], [f64; 3]) {
        let p = interpolate(&self.input.corners, t, u, v);
        let g = self.face[t].map(f64::from);
        let mut n = g;
        if let Some(normals) = &self.input.normals {
            let s = interpolate(normals, t, u, v);
            let l = length(s);
            if l > 1e-6 && dot(s, g) > 0. {
                n = s.map(|a| a / l);
            }
        }
        (p, g, n)
    }
}
pub fn reconstruct_normals(corners: &[f32], crease_degrees: f64) -> super::Result<Vec<f32>> {
    super::check(
        corners.len().is_multiple_of(9)
            && corners.iter().all(|v| v.is_finite())
            && (0. ..=180.).contains(&crease_degrees),
        "法線再構築の入力が不正です",
    )?;
    if corners.is_empty() {
        return Ok(vec![]);
    }
    let mut min = [f64::MAX; 3];
    let mut max = [f64::MIN; 3];
    for p in corners.chunks_exact(3) {
        for a in 0..3 {
            min[a] = min[a].min(p[a] as f64);
            max[a] = max[a].max(p[a] as f64);
        }
    }
    let tol = (length(sub(max, min)) * 1e-6).max(1e-9);
    let count = corners.len() / 9;
    let mut face = vec![[0.; 3]; count];
    let mut angle = vec![0.; count * 3];
    for (t, f) in face.iter_mut().enumerate() {
        let k = t * 9;
        let e1 = std::array::from_fn(|a| (corners[k + 3 + a] - corners[k + a]) as f64);
        let e2 = std::array::from_fn(|a| (corners[k + 6 + a] - corners[k + a]) as f64);
        let n = cross(e1, e2);
        let l = length(n);
        if l <= 1e-30 {
            continue;
        }
        *f = n.map(|v| v / l);
        for c in 0..3 {
            let a = k + c * 3;
            let b = k + (c + 1) % 3 * 3;
            let d = k + (c + 2) % 3 * 3;
            let u = std::array::from_fn(|i| (corners[b + i] - corners[a + i]) as f64);
            let v = std::array::from_fn(|i| (corners[d + i] - corners[a + i]) as f64);
            angle[t * 3 + c] = length(cross(u, v)).atan2(dot(u, v));
        }
    }
    let mut groups: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    let keys: Vec<[i64; 3]> = corners
        .chunks_exact(3)
        .map(|p| std::array::from_fn(|a| (p[a] as f64 / tol).round_ties_even() as i64))
        .collect();
    for (i, &k) in keys.iter().enumerate() {
        groups.entry(k).or_default().push(i);
    }
    let limit = (crease_degrees * std::f64::consts::PI / 180.).cos() - 1e-9;
    let mut out = vec![0.; corners.len()];
    for (i, key) in keys.iter().enumerate() {
        let f = face[i / 3];
        if f == [0.; 3] {
            continue;
        }
        let mut sum = [0.; 3];
        for &other in &groups[key] {
            let n = face[other / 3];
            if dot(n, f) < limit {
                continue;
            }
            for a in 0..3 {
                sum[a] += n[a] * angle[other];
            }
        }
        let mut l = length(sum);
        if l <= 1e-30 {
            sum = f;
            l = 1.;
        }
        for a in 0..3 {
            out[i * 3 + a] = (sum[a] / l) as f32;
        }
    }
    Ok(out)
}
pub(crate) struct Frames {
    t: Vec<f32>,
    b: Vec<f32>,
    n: Vec<f32>,
    /// 接線が無く UV から作った三角形の数。
    pub fallback: usize,
}
impl Frames {
    pub fn new(s: &Surface, receivers: &[usize]) -> Self {
        let input = s.input;
        let mut out = Self {
            t: vec![0.; input.corners.len()],
            b: vec![0.; input.corners.len()],
            n: vec![0.; input.corners.len()],
            fallback: 0,
        };
        for &t in receivers {
            let k = t * 9;
            let q = t * 6;
            let has = input
                .tangents
                .as_ref()
                .is_some_and(|v| (0..3).any(|c| (0..3).any(|a| v[t * 12 + c * 4 + a] != 0.)));
            let mut tangent = [0.; 3];
            let mut sign = 1.;
            if !has {
                out.fallback += 1;
                let e1: [f64; 3] = std::array::from_fn(|a| {
                    (input.corners[k + 3 + a] - input.corners[k + a]) as f64
                });
                let e2: [f64; 3] = std::array::from_fn(|a| {
                    (input.corners[k + 6 + a] - input.corners[k + a]) as f64
                });
                let du1 = (input.uvs[q + 2] - input.uvs[q]) as f64;
                let dv1 = (input.uvs[q + 3] - input.uvs[q + 1]) as f64;
                let du2 = (input.uvs[q + 4] - input.uvs[q]) as f64;
                let dv2 = (input.uvs[q + 5] - input.uvs[q + 1]) as f64;
                let r = du1 * dv2 - du2 * dv1;
                let r = if r.abs() > 1e-30 { 1. / r } else { 0. };
                tangent = std::array::from_fn(|a| (e1[a] * dv2 - e2[a] * dv1) * r);
                let v = std::array::from_fn(|a| (e2[a] * du1 - e1[a] * du2) * r);
                if dot(cross(s.face[t].map(f64::from), tangent), v) < 0. {
                    sign = -1.;
                }
            }
            for c in 0..3 {
                let o = k + c * 3;
                let mut n = s.face[t].map(f64::from);
                if let Some(normals) = &input.normals {
                    let sn = std::array::from_fn(|a| normals[o + a] as f64);
                    let l = length(sn);
                    if l > 1e-6 {
                        n = sn.map(|v| v / l);
                    }
                }
                let (tangent, w) = if has {
                    let v = input.tangents.as_ref().unwrap();
                    (
                        std::array::from_fn(|a| v[t * 12 + c * 4 + a] as f64),
                        if v[t * 12 + c * 4 + 3] < 0. { -1. } else { 1. },
                    )
                } else {
                    let d = dot(tangent, n);
                    let mut v = std::array::from_fn(|a| tangent[a] - n[a] * d);
                    let l = length(v);
                    if l > 1e-30 {
                        v = v.map(|v| v / l);
                    }
                    (v, sign)
                };
                let b = cross(n, tangent).map(|v| v * w);
                for a in 0..3 {
                    out.t[o + a] = tangent[a] as f32;
                    out.b[o + a] = b[a] as f32;
                    out.n[o + a] = n[a] as f32;
                }
            }
        }
        out
    }
    /// 三角形ごとの接線・従接線・法線（角ごとに 3 つ、9 個）。受け手でない三角形は 0。
    pub fn arrays(&self) -> (&[f32], &[f32], &[f32]) {
        (&self.t, &self.b, &self.n)
    }
    pub fn tangent_space(&self, t: usize, u: f64, v: f64, m: [f64; 3]) -> [f64; 3] {
        let tv = interpolate(&self.t, t, u, v);
        let b = interpolate(&self.b, t, u, v);
        let n = interpolate(&self.n, t, u, v);
        let bn = cross(b, n);
        let det = dot(tv, bn);
        let value = if det.abs() > 1e-12 {
            [
                dot(m, bn) / det,
                dot(tv, cross(m, n)) / det,
                dot(tv, cross(b, m)) / det,
            ]
        } else {
            [dot(m, tv), dot(m, b), dot(m, n)]
        };
        let l = length(value);
        if l > 1e-12 {
            value.map(|v| v / l)
        } else {
            [0., 0., 1.]
        }
    }
}

pub(crate) fn face_normal(corners: &[f32], t: usize) -> ([f32; 3], bool) {
    let k = t * 9;
    let e1 = std::array::from_fn(|a| (corners[k + 3 + a] - corners[k + a]) as f64);
    let e2 = std::array::from_fn(|a| (corners[k + 6 + a] - corners[k + a]) as f64);
    let n = cross(e1, e2);
    let l = length(n);
    (
        if l > 1e-30 {
            n.map(|v| (v / l) as f32)
        } else {
            [0.; 3]
        },
        l > 1e-30,
    )
}
