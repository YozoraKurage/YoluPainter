//! スポイトの印: スポイトで色を取っている間（スポイトのツールでポインタを乗せている間・2D で右ボタンを押している間・3D で右ボタンを押して
//! まだ動かしていない間）、ポインタに出す。ポインタの周りの輪（上半分が今の色、下半分がポインタの下の色。スカラーのチャンネルは灰色の濃さ）と、
//! 右上のスポイトの絵（白に黒の縁）。OS の矢印は出さない（呼ぶ側が隠す）。文は出さない。

use egui::{pos2, vec2, Color32, Painter, Pos2, Rect, Shape, Stroke};

/// 輪の半径（輪の太さの真ん中。画面の点）と、輪の太さ。
pub const RING_RADIUS: f32 = 28.0;
pub const RING_WIDTH: f32 = 7.0;
/// スポイトの絵の大きさ。ポインタの右上に、絵の左下の先がポインタの位置の近くへ来るように置く。
const ICON_SIZE: f32 = 22.0;
/// ポインタの下に色が無いとき（透明・キャンバスの外・読めない）の下半分。
const NO_SAMPLE: Color32 = Color32::from_rgba_premultiplied(45, 45, 45, 90);

/// `at` に印を描く。`current` は今の色、`sample` はポインタの下の色（無ければ None）。
pub fn paint(painter: &Painter, at: Pos2, current: Color32, sample: Option<Color32>) {
    let outline = Stroke::new(RING_WIDTH + 3.0, Color32::from_black_alpha(150));
    painter.circle_stroke(at, RING_RADIUS, outline);
    // 上半分が今の色、下半分がポインタの下の色（円弧は左から時計回りに上を通る / 右から下を通って左まで）
    painter.add(Shape::line(
        arc(at, std::f32::consts::PI, std::f32::consts::TAU),
        Stroke::new(RING_WIDTH, current),
    ));
    painter.add(Shape::line(
        arc(at, 0.0, std::f32::consts::PI),
        Stroke::new(RING_WIDTH, sample.unwrap_or(NO_SAMPLE)),
    ));
    // 明るい細い縁（暗い色でも輪が読めるように）
    let rim = Stroke::new(1.0, Color32::from_white_alpha(190));
    painter.circle_stroke(at, RING_RADIUS + RING_WIDTH / 2.0, rim);
    painter.circle_stroke(at, RING_RADIUS - RING_WIDTH / 2.0, rim);
    // ポインタの位置
    painter.circle_filled(at, 2.5, Color32::from_black_alpha(170));
    painter.circle_filled(at, 1.5, Color32::WHITE);
    // スポイトの絵（白に黒の縁）
    let center = at + vec2(ICON_SIZE / 2.0 + 1.5, -(ICON_SIZE / 2.0 + 1.5));
    let icon_at = |offset: Pos2| Rect::from_center_size(offset, vec2(ICON_SIZE, ICON_SIZE));
    for (dx, dy) in [
        (-1.0, 0.0),
        (1.0, 0.0),
        (0.0, -1.0),
        (0.0, 1.0),
        (-1.0, -1.0),
        (1.0, -1.0),
        (-1.0, 1.0),
        (1.0, 1.0),
    ] {
        crate::ui::icons::paint(
            painter,
            icon_at(pos2(center.x + dx, center.y + dy)),
            "tools/eyedropper",
            Color32::BLACK,
            ICON_SIZE,
        );
    }
    crate::ui::icons::paint(
        painter,
        icon_at(center),
        "tools/eyedropper",
        Color32::WHITE,
        ICON_SIZE,
    );
}

/// 輪の円弧の点（角度は画面で時計回りが正、0 が右、π が左）。
fn arc(center: Pos2, from: f32, to: f32) -> Vec<Pos2> {
    const SEGMENTS: usize = 32;
    (0..=SEGMENTS)
        .map(|i| {
            let a = from + (to - from) * i as f32 / SEGMENTS as f32;
            center + vec2(a.cos(), a.sin()) * RING_RADIUS
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 印を描いて、出た図形（輪の半円の線）の色を返す。
    fn painted(current: Color32, sample: Option<Color32>) -> Vec<(Color32, f32, Vec<Pos2>)> {
        let ctx = egui::Context::default();
        let mut lines = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0))),
                ..Default::default()
            },
            |ui| {
                paint(&ui.painter().clone(), pos2(100.0, 100.0), current, sample);
            },
        );
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut lines);
        }
        output.textures_delta.clear();
        lines
    }

    fn collect(shape: &Shape, out: &mut Vec<(Color32, f32, Vec<Pos2>)>) {
        match shape {
            Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            Shape::Path(path) => {
                if let egui::epaint::PathStroke {
                    width,
                    color: egui::epaint::ColorMode::Solid(color),
                    ..
                } = path.stroke
                {
                    out.push((color, width, path.points.clone()));
                }
            }
            _ => {}
        }
    }

    #[test]
    fn the_top_half_of_the_ring_is_the_current_color_and_the_bottom_half_is_the_one_under_the_pointer(
    ) {
        let current = Color32::from_rgb(10, 20, 30);
        let under = Color32::from_rgb(200, 100, 50);
        let lines = painted(current, Some(under));
        let half = |color: Color32| {
            lines
                .iter()
                .find(|(c, w, _)| *c == color && (*w - RING_WIDTH).abs() < 1e-3)
                .unwrap_or_else(|| panic!("{color:?} の半円がありません"))
                .2
                .clone()
        };
        // 上半分は中心より上（y が小さい）だけ、下半分は中心より下だけ
        assert!(half(current).iter().all(|p| p.y <= 100.0 + 1e-3));
        assert!(half(under).iter().all(|p| p.y >= 100.0 - 1e-3));
        // 半円は、ポインタを中心に輪の半径の円周上にある
        for p in half(current).iter().chain(half(under).iter()) {
            assert!((p.distance(pos2(100.0, 100.0)) - RING_RADIUS).abs() < 1e-3);
        }
    }

    #[test]
    fn a_pointer_over_nothing_leaves_the_bottom_half_unfilled_not_the_current_color() {
        let current = Color32::from_rgb(10, 20, 30);
        let lines = painted(current, None);
        let halves: Vec<_> = lines
            .iter()
            .filter(|(_, w, _)| (*w - RING_WIDTH).abs() < 1e-3)
            .collect();
        assert_eq!(halves.len(), 2);
        assert_eq!(halves.iter().filter(|(c, _, _)| *c == current).count(), 1);
        assert!(halves.iter().any(|(c, _, _)| *c == NO_SAMPLE));
    }
}
