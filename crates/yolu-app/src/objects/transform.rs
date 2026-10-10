//! G/R/S（移動・回転・拡縮）の途中。編集のモードは選んだ物、ポーズのモードは選んでいるボーンに当てる。
//!
//! - キー（G・R・S）を押すと始まり、マウスを動かした量（画面の上の量）で変える。X/Y/Z で軸（同じ軸をもう一度押すと物の向きの軸、
//!   3 回で軸なし）、Shift＋X/Y/Z でその軸を除いた面。数字・`-`（符号）・`.`・Backspace で値を打つと、マウスより値が先（軸が無ければ X）。
//!   左クリックか Enter で決め、右クリックか Esc でやめる（始める前の形へ戻し、履歴に残さない）。フォーカスを失ったら、そこまでで決める。
//! - Ctrl を押している間はスナップ（移動・回転・拡縮の刻み `Steps`）。Shift+Tab でスナップを常にする・しない（そのときの Ctrl は逆）。
//! - 開いている間は `PopupKind::Transform` のポップアップとして、画面全体の受け皿で下の部品の押しとキーの表を止める（パイと同じ）。
//!   マウスのボタンを押している間・ドラッグの途中は始めない。
//! - 拡縮の軸は物の向きの軸（大きさの X・Y・Z）。移動と回転の軸は、1 回目が世界の軸、2 回目が物の向きの軸。点とパスは向きを持たない。
//! - 当て方: 形（箱・デカール・グラデーションデカール・フィルターの形）・点・3D の定規は文書へ続けて変える操作として入れ（1 回の取り消し、途中は
//!   粗い表示）、決めたらまとめを終え、やめたらまとめを捨てる。定規の移動は a・b を動かし、回転は中心のまわりに a・b と向きを回し、拡縮は中心から a・b の
//!   距離を変える（定規の向きの枠は X = a→b、Y = 定規の向き、Z = X × Y。2 回目の軸の固定はこの枠の軸）。3D パスは途中は線を重ねて見せるだけで、決めたときに全部の点を最も近い
//!   面の点へ当て直して 1 回の取り消しで入れる（面に乗らない点があれば決めるのを断り、続けられる）。ボーンはポーズの続けて変える操作
//!   （ポーズの取り消し 1 段）。

use egui::{pos2, vec2, Color32, Event, Id, Key, Modifiers, Order, Pos2, Rect, Sense, Stroke};
use yolu_core::glam::{Mat3, Mat4, Quat, Vec2, Vec3};
use yolu_core::paths::{point_of, rebind_tolerance, SurfacePath, Tangent, REBIND_MAX_NODE_VISITS};
use yolu_core::skin::{Pose, Rig};
use yolu_core::LayerPath;

use super::{marker_of, polyline_middle, shape_of, Object};
use crate::fillfx::gizmo::{self, Target};
use crate::lang::Lang;
use crate::mode::EditorMode;
use crate::notice::Source;
use crate::state::{AppState, OpenPopup, PopupKind};
use crate::ui::menu::PopupState;
use crate::ui::theme;
use crate::ui::widgets::{self as w, Align};
use crate::view3d::pose;
use crate::view3d::shape_gizmo as sg;

/// G/R/S の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Grab,
    Rotate,
    Scale,
}

impl Kind {
    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Kind::Grab => lang.pick("移動", "Move"),
            Kind::Rotate => lang.pick("回転", "Rotate"),
            Kind::Scale => lang.pick("拡縮", "Scale"),
        }
    }
}

/// 軸の決め方。`local` は物の向きの軸。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constraint {
    Free,
    Axis {
        axis: usize,
        local: bool,
    },
    /// この軸を除いた面。
    Plane {
        axis: usize,
        local: bool,
    },
}

/// G/R/S を当てるもの。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum What {
    Object(Object),
    Bone(usize),
}

/// 始めたときの形（毎フレーム、ここから計算する。ずれが溜まらない）。
#[derive(Clone, Debug)]
enum Start {
    Shape(sg::Shape),
    Point([f64; 3]),
    /// 3D の定規（始めの値。毎フレーム、ここから量を当てる）。
    Ruler(yolu_core::Ruler),
    /// 3D パス: 点の世界の位置と、その面の法線（決めるとき、動きに合わせて回した法線の向きの面だけへ当て直す）。
    Path {
        path: Box<SurfacePath>,
        world: Vec<Vec3>,
        normals: Vec<Vec3>,
    },
    Bone {
        pose: Pose,
        world: Vec<Mat4>,
    },
}

/// 今の量。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    /// 世界の移動量。
    Move(Vec3),
    /// 世界の軸のまわりの角度（度）。
    Turn { axis: Vec3, degrees: f32 },
    /// 物の向きの軸ごとの倍率。
    Factor(Vec3),
}

/// G/R/S の途中。
#[derive(Clone, Debug)]
pub struct Transform {
    pub kind: Kind,
    pub what: What,
    start: Start,
    /// 回す・拡縮する中心（世界）。
    pub pivot: Vec3,
    /// 物の向き（2 回目の軸・拡縮の軸）。
    orientation: Quat,
    /// 向きを持つ物か（形・ボーン）。拡縮の軸は、向きを持つ物では 1 回目から物の向きの軸。
    oriented: bool,
    pub constraint: Constraint,
    /// 打った値。
    pub typed: String,
    /// 始めたときのポインタと、前のフレームのポインタ（画面の点）。
    from: Pos2,
    last: Pos2,
    /// ポインタで回した角度の合計（度。画面で反時計回りが正）。
    angle: f32,
    doc_id: u128,
    /// 始めたフレーム（そのフレームの押しは数えない）。
    pub(crate) opened_frame: u64,
    pub value: Value,
    /// ツールの帯の移動・回転・拡縮のドラッグで始めた（離して決める）。押した入力（マウスかペン）。
    pub drag: Option<gizmo::Source>,
}

/// 1 フレームの入力（egui の入力から作る。試験は直に作る）。
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub pointer: Option<Pos2>,
    pub ctrl: bool,
    /// 押したキー（繰り返しを除く）と、その時の修飾。
    pub keys: Vec<(Key, Modifiers)>,
    /// 左・右ボタンを押した（始めたフレームの押しは除く）。
    pub primary: bool,
    pub secondary: bool,
    /// 左ボタン（ドラッグで始めたときは、その入力。ペンは離した）を離した。
    pub released: bool,
    pub focus_lost: bool,
}

