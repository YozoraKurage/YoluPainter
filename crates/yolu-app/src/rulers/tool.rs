//! 定規のツールの欄: オプションバー（種類・パースの点の数・削除・スナップ 2 つ）と、左のドックのツールプロパティ（種類・点の数・線の本数・線対称・
//! 角度の刻み・編集レイヤーに作成・削除・スナップ 2 つ）。ここで選ぶ種類と設定は「これから作る定規」のもので、置いてある定規は替えない
//! （置いた定規はプロパティのレイヤーの欄の「定規」で替える）。ブラシ・図形のオプションバーにも、スナップ 2 つのボタンを置く。

use egui::{pos2, vec2, Rect, Ui};
use yolu_core::RulerKind;

use super::{RulerAction, STEP_RANGE};
use crate::lang::Lang;
use crate::panels::properties::{choice_buttons, slider_row, toggle_row, ChoiceButton};
use crate::state::{Action, AppState};
use crate::ui::{
    theme as t,
    widgets::{self as w, NumberFormat, Rows},
};

/// 種類の並び（ボタンの順）。
pub const KINDS: [RulerKind; 5] = [
    RulerKind::Line,
    RulerKind::Parallel,
    RulerKind::Concentric,
    RulerKind::Perspective,
    RulerKind::Symmetry,
];

/// 種類の名前。
pub fn kind_name(lang: Lang, kind: RulerKind) -> &'static str {
    match kind {
        RulerKind::Line => lang.pick("直線定規", "Straight Ruler"),
        RulerKind::Parallel => lang.pick("平行線", "Parallel"),
        RulerKind::Concentric => lang.pick("同心円", "Concentric"),
        RulerKind::Perspective => lang.pick("パース", "Perspective"),
        RulerKind::Symmetry => lang.pick("対称", "Symmetry"),
    }
}

fn kind_id(kind: RulerKind) -> &'static str {
    match kind {
        RulerKind::Line => "line",
        RulerKind::Parallel => "parallel",
        RulerKind::Concentric => "circle",
        RulerKind::Perspective => "perspective",
        RulerKind::Symmetry => "symmetry",
    }
}

/// スナップ 2 つのボタン（定規にスナップ・特殊定規にスナップ。入っているかが見える）。
pub fn snap_buttons(ui: &mut Ui, app: &mut AppState, straight: Rect, special: Rect) {
    let lang = app.lang;
    let enabled = !app.is_stroking();
    let items = [
        (
            straight,
            "rulers.snap",
            "snap_ruler",
            lang.pick("定規にスナップ", "Snap to Ruler"),
            RulerAction::ToggleSnapRuler,
            app.rulers.snap_ruler,
        ),
        (
            special,
            "rulers.snap_special",
            "snap_special",
            lang.pick("特殊定規にスナップ", "Snap to Special Ruler"),
            RulerAction::ToggleSnapSpecial,
            app.rulers.snap_special,
        ),
    ];
    for (at, id, icon, name, op, on) in items {
        let action = Action::Ruler(op);
        let tip = crate::shortcuts::tip_with_key(lang, name, &action);
        if w::icon_button(ui, at, id, icon, &tip, on, enabled, 20.0).clicked() {
            app.apply(action);
        }
    }
}

/// オプションバー（定規のツール）。
pub fn options(ui: &mut Ui, app: &mut AppState, r: Rect, mut x: f32) {
    let lang = app.lang;
    let enabled = !app.is_stroking();
    let mut button = |ui: &mut Ui, id: &str, label: &str, selected: bool, enabled: bool| {
        let width = w::text_width(ui.painter(), label, t::LABEL) + 22.0;
        let at = Rect::from_min_size(pos2(x, r.top() + 6.0), vec2(width, r.height() - 12.0));
        x += width + 3.0;
        w::button(ui, at, id, label, selected, enabled, None, None).clicked()
    };
    for kind in KINDS {
        let id = format!("ruler.{}", kind_id(kind));
        if button(
            ui,
            &id,
            kind_name(lang, kind),
            app.rulers.kind == kind,
            enabled,
        ) {
            app.rulers.kind = kind;
        }
    }
    if app.rulers.kind == RulerKind::Perspective {
        for (two, id, name) in [
            (false, "ruler.one", lang.pick("1 点", "1 Point")),
            (true, "ruler.two", lang.pick("2 点", "2 Points")),
        ] {
            if button(ui, id, name, app.rulers.two_points == two, enabled) {
                app.rulers.two_points = two;
            }
        }
    }
    // 消せる定規が無いあいだは無効（ツールプロパティの「削除」と同じ判定）
    let can_delete = enabled && app.has_ruler();
    if button(
        ui,
        "ruler.delete",
        lang.pick("削除", "Delete"),
        false,
        can_delete,
    ) {
        app.delete_rulers();
    }
    let at = |x: f32| Rect::from_min_size(pos2(x, r.top() + 6.0), vec2(28.0, r.height() - 12.0));
    snap_buttons(ui, app, at(x), at(x + 31.0));
}

