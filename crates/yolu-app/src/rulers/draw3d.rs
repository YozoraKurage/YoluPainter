//! 3D ビューの上の定規の描画: 見えている 3D の定規（種類ごとの線。対称は今までの対称の面と軸の描き方）、引いている途中の形、編集のモードで選んだ
//! 定規（橙の線と端の点の四角）。効いている物は濃く、効いていない特殊定規は薄く描く。点はカメラの後ろに出たところで線を切る。

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2 as EVec2};
use yolu_core::geometry::CameraView;
use yolu_core::glam::{Quat, Vec3};
use yolu_core::{Ruler, RulerKind};

use super::edit3d::{ends, points};
use super::Place;
use crate::state::AppState;
use crate::view3d::input::camera_view;

/// 定規の線の色（効いているもの）と、対称の線の色（水色。今までの対称の軸と同じ）。
const RULER_RGB: [u8; 3] = [90, 170, 230];
const SYMMETRY_RGB: [u8; 3] = [115, 209, 255];
/// 効いていない特殊定規の濃さ。
const DIM: f32 = 0.35;
/// 端の点の四角の大きさ（画面の点）。
pub const SQUARE_POINTS: f32 = 9.0;

/// 描き方。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    /// 引いている途中の形（効いている濃さ）。
    Draft,
    /// 文書の定規。`effective` は寄せ・写しが効いている、`selected` は選んでいる（橙）。
    Shown { effective: bool, selected: bool },
}

/// 線の色と、黒い縁の濃さの割合。効いていない定規は、線の色のアルファも縁も `DIM` 倍にする（対称の面の塗りは線の色から作るので、一緒に薄くなる）。
fn color(r: &Ruler, look: Look) -> (Color32, f32) {
    let rgb = if r.kind == RulerKind::Symmetry {
        SYMMETRY_RGB
    } else {
        RULER_RGB
    };
    let base = Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], 204);
    match look {
        Look::Shown { selected: true, .. } => (crate::objects::SELECTED, 1.0),
        Look::Shown {
            effective: false, ..
        } => (dimmed(base, DIM), DIM),
        _ => (base, 1.0),
    }
}

fn dimmed(c: Color32, k: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * k) as u8)
}

/// 線の列（カメラの後ろの点で切る）を、外が黒の細い影、中が色の二重の線で描く。
fn polyline(
    painter: &Painter,
    view: &CameraView,
    rect: Rect,
    world: &[Vec3],
    color: Color32,
    shade: f32,
    width: f32,
) {
    let mut part: Vec<Pos2> = Vec::new();
    let flush = |part: &mut Vec<Pos2>| {
        if part.len() > 1 {
            painter.add(Shape::line(
                part.clone(),
                Stroke::new(width + 1.5, Color32::from_black_alpha((90.0 * shade) as u8)),
            ));
            painter.add(Shape::line(part.clone(), Stroke::new(width, color)));
        }
        part.clear();
    };
    for p in world {
        match view.to_screen(*p) {
            Some(s) => part.push(Pos2::new(rect.left() + s.x, rect.top() + s.y)),
            None => flush(&mut part),
        }
    }
    flush(&mut part);
}

/// 2 点の間を細かく分けた点（遠近の写しでカメラの後ろへ回る線を、手前の部分だけ描けるように）。
fn along(a: Vec3, b: Vec3) -> Vec<Vec3> {
    (0..=32).map(|k| a.lerp(b, k as f32 / 32.0)).collect()
}

/// 面 `axis`（単位）に直交する、`center` を通る半径 `radius` の円。
fn circle(center: Vec3, axis: Vec3, radius: f32) -> Vec<Vec3> {
    let u = axis
        .cross(if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::X })
        .normalize_or_zero();
    let w = axis.cross(u);
    (0..=64)
        .map(|k| {
            let t = k as f32 / 64.0 * std::f32::consts::TAU;
            center + (u * t.cos() + w * t.sin()) * radius
        })
        .collect()
}

