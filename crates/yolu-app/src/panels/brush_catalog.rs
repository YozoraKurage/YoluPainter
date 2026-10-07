//! 「＋」の窓（ブラシを追加）: 左に種類（組み込み・同梱の Krita・Photoshop・CLIP STUDIO・そのほかの取り込み・自分のブラシ）、上に名前の
//! 検索、行は名前と見本のストローク（並びにある物は印）。押して選び（複数）、下の「追加」で今のグループの後ろへ置く（並びにある物と
//! 同梱の Krita は写しのファイルを作る）。利用者のブラシのファイルは右クリックで消せる（消す前に確かめる）。浮いた窓で、開いたまま描ける。
//! 画面には名前だけを出し、説明はツールチップ。中身（行・選び）は `toolset::catalog`。

use egui::{pos2, vec2, Color32, Id, Rect, Sense, Ui, Vec2, WidgetInfo, WidgetType};

use super::brushes::paint_sample;
use crate::brushes::sample::SampleSpec;
use crate::brushes::BrushAction;
use crate::m2_menu::Popup;
use crate::state::{Action, AppState};
use crate::toolset::catalog::{self, CatalogItem, Kind, Row};
use crate::ui::menu::{context_anchor, Entry};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::ui::window::{self, Spec};

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 480.0;
const SIDEBAR: f32 = 170.0;
const KIND_HEIGHT: f32 = 30.0;
const ROW_HEIGHT: f32 = 40.0;
const SEARCH_HEIGHT: f32 = 36.0;
const FOOTER: f32 = 44.0;

/// 窓の名前（試験が窓の矩形を引く）。
pub fn id() -> Id {
    Id::new(("yolu.window", "brush-catalog"))
}

fn sidebar(ui: &mut Ui, app: &mut AppState, pane: Rect) {
    let lang = app.lang;
    w::fill(ui.painter(), pane, t::PANEL_HEADER);
    w::vline(
        ui.painter(),
        pane.right() - 1.0,
        pane.top(),
        pane.bottom(),
        t::BORDER,
    );
    let current = app.toolset.catalog.kind;
    for (i, kind) in Kind::ALL.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(pane.left(), pane.top() + 6.0 + i as f32 * KIND_HEIGHT),
            vec2(pane.width() - 1.0, KIND_HEIGHT),
        );
        let name = kind.name(lang);
        let response = ui.interact(
            row,
            ui.make_persistent_id(("brush.catalog.kind", i)),
            Sense::click(),
        );
        let on = *kind == current;
        let p = ui.painter();
        if on {
            w::fill(p, row, t::ACCENT_SOFT);
            w::fill(
                p,
                Rect::from_min_size(row.min, vec2(3.0, row.height())),
                t::ACCENT,
            );
        } else if response.hovered() {
            w::fill(p, row, t::CONTROL_HOVER);
        }
        w::icon(
            p,
            Rect::from_min_size(pos2(row.left() + 8.0, row.top()), vec2(20.0, row.height())),
            kind.icon(),
            if on { Color32::WHITE } else { t::TEXT_DIM },
            15.0,
        );
        let shown = w::fit(p, name, row.width() - 40.0, t::LABEL);
        w::text(
            p,
            Rect::from_min_max(pos2(row.left() + 34.0, row.top()), row.max),
            &shown,
            t::LABEL.with_color(if on { Color32::WHITE } else { t::TEXT }),
            Align::Left,
        );
        response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, on, name));
        if response.clicked() && !on {
            app.toolset.catalog.kind = *kind;
            app.toolset.catalog.scroll = 0.0;
        }
    }
}

