//! 選んでいる効果の行の設定（プロパティの欄。Unity 版の `DrawFilterParameters` / `DrawGeneratorParameters` / `AnchorRows`）。
//! フィルターは値と強さ、Generator は読むマップとその状態・効かない理由・値・範囲・崩し・合成、Anchor の行は名前と読んでいる段。
//! 値は `Action::Fx` を通る（1 回の Undo。スライダーのドラッグは離したところで区切る）。画面には名前と値と短い状態だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Sense, Ui};
use yolu_core::filter::Settings as Filter;
use yolu_core::generator::{self, Kind, MapKind, MapState, Shape};
use yolu_core::mesh_maps::MeshMapState;
use yolu_core::{
    AnchorId, AnchorPlacement, EffectSettings, FilterEffect, FilterTarget, InactiveReason, LayerId,
};

use super::properties::{
    choice_row, group_label, open_popup, percent_row, section, slider_row, status_row, toggle_row,
};
use crate::bake::{kind_label, state_label, BakeAction};
use crate::fx::menu::{anchor_choice_name, FxChoice};
use crate::fx::{inputs, names, FxOp};
use crate::lang::Lang;
use crate::m2_menu::Popup;
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows};

fn decimals(places: u8) -> NumberFormat<'static> {
    NumberFormat {
        decimals: places,
        trim: true,
        suffix: "",
    }
}

/// 効果の欄の全部（選んでいる行が段か Anchor かで）。
pub fn effect_body(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context) {
    if let Some((layer, effect, target)) = app
        .fx
        .filter(&app.session.doc)
        .map(|(l, e, t)| (l, e.clone(), t))
    {
        filter_body(ui, app, rows, ctx, layer, &effect, target);
    } else if let Some(info) = app
        .fx
        .anchor(&app.session.doc)
        .map(|i| (i.anchor.id(), i.anchor.name().to_owned()))
    {
        anchor_body(ui, app, rows, info.0, &info.1);
    }
}

// ───────── 名前と数の入力 ─────────

/// 名前と文字の入力欄の 1 行。決めた文字を返す（Enter か外を押したとき）。
fn text_row(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    label: &str,
    shown: &str,
    tooltip: Option<&str>,
) -> Option<String> {
    let r = rows.row(t::ROW_HEIGHT, 4.0);
    let lw = (r.width() * 0.42).floor();
    w::text(
        ui.painter(),
        Rect::from_min_size(r.min, vec2(lw, r.height())),
        label,
        t::LABEL,
        w::Align::Left,
    );
    let field = Rect::from_min_max(pos2(r.left() + lw, r.top()), r.max);
    w::text_field(ui, field, id, shown, tooltip, false).committed
}

/// 整数の入力欄の 1 行（決めた値）。
fn int_row(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    label: &str,
    value: i32,
    tooltip: Option<&str>,
) -> Option<i32> {
    text_row(ui, rows, id, label, &value.to_string(), tooltip)
        .and_then(|s| s.trim().parse::<i32>().ok())
        .filter(|v| *v != value)
}

/// 3 つの数の入力欄（見出しの下に 1 行）。決めた値を返す。
fn vec3_row(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    label: &str,
    value: [f64; 3],
    tooltip: Option<&str>,
) -> Option<[f64; 3]> {
    group_label(ui, rows, label);
    let r = rows.row(t::ROW_HEIGHT, 4.0);
    let mut next = None;
    for (i, cell) in Rows::split(r, 3, 4.0).into_iter().enumerate() {
        let shown = names::trim(value[i], 3);
        if let Some(text) = w::text_field(ui, cell, (id, i), &shown, tooltip, false).committed {
            if let Ok(v) = text.trim().parse::<f64>() {
                if v.is_finite() && v != value[i] {
                    let mut n = value;
                    n[i] = v;
                    next = Some(n);
                }
            }
        }
    }
    next
}

fn set_settings(
    app: &mut AppState,
    layer: LayerId,
    id: yolu_core::FilterId,
    settings: EffectSettings,
    coalesce: bool,
) {
    app.apply(Action::Fx(FxOp::SetSettings {
        layer,
        id,
        settings,
        coalesce,
    }));
    if !coalesce {
        // 1 回の操作で決まる変更は、続く変更とまとめない
        app.m2_end_drag();
    }
}

// ───────── フィルター ─────────