/// 1 フレームの結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Open,
    Done,
}

fn axis_vector(i: usize) -> Vec3 {
    [Vec3::X, Vec3::Y, Vec3::Z][i]
}

fn axis_letter(i: usize) -> &'static str {
    ["X", "Y", "Z"][i]
}

fn axis_color(i: usize) -> Color32 {
    [sg::AXIS_X, sg::AXIS_Y, sg::AXIS_Z][i]
}

fn snapped(v: f32, step: f32) -> f32 {
    if step > 0.0 && step.is_finite() {
        (v / step).round() * step
    } else {
        v
    }
}

fn wrap_degrees(d: f32) -> f32 {
    let mut d = d % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d <= -180.0 {
        d += 360.0;
    }
    d
}

/// 移動・回転・拡縮で 1 点を動かす（`orientation` は拡縮の軸の向き）。
pub fn map_point(p: Vec3, pivot: Vec3, value: Value, orientation: Quat) -> Vec3 {
    match value {
        Value::Move(d) => p + d,
        _ => pivot + map_vector(p - pivot, value, orientation),
    }
}

/// 移動・回転・拡縮で向き（パスの取っ手）を変える（移動は変えない）。
pub fn map_vector(v: Vec3, value: Value, orientation: Quat) -> Vec3 {
    match value {
        Value::Move(_) => v,
        Value::Turn { axis, degrees } => {
            Quat::from_axis_angle(axis.normalize_or_zero(), degrees.to_radians()) * v
        }
        Value::Factor(f) => orientation * (f * (orientation.inverse() * v)),
    }
}

/// 移動・回転・拡縮で面の法線を変える（拡縮は逆の倍率で。向きだけを返す）。
pub fn map_normal(n: Vec3, value: Value, orientation: Quat) -> Vec3 {
    match value {
        Value::Move(_) => n,
        Value::Turn { .. } => map_vector(n, value, orientation),
        Value::Factor(f) => orientation * ((orientation.inverse() * n) / f),
    }
    .normalize_or_zero()
}

/// 形に量を当てる（ルートは原点で回転も無いので、世界の量がそのまま形の空間の量）。
pub fn shape_after(start: &sg::Shape, value: Value) -> sg::Shape {
    let mut s = *start;
    match value {
        Value::Move(d) => {
            for (k, c) in s.center.iter_mut().enumerate() {
                *c = start.center[k] + f64::from(d[k]);
            }
        }
        Value::Turn { axis, degrees } => {
            let q = Quat::from_axis_angle(axis.normalize_or_zero(), degrees.to_radians())
                * sg::rotation_of(start);
            s.rotation = sg::euler_degrees(q);
        }
        Value::Factor(f) => {
            if start.kind == sg::Kind::Sphere {
                // 球は大きさが 1 つ: 軸を決めていればその倍率、無ければ一様の倍率
                let k = [f.x, f.y, f.z]
                    .into_iter()
                    .find(|v| (*v - 1.0).abs() > f32::EPSILON)
                    .unwrap_or(1.0);
                s.size[0] = sg::clamp_size(start.size[0] * f64::from(k));
            } else {
                for (i, size) in s.size.iter_mut().enumerate() {
                    *size = sg::clamp_size(start.size[i] * f64::from(f[i]));
                }
            }
        }
    }
    s
}

/// ボーンに量を当てたポーズ（`world` は始めたときのポーズの世界の行列）。
pub fn pose_after(rig: &Rig, start: &Pose, world: &[Mat4], bone: usize, value: Value) -> Pose {
    let mut p = start.clone();
    if bone >= p.locals.len() {
        return p;
    }
    match value {
        Value::Move(d) => {
            let parent = rig.bones()[bone]
                .parent
                .and_then(|i| world.get(i as usize))
                .map_or(Mat3::IDENTITY, |m| Mat3::from_mat4(*m));
            let local = parent.inverse() * d;
            if local.is_finite() {
                p.locals[bone].translation = start.locals[bone].translation + local;
            }
        }
        Value::Turn { axis, degrees } => {
            rig.rotate_bone_world(&mut p, world, bone, axis, degrees.to_radians());
        }
        Value::Factor(f) => p.locals[bone].scale = start.locals[bone].scale * f,
    }
    p
}

fn map_tangent(t: Tangent<Vec3>, value: Value, orientation: Quat) -> Tangent<Vec3> {
    match t {
        Tangent::Handles { incoming, outgoing } => Tangent::Handles {
            incoming: map_vector(incoming, value, orientation),
            outgoing: map_vector(outgoing, value, orientation),
        },
        other => other,
    }
}

impl Transform {
    /// 線を引く軸の向き（世界の軸か、物の向きの軸）。
    pub fn axis_direction(&self, axis: usize, local: bool) -> Vec3 {
        self.direction(axis, local)
    }

    /// 世界の軸か、物の向きの軸。
    fn direction(&self, axis: usize, local: bool) -> Vec3 {
        if local {
            self.orientation * axis_vector(axis)
        } else {
            axis_vector(axis)
        }
    }

    /// 打った値（打っていなければ None。符号・小数点だけなら 0）。
    pub fn typed_value(&self) -> Option<f32> {
        if self.typed.is_empty() {
            return None;
        }
        Some(self.typed.parse::<f32>().unwrap_or(0.0)).filter(|v| v.is_finite())
    }

    /// X/Y/Z（`plane` なら Shift）: 同じ軸を押すたびに、世界の軸 → 物の向きの軸 → 軸なし。向きを持つ物の拡縮は、物の向きの軸 →
    /// 軸なし（大きさは物の向きの軸ごとにしか変えられないので、世界の軸は持たない）。
    fn toggle(&mut self, axis: usize, plane: bool) {
        if self.kind == Kind::Scale {
            let local = self.oriented;
            self.constraint = match (self.constraint, plane) {
                (Constraint::Axis { axis: a, .. }, false) if a == axis => Constraint::Free,
                (Constraint::Plane { axis: a, .. }, true) if a == axis => Constraint::Free,
                (_, false) => Constraint::Axis { axis, local },
                (_, true) => Constraint::Plane { axis, local },
            };
            return;
        }
        self.constraint = match (self.constraint, plane) {
            (
                Constraint::Axis {
                    axis: a,
                    local: false,
                },
                false,
            ) if a == axis => Constraint::Axis { axis, local: true },
            (
                Constraint::Axis {
                    axis: a,
                    local: true,
                },
                false,
            ) if a == axis => Constraint::Free,
            (
                Constraint::Plane {
                    axis: a,
                    local: false,
                },
                true,
            ) if a == axis => Constraint::Plane { axis, local: true },
            (
                Constraint::Plane {
                    axis: a,
                    local: true,
                },
                true,
            ) if a == axis => Constraint::Free,
            (_, false) => Constraint::Axis { axis, local: false },
            (_, true) => Constraint::Plane { axis, local: false },
        };
    }

