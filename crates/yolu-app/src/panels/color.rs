//! カラーのパネル（Unity 版の ColorPanel）: 彩度×明度の四角と色相の帯（または色相の円とその中の四角。右上の切り替え）、
//! 16 進の欄とアルファ、使った色の履歴。左下にメインの色（描画色）とサブの色（背景色）を Photoshop の配置で重ね、
//! 入れ替え（X）と初期設定（D）のボタンを添える。色相は描き手の操作で決めた値を覚え、彩度や明度が 0 になっても失わない。

use egui::{
    pos2, vec2, Color32, ColorImage, Pos2, Rect, Sense, TextureHandle, TextureOptions, Ui,
    WidgetInfo, WidgetType,
};

use crate::state::{hsv_to_rgb, parse_hex, to_hex, Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};

/// 色相の円の太さ（半径に対する割合）。
pub const RING_THICKNESS: f32 = 0.17;
/// 左下のメインとサブの色の場所の高さ。
const SWATCH_BLOCK: f32 = 56.0;

/// 色のパネルの絵（彩度×明度は色相が変わったら作り直す）。
#[derive(Default)]
pub struct ColorTextures {
    sv: Option<(f32, TextureHandle)>,
    hue_bar: Option<TextureHandle>,
    ring: Option<TextureHandle>,
}

fn rgb32(r: f32, g: f32, b: f32) -> Color32 {
    Color32::from_rgb(w::to_byte(r), w::to_byte(g), w::to_byte(b))
}

impl ColorTextures {
    fn sv(&mut self, ctx: &egui::Context, hue: f32) -> egui::TextureId {
        if !self
            .sv
            .as_ref()
            .is_some_and(|(h, _)| (h - hue).abs() < 1e-5)
        {
            const N: usize = 64;
            let mut pixels = Vec::with_capacity(N * N);
            for y in 0..N {
                for x in 0..N {
                    let (r, g, b) = hsv_to_rgb(
                        hue,
                        x as f32 / (N - 1) as f32,
                        1.0 - y as f32 / (N - 1) as f32,
                    );
                    pixels.push(rgb32(r, g, b));
                }
            }
            let image = ColorImage::new([N, N], pixels);
            match &mut self.sv {
                Some((h, handle)) => {
                    handle.set(image, TextureOptions::LINEAR);
                    *h = hue;
                }
                None => {
                    self.sv = Some((
                        hue,
                        ctx.load_texture("color-sv", image, TextureOptions::LINEAR),
                    ))
                }
            }
        }
        self.sv.as_ref().expect("sv texture").1.id()
    }

    fn hue_bar(&mut self, ctx: &egui::Context) -> egui::TextureId {
        self.hue_bar
            .get_or_insert_with(|| {
                const N: usize = 128;
                let pixels = (0..N)
                    .map(|y| {
                        let (r, g, b) = hsv_to_rgb(1.0 - y as f32 / (N - 1) as f32, 1.0, 1.0);
                        rgb32(r, g, b)
                    })
                    .collect();
                ctx.load_texture(
                    "color-hue",
                    ColorImage::new([1, N], pixels),
                    TextureOptions::LINEAR,
                )
            })
            .id()
    }

    fn ring(&mut self, ctx: &egui::Context) -> egui::TextureId {
        self.ring
            .get_or_insert_with(|| {
                const N: usize = 256;
                let outer = N as f32 * 0.5;
                let inner = outer * (1.0 - RING_THICKNESS);
                let mut pixels = Vec::with_capacity(N * N);
                for y in 0..N {
                    for x in 0..N {
                        let (dx, dy) = (x as f32 + 0.5 - outer, y as f32 + 0.5 - outer);
                        let d = (dx * dx + dy * dy).sqrt();
                        let alpha = (outer - d).clamp(0.0, 1.0) * (d - inner).clamp(0.0, 1.0);
                        let mut hue = dx.atan2(-dy) / std::f32::consts::TAU;
                        if hue < 0.0 {
                            hue += 1.0;
                        }
                        let (r, g, b) = hsv_to_rgb(hue, 1.0, 1.0);
                        pixels.push(Color32::from_rgba_unmultiplied(
                            w::to_byte(r),
                            w::to_byte(g),
                            w::to_byte(b),
                            w::to_byte(alpha),
                        ));
                    }
                }
                ctx.load_texture(
                    "color-ring",
                    ColorImage::new([N, N], pixels),
                    TextureOptions::LINEAR,
                )
            })
            .id()
    }
}

