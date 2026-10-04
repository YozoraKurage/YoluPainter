//! プロパティの欄の描くツールのタブ（Unity 版の Brush・Alpha・Stencil・Material と同じ並び）: ブラシのタブは組み込みのブラシ・直径・流量・
//! 不透明度・間隔・角度と、小見出し（ゆらぎ・テクスチャ・デュアルブラシ・色の変化・フェードと傾き・手ぶれ補正と入り抜き）、効果のブラシ。
//! アルファのタブは先端の画像（丸と組み込みの 7 つ）・硬さ・真円率・反転。値は全部入りのブラシ（`AppState::m2.brush`）と基本の値
//! （`AppState::brush`）を直に変え、ストロークを始めたときに写して固定する（途中で変えても、そのストロークには効かない）。

use std::collections::HashMap;

use egui::{
    pos2, vec2, Color32, ColorImage, Id, Rect, Sense, TextureHandle, TextureId, TextureOptions, Ui,
};

use super::properties::{
    choice_row, group_label, open_popup, pen_slider, percent_row, section, slider_row, status_row,
    subsection, toggle_row,
};
use crate::engine::{
    Brush, BrushEffect, ColorDynamics, Controls, DVec2, Jitter, StrokeAssist, TipShape,
};
use crate::lang::Lang;
use crate::m2::{
    self, dual_mode_label, preset_label, texture_mode_label, tip_label, BrushOp, EffectKind, UiOp,
};
use crate::m2_menu::Popup;
use crate::state::{Action, AppState, BrushState, Tool};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};

fn decimals(places: u8) -> NumberFormat<'static> {
    NumberFormat {
        decimals: places,
        trim: true,
        suffix: "",
    }
}

/// 2 列のチェック（左右）。変わったほうだけ新しい値を返す。
#[allow(clippy::too_many_arguments)]
fn toggle_pair(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    left: (&str, &str, bool),
    right: Option<(&str, &str, bool)>,
) -> (Option<bool>, Option<bool>) {
    let row = rows.row(t::ROW_HEIGHT, 2.0);
    let cols = Rows::split(row, 2, 6.0);
    let a = w::toggle(ui, cols[0], (id, 0), left.0, left.2, Some(left.1), true);
    let b = right.and_then(|right| {
        let b = w::toggle(ui, cols[1], (id, 1), right.0, right.2, Some(right.1), true);
        (b != right.2).then_some(b)
    });
    ((a != left.2).then_some(a), b)
}

fn reset_brush(app: &mut AppState) {
    let hardness = app.brush.hardness;
    app.brush = BrushState {
        hardness,
        ..BrushState::default()
    };
    app.m2.brush = Brush::default();
    app.m2.preset = None;
}

/// ブラシのタブ。
pub fn brush_tab(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context) {
    let lang = app.lang;
    let (open, reset) = section(
        ui,
        app,
        rows,
        "brush",
        lang.pick("ブラシ", "Brush"),
        "paint_brush",
        Some(lang.pick("ブラシの既定の値に戻す", "Reset the brush to its defaults")),
    );
    if reset {
        reset_brush(app);
    }
    if open {
        brush_basics(ui, app, rows, ctx, lang);
        jitter(ui, app, rows, lang);
        texture(ui, app, rows, ctx, lang);
        dual(ui, app, rows, ctx, lang);
        color_dynamics(ui, app, rows, lang);
        fade_and_tilt(ui, app, rows, lang);
        assist(ui, app, rows, lang);
    }
    rows.indent = 0.0;
    effect(ui, app, rows, ctx, lang);
    crate::selection::props::symmetry_section(ui, app, rows, lang);
}

