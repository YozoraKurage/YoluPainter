//! 面を隠す下書き（A・B）が共通で使う、灰色のモデルの絵と当たりの判定。モデルは大きさの違う箱を並べた形で、箱の面が UV アイランド
//! （3D のモデルの素材が無いので、本物のモデルは使わない）。アイランドの名前は画面に出さない。

use egui::{pos2, Color32, Painter, Pos2, Rect, Shape, Stroke};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets as w;

/// アイランドの番号（描く順。後ろのほうが手前）。箱ごとに、正面・上・右の面が 1 つずつのアイランド（大きい箱の正面は 2 つに割れている）。
pub const WIDE_FRONT_L: usize = 0;
pub const WIDE_FRONT_R: usize = 1;
pub const WIDE_TOP: usize = 2;
pub const WIDE_SIDE: usize = 3;
pub const SMALL_FRONT: usize = 4;
pub const SMALL_TOP: usize = 5;
pub const SMALL_SIDE: usize = 6;
pub const TALL_FRONT: usize = 7;
pub const TALL_TOP: usize = 8;
pub const TALL_SIDE: usize = 9;
pub const CUBE_FRONT: usize = 10;
pub const CUBE_TOP: usize = 11;
pub const CUBE_SIDE: usize = 12;
pub const COUNT: usize = 13;

/// 見え方。
#[derive(Clone, Copy, PartialEq)]
pub enum Look {
    Normal,
    /// 選んでいる（青く塗って強調）。
    Hover,
    /// ポインターが乗っている（選ぶ前の候補。薄く塗って青い輪郭）。
    Pointed,
    /// 隠しているアイランドの、隠すモードの間だけ出す点線の輪郭。
    Ghost,
}

/// 箱の 3 つの面（正面・上・右）。`(x, y)` は正面の左下、`w`・`h` は正面の幅と高さ、奥行きは右上への斜めのずれ。
struct Boxy {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    d: f32,
}

impl Boxy {
    fn dx(&self) -> f32 {
        self.d * 0.55
    }
    fn dy(&self) -> f32 {
        -self.d * 0.38
    }
    fn front(&self, from: f32, to: f32) -> Vec<Pos2> {
        let (l, r) = (self.x + self.w * from, self.x + self.w * to);
        vec![
            pos2(l, self.y - self.h),
            pos2(r, self.y - self.h),
            pos2(r, self.y),
            pos2(l, self.y),
        ]
    }
    fn top(&self) -> Vec<Pos2> {
        let (dx, dy) = (self.dx(), self.dy());
        vec![
            pos2(self.x, self.y - self.h),
            pos2(self.x + dx, self.y - self.h + dy),
            pos2(self.x + self.w + dx, self.y - self.h + dy),
            pos2(self.x + self.w, self.y - self.h),
        ]
    }
    fn side(&self) -> Vec<Pos2> {
        let (dx, dy) = (self.dx(), self.dy());
        let r = self.x + self.w;
        vec![
            pos2(r, self.y - self.h),
            pos2(r + dx, self.y - self.h + dy),
            pos2(r + dx, self.y + dy),
            pos2(r, self.y),
        ]
    }
}

const WIDE: Boxy = Boxy {
    x: -0.5,
    y: 0.5,
    w: 0.44,
    h: 0.22,
    d: 0.28,
};
const SMALL: Boxy = Boxy {
    x: -0.43,
    y: 0.255,
    w: 0.14,
    h: 0.09,
    d: 0.12,
};
const TALL: Boxy = Boxy {
    x: 0.02,
    y: 0.5,
    w: 0.2,
    h: 0.42,
    d: 0.24,
};
const CUBE: Boxy = Boxy {
    x: 0.33,
    y: 0.5,
    w: 0.15,
    h: 0.15,
    d: 0.16,
};

/// アイランドの輪郭（モデルの座標。x は中心が 0 で −0.5〜0.5、y は上が 0 で 0.5 が床）。
fn outline(island: usize) -> Vec<Pos2> {
    match island {
        WIDE_FRONT_L => WIDE.front(0.0, 0.58),
        WIDE_FRONT_R => WIDE.front(0.58, 1.0),
        WIDE_TOP => WIDE.top(),
        WIDE_SIDE => WIDE.side(),
        SMALL_FRONT => SMALL.front(0.0, 1.0),
        SMALL_TOP => SMALL.top(),
        SMALL_SIDE => SMALL.side(),
        TALL_FRONT => TALL.front(0.0, 1.0),
        TALL_TOP => TALL.top(),
        TALL_SIDE => TALL.side(),
        CUBE_FRONT => CUBE.front(0.0, 1.0),
        CUBE_TOP => CUBE.top(),
        _ => CUBE.side(),
    }
}

/// モデルを置く矩形（高さ `height` の中で、真ん中に寄せる）。
#[derive(Clone, Copy)]
pub struct Figure {
    /// 箱の並びの中心の x と、上の端の y（画面の点）。
    top: Pos2,
    scale: f32,
}

