//! カラーのパネル（Unity 版の ColorPanel）: 彩度×明度の四角と色相の帯（または色相の円とその中の四角。右上の切り替え）、
//! 16 進の欄とアルファ、使った色の履歴。メインの色（描画色）とサブの色（背景色）の 2 枚は、左のツールの帯の一番下へ置く
//! （`color_swatch`）。色相は描き手の操作で決めた値を覚え、彩度や明度が 0 になっても失わない。

use egui::{
    pos2, vec2, Color32, ColorImage, Pos2, Rect, Sense, TextureHandle, TextureOptions, Ui,
    WidgetInfo, WidgetType,
};

use crate::notice::Source;
use crate::state::{hsv_to_rgb, parse_hex, to_hex, Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};

/// 色相の円の太さ（半径に対する割合）。
pub const RING_THICKNESS: f32 = 0.17;

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
    pub(crate) fn sv(&mut self, ctx: &egui::Context, hue: f32) -> egui::TextureId {
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

    pub(crate) fn ring(&mut self, ctx: &egui::Context) -> egui::TextureId {
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

/// 右上の切り替えのボタンの大きさ。
const TOGGLE: f32 = 22.0;
/// 小さな円で、ボタンを円の外側の角に置けないときに、円の脇（右か上）へ空ける分。
const TOGGLE_LANE: f32 = 26.0;

/// 欄の右上の角の切り替えのボタン。
pub fn toggle_rect(area: Rect) -> Rect {
    Rect::from_min_size(
        pos2(area.right() - TOGGLE, area.top()),
        vec2(TOGGLE, TOGGLE),
    )
}

/// 色相の選ぶ印（輪の上の白い円）の半径の、輪の太さに対する割合。
pub(crate) const MARKER_SCALE: f32 = 0.42;
/// 選ぶ印の縁の線の太さの半分。
const MARKER_HALF_STROKE: f32 = 1.0;

/// 直径 `wheel_width` の円の外へ、選ぶ印の輪がはみ出す量（印が輪の真上・真下・真左・真右にあるとき）。印の中心は輪の太さの真ん中（半径 w/2·(1−T/2)）、
/// 半径は w·T·`MARKER_SCALE`、縁の線の外側は半分だけ外へ出る。
pub fn marker_overhang(wheel_width: f32) -> f32 {
    wheel_width * RING_THICKNESS * (MARKER_SCALE - 0.25) + MARKER_HALF_STROKE
}

/// 一辺 `side` の正方形に、円と、はみ出す選ぶ印の輪が収まる円の直径。
fn wheel_diameter(side: f32) -> f32 {
    ((side - 2.0 * MARKER_HALF_STROKE) / (1.0 + 2.0 * RING_THICKNESS * (MARKER_SCALE - 0.25)))
        .max(0.0)
}

/// 欄の右上の角の切り替えのボタンが、円 `wheel`（欄の中の円の外接の正方形）と選ぶ印の輪に掛からない（余裕 1 pt）。
fn corner_toggle_clear(area: Rect, wheel: Rect) -> bool {
    let d = wheel.width();
    let toggle = toggle_rect(area);
    let nearest = pos2(
        wheel.center().x.clamp(toggle.left(), toggle.right()),
        wheel.center().y.clamp(toggle.top(), toggle.bottom()),
    );
    nearest.distance(wheel.center()) >= d * 0.5 + marker_overhang(d) + 1.0
}

/// 円の外接の正方形の一辺の上限（点）: 中身の幅 `width` まで。幅が狭く、正方形の欄の右上の角のボタンが円に掛かるときは、ボタンを円の脇へ空ける分
/// （`TOGGLE_LANE`）も高さに足す（円が幅いっぱいを保てるように）。
fn wheel_most(width: f32) -> f32 {
    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, width));
    let d = wheel_diameter(width);
    let full = Rect::from_center_size(area.center(), vec2(d, d));
    let lane = if corner_toggle_clear(area, full) {
        0.0
    } else {
        TOGGLE_LANE
    };
    (width + lane).clamp(72.0, 346.0)
}

