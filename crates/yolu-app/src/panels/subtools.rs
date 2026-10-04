//! 左のドックの「サブツール」のパネル（クリスタのサブツール・ツールプロパティ・ブラシサイズを 1 か所にしたもの）。中身は今の道具に合わせて替わる:
//! 上から、サブツールの一覧（ブラシは取り込みも含むブラシの一覧とグループのタブ、消しゴムは消しゴムの一覧、バケツ・グラデーションなどはその道具の
//! 設定の組のプリセット、選択の道具は選択の道具の一覧、パスは 1 つ）、ツールプロパティ（今の道具の設定の全部。値はオプションバーと同じ状態）、
//! ブラシサイズ（大きさを持つ道具だけ）。入りきらなければ全体がスクロールする。右の「プロパティ」には道具の設定を出さない。
//! 一覧の操作は、ブラシ・消しゴムが `Action::Brush`、そのほかのプリセットが `Action::SubTool`、選択の道具が `Action::SelectTool`。
//! 画面には名前と値だけを出し、説明はツールチップ。道具の欄は道具の表（`tools`）が持つ。

use egui::{pos2, vec2, Color32, Rect, Sense, Ui, WidgetInfo, WidgetType};

use super::brushes::{self, FOOTER_HEIGHT, GROUP_STRIP, MIN_LIST, ROW_HEIGHT, SECTION_HEIGHT};
use crate::brushes::Group;
use crate::m2_menu::Popup;
use crate::state::{Action, AppState, Tool};
use crate::subtool::{Key, SubToolAction};
use crate::tools::SubTools;
use crate::ui::menu::context_anchor;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, Rows};

/// プリセット・道具の行の高さと、一覧が縮めずに出す行の数の上限。
const ROW: f32 = 28.0;
const MAX_ROWS: f32 = 8.0;

/// 一覧の縦の組み立て（上の帯・行・下の帯）。
struct ListShape {
    strip: f32,
    /// 行の全部の高さ（スクロールの中身）。
    body: f32,
    footer: f32,
    /// 余った高さを全部使う（ブラシの一覧。グループを替えても下の欄が動かない）。
    fills: bool,
}

fn list_shape(app: &AppState, tool: Tool) -> ListShape {
    match tool.def().subtools {
        SubTools::Brushes => ListShape {
            strip: GROUP_STRIP,
            body: app.brushes.lib.in_group(brush_group(app)).len() as f32 * ROW_HEIGHT,
            footer: FOOTER_HEIGHT,
            fills: true,
        },
        SubTools::Erasers => ListShape {
            strip: 0.0,
            body: app.brushes.lib.in_group(Group::Eraser).len() as f32 * ROW_HEIGHT,
            footer: FOOTER_HEIGHT,
            fills: true,
        },
        SubTools::Presets => ListShape {
            strip: 0.0,
            body: app.subtool_list(tool).map_or(0, |l| l.entries().len()) as f32 * ROW,
            footer: FOOTER_HEIGHT,
            fills: false,
        },
        SubTools::Tools(tools) => ListShape {
            strip: 0.0,
            body: tools.len() as f32 * ROW,
            footer: 0.0,
            fills: false,
        },
        SubTools::Single => ListShape {
            strip: 0.0,
            body: ROW,
            footer: 0.0,
            fills: false,
        },
    }
}