fn filter_body(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    ctx: &egui::Context,
    layer: LayerId,
    effect: &FilterEffect,
    target: FilterTarget,
) {
    let lang = app.lang;
    let enabled = app.can_edit();
    let id = effect.id();
    let name = names::effect_name(lang, effect.settings());
    let title = match target {
        FilterTarget::Mask => lang.f("panels-mask", &[("name", &name)]),
        FilterTarget::Content => name.to_owned(),
    };
    let (open, _) = section(
        ui,
        app,
        rows,
        "effect",
        &title,
        names::effect_icon(effect.settings()),
        None,
    );
    if !open {
        return;
    }
    let mut next: Option<EffectSettings> = None;
    // 1 回の操作で決まる変更（曲線・分岐点・切り替え）は、まとめずに独立の 1 回の取り消しにする
    let mut discrete = false;
    match effect.settings() {
        EffectSettings::Filter(Filter::GaussianBlur { radius }) => {
            if let Some(v) = slider_row(
                ui,
                rows,
                "fx.radius",
                lang.t("panels-radius"),
                *radius as f32,
                (1.0, 256.0),
                NumberFormat::int(" px"),
                Some(lang.t("panels-reach-in-pixels-standard-deviation-tip")),
                enabled,
            ) {
                next = Some(EffectSettings::blur(v.round().clamp(1.0, 256.0) as u32));
            }
        }
        EffectSettings::Filter(Filter::Sharpen {
            radius,
            amount,
            threshold,
        }) => {
            let (mut r, mut a, mut th) = (*radius, *amount, *threshold);
            let mut changed = false;
            if let Some(v) = slider_row(
                ui,
                rows,
                "fx.radius",
                lang.t("panels-radius"),
                r as f32,
                (1.0, 64.0),
                NumberFormat::int(" px"),
                None,
                enabled,
            ) {
                r = v.round().clamp(1.0, 64.0) as u32;
                changed = true;
            }
            if let Some(v) = slider_row(
                ui,
                rows,
                "fx.amount",
                lang.t("panels-amount"),
                a as f32,
                (0.0, 5.0),
                decimals(2),
                Some(lang.t("panels-how-much-the-edges-are-tip")),
                enabled,
            ) {
                a = v as f64;
                changed = true;
            }
            if let Some(v) = slider_row(
                ui,
                rows,
                "fx.threshold",
                lang.t("panels-threshold"),
                th as f32,
                (0.0, 255.0),
                NumberFormat::int(""),
                Some(lang.t("panels-differences-smaller-than-this-0-tip")),
                enabled,
            ) {
                th = v.round().clamp(0.0, 255.0) as u32;
                changed = true;
            }
            if changed {
                next = Some(EffectSettings::sharpen(r, a, th));
            }
        }
        EffectSettings::Filter(Filter::Noise {
            amount,
            seed,
            monochrome,
        }) => {
            let (mut a, mut s) = (*amount, *seed);
            let mut changed = false;
            if let Some(v) = percent_row(
                ui,
                rows,
                "fx.amount",
                lang.t("panels-amount"),
                a,
                (0.0, 1.0),
                None,
                enabled,
            ) {
                a = v;
                changed = true;
            }
            if let Some(v) = int_row(
                ui,
                rows,
                "fx.seed",
                lang.t("panels-seed"),
                s,
                Some(lang.t("panels-the-same-seed-gives-the-tip")),
            ) {
                s = v;
                changed = true;
            }
            if changed {
                next = Some(EffectSettings::noise(a, s, *monochrome));
            }
        }
        EffectSettings::Filter(Filter::Levels {
            input_black,
            input_white,
            gamma,
            output_black,
            output_white,
        }) => {
            let (mut ib, mut iw, mut gm, mut ob, mut ow) = (
                *input_black as f32,
                *input_white as f32,
                *gamma as f32,
                *output_black as f32,
                *output_white as f32,
            );
            let mut changed = false;
            let range = (0.0, 1.0);
            let gap = 1.0 / 255.0 + 1e-4;
            group_label(ui, rows, lang.t("panels-input"));
            if let Some(v) = slider_row(
                ui,
                rows,
                "fx.ib",
                lang.t("panels-black"),
                ib,
                range,
                decimals(3),
                None,
                enabled,
            ) {
                ib = v.min(iw - gap).max(0.0);
                changed = true;
            }
            if let Some(v) = slider_row(
                ui,
                rows,
                "fx.iw",
                lang.t("panels-white"),
                iw,
                range,
                decimals(3),
                None,
                enabled,
            ) {
                iw = v.max(ib + gap).min(1.0);
                changed = true;
            }
            if let Some(v) = slider_row(
                ui,
                rows,
                "fx.gamma",
                lang.t("panels-gamma"),
                gm,
                (0.1, 9.99),
                decimals(2),
                None,
                enabled,
            ) {
                gm = v;
                changed = true;
            }
            group_label(ui, rows, lang.t("panels-output"));
            if let Some(v) = slider_row(
                ui,
                rows,
                "fx.ob",
                lang.t("panels-black"),
                ob,
                range,
                decimals(3),
                None,
                enabled,
            ) {
                ob = v;
                changed = true;
            }
            if let Some(v) = slider_row(
                ui,
                rows,
                "fx.ow",
                lang.t("panels-white"),
                ow,
                range,
                decimals(3),
                None,
                enabled,
            ) {
                ow = v;
                changed = true;
            }
            if changed {
                next = Some(EffectSettings::levels(
                    ib as f64, iw as f64, gm as f64, ob as f64, ow as f64,
                ));
            }
        }
        EffectSettings::Filter(f)
            if names::FilterKind::of(f)
                .and_then(|k| k.catalog_id())
                .is_some() =>
        {
            next = catalog_rows(ui, app, rows, effect.settings(), &[]);
            // 選択肢のボタンは 1 回で決まる変更
            discrete = next
                .as_ref()
                .is_some_and(|n| choices_differ(n, effect.settings()));
        }
        EffectSettings::Filter(_) => {
            if let Some(value) = effect.settings().color_adjust() {
                let mut failure = None;
                let mut params = super::color_adjust::Params {
                    key: ("effect", id.0),
                    enabled,
                    why: None,
                    paint: app.color.main,
                    sub: app.color.sub,
                    lang,
                    histogram: None,
                    sets: &mut app.ramp_sets,
                    eyedrop: &mut app.eyedrop,
                    failure: &mut failure,
                };
                if let Some(change) = super::color_adjust::rows(ui, rows, &mut params, &value) {
                    discrete = change.discrete;
                    next = Some(EffectSettings::from_color_adjust(change.value));
                }
                if let Some(text) = failure {
                    app.fail(crate::notice::Source::Gradient, text);
                }
            }
        }
        EffectSettings::Generator(g) => {
            if let Some(g) = generator_rows(ui, app, rows, ctx, layer, effect, g, target) {
                next = Some(EffectSettings::generator(g));
            }
        }
    }
    if let Some(settings) = next {
        if settings != *effect.settings() {
            set_settings(app, layer, id, settings, !discrete);
        }
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "fx.strength",
        lang.t("panels-strength"),
        effect.strength(),
        (0.0, 1.0),
        Some(lang.t("panels-how-much-of-the-result-tip")),
        enabled,
    ) {
        app.apply(Action::Fx(FxOp::SetStrength {
            layer,
            id,
            strength: v,
            coalesce: true,
        }));
    }
    // 近傍の段は、モデルがあれば UV の継ぎ目をまたげる（文書の設定。どの段の欄からも同じ値）
    if effect.settings().halo() > 0 && app.session.doc.effect_inputs().topology().is_some() {
        if let Some(on) = toggle_row(
            ui,
            rows,
            "fx.seams",
            lang.t("panels-across-uv-seams"),
            app.session.doc.filter_seams(),
            Some(lang.t("panels-a-setting-for-every-filter-tip")),
            enabled,
        ) {
            app.apply(Action::Fx(FxOp::SetFilterSeams(on)));
        }
    }
    // 描くチャンネルに掛からない画素の段は短く知らせる
    let paint = app.m2.paint_channel;
    if target == FilterTarget::Content && !effect.applies_to(paint) {
        let channel = crate::m2::channel_name(lang, &app.session.doc, paint);
        status_row(
            ui,
            rows,
            &lang.f("panels-not-applied-to", &[("channel", &channel)]),
        );
    }
    rows.space(4.0);
}

