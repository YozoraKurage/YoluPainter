//! 色と文字（`ui::theme` の色と文字の形、描くだけの部品）。

use egui::{pos2, vec2, Color32, Rect, Ui};
use yolu_app::ui::theme::{self as t, TextStyle};
use yolu_app::ui::widgets::{self as w, Align};

use super::{rows, Entry, Section};
use crate::page::SketchCtx;

pub fn section() -> Section {
    Section {
        title: ("色と文字", "Colors and text"),
        entries: vec![
            Entry::new(
                "theme::WINDOW_BG … SEPARATOR",
                ("背景・面・線の色", "Background, surface, line colors"),
                146.0,
                surfaces,
            ),
            Entry::new(
                "theme::ACCENT … TEXT_DISABLED",
                ("強調・状態・文字の色", "Accent, state, text colors"),
                124.0,
                accents,
            ),
            Entry::new(
                "theme::LABEL … VALUE",
                (
                    "文字の形（大きさ・太さ・色）",
                    "Text styles (size, weight, color)",
                ),
                156.0,
                text_styles,
            ),
            Entry::new(
                "widgets::text・fit・wrapped_text",
                (
                    "1 行の文字・幅に収める・折り返す",
                    "One-line text, fitting, wrapping",
                ),
                150.0,
                texts,
            ),
            Entry::new(
                "widgets::fill・rounded・outline・hline・vline",
                ("矩形・角丸・枠・線", "Rectangles, outlines, lines"),
                64.0,
                shapes,
            ),
            Entry::new(
                "widgets::checker",
                ("透明を表す市松", "Transparency checkerboard"),
                60.0,
                checkers,
            ),
            Entry::new(
                "widgets::progress_bar",
                ("進み具合の帯", "Progress bar"),
                60.0,
                progress,
            ),
            Entry::new(
                "widgets::color_swatch",
                (
                    "色の見本（半透明は市松の上）",
                    "Color swatch (alpha over checker)",
                ),
                48.0,
                swatches,
            ),
        ],
    }
}

/// 名前つきの色の見本を、2 列に並べる。
fn palette(ui: &mut Ui, sample: Rect, items: &[(&str, Color32)]) {
    let half = (sample.width() - 24.0) / 2.0;
    let p = ui.painter().clone();
    for (i, (name, color)) in items.iter().enumerate() {
        let column = i % 2;
        let row = i / 2;
        let x = sample.left() + 8.0 + column as f32 * (half + 8.0);
        let y = sample.top() + 8.0 + row as f32 * 22.0;
        let swatch = Rect::from_min_size(pos2(x, y + 1.0), vec2(18.0, 18.0));
        w::checker(&p, swatch, 3.0);
        w::rounded(&p, swatch, *color, 3.0);
        w::outline(&p, swatch, t::SEPARATOR, 1.0, 3.0);
        w::text(
            &p,
            Rect::from_min_size(pos2(x + 24.0, y), vec2(half - 24.0, 20.0)),
            name,
            t::LABEL_DIM,
            Align::Left,
        );
    }
}

fn surfaces(ui: &mut Ui, sample: Rect, _: &mut SketchCtx) {
    palette(
        ui,
        sample,
        &[
            ("WINDOW_BG", t::WINDOW_BG),
            ("PANEL_BG", t::PANEL_BG),
            ("PANEL_HEADER", t::PANEL_HEADER),
            ("MENU_BG", t::MENU_BG),
            ("CONTROL_BG", t::CONTROL_BG),
            ("CONTROL_HOVER", t::CONTROL_HOVER),
            ("CONTROL_ACTIVE", t::CONTROL_ACTIVE),
            ("SECTION_BAND", t::SECTION_BAND),
            ("CANVAS_BG", t::CANVAS_BG),
            ("SEPARATOR", t::SEPARATOR),
            ("BORDER", t::BORDER),
        ],
    );
}

fn accents(ui: &mut Ui, sample: Rect, _: &mut SketchCtx) {
    palette(
        ui,
        sample,
        &[
            ("ACCENT", t::ACCENT),
            ("ACCENT_DIM", t::ACCENT_DIM),
            ("ACCENT_SOFT", t::ACCENT_SOFT),
            ("SLIDER_FILL", t::SLIDER_FILL),
            ("WARNING", t::WARNING),
            ("OK", t::OK),
            ("ERROR", t::ERROR),
            ("TEXT", t::TEXT),
            ("TEXT_DIM", t::TEXT_DIM),
            ("TEXT_DISABLED", t::TEXT_DISABLED),
        ],
    );
}

