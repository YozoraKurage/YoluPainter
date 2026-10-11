//! パスのツールの 3D ビュー: 入力（押す・動く・離す・ペン）と、パスの線・点の重ね表示。
//!
//! 押した所に点があれば掴んで選び、曲線の上なら、その区間に（ポインタの下の面の点を）差し込み、どちらでもなければ終わりに足す。
//! 面の点は、見せる形（隠したマテリアルを除いた形）で当てて、保存と同じ受けたままの形の三角形の番号に直す。描くテクスチャセット
//! のマテリアルの面にだけ置き、ほかのセットの面・モデルの外は断る。モデルに遮られる点は薄く出し、掴まない。
//! パスが別のモデルで描かれている（指紋が違う）あいだは出さず、編集もしない。

use egui::{CursorIcon, Painter, Pos2, Rect};
use yolu_core::geometry::{pick, CameraView, SurfaceGeometry};
use yolu_core::glam::{DMat3, DVec3, Mat4, Vec2, Vec3, Vec4};
use yolu_core::paths::{point_position, SurfacePath};
use yolu_core::LayerPath;

use super::canvas::{
    draw_curve, draw_handle, draw_insert_ring, draw_marker, draw_other_curve, draw_rect,
    shown_tangents, MarkerStyle,
};
use super::curve::{nearest_point, nearest_segment, sample_screen, P3};
use super::edit::{self, Place, PointOp, Pt, TangentValue};
use super::{
    HandleSide, Hover, PathAction, PenDown, PointDrag, PointRef, RectDrag, SurfaceCtx,
    DOUBLE_CLICK, GRAB_RADIUS,
};
use crate::notice::Source;
use crate::state::{AppState, StrokeSource};
use crate::view3d::input::camera_view;

/// 標本の間隔（画面の点）。
const STEP: f32 = 6.0;
/// 遮られるかを調べる点の数の上限（これより多いときは調べず、全部見えるものとして出す）。
const OCCLUSION_LIMIT: usize = 256;

/// 世界の点を表示域の画面の点にする（行列は 1 回だけ作る）。
struct Projector {
    vp: Mat4,
    rect: Rect,
    width: f32,
    height: f32,
    /// 正投影（近い面より手前の点は写さない。`CameraView::to_screen` と同じ）。
    orthographic: bool,
}

impl Projector {
    fn new(view: &CameraView, rect: Rect) -> Projector {
        Projector {
            vp: view.view_projection(),
            rect,
            width: view.width,
            height: view.height,
            orthographic: view.is_orthographic(),
        }
    }

    fn project(&self, world: Vec3) -> Option<Pos2> {
        let clip = self.vp * Vec4::new(world.x, world.y, world.z, 1.0);
        // 正投影の切り取りの深度は、近い面で 0（その手前は負）
        if clip.w <= 1e-12 || (self.orthographic && clip.z < 0.0) {
            return None;
        }
        let (x, y) = (clip.x / clip.w, clip.y / clip.w);
        Some(Pos2::new(
            self.rect.left() + (x + 1.0) * 0.5 * self.width,
            self.rect.top() + (1.0 - y) * 0.5 * self.height,
        ))
    }
}

fn local(rect: Rect, p: Pos2) -> Vec2 {
    Vec2::new(p.x - rect.left(), p.y - rect.top())
}

/// 制御点の 3D の位置（隠したマテリアルの点・三角形の無い点は None）。
fn positions(
    app: &AppState,
    path: &SurfacePath,
    g: &SurfaceGeometry,
    preview: Option<PointDrag>,
) -> Vec<Option<Vec3>> {
    let mut out: Vec<Option<Vec3>> = path
        .points
        .iter()
        .map(|p| {
            let t = g.triangles().get(p.triangle as usize)?;
            if app.view3d.is_material_hidden(t.material) || app.view3d.is_face_hidden(p.triangle) {
                return None;
            }
            point_position(g, p).map(|(position, _)| position)
        })
        .collect();
    // ドラッグ中の点は今の置き場所（閉じたパスの始め・終わりは一緒に）
    if let Some(d) = preview {
        if let (Some(Place::Surface { triangle, u, v }), true) = (d.target, d.index < out.len()) {
            if let Some(t) = g.triangles().get(triangle as usize) {
                let at = t.a * (1.0 - u - v) as f32 + t.b * u as f32 + t.c * v as f32;
                let n = out.len();
                out[d.index] = Some(at);
                if edit::is_closed(&path.points) && (d.index == 0 || d.index == n - 1) {
                    out[0] = Some(at);
                    out[n - 1] = Some(at);
                }
            }
        }
    }
    out
}