/// 円の外接の正方形。円は欄の幅と高さの小さいほうをいっぱいに使い（選ぶ印の輪が円の外へはみ出す分は、四方に余白として取る）、右上の切り替えのボタンは
/// 円と印の外側の角（重ならない所）に置く。小さな円ではその角が円に掛かるので、そのときだけボタンの分を、右に空ける（横長の欄）か上に空ける（縦長の細い欄）かの、
/// 円が大きくなるほうに置く。
pub fn wheel_rect(area: Rect) -> Rect {
    let side = area.width().min(area.height()).max(0.0);
    let d = wheel_diameter(side);
    let full = Rect::from_center_size(area.center(), vec2(d, d));
    if corner_toggle_clear(area, full) {
        return full;
    }
    let beside = area.height().min(area.width() - TOGGLE_LANE).max(0.0);
    let below = (area.height() - TOGGLE_LANE).min(area.width()).max(0.0);
    let outer = if below > beside {
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
    };
    let d = wheel_diameter(outer.width());
    Rect::from_center_size(outer.center(), vec2(d, d))
}

/// 16 進とアルファを 1 行に並べるのに足りる幅（これより狭ければ 2 行に分けて、どちらも欄の幅で見せる）。
const HEX_ALPHA_ONE_LINE: f32 = 160.0;

/// 16 進とアルファの欄・使った色の行の高さと、行の間・上下の余白（円を大きく取るため詰めてある。字の大きさはほかの欄と同じ）。
const FIELD_HEIGHT: f32 = 18.0;
const FIELD_GAP: f32 = 3.0;
const FIELD_TOP: f32 = 3.0;
const RECENT_HEIGHT: f32 = 14.0;

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
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::Other,
            true,
            app.lang.pick("彩度と明度", "Saturation and value"),
        )
    });
}

pub fn show(ui: &mut Ui, app: &mut AppState, tex: &mut ColorTextures) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let ctx = ui.ctx().clone();
    app.color.sync_hsv();
    let mut rows = Rows::new(r, FIELD_TOP);
    let stacked = r.width() - 2.0 * t::PADDING < HEX_ALPHA_ONE_LINE;
    let lines = if stacked { 2.0 } else { 1.0 };
    // 使った色の行は、色があるときだけ取る（無いのに空けておくと、16 進とアルファの下が空く）
    let has_recent = !app.color.recent.is_empty();
    // 上の余白・円の下の間・16 進とアルファ（と下の間）・使った色（と下の間）
    let fixed = FIELD_TOP
        + FIELD_GAP
        + lines * (FIELD_HEIGHT + FIELD_GAP)
        + if has_recent {
            RECENT_HEIGHT + FIELD_GAP
        } else {
            0.0
        };
    // 円は欄の幅と高さの小さいほうまで大きくする（幅の広い欄で小さく見えないように。上限は 346）
    let most = if app.color.wheel {
        wheel_most(r.width() - 2.0 * t::PADDING)
    } else {
        // 四角と色相の帯は、2 枚の色を帯へ移して空いた高さの分、幅に合わせて縦にも伸ばす（細い欄は 160 のまま）
        ((r.width() - 2.0 * t::PADDING - 52.0) * 1.25).clamp(160.0, 320.0)
    };
    let sv_height = (r.height() - fixed).clamp(72.0, most);
    let area = rows.row(sv_height, FIELD_GAP);
    let toggle = toggle_rect(area);
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
        let m = wheel.width() * RING_THICKNESS * MARKER_SCALE;
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
        response.widget_info(|| {
            WidgetInfo::labeled(
                WidgetType::Other,
                true,
                app.lang.pick("色相の円", "Hue wheel"),
            )
        });
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
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Slider, true, app.lang.pick("色相", "Hue"))
        });
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
        [
            rows.row(FIELD_HEIGHT, FIELD_GAP),
            rows.row(FIELD_HEIGHT, FIELD_GAP),
        ]
    } else {
        let line = rows.row(FIELD_HEIGHT, FIELD_GAP);
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
            let typed = app.lang.quote(&typed);
            app.fail(
                Source::Color,
                app.lang.pick(
                    format!("{typed}は 16 進の色として読めません。"),
                    format!("{typed} is not a valid hex color."),
                ),
            );
        }
    }
    let spec = SliderSpec::new("A", 0.0, 100.0, NumberFormat::int("%")).tooltip(
        app.lang
            .pick("描画色のアルファ", "Alpha of the brush color"),
    );
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

    // 使った色（色があるときだけ行を取る）
    if !has_recent {
        return;
    }
    let recent = rows.row(RECENT_HEIGHT, FIELD_GAP);
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
}

#[cfg(test)]
mod wheel_layout_tests {
    use super::*;

