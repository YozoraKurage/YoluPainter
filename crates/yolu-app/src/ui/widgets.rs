//! ペイントソフトの部品（絶対座標。Unity 版の `PaintGui` の寸法と色）。背景・枠・塗りは色の矩形と角丸で描き、egui の標準の
//! ボタンやスライダーの見た目は使わない。部品は押した・変えたの結果を返し、状態は呼ぶ側が持つ（Unity 版と同じ即時の形）。
//! 試験と読み上げのため、押せる部品には名前（`WidgetInfo`）を付ける（egui_kittest は名前で部品を探す）。

use egui::{
    pos2, vec2, Align2, Color32, Id, Painter, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    WidgetInfo, WidgetType,
};

use super::icons;
use super::theme::{self as t, TextStyle};

// ───────── 描くだけの部品 ─────────

pub fn fill(p: &Painter, r: Rect, c: Color32) {
    p.rect_filled(r, 0.0, c);
}

pub fn rounded(p: &Painter, r: Rect, c: Color32, radius: f32) {
    p.rect_filled(r, radius, c);
}

/// 内側に引く枠（Unity の `GUI.DrawTexture` の枠と同じく矩形の内側）。
pub fn outline(p: &Painter, r: Rect, c: Color32, width: f32, radius: f32) {
    p.rect_stroke(r, radius, Stroke::new(width, c), StrokeKind::Inside);
}

pub fn hline(p: &Painter, x0: f32, x1: f32, y: f32, c: Color32) {
    fill(p, Rect::from_min_max(pos2(x0, y), pos2(x1, y + 1.0)), c);
}

pub fn vline(p: &Painter, x: f32, y0: f32, y1: f32, c: Color32) {
    fill(p, Rect::from_min_max(pos2(x, y0), pos2(x + 1.0, y1)), c);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// rect の中に 1 行の文字を書く（縦は中央。rect の外は切る）。
pub fn text(p: &Painter, r: Rect, s: &str, style: TextStyle, align: Align) {
    if s.is_empty() || r.width() <= 0.0 {
        return;
    }
    let galley = p.layout_no_wrap(s.to_owned(), style.font(), style.color);
    let size = galley.size();
    let x = match align {
        Align::Left => r.left(),
        Align::Center => r.center().x - size.x / 2.0,
        Align::Right => r.right() - size.x,
    };
    let y = r.center().y - size.y / 2.0;
    p.with_clip_rect(r.intersect(p.clip_rect()))
        .galley(pos2(x, y), galley, style.color);
}

/// 折り返す文字（説明文）。
pub fn wrapped_text(p: &Painter, r: Rect, s: &str, style: TextStyle) -> f32 {
    let galley = p.layout(s.to_owned(), style.font(), style.color, r.width().max(1.0));
    let h = galley.size().y;
    p.with_clip_rect(r.intersect(p.clip_rect()))
        .galley(r.min, galley, style.color);
    h
}

pub fn text_width(p: &Painter, s: &str, style: TextStyle) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    p.layout_no_wrap(s.to_owned(), style.font(), style.color)
        .size()
        .x
}

/// 幅に収まらなければ後ろを「…」で詰める。
pub fn fit(p: &Painter, s: &str, width: f32, style: TextStyle) -> String {
    if text_width(p, s, style) <= width {
        return s.to_owned();
    }
    let chars: Vec<char> = s.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if text_width(p, &candidate, style) <= width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        String::new()
    } else {
        chars[..lo].iter().collect::<String>() + "…"
    }
}

pub fn icon(p: &Painter, r: Rect, name: &str, color: Color32, size: f32) {
    icons::paint(p, r, name, color, size);
}

/// 透明を表す市松（cell の大きさ）。
pub fn checker(p: &Painter, r: Rect, cell: f32) {
    fill(p, r, t::CHECKER_LIGHT);
    let p = p.with_clip_rect(r.intersect(p.clip_rect()));
    let mut y = 0.0;
    let mut row = 0;
    while y < r.height() {
        let mut x = if row % 2 == 0 { 0.0 } else { cell };
        while x < r.width() {
            let cr = Rect::from_min_size(
                pos2(r.left() + x, r.top() + y),
                vec2(cell.min(r.width() - x), cell.min(r.height() - y)),
            );
            p.rect_filled(cr, 0.0, t::CHECKER_DARK);
            x += cell * 2.0;
        }
        y += cell;
        row += 1;
    }
}

/// 縦の帯の区切り。
pub fn strip_separator(p: &Painter, r: Rect) {
    hline(
        p,
        r.left() + 6.0,
        r.right() - 6.0,
        r.center().y.round(),
        t::SEPARATOR,
    );
}

// ───────── 押せる部品 ─────────

fn interact(ui: &mut Ui, r: Rect, id: Id, enabled: bool, sense: Sense) -> Response {
    ui.interact(r, id, if enabled { sense } else { Sense::hover() })
}

