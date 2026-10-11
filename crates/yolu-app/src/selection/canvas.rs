//! 選択のツールのキャンバスの入力（押す・動く・離す・Esc・Enter・Backspace）と、キャンバスの上の表示（選択の縁の流れる点線・
//! ドラッグ中の形・多角形の途中・対称の軸と映した側のカーソル）。
//!
//! 形はキャンバスの座標（左下が原点）で決まり、表示を回している・反転しているときは回って見える（矩形・楕円はキャンバスの軸に沿う。
//! Photoshop・CLIP STUDIO と同じ）。組み合わせ方は Shift で足す・Ctrl で引く・両方で重ねる（離したときの修飾。Unity 版と同じ）で、
//! 修飾が無ければオプションバーの値。ストロークの最中・読むだけのセットでは始めない。フォーカスを失う・Esc・ツールの切り替えで、
//! 途中の形は捨てる（取り残さない）。

use std::time::Duration;

use egui::{Color32, Modifiers, Painter, Pos2, Rect, Shape, Stroke};

use super::outline::Run;
use super::overlay::Tint;
use super::{combine_of, pen, quick, shape, SelAction, SelEdit, ShapeDrag};
use crate::canvas::view::CanvasView;
use crate::engine::{CanvasSymmetry, SelectionCombine};
use crate::lang::Lang;
use crate::notice::Source;
use crate::rulers::draw::mirrored_points;
use crate::state::{Action, AppState, StrokeSource, Tool};
use crate::ui::theme as t;
use crate::ui::widgets as w;

/// これ以内（画面の点）しか動かさなければ、ドラッグでなくクリック。
pub(crate) const CLICK_RADIUS: f32 = 4.0;
/// 多角形の始めの点にこれ以内（画面の点）で押すと閉じる。
pub(crate) const CLOSE_RADIUS: f32 = 9.0;
/// 多角形をダブルクリックで閉じる間隔（秒）。
pub(crate) const DOUBLE_CLICK: f64 = 0.4;
/// 点線で流す縁の線分の上限（画面に見える数。これより多いときは点線にせず、動かさない実線で描く）。
const MAX_DASHED_RUNS: usize = 6_000;
/// 1 画面で描く縁の線分の上限（それより多いときは先頭から描く）。
const MAX_DRAWN_RUNS: usize = 40_000;
/// 点線を流す間は、この間隔で描き直す。
const ANTS_FRAME: Duration = Duration::from_millis(60);
/// 選択ペンのストロークの被覆の色（足す: 青、消す: 橙）と濃さ。
const PEN_ADD_TINT: Tint = Tint {
    rgb: [70, 150, 255],
    alpha: 0.45,
};
const PEN_ERASE_TINT: Tint = Tint {
    rgb: [255, 160, 40],
    alpha: 0.45,
};
/// 点線の黒の長さと 1 周期（画面の点）。
const DASH_LENGTH: f32 = 4.0;
const DASH_PERIOD: f32 = 8.0;
/// 線分を切る矩形を、画面の矩形より広げる余白（線の太さや点線の切れ目が画面にかからないように）。
const CLIP_MARGIN: f32 = 8.0;

/// 押した（ペン・マウス）。
pub fn press(
    app: &mut AppState,
    view: &CanvasView,
    pos: Pos2,
    source: StrokeSource,
    modifiers: Modifiers,
    now: f64,
) {
    // 3D ビューで形を引いている間は、2D のキャンバスでは始めない（選択範囲の途中の形は 1 つ）
    if app.is_stroking() || !app.tool.is_select() || crate::view3d::select::dragging(app) {
        return;
    }
    if let Some(reason) = app.read_only_reason().map(str::to_owned) {
        app.refuse(
            Source::Selection,
            crate::lang::refusals::read_only_set(app.lang, &reason),
        );
        return;
    }
    let canvas = view.to_canvas(pos);
    app.sel.press_modifiers = modifiers;
    match app.tool {
        Tool::SelectPen => pen::press(app, view, pos, source, modifiers),
        Tool::Wand => {
            let x = canvas.0.floor().clamp(0.0, (app.doc.width() - 1) as f64) as u32;
            let y = canvas.1.floor().clamp(0.0, (app.doc.height() - 1) as f64) as u32;
            // 押した画素（キャンバスの外は端の画素へ寄せる）の中心から、対称定規の写しの種を作る。左右の写しが画素で揃い、外の写しは捨てる
            let seeds = app
                .symmetry_seeds((x as f64 + 0.5, y as f64 + 0.5), app.sel.snap_symmetry)
                .into_iter()
                .map(|(sx, sy)| (sx as u32, sy as u32))
                .collect();
            let mode = combine_of(app.sel.combine, app.sel.press_button, modifiers);
            app.apply(Action::Sel(SelAction::Edit(SelEdit::Wand { seeds, mode })));
        }
        Tool::Polygon => polygon_press(app, view, pos, canvas, modifiers, now),
        tool => {
            app.sel.drag = Some(ShapeDrag {
                tool,
                source,
                start_screen: pos,
                start: canvas,
                current: canvas,
                lasso: vec![canvas],
                moved: 0.0,
            });
        }
    }
}

