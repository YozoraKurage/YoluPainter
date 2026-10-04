//! プロパティの欄（Substance Painter の並び）: 選んでいる物の中身だけを出す。描く文脈（ペイントのレイヤーか、どのレイヤーでもマスク）は
//! 頭にタブ（アルファ｜ステンシル｜マテリアル（マスクに描くあいだはマスク）｜レイヤー）、塗りつぶし・調整・グループの文脈は
//! レイヤーの欄だけ。中身は縦に積み、はみ出したらスクロールする（アルファの欄は `brush_props`、レイヤーの欄は `layer_props`）。
//! ブラシそのもの（一覧・ツールプロパティ・詳細）は左のドックのブラシのパネル（`brushes`）と詳細の窓（`brush_detail`）。
//! 値の操作はブラシの設定なら画面の状態を直に、レイヤーの設定は `Action::M2` を通す（1 回の Undo）。画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Ui};

use crate::engine::LayerKind;
use crate::m2_menu::Popup;
use crate::state::{AppState, OpenPopup, PopupKind};
use crate::ui::menu::PopupState;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};

pub const TAB_ICONS: [&str; 4] = ["shapes", "square", "layers", "tune"];

/// 欄の文脈。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// ブラシ・消しゴムで描く（ペイントのレイヤーか、マスク）。
    Paint,
    /// 塗りつぶし・調整・グループの中身（これらには描けない）。
    Layer,
    /// 範囲の道具（バケツ・ポリゴン塗りつぶし・ID の色で選択）。ブラシのタブは出さず、その道具の欄だけ。
    Tool,
    /// 選択の道具（選択範囲を変更。どの層を選んでいても）。
    Selection,
    /// 移動・変形の道具（変形の数値・補間と、選んでいる層のロック。どの層を選んでいても）。
    Transform,
}

/// 今の文脈（マスクを選んでいればどの層でも描く文脈）。
pub fn context(app: &AppState) -> Context {
    if app.tool.is_select() {
        return Context::Selection;
    }
    if app.tool == crate::state::Tool::Move {
        return Context::Transform;
    }
    let kind = app
        .selected_layer
        .and_then(|id| app.doc.layer(id))
        .map(|l| l.kind());
    let base = match kind {
        Some(LayerKind::Raster) | None => Context::Paint,
        Some(_) if app.m2.edit_mask => Context::Paint,
        Some(_) => Context::Layer,
    };
    // 範囲の道具は、その道具の欄（ID の色で選択はどの層でも。バケツとポリゴン塗りつぶしは、塗れる層のとき）
    if super::region_props::owns_properties(app, base == Context::Paint) {
        Context::Tool
    } else {
        base
    }
}

/// タブの名前（3 つ目はマスクに描くあいだだけマスク）。
pub fn tab_labels(app: &AppState) -> [&'static str; 4] {
    let l = app.lang;
    [
        l.pick("アルファ", "Alpha"),
        l.pick("ステンシル", "Stencil"),
        if app.m2.edit_mask {
            l.pick("マスク", "Mask")
        } else {
            l.pick("マテリアル", "Material")
        },
        l.pick("レイヤー", "Layer"),
    ]
}

pub fn open_popup(
    app: &mut AppState,
    ctx: &egui::Context,
    popup: Popup,
    anchor: Rect,
    min_width: f32,
) {
    app.popup = Some(OpenPopup {
        kind: PopupKind::M2(popup),
        state: PopupState::new(ctx, anchor).with_min_width(min_width),
    });
}

/// 筆圧に従わせられるスライダー（2 行目の右にペンのボタン）。
#[allow(clippy::too_many_arguments)]
pub fn pen_slider(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    spec: SliderSpec,
    shown: f32,
    pressure: &mut bool,
    pen_tooltip: &str,
) -> Option<f32> {
    const PEN: f32 = 24.0;
    let row = rows.slider_row();
    let out = w::slider(ui, row, id, shown, &spec.inset(PEN + 4.0));
    let button = Rect::from_min_size(
        pos2(row.right() - PEN, row.bottom() - 20.0),
        vec2(PEN, 20.0),
    );
    if w::icon_button(
        ui,
        button,
        (id, "pen"),
        "stylus",
        pen_tooltip,
        *pressure,
        true,
        15.0,
    )
    .clicked()
    {
        *pressure = !*pressure;
    }
    out.changed.then_some(out.value)
}

