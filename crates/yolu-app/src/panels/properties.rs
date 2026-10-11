//! プロパティの欄（Substance Painter の並び）: 選んでいるレイヤーの中身だけを出す。ツールの設定は出さない（ツールの設定は左のドックの
//! ツールプロパティのパネル `tool_props`。どのツールを選んでいても、この欄は同じレイヤーには同じ中身）。描く文脈（ペイントのレイヤーか、どのレイヤーでもマスク）は
//! 頭にタブ（ステンシル｜レイヤー）、塗りつぶし・調整・グループの文脈はレイヤーの欄だけ、レイヤーの下の効果の行を選んでいるときは
//! その行の設定だけ。中身は縦に積み、はみ出したらスクロールする（ステンシルの欄は `brush_props` 経由、レイヤーの欄は `layer_props`）。
//! テクスチャセットの見た目は別のパネル「マテリアル」（`material`）、ブラシそのもの（一覧・ツールプロパティ・ブラシサイズ・詳細）は左のドックの
//! サブツール・ツールプロパティ・ブラシサイズのパネル（`subtools`・`tool_props`・`brushes`）と詳細のウィンドウ（`brush_detail`）。
//! 値の操作はブラシの設定なら画面の状態を直に、レイヤーの設定は `Action::M2` を通す（1 回の Undo）。画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Ui};

use crate::engine::LayerKind;
use crate::m2_menu::Popup;
use crate::state::{AppState, OpenPopup, PopupKind};
use crate::ui::menu::PopupState;
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};

pub const TAB_ICONS: [&str; 2] = ["square", "tune"];

/// 欄の文脈（選んでいるレイヤーで決まる。ツールでは決まらない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// 描けるレイヤー（ペイントのレイヤーか、マスク）。
    Paint,
    /// 塗りつぶし・調整・グループの中身（これらには描けない）。
    Layer,
    /// レイヤーの下の効果の行（フィルター・Generator・アンカー）を選んでいる。タブは出さず、その行の設定だけ。
    Effect,
}

/// 今の文脈（マスクを選んでいればどのレイヤーでも描く文脈）。
pub fn context(app: &AppState) -> Context {
    if crate::fx::props_visible(app) {
        return Context::Effect;
    }
    let kind = app
        .selected_layer
        .and_then(|id| app.doc.layer(id))
        .map(|l| l.kind());
    match kind {
        Some(LayerKind::Raster) | None => Context::Paint,
        Some(_) if app.m2.edit_mask => Context::Paint,
        Some(_) => Context::Layer,
    }
}

