//! 左のドックの「サブツール」のパネル（クリスタのサブツールに当たる）: 今のツールのサブツールの一覧がパネルの高さいっぱいに出る
//! （ブラシは取り込みも含むブラシの一覧とグループのタブ、消しゴムは消しゴムの一覧、バケツ・グラデーションなどはそのツールの設定の組のプリセット、
//! 選択のツールは選択のツールの一覧、パスは 1 つ）。ツールプロパティとブラシサイズは別のパネル（`tool_props`）。右の「プロパティ」にはツールの設定を出さない。
//! 一覧の操作は、ブラシ・消しゴムが `Action::Brush`、そのほかのプリセットが `Action::SubTool`、選択のツールが `Action::SelectTool`。
//! 画面には名前と値だけを出し、説明はツールチップ。ツールの欄はツールの表（`tools`）が持つ。

use egui::{pos2, vec2, Color32, Rect, Sense, Ui, WidgetInfo, WidgetType};

use super::brushes::{self, FOOTER_HEIGHT};
use crate::m2_menu::Popup;
use crate::state::{Action, AppState, Tool};
use crate::subtool::{Key, SubToolAction};
use crate::tools::SubTools;
use crate::toolset::SlotId;
use crate::ui::menu::context_anchor;
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};

/// プリセット・ツールの行の高さ。
const ROW: f32 = 28.0;

/// 一覧の縦の組み立て（上の帯・下の帯。間が行）。
struct ListShape {
    strip: f32,
    footer: f32,
    /// グループのタブを並べる幅（スクロールの帯の分を先に引く。組み立てと描くのを同じ幅にする）。
    tabs_width: f32,
}

/// ブラシ・消しゴムのツールの一覧に出すツールの列の 1 つ（今のツール。列に無いツールをキーで使っているときは、そのツールの最初の 1 つ）。
fn brush_slot(app: &AppState, tool: Tool) -> Option<SlotId> {
    let set = &app.toolset.set;
    set.active_slot()
        .filter(|s| s.tool == tool)
        .map(|s| s.id)
        .or_else(|| set.first_of(tool))
}

fn list_shape(ui: &Ui, app: &AppState, tool: Tool, width: f32) -> ListShape {
    let tabs_width = (width - crate::ui::scroll::BAR_WIDTH).max(40.0);
    match tool.def().subtools {
        SubTools::Brushes | SubTools::Erasers => {
            let strip = brush_slot(app, tool).map_or(0.0, |s| {
                brushes::tab_layout(ui.painter(), app, s, tabs_width).1
            });
            ListShape {
                strip,
                footer: FOOTER_HEIGHT,
                tabs_width,
            }
        }
        SubTools::Presets => ListShape {
            strip: 0.0,
            footer: FOOTER_HEIGHT,
            tabs_width,
        },
        SubTools::Tools(_) | SubTools::Single => ListShape {
            strip: 0.0,
            footer: 0.0,
            tabs_width,
        },
    }
}

/// 行の背景（選んでいる・ホバー）と区切りの線。
fn row_background(ui: &Ui, row: Rect, selected: bool, hovered: bool) {
    let painter = ui.painter_at(ui.clip_rect());
    if selected {
        w::fill(&painter, row, t::ACCENT_SOFT);
        w::fill(
            &painter,
            Rect::from_min_size(row.min, vec2(3.0, row.height())),
            t::ACCENT,
        );
    } else if hovered {
        w::fill(&painter, row, t::CONTROL_HOVER);
    }
    w::hline(
        &painter,
        row.left(),
        row.right(),
        row.bottom() - 1.0,
        t::BORDER,
    );
}

/// プリセットの一覧の行（名前・変更ありの印）。押して替える・ダブルクリックで名前（利用者のもの）・右クリックでメニュー。
/// プリセットの行の見せ方（名前・変更あり・選んでいる）。
struct RowView<'a> {
    key: Key,
    name: &'a str,
    modified: bool,
    selected: bool,
}

