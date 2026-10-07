//! アクションのパネル（ドックのタブ「アクション」）: 操作の記録と再生。
//!
//! - 上の帯: 記録の開始・止め（記録中は押し込まれた見た目）、記録しなかった操作があった印（記録中だけ。数は出さない。理由はツールチップ）、
//!   右に選んだアクションの再生・削除。
//! - 一覧: 置いてあるアクションの名前（設定のフォルダの actions/ の順）。押して選び、ダブルクリックで名前の変更、ドラッグで並べ替え、
//!   右クリックで再生・名前の変更・上へ・下へ・削除。説明の文は置かない。

use egui::{pos2, vec2, Id, Rect, Sense, Stroke, Ui, WidgetInfo, WidgetType};

use crate::automation::AutomationOp;
use crate::state::{Action, AppState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};

pub const ROW_HEIGHT: f32 = 22.0;
pub const TOOLBAR_HEIGHT: f32 = 30.0;
const PAD: f32 = 8.0;

/// パネルの一覧のずらし量（画面の状態。保存しない）。
fn scroll_id() -> Id {
    Id::new("actions.scroll.value")
}

fn op(app: &mut AppState, op: AutomationOp) {
    app.apply(Action::Automation(op));
}

pub fn show(ui: &mut Ui, app: &mut AppState) {
    crate::automation::record::sync(app);
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let lang = app.lang;
    let recording = app.automation.is_recording();
    let selected = app
        .automation
        .selected
        .filter(|i| *i < app.automation.store.len());

    // 上の帯: 記録・（記録しなかった印）・再生・削除
    let bar = Rect::from_min_size(r.min, vec2(r.width(), TOOLBAR_HEIGHT));
    w::fill(ui.painter(), bar, t::PANEL_HEADER);
    let button = |x: f32| Rect::from_min_size(pos2(x, bar.top() + 3.0), vec2(26.0, 24.0));
    let (icon, tip) = if recording {
        ("stop", lang.pick("記録を止める", "Stop Recording"))
    } else {
        ("record", lang.pick("記録", "Record"))
    };
    if w::icon_button(
        ui,
        button(bar.left() + 4.0),
        "actions.record",
        icon,
        tip,
        recording,
        true,
        17.0,
    )
    .clicked()
    {
        op(
            app,
            if recording {
                AutomationOp::StopRecording
            } else {
                AutomationOp::StartRecording
            },
        );
    }
    if recording && app.automation.skipped() {
        let mark = button(bar.left() + 34.0);
        w::icon(ui.painter(), mark, "warning", t::WARNING, 15.0);
        let text = lang.pick(
            "記録しなかった操作があります（描く・選択範囲・変形など、命令の無い操作）",
            "Some operations were not recorded (drawing, selections, transforms and others without a command)",
        );
        let response = ui.interact(mark, Id::new("actions.skipped"), Sense::hover());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
        response.on_hover_text(text);
    }
    let right = bar.right() - 4.0;
    if w::icon_button(
        ui,
        button(right - 26.0 * 2.0 - 2.0),
        "actions.play",
        "play",
        lang.pick("再生", "Play"),
        false,
        selected.is_some() && !recording,
        16.0,
    )
    .clicked()
    {
        if let Some(i) = selected {
            op(app, AutomationOp::Play(i));
        }
    }
    if w::icon_button(
        ui,
        button(right - 26.0),
        "actions.delete",
        "delete",
        lang.pick("削除", "Delete"),
        false,
        selected.is_some(),
        17.0,
    )
    .clicked()
    {
        if let Some(i) = selected {
            op(app, AutomationOp::Delete(i));
        }
    }

    // 一覧
    let body = Rect::from_min_max(pos2(r.left(), bar.bottom()), r.max);
    w::fill(&ui.painter_at(body), body, t::PANEL_BG);
    let names: Vec<String> = app
        .automation
        .store
        .items()
        .iter()
        .map(|s| s.name.clone())
        .collect();
    let content = names.len() as f32 * ROW_HEIGHT;
    let mut scroll = ui.data(|d| d.get_temp::<f32>(scroll_id()).unwrap_or(0.0));
    let bars = Scroll::begin(ui, body, content, &mut scroll);
    let width = body.width() - bars.reserved();
    let row_at = |i: usize, scroll: f32| {
        Rect::from_min_size(
            pos2(body.left(), body.top() + i as f32 * ROW_HEIGHT - scroll),
            vec2(width, ROW_HEIGHT),
        )
    };
    let released = ui.input(|i| i.pointer.any_released());
    let pointer = ui.input(|i| i.pointer.interact_pos());
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.automation.dragging = None;
    }
    let mut pending: Vec<AutomationOp> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let row = row_at(i, scroll);
        if row.bottom() < body.top() || row.top() > body.bottom() {
            continue;
        }
        let painter = ui.painter_at(body);
        let is_selected = selected == Some(i);
        if app.automation.renaming == Some(i) {
            let first = !app.automation.rename_started;
            app.automation.rename_started = true;
            let field = Rect::from_min_max(
                pos2(row.left() + 4.0, row.top() + 1.0),
                pos2(row.right() - 4.0, row.bottom() - 1.0),
            );
            let out = w::text_field(
                ui,
                field,
                ("actions.rename", i),
                name,
                Some(lang.pick("アクションの名前", "Action name")),
                first,
            );
            if let Some(next) = out.committed {
                pending.push(AutomationOp::Rename(i, next));
            } else if !first && !out.focused {
                pending.push(AutomationOp::CancelRename);
            }
            continue;
        }
        let response = ui.interact(
            row.intersect(body),
            Id::new(("actions.row", i)),
            Sense::click_and_drag(),
        );
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::SelectableLabel, true, is_selected, name)
        });
        if is_selected {
            w::fill(&painter, row, t::ACCENT_SOFT);
        } else if response.hovered() {
            w::fill(&painter, row, t::CONTROL_HOVER);
        }
        let label = Rect::from_min_max(pos2(row.left() + PAD, row.top()), row.max);
        w::text(
            &painter,
            label,
            &w::fit(&painter, name, label.width() - PAD, t::LABEL),
            t::LABEL,
            Align::Left,
        );
        if response.double_clicked() || response.triple_clicked() {
            pending.push(AutomationOp::StartRename(i));
        } else if response.clicked() || response.secondary_clicked() {
            pending.push(AutomationOp::Select(Some(i)));
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            app.automation.dragging = Some(i);
        }
        let count = names.len();
        response.context_menu(|ui| {
            if ui
                .add_enabled(!recording, egui::Button::new(lang.pick("再生", "Play")))
                .clicked()
            {
                pending.push(AutomationOp::Play(i));
                ui.close();
            }
            if ui.button(lang.pick("名前を変更", "Rename")).clicked() {
                pending.push(AutomationOp::StartRename(i));
                ui.close();
            }
            if ui
                .add_enabled(i > 0, egui::Button::new(lang.pick("上へ移動", "Move Up")))
                .clicked()
            {
                pending.push(AutomationOp::Move { from: i, to: i - 1 });
                ui.close();
            }
            if ui
                .add_enabled(
                    i + 1 < count,
                    egui::Button::new(lang.pick("下へ移動", "Move Down")),
                )
                .clicked()
            {
                pending.push(AutomationOp::Move { from: i, to: i + 1 });
                ui.close();
            }
            ui.separator();
            if ui.button(lang.pick("削除", "Delete")).clicked() {
                pending.push(AutomationOp::Delete(i));
                ui.close();
            }
        });
    }
    // 並べ替えのドラッグ: 落とす所の線を引き、離したら動かす
    if let (Some(from), Some(at)) = (app.automation.dragging, pointer) {
        let slot = (((at.y - body.top() + scroll) / ROW_HEIGHT).round().max(0.0) as usize)
            .min(names.len());
        let to = if slot > from { slot - 1 } else { slot };
        if to != from {
            let y = body.top() + slot as f32 * ROW_HEIGHT - scroll;
            ui.painter_at(body).line_segment(
                [
                    pos2(body.left() + 2.0, y),
                    pos2(body.left() + width - 2.0, y),
                ],
                Stroke::new(2.0, t::ACCENT),
            );
        }
        if released {
            app.automation.dragging = None;
            if to != from && from < names.len() {
                pending.push(AutomationOp::Move { from, to });
            }
        }
    } else if released {
        app.automation.dragging = None;
    }
    // 何も無い所を押したら選びを外す
    let below = Rect::from_min_max(
        pos2(body.left(), (body.top() + content - scroll).max(body.top())),
        body.max,
    );
    if below.height() > 0.0
        && ui
            .interact(below, Id::new("actions.empty"), Sense::click())
            .clicked()
    {
        pending.push(AutomationOp::Select(None));
    }
    bars.end(ui, "actions.scroll", &mut scroll);
    ui.data_mut(|d| d.insert_temp(scroll_id(), scroll));
    for o in pending {
        op(app, o);
    }
}
