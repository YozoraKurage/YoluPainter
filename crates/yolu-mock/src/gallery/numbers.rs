//! 数の入力とスライダー（スライダー・数値の欄・文字の入力欄）。

use egui::{pos2, vec2, Color32, Rect, Ui};
use yolu_app::ui::numfield::{self, NumSpec};
use yolu_app::ui::widgets::{self as w, NumberFormat, SliderSpec};

use super::{rows, Entry, Section};
use crate::page::SketchCtx;

pub fn section() -> Section {
    Section {
        title: ("数の入力とスライダー", "Number input and sliders"),
        entries: vec![
            Entry::new(
                "widgets::slider (one line)",
                ("名前と値が溝の中に出る", "Label and value inside the track"),
                84.0,
                one_line,
            ),
            Entry::new(
                "widgets::slider (two lines)",
                (
                    "名前と値の箱の下に溝",
                    "Label and value box above the track",
                ),
                122.0,
                two_lines,
            ),
            Entry::new(
                "widgets::NumberFormat",
                (
                    "値の書き方（整数・小数・単位）",
                    "Value format (integer, decimals, unit)",
                ),
                84.0,
                formats,
            ),
            Entry::new(
                "numfield::number_field",
                (
                    "数値の欄（軸の色の下線）",
                    "Number field with axis underline",
                ),
                56.0,
                number_fields,
            ),
            Entry::new(
                "widgets::text_field",
                ("文字の入力欄", "Text input"),
                48.0,
                text_fields,
            ),
        ],
    }
}

/// スライダーを 1 つ描いて、動いたら値の置き場へ書く。
fn slider_at(
    ui: &mut Ui,
    ctx: &mut SketchCtx,
    r: Rect,
    key: &str,
    default: f32,
    spec: &SliderSpec<'_>,
) {
    let name = format!("gallery.slider.{key}");
    let value = ctx.values.float(&name, default);
    let out = w::slider(ui, r, key.to_owned(), value, spec);
    if out.changed {
        ctx.values.set_float(&name, out.value);
    }
}

fn one_line(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let mut rows = w::Rows::compact(sample, 8.0);
    let (opacity, size, angle) = (
        ctx.pick("不透明度", "Opacity"),
        ctx.pick("サイズ", "Size"),
        ctx.pick("角度", "Angle"),
    );
    let r = rows.slider_row();
    slider_at(
        ui,
        ctx,
        r,
        "opacity",
        70.0,
        &SliderSpec::new(opacity, 0.0, 100.0, NumberFormat::int("%")),
    );
    let r = rows.slider_row();
    slider_at(
        ui,
        ctx,
        r,
        "size",
        24.0,
        &SliderSpec::new(size, 1.0, 500.0, NumberFormat::int(" px")),
    );
    let r = rows.slider_row();
    slider_at(
        ui,
        ctx,
        r,
        "angle",
        0.0,
        &SliderSpec::new(angle, -180.0, 180.0, NumberFormat::int("°")),
    );
}

fn two_lines(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let mut rows = w::Rows::new(sample, 8.0);
    let (opacity, size, off) = (
        ctx.pick("不透明度", "Opacity"),
        ctx.pick("ブラシのサイズ", "Brush size"),
        ctx.pick("押せない", "Disabled"),
    );
    let r = rows.slider_row();
    slider_at(
        ui,
        ctx,
        r,
        "two.opacity",
        70.0,
        &SliderSpec::new(opacity, 0.0, 100.0, NumberFormat::int("%")),
    );
    let r = rows.slider_row();
    slider_at(
        ui,
        ctx,
        r,
        "two.size",
        24.0,
        &SliderSpec::new(size, 1.0, 500.0, NumberFormat::int(" px")).power(2.0),
    );
    let r = rows.slider_row();
    slider_at(
        ui,
        ctx,
        r,
        "two.off",
        40.0,
        &SliderSpec::new(off, 0.0, 100.0, NumberFormat::int("%")).enabled(false),
    );
}

fn formats(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let mut rows = w::Rows::compact(sample, 8.0);
    let decimals = NumberFormat {
        decimals: 2,
        trim: true,
        suffix: "",
    };
    let r = rows.slider_row();
    slider_at(
        ui,
        ctx,
        r,
        "fmt.int",
        50.0,
        &SliderSpec::new(
            ctx.pick("整数", "Integer"),
            0.0,
            100.0,
            NumberFormat::int(""),
        ),
    );
    let r = rows.slider_row();
    slider_at(
        ui,
        ctx,
        r,
        "fmt.trim",
        0.5,
        &SliderSpec::new(
            ctx.pick("小数（0 を落とす）", "Decimals (trimmed)"),
            0.0,
            1.0,
            decimals,
        ),
    );
    let r = rows.slider_row();
    slider_at(
        ui,
        ctx,
        r,
        "fmt.unit",
        12.0,
        &SliderSpec::new(
            ctx.pick("単位つき", "With a unit"),
            0.0,
            100.0,
            NumberFormat::int(" px"),
        ),
    );
}

fn number_fields(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let spec = NumSpec::new(-1000.0, 1000.0, 0.5, 2);
    let axes = [
        ("X", Color32::from_rgb(220, 80, 80)),
        ("Y", Color32::from_rgb(90, 190, 100)),
        ("Z", Color32::from_rgb(80, 130, 230)),
    ];
    let width = (sample.width() - 20.0 - 2.0 * 6.0) / 3.0;
    for (i, (label, color)) in axes.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(
                sample.left() + 10.0 + i as f32 * (width + 6.0),
                sample.top() + 14.0,
            ),
            vec2(width, 24.0),
        );
        let key = format!("gallery.number.{label}");
        let value = f64::from(ctx.values.float(&key, 10.0 * (i as f32 + 1.0)));
        let out = numfield::number_field(
            ui,
            r,
            ("number", i),
            label,
            value,
            &spec,
            Some(label),
            Some(*color),
            true,
        );
        if out.changed {
            ctx.values.set_float(&key, out.value as f32);
        }
    }
}

fn text_fields(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let mut rows = rows(sample);
    rows.space(10.0);
    let r = rows.row(24.0, 0.0);
    let current = ctx
        .values
        .text("gallery.text", ctx.pick("レイヤー 3", "Layer 3"));
    let out = w::text_field(ui, r, "text", &current, None, false);
    if let Some(next) = out.committed {
        ctx.values.set_text("gallery.text", next);
    }
}