/// 右上の切り替えのボタンの分の幅（円はその左の残りの幅に収める）。
const TOGGLE_LANE: f32 = 26.0;

/// 円の外接の正方形。右上の切り替えのボタンの分を、右に空ける（横長の欄）か上に空ける（縦長の細い欄）かの、円が大きくなるほうに置く。
pub fn wheel_rect(area: Rect) -> Rect {
    let beside = area.height().min(area.width() - TOGGLE_LANE).max(0.0);
    let below = (area.height() - TOGGLE_LANE).min(area.width()).max(0.0);
    if below > beside {
        let top = area.top() + TOGGLE_LANE;
        Rect::from_min_size(
            pos2(
                area.center().x - below * 0.5,
                top + (area.bottom() - top - below) * 0.5,
            ),
            vec2(below, below),
        )
    } else {
        let lane = area.width() - TOGGLE_LANE;
        Rect::from_min_size(
            pos2(
                area.left() + (lane - beside) * 0.5,
                area.top() + (area.height() - beside) * 0.5,
            ),
            vec2(beside, beside),
        )
    }
}

/// 16 進とアルファを 1 行に並べるのに足りる幅（これより狭ければ 2 行に分けて、どちらも欄の幅で見せる）。
const HEX_ALPHA_ONE_LINE: f32 = 180.0;

/// 円の中の彩度×明度の四角。
pub fn wheel_square(wheel: Rect) -> Rect {
    let inner = wheel.width() * 0.5 * (1.0 - RING_THICKNESS) - 4.0;
    let side = (inner * std::f32::consts::SQRT_2).floor().max(0.0);
    Rect::from_min_size(
        pos2(
            (wheel.center().x - side * 0.5).round(),
            (wheel.center().y - side * 0.5).round(),
        ),
        vec2(side, side),
    )
}

/// 円の上の点の色相（真上が赤、時計回り）。
pub fn hue_at(wheel: Rect, p: Pos2) -> f32 {
    let d = p - wheel.center();
    let a = d.x.atan2(-d.y) / std::f32::consts::TAU;
    if a < 0.0 {
        a + 1.0
    } else {
        a
    }
}

pub fn in_ring(wheel: Rect, p: Pos2) -> bool {
    let r = wheel.width() * 0.5;
    let d = p.distance(wheel.center());
    d <= r + 2.0 && d >= r * (1.0 - RING_THICKNESS) - 2.0
}

fn marker(p: &egui::Painter, at: Pos2, radius: f32) {
    p.circle_stroke(at, radius, egui::Stroke::new(2.0, Color32::BLACK));
    p.circle_stroke(at, radius - 1.0, egui::Stroke::new(1.5, Color32::WHITE));
}

fn sv_square(ui: &mut Ui, app: &mut AppState, tex: &mut ColorTextures, r: Rect, id: &str) {
    let response = ui.interact(r, ui.make_persistent_id(id), Sense::click_and_drag());
    if response.is_pointer_button_down_on() {
        if let Some(p) = response.interact_pointer_pos() {
            app.color.pick_sv(
                (p.x - r.left()) / r.width(),
                1.0 - (p.y - r.top()) / r.height(),
            );
        }
    }
    let texture = tex.sv(ui.ctx(), app.color.hue);
    let p = ui.painter();
    p.image(
        texture,
        r,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    w::outline(p, r, t::BORDER, 1.0, 0.0);
    let at = pos2(
        r.left() + app.color.sat * r.width(),
        r.top() + (1.0 - app.color.val) * r.height(),
    );
    marker(&p.with_clip_rect(r.expand(8.0)), at, 6.0);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, app.lang.pick("彩度と明度", "Saturation and value")));
}

