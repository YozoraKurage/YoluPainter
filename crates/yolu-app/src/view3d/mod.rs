//! 3D ビュー: モデル（`model`）、wgpu の描画（`render`）、入力（`input`: 面に描く・回す・パン・寄る）、ブラシのカーソル。
//! 計算（当たり・ダブ・カメラの式）は core の `geometry`。ここは状態を持ち、入力を渡し、描くだけ。

pub mod gizmo;
pub mod input;
pub mod model;
pub mod pose;
pub mod render;

use std::sync::Arc;

use yolu_core::geometry::OrbitCamera;

use self::model::{ViewError, ViewModel};
use crate::state::StrokeSource;

/// 回す・パンのドラッグ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Orbit,
    Pan,
}

/// 3D ビューの入力の途中の状態。
#[derive(Default)]
pub struct SurfaceInput {
    pub stroke: Option<StrokeSource>,
    pub surface: Option<yolu_core::geometry::SurfaceStroke>,
    /// ドラッグで回している・パンしている（押したボタンと一緒に）。
    pub nav: Option<(Nav, egui::PointerButton)>,
    pub last_pointer: Option<egui::Pos2>,
    /// 最後のストロークで受けた点の数（試験用）。
    pub stroke_points: usize,
}

/// 3D ビューの状態（モデル・カメラ・描くテクスチャセット）。
///
/// モデルは 2 つの形で持つ: `full` は受けたまま（試しの立方体・Live Link のモデルとそのポーズ）、`model` は見せる形（目を閉じた
/// テクスチャセットのマテリアルの三角形を除いて組み直したもの。描画・当たり・カーソルはこちらを読むので、隠した面は見えず、
/// 描くレイも遮らない）。隠すマテリアルが無ければ同じもの。描いている最中は、どちらも入れ替えない（終わってから）。
#[derive(Default)]
pub struct View3dState {
    /// 見せる形（描画・当たり・カーソルが読む）。
    pub model: Option<Arc<ViewModel>>,
    pub camera: OrbitCamera,
    /// 描くテクスチャセット（マテリアルの組の番号。負ならどの面にも描かない）。`AppState::sync_view3d` が今のセットから決める。
    pub material: i32,
    /// 受けたままの形。
    full: Option<Arc<ViewModel>>,
    /// 見せる形から除いているマテリアル（`model` を組んだ時の値）と、次に除くマテリアル。
    shown_hidden: Vec<i32>,
    hidden: Vec<i32>,
    /// ストロークの最中に来たモデル（ストロークが終わってから入れ替える。ストロークの間はモデルを変えない）。
    pending: Option<Arc<ViewModel>>,
    /// ストロークの最中にモデルを閉じると言われた（終わってから閉じる）。
    pending_close: bool,
    revision: u32,
    pub input: SurfaceInput,
    /// ポーズの変更（スキンのあるモデル・ポーズ・ギズモ）。
    pub pose: pose::PoseEditor,
    /// 3D ビューのタブが見えているか（`YoluApp::frame` が描いた後に毎フレーム入れる。次のフレームのキー入力が読む。別のタブの
    /// 裏にあるあいだは、ポーズのモードでも取り消し・やり直しを画素へ回す）。
    pub visible: bool,
}

impl View3dState {
    /// 3D のタブが見えていて、描けるモデルがあるか（`visible` は前のフレームの結果。プロパティの欄が、3D では効かない設定に
    /// 短い理由を出す）。
    pub fn paintable_on_screen(&self) -> bool {
        self.visible && self.model.is_some()
    }

    /// 次のスナップショットの世代（モデルを作るときに使う）。
    pub fn next_revision(&mut self) -> u32 {
        self.revision += 1;
        self.revision
    }

    /// モデルを入れ替える（描いている最中なら、終わってから）。カメラはモデル全体が入る位置へ。
    pub fn set_model(&mut self, model: ViewModel) {
        let model = Arc::new(model);
        if self.input.stroke.is_some() {
            self.pending = Some(model);
            self.pending_close = false;
            return;
        }
        self.apply_model(model);
    }

    fn apply_model(&mut self, model: Arc<ViewModel>) {
        let keep_camera = self
            .full
            .as_ref()
            .is_some_and(|m| m.name == model.name && m.triangle_count() == model.triangle_count());
        if !keep_camera {
            self.camera = OrbitCamera::framing(&model.geometry.bounds());
        }
        self.full = Some(model);
        self.rebuild_shown();
    }

