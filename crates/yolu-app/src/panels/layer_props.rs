//! プロパティの欄のレイヤーの中身（Unity 版の LayerPanels と同じ並び）: 選んでいるレイヤーの名前の見出しの下に、グループの
//! 通過、塗りつぶしのチャンネルごとの値、調整の設定、チャンネルごとの有効と自分の合成、レイヤーマスク。値は `Action::M2` を通る
//! （1 回の Undo。スライダーのドラッグは離したところで区切る）。描いている間・読むだけのセットでは触れない。

use egui::{pos2, vec2, Rect, Ui};

use super::color_window::{self, Pick};
use super::properties::{group_label, percent_row, section, slider_row, toggle_row};
use crate::engine::{
    AdjustmentSettings, AdjustmentType, BlendMode, ChannelKind, LayerId, LayerKind, Rgba8,
};
use crate::lang::Lang;
use crate::m2::{self, AdjustmentKind, Edit, UiOp};
use crate::notice::Source;
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};

fn decimals(places: u8) -> NumberFormat<'static> {
    NumberFormat {
        decimals: places,
        trim: true,
        suffix: "",
    }
}

fn edit(app: &mut AppState, edit: Edit) {
    app.apply(Action::M2(edit));
}

/// レイヤーの欄の全部（選んでいるレイヤーの種類に合わせて）。
pub fn layer_body(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    let lang = app.lang;
    let Some(id) = app.selected_layer else {
        return;
    };
    let Some(layer) = app.doc.layer(id) else {
        return;
    };
    let (kind, name) = (layer.kind(), layer.name().to_owned());
    let icon = if layer.text().is_some() {
        "tools/text"
    } else {
        m2::layer_kind_icon(kind)
    };
    let enabled = app.can_edit();

    let (open, _) = section(ui, app, rows, "layer", &name, icon, None);
    if open {
        layer_section(ui, app, rows, id, enabled, lang);
    }
    lock_section(ui, app, rows);
    crate::rulers::props::section_for_layer(ui, app, rows);
    if app.doc.layer(id).is_some_and(|l| l.text().is_some()) {
        crate::textlayer::props::layer_section(ui, app, rows, enabled);
    }
    match kind {
        LayerKind::Fill => {
            fill_section(ui, app, rows, id, enabled, lang);
            super::fill_props::sections(ui, app, rows, id, enabled, lang);
        }
        LayerKind::Adjustment => adjustment_section(ui, app, rows, id, enabled, lang),
        _ => {}
    }
    if kind != LayerKind::Group {
        channels_section(ui, app, rows, id, enabled, lang);
    }
    mask_section(ui, app, rows, id, enabled, lang);
}

/// マスクに描くときのタブ（レイヤーマスクの設定だけ）。
pub fn mask_tab(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    let Some(id) = app.selected_layer else {
        return;
    };
    let enabled = app.can_edit();
    mask_section(ui, app, rows, id, enabled, lang);
}

fn layer_section(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    id: LayerId,
    enabled: bool,
    lang: Lang,
) {
    let Some(layer) = app.doc.layer(id) else {
        return;
    };
    let (group, blend) = (layer.is_group(), layer.blend_mode());
    if group {
        if let Some(on) = toggle_row(
            ui,
            rows,
            "layer.passthrough",
            lang.pick("通過", "Pass through"),
            blend == BlendMode::PassThrough,
            Some(lang.pick(
                "入: 中身をグループでないかのように下へ重ねる。切: 中身を先にまとめてから重ねる（分離）",
                "On: the contents blend with the layers below as if not grouped. Off: they are composited together first (isolated)",
            )),
            enabled,
        ) {
            edit(
                app,
                Edit::BlendMode {
                    id,
                    channel: None,
                    mode: if on {
                        BlendMode::PassThrough
                    } else {
                        BlendMode::Normal
                    },
                },
            );
        }
    }
    // このレイヤーまでの合成に名前を付ける（上のレイヤーの Generator が読む）
    super::effect_props::anchor_row(ui, app, rows, id, yolu_core::AnchorPlacement::Layer);
    // 画素へのフィルター（調整・グループのレイヤーには画素が無い）
    if matches!(
        app.doc.layer(id).map(|l| l.kind()),
        Some(LayerKind::Raster | LayerKind::Fill)
    ) {
        super::effect_props::add_effect_row(ui, app, rows, yolu_core::FilterTarget::Content);
    }
}

