//! ポインタの下の範囲の強調: 3D ビューは範囲の面（手前に見える三角形）を薄い色で、2D キャンバスは範囲の UV の輪郭で見せる
//! （Unity 版と同じ。塗るは橙、消すは桃色で、UV のワイヤーフレームの水色と見分けられる）。範囲を引くのは `tools::update_hover`
//! （範囲が変わったときだけ）で、ここは描くだけ。

use std::sync::Arc;

use egui::epaint::{Mesh, Vertex};
use egui::{pos2, Color32, Painter, Rect, Shape, Stroke};
use yolu_core::geometry::CameraView;
use yolu_core::glam::Vec4;

use super::tools::{update_hover, Where};
use crate::canvas::view::CanvasView;
use crate::state::AppState;

/// 塗る・消すの強調の色。
const PAINT: Color32 = Color32::from_rgb(255, 158, 41);
const ERASE: Color32 = Color32::from_rgb(255, 92, 158);
/// 3D で面を塗る薄さ。
const FILL_ALPHA: u8 = 62;
/// 2D の輪郭の線の数の上限（これを超える範囲は輪郭を出さない）。
const MAX_SEGMENTS: usize = 40_000;
/// 3D で、手前に隠れた三角形を光線で除く範囲の三角形の数の上限（これより多い範囲は、裏向きだけを除く）。
const OCCLUSION_LIMIT: usize = 8000;
/// 3D の 1 つのメッシュの三角形の数。
const CHUNK: usize = 20_000;

fn color_of(erase: bool) -> Color32 {
    if erase {
        ERASE
    } else {
        PAINT
    }
}

/// 2D キャンバス: 範囲を求め直し（ポインタが表示域の上にあるときだけ）、範囲の UV の輪郭を描く。
pub fn paint_canvas(
    painter: &Painter,
    app: &mut AppState,
    view: &CanvasView,
    pointer: Option<egui::Pos2>,
) {
    // ベイクのウィンドウでアイランドを選んでいる・アイランドのメニューを開いている間は、ツールによらず、そのアイランドを同じ形で強調する
    if !crate::bake::overlap::update_hover(app, Where::Canvas(view), pointer) {
        if !app.tool.is_region() {
            app.region.hover = None;
            return;
        }
        update_hover(app, Where::Canvas(view), pointer);
    }
    let Some(h) = app.region.hover.as_ref().filter(|h| !h.on_surface) else {
        return;
    };
    if h.outline.is_empty() || h.outline.len() > MAX_SEGMENTS {
        return;
    }
    let (w, ht) = (app.doc.width() as f64, app.doc.height() as f64);
    let points: Vec<[egui::Pos2; 2]> = h
        .outline
        .iter()
        .map(|[a, b]| {
            [
                view.to_screen(a.x as f64 * w, a.y as f64 * ht),
                view.to_screen(b.x as f64 * w, b.y as f64 * ht),
            ]
        })
        .collect();
    let color = color_of(h.erase);
    let shadow = Stroke::new(2.5, Color32::from_black_alpha(170));
    let line = Stroke::new(1.6, color);
    painter.extend(points.iter().map(|p| {
        Shape::line_segment(
            [p[0] + egui::vec2(1.0, 1.0), p[1] + egui::vec2(1.0, 1.0)],
            shadow,
        )
    }));
    painter.extend(points.iter().map(|p| Shape::line_segment(*p, line)));
}

/// 範囲の三角形のうち、今のカメラで手前に見えるもの（裏向きと、範囲が小さければ手前の面に隠れたものを除く）。
fn visible(hover: &super::tools::Hover, view: &CameraView) -> Arc<Vec<u32>> {
    let geometry = &hover.geometry;
    let triangles = geometry.triangles();
    let occlusion = hover.tris.len() <= OCCLUSION_LIMIT;
    let mut out = Vec::with_capacity(hover.tris.len());
    for &i in hover.tris.iter() {
        let t = &triangles[i as usize];
        let centre = (t.a + t.b + t.c) / 3.0;
        if t.normal().dot(view.to_viewer(centre)) <= 0.0 {
            continue; // 裏向き
        }
        if occlusion {
            let sight = view.sight(centre);
            let distance = sight.distance;
            if let Some(hit) = geometry.raycast(sight.ray(), true, f32::INFINITY) {
                if hit.triangle != i
                    && hit.distance < distance - distance * 1e-3 - geometry.visibility_epsilon()
                {
                    continue; // 手前の面に隠れている
                }
            }
        }
        out.push(i);
    }
    Arc::new(out)
}

/// 3D ビュー: 範囲を求め直し（ポインタが表示域の上にあるときだけ）、範囲の面を薄い色で重ねる。回している・ペイントのモードでないときは出さない。
pub fn paint_surface(
    painter: &Painter,
    app: &mut AppState,
    rect: Rect,
    pointer: Option<egui::Pos2>,
) {
    let busy = app.view3d.input.nav.is_some() || !app.mode.paints();
    let picking = crate::bake::overlap::highlighting(app);
    if (!app.tool.is_region() && !picking) || busy {
        if app.region.hover.as_ref().is_some_and(|h| h.on_surface) {
            app.region.hover = None;
        }
        return;
    }
    // ベイクのウィンドウでアイランドを選んでいる・アイランドのメニューを開いている間は、ツールによらず、そのアイランドを同じ形で強調する
    if !crate::bake::overlap::update_hover(app, Where::Surface(rect), pointer) {
        update_hover(app, Where::Surface(rect), pointer);
    }
    let view = app.view3d.camera.view(rect.width(), rect.height());
    let Some(h) = app.region.hover.as_mut().filter(|h| h.on_surface) else {
        return;
    };
    let tris = match &h.visible {
        Some((v, tris)) if *v == view => tris.clone(),
        _ => {
            let tris = visible(h, &view);
            h.visible = Some((view, tris.clone()));
            tris
        }
    };
    let geometry = h.geometry.clone();
    let vp = view.view_projection();
    let base = color_of(h.erase);
    let color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), FILL_ALPHA);
    let project = |p: yolu_core::glam::Vec3| -> Option<egui::Pos2> {
        let clip = vp * Vec4::new(p.x, p.y, p.z, 1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        Some(pos2(
            rect.left() + (clip.x / clip.w + 1.0) * 0.5 * rect.width(),
            rect.top() + (1.0 - clip.y / clip.w) * 0.5 * rect.height(),
        ))
    };
    let triangles = geometry.triangles();
    for chunk in tris.chunks(CHUNK) {
        let mut mesh = Mesh::default();
        for &i in chunk {
            let t = &triangles[i as usize];
            let (Some(a), Some(b), Some(c)) = (project(t.a), project(t.b), project(t.c)) else {
                continue;
            };
            let n = mesh.vertices.len() as u32;
            for p in [a, b, c] {
                mesh.vertices.push(Vertex {
                    pos: p,
                    uv: egui::epaint::WHITE_UV,
                    color,
                });
            }
            mesh.indices.extend_from_slice(&[n, n + 1, n + 2]);
        }
        if !mesh.is_empty() {
            painter.add(Shape::mesh(mesh));
        }
    }
}
