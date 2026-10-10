//! 選択の形の幾何（ドラッグの 2 つの角の決め方・角を丸めた長方形の輪郭）。画面にも文書にも触れない計算だけ。

use egui::Modifiers;

use super::SelState;
use crate::engine::DVec2;
use crate::state::Tool;

/// 長方形・楕円のドラッグの 2 つの角（押した点 `start`・今の点 `current`。キャンバスの座標）。`square` なら縦横を同じ長さに（長い方に合わせ、
/// 動かした向きを保つ）、`center` なら押した点が中心になるように反対側へ同じだけ広げる。返すのは（もう一方の角, 今の点の角）。
pub fn drag_corners(
    start: (f64, f64),
    current: (f64, f64),
    square: bool,
    center: bool,
) -> ((f64, f64), (f64, f64)) {
    let (mut dx, mut dy) = (current.0 - start.0, current.1 - start.1);
    if square {
        let m = dx.abs().max(dy.abs());
        dx = if dx < 0.0 { -m } else { m };
        dy = if dy < 0.0 { -m } else { m };
    }
    let b = (start.0 + dx, start.1 + dy);
    let a = if center {
        (start.0 - dx, start.1 - dy)
    } else {
        start
    };
    (a, b)
}

/// ドラッグ中・離したときの形の決め方（縦横比・中心から）。設定が常に効き、Shift は押し始めたあとに押したときだけ縦横比を固定、
/// Alt は押しているあいだ中心から（Photoshop・CLIP STUDIO と同じ。押し始めに押した Shift は「追加」の合図なので縦横比には使わない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Constraint {
    pub square: bool,
    pub center: bool,
}

impl Constraint {
    pub fn of(sel: &SelState, tool: Tool, pressed_with: Modifiers, now: Modifiers) -> Constraint {
        let shape_tool = matches!(tool, Tool::SelectRect | Tool::SelectEllipse);
        Constraint {
            square: shape_tool && (sel.fixed_ratio || shift_constrains(pressed_with, now)),
            center: shape_tool && (sel.from_center || now.alt),
        }
    }
}

/// 縦横比を固定する・中心から広げるときの修飾キー（`Constraint::of` が読むキー。割り当ての表には無い固定のキーなので、ツールチップの文字はここから引く）。
pub const RATIO_KEY: &str = "Shift";
pub const CENTER_KEY: &str = "Alt";

/// Shift を押し始めたあとに押した（押し始めには押していなかった）か。
pub fn shift_constrains(pressed_with: Modifiers, now: Modifiers) -> bool {
    !pressed_with.shift && now.shift
}

/// 角を丸めた長方形（画素の座標 x0..x1 × y0..y1。逆向きの角は入れ替える）の輪郭の点。角の半径は短い辺の半分までに丸める。
/// 半径が 0 なら 4 隅だけ。反時計回り。
pub fn rounded_rect_points(x0: i64, y0: i64, x1: i64, y1: i64, radius: u32) -> Vec<DVec2> {
    rounded_rect_outline(x0 as f64, y0 as f64, x1 as f64, y1 as f64, radius as f64)
}

/// `rounded_rect_points` の、小数の座標の版（3D ビューの画面の点で引く長方形の輪郭）。
pub fn rounded_rect_outline(x0: f64, y0: f64, x1: f64, y1: f64, radius: f64) -> Vec<DVec2> {
    let (x0, x1) = if x1 < x0 { (x1, x0) } else { (x0, x1) };
    let (y0, y1) = if y1 < y0 { (y1, y0) } else { (y0, y1) };
    let r = radius.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0);
    if r <= 0.0 {
        return vec![
            DVec2::new(x0, y0),
            DVec2::new(x1, y0),
            DVec2::new(x1, y1),
            DVec2::new(x0, y1),
        ];
    }
    // 1 つの角を、弧の長さに応じた数の辺で近づける（半径 100 で 50 辺、頭打ち 64）
    let n = ((r * 0.5).ceil() as usize).clamp(6, 64);
    let mut points = Vec::with_capacity(4 * (n + 1));
    for (cx, cy, from) in [
        (x1 - r, y0 + r, -std::f64::consts::FRAC_PI_2),
        (x1 - r, y1 - r, 0.0),
        (x0 + r, y1 - r, std::f64::consts::FRAC_PI_2),
        (x0 + r, y0 + r, std::f64::consts::PI),
    ] {
        for i in 0..=n {
            let t = from + std::f64::consts::FRAC_PI_2 * i as f64 / n as f64;
            points.push(DVec2::new(cx + r * t.cos(), cy + r * t.sin()));
        }
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_drag_follows_the_longer_side_and_keeps_the_direction() {
        let (a, b) = drag_corners((10.0, 10.0), (50.0, 20.0), true, false);
        assert_eq!((a, b), ((10.0, 10.0), (50.0, 50.0)));
        let (a, b) = drag_corners((10.0, 10.0), (-5.0, -40.0), true, false);
        assert_eq!((a, b), ((10.0, 10.0), (-40.0, -40.0)));
        // 動いていない向きは正の向き
        let (_, b) = drag_corners((10.0, 10.0), (10.0, 30.0), true, false);
        assert_eq!(b, (30.0, 30.0));
    }

    #[test]
    fn from_the_center_mirrors_the_corner_around_the_press() {
        let (a, b) = drag_corners((100.0, 100.0), (130.0, 90.0), false, true);
        assert_eq!((a, b), ((70.0, 110.0), (130.0, 90.0)));
        let (a, b) = drag_corners((100.0, 100.0), (130.0, 90.0), true, true);
        assert_eq!((a, b), ((70.0, 130.0), (130.0, 70.0)));
    }

    #[test]
    fn a_shift_held_from_the_press_is_the_add_gesture_not_the_aspect_lock() {
        let shift = Modifiers::SHIFT;
        assert!(!shift_constrains(shift, shift));
        assert!(shift_constrains(Modifiers::NONE, shift));
        assert!(!shift_constrains(Modifiers::NONE, Modifiers::NONE));
    }

    #[test]
    fn a_rounded_rectangle_stays_inside_its_box_and_the_radius_is_clamped() {
        let points = rounded_rect_points(10, 20, 110, 70, 1000);
        assert!(points.iter().all(|p| p.x >= 10.0 - 1e-9
            && p.x <= 110.0 + 1e-9
            && p.y >= 20.0 - 1e-9
            && p.y <= 70.0 + 1e-9));
        // 半径は短い辺（50）の半分の 25 まで: 上下の辺の直線は 50 画素残る
        let top: Vec<f64> = points
            .iter()
            .filter(|p| (p.y - 70.0).abs() < 1e-9)
            .map(|p| p.x)
            .collect();
        assert!(top.iter().cloned().fold(f64::MAX, f64::min) >= 35.0 - 1e-9);
        // 半径 0 は 4 隅
        assert_eq!(rounded_rect_points(0, 0, 8, 4, 0).len(), 4);
        // 逆向きの角は入れ替える
        assert_eq!(
            rounded_rect_points(8, 4, 0, 0, 2),
            rounded_rect_points(0, 0, 8, 4, 2)
        );
    }
}
