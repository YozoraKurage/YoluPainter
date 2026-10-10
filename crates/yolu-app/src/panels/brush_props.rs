//! ブラシの欄。詳細のウィンドウ（`brush_detail`）が、左のカテゴリ（形状・ストローク・筆圧・入り抜きとペン・ゆらぎ・テクスチャ・デュアルブラシ・色の揺らぎ・
//! 効果・対称）ごとにここの欄を出す。筆先の形（画像・硬さ・真円率・角度・反転・組み込みの一覧）は「形状」の欄。
//! 値は全部入りのブラシ（`AppState::m2.brush`）と基本の値（`AppState::brush`）を直に変え、ストロークを始めたときに写して固定する
//! （途中で変えても、そのストロークには効かない）。3D のビューの面のダブも同じ欄を 2D と同じ式で使う（長さの欄は、3D では画面の点）。

use std::collections::HashMap;

use egui::{
    pos2, vec2, Color32, ColorImage, Id, Rect, Sense, TextureHandle, TextureId, TextureOptions, Ui,
};

use super::properties::{choice_row, group_label, open_popup, percent_row, slider_row, toggle_row};

/// まとまりの小見出し（前の行との間を少し空ける）。
fn group(ui: &mut Ui, rows: &mut Rows, text: &str) {
    rows.space(5.0);
    group_label(ui, rows, text);
}
use super::tip_library;
use crate::brushes::Category;
use crate::engine::{
    BrushEffect, ColorDynamics, ColorMix, Controls, DVec2, Jitter, MixGround, MixMode,
    PressureResponse, PressureResponses, TipShape,
};
use crate::lang::Lang;
use crate::m2::{
    self, anti_alias_label, dual_mode_label, texture_mode_label, tip_label, BrushOp, EffectKind,
    UiOp,
};
use crate::m2_menu::Popup;
use crate::state::{Action, AppState, BrushState, Tool};
use crate::ui::curve;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows};
use yolu_core::curve::Curve;

fn decimals(places: u8) -> NumberFormat<'static> {
    NumberFormat {
        decimals: places,
        trim: true,
        suffix: "",
    }
}

/// 硬さが効くか。丸い筆先の縁の硬さなので、画像の筆先では効かない（画像の縁のまま。2D も 3D の面のダブも同じ）。ツールプロパティと
/// 形状の欄が同じ判定を使う。
pub fn hardness_applies(app: &AppState) -> bool {
    let tip = &app.m2.brush.tip;
    tip.image.is_none() && tip.images.is_empty()
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
    parts.push(lang.pick("クリック", "Click"));
    Some(parts.join("+"))
}

/// 2 列のチェック（左右。名前が列に収まらないほど狭ければ縦に 2 段）。変わったほうだけ新しい値を返す。無効なら押せない。
fn toggle_pair(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    enabled: bool,
    left: (&str, &str, bool),
    right: Option<(&str, &str, bool)>,
) -> (Option<bool>, Option<bool>) {
    let row = rows.row(t::ROW_HEIGHT, 2.0);
    let cols = Rows::split(row, 2, 6.0);
    // チェックの箱と間の分（23 点）を足して、名前が列に収まるか
    let fits =
        |label: &str, width: f32| w::text_width(ui.painter(), label, t::LABEL) + 23.0 <= width;
    let side_by_side =
        right.is_none_or(|r| fits(left.0, cols[0].width()) && fits(r.0, cols[1].width()));
    let a = w::toggle(
        ui,
        if side_by_side || right.is_none() {
            cols[0]
        } else {
            row
        },
        (id, 0),
        left.0,
        left.2,
        Some(left.1).filter(|t| !t.is_empty()),
        enabled,
    );
    let b = right.and_then(|right| {
        let at = if side_by_side {
            cols[1]
        } else {
            rows.row(t::ROW_HEIGHT, 2.0)
        };
        let b = w::toggle(
            ui,
            at,
            (id, 1),
            right.0,
            right.2,
            Some(right.1).filter(|t| !t.is_empty()),
            enabled,
        );
        (b != right.2).then_some(b)
    });
    ((a != left.2).then_some(a), b)
}

/// カテゴリの欄を縦に積む（見出しはここに無い。ウィンドウが出す）。
pub fn category_body(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    ctx: &egui::Context,
    category: Category,
) {
    let lang = app.lang;
    rows.indent = 0.0;
    match category {
        Category::Shape => tip_fields(ui, app, rows, ctx, lang),
        Category::Stroke => stroke_fields(ui, app, rows, lang),
        Category::Pressure => pressure_fields(ui, app, rows, lang),
        Category::Dynamics => dynamics_fields(ui, app, rows, lang),
        Category::Jitter => jitter_fields(ui, app, rows, lang),
        Category::Texture => texture_fields(ui, app, rows, ctx, lang),
        Category::Dual => dual_fields(ui, app, rows, ctx, lang),
        Category::Color => color_fields(ui, app, rows, lang),
        Category::Mix => mix_fields(ui, app, rows, lang),
        Category::Effect => effect_fields(ui, app, rows, ctx, lang),
    }
}

