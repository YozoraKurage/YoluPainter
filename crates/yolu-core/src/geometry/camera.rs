//! 3D ビューのカメラ（Unity 版の IsolatedModelPreview のカメラと操作）: 注視点のまわりを回る（yaw・pitch は度、Unity の
//! `Quaternion.Euler(pitch, yaw, 0)`）、縦の画角 30°、注視点からの距離。左手系（Unity と同じ。前は +Z、上は +Y）。
//!
//! 画面の点からのレイ・画面への写し・世界の半径の画面での大きさ（ブラシのカーソルとストロークの間隔）、wgpu の描画の行列（深度 0〜1）
//! を同じカメラから出す。

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::unity::{Bounds, Ray};

/// 縦の画角（度。Unity 版と同じ）。
pub const FIELD_OF_VIEW: f32 = 30.0;

/// 注視点のまわりを回るカメラ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    pub target: Vec3,
    /// 度。
    pub yaw: f32,
    /// 度（−89〜89）。
    pub pitch: f32,
    pub distance: f32,
    /// モデルの半径（寄る範囲・近い面と遠い面の目安）。
    pub model_radius: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        OrbitCamera {
            target: Vec3::ZERO,
            yaw: 25.0,
            pitch: 10.0,
            distance: 3.0,
            model_radius: 1.0,
        }
    }
}

impl OrbitCamera {
    /// モデル全体が入る位置（Unity 版の FrameModel: 中心を見て yaw 25°・pitch 10°、距離は半径 / sin 15° × 1.15）。
    pub fn framing(bounds: &Bounds) -> OrbitCamera {
        let radius = super::unity::fmax(0.0001, super::unity::magnitude(bounds.extents));
        OrbitCamera {
            target: bounds.center,
            yaw: 25.0,
            pitch: 10.0,
            distance: radius / (15.0f32).to_radians().sin() * 1.15,
            model_radius: radius,
        }
    }

    /// 向き（`Quaternion.Euler(pitch, yaw, 0)`: Z、X、Y の順に回す）。
    pub fn rotation(&self) -> Quat {
        Quat::from_euler(
            glam::EulerRot::YXZ,
            self.yaw.to_radians(),
            self.pitch.to_radians(),
            0.0,
        )
    }

    pub fn position(&self) -> Vec3 {
        self.target - self.rotation() * Vec3::Z * self.distance
    }

    /// 回す（Unity 版: 画面の点 1 つで 0.35°。下へドラッグすると上から見下ろす）。
    pub fn orbit(&mut self, dx_points: f32, dy_points: f32) {
        self.yaw += dx_points * 0.35;
        self.pitch = (self.pitch + dy_points * 0.35).clamp(-89.0, 89.0);
    }

    /// パン（ポインタの下の注視点の面が指についてくる。view_height は表示域の高さの点）。
    pub fn pan(&mut self, dx_points: f32, dy_points: f32, view_height: f32) {
        let units =
            2.0 * self.distance * (FIELD_OF_VIEW * 0.5).to_radians().tan() / view_height.max(1.0);
        self.target += self.rotation() * Vec3::new(-dx_points * units, dy_points * units, 0.0);
    }

    /// 寄る・引く（notches はホイールの目盛り、正で寄る。1 目盛りで約 1.2 倍。Unity 版の exp(0.06 × 3)）。
    pub fn zoom(&mut self, notches: f32) {
        let r = self.model_radius;
        self.distance = (self.distance * (-notches * 0.18).exp()).clamp(r * 0.05, r * 100.0);
    }

    /// 任意の点のまわりを回す。その点の画面上の位置は変えない。
    pub fn orbit_about(&mut self, pivot: Vec3, dx_points: f32, dy_points: f32) {
        let before = self.rotation();
        self.orbit(dx_points, dy_points);
        self.target = pivot + (self.rotation() * before.inverse()) * (self.target - pivot);
    }

    /// 押した点の奥行きに合わせてパンする（奥行きは視線方向の距離）。
    pub fn pan_at_depth(&mut self, dx_points: f32, dy_points: f32, view_height: f32, depth: f32) {
        let units = 2.0 * depth * (FIELD_OF_VIEW * 0.5).to_radians().tan() / view_height.max(1.0);
        self.target += self.rotation() * Vec3::new(-dx_points * units, dy_points * units, 0.0);
    }

    /// 点へ寄る。その点と同じレイの画面位置を保ち、従来の距離制限を使う。
    pub fn zoom_towards(&mut self, point: Vec3, notches: f32) {
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
    }

    /// この大きさ（物理の画素）の表示域で見たときのカメラ。
    pub fn view(&self, width: f32, height: f32) -> CameraView {
        let rotation = self.rotation();
        let near = 0.00001f32.max((self.distance * 0.02).min(self.model_radius * 0.01));
        let far = (self.distance + self.model_radius * 20.0).max(near + 1.0);
        CameraView {
            position: self.position(),
            rotation,
            forward: rotation * Vec3::Z,
            near,
            far,
            width: width.max(1.0),
            height: height.max(1.0),
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
}

impl CameraView {
    fn tan_half(&self) -> f32 {
        (FIELD_OF_VIEW * 0.5).to_radians().tan()
    }

    pub fn aspect(&self) -> f32 {
        self.width / self.height
    }

    /// 世界 → 切り取りの行列（wgpu 向け。左手系、深度 0〜1）。
    pub fn view_projection(&self) -> Mat4 {
        let up = self.rotation * Vec3::Y;
        let view = glam::camera::lh::view::look_to_mat4(self.position, self.forward, up);
        // wgpu の切り取りの空間は D3D と同じ（y が上、深度 0〜1）
        let proj = glam::camera::lh::proj::directx::perspective(
            FIELD_OF_VIEW.to_radians(),
            self.aspect(),
            self.near,
            self.far,
        );
        proj * view
    }

    /// 画面の点を通るレイ（始点は近い面の上。Unity の ViewportPointToRay と同じ考え方）。
    pub fn ray(&self, screen: Vec2) -> Ray {
        let vx = screen.x / self.width * 2.0 - 1.0;
        let vy = 1.0 - screen.y / self.height * 2.0;
        let t = self.tan_half();
        let local = Vec3::new(vx * t * self.aspect(), vy * t, 1.0) * self.near;
        let origin = self.position + self.rotation * local;
        Ray::new(origin, origin - self.position)
    }

    /// 世界の点が画面のどこに見えるか（カメラの後ろなら None）。
    pub fn to_screen(&self, world: Vec3) -> Option<Vec2> {
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

    /// 世界の点の所での半径（モデルの単位）が画面で何画素か（Unity 版の WorldRadiusToGuiPoints。近い面より手前なら 0）。
    pub fn world_radius_to_screen(&self, world: Vec3, radius: f32) -> f32 {
        if radius <= 0.0 {
            return 0.0;
        }
        let depth = (world - self.position).dot(self.forward);
        if depth <= self.near {
            return 0.0;
        }
        radius * self.height / (2.0 * depth * self.tan_half())
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
        assert!((to_target - r.direction()).length() < 1e-5);
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
            assert!((back - s).length() < 1e-2, "{s} → {back}");
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
}