/// ブラシの道具で出すグループ（消しゴムのグループは消しゴムの道具のものなので、そのときは先頭のグループ）。
fn brush_group(app: &AppState) -> Group {
    let group = app.brushes.ui.group;
    if group.is_eraser() {
        Group::Pen
    } else {
        group
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

/// 道具の一覧の行（アイコン・名前・キー）。押すと道具が替わる。
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
    let key = Some(tool.key())
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
    let tip = match tool.key() {
        "" => name.to_owned(),
        k => format!("{name} ({k})"),
    };
    let _ = response.on_hover_text(tip);
}

/// 今の行（`current` の番号）が窓（高さ `window`）に入るスクロールの値。すでに入っていれば `scroll` のまま。
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
    let max_scroll = (content - list.height()).max(0.0);
    if std::mem::take(&mut app.subtools.ui.reveal) {
        if let Some(i) = current {
            app.subtools.ui.list_scroll =
                scroll_to_show(app.subtools.ui.list_scroll, i, list.height());
        }
    }
    app.subtools.ui.list_scroll = app.subtools.ui.list_scroll.clamp(0.0, max_scroll);
    let scroll = app.subtools.ui.list_scroll;
    let row_width = list.width() - if max_scroll > 0.0 { 10.0 } else { 0.0 };
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
    if max_scroll > 0.0 {
        let bar_h = (list.height() * list.height() / content).max(16.0);
        let bar_y = list.top() + (list.height() - bar_h) * scroll / max_scroll;
        w::rounded(
            ui.painter(),
            Rect::from_min_size(pos2(list.right() - 6.0, bar_y), vec2(4.0, bar_h)),
            t::CONTROL_ACTIVE,
            2.0,
        );
    }
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

/// サブツールの一覧の全体（上の帯・行・下の帯）。`area` は一覧の矩形。
fn list_section(ui: &mut Ui, app: &mut AppState, tool: Tool, area: Rect, shape: &ListShape) {
    let strip = Rect::from_min_size(area.min, vec2(area.width(), shape.strip));
    let body = Rect::from_min_max(
        pos2(area.left(), area.top() + shape.strip),
        pos2(area.right(), area.bottom() - shape.footer),
    );
    let footer = Rect::from_min_max(pos2(area.left(), area.bottom() - shape.footer), area.max);
    match tool.def().subtools {
        SubTools::Brushes => {
            brushes::group_strip(ui, strip, app);
            let group = brush_group(app);
            brushes::list_body(ui, app, body, group);
            brushes::footer(ui, app, footer, true);
        }
        SubTools::Erasers => {
            brushes::list_body(ui, app, body, Group::Eraser);
            // 取り込みのファイルを落とす先はブラシの一覧だけ
            app.brushes.ui.list_rect = None;
            brushes::footer(ui, app, footer, false);
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
    let ctx = ui.ctx().clone();
    app.brushes.samples.begin_frame(ctx.cumulative_pass_nr());
    let lang = app.lang;
    let tool = app.tool;
    let def = tool.def();
    app.subtools.ui.panel_right = r.right();
    app.subtool_follow(tool);

    // 取り込んだブラシが無くなったら、「取り込み」のタブは消えるので、ほかのグループへ戻す
    if app.brushes.ui.group == Group::Imported
        && !brushes::tab_groups(app).contains(&Group::Imported)
    {
        app.brushes.ui.group = Group::Pen;
        app.brushes.ui.list_scroll = 0.0;
    }
    let props_open = app.section_open("tool-props", true);
    let sized = def.sized;
    let size_open = sized && app.section_open("brush-sizes", true);
    let props_h = if props_open {
        app.subtools.ui.props_content[tool as usize]
    } else {
        0.0
    };
    let size_h = if size_open {
        brushes::sizes_height(r.width() - 8.0)
    } else {
        0.0
    };
    let fixed = SECTION_HEIGHT + props_h + if sized { SECTION_HEIGHT + size_h } else { 0.0 };
    let shape = list_shape(app, tool);
    let avail = (r.height() - fixed).max(0.0);
    // ブラシの一覧は余った高さを全部使う。ほかの一覧は行の数の高さ（ツールプロパティが長くても縮めない。全体がスクロールする）で、
    // 多くなったら一覧の中でスクロールする
    let list_h = if shape.fills {
        avail.max(shape.strip + MIN_LIST + shape.footer)
    } else {
        shape.strip + shape.body.min(MAX_ROWS * ROW) + shape.footer
    };
    let content = fixed + list_h;
    app.brushes.ui.panel_content = content;

    // 全体のスクロール（一覧の中でホイールを使い切れなければ、全体を送る）
    let max_scroll = (content - r.height()).max(0.0);
    let (list_content, list_view) = match def.subtools {
        SubTools::Brushes | SubTools::Erasers => (
            app.brushes.ui.list_content,
            list_h - shape.strip - shape.footer,
        ),
        _ => (
            app.subtools.ui.list_content,
            list_h - shape.strip - shape.footer,
        ),
    };
    let list_scrolls = list_content > list_view;
    let list_top_guess = r.top() - app.brushes.ui.panel_scroll + shape.strip;
    let list_rect_guess =
        Rect::from_min_size(pos2(r.left(), list_top_guess), vec2(r.width(), list_view));
    if ui.rect_contains_pointer(r) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        if ui.rect_contains_pointer(list_rect_guess) && list_scrolls {
            match def.subtools {
                SubTools::Brushes | SubTools::Erasers => app.brushes.ui.list_scroll -= wheel,
                _ => app.subtools.ui.list_scroll -= wheel,
            }
        } else {
            app.brushes.ui.panel_scroll -= wheel;
        }
    }
    app.brushes.ui.panel_scroll = app.brushes.ui.panel_scroll.clamp(0.0, max_scroll);
    let scroll = app.brushes.ui.panel_scroll;
    let bar = if max_scroll > 0.0 { 8.0 } else { 0.0 };
    let area = Rect::from_min_max(
        pos2(r.left(), r.top() - scroll),
        pos2(r.right() - bar, r.bottom()),
    );
    let outer = ui.clip_rect();
    ui.set_clip_rect(r.intersect(outer));

    let mut y = area.top();
    list_section(
        ui,
        app,
        tool,
        Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), list_h)),
        &shape,
    );
    y += list_h;
    // ツールプロパティ
    let open = brushes::section_band(
        ui,
        app,
        y,
        area,
        "tool-props",
        lang.pick("ツールプロパティ", "Tool Properties"),
        "tune",
    );
    y += SECTION_HEIGHT;
    if open {
        let mut rows = Rows::compact(
            Rect::from_min_max(
                pos2(area.left(), y),
                pos2(area.right(), area.bottom().max(y + 1.0)),
            ),
            0.0,
        );
        (def.properties)(ui, app, &mut rows, &ctx);
        rows.indent = 0.0;
        rows.space(6.0);
        // 中身の高さは次のフレームの組み立てに使う（変わったら、すぐ組み立て直す）
        if (rows.used() - app.subtools.ui.props_content[tool as usize]).abs() > 0.5 {
            ctx.request_repaint();
        }
        app.subtools.ui.props_content[tool as usize] = rows.used();
        y += rows.used();
    }
    // ブラシサイズ（大きさを持つ道具だけ）
    if sized {
        let open = brushes::section_band(
            ui,
            app,
            y,
            area,
            "brush-sizes",
            lang.pick("ブラシサイズ", "Brush Size"),
            "target",
        );
        y += SECTION_HEIGHT;
        if open {
            brushes::sizes_body(
                ui,
                app,
                Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), size_h)),
            );
        }
    }
    ui.set_clip_rect(outer);
    if max_scroll > 0.0 {
        let track = r.height();
        let bar_h = (track * track / content).max(16.0);
        let bar_y = r.top() + (track - bar_h) * scroll / max_scroll;
        w::rounded(
            ui.painter(),
            Rect::from_min_size(pos2(r.right() - 6.0, bar_y), vec2(4.0, bar_h)),
            t::CONTROL_ACTIVE,
            2.0,
        );
    }
}
