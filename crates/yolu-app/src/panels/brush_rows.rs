//! ブラシの行を描く。`brushes::rows` の表の行 1 つを、詳細のウィンドウの欄（左端に目のボタン、名前、操作、右に影響元のボタン）と、ツールプロパティの
//! パネル（狭い欄: 名前は溝や箱の中）の両方で、同じ値・同じ操作で描く。値は全部入りのブラシ（`AppState::m2.brush`）と基本の値（`AppState::brush`）を
//! 直に変え、ストロークを始めたときに写して固定する（途中で変えても、そのストロークには効かない）。影響元（筆圧・傾き・速度・ランダム）の設定は、行の右の
//! ボタンから開く小さなウィンドウ（`dynamics_window`）で変える。
//!
//! 行を足すときは `brushes::rows::Row` に足し、ここの区分の関数（`size_rows`・`ink_rows` など。`show` が `Row::category` で振り分ける）に手を 1 つ足す
//! （足し忘れは、全部の行を描いてみるテストが落とす）。

use egui::{pos2, vec2, Id, Rect, Ui, Vec2};

use super::properties::open_popup;
use crate::brushes::rows::{self, Category, Dynamics, Row};
use crate::engine::{BrushEffect, ChannelKind, Influence, MixBlur, MixGround, MixMode, MixSpace};
use crate::lang::Lang;
use crate::m2::{
    self, anti_alias_label, dual_mode_label, texture_mode_label, tip_display, BrushOp, EffectKind,
    UiOp,
};
use crate::m2_menu::Popup;
use crate::state::{Action, AppState, Tool};
use crate::ui::curve;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, Rows, SliderSpec};
use crate::ui::window::{self, Spec};
use yolu_core::brush::{Input, InputUse, Item};
use yolu_core::curve::Curve;

/// 詳細のウィンドウの行の高さと、行の間。
pub const ROW: f32 = 24.0;
pub const GAP: f32 = 4.0;
/// 目のボタンの幅と、その右の間。
const EYE: f32 = 24.0;
const EYE_GAP: f32 = 8.0;
/// 詳細の行の名前の幅。
const NAME_WIDTH: f32 = 132.0;
/// 影響元のボタンの幅。
const BUTTON: f32 = 24.0;

/// 行を置く場所。
pub enum At<'a> {
    /// 詳細のウィンドウの欄: 目のボタンの右から行の右端まで（名前は左の列に出る）。
    Detail(Rect),
    /// ツールプロパティ: 縦に積む（名前は溝や箱の中）。
    Tool(&'a mut Rows),
}

impl At<'_> {
    fn is_tool(&self) -> bool {
        matches!(self, At::Tool(_))
    }

    /// 部品の名前（詳細とツールプロパティで同じ行が同時に出ても、重ならない）。
    fn id(&self, name: &str) -> (&'static str, String) {
        (
            if self.is_tool() {
                "toolrow"
            } else {
                "brushrow"
            },
            name.to_owned(),
        )
    }
}

/// 影響元のボタンの状態: どれかの入力を使っているか・設定のウィンドウがこの行のために開いているか。
#[derive(Clone, Copy)]
struct Btn {
    on: bool,
    open: bool,
    /// 押せるか（硬さの影響元は、画像の筆先では効かないので押せない）。
    enabled: bool,
}

/// 行 1 つを描いたときの、押された影響元のボタン。
#[derive(Default)]
struct Out {
    dynamics_clicked: bool,
}

fn row_id(at: &At, row: Row, part: &str) -> (&'static str, String) {
    at.id(&format!("{}.{part}", row.id()))
}

/// 詳細の行の名前（左の列。部品の名前は溝が持つ）。
fn detail_name(ui: &mut Ui, r: Rect, text: &str, enabled: bool, tooltip: Option<&str>) {
    let shown = w::fit(ui.painter(), text, NAME_WIDTH - 4.0, t::LABEL);
    w::text(
        ui.painter(),
        r,
        &shown,
        t::LABEL.with_color(if enabled { t::TEXT } else { t::TEXT_DISABLED }),
        Align::Left,
    );
    let response = ui.interact(r, ui.id().with(("brush.name", text)), egui::Sense::hover());
    // 名前の上でも、押せない理由が出る（溝の上と同じ文）
    if let Some(tip) = tooltip.filter(|t| !t.is_empty()) {
        let _ = response.on_hover_text(tip);
    }
}

/// 影響元のボタン（ブラシの詳細・ツールプロパティで同じ。どれかの入力を使っていれば押し込んだ形、設定のウィンドウが開いていれば青い縁）。
fn dynamics_button(ui: &mut Ui, at: &At, row: Row, lang: Lang, r: Rect, btn: Btn) -> bool {
    let response = w::icon_button(
        ui,
        r,
        row_id(at, row, "dynamics"),
        "stylus",
        lang.t("brushrows-dynamics"),
        btn.on,
        btn.enabled,
        15.0,
    );
    if btn.open {
        w::outline(ui.painter(), r.expand(1.0), t::ACCENT, 1.5, 5.0);
    }
    response.clicked()
}

/// スライダー 1 行。変わった値と、影響元のボタンが押されたかを返す。
#[allow(clippy::too_many_arguments)]
fn slider(
    ui: &mut Ui,
    at: &mut At,
    row: Row,
    lang: Lang,
    value: f32,
    range: (f32, f32),
    format: NumberFormat,
    tooltip: Option<&str>,
    enabled: bool,
    btn: Option<Btn>,
) -> (Option<f32>, Out) {
    let label = row.name(lang);
    let mut out = Out::default();
    let mut spec = SliderSpec::new(label, range.0, range.1, format).enabled(enabled);
    if let Some(tip) = tooltip {
        spec = spec.tooltip(tip);
    }
    let changed = match at {
        At::Detail(body) => {
            let body = *body;
            let name = Rect::from_min_size(body.min, vec2(NAME_WIDTH, body.height()));
            detail_name(ui, name, label, enabled, tooltip);
            let right = body.right() - if btn.is_some() { BUTTON + 6.0 } else { 0.0 };
            let control = Rect::from_min_max(
                pos2(name.right() + 4.0, body.top() + 1.0),
                pos2(right, body.bottom() - 1.0),
            );
            // 溝の中の名前は出さない（名前は左の列）。部品の読み上げの名前だけ付ける
            let inside = SliderSpec {
                label: "",
                access_name: Some(label),
                ..spec
            };
            let id = row_id(at, row, "slider");
            let result = w::slider(ui, control, id, value, &inside);
            if let Some(btn) = btn {
                let button = Rect::from_min_size(
                    pos2(body.right() - BUTTON, body.top()),
                    vec2(BUTTON, body.height()),
                );
                out.dynamics_clicked = dynamics_button(ui, at, row, lang, button, btn);
            }
            result.changed.then_some(result.value)
        }
        At::Tool(rows) => {
            let r = rows.slider_row();
            let right = r.right() - if btn.is_some() { BUTTON + 3.0 } else { 0.0 };
            let control = Rect::from_min_max(r.min, pos2(right, r.bottom()));
            let id = ("toolrow", format!("{}.slider", row.id()));
            let result = w::slider(ui, control, id, value, &spec);
            if let Some(btn) = btn {
                let button = Rect::from_min_size(
                    pos2(r.right() - BUTTON, r.top()),
                    vec2(BUTTON, r.height()),
                );
                out.dynamics_clicked = dynamics_button(ui, at, row, lang, button, btn);
            }
            result.changed.then_some(result.value)
        }
    };
    (changed, out)
}

/// 0〜1 の値を % で出すスライダー（値は 0〜1 のまま受け渡す）。
#[allow(clippy::too_many_arguments)]
fn percent(
    ui: &mut Ui,
    at: &mut At,
    row: Row,
    lang: Lang,
    value: f64,
    range: (f64, f64),
    tooltip: Option<&str>,
    enabled: bool,
    btn: Option<Btn>,
) -> (Option<f64>, Out) {
    let (changed, out) = slider(
        ui,
        at,
        row,
        lang,
        (value * 100.0) as f32,
        ((range.0 * 100.0) as f32, (range.1 * 100.0) as f32),
        NumberFormat::int("%"),
        tooltip,
        enabled,
        btn,
    );
    (
        changed.map(|v| (v as f64 / 100.0).clamp(range.0, range.1)),
        out,
    )
}

/// チェック 1 行。
fn toggle(
    ui: &mut Ui,
    at: &mut At,
    row: Row,
    lang: Lang,
    value: bool,
    tooltip: Option<&str>,
    enabled: bool,
) -> Option<bool> {
    let label = row.name(lang);
    let id = row_id(at, row, "toggle");
    let next = match at {
        At::Detail(body) => w::toggle(ui, *body, id, label, value, tooltip, enabled),
        At::Tool(rows) => {
            let height = w::toggle_height(ui.painter(), rows.width(), label);
            let r = rows.row(height, 2.0);
            w::toggle(ui, r, id, label, value, tooltip, enabled)
        }
    };
    (next != value).then_some(next)
}

/// 名前と値の箱（押すとメニュー）の 1 行。押されたら箱の矩形を返す。
fn dropdown(
    ui: &mut Ui,
    at: &mut At,
    row: Row,
    lang: Lang,
    value: &str,
    tooltip: Option<&str>,
    enabled: bool,
) -> Option<Rect> {
    let label = row.name(lang);
    let id = row_id(at, row, "choice");
    match at {
        // 詳細の欄: 名前を左の列にそろえた箱（名前の部分を押しても開く。部品の名前は「名前: 値」）
        At::Detail(body) => {
            let (response, anchor) = w::dropdown(
                ui,
                *body,
                id,
                Some(label),
                value,
                tooltip,
                enabled,
                NAME_WIDTH + 4.0,
            );
            response.clicked().then_some(anchor)
        }
        At::Tool(rows) => {
            let r = rows.row(t::ROW_HEIGHT, 4.0);
            let label_width =
                (w::text_width(ui.painter(), label, t::LABEL) + 10.0).min(r.width() * 0.6);
            let (response, anchor) =
                w::dropdown(ui, r, id, Some(label), value, tooltip, enabled, label_width);
            response.clicked().then_some(anchor)
        }
    }
}

// ───────── 行の表示の条件 ─────────