fn row_view(ui: &mut Ui, app: &mut AppState, row: Rect, r: &Row) {
    let lang = app.lang;
    let clip = ui.clip_rect();
    let response = ui.interact(
        row.intersect(clip),
        ui.make_persistent_id(("brush.catalog.row", r.item)),
        Sense::click(),
    );
    let selected = app.toolset.catalog.is_selected(r.item);
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
    w::hline(
        &painter,
        row.left(),
        row.right(),
        row.bottom() - 1.0,
        t::BORDER,
    );
    // 選ぶ印
    let mark = Rect::from_min_size(
        pos2(row.left() + 8.0, row.center().y - 8.0),
        vec2(16.0, 16.0),
    );
    w::outline(
        &painter,
        mark,
        if selected { t::ACCENT } else { t::BORDER },
        1.0,
        3.0,
    );
    if selected {
        w::icon(&painter, mark, "check", Color32::WHITE, 13.0);
    }
    let name_width = (row.width() * 0.42).clamp(90.0, 170.0);
    let name_rect = Rect::from_min_size(
        pos2(mark.right() + 8.0, row.top() + 3.0),
        vec2(name_width, row.height() - 6.0),
    );
    let placed_w = if r.placed { 18.0 } else { 0.0 };
    let shown = w::fit(&painter, &r.name, name_rect.width() - placed_w, t::LABEL);
    w::text(
        &painter,
        name_rect,
        &shown,
        t::LABEL.with_color(if selected { Color32::WHITE } else { t::TEXT }),
        Align::Left,
    );
    // 並びにある物の印（追加すると写しを作る）
    if r.placed {
        let x = name_rect.left() + w::text_width(&painter, &shown, t::LABEL) + 6.0;
        w::icon(
            &painter,
            Rect::from_min_size(pos2(x, name_rect.top()), vec2(14.0, name_rect.height())),
            "content_copy",
            t::TEXT_DIM,
            12.0,
        );
    }
    let sample = Rect::from_min_max(
        pos2(name_rect.right() + 6.0, row.top() + 4.0),
        pos2(row.right() - 6.0, row.bottom() - 5.0),
    );
    if let Some(brush) = catalog::brush_of(app, r.item) {
        paint_sample(
            ui,
            app,
            sample,
            &brush,
            SampleSpec::row(false),
            Id::new(("brush.sample.catalog", r.item)),
        );
    }
    if response.clicked() {
        app.toolset.catalog.toggle(r.item);
    }
    if response.secondary_clicked() && matches!(r.item, CatalogItem::User(_)) {
        if let Some(at) = response.interact_pointer_pos() {
            app.toolset.catalog.context = Some(r.item);
            super::properties::open_popup(
                app,
                ui.ctx(),
                Popup::CatalogContext,
                context_anchor(at),
                0.0,
            );
        }
    }
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &r.name));
    let tip = if r.placed {
        lang.pick(
            format!("{}（並びにあります。追加すると写しを作ります）", r.name),
            format!("{} (already in a group; adding makes a copy)", r.name),
        )
    } else {
        r.name.clone()
    };
    let _ = response.on_hover_text(tip);
}

fn content(ui: &mut Ui, app: &mut AppState, pane: Rect) {
    let lang = app.lang;
    let ctx = ui.ctx().clone();
    // 同梱の Krita は、種類を開いたときに読む（別のスレッド。できたら描き直す）
    if app.toolset.catalog.kind == Kind::Krita {
        app.brushes.krita.poll(&ctx);
    }
    let search = Rect::from_min_size(
        pos2(pane.left() + 10.0, pane.top() + 6.0),
        vec2(pane.width() - 20.0, SEARCH_HEIGHT - 12.0),
    );
    if super::assets::search_field(
        ui,
        search,
        "brush.catalog.search",
        &mut app.toolset.catalog.search,
        lang.pick("検索", "Search"),
    ) {
        app.toolset.catalog.scroll = 0.0;
    }
    let list = Rect::from_min_max(pos2(pane.left(), pane.top() + SEARCH_HEIGHT), pane.max);
    w::fill(ui.painter(), list, t::CONTROL_BG);
    let rows = catalog::rows(app, app.toolset.catalog.kind, &app.toolset.catalog.search);
    let content = rows.len() as f32 * ROW_HEIGHT;
    app.toolset.catalog.content = content;
    if ui.rect_contains_pointer(list) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        app.toolset.catalog.scroll -= wheel;
    }
    let bar = Scroll::new(list, content, &mut app.toolset.catalog.scroll);
    let scroll = app.toolset.catalog.scroll;
    let row_width = list.width() - bar.reserved();
    let outer = ui.clip_rect();
    ui.set_clip_rect(list.intersect(outer));
    for (i, r) in rows.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + i as f32 * ROW_HEIGHT - scroll),
            vec2(row_width, ROW_HEIGHT),
        );
        if row.bottom() < list.top() || row.top() > list.bottom() {
            continue;
        }
        row_view(ui, app, row, r);
    }
    ui.set_clip_rect(outer);
    bar.end(ui, "brush.catalog.scroll", &mut app.toolset.catalog.scroll);
}