/// モデルの外形から決める、線を伸ばす長さ。
pub(super) fn reach(app: &AppState) -> f32 {
    app.view3d.model.as_ref().map_or(1.0, |m| {
        (m.geometry.bounds().extents.max_element() * 1.25).max(1e-4)
    })
}

/// 点（表示域の左上から）を egui の点へ。
fn screen_at(view: &CameraView, rect: Rect, p: Vec3) -> Option<Pos2> {
    let s = view.to_screen(p)?;
    Some(Pos2::new(rect.left() + s.x, rect.top() + s.y))
}

/// `up` に直交する面の、最初の向き（a→b の `up` に直交する成分）。
fn first_direction(a: Vec3, b: Vec3, up: Vec3) -> Vec3 {
    let along = b - a;
    let flat = along - up * along.dot(up);
    flat.try_normalize()
        .unwrap_or_else(|| up.cross(if up.x.abs() < 0.9 { Vec3::X } else { Vec3::Y }))
        .normalize_or_zero()
}

/// 定規 1 つを描く。
pub fn paint_ruler(painter: &Painter, app: &AppState, rect: Rect, r: &Ruler, look: Look) {
    let Some((a, b, up)) = points(r) else {
        return;
    };
    let view = camera_view(app, rect);
    let (color, shade) = color(r, look);
    let (a, b, up) = (a.as_vec3(), b.as_vec3(), up.as_vec3());
    let reach = reach(app);
    let line =
        |world: &[Vec3], width: f32| polyline(painter, &view, rect, world, color, shade, width);
    let ring = |at: Pos2, radius: f32| {
        painter.circle_stroke(
            at,
            radius + 1.2,
            Stroke::new(2.5, Color32::from_black_alpha((90.0 * shade) as u8)),
        );
        painter.circle_stroke(at, radius, Stroke::new(1.5, color));
    };
    match r.kind {
        RulerKind::Line => {
            let d = (b - a).try_normalize().unwrap_or(Vec3::X);
            line(&along(a - d * reach * 4.0, a + d * reach * 4.0), 1.5);
            for p in [a, b] {
                if let Some(at) = screen_at(&view, rect, p) {
                    ring(at, 4.0);
                }
            }
        }
        RulerKind::Parallel => {
            let d = (b - a).try_normalize().unwrap_or(Vec3::X);
            let across = up.cross(d).try_normalize().unwrap_or(Vec3::Y);
            for k in -2..=2 {
                let o = a + across * (k as f32 * reach * 0.2);
                line(&along(o - d * reach * 0.5, o + d * reach * 0.5), 1.5);
            }
            if let Some(at) = screen_at(&view, rect, a) {
                ring(at, 4.0);
            }
        }
        RulerKind::Concentric => {
            let radius = first_direction(a, b, up).dot(b - a).abs().max(reach * 0.05);
            for k in 1..=3 {
                line(&circle(a, up, radius * k as f32), 1.5);
            }
            if let Some(at) = screen_at(&view, rect, a) {
                ring(at, 4.0);
            }
        }
        RulerKind::Perspective => {
            let d0 = first_direction(a, b, up);
            let vanishing: Vec<Vec3> = if r.two_points { vec![a, b] } else { vec![a] };
            for v in vanishing {
                for k in 0..8 {
                    let t = k as f32 / 8.0 * std::f32::consts::TAU;
                    let d = Quat::from_axis_angle(up, t) * d0;
                    line(&along(v, v + d * reach), 1.5);
                }
                if let Some(at) = screen_at(&view, rect, v) {
                    ring(at, 4.0);
                }
            }
        }
        RulerKind::Symmetry => {
            paint_symmetry(painter, &view, rect, r, (a, b, up), (color, shade), reach)
        }
    }
}