/// モデル（見せる形）に遮られる点。調べない（数が多い）ときは全部 false。
fn occluded(app: &AppState, view: &CameraView, positions: &[Option<Vec3>]) -> Vec<bool> {
    let Some(shown) = app.view3d.model.as_ref() else {
        return vec![false; positions.len()];
    };
    if positions.len() > OCCLUSION_LIMIT {
        return vec![false; positions.len()];
    }
    let scale = shown.geometry.brush_scale();
    positions
        .iter()
        .map(|p| {
            let Some(p) = p else { return false };
            let sight = view.sight(*p);
            let dist = sight.distance;
            if dist < 1e-6 {
                return false;
            }
            shown
                .geometry
                .raycast(sight.ray(), true, dist - dist * 0.003 - scale * 1e-4)
                .is_some()
        })
        .collect()
}

/// 三角形の辺 2 本と、長さが辺ほどの法線の 3 本の列（モデルの空間）。潰れた三角形は None。
fn frame(g: &SurfaceGeometry, triangle: u32) -> Option<DMat3> {
    let t = g.triangles().get(triangle as usize)?;
    let e1 = (t.b - t.a).as_dvec3();
    let e2 = (t.c - t.a).as_dvec3();
    let n = e1.cross(e2);
    let area = n.length();
    if area.is_nan() || area <= 1e-20 {
        return None;
    }
    Some(DMat3::from_cols(e1, e2, n / area.sqrt()))
}

/// 休みの形の向きを、見ている形（ポーズを付けた形）の向きへ写す行列（点の三角形の変形）。同じ形・潰れた三角形は単位行列。
pub(super) fn rest_to_shown(
    rest: &SurfaceGeometry,
    shown: &SurfaceGeometry,
    triangle: u32,
) -> DMat3 {
    if std::ptr::eq(rest, shown) {
        return DMat3::IDENTITY;
    }
    match (frame(rest, triangle), frame(shown, triangle)) {
        (Some(r), Some(s)) if r.determinant().abs() > 1e-30 => s * r.inverse(),
        _ => DMat3::IDENTITY,
    }
}

struct Scene {
    proj: Projector,
    positions: Vec<Option<Vec3>>,
    /// 見ている形での点ごとの接線（取っ手は見ている形の向き）。
    tangents: Vec<TangentValue>,
    hidden_behind: Vec<bool>,
}

fn scene(
    app: &AppState,
    rect: Rect,
    path: &SurfacePath,
    g: &SurfaceGeometry,
    preview: Option<PointDrag>,
) -> Scene {
    let view = camera_view(app, rect);
    let positions = positions(app, path, g, preview);
    // 取っ手は休みの形の空間で持つので、点の三角形の変形で見ている形へ写す（ドラッグ中の取っ手も）
    let rest = app.view3d.full_model().map(|m| m.rest_geometry().clone());
    let now: Vec<TangentValue> = path.points.iter().map(Pt::tangent_value).collect();
    let now = match (app.selected_layer, preview) {
        (Some(layer), Some(_)) => shown_tangents(app, &now, layer, path.id),
        _ => now,
    };
    let tangents = path
        .points
        .iter()
        .zip(now)
        .map(|(p, t)| match (t, &rest) {
            (TangentValue::Handles { incoming, outgoing }, Some(rest)) => {
                let m = rest_to_shown(rest, g, p.triangle);
                TangentValue::Handles {
                    incoming: (m * DVec3::from_array(incoming)).to_array(),
                    outgoing: (m * DVec3::from_array(outgoing)).to_array(),
                }
            }
            (t, _) => t,
        })
        .collect();
    Scene {
        proj: Projector::new(&view, rect),
        hidden_behind: occluded(app, &view, &positions),
        positions,
        tangents,
    }
}