fn footer(ui: &mut Ui, app: &mut AppState, bar: Rect) -> bool {
    let lang = app.lang;
    w::fill(ui.painter(), bar, t::PANEL_HEADER);
    w::hline(ui.painter(), bar.left(), bar.right(), bar.top(), t::BORDER);
    let count = app.toolset.catalog.selected.len();
    let free = !app.is_stroking() && app.toolset.set.locked.is_none();
    let add = Rect::from_min_size(
        pos2(bar.right() - 12.0 - 110.0, bar.top() + 8.0),
        vec2(110.0, bar.height() - 16.0),
    );
    let label = if count > 1 {
        lang.pick(format!("{count} 個を追加"), format!("Add {count}"))
    } else {
        lang.pick("追加", "Add").to_owned()
    };
    w::button(
        ui,
        add,
        "brush.catalog.add",
        &label,
        true,
        free && count > 0,
        None,
        Some("add"),
    )
    .clicked()
}

/// 窓を描く（開いていなければ何もしない）。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.toolset.catalog.open {
        return;
    }
    app.brushes.samples.begin_frame(ctx.cumulative_pass_nr());
    let lang = app.lang;
    if !app.toolset.catalog.placed {
        // 初めは、左のドックの右に置く（キャンバスの真ん中を空ける）
        let screen = ctx.content_rect();
        let dock_right = match app.subtools.ui.panel_right {
            right if right > 0.0 => right,
            _ => screen.left() + t::TOOL_STRIP_WIDTH + t::DOCK_WIDTH,
        };
        let min = pos2(dock_right + 24.0, screen.top() + 96.0);
        let center = min + vec2(WIDTH, HEIGHT) / 2.0;
        app.toolset.catalog.offset = center - screen.center();
        app.toolset.catalog.placed = true;
    }
    let title = lang.pick("ブラシを追加", "Add Brushes");
    let spec = Spec {
        title,
        icon: Some("add"),
        size: vec2(WIDTH, HEIGHT),
        modal: false,
        close_label: lang.pick("閉じる", "Close"),
    };
    let mut offset: Vec2 = app.toolset.catalog.offset;
    let mut add = false;
    let closed = window::show(ctx, id(), &spec, &mut offset, false, |ui, frame| {
        let body = frame.body;
        let side = Rect::from_min_size(body.min, vec2(SIDEBAR, body.height() - FOOTER));
        let pane = Rect::from_min_max(
            pos2(side.right(), body.top()),
            pos2(body.right(), body.bottom() - FOOTER),
        );
        sidebar(ui, app, side);
        content(ui, app, pane);
        let bar = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOTER), body.max);
        add = footer(ui, app, bar);
    });
    app.toolset.catalog.offset = offset;
    if add {
        let items = std::mem::take(&mut app.toolset.catalog.selected);
        app.apply(Action::Brush(BrushAction::AddFrom(items)));
    }
    if closed {
        app.toolset.catalog.open = false;
    }
}

/// 行の右クリックのメニュー（利用者のブラシのファイルを消す）。
pub fn context_menu(app: &AppState) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let Some(CatalogItem::User(id)) = app.toolset.catalog.context else {
        return Vec::new();
    };
    let free = !app.is_stroking() && !app.brushes.import.is_busy();
    vec![Entry::item(
        lang.pick("ファイルを削除…", "Delete File…"),
        Action::Brush(BrushAction::DeleteFileDialog(
            crate::brushes::BrushKey::User(id),
        )),
    )
    .enabled(free)]
}
