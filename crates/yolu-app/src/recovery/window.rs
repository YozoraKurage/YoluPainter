//! 復旧の窓: 世代の一覧（名前・セットの数・経過時間）から、開く・捨てる。起動したとき前回が正しく閉じていなければ自動で出し、
//! ファイル ▸ 復旧… からも開く。書き置きの間隔と残す世代の数もここで選ぶ（設定のファイルに書く）。
//! 画面には名前・状態・短い理由だけを出し、説明はツールチップに置く。世代が 1 つも無いときは何も書かない（一覧の場所が空くだけ。
//! 状態の行は、開く・捨てるが断られた理由があるときだけ出す）。

use std::time::{SystemTime, UNIX_EPOCH};

use egui::{pos2, vec2, Id, Key, Rect, Sense, Vec2};

use super::{pool::Row, text, RecoveryAction, INTERVAL_RANGE, KEEP_RANGE};
use crate::lang::Lang;
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::ui::window::{self, Spec};

/// 書き置きの間隔（秒）・残す世代の数の選択肢。
const INTERVALS: [u32; 5] = [10, 15, 30, 60, 300];
const KEEPS: [u32; 5] = [2, 3, 5, 10, 20];

const ROW_HEIGHT: f32 = 26.0;
const MAX_ROWS: usize = 8;
const SETTINGS_ROW: f32 = 28.0;
const FOOTER: f32 = 48.0;

/// 窓の状態。
#[derive(Debug, Default)]
pub struct WindowState {
    pub rows: Vec<Row>,
    pub selected: Option<usize>,
    /// 捨てる前の確かめの対象。
    pub confirm: Option<Row>,
    /// 開く・捨てるが断られた理由（短く）。
    pub error: Option<String>,
    pub offset: Vec2,
    pub scroll: f32,
}

