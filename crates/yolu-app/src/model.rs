//! 読み込んだモデルの記録（今は Live Link で Unity から受けたもの）と、受け取りの口。
//!
//! モデルの形（位置・法線・UV・三角形）は 3D ビュー（`view3d`。ストロークの最中は入れ替えない・ポーズを当てる）だけが持つ。ここに
//! 置くのは、テクスチャセットの結び付けと Live Link に要る記録（出どころ・世代・マテリアルの鍵とシェーダーと流し込み先・スロットごとの
//! マテリアル）だけ。Live Link（`livelink`）も、外からの口（`YoluApp::load_live_link_model`・`apply_live_link_pose`）も、
//! `AppState::receive_link_model`・`receive_link_pose`・`close_link_model` を通る。
//!
//! 3D ビューで描くのは今のテクスチャセットのマテリアルの面だけで、目を閉じたセットのマテリアルの面は 3D から除く
//! （`AppState::sync_view3d`）。試しの立方体（Live Link のモデルでない）には、今のセットの文書を貼る。

use std::sync::atomic::{AtomicU64, Ordering};

use yolu_protocol::{MaterialInfo, MaterialsUpdate, Model, Pose};

use crate::sets::BindReport;
use crate::state::AppState;

/// モデルの出どころ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelSource {
    /// Live Link のつながり（番号。0 はつながりの外から読んだもの）。
    LiveLink { session: u64 },
}

/// 読み込んだモデルの記録。
#[derive(Clone, Debug, PartialEq)]
pub struct SceneModel {
    pub source: ModelSource,
    pub name: String,
    /// 送り手の世代（Pose・Materials・ModelClosed はこの番号で同じモデルを指す）。
    pub generation: u32,
    pub materials: Vec<MaterialInfo>,
    /// スロット（メッシュ × サブメッシュを並びの順に平らにしたもの）ごとのマテリアルの番号。Unity 版の `{"slot": n}` の鍵を読み替える。
    pub slots: Vec<u32>,
    pub meshes: usize,
    pub vertices: usize,
    pub triangles: usize,
    /// モデルを受け直すたびに増える（プロセスの中で一意）。
    pub revision: u64,
    /// 出どころとつながっている（Live Link が切れたら false。3D の形と結び付けは最後のまま残す）。
    pub live: bool,
}

static REVISION: AtomicU64 = AtomicU64::new(1);

impl SceneModel {
    /// Live Link で受けたモデルから作る。
    pub fn from_link(model: &Model, session: u64) -> SceneModel {
        SceneModel {
            source: ModelSource::LiveLink { session },
            name: model.name.clone(),
            generation: model.generation,
            materials: model.materials.clone(),
            slots: model
                .meshes
                .iter()
                .flat_map(|m| m.submeshes.iter().map(|s| s.material))
                .collect(),
            meshes: model.meshes.len(),
            vertices: model.meshes.iter().map(|m| m.positions.len()).sum(),
            triangles: model
                .meshes
                .iter()
                .flat_map(|m| &m.submeshes)
                .map(|s| s.indices.len() / 3)
                .sum(),
            revision: REVISION.fetch_add(1, Ordering::Relaxed),
            live: true,
        }
    }

    /// マテリアルの情報（シェーダー・テクスチャのプロパティ・流し込み先）を置き換える。数が違えば断る。鍵が変わったかを返す。
    pub fn apply_materials(&mut self, update: &MaterialsUpdate) -> Result<bool, String> {
        if update.generation != self.generation {
            return Err(format!(
                "マテリアルの更新の世代 {} は今のモデルの世代 {} と違います",
                update.generation, self.generation
            ));
        }
        if update.materials.len() != self.materials.len() {
            return Err(format!(
                "マテリアルの更新の数 {} がモデルの {} と違います",
                update.materials.len(),
                self.materials.len()
            ));
        }
        let keys_changed = self
            .materials
            .iter()
            .zip(&update.materials)
            .any(|(a, b)| a.key != b.key);
        self.materials.clone_from(&update.materials);
        Ok(keys_changed)
    }
}

