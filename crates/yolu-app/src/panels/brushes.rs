//! 左のドックのサブツールのパネルの、ブラシと消しゴムの部分（クリスタのサブツールの一覧・ツールプロパティ・ブラシサイズに当たる）。
//! パネルの組み立ては `subtools`。ここは、ブラシの一覧（グループのタブ・名前と、そのブラシの実際の設定で core が描いた見本のストロークの行・
//! 一覧の操作の帯。消しゴムの道具は消しゴムのグループだけを出す）、ブラシ・消しゴムのツールプロパティ（今の設定の見本と主な項目、右下の
//! 調整のボタンで詳細の窓）、ブラシサイズ（決まった大きさの丸）、オプションバーの項目（直径・不透明度・対称）を持つ。
//! 一覧の操作は `Action::Brush`（行を押して替える・追加・複製・削除・名前・並べ替え・元に戻す）。ブラシの設定は文書ではないので Undo に
//! 入れない。画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Color32, Rect, Sense, Ui, WidgetInfo, WidgetType};

use crate::brushes::sample::SampleSpec;
use crate::brushes::{BrushAction, BrushDrag, BrushKey, DropAt, Group};
use crate::engine::{Brush, BrushEffect};
use crate::m2_menu::Popup;
use crate::state::{Action, AppState};
use crate::ui::menu::context_anchor;
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, Rows, SliderSpec};

/// 決まった直径（px）。アプリの直径の上限（256）まで。
pub const SIZES: [u32; 15] = [1, 2, 3, 5, 8, 12, 16, 24, 32, 48, 64, 96, 128, 192, 256];

pub(super) const GROUP_STRIP: f32 = 30.0;
pub(super) const ROW_HEIGHT: f32 = 36.0;
pub(super) const FOOTER_HEIGHT: f32 = 28.0;
pub(super) const MIN_LIST: f32 = 96.0;
pub(super) const SECTION_HEIGHT: f32 = t::PANEL_HEADER_HEIGHT;
/// ツールプロパティの項目の行の高さと間。
const FIELD_HEIGHT: f32 = 20.0;
const FIELD_GAP: f32 = 3.0;
const TOOL_SAMPLE_HEIGHT: f32 = 44.0;
/// ブラシサイズの丸の 1 マスの最小の幅と、1 段の高さ。幅に入るだけ並べ、入りきらなければ段を足す。
const SIZE_CELL_MIN: f32 = 32.0;
const SIZE_ROW: f32 = 42.0;
const PEN_BUTTON: f32 = 24.0;
/// 大きさの数字（細い丸の幅に収める）。
const SIZE_LABEL: t::TextStyle = t::TextStyle {
    size: 9.0,
    bold: false,
    color: t::TEXT_DIM,
};