fn brush_basics(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context, lang: Lang) {
    rows.indent = crate::ui::theme::SECTION_INDENT;
    // 組み込みのブラシ
    let preset = app
        .m2
        .preset
        .and_then(|i| m2::presets().get(i))
        .map(|p| preset_label(lang, p).0)
        .unwrap_or(lang.pick("カスタム", "Custom"));
    if let Some(b) = choice_row(
        ui,
        rows,
        "brush.preset",
        lang.pick("プリセット", "Preset"),
        preset,
        Some(lang.pick("組み込みのブラシ", "Built-in brushes")),
        true,
    ) {
        open_popup(app, ctx, Popup::Preset, b, b.width());
    }
    let b = &mut app.brush;
    let spec = SliderSpec::new(
        lang.pick("直径", "Size"),
        1.0,
        256.0,
        NumberFormat::int(" px"),
    )
    .tooltip(lang.pick("ブラシの直径（[ と ]）", "Brush diameter ([ and ])"));
    if let Some(v) = pen_slider(
        ui,
        rows,
        "brush.size",
        spec,
        b.radius * 2.0,
        &mut b.pressure_size,
        lang.pick("筆圧で直径を変える", "Pen pressure changes the size"),
    ) {
        b.radius = (v / 2.0).max(0.5);
    }
    let spec = SliderSpec::new(
        lang.pick("流量", "Flow"),
        0.0,
        100.0,
        NumberFormat::int("%"),
    )
    .tooltip(lang.pick("ダブ 1 つが足す量", "How much each dab adds"));
    if let Some(v) = pen_slider(
        ui,
        rows,
        "brush.flow",
        spec,
        b.flow * 100.0,
        &mut b.pressure_flow,
        lang.pick("筆圧で流量を変える", "Pen pressure changes the flow"),
    ) {
        b.flow = v / 100.0;
    }
    let spec = SliderSpec::new(
        lang.pick("不透明度", "Opacity"),
        0.0,
        100.0,
        NumberFormat::int("%"),
    )
    .tooltip(lang.pick(
        "1 本のストロークが覆える上限",
        "The most one stroke can cover",
    ));
    if let Some(v) = pen_slider(
        ui,
        rows,
        "brush.opacity",
        spec,
        b.opacity * 100.0,
        &mut b.pressure_opacity,
        lang.pick("筆圧で不透明度を変える", "Pen pressure changes the opacity"),
    ) {
        b.opacity = v / 100.0;
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "brush.spacing",
        lang.pick("間隔", "Spacing"),
        b.spacing * 100.0,
        (1.0, 100.0),
        NumberFormat::int("%"),
        Some(lang.pick(
            "ダブの間隔（直径に対する割合）",
            "Distance between dabs (of the diameter)",
        )),
        true,
    ) {
        b.spacing = v / 100.0;
    }
    if app.view3d.paintable_on_screen() {
        status_row(
            ui,
            rows,
            lang.pick("角度は 3D では効きません", "No effect on angle in 3D"),
        );
    }
    let tip = &mut app.m2.brush.tip;
    if let Some(v) = slider_row(
        ui,
        rows,
        "brush.angle",
        lang.pick("角度", "Angle"),
        tip.angle as f32,
        (-180.0, 180.0),
        NumberFormat::int("°"),
        Some(lang.pick(
            "先端の回転（反時計回り）",
            "Tip rotation (counterclockwise)",
        )),
        true,
    ) {
        tip.angle = v as f64;
    }
    if let Some(v) = toggle_row(
        ui,
        rows,
        "brush.follow",
        lang.pick("線の向きに従う", "Follow direction"),
        tip.follow_direction,
        Some(lang.pick(
            "線の向きを先端の角度に足す",
            "Adds the stroke direction to the tip angle",
        )),
        true,
    ) {
        tip.follow_direction = v;
    }
}

/// 3D のビューが出ているあいだ、面のダブが使わない設定に短い理由を出す。3D のストロークは基本の値（直径・硬さ・流量・不透明度・
/// 筆圧）と色・色の変化・消しゴムだけを使い、筆先・ゆらぎ・質感・デュアル・フェード・傾き・回転・速さ・手ぶれ補正は受け取らない。
fn no_effect_in_3d(ui: &mut Ui, app: &AppState, rows: &mut Rows, lang: Lang) {
    if app.view3d.paintable_on_screen() {
        status_row(ui, rows, lang.pick("3D では効きません", "No effect in 3D"));
    }
}

fn jitter(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
    let (open, reset) = subsection(
        ui,
        app,
        rows,
        "brush-jitter",
        lang.pick("ゆらぎ", "Jitter"),
        Some(lang.pick("ゆらぎを既定に戻す", "Reset the jitter")),
    );
    if reset {
        app.m2.brush.jitter = Jitter::default();
    }
    if !open {
        return;
    }
    no_effect_in_3d(ui, app, rows, lang);
    let j = &mut app.m2.brush.jitter;
    let tips = [
        lang.pick(
            "大きさを最大でこの割合だけ小さくする",
            "Shrinks the size by up to this much",
        ),
        lang.pick(
            "角度を最大 ±180° × これだけ回す",
            "Rotates by up to ±180° × this",
        ),
        lang.pick(
            "真円率を最大でこの割合だけ潰す",
            "Squashes the roundness by up to this much",
        ),
        lang.pick(
            "不透明度を最大でこの割合だけ下げる",
            "Lowers the opacity by up to this much",
        ),
        lang.pick(
            "流量を最大でこの割合だけ下げる",
            "Lowers the flow by up to this much",
        ),
    ];
    let unit = (0.0, 1.0);
    if let Some(v) = percent_row(
        ui,
        rows,
        "jitter.size",
        lang.pick("サイズ", "Size"),
        j.size,
        unit,
        Some(tips[0]),
        true,
    ) {
        j.size = v;
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "jitter.angle",
        lang.pick("角度", "Angle"),
        j.angle,
        unit,
        Some(tips[1]),
        true,
    ) {
        j.angle = v;
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "jitter.roundness",
        lang.pick("真円率", "Roundness"),
        j.roundness,
        unit,
        Some(tips[2]),
        true,
    ) {
        j.roundness = v;
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "jitter.opacity",
        lang.pick("不透明度", "Opacity"),
        j.opacity,
        unit,
        Some(tips[3]),
        true,
    ) {
        j.opacity = v;
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "jitter.flow",
        lang.pick("流量", "Flow"),
        j.flow,
        unit,
        Some(tips[4]),
        true,
    ) {
        j.flow = v;
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "jitter.scatter",
        lang.pick("散布", "Scatter"),
        j.scatter,
        (0.0, 10.0),
        Some(lang.pick(
            "位置を直径の何倍までずらすか",
            "How far dabs spread, in diameters",
        )),
        true,
    ) {
        j.scatter = v;
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "jitter.count",
        lang.pick("数", "Count"),
        j.count as f32,
        (1.0, 16.0),
        NumberFormat::int(""),
        Some(lang.pick(
            "1 つの間隔に置くダブの数",
            "Dabs placed at every spacing step",
        )),
        true,
    ) {
        j.count = v.round().clamp(1.0, 16.0) as u32;
    }
}

