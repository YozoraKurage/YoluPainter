//! 3D ビューのカメラ（Unity 版の IsolatedModelPreview のカメラと操作）: 注視点のまわりを回る（yaw・pitch は度、Unity の
//! `Quaternion.Euler(pitch, yaw, 0)`）、縦の画角 30°、注視点からの距離。左手系（Unity と同じ。前は +Z、上は +Y）。
//!
//! 画面の点からのレイ・画面への写し・世界の半径の画面での大きさ（ブラシのカーソルとストロークの間隔）、wgpu の描画の行列（深度 0〜1）
//! を同じカメラから出す。
//!
//! 投影は透視か正投影（[`Projection`]）。投影で変わる式（レイ・写し・半径・点から見る人への向き・見えるかを確かめる視線・奥行き）は
//! [`CameraView`] の関数にまとめ、呼び手は投影を見ずにそれを使う。透視の式は正投影を入れる前と同じ（同じ値・同じビット）。

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::unity::{Bounds, Ray};

/// 縦の画角（度。Unity 版と同じ）。
pub const FIELD_OF_VIEW: f32 = 30.0;

/// 既定の yaw（度）。カメラは `target - rotation * Z * distance` の位置から +Z 方向を向くので、yaw 0° は -Z 側（モデルの背中）から見る。
/// モデルは Unity の流儀で +Z を向くため、前（+Z 側）から見るには 180° 回す。斜めに振る 25° はそのまま足して、背中から見ていた
/// ときと同じ角度のまま向きだけを前へ移す。
pub const DEFAULT_YAW: f32 = 180.0 + 25.0;
/// 既定の pitch（度。わずかに見下ろす）。
pub const DEFAULT_PITCH: f32 = 10.0;

/// 回すときの、画面の点 1 つあたりの角度（度。Unity 版と同じ）。
pub const ORBIT_DEGREES_PER_POINT: f32 = 0.35;
/// 軸の向きへ吸い付く角度（度）。
pub const SNAP_ANGLE: f32 = 15.0;

/// 軸に沿った視点（正面・背面・右・左・上・下）。モデルは Unity の流儀で +Z を向き、+Y が上。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisView {
    /// 前（+Z 側）から見る。
    Front,
    /// 背中（-Z 側）から見る。
    Back,
    /// モデルの右（+X 側）から見る。
    Right,
    /// モデルの左（-X 側）から見る。
    Left,
    /// 真上（+Y 側）から見下ろす。
    Top,
    /// 真下（-Y 側）から見上げる。
    Bottom,
}

impl AxisView {
    pub const ALL: [AxisView; 6] = [
        Self::Front,
        Self::Back,
        Self::Right,
        Self::Left,
        Self::Top,
        Self::Bottom,
    ];

    /// この視点の (yaw, pitch)（度）。上・下は pitch ±90°（`orbit` は ±89° までなので、吸い付くときだけここへ来る）で、
    /// yaw は 180°（画面の上がモデルの背中側）。
    pub fn orientation(self) -> (f32, f32) {
        match self {
            Self::Front => (180.0, 0.0),
            Self::Back => (0.0, 0.0),
            Self::Right => (-90.0, 0.0),
            Self::Left => (90.0, 0.0),
            Self::Top => (180.0, 90.0),
            Self::Bottom => (180.0, -90.0),
        }
    }

    /// カメラの前の向き（見ている向き）。
    fn forward(self) -> Vec3 {
        let (yaw, pitch) = self.orientation();
        orientation_rotation(yaw, pitch) * Vec3::Z
    }

    /// この視点のカメラがいる側の軸の向き（世界の単位の向き。右は +X、上は +Y、正面は +Z）。
    pub fn side(self) -> Vec3 {
        match self {
            Self::Front => Vec3::Z,
            Self::Back => Vec3::NEG_Z,
            Self::Right => Vec3::X,
            Self::Left => Vec3::NEG_X,
            Self::Top => Vec3::Y,
            Self::Bottom => Vec3::NEG_Y,
        }
    }
}

/// 見ている向きが軸の視点とそろっているとみなす cos（約 0.26°。吸い付き・軸の視点の選びの後の向きはこの中に入る）。
const ALIGNED_COS: f32 = 0.99999;

/// 見ている向きが軸の視点にそろっていれば、その視点（吸い付いた・軸の視点を選んだ後の向き。回して外れたら None）。
pub fn aligned_axis_view(yaw: f32, pitch: f32) -> Option<AxisView> {
    let forward = orientation_rotation(yaw, pitch) * Vec3::Z;
    AxisView::ALL
        .into_iter()
        .find(|view| forward.dot(view.forward()) > ALIGNED_COS)
}

/// 投影の種類。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Projection {
    /// 透視（縦の画角 [`FIELD_OF_VIEW`]）。
    #[default]
    Perspective,
    /// 正投影。`height` は表示域の縦に見える世界の長さ（モデルの単位、0 より大きい）。
    Orthographic { height: f32 },
}

impl Projection {
    pub fn is_orthographic(self) -> bool {
        matches!(self, Self::Orthographic { .. })
    }
}

/// 透視で、カメラから距離 distance の所に縦に見える世界の長さ（2 × 距離 × tan(画角/2)）。透視と正投影を切り替えるとき、注視点の所の
/// 画面の大きさを変えない正投影の高さ。
pub fn visible_height(distance: f32) -> f32 {
    2.0 * distance * (FIELD_OF_VIEW * 0.5).to_radians().tan()
}

/// 正投影の拡縮の範囲（透視の距離の範囲 [半径 × 0.05, 半径 × 100] で見える高さ）。回す中心のまわりで切り替えた高さはこの外にも
/// なるので、今の高さが外なら外の側の端を今の高さにする（動かした向きにだけ制限をかける。範囲の外から 1 目盛り寄って、逆に引いた
/// 絵にならない）。
fn orthographic_zoom_range(model_radius: f32, height: f32) -> (f32, f32) {
    (
        visible_height(model_radius * 0.05).min(height),
        visible_height(model_radius * 100.0).max(height),
    )
}

/// 見る人（遮蔽のレイと面の向きの元）: 透視はカメラの位置の 1 点、正投影は前の向きに平行な視線。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Viewer {
    /// カメラの位置から放射状に見る。
    Point(Vec3),
    /// 前の向き `forward`（単位）に平行に見る。視線は前の向きの上の位置 `near`（世界の点 p の位置は p · forward）の面から始まる。
    Parallel { forward: Vec3, near: f32 },
}

impl From<Vec3> for Viewer {
    fn from(camera: Vec3) -> Viewer {
        Viewer::Point(camera)
    }
}

impl Viewer {
    pub fn is_finite(&self) -> bool {
        match *self {
            Viewer::Point(p) => p.is_finite(),
            Viewer::Parallel { forward, near } => forward.is_finite() && near.is_finite(),
        }
    }

    /// 世界の点から見る人への向き（透視はカメラの位置への差で、長さは距離。正投影は前の逆の単位の向き）。面の表裏・手前の半分の判定に使う。
    pub fn to_viewer(&self, world: Vec3) -> Vec3 {
        match *self {
            Viewer::Point(p) => p - world,
            Viewer::Parallel { forward, .. } => -forward,
        }
    }

    /// 見る人から世界の点までの視線（透視はカメラの位置から、正投影は点を通る視線と近い面の交わりから）。
    pub fn sight(&self, world: Vec3) -> Sight {
        match *self {
            Viewer::Point(p) => {
                let to = world - p;
                Sight {
                    origin: p,
                    to,
                    distance: to.length(),
                }
            }
            Viewer::Parallel { forward, near } => {
                let distance = world.dot(forward) - near;
                Sight {
                    origin: world - forward * distance,
                    to: forward * distance,
                    distance,
                }
            }
        }
    }
}

/// 見る人から世界の点までの視線（[`CameraView::sight`]）。点が見る人の所・近い面より手前なら、`distance` は 0 以下。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sight {
    /// 始め。
    pub origin: Vec3,
    /// 始めから点への向き（長さは `distance`）。
    pub to: Vec3,
    pub distance: f32,
}

impl Sight {
    /// 始めから点へ向かうレイ。
    pub fn ray(&self) -> Ray {
        Ray::new(self.origin, self.to)
    }
}

fn orientation_rotation(yaw: f32, pitch: f32) -> Quat {
    Quat::from_euler(
        glam::EulerRot::YXZ,
        yaw.to_radians(),
        pitch.to_radians(),
        0.0,
    )
}