/// 線の本数のスライダーの値を本数に（線対称は偶数だけ。いちばん近い偶数へ）。
pub fn fit_count(value: f32, line_symmetry: bool) -> u8 {
    let n = if line_symmetry {
        (value / 2.0).round() * 2.0
    } else {
        value.round()
    };
    super::fit_lines(n.clamp(2.0, 16.0) as u8, line_symmetry)
}

/// ツールプロパティ（定規のツール）。
pub fn props(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    let lang = app.lang;
    let enabled = !app.is_stroking();
    let ids: Vec<String> = KINDS
        .iter()
        .map(|k| format!("props.ruler.{}", kind_id(*k)))
        .collect();
    let items: Vec<ChoiceButton> = KINDS
        .iter()
        .zip(&ids)
        .map(|(kind, id)| ChoiceButton {
            id,
            label: kind_name(lang, *kind),
            selected: app.rulers.kind == *kind,
            enabled,
            tooltip: None,
        })
        .collect();
    if let Some(i) = choice_buttons(ui, rows, &items) {
        app.rulers.kind = KINDS[i];
    }
    if app.rulers.kind == RulerKind::Perspective {
        let items = [
            ChoiceButton {
                id: "props.ruler.one",
                label: lang.pick("1 点", "1 Point"),
                selected: !app.rulers.two_points,
                enabled,
                tooltip: None,
            },
            ChoiceButton {
                id: "props.ruler.two",
                label: lang.pick("2 点", "2 Points"),
                selected: app.rulers.two_points,
                enabled,
                tooltip: None,
            },
        ];
        if let Some(i) = choice_buttons(ui, rows, &items) {
            app.rulers.two_points = i == 1;
        }
    }
    if app.rulers.kind == RulerKind::Symmetry {
        let line_symmetry = app.rulers.line_symmetry;
        if let Some(v) = slider_row(
            ui,
            rows,
            "props.ruler.lines",
            lang.pick("線の本数", "Lines"),
            app.rulers.lines as f32,
            (2.0, 16.0),
            NumberFormat::int(""),
            None,
            enabled,
        ) {
            app.rulers.set_lines(fit_count(v, line_symmetry));
        }
        if let Some(on) = toggle_row(
            ui,
            rows,
            "props.ruler.line_symmetry",
            lang.pick("線対称", "Line Symmetry"),
            line_symmetry,
            None,
            enabled,
        ) {
            app.rulers.set_line_symmetry(on);
        }
    }
    if let Some(on) = toggle_row(
        ui,
        rows,
        "props.ruler.angle_step",
        lang.pick("角度の刻み", "Angle Step"),
        app.rulers.angle_step,
        None,
        enabled,
    ) {
        app.rulers.angle_step = on;
    }
    if app.rulers.angle_step {
        if let Some(v) = slider_row(
            ui,
            rows,
            "props.ruler.step",
            lang.pick("刻み", "Step"),
            app.rulers.step as f32,
            (*STEP_RANGE.start() as f32, *STEP_RANGE.end() as f32),
            NumberFormat::int("°"),
            None,
            enabled,
        ) {
            app.rulers.set_step(v.round() as u32);
        }
    }
    if let Some(on) = toggle_row(
        ui,
        rows,
        "props.ruler.on_layer",
        lang.pick("編集レイヤーに作成", "Create on Edit Layer"),
        app.rulers.on_layer,
        None,
        enabled,
    ) {
        app.rulers.on_layer = on;
    }
    let delete = [ChoiceButton {
        id: "props.ruler.delete",
        label: lang.pick("削除", "Delete"),
        selected: false,
        enabled: enabled && app.has_ruler(),
        tooltip: None,
    }];
    if choice_buttons(ui, rows, &delete).is_some() {
        app.delete_rulers();
    }
    for (id, name, value, op) in [
        (
            "props.ruler.snap",
            lang.pick("定規にスナップ", "Snap to Ruler"),
            app.rulers.snap_ruler,
            RulerAction::ToggleSnapRuler,
        ),
        (
            "props.ruler.snap_special",
            lang.pick("特殊定規にスナップ", "Snap to Special Ruler"),
            app.rulers.snap_special,
            RulerAction::ToggleSnapSpecial,
        ),
    ] {
        let action = Action::Ruler(op);
        let key = crate::shortcuts::shortcut_text(&action);
        if toggle_row(ui, rows, id, name, value, key.as_deref(), enabled).is_some() {
            app.apply(action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slider_value_becomes_a_line_count_that_suits_the_symmetry() {
        // 線対称は偶数だけ
        assert_eq!(fit_count(2.0, true), 2);
        assert_eq!(fit_count(2.9, true), 2);
        assert_eq!(fit_count(3.1, true), 4);
        assert_eq!(fit_count(15.9, true), 16);
        // 回転対称は 2〜16 の全部
        assert_eq!(fit_count(3.0, false), 3);
        assert_eq!(fit_count(16.4, false), 16);
        assert_eq!(fit_count(0.0, false), 2);
    }
}
