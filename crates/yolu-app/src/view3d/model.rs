//! 3D ビューに見せるモデル: 索引つきのメッシュ（表示）と、そこから作った当たりの幾何（core の `SurfaceGeometry`）。
//! 今は試しの立方体と、Live Link で受けたモデル（`yolu_protocol::Model`）の口。どちらも作った後は変えない（ポーズが変われば作り直す）。

use std::sync::Arc;

use yolu_core::geometry::{
    demo_cube, model_triangles, ModelMesh, Submesh, SurfaceGeometry, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Vec2, Vec3};

/// 3D ビューのモデル。
pub struct ViewModel {
    pub name: String,
    pub meshes: Vec<ModelMesh>,
    pub geometry: Arc<SurfaceGeometry>,
    /// マテリアルの組の名前（番号 = 組）。
    pub materials: Vec<String>,
    /// Live Link のモデルの世代（ポーズはこの世代のモデルにだけ当てる）。試しの立方体は None。
    pub link_generation: Option<u32>,
}

impl ViewModel {
    /// メッシュから作る（revision はスナップショットの世代。ストロークの当たりはこの世代で確かめる）。
    pub fn new(
        name: &str,
        meshes: Vec<ModelMesh>,
        materials: Vec<String>,
        revision: u32,
    ) -> Result<ViewModel, String> {
        let triangles = model_triangles(&meshes).ok_or("メッシュの添字が頂点の数を超えています")?;
        if triangles.is_empty() {
            return Err("三角形がありません".into());
        }
        let geometry = SurfaceGeometry::new(triangles, revision, DEFAULT_WELD_TOLERANCE)
            .map_err(|e| e.to_string())?;
        Ok(ViewModel {
            name: name.to_string(),
            meshes,
            geometry: Arc::new(geometry),
            materials,
            link_generation: None,
        })
    }

    /// 試しの立方体（Unity 版のデモと同じ。6 面が別の UV アイランド）。
    pub fn demo(revision: u32) -> ViewModel {
        ViewModel::new(
            "試しの立方体",
            vec![demo_cube()],
            vec!["試しの立方体".into()],
            revision,
        )
        .expect("試しの立方体は作れる")
    }

    /// Live Link で受けたモデル（位置は Unity の座標・根のローカルの空間。UV の無いメッシュは描けない（UV は 0））。
    pub fn from_live_link(
        model: &yolu_protocol::Model,
        revision: u32,
    ) -> Result<ViewModel, String> {
        let meshes = model
            .meshes
            .iter()
            .map(|m| ModelMesh {
                name: m.name.clone(),
                positions: m.positions.iter().map(|p| Vec3::from_array(*p)).collect(),
                normals: m.normals.iter().map(|n| Vec3::from_array(*n)).collect(),
                uvs: m.uv0.iter().map(|u| Vec2::from_array(*u)).collect(),
                submeshes: m
                    .submeshes
                    .iter()
                    .map(|s| Submesh {
                        material: s.material as i32,
                        indices: s.indices.clone(),
                    })
                    .collect(),
            })
            .collect();
        let materials = model
            .materials
            .iter()
            .map(|m| match &m.key {
                yolu_protocol::MaterialKey::Unassigned => "マテリアルなし".to_string(),
                yolu_protocol::MaterialKey::Material { name, .. } => name.clone(),
            })
            .collect();
        let mut m = ViewModel::new(&model.name, meshes, materials, revision)?;
        m.link_generation = Some(model.generation);
        Ok(m)
    }