fn polygon_press(
    app: &mut AppState,
    view: &CanvasView,
    pos: Pos2,
    canvas: (f64, f64),
    modifiers: Modifiers,
    now: f64,
) {
    let double = app
        .sel
        .last_press
        .is_some_and(|(t, p)| now - t <= DOUBLE_CLICK && p.distance(pos) <= CLICK_RADIUS * 2.0);
    app.sel.last_press = Some((now, pos));
    // 3D ビューで打っていた途中の点は捨てる（途中の多角形は 1 つ）
    app.sel.view3d.drop_polygon();
    let n = app.sel.polygon.len();
    if n >= 3 {
        let first = app.sel.polygon[0];
        let near_first = view.to_screen(first.0, first.1).distance(pos) <= CLOSE_RADIUS;
        if double || near_first {
            finish_polygon(app, modifiers);
            return;
        }
    } else if double && n > 0 {
        return; // 点が足りないままのダブルクリックは、同じ所に点を重ねない
    }
    app.sel.polygon.push(canvas);
    app.sel.polygon_hover = Some(canvas);
}

/// 多角形を閉じて選択範囲にする（Enter・始めの点・ダブルクリック）。3 つ未満なら捨てる。
pub fn finish_polygon(app: &mut AppState, modifiers: Modifiers) {
    let points = std::mem::take(&mut app.sel.polygon);
    app.sel.polygon_hover = None;
    app.sel.last_press = None;
    if points.is_empty() {
        return;
    }
    if points.len() < 3 {
        refuse_too_few_points(app);
        return;
    }
    let mode = combine_of(app.sel.combine, app.sel.press_button, modifiers);
    app.apply(Action::Sel(SelAction::Edit(SelEdit::Polygon {
        points,
        mode,
    })));
}

/// 多角形の点が 3 つに満たないときの断り（2D のキャンバスと 3D ビューで同じ文）。
pub(crate) fn refuse_too_few_points(app: &mut AppState) {
    app.refuse(
        Source::Selection,
        app.lang
            .pick("点が足りません（3 つ以上）。", "Needs at least 3 points."),
    );
}

/// 多角形の最後の点を消す（Backspace。2D のキャンバスで打った点だけ。3D ビューの点は 3D の入力が消す）。
pub fn remove_last_point(app: &mut AppState) {
    app.sel.polygon.pop();
    if app.sel.polygon.is_empty() {
        app.sel.polygon_hover = None;
        app.sel.last_press = None;
    }
}

/// ポインタが動いた（ドラッグの形・多角形のゴムの線を更新する）。
pub fn moved(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource) {
    let canvas = view.to_canvas(pos);
    if app.tool == Tool::Polygon && !app.sel.polygon.is_empty() {
        app.sel.polygon_hover = Some(canvas);
    }
    let Some(drag) = app.sel.drag.as_mut() else {
        return;
    };
    if drag.source != source {
        return;
    }
    drag.current = canvas;
    drag.moved = drag.moved.max(pos.distance(drag.start_screen));
    if drag.tool == Tool::Lasso {
        let far = drag
            .lasso
            .last()
            .is_none_or(|l| ((l.0 - canvas.0).powi(2) + (l.1 - canvas.1).powi(2)).sqrt() >= 1.0);
        if far {
            drag.lasso.push(canvas);
        }
    }
    if drag.tool == Tool::SelectPen {
        pen::moved(app, view, pos, source);
    }
}