    fn type_key(&mut self, key: Key) {
        let digit = match key {
            Key::Num0 => Some('0'),
            Key::Num1 => Some('1'),
            Key::Num2 => Some('2'),
            Key::Num3 => Some('3'),
            Key::Num4 => Some('4'),
            Key::Num5 => Some('5'),
            Key::Num6 => Some('6'),
            Key::Num7 => Some('7'),
            Key::Num8 => Some('8'),
            Key::Num9 => Some('9'),
            _ => None,
        };
        if let Some(c) = digit {
            if self.typed.len() < 16 {
                self.typed.push(c);
            }
            return;
        }
        match key {
            Key::Period if !self.typed.contains('.') => {
                if self.typed.is_empty() || self.typed == "-" {
                    self.typed.push('0');
                }
                self.typed.push('.');
            }
            Key::Minus => {
                if let Some(rest) = self.typed.strip_prefix('-') {
                    self.typed = rest.to_owned();
                } else {
                    self.typed.insert(0, '-');
                }
            }
            Key::Backspace => {
                self.typed.pop();
            }
            _ => {}
        }
    }

    /// ポインタの動き（回転の角度を、前の位置からの角度の差で足していく）。
    fn follow(&mut self, pointer: Pos2, center: Pos2) {
        if self.kind == Kind::Rotate {
            let (a, b) = (self.last - center, pointer - center);
            if a.length() > 2.0 && b.length() > 2.0 {
                let angle = |v: egui::Vec2| (-v.y).atan2(v.x).to_degrees();
                self.angle += wrap_degrees(angle(b) - angle(a));
            }
        }
        self.last = pointer;
    }

    /// 今の量（ポインタか打った値から。`snap` で刻みに合わせる）。
    fn compute(&self, app: &AppState, rect: Rect, snap: bool) -> Value {
        let view = app.view3d.camera.view(rect.width(), rect.height());
        let local = |p: Pos2| Vec2::new(p.x - rect.left(), p.y - rect.top());
        let (from, now) = (local(self.from), local(self.last));
        let steps = app.objects.steps;
        let typed = self.typed_value();
        let toward_camera = -view.forward;
        match self.kind {
            Kind::Grab => {
                let delta =
                    if let Some(v) = typed {
                        let dir = match self.constraint {
                            Constraint::Free => Vec3::X,
                            Constraint::Axis { axis, local } => self.direction(axis, local),
                            Constraint::Plane { axis, local } => {
                                self.direction((axis + 1) % 3, local)
                            }
                        };
                        dir * v
                    } else {
                        let plane = |n: Vec3| -> Option<Vec3> {
                            let r0 = sg::ray_of(&view, from);
                            let r1 = sg::ray_of(&view, now);
                            let t0 = sg::ray_plane(r0, self.pivot, n)?;
                            let t1 = sg::ray_plane(r1, self.pivot, n)?;
                            let d = (r1.0 + r1.1 * t1) - (r0.0 + r0.1 * t0);
                            Some(d - n * d.dot(n))
                        };
                        let (d, frame) =
                            match self.constraint {
                                Constraint::Free => (plane(view.forward), Quat::IDENTITY),
                                Constraint::Axis { axis, local } => {
                                    let a = self.direction(axis, local);
                                    let t = sg::axis_delta(&view, self.pivot, a, from, now)
                                        .map(|t| if snap { snapped(t, steps.movement) } else { t });
                                    return Value::Move(t.map_or(Vec3::ZERO, |t| a * t));
                                }
                                Constraint::Plane { axis, local } => (
                                    plane(self.direction(axis, local)),
                                    if local {
                                        self.orientation
                                    } else {
                                        Quat::IDENTITY
                                    },
                                ),
                            };
                        let d = d.unwrap_or(Vec3::ZERO);
                        if snap {
                            let l = frame.inverse() * d;
                            frame
                                * Vec3::new(
                                    snapped(l.x, steps.movement),
                                    snapped(l.y, steps.movement),
                                    snapped(l.z, steps.movement),
                                )
                        } else {
                            d
                        }
                    };
                Value::Move(delta)
            }
            Kind::Rotate => {
                let (axis, sign) = match self.constraint {
                    Constraint::Free => (toward_camera, 1.0),
                    Constraint::Axis { axis, local } | Constraint::Plane { axis, local } => {
                        let a = self.direction(axis, local);
                        (
                            a,
                            if a.dot(toward_camera) >= 0.0 {
                                1.0
                            } else {
                                -1.0
                            },
                        )
                    }
                };
                let degrees = match typed {
                    Some(v) => v,
                    None => {
                        let d = self.angle * sign;
                        if snap {
                            snapped(d, steps.rotation)
                        } else {
                            d
                        }
                    }
                };
                Value::Turn { axis, degrees }
            }
            Kind::Scale => {
                // 打った値が「-」だけ・0・負の間は、形を潰さず前の量のまま（負の大きさは形もボーンも持たない）
                if typed.is_some_and(|v| v <= 0.0) {
                    return self.value;
                }
                let f = typed.unwrap_or_else(|| {
                    let center = view
                        .to_screen(self.pivot)
                        .map_or(rect.center(), |s| pos2(rect.left() + s.x, rect.top() + s.y));
                    let (d0, d1) = (self.from.distance(center), self.last.distance(center));
                    let f = if d0 > 4.0 {
                        d1 / d0
                    } else {
                        1.0 + (self.last.x - self.from.x) / 100.0
                    };
                    if snap {
                        snapped(f, steps.scale)
                    } else {
                        f
                    }
                });
                let f = f.clamp(1e-3, 1e3);
                let v = match self.constraint {
                    Constraint::Free => Vec3::splat(f),
                    Constraint::Axis { axis, .. } => {
                        let mut v = Vec3::ONE;
                        v[axis] = f;
                        v
                    }
                    Constraint::Plane { axis, .. } => {
                        let mut v = Vec3::splat(f);
                        v[axis] = 1.0;
                        v
                    }
                };
                Value::Factor(v)
            }
        }
    }

