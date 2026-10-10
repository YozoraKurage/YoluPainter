//! 3D ビュー: モデル（`model`）、wgpu の描画（`render`）、入力（`input`: 面に描く・回す・パン・寄る）、ブラシのカーソル。
//! 計算（当たり・ダブ・カメラの式）は core の `geometry`。ここは状態を持ち、入力を渡し、描くだけ。

pub mod axis_gizmo;
pub mod brdf;
pub mod display;
pub mod draft;
pub mod environment;
pub mod gizmo;
pub mod input;
mod liltoon_wgsl;
pub mod look_gpu;
pub mod model;
pub mod navigation;
pub mod other_sets;
pub mod pacing;
pub mod paint;
pub mod pose;
mod quick;
pub mod received_layers;
pub mod render;
pub mod select;
pub mod selection_overlay;
pub mod shape_gizmo;
pub mod tangents;
pub mod user_layers;

use std::sync::Arc;

use yolu_core::geometry::OrbitCamera;
use yolu_core::geometry::{model_triangles, BvhUpdate, ModelMesh, Submesh, SurfaceGeometry};

use self::model::{ViewError, ViewModel};
use crate::state::StrokeSource;

/// 回す・パンのドラッグ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Orbit,
    /// 軸の向きへ吸い付く回転（Alt + 左）。
    SnapOrbit,
    Pan,
    /// Ctrl+Space（寄る・引く）。
    Zoom,
}

/// 3D ビューの入力の途中の状態。
#[derive(Default)]
pub struct SurfaceInput {
    pub stroke: Option<StrokeSource>,
    /// 面のストロークを離した（マウス・ペンを離した・ウィンドウのフォーカスを失った）。持ち越したダブが残っていれば、フレームごとに時間の枠で
    /// 塗り続け、塗り終えたら文書のストロークを確定する（それまでは描いている最中のまま。新しい点は受けない）。
    pub released: bool,
    /// 1 フレームに面のダブを塗ってよい時間の上書き（試験用。None なら溜まった仕事の見込みで決める。`pacing::frame_budget`）。
    pub paint_budget: Option<std::time::Duration>,
    /// 今のストロークの 1 ダブの平均の時間（溜まった仕事の見込みの元。ストロークが終わると忘れる）。
    pub dab_clock: pacing::DabClock,
    /// 直前の `paint_queued` が今のフレームに塗ったダブの数（同じフレームの表示の同期の時間を、ダブあたりに直すため。`note_sync` が取る）。
    pub frame_dabs: usize,
    /// ペンの今の押し（2D の `CanvasInput::pen_press` と同じ。押した瞬間に終わるバケツ・ID の色で選択を押し直さない印も兼ねる）。
    pub pen_press: Option<crate::pen::PenPress>,
    /// Ctrl+Space の拡縮のドラッグ（`nav` が `Nav::Zoom` のあいだ）。
    pub zoom: Option<crate::gesture::ZoomDrag>,
    pub surface: Option<yolu_core::geometry::SurfaceStroke>,
    /// クイックマスクのブラシ・消しゴムの 3D のストローク（選択範囲の被覆を集める。被覆は `AppState::sel` の選択ペンが持つ）。
    pub cover: Option<yolu_core::geometry::SurfaceCoverStroke>,
    /// ドラッグで回している・パンしている（押したボタンと一緒に）。
    pub nav: Option<(Nav, egui::PointerButton)>,
    pub navigation: Option<navigation::Drag>,
    pub last_pointer: Option<egui::Pos2>,
    /// 最後のストロークで受けた点の数（試験用）。
    pub stroke_points: usize,
    /// 描いているストロークに固めた 3D の対称（対称の面の表示と、写しのカーソルはこれを読む）。
    pub symmetry: Option<yolu_core::geometry::SurfaceSymmetrySetup>,
    /// クローンの元を決める組み合わせ（既定は Alt + 左）で押した点とボタン（動かさずに離したらクローンの元にする。動かしたらドラッグの操作だけ）。
    pub clone_press: Option<(egui::Pos2, egui::PointerButton)>,
    /// 視点の移動の間に押されていた移動キー（右ボタンを先に離しても、押したままの間は、キーの繰り返しをキーの表に渡さない。`keymap::take_fly_keys`）。
    pub fly_held: Vec<egui::Key>,
    /// 右ボタン（ペンのサイドボタン）を押した点と、押したときの見本（動かさずに離したらスポイト。動かしたら回すだけ）。
    pub eyedrop: Option<crate::eyedrop::RightPress>,
    /// 確定した 3D ストロークの終点（面の点。Shift + 押しの直線の始め。今のカメラで画面へ写し直して使う。今のモデルの世代のときだけ使う）。
    pub previous_end: Option<yolu_core::geometry::SurfaceHit>,
    /// 描いている 3D ストロークの今の終点（画面の点。Shift のぶれの抑え・向きの固定を当てた後）。
    pub last_point: Option<egui::Pos2>,
    /// 描いている 3D ストロークの入力で、面に当たった最後の点（確定すると `previous_end` になる。モデルの外へ出て終えても、面の上の終わりを覚える）。
    pub last_hit: Option<yolu_core::geometry::SurfaceHit>,
    /// Shift で始めたストロークの、押した点のぶれの抑えと向きの固定（画面の点。2D の `CanvasInput::shift_hold` と同じ決まり）。
    pub shift_hold: Option<crate::state::ShiftHold>,
    /// 定規にスナップするストロークの寄せ先（表示域の画面の点。ストロークの始めに凍結する）。
    pub ruler_constraint: Option<crate::drafting::Constraint>,
    /// グラデーション・図形・定規のドラッグの途中（画面の上の形。離すまで文書を変えない）。
    pub draft: Option<draft::SurfaceDraft>,
}