/// 離した。形を選択範囲にする（クリックなら解除。作成方法を追加・削除・共通にしていれば何もしない）。
pub fn release(
    app: &mut AppState,
    view: &CanvasView,
    pos: Pos2,
    source: StrokeSource,
    modifiers: Modifiers,
) {
    if app.sel.drag.as_ref().is_none_or(|d| d.source != source) {
        return;
    }
    let Some(mut drag) = app.sel.drag.take() else {
        return;
    };
    if drag.tool == Tool::SelectPen {
        // 離したら、ストロークを選択範囲に反映する（1 回の Undo）
        pen::finish(app, false);
        return;
    }
    let canvas = view.to_canvas(pos);
    drag.current = canvas;
    drag.moved = drag.moved.max(pos.distance(drag.start_screen));
    if drag.tool == Tool::Lasso {
        drag.lasso.push(canvas);
    }
    let click = drag.moved < CLICK_RADIUS;
    let pressed_with = app.sel.press_modifiers;
    let mode = drag_mode(
        app.sel.combine,
        drag.tool,
        app.sel.press_button,
        pressed_with,
        modifiers,
    );
    let c = shape::Constraint::of(&app.sel, drag.tool, pressed_with, modifiers);
    let (a, b) = shape::drag_corners(drag.start, drag.current, c.square, c.center);
    let corner_radius = app.sel.corner_radius;
    let edit = match drag.tool {
        _ if click => {
            if mode == SelectionCombine::Replace && app.doc.selection().is_some() {
                Some(SelEdit::Clear)
            } else {
                None
            }
        }
        Tool::SelectRect if corner_radius > 0 => Some(SelEdit::RoundRect {
            x0: a.0.min(b.0).round() as i64,
            y0: a.1.min(b.1).round() as i64,
            x1: a.0.max(b.0).round() as i64,
            y1: a.1.max(b.1).round() as i64,
            radius: corner_radius,
            mode,
        }),
        Tool::SelectRect => Some(SelEdit::Rect {
            x0: a.0.min(b.0).round() as i64,
            y0: a.1.min(b.1).round() as i64,
            x1: a.0.max(b.0).round() as i64,
            y1: a.1.max(b.1).round() as i64,
            mode,
        }),
        Tool::SelectEllipse => Some(SelEdit::Ellipse {
            cx: (a.0 + b.0) / 2.0,
            cy: (a.1 + b.1) / 2.0,
            rx: (b.0 - a.0).abs() / 2.0,
            ry: (b.1 - a.1).abs() / 2.0,
            mode,
        }),
        Tool::Lasso => Some(SelEdit::Polygon {
            points: drag.lasso,
            mode,
        }),
        _ => None,
    };
    if let Some(edit) = edit {
        app.apply(Action::Sel(SelAction::Edit(edit)));
    }
}

/// 形のドラッグの組み合わせ方。Shift は押し始めに押していれば「追加」、押し始めたあとに押したなら縦横比の固定（長方形・楕円）で、
/// 追加には使わない。Ctrl は押し始めか離したときのどちらかに押していれば「削除」。修飾が無ければオプションバーの値。
pub(crate) fn drag_mode(
    base: SelectionCombine,
    tool: Tool,
    button: egui::PointerButton,
    pressed_with: Modifiers,
    now: Modifiers,
) -> SelectionCombine {
    let constrains = matches!(tool, Tool::SelectRect | Tool::SelectEllipse)
        && shape::shift_constrains(pressed_with, now);
    let shift = pressed_with.shift || (now.shift && !constrains);
    let ctrl = pressed_with.ctrl || pressed_with.command || now.ctrl || now.command;
    combine_of(
        base,
        button,
        Modifiers {
            shift,
            ctrl,
            command: ctrl,
            ..Modifiers::NONE
        },
    )
}

/// ペン（Windows Ink）の 1 点。触れた・動いた・離したを、押す・動く・離すにする。
pub fn pen_sample(
    app: &mut AppState,
    view: &CanvasView,
    pos: Pos2,
    pointer_id: u32,
    contact: bool,
    modifiers: Modifiers,
    now: f64,
) {
    let source = StrokeSource::Pen(pointer_id);
    match (contact, app.sel.pen_down) {
        (true, None) => {
            app.sel.pen_down = Some(pointer_id);
            press(app, view, pos, source, modifiers, now);
        }
        (true, Some(id)) if id == pointer_id => moved(app, view, pos, source),
        (false, Some(id)) if id == pointer_id => {
            app.sel.pen_down = None;
            release(app, view, pos, source, modifiers);
        }
        _ => {}
    }
}

/// ペンの 1 点が来たことを覚える（選択ペンが筆圧と消しゴムの端を使う。キャンバスの入力が、ペンの点ごとに 1 度呼ぶ）。
pub fn note_pen(app: &mut AppState, pointer_id: u32, pressure: f32, eraser_end: bool) {
    app.sel.pen_note = Some((pointer_id, pressure, eraser_end));
}

/// Esc: 途中の形を捨てる。何かあったか。
pub fn cancel(app: &mut AppState) -> bool {
    let any = app.sel.cancel_drafts();
    if any {
        app.info(
            Source::Selection,
            app.lang
                .pick("選択の途中をやめました。", "Selection cancelled."),
        );
    }
    any
}

// ───────── 表示 ─────────

fn black() -> Stroke {
    Stroke::new(3.0, Color32::from_black_alpha(140))
}

fn white() -> Stroke {
    Stroke::new(1.2, Color32::from_white_alpha(230))
}

/// 折れ線（closed なら最後を始めに結ぶ）を、黒の縁取りと白で描く。
fn path(painter: &Painter, points: &[Pos2], closed: bool) {
    if points.len() < 2 {
        return;
    }
    let mut pts = points.to_vec();
    if closed {
        pts.push(points[0]);
    }
    painter.add(Shape::line(pts.clone(), black()));
    painter.add(Shape::line(pts, white()));
}

/// 縁の線分の画面の端点（画面の矩形で切ったあと）と、切り落とした始めの長さ（点線の位相を元の始点からの距離で保つ）。
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScreenRun {
    a: Pos2,
    b: Pos2,
    skipped: f32,
}