impl AppState {
    /// Live Link で受けたモデルを読む: 記録を置き換えてテクスチャセットを結び付け（`bind_model`）、3D ビューに形を読む（描いている
    /// 最中なら、3D の形はストロークが終わってから入れ替わる）。返すのは結び付けの結果と、3D に読めたか（三角形が無い・添字が
    /// 範囲の外なら理由。記録と結び付けはそのまま使う）。
    pub fn receive_link_model(
        &mut self,
        model: &Model,
        session: u64,
    ) -> (BindReport, Result<(), String>) {
        let shape = self.view3d.load_live_link(model);
        self.model = Some(SceneModel::from_link(model, session));
        let report = self.bind_model();
        (report, shape)
    }

    /// Live Link で受けたポーズを 3D ビューの形に当てる（描いている最中なら、待たせている形に当てて終わってから）。今の記録と
    /// 世代が違う・メッシュや頂点の数が合わないものは、何も変えずに断る。
    pub fn receive_link_pose(&mut self, pose: &Pose) -> Result<(), String> {
        match &self.model {
            Some(m) if m.generation == pose.generation => {}
            Some(m) => {
                return Err(format!(
                    "ポーズの世代 {} は今のモデルの世代 {} と違います",
                    pose.generation, m.generation
                ))
            }
            None => return Err("モデルを受ける前のポーズは使えません".into()),
        }
        self.view3d.apply_live_link_pose(pose)
    }

    /// Live Link のモデルを閉じる（記録と 3D の形を外し、セットの結び付けを解く。セットは残す）。世代が違えば何もしないで false。
    pub fn close_link_model(&mut self, generation: u32) -> bool {
        if !self
            .model
            .as_ref()
            .is_some_and(|m| m.generation == generation)
        {
            return false;
        }
        self.model = None;
        self.view3d.close_live_link(generation);
        self.bind_model();
        true
    }

