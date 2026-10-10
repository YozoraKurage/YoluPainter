//! 形のギズモ（3D ビュー）: 形のグラデーションの範囲と、塗りつぶしの投影・デカールの箱を、モデルの面の上で動かす・回す・大きさを変える
//! ための線とハンドル（Unity 版の `ShapeGizmo` と同じ操作）。形はモデルのルートの空間（シーンの単位）にあり、ここは画面の幾何と
//! 当たり判定とドラッグの計算だけ。値を文書へ入れるのは `crate::fillfx::gizmo`。
//!
//! - 1 つのギズモに、中心の四角・移動の矢印・回す輪・大きさのつまみを同時に出す（切り替えは無い。大きさを持たない定規の物 `Kind::Anchor` は、大きさのつまみを出さない）。輪は矢印の外側（矢印の長さ
//!   `ARROW_POINTS` < 輪の半径 `RING_POINTS`）。重なった所の当たりは、中心の四角 → 大きさのつまみ → 矢印 → 輪の順（小さな的を先に）。
//! - 移動の矢印はルートの軸に沿い、掴むと中心がその軸に沿って動く（押したときのレイと今のレイが、軸の直線に最も近づく点の差）。
//!   中心の四角は、カメラに向いた面の中で動かす。
//! - 輪はルートの軸のまわりに形を回す（押したときと今のレイが輪の面に当たる点の角度の差。Ctrl で 15° ずつ）。真横から見た輪は、画面の
//!   接線に沿ったポインタの動きで回す。輪の奥の半分は薄く描く（掴める）。
//! - 大きさのつまみは形の面（自分の軸）の上にあり、掴んだ面だけが動いて反対の面は動かない（中心が追う）。Shift で両側が動いて中心は動かない。
//!   球のつまみは半径を変える（中心は動かない）。平面のつまみは 0 と 1 の境で、ランプの幅を変える。
//! - ドラッグは始まりの形とポインタの位置から毎回計算する（ずれが溜まらない）。軸がカメラを向いていて決まらないドラッグは形を変えない。
//!
//! 角度は度、回転の順は Unity の Z → X → Y（`Quat::from_euler(YXZ, y, x, z)`）。

use egui::Color32;
use yolu_core::fill_image::Placement;
use yolu_core::generator::{Shape as GenShape, Volume};
use yolu_core::geometry::CameraView;
use yolu_core::glam::{Mat3, Quat, Vec2, Vec3};

/// 画面での長さ（点）: 移動の矢印・輪の半径（矢印より外）・ハンドルを掴める近さ・つまみと中心の四角の大きさ。
pub const ARROW_POINTS: f32 = 64.0;
pub const RING_POINTS: f32 = 88.0;
pub const GRAB_POINTS: f32 = 7.0;
pub const KNOB_POINTS: f32 = 10.0;
pub const CENTER_POINTS: f32 = 13.0;
/// 輪を Ctrl で回すときの刻み（度）。
pub const SNAP_DEGREES: f32 = 15.0;
/// 軸がカメラの向きとこれ以上平行なら、その軸のハンドルは出さない（約 14°）。
const FACING_CAMERA: f32 = 0.97;

pub const AXIS_X: Color32 = Color32::from_rgb(245, 77, 64);
pub const AXIS_Y: Color32 = Color32::from_rgb(115, 224, 64);
pub const AXIS_Z: Color32 = Color32::from_rgb(64, 133, 255);
pub const HOVER: Color32 = Color32::from_rgb(255, 217, 51);
pub const OUTLINE: Color32 = Color32::from_rgba_premultiplied(242, 148, 48, 242);
pub const INNER: Color32 = Color32::from_rgba_premultiplied(115, 70, 23, 115);

/// 形の限界（core の `Volume::validate` と同じ）。
pub const MIN_SIZE: f64 = 1e-6;
pub const MAX_SIZE: f64 = 1e6;

/// 掴めるもの。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Handle {
    #[default]
    None,
    MoveX,
    MoveY,
    MoveZ,
    MoveFree,
    RotateX,
    RotateY,
    RotateZ,
    SizeXPos,
    SizeXNeg,
    SizeYPos,
    SizeYNeg,
    SizeZPos,
    SizeZNeg,
    /// 定規の端の点 a・b（`Kind::Anchor` の形のとき、`rulers::edit3d` が出して掴む。形の幾何の計算には出てこない）。
    EndA,
    EndB,
}

impl Handle {
    fn move_axis(i: usize) -> Handle {
        [Handle::MoveX, Handle::MoveY, Handle::MoveZ][i]
    }
    fn rotate_axis(i: usize) -> Handle {
        [Handle::RotateX, Handle::RotateY, Handle::RotateZ][i]
    }
    fn size_knob(axis: usize, positive: bool) -> Handle {
        [
            [Handle::SizeXPos, Handle::SizeXNeg],
            [Handle::SizeYPos, Handle::SizeYNeg],
            [Handle::SizeZPos, Handle::SizeZNeg],
        ][axis][usize::from(!positive)]
    }
    /// 大きさのつまみなら、(軸, 正の側か)。
    fn knob_of(self) -> Option<(usize, bool)> {
        Some(match self {
            Handle::SizeXPos => (0, true),
            Handle::SizeXNeg => (0, false),
            Handle::SizeYPos => (1, true),
            Handle::SizeYNeg => (1, false),
            Handle::SizeZPos => (2, true),
            Handle::SizeZNeg => (2, false),
            _ => return None,
        })
    }
    pub fn is_size(self) -> bool {
        self.knob_of().is_some()
    }
}

/// 形の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Box,
    Sphere,
    Plane,
    /// 大きさを持たない物（定規）。外形も大きさのつまみも出さず、中心の四角・移動の矢印・回す輪だけ。
    Anchor,
}

/// ギズモが動かす形（モデルのルートの空間、シーンの単位。回転は度）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub kind: Kind,
    pub center: [f64; 3],
    pub rotation: [f64; 3],
    pub size: [f64; 3],
    pub falloff: f64,
}