/// 線分 a→b を矩形で切って、残る区間の媒介変数 (t0, t1)（0 が a、1 が b）を返す（交わらなければ None）。
fn clip_segment(a: Pos2, b: Pos2, rect: Rect) -> Option<(f32, f32)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [
        (-d.x, a.x - rect.left()),
        (d.x, rect.right() - a.x),
        (-d.y, a.y - rect.top()),
        (d.y, rect.bottom() - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return None;
                }
                t0 = t0.max(r);
            } else {
                if r < t0 {
                    return None;
                }
                t1 = t1.min(r);
            }
        }
    }
    (t1 > t0).then_some((t0, t1))
}

/// 縁の線分を画面の矩形で切って残す（見える部分だけ。拡大しても点線の図形の数が画面の大きさで頭打ちになる）。
/// `limit` を超えた時点でやめる（超えたかを知れればよい）。
fn screen_runs(view: &CanvasView, clip: Rect, runs: &[Run], limit: usize) -> Vec<ScreenRun> {
    let clip = clip.expand(CLIP_MARGIN);
    let mut out = Vec::new();
    for r in runs {
        let (x0, y0, x1, y1) = r.ends();
        let (a, b) = (view.to_screen(x0, y0), view.to_screen(x1, y1));
        let Some((t0, t1)) = clip_segment(a, b, clip) else {
            continue;
        };
        let d = b - a;
        out.push(ScreenRun {
            a: a + d * t0,
            b: a + d * t1,
            skipped: d.length() * t0,
        });
        if out.len() > limit {
            break;
        }
    }
    out
}

/// 縁を描く図形（白の線の上に黒の点線）と、どう描いたか。
struct Ants {
    shapes: Vec<Shape>,
    /// 点線にしたか（多いときは動かさない実線）。
    dashed: bool,
    /// 1 画面の上限で、見える線分の一部を描かなかったか。
    cut: bool,
}

/// 切った線分の点線の始まりの位置（egui は「始点から `offset` 先で最初の黒が始まる」）。切り落とした分だけ進んだ位相にする。
/// 切った始点は画面の外（余白の外）なので、負の位置から始まる黒（4 点以内）が画面にかかることはない。
fn dash_offset(phase: f32, skipped: f32) -> f32 {
    if skipped <= 0.0 {
        return phase;
    }
    let o = (phase - skipped).rem_euclid(DASH_PERIOD);
    if o > DASH_LENGTH {
        o - DASH_PERIOD
    } else {
        o
    }
}

/// 縁の線分から、画面に描く図形を作る。見える線分が `MAX_DASHED_RUNS` を超えたら実線、`MAX_DRAWN_RUNS` を超えたら先頭から描く。
fn ants_shapes(view: &CanvasView, clip: Rect, runs: &[Run], phase: f32) -> Ants {
    let mut visible = screen_runs(view, clip, runs, MAX_DRAWN_RUNS);
    let cut = visible.len() > MAX_DRAWN_RUNS;
    visible.truncate(MAX_DRAWN_RUNS);
    let dashed = visible.len() <= MAX_DASHED_RUNS;
    let thin_white = Stroke::new(1.5, Color32::from_white_alpha(235));
    let thin_black = Stroke::new(1.5, Color32::from_black_alpha(235));
    let mut shapes = Vec::with_capacity(visible.len() * 2);
    for r in &visible {
        shapes.push(Shape::line_segment([r.a, r.b], thin_white));
    }
    for r in &visible {
        if dashed {
            Shape::dashed_line_many_with_offset(
                &[r.a, r.b],
                thin_black,
                &[DASH_LENGTH],
                &[DASH_PERIOD - DASH_LENGTH],
                dash_offset(phase, r.skipped),
                &mut shapes,
            );
        } else {
            // 多いときは白の線の上に細い黒の線を重ねるだけ（点線の分割と、流す描き直しを省く）
            shapes.push(Shape::line_segment(
                [r.a, r.b],
                Stroke::new(0.8, Color32::from_black_alpha(235)),
            ));
        }
    }
    Ants {
        shapes,
        dashed,
        cut,
    }
}

/// 縁を一部しか描いていない理由（キャンバスの隅に出す短い状態。使い方の説明ではない）。
fn partial_edge_note(lang: Lang) -> &'static str {
    lang.pick(
        "縁が多いため一部だけ表示",
        "Edge partly shown: too many segments",
    )
}

/// 縁が一部だけの印を、キャンバスの左下に出す。
fn paint_partial_edge_note(painter: &Painter, lang: Lang) {
    let text = partial_edge_note(lang);
    let clip = painter.clip_rect();
    let width = w::text_width(painter, text, t::LABEL_DIM) + 16.0;
    let r = Rect::from_min_size(
        egui::pos2(clip.left() + 8.0, clip.bottom() - 28.0),
        egui::vec2(width.min((clip.width() - 16.0).max(0.0)), 20.0),
    );
    w::rounded(painter, r, t::CONTROL_BG, 3.0);
    w::text(
        painter,
        r.shrink2(egui::vec2(8.0, 0.0)),
        text,
        t::LABEL_DIM.with_color(t::WARNING),
        w::Align::Left,
    );
}