/// ロックの 4 種（選んでいるレイヤーの全部に効く）。持っているロックはチェック。グループやすべてのロックから効いているだけのものは
/// チェックせず、ツールチップで言う。1 回の Undo。
pub fn lock_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    use crate::layerops::{lock_name, LOCK_FLAGS};
    let lang = app.lang;
    let ids = app.selected_layers();
    if ids.is_empty() {
        return;
    }
    let (open, _) = section(
        ui,
        app,
        rows,
        "locks",
        lang.pick("ロック", "Lock"),
        "lock",
        None,
    );
    if !open {
        return;
    }
    let enabled = app.can_edit();
    for flag in LOCK_FLAGS {
        let own = ids
            .iter()
            .all(|id| app.doc.layer(*id).is_some_and(|l| l.locks().contains(flag)));
        let effective = ids.iter().all(|id| {
            app.doc
                .effective_locks(*id)
                .is_ok_and(|locks| locks.contains(flag))
        });
        let tip = if flag == yolu_core::LayerLocks::TRANSPARENCY {
            lang.pick(
                "描いても各画素の透明度は変わらず、色だけが変わる",
                "Painting keeps each pixel's transparency and changes only its colour",
            )
        } else if flag == yolu_core::LayerLocks::PIXELS {
            lang.pick(
                "レイヤーの画素は変えられない（移動とマスクへの描画はできる）",
                "The layer's pixels cannot be changed (it can still be moved and its mask painted)",
            )
        } else if flag == yolu_core::LayerLocks::POSITION {
            lang.pick(
                "レイヤーを動かす・変形できない",
                "The layer cannot be moved or transformed",
            )
        } else {
            lang.pick(
                "画素・位置・レイヤーの設定を変えられない",
                "Pixels, position and the layer's settings cannot be changed",
            )
        };
        let tip = if effective && !own {
            format!(
                "{tip} — {}",
                lang.pick(
                    "すべてのロックかグループのロックが効いています",
                    "in effect through Lock all or a locked group"
                )
            )
        } else {
            tip.to_owned()
        };
        if let Some(on) = toggle_row(
            ui,
            rows,
            &format!("layer.lock.{}", flag.bits()),
            lock_name(lang, flag),
            own,
            Some(&tip),
            enabled,
        ) {
            edit(
                app,
                Edit::Lock {
                    ids: ids.clone(),
                    flag,
                    on,
                },
            );
        }
    }
}