    /// 状態の札の文（「移動 X 0.250」）。
    pub fn status(&self, lang: Lang) -> String {
        let axis = match self.constraint {
            Constraint::Free => String::new(),
            Constraint::Axis { axis, local } => {
                let a = axis_letter(axis);
                if local {
                    lang.pick(format!(" ローカル {a}"), format!(" Local {a}"))
                } else {
                    format!(" {a}")
                }
            }
            Constraint::Plane { axis, local } => {
                let p = format!(
                    "{}{}",
                    axis_letter((axis + 1) % 3).min(axis_letter((axis + 2) % 3)),
                    axis_letter((axis + 1) % 3).max(axis_letter((axis + 2) % 3))
                );
                if local {
                    lang.pick(format!(" ローカル {p}"), format!(" Local {p}"))
                } else {
                    format!(" {p}")
                }
            }
        };
        let amount = if !self.typed.is_empty() {
            self.typed.clone()
        } else {
            match self.value {
                Value::Move(d) => match self.constraint {
                    Constraint::Axis { axis, local } => {
                        format!("{:.3}", d.dot(self.direction(axis, local)))
                    }
                    _ => format!("{:.3}", d.length()),
                },
                Value::Turn { degrees, .. } => format!("{degrees:.1}°"),
                Value::Factor(f) => {
                    let k = match self.constraint {
                        Constraint::Axis { axis, .. } => f[axis],
                        Constraint::Plane { axis, .. } => f[(axis + 1) % 3],
                        Constraint::Free => f.x,
                    };
                    format!("{k:.3}")
                }
            }
        };
        format!("{}{axis} {amount}", self.kind.name(lang))
    }

    /// 3D パスの、今の量で動かした点（世界。重ね描き用）。
    pub fn path_preview(&self) -> Option<Vec<Vec3>> {
        let Start::Path { world, .. } = &self.start else {
            return None;
        };
        Some(
            world
                .iter()
                .map(|p| map_point(*p, self.pivot, self.value, self.orientation))
                .collect(),
        )
    }

    /// 物がまだあるか（文書・モデル・セッションが替わっていないか）。
    fn still_valid(&self, app: &AppState) -> bool {
        match self.what {
            What::Object(o) => {
                app.doc.id() == self.doc_id
                    && app.mode == EditorMode::Edit
                    && super::selectable(app, o)
            }
            What::Bone(b) => {
                app.mode == EditorMode::Pose
                    && app
                        .view3d
                        .pose
                        .session
                        .as_ref()
                        .is_some_and(|s| b < s.rig.bones().len())
            }
        }
    }

    /// 今の量を当てる（形と点は文書へ続けて変える操作として、ボーンはポーズの続けて変える操作として。パスは重ね描きだけ）。
    fn apply(&self, app: &mut AppState) {
        match (&self.start, self.what) {
            (Start::Shape(start), What::Object(Object::Shape(target))) => {
                let next = shape_after(start, self.value);
                if shape_of(app, target) == Some(next) {
                    return;
                }
                if let Some(Err(e)) = gizmo::write_shape(app, target, next, true) {
                    app.notify(
                        crate::notice::Kind::of_core(&e),
                        Source::FillLayer,
                        app.lang.core_error(&e),
                    );
                }
            }
            (Start::Ruler(start), What::Object(Object::Ruler { layer, id })) => {
                let next = crate::rulers::edit3d::ruler_after(
                    start,
                    self.pivot,
                    self.value,
                    self.orientation,
                );
                let Some(current) = super::ruler_of(app, layer, id) else {
                    return;
                };
                if next == current || next.validate().is_err() {
                    return;
                }
                if let Err(e) = app.ruler_put(layer, next, true) {
                    app.notify(
                        crate::notice::Kind::of_core(&e),
                        Source::Ruler,
                        app.lang.core_error(&e),
                    );
                }
            }
            (
                Start::Point(start),
                What::Object(Object::Point {
                    layer,
                    channel,
                    index,
                }),
            ) => {
                let Value::Move(d) = self.value else {
                    return;
                };
                let Some(mut g) = app
                    .doc
                    .layer(layer)
                    .and_then(|l| l.fill_points(channel))
                    .cloned()
                else {
                    return;
                };
                let Some(p) = g.points.get_mut(index) else {
                    return;
                };
                let next = [0, 1, 2].map(|k| start[k] + f64::from(d[k]));
                if p.position == next {
                    return;
                }
                p.position = next;
                let revision = app.doc.revision();
                match app.doc.set_fill_points(layer, channel, Some(g), true) {
                    Ok(()) => {
                        if app.doc.revision() != revision {
                            app.modified = true;
                        }
                    }
                    Err(e) => app.notify(
                        crate::notice::Kind::of_core(&e),
                        Source::FillLayer,
                        app.lang.core_error(&e),
                    ),
                }
            }
            (Start::Bone { pose: start, world }, What::Bone(bone)) => {
                let Some(rig) = app.view3d.pose.session.as_ref().map(|s| s.rig.clone()) else {
                    return;
                };
                let next = pose_after(&rig, start, world, bone, self.value);
                if let Err(e) = pose::edit(&mut app.view3d, next) {
                    app.notify(e.notice_kind(), Source::Pose, app.lang.view_error(&e));
                }
            }
            _ => {}
        }
    }

    /// 3D パスの点を、動かした所の最も近い面の点へ当て直す（今のテクスチャセットの面だけ）。乗らない点があれば、その理由。
    fn projected_path(&self, app: &AppState) -> Result<Option<SurfacePath>, String> {
        let Start::Path {
            path,
            world,
            normals,
        } = &self.start
        else {
            return Ok(None);
        };
        let lang = app.lang;
        let what = lang.pick("パスを動かせません", "Cannot move the path");
        let model =
            app.view3d.full_model().cloned().ok_or_else(|| {
                lang.with_reason(what, lang.pick("モデルがありません", "no model"))
            })?;
        let tolerance = rebind_tolerance(&model.geometry, path);
        let mut points = Vec::with_capacity(path.points.len());
        for (i, ((p, at), normal)) in path.points.iter().zip(world).zip(normals).enumerate() {
            let target = map_point(*at, self.pivot, self.value, self.orientation);
            // 動きに合わせて回した法線の向きの面だけを探す（薄い板の表の点が、裏の面へ移らない。モデルの差し替えの当て直しと同じ）
            let facing = map_normal(*normal, self.value, self.orientation);
            let off = || {
                lang.with_reason(
                    what,
                    lang.pick(
                        format!("{} 番目の点が面に乗りません", i + 1),
                        format!("point {} is not on the surface", i + 1),
                    ),
                )
            };
            let hit = model
                .geometry
                .find_closest_point(
                    target,
                    tolerance,
                    facing,
                    REBIND_MAX_NODE_VISITS,
                    app.view3d.material,
                )
                .map_err(|_| off())?
                .ok_or_else(off)?;
            let point = point_of(&hit, p.pressure).map_err(|_| off())?;
            points.push(point.with_tangent(map_tangent(p.tangent, self.value, self.orientation)));
        }
        Ok(Some(SurfacePath {
            points,
            ..(**path).clone()
        }))
    }

