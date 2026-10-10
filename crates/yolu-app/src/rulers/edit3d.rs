//! 3D の定規（モデルの空間の定規）を 3D ビューで扱う幾何: 3D ビューで引いて作る、ストロークの始めのカメラで画面に写した寄せ先にする、編集のモードで
//! 動かす（印の位置・向きの枠・端の点・G/R/S で当てる量）。文書に入れるのは `ops`（`RulerAction`）で、線の描画は `draw3d`。
//!
//! - 点の意味は core の `RulerPlace::Model { a, b, up }`（直線・平行線・パースは作った面の法線が `up`、同心円は円の面の法線、対称は回転の軸。
//!   最初の鏡の面は `up` と `b − a` を含む）。
//! - 編集のモードの印は、2 つの点を持つ物（直線・平行線・2 点のパース）では 2 点の真ん中、ほかは `a`。向きの枠は X = a→b（`up` に直交する成分）、
//!   Y = `up`、Z = X × Y。

use egui::Rect;
use yolu_core::geometry::{pick, CameraView, SurfaceHit};
use yolu_core::glam::{DVec2, DVec3, Mat3, Quat, Vec2, Vec3};
use yolu_core::{LayerId, Ruler, RulerId, RulerKind, RulerPlace};

use super::active::nearest_line;
use super::ops::configure_new;
use super::Place;
use crate::drafting::Constraint;
use crate::fillfx::gizmo::{ShapeDrag, Target};
use crate::objects::transform::{map_point, map_vector, Kind, Value};
use crate::state::AppState;
use crate::view3d::draft::SurfaceDraft;
use crate::view3d::input::camera_view;
use crate::view3d::shape_gizmo as sg;

/// 引いたとみなす、押してから離すまでに画面で動いた距離（点）。これ以下の対称定規は、境界の中心を通る X の鏡。
pub const DRAG_POINTS: f64 = 4.0;
/// 同心円を折れ線にするときの分割数。
pub const CIRCLE_SEGMENTS: usize = 256;
/// 端の点のつまみを掴める近さ（画面の点）。
pub const END_POINTS: f32 = 9.0;

/// 端の点 a・b。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    A,
    B,
}

/// 3D の定規の `a`・`b`・単位にした `up`（2D の定規は None）。
pub fn points(r: &Ruler) -> Option<(DVec3, DVec3, DVec3)> {
    match r.place {
        RulerPlace::Model { a, b, up } => Some((a, b, up.normalize_or_zero())),
        RulerPlace::Canvas { .. } => None,
    }
}

/// 2 つの点を持つ物か（直線・平行線・2 点のパース）。
pub fn has_two_points(r: &Ruler) -> bool {
    matches!(r.kind, RulerKind::Line | RulerKind::Parallel)
        || (r.kind == RulerKind::Perspective && r.two_points)
}

/// 編集のモードの印・G/R/S の中心（2 つの点を持つ物は 2 点の真ん中、ほかは `a`）。
pub fn center(r: &Ruler) -> Option<DVec3> {
    let (a, b, _) = points(r)?;
    Some(if has_two_points(r) { (a + b) * 0.5 } else { a })
}

/// 向きの枠（X = a→b の `up` に直交する成分、Y = `up`、Z = X × Y）。a→b が `up` と平行なときは、`up` に直交する適当な向きを X にする。
pub fn frame(r: &Ruler) -> Option<Quat> {
    let (a, b, up) = points(r)?;
    let y = up.as_vec3();
    if y.length_squared() < 0.5 {
        return None;
    }
    let along = (b - a).as_vec3();
    let mut x = along - y * along.dot(y);
    if x.length_squared() < 1e-18 {
        x = y.cross(if y.x.abs() < 0.9 { Vec3::X } else { Vec3::Y });
    }
    let x = x.normalize();
    let z = x.cross(y);
    Some(Quat::from_mat3(&Mat3::from_cols(x, y, z)).normalize())
}

fn d3(v: Vec3) -> DVec3 {
    v.as_dvec3()
}

/// 形のギズモ（移動の矢印と回す輪）に載せる形: 中心と向きの枠だけで、大きさは無い。
pub fn gizmo_shape(r: &Ruler) -> Option<sg::Shape> {
    let c = center(r)?;
    let q = frame(r)?;
    Some(sg::Shape {
        kind: sg::Kind::Anchor,
        center: [c.x, c.y, c.z],
        rotation: sg::euler_degrees(q),
        size: [0.0; 3],
        falloff: 0.0,
    })
}

/// 形のギズモで動かした形 `next` を定規へ当てる（`current` の今の形との差の移動と回転を、中心のまわりで a・b・`up` に）。
pub fn ruler_after_shape(current: &Ruler, next: &sg::Shape) -> Ruler {
    let Some(from) = gizmo_shape(current) else {
        return current.clone();
    };
    let Some((a, b, up)) = points(current) else {
        return current.clone();
    };
    let c0 = DVec3::from_array(from.center);
    let c1 = DVec3::from_array(next.center);
    let turn = sg::rotation_of(next) * sg::rotation_of(&from).inverse();
    // 回したか: 回転の軸の成分の長さ（sin(角度/2)）が、オイラー角を通した f32 の丸め（約 1e-7）より大きいか。移動だけのドラッグは、毎フレーム今の定規から
    // 作り直した向きと同じ向きなので、丸めの差で回す道（f32）へ入らず、動かした分を f64 のまま足す
    let (a, b, up) = if turn.xyz().length() < 1e-6 {
        let delta = c1 - c0;
        (a + delta, b + delta, up)
    } else {
        let at = |p: DVec3| c1 + d3(turn * (p - c0).as_vec3());
        (at(a), at(b), d3(turn * up.as_vec3()).normalize_or_zero())
    };
    with_points(current, a, b, up)
}