pub fn show(ui: &mut Ui, app: &mut AppState, tex: &mut ColorTextures) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let ctx = ui.ctx().clone();
    app.color.sync_hsv();
    let mut rows = Rows::new(r, 8.0);
    let stacked = r.width() - 2.0 * t::PADDING < HEX_ALPHA_ONE_LINE;
    let lines = if stacked { 2.0 } else { 1.0 };
    let fixed = 8.0 + 6.0 + lines * (22.0 + 6.0) + 16.0 + 6.0 + SWATCH_BLOCK + 8.0;
    // 円は欄の幅いっぱいまで大きくする（幅の広い欄で小さく見えないように。上限は 320）
    let most = if app.color.wheel {
        // 細い欄では切り替えのボタンを円の上に置くので、その分も高さに足す
        (r.width() - 2.0 * t::PADDING + TOGGLE_LANE).clamp(72.0, 346.0)
    } else {
        160.0
    };
    let sv_height = (r.height() - fixed).clamp(72.0, most);
    let area = rows.row(sv_height, 6.0);
    let toggle = Rect::from_min_size(pos2(area.right() - 22.0, area.top()), vec2(22.0, 22.0));
    if app.color.wheel {
        let wheel = wheel_rect(area);
        let id = ui.make_persistent_id("color.wheel");
        let response = ui.interact(wheel, id, Sense::click_and_drag());
        // 押した所が輪なら色相、中の四角なら彩度と明度（ドラッグの間は押したほうのまま）
        let mode_id = id.with("mode");
        if response.drag_started()
            || (response.is_pointer_button_down_on() && ui.input(|i| i.pointer.any_pressed()))
        {
            let origin = ui.input(|i| i.pointer.press_origin());
            let mode = origin
                .map(|o| {
                    if in_ring(wheel, o) {
                        1u8
                    } else if wheel_square(wheel).contains(o) {
                        2
                    } else {
                        0
                    }
                })
                .unwrap_or(0);
            ui.data_mut(|d| d.insert_temp(mode_id, mode));
        }
        if response.is_pointer_button_down_on() {
            let mode: u8 = ui.data(|d| d.get_temp(mode_id).unwrap_or(0));
            if let Some(p) = response.interact_pointer_pos() {
                match mode {
                    1 => app.color.set_hue(hue_at(wheel, p)),
                    2 => {
                        let sq = wheel_square(wheel);
                        app.color.pick_sv(
                            (p.x - sq.left()) / sq.width(),
                            1.0 - (p.y - sq.top()) / sq.height(),
                        );
                    }
                    _ => {}
                }
            }
        }
        let ring = tex.ring(&ctx);
        let p = ui.painter();
        p.image(
            ring,
            wheel,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let radius = wheel.width() * 0.5 * (1.0 - RING_THICKNESS * 0.5);
        let a = app.color.hue * std::f32::consts::TAU;
        let m = wheel.width() * RING_THICKNESS * 0.42;
        marker(
            p,
            pos2(
                wheel.center().x + a.sin() * radius,
                wheel.center().y - a.cos() * radius,
            ),
            m,
        );
        let sq = wheel_square(wheel);
        let texture = tex.sv(&ctx, app.color.hue);
        p.image(
            texture,
            sq,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        w::outline(p, sq, t::BORDER, 1.0, 0.0);
        marker(
            p,
            pos2(
                sq.left() + app.color.sat * sq.width(),
                sq.top() + (1.0 - app.color.val) * sq.height(),
            ),
            6.0,
        );
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, app.lang.pick("色相の円", "Hue wheel")));
    } else {
        let sv = Rect::from_min_size(
            area.min,
            vec2((area.width() - 52.0).max(10.0), area.height()),
        );
        let hue = Rect::from_min_size(
            pos2(sv.right() + 8.0, area.top()),
            vec2(18.0, area.height()),
        );
        sv_square(ui, app, tex, sv, "color.sv");
        let response = ui.interact(
            hue,
            ui.make_persistent_id("color.hue"),
            Sense::click_and_drag(),
        );
        if response.is_pointer_button_down_on() {
            if let Some(p) = response.interact_pointer_pos() {
                app.color
                    .set_hue((1.0 - (p.y - hue.top()) / hue.height()).clamp(0.0, 1.0));
            }
        }
        let texture = tex.hue_bar(&ctx);
        let p = ui.painter();
        p.image(
            texture,
            hue,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        w::outline(p, hue, t::BORDER, 1.0, 0.0);
        let y = hue.top() + (1.0 - app.color.hue) * hue.height();
        w::outline(
            &p.with_clip_rect(hue.expand(4.0)),
            Rect::from_min_size(
                pos2(hue.left() - 2.0, y - 3.0),
                vec2(hue.width() + 4.0, 6.0),
            ),
            Color32::WHITE,
            1.5,
            2.0,
        );
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Slider, true, app.lang.pick("色相", "Hue")));
    }
    let tip = if app.color.wheel {
        app.lang.pick("四角と色相の帯", "Square and hue bar")
    } else {
        app.lang.pick("色相の円", "Hue wheel")
    };
    if w::icon_button(
        ui,
        toggle,
        "color.mode",
        if app.color.wheel {
            "color_square"
        } else {
            "target"
        },
        tip,
        false,
        true,
        16.0,
    )
    .clicked()
    {
        app.apply(Action::ToggleColorWheel);
    }

    // 16 進とアルファ
    // 16 進の欄を広めに取る（同梱の書体の数字は幅広で、「#RRGGBB」が半分の幅に収まらない）
    let cells = if stacked {
        // 狭い欄: 16 進とアルファを 2 行に（「#RRGGBB」が欠けないように）
        [rows.row(22.0, 6.0), rows.row(22.0, 6.0)]
    } else {
        let line = rows.row(22.0, 6.0);
        let hex_width = ((line.width() - 6.0) * 0.52).round();
        [
            Rect::from_min_size(line.min, vec2(hex_width, line.height())),
            Rect::from_min_max(pos2(line.left() + hex_width + 6.0, line.top()), line.max),
        ]
    };
    let hex = format!("#{}", to_hex(app.color.main));
    if let Some(typed) = w::text_field(
        ui,
        cells[0],
        "color.hex",
        &hex,
        Some(app.lang.pick("16 進の色（#RRGGBB）", "Hex color (#RRGGBB)")),
        false,
    )
    .committed
    {
        if let Some(rgb) = parse_hex(&typed) {
            app.color
                .set_main([rgb[0], rgb[1], rgb[2], app.color.main[3]]);
        } else {
            app.message = format!("{}: {typed}", app.lang.pick("16 進の色として読めません", "Invalid hex color"));
        }
    }
    let spec = SliderSpec::new("A", 0.0, 100.0, NumberFormat::int("%")).tooltip(app.lang.pick("描画色のアルファ", "Alpha of the brush color"));
    let alpha = w::slider(
        ui,
        cells[1],
        "color.alpha",
        app.color.main[3] * 100.0,
        &spec,
    );
    if alpha.changed {
        let mut c = app.color.main;
        c[3] = alpha.value / 100.0;
        app.color.set_main(c);
    }

    // 使った色
    let recent = rows.row(16.0, 6.0);
    let size = recent.height();
    let columns = ((recent.width() + 3.0) / (size + 3.0)).floor().max(1.0) as usize;
    let recent_colors = app.color.recent.clone();
    for (i, c) in recent_colors.iter().take(columns).enumerate() {
        let cell = Rect::from_min_size(
            pos2(recent.left() + i as f32 * (size + 3.0), recent.top()),
            vec2(size, size),
        );
        let response = ui.interact(
            cell,
            ui.make_persistent_id(("color.recent", i)),
            Sense::click(),
        );
        let p = ui.painter();
        w::rounded(p, cell, rgb32(c[0], c[1], c[2]), 2.0);
        w::outline(
            p,
            cell,
            if response.hovered() {
                t::ACCENT
            } else {
                t::BORDER
            },
            1.0,
            2.0,
        );
        let label = format!("#{}{:02X}", to_hex(*c), w::to_byte(c[3]));
        if response.clicked() {
            app.color.set_main(*c);
        }
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
    }

    // 左下: メインの色とサブの色（描画色が左上、背景色が右下に重なる。右上に入れ替え、左下に初期設定）
    let bottom = (r.bottom() - 10.0).max(rows.y() + SWATCH_BLOCK - 10.0);
    let left = r.left() + t::PADDING + 4.0;
    let front = Rect::from_min_size(pos2(left, bottom - 42.0), vec2(22.0, 22.0));
    let back = Rect::from_min_size(pos2(left + 11.0, bottom - 31.0), vec2(22.0, 22.0));
    if w::color_swatch(
        ui,
        back,
        "color.sub",
        app.color.sub,
        &format!(
            "{} #{}",
            app.lang.pick("サブの色（背景色）。押すとメインの色と入れ替えます", "Background color. Click to swap with the foreground color."),
            to_hex(app.color.sub)
        ),
        true,
    )
    .clicked()
    {
        app.apply(Action::SwapColors);
    }
    w::fill(ui.painter(), front.expand(1.0), t::PANEL_BG);
    let _ = w::color_swatch(
        ui,
        front,
        "color.main",
        app.color.main,
        &format!(
            "{} #{}",
            app.lang.pick("メインの色（描画色。ブラシで塗る色）", "Foreground color (the color the brush paints)"),
            to_hex(app.color.main)
        ),
        true,
    );
    if w::icon_button(
        ui,
        Rect::from_min_size(pos2(left + 21.0, bottom - 56.0), vec2(14.0, 14.0)),
        "color.swap",
        "swap_horiz",
        app.lang.pick("メインとサブの色を入れ替え（X）", "Swap foreground and background colors (X)"),
        false,
        true,
        12.0,
    )
    .clicked()
    {
        app.apply(Action::SwapColors);
    }
    if w::icon_button(
        ui,
        Rect::from_min_size(pos2(left - 2.0, bottom - 8.0), vec2(14.0, 14.0)),
        "color.default",
        "restart_alt",
        app.lang.pick("初期設定の色（D）", "Default colors (D)"),
        false,
        true,
        11.0,
    )
    .clicked()
    {
        app.apply(Action::DefaultColors);
    }
}

#[cfg(test)]
mod wheel_layout_tests {
    use super::*;

    #[test]
    fn a_narrow_tall_panel_puts_the_toggle_above_and_the_wheel_uses_the_whole_width() {
        // 狭い欄（幅 110・高さ 300）: 右に空けると 84、上に空けると 110。大きいほう
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(110.0, 300.0));
        let wheel = wheel_rect(area);
        assert_eq!(wheel.width(), 110.0);
        assert!(wheel.top() >= TOGGLE_LANE);
        // 横長の欄（幅 300・高さ 160）: 右に空けて高さいっぱい
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 160.0));
        let wheel = wheel_rect(area);
        assert_eq!(wheel.width(), 160.0);
        assert!(wheel.right() <= 300.0 - TOGGLE_LANE);
    }
}
