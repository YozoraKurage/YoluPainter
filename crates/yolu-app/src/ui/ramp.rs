//! グラデーションのランプの編集の部品（Unity 版の `PaintGui.GradientEditor` と曲線の編集）: 不透明度の分岐点（上の行）と色（値）の分岐点
//! （下の行）が独立で、分岐点の間の中点（小さなひし形）と、形の値からランプの位置への値のカーブを持つ。
//!
//! - 分岐点は、何も無い所を押すと足し（その位置のランプの色・不透明度で）、ドラッグで動かし、右クリックか、行の外へドラッグして離すと消す
//!   （2 つは残す）。ひし形は中点を動かす。ドラッグは下書きに溜め、離したところで 1 回の変更として返す（Esc は元のまま。文書も履歴も触らない）。
//! - 値のカーブの編集は `ui::curve`（トーンカーブ・筆圧のカーブと共通の部品）へ委ね、ここは `Ramp` の値のカーブへの出し入れだけ。
//! - 編集の計算（足す・消す・動かす）は `ops` にまとめた純粋な関数で、試験が直に確かめる。

use egui::{pos2, vec2, Color32, Rect, Sense, Ui};
use yolu_core::curve::Curve;
use yolu_core::generator::{ColorStop, CurvePoint, OpacityStop, Ramp};
use yolu_core::Rgba8;

use super::curve;
use super::settle;
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

    /// 色の分岐点を差し替える（値のカーブ・混色は残し、混合率曲線は数が同じときだけ残す）。
    pub fn with_colors(ramp: &Ramp, colors: Vec<ColorStop>) -> Option<Ramp> {
        ramp.with_stops(colors, ramp.opacities().to_vec()).ok()
    }
    pub fn with_opacities(ramp: &Ramp, opacities: Vec<OpacityStop>) -> Option<Ramp> {
        ramp.with_stops(ramp.colors().to_vec(), opacities).ok()
    }
    pub fn with_curve(ramp: &Ramp, curve: Vec<CurvePoint>) -> Option<Ramp> {
        Some(ramp.with_value_curve(Curve::new(curve).ok()?))
    }

    /// 混合率曲線を左右に分ける用の標本の数（曲線の点は 9 個になる。点の間隔の下限 0.02 より十分に広い）。
    const SPLIT_SAMPLES: usize = 8;

    /// 区間の混合率曲線 `curve` を、区間の内側の位置 `u`（0〜1）で左右に分ける。分岐点で割った 2 つの区間が、元の混ざり方（色の線形の補間では
    /// 左の区間の重み = 元の重み / 位置 u の重み、右は (元の重み − u の重み) / (1 − u の重み)）に近くなるよう、9 点で標本にして曲線を作り直す。
    /// 割る位置の重みが 0 か 1 に張り付いて割れない（割り算できない）ときの側は `None`（中点の混ざり方へ戻る）。
    pub fn split_segment_curve(curve: &Curve, u: f64) -> (Option<Curve>, Option<Curve>) {
        let at = |x: f64| curve.value_unchecked(x);
        let wp = at(u);
        let make = |f: &dyn Fn(f64) -> f64| {
            let points = (0..=SPLIT_SAMPLES)
                .map(|i| {
                    let v = i as f64 / SPLIT_SAMPLES as f64;
                    CurvePoint {
                        x: v,
                        y: f(v).clamp(0.0, 1.0),
                    }
                })
                .collect();
            Curve::new(points).ok()
        };
        let left = (wp > 1e-6).then(|| make(&|v| at(u * v) / wp)).flatten();
        let right = (wp < 1.0 - 1e-6)
            .then(|| make(&|v| (at(u + (1.0 - u) * v) - wp) / (1.0 - wp)))
            .flatten();
        (left, right)
    }

    /// 色の分岐点を足したあとの混合率曲線（`new_index` が足した番号。内側なら元の区間を左右に分け、外側なら新しい区間は曲線なし）。
    fn segments_after_add(ramp: &Ramp, new_index: usize, p: f64) -> Vec<Option<Curve>> {
        let old = ramp.segment_curves();
        let n_old = ramp.colors().len();
        if old.is_empty() {
            return Vec::new();
        }
        let mut next: Vec<Option<Curve>> = Vec::with_capacity(n_old);
        if new_index == 0 {
            next.push(None);
            next.extend(old.iter().cloned());
        } else if new_index >= n_old {
            next.extend(old.iter().cloned());
            next.push(None);
        } else {
            // 元の区間 new_index − 1 を左右に分ける
            let k = new_index - 1;
            let (a, b) = (ramp.colors()[k].position, ramp.colors()[k + 1].position);
            let u = ((p - a) / (b - a)).clamp(0.0, 1.0);
            let (left, right) = match &old[k] {
                Some(c) => split_segment_curve(c, u),
                None => (None, None),
            };
            next.extend(old[..k].iter().cloned());
            next.push(left);
            next.push(right);
            next.extend(old[k + 1..].iter().cloned());
        }
        next
    }

    /// 色の分岐点 `k` を消したあとの混合率曲線（両隣の区間は 1 つにつながり、曲線なし。端なら、その側の区間だけ無くなる）。
    fn segments_after_remove(ramp: &Ramp, k: usize) -> Vec<Option<Curve>> {
        let old = ramp.segment_curves();
        if old.is_empty() {
            return Vec::new();
        }
        let n = ramp.colors().len();
        let mut next: Vec<Option<Curve>> = old.to_vec();
        if k == 0 {
            next.remove(0);
        } else if k == n - 1 {
            next.pop();
        } else {
            next.splice(k - 1..=k, [None]);
        }
        next
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
        let segments = segments_after_add(ramp, index, p);
        let next = with_colors(ramp, list)?
            .with_segment_curves(segments)
            .ok()?;
        Some((next, index))
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
            let segments = segments_after_remove(ramp, k);
            list.remove(k);
            with_colors(ramp, list)?.with_segment_curves(segments).ok()
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

/// 色の分岐点をダブルクリックした知らせ（色の選びを、その分岐点のそばに出す頼み）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorRequest {
    /// 色の分岐点の番号。
    pub index: usize,
    /// 分岐点の印のあたり（画面の点。色の選びを置く目安）。
    pub anchor: Rect,
}