/// 今の直径に一番近い決まった大きさ（比で近いほう）の添字。
pub fn nearest_size(diameter: f32) -> usize {
    let d = diameter.max(0.5);
    SIZES
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let (da, db) = ((d / **a as f32).ln().abs(), (d / **b as f32).ln().abs());
            da.total_cmp(&db)
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// 今の設定のブラシ（手ぶれ補正・入り抜きも含む。見本に渡す）。
pub(super) fn live_brush(app: &AppState) -> Brush {
    let mut brush = app.m2.brush.clone();
    brush.base = app.brush.settings([1.0; 4], false);
    brush
}

/// 白い紙の上の見本のストローク（まだ描けていなければ紙だけ。描くのは次のフレーム）。
pub(super) fn paint_sample(
    ui: &mut Ui,
    app: &mut AppState,
    rect: Rect,
    brush: &Brush,
    spec: SampleSpec,
) {
    w::rounded(ui.painter(), rect, Color32::WHITE, 3.0);
    if !ui.is_rect_visible(rect) {
        return;
    }
    // 画像の縦横比を保って、紙の真ん中に置く（紙は白なので、余りは見えない）
    let inner = rect.shrink(1.0);
    let aspect = spec.width as f32 / spec.height as f32;
    let fitted = if inner.width() / inner.height() > aspect {
        Rect::from_center_size(
            inner.center(),
            vec2(inner.height() * aspect, inner.height()),
        )
    } else {
        Rect::from_center_size(inner.center(), vec2(inner.width(), inner.width() / aspect))
    };
    match app.brushes.samples.request(brush, spec) {
        Some(key) => {
            if let Some(texture) = app.brushes.samples.texture(ui.ctx(), key) {
                ui.painter().image(
                    texture,
                    fitted,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        None => {
            // 別のスレッドで描いている最中なら、絵ができたときに描き直しが来る
            if app.brushes.samples.needs_next_frame() {
                ui.ctx().request_repaint();
            }
        }
    }
}

pub(super) fn is_eraser(app: &AppState) -> bool {
    app.brushes
        .lib
        .entry(app.brushes.lib.current())
        .is_some_and(|e| e.group.is_eraser())
}

/// ブラシの道具のタブに出すグループ（組み込みのあるグループはいつも。取り込んだブラシが 1 つでもあれば「取り込み」も）。消しゴムのグループは
/// 消しゴムの道具の一覧なので出さない（ブラシと消しゴムを同じ一覧に並べない）。
pub fn tab_groups(app: &AppState) -> Vec<Group> {
    let mut groups: Vec<Group> = Group::ALL
        .iter()
        .copied()
        .filter(|g| !g.is_eraser())
        .collect();
    if app
        .brushes
        .lib
        .entries()
        .iter()
        .any(|e| e.group == Group::Imported)
    {
        groups.push(Group::Imported);
    }
    groups
}

/// グループのタブ（短い名前。全名はツールチップ。入りきらなければ名前を詰め、それでも入らなければアイコンだけ）。
pub(super) fn group_strip(ui: &mut Ui, r: Rect, app: &mut AppState) {
    let lang = app.lang;
    {
        let p = ui.painter();
        w::fill(p, r, t::PANEL_HEADER);
        w::hline(p, r.left(), r.right(), r.bottom() - 1.0, t::BORDER);
    }
    let groups = tab_groups(app);
    let n = groups.len();
    let tab_w = (r.width() - 4.0) / n as f32;
    let widest = groups
        .iter()
        .map(|g| w::text_width(ui.painter(), g.short(lang), t::HEADER))
        .fold(0.0f32, f32::max);
    let text_only = tab_w.round() - 1.0 >= widest + 8.0;
    for (i, group) in groups.iter().enumerate() {
        let tab = Rect::from_min_size(
            pos2((r.left() + 2.0 + i as f32 * tab_w).round(), r.top() + 2.0),
            vec2(tab_w.round() - 1.0, r.height() - 3.0),
        );
        let on = app.brushes.ui.group == *group;
        let response = ui.interact(
            tab,
            ui.make_persistent_id(("brush.group", i)),
            Sense::click(),
        );
        if response.clicked() {
            app.brushes.ui.group = *group;
            app.brushes.ui.list_scroll = 0.0;
        }
        let p = ui.painter();
        if on {
            w::rounded(p, tab, t::PANEL_BG, 3.0);
            w::fill(
                p,
                Rect::from_min_size(
                    pos2(tab.left() + 4.0, tab.bottom() - 2.0),
                    vec2(tab.width() - 8.0, 2.0),
                ),
                t::ACCENT,
            );
        } else if response.hovered() || response.is_pointer_button_down_on() {
            w::rounded(p, tab, t::CONTROL_HOVER, 3.0);
        }
        let color = if on { Color32::WHITE } else { t::TEXT_DIM };
        if text_only {
            let shown = w::fit(p, group.short(lang), tab.width() - 6.0, t::HEADER);
            w::text(p, tab, &shown, t::HEADER.with_color(color), Align::Center);
        } else {
            w::icon(p, tab, group.icon(), color, 15.0);
        }
        let full = group.name(lang);
        response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, on, full));
        let _ = response.on_hover_text(full);
    }
}

/// ドラッグの落とす先（一覧の中の高さから。行の間のいちばん近い所）。
fn drop_target(list_keys: &[BrushKey], position: f32) -> DropAt {
    let at = (position + 0.5).floor().clamp(0.0, list_keys.len() as f32) as usize;
    match list_keys.get(at) {
        Some(key) => DropAt::Before(*key),
        None => DropAt::End,
    }
}

/// ブラシの一覧の中（行・ドラッグの追従・スクロール・空白）。`group` は出すグループ（消しゴムの道具は消しゴムのグループ）。
pub(super) fn list_body(ui: &mut Ui, app: &mut AppState, list: Rect, group: Group) {
    let lang = app.lang;
    let live = app.brush_live();
    let current = app.brushes.lib.current();
    let rows: Vec<(BrushKey, String, bool, Group)> = app
        .brushes
        .lib
        .in_group(group)
        .into_iter()
        .map(|e| {
            (
                e.key,
                e.name_in(lang),
                app.brushes.lib.is_modified(e.key, &live),
                e.group,
            )
        })
        .collect();
    let keys: Vec<BrushKey> = rows.iter().map(|r| r.0).collect();
    // 名前を変えているブラシが、見ている一覧に無くなったら（グループを替えた）やめる
    if app.brushes.ui.renaming.is_some_and(|k| !keys.contains(&k)) {
        app.brushes.ui.renaming = None;
    }
    w::fill(ui.painter(), list, t::CONTROL_BG);
    app.brushes.ui.list_rect = Some(list);
    let content = rows.len() as f32 * ROW_HEIGHT;
    app.brushes.ui.list_content = content;
    // ブラシが替わったあとは、今のブラシの行が見えるところまで送る
    if std::mem::take(&mut app.brushes.ui.reveal) {
        if let Some(i) = keys.iter().position(|k| *k == current) {
            let top = i as f32 * ROW_HEIGHT;
            let scroll = &mut app.brushes.ui.list_scroll;
            if top < *scroll {
                *scroll = top;
            } else if top + ROW_HEIGHT > *scroll + list.height() {
                *scroll = top + ROW_HEIGHT - list.height();
            }
        }
    }
    // ホイールは、このパネルを包む `subtools` が（全体のスクロールと分けて）受ける
    let bar = Scroll::new(list, content, &mut app.brushes.ui.list_scroll);
    // ドラッグ: ボタンを押しているあいだは落とす先をポインタに追わせ、離したら落とす（行がスクロールで見えなくなっても）
    if let Some(drag) = app.brushes.ui.drag {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            // Esc: 動かすのをやめる（落とさない）
            app.brushes.ui.drag = None;
        } else if ui.input(|i| i.pointer.primary_down()) {
            if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
                let position = (p.y - list.top() + app.brushes.ui.list_scroll) / ROW_HEIGHT;
                app.brushes.ui.drag = Some(BrushDrag {
                    target: Some(drop_target(&keys, position)),
                    ..drag
                });
            }
        } else {
            app.brushes.ui.drag = None;
            if let Some(at) = drag.target {
                app.apply(Action::Brush(BrushAction::Move { key: drag.key, at }));
            }
        }
    }
    let scroll = app.brushes.ui.list_scroll;
    let row_width = list.width() - bar.reserved();
    let outer = ui.clip_rect();
    ui.set_clip_rect(list.intersect(outer));
    for (i, (key, name, modified, row_group)) in rows.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + i as f32 * ROW_HEIGHT - scroll),
            vec2(row_width, ROW_HEIGHT),
        );
        if row.bottom() < list.top() || row.top() > list.bottom() {
            continue;
        }
        brush_row(
            ui,
            app,
            row,
            (*key, name, *modified, *row_group),
            *key == current,
        );
    }
    // ドラッグの落とす先の線
    if let Some(BrushDrag {
        target: Some(at), ..
    }) = app.brushes.ui.drag
    {
        let index = match at {
            DropAt::Before(key) => keys.iter().position(|k| *k == key).unwrap_or(keys.len()),
            DropAt::End => keys.len(),
        };
        let y = list.top() + index as f32 * ROW_HEIGHT - scroll;
        w::fill(
            ui.painter(),
            Rect::from_min_size(pos2(list.left(), y - 1.0), vec2(row_width, 2.0)),
            t::ACCENT,
        );
    }
    ui.set_clip_rect(outer);
    bar.end(ui, "brushes.list.scroll", &mut app.brushes.ui.list_scroll);
}

