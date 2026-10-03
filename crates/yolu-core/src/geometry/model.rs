//! 索引つきのメッシュ（Live Link で受けたメッシュ、試しの立方体・球）と、そこから当たりの三角形のスープを作る式。
//!
//! スロットの番号は Unity 版と同じく、メッシュ（レンダラー）の順 × サブメッシュの順の通し番号。マテリアルの組はサブメッシュが指す
//! マテリアルの番号（同じマテリアルを使うスロットは同じ組 = 同じテクスチャセット）。

use glam::{Vec2, Vec3};

use super::SurfaceTriangle;

/// 三角形の組 1 つ（1 つのマテリアルで描く）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Submesh {
    /// マテリアルの組の番号。
    pub material: i32,
    /// 三角形の頂点の添字（3 つずつ）。
    pub indices: Vec<u32>,
}

/// レンダラー 1 つのメッシュ（位置はモデルの空間、左手系）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelMesh {
    pub name: String,
    pub positions: Vec<Vec3>,
    /// 空か、positions と同じ数。
    pub normals: Vec<Vec3>,
    /// 空か、positions と同じ数（空なら UV は全部 0 で、描けない）。
    pub uvs: Vec<Vec2>,
    pub submeshes: Vec<Submesh>,
}

impl ModelMesh {
    pub fn triangle_count(&self) -> usize {
        self.submeshes.iter().map(|s| s.indices.len() / 3).sum()
    }

    /// 頂点の法線（あればそれ、無ければ面の法線を頂点ごとに足して正規化する。Unity の RecalculateNormals と同じ考え方で、
    /// 位置で溶接はしない）。
    pub fn vertex_normals(&self) -> Vec<Vec3> {
        if self.normals.len() == self.positions.len() {
            return self.normals.clone();
        }
        let mut n = vec![Vec3::ZERO; self.positions.len()];
        for s in &self.submeshes {
            for t in s.indices.chunks_exact(3) {
                let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                let (pa, pb, pc) = (self.positions[a], self.positions[b], self.positions[c]);
                let face = (pb - pa).cross(pc - pa);
                n[a] += face;
                n[b] += face;
                n[c] += face;
            }
        }
        n.iter().map(|v| v.normalize_or_zero()).collect()
    }

    /// 添字が頂点の数を超えないか・三角形の数が 3 の倍数か。
    pub fn is_valid(&self) -> bool {
        (self.normals.is_empty() || self.normals.len() == self.positions.len())
            && (self.uvs.is_empty() || self.uvs.len() == self.positions.len())
            && self.submeshes.iter().all(|s| {
                s.indices.len() % 3 == 0
                    && s.indices
                        .iter()
                        .all(|&i| (i as usize) < self.positions.len())
            })
    }
}

/// モデルのメッシュから当たりの三角形（レンダラー = メッシュの番号、スロット = 通し番号、マテリアル = サブメッシュの組）。
/// 添字が範囲外のメッシュは None（先に `is_valid` で確かめる）。
pub fn model_triangles(meshes: &[ModelMesh]) -> Option<Vec<SurfaceTriangle>> {
    let mut out = Vec::with_capacity(meshes.iter().map(|m| m.triangle_count()).sum());
    let mut slot = 0i32;
    for (r, m) in meshes.iter().enumerate() {
        if !m.is_valid() {
            return None;
        }
        let uv = |i: u32| m.uvs.get(i as usize).copied().unwrap_or(Vec2::ZERO);
        for s in &m.submeshes {
            for t in s.indices.chunks_exact(3) {
                out.push(
                    SurfaceTriangle::new(
                        m.positions[t[0] as usize],
                        m.positions[t[1] as usize],
                        m.positions[t[2] as usize],
                        uv(t[0]),
                        uv(t[1]),
                        uv(t[2]),
                    )
                    .with_slot(r as i32, slot, s.material.max(0)),
                );
            }
            slot += 1;
        }
    }
    Some(out)
}