/// 塗りつぶしのチャンネルごとの値。
fn fill_section(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    id: LayerId,
    enabled: bool,
    lang: Lang,
) {
    let (open, _) = section(
        ui,
        app,
        rows,
        "fill",
        lang.pick("塗りつぶし", "Fill"),
        "format_color_fill",
        None,
    );
    if !open {
        return;
    }
    let channels = app.doc.channels();
    for channel in channels {
        let Some(info) = app.doc.channel_info(channel).cloned() else {
            continue;
        };
        let name = m2::channel_name(lang, &app.doc, channel);
        let value = app.doc.layer(id).and_then(|l| l.fill_value(channel));
        let row = rows.row(t::ROW_HEIGHT, 3.0);
        let main = Rect::from_min_max(row.min, pos2(row.right() - 26.0, row.bottom()));
        let side = Rect::from_min_size(
            pos2(row.right() - 24.0, row.top()),
            vec2(24.0, row.height()),
        );
        match value {
            None => {
                w::text(ui.painter(), main, &name, t::LABEL_DIM, w::Align::Left);
                if w::icon_button(
                    ui,
                    side,
                    ("fill.add", channel.index()),
                    "add",
                    &lang.pick(
                        format!("{name} の値を追加（描画色から）"),
                        format!("Add a value for {name} (from the paint color)"),
                    ),
                    false,
                    enabled,
                    16.0,
                )
                .clicked()
                {
                    let c = app.color.main;
                    let v = m2::fill_from_color(c);
                    edit(
                        app,
                        Edit::FillValue {
                            id,
                            channel,
                            value: Some(v),
                        },
                    );
                }
            }
            Some(v) => {
                if info.kind == ChannelKind::Scalar {
                    let spec = SliderSpec::new(&name, 0.0, 1.0, decimals(3)).enabled(enabled);
                    let out = w::slider(
                        ui,
                        main,
                        ("fill.value", channel.index()),
                        v.r as f32 / 255.0,
                        &spec,
                    );
                    if out.changed {
                        let g = (out.value.clamp(0.0, 1.0) * 255.0).round() as u8;
                        edit(
                            app,
                            Edit::FillValue {
                                id,
                                channel,
                                value: Some(Rgba8::new(g, g, g, v.a)),
                            },
                        );
                    }
                    if out.released {
                        app.m2_end_drag();
                    }
                } else {
                    let label = Rect::from_min_size(main.min, vec2(80.0, main.height()));
                    w::text(ui.painter(), label, &name, t::LABEL, w::Align::Left);
                    let swatch = Rect::from_min_max(
                        pos2(main.left() + 84.0, main.top() + 1.0),
                        pos2(main.right(), main.bottom() - 1.0),
                    );
                    // 押すと色のウィンドウ（相手はレイヤーとチャンネルごと）。ウィンドウの変更はその場で当て、ドラッグ 1 回を 1 回の取り消しにまとめる
                    let target = egui::Id::new(("fill.value", id.0, channel.index()));
                    if let Some(u) = color_window::field(
                        ui,
                        swatch,
                        target,
                        &name,
                        Pick::rgb([v.r, v.g, v.b]),
                        &lang.pick(format!("{name} の値"), format!("Value of {name}")),
                        enabled,
                    ) {
                        let [r, g, b] = u.pick.rgb;
                        let next = Rgba8::new(r, g, b, v.a);
                        if next != v {
                            edit(
                                app,
                                Edit::FillValue {
                                    id,
                                    channel,
                                    value: Some(next),
                                },
                            );
                        }
                        if u.done {
                            app.m2_end_drag();
                        }
                    }
                }
                if w::icon_button(
                    ui,
                    side,
                    ("fill.remove", channel.index()),
                    "delete",
                    &lang.pick(
                        format!("{name} の値を外す"),
                        format!("Remove the value for {name}"),
                    ),
                    false,
                    enabled,
                    16.0,
                )
                .clicked()
                {
                    edit(
                        app,
                        Edit::FillValue {
                            id,
                            channel,
                            value: None,
                        },
                    );
                    app.m2_end_drag();
                }
            }
        }
    }
}