/// カテゴリの設定を既定に戻す（見出しの右の「既定に戻す」）。
pub fn reset_category(app: &mut AppState, category: Category) {
    let defaults = BrushState::default();
    let brush = &mut app.m2.brush;
    match category {
        Category::Shape => {
            app.brush.hardness = defaults.hardness;
            app.brush.anti_alias = defaults.anti_alias;
            brush.tip = TipShape::default();
        }
        Category::Stroke => {
            app.brush.radius = defaults.radius;
            app.brush.spacing = defaults.spacing;
            app.brush.flow = defaults.flow;
            app.brush.opacity = defaults.opacity;
            brush.assist.stabilizer = 0.0;
            brush.assist.curve = false;
            app.view3d.projection = Default::default();
        }
        Category::Pressure => {
            app.brush.pressure_size = defaults.pressure_size;
            app.brush.pressure_opacity = defaults.pressure_opacity;
            app.brush.pressure_flow = defaults.pressure_flow;
            brush.controls.pressure_hardness = false;
            brush.pressure = PressureResponses::default();
        }
        Category::Dynamics => {
            // 筆圧で硬さを変える切り替えは「筆圧」の側が持つ（ここでは残す）
            let keep = brush.controls.pressure_hardness;
            brush.controls = Controls {
                pressure_hardness: keep,
                ..Controls::default()
            };
            brush.assist.taper_in = 0.0;
            brush.assist.taper_out = 0.0;
        }
        Category::Jitter => brush.jitter = Jitter::default(),
        Category::Texture => {
            brush.texture = None;
            app.m2.texture_prefs = m2::TexturePrefs::default();
        }
        Category::Dual => {
            brush.dual = None;
            app.m2.dual_stash = Default::default();
        }
        Category::Color => brush.color = ColorDynamics::default(),
        Category::Mix => brush.mix = ColorMix::default(),
        Category::Effect => brush.effect = BrushEffect::Paint,
    }
}

// ───────── ストローク ─────────

fn stroke_fields(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
    let b = &mut app.brush;
    if let Some(v) = slider_row(
        ui,
        rows,
        "brush.size",
        lang.pick("直径", "Size"),
        b.radius * 2.0,
        (1.0, 256.0),
        NumberFormat::int(" px"),
        Some(lang.pick("ブラシの直径（[ と ]）", "Brush diameter ([ and ])")),
        true,
    ) {
        b.radius = (v / 2.0).max(0.5);
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "brush.spacing",
        lang.pick("間隔", "Spacing"),
        b.spacing * 100.0,
        (1.0, 100.0),
        NumberFormat::int("%"),
        None,
        true,
    ) {
        b.spacing = v / 100.0;
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "brush.flow",
        lang.pick("流量", "Flow"),
        b.flow * 100.0,
        (0.0, 100.0),
        NumberFormat::int("%"),
        None,
        true,
    ) {
        b.flow = v / 100.0;
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "brush.opacity",
        lang.pick("不透明度", "Opacity"),
        b.opacity * 100.0,
        (0.0, 100.0),
        NumberFormat::int("%"),
        None,
        true,
    ) {
        b.opacity = v / 100.0;
    }
    let a = &mut app.m2.brush.assist;
    if let Some(v) = slider_row(
        ui,
        rows,
        "assist.stabilizer",
        lang.pick("手ぶれ補正", "Stabilizer"),
        a.stabilizer as f32,
        (0.0, 200.0),
        NumberFormat::int(" px"),
        None,
        true,
    ) {
        a.stabilizer = v as f64;
    }
    if let Some(v) = toggle_row(
        ui,
        rows,
        "assist.curve",
        lang.pick("曲線", "Curve"),
        a.curve,
        None,
        true,
    ) {
        a.curve = v;
    }
    if app.view3d.paintable_on_screen() {
        projection_fields(ui, app, rows, lang);
    } else {
        app.view3d.projection_dragging = false;
    }
}

