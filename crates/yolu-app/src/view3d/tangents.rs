//! 接線（法線マップの向き）: Unity の `Mesh.RecalculateTangents` と同じ MikkTSpace（`bevy_mikktspace`。参照実装とバイトまで同じ出力）で、
//! 三角形の角ごとに作る。法線マップは YoluPainter の Normal の出力（OpenGL の Y+、緑 = +V）で、シェーダーは
//! `normalize(T × x + B × y + N × z)`、`B = cross(N, T) × w`（Unity の頂点シェーダーと同じ）で読む。
//!
//! - 角ごと（`ensure_mesh` が上げる頂点の並びと同じ: メッシュ → サブメッシュ → 三角形 → 3 つの角）。Unity は頂点ごとに 1 つへまとめるが、
//!   ここは MikkTSpace が角ごとに出す値をそのまま使う（UV の継ぎ目・ミラーの境で頂点を共有していても接線が混ざらない。ベイクと
//!   同じ接空間になる）。
//! - `w` は従接線の向きの符号。MikkTSpace が返す従接線（V が増える向き）と `cross(N, T)` の向きの一致で決める（座標系が左手でも右手でも
//!   同じ式。Unity の組み込みのメッシュの `w = −1` と同じ意味になる）。
//! - UV が無いメッシュ・縮退した三角形は、法線に直交する適当な接線（w = 1）。UV が無ければ描けず、法線マップも読まない。

use bevy_mikktspace::{generate_tangents, Geometry, TangentSpace};
use yolu_core::geometry::ModelMesh;
use yolu_core::glam::Vec3;

/// 角 1 つの接線（xyz = 接線、w = 従接線の向きの符号）。
pub type Tangent = [f32; 4];

struct Corners<'a> {
    mesh: &'a ModelMesh,
    normals: Vec<Vec3>,
    /// 三角形ごとの 3 つの頂点の添字（サブメッシュを順に並べたもの）。
    triangles: Vec<[u32; 3]>,
    out: Vec<Tangent>,
}

impl Geometry for Corners<'_> {
    fn num_faces(&self) -> usize {
        self.triangles.len()
    }
    fn num_vertices_of_face(&self, _face: usize) -> usize {
        3
    }
    fn position(&self, face: usize, vert: usize) -> [f32; 3] {
        self.mesh.positions[self.triangles[face][vert] as usize].to_array()
    }
    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        self.normals[self.triangles[face][vert] as usize].to_array()
    }
    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        self.mesh
            .uvs
            .get(self.triangles[face][vert] as usize)
            .copied()
            .unwrap_or_default()
            .to_array()
    }
    fn set_tangent(&mut self, space: Option<TangentSpace>, face: usize, vert: usize) {
        let corner = face * 3 + vert;
        let normal = self.normals[self.triangles[face][vert] as usize];
        self.out[corner] = match space {
            Some(s) => encode(normal, s),
            None => fallback(normal),
        };
    }
}

/// 法線に直交する適当な接線（w = 1）。
fn fallback(normal: Vec3) -> Tangent {
    let helper = if normal.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let t = (helper - normal * normal.dot(helper)).normalize_or_zero();
    let t = if t == Vec3::ZERO { Vec3::X } else { t };
    [t.x, t.y, t.z, 1.0]
}

fn encode(normal: Vec3, space: TangentSpace) -> Tangent {
    let t = Vec3::from_array(space.tangent());
    let b = Vec3::from_array(space.bi_tangent());
    if !t.is_finite() || !b.is_finite() || t.length_squared() < 1e-12 {
        return fallback(normal);
    }
    // 従接線の向き（V が増える向き）に合う側の符号
    let w = if normal.cross(t).dot(b) < 0.0 {
        -1.0
    } else {
        1.0
    };
    [t.x, t.y, t.z, w]
}

/// メッシュ 1 つの角ごとの接線（サブメッシュ → 三角形 → 角の順。`positions` と `uvs` が合わなければ全部 `fallback`）。
pub fn mesh_tangents(mesh: &ModelMesh) -> Vec<Tangent> {
    let normals = mesh.vertex_normals();
    let triangles: Vec<[u32; 3]> = mesh
        .submeshes
        .iter()
        .flat_map(|s| s.indices.as_chunks::<3>().0.iter().copied())
        .collect();
    let n = triangles.len() * 3;
    let usable = mesh.uvs.len() == mesh.positions.len() && normals.len() == mesh.positions.len();
    let mut corners = Corners {
        mesh,
        normals,
        triangles,
        out: vec![[1.0, 0.0, 0.0, 1.0]; n],
    };
    if usable && n > 0 {
        // 失敗の型は空（将来の予約）。出力は埋めてあるので、起きても法線マップの向きが乱れるだけ
        let _ = generate_tangents(&mut corners);
    } else {
        for t in 0..corners.triangles.len() {
            for v in 0..3 {
                let normal = corners.normals[corners.triangles[t][v] as usize];
                corners.out[t * 3 + v] = fallback(normal);
            }
        }
    }
    corners.out
}