fn with_tooltip(response: Response, tooltip: Option<&str>) -> Response {
    match tooltip {
        Some(tip) if !tip.is_empty() => response.on_hover_text(tip),
        _ => response,
    }
}

/// アイコンだけのボタン（ツール・パネルの操作）。selected は押し込まれた見た目。名前（試験・読み上げ）はツールチップ。
#[allow(clippy::too_many_arguments)]
pub fn icon_button(
    ui: &mut Ui,
    r: Rect,
    id_salt: impl egui::AsIdSalt,
    icon_name: &str,
    tooltip: &str,
    selected: bool,
    enabled: bool,
    size: f32,
) -> Response {
    let id = ui.make_persistent_id(id_salt);
    let response = interact(ui, r, id, enabled, Sense::click());
    let hover = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let p = ui.painter();
    if selected {
        rounded(p, r, t::ACCENT_DIM, 4.0);
    } else if pressed {
        rounded(p, r, t::CONTROL_ACTIVE, 4.0);
    } else if hover {
        rounded(p, r, t::CONTROL_HOVER, 4.0);
    }
    let color = if !enabled {
        t::TEXT_DISABLED
    } else if selected || hover {
        Color32::WHITE
    } else {
        t::TEXT
    };
    icon(p, r, icon_name, color, size);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, selected, tooltip));
    with_tooltip(response, Some(tooltip))
}

/// ツールの帯のボタン（ツールのアイコンは tools/<id> と tools/<id>_selected）。
pub fn tool_button(ui: &mut Ui, r: Rect, tool_id: &str, tooltip: &str, selected: bool) -> Response {
    let id = ui.make_persistent_id(("tool", tool_id));
    let response = ui.interact(r, id, Sense::click());
    let hover = response.hovered();
    let p = ui.painter();
    if selected {
        rounded(p, r, t::ACCENT_DIM, 4.0);
    } else if response.is_pointer_button_down_on() {
        rounded(p, r, t::CONTROL_ACTIVE, 4.0);
    } else if hover {
        rounded(p, r, t::CONTROL_HOVER, 4.0);
    }
    let name = if selected {
        format!("tools/{tool_id}_selected")
    } else {
        format!("tools/{tool_id}")
    };
    icon(
        p,
        r,
        &name,
        if selected || hover {
            Color32::WHITE
        } else {
            t::TEXT
        },
        22.0,
    );
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, tooltip));
    with_tooltip(response, Some(tooltip))
}