/// 選択範囲の縁（白の線の上を黒の点線が流れる）。
fn paint_ants(ctx: &egui::Context, painter: &Painter, view: &CanvasView, app: &mut AppState) {
    // クイックマスクでは赤い重ねが選択範囲（縁の点線は出さない）
    if app.sel.quick {
        app.sel.edge_partial = false;
        return;
    }
    let Some(mask) = app.doc.selection().cloned() else {
        app.sel.edge_partial = false;
        return;
    };
    let animate = app.sel.animate;
    let lang = app.lang;
    let time = ctx.input(|i| i.time);
    let phase = if animate {
        ((time * 24.0) % DASH_PERIOD as f64) as f32
    } else {
        0.0
    };
    let (runs, truncated) = app.sel.outline_of(&mask);
    let ants = ants_shapes(view, painter.clip_rect(), runs, phase);
    painter.extend(ants.shapes);
    let partial = truncated || ants.cut;
    if partial {
        paint_partial_edge_note(painter, lang);
    }
    app.sel.edge_partial = partial;
    if animate && ants.dashed {
        ctx.request_repaint_after(ANTS_FRAME);
    }
}

/// ドラッグ中の形（長方形・楕円・なげなわ）の輪郭を描く。2D のキャンバスと 3D ビューで同じ見た目。`a`・`b` は向かい合う角、`lasso` はなげなわの点、
/// `corner_radius` は長方形の角の半径、`screen` は形の座標を画面の点へ写す。
pub(crate) fn paint_shape_outline(
    painter: &Painter,
    tool: Tool,
    (a, b): ((f64, f64), (f64, f64)),
    lasso: &[(f64, f64)],
    corner_radius: f64,
    screen: &dyn Fn((f64, f64)) -> Pos2,
) {
    match tool {
        Tool::SelectRect if corner_radius > 0.0 => {
            let points: Vec<Pos2> = shape::rounded_rect_outline(
                a.0.min(b.0),
                a.1.min(b.1),
                a.0.max(b.0),
                a.1.max(b.1),
                corner_radius,
            )
            .iter()
            .map(|p| screen((p.x, p.y)))
            .collect();
            path(painter, &points, true);
        }
        Tool::SelectRect => {
            let corners = [(a.0, a.1), (b.0, a.1), (b.0, b.1), (a.0, b.1)].map(screen);
            path(painter, &corners, true);
        }
        Tool::SelectEllipse => {
            let c = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
            let r = ((b.0 - a.0).abs() / 2.0, (b.1 - a.1).abs() / 2.0);
            let points: Vec<Pos2> = (0..64)
                .map(|i| {
                    let t = i as f64 / 64.0 * std::f64::consts::TAU;
                    screen((c.0 + t.cos() * r.0, c.1 + t.sin() * r.1))
                })
                .collect();
            path(painter, &points, true);
        }
        Tool::Lasso => {
            let points: Vec<Pos2> = lasso.iter().map(|p| screen(*p)).collect();
            path(painter, &points, false);
            if let (Some(first), Some(last)) = (points.first(), points.last()) {
                painter.line_segment(
                    [*last, *first],
                    Stroke::new(1.0, Color32::from_white_alpha(120)),
                );
            }
        }
        _ => {}
    }
}

/// 多角形の途中を描く（打った点 `placed` を結び、ポインタ `hover` までのゴムの線と、点の印。3 点以上で始めの点にポインタが届けば、閉じる印を大きく）。
/// 2D のキャンバスと 3D ビューで同じ見た目。
pub(crate) fn paint_polygon_draft(painter: &Painter, placed: &[Pos2], hover: Option<Pos2>) {
    let Some(&first) = placed.first() else {
        return;
    };
    let mut points = placed.to_vec();
    if let Some(h) = hover {
        points.push(h);
    }
    path(painter, &points, false);
    let closable = placed.len() >= 3
        && points
            .last()
            .is_some_and(|p| p.distance(first) <= CLOSE_RADIUS);
    for (i, p) in points.iter().take(placed.len()).enumerate() {
        let size = if i == 0 && closable { 6.0 } else { 3.5 };
        let r = Rect::from_center_size(*p, egui::vec2(size * 2.0, size * 2.0));
        painter.rect_filled(r, 1.0, Color32::from_black_alpha(160));
        painter.rect_filled(r.shrink(1.2), 1.0, Color32::WHITE);
    }
}

