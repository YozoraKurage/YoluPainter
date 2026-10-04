//! 3D ビューの環境（Unity 版の `PreviewEnvironment` と同じ作り）: 元（内蔵の空・スタジオ）を 1 面 [`FACE_SIZE`] のキューブの写しにし、粗さの段
//! （Unity の映り込みと同じ mip 0〜6）ごとに GGX で畳み込んだキューブ（[`Baked::mips`]。マテリアル表示の映り込みと中立の表示の映り込みと
//! 背景のぼかしに使う）と、拡散の環境光の SH（[`Baked::sh`]。Unity の SphericalHarmonicsL2 と同じ並びと式）を作る。
//!
//! 焼きは CPU で行う（元が変わったときだけ。回転と明るさは焼かずに使う所で掛ける）。GPU で畳み込む案は、パイプラインが増えて GPU の機種ごとの
//! 差が試験に入るので採らなかった: CPU なら決定的で、SH・畳み込み・向きの表を GPU なしの試験で確かめられる。失ったのは、巨大な HDRI
//! を焼き直すときの速さ（今の元は式で書けるので 128² の写しで足りる）。
//!
//! 向きの表: キューブの面は 0 +X・1 −X・2 +Y・3 −Y・4 +Z・5 −Z、行 0 が上（t = −1 の側）。方向 d = Z + s·X + t·Y（`face_basis`）は
//! GL・D3D・Vulkan のキューブの表（+X 面は sc = −rz、tc = −ry）と同じで、wgpu の `texture_cube` の参照と一致する。

use half::f16;
use rayon::prelude::*;
use yolu_core::glam::Vec3;

use super::brdf::{roughness_of_mip, srgb_to_linear, REFLECTION_STEPS};

/// 写しの 1 面の画素数（mip 0）。mip i は `FACE_SIZE >> i`。
pub const FACE_SIZE: u32 = 128;
/// mip の数（Unity の映り込みの粗さの段 0〜6）。
pub const MIP_COUNT: u32 = REFLECTION_STEPS + 1;
/// 畳み込みの標本の数（Hammersley）。
const SAMPLES: u32 = 64;
const PI: f32 = std::f32::consts::PI;

/// 内蔵の空の色（色の欄の値 = sRGB。天頂・地平線・地面）。既定は Unity 版と同じ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyColors {
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    pub ground: [f32; 3],
}

impl Default for SkyColors {
    fn default() -> Self {
        SkyColors {
            zenith: [0.42, 0.55, 0.75],
            horizon: [0.86, 0.87, 0.88],
            ground: [0.24, 0.23, 0.22],
        }
    }
}

/// 環境の元。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    /// 内蔵の手続きの空（天頂・地平線・地面の勾配）。上の軸のまわりに対称なので、回転は見た目を変えない。
    Sky(SkyColors),
    /// 内蔵のスタジオ（暗い灰色の背景に、正面の右上の大きな面光源・左の弱い面光源・後ろのふち取りの細い光源）。金属の映り込みと回転が分かる。
    Studio,
}

impl Source {
    /// 向き d（単位ベクトル）の放射輝度（リニア）。
    pub fn radiance(&self, d: Vec3) -> Vec3 {
        match self {
            Source::Sky(c) => sky(c, d),
            Source::Studio => studio(d),
        }
    }
}

fn lerp3(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a + (b - a) * t
}

fn srgb3(c: [f32; 3]) -> Vec3 {
    Vec3::new(
        srgb_to_linear(c[0]),
        srgb_to_linear(c[1]),
        srgb_to_linear(c[2]),
    )
}