/// 文字のボタン（primary は青）。
#[allow(clippy::too_many_arguments)]
pub fn button(
    ui: &mut Ui,
    r: Rect,
    id_salt: impl egui::AsIdSalt,
    label: &str,
    primary: bool,
    enabled: bool,
    tooltip: Option<&str>,
    icon_name: Option<&str>,
) -> Response {
    let id = ui.make_persistent_id(id_salt);
    let response = interact(ui, r, id, enabled, Sense::click());
    let hover = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let bg = if !enabled {
        t::CONTROL_BG
    } else if primary {
        if pressed {
            t::ACCENT_DIM
        } else if hover {
            t::SLIDER_FILL_HOVER
        } else {
            t::ACCENT
        }
    } else if pressed {
        t::CONTROL_ACTIVE
    } else if hover {
        t::CONTROL_HOVER
    } else {
        t::PANEL_HEADER
    };
    let p = ui.painter();
    rounded(p, r, bg, 4.0);
    if !primary {
        outline(p, r, t::BORDER, 1.0, 4.0);
    }
    let color = if !enabled {
        t::TEXT_DISABLED
    } else if primary {
        Color32::WHITE
    } else {
        t::TEXT
    };
    match icon_name {
        Some(name) => {
            let w = text_width(p, label, t::LABEL) + 22.0;
            let start = Rect::from_min_size(
                pos2(r.center().x - w / 2.0, r.top()),
                vec2(18.0, r.height()),
            );
            icon(p, start, name, color, 16.0);
            text(
                p,
                Rect::from_min_max(pos2(start.right() + 4.0, r.top()), r.max),
                label,
                t::LABEL.with_color(color),
                Align::Left,
            );
        }
        None => text(p, r, label, t::LABEL.with_color(color), Align::Center),
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    with_tooltip(response, tooltip)
}

/// 大見出しの結果。
pub struct SectionOutcome {
    pub open: bool,
    pub reset: bool,
}

/// プロパティの欄の大見出し（折りたためる）: 全幅の帯（濃い地・太字）、左端に ▸/▾。reset を渡すと右端に「既定に戻す」。
pub fn section_header(
    ui: &mut Ui,
    r: Rect,
    id_salt: impl egui::AsIdSalt,
    title: &str,
    open: bool,
    icon_name: Option<&str>,
    reset: Option<&str>,
) -> SectionOutcome {
    let id = ui.make_persistent_id(id_salt);
    let response = ui.interact(r, id, Sense::click());
    let hover = response.hovered();
    {
        let p = ui.painter();
        fill(
            p,
            r,
            if hover {
                t::SECTION_BAND_HOVER
            } else {
                t::SECTION_BAND
            },
        );
        hline(p, r.left(), r.right(), r.top(), t::BORDER);
        hline(p, r.left(), r.right(), r.bottom() - 1.0, t::BORDER);
    }
    let mut right = r.right() - 2.0;
    let mut reset_clicked = false;
    if let Some(tip) = reset {
        let b = Rect::from_min_size(
            pos2(right - 22.0, r.top() + 2.0),
            vec2(22.0, r.height() - 4.0),
        );
        right = b.left() - 2.0;
        reset_clicked = icon_button(
            ui,
            b,
            id.with("reset"),
            "restart_alt",
            tip,
            false,
            true,
            14.0,
        )
        .clicked();
    }
    let p = ui.painter();
    icon(
        p,
        Rect::from_min_size(pos2(r.left() + 3.0, r.top()), vec2(16.0, r.height())),
        if open { "expand_more" } else { "chevron_right" },
        t::TEXT,
        15.0,
    );
    let mut x = r.left() + t::SECTION_TITLE_X;
    if let Some(name) = icon_name {
        icon(
            p,
            Rect::from_min_size(pos2(x, r.top()), vec2(16.0, r.height())),
            name,
            t::TEXT_DIM,
            14.0,
        );
        x += 20.0;
    }
    let shown = fit(p, title, right - x, t::HEADER);
    text(
        p,
        Rect::from_min_max(pos2(x, r.top()), pos2(right, r.bottom())),
        &shown,
        t::HEADER,
        Align::Left,
    );
    response.widget_info(|| WidgetInfo::selected(WidgetType::CollapsingHeader, true, open, title));
    let clicked = response.clicked();
    SectionOutcome {
        open: if clicked { !open } else { open },
        reset: reset_clicked,
    }
}

/// 大見出しの中の小見出し（帯ではなく ▸/▾ と文字と、右へ伸びる細い線）。
pub fn subsection_header(
    ui: &mut Ui,
    r: Rect,
    id_salt: impl egui::AsIdSalt,
    title: &str,
    open: bool,
) -> bool {
    let id = ui.make_persistent_id(id_salt);
    let response = ui.interact(r, id, Sense::click());
    let p = ui.painter();
    if response.hovered() {
        rounded(
            p,
            Rect::from_min_max(pos2(r.left() - 2.0, r.top()), r.max),
            t::CONTROL_HOVER,
            3.0,
        );
    }
    icon(
        p,
        Rect::from_min_size(r.min, vec2(14.0, r.height())),
        if open { "expand_more" } else { "chevron_right" },
        t::TEXT_DIM,
        13.0,
    );
    let x = r.left() + 16.0;
    let shown = fit(p, title, r.right() - x - 4.0, t::LABEL_BOLD);
    text(
        p,
        Rect::from_min_max(pos2(x, r.top()), r.max),
        &shown,
        t::LABEL_BOLD.with_color(if open { t::TEXT } else { t::TEXT_DIM }),
        Align::Left,
    );
    let line = x + text_width(p, &shown, t::LABEL_BOLD) + 6.0;
    if line < r.right() - 4.0 {
        hline(p, line, r.right() - 4.0, r.center().y.round(), t::SEPARATOR);
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::CollapsingHeader, true, open, title));
    if response.clicked() {
        !open
    } else {
        open
    }
}

/// アイコンと名前のタブの帯（プロパティの欄の頭）。押されたタブの番号を返す（押されなければ active）。幅が足りなければ名前を詰め、
/// さらに狭ければアイコンだけ。
pub fn tab_strip(
    ui: &mut Ui,
    r: Rect,
    id_salt: impl egui::AsIdSalt,
    labels: &[&str],
    icon_names: &[&str],
    active: usize,
) -> usize {
    {
        let p = ui.painter();
        fill(p, r, t::PANEL_HEADER);
        hline(p, r.left(), r.right(), r.bottom() - 1.0, t::BORDER);
    }
    let n = labels.len();
    if n == 0 {
        return active;
    }
    let base = ui.make_persistent_id(id_salt);
    let w = (r.width() - 4.0) / n as f32;
    let mut result = active;
    for i in 0..n {
        let tr = Rect::from_min_size(
            pos2((r.left() + 2.0 + i as f32 * w).round(), r.top() + 2.0),
            vec2(w.round() - 1.0, r.height() - 3.0),
        );
        let on = i == active;
        let response = ui.interact(tr, base.with(i), Sense::click());
        if response.clicked() && !on {
            result = i;
        }
        let p = ui.painter();
        if on {
            rounded(p, tr, t::PANEL_BG, 3.0);
            fill(
                p,
                Rect::from_min_size(
                    pos2(tr.left() + 4.0, tr.bottom() - 2.0),
                    vec2(tr.width() - 8.0, 2.0),
                ),
                t::ACCENT,
            );
        } else if response.hovered() || response.is_pointer_button_down_on() {
            rounded(p, tr, t::CONTROL_HOVER, 3.0);
        }
        let color = if on { Color32::WHITE } else { t::TEXT_DIM };
        let label_width = text_width(p, labels[i], t::HEADER);
        if tr.width() >= label_width + 34.0 {
            let x = tr.center().x - (label_width + 20.0) / 2.0;
            icon(
                p,
                Rect::from_min_size(pos2(x, tr.top()), vec2(16.0, tr.height())),
                icon_names[i],
                color,
                15.0,
            );
            text(
                p,
                Rect::from_min_size(
                    pos2(x + 20.0, tr.top()),
                    vec2(label_width + 2.0, tr.height()),
                ),
                labels[i],
                t::HEADER.with_color(color),
                Align::Left,
            );
        } else if tr.width() >= 40.0 {
            let shown = fit(p, labels[i], tr.width() - 6.0, t::HEADER);
            text(p, tr, &shown, t::HEADER.with_color(color), Align::Center);
        } else {
            icon(p, tr, icon_names[i], color, 15.0);
        }
        let label = labels[i];
        response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, on, label));
        let _ = with_tooltip(response, Some(label));
    }
    result
}

