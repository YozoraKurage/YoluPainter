//! 2D キャンバスの表示の写し（Unity 版の `CanvasView` と同じ式）: キャンバスの画素の座標（左下が原点、y は上向き）と画面の座標
//! （y は下向き）の間。文書を表示域に収める大きさ × 拡大率で縮め、表示域の中心 + パンを画像の中心に置き、画像の中心のまわりで
//! 左右を反転してから回す。回転は画面の上で時計回りが正。表示だけの写しで、正本は画素の座標のまま。
//! 回転も反転も無いときは軸に沿った式、回っているときは f64 で求め、90° の倍数の cos・sin はちょうどの値を使う。

use egui::{pos2, vec2, Pos2, Rect, Vec2};

use crate::engine::{DVec2, Tilt};

pub const ROTATE_STEP: f32 = 15.0;
// 全体フィットに対する倍率。最大32768画素の文書を1点の表示域から100%にしても、
// 次の拡大を扱える範囲。小さい文書の100%も同じ入口で扱う。
pub const MIN_ZOOM: f32 = 1.0 / 65536.0;
pub const MAX_ZOOM: f32 = 65536.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasView {
    /// 回転を戻した（反転しない）ときの画像の矩形。
    pub image: Rect,
    /// 画面の上の回転（度、時計回りが正、(-180, 180]）。
    pub angle: f32,
    pub flip: bool,
    width: u32,
    height: u32,
    cos: f64,
    sin: f64,
    scale: f64,
    cx: f64,
    cy: f64,
}

/// 角度を (-180, 180] に。
pub fn normalize_angle(degrees: f32) -> f32 {
    if !degrees.is_finite() {
        return 0.0;
    }
    let t = degrees + 180.0;
    let a = t - (t / 360.0).floor() * 360.0 - 180.0;
    if a <= -180.0 {
        180.0
    } else if a == 0.0 {
        0.0
    } else {
        a
    }
}

fn cos_sin(degrees: f32) -> (f64, f64) {
    if degrees == 0.0 {
        (1.0, 0.0)
    } else if degrees == 90.0 {
        (0.0, 1.0)
    } else if degrees == 180.0 {
        (-1.0, 0.0)
    } else if degrees == -90.0 {
        (0.0, -1.0)
    } else {
        let r = degrees as f64 * std::f64::consts::PI / 180.0;
        (r.cos(), r.sin())
    }
}

impl CanvasView {
    pub fn new(
        view: Rect,
        width: u32,
        height: u32,
        zoom: f32,
        pan: Vec2,
        angle: f32,
        flip: bool,
    ) -> CanvasView {
        let fit = (view.width() / width as f32).min(view.height() / height as f32) * zoom;
        let (w, h) = (width as f32 * fit, height as f32 * fit);
        let image = Rect::from_min_size(
            pos2(
                view.center().x - w * 0.5 + pan.x,
                view.center().y - h * 0.5 + pan.y,
            ),
            vec2(w, h),
        );
        let angle = normalize_angle(angle);
        let (cos, sin) = cos_sin(angle);
        CanvasView {
            image,
            angle,
            flip,
            width,
            height,
            cos,
            sin,
            scale: image.width() as f64 / width as f64,
            cx: image.left() as f64 + image.width() as f64 * 0.5,
            cy: image.top() as f64 + image.height() as f64 * 0.5,
        }
    }

    pub fn axis_aligned(&self) -> bool {
        self.angle == 0.0 && !self.flip
    }

    /// 画素 1 つの画面の大きさ（点）。
    pub fn pixel_size(&self) -> f32 {
        self.image.width() / self.width as f32
    }

    pub fn center(&self) -> Pos2 {
        pos2(self.cx as f32, self.cy as f32)
    }

    /// キャンバスの点（画素の座標、範囲外も可）の画面の座標。
    pub fn to_screen(&self, x: f64, y: f64) -> Pos2 {
        if self.axis_aligned() {
            let i = self.image;
            return pos2(
                i.left() + (x as f32) / self.width as f32 * i.width(),
                i.top() + (1.0 - (y as f32) / self.height as f32) * i.height(),
            );
        }
        let mut ux = self.scale * (x - self.width as f64 * 0.5);
        let uy = -self.scale * (y - self.height as f64 * 0.5);
        if self.flip {
            ux = -ux;
        }
        pos2(
            (self.cx + self.cos * ux - self.sin * uy) as f32,
            (self.cy + self.sin * ux + self.cos * uy) as f32,
        )
    }

