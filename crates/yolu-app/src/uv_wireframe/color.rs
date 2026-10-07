//! UV の個人設定用の色・不透明度。見本を押すと色の窓（色と不透明度）。内部の数値型の切り替えは出さない。
use crate::{
    panels::color_window::{self, Pick},
    state::AppState,
    ui::{
        theme as t,
        widgets::{self as w, Align},
    },
};
use egui::{pos2, vec2, Rect, Ui};

/// 色の窓の相手の名前（試験が窓の相手を確かめる）。
pub fn window_target() -> egui::Id {
    egui::Id::new("uv.wireframe.color")
}

/// 色の窓で色・不透明度をドラッグしている間は true（離すまで設定のファイルへ書かない）。
pub fn settings_row(ui: &mut Ui, rows: &mut w::Rows, app: &mut AppState) -> bool {
    let row = rows.row(t::ROW_HEIGHT, 4.0);
    let lang = app.lang;
    let name = lang.pick("UV ワイヤーフレーム", "UV Wireframe");
    w::text(
        ui.painter(),
        Rect::from_min_max(row.min, pos2(row.right() - 56.0, row.bottom())),
        name,
        t::LABEL,
        Align::Left,
    );
    let swatch = Rect::from_min_size(
        pos2(row.right() - 48.0, row.top()),
        vec2(48.0, row.height()),
    );
    let c = app.prefs.settings.uv_wireframe_color;
    let current = Pick {
        rgb: [c[0], c[1], c[2]],
        alpha: Some(c[3]),
    };
    if let Some(u) = color_window::field(
        ui,
        swatch,
        window_target(),
        name,
        current,
        lang.pick(
            "UV ワイヤーフレームの色と不透明度",
            "UV wireframe color and opacity",
        ),
        true,
    ) {
        let [r, g, b] = u.pick.rgb;
        app.prefs.settings.uv_wireframe_color = [r, g, b, u.pick.alpha.unwrap_or(c[3])];
    }
    color_window::dragging(ui.ctx(), window_target())
}
