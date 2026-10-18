//! ハイドのモードの 3D ビューの重ね: 選んだ UV アイランド（青く塗って輪郭）・ポインタの下のアイランド（薄く塗って輪郭）・隠した面の輪郭
//! （点線。ハイドの設定で切れる）・四角で選んでいる途中の枠と、ハイドのモードの印のビューの縁（`frame`）。面の位置はそのビューの形（ポーズの
//! プリセットがあればその形）で写す。選んだ面の塗りは、裏の面と（数が少なければ）手前の面に隠れた面を除く。

use std::collections::HashMap;
use std::sync::Arc;

use egui::epaint::{Mesh, Vertex};
use egui::{pos2, Color32, Painter, Pos2, Rect, Shape, Stroke};
use yolu_core::geometry::{CameraView, SurfaceGeometry};
use yolu_core::glam::{Vec3, Vec4};

use crate::state::AppState;
use crate::ui::theme as t;
use crate::view3d::model::ViewModel;

/// 手前の面に隠れた三角形を光線で除く数の上限（これより多い組は、裏向きだけを除く）。
const OCCLUSION_LIMIT: usize = 8000;
/// 1 つのメッシュの三角形の数。
const CHUNK: usize = 20_000;
/// 輪郭の線の数の上限（これより多い輪郭は出さない）。
const MAX_EDGES: usize = 60_000;

/// 三角形の辺 1 本（三角形の番号と、辺の番号 0: a–b、1: b–c、2: c–a）。位置はビューの形から毎回写す。
type Edge = (u32, u8);
/// 1e-5 に丸めた位置。
type Corner = (i64, i64, i64);

/// 前のフレームと同じなら使い回す、塗る三角形と輪郭（鍵はカメラ・形の世代・組）。
#[derive(Clone, Debug, Default)]
pub struct Cache {
    key: Option<(CameraView, u32, Vec<u32>, Option<u32>)>,
    selected: Arc<Vec<u32>>,
    selected_edges: Arc<Vec<Edge>>,
    pointed: Arc<Vec<u32>>,
    pointed_edges: Arc<Vec<Edge>>,
    /// 隠した面の輪郭（鍵は隠す面の印のアドレス）。
    hidden_key: usize,
    hidden_edges: Arc<Vec<Edge>>,
}

fn quantize(v: f32) -> i64 {
    (v as f64 * 1e5).round_ties_even() as i64
}

fn corner(g: &SurfaceGeometry, t: u32, k: u8) -> Vec3 {
    let tri = &g.triangles()[t as usize];
    match k % 3 {
        0 => tri.a,
        1 => tri.b,
        _ => tri.c,
    }
}

/// 三角形の組の外周の辺（組の中の 2 つの三角形が共有しない辺。位置を 1e-5 に丸めて比べる）。
fn boundary(g: &SurfaceGeometry, tris: impl Iterator<Item = u32>) -> Vec<Edge> {
    let key = |p: Vec3| (quantize(p.x), quantize(p.y), quantize(p.z));
    let mut count: HashMap<(Corner, Corner), (u32, Edge)> = HashMap::new();
    for t in tris {
        for k in 0..3u8 {
            let (a, b) = (key(corner(g, t, k)), key(corner(g, t, k + 1)));
            let e = if a <= b { (a, b) } else { (b, a) };
            count.entry(e).or_insert((0, (t, k))).0 += 1;
        }
    }
    let mut out: Vec<Edge> = count
        .into_values()
        .filter(|(n, _)| *n == 1)
        .map(|(_, e)| e)
        .collect();
    out.sort_unstable();
    out
}

/// 組の三角形のうち、今のカメラで手前に見えるもの（裏向きと、組が小さければ見せる形の手前の面に隠れたものを除く）。
fn visible(
    source: &SurfaceGeometry,
    shown: &SurfaceGeometry,
    tris: &[u32],
    view: &CameraView,
) -> Vec<u32> {
    let occlusion = tris.len() <= OCCLUSION_LIMIT;
    let triangles = source.triangles();
    tris.iter()
        .copied()
        .filter(|&i| {
            let t = &triangles[i as usize];
            let centre = (t.a + t.b + t.c) / 3.0;
            if t.normal().dot(view.to_viewer(centre)) <= 0.0 {
                return false;
            }
            if occlusion {
                let sight = view.sight(centre);
                let d = sight.distance;
                if let Some(hit) = shown.raycast(sight.ray(), true, f32::INFINITY) {
                    if hit.distance < d - d * 1e-3 - shown.visibility_epsilon() {
                        return false;
                    }
                }
            }
            true
        })
        .collect()
}

/// ハイドのモードの印（ビューの縁。どのビューにも）。
pub fn frame(painter: &Painter, rect: Rect) {
    crate::ui::widgets::outline(painter, rect.shrink(1.0), t::ACCENT, 2.0, 0.0);
}