/// 0.5.0 のフィルターの値の欄。効果の目録の欄の型と範囲から並べる（整数・実数はスライダー、シードは数の入力、選択肢はボタン）。
/// 変えた値の設定を返す。
fn catalog_rows(
    ui: &mut Ui,
    app: &AppState,
    rows: &mut Rows,
    settings: &EffectSettings,
    skip: &[&str],
) -> Option<EffectSettings> {
    use yolu_core::effects::catalog::{self, ParamType, ParamValue};
    let lang = app.lang;
    let enabled = app.can_edit();
    let kind = catalog::kind(settings.kind_id())?;
    let mut changed: std::collections::BTreeMap<String, ParamValue> = Default::default();
    for (name, value) in settings.catalog_values() {
        let Some(param) = kind.param(name) else {
            continue;
        };
        if skip.contains(&name) {
            continue;
        }
        if let Some(heading) = names::param_group(lang, kind.id, name) {
            group_label(ui, rows, heading);
        }
        let label = names::param_label(lang, kind.id, name);
        let hint = names::param_hint(lang, kind.id, name);
        let key = format!("fx.{name}");
        // 画素で数える欄は px、角度は °（Generator の欄は画素で数えない）
        let suffix = match name {
            "angle" | "azimuth" | "elevation" => "°",
            _ if kind.generator => "",
            "intensity" if kind.id == "glow" => "",
            "intensity" | "scale" | "distance" | "radius" | "width" => " px",
            _ => "",
        };
        let next = match (&param.ty, &value) {
            (ParamType::Integer { .. }, ParamValue::Number(v)) if name == "seed" => {
                int_row(ui, rows, &key, label, *v as i32, hint).map(f64::from)
            }
            (ParamType::Integer { min, max }, ParamValue::Number(v)) => slider_row(
                ui,
                rows,
                &key,
                label,
                *v as f32,
                (*min as f32, *max as f32),
                NumberFormat::int(suffix),
                hint,
                enabled,
            )
            .map(|n| f64::from(n.round()).clamp(*min as f64, *max as f64)),
            (ParamType::Number { min, max }, ParamValue::Number(v))
                if (*min, *max) == (0.0, 1.0) =>
            {
                percent_row(ui, rows, &key, label, *v, (0.0, 1.0), hint, enabled)
            }
            (ParamType::Number { min, max }, ParamValue::Number(v)) => slider_row(
                ui,
                rows,
                &key,
                label,
                *v as f32,
                (*min as f32, *max as f32),
                NumberFormat {
                    decimals: if max - min > 50.0 { 1 } else { 2 },
                    trim: true,
                    suffix,
                },
                hint,
                enabled,
            )
            .map(|n| f64::from(n).clamp(*min, *max)),
            (ParamType::Choice { options }, ParamValue::Choice(current)) => {
                group_label(ui, rows, label);
                let buttons: Vec<super::properties::ChoiceButton> = options
                    .iter()
                    .map(|o| super::properties::ChoiceButton {
                        id: o,
                        label: names::option_label(lang, o),
                        selected: current == o,
                        enabled,
                        tooltip: hint,
                    })
                    .collect();
                if let Some(i) = super::properties::choice_buttons(ui, rows, &buttons) {
                    if options[i] != current {
                        changed.insert(name.into(), ParamValue::Choice(options[i].into()));
                    }
                }
                None
            }
            (ParamType::Bool, ParamValue::Bool(v)) => {
                if let Some(b) = toggle_row(ui, rows, &key, label, *v, hint, enabled) {
                    changed.insert(name.into(), ParamValue::Bool(b));
                }
                None
            }
            _ => None,
        };
        if let Some(n) = next {
            if value != ParamValue::Number(n) {
                changed.insert(name.into(), ParamValue::Number(n));
            }
        }
    }
    if changed.is_empty() {
        return None;
    }
    settings.with_catalog_values(&changed).ok()
}

/// 選択肢の欄（合わせ方・向き）が違うか（1 回で決まる変更として、続くスライダーの変更とまとめない）。
fn choices_differ(a: &EffectSettings, b: &EffectSettings) -> bool {
    use yolu_core::effects::catalog::ParamValue;
    let choices = |s: &EffectSettings| {
        s.catalog_values()
            .into_iter()
            .filter(|(_, v)| matches!(v, ParamValue::Choice(_)))
            .collect::<Vec<_>>()
    };
    choices(a) != choices(b)
}

// ───────── Generator ─────────

/// 読むマップの今の状態（文書へ渡した入力から。渡していなければ未）。
fn map_state(app: &AppState, kind: MapKind) -> Option<MeshMapState> {
    app.session
        .doc
        .effect_inputs()
        .map(kind)
        .map(|m| match m.state {
            MapState::Current => MeshMapState::Current,
            MapState::Stale => MeshMapState::Stale,
            MapState::Unverified => MeshMapState::Unverified,
        })
}

/// 読むマップの 1 行（印・名前・状態）。ツールチップに焼いたときの大きさとスロット。
fn map_row(ui: &mut Ui, app: &AppState, rows: &mut Rows, kind: MapKind, pinned: bool) {
    let lang = app.lang;
    let mesh = inputs::mesh_kind(kind);
    let state = map_state(app, kind);
    let usable = state == Some(MeshMapState::Current);
    let label = if usable && pinned {
        lang.t("panels-this-bake")
    } else {
        state_label(lang, state)
    };
    let row = rows.row(20.0, 2.0);
    let color = if usable {
        egui::Color32::from_rgb(89, 199, 107)
    } else {
        t::WARNING
    };
    let dot = Rect::from_center_size(pos2(row.left() + 6.0, row.center().y), vec2(8.0, 8.0));
    ui.painter().circle_filled(dot.center(), 4.0, color);
    let right = w::text_width(ui.painter(), label, t::LABEL_DIM) + 4.0;
    let name_rect = Rect::from_min_max(
        pos2(row.left() + 18.0, row.top()),
        pos2(row.right() - right, row.bottom()),
    );
    let name = kind_label(lang, mesh);
    let shown = w::fit(ui.painter(), name, name_rect.width(), t::LABEL);
    w::text(ui.painter(), name_rect, &shown, t::LABEL, w::Align::Left);
    w::text(
        ui.painter(),
        Rect::from_min_max(pos2(row.right() - right, row.top()), row.max),
        label,
        t::LABEL_DIM.with_color(color),
        w::Align::Right,
    );
    let tip = app
        .session
        .sets
        .current()
        .mesh_maps
        .get(mesh)
        .map(|m| {
            let p = m.provenance();
            format!(
                "{name} · {} × {} · {}",
                p.width,
                p.height,
                crate::bake::slot_list(&[p.target_slot])
            )
        })
        .unwrap_or_else(|| name.to_owned());
    ui.interact(
        row,
        ui.make_persistent_id(("fx.map", mesh as i32)),
        Sense::hover(),
    )
    .on_hover_text(tip);
}

/// 効かない理由の短い 1 行（警告の印と理由）。
fn warning_row(ui: &mut Ui, rows: &mut Rows, text: &str) {
    let r = rows.row(t::ROW_HEIGHT, 2.0);
    w::icon(
        ui.painter(),
        Rect::from_min_size(r.min, vec2(16.0, r.height())),
        "warning",
        t::WARNING,
        14.0,
    );
    let text_rect = Rect::from_min_max(pos2(r.left() + 20.0, r.top()), r.max);
    let shown = w::fit(ui.painter(), text, text_rect.width(), t::LABEL_DIM);
    w::text(
        ui.painter(),
        text_rect,
        &shown,
        t::LABEL_DIM.with_color(t::WARNING),
        w::Align::Left,
    );
    if shown != text {
        ui.interact(
            r,
            ui.make_persistent_id(("fx.warning", text.len())),
            Sense::hover(),
        )
        .on_hover_text(text);
    }
}