fn texture(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context, lang: Lang) {
    let (open, reset) = subsection(
        ui,
        app,
        rows,
        "brush-texture",
        lang.pick("テクスチャ", "Texture"),
        Some(lang.pick("テクスチャを既定に戻す", "Reset the texture")),
    );
    if reset {
        app.m2.brush.texture = None;
        app.m2.texture_prefs = m2::TexturePrefs::default();
    }
    if !open {
        return;
    }
    no_effect_in_3d(ui, app, rows, lang);
    let name = app
        .m2
        .brush
        .texture
        .as_ref()
        .map(|t| tip_label(lang, t.image.name()))
        .unwrap_or(lang.pick("なし", "None"));
    if let Some(b) = choice_row(
        ui,
        rows,
        "texture.image",
        lang.pick("画像", "Image"),
        name,
        Some(lang.pick("紙の質感", "Paper texture")),
        true,
    ) {
        open_popup(app, ctx, Popup::Texture, b, b.width());
    }
    let mode = app.m2.brush.texture.as_ref().map(|t| t.mode);
    if let Some(mode) = mode {
        if let Some(t) = app.m2.brush.texture.as_mut() {
            if let Some(v) = percent_row(
                ui,
                rows,
                "texture.depth",
                lang.pick("深さ", "Depth"),
                t.depth,
                (0.0, 1.0),
                Some(lang.pick("質感の効き", "How much the texture shows")),
                true,
            ) {
                t.depth = v;
            }
            if let Some(v) = slider_row(
                ui,
                rows,
                "texture.scale",
                lang.pick("スケール", "Scale"),
                t.scale as f32,
                (0.05, 16.0),
                decimals(2),
                Some(lang.pick(
                    "質感 1 画素あたりの画布の画素",
                    "Canvas pixels per texture pixel",
                )),
                true,
            ) {
                t.scale = v as f64;
            }
        }
        if let Some(b) = choice_row(
            ui,
            rows,
            "texture.mode",
            lang.pick("モード", "Mode"),
            texture_mode_label(lang, mode),
            Some(lang.pick("質感の合わせ方", "How the texture combines")),
            true,
        ) {
            open_popup(app, ctx, Popup::TextureMode, b, b.width());
        }
    }
}

fn dual(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context, lang: Lang) {
    let (open, reset) = subsection(
        ui,
        app,
        rows,
        "brush-dual",
        lang.pick("デュアルブラシ", "Dual Brush"),
        Some(lang.pick("デュアルブラシを既定に戻す", "Reset the dual brush")),
    );
    if reset {
        app.m2.brush.dual = None;
        app.m2.dual_stash = Default::default();
    }
    if !open {
        return;
    }
    no_effect_in_3d(ui, app, rows, lang);
    let on = app.m2.brush.dual.is_some();
    if let Some(v) = toggle_row(
        ui,
        rows,
        "dual.enabled",
        lang.pick("デュアルブラシを使う", "Use a dual brush"),
        on,
        Some(lang.pick(
            "同じ道筋の 2 つ目の筆先が、主の筆先の覆いを削る",
            "A second tip along the same path masks the main tip",
        )),
        true,
    ) {
        app.apply(Action::M2Ui(UiOp::Brush(BrushOp::DualEnabled(v))));
    }
    let Some(d) = app.m2.brush.dual.as_ref() else {
        return;
    };
    let tip_name = d
        .tip
        .as_ref()
        .map(|t| tip_label(lang, t.name()))
        .unwrap_or(lang.pick("丸（硬さ）", "Round (hardness)"));
    let round = d.tip.is_none();
    let mode = d.mode;
    if let Some(b) = choice_row(
        ui,
        rows,
        "dual.tip",
        lang.pick("先端", "Tip"),
        tip_name,
        None,
        true,
    ) {
        open_popup(app, ctx, Popup::DualTip, b, b.width());
    }
    if let Some(b) = choice_row(
        ui,
        rows,
        "dual.mode",
        lang.pick("モード", "Mode"),
        dual_mode_label(lang, mode),
        Some(lang.pick(
            "2 つ目の筆先の覆いの合わせ方",
            "How the second tip combines",
        )),
        true,
    ) {
        open_popup(app, ctx, Popup::DualMode, b, b.width());
    }
    let Some(d) = app.m2.brush.dual.as_mut() else {
        return;
    };
    if let Some(v) = slider_row(
        ui,
        rows,
        "dual.size",
        lang.pick("直径", "Size"),
        (d.radius * 2.0) as f32,
        (1.0, 256.0),
        NumberFormat::int(" px"),
        None,
        true,
    ) {
        d.radius = (v as f64 / 2.0).max(0.5);
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "dual.hardness",
        lang.pick("硬さ", "Hardness"),
        d.hardness,
        (0.0, 1.0),
        None,
        round,
    ) {
        d.hardness = v;
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "dual.spacing",
        lang.pick("間隔", "Spacing"),
        d.spacing,
        (0.01, 4.0),
        None,
        true,
    ) {
        d.spacing = v;
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "dual.angle",
        lang.pick("角度", "Angle"),
        d.angle as f32,
        (-180.0, 180.0),
        NumberFormat::int("°"),
        None,
        true,
    ) {
        d.angle = v as f64;
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "dual.roundness",
        lang.pick("真円率", "Roundness"),
        d.roundness,
        (0.01, 1.0),
        None,
        true,
    ) {
        d.roundness = v;
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "dual.scatter",
        lang.pick("散布", "Scatter"),
        d.scatter,
        (0.0, 10.0),
        None,
        true,
    ) {
        d.scatter = v;
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "dual.count",
        lang.pick("数", "Count"),
        d.count as f32,
        (1.0, 16.0),
        NumberFormat::int(""),
        None,
        true,
    ) {
        d.count = v.round().clamp(1.0, 16.0) as u32;
    }
}

