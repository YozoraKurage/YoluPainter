//! 回転のギズモ（ポーズのモードの 3D ビュー）: 選んだ骨の原点に、骨のローカルの X・Y・Z の輪と、画面に向いた輪を出す。
//! 輪を左ドラッグすると、その軸のまわりに回す。輪の外を押すと、その面をいちばん動かす骨を選ぶ。
//!
//! - 角度は「掴んだ輪の点を、ポインタについて行かせる」: 掴んだ点での回転の向き（軸 × 中心からの向き）を画面へ写した向きに、
//!   ポインタの動きを写して、その向きの 1 ラジアンあたりの画素で割る。輪が真横を向いていても同じ式で回る。
//! - ドラッグの間は、始まりのポーズに合計の角度を 1 回で当て直す（少しずつ足さないので、ずれが溜まらない）。Esc で始まりへ戻す。
//!   Ctrl を押している間は 5° 刻み。
//! - 回す計算は core の `Rig::rotate_bone_world`。ここは画面の幾何と入力だけ。

use egui::{Color32, Pos2, Rect, Shape, Stroke, Ui};
use yolu_core::geometry::{pick, CameraView};
use yolu_core::glam::{Mat4, Vec2, Vec3};
use yolu_core::skin::{Pose, Rig};

use super::pose;
use crate::state::AppState;

/// 骨の輪の画面での半径（点）。
pub const RING_RADIUS: f32 = 64.0;
/// 画面に向いた輪の半径（点）。
pub const VIEW_RING_RADIUS: f32 = 80.0;
/// 輪を掴める距離（点）。
pub const PICK_DISTANCE: f32 = 7.0;
const SEGMENTS: usize = 64;
const SNAP_DEGREES: f32 = 5.0;

/// 輪の色（X・Y・Z・画面）。
pub const AXIS_COLORS: [Color32; 4] = [
    Color32::from_rgb(228, 82, 82),
    Color32::from_rgb(110, 200, 90),
    Color32::from_rgb(80, 140, 240),
    Color32::from_rgb(200, 200, 205),
];
const ACTIVE: Color32 = Color32::from_rgb(250, 210, 70);

/// ギズモのドラッグの途中。
#[derive(Clone, Debug)]
pub struct GizmoDrag {
    pub bone: usize,
    /// 0・1・2 = 骨の X・Y・Z、3 = 画面に向いた軸。
    pub axis: usize,
    axis_world: Vec3,
    /// 掴んだ点での回転の向き（画面の単位の向き）と、1 ラジアンあたりの画素。
    tangent: Vec2,
    pixels_per_radian: f32,
    start_pointer: Vec2,
    start: Pose,
    world: Vec<Mat4>,
    /// 今の角度（ラジアン）。
    pub angle: f32,
}

/// 輪の幾何（画面の点は表示域の左上から）。
pub struct Rings {
    pub bone: usize,
    pub center: Vec3,
    pub center_screen: Vec2,
    pub axes: [Vec3; 4],
    /// 輪ごとの世界の点と画面の点（カメラの側の半分か）。
    pub points: [Vec<(Vec3, Vec2, bool)>; 4],
}