/// G/R/S の量 `value` を、始めの定規 `start` に当てる（`pivot` はまわす・拡縮の中心、`orientation` は拡縮の軸の向き）。移動は a・b を動かし、回転は中心のまわりに
/// a・b と `up` を回し、拡縮は中心から a・b の距離を変える（`up` は変えない）。量が恒等（0 の移動・0 度・倍率 1）なら定規の値を変えない。
pub fn ruler_after(start: &Ruler, pivot: Vec3, value: Value, orientation: Quat) -> Ruler {
    let Some((a, b, up)) = points(start) else {
        return start.clone();
    };
    match value {
        Value::Move(delta) => {
            let delta = d3(delta);
            with_points(start, a + delta, b + delta, up)
        }
        Value::Turn { degrees: 0.0, .. } => start.clone(),
        Value::Factor(f) if f == Vec3::ONE => start.clone(),
        _ => {
            let map = |p: DVec3| d3(map_point(p.as_vec3(), pivot, value, orientation));
            let up = match value {
                Value::Turn { .. } => d3(map_vector(up.as_vec3(), value, orientation)),
                _ => up,
            };
            with_points(start, map(a), map(b), up.normalize_or_zero())
        }
    }
}

/// 点を替えた定規（ほかの値は同じ）。
pub fn with_points(r: &Ruler, a: DVec3, b: DVec3, up: DVec3) -> Ruler {
    let mut next = r.clone();
    next.place = RulerPlace::Model { a, b, up };
    next
}

/// 掴める端の点（編集のモード）: 直線・平行線は a・b、2 点のパースも a・b、同心円と対称は b（半径・最初の線）。ほかの点は印（中心）のほうで動かす。
pub fn ends(r: &Ruler) -> Vec<(End, DVec3)> {
    let Some((a, b, _)) = points(r) else {
        return Vec::new();
    };
    if has_two_points(r) {
        vec![(End::A, a), (End::B, b)]
    } else if matches!(r.kind, RulerKind::Concentric | RulerKind::Symmetry) {
        vec![(End::B, b)]
    } else {
        Vec::new()
    }
}

/// 端の点を `to` へ動かした定規。
pub fn with_end(r: &Ruler, end: End, to: DVec3) -> Ruler {
    let Some((a, b, up)) = points(r) else {
        return r.clone();
    };
    match end {
        End::A => with_points(r, to, b, up),
        End::B => with_points(r, a, to, up),
    }
}

/// 端の点を掴める画面の点（表示域の左上から）。
pub fn end_screen(view: &CameraView, p: DVec3) -> Option<Vec2> {
    view.to_screen(p.as_vec3())
}

/// ポインタの下の端の点（掴める近さの内の一番近いもの）。
pub fn end_at(r: &Ruler, view: &CameraView, local: Vec2) -> Option<End> {
    ends(r)
        .into_iter()
        .filter_map(|(end, p)| Some((end, end_screen(view, p)?.distance(local))))
        .filter(|(_, d)| *d <= END_POINTS)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(end, _)| end)
}

/// ポインタの下の点: 面に当たればその点、当たらなければ `plane_point` を通りカメラに向いた面の上の点。
pub fn point_under(
    app: &AppState,
    view: &CameraView,
    local: Vec2,
    plane_point: Vec3,
) -> Option<Vec3> {
    if let Some(model) = &app.view3d.model {
        if let Some(hit) = pick(&model.geometry, view, local) {
            return Some(hit.position);
        }
    }
    on_camera_plane(view, local, plane_point)
}

/// `plane_point` を通りカメラに向いた面の上の、画面の点の下の点。
pub fn on_camera_plane(view: &CameraView, local: Vec2, plane_point: Vec3) -> Option<Vec3> {
    let ray = sg::ray_of(view, local);
    let t = sg::ray_plane(ray, plane_point, -view.forward)?;
    Some(ray.0 + ray.1 * t)
}

/// モデルの外形の対角線の長さ（モデルが無ければ 1）。
fn model_size(app: &AppState) -> f32 {
    app.view3d
        .model
        .as_ref()
        .map_or(1.0, |m| m.geometry.bounds().size().length())
        .max(1e-6)
}

/// ドラッグで引いた対称定規の `(a, b, up)`: 押した面の点 `p` を通り、画面で引いた線 `l`（カメラに向いた面の上の動き）と視線を含む面が最初の鏡の面
/// （`up` は引いた線に直交して面に入る向き）。`shift` なら鏡の法線をモデルの X・Y・Z のうち近い軸へ。引いた線が 0、または視線と平行なら None。
fn symmetry_from_drag(
    view: &CameraView,
    p: Vec3,
    end: Vec3,
    shift: bool,
) -> Option<(DVec3, DVec3, DVec3)> {
    let l = end - p;
    let length = l.length();
    if length.is_nan() || length <= 1e-9 {
        return None;
    }
    let along = l / length;
    let sight = view.sight(p);
    let v = sight.to / sight.distance.max(1e-12);
    let mut up = (v - along * v.dot(along)).try_normalize()?;
    let mut d = along;
    if shift {
        let (_, n) = closest_axis(up.cross(along).try_normalize()?, None);
        d = (along - n * along.dot(n)).try_normalize()?;
        up = d.cross(n).try_normalize()?;
    }
    Some((d3(p), d3(p + d * length), d3(up)))
}