impl WindowState {
    /// 一覧を入れ替える。選んでいた世代が残っていればそのまま、無ければ新しい読める世代を選ぶ。
    pub fn set_rows(&mut self, rows: Vec<Row>) {
        let keep = self
            .selected_row()
            .map(|r| (r.pool.clone(), r.id.clone()));
        self.rows = rows;
        self.selected = keep
            .and_then(|(pool, id)| self.rows.iter().position(|r| r.pool == pool && r.id == id))
            .or_else(|| self.rows.iter().position(|r| r.problem.is_none()));
        self.scroll = 0.0;
    }
    pub fn select(&mut self, index: usize) {
        if index < self.rows.len() {
            self.selected = Some(index);
            self.error = None;
        }
    }
    pub fn selected_row(&self) -> Option<&Row> {
        self.selected.and_then(|i| self.rows.get(i))
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 行の右の列: 経過時間（壊れた世代は「読めません」）。
fn row_age(lang: Lang, row: &Row) -> String {
    if row.problem.is_some() {
        return lang.pick("読めません", "Unreadable").into();
    }
    match row.time_ms {
        Some(ms) => lang.age_text(std::time::Duration::from_millis(now_ms().saturating_sub(ms))),
        None => String::new(),
    }
}

/// 行の名前（元の .ylp の名前。無ければ「名称未設定」）。
pub fn row_name(lang: Lang, row: &Row) -> String {
    if row.name.is_empty() {
        lang.pick("名称未設定", "Untitled").into()
    } else {
        row.name.clone()
    }
}

/// 窓を描く（開いていれば）。押されたものは `Action::Recovery` として当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if app.recovery.window.is_none() {
        return;
    }
    let lang = app.lang;
    let (interval, keep) = (
        app.recovery.settings().interval_seconds,
        app.recovery.settings().generations_to_keep,
    );
    let Some(state) = app.recovery.window.as_ref() else {
        return;
    };
    let rows = state.rows.clone();
    let selected = state.selected;
    let error = state.error.clone();
    let confirm = state.confirm.clone();
    let mut offset = state.offset;
    let mut scroll = state.scroll;
    let visible = rows.len().clamp(1, MAX_ROWS);
    // 状態の 1 行（断られた理由）があるときだけ場所を取る
    let summary_h = if error.is_some() { 26.0 } else { 6.0 };
    let height = window::HEADER_HEIGHT
        + summary_h
        + visible as f32 * ROW_HEIGHT
        + 10.0
        + 2.0 * SETTINGS_ROW
        + FOOTER;
    let spec = Spec {
        title: lang.pick("復旧", "Recovery"),
        icon: Some("restart_alt"),
        size: vec2(580.0, height),
        modal: false,
        close_label: lang.pick("ウィンドウを閉じる", "Close Window"),
    };
    let id = Id::new(("yolu.window", "recovery"));
    let mut actions: Vec<RecoveryAction> = Vec::new();
    let can_open = selected
        .and_then(|i| rows.get(i))
        .is_some_and(|r| r.problem.is_none());
    let can_discard = selected.is_some();
    let mut esc = false;
    let closed = window::show(ctx, id, &spec, &mut offset, false, |ui, frame| {
        // 確かめの窓が上にあるあいだは、Esc はそちらへ
        esc = confirm.is_none()
            && ui.input(|i| i.key_pressed(Key::Escape))
            && ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|p| frame.rect.contains(p));
        let body = frame.body;
        let p = ui.painter().clone();
        // 状態の 1 行（断られた理由があるときだけ）
        let top = body.top() + 4.0;
        if let Some(summary) = &error {
            let r = Rect::from_min_size(pos2(body.left() + 14.0, top), vec2(body.width() - 28.0, 22.0));
            let shown = w::fit(&p, summary, r.width(), t::LABEL);
            w::text(&p, r, &shown, t::LABEL.with_color(t::WARNING), Align::Left);
            if shown != *summary {
                ui.interact(r, id.with("summary"), Sense::hover()).on_hover_text(summary);
            }
        }
        let list = Rect::from_min_size(
            pos2(body.left(), top + summary_h - 4.0),
            vec2(body.width(), visible as f32 * ROW_HEIGHT),
        );
        let content = rows.len() as f32 * ROW_HEIGHT;
        let max_scroll = (content - list.height()).max(0.0);
        if ui.rect_contains_pointer(list) {
            scroll -= ui.input(|i| i.smooth_scroll_delta.y);
        }
        scroll = scroll.clamp(0.0, max_scroll);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
        child.set_clip_rect(list.intersect(ui.clip_rect()));
        let cp = child.painter().clone();
        let right_w = rows
            .iter()
            .map(|r| w::text_width(&cp, &row_age(lang, r), t::LABEL_DIM))
            .fold(0.0f32, f32::max);
        let mid_w = rows
            .iter()
            .map(|r| w::text_width(&cp, &lang.sets_text(r.documents), t::LABEL_DIM))
            .fold(0.0f32, f32::max);
        for (i, row) in rows.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(list.left() + 8.0, list.top() + i as f32 * ROW_HEIGHT - scroll),
                vec2(list.width() - 16.0 - if max_scroll > 0.0 { 8.0 } else { 0.0 }, ROW_HEIGHT),
            );
            if r.bottom() < list.top() || r.top() > list.bottom() {
                continue;
            }
            let name = row_name(lang, row);
            let on = selected == Some(i);
            let response = child.interact(r, id.with(("row", i)), Sense::click());
            if on {
                w::rounded(&cp, r, t::CONTROL_ACTIVE, 3.0);
            } else if response.hovered() {
                w::rounded(&cp, r, t::CONTROL_HOVER, 3.0);
            }
            let color = if row.problem.is_some() { t::WARNING } else { t::TEXT };
            let right = Rect::from_min_size(pos2(r.right() - 8.0 - right_w, r.top()), vec2(right_w, r.height()));
            let middle = Rect::from_min_size(
                pos2(right.left() - 14.0 - mid_w, r.top()),
                vec2(mid_w, r.height()),
            );
            let left = Rect::from_min_max(pos2(r.left() + 8.0, r.top()), pos2(middle.left() - 12.0, r.bottom()));
            let shown = w::fit(&cp, &name, left.width(), t::LABEL);
            w::text(&cp, left, &shown, t::LABEL.with_color(color), Align::Left);
            w::text(&cp, middle, &lang.sets_text(row.documents), t::LABEL_DIM, Align::Left);
            w::text(
                &cp,
                right,
                &row_age(lang, row),
                t::LABEL_DIM.with_color(if row.problem.is_some() { t::WARNING } else { t::TEXT_DIM }),
                Align::Right,
            );
            let tooltip = match (&row.problem, row.time_ms) {
                (Some(reason), _) => lang.pick(reason.clone(), "Damaged generation (cannot be read)".into()),
                (None, Some(ms)) => text::utc_text(ms),
                (None, None) => String::new(),
            };
            let label = name.clone();
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, &label)
            });
            if response.clicked() {
                actions.push(RecoveryAction::Select(i));
            }
            if response.double_clicked() && row.problem.is_none() {
                actions.push(RecoveryAction::Select(i));
                actions.push(RecoveryAction::Open);
            }
            if !tooltip.is_empty() {
                response.on_hover_text(tooltip);
            }
        }
        if max_scroll > 0.0 {
            let bar_h = (list.height() * list.height() / content).max(16.0);
            let bar_y = list.top() + (list.height() - bar_h) * scroll / max_scroll;
            w::rounded(
                &cp,
                Rect::from_min_size(pos2(list.right() - 8.0, bar_y), vec2(4.0, bar_h)),
                t::CONTROL_ACTIVE,
                2.0,
            );
        }
        // 設定（間隔・残す世代）
        let mut y = list.bottom() + 6.0;
        let seconds = |n: u32| -> String {
            if n >= 60 && n.is_multiple_of(60) {
                lang.pick(format!("{} 分", n / 60), format!("{} min", n / 60))
            } else {
                lang.pick(format!("{n} 秒"), format!("{n} s"))
            }
        };
        let mut interval_values: Vec<u32> = INTERVALS.to_vec();
        if !interval_values.contains(&interval) && (INTERVAL_RANGE.0..=INTERVAL_RANGE.1).contains(&interval) {
            interval_values.push(interval);
            interval_values.sort_unstable();
        }
        let interval_labels: Vec<String> = interval_values.iter().map(|n| seconds(*n)).collect();
        let mut keep_values: Vec<u32> = KEEPS.to_vec();
        if !keep_values.contains(&keep) && (KEEP_RANGE.0..=KEEP_RANGE.1).contains(&keep) {
            keep_values.push(keep);
            keep_values.sort_unstable();
        }
        let keep_labels: Vec<String> = keep_values.iter().map(|n| n.to_string()).collect();
        let settings_rows: [(&str, &[String], usize, &str); 2] = [
            (
                lang.pick("書き置きの間隔", "Checkpoint interval"),
                &interval_labels,
                interval_values.iter().position(|n| *n == interval).unwrap_or(0),
                "interval",
            ),
            (
                lang.pick("残す世代", "Generations kept"),
                &keep_labels,
                keep_values.iter().position(|n| *n == keep).unwrap_or(0),
                "keep",
            ),
        ];
        let label_w = settings_rows
            .iter()
            .map(|(l, ..)| w::text_width(&p, l, t::LABEL_DIM))
            .fold(0.0f32, f32::max)
            + 8.0;
        for (label, options, active, key) in settings_rows {
            let r = Rect::from_min_size(pos2(body.left() + 14.0, y), vec2(body.width() - 28.0, SETTINGS_ROW));
            let lr = Rect::from_min_size(r.min, vec2(label_w, r.height()));
            w::text(&p, lr, label, t::LABEL_DIM, Align::Left);
            ui.interact(lr, id.with(("settings-tip", key)), Sense::hover())
                .on_hover_text(text::settings_tip(lang, key));
            let strip = Rect::from_min_max(pos2(r.left() + label_w, r.top() + 2.0), pos2(r.right(), r.bottom() - 2.0));
            if let Some(picked) = segmented(ui, strip, id.with(key), options, active) {
                actions.push(if key == "interval" {
                    RecoveryAction::SetInterval(interval_values[picked])
                } else {
                    RecoveryAction::SetKeep(keep_values[picked])
                });
            }
            y += SETTINGS_ROW;
        }
        // 下の帯
        let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOTER), body.max);
        w::fill(&p, footer, t::PANEL_HEADER);
        w::hline(&p, footer.left(), footer.right(), footer.top(), t::BORDER);
        let mut x = footer.right() - 14.0;
        let buttons: [(&str, bool, bool, &str, RecoveryAction); 3] = [
            (
                lang.pick("開く", "Open"),
                true,
                can_open,
                lang.pick(
                    "選んだ世代を「名称未設定（復旧）」として開きます。元のファイルには書きません",
                    "Opens the selected generation as “Untitled (Recovered)”. The original file is not written",
                ),
                RecoveryAction::Open,
            ),
            (
                lang.pick("捨てる", "Discard"),
                false,
                can_discard,
                lang.pick(
                    "選んだ世代を消します。保存した .ylp は残ります",
                    "Deletes the selected generation. Saved .ylp files stay",
                ),
                RecoveryAction::Discard,
            ),
            (lang.pick("閉じる", "Close"), false, true, "", RecoveryAction::CloseWindow),
        ];
        for (i, (label, primary, enabled, tooltip, action)) in buttons.into_iter().enumerate() {
            let bw = w::text_width(&p, label, t::LABEL) + 32.0;
            let r = Rect::from_min_size(pos2(x - bw, footer.top() + 10.0), vec2(bw, 28.0));
            x = r.left() - 8.0;
            let tooltip = (!tooltip.is_empty()).then_some(tooltip);
            if w::button(ui, r, id.with(("button", i)), label, primary, enabled, tooltip, None).clicked() {
                actions.push(action);
            }
        }
    });
    if let Some(state) = app.recovery.window.as_mut() {
        state.offset = offset;
        state.scroll = scroll;
    }
    if closed || esc {
        actions.push(RecoveryAction::CloseWindow);
    }
    for action in actions {
        app.apply(Action::Recovery(action));
    }
    discard_confirm(ctx, app);
}