/// 回したあとの (yaw, pitch)（`orbit` と同じ動き。pitch は ±89° まで。今の pitch がそれを越えている（軸の視点へ吸い付いて ±90°）ときは、
/// その値まで止めを広げる: 真上・真下から回し始めても、最初に 1° はねない）。
pub fn orbited(yaw: f32, pitch: f32, dx_points: f32, dy_points: f32) -> (f32, f32) {
    let limit = pitch.abs().clamp(89.0, 90.0);
    (
        yaw + dx_points * ORBIT_DEGREES_PER_POINT,
        (pitch + dy_points * ORBIT_DEGREES_PER_POINT).clamp(-limit, limit),
    )
}

/// この向きに `SNAP_ANGLE` 以内で近い軸の視点（見ている向きどうしの角度。いちばん近いもの。無ければ None）。
pub fn nearest_axis_view(yaw: f32, pitch: f32) -> Option<AxisView> {
    let forward = orientation_rotation(yaw, pitch) * Vec3::Z;
    AxisView::ALL
        .into_iter()
        .map(|view| (view, forward.dot(view.forward()).clamp(-1.0, 1.0).acos()))
        .filter(|(_, angle)| angle.to_degrees() <= SNAP_ANGLE)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(view, _)| view)
}

/// 向きを、`SNAP_ANGLE` 以内にある軸の視点へ吸い付ける（無ければそのまま）。yaw は回した数（360° の何周か）を保つ。
/// 上・下は画面の向きが決まらないので、今の yaw にいちばん近い 90° の倍数にする。
pub fn snap_orientation(yaw: f32, pitch: f32) -> (f32, f32) {
    let Some(view) = nearest_axis_view(yaw, pitch) else {
        return (yaw, pitch);
    };
    let (axis_yaw, axis_pitch) = view.orientation();
    let target_yaw = match view {
        AxisView::Top | AxisView::Bottom => (yaw / 90.0).round() * 90.0,
        _ => axis_yaw + 360.0 * ((yaw - axis_yaw) / 360.0).round(),
    };
    (target_yaw, axis_pitch)
}

/// 注視点のまわりを回るカメラ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    pub target: Vec3,
    /// 度。
    pub yaw: f32,
    /// 度（`orbit` は −89〜89。軸の視点へ吸い付いたときだけ ±90）。
    pub pitch: f32,
    pub distance: f32,
    /// モデルの半径（寄る範囲・近い面と遠い面の目安）。
    pub model_radius: f32,
    /// 投影。正投影でも `distance` はカメラの位置（光・陰の視線）と近い面・遠い面に使い、拡縮は見える高さだけを変える。
    pub projection: Projection,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        OrbitCamera {
            target: Vec3::ZERO,
            yaw: DEFAULT_YAW,
            pitch: DEFAULT_PITCH,
            distance: 3.0,
            model_radius: 1.0,
            projection: Projection::Perspective,
        }
    }
}

impl OrbitCamera {
    /// モデル全体が入る位置（Unity 版の FrameModel と同じ斜めの角度（yaw 25°・pitch 10°）で、モデルの前（+Z 側）から中心を見る。
    /// 距離は半径 / sin 15° × 1.15）。開いた直後・モデルを替えたとき・「全体を表示」が同じ向きになる。
    pub fn framing(bounds: &Bounds) -> OrbitCamera {
        let radius = super::unity::fmax(0.0001, super::unity::magnitude(bounds.extents));
        OrbitCamera {
            target: bounds.center,
            yaw: DEFAULT_YAW,
            pitch: DEFAULT_PITCH,
            distance: radius / (15.0f32).to_radians().sin() * 1.15,
            model_radius: radius,
            projection: Projection::Perspective,
        }
    }

    pub fn is_orthographic(&self) -> bool {
        self.projection.is_orthographic()
    }

    /// 注視点の所に縦に見える世界の長さ（正投影は見える高さ、透視は距離から）。
    pub fn height_at_target(&self) -> f32 {
        match self.projection {
            Projection::Perspective => visible_height(self.distance),
            Projection::Orthographic { height } => height,
        }
    }

    /// 透視と正投影を切り替える（注視点の所の画面の大きさは変えない: 正投影の見える高さは [`visible_height`] の距離から。透視へ戻すときは、
    /// その高さに見える距離へ（寄る・引く範囲に収める））。同じ投影なら何もしない。
    pub fn set_orthographic(&mut self, on: bool) {
        self.set_orthographic_about(on, self.target);
    }

    /// 透視と正投影を切り替え、点 pivot の画面の位置と大きさを変えない（回す中心の点。正投影の見える高さは、pivot の奥行きの透視で
    /// 見える高さ。透視へ戻すときは、その高さに見える奥行きまでカメラを前後に動かす（注視点はそのまま。距離を寄る・引く範囲に収めた
    /// ときは大きさが少し変わる））。pivot がカメラの前に無ければ注視点で。同じ投影なら何もしない。
    pub fn set_orthographic_about(&mut self, on: bool, pivot: Vec3) {
        // pivot の奥行きと注視点の奥行きの差（カメラの前の向きに測る）
        let along = (pivot - self.target).dot(self.rotation() * Vec3::Z);
        let along = if along.is_finite() && self.distance + along > self.distance * 0.01 {
            along
        } else {
            0.0
        };
        match (on, self.projection) {
            (true, Projection::Perspective) => {
                self.projection = Projection::Orthographic {
                    height: visible_height(self.distance + along),
                };
            }
            (false, Projection::Orthographic { height }) => {
                let r = self.model_radius;
                let distance = height / visible_height(1.0) - along;
                if distance.is_finite() {
                    self.distance = distance.clamp(r * 0.05, r * 100.0);
                }
                self.projection = Projection::Perspective;
            }
            _ => {}
        }
    }

    /// 向き（`Quaternion.Euler(pitch, yaw, 0)`: Z、X、Y の順に回す）。
    pub fn rotation(&self) -> Quat {
        orientation_rotation(self.yaw, self.pitch)
    }

    pub fn position(&self) -> Vec3 {
        self.target - self.rotation() * Vec3::Z * self.distance
    }

    /// 回す（Unity 版: 画面の点 1 つで 0.35°。下へドラッグすると上から見下ろす）。
    pub fn orbit(&mut self, dx_points: f32, dy_points: f32) {
        (self.yaw, self.pitch) = orbited(self.yaw, self.pitch, dx_points, dy_points);
    }

    /// 向きを (yaw, pitch) にする（pitch は ±90° まで）。`pivot` の画面上の位置は変えない（`orbit_about` と同じ置き方）。
    pub fn set_orientation_about(&mut self, pivot: Vec3, yaw: f32, pitch: f32) {
        let before = self.rotation();
        self.yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
        self.target = pivot + (self.rotation() * before.inverse()) * (self.target - pivot);
    }

    /// 注視点とカメラを一緒に動かす（距離は変えない）。`local` はカメラの (右, 上, 前) の向きの移動量（モデルの単位）。正投影では前後に
    /// 動いても絵の大きさが変わらず、近い面だけがモデルへ入って急に切れるので、前後の分は寄る・引く（注視点の所の物が透視でその分
    /// 近づいた・離れたのと同じ倍率で見える高さを変える）にし、カメラは動かさない。
    pub fn fly(&mut self, local: Vec3) {
        if let Projection::Orthographic { height } = &mut self.projection {
            let (lo, hi) = orthographic_zoom_range(self.model_radius, *height);
            *height = (*height * (-local.z / self.distance).exp()).clamp(lo, hi);
            self.target += self.rotation() * Vec3::new(local.x, local.y, 0.0);
            return;
        }
        self.target += self.rotation() * local;
    }

    /// パン（ポインタの下の注視点の面が指についてくる。view_height は表示域の高さの点）。正投影は見える高さから。
    pub fn pan(&mut self, dx_points: f32, dy_points: f32, view_height: f32) {
        let units = match self.projection {
            Projection::Perspective => {
                2.0 * self.distance * (FIELD_OF_VIEW * 0.5).to_radians().tan()
                    / view_height.max(1.0)
            }
            Projection::Orthographic { height } => height / view_height.max(1.0),
        };
        self.target += self.rotation() * Vec3::new(-dx_points * units, dy_points * units, 0.0);
    }