impl Figure {
    /// `view` の中に収める（箱の並びを真ん中に寄せる。上は隅のアイコンの列を避けて少し空ける）。
    pub fn fit_zoom(view: Rect, zoom: f32) -> Figure {
        let scale = (view.width() * 0.8)
            .min((view.height() - 90.0) / 0.64)
            .max(120.0)
            * zoom;
        Figure {
            top: pos2(view.center().x, view.center().y - 0.26 * scale + 18.0),
            scale,
        }
    }

    /// 箱の並びの高さ 1 に当たる画面の長さ（点）。
    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn screen_point(&self, p: Pos2) -> Pos2 {
        pos2(self.top.x + p.x * self.scale, self.top.y + p.y * self.scale)
    }

    pub fn island(&self, island: usize) -> Vec<Pos2> {
        outline(island)
            .into_iter()
            .map(|p| self.screen_point(p))
            .collect()
    }

    /// アイランドの真ん中あたり（画面の点）。
    pub fn center_of(&self, island: usize) -> Pos2 {
        let points = self.island(island);
        let n = points.len() as f32;
        let sum = points.iter().fold(egui::Vec2::ZERO, |a, p| a + p.to_vec2());
        (sum / n).to_pos2()
    }
}

fn gray_of(island: usize) -> Color32 {
    // 上の面は明るく、正面は中間、右の面は暗く
    let base = match island {
        WIDE_TOP | SMALL_TOP | TALL_TOP | CUBE_TOP => 0x8A,
        WIDE_SIDE | SMALL_SIDE | TALL_SIDE | CUBE_SIDE => 0x5A,
        WIDE_FRONT_R => 0x72,
        _ => 0x6C,
    };
    Color32::from_gray(base)
}

/// アイランド 1 つを描く（灰色の面と、面の割れ目の細い線と、輪郭）。
pub fn draw_island(p: &Painter, figure: &Figure, island: usize, look: Look) {
    let points = figure.island(island);
    match look {
        Look::Ghost => {
            let mut path = points.clone();
            path.push(points[0]);
            p.extend(Shape::dashed_line(
                &path,
                Stroke::new(1.0, t::TEXT_DISABLED),
                4.0,
                3.0,
            ));
        }
        Look::Normal | Look::Hover | Look::Pointed => {
            p.add(Shape::convex_polygon(
                points.clone(),
                gray_of(island),
                Stroke::new(1.0, Color32::from_gray(0x2C)),
            ));
            let centre = figure.center_of(island);
            let seam = Stroke::new(1.0, Color32::from_black_alpha(46));
            for q in &points {
                p.line_segment([centre, *q], seam);
            }
            let inner: Vec<Pos2> = points
                .iter()
                .map(|q| centre + (*q - centre) * 0.55)
                .collect();
            p.add(Shape::closed_line(inner, seam));
            match look {
                Look::Hover => {
                    p.add(Shape::convex_polygon(
                        points,
                        t::ACCENT.gamma_multiply(0.45),
                        Stroke::new(2.0, t::ACCENT),
                    ));
                }
                Look::Pointed => {
                    p.add(Shape::convex_polygon(
                        points,
                        t::ACCENT.gamma_multiply(0.16),
                        Stroke::new(2.0, t::ACCENT_DIM),
                    ));
                }
                _ => {}
            }
        }
    }
}

/// モデル全体（隠しているアイランドは描かない。`ghosts` なら点線の輪郭で残す）。`look` は、隠していないアイランドの見え方。
pub fn draw_figure(
    p: &Painter,
    figure: &Figure,
    hidden: &dyn Fn(usize) -> bool,
    look: &dyn Fn(usize) -> Look,
    ghosts: bool,
) {
    for island in 0..COUNT {
        if hidden(island) {
            if ghosts {
                draw_island(p, figure, island, Look::Ghost);
            }
        } else {
            draw_island(p, figure, island, look(island));
        }
    }
}

/// マウスのポインターの絵（下書きの中の目印。アイコンではない）。
pub fn draw_pointer(p: &Painter, at: Pos2) {
    let shape = [
        at,
        at + egui::vec2(0.0, 17.0),
        at + egui::vec2(4.5, 13.0),
        at + egui::vec2(8.0, 20.0),
        at + egui::vec2(10.5, 19.0),
        at + egui::vec2(7.0, 12.0),
        at + egui::vec2(12.5, 12.0),
    ];
    p.add(Shape::convex_polygon(
        shape.to_vec(),
        Color32::WHITE,
        Stroke::new(1.0, Color32::BLACK),
    ));
}

/// 隠すモードの間、ビューの縁に引く細い枠。
pub fn draw_mode_frame(p: &Painter, view: Rect) {
    w::outline(p, view.shrink(1.0), t::ACCENT, 2.0, 0.0);
}
