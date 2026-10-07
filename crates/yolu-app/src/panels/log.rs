//! ログのパネル（ドックのタブ「ログ」）: 起動してからの注意と失敗の知らせ（`notice::NoticeLog`）を、古い物を上・新しい物を下に並べる。
//! 1 行は 時刻（時:分:秒）・種類の印・出どころ・文。長い文は 1 行に切り、全文はツールチップ。同じ知らせが続いたら 1 行にまとめ、
//! 右端に回数（×3）を出す。済んだ知らせと断りは入らない（処理の数・外からの操作の記録も入らない）。
//! 上の帯: エラーだけに絞る・選んだ行を写す・全部を写す・消す（画面の一覧だけ。診断の記録のファイルは消さない）。
//! 行は押して選ぶ（Ctrl で足し引き、Shift で範囲）。一覧を押したあとは Ctrl+C で選んだ行（選んでいなければ全部）を写す。

use std::collections::BTreeSet;

use egui::{pos2, vec2, Id, Rect, Sense, Ui, WidgetInfo, WidgetType};

use crate::lang::Lang;
use crate::notice::{clock_text, Entry, Kind};
use crate::state::AppState;
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};

pub const ROW_HEIGHT: f32 = 20.0;
pub const TOOLBAR_HEIGHT: f32 = 30.0;
const TIME_WIDTH: f32 = 58.0;
const MARK_WIDTH: f32 = 20.0;
const SOURCE_WIDTH: f32 = 92.0;
const PAD: f32 = 6.0;

/// ログのパネルの画面の状態（保存しない）。
#[derive(Clone, Debug)]
pub struct LogView {
    /// エラーだけに絞る。
    pub errors_only: bool,
    /// 選んだ行（`Entry::id`）。
    pub selected: BTreeSet<u64>,
    /// Shift で範囲を選ぶ起点の行。
    anchor: Option<u64>,
    /// 一覧のずらし量。
    scroll: f32,
    /// 一番下を見ている（新しい行が来たら一番下へ送る）。
    pinned: bool,
    /// 前のフレームで見えていた行の数（増えたら一番下へ送るかを決める）。
    seen: usize,
}

impl Default for LogView {
    fn default() -> LogView {
        LogView {
            errors_only: false,
            selected: BTreeSet::new(),
            anchor: None,
            scroll: 0.0,
            pinned: true,
            seen: 0,
        }
    }
}

/// 一覧に出す行（絞りを当てた後。古い順）。
pub fn visible_entries(app: &AppState) -> Vec<&Entry> {
    let errors_only = app.log_view.errors_only;
    app.notice_log
        .entries()
        .filter(|e| !errors_only || e.notice.kind == Kind::Error)
        .collect()
}

/// 写す 1 行（時刻・種類・出どころ・文。まとめた行は回数を添える）。
pub fn row_text(lang: Lang, entry: &Entry) -> String {
    let n = &entry.notice;
    let mut line = format!(
        "{} {} {}: {}",
        clock_text(n.at),
        n.kind.name(lang),
        n.source.name(lang),
        n.text.replace('\n', " ")
    );
    if entry.count > 1 {
        line.push_str(&format!(" (×{})", entry.count));
    }
    line
}

