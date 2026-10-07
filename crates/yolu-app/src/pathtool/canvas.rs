//! パスのツールの 2D のキャンバス: 入力（押す・動く・離す・ペン）と、パスの線・点の重ね表示。
//!
//! 押した所に点があれば掴んで選び（動かして離すと 1 回で動く）、曲線の上なら、その区間に点を差し込み、どちらでもなければ終わりに足す。
//! キャンバスの外には足さず、動かしてもキャンバスの中に収める。表示を回している・反転しているときも、点・線の当たりは画面の座標で測る。

use egui::{Color32, CursorIcon, Painter, Pos2, Shape, Stroke};

use super::curve::{nearest_point, nearest_segment, sample_screen, P3};
use super::edit::{self, Place, PointOp, Pt, TangentValue};
use super::{
    HandleSide, Hover, PathAction, PenDown, PointDrag, PointRef, RectDrag, DOUBLE_CLICK,
    GRAB_RADIUS, PATH_COLOR,
};
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

fn tangents(path: &CanvasPath) -> Vec<TangentValue> {
    path.points.iter().map(Pt::tangent_value).collect()
}

/// 選んでいる点の取っ手の先（画面の点。取っ手の点だけ。長さ 0 の側は出さない）。
fn handle_ends(
    points: &[P3],
    tangents: &[TangentValue],
    index: usize,
    view: &CanvasView,
) -> Vec<(HandleSide, Pos2, Pos2)> {
    let (Some(p), Some(TangentValue::Handles { incoming, outgoing })) =
        (points.get(index), tangents.get(index))
    else {
        return Vec::new();
    };
    let at = view.to_screen(p[0], p[1]);
    [(HandleSide::In, incoming), (HandleSide::Out, outgoing)]
        .into_iter()
        .filter(|(_, h)| h[0] != 0.0 || h[1] != 0.0)
        .map(|(side, h)| (side, at, view.to_screen(p[0] + h[0], p[1] + h[1])))
        .collect()
}

/// ポインタの下に何があるか（選んでいる点の取っ手の先、点、曲線の順）。
pub fn hover_of(
    path: &CanvasPath,
    view: &CanvasView,
    pointer: Pos2,
    selected: Option<usize>,
) -> Hover {
    if let Some(i) = selected {
        for (side, _, end) in handle_ends(&p3(path), &tangents(path), i, view) {
            if end.distance(pointer) <= GRAB_RADIUS {
                return Hover::Handle(i, side);
            }
        }
    }
    let screen: Vec<Option<Pos2>> = path
        .points
        .iter()
        .map(|p| Some(view.to_screen(p.x, p.y)))
        .collect();
    if let Some(i) = nearest_point(&screen, pointer, GRAB_RADIUS) {
        return Hover::Point(i);
    }
    let samples = sample_screen(&p3(path), &tangents(path), &project(view), STEP);
    match nearest_segment(&samples, pointer, GRAB_RADIUS) {
        Some((s, at)) => Hover::Segment(s, at),
        None => Hover::None,
    }
}

fn canvas_size(app: &AppState) -> (f64, f64) {
    (app.doc.width() as f64, app.doc.height() as f64)
}

/// 選んでいるレイヤーの、編集していないほかの 2D のパス（一覧の上のものから）。
fn others(app: &AppState) -> Vec<(u128, &CanvasPath)> {
    let active = app.path_layer().map(|(_, p)| p.id());
    app.path_entries()
        .iter()
        .rev()
        .filter_map(|e| match &e.path {
            LayerPath::Canvas(c) if Some(c.id) != active => Some((c.id, c)),
            _ => None,
        })
        .collect()
}