    /// 見せる形を組み直す（隠すマテリアルが無ければ受けたまま。隠すと三角形が残らなければ見せる形は無し）。
    fn rebuild_shown(&mut self) {
        self.shown_hidden = self.hidden.clone();
        let Some(full) = self.full.clone() else {
            self.model = None;
            return;
        };
        let hides = |m: i32| self.hidden.contains(&m);
        if !full
            .meshes
            .iter()
            .any(|mesh| mesh.submeshes.iter().any(|s| hides(s.material)))
        {
            self.model = Some(full);
            return;
        }
        let meshes = full
            .meshes
            .iter()
            .map(|mesh| {
                let mut mesh = mesh.clone();
                mesh.submeshes.retain(|s| !hides(s.material));
                mesh
            })
            .collect();
        let revision = self.next_revision();
        self.model = ViewModel::new(&full.name, meshes, full.materials.clone(), revision)
            .ok()
            .map(|mut m| {
                m.link_generation = full.link_generation;
                m.demo = full.demo;
                Arc::new(m)
            });
    }

    /// 隠すマテリアル（目を閉じたテクスチャセットのもの）を決める。変わったら見せる形を組み直す（描いている最中は終わってから）。
    pub fn set_hidden(&mut self, mut hidden: Vec<i32>) {
        hidden.sort_unstable();
        hidden.dedup();
        if hidden == self.hidden {
            return;
        }
        self.hidden = hidden;
        if self.input.stroke.is_none() && self.pending.is_none() {
            self.rebuild_shown();
        }
    }

    /// 試しの立方体を読む。
    pub fn load_demo(&mut self) {
        let revision = self.next_revision();
        self.material = 0;
        self.set_model(ViewModel::demo(revision));
    }

    /// Live Link で受けたモデルを読む（描くテクスチャセットは `AppState::sync_view3d` が決める）。
    pub fn load_live_link(&mut self, model: &yolu_protocol::Model) -> Result<(), ViewError> {
        let revision = self.next_revision();
        let m = ViewModel::from_live_link(model, revision)?;
        self.set_model(m);
        Ok(())
    }

    /// Live Link のポーズを当てる（描いている最中なら、待たせているモデルに当てて終わってから入れ替える。カメラはそのまま）。
    pub fn apply_live_link_pose(&mut self, pose: &yolu_protocol::Pose) -> Result<(), ViewError> {
        let base = self
            .pending
            .as_ref()
            .or(self.full.as_ref())
            .cloned()
            .ok_or(ViewError::NoPoseBase)?;
        let revision = self.next_revision();
        let posed = base.with_pose(pose, revision)?;
        self.set_model(posed);
        Ok(())
    }

    /// Live Link のモデル（その世代）を閉じる（描いている最中なら、終わってから）。試しの立方体・別の世代なら何もしない。
    pub fn close_live_link(&mut self, generation: u32) {
        let target = self.pending.as_ref().or(self.full.as_ref());
        if target.and_then(|m| m.link_generation) != Some(generation) {
            return;
        }
        if self.input.stroke.is_some() {
            self.pending = None;
            self.pending_close = true;
            return;
        }
        self.full = None;
        self.model = None;
    }

    /// 受けたままの形が Live Link のモデルなら、その世代（待っているモデルがあればそちら）。
    pub fn link_generation(&self) -> Option<u32> {
        self.pending
            .as_ref()
            .or(self.full.as_ref())
            .and_then(|m| m.link_generation)
    }

    /// 受けたままの形（試験用）。
    pub fn full_model(&self) -> Option<&Arc<ViewModel>> {
        self.full.as_ref()
    }

    /// カメラをモデル全体が入る位置へ戻す。
    pub fn frame_model(&mut self) {
        if let Some(m) = &self.full {
            self.camera = OrbitCamera::framing(&m.geometry.bounds());
        }
    }

    /// ストロークが終わったら、待たせていたモデル・閉じる・隠すを当てる。
    pub(crate) fn stroke_ended(&mut self) {
        self.input.stroke = None;
        self.input.surface = None;
        if std::mem::take(&mut self.pending_close) {
            self.full = None;
            self.model = None;
        } else if let Some(m) = self.pending.take() {
            self.apply_model(m);
        } else if self.shown_hidden != self.hidden {
            self.rebuild_shown();
        }
    }

    /// いちばん新しい受けたままの形（ストロークの後に入れ替わるのを待っているものがあればそれ）。
    pub fn latest_model(&self) -> Option<&Arc<ViewModel>> {
        self.pending.as_ref().or(self.full.as_ref())
    }

    /// 待っているモデルがあるか（試験用）。
    pub fn has_pending_model(&self) -> bool {
        self.pending.is_some()
    }
}