fn brush_row(
    ui: &mut Ui,
    app: &mut AppState,
    row: Rect,
    (key, name, modified, group): (BrushKey, &str, bool, Group),
    selected: bool,
) {
    let lang = app.lang;
    let import = app.brushes.lib.entry(key).and_then(|e| e.import.clone());
    let gaps: Vec<crate::brushes::Gap> = import.iter().flat_map(|m| m.gaps.clone()).collect();
    let clip = ui.clip_rect();
    let hit = row.intersect(clip);
    let response = ui.interact(
        hit,
        ui.make_persistent_id(("brush.row", key)),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(clip);
    if selected {
        w::fill(&painter, row, t::ACCENT_SOFT);
        w::fill(
            &painter,
            Rect::from_min_size(row.min, vec2(3.0, row.height())),
            t::ACCENT,
        );
    } else if response.hovered() {
        w::fill(&painter, row, t::CONTROL_HOVER);
    }
    if app.brushes.ui.drag.is_some_and(|d| d.key == key) {
        w::fill(&painter, row, t::CONTROL_ACTIVE);
    }
    w::hline(
        &painter,
        row.left(),
        row.right(),
        row.bottom() - 1.0,
        t::BORDER,
    );

    let name_width = (row.width() * 0.52).clamp(80.0, 140.0);
    let name_rect = Rect::from_min_size(
        pos2(row.left() + 10.0, row.top() + 3.0),
        vec2(name_width, row.height() - 6.0),
    );
    let sample_rect = Rect::from_min_max(
        pos2(name_rect.right() + 6.0, row.top() + 4.0),
        pos2(row.right() - 6.0, row.bottom() - 5.0),
    );

    if response.clicked() {
        app.apply(Action::Brush(BrushAction::Select(key)));
        if app.brushes.ui.renaming != Some(key) {
            app.brushes.ui.renaming = None;
        }
    }
    if (response.double_clicked() || response.triple_clicked()) && key.is_user() {
        app.apply(Action::Brush(BrushAction::StartRename(key)));
    }
    if response.drag_started() {
        app.brushes.ui.drag = Some(BrushDrag { key, target: None });
    }
    if response.secondary_clicked() {
        if let Some(at) = response.interact_pointer_pos() {
            app.brushes.ui.context = Some(key);
            super::properties::open_popup(
                app,
                ui.ctx(),
                Popup::BrushContext,
                context_anchor(at),
                0.0,
            );
        }
    }

    // 名前（利用者のブラシはダブルクリックで変える）と、変更ありの印
    if app.brushes.ui.renaming == Some(key) {
        let first = !app.brushes.ui.rename_started;
        app.brushes.ui.rename_started = true;
        let out = w::text_field(ui, name_rect, ("brush.rename", key), name, None, first);
        if let Some(next) = out.committed {
            app.apply(Action::Brush(BrushAction::Rename(key, next)));
        }
        if !first && !out.focused {
            app.brushes.ui.renaming = None;
        }
    } else {
        let dot = if modified { 12.0 } else { 0.0 };
        let mark = if gaps.is_empty() { 0.0 } else { 16.0 };
        let shown = w::fit(&painter, name, name_rect.width() - dot - mark, t::LABEL);
        let text_color = if selected { Color32::WHITE } else { t::TEXT };
        w::text(
            &painter,
            name_rect,
            &shown,
            t::LABEL.with_color(text_color),
            Align::Left,
        );
        let mut x = name_rect.left() + w::text_width(&painter, &shown, t::LABEL) + 8.0;
        if modified {
            painter.circle_filled(pos2(x, name_rect.center().y), 3.0, t::WARNING);
            x += 12.0;
        }
        // 取り込んだときに表せなかった項目がある印（項目の名前はツールチップ）
        if !gaps.is_empty() {
            w::icon(
                &painter,
                Rect::from_center_size(pos2(x + 2.0, name_rect.center().y), vec2(14.0, 14.0)),
                "warning",
                t::WARNING,
                13.0,
            );
        }
    }

    // 見本のストローク
    let brush = {
        let live = live_brush(app);
        if selected {
            live
        } else {
            let entry = app.brushes.lib.entry(key);
            let effective = entry.map(|e| e.effective().clone()).unwrap_or_default();
            // 入り抜き・手ぶれ補正を持つブラシは、見本も持つ値で描く（持たないブラシは、選んでいるブラシによらず描き手の設定）
            let assist = app.brush_row_assist(entry.and_then(|e| e.assist));
            Brush {
                assist,
                ..effective
            }
        }
    };
    paint_sample(
        ui,
        app,
        sample_rect,
        &brush,
        SampleSpec::row(group.is_eraser()),
    );

    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, name));
    let tooltip = crate::brushes::gaps::row_tooltip(lang, name, modified, import.as_ref());
    let _ = response.on_hover_text(tooltip);
}