impl Shape {
    pub fn from_volume(v: &Volume) -> Shape {
        Shape {
            kind: match v.shape {
                GenShape::Box => Kind::Box,
                GenShape::Sphere => Kind::Sphere,
                GenShape::Plane => Kind::Plane,
            },
            center: v.center,
            rotation: v.rotation,
            size: v.size,
            falloff: v.falloff,
        }
    }
    /// 形のグラデーションの形へ戻す（形の種類と減衰は元のものを保つ）。
    pub fn into_volume(self, base: &Volume) -> Volume {
        Volume {
            center: self.center,
            rotation: self.rotation,
            size: self.size,
            ..*base
        }
    }
    /// 投影の置き場（箱）。`sphere` なら球として見せる（球の投影）。
    pub fn from_placement(p: &Placement, sphere: bool) -> Shape {
        Shape {
            kind: if sphere { Kind::Sphere } else { Kind::Box },
            center: p.center,
            rotation: p.rotation,
            size: p.size,
            falloff: 0.0,
        }
    }
    pub fn into_placement(self) -> Placement {
        Placement {
            center: self.center,
            rotation: self.rotation,
            size: self.size,
        }
    }
}

/// モデルのルートの位置と向き（形はこの空間にある）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Root {
    pub position: Vec3,
    pub rotation: Quat,
}

impl Default for Root {
    fn default() -> Self {
        Root {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
        }
    }
}

/// 増分の吸着（押した所からの値。移動はルートの軸ごと、大きさは全幅、回転は度）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    pub movement: Vec3,
    pub size: f32,
    pub rotation: f32,
}

impl Default for Snap {
    fn default() -> Self {
        Snap {
            movement: Vec3::ONE,
            size: 0.1,
            rotation: SNAP_DEGREES,
        }
    }
}

fn snapped(value: f32, increment: f32) -> f32 {
    if increment > 0.0 && increment.is_finite() {
        (value / increment).round() * increment
    } else {
        value
    }
}

/// 折れ線を足す関数（カメラの後ろの点で切る）。
type Polyline<'a> = &'a mut dyn FnMut(&[Vec3], Color32, f32, &mut Vec<Line>);

/// 描く線 1 本（画面の点は表示域の左上から）。`filled` なら塗る凸の多角形（矢じり）。
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub points: Vec<Vec2>,
    pub color: Color32,
    pub width: f32,
    pub filled: bool,
}

fn axis(i: usize) -> Vec3 {
    [Vec3::X, Vec3::Y, Vec3::Z][i]
}

fn axis_color(i: usize) -> Color32 {
    [AXIS_X, AXIS_Y, AXIS_Z][i]
}

fn v3(a: [f64; 3]) -> Vec3 {
    Vec3::new(a[0] as f32, a[1] as f32, a[2] as f32)
}

/// 形の回転（Unity の `Quaternion.Euler(x, y, z)`: Z → X → Y の順）。
pub fn rotation_of(s: &Shape) -> Quat {
    let [x, y, z] = s.rotation.map(|d| (d as f32).to_radians());
    Quat::from_euler(yolu_core::glam::EulerRot::YXZ, y, x, z)
}

/// 回転を度の (x, y, z)（Z → X → Y の順）にして、各軸を −180〜180 に収める。
pub fn euler_degrees(q: Quat) -> [f64; 3] {
    let m = Mat3::from_quat(q.normalize());
    // R = Ry(b)·Rx(a)·Rz(c)（core の `Placement::matrix` と同じ並び）: R[1][2] = −sin a、R[0][2] = sin b·cos a、R[2][2] = cos b·cos a、
    // R[1][0] = cos a·sin c、R[1][1] = cos a·cos c
    let r = |row: usize, col: usize| m.col(col)[row];
    let sin_a = (-r(1, 2)).clamp(-1.0, 1.0);
    let a = sin_a.asin();
    let (b, c) = if sin_a.abs() < 0.999_999 {
        (r(0, 2).atan2(r(2, 2)), r(1, 0).atan2(r(1, 1)))
    } else {
        // 真上・真下を向く（ジンバルロック）: z を 0 にして y に寄せる
        ((-r(2, 0)).atan2(r(0, 0)), 0.0)
    };
    [a, b, c].map(|v| wrap_degrees(v.to_degrees() as f64))
}