/// 3D の塗りの切り替え（3D のビューが出ているあいだだけ。ストロークの始めに固める）。
fn projection_fields(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
    let free = !app.is_stroking();
    group(ui, rows, "3D");
    let mut dragging = false;
    let p = &mut app.view3d.projection;
    if let Some(v) = toggle_row(
        ui,
        rows,
        "projection.hidden",
        lang.pick("隠れた所も塗る", "Paint hidden areas"),
        p.paint_hidden,
        None,
        free,
    ) {
        p.paint_hidden = v;
    }
    if let Some(v) = toggle_row(
        ui,
        rows,
        "projection.backfaces",
        lang.pick("裏の面も塗る", "Paint back faces"),
        p.paint_backfaces,
        None,
        free,
    ) {
        p.paint_backfaces = v;
    }
    if let Some(v) = toggle_row(
        ui,
        rows,
        "projection.falloff",
        lang.pick("面の向きで弱める", "Fade by angle"),
        p.angle_falloff,
        None,
        free,
    ) {
        p.angle_falloff = v;
    }
    if p.angle_falloff {
        if let Some(v) = projection_slider(
            ui,
            rows,
            &mut dragging,
            "projection.angle-start",
            lang.pick("弱め始め", "Fade start"),
            p.angle_start,
            (0.0, 90.0),
            NumberFormat::int("°"),
            None,
            free,
        ) {
            p.angle_start = v.round();
            p.angle_end = p.angle_end.max(p.angle_start);
        }
        if let Some(v) = projection_slider(
            ui,
            rows,
            &mut dragging,
            "projection.angle-end",
            lang.pick("塗らない角度", "Fade end"),
            p.angle_end,
            (0.0, 90.0),
            NumberFormat::int("°"),
            None,
            free,
        ) {
            p.angle_end = v.round();
            p.angle_start = p.angle_start.min(p.angle_end);
        }
    }
    if let Some(v) = projection_slider(
        ui,
        rows,
        &mut dragging,
        "projection.bleed",
        lang.pick("継ぎ目のにじみ", "Seam bleed"),
        p.seam_bleed as f32,
        (0.0, yolu_core::geometry::MAX_SEAM_BLEED as f32),
        NumberFormat::int(" px"),
        None,
        free,
    ) {
        p.seam_bleed = v
            .round()
            .clamp(0.0, yolu_core::geometry::MAX_SEAM_BLEED as f32) as u32;
    }
    app.view3d.projection_dragging = dragging;
}

/// 3D の塗りの切り替えのスライダーの 1 行（[`slider_row`] と同じ形）。ドラッグしている間は dragging を立てる。
#[allow(clippy::too_many_arguments)]
fn projection_slider(
    ui: &mut Ui,
    rows: &mut Rows,
    dragging: &mut bool,
    id: &str,
    label: &str,
    value: f32,
    range: (f32, f32),
    format: NumberFormat,
    tooltip: Option<&str>,
    enabled: bool,
) -> Option<f32> {
    let mut spec = w::SliderSpec::new(label, range.0, range.1, format).enabled(enabled);
    if let Some(tip) = tooltip {
        spec = spec.tooltip(tip);
    }
    let out = w::slider(ui, rows.slider_row(), id, value, &spec);
    *dragging |= out.active;
    out.changed.then_some(out.value)
}

// ───────── 筆圧 ─────────

/// 筆圧の項目（それぞれ、切り替え・最小値・曲線を持つ）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum PressureItem {
    Size,
    Opacity,
    Flow,
    Hardness,
}

impl PressureItem {
    const ALL: [PressureItem; 4] = [
        PressureItem::Size,
        PressureItem::Opacity,
        PressureItem::Flow,
        PressureItem::Hardness,
    ];

    fn key(self) -> &'static str {
        match self {
            PressureItem::Size => "size",
            PressureItem::Opacity => "opacity",
            PressureItem::Flow => "flow",
            PressureItem::Hardness => "hardness",
        }
    }

    fn name(self, lang: Lang) -> &'static str {
        match self {
            PressureItem::Size => lang.pick("サイズ", "Size"),
            PressureItem::Opacity => lang.pick("不透明度", "Opacity"),
            PressureItem::Flow => lang.pick("流量", "Flow"),
            PressureItem::Hardness => lang.pick("硬さ", "Hardness"),
        }
    }

    fn use_tip(self, lang: Lang) -> &'static str {
        match self {
            PressureItem::Size => lang.pick("筆圧で直径を変える", "Pen pressure changes the size"),
            PressureItem::Opacity => {
                lang.pick("筆圧で不透明度を変える", "Pen pressure changes the opacity")
            }
            PressureItem::Flow => lang.pick("筆圧で流量を変える", "Pen pressure changes the flow"),
            PressureItem::Hardness => lang.pick(
                "筆圧で丸い筆先の縁の硬さを変える",
                "Pen pressure changes the edge hardness of the round tip",
            ),
        }
    }

    fn on(self, app: &AppState) -> bool {
        match self {
            PressureItem::Size => app.brush.pressure_size,
            PressureItem::Opacity => app.brush.pressure_opacity,
            PressureItem::Flow => app.brush.pressure_flow,
            PressureItem::Hardness => app.m2.brush.controls.pressure_hardness,
        }
    }

    fn set_on(self, app: &mut AppState, on: bool) {
        match self {
            PressureItem::Size => app.brush.pressure_size = on,
            PressureItem::Opacity => app.brush.pressure_opacity = on,
            PressureItem::Flow => app.brush.pressure_flow = on,
            PressureItem::Hardness => app.m2.brush.controls.pressure_hardness = on,
        }
    }

    fn response(self, app: &mut AppState) -> &mut PressureResponse {
        let p = &mut app.m2.brush.pressure;
        match self {
            PressureItem::Size => &mut p.size,
            PressureItem::Opacity => &mut p.opacity,
            PressureItem::Flow => &mut p.flow,
            PressureItem::Hardness => &mut p.hardness,
        }
    }
}

