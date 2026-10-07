//! 値のカーブの編集の部品（`yolu_core::curve::Curve`。グラデーションのランプの値のカーブ・トーンカーブ・筆圧のカーブが共通で使う）。
//!
//! 点の無い所を押すと点を足してそのまま動かせ、点をドラッグで動かし、右クリックか枠の外へ離すと消す（両端は横に動かず、消せない）。
//! ドラッグは下書きに溜め、離したところで 1 回の変更として返す（Esc は元のまま。文書も履歴も触らない）。
//! 編集の計算（足す・消す・動かす・プリセット）は `ops` にまとめた純粋な関数で、試験が直に確かめる。

use egui::{pos2, Color32, Sense, Ui};
use yolu_core::curve::{Curve, CurvePoint};

use super::settle;
use super::theme as t;
use super::widgets::{outline, rounded};

/// 部品の高さ。
pub const HEIGHT: f32 = 116.0;
/// 点の数の上限と、点の最小の横の間隔（core の `Curve` の検査と同じ。間隔は丸めの余裕を含めて 0.02）。
pub const MAX_POINTS: usize = Curve::MAX_POINTS;
pub const MIN_GAP: f64 = 0.02;
const GRAB: f32 = 7.0;
const REMOVE_MARGIN: f32 = 16.0;

/// 編集の計算（足す・消す・動かす・置き換える）。失敗（範囲外・数が足りない）は `None` で、元は変えない。
pub mod ops {
    use super::*;

    /// 点の並びからカーブを作る（検査に落ちたら `None`）。
    pub fn with_points(points: Vec<CurvePoint>) -> Option<Curve> {
        Curve::new(points).ok()
    }

    /// 点を足す（横の間隔を残せる所だけ）。足した番号も返す。
    pub fn add_point(curve: &Curve, x: f64, y: f64) -> Option<(Curve, usize)> {
        let points = curve.points();
        let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
        if points.len() >= MAX_POINTS || points.iter().any(|p| (p.x - x).abs() < MIN_GAP - 1e-6) {
            return None;
        }
        let mut list = points.to_vec();
        list.push(CurvePoint { x, y });
        list.sort_by(|a, b| a.x.total_cmp(&b.x));
        let index = list.iter().position(|p| p.x == x)?;
        Some((with_points(list)?, index))
    }

    /// 点を動かす（両端は縦だけ。ほかは隣との間隔を残す）。
    pub fn move_point(curve: &Curve, k: usize, x: f64, y: f64) -> Option<Curve> {
        let mut list = curve.points().to_vec();
        let last = list.len().checked_sub(1)?;
        if k > last {
            return None;
        }
        let y = y.clamp(0.0, 1.0);
        let x = if k == 0 {
            0.0
        } else if k == last {
            1.0
        } else {
            let min = list[k - 1].x + MIN_GAP;
            let max = list[k + 1].x - MIN_GAP;
            x.clamp(min, max.max(min))
        };
        list[k] = CurvePoint { x, y };
        with_points(list)
    }

    /// 点を消す（両端・2 点は消さない）。
    pub fn remove_point(curve: &Curve, k: usize) -> Option<Curve> {
        let mut list = curve.points().to_vec();
        if list.len() <= 2 || k == 0 || k + 1 >= list.len() {
            return None;
        }
        list.remove(k);
        with_points(list)
    }

    /// よく使う形（線形・やわらかい・かたい・S 字。Unity 版の筆圧のカーブと同じ点）。
    pub const PRESETS: [&[(f64, f64)]; 4] = [
        &[(0.0, 0.0), (1.0, 1.0)],
        &[(0.0, 0.0), (0.3, 0.55), (1.0, 1.0)],
        &[(0.0, 0.0), (0.55, 0.3), (1.0, 1.0)],
        &[(0.0, 0.0), (0.3, 0.15), (0.7, 0.85), (1.0, 1.0)],
    ];

    pub fn preset(index: usize) -> Option<Curve> {
        let points = PRESETS.get(index)?;
        with_points(points.iter().map(|&(x, y)| CurvePoint { x, y }).collect())
    }
}

/// ドラッグの途中（下書き）。
#[derive(Clone)]
struct Drag {
    index: usize,
    original: Curve,
    draft: Curve,
}

/// 曲線の見た目の指定（操作は変わらない）。
#[derive(Clone, Copy, Default)]
pub struct CurveStyle<'a> {
    /// 曲線の色（無ければ既定）。
    pub line: Option<Color32>,
    /// 後ろに薄く敷く分布（左から右へ並べた 0〜1 の高さ。数は出さない）。
    pub backdrop: Option<&'a [f32]>,
    /// 入力をそのまま返す斜めの線を薄く引くか。
    pub diagonal: bool,
}

