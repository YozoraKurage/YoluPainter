//! ステンシルの、1 つの表示域での置き場（Unity 版の `StencilFrame`）: 中心（画面の点）、画面の上の幅・高さ（点）、角度（時計回りが正）。
//! 画面の点 → 画像の画素（左下が原点、y は上向き）の写しと、その係数・回した 4 隅。表示域は画面に貼り付いた枠で、2D のキャンバスも
//! 3D のビューも同じ割合の置き場（中心は表示域に対する割合、大きさは表示域の高さに対する割合）を、自分の表示域の大きさで解く。

use egui::{pos2, Pos2, Rect};

use crate::canvas::view::normalize_angle;

/// 初めの置き場（中心・表示域の高さに対する大きさ）と重ねの不透明度、大きさの範囲。
pub const DEFAULT_CENTER: [f32; 2] = [0.5, 0.5];
pub const DEFAULT_SIZE: f32 = 0.6;
pub const DEFAULT_OPACITY: f32 = 0.5;
pub const MIN_SIZE: f32 = 0.02;
pub const MAX_SIZE: f32 = 20.0;
/// プロパティのスライダーが出す大きさの上限（ドラッグの拡大はもっと大きくできる）。
pub const SLIDER_MAX_SIZE: f32 = 4.0;
/// Shift を押して回すときの刻み（度）。
pub const ROTATE_STEP: f32 = 15.0;

/// 90° の倍数は cos・sin がちょうどの値になる。
fn cos_sin(degrees: f32) -> (f64, f64) {
    let a = normalize_angle(degrees);
    if a == 0.0 {
        (1.0, 0.0)
    } else if a == 90.0 {
        (0.0, 1.0)
    } else if a == 180.0 {
        (-1.0, 0.0)
    } else if a == -90.0 {
        (0.0, -1.0)
    } else {
        let r = a as f64 * std::f64::consts::PI / 180.0;
        (r.cos(), r.sin())
    }
}

/// ある表示域での置き場。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StencilFrame {
    pub center: (f64, f64),
    pub width: f64,
    pub height: f64,
    cos: f64,
    sin: f64,
    pub image_width: usize,
    pub image_height: usize,
}

impl StencilFrame {
    /// view は表示域（画面の点）、center は表示域に対する中心（左上が 0, 0・右下が 1, 1）、size は表示域の高さに対する画像の高さ。
    pub fn new(
        view: Rect,
        center: [f32; 2],
        size: f32,
        angle: f32,
        image_width: usize,
        image_height: usize,
    ) -> StencilFrame {
        let height = (size as f64 * view.height() as f64).max(1e-3);
        let (cos, sin) = cos_sin(angle);
        StencilFrame {
            center: (
                view.left() as f64 + center[0] as f64 * view.width() as f64,
                view.top() as f64 + center[1] as f64 * view.height() as f64,
            ),
            width: height * image_width as f64 / image_height as f64,
            height,
            cos,
            sin,
            image_width,
            image_height,
        }
    }

    /// 画像の画素 1 つの画面の点に対する比（画像の画素 / 点）。
    pub fn image_per_point(&self) -> f64 {
        self.image_height as f64 / self.height
    }

    /// 画面の点 (gx, gy) の、画像の画素の座標（左下が原点、y は上向き。画像の外も返す）。
    pub fn to_image(&self, gx: f64, gy: f64) -> (f64, f64) {
        let (dx, dy) = (gx - self.center.0, gy - self.center.1);
        let (lx, ly) = (
            self.cos * dx + self.sin * dy,
            -self.sin * dx + self.cos * dy,
        ); // 角度を戻す
        (
            (lx / self.width + 0.5) * self.image_width as f64,
            (0.5 - ly / self.height) * self.image_height as f64,
        )
    }