/// Unity 版の `Sky`（`PreviewEnvironment.shader`）。
fn sky(c: &SkyColors, d: Vec3) -> Vec3 {
    let (zenith, horizon, ground) = (srgb3(c.zenith), srgb3(c.horizon), srgb3(c.ground));
    let y = d.y;
    if y >= 0.0 {
        lerp3(horizon, zenith, y.clamp(0.0, 1.0).sqrt())
    } else {
        lerp3(horizon, ground, (-y * 6.0).clamp(0.0, 1.0))
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 面光源（方位・高さ・半分の角度の幅と高さ・放射輝度）。
struct Softbox {
    azimuth: f32,
    elevation: f32,
    half_width: f32,
    half_height: f32,
    radiance: Vec3,
}

const STUDIO_BOXES: [Softbox; 3] = [
    // 正面（Unity の +Z を向いたモデルをカメラの側 −Z から見る）の右上: 大きな面光源
    Softbox {
        azimuth: 150.0,
        elevation: 40.0,
        half_width: 0.45,
        half_height: 0.30,
        radiance: Vec3::new(7.0, 6.8, 6.4),
    },
    // 正面の左: 弱い冷たい面光源
    Softbox {
        azimuth: -135.0,
        elevation: 10.0,
        half_width: 0.60,
        half_height: 0.35,
        radiance: Vec3::new(1.1, 1.25, 1.5),
    },
    // 後ろ: ふち取りの縦の細い光源
    Softbox {
        azimuth: 10.0,
        elevation: 35.0,
        half_width: 0.12,
        half_height: 0.60,
        radiance: Vec3::new(4.5, 4.6, 5.0),
    },
];

fn studio(d: Vec3) -> Vec3 {
    let y = d.y.clamp(-1.0, 1.0);
    let wall = Vec3::new(0.10, 0.105, 0.115);
    let mut c = if y >= 0.0 {
        lerp3(wall, Vec3::new(0.20, 0.21, 0.23), y.sqrt())
    } else {
        lerp3(
            wall,
            Vec3::new(0.035, 0.033, 0.03),
            (-y * 4.0).clamp(0.0, 1.0),
        )
    };
    for b in &STUDIO_BOXES {
        let (az, el) = (b.azimuth.to_radians(), b.elevation.to_radians());
        let center = Vec3::new(az.sin() * el.cos(), el.sin(), az.cos() * el.cos());
        let right = Vec3::Y.cross(center).normalize();
        let up = center.cross(right);
        let z = d.dot(center);
        if z <= 0.0 {
            continue;
        }
        let ax = d.dot(right).atan2(z).abs();
        let ay = d.dot(up).atan2(z).abs();
        let mask = (1.0 - smoothstep(b.half_width * 0.75, b.half_width, ax))
            * (1.0 - smoothstep(b.half_height * 0.75, b.half_height, ay));
        c += b.radiance * mask;
    }
    c
}

// ───────── キューブの向きの表 ─────────

/// 面 f の (主軸 Z、列が進む向き X、行が進む向き Y)。方向は Z + s·X + t·Y（s・t は −1〜1、t = −1 が行 0）。
pub fn face_basis(face: usize) -> (Vec3, Vec3, Vec3) {
    match face {
        0 => (Vec3::X, Vec3::NEG_Z, Vec3::NEG_Y),
        1 => (Vec3::NEG_X, Vec3::Z, Vec3::NEG_Y),
        2 => (Vec3::Y, Vec3::X, Vec3::Z),
        3 => (Vec3::NEG_Y, Vec3::X, Vec3::NEG_Z),
        4 => (Vec3::Z, Vec3::X, Vec3::NEG_Y),
        _ => (Vec3::NEG_Z, Vec3::NEG_X, Vec3::NEG_Y),
    }
}

/// 面 f の (s, t) の向き（単位ベクトル）。
pub fn face_direction(face: usize, s: f32, t: f32) -> Vec3 {
    let (z, x, y) = face_basis(face);
    (z + x * s + y * t).normalize()
}

/// 向き d が当たる面と、その面の中の (u, v)（0〜1、v = 0 が行 0）。
pub fn direction_to_face(d: Vec3) -> (usize, f32, f32) {
    let a = d.abs();
    let (face, sc, tc, ma) = if a.x >= a.y && a.x >= a.z {
        if d.x > 0.0 {
            (0, -d.z, -d.y, a.x)
        } else {
            (1, d.z, -d.y, a.x)
        }
    } else if a.y >= a.z {
        if d.y > 0.0 {
            (2, d.x, d.z, a.y)
        } else {
            (3, d.x, -d.z, a.y)
        }
    } else if d.z > 0.0 {
        (4, d.x, -d.y, a.z)
    } else {
        (5, -d.x, -d.y, a.z)
    };
    let ma = ma.max(1e-20);
    (face, 0.5 * (sc / ma + 1.0), 0.5 * (tc / ma + 1.0))
}

/// 1 つの mip の 6 面（面 → 行 → 列の順に並べた RGB。A は使わない）。
#[derive(Clone, Debug)]
pub struct CubeLevel {
    pub size: u32,
    pub texels: Vec<Vec3>,
}

impl CubeLevel {
    fn new(size: u32) -> CubeLevel {
        CubeLevel {
            size,
            texels: vec![Vec3::ZERO; (6 * size * size) as usize],
        }
    }

    fn index(&self, face: usize, x: u32, y: u32) -> usize {
        (face as u32 * self.size * self.size + y * self.size + x) as usize
    }

    /// 向き d の値（面の中の双線形。面の縁は端の画素に張り付く）。
    pub fn sample(&self, d: Vec3) -> Vec3 {
        let (face, u, v) = direction_to_face(d);
        let n = self.size as f32;
        let (fx, fy) = (u * n - 0.5, v * n - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let clamp = |i: f32| (i as i64).clamp(0, self.size as i64 - 1) as u32;
        let (xa, xb, ya, yb) = (clamp(x0), clamp(x0 + 1.0), clamp(y0), clamp(y0 + 1.0));
        let at = |x, y| self.texels[self.index(face, x, y)];
        lerp3(
            lerp3(at(xa, ya), at(xb, ya), tx),
            lerp3(at(xa, yb), at(xb, yb), tx),
            ty,
        )
    }

    /// 2 × 2 の箱で縮めた次の mip。
    fn downsample(&self) -> CubeLevel {
        let size = (self.size / 2).max(1);
        let mut out = CubeLevel::new(size);
        for face in 0..6 {
            for y in 0..size {
                for x in 0..size {
                    let (sx, sy) = ((x * 2).min(self.size - 1), (y * 2).min(self.size - 1));
                    let (sx1, sy1) = ((sx + 1).min(self.size - 1), (sy + 1).min(self.size - 1));
                    let sum = self.texels[self.index(face, sx, sy)]
                        + self.texels[self.index(face, sx1, sy)]
                        + self.texels[self.index(face, sx, sy1)]
                        + self.texels[self.index(face, sx1, sy1)];
                    let i = out.index(face, x, y);
                    out.texels[i] = sum * 0.25;
                }
            }
        }
        out
    }
}

/// 焼いた環境。
#[derive(Clone, Debug)]
pub struct Baked {
    /// 畳み込んだキューブ。`mips[i]` は mip i（粗さ `roughness_of_mip(i)`）で、1 面 `FACE_SIZE >> i` 画素。
    pub mips: Vec<CubeLevel>,
    /// 拡散の環境光（Unity の SphericalHarmonicsL2 の並び。拡散の色 = 照度 / π。明るさ 1）。
    pub sh: [Vec3; 9],
}

fn radical_inverse(mut bits: u32) -> f32 {
    bits = bits.rotate_left(16);
    bits = ((bits & 0x5555_5555) << 1) | ((bits & 0xAAAA_AAAA) >> 1);
    bits = ((bits & 0x3333_3333) << 2) | ((bits & 0xCCCC_CCCC) >> 2);
    bits = ((bits & 0x0F0F_0F0F) << 4) | ((bits & 0xF0F0_F0F0) >> 4);
    bits = ((bits & 0x00FF_00FF) << 8) | ((bits & 0xFF00_FF00) >> 8);
    bits as f32 * 2.328_306_4e-10
}

/// 元を焼く。
pub fn bake(source: &Source) -> Baked {
    // 元をそのままキューブへ（mip 0）。箱で縮めた段が畳み込みの元
    let mut base = CubeLevel::new(FACE_SIZE);
    let n = FACE_SIZE as usize;
    base.texels
        .par_chunks_mut(n * n)
        .enumerate()
        .for_each(|(face, chunk)| {
            for y in 0..n {
                for x in 0..n {
                    let s = 2.0 * (x as f32 + 0.5) / n as f32 - 1.0;
                    let t = 2.0 * (y as f32 + 0.5) / n as f32 - 1.0;
                    chunk[y * n + x] = source.radiance(face_direction(face, s, t));
                }
            }
        });
    let mut source_mips = vec![base];
    while source_mips.last().is_some_and(|l| l.size > 1) {
        let next = source_mips.last().expect("あった").downsample();
        source_mips.push(next);
    }
    let last_source_mip = (source_mips.len() - 1) as f32;
    let texel_solid_angle = 4.0 * PI / (6.0 * (FACE_SIZE * FACE_SIZE) as f32);
    let samples: Vec<(f32, f32)> = (0..SAMPLES)
        .map(|i| ((i as f32 + 0.5) / SAMPLES as f32, radical_inverse(i)))
        .collect();

    let sample_source = |d: Vec3, lod: f32| -> Vec3 {
        let lod = lod.clamp(0.0, last_source_mip);
        let (a, b) = (
            lod.floor() as usize,
            (lod.ceil() as usize).min(source_mips.len() - 1),
        );
        let t = lod - lod.floor();
        let lo = source_mips[a].sample(d);
        if a == b {
            lo
        } else {
            lerp3(lo, source_mips[b].sample(d), t)
        }
    };

    let mut mips = Vec::with_capacity(MIP_COUNT as usize);
    for mip in 0..MIP_COUNT {
        let size = (FACE_SIZE >> mip).max(1);
        if mip == 0 {
            mips.push(source_mips[0].clone());
            continue;
        }
        let alpha = {
            let r = roughness_of_mip(mip);
            (r * r).max(1e-3)
        };
        let a2 = alpha * alpha;
        let mut level = CubeLevel::new(size);
        let n = size as usize;
        level
            .texels
            .par_chunks_mut(n * n)
            .enumerate()
            .for_each(|(face, chunk)| {
                for y in 0..n {
                    for x in 0..n {
                        let s = 2.0 * (x as f32 + 0.5) / n as f32 - 1.0;
                        let t = 2.0 * (y as f32 + 0.5) / n as f32 - 1.0;
                        let normal = face_direction(face, s, t);
                        chunk[y * n + x] = convolve(
                            normal,
                            a2,
                            &samples,
                            texel_solid_angle,
                            last_source_mip,
                            &sample_source,
                        );
                    }
                }
            });
        mips.push(level);
    }
    let sh = project_sh(&source_mips[2.min(source_mips.len() - 1)]);
    Baked { mips, sh }
}

/// 見る向きを法線とみなした GGX の重要度標本（Unity 版の CONVOLVE と同じ式。標本の立体角から読む mip を決めてちらつきを抑える）。
fn convolve(
    n: Vec3,
    a2: f32,
    samples: &[(f32, f32)],
    texel_solid_angle: f32,
    last_mip: f32,
    source: &dyn Fn(Vec3, f32) -> Vec3,
) -> Vec3 {
    let up = if n.y.abs() < 0.999 { Vec3::Y } else { Vec3::X };
    let tx = up.cross(n).normalize();
    let ty = n.cross(tx);
    let mut sum = Vec3::ZERO;
    let mut weight = 0.0;
    for &(xi1, xi2) in samples {
        let phi = 2.0 * PI * xi1;
        let cos_theta = ((1.0 - xi2) / (1.0 + (a2 - 1.0) * xi2)).sqrt();
        let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
        let h = tx * (sin_theta * phi.cos()) + ty * (sin_theta * phi.sin()) + n * cos_theta;
        let l = h * (2.0 * n.dot(h)) - n;
        let nl = n.dot(l);
        if nl <= 0.0 {
            continue;
        }
        let nh = cos_theta.clamp(0.0, 1.0);
        let d = nh * nh * (a2 - 1.0) + 1.0;
        let pdf = a2 / (PI * d * d) * 0.25;
        let sample_solid_angle = 1.0 / (samples.len() as f32 * pdf + 1e-6);
        let lod =
            (0.5 * (sample_solid_angle / texel_solid_angle).log2() + 1.0).clamp(0.0, last_mip);
        sum += source(l, lod) * nl;
        weight += nl;
    }
    sum / weight.max(1e-4)
}

// ───────── 拡散の環境光（SH） ─────────

const K: [f32; 9] = [
    0.282095, 0.488603, 0.488603, 0.488603, 1.092548, 1.092548, 0.315392, 1.092548, 0.546274,
];
const BAND: [f32; 9] = [
    1.0,
    2.0 / 3.0,
    2.0 / 3.0,
    2.0 / 3.0,
    0.25,
    0.25,
    0.25,
    0.25,
    0.25,
];

fn basis(i: usize, d: Vec3) -> f32 {
    match i {
        0 => 1.0,
        1 => d.y,
        2 => d.z,
        3 => d.x,
        4 => d.x * d.y,
        5 => d.y * d.z,
        6 => 3.0 * d.z * d.z - 1.0,
        7 => d.x * d.z,
        _ => d.x * d.x - d.y * d.y,
    }
}

/// キューブの 1 つの mip から、拡散の色（照度 / π）の SH を作る（Unity 版の `Project`: c_i = 帯の畳み込み × ∫ L Y_i dω × 正規化の定数。
/// 一様な 1 の環境で c0 = 1）。画素の立体角は面の上の位置から正確に出す。
fn project_sh(level: &CubeLevel) -> [Vec3; 9] {
    let n = level.size as usize;
    let mut sums = [[0.0f64; 3]; 9];
    for face in 0..6 {
        for y in 0..n {
            for x in 0..n {
                let s = 2.0 * (x as f32 + 0.5) / n as f32 - 1.0;
                let t = 2.0 * (y as f32 + 0.5) / n as f32 - 1.0;
                let d = face_direction(face, s, t);
                let solid = 4.0 / ((n * n) as f32 * (1.0 + s * s + t * t).powf(1.5));
                let c = level.texels[level.index(face, x as u32, y as u32)];
                for (i, row) in sums.iter_mut().enumerate() {
                    let b = (K[i] * basis(i, d) * solid) as f64;
                    row[0] += c.x as f64 * b;
                    row[1] += c.y as f64 * b;
                    row[2] += c.z as f64 * b;
                }
            }
        }
    }
    let mut sh = [Vec3::ZERO; 9];
    for (i, out) in sh.iter_mut().enumerate() {
        let f = (BAND[i] * K[i]) as f64;
        *out = Vec3::new(
            (sums[i][0] * f) as f32,
            (sums[i][1] * f) as f32,
            (sums[i][2] * f) as f32,
        );
    }
    sh
}

impl Baked {
    /// GPU へ上げる並び（mip → 面 → 行 → 列、RGBA の half）。`mip_bytes(i)` が mip i の全 6 面ぶん。
    pub fn mip_bytes(&self, mip: usize) -> Vec<u8> {
        let level = &self.mips[mip];
        let mut out = Vec::with_capacity(level.texels.len() * 8);
        for c in &level.texels {
            for v in [c.x, c.y, c.z, 1.0] {
                out.extend_from_slice(&f16::from_f32(v.clamp(0.0, 65504.0)).to_le_bytes());
            }
        }
        out
    }

    /// 向き d（環境の元の空間）の、粗さの段 mip での値（mip は小数。隣り合う段の補間）。試験・参照用。
    pub fn sample(&self, d: Vec3, mip: f32) -> Vec3 {
        let mip = mip.clamp(0.0, (self.mips.len() - 1) as f32);
        let (a, b) = (mip.floor() as usize, mip.ceil() as usize);
        let lo = self.mips[a].sample(d);
        if a == b {
            lo
        } else {
            lerp3(lo, self.mips[b].sample(d), mip - mip.floor())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view3d::brdf::evaluate_sh;

    fn close3(a: Vec3, b: Vec3, tol: f32) -> bool {
        (a - b).abs().max_element() <= tol
    }

    #[test]
    fn face_table_round_trips_through_every_texel_of_every_face() {
        for face in 0..6 {
            for &(s, t) in &[
                (-0.9, -0.9),
                (0.3, -0.6),
                (0.0, 0.0),
                (0.8, 0.7),
                (-0.4, 0.95),
            ] {
                let d = face_direction(face, s, t);
                let (f, u, v) = direction_to_face(d);
                assert_eq!(f, face, "{d:?}");
                assert!((u - 0.5 * (s + 1.0)).abs() < 1e-5, "{face}: u {u} vs {s}");
                assert!((v - 0.5 * (t + 1.0)).abs() < 1e-5, "{face}: v {v} vs {t}");
            }
        }
        // 軸の向き: +Y は面 2 の真ん中、−Z は面 5
        assert_eq!(direction_to_face(Vec3::Y).0, 2);
        assert_eq!(direction_to_face(Vec3::NEG_Z).0, 5);
    }

    #[test]
    fn uniform_environment_has_unit_sh_and_flat_mips() {
        let level = {
            let mut l = CubeLevel::new(8);
            l.texels.fill(Vec3::ONE);
            l
        };
        // 8² の面では画素の立体角を中心の値で代表させる誤差が 0.4%（焼きで使う 32² では 0.03%）
        let sh = project_sh(&level);
        assert!((sh[0].x - 1.0).abs() < 8e-3, "c0 = {:?}", sh[0]);
        for (i, c) in sh.iter().enumerate().skip(1) {
            assert!(c.abs().max_element() < 4e-3, "c{i} = {c:?}");
        }
        // どの向きでも拡散は 1
        for d in [Vec3::X, Vec3::NEG_Y, Vec3::new(1.0, 1.0, 1.0).normalize()] {
            assert!(close3(evaluate_sh(&sh, d), Vec3::ONE, 1e-2));
        }
    }

    #[test]
    fn sky_bakes_to_the_zenith_horizon_and_ground_colors() {
        let colors = SkyColors::default();
        let baked = bake(&Source::Sky(colors));
        assert_eq!(baked.mips.len(), MIP_COUNT as usize);
        assert_eq!(baked.mips[0].size, FACE_SIZE);
        assert_eq!(baked.mips[6].size, FACE_SIZE >> 6);
        let up = baked.sample(Vec3::Y, 0.0);
        let zenith = srgb3(colors.zenith);
        assert!(close3(up, zenith, 0.03), "天頂 {up:?} vs {zenith:?}");
        let down = baked.sample(Vec3::NEG_Y, 0.0);
        assert!(close3(down, srgb3(colors.ground), 0.02), "地面 {down:?}");
        // 地平線の真上は sqrt の勾配が立つので、行の境目の向きは上下の平均（地平線の色の少し下）になる
        let horizon = baked.sample(Vec3::Z, 0.0);
        assert!(
            close3(horizon, srgb3(colors.horizon), 0.08),
            "地平線 {horizon:?}"
        );
        // 拡散の環境光: 上を向く面は空の色（青い）、下を向く面は暗い
        let top = evaluate_sh(&baked.sh, Vec3::Y);
        let bottom = evaluate_sh(&baked.sh, Vec3::NEG_Y);
        assert!(top.z > top.x && top.x > bottom.x, "{top:?} {bottom:?}");
        assert!(top.max_element() < 1.0 && bottom.min_element() >= 0.0);
        // 粗い mip でも天頂は空の色に寄ったまま（面積の広い空）
        let rough = baked.sample(Vec3::Y, 6.0);
        assert!(rough.z > rough.x, "{rough:?}");
    }

    #[test]
    fn convolution_blurs_a_bright_light_and_keeps_its_energy() {
        let baked = bake(&Source::Studio);
        // 正面右上の面光源（方位 150°・高さ 40°）の真ん中
        let (az, el) = (150f32.to_radians(), 40f32.to_radians());
        let d = Vec3::new(az.sin() * el.cos(), el.sin(), az.cos() * el.cos());
        let sharp = baked.sample(d, 0.0);
        assert!(sharp.x > 5.0, "面光源の真ん中は明るい: {sharp:?}");
        // 粗くするほど面光源の真ん中は暗く、まわりは明るくなる
        let mut last = sharp.x;
        for mip in 1..=6 {
            let v = baked.sample(d, mip as f32).x;
            assert!(v <= last * 1.02, "mip {mip}: {v} > {last}");
            last = v;
        }
        // エネルギー: 粗さ最大の mip の全体の平均は、元の全体の平均とほぼ同じ（立体角で重みを付ける）
        let mean = |level: &CubeLevel| -> f32 {
            let n = level.size as usize;
            let (mut sum, mut total) = (0.0, 0.0);
            for face in 0..6 {
                for y in 0..n {
                    for x in 0..n {
                        let s = 2.0 * (x as f32 + 0.5) / n as f32 - 1.0;
                        let t = 2.0 * (y as f32 + 0.5) / n as f32 - 1.0;
                        let w = 1.0 / (1.0 + s * s + t * t).powf(1.5);
                        sum += level.texels[level.index(face, x as u32, y as u32)].y * w;
                        total += w;
                    }
                }
            }
            sum / total
        };
        let (m0, m6) = (mean(&baked.mips[0]), mean(&baked.mips[6]));
        assert!(
            (m6 / m0 - 1.0).abs() < 0.2,
            "元の平均 {m0}、最も粗い段の平均 {m6}"
        );
        // SH の拡散: 光源の側へ向く面のほうが、反対を向く面より明るい
        let toward = evaluate_sh(&baked.sh, d);
        let away = evaluate_sh(&baked.sh, -d);
        assert!(toward.x > away.x * 1.5, "{toward:?} {away:?}");
    }

    #[test]
    fn mip_bytes_are_half_floats_in_upload_order() {
        let baked = bake(&Source::Sky(SkyColors::default()));
        for mip in [0usize, 3, 6] {
            let bytes = baked.mip_bytes(mip);
            assert_eq!(bytes.len(), baked.mips[mip].texels.len() * 8);
        }
        let bytes = baked.mip_bytes(0);
        let first = f16::from_le_bytes([bytes[0], bytes[1]]).to_f32();
        assert!((first - baked.mips[0].texels[0].x).abs() < 1e-3);
        assert_eq!(
            f16::from_le_bytes([bytes[6], bytes[7]]).to_f32(),
            1.0,
            "A は 1"
        );
    }

    #[test]
    fn radical_inverse_is_the_van_der_corput_sequence() {
        assert_eq!(radical_inverse(0), 0.0);
        assert_eq!(radical_inverse(1), 0.5);
        assert_eq!(radical_inverse(2), 0.25);
        assert_eq!(radical_inverse(3), 0.75);
    }

    #[test]
    #[ignore = "計測（cargo test -p yolu-app --lib environment -- --ignored --nocapture）"]
    fn measure_bake() {
        let started = std::time::Instant::now();
        let _ = bake(&Source::Studio);
        println!(
            "環境の焼き: {:.0} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
}
