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
        Item::Label(lang.pick("方式", "Mode").into()),
        Item::Mode(
            ExportMode::Bake,
            lang.pick("焼き込んで書く", "Bake and write"),
        ),
        Item::Mode(
            ExportMode::Flat,
            lang.pick("平らに 1 枚", "Flatten to one layer"),
        ),
        Item::Label(lang.pick("チャンネル", "Channels").into()),
    ];
    for c in app.doc.channels() {
        items.push(Item::Channel(
            c,
            channel_name(lang, &app.doc, c),
            app.psd.export.channels.contains(&c),
        ));
    }
    let visible = items.len().min(MAX_ROWS);
    let height = window::HEADER_HEIGHT + 8.0 + visible as f32 * ROW_HEIGHT + 10.0 + FOOTER;
    let title = lang.pick("PSD の書き出し", "Export PSD");
    let close_label = lang.pick("ウィンドウを閉じる", "Close Window");
    let spec = Spec {
        title,
        icon: Some("folder_open"),
        size: vec2(380.0, height),
        modal: true,
        close_label,
    };
    let id = Id::new(("yolu.window", OPTIONS));
    let mut offset = app.psd.options_offset;
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
                lang.pick("やめる", "Cancel"),
                false,
                None,
                PsdAction::CancelExportOptions,
            ),
            (
                lang.pick("書き出し…", "Export…"),
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
    app.psd.options_offset = offset;
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
        ExportMode::Bake => lang.pick(
            "レイヤーを残し、PSD に形の無いフィルター・ジェネレーター・画像・パスなどは、評価した画素にして書きます。書く前に、焼くものを一覧で確かめます",
            "Keeps the layers. Filters, generators, images, paths and other features PSD has no form for are written as evaluated pixels. What changes is listed before writing",
        ),
        ExportMode::Flat => lang.pick(
            "合成した絵だけを、1 枚のレイヤーに書きます",
            "Writes only the composite, as a single layer",
        ),
    }
}

/// 「書き出し…」のツールチップ。
fn export_tooltip(lang: Lang) -> &'static str {
    lang.pick(
        "書き出す先を選びます。複数のチャンネルは 名前_チャンネル.psd で並べて書きます",
        "Choose where to write. Several channels are written as Name_Channel.psd",
    )
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
        parts.push(lang.pick(format!("焼く {baked}"), format!("Bake {baked}")));
    }
    if rounded > 0 {
        parts.push(lang.pick(format!("丸める {rounded}"), format!("Round {rounded}")));
    }
    if dropped > 0 {
        parts.push(lang.pick(format!("落とす {dropped}"), format!("Drop {dropped}")));
    }
    if other > 0 {
        parts.push(lang.pick(format!("注意 {other}"), format!("Note {other}")));
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
        title: lang
            .pick("PSD の書き出しの確かめ", "Export PSD Check")
            .into(),
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
                label: lang.pick("やめる", "Cancel").into(),
                primary: false,
                tooltip: None,
            },
            Button {
                label: lang.pick("書く", "Write").into(),
                primary: true,
                tooltip: Some(
                    lang.pick(
                        "一覧のとおりに PSD を書きます。プロジェクトは変えません（効果はプロジェクトに残ります）",
                        "Writes the PSD as listed. The project is not changed (its effects stay)",
                    )
                    .into(),
                ),
            },
        ],
        close_label: lang.pick("ウィンドウを閉じる", "Close Window").into(),
    };
    let mut offset = app.psd.notes_offset;
    let mut scroll = 0.0;
    let reply = show_list(ctx, &spec, &mut offset, &mut scroll);
    app.psd.notes_offset = offset;
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
        .join(lang.pick("・", ", "))
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
        RoundedParameter::InputBlack => lang.pick("入力の黒", "Input black").into(),
        RoundedParameter::InputWhite => lang.pick("入力の白", "Input white").into(),
        RoundedParameter::OutputBlack => lang.pick("出力の黒", "Output black").into(),
        RoundedParameter::OutputWhite => lang.pick("出力の白", "Output white").into(),
        RoundedParameter::Gamma => lang.pick("ガンマ", "Gamma").into(),
        RoundedParameter::Hue => lang.pick("色相", "Hue").into(),
        RoundedParameter::Saturation => lang.pick("彩度", "Saturation").into(),
        RoundedParameter::Lightness => lang.pick("明度", "Lightness").into(),
        RoundedParameter::Balance { range, axis } => {
            let range = match range {
                BalanceRange::Shadows => lang.pick("シャドウ", "Shadows"),
                BalanceRange::Midtones => lang.pick("中間", "Midtones"),
                BalanceRange::Highlights => lang.pick("ハイライト", "Highlights"),
            };
            let axis = match axis {
                0 => lang.pick("シアン — レッド", "Cyan — Red"),
                1 => lang.pick("マゼンタ — グリーン", "Magenta — Green"),
                _ => lang.pick("イエロー — ブルー", "Yellow — Blue"),
            };
            format!("{range} {axis}")
        }
        RoundedParameter::Brightness => lang.pick("明るさ", "Brightness").into(),
        RoundedParameter::Contrast => lang.pick("コントラスト", "Contrast").into(),
        RoundedParameter::CurveInput { curve, point } => lang.pick(
            format!("{} 点 {} の入力", tone(curve), point + 1),
            format!("{} point {} input", tone(curve), point + 1),
        ),
        RoundedParameter::CurveOutput { curve, point } => lang.pick(
            format!("{} 点 {} の出力", tone(curve), point + 1),
            format!("{} point {} output", tone(curve), point + 1),
        ),
        RoundedParameter::ColorStopPosition(i) => lang.pick(
            format!("色 {} の位置", i + 1),
            format!("Color {} position", i + 1),
        ),
        RoundedParameter::ColorStopMidpoint(i) => lang.pick(
            format!("色 {} の中点", i + 1),
            format!("Color {} midpoint", i + 1),
        ),
        RoundedParameter::OpacityStopPosition(i) => lang.pick(
            format!("不透明度 {} の位置", i + 1),
            format!("Opacity {} position", i + 1),
        ),
        RoundedParameter::OpacityStopMidpoint(i) => lang.pick(
            format!("不透明度 {} の中点", i + 1),
            format!("Opacity {} midpoint", i + 1),
        ),
        RoundedParameter::OpacityStopValue(i) => lang.pick(
            format!("不透明度 {} の値", i + 1),
            format!("Opacity {} value", i + 1),
        ),
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
        parts.push(lang.pick(format!("ほか {rest}"), format!("{rest} more")));
    }
    parts.join(lang.pick("、", ", "))
}