    /// [`StencilFrame::to_image`] の係数 [xx, xy, x0, yx, yy, y0]: x = xx·gx + xy·gy + x0、y = yx·gx + yy·gy + y0。
    pub fn image_affine(&self) -> [f64; 6] {
        let kx = self.image_width as f64 / self.width;
        let ky = self.image_height as f64 / self.height;
        let (cx, cy) = self.center;
        let (xx, xy) = (kx * self.cos, kx * self.sin);
        let x0 = self.image_width as f64 * 0.5 - xx * cx - xy * cy;
        let (yx, yy) = (ky * self.sin, -ky * self.cos);
        let y0 = self.image_height as f64 * 0.5 - yx * cx - yy * cy;
        [xx, xy, x0, yx, yy, y0]
    }

    /// 中心からの（回していない）画面の向きのずれ (x, y)（y は下向き）の、回した点。
    pub fn rotated(&self, x: f64, y: f64) -> Pos2 {
        pos2(
            (self.center.0 + self.cos * x - self.sin * y) as f32,
            (self.center.1 + self.sin * x + self.cos * y) as f32,
        )
    }

    /// 画像 1 枚の 4 隅（左上・右上・右下・左下。回した後の画面の点）。
    pub fn corners(&self) -> [Pos2; 4] {
        let (hw, hh) = (self.width * 0.5, self.height * 0.5);
        [
            self.rotated(-hw, -hh),
            self.rotated(hw, -hh),
            self.rotated(hw, hh),
            self.rotated(-hw, hh),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    #[test]
    fn the_center_maps_to_the_middle_of_the_image_and_the_top_to_the_top() {
        let view = Rect::from_min_size(pos2(100.0, 50.0), vec2(400.0, 300.0));
        let f = StencilFrame::new(view, DEFAULT_CENTER, 0.5, 0.0, 200, 100);
        // 画面の中心 (300, 200)。高さは 150 点、幅は 300 点
        assert!(close(f.to_image(300.0, 200.0), (100.0, 50.0)));
        // 画像の上の辺（中心の 75 点上）は y = 100（画像の上）、左の辺は x = 0
        assert!(close(f.to_image(150.0, 125.0), (0.0, 100.0)));
        assert!(close(f.to_image(450.0, 275.0), (200.0, 0.0)));
        assert!((f.image_per_point() - 100.0 / 150.0).abs() < 1e-12);
    }

    #[test]
    fn a_clockwise_quarter_turn_moves_the_top_of_the_image_to_the_right() {
        let view = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 400.0));
        let f = StencilFrame::new(view, DEFAULT_CENTER, 0.5, 90.0, 100, 100);
        // 画像の上の中央は、画面の中心の右（時計回りに 90°）
        let top = f.rotated(0.0, -f.height * 0.5);
        assert!(
            (top.x - 300.0).abs() < 1e-4 && (top.y - 200.0).abs() < 1e-4,
            "{top:?}"
        );
        assert!(close(f.to_image(top.x as f64, top.y as f64), (50.0, 100.0)));
        let corners = f.corners();
        assert!((corners[0].x - 300.0).abs() < 1e-3 && (corners[0].y - 100.0).abs() < 1e-3);
    }

    #[test]
    fn the_affine_is_the_same_map_as_to_image() {
        let view = Rect::from_min_size(pos2(30.0, 20.0), vec2(640.0, 480.0));
        for angle in [0.0, 15.0, 90.0, -90.0, 180.0, -37.5] {
            let f = StencilFrame::new(view, [0.4, 0.55], 0.7, angle, 321, 123);
            let [xx, xy, x0, yx, yy, y0] = f.image_affine();
            for (gx, gy) in [(30.0, 20.0), (400.0, 123.0), (-50.0, 900.0)] {
                let (x, y) = f.to_image(gx, gy);
                assert!(
                    close((xx * gx + xy * gy + x0, yx * gx + yy * gy + y0), (x, y)),
                    "{angle}"
                );
            }
        }
    }
}