    /// 寄る・引く（notches はホイールの目盛り、正で寄る。1 目盛りで約 1.2 倍。Unity 版の exp(0.06 × 3)）。正投影は見える高さを変え
    /// （範囲は透視の距離の範囲で見える高さ）、カメラの位置は動かさない。
    pub fn zoom(&mut self, notches: f32) {
        let r = self.model_radius;
        match &mut self.projection {
            Projection::Perspective => {
                self.distance =
                    (self.distance * (-notches * 0.18).exp()).clamp(r * 0.05, r * 100.0);
            }
            Projection::Orthographic { height } => {
                let (lo, hi) = orthographic_zoom_range(r, *height);
                *height = (*height * (-notches * 0.18).exp()).clamp(lo, hi);
            }
        }
    }

    /// 任意の点のまわりを回す。その点の画面上の位置は変えない。
    pub fn orbit_about(&mut self, pivot: Vec3, dx_points: f32, dy_points: f32) {
        let before = self.rotation();
        self.orbit(dx_points, dy_points);
        self.target = pivot + (self.rotation() * before.inverse()) * (self.target - pivot);
    }

    /// 押した点の奥行きに合わせてパンする（奥行きは視線方向の距離）。正投影は奥行きによらない（[`OrbitCamera::pan`] と同じ）。
    pub fn pan_at_depth(&mut self, dx_points: f32, dy_points: f32, view_height: f32, depth: f32) {
        if self.is_orthographic() {
            self.pan(dx_points, dy_points, view_height);
            return;
        }
        let units = 2.0 * depth * (FIELD_OF_VIEW * 0.5).to_radians().tan() / view_height.max(1.0);
        self.target += self.rotation() * Vec3::new(-dx_points * units, dy_points * units, 0.0);
    }

    /// 点へ寄る。その点と同じレイの画面位置を保ち、従来の距離制限を使う。正投影は見える高さを変え、注視点を画面の上で点の方へ
    /// 寄せる（奥行きは変えない: カメラの位置と近い面・遠い面を動かさない）。
    pub fn zoom_towards(&mut self, point: Vec3, notches: f32) {
        if let Projection::Orthographic { height } = self.projection {
            self.zoom(notches);
            let ratio = self.height_at_target() / height;
            let forward = self.rotation() * Vec3::Z;
            let d = (point - self.target) * (1.0 - ratio);
            self.target += d - forward * d.dot(forward);
            return;
        }
        let distance = self.distance;
        self.zoom(notches);
        let ratio = self.distance / distance;
        self.target += (point - self.target) * (1.0 - ratio);
    }

    /// 今の向きを保ち、縦横の狭い方にも境界球が入る距離へ移る。モデル全体の半径は保つ。
    pub fn frame_bounds(&mut self, bounds: &Bounds, width: f32, height: f32) {
        let tan =
            (FIELD_OF_VIEW * 0.5).to_radians().tan() * (width.max(1.0) / height.max(1.0)).min(1.0);
        let radius = bounds.extents.length().max(0.0001);
        self.target = bounds.center;
        self.distance = (radius / tan.atan().sin() * 1.15).max(self.model_radius * 0.05);
        // 正投影は、その距離の透視と同じ大きさに見える高さ（境界球は透視より余裕をもって入る）
        if let Projection::Orthographic { height } = &mut self.projection {
            *height = visible_height(self.distance);
        }
    }

    /// この大きさ（物理の画素）の表示域で見たときのカメラ。
    pub fn view(&self, width: f32, height: f32) -> CameraView {
        let rotation = self.rotation();
        let near = 0.00001f32.max((self.distance * 0.02).min(self.model_radius * 0.01));
        let (near, projection) = match self.projection {
            Projection::Perspective => (near, Projection::Perspective),
            Projection::Orthographic { height } => {
                // 平行な視線は、カメラの位置の後ろの物も写せる。透視で寄ったまま切り替えてカメラがモデルの中にいても、注視点のまわり
                // （モデルの半径の 2 倍まで）を近い面で切らないよう、近い面をカメラの後ろへ下げる
                let height = if height.is_finite() && height > 0.0 {
                    height
                } else {
                    visible_height(self.distance)
                };
                (
                    near.min(self.distance - self.model_radius * 2.0),
                    Projection::Orthographic { height },
                )
            }
        };
        let far = (self.distance + self.model_radius * 20.0).max(near + 1.0);
        CameraView {
            position: self.position(),
            rotation,
            forward: rotation * Vec3::Z,
            near,
            far,
            width: width.max(1.0),
            height: height.max(1.0),
            projection,
        }
    }
}

/// ある大きさの表示域で見たカメラ（ストロークの間は固定して使う）。画面の座標は表示域の左上が原点、下が +y。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraView {
    pub position: Vec3,
    pub rotation: Quat,
    pub forward: Vec3,
    pub near: f32,
    pub far: f32,
    pub width: f32,
    pub height: f32,
    /// 投影（正投影なら見える高さ）。正投影の `near` は負（カメラの位置の後ろ）にもなる。
    pub projection: Projection,
}

impl CameraView {
    fn tan_half(&self) -> f32 {
        (FIELD_OF_VIEW * 0.5).to_radians().tan()
    }

    pub fn aspect(&self) -> f32 {
        self.width / self.height
    }

    pub fn is_orthographic(&self) -> bool {
        self.projection.is_orthographic()
    }

    /// 正投影の見える高さ（世界）。透視なら None。
    pub fn orthographic_height(&self) -> Option<f32> {
        match self.projection {
            Projection::Perspective => None,
            Projection::Orthographic { height } => Some(height),
        }
    }

    /// 正投影の、横と縦の見える長さの半分（世界）。透視なら None。
    fn half_extent(&self) -> Option<Vec2> {
        match self.projection {
            Projection::Perspective => None,
            Projection::Orthographic { height } => {
                Some(Vec2::new(height * 0.5 * self.aspect(), height * 0.5))
            }
        }
    }

    /// 世界 → 切り取りの行列（wgpu 向け。左手系、深度 0〜1）。
    pub fn view_projection(&self) -> Mat4 {
        let up = self.rotation * Vec3::Y;
        let view = glam::camera::lh::view::look_to_mat4(self.position, self.forward, up);
        // wgpu の切り取りの空間は D3D と同じ（y が上、深度 0〜1）
        let proj = match self.half_extent() {
            None => glam::camera::lh::proj::directx::perspective(
                FIELD_OF_VIEW.to_radians(),
                self.aspect(),
                self.near,
                self.far,
            ),
            Some(h) => glam::camera::lh::proj::directx::orthographic(
                -h.x, h.x, -h.y, h.y, self.near, self.far,
            ),
        };
        proj * view
    }

    /// 画面の点を通るレイ（始点は近い面の上。Unity の ViewportPointToRay と同じ考え方）。正投影は前の向きに平行。
    pub fn ray(&self, screen: Vec2) -> Ray {
        let vx = screen.x / self.width * 2.0 - 1.0;
        let vy = 1.0 - screen.y / self.height * 2.0;
        if let Some(h) = self.half_extent() {
            let origin = self.position + self.rotation * Vec3::new(vx * h.x, vy * h.y, self.near);
            return Ray::new(origin, self.forward);
        }
        let t = self.tan_half();
        let local = Vec3::new(vx * t * self.aspect(), vy * t, 1.0) * self.near;
        let origin = self.position + self.rotation * local;
        Ray::new(origin, origin - self.position)
    }

    /// 世界の点の奥行き（カメラの位置から前の向きに測った長さ）。
    pub fn depth(&self, world: Vec3) -> f32 {
        (world - self.position).dot(self.forward)
    }

    /// 見る人（遮蔽のレイと面の向きの元。透視はカメラの位置、正投影は近い面から前の向きに平行な視線）。
    pub fn viewer(&self) -> Viewer {
        match self.projection {
            Projection::Perspective => Viewer::Point(self.position),
            Projection::Orthographic { .. } => Viewer::Parallel {
                forward: self.forward,
                near: self.position.dot(self.forward) + self.near,
            },
        }
    }

    /// 世界の点から見る人への向き（透視はカメラの位置への差で、長さは距離。正投影は前の逆の単位の向き）。
    pub fn to_viewer(&self, world: Vec3) -> Vec3 {
        match self.projection {
            Projection::Perspective => self.position - world,
            Projection::Orthographic { .. } => -self.forward,
        }
    }