impl SurfaceInput {
    /// このフレームの 3D の表示の同期（変わったタイルの合成・上げ・ミップ）に `sync` かかった。このフレームに塗ったダブの数で割って、
    /// 1 ダブの時間の見積もりに足す（塗っていないフレームでは何もしない）。
    pub fn note_sync(&mut self, sync: std::time::Duration) {
        let painted = std::mem::take(&mut self.frame_dabs);
        self.dab_clock.record_sync(painted, sync);
    }

    /// 押しの印と、回し・パン・拡縮の途中を全部捨てる（ビューが隠れて、ペンの離れ・ボタンの離れを受け取れなかったとき。印が残ると、次の押しを
    /// 前の押しの続きとして扱い、Alt で押した点の近くで離せばクローンの元を決めてしまう）。描いているストロークは別の持ち主が終える。
    pub fn drop_presses(&mut self) {
        self.pen_press = None;
        self.nav = None;
        self.navigation = None;
        self.zoom = None;
        self.clone_press = None;
        self.eyedrop = None;
        self.draft = None;
    }
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
    /// 今の正投影は、軸の向きに入ったので自動で替えたもの（回して軸から外れたら透視へ戻す。`navigation::after_orbit`）。
    pub auto_orthographic: bool,
    /// 描くテクスチャセット（マテリアルの組の番号。負ならどの面にも描かない）。`AppState::sync_view3d` が今のセットから決める。
    pub material: i32,
    /// メモリの予算が足りずに、絵を 3D に見せていないセットのマテリアル（今のセットでないセットだけ。描くたびに `other_sets::finish` が入れる。
    /// テクスチャセットの一覧が印にする）。3D のタブが前に出ていないあいだは描かれず更新されないので、モデルが替わる・閉じる・今のセットの
    /// マテリアルが替わるときに空にする（別のモデルのマテリアルの番号の印を、別のセットの行に残さない）。
    pub unpainted: Vec<i32>,
    /// 受けたままの形。
    full: Option<Arc<ViewModel>>,
    /// 見せる形から除いているマテリアル（`model` を組んだ時の値）と、次に除くマテリアル。
    shown_hidden: Vec<i32>,
    hidden: Vec<i32>,
    /// 見せる形から除く面（受けたままの形の三角形の番号ごと。ポーズの画面の「面を隠す」）と、`model` を組んだ時の値。
    face_mask: Option<Arc<pose::hide::FaceMask>>,
    shown_mask: Option<Arc<pose::hide::FaceMask>>,
    /// 見せる形の三角形の番号 → 受けたままの形の番号（何かを隠しているときだけ。隠していなければ空）。
    shown_to_full: Vec<u32>,
    /// 隠した形の幾何（同じ隠し方でポーズだけが替わったとき、組み直さず box だけ当て直す元）。
    shown_base: Option<ShownBase>,
    /// ストロークの最中に来たモデル（ストロークが終わってから入れ替える。ストロークの間はモデルを変えない）。
    pending: Option<Arc<ViewModel>>,
    /// ストロークの最中にモデルを閉じると言われた（終わってから閉じる）。
    pending_close: bool,
    /// 三角形・UV・スロットの並びが替わるモデルに入れ替えたときの、替わる前の形（アプリが 3D のパスの付け直しに使って取り出す。
    /// ポーズだけの入れ替えでは残さない。続けて替わったら、最初の形のまま）。
    replaced: Option<Arc<ViewModel>>,
    revision: u32,
    pub input: SurfaceInput,
    /// 表示の設定（マテリアル・中立・チャンネルだけ、光・環境・トーンマッピング）。
    pub display: display::Display,
    /// ポーズの変更（スキンのあるモデル・ポーズ・ギズモ）。
    pub pose: pose::PoseEditor,
    /// 3D の塗りの切り替え（隠れた所・裏の面・面の向きの弱め・継ぎ目のにじみ）。ストロークの始めに固める。設定のファイルに書く
    /// （`Settings::view3d_paint`）。
    pub projection: yolu_core::geometry::ProjectionSettings,
    /// 3D の塗りの切り替えのスライダーをドラッグしている（その間は設定のファイルに書かず、離したときの値を書く）。
    pub projection_dragging: bool,
    /// 3D ビューのタブが見えているか（`YoluApp::frame` が描いた後に毎フレーム入れる。次のフレームのキー入力が読む。別のタブの
    /// 裏にあるあいだは、ポーズのモードでも取り消し・やり直しを画素へ回す）。
    pub visible: bool,
    /// 最後に描いた 3D ビューの表示域（画面の点。パイから「収める」ときの縦横の比・G/R/S の線と札）。
    pub view_rect: Option<egui::Rect>,
    /// 最後に 3D ビューを描いたウィンドウ（G/R/S は、このウィンドウのキーとポインタで動かす）。
    pub viewport: Option<egui::ViewportId>,
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
        if let Some(old) = &self.full {
            if !same_structure(&old.geometry, &model.geometry) {
                self.replaced.get_or_insert_with(|| old.clone());
            }
        }
        let keep_camera = self
            .full
            .as_ref()
            .is_some_and(|m| m.name == model.name && m.triangle_count() == model.triangle_count());
        if !keep_camera {
            self.reframe(&model.geometry.bounds());
        }
        self.full = Some(model);
        self.unpainted.clear();
        self.rebuild_shown();
    }

    /// 見せる形を組み直す（隠すマテリアルも隠す面も無ければ受けたまま。隠すと三角形が残らなければ見せる形は無し）。
    /// 見せる三角形だけを残したメッシュを組み、幾何は、同じ隠し方でポーズだけが替わったなら前の幾何の箱を当て直し、そうでなければ
    /// 受けたままの形の隣り合わせを写して作る（溶接し直さない。ブラシの大きさの基準も受けたままの形のもの）。
    fn rebuild_shown(&mut self) {
        self.shown_hidden = self.hidden.clone();
        self.shown_mask = self.face_mask.clone();
        self.shown_to_full.clear();
        let Some(full) = self.full.clone() else {
            self.model = None;
            self.shown_base = None;
            return;
        };
        let total = full.triangle_count();
        let mask = self
            .face_mask
            .clone()
            .filter(|m| m.len() == total && m.hidden_count() > 0);
        let hides = |m: i32| self.hidden.contains(&m);
        if mask.is_none()
            && !full
                .meshes
                .iter()
                .any(|mesh| mesh.submeshes.iter().any(|s| hides(s.material)))
        {
            self.model = Some(full);
            self.shown_base = None;
            return;
        }
        let mut keep = vec![false; total];
        let mut map: Vec<u32> = Vec::new();
        let mut base = 0usize;
        let mut meshes = Vec::with_capacity(full.meshes.len());
        for mesh in &full.meshes {
            let mut out = ModelMesh {
                name: mesh.name.clone(),
                positions: mesh.positions.clone(),
                normals: mesh.normals.clone(),
                uvs: mesh.uvs.clone(),
                submeshes: Vec::new(),
            };
            for s in &mesh.submeshes {
                let n = s.indices.len() / 3;
                if !hides(s.material) {
                    let mut indices = Vec::with_capacity(s.indices.len());
                    for (k, t) in s.indices.as_chunks::<3>().0.iter().enumerate() {
                        let at = base + k;
                        if mask.as_ref().is_some_and(|m| m.is_hidden(at)) {
                            continue;
                        }
                        indices.extend_from_slice(t);
                        keep[at] = true;
                        map.push(at as u32);
                    }
                    // 面を隠して三角形が無くなったサブメッシュは、隠したマテリアルと同じく落とす
                    if !indices.is_empty() || n == 0 {
                        out.submeshes.push(Submesh {
                            material: s.material,
                            indices,
                        });
                    }
                }
                base += n;
            }
            meshes.push(out);
        }
        // 見せる面が 1 つも無い（全部のマテリアルを隠した）ときは、形を持たない（3D ビューの操作の途中も終わる）
        if !keep.iter().any(|k| *k) {
            self.model = None;
            self.shown_base = None;
            return;
        }
        self.shown_base = self
            .shown_base
            .take()
            .filter(|b| b.hidden == self.hidden && same_mask(&b.mask, &mask) && b.total == total);
        let revision = self.next_revision();
        let triangles = || model_triangles(&meshes);
        let reused = self.shown_base.as_ref().and_then(|b| {
            b.geometry
                .reposition(triangles()?, revision, BvhUpdate::Refit)
                .ok()
        });
        let geometry = match reused {
            Some(g) => Some(g),
            None => triangles().and_then(|t| full.geometry.restrict(&keep, t, revision).ok()),
        };
        let Some(geometry) = geometry else {
            self.model = None;
            self.shown_base = None;
            return;
        };
        let geometry = Arc::new(geometry);
        let mut shown =
            ViewModel::with_geometry(&full.name, meshes, full.materials.clone(), geometry.clone());
        shown.link_generation = full.link_generation;
        shown.demo = full.demo;
        self.shown_base = Some(ShownBase {
            hidden: self.hidden.clone(),
            mask,
            total,
            geometry,
        });
        self.shown_to_full = map;
        self.model = Some(Arc::new(shown));
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

    /// 見せる形から除く面を決める（ポーズの画面の「面を隠す」。三角形は受けたままの形の番号。変わったら見せる形を組み直す。
    /// 描いている最中は終わってから。三角形の数が今のモデルと違う印は使わない）。
    pub fn set_face_mask(&mut self, mask: Option<Arc<pose::hide::FaceMask>>) {
        if same_mask(&mask, &self.face_mask) {
            return;
        }
        self.face_mask = mask;
        if self.input.stroke.is_none() && self.pending.is_none() {
            self.rebuild_shown();
        }
    }

    /// 今決めている隠す面（見せる形へはまだ入っていないことがある）。
    pub fn face_mask(&self) -> Option<&Arc<pose::hide::FaceMask>> {
        self.face_mask.as_ref()
    }

    /// 受けたままの形のこの三角形を、今は見せる形から除いている面として隠しているか（マテリアルを隠しているものは含めない）。
    pub fn is_face_hidden(&self, full_triangle: u32) -> bool {
        let total = self.full.as_ref().map_or(0, |m| m.triangle_count());
        self.shown_mask
            .as_ref()
            .is_some_and(|m| m.len() == total && m.is_hidden(full_triangle as usize))
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
        self.unpainted.clear();
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
        if let Some(bounds) = self.full.as_ref().map(|m| m.geometry.bounds()) {
            self.reframe(&bounds);
        }
    }

    /// カメラを、この境界が全部入る既定の向きの位置へ置き直す。手で選んだ正投影は保ち、軸の向きで自動で替えた正投影は透視へ戻す
    /// （既定の向きは軸の向きではない）。
    fn reframe(&mut self, bounds: &yolu_core::geometry::Bounds) {
        let orthographic = self.camera.is_orthographic() && !self.auto_orthographic;
        self.camera = OrbitCamera::framing(bounds);
        self.camera.set_orthographic(orthographic);
        self.auto_orthographic = false;
    }

    /// ストロークが終わったら、待たせていたモデル・閉じる・隠すを当てる。
    pub(crate) fn stroke_ended(&mut self) {
        self.input.stroke = None;
        self.input.released = false;
        self.input.dab_clock = pacing::DabClock::default();
        self.input.frame_dabs = 0;
        self.input.surface = None;
        self.input.cover = None;
        self.input.symmetry = None;
        self.input.last_point = None;
        self.input.last_hit = None;
        self.input.shift_hold = None;
        self.input.ruler_constraint = None;
        if std::mem::take(&mut self.pending_close) {
            self.full = None;
            self.model = None;
            self.unpainted.clear();
        } else if let Some(m) = self.pending.take() {
            self.apply_model(m);
        } else if self.shown_hidden != self.hidden || !same_mask(&self.shown_mask, &self.face_mask)
        {
            self.rebuild_shown();
        }
    }

    /// いちばん新しい受けたままの形（ストロークの後に入れ替わるのを待っているものがあればそれ）。
    pub fn latest_model(&self) -> Option<&Arc<ViewModel>> {
        self.pending.as_ref().or(self.full.as_ref())
    }

    /// 見せる形の三角形の番号を、受けたままの形の番号に直す（隠したマテリアルのサブメッシュや隠した面を除いて組んだ形は、そのぶん番号が
    /// ずれる。3D のパスの点の番号は、保存と Unity 版と同じ受けたままの形のもの）。範囲外なら None。
    pub fn full_triangle(&self, shown: u32) -> Option<u32> {
        let (full, model) = (self.full.as_ref()?, self.model.as_ref()?);
        if Arc::ptr_eq(full, model) {
            return ((shown as usize) < model.triangle_count()).then_some(shown);
        }
        self.shown_to_full.get(shown as usize).copied()
    }

    /// このマテリアルの面を、今は見せる形から除いているか。
    pub fn is_material_hidden(&self, material: i32) -> bool {
        self.shown_hidden.contains(&material)
    }

    /// モデルの三角形・UV・スロットの並びが替わったときの、替わる前の形（取り出す。無ければ None）。
    pub fn take_replaced(&mut self) -> Option<Arc<ViewModel>> {
        self.replaced.take()
    }

    /// 待っているモデルがあるか（試験用）。
    pub fn has_pending_model(&self) -> bool {
        self.pending.is_some()
    }
}

/// 三角形の並び・UV・レンダラーとスロットが同じか（位置・法線は見ない。3D のパスの指紋と同じ範囲を、ハッシュを作らずに比べる）。
fn same_structure(
    a: &yolu_core::geometry::SurfaceGeometry,
    b: &yolu_core::geometry::SurfaceGeometry,
) -> bool {
    a.triangle_count() == b.triangle_count()
        && a.triangles().iter().zip(b.triangles()).all(|(x, y)| {
            x.renderer == y.renderer
                && x.material_slot == y.material_slot
                && x.uv_a == y.uv_a
                && x.uv_b == y.uv_b
                && x.uv_c == y.uv_c
        })
}

/// 隠した形の幾何と、それを作ったときの隠し方（同じ隠し方でポーズだけが替わったときに使い回す）。
struct ShownBase {
    hidden: Vec<i32>,
    mask: Option<Arc<pose::hide::FaceMask>>,
    total: usize,
    geometry: Arc<SurfaceGeometry>,
}

fn same_mask(a: &Option<Arc<pose::hide::FaceMask>>, b: &Option<Arc<pose::hide::FaceMask>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}