    /// 決める。3D パスが面に乗らないときは断りの文を返す（続けられる）。
    fn confirm(&self, app: &mut AppState) -> Result<(), String> {
        match (&self.start, self.what) {
            (Start::Path { path, .. }, What::Object(Object::Path { layer, id })) => {
                let Some(moved) = self.projected_path(app)? else {
                    return Ok(());
                };
                if moved.points == path.points {
                    return Ok(());
                }
                let Some(mut entries) = app.doc.layer(layer).map(|l| l.paths().to_vec()) else {
                    return Ok(());
                };
                let Some(entry) = entries.iter_mut().find(|e| e.path.id() == id) else {
                    return Ok(());
                };
                entry.path = LayerPath::Surface(moved);
                app.doc.end_coalescing();
                // パスの一覧の口（点の選びを外し、面から外れて描かなかった標本を知らせる）
                app.path_commit_list(layer, entries, None);
                Ok(())
            }
            (_, What::Bone(_)) => {
                pose::end_edit(&mut app.view3d, true);
                Ok(())
            }
            _ => {
                if app.doc.id() == self.doc_id {
                    app.doc.end_coalescing();
                }
                Ok(())
            }
        }
    }

    /// やめる（始める前の形へ戻し、履歴に残さない）。
    fn cancel(&self, app: &mut AppState) {
        match self.what {
            What::Bone(_) => pose::end_edit(&mut app.view3d, false),
            What::Object(Object::Path { .. }) => {}
            What::Object(_) => {
                if app.doc.id() == self.doc_id {
                    let _ = app.doc.cancel_coalescing();
                }
            }
        }
    }
}

/// G/R/S を当てるもの（断るときは理由の文）。
fn target(app: &AppState, kind: Kind) -> Result<What, String> {
    let lang = app.lang;
    let what = lang.pick(
        format!("{}できません", kind.name(lang)),
        format!("Cannot {}", kind.name(lang).to_lowercase()),
    );
    let refuse = |reason: &str| lang.with_reason(&what, reason);
    if app.is_stroking() {
        return Err(crate::lang::refusals::during_stroke(lang).to_owned());
    }
    if !app.view3d.visible || app.view3d.view_rect.is_none() || app.view3d.model.is_none() {
        return Err(refuse(
            lang.pick("3D ビューが見えていません", "the 3D View is not shown"),
        ));
    }
    match app.mode {
        EditorMode::Paint => Err(refuse(lang.pick(
            "編集かポーズのモードで使います",
            "used in Edit or Pose mode",
        ))),
        EditorMode::Edit => {
            let o = super::selected(app).ok_or_else(|| refuse(super::nothing_selected(lang)))?;
            if matches!(o, Object::Point { .. }) && kind != Kind::Grab {
                return Err(refuse(
                    lang.pick("点は移動だけです", "points can only be moved"),
                ));
            }
            if let Some(reason) = app.read_only_reason() {
                return Err(crate::lang::refusals::read_only_set(lang, reason));
            }
            Ok(What::Object(o))
        }
        EditorMode::Pose => app
            .view3d
            .pose
            .session
            .as_ref()
            .and_then(|s| s.selected)
            .map(What::Bone)
            .ok_or_else(|| refuse(lang.pick("ボーンを選んでいません", "no bone is selected"))),
    }
}

/// G/R/S を始める頼み（キーの処理が、ポインタの所から始める）。描いている間・ドラッグの途中・当てるものが無いときは断る。
pub fn request(app: &mut AppState, kind: Kind) {
    if crate::pie::dragging(app) || super::transforming(app) {
        let lang = app.lang;
        let text = lang.with_reason(
            lang.pick(
                format!("{}できません", kind.name(lang)),
                format!("Cannot {}", kind.name(lang).to_lowercase()),
            ),
            lang.pick("ほかの操作の途中です", "another operation is in progress"),
        );
        app.refuse(Source::Edit, text);
        return;
    }
    match target(app, kind) {
        Ok(_) => app.objects.request = Some(kind),
        Err(text) => app.refuse(Source::Edit, text),
    }
}

/// 始める（`pointer` は今のポインタの画面の点）。断るときは理由の文。
pub fn begin(app: &mut AppState, kind: Kind, pointer: Pos2, frame: u64) -> Result<(), String> {
    begin_with(app, kind, pointer, frame, None)
}

