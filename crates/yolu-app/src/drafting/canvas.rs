use super::{endpoints, outline, Drag, Figure};
use crate::notice::Source;
use crate::{
    canvas::view::CanvasView,
    state::{AppState, StrokeSource, Tool},
};
use egui::{Color32, Modifiers, Painter, Pos2, Rect, Shape, Stroke};
use yolu_core::{glam::DVec2, BrushSample, SelectionMask};

pub fn press(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource, m: Modifiers) {
    if app.is_stroking() {
        return;
    }
    if app.tool == Tool::Ruler {
        crate::rulers::canvas::press(app, view, pos, source, m);
        return;
    }
    if let Err(reason) = crate::region::tools::paint_gate(app) {
        app.refuse(Source::Ruler, reason);
        return;
    }
    let (x, y) = view.to_canvas(pos);
    let p = DVec2::new(x, y);
    app.drafting.drag = Some(Drag {
        source,
        start: p,
        current: p,
        shift: m.shift,
        alt: m.alt,
    });
}

pub fn moved(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource, m: Modifiers) {
    crate::rulers::canvas::moved(app, view, pos, source, m);
    if let Some(d) = app.drafting.drag.as_mut().filter(|d| d.source == source) {
        let (x, y) = view.to_canvas(pos);
        d.current = DVec2::new(x, y);
        d.shift = m.shift;
        d.alt = m.alt;
    }
}

pub fn release(
    app: &mut AppState,
    view: &CanvasView,
    pos: Pos2,
    source: StrokeSource,
    m: Modifiers,
    rect: Rect,
) {
    crate::rulers::canvas::release(app, view, pos, source, m);
    if app.drafting.drag.is_none_or(|d| d.source != source) {
        return;
    }
    moved(app, view, pos, source, m);
    let d = app.drafting.drag.take().unwrap();
    if d.start.distance(d.current) > 0.01 {
        let (a, b) = endpoints(d.start, d.current, app.drafting.figure, d.shift, d.alt);
        paint(app, a, b, rect);
    }
}

/// 離したときだけ画素を変更する。範囲は既存の選択量と core で乗算される。
pub fn paint(app: &mut AppState, a: DVec2, b: DVec2, rect: Rect) {
    if app.is_stroking() {
        return;
    }
    let layer = match crate::region::tools::paint_gate(app) {
        Ok(id) => id,
        Err(e) => {
            app.refuse(Source::Ruler, e);
            return;
        }
    };
    let figure = app.drafting.figure;
    let points = outline(figure, a, b, app.drafting.corner as f64);
    let before = app.doc.revision();
    app.doc.end_coalescing();
    let result = if app.drafting.fill && figure != Figure::Line {
        SelectionMask::polygon(&app.doc, &points)
            .and_then(|mask| fill_region(app, layer, &mask))
            .map(|_| ())
    } else {
        app.canvas_stencil(rect)
            .and_then(|stencil| app.begin_guided_canvas_stroke(layer, false, stencil))
            .and_then(|mut stroke| {
                // 頂点には同じ時刻を渡す。core は速さを「前の点との距離 ÷ 時刻差」で求め、時刻差が無ければ 0 のままなので、
                // 速さの制御（サイズ・不透明度・流量）を入れたブラシでも頂点の間隔で線が細く・薄くならない。
                for p in &points {
                    let sample = BrushSample::new(p.x, p.y, 1.0, 0.0, DVec2::ZERO)?;
                    if let Err(e) = stroke.add_sample(&mut app.doc, sample) {
                        app.doc.cancel_stroke(stroke);
                        return Err(e);
                    }
                }
                app.doc.end_stroke(stroke).map(|_| ())
            })
    };
    finish_paint(app, result, before);
}

/// 図形の「塗る」: 範囲の量で、マスクを描くときはマスクへ（白で見せる）、そうでなければ描くチャンネル（マテリアルで塗るときは組の全部）へ
/// 不透明度で塗る（1 回の Undo。範囲は選択範囲と core で合わせる）。画素が変わったか。
pub(crate) fn fill_region(
    app: &mut AppState,
    layer: yolu_core::LayerId,
    mask: &SelectionMask,
) -> Result<bool, yolu_core::CoreError> {
    if app.m2.edit_mask {
        let reveal = crate::region::tools::mask_reveals(app, layer, false);
        app.doc
            .fill_mask(layer, app.brush.opacity as f64, Some(mask), reveal)
    } else {
        let channels = app.paint_channels();
        app.doc.fill_material(
            layer,
            &channels,
            app.brush.opacity as f64,
            Some(mask),
            false,
        )
    }
}

/// 図形を描いた後: 失敗を知らせ（途中のストロークは取り消す）、文書が変わったら印を付けて色を覚える。
pub(crate) fn finish_paint(
    app: &mut AppState,
    result: Result<(), yolu_core::CoreError>,
    before: u64,
) {
    if let Err(e) = result {
        app.doc.cancel_active_stroke();
        app.notify(
            crate::notice::Kind::of_core(&e),
            Source::Ruler,
            app.lang.core_error(&e),
        );
    }
    if app.doc.revision() != before {
        app.modified = true;
        app.color.remember();
    }
}

pub fn pen_sample(
    app: &mut AppState,
    view: &CanvasView,
    pos: Pos2,
    id: u32,
    contact: bool,
    m: Modifiers,
    rect: Rect,
) {
    let source = StrokeSource::Pen(id);
    match (contact, app.drafting.pen_down) {
        (true, None) => {
            app.drafting.pen_down = Some(id);
            press(app, view, pos, source, m);
        }
        (true, Some(old)) if old == id => moved(app, view, pos, source, m),
        (false, Some(old)) if old == id => {
            app.drafting.pen_down = None;
            release(app, view, pos, source, m, rect);
        }
        _ => {}
    }
}

pub fn paint_overlay(painter: &Painter, view: &CanvasView, app: &AppState) {
    crate::rulers::draw::paint_overlay(painter, view, app);
    let screen = |p: DVec2| view.to_screen(p.x, p.y);
    if let Some(d) = app.drafting.drag {
        let (a, b) = endpoints(d.start, d.current, app.drafting.figure, d.shift, d.alt);
        let points: Vec<_> = outline(app.drafting.figure, a, b, app.drafting.corner as f64)
            .into_iter()
            .map(screen)
            .collect();
        paint_outline(painter, points);
    }
}

/// 図形のドラッグの途中の輪郭（黒の縁取りと白。2D と 3D で同じ見た目）。
pub(crate) fn paint_outline(painter: &Painter, points: Vec<Pos2>) {
    painter.add(Shape::line(
        points.clone(),
        Stroke::new(3.0, Color32::from_black_alpha(140)),
    ));
    painter.add(Shape::line(points, Stroke::new(1.2, Color32::WHITE)));
}