/// 捨てる前の確かめ（開いている間は下の窓を触れない）。
fn discard_confirm(ctx: &egui::Context, app: &mut AppState) {
    let Some(row) = app.recovery.window.as_ref().and_then(|w| w.confirm.clone()) else {
        return;
    };
    let lang = app.lang;
    let age = row_age(lang, &row);
    let spec = crate::windows::ListSpec {
        id: "recovery-discard",
        title: lang.pick("世代を捨てる", "Discard Generation").into(),
        icon: "warning",
        modal: true,
        width: 440.0,
        summary: Some((format!("{} · {age}", row_name(lang, &row)), true)),
        rows: Vec::new(),
        buttons: vec![
            crate::windows::Button {
                label: lang.pick("やめる", "Cancel").into(),
                primary: false,
                tooltip: None,
            },
            crate::windows::Button {
                label: lang.pick("捨てる", "Discard").into(),
                primary: true,
                tooltip: Some(
                    lang.pick(
                        "この世代を消します。元に戻せません。保存した .ylp は残ります",
                        "Deletes this generation permanently. Saved .ylp files stay",
                    )
                    .into(),
                ),
            },
        ],
        close_label: lang.pick("ウィンドウを閉じる", "Close Window").into(),
    };
    let mut offset = Vec2::ZERO;
    let mut scroll = 0.0;
    match crate::windows::show_list(ctx, &spec, &mut offset, &mut scroll) {
        Some(crate::windows::Reply::Button(1)) => app.apply(Action::Recovery(RecoveryAction::ConfirmDiscard)),
        Some(_) => app.apply(Action::Recovery(RecoveryAction::CancelDiscard)),
        None => {}
    }
}

