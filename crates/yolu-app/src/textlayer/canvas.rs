//! テキストツールのキャンバス: 押して離した所で打ち始め（選んでいるテキストレイヤーの箱の中なら、その層を打ち直す。箱の中の押しはカーソルを動かす）、
//! 打っている間は箱の枠・カーソル・選んだ範囲を、描いた文字の上に重ねる。打つのは見えない入力欄（egui の文字の欄。かな漢字の変換も
//! 同じ欄が受ける）で、カーソルの位置は core の並べ（`TextLine::carets`）から描く（回した文字・折り返し・禁則でも描いた字に合う）。

use egui::{vec2, Color32, Id, Order, Painter, Pos2, Stroke, Ui};
use yolu_core::text::{TextLayout, TextSettings};

use super::{layout_box, to_document, to_local, TextAction};
use crate::canvas::view::CanvasView;
use crate::state::{Action, AppState, StrokeSource, Tool};
use crate::ui::theme as t;

/// 見えない入力欄の ID。
pub fn input_id() -> Id {
    Id::new("yolu.text.input")
}

/// 押した（離したところで打ち始める）。打っている間に箱の外を押したら、打ち終わる。
pub fn press(app: &mut AppState, view: &CanvasView, at: Pos2, source: StrokeSource) {
    let (x, y) = view.to_canvas(at);
    if let Some(index) = caret_at(app, x, y) {
        // 打っている箱の中: カーソルを動かす（入力欄のフォーカスは次のフレームで戻す）
        if let Some(e) = app.text.editing.as_mut() {
            e.move_cursor = Some(index);
            e.focus_wanted = true;
            e.had_focus = false;
        }
        return;
    }
    app.apply(Action::Text(TextAction::Commit));
    app.text.press = Some(((x, y), source));
}

/// 離した: 選んでいるテキストレイヤーの箱の中なら打ち直し、外なら新しい文字。
pub fn release(app: &mut AppState, source: StrokeSource) {
    let Some(((x, y), from)) = app.text.press.take() else {
        return;
    };
    if from != source {
        return;
    }
    if let Some(id) = app.selected_layer.filter(|id| inside_layer(app, *id, x, y)) {
        app.apply(Action::Text(TextAction::Edit(id)));
        // 押した所へカーソルを置く
        let index = caret_at(app, x, y);
        if let (Some(e), Some(i)) = (app.text.editing.as_mut(), index) {
            e.move_cursor = Some(i);
        }
        return;
    }
    app.apply(Action::Text(TextAction::Begin { x, y }));
}

/// 移動・変形ツールのダブルクリックの間（秒）と、2 回の押しの離れの上限（画面の画素）。
const DOUBLE_CLICK: f64 = crate::pathtool::DOUBLE_CLICK;
const DOUBLE_CLICK_DISTANCE: f32 = 6.0;

/// 移動・変形ツールで押した: 前の押しとダブルクリックになり、テキストレイヤーの箱の中なら、離したときにテキストツールへ替えて
/// そのレイヤーを打ち直す（押した所にカーソル）。そうなら true（移動・変形のドラッグは始めない）。
pub fn move_press(
    app: &mut AppState,
    view: &CanvasView,
    at: Pos2,
    source: StrokeSource,
    now: f64,
) -> bool {
    let last = app.text.move_click.replace((now, at));
    let Some((t, p)) = last else {
        return false;
    };
    if now - t > DOUBLE_CLICK || (p - at).length() > DOUBLE_CLICK_DISTANCE {
        return false;
    }
    let (x, y) = view.to_canvas(at);
    let selected = app.selected_layer.filter(|id| inside_layer(app, *id, x, y));
    let target = selected.or_else(|| {
        let tops: Vec<crate::engine::LayerId> = app
            .doc
            .layers()
            .iter()
            .rev()
            .filter(|l| l.text().is_some() && l.visible())
            .map(|l| l.id())
            .collect();
        tops.into_iter().find(|id| inside_layer(app, *id, x, y))
    });
    let Some(id) = target else {
        return false;
    };
    app.text.move_click = None;
    app.text.move_press = Some((id, (x, y), source));
    true
}

/// 移動・変形ツールで離した: ダブルクリックの押しなら、テキストツールへ替えてそのレイヤーを打ち直す。そうなら true。
pub fn move_release(app: &mut AppState, source: StrokeSource) -> bool {
    let Some((id, (x, y), from)) = app.text.move_press.take() else {
        return false;
    };
    if from != source {
        return true;
    }
    app.apply(Action::SelectTool(Tool::Text));
    if app.tool != Tool::Text {
        return true;
    }
    app.apply(Action::Text(TextAction::Edit(id)));
    let index = caret_at(app, x, y);
    if let (Some(e), Some(i)) = (app.text.editing.as_mut(), index) {
        e.move_cursor = Some(i);
    }
    true
}