fn color_dynamics(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
    let (open, reset) = subsection(
        ui,
        app,
        rows,
        "brush-color",
        lang.pick("色の変化", "Color Dynamics"),
        Some(lang.pick("色の変化を既定に戻す", "Reset the color dynamics")),
    );
    if reset {
        app.m2.brush.color = ColorDynamics::default();
    }
    if !open {
        return;
    }
    let kind = app
        .doc
        .channel_info(app.m2.paint_channel)
        .map(|i| i.kind)
        .unwrap_or(crate::engine::ChannelKind::Color);
    if app.m2.edit_mask || !yolu_core::brush::carries_color(kind) {
        status_row(
            ui,
            rows,
            lang.pick("このチャンネルでは効きません", "No effect on this channel"),
        );
    }
    // 背景色（カラーのパネルのサブの色）
    let row = rows.row(t::ROW_HEIGHT, 2.0);
    let swatch = Rect::from_min_size(row.min + vec2(0.0, 2.0), vec2(30.0, row.height() - 4.0));
    w::color_swatch(
        ui,
        swatch,
        "color.secondary",
        app.color.sub,
        lang.pick(
            "背景色（カラーのパネルのサブの色）。描画色との間でゆらぐ",
            "Background color (the Color panel's secondary color); dabs vary between it and the paint color",
        ),
        true,
    );
    w::text(
        ui.painter(),
        Rect::from_min_max(pos2(swatch.right() + 8.0, row.top()), row.max),
        lang.pick("背景色", "Background"),
        t::LABEL,
        w::Align::Left,
    );
    let c = &mut app.m2.brush.color;
    let unit = (0.0, 1.0);
    if let Some(v) = percent_row(
        ui,
        rows,
        "color.fgbg",
        lang.pick("前景/背景", "Fg/Bg jitter"),
        c.foreground_background,
        unit,
        Some(lang.pick(
            "ダブごとに背景色へ寄る量の上限",
            "Each dab mixes toward the background by up to this",
        )),
        true,
    ) {
        c.foreground_background = v;
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "color.hue",
        lang.pick("色相", "Hue"),
        c.hue,
        unit,
        Some(lang.pick(
            "色相が ± これ × 180° まで動く",
            "The hue moves by up to ± this × 180°",
        )),
        true,
    ) {
        c.hue = v;
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "color.saturation",
        lang.pick("彩度", "Saturation"),
        c.saturation,
        unit,
        Some(lang.pick("彩度が ± これまで動く", "Saturation moves by up to ± this")),
        true,
    ) {
        c.saturation = v;
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "color.brightness",
        lang.pick("明るさ", "Brightness"),
        c.brightness,
        unit,
        Some(lang.pick("明るさが ± これまで動く", "Value moves by up to ± this")),
        true,
    ) {
        c.brightness = v;
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "color.purity",
        lang.pick("純度", "Purity"),
        c.purity,
        (-1.0, 1.0),
        Some(lang.pick(
            "−100% で灰色、0 でそのまま、100% で彩度いっぱい",
            "-100% gray, 0 unchanged, 100% fully saturated",
        )),
        true,
    ) {
        c.purity = v;
    }
    if let Some(v) = toggle_row(
        ui,
        rows,
        "color.per_tip",
        lang.pick("描点ごとに適用", "Apply per tip"),
        c.per_tip,
        Some(lang.pick(
            "ダブごとに新しい色。切ると 1 本のストロークに 1 色",
            "A new color for every dab. Off: one color per stroke",
        )),
        true,
    ) {
        c.per_tip = v;
    }
}

