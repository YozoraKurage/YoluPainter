//! 3D ビュー: モデル（`model`）、wgpu の描画（`render`）、入力（`input`: 面に描く・回す・パン・寄る）、ブラシのカーソル。
//! 計算（当たり・ダブ・カメラの式）は core の `geometry`。ここは状態を持ち、入力を渡し、描くだけ。

pub mod axis_gizmo;
pub mod bone_marks;
pub mod brdf;
pub mod display;
pub mod draft;
pub mod environment;
pub mod gizmo;
mod held;
pub mod hide;
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
pub mod views;

use std::sync::Arc;

use yolu_core::geometry::OrbitCamera;
use yolu_core::geometry::{model_triangles, BvhUpdate, ModelMesh, Submesh, SurfaceGeometry};

use self::model::{ViewError, ViewModel};
use crate::state::StrokeSource;

/// 回す・パンのドラッグ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Orbit,
    /// ドラッグした向きの、90° 隣の軸の視点へ切り替える回転（Alt + 右）。
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
    pub pen_press: Option<crate::input::Press>,
    /// マウスのツールの押し（2D の `CanvasInput::mouse_press` と同じ。始めたボタンを離したらツールが終える。無ければ左ボタン）。
    pub mouse_press: Option<crate::input::Press>,
    /// Ctrl+Space の拡縮のドラッグ（`nav` が `Nav::Zoom` のあいだ）。
    pub zoom: Option<crate::gesture::ZoomDrag>,
    pub surface: Option<yolu_core::geometry::SurfaceStroke>,
    /// クイックマスクのブラシ・消しゴムの 3D のストローク（選択範囲の被覆を集める。被覆は `AppState::sel` の選択ペンが持つ）。
    pub cover: Option<yolu_core::geometry::SurfaceCoverStroke>,
    /// ドラッグで回している・パンしている（押したボタンと一緒に）。
    pub nav: Option<(Nav, crate::state::key::PointerButton)>,
    pub navigation: Option<navigation::Drag>,
    pub last_pointer: Option<egui::Pos2>,
    /// 最後のストロークで受けた点の数（試験用）。
    pub stroke_points: usize,
    /// 描いているストロークに固めた 3D の対称（対称の面の表示と、写しのカーソルはこれを読む）。
    pub symmetry: Option<yolu_core::geometry::SurfaceSymmetrySetup>,
    /// クローンの元を決める組み合わせ（既定は Alt + 左）で押した点とボタン（動かさずに離したらクローンの元にする。動かしたらドラッグの操作だけ）。
    pub clone_press: Option<(egui::Pos2, crate::state::key::PointerButton)>,
    /// 視点の移動の間に押されていた移動キー（右ボタンを先に離しても、押したままの間は、キーの繰り返しをキーの表に渡さない。`keymap::take_fly_keys`）。
    pub fly_held: Vec<crate::state::key::Key>,
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
        self.mouse_press = None;
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
/// モデルは 2 つの形で持つ: `full` は受けたまま（試しの立方体・Live Link のモデルとそのポーズ）、`model` は今のビューの見せる形（目を
/// 閉じたテクスチャセットのマテリアルの三角形と、そのビューで隠した面を除いて作り直し、そのビューのポーズのプリセットがあればその形に
/// したもの。描画・当たり・カーソルはこちらを読むので、隠した面は見えず、描くレイも遮らない）。隠す物もビューのポーズも無ければ同じもの。
/// ほかのビューの見せる形は、そのビューの `ViewState::shape` が持ち、今のビューを替えるときに入れ替える。同じ元・同じ隠し方のビューは
/// 同じ形（同じ世代）を共有する。描いている最中は、どれも入れ替えない（終わってから）。
#[derive(Default)]
pub struct View3dState {
    /// 今のビューの見せる形（描画・当たり・カーソルが読む）。
    pub model: Option<Arc<ViewModel>>,
    /// 今のビュー（入力を受け、キーの視点の操作が効くビュー）の状態。ほかのビューの状態は `parked` に置き、今のビューを替えるときに
    /// 入れ替える（`views`）。
    pub view: ViewState,
    /// 今のビューでないビューの状態（足した順）。
    parked: Vec<ViewState>,
    /// 描くテクスチャセット（マテリアルの組の番号。負ならどの面にも描かない）。`AppState::sync_view3d` が今のセットから決める。
    pub material: i32,
    /// メモリの予算が足りずに、絵を 3D に見せていないセットのマテリアル（今のセットでないセットだけ。描くたびに `other_sets::finish` が入れる。
    /// テクスチャセットの一覧が印にする）。3D のタブが前に出ていないあいだは描かれず更新されないので、モデルが替わる・閉じる・今のセットの
    /// マテリアルが替わるときに空にする（別のモデルのマテリアルの番号の印を、別のセットの行に残さない）。
    pub unpainted: Vec<i32>,
    /// 受けたままの形。
    full: Option<Arc<ViewModel>>,
    /// 見せる形から除くマテリアル（目を閉じたテクスチャセットのもの。全部のビューで共通。作った時の値はビューごとの `Shape::hidden`）。
    hidden: Vec<i32>,
    /// ストロークの最中に来たモデル（ストロークが終わってから入れ替える。ストロークの間はモデルを変えない）。
    pending: Option<Arc<ViewModel>>,
    /// ストロークの最中にモデルを閉じると言われた（終わってから閉じる）。
    pending_close: bool,
    /// 三角形・UV・スロットの並びが替わるモデルに入れ替えたときの、替わる前の形（アプリが 3D のパスの付け直しに使って取り出す。
    /// ポーズだけの入れ替えでは残さない。続けて替わったら、最初の形のまま）。
    replaced: Option<Arc<ViewModel>>,
    revision: u32,
    /// ポーズの変更（スキンのあるモデル・ポーズ・ギズモ）。
    pub pose: pose::PoseEditor,
    /// ハイドのモードの、全部のビューで共通の状態（選ぶツール・設定・UV アイランドの索引）。
    pub hiding: hide::HideShared,
    /// 3D の塗りの切り替え（隠れた所・裏の面・面の向きの弱め・継ぎ目のにじみ）。ストロークの始めに固める。設定のファイルに書く
    /// （`Settings::view3d_paint`）。
    pub projection: yolu_core::geometry::ProjectionSettings,
    /// 3D の塗りの切り替えのスライダーをドラッグしている（その間は設定のファイルに書かず、離したときの値を書く）。
    pub projection_dragging: bool,
    /// 表示の種類のメニューを開いたビュー（メニューで選んだ種類を当てるビュー）。
    pub shading_menu: ViewId,
    /// 3D ビューのタブのどれかが見えているか（`YoluApp::frame` が描いた後に毎フレーム入れる。次のフレームのキー入力が読む。全部の
    /// ビューが別のタブの裏にあるあいだは、ポーズのモードでも取り消し・やり直しを画素へ回す）。
    pub visible: bool,
}