    /// キャンバスの画素の座標 → 画面の座標の係数 [a, b, c, d, e, g]: 画面の x = a·x + b·y + c、y = d·x + e·y + g（`to_screen` と同じ写し）。
    /// 画像を画面に貼ったステンシルなど、画面を通る写しを 1 つの行列にまとめるのに使う。
    pub fn screen_affine(&self) -> [f64; 6] {
        let f = if self.flip { -1.0 } else { 1.0 };
        let (w, h, s) = (self.width as f64, self.height as f64, self.scale);
        [
            self.cos * f * s,
            self.sin * s,
            self.cx - self.cos * f * s * w * 0.5 - self.sin * s * h * 0.5,
            self.sin * f * s,
            -self.cos * s,
            self.cy - self.sin * f * s * w * 0.5 + self.cos * s * h * 0.5,
        ]
    }

    /// 画面の向き（y は下向き）をキャンバスの向き（y は上向き）に。回転・反転・y の向きを直し、長さは保つ（拡大は掛けない）。
    pub fn direction_to_canvas(&self, dx: f64, dy: f64) -> (f64, f64) {
        let ux = self.cos * dx + self.sin * dy;
        let uy = -self.sin * dx + self.cos * dy;
        (if self.flip { -ux } else { ux }, -uy)
    }

    /// ペンの傾き（度。画面の右・下へ倒れる向きが正）を、キャンバスの軸ごとの直立からの角度（ラジアン）に。倒れた量は変わらず、
    /// 向きだけ表示の回転・反転とキャンバスの y の向きに合わせて直す（倒れた向きの傾き (tan θx, tan θy) を回してから角度に戻す）。
    pub fn tilt_to_canvas(&self, tilt: Tilt) -> DVec2 {
        const MAX: f64 = std::f64::consts::FRAC_PI_2 - 1e-6;
        let tx = (tilt.x as f64).to_radians().clamp(-MAX, MAX).tan();
        let ty = (tilt.y as f64).to_radians().clamp(-MAX, MAX).tan();
        let (cx, cy) = self.direction_to_canvas(tx, ty);
        DVec2::new(cx.atan(), cy.atan())
    }

    /// ペンの軸まわりの回転（画面で時計回りの度）を、キャンバスで反時計回りのラジアンに。表示を回している・反転しているときは
    /// 0 度も 0 にならない（向きが変わるので）。回転の情報が無い入力は、これを呼ばずに core へ回転を渡さない。
    pub fn rotation_to_canvas(&self, degrees: f32) -> f64 {
        let a = -(degrees as f64).to_radians(); // 画面で反時計回り
        let (cx, cy) = self.direction_to_canvas(a.cos(), -a.sin());
        cy.atan2(cx)
    }

    /// 画面の座標をキャンバスの画素の座標（左下が原点、範囲外も返す）に。ストロークの点はこれ。
    pub fn to_canvas(&self, p: Pos2) -> (f64, f64) {
        if self.axis_aligned() {
            let i = self.image;
            return (
                ((p.x - i.left()) / i.width() * self.width as f32) as f64,
                ((1.0 - (p.y - i.top()) / i.height()) * self.height as f32) as f64,
            );
        }
        let (dx, dy) = (p.x as f64 - self.cx, p.y as f64 - self.cy);
        let mut ux = self.cos * dx + self.sin * dy;
        let uy = -self.sin * dx + self.cos * dy;
        if self.flip {
            ux = -ux;
        }
        (
            ux / self.scale + self.width as f64 * 0.5,
            -uy / self.scale + self.height as f64 * 0.5,
        )
    }
}

/// 表示の状態（拡大・パン・回転・反転）。文書には入れない。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewState {
    pub zoom: f32,
    pub pan: Vec2,
    pub angle: f32,
    pub flip: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            zoom: 1.0,
            pan: Vec2::ZERO,
            angle: 0.0,
            flip: false,
        }
    }
}