/// 選んだ骨のギズモの輪（モデル・選んだ骨が無い・骨がカメラの後ろなら None）。
pub fn rings(rig: &Rig, pose: &Pose, bone: usize, view: &CameraView) -> Option<Rings> {
    let world = rig.world_matrices(pose).ok()?;
    let center = Rig::bone_origin(&world, bone);
    let center_screen = view.to_screen(center)?;
    let per_unit = view.world_radius_to_screen(center, 1.0);
    if per_unit <= 0.0 {
        return None;
    }
    let q = Rig::world_rotation(&world, bone);
    let toward_camera = view.to_viewer(center).normalize_or_zero();
    let axes = [q * Vec3::X, q * Vec3::Y, q * Vec3::Z, -view.forward];
    let points = std::array::from_fn(|i| {
        let a = axes[i];
        let radius = if i == 3 {
            VIEW_RING_RADIUS
        } else {
            RING_RADIUS
        } / per_unit;
        let u = a.any_orthonormal_vector();
        let v = a.cross(u);
        (0..=SEGMENTS)
            .filter_map(|k| {
                let t = std::f32::consts::TAU * k as f32 / SEGMENTS as f32;
                let p = center + (u * t.cos() + v * t.sin()) * radius;
                let s = view.to_screen(p)?;
                let front = i == 3 || (p - center).dot(toward_camera) >= -1e-6 * radius;
                Some((p, s, front))
            })
            .collect()
    });
    Some(Rings {
        bone,
        center,
        center_screen,
        axes,
        points,
    })
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let d = b - a;
    let t = if d.length_squared() > 0.0 {
        ((p - a).dot(d) / d.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + d * t - p).length()
}

/// ポインタに近い輪（前の半分を先に。PICK_DISTANCE 以内）と、その輪のいちばん近い世界の点。
pub fn pick_ring(r: &Rings, p: Vec2) -> Option<(usize, Vec3)> {
    let mut best: Option<(usize, Vec3, f32, bool)> = None;
    for (axis, pts) in r.points.iter().enumerate() {
        for w in pts.windows(2) {
            let d = segment_distance(p, w[0].1, w[1].1);
            let front = w[0].2 && w[1].2;
            if d > PICK_DISTANCE {
                continue;
            }
            let better = match best {
                None => true,
                Some((_, _, bd, bf)) => (front && !bf) || (front == bf && d < bd),
            };
            if better {
                best = Some((axis, w[0].0, d, front));
            }
        }
    }
    best.map(|(a, w, _, _)| (a, w))
}

fn local(rect: Rect, p: Pos2) -> Vec2 {
    Vec2::new(p.x - rect.left(), p.y - rect.top())
}

fn camera_view(app: &AppState, rect: Rect) -> CameraView {
    app.view3d.camera.view(rect.width(), rect.height())
}

/// 今のギズモの輪（ポーズのモードで、骨を選んでいるとき）。
pub fn current_rings(app: &AppState, rect: Rect) -> Option<Rings> {
    let s = app.view3d.pose.session.as_ref()?;
    let bone = s.selected?;
    rings(&s.rig, s.pose(), bone, &camera_view(app, rect))
}

/// 掴んだ点をポインタについて行かせる回転の向き（画面の単位の向き）と、1 ラジアンあたりの画素。掴んだ点での回転の向きは
/// 軸 × 中心からの向き（長さは半径なので、ε 進むと ε ラジアン）。それを画面へ写す。輪が真横を向いていて、掴んだ点が奥行きの
/// 向きへ動く（画面ではほとんど動かない）ときは、中心を囲む向き（中心からポインタへの向きに直交する向き）と、中心からポインタ
/// までの画素で代える。
fn grab_motion(
    view: &CameraView,
    axis: Vec3,
    center: Vec3,
    center_screen: Vec2,
    grabbed: Vec3,
    pointer: Vec2,
) -> (Vec2, f32) {
    let t = axis.cross(grabbed - center);
    let eps = 1e-3;
    match (view.to_screen(grabbed), view.to_screen(grabbed + t * eps)) {
        (Some(s0), Some(s1)) if (s1 - s0).length() / eps > 1.0 => {
            ((s1 - s0).normalize(), (s1 - s0).length() / eps)
        }
        _ => {
            let d = pointer - center_screen;
            (
                Vec2::new(-d.y, d.x).normalize_or_zero(),
                d.length().max(1.0),
            )
        }
    }
}

/// 左ボタンを押した（ポーズのモード）: 輪の上ならドラッグを始め、外なら面をいちばん動かす骨を選ぶ。
pub fn press(app: &mut AppState, rect: Rect, at: Pos2) {
    if app.is_stroking() {
        return;
    }
    let p = local(rect, at);
    let view = camera_view(app, rect);
    if let Some(r) = current_rings(app, rect) {
        if let Some((axis, grabbed)) = pick_ring(&r, p) {
            let Some(s) = app.view3d.pose.session.as_ref() else {
                return;
            };
            let a = r.axes[axis];
            let world = match s.rig.world_matrices(s.pose()) {
                Ok(w) => w,
                Err(e) => {
                    let e: super::model::ViewError = e.into();
                    app.notify(
                        e.notice_kind(),
                        crate::notice::Source::Pose,
                        app.lang.view_error(&e),
                    );
                    return;
                }
            };
            let start = s.pose().clone();
            let (tangent, ppr) = grab_motion(&view, a, r.center, r.center_screen, grabbed, p);
            if let Err(e) = pose::begin_edit(&mut app.view3d) {
                app.notify(
                    e.notice_kind(),
                    crate::notice::Source::Pose,
                    app.lang.view_error(&e),
                );
                return;
            }
            app.view3d.pose.drag = Some(GizmoDrag {
                bone: r.bone,
                axis,
                axis_world: a,
                tangent,
                pixels_per_radian: ppr,
                start_pointer: p,
                start,
                world,
                angle: 0.0,
            });
            return;
        }
    }
    // 輪の外: 面の下のボーンを選ぶ
    if let Some(bone) = bone_at(app, rect, at) {
        if let Some(s) = app.view3d.pose.session.as_mut() {
            s.selected = Some(bone);
            reveal(s, bone);
        }
    }
}

/// 画面の点の下の面のボーン（見えている形で当て、三角形の番号をスキンと同じ受けたままの形の番号へ直す。隠したマテリアルや
/// ボーンの影響で隠した面は見えないので当たらない）。
pub fn bone_at(app: &AppState, rect: Rect, at: Pos2) -> Option<usize> {
    let model = app.view3d.model.as_ref()?;
    let view = camera_view(app, rect);
    let hit = pick(&model.geometry, &view, local(rect, at))?;
    let triangle = app.view3d.full_triangle(hit.triangle)?;
    app.view3d
        .pose
        .session
        .as_ref()?
        .rig
        .bone_at_triangle(triangle, hit.barycentric)
}

/// 木で骨が見えるように、親を全部開く。
pub fn reveal(s: &mut pose::PoseSession, bone: usize) {
    let mut p = s.rig.bones()[bone].parent;
    while let Some(i) = p {
        s.expanded.insert(i as usize);
        p = s.rig.bones()[i as usize].parent;
    }
    s.reveal = Some(bone);
}

/// ドラッグの途中（snap は 5° 刻み）。
pub fn drag_to(app: &mut AppState, rect: Rect, at: Pos2, snap: bool) {
    let Some(d) = app.view3d.pose.drag.as_mut() else {
        return;
    };
    let moved = local(rect, at) - d.start_pointer;
    let mut angle = moved.dot(d.tangent) / d.pixels_per_radian;
    if snap {
        let step = SNAP_DEGREES.to_radians();
        angle = (angle / step).round() * step;
    }
    if angle == d.angle {
        return;
    }
    d.angle = angle;
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return;
    };
    let mut next = d.start.clone();
    s.rig
        .rotate_bone_world(&mut next, &d.world, d.bone, d.axis_world, angle);
    if let Err(e) = pose::edit(&mut app.view3d, next) {
        app.notify(
            e.notice_kind(),
            crate::notice::Source::Pose,
            app.lang.view_error(&e),
        );
    }
}