/// 始める（`drag` はツールの帯のドラッグで始めたときの入力。離して決める）。
pub fn begin_with(
    app: &mut AppState,
    kind: Kind,
    pointer: Pos2,
    frame: u64,
    drag: Option<gizmo::Source>,
) -> Result<(), String> {
    let what = target(app, kind)?;
    let lang = app.lang;
    let (start, pivot, orientation) = match what {
        What::Object(Object::Shape(t)) => {
            let s = shape_of(app, t).ok_or_else(|| super::nothing_selected(lang).to_owned())?;
            (
                Start::Shape(s),
                sg::world_center(&s, &gizmo::root()),
                sg::world_rotation(&s, &gizmo::root()),
            )
        }
        What::Object(Object::Ruler { layer, id }) => {
            let r = super::ruler_of(app, layer, id)
                .ok_or_else(|| super::nothing_selected(lang).to_owned())?;
            let at = crate::rulers::edit3d::center(&r)
                .ok_or_else(|| super::nothing_selected(lang).to_owned())?
                .as_vec3();
            let frame = crate::rulers::edit3d::frame(&r)
                .ok_or_else(|| super::nothing_selected(lang).to_owned())?;
            (Start::Ruler(r), at, frame)
        }
        What::Object(o @ Object::Point { .. }) => {
            let at = marker_of(app, o).ok_or_else(|| super::nothing_selected(lang).to_owned())?;
            (
                Start::Point([at.x as f64, at.y as f64, at.z as f64]),
                at,
                Quat::IDENTITY,
            )
        }
        What::Object(Object::Path { layer, id }) => {
            let (world, normals): (Vec<Vec3>, Vec<Vec3>) =
                super::path_points_with_normals(app, layer, id)
                    .ok_or_else(|| super::nothing_selected(lang).to_owned())?
                    .into_iter()
                    .unzip();
            let path = app
                .doc
                .layer(layer)
                .and_then(|l| l.paths().iter().find(|e| e.path.id() == id))
                .and_then(|e| match &e.path {
                    LayerPath::Surface(p) => Some(p.clone()),
                    LayerPath::Canvas(_) => None,
                })
                .ok_or_else(|| super::nothing_selected(lang).to_owned())?;
            let pivot = polyline_middle(&world).unwrap_or(Vec3::ZERO);
            (
                Start::Path {
                    path: Box::new(path),
                    world,
                    normals,
                },
                pivot,
                Quat::IDENTITY,
            )
        }
        What::Bone(bone) => {
            let s = app
                .view3d
                .pose
                .session
                .as_ref()
                .ok_or_else(|| super::nothing_selected(lang).to_owned())?;
            let world = s.rig.world_matrices(s.pose()).map_err(|e| {
                let e: crate::view3d::model::ViewError = e.into();
                lang.view_error(&e)
            })?;
            let pivot = Rig::bone_origin(&world, bone);
            let orientation = Rig::world_rotation(&world, bone);
            let pose = s.pose().clone();
            pose::begin_edit(&mut app.view3d).map_err(|e| lang.view_error(&e))?;
            (Start::Bone { pose, world }, pivot, orientation)
        }
    };
    if matches!(what, What::Object(_)) {
        // 前の欄のドラッグにまとめない
        app.doc.end_coalescing();
    }
    let value = match kind {
        Kind::Grab => Value::Move(Vec3::ZERO),
        Kind::Rotate => Value::Turn {
            axis: Vec3::Z,
            degrees: 0.0,
        },
        Kind::Scale => Value::Factor(Vec3::ONE),
    };
    app.objects.pen_at = None;
    app.objects.pen_lifted = false;
    app.objects.transform = Some(Transform {
        kind,
        what,
        start,
        pivot,
        orientation,
        oriented: matches!(
            what,
            What::Object(Object::Shape(_) | Object::Ruler { .. }) | What::Bone(_)
        ),
        constraint: Constraint::Free,
        typed: String::new(),
        from: pointer,
        last: pointer,
        angle: 0.0,
        doc_id: app.doc.id(),
        opened_frame: frame,
        value,
        drag,
    });
    Ok(())
}

/// 1 フレームを進める（キー・ポインタ・ボタンを受けて量を決め、当てる。決める・やめるなら終える）。
pub fn step(app: &mut AppState, input: &Input) -> Step {
    let Some(mut t) = app.objects.transform.take() else {
        return Step::Done;
    };
    let Some(rect) = app.view3d.view_rect else {
        t.cancel(app);
        return Step::Done;
    };
    if !t.still_valid(app) {
        // 文書・物・モードが替わった: 何も当てずに終える（今の文書のまとめだけ捨てる）
        t.cancel(app);
        return Step::Done;
    }
    // キーで始めたときは押して決め、ドラッグで始めたときは離して決める
    let ends = if t.drag.is_some() {
        input.released
    } else {
        input.primary
    };
    let (mut confirm, mut cancel) = (ends || input.focus_lost, input.secondary);
    let typed_before = t.typed.clone();
    for (key, m) in &input.keys {
        match key {
            Key::Escape => cancel = true,
            Key::Enter => confirm = true,
            Key::Tab if m.shift => app.objects.snap = !app.objects.snap,
            Key::X => t.toggle(0, m.shift),
            Key::Y => t.toggle(1, m.shift),
            Key::Z => t.toggle(2, m.shift),
            k => t.type_key(*k),
        }
    }
    if cancel {
        t.cancel(app);
        return Step::Done;
    }
    if t.kind == Kind::Scale && t.typed != typed_before && t.typed_value().is_some_and(|v| v < 0.0)
    {
        // 負の倍率は形もボーンも持たない: 前の量のままにして理由を出す
        let lang = app.lang;
        let text = lang.with_reason(
            lang.pick("拡縮できません", "Cannot scale"),
            lang.pick("負の倍率は使えません", "a negative factor cannot be used"),
        );
        app.refuse(Source::Edit, text);
    }
    if let Some(p) = input.pointer {
        let view = app.view3d.camera.view(rect.width(), rect.height());
        let center = view
            .to_screen(t.pivot)
            .map_or(rect.center(), |s| pos2(rect.left() + s.x, rect.top() + s.y));
        t.follow(p, center);
    }
    let snap = app.objects.snap != input.ctrl;
    t.value = t.compute(app, rect, snap);
    t.apply(app);
    if confirm {
        match t.confirm(app) {
            Ok(()) => return Step::Done,
            Err(text) => {
                app.refuse(Source::Path, text);
                if input.focus_lost {
                    // 離したのを受け取れないので続けない: やめる
                    t.cancel(app);
                    return Step::Done;
                }
            }
        }
    }
    app.objects.transform = Some(t);
    Step::Open
}

/// 途中の G/R/S を終える（`commit` なら決める。決められなければ（面に乗らないパス）やめる）。モードを替える前・ポップアップが外から
/// 閉じられたとき。
pub fn finish(app: &mut AppState, commit: bool) {
    let Some(t) = app.objects.transform.take() else {
        return;
    };
    if matches!(
        app.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::Transform)
    ) {
        app.popup = None;
    }
    if !t.still_valid(app) {
        t.cancel(app);
        return;
    }
    if commit {
        if let Err(text) = t.confirm(app) {
            app.refuse(Source::Path, text);
            t.cancel(app);
        }
    } else {
        t.cancel(app);
    }
}

/// ポップアップを外から閉じられた G/R/S を、そこまでで決める（フレームの初め）。
pub fn settle(app: &mut AppState) {
    if super::transforming(app)
        && !matches!(
            app.popup.as_ref().map(|p| p.kind),
            Some(PopupKind::Transform)
        )
    {
        finish(app, true);
    }
}