/// 筆圧の曲線の行（`ui::curve::curve_editor`）。点を足す・動かす・消す操作が決まったとき（ドラッグは離したとき）だけ、新しい曲線を返す。
/// 最小値はスライダーが持つので、枠が描くのは曲線そのもの。
fn pressure_curve_row(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    curve: &Curve,
    tooltip: &str,
    enabled: bool,
) -> Option<Curve> {
    let rect = rows.row(curve::HEIGHT, 6.0);
    curve::curve_editor(ui, rect, id.to_owned(), curve, tooltip, enabled)
}

fn pressure_fields(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
    let hardness_on = hardness_applies(app);
    for item in PressureItem::ALL {
        let usable = item != PressureItem::Hardness || hardness_on;
        let on = item.on(app);
        let response = item.response(app).clone();
        let key = item.key();
        group(ui, rows, item.name(lang));
        let image_tip = lang.pick("画像の筆先では効きません", "No effect on an image tip");
        let tip_on = if usable {
            item.use_tip(lang)
        } else {
            image_tip
        };
        if let Some(v) = toggle_row(
            ui,
            rows,
            &format!("pressure.{key}.on"),
            lang.pick("筆圧を使う", "Use pen pressure"),
            on,
            Some(tip_on),
            usable,
        ) {
            item.set_on(app, v);
        }
        let used = usable && on;
        let off_reason = if usable {
            lang.pick("筆圧を使っていません", "Pen pressure is not used")
        } else {
            image_tip
        };
        if let Some(v) = percent_row(
            ui,
            rows,
            &format!("pressure.{key}.min"),
            lang.pick("最小", "Minimum"),
            response.min(),
            (0.0, 1.0),
            (!used).then_some(off_reason),
            used,
        ) {
            if let Ok(next) = response.with_min(v) {
                *item.response(app) = next;
            }
        }
        let curve_tip = if used {
            lang.pick("筆圧の曲線", "Pressure curve")
        } else {
            off_reason
        };
        if let Some(next) = pressure_curve_row(
            ui,
            rows,
            &format!("pressure.{key}.curve"),
            &response.curve_shape(),
            curve_tip,
            used,
        ) {
            if let Ok(next) = response.with_curve_shape(next) {
                *item.response(app) = next;
            }
        }
    }
}

// ───────── 入り抜きとペン ─────────

fn dynamics_fields(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
    group(ui, rows, lang.pick("入り抜き", "Taper"));
    let assist = &mut app.m2.brush.assist;
    if let Some(v) = slider_row(
        ui,
        rows,
        "assist.taper_in",
        lang.pick("入り", "Taper in"),
        assist.taper_in as f32,
        (0.0, 500.0),
        NumberFormat::int(" px"),
        None,
        true,
    ) {
        assist.taper_in = v as f64;
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "assist.taper_out",
        lang.pick("抜き", "Taper out"),
        assist.taper_out as f32,
        (0.0, 500.0),
        NumberFormat::int(" px"),
        None,
        true,
    ) {
        assist.taper_out = v as f64;
    }
    let k = &mut app.m2.brush.controls;
    group(ui, rows, lang.pick("フェード（描点の数）", "Fade (dabs)"));
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
            None,
            true,
        ) {
            *field = v.round().max(0.0) as u32;
        }
    }
    let pen = lang.pick("マウスでは効きません", "No effect with a mouse");
    group(ui, rows, lang.pick("ペンの傾き", "Pen tilt"));
    let (a, b) = toggle_pair(
        ui,
        rows,
        "tilt.a",
        true,
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
        true,
        (lang.pick("流量", "Flow"), pen, k.tilt_flow),
        Some((lang.pick("角度", "Angle"), "", k.tilt_angle)),
    );
    if let Some(v) = a {
        k.tilt_flow = v;
    }
    if let Some(v) = b {
        k.tilt_angle = v;
    }
    group(ui, rows, lang.pick("ペンの回転", "Pen rotation"));
    if let Some(v) = toggle_row(
        ui,
        rows,
        "rotation.angle",
        lang.pick("角度に足す", "Add to angle"),
        k.rotation_angle,
        None,
        true,
    ) {
        k.rotation_angle = v;
    }
    group(ui, rows, lang.pick("筆の速さ", "Speed"));
    let speed = "";
    let (a, b) = toggle_pair(
        ui,
        rows,
        "speed.a",
        true,
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
        true,
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
        None,
        true,
    ) {
        k.speed_max = v as f64;
    }
}

// ───────── ゆらぎ ─────────

fn jitter_fields(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
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
    for (i, (id, label, field)) in [
        ("jitter.size", lang.pick("サイズ", "Size"), &mut j.size),
        ("jitter.angle", lang.pick("角度", "Angle"), &mut j.angle),
        (
            "jitter.roundness",
            lang.pick("真円率", "Roundness"),
            &mut j.roundness,
        ),
        (
            "jitter.opacity",
            lang.pick("不透明度", "Opacity"),
            &mut j.opacity,
        ),
        ("jitter.flow", lang.pick("流量", "Flow"), &mut j.flow),
    ]
    .into_iter()
    .enumerate()
    {
        if let Some(v) = percent_row(ui, rows, id, label, *field, unit, Some(tips[i]), true) {
            *field = v;
        }
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "jitter.scatter",
        lang.pick("散布", "Scatter"),
        j.scatter,
        (0.0, 10.0),
        None,
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
        None,
        true,
    ) {
        j.count = v.round().clamp(1.0, 16.0) as u32;
    }
}

