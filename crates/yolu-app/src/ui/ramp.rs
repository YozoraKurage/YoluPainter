//! グラデーションのランプの編集の部品（Unity 版の `PaintGui.GradientEditor` と曲線の編集）: 不透明度の分岐点（上の行）と色（値）の分岐点
//! （下の行）が独立で、分岐点の間の中点（小さなひし形）と、形の値からランプの位置への値のカーブを持つ。
//!
//! - 分岐点は、何も無い所を押すと足し（その位置のランプの色・不透明度で）、ドラッグで動かし、右クリックか、行の外へドラッグして離すと消す
//!   （2 つは残す）。ひし形は中点を動かす。ドラッグは下書きに溜め、離したところで 1 回の変更として返す（Esc は元のまま。文書も履歴も触らない）。
//! - 値のカーブの編集は `ui::curve`（トーンカーブ・筆圧のカーブと共通の部品）へ委ね、ここは `Ramp` の値のカーブへの出し入れだけ。
//! - 編集の計算（足す・消す・動かす）は `ops` にまとめた純粋な関数で、試験が直に確かめる。

use egui::{pos2, vec2, Color32, Rect, Sense, Ui};
use yolu_core::generator::{ColorStop, CurvePoint, OpacityStop, Ramp};
use yolu_core::Rgba8;

use super::curve;
use super::theme as t;
use super::widgets::{checker, fill, outline};

/// 色・不透明度の分岐点の数の上限と、カーブの点の数の上限、分岐点の最小の間隔（core の `Ramp::new` の検査と同じ）。
pub const MAX_STOPS: usize = 32;
pub const MAX_CURVE_POINTS: usize = curve::MAX_POINTS;
pub const MIN_GAP: f64 = 0.0001;
/// カーブの点の最小の横の間隔。
pub const MIN_CURVE_GAP: f64 = curve::MIN_GAP;

/// 選んでいる分岐点（行と番号）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub index: usize,
    /// 不透明度の行か（false は色・値の行）。
    pub alpha: bool,
}

/// 編集の計算（足す・消す・動かす・置き換える）。失敗（範囲外・数が足りない）は `None` で、元は変えない。
pub mod ops {
    use super::*;

    fn rebuild(
        ramp: &Ramp,
        colors: Option<Vec<ColorStop>>,
        opacities: Option<Vec<OpacityStop>>,
        curve: Option<Vec<CurvePoint>>,
    ) -> Option<Ramp> {
        Ramp::new(
            colors.unwrap_or_else(|| ramp.colors().to_vec()),
            opacities.unwrap_or_else(|| ramp.opacities().to_vec()),
            Some(curve.unwrap_or_else(|| ramp.curve().to_vec())),
        )
        .ok()
    }

    pub fn with_colors(ramp: &Ramp, colors: Vec<ColorStop>) -> Option<Ramp> {
        rebuild(ramp, Some(colors), None, None)
    }
    pub fn with_opacities(ramp: &Ramp, opacities: Vec<OpacityStop>) -> Option<Ramp> {
        rebuild(ramp, None, Some(opacities), None)
    }
    pub fn with_curve(ramp: &Ramp, curve: Vec<CurvePoint>) -> Option<Ramp> {
        rebuild(ramp, None, None, Some(curve))
    }

    /// 位置 p に、その位置のランプの色（値）の分岐点を足す。近すぎる・上限なら `None`。足した番号も返す。
    pub fn add_color(ramp: &Ramp, p: f64, scalar: bool) -> Option<(Ramp, usize)> {
        let p = p.clamp(0.0, 1.0);
        let stops = ramp.colors();
        if stops.len() >= MAX_STOPS || stops.iter().any(|s| (s.position - p).abs() < MIN_GAP) {
            return None;
        }
        let sampled = ramp.sample_stops(p, scalar).ok()?;
        let mut list = stops.to_vec();
        list.push(ColorStop {
            position: p,
            color: Rgba8::new(sampled.r, sampled.g, sampled.b, 255),
            midpoint: 0.5,
        });
        list.sort_by(|a, b| a.position.total_cmp(&b.position));
        let index = list.iter().position(|s| s.position == p)?;
        Some((with_colors(ramp, list)?, index))
    }