/// 頼まれた G/R/S を、ポインタの所から始める（キーの処理から）。3D ビューがこのウィンドウに無い・マウスのボタンを押している・ドラッグの
/// 途中は断る。
pub fn start_requested(ctx: &egui::Context, app: &mut AppState) {
    let Some(kind) = app.objects.request.take() else {
        return;
    };
    let lang = app.lang;
    let refuse_with = |app: &mut AppState, reason: &str| {
        let text = lang.with_reason(
            lang.pick(
                format!("{}できません", kind.name(lang)),
                format!("Cannot {}", kind.name(lang).to_lowercase()),
            ),
            reason,
        );
        app.refuse(Source::Edit, text);
    };
    if crate::pie::dragging(app) || ctx.input(|i| i.pointer.any_down()) || app.popup.is_some() {
        refuse_with(
            app,
            lang.pick("ほかの操作の途中です", "another operation is in progress"),
        );
        return;
    }
    if app.view3d.viewport != Some(ctx.viewport_id()) {
        refuse_with(
            app,
            lang.pick(
                "3D ビューが別のウィンドウにあります",
                "the 3D View is in another window",
            ),
        );
        return;
    }
    let pointer = ctx
        .input(|i| i.pointer.latest_pos())
        .or_else(|| app.view3d.view_rect.map(|r| r.center()))
        .unwrap_or_default();
    match begin(app, kind, pointer, ctx.cumulative_frame_nr()) {
        Ok(()) => {
            app.popup = Some(OpenPopup {
                kind: PopupKind::Transform,
                state: PopupState::new(ctx, Rect::from_min_size(pointer, egui::Vec2::ZERO)),
            });
        }
        Err(text) => app.refuse(Source::Edit, text),
    }
}

/// ツールの帯の移動・回転・拡縮で、選んだ物（ボーン）から左ドラッグを始めた（`objects.drag_request`）: その種類の G/R/S を、押した所から
/// 始める。量はドラッグの動きで、離して決める（Esc・右クリックでやめる）。3D ビューの入力が、押しを受けたフレームに呼ぶ。
pub fn start_drag(ctx: &egui::Context, app: &mut AppState) {
    let Some((kind, at, source)) = app.objects.drag_request.take() else {
        return;
    };
    if app.popup.is_some() || super::transforming(app) {
        return;
    }
    match begin_with(app, kind, at, ctx.cumulative_frame_nr(), Some(source)) {
        Ok(()) => {
            app.popup = Some(OpenPopup {
                kind: PopupKind::Transform,
                state: PopupState::new(ctx, Rect::from_min_size(at, egui::Vec2::ZERO)),
            });
        }
        Err(text) => app.refuse(Source::Edit, text),
    }
}

/// 開いている G/R/S を進めて、軸の線と状態の札を描く（`PopupKind::Transform` のポップアップを描くウィンドウのパスから）。続くなら true。
pub fn show(ctx: &egui::Context, app: &mut AppState, popup: &mut PopupState) -> bool {
    let Some(opened) = app.objects.transform.as_ref().map(|t| t.opened_frame) else {
        return false;
    };
    let screen_rect = ctx.content_rect();
    // 開いているあいだはキーを G/R/S が受ける（ほかの部品のフォーカスを外す。数字・Enter・Backspace を入力欄に取られない）
    if let Some(focused) = ctx.memory(|m| m.focused()) {
        ctx.memory_mut(|m| m.surrender_focus(focused));
    }
    // 入力の受け皿（下の部品の押しを止める）
    egui::Area::new(Id::new("yolu.transform.blocker"))
        .order(Order::Foreground)
        .fixed_pos(screen_rect.min)
        .constrain(false)
        .interactable(true)
        .show(ctx, |ui| {
            ui.interact(
                screen_rect,
                Id::new("yolu.transform.hit"),
                Sense::click_and_drag(),
            );
        });
    let frame = ctx.cumulative_frame_nr();
    let drag = app.objects.transform.as_ref().and_then(|t| t.drag);
    // ペンのドラッグは、ペンの点（3D ビューの入力が渡す）で追い、ペンを離したら決める
    let pen = matches!(drag, Some(gizmo::Source::Pen(_)));
    let pen_at = if pen { app.objects.pen_at.take() } else { None };
    let pen_lifted = pen && std::mem::take(&mut app.objects.pen_lifted);
    let input = ctx.input(|i| Input {
        pointer: if pen { pen_at } else { i.pointer.latest_pos() },
        ctrl: i.modifiers.ctrl || i.modifiers.command,
        keys: i
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
            .collect(),
        primary: frame != opened && i.pointer.primary_pressed(),
        secondary: frame != opened && i.pointer.secondary_pressed(),
        released: match drag {
            Some(gizmo::Source::Mouse) => i.pointer.primary_released(),
            Some(gizmo::Source::Pen(_)) => pen_lifted,
            None => false,
        },
        focus_lost: i
            .events
            .iter()
            .any(|e| matches!(e, Event::WindowFocused(false))),
    });
    if input.keys.iter().any(|(k, _)| *k == Key::Escape) {
        // この Esc は G/R/S が使った（後で描くウィンドウ・一覧が、同じ Esc で閉じたり選びを外したりしない）
        crate::ui::window::note_escape_taken(ctx);
    }
    if step(app, &input) == Step::Done {
        return false;
    }
    let (Some(t), Some(rect)) = (app.objects.transform.as_ref(), app.view3d.view_rect) else {
        return false;
    };
    popup.rect = rect;
    let painter = ctx
        .layer_painter(egui::LayerId::new(
            Order::Foreground,
            Id::new("yolu.transform"),
        ))
        .with_clip_rect(rect);
    let view = app.view3d.camera.view(rect.width(), rect.height());
    let to = |v: Vec2| pos2(rect.left() + v.x, rect.top() + v.y);
    let center = view.to_screen(t.pivot).map(to);
    // 軸の線（軸は 1 本、面はその 2 本）。物の中心を通して、表示域の端まで
    let axes: Vec<(usize, bool)> = match t.constraint {
        Constraint::Free => Vec::new(),
        Constraint::Axis { axis, local } => vec![(axis, local)],
        Constraint::Plane { axis, local } => vec![((axis + 1) % 3, local), ((axis + 2) % 3, local)],
    };
    let unit = sg::world_per_point(&view, t.pivot).max(1e-6) * 40.0;
    for (axis, local) in axes {
        let dir = t.direction(axis, local);
        let (Some(a), Some(b)) = (
            view.to_screen(t.pivot - dir * unit).map(to),
            view.to_screen(t.pivot + dir * unit).map(to),
        ) else {
            continue;
        };
        let d = (b - a).normalized();
        if !d.x.is_finite() || d == egui::Vec2::ZERO {
            continue;
        }
        let reach = rect.width() + rect.height();
        let mid = a + (b - a) / 2.0;
        painter.line_segment(
            [mid - d * reach, mid + d * reach],
            Stroke::new(1.5, axis_color(axis)),
        );
    }
    // 回転・拡縮は、中心からポインタへ細い線
    if matches!(t.kind, Kind::Rotate | Kind::Scale) {
        if let Some(c) = center {
            painter.line_segment(
                [c, t.last],
                Stroke::new(1.0, Color32::from_white_alpha(140)),
            );
        }
    }
    // 状態の札（表示域の左下）
    let text = t.status(app.lang);
    let p = ctx.layer_painter(egui::LayerId::new(
        Order::Foreground,
        Id::new("yolu.transform.status"),
    ));
    let width = w::text_width(&p, &text, theme::LABEL) + 20.0;
    let pill = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.bottom() - 36.0),
        vec2(width, 26.0),
    );
    w::rounded(&p, pill, theme::PANEL_BG, 4.0);
    w::outline(&p, pill, theme::SEPARATOR, 1.0, 4.0);
    w::text(&p, pill, &text, theme::LABEL, Align::Center);
    true
}