/// 調整の設定。
fn adjustment_section(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    id: LayerId,
    enabled: bool,
    lang: Lang,
) {
    let Some(a) = app.doc.layer(id).and_then(|l| l.adjustment().cloned()) else {
        return;
    };
    let kind = AdjustmentKind::of(&a);
    // 反転には設定が無い（レイヤーの欄の見出しが名前を出している）
    if a.kind() == AdjustmentType::Invert {
        return;
    }
    let (open, _) = section(
        ui,
        app,
        rows,
        "adjustment",
        kind.name(lang),
        kind.icon(),
        None,
    );
    if !open {
        return;
    }
    // 描くチャンネルに使えない調整は、注記の行を置かず、理由をツールチップに出す。欄は無効にしない: レイヤーの値は効くチャンネル（色）の
    // 出力には今も効くので、直すためにチャンネルを替えさせない
    let paint = app.m2.paint_channel;
    let reason = app
        .doc
        .channel_info(paint)
        .filter(|info| !a.applies_to(info.kind))
        .map(|_| {
            let name = m2::channel_name(lang, &app.doc, paint);
            lang.pick(
                format!("{name} には効きません"),
                format!("No effect on {name}"),
            )
        });
    let why = reason.as_deref();
    let mut next: Option<Result<AdjustmentSettings, crate::engine::CoreError>> = None;
    // 1 回の操作で決まる変更（曲線・分岐点・切り替え）は、まとめずに独立の 1 回の取り消しにする
    let mut discrete = false;
    match a.kind() {
        AdjustmentType::Invert => {}
        AdjustmentType::GradientMap
        | AdjustmentType::ToneCurve
        | AdjustmentType::ColorBalance
        | AdjustmentType::BrightnessContrast
        | AdjustmentType::Threshold
        | AdjustmentType::Posterize => {
            if let Some(value) = a.color_adjust() {
                let histogram = (a.kind() == AdjustmentType::ToneCurve)
                    .then(|| super::color_adjust::cached_histogram(ui, &app.doc, id, paint))
                    .flatten();
                let mut failure = None;
                let mut params = super::color_adjust::Params {
                    key: ("adjustment", id.0),
                    enabled,
                    why,
                    paint: app.color.main,
                    sub: app.color.sub,
                    lang,
                    histogram: histogram.as_deref(),
                    sets: &mut app.ramp_sets,
                    eyedrop: &mut app.eyedrop,
                    failure: &mut failure,
                };
                if let Some(change) = super::color_adjust::rows(ui, rows, &mut params, &value) {
                    discrete = change.discrete;
                    next = Some(Ok(change.value.into_settings()));
                }
                if let Some(text) = failure {
                    app.fail(crate::notice::Source::Gradient, text);
                }
            }
        }
        AdjustmentType::Levels => {
            let (mut ib, mut iw, mut gamma, mut ob, mut ow) = (
                a.input_black() as f32,
                a.input_white() as f32,
                a.gamma() as f32,
                a.output_black() as f32,
                a.output_white() as f32,
            );
            let mut changed = false;
            let range = (0.0, 1.0);
            group_label(ui, rows, lang.pick("入力", "Input"));
            if let Some(v) = slider_row(
                ui,
                rows,
                "levels.ib",
                lang.pick("黒", "Black"),
                ib,
                range,
                decimals(3),
                why.or(Some(lang.pick(
                    "これ以下の入力は黒",
                    "Input at or below this becomes black",
                ))),
                enabled,
            ) {
                ib = v.min(iw - 0.004).max(0.0);
                changed = true;
            }
            if let Some(v) = slider_row(
                ui,
                rows,
                "levels.iw",
                lang.pick("白", "White"),
                iw,
                range,
                decimals(3),
                why.or(Some(lang.pick(
                    "これ以上の入力は白",
                    "Input at or above this becomes white",
                ))),
                enabled,
            ) {
                iw = v.max(ib + 0.004).min(1.0);
                changed = true;
            }
            if let Some(v) = slider_row(
                ui,
                rows,
                "levels.gamma",
                lang.pick("ガンマ", "Gamma"),
                gamma,
                (0.1, 9.99),
                decimals(2),
                why.or(Some(lang.pick(
                    "中間の明るさ。1 より大きいと明るく、小さいと暗く",
                    "Midtones: above 1 brightens, below 1 darkens",
                ))),
                enabled,
            ) {
                gamma = v;
                changed = true;
            }
            group_label(ui, rows, lang.pick("出力", "Output"));
            if let Some(v) = slider_row(
                ui,
                rows,
                "levels.ob",
                lang.pick("黒", "Black"),
                ob,
                range,
                decimals(3),
                why,
                enabled,
            ) {
                ob = v;
                changed = true;
            }
            if let Some(v) = slider_row(
                ui,
                rows,
                "levels.ow",
                lang.pick("白", "White"),
                ow,
                range,
                decimals(3),
                why,
                enabled,
            ) {
                ow = v;
                changed = true;
            }
            if changed {
                next = Some(AdjustmentSettings::levels(
                    ib as f64,
                    iw as f64,
                    gamma as f64,
                    ob as f64,
                    ow as f64,
                ));
            }
        }
        AdjustmentType::HueSaturation => {
            let (mut hue, mut sat, mut light) =
                (a.hue() as f32, a.saturation() as f32, a.lightness() as f32);
            let mut changed = false;
            if let Some(v) = slider_row(
                ui,
                rows,
                "hs.hue",
                lang.pick("色相", "Hue"),
                hue,
                (-180.0, 180.0),
                NumberFormat::int("°"),
                why,
                enabled,
            ) {
                hue = v;
                changed = true;
            }
            if let Some(v) = percent_row(
                ui,
                rows,
                "hs.saturation",
                lang.pick("彩度", "Saturation"),
                sat as f64,
                (-1.0, 1.0),
                why,
                enabled,
            ) {
                sat = v as f32;
                changed = true;
            }
            if let Some(v) = percent_row(
                ui,
                rows,
                "hs.lightness",
                lang.pick("明度", "Lightness"),
                light as f64,
                (-1.0, 1.0),
                why,
                enabled,
            ) {
                light = v as f32;
                changed = true;
            }
            if changed {
                next = Some(AdjustmentSettings::hue_saturation(
                    hue as f64,
                    sat as f64,
                    light as f64,
                ));
            }
        }
    }
    match next {
        Some(Ok(settings)) if settings != a => {
            edit(app, Edit::Adjust { id, settings });
            if discrete {
                app.m2_end_drag();
            }
        }
        Some(Err(e)) => app.notify(
            crate::notice::Kind::of_core(&e),
            Source::Layer,
            app.lang.core_error(&e),
        ),
        _ => {}
    }
}