/// ドラッグを終える（commit でなければ始まりへ戻す）。
pub fn release(app: &mut AppState, commit: bool) {
    if app.view3d.pose.drag.take().is_some() {
        pose::end_edit(&mut app.view3d, commit);
    }
}

/// ギズモと、選んだ骨の線（親と子へ）を描く。hover はポインタ（輪を強調する）。
pub fn draw(ui: &Ui, app: &mut AppState, rect: Rect, hover: Option<Pos2>) {
    let Some(r) = current_rings(app, rect) else {
        app.view3d.pose.hover_axis = None;
        return;
    };
    let painter = ui.painter_at(rect);
    let to = |s: Vec2| Pos2::new(rect.left() + s.x, rect.top() + s.y);
    let view = camera_view(app, rect);
    // 骨の線
    if let Some(s) = &app.view3d.pose.session {
        if let Ok(world) = s.rig.world_matrices(s.pose()) {
            let line = Stroke::new(1.5, Color32::from_white_alpha(170));
            let mut others: Vec<usize> =
                s.rig.children(r.bone).iter().map(|&c| c as usize).collect();
            others.extend(s.rig.bones()[r.bone].parent.map(|p| p as usize));
            for o in others {
                if let Some(e) = view.to_screen(Rig::bone_origin(&world, o)) {
                    painter.line_segment([to(r.center_screen), to(e)], line);
                    painter.circle_filled(to(e), 2.5, Color32::from_white_alpha(170));
                }
            }
        }
    }
    let active = app.view3d.pose.drag.as_ref().map(|d| d.axis);
    let hovered = if active.is_none() {
        hover.and_then(|h| pick_ring(&r, local(rect, h)).map(|(a, _)| a))
    } else {
        None
    };
    app.view3d.pose.hover_axis = hovered;
    // 後ろの半分を先に、薄く
    for pass in [false, true] {
        for (axis, pts) in r.points.iter().enumerate() {
            let strong = active == Some(axis) || hovered == Some(axis);
            let color = if active == Some(axis) {
                ACTIVE
            } else {
                AXIS_COLORS[axis]
            };
            let width = if strong { 3.0 } else { 2.0 };
            let mut run: Vec<Pos2> = Vec::new();
            let flush = |run: &mut Vec<Pos2>| {
                if run.len() > 1 {
                    let c = if pass {
                        color
                    } else {
                        color.gamma_multiply(0.3)
                    };
                    painter.add(Shape::line(std::mem::take(run), Stroke::new(width, c)));
                }
                run.clear();
            };
            for &(_, s, front) in pts {
                if front == pass {
                    run.push(to(s));
                } else {
                    flush(&mut run);
                }
            }
            flush(&mut run);
        }
    }
    painter.circle_filled(to(r.center_screen), 3.5, ACTIVE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Action;
    use crate::view3d::pose::PoseAction;
    use yolu_core::geometry::OrbitCamera;
    use yolu_core::glam::Quat;

    fn rect() -> Rect {
        Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))
    }

    /// 試しの人形を読んで、右上腕を選んだ状態。
    fn selected() -> (AppState, usize) {
        let mut app = AppState::new(64, 64);
        app.apply(Action::Pose(PoseAction::LoadFigure));
        let s = app.view3d.pose.session.as_mut().expect("読めた");
        let bone = s
            .rig
            .bones()
            .iter()
            .position(|b| b.name == "右上腕")
            .unwrap();
        s.selected = Some(bone);
        (app, bone)
    }

    /// 輪 axis の、カメラ側で、ほかの輪から最も離れた区間の、真ん中の画面の点（掴む位置）と、その区間の始まりの世界の点（掴まれる
    /// 点）の画面の点と、輪の画面の向き（単位）。
    fn grab_at(r: &Rings, axis: usize) -> (Vec2, Vec2, Vec2) {
        let others: Vec<Vec2> = r
            .points
            .iter()
            .enumerate()
            .filter(|(a, _)| *a != axis)
            .flat_map(|(_, p)| p.iter().map(|x| x.1))
            .collect();
        let pts = &r.points[axis];
        let apart = |s: Vec2| {
            others
                .iter()
                .map(|o| (*o - s).length())
                .fold(f32::MAX, f32::min)
        };
        let k = (0..pts.len() - 1)
            .filter(|&k| pts[k].2 && pts[k + 1].2)
            .max_by(|&a, &b| apart(pts[a].1).total_cmp(&apart(pts[b].1)))
            .expect("輪の前の点");
        (
            (pts[k].1 + pts[k + 1].1) * 0.5,
            pts[k].1,
            (pts[k + 1].1 - pts[k].1).normalize(),
        )
    }

    fn at(p: Vec2) -> Pos2 {
        Pos2::new(p.x, p.y)
    }

    /// 骨のワールドの回転の、始まりからの変化。
    fn turned(app: &AppState, bone: usize, start: &Pose) -> Quat {
        let s = app.view3d.pose.session.as_ref().unwrap();
        let before = s.rig.world_matrices(start).unwrap();
        let after = s.rig.world_matrices(s.pose()).unwrap();
        Rig::world_rotation(&after, bone) * Rig::world_rotation(&before, bone).inverse()
    }

    #[test]
    fn the_grabbed_point_follows_the_pointer_in_both_directions() {
        let mut tested = 0;
        for axis in 0..4 {
            let (mut app, bone) = selected();
            let rect = rect();
            let r = current_rings(&app, rect).expect("輪");
            let (grab, point, dir) = grab_at(&r, axis);
            press(&mut app, rect, at(grab));
            let d = app.view3d.pose.drag.as_ref().expect("輪を掴んだ");
            assert_eq!(d.axis, axis);
            let ppr = d.pixels_per_radian;
            if ppr < 12.0 {
                continue; // 真横に近い輪は画素あたりの角度が大きく、小さな動きの近似が合わない（別の試験で見る）
            }
            tested += 1;
            let (_, grabbed) = pick_ring(&r, grab).unwrap();
            let view = camera_view(&app, rect);
            assert!(
                (view.to_screen(grabbed).unwrap() - point).length() < 1e-3,
                "掴まれたのは掴んだ区間の始まりの点"
            );
            let start = app.view3d.pose.session.as_ref().unwrap().pose().clone();
            // 輪に沿って +・−・もう一度 + へ（始まりからの動きなので、足し算で溜まらない）
            for sign in [1.0f32, -1.0, 1.0] {
                let delta = 0.08 * ppr * sign; // 約 0.08 ラジアン
                drag_to(&mut app, rect, at(grab + dir * delta), false);
                let q = turned(&app, bone, &start);
                // 掴んだ点が、骨の回転と一緒に動いたとして、画面のどこへ行くか（ポインタと同じだけ動くはず）
                let moved = r.center + q * (grabbed - r.center);
                let got = view.to_screen(moved).unwrap() - point;
                let want = dir * delta;
                assert!(
                    (got - want).length() < 0.15 * delta.abs() + 0.3,
                    "輪 {axis}: 掴んだ点は {got} だけ動いた（ポインタは {want}。ppr {ppr}）"
                );
                let angle = app.view3d.pose.drag.as_ref().unwrap().angle;
                assert!(
                    (q.angle_between(Quat::IDENTITY) - angle.abs()).abs() < 1e-4,
                    "輪 {axis}: ドラッグの角度 {angle} と回った角度が合う"
                );
            }
            release(&mut app, true);
        }
        assert!(tested >= 2, "{tested} 本の輪で確かめた");
    }

    #[test]
    fn ctrl_snaps_the_angle_to_five_degree_steps() {
        let (mut app, bone) = selected();
        let rect = rect();
        let r = current_rings(&app, rect).unwrap();
        let (grab, _, dir) = grab_at(&r, 2);
        press(&mut app, rect, at(grab));
        let start = app.view3d.pose.session.as_ref().unwrap().pose().clone();
        let step = SNAP_DEGREES.to_radians();
        let mut off_grid = 0;
        for px in [1.0f32, 4.0, 9.0, 13.0, 27.0, -5.0, -31.0] {
            drag_to(&mut app, rect, at(grab + dir * px), true);
            let angle = app.view3d.pose.drag.as_ref().unwrap().angle;
            let steps = angle / step;
            assert!(
                (steps - steps.round()).abs() < 1e-4,
                "{px} 画素: {angle} ラジアンは 5° の倍数"
            );
            let q = turned(&app, bone, &start);
            assert!((q.angle_between(Quat::IDENTITY) - angle.abs()).abs() < 1e-4);
            // 同じ位置を刻まずに当てると、5° の倍数にはならない
            drag_to(&mut app, rect, at(grab + dir * px), false);
            let free = app.view3d.pose.drag.as_ref().unwrap().angle / step;
            if (free - free.round()).abs() > 0.01 {
                off_grid += 1;
            }
        }
        assert!(off_grid >= 4, "刻みが効いている（{off_grid}）");
        release(&mut app, true);
    }

    /// 真横を向いた輪（掴んだ点が奥行きへ動く）では、中心を囲む向きと中心からの画素で代える。それ以外は輪の接線を画面へ写す。
    #[test]
    fn grab_motion_falls_back_when_the_ring_is_edge_on() {
        let camera = OrbitCamera {
            yaw: 0.0,
            pitch: 0.0,
            ..OrbitCamera::default()
        };
        let view = camera.view(800.0, 600.0);
        let forward = view.forward;
        let right = forward.cross(Vec3::Y).normalize();
        let radius = 64.0 / view.world_radius_to_screen(camera.target, 1.0);
        // 軸は上。掴んだ点は画面の中心（光軸の上）で、中心はその左: 回転の向き（軸 × 中心からの向き）は奥行きと同じ向き
        let grabbed = camera.target;
        let center = grabbed - right * radius;
        let center_screen = view.to_screen(center).unwrap();
        for pointer in [Vec2::new(0.0, 50.0), Vec2::new(30.0, 40.0)] {
            let (tangent, ppr) = grab_motion(
                &view,
                Vec3::Y,
                center,
                center_screen,
                grabbed,
                center_screen + pointer,
            );
            assert!((ppr - pointer.length()).abs() < 1e-3, "{ppr}");
            let want = Vec2::new(-pointer.y, pointer.x).normalize();
            assert!((tangent - want).length() < 1e-4, "{tangent}");
        }
        // 中心に重なるポインタでも壊れない（向きは 0、画素は 1 以上）
        let (tangent, ppr) = grab_motion(
            &view,
            Vec3::Y,
            center,
            center_screen,
            grabbed,
            center_screen,
        );
        assert_eq!((tangent, ppr), (Vec2::ZERO, 1.0));
        // 掴んだ点が手前（奥行きの向き）にある輪は、普通に画面へ写る: 向きは単位で、画素あたりの角度は輪の画面の半径
        let center = camera.target;
        let center_screen = view.to_screen(center).unwrap();
        let grabbed = center - forward * radius;
        let (tangent, ppr) =
            grab_motion(&view, Vec3::Y, center, center_screen, grabbed, Vec2::ZERO);
        assert!((tangent.length() - 1.0).abs() < 1e-4);
        assert!(tangent.x.abs() > 0.999, "横へ動く: {tangent}");
        assert!((ppr - 64.0).abs() < 6.0, "{ppr}");
    }
}
