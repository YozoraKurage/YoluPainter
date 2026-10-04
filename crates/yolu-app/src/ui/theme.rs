//! ペイントソフトとしての見た目（Unity 版の `PaintTheme` と同じ配色と寸法）。暗いグレーと青のアクセント。
//! 部品の背景は `widgets` が色の矩形と角丸で描き、egui の標準の見た目には頼らない（文字の入力欄とスクロールだけ egui の部品）。

use egui::{Color32, FontFamily, FontId};

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const WINDOW_BG: Color32 = hex(0x1B1B1D);
pub const PANEL_BG: Color32 = hex(0x252528);
pub const PANEL_HEADER: Color32 = hex(0x2E2E32);
pub const MENU_BG: Color32 = hex(0x202023);
pub const CONTROL_BG: Color32 = hex(0x18181A);
pub const CONTROL_HOVER: Color32 = hex(0x36363B);
pub const CONTROL_ACTIVE: Color32 = hex(0x404048);
pub const BORDER: Color32 = hex(0x111113);
pub const SEPARATOR: Color32 = hex(0x343438);
pub const CANVAS_BG: Color32 = hex(0x131314);
pub const ACCENT: Color32 = hex(0x3D8EF0);
pub const ACCENT_DIM: Color32 = hex(0x2B5D9C);
/// Unity 版の (.24, .56, .94, .22)（egui の色は乗算済み）。
pub const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(13, 31, 53, 56);
pub const TEXT: Color32 = hex(0xD9D9DC);
pub const TEXT_DIM: Color32 = hex(0x9A9AA0);
pub const TEXT_DISABLED: Color32 = hex(0x5C5C62);
pub const WARNING: Color32 = hex(0xE8B03C);
/// 接続できている印（Live Link の入口）。
pub const OK: Color32 = hex(0x4CAF6A);
pub const ERROR: Color32 = hex(0xE5534B);
pub const SLIDER_FILL: Color32 = hex(0x355F96);
pub const SLIDER_FILL_HOVER: Color32 = hex(0x3F70B0);
/// プロパティの欄の大見出しの帯（パネルの地より濃い）。
pub const SECTION_BAND: Color32 = hex(0x1E1E21);
pub const SECTION_BAND_HOVER: Color32 = hex(0x2A2A2F);
/// 透明を表す市松（Unity 版の `PaintGui.Checker`）。
pub const CHECKER_LIGHT: Color32 = Color32::from_rgb(107, 107, 112);
pub const CHECKER_DARK: Color32 = Color32::from_rgb(77, 77, 82);

pub const MENU_BAR_HEIGHT: f32 = 24.0;
pub const OPTIONS_BAR_HEIGHT: f32 = 36.0;
pub const STATUS_BAR_HEIGHT: f32 = 22.0;
pub const TOOL_STRIP_WIDTH: f32 = 44.0;
pub const DOCK_WIDTH: f32 = 300.0;
pub const ROW_HEIGHT: f32 = 22.0;
pub const PADDING: f32 = 8.0;
/// プロパティの欄の大見出しの文字の位置（▸/▾ の右）。
pub const SECTION_TITLE_X: f32 = 20.0;
/// 大見出しの中身の字下げ。
pub const SECTION_INDENT: f32 = 12.0;
/// 2 行のスライダーの行の高さ。
pub const SLIDER_ROW_HEIGHT: f32 = 34.0;
/// ドックのパネルの見出しの高さ。
pub const PANEL_HEADER_HEIGHT: f32 = 24.0;
/// プロパティの欄のタブの帯の高さ。
pub const PROPERTY_TAB_STRIP_HEIGHT: f32 = 30.0;

/// 太字の書体の名前（`fonts` が入れる。無ければ普通の書体）。
pub const BOLD: &str = "bold";

/// 文字の形（Unity 版の `PaintTheme` の GUIStyle に当たる。大きさと色と寄せ）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub bold: bool,
    pub color: Color32,
}

impl TextStyle {
    pub fn font(&self) -> FontId {
        FontId::new(
            self.size,
            if self.bold {
                FontFamily::Name(BOLD.into())
            } else {
                FontFamily::Proportional
            },
        )
    }
    pub fn with_color(self, color: Color32) -> TextStyle {
        TextStyle { color, ..self }
    }
}

pub const LABEL: TextStyle = TextStyle {
    size: 12.0,
    bold: false,
    color: TEXT,
};
pub const LABEL_DIM: TextStyle = TextStyle {
    size: 11.0,
    bold: false,
    color: TEXT_DIM,
};
pub const LABEL_SMALL: TextStyle = TextStyle {
    size: 10.0,
    bold: false,
    color: TEXT_DIM,
};
pub const LABEL_BOLD: TextStyle = TextStyle {
    size: 12.0,
    bold: true,
    color: TEXT,
};
pub const HEADER: TextStyle = TextStyle {
    size: 11.0,
    bold: true,
    color: TEXT,
};
pub const VALUE: TextStyle = TextStyle {
    size: 11.0,
    bold: false,
    color: TEXT,
};

/// egui の部品（文字の入力欄・スクロールバー・ツールチップ・ドックの境目）をこの配色にする。
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        use egui::TextStyle as T;
        style.text_styles = [
            (T::Small, FontId::new(10.0, FontFamily::Proportional)),
            (T::Body, FontId::new(12.0, FontFamily::Proportional)),
            (T::Button, FontId::new(12.0, FontFamily::Proportional)),
            (T::Heading, FontId::new(14.0, FontFamily::Name(BOLD.into()))),
            (T::Monospace, FontId::new(12.0, FontFamily::Monospace)),
        ]
        .into();
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = PANEL_BG;
        v.window_fill = PANEL_BG;
        v.window_stroke = egui::Stroke::new(1.0, SEPARATOR);
        v.extreme_bg_color = CONTROL_BG;
        v.faint_bg_color = PANEL_HEADER;
        v.code_bg_color = CONTROL_BG;
        v.hyperlink_color = ACCENT;
        v.warn_fg_color = WARNING;
        v.error_fg_color = ERROR;
        v.selection.bg_fill = ACCENT_DIM;
        v.selection.stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
        v.text_cursor.stroke = egui::Stroke::new(1.5, TEXT);
        v.window_corner_radius = egui::CornerRadius::same(8);
        v.menu_corner_radius = egui::CornerRadius::same(8);
        let w = &mut v.widgets;
        for (state, bg) in [
            (&mut w.noninteractive, PANEL_BG),
            (&mut w.inactive, CONTROL_BG),
            (&mut w.hovered, CONTROL_HOVER),
            (&mut w.active, CONTROL_ACTIVE),
            (&mut w.open, CONTROL_ACTIVE),
        ] {
            state.bg_fill = bg;
            state.weak_bg_fill = bg;
            state.corner_radius = egui::CornerRadius::same(3);
            state.fg_stroke = egui::Stroke::new(1.0, TEXT);
            state.bg_stroke = egui::Stroke::new(1.0, BORDER);
            state.expansion = 0.0;
        }
        w.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT);
        w.noninteractive.bg_stroke = egui::Stroke::new(1.0, SEPARATOR);
        w.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT_DIM);
        w.active.bg_stroke = egui::Stroke::new(1.0, ACCENT);
        style.spacing.item_spacing = egui::vec2(4.0, 4.0);
        style.spacing.button_padding = egui::vec2(6.0, 2.0);
        style.spacing.scroll = egui::style::ScrollStyle::thin();
        style.interaction.tooltip_delay = 0.5;
    });
}