/// チャンネルごとの有効と、自分の合成（持っているチャンネルだけ。× でレイヤーの値に戻す）。
fn channels_section(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    id: LayerId,
    enabled: bool,
    lang: Lang,
) {
    let (open, _) = section(
        ui,
        app,
        rows,
        "layer-channels",
        lang.pick("チャンネル", "Channels"),
        "layers",
        None,
    );
    if !open {
        return;
    }
    for channel in app.doc.channels() {
        let name = m2::channel_name(lang, &app.doc, channel);
        let Some(layer) = app.doc.layer(id) else {
            return;
        };
        let on = layer.is_channel_enabled(channel);
        let blend = layer.channel_blend(channel);
        let layer_blend = (layer.blend_mode(), layer.opacity());
        let row = rows.row(t::ROW_HEIGHT, 2.0);
        let own = !blend.is_empty();
        let right = if own { 112.0 } else { 0.0 };
        let left = Rect::from_min_max(row.min, pos2(row.right() - right, row.bottom()));
        let next = w::toggle(
            ui,
            left,
            ("layer.channel", channel.index()),
            &name,
            on,
            Some(&lang.pick(
                format!("{name} に描く・合成する（切ってもそこにある画素は残る）"),
                format!("Use this layer in {name} (what it holds there is kept when off)"),
            )),
            enabled,
        );
        if next != on {
            edit(
                app,
                Edit::ChannelEnabled {
                    id,
                    channel,
                    enabled: next,
                },
            );
        }
        if own {
            let mode = blend.mode.unwrap_or(layer_blend.0);
            let opacity = blend.opacity.unwrap_or(layer_blend.1);
            let text = format!(
                "{} · {}%",
                m2::blend_label(lang, mode),
                (opacity * 100.0).round() as i32
            );
            let label = Rect::from_min_max(
                pos2(row.right() - right, row.top()),
                pos2(row.right() - 26.0, row.bottom()),
            );
            let shown = w::fit(ui.painter(), &text, label.width(), t::LABEL_DIM);
            w::text(ui.painter(), label, &shown, t::LABEL_DIM, w::Align::Right);
            let clear = Rect::from_min_size(
                pos2(row.right() - 24.0, row.top()),
                vec2(24.0, row.height()),
            );
            if w::icon_button(
                ui,
                clear,
                ("layer.channel.clear", channel.index()),
                "close",
                &lang.pick(
                    format!("{name} もレイヤーの合成モードと不透明度に戻す"),
                    format!("Use the layer's blend mode and opacity in {name} again"),
                ),
                false,
                enabled,
                14.0,
            )
            .clicked()
            {
                edit(
                    app,
                    Edit::OwnBlend {
                        id,
                        channel,
                        own: false,
                    },
                );
            }
        }
    }
}