/// 一覧の下の帯: 元に戻す・複製・取り込み・追加・削除（右寄せ）。`with_import` が偽なら取り込みは出さない（消しゴムの一覧）。
pub(super) fn footer(ui: &mut Ui, app: &mut AppState, bar: Rect, with_import: bool) {
    let lang = app.lang;
    w::fill(ui.painter(), bar, t::PANEL_HEADER);
    w::hline(ui.painter(), bar.left(), bar.right(), bar.top(), t::BORDER);
    let current = app.brushes.lib.current();
    let user = current.is_user();
    let modified = app.brush_is_modified(current);
    let free = !app.is_stroking();
    let button =
        |x: f32| Rect::from_min_size(pos2(x, bar.top() + 2.0), vec2(26.0, bar.height() - 4.0));
    let mut x = bar.right() - 4.0 - 26.0;
    if w::icon_button(
        ui,
        button(x),
        "brush.delete",
        "delete",
        if user {
            lang.pick("ブラシを削除", "Delete Brush")
        } else {
            lang.pick(
                "組み込みのブラシは消せません",
                "Built-in brushes cannot be deleted",
            )
        },
        false,
        free && user,
        17.0,
    )
    .clicked()
    {
        app.apply(Action::Brush(BrushAction::Delete(current)));
    }
    x -= 28.0;
    if w::icon_button(
        ui,
        button(x),
        "brush.add",
        "add",
        lang.pick(
            "今の設定を新しいブラシに",
            "Add the Current Settings as a Brush",
        ),
        false,
        free,
        18.0,
    )
    .clicked()
    {
        app.apply(Action::Brush(BrushAction::Add));
    }
    x -= 28.0;
    if with_import
        && w::icon_button(
            ui,
            button(x),
            "brush.import",
            "import",
            lang.pick(
                "ブラシを取り込む（ABR・GBR・GIH・VBR・PNG・PAT）",
                "Import brushes (ABR, GBR, GIH, VBR, PNG, PAT)",
            ),
            app.brushes.import.is_busy(),
            !app.brushes.import.is_busy(),
            17.0,
        )
        .clicked()
    {
        app.apply(Action::Brush(BrushAction::ImportDialog));
    }
    if with_import {
        x -= 28.0;
        if w::icon_button(
            ui,
            button(x),
            "brush.import_csp",
            "folder_open",
            lang.pick("CLIP STUDIO から取り込む", "Import from CLIP STUDIO"),
            app.brushes.csp.open,
            !app.brushes.import.is_busy(),
            17.0,
        )
        .clicked()
        {
            app.apply(Action::Brush(BrushAction::ClipStudioOpen));
        }
        x -= 28.0;
    }
    if w::icon_button(
        ui,
        button(x),
        "brush.duplicate",
        "content_copy",
        lang.pick("ブラシを複製", "Duplicate Brush"),
        false,
        free,
        16.0,
    )
    .clicked()
    {
        app.apply(Action::Brush(BrushAction::Duplicate(current)));
    }
    x -= 28.0;
    if w::icon_button(
        ui,
        button(x),
        "brush.revert",
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
        app.apply(Action::Brush(BrushAction::Revert(current)));
    }
}