/// Anchor を読む段の行（Anchor・チャンネル・読み方・置いたレイヤーへ移る）。
#[allow(clippy::too_many_arguments)]
fn anchor_reading_rows(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    ctx: &egui::Context,
    g: &generator::Settings,
    enabled: bool,
) {
    let lang = app.lang;
    let current = (g.anchor.id != 0).then_some(AnchorId(g.anchor.id));
    let info = current.and_then(|id| {
        app.session.doc.find_anchor(id).map(|i| {
            (
                i.layer,
                i.placement,
                app.session.doc.layer(i.layer).map(|l| l.name().to_owned()),
            )
        })
    });
    let tip = lang.t("panels-the-anchor-this-stage-reads-tip");
    if let Some(rect) = choice_row(
        ui,
        rows,
        "fx.anchor",
        lang.t("panels-anchor"),
        &anchor_choice_name(app, current),
        Some(&match &info {
            Some((_, _, Some(name))) => format!(
                "{tip}\n{}",
                lang.f("panels-on-the-layer-tip", &[("name", &name)])
            ),
            _ => tip.to_owned(),
        }),
        enabled,
    ) {
        open_popup(app, ctx, Popup::Fx(FxChoice::Anchor), rect, rect.width());
    }
    let mask = info
        .as_ref()
        .is_some_and(|(_, p, _)| *p == AnchorPlacement::Mask);
    let channel_name = crate::m2::channel_name(lang, &app.session.doc, g.anchor.channel);
    if let Some(rect) = choice_row(
        ui,
        rows,
        "fx.anchor.channel",
        lang.t("panels-channel"),
        &channel_name,
        Some(if mask {
            lang.t("panels-a-mask-anchor-has-one-tip")
        } else {
            lang.t("panels-which-channel-of-the-stack-tip")
        }),
        enabled && !mask,
    ) {
        open_popup(
            app,
            ctx,
            Popup::Fx(FxChoice::AnchorChannel),
            rect,
            rect.width(),
        );
    }
    if let Some(rect) = choice_row(
        ui,
        rows,
        "fx.anchor.read",
        lang.t("panels-read"),
        names::read_mode_name(lang, g.anchor.read),
        Some(lang.t("panels-value-the-channel-s-value-tip")),
        enabled && !mask,
    ) {
        open_popup(
            app,
            ctx,
            Popup::Fx(FxChoice::AnchorRead),
            rect,
            rect.width(),
        );
    }
    if let Some((_, _, _)) = info {
        let r = rows.row(22.0, 4.0);
        if w::button(
            ui,
            r,
            "fx.anchor.go",
            lang.t("panels-select-the-anchor-s-layer"),
            false,
            enabled,
            Some(lang.t("panels-select-the-layer-the-anchor-tip")),
            Some("anchor"),
        )
        .clicked()
        {
            if let Some(id) = current {
                app.apply(Action::Fx(FxOp::GoToAnchor(id)));
            }
        }
    }
}