/// 行を今のブラシ・ツール・画面の状態で出すか。ツールプロパティでも詳細でも、同じ判定（効果のブラシだけの行・3D のビューが出ているときだけの行）。
pub fn visible(app: &AppState, row: Row) -> bool {
    match row {
        Row::EffectBlur => matches!(app.m2.brush.effect, BrushEffect::Blur { .. }),
        Row::EffectSmudge => matches!(app.m2.brush.effect, BrushEffect::Smudge { .. }),
        Row::EffectCloneX
        | Row::EffectCloneY
        | Row::EffectCloneAllLayers
        | Row::EffectCloneAligned => matches!(app.m2.brush.effect, BrushEffect::Clone { .. }),
        Row::ProjectionAlignment
        | Row::ProjectionHidden
        | Row::ProjectionBackfaces
        | Row::ProjectionFalloff
        | Row::ProjectionBleed => app.view3d.paintable_on_screen(),
        Row::ProjectionAngleStart | Row::ProjectionAngleEnd => {
            app.view3d.paintable_on_screen() && app.view3d.projection.angle_falloff
        }
        // 色混ぜの拡張の行は、混ぜ方を選んでいるときだけ（なしの間は効かない行を並べない）
        Row::MixSpace | Row::MixCorrection | Row::MixBlur | Row::MixBlurLinked | Row::MixLoad => {
            app.m2.brush.mix.is_active()
        }
        _ => true,
    }
}

/// 硬さが効くか。丸い筆先の縁の硬さなので、画像の筆先では効かない（画像の縁のまま。2D も 3D の面のダブも同じ）。
pub fn hardness_applies(app: &AppState) -> bool {
    let tip = &app.m2.brush.tip;
    tip.image.is_none() && tip.images.is_empty()
}

/// 色の混ぜが、今のツール・ブラシ・描く先では効かない理由（効くなら None）。
fn mix_unavailable(app: &AppState, lang: Lang) -> Option<&'static str> {
    if app.tool == Tool::Eraser || app.m2.erase_mode {
        return Some(lang.t("panels-no-effect-with-the-eraser"));
    }
    if !app.m2.brush.effect.is_paint() {
        return Some(lang.t("panels-no-effect-on-effect-brushes"));
    }
    color_unavailable(app, lang)
}

/// 描く先のチャンネルが色を持たない（データのチャンネル・マスク）とき、その理由。
fn color_unavailable(app: &AppState, lang: Lang) -> Option<&'static str> {
    let kind = app
        .session
        .doc
        .channel_info(app.m2.paint_channel)
        .map(|i| i.kind)
        .unwrap_or(ChannelKind::Color);
    (app.m2.edit_mask || !yolu_core::brush::carries_color(kind))
        .then(|| lang.t("panels-no-effect-on-this-channel"))
}

// ───────── 影響元 ─────────

/// 項目の影響元（筆圧の切り替えは、大きさ・不透明度・流量だけ `AppState::brush` が持つので、それを重ねて読む）。
pub fn influence_of(app: &AppState, dynamics: Dynamics) -> Influence {
    let mut influence = dynamics.get(&app.m2.brush.influences).clone();
    if let Dynamics::Item(item) = dynamics {
        let switches = app.brush.pressure;
        match item {
            Item::Size => influence.pressure.on = switches.size,
            Item::Opacity => influence.pressure.on = switches.opacity,
            Item::Flow => influence.pressure.on = switches.flow,
            _ => {}
        }
    }
    influence
}

/// 項目の影響元を書き戻す（大きさ・不透明度・流量の筆圧の切り替えは `AppState::brush` へも）。
pub fn set_influence(app: &mut AppState, dynamics: Dynamics, influence: Influence) {
    if let Dynamics::Item(item) = dynamics {
        let on = influence.pressure.on;
        match item {
            Item::Size => app.brush.pressure.size = on,
            Item::Opacity => app.brush.pressure.opacity = on,
            Item::Flow => app.brush.pressure.flow = on,
            _ => {}
        }
    }
    *dynamics.get_mut(&mut app.m2.brush.influences) = influence;
}

fn button_state(app: &AppState, row: Row) -> Option<Btn> {
    let dynamics = row.dynamics()?;
    Some(Btn {
        on: influence_of(app, dynamics).is_on(),
        open: app.brushes.ui.detail.dynamics == Some(row),
        enabled: dynamics_usable(app, row),
    })
}

/// 行の影響元が今のブラシで効くか（硬さは、画像の筆先では効かない）。
fn dynamics_usable(app: &AppState, row: Row) -> bool {
    row != Row::Hardness || hardness_applies(app)
}

/// 影響元のボタンが押されたとき: その行の設定のウィンドウを開く（同じ行をもう一度押すと閉じる）。
fn toggle_dynamics_window(app: &mut AppState, row: Row) {
    let detail = &mut app.brushes.ui.detail;
    detail.dynamics = (detail.dynamics != Some(row)).then_some(row);
}

// ───────── 行 ─────────

/// 行を描く関数に渡す、行ごとの決まった値。
#[derive(Clone, Copy)]
struct Env {
    lang: Lang,
    /// 値を変えられるか（ツールプロパティはストロークの最中は触れない）。
    live: bool,
    /// ストロークの最中でないか（3D の塗りの切り替えは、どのパネルでもストロークの最中は触れない）。
    free: bool,
    btn: Option<Btn>,
}

/// 行 1 つを描く（`visible` が偽なら呼ばない）。区分ごとの関数に振り分ける。
pub fn show(ui: &mut Ui, app: &mut AppState, ctx: &egui::Context, row: Row, at: &mut At) {
    let free = !app.is_stroking();
    let env = Env {
        lang: app.lang,
        live: !at.is_tool() || free,
        free,
        btn: button_state(app, row),
    };
    let out = match row.category() {
        Category::Size => size_rows(ui, app, ctx, row, at, env),
        Category::Ink => ink_rows(ui, app, ctx, row, at, env),
        Category::ColorChange => color_rows(ui, app, ctx, row, at, env),
        Category::AntiAlias => anti_alias_rows(ui, app, ctx, row, at, env),
        Category::Tip => tip_rows(ui, app, ctx, row, at, env),
        Category::Spray => spray_rows(ui, app, ctx, row, at, env),
        Category::Stroke => stroke_rows(ui, app, ctx, row, at, env),
        Category::Paper => paper_rows(ui, app, ctx, row, at, env),
        Category::DualTip | Category::DualSpray | Category::DualStroke => {
            dual_rows(ui, app, ctx, row, at, env)
        }
        Category::DualPaper => dual_paper_rows(ui, app, ctx, row, at, env),
        Category::Erase => erase_rows(ui, app, ctx, row, at, env),
        Category::WaterEdge => water_edge_rows(ui, app, ctx, row, at, env),
        Category::ReferenceLines => refer_rows(ui, app, ctx, row, at, env),
        Category::Stabilization => stabilization_rows(ui, app, ctx, row, at, env),
        Category::Taper => taper_rows(ui, app, ctx, row, at, env),
        // 見出しの区分に行は無い
        Category::Shape | Category::Dual => Out::default(),
    };
    if out.dynamics_clicked {
        toggle_dynamics_window(app, row);
    }
}