/// モデルの全メッシュの角ごとの接線（メッシュの並び）。
pub fn model_tangents(meshes: &[ModelMesh]) -> Vec<Tangent> {
    meshes.iter().flat_map(mesh_tangents).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::geometry::{cube_sphere, demo_cube, Submesh};
    use yolu_core::glam::Vec2;

    /// 三角形の角の位置・UV から、U と V が増える向き（世界）。
    fn uv_frame(mesh: &ModelMesh, tri: [u32; 3]) -> (Vec3, Vec3) {
        let p = |i: u32| mesh.positions[i as usize];
        let uv = |i: u32| mesh.uvs[i as usize];
        let (e1, e2) = (p(tri[1]) - p(tri[0]), p(tri[2]) - p(tri[0]));
        let (d1, d2) = (uv(tri[1]) - uv(tri[0]), uv(tri[2]) - uv(tri[0]));
        let det = d1.x * d2.y - d2.x * d1.y;
        let r = 1.0 / det;
        let du = (e1 * d2.y - e2 * d1.y) * r;
        let dv = (e2 * d1.x - e1 * d2.x) * r;
        (du.normalize(), dv.normalize())
    }

    fn check_mesh(mesh: &ModelMesh) {
        let tangents = mesh_tangents(mesh);
        let normals = mesh.vertex_normals();
        let triangles: Vec<[u32; 3]> = mesh
            .submeshes
            .iter()
            .flat_map(|s| s.indices.as_chunks::<3>().0.iter().copied())
            .collect();
        assert_eq!(tangents.len(), triangles.len() * 3);
        let mut checked = 0;
        for (t, tri) in triangles.iter().enumerate() {
            let (du, dv) = uv_frame(mesh, *tri);
            for k in 0..3 {
                let [x, y, z, w] = tangents[t * 3 + k];
                let tangent = Vec3::new(x, y, z);
                let normal = normals[tri[k] as usize];
                assert!(tangent.is_finite() && (tangent.length() - 1.0).abs() < 1e-3);
                assert!(w == 1.0 || w == -1.0);
                // 接線は U が増える向き、従接線（cross(N, T) × w）は V が増える向きに寄る（球は面ごとに曲がるので、鈍い角度で見る）
                let bitangent = normal.cross(tangent) * w;
                assert!(
                    tangent.dot(du) > 0.5,
                    "三角形 {t} 角 {k}: 接線 {tangent:?} と U の向き {du:?}"
                );
                assert!(
                    bitangent.dot(dv) > 0.5,
                    "三角形 {t} 角 {k}: 従接線 {bitangent:?} と V の向き {dv:?}"
                );
                checked += 1;
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn tangent_frames_follow_the_uv_directions_on_the_cube_and_sphere() {
        check_mesh(&demo_cube());
        check_mesh(&cube_sphere(6, 1.0));
    }

    #[test]
    fn a_unity_quad_gets_the_unity_primitive_tangent() {
        // Unity の Quad: 外向きの法線は −Z、U は +X、V は +Y。接線は (1, 0, 0)、従接線 = cross(N, T) × w が +Y なので w = −1
        let mesh = ModelMesh {
            name: "板".into(),
            positions: vec![
                Vec3::new(-0.5, -0.5, 0.0),
                Vec3::new(0.5, -0.5, 0.0),
                Vec3::new(-0.5, 0.5, 0.0),
                Vec3::new(0.5, 0.5, 0.0),
            ],
            normals: vec![Vec3::NEG_Z; 4],
            uvs: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, 1.0),
                Vec2::new(1.0, 1.0),
            ],
            submeshes: vec![Submesh {
                material: 0,
                indices: vec![0, 2, 1, 2, 3, 1],
            }],
        };
        let tangents = mesh_tangents(&mesh);
        assert_eq!(tangents.len(), 6);
        for t in &tangents {
            assert!((t[0] - 1.0).abs() < 1e-5 && t[1].abs() < 1e-5 && t[2].abs() < 1e-5);
            assert_eq!(t[3], -1.0, "{t:?}");
        }
    }

    #[test]
    fn meshes_without_uvs_fall_back_to_a_perpendicular_tangent() {
        let mesh = ModelMesh {
            name: "UV なし".into(),
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            normals: vec![Vec3::Z; 3],
            uvs: Vec::new(),
            submeshes: vec![Submesh {
                material: 0,
                indices: vec![0, 1, 2],
            }],
        };
        for t in mesh_tangents(&mesh) {
            let tangent = Vec3::new(t[0], t[1], t[2]);
            assert!(tangent.dot(Vec3::Z).abs() < 1e-5 && (tangent.length() - 1.0).abs() < 1e-5);
            assert_eq!(t[3], 1.0);
        }
    }

    #[test]
    fn model_tangents_follow_the_upload_order_of_all_meshes() {
        let a = demo_cube();
        let b = cube_sphere(2, 1.0);
        let all = model_tangents(&[a.clone(), b.clone()]);
        assert_eq!(all.len(), (a.triangle_count() + b.triangle_count()) * 3);
        assert_eq!(&all[..a.triangle_count() * 3], &mesh_tangents(&a)[..]);
    }

    #[test]
    #[ignore = "計測（cargo test -p yolu-app --lib tangents -- --ignored --nocapture）"]
    fn measure_seventy_thousand_triangles() {
        let mesh = cube_sphere(77, 1.0); // 12 × 77² = 71148 三角形
        let started = std::time::Instant::now();
        let t = mesh_tangents(&mesh);
        println!(
            "{} 三角形の接線: {:.1} ms",
            mesh.triangle_count(),
            started.elapsed().as_secs_f64() * 1000.0
        );
        assert_eq!(t.len(), mesh.triangle_count() * 3);
    }
}