/// Alt+G/R/S: 選んだ物の位置・回転・大きさを既定へ（形はモデルの外形に合わせた置き場、ボーンはファイルの形。1 回の取り消し）。
pub fn reset(app: &mut AppState, kind: Kind) {
    let lang = app.lang;
    let what = lang.pick(
        match kind {
            Kind::Grab => "位置を戻せません",
            Kind::Rotate => "回転を戻せません",
            Kind::Scale => "大きさを戻せません",
        },
        match kind {
            Kind::Grab => "Cannot reset the position",
            Kind::Rotate => "Cannot reset the rotation",
            Kind::Scale => "Cannot reset the scale",
        },
    );
    if app.is_stroking() || super::transforming(app) || crate::pie::dragging(app) {
        app.refuse(Source::Edit, crate::lang::refusals::during_stroke(lang));
        return;
    }
    match app.mode {
        EditorMode::Paint => {}
        EditorMode::Pose => {
            let Some(bone) = app.view3d.pose.session.as_ref().and_then(|s| s.selected) else {
                let text = lang.with_reason(
                    what,
                    lang.pick("ボーンを選んでいません", "no bone is selected"),
                );
                app.refuse(Source::Pose, text);
                return;
            };
            let part = match kind {
                Kind::Grab => pose::edit::Part::Position,
                Kind::Rotate => pose::edit::Part::Rotation,
                Kind::Scale => pose::edit::Part::Scale,
            };
            pose::edit::reset(app, pose::edit::Reset::Part(bone, part));
        }
        EditorMode::Edit => {
            let Some(o) = super::selected(app) else {
                let text = lang.with_reason(what, super::nothing_selected(lang));
                app.refuse(Source::Edit, text);
                return;
            };
            if let Object::Ruler { layer, id } = o {
                if let Some(reason) = app.read_only_reason() {
                    let text = crate::lang::refusals::read_only_set(lang, reason);
                    app.refuse(Source::Edit, text);
                    return;
                }
                crate::rulers::edit3d::reset(app, kind, layer, id);
                return;
            };
            let Object::Shape(target) = o else {
                let text = lang.with_reason(
                    what,
                    lang.pick("既定の置き場がありません", "it has no default placement"),
                );
                app.refuse(Source::Edit, text);
                return;
            };
            if let Some(reason) = app.read_only_reason() {
                let text = crate::lang::refusals::read_only_set(lang, reason);
                app.refuse(Source::Edit, text);
                return;
            }
            let (Some(current), Some(default)) =
                (shape_of(app, target), default_shape(app, target))
            else {
                return;
            };
            let mut next = current;
            match kind {
                Kind::Grab => next.center = default.center,
                Kind::Rotate => next.rotation = default.rotation,
                Kind::Scale => next.size = default.size,
            }
            if next == current {
                return;
            }
            app.doc.end_coalescing();
            if let Some(Err(e)) = gizmo::write_shape(app, target, next, false) {
                app.notify(
                    crate::notice::Kind::of_core(&e),
                    Source::FillLayer,
                    app.lang.core_error(&e),
                );
            }
        }
    }
}

/// 形の既定の置き場（モデルの外形に合わせたもの。新しく作るときと同じ）。
fn default_shape(app: &AppState, target: Target) -> Option<sg::Shape> {
    use crate::fillfx::placement::{fit_to_bounds, new_shape_gradient_of};
    let bounds = app.view3d.full_model()?.geometry.bounds();
    let current = shape_of(app, target)?;
    let layer = app.doc.layer(target.layer())?;
    let from_placement = |p: yolu_core::fill_image::Placement| sg::Shape {
        center: p.center,
        rotation: p.rotation,
        size: p.size,
        ..current
    };
    let from_volume = |v: yolu_core::generator::Volume| sg::Shape {
        center: v.center,
        rotation: v.rotation,
        size: v.size,
        ..current
    };
    Some(match target {
        Target::Projection(_) => from_placement(fit_to_bounds(&bounds, layer.projection().mode)),
        Target::Gradient(_, channel) => {
            let g = layer.fill_gradient(channel)?;
            from_volume(new_shape_gradient_of(g.volume.shape, Some(&bounds)).volume)
        }
        Target::Filter(_, filter) => {
            let (_, effect, _) = app.doc.find_filter(filter)?;
            let g = effect.settings().generator_settings()?;
            if g.kind == yolu_core::generator::Kind::Image {
                from_placement(fit_to_bounds(&bounds, g.image.projection.mode))
            } else {
                from_volume(new_shape_gradient_of(g.volume.shape, Some(&bounds)).volume)
            }
        }
        // 定規は形の既定の置き場を持たない（Alt+G/R/S は `rulers::edit3d::reset`）
        Target::Ruler(..) => return None,
    })
}
