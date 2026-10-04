//! 読み込んだモデルの記録（Live Link で Unity から受けたものと、ポーズを付けられるモデル（FBX・試しの人形））と、受け取りの口。
//!
//! モデルの形（位置・法線・UV・三角形）は 3D ビュー（`view3d`。ストロークの最中は入れ替えない・ポーズを当てる）だけが持つ。ここに
//! 置くのは、テクスチャセットの結び付けと Live Link に要る記録（出どころ・世代・マテリアルの鍵とシェーダーと流し込み先・スロットごとの
//! マテリアル）だけ。Live Link（`livelink`）も、外からの口（`YoluApp::load_live_link_model`・`apply_live_link_pose`）も、
//! `AppState::receive_link_model`・`receive_link_pose`・`close_link_model` を通る。
//!
//! 3D ビューで描くのは今のテクスチャセットのマテリアルの面だけで、目を閉じたセットのマテリアルの面は 3D から除く
//! （`AppState::sync_view3d`）。試しの立方体（記録の無いモデル）には、今のセットの文書を貼る。
//!
//! ポーズを付けられるモデル（`view3d::pose` のセッションの `Rig`）も、Live Link と同じ照合（識別子 → 未割り当て → 名前 → スロット）で
//! マテリアルごとにセットを付けるか作る。鍵は名前だけ（FBX にアセットの識別子は無い）で、シェーダー・流し込み先は持たない
//! （Unity には出さない）。モデルは `Arc<Rig>` の同一性で指す（ポーズを付けても同じ。別のモデルに替わると外れる）。

use std::sync::atomic::{AtomicU64, Ordering};

use std::sync::Arc;

use yolu_core::skin::Rig;
use yolu_protocol::{MaterialInfo, MaterialKey, MaterialsUpdate, Model, Pose};

use crate::sets::BindReport;
use crate::state::AppState;

/// モデルの出どころ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelSource {
    /// Live Link のつながり（番号。0 はつながりの外から読んだもの）。
    LiveLink { session: u64 },
    /// ポーズを付けられるモデル（FBX・試しの人形）。`rig` は `Arc<Rig>` の同一性（`SceneModel::rig_id`）。
    Rig { rig: usize },
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