/// 折りたたまない見出し（パネルの名前）。
pub fn panel_title(p: &Painter, r: Rect, title: &str, icon_name: Option<&str>) {
    fill(p, r, t::PANEL_HEADER);
    hline(p, r.left(), r.right(), r.bottom() - 1.0, t::BORDER);
    let mut x = r.left() + 8.0;
    if let Some(name) = icon_name {
        icon(
            p,
            Rect::from_min_size(pos2(x, r.top()), vec2(16.0, r.height())),
            name,
            t::TEXT_DIM,
            15.0,
        );
        x += 20.0;
    }
    text(
        p,
        Rect::from_min_max(pos2(x, r.top()), r.max),
        title,
        t::HEADER,
        Align::Left,
    );
}

// ───────── スライダー ─────────

/// 2 行の形の 1 行目（名前と値の箱）の高さ。
pub const TWO_LINE_LABEL_HEIGHT: f32 = 15.0;
/// この高さ以上の矩形は 2 行の形で描く。
pub const TWO_LINE_MIN_HEIGHT: f32 = 30.0;

/// スライダーの値の書き方。
#[derive(Clone, Copy, Debug)]
pub struct NumberFormat<'a> {
    pub decimals: u8,
    /// 小数の後ろの 0 を落とす（Unity の "0.##"）。
    pub trim: bool,
    pub suffix: &'a str,
}

impl<'a> NumberFormat<'a> {
    pub const fn int(suffix: &'a str) -> Self {
        NumberFormat {
            decimals: 0,
            trim: false,
            suffix,
        }
    }
    pub fn format(&self, v: f32) -> String {
        let scale = 10f32.powi(self.decimals as i32);
        let rounded = (v * scale).round() / scale; // 0.5 は 0 から離れる向き（C# の ToString と同じ）
        let mut s = format!("{:.*}", self.decimals as usize, rounded);
        if self.trim && s.contains('.') {
            s = s.trim_end_matches('0').trim_end_matches('.').to_owned();
        }
        if s == "-0" {
            s = "0".into();
        }
        s + self.suffix
    }
}

pub struct SliderSpec<'a> {
    pub label: &'a str,
    pub min: f32,
    pub max: f32,
    pub format: NumberFormat<'a>,
    pub tooltip: Option<&'a str>,
    pub enabled: bool,
    /// 2 行目の右に空ける幅（筆圧のペンのボタン）。
    pub track_inset: f32,
}