/// ツールプロパティの項目の行の数（効果のブラシなら 1 行増える）。
pub(super) fn tool_fields(app: &AppState) -> usize {
    6 + matches!(
        app.m2.brush.effect,
        BrushEffect::Blur { .. } | BrushEffect::Smudge { .. }
    ) as usize
}

pub(super) fn tool_body_height(app: &AppState) -> f32 {
    4.0 + TOOL_SAMPLE_HEIGHT + 6.0 + tool_fields(app) as f32 * (FIELD_HEIGHT + FIELD_GAP) + 4.0
}

/// 1 行のスライダー（右に筆圧のボタンを置く行は `pen` を渡す）。変わった値を返す。
#[allow(clippy::too_many_arguments)]
fn field(
    ui: &mut Ui,
    row: Rect,
    id: &str,
    spec: SliderSpec,
    value: f32,
    pen: Option<(&mut bool, &str)>,
    editable: bool,
) -> Option<f32> {
    let slider_rect = match &pen {
        Some(_) => Rect::from_min_max(row.min, pos2(row.right() - PEN_BUTTON - 4.0, row.bottom())),
        None => row,
    };
    let out = w::slider(ui, slider_rect, id, value, &spec.enabled(editable));
    if let Some((flag, tooltip)) = pen {
        let button = Rect::from_min_size(
            pos2(row.right() - PEN_BUTTON, row.top()),
            vec2(PEN_BUTTON, row.height()),
        );
        if w::icon_button(
            ui,
            button,
            (id, "pen"),
            "stylus",
            tooltip,
            *flag,
            editable,
            15.0,
        )
        .clicked()
        {
            *flag = !*flag;
        }
    }
    out.changed.then_some(out.value)
}