/// 注記の「機能」と「結果」（確認のウィンドウの 2 つの列。文は名詞句）。
pub fn note_columns(lang: Lang, note: &ExportNote) -> (String, String) {
    let pixels: String = lang.pick("画素へ", "To pixels").into();
    let mask_pixels: String = lang.pick("マスクの画素へ", "To mask pixels").into();
    let dropped: String = lang.pick("落とす", "Dropped").into();
    match &note.action {
        NoteAction::BakedFilters(v) => (
            format!("{}: {}", lang.pick("フィルター", "Filters"), names(lang, v)),
            pixels,
        ),
        NoteAction::BakedFill(s) => {
            let mut parts: Vec<String> = Vec::new();
            if s.image {
                parts.push(lang.pick("画像", "Image").into());
            }
            if s.decal {
                parts.push(lang.pick("デカール", "Decal").into());
            }
            if s.gradient {
                parts.push(lang.pick("グラデーション", "Gradient").into());
            }
            let mut what = parts.join(lang.pick("・", ", "));
            if let Some(mode) = s.projection {
                what += &format!(
                    "（{}）",
                    crate::panels::fill_props::projection_name(lang, mode)
                );
            }
            (
                format!("{}: {what}", lang.pick("塗りつぶし", "Fill")),
                pixels,
            )
        }
        NoteAction::BakedTranslucentFill => (
            lang.pick("半透明の塗りつぶし", "Translucent fill").into(),
            pixels,
        ),
        NoteAction::BakedPath => (
            lang.pick("パス", "Path").into(),
            lang.pick("画素のみ", "Pixels only").into(),
        ),
        NoteAction::BakedMaskFilters(v) => (
            format!(
                "{}: {}",
                lang.pick("マスクのフィルター", "Mask filters"),
                names(lang, v)
            ),
            mask_pixels,
        ),
        NoteAction::BakedInvertedMask => (
            lang.pick("反転したマスク", "Inverted mask").into(),
            mask_pixels,
        ),
        NoteAction::BakedClippedGroup => (
            lang.pick("クリッピングされたグループ", "Clipped group")
                .into(),
            lang.pick("1 枚へ", "To one layer").into(),
        ),
        NoteAction::DroppedClippingMark => (
            lang.pick("効いていないクリッピングの印", "Idle clipping mark")
                .into(),
            dropped,
        ),
        NoteAction::DroppedFilters(v) => (
            format!(
                "{}: {}",
                lang.pick("効いていないフィルター", "Inactive filters"),
                names(lang, v)
            ),
            dropped,
        ),
        NoteAction::DroppedMaskFilters(v) => (
            format!(
                "{}: {}",
                lang.pick("効いていないマスクのフィルター", "Inactive mask filters"),
                names(lang, v)
            ),
            dropped,
        ),
        NoteAction::DroppedAnchor => (lang.pick("アンカー", "Anchor").into(), dropped),
        NoteAction::DroppedMaskAnchor => {
            (lang.pick("マスクのアンカー", "Mask anchor").into(), dropped)
        }
        NoteAction::NormalBlend => (
            lang.pick("法線の重ね方", "Normal blending").into(),
            lang.pick("色の式で重なる", "As colors").into(),
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
                lang.pick(format!("最大差 {max_diff}"), format!("Max diff {max_diff}")),
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
                lang.pick(format!("最大差 {max_diff}"), format!("Max diff {max_diff}")),
            )
        }
        NoteAction::BakedText => (
            lang.pick("テキストレイヤー", "Text layer").into(),
            lang.pick("画素のみ", "Pixels only").into(),
        ),
    }
}

