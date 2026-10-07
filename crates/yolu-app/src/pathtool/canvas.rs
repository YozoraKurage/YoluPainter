//! パスの道具の 2D のキャンバス: 入力（押す・動く・離す・ペン）と、パスの線・点の重ね表示。
//!
//! 押した所に点があれば掴んで選び（動かして離すと 1 回で動く）、曲線の上なら、その区間に点を差し込み、どちらでもなければ終わりに足す。
//! キャンバスの外には足さず、動かしてもキャンバスの中に収める。表示を回している・反転しているときも、点・線の当たりは画面の座標で測る。

use egui::{Color32, CursorIcon, Painter, Pos2, Shape, Stroke};

use super::curve::{nearest_point, nearest_segment, sample_screen, P3};
use super::edit::{self, Place, PointOp};
use super::{Hover, PathAction, PenDown, PointDrag, PointRef, GRAB_RADIUS, PATH_COLOR};
use crate::canvas::view::CanvasView;
use crate::notice::Source;
use crate::state::{AppState, StrokeSource};
use yolu_core::paths::CanvasPath;
use yolu_core::LayerPath;

/// 標本の間隔（画面の点）。
const STEP: f32 = 6.0;

fn project(view: &CanvasView) -> impl Fn(P3) -> Option<Pos2> + '_ {
    move |p| Some(view.to_screen(p[0], p[1]))
}

fn p3(path: &CanvasPath) -> Vec<P3> {
    path.points.iter().map(|p| [p.x, p.y, 0.0]).collect()
}

/// ポインタの下に何があるか（点が先、次に曲線）。
pub fn hover_of(path: &CanvasPath, view: &CanvasView, pointer: Pos2) -> Hover {
    let screen: Vec<Option<Pos2>> = path
        .points
        .iter()
        .map(|p| Some(view.to_screen(p.x, p.y)))
        .collect();
    if let Some(i) = nearest_point(&screen, pointer, GRAB_RADIUS) {
        return Hover::Point(i);
    }
    let samples = sample_screen(&p3(path), &project(view), STEP);
    match nearest_segment(&samples, pointer, GRAB_RADIUS) {
        Some((s, at)) => Hover::Segment(s, at),
        None => Hover::None,
    }
}

fn canvas_size(app: &AppState) -> (f64, f64) {
    (app.doc.width() as f64, app.doc.height() as f64)
}

/// 押した（ペン・マウス）。
pub fn press(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource) {
    if !app.tool.is_path() || app.path.drag.is_some() {
        return;
    }
    let Some((layer, existing)) = app.path_target(Some(false)) else {
        return;
    };
    let (x, y) = view.to_canvas(pos);
    if let Some(LayerPath::Canvas(path)) = &existing {
        match hover_of(path, view, pos) {
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
                    surface: false,
                    start: pos,
                    moved: 0.0,
                    target: None,
                });
                return;
            }
            Hover::Segment(segment, _) => {
                if inside(app, x, y) {
                    app.path_apply(PathAction::Point(PointOp::Insert {
                        segment,
                        place: Place::Canvas { x, y },
                    }));
                } else {
                    outside(app);
                }
                return;
            }
            Hover::None => {}
        }
    }
    if inside(app, x, y) {
        app.path_apply(PathAction::Point(PointOp::Add(Place::Canvas { x, y })));
    } else {
        outside(app);
    }
}

fn inside(app: &AppState, x: f64, y: f64) -> bool {
    let (w, h) = canvas_size(app);
    (0.0..=w).contains(&x) && (0.0..=h).contains(&y)
}

fn outside(app: &mut AppState) {
    app.refuse(
        Source::Path,
        app.lang.pick("キャンバスの外です", "Outside the canvas"),
    );
}

/// 動いた。
pub fn moved(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource) {
    let (w, h) = canvas_size(app);
    let Some(d) = app
        .path
        .drag
        .as_mut()
        .filter(|d| d.source == source && !d.surface)
    else {
        return;
    };
    d.moved = d.moved.max(pos.distance(d.start));
    let (x, y) = view.to_canvas(pos);
    d.target = Some(Place::Canvas {
        x: x.clamp(0.0, w),
        y: y.clamp(0.0, h),
    });
}

/// 離した。
pub fn release(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource) {
    if app
        .path
        .drag
        .is_none_or(|d| d.source != source || d.surface)
    {
        return;
    }
    moved(app, view, pos, source);
    app.path_finish_drag();
}

/// ペン（Windows Ink）の 1 点。触れた・動いた・離したを、押す・動く・離すにする。3D のビューが触れているペンは扱わない
/// （同じ列が両方のビューに渡るので、離したサンプルで相手のドラッグを取り残さないため）。
pub fn pen_sample(
    app: &mut AppState,
    view: &CanvasView,
    pos: Pos2,
    pointer_id: u32,
    contact: bool,
) {
    let source = StrokeSource::Pen(pointer_id);
    match (contact, app.path.pen_down) {
        (true, None) => {
            app.path.pen_down = Some(PenDown {
                id: pointer_id,
                surface: false,
            });
            press(app, view, pos, source);
        }
        (true, Some(p)) if p.id == pointer_id && !p.surface => moved(app, view, pos, source),
        (false, Some(p)) if p.id == pointer_id && !p.surface => {
            app.path.pen_down = None;
            release(app, view, pos, source);
        }
        _ => {}
    }
}

