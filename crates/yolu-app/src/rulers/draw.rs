//! 2D のキャンバスの上の定規の描画: 見えている 2D の定規（種類ごとの線）、対称定規の線（中心からの半直線）、定規のツールのときの編集できる定規のつまみ、
//! ドラッグの途中の形。効いている定規と寄せの候補は今の濃さ、効いていない特殊定規は薄く描く。

use egui::{Color32, Painter, Rect, Stroke, Vec2};
use yolu_core::glam::DVec2;
use yolu_core::{Ruler, RulerKind};

use super::active::canvas_points;
use super::canvas::{grabbed, handles};
use super::Handle;
use crate::canvas::view::CanvasView;
use crate::state::{AppState, Tool};

/// 定規の線の色（効いているもの）と、効いていないもの。
const RULER_RGB: [u8; 3] = [90, 170, 230];
/// 対称の線の色（水色）。
const SYMMETRY_RGB: [u8; 3] = [115, 209, 255];

fn ruler_stroke(effective: bool) -> Stroke {
    let alpha = if effective { 110 } else { 45 };
    Stroke::new(
        1.0,
        Color32::from_rgba_unmultiplied(RULER_RGB[0], RULER_RGB[1], RULER_RGB[2], alpha),
    )
}

/// 対称定規の半直線の向き（ラジアン）: 最初の線 `b − a` から、`lines` 本が一周を等分する（線対称 N 本は鏡の軸 N/2 本の両側の半直線）。
pub fn symmetry_directions(r: &Ruler) -> Vec<DVec2> {
    let Some((a, b)) = canvas_points(r) else {
        return Vec::new();
    };
    let lines = usize::from(r.lines.max(1));
    let first = (b - a).try_normalize().unwrap_or(DVec2::X);
    (0..lines)
        .map(|k| {
            let turn = std::f64::consts::TAU * k as f64 / lines as f64;
            let (s, c) = turn.sin_cos();
            DVec2::new(first.x * c - first.y * s, first.x * s + first.y * c)
        })
        .collect()
}

/// 2D の定規 1 つの線。
fn paint_lines(painter: &Painter, view: &CanvasView, r: &Ruler, effective: bool, long: f64) {
    let screen = |p: DVec2| view.to_screen(p.x, p.y);
    if r.kind == RulerKind::Symmetry {
        let Some((a, _)) = canvas_points(r) else {
            return;
        };
        let dim = if effective { 1.0 } else { 0.35 };
        let shadow = Stroke::new(3.0, Color32::from_black_alpha((90.0 * dim) as u8));
        let line = Stroke::new(
            1.5,
            Color32::from_rgba_unmultiplied(
                SYMMETRY_RGB[0],
                SYMMETRY_RGB[1],
                SYMMETRY_RGB[2],
                (204.0 * dim) as u8,
            ),
        );
        for d in symmetry_directions(r) {
            let end = screen(a + d * long);
            painter.line_segment([screen(a), end], shadow);
            painter.line_segment([screen(a), end], line);
        }
        return;
    }
    let Some((a, b)) = canvas_points(r) else {
        return;
    };
    let stroke = ruler_stroke(effective);
    let direction = |v: DVec2| v.try_normalize().unwrap_or(DVec2::X);
    let line = |from: DVec2, to: DVec2| {
        let dir = direction(to - from) * long;
        painter.line_segment([screen(from - dir), screen(from + dir)], stroke);
    };
    match r.kind {
        RulerKind::Line => line(a, b),
        RulerKind::Parallel => {
            let d = direction(b - a);
            let normal = DVec2::new(-d.y, d.x) * 32.0;
            for i in -8..=8 {
                line(a + normal * i as f64, b + normal * i as f64);
            }
        }
        RulerKind::Concentric => {
            // 重ねる円は egui の円（画面の画素で分割される）。寄せ先は真の円で、拡大しても多角形に見えない。
            let radius = a.distance(b).max(1.0);
            let center = screen(a);
            let per_unit = center.distance(screen(a + DVec2::X));
            for i in 1..=4 {
                painter.circle_stroke(center, (radius * i as f64) as f32 * per_unit, stroke);
            }
        }
        RulerKind::Perspective => {
            for center in [Some(a), r.two_points.then_some(b)].into_iter().flatten() {
                for i in 0..12 {
                    let t = i as f64 * std::f64::consts::PI / 12.0;
                    line(center, center + DVec2::new(t.cos(), t.sin()));
                }
            }
        }
        RulerKind::Symmetry => {}
    }
}