impl<'a> SliderSpec<'a> {
    pub fn new(label: &'a str, min: f32, max: f32, format: NumberFormat<'a>) -> Self {
        SliderSpec {
            label,
            min,
            max,
            format,
            tooltip: None,
            enabled: true,
            track_inset: 0.0,
        }
    }
    pub fn tooltip(mut self, tip: &'a str) -> Self {
        self.tooltip = Some(tip);
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn inset(mut self, inset: f32) -> Self {
        self.track_inset = inset;
        self
    }
}

pub struct SliderOutcome {
    pub value: f32,
    pub changed: bool,
    /// ドラッグ（押している）の最中。
    pub active: bool,
    /// 押していたのを離した（打った値を決めたときも）。
    pub released: bool,
}

/// 2 行の形の値の箱（値の幅は min・max・今の値の広いほう。動かしても箱が揺れない）。
pub fn two_line_value_box(r: Rect, value_width: f32) -> Rect {
    let w = r.width().min((value_width + 12.0).max(34.0));
    Rect::from_min_size(
        pos2(r.right() - w, r.top()),
        vec2(w, TWO_LINE_LABEL_HEIGHT + 1.0),
    )
}

/// スライダー。押した位置の値になり、ドラッグで動かす。高さで形が決まる: 1 行（値が中に出る。ダブルクリックで数値を打つ）と、
/// 2 行（1 行目に名前と値の箱、2 行目に細い溝とつまみ。値の箱を押すと数値を打つ）。Enter で決め、Esc か外を押すとやめる。
pub fn slider(
    ui: &mut Ui,
    r: Rect,
    id_salt: impl egui::AsIdSalt,
    value: f32,
    spec: &SliderSpec,
) -> SliderOutcome {
    let id = ui.make_persistent_id(id_salt);
    let enabled = spec.enabled && ui.is_enabled();
    let two = r.height() >= TWO_LINE_MIN_HEIGHT;
    let shown = spec.format.format(value);
    let box_rect = if two {
        let p = ui.painter();
        let widest = text_width(p, &spec.format.format(spec.min), t::VALUE)
            .max(text_width(p, &spec.format.format(spec.max), t::VALUE))
            .max(text_width(p, &shown, t::VALUE));
        two_line_value_box(r, widest)
    } else {
        r
    };
    let track = if two {
        Rect::from_min_size(
            pos2(r.left(), r.top() + TWO_LINE_LABEL_HEIGHT),
            vec2(
                (r.width() - spec.track_inset).max(1.0),
                r.height() - TWO_LINE_LABEL_HEIGHT,
            ),
        )
    } else {
        r
    };
    let range = spec.max - spec.min;
    let fraction = |v: f32| {
        if range > 0.0 {
            ((v - spec.min) / range).clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let from = if spec.min < 0.0 && spec.max > 0.0 {
        fraction(0.0)
    } else {
        0.0
    };
    let edit_id = id.with("edit");
    let editing: Option<String> = ui.data(|d| d.get_temp(edit_id));
    let mut out = SliderOutcome {
        value,
        changed: false,
        active: false,
        released: false,
    };

    if let Some(mut buffer) = editing {
        let field = if two {
            if ui.is_rect_visible(r) {
                draw_two_line(
                    ui.painter(),
                    r,
                    track,
                    box_rect,
                    spec.label,
                    "",
                    from,
                    fraction(value),
                    enabled,
                    false,
                    false,
                );
            }
            let w = r.width().min(box_rect.width().max(90.0));
            Rect::from_min_size(
                pos2(r.right() - w, r.top()),
                vec2(w, TWO_LINE_LABEL_HEIGHT + 3.0),
            )
        } else {
            r
        };
        {
            let p = ui.painter();
            rounded(p, field, t::CONTROL_BG, 3.0);
            outline(p, field, t::ACCENT, 1.0, 3.0);
        }
        let te_id = id.with("field");
        let inner = field.shrink2(vec2(4.0, 1.0));
        let response = ui.put(
            inner,
            egui::TextEdit::singleline(&mut buffer)
                .id(te_id)
                .frame(egui::Frame::NONE)
                .font(t::LABEL.font())
                .text_color(t::TEXT)
                .margin(egui::Margin::ZERO)
                .vertical_align(egui::Align::Center),
        );
        let first: bool = ui.data(|d| d.get_temp::<bool>(edit_id.with("focus")).unwrap_or(false));
        if first {
            response.request_focus();
            ui.data_mut(|d| d.insert_temp(edit_id.with("focus"), false));
        }
        let (enter, escape) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if response.lost_focus() || (!first && !response.has_focus()) {
            if enter && !escape {
                if let Ok(typed) = buffer
                    .trim()
                    .trim_end_matches(spec.format.suffix.trim())
                    .trim()
                    .parse::<f32>()
                {
                    out.value = typed.clamp(spec.min, spec.max);
                    out.changed = out.value != value;
                    out.released = true;
                }
            }
            ui.data_mut(|d| d.remove::<String>(edit_id));
        } else {
            ui.data_mut(|d| d.insert_temp(edit_id, buffer));
        }
        return out;
    }

    // 当たりは行の全体（1 行目の右端の値の箱まで押せる）。2 行目の右の空けた所（ペンのボタン）の押下は値にしない
    // （ボタンは後から置くので、押下はそちらが受ける）
    let response = interact(ui, r, id, enabled, Sense::click_and_drag());
    let hover = enabled && response.hovered();
    // 押しているあいだは押し始めの位置、離したフレーム（クリック）は応答の位置（離すと press_origin は空になる）
    let press_origin = ui
        .input(|i| i.pointer.press_origin())
        .or(response.interact_pointer_pos());
    let on_box = two && press_origin.is_some_and(|o| box_rect.contains(o));
    let in_inset = two && press_origin.is_some_and(|o| o.x > track.right() && o.y > track.top());
    let start_edit =
        enabled && ((two && on_box && response.clicked()) || (!two && response.double_clicked()));
    if start_edit {
        ui.data_mut(|d| {
            d.insert_temp(
                edit_id,
                spec.format
                    .format(value)
                    .trim_end_matches(spec.format.suffix)
                    .to_owned(),
            );
            d.insert_temp(edit_id.with("focus"), true);
        });
        ui.ctx().request_repaint();
    } else if enabled && response.is_pointer_button_down_on() && !on_box && !in_inset {
        if let Some(pointer) = response.interact_pointer_pos() {
            let next = spec.min
                + range * ((pointer.x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0);
            if next != value {
                out.value = next;
                out.changed = true;
            }
        }
        out.active = true;
    }
    if enabled && ((response.drag_stopped() || response.clicked()) && !on_box && !in_inset) {
        out.released = true;
    }
    let active = out.active;
    if ui.is_rect_visible(r) {
        let p = ui.painter();
        let shown = spec.format.format(out.value);
        if two {
            draw_two_line(
                p,
                r,
                track,
                box_rect,
                spec.label,
                &shown,
                from,
                fraction(out.value),
                enabled,
                hover,
                active,
            );
        } else {
            let tv = fraction(out.value);
            let color = if enabled { t::TEXT } else { t::TEXT_DISABLED };
            rounded(p, r, t::CONTROL_BG, 3.0);
            let fill_rect = Rect::from_min_size(
                pos2(r.left() + r.width() * from.min(tv), r.top()),
                vec2((r.width() * (tv - from).abs()).max(0.0), r.height()),
            );
            if fill_rect.width() > 0.0 {
                rounded(
                    p,
                    fill_rect,
                    if !enabled {
                        t::CONTROL_HOVER
                    } else if active || hover {
                        t::SLIDER_FILL_HOVER
                    } else {
                        t::SLIDER_FILL
                    },
                    3.0,
                );
            }
            outline(
                p,
                r,
                if hover || active {
                    t::ACCENT_DIM
                } else {
                    t::BORDER
                },
                1.0,
                3.0,
            );
            let inner = r.shrink2(vec2(7.0, 0.0));
            text(
                p,
                inner,
                spec.label,
                t::LABEL.with_color(color),
                Align::Left,
            );
            text(p, inner, &shown, t::VALUE.with_color(color), Align::Right);
        }
    }
    let label = spec.label;
    let current = out.value as f64;
    response.widget_info(|| WidgetInfo::slider(enabled, current, label));
    let _ = with_tooltip(response, spec.tooltip);
    out
}

#[allow(clippy::too_many_arguments)]
fn draw_two_line(
    p: &Painter,
    r: Rect,
    track: Rect,
    box_rect: Rect,
    label: &str,
    value: &str,
    from: f32,
    tv: f32,
    enabled: bool,
    hover: bool,
    active: bool,
) {
    let color = if enabled { t::TEXT } else { t::TEXT_DISABLED };
    if !label.is_empty() {
        text(
            p,
            Rect::from_min_size(
                pos2(r.left() + 1.0, r.top()),
                vec2(
                    (box_rect.left() - r.left() - 5.0).max(0.0),
                    TWO_LINE_LABEL_HEIGHT,
                ),
            ),
            label,
            t::LABEL.with_color(color),
            Align::Left,
        );
    }
    rounded(p, box_rect, t::CONTROL_BG, 3.0);
    outline(
        p,
        box_rect,
        if hover || active {
            t::ACCENT_DIM
        } else {
            t::BORDER
        },
        1.0,
        3.0,
    );
    text(
        p,
        box_rect.shrink2(vec2(4.0, 0.0)),
        value,
        t::VALUE.with_color(color),
        Align::Right,
    );
    let cy = (track.center().y + 1.0).round();
    let groove = Rect::from_min_size(
        pos2(track.left() + 1.0, cy - 2.0),
        vec2(track.width() - 2.0, 4.0),
    );
    rounded(p, groove, t::CONTROL_BG, 2.0);
    outline(p, groove, t::BORDER, 1.0, 2.0);
    let fill_rect = Rect::from_min_size(
        pos2(groove.left() + groove.width() * from.min(tv), groove.top()),
        vec2(
            (groove.width() * (tv - from).abs()).max(0.0),
            groove.height(),
        ),
    );
    if fill_rect.width() > 0.0 {
        rounded(
            p,
            fill_rect,
            if !enabled {
                t::CONTROL_HOVER
            } else if active || hover {
                t::SLIDER_FILL_HOVER
            } else {
                t::ACCENT
            },
            2.0,
        );
    }
    const KNOB: f32 = 10.0;
    // 溝が狭くても壊れないように（Unity の Mathf.Clamp と同じく、下限が上限を超えたら上限）
    let kx = (track.left() + track.width() * tv)
        .max(track.left() + KNOB / 2.0)
        .min(track.right() - KNOB / 2.0);
    let k = Rect::from_center_size(pos2(kx, cy), vec2(KNOB, KNOB));
    rounded(
        p,
        k,
        if !enabled {
            t::TEXT_DISABLED
        } else if active || hover {
            Color32::WHITE
        } else {
            t::TEXT
        },
        KNOB / 2.0,
    );
    outline(p, k, t::BORDER, 1.0, KNOB / 2.0);
}

// ───────── チェック・ドロップダウン・入力欄・色 ─────────

/// チェック（押すと反転した値を返す）。
pub fn toggle(
    ui: &mut Ui,
    r: Rect,
    id_salt: impl egui::AsIdSalt,
    label: &str,
    value: bool,
    tooltip: Option<&str>,
    enabled: bool,
) -> bool {
    let id = ui.make_persistent_id(id_salt);
    let response = interact(ui, r, id, enabled, Sense::click());
    let next = if response.clicked() { !value } else { value };
    let hover = enabled && response.hovered();
    let p = ui.painter();
    let b = Rect::from_min_size(pos2(r.left(), r.center().y - 8.0), vec2(16.0, 16.0));
    rounded(
        p,
        b,
        if next {
            if enabled {
                t::ACCENT
            } else {
                t::CONTROL_HOVER
            }
        } else {
            t::CONTROL_BG
        },
        3.0,
    );
    if next {
        icon(p, b, "check", Color32::WHITE, 14.0);
    } else {
        outline(
            p,
            b,
            if hover { t::ACCENT_DIM } else { t::SEPARATOR },
            1.0,
            3.0,
        );
    }
    text(
        p,
        Rect::from_min_max(pos2(b.right() + 7.0, r.top()), r.max),
        label,
        t::LABEL.with_color(if enabled { t::TEXT } else { t::TEXT_DISABLED }),
        Align::Left,
    );
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, next, label));
    let _ = with_tooltip(response, tooltip);
    next
}

/// 選んでいる値を出す箱（押すと呼ぶ側が自前のメニューを箱の下に開く）。label があれば左に。返すのは箱の応答と箱の矩形。
#[allow(clippy::too_many_arguments)]
pub fn dropdown(
    ui: &mut Ui,
    r: Rect,
    id_salt: impl egui::AsIdSalt,
    label: Option<&str>,
    value: &str,
    tooltip: Option<&str>,
    enabled: bool,
    label_width: f32,
) -> (Response, Rect) {
    let mut b = r;
    if let Some(label) = label.filter(|l| !l.is_empty()) {
        let p = ui.painter();
        let lw = if label_width > 0.0 {
            label_width
        } else {
            (r.width() * 0.42).min(text_width(p, label, t::LABEL) + 10.0)
        };
        text(
            p,
            Rect::from_min_size(r.min, vec2(lw, r.height())),
            label,
            t::LABEL.with_color(if enabled { t::TEXT } else { t::TEXT_DISABLED }),
            Align::Left,
        );
        b = Rect::from_min_max(pos2(r.left() + lw, r.top()), r.max);
    }
    let id = ui.make_persistent_id(id_salt);
    let response = interact(ui, b, id, enabled, Sense::click());
    let hover = enabled && response.hovered();
    let p = ui.painter();
    rounded(
        p,
        b,
        if enabled && response.is_pointer_button_down_on() {
            t::CONTROL_ACTIVE
        } else if hover {
            t::CONTROL_HOVER
        } else {
            t::CONTROL_BG
        },
        3.0,
    );
    outline(p, b, t::BORDER, 1.0, 3.0);
    let value_rect = Rect::from_min_size(
        pos2(b.left() + 7.0, b.top()),
        vec2((b.width() - 26.0).max(0.0), b.height()),
    );
    let shown = fit(p, value, value_rect.width(), t::LABEL);
    text(
        p,
        value_rect,
        &shown,
        t::LABEL.with_color(if enabled { t::TEXT } else { t::TEXT_DISABLED }),
        Align::Left,
    );
    icon(
        p,
        Rect::from_min_size(pos2(b.right() - 20.0, b.top()), vec2(18.0, b.height())),
        "arrow_drop_down",
        t::TEXT_DIM,
        18.0,
    );
    let name = label
        .map(|l| format!("{l}: {value}"))
        .unwrap_or_else(|| value.to_owned());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, enabled, &name));
    (with_tooltip(response, tooltip), b)
}

