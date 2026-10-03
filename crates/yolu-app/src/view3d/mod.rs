//! 3D ビュー: モデル（`model`）、wgpu の描画（`render`）、入力（`input`: 面に描く・回す・パン・寄る）、ブラシのカーソル。
//! 計算（当たり・ダブ・カメラの式）は core の `geometry`。ここは状態を持ち、入力を渡し、描くだけ。

pub mod input;
pub mod model;
pub mod render;

use std::sync::Arc;

use yolu_core::geometry::OrbitCamera;

use self::model::ViewModel;
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
#[derive(Default)]
pub struct View3dState {
    pub model: Option<Arc<ViewModel>>,
    pub camera: OrbitCamera,
    /// 描くテクスチャセット（マテリアルの組の番号）。文書 1 つが 1 つのテクスチャセットを描く。
    pub material: i32,
    /// ストロークの最中に来たモデル（ストロークが終わってから入れ替える。ストロークの間はモデルを変えない）。
    pending: Option<Arc<ViewModel>>,
    revision: u32,
    pub input: SurfaceInput,
}

impl View3dState {
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
            return;
        }
        self.apply_model(model);
    }

    fn apply_model(&mut self, model: Arc<ViewModel>) {
        let keep_camera = self
            .model
            .as_ref()
            .is_some_and(|m| m.name == model.name && m.triangle_count() == model.triangle_count());
        if !keep_camera {
            self.camera = OrbitCamera::framing(&model.geometry.bounds());
        }
        self.model = Some(model);
    }

    /// 試しの立方体を読む。
    pub fn load_demo(&mut self) {
        let revision = self.next_revision();
        self.material = 0;
        self.set_model(ViewModel::demo(revision));
    }

    /// Live Link で受けたモデルを読む（描くテクスチャセットはそのまま。範囲の外なら 0）。
    pub fn load_live_link(&mut self, model: &yolu_protocol::Model) -> Result<(), String> {
        let revision = self.next_revision();
        let m = ViewModel::from_live_link(model, revision)?;
        if self.material < 0 || self.material as usize >= m.materials.len().max(1) {
            self.material = 0;
        }
        self.set_model(m);
        Ok(())
    }

    /// Live Link のポーズを当てる（描いている最中なら、待たせているモデルに当てて終わってから入れ替える。カメラはそのまま）。
    pub fn apply_live_link_pose(&mut self, pose: &yolu_protocol::Pose) -> Result<(), String> {
        let base = self
            .pending
            .as_ref()
            .or(self.model.as_ref())
            .cloned()
            .ok_or("ポーズを当てるモデルがありません")?;
        let revision = self.next_revision();
        let posed = base.with_pose(pose, revision)?;
        self.set_model(posed);
        Ok(())
    }

    /// カメラをモデル全体が入る位置へ戻す。
    pub fn frame_model(&mut self) {
        if let Some(m) = &self.model {
            self.camera = OrbitCamera::framing(&m.geometry.bounds());
        }
    }

    /// ストロークが終わったら、待たせていたモデルを入れる。
    pub(crate) fn stroke_ended(&mut self) {
        self.input.stroke = None;
        self.input.surface = None;
        if let Some(m) = self.pending.take() {
            self.apply_model(m);
        }
    }

    /// 待っているモデルがあるか（試験用）。
    pub fn has_pending_model(&self) -> bool {
        self.pending.is_some()
    }
}
