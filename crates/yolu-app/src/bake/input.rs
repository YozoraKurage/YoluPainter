//! ベイクの入力: 3D ビューのモデル（受けたままの形。ポーズを付けていればその姿勢の静的な写し）から、三角形ごとの位置・UV・スロット・
//! レンダラーを `MeshBakeInput` にする（Unity 版の `BuildMeshBakeInput` と同じ並び）。
//!
//! スロットはメッシュ（レンダラー）の順 × サブメッシュの順の通し番号（`model_triangles` と同じ）。頂点法線は、全部のメッシュが
//! 持っていればそのまま（由来 `authored`）、1 つでも無ければ形から作り直す（折り目 60°、由来 `reconstructed-crease-60`。編集した
//! 法線は再現しない）。接線・頂点カラー・マテリアルの識別は渡さない（接空間法線は UV から接線を作り、ID は頂点カラー・素材の識別を
//! 選べない）。

use yolu_core::mesh_maps::{reconstruct_normals, MeshBakeAttributes, MeshBakeInput};

use crate::view3d::model::ViewModel;

/// 法線を形から作り直すときの折り目の角度（Unity 版の取り込みの既定と同じ）。
pub const NORMAL_CREASE_DEGREES: f64 = 60.0;

/// モデルから入力を作る。三角形が無い・位置か UV が有限でない・大きさが 0 なら理由を返す。
pub fn build_input(model: &ViewModel) -> Result<MeshBakeInput, String> {
    let triangles: usize = model.meshes.iter().map(|m| m.triangle_count()).sum();
    let mut corners = Vec::with_capacity(triangles * 9);
    let mut uvs = Vec::with_capacity(triangles * 6);
    let mut slots = Vec::with_capacity(triangles);
    let mut renderers = Vec::with_capacity(triangles);
    let mut authored = Vec::with_capacity(triangles * 9);
    let mut all_authored = true;
    let mut slot = 0i32;
    for (renderer, mesh) in model.meshes.iter().enumerate() {
        if !mesh.is_valid() {
            return Err("メッシュの添字が頂点の数を超えています".into());
        }
        let has_normals = mesh.normals.len() == mesh.positions.len();
        all_authored &= has_normals;
        for sub in &mesh.submeshes {
            for tri in sub.indices.as_chunks::<3>().0 {
                for &i in tri {
                    let p = mesh.positions[i as usize];
                    corners.extend_from_slice(&[p.x, p.y, p.z]);
                    if has_normals {
                        let n = mesh.normals[i as usize];
                        authored.extend_from_slice(&[n.x, n.y, n.z]);
                    }
                    let uv = mesh.uvs.get(i as usize).copied().unwrap_or_default();
                    uvs.extend_from_slice(&[uv.x, uv.y]);
                }
                slots.push(slot);
                renderers.push(renderer as i32);
            }
            slot += 1;
        }
    }
    let (normals, source) = if all_authored && authored.len() == corners.len() {
        (authored, "authored".to_owned())
    } else {
        (
            reconstruct_normals(&corners, NORMAL_CREASE_DEGREES).map_err(|e| e.to_string())?,
            format!("reconstructed-crease-{NORMAL_CREASE_DEGREES}"),
        )
    };
    MeshBakeInput::new(
        corners,
        uvs,
        slots,
        MeshBakeAttributes {
            normals: Some(normals),
            renderers: Some(renderers),
            renderer_names: Some(model.meshes.iter().map(|m| m.name.clone()).collect()),
            uv_channel: 0,
            normal_source: Some(source),
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())
}

/// マテリアルの組（サブメッシュが指すマテリアル）を使うスロットの番号（通し番号の昇順）。
pub fn slots_of_material(model: &ViewModel, material: i32) -> Vec<i32> {
    let mut out = Vec::new();
    let mut slot = 0i32;
    for mesh in &model.meshes {
        for sub in &mesh.submeshes {
            if sub.material == material {
                out.push(slot);
            }
            slot += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::geometry::{ModelMesh, Submesh};
    use yolu_core::glam::{Vec2, Vec3};

    fn quad(offset: f32, normals: bool, materials: [i32; 2]) -> ModelMesh {
        ModelMesh {
            name: format!("板{offset}"),
            positions: vec![
                Vec3::new(offset, 0.0, 0.0),
                Vec3::new(offset + 1.0, 0.0, 0.0),
                Vec3::new(offset, 1.0, 0.0),
                Vec3::new(offset + 1.0, 1.0, 0.0),
            ],
            normals: if normals {
                vec![Vec3::Z; 4]
            } else {
                Vec::new()
            },
            uvs: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, 1.0),
                Vec2::new(1.0, 1.0),
            ],
            submeshes: vec![
                Submesh {
                    material: materials[0],
                    indices: vec![0, 1, 2],
                },
                Submesh {
                    material: materials[1],
                    indices: vec![2, 1, 3],
                },
            ],
        }
    }

    fn model(meshes: Vec<ModelMesh>) -> ViewModel {
        ViewModel::new("試し", meshes, vec!["A".into(), "B".into()], 1).unwrap()
    }

    #[test]
    fn slots_run_through_renderers_and_submeshes_and_follow_the_material() {
        let m = model(vec![quad(0.0, true, [0, 1]), quad(2.0, true, [1, 0])]);
        assert_eq!(slots_of_material(&m, 0), [0, 3]);
        assert_eq!(slots_of_material(&m, 1), [1, 2]);
        assert!(slots_of_material(&m, 2).is_empty());
        let input = build_input(&m).unwrap();
        assert_eq!(input.triangle_count(), 4);
        assert_eq!(input.normal_source(), "authored");
        assert_eq!(input.uv_channel(), 0);
    }

    #[test]
    fn a_mesh_without_normals_makes_the_whole_input_reconstructed() {
        let m = model(vec![quad(0.0, true, [0, 0]), quad(2.0, false, [0, 0])]);
        assert_eq!(
            build_input(&m).unwrap().normal_source(),
            "reconstructed-crease-60"
        );
    }

    #[test]
    fn the_same_shape_gives_the_same_fingerprint_and_a_moved_vertex_changes_it() {
        let a = build_input(&model(vec![quad(0.0, true, [0, 0])])).unwrap();
        let b = build_input(&model(vec![quad(0.0, true, [0, 0])])).unwrap();
        assert_eq!(a.hash(), b.hash());
        let mut moved = quad(0.0, true, [0, 0]);
        moved.positions[3].z = 0.5;
        let c = build_input(&model(vec![moved])).unwrap();
        assert_ne!(a.hash(), c.hash(), "頂点が動けば指紋が変わる");
        assert_eq!(
            a.topology_hash(),
            c.topology_hash(),
            "三角形・UV・スロットは同じ"
        );
    }
}