// ───────── テクスチャ ─────────

fn texture_fields(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    ctx: &egui::Context,
    lang: Lang,
) {
    let name = app
        .m2
        .brush
        .texture
        .as_ref()
        .map(|t| m2::tip_display(lang, &t.image))
        .unwrap_or_else(|| lang.pick("なし", "None").to_owned());
    if let Some(b) = choice_row(
        ui,
        rows,
        "texture.image",
        lang.pick("画像", "Image"),
        &name,
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
                None,
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
                None,
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
            None,
            true,
        ) {
            open_popup(app, ctx, Popup::TextureMode, b, b.width());
        }
    }
}

// ───────── デュアルブラシ ─────────

fn dual_fields(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context, lang: Lang) {
    let on = app.m2.brush.dual.is_some();
    if let Some(v) = toggle_row(
        ui,
        rows,
        "dual.enabled",
        lang.pick("デュアルブラシを使う", "Use a dual brush"),
        on,
        None,
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
        .map(|t| m2::tip_display(lang, t))
        .unwrap_or_else(|| lang.pick("丸（硬さ）", "Round (hardness)").to_owned());
    let round = d.tip.is_none();
    let mode = d.mode;
    if let Some(b) = choice_row(
        ui,
        rows,
        "dual.tip",
        lang.pick("筆先", "Tip"),
        &tip_name,
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
        None,
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
        (!round).then(|| lang.pick("画像の筆先では効きません", "No effect on an image tip")),
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

// ───────── 色の揺らぎ ─────────

fn color_fields(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
    let kind = app
        .doc
        .channel_info(app.m2.paint_channel)
        .map(|i| i.kind)
        .unwrap_or(crate::engine::ChannelKind::Color);
    let free = !(app.m2.edit_mask || !yolu_core::brush::carries_color(kind));
    let off =
        (!free).then(|| lang.pick("このチャンネルでは効きません", "No effect on this channel"));
    // 背景色（カラーのパネルのサブの色）
    let row = rows.row(t::ROW_HEIGHT, 2.0);
    let swatch = Rect::from_min_size(row.min + vec2(0.0, 2.0), vec2(30.0, row.height() - 4.0));
    w::color_swatch(
        ui,
        swatch,
        "color.secondary",
        app.color.sub,
        lang.pick("背景色", "Background"),
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
        off,
        free,
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
        off,
        free,
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
        off,
        free,
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
        off,
        free,
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
        off,
        free,
    ) {
        c.purity = v;
    }
    if let Some(v) = toggle_row(
        ui,
        rows,
        "color.per_tip",
        lang.pick("描点ごとに適用", "Apply per tip"),
        c.per_tip,
        off,
        free,
    ) {
        c.per_tip = v;
    }
}

// ───────── 色の混ぜ ─────────

/// 色の混ぜが、今のツール・ブラシ・描く先では効かない理由（効くなら None）。
fn mix_unavailable(app: &AppState, lang: Lang) -> Option<&'static str> {
    if app.tool == Tool::Eraser {
        return Some(lang.pick("消しゴムでは効きません", "No effect with the eraser"));
    }
    if !app.m2.brush.effect.is_paint() {
        return Some(lang.pick("効果のブラシでは効きません", "No effect on effect brushes"));
    }
    let kind = app
        .doc
        .channel_info(app.m2.paint_channel)
        .map(|i| i.kind)
        .unwrap_or(crate::engine::ChannelKind::Color);
    (app.m2.edit_mask || !yolu_core::brush::carries_color(kind))
        .then(|| lang.pick("このチャンネルでは効きません", "No effect on this channel"))
}

/// 筆圧で変えられる色の混ぜの項目。
#[derive(Clone, Copy, PartialEq, Eq)]
enum MixItem {
    Paint,
    Density,
}

impl MixItem {
    fn key(self) -> &'static str {
        match self {
            MixItem::Paint => "paint",
            MixItem::Density => "density",
        }
    }

    fn name(self, lang: Lang) -> &'static str {
        match self {
            MixItem::Paint => lang.pick("絵の具の量", "Paint amount"),
            MixItem::Density => lang.pick("絵の具の濃さ", "Paint density"),
        }
    }

    fn on(self, mix: &ColorMix) -> bool {
        match self {
            MixItem::Paint => mix.pressure_paint,
            MixItem::Density => mix.pressure_density,
        }
    }

    fn set_on(self, mix: &mut ColorMix, on: bool) {
        match self {
            MixItem::Paint => mix.pressure_paint = on,
            MixItem::Density => mix.pressure_density = on,
        }
    }

    fn response(self, mix: &mut ColorMix) -> &mut PressureResponse {
        match self {
            MixItem::Paint => &mut mix.response_paint,
            MixItem::Density => &mut mix.response_density,
        }
    }
}