impl Scene {
    fn screen(&self) -> Vec<Option<Pos2>> {
        self.positions
            .iter()
            .map(|p| p.and_then(|p| self.proj.project(p)))
            .collect()
    }

    /// 曲線の標本（全部の点が 3D にあるときだけ。無ければ空）。
    fn samples(&self) -> Vec<Vec<Option<Pos2>>> {
        if self.positions.len() < 2 || self.positions.iter().any(Option::is_none) {
            return Vec::new();
        }
        let pts: Vec<P3> = self
            .positions
            .iter()
            .flatten()
            .map(|p| [p.x as f64, p.y as f64, p.z as f64])
            .collect();
        sample_screen(
            &pts,
            &self.tangents,
            &|p: P3| {
                self.proj
                    .project(Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32))
            },
            STEP,
        )
    }

    /// 選んでいる点の取っ手（側、点の画面の点、先の画面の点）。取っ手の点だけ、長さ 0 の側は出さない。
    fn handle_ends(&self, index: usize) -> Vec<(HandleSide, Pos2, Pos2)> {
        let (Some(Some(p)), Some(TangentValue::Handles { incoming, outgoing })) =
            (self.positions.get(index), self.tangents.get(index))
        else {
            return Vec::new();
        };
        let Some(at) = self.proj.project(*p) else {
            return Vec::new();
        };
        [(HandleSide::In, incoming), (HandleSide::Out, outgoing)]
            .into_iter()
            .filter(|(_, h)| h.iter().any(|v| *v != 0.0))
            .filter_map(|(side, h)| {
                let end = *p + DVec3::from_array(*h).as_vec3();
                Some((side, at, self.proj.project(end)?))
            })
            .collect()
    }

    fn hover(&self, pointer: Pos2, selected: Option<usize>) -> Hover {
        if let Some(i) = selected {
            for (side, _, end) in self.handle_ends(i) {
                if end.distance(pointer) <= GRAB_RADIUS {
                    return Hover::Handle(i, side);
                }
            }
        }
        // 遮られている点は掴まない
        let grabbable: Vec<Option<Pos2>> = self
            .screen()
            .into_iter()
            .zip(&self.hidden_behind)
            .map(|(p, hidden)| if *hidden { None } else { p })
            .collect();
        if let Some(i) = nearest_point(&grabbable, pointer, GRAB_RADIUS) {
            return Hover::Point(i);
        }
        match nearest_segment(&self.samples(), pointer, GRAB_RADIUS) {
            Some((s, at)) => Hover::Segment(s, at),
            None => Hover::None,
        }
    }
}

/// 今のレイヤーの 3D のパスと、受けたままの形（指紋が合うときだけ）。
fn current(app: &AppState) -> Option<(yolu_core::LayerId, &SurfacePath, &SurfaceGeometry)> {
    let (layer, LayerPath::Surface(path)) = app.path_layer()? else {
        return None;
    };
    let model = app.view3d.full_model()?;
    (*app.path_fingerprint(&model.geometry) == path.model_fingerprint).then_some((
        layer,
        path,
        &*model.geometry,
    ))
}

/// 選んでいるレイヤーの、編集していないほかの 3D のパス（一覧の上のものから。今のモデルで描かれたものだけ）と、受けたままの形。
fn others(app: &AppState) -> Vec<(u128, &SurfacePath)> {
    let Some(model) = app.view3d.full_model() else {
        return Vec::new();
    };
    let print = app.path_fingerprint(&model.geometry);
    let active = app.path_layer().map(|(_, p)| p.id());
    app.path_entries()
        .iter()
        .rev()
        .filter_map(|e| match &e.path {
            LayerPath::Surface(s) if Some(s.id) != active && *print == s.model_fingerprint => {
                Some((s.id, s))
            }
            _ => None,
        })
        .collect()
}

/// ポインタの下の、編集していないほかのパス（点か曲線）。
fn other_under(app: &AppState, rect: Rect, pointer: Pos2) -> Option<u128> {
    let model = app.view3d.full_model()?;
    others(app)
        .into_iter()
        .find(|(_, p)| {
            scene(app, rect, p, &model.geometry, None).hover(pointer, None) != Hover::None
        })
        .map(|(id, _)| id)
}