/// ハイドのモードの重ねを描く（今のビュー）。
pub fn draw(painter: &Painter, app: &mut AppState, rect: Rect) {
    if app.mode != crate::mode::EditorMode::Hide {
        return;
    }
    let (Some(source), Some(shown)) = (app.view3d.view_source().cloned(), app.view3d.model.clone())
    else {
        return;
    };
    let Some(index) = super::island_index(app) else {
        return;
    };
    if index.triangle_count() != source.triangle_count() {
        return;
    }
    let view = app.view3d.view.camera.view(rect.width(), rect.height());
    let g = source.geometry.clone();
    let mut cache = std::mem::take(&mut app.view3d.view.hide.overlay);
    refresh(app, &mut cache, &index, &g, &shown, &view);
    paint(painter, rect, &g, &view, &cache, app);
    app.view3d.view.hide.overlay = cache;
}

/// カメラ・形の世代・選んだ組が変わったときだけ、塗る三角形と輪郭を作り直す（隠した面の輪郭は、隠す面の印が替わったときだけ）。
fn refresh(
    app: &AppState,
    cache: &mut Cache,
    index: &super::IslandIndex,
    g: &SurfaceGeometry,
    shown: &ViewModel,
    view: &CameraView,
) {
    let selected = app.view3d.view.hide.selected.clone();
    let pointed = app
        .view3d
        .view
        .hide
        .pointed
        .filter(|p| !selected.contains(p));
    let key = (*view, shown.revision(), selected.clone(), pointed);
    if cache.key.as_ref() != Some(&key) {
        let picked: Vec<u32> = selected
            .iter()
            .flat_map(|i| index.triangles(*i).iter().copied())
            .filter(|t| !app.view3d.is_face_hidden(*t))
            .collect();
        cache.selected = Arc::new(visible(g, &shown.geometry, &picked, view));
        cache.selected_edges = Arc::new(boundary(g, picked.iter().copied()));
        let near: Vec<u32> = pointed
            .map(|i| index.triangles(i).to_vec())
            .unwrap_or_default();
        cache.pointed = Arc::new(visible(g, &shown.geometry, &near, view));
        cache.pointed_edges = Arc::new(boundary(g, near.iter().copied()));
        cache.key = Some(key);
    }
    // 隠した面の輪郭（ハイドの設定で出すとき）
    if app.view3d.hiding.outline {
        let mask = app.view3d.face_mask();
        let at = mask.map_or(0, |m| Arc::as_ptr(m) as usize);
        if cache.hidden_key != at {
            cache.hidden_key = at;
            cache.hidden_edges = Arc::new(match mask {
                Some(m) if m.len() == g.triangle_count() => {
                    boundary(g, (0..m.len() as u32).filter(|t| m.is_hidden(*t as usize)))
                }
                _ => Vec::new(),
            });
        }
    }
}

/// 作ってある塗りと輪郭・四角で選んでいる途中の枠を画面に描く。
fn paint(
    painter: &Painter,
    rect: Rect,
    g: &SurfaceGeometry,
    view: &CameraView,
    cache: &Cache,
    app: &AppState,
) {
    let vp = view.view_projection();
    let project = |p: Vec3| -> Option<Pos2> {
        let clip = vp * Vec4::new(p.x, p.y, p.z, 1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        Some(pos2(
            rect.left() + (clip.x / clip.w + 1.0) * 0.5 * rect.width(),
            rect.top() + (1.0 - clip.y / clip.w) * 0.5 * rect.height(),
        ))
    };
    let fill = |tris: &[u32], color: Color32| {
        let triangles = g.triangles();
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
    };
    let lines = |edges: &[Edge], stroke: Stroke, dashed: bool| {
        if edges.len() > MAX_EDGES {
            return;
        }
        for &(t, k) in edges {
            let (Some(a), Some(b)) = (project(corner(g, t, k)), project(corner(g, t, k + 1)))
            else {
                continue;
            };
            if dashed {
                painter.extend(Shape::dashed_line(&[a, b], stroke, 4.0, 3.0));
            } else {
                painter.line_segment([a, b], stroke);
            }
        }
    };
    if app.view3d.hiding.outline {
        lines(
            &cache.hidden_edges,
            Stroke::new(1.0, Color32::from_white_alpha(110)),
            true,
        );
    }
    fill(&cache.pointed, t::ACCENT.gamma_multiply(0.16));
    lines(&cache.pointed_edges, Stroke::new(2.0, t::ACCENT_DIM), false);
    fill(&cache.selected, t::ACCENT.gamma_multiply(0.45));
    lines(&cache.selected_edges, Stroke::new(2.0, t::ACCENT), false);
    // 四角で選んでいる途中の枠
    if let Some(r) = app.view3d.view.hide.rect {
        let area = r.rect().intersect(rect);
        painter.rect_filled(area, 0.0, t::ACCENT.gamma_multiply(0.08));
        painter.rect_stroke(
            area,
            0.0,
            Stroke::new(1.0, t::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
}