/// Generator の欄（返すのは変えた設定。変わらなければ None）。
#[allow(clippy::too_many_arguments)]
fn generator_rows(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    ctx: &egui::Context,
    layer: LayerId,
    effect: &FilterEffect,
    g: &generator::Settings,
    target: FilterTarget,
) -> Option<generator::Settings> {
    let lang = app.lang;
    let enabled = app.can_edit();
    let id = effect.id();
    let mut next = g.clone();
    // 値（範囲を通した 0〜1）を使うか: 画像の段の色のチャンネルは画素の色をそのまま使い、範囲と成分は効かない
    let values = g.kind != Kind::Image
        || target == FilterTarget::Mask
        || effect.channels().iter().any(|c| {
            app.session
                .doc
                .channel_info(*c)
                .is_some_and(|i| i.kind != yolu_core::ChannelKind::Color)
        });
    if g.kind == Kind::Anchor {
        anchor_reading_rows(ui, app, rows, ctx, g, enabled);
    }
    if g.kind == Kind::Image {
        image_source_rows(ui, app, rows, ctx, layer, id, g, &mut next, values, enabled);
    }
    // 読むマップと状態
    let used = g.used_maps();
    if !used.is_empty() {
        group_label(ui, rows, lang.t("panels-reads"));
        for kind in &used {
            map_row(ui, app, rows, *kind, !g.pins.is_empty());
        }
    }
    // 効かない理由と、ベイクのウィンドウへ
    let inactive = if effect.is_active() {
        app.session.doc.generator_inactive(layer, id).ok().flatten()
    } else {
        None
    };
    if let Some(why) = &inactive {
        warning_row(ui, rows, &lang.inactive_reason(why));
        let maps = matches!(
            why,
            InactiveReason::Generator(
                generator::Inactive::MissingMap(_)
                    | generator::Inactive::StaleMap(_)
                    | generator::Inactive::UnverifiedMap(_)
                    | generator::Inactive::MapSize(_)
                    | generator::Inactive::PinMismatch(_)
            )
        );
        if maps && !app.bake.is_baking() {
            let r = rows.row(24.0, 4.0);
            if w::button(
                ui,
                r,
                "fx.bake",
                lang.t("panels-bake-mesh-maps"),
                false,
                enabled,
                Some(lang.t("panels-open-the-bake-window-tip")),
                Some("data_scatter"),
            )
            .clicked()
            {
                app.apply(Action::Bake(BakeAction::OpenWindow));
            }
        }
    }
    // このベイクだけを読む（画像の段は塗りつぶしレイヤーの投影と同じく、いつも最新のベイクを読む）
    if !used.is_empty() && !g.candidate_maps().is_empty() {
        let pinned = !g.pins.is_empty();
        let all_usable = used
            .iter()
            .all(|k| map_state(app, *k) == Some(MeshMapState::Current));
        if let Some(on) = toggle_row(
            ui,
            rows,
            "fx.pin",
            lang.t("panels-only-this-bake"),
            pinned,
            Some(lang.t("panels-on-keep-reading-this-bake-tip")),
            enabled && (pinned || all_usable),
        ) {
            next.pins.clear();
            if on {
                for kind in &used {
                    if let Some(m) = app.session.doc.effect_inputs().map(*kind) {
                        next.pins.insert(*kind, m.condition_key.clone());
                    }
                }
            }
        }
    }
    // 種類ごとの値
    match g.kind {
        Kind::Dirt => {
            if let Some(v) = percent_row(
                ui,
                rows,
                "fx.balance",
                lang.t("panels-ao-cavities"),
                g.balance,
                (0.0, 1.0),
                Some(lang.t("panels-0-ambient-occlusion-only-100-tip")),
                enabled,
            ) {
                next.balance = v;
            }
        }
        Kind::PositionGradient => {
            if let Some(rect) = choice_row(
                ui,
                rows,
                "fx.axis",
                lang.t("panels-axis"),
                names::axis_name(g.axis),
                Some(lang.t("panels-0-at-the-bounding-box-tip")),
                enabled,
            ) {
                open_popup(app, ctx, Popup::Fx(FxChoice::Axis), rect, rect.width());
            }
        }
        Kind::Direction => {
            if let Some(rect) = choice_row(
                ui,
                rows,
                "fx.direction",
                lang.t("panels-direction"),
                &names::direction_name(lang, g.direction),
                Some(lang.t("panels-faces-turned-toward-this-world-tip")),
                enabled,
            ) {
                open_popup(app, ctx, Popup::Fx(FxChoice::Direction), rect, rect.width());
            }
            if let Some(on) = toggle_row(
                ui,
                rows,
                "fx.bent",
                lang.t("panels-bent-normal"),
                g.use_bent_normal,
                Some(lang.t("panels-read-the-bent-normal-the-tip")),
                enabled,
            ) {
                next.use_bent_normal = on;
            }
        }
        Kind::ShapeGradient => {
            shape_rows(ui, app, rows, ctx, g, &mut next, enabled);
            let editing = app.fillfx.edit_filter == Some((layer, id));
            let r = rows.row(24.0, 4.0);
            if w::button(
                ui,
                r,
                "fx.shape.edit",
                lang.t("panels-edit-in-3d-view"),
                editing,
                enabled && app.view3d.model.is_some(),
                Some(lang.t("panels-show-the-shape-in-the-tip")),
                Some("view_in_ar"),
            )
            .clicked()
            {
                app.apply(Action::Fill(crate::fillfx::FillOp::EditFilter(
                    if editing { None } else { Some((layer, id)) },
                )));
            }
        }
        Kind::IdColor => id_color_rows(ui, app, rows, layer, id, g, &mut next, enabled),
        Kind::Noise | Kind::Grunge => procedural_rows(ui, app, rows, ctx, &mut next, enabled),
        // 模様・ライト・マスクの組み立ては目録の欄から並べる（範囲・反転・合成は下の共通の行。模様・ライトの softness は自分の欄）
        Kind::Pattern | Kind::Light | Kind::MaskBuilder => {
            let common: &[&str] = if g.kind == Kind::MaskBuilder {
                &["low", "high", "softness", "invert", "blend"]
            } else {
                &["low", "high", "invert", "blend"]
            };
            if let Some(EffectSettings::Generator(r)) =
                catalog_rows(ui, app, rows, effect.settings(), common)
            {
                next.pattern = r.pattern;
                next.light = r.light;
                next.mask_builder = r.mask_builder;
            }
        }
        // アイランドごとのばらつき: シードは目録の欄から、最小・最大は互いを越えないように（範囲・反転・合成は下の共通の行）
        Kind::UvIslandVariation => {
            let kind = "uv_island_variation";
            let skip = ["low", "high", "softness", "invert", "blend", "min", "max"];
            if let Some(EffectSettings::Generator(r)) =
                catalog_rows(ui, app, rows, effect.settings(), &skip)
            {
                next.island.seed = r.island.seed;
            }
            if let Some(v) = percent_row(
                ui,
                rows,
                "fx.min",
                names::param_label(lang, kind, "min"),
                g.island.min,
                (0.0, 1.0),
                names::param_hint(lang, kind, "min"),
                enabled,
            ) {
                next.island.min = v.min(g.island.max);
            }
            if let Some(v) = percent_row(
                ui,
                rows,
                "fx.max",
                names::param_label(lang, kind, "max"),
                g.island.max,
                (0.0, 1.0),
                names::param_hint(lang, kind, "max"),
                enabled,
            ) {
                next.island.max = v.max(g.island.min);
            }
        }
        Kind::EdgeWear | Kind::Thickness | Kind::Anchor | Kind::Image => {}
    }
    // 範囲（ID の色は 0 か 1 なので範囲とやわらかさは出さない。反転は出す。模様・ライトのやわらかさは自分の欄）
    if g.kind != Kind::IdColor && values {
        group_label(ui, rows, lang.t("panels-range"));
        let gap = 0.001 + 1e-6;
        if let Some(v) = slider_row(
            ui,
            rows,
            "fx.low",
            lang.t("panels-low"),
            g.low as f32,
            (0.0, 1.0),
            decimals(3),
            Some(lang.t("panels-base-values-at-or-below-tip")),
            enabled,
        ) {
            next.low = (v as f64).min(g.high - gap).max(0.0);
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "fx.high",
            lang.t("panels-high"),
            g.high as f32,
            (0.0, 1.0),
            decimals(3),
            Some(lang.t("panels-base-values-at-or-above-tip")),
            enabled,
        ) {
            next.high = (v as f64).max(g.low + gap).min(1.0);
        }
        if !matches!(g.kind, Kind::Pattern | Kind::Light) {
            if let Some(v) = percent_row(
                ui,
                rows,
                "fx.softness",
                lang.t("panels-softness"),
                g.softness,
                (0.0, 1.0),
                Some(lang.t("panels-0-a-straight-ramp-from-tip")),
                enabled,
            ) {
                next.softness = v;
            }
        }
    }
    if let Some(on) = toggle_row(
        ui,
        rows,
        "fx.invert",
        lang.t("panels-invert"),
        g.invert,
        Some(lang.t("panels-swap-0-and-1-after-tip")),
        enabled,
    ) {
        next.invert = on;
    }
    // 崩し（ノイズ・グランジ・画像・模様・ライト・マスクの組み立ては重ねるノイズを持たない。core が断るので出さない）
    if !g.kind.is_procedural() && !g.kind.is_050() && g.kind != Kind::Image {
        group_label(ui, rows, lang.t("panels-breakup"));
        if let Some(v) = percent_row(
            ui,
            rows,
            "fx.noise",
            lang.t("panels-amount"),
            g.noise_amount,
            (0.0, 1.0),
            None,
            enabled,
        ) {
            next.noise_amount = v;
        }
        if let Some(v) = int_row(
            ui,
            rows,
            "fx.noise.seed",
            lang.t("panels-seed"),
            g.noise_seed,
            Some(lang.t("panels-the-same-seed-gives-the-tip")),
        ) {
            next.noise_seed = v;
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "fx.noise.scale",
            lang.t("panels-size-3"),
            g.noise_scale as f32,
            (0.005, 0.5),
            decimals(3),
            Some(lang.t("panels-the-size-of-the-largest-tip")),
            enabled && g.noise_amount > 0.0,
        ) {
            next.noise_scale = (v as f64).clamp(0.001, 1.0);
        }
        if let Some(rect) = choice_row(
            ui,
            rows,
            "fx.noise.space",
            lang.t("panels-placed"),
            names::noise_space_name(lang, g.noise_space),
            Some(lang.t("panels-on-the-model-continuous-across-tip")),
            enabled && g.noise_amount > 0.0,
        ) {
            open_popup(
                app,
                ctx,
                Popup::Fx(FxChoice::NoiseSpace),
                rect,
                rect.width(),
            );
        }
    }
    // 合成
    if let Some(rect) = choice_row(
        ui,
        rows,
        "fx.blend",
        lang.t("panels-combine"),
        names::blend_name(lang, g.blend),
        Some(lang.t("panels-how-the-value-meets-what-tip")),
        enabled,
    ) {
        open_popup(app, ctx, Popup::Fx(FxChoice::Blend), rect, rect.width());
    }
    (next != *g).then_some(next)
}