fn fade_and_tilt(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
    let (open, reset) = subsection(
        ui,
        app,
        rows,
        "brush-fade",
        lang.pick("フェードと傾き", "Fade & Tilt"),
        Some(lang.pick("フェードと傾きを既定に戻す", "Reset fade and tilt")),
    );
    if reset {
        app.m2.brush.controls = Controls::default();
    }
    if !open {
        return;
    }
    no_effect_in_3d(ui, app, rows, lang);
    let k = &mut app.m2.brush.controls;
    group_label(ui, rows, lang.pick("フェード（描点の数）", "Fade (dabs)"));
    let fade_tip = lang.pick(
        "ストロークの始めからこの数のダブで 1 から 0 へ",
        "Falls from full to nothing over this many dabs",
    );
    for (id, label, field) in [
        ("fade.size", lang.pick("サイズ", "Size"), &mut k.fade_size),
        (
            "fade.opacity",
            lang.pick("不透明度", "Opacity"),
            &mut k.fade_opacity,
        ),
        ("fade.flow", lang.pick("流量", "Flow"), &mut k.fade_flow),
    ] {
        if let Some(v) = slider_row(
            ui,
            rows,
            id,
            label,
            *field as f32,
            (0.0, 2000.0),
            NumberFormat::int(""),
            Some(fade_tip),
            true,
        ) {
            *field = v.round().max(0.0) as u32;
        }
    }
    let pen = lang.pick("マウスでは効きません", "No effect with a mouse");
    group_label(ui, rows, lang.pick("ペンの傾き", "Pen tilt"));
    let (a, b) = toggle_pair(
        ui,
        rows,
        "tilt.a",
        (lang.pick("サイズ", "Size"), pen, k.tilt_size),
        Some((lang.pick("不透明度", "Opacity"), pen, k.tilt_opacity)),
    );
    if let Some(v) = a {
        k.tilt_size = v;
    }
    if let Some(v) = b {
        k.tilt_opacity = v;
    }
    let (a, b) = toggle_pair(
        ui,
        rows,
        "tilt.b",
        (lang.pick("流量", "Flow"), pen, k.tilt_flow),
        Some((
            lang.pick("角度", "Angle"),
            lang.pick(
                "倒れた向きを先端の角度に足す",
                "Adds the lean direction to the tip angle",
            ),
            k.tilt_angle,
        )),
    );
    if let Some(v) = a {
        k.tilt_flow = v;
    }
    if let Some(v) = b {
        k.tilt_angle = v;
    }
    group_label(ui, rows, lang.pick("ペンの回転", "Pen rotation"));
    if let Some(v) = toggle_row(
        ui,
        rows,
        "rotation.angle",
        lang.pick("角度に足す", "Add to angle"),
        k.rotation_angle,
        Some(lang.pick(
            "ペンの軸の回転を先端の角度に足す（2D のキャンバスだけ）。回転を送れないペンとマウスでは 0",
            "Adds the pen's barrel rotation to the tip angle (2D canvas only); 0 for a mouse or a pen without rotation",
        )),
        true,
    ) {
        k.rotation_angle = v;
    }
    group_label(ui, rows, lang.pick("筆の速さ", "Speed"));
    let speed = lang.pick(
        "手ぶれ補正の後の筆の速さ。時刻を持つ入力（ペン・マウス）で、2D のキャンバスだけ",
        "Brush speed after the stabilizer; needs timed input (pen, mouse), 2D canvas only",
    );
    let (a, b) = toggle_pair(
        ui,
        rows,
        "speed.a",
        (lang.pick("サイズ", "Size"), speed, k.speed_size),
        Some((lang.pick("不透明度", "Opacity"), speed, k.speed_opacity)),
    );
    if let Some(v) = a {
        k.speed_size = v;
    }
    if let Some(v) = b {
        k.speed_opacity = v;
    }
    let (a, _) = toggle_pair(
        ui,
        rows,
        "speed.b",
        (lang.pick("流量", "Flow"), speed, k.speed_flow),
        None,
    );
    if let Some(v) = a {
        k.speed_flow = v;
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "speed.max",
        lang.pick("効きが最大の速さ", "Full-effect speed"),
        k.speed_max as f32,
        (100.0, 10000.0),
        NumberFormat::int(" px/s"),
        Some(lang.pick(
            "この速さで効きが一杯になる（1 秒あたりの画素）",
            "The speed at which the effect is complete (pixels per second)",
        )),
        true,
    ) {
        k.speed_max = v as f64;
    }
}

