use super::{
    input::{dot, length},
    MeshBakeSettings, MeshMapKind, MeshOcclusionFalloff, MeshRayBvh,
};
use std::f64::consts::PI;
pub(crate) struct Rays {
    ao: Vec<[f64; 3]>,
    th: Vec<[f64; 3]>,
    ao_cos2: f64,
    th_cos2: f64,
    offset: f64,
    ao_max: f64,
    th_max: f64,
    want_ao: bool,
    want_th: bool,
    any: bool,
    back: bool,
}
fn sequence(count: i32) -> Vec<[f64; 3]> {
    (0..count)
        .map(|i| {
            let phi = 2. * PI * ((i as u32).reverse_bits() as f64 * 2.3283064365386963e-10);
            [(i as f64 + 0.5) / count as f64, phi.cos(), phi.sin()]
        })
        .collect()
}
fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^= x >> 16;
    x
}
/// レイの方向列と設定（GPU へ渡す形。`Rays::trace` と同じ値）。
#[derive(Clone, Debug)]
pub struct MeshRayScene {
    /// 方向の列: [(i + 0.5) / n, cos φ, sin φ]。
    pub ao: Vec<[f64; 3]>,
    pub thickness: Vec<[f64; 3]>,
    pub ao_cos2: f64,
    pub thickness_cos2: f64,
    pub offset: f64,
    pub ao_max: f64,
    pub thickness_max: f64,
    pub want_ao: bool,
    pub want_thickness: bool,
    pub any_hit: bool,
    pub ignore_backfaces: bool,
}
impl Rays {
    pub fn scene(&self) -> MeshRayScene {
        MeshRayScene {
            ao: self.ao.clone(),
            thickness: self.th.clone(),
            ao_cos2: self.ao_cos2,
            thickness_cos2: self.th_cos2,
            offset: self.offset,
            ao_max: self.ao_max,
            thickness_max: self.th_max,
            want_ao: self.want_ao,
            want_thickness: self.want_th,
            any_hit: self.any,
            ignore_backfaces: self.back,
        }
    }
    pub fn new(s: &MeshBakeSettings, diagonal: f64) -> Self {
        let cos2 = |spread: f64| {
            let half = spread * 0.5 * PI / 180.;
            if spread >= 180. {
                0.
            } else {
                half.cos() * half.cos()
            }
        };
        Self {
            ao: sequence(s.ao_samples),
            th: sequence(s.thickness_samples),
            ao_cos2: cos2(s.ao_spread_degrees),
            th_cos2: cos2(s.thickness_spread_degrees),
            offset: 1e-5 * diagonal,
            ao_max: s.ao_max_distance * diagonal,
            th_max: s.thickness_max_distance * diagonal,
            want_ao: s.includes(MeshMapKind::AmbientOcclusion)
                || s.includes(MeshMapKind::BentNormal),
            want_th: s.includes(MeshMapKind::Thickness),
            any: s.ao_falloff == MeshOcclusionFalloff::None,
            back: s.ao_ignore_backfaces,
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn trace(
        &self,
        bvh: &MeshRayBvh,
        p: [f64; 3],
        n: [f64; 3],
        g: [f64; 3],
        triangle: usize,
        x: usize,
        y: usize,
        sub: usize,
    ) -> (f64, f64, [f64; 3], u64) {
        let h = hash(
            (x as u32).wrapping_mul(0x9e3779b1)
                ^ hash((y as u32).wrapping_add(0x68e31da4))
                ^ (sub as u32).wrapping_mul(0x85ebca6b),
        );
        let shift = (h >> 8) as f64 * (1. / 16777216.);
        let h2 = hash(h ^ 0xb5297a4d);
        let angle = 2. * PI * ((h2 >> 8) as f64 * (1. / 16777216.));
        let rc = angle.cos();
        let rs = angle.sin();
        let sign = if n[2] >= 0. { 1. } else { -1. };
        let a = -1. / (sign + n[2]);
        let bb = n[0] * n[1] * a;
        let t = [1. + sign * n[0] * n[0] * a, sign * bb, -sign * n[0]];
        let q = [bb, sign + n[1] * n[1] * a, -n[1]];
        let direction = |sample: [f64; 3], cos2: f64, inward: bool| {
            let mut u = sample[0] + shift;
            if u >= 1. {
                u -= 1.;
            }
            let cos2 = 1. - u * (1. - cos2);
            let ct = cos2.sqrt();
            let st = if cos2 < 1. { (1. - cos2).sqrt() } else { 0. };
            let cp = sample[1] * rc - sample[2] * rs;
            let sp = sample[2] * rc + sample[1] * rs;
            let lx = st * cp;
            let ly = st * sp;
            std::array::from_fn(|a| {
                if inward {
                    t[a] * lx + q[a] * ly - n[a] * ct
                } else {
                    t[a] * lx + q[a] * ly + n[a] * ct
                }
            })
        };
        let mut rays = 0;
        let mut ao = 0.;
        let mut thickness = 0.;
        let mut bent = n;
        if self.want_ao {
            let origin = std::array::from_fn(|a| p[a] + g[a] * self.offset);
            let mut occlusion = 0.;
            let mut sum = [0.; 3];
            let mut valid = 0;
            for &sample in &self.ao {
                let d = direction(sample, self.ao_cos2, false);
                if dot(d, g) <= 1e-4 {
                    continue;
                }
                valid += 1;
                match bvh.trace(origin, d, self.ao_max, Some(triangle), self.any, self.back) {
                    Some(h) => {
                        occlusion += if self.any {
                            1.
                        } else {
                            1. - h.distance / self.ao_max
                        }
                    }
                    None => {
                        for a in 0..3 {
                            sum[a] += d[a];
                        }
                    }
                }
            }
            rays += valid;
            ao = if valid > 0 {
                1. - occlusion / valid as f64
            } else {
                1.
            };
            let l = length(sum);
            if l > 1e-12 {
                bent = sum.map(|v| v / l);
            }
        }
        if self.want_th {
            let origin = std::array::from_fn(|a| p[a] - g[a] * self.offset);
            let mut distance = 0.;
            let mut valid = 0;
            for &sample in &self.th {
                let d = direction(sample, self.th_cos2, true);
                if -dot(d, g) <= 1e-4 {
                    continue;
                }
                valid += 1;
                distance += bvh
                    .trace(origin, d, self.th_max, Some(triangle), false, false)
                    .map_or(self.th_max, |h| h.distance.min(self.th_max));
            }
            rays += valid;
            thickness = if valid > 0 {
                distance / valid as f64 / self.th_max
            } else {
                1.
            };
        }
        (ao, thickness, bent, rays)
    }
}
