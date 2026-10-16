//! PSD の書き出しのウィンドウ 2 つ: 書き出しの設定（方式・チャンネル）と、書く前の確かめ（焼く・丸める・落とすものをレイヤーの名前つきで並べる）。
//! ウィンドウには名前・状態・短い理由だけを書き、説明はツールチップに置く。何を焼くかの判断は `yolu_io::psd::plan_export`（ここは文にするだけ）。

use egui::{pos2, vec2, Id, Key, Rect, Sense, Ui};
use yolu_core::{AdjustmentType, BalanceRange, Channel, Document, ToneChannel};
use yolu_io::psd::{
    Blocker, ExportMode, ExportNote, GradientExpansion, NoteAction, Refusal, RoundedParameter,
    RoundedValue,
};

use crate::lang::Lang;
use crate::m2::{channel_name, AdjustmentKind};
use crate::psd::PsdAction;
use crate::state::{Action, AppState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::ui::window::{self, Spec};
use crate::windows::{show_list, Button, ListSpec, Reply, Row};

/// ウィンドウの名前（`windows::window_rect` で矩形を引く名前）。
pub const OPTIONS: &str = "psd-export";
pub const CONFIRM: &str = "psd-bake";

const ROW_HEIGHT: f32 = 26.0;
const MAX_ROWS: usize = 12;
const FOOTER: f32 = 48.0;

/// ウィンドウの 1 行。
enum Item {
    /// 区切りの名前（押せない）。
    Label(String),
    /// 方式の選び（ラジオ）。
    Mode(ExportMode, &'static str),
    /// チャンネルの選び（チェック）。
    Channel(Channel, String, bool),
}

// ───────── 書き出しの設定のウィンドウ ─────────

/// 毎フレーム、開いている設定のウィンドウを描き、押された操作を当てる。
pub fn show_options(ctx: &egui::Context, app: &mut AppState) {
    if !app.psd.options_open {
        return;
    }
    let lang = app.lang;
    let mode = app.psd.export.mode;
    let mut items = vec![
        Item::Label(lang.t("psd_export-mode").into()),
        Item::Mode(ExportMode::Bake, lang.t("psd_export-bake-and-write")),
        Item::Mode(ExportMode::Flat, lang.t("psd_export-flatten-to-one-layer")),
        Item::Label(lang.t("psd_export-channels").into()),
    ];
    for c in app.session.doc.channels() {
        items.push(Item::Channel(
            c,
            channel_name(lang, &app.session.doc, c),
            app.psd.export.channels.contains(&c),
        ));
    }
    let visible = items.len().min(MAX_ROWS);
    let height = window::HEADER_HEIGHT + 8.0 + visible as f32 * ROW_HEIGHT + 10.0 + FOOTER;
    let title = lang.t("psd_export-export-psd");
    let close_label = lang.t("psd_export-close-window");
    let spec = Spec {
        title,
        icon: Some("folder_open"),
        size: vec2(380.0, height),
        modal: true,
        close_label,
    };
    let id = Id::new(("yolu.window", OPTIONS));
    let mut offset = app.ui.psd_options_offset;
    let mut scroll = app.psd.options_scroll;
    let mut actions: Vec<PsdAction> = Vec::new();
    let mut esc = false;
    let closed = window::show(ctx, id, &spec, &mut offset, false, |ui, frame| {
        esc = ui.input(|i| i.key_pressed(Key::Escape));
        let body = frame.body;
        let list = Rect::from_min_size(
            pos2(body.left(), body.top() + 8.0),
            vec2(body.width(), visible as f32 * ROW_HEIGHT),
        );
        let content = items.len() as f32 * ROW_HEIGHT;
        let bar = Scroll::begin(ui, list, content, &mut scroll);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
        child.set_clip_rect(list.intersect(ui.clip_rect()));
        for (i, item) in items.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(
                    list.left() + 14.0,
                    list.top() + i as f32 * ROW_HEIGHT - scroll,
                ),
                vec2(list.width() - 28.0 - bar.reserved(), ROW_HEIGHT),
            );
            if r.bottom() < list.top() || r.top() > list.bottom() {
                continue;
            }
            match item {
                Item::Label(text) => {
                    let p = child.painter().clone();
                    w::text(&p, r, text, t::LABEL_DIM, Align::Left);
                }
                Item::Mode(m, label) => {
                    let tip = mode_tooltip(lang, *m);
                    if radio(&mut child, r, ("mode", i), label, *m == mode, tip) && *m != mode {
                        actions.push(PsdAction::SetExportMode(*m));
                    }
                }
                Item::Channel(c, label, on) => {
                    let next = w::toggle(&mut child, r, ("channel", i), label, *on, None, true);
                    if next != *on {
                        actions.push(PsdAction::ToggleExportChannel(*c));
                    }
                }
            }
        }
        bar.end(ui, id.with("list-scroll"), &mut scroll);
        // 下の帯
        let p = ui.painter().clone();
        let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOTER), body.max);
        w::fill(&p, footer, t::PANEL_HEADER);
        w::hline(&p, footer.left(), footer.right(), footer.top(), t::BORDER);
        let mut x = footer.right() - 14.0;
        let buttons = [
            (
                lang.t("psd_export-cancel"),
                false,
                None,
                PsdAction::CancelExportOptions,
            ),
            (
                lang.t("psd_export-export"),
                true,
                Some(export_tooltip(lang)),
                PsdAction::ChooseExportFile,
            ),
        ];
        for (i, (label, primary, tip, action)) in buttons.into_iter().enumerate().rev() {
            let bw = w::text_width(&p, label, t::LABEL) + 32.0;
            let r = Rect::from_min_size(pos2(x - bw, footer.top() + 10.0), vec2(bw, 28.0));
            x = r.left() - 8.0;
            if w::button(
                ui,
                r,
                id.with(("button", i)),
                label,
                primary,
                true,
                tip,
                None,
            )
            .clicked()
            {
                actions.push(action);
            }
        }
    });
    app.ui.psd_options_offset = offset;
    app.psd.options_scroll = scroll;
    if (closed || esc) && actions.is_empty() {
        actions.push(PsdAction::CancelExportOptions);
    }
    for action in actions {
        app.apply(Action::Psd(action));
    }
}