/// FBX でマテリアルの無い面につく名前（yolu-model が付ける）。未割り当ての鍵にする。
const NO_MATERIAL: &str = "マテリアルなし";

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

    /// ポーズを付けられるモデル（`Arc<Rig>`）の同一性。同じものを読み直しても別のモデルとして数える。
    pub fn rig_id(rig: &Arc<Rig>) -> usize {
        Arc::as_ptr(rig) as *const () as usize
    }

    /// ポーズを付けられるモデルから作る。マテリアルは名前だけを鍵にし（名前の無いスロットは未割り当て）、スロットはメッシュ × サブメッシュの
    /// 並びの順（3D ビューの形と同じ）。
    pub fn from_rig(rig: &Arc<Rig>) -> SceneModel {
        SceneModel {
            source: ModelSource::Rig {
                rig: Self::rig_id(rig),
            },
            name: rig.name().to_owned(),
            generation: 0,
            materials: rig
                .materials()
                .iter()
                .map(|name| MaterialInfo {
                    key: if name == NO_MATERIAL {
                        MaterialKey::Unassigned
                    } else {
                        MaterialKey::Material {
                            name: name.clone(),
                            asset: None,
                        }
                    },
                    shader: String::new(),
                    textures: vec![],
                    routes: vec![],
                })
                .collect(),
            slots: rig
                .meshes()
                .iter()
                .flat_map(|m| &m.mesh.submeshes)
                .map(|s| s.material.max(0) as u32)
                .collect(),
            meshes: rig.meshes().len(),
            vertices: rig.vertex_count(),
            triangles: rig.triangle_count(),
            revision: REVISION.fetch_add(1, Ordering::Relaxed),
            live: false,
        }
    }

    /// Live Link で受けたモデルか。
    pub fn is_link(&self) -> bool {
        matches!(self.source, ModelSource::LiveLink { .. })
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
    /// 今のポーズのセッション（FBX・試しの人形）のモデルにテクスチャセットを結び付け、セットの無いマテリアルにはセットを作る
    /// （Live Link と同じ照合）。今のセットが付かなかったときは、付いたセットの先頭へ替える（描き始められるように）。
    /// セッションが無ければ何もしない。知らせの文（作ったセット・モデルに無いセット）を返す。
    pub fn bind_rig_model(&mut self) -> Option<String> {
        let rig = self.view3d.pose.session.as_ref()?.rig.clone();
        self.model = Some(SceneModel::from_rig(&rig));
        let report = self.bind_model();
        if self.sets.current().bound.is_none() {
            if let Some(i) = self.sets.iter().position(|s| s.bound.is_some()) {
                let _ = self.switch_set(i);
            }
        }
        let lang = self.lang;
        let mut note = String::new();
        if !report.created.is_empty() {
            let names = report.created.join(lang.pick("・", ", "));
            note += &lang.pick(
                format!("新しいテクスチャセット: {names}。"),
                format!("New texture sets: {names}."),
            );
        }
        if !report.unmatched.is_empty() {
            let names = report.unmatched.join(lang.pick("・", ", "));
            if !note.is_empty() {
                note.push(' ');
            }
            note += &lang.pick(
                format!("モデルに無いセット: {names}。"),
                format!("Sets not in the model: {names}."),
            );
        }
        (!note.is_empty()).then_some(note)
    }

    /// ポーズを付けられるモデルの記録が、今のセッションのものでなくなっていたら（別のモデルに替わった）外す
    /// （記録と結び付けを解く。セットは残す）。Live Link の記録には触らない。
    pub fn sync_rig_model(&mut self) {
        let stale = match (&self.model, self.view3d.pose.session.as_ref()) {
            (
                Some(SceneModel {
                    source: ModelSource::Rig { rig },
                    ..
                }),
                session,
            ) => session.is_none_or(|s| *rig != SceneModel::rig_id(&s.rig)),
            _ => false,
        };
        if stale {
            self.model = None;
            self.bind_model();
        }
    }

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
            Some(m) if m.is_link() && m.generation == pose.generation => {}
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
            .is_some_and(|m| m.is_link() && m.generation == generation)
        {
            return false;
        }
        self.model = None;
        self.view3d.close_live_link(generation);
        self.bind_model();
        true
    }

    /// 3D ビューで描くマテリアルと、隠すマテリアルを今のテクスチャセットから決める（描いている最中は変えない）。
    /// - Live Link のモデル・ポーズを付けられるモデル（FBX・試しの人形）: 描くのは今のセットの付いたマテリアル（付いていなければ
    ///   どこにも描かない）。目を閉じたセットのマテリアルは隠す。
    /// - 試しの立方体: 今のセットの文書を貼る（マテリアル 0）。
    pub fn sync_view3d(&mut self) {
        if self.view3d.input.stroke.is_some() {
            return;
        }
        // 今のセットのマテリアルを描き、目を閉じたセットのマテリアルを隠す（付いていなければどこにも描かない）
        let bound = |app: &AppState| {
            (
                app.sets.current().bound.map_or(-1, |m| m as i32),
                app.sets
                    .iter()
                    .filter(|s| !s.visible)
                    .filter_map(|s| s.bound.map(|m| m as i32))
                    .collect::<Vec<_>>(),
            )
        };
        let (material, hidden) = match self.view3d.link_generation() {
            None => match (&self.model, self.view3d.pose.session.as_ref()) {
                // ポーズを付けられるモデル（記録が今のセッションのもの）
                (
                    Some(SceneModel {
                        source: ModelSource::Rig { rig },
                        ..
                    }),
                    Some(session),
                ) if *rig == SceneModel::rig_id(&session.rig) => bound(self),
                _ => (0, Vec::new()),
            },
            Some(g)
                if self
                    .model
                    .as_ref()
                    .is_some_and(|m| m.is_link() && m.generation == g) =>
            {
                bound(self)
            }
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

    // ───────── ポーズを付けられるモデル（FBX・試しの人形） ─────────

    use crate::lang::Lang;
    use crate::state::Action;
    use crate::view3d::pose::{self, PoseAction};

    fn load_figure(s: &mut AppState) {
        s.apply(Action::Pose(PoseAction::LoadFigure));
        assert!(s.view3d.pose.session.is_some(), "{}", s.message);
    }

    /// 最小の ASCII の FBX: 三角形 1 つ（UV つき・マテリアルなし）。
    const TRIANGLE_FBX: &str = "; FBX 7.4.0 project file\nFBXHeaderExtension:  {\n\tFBXVersion: 7400\n}\nGlobalSettings:  {\n\tVersion: 1000\n\tProperties70:  {\n\t\tP: \"UnitScaleFactor\", \"double\", \"Number\", \"\",100\n\t}\n}\nObjects:  {\n\tModel: 100, \"Model::Tri\", \"Mesh\" {\n\t\tVersion: 232\n\t}\n\tGeometry: 200, \"Geometry::Tri\", \"Mesh\" {\n\t\tVertices: *9 {\n\t\t\ta: 0,0,0,1,0,0,0,1,0\n\t\t}\n\t\tPolygonVertexIndex: *3 {\n\t\t\ta: 0,1,-3\n\t\t}\n\t\tLayerElementUV: 0 {\n\t\t\tMappingInformationType: \"ByPolygonVertex\"\n\t\t\tReferenceInformationType: \"Direct\"\n\t\t\tUV: *6 {\n\t\t\t\ta: 0,0,1,0,0,1\n\t\t\t}\n\t\t}\n\t\tLayer: 0 {\n\t\t\tLayerElement:  {\n\t\t\t\tType: \"LayerElementUV\"\n\t\t\t\tTypedIndex: 0\n\t\t\t}\n\t\t}\n\t}\n}\nConnections:  {\n\tC: \"OO\",100,0\n\tC: \"OO\",200,100\n}\n";

    #[test]
    fn a_rig_model_gets_a_set_per_material_and_paints_the_current_sets_material() {
        let mut s = AppState::new(64, 64);
        load_figure(&mut s);
        // マテリアル（肌・顔）ごとに 1 つ。最初のセットはスロット 0 のマテリアルに付き、もう 1 つは作る
        assert_eq!(s.sets.len(), 2, "{}", s.message);
        let mut bound: Vec<u32> = s.sets.iter().filter_map(|x| x.bound).collect();
        bound.sort_unstable();
        assert_eq!(bound, [0, 1]);
        let names: Vec<&str> = s.sets.iter().map(|x| x.name.as_str()).collect();
        assert!(names.contains(&"肌") && names.contains(&"顔"), "{names:?}");
        assert!(
            s.message.contains("新しいテクスチャセット"),
            "{}",
            s.message
        );
        // 3D で描くのは今のセットのマテリアル。セットを替えると描く先も替わる
        let current = s.sets.current().bound.unwrap();
        assert_eq!(s.view3d.material, current as i32);
        let other = s
            .sets
            .iter()
            .position(|x| x.bound != Some(current))
            .unwrap();
        s.switch_set(other).unwrap();
        assert_eq!(s.view3d.material, s.sets.current().bound.unwrap() as i32);
        assert_ne!(s.view3d.material, current as i32);
        // 目を閉じたセットのマテリアルの面は 3D から除く（受けた形はそのまま）
        let full = s.view3d.full_model().unwrap().triangle_count();
        let closed = s
            .sets
            .iter()
            .find(|x| x.bound == Some(current))
            .unwrap()
            .uid;
        s.toggle_set_visible(closed);
        let shown = s.view3d.model.clone().unwrap();
        assert!(shown.triangle_count() < full);
        assert!(shown
            .geometry
            .triangles()
            .iter()
            .all(|t| t.material == s.view3d.material));
        assert_eq!(s.view3d.full_model().unwrap().triangle_count(), full);
        // 英語の知らせ
        let mut e = AppState::new(64, 64);
        e.lang = Lang::En;
        load_figure(&mut e);
        assert!(e.message.contains("New texture sets"), "{}", e.message);
    }

    #[test]
    fn loading_the_same_model_again_reuses_the_sets_by_name() {
        let mut s = AppState::new(64, 64);
        load_figure(&mut s);
        let uids: Vec<u32> = s.sets.iter().map(|x| x.uid).collect();
        load_figure(&mut s);
        assert_eq!(
            s.sets.iter().map(|x| x.uid).collect::<Vec<_>>(),
            uids,
            "名前で照合して、同じセットに付け直す"
        );
        assert!(s.sets.iter().all(|x| x.bound.is_some()));
        assert!(
            !s.message.contains("新しいテクスチャセット"),
            "{}",
            s.message
        );
    }

    #[test]
    fn a_material_less_fbx_binds_the_first_set_to_the_unassigned_slot() {
        let dir = std::env::temp_dir().join(format!("yolu-app-rigbind-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tri.fbx");
        std::fs::write(&path, TRIANGLE_FBX).unwrap();
        let mut s = AppState::new(64, 64);
        pose::open_fbx(&mut s.view3d, &path);
        let mut installed = false;
        for _ in 0..500 {
            installed = pose::poll(&mut s.view3d).1;
            if installed {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(installed);
        assert!(s.bind_rig_model().is_none(), "付いただけで作らない");
        let m = s.model.as_ref().unwrap();
        assert!(matches!(m.source, ModelSource::Rig { .. }));
        assert_eq!(m.materials.len(), 1);
        assert_eq!(m.materials[0].key, MaterialKey::Unassigned);
        assert_eq!(m.slots, [0]);
        assert_eq!(s.sets.len(), 1);
        assert_eq!(s.sets.current().bound, Some(0));
        assert_eq!(s.view3d.material, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replacing_the_model_unbinds_the_sets_and_live_link_calls_ignore_a_rig_record() {
        let mut s = AppState::new(64, 64);
        load_figure(&mut s);
        // Live Link の閉じる・ポーズは、ポーズを付けられるモデルの記録には当たらない
        assert!(!s.close_link_model(0));
        assert!(s
            .receive_link_pose(&Pose {
                generation: 0,
                meshes: vec![]
            })
            .is_err());
        assert!(s.model.is_some());
        // 付いたセットは Unity 側の流し込み先が無くても警告を出さない（Unity には出さない）
        for i in 0..s.sets.len() {
            assert!(crate::panels::texture_sets::set_state(&s, i).is_none());
        }
        // 試しの立方体に替えると、セッションが終わり、記録を外して結び付けを解く（セットは残す）
        s.apply(Action::LoadDemoModel);
        pose::poll(&mut s.view3d);
        s.sync_rig_model();
        assert!(s.model.is_none());
        assert_eq!(s.sets.len(), 2);
        assert!(s.sets.iter().all(|x| x.bound.is_none()));
        assert_eq!(s.view3d.material, 0, "立方体は今のセットの文書を貼る");
        // Live Link の記録には触らない
        let _ = s.receive_link_model(&model(5), 0);
        s.sync_rig_model();
        assert!(s.model.as_ref().is_some_and(|m| m.is_link()));
    }
}