/// 値のカーブの編集（横が入力、縦が出力）。変更が決まったとき（足した・消した・ドラッグを離した）だけ新しいカーブを返す。
pub fn curve_editor(
    ui: &mut Ui,
    r: egui::Rect,
    id_salt: impl egui::AsIdSalt,
    curve: &Curve,
    tooltip: &str,
    enabled: bool,
) -> Option<Curve> {
    curve_editor_with(
        ui,
        r,
        id_salt,
        curve,
        &CurveStyle::default(),
        tooltip,
        enabled,
    )
}

/// `curve_editor` の、見た目（曲線の色・後ろの分布・斜めの線）を指定する形。操作と返す値は同じ。
pub fn curve_editor_with(
    ui: &mut Ui,
    r: egui::Rect,
    id_salt: impl egui::AsIdSalt,
    curve: &Curve,
    style: &CurveStyle<'_>,
    tooltip: &str,
    enabled: bool,
) -> Option<Curve> {
    let id = ui.make_persistent_id(id_salt);
    let enabled = enabled && ui.is_enabled();
    let response = ui.interact(
        r,
        id,
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let drag_id = id.with("drag");
    let mut drag: Option<Drag> = ui.data(|d| d.get_temp(drag_id));
    // 別のカーブのドラッグ（レイヤーを替えたなど）の下書きは、今のカーブへ当てずに捨てる
    if drag.as_ref().is_some_and(|d| d.original != *curve) {
        ui.data_mut(|data| data.remove::<Drag>(drag_id));
        drag = None;
    }
    let g = r.shrink(6.0);
    let to_xy = |m: egui::Pos2| {
        (
            f64::from((m.x - g.left()) / g.width().max(1.0)).clamp(0.0, 1.0),
            f64::from(1.0 - (m.y - g.top()) / g.height().max(1.0)).clamp(0.0, 1.0),
        )
    };
    let to_screen = |x: f64, y: f64| {
        pos2(
            g.left() + x as f32 * g.width(),
            g.bottom() - y as f32 * g.height(),
        )
    };
    let pointer = ui.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()));
    let (pressed, secondary, down) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.secondary_pressed(),
            i.pointer.primary_down(),
        )
    });
    let mut result: Option<Curve> = None;
    if enabled && drag.is_none() && (pressed || secondary) && response.hovered() {
        if let Some(m) = pointer {
            let nearest = curve
                .points()
                .iter()
                .enumerate()
                .map(|(k, p)| (k, to_screen(p.x, p.y).distance(m)))
                .filter(|(_, d)| *d <= GRAB)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(k, _)| k);
            if secondary {
                if let Some(k) = nearest {
                    result = ops::remove_point(curve, k);
                }
            } else {
                let (x, y) = to_xy(m);
                let started = match nearest {
                    Some(k) => Some((k, curve.clone())),
                    None => ops::add_point(curve, x, y).map(|(next, k)| (k, next)),
                };
                if let Some((k, working)) = started {
                    drag = Some(Drag {
                        index: k,
                        original: curve.clone(),
                        draft: working,
                    });
                }
            }
        }
    }
    if let Some(mut d) = drag.take() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            // 元のまま（文書も履歴も触らない）
            ui.data_mut(|data| data.remove::<Drag>(drag_id));
        } else if down {
            if let Some(m) = pointer {
                let (x, y) = to_xy(m);
                if let Some(next) = ops::move_point(&d.draft, d.index, x, y) {
                    d.draft = next;
                }
            }
            ui.data_mut(|data| data.insert_temp(drag_id, d));
        } else {
            let outside = pointer.is_some_and(|m| !r.expand(REMOVE_MARGIN).contains(m));
            let mut done = d.draft.clone();
            if outside {
                if let Some(next) = ops::remove_point(&done, d.index) {
                    done = next;
                }
            }
            if done != d.original {
                result = Some(done);
            }
            ui.data_mut(|data| data.remove::<Drag>(drag_id));
        }
    }
    let live: Option<Curve> = ui
        .data(|data| data.get_temp::<Drag>(drag_id))
        .map(|d| d.draft);
    // 離したフレームも、文書が追いつくまで離した値を見せる（元の位置へ一瞬戻らない）
    if let Some(done) = &result {
        settle::start(ui, id, curve, done);
    }
    let settled = result.clone().or_else(|| settle::shown(ui, id, curve));
    let shown = live.as_ref().or(settled.as_ref()).unwrap_or(curve);
    ui.data_mut(|d| d.insert_temp(id.with("painted"), shown.clone()));
    if ui.is_rect_visible(r) {
        let p = ui.painter();
        rounded(p, r, t::CONTROL_BG, 3.0);
        outline(p, r, t::BORDER, 1.0, 3.0);
        if let Some(bins) = style.backdrop.filter(|b| b.len() >= 2) {
            let step = g.width() / bins.len() as f32;
            let faint = Color32::from_rgba_unmultiplied(160, 170, 190, 54);
            for (i, v) in bins.iter().enumerate() {
                let h = g.height() * v.clamp(0.0, 1.0);
                if h >= 0.5 {
                    p.rect_filled(
                        egui::Rect::from_min_max(
                            pos2(g.left() + step * i as f32, g.bottom() - h),
                            pos2(g.left() + step * (i + 1) as f32 + 0.5, g.bottom()),
                        ),
                        0.0,
                        faint,
                    );
                }
            }
        }
        for q in 1..4 {
            let f = q as f32 / 4.0;
            p.line_segment(
                [
                    pos2(g.left() + g.width() * f, g.top()),
                    pos2(g.left() + g.width() * f, g.bottom()),
                ],
                egui::Stroke::new(1.0, t::SEPARATOR),
            );
            p.line_segment(
                [
                    pos2(g.left(), g.top() + g.height() * f),
                    pos2(g.right(), g.top() + g.height() * f),
                ],
                egui::Stroke::new(1.0, t::SEPARATOR),
            );
        }
        if style.diagonal {
            p.line_segment(
                [to_screen(0.0, 0.0), to_screen(1.0, 1.0)],
                egui::Stroke::new(1.0, t::SEPARATOR),
            );
        }
        let line: Vec<egui::Pos2> = (0..=64)
            .map(|k| {
                let x = k as f64 / 64.0;
                to_screen(x, shown.value(x).unwrap_or(x))
            })
            .collect();
        p.add(egui::Shape::line(
            line,
            egui::Stroke::new(1.5, style.line.unwrap_or(t::ACCENT)),
        ));
        let active = ui
            .data(|data| data.get_temp::<Drag>(drag_id))
            .map(|d| d.index);
        for (k, point) in shown.points().iter().enumerate() {
            let c = to_screen(point.x, point.y);
            p.circle_filled(
                c,
                4.0,
                if active == Some(k) {
                    Color32::YELLOW
                } else {
                    Color32::WHITE
                },
            );
            p.circle_stroke(c, 4.0, egui::Stroke::new(1.0, Color32::BLACK));
        }
    }
    let _ = response.on_hover_text(tooltip);
    result
}