fn preset_row(ui: &mut Ui, app: &mut AppState, tool: Tool, row: Rect, view: RowView) {
    let RowView {
        key,
        name,
        modified,
        selected,
    } = view;
    let clip = ui.clip_rect();
    let response = ui.interact(
        row.intersect(clip),
        ui.make_persistent_id(("subtool.row", tool.id(), key)),
        Sense::click(),
    );
    row_background(ui, row, selected, response.hovered());
    let painter = ui.painter_at(clip);
    let name_rect = Rect::from_min_max(
        pos2(row.left() + 10.0, row.top() + 2.0),
        pos2(row.right() - 8.0, row.bottom() - 2.0),
    );
    if app.subtools.ui.renaming == Some((tool, key)) {
        let first = !app.subtools.ui.rename_started;
        app.subtools.ui.rename_started = true;
        let out = w::text_field(
            ui,
            name_rect,
            ("subtool.rename", tool.id(), key),
            name,
            None,
            first,
        );
        if let Some(next) = out.committed {
            app.apply(Action::SubTool(SubToolAction::Rename(tool, key, next)));
        }
        if !first && !out.focused {
            app.subtools.ui.renaming = None;
        }
    } else {
        let dot = if modified { 14.0 } else { 0.0 };
        let shown = w::fit(&painter, name, name_rect.width() - dot, t::LABEL);
        let color = if selected { Color32::WHITE } else { t::TEXT };
        w::text(
            &painter,
            name_rect,
            &shown,
            t::LABEL.with_color(color),
            Align::Left,
        );
        if modified {
            let x = name_rect.left() + w::text_width(&painter, &shown, t::LABEL) + 8.0;
            painter.circle_filled(pos2(x, name_rect.center().y), 3.0, t::WARNING);
        }
    }
    if response.clicked() {
        app.apply(Action::SubTool(SubToolAction::Select(tool, key)));
        if app.subtools.ui.renaming != Some((tool, key)) {
            app.subtools.ui.renaming = None;
        }
    }
    if (response.double_clicked() || response.triple_clicked()) && key.is_user() {
        app.apply(Action::SubTool(SubToolAction::StartRename(tool, key)));
    }
    if response.secondary_clicked() {
        if let Some(at) = response.interact_pointer_pos() {
            app.subtools.ui.context = Some((tool, key));
            super::properties::open_popup(
                app,
                ui.ctx(),
                Popup::SubToolContext,
                context_anchor(at),
                0.0,
            );
        }
    }
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, name));
    let tip = if modified {
        app.lang
            .pick(format!("{name}（変更あり）"), format!("{name} (modified)"))
    } else {
        name.to_owned()
    };
    let _ = response.on_hover_text(tip);
}

/// ツールの一覧の行（アイコン・名前・キー）。押すとツールが替わる。
fn tool_row(
    ui: &mut Ui,
    app: &mut AppState,
    row: Rect,
    tool: Tool,
    selected: bool,
    clickable: bool,
) {
    let lang = app.lang;
    let clip = ui.clip_rect();
    let response = ui.interact(
        row.intersect(clip),
        ui.make_persistent_id(("subtool.tool", tool.id())),
        if clickable {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    row_background(ui, row, selected, response.hovered() && clickable);
    let painter = ui.painter_at(clip);
    w::icon(
        &painter,
        Rect::from_min_size(pos2(row.left() + 8.0, row.top()), vec2(22.0, row.height())),
        &format!("tools/{}", tool.id()),
        if selected { Color32::WHITE } else { t::TEXT },
        18.0,
    );
    let name = tool.name_in(lang);
    // キーは、名前が入りきるときだけ右に出す（狭いパネルで名前を詰めない。キーはツールチップにも）
    let key_text = crate::shortcuts::tool_key(tool);
    let key = Some(key_text.as_str())
        .filter(|k| !k.is_empty())
        .filter(|k| {
            let needed = 36.0
                + w::text_width(&painter, name, t::LABEL)
                + w::text_width(&painter, k, t::LABEL_DIM)
                + 24.0;
            needed <= row.width()
        })
        .unwrap_or("");
    let key_w = if key.is_empty() {
        0.0
    } else {
        w::text_width(&painter, key, t::LABEL_DIM) + 8.0
    };
    let name_rect = Rect::from_min_max(
        pos2(row.left() + 36.0, row.top()),
        pos2(row.right() - key_w - 8.0, row.bottom()),
    );
    let shown = w::fit(&painter, name, name_rect.width(), t::LABEL);
    w::text(
        &painter,
        name_rect,
        &shown,
        t::LABEL.with_color(if selected { Color32::WHITE } else { t::TEXT }),
        Align::Left,
    );
    if !key.is_empty() {
        w::text(
            &painter,
            Rect::from_min_max(
                pos2(row.right() - key_w - 4.0, row.top()),
                pos2(row.right() - 8.0, row.bottom()),
            ),
            key,
            t::LABEL_DIM,
            Align::Right,
        );
    }
    if response.clicked() {
        app.apply(Action::SelectTool(tool));
    }
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, clickable, selected, name)
    });
    let tip = match key_text.as_str() {
        "" => name.to_owned(),
        k => format!("{name} ({k})"),
    };
    let _ = response.on_hover_text(tip);
}