fn rotate_pan(pan: Vec2, degrees: f32) -> Vec2 {
    let a = normalize_angle(degrees);
    if a == 0.0 {
        return pan;
    }
    let (c, s) = cos_sin(a);
    vec2(
        (c * pan.x as f64 - s * pan.y as f64) as f32,
        (s * pan.x as f64 + c * pan.y as f64) as f32,
    )
}

impl ViewState {
    pub fn view(&self, rect: Rect, width: u32, height: u32) -> CanvasView {
        CanvasView::new(
            rect, width, height, self.zoom, self.pan, self.angle, self.flip,
        )
    }

    /// 角度を変え、表示域の中心にある画素が動かないようにパンも同じだけ回す。
    pub fn set_angle(&mut self, degrees: f32) {
        let next = normalize_angle(degrees);
        self.pan = rotate_pan(self.pan, next - self.angle);
        self.angle = next;
    }

    pub fn rotate_by(&mut self, degrees: f32) {
        self.set_angle(self.angle + degrees);
    }

    /// 回し始めの角度とパンから、回した量だけ（回すドラッグ）。
    pub fn rotate_from(&mut self, start_angle: f32, start_pan: Vec2, swept: f32) {
        self.angle = normalize_angle(start_angle + swept);
        self.pan = rotate_pan(start_pan, self.angle - start_angle);
    }

    /// 左右に反転する（表示域の中心の縦の線で鏡に映す。角度の符号が変わる）。
    pub fn flip_horizontally(&mut self) {
        self.flip = !self.flip;
        self.angle = normalize_angle(-self.angle);
        self.pan.x = -self.pan.x;
    }

    /// 画面に合わせる: 拡大 100%・パンなし・回転 0（反転はそのまま）。
    pub fn fit(&mut self) {
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
        self.angle = 0.0;
    }

    /// 文書の 1 画素を画面の 1 点で表示する。表示域の中心の画素を保つ。
    pub fn actual_size(&mut self, rect: Rect, width: u32, height: u32) {
        let fit = (rect.width() / width as f32).min(rect.height() / height as f32);
        if fit.is_finite() && fit > 0.0 {
            self.zoom_to(1.0 / fit, None, rect);
        }
    }

    /// 拡大率を変える。pointer の下の画素は動かない（None なら表示域の中心）。
    pub fn zoom_to(&mut self, zoom: f32, pointer: Option<Pos2>, rect: Rect) {
        let next = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if next == self.zoom {
            return;
        }
        let k = next / self.zoom;
        let p = pointer.unwrap_or(rect.center()) - rect.center();
        self.pan = p - k * (p - self.pan);
        self.zoom = next;
    }
}

