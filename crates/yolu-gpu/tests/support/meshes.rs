//! ベイクの試験に使う人工のメッシュ（実モデルは使わない）。整数のハッシュで作るので、どの環境でも同じ。
use yolu_core::mesh_maps::{MeshBakeAttributes, MeshBakeInput};

pub fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}
pub fn unit(seed: u32) -> f32 {
    (hash(seed) >> 8) as f32 / 16_777_216.0
}

/// 6 面の分割立方体（C# の MeshGolden と同じ並び）。UV は 3×2 の格子に面ごとの島。`bulge` は球に近づける割合、
/// `noise` は頂点ごとの凹凸。面（スロット）は面ごとに 1 つ。
pub struct Cube {
    pub corners: Vec<f32>,
    pub uvs: Vec<f32>,
    pub slots: Vec<i32>,
}
pub fn cube(divisions: usize, bulge: f32, noise: f32, seed: u32) -> Cube {
    let triangles = 12 * divisions * divisions;
    let mut corners = vec![0f32; triangles * 9];
    let mut uvs = vec![0f32; triangles * 6];
    let mut slots = vec![0; triangles];
    let order = [0, 1, 2, 0, 2, 3];
    let mut at = 0;
    for face in 0..6 {
        for y in 0..divisions {
            for x in 0..divisions {
                let axis = face / 2;
                let sign = if face % 2 == 0 { 1. } else { -1. };
                for &v in &order {
                    let u = (x + usize::from(v != 0 && v != 3)) as f32 / divisions as f32;
                    let w = (y + usize::from(v >= 2)) as f32 / divisions as f32;
                    let k = at * 3;
                    let mut p = [0f32; 3];
                    p[axis] = sign;
                    p[(axis + 1) % 3] = u * 2. - 1.;
                    p[(axis + 2) % 3] = (w * 2. - 1.) * sign;
                    // 同じ位置の頂点は同じ凹凸にする（接する面でも割れない）
                    let key = ((p[0] * 1024.) as i32 as u32)
                        ^ ((p[1] * 1024.) as i32 as u32).wrapping_mul(7919)
                        ^ ((p[2] * 1024.) as i32 as u32).wrapping_mul(104_729)
                        ^ seed;
                    let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                    let r = 1. - bulge + bulge / len + noise * (unit(key) - 0.5);
                    corners[k..k + 3].copy_from_slice(&[p[0] * r, p[1] * r, p[2] * r]);
                    uvs[at * 2] = (face as f32 % 3. + u) / 3.;
                    uvs[at * 2 + 1] = ((face / 3) as f32 + w) / 2.;
                    slots[at / 3] = face as i32;
                    at += 1;
                }
            }
        }
    }
    Cube {
        corners,
        uvs,
        slots,
    }
}

/// UV の格子を、テクセルの中心が三角形の共有する辺（四角形の対角線）の上に乗らないようにずらす。辺の上のサンプルは、どちらの
/// 三角形の面の法線を使うかで AO のレイが変わる（CPU 同士でも UV を数 ulp ずらすだけで変わる）ので、実装の差を測る試験は
/// 外し、辺の上に乗る入力は別に試す。
pub fn skewed(mut c: Cube) -> Cube {
    for v in c.uvs.iter_mut() {
        *v = *v * 0.9731 + 0.0127;
    }
    c
}

/// 頂点法線（位置から原点への向き）と接線（UV の u 方向）付きの入力。
pub fn input_with_normals(c: &Cube) -> MeshBakeInput {
    let n = c.corners.len() / 9;
    let mut normals = vec![0f32; n * 9];
    for i in 0..n * 3 {
        let p = &c.corners[i * 3..i * 3 + 3];
        let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-9);
        normals[i * 3..i * 3 + 3].copy_from_slice(&[p[0] / l, p[1] / l, p[2] / l]);
    }
    MeshBakeInput::new(
        c.corners.clone(),
        c.uvs.clone(),
        c.slots.clone(),
        MeshBakeAttributes {
            normals: Some(normals),
            ..Default::default()
        },
    )
    .unwrap()
}
pub fn input_plain(c: &Cube) -> MeshBakeInput {
    MeshBakeInput::new(
        c.corners.clone(),
        c.uvs.clone(),
        c.slots.clone(),
        MeshBakeAttributes::default(),
    )
    .unwrap()
}

/// 頂点法線と、レンダラー（部品）の番号・名前・頂点カラー・マテリアルの識別子を足せる入力。
#[derive(Default)]
pub struct Extras {
    pub normals: bool,
    /// 面（立方体の 6 面）ごとのレンダラー番号。
    pub renderers: Option<[i32; 6]>,
    pub renderer_names: Option<Vec<String>>,
    pub colors: bool,
    pub material_keys: Option<[&'static str; 6]>,
}
pub fn input_with(c: &Cube, e: &Extras) -> MeshBakeInput {
    let n = c.corners.len() / 9;
    let per_face = n / 6;
    let normals = e.normals.then(|| {
        let mut v = vec![0f32; n * 9];
        for i in 0..n * 3 {
            let p = &c.corners[i * 3..i * 3 + 3];
            let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-9);
            v[i * 3..i * 3 + 3].copy_from_slice(&[p[0] / l, p[1] / l, p[2] / l]);
        }
        v
    });
    let colors = e.colors.then(|| {
        let mut v = vec![0f32; n * 12];
        for i in 0..n * 3 {
            let face = i / 3 / per_face;
            v[i * 4..i * 4 + 4].copy_from_slice(&[
                unit(face as u32 * 3 + 1),
                unit(face as u32 * 3 + 2),
                unit(face as u32 * 3 + 3),
                1.0,
            ]);
        }
        v
    });
    MeshBakeInput::new(
        c.corners.clone(),
        c.uvs.clone(),
        c.slots.clone(),
        MeshBakeAttributes {
            normals,
            colors,
            renderers: e
                .renderers
                .map(|r| (0..n).map(|t| r[t / per_face]).collect()),
            renderer_names: e.renderer_names.clone(),
            material_keys: e
                .material_keys
                .map(|k| (0..n).map(|t| Some(k[t / per_face].to_string())).collect()),
            ..Default::default()
        },
    )
    .unwrap()
}
