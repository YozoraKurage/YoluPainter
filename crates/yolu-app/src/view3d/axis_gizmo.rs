//! 3D ビューの軸の印: 表示域の右上のアイコンの下に、X・Y・Z の正と負の向きの丸。今の視点の向きで回して描き、手前の丸を上に重ねる。
//! 丸を押すとその向きから見る視点（`navigation::axis_view`。正の X は右・正の Y は上・正の Z は正面）、真ん中の丸は透視と正投影の
//! 切り替え、印の上のドラッグは視点を回す。設定のパネルとは重なるので、パネルが開いている間は出さない。文字は軸の名前だけで、
//! 押した先の名前はツールチップ。
//!
//! 別のレイヤー（`egui::Area`）に置くので、印の上の押しは 3D ビューの描く・回す入力に渡らない（隅のアイコンと同じ）。

use egui::{
    pos2, vec2, Align2, Color32, Id, Pos2, Rect, Sense, Stroke, Ui, WidgetInfo, WidgetType,
};
use yolu_core::geometry::{AxisView, OrbitCamera};
use yolu_core::glam::Vec3;

use super::navigation::{self, NavOp, OrbitCenter};
use super::shape_gizmo::{AXIS_X, AXIS_Y, AXIS_Z};
use crate::lang::Lang;
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::CORNER_MARGIN;

/// 印の一辺（点）。
pub const SIZE: f32 = 84.0;
/// 隅のアイコンの列との間。
const GAP: f32 = 4.0;
/// 中心から丸までの長さ。
const REACH: f32 = 30.0;
/// 正の向きの丸・負の向きの丸・真ん中の丸の半径。
const BALL: f32 = 9.0;
const NEG_BALL: f32 = 6.5;
const CENTER: f32 = 6.0;

/// 印の中の押せる所。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// その軸の向きから見る視点。
    Axis(AxisView),
    /// 透視と正投影の切り替え。
    Center,
}

/// 丸 1 つ。
#[derive(Clone, Copy, Debug)]
pub struct Ball {
    pub view: AxisView,
    /// 画面の位置。
    pub at: Pos2,
    /// 今のカメラの前の向きへの成分（大きいほど奥）。
    pub depth: f32,
    /// 軸の正の向き（X・Y・Z の名前を書く大きい丸）か。
    pub positive: bool,
}

impl Ball {
    fn radius(&self) -> f32 {
        if self.positive {
            BALL
        } else {
            NEG_BALL
        }
    }
}

/// 6 つの丸を、奥から手前の順に（描く順。手前の丸が上に重なる）。
pub fn balls(camera: &OrbitCamera, center: Pos2) -> Vec<Ball> {
    let q = camera.rotation();
    let (right, up, forward) = (q * Vec3::X, q * Vec3::Y, q * Vec3::Z);
    let mut out: Vec<Ball> = AxisView::ALL
        .into_iter()
        .map(|view| {
            let a = view.side();
            Ball {
                view,
                at: center + vec2(a.dot(right), -a.dot(up)) * REACH,
                depth: a.dot(forward),
                positive: a.x + a.y + a.z > 0.0,
            }
        })
        .collect();
    out.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    out
}

/// 印の矩形（表示域の右上、隅のアイコンの列の下 `below` から）。
pub fn rect(view: Rect, below: f32) -> Rect {
    Rect::from_min_size(
        pos2(view.right() - CORNER_MARGIN - SIZE, below + GAP),
        vec2(SIZE, SIZE),
    )
}

/// 点 p の下の押せる所。真ん中の丸が先（軸の向きから見ているとき、その軸の丸は真ん中に重なる）、次に手前の丸から。
pub fn hit(camera: &OrbitCamera, rect: Rect, p: Pos2) -> Option<Part> {
    let center = rect.center();
    if center.distance(p) <= CENTER + 1.5 {
        return Some(Part::Center);
    }
    balls(camera, center)
        .iter()
        .rev()
        .find(|b| b.at.distance(p) <= b.radius() + 1.5)
        .map(|b| Part::Axis(b.view))
}

/// 軸の色（X 赤・Y 緑・Z 青。3D のギズモの軸と同じ）。
fn color(view: AxisView) -> Color32 {
    match view {
        AxisView::Right | AxisView::Left => AXIS_X,
        AxisView::Top | AxisView::Bottom => AXIS_Y,
        AxisView::Front | AxisView::Back => AXIS_Z,
    }
}

