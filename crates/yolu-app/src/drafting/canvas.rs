use super::{endpoints, outline, Drag, Figure, Ruler, RulerKind};
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
    let ruler = app.tool == Tool::Ruler;
    if !ruler {
        if let Err(reason) = crate::region::tools::paint_gate(app) {
            app.refuse(Source::Ruler, reason);
            return;
        }
    }
    let (x, y) = view.to_canvas(pos);
    let p = DVec2::new(x, y);
    let original = ruler.then(|| app.ruler()).flatten();
    let handle = original.map_or(0, |r| {
        if view.to_screen(r.a.x, r.a.y).distance(pos) < 12.0 {
            1
        } else if view.to_screen(r.b.x, r.b.y).distance(pos) < 12.0 {
            2
        } else {
            0
        }
    });
    app.drafting.drag = Some(Drag {
        source,
        start: p,
        current: p,
        shift: m.shift,
        alt: m.alt,
        ruler,
        original,
        handle,
    });
}

pub fn moved(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource, m: Modifiers) {
    if let Some(d) = app.drafting.drag.as_mut().filter(|d| d.source == source) {
        let (x, y) = view.to_canvas(pos);
        d.current = DVec2::new(x, y);
        d.shift = m.shift;
        d.alt = m.alt;
    }
}

fn draft_ruler(app: &AppState, d: Drag) -> Ruler {
    if let Some(mut r) = d.original {
        match d.handle {
            1 => r.a = d.current,
            2 => r.b = d.current,
            _ => {
                r.a += d.current - d.start;
                r.b += d.current - d.start;
            }
        }
        r
    } else {
        let (a, b) = endpoints(d.start, d.current, Figure::Line, d.shift, false);
        Ruler {
            kind: app.drafting.ruler_kind,
            a,
            b,
            two_points: app.drafting.two_points,
        }
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
    if app.drafting.drag.is_none_or(|d| d.source != source) {
        return;
    }
    moved(app, view, pos, source, m);
    let d = app.drafting.drag.take().unwrap();
    if d.ruler {
        let r = draft_ruler(app, d);
        if r.is_placeable() {
            app.drafting.rulers.insert(app.doc.id(), r);
        }
    } else if d.start.distance(d.current) > 0.01 {
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
            .and_then(|mask| {
                if app.m2.edit_mask {
                    let reveal = crate::region::tools::mask_reveals(app, layer, false);
                    app.doc
                        .fill_mask(layer, app.brush.opacity as f64, Some(&mask), reveal)
                } else {
                    let channels = app.paint_channels();
                    app.doc.fill_material(
                        layer,
                        &channels,
                        app.brush.opacity as f64,
                        Some(&mask),
                        false,
                    )
                }
            })
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
    let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(90, 170, 230, 110));
    let screen = |p: DVec2| view.to_screen(p.x, p.y);
    let long = app.doc.width().max(app.doc.height()) as f64 * 8.0;
    let line = |a: DVec2, b: DVec2| {
        let dir = super::direction(b - a) * long;
        painter.line_segment([screen(a - dir), screen(a + dir)], stroke);
    };
    let ruler = app
        .drafting
        .drag
        .filter(|d| d.ruler)
        .map(|d| draft_ruler(app, d))
        .filter(Ruler::is_placeable)
        .or_else(|| app.ruler());
    if let Some(r) = ruler {
        match r.kind {
            RulerKind::Line => line(r.a, r.b),
            RulerKind::Parallel => {
                let d = super::direction(r.b - r.a);
                let normal = DVec2::new(-d.y, d.x) * 32.0;
                for i in -8..=8 {
                    line(r.a + normal * i as f64, r.b + normal * i as f64);
                }
            }
            RulerKind::Concentric => {
                // 重ねる円は egui の円（画面の画素で分割される）。寄せ先は真の円で、拡大しても多角形に見えない。
                let radius = r.a.distance(r.b).max(1.0);
                let center = screen(r.a);
                let per_unit = center.distance(screen(r.a + DVec2::X));
                for i in 1..=4 {
                    painter.circle_stroke(center, (radius * i as f64) as f32 * per_unit, stroke);
                }
            }
            RulerKind::Perspective => {
                for center in [Some(r.a), r.two_points.then_some(r.b)]
                    .into_iter()
                    .flatten()
                {
                    for i in 0..12 {
                        let t = i as f64 * std::f64::consts::PI / 12.0;
                        line(center, center + DVec2::new(t.cos(), t.sin()));
                    }
                }
            }
        }
        if app.tool == Tool::Ruler {
            for point in [r.a, r.b] {
                painter.circle_stroke(screen(point), 5.0, stroke);
            }
        }
    }
    if let Some(d) = app.drafting.drag.filter(|d| !d.ruler) {
        let (a, b) = endpoints(d.start, d.current, app.drafting.figure, d.shift, d.alt);
        let points: Vec<_> = outline(app.drafting.figure, a, b, app.drafting.corner as f64)
            .into_iter()
            .map(screen)
            .collect();
        painter.add(Shape::line(
            points.clone(),
            Stroke::new(3.0, Color32::from_black_alpha(140)),
        ));
        painter.add(Shape::line(points, Stroke::new(1.2, Color32::WHITE)));
    }
}