pub(super) fn tool_body(ui: &mut Ui, app: &mut AppState, area: Rect) {
    let lang = app.lang;
    let editable = !app.is_stroking();
    let in_3d = app.view3d.paintable_on_screen();
    // 今の設定の見本
    let sample = Rect::from_min_size(
        pos2(area.left() + t::PADDING, area.top() + 4.0),
        vec2(area.width() - 2.0 * t::PADDING, TOOL_SAMPLE_HEIGHT),
    );
    let live = live_brush(app);
    paint_sample(ui, app, sample, &live, SampleSpec::tool(is_eraser(app)));
    let mut y = sample.bottom() + 6.0;
    let mut next = || {
        let r = Rect::from_min_size(
            pos2(area.left() + t::PADDING, y),
            vec2(area.width() - 2.0 * t::PADDING, FIELD_HEIGHT),
        );
        y += FIELD_HEIGHT + FIELD_GAP;
        r
    };
    let hardness_applies = super::brush_props::hardness_applies(app);
    let b = &mut app.brush;
    let size_tip = lang.pick("ブラシの直径（[ と ]）", "Brush diameter ([ and ])");
    if let Some(v) = field(
        ui,
        next(),
        "tool.size",
        SliderSpec::new(
            lang.pick("直径", "Size"),
            1.0,
            256.0,
            NumberFormat::int(" px"),
        )
        .tooltip(size_tip),
        b.radius * 2.0,
        Some((
            &mut b.pressure_size,
            lang.pick("筆圧で直径を変える", "Pen pressure changes the size"),
        )),
        editable,
    ) {
        b.radius = (v / 2.0).max(0.5);
    }
    if let Some(v) = field(
        ui,
        next(),
        "tool.opacity",
        SliderSpec::new(
            lang.pick("不透明度", "Opacity"),
            0.0,
            100.0,
            NumberFormat::int("%"),
        )
        .tooltip(lang.pick(
            "1 本のストロークが覆える上限",
            "The most one stroke can cover",
        )),
        b.opacity * 100.0,
        Some((
            &mut b.pressure_opacity,
            lang.pick("筆圧で不透明度を変える", "Pen pressure changes the opacity"),
        )),
        editable,
    ) {
        b.opacity = v / 100.0;
    }
    if let Some(v) = field(
        ui,
        next(),
        "tool.hardness",
        SliderSpec::new(
            lang.pick("硬さ", "Hardness"),
            0.0,
            100.0,
            NumberFormat::int("%"),
        )
        .tooltip(lang.pick(
            "丸い筆先の縁の硬さ（画像の筆先は画像の縁のまま）",
            "Edge hardness of the round tip (an image keeps its own edge)",
        )),
        b.hardness * 100.0,
        Some((
            &mut app.m2.brush.controls.pressure_hardness,
            lang.pick("筆圧で硬さを変える", "Pen pressure changes the hardness"),
        )),
        editable && hardness_applies,
    ) {
        b.hardness = v / 100.0;
    }
    if let Some(v) = field(
        ui,
        next(),
        "tool.flow",
        SliderSpec::new(
            lang.pick("流量", "Flow"),
            0.0,
            100.0,
            NumberFormat::int("%"),
        )
        .tooltip(lang.pick("ダブ 1 つが足す量", "How much each dab adds")),
        b.flow * 100.0,
        Some((
            &mut b.pressure_flow,
            lang.pick("筆圧で流量を変える", "Pen pressure changes the flow"),
        )),
        editable,
    ) {
        b.flow = v / 100.0;
    }
    if let Some(v) = field(
        ui,
        next(),
        "tool.spacing",
        SliderSpec::new(
            lang.pick("間隔", "Spacing"),
            1.0,
            100.0,
            NumberFormat::int("%"),
        )
        .tooltip(lang.pick(
            "ダブの間隔（直径に対する割合）",
            "Distance between dabs (of the diameter)",
        )),
        b.spacing * 100.0,
        None,
        editable,
    ) {
        b.spacing = v / 100.0;
    }
    let off = in_3d.then(|| lang.pick("3D では効きません", "No effect in 3D"));
    let stabilizer = app.m2.brush.assist.stabilizer as f32;
    // 手ぶれ補正の行の右端に、詳細の窓のボタン
    let row = next();
    let detail = Rect::from_min_size(
        pos2(row.right() - PEN_BUTTON, row.top()),
        vec2(PEN_BUTTON, row.height()),
    );
    if let Some(v) = field(
        ui,
        Rect::from_min_max(row.min, pos2(detail.left() - 4.0, row.bottom())),
        "tool.stabilizer",
        SliderSpec::new(
            lang.pick("手ぶれ補正", "Stabilizer"),
            0.0,
            200.0,
            NumberFormat::int(" px"),
        )
        .tooltip(off.unwrap_or(lang.pick(
            "筆が入力に引かれる糸の長さ。0 で切",
            "Length of the string that pulls the brush. 0 = off",
        ))),
        stabilizer,
        None,
        editable && off.is_none(),
    ) {
        app.m2.brush.assist.stabilizer = v as f64;
    }
    let open = app.brushes.ui.detail.open;
    if w::icon_button(
        ui,
        detail,
        "tool.detail",
        "tune",
        lang.pick("ブラシの詳細", "Brush Details"),
        open,
        true,
        16.0,
    )
    .clicked()
    {
        app.brushes.ui.detail.open = !open;
    }
    // 効果のブラシの主な値
    let tool_is_eraser = app.tool.erases();
    let effect_off = if tool_is_eraser {
        Some(lang.pick("消しゴムでは使えません", "Not available with the eraser"))
    } else if in_3d {
        Some(lang.pick("3D では使えません", "Not available in 3D"))
    } else {
        None
    };
    match app.m2.brush.effect {
        BrushEffect::Blur { radius } => {
            if let Some(v) = field(
                ui,
                next(),
                "tool.blur",
                SliderSpec::new(
                    lang.pick("ぼかしの半径", "Blur radius"),
                    1.0,
                    64.0,
                    NumberFormat::int(" px"),
                )
                .tooltip(effect_off.unwrap_or(lang.pick("ぼかす範囲の半径", "Radius of the blur"))),
                radius as f32,
                None,
                editable && effect_off.is_none(),
            ) {
                app.m2.brush.effect = BrushEffect::Blur {
                    radius: v.round().clamp(1.0, 64.0) as u32,
                };
            }
        }
        BrushEffect::Smudge { strength } => {
            if let Some(v) = field(
                ui,
                next(),
                "tool.smudge",
                SliderSpec::new(
                    lang.pick("指先の強さ", "Smudge strength"),
                    0.0,
                    100.0,
                    NumberFormat::int("%"),
                )
                .tooltip(
                    effect_off.unwrap_or(lang.pick("流量に掛ける強さ", "Multiplies the flow")),
                ),
                strength as f32 * 100.0,
                None,
                editable && effect_off.is_none(),
            ) {
                app.m2.brush.effect = BrushEffect::Smudge {
                    strength: (v as f64 / 100.0).clamp(0.0, 1.0),
                };
            }
        }
        _ => {}
    }
}