    /// 見る人から世界の点までの視線（遮蔽を確かめるレイ。透視はカメラの位置から、正投影は点を通る視線と近い面の交わりから）。
    pub fn sight(&self, world: Vec3) -> Sight {
        match self.projection {
            Projection::Perspective => Viewer::Point(self.position).sight(world),
            Projection::Orthographic { .. } => {
                let distance = self.depth(world) - self.near;
                Sight {
                    origin: world - self.forward * distance,
                    to: self.forward * distance,
                    distance,
                }
            }
        }
    }

    /// 画面の点を通る視線の上で、奥行き depth の点（面に当たらない所を、注視点と同じ奥行きの面へ落とす）。
    pub fn point_at_depth(&self, screen: Vec2, depth: f32) -> Vec3 {
        let ray = self.ray(screen);
        match self.projection {
            Projection::Perspective => {
                self.position + ray.direction() * (depth / ray.direction().dot(self.forward))
            }
            Projection::Orthographic { .. } => ray.origin() + self.forward * (depth - self.near),
        }
    }

    /// 世界の点が画面のどこに見えるか（透視でカメラの後ろなら None。正投影は近い面より手前なら None: 描かれずに切れる所に印を
    /// 出さない）。
    pub fn to_screen(&self, world: Vec3) -> Option<Vec2> {
        if self.is_orthographic() && self.depth(world) < self.near {
            return None;
        }
        let clip = self.view_projection() * Vec4::new(world.x, world.y, world.z, 1.0);
        if clip.w <= 1e-12 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Vec2::new(
            (ndc.x + 1.0) * 0.5 * self.width,
            (1.0 - ndc.y) * 0.5 * self.height,
        ))
    }

    /// 世界の点の所での半径（モデルの単位）が画面で何画素か（Unity 版の WorldRadiusToGuiPoints。近い面より手前なら 0）。正投影は
    /// 奥行きによらない。
    pub fn world_radius_to_screen(&self, world: Vec3, radius: f32) -> f32 {
        if radius <= 0.0 {
            return 0.0;
        }
        let depth = self.depth(world);
        if depth <= self.near {
            return 0.0;
        }
        match self.projection {
            Projection::Perspective => radius * self.height / (2.0 * depth * self.tan_half()),
            Projection::Orthographic { height } => radius * self.height / height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_matches_unity_euler_order() {
        // Unity: Quaternion.Euler(0, 90, 0) * forward = right、Euler(90, 0, 0) * forward = down
        let mut c = OrbitCamera {
            yaw: 90.0,
            pitch: 0.0,
            ..OrbitCamera::default()
        };
        assert!((c.rotation() * Vec3::Z - Vec3::X).length() < 1e-6);
        c.yaw = 0.0;
        c.pitch = 90.0;
        assert!((c.rotation() * Vec3::Z - Vec3::NEG_Y).length() < 1e-6);
    }

    #[test]
    fn center_ray_hits_target_and_projects_back() {
        let c = OrbitCamera {
            target: Vec3::new(1.0, 2.0, 3.0),
            ..OrbitCamera::default()
        };
        let v = c.view(400.0, 300.0);
        let r = v.ray(Vec2::new(200.0, 150.0));
        let to_target = (c.target - r.origin()).normalize();
        // 近い面の距離（0.01）に対して座標（約 3）の f32 の丸め（約 2.4e-7）が効くので、向きは 1e-4 まで合えばよい
        // （向きによって 1e-5 前後で出入りするため、どの既定の向きでも通る幅にしておく）
        let error = (to_target - r.direction()).length();
        assert!(error < 1e-4, "{error}");
        let p = v.to_screen(c.target).unwrap();
        assert!((p - Vec2::new(200.0, 150.0)).length() < 1e-3);
        // 画面の右の点は右へ、上の点は上へ
        let right = v
            .to_screen(c.target + c.rotation() * Vec3::X * 0.1)
            .unwrap();
        let up = v
            .to_screen(c.target + c.rotation() * Vec3::Y * 0.1)
            .unwrap();
        assert!(right.x > 200.0 && (right.y - 150.0).abs() < 1e-3);
        assert!(up.y < 150.0 && (up.x - 200.0).abs() < 1e-3);
        // 画面のどの点のレイも、写すと同じ点へ戻る
        for s in [Vec2::new(10.0, 20.0), Vec2::new(390.0, 290.0)] {
            let r = v.ray(s);
            let back = v.to_screen(r.point(2.0)).unwrap();
            // f32 の丸めで向きによって 0.01 画素前後ぶれる。画素の 1/20 に収まれば戻っている
            assert!((back - s).length() < 5e-2, "{s} → {back}");
        }
    }

    #[test]
    fn radius_on_screen_matches_projection() {
        let c = OrbitCamera::default();
        let v = c.view(500.0, 500.0);
        let px = v.world_radius_to_screen(c.target, 0.1);
        let edge = v
            .to_screen(c.target + c.rotation() * Vec3::X * 0.1)
            .unwrap();
        assert!((px - (edge.x - 250.0)).abs() < 0.05, "{px} {edge}");
    }

    #[test]
    fn orbit_about_keeps_an_off_center_point_fixed_even_at_pitch_limit() {
        let mut c = OrbitCamera::default();
        let pivot = c.target + c.rotation() * Vec3::new(0.3, -0.2, -0.5);
        let screen = c.view(640.0, 480.0).to_screen(pivot).unwrap();
        let distance = c.position().distance(pivot);
        for (dx, dy) in [(30.0, 10.0), (-20.0, 400.0), (10.0, -800.0)] {
            c.orbit_about(pivot, dx, dy);
            assert!((c.view(640.0, 480.0).to_screen(pivot).unwrap() - screen).length() < 0.002);
            assert!((c.position().distance(pivot) - distance).abs() < 0.00001);
        }
    }

    #[test]
    fn depth_pan_tracks_the_pointer_in_screen_points() {
        let mut c = OrbitCamera::default();
        let p = c.target + c.rotation() * Vec3::new(0.2, 0.1, -1.0);
        let before = c.view(640.0, 480.0).to_screen(p).unwrap();
        let depth = (p - c.position()).dot(c.rotation() * Vec3::Z);
        c.pan_at_depth(43.0, -21.0, 480.0, depth);
        let after = c.view(640.0, 480.0).to_screen(p).unwrap();
        assert!((after - before - Vec2::new(43.0, -21.0)).length() < 0.002);
    }

    #[test]
    fn zoom_towards_keeps_the_point_fixed_including_distance_limits() {
        let mut c = OrbitCamera::default();
        let p = c.target + c.rotation() * Vec3::new(0.4, -0.2, -0.5);
        let before = c.view(640.0, 480.0).to_screen(p).unwrap();
        for notches in [1.0, -2.0, 100.0, 1.0, -100.0, -1.0] {
            c.zoom_towards(p, notches);
            assert!((c.view(640.0, 480.0).to_screen(p).unwrap() - before).length() < 0.03);
            assert!((c.model_radius * 0.05..=c.model_radius * 100.0).contains(&c.distance));
        }
    }

    #[test]
    fn frame_bounds_fits_both_wide_and_narrow_views_without_changing_orientation() {
        let b = Bounds::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(2.0, 1.0, 3.0));
        for (w, h) in [(800.0, 300.0), (120.0, 600.0)] {
            let mut c = OrbitCamera::default();
            let original = c;
            c.frame_bounds(&b, w, h);
            assert_eq!(
                (c.yaw, c.pitch, c.model_radius),
                (original.yaw, original.pitch, original.model_radius)
            );
            for x in [-1.0, 1.0] {
                for y in [-1.0, 1.0] {
                    for z in [-1.0, 1.0] {
                        let p = c
                            .view(w, h)
                            .to_screen(b.center + b.extents * Vec3::new(x, y, z))
                            .unwrap();
                        assert!(p.x > 0.0 && p.x < w && p.y > 0.0 && p.y < h, "{p}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_default_view_looks_at_the_model_from_the_front() {
        // モデルは +Z を向く（Unity の約束）。開いた直後・モデルを替えたとき・全体を表示する位置は同じ向きで、前（+Z 側）にある
        let b = Bounds::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(2.0, 4.0, 1.0));
        let framed = OrbitCamera::framing(&b);
        let default = OrbitCamera::default();
        assert_eq!((framed.yaw, framed.pitch), (DEFAULT_YAW, DEFAULT_PITCH));
        assert_eq!((default.yaw, default.pitch), (framed.yaw, framed.pitch));
        let offset = framed.position() - framed.target;
        assert!(offset.z > 0.0, "カメラはモデルの前（+Z 側）: {offset}");
        // 前から見ていても、背中から見ていたときと同じ 25°の斜め・10°の見下ろし
        let azimuth = offset.x.atan2(offset.z).to_degrees();
        assert!((azimuth - 25.0).abs() < 1e-3, "{azimuth}");
        let elevation = (offset.y / offset.length()).asin().to_degrees();
        assert!((elevation - 10.0).abs() < 1e-3, "{elevation}");
        // 前を向く面（+Z の法線）がこちらを向き、モデルの前の面の中心が後ろの面の中心より近い
        let view = framed.view(400.0, 300.0);
        assert!(view.forward.dot(Vec3::Z) < -0.85, "{}", view.forward);
        let front = b.center + Vec3::Z * b.extents.z;
        let back = b.center - Vec3::Z * b.extents.z;
        assert!(front.distance(framed.position()) < back.distance(framed.position()));
        // 注視点は画面の中央、全体が入る
        assert!((view.to_screen(b.center).unwrap() - Vec2::new(200.0, 150.0)).length() < 1e-2);
        for x in [-1.0, 1.0] {
            for y in [-1.0, 1.0] {
                for z in [-1.0, 1.0] {
                    let p = view
                        .to_screen(b.center + b.extents * Vec3::new(x, y, z))
                        .unwrap();
                    assert!(p.x > 0.0 && p.x < 400.0 && p.y > 0.0 && p.y < 300.0, "{p}");
                }
            }
        }
    }

    #[test]
    fn the_axis_views_look_at_the_target_from_the_side_they_are_named_for() {
        for (view, side) in [
            (AxisView::Front, Vec3::Z),
            (AxisView::Back, Vec3::NEG_Z),
            (AxisView::Right, Vec3::X),
            (AxisView::Left, Vec3::NEG_X),
            (AxisView::Top, Vec3::Y),
            (AxisView::Bottom, Vec3::NEG_Y),
        ] {
            let (yaw, pitch) = view.orientation();
            let c = OrbitCamera {
                yaw,
                pitch,
                ..OrbitCamera::default()
            };
            let offset = (c.position() - c.target).normalize();
            assert!((offset - side).length() < 1e-5, "{view:?}: {offset}");
        }
    }

    #[test]
    fn the_orientation_snaps_to_an_axis_view_only_within_the_snap_angle() {
        // 正面（yaw 180・pitch 0）の 14° 手前は吸い付き、16° 手前は吸い付かない
        let near = snap_orientation(180.0 + 14.0, 0.0);
        assert_eq!(near, (180.0, 0.0));
        assert_eq!(snap_orientation(180.0 - 14.0, 0.0), (180.0, 0.0));
        assert_eq!(snap_orientation(180.0 + 16.0, 0.0), (196.0, 0.0));
        // 縦の向きも見る
        assert_eq!(snap_orientation(180.0, 14.0), (180.0, 0.0));
        assert_eq!(snap_orientation(180.0, 16.0), (180.0, 16.0));
        // 背面・右・左
        assert_eq!(snap_orientation(5.0, -3.0), (0.0, 0.0));
        assert_eq!(snap_orientation(-80.0, 4.0), (-90.0, 0.0));
        assert_eq!(snap_orientation(100.0, 4.0), (90.0, 0.0));
        // 斜め 45° はどの軸にも吸い付かない
        assert_eq!(snap_orientation(45.0, 0.0), (45.0, 0.0));
        assert_eq!(snap_orientation(225.0, 35.0), (225.0, 35.0));
        // 角度は、yaw と pitch が同時に振れた分で測る（yaw 10°・pitch 10° は 約 14° で内、yaw 11°・pitch 11° は 約 15.4° で外）
        assert_eq!(snap_orientation(180.0 + 10.0, 10.0), (180.0, 0.0));
        assert_eq!(snap_orientation(180.0 + 11.0, 11.0), (191.0, 11.0));
    }

    #[test]
    fn snapping_keeps_the_number_of_full_turns_of_yaw() {
        // 回した数（360° の何周か）は保つ。-180° も 540° も正面
        assert_eq!(snap_orientation(540.0 + 10.0, 0.0), (540.0, 0.0));
        assert_eq!(snap_orientation(-180.0 + 10.0, 0.0), (-180.0, 0.0));
        assert_eq!(snap_orientation(360.0 + 5.0, 0.0), (360.0, 0.0));
        assert_eq!(snap_orientation(-270.0 - 5.0, 0.0), (-270.0, 0.0));
    }

    #[test]
    fn looking_nearly_straight_up_or_down_snaps_to_pitch_90_with_a_quarter_turn_yaw() {
        // orbit の上限（89°）まで来れば、上へ吸い付く
        assert_eq!(snap_orientation(180.0, 89.0), (180.0, 90.0));
        assert_eq!(snap_orientation(180.0, 80.0), (180.0, 90.0));
        assert_eq!(snap_orientation(180.0, 74.0), (180.0, 74.0));
        assert_eq!(snap_orientation(180.0, -80.0), (180.0, -90.0));
        // 画面の向きは、今の yaw にいちばん近い 90° の倍数
        assert_eq!(snap_orientation(200.0, 85.0), (180.0, 90.0));
        assert_eq!(snap_orientation(230.0, 85.0), (270.0, 90.0));
        assert_eq!(snap_orientation(-50.0, -85.0), (-90.0, -90.0));
    }

    #[test]
    fn the_nearest_axis_view_wins_when_two_are_in_range() {
        // 正面（180・0）と上（pitch 90）の間。pitch 50° は上のほうが近い（40°）が、15° 以内の物が無ければ None
        assert_eq!(nearest_axis_view(180.0, 50.0), None);
        assert_eq!(nearest_axis_view(180.0, 8.0), Some(AxisView::Front));
        assert_eq!(nearest_axis_view(180.0, 82.0), Some(AxisView::Top));
        assert_eq!(nearest_axis_view(90.0 + 5.0, 0.0), Some(AxisView::Left));
        assert_eq!(nearest_axis_view(-90.0 + 5.0, 0.0), Some(AxisView::Right));
    }

    #[test]
    fn rays_projection_pan_and_pivoting_still_work_at_pitch_90_and_minus_90() {
        for view in [AxisView::Top, AxisView::Bottom] {
            let (yaw, pitch) = view.orientation();
            let mut c = OrbitCamera {
                target: Vec3::new(1.0, 2.0, 3.0),
                yaw,
                pitch,
                ..OrbitCamera::default()
            };
            let v = c.view(400.0, 300.0);
            assert!(v
                .view_projection()
                .to_cols_array()
                .iter()
                .all(|x| x.is_finite()));
            // 見ている向きは真下（上から）・真上（下から）
            let down = if pitch > 0.0 { Vec3::NEG_Y } else { Vec3::Y };
            assert!(
                (v.forward - down).length() < 1e-6,
                "{view:?}: {}",
                v.forward
            );
            // 画面の中心のレイは注視点へ向かう
            let r = v.ray(Vec2::new(200.0, 150.0));
            assert!(((c.target - r.origin()).normalize() - r.direction()).length() < 1e-3);
            let p = v.to_screen(c.target).unwrap();
            assert!((p - Vec2::new(200.0, 150.0)).length() < 1e-2);
            // 画面のどの点のレイも、写すと同じ点へ戻る
            for s in [
                Vec2::new(10.0, 20.0),
                Vec2::new(390.0, 290.0),
                Vec2::new(200.0, 40.0),
            ] {
                let back = v.to_screen(v.ray(s).point(2.0)).unwrap();
                assert!((back - s).length() < 5e-2, "{view:?}: {s} → {back}");
            }
            // 画面の右・上へ向く物は、画面の右・上に写る
            let right = v
                .to_screen(c.target + c.rotation() * Vec3::X * 0.1)
                .unwrap();
            let up = v
                .to_screen(c.target + c.rotation() * Vec3::Y * 0.1)
                .unwrap();
            assert!(right.x > 200.0 && (right.y - 150.0).abs() < 1e-2);
            assert!(up.y < 150.0 && (up.x - 200.0).abs() < 1e-2);
            // ブラシの大きさの式は、向きによらず同じ距離で同じ値
            let near = OrbitCamera::default().view(400.0, 300.0);
            let at_default = near.world_radius_to_screen(OrbitCamera::default().target, 0.1);
            let at_axis = OrbitCamera {
                yaw,
                pitch,
                ..OrbitCamera::default()
            }
            .view(400.0, 300.0)
            .world_radius_to_screen(Vec3::ZERO, 0.1);
            assert!(
                (at_default - at_axis).abs() < 1e-3,
                "{at_default} {at_axis}"
            );
            // パン: 右へドラッグすると注視点は画面の左へ
            let before = c.target;
            c.pan(10.0, 0.0, 300.0);
            assert!((c.target - before).dot(c.rotation() * Vec3::X) < 0.0);
            // 注視点以外の点のまわりの置き直しは、その点の画面上の位置を保つ
            let mut c = OrbitCamera {
                yaw,
                pitch,
                ..OrbitCamera::default()
            };
            let pivot = c.target + c.rotation() * Vec3::new(0.3, -0.2, -0.5);
            let screen = c.view(640.0, 480.0).to_screen(pivot).unwrap();
            c.set_orientation_about(pivot, yaw + 40.0, pitch - 20.0);
            let moved = c.view(640.0, 480.0).to_screen(pivot).unwrap();
            assert!(
                (moved - screen).length() < 2e-3,
                "{view:?}: {screen} → {moved}"
            );
        }
    }

    #[test]
    fn setting_the_orientation_about_a_pivot_reaches_pitch_90_and_keeps_the_pivot_in_place() {
        let mut c = OrbitCamera::default();
        let pivot = c.target + c.rotation() * Vec3::new(0.3, -0.2, -0.5);
        let screen = c.view(640.0, 480.0).to_screen(pivot).unwrap();
        let distance = c.position().distance(pivot);
        c.set_orientation_about(pivot, 180.0, 90.0);
        assert_eq!((c.yaw, c.pitch), (180.0, 90.0));
        let moved = c.view(640.0, 480.0).to_screen(pivot).unwrap();
        assert!((moved - screen).length() < 2e-3, "{screen} → {moved}");
        assert!((c.position().distance(pivot) - distance).abs() < 1e-5);
        // 90° を越える値は 90° に止める
        c.set_orientation_about(pivot, 10.0, 200.0);
        assert_eq!(c.pitch, 90.0);
        c.set_orientation_about(pivot, 10.0, -200.0);
        assert_eq!(c.pitch, -90.0);
        // 回す（orbit）は、±90° にいるときはそのまま（はねない）。89° の内側からは今までどおり ±89° で止まる
        c.orbit(0.0, -1000.0);
        assert_eq!(c.pitch, -90.0);
        c.pitch = 0.0;
        c.orbit(0.0, -1000.0);
        assert_eq!(c.pitch, -89.0);
    }

    #[test]
    fn orbiting_from_straight_up_or_down_does_not_jump_to_the_89_degree_limit() {
        for pitch in [90.0f32, -90.0] {
            let mut c = OrbitCamera {
                yaw: 180.0,
                pitch,
                ..OrbitCamera::default()
            };
            // 動かさない・さらに向こうへ回すだけでは、pitch は変わらない
            c.orbit(10.0, 0.0);
            assert_eq!(c.pitch, pitch);
            c.orbit(0.0, 30.0 * pitch.signum());
            assert_eq!(c.pitch, pitch);
            assert!((c.yaw - (180.0 + 10.0 * ORBIT_DEGREES_PER_POINT)).abs() < 1e-5);
            // 戻す向きには、なめらかに離れる（1° はねない）
            c.orbit(0.0, -10.0 * pitch.signum());
            assert!(
                (c.pitch - (pitch - 3.5 * pitch.signum())).abs() < 1e-4,
                "{}",
                c.pitch
            );
            // 89° の内側から始めれば、今までどおり ±89° で止まる
            c.orbit(0.0, 1000.0 * pitch.signum());
            assert_eq!(c.pitch, 89.0 * pitch.signum());
        }
        // 90° を越える値は 90° までに止める（範囲を広げすぎない）
        assert_eq!(orbited(0.0, 120.0, 0.0, 100.0).1, 90.0);
    }

    #[test]
    fn orbiting_by_points_matches_the_free_orbit() {
        let mut c = OrbitCamera::default();
        let (yaw, pitch) = orbited(c.yaw, c.pitch, 30.0, -20.0);
        c.orbit(30.0, -20.0);
        assert_eq!((c.yaw, c.pitch), (yaw, pitch));
        assert!((yaw - (DEFAULT_YAW + 30.0 * ORBIT_DEGREES_PER_POINT)).abs() < 1e-5);
    }

    #[test]
    fn flying_moves_the_target_and_the_camera_together_without_changing_the_distance() {
        let mut c = OrbitCamera::default();
        let before = (c.position(), c.target, c.distance);
        c.fly(Vec3::new(0.0, 0.0, 0.5));
        let forward = c.rotation() * Vec3::Z;
        assert!((c.target - before.1 - forward * 0.5).length() < 1e-6);
        assert!((c.position() - before.0 - forward * 0.5).length() < 1e-6);
        assert_eq!(c.distance, before.2);
        // 右・上はカメラの右・上
        let (right, up) = (c.rotation() * Vec3::X, c.rotation() * Vec3::Y);
        let at = c.target;
        c.fly(Vec3::new(0.25, -0.5, 0.0));
        assert!((c.target - at - right * 0.25 + up * 0.5).length() < 1e-6);
        // 向きは変わらない
        assert_eq!((c.yaw, c.pitch), (DEFAULT_YAW, DEFAULT_PITCH));
    }

    #[test]
    fn navigation_limits() {
        let b = Bounds::new(Vec3::ZERO, Vec3::ONE);
        let mut c = OrbitCamera::framing(&b);
        c.orbit(0.0, 1000.0);
        assert_eq!(c.pitch, 89.0);
        for _ in 0..100 {
            c.zoom(10.0);
        }
        assert!((c.distance - c.model_radius * 0.05).abs() < 1e-6);
        let before = c.target;
        c.pan(10.0, 0.0, 500.0);
        assert!(
            (c.target - before).dot(c.rotation() * Vec3::X) < 0.0,
            "右へドラッグすると注視点は左へ"
        );
    }

    /// 正投影を入れる前の透視の式（同じ値・同じビットを返すことを確かめる元）。
    fn old_perspective(v: &CameraView, s: Vec2, p: Vec3) -> (Mat4, Ray, f32) {
        let up = v.rotation * Vec3::Y;
        let view = glam::camera::lh::view::look_to_mat4(v.position, v.forward, up);
        let proj = glam::camera::lh::proj::directx::perspective(
            FIELD_OF_VIEW.to_radians(),
            v.aspect(),
            v.near,
            v.far,
        );
        let vx = s.x / v.width * 2.0 - 1.0;
        let vy = 1.0 - s.y / v.height * 2.0;
        let t = (FIELD_OF_VIEW * 0.5).to_radians().tan();
        let local = Vec3::new(vx * t * v.aspect(), vy * t, 1.0) * v.near;
        let origin = v.position + v.rotation * local;
        let depth = (p - v.position).dot(v.forward);
        let radius = if depth <= v.near {
            0.0
        } else {
            0.1 * v.height / (2.0 * depth * t)
        };
        (proj * view, Ray::new(origin, origin - v.position), radius)
    }

    #[test]
    fn the_perspective_results_are_bit_for_bit_the_same_as_before() {
        for (yaw, pitch, distance) in [
            (DEFAULT_YAW, DEFAULT_PITCH, 3.0),
            (0.0, 90.0, 0.2),
            (-73.0, -41.0, 40.0),
        ] {
            let c = OrbitCamera {
                target: Vec3::new(0.3, -1.2, 2.0),
                yaw,
                pitch,
                distance,
                ..OrbitCamera::default()
            };
            let v = c.view(640.0, 360.0);
            assert_eq!(v.projection, Projection::Perspective);
            for (s, p) in [
                (Vec2::new(10.0, 20.0), c.target),
                (
                    Vec2::new(600.0, 300.0),
                    c.target + Vec3::new(0.4, 0.2, -0.3),
                ),
            ] {
                let (m, ray, radius) = old_perspective(&v, s, p);
                assert_eq!(v.view_projection().to_cols_array(), m.to_cols_array());
                assert_eq!(v.ray(s), ray);
                assert_eq!(v.world_radius_to_screen(p, 0.1).to_bits(), radius.to_bits());
                // 見る人への向き・視線・奥行きの点は、各所が書いていた式と同じ
                assert_eq!(v.to_viewer(p), v.position - p);
                let sight = v.sight(p);
                assert_eq!((sight.origin, sight.to), (v.position, p - v.position));
                assert_eq!(
                    sight.distance.to_bits(),
                    (p - v.position).length().to_bits()
                );
                let r = v.ray(s);
                assert_eq!(
                    v.point_at_depth(s, c.distance),
                    v.position + r.direction() * (c.distance / r.direction().dot(v.forward))
                );
                assert_eq!(v.viewer(), Viewer::Point(v.position));
            }
        }
    }

    fn ortho(yaw: f32, pitch: f32) -> OrbitCamera {
        let mut c = OrbitCamera {
            target: Vec3::new(1.0, 2.0, 3.0),
            yaw,
            pitch,
            ..OrbitCamera::default()
        };
        c.set_orthographic(true);
        c
    }

    #[test]
    fn orthographic_rays_are_parallel_and_project_back_to_the_same_point() {
        for (yaw, pitch) in [
            (DEFAULT_YAW, DEFAULT_PITCH),
            (0.0, 0.0),
            (180.0, 90.0),
            (180.0, -90.0),
        ] {
            let c = ortho(yaw, pitch);
            let v = c.view(400.0, 300.0);
            assert!(v.is_orthographic());
            assert!(v
                .view_projection()
                .to_cols_array()
                .iter()
                .all(|x| x.is_finite()));
            // 画面の真ん中のレイは注視点を通る
            let r = v.ray(Vec2::new(200.0, 150.0));
            let off = (c.target - r.origin())
                - r.direction() * (c.target - r.origin()).dot(r.direction());
            assert!(off.length() < 1e-4, "{off}");
            for s in [
                Vec2::new(10.0, 20.0),
                Vec2::new(390.0, 290.0),
                Vec2::new(200.0, 40.0),
            ] {
                let r = v.ray(s);
                assert!((r.direction() - v.forward).length() < 1e-6);
                // 始めは近い面の上
                assert!((v.depth(r.origin()) - v.near).abs() < 1e-4);
                // 近い面のすぐ奥から（近い面の上ちょうどは、丸めで手前に出ると写さない）
                for t in [0.001, 1.0, 5.0] {
                    let back = v.to_screen(r.point(t)).unwrap();
                    assert!((back - s).length() < 2e-2, "{s} → {back}");
                }
            }
            // 画面の右・上へ向く物は、画面の右・上に写る
            let right = v
                .to_screen(c.target + c.rotation() * Vec3::X * 0.1)
                .unwrap();
            let up = v
                .to_screen(c.target + c.rotation() * Vec3::Y * 0.1)
                .unwrap();
            assert!(right.x > 200.0 && (right.y - 150.0).abs() < 1e-2);
            assert!(up.y < 150.0 && (up.x - 200.0).abs() < 1e-2);
        }
    }

    #[test]
    fn the_orthographic_radius_on_screen_does_not_depend_on_depth() {
        let c = ortho(DEFAULT_YAW, DEFAULT_PITCH);
        let v = c.view(500.0, 400.0);
        let height = c.height_at_target();
        let forward = c.rotation() * Vec3::Z;
        for depth in [-0.5f32, 0.0, 0.7, 2.0] {
            let p = c.target + forward * depth;
            let px = v.world_radius_to_screen(p, 0.1);
            assert!((px - 0.1 * 400.0 / height).abs() < 1e-4, "{depth}: {px}");
            let edge = v.to_screen(p + c.rotation() * Vec3::X * 0.1).unwrap();
            let center = v.to_screen(p).unwrap();
            assert!(
                (px - (edge.x - center.x)).abs() < 0.05,
                "{px} {edge} {center}"
            );
        }
        // 近い面より手前は 0（透視と同じ決まり）
        assert_eq!(
            v.world_radius_to_screen(v.position + forward * (v.near - 0.1), 0.1),
            0.0
        );
    }

    #[test]
    fn switching_the_projection_keeps_the_size_at_the_target() {
        let mut c = OrbitCamera::default();
        let persp = c.view(640.0, 480.0).world_radius_to_screen(c.target, 0.2);
        c.set_orthographic(true);
        assert_eq!(
            c.projection,
            Projection::Orthographic {
                height: visible_height(c.distance)
            }
        );
        let ortho = c.view(640.0, 480.0).world_radius_to_screen(c.target, 0.2);
        assert!((persp - ortho).abs() < 1e-3, "{persp} {ortho}");
        // 同じ投影へは何もしない
        let before = c;
        c.set_orthographic(true);
        assert_eq!(c, before);
        // 正投影で寄ってから透視へ戻すと、その大きさに見える距離へ
        c.zoom(3.0);
        let zoomed = c.view(640.0, 480.0).world_radius_to_screen(c.target, 0.2);
        c.set_orthographic(false);
        assert!(!c.is_orthographic());
        let back = c.view(640.0, 480.0).world_radius_to_screen(c.target, 0.2);
        assert!((zoomed - back).abs() < 1e-2 * zoomed, "{zoomed} {back}");
        // 戻す距離は寄る・引く範囲に収める
        let mut far = OrbitCamera {
            projection: Projection::Orthographic { height: 1e9 },
            ..OrbitCamera::default()
        };
        far.set_orthographic(false);
        assert_eq!(far.distance, far.model_radius * 100.0);
    }

    #[test]
    fn switching_about_a_pivot_keeps_its_place_and_size_on_screen() {
        for offset in [Vec3::new(0.3, -0.2, -0.6), Vec3::new(-0.4, 0.1, 0.8)] {
            let mut c = OrbitCamera::default();
            let pivot = c.target + c.rotation() * offset;
            let probe = pivot + c.rotation() * Vec3::X * 0.05;
            let at = |c: &OrbitCamera| {
                let v = c.view(640.0, 480.0);
                (v.to_screen(pivot).unwrap(), v.to_screen(probe).unwrap())
            };
            let before = at(&c);
            c.set_orthographic_about(true, pivot);
            let ortho = at(&c);
            assert!((ortho.0 - before.0).length() < 1e-2, "{before:?} {ortho:?}");
            assert!((ortho.1 - before.1).length() < 1e-2, "{before:?} {ortho:?}");
            // 正投影で寄ってから戻しても、その点の所と大きさは正投影のまま
            c.zoom_towards(pivot, 1.5);
            let zoomed = at(&c);
            let target = c.target;
            c.set_orthographic_about(false, pivot);
            let back = at(&c);
            assert!((back.0 - zoomed.0).length() < 2e-2, "{zoomed:?} {back:?}");
            assert!((back.1 - zoomed.1).length() < 2e-2, "{zoomed:?} {back:?}");
            assert_eq!(c.target, target, "注視点は動かさない");
        }
        // カメラの後ろの点は、注視点で（set_orthographic と同じ）
        let mut a = OrbitCamera::default();
        let mut b = a;
        a.set_orthographic_about(true, a.position() - a.rotation() * Vec3::Z);
        b.set_orthographic(true);
        assert_eq!(a, b);
    }

    #[test]
    fn orthographic_zoom_and_pan_change_the_height_and_not_the_camera_depth() {
        let mut c = ortho(DEFAULT_YAW, DEFAULT_PITCH);
        let (position, distance) = (c.position(), c.distance);
        let h = c.height_at_target();
        c.zoom(1.0);
        assert!((c.height_at_target() - h * (-0.18f32).exp()).abs() < 1e-5);
        assert_eq!(
            (c.position(), c.distance),
            (position, distance),
            "カメラは動かない"
        );
        for _ in 0..200 {
            c.zoom(10.0);
        }
        assert!((c.height_at_target() - visible_height(c.model_radius * 0.05)).abs() < 1e-5);
        for _ in 0..200 {
            c.zoom(-10.0);
        }
        assert!((c.height_at_target() - visible_height(c.model_radius * 100.0)).abs() < 1e-2);
        // パン: 指についてくる（奥行きによらない）
        let mut c = ortho(DEFAULT_YAW, DEFAULT_PITCH);
        let forward = c.rotation() * Vec3::Z;
        let p = c.target + c.rotation() * Vec3::new(0.2, 0.1, 0.0) + forward * 1.5;
        let before = c.view(640.0, 480.0).to_screen(p).unwrap();
        c.pan(43.0, -21.0, 480.0);
        let after = c.view(640.0, 480.0).to_screen(p).unwrap();
        assert!((after - before - Vec2::new(43.0, -21.0)).length() < 0.002);
        let mut d = c;
        d.pan_at_depth(10.0, 5.0, 480.0, 123.0);
        c.pan(10.0, 5.0, 480.0);
        assert_eq!(c, d, "正投影は奥行きによらない");
        // ポインタへの拡縮: 点の画面の位置を保ち、注視点の奥行きは変えない
        let mut c = ortho(DEFAULT_YAW, DEFAULT_PITCH);
        let p = c.target + c.rotation() * Vec3::new(0.4, -0.2, -0.5);
        let before = c.view(640.0, 480.0).to_screen(p).unwrap();
        let depth = c.target.dot(forward);
        for notches in [1.0, -2.0, 100.0, 1.0, -100.0, -1.0] {
            c.zoom_towards(p, notches);
            assert!((c.view(640.0, 480.0).to_screen(p).unwrap() - before).length() < 0.03);
            assert!((c.target.dot(forward) - depth).abs() < 1e-4);
        }
    }

    #[test]
    fn orthographic_zoom_outside_the_range_only_limits_the_way_it_moves() {
        // 透視でいちばん寄った所から、注視点より手前の点のまわりで正投影へ: 高さは拡縮の下限より小さい
        let mut c = OrbitCamera {
            distance: 0.05,
            model_radius: 1.0,
            ..OrbitCamera::default()
        };
        let pivot = c.target - c.rotation() * Vec3::Z * 0.03;
        c.set_orthographic_about(true, pivot);
        let h = c.height_at_target();
        assert!(h < visible_height(0.05), "{h}");
        // 1 目盛り寄っても引いた絵にならない（今の高さのまま）、引けば普通に引く
        c.zoom(1.0);
        assert_eq!(c.height_at_target(), h);
        c.zoom(-1.0);
        assert!((c.height_at_target() - h * 0.18f32.exp()).abs() < 1e-6);
        // 上限の側も同じ
        let mut c = OrbitCamera {
            projection: Projection::Orthographic {
                height: visible_height(150.0),
            },
            ..OrbitCamera::default()
        };
        c.zoom(-1.0);
        assert_eq!(c.height_at_target(), visible_height(150.0));
        c.zoom(1.0);
        assert!((c.height_at_target() - visible_height(150.0) * (-0.18f32).exp()).abs() < 1e-2);
        // 前後の移動（右を押している間の W/S）も同じ制限
        c.fly(Vec3::new(0.0, 0.0, -1000.0));
        assert!(c.height_at_target() <= visible_height(150.0) + 1e-2);
    }

    #[test]
    fn flying_forward_in_orthographic_zooms_instead_of_moving_the_camera() {
        let mut c = OrbitCamera {
            target: Vec3::new(1.0, 2.0, 3.0),
            ..OrbitCamera::default()
        };
        c.set_orthographic(true);
        let (position, h) = (c.position(), c.height_at_target());
        c.fly(Vec3::new(0.0, 0.0, 0.3));
        assert_eq!(c.position(), position, "前後ではカメラは動かない");
        assert!((c.height_at_target() - h * (-0.3 / c.distance).exp()).abs() < 1e-5);
        c.fly(Vec3::new(0.0, 0.0, -0.3));
        assert!((c.height_at_target() - h).abs() < 1e-5);
        // 左右・上下は今までどおり一緒に動く
        let (right, up) = (c.rotation() * Vec3::X, c.rotation() * Vec3::Y);
        c.fly(Vec3::new(0.25, -0.5, 0.0));
        assert!((c.position() - position - right * 0.25 + up * 0.5).length() < 1e-5);
    }

    #[test]
    fn orthographic_does_not_place_points_in_front_of_the_near_plane() {
        let c = ortho(DEFAULT_YAW, DEFAULT_PITCH);
        let v = c.view(400.0, 300.0);
        let f = v.forward;
        assert!(v.to_screen(v.position + f * (v.near + 0.01)).is_some());
        assert!(v.to_screen(v.position + f * (v.near - 0.01)).is_none());
        assert!(v.to_screen(v.position - f * 5.0).is_none());
    }

    #[test]
    fn the_orthographic_volume_keeps_the_model_even_with_the_camera_inside_it() {
        // 透視で寄ったまま（カメラが半径の内側）切り替える
        let mut c = OrbitCamera {
            distance: 0.05,
            model_radius: 1.0,
            ..OrbitCamera::default()
        };
        c.set_orthographic(true);
        let v = c.view(400.0, 400.0);
        let forward = c.rotation() * Vec3::Z;
        assert!(v.near < 0.0, "近い面はカメラの後ろ: {}", v.near);
        for offset in [-1.9f32, -1.0, 0.0, 1.0, 1.9] {
            let p = c.target + forward * offset;
            let clip = v.view_projection() * p.extend(1.0);
            let z = clip.z / clip.w;
            assert!((0.0..=1.0).contains(&z), "{offset}: {z}");
        }
        // 離れていれば透視と同じ近い面
        let far = ortho(DEFAULT_YAW, DEFAULT_PITCH).view(400.0, 400.0);
        let persp = OrbitCamera {
            target: Vec3::new(1.0, 2.0, 3.0),
            ..OrbitCamera::default()
        }
        .view(400.0, 400.0);
        assert_eq!((far.near, far.far), (persp.near, persp.far));
    }

    #[test]
    fn orthographic_sight_and_viewer_are_parallel_to_the_view() {
        let c = ortho(DEFAULT_YAW, DEFAULT_PITCH);
        let v = c.view(400.0, 300.0);
        let p = c.target + Vec3::new(0.3, -0.2, 0.1);
        assert_eq!(v.to_viewer(p), -v.forward);
        let sight = v.sight(p);
        assert!((sight.origin + sight.to - p).length() < 1e-5);
        assert!((sight.to.normalize() - v.forward).length() < 1e-5);
        assert!((sight.distance - (v.depth(p) - v.near)).abs() < 1e-5);
        assert!((sight.ray().direction() - v.forward).length() < 1e-6);
        // 見る人（ダブ）の視線も同じ
        let Viewer::Parallel { forward, near } = v.viewer() else {
            panic!("平行")
        };
        assert_eq!(forward, v.forward);
        let s = v.viewer().sight(p);
        assert!((s.distance - sight.distance).abs() < 1e-4);
        assert!((near - (v.position.dot(v.forward) + v.near)).abs() < 1e-6);
        // 画面の点の、注視点と同じ奥行きの点
        let at = v.point_at_depth(Vec2::new(120.0, 80.0), c.distance);
        assert!((v.depth(at) - c.distance).abs() < 1e-4);
        assert!((v.to_screen(at).unwrap() - Vec2::new(120.0, 80.0)).length() < 1e-2);
    }

    #[test]
    fn framing_the_bounds_in_orthographic_keeps_the_projection_and_fits() {
        let b = Bounds::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(2.0, 1.0, 3.0));
        for (w, h) in [(800.0, 300.0), (120.0, 600.0)] {
            let mut c = ortho(DEFAULT_YAW, DEFAULT_PITCH);
            c.frame_bounds(&b, w, h);
            assert!(c.is_orthographic());
            assert_eq!(c.height_at_target(), visible_height(c.distance));
            for x in [-1.0, 1.0] {
                for y in [-1.0, 1.0] {
                    for z in [-1.0, 1.0] {
                        let p = c
                            .view(w, h)
                            .to_screen(b.center + b.extents * Vec3::new(x, y, z))
                            .unwrap();
                        assert!(p.x > 0.0 && p.x < w && p.y > 0.0 && p.y < h, "{p}");
                    }
                }
            }
        }
        // 開いた直後・全体を表示は透視
        assert_eq!(OrbitCamera::framing(&b).projection, Projection::Perspective);
    }

    #[test]
    fn only_orientations_on_an_axis_are_aligned() {
        for view in AxisView::ALL {
            let (yaw, pitch) = view.orientation();
            assert_eq!(aligned_axis_view(yaw, pitch), Some(view));
            assert_eq!(aligned_axis_view(yaw + 720.0, pitch), Some(view));
            // 吸い付いた向きもそろう
            let (y, p) = snap_orientation(yaw + 10.0, pitch.clamp(-80.0, 80.0));
            assert_eq!(aligned_axis_view(y, p), Some(view), "{view:?}");
            // 少しでも回せば外れる
            assert_eq!(aligned_axis_view(yaw + 1.0, pitch.clamp(-89.0, 89.0)), None);
            // 側はカメラのいる側
            let c = OrbitCamera {
                yaw,
                pitch,
                ..OrbitCamera::default()
            };
            assert!(((c.position() - c.target).normalize() - view.side()).length() < 1e-5);
        }
        assert_eq!(aligned_axis_view(DEFAULT_YAW, DEFAULT_PITCH), None);
    }
}