/// 大見出し（開閉を覚える）。返すのは (開いているか, 既定に戻す頼み)。
pub fn section(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    key: &'static str,
    title: &str,
    icon: &str,
    reset: Option<&str>,
) -> (bool, bool) {
    let open = app.section_open(key, true);
    rows.indent = 0.0;
    let header = rows.full_row(t::PANEL_HEADER_HEIGHT, 5.0);
    let out = w::section_header(ui, header, ("section", key), title, open, Some(icon), reset);
    if out.open != open {
        app.sections.insert(key, out.open);
    }
    if out.open {
        rows.indent = t::SECTION_INDENT;
    }
    (out.open, out.reset)
}

/// 大見出しの中の小見出し（初めは閉じている）。reset を渡すと右端に「既定に戻す」。返すのは (開いているか, 既定に戻す頼み)。
pub fn subsection(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    key: &'static str,
    title: &str,
    reset: Option<&str>,
) -> (bool, bool) {
    let open = app.section_open(key, false);
    rows.indent = t::SECTION_INDENT;
    let header = rows.row(20.0, 3.0);
    let next = w::subsection_header(ui, header, ("subsection", key), title, open);
    let mut reset_clicked = false;
    if let Some(tip) = reset {
        let b = Rect::from_min_size(
            pos2(header.right() - 22.0, header.top() - 1.0),
            vec2(22.0, header.height() + 2.0),
        );
        reset_clicked = w::icon_button(
            ui,
            b,
            ("subsection.reset", key),
            "restart_alt",
            tip,
            false,
            true,
            13.0,
        )
        .clicked();
    }
    if next != open {
        app.sections.insert(key, next);
    }
    rows.indent = t::SECTION_INDENT + 10.0;
    (next, reset_clicked)
}

/// スライダーの 1 行（2 行の形）。値が替わったら新しい値を返す。
#[allow(clippy::too_many_arguments)]
pub fn slider_row(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    label: &str,
    value: f32,
    range: (f32, f32),
    format: NumberFormat,
    tooltip: Option<&str>,
    enabled: bool,
) -> Option<f32> {
    let mut spec = SliderSpec::new(label, range.0, range.1, format).enabled(enabled);
    if let Some(tip) = tooltip {
        spec = spec.tooltip(tip);
    }
    let out = w::slider(ui, rows.slider_row(), id, value, &spec);
    out.changed.then_some(out.value)
}

/// 0〜1 の値を % で出すスライダー（値は 0〜1 のまま受け渡す。range も 0〜1 の側）。
#[allow(clippy::too_many_arguments)]
pub fn percent_row(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    label: &str,
    value: f64,
    range: (f64, f64),
    tooltip: Option<&str>,
    enabled: bool,
) -> Option<f64> {
    slider_row(
        ui,
        rows,
        id,
        label,
        (value * 100.0) as f32,
        ((range.0 * 100.0) as f32, (range.1 * 100.0) as f32),
        NumberFormat::int("%"),
        tooltip,
        enabled,
    )
    .map(|v| (v as f64 / 100.0).clamp(range.0, range.1))
}

/// チェックの 1 行。
pub fn toggle_row(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    label: &str,
    value: bool,
    tooltip: Option<&str>,
    enabled: bool,
) -> Option<bool> {
    let r = rows.row(t::ROW_HEIGHT, 2.0);
    let next = w::toggle(ui, r, id, label, value, tooltip, enabled);
    (next != value).then_some(next)
}

/// 名前と値の箱（押すとポップアップ）の 1 行。押されたら箱の矩形を返す。
pub fn choice_row(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    label: &str,
    value: &str,
    tooltip: Option<&str>,
    enabled: bool,
) -> Option<Rect> {
    let r = rows.row(t::ROW_HEIGHT, 4.0);
    let (response, b) = w::dropdown(ui, r, id, Some(label), value, tooltip, enabled, 92.0);
    response.clicked().then_some(b)
}