impl ViewState {
    /// カメラを、この境界が全部入る既定の向きの位置へ置き直す。手で選んだ正投影は保ち、軸の向きで自動で替えた正投影は透視へ戻す
    /// （既定の向きは軸の向きではない）。
    fn reframe(&mut self, bounds: &yolu_core::geometry::Bounds) {
        let orthographic = self.camera.is_orthographic() && !self.auto_orthographic;
        self.camera = OrbitCamera::framing(bounds);
        self.camera.set_orthographic(orthographic);
        self.auto_orthographic = false;
    }
}

/// 3D ビュー 1 つの見分け（0 が最初のビュー。タブの「3D ビュー n」の n は番号 + 1）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewId(pub u32);

impl ViewId {
    /// 最初のビュー（「ウィンドウ」のメニューの 3D ビュー。閉じられない）。
    pub const FIRST: ViewId = ViewId(0);

    /// タブの名前に出す番号（1 から）。
    pub fn number(self) -> u32 {
        self.0 + 1
    }
}

/// ビューごとの状態: カメラ・正投影の切り替え・表示の設定（表示の種類はビューごと。光・環境・トーンマッピング・仕上げは全部のビューで同じ値を
/// 持つ。`View3dState::apply_display`）・表示域・そのビューで始まった入力。モデル・描くセット・塗りの設定・ポーズは全部のビューで共通
/// （`View3dState` が持つ）。ビューごとに持たせたい物は、ここへ足す（今のビューを替えるとき、まとめて入れ替わる）。
#[derive(Default)]
pub struct ViewState {
    pub id: ViewId,
    pub camera: OrbitCamera,
    /// 今の正投影は、軸の向きに入ったので自動で替えたもの（回して軸から外れたら透視へ戻す。`navigation::after_orbit`）。
    pub auto_orthographic: bool,
    /// 表示の設定（マテリアル・中立・チャンネルだけ、光・環境・トーンマッピング）。
    pub display: display::Display,
    /// このビューで始まった入力の途中の状態（ストロークは始めたビューのカメラで最後まで。途中で今のビューは替えない）。
    pub input: SurfaceInput,
    /// 最後に描いた表示域（画面の点。パイから「収める」ときの縦横の比・G/R/S の線と札）。
    pub view_rect: Option<egui::Rect>,
    /// このビューの隠し方と、ハイドのモードの画面の状態（引き出し・シェルフの開閉・選んだ UV アイランド）。
    pub hide: hide::ViewHide,
    /// このビューのポーズ（シェルフのポーズのプリセット。今のポーズでなければ、その形を `pose_model` に持つ）。
    pub pose: hide::ViewPose,
    /// このビューの見せる形から除く面（受けたままの形の三角形の番号ごと。ハイドのモードの UV アイランド・ボーンで隠した面の和。
    /// シェルフの目を閉じていれば None）。見せる形へ入れるのは `View3dState::set_view_face_mask`。
    face_mask: Option<Arc<pose::hide::FaceMask>>,
    /// このビューのポーズのプリセットの形（受けたままの形と同じ三角形の並びで、位置だけが違う。None なら受けたままの形）。
    pose_model: Option<Arc<ViewModel>>,
    /// このビューの見せる形（今のビューのものは、今のビューのあいだ `View3dState::model` へ出してある）。
    shape: Shape,
}

