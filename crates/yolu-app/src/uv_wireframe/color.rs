//! UV の個人設定用の色・不透明度（ワイヤーフレームと、重なった UV）。見本を押すと色のウィンドウ（色と不透明度）。内部の数値型の切り替えは出さない。
use crate::{
    panels::color_window::{self, Pick},
    state::AppState,
    ui::{
        theme as t,
        widgets::{self as w, Align},
    },
};
use egui::{pos2, vec2, Rect, Ui};

/// 色の見本の幅と間。
const SWATCH: f32 = 48.0;
const GAP: f32 = 8.0;

/// ワイヤーフレームの色のウィンドウの相手の名前（試験がウィンドウの相手を確かめる）。
pub fn window_target() -> egui::Id {
    egui::Id::new("uv.wireframe.color")
}

/// 重なった UV の色のウィンドウの相手の名前。
pub fn overlap_window_target() -> egui::Id {
    egui::Id::new("uv.overlap.color")
}

/// 色のウィンドウで色・不透明度をドラッグしている間は true（離すまで設定のファイルへ書かない）。
pub fn settings_row(ui: &mut Ui, rows: &mut w::Rows, app: &mut AppState) -> bool {
    let row = rows.row(t::ROW_HEIGHT, crate::prefs::GAP);
    let lang = app.lang;
    let name = lang.pick("UV ワイヤーフレーム", "UV Wireframe");
    w::text(
        ui.painter(),
        Rect::from_min_max(
            row.min,
            pos2(row.right() - 2.0 * SWATCH - GAP - 8.0, row.bottom()),
        ),
        name,
        t::LABEL,
        Align::Left,
    );
    let wire = Rect::from_min_size(
        pos2(row.right() - 2.0 * SWATCH - GAP, row.top()),
        vec2(SWATCH, row.height()),
    );
    let overlap = Rect::from_min_size(
        pos2(row.right() - SWATCH, row.top()),
        vec2(SWATCH, row.height()),
    );
    swatch(
        ui,
        wire,
        window_target(),
        name,
        &mut app.prefs.settings.uv_wireframe_color,
        lang.pick(
            "UV ワイヤーフレームの色と不透明度",
            "UV wireframe color and opacity",
        ),
    );
    swatch(
        ui,
        overlap,
        overlap_window_target(),
        lang.pick("重なった UV", "Overlapping UV"),
        &mut app.prefs.settings.uv_overlap_color,
        lang.pick(
            "重なった UV の色と不透明度",
            "Overlapping UV color and opacity",
        ),
    );
    color_window::dragging(ui.ctx(), window_target())
        || color_window::dragging(ui.ctx(), overlap_window_target())
}

/// 色の見本 1 つ。押すと色のウィンドウ（色と不透明度）を開き、選んだ色を `color` へ入れる。
fn swatch(ui: &mut Ui, r: Rect, target: egui::Id, name: &str, color: &mut [u8; 4], tip: &str) {
    let c = *color;
    let current = Pick {
        rgb: [c[0], c[1], c[2]],
        alpha: Some(c[3]),
    };
    if let Some(u) = color_window::field(ui, r, target, name, current, tip, true) {
        let [r, g, b] = u.pick.rgb;
        *color = [r, g, b, u.pick.alpha.unwrap_or(c[3])];
    }
}