/// ドラッグ中の形と多角形の途中。
fn paint_drafts(painter: &Painter, view: &CanvasView, app: &AppState, modifiers: Modifiers) {
    let screen = |p: (f64, f64)| view.to_screen(p.0, p.1);
    if let Some(d) = &app.sel.drag {
        if d.moved < CLICK_RADIUS || d.tool == Tool::SelectPen {
            return;
        }
        let c = shape::Constraint::of(&app.sel, d.tool, app.sel.press_modifiers, modifiers);
        let (a, b) = shape::drag_corners(d.start, d.current, c.square, c.center);
        // 角を丸めた長方形は、選択範囲と同じく画素の座標へ丸めた角から
        let (a, b) = if d.tool == Tool::SelectRect && app.sel.corner_radius > 0 {
            ((a.0.round(), a.1.round()), (b.0.round(), b.1.round()))
        } else {
            (a, b)
        };
        paint_shape_outline(
            painter,
            d.tool,
            (a, b),
            &d.lasso,
            app.sel.corner_radius as f64,
            &screen,
        );
    }
    if app.tool == Tool::Polygon && !app.sel.polygon.is_empty() {
        let placed: Vec<Pos2> = app.sel.polygon.iter().map(|p| screen(*p)).collect();
        paint_polygon_draft(painter, &placed, app.sel.polygon_hover.map(screen));
    }
}

/// 軸と映した側のカーソルに使う対称（描いている間はそのストロークに固めた対称）。クイックマスクと選択ペンのストロークは対称を使わない
/// （選択範囲は 1 か所しか変わらない）ので、写しのカーソルも軸も出さない。クイックマスクのストロークは `begin_canvas_stroke` を通らず、
/// `stroke_symmetry` は前の通常のストロークのものが残るので、ここで先に断つ。
pub fn active_symmetry(app: &AppState) -> Option<CanvasSymmetry> {
    if app.sel.quick || app.tool == Tool::SelectPen {
        return None;
    }
    let s = if app.is_stroking() {
        app.sel.stroke_symmetry?
    } else {
        app.canvas_symmetry()
    };
    s.enabled().then_some(s)
}

fn axis_color() -> Color32 {
    Color32::from_rgba_unmultiplied(115, 209, 255, 204)
}

/// キャンバスの上に、選択の縁・ドラッグ中の形を描く（合成の絵の上、ブラシのカーソルの下）。対称の線は定規の描画（`rulers::draw`）。
pub fn paint_overlay(
    ctx: &egui::Context,
    painter: &Painter,
    view: &CanvasView,
    app: &mut AppState,
) {
    if app.sel.quick {
        quick::paint(painter, view, app);
    } else {
        app.sel.quick_overlay.clear();
    }
    paint_ants(ctx, painter, view, app);
    paint_pen(ctx, painter, view, app);
    paint_drafts(painter, view, app, ctx.input(|i| i.modifiers));
}

/// 選択ペンのツール: 動いているストロークの被覆（足す・消すで色を変える）と、ブラシの直径の輪のカーソル。
fn paint_pen(ctx: &egui::Context, painter: &Painter, view: &CanvasView, app: &mut AppState) {
    let drawing = app.sel.pen.as_ref().is_some_and(|a| !a.quick);
    if drawing {
        pen::sync(app);
    }
    let sel = &mut app.sel;
    // 変わったタイルは、ここで受け取って空にする（3D ビューも `sync` を呼ぶので、先に呼んだほうの分も溜まっている）
    let changed = sel
        .pen
        .as_mut()
        .filter(|a| !a.quick)
        .and_then(|a| a.stroke.take_synced());
    match sel.pen.as_ref().filter(|a| !a.quick) {
        Some(active) => {
            let tint = if active.stroke.erase {
                PEN_ERASE_TINT
            } else {
                PEN_ADD_TINT
            };
            sel.pen_overlay.paint(
                painter,
                view,
                active.stroke.cover(),
                tint,
                changed.as_deref(),
                "select-pen",
            );
        }
        None => sel.pen_overlay.clear(),
    }
    if app.tool != Tool::SelectPen {
        return;
    }
    let Some(at) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    if !painter.clip_rect().contains(at) || ctx.layer_id_at(at) != Some(painter.layer_id()) {
        return;
    }
    let radius = (app.brush.radius * view.pixel_size()).max(1.5);
    painter.circle_stroke(at, radius, Stroke::new(3.0, Color32::from_black_alpha(140)));
    painter.circle_stroke(at, radius, Stroke::new(1.2, Color32::from_white_alpha(230)));
}