/// 1 つのビューの見せる形と、それを作った元（元・除いたマテリアル・除いた面が同じなら作り直さない。ほかのビューの同じ元の形と共有する）。
#[derive(Clone, Default)]
struct Shape {
    model: Option<Arc<ViewModel>>,
    source: Option<Arc<ViewModel>>,
    hidden: Vec<i32>,
    mask: Option<Arc<pose::hide::FaceMask>>,
    /// 見せる形の三角形の番号 → 受けたままの形の番号（None なら同じ番号）。
    to_full: Option<Arc<Vec<u32>>>,
    /// 隠した形の幾何（同じ隠し方でポーズだけが替わったとき、作り直さず box だけ当て直す元）。
    base: Option<Arc<ShownBase>>,
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
        if self.view.input.stroke.is_some() {
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
                // 三角形の並びが替わった: UV アイランド・除く面・ビューのポーズの形は前の形の番号なので捨てる
                for v in self.views_mut() {
                    v.hide.islands.clear();
                    v.hide.selected.clear();
                    v.hide.pointed = None;
                    v.face_mask = None;
                    v.pose_model = None;
                    v.pose = hide::ViewPose::Current;
                }
            }
        }
        let keep_camera = self
            .full
            .as_ref()
            .is_some_and(|m| m.name == model.name && m.triangle_count() == model.triangle_count());
        if !keep_camera {
            // 別のモデルになった: 全部のビューのカメラを、モデル全体が入る位置へ
            let bounds = model.geometry.bounds();
            for view in std::iter::once(&mut self.view).chain(self.parked.iter_mut()) {
                view.reframe(&bounds);
            }
        }
        self.full = Some(model);
        self.unpainted.clear();
        self.rebuild_shown();
    }

    /// 全部のビューの見せる形を作り直す（元・除くマテリアル・除く面が前と同じビューはそのまま。同じ元・同じ隠し方のビューは、先に作った
    /// 形を共有する）。
    fn rebuild_shown(&mut self) {
        // 今のビューの形も、ビューの欄へ戻してから作る
        self.view.shape.model = self.model.take();
        let Some(full) = self.full.clone() else {
            for v in self.views_mut() {
                v.shape = Shape::default();
            }
            return;
        };
        let total = full.triangle_count();
        let hidden = self.hidden.clone();
        let count = self.view_count();
        let mut done: Vec<Shape> = Vec::with_capacity(count);
        for i in 0..count {
            let (source, mask, mut old) = {
                let v = self.view_at_mut(i);
                let source = v
                    .pose_model
                    .clone()
                    .filter(|m| m.triangle_count() == total)
                    .unwrap_or_else(|| full.clone());
                let mask = v
                    .face_mask
                    .clone()
                    .filter(|m| m.len() == total && m.hidden_count() > 0);
                (source, mask, std::mem::take(&mut v.shape))
            };
            let made = |s: &Shape| {
                s.source.as_ref().is_some_and(|x| Arc::ptr_eq(x, &source))
                    && s.hidden == hidden
                    && same_mask_content(&s.mask, &mask)
            };
            let shape = if let Some(shared) = done.iter().find(|s| made(s)) {
                shared.clone()
            } else if made(&old) && old.model.is_some() {
                old
            } else {
                let base = old.base.take();
                self.build_shape(&full, source, &hidden, mask, base)
            };
            if !done.iter().any(|s| same_shape(s, &shape)) {
                done.push(shape.clone());
            }
            self.view_at_mut(i).shape = shape;
        }
        self.model = self.view.shape.model.take();
    }

    /// 番号 i のビュー（0 が今のビュー、続けて置いてあるビュー）。
    fn view_at_mut(&mut self, i: usize) -> &mut ViewState {
        if i == 0 {
            &mut self.view
        } else {
            &mut self.parked[i - 1]
        }
    }

    /// 1 つの見せる形を作る（隠すマテリアルも隠す面も無ければ元のまま。隠すと三角形が残らなければ見せる形は無し）。見せる三角形だけを
    /// 残したメッシュを組み、幾何は、同じ隠し方でポーズだけが替わったなら前の幾何の箱を当て直し、そうでなければ受けたままの形の隣り合わせを
    /// 写して作る（溶接し直さない。ブラシの大きさの基準も受けたままの形のもの）。
    fn build_shape(
        &mut self,
        full: &Arc<ViewModel>,
        source: Arc<ViewModel>,
        hidden: &[i32],
        mask: Option<Arc<pose::hide::FaceMask>>,
        base: Option<Arc<ShownBase>>,
    ) -> Shape {
        let total = full.triangle_count();
        let mut shape = Shape {
            model: None,
            source: Some(source.clone()),
            hidden: hidden.to_vec(),
            mask: mask.clone(),
            to_full: None,
            base: None,
        };
        let hides = |m: i32| hidden.contains(&m);
        if mask.is_none()
            && !source
                .meshes
                .iter()
                .any(|mesh| mesh.submeshes.iter().any(|s| hides(s.material)))
        {
            shape.model = Some(source);
            return shape;
        }
        let mut keep = vec![false; total];
        let mut map: Vec<u32> = Vec::new();
        let mut at_base = 0usize;
        let mut meshes = Vec::with_capacity(source.meshes.len());
        for mesh in &source.meshes {
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
                        let at = at_base + k;
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
                at_base += n;
            }
            meshes.push(out);
        }
        // 見せる面が 1 つも無い（全部のマテリアルを隠した）ときは、形を持たない（3D ビューの操作の途中も終わる）
        if !keep.iter().any(|k| *k) {
            return shape;
        }
        let base = base.filter(|b| {
            b.hidden.as_slice() == hidden && same_mask(&b.mask, &mask) && b.total == total
        });
        let revision = self.next_revision();
        let triangles = || model_triangles(&meshes);
        let reused = base.as_ref().and_then(|b| {
            b.geometry
                .reposition(triangles()?, revision, BvhUpdate::Refit)
                .ok()
        });
        let geometry = match reused {
            Some(g) => Some(g),
            None => triangles().and_then(|t| full.geometry.restrict(&keep, t, revision).ok()),
        };
        let Some(geometry) = geometry else {
            return shape;
        };
        let geometry = Arc::new(geometry);
        let mut shown = ViewModel::with_geometry(
            &source.name,
            meshes,
            source.materials.clone(),
            geometry.clone(),
        );
        shown.link_generation = full.link_generation;
        shown.demo = full.demo;
        shape.base = Some(Arc::new(ShownBase {
            hidden: hidden.to_vec(),
            mask,
            total,
            geometry,
        }));
        shape.to_full = Some(Arc::new(map));
        shape.model = Some(Arc::new(shown));
        shape
    }

    /// どれかのビューの見せる形が、今の元・除くマテリアル・除く面と違うか（描いている最中に変わった分を、終わってから作り直す）。
    fn shapes_stale(&self) -> bool {
        let Some(full) = &self.full else {
            return false;
        };
        let total = full.triangle_count();
        self.views().any(|v| {
            let source = v
                .pose_model
                .as_ref()
                .filter(|m| m.triangle_count() == total)
                .unwrap_or(full);
            let mask = v
                .face_mask
                .clone()
                .filter(|m| m.len() == total && m.hidden_count() > 0);
            !v.shape
                .source
                .as_ref()
                .is_some_and(|s| Arc::ptr_eq(s, source))
                || v.shape.hidden != self.hidden
                || !same_mask_content(&v.shape.mask, &mask)
        })
    }

    /// 隠すマテリアル（目を閉じたテクスチャセットのもの）を決める。変わったら見せる形を組み直す（描いている最中は終わってから）。
    pub fn set_hidden(&mut self, mut hidden: Vec<i32>) {
        hidden.sort_unstable();
        hidden.dedup();
        if hidden == self.hidden {
            return;
        }
        self.hidden = hidden;
        if self.view.input.stroke.is_none() && self.pending.is_none() {
            self.rebuild_shown();
        }
    }

    /// 今のビューの見せる形から除く面を決める（`set_view_face_mask` の今のビューのもの）。
    pub fn set_face_mask(&mut self, mask: Option<Arc<pose::hide::FaceMask>>) {
        let id = self.view.id;
        self.set_view_face_mask(id, mask);
    }

    /// このビューの見せる形から除く面を決める（三角形は受けたままの形の番号。変わったら見せる形を作り直す。描いている最中は終わってから。
    /// 三角形の数が今のモデルと違う印は使わない）。
    pub fn set_view_face_mask(&mut self, id: ViewId, mask: Option<Arc<pose::hide::FaceMask>>) {
        let Some(view) = self.view_of_mut(id) else {
            return;
        };
        if same_mask(&mask, &view.face_mask) {
            return;
        }
        view.face_mask = mask;
        if self.view.input.stroke.is_none() && self.pending.is_none() {
            self.rebuild_shown();
        }
    }

    /// このビューのポーズの形を決める（None なら受けたままの形。三角形の数が違う形は使わない。変わったら見せる形を作り直す。描いている
    /// 最中は終わってから）。
    pub fn set_view_pose_model(&mut self, id: ViewId, model: Option<Arc<ViewModel>>) {
        let Some(view) = self.view_of_mut(id) else {
            return;
        };
        let same = match (&model, &view.pose_model) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        if same {
            return;
        }
        view.pose_model = model;
        if self.view.input.stroke.is_none() && self.pending.is_none() {
            self.rebuild_shown();
        }
    }

    /// このビューのポーズの形（None なら受けたままの形を見せる）。
    pub fn view_pose_model(&self, id: ViewId) -> Option<&Arc<ViewModel>> {
        self.view_of(id)?.pose_model.as_ref()
    }

    /// 今のビューで決めている隠す面（見せる形へはまだ入っていないことがある）。
    pub fn face_mask(&self) -> Option<&Arc<pose::hide::FaceMask>> {
        self.view.face_mask.as_ref()
    }

    /// 受けたままの形のこの三角形を、今のビューの見せる形から除いている面として隠しているか（マテリアルを隠しているものは含めない）。
    pub fn is_face_hidden(&self, full_triangle: u32) -> bool {
        let total = self.full.as_ref().map_or(0, |m| m.triangle_count());
        self.view
            .shape
            .mask
            .as_ref()
            .is_some_and(|m| m.len() == total && m.is_hidden(full_triangle as usize))
    }

    /// このフレームに描く見せる形の全部（ビューの番号の順、重ならない）と、テクスチャの塗り広げとほかのセットの持ち方が読む形（全部のビューが
    /// 同じ形なら、その形。違えば受けたままの形。ビューごとに違う形を読むと、ビューを描くたびに作り直すので）。
    pub fn frame_models(&self) -> (Vec<Arc<ViewModel>>, Option<Arc<ViewModel>>) {
        let mut views: Vec<&ViewState> = self.views().collect();
        views.sort_by_key(|v| v.id);
        let mut out: Vec<Arc<ViewModel>> = Vec::new();
        for v in views {
            let m = if v.id == self.view.id {
                self.model.as_ref()
            } else {
                v.shape.model.as_ref()
            };
            if let Some(m) = m {
                if !out.iter().any(|o| Arc::ptr_eq(o, m)) {
                    out.push(m.clone());
                }
            }
        }
        let base = match out.as_slice() {
            [one] => Some(one.clone()),
            [] => None,
            _ => self.full.clone(),
        };
        (out, base)
    }

    /// 今のビューの見せる形の元（受けたままの形か、そのビューのポーズのプリセットの形。三角形の番号は受けたままの形と同じ）。
    pub fn view_source(&self) -> Option<&Arc<ViewModel>> {
        self.view.shape.source.as_ref()
    }

    /// このビューの見せる形（今のビューなら `model`）。
    pub fn view_model(&self, id: ViewId) -> Option<&Arc<ViewModel>> {
        if id == self.view.id {
            return self.model.as_ref();
        }
        self.view_of(id)?.shape.model.as_ref()
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
        if self.view.input.stroke.is_some() {
            self.pending = None;
            self.pending_close = true;
            return;
        }
        self.full = None;
        self.model = None;
        self.unpainted.clear();
        self.rebuild_shown();
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

    /// 今のビューのカメラをモデル全体が入る位置へ戻す。
    pub fn frame_model(&mut self) {
        if let Some(bounds) = self.full.as_ref().map(|m| m.geometry.bounds()) {
            self.view.reframe(&bounds);
        }
    }

    /// ストロークが終わったら、待たせていたモデル・閉じる・隠すを当てる。
    pub(crate) fn stroke_ended(&mut self) {
        self.view.input.stroke = None;
        self.view.input.released = false;
        self.view.input.dab_clock = pacing::DabClock::default();
        self.view.input.frame_dabs = 0;
        self.view.input.surface = None;
        self.view.input.cover = None;
        self.view.input.symmetry = None;
        self.view.input.last_point = None;
        self.view.input.last_hit = None;
        self.view.input.shift_hold = None;
        self.view.input.ruler_constraint = None;
        if std::mem::take(&mut self.pending_close) {
            self.full = None;
            self.model = None;
            self.unpainted.clear();
            self.rebuild_shown();
        } else if let Some(m) = self.pending.take() {
            self.apply_model(m);
        } else if self.shapes_stale() {
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
        let model = self.model.as_ref()?;
        match &self.view.shape.to_full {
            None => ((shown as usize) < model.triangle_count()).then_some(shown),
            Some(map) => map.get(shown as usize).copied(),
        }
    }

    /// このマテリアルの面を、今は見せる形から除いているか。
    pub fn is_material_hidden(&self, material: i32) -> bool {
        self.view.shape.hidden.contains(&material)
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
#[derive(Clone)]
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

/// 隠す面が同じか（中身で比べる。ビューどうしで同じ隠し方なら、別に作った印でも同じ形を共有する）。
fn same_mask_content(
    a: &Option<Arc<pose::hide::FaceMask>>,
    b: &Option<Arc<pose::hide::FaceMask>>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || **a == **b,
        _ => false,
    }
}

/// 同じ見せる形か（共有している）。
fn same_shape(a: &Shape, b: &Shape) -> bool {
    match (&a.model, &b.model) {
        (Some(x), Some(y)) => Arc::ptr_eq(x, y),
        _ => false,
    }
}