/// 写す文: 選んだ行（`selected_only` で、選んだ行が無ければ空）か、見えている行の全部。
pub fn copy_text(app: &AppState, selected_only: bool) -> String {
    let lang = app.lang;
    visible_entries(app)
        .into_iter()
        .filter(|e| !selected_only || app.log_view.selected.contains(&e.id))
        .map(|e| row_text(lang, e))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 画面の一覧を消す（診断の記録のファイルは消さない）。
pub fn clear(app: &mut AppState) {
    app.notice_log.clear();
    app.log_view.selected.clear();
    app.log_view.anchor = None;
    app.log_view.scroll = 0.0;
}

/// エラーだけに絞る・戻す（見えなくなった行の選びは外す）。
pub fn set_errors_only(app: &mut AppState, on: bool) {
    app.log_view.errors_only = on;
    if on {
        let errors: BTreeSet<u64> = app
            .notice_log
            .entries()
            .filter(|e| e.notice.kind == Kind::Error)
            .map(|e| e.id)
            .collect();
        app.log_view.selected.retain(|id| errors.contains(id));
    }
}

/// 行を押した: 押しただけなら 1 行を選び、Ctrl（command）なら足し引き、Shift なら起点からの範囲。
pub fn click_row(app: &mut AppState, id: u64, modifiers: egui::Modifiers) {
    let view = &mut app.log_view;
    if modifiers.shift {
        if let Some(anchor) = view.anchor {
            let ids: Vec<u64> = {
                let errors_only = view.errors_only;
                app.notice_log
                    .entries()
                    .filter(|e| !errors_only || e.notice.kind == Kind::Error)
                    .map(|e| e.id)
                    .collect()
            };
            let view = &mut app.log_view;
            let (a, b) = (
                ids.iter().position(|i| *i == anchor),
                ids.iter().position(|i| *i == id),
            );
            if let (Some(a), Some(b)) = (a, b) {
                if !modifiers.command {
                    view.selected.clear();
                }
                view.selected.extend(&ids[a.min(b)..=a.max(b)]);
                return;
            }
        }
    }
    let view = &mut app.log_view;
    if modifiers.command {
        if !view.selected.remove(&id) {
            view.selected.insert(id);
        }
    } else {
        view.selected.clear();
        view.selected.insert(id);
    }
    view.anchor = Some(id);
}

/// 種類の印（アイコンの名前と色）。
pub fn mark(kind: Kind) -> (&'static str, egui::Color32) {
    match kind {
        Kind::Error => ("error_circle", t::ERROR),
        Kind::Warning => ("warning", t::WARNING),
        Kind::Refusal => ("error_circle", t::ERROR),
        Kind::Info => ("info", t::ACCENT),
    }
}

pub fn show(ui: &mut Ui, app: &mut AppState) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let lang = app.lang;
    let list_id = Id::new("log.list");

    // 上の帯: エラーだけ・写す・全部を写す・消す
    let bar = Rect::from_min_size(r.min, vec2(r.width(), TOOLBAR_HEIGHT));
    w::fill(ui.painter(), bar, t::PANEL_HEADER);
    let button = |x: f32| Rect::from_min_size(pos2(x, bar.top() + 3.0), vec2(26.0, 24.0));
    let has_rows = !app.notice_log.is_empty();
    if w::icon_button(
        ui,
        button(bar.left() + 4.0),
        "log.errors_only",
        "error_circle",
        lang.pick("エラーだけ", "Errors Only"),
        app.log_view.errors_only,
        true,
        17.0,
    )
    .clicked()
    {
        let on = !app.log_view.errors_only;
        set_errors_only(app, on);
    }
    let mut copy = None;
    let right = bar.right() - 4.0;
    if w::icon_button(
        ui,
        button(right - 26.0 * 3.0 - 4.0),
        "log.copy",
        "content_copy",
        lang.pick("選んだ行を写す", "Copy Selected"),
        false,
        !app.log_view.selected.is_empty(),
        16.0,
    )
    .clicked()
    {
        copy = Some(copy_text(app, true));
    }
    if w::icon_button(
        ui,
        button(right - 26.0 * 2.0 - 2.0),
        "log.copy_all",
        "document_copy",
        lang.pick("全部を写す", "Copy All"),
        false,
        has_rows,
        16.0,
    )
    .clicked()
    {
        copy = Some(copy_text(app, false));
    }
    if w::icon_button(
        ui,
        button(right - 26.0),
        "log.clear",
        "delete",
        lang.pick("ログを消す", "Clear Log"),
        false,
        has_rows,
        17.0,
    )
    .clicked()
    {
        clear(app);
    }

    // 一覧
    let body = Rect::from_min_max(pos2(r.left(), bar.bottom()), r.max);
    w::fill(&ui.painter_at(body), body, t::PANEL_BG);
    let rows: Vec<Entry> = visible_entries(app).into_iter().cloned().collect();
    let content = rows.len() as f32 * ROW_HEIGHT;
    // 新しい行が来たとき、一番下を見ていたなら一番下へ送る
    if rows.len() != app.log_view.seen {
        if app.log_view.pinned {
            app.log_view.scroll = f32::MAX;
        }
        app.log_view.seen = rows.len();
    }
    let mut scroll = app.log_view.scroll;
    let bars = Scroll::begin(ui, body, content, &mut scroll);
    app.log_view.scroll = scroll;
    // 一覧の何も無い所も押せる（キーボードの Ctrl+C を受けるための選び）
    let list = ui.interact(body, list_id, Sense::click());
    if list.clicked() {
        list.request_focus();
    }
    let painter = ui.painter_at(body);
    let width = body.width() - bars.reserved();
    let mut clicked = None;
    for (i, entry) in rows.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(
                body.left(),
                body.top() + i as f32 * ROW_HEIGHT - app.log_view.scroll,
            ),
            vec2(width, ROW_HEIGHT),
        );
        if row.bottom() < body.top() || row.top() > body.bottom() {
            continue;
        }
        let selected = app.log_view.selected.contains(&entry.id);
        let full = row_text(lang, entry);
        let response = ui
            .interact(
                row.intersect(body),
                Id::new(("log.row", entry.id)),
                Sense::click(),
            )
            .on_hover_text(full.clone());
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, full.clone())
        });
        if response.clicked() {
            clicked = Some((entry.id, ui.input(|i| i.modifiers)));
            response.request_focus();
        }
        if selected {
            w::fill(&painter, row, t::ACCENT_SOFT);
        } else if response.hovered() {
            w::fill(&painter, row, t::CONTROL_HOVER);
        }
        draw_row(&painter, row, lang, entry);
    }
    if let Some((id, modifiers)) = clicked {
        click_row(app, id, modifiers);
    }
    let mut scroll = app.log_view.scroll;
    bars.end(ui, "log.scroll", &mut scroll);
    app.log_view.scroll = scroll;
    app.log_view.pinned = scroll >= bars.max - 0.5;

    // 一覧（か、その行）を押したあとは Ctrl+C で写す（選んでいなければ全部）。ほかの所の Ctrl+C（画素の写し）は取らない
    let focused = ui.ctx().memory(|m| {
        m.has_focus(list_id) || rows.iter().any(|e| m.has_focus(Id::new(("log.row", e.id))))
    });
    if focused {
        // キーは表（`keymap::CLIPBOARD_KEYS` の写す）のもの。egui-winit は Ctrl+C を `Event::Copy` に置き換えて渡す
        let (modifiers, key) = crate::keymap::CLIPBOARD_KEYS
            .iter()
            .find(|(_, _, action)| *action == crate::clipboard::ClipAction::Copy)
            .map(|(m, k, _)| (*m, *k))
            .expect("写すキー");
        let pressed = ui.input_mut(|i| {
            let event = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
            i.events.retain(|e| !matches!(e, egui::Event::Copy));
            event || i.consume_key(modifiers, key)
        });
        if pressed && !rows.is_empty() {
            copy = Some(copy_text(app, !app.log_view.selected.is_empty()));
        }
    }
    if let Some(text) = copy.filter(|t| !t.is_empty()) {
        ui.ctx().copy_text(text);
    }
}