/// ポインタの形（点の上は掴む手、曲線の上は足す形）。
pub fn cursor_icon(app: &AppState, view: &CanvasView, pointer: Pos2) -> CursorIcon {
    if app.path.drag.is_some() {
        return CursorIcon::Grabbing;
    }
    match app.path_layer() {
        Some((_, LayerPath::Canvas(path))) => match hover_of(path, view, pointer) {
            Hover::Point(_) => CursorIcon::Grab,
            Hover::Segment(..) => CursorIcon::Copy,
            Hover::None => CursorIcon::Crosshair,
        },
        _ => CursorIcon::Crosshair,
    }
}

/// 重ね表示で使う点の列（ドラッグ中の点は今の置き場所。閉じたパスの始め・終わりは一緒に動く）。
fn shown_points(app: &AppState, path: &CanvasPath, layer: yolu_core::LayerId) -> Vec<(f64, f64)> {
    let mut pts: Vec<(f64, f64)> = path.points.iter().map(|p| (p.x, p.y)).collect();
    if let Some(d) = app
        .path
        .drag
        .filter(|d| !d.surface && d.layer == layer && d.path == path.id && d.index < pts.len())
    {
        if let Some(Place::Canvas { x, y }) = d.target {
            let n = pts.len();
            pts[d.index] = (x, y);
            if edit::is_closed(&path.points) && (d.index == 0 || d.index == n - 1) {
                pts[0] = (x, y);
                pts[n - 1] = (x, y);
            }
        }
    }
    pts
}

pub(super) fn halo() -> Stroke {
    Stroke::new(4.0, Color32::from_black_alpha(150))
}

pub(super) fn line() -> Stroke {
    Stroke::new(2.0, PATH_COLOR)
}

/// 画面の点の列の曲線を、見えない所（None）で切りながら描く。
pub(super) fn draw_curve(painter: &Painter, samples: &[Vec<Option<Pos2>>]) {
    let mut run: Vec<Pos2> = Vec::new();
    let flush = |run: &mut Vec<Pos2>| {
        if run.len() >= 2 {
            painter.add(Shape::line(run.clone(), halo()));
            painter.add(Shape::line(run.clone(), line()));
        }
        run.clear();
    };
    for segment in samples {
        for (k, s) in segment.iter().enumerate() {
            match s {
                // 区間の最初の標本は、前の区間の最後の標本と同じ点
                Some(_) if k == 0 && !run.is_empty() => {}
                Some(p) => run.push(*p),
                None => flush(&mut run),
            }
        }
    }
    flush(&mut run);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MarkerStyle {
    Plain,
    Hover,
    Selected,
    /// 隠れている・別の面にある点（薄く）。
    Dim,
}

pub(super) fn draw_marker(painter: &Painter, at: Pos2, style: MarkerStyle) {
    let (half, fill, edge) = match style {
        MarkerStyle::Plain => (3.5, PATH_COLOR, Color32::from_black_alpha(190)),
        MarkerStyle::Hover => (5.0, PATH_COLOR, Color32::WHITE),
        MarkerStyle::Selected => (5.0, Color32::WHITE, PATH_COLOR),
        MarkerStyle::Dim => (
            3.0,
            PATH_COLOR.gamma_multiply(0.35),
            Color32::from_black_alpha(90),
        ),
    };
    let r = egui::Rect::from_center_size(at, egui::vec2(half * 2.0, half * 2.0));
    painter.rect_filled(r, 1.0, fill);
    painter.rect_stroke(r, 1.0, Stroke::new(1.5, edge), egui::StrokeKind::Outside);
}

/// 差し込む位置の印（曲線の上の輪）。
pub(super) fn draw_insert_ring(painter: &Painter, at: Pos2) {
    painter.circle_stroke(at, 5.5, Stroke::new(3.0, Color32::from_black_alpha(150)));
    painter.circle_stroke(at, 5.5, Stroke::new(1.5, PATH_COLOR));
}

/// 選んでいる層の 2D のパスの線と点をキャンバスに重ねる（パスの道具のあいだだけ）。
pub fn paint_overlay(painter: &Painter, view: &CanvasView, app: &AppState, pointer: Option<Pos2>) {
    if !app.tool.is_path() {
        return;
    }
    let Some((layer, LayerPath::Canvas(path))) = app.path_layer() else {
        return;
    };
    let pts = shown_points(app, path, layer);
    if pts.is_empty() {
        return;
    }
    let p3s: Vec<P3> = pts.iter().map(|p| [p.0, p.1, 0.0]).collect();
    let samples = sample_screen(&p3s, &project(view), STEP);
    draw_curve(painter, &samples);
    let hover = match (pointer, app.path.drag) {
        (Some(p), None) => hover_of(path, view, p),
        _ => Hover::None,
    };
    let selected = app.path_selected_index();
    let closed = edit::is_closed(&path.points);
    let n = pts.len();
    for (i, p) in pts.iter().enumerate() {
        // 閉じたパスの終わりの点は始めの点と同じ場所なので、始めの点だけを描く
        if closed && i == n - 1 {
            continue;
        }
        let is_selected = selected == Some(i) || (closed && i == 0 && selected == Some(n - 1));
        let is_hover =
            hover == Hover::Point(i) || (closed && i == 0 && hover == Hover::Point(n - 1));
        let style = if is_selected {
            MarkerStyle::Selected
        } else if is_hover {
            MarkerStyle::Hover
        } else {
            MarkerStyle::Plain
        };
        draw_marker(painter, view.to_screen(p.0, p.1), style);
    }
    if let Hover::Segment(_, at) = hover {
        draw_insert_ring(painter, at);
    }
}