    /// 位置 p に、その位置のランプの不透明度の分岐点を足す。
    pub fn add_opacity(ramp: &Ramp, p: f64) -> Option<(Ramp, usize)> {
        let p = p.clamp(0.0, 1.0);
        let stops = ramp.opacities();
        if stops.len() >= MAX_STOPS || stops.iter().any(|s| (s.position - p).abs() < MIN_GAP) {
            return None;
        }
        let sampled = ramp.sample_stops(p, false).ok()?;
        let mut list = stops.to_vec();
        list.push(OpacityStop {
            position: p,
            opacity: f64::from(sampled.a) / 255.0,
            midpoint: 0.5,
        });
        list.sort_by(|a, b| a.position.total_cmp(&b.position));
        let index = list.iter().position(|s| s.position == p)?;
        Some((with_opacities(ramp, list)?, index))
    }

    /// 分岐点を消す（2 つは残す）。
    pub fn remove(ramp: &Ramp, alpha: bool, k: usize) -> Option<Ramp> {
        if alpha {
            let mut list = ramp.opacities().to_vec();
            if list.len() <= 2 || k >= list.len() {
                return None;
            }
            list.remove(k);
            with_opacities(ramp, list)
        } else {
            let mut list = ramp.colors().to_vec();
            if list.len() <= 2 || k >= list.len() {
                return None;
            }
            list.remove(k);
            with_colors(ramp, list)
        }
    }

    /// 位置 k の分岐点が動ける範囲（隣との間隔を残す）。
    pub fn position_range(ramp: &Ramp, alpha: bool, k: usize) -> (f64, f64) {
        let positions: Vec<f64> = if alpha {
            ramp.opacities().iter().map(|s| s.position).collect()
        } else {
            ramp.colors().iter().map(|s| s.position).collect()
        };
        let min = if k == 0 {
            0.0
        } else {
            positions[k - 1] + MIN_GAP * 1.001
        };
        let max = if k + 1 >= positions.len() {
            1.0
        } else {
            positions[k + 1] - MIN_GAP * 1.001
        };
        (min, max)
    }

    /// 分岐点を p へ動かす（隣を越えない）。
    pub fn move_stop(ramp: &Ramp, alpha: bool, k: usize, p: f64) -> Option<Ramp> {
        let (min, max) = position_range(ramp, alpha, k);
        let at = p.clamp(min, max.max(min));
        if alpha {
            let mut list = ramp.opacities().to_vec();
            list.get_mut(k)?.position = at;
            with_opacities(ramp, list)
        } else {
            let mut list = ramp.colors().to_vec();
            list.get_mut(k)?.position = at;
            with_colors(ramp, list)
        }
    }

    /// 分岐点 k と次の分岐点の間の中点を、位置 p（全体の位置）へ動かす（0.01〜0.99 に収める）。
    pub fn move_midpoint(ramp: &Ramp, alpha: bool, k: usize, p: f64) -> Option<Ramp> {
        let mid = |from: f64, to: f64| ((p - from) / (to - from)).clamp(0.01, 0.99);
        if alpha {
            let mut list = ramp.opacities().to_vec();
            let (a, b) = (list.get(k)?.position, list.get(k + 1)?.position);
            list[k].midpoint = mid(a, b);
            with_opacities(ramp, list)
        } else {
            let mut list = ramp.colors().to_vec();
            let (a, b) = (list.get(k)?.position, list.get(k + 1)?.position);
            list[k].midpoint = mid(a, b);
            with_colors(ramp, list)
        }
    }

    /// カーブの点を足す（横の間隔を残せる所だけ）。足した番号も返す。
    pub fn add_curve_point(ramp: &Ramp, x: f64, y: f64) -> Option<(Ramp, usize)> {
        let (next, k) = curve::ops::add_point(ramp.value_curve(), x, y)?;
        Some((ramp.with_value_curve(next), k))
    }

    /// カーブの点を動かす（両端は縦だけ。ほかは隣との間隔を残す）。
    pub fn move_curve_point(ramp: &Ramp, k: usize, x: f64, y: f64) -> Option<Ramp> {
        Some(ramp.with_value_curve(curve::ops::move_point(ramp.value_curve(), k, x, y)?))
    }