fn letter(view: AxisView) -> &'static str {
    match view {
        AxisView::Right | AxisView::Left => "X",
        AxisView::Top | AxisView::Bottom => "Y",
        AxisView::Front | AxisView::Back => "Z",
    }
}

/// 押せる所の名前（ツールチップ・読み上げ。視点のパイと同じ操作の名前）。
pub fn name(part: Part, lang: Lang) -> &'static str {
    let id = match part {
        Part::Center => "view3d.ortho",
        Part::Axis(AxisView::Front) => "view3d.view_front",
        Part::Axis(AxisView::Back) => "view3d.view_back",
        Part::Axis(AxisView::Right) => "view3d.view_right",
        Part::Axis(AxisView::Left) => "view3d.view_left",
        Part::Axis(AxisView::Top) => "view3d.view_top",
        Part::Axis(AxisView::Bottom) => "view3d.view_bottom",
    };
    crate::commands::find(id)
        .and_then(|c| c.static_label(lang))
        .unwrap_or("")
}

/// 3D ビューの右上に印を描き、押し・ドラッグを当てる。`below` は隅のアイコンの列の下の端。設定のパネルが開いている・モデルが無いときは
/// 何もしない。
pub fn show(ui: &mut Ui, app: &mut AppState, view: Rect, below: f32) {
    if app.view3d.display.settings_open || app.view3d.model.is_none() {
        return;
    }
    let r = rect(view, below);
    if !view.contains_rect(r) {
        return;
    }
    let lang = app.lang;
    let movable = navigation::can_move_view(app);
    egui::Area::new(Id::new("yolu.view3d.axis_gizmo"))
        .order(egui::Order::Middle)
        .fixed_pos(r.min)
        .constrain(false)
        .show(ui.ctx(), |ui| {
            ui.allocate_exact_size(r.size(), Sense::hover());
            let id = Id::new("yolu.view3d.axis_gizmo.area");
            let response = ui.interact(
                r,
                id,
                if movable {
                    Sense::click_and_drag()
                } else {
                    Sense::hover()
                },
            );
            let camera = app.view3d.camera;
            let hovered = response
                .hover_pos()
                .filter(|_| !response.dragged())
                .and_then(|p| hit(&camera, r, p));
            paint(
                ui.painter(),
                &camera,
                r,
                hovered,
                response.hovered() || response.dragged(),
            );
            response.widget_info(|| {
                WidgetInfo::labeled(
                    WidgetType::Other,
                    movable,
                    lang.pick("視点の軸", "View axes"),
                )
            });
            if response.dragged() && movable {
                let d = response.drag_delta();
                if d != egui::Vec2::ZERO {
                    orbit(app, d);
                }
            }
            if response.clicked() {
                if let Some(part) = response
                    .interact_pointer_pos()
                    .and_then(|p| hit(&camera, r, p))
                {
                    let op = match part {
                        Part::Axis(v) => NavOp::Axis(v),
                        Part::Center => NavOp::ToggleOrthographic,
                    };
                    app.apply(Action::View3dNav(op));
                }
            }
            if let Some(part) = hovered {
                let tip = name(part, lang);
                if !tip.is_empty() {
                    response.on_hover_text(tip);
                }
            }
        });
}

/// 印のドラッグで回す（右ドラッグと同じ量。回す中心の設定がモデル・テクスチャセットの中心なら、その点のまわり）。軸から外れたら、
/// 軸の向きで自動に替えた正投影は戻す。
fn orbit(app: &mut AppState, d: egui::Vec2) {
    let pivot = navigation::pivot_without_press(app);
    if app.prefs.settings.navigation.orbit == OrbitCenter::View {
        app.view3d.camera.orbit(d.x, d.y);
    } else {
        app.view3d.camera.orbit_about(pivot, d.x, d.y);
    }
    navigation::after_orbit(&mut app.view3d, pivot);
    app.view3d.input.eyedrop = None;
}