/// 試しの立方体（Unity 版の LoadDemoMesh と同じ: 一辺 1、6 面がそれぞれ別の UV アイランド（3 × 2 に並べる）、頂点は面ごとに別）。
pub fn demo_cube() -> ModelMesh {
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
    let h = 0.5f32;
    let faces = [
        [v(-h, -h, -h), v(-h, h, -h), v(h, -h, -h), v(h, h, -h)],
        [v(h, -h, h), v(h, h, h), v(-h, -h, h), v(-h, h, h)],
        [v(-h, -h, h), v(-h, h, h), v(-h, -h, -h), v(-h, h, -h)],
        [v(h, -h, -h), v(h, h, -h), v(h, -h, h), v(h, h, h)],
        [v(-h, h, -h), v(-h, h, h), v(h, h, -h), v(h, h, h)],
        [v(-h, -h, h), v(-h, -h, -h), v(h, -h, h), v(h, -h, -h)],
    ];
    for f in faces {
        let start = positions.len() as u32;
        let face = start / 4;
        positions.extend_from_slice(&f);
        // C# の (face % 3) / 3f + 0.02f、(face / 3) * 0.5f + 0.03f、幅 1/3 − 0.04、高さ 0.44
        let u = (face % 3) as f32 / 3.0 + 0.02;
        let vv = (face / 3) as f32 * 0.5 + 0.03;
        let (w, hh) = (1.0f32 / 3.0 - 0.04, 0.44f32);
        uvs.extend_from_slice(&[
            Vec2::new(u, vv),
            Vec2::new(u, vv + hh),
            Vec2::new(u + w, vv),
            Vec2::new(u + w, vv + hh),
        ]);
        indices.extend_from_slice(&[start, start + 1, start + 2, start + 2, start + 1, start + 3]);
    }
    ModelMesh {
        name: "試しの立方体".into(),
        positions,
        normals: Vec::new(),
        uvs,
        submeshes: vec![Submesh {
            material: 0,
            indices,
        }],
    }
}

/// 立方体の 6 面を n × n に分けて球へ膨らませたメッシュ（三角形 12·n²。面ごとに別の UV アイランド（3 × 2）、継ぎ目は位置の溶接で
/// つながる）。位置は四則と平方根だけで作るので、C# と同じ値になる（三角関数の実装の違いに頼らない。正解のファイルと計測に使う）。
pub fn cube_sphere(n: u32, radius: f32) -> ModelMesh {
    let n = n.max(1);
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    // 面の中心の向き・横・縦（外から見て、横 × 縦 が外向きになる左手系の並び）
    let frames: [(Vec3, Vec3, Vec3); 6] = [
        (
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ),
        (
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ),
        (
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, 1.0, 0.0),
        ),
        (
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 1.0, 0.0),
        ),
        (
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ),
        (
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, -1.0),
        ),
    ];
    let nf = n as f32;
    for (face, (center, right, up)) in frames.iter().enumerate() {
        let start = positions.len() as u32;
        let u0 = (face % 3) as f32 / 3.0 + 0.01;
        let v0 = (face / 3) as f32 * 0.5 + 0.01;
        let (w, h) = (1.0f32 / 3.0 - 0.02, 0.48f32);
        for j in 0..=n {
            for i in 0..=n {
                // (2i − n) / n: 隣の面の同じ辺の点が、向きが逆でもビットまで同じ値になる（符号だけ違う分子を同じ数で割る）
                let s = (2 * i as i64 - n as i64) as f32 / nf;
                let t = (2 * j as i64 - n as i64) as f32 / nf;
                let p = Vec3::new(
                    center.x + right.x * s + up.x * t,
                    center.y + right.y * s + up.y * t,
                    center.z + right.z * s + up.z * t,
                );
                let len = (p.x * p.x + p.y * p.y + p.z * p.z).sqrt();
                positions.push(Vec3::new(
                    p.x / len * radius,
                    p.y / len * radius,
                    p.z / len * radius,
                ));
                uvs.push(Vec2::new(
                    u0 + w * (i as f32 / nf),
                    v0 + h * (j as f32 / nf),
                ));
            }
        }
        let row = n + 1;
        for j in 0..n {
            for i in 0..n {
                let a = start + j * row + i;
                let (b, c, d) = (a + row, a + 1, a + row + 1);
                // 外から見て時計回り（Unity の表）
                indices.extend_from_slice(&[a, b, c, c, b, d]);
            }
        }
    }
    let normals = positions.iter().map(|p| *p / radius).collect();
    ModelMesh {
        name: format!("球 {}", 12 * n * n),
        positions,
        normals,
        uvs,
        submeshes: vec![Submesh {
            material: 0,
            indices,
        }],
    }
}