/// 文字の入力欄の結果。
pub struct TextFieldOutcome {
    /// 決めた文字（Enter か、ほかを押してフォーカスが外れたとき。Esc はやめる）。
    pub committed: Option<String>,
    pub focused: bool,
}

/// 文字の入力欄。打っている間は値を返さず、Enter か、外を押したときに決める（Esc でやめる）。focus なら初めにフォーカスを取る。
pub fn text_field(
    ui: &mut Ui,
    r: Rect,
    id_salt: impl egui::AsIdSalt,
    current: &str,
    tooltip: Option<&str>,
    focus: bool,
) -> TextFieldOutcome {
    let id = ui.make_persistent_id(id_salt);
    let te_id = id.with("te");
    let buf_id = id.with("buf");
    let had_focus = ui.memory(|m| m.has_focus(te_id));
    let mut buffer: String = if had_focus {
        ui.data(|d| d.get_temp(buf_id))
            .unwrap_or_else(|| current.to_owned())
    } else {
        current.to_owned()
    };
    let hover = ui.rect_contains_pointer(r);
    {
        let p = ui.painter();
        rounded(p, r, t::CONTROL_BG, 3.0);
        outline(
            p,
            r,
            if had_focus {
                t::ACCENT
            } else if hover {
                t::ACCENT_DIM
            } else {
                t::BORDER
            },
            1.0,
            3.0,
        );
    }
    let inner = r.shrink2(vec2(6.0, 1.0));
    let response = ui.put(
        inner,
        egui::TextEdit::singleline(&mut buffer)
            .id(te_id)
            .frame(egui::Frame::NONE)
            .font(t::LABEL.font())
            .text_color(t::TEXT)
            .margin(egui::Margin::ZERO)
            .vertical_align(egui::Align::Center),
    );
    if focus && !had_focus {
        response.request_focus();
    }
    let mut out = TextFieldOutcome {
        committed: None,
        focused: response.has_focus(),
    };
    if response.lost_focus() {
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if !escape && buffer != current {
            out.committed = Some(buffer);
        }
        ui.data_mut(|d| d.remove::<String>(buf_id));
    } else if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(buf_id, buffer));
    }
    let _ = with_tooltip(response, tooltip);
    out
}