/// つまみ（定規のツールのとき、編集できる定規に）。選んでいる定規は橙、ほかは定規の色。
fn paint_handles(painter: &Painter, view: &CanvasView, r: &Ruler, selected: bool) {
    let color = if selected {
        crate::objects::SELECTED
    } else {
        Color32::from_rgba_unmultiplied(RULER_RGB[0], RULER_RGB[1], RULER_RGB[2], 200)
    };
    let stroke = Stroke::new(if selected { 1.8 } else { 1.2 }, color);
    for (handle, point) in handles(r) {
        let at = view.to_screen(point.x, point.y);
        if r.kind == RulerKind::Symmetry {
            // 中心（全体を動かす）は塗った四角、向き（最初の線を回す）は輪郭だけの四角
            let square = Rect::from_center_size(at, Vec2::splat(9.0));
            match handle {
                Handle::A => {
                    painter.rect_filled(square, 1.0, color);
                }
                _ => {
                    painter.rect_stroke(square, 1.0, stroke, egui::StrokeKind::Middle);
                }
            }
        } else {
            painter.circle_stroke(at, 5.0, stroke);
        }
    }
}

/// 線を伸ばす長さ（文書の画素）。
fn reach(app: &AppState) -> f64 {
    app.doc.width().max(app.doc.height()) as f64 * 8.0
}

/// キャンバスの上に、見えている 2D の定規と、ドラッグの途中の形を描く。
pub fn paint_overlay(painter: &Painter, view: &CanvasView, app: &AppState) {
    let long = reach(app);
    let ruler_tool = app.tool == Tool::Ruler;
    let grabbed = grabbed(app);
    for shown in app.rulers_shown(super::Place::Canvas) {
        let ruler = match &grabbed {
            Some((owner, moved)) if *owner == shown.owner && moved.id == shown.ruler.id => moved,
            _ => shown.ruler,
        };
        paint_lines(painter, view, ruler, shown.effective, long);
        if ruler_tool && shown.editable {
            paint_handles(painter, view, ruler, shown.selected);
        }
    }
    // 新しい定規を引いている形
    if let Some(drag) = app.rulers.drag.as_ref().filter(|d| d.grab.is_none()) {
        let ruler = super::canvas::dragged_ruler(&app.rulers, drag);
        if ruler.validate().is_ok() {
            paint_lines(painter, view, &ruler, true, long);
            paint_handles(painter, view, &ruler, true);
        }
    }
}

/// 点（キャンバスの座標）の、対称の写し（元の点を除く）。映した側のカーソルに使う。
pub fn mirrored_points(s: &yolu_core::CanvasSymmetry, x: f64, y: f64) -> Vec<(f64, f64)> {
    s.transforms()
        .map(|ts| ts.iter().skip(1).map(|t| t.map(x, y)).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::RulerId;

    #[test]
    fn symmetry_rays_split_the_turn_evenly_from_the_first_line() {
        let mut r = Ruler::canvas(
            RulerId(1),
            RulerKind::Symmetry,
            DVec2::new(5.0, 5.0),
            DVec2::new(5.0, 9.0),
        );
        r.lines = 4;
        let rays = symmetry_directions(&r);
        assert_eq!(rays.len(), 4);
        assert!((rays[0] - DVec2::Y).length() < 1e-12, "最初の線の向き");
        assert!((rays[1] - DVec2::NEG_X).length() < 1e-12);
        assert!((rays[2] - DVec2::NEG_Y).length() < 1e-12);
        assert!((rays[3] - DVec2::X).length() < 1e-12);
        let two = {
            r.lines = 2;
            symmetry_directions(&r)
        };
        assert!(
            (two[0] + two[1]).length() < 1e-12,
            "線対称 2 本は 1 本の直線"
        );
    }
}