/// 画像の段の画像（箱・読み方）・成分・投影（塗りつぶしレイヤーの画像と投影の欄と同じ並びと部品）。`values` は成分を使うか
/// （マスクかスカラーのチャンネルに掛かる）。
#[allow(clippy::too_many_arguments)]
fn image_source_rows(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    ctx: &egui::Context,
    layer: LayerId,
    id: yolu_core::FilterId,
    g: &generator::Settings,
    next: &mut generator::Settings,
    values: bool,
    enabled: bool,
) {
    use super::fill_props::{self as fp, ImageRow, ProjectionPopups};
    use yolu_core::fill_image::ProjectionMode;
    let lang = app.lang;
    let image = (g.image.image != 0).then_some(yolu_core::ImageId(g.image.image));
    fp::image_row(
        ui,
        app,
        rows,
        ImageRow {
            key: "fx.image",
            box_key: "fx.image",
            image,
            enabled,
            popup: Popup::Fx(FxChoice::Image),
            clear_tip: lang.t("panels-remove-the-image-the-stage-tip"),
        },
        |image| Action::Fx(FxOp::SetImage { layer, id, image }),
    );
    if values {
        if let Some(rect) = choice_row(
            ui,
            rows,
            "fx.image.component",
            lang.t("panels-component"),
            names::image_component_name(lang, g.image.component),
            Some(lang.t("panels-the-image-component-used-as-tip")),
            enabled,
        ) {
            open_popup(
                app,
                ctx,
                Popup::Fx(FxChoice::ImageComponent),
                rect,
                rect.width(),
            );
        }
    }
    // 投影（見出しは置かない。最初の行の名前が「投影」）
    let p = g.image.projection;
    let popups = ProjectionPopups {
        mode: Popup::Fx(FxChoice::ImageMode),
        wrap: Popup::Fx(FxChoice::ImageWrap),
    };
    if let Some(changed) = fp::projection_rows(
        ui,
        app,
        rows,
        ctx,
        "fx.image.projection",
        &p,
        popups,
        enabled,
    ) {
        next.image.projection = changed;
    }
    if p.mode != ProjectionMode::Uv {
        let editing = app.fillfx.edit_filter == Some((layer, id));
        let fit = {
            let g = g.clone();
            move |app: &mut AppState| match app.fitted_placement_for(p.mode, None) {
                Some(placement) => {
                    let mut fitted = g;
                    fitted.image.projection.placement = placement;
                    set_settings(app, layer, id, EffectSettings::generator(fitted), false);
                }
                None => app.refuse(crate::notice::Source::Effect, app.lang.t("panels-no-model")),
            }
        };
        if let Some(placement) = fp::placement_fields(
            ui,
            app,
            rows,
            "fx.image.projection",
            &p,
            editing,
            enabled,
            lang,
            move |app| {
                app.apply(Action::Fill(crate::fillfx::FillOp::EditFilter(
                    if editing { None } else { Some((layer, id)) },
                )));
            },
            fit,
        ) {
            next.image.projection.placement = placement;
        }
    }
}

/// 形のグラデーションの形・置き場・減衰。
fn shape_rows(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    ctx: &egui::Context,
    g: &generator::Settings,
    next: &mut generator::Settings,
    enabled: bool,
) {
    let lang = app.lang;
    let v = g.volume;
    // 形の説明は新規塗りつぶしレイヤーのメニュー・塗りつぶしの欄と同じ文
    let shape_tip = names::shape_tooltip(lang);
    if let Some(rect) = choice_row(
        ui,
        rows,
        "fx.shape",
        lang.t("panels-shape"),
        names::shape_name(lang, v.shape),
        Some(shape_tip.as_str()),
        enabled,
    ) {
        open_popup(app, ctx, Popup::Fx(FxChoice::Shape), rect, rect.width());
    }
    let frame_tip = lang.t("panels-in-the-model-root-s-tip");
    if let Some(c) = vec3_row(
        ui,
        rows,
        "fx.shape.center",
        lang.t("panels-placement"),
        v.center,
        Some(frame_tip),
    ) {
        next.volume.center = c;
    }
    if let Some(r) = vec3_row(
        ui,
        rows,
        "fx.shape.rotation",
        lang.t("panels-rotation"),
        v.rotation,
        Some(lang.t("panels-euler-angles-in-degrees-turned-tip")),
    ) {
        next.volume.rotation = r.map(|d| d.clamp(-360.0, 360.0));
    }
    match v.shape {
        Shape::Box => {
            if let Some(s) = vec3_row(
                ui,
                rows,
                "fx.shape.size",
                lang.t("panels-size-3"),
                v.size,
                None,
            ) {
                next.volume.size = s.map(|d| d.clamp(1e-6, 1e6));
            }
        }
        Shape::Sphere => {
            if let Some(text) = text_row(
                ui,
                rows,
                "fx.shape.radius",
                lang.t("panels-radius"),
                &names::trim(v.size[0] / 2.0, 3),
                None,
            ) {
                if let Ok(r) = text.trim().parse::<f64>() {
                    if r.is_finite() && r > 0.0 {
                        let d = (r * 2.0).clamp(1e-6, 1e6);
                        next.volume.size = [d, d, d];
                    }
                }
            }
        }
        Shape::Plane => {
            if let Some(text) = text_row(
                ui,
                rows,
                "fx.shape.width",
                lang.t("panels-width"),
                &names::trim(v.size[1], 3),
                None,
            ) {
                if let Ok(width) = text.trim().parse::<f64>() {
                    if width.is_finite() && width > 0.0 {
                        next.volume.size[1] = width.clamp(1e-6, 1e6);
                    }
                }
            }
        }
    }
    if let Some(f) = percent_row(
        ui,
        rows,
        "fx.shape.falloff",
        lang.t("panels-falloff"),
        v.falloff,
        (0.0, 1.0),
        Some(lang.t("panels-how-much-of-the-inside-tip")),
        enabled,
    ) {
        next.volume.falloff = f;
    }
}