/// 色の見本（straight の RGBA 0〜1）。左半分は不透明、右半分は市松の上にアルファで。
pub fn color_swatch(
    ui: &mut Ui,
    r: Rect,
    id_salt: impl egui::AsIdSalt,
    color: [f32; 4],
    tooltip: &str,
    enabled: bool,
) -> Response {
    let id = ui.make_persistent_id(id_salt);
    let response = interact(ui, r, id, enabled, Sense::click());
    let hover = enabled && response.hovered();
    let p = ui.painter();
    let c = |a: f32| {
        Color32::from_rgba_unmultiplied(
            to_byte(color[0]),
            to_byte(color[1]),
            to_byte(color[2]),
            to_byte(a),
        )
    };
    rounded(p, r, Color32::WHITE, 3.0);
    let gray = Color32::from_gray(191);
    fill(
        p,
        Rect::from_min_size(
            pos2(r.left() + r.width() / 2.0, r.top()),
            vec2(r.width() / 4.0, r.height() / 2.0),
        ),
        gray,
    );
    fill(
        p,
        Rect::from_min_size(
            pos2(r.left() + r.width() * 0.75, r.top() + r.height() / 2.0),
            vec2(r.width() / 4.0, r.height() / 2.0),
        ),
        gray,
    );
    rounded(
        p,
        Rect::from_min_size(r.min, vec2(r.width() / 2.0, r.height())),
        c(1.0),
        3.0,
    );
    fill(
        p,
        Rect::from_min_size(
            pos2(r.left() + r.width() / 2.0, r.top()),
            vec2(r.width() / 2.0, r.height()),
        ),
        c(color[3]),
    );
    outline(p, r, if hover { t::ACCENT } else { t::BORDER }, 1.0, 3.0);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::ColorButton, enabled, tooltip));
    with_tooltip(response, Some(tooltip))
}