fn assist(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
    let (open, reset) = subsection(
        ui,
        app,
        rows,
        "brush-assist",
        lang.pick("手ぶれ補正と入り抜き", "Stabilizer & Taper"),
        Some(lang.pick("補正を既定に戻す", "Reset the stabilizer and taper")),
    );
    if reset {
        app.m2.brush.assist = StrokeAssist::default();
    }
    if !open {
        return;
    }
    no_effect_in_3d(ui, app, rows, lang);
    let a = &mut app.m2.brush.assist;
    if let Some(v) = slider_row(
        ui,
        rows,
        "assist.stabilizer",
        lang.pick("手ぶれ補正", "Stabilizer"),
        a.stabilizer as f32,
        (0.0, 200.0),
        NumberFormat::int(" px"),
        Some(lang.pick(
            "筆が入力に引かれる糸の長さ。0 で切",
            "Length of the string that pulls the brush. 0 = off",
        )),
        true,
    ) {
        a.stabilizer = v as f64;
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "assist.taper_in",
        lang.pick("入り", "Taper in"),
        a.taper_in as f32,
        (0.0, 500.0),
        NumberFormat::int(" px"),
        Some(lang.pick(
            "線の始めでこの長さをかけて太くなる",
            "The stroke grows over this length",
        )),
        true,
    ) {
        a.taper_in = v as f64;
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "assist.taper_out",
        lang.pick("抜き", "Taper out"),
        a.taper_out as f32,
        (0.0, 500.0),
        NumberFormat::int(" px"),
        Some(lang.pick(
            "線の終わりでこの長さをかけて細くなる",
            "The stroke thins over this length",
        )),
        true,
    ) {
        a.taper_out = v as f64;
    }
    if let Some(v) = toggle_row(
        ui,
        rows,
        "assist.curve",
        lang.pick("曲線", "Curve"),
        a.curve,
        Some(lang.pick(
            "入力の点を滑らかな曲線で結ぶ",
            "Joins the input points with a smooth curve",
        )),
        true,
    ) {
        a.curve = v;
    }
}

/// 効果のブラシ（ぼかし・指先・クローン）。
fn effect(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context, lang: Lang) {
    let (open, reset) = section(
        ui,
        app,
        rows,
        "brush-effect",
        lang.pick("効果", "Effect"),
        "blur_on",
        Some(lang.pick("効果をペイントに戻す", "Back to painting")),
    );
    if reset {
        app.m2.brush.effect = BrushEffect::Paint;
    }
    if !open {
        return;
    }
    let kind = EffectKind::of(&app.m2.brush.effect);
    let usable = app.tool != Tool::Eraser;
    if let Some(b) = choice_row(
        ui,
        rows,
        "effect.kind",
        lang.pick("種類", "Type"),
        kind.name(lang),
        Some(lang.pick(
            "色を塗らず、今の画素から読んだ色を混ぜる。消しゴムでは使えない",
            "Mixes colors read from the layer instead of painting. Not with the eraser",
        )),
        true,
    ) {
        open_popup(app, ctx, Popup::Effect, b, b.width());
    }
    match &mut app.m2.brush.effect {
        BrushEffect::Paint => {}
        BrushEffect::Blur { radius } => {
            if let Some(v) = slider_row(
                ui,
                rows,
                "effect.blur",
                lang.pick("半径", "Radius"),
                *radius as f32,
                (1.0, 64.0),
                NumberFormat::int(" px"),
                None,
                usable,
            ) {
                *radius = v.round().clamp(1.0, 64.0) as u32;
            }
        }
        BrushEffect::Smudge { strength } => {
            if let Some(v) = percent_row(
                ui,
                rows,
                "effect.smudge",
                lang.pick("強さ", "Strength"),
                *strength,
                (0.0, 1.0),
                Some(lang.pick("流量に掛ける強さ", "Multiplies the flow")),
                usable,
            ) {
                *strength = v;
            }
        }
        BrushEffect::Clone { offset } => {
            let mut next = *offset;
            if let Some(v) = slider_row(
                ui,
                rows,
                "effect.clone.x",
                lang.pick("ずれ X", "Offset X"),
                offset.x as f32,
                (-2048.0, 2048.0),
                NumberFormat::int(" px"),
                Some(lang.pick(
                    "コピー元までの横の距離",
                    "Horizontal distance to the source",
                )),
                usable,
            ) {
                next.x = v as f64;
            }
            if let Some(v) = slider_row(
                ui,
                rows,
                "effect.clone.y",
                lang.pick("ずれ Y", "Offset Y"),
                offset.y as f32,
                (-2048.0, 2048.0),
                NumberFormat::int(" px"),
                Some(lang.pick(
                    "コピー元までの縦の距離（上が正）",
                    "Vertical distance to the source (up is positive)",
                )),
                usable,
            ) {
                next.y = v as f64;
            }
            *offset = DVec2::new(next.x, next.y);
            // 見えているレイヤーの重なりを読む・3D の面のクローンの揃え方（3D のビューを出しているとき）。
            // マスクを描くあいだは描いているマスクだけを読むので、入れても効かない切り替えは薄くして切った表示にする
            let masked = app.m2.edit_mask;
            let clone = &mut app.view3d.clone;
            if let Some(v) = toggle_row(
                ui,
                rows,
                "effect.clone.all-layers",
                lang.pick("全レイヤーから", "All layers"),
                clone.all_layers && !masked,
                Some(lang.pick(
                    if masked {
                        "マスクを描くあいだは、描いているマスクだけを読む"
                    } else {
                        "描くレイヤーだけでなく、見えているレイヤーの重なりを読む"
                    },
                    if masked {
                        "While painting a mask, only that mask is read"
                    } else {
                        "Read the visible layers together, not only the layer being painted"
                    },
                )),
                usable && !masked,
            ) {
                clone.all_layers = v;
            }
            if app.view3d.paintable_on_screen() {
                // 元の有無は文では言わず、揃えるの薄さで示す（元は 3D ビューの十字で見える。決めるのは Alt クリック）
                let has_source = app
                    .view3d
                    .model
                    .as_ref()
                    .is_some_and(|m| app.view3d.clone.source_for(&m.geometry).is_some());
                let clone = &mut app.view3d.clone;
                if let Some(v) = toggle_row(
                    ui,
                    rows,
                    "effect.clone.aligned",
                    lang.pick("揃える", "Aligned"),
                    clone.aligned,
                    Some(lang.pick(
                        if has_source {
                            "3D: 前のストロークと同じ位置関係で続ける。切ると、ストロークごとに最初の点が元に重なる"
                        } else {
                            "3D: 元を決めると使える（Alt を押しながらクリック）"
                        },
                        if has_source {
                            "3D: Keep the offset from the previous stroke. Off: every stroke starts on the source"
                        } else {
                            "3D: Available once a source is set (Alt+click)"
                        },
                    )),
                    usable && has_source,
                ) {
                    clone.set_aligned(v);
                }
            }
        }
    }
}

