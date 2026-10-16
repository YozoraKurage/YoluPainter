//! 一覧とサムネイル（欄の節・スクロールする一覧・行を積む補助・レイヤーのサムネイル）。

use egui::{pos2, vec2, Rect, Ui};
use yolu_app::ui::scroll::Scroll;
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align};

use super::{rows, Entry, Section};
use crate::page::SketchCtx;
use crate::sketches::layers_common::{self as parts, Kind};

pub fn section() -> Section {
    Section {
        title: ("一覧とサムネイル", "Lists and thumbnails"),
        entries: vec![
            Entry::new(
                "widgets::section_header",
                ("折りたためる大見出しの帯", "Collapsible section band"),
                88.0,
                section_headers,
            ),
            Entry::new(
                "widgets::subsection_header",
                ("大見出しの中の小見出し", "Sub-heading inside a section"),
                62.0,
                subsection_headers,
            ),
            Entry::new(
                "widgets::panel_title",
                ("折りたたまない見出し", "Non-collapsing heading"),
                56.0,
                panel_titles,
            ),
            Entry::new(
                "scroll::Scroll",
                ("自前の一覧のスクロール", "Scrolling for hand-laid lists"),
                150.0,
                scrolling_list,
            ),
            Entry::new(
                "widgets::Rows",
                (
                    "行を積む・等分する補助",
                    "Row stacking and splitting helper",
                ),
                86.0,
                row_helper,
            ),
            Entry::new(
                "thumbnails: checker, rounded, outline, icon",
                ("レイヤーの種類ごとの絵", "One picture per layer kind"),
                78.0,
                thumbnails,
            ),
        ],
    }
}

fn section_headers(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let mut rows = w::Rows::new(sample, 6.0);
    for (i, (key, ja, en, icon, reset)) in [
        ("a", "ブラシ", "Brush", Some("paint_brush"), true),
        ("b", "不透明度と流量", "Opacity and flow", None, false),
    ]
    .iter()
    .enumerate()
    {
        let r = rows.full_row(24.0, 4.0);
        let name = format!("gallery.section.{key}");
        let open = ctx.values.bool(&name, i == 0);
        let out = w::section_header(
            ui,
            r,
            ("section", i),
            ctx.pick(ja, en),
            open,
            *icon,
            reset.then(|| ctx.pick("既定に戻す", "Reset")),
        );
        ctx.values.set_bool(&name, out.open);
    }
}

fn subsection_headers(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let mut rows = rows(sample);
    rows.space(6.0);
    for (i, (ja, en)) in [("形", "Shape"), ("入力", "Input")].iter().enumerate() {
        let r = rows.row(20.0, 4.0);
        let name = format!("gallery.subsection.{i}");
        let open = ctx.values.bool(&name, i == 0);
        let next = w::subsection_header(ui, r, ("subsection", i), ctx.pick(ja, en), open);
        ctx.values.set_bool(&name, next);
    }
}

fn panel_titles(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let p = ui.painter().clone();
    let r = Rect::from_min_size(
        pos2(sample.left(), sample.top() + 6.0),
        vec2(sample.width(), 22.0),
    );
    w::panel_title(&p, r, ctx.pick("レイヤー", "Layers"), Some("layers"));
    let r = r.translate(vec2(0.0, 26.0));
    w::panel_title(&p, r, ctx.pick("名前だけ", "Name only"), None);
}

fn scrolling_list(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let view = sample.shrink(6.0);
    w::fill(ui.painter(), view, t::CONTROL_BG);
    let count = 14;
    let row_h = 22.0;
    let mut offset = ctx.values.float("gallery.scroll", 0.0);
    let bar = Scroll::begin(ui, view, count as f32 * row_h, &mut offset);
    let p = ui.painter_at(view);
    let width = view.width() - bar.reserved();
    for i in 0..count {
        let y = view.top() + i as f32 * row_h - offset;
        let row = Rect::from_min_size(pos2(view.left(), y), vec2(width, row_h));
        if row.bottom() < view.top() || row.top() > view.bottom() {
            continue;
        }
        w::hline(&p, row.left(), row.right(), row.bottom() - 1.0, t::BORDER);
        w::text(
            &p,
            row.shrink2(vec2(8.0, 0.0)),
            &format!("{} {}", ctx.pick("項目", "Item"), i + 1),
            t::LABEL,
            Align::Left,
        );
    }
    bar.end(ui, "gallery-scroll", &mut offset);
    ctx.values.set_float("gallery.scroll", offset);
}

fn row_helper(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let mut rows = rows(sample);
    rows.space(6.0);
    let p = ui.painter().clone();
    let line = rows.row(20.0, 4.0);
    for (i, cell) in w::Rows::split(line, 3, 4.0).iter().enumerate() {
        w::rounded(&p, *cell, t::CONTROL_HOVER, 3.0);
        w::text(&p, *cell, &format!("{}", i + 1), t::LABEL, Align::Center);
    }
    rows.indent = 16.0;
    let line = rows.row(20.0, 0.0);
    w::rounded(&p, line, t::ACCENT_DIM, 3.0);
    w::text(
        &p,
        line,
        ctx.pick("字下げした行", "Indented row"),
        t::LABEL,
        Align::Center,
    );
}

fn thumbnails(ui: &mut Ui, sample: Rect, _: &mut SketchCtx) {
    let p = ui.painter().clone();
    let kinds = [
        Kind::Raster(egui::Color32::from_rgb(222, 150, 80)),
        Kind::Fill(egui::Color32::from_rgb(70, 120, 200)),
        Kind::Adjustment,
        Kind::Group,
    ];
    for (i, kind) in kinds.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(sample.left() + 14.0 + i as f32 * 48.0, sample.top() + 14.0),
            vec2(42.0, 42.0),
        );
        parts::thumb(&p, r, *kind);
    }
    let mask = Rect::from_min_size(
        pos2(sample.left() + 14.0 + 4.0 * 48.0, sample.top() + 14.0),
        vec2(42.0, 42.0),
    );
    parts::mask_thumb(&p, mask, true);
}