/// ダブルクリックの知らせを 1 回だけ取り出す（`stops_editor` と同じ `id_salt`）。
pub fn take_color_request(ui: &Ui, id_salt: impl egui::AsIdSalt) -> Option<ColorRequest> {
    let id = ui.make_persistent_id(id_salt).with("color-request");
    let request = ui.data(|d| d.get_temp::<ColorRequest>(id));
    if request.is_some() {
        ui.data_mut(|d| d.remove::<ColorRequest>(id));
    }
    request
}

/// 位置 `p`（0〜1）に一番近い分岐点（`lane` の行の中点でなく分岐点そのもの）。`width` は棒の幅（点）で、8 点の内側だけ拾う。
fn nearest_stop(ramp: &Ramp, alpha: bool, p: f64, width: f32) -> Option<usize> {
    let positions: Vec<f64> = if alpha {
        ramp.opacities().iter().map(|s| s.position).collect()
    } else {
        ramp.colors().iter().map(|s| s.position).collect()
    };
    let mut best = f64::from(8.0 / width);
    let mut hit = None;
    for (k, pos) in positions.into_iter().enumerate() {
        let d = (p - pos).abs();
        if d < best {
            best = d;
            hit = Some(k);
        }
    }
    hit
}

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
    // 離した直後は、文書が追いつくまで離した値を見せる（元の位置へ一瞬戻らない）
    let settled = settle::shown(ui, id, ramp);
    let shown_for_hit = drag
        .as_ref()
        .map(|d| &d.draft)
        .or(settled.as_ref())
        .unwrap_or(ramp)
        .clone();
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
    // 色の分岐点のダブルクリック: その分岐点を選んで、色の選びを出す頼みを残す
    if enabled && response.double_clicked() {
        if let Some(m) = pointer {
            let (alpha, middle) = lane(m.y, r);
            let between = m.y >= r.top() + 25.0 && m.y <= r.top() + 47.0;
            let now = result.as_ref().unwrap_or(&shown_for_hit);
            if !alpha && !middle && !between {
                if let Some(k) = nearest_stop(now, false, position_at(m.x), width) {
                    *selection = Selection {
                        index: k,
                        alpha: false,
                    };
                    let x = left + now.colors()[k].position as f32 * width;
                    let anchor =
                        Rect::from_center_size(pos2(x, r.top() + COLOR_Y), vec2(10.0, 10.0));
                    ui.data_mut(|d| {
                        d.insert_temp(id.with("color-request"), ColorRequest { index: k, anchor })
                    });
                }
            }
        }
    }
    let live: Option<Ramp> = ui
        .data(|data| data.get_temp::<StopDrag>(drag_id))
        .map(|d| d.draft);
    // 離したフレームも、離した値を見せる（この関数が返す値を呼び手が当てるのは描いたあと）
    if let Some(done) = &result {
        settle::start(ui, id, ramp, done);
    }
    let settled = result
        .clone()
        .or_else(|| settle::shown(ui, id, ramp))
        .or(settled);
    let shown = live.as_ref().or(settled.as_ref()).unwrap_or(ramp);
    *selection = ops::clamp_selection(shown, *selection);
    ui.data_mut(|d| d.insert_temp(id.with("painted"), shown.clone()));
    if ui.is_rect_visible(r) {
        paint_stops(ui, r, shown, *selection, scalar);
    }
    let _ = response.on_hover_text(tooltip);
    result
}