/// 押しの途中か。
pub fn dragging(app: &AppState, source: Option<StrokeSource>) -> bool {
    app.text
        .press
        .is_some_and(|(_, s)| source.is_none_or(|w| w == s))
}

/// テキストレイヤーの箱の中か。
fn inside_layer(app: &mut AppState, id: crate::engine::LayerId, x: f64, y: f64) -> bool {
    let Some((value, layout)) = app.text_layer_layout(id) else {
        return false;
    };
    let b = layout_box(&value, &layout);
    let (lx, ly) = to_local(&value, x, y);
    let pad = value.size * 0.2;
    lx >= b[0] - pad && lx <= b[2] + pad && ly >= b[1] - pad && ly <= b[3] + pad
}

/// 打っている箱の中の点のカーソルの位置（文字の番号）。箱の外は None。
fn caret_at(app: &mut AppState, x: f64, y: f64) -> Option<usize> {
    let layout = app.text_layout()?;
    let e = app.text.editing.as_ref()?;
    let value = &e.value;
    let b = layout_box(value, &layout);
    let (lx, ly) = to_local(value, x, y);
    let pad = value.size * 0.2;
    if !(lx >= b[0] - pad && lx <= b[2] + pad && ly >= b[1] - pad && ly <= b[3] + pad) {
        return None;
    }
    let line = layout
        .lines
        .iter()
        .find(|l| ly <= l.top && ly > l.top - layout.line_advance)
        .or_else(|| {
            if ly > 0.0 {
                layout.lines.first()
            } else {
                layout.lines.last()
            }
        });
    let byte = match line {
        Some(l) => l
            .carets
            .iter()
            .min_by(|a, b| (a.1 - lx).abs().total_cmp(&(b.1 - lx).abs()))
            .map_or(l.start, |c| c.0),
        None => 0,
    };
    Some(e.buffer[..byte.min(e.buffer.len())].chars().count())
}

/// 文の中のバイトの位置の、箱の中のカーソルの線（上端・下端。基準の点から）。
fn caret_line(layout: &TextLayout, value: &TextSettings, byte: usize) -> ((f64, f64), (f64, f64)) {
    let lines = &layout.lines;
    let at = lines
        .iter()
        .enumerate()
        .find(|(i, l)| {
            l.start <= byte
                && (byte < l.end
                    || byte == l.end && lines.get(i + 1).is_none_or(|n| n.start != byte))
        })
        .map(|(_, l)| l)
        .or(lines.last());
    match at {
        Some(l) => {
            let x = l
                .carets
                .iter()
                .rev()
                .find(|c| c.0 <= byte)
                .map_or(l.x, |c| c.1);
            ((x, l.top), (x, l.top - layout.line_advance))
        }
        None => {
            let advance = layout.line_advance.max(value.size);
            ((0.0, 0.0), (0.0, -advance))
        }
    }
}

/// 打っている文字の枠・カーソル・選んだ範囲と、見えない入力欄。テキストツールでテキストレイヤーを選んでいるときは、その層の枠。
pub fn paint_overlay(ui: &mut Ui, painter: &Painter, view: &CanvasView, app: &mut AppState) {
    if app.text.editing.is_none() {
        if app.tool == Tool::Text {
            if let Some(id) = app.selected_layer {
                if let Some((value, layout)) = app.text_layer_layout(id) {
                    outline(painter, view, &value, layout_box(&value, &layout), false);
                }
            }
        }
        return;
    }
    let Some(layout) = app.text_layout() else {
        return;
    };
    let e = app.text.editing.as_ref().expect("打っている");
    let value = e.value.clone();
    outline(painter, view, &value, layout_box(&value, &layout), true);
    // 選んだ範囲とカーソル（文字の番号 → 文の中のバイト）
    let byte_of = |chars: usize| {
        e.buffer
            .char_indices()
            .nth(chars)
            .map_or(e.buffer.len(), |(b, _)| b)
    };
    let screen = |p: (f64, f64)| {
        let (x, y) = to_document(&value, p.0, p.1);
        view.to_screen(x, y)
    };
    if let Some((primary, secondary)) = e.cursor {
        let (a, b) = (
            byte_of(primary.min(secondary)),
            byte_of(primary.max(secondary)),
        );
        if a < b {
            for l in &layout.lines {
                let (s, t) = (a.max(l.start), b.min(l.end));
                if s >= t && !(a <= l.start && b > l.end) {
                    continue;
                }
                let x_of = |byte: usize| {
                    l.carets
                        .iter()
                        .rev()
                        .find(|c| c.0 <= byte)
                        .map_or(l.x, |c| c.1)
                };
                let (x0, x1) = (x_of(s), x_of(t).max(x_of(s) + value.size * 0.25));
                let (top, bottom) = (l.top, l.top - layout.line_advance);
                let quad = vec![
                    screen((x0, top)),
                    screen((x1, top)),
                    screen((x1, bottom)),
                    screen((x0, bottom)),
                ];
                painter.add(egui::Shape::convex_polygon(
                    quad,
                    t::ACCENT.gamma_multiply(0.35),
                    Stroke::NONE,
                ));
            }
        }
        let (top, bottom) = caret_line(&layout, &value, byte_of(primary));
        let (p, q) = (screen(top), screen(bottom));
        painter.line_segment([p, q], Stroke::new(3.0, Color32::from_black_alpha(160)));
        painter.line_segment([p, q], Stroke::new(1.5, Color32::WHITE));
    }
    input(ui, app, view, &layout);
}