    /// 3D ビューで描くマテリアルと、隠すマテリアルを今のテクスチャセットから決める（描いている最中は変えない）。
    /// - Live Link のモデル: 描くのは今のセットの付いたマテリアル（付いていなければどこにも描かない）。目を閉じたセットのマテリアルは隠す。
    /// - 試しの立方体: 今のセットの文書を貼る（マテリアル 0）。
    pub fn sync_view3d(&mut self) {
        if self.view3d.input.stroke.is_some() {
            return;
        }
        let (material, hidden) = match self.view3d.link_generation() {
            None => (0, Vec::new()),
            Some(g) if self.model.as_ref().is_some_and(|m| m.generation == g) => (
                self.sets.current().bound.map_or(-1, |m| m as i32),
                self.sets
                    .iter()
                    .filter(|s| !s.visible)
                    .filter_map(|s| s.bound.map(|m| m as i32))
                    .collect(),
            ),
            Some(_) => (-1, Vec::new()),
        };
        self.view3d.material = material;
        self.view3d.set_hidden(hidden);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_protocol::{MaterialKey, MeshData, MeshPose, Submesh};

    fn info(name: Option<&str>) -> MaterialInfo {
        MaterialInfo {
            key: match name {
                Some(n) => MaterialKey::Material {
                    name: n.into(),
                    asset: None,
                },
                None => MaterialKey::Unassigned,
            },
            shader: String::new(),
            textures: vec![],
            routes: vec![],
        }
    }

    /// 2 枚の板（どちらも Skin と Unassigned の 2 つのサブメッシュ）。
    fn model(generation: u32) -> Model {
        let quad = |x: f32| MeshData {
            key: format!("{x}"),
            name: "板".into(),
            skinned: true,
            positions: vec![
                [x, 0.0, 0.0],
                [x, 1.0, 0.0],
                [x + 1.0, 0.0, 0.0],
                [x + 1.0, 1.0, 0.0],
            ],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]],
            submeshes: vec![
                Submesh {
                    material: 1,
                    indices: vec![0, 1, 2],
                },
                Submesh {
                    material: 0,
                    indices: vec![2, 1, 3],
                },
            ],
        };
        Model {
            generation,
            name: "二枚".into(),
            materials: vec![info(None), info(Some("Skin"))],
            meshes: vec![quad(0.0), quad(2.0)],
        }
    }

    #[test]
    fn the_record_keeps_slots_and_counts_but_not_the_shape() {
        let m = SceneModel::from_link(&model(3), 1);
        assert_eq!(m.slots, [1, 0, 1, 0]);
        assert_eq!((m.meshes, m.vertices, m.triangles), (2, 8, 4));
        let n = SceneModel::from_link(&model(3), 1);
        assert!(n.revision > m.revision, "受け直すたびに新しい版");
    }

    #[test]
    fn receiving_binds_sets_and_loads_the_3d_shape() {
        let mut s = AppState::new(64, 64);
        let (report, shape) = s.receive_link_model(&model(1), 0);
        assert_eq!(shape, Ok(()));
        assert_eq!(
            report.matched, 1,
            "最初のセットはスロット 0 のマテリアル 1（Skin）"
        );
        assert_eq!(report.created, ["Unassigned"]);
        assert_eq!(s.sets.current().bound, Some(1));
        assert_eq!(s.view3d.material, 1, "3D で描くのは今のセットのマテリアル");
        assert_eq!(s.view3d.model.as_ref().unwrap().triangle_count(), 4);
        // Unassigned のセットの目を閉じると、そのマテリアルの面を 3D から除く
        let unassigned = s.sets.get(1).unwrap().uid;
        s.toggle_set_visible(unassigned);
        let shown = s.view3d.model.clone().unwrap();
        assert_eq!(shown.triangle_count(), 2);
        assert!(shown.geometry.triangles().iter().all(|t| t.material == 1));
        assert_eq!(
            s.view3d.full_model().unwrap().triangle_count(),
            4,
            "受けた形はそのまま"
        );
        // 今のセットを Unassigned に替えると、描くのは 0
        s.toggle_set_visible(unassigned);
        assert_eq!(s.view3d.model.as_ref().unwrap().triangle_count(), 4);
        s.switch_set(1).unwrap();
        assert_eq!(s.view3d.material, 0);
    }

    #[test]
    fn poses_go_to_the_3d_shape_and_wrong_ones_change_nothing() {
        let mut s = AppState::new(64, 64);
        let early = Pose {
            generation: 1,
            meshes: vec![],
        };
        assert!(s.receive_link_pose(&early).is_err());
        let _ = s.receive_link_model(&model(2), 0);
        let before = s.view3d.model.clone().unwrap();
        let wrong_count = Pose {
            generation: 2,
            meshes: vec![MeshPose {
                mesh: 1,
                positions: vec![[1.0; 3]; 3],
                normals: vec![],
            }],
        };
        assert!(s.receive_link_pose(&wrong_count).is_err());
        let old = Pose {
            generation: 1,
            meshes: vec![],
        };
        assert!(s.receive_link_pose(&old).is_err());
        assert!(std::sync::Arc::ptr_eq(
            s.view3d.model.as_ref().unwrap(),
            &before
        ));
        let good = Pose {
            generation: 2,
            meshes: vec![MeshPose {
                mesh: 1,
                positions: vec![[5.0; 3]; 4],
                normals: vec![],
            }],
        };
        assert_eq!(s.receive_link_pose(&good), Ok(()));
        let now = s.view3d.model.clone().unwrap();
        assert_eq!(now.meshes[1].positions[0].x, 5.0);
        assert_eq!(now.meshes[0], before.meshes[0]);
        assert_ne!(now.revision(), before.revision());
        // 閉じると、記録と形を外してセットは残す
        assert!(!s.close_link_model(1));
        assert!(s.close_link_model(2));
        assert!(s.model.is_none() && s.view3d.model.is_none());
        assert_eq!(s.sets.len(), 2);
        assert!(s.sets.iter().all(|x| x.bound.is_none()));
    }
}