/// レイヤーマスク（全チャンネルで共有）。
fn mask_section(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    id: LayerId,
    enabled: bool,
    lang: Lang,
) {
    let (open, _) = section(
        ui,
        app,
        rows,
        "mask",
        lang.pick("レイヤーマスク", "Layer Mask"),
        "vignette",
        None,
    );
    if !open {
        return;
    }
    let mask = app
        .doc
        .layer(id)
        .and_then(|l| l.mask())
        .map(|m| (m.enabled(), m.inverted(), m.density()));
    let Some((on, inverted, density)) = mask else {
        let r = rows.row(24.0, 4.0);
        if w::button(
            ui,
            r,
            "mask.add",
            lang.pick("レイヤーマスクを追加", "Add Layer Mask"),
            false,
            enabled,
            Some(lang.pick(
                "マスクはレイヤーの一部を隠す。全チャンネルで共有する",
                "A mask hides parts of the layer; it is shared by all channels",
            )),
            Some("add"),
        )
        .clicked()
        {
            edit(app, Edit::AddMask(id));
        }
        return;
    };
    let editing = app.m2.edit_mask && app.selected_layer == Some(id);
    let r = rows.row(24.0, 4.0);
    if w::button(
        ui,
        r,
        "mask.paint",
        lang.pick("マスクに描く", "Paint on Mask"),
        editing,
        enabled,
        Some(lang.pick(
            "ブラシはマスクに描く。塗ると隠し、消すと見せる",
            "Strokes go to the mask: painting hides, erasing reveals",
        )),
        Some("paint_brush"),
    )
    .clicked()
    {
        app.apply(Action::M2Ui(UiOp::EditMask(!editing)));
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "mask.density",
        lang.pick("濃度", "Density"),
        density,
        (0.0, 1.0),
        Some(lang.pick(
            "マスクが隠す強さ（0% でマスクなしと同じ）",
            "How strongly the mask hides (0% = as if there were no mask)",
        )),
        enabled,
    ) {
        edit(app, Edit::MaskDensity(id, v));
    }
    let row = rows.row(t::ROW_HEIGHT, 4.0);
    let cols = Rows::split(
        Rect::from_min_max(row.min, pos2(row.right() - 28.0, row.bottom())),
        2,
        6.0,
    );
    let a = w::toggle(
        ui,
        cols[0],
        "mask.enabled",
        lang.pick("有効", "Enabled"),
        on,
        Some(lang.pick(
            "切るとマスクなしと同じ（マスクは残る）",
            "Off: the layer shows as if it had no mask (the mask is kept)",
        )),
        enabled,
    );
    if a != on {
        edit(app, Edit::MaskEnabled(id, a));
    }
    let b = w::toggle(
        ui,
        cols[1],
        "mask.invert",
        lang.pick("反転", "Invert"),
        inverted,
        Some(lang.pick(
            "隠す所と見せる所を入れ替える",
            "Swaps what the mask hides and reveals",
        )),
        enabled,
    );
    if b != inverted {
        edit(app, Edit::MaskInverted(id, b));
    }
    let del = Rect::from_min_size(
        pos2(row.right() - 24.0, row.top()),
        vec2(24.0, row.height()),
    );
    if w::icon_button(
        ui,
        del,
        "mask.delete",
        "delete",
        lang.pick("レイヤーマスクを削除", "Delete Layer Mask"),
        false,
        enabled,
        16.0,
    )
    .clicked()
    {
        edit(app, Edit::RemoveMask(id));
        return;
    }
    // このマスクに名前を付ける（上のレイヤーの Generator が、このレイヤーの見える度合いを読む）
    super::effect_props::anchor_row(ui, app, rows, id, yolu_core::AnchorPlacement::Mask);
    super::effect_props::add_effect_row(ui, app, rows, yolu_core::FilterTarget::Mask);
}