/// タブの名前。
pub fn tab_labels(app: &AppState) -> [&'static str; 2] {
    let l = app.lang;
    [l.pick("ステンシル", "Stencil"), l.pick("レイヤー", "Layer")]
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

/// 大見出し（開閉を覚える。初めは開いている）。返すのは (開いているか, 既定に戻す頼み)。
pub fn section(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    key: &'static str,
    title: &str,
    icon: &str,
    reset: Option<&str>,
) -> (bool, bool) {
    section_default(ui, app, rows, key, title, icon, reset, true, false)
}

/// 大見出し（開閉を覚える。初めに開いているかは `default_open`。`marked` なら、見出しの右端に点の印を付ける: 閉じていても、中の機能が入っていると分かる）。
#[allow(clippy::too_many_arguments)]
pub fn section_default(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    key: &'static str,
    title: &str,
    icon: &str,
    reset: Option<&str>,
    default_open: bool,
    marked: bool,
) -> (bool, bool) {
    let open = app.section_open(key, default_open);
    rows.indent = 0.0;
    let header = rows.full_row(t::PANEL_HEADER_HEIGHT, 5.0);
    let out = w::section_header(ui, header, ("section", key), title, open, Some(icon), reset);
    if marked {
        let at = pos2(header.right() - 14.0, header.center().y);
        ui.painter().circle_filled(at, 3.0, t::ACCENT);
    }
    if out.open != open {
        app.ui.sections.insert(key, out.open);
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
        app.ui.sections.insert(key, next);
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
    let height = w::toggle_height(ui.painter(), rows.width(), label);
    let r = rows.row(height, 2.0);
    let next = w::toggle(ui, r, id, label, value, tooltip, enabled);
    (next != value).then_some(next)
}

/// バケツ・自動選択の「対称定規にスナップ」の 1 行。`unavailable` が理由（短い文）なら押せず、ツールチップに理由を出す。
pub fn snap_symmetry_row(
    ui: &mut Ui,
    rows: &mut Rows,
    lang: crate::lang::Lang,
    id: &str,
    value: bool,
    unavailable: Option<&str>,
) -> Option<bool> {
    toggle_row(
        ui,
        rows,
        id,
        lang.pick("対称定規にスナップ", "Snap to Symmetry Ruler"),
        value,
        unavailable,
        unavailable.is_none(),
    )
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
    // 狭い欄（左のドックのツールプロパティ）は、名前の幅に合わせる（値の箱を広く取る）
    let label_width = if rows.compact {
        (w::text_width(ui.painter(), label, t::LABEL) + 10.0).min(r.width() * 0.6)
    } else {
        92.0
    };
    let (response, b) = w::dropdown(ui, r, id, Some(label), value, tooltip, enabled, label_width);
    response.clicked().then_some(b)
}

/// 選び・切り替えのボタン 1 つ（`choice_buttons` に並べる）。
pub struct ChoiceButton<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub selected: bool,
    pub enabled: bool,
    pub tooltip: Option<&'a str>,
}

/// ボタンを欄の幅に並べる。全部が 1 行に収まらないとき（狭い欄の英語など）は、収まる最大の列数で折り返す（名前を切らない・「…」で詰めない）。
/// どの行も同じ幅のボタン。押されたボタンの番号を返す。
pub fn choice_buttons(ui: &mut Ui, rows: &mut Rows, items: &[ChoiceButton]) -> Option<usize> {
    const GAP: f32 = 4.0;
    if items.is_empty() {
        return None;
    }
    let needed = items
        .iter()
        .map(|b| w::text_width(ui.painter(), b.label, t::LABEL))
        .fold(0.0, f32::max)
        + 14.0;
    let width = rows.width();
    let cols = (1..=items.len())
        .rev()
        .find(|c| (width - GAP * (*c as f32 - 1.0)) / *c as f32 >= needed)
        .unwrap_or(1);
    let mut clicked = None;
    for (line_no, line) in items.chunks(cols).enumerate() {
        let row = rows.row(24.0, GAP);
        let cells = Rows::split(row, cols, GAP);
        for (i, (cell, b)) in cells.iter().zip(line).enumerate() {
            if w::button(
                ui,
                *cell,
                (b.id, "choice"),
                b.label,
                b.selected,
                b.enabled,
                b.tooltip,
                None,
            )
            .clicked()
            {
                clicked = Some(line_no * cols + i);
            }
        }
    }
    clicked
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
    let mut tab = app.ui.property_tab.min(TAB_ICONS.len() - 1);
    if context == Context::Paint {
        let strip = Rect::from_min_size(r.min, vec2(r.width(), t::PROPERTY_TAB_STRIP_HEIGHT));
        let labels = tab_labels(app);
        let chosen = w::tab_strip(ui, strip, "props.tabs", &labels, &TAB_ICONS, tab);
        if chosen != tab {
            app.m2.props_scroll = 0.0;
        }
        tab = chosen;
        app.ui.property_tab = tab;
        top = strip.bottom();
    }
    let body = Rect::from_min_max(pos2(r.left(), top), r.max);

    // スクロール（中身の高さは前のフレームのもの。はみ出していれば右端に細い帯）
    let bar = Scroll::begin(ui, body, app.m2.props_content, &mut app.m2.props_scroll);
    let scroll = app.m2.props_scroll;
    let area = Rect::from_min_max(
        pos2(body.left(), body.top() - scroll),
        pos2(body.right() - bar.reserved(), body.bottom()),
    );
    let outer_clip = ui.clip_rect();
    ui.set_clip_rect(body.intersect(outer_clip));
    let mut rows = Rows::new(area, 0.0);
    match context {
        Context::Effect => super::effect_props::effect_body(ui, app, &mut rows, &ctx),
        Context::Layer => super::layer_props::layer_body(ui, app, &mut rows, &ctx),
        Context::Paint => match tab {
            0 => super::brush_props::stencil_tab(ui, app, &mut rows),
            _ => super::layer_props::layer_body(ui, app, &mut rows, &ctx),
        },
    }
    rows.indent = 0.0;
    rows.space(8.0);
    app.m2.props_content = rows.used();
    ui.set_clip_rect(outer_clip);
    end_drag_when_released(ui, app);
    bar.end(ui, "properties.scroll", &mut app.m2.props_scroll);
}

/// スライダーのドラッグを離したら、まとめていた変更を 1 回の Undo にする。形のギズモと点のグラデーションの点のドラッグ中は終えない
/// （ペンの接触は egui のポインタの押下にならないので、ここで毎フレーム終えると、ドラッグの 1 フレームごとに別の Undo の段になる。
/// ドラッグの側が離す・Esc・フォーカスの喪失で自分で終える）。レイヤーの値を変える欄を持つパネル（プロパティ・マテリアル・ツールプロパティの
/// マスクの区分）が、描いたあとに呼ぶ。
pub fn end_drag_when_released(ui: &Ui, app: &mut AppState) {
    if !ui.input(|i| i.pointer.primary_down()) && !crate::fillfx::dragging(app) {
        app.m2_end_drag();
    }
}