/// この部品が直前に描いたランプ（ドラッグの下書き・離した直後の値を含む）。部品の外から「何が見えているか」を確かめる試験の口。
pub fn last_painted(ui: &Ui, id_salt: impl egui::AsIdSalt) -> Option<Ramp> {
    let id = ui.make_persistent_id(id_salt);
    ui.data(|d| d.get_temp(id.with("painted")))
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

    fn coloured() -> Ramp {
        let stop = |position, rgb: [u8; 3]| ColorStop {
            position,
            color: Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
            midpoint: 0.5,
        };
        Ramp::new(
            vec![
                stop(0.0, [10, 20, 120]),
                stop(0.5, [200, 60, 90]),
                stop(1.0, [250, 230, 120]),
            ],
            Ramp::default().opacities().to_vec(),
            None,
        )
        .unwrap()
    }

    fn bent(points: &[(f64, f64)]) -> Curve {
        Curve::new(
            points
                .iter()
                .map(|&(x, y)| CurvePoint { x, y })
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    #[test]
    fn adding_a_stop_inside_a_curved_segment_splits_the_curve_and_keeps_the_look() {
        let base = coloured()
            .with_segment_curve(0, Some(bent(&[(0.0, 0.0), (0.3, 0.8), (1.0, 1.0)])))
            .unwrap();
        let (with, k) = ops::add_color(&base, 0.2, false).unwrap();
        assert_eq!(k, 1);
        assert_eq!(with.segment_curves().len(), 3);
        assert!(with.segment_curve(0).is_some() && with.segment_curve(1).is_some());
        assert!(
            with.segment_curve(2).is_none(),
            "触っていない区間はそのまま"
        );
        for p in [0.02, 0.1, 0.19, 0.21, 0.3, 0.4, 0.6, 0.9] {
            let (a, b) = (
                base.sample_stops(p, false).unwrap(),
                with.sample_stops(p, false).unwrap(),
            );
            for (x, y) in [(a.r, b.r), (a.g, b.g), (a.b, b.b)] {
                assert!((i32::from(x) - i32::from(y)).abs() <= 8, "{p}: {a:?} {b:?}");
            }
        }
        // 外側（最初の分岐点の前の隙間が無ければ足せない）と、曲線の無い区間は曲線を作らない
        let plain = coloured();
        let (added, _) = ops::add_color(&plain, 0.2, false).unwrap();
        assert!(added.segment_curves().is_empty());
    }

    #[test]
    fn removing_a_stop_joins_its_segments_without_a_curve_and_keeps_the_rest() {
        let curve = bent(&[(0.0, 0.0), (0.4, 0.7), (1.0, 1.0)]);
        let (four, _) = ops::add_color(&coloured(), 0.75, false).unwrap();
        let four = four
            .with_segment_curves(vec![
                Some(curve.clone()),
                Some(curve.clone()),
                Some(curve.clone()),
            ])
            .unwrap();
        // 内側の分岐点: 両隣の区間が 1 つにつながり、曲線なし
        let less = ops::remove(&four, false, 1).unwrap();
        assert_eq!(less.segment_curves().len(), 2);
        assert!(less.segment_curve(0).is_none(), "つなげた区間");
        assert!(less.segment_curve(1).is_some(), "触っていない区間");
        // 端の分岐点: その側の区間だけ無くなる
        let first = ops::remove(&four, false, 0).unwrap();
        assert_eq!(first.segment_curves().len(), 2);
        assert!(first.segment_curve(0).is_some() && first.segment_curve(1).is_some());
        let last = ops::remove(&four, false, 3).unwrap();
        assert_eq!(last.segment_curves().len(), 2);
        // 混色は残る
        let mixed = four.with_mixing(
            yolu_core::generator::MixMode::Perceptual,
            yolu_core::generator::LuminanceCorrection::Low,
        );
        assert_eq!(
            ops::remove(&mixed, false, 1).unwrap().mix_mode(),
            yolu_core::generator::MixMode::Perceptual
        );
    }

    #[test]
    fn moving_and_recolouring_a_stop_keep_the_segment_curves() {
        let base = coloured()
            .with_segment_curve(1, Some(bent(&[(0.0, 0.0), (0.5, 0.2), (1.0, 1.0)])))
            .unwrap();
        let moved = ops::move_stop(&base, false, 1, 0.4).unwrap();
        assert!(moved.segment_curve(1).is_some());
        let mut list = base.colors().to_vec();
        list[0].color = Rgba8::new(1, 2, 3, 255);
        assert!(ops::with_colors(&base, list)
            .unwrap()
            .segment_curve(1)
            .is_some());
        // 不透明度の分岐点を足しても、色の混合率曲線は残る
        let (op, _) = ops::add_opacity(&base, 0.3).unwrap();
        assert!(op.segment_curve(1).is_some());
    }

    #[test]
    fn splitting_a_curve_reproduces_its_shape_on_both_sides() {
        let curve = bent(&[(0.0, 0.0), (0.3, 0.8), (1.0, 1.0)]);
        let u = 0.4;
        let (left, right) = ops::split_segment_curve(&curve, u);
        let (left, right) = (left.unwrap(), right.unwrap());
        let wp = curve.value(u).unwrap();
        for v in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let want_left = curve.value(u * v).unwrap() / wp;
            assert!(
                (left.value(v).unwrap() - want_left).abs() < 0.05,
                "左 {v}: {} {want_left}",
                left.value(v).unwrap()
            );
            let want_right = (curve.value(u + (1.0 - u) * v).unwrap() - wp) / (1.0 - wp);
            assert!(
                (right.value(v).unwrap() - want_right).abs() < 0.05,
                "右 {v}"
            );
        }
        // 張り付いた重み（割り算できない側）は作らない
        let flat = bent(&[(0.0, 0.0), (1.0, 0.0)]);
        assert_eq!(ops::split_segment_curve(&flat, 0.5).0, None);
        assert!(ops::split_segment_curve(&flat, 0.5).1.is_some());
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