fn mix_fields(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
    let reason = mix_unavailable(app, lang);
    let usable = reason.is_none();
    let mode = app.m2.brush.mix.mode;
    // 混ぜ方: 排他の 2 つの切り替え（どちらも切ならなし）
    let (mixes, smears) = toggle_pair(
        ui,
        rows,
        "mix.mode",
        usable,
        (
            lang.pick("混ぜる", "Mix"),
            reason.unwrap_or(""),
            mode == MixMode::Mix,
        ),
        Some((
            lang.pick("伸ばす", "Smear"),
            reason.unwrap_or(""),
            mode == MixMode::Smear,
        )),
    );
    if let Some(on) = mixes {
        app.m2.brush.mix.mode = if on { MixMode::Mix } else { MixMode::Off };
    }
    if let Some(on) = smears {
        app.m2.brush.mix.mode = if on { MixMode::Smear } else { MixMode::Off };
    }
    let off_reason = reason.or_else(|| {
        (!app.m2.brush.mix.is_active()).then(|| lang.pick("混ぜ方がなしです", "Mixing is off"))
    });
    let active = off_reason.is_none();
    let unit = (0.0, 1.0);
    let mix = &mut app.m2.brush.mix;
    if let Some(v) = percent_row(
        ui,
        rows,
        "mix.paint",
        MixItem::Paint.name(lang),
        mix.paint,
        unit,
        off_reason,
        active,
    ) {
        mix.paint = v;
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "mix.density",
        MixItem::Density.name(lang),
        mix.density,
        unit,
        off_reason,
        active,
    ) {
        mix.density = v;
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "mix.stretch",
        lang.pick("色延び", "Color stretch"),
        mix.stretch,
        unit,
        off_reason,
        active,
    ) {
        mix.stretch = v;
    }
    let masked = app.m2.edit_mask;
    if let Some(v) = toggle_row(
        ui,
        rows,
        "mix.ground",
        lang.pick("全レイヤーから", "All layers"),
        mix.ground == MixGround::Composite,
        off_reason,
        active && !masked,
    ) {
        mix.ground = if v {
            MixGround::Composite
        } else {
            MixGround::Layer
        };
    }
    // 筆圧（量・濃さ）: 筆圧のカテゴリと同じ形（切り替え・最小値・曲線）
    for item in [MixItem::Paint, MixItem::Density] {
        let key = item.key();
        let response = item.response(&mut app.m2.brush.mix).clone();
        let on = item.on(&app.m2.brush.mix);
        group(
            ui,
            rows,
            &lang.pick(
                format!("{}（筆圧）", item.name(lang)),
                format!("{} (pressure)", item.name(lang)),
            ),
        );
        if let Some(v) = toggle_row(
            ui,
            rows,
            &format!("mix.{key}.on"),
            lang.pick("筆圧を使う", "Use pen pressure"),
            on,
            off_reason,
            active,
        ) {
            item.set_on(&mut app.m2.brush.mix, v);
        }
        let used = active && on;
        let pressure_off = off_reason
            .unwrap_or_else(|| lang.pick("筆圧を使っていません", "Pen pressure is not used"));
        if let Some(v) = percent_row(
            ui,
            rows,
            &format!("mix.{key}.min"),
            lang.pick("最小", "Minimum"),
            response.min(),
            (0.0, 1.0),
            (!used).then_some(pressure_off),
            used,
        ) {
            if let Ok(next) = response.with_min(v) {
                *item.response(&mut app.m2.brush.mix) = next;
            }
        }
        let curve_tip = if used {
            lang.pick("筆圧の曲線", "Pressure curve")
        } else {
            pressure_off
        };
        if let Some(next) = pressure_curve_row(
            ui,
            rows,
            &format!("mix.{key}.curve"),
            &response.curve_shape(),
            curve_tip,
            used,
        ) {
            if let Ok(next) = response.with_curve_shape(next) {
                *item.response(&mut app.m2.brush.mix) = next;
            }
        }
    }
}

// ───────── 効果 ─────────