/// ブラシサイズの行。
fn size_rows(
    ui: &mut Ui,
    app: &mut AppState,
    _ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env {
        lang, live, btn, ..
    } = env;
    match row {
        Row::Size => {
            let (changed, out) = slider(
                ui,
                at,
                row,
                lang,
                app.brush.radius * 2.0,
                (1.0, 256.0),
                NumberFormat::int(" px"),
                Some(lang.t("panels-brush-diameter-and-tip")),
                live,
                btn,
            );
            if let Some(v) = changed {
                app.brush.radius = (v / 2.0).max(0.5);
            }
            out
        }
        Row::SizeByScreen => {
            // パスは `BrushSettings` だけを持ち、`Controls` を描きに渡さないので、パスのツールの間は押せない
            let usable = app.tool != Tool::Path;
            let reason = (!usable).then(|| lang.t("panels-not-available-with-paths"));
            if let Some(v) = toggle(
                ui,
                at,
                row,
                lang,
                app.m2.brush.controls.size_by_screen,
                reason,
                usable && live,
            ) {
                app.m2.brush.controls.size_by_screen = v;
            }
            Out::default()
        }
        Row::MinOnePixel => {
            let usable = app.tool != Tool::Path;
            let reason = (!usable).then(|| lang.t("panels-not-available-with-paths"));
            if let Some(v) = toggle(
                ui,
                at,
                row,
                lang,
                app.m2.brush.controls.min_one_pixel,
                reason,
                usable && live,
            ) {
                app.m2.brush.controls.min_one_pixel = v;
            }
            Out::default()
        }
        Row::JitterSize => {
            let (v, out) = percent(
                ui,
                at,
                row,
                lang,
                app.m2.brush.jitter.size,
                (0.0, 1.0),
                None,
                live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.jitter.size = v;
            }
            out
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// インク（不透明度・流量。色混ぜと効果は別の関数）の行。
fn ink_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env {
        lang, live, btn, ..
    } = env;
    match row {
        Row::MixMode | Row::MixPaint | Row::MixDensity | Row::MixStretch | Row::MixGround => {
            mix_rows(ui, app, ctx, row, at, env)
        }
        Row::MixSpace | Row::MixCorrection | Row::MixBlur | Row::MixBlurLinked | Row::MixLoad => {
            mix_more_rows(ui, app, ctx, row, at, env)
        }
        Row::EffectKind
        | Row::EffectBlur
        | Row::EffectSmudge
        | Row::EffectCloneX
        | Row::EffectCloneY
        | Row::EffectCloneAllLayers
        | Row::EffectCloneAligned => effect_rows(ui, app, ctx, row, at, env),
        Row::Opacity => {
            let (changed, out) = slider(
                ui,
                at,
                row,
                lang,
                app.brush.opacity * 100.0,
                (0.0, 100.0),
                NumberFormat::int("%"),
                None,
                live,
                btn,
            );
            if let Some(v) = changed {
                app.brush.opacity = v / 100.0;
            }
            out
        }
        Row::Flow => {
            let (changed, out) = slider(
                ui,
                at,
                row,
                lang,
                app.brush.flow * 100.0,
                (0.0, 100.0),
                NumberFormat::int("%"),
                None,
                live,
                btn,
            );
            if let Some(v) = changed {
                app.brush.flow = v / 100.0;
            }
            out
        }
        Row::JitterOpacity => {
            let (v, out) = percent(
                ui,
                at,
                row,
                lang,
                app.m2.brush.jitter.opacity,
                (0.0, 1.0),
                None,
                live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.jitter.opacity = v;
            }
            out
        }
        Row::JitterFlow => {
            let (v, out) = percent(
                ui,
                at,
                row,
                lang,
                app.m2.brush.jitter.flow,
                (0.0, 1.0),
                None,
                live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.jitter.flow = v;
            }
            out
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// インクの色混ぜの行。
fn mix_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env {
        lang, live, btn, ..
    } = env;
    match row {
        Row::MixMode => {
            let reason = mix_unavailable(app, lang);
            let name = match app.m2.brush.mix.mode {
                MixMode::Off => lang.t("panels-none"),
                MixMode::Mix => lang.t("panels-mix"),
                MixMode::Layer => lang.t("panels-mix-layer"),
                MixMode::Smear => lang.t("panels-smear"),
            };
            if let Some(anchor) =
                dropdown(ui, at, row, lang, name, reason, reason.is_none() && live)
            {
                open_popup(app, ctx, Popup::MixMode, anchor, anchor.width());
            }
            Out::default()
        }
        Row::MixPaint | Row::MixDensity => {
            let off = mix_unavailable(app, lang).or_else(|| {
                (!app.m2.brush.mix.is_active()).then(|| lang.t("panels-mixing-is-off"))
            });
            let value = if row == Row::MixPaint {
                app.m2.brush.mix.paint
            } else {
                app.m2.brush.mix.density
            };
            let (v, out) = percent(
                ui,
                at,
                row,
                lang,
                value,
                (0.0, 1.0),
                off,
                off.is_none() && live,
                btn,
            );
            if let Some(v) = v {
                if row == Row::MixPaint {
                    app.m2.brush.mix.paint = v;
                } else {
                    app.m2.brush.mix.density = v;
                }
            }
            out
        }
        Row::MixStretch => {
            let off = mix_unavailable(app, lang).or_else(|| {
                (!app.m2.brush.mix.is_active()).then(|| lang.t("panels-mixing-is-off"))
            });
            let (v, out) = percent(
                ui,
                at,
                row,
                lang,
                app.m2.brush.mix.stretch,
                (0.0, 1.0),
                off,
                off.is_none() && live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.mix.stretch = v;
            }
            out
        }
        Row::MixGround => {
            let off = mix_unavailable(app, lang).or_else(|| {
                (!app.m2.brush.mix.is_active()).then(|| lang.t("panels-mixing-is-off"))
            });
            let masked = app.m2.edit_mask;
            if let Some(v) = toggle(
                ui,
                at,
                row,
                lang,
                app.m2.brush.mix.ground == MixGround::Composite,
                off,
                off.is_none() && !masked && live,
            ) {
                app.m2.brush.mix.ground = if v {
                    MixGround::Composite
                } else {
                    MixGround::Layer
                };
            }
            Out::default()
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// 固定のにじむ強さを入れたときの初めの値（画素）。大きさに連動する形から固定へ替えるとき、この値から始める。
const FIXED_MIX_BLUR: f64 = 2.0;

/// インクの色混ぜの拡張の行（混色モード・輝度の補正・にじむ強さと連動・描き始めの絵の具の量）。
fn mix_more_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, live, .. } = env;
    let off = mix_unavailable(app, lang)
        .or_else(|| (!app.m2.brush.mix.is_active()).then(|| lang.t("panels-mixing-is-off")));
    let mix = app.m2.brush.mix.clone();
    let smear = off.is_none() && mix.mode == MixMode::Smear;
    match row {
        Row::MixSpace => {
            let label = m2::mix_spaces(lang)
                .into_iter()
                .find(|(s, _)| *s == mix.space)
                .map_or("", |(_, l)| l);
            if let Some(anchor) = dropdown(ui, at, row, lang, label, off, off.is_none() && live) {
                open_popup(app, ctx, Popup::MixSpace, anchor, anchor.width());
            }
        }
        Row::MixCorrection => {
            let label = m2::mix_corrections(lang)
                .into_iter()
                .find(|(c, _)| *c == mix.correction)
                .map_or("", |(_, l)| l);
            // 輝度の補正は知覚的のときだけ働く
            let reason = off.or_else(|| {
                (mix.space != MixSpace::Perceptual)
                    .then(|| lang.t("panels-available-only-when-the-color-tip"))
            });
            if let Some(anchor) =
                dropdown(ui, at, row, lang, label, reason, reason.is_none() && live)
            {
                open_popup(app, ctx, Popup::MixCorrection, anchor, anchor.width());
            }
        }
        Row::MixBlur => {
            let (value, fixed) = match mix.blur {
                MixBlur::Fixed(v) => (v, true),
                _ => (FIXED_MIX_BLUR, false),
            };
            let (v, out) = slider(
                ui,
                at,
                row,
                lang,
                value as f32,
                (0.0, yolu_core::MAX_MIX_BLUR as f32),
                NumberFormat {
                    decimals: 1,
                    trim: true,
                    suffix: " px",
                },
                off,
                smear && fixed && live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.mix.blur =
                    MixBlur::Fixed((v as f64).clamp(0.0, yolu_core::MAX_MIX_BLUR));
            }
            return out;
        }
        Row::MixBlurLinked => {
            let linked = !matches!(mix.blur, MixBlur::Fixed(_));
            if let Some(v) = toggle(ui, at, row, lang, linked, off, smear && live) {
                app.m2.brush.mix.blur = if v {
                    MixBlur::Linked
                } else {
                    MixBlur::Fixed(FIXED_MIX_BLUR)
                };
            }
        }
        Row::MixLoad => {
            let (v, out) = percent(
                ui,
                at,
                row,
                lang,
                mix.load,
                (0.0, 1.0),
                off,
                off.is_none() && live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.mix.load = v;
            }
            return out;
        }
        other => debug_assert!(false, "{other:?}: この区分の関数に描く手が無い"),
    }
    Out::default()
}

/// インクの効果（ぼかし・指先・クローン）の行。
fn effect_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, live, .. } = env;
    match row {
        Row::EffectKind => {
            let usable = app.tool != Tool::Eraser;
            let eraser = (!usable).then(|| lang.t("panels-not-available-with-the-eraser"));
            let kind = EffectKind::of(&app.m2.brush.effect);
            if let Some(anchor) =
                dropdown(ui, at, row, lang, kind.name(lang), eraser, usable && live)
            {
                open_popup(app, ctx, Popup::Effect, anchor, anchor.width());
            }
            Out::default()
        }
        Row::EffectBlur => {
            if let BrushEffect::Blur { radius } = app.m2.brush.effect {
                let off =
                    (app.tool.erases()).then(|| lang.t("panels-not-available-with-the-eraser"));
                let (v, out) = slider(
                    ui,
                    at,
                    row,
                    lang,
                    radius as f32,
                    (1.0, 64.0),
                    NumberFormat::int(" px"),
                    off,
                    live && off.is_none(),
                    None,
                );
                if let Some(v) = v {
                    app.m2.brush.effect = BrushEffect::Blur {
                        radius: v.round().clamp(1.0, 64.0) as u32,
                    };
                }
                out
            } else {
                Out::default()
            }
        }
        Row::EffectSmudge => {
            if let BrushEffect::Smudge { strength } = app.m2.brush.effect {
                let off =
                    (app.tool.erases()).then(|| lang.t("panels-not-available-with-the-eraser"));
                let (v, out) = percent(
                    ui,
                    at,
                    row,
                    lang,
                    strength,
                    (0.0, 1.0),
                    off,
                    live && off.is_none(),
                    None,
                );
                if let Some(v) = v {
                    app.m2.brush.effect = BrushEffect::Smudge {
                        strength: v.clamp(0.0, 1.0),
                    };
                }
                out
            } else {
                Out::default()
            }
        }
        Row::EffectCloneX | Row::EffectCloneY => {
            clone_offset(ui, app, at, row, lang, live);
            Out::default()
        }
        Row::EffectCloneAllLayers => {
            let masked = app.m2.edit_mask;
            let usable = app.tool != Tool::Eraser;
            let reason = if !usable {
                Some(lang.t("panels-not-available-with-the-eraser"))
            } else {
                masked.then(|| lang.t("panels-while-painting-a-mask-only-tip"))
            };
            if let Some(v) = toggle(
                ui,
                at,
                row,
                lang,
                app.clone.all_layers && !masked,
                reason,
                usable && !masked && live,
            ) {
                app.clone.all_layers = v;
            }
            Out::default()
        }
        Row::EffectCloneAligned => {
            clone_aligned(ui, app, at, row, lang, live);
            Out::default()
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// 色の変化（ブラシ先端ごと。ストロークごと・その他は別の関数）の行。
fn color_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env {
        lang, live, btn, ..
    } = env;
    match row {
        Row::ColorStroke
        | Row::ColorStrokeHue
        | Row::ColorStrokeSaturation
        | Row::ColorStrokeValue
        | Row::ColorStrokeSubMix => color_stroke_rows(ui, app, ctx, row, at, env),
        Row::ColorTarget
        | Row::ColorBackground
        | Row::ColorJitterForeBack
        | Row::ColorJitterHue
        | Row::ColorJitterSaturation
        | Row::ColorJitterBrightness
        | Row::ColorPurity
        | Row::ColorPerTip => color_other_rows(ui, app, ctx, row, at, env),
        Row::ColorTip => {
            let free = color_unavailable(app, lang);
            let on = app.m2.brush.color.tip.is_some();
            if let Some(v) = toggle(ui, at, row, lang, on, free, free.is_none() && live) {
                let c = &mut app.m2.brush.color;
                if v {
                    c.tip = Some(app.m2.color_tip_stash);
                } else if let Some(tip) = c.tip.take() {
                    app.m2.color_tip_stash = tip;
                }
            }
            Out::default()
        }
        Row::ColorHue | Row::ColorSaturation | Row::ColorValue => {
            let free = color_unavailable(app, lang);
            let shown = app.m2.brush.color.tip.unwrap_or(app.m2.color_tip_stash);
            let (value, range, suffix, scale) = match row {
                Row::ColorHue => (shown.hue as f32, (-360.0, 360.0), "°", 1.0),
                Row::ColorSaturation => {
                    (shown.saturation as f32 * 100.0, (-100.0, 100.0), "%", 100.0)
                }
                _ => (shown.value as f32 * 100.0, (-100.0, 100.0), "%", 100.0),
            };
            let enabled = app.m2.brush.color.tip.is_some() && free.is_none() && live;
            let (v, out) = slider(
                ui,
                at,
                row,
                lang,
                value,
                range,
                NumberFormat::int(suffix),
                free,
                enabled,
                btn,
            );
            if let (Some(v), Some(tip)) = (v, app.m2.brush.color.tip.as_mut()) {
                let v = (v / scale) as f64;
                match row {
                    Row::ColorHue => tip.hue = v.clamp(-360.0, 360.0),
                    Row::ColorSaturation => tip.saturation = v.clamp(-1.0, 1.0),
                    _ => tip.value = v.clamp(-1.0, 1.0),
                }
            }
            out
        }
        Row::ColorSubMix => {
            let free = color_unavailable(app, lang);
            let (v, out) = percent(
                ui,
                at,
                row,
                lang,
                app.m2.brush.color.sub_mix,
                (0.0, 1.0),
                free,
                free.is_none() && live,
                btn,
            );
            if let Some(v) = v {
                app.m2.brush.color.sub_mix = v;
            }
            out
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// 色の変化（ストロークごと）の行。
fn color_stroke_rows(
    ui: &mut Ui,
    app: &mut AppState,
    _ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, live, .. } = env;
    match row {
        Row::ColorStroke => {
            let free = color_unavailable(app, lang);
            let on = app.m2.brush.color.stroke.is_some();
            if let Some(v) = toggle(ui, at, row, lang, on, free, free.is_none() && live) {
                let c = &mut app.m2.brush.color;
                if v {
                    c.stroke = Some(app.m2.color_stroke_stash);
                } else if let Some(stroke) = c.stroke.take() {
                    app.m2.color_stroke_stash = stroke;
                }
            }
            Out::default()
        }
        Row::ColorStrokeHue
        | Row::ColorStrokeSaturation
        | Row::ColorStrokeValue
        | Row::ColorStrokeSubMix => {
            let free = color_unavailable(app, lang);
            let shown = app
                .m2
                .brush
                .color
                .stroke
                .unwrap_or(app.m2.color_stroke_stash);
            let (value, range, suffix, scale) = match row {
                Row::ColorStrokeHue => (shown.hue as f32, (0.0, 180.0), "°", 1.0),
                Row::ColorStrokeSaturation => {
                    (shown.saturation as f32 * 100.0, (0.0, 100.0), "%", 100.0)
                }
                Row::ColorStrokeValue => (shown.value as f32 * 100.0, (0.0, 100.0), "%", 100.0),
                _ => (shown.sub_mix as f32 * 100.0, (0.0, 100.0), "%", 100.0),
            };
            let enabled = app.m2.brush.color.stroke.is_some() && free.is_none() && live;
            let (v, _) = slider(
                ui,
                at,
                row,
                lang,
                value,
                range,
                NumberFormat::int(suffix),
                free,
                enabled,
                None,
            );
            if let (Some(v), Some(stroke)) = (v, app.m2.brush.color.stroke.as_mut()) {
                let v = (v / scale) as f64;
                match row {
                    Row::ColorStrokeHue => stroke.hue = v.clamp(0.0, 180.0),
                    Row::ColorStrokeSaturation => stroke.saturation = v.clamp(0.0, 1.0),
                    Row::ColorStrokeValue => stroke.value = v.clamp(0.0, 1.0),
                    _ => stroke.sub_mix = v.clamp(0.0, 1.0),
                }
            }
            Out::default()
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// 色の変化（描く色・ばらつき）の行。
fn color_other_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, live, .. } = env;
    match row {
        Row::ColorTarget => {
            let free = color_unavailable(app, lang);
            let current = app.m2.brush.color.target;
            let name = m2::color_targets(lang)
                .into_iter()
                .find(|(t, _)| *t == current)
                .map_or("", |(_, n)| n);
            if let Some(anchor) = dropdown(ui, at, row, lang, name, free, free.is_none() && live) {
                open_popup(app, ctx, Popup::ColorTarget, anchor, anchor.width());
            }
            Out::default()
        }
        Row::ColorBackground => {
            background_swatch(ui, app, at, row, lang);
            Out::default()
        }
        Row::ColorJitterForeBack
        | Row::ColorJitterHue
        | Row::ColorJitterSaturation
        | Row::ColorJitterBrightness => {
            let free = color_unavailable(app, lang);
            let c = &mut app.m2.brush.color;
            let field = match row {
                Row::ColorJitterForeBack => &mut c.foreground_background,
                Row::ColorJitterHue => &mut c.hue,
                Row::ColorJitterSaturation => &mut c.saturation,
                _ => &mut c.brightness,
            };
            let (v, out) = percent(
                ui,
                at,
                row,
                lang,
                *field,
                (0.0, 1.0),
                free,
                free.is_none() && live,
                None,
            );
            if let Some(v) = v {
                *field = v;
            }
            out
        }
        Row::ColorPurity => {
            let free = color_unavailable(app, lang);
            let (v, out) = percent(
                ui,
                at,
                row,
                lang,
                app.m2.brush.color.purity,
                (-1.0, 1.0),
                free,
                free.is_none() && live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.color.purity = v;
            }
            out
        }
        Row::ColorPerTip => {
            let free = color_unavailable(app, lang);
            if let Some(v) = toggle(
                ui,
                at,
                row,
                lang,
                app.m2.brush.color.per_tip,
                free,
                free.is_none() && live,
            ) {
                app.m2.brush.color.per_tip = v;
            }
            Out::default()
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// アンチエイリアスの行。
fn anti_alias_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, live, .. } = env;
    match row {
        Row::AntiAlias => {
            if let Some(anchor) = dropdown(
                ui,
                at,
                row,
                lang,
                anti_alias_label(lang, app.brush.anti_alias),
                None,
                live,
            ) {
                open_popup(app, ctx, Popup::AntiAlias, anchor, anchor.width());
            }
            Out::default()
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// ブラシ先端（形。向きの規則・繰り返しは別の関数）の行。
fn tip_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env {
        lang, live, btn, ..
    } = env;
    match row {
        Row::JitterAngle | Row::JitterRoundness | Row::Repeat | Row::FlipX | Row::FlipY => {
            tip_rule_rows(ui, app, ctx, row, at, env)
        }
        Row::Hardness => {
            let applies = hardness_applies(app);
            let reason = (!applies).then(|| lang.t("panels-no-effect-on-an-image"));
            let (changed, out) = slider(
                ui,
                at,
                row,
                lang,
                app.brush.hardness * 100.0,
                (0.0, 100.0),
                NumberFormat::int("%"),
                reason,
                applies && live,
                btn,
            );
            if let Some(v) = changed {
                app.brush.hardness = v / 100.0;
            }
            out
        }
        Row::Roundness => {
            let (v, out) = percent(
                ui,
                at,
                row,
                lang,
                app.m2.brush.tip.roundness,
                (0.01, 1.0),
                None,
                live,
                btn,
            );
            if let Some(v) = v {
                app.m2.brush.tip.roundness = v;
            }
            out
        }
        Row::TipAngle => {
            let (v, out) = slider(
                ui,
                at,
                row,
                lang,
                app.m2.brush.tip.angle as f32,
                (-180.0, 180.0),
                NumberFormat::int("°"),
                None,
                live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.tip.angle = v as f64;
            }
            out
        }
        Row::FollowDirection => {
            if let Some(v) = toggle(
                ui,
                at,
                row,
                lang,
                app.m2.brush.tip.follow_direction,
                None,
                live,
            ) {
                app.m2.brush.tip.follow_direction = v;
            }
            Out::default()
        }
        Row::TiltAngle => {
            if let Some(v) = toggle(
                ui,
                at,
                row,
                lang,
                app.m2.brush.controls.tilt_angle,
                None,
                live,
            ) {
                app.m2.brush.controls.tilt_angle = v;
            }
            Out::default()
        }
        Row::RotationAngle => {
            if let Some(v) = toggle(
                ui,
                at,
                row,
                lang,
                app.m2.brush.controls.rotation_angle,
                None,
                live,
            ) {
                app.m2.brush.controls.rotation_angle = v;
            }
            Out::default()
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// ブラシ先端（ばらつき・繰り返し・反転）の行。
fn tip_rule_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, live, .. } = env;
    match row {
        Row::JitterAngle => {
            let (v, out) = percent(
                ui,
                at,
                row,
                lang,
                app.m2.brush.jitter.angle,
                (0.0, 1.0),
                None,
                live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.jitter.angle = v;
            }
            out
        }
        Row::JitterRoundness => {
            let (v, out) = percent(
                ui,
                at,
                row,
                lang,
                app.m2.brush.jitter.roundness,
                (0.0, 1.0),
                None,
                live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.jitter.roundness = v;
            }
            out
        }
        Row::Repeat => {
            let current = app.m2.brush.tip.selection;
            let name = m2::tip_repeats(lang)
                .into_iter()
                .find(|(s, _)| *s == current)
                .map_or("", |(_, n)| n);
            if let Some(anchor) = dropdown(ui, at, row, lang, name, None, live) {
                open_popup(app, ctx, Popup::TipRepeat, anchor, anchor.width());
            }
            Out::default()
        }
        Row::FlipX | Row::FlipY => {
            let tip = &app.m2.brush.tip;
            let image = tip.image.is_some() || !tip.images.is_empty();
            let reason = (!image).then(|| lang.t("panels-mirrors-an-image-tip-tip"));
            let current = if row == Row::FlipX {
                tip.flip_x
            } else {
                tip.flip_y
            };
            let name = m2::flip_rules(lang)
                .into_iter()
                .find(|(r, _)| *r == current)
                .map_or("", |(_, n)| n);
            if let Some(anchor) = dropdown(ui, at, row, lang, name, reason, image && live) {
                let popup = if row == Row::FlipX {
                    Popup::FlipX
                } else {
                    Popup::FlipY
                };
                open_popup(app, ctx, popup, anchor, anchor.width());
            }
            Out::default()
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// 散布効果の行。
fn spray_rows(
    ui: &mut Ui,
    app: &mut AppState,
    _ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, live, .. } = env;
    match row {
        Row::Scatter => {
            let (v, out) = percent(
                ui,
                at,
                row,
                lang,
                app.m2.brush.jitter.scatter,
                (0.0, 10.0),
                None,
                live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.jitter.scatter = v;
            }
            out
        }
        Row::ScatterCount => {
            let (v, out) = slider(
                ui,
                at,
                row,
                lang,
                app.m2.brush.jitter.count as f32,
                (1.0, 16.0),
                NumberFormat::int(""),
                None,
                live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.jitter.count = v.round().clamp(1.0, 16.0) as u32;
            }
            out
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// ストロークの行。
fn stroke_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, live, .. } = env;
    match row {
        Row::Spacing => {
            let (changed, out) = slider(
                ui,
                at,
                row,
                lang,
                app.brush.spacing * 100.0,
                (1.0, 100.0),
                NumberFormat::int("%"),
                None,
                live,
                None,
            );
            if let Some(v) = changed {
                app.brush.spacing = v / 100.0;
            }
            out
        }
        Row::Continuous => {
            // 後補正が入っている間は効かない（行も触れない）
            let brush = &app.m2.brush;
            let enabled = live && !brush.assist.post.on;
            if let Some(v) = toggle(ui, at, row, lang, brush.tip.continuous, None, enabled) {
                app.m2.brush.tip.continuous = v;
            }
            Out::default()
        }
        Row::Ribbon => {
            // 散布効果が入り・筆先が円形の間は効かない（行も触れない）
            let brush = &app.m2.brush;
            let enabled = live && brush.spray.is_none() && !hardness_applies(app);
            if let Some(v) = toggle(ui, at, row, lang, brush.tip.ribbon, None, enabled) {
                app.m2.brush.tip.ribbon = v;
            }
            Out::default()
        }
        Row::DarkenOverlap => {
            if let Some(v) = toggle(
                ui,
                at,
                row,
                lang,
                app.m2.brush.tip.darken_overlap,
                None,
                live,
            ) {
                app.m2.brush.tip.darken_overlap = v;
            }
            Out::default()
        }
        Row::SpeedMax => {
            let (v, out) = slider(
                ui,
                at,
                row,
                lang,
                app.m2.brush.influences.speed_max as f32,
                (100.0, 10000.0),
                NumberFormat::int(" px/s"),
                None,
                live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.influences.speed_max = v as f64;
            }
            out
        }
        Row::ProjectionAlignment => {
            let alignment = app.view3d.projection.alignment;
            if let Some(anchor) = dropdown(
                ui,
                at,
                row,
                lang,
                crate::m2::alignment_label(lang, alignment),
                None,
                env.free,
            ) {
                open_popup(app, ctx, Popup::ProjectionAlignment, anchor, anchor.width());
            }
            Out::default()
        }
        Row::ProjectionHidden | Row::ProjectionBackfaces | Row::ProjectionFalloff => {
            // 隠れた所・面の向きの弱めは、カメラから投影する塗りだけの切り替え
            let free = env.free && (row == Row::ProjectionBackfaces || camera_aligned(app));
            let p = &mut app.view3d.projection;
            let field = match row {
                Row::ProjectionHidden => &mut p.paint_hidden,
                Row::ProjectionBackfaces => &mut p.paint_backfaces,
                _ => &mut p.angle_falloff,
            };
            if let Some(v) = toggle(ui, at, row, lang, *field, None, free) {
                *field = v;
            }
            Out::default()
        }
        Row::ProjectionAngleStart | Row::ProjectionAngleEnd | Row::ProjectionBleed => {
            let free = env.free && (row == Row::ProjectionBleed || camera_aligned(app));
            projection_slider(ui, app, at, row, lang, free);
            Out::default()
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// 紙質の行。
fn paper_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env {
        lang, live, btn, ..
    } = env;
    match row {
        Row::PaperImage => {
            let name = app
                .m2
                .brush
                .texture
                .as_ref()
                .map(|t| tip_display(lang, &t.image))
                .unwrap_or_else(|| lang.t("panels-none").to_owned());
            if let Some(anchor) = dropdown(
                ui,
                at,
                row,
                lang,
                &name,
                Some(lang.t("panels-paper-texture-tip")),
                live,
            ) {
                open_popup(app, ctx, Popup::Texture, anchor, anchor.width());
            }
            Out::default()
        }
        Row::PaperDepth => {
            let on = app.m2.brush.texture.is_some();
            let depth = app
                .m2
                .brush
                .texture
                .as_ref()
                .map_or(app.m2.texture_prefs.depth, |t| t.depth);
            let (v, out) = percent(ui, at, row, lang, depth, (0.0, 1.0), None, on && live, btn);
            if let (Some(v), Some(t)) = (v, app.m2.brush.texture.as_mut()) {
                t.depth = v;
            }
            out
        }
        Row::PaperScale => {
            let on = app.m2.brush.texture.is_some();
            let scale = app
                .m2
                .brush
                .texture
                .as_ref()
                .map_or(app.m2.texture_prefs.scale, |t| t.scale);
            let (v, out) = slider(
                ui,
                at,
                row,
                lang,
                scale as f32,
                (0.05, 16.0),
                NumberFormat {
                    decimals: 2,
                    trim: true,
                    suffix: "",
                },
                None,
                on && live,
                None,
            );
            if let (Some(v), Some(t)) = (v, app.m2.brush.texture.as_mut()) {
                t.scale = v as f64;
            }
            out
        }
        Row::PaperAngle => {
            let on = app.m2.brush.texture.is_some();
            let angle = app.m2.brush.texture.as_ref().map_or(0.0, |t| t.angle);
            let (v, out) = slider(
                ui,
                at,
                row,
                lang,
                angle as f32,
                (-180.0, 180.0),
                NumberFormat::int("°"),
                None,
                on && live,
                None,
            );
            if let (Some(v), Some(t)) = (v, app.m2.brush.texture.as_mut()) {
                t.angle = v as f64;
            }
            out
        }
        Row::PaperMode => {
            let on = app.m2.brush.texture.is_some();
            let mode = app
                .m2
                .brush
                .texture
                .as_ref()
                .map_or(app.m2.texture_prefs.mode, |t| t.mode);
            if let Some(anchor) = dropdown(
                ui,
                at,
                row,
                lang,
                texture_mode_label(lang, mode),
                None,
                on && live,
            ) {
                open_popup(app, ctx, Popup::TextureMode, anchor, anchor.width());
            }
            Out::default()
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// 2-ブラシ形状（先端・散布効果・ストロークの行）の行。
fn dual_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, live, .. } = env;
    match row {
        Row::DualEnabled => {
            let on = app.m2.brush.dual.is_some();
            if let Some(v) = toggle(ui, at, row, lang, on, None, live) {
                app.apply(Action::M2Ui(UiOp::Brush(BrushOp::DualEnabled(v))));
            }
            Out::default()
        }
        Row::DualTip => {
            let on = app.m2.brush.dual.is_some();
            let dual = app.m2.brush.dual.as_ref().unwrap_or(&app.m2.dual_stash);
            let name = dual
                .tip
                .as_ref()
                .map(|t| tip_display(lang, t))
                .unwrap_or_else(|| lang.t("panels-round-hardness-tip").to_owned());
            if let Some(anchor) = dropdown(ui, at, row, lang, &name, None, on && live) {
                open_popup(app, ctx, Popup::DualTip, anchor, anchor.width());
            }
            Out::default()
        }
        Row::DualMode => {
            let on = app.m2.brush.dual.is_some();
            let dual = app.m2.brush.dual.as_ref().unwrap_or(&app.m2.dual_stash);
            let mode = dual.mode;
            if let Some(anchor) = dropdown(
                ui,
                at,
                row,
                lang,
                dual_mode_label(lang, mode),
                None,
                on && live,
            ) {
                open_popup(app, ctx, Popup::DualMode, anchor, anchor.width());
            }
            Out::default()
        }
        Row::DualSize
        | Row::DualHardness
        | Row::DualAngle
        | Row::DualRoundness
        | Row::DualScatter
        | Row::DualCount
        | Row::DualSpacing => {
            dual_slider(ui, app, at, row, lang, live);
            Out::default()
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// 2-紙質の行（2-ブラシ形状が切のときは、前に使った値を薄く見せる）。
fn dual_paper_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, live, .. } = env;
    let on = app.m2.brush.dual.is_some();
    let shown = app.m2.brush.dual.as_ref().unwrap_or(&app.m2.dual_stash);
    let texture = shown.texture.clone();
    let prefs = app.m2.texture_prefs;
    // 数値と合わせ方の行は、質感が付いているときだけ触れる
    let enabled = on && texture.is_some() && live;
    let mut out = Out::default();
    let put = |app: &mut AppState, set: &dyn Fn(&mut yolu_core::PaperTexture)| {
        if let Some(t) = app.m2.brush.dual.as_mut().and_then(|d| d.texture.as_mut()) {
            set(t);
        }
    };
    match row {
        Row::DualPaperImage => {
            let name = texture
                .as_ref()
                .map(|t| tip_display(lang, &t.image))
                .unwrap_or_else(|| lang.t("panels-none").to_owned());
            let tip = Some(lang.t("panels-paper-texture-tip"));
            if let Some(anchor) = dropdown(ui, at, row, lang, &name, tip, on && live) {
                open_popup(app, ctx, Popup::DualTexture, anchor, anchor.width());
            }
        }
        Row::DualPaperDepth => {
            let depth = texture.as_ref().map_or(prefs.depth, |t| t.depth);
            let (v, o) = percent(ui, at, row, lang, depth, (0.0, 1.0), None, enabled, None);
            out = o;
            if let Some(v) = v {
                put(app, &|t| t.depth = v);
            }
        }
        Row::DualPaperScale => {
            let scale = texture.as_ref().map_or(prefs.scale, |t| t.scale);
            let format = NumberFormat {
                decimals: 2,
                trim: true,
                suffix: "",
            };
            let range = (0.05, 16.0);
            let (v, o) = slider(
                ui,
                at,
                row,
                lang,
                scale as f32,
                range,
                format,
                None,
                enabled,
                None,
            );
            out = o;
            if let Some(v) = v {
                put(app, &|t| t.scale = v as f64);
            }
        }
        Row::DualPaperAngle => {
            let angle = texture.as_ref().map_or(0.0, |t| t.angle);
            let format = NumberFormat::int("°");
            let range = (-180.0, 180.0);
            let (v, o) = slider(
                ui,
                at,
                row,
                lang,
                angle as f32,
                range,
                format,
                None,
                enabled,
                None,
            );
            out = o;
            if let Some(v) = v {
                put(app, &|t| t.angle = v as f64);
            }
        }
        Row::DualPaperMode => {
            let mode = texture.as_ref().map_or(prefs.mode, |t| t.mode);
            let label = texture_mode_label(lang, mode);
            if let Some(anchor) = dropdown(ui, at, row, lang, label, None, enabled) {
                open_popup(app, ctx, Popup::DualTextureMode, anchor, anchor.width());
            }
        }
        other => debug_assert!(false, "{other:?}: この区分の関数に描く手が無い"),
    }
    out
}

/// 消去の行。
fn erase_rows(
    ui: &mut Ui,
    app: &mut AppState,
    _ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, .. } = env;
    match row {
        Row::Erase => {
            if let Some(v) = toggle(ui, at, row, lang, app.m2.erase_mode, None, true) {
                // 押した値に決める（命令の記録に残る切り替えと同じ入口）
                if v != app.m2.erase_mode {
                    app.apply(Action::ToggleEraseMode);
                }
            }
            Out::default()
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// 水彩境界の行（色のチャンネルを色で塗るブラシだけに効く。切の間は、入り切り以外の行は触れない）。
fn water_edge_rows(
    ui: &mut Ui,
    app: &mut AppState,
    _ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, live, .. } = env;
    let reason = mix_unavailable(app, lang);
    let edge = app.m2.brush.water_edge;
    let usable = live && reason.is_none();
    let on = usable && edge.is_some();
    let shown = edge.unwrap_or_default();
    let px = NumberFormat {
        decimals: 1,
        trim: true,
        suffix: " px",
    };
    let set = |app: &mut AppState, f: &dyn Fn(&mut yolu_core::WaterEdge)| {
        if let Some(e) = app.m2.brush.water_edge.as_mut() {
            f(e);
        }
    };
    match row {
        Row::WaterEdge => {
            if let Some(v) = toggle(ui, at, row, lang, edge.is_some(), reason, usable) {
                app.m2.brush.water_edge = v.then(yolu_core::WaterEdge::default);
            }
        }
        Row::WaterEdgeWidth | Row::WaterEdgeBlur => {
            let (value, range, enabled) = if row == Row::WaterEdgeWidth {
                (
                    shown.width,
                    (
                        yolu_core::brush::MIN_WATER_EDGE_WIDTH,
                        yolu_core::brush::MAX_WATER_EDGE_WIDTH,
                    ),
                    on,
                )
            } else {
                // ぼかし幅はドラッグ後に処理のときだけ効く
                (
                    shown.blur,
                    (0.0, yolu_core::brush::MAX_WATER_EDGE_BLUR),
                    on && shown.after_stroke,
                )
            };
            let (v, out) = slider(
                ui,
                at,
                row,
                lang,
                value as f32,
                (range.0 as f32, range.1 as f32),
                px,
                reason,
                enabled,
                None,
            );
            if let Some(v) = v {
                let v = (v as f64).clamp(range.0, range.1);
                if row == Row::WaterEdgeWidth {
                    set(app, &|e| e.width = v);
                } else {
                    set(app, &|e| e.blur = v);
                }
            }
            return out;
        }
        Row::WaterEdgeOpacity | Row::WaterEdgeDarkness => {
            let value = if row == Row::WaterEdgeOpacity {
                shown.opacity
            } else {
                shown.darkness
            };
            let (v, out) = percent(ui, at, row, lang, value, (0.0, 1.0), reason, on, None);
            if let Some(v) = v {
                if row == Row::WaterEdgeOpacity {
                    set(app, &|e| e.opacity = v);
                } else {
                    set(app, &|e| e.darkness = v);
                }
            }
            return out;
        }
        Row::WaterEdgeAfterStroke => {
            if let Some(v) = toggle(ui, at, row, lang, shown.after_stroke, reason, on) {
                set(app, &|e| e.after_stroke = v);
            }
        }
        other => debug_assert!(false, "{other:?}: この区分の関数に描く手が無い"),
    }
    Out::default()
}

/// はみ出し防止の行（切の間は、入り切り以外の行は触れない。領域拡縮の量と方法は、領域拡縮が入りのときだけ）。
fn refer_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, live, .. } = env;
    let r = app.m2.brush.reference_lines;
    let on = live && r.enabled;
    match row {
        Row::ReferLines => {
            if let Some(v) = toggle(ui, at, row, lang, r.enabled, None, live) {
                app.m2.brush.reference_lines.enabled = v;
            }
        }
        Row::ReferMargin => {
            let (v, out) = slider(
                ui,
                at,
                row,
                lang,
                r.margin as f32,
                (0.0, yolu_core::MAX_REFERENCE_MARGIN as f32),
                NumberFormat {
                    decimals: 1,
                    trim: true,
                    suffix: "%",
                },
                None,
                on,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.reference_lines.margin =
                    (v as f64).clamp(0.0, yolu_core::MAX_REFERENCE_MARGIN);
            }
            return out;
        }
        Row::ReferExpand => {
            if let Some(v) = toggle(ui, at, row, lang, r.expand, None, on) {
                app.m2.brush.reference_lines.expand = v;
            }
        }
        Row::ReferAmount => {
            let max = yolu_core::MAX_REFERENCE_EXPAND as f32;
            let (v, out) = slider(
                ui,
                at,
                row,
                lang,
                r.amount as f32,
                (-max, max),
                NumberFormat::int(" px"),
                None,
                on && r.expand,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.reference_lines.amount =
                    (v.round() as f64).clamp(-max as f64, max as f64);
            }
            return out;
        }
        Row::ReferMode => {
            let label = m2::expand_modes(lang)
                .into_iter()
                .find(|(m, _)| *m == r.mode)
                .map_or("", |(_, l)| l);
            if let Some(anchor) = dropdown(ui, at, row, lang, label, None, on && r.expand) {
                open_popup(app, ctx, Popup::ReferMode, anchor, anchor.width());
            }
        }
        other => debug_assert!(false, "{other:?}: この区分の関数に描く手が無い"),
    }
    Out::default()
}

/// 補正の行。
fn stabilization_rows(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, live, .. } = env;
    match row {
        Row::Stabilizer => {
            let (v, out) = slider(
                ui,
                at,
                row,
                lang,
                app.m2.brush.assist.stabilizer as f32,
                (0.0, 200.0),
                NumberFormat::int(" px"),
                None,
                live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.assist.stabilizer = v as f64;
            }
            out
        }
        Row::StabilizerBySpeed => {
            let on = app.m2.brush.assist.stabilizer_speed.is_some();
            if let Some(v) = toggle(ui, at, row, lang, on, None, live) {
                let kind = m2::stabilizer_speed_type(&app.m2.brush.assist);
                app.m2.brush.assist.stabilizer_speed = v.then_some(kind);
            }
            Out::default()
        }
        Row::StabilizerSpeedType => {
            let a = &app.m2.brush.assist;
            let kind = m2::stabilizer_speed_type(a);
            let label = m2::stabilizer_speed_types(lang)
                .into_iter()
                .find(|(k, _)| *k == kind)
                .map_or("", |(_, l)| l);
            let enabled = live && a.stabilizer_speed.is_some();
            if let Some(anchor) = dropdown(ui, at, row, lang, label, None, enabled) {
                open_popup(app, ctx, Popup::StabilizerSpeedType, anchor, anchor.width());
            }
            Out::default()
        }
        Row::Sweep => {
            let (v, out) = percent(
                ui,
                at,
                row,
                lang,
                app.m2.brush.assist.sweep,
                (0.0, 1.0),
                None,
                live,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.assist.sweep = v;
            }
            out
        }
        Row::PostCorrection => {
            let on = app.m2.brush.assist.post.on;
            if let Some(v) = toggle(ui, at, row, lang, on, None, live) {
                app.m2.brush.assist.post.on = v;
            }
            Out::default()
        }
        Row::PostStrength => {
            let post = app.m2.brush.assist.post;
            let (v, out) = slider(
                ui,
                at,
                row,
                lang,
                post.strength as f32,
                (0.0, 100.0),
                NumberFormat::int(""),
                None,
                live && post.on,
                None,
            );
            if let Some(v) = v {
                app.m2.brush.assist.post.strength = v.round().clamp(0.0, 100.0) as f64;
            }
            out
        }
        Row::PostBySpeed | Row::PostByViewScale => {
            let post = &mut app.m2.brush.assist.post;
            let enabled = live && post.on;
            let field = if row == Row::PostBySpeed {
                &mut post.by_speed
            } else {
                &mut post.by_view_scale
            };
            if let Some(v) = toggle(ui, at, row, lang, *field, None, enabled) {
                *field = v;
            }
            Out::default()
        }
        Row::Curve => {
            if let Some(v) = toggle(ui, at, row, lang, app.m2.brush.assist.curve, None, live) {
                app.m2.brush.assist.curve = v;
            }
            Out::default()
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// 入り抜きの行。
fn taper_rows(
    ui: &mut Ui,
    app: &mut AppState,
    _ctx: &egui::Context,
    row: Row,
    at: &mut At,
    env: Env,
) -> Out {
    let Env { lang, live, .. } = env;
    match row {
        Row::TaperIn | Row::TaperOut => {
            let a = &mut app.m2.brush.assist;
            let field = if row == Row::TaperIn {
                &mut a.taper_in
            } else {
                &mut a.taper_out
            };
            let (v, out) = slider(
                ui,
                at,
                row,
                lang,
                *field as f32,
                (0.0, 500.0),
                NumberFormat::int(" px"),
                None,
                live,
                None,
            );
            if let Some(v) = v {
                *field = v as f64;
            }
            out
        }
        Row::FadeSize | Row::FadeOpacity | Row::FadeFlow => {
            let k = &mut app.m2.brush.controls;
            let field = match row {
                Row::FadeSize => &mut k.fade_size,
                Row::FadeOpacity => &mut k.fade_opacity,
                _ => &mut k.fade_flow,
            };
            let (v, out) = slider(
                ui,
                at,
                row,
                lang,
                *field as f32,
                (0.0, 2000.0),
                NumberFormat::int(""),
                None,
                live,
                None,
            );
            if let Some(v) = v {
                *field = v.round().max(0.0) as u32;
            }
            out
        }
        other => {
            debug_assert!(false, "{other:?}: この区分の関数に描く手が無い");
            Out::default()
        }
    }
}

/// 詳細の欄の行 1 つ: 左端に目のボタン（ツールプロパティに出すか）と、行。
pub fn detail_row(ui: &mut Ui, app: &mut AppState, ctx: &egui::Context, row: Row, rows: &mut Rows) {
    let lang = app.lang;
    let r = rows.row(ROW, GAP);
    let shown = rows::is_shown(&app.m2.brush, row);
    let eye = Rect::from_min_size(r.min, vec2(EYE, r.height()));
    if w::icon_button(
        ui,
        eye,
        ("brush.eye", row.id()),
        if shown {
            "visibility"
        } else {
            "visibility_off"
        },
        lang.t("brushrows-show-in-tool-properties"),
        shown,
        true,
        16.0,
    )
    .clicked()
    {
        rows::set_shown(&mut app.m2.brush, row, !shown);
    }
    let body = Rect::from_min_max(pos2(eye.right() + EYE_GAP, r.top()), r.max);
    show(ui, app, ctx, row, &mut At::Detail(body));
}

// ───────── 行ごとの部品 ─────────

/// 2D のクローンのずれ（X・Y）。3D の面だけを出しているときは面の点で決まり、元から決め直すときは欄では決まらない。
fn clone_offset(ui: &mut Ui, app: &mut AppState, at: &mut At, row: Row, lang: Lang, live: bool) {
    let BrushEffect::Clone { offset } = app.m2.brush.effect else {
        return;
    };
    let only_3d = app.paints_only_in_3d();
    let resets_offset = app.clone.canvas_resets_offset(app.session.doc.id());
    let reason = if app.tool == Tool::Eraser {
        Some(lang.t("panels-not-available-with-the-eraser"))
    } else if only_3d {
        Some(lang.t("panels-on-3d-surfaces-the-source"))
    } else if resets_offset {
        Some(lang.t("panels-without-aligned-every-stroke-starts"))
    } else {
        None
    };
    let value = if row == Row::EffectCloneX {
        offset.x
    } else {
        offset.y
    };
    let (v, _) = slider(
        ui,
        at,
        row,
        lang,
        value as f32,
        (-2048.0, 2048.0),
        NumberFormat::int(" px"),
        reason,
        reason.is_none() && live,
        None,
    );
    if let Some(v) = v {
        let mut next = offset;
        if row == Row::EffectCloneX {
            next.x = v as f64;
        } else {
            next.y = v as f64;
        }
        app.m2.brush.effect = BrushEffect::Clone { offset: next };
        // 欄で直した offset は、次のストロークから使う（元からは決め直さない）。ブラシの設定を直したので「変えた」に数える
        app.clone.offset_edited(next);
    }
}

/// クローンの「揃える」（元があるときだけ押せる。元が無いときの理由は、元を決める入力をツールチップに添える）。
fn clone_aligned(ui: &mut Ui, app: &mut AppState, at: &mut At, row: Row, lang: Lang, live: bool) {
    let eraser = (app.tool == Tool::Eraser).then(|| lang.t("panels-not-available-with-the-eraser"));
    let has_source = app.clone.canvas_source_for(app.session.doc.id()).is_some()
        || app
            .view3d
            .model
            .as_ref()
            .is_some_and(|m| app.clone.source_for(&m.geometry).is_some());
    let tip = match eraser {
        Some(reason) => Some(reason.to_owned()),
        None if !has_source => Some(crate::shortcuts::named_with_keys(
            lang,
            lang.t("panels-available-once-a-source-is-tip"),
            &[clone_source_tip_key(lang)],
        )),
        None => None,
    };
    let enabled = eraser.is_none() && has_source && live;
    if let Some(v) = toggle(
        ui,
        at,
        row,
        lang,
        app.clone.aligned,
        tip.as_deref(),
        enabled,
    ) {
        app.clone.set_aligned(v);
    }
}

/// クローンの元を決める入力の文字（組み合わせの表の「動かさずに離す」の行から。「Alt+クリック」。外していれば None）。
fn clone_source_tip_key(lang: Lang) -> Option<String> {
    let keymap = crate::keymap::current();
    let g = keymap
        .gestures()
        .iter()
        .find(|g| g.operation == crate::keymap::Operation::CloneSource && g.click)?;
    let mac = cfg!(target_os = "macos");
    let mut parts = Vec::new();
    if g.ctrl {
        parts.push(if mac { "Cmd" } else { "Ctrl" });
    }
    if g.alt {
        parts.push("Alt");
    }
    if g.shift {
        parts.push("Shift");
    }
    parts.push(lang.t("panels-click-tip"));
    Some(parts.join("+"))
}

/// 背景色（カラーのパネルのサブの色）の行: 色の見本と名前。
fn background_swatch(ui: &mut Ui, app: &mut AppState, at: &mut At, row: Row, lang: Lang) {
    let r = match at {
        At::Detail(body) => *body,
        At::Tool(rows) => rows.row(t::ROW_HEIGHT, 2.0),
    };
    let swatch = Rect::from_min_size(r.min + vec2(0.0, 2.0), vec2(30.0, r.height() - 4.0));
    w::color_swatch(
        ui,
        swatch,
        row_id(at, row, "swatch"),
        app.color.sub,
        lang.t("panels-background-tip"),
        true,
    );
    w::text(
        ui.painter(),
        Rect::from_min_max(pos2(swatch.right() + 8.0, r.top()), r.max),
        row.name(lang),
        t::LABEL,
        Align::Left,
    );
}

/// 3D の塗りが、カメラから投影するアライメントか（隠れた所・面の向きの弱めの行が効くか）。
fn camera_aligned(app: &AppState) -> bool {
    app.view3d.projection.alignment == yolu_core::geometry::SurfaceAlignment::Camera
}

/// 3D の塗りの切り替えのスライダー（面の向きで弱める角度と、継ぎ目のにじみ）。ドラッグしている間は設定のファイルに書かない。
fn projection_slider(
    ui: &mut Ui,
    app: &mut AppState,
    at: &mut At,
    row: Row,
    lang: Lang,
    free: bool,
) {
    let p = app.view3d.projection;
    let (value, range) = match row {
        Row::ProjectionAngleStart => (p.angle_start, (0.0, 90.0)),
        Row::ProjectionAngleEnd => (p.angle_end, (0.0, 90.0)),
        _ => (
            p.seam_bleed as f32,
            (0.0, yolu_core::geometry::MAX_SEAM_BLEED as f32),
        ),
    };
    let label = row.name(lang);
    let format = if row == Row::ProjectionBleed {
        NumberFormat::int(" px")
    } else {
        NumberFormat::int("°")
    };
    // この行だけ、ドラッグの最中を知る必要があるので、部品を直に使う
    let spec = SliderSpec::new(label, range.0, range.1, format).enabled(free);
    let id = row_id(at, row, "slider");
    let (result, name_label) = match at {
        At::Detail(body) => {
            let body = *body;
            let name = Rect::from_min_size(body.min, vec2(NAME_WIDTH, body.height()));
            detail_name(ui, name, label, free, None);
            let control = Rect::from_min_max(
                pos2(name.right() + 4.0, body.top() + 1.0),
                pos2(body.right(), body.bottom() - 1.0),
            );
            let inside = SliderSpec {
                label: "",
                access_name: Some(label),
                ..spec
            };
            (w::slider(ui, control, id, value, &inside), label)
        }
        At::Tool(rows) => {
            let r = rows.slider_row();
            (w::slider(ui, r, id, value, &spec), label)
        }
    };
    let _ = name_label;
    if result.active {
        app.view3d.projection_dragging = true;
    }
    if result.changed {
        let p = &mut app.view3d.projection;
        match row {
            Row::ProjectionAngleStart => {
                p.angle_start = result.value.round();
                p.angle_end = p.angle_end.max(p.angle_start);
            }
            Row::ProjectionAngleEnd => {
                p.angle_end = result.value.round();
                p.angle_start = p.angle_start.min(p.angle_end);
            }
            _ => {
                p.seam_bleed = result
                    .value
                    .round()
                    .clamp(0.0, yolu_core::geometry::MAX_SEAM_BLEED as f32)
                    as u32;
            }
        }
    }
}

/// 2-ブラシ形状の数値の行（2-ブラシ形状が切のときは、前に使った値を薄く見せる）。
fn dual_slider(ui: &mut Ui, app: &mut AppState, at: &mut At, row: Row, lang: Lang, live: bool) {
    let d = app
        .m2
        .brush
        .dual
        .clone()
        .unwrap_or_else(|| app.m2.dual_stash.clone());
    // 値・範囲・単位（% の行は None）
    let (value, range, suffix): (f64, (f64, f64), Option<&str>) = match row {
        Row::DualSize => (d.radius * 2.0, (1.0, 256.0), Some(" px")),
        Row::DualHardness => (d.hardness, (0.0, 1.0), None),
        Row::DualAngle => (d.angle, (-180.0, 180.0), Some("°")),
        Row::DualRoundness => (d.roundness, (0.01, 1.0), None),
        Row::DualScatter => (d.scatter, (0.0, 10.0), None),
        Row::DualCount => (d.count as f64, (1.0, 16.0), Some("")),
        _ => (d.spacing, (0.01, 4.0), None),
    };
    // 硬さは丸い筆先の縁なので、画像の筆先では効かない
    let image = row == Row::DualHardness && d.tip.is_some();
    let reason = image.then(|| lang.t("panels-no-effect-on-an-image-tip"));
    let enabled = app.m2.brush.dual.is_some() && live && !image;
    let changed = match suffix {
        None => percent(ui, at, row, lang, value, range, reason, enabled, None).0,
        Some(suffix) => {
            let range = (range.0 as f32, range.1 as f32);
            let format = NumberFormat::int(suffix);
            let slider = slider(
                ui,
                at,
                row,
                lang,
                value as f32,
                range,
                format,
                reason,
                enabled,
                None,
            );
            slider.0.map(f64::from)
        }
    };
    let (Some(v), Some(d)) = (changed, app.m2.brush.dual.as_mut()) else {
        return;
    };
    match row {
        Row::DualSize => d.radius = (v / 2.0).max(0.5),
        Row::DualHardness => d.hardness = v,
        Row::DualAngle => d.angle = v,
        Row::DualRoundness => d.roundness = v,
        Row::DualScatter => d.scatter = v,
        Row::DualCount => d.count = v.round().clamp(1.0, 16.0) as u32,
        _ => d.spacing = v,
    }
}

// ───────── 影響元設定のウィンドウ ─────────

/// 影響元設定のウィンドウの名前（`windows::window_rect` と同じ形の Id。テストがウィンドウの矩形を引く）。
pub fn dynamics_id() -> Id {
    Id::new(("yolu.window", "brush-dynamics"))
}

const DYNAMICS_SIZE: Vec2 = vec2(480.0, 350.0);

/// 入力の名前。
fn input_name(lang: Lang, input: Input) -> &'static str {
    match input {
        Input::Pressure => lang.t("brushrows-pressure"),
        Input::Tilt => lang.t("brushrows-tilt"),
        Input::Speed => lang.t("brushrows-speed"),
        Input::Random => lang.t("brushrows-random"),
    }
}

/// 「◯◯影響元設定」のウィンドウ（開いていなければ何もしない）。上に入力ごとの行（使うか・最小値・傾きは最大値も）、下に筆圧・傾き・速度の曲線を
/// 横に 3 つ。値は項目の影響元を直に変える。
pub fn dynamics_window(ctx: &egui::Context, app: &mut AppState) {
    let Some(row) = app.brushes.ui.detail.dynamics else {
        return;
    };
    let Some(dynamics) = row.dynamics() else {
        app.brushes.ui.detail.dynamics = None;
        return;
    };
    let lang = app.lang;
    let name = row.name(lang);
    let title = lang.f("brushrows-dynamics-title", &[("name", &name)]);
    let spec = Spec {
        title: &title,
        icon: Some("stylus"),
        size: DYNAMICS_SIZE,
        modal: false,
        close_label: lang.t("panels-close"),
    };
    // 初めて開くときは、詳細のウィンドウの右下寄りに置く（キャンバスの真ん中を空ける。見出しのドラッグで動かせる）
    if !app.brushes.ui.detail.dynamics_placed {
        app.brushes.ui.detail.dynamics_placed = true;
        app.ui.brushes_dynamics_offset = vec2(150.0, 110.0);
    }
    let mut offset = app.ui.brushes_dynamics_offset;
    let closed = window::show(
        ctx,
        dynamics_id(),
        &spec,
        &mut offset,
        false,
        |ui, frame| {
            dynamics_body(ui, app, row, dynamics, frame.body);
        },
    );
    app.ui.brushes_dynamics_offset = offset;
    if closed {
        app.brushes.ui.detail.dynamics = None;
    }
}

fn dynamics_body(ui: &mut Ui, app: &mut AppState, row: Row, dynamics: Dynamics, body: Rect) {
    let lang = app.lang;
    let area = body.shrink(10.0);
    let heading = lang.f("brushrows-dynamics-inputs", &[("name", &row.name(lang))]);
    w::text(
        ui.painter(),
        Rect::from_min_size(area.min, vec2(area.width(), 18.0)),
        &heading,
        t::LABEL_DIM,
        Align::Left,
    );
    // 硬さの影響元は、画像の筆先では効かないので触れない（入っていても）
    let usable = dynamics_usable(app, row);
    let mut influence = influence_of(app, dynamics);
    let before = influence.clone();
    let y = dynamics_inputs(
        ui,
        app,
        row,
        dynamics,
        area,
        area.top() + 24.0,
        &mut influence,
    );
    dynamics_curves(ui, lang, area, y + 6.0, &mut influence, usable);
    if influence != before {
        set_influence(app, dynamics, influence);
    }
}

/// 影響元設定のウィンドウの入力ごとの行（使うか・最小値。傾きは最大値の行もある）。次の行の上端を返す。
fn dynamics_inputs(
    ui: &mut Ui,
    app: &AppState,
    row: Row,
    dynamics: Dynamics,
    area: Rect,
    mut y: f32,
    influence: &mut Influence,
) -> f32 {
    let lang = app.lang;
    let usable = dynamics_usable(app, row);
    let reason = (!usable).then(|| lang.t("panels-no-effect-on-an-image"));
    let (lo, hi) = if dynamics.signed() {
        (-100.0, 100.0)
    } else {
        (0.0, 100.0)
    };
    for input in Input::ALL {
        let line = Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), 22.0));
        let left = Rect::from_min_size(line.min, vec2(104.0, 22.0));
        let id = ("brush.dynamics.input", input.id());
        let use_ = influence.input_mut(input);
        let on = w::toggle(
            ui,
            left,
            id,
            input_name(lang, input),
            use_.on,
            reason,
            usable,
        );
        if on != use_.on {
            use_.on = on;
            // 符号つきの項目のランダムは、はじめて入れるとき ±値（最小値 −100%）にする
            if on && input == Input::Random && dynamics.signed() && use_.response.is_identity() {
                if let Ok(next) = use_.response.with_min_signed(-1.0) {
                    use_.response = next;
                }
            }
        }
        let track = Rect::from_min_max(pos2(left.right() + 6.0, y), line.max);
        let spec = SliderSpec::new(lang.t("brushrows-minimum"), lo, hi, NumberFormat::int("%"))
            .enabled(on && usable)
            .tooltip_reason(reason);
        let out = w::slider(
            ui,
            track,
            ("brush.dynamics.min", input.id()),
            (use_.response.min() * 100.0) as f32,
            &spec,
        );
        if out.changed {
            let v = (out.value as f64 / 100.0).clamp(lo as f64 / 100.0, 1.0);
            let next = if dynamics.signed() {
                use_.response.with_min_signed(v)
            } else {
                use_.response.with_min(v)
            };
            if let Ok(next) = next {
                use_.response = next;
            }
        }
        y += 26.0;
        if input == Input::Tilt {
            // 傾きだけ最大値もある（最小値と最大値の間に写る）
            let track =
                Rect::from_min_max(pos2(left.right() + 6.0, y), pos2(area.right(), y + 22.0));
            tilt_max_row(ui, lang, track, use_, on && usable);
            y += 26.0;
        }
    }
    y
}

/// 傾きの最大値の行。
fn tilt_max_row(ui: &mut Ui, lang: Lang, track: Rect, use_: &mut InputUse, enabled: bool) {
    let spec = SliderSpec::new(
        lang.t("brushrows-maximum"),
        0.0,
        100.0,
        NumberFormat::int("%"),
    )
    .enabled(enabled);
    let out = w::slider(
        ui,
        track,
        ("brush.dynamics.max", Input::Tilt.id()),
        (use_.response.max() * 100.0) as f32,
        &spec,
    );
    if out.changed {
        if let Ok(next) = use_
            .response
            .with_max((out.value as f64 / 100.0).clamp(0.0, 1.0))
        {
            use_.response = next;
        }
    }
}

/// 影響元設定のウィンドウの下の、筆圧・傾き・速度の曲線（横に 3 つ。名前の行の下が曲線の枠）。
fn dynamics_curves(
    ui: &mut Ui,
    lang: Lang,
    area: Rect,
    y: f32,
    influence: &mut Influence,
    usable: bool,
) {
    let gap = 10.0;
    let column = (area.width() - 2.0 * gap) / 3.0;
    let captions = [
        (
            Input::Pressure,
            lang.t("brushrows-pressure-curve"),
            lang.t("brushrows-pressure-curve-tip"),
        ),
        (
            Input::Tilt,
            lang.t("brushrows-tilt-curve"),
            lang.t("brushrows-tilt-curve-tip"),
        ),
        (
            Input::Speed,
            lang.t("brushrows-speed-curve"),
            lang.t("brushrows-speed-curve-tip"),
        ),
    ];
    for (n, (input, caption, tip)) in captions.into_iter().enumerate() {
        let x = area.left() + n as f32 * (column + gap);
        w::text(
            ui.painter(),
            Rect::from_min_size(pos2(x, y), vec2(column, 18.0)),
            caption,
            t::LABEL_DIM,
            Align::Center,
        );
        let h = (area.bottom() - y - 20.0).clamp(40.0, curve::HEIGHT);
        let r = Rect::from_min_size(pos2(x, y + 20.0), vec2(column, h));
        let use_ = influence.input_mut(input);
        let shape: Curve = use_.response.curve_shape();
        if let Some(next) = curve::curve_editor(
            ui,
            r,
            ("brush.dynamics.curve", input.id()),
            &shape,
            tip,
            use_.on && usable,
        ) {
            if let Ok(next) = use_.response.with_curve_shape(next) {
                use_.response = next;
            }
        }
    }
}

/// 影響元設定のウィンドウの本体 `body`（見出しの帯の下）で、`n` 番目（0 筆圧・1 傾き・2 速度）の曲線の枠（テストが曲線の点を押す位置を決めるのに使う）。
pub fn dynamics_curve_frame(body: Rect, n: usize) -> Rect {
    let area = body.shrink(10.0);
    // 見出しの文・入力の 4 行（傾きは最大値の行がもう 1 つ）・曲線の名前の行
    let y = area.top() + 24.0 + 4.0 * 26.0 + 26.0 + 6.0 + 20.0;
    let gap = 10.0;
    let column = (area.width() - 2.0 * gap) / 3.0;
    let h = (area.bottom() - (y - 20.0) - 20.0).clamp(40.0, curve::HEIGHT);
    Rect::from_min_size(
        pos2(area.left() + n as f32 * (column + gap), y),
        vec2(column, h),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全部の行が、区分の関数のどれかに描く手を持つ（足し忘れは、描く関数の `debug_assert!` が落とす）。
    #[test]
    fn every_row_has_a_drawing_arm_in_its_category_function() {
        let ctx = egui::Context::default();
        let mut app = AppState::new_in(64, 64, Lang::Ja);
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let ctx = ui.ctx().clone();
            for row in Row::ALL {
                let body = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, ROW));
                show(ui, &mut app, &ctx, row, &mut At::Detail(body));
            }
        });
        output.drop_without_applying_deltas();
    }
}