/// 箱の枠（打っている間は実線、選んでいるだけなら薄く）。
fn outline(painter: &Painter, view: &CanvasView, value: &TextSettings, b: [f64; 4], editing: bool) {
    let corners = [(b[0], b[3]), (b[2], b[3]), (b[2], b[1]), (b[0], b[1])].map(|(x, y)| {
        let (x, y) = to_document(value, x, y);
        view.to_screen(x, y)
    });
    let color = if editing {
        t::ACCENT
    } else {
        Color32::from_white_alpha(120)
    };
    for i in 0..4 {
        let (p, q) = (corners[i], corners[(i + 1) % 4]);
        painter.line_segment([p, q], Stroke::new(2.5, Color32::from_black_alpha(120)));
        painter.line_segment([p, q], Stroke::new(1.0, color));
    }
}

/// 見えない入力欄（カーソルの所に置く。かな漢字の変換の窓もそこに出る）。文が変わったら描き直し、フォーカスを失ったら打ち終わる。
fn input(ui: &mut Ui, app: &mut AppState, view: &CanvasView, layout: &TextLayout) {
    let id = input_id();
    let ctx = ui.ctx().clone();
    let Some(e) = app.text.editing.as_ref() else {
        return;
    };
    let value = e.value.clone();
    let byte = e
        .cursor
        .map(|(p, _)| {
            e.buffer
                .char_indices()
                .nth(p)
                .map_or(e.buffer.len(), |(b, _)| b)
        })
        .unwrap_or(e.buffer.len());
    let ((cx, cy), _) = caret_line(layout, &value, byte);
    let (dx, dy) = to_document(&value, cx, cy);
    let at = view.to_screen(dx, dy);
    // カーソルを動かす頼み（箱の中を押した・打ち直し始め）
    if let Some(index) = app.text.editing.as_mut().and_then(|e| e.move_cursor.take()) {
        let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(index),
            )));
        state.store(&ctx, id);
    }
    let mut buffer = app
        .text
        .editing
        .as_ref()
        .expect("打っている")
        .buffer
        .clone();
    let mut cursor = None;
    let mut has_focus = false;
    egui::Area::new(id.with("area"))
        .order(Order::Foreground)
        .fixed_pos(at)
        .constrain(false)
        .show(&ctx, |ui| {
            let visuals = ui.visuals_mut();
            visuals.text_cursor.stroke = Stroke::NONE;
            visuals.selection.bg_fill = Color32::TRANSPARENT;
            visuals.selection.stroke = Stroke::NONE;
            ui.set_max_size(vec2(2.0, 2.0));
            let out = egui::TextEdit::multiline(&mut buffer)
                .id(id)
                .frame(egui::Frame::NONE)
                .text_color(Color32::TRANSPARENT)
                .desired_width(2.0)
                .desired_rows(1)
                .clip_text(true)
                .lock_focus(true)
                .show(ui);
            cursor = out
                .cursor_range
                .map(|r| (r.primary.index.0, r.secondary.index.0));
            has_focus = out.response.has_focus();
        });
    let pressed = ctx.input(|i| i.pointer.any_pressed() || i.pointer.any_released());
    let Some(e) = app.text.editing.as_mut() else {
        return;
    };
    e.cursor = cursor.or(e.cursor);
    if e.focus_wanted && !pressed {
        ctx.memory_mut(|m| m.request_focus(id));
        e.focus_wanted = false;
    } else if has_focus {
        e.had_focus = true;
    } else if e.had_focus && !e.focus_wanted {
        // Esc・ほかの所を押した: 打ち終わる
        app.apply(Action::Text(TextAction::Commit));
        return;
    }
    if buffer != e.buffer {
        app.text_typed(buffer);
    }
}
