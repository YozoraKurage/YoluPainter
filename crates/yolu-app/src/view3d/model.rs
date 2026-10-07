//! 3D ビューに見せるモデル: 索引つきのメッシュ（表示）と、そこから作った当たりの幾何（core の `SurfaceGeometry`）。
//! 今は試しの立方体と、Live Link で受けたモデル（`yolu_protocol::Model`）の口。どちらも作った後は変えない（ポーズが変われば作り直す）。

use std::sync::{Arc, OnceLock};

use yolu_core::geometry::GeometryError;
use yolu_core::geometry::{
    demo_cube, model_triangles, ModelMesh, Submesh, SurfaceGeometry, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::skin::RigError;
use yolu_model::ModelError;

/// 3D ビュー・ポーズの失敗。文は表示するときに `Lang::view_error` が種類から作る（Display は日本語）。
#[derive(Clone, Debug, PartialEq)]
pub enum ViewError {
    /// 描いている間はポーズを変えない・読まない。
    Stroking,
    /// ポーズを付けるモデルが無い。
    NoPoseModel,
    /// 続けて変える操作が始まっていない。
    NoPoseEdit,
    /// Live Link のポーズを受けたとき、その元のモデル（Unity から受けたもの）がまだ無い。
    NoLinkModel,
    /// 3D ビューに、Live Link のポーズを当てるモデルが無い。
    NoPoseBase,
    /// メッシュの添字が頂点の数を超える。
    BadMeshIndex,
    /// 三角形が無い。
    NoTriangles,
    /// 読み込みの途中で取り消した。
    Cancelled,
    /// 読み込みのスレッドが結果を返さずに止まった。
    LoadStopped,
    /// Live Link のポーズの世代がモデルと違う（モデルが無ければ None）。
    PoseGeneration {
        pose: u32,
        model: Option<u32>,
    },
    /// Live Link のポーズのメッシュの番号が範囲外。
    PoseMesh,
    /// Live Link のポーズの頂点の数がメッシュと違う。
    PoseVertices,
    Rig(RigError),
    Geometry(GeometryError),
    Model(ModelError),
}

impl ViewError {
    /// 知らせの種類: 今の状態で受けない物（描いている間・モデルが無い・操作が始まっていない）は断り、取り消しは済んだ知らせ、ほかは失敗。
    pub fn notice_kind(&self) -> crate::notice::Kind {
        use crate::notice::Kind;
        match self {
            Self::Stroking
            | Self::NoPoseModel
            | Self::NoPoseEdit
            | Self::NoLinkModel
            | Self::NoPoseBase => Kind::Refusal,
            Self::Cancelled => Kind::Info,
            _ => Kind::Error,
        }
    }
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stroking => {
                f.write_str(crate::lang::refusals::during_stroke(crate::lang::Lang::Ja))
            }
            Self::NoPoseModel => f.write_str("ポーズを付けるモデルがありません"),
            Self::NoPoseEdit => f.write_str("ポーズの操作が始まっていません"),
            Self::NoLinkModel => f.write_str("モデルを受ける前のポーズは使えません"),
            Self::NoPoseBase => f.write_str("ポーズを当てるモデルがありません"),
            Self::BadMeshIndex => f.write_str("メッシュの添字が頂点の数を超えています"),
            Self::NoTriangles => f.write_str("三角形がありません"),
            Self::Cancelled => f.write_str("取り消しました"),
            Self::LoadStopped => f.write_str("読み込みが止まりました"),
            Self::PoseGeneration {
                pose,
                model: Some(model),
            } => write!(
                f,
                "ポーズの世代 {pose} は今のモデルの世代 {model} と違います"
            ),
            Self::PoseGeneration { pose, model: None } => {
                write!(f, "ポーズの世代 {pose} のモデルがありません")
            }
            Self::PoseMesh => f.write_str("ポーズのメッシュの番号が範囲外です"),
            Self::PoseVertices => f.write_str("ポーズの頂点の数がメッシュと違います"),
            Self::Rig(e) => e.fmt(f),
            Self::Geometry(e) => e.fmt(f),
            Self::Model(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ViewError {}

impl From<RigError> for ViewError {
    fn from(e: RigError) -> Self {
        Self::Rig(e)
    }
}
impl From<GeometryError> for ViewError {
    fn from(e: GeometryError) -> Self {
        Self::Geometry(e)
    }
}
impl From<ModelError> for ViewError {
    fn from(e: ModelError) -> Self {
        match e {
            // 読み込みを取り消したことは、ほかの失敗と区別する（知らせ・取り消しの後始末が同じ道を通る）
            ModelError::Cancelled => Self::Cancelled,
            e => Self::Model(e),
        }
    }
}

use super::tangents::{model_tangents, Tangent};

/// 3D ビューのモデル。
pub struct ViewModel {
    pub name: String,
    pub meshes: Vec<ModelMesh>,
    pub geometry: Arc<SurfaceGeometry>,
    /// マテリアルの組（番号 = 組）の名前。マテリアルの無いスロットは None（名前は表示のときに言語で作る）。
    pub materials: Vec<Option<String>>,
    /// Live Link のモデルの世代（ポーズはこの世代のモデルにだけ当てる）。試しの立方体は None。
    pub link_generation: Option<u32>,
    /// 試しの立方体か（見出しの名前を表示の言語に合わせる）。
    pub demo: bool,
    /// 角ごとの接線（`tangents`）。法線マップを見せるときだけ、最初に要る所で作る。
    tangents: OnceLock<Arc<Vec<Tangent>>>,
}

impl ViewModel {
    /// メッシュから作る（revision はスナップショットの世代。ストロークの当たりはこの世代で確かめる）。
    pub fn new(
        name: &str,
        meshes: Vec<ModelMesh>,
        materials: Vec<Option<String>>,
        revision: u32,
    ) -> Result<ViewModel, ViewError> {
        let triangles = model_triangles(&meshes).ok_or(ViewError::BadMeshIndex)?;
        if triangles.is_empty() {
            return Err(ViewError::NoTriangles);
        }
        let geometry = SurfaceGeometry::new(triangles, revision, DEFAULT_WELD_TOLERANCE)?;
        Ok(ViewModel {
            name: name.to_string(),
            meshes,
            geometry: Arc::new(geometry),
            materials,
            link_generation: None,
            demo: false,
            tangents: OnceLock::new(),
        })
    }

    /// 作ってあるスナップショットから（ポーズを付けた形。メッシュはスナップショットと同じ三角形の並び）。
    pub fn with_geometry(
        name: &str,
        meshes: Vec<ModelMesh>,
        materials: Vec<Option<String>>,
        geometry: Arc<SurfaceGeometry>,
    ) -> ViewModel {
        ViewModel {
            name: name.to_string(),
            meshes,
            geometry,
            materials,
            link_generation: None,
            demo: false,
            tangents: OnceLock::new(),
        }
    }

    /// 試しの立方体（Unity 版のデモと同じ。6 面が別の UV アイランド）。
    pub fn demo(revision: u32) -> ViewModel {
        let mut m = ViewModel::new(
            "試しの立方体",
            vec![demo_cube()],
            vec![Some("試しの立方体".into())],
            revision,
        )
        .expect("試しの立方体は作れる");
        m.demo = true;
        m
    }

    /// スロットのマテリアルの名前（マテリアルの無いスロットと試しの立方体は表示の言語で。範囲外は番号）。
    pub fn material_name(&self, slot: usize, lang: crate::lang::Lang) -> String {
        match self.materials.get(slot) {
            Some(_) if self.demo => self.display_name(lang).to_owned(),
            Some(Some(name)) => name.clone(),
            Some(None) => lang.pick("マテリアルなし", "No material").into(),
            None => slot.to_string(),
        }
    }

    /// 見出しに出す名前（試しの立方体は表示の言語で）。
    pub fn display_name(&self, lang: crate::lang::Lang) -> &str {
        if self.demo {
            lang.pick("試しの立方体", "Test Cube")
        } else {
            &self.name
        }
    }

    /// Live Link で受けたモデル（位置は Unity の座標・根のローカルの空間。UV の無いメッシュは描けない（UV は 0））。
    pub fn from_live_link(
        model: &yolu_protocol::Model,
        revision: u32,
    ) -> Result<ViewModel, ViewError> {
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
                yolu_protocol::MaterialKey::Unassigned => None,
                yolu_protocol::MaterialKey::Material { name, .. } => Some(name.clone()),
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
    ) -> Result<ViewModel, ViewError> {
        if self.link_generation != Some(pose.generation) {
            return Err(ViewError::PoseGeneration {
                pose: pose.generation,
                model: self.link_generation,
            });
        }
        let mut meshes = self.meshes.clone();
        for p in &pose.meshes {
            let m = meshes.get_mut(p.mesh as usize).ok_or(ViewError::PoseMesh)?;
            if p.positions.len() != m.positions.len()
                || (!p.normals.is_empty() && p.normals.len() != m.positions.len())
            {
                return Err(ViewError::PoseVertices);
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

    /// 角ごとの接線（MikkTSpace。`render` が上げる頂点の並び）。最初に呼んだスレッドが作り、ほかは待つ（数万三角形で数百ミリ秒）。
    pub fn tangents(&self) -> &Arc<Vec<Tangent>> {
        self.tangents
            .get_or_init(|| Arc::new(model_tangents(&self.meshes)))
    }

    /// 出来ていれば接線（待たない）。
    pub fn tangents_ready(&self) -> Option<&Arc<Vec<Tangent>>> {
        self.tangents.get()
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
        assert_eq!(
            m.materials,
            vec![Some("肌".to_string()), Some("服".to_string())]
        );
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