/// 対称定規: 回転の軸の線と、線対称 2 本なら鏡の面の四角、それ以外は軸に直交する面の中心からの線 `lines` 本（2D の対称の線と同じ）。
fn paint_symmetry(
    painter: &Painter,
    view: &CameraView,
    rect: Rect,
    r: &Ruler,
    (a, b, up): (Vec3, Vec3, Vec3),
    (color, shade): (Color32, f32),
    reach: f32,
) {
    let line =
        |world: &[Vec3], width: f32| polyline(painter, view, rect, world, color, shade, width);
    line(&along(a - up * reach, a + up * reach), 1.5);
    if r.line_symmetry && r.lines == 2 {
        let Some(plane) = r.surface_symmetry().and_then(|s| s.mirror) else {
            return;
        };
        let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(u, v)| {
            screen_at(
                view,
                rect,
                a + plane.axis_u * (u * reach) + plane.axis_v * (v * reach),
            )
        });
        if corners.iter().all(Option::is_some) {
            let c: Vec<Pos2> = corners.iter().flatten().copied().collect();
            painter.add(Shape::convex_polygon(
                c.clone(),
                dimmed(color, 0.13),
                Stroke::NONE,
            ));
            for i in 0..4 {
                let edge = [c[i], c[(i + 1) % 4]];
                painter.line_segment(
                    edge,
                    Stroke::new(3.0, Color32::from_black_alpha((90.0 * shade) as u8)),
                );
                painter.line_segment(edge, Stroke::new(1.5, color));
            }
        }
    } else {
        let d0 = first_direction(a, b, up);
        let lines = u32::from(r.lines.max(2));
        for k in 0..lines {
            let t = k as f32 / lines as f32 * std::f32::consts::TAU;
            let d = Quat::from_axis_angle(up, t) * d0;
            line(&along(a, a + d * reach), 1.5);
        }
    }
    if let Some(at) = screen_at(view, rect, a) {
        painter.circle_filled(at, 3.0, color);
    }
}

/// 3D ビューの上に、見えている 3D の定規を描く（ペイントのモード。選んでいるレイヤーから見えて効く物は濃く、効いていない特殊定規は薄く）。
pub fn paint_shown(painter: &Painter, app: &AppState, rect: Rect) {
    if app.view3d.model.is_none() {
        return;
    }
    for shown in app.rulers_shown(Place::View3d) {
        paint_ruler(
            painter,
            app,
            rect,
            shown.ruler,
            Look::Shown {
                effective: shown.effective,
                selected: shown.selected,
            },
        );
    }
}

/// 編集のモードの選んだ定規: 橙の線と、端の点の四角（G/R/S の途中も、文書へ当てた今の形）。
pub fn paint_selected(painter: &Painter, app: &AppState, rect: Rect, r: &Ruler) {
    paint_ruler(
        painter,
        app,
        rect,
        r,
        Look::Shown {
            effective: true,
            selected: true,
        },
    );
}