/// 効果のブラシ（ぼかし・指先・クローン）。消しゴムでは使えない。2D のキャンバスでも 3D の面でも効く。
fn effect_fields(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    ctx: &egui::Context,
    lang: Lang,
) {
    let kind = EffectKind::of(&app.m2.brush.effect);
    let usable = app.tool != Tool::Eraser;
    let eraser =
        (!usable).then(|| lang.pick("消しゴムでは使えません", "Not available with the eraser"));
    if let Some(b) = choice_row(
        ui,
        rows,
        "effect.kind",
        lang.pick("種類", "Type"),
        kind.name(lang),
        eraser,
        usable,
    ) {
        open_popup(app, ctx, Popup::Effect, b, b.width());
    }
    // ぼかし・指先・クローンは 3D の面でも効く（3D の面のブラシ）。断るのは消しゴムだけ
    let reason = eraser;
    let enabled = reason.is_none();
    // クローンの「ずれ」の欄: 3D の面だけを出しているときは面の点で決まり、2D の元を決めて揃えないときはストロークごとに元から決め直すので、
    // どちらも欄では決まらない
    let only_3d = app.paints_only_in_3d();
    let resets_offset = app.clone.canvas_resets_offset(app.doc.id());
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
                reason,
                enabled,
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
                reason,
                enabled,
            ) {
                *strength = v;
            }
        }
        BrushEffect::Clone { offset } => {
            let mut next = *offset;
            let offset_reason = if only_3d {
                Some(lang.pick(
                    "3D では、元の面の点で決まる",
                    "On 3D surfaces the source point decides this",
                ))
            } else if resets_offset {
                Some(lang.pick(
                    "揃えないときは、ストロークごとに始めが元に重なる",
                    "Without Aligned, every stroke starts on the source",
                ))
            } else {
                None
            };
            let offset_tip = eraser.or(offset_reason);
            let offset_enabled = enabled && offset_reason.is_none();
            let mut edited = false;
            if let Some(v) = slider_row(
                ui,
                rows,
                "effect.clone.x",
                lang.pick("ずれ X", "Offset X"),
                offset.x as f32,
                (-2048.0, 2048.0),
                NumberFormat::int(" px"),
                offset_tip,
                offset_enabled,
            ) {
                next.x = v as f64;
                edited = true;
            }
            if let Some(v) = slider_row(
                ui,
                rows,
                "effect.clone.y",
                lang.pick("ずれ Y", "Offset Y"),
                offset.y as f32,
                (-2048.0, 2048.0),
                NumberFormat::int(" px"),
                offset_tip,
                offset_enabled,
            ) {
                next.y = v as f64;
                edited = true;
            }
            *offset = DVec2::new(next.x, next.y);
            // 欄で直した offset は、次のストロークから使う（元からは決め直さない）。ブラシの設定を直したので「変えた」に数える
            if edited {
                app.clone.offset_edited(next);
            }
            // 見えているレイヤーの重なりを読む・揃え方（2D のキャンバスも 3D の面も同じ設定）。
            // マスクを描くあいだは描いているマスクだけを読むので、入れても効かない切り替えは薄くして切った表示にする
            let masked = app.m2.edit_mask;
            let clone = &mut app.clone;
            if let Some(v) = toggle_row(
                ui,
                rows,
                "effect.clone.all-layers",
                lang.pick("全レイヤーから", "All layers"),
                clone.all_layers && !masked,
                eraser.or_else(|| {
                    masked.then(|| {
                        lang.pick(
                            "マスクを描くあいだは、描いているマスクだけを読む",
                            "While painting a mask, only that mask is read",
                        )
                    })
                }),
                usable && !masked,
            ) {
                clone.all_layers = v;
            }
            // 元の有無は文では言わず、揃えるの薄さで示す（元は十字で見える。決めるのは Alt クリック）
            let has_source = app.clone.canvas_source_for(app.doc.id()).is_some()
                || app
                    .view3d
                    .model
                    .as_ref()
                    .is_some_and(|m| app.clone.source_for(&m.geometry).is_some());
            // 元が無いときの理由は、元を決める入力（キーの表から引く）を添える。消しゴムが理由なら、それを先に出す
            let aligned_tip = match eraser {
                Some(reason) => Some(reason.to_owned()),
                None if !has_source => Some(crate::shortcuts::named_with_keys(
                    lang,
                    lang.pick("元を決めると使える", "Available once a source is set"),
                    &[clone_source_tip_key(lang)],
                )),
                None => None,
            };
            let clone = &mut app.clone;
            if let Some(v) = toggle_row(
                ui,
                rows,
                "effect.clone.aligned",
                lang.pick("揃える", "Aligned"),
                clone.aligned,
                aligned_tip.as_deref(),
                usable && has_source,
            ) {
                clone.set_aligned(v);
            }
        }
    }
}

// ───────── 形状（筆先） ─────────

/// 一覧の 1 マス（見本の大きさは `brushes::krita::THUMB`）。
const CELL: f32 = 40.0;
const CELL_GAP: f32 = 4.0;

/// 筆先の画像の見本（白地に黒。丸い筆先は縁がやわらかい円）。
fn tip_image(id: Option<&str>) -> ColorImage {
    crate::brushes::krita::thumbnail(id.and_then(yolu_core::builtin_tip).as_deref())
}