/// 方式のツールチップ（説明はここに置く）。
fn mode_tooltip(lang: Lang, mode: ExportMode) -> &'static str {
    match mode {
        ExportMode::Bake => lang.t("psd_export-keeps-the-layers-filters-generators-tip"),
        ExportMode::Flat => lang.t("psd_export-writes-only-the-composite-as-tip"),
    }
}

/// 「書き出し…」のツールチップ。
fn export_tooltip(lang: Lang) -> &'static str {
    lang.t("psd_export-choose-where-to-write-several-tip")
}

/// ラジオ（選んでいる丸）。押されたら true。
fn radio(
    ui: &mut Ui,
    r: Rect,
    salt: impl egui::AsIdSalt,
    label: &str,
    selected: bool,
    tooltip: &str,
) -> bool {
    let id = ui.make_persistent_id(("psd-radio", salt));
    let response = ui.interact(r, id, Sense::click());
    let p = ui.painter().clone();
    let center = pos2(r.left() + 8.0, r.center().y);
    p.circle_filled(center, 8.0, t::CONTROL_BG);
    p.circle_stroke(
        center,
        8.0,
        egui::Stroke::new(
            1.0,
            if selected || response.hovered() {
                t::ACCENT
            } else {
                t::SEPARATOR
            },
        ),
    );
    if selected {
        p.circle_filled(center, 4.0, t::ACCENT);
    }
    w::text(
        &p,
        Rect::from_min_max(pos2(r.left() + 23.0, r.top()), r.max),
        label,
        t::LABEL.with_color(t::TEXT),
        Align::Left,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, label)
    });
    let response = response.on_hover_text(tooltip);
    response.clicked()
}

// ───────── 書く前の確認のウィンドウ ─────────