/// ポインタの下の面の点（描くテクスチャセットのマテリアルの面だけ）。置けなければ理由。
fn pick_place(app: &AppState, rect: Rect, at: Pos2, ctx: &SurfaceCtx) -> Result<Place, String> {
    let lang = app.lang;
    let Some(shown) = app.view3d.model.clone() else {
        return Err(lang.pick("モデルがありません", "No model").into());
    };
    let view = camera_view(app, rect);
    let Some(hit) = pick(&shown.geometry, &view, local(rect, at)) else {
        return Err(lang
            .pick("モデルの上ではありません", "Not on the model")
            .into());
    };
    if hit.material != ctx.material {
        let name = shown.material_name(hit.material as usize, lang);
        return Err(lang.pick(
            format!("ほかのテクスチャセット（{name}）の面です。"),
            format!("Surface of another texture set ({name})."),
        ));
    }
    let Some(triangle) = app.view3d.full_triangle(hit.triangle) else {
        return Err(lang
            .pick("モデルの上ではありません", "Not on the model")
            .into());
    };
    let (mut u, mut v) = (hit.barycentric.y as f64, hit.barycentric.z as f64);
    let sum = u + v;
    if sum > 1.0 {
        u /= sum;
        v /= sum;
    }
    Ok(Place::Surface {
        triangle,
        u: u.max(0.0),
        v: v.max(0.0),
    })
}

/// 押した（ペン・マウス）。
pub fn press(app: &mut AppState, rect: Rect, at: Pos2, source: StrokeSource) {
    if !app.tool.is_path() || app.path.drag.is_some() {
        return;
    }
    let Some((layer, existing)) = app.path_target(Some(true)) else {
        return;
    };
    let ctx = match app.path_surface_ctx() {
        Ok(c) => c,
        Err(m) => {
            app.refuse(Source::Path, m);
            return;
        }
    };
    if let Some(LayerPath::Surface(path)) = &existing {
        if path.model_fingerprint != *ctx.fingerprint {
            app.refuse(
                Source::Path,
                app.lang.pick(
                    "別のモデルで描かれたパスです",
                    "The path was drawn on another model",
                ),
            );
            return;
        }
        // Shift を押して押したら、点を矩形で選ぶ
        if app.path.input.shift {
            app.path.rect = Some(RectDrag {
                source,
                surface: true,
                start: at,
                now: at,
            });
            return;
        }
        let s = scene(app, rect, path, &ctx.model.geometry, None);
        let selected = app.path_selected_index();
        let hover = s.hover(at, selected);
        // 編集していないパスの上を押したら、そのパスを選ぶ（点は置かない）
        if hover == Hover::None {
            if let Some(id) = other_under(app, rect, at) {
                app.path_apply(PathAction::SelectPath(Some(id)));
                return;
            }
        }
        match hover {
            Hover::Handle(index, side) => {
                app.path.drag = Some(PointDrag {
                    layer,
                    path: path.id,
                    index,
                    source,
                    surface: true,
                    start: at,
                    moved: 0.0,
                    target: None,
                    handle: Some(side),
                    vector: None,
                });
                return;
            }
            Hover::Point(index) => {
                let point = PointRef {
                    layer,
                    path: path.id,
                    index,
                };
                // 同じ点のダブルクリックは、角と滑らかの切り替え（ドラッグは始めない）
                if let (Some(now), Some((t, last))) = (app.path.input.now, app.path.last_press) {
                    if last == point && now - t <= DOUBLE_CLICK {
                        app.path.last_press = None;
                        app.path.selected = Some(point);
                        app.path_apply(PathAction::Point(PointOp::ToggleCorner(index)));
                        return;
                    }
                }
                app.path.last_press = app.path.input.now.map(|now| (now, point));
                app.path.selected = Some(point);
                app.path.drag = Some(PointDrag {
                    layer,
                    path: path.id,
                    index,
                    source,
                    surface: true,
                    start: at,
                    moved: 0.0,
                    target: None,
                    handle: None,
                    vector: None,
                });
                return;
            }
            Hover::Segment(segment, _) => {
                match pick_place(app, rect, at, &ctx) {
                    Ok(place) => {
                        app.path_apply(PathAction::Point(PointOp::Insert { segment, place }))
                    }
                    Err(m) => app.refuse(Source::Path, m),
                }
                return;
            }
            Hover::None => {}
        }
    }
    if existing.is_none() {
        if let Some(id) = other_under(app, rect, at) {
            app.path_apply(PathAction::SelectPath(Some(id)));
            return;
        }
    }
    match pick_place(app, rect, at, &ctx) {
        Ok(place) => app.path_apply(PathAction::Point(PointOp::Add(place))),
        Err(m) => app.refuse(Source::Path, m),
    }
}

