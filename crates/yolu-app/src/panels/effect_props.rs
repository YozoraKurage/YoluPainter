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
    if let Some((layer, effect, target)) =
        app.fx.filter(&app.doc).map(|(l, e, t)| (l, e.clone(), t))
    {
        filter_body(ui, app, rows, ctx, layer, &effect, target);
    } else if let Some(info) = app
        .fx
        .anchor(&app.doc)
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
        FilterTarget::Mask => lang.pick(format!("{name}（マスク）"), format!("{name} (mask)")),
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
                lang.pick("半径", "Radius"),
                *radius as f32,
                (1.0, 256.0),
                NumberFormat::int(" px"),
                Some(lang.pick(
                    "広がり（画素。標準偏差は半径の約 1/3）",
                    "Reach in pixels (standard deviation ≈ radius / 3)",
                )),
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
                lang.pick("半径", "Radius"),
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
                lang.pick("量", "Amount"),
                a as f32,
                (0.0, 5.0),
                decimals(2),
                Some(lang.pick(
                    "輪郭をどれだけ強めるか",
                    "How much the edges are strengthened",
                )),
                enabled,
            ) {
                a = v as f64;
                changed = true;
            }
            if let Some(v) = slider_row(
                ui,
                rows,
                "fx.threshold",
                lang.pick("しきい値", "Threshold"),
                th as f32,
                (0.0, 255.0),
                NumberFormat::int(""),
                Some(lang.pick(
                    "これより小さい差（0〜255）は触らない",
                    "Differences smaller than this (0–255) are left alone",
                )),
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
                lang.pick("量", "Amount"),
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
                lang.pick("シード", "Seed"),
                s,
                Some(lang.pick(
                    "同じシードなら同じノイズ",
                    "The same seed gives the same noise.",
                )),
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
            group_label(ui, rows, lang.pick("入力", "Input"));
            if let Some(v) = slider_row(
                ui,
                rows,
                "fx.ib",
                lang.pick("黒", "Black"),
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
                lang.pick("白", "White"),
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
                lang.pick("ガンマ", "Gamma"),
                gm,
                (0.1, 9.99),
                decimals(2),
                None,
                enabled,
            ) {
                gm = v;
                changed = true;
            }
            group_label(ui, rows, lang.pick("出力", "Output"));
            if let Some(v) = slider_row(
                ui,
                rows,
                "fx.ob",
                lang.pick("黒", "Black"),
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
                lang.pick("白", "White"),
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
        EffectSettings::Filter(_) => {
            if let Some(value) = effect.settings().color_adjust() {
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
                    message: &mut app.message,
                };
                if let Some(change) = super::color_adjust::rows(ui, rows, &mut params, &value) {
                    discrete = change.discrete;
                    next = Some(EffectSettings::from_color_adjust(change.value));
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
        lang.pick("強さ", "Strength"),
        effect.strength(),
        (0.0, 1.0),
        Some(lang.pick(
            "結果をどれだけ混ぜるか",
            "How much of the result is mixed in",
        )),
        enabled,
    ) {
        app.apply(Action::Fx(FxOp::SetStrength {
            layer,
            id,
            strength: v,
            coalesce: true,
        }));
    }
    // 描くチャンネルに掛からない画素の段は短く知らせる
    let paint = app.m2.paint_channel;
    if target == FilterTarget::Content && !effect.applies_to(paint) {
        let channel = crate::m2::channel_name(lang, &app.doc, paint);
        status_row(
            ui,
            rows,
            &lang.pick(
                format!("{channel} には掛かりません"),
                format!("Not applied to {channel}"),
            ),
        );
    }
    rows.space(4.0);
}

// ───────── Generator ─────────

/// 読むマップの今の状態（文書へ渡した入力から。渡していなければ未）。
fn map_state(app: &AppState, kind: MapKind) -> Option<MeshMapState> {
    app.doc.effect_inputs().map(kind).map(|m| match m.state {
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
        lang.pick("このベイク", "This bake")
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

/// Anchor を読む段の行（Anchor・チャンネル・読み方・置いた層へ移る）。
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
        app.doc.find_anchor(id).map(|i| {
            (
                i.layer,
                i.placement,
                app.doc.layer(i.layer).map(|l| l.name().to_owned()),
            )
        })
    });
    let tip = lang.pick(
        "この段が読むアンカー（この層より下の層のもの、または下の層のマスクのもの）",
        "The anchor this stage reads (one on a layer below, or on a lower layer's mask)",
    );
    if let Some(rect) = choice_row(
        ui,
        rows,
        "fx.anchor",
        lang.pick("アンカー", "Anchor"),
        &anchor_choice_name(app, current),
        Some(&match &info {
            Some((_, _, Some(name))) => format!(
                "{tip}\n{}",
                lang.pick(
                    format!("層「{name}」にあります"),
                    format!("On the layer \"{name}\"")
                )
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
    let channel_name = crate::m2::channel_name(lang, &app.doc, g.anchor.channel);
    if let Some(rect) = choice_row(
        ui,
        rows,
        "fx.anchor.channel",
        lang.pick("チャンネル", "Channel"),
        &channel_name,
        Some(if mask {
            lang.pick(
                "マスクのアンカーの値は 1 つ（その層の見える度合い）",
                "A mask anchor has one value: how much its layer shows",
            )
        } else {
            lang.pick(
                "合成のどのチャンネルを読むか（ハイト: 描いた凹凸）",
                "Which channel of the stack to read (Height: the painted relief)",
            )
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
        lang.pick("読み方", "Read"),
        names::read_mode_name(lang, g.anchor.read),
        Some(lang.pick(
            "値: 描いてある所のチャンネルの値（何も無い所は 0。色は明るさ）。覆い: その所の合成の不透明さ。",
            "Value: the channel's value where something is painted (0 where nothing is; colours by brightness). Coverage: how opaque the stack is there.",
        )),
        enabled && !mask,
    ) {
        open_popup(app, ctx, Popup::Fx(FxChoice::AnchorRead), rect, rect.width());
    }
    if let Some((_, _, _)) = info {
        let r = rows.row(22.0, 4.0);
        if w::button(
            ui,
            r,
            "fx.anchor.go",
            lang.pick("アンカーの層を選ぶ", "Select the Anchor's Layer"),
            false,
            enabled,
            Some(lang.pick(
                "アンカーを置いた層を選びます",
                "Select the layer the anchor is on",
            )),
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
    _target: FilterTarget,
) -> Option<generator::Settings> {
    let lang = app.lang;
    let enabled = app.can_edit();
    let id = effect.id();
    let mut next = g.clone();
    if g.kind == Kind::Anchor {
        anchor_reading_rows(ui, app, rows, ctx, g, enabled);
    }
    // 読むマップと状態
    let used = g.used_maps();
    if !used.is_empty() {
        group_label(ui, rows, lang.pick("読むマップ", "Reads"));
        for kind in &used {
            map_row(ui, app, rows, *kind, !g.pins.is_empty());
        }
    }
    // 効かない理由と、ベイクの窓へ
    let inactive = if effect.is_active() {
        app.doc.generator_inactive(layer, id).ok().flatten()
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
                lang.pick("メッシュマップをベイク…", "Bake Mesh Maps…"),
                false,
                enabled,
                Some(lang.pick("ベイクの窓を開く", "Open the bake window")),
                Some("data_scatter"),
            )
            .clicked()
            {
                app.apply(Action::Bake(BakeAction::OpenWindow));
            }
        }
    }
    // このベイクだけを読む
    if !used.is_empty() {
        let pinned = !g.pins.is_empty();
        let all_usable = used
            .iter()
            .all(|k| map_state(app, *k) == Some(MeshMapState::Current));
        if let Some(on) = toggle_row(
            ui,
            rows,
            "fx.pin",
            lang.pick("このベイクだけを読む", "Only this bake"),
            pinned,
            Some(lang.pick(
                "入: 今のベイクを読み続ける。切: 最新のベイクに従う",
                "On: keep reading this bake. Off: follow the latest bake.",
            )),
            enabled && (pinned || all_usable),
        ) {
            next.pins.clear();
            if on {
                for kind in &used {
                    if let Some(m) = app.doc.effect_inputs().map(*kind) {
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
                lang.pick("AO ↔ 隙間", "AO ↔ Cavities"),
                g.balance,
                (0.0, 1.0),
                Some(lang.pick(
                    "0 %: 環境遮蔽だけ。100 %: 隙間（曲率の凹）だけ。",
                    "0 %: ambient occlusion only. 100 %: cavities (concave curvature) only.",
                )),
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
                lang.pick("軸", "Axis"),
                names::axis_name(g.axis),
                Some(lang.pick(
                    "このワールドの軸に沿って、境界箱のいちばん小さい所が 0、いちばん大きい所が 1",
                    "0 at the bounding box's minimum, 1 at its maximum along this world axis",
                )),
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
                lang.pick("向き", "Direction"),
                &names::direction_name(lang, g.direction),
                Some(lang.pick(
                    "このワールドの向きを向いた面が 1、直角の面が 0.5、反対を向いた面が 0",
                    "Faces turned toward this world direction get 1, faces at right angles 0.5, faces turned away 0",
                )),
                enabled,
            ) {
                open_popup(app, ctx, Popup::Fx(FxChoice::Direction), rect, rect.width());
            }
            if let Some(on) = toggle_row(
                ui,
                rows,
                "fx.bent",
                lang.pick("ベントノーマル", "Bent Normal"),
                g.use_bent_normal,
                Some(lang.pick(
                    "面の法線の代わりにベントノーマル（遮られない向き）を読みます",
                    "Read the bent normal (the open directions) instead of the surface normal",
                )),
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
                lang.pick("3D ビューで編集", "Edit in 3D View"),
                editing,
                enabled && app.view3d.model.is_some(),
                Some(lang.pick(
                    "3D ビューに形とハンドルを出す",
                    "Show the shape in the 3D view with handles",
                )),
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
        Kind::EdgeWear | Kind::Thickness | Kind::Anchor => {}
    }
    // 範囲（ID の色は 0 か 1 なので範囲とやわらかさは出さない。反転は出す）
    if g.kind != Kind::IdColor {
        group_label(ui, rows, lang.pick("範囲", "Range"));
        let gap = 0.001 + 1e-6;
        if let Some(v) = slider_row(
            ui,
            rows,
            "fx.low",
            lang.pick("下限", "Low"),
            g.low as f32,
            (0.0, 1.0),
            decimals(3),
            Some(lang.pick(
                "元の値がこれ以下なら 0",
                "Base values at or below this give 0",
            )),
            enabled,
        ) {
            next.low = (v as f64).min(g.high - gap).max(0.0);
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "fx.high",
            lang.pick("上限", "High"),
            g.high as f32,
            (0.0, 1.0),
            decimals(3),
            Some(lang.pick(
                "元の値がこれ以上なら 1",
                "Base values at or above this give 1",
            )),
            enabled,
        ) {
            next.high = (v as f64).max(g.low + gap).min(1.0);
        }
        if let Some(v) = percent_row(
            ui,
            rows,
            "fx.softness",
            lang.pick("やわらかさ", "Softness"),
            g.softness,
            (0.0, 1.0),
            Some(lang.pick(
                "0 %: 下限から上限までまっすぐ。100 %: なめらかな S 字。",
                "0 %: a straight ramp from low to high. 100 %: a smooth S curve.",
            )),
            enabled,
        ) {
            next.softness = v;
        }
    }
    if let Some(on) = toggle_row(
        ui,
        rows,
        "fx.invert",
        lang.pick("反転", "Invert"),
        g.invert,
        Some(lang.pick(
            "範囲の後で 0 と 1 を入れ替えます",
            "Swap 0 and 1 after the range",
        )),
        enabled,
    ) {
        next.invert = on;
    }
    // 崩し（ノイズ・グランジは重ねるノイズを持たない。core が断るので出さない）
    if !g.kind.is_procedural() {
        group_label(ui, rows, lang.pick("崩し", "Breakup"));
        if let Some(v) = percent_row(
            ui,
            rows,
            "fx.noise",
            lang.pick("量", "Amount"),
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
            lang.pick("シード", "Seed"),
            g.noise_seed,
            Some(lang.pick(
                "同じシードなら同じノイズ",
                "The same seed gives the same noise.",
            )),
        ) {
            next.noise_seed = v;
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "fx.noise.scale",
            lang.pick("大きさ", "Size"),
            g.noise_scale as f32,
            (0.005, 0.5),
            decimals(3),
            Some(lang.pick(
                "いちばん大きな模様の大きさ（モデルの境界箱の対角線に対する割合。UV なら UV の正方形に対する割合）",
                "The size of the largest features, as a fraction of the model's bounding-box diagonal (or of the UV square)",
            )),
            enabled && g.noise_amount > 0.0,
        ) {
            next.noise_scale = (v as f64).clamp(0.001, 1.0);
        }
        if let Some(rect) = choice_row(
            ui,
            rows,
            "fx.noise.space",
            lang.pick("置き場", "Placed"),
            names::noise_space_name(lang, g.noise_space),
            Some(lang.pick(
                "モデルの上: UV の継ぎ目でも途切れません（位置のマップを読みます）。UV: マップは要りませんが、UV アイランドの境目で途切れます。",
                "On the model: continuous across UV seams (reads the Position map). UV: needs no map, but jumps where UV islands meet.",
            )),
            enabled && g.noise_amount > 0.0,
        ) {
            open_popup(app, ctx, Popup::Fx(FxChoice::NoiseSpace), rect, rect.width());
        }
    }
    // 合成
    if let Some(rect) = choice_row(
        ui,
        rows,
        "fx.blend",
        lang.pick("合成", "Combine"),
        names::blend_name(lang, g.blend),
        Some(lang.pick(
            "値とスタックの下の結果の合わせ方（マスクでは、レイヤーの見える度合いに対して。白が見える）",
            "How the value meets what is below it in the stack (on a mask: how much of the layer shows; white shows)",
        )),
        enabled,
    ) {
        open_popup(app, ctx, Popup::Fx(FxChoice::Blend), rect, rect.width());
    }
    (next != *g).then_some(next)
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
        lang.pick("形", "Shape"),
        names::shape_name(lang, v.shape),
        Some(shape_tip.as_str()),
        enabled,
    ) {
        open_popup(app, ctx, Popup::Fx(FxChoice::Shape), rect, rect.width());
    }
    let frame_tip = lang.pick(
        "モデルのルートの空間での位置と回転（シーンの単位）",
        "In the model root's space: its position and rotation, in scene units",
    );
    if let Some(c) = vec3_row(
        ui,
        rows,
        "fx.shape.center",
        lang.pick("位置", "Placement"),
        v.center,
        Some(frame_tip),
    ) {
        next.volume.center = c;
    }
    if let Some(r) = vec3_row(
        ui,
        rows,
        "fx.shape.rotation",
        lang.pick("回転", "Rotation"),
        v.rotation,
        Some(lang.pick(
            "度（Z、X、Y の順に回した角度）",
            "Euler angles in degrees (turned about Z, then X, then Y)",
        )),
    ) {
        next.volume.rotation = r.map(|d| d.clamp(-360.0, 360.0));
    }
    match v.shape {
        Shape::Box => {
            if let Some(s) = vec3_row(
                ui,
                rows,
                "fx.shape.size",
                lang.pick("大きさ", "Size"),
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
                lang.pick("半径", "Radius"),
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
                lang.pick("幅", "Width"),
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
        lang.pick("減衰", "Falloff"),
        v.falloff,
        (0.0, 1.0),
        Some(lang.pick(
            "境界で 1 から 0 へ落ちる範囲（0 %: 硬い縁。100 %: 真ん中まで）",
            "How much of the inside fades from 1 to 0 at the boundary (0 %: a hard edge; 100 %: the fade reaches the middle)",
        )),
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
    group_label(ui, rows, lang.pick("ID の色", "ID Colors"));
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
            lang.pick("この色を外す", "Take this colour out"),
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
        lang.pick("ID マップから選ぶ", "Pick from ID Map"),
        picking,
        enabled,
        Some(lang.pick(
            "押してから 2D のキャンバスか 3D ビューを押すと、その所の ID の色を追加します（Ctrl を押しながらなら外します）。もう一度押すと終わります",
            "Then click the 2D canvas or the 3D view: each click adds the ID colour there (Ctrl+click takes it out). Press again to stop",
        )),
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
        lang.pick("許容", "Tolerance"),
        g.id_tolerance as f32,
        (0.0, 255.0),
        NumberFormat::int(""),
        Some(lang.pick(
            "ID の色が、並べた色からどこまで離れていてよいか（8 ビットの各チャンネルの差の最大）",
            "How far (largest 8-bit channel difference) a texel's ID colour may be from a listed one",
        )),
        enabled,
    ) {
        next.id_tolerance = v.round().clamp(0.0, 255.0) as u8;
    }
}

// ───────── Anchor ─────────

/// Anchor の行の欄（名前・読んでいる段の数・外す）。
fn anchor_body(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, id: AnchorId, name: &str) {
    let lang = app.lang;
    let title = lang.pick("アンカー", "Anchor");
    let (open, _) = section(ui, app, rows, "effect", title, "anchor", None);
    if !open {
        return;
    }
    anchor_fields(ui, app, rows, id, name, "fx");
}

/// Anchor の名前の欄・読んでいる段の数・外すボタン（プロパティの「層」と「レイヤーマスク」の欄と、Anchor の行の欄で共有）。
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
    let readers = super::effect_rows::anchor_readers(&app.doc, id);
    let count = lang.pick(
        format!("{} 段が読む", readers.len()),
        format!("read by {}", readers.len()),
    );
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
        Some(lang.pick(
            "アンカーの名前（上の層のジェネレーターはこの名前で選びます）",
            "The anchor's name: generators above list it by this name",
        )),
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
        lang.pick(
            "読んでいるジェネレーターはありません",
            "No generator reads this anchor.",
        )
        .to_owned()
    } else {
        let names: Vec<String> = readers
            .iter()
            .filter_map(|(l, target)| {
                app.doc.layer(*l).map(|layer| {
                    if *target == FilterTarget::Mask {
                        lang.pick(
                            format!("{}（マスク）", layer.name()),
                            format!("{} (mask)", layer.name()),
                        )
                    } else {
                        layer.name().to_owned()
                    }
                })
            })
            .collect();
        format!(
            "{}\n{}",
            lang.pick("読んでいる段:", "Read by:"),
            names.join("\n")
        )
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
        lang.pick(
            "アンカーを外す（読んでいるジェネレーターは、取り消すまで入力をそのまま通します）",
            "Remove the anchor (generators that read it pass their input through until you undo)",
        ),
        false,
        enabled,
        16.0,
    )
    .clicked()
    {
        app.apply(Action::Fx(FxOp::RemoveAnchor(id)));
    }
}

/// プロパティの「層」と「レイヤーマスク」の欄の「フィルターを足す」（押すと、画素かマスクへ足すフィルターと Generator の一覧）。
pub fn add_effect_row(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, target: FilterTarget) {
    let lang: Lang = app.lang;
    let enabled = app.can_edit();
    let r = rows.row(24.0, 4.0);
    let (key, label, tip) = match target {
        FilterTarget::Content => (
            "fx.add",
            lang.pick("フィルターを追加", "Add Filter"),
            lang.pick(
                "画素にフィルターかジェネレーター（焼いたメッシュマップから値を作る）を追加",
                "Add a filter or a generator (values from the baked mesh maps) on the pixels",
            ),
        ),
        FilterTarget::Mask => (
            "fx.add.mask",
            lang.pick("マスクにフィルターを追加", "Add Filter to Mask"),
            lang.pick(
                "マスクにフィルターかジェネレーター（レイヤーの見える所を、焼いたメッシュマップから作る）を追加",
                "Add a filter or a generator on the mask (where the layer shows, from the baked mesh maps)",
            ),
        ),
    };
    if w::button(
        ui,
        r,
        key,
        label,
        false,
        enabled,
        Some(tip),
        Some("auto_awesome"),
    )
    .clicked()
    {
        let ctx = ui.ctx().clone();
        open_popup(app, &ctx, Popup::AddEffect(target), r, r.width());
    }
}

/// プロパティの「層」（placement が Layer）と「レイヤーマスク」（Mask）の欄の Anchor の行: 無ければ置くボタン、あれば名前の欄。
pub fn anchor_row(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    layer: LayerId,
    placement: AnchorPlacement,
) {
    let lang: Lang = app.lang;
    let enabled = app.can_edit();
    let anchor = app.doc.layer(layer).and_then(|l| match placement {
        AnchorPlacement::Layer => l.anchor(),
        AnchorPlacement::Mask => l.mask().and_then(|m| m.anchor()),
    });
    match anchor.map(|a| (a.id(), a.name().to_owned())) {
        None => {
            let r = rows.row(24.0, 4.0);
            let (label, tip) = match placement {
                AnchorPlacement::Layer => (
                    lang.pick("アンカーを置く", "Add Anchor"),
                    lang.pick(
                        "この層までの合成の結果に名前を付けて、上の層のジェネレーターが読めるようにします（下の層の Height で上の層の摩耗を決める、など）",
                        "Name the stack's result up to this layer, so generators on the layers above can read it (a lower layer's Height can drive an upper layer's wear, for example)",
                    ),
                ),
                AnchorPlacement::Mask => (
                    lang.pick("マスクにアンカーを置く", "Add Anchor to Mask"),
                    lang.pick(
                        "このマスクに名前を付けて、上の層のジェネレーターが「この層がどれだけ見えるか」を読めるようにします",
                        "Name this mask, so generators on the layers above can read how much this layer shows",
                    ),
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
        lang.pick("空間", "Space"),
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
            lang.pick("基底", "Basis"),
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
                lang.pick("セルの出力", "Cell Output"),
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
            lang.pick("重ね方", "Fractal"),
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
            lang.pick("オクターブ", "Octaves"),
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
            lang.pick("ラクナリティ", "Lacunarity"),
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
            lang.pick("ゲイン", "Gain"),
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
        group_label(ui, rows, lang.pick("プリセット", "Preset"));
        let r = rows.row(super::grunge_picker::height(rows.width()), 4.0);
        ui.add_enabled_ui(enabled, |ui| {
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
        lang.pick("シード", "Seed"),
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
        lang.pick("振り直す", "Reroll"),
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
        lang.pick("模様の大きさ", "Pattern Size"),
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
        lang.pick("にじみ", "Bleed"),
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
        lang.pick("トライプラナーの幅", "Triplanar Width"),
        p.blend_width as f32,
        (0.0, 1.0),
        decimals(3),
        None,
        enabled,
    ) {
        p.blend_width = (v as f64).clamp(0.0, 1.0);
    }
    let rotation_tip = lang.pick(
        "UV では回転は効きません",
        "Rotation has no effect in UV space",
    );
    for (i, axis) in (0..3).map(|i| (i, names::axis_name(i))) {
        if let Some(v) = slider_row(
            ui,
            rows,
            &format!("fx.procedural.rotation.{i}"),
            &format!("{} {axis}", lang.pick("回転", "Rotation")),
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