/// 確認のウィンドウの 1 行目の文（焼く・丸める・落とすの数。短い状態）。
fn summary(lang: Lang, sections: &[(Channel, Vec<ExportNote>)]) -> String {
    let (mut baked, mut rounded, mut dropped, mut other) = (0, 0, 0, 0);
    for (_, notes) in sections {
        for n in notes {
            match n.action {
                NoteAction::Rounded { .. } | NoteAction::ExpandedGradientCurve { .. } => {
                    rounded += 1
                }
                NoteAction::DroppedClippingMark
                | NoteAction::DroppedFilters(_)
                | NoteAction::DroppedMaskFilters(_)
                | NoteAction::DroppedAnchor
                | NoteAction::DroppedMaskAnchor => dropped += 1,
                NoteAction::NormalBlend => other += 1,
                _ => baked += 1,
            }
        }
    }
    let mut parts = Vec::new();
    if baked > 0 {
        parts.push(lang.f("psd_export-bake", &[("baked", &baked)]));
    }
    if rounded > 0 {
        parts.push(lang.f("psd_export-round", &[("rounded", &rounded)]));
    }
    if dropped > 0 {
        parts.push(lang.f("psd_export-drop", &[("dropped", &dropped)]));
    }
    if other > 0 {
        parts.push(lang.f("psd_export-note", &[("other", &other)]));
    }
    parts.join(" · ")
}

/// 確認のウィンドウの 1 行目の文（試験用）。
#[cfg(test)]
pub(crate) fn summary_for_test(lang: Lang, sections: &[(Channel, Vec<ExportNote>)]) -> String {
    summary(lang, sections)
}

/// 毎フレーム、書く前の確認のウィンドウを描き、押された操作を当てる。
pub fn show_confirm(ctx: &egui::Context, app: &mut AppState) {
    let Some(confirm) = app.psd.notes_confirm.as_ref() else {
        return;
    };
    let lang = app.lang;
    let doc = confirm.document();
    let sections = confirm.sections();
    let many = sections.len() > 1;
    let mut rows: Vec<Row> = Vec::new();
    for (channel, notes) in &sections {
        if many {
            rows.push(Row::text(channel_name(lang, doc, *channel), false));
        }
        for n in notes {
            let (what, how) = note_columns(lang, n);
            // チャンネルの注記（法線の重ね方）は、レイヤーの名前でなくチャンネルの名前を左に
            let left = if matches!(n.action, NoteAction::NormalBlend) {
                channel_name(lang, doc, *channel)
            } else {
                n.layer.clone()
            };
            rows.push(Row {
                left,
                middle: what,
                right: how,
                warning: false,
            });
        }
    }
    let spec = ListSpec {
        id: CONFIRM,
        title: lang.t("psd_export-export-psd-check").into(),
        icon: "warning",
        modal: true,
        width: 640.0,
        summary: Some((
            format!("{} · {}", confirm.file(), summary(lang, &sections)),
            true,
        )),
        rows,
        buttons: vec![
            Button {
                label: lang.t("psd_export-cancel").into(),
                primary: false,
                tooltip: None,
            },
            Button {
                label: lang.t("psd_export-write").into(),
                primary: true,
                tooltip: Some(lang.t("psd_export-writes-the-psd-as-listed-tip").into()),
            },
        ],
        close_label: lang.t("psd_export-close-window").into(),
    };
    let mut offset = app.ui.psd_notes_offset;
    let mut scroll = 0.0;
    let reply = show_list(ctx, &spec, &mut offset, &mut scroll);
    app.ui.psd_notes_offset = offset;
    match reply {
        Some(Reply::Button(1)) => app.apply(Action::Psd(PsdAction::ConfirmWrite)),
        Some(_) => app.apply(Action::Psd(PsdAction::CancelWrite)),
        None => {}
    }
}

// ───────── 文 ─────────

fn names(lang: Lang, stages: &[yolu_core::EffectSettings]) -> String {
    stages
        .iter()
        .map(|s| crate::fx::names::effect_name(lang, s))
        .collect::<Vec<_>>()
        .join(lang.t("psd_export-text"))
}

fn adjustment_name(lang: Lang, kind: AdjustmentType) -> &'static str {
    AdjustmentKind::ALL
        .iter()
        .find(|k| k.settings().kind() == kind)
        .map_or("", |k| k.name(lang))
}