/// ブラシ・消しゴムのツールプロパティ（今の設定の見本と主な項目。欄の中身は `tool_body`）。
pub fn props(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    let area = rows.full_row(tool_body_height(app), 0.0);
    tool_body(ui, app, area);
}

/// オプションバー（ブラシ・消しゴム）: 定規へのスナップ（入っているかが見える）、直径と不透明度（ツールプロパティと同じ値）、右端の対称。
/// 硬さ・流量・間隔・筆圧はツールプロパティ。
pub fn options(ui: &mut Ui, app: &mut AppState, r: Rect, x: f32) {
    let (y, h) = (r.top() + 6.0, r.height() - 12.0);
    let mut x = x + 4.0;
    let mut next = |width: f32| {
        let at = Rect::from_min_size(pos2(x, y), vec2(width, h));
        x += width + 8.0;
        at
    };
    let l = app.lang;
    let editable = !app.is_stroking();
    crate::drafting::props::snap_button(ui, app, next(28.0));
    let b = &mut app.brush;
    let out = w::slider(
        ui,
        next(150.0),
        "options.size",
        b.radius * 2.0,
        &SliderSpec::new(l.pick("直径", "Size"), 1.0, 256.0, NumberFormat::int(" px"))
            .tooltip(l.pick("ブラシの直径（[ と ]）", "Brush diameter ([ and ])"))
            .enabled(editable),
    );
    if out.changed {
        b.radius = (out.value / 2.0).max(0.5);
    }
    let out = w::slider(
        ui,
        next(130.0),
        "options.opacity",
        b.opacity * 100.0,
        &SliderSpec::new(
            l.pick("不透明度", "Opacity"),
            0.0,
            100.0,
            NumberFormat::int("%"),
        )
        .enabled(editable),
    );
    if out.changed {
        b.opacity = out.value / 100.0;
    }
    // 対称（右端。左の部品に重なるほど狭ければ出さない）
    crate::selection::props::symmetry_options(ui, app, r, x);
}

/// ブラシサイズの格子の列の数（欄の幅に入るだけ。1 列以上、全部の数まで）。
pub(super) fn size_columns(width: f32) -> usize {
    (((width - 2.0 * t::PADDING) / SIZE_CELL_MIN)
        .floor()
        .max(1.0) as usize)
        .min(SIZES.len())
}

/// ブラシサイズの格子の高さ（段の数 × 段の高さと上下の余白）。
pub(super) fn sizes_height(width: f32) -> f32 {
    let rows = SIZES.len().div_ceil(size_columns(width));
    rows as f32 * SIZE_ROW + 8.0
}