    /// カーブの点を消す（両端・2 点は消さない）。
    pub fn remove_curve_point(ramp: &Ramp, k: usize) -> Option<Ramp> {
        Some(ramp.with_value_curve(curve::ops::remove_point(ramp.value_curve(), k)?))
    }

    /// カーブのよく使う形（線形・やわらかい・かたい・S 字。Unity 版の筆圧のカーブと同じ点）。
    pub const CURVE_PRESETS: [&[(f64, f64)]; 4] = curve::ops::PRESETS;

    pub fn curve_preset(ramp: &Ramp, index: usize) -> Option<Ramp> {
        Some(ramp.with_value_curve(curve::ops::preset(index)?))
    }

    /// 選んだ分岐点の位置（行と番号が範囲外なら 0）。
    pub fn selected_position(ramp: &Ramp, sel: Selection) -> f64 {
        if sel.alpha {
            ramp.opacities().get(sel.index).map_or(0.0, |s| s.position)
        } else {
            ramp.colors().get(sel.index).map_or(0.0, |s| s.position)
        }
    }

    /// 選びが範囲に収まるようにする（分岐点を消した・ランプが替わったあと）。
    pub fn clamp_selection(ramp: &Ramp, sel: Selection) -> Selection {
        let count = if sel.alpha {
            ramp.opacities().len()
        } else {
            ramp.colors().len()
        };
        Selection {
            index: sel.index.min(count.saturating_sub(1)),
            alpha: sel.alpha,
        }
    }
}

// ───────── 分岐点の編集 ─────────

/// 部品の高さ。
pub const STOPS_HEIGHT: f32 = 76.0;
const SIDE: f32 = 7.0;
const OPACITY_Y: f32 = 7.0;
const OPACITY_MID_Y: f32 = 20.0;
const BAR_Y: f32 = 27.0;
const BAR_H: f32 = 20.0;
const COLOR_MID_Y: f32 = 54.0;
const COLOR_Y: f32 = 67.0;

/// ドラッグの途中（下書き）。
#[derive(Clone)]
struct StopDrag {
    index: usize,
    alpha: bool,
    middle: bool,
    original: Ramp,
    draft: Ramp,
}

fn lane(y: f32, r: Rect) -> (bool, bool) {
    // (不透明度の行か, 中点の行か)
    let alpha = y < r.top() + 25.0;
    let middle = if alpha {
        y > r.top() + 13.0
    } else {
        y < r.top() + 61.0
    };
    (alpha, middle)
}