/// 選択肢の帯（1 つを選ぶ）。押された選択肢の番号を返す（今のものを押しても返さない）。名前は選択肢の文字。
fn segmented(ui: &mut egui::Ui, r: Rect, id: Id, options: &[String], active: usize) -> Option<usize> {
    let n = options.len().max(1);
    let seg_w = (r.width() / n as f32).min(86.0);
    let mut picked = None;
    for (i, label) in options.iter().enumerate() {
        let sr = Rect::from_min_size(pos2(r.left() + i as f32 * (seg_w + 4.0), r.top()), vec2(seg_w, r.height()));
        let on = i == active;
        let response = ui.interact(sr, id.with(i), Sense::click());
        let p = ui.painter();
        if on {
            w::rounded(p, sr, t::ACCENT_DIM, 4.0);
        } else if response.hovered() {
            w::rounded(p, sr, t::CONTROL_HOVER, 4.0);
        } else {
            w::rounded(p, sr, t::CONTROL_BG, 4.0);
        }
        w::text(
            p,
            sr,
            label,
            t::LABEL.with_color(if on { egui::Color32::WHITE } else { t::TEXT }),
            Align::Center,
        );
        let name = label.clone();
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, &name));
        if response.clicked() && !on {
            picked = Some(i);
        }
    }
    picked
}