fn parameter_name(lang: Lang, p: RoundedParameter) -> String {
    let tone = |curve: ToneChannel| match curve {
        ToneChannel::Composite => "RGB",
        ToneChannel::Red => "R",
        ToneChannel::Green => "G",
        ToneChannel::Blue => "B",
    };
    match p {
        RoundedParameter::InputBlack => lang.t("psd_export-input-black").into(),
        RoundedParameter::InputWhite => lang.t("psd_export-input-white").into(),
        RoundedParameter::OutputBlack => lang.t("psd_export-output-black").into(),
        RoundedParameter::OutputWhite => lang.t("psd_export-output-white").into(),
        RoundedParameter::Gamma => lang.t("psd_export-gamma").into(),
        RoundedParameter::Hue => lang.t("psd_export-hue").into(),
        RoundedParameter::Saturation => lang.t("psd_export-saturation").into(),
        RoundedParameter::Lightness => lang.t("psd_export-lightness").into(),
        RoundedParameter::Balance { range, axis } => {
            let range = match range {
                BalanceRange::Shadows => lang.t("psd_export-shadows"),
                BalanceRange::Midtones => lang.t("psd_export-midtones"),
                BalanceRange::Highlights => lang.t("psd_export-highlights"),
            };
            let axis = match axis {
                0 => lang.t("psd_export-cyan-red"),
                1 => lang.t("psd_export-magenta-green"),
                _ => lang.t("psd_export-yellow-blue"),
            };
            format!("{range} {axis}")
        }
        RoundedParameter::Brightness => lang.t("psd_export-brightness").into(),
        RoundedParameter::Contrast => lang.t("psd_export-contrast").into(),
        RoundedParameter::CurveInput { curve, point } => lang.f(
            "psd_export-point-input",
            &[("curve", &tone(curve)), ("point", &(point + 1))],
        ),
        RoundedParameter::CurveOutput { curve, point } => lang.f(
            "psd_export-point-output",
            &[("curve", &tone(curve)), ("point", &(point + 1))],
        ),
        RoundedParameter::ColorStopPosition(i) => {
            lang.f("psd_export-color-position", &[("i", &(i + 1))])
        }
        RoundedParameter::ColorStopMidpoint(i) => {
            lang.f("psd_export-color-midpoint", &[("i", &(i + 1))])
        }
        RoundedParameter::OpacityStopPosition(i) => {
            lang.f("psd_export-opacity-position", &[("i", &(i + 1))])
        }
        RoundedParameter::OpacityStopMidpoint(i) => {
            lang.f("psd_export-opacity-midpoint", &[("i", &(i + 1))])
        }
        RoundedParameter::OpacityStopValue(i) => {
            lang.f("psd_export-opacity-value", &[("i", &(i + 1))])
        }
    }
}