/// 書けない理由（レイヤーの名前つき。画面の言語）。`channel` は複数のチャンネルを書くときだけ付ける。
pub fn blocker_text(lang: Lang, doc: &Document, channel: Option<Channel>, b: &Blocker) -> String {
    let name = &b.layer;
    let what = match &b.refusal {
        Refusal::NormalLevels => lang.pick(
            format!("「{name}」のレベル補正は、DirectX 向きの Normal の PSD に書けません"),
            format!("\"{name}\" has levels that a DirectX Normal PSD cannot hold"),
        ),
        Refusal::ClippedGroup => lang.pick(
            format!("「{name}」はクリッピングされたグループです"),
            format!("\"{name}\" is a clipped group"),
        ),
        Refusal::InvertedMask => lang.pick(
            format!("「{name}」のマスクは反転しています"),
            format!("\"{name}\" has an inverted mask"),
        ),
        Refusal::FillTranslucent => lang.pick(
            format!("「{name}」は半透明の塗りつぶしです"),
            format!("\"{name}\" is a translucent fill"),
        ),
        Refusal::FillPixels => lang.pick(
            format!("「{name}」の塗りつぶしは画像・投影・グラデーションを使っています"),
            format!("\"{name}\" is a fill that reads an image, projection or gradient"),
        ),
        Refusal::LevelsBetweenSteps => lang.pick(
            format!("「{name}」のレベル補正は PSD の刻みの間にあります"),
            format!("\"{name}\" has levels between PSD's steps"),
        ),
        Refusal::LevelsRange => lang.pick(
            format!("「{name}」のレベル補正の入力が PSD の範囲に収まりません"),
            format!("\"{name}\" has a levels input outside PSD's range"),
        ),
        Refusal::HueSaturationBetweenSteps => lang.pick(
            format!("「{name}」の色相・彩度は PSD の刻みの間にあります"),
            format!("\"{name}\" has hue/saturation between PSD's steps"),
        ),
        Refusal::GradientMapBetweenSteps => lang.pick(
            format!("「{name}」のグラデーションマップは PSD の刻みの間にあります"),
            format!("\"{name}\" has a gradient map between PSD's steps"),
        ),
        Refusal::GradientMapCurveStops => lang.pick(
            format!("「{name}」のグラデーションマップの値のカーブは、PSD の停止点の上限の中で展開できません"),
            format!("\"{name}\" has a gradient map whose value curve does not fit PSD's stop limit"),
        ),
        Refusal::GradientMapMixingStops => lang.pick(
            format!("「{name}」のグラデーションマップの混色は、PSD の停止点の上限の中で展開できません"),
            format!("\"{name}\" has a gradient map whose color mixing does not fit PSD's stop limit"),
        ),
        Refusal::GradientMapMixing => lang.pick(
            format!("「{name}」のグラデーションマップに混色（混色モード・混合率曲線）があります"),
            format!("\"{name}\" has a gradient map with color mixing (mode or mixing curves)"),
        ),
        Refusal::GradientMapCurve => lang.pick(
            format!("「{name}」のグラデーションマップに値のカーブがあります"),
            format!("\"{name}\" has a gradient map with a value curve"),
        ),
        Refusal::ToneCurveBetweenSteps => lang.pick(
            format!("「{name}」のトーンカーブは PSD の刻みの間にあります"),
            format!("\"{name}\" has a tone curve between PSD's steps"),
        ),
        Refusal::ColorBalanceBetweenSteps => lang.pick(
            format!("「{name}」のカラーバランスは PSD の刻みの間にあります"),
            format!("\"{name}\" has color balance between PSD's steps"),
        ),
        Refusal::BrightnessContrastBetweenSteps => lang.pick(
            format!("「{name}」の明るさ・コントラストは PSD の刻みの間にあります"),
            format!("\"{name}\" has brightness/contrast between PSD's steps"),
        ),
        Refusal::Effects => lang.pick(
            format!("「{name}」にフィルターかジェネレーターがあります"),
            format!("\"{name}\" has filters or generators"),
        ),
        Refusal::Anchor => lang.pick(
            format!("「{name}」にアンカーがあります"),
            format!("\"{name}\" has an anchor"),
        ),
        Refusal::Path => lang.pick(
            format!("「{name}」にパスがあります"),
            format!("\"{name}\" has a path"),
        ),
        Refusal::Text => lang.pick(
            format!("「{name}」はテキストレイヤーです"),
            format!("\"{name}\" is a text layer"),
        ),
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