// ───────── アルファ（先端の画像） ─────────

/// 先端の見本の大きさ（点）と、一覧の 1 マス。
const THUMB: usize = 48;
const CELL: f32 = 40.0;
const CELL_GAP: f32 = 4.0;

/// 先端の画像の見本（白地に黒。丸い先端は縁がやわらかい円）。
fn tip_image(id: Option<&str>) -> ColorImage {
    let tip = id.and_then(yolu_core::builtin_tip);
    let n = THUMB;
    let mut pixels = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            let fx = (x as f64 + 0.5) / n as f64 - 0.5;
            let fy = 0.5 - (y as f64 + 0.5) / n as f64; // 画像の上が先端の上（行は下から）
            let coverage = match &tip {
                Some(t) => {
                    // 長い辺を見本の幅に合わせて、縦横比を保つ
                    let aspect = t.width() as f64 / t.height() as f64;
                    let (u, v) = if aspect >= 1.0 {
                        (0.5 + fx, 0.5 + fy * aspect)
                    } else {
                        (0.5 + fx / aspect, 0.5 + fy)
                    };
                    t.sample(u, v)
                }
                None => {
                    let d = (fx * fx + fy * fy).sqrt() * 2.0;
                    ((0.95 - d) / 0.4).clamp(0.0, 1.0)
                }
            };
            pixels.push(Color32::from_gray((255.0 * (1.0 - coverage)).round() as u8));
        }
    }
    ColorImage::new([n, n], pixels)
}

#[derive(Clone, Default)]
struct TipThumbs(HashMap<&'static str, TextureHandle>);

/// 先端の見本の絵（初めて出すときに作って、文脈に覚えておく）。None は丸。
fn tip_texture(ctx: &egui::Context, id: Option<&'static str>) -> TextureId {
    let cache_id = Id::new("yolu.tip-thumbs");
    let mut cache: TipThumbs = ctx.data(|d| d.get_temp(cache_id)).unwrap_or_default();
    let key = id.unwrap_or("round");
    if !cache.0.contains_key(key) {
        let handle = ctx.load_texture(format!("tip-{key}"), tip_image(id), TextureOptions::LINEAR);
        cache.0.insert(key, handle);
        ctx.data_mut(|d| d.insert_temp(cache_id, cache.clone()));
    }
    cache.0[key].id()
}

fn tip_cell(ui: &mut Ui, r: Rect, id: Option<&'static str>, selected: bool, tooltip: &str) -> bool {
    let key = id.unwrap_or("round");
    let response = ui.interact(r, ui.make_persistent_id(("alpha.tip", key)), Sense::click());
    let texture = tip_texture(ui.ctx(), id);
    let p = ui.painter();
    w::rounded(p, r, Color32::WHITE, 3.0);
    p.image(
        texture,
        r.shrink(2.0),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    if selected {
        w::outline(p, r.expand(2.0), t::ACCENT, 2.0, 4.0);
    } else if response.hovered() {
        w::outline(p, r.expand(1.0), t::ACCENT_DIM, 1.0, 4.0);
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, tooltip)
    });
    let clicked = response.clicked();
    let _ = response.on_hover_text(tooltip);
    clicked
}