#[allow(clippy::too_many_arguments)]
/// 分岐点の編集。変更が決まったとき（足した・消した・ドラッグを離した）だけ新しいランプを返す。`scalar` はデータのチャンネル
/// （色を輝度の灰色で見せる）。
pub fn stops_editor(
    ui: &mut Ui,
    r: Rect,
    id_salt: impl egui::AsIdSalt,
    ramp: &Ramp,
    selection: &mut Selection,
    scalar: bool,
    tooltip: &str,
    enabled: bool,
) -> Option<Ramp> {
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
    let mut drag: Option<StopDrag> = ui.data(|d| d.get_temp(drag_id));
    // 別のランプのドラッグ（層を替えたなど）の下書きは、今のランプへ当てずに捨てる
    if drag.as_ref().is_some_and(|d| d.original != *ramp) {
        ui.data_mut(|data| data.remove::<StopDrag>(drag_id));
        drag = None;
    }
    let left = r.left() + SIDE;
    let width = (r.width() - 2.0 * SIDE).max(1.0);
    let position_at = |x: f32| (f64::from(x - left) / f64::from(width)).clamp(0.0, 1.0);
    let mut result: Option<Ramp> = None;
    let pointer = ui.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()));
    let (pressed, secondary, down) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.secondary_pressed(),
            i.pointer.primary_down(),
        )
    });
    let shown_for_hit = drag.as_ref().map_or(ramp, |d| &d.draft).clone();
    if enabled && drag.is_none() && (pressed || secondary) && response.hovered() {
        if let Some(m) = pointer {
            let (alpha, middle) = lane(m.y, r);
            // 色の行と不透明度の行の間の帯は何もしない
            if !(m.y >= r.top() + 25.0 && m.y <= r.top() + 47.0) {
                let p = position_at(m.x);
                let count = if alpha {
                    shown_for_hit.opacities().len()
                } else {
                    shown_for_hit.colors().len()
                };
                let mut hit: Option<usize> = None;
                let mut best = f64::from(8.0 / width);
                for k in 0..count - usize::from(middle) {
                    let (pos, next, mid) = if alpha {
                        let o = shown_for_hit.opacities();
                        (
                            o[k].position,
                            o.get(k + 1).map(|s| s.position),
                            o[k].midpoint,
                        )
                    } else {
                        let c = shown_for_hit.colors();
                        (
                            c[k].position,
                            c.get(k + 1).map(|s| s.position),
                            c[k].midpoint,
                        )
                    };
                    let at = if middle {
                        pos + (next.unwrap_or(pos) - pos) * mid
                    } else {
                        pos
                    };
                    let d = (p - at).abs();
                    if d < best {
                        best = d;
                        hit = Some(k);
                    }
                }
                if secondary {
                    if !middle {
                        if let Some(k) = hit {
                            result = ops::remove(ramp, alpha, k);
                        }
                    }
                } else {
                    let mut working = ramp.clone();
                    if hit.is_none() && !middle {
                        let added = if alpha {
                            ops::add_opacity(ramp, p)
                        } else {
                            ops::add_color(ramp, p, scalar)
                        };
                        if let Some((next, k)) = added {
                            working = next;
                            hit = Some(k);
                        }
                    }
                    if let Some(k) = hit {
                        *selection = Selection { index: k, alpha };
                        drag = Some(StopDrag {
                            index: k,
                            alpha,
                            middle,
                            original: ramp.clone(),
                            draft: working,
                        });
                    }
                }
            }
        }
    }
    if let Some(mut d) = drag.take() {
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if escape {
            // 元のまま（文書も履歴も触らない）
            ui.data_mut(|data| data.remove::<StopDrag>(drag_id));
        } else if down {
            if let Some(m) = pointer {
                let p = position_at(m.x);
                let next = if d.middle {
                    ops::move_midpoint(&d.draft, d.alpha, d.index, p)
                } else {
                    ops::move_stop(&d.draft, d.alpha, d.index, p)
                };
                if let Some(next) = next {
                    d.draft = next;
                }
            }
            ui.data_mut(|data| data.insert_temp(drag_id, d));
        } else {
            // 離した: 行の外まで引き出していたら消す（中点は消さない）
            let outside = pointer.is_some_and(|m| m.y < r.top() - 16.0 || m.y > r.bottom() + 16.0);
            let mut done = d.draft.clone();
            if outside && !d.middle {
                if let Some(next) = ops::remove(&done, d.alpha, d.index) {
                    done = next;
                }
            }
            if done != d.original {
                result = Some(done);
            }
            ui.data_mut(|data| data.remove::<StopDrag>(drag_id));
        }
    }
    let live: Option<Ramp> = ui
        .data(|data| data.get_temp::<StopDrag>(drag_id))
        .map(|d| d.draft);
    let shown = live.as_ref().unwrap_or(ramp);
    *selection = ops::clamp_selection(shown, *selection);
    if ui.is_rect_visible(r) {
        paint_stops(ui, r, shown, *selection, scalar);
    }
    let _ = response.on_hover_text(tooltip);
    result
}