/// 今の行（`current` の番号）がウィンドウ（高さ `window`）に入るスクロールの値。すでに入っていれば `scroll` のまま。
fn scroll_to_show(scroll: f32, current: usize, window: f32) -> f32 {
    let (top, bottom) = (current as f32 * ROW, (current + 1) as f32 * ROW);
    if top < scroll {
        top
    } else if bottom > scroll + window {
        bottom - window
    } else {
        scroll
    }
}

/// 行の一覧の中（スクロールつき）。`current` は今の行の番号（替えたあと、その行が見えるところまでスクロールする）、
/// `draw` は行を描く関数（行の矩形・何番目か）。
fn rows_body(
    ui: &mut Ui,
    app: &mut AppState,
    list: Rect,
    count: usize,
    current: Option<usize>,
    mut draw: impl FnMut(&mut Ui, &mut AppState, Rect, usize),
) {
    w::fill(ui.painter(), list, t::CONTROL_BG);
    let content = count as f32 * ROW;
    app.subtools.ui.list_content = content;
    if std::mem::take(&mut app.subtools.ui.reveal) {
        if let Some(i) = current {
            app.subtools.ui.list_scroll =
                scroll_to_show(app.subtools.ui.list_scroll, i, list.height());
        }
    }
    // ホイールは、このパネルを包む全体のスクロールが（一覧とどちらが受けるかを決めて）受ける
    let bar = Scroll::new(list, content, &mut app.subtools.ui.list_scroll);
    let scroll = app.subtools.ui.list_scroll;
    let row_width = list.width() - bar.reserved();
    let outer = ui.clip_rect();
    ui.set_clip_rect(list.intersect(outer));
    for i in 0..count {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + i as f32 * ROW - scroll),
            vec2(row_width, ROW),
        );
        if row.bottom() < list.top() || row.top() > list.bottom() {
            continue;
        }
        draw(ui, app, row, i);
    }
    ui.set_clip_rect(outer);
    bar.end(ui, "subtools.list.scroll", &mut app.subtools.ui.list_scroll);
}

/// プリセットの一覧の下の帯: 元に戻す・複製・追加・削除（右寄せ）。
fn preset_footer(ui: &mut Ui, app: &mut AppState, tool: Tool, bar: Rect) {
    let lang = app.lang;
    w::fill(ui.painter(), bar, t::PANEL_HEADER);
    w::hline(ui.painter(), bar.left(), bar.right(), bar.top(), t::BORDER);
    let Some(list) = app.subtool_list(tool) else {
        return;
    };
    let current = list.current();
    let user = current.is_user();
    let modified = app.subtool_is_modified(tool, current);
    let free = !app.is_stroking();
    let button =
        |x: f32| Rect::from_min_size(pos2(x, bar.top() + 2.0), vec2(26.0, bar.height() - 4.0));
    let mut x = bar.right() - 4.0 - 26.0;
    if w::icon_button(
        ui,
        button(x),
        ("subtool.delete", tool.id()),
        "delete",
        if user {
            lang.pick("サブツールを削除", "Delete Sub Tool")
        } else {
            lang.pick(
                "組み込みのサブツールは消せません",
                "Built-in sub tools cannot be deleted",
            )
        },
        false,
        free && user,
        17.0,
    )
    .clicked()
    {
        app.apply(Action::SubTool(SubToolAction::Delete(tool, current)));
    }
    x -= 28.0;
    if w::icon_button(
        ui,
        button(x),
        ("subtool.add", tool.id()),
        "add",
        lang.pick(
            "今の設定を新しいサブツールに",
            "Add the Current Settings as a Sub Tool",
        ),
        false,
        free,
        18.0,
    )
    .clicked()
    {
        app.apply(Action::SubTool(SubToolAction::Add(tool)));
    }
    x -= 28.0;
    if w::icon_button(
        ui,
        button(x),
        ("subtool.duplicate", tool.id()),
        "content_copy",
        lang.pick("サブツールを複製", "Duplicate Sub Tool"),
        false,
        free,
        16.0,
    )
    .clicked()
    {
        app.apply(Action::SubTool(SubToolAction::Duplicate(tool, current)));
    }
    x -= 28.0;
    if w::icon_button(
        ui,
        button(x),
        ("subtool.revert", tool.id()),
        "restart_alt",
        if modified {
            lang.pick(
                "元の設定に戻す（変更あり）",
                "Revert to the original settings (modified)",
            )
        } else {
            lang.pick("元の設定に戻す", "Revert to the original settings")
        },
        false,
        free && modified,
        16.0,
    )
    .clicked()
    {
        app.apply(Action::SubTool(SubToolAction::Revert(tool, current)));
    }
}

