//! 移動・変形のツール（V、Unity 版の Move / Transform）: 選んでいるレイヤー（グループなら中身のラスターレイヤーごと、複数選んでいればその全部）を、
//! 範囲の外枠のハンドルで移動・拡大縮小・回転する。選択範囲があればその中の画素と選択範囲を動かす（core の `transform_layers`）。
//!
//! 角のハンドルは拡大縮小（Shift で縦横比を保つ）、辺の中点は片方向、角の外側は回転（Shift で 15° 刻み）、それ以外は移動（整数画素）。
//! ドラッグの間は変形後の外枠だけを見せ、離したところで 1 回の Undo にする（途中で文書を変えないので、Esc・フォーカスを失う・
//! ツールの切り替えは何も変えずにやめるだけ。取り残さない）。Enter はドラッグの途中で押すとその位置で確定する。矢印キーは 1 画素
//! （Shift で 10）の移動で、表示を回していても画面の向きに合わせる。数値の変形・90° 回転・反転・補間はツールの設定の欄とメニューから。
//!
//! 形はキャンバスの座標（左下が原点）で決まり、表示を回している・反転しているときは回って見える。

pub mod advanced;
pub mod canvas;
pub mod props;

use egui::Pos2;
use yolu_core::{Affine2D, Resampling};

use crate::canvas::view::CanvasView;
use crate::engine::LayerId;
use crate::state::StrokeSource;

/// 動かすものの範囲（キャンバスの座標。右・上は含まない）。
pub type Bounds = (i64, i64, i64, i64);

/// ハンドルを掴める距離（画面の点）。
pub const HANDLE_HIT: f32 = 6.0;
/// 角の外側でこの距離（画面の点）までは回転。
pub const ROTATE_REACH: f32 = 26.0;
/// これより小さい範囲では、ハンドルを出さずどこを掴んでも移動にする（画面の上の辺の長さ。回転では変わらない）。
pub const MIN_HANDLE_BOX: f32 = 36.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Move,
    /// `anchor` は動かさない反対側の点、`handle` は掴んだ点、`axes` はビット 0 が横・ビット 1 が縦に拡大縮小。
    Scale {
        anchor: (f64, f64),
        handle: (f64, f64),
        axes: u8,
    },
    Rotate,
}

/// ドラッグの途中（押した所と今の所はキャンバスの座標）。
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    pub mode: Mode,
    pub source: StrokeSource,
    pub bounds: Bounds,
    pub start: (f64, f64),
    pub current: (f64, f64),
    pub shift: bool,
}

/// 数値の変形の欄（動かすものの中心を軸に。拡大縮小は %、角度は反時計回り）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Numeric {
    pub dx: f64,
    pub dy: f64,
    pub degrees: f64,
    pub scale_x: f64,
    pub scale_y: f64,
}

impl Default for Numeric {
    fn default() -> Self {
        Numeric {
            dx: 0.0,
            dy: 0.0,
            degrees: 0.0,
            scale_x: 100.0,
            scale_y: 100.0,
        }
    }
}

/// ツールの状態。
#[derive(Debug)]
pub struct TransformState {
    pub advanced: advanced::State,
    pub drag: Option<Drag>,
    /// ペンで押している間のペンの番号。
    pub pen_down: Option<u32>,
    pub resampling: Resampling,
    pub numeric: Numeric,
    /// 範囲の計算は画素を数えるので、文書の版・動かすレイヤーが変わるまで覚える。
    cache: Option<(u64, Vec<LayerId>, Option<Bounds>)>,
}

impl Default for TransformState {
    fn default() -> Self {
        TransformState {
            advanced: advanced::State::default(),
            drag: None,
            pen_down: None,
            resampling: Resampling::Bilinear,
            numeric: Numeric::default(),
            cache: None,
        }
    }
}

impl TransformState {
    /// 覚えている範囲を捨てる。
    pub fn forget_bounds(&mut self) {
        self.cache = None;
    }
}

pub fn resampling_name(lang: crate::lang::Lang, mode: Resampling) -> &'static str {
    match mode {
        Resampling::Bilinear => lang.pick("バイリニア", "Bilinear"),
        Resampling::Nearest => lang.pick("ニアレストネイバー", "Nearest"),
    }
}

/// 8 つのハンドルの位置: 4 つの角（左下・右下・右上・左上）、辺の中点（下・右・上・左）。
pub fn handle_points(b: Bounds) -> [(f64, f64); 8] {
    let (x0, y0, x1, y1) = (b.0 as f64, b.1 as f64, b.2 as f64, b.3 as f64);
    let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    [
        (x0, y0),
        (x1, y0),
        (x1, y1),
        (x0, y1),
        (cx, y0),
        (x1, cy),
        (cx, y1),
        (x0, cy),
    ]
}

/// ハンドルを出せる大きさか。
pub fn handles_usable(view: &CanvasView, b: Bounds) -> bool {
    if view.axis_aligned() {
        let a = view.to_screen(b.0 as f64, b.1 as f64);
        let c = view.to_screen(b.2 as f64, b.3 as f64);
        (c.x - a.x).abs() >= MIN_HANDLE_BOX && (c.y - a.y).abs() >= MIN_HANDLE_BOX
    } else {
        (b.2 - b.0) as f32 * view.pixel_size() >= MIN_HANDLE_BOX
            && (b.3 - b.1) as f32 * view.pixel_size() >= MIN_HANDLE_BOX
    }
}

