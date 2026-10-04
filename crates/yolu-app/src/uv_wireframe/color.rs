//! UV の個人設定用の色・不透明度。内部の数値型の切り替えは出さない。
use crate::{
    state::AppState,
    ui::{
        theme as t,
        widgets::{self as w, Align, NumberFormat, SliderSpec},
    },
};
use egui::{pos2, vec2, Rect, Sense, Ui};

/// スライダーをドラッグしている間は true。
pub fn settings_row(ui: &mut Ui, rows: &mut w::Rows, app: &mut AppState) -> bool {
    let mut dragging = false;
    let row = rows.row(t::ROW_HEIGHT, 4.0);
    let lang = app.lang;
    w::text(
        ui.painter(),
        Rect::from_min_max(row.min, pos2(row.right() - 56.0, row.bottom())),
        lang.pick("UV ワイヤーフレーム", "UV Wireframe"),
        t::LABEL,
        Align::Left,
    );
    let swatch = Rect::from_min_size(
        pos2(row.right() - 48.0, row.top()),
        vec2(48.0, row.height()),
    );
    let response = w::color_swatch(
        ui,
        swatch,
        "uv.color",
        app.prefs
            .settings
            .uv_wireframe_color
            .map(|v| v as f32 / 255.0),
        lang.pick(
            "UV ワイヤーフレームの色と不透明度",
            "UV wireframe color and opacity",
        ),
        true,
    );
    egui::Popup::from_toggle_button_response(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(280.0)
        .show(|ui| {
            let (area, _) = ui.allocate_exact_size(
                vec2(280.0, 30.0 + 4.0 * (t::SLIDER_ROW_HEIGHT + 4.0)),
                Sense::hover(),
            );
            let mut rows = w::Rows::new(area, 0.0);
            let preview = rows.row(26.0, 4.0);
            w::color_swatch(
                ui,
                preview,
                "uv.preview",
                app.prefs
                    .settings
                    .uv_wireframe_color
                    .map(|v| v as f32 / 255.0),
                lang.pick("選択中の色", "Current color"),
                false,
            );
            let labels = [
                lang.pick("赤", "Red"),
                lang.pick("緑", "Green"),
                lang.pick("青", "Blue"),
                lang.pick("不透明度", "Opacity"),
            ];
            let tips = [
                lang.pick("赤の強さ", "Red intensity"),
                lang.pick("緑の強さ", "Green intensity"),
                lang.pick("青の強さ", "Blue intensity"),
                lang.pick("UV ワイヤーフレームの不透明度", "UV wireframe opacity"),
            ];
            for (i, (label, tip)) in labels.into_iter().zip(tips).enumerate() {
                let alpha = i == 3;
                let scale = if alpha { 100.0 / 255.0 } else { 1.0 };
                let out = w::slider(
                    ui,
                    rows.row(t::SLIDER_ROW_HEIGHT, 4.0),
                    ("uv.component", i),
                    app.prefs.settings.uv_wireframe_color[i] as f32 * scale,
                    &SliderSpec::new(
                        label,
                        0.0,
                        if alpha { 100.0 } else { 255.0 },
                        NumberFormat::int(if alpha { "%" } else { "" }),
                    )
                    .tooltip(tip),
                );
                dragging |= out.active;
                if out.changed {
                    app.prefs.settings.uv_wireframe_color[i] =
                        (out.value / scale).round().clamp(0.0, 255.0) as u8;
                }
            }
        });
    dragging
}