/// 角度を −180〜180 に。
pub fn wrap_degrees(degrees: f64) -> f64 {
    if (-180.0..=180.0).contains(&degrees) {
        return degrees;
    }
    let wrapped = degrees % 360.0;
    if wrapped > 180.0 {
        wrapped - 360.0
    } else if wrapped <= -180.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

pub fn world_center(s: &Shape, root: &Root) -> Vec3 {
    root.position + root.rotation * v3(s.center)
}

pub fn world_rotation(s: &Shape, root: &Root) -> Quat {
    root.rotation * rotation_of(s)
}

/// その点での画面の 1 点が、シーンで何の長さか（見えなければ 0）。
pub fn world_per_point(view: &CameraView, at: Vec3) -> f32 {
    let per_unit = view.world_radius_to_screen(at, 1.0);
    if per_unit > 0.0 {
        1.0 / per_unit
    } else {
        0.0
    }
}

pub(crate) fn ray_of(view: &CameraView, gui: Vec2) -> (Vec3, Vec3) {
    let r = view.ray(gui);
    (r.origin(), r.direction())
}

/// 軸の方向がカメラを向いていないか（向いているとき、その軸のハンドルは中心に重なって掴めない）。
fn axis_visible(view: &CameraView, c: Vec3, a: Vec3) -> bool {
    let Some(g) = view.to_screen(c) else {
        return false;
    };
    let (_, dir) = ray_of(view, g);
    a.dot(dir).abs() <= FACING_CAMERA
}

/// 大きさのつまみの世界の位置: 箱は 6 つの面の中心、球は軸の上の表面の 6 点、平面は 0（後ろ）と 1（前）の境の 2 つ。
pub fn knobs(s: &Shape, root: &Root) -> Vec<(Handle, Vec3)> {
    let c = world_center(s, root);
    let q = world_rotation(s, root);
    let half = s.size.map(|v| v as f32 / 2.0);
    let mut list = Vec::new();
    if s.kind == Kind::Anchor {
        return list;
    }
    for i in 0..3 {
        if s.kind == Kind::Plane && i != 1 {
            continue;
        }
        let h = if s.kind == Kind::Sphere {
            half[0]
        } else {
            half[i]
        };
        list.push((Handle::size_knob(i, true), c + q * axis(i) * h));
        list.push((Handle::size_knob(i, false), c - q * axis(i) * h));
    }
    list
}

/// 軸がカメラを向いていないつまみ（向いたものは中心に重なって掴めない）。
fn visible_knobs(s: &Shape, root: &Root, view: &CameraView) -> Vec<(Handle, Vec3)> {
    let q = world_rotation(s, root);
    knobs(s, root)
        .into_iter()
        .filter(|(handle, world)| {
            let Some((axis_index, _)) = handle.knob_of() else {
                return false;
            };
            let a = q * axis(axis_index);
            let Some(g) = view.to_screen(*world) else {
                return false;
            };
            let (_, dir) = ray_of(view, g);
            a.dot(dir).abs() <= FACING_CAMERA
        })
        .collect()
}

/// 出すハンドルと、それを表す画面の点（つまみ・四角の中心・矢印の先・輪の上の掴める点）。
pub fn handle_points(s: &Shape, root: &Root, view: &CameraView) -> Vec<(Handle, Vec2)> {
    let mut list = Vec::new();
    let c = world_center(s, root);
    let unit = world_per_point(view, c);
    let Some(cg) = view.to_screen(c) else {
        return list;
    };
    if unit <= 0.0 {
        return list;
    }
    for (handle, world) in visible_knobs(s, root, view) {
        if let Some(g) = view.to_screen(world) {
            list.push((handle, g));
        }
    }
    list.push((Handle::MoveFree, cg));
    for i in 0..3 {
        let a = root.rotation * axis(i);
        if axis_visible(view, c, a) {
            if let Some(tip) = view.to_screen(c + a * ARROW_POINTS * unit) {
                list.push((Handle::move_axis(i), tip));
            }
        }
    }
    for i in 0..3 {
        let handle = Handle::rotate_axis(i);
        let ring = ring(view, c, root.rotation * axis(i), RING_POINTS * unit, 64);
        // 輪の上で、ほかのハンドルに取られない点（軸のあいだの 45° から）。無ければ最初の点
        let at = [8usize, 24, 40, 56, 0, 16, 32, 48]
            .into_iter()
            .filter_map(|k| ring.get(k).copied())
            .find(|p| hit(s, root, view, *p) == handle)
            .or_else(|| ring.first().copied());
        if let Some(at) = at {
            list.push((handle, at));
        }
    }
    list
}

fn distance_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let l = ab.length_squared();
    let t = if l > 0.0 {
        ((p - a).dot(ab) / l).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

/// 画面の点の下のハンドル（なければ `None`）。重なった所は、中心の四角 → 大きさのつまみ → 矢印 → 輪の順（小さな的を先に）。
/// つまみ・矢印・輪は、それぞれの掴める近さの中で最も近いもの。
pub fn hit(s: &Shape, root: &Root, view: &CameraView, mouse: Vec2) -> Handle {
    let c = world_center(s, root);
    let unit = world_per_point(view, c);
    let Some(cg) = view.to_screen(c) else {
        return Handle::None;
    };
    if unit <= 0.0 {
        return Handle::None;
    }
    if (mouse.x - cg.x).abs() <= CENTER_POINTS / 2.0 + 2.0
        && (mouse.y - cg.y).abs() <= CENTER_POINTS / 2.0 + 2.0
    {
        return Handle::MoveFree;
    }
    let mut best = Handle::None;
    let mut best_distance = f32::MAX;
    for (handle, world) in visible_knobs(s, root, view) {
        if let Some(g) = view.to_screen(world) {
            let d = g.distance(mouse);
            if d <= KNOB_POINTS / 2.0 + 2.0 && d < best_distance {
                best = handle;
                best_distance = d;
            }
        }
    }
    if best != Handle::None {
        return best;
    }
    for i in 0..3 {
        let a = root.rotation * axis(i);
        if !axis_visible(view, c, a) {
            continue;
        }
        let Some(tip) = view.to_screen(c + a * ARROW_POINTS * unit) else {
            continue;
        };
        let d = distance_to_segment(mouse, cg, tip);
        if d <= GRAB_POINTS && d < best_distance {
            best = Handle::move_axis(i);
            best_distance = d;
        }
    }
    if best != Handle::None {
        return best;
    }
    for i in 0..3 {
        let ring = ring(view, c, root.rotation * axis(i), RING_POINTS * unit, 64);
        for w in ring.windows(2) {
            let d = distance_to_segment(mouse, w[0], w[1]);
            if d <= GRAB_POINTS && d < best_distance {
                best = Handle::rotate_axis(i);
                best_distance = d;
            }
        }
    }
    best
}

pub(crate) fn clamp_size(size: f64) -> f64 {
    size.clamp(MIN_SIZE, MAX_SIZE)
}

fn with_world_center(mut s: Shape, world: Vec3, root: &Root) -> Shape {
    let local = root.rotation.inverse() * (world - root.position);
    s.center = [local.x as f64, local.y as f64, local.z as f64];
    s
}

/// 直線（c を通り単位ベクトル a の向き）に沿って、ポインタが動いた量（押したときと今のレイが直線に最も近づく点の差）。
/// 直線がカメラのほぼ真正面を向くときは `None`。
pub(crate) fn axis_delta(view: &CameraView, c: Vec3, a: Vec3, from: Vec2, to: Vec2) -> Option<f32> {
    let s0 = line_parameter(ray_of(view, from), c, a)?;
    let s1 = line_parameter(ray_of(view, to), c, a)?;
    Some(s1 - s0)
}

fn line_parameter((origin, direction): (Vec3, Vec3), c: Vec3, a: Vec3) -> Option<f32> {
    let w = c - origin;
    let b = a.dot(direction);
    let denominator = 1.0 - b * b;
    if denominator < 1e-3 {
        return None;
    }
    Some((b * direction.dot(w) - a.dot(w)) / denominator)
}

/// 平面（点 c・法線 n）とレイの交点までの距離（手前向きに進むときだけ）。
pub(crate) fn ray_plane((origin, direction): (Vec3, Vec3), c: Vec3, n: Vec3) -> Option<f32> {
    let denominator = direction.dot(n);
    if denominator.abs() < 1e-9 {
        return None;
    }
    let t = (c - origin).dot(n) / denominator;
    (t > 0.0).then_some(t)
}

/// 輪を回した角度（度。軸 a のまわり）: 押したときと今のレイが輪の面に当たる点の角度の差。ほぼ真横から見た輪は、画面の接線に沿った動き。
fn ring_angle(view: &CameraView, c: Vec3, a: Vec3, radius: f32, from: Vec2, to: Vec2) -> f32 {
    let r0 = ray_of(view, from);
    let r1 = ray_of(view, to);
    if r0.1.dot(a).abs() > 0.2 {
        if let (Some(t0), Some(t1)) = (ray_plane(r0, c, a), ray_plane(r1, c, a)) {
            let v0 = r0.0 + r0.1 * t0 - c;
            let v1 = r1.0 + r1.1 * t1 - c;
            if v0.length_squared() > 1e-12 && v1.length_squared() > 1e-12 {
                return signed_angle(v0, v1, a);
            }
            return 0.0;
        }
    }
    // 横から見た輪: 押した所にいちばん近い輪の点の、画面の接線に沿って動いた量
    let ring = ring(view, c, a, radius, 64);
    if ring.len() < 2 || radius <= 0.0 {
        return 0.0;
    }
    let mut nearest = 0;
    let mut best = f32::MAX;
    for (k, p) in ring.iter().enumerate() {
        let d = p.distance(from);
        if d < best {
            best = d;
            nearest = k;
        }
    }
    let tangent = ring[(nearest + 1).min(ring.len() - 1)] - ring[nearest.saturating_sub(1)];
    if tangent.length_squared() < 1e-6 {
        return 0.0;
    }
    (to - from).dot(tangent.normalize()) / RING_POINTS * 180.0 / std::f32::consts::PI
}

/// v0 から v1 へ、a のまわりに回る角度（度。符号付き。`Quat::from_axis_angle(a, 角度) * v0` が v1 の向きになる）。
fn signed_angle(v0: Vec3, v1: Vec3, a: Vec3) -> f32 {
    let (p0, p1) = (v0 - a * v0.dot(a), v1 - a * v1.dot(a));
    let angle = p0.angle_between(p1).to_degrees();
    if p0.cross(p1).dot(a) < 0.0 {
        -angle
    } else {
        angle
    }
}

/// ハンドルを押した点 `from` から `to` までドラッグしたあとの形（どちらも押した瞬間の画面の点）。`symmetric` は大きさのつまみで両側を
/// 動かす（Shift）。`snap` は輪を `SNAP_DEGREES` ずつ（Ctrl）。画面の向きで決まらないドラッグは形を変えない。
#[allow(clippy::too_many_arguments)]
pub fn drag(
    handle: Handle,
    start: &Shape,
    root: &Root,
    view: &CameraView,
    from: Vec2,
    to: Vec2,
    symmetric: bool,
    snap: bool,
    steps: Snap,
) -> Shape {
    if from == to {
        return *start; // 動いていない（f32 の丸めで値を揺らさない）
    }
    let c0 = world_center(start, root);
    let q0 = world_rotation(start, root);
    match handle {
        Handle::None => *start,
        Handle::MoveX | Handle::MoveY | Handle::MoveZ => {
            let i = match handle {
                Handle::MoveX => 0,
                Handle::MoveY => 1,
                _ => 2,
            };
            let a = root.rotation * axis(i);
            let Some(mut delta) = axis_delta(view, c0, a, from, to) else {
                return *start;
            };
            if snap {
                delta = snapped(delta, steps.movement[i]);
            }
            with_world_center(*start, c0 + a * delta, root)
        }
        Handle::MoveFree => {
            let r0 = ray_of(view, from);
            let r1 = ray_of(view, to);
            let n = -r0.1;
            let (Some(t0), Some(t1)) = (ray_plane(r0, c0, n), ray_plane(r1, c0, n)) else {
                return *start;
            };
            let mut delta = (r1.0 + r1.1 * t1) - (r0.0 + r0.1 * t0);
            if snap {
                let mut local = root.rotation.inverse() * delta;
                for k in 0..3 {
                    local[k] = snapped(local[k], steps.movement[k]);
                }
                delta = root.rotation * local;
            }
            with_world_center(*start, c0 + delta, root)
        }
        Handle::RotateX | Handle::RotateY | Handle::RotateZ => {
            let i = match handle {
                Handle::RotateX => 0,
                Handle::RotateY => 1,
                _ => 2,
            };
            let a = root.rotation * axis(i);
            let radius = RING_POINTS * world_per_point(view, c0);
            let mut angle = ring_angle(view, c0, a, radius, from, to);
            if snap {
                angle = snapped(angle, steps.rotation);
            }
            if angle == 0.0 {
                return *start;
            }
            let local =
                root.rotation.inverse() * (Quat::from_axis_angle(a, angle.to_radians()) * q0);
            let mut next = *start;
            next.rotation = euler_degrees(local);
            next
        }
        _ => {
            let Some((i, positive)) = handle.knob_of() else {
                return *start;
            };
            let sign = if positive { 1.0f32 } else { -1.0 };
            let b = q0 * axis(i);
            let Some(delta) = axis_delta(view, c0, b, from, to) else {
                return *start;
            };
            let outward = f64::from(sign * delta);
            match start.kind {
                Kind::Sphere => {
                    let mut change = 2.0 * outward;
                    if snap {
                        change = f64::from(snapped(change as f32, steps.size));
                    }
                    let d = clamp_size(start.size[0] + change);
                    let mut next = *start;
                    next.size[0] = d;
                    next
                }
                _ => {
                    let mut change = if symmetric { 2.0 * outward } else { outward };
                    if snap {
                        change = f64::from(snapped(change as f32, steps.size));
                    }
                    let mut size = start.size;
                    let value = clamp_size(size[i] + change);
                    let grown = value - size[i];
                    size[i] = value;
                    let mut sized = *start;
                    sized.size = size;
                    if symmetric {
                        return sized;
                    }
                    // 反対の面は動かない
                    with_world_center(sized, c0 + b * (sign * grown as f32 / 2.0), root)
                }
            }
        }
    }
}

/// 面 a（単位）に垂直で c を通る円（閉じる。カメラの後ろの点は除く）。画面の点。
pub fn ring(view: &CameraView, c: Vec3, a: Vec3, radius: f32, segments: usize) -> Vec<Vec2> {
    ring_world(c, a, radius, segments)
        .into_iter()
        .filter_map(|p| view.to_screen(p))
        .collect()
}

/// `ring` の世界の点（閉じる）。
fn ring_world(c: Vec3, a: Vec3, radius: f32, segments: usize) -> Vec<Vec3> {
    let u = a
        .cross(if a.y.abs() < 0.9 { Vec3::Y } else { Vec3::X })
        .normalize_or_zero();
    let w = a.cross(u);
    (0..=segments)
        .map(|k| {
            let t = k as f32 * std::f32::consts::TAU / segments as f32;
            c + (u * t.cos() + w * t.sin()) * radius
        })
        .collect()
}

/// 描く線: 形の外形（減衰があれば値が 1 になる内側の形も薄く）、平面の 2 つの境と 1 へ向かう矢印、回す輪（奥の半分は薄く）と移動の
/// 矢印（輪の上に重ねる。掴める所の強調つき）。
pub fn lines(s: &Shape, root: &Root, view: &CameraView, hover: Handle) -> Vec<Line> {
    lines_with(s, root, view, hover, true)
}

/// 形の外形の線だけ（回す輪・移動の矢印は無い。編集のモードの選んだ物の枠、G/R/S の途中）。
pub fn outline(s: &Shape, root: &Root, view: &CameraView) -> Vec<Line> {
    lines_with(s, root, view, Handle::None, false)
}

fn lines_with(
    s: &Shape,
    root: &Root,
    view: &CameraView,
    hover: Handle,
    handles: bool,
) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    let c = world_center(s, root);
    let q = world_rotation(s, root);
    let unit = world_per_point(view, c);
    let mut polyline = |world: &[Vec3], color: Color32, width: f32, out: &mut Vec<Line>| {
        // カメラの後ろの点で線を切る
        let mut part: Vec<Vec2> = Vec::new();
        for p in world {
            match view.to_screen(*p) {
                Some(g) => part.push(g),
                None => {
                    if part.len() > 1 {
                        out.push(Line {
                            points: std::mem::take(&mut part),
                            color,
                            width,
                            filled: false,
                        });
                    }
                    part.clear();
                }
            }
        }
        if part.len() > 1 {
            out.push(Line {
                points: part,
                color,
                width,
                filled: false,
            });
        }
    };
    let segment =
        |a: Vec3, b: Vec3, color: Color32, width: f32, out: &mut Vec<Line>, poly: Polyline| {
            let pts: Vec<Vec3> = (0..9).map(|k| a.lerp(b, k as f32 / 8.0)).collect();
            poly(&pts, color, width, out);
        };
    let half = s.size.map(|v| v as f32 / 2.0);
    match s.kind {
        // 外形は無い（定規の線は `rulers` が描く）
        Kind::Anchor => {}
        Kind::Box => {
            let mut draw_box = |h: Vec3, color: Color32, width: f32, out: &mut Vec<Line>| {
                for i in 0..3 {
                    let (j, k) = ((i + 1) % 3, (i + 2) % 3);
                    for sj in [-1.0f32, 1.0] {
                        for sk in [-1.0f32, 1.0] {
                            let o = axis(j) * h[j] * sj + axis(k) * h[k] * sk;
                            segment(
                                c + q * (o - axis(i) * h[i]),
                                c + q * (o + axis(i) * h[i]),
                                color,
                                width,
                                out,
                                &mut polyline,
                            );
                        }
                    }
                }
            };
            let h = Vec3::new(half[0], half[1], half[2]);
            draw_box(h, OUTLINE, 2.0, &mut out);
            let band = s.falloff as f32 * half[0].min(half[1].min(half[2]));
            if band > 0.0 && half[0] - band > 0.0 && half[1] - band > 0.0 && half[2] - band > 0.0 {
                draw_box(h - Vec3::splat(band), INNER, 1.0, &mut out);
            }
        }
        Kind::Sphere => {
            let circle = |a: Vec3, radius: f32, color: Color32, width: f32, out: &mut Vec<Line>| {
                let u = a
                    .cross(if a.y.abs() < 0.9 { Vec3::Y } else { Vec3::X })
                    .normalize_or_zero();
                let w = a.cross(u);
                let pts: Vec<Vec3> = (0..=64)
                    .map(|k| {
                        let t = k as f32 * std::f32::consts::TAU / 64.0;
                        c + (u * t.cos() + w * t.sin()) * radius
                    })
                    .collect();
                polyline(&pts, color, width, out);
            };
            for i in 0..3 {
                circle(
                    q * axis(i),
                    half[0],
                    OUTLINE,
                    if i == 1 { 2.0 } else { 1.5 },
                    &mut out,
                );
            }
            if let Some(cg) = view.to_screen(c) {
                let toward = ray_of(view, cg).1;
                circle(toward, half[0], OUTLINE, 2.0, &mut out); // 外形
                let inner = half[0] - s.falloff as f32 * half[0];
                if inner > 0.0 && s.falloff > 0.0 {
                    circle(toward, inner, INNER, 1.0, &mut out);
                }
            }
        }
        Kind::Plane => {
            let face = [
                Vec3::new(-half[0], 0.0, -half[2]),
                Vec3::new(half[0], 0.0, -half[2]),
                Vec3::new(half[0], 0.0, half[2]),
                Vec3::new(-half[0], 0.0, half[2]),
                Vec3::new(-half[0], 0.0, -half[2]),
            ];
            let mid: Vec<Vec3> = face.iter().map(|p| c + q * *p).collect();
            let back: Vec<Vec3> = face
                .iter()
                .map(|p| c + q * (*p - Vec3::Y * half[1]))
                .collect();
            let front: Vec<Vec3> = face
                .iter()
                .map(|p| c + q * (*p + Vec3::Y * half[1]))
                .collect();
            for k in 0..4 {
                segment(mid[k], mid[k + 1], OUTLINE, 2.0, &mut out, &mut polyline);
                segment(back[k], back[k + 1], INNER, 1.0, &mut out, &mut polyline);
                segment(front[k], front[k + 1], INNER, 1.0, &mut out, &mut polyline);
            }
            let normal = q * Vec3::Y;
            segment(
                c,
                c + normal * half[1],
                OUTLINE,
                2.0,
                &mut out,
                &mut polyline,
            );
            if unit > 0.0 {
                let side = q * Vec3::X * 6.0 * unit;
                let tip = c + normal * half[1];
                segment(
                    tip,
                    c + normal * half[1] * 0.8 + side,
                    OUTLINE,
                    2.0,
                    &mut out,
                    &mut polyline,
                );
                segment(
                    tip,
                    c + normal * half[1] * 0.8 - side,
                    OUTLINE,
                    2.0,
                    &mut out,
                    &mut polyline,
                );
            }
        }
    }
    if unit <= 0.0 || !handles {
        return out;
    }
    // 回す輪: 手前の半分と奥の半分（薄く細く）に分けて描く
    let eye = view.to_viewer(c).normalize_or_zero();
    for i in 0..3 {
        let hot = hover == Handle::rotate_axis(i);
        let color = if hot { HOVER } else { axis_color(i) };
        let world = ring_world(c, root.rotation * axis(i), RING_POINTS * unit, 64);
        let mut run: Vec<Vec3> = Vec::new();
        let mut run_front = true;
        let flush = |run: &mut Vec<Vec3>, front: bool, out: &mut Vec<Line>| {
            if run.len() > 1 {
                let (color, width) = if front {
                    (color, if hot { 4.5 } else { 3.0 })
                } else {
                    (color.gamma_multiply(0.4), if hot { 3.0 } else { 2.0 })
                };
                polyline(run, color, width, out);
            }
            run.clear();
        };
        for w in world.windows(2) {
            let front = ((w[0] + w[1]) * 0.5 - c).dot(eye) >= 0.0;
            if run.is_empty() {
                run.push(w[0]);
                run_front = front;
            } else if front != run_front {
                let last = *run.last().expect("空でない");
                flush(&mut run, run_front, &mut out);
                run.push(last);
                run_front = front;
            }
            run.push(w[1]);
        }
        flush(&mut run, run_front, &mut out);
    }
    // 移動の矢印（輪より内側。輪の上に重ねる）
    let to_camera = view
        .to_screen(c)
        .map(|g| -ray_of(view, g).1)
        .unwrap_or(Vec3::NEG_Z);
    for i in 0..3 {
        let a = root.rotation * axis(i);
        if !axis_visible(view, c, a) {
            continue;
        }
        let tip = c + a * ARROW_POINTS * unit;
        let hot = hover == Handle::move_axis(i);
        let color = if hot { HOVER } else { axis_color(i) };
        segment(
            c,
            tip - a * 12.0 * unit,
            color,
            if hot { 4.5 } else { 3.0 },
            &mut out,
            &mut polyline,
        );
        // 矢じり: 画面を向いた三角形（塗る）
        let side = a.cross(to_camera).normalize_or_zero() * 5.5 * unit;
        if let (Some(t0), Some(t1), Some(t2)) = (
            view.to_screen(tip),
            view.to_screen(tip - a * 15.0 * unit + side),
            view.to_screen(tip - a * 15.0 * unit - side),
        ) {
            out.push(Line {
                points: vec![t0, t1, t2],
                color,
                width: 0.0,
                filled: true,
            });
        }
    }
    out
}

/// ハンドルの色（つまみは軸の色）。
pub fn knob_color(handle: Handle, hover: Handle) -> Color32 {
    if hover == handle {
        return HOVER;
    }
    match handle.knob_of() {
        Some((i, _)) => axis_color(i),
        None => Color32::WHITE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::geometry::OrbitCamera;

    fn view() -> CameraView {
        OrbitCamera::default().view(800.0, 600.0)
    }

    fn cube() -> Shape {
        Shape {
            kind: Kind::Box,
            center: [0.0, 0.0, 0.0],
            rotation: [0.0; 3],
            size: [1.0; 3],
            falloff: 0.5,
        }
    }

    fn near(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() < eps
    }

    fn handle_at(s: &Shape, v: &CameraView, handle: Handle) -> Vec2 {
        handle_points(s, &Root::default(), v)
            .into_iter()
            .find(|(h, _)| *h == handle)
            .unwrap_or_else(|| panic!("{handle:?} が出ていない"))
            .1
    }

    #[test]
    fn euler_round_trips_through_the_unity_order() {
        for rot in [
            [0.0, 0.0, 0.0],
            [10.0, 20.0, 30.0],
            [-45.0, 170.0, -100.0],
            [80.0, -30.0, 5.0],
            [0.0, 90.0, 0.0],
        ] {
            let mut s = cube();
            s.rotation = rot;
            let q = rotation_of(&s);
            let back = euler_degrees(q);
            let mut s2 = s;
            s2.rotation = back;
            let q2 = rotation_of(&s2);
            assert!(same_rotation(q, q2), "{rot:?} → {back:?}");
        }
        // 真上を向く（x = ±90°）でも同じ向きになる
        let mut s = cube();
        s.rotation = [90.0, 40.0, 0.0];
        let q = rotation_of(&s);
        let mut s2 = s;
        s2.rotation = euler_degrees(q);
        assert!(same_rotation(q, rotation_of(&s2)));
    }

    /// 2 つの回転が同じか（四元数の内積。acos の丸めで小さな角度が揺れるので、角度では比べない）。
    fn same_rotation(a: Quat, b: Quat) -> bool {
        1.0 - a.dot(b).abs() < 1e-6
    }

    #[test]
    fn the_rotation_matches_core_placement_matrix() {
        // 投影の置き場と形のグラデーションの回転の式（Z → X → Y）と同じ向き
        let rot = [20.0, -35.0, 50.0];
        let mut s = cube();
        s.rotation = rot;
        let q = rotation_of(&s);
        let m = Mat3::from_quat(q);
        let p = Placement {
            rotation: rot,
            ..Placement::default()
        };
        let _ = p;
        // Volume::rotation_matrix は行優先 9 つ
        let v = Volume {
            rotation: rot,
            ..Volume::default()
        };
        let r = v.rotation_matrix();
        for row in 0..3 {
            for col in 0..3 {
                assert!(
                    (m.col(col)[row] as f64 - r[row * 3 + col]).abs() < 1e-5,
                    "{row},{col}"
                );
            }
        }
    }

    #[test]
    fn knobs_sit_on_the_faces_and_a_plane_has_two() {
        let mut s = cube();
        s.size = [2.0, 4.0, 6.0];
        s.center = [1.0, 0.0, 0.0];
        let k = knobs(&s, &Root::default());
        assert_eq!(k.len(), 6);
        let find = |h: Handle| k.iter().find(|(x, _)| *x == h).unwrap().1;
        assert!((find(Handle::SizeXPos) - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-6);
        assert!((find(Handle::SizeYNeg) - Vec3::new(1.0, -2.0, 0.0)).length() < 1e-6);
        assert!((find(Handle::SizeZPos) - Vec3::new(1.0, 0.0, 3.0)).length() < 1e-6);
        s.kind = Kind::Plane;
        assert_eq!(knobs(&s, &Root::default()).len(), 2);
        s.kind = Kind::Sphere;
        let k = knobs(&s, &Root::default());
        assert!(
            (k[0].1 - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-6,
            "球の半径は X の幅の半分"
        );
    }

    #[test]
    fn moving_along_an_axis_moves_the_centre_that_far_and_does_not_drift() {
        let v = view();
        let s = cube();
        let from = handle_at(&s, &v, Handle::MoveX);
        let root = Root::default();
        // 矢印の先を、ワールドの +X へ 0.5 の所の画面の点まで動かす
        let c = world_center(&s, &root);
        let unit = world_per_point(&v, c);
        let tip = c + Vec3::X * ARROW_POINTS * unit;
        let target = v.to_screen(tip + Vec3::X * 0.5).unwrap();
        let moved = drag(
            Handle::MoveX,
            &s,
            &root,
            &v,
            from,
            target,
            false,
            false,
            Snap::default(),
        );
        assert!(near(moved.center[0], 0.5, 1e-3), "{:?}", moved.center);
        assert!(near(moved.center[1], 0.0, 1e-4) && near(moved.center[2], 0.0, 1e-4));
        // 始まりから計算するので、途中を経由しても同じ
        let way = drag(
            Handle::MoveX,
            &s,
            &root,
            &v,
            from,
            from + Vec2::new(40.0, 3.0),
            false,
            false,
            Snap::default(),
        );
        assert_ne!(way.center, moved.center);
        let again = drag(
            Handle::MoveX,
            &s,
            &root,
            &v,
            from,
            target,
            false,
            false,
            Snap::default(),
        );
        assert_eq!(again, moved);
        // 刻み 1 なら 0.5 は 1 か 0 に丸まる（四捨五入）
        let snapped = drag(
            Handle::MoveX,
            &s,
            &root,
            &v,
            from,
            target,
            false,
            true,
            Snap::default(),
        );
        assert!(near(snapped.center[0], 1.0, 1e-6) || near(snapped.center[0], 0.0, 1e-6));
    }

    #[test]
    fn a_free_move_stays_in_the_plane_facing_the_camera() {
        let v = view();
        let s = cube();
        let root = Root::default();
        let from = handle_at(&s, &v, Handle::MoveFree);
        let moved = drag(
            Handle::MoveFree,
            &s,
            &root,
            &v,
            from,
            from + Vec2::new(60.0, 0.0),
            false,
            false,
            Snap::default(),
        );
        let delta = v3(moved.center) - v3(s.center);
        assert!(delta.length() > 0.01);
        assert!(
            delta.dot(v.forward).abs() < 1e-4,
            "カメラに向いた面の中で動く"
        );
    }

    #[test]
    fn a_size_knob_keeps_the_opposite_face_and_shift_grows_both() {
        let v = view();
        let s = cube();
        let root = Root::default();
        let from = handle_at(&s, &v, Handle::SizeXPos);
        let to = v.to_screen(Vec3::new(1.0, 0.0, 0.0)).unwrap(); // 面を +X 側へ 0.5 動かす
        let out = drag(
            Handle::SizeXPos,
            &s,
            &root,
            &v,
            from,
            to,
            false,
            false,
            Snap::default(),
        );
        assert!(out.size[0] > 1.4, "{:?}", out.size);
        let low = out.center[0] - out.size[0] / 2.0;
        assert!(near(low, -0.5, 1e-3), "反対の面（−X）は動かない: {low}");
        let both = drag(
            Handle::SizeXPos,
            &s,
            &root,
            &v,
            from,
            to,
            true,
            false,
            Snap::default(),
        );
        assert!(both.size[0] > out.size[0]);
        assert!(near(both.center[0], 0.0, 1e-9), "Shift は中心が動かない");
        // 小さくしても限界の下へは行かない
        let far = v.to_screen(Vec3::new(-3.0, 0.0, 0.0)).unwrap();
        let tiny = drag(
            Handle::SizeXPos,
            &s,
            &root,
            &v,
            from,
            far,
            true,
            false,
            Snap::default(),
        );
        assert!(tiny.size[0] >= MIN_SIZE);
    }

    #[test]
    fn a_sphere_knob_changes_only_the_radius() {
        let v = view();
        let mut s = cube();
        s.kind = Kind::Sphere;
        let root = Root::default();
        let from = handle_at(&s, &v, Handle::SizeXPos);
        let to = v.to_screen(Vec3::new(1.0, 0.0, 0.0)).unwrap();
        let out = drag(
            Handle::SizeXPos,
            &s,
            &root,
            &v,
            from,
            to,
            false,
            false,
            Snap::default(),
        );
        assert!(out.size[0] > 1.5);
        assert_eq!(out.center, s.center);
        assert_eq!(out.size[1], s.size[1]);
    }

    #[test]
    fn rotating_a_ring_turns_about_the_roots_axis_and_ctrl_snaps() {
        let v = view();
        let s = cube();
        let root = Root::default();
        // Y の輪（ほぼ水平な輪）の、カメラ側（手前）と反対側（奥）の点のどちらを掴んでも、輪に沿って動かした分だけ回る。
        // 手前の向きはカメラの位置から取る（既定のカメラがモデルのどちら側にあるかに依らない）
        let c = world_center(&s, &root);
        let unit = world_per_point(&v, c);
        let radius = RING_POINTS * unit;
        let toward = Vec3::new(v.position.x - c.x, 0.0, v.position.z - c.z).normalize();
        let angle = 30f32.to_radians();
        for (side, p0) in [("手前", c + toward * radius), ("奥", c - toward * radius)] {
            let p1 = c + Quat::from_axis_angle(Vec3::Y, angle) * (p0 - c);
            let (a, b) = (v.to_screen(p0).unwrap(), v.to_screen(p1).unwrap());
            let out = drag(
                Handle::RotateY,
                &s,
                &root,
                &v,
                a,
                b,
                false,
                false,
                Snap::default(),
            );
            let turned = rotation_of(&out);
            assert!(
                1.0 - turned.dot(Quat::from_axis_angle(Vec3::Y, angle)).abs() < 1e-3,
                "{side}: {:?}",
                out.rotation
            );
            assert!(out.rotation[1].abs() > 20.0, "{side}: {:?}", out.rotation);
            // Ctrl: 15° ずつ
            let snapped = drag(
                Handle::RotateY,
                &s,
                &root,
                &v,
                a,
                b,
                false,
                true,
                Snap::default(),
            );
            let k = snapped.rotation[1] / 15.0;
            assert!(near(k, k.round(), 1e-3), "{side}: {:?}", snapped.rotation);
            // 動かさなければ形は変わらない
            let same = drag(
                Handle::RotateY,
                &s,
                &root,
                &v,
                a,
                a,
                false,
                false,
                Snap::default(),
            );
            assert_eq!(same, s, "{side}");
        }
    }

    #[test]
    fn handles_facing_the_camera_are_not_shown_or_grabbed() {
        // 真正面から見る: Z の矢印と Z の面のつまみは中心に重なるので出さない
        let cam = OrbitCamera {
            yaw: 0.0,
            pitch: 0.0,
            ..OrbitCamera::default()
        };
        let v = cam.view(800.0, 600.0);
        let s = cube();
        let points = handle_points(&s, &Root::default(), &v);
        assert!(!points.iter().any(|(h, _)| *h == Handle::MoveZ));
        assert!(!points
            .iter()
            .any(|(h, _)| matches!(h, Handle::SizeZPos | Handle::SizeZNeg)));
        assert!(points.iter().any(|(h, _)| *h == Handle::MoveX));
        // その向きのドラッグは何も変えない
        let out = drag(
            Handle::MoveZ,
            &s,
            &Root::default(),
            &v,
            Vec2::new(400.0, 300.0),
            Vec2::new(430.0, 300.0),
            false,
            false,
            Snap::default(),
        );
        assert_eq!(out, s);
    }

    #[test]
    fn one_gizmo_shows_the_arrows_the_centre_and_the_rings_together() {
        let v = view();
        let s = cube();
        let root = Root::default();
        let points = handle_points(&s, &root, &v);
        for h in [
            Handle::MoveFree,
            Handle::MoveX,
            Handle::MoveY,
            Handle::MoveZ,
            Handle::RotateX,
            Handle::RotateY,
            Handle::RotateZ,
            Handle::SizeXPos,
        ] {
            assert!(points.iter().any(|(x, _)| *x == h), "{h:?} が出ていない");
        }
        // どのハンドルも、出した点を押せばそのハンドルが取れる（輪の点はほかに取られない所を選ぶ）
        for (handle, at) in &points {
            assert_eq!(hit(&s, &root, &v, *at), *handle, "{handle:?}");
        }
        // 輪は矢印の外側
        let c = v.to_screen(world_center(&s, &root)).unwrap();
        let tip = handle_at(&s, &v, Handle::MoveX);
        for h in [Handle::RotateX, Handle::RotateY, Handle::RotateZ] {
            let at = handle_at(&s, &v, h);
            assert!(at.distance(c) > tip.distance(c) * 0.5, "{h:?}");
        }
        // 何も無い所
        assert_eq!(hit(&s, &root, &v, Vec2::new(5.0, 5.0)), Handle::None);
    }

    #[test]
    fn hit_prefers_the_centre_then_the_arrows_then_the_rings() {
        let v = view();
        let s = cube();
        let root = Root::default();
        let c = v.to_screen(world_center(&s, &root)).unwrap();
        // 中心は矢印の根もとと重なるが、中心の四角が先
        assert_eq!(hit(&s, &root, &v, c), Handle::MoveFree);
        assert_eq!(
            hit(&s, &root, &v, c + Vec2::new(CENTER_POINTS / 2.0, 0.0)),
            Handle::MoveFree
        );
        // 矢印の先は、その近くを通る輪より矢印が先
        let tip = handle_at(&s, &v, Handle::MoveX);
        assert_eq!(hit(&s, &root, &v, tip), Handle::MoveX);
        // 矢印と輪が画面で交わる所: 矢印が先（矢印の線の上で、輪にも掴める近さの点）
        let c3 = world_center(&s, &root);
        let radius = RING_POINTS * world_per_point(&v, c3);
        let mut crossings = 0;
        for (arrow, i) in [(Handle::MoveX, 0), (Handle::MoveY, 1), (Handle::MoveZ, 2)] {
            let Some((_, tip)) = handle_points(&s, &root, &v)
                .into_iter()
                .find(|(h, _)| *h == arrow)
            else {
                continue;
            };
            // 矢印は自分の軸の輪の面に垂直で、ほかの 2 つの輪の面の中（その輪の内側）にある。交わりうるのは自分の軸の輪だけ
            let rings = ring(&v, c3, axis(i), radius, 256);
            for k in 1..=40 {
                let p = c + (tip - c) * (k as f32 / 40.0);
                if p.distance(c) <= CENTER_POINTS || !rings.iter().any(|r| r.distance(p) <= 2.0) {
                    continue;
                }
                crossings += 1;
                let got = hit(&s, &root, &v, p);
                assert!(
                    got == arrow || got.is_size(),
                    "{arrow:?} の上の点 {k} で {got:?}"
                );
            }
        }
        assert!(crossings > 0, "矢印と輪の交わる所が無い（カメラを替える）");
    }

    #[test]
    fn lines_draw_the_outline_rings_and_arrows_together() {
        let v = view();
        let s = cube();
        let root = Root::default();
        let all = lines(&s, &root, &v, Handle::None);
        let outline = all.iter().filter(|l| l.color == OUTLINE).count();
        assert_eq!(outline, 12, "箱の辺");
        assert_eq!(
            all.iter().filter(|l| l.color == INNER).count(),
            12,
            "減衰の内側の箱"
        );
        assert!(all.iter().any(|l| l.filled), "矢じり");
        for color in [AXIS_X, AXIS_Y, AXIS_Z] {
            assert!(all.iter().any(|l| l.color == color), "軸の色の矢印");
            assert!(
                all.iter().any(|l| l.color == color.gamma_multiply(0.4)),
                "輪の奥の半分は薄く"
            );
        }
        let hot = lines(&s, &root, &v, Handle::RotateX);
        assert!(
            hot.iter().any(|l| l.color == HOVER && !l.filled),
            "掴める輪は強調"
        );
        let hot = lines(&s, &root, &v, Handle::MoveY);
        assert!(
            hot.iter().any(|l| l.color == HOVER && l.filled),
            "掴める矢印は強調"
        );
        let mut plane = s;
        plane.kind = Kind::Plane;
        assert!(!lines(&plane, &root, &v, Handle::None).is_empty());
        let mut sphere = s;
        sphere.kind = Kind::Sphere;
        assert!(lines(&sphere, &root, &v, Handle::None).len() >= 4);
    }

    #[test]
    fn a_root_pose_moves_the_shape_with_it() {
        let root = Root {
            position: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        };
        let mut s = cube();
        s.center = [1.0, 0.0, 0.0];
        let c = world_center(&s, &root);
        // ルートが +Y のまわりに 90° 回っていれば、ルートの +X は世界の −Z
        assert!((c - Vec3::new(1.0, 2.0, 2.0)).length() < 1e-5, "{c:?}");
        let v = view();
        let from = Vec2::new(400.0, 300.0);
        let out = drag(
            Handle::MoveFree,
            &s,
            &root,
            &v,
            from,
            from,
            false,
            false,
            Snap::default(),
        );
        assert_eq!(out, s);
    }
}