/// アルファのタブ。
pub fn alpha_tab(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context) {
    let lang = app.lang;
    let (open, reset) = section(
        ui,
        app,
        rows,
        "alpha",
        lang.pick("アルファ", "Alpha"),
        "shapes",
        Some(lang.pick("先端の形を既定に戻す", "Reset the tip shape")),
    );
    if reset {
        app.brush.hardness = BrushState::default().hardness;
        let (angle, follow) = (app.m2.brush.tip.angle, app.m2.brush.tip.follow_direction);
        app.m2.brush.tip = TipShape {
            angle,
            follow_direction: follow,
            ..TipShape::default()
        };
    }
    if !open {
        return;
    }
    if app.view3d.paintable_on_screen() {
        status_row(
            ui,
            rows,
            lang.pick(
                "硬さ以外は 3D では効きません",
                "Only hardness applies in 3D",
            ),
        );
    }
    let current: Option<&'static str> = app.m2.brush.tip.image.as_ref().and_then(|t| {
        yolu_core::brush::BUILTIN_TIPS
            .into_iter()
            .find(|id| *id == t.name())
    });
    let round = app.m2.brush.tip.image.is_none();
    // 今の先端: 見本と名前
    let head = rows.row(48.0, 6.0);
    let swatch = Rect::from_min_size(head.min, vec2(48.0, 48.0));
    let p = ui.painter();
    w::rounded(p, swatch, Color32::WHITE, 3.0);
    p.image(
        tip_texture(ctx, current),
        swatch.shrink(2.0),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    let name = current
        .map(|id| tip_label(lang, id))
        .unwrap_or(lang.pick("丸（硬さ）", "Round (hardness)"));
    let from = if round {
        lang.pick("先端の形", "Tip shape")
    } else {
        lang.pick("組み込みの画像", "Built-in image")
    };
    let tx = head.left() + 56.0;
    let tw = head.width() - 56.0;
    w::text(
        ui.painter(),
        Rect::from_min_size(pos2(tx, head.top() + 6.0), vec2(tw, 18.0)),
        name,
        t::LABEL_BOLD,
        w::Align::Left,
    );
    w::text(
        ui.painter(),
        Rect::from_min_size(pos2(tx, head.top() + 26.0), vec2(tw, 16.0)),
        from,
        t::LABEL_DIM,
        w::Align::Left,
    );
    // 形の設定
    if let Some(v) = percent_row(
        ui,
        rows,
        "alpha.hardness",
        lang.pick("硬さ", "Hardness"),
        app.brush.hardness as f64,
        (0.0, 1.0),
        Some(lang.pick(
            "丸い先端の縁の硬さ（画像の先端は画像の縁のまま）",
            "Edge hardness of the round tip (an image keeps its own edge)",
        )),
        round,
    ) {
        app.brush.hardness = v as f32;
    }
    let tip = &mut app.m2.brush.tip;
    if let Some(v) = percent_row(
        ui,
        rows,
        "alpha.roundness",
        lang.pick("真円率", "Roundness"),
        tip.roundness,
        (0.01, 1.0),
        Some(lang.pick(
            "先端の角度に沿って潰す（100% で潰さない）",
            "Squashes the tip along its angle (100% keeps its shape)",
        )),
        true,
    ) {
        tip.roundness = v;
    }
    let image = tip.image.is_some();
    let flip_tip = lang.pick("画像の先端を反転する", "Mirrors an image tip");
    let (a, b) = toggle_pair(
        ui,
        rows,
        "alpha.flip",
        (lang.pick("左右反転", "Flip X"), flip_tip, tip.flip_x),
        Some((lang.pick("上下反転", "Flip Y"), flip_tip, tip.flip_y)),
    );
    if image {
        if let Some(v) = a {
            tip.flip_x = v;
        }
        if let Some(v) = b {
            tip.flip_y = v;
        }
    }
    // 先端の一覧（丸と組み込みの画像）
    group_label(ui, rows, lang.pick("組み込み", "Built-in"));
    let ids: Vec<Option<&'static str>> = std::iter::once(None)
        .chain(yolu_core::brush::BUILTIN_TIPS.into_iter().map(Some))
        .collect();
    let columns = (((rows.width() + CELL_GAP) / (CELL + CELL_GAP)).floor() as usize).max(1);
    for chunk in ids.chunks(columns) {
        let line = rows.row(CELL, CELL_GAP);
        for (k, id) in chunk.iter().enumerate() {
            let cell = Rect::from_min_size(
                pos2(line.left() + k as f32 * (CELL + CELL_GAP), line.top()),
                vec2(CELL, CELL),
            );
            let tip_name = id
                .map(|i| tip_label(lang, i))
                .unwrap_or(lang.pick("丸（硬さ）", "Round (hardness)"));
            if tip_cell(ui, cell, *id, *id == current, tip_name) {
                app.apply(Action::M2Ui(UiOp::Brush(BrushOp::Tip(*id))));
            }
        }
    }
}

/// ステンシルのタブ（中身は `stencil_props`）。
pub fn stencil_tab(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let ctx = ui.ctx().clone();
    super::stencil_props::stencil_tab(ui, app, rows, &ctx);
}

pub fn material_tab(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    super::material::material_tab(ui, app, rows);
}