/// 一覧の行の矩形（上の帯と下の帯の間。パネルが低すぎて入らなければ高さ 0）。
fn list_body_rect(area: Rect, shape: &ListShape) -> Rect {
    let top = area.top() + shape.strip;
    Rect::from_min_max(
        pos2(area.left(), top),
        pos2(area.right(), (area.bottom() - shape.footer).max(top)),
    )
}

/// サブツールの一覧の全体（上の帯・行・下の帯）。`area` は一覧の矩形。
fn list_section(ui: &mut Ui, app: &mut AppState, tool: Tool, area: Rect, shape: &ListShape) {
    let strip = Rect::from_min_size(area.min, vec2(area.width(), shape.strip));
    let body = list_body_rect(area, shape);
    let footer = Rect::from_min_max(pos2(area.left(), area.bottom() - shape.footer), area.max);
    match tool.def().subtools {
        SubTools::Brushes | SubTools::Erasers => {
            let Some(slot) = brush_slot(app, tool) else {
                return;
            };
            let tabs = Rect::from_min_size(strip.min, vec2(shape.tabs_width, strip.height()));
            {
                let p = ui.painter();
                w::fill(p, strip, t::PANEL_HEADER);
                w::hline(
                    p,
                    strip.left(),
                    strip.right(),
                    strip.bottom() - 1.0,
                    t::BORDER,
                );
            }
            brushes::group_tabs(ui, tabs, app, slot);
            brushes::list_body(ui, app, body, slot);
            let erasers = tool.def().subtools == SubTools::Erasers;
            if erasers {
                // 取り込みのファイルを落とす先はブラシの一覧だけ
                app.brushes.ui.list_rect = None;
            }
            brushes::footer(ui, app, footer, !erasers);
        }
        SubTools::Presets => {
            let lang = app.lang;
            let Some(list) = app.subtool_list(tool) else {
                return;
            };
            let current = list.current();
            let rows: Vec<(Key, String)> = list
                .entries()
                .iter()
                .map(|e| (e.key, list.name_in(e.key, lang)))
                .collect();
            let count = rows.len();
            let at = rows.iter().position(|(key, _)| *key == current);
            rows_body(ui, app, body, count, at, |ui, app, row, i| {
                let (key, name) = &rows[i];
                let modified = app.subtool_is_modified(tool, *key);
                preset_row(
                    ui,
                    app,
                    tool,
                    row,
                    RowView {
                        key: *key,
                        name,
                        modified,
                        selected: *key == current,
                    },
                );
            });
            preset_footer(ui, app, tool, footer);
        }
        SubTools::Tools(tools) => {
            let at = tools.iter().position(|t| *t == app.tool);
            rows_body(ui, app, body, tools.len(), at, |ui, app, row, i| {
                let selected = app.tool == tools[i];
                tool_row(ui, app, row, tools[i], selected, true);
            });
        }
        SubTools::Single => {
            rows_body(ui, app, body, 1, Some(0), |ui, app, row, _| {
                tool_row(ui, app, row, tool, true, false)
            });
        }
    }
}

pub fn show(ui: &mut Ui, app: &mut AppState) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    app.brushes
        .samples
        .begin_frame(ui.ctx().cumulative_pass_nr());
    let tool = app.tool;
    app.subtools.ui.panel_right = r.right();
    app.subtool_follow(tool);
    let shape = list_shape(ui, app, tool, r.width());
    // ホイールは一覧の中で受ける（行の外へ送る全体のスクロールは無い。範囲は一覧が収める）
    if ui.rect_contains_pointer(list_body_rect(r, &shape)) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        match tool.def().subtools {
            SubTools::Brushes | SubTools::Erasers => app.brushes.ui.list_scroll -= wheel,
            _ => app.subtools.ui.list_scroll -= wheel,
        }
    }
    let outer = ui.clip_rect();
    ui.set_clip_rect(r.intersect(outer));
    list_section(ui, app, tool, r, &shape);
    ui.set_clip_rect(outer);
}