/// 端の点のつまみのドラッグの途中: ポインタの下の点へ（面に当たればその点、当たらなければ前の点を通りカメラに向いた面の上）。文書へは続けて変える操作として
/// 入れる（1 回の取り消し。確かめに通らない場所には動かさない）。
pub fn drag_end(app: &mut AppState, rect: Rect, d: &ShapeDrag, at: egui::Pos2) {
    let Target::Ruler(layer, id) = d.target else {
        return;
    };
    let end = match d.handle {
        sg::Handle::EndA => End::A,
        sg::Handle::EndB => End::B,
        _ => return,
    };
    let Some(ruler) = crate::objects::ruler_of(app, layer, id) else {
        return;
    };
    let Some(previous) = ends(&ruler)
        .into_iter()
        .find(|(e, _)| *e == end)
        .map(|e| e.1)
    else {
        return;
    };
    let view = camera_view(app, rect);
    // 押した点と端の四角の中心のずれを保つ（最初の動きで、端がポインタの真下へ跳ばない）
    let local = Vec2::new(at.x - rect.left(), at.y - rect.top()) + d.grab;
    let Some(to) = point_under(app, &view, local, previous.as_vec3()) else {
        return;
    };
    let moved = with_end(&ruler, end, d3(to));
    if moved == ruler || moved.validate().is_err() {
        return;
    }
    if let Err(e) = app.ruler_put(layer, moved, true) {
        app.notify(
            crate::notice::Kind::of_core(&e),
            crate::notice::Source::Ruler,
            app.lang.core_error(&e),
        );
        // 保存できなかった: ここでやめる（毎フレーム同じ知らせを出さない。形のギズモと同じ）
        crate::fillfx::gizmo::release(app, false);
    }
}

/// 端の点のつまみを押したときの、押した点（表示域の左上から）から端の四角の中心までのずれ。端のつまみでなければ 0。
pub fn grab_offset(
    app: &AppState,
    view: &CameraView,
    target: Target,
    handle: sg::Handle,
    pressed: Vec2,
) -> Vec2 {
    let Target::Ruler(layer, id) = target else {
        return Vec2::ZERO;
    };
    let end = match handle {
        sg::Handle::EndA => End::A,
        sg::Handle::EndB => End::B,
        _ => return Vec2::ZERO,
    };
    crate::objects::ruler_of(app, layer, id)
        .and_then(|r| ends(&r).into_iter().find(|(e, _)| *e == end))
        .and_then(|(_, p)| end_screen(view, p))
        .map_or(Vec2::ZERO, |center| center - pressed)
}

/// 軸（モデルの X・Y・Z の符号つき）のうち、`v` に一番近いもの。`not` の軸は除く。
fn closest_axis(v: Vec3, not: Option<usize>) -> (usize, Vec3) {
    let k = (0..3)
        .filter(|k| Some(*k) != not)
        .max_by(|i, j| v[*i].abs().total_cmp(&v[*j].abs()))
        .unwrap_or(0);
    let mut axis = Vec3::ZERO;
    axis[k] = if v[k] < 0.0 { -1.0 } else { 1.0 };
    (k, axis)
}

/// Alt+G/R/S: 中心をモデルの境界の中心へ・向きをモデルの軸へ（`up` を近い軸、a→b をその次に近い軸）・大きさ（a–b の距離）をモデルの大きさの 1/4 へ。
/// 1 回の取り消し。
pub fn reset(app: &mut AppState, kind: Kind, layer: LayerId, id: RulerId) {
    let (Some(r), Some(model)) = (
        crate::objects::ruler_of(app, layer, id),
        app.view3d.model.clone(),
    ) else {
        return;
    };
    let (Some((a, b, up)), Some(c)) = (points(&r), center(&r)) else {
        return;
    };
    let bounds = model.geometry.bounds();
    let length = a.distance(b);
    let along = (b - a) / length;
    // 中心を保ったまま、向き `d`・長さ `len` の a・b にする（2 つの点を持つ物は中心が真ん中、ほかは a が中心）
    let place = |d: DVec3, len: f64, up: DVec3| {
        if has_two_points(&r) {
            with_points(&r, c - d * len * 0.5, c + d * len * 0.5, up)
        } else {
            with_points(&r, a, a + d * len, up)
        }
    };
    let next = match kind {
        Kind::Grab => {
            let delta = d3(bounds.center) - c;
            with_points(&r, a + delta, b + delta, up)
        }
        Kind::Rotate => {
            let (up_axis, new_up) = closest_axis(up.as_vec3(), None);
            let flat = along.as_vec3() - new_up * along.as_vec3().dot(new_up);
            let (_, d) = closest_axis(flat, Some(up_axis));
            place(d3(d), length, d3(new_up))
        }
        // 1 点のパースは b を使わない（動かすのは b だけなので、何も変わらない取り消しの段を作らない）
        Kind::Scale if r.kind == RulerKind::Perspective && !r.two_points => return,
        Kind::Scale => place(
            along,
            f64::from((bounds.size().length() * 0.25).max(1e-3)),
            up,
        ),
    };
    if next == r || next.validate().is_err() {
        return;
    }
    app.doc.end_coalescing();
    if let Err(e) = app.ruler_put(layer, next, false) {
        app.notify(
            crate::notice::Kind::of_core(&e),
            crate::notice::Source::Ruler,
            app.lang.core_error(&e),
        );
    }
}