fn text_styles(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let styles: [(&str, TextStyle); 6] = [
        ("LABEL", t::LABEL),
        ("LABEL_DIM", t::LABEL_DIM),
        ("LABEL_SMALL", t::LABEL_SMALL),
        ("LABEL_BOLD", t::LABEL_BOLD),
        ("HEADER", t::HEADER),
        ("VALUE", t::VALUE),
    ];
    let p = ui.painter().clone();
    let text = ctx.pick("レイヤー 3  100%", "Layer 3  100%");
    for (i, (name, style)) in styles.iter().enumerate() {
        let y = sample.top() + 8.0 + i as f32 * 23.0;
        w::text(
            &p,
            Rect::from_min_size(pos2(sample.left() + 8.0, y), vec2(150.0, 22.0)),
            text,
            *style,
            Align::Left,
        );
        w::text(
            &p,
            Rect::from_min_size(pos2(sample.left() + 160.0, y), vec2(132.0, 22.0)),
            name,
            t::LABEL_SMALL,
            Align::Right,
        );
    }
}

fn texts(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let mut rows = rows(sample);
    rows.space(6.0);
    let p = ui.painter().clone();
    let word = ctx.pick("名前", "Name");
    for align in [Align::Left, Align::Center, Align::Right] {
        let r = rows.row(22.0, 2.0);
        w::outline(&p, r, t::SEPARATOR, 1.0, 0.0);
        w::text(&p, r.shrink2(vec2(4.0, 0.0)), word, t::LABEL, align);
    }
    let r = rows.row(22.0, 2.0);
    let long = ctx.pick(
        "背景に使うとても長いレイヤーの名前",
        "A very long layer name used for the background",
    );
    let narrow = Rect::from_min_size(r.min, vec2(r.width() * 0.6, r.height()));
    w::outline(&p, narrow, t::SEPARATOR, 1.0, 0.0);
    let shown = w::fit(&p, long, narrow.width() - 8.0, t::LABEL);
    w::text(
        &p,
        narrow.shrink2(vec2(4.0, 0.0)),
        &shown,
        t::LABEL,
        Align::Left,
    );
    let r = rows.row(34.0, 0.0);
    w::wrapped_text(
        &p,
        r,
        ctx.pick(
            "ベースカラー・ラフネス・メタリック・ノーマル・ハイト・アンビエントオクルージョン",
            "Base color, roughness, metallic, normal, height, ambient occlusion",
        ),
        t::LABEL_DIM,
    );
}

fn shapes(ui: &mut Ui, sample: Rect, _: &mut SketchCtx) {
    let p = ui.painter().clone();
    let y = sample.top() + 10.0;
    let cell =
        |i: f32| Rect::from_min_size(pos2(sample.left() + 10.0 + i * 56.0, y), vec2(44.0, 44.0));
    w::fill(&p, cell(0.0), t::CONTROL_HOVER);
    w::rounded(&p, cell(1.0), t::ACCENT_DIM, 8.0);
    w::outline(&p, cell(2.0), t::ACCENT, 2.0, 6.0);
    let line = cell(3.0);
    w::hline(&p, line.left(), line.right(), line.center().y, t::TEXT);
    w::vline(&p, line.center().x, line.top(), line.bottom(), t::TEXT_DIM);
}

fn checkers(ui: &mut Ui, sample: Rect, _: &mut SketchCtx) {
    let p = ui.painter().clone();
    for (i, cell) in [3.0_f32, 6.0, 10.0].iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(sample.left() + 10.0 + i as f32 * 94.0, sample.top() + 10.0),
            vec2(84.0, 40.0),
        );
        w::checker(&p, r, *cell);
        w::outline(&p, r, t::BORDER, 1.0, 0.0);
    }
}

fn progress(ui: &mut Ui, sample: Rect, _: &mut SketchCtx) {
    let mut rows = rows(sample);
    rows.space(10.0);
    let p = ui.painter().clone();
    w::progress_bar(&p, rows.row(10.0, 12.0), Some(0.6), 0.0);
    // 割合が分からない仕事の帯は、時刻で往復する（見本は止めた位置）
    w::progress_bar(&p, rows.row(10.0, 0.0), None, 0.35);
}

fn swatches(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let colors: [([f32; 4], &str, &str); 3] = [
        ([0.86, 0.3, 0.25, 1.0], "赤", "Red"),
        ([0.2, 0.5, 0.9, 0.5], "青（半透明）", "Blue (half alpha)"),
        ([1.0, 1.0, 1.0, 0.0], "透明", "Transparent"),
    ];
    for (i, (color, ja, en)) in colors.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(sample.left() + 10.0 + i as f32 * 94.0, sample.top() + 10.0),
            vec2(84.0, 28.0),
        );
        let _ = w::color_swatch(ui, r, ("swatch", i), *color, ctx.pick(ja, en), true);
    }
}