    /// 切り替えのボタンが、円と選ぶ印の輪に掛からない（ボタンの矩形の、円の中心に最も近い点が、円と印の外）。
    fn toggle_clear_of(area: Rect, wheel: Rect) -> bool {
        let toggle = toggle_rect(area);
        let c = wheel.center();
        let nearest = pos2(
            c.x.clamp(toggle.left(), toggle.right()),
            c.y.clamp(toggle.top(), toggle.bottom()),
        );
        nearest.distance(c) >= wheel.width() * 0.5 + marker_overhang(wheel.width())
    }

    /// 選ぶ印の輪（円の真上・真下・真左・真右の、輪の太さの真ん中にあるとき）が収まる矩形。
    fn marker_extent(wheel: Rect) -> Rect {
        let w = wheel.width();
        let center_radius = w * 0.5 * (1.0 - RING_THICKNESS * 0.5);
        let reach = center_radius + w * RING_THICKNESS * MARKER_SCALE + MARKER_HALF_STROKE;
        Rect::from_center_size(wheel.center(), vec2(2.0 * reach, 2.0 * reach))
    }

    /// 細い欄でも、円は切り替えのボタンを脇へ空けて幅いっぱいを保つ（高さにその分を足す）。既定の並びの幅では足さない。
    #[test]
    fn a_narrow_column_keeps_the_wheel_at_the_full_width_by_adding_the_toggle_lane() {
        let inner = 140.0;
        let most = wheel_most(inner);
        assert_eq!(most, inner + TOGGLE_LANE);
        let tall = wheel_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(inner, most)));
        let square = wheel_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(inner, inner)));
        // 前（高さにボタンの分を足していた）の円: 幅いっぱいの正方形に収まる直径
        assert!(
            (tall.width() - wheel_diameter(inner)).abs() < 0.01,
            "{tall:?}"
        );
        assert!(tall.width() > square.width() + 20.0, "{tall:?} {square:?}");
        // 既定の 3 つの大きさ（960×640・1280×800・1600×900）の中身の幅は、足さない
        for inner in [176.0, 190.0, 205.0, 240.0, 346.0] {
            assert_eq!(wheel_most(inner), inner.clamp(72.0, 346.0), "{inner}");
        }
    }

    #[test]
    fn the_wheel_uses_the_smaller_of_the_width_and_height_with_room_for_the_marker_and_the_toggle_outside(
    ) {
        // 横長の欄（幅 300・高さ 160）: 高さいっぱい（選ぶ印の輪が収まる分を除く）
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 160.0));
        let wheel = wheel_rect(area);
        assert!(
            (marker_extent(wheel).height() - 160.0).abs() < 0.6,
            "{wheel:?}"
        );
        assert!(toggle_clear_of(area, wheel));
        // 縦長の細い欄（幅 110・高さ 300）: 幅いっぱい（ボタンの分を空けない）
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(110.0, 300.0));
        let wheel = wheel_rect(area);
        assert!(
            (marker_extent(wheel).width() - 110.0).abs() < 0.6,
            "{wheel:?}"
        );
        assert!(toggle_clear_of(area, wheel));
        // 正方形に近い欄（200×200）: 全部使い、ボタンは円の外の角
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 200.0));
        let wheel = wheel_rect(area);
        assert!(
            (marker_extent(wheel).width() - 200.0).abs() < 0.6,
            "{wheel:?}"
        );
        assert!(toggle_clear_of(area, wheel));
        // 小さな正方形（140×140）は、角が円に掛かるので、そのときだけボタンの分を空ける
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(140.0, 140.0));
        let wheel = wheel_rect(area);
        assert!(marker_extent(wheel).width() < 140.0 - 1.0);
        assert!(toggle_clear_of(area, wheel));
        // どの大きさでも、円と選ぶ印の輪は欄の中（上下左右とも）で、ボタンに掛からない
        for w in (72..400).step_by(7) {
            for h in (72..400).step_by(11) {
                let area = Rect::from_min_size(pos2(5.0, 9.0), vec2(w as f32, h as f32));
                let wheel = wheel_rect(area);
                assert!(
                    area.expand(0.01).contains_rect(marker_extent(wheel)),
                    "{area:?} {wheel:?}"
                );
                assert!(toggle_clear_of(area, wheel), "{area:?} {wheel:?}");
                assert!(
                    wheel.width()
                        >= wheel_diameter((w.min(h) as f32 - TOGGLE_LANE).max(0.0)) - 0.01,
                    "{area:?} {wheel:?}"
                );
            }
        }
    }
}