/// ID の色の一覧（見本・16 進・外す）と許容の幅。
#[allow(clippy::too_many_arguments)]
fn id_color_rows(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    layer: LayerId,
    filter: yolu_core::FilterId,
    g: &generator::Settings,
    next: &mut generator::Settings,
    enabled: bool,
) {
    let lang = app.lang;
    group_label(ui, rows, lang.t("panels-id-colors"));
    for (i, rgb) in g.id_colors.iter().enumerate() {
        let row = rows.row(20.0, 2.0);
        let swatch = Rect::from_min_size(
            pos2(row.left(), row.top() + 2.0),
            vec2(34.0, row.height() - 4.0),
        );
        w::rounded(
            ui.painter(),
            swatch,
            egui::Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, *rgb as u8),
            3.0,
        );
        w::outline(ui.painter(), swatch, t::BORDER, 1.0, 3.0);
        w::text(
            ui.painter(),
            Rect::from_min_max(
                pos2(swatch.right() + 8.0, row.top()),
                pos2(row.right() - 28.0, row.bottom()),
            ),
            &format!("{rgb:06X}"),
            t::LABEL,
            w::Align::Left,
        );
        if w::icon_button(
            ui,
            Rect::from_min_size(
                pos2(row.right() - 24.0, row.top()),
                vec2(24.0, row.height()),
            ),
            ("fx.id.remove", i),
            "close",
            lang.t("panels-take-this-colour-out-tip"),
            false,
            enabled,
            15.0,
        )
        .clicked()
        {
            next.id_colors.retain(|c| c != rgb);
        }
    }
    let picking = app.fx.id_pick == Some((layer, filter));
    let r = rows.row(24.0, 4.0);
    if w::button(
        ui,
        r,
        "fx.id.pick",
        lang.t("panels-pick-from-id-map"),
        picking,
        enabled,
        Some(lang.t("panels-then-click-the-2d-canvas-tip")),
        Some("target"),
    )
    .clicked()
    {
        app.apply(Action::Fx(FxOp::PickIdColors {
            layer,
            id: filter,
            on: !picking,
        }));
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "fx.id.tolerance",
        lang.t("panels-tolerance"),
        g.id_tolerance as f32,
        (0.0, 255.0),
        NumberFormat::int(""),
        Some(lang.t("panels-how-far-largest-8-bit-tip")),
        enabled,
    ) {
        next.id_tolerance = v.round().clamp(0.0, 255.0) as u8;
    }
}

// ───────── Anchor ─────────

/// Anchor の行の欄（名前・読んでいる段の数・外す）。
fn anchor_body(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, id: AnchorId, name: &str) {
    let lang = app.lang;
    let title = lang.t("panels-anchor");
    let (open, _) = section(ui, app, rows, "effect", title, "anchor", None);
    if !open {
        return;
    }
    anchor_fields(ui, app, rows, id, name, "fx");
}

/// Anchor の名前の欄・読んでいる段の数・外すボタン（プロパティの「レイヤー」と「レイヤーマスク」の欄と、Anchor の行の欄で共有）。
fn anchor_fields(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    id: AnchorId,
    name: &str,
    key: &str,
) {
    let lang = app.lang;
    let enabled = app.can_edit();
    let readers = super::effect_rows::anchor_readers(&app.session.doc, id);
    let count = lang.f("panels-read-by", &[("readers", &readers.len())]);
    let row = rows.row(t::ROW_HEIGHT, 4.0);
    w::icon(
        ui.painter(),
        Rect::from_min_size(row.min, vec2(18.0, row.height())),
        "anchor",
        t::ACCENT,
        15.0,
    );
    let count_w = w::text_width(ui.painter(), &count, t::LABEL_DIM) + 6.0;
    let name_rect = Rect::from_min_max(
        pos2(row.left() + 22.0, row.top()),
        pos2(
            (row.right() - 28.0 - count_w).max(row.left() + 62.0),
            row.bottom(),
        ),
    );
    let out = w::text_field(
        ui,
        name_rect,
        (key, "anchor.name", id.0),
        name,
        Some(lang.t("panels-the-anchor-s-name-generators-tip")),
        false,
    );
    if let Some(next) = out.committed {
        if !next.trim().is_empty() {
            app.apply(Action::Fx(FxOp::RenameAnchor { id, name: next }));
        }
    }
    let count_rect = Rect::from_min_max(
        pos2(name_rect.right() + 4.0, row.top()),
        pos2(row.right() - 26.0, row.bottom()),
    );
    w::text(
        ui.painter(),
        count_rect,
        &count,
        t::LABEL_DIM.with_color(t::TEXT_DIM),
        w::Align::Left,
    );
    let tip = if readers.is_empty() {
        lang.t("panels-no-generator-reads-this-anchor-tip")
            .to_owned()
    } else {
        let names: Vec<String> = readers
            .iter()
            .filter_map(|(l, target)| {
                app.session.doc.layer(*l).map(|layer| {
                    if *target == FilterTarget::Mask {
                        lang.f("panels-mask", &[("name", &layer.name())])
                    } else {
                        layer.name().to_owned()
                    }
                })
            })
            .collect();
        format!("{}\n{}", lang.t("panels-read-by-2"), names.join("\n"))
    };
    ui.interact(
        count_rect,
        ui.make_persistent_id((key, "anchor.readers", id.0)),
        Sense::hover(),
    )
    .on_hover_text(tip);
    if w::icon_button(
        ui,
        Rect::from_min_size(
            pos2(row.right() - 24.0, row.top()),
            vec2(24.0, row.height()),
        ),
        (key, "anchor.remove", id.0),
        "delete",
        lang.t("panels-remove-the-anchor-generators-that-tip"),
        false,
        enabled,
        16.0,
    )
    .clicked()
    {
        app.apply(Action::Fx(FxOp::RemoveAnchor(id)));
    }
}

/// プロパティの「レイヤー」と「レイヤーマスク」の欄の「フィルターを追加」と「ジェネレーターを追加」の 2 行（押すと、画素かマスクへ足す
/// フィルター・ジェネレーターの一覧。どちらも同じスタックに積む）。
pub fn add_effect_row(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, target: FilterTarget) {
    let lang: Lang = app.lang;
    let enabled = app.can_edit();
    let mask = target == FilterTarget::Mask;
    let buttons = [
        (
            Popup::AddFilter(target),
            if mask { "fx.add.mask" } else { "fx.add" },
            if mask {
                lang.t("panels-add-filter-to-mask")
            } else {
                crate::fx::menu::add_filter_label(lang)
            },
            if mask {
                lang.t("panels-add-a-filter-blur-levels")
            } else {
                lang.t("panels-add-a-filter-blur-levels-2")
            },
            "auto_awesome",
        ),
        (
            Popup::AddGenerator(target),
            if mask {
                "fx.add.generator.mask"
            } else {
                "fx.add.generator"
            },
            if mask {
                lang.t("panels-add-generator-to-mask")
            } else {
                crate::fx::menu::add_generator_label(lang)
            },
            if mask {
                lang.t("panels-add-a-generator-on-the")
            } else {
                lang.t("panels-add-a-generator-values-from")
            },
            "generator",
        ),
    ];
    for (popup, key, label, tip, icon) in buttons {
        let r = rows.row(24.0, 4.0);
        if w::button(ui, r, key, label, false, enabled, Some(tip), Some(icon)).clicked() {
            let ctx = ui.ctx().clone();
            open_popup(app, &ctx, popup, r, r.width());
        }
    }
}