/// ブラシのカーソルを、対称の写しの所にも描く（水色の円。radius は画面の点）。2D の対称の写しと、3D の対称（`model`。今のモデルの
/// 面を写した先の UV）の写し、両方なら 3D の写しのそれぞれの 2D の写し。3D の写しの円は、写した先のテクスチャの細かさに合わせた大きさ。
pub fn paint_mirrored_cursors(
    painter: &Painter,
    view: &CanvasView,
    app: &AppState,
    model: Option<&yolu_core::geometry::ModelSymmetry>,
    at: Pos2,
    radius: f32,
) {
    if app.sel.quick || app.tool == Tool::SelectPen {
        return;
    }
    let (x, y) = view.to_canvas(at);
    let symmetry = active_symmetry(app);
    let canvas = symmetry
        .and_then(|s| s.transforms().ok())
        .unwrap_or_default();
    let copies = model
        .and_then(|m| {
            m.copies(
                x,
                y,
                app.brush.radius as f64,
                app.doc.width(),
                app.doc.height(),
            )
            .ok()
        })
        .map(|c| c.copies)
        .unwrap_or_default();
    let mut points: Vec<((f64, f64), f32)> = symmetry
        .map(|s| mirrored_points(&s, x, y))
        .unwrap_or_default()
        .into_iter()
        .map(|p| (p, radius))
        .collect();
    for c in &copies {
        let p = c.map(x, y);
        let scale = (c.m[0] * c.m[3] - c.m[1] * c.m[2]).abs().sqrt() as f32;
        points.push((p, radius * scale));
        for t in canvas.iter().skip(1) {
            points.push((t.map(p.0, p.1), radius * scale));
        }
    }
    for ((mx, my), r) in points {
        let p = view.to_screen(mx, my);
        painter.circle_stroke(p, r, Stroke::new(3.0, Color32::from_black_alpha(100)));
        painter.circle_stroke(p, r, Stroke::new(1.2, axis_color()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2, Vec2};

    fn view_for(canvas: u32, viewport: Rect, zoom: f32, angle: f32) -> CanvasView {
        CanvasView::new(viewport, canvas, canvas, zoom, Vec2::ZERO, angle, false)
    }

    fn hrun(fixed: u32, from: u32, to: u32) -> Run {
        Run {
            horizontal: true,
            fixed,
            from,
            to,
        }
    }

    fn is_black_dash(stroke: &Stroke) -> bool {
        stroke.width == 1.5 && stroke.color == Color32::from_black_alpha(235)
    }

    fn black_dashes(shapes: &[Shape]) -> usize {
        shapes
            .iter()
            .filter(|s| matches!(s, Shape::LineSegment { stroke, .. } if is_black_dash(stroke)))
            .count()
    }

    /// 黒の点線の区間（a から dir の向きの距離。lo..hi に切る）。
    fn dash_spans(shapes: &[Shape], a: Pos2, dir: Vec2, lo: f32, hi: f32) -> Vec<(f32, f32)> {
        let mut spans: Vec<(f32, f32)> = shapes
            .iter()
            .filter_map(|s| match s {
                Shape::LineSegment { points, stroke } if is_black_dash(stroke) => {
                    let (s0, s1) = ((points[0] - a).dot(dir), (points[1] - a).dot(dir));
                    let (s0, s1) = (s0.min(s1).max(lo), s0.max(s1).min(hi));
                    (s1 - s0 > 1e-3).then_some((s0, s1))
                }
                _ => None,
            })
            .collect();
        spans.sort_by(|x, y| x.0.total_cmp(&y.0));
        spans
    }

    #[test]
    fn a_segment_is_clipped_to_the_screen_and_keeps_how_much_was_cut_off() {
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0));
        // 横切る線: 両端が外
        let (t0, t1) = clip_segment(pos2(-100.0, 50.0), pos2(300.0, 50.0), rect).unwrap();
        assert!(
            (t0 - 0.25).abs() < 1e-6 && (t1 - 0.5).abs() < 1e-6,
            "{t0} {t1}"
        );
        // 全部中・全部外・矩形に触れるだけ・斜め
        assert_eq!(
            clip_segment(pos2(10.0, 10.0), pos2(90.0, 90.0), rect),
            Some((0.0, 1.0))
        );
        assert!(clip_segment(pos2(-10.0, 10.0), pos2(-10.0, 90.0), rect).is_none());
        assert!(clip_segment(pos2(100.0, -50.0), pos2(100.0, -10.0), rect).is_none());
        assert!(clip_segment(pos2(150.0, 0.0), pos2(250.0, 100.0), rect).is_none());
        let (t0, t1) = clip_segment(pos2(-50.0, -50.0), pos2(150.0, 150.0), rect).unwrap();
        assert!((t0 - 0.25).abs() < 1e-6 && (t1 - 0.75).abs() < 1e-6);
    }

    #[test]
    fn at_the_maximum_zoom_on_a_large_canvas_the_dash_count_follows_the_screen_not_the_edge_length()
    {
        // 8192 × 8192 を最大に拡大（1 画素 16 点）: 辺 1 本は 131072 点。切らなければ 1 本が約 16000 個の点線になる
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(8192.0, 8192.0));
        let view = view_for(8192, viewport, 16.0, 0.0);
        let screen = Rect::from_center_size(viewport.center(), vec2(1200.0, 800.0));
        let runs = [
            hrun(4096, 0, 8192),
            Run {
                horizontal: false,
                fixed: 4096,
                from: 0,
                to: 8192,
            },
            hrun(0, 0, 8192),
            hrun(8192, 0, 8192),
        ];
        let ants = ants_shapes(&view, screen, &runs, 3.0);
        assert!(ants.dashed && !ants.cut);
        // 横 1216 + 縦 816 点（余白つき）を 8 点ごと。画面に交わらない辺は作らない
        let dashes = black_dashes(&ants.shapes);
        assert!((240..=270).contains(&dashes), "{dashes}");
        assert_eq!(ants.shapes.len(), 2 + dashes, "白の線 2 本 + 黒の点線");
    }

    #[test]
    fn clipped_dashes_keep_the_phase_measured_from_the_original_start() {
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(512.0, 512.0));
        for angle in [0.0, 30.0, -75.0] {
            let view = view_for(512, viewport, 1.0, angle);
            let run = hrun(256, 0, 512);
            let (x0, y0, x1, y1) = run.ends();
            let (a, b) = (view.to_screen(x0, y0), view.to_screen(x1, y1));
            let dir = (b - a).normalized();
            // 回しても中心を通る線なので、中心のまわりの小さな矩形で両端を切る
            let screen = Rect::from_center_size(viewport.center(), vec2(160.0, 160.0));
            let (t0, t1) = clip_segment(a, b, screen).unwrap();
            let (lo, hi) = ((b - a).length() * t0, (b - a).length() * t1);
            assert!(lo > 100.0, "両端が切れていること: {lo}");
            for phase in [0.0f32, 1.5, 3.9, 4.0, 6.5, 7.9] {
                let ants = ants_shapes(&view, screen, &[run], phase);
                let mut whole = Vec::new();
                Shape::dashed_line_many_with_offset(
                    &[a, b],
                    Stroke::new(1.5, Color32::from_black_alpha(235)),
                    &[DASH_LENGTH],
                    &[DASH_PERIOD - DASH_LENGTH],
                    phase,
                    &mut whole,
                );
                let want = dash_spans(&whole, a, dir, lo, hi);
                let got = dash_spans(&ants.shapes, a, dir, lo, hi);
                assert_eq!(want.len(), got.len(), "angle {angle} phase {phase}");
                for (w, g) in want.iter().zip(&got) {
                    assert!(
                        (w.0 - g.0).abs() < 0.05 && (w.1 - g.1).abs() < 0.05,
                        "angle {angle} phase {phase}: {w:?} {g:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn many_visible_runs_switch_to_solid_and_past_the_drawn_limit_only_the_first_are_drawn() {
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(512.0, 512.0));
        let view = view_for(512, viewport, 1.0, 0.0);
        let runs = |n: usize| -> Vec<Run> {
            (0..n)
                .map(|i| {
                    hrun(
                        (i / 256) as u32,
                        (i % 256) as u32 * 2,
                        (i % 256) as u32 * 2 + 1,
                    )
                })
                .collect()
        };
        // 点線の上限ちょうど: 点線
        let ants = ants_shapes(&view, viewport, &runs(MAX_DASHED_RUNS), 0.0);
        assert!(ants.dashed && !ants.cut);
        assert!(black_dashes(&ants.shapes) >= MAX_DASHED_RUNS / 2);
        // 1 本超えたら実線（白と細い黒の 2 本ずつ。流さない）
        let ants = ants_shapes(&view, viewport, &runs(MAX_DASHED_RUNS + 1), 0.0);
        assert!(!ants.dashed && !ants.cut);
        assert_eq!(ants.shapes.len(), 2 * (MAX_DASHED_RUNS + 1));
        // 描く上限ちょうどは全部、1 本超えたら先頭から上限まで
        let ants = ants_shapes(&view, viewport, &runs(MAX_DRAWN_RUNS), 0.0);
        assert!(!ants.dashed && !ants.cut);
        assert_eq!(ants.shapes.len(), 2 * MAX_DRAWN_RUNS);
        let ants = ants_shapes(&view, viewport, &runs(MAX_DRAWN_RUNS + 5000), 0.0);
        assert!(!ants.dashed && ants.cut);
        assert_eq!(ants.shapes.len(), 2 * MAX_DRAWN_RUNS);
        // 先頭の線分が残る（先頭から描く）
        let Shape::LineSegment { points, .. } = &ants.shapes[0] else {
            panic!("線分のはず");
        };
        let (x0, y0, _, _) = runs(1)[0].ends();
        assert_eq!(points[0], view.to_screen(x0, y0));
    }

    #[test]
    fn the_partial_edge_note_has_both_languages_and_says_only_the_reason() {
        let (ja, en) = (partial_edge_note(Lang::Ja), partial_edge_note(Lang::En));
        assert_ne!(ja, en);
        assert!(en.is_ascii(), "{en}");
        assert!(!ja.contains('。') && !en.ends_with('.'), "状態の短い文");
    }
}
