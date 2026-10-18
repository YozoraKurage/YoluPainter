//! 3D ビュー: Face のセットを描いている形。Face と Ear（このセットのマテリアル）の箱は描ける明るさ、ほかの箱は今と同じく暗い。
//! モデルは灰色の箱と輪郭の線で置き換えたダミー（マテリアルごとに 1 つの箱。形に意味は無い）。

use egui::{epaint::PathStroke, pos2, Color32, Painter, Pos2, Shape, Ui};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets as w;

use super::multimat_common::SETS;
use crate::page::SketchCtx;

/// 奥行きが画面でずれる量（奥行き 1 あたり、右へ・上へ）。
const DEPTH_X: f32 = 0.5;
const DEPTH_Y: f32 = 0.38;

/// 箱 1 つ（位置と大きさはモデルの座標。`z` は手前へ出す量）。
struct Part {
    material: usize,
    x: f32,
    base: f32,
    w: f32,
    h: f32,
    d: f32,
    z: f32,
}

const fn part(material: usize, x: f32, base: f32, w: f32, h: f32, d: f32, z: f32) -> Part {
    Part {
        material,
        x,
        base,
        w,
        h,
        d,
        z,
    }
}

/// 描く順（奥の段、手前の段の順）。添え字は MATERIALS: 0 Face・1 Ear・2 Eye・3 Tongue・4 Body・5 Cloth・6 Hair。
const PARTS: [Part; 7] = [
    part(4, -120.0, 190.0, 120.0, 160.0, 90.0, 0.0),
    part(5, 60.0, 190.0, 140.0, 40.0, 120.0, 0.0),
    part(6, 230.0, 190.0, 50.0, 150.0, 50.0, 0.0),
    part(0, -170.0, 0.0, 120.0, 120.0, 100.0, 0.0),
    part(1, -30.0, 0.0, 44.0, 70.0, 40.0, 0.0),
    part(2, 50.0, 0.0, 40.0, 40.0, 40.0, 0.0),
    part(3, 130.0, 0.0, 26.0, 60.0, 22.0, 0.0),
];

fn shade(c: u8, k: f32) -> Color32 {
    Color32::from_gray((c as f32 * k).clamp(0.0, 255.0) as u8)
}

fn polygon(p: &Painter, points: Vec<Pos2>, fill: Color32, outline: Color32) {
    p.add(Shape::convex_polygon(
        points,
        fill,
        PathStroke::new(1.2, outline),
    ));
}

fn draw_part(p: &Painter, origin: Pos2, s: f32, part: &Part, lit: bool) {
    let (base, line) = if lit { (178, 232) } else { (56, 96) };
    let line = Color32::from_gray(line);
    // 前の面の左下（画面。y は下向き）。手前へ出すと、奥行きとは逆向きにずれる
    let fx = origin.x + (part.x - part.w / 2.0 - DEPTH_X * part.z) * s;
    let fy = origin.y - (part.base - DEPTH_Y * part.z) * s;
    let (w, h) = (part.w * s, part.h * s);
    let (dx, dy) = (DEPTH_X * part.d * s, -DEPTH_Y * part.d * s);
    let bl = pos2(fx, fy);
    let br = pos2(fx + w, fy);
    let tl = pos2(fx, fy - h);
    let tr = pos2(fx + w, fy - h);
    let off = |q: Pos2| pos2(q.x + dx, q.y + dy);
    polygon(p, vec![br, off(br), off(tr), tr], shade(base, 0.7), line);
    polygon(p, vec![tl, tr, off(tr), off(tl)], shade(base, 1.12), line);
    polygon(p, vec![bl, br, tr, tl], shade(base, 1.0), line);
}

pub fn draw(ui: &mut Ui, _ctx: &mut SketchCtx) {
    let view = ui.max_rect();
    ui.advance_cursor_after_rect(view);
    let p = ui.painter_at(view);
    w::fill(&p, view, t::CANVAS_BG);
    let s = (view.height() / 440.0).min(view.width() / 700.0);
    let origin = pos2(view.center().x, view.center().y + 190.0 * s);
    // 描いているセットは先頭の Face（Face と Ear のマテリアルが結び付いている）
    let current = SETS[0].materials;
    for part in &PARTS {
        draw_part(&p, origin, s, part, current.contains(&part.material));
    }
}
