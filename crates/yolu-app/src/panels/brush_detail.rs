//! ブラシの詳細の窓（クリスタのサブツール詳細に当たる）: 左にカテゴリの一覧、右にそのカテゴリの欄、上に今の設定の見本のストローク。
//! 浮いた窓で、見出しのドラッグで動かせる。モーダルではないので、開いたまま（窓の外で）描ける。開くのはツールプロパティの調整の
//! ボタン、閉じるのは窓の閉じるボタンかもう一度そのボタン。欄は `brush_props` のカテゴリごとの関数で、ブラシの設定をその場で変える。

use egui::{pos2, vec2, Color32, Id, Rect, Sense, Ui, Vec2, WidgetInfo, WidgetType};

use super::brush_props;
use super::brushes::{is_eraser, live_brush, paint_sample};
use crate::brushes::sample::SampleSpec;
use crate::brushes::Category;
use crate::state::AppState;
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, Rows};
use crate::ui::window::{self, Spec};

const WIDTH: f32 = 640.0;
const HEIGHT: f32 = 540.0;
const SIDEBAR: f32 = 168.0;
const CATEGORY_HEIGHT: f32 = 30.0;
const SAMPLE_HEIGHT: f32 = 62.0;
const TITLE_HEIGHT: f32 = 26.0;

/// 窓の名前（`windows::window_rect` と同じ形の Id。試験が窓の矩形を引く）。
pub fn id() -> Id {
    Id::new(("yolu.window", "brush-detail"))
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
    let current = app.brushes.ui.detail.category;
    for (i, category) in Category::ALL.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(pane.left(), pane.top() + 6.0 + i as f32 * CATEGORY_HEIGHT),
            vec2(pane.width() - 1.0, CATEGORY_HEIGHT),
        );
        let name = category.name(lang);
        let response = ui.interact(
            row,
            ui.make_persistent_id(("brush.category", i)),
            Sense::click(),
        );
        let on = *category == current;
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
            category.icon(),
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
            app.brushes.ui.detail.category = *category;
            app.brushes.ui.detail.scroll = 0.0;
        }
    }
}

fn content(ui: &mut Ui, app: &mut AppState, pane: Rect) {
    let lang = app.lang;
    let ctx = ui.ctx().clone();
    let category = app.brushes.ui.detail.category;
    // 上: 今の設定の見本
    let sample = Rect::from_min_size(
        pos2(pane.left() + 10.0, pane.top() + 10.0),
        vec2(pane.width() - 20.0, SAMPLE_HEIGHT),
    );
    let eraser = is_eraser(app);
    let live = live_brush(app);
    paint_sample(
        ui,
        app,
        sample,
        &live,
        SampleSpec::detail(eraser),
        egui::Id::new("brush.sample.detail"),
    );
    // カテゴリの名前と既定に戻す
    let title = Rect::from_min_size(
        pos2(pane.left() + 10.0, sample.bottom() + 6.0),
        vec2(pane.width() - 20.0, TITLE_HEIGHT),
    );
    w::text(
        ui.painter(),
        Rect::from_min_max(title.min, pos2(title.right() - 30.0, title.bottom())),
        category.name(lang),
        t::HEADER,
        Align::Left,
    );
    let reset = Rect::from_min_size(
        pos2(title.right() - 26.0, title.top() + 1.0),
        vec2(26.0, 24.0),
    );
    if w::icon_button(
        ui,
        reset,
        "brush.detail.reset",
        "restart_alt",
        &lang.pick(
            format!("{}を既定に戻す", category.name(lang)),
            format!("Reset {}", category.name(lang)),
        ),
        false,
        true,
        15.0,
    )
    .clicked()
    {
        brush_props::reset_category(app, category);
    }
    w::hline(
        ui.painter(),
        pane.left(),
        pane.right(),
        title.bottom() + 1.0,
        t::BORDER,
    );
    // 欄（スクロール）
    let body = Rect::from_min_max(pos2(pane.left(), title.bottom() + 2.0), pane.max);
    let detail = &mut app.brushes.ui.detail;
    let bar = Scroll::begin(ui, body, detail.content, &mut detail.scroll);
    let scroll = detail.scroll;
    let area = Rect::from_min_max(
        pos2(body.left(), body.top() - scroll),
        pos2(body.right() - bar.reserved(), body.bottom()),
    );
    let outer = ui.clip_rect();
    ui.set_clip_rect(body.intersect(outer));
    let mut rows = Rows::new(area, 6.0);
    brush_props::category_body(ui, app, &mut rows, &ctx, category);
    rows.space(10.0);
    app.brushes.ui.detail.content = rows.used();
    ui.set_clip_rect(outer);
    bar.end(ui, "brush_detail.scroll", &mut app.brushes.ui.detail.scroll);
}

/// 窓を描く（開いていなければ何もしない）。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.brushes.ui.detail.open {
        return;
    }
    app.brushes.samples.begin_frame(ctx.cumulative_pass_nr());
    let lang = app.lang;
    if !app.brushes.ui.detail.placed {
        // 初めは、左のドックの右・上の寄りに置く（キャンバスの真ん中を空ける）
        let screen = ctx.content_rect();
        let dock_right = match app.subtools.ui.panel_right {
            right if right > 0.0 => right,
            _ => screen.left() + t::TOOL_STRIP_WIDTH + t::DOCK_WIDTH,
        };
        let min = pos2(dock_right + 24.0, screen.top() + 124.0);
        let center = min + vec2(WIDTH, HEIGHT) / 2.0;
        app.brushes.ui.detail.offset = center - screen.center();
        app.brushes.ui.detail.placed = true;
    }
    let name = app
        .brushes
        .lib
        .entry(app.brushes.lib.current())
        .map(|e| e.name_in(lang))
        .unwrap_or_default();
    let title = format!("{} — {name}", lang.pick("ブラシの詳細", "Brush Details"));
    let spec = Spec {
        title: &title,
        icon: Some("tune"),
        size: vec2(WIDTH, HEIGHT),
        modal: false,
        close_label: lang.pick("閉じる", "Close"),
    };
    let mut offset: Vec2 = app.brushes.ui.detail.offset;
    let closed = window::show(ctx, id(), &spec, &mut offset, false, |ui, frame| {
        let body = frame.body;
        let side = Rect::from_min_size(body.min, vec2(SIDEBAR, body.height()));
        let pane = Rect::from_min_max(pos2(side.right(), body.top()), body.max);
        sidebar(ui, app, side);
        content(ui, app, pane);
    });
    app.brushes.ui.detail.offset = offset;
    if closed {
        app.brushes.ui.detail.open = false;
    }
}