fn paint_stops(ui: &Ui, r: Rect, ramp: &Ramp, selection: Selection, scalar: bool) {
    let p = ui.painter();
    let left = r.left() + SIDE;
    let width = (r.width() - 2.0 * SIDE).max(1.0);
    let bar = Rect::from_min_size(pos2(left, r.top() + BAR_Y), vec2(width, BAR_H));
    checker(p, bar, 5.0);
    const SAMPLES: usize = 128;
    for k in 0..SAMPLES {
        if let Ok(c) = ramp.sample_stops(k as f64 / (SAMPLES - 1) as f64, scalar) {
            let x0 = left + width * k as f32 / SAMPLES as f32;
            fill(
                p,
                Rect::from_min_size(
                    pos2(x0, bar.top()),
                    vec2(width / SAMPLES as f32 + 1.0, BAR_H),
                ),
                Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a),
            );
        }
    }
    outline(p, bar, t::BORDER, 1.0, 0.0);
    let mark = |pos: f64, y: f32, color: Color32, picked: bool, diamond: bool| {
        let center = pos2(left + pos as f32 * width, r.top() + y);
        if diamond {
            for row in -4i32..=4 {
                let half = 4 - row.abs();
                fill(
                    p,
                    Rect::from_min_size(
                        pos2(center.x - half as f32, center.y + row as f32),
                        vec2(half as f32 * 2.0 + 1.0, 1.0),
                    ),
                    Color32::BLACK,
                );
                if half > 0 {
                    fill(
                        p,
                        Rect::from_min_size(
                            pos2(center.x - half as f32 + 1.0, center.y + row as f32),
                            vec2(half as f32 * 2.0 - 1.0, 1.0),
                        ),
                        color,
                    );
                }
            }
        } else {
            let b = Rect::from_center_size(center, vec2(8.0, 8.0));
            fill(p, b, color);
            outline(
                p,
                b,
                if picked {
                    Color32::YELLOW
                } else {
                    Color32::BLACK
                },
                if picked { 2.0 } else { 1.0 },
                0.0,
            );
        }
    };
    let colors = ramp.colors();
    for (k, s) in colors.iter().enumerate() {
        mark(
            s.position,
            COLOR_Y,
            Color32::from_rgb(s.color.r, s.color.g, s.color.b),
            !selection.alpha && selection.index == k,
            false,
        );
        if let Some(next) = colors.get(k + 1) {
            mark(
                s.position + (next.position - s.position) * s.midpoint,
                COLOR_MID_Y,
                Color32::GRAY,
                false,
                true,
            );
        }
    }
    let opacities = ramp.opacities();
    for (k, s) in opacities.iter().enumerate() {
        let g = (s.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        mark(
            s.position,
            OPACITY_Y,
            Color32::from_gray(g),
            selection.alpha && selection.index == k,
            false,
        );
        if let Some(next) = opacities.get(k + 1) {
            mark(
                s.position + (next.position - s.position) * s.midpoint,
                OPACITY_MID_Y,
                Color32::GRAY,
                false,
                true,
            );
        }
    }
}

// ───────── 値のカーブの編集 ─────────

/// 曲線の編集の部品の高さ。
pub const CURVE_HEIGHT: f32 = curve::HEIGHT;