/// 動いた。面の上に置けない所では、前の置き場所のまま。
pub fn moved(app: &mut AppState, rect: Rect, at: Pos2, source: StrokeSource) {
    if let Some(r) = app
        .path
        .rect
        .as_mut()
        .filter(|r| r.source == source && r.surface)
    {
        r.now = at;
        return;
    }
    if app
        .path
        .drag
        .is_none_or(|d| d.source != source || !d.surface)
    {
        return;
    }
    if app.path.drag.is_some_and(|d| d.handle.is_some()) {
        let vector = handle_vector(app, rect, at);
        if let Some(d) = app.path.drag.as_mut() {
            d.moved = d.moved.max(at.distance(d.start));
            if vector.is_some() {
                d.vector = vector;
            }
        }
        return;
    }
    let place = app
        .path_surface_ctx()
        .ok()
        .and_then(|ctx| pick_place(app, rect, at, &ctx).ok());
    if let Some(d) = app.path.drag.as_mut() {
        d.moved = d.moved.max(at.distance(d.start));
        if place.is_some() {
            d.target = place;
        }
    }
}

/// 取っ手のドラッグの新しい向き（休みの形のモデルの空間）: ポインタのレイと、点を通り見ている形の面の法線に直交する面との
/// 交わりから点までを、休みの形へ戻す（取っ手は点の面に沿って動く）。面とレイが平行なら None。
fn handle_vector(app: &AppState, rect: Rect, at: Pos2) -> Option<[f64; 3]> {
    let d = app.path.drag?;
    let (_, path, g) = current(app)?;
    let p = path.points.get(d.index)?;
    let (position, normal) = point_position(g, p)?;
    let view = camera_view(app, rect);
    let ray = view.ray(local(rect, at));
    let denom = ray.direction().dot(normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (position - ray.origin()).dot(normal) / denom;
    if t.is_nan() || t <= 0.0 {
        return None;
    }
    let shown = ray.point(t) - position;
    let rest = app.view3d.full_model()?.rest_geometry().clone();
    let m = rest_to_shown(&rest, g, p.triangle);
    let back = if m.determinant().abs() > 1e-30 {
        m.inverse() * shown.as_dvec3()
    } else {
        shown.as_dvec3()
    };
    Some(back.to_array())
}

/// 矩形の選びを終える: 矩形に入る、遮られていない点を選ぶ。
fn finish_rect(app: &mut AppState, rect: Rect) {
    let Some(r) = app.path.rect.take() else {
        return;
    };
    let Some((layer, path, g)) = current(app) else {
        return;
    };
    let s = scene(app, rect, path, g, None);
    let area = Rect::from_two_pos(r.start, r.now);
    let inside: Vec<usize> = s
        .screen()
        .iter()
        .enumerate()
        .filter(|(i, p)| !s.hidden_behind[*i] && p.is_some_and(|p| area.contains(p)))
        .map(|(i, _)| i)
        .collect();
    let a = super::ActivePath {
        layer,
        path: path.id,
    };
    app.path.selected = None;
    app.path.marked = (!inside.is_empty()).then_some((a, inside));
}

/// 離した。
pub fn release(app: &mut AppState, rect: Rect, at: Pos2, source: StrokeSource) {
    if app
        .path
        .rect
        .is_some_and(|r| r.source == source && r.surface)
    {
        moved(app, rect, at, source);
        finish_rect(app, rect);
        return;
    }
    if app
        .path
        .drag
        .is_none_or(|d| d.source != source || !d.surface)
    {
        return;
    }
    moved(app, rect, at, source);
    app.path_finish_drag();
}

/// ペン（Windows Ink）の 1 点。触れた・動いた・離したを、押す・動く・離すにする。`usable` は新しく押してよい所か。2D のキャンバスが
/// 触れているペンは扱わない（同じ列が両方のビューに渡るので、離したサンプルで相手のドラッグを取り残さないため）。
pub fn pen_sample(
    app: &mut AppState,
    rect: Rect,
    at: Pos2,
    pointer_id: u32,
    contact: bool,
    usable: bool,
) {
    let source = StrokeSource::Pen(pointer_id);
    match (contact, app.path.pen_down) {
        (true, None) if usable => {
            app.path.pen_down = Some(PenDown {
                id: pointer_id,
                surface: true,
            });
            press(app, rect, at, source);
        }
        (true, Some(p)) if p.id == pointer_id && p.surface => moved(app, rect, at, source),
        (false, Some(p)) if p.id == pointer_id && p.surface => {
            app.path.pen_down = None;
            release(app, rect, at, source);
        }
        _ => {}
    }
}

/// ポインタの形（点の上は掴む手、曲線の上は足す形）。
pub fn cursor_icon(app: &AppState, rect: Rect, pointer: Pos2) -> CursorIcon {
    if app.path.drag.is_some() {
        return CursorIcon::Grabbing;
    }
    let active = match current(app) {
        Some((_, path, g)) => {
            scene(app, rect, path, g, None).hover(pointer, app.path_selected_index())
        }
        None => Hover::None,
    };
    match active {
        Hover::Point(_) | Hover::Handle(..) => CursorIcon::Grab,
        Hover::Segment(..) => CursorIcon::Copy,
        Hover::None if other_under(app, rect, pointer).is_some() => CursorIcon::PointingHand,
        Hover::None => CursorIcon::Crosshair,
    }
}

/// 選んでいるレイヤーの 3D のパスの線と点をモデルの上に重ねる（パスのツールのあいだだけ）。
pub fn paint_overlay(painter: &Painter, app: &AppState, rect: Rect, pointer: Option<Pos2>) {
    if !app.tool.is_path() {
        return;
    }
    // 編集していないパスは薄い線だけ（押すと選ぶ）
    if let Some(model) = app.view3d.full_model() {
        for (_, other) in others(app) {
            draw_other_curve(
                painter,
                &scene(app, rect, other, &model.geometry, None).samples(),
            );
        }
    }
    let Some((layer, path, g)) = current(app) else {
        return;
    };
    let drag = app
        .path
        .drag
        .filter(|d| d.surface && d.layer == layer && d.path == path.id);
    let s = scene(app, rect, path, g, drag);
    let samples = s.samples();
    draw_curve(painter, &samples);
    let selected = app.path_selected_index();
    let hover = match (pointer, drag) {
        (Some(p), None) => s.hover(p, selected),
        _ => Hover::None,
    };
    // 選んでいる点の取っ手（線と先の丸）
    if let Some(i) = selected {
        for (side, from, end) in s.handle_ends(i) {
            let hot = hover == Hover::Handle(i, side)
                || drag.is_some_and(|d| d.handle == Some(side) && d.index == i);
            draw_handle(painter, from, end, hot);
        }
    }
    let marked = app.path_selected_indices();
    let closed = edit::is_closed(&path.points);
    let n = path.points.len();
    let screen = s.screen();
    for (i, at) in screen.iter().enumerate() {
        let Some(at) = at else { continue };
        if closed && i == n - 1 {
            continue;
        }
        if s.positions[i].is_none() {
            continue;
        }
        let is_selected = marked.contains(&i) || (closed && i == 0 && marked.contains(&(n - 1)));
        let is_hover =
            hover == Hover::Point(i) || (closed && i == 0 && hover == Hover::Point(n - 1));
        let style = if is_selected {
            MarkerStyle::Selected
        } else if s.hidden_behind[i] {
            MarkerStyle::Dim
        } else if is_hover {
            MarkerStyle::Hover
        } else {
            MarkerStyle::Plain
        };
        draw_marker(painter, *at, style);
    }
    if let Hover::Segment(_, at) = hover {
        draw_insert_ring(painter, at);
    }
    if let Some(r) = app.path.rect.filter(|r| r.surface) {
        draw_rect(painter, &r);
    }
}