/// 範囲の内側か（表示が回っていればキャンバスの座標で見る）。
pub fn inside_box(view: &CanvasView, b: Bounds, pointer: Pos2) -> bool {
    let (x, y) = view.to_canvas(pointer);
    x >= b.0 as f64 && x < b.2 as f64 && y >= b.1 as f64 && y < b.3 as f64
}

/// 押した所で何をするか。
pub fn hit(view: &CanvasView, b: Bounds, pointer: Pos2) -> Mode {
    if !handles_usable(view, b) {
        return Mode::Move;
    }
    let points = handle_points(b);
    for (i, p) in points.iter().enumerate() {
        if view.to_screen(p.0, p.1).distance(pointer) > HANDLE_HIT {
            continue;
        }
        let anchor = ((b.0 + b.2) as f64 - p.0, (b.1 + b.3) as f64 - p.1);
        let axes = if i < 4 {
            3
        } else if i == 5 || i == 7 {
            1
        } else {
            2
        };
        return Mode::Scale {
            anchor,
            handle: *p,
            axes,
        };
    }
    if inside_box(view, b, pointer) {
        return Mode::Move;
    }
    if points[..4]
        .iter()
        .any(|p| view.to_screen(p.0, p.1).distance(pointer) <= ROTATE_REACH)
    {
        return Mode::Rotate;
    }
    Mode::Move
}

impl Drag {
    pub fn center(&self) -> (f64, f64) {
        (
            (self.bounds.0 + self.bounds.2) as f64 / 2.0,
            (self.bounds.1 + self.bounds.3) as f64 / 2.0,
        )
    }

    /// 回転の角度（度、反時計回り、-180〜180。Shift で 15° 刻み）。
    pub fn angle(&self) -> f64 {
        let (cx, cy) = self.center();
        let a = (self.current.1 - cy).atan2(self.current.0 - cx)
            - (self.start.1 - cy).atan2(self.start.0 - cx);
        let degrees = a.to_degrees();
        let degrees = (degrees + 180.0).rem_euclid(360.0) - 180.0;
        if self.shift {
            (degrees / 15.0).round_ties_even() * 15.0
        } else {
            degrees
        }
    }

    /// 移動の量（整数画素）。
    pub fn delta(&self) -> (i32, i32) {
        (
            (self.current.0 - self.start.0).round_ties_even() as i32,
            (self.current.1 - self.start.1).round_ties_even() as i32,
        )
    }

    /// 今のドラッグが表す変形。
    pub fn transform(&self) -> Affine2D {
        let built = match self.mode {
            Mode::Move => {
                let (dx, dy) = self.delta();
                Ok(Affine2D::translation(dx as f64, dy as f64))
            }
            Mode::Scale {
                anchor,
                handle,
                axes,
            } => {
                let mut sx = if axes & 1 != 0 && handle.0 != anchor.0 {
                    (self.current.0 - anchor.0) / (handle.0 - anchor.0)
                } else {
                    1.0
                };
                let mut sy = if axes & 2 != 0 && handle.1 != anchor.1 {
                    (self.current.1 - anchor.1) / (handle.1 - anchor.1)
                } else {
                    1.0
                };
                if self.shift && axes == 3 {
                    let m = sx.abs().max(sy.abs());
                    sx = m * if sx < 0.0 { -1.0 } else { 1.0 };
                    sy = m * if sy < 0.0 { -1.0 } else { 1.0 };
                }
                Affine2D::from_parts(anchor, (0.0, 0.0), 0.0, (sx, sy))
            }
            Mode::Rotate => {
                let degrees = self.angle();
                let (mut cx, mut cy) = self.center();
                // 90° の倍数（180° を除く）の回転は、軸を画素の格子に合わせて画素をそのまま写す
                let quarter = (degrees / 90.0).rem_euclid(2.0);
                if (quarter - 1.0).abs() < 1e-9 {
                    cx = cx.round_ties_even();
                    cy = cy.round_ties_even();
                }
                Affine2D::from_parts((cx, cy), (0.0, 0.0), degrees, (1.0, 1.0))
            }
        };
        built.unwrap_or(Affine2D::IDENTITY)
    }
}

/// 矢印キーの向き（画面の右・下が正）を、キャンバスの向き（上・右が正）の 1 画素の軸に（回っているときは長い軸の向き）。
pub fn arrow_to_canvas(view: &CanvasView, screen: (f64, f64)) -> (i32, i32) {
    let (x, y) = view.direction_to_canvas(screen.0, screen.1);
    if x.abs() >= y.abs() {
        (if x > 0.0 { 1 } else { -1 }, 0)
    } else {
        (0, if y > 0.0 { 1 } else { -1 })
    }
}

/// 変形が潰れるか（行列式がほぼ 0）。
pub fn collapses(t: &Affine2D) -> bool {
    (t.a * t.d - t.b * t.c).abs() < 1e-6
}