fn paint(
    painter: &egui::Painter,
    camera: &OrbitCamera,
    r: Rect,
    hovered: Option<Part>,
    active: bool,
) {
    let center = r.center();
    if active {
        painter.circle_filled(center, SIZE * 0.5 - 1.0, Color32::from_white_alpha(22));
    }
    let balls = balls(camera, center);
    // 正の向きへの線（奥の丸より上、手前の丸より下に見えるよう、丸と同じ奥行きの順で描く）
    for b in &balls {
        let c = color(b.view);
        let hot = hovered == Some(Part::Axis(b.view));
        if b.positive {
            painter.line_segment([center, b.at], Stroke::new(2.0, c));
            painter.circle_filled(b.at, BALL, c);
            if hot {
                painter.circle_stroke(b.at, BALL, Stroke::new(1.5, Color32::WHITE));
            }
            painter.text(
                b.at,
                Align2::CENTER_CENTER,
                letter(b.view),
                t::LABEL_BOLD
                    .with_color(Color32::from_rgb(16, 16, 18))
                    .font(),
                Color32::from_rgb(16, 16, 18),
            );
        } else {
            painter.circle_filled(
                b.at,
                NEG_BALL,
                c.gamma_multiply(if hot { 0.75 } else { 0.4 }),
            );
            painter.circle_stroke(
                b.at,
                NEG_BALL,
                Stroke::new(1.2, if hot { Color32::WHITE } else { c }),
            );
        }
    }
    // 真ん中の丸（正投影のあいだは明るく塗る。軸の色と取り違えない色）
    let hot = hovered == Some(Part::Center);
    let fill = if camera.is_orthographic() {
        t::TEXT
    } else {
        t::PANEL_BG
    };
    painter.circle_filled(center, CENTER, fill);
    painter.circle_stroke(
        center,
        CENTER,
        Stroke::new(1.5, if hot { Color32::WHITE } else { t::TEXT_DIM }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::geometry::DEFAULT_YAW;

    fn camera(yaw: f32, pitch: f32) -> OrbitCamera {
        OrbitCamera {
            yaw,
            pitch,
            ..OrbitCamera::default()
        }
    }

    #[test]
    fn the_balls_follow_the_view_and_the_nearest_is_drawn_last() {
        let r = Rect::from_min_size(pos2(100.0, 50.0), vec2(SIZE, SIZE));
        let c = r.center();
        // 正面から: +Y は上、+Z は真ん中で手前、+X は横（左手系で正面から見ると、モデルの右は画面の左）
        let (yaw, pitch) = AxisView::Front.orientation();
        let list = balls(&camera(yaw, pitch), c);
        let at = |v: AxisView| list.iter().find(|b| b.view == v).unwrap().at;
        assert!((at(AxisView::Top) - (c + vec2(0.0, -REACH))).length() < 1e-3);
        assert!((at(AxisView::Bottom) - (c + vec2(0.0, REACH))).length() < 1e-3);
        assert!((at(AxisView::Front) - c).length() < 1e-3);
        assert!((at(AxisView::Right).y - c.y).abs() < 1e-3);
        assert!((at(AxisView::Right).x - (c.x - REACH)).abs() < 1e-3);
        assert!((at(AxisView::Left).x - (c.x + REACH)).abs() < 1e-3);
        assert_eq!(list.last().unwrap().view, AxisView::Front, "手前が最後");
        assert_eq!(list.first().unwrap().view, AxisView::Back, "奥が最初");
        // 既定の斜めの視点: 全部の丸が、中心から REACH 以内
        for b in balls(&camera(DEFAULT_YAW, 10.0), c) {
            assert!(b.at.distance(c) <= REACH + 1e-3);
        }
    }

    #[test]
    fn the_center_wins_over_a_ball_on_it_and_balls_are_hit_front_first() {
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(SIZE, SIZE));
        let (yaw, pitch) = AxisView::Front.orientation();
        let cam = camera(yaw, pitch);
        assert_eq!(hit(&cam, r, r.center()), Some(Part::Center));
        assert_eq!(
            hit(&cam, r, r.center() + vec2(0.0, -REACH)),
            Some(Part::Axis(AxisView::Top))
        );
        assert_eq!(hit(&cam, r, r.min), None);
        // 上から見ると、+Y が真ん中の奥（手前）で、正面（+Z）は画面の下
        let (yaw, pitch) = AxisView::Top.orientation();
        let cam = camera(yaw, pitch);
        let front = balls(&cam, r.center())
            .into_iter()
            .find(|b| b.view == AxisView::Front)
            .unwrap();
        assert_eq!(hit(&cam, r, front.at), Some(Part::Axis(AxisView::Front)));
    }

    #[test]
    fn every_part_has_a_name_in_both_languages() {
        for lang in [Lang::Ja, Lang::En] {
            for part in AxisView::ALL
                .into_iter()
                .map(Part::Axis)
                .chain([Part::Center])
            {
                assert!(!name(part, lang).is_empty(), "{part:?}");
            }
        }
    }
}
