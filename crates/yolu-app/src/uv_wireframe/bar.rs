//! キャンバスの右上の UV の入口（置き場は `canvas::bar`）: 「目・UV・▽」のボタンと、▽ で開く線の色と不透明度の欄。
//!
//! 目と UV の文字を押すと UV の線の表示・非表示がすぐ切り替わり、▽（細い仕切りの右）を押すと欄が開く。重なった UV の色は出さない。

use egui::{pos2, vec2, Color32, Id, Rect, Sense, Ui, WidgetInfo, WidgetType};

use crate::canvas::bar::{button_bg, BUTTON_HEIGHT, PAD};
use crate::panels::color_window::{self, Pick};
use crate::state::AppState;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};

const LABEL: &str = "UV";
/// ▽ の押せる範囲の幅。
pub const ARROW_WIDTH: f32 = 22.0;

/// 色の欄の相手の名前（色のウィンドウの相手。設定のウィンドウの欄とは別）。
fn color_target() -> Id {
    Id::new("canvasbar.uv.color")
}

/// UV の線の色を色のウィンドウでドラッグしている間（設定のファイルへは、離すまで書かない）。
fn color_dragging(ctx: &egui::Context) -> bool {
    color_window::dragging(ctx, color_target())
}

/// 欄から開いた色のウィンドウの場所（開いていれば）。欄の外の押しで欄を閉じるとき、このウィンドウの上は除く。
pub fn color_window_rect(ctx: &egui::Context) -> Option<Rect> {
    color_window::rect(ctx)
}

/// 目と UV の文字の範囲の幅。
pub fn toggle_width(ui: &Ui) -> f32 {
    6.0 + 18.0 + 3.0 + w::text_width(ui.painter(), LABEL, t::LABEL) + 8.0
}

/// 押せる範囲 2 つ（`toggle` が表示・非表示、`arrow` が欄）を描き、押したかを返す（表示・非表示, 欄を開く）。地は 1 つで、乗せた・押した範囲だけ明るくする。
pub fn button(ui: &mut Ui, app: &AppState, toggle: Rect, arrow: Rect, free: bool) -> (bool, bool) {
    let enabled = free && app.view3d.model.is_some();
    let toggle_id = Id::new("canvasbar.uv.toggle");
    let arrow_id = Id::new("canvasbar.uv.arrow");
    let toggle_look = w::look(ui.ctx(), toggle_id, enabled);
    let arrow_look = w::look(ui.ctx(), arrow_id, enabled);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let toggle_r = ui.interact(toggle, toggle_id, sense);
    let arrow_r = ui.interact(arrow, arrow_id, sense);
    paint_button(
        ui,
        app,
        [
            (toggle, &toggle_r, toggle_look),
            (arrow, &arrow_r, arrow_look),
        ],
    );
    let tip = super::tip(app);
    toggle_r.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &tip));
    let toggle_r = toggle_r.on_hover_text(&tip);
    let arrow_tip = app
        .lang
        .t("uv_wireframe-uv-wireframe-color-and-opacity-tip");
    arrow_r.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, arrow_tip));
    let arrow_r = arrow_r.on_hover_text(arrow_tip);
    (enabled && toggle_r.clicked(), enabled && arrow_r.clicked())
}