/// ブラシサイズの格子: 決まった大きさの丸（押すと直径を替える。今の直径に近い丸に印）。欄の幅に入るだけ並べ、入りきらなければ段を足す。
pub(super) fn sizes_body(ui: &mut Ui, app: &mut AppState, area: Rect) {
    let lang = app.lang;
    let editable = !app.is_stroking();
    let diameter = app.brush.radius * 2.0;
    let near = nearest_size(diameter);
    let n = SIZES.len();
    let columns = size_columns(area.width());
    let inner = Rect::from_min_size(
        pos2(area.left() + t::PADDING, area.top() + 4.0),
        vec2(area.width() - 2.0 * t::PADDING, area.height() - 8.0),
    );
    let cell_w = inner.width() / columns as f32;
    for (i, size) in SIZES.iter().enumerate() {
        let (row, column) = (i / columns, i % columns);
        let cell = Rect::from_min_size(
            pos2(
                inner.left() + column as f32 * cell_w,
                inner.top() + row as f32 * SIZE_ROW,
            ),
            vec2(cell_w, SIZE_ROW),
        );
        let label = lang.pick(format!("{size} px"), format!("{size} px"));
        let response = ui.interact(
            cell,
            ui.make_persistent_id(("brush.size-cell", *size)),
            if editable {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let on = i == near;
        let p = ui.painter();
        if on {
            w::rounded(p, cell.shrink2(vec2(0.5, 0.0)), t::ACCENT_SOFT, 3.0);
        } else if response.hovered() && editable {
            w::rounded(p, cell.shrink2(vec2(0.5, 0.0)), t::CONTROL_HOVER, 3.0);
        }
        // 丸は大きさの対数で 2〜16 点
        let draw = 2.0 + 14.0 * (*size as f32).log2() / (SIZES[n - 1] as f32).log2();
        let center = pos2(cell.center().x, cell.top() + 15.0);
        p.circle_filled(
            center,
            draw / 2.0,
            if on {
                t::ACCENT
            } else if response.hovered() {
                Color32::WHITE
            } else {
                t::TEXT_DIM
            },
        );
        {
            w::text(
                p,
                Rect::from_min_size(
                    pos2(cell.left() - 3.0, cell.top() + 26.0),
                    vec2(cell.width() + 6.0, 12.0),
                ),
                &size.to_string(),
                SIZE_LABEL.with_color(if on { Color32::WHITE } else { t::TEXT_DIM }),
                Align::Center,
            );
        }
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::SelectableLabel, editable, on, &label)
        });
        if response.clicked() {
            app.brush.radius = (*size as f32 / 2.0).max(0.5);
        }
        let _ = response.on_hover_text(label);
    }
}

/// 見出しの帯（折りたためる。開閉は `AppState::sections` が覚える）。
pub(super) fn section_band(
    ui: &mut Ui,
    app: &mut AppState,
    y: f32,
    r: Rect,
    key: &'static str,
    title: &str,
    icon: &str,
) -> bool {
    let open = app.section_open(key, true);
    let header = Rect::from_min_size(pos2(r.left(), y), vec2(r.width(), SECTION_HEIGHT));
    let out = w::section_header(
        ui,
        header,
        ("brush.section", key),
        title,
        open,
        Some(icon),
        None,
    );
    if out.open != open {
        app.sections.insert(key, out.open);
    }
    out.open
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_size_nearest_to_the_diameter_is_marked() {
        assert_eq!(SIZES[nearest_size(16.0)], 16);
        assert_eq!(SIZES[nearest_size(1.0)], 1);
        assert_eq!(SIZES[nearest_size(256.0)], 256);
        assert_eq!(SIZES[nearest_size(300.0)], 256);
        assert_eq!(SIZES[nearest_size(0.2)], 1);
        // 比で近いほう（10 は 8 と 12 の間。12 のほうが比で近い）
        assert_eq!(SIZES[nearest_size(10.0)], 12);
        assert_eq!(SIZES[nearest_size(35.0)], 32);
        assert_eq!(SIZES[nearest_size(45.0)], 48);
        for (i, s) in SIZES.iter().enumerate() {
            assert_eq!(nearest_size(*s as f32), i);
        }
    }

    #[test]
    fn drop_targets_are_the_nearest_gap() {
        let keys = [BrushKey::User(1), BrushKey::User(2), BrushKey::User(3)];
        assert_eq!(drop_target(&keys, -3.0), DropAt::Before(keys[0]));
        assert_eq!(drop_target(&keys, 0.4), DropAt::Before(keys[0]));
        assert_eq!(drop_target(&keys, 0.6), DropAt::Before(keys[1]));
        assert_eq!(drop_target(&keys, 2.4), DropAt::Before(keys[2]));
        assert_eq!(drop_target(&keys, 2.6), DropAt::End);
        assert_eq!(drop_target(&keys, 99.0), DropAt::End);
        assert_eq!(drop_target(&[], 0.0), DropAt::End);
    }
}
