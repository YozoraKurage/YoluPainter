//! グラデーションのツールの欄: オプションバー（形・不透明度・塗る/消す）と、左のドックのツールプロパティ（形・終点・不透明度・塗る/消す、マテリアルで塗る
//! ときの終点）。形と終点の組はサブツールの一覧（`subtool`）でも選べる。画面には名前と値だけを出し、説明はツールチップ。操作は `Action::Gradient`
//! （キー・試験と同じ道）。

use egui::{pos2, vec2, Rect, Ui};
use yolu_core::material::GradientShape;

use super::{End, GradientOp};
use crate::lang::Lang;
use crate::panels::properties::{choice_buttons, slider_row, toggle_row, ChoiceButton};
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};

pub fn shape_name(lang: Lang, shape: GradientShape) -> &'static str {
    match shape {
        GradientShape::Linear => lang.t("gradient-linear"),
        GradientShape::Radial => lang.t("gradient-radial"),
    }
}

pub fn end_name(lang: Lang, end: End) -> &'static str {
    match end {
        End::Transparent => lang.t("gradient-to-transparent"),
        End::Sub => lang.t("gradient-to-background"),
    }
}

fn op(app: &mut AppState, op: GradientOp) {
    app.apply(Action::Gradient(op));
}

/// 塗る・消すの名前（マスクでは白・黒）。
fn paint_erase_names(app: &AppState) -> ([&'static str; 2], [&'static str; 2]) {
    let lang = app.lang;
    if app.m2.edit_mask {
        (
            [lang.t("gradient-white-show"), lang.t("gradient-black-hide")],
            [
                lang.t("gradient-fill-the-mask-with-a"),
                lang.t("gradient-fill-the-mask-with-a-2"),
            ],
        )
    } else {
        (
            [lang.t("gradient-paint"), lang.t("gradient-erase")],
            [
                lang.t("gradient-paint-with-the-paint-color"),
                lang.t("gradient-erase-toward-transparent"),
            ],
        )
    }
}

/// オプションバーの中身（ツールのアイコンの右から）。
pub fn options(ui: &mut Ui, app: &mut AppState, r: Rect, x: f32) {
    let lang = app.lang;
    let mut x = x + 4.0;
    let (y, h) = (r.top() + 6.0, r.height() - 12.0);
    // 形（線形・放射）
    for shape in [GradientShape::Linear, GradientShape::Radial] {
        let name = shape_name(lang, shape);
        let width = w::text_width(ui.painter(), name, t::LABEL) + 22.0;
        let at = Rect::from_min_size(pos2(x, y), vec2(width, h));
        if w::button(
            ui,
            at,
            ("gradient.shape", shape as u8),
            name,
            app.gradient.shape == shape,
            true,
            Some(match shape {
                GradientShape::Linear => lang.t("gradient-in-a-straight-line-from-tip"),
                GradientShape::Radial => lang.t("gradient-around-the-start-out-to-tip"),
            }),
            None,
        )
        .clicked()
        {
            op(app, GradientOp::Shape(shape));
        }
        x += width + 2.0;
    }
    x += 10.0;
    // 不透明度
    let at = Rect::from_min_size(pos2(x, y), vec2(130.0, h));
    let out = w::slider(
        ui,
        at,
        "gradient.opacity",
        app.brush.opacity * 100.0,
        &SliderSpec::new(
            lang.t("gradient-opacity"),
            0.0,
            100.0,
            NumberFormat::int("%"),
        ),
    );
    if out.changed {
        app.brush.opacity = out.value / 100.0;
    }
    x += 138.0;
    // 塗る・消す
    let (names, tips) = paint_erase_names(app);
    for (k, erase) in [false, true].into_iter().enumerate() {
        let width = w::text_width(ui.painter(), names[k], t::LABEL) + 20.0;
        let at = Rect::from_min_size(pos2(x, y), vec2(width, h));
        if w::button(
            ui,
            at,
            ("gradient.erase", k),
            names[k],
            app.gradient.erase == erase,
            true,
            Some(tips[k]),
            None,
        )
        .clicked()
        {
            op(app, GradientOp::Erase(erase));
        }
        x += width + if k == 0 { 0.0 } else { 8.0 };
    }
}

/// ツールプロパティ: ツールの設定（形・終点・不透明度・塗る/消す）と、マテリアルで塗るときの終点。
pub fn body(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    let lang = app.lang;
    let shapes = [GradientShape::Linear, GradientShape::Radial];
    let items: Vec<ChoiceButton> = shapes
        .iter()
        .enumerate()
        .map(|(k, shape)| ChoiceButton {
            id: ["gradient.prop.shape.0", "gradient.prop.shape.1"][k],
            label: shape_name(lang, *shape),
            selected: app.gradient.shape == *shape,
            enabled: true,
            tooltip: None,
        })
        .collect();
    if let Some(i) = choice_buttons(ui, rows, &items) {
        op(app, GradientOp::Shape(shapes[i]));
    }
    if !app.m2.edit_mask {
        let ends = [End::Transparent, End::Sub];
        let items: Vec<ChoiceButton> = ends
            .iter()
            .enumerate()
            .map(|(k, end)| ChoiceButton {
                id: ["gradient.prop.end.0", "gradient.prop.end.1"][k],
                label: end_name(lang, *end),
                selected: app.gradient.end == *end,
                enabled: true,
                tooltip: None,
            })
            .collect();
        if let Some(i) = choice_buttons(ui, rows, &items) {
            op(app, GradientOp::End(ends[i]));
        }
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "gradient.prop.opacity",
        lang.t("gradient-opacity"),
        app.brush.opacity * 100.0,
        (0.0, 100.0),
        NumberFormat::int("%"),
        None,
        true,
    ) {
        app.brush.opacity = v / 100.0;
    }
    // 塗る・消す
    let (names, tips) = paint_erase_names(app);
    let items: Vec<ChoiceButton> = [false, true]
        .into_iter()
        .enumerate()
        .map(|(k, erase)| ChoiceButton {
            id: ["gradient.prop.erase.0", "gradient.prop.erase.1"][k],
            label: names[k],
            selected: app.gradient.erase == erase,
            enabled: true,
            tooltip: Some(tips[k]),
        })
        .collect();
    if let Some(i) = choice_buttons(ui, rows, &items) {
        op(app, GradientOp::Erase(i == 1));
    }
    // マテリアルで塗るとき: 終点を現在のマテリアルにして、2 つのマテリアルの間を塗る
    if app.paints_material() {
        let on = app.gradient.between;
        if let Some(v) = toggle_row(
            ui,
            rows,
            "gradient.between",
            lang.t("gradient-between-two-materials"),
            on,
            None,
            app.gradient.end_material.is_some(),
        ) {
            op(app, GradientOp::Between(v));
        }
        let r = rows.row(24.0, 4.0);
        if w::button(
            ui,
            r,
            "gradient.capture",
            lang.t("gradient-use-current-material-as-end"),
            false,
            true,
            None,
            None,
        )
        .clicked()
        {
            op(app, GradientOp::CaptureEnd);
        }
    }
    rows.space(4.0);
}