#[derive(Clone, Default)]
struct TipThumbs(HashMap<&'static str, TextureHandle>);

/// 筆先の見本の絵（初めて出すときに作って、文脈に覚えておく）。None は丸。
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

/// 筆先の欄: 今の筆先の見本と名前・硬さ・真円率・角度・線の向き・反転・組み込みの筆先の一覧。
fn tip_fields(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context, lang: Lang) {
    let kind = tip_library::current(&app.m2.brush.tip);
    let hardness_on = hardness_applies(app);
    // 今の筆先: 見本と名前
    let head = rows.row(48.0, 6.0);
    let swatch = Rect::from_min_size(head.min, vec2(48.0, 48.0));
    let swatch_texture = match &kind {
        tip_library::Current::Round => Some(tip_texture(ctx, None)),
        tip_library::Current::Builtin(id) => Some(tip_texture(ctx, Some(id))),
        tip_library::Current::Krita(index) => app.brushes.krita.texture(ctx, *index),
        tip_library::Current::Image(tip) => Some(tip_library::image_texture(ctx, tip)),
    };
    let p = ui.painter();
    w::rounded(p, swatch, Color32::WHITE, 3.0);
    if let Some(texture) = swatch_texture {
        p.image(
            texture,
            swatch.shrink(2.0),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    let name: String = match &kind {
        tip_library::Current::Round => lang.pick("丸（硬さ）", "Round (hardness)").to_owned(),
        tip_library::Current::Builtin(id) => tip_label(lang, id).to_owned(),
        tip_library::Current::Krita(index) => app
            .brushes
            .krita
            .name(*index)
            .map(str::to_owned)
            .unwrap_or_else(|| "Krita".to_owned()),
        tip_library::Current::Image(tip) => m2::tip_display(lang, tip),
    };
    let from = match &kind {
        tip_library::Current::Round => lang.pick("筆先", "Tip"),
        tip_library::Current::Builtin(_) => lang.pick("組み込みの画像", "Built-in image"),
        tip_library::Current::Krita(_) => "Krita",
        tip_library::Current::Image(_) => lang.pick("取り込んだ画像", "Imported image"),
    };
    let tx = head.left() + 56.0;
    let tw = head.width() - 56.0;
    let shown = w::fit(ui.painter(), &name, tw, t::LABEL_BOLD);
    w::text(
        ui.painter(),
        Rect::from_min_size(pos2(tx, head.top() + 6.0), vec2(tw, 18.0)),
        &shown,
        t::LABEL_BOLD,
        w::Align::Left,
    );
    let shown = w::fit(ui.painter(), from, tw, t::LABEL_DIM);
    w::text(
        ui.painter(),
        Rect::from_min_size(pos2(tx, head.top() + 26.0), vec2(tw, 16.0)),
        &shown,
        t::LABEL_DIM,
        w::Align::Left,
    );
    // 形の設定（硬さは 3D でも効く）
    if let Some(v) = percent_row(
        ui,
        rows,
        "alpha.hardness",
        lang.pick("硬さ", "Hardness"),
        app.brush.hardness as f64,
        (0.0, 1.0),
        (!hardness_on).then(|| lang.pick("画像の筆先では効きません", "No effect on an image tip")),
        hardness_on,
    ) {
        app.brush.hardness = v as f32;
    }
    // 縁のアンチエイリアス（丸い筆先の縁と、画像の筆先の小さなダブ。3D でも効く）
    if let Some(b) = choice_row(
        ui,
        rows,
        "alpha.anti_alias",
        lang.pick("アンチエイリアス", "Anti-aliasing"),
        anti_alias_label(lang, app.brush.anti_alias),
        None,
        true,
    ) {
        open_popup(app, ctx, Popup::AntiAlias, b, b.width());
    }
    let tip = &mut app.m2.brush.tip;
    if let Some(v) = percent_row(
        ui,
        rows,
        "alpha.roundness",
        lang.pick("真円率", "Roundness"),
        tip.roundness,
        (0.01, 1.0),
        None,
        true,
    ) {
        tip.roundness = v;
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "brush.angle",
        lang.pick("角度", "Angle"),
        tip.angle as f32,
        (-180.0, 180.0),
        NumberFormat::int("°"),
        None,
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
        None,
        true,
    ) {
        tip.follow_direction = v;
    }
    // ホース（images が複数）にも反転は掛かる
    let image = tip.image.is_some() || !tip.images.is_empty();
    let flip_tip = lang.pick("画像の筆先を反転する", "Mirrors an image tip");
    let (a, b) = toggle_pair(
        ui,
        rows,
        "alpha.flip",
        image,
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
    // 筆先の一覧（丸と組み込みの画像）
    group(ui, rows, lang.pick("組み込み", "Built-in"));
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
            // 印は種類から決める（Krita・取り込んだ画像のときは丸にも組み込みにも付けない）
            let selected = match &kind {
                tip_library::Current::Round => id.is_none(),
                tip_library::Current::Builtin(b) => *id == Some(*b),
                _ => false,
            };
            if tip_cell(ui, cell, *id, selected, tip_name) {
                app.apply(Action::M2Ui(UiOp::Brush(BrushOp::Tip(*id))));
            }
        }
    }
    // 同梱の Krita の筆先
    tip_library::krita_section(ui, app, rows, ctx);
}

/// ステンシルのタブ（中身は `stencil_props`）。
pub fn stencil_tab(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let ctx = ui.ctx().clone();
    super::stencil_props::stencil_tab(ui, app, rows, &ctx);
}