/// ドラッグで決めるこれから作る定規（押した面の点・離した点・設定から）。作れなければ None（モデルの外で押した・引いた長さが 0・値が確かめに通らない）。
pub fn build(app: &AppState, rect: Rect, d: &SurfaceDraft) -> Option<Ruler> {
    let hit = d.hit?;
    let view = camera_view(app, rect);
    let local = |p: DVec2| Vec2::new(p.x as f32, p.y as f32);
    let pressed = hit.position;
    let dragged = d.start.distance(d.current) > DRAG_POINTS;
    let kind = app.rulers.kind;
    let (a, b, up) = if kind == RulerKind::Symmetry {
        if dragged {
            let end = on_camera_plane(&view, local(d.current), pressed)?;
            symmetry_from_drag(&view, pressed, end, d.shift)?
        } else {
            // 押して離しただけ: モデルの境界の中心を通る X の鏡（今の 3D の対称の既定と同じ置き方）
            let center = app.view3d.model.as_ref()?.geometry.bounds().center;
            let reach = model_size(app) * 0.25;
            (d3(center), d3(center + Vec3::Z * reach), DVec3::Y)
        }
    } else {
        if !dragged {
            return None;
        }
        let end = point_under(app, &view, local(d.current), pressed)?;
        (d3(pressed), d3(end), d3(hit.normal))
    };
    let ruler = configure_new(&app.rulers, Ruler::model(RulerId(1), kind, a, b, up));
    ruler.validate().is_ok().then_some(ruler)
}

/// 離した: 文書の定規を作る（作る先と取り消しの単位は `ruler_create`）。
pub fn create(app: &mut AppState, rect: Rect, d: &SurfaceDraft) {
    if let Some(ruler) = build(app, rect, d) {
        app.apply(crate::state::Action::Ruler(super::RulerAction::Create(
            ruler,
        )));
    }
}

/// 直線定規を画面へ写す 3D の線分: a を通り a→b の向きに、描く長さ（`draw3d` の線と同じ、モデルの外形の `reach` の 4 倍）まで伸ばし、カメラの手前の側
/// （画面に写せる側）で切る。片端がカメラの後ろでも、手前に残る部分で線が決まる。全部が後ろなら None。
fn visible_line(view: &CameraView, a: Vec3, b: Vec3, reach: f32) -> Option<(Vec3, Vec3)> {
    let d = (b - a).try_normalize()?;
    let (p0, p1) = (a - d * reach * 4.0, a + d * reach * 4.0);
    // 写せる一番浅い奥行き（正投影は近い面、透視はカメラの面の少し前）。少し手前に余裕を見る
    let min = if view.is_orthographic() {
        view.near
    } else {
        view.near.max(1e-4)
    };
    let min = min + 1e-4 * (1.0 + min.abs());
    let (d0, d1) = (view.depth(p0), view.depth(p1));
    // 奥行きは線に沿って一次なので、`min` 以上の区間を切り出す
    let (lo, hi) = match (d0 >= min, d1 >= min) {
        (true, true) => (0.0, 1.0),
        (false, false) => return None,
        (true, false) => (0.0, (min - d0) / (d1 - d0)),
        (false, true) => ((min - d0) / (d1 - d0), 1.0),
    };
    Some((p0.lerp(p1, lo), p0.lerp(p1, hi)))
}

fn screen2(p: Vec2) -> DVec2 {
    DVec2::new(p.x as f64, p.y as f64)
}

impl AppState {
    /// 3D ビューのストロークの始めに決める寄せ先（押した点 `press` は表示域の左上からの画面の点、`hit` は押した面の点。カメラは今のカメラ）。
    /// 直線定規は、a を通り a→b の向きの 3D の線（描く長さまで伸ばし、カメラの手前の側で切る）を画面へ写した直線で、押した点から画面で `SNAP_POINTS` の内に
    /// あるときだけ（面の点は要らない。画面で 1 点より縮んだ線は入れない）。平行線・パースは押した面の点を通る線（2 点のパースで b の向きが取れなければ 1 点目だけ）、
    /// 同心円は、a を通る軸（`up`）のまわりの、押した面の点の高さの円を 256 に分けて写した閉じた折れ線で、面の点が無ければ寄せない。
    pub fn view3d_ruler_constraint(
        &self,
        view: &CameraView,
        drawing: Option<LayerId>,
        press: DVec2,
        hit: Option<&SurfaceHit>,
    ) -> Option<Constraint> {
        let active = self.rulers_active_for(Place::View3d, drawing);
        if active.straight.is_empty() && active.special.is_none() {
            return None;
        }
        let reach = super::draw3d::reach(self);
        let lines: Vec<(DVec2, DVec2)> = active
            .straight
            .iter()
            .filter_map(|r| {
                let (a, b, _) = points(r.ruler)?;
                let (a, b) = visible_line(view, a.as_vec3(), b.as_vec3(), reach)?;
                Some((screen2(view.to_screen(a)?), screen2(view.to_screen(b)?)))
            })
            .collect();
        let at = |p: DVec2| egui::pos2(p.x as f32, p.y as f32);
        if let Some(c) = nearest_line(&lines, &at, at(press)) {
            return Some(c);
        }
        let special = active.special?;
        special_constraint(view, special.ruler, press, hit?)
    }
}