/// この部品が直前に描いたカーブ（ドラッグの下書き・離した直後の値を含む）。部品の外から「何が見えているか」を確かめる試験の口。
pub fn last_painted(ui: &Ui, id_salt: impl egui::AsIdSalt) -> Option<Curve> {
    let id = ui.make_persistent_id(id_salt);
    ui.data(|d| d.get_temp(id.with("painted")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_add_move_and_remove_with_fixed_ends() {
        let c = Curve::identity();
        let (with, k) = ops::add_point(&c, 0.5, 0.2).unwrap();
        assert_eq!((with.points().len(), k), (3, 1));
        assert!((with.value(0.5).unwrap() - 0.2).abs() < 0.02);
        // 端は横に動かない
        let end = ops::move_point(&with, 0, 0.4, 0.3).unwrap();
        assert_eq!((end.points()[0].x, end.points()[0].y), (0.0, 0.3));
        let end = ops::move_point(&with, 2, 0.4, 0.9).unwrap();
        assert_eq!(end.points()[2].x, 1.0);
        // 隣との間隔を残す
        let squeezed = ops::move_point(&with, 1, 5.0, 0.5).unwrap();
        assert!(squeezed.points()[1].x <= 1.0 - MIN_GAP + 1e-9);
        // 範囲外の番号
        assert!(ops::move_point(&with, 3, 0.5, 0.5).is_none());
        // 消せるのは途中の点だけ
        assert!(ops::remove_point(&with, 0).is_none());
        assert!(ops::remove_point(&with, 2).is_none());
        assert_eq!(ops::remove_point(&with, 1).unwrap().points().len(), 2);
        assert!(ops::remove_point(&c, 1).is_none(), "2 点は消さない");
        assert!(
            ops::add_point(&with, 0.505, 0.2).is_none(),
            "間隔が足りない"
        );
        // 点の数の上限
        let mut many = c.clone();
        for i in 1..15 {
            many = ops::add_point(&many, i as f64 / 16.0, 0.5).unwrap().0;
        }
        assert_eq!(many.points().len(), MAX_POINTS);
        assert!(ops::add_point(&many, 0.03, 0.5).is_none());
    }

    #[test]
    fn presets_are_valid_and_the_first_is_linear() {
        for i in 0..ops::PRESETS.len() {
            assert!(ops::preset(i).is_some(), "{i}");
        }
        let linear = ops::preset(0).unwrap();
        assert!(linear.is_identity());
        assert!((linear.value(0.3).unwrap() - 0.3).abs() < 1e-9);
        assert!(ops::preset(1).unwrap().value(0.3).unwrap() > 0.5);
        assert!(ops::preset(9).is_none());
    }
}