/// 見出しに出す角度（整数ならそのまま、そうでなければ小数 1 桁）。
pub fn angle_label(degrees: f32) -> String {
    let rounded = (degrees * 10.0).round() / 10.0;
    if (rounded - rounded.round()).abs() < 1e-4 {
        format!("{}°", rounded.round() as i32)
    } else {
        format!("{rounded:.1}°")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn axis_aligned_maps_bottom_left_origin() {
        let v = CanvasView::new(
            Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 100.0)),
            100,
            100,
            1.0,
            Vec2::ZERO,
            0.0,
            false,
        );
        assert_eq!(
            v.image,
            Rect::from_min_size(pos2(50.0, 0.0), vec2(100.0, 100.0))
        );
        assert_eq!(v.to_screen(0.0, 0.0), pos2(50.0, 100.0)); // 左下の画素の角は画面の下
        assert!(close(v.to_canvas(pos2(60.0, 10.0)), (10.0, 90.0)));
    }

    #[test]
    fn rotation_and_flip_round_trip() {
        for angle in [0.0, 15.0, 90.0, -90.0, 180.0, 33.3] {
            for flip in [false, true] {
                let v = CanvasView::new(
                    Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 200.0)),
                    256,
                    128,
                    1.7,
                    vec2(12.0, -5.0),
                    angle,
                    flip,
                );
                let s = v.to_screen(37.25, 99.5);
                assert!(
                    close(v.to_canvas(s), (37.25, 99.5)),
                    "angle {angle} flip {flip}"
                );
            }
        }
    }

    #[test]
    fn rotating_keeps_the_center_pixel() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        let mut s = ViewState {
            zoom: 2.0,
            pan: vec2(40.0, 25.0),
            ..Default::default()
        };
        let before = s.view(rect, 512, 512).to_canvas(rect.center());
        s.rotate_by(30.0);
        let after = s.view(rect, 512, 512).to_canvas(rect.center());
        assert!(close(before, after));
        s.flip_horizontally();
        assert!(close(
            before,
            s.view(rect, 512, 512).to_canvas(rect.center())
        ));
    }

    #[test]
    fn zoom_keeps_the_pixel_under_the_pointer() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        let mut s = ViewState::default();
        let at = pos2(123.0, 77.0);
        let before = s.view(rect, 256, 256).to_canvas(at);
        s.zoom_to(3.5, Some(at), rect);
        assert!(close(before, s.view(rect, 256, 256).to_canvas(at)));
        s.zoom_to(MAX_ZOOM * 2.0, None, rect);
        assert_eq!(s.zoom, MAX_ZOOM);
    }

    #[test]
    fn tilt_and_rotation_follow_the_view() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 200.0));
        let plain = CanvasView::new(rect, 100, 100, 1.0, Vec2::ZERO, 0.0, false);
        // 画面の下へ 30° 倒すと、キャンバス（y が上向き）では −y へ
        let t = plain.tilt_to_canvas(Tilt { x: 0.0, y: 30.0 });
        assert!(
            t.x.abs() < 1e-9 && (t.y + 30f64.to_radians()).abs() < 1e-9,
            "{t:?}"
        );
        let t = plain.tilt_to_canvas(Tilt { x: 20.0, y: 0.0 });
        assert!((t.x - 20f64.to_radians()).abs() < 1e-9 && t.y.abs() < 1e-9);
        assert_eq!(plain.tilt_to_canvas(Tilt::default()), DVec2::ZERO);
        // 画面を時計回りに 90° 回している: キャンバスの上が画面の右になるので、画面の右へ倒すとキャンバスでは上（+y）へ
        let rotated = CanvasView::new(rect, 100, 100, 1.0, Vec2::ZERO, 90.0, false);
        let t = rotated.tilt_to_canvas(Tilt { x: 25.0, y: 0.0 });
        assert!(
            t.x.abs() < 1e-9 && (t.y - 25f64.to_radians()).abs() < 1e-9,
            "{t:?}"
        );
        // 左右反転では x の向きが替わる
        let flipped = CanvasView::new(rect, 100, 100, 1.0, Vec2::ZERO, 0.0, true);
        let t = flipped.tilt_to_canvas(Tilt { x: 25.0, y: 0.0 });
        assert!((t.x + 25f64.to_radians()).abs() < 1e-9);
        // 回転: 画面で時計回りの 90° は、キャンバスで反時計回りの −90°
        assert_eq!(plain.rotation_to_canvas(0.0), 0.0);
        assert!((plain.rotation_to_canvas(90.0) + std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        // 表示を時計回りに 90° 回していると、同じ向きの軸はキャンバスの上では 90° 戻った向きになる
        let r = rotated.rotation_to_canvas(90.0);
        assert!(r.abs() < 1e-9, "{r}");
    }

    #[test]
    fn the_screen_affine_is_the_same_map_as_to_screen() {
        for angle in [0.0, 15.0, 90.0, -90.0, 180.0, 33.3] {
            for flip in [false, true] {
                let v = CanvasView::new(
                    Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 200.0)),
                    256,
                    128,
                    1.7,
                    vec2(12.0, -5.0),
                    angle,
                    flip,
                );
                let [a, b, c, d, e, g] = v.screen_affine();
                for (x, y) in [(0.0, 0.0), (37.25, 99.5), (256.0, 128.0), (-10.0, 300.0)] {
                    let s = v.to_screen(x, y);
                    let (sx, sy) = (a * x + b * y + c, d * x + e * y + g);
                    assert!(
                        (sx - s.x as f64).abs() < 1e-3 && (sy - s.y as f64).abs() < 1e-3,
                        "angle {angle} flip {flip}: ({sx}, {sy}) {s:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn normalize_and_label() {
        assert_eq!(normalize_angle(-180.0), 180.0);
        assert_eq!(normalize_angle(195.0), -165.0);
        assert_eq!(normalize_angle(-0.0), 0.0);
        assert_eq!(angle_label(15.0), "15°");
        assert_eq!(angle_label(-12.34), "-12.3°");
    }
}