/// 平行線・同心円・パースの寄せ先（押した面の点 `hit` から。画面の点の空間）。
pub fn special_constraint(
    view: &CameraView,
    r: &Ruler,
    press: DVec2,
    hit: &SurfaceHit,
) -> Option<Constraint> {
    let (a, b, up) = points(r)?;
    let p = hit.position;
    // 押した点を通る 3D の線が画面に写る向き: 押した面の点から `toward` へ、画面で 64 点ほど進んだ点との差（透視の写しで直線は直線）
    let step = sg::world_per_point(view, p) * 64.0;
    let origin = view.to_screen(p)?;
    let direction_of = |toward: Vec3| -> Option<DVec2> {
        let toward = toward.try_normalize()?;
        let to = view
            .to_screen(p + toward * step)
            .or_else(|| view.to_screen(p - toward * step))?;
        (screen2(to) - screen2(origin)).try_normalize()
    };
    match r.kind {
        RulerKind::Parallel => Some(Constraint::Line {
            origin: press,
            direction: direction_of((b - a).as_vec3())?,
        }),
        RulerKind::Perspective => {
            // 消失点は画面の遠くの点にして渡す（消失点がカメラの後ろでも、押した点から消失点への向きの線は同じ）
            let far = |vanishing: DVec3| -> Option<DVec2> {
                Some(press + direction_of(vanishing.as_vec3() - p)? * 1000.0)
            };
            // b が取れない（押した面の点と重なる・向きが決まらない）ときは、1 点目だけで寄せる
            Some(Constraint::Perspective {
                start: press,
                a: far(a)?,
                b: if r.two_points { far(b) } else { None },
            })
        }
        RulerKind::Concentric => {
            let axis = up.as_vec3();
            let from_center = p - a.as_vec3();
            let radial = from_center - axis * from_center.dot(axis);
            let radius = radial.length();
            if radius.is_nan() || radius <= 1e-6 {
                return None;
            }
            let center = a.as_vec3() + axis * from_center.dot(axis);
            let u = radial / radius;
            let w = axis.cross(u);
            let ring: Vec<Option<DVec2>> = (0..=CIRCLE_SEGMENTS)
                .map(|k| {
                    let t = k as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
                    view.to_screen(center + (u * t.cos() + w * t.sin()) * radius)
                        .map(screen2)
                })
                .collect();
            let segments = ring
                .windows(2)
                .map(|w| Some((w[0]?, w[1]?)))
                .collect::<Vec<_>>();
            Constraint::polyline(segments, press)
        }
        RulerKind::Line | RulerKind::Symmetry => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::geometry::OrbitCamera;

    fn line(a: DVec3, b: DVec3, up: DVec3) -> Ruler {
        Ruler::model(RulerId(1), RulerKind::Line, a, b, up)
    }

    fn near(a: DVec3, b: DVec3) -> bool {
        a.distance(b) < 1e-5
    }

    fn pts(r: &Ruler) -> (DVec3, DVec3, DVec3) {
        points(r).unwrap()
    }

    #[test]
    fn the_frame_has_x_along_a_to_b_flat_to_up_y_up_and_z_by_the_cross_product() {
        // a→b が up と直角でない向き: X は up に直交する成分
        let r = line(DVec3::ZERO, DVec3::new(1.0, 1.0, 0.0), DVec3::Y);
        let q = frame(&r).unwrap();
        assert!((q * Vec3::X - Vec3::X).length() < 1e-6);
        assert!((q * Vec3::Y - Vec3::Y).length() < 1e-6);
        assert!((q * Vec3::Z - Vec3::Z).length() < 1e-6);
        let r = line(DVec3::ZERO, DVec3::new(0.0, 0.0, 2.0), DVec3::Y);
        let q = frame(&r).unwrap();
        assert!((q * Vec3::X - Vec3::Z).length() < 1e-6, "X = a→b");
        assert!(((q * Vec3::Z) - (q * Vec3::X).cross(q * Vec3::Y)).length() < 1e-6);
        // a→b が up と平行でも、枠は決まる（up に直交する向きが X）
        let r = line(DVec3::ZERO, DVec3::new(0.0, 3.0, 0.0), DVec3::Y);
        let q = frame(&r).unwrap();
        assert!((q * Vec3::Y - Vec3::Y).length() < 1e-6);
        assert!((q * Vec3::X).dot(Vec3::Y).abs() < 1e-6);
    }

    #[test]
    fn the_center_is_the_midpoint_of_two_point_rulers_and_a_otherwise() {
        let a = DVec3::new(1.0, 2.0, 3.0);
        let b = DVec3::new(3.0, 2.0, 3.0);
        for kind in [RulerKind::Line, RulerKind::Parallel] {
            let r = Ruler::model(RulerId(1), kind, a, b, DVec3::Y);
            assert!(near(center(&r).unwrap(), (a + b) * 0.5), "{kind:?}");
        }
        let mut p = Ruler::model(RulerId(1), RulerKind::Perspective, a, b, DVec3::Y);
        assert!(near(center(&p).unwrap(), a), "1 点");
        p.two_points = true;
        assert!(near(center(&p).unwrap(), (a + b) * 0.5), "2 点");
        for kind in [RulerKind::Concentric, RulerKind::Symmetry] {
            let r = Ruler::model(RulerId(1), kind, a, b, DVec3::Y);
            assert!(near(center(&r).unwrap(), a), "{kind:?}");
        }
        assert!(center(&Ruler::canvas(
            RulerId(1),
            RulerKind::Line,
            yolu_core::glam::DVec2::ZERO,
            yolu_core::glam::DVec2::X
        ))
        .is_none());
    }

    #[test]
    fn the_ends_are_a_and_b_for_two_point_rulers_and_only_b_for_the_radius_and_the_first_line() {
        let r = line(DVec3::ZERO, DVec3::X, DVec3::Y);
        assert_eq!(
            ends(&r).iter().map(|e| e.0).collect::<Vec<_>>(),
            [End::A, End::B]
        );
        for kind in [RulerKind::Concentric, RulerKind::Symmetry] {
            let r = Ruler::model(RulerId(1), kind, DVec3::ZERO, DVec3::X, DVec3::Y);
            assert_eq!(ends(&r).iter().map(|e| e.0).collect::<Vec<_>>(), [End::B]);
        }
        let mut p = Ruler::model(
            RulerId(1),
            RulerKind::Perspective,
            DVec3::ZERO,
            DVec3::X,
            DVec3::Y,
        );
        assert!(ends(&p).is_empty(), "1 点のパースは b を使わない");
        p.two_points = true;
        assert_eq!(ends(&p).len(), 2);
        let moved = with_end(&r, End::B, DVec3::new(0.0, 5.0, 0.0));
        assert!(near(pts(&moved).0, DVec3::ZERO) && near(pts(&moved).1, DVec3::new(0.0, 5.0, 0.0)));
    }

    #[test]
    fn a_gizmo_shape_without_a_change_leaves_the_ruler_as_it_is_and_a_move_adds_exactly() {
        let r = line(
            DVec3::new(-0.3, -0.2, -0.5),
            DVec3::new(0.3, 0.2, -0.5),
            DVec3::NEG_Z,
        );
        let shape = gizmo_shape(&r).unwrap();
        assert_eq!(shape.kind, sg::Kind::Anchor);
        assert_eq!(
            ruler_after_shape(&r, &shape),
            r,
            "変えていない形は値を変えない"
        );
        let mut moved = shape;
        moved.center[0] += 0.25;
        let next = ruler_after_shape(&r, &moved);
        let (a, b, up) = pts(&next);
        assert_eq!(
            a,
            DVec3::new(-0.3 + 0.25, -0.2, -0.5),
            "動かした分を f64 のまま足す"
        );
        assert_eq!(b, DVec3::new(0.3 + 0.25, 0.2, -0.5));
        assert_eq!(up, DVec3::NEG_Z);
    }

    #[test]
    fn a_gizmo_turn_rotates_a_b_and_up_around_the_center_keeping_lengths() {
        let r = line(
            DVec3::new(-0.3, -0.2, -0.5),
            DVec3::new(0.3, 0.2, -0.5),
            DVec3::NEG_Z,
        );
        let c = center(&r).unwrap();
        let mut turned = gizmo_shape(&r).unwrap();
        // 世界の Y の軸のまわりに 90 度
        let q = Quat::from_axis_angle(Vec3::Y, 90f32.to_radians()) * sg::rotation_of(&turned);
        turned.rotation = sg::euler_degrees(q);
        let next = ruler_after_shape(&r, &turned);
        let (a, b, up) = pts(&next);
        assert!(near(center(&next).unwrap(), c), "中心は動かない");
        assert!((a.distance(b) - pts(&r).0.distance(pts(&r).1)).abs() < 1e-5);
        let expected = d3(Quat::from_axis_angle(Vec3::Y, 90f32.to_radians()) * Vec3::NEG_Z);
        assert!(near(up, expected), "{up} {expected}");
    }

    #[test]
    fn g_r_s_values_apply_to_the_points_and_an_identity_value_changes_nothing() {
        let r = line(
            DVec3::new(-1.0, 0.0, 0.0),
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::Y,
        );
        let pivot = Vec3::ZERO;
        let q = Quat::IDENTITY;
        assert_eq!(ruler_after(&r, pivot, Value::Move(Vec3::ZERO), q), r);
        assert_eq!(
            ruler_after(
                &r,
                pivot,
                Value::Turn {
                    axis: Vec3::Z,
                    degrees: 0.0
                },
                q
            ),
            r
        );
        assert_eq!(ruler_after(&r, pivot, Value::Factor(Vec3::ONE), q), r);
        let moved = ruler_after(&r, pivot, Value::Move(Vec3::new(0.0, 2.0, 0.0)), q);
        assert_eq!(pts(&moved).0, DVec3::new(-1.0, 2.0, 0.0));
        let turned = ruler_after(
            &r,
            pivot,
            Value::Turn {
                axis: Vec3::Z,
                degrees: 90.0,
            },
            q,
        );
        assert!(near(pts(&turned).0, DVec3::new(0.0, -1.0, 0.0)));
        assert!(near(pts(&turned).2, DVec3::NEG_X), "向きも回る");
        let scaled = ruler_after(&r, pivot, Value::Factor(Vec3::splat(3.0)), q);
        assert!(near(pts(&scaled).1, DVec3::new(3.0, 0.0, 0.0)));
        assert!(near(pts(&scaled).2, DVec3::Y), "拡縮は向きを変えない");
    }

    fn view() -> CameraView {
        OrbitCamera::default().view(800.0, 600.0)
    }

    fn hit(p: Vec3) -> SurfaceHit {
        SurfaceHit {
            revision: 0,
            renderer: 0,
            material_slot: 0,
            material: 0,
            triangle: 0,
            position: p,
            normal: Vec3::Y,
            barycentric: Vec3::ZERO,
            uv: Vec2::ZERO,
            distance: 0.0,
        }
    }

    #[test]
    fn a_line_with_an_end_behind_the_camera_keeps_the_part_in_front_and_one_wholly_behind_is_dropped(
    ) {
        for orthographic in [false, true] {
            let mut camera = OrbitCamera::default();
            camera.set_orthographic(orthographic);
            let view = camera.view(800.0, 600.0);
            let forward = view.forward;
            // a はカメラの後ろ、b は注視点の側（前）
            let behind = view.position - forward * 1.5;
            let front = camera.target;
            let (p0, p1) = visible_line(&view, behind, front, 1.0).expect("手前の部分が残る");
            for p in [p0, p1] {
                assert!(view.to_screen(p).is_some(), "{orthographic}: 写せる");
            }
            // 切ったあとの 2 点は、元の直線の上
            let d = (front - behind).normalize();
            for p in [p0, p1] {
                assert!((p - behind).cross(d).length() < 1e-3, "{orthographic}");
            }
            // 両端が前でも、そのまま（延ばした線の両端）
            let (q0, q1) =
                visible_line(&view, front, front + view.rotation * Vec3::X, 1.0).unwrap();
            assert!(q0.distance(q1) > 7.0, "{orthographic}: 延ばした長さ");
            // 全部がカメラの後ろ（カメラの面に平行な線）なら None
            let far_behind = view.position - forward * (10.0 + view.near.abs());
            assert!(
                visible_line(&view, far_behind, far_behind + view.rotation * Vec3::X, 1.0)
                    .is_none(),
                "{orthographic}"
            );
        }
    }

    #[test]
    fn a_two_point_perspective_whose_second_direction_is_missing_snaps_to_the_first_point() {
        let view = view();
        let r = {
            let mut r = Ruler::model(
                RulerId(1),
                RulerKind::Perspective,
                DVec3::new(0.0, 2.0, 0.0),
                // b は押した面の点と同じ所（向きが決まらない）
                DVec3::new(0.5, 0.0, 0.0),
                DVec3::Y,
            );
            r.two_points = true;
            r
        };
        let p = Vec3::new(0.5, 0.0, 0.0);
        let press = screen2(view.to_screen(p).unwrap());
        let c = special_constraint(&view, &r, press, &hit(p)).expect("1 点目だけで寄せる");
        let Constraint::Perspective { a, b, .. } = c else {
            panic!("{c:?}");
        };
        assert_ne!(a, press, "1 点目の向きは取れる");
        assert!(b.is_none(), "2 点目は取れないので使わない");
        // 2 点目が取れるときは両方
        let q = Vec3::new(0.2, 0.0, 0.3);
        let press = screen2(view.to_screen(q).unwrap());
        let Some(Constraint::Perspective { b, .. }) = special_constraint(&view, &r, press, &hit(q))
        else {
            panic!();
        };
        assert!(b.is_some());
    }

    #[test]
    fn a_move_only_gizmo_drag_whose_euler_angles_differ_by_rounding_still_adds_exactly() {
        let r = line(
            DVec3::new(-0.3, -0.2, -0.5),
            DVec3::new(0.3, 0.2, -0.5),
            DVec3::NEG_Z,
        );
        let mut shape = gizmo_shape(&r).unwrap();
        // 毎フレーム今の定規から作り直すオイラー角は、f32 の丸めの分だけ違いうる（ここでは境界の ±180 をまたぐ表し方の違いと、小さな差）
        shape.rotation[1] += 2e-6;
        shape.center[0] += 0.25;
        let next = ruler_after_shape(&r, &shape);
        let (a, b, up) = pts(&next);
        assert_eq!(
            a,
            DVec3::new(-0.3 + 0.25, -0.2, -0.5),
            "f32 の回す道へ入らない"
        );
        assert_eq!(b, DVec3::new(0.3 + 0.25, 0.2, -0.5));
        assert_eq!(up, DVec3::NEG_Z);
        // はっきり回したら回す道
        let mut turned = gizmo_shape(&r).unwrap();
        turned.rotation[1] += 5.0;
        assert_ne!(pts(&ruler_after_shape(&r, &turned)).2, DVec3::NEG_Z);
    }

    #[test]
    fn a_gizmo_value_out_of_range_is_reported_and_leaves_the_ruler_alone() {
        let mut app = AppState::new(64, 64);
        let id = app
            .ruler_create(line(DVec3::ZERO, DVec3::X, DVec3::Y))
            .unwrap();
        let layer = app.selected_layer.unwrap();
        let current = crate::objects::ruler_of(&app, layer, id).unwrap();
        let mut shape = gizmo_shape(&current).unwrap();
        let steps = app.doc.undo_count();
        shape.center[0] = 5e6; // モデルの空間の範囲（±1e6）の外
        let result =
            crate::fillfx::gizmo::write_shape(&mut app, Target::Ruler(layer, id), shape, false);
        assert!(result.is_none());
        assert_eq!(app.doc.undo_count(), steps);
        assert_eq!(crate::objects::ruler_of(&app, layer, id), Some(current));
        assert!(
            app.message.contains("定規の値が範囲外です"),
            "{}",
            app.message
        );
    }

    #[test]
    fn grabbing_an_end_square_off_center_keeps_the_offset_to_its_center() {
        let view = view();
        let r = line(
            DVec3::new(-0.3, 0.0, 0.0),
            DVec3::new(0.3, 0.0, 0.0),
            DVec3::Y,
        );
        let center_of_b = view.to_screen(Vec3::new(0.3, 0.0, 0.0)).unwrap();
        let pressed = center_of_b + Vec2::new(4.0, -3.0);
        let offset = |handle| {
            let mut app = AppState::new(64, 64);
            let id = app.ruler_create(r.clone()).unwrap();
            let layer = app.selected_layer.unwrap();
            grab_offset(&app, &view, Target::Ruler(layer, id), handle, pressed)
        };
        let o = offset(sg::Handle::EndB);
        assert!((o - Vec2::new(-4.0, 3.0)).length() < 1e-3, "{o}");
        assert_eq!(
            offset(sg::Handle::MoveX),
            Vec2::ZERO,
            "端のつまみでなければ 0"
        );
    }

    #[test]
    fn a_pulled_symmetry_plane_holds_the_drawn_line_and_the_sight_and_shift_snaps_its_normal_to_an_axis(
    ) {
        let view = view();
        let p = Vec3::new(0.1, 0.2, 0.0);
        let sight = {
            let s = view.sight(p);
            s.to / s.distance
        };
        // カメラに向いた面の上の、斜めの動き
        let to = p + view.rotation * Vec3::new(0.3, 0.1, 0.0);
        let (a, b, up) = symmetry_from_drag(&view, p, to, false).unwrap();
        assert!(near(a, d3(p)));
        let d = (b - a).normalize();
        let normal = up.cross(d).normalize();
        assert!(normal.dot(sight.as_dvec3()).abs() < 1e-5, "視線を含む");
        assert!(
            normal.dot((to - p).as_dvec3().normalize()).abs() < 1e-5,
            "引いた線を含む"
        );
        assert!(
            ((b - a).length() as f32 - (to - p).length()).abs() < 1e-5,
            "長さは引いた長さ"
        );
        let (_, b2, up2) = symmetry_from_drag(&view, p, to, true).unwrap();
        let n2 = up2.cross((b2 - a).normalize()).normalize();
        assert!(
            [DVec3::X, DVec3::Y, DVec3::Z]
                .iter()
                .any(|axis| n2.dot(*axis).abs() > 1.0 - 1e-6),
            "法線はモデルの軸: {n2}"
        );
        // 引いた長さが 0 なら作れない
        assert!(symmetry_from_drag(&view, p, p, false).is_none());
    }

    #[test]
    fn a_concentric_constraint_is_a_closed_polyline_of_the_projected_circle_and_none_on_the_axis() {
        let view = view();
        let r = Ruler::model(
            RulerId(1),
            RulerKind::Concentric,
            DVec3::ZERO,
            DVec3::X,
            DVec3::Y,
        );
        let p = Vec3::new(0.5, 0.0, 0.0);
        let press = screen2(view.to_screen(p).unwrap());
        let mut c = special_constraint(&view, &r, press, &hit(p)).expect("円");
        assert!(matches!(c, Constraint::Polyline { .. }));
        // 押した点を通る円（中心 0・半径 0.5・軸 Y）を写した折れ線の上へ寄る
        let ring: Vec<DVec2> = (0..=CIRCLE_SEGMENTS)
            .map(|k| {
                let t = k as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
                screen2(
                    view.to_screen(Vec3::new(t.cos(), 0.0, t.sin()) * 0.5)
                        .unwrap(),
                )
            })
            .collect();
        let sq = screen2(view.to_screen(Vec3::new(0.0, 0.0, 0.5)).unwrap());
        for pointer in [sq + DVec2::new(0.0, 60.0), sq + DVec2::new(-90.0, -40.0)] {
            let got = c.project(pointer);
            let on_ring = ring
                .windows(2)
                .map(|w| {
                    let ab = w[1] - w[0];
                    let t = ((got - w[0]).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
                    (w[0] + ab * t).distance(got)
                })
                .fold(f64::INFINITY, f64::min);
            assert!(on_ring < 1e-6, "写した円の上: {got} {on_ring}");
            assert!(got.distance(pointer) < pointer.distance(sq), "近づいた");
        }
        // 軸の上で押したら円が作れない
        assert!(special_constraint(&view, &r, press, &hit(Vec3::new(0.0, 0.3, 0.0))).is_none());
    }
}