/// 1 行を描く: 時刻・種類の印・出どころ・文（切る）・回数。
fn draw_row(p: &egui::Painter, row: Rect, lang: Lang, entry: &Entry) {
    let n = &entry.notice;
    let mut x = row.left() + PAD;
    let cell = |x: f32, width: f32| {
        Rect::from_min_size(pos2(x, row.top()), vec2(width.max(0.0), row.height()))
    };
    w::text(
        p,
        cell(x, TIME_WIDTH),
        &clock_text(n.at),
        t::LABEL_DIM,
        Align::Left,
    );
    x += TIME_WIDTH;
    let (icon, color) = mark(n.kind);
    w::icon(p, cell(x, MARK_WIDTH - 4.0), icon, color, 14.0);
    x += MARK_WIDTH;
    let source = n.source.name(lang);
    w::text(
        p,
        cell(x, SOURCE_WIDTH - PAD),
        &w::fit(p, source, SOURCE_WIDTH - PAD, t::LABEL_DIM),
        t::LABEL_DIM,
        Align::Left,
    );
    x += SOURCE_WIDTH;
    let mut right = row.right() - PAD;
    if entry.count > 1 {
        let count = format!("×{}", entry.count);
        let width = w::text_width(p, &count, t::LABEL_SMALL) + 4.0;
        w::text(
            p,
            Rect::from_min_size(pos2(right - width, row.top()), vec2(width, row.height())),
            &count,
            t::LABEL_SMALL.with_color(t::TEXT_DIM),
            Align::Right,
        );
        right -= width + PAD;
    }
    let text = n.text.replace('\n', " ");
    let room = right - x;
    w::text(
        p,
        cell(x, room),
        &w::fit(p, &text, room, t::LABEL),
        t::LABEL,
        Align::Left,
    );
}