/// プロパティの「レイヤー」（placement が Layer）と「レイヤーマスク」（Mask）の欄の Anchor の行: 無ければ置くボタン、あれば名前の欄。
pub fn anchor_row(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    layer: LayerId,
    placement: AnchorPlacement,
) {
    let lang: Lang = app.lang;
    let enabled = app.can_edit();
    let anchor = app.session.doc.layer(layer).and_then(|l| match placement {
        AnchorPlacement::Layer => l.anchor(),
        AnchorPlacement::Mask => l.mask().and_then(|m| m.anchor()),
    });
    match anchor.map(|a| (a.id(), a.name().to_owned())) {
        None => {
            let r = rows.row(24.0, 4.0);
            let (label, tip) = match placement {
                AnchorPlacement::Layer => (
                    lang.t("panels-add-anchor"),
                    lang.t("panels-name-the-stack-s-result"),
                ),
                AnchorPlacement::Mask => (
                    lang.t("panels-add-anchor-to-mask"),
                    lang.t("panels-name-this-mask-so-generators"),
                ),
            };
            let key = match placement {
                AnchorPlacement::Layer => "anchor.add",
                AnchorPlacement::Mask => "mask.anchor.add",
            };
            if w::button(ui, r, key, label, false, enabled, Some(tip), Some("anchor")).clicked() {
                app.apply(Action::Fx(FxOp::AddAnchor { layer, placement }));
            }
        }
        Some((id, name)) => {
            let key = match placement {
                AnchorPlacement::Layer => "layer",
                AnchorPlacement::Mask => "mask",
            };
            anchor_fields(ui, app, rows, id, &name, key);
        }
    }
}

/// ノイズ・グランジの欄。共通（空間・シード・大きさ・回転ほか）を土台に、ノイズ専用の段とグランジのプリセットの格子を足す。
fn procedural_rows(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    ctx: &egui::Context,
    next: &mut generator::Settings,
    enabled: bool,
) {
    let lang = app.lang;
    let kind = next.kind;
    let p = &mut next.procedural;
    // 共通
    if let Some(rect) = choice_row(
        ui,
        rows,
        "fx.procedural.space",
        lang.t("panels-space"),
        names::procedural_space_name(lang, p.space),
        None,
        enabled,
    ) {
        open_popup(
            app,
            ctx,
            Popup::Fx(FxChoice::ProceduralSpace),
            rect,
            rect.width(),
        );
    }
    // ノイズ専用（core は、グランジが基底・重ね方などを既定から動かすと断る）
    if kind == Kind::Noise {
        if let Some(rect) = choice_row(
            ui,
            rows,
            "fx.procedural.basis",
            lang.t("panels-basis"),
            names::noise_basis_name(lang, p.basis),
            None,
            enabled,
        ) {
            open_popup(
                app,
                ctx,
                Popup::Fx(FxChoice::NoiseBasis),
                rect,
                rect.width(),
            );
        }
        if p.basis == generator::NoiseBasis::Worley {
            if let Some(rect) = choice_row(
                ui,
                rows,
                "fx.procedural.cell_output",
                lang.t("panels-cell-output"),
                names::cell_output_name(p.cell_output),
                None,
                enabled,
            ) {
                open_popup(
                    app,
                    ctx,
                    Popup::Fx(FxChoice::CellOutput),
                    rect,
                    rect.width(),
                );
            }
        }
        if let Some(rect) = choice_row(
            ui,
            rows,
            "fx.procedural.fractal",
            lang.t("panels-fractal"),
            names::fractal_mode_name(p.fractal),
            None,
            enabled,
        ) {
            open_popup(
                app,
                ctx,
                Popup::Fx(FxChoice::FractalMode),
                rect,
                rect.width(),
            );
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "fx.procedural.octaves",
            lang.t("panels-octaves"),
            p.octaves as f32,
            (1.0, 8.0),
            NumberFormat::int(""),
            None,
            enabled,
        ) {
            p.octaves = v.round() as u32;
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "fx.procedural.lacunarity",
            lang.t("panels-lacunarity"),
            p.lacunarity as f32,
            (1.0, 4.0),
            decimals(3),
            None,
            enabled,
        ) {
            p.lacunarity = v as f64;
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "fx.procedural.gain",
            lang.t("panels-gain"),
            p.gain as f32,
            (0.0, 1.0),
            decimals(3),
            None,
            enabled,
        ) {
            p.gain = v as f64;
        }
    }
    // グランジ専用（core は、ノイズがプリセットを既定から動かすと断る）
    if kind == Kind::Grunge {
        group_label(ui, rows, lang.t("panels-preset"));
        let r = rows.row(super::grunge_picker::height(rows.width()), 4.0);
        crate::ui::widgets::enabled_scope(ui, "fx.procedural.preset", enabled, |ui| {
            if let Some(preset) = super::grunge_picker::show(ui, r, p.preset, lang) {
                p.preset = preset;
                p.scale = preset.default_scale();
                (next.low, next.high) = preset.default_levels();
            }
        });
    }
    // 共通
    if let Some(v) = int_row(
        ui,
        rows,
        "fx.procedural.seed",
        lang.t("panels-seed"),
        p.seed,
        None,
    ) {
        p.seed = v;
    }
    let r = rows.row(24.0, 4.0);
    if w::button(
        ui,
        r,
        "fx.procedural.reroll",
        lang.t("panels-reroll"),
        false,
        enabled,
        None,
        Some("restart_alt"),
    )
    .clicked()
    {
        p.seed = p.seed.wrapping_mul(1664525).wrapping_add(1013904223);
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "fx.procedural.scale",
        lang.t("panels-pattern-size"),
        p.scale as f32,
        (0.001, 1.0),
        decimals(3),
        None,
        enabled,
    ) {
        p.scale = (v as f64).clamp(0.001, 1.0);
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "fx.procedural.bleed",
        lang.t("panels-bleed"),
        p.bleed as f32,
        (0.0, 1.0),
        decimals(3),
        None,
        enabled,
    ) {
        p.bleed = (v as f64).clamp(0.0, 1.0);
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "fx.procedural.blend_width",
        lang.t("panels-triplanar-width"),
        p.blend_width as f32,
        (0.0, 1.0),
        decimals(3),
        None,
        enabled,
    ) {
        p.blend_width = (v as f64).clamp(0.0, 1.0);
    }
    let rotation_tip = lang.t("panels-rotation-has-no-effect-in-tip");
    for (i, axis) in (0..3).map(|i| (i, names::axis_name(i))) {
        if let Some(v) = slider_row(
            ui,
            rows,
            &format!("fx.procedural.rotation.{i}"),
            &format!("{} {axis}", lang.t("panels-rotation")),
            p.rotation[i] as f32,
            (-360.0, 360.0),
            decimals(1),
            Some(rotation_tip),
            enabled,
        ) {
            p.rotation[i] = v as f64;
        }
    }
}