/// 小さな見出しの 1 行（まとまりの名前）。
pub fn group_label(ui: &mut Ui, rows: &mut Rows, text: &str) {
    let r = rows.row(16.0, 1.0);
    w::text(
        ui.painter(),
        r,
        text,
        t::LABEL_BOLD.with_color(t::TEXT_DIM),
        w::Align::Left,
    );
}

/// 短い状態の行（名前だけ。説明の文は置かない）。
pub fn status_row(ui: &mut Ui, rows: &mut Rows, text: &str) {
    let r = rows.row(t::ROW_HEIGHT, 2.0);
    let shown = w::fit(ui.painter(), text, r.width(), t::LABEL_DIM);
    w::text(ui.painter(), r, &shown, t::LABEL_DIM, w::Align::Left);
}

pub fn show(ui: &mut Ui, app: &mut AppState) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let ctx = ui.ctx().clone();
    let context = context(app);
    let mut top = r.top();
    let mut tab = app.property_tab.min(TAB_ICONS.len() - 1);
    if context == Context::Paint {
        let strip = Rect::from_min_size(r.min, vec2(r.width(), t::PROPERTY_TAB_STRIP_HEIGHT));
        let labels = tab_labels(app);
        let chosen = w::tab_strip(ui, strip, "props.tabs", &labels, &TAB_ICONS, tab);
        if chosen != tab {
            app.m2.props_scroll = 0.0;
        }
        tab = chosen;
        app.property_tab = tab;
        top = strip.bottom();
    }
    let body = Rect::from_min_max(pos2(r.left(), top), r.max);

    // スクロール（中身の高さは前のフレームのもの。はみ出していれば右端に細い帯）
    let max_scroll = (app.m2.props_content - body.height()).max(0.0);
    if ui.rect_contains_pointer(body) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        app.m2.props_scroll -= wheel;
    }
    app.m2.props_scroll = app.m2.props_scroll.clamp(0.0, max_scroll);
    let scroll = app.m2.props_scroll;
    let bar = if max_scroll > 0.0 { 8.0 } else { 0.0 };
    let area = Rect::from_min_max(
        pos2(body.left(), body.top() - scroll),
        pos2(body.right() - bar, body.bottom()),
    );
    let outer_clip = ui.clip_rect();
    ui.set_clip_rect(body.intersect(outer_clip));
    let mut rows = Rows::new(area, 0.0);
    match context {
        Context::Layer => super::layer_props::layer_body(ui, app, &mut rows, &ctx),
        Context::Tool => super::region_props::body(ui, app, &mut rows, &ctx),
        Context::Selection => crate::selection::props::selection_body(ui, app, &mut rows),
        Context::Transform => crate::transform::props::body(ui, app, &mut rows, &ctx),
        Context::Paint => match tab {
            0 => super::brush_props::alpha_tab(ui, app, &mut rows, &ctx),
            1 => super::brush_props::stencil_tab(ui, app, &mut rows),
            2 if app.m2.edit_mask => super::layer_props::mask_tab(ui, app, &mut rows),
            2 => super::brush_props::material_tab(ui, app, &mut rows),
            _ => super::layer_props::layer_body(ui, app, &mut rows, &ctx),
        },
    }
    rows.indent = 0.0;
    rows.space(8.0);
    app.m2.props_content = rows.used();
    ui.set_clip_rect(outer_clip);
    // スライダーのドラッグを離したら、まとめていた変更を 1 回の Undo にする
    if !ui.input(|i| i.pointer.primary_down()) {
        app.m2_end_drag();
    }
    if max_scroll > 0.0 {
        let track = body.height();
        let bar_h = (track * track / app.m2.props_content).max(16.0);
        let bar_y = body.top() + (track - bar_h) * scroll / max_scroll;
        w::rounded(
            ui.painter(),
            Rect::from_min_size(pos2(body.right() - 6.0, bar_y), vec2(4.0, bar_h)),
            t::CONTROL_ACTIVE,
            2.0,
        );
    }
}