/// 数を短く（小数 3 桁まで・末尾の 0 は落とす）。
fn number(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// 丸めた値の並び（多いときは先頭の数個と残りの数）。
fn changes_text(lang: Lang, changes: &[RoundedValue]) -> String {
    const SHOWN: usize = 3;
    let mut parts: Vec<String> = changes
        .iter()
        .take(SHOWN)
        .map(|c| {
            format!(
                "{} {}→{}",
                parameter_name(lang, c.parameter),
                number(c.from),
                number(c.to)
            )
        })
        .collect();
    if changes.len() > SHOWN {
        let rest = changes.len() - SHOWN;
        parts.push(lang.f("psd_export-more", &[("rest", &rest)]));
    }
    parts.join(lang.t("psd_export-text-2"))
}

/// 注記の「機能」と「結果」（確認のウィンドウの 2 つの列。文は名詞句）。
pub fn note_columns(lang: Lang, note: &ExportNote) -> (String, String) {
    let pixels: String = lang.t("psd_export-to-pixels").into();
    let mask_pixels: String = lang.t("psd_export-to-mask-pixels").into();
    let dropped: String = lang.t("psd_export-dropped").into();
    match &note.action {
        NoteAction::BakedFilters(v) => (
            format!("{}: {}", lang.t("psd_export-filters"), names(lang, v)),
            pixels,
        ),
        NoteAction::BakedFill(s) => {
            let mut parts: Vec<String> = Vec::new();
            if s.image {
                parts.push(lang.t("psd_export-image").into());
            }
            if s.decal {
                parts.push(lang.t("psd_export-decal").into());
            }
            if s.gradient {
                parts.push(lang.t("psd_export-gradient").into());
            }
            let mut what = parts.join(lang.t("psd_export-text"));
            if let Some(mode) = s.projection {
                what += &format!(
                    "（{}）",
                    crate::panels::fill_props::projection_name(lang, mode)
                );
            }
            (format!("{}: {what}", lang.t("psd_export-fill")), pixels)
        }
        NoteAction::BakedTranslucentFill => (lang.t("psd_export-translucent-fill").into(), pixels),
        NoteAction::BakedPath => (
            lang.t("psd_export-path").into(),
            lang.t("psd_export-pixels-only").into(),
        ),
        NoteAction::BakedMaskFilters(v) => (
            format!("{}: {}", lang.t("psd_export-mask-filters"), names(lang, v)),
            mask_pixels,
        ),
        NoteAction::BakedInvertedMask => (lang.t("psd_export-inverted-mask").into(), mask_pixels),
        NoteAction::BakedClippedGroup => (
            lang.t("psd_export-clipped-group").into(),
            lang.t("psd_export-to-one-layer").into(),
        ),
        NoteAction::DroppedClippingMark => {
            (lang.t("psd_export-idle-clipping-mark").into(), dropped)
        }
        NoteAction::DroppedFilters(v) => (
            format!(
                "{}: {}",
                lang.t("psd_export-inactive-filters"),
                names(lang, v)
            ),
            dropped,
        ),
        NoteAction::DroppedMaskFilters(v) => (
            format!(
                "{}: {}",
                lang.t("psd_export-inactive-mask-filters"),
                names(lang, v)
            ),
            dropped,
        ),
        NoteAction::DroppedAnchor => (lang.t("psd_export-anchor").into(), dropped),
        NoteAction::DroppedMaskAnchor => (lang.t("psd_export-mask-anchor").into(), dropped),
        NoteAction::NormalBlend => (
            lang.t("psd_export-normal-blending").into(),
            lang.t("psd_export-as-colors").into(),
        ),
        NoteAction::ExpandedGradientCurve {
            cause,
            colors,
            opacities,
            max_diff,
        } => {
            // 展開した理由の語（使っていないものは出さない）
            let (ja, en) = match cause {
                GradientExpansion::Curve => ("カーブ", "Curve"),
                GradientExpansion::Mixing => ("混色", "Mixing"),
                GradientExpansion::CurveAndMixing => ("カーブ・混色", "Curve/mixing"),
            };
            (
                format!(
                    "{}: {}",
                    adjustment_name(lang, AdjustmentType::GradientMap),
                    lang.pick(
                        format!("{ja} → 停止点 色 {colors}・不透明度 {opacities}"),
                        format!("{en} → stops: {colors} color, {opacities} opacity")
                    )
                ),
                lang.f("psd_export-max-diff", &[("max_diff", &max_diff)]),
            )
        }
        NoteAction::Rounded {
            kind,
            changes,
            max_diff,
        } => {
            let mut what = adjustment_name(lang, *kind).to_owned();
            if !changes.is_empty() {
                what += &format!(": {}", changes_text(lang, changes));
            }
            (
                what,
                lang.f("psd_export-max-diff", &[("max_diff", &max_diff)]),
            )
        }
        NoteAction::BakedText => (
            lang.t("psd_export-text-layer").into(),
            lang.t("psd_export-pixels-only").into(),
        ),
    }
}

/// 書けない理由（レイヤーの名前つき。画面の言語）。`channel` は複数のチャンネルを書くときだけ付ける。
pub fn blocker_text(lang: Lang, doc: &Document, channel: Option<Channel>, b: &Blocker) -> String {
    let name = &b.layer;
    let what = match &b.refusal {
        Refusal::NormalLevels => lang.f("psd_export-has-levels-that-a-directx", &[("name", &name)]),
        Refusal::ClippedGroup => lang.f("psd_export-is-a-clipped-group", &[("name", &name)]),
        Refusal::InvertedMask => lang.f("psd_export-has-an-inverted-mask", &[("name", &name)]),
        Refusal::FillTranslucent => lang.f("psd_export-is-a-translucent-fill", &[("name", &name)]),
        Refusal::FillPixels => lang.f("psd_export-is-a-fill-that-reads", &[("name", &name)]),
        Refusal::LevelsBetweenSteps => {
            lang.f("psd_export-has-levels-between-psd-s", &[("name", &name)])
        }
        Refusal::LevelsRange => lang.f("psd_export-has-a-levels-input-outside", &[("name", &name)]),
        Refusal::HueSaturationBetweenSteps => lang.f(
            "psd_export-has-hue-saturation-between-psd",
            &[("name", &name)],
        ),
        Refusal::GradientMapBetweenSteps => {
            lang.f("psd_export-has-a-gradient-map-between", &[("name", &name)])
        }
        Refusal::GradientMapCurveStops => {
            lang.f("psd_export-has-a-gradient-map-whose", &[("name", &name)])
        }
        Refusal::GradientMapMixingStops => {
            lang.f("psd_export-has-a-gradient-map-whose-2", &[("name", &name)])
        }
        Refusal::GradientMapMixing => {
            lang.f("psd_export-has-a-gradient-map-with", &[("name", &name)])
        }
        Refusal::GradientMapCurve => {
            lang.f("psd_export-has-a-gradient-map-with-2", &[("name", &name)])
        }
        Refusal::ToneCurveBetweenSteps => {
            lang.f("psd_export-has-a-tone-curve-between", &[("name", &name)])
        }
        Refusal::ColorBalanceBetweenSteps => lang.f(
            "psd_export-has-color-balance-between-psd",
            &[("name", &name)],
        ),
        Refusal::BrightnessContrastBetweenSteps => lang.f(
            "psd_export-has-brightness-contrast-between-psd",
            &[("name", &name)],
        ),
        Refusal::Effects => lang.f("psd_export-has-filters-or-generators", &[("name", &name)]),
        Refusal::Anchor => lang.f("psd_export-has-an-anchor", &[("name", &name)]),
        Refusal::Path => lang.f("psd_export-has-a-path", &[("name", &name)]),
        Refusal::Text => lang.f("psd_export-is-a-text-layer", &[("name", &name)]),
    };
    match channel {
        Some(c) => format!("{}: {what}", channel_name(lang, doc, c)),
        None => what,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expanded(cause: GradientExpansion) -> ExportNote {
        ExportNote {
            layer: "マップ".into(),
            action: NoteAction::ExpandedGradientCurve {
                cause,
                colors: 6,
                opacities: 2,
                max_diff: 3,
            },
        }
    }

    /// 展開した理由の語は、使っているものだけ（値のカーブだけなら混色の語を出さない。混色だけならカーブの語を出さない）。
    #[test]
    fn the_expansion_note_names_only_what_the_gradient_map_used() {
        let cases = [
            (
                GradientExpansion::Curve,
                "カーブ → 停止点 色 6・不透明度 2",
                "Curve → stops: 6 color, 2 opacity",
            ),
            (
                GradientExpansion::Mixing,
                "混色 → 停止点 色 6・不透明度 2",
                "Mixing → stops: 6 color, 2 opacity",
            ),
            (
                GradientExpansion::CurveAndMixing,
                "カーブ・混色 → 停止点 色 6・不透明度 2",
                "Curve/mixing → stops: 6 color, 2 opacity",
            ),
        ];
        for (cause, ja, en) in cases {
            let (what, how) = note_columns(Lang::Ja, &expanded(cause));
            assert!(what.ends_with(ja), "{what}");
            assert_eq!(how, "最大差 3");
            let (what, how) = note_columns(Lang::En, &expanded(cause));
            assert!(what.ends_with(en), "{what}");
            assert_eq!(how, "Max diff 3");
        }
    }
}