/// ボタンの見た目（`parts` は左の表示・非表示と右の ▽ の、範囲・当たり・見た目）。
fn paint_button(ui: &Ui, app: &AppState, parts: [(Rect, &egui::Response, w::Look); 2]) {
    let [(toggle, toggle_r, toggle_look), (arrow, arrow_r, arrow_look)] = parts;
    let uv = toggle.union(arrow);
    let uv_on = app.prefs.settings.uv_wireframe && app.view3d.model.is_some();
    let menu_open = app.ui.canvas_bar.uv_menu;
    let p = ui.painter().clone();
    w::rounded(&p, uv, button_bg(false, false, false, !uv_on), 6.0);
    // 乗せた・押した範囲だけ明るくする
    for (part, response, look, left) in [
        (toggle, toggle_r, toggle_look, true),
        (arrow, arrow_r, arrow_look, false),
    ] {
        let bg = if !left && menu_open {
            Some(t::ACCENT_DIM)
        } else if look.live && response.is_pointer_button_down_on() {
            Some(Color32::from_black_alpha(220))
        } else if look.live && response.hovered() {
            Some(Color32::from_black_alpha(120))
        } else {
            None
        };
        if let Some(bg) = bg {
            // 角は外側だけ丸く（内側の仕切りの側は角にしない）
            let radius = egui::CornerRadius {
                nw: if left { 6 } else { 0 },
                sw: if left { 6 } else { 0 },
                ne: if left { 0 } else { 6 },
                se: if left { 0 } else { 6 },
            };
            p.rect_filled(part, radius, bg);
        }
    }
    let fg = if !toggle_look.enabled {
        t::TEXT_DISABLED
    } else if uv_on {
        t::TEXT
    } else {
        t::TEXT_DIM
    };
    let fg_hot = if toggle_look.live && toggle_r.hovered() {
        Color32::WHITE
    } else {
        fg
    };
    w::icon(
        &p,
        Rect::from_min_size(toggle.min + vec2(6.0, 0.0), vec2(18.0, BUTTON_HEIGHT)),
        if uv_on {
            "visibility"
        } else {
            "visibility_off"
        },
        fg_hot,
        16.0,
    );
    w::text(
        &p,
        Rect::from_min_max(pos2(toggle.left() + 27.0, toggle.top()), toggle.max),
        LABEL,
        t::LABEL.with_color(fg_hot),
        Align::Left,
    );
    w::vline(
        &p,
        arrow.left(),
        uv.top() + 5.0,
        uv.bottom() - 5.0,
        Color32::from_white_alpha(50),
    );
    w::icon(
        &p,
        arrow,
        "arrow_drop_down",
        if !arrow_look.enabled {
            t::TEXT_DISABLED
        } else if menu_open || (arrow_look.live && arrow_r.hovered()) {
            Color32::WHITE
        } else {
            fg
        },
        18.0,
    );
}

/// UV のボタンの下に開く小さな欄: 線の色（色の見本。押すと色のウィンドウ）と不透明度（スライダー）。矩形を返す。
pub fn popover(ctx: &egui::Context, app: &mut AppState, anchor: Rect) -> Rect {
    let lang = app.lang;
    let width = 224.0;
    let height = PAD * 2.0 + 26.0 + 6.0 + 24.0;
    let pop = Rect::from_min_size(
        pos2(anchor.right() - width, anchor.bottom() + 4.0),
        vec2(width, height),
    );
    egui::Area::new(Id::new("canvasbar.uv.pop"))
        .order(egui::Order::Middle)
        .fixed_pos(pop.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.allocate_exact_size(pop.size(), Sense::hover());
            let p = ui.painter().clone();
            w::rounded(&p, pop, t::MENU_BG, 6.0);
            w::outline(&p, pop, t::SEPARATOR, 1.0, 6.0);
            let body = pop.shrink(PAD + 2.0);
            let row = Rect::from_min_size(body.min, vec2(body.width(), 26.0));
            w::text(
                &p,
                Rect::from_min_max(row.min, pos2(row.right() - 70.0, row.bottom())),
                lang.t("canvasbar-color"),
                t::LABEL,
                Align::Left,
            );
            let c = app.prefs.settings.uv_wireframe_color;
            let swatch =
                Rect::from_min_size(pos2(row.right() - 64.0, row.top() + 2.0), vec2(64.0, 22.0));
            let current = Pick {
                rgb: [c[0], c[1], c[2]],
                alpha: Some(c[3]),
            };
            if let Some(u) = color_window::field(
                ui,
                swatch,
                color_target(),
                lang.t("uv_wireframe-uv-wireframe"),
                current,
                lang.t("uv_wireframe-uv-wireframe-color-and-opacity-tip"),
                true,
            ) {
                let [r, g, b] = u.pick.rgb;
                app.prefs.settings.uv_wireframe_color = [r, g, b, u.pick.alpha.unwrap_or(c[3])];
            }
            // 不透明度（色のウィンドウの不透明度と同じ値）
            let c = app.prefs.settings.uv_wireframe_color;
            let slider = Rect::from_min_size(
                pos2(body.left(), row.bottom() + 6.0),
                vec2(body.width(), 24.0),
            );
            let spec =
                SliderSpec::new(lang.t("panels-opacity"), 0.0, 100.0, NumberFormat::int("%"));
            let out = w::slider(
                ui,
                slider,
                "canvasbar.uv.opacity",
                c[3] as f32 / 255.0 * 100.0,
                &spec,
            );
            if out.changed {
                app.prefs.settings.uv_wireframe_color[3] =
                    (out.value / 100.0 * 255.0).round().clamp(0.0, 255.0) as u8;
            }
            app.ui.canvas_bar.dragging = out.active || color_dragging(ui.ctx());
        });
    pop
}