    /// Live Link のポーズ（変わったメッシュの新しい位置・法線）を当てた新しいモデル（三角形・UV・スロットは同じで、スナップショットの
    /// 世代だけ新しい）。法線が無ければ面から求め直す。世代・メッシュの番号・頂点の数が合わなければ断る。
    pub fn with_pose(
        &self,
        pose: &yolu_protocol::Pose,
        revision: u32,
    ) -> Result<ViewModel, String> {
        if self.link_generation != Some(pose.generation) {
            return Err("ポーズの世代がモデルと違います".into());
        }
        let mut meshes = self.meshes.clone();
        for p in &pose.meshes {
            let m = meshes
                .get_mut(p.mesh as usize)
                .ok_or("ポーズのメッシュの番号が範囲外です")?;
            if p.positions.len() != m.positions.len()
                || (!p.normals.is_empty() && p.normals.len() != m.positions.len())
            {
                return Err("ポーズの頂点の数がメッシュと違います".into());
            }
            m.positions = p.positions.iter().map(|v| Vec3::from_array(*v)).collect();
            m.normals = p.normals.iter().map(|v| Vec3::from_array(*v)).collect();
        }
        let mut m = ViewModel::new(&self.name, meshes, self.materials.clone(), revision)?;
        m.link_generation = self.link_generation;
        Ok(m)
    }

    pub fn triangle_count(&self) -> usize {
        self.geometry.triangle_count()
    }

    pub fn revision(&self) -> u32 {
        self.geometry.revision()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh as WireSubmesh};

    #[test]
    fn live_link_model_becomes_geometry_with_slots_and_materials() {
        let quad = |offset: f32| MeshData {
            key: "0".into(),
            name: "板".into(),
            skinned: false,
            positions: vec![
                [offset, 0.0, 0.0],
                [offset, 1.0, 0.0],
                [offset + 1.0, 0.0, 0.0],
                [offset + 1.0, 1.0, 0.0],
            ],
            normals: Vec::new(),
            uv0: vec![[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]],
            submeshes: vec![
                WireSubmesh {
                    material: 0,
                    indices: vec![0, 1, 2],
                },
                WireSubmesh {
                    material: 1,
                    indices: vec![2, 1, 3],
                },
            ],
        };
        let info = |name: &str| MaterialInfo {
            key: MaterialKey::Material {
                name: name.into(),
                asset: None,
            },
            shader: String::new(),
            textures: Vec::new(),
            routes: Vec::new(),
        };
        let model = Model {
            generation: 1,
            name: "二枚".into(),
            materials: vec![info("肌"), info("服")],
            meshes: vec![quad(0.0), quad(2.0)],
        };
        let m = ViewModel::from_live_link(&model, 5).unwrap();
        assert_eq!(m.triangle_count(), 4);
        assert_eq!(m.revision(), 5);
        assert_eq!(m.materials, vec!["肌".to_string(), "服".to_string()]);
        let t = m.geometry.triangles();
        assert_eq!(
            t.iter()
                .map(|t| (t.renderer, t.material_slot, t.material))
                .collect::<Vec<_>>(),
            vec![(0, 0, 0), (0, 1, 1), (1, 2, 0), (1, 3, 1)],
            "スロットはメッシュ × サブメッシュの通し番号、組はマテリアル"
        );
        // ポーズ: 2 つ目のメッシュを上へ 1 動かすと、三角形と UV はそのままで位置と世代が変わる
        let pose = yolu_protocol::Pose {
            generation: 1,
            meshes: vec![yolu_protocol::MeshPose {
                mesh: 1,
                positions: model.meshes[1]
                    .positions
                    .iter()
                    .map(|p| [p[0], p[1] + 1.0, p[2]])
                    .collect(),
                normals: Vec::new(),
            }],
        };
        let posed = m.with_pose(&pose, 6).unwrap();
        assert_eq!(posed.revision(), 6);
        assert_eq!(posed.geometry.triangles()[2].a.y, 1.0);
        assert_eq!(
            posed.geometry.triangles()[2].uv_b,
            m.geometry.triangles()[2].uv_b
        );
        assert_eq!(posed.geometry.triangles()[0], m.geometry.triangles()[0]);
        let mut wrong = pose.clone();
        wrong.generation = 2;
        assert!(m.with_pose(&wrong, 7).is_err(), "別の世代のポーズは断る");
        wrong.generation = 1;
        wrong.meshes[0].positions.pop();
        assert!(
            m.with_pose(&wrong, 7).is_err(),
            "頂点の数が違うポーズは断る"
        );
        // 添字が範囲外のメッシュは断る
        let mut bad = model.clone();
        bad.meshes[0].submeshes[0].indices = vec![0, 1, 9];
        assert!(ViewModel::from_live_link(&bad, 6).is_err());
    }
}