pub fn to_byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// 縦に行を積むだけの配置の補助（Unity 版の `UiRows`）。
pub struct Rows {
    area: Rect,
    y: f32,
    pub indent: f32,
}

impl Rows {
    pub fn new(area: Rect, top: f32) -> Rows {
        Rows {
            area,
            y: area.top() + top,
            indent: 0.0,
        }
    }
    pub fn width(&self) -> f32 {
        self.area.width() - 2.0 * t::PADDING - self.indent
    }
    pub fn used(&self) -> f32 {
        self.y - self.area.top()
    }
    pub fn y(&self) -> f32 {
        self.y
    }
    pub fn row(&mut self, height: f32, gap: f32) -> Rect {
        let r = Rect::from_min_size(
            pos2(self.area.left() + t::PADDING + self.indent, self.y),
            vec2(self.width(), height),
        );
        self.y += height + gap;
        r
    }
    pub fn full_row(&mut self, height: f32, gap: f32) -> Rect {
        let r = Rect::from_min_size(
            pos2(self.area.left(), self.y),
            vec2(self.area.width(), height),
        );
        self.y += height + gap;
        r
    }
    pub fn slider_row(&mut self) -> Rect {
        self.row(t::SLIDER_ROW_HEIGHT, 4.0)
    }
    pub fn space(&mut self, h: f32) {
        self.y += h;
    }
    /// 行を幅で n 等分する（間は gap）。
    pub fn split(r: Rect, n: usize, gap: f32) -> Vec<Rect> {
        let w = (r.width() - gap * (n as f32 - 1.0)) / n as f32;
        (0..n)
            .map(|i| {
                Rect::from_min_size(
                    pos2(r.left() + i as f32 * (w + gap), r.top()),
                    vec2(w, r.height()),
                )
            })
            .collect()
    }
}

/// テキストの配置（左上・中央）の別名。
pub fn align2(a: Align) -> Align2 {
    match a {
        Align::Left => Align2::LEFT_CENTER,
        Align::Center => Align2::CENTER_CENTER,
        Align::Right => Align2::RIGHT_CENTER,
    }
}