/// 値のカーブの編集（横が形の値、縦がランプの位置）。変更が決まったとき（足した・消した・ドラッグを離した）だけ新しいランプを返す。
/// 点の操作は `ui::curve::curve_editor` と同じ。
pub fn curve_editor(
    ui: &mut Ui,
    r: Rect,
    id_salt: impl egui::AsIdSalt,
    ramp: &Ramp,
    tooltip: &str,
    enabled: bool,
) -> Option<Ramp> {
    curve::curve_editor(ui, r, id_salt, ramp.value_curve(), tooltip, enabled)
        .map(|next| ramp.with_value_curve(next))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::generator::Preset;

    fn ramp() -> Ramp {
        Ramp::default()
    }

    #[test]
    fn adding_a_stop_keeps_the_gradient_where_it_was() {
        let r = ramp();
        let (with, k) = ops::add_color(&r, 0.5, false).unwrap();
        assert_eq!(with.colors().len(), 3);
        assert_eq!(k, 1);
        for p in [0.0, 0.2, 0.5, 0.8, 1.0] {
            let a = r.sample_stops(p, false).unwrap();
            let b = with.sample_stops(p, false).unwrap();
            assert!(
                (i32::from(a.r) - i32::from(b.r)).abs() <= 1,
                "{p}: {a:?} {b:?}"
            );
        }
        // 近すぎる所には足さない
        assert!(ops::add_color(&with, 0.5 + 0.00001, false).is_none());
        // 不透明度は独立
        let (op, ok) = ops::add_opacity(&r, 0.25).unwrap();
        assert_eq!((op.opacities().len(), op.colors().len(), ok), (3, 2, 1));
    }

    #[test]
    fn removing_leaves_two_and_moving_stops_at_the_neighbours() {
        let r = ramp();
        assert!(ops::remove(&r, false, 0).is_none(), "2 つは残す");
        let (three, _) = ops::add_color(&r, 0.5, false).unwrap();
        let two = ops::remove(&three, false, 1).unwrap();
        assert_eq!(two.colors().len(), 2);
        // 動かす: 隣を越えない
        let moved = ops::move_stop(&three, false, 1, 5.0).unwrap();
        assert!(moved.colors()[1].position < 1.0 && moved.colors()[1].position > 0.99);
        let moved = ops::move_stop(&three, false, 1, -3.0).unwrap();
        assert!(moved.colors()[1].position > 0.0 && moved.colors()[1].position < 0.001);
        // 両端は 0 と 1 の外へ出ない
        let end = ops::move_stop(&three, false, 2, 3.0).unwrap();
        assert_eq!(end.colors()[2].position, 1.0);
    }

    #[test]
    fn a_midpoint_moves_between_its_two_stops() {
        let r = ramp();
        let m = ops::move_midpoint(&r, false, 0, 0.25).unwrap();
        assert!((m.colors()[0].midpoint - 0.25).abs() < 1e-9);
        let m = ops::move_midpoint(&r, false, 0, 5.0).unwrap();
        assert_eq!(m.colors()[0].midpoint, 0.99);
        assert!(
            ops::move_midpoint(&r, false, 1, 0.5).is_none(),
            "最後の分岐点には中点が無い"
        );
    }

    #[test]
    fn curve_points_add_move_and_remove_with_fixed_ends() {
        let r = ramp();
        let (with, k) = ops::add_curve_point(&r, 0.5, 0.2).unwrap();
        assert_eq!((with.curve().len(), k), (3, 1));
        assert!((with.curve_value(0.5).unwrap() - 0.2).abs() < 0.02);
        // 端は横に動かない
        let end = ops::move_curve_point(&with, 0, 0.4, 0.3).unwrap();
        assert_eq!((end.curve()[0].x, end.curve()[0].y), (0.0, 0.3));
        let end = ops::move_curve_point(&with, 2, 0.4, 0.9).unwrap();
        assert_eq!(end.curve()[2].x, 1.0);
        // 隣との間隔を残す
        let squeezed = ops::move_curve_point(&with, 1, 5.0, 0.5).unwrap();
        assert!(squeezed.curve()[1].x <= 1.0 - MIN_CURVE_GAP + 1e-9);
        // 消せるのは途中の点だけ
        assert!(ops::remove_curve_point(&with, 0).is_none());
        assert!(ops::remove_curve_point(&with, 2).is_none());
        assert_eq!(ops::remove_curve_point(&with, 1).unwrap().curve().len(), 2);
        assert!(
            ops::add_curve_point(&with, 0.505, 0.2).is_none(),
            "間隔が足りない"
        );
        // 点の数の上限
        let mut many = r.clone();
        for i in 1..15 {
            many = ops::add_curve_point(&many, i as f64 / 16.0, 0.5).unwrap().0;
        }
        assert_eq!(many.curve().len(), MAX_CURVE_POINTS);
        assert!(ops::add_curve_point(&many, 0.03, 0.5).is_none());
    }

    #[test]
    fn curve_presets_are_valid_and_the_first_is_linear() {
        let base = Ramp::preset(
            Preset::WarmCool,
            Rgba8::new(0, 0, 0, 255),
            Rgba8::new(255, 255, 255, 255),
        );
        for (i, _) in ops::CURVE_PRESETS.iter().enumerate() {
            let r = ops::curve_preset(&base, i).unwrap();
            assert_eq!(r.colors(), base.colors(), "色は変えない");
        }
        let linear = ops::curve_preset(&base, 0).unwrap();
        assert!((linear.curve_value(0.3).unwrap() - 0.3).abs() < 1e-9);
        let soft = ops::curve_preset(&base, 1).unwrap();
        assert!(soft.curve_value(0.3).unwrap() > 0.5);
        assert!(ops::curve_preset(&base, 9).is_none());
    }

    #[test]
    fn the_selection_stays_inside_the_stops() {
        let r = ramp();
        let sel = ops::clamp_selection(
            &r,
            Selection {
                index: 9,
                alpha: false,
            },
        );
        assert_eq!(sel.index, 1);
        assert_eq!(
            ops::selected_position(
                &r,
                Selection {
                    index: 1,
                    alpha: true
                }
            ),
            1.0
        );
    }
}
