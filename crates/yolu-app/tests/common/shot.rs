//! 画面の絵のテストの撮る口。ウィンドウ全体の絵は `window`（並びの見本だけに使う）、ほかは `region` と `node` で、部品・パネルの
//! 切り抜きを撮る。切り抜きにすると、ある部品の見た目を変えたときに変わる絵が、その部品の絵だけになる。
#![allow(dead_code)]

use egui::Rect;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

/// ウィンドウ全体を撮る。画面全体の並びを見る見本の絵だけに使う。
pub fn window<State>(harness: &mut Harness<'_, State>, name: &str) {
    harness.snapshot(name);
}

/// 画面の範囲 `rect`（論理の点）を切り抜いて撮る。範囲は画面の中に丸める（外にはみ出した分は撮らない）。
pub fn region<State>(harness: &mut Harness<'_, State>, name: &str, rect: Rect) {
    let image = harness.render().expect("画面を描く");
    let cropped = crop(&image, rect, harness.ctx.pixels_per_point());
    egui_kittest::image_snapshot(&cropped, name);
}

/// ラベル `label` の部品（AccessKit の節）の範囲に、周りの余白 `margin`（論理の点）を足して撮る。
pub fn node<State>(harness: &mut Harness<'_, State>, name: &str, label: &str, margin: f32) {
    let rect = harness.get_by_label(label).rect().expand(margin);
    region(harness, name, rect);
}

/// 描いた絵 `image` から、論理の点の範囲 `rect` を切り抜く（拡大率 `pixels_per_point`）。範囲が空なら 1×1 にする。
pub fn crop(image: &image::RgbaImage, rect: Rect, pixels_per_point: f32) -> image::RgbaImage {
    let (w, h) = (image.width(), image.height());
    let px = |v: f32, max: u32| ((v * pixels_per_point).round().max(0.0) as u32).min(max);
    let (x0, y0) = (px(rect.min.x, w), px(rect.min.y, h));
    let (x1, y1) = (
        px(rect.max.x, w).max(x0 + 1).min(w),
        px(rect.max.y, h).max(y0 + 1).min(h),
    );
    image::imageops::crop_imm(
        image,
        x0.min(w - 1),
        y0.min(h - 1),
        x1 - x0.min(w - 1),
        y1 - y0.min(h - 1),
    )
    .to_image()
}