/// ポインタの下の、編集していないほかのパス（点か曲線）。
fn other_under(app: &AppState, view: &CanvasView, pointer: Pos2) -> Option<u128> {
    others(app)
        .into_iter()
        .find(|(_, p)| hover_of(p, view, pointer, None) != Hover::None)
        .map(|(id, _)| id)
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
    let selected = app.path_selected_index();
    // Shift を押して押したら、点を矩形で選ぶ（点や取っ手の上でも）
    if app.path.input.shift && matches!(existing, Some(LayerPath::Canvas(_))) {
        app.path.rect = Some(RectDrag {
            source,
            surface: false,
            start: pos,
            now: pos,
        });
        return;
    }
    let on_active = match &existing {
        Some(LayerPath::Canvas(path)) => hover_of(path, view, pos, selected) != Hover::None,
        _ => false,
    };
    // 編集していないパスの上を押したら、そのパスを選ぶ（点は置かない）
    if !on_active {
        if let Some(id) = other_under(app, view, pos) {
            app.path_apply(PathAction::SelectPath(Some(id)));
            return;
        }
    }
    if let Some(LayerPath::Canvas(path)) = &existing {
        match hover_of(path, view, pos, selected) {
            Hover::Handle(index, side) => {
                app.path.drag = Some(PointDrag {
                    layer,
                    path: path.id,
                    index,
                    source,
                    surface: false,
                    start: pos,
                    moved: 0.0,
                    target: None,
                    handle: Some(side),
                    vector: None,
                });
                return;
            }
            Hover::Point(index) => {
                let at = PointRef {
                    layer,
                    path: path.id,
                    index,
                };
                // 同じ点のダブルクリックは、角と滑らかの切り替え（ドラッグは始めない）
                if let (Some(now), Some((t, last))) = (app.path.input.now, app.path.last_press) {
                    if last == at && now - t <= DOUBLE_CLICK {
                        app.path.last_press = None;
                        app.path.selected = Some(at);
                        app.path_apply(PathAction::Point(PointOp::ToggleCorner(index)));
                        return;
                    }
                }
                app.path.last_press = app.path.input.now.map(|now| (now, at));
                app.path.selected = Some(at);
                app.path.drag = Some(PointDrag {
                    layer,
                    path: path.id,
                    index,
                    source,
                    surface: false,
                    start: pos,
                    moved: 0.0,
                    target: None,
                    handle: None,
                    vector: None,
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
    if let Some(r) = app
        .path
        .rect
        .as_mut()
        .filter(|r| r.source == source && !r.surface)
    {
        r.now = pos;
        return;
    }
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
    if d.handle.is_some() {
        // 取っ手の向きは点からポインタまで（キャンバスの外へも伸ばせる）
        let d = *d;
        let at = app.path_layer().and_then(|(_, p)| match p {
            LayerPath::Canvas(c) if c.id == d.path => c.points.get(d.index).copied(),
            _ => None,
        });
        if let (Some(p), Some(drag)) = (at, app.path.drag.as_mut()) {
            drag.vector = Some([x - p.x, y - p.y, 0.0]);
        }
        return;
    }
    d.target = Some(Place::Canvas {
        x: x.clamp(0.0, w),
        y: y.clamp(0.0, h),
    });
}

/// 離した。
pub fn release(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource) {
    if app
        .path
        .rect
        .is_some_and(|r| r.source == source && !r.surface)
    {
        moved(app, view, pos, source);
        finish_rect(app, view);
        return;
    }
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

/// 矩形の選びを終える: 矩形に入る点（画面の点で測る）を選ぶ。
pub fn finish_rect(app: &mut AppState, view: &CanvasView) {
    let Some(r) = app.path.rect.take() else {
        return;
    };
    let Some((layer, LayerPath::Canvas(path))) = app.path_layer() else {
        return;
    };
    let area = egui::Rect::from_two_pos(r.start, r.now);
    let inside: Vec<usize> = path
        .points
        .iter()
        .enumerate()
        .filter(|(_, p)| area.contains(view.to_screen(p.x, p.y)))
        .map(|(i, _)| i)
        .collect();
    let a = super::ActivePath {
        layer,
        path: path.id,
    };
    app.path.selected = None;
    app.path.marked = (!inside.is_empty()).then_some((a, inside));
}

/// 矩形の選びの枠。
pub(super) fn draw_rect(painter: &Painter, r: &RectDrag) {
    let area = egui::Rect::from_two_pos(r.start, r.now);
    painter.rect_filled(area, 0.0, PATH_COLOR.gamma_multiply(0.12));
    painter.rect_stroke(
        area,
        0.0,
        Stroke::new(1.0, PATH_COLOR),
        egui::StrokeKind::Inside,
    );
}

/// ポインタの形（点の上は掴む手、曲線の上は足す形）。
pub fn cursor_icon(app: &AppState, view: &CanvasView, pointer: Pos2) -> CursorIcon {
    if app.path.drag.is_some() {
        return CursorIcon::Grabbing;
    }
    let active = match app.path_layer() {
        Some((_, LayerPath::Canvas(path))) => {
            hover_of(path, view, pointer, app.path_selected_index())
        }
        _ => Hover::None,
    };
    match active {
        Hover::Point(_) | Hover::Handle(..) => CursorIcon::Grab,
        Hover::Segment(..) => CursorIcon::Copy,
        Hover::None if other_under(app, view, pointer).is_some() => CursorIcon::PointingHand,
        Hover::None => CursorIcon::Crosshair,
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

/// 編集していないパスの線（細く薄く）。
pub(super) fn other_line() -> Stroke {
    Stroke::new(1.5, PATH_COLOR.gamma_multiply(0.45))
}

/// 画面の点の列の曲線を、見えない所（None）で切りながら描く。
pub(super) fn draw_curve(painter: &Painter, samples: &[Vec<Option<Pos2>>]) {
    draw_curve_with(painter, samples, Some(halo()), line());
}

/// 編集していないパスの曲線（縁取りなし・薄い線）。
pub(super) fn draw_other_curve(painter: &Painter, samples: &[Vec<Option<Pos2>>]) {
    draw_curve_with(painter, samples, None, other_line());
}

fn draw_curve_with(
    painter: &Painter,
    samples: &[Vec<Option<Pos2>>],
    halo: Option<Stroke>,
    line: Stroke,
) {
    let mut run: Vec<Pos2> = Vec::new();
    let flush = |run: &mut Vec<Pos2>| {
        if run.len() >= 2 {
            if let Some(h) = halo {
                painter.add(Shape::line(run.clone(), h));
            }
            painter.add(Shape::line(run.clone(), line));
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

/// ドラッグ中の取っ手を当てた接線の並び（取っ手のドラッグが無ければ今のまま）。
pub(super) fn shown_tangents(
    app: &AppState,
    now: &[TangentValue],
    layer: yolu_core::LayerId,
    path: u128,
) -> Vec<TangentValue> {
    let mut out = now.to_vec();
    if let Some(d) = app
        .path
        .drag
        .filter(|d| d.layer == layer && d.path == path && d.index < out.len())
    {
        if let (Some(side), Some(vector)) = (d.handle, d.vector) {
            out[d.index] =
                AppState::path_handle_tangent(out[d.index], side, vector, app.path.input);
        }
    }
    out
}

/// 取っ手（点から先までの線と、先の丸）。
pub(super) fn draw_handle(painter: &Painter, from: Pos2, end: Pos2, hot: bool) {
    painter.line_segment(
        [from, end],
        Stroke::new(3.0, Color32::from_black_alpha(140)),
    );
    painter.line_segment([from, end], Stroke::new(1.2, Color32::WHITE));
    let r = if hot { 5.0 } else { 4.0 };
    painter.circle_filled(end, r, if hot { PATH_COLOR } else { Color32::WHITE });
    painter.circle_stroke(end, r, Stroke::new(1.5, Color32::from_black_alpha(190)));
}

/// 差し込む位置の印（曲線の上の輪）。
pub(super) fn draw_insert_ring(painter: &Painter, at: Pos2) {
    painter.circle_stroke(at, 5.5, Stroke::new(3.0, Color32::from_black_alpha(150)));
    painter.circle_stroke(at, 5.5, Stroke::new(1.5, PATH_COLOR));
}

/// 選んでいるレイヤーの 2D のパスの線と点をキャンバスに重ねる（パスのツールのあいだだけ）。
pub fn paint_overlay(painter: &Painter, view: &CanvasView, app: &AppState, pointer: Option<Pos2>) {
    if !app.tool.is_path() {
        return;
    }
    // 編集していないパスは薄い線だけ（押すと選ぶ）
    for (_, other) in others(app) {
        draw_other_curve(
            painter,
            &sample_screen(&p3(other), &tangents(other), &project(view), STEP),
        );
    }
    let Some((layer, LayerPath::Canvas(path))) = app.path_layer() else {
        return;
    };
    let pts = shown_points(app, path, layer);
    if pts.is_empty() {
        return;
    }
    let p3s: Vec<P3> = pts.iter().map(|p| [p.0, p.1, 0.0]).collect();
    let shown = shown_tangents(app, &tangents(path), layer, path.id);
    let samples = sample_screen(&p3s, &shown, &project(view), STEP);
    draw_curve(painter, &samples);
    let selected = app.path_selected_index();
    let hover = match (pointer, app.path.drag) {
        (Some(p), None) => hover_of(path, view, p, selected),
        _ => Hover::None,
    };
    // 選んでいる点の取っ手（線と先の丸）
    if let Some(i) = selected {
        for (side, from, end) in handle_ends(&p3s, &shown, i, view) {
            let hot = hover == Hover::Handle(i, side)
                || app
                    .path
                    .drag
                    .is_some_and(|d| d.handle == Some(side) && d.index == i);
            draw_handle(painter, from, end, hot);
        }
    }
    let marked = app.path_selected_indices();
    let closed = edit::is_closed(&path.points);
    let n = pts.len();
    for (i, p) in pts.iter().enumerate() {
        // 閉じたパスの終わりの点は始めの点と同じ場所なので、始めの点だけを描く
        if closed && i == n - 1 {
            continue;
        }
        let is_selected = marked.contains(&i) || (closed && i == 0 && marked.contains(&(n - 1)));
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
    if let Some(r) = app.path.rect.filter(|r| !r.surface) {
        draw_rect(painter, &r);
    }
}