/// 端の点の四角（掴めるつまみ。`hover` はポインタを乗せているもの）。
pub fn paint_squares(
    painter: &Painter,
    app: &AppState,
    rect: Rect,
    r: &Ruler,
    hover: Option<super::edit3d::End>,
) {
    let view = camera_view(app, rect);
    for (end, p) in ends(r) {
        let Some(at) = screen_at(&view, rect, p.as_vec3()) else {
            continue;
        };
        let square = Rect::from_center_size(at, EVec2::splat(SQUARE_POINTS));
        painter.rect_filled(square.expand(1.0), 0.0, Color32::from_black_alpha(180));
        let fill = if hover == Some(end) {
            crate::view3d::shape_gizmo::HOVER
        } else {
            crate::objects::SELECTED
        };
        painter.rect_filled(square, 0.0, fill);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Action;
    use egui::pos2;
    use yolu_core::glam::DVec3;
    use yolu_core::RulerId;

    const RECT: Rect = Rect {
        min: pos2(0.0, 0.0),
        max: pos2(800.0, 600.0),
    };

    fn app() -> AppState {
        let mut app = AppState::new(64, 64);
        app.apply(Action::LoadDemoModel);
        app
    }

    fn collect(shape: &Shape, out: &mut Vec<Color32>) {
        match shape {
            Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            Shape::Path(path) => {
                out.push(path.fill);
                if let egui::epaint::ColorMode::Solid(color) = path.stroke.color {
                    out.push(color);
                }
            }
            Shape::Circle(circle) => {
                out.push(circle.fill);
                out.push(circle.stroke.color);
            }
            Shape::LineSegment { stroke, .. } => out.push(stroke.color),
            _ => {}
        }
    }

    /// 定規を描いて、出た図形の色（黒い縁と透明を除く）の全部を返す。
    fn painted(app: &AppState, r: &Ruler, look: Look) -> Vec<Color32> {
        let ctx = egui::Context::default();
        let mut colors = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(RECT),
                ..Default::default()
            },
            |ui| paint_ruler(&ui.painter().clone(), app, RECT, r, look),
        );
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut colors);
        }
        output.textures_delta.clear();
        colors
            .into_iter()
            .filter(|c| c.a() > 0 && (c.r(), c.g(), c.b()) != (0, 0, 0))
            .collect()
    }

    fn strongest(colors: &[Color32]) -> u8 {
        colors.iter().map(|c| c.a()).max().unwrap_or(0)
    }

    fn special(kind: RulerKind) -> Ruler {
        Ruler::model(
            RulerId(1),
            kind,
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(0.3, 0.0, 0.0),
            DVec3::Y,
        )
    }

    #[test]
    fn a_ruler_that_is_not_in_effect_is_drawn_with_a_fainter_color_than_one_in_effect() {
        let app = app();
        for kind in [
            RulerKind::Line,
            RulerKind::Parallel,
            RulerKind::Concentric,
            RulerKind::Perspective,
            RulerKind::Symmetry,
        ] {
            let r = special(kind);
            let shown = |effective: bool| {
                painted(
                    &app,
                    &r,
                    Look::Shown {
                        effective,
                        selected: false,
                    },
                )
            };
            let (on, off) = (shown(true), shown(false));
            assert!(!on.is_empty() && !off.is_empty(), "{kind:?}");
            assert_eq!(strongest(&on), 204, "{kind:?}: 効いている線");
            assert!(
                strongest(&off) < strongest(&on) / 2,
                "{kind:?}: 効いていない線は薄い {} {}",
                strongest(&off),
                strongest(&on)
            );
            // 引いている途中の形は、効いている濃さ
            assert_eq!(strongest(&painted(&app, &r, Look::Draft)), 204, "{kind:?}");
        }
    }

    #[test]
    fn the_mirror_square_fill_of_a_symmetry_ruler_not_in_effect_is_fainter_too() {
        let app = app();
        let mut r = special(RulerKind::Symmetry);
        r.lines = 2;
        let fills = |effective: bool| -> Vec<u8> {
            painted(
                &app,
                &r,
                Look::Shown {
                    effective,
                    selected: false,
                },
            )
            .into_iter()
            .map(|c| c.a())
            .filter(|a| *a < 100)
            .collect()
        };
        let (on, off) = (fills(true), fills(false));
        assert!(on.contains(&26), "効いている四角の塗り {on:?}");
        assert!(
            off.contains(&9) && !off.contains(&26),
            "効いていない四角の塗りは薄い（26 の 0.35 倍） {off:?}"
        );
    }

    #[test]
    fn the_selected_ruler_is_orange_whatever_its_effect() {
        let app = app();
        let r = special(RulerKind::Parallel);
        for effective in [true, false] {
            let colors = painted(
                &app,
                &r,
                Look::Shown {
                    effective,
                    selected: true,
                },
            );
            assert!(
                colors.iter().all(|c| *c == crate::objects::SELECTED),
                "{effective}: {colors:?}"
            );
        }
    }
}
