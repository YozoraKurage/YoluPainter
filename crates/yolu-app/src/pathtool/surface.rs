//! パスの道具の 3D ビュー: 入力（押す・動く・離す・ペン）と、パスの線・点の重ね表示。
//!
//! 押した所に点があれば掴んで選び、曲線の上なら、その区間に（ポインタの下の面の点を）差し込み、どちらでもなければ終わりに足す。
//! 面の点は、見せる形（隠したマテリアルを除いた形）で当てて、保存と同じ受けたままの形の三角形の番号に直す。描くテクスチャセット
//! のマテリアルの面にだけ置き、ほかのセットの面・モデルの外は断る。モデルに遮られる点は薄く出し、掴まない。
//! パスが別のモデルで描かれている（指紋が違う）あいだは出さず、編集もしない。

use egui::{CursorIcon, Painter, Pos2, Rect};
use yolu_core::geometry::{pick, CameraView, Ray, SurfaceGeometry};
use yolu_core::glam::{Mat4, Vec2, Vec3, Vec4};
use yolu_core::paths::{point_position, SurfacePath};
use yolu_core::LayerPath;

use super::canvas::{draw_curve, draw_insert_ring, draw_marker, MarkerStyle};
use super::curve::{nearest_point, nearest_segment, sample_screen, P3};
use super::edit::{self, Place, PointOp};
use super::{Hover, PathAction, PenDown, PointDrag, PointRef, SurfaceCtx, GRAB_RADIUS};
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
}

impl Projector {
    fn new(view: &CameraView, rect: Rect) -> Projector {
        Projector {
            vp: view.view_projection(),
            rect,
            width: view.width,
            height: view.height,
        }
    }

    fn project(&self, world: Vec3) -> Option<Pos2> {
        let clip = self.vp * Vec4::new(world.x, world.y, world.z, 1.0);
        if clip.w <= 1e-12 {
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
            let to = *p - view.position;
            let dist = to.length();
            if dist < 1e-6 {
                return false;
            }
            shown
                .geometry
                .raycast(
                    Ray::new(view.position, to),
                    true,
                    dist - dist * 0.003 - scale * 1e-4,
                )
                .is_some()
        })
        .collect()
}

struct Scene {
    proj: Projector,
    positions: Vec<Option<Vec3>>,
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
    Scene {
        proj: Projector::new(&view, rect),
        hidden_behind: occluded(app, &view, &positions),
        positions,
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
            &|p| {
                self.proj
                    .project(Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32))
            },
            STEP,
        )
    }

    fn hover(&self, pointer: Pos2) -> Hover {
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

/// 今の層の 3D のパスと、受けたままの形（指紋が合うときだけ）。
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
        let s = scene(app, rect, path, &ctx.model.geometry, None);
        match s.hover(at) {
            Hover::Point(index) => {
                app.path.selected = Some(PointRef {
                    layer,
                    path: path.id,
                    index,
                });
                app.path.drag = Some(PointDrag {
                    layer,
                    path: path.id,
                    index,
                    source,
                    surface: true,
                    start: at,
                    moved: 0.0,
                    target: None,
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
    match pick_place(app, rect, at, &ctx) {
        Ok(place) => app.path_apply(PathAction::Point(PointOp::Add(place))),
        Err(m) => app.refuse(Source::Path, m),
    }
}

/// 動いた。面の上に置けない所では、前の置き場所のまま。
pub fn moved(app: &mut AppState, rect: Rect, at: Pos2, source: StrokeSource) {
    if app
        .path
        .drag
        .is_none_or(|d| d.source != source || !d.surface)
    {
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

/// 離した。
pub fn release(app: &mut AppState, rect: Rect, at: Pos2, source: StrokeSource) {
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
    match current(app) {
        Some((_, path, g)) => match scene(app, rect, path, g, None).hover(pointer) {
            Hover::Point(_) => CursorIcon::Grab,
            Hover::Segment(..) => CursorIcon::Copy,
            Hover::None => CursorIcon::Crosshair,
        },
        None => CursorIcon::Crosshair,
    }
}

/// 選んでいる層の 3D のパスの線と点をモデルの上に重ねる（パスの道具のあいだだけ）。
pub fn paint_overlay(painter: &Painter, app: &AppState, rect: Rect, pointer: Option<Pos2>) {
    if !app.tool.is_path() {
        return;
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
    let hover = match (pointer, drag) {
        (Some(p), None) => s.hover(p),
        _ => Hover::None,
    };
    let selected = app.path_selected_index();
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
        let is_selected = selected == Some(i) || (closed && i == 0 && selected == Some(n - 1));
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
}
