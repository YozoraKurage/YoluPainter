//! 見た目の設定の欄（右のドックの「マテリアル」のパネル）: 種類（標準・lilToon）と、lilToon のときはインスペクター（lilToon 2.3.4 の
//! 詳細設定）と同じ節の分け方・並び・名前の欄（見た目はよるぺの欄の部品）。描画モード → 基本設定 → ライティング・明るさ設定 → UV 設定 →
//! [色設定] メインカラー / 透過設定・影設定・リムシェード・発光設定 → [ノーマルマップ・光沢設定] ノーマルマップ設定・逆光ライト・
//! 光沢設定・マットキャップ設定・リムライト設定・ラメ設定 → [拡張設定] 輪郭線設定・距離フェード。機能の入切は、その機能の節の頭
//! （輪郭線も輪郭線設定の頭。Unity ではシェーダーの名前が変わる）。値を変えると 3D ビューがその場で変わり、1 回の Undo（スライダーの
//! ドラッグは 1 段）。画面には名前と値だけを出し、説明はツールチップ（lilToon のプロパティの名前も）。再現しない機能の欄は出さない。
//!
//! lilToon の 1 行の「テクスチャと色」（`TexturePropertySingleLine`）は、スロットの行（lilToon の行の名前・割り当て・描く口）と、その下の
//! 色の行（色・16 進・不透明度）の 2 行にする。スロットの行の右の筆のボタンが、そのスロットを描く口（割り当てが無ければチャンネルを
//! 作って割り当て、描くチャンネルにする。今描いているスロットはボタンが押し込まれた見た目）。
//!
//! Live Link で Unity の本物のマテリアルの値を受けたセット（`Document::received_look`）は、欄に描く見た目（Unity の値の上に欄で変えた値）
//! を見せ、欄で変えた項目の名前に印（•）を付けてツールチップに Unity の値を添える。「Unity の値」の行は、受けている・最後の値の様子と、
//! 欄で変えた値を外して Unity に合わせるボタン（`LookOp::FollowReceived`）。

use egui::{pos2, vec2, Rect, Ui};
use yolu_core::look::{LookKind, LookValue, PlaneSource, TextureSource};
use yolu_core::ChannelKind;

use super::fields::{self, copy_mode, mirror_mode, specular_mode};
use super::liltoon::{self, Kind, RenderMode, SlotDefault, SlotUse, SLOTS};
use super::{LightingPreset, LookOp, Section};
use crate::lang::Lang;
use crate::m2::channel_name;
use crate::m2_menu::Popup;
use crate::panels::properties::{group_label, open_popup, section, subsection, toggle_row};
use crate::state::{Action, AppState};
use crate::ui::menu::Entry;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};

/// 欄のドロップダウン（`Popup::Look`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookChoice {
    Kind,
    Mode,
    /// 選ぶプロパティ（Cull・合成モードなど）。
    Prop(&'static str),
    /// テクスチャのスロット（番号は `liltoon::SLOTS`）。
    Slot(usize),
    /// 成分ごとの詰め合わせの 1 成分（スロット・成分）。
    Plane(usize, u8),
    /// 光沢のタイプ（無効・リアル・トゥーン。`_ApplySpecular`・`_SpecularToon`）。
    SpecularMode,
    /// デカールのミラーモード（メインカラー 2nd・3rd の接頭辞）。
    Mirror(&'static str),
    /// デカールの複製モード。
    Copy(&'static str),
}

const LABEL_W: f32 = 92.0;
/// 機能の節の中身の字下げ（lilToon の内側の箱）。
const INNER: f32 = 8.0;
/// スロットの行の右の、描く口のボタンの幅。
const PAINT_W: f32 = 22.0;

fn look_action(op: LookOp) -> Action {
    Action::Look(op)
}

/// 「見た目」の節。
pub fn look_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    let (open, _) = section(
        ui,
        app,
        rows,
        "look",
        lang.pick("見た目", "Look"),
        "view_in_ar",
        None,
    );
    if !open {
        return;
    }
    let free = !app.is_stroking() && app.read_only_reason().is_none();
    let look = app.doc.drawn_look().clone();
    let kind_name = kind_label(lang, look.kind);
    if let Some(at) = choice(
        ui,
        rows,
        "look.kind",
        lang.pick("種類", "Kind"),
        &kind_name,
        lang.pick(
            "このテクスチャセットの面を 3D ビューで描く方式（標準は PBR、lilToon は lilToon 2.3 の見た目の再現）",
            "How the 3D View draws this texture set (Standard is PBR, lilToon reproduces the look of lilToon 2.3)",
        ),
        free,
    ) {
        open_popup(app, &ui.ctx().clone(), Popup::Look(LookChoice::Kind), at, at.width());
    }
    received_row(ui, app, rows, free);
    if look.kind != LookKind::LilToon {
        rows.space(4.0);
        return;
    }
    let row = rows.row(24.0, 4.0);
    if w::button(
        ui,
        row,
        "look.template",
        lang.pick("lilToon のひな形", "lilToon Template"),
        false,
        free,
        Some(lang.pick(
            "入にしている機能（メインカラー 2nd・3rd・影・リムシェード・発光・異方性反射・逆光ライト・光沢・マットキャップ・リムライト・ラメ・輪郭線）のマスクのユーザーチャンネルを作って割り当てる。機能が 1 つも入でなければ影を入にする",
            "Creates user channels for the masks of enabled features (Main Color 2nd/3rd, shadow, rim shade, emission, anisotropy, backlight, reflection, MatCap, rim light, glitter, outline) and assigns them; turns the shadow on when nothing is on",
        )),
        Some("auto_awesome"),
    )
    .clicked()
    {
        app.apply(look_action(LookOp::Template));
    }
    rows.space(2.0);
    rendering_mode(ui, app, rows, free);
    base(ui, app, rows, free);
    lighting(ui, app, rows, free);
    uv(ui, app, rows, free);
    group(ui, rows, lang.pick("色設定", "Color"));
    main_color(ui, app, rows, free);
    shadow(ui, app, rows, free);
    rim_shade(ui, app, rows, free);
    emission(ui, app, rows, free);
    group(
        ui,
        rows,
        lang.pick("ノーマルマップ・光沢設定", "Normal Map & Reflection"),
    );
    normal_map(ui, app, rows, free);
    backlight(ui, app, rows, free);
    reflection(ui, app, rows, free);
    matcap(ui, app, rows, free);
    rim(ui, app, rows, free);
    glitter(ui, app, rows, free);
    group(ui, rows, lang.pick("拡張設定", "Advanced"));
    outline(ui, app, rows, free);
    distance_fade(ui, app, rows, free);
    rows.indent = t::SECTION_INDENT;
    rows.space(4.0);
}

/// 節の組の見出し（lilToon の太字の見出し: 色設定・ノーマルマップ・光沢設定・拡張設定）。
fn group(ui: &mut Ui, rows: &mut Rows, text: &str) {
    rows.indent = t::SECTION_INDENT;
    rows.space(4.0);
    group_label(ui, rows, text);
}

/// Unity から受けた値の行（受けているセットだけ）: 名前・様子（接続中・最後の値）と、欄で変えた値を外して Unity に合わせるボタン。
fn received_row(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    let lang = app.lang;
    let Some(received) = app.doc.received_look() else {
        return;
    };
    let live = app.sets.current().bound.is_some()
        && app.model.as_ref().is_some_and(|m| m.is_link() && m.live);
    let mine = app.doc.look();
    let changed = mine.properties.len()
        + usize::from(!mine.shader.is_empty())
        + usize::from(!mine.keywords.is_empty())
        + usize::from(mine.kind_chosen);
    let mut tip = vec![if received.source.is_empty() {
        lang.pick("Unity のマテリアルの値", "Values of the Unity material")
            .to_owned()
    } else {
        received.source.clone()
    }];
    tip.push(lang.pick(
        format!("欄で変えた値 {changed}（印 • の付いた項目）"),
        format!("{changed} values changed here (marked •)"),
    ));
    for (slot, why) in &received.missing {
        let label = liltoon::slot(slot).map_or(slot.as_str(), |s| s.label(lang));
        tip.push(format!("{label}: {}", missing_text(lang, *why)));
    }
    let tip = tip.join("\n");
    let state = if live {
        lang.pick("接続中", "Connected")
    } else {
        lang.pick("最後の値", "Last received")
    };
    let row = rows.row(t::ROW_HEIGHT, 2.0);
    let name = lang.pick("Unity の値", "Unity Values");
    w::text(
        ui.painter(),
        Rect::from_min_size(row.min, vec2(LABEL_W, row.height())),
        name,
        t::LABEL.with_color(t::TEXT),
        w::Align::Left,
    );
    w::text(
        ui.painter(),
        Rect::from_min_max(pos2(row.left() + LABEL_W, row.top()), row.max),
        state,
        t::LABEL.with_color(if live { t::TEXT } else { t::TEXT_DIM }),
        w::Align::Left,
    );
    let response = ui.interact(
        row,
        ui.make_persistent_id("look.received"),
        egui::Sense::hover(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{name}: {state}"))
    });
    response.on_hover_text(tip);
    let b = rows.row(24.0, 4.0);
    if w::button(
        ui,
        b,
        "look.follow",
        lang.pick("Unity に合わせる", "Match Unity"),
        false,
        free && changed > 0,
        Some(lang.pick(
            "欄で変えた値・描画モード・輪郭線・種類の選びを外し、Unity の値で描く（スロットの割り当ては残す）",
            "Drops the values, rendering mode, outline and kind changed here and draws with the Unity values (slot assignments stay)",
        )),
        Some("sync"),
    )
    .clicked()
    {
        app.apply(look_action(LookOp::FollowReceived));
    }
}

/// スロットの行の値（短く。理由の全文はツールチップ）。
fn missing_short(lang: Lang, why: yolu_core::look::MissingImage) -> &'static str {
    use yolu_core::look::MissingImage;
    match why {
        MissingImage::OverBudget => lang.pick("Unity（予算を超える）", "Unity (over budget)"),
        MissingImage::Unreadable => lang.pick("Unity（読めない）", "Unity (unreadable)"),
        MissingImage::NotAFile => lang.pick("Unity（ファイルなし）", "Unity (no file)"),
    }
}

fn missing_text(lang: Lang, why: yolu_core::look::MissingImage) -> &'static str {
    use yolu_core::look::MissingImage;
    match why {
        MissingImage::OverBudget => lang.pick(
            "Unity のテクスチャ（受けたテクスチャの予算を超えるので読まない）",
            "Unity texture (over the budget for received textures, not read)",
        ),
        MissingImage::Unreadable => lang.pick(
            "Unity のテクスチャ（ファイルを読めない）",
            "Unity texture (the file cannot be read)",
        ),
        MissingImage::NotAFile => lang.pick(
            "Unity のテクスチャ（Unity の中にしかなく、ファイルが無い）",
            "Unity texture (only inside Unity, no file)",
        ),
    }
}

/// Unity から受けた値があり、欄でこの項目を変えている（欄の値が勝っている）なら、ツールチップに添える Unity の値。
fn overridden(app: &AppState, name: &str) -> Option<String> {
    let received = app.doc.received_look()?;
    if !app.doc.look().properties.contains_key(name) {
        return None;
    }
    let v = received.look.get(name).map_or_else(
        || app.lang.pick("（既定）", "(default)").to_owned(),
        |v| match v {
            LookValue::Float(x) => format!("{x:.3}"),
            LookValue::Int(x) => x.to_string(),
            LookValue::Color(c) => {
                let c = if liltoon::is_linear_color(name) {
                    to_gamma(c)
                } else {
                    c
                };
                format!(
                    "{} · {:.2}",
                    crate::state::to_hex([c[0], c[1], c[2], 1.0]),
                    c[3]
                )
            }
            LookValue::Vector(c) => format!("({:.3}, {:.3}, {:.3}, {:.3})", c[0], c[1], c[2], c[3]),
        },
    );
    Some(format!("Unity: {v}"))
}

/// 名前（欄で変えた項目は印つき）とツールチップ（プロパティの名前と、変えていれば Unity の値）。
fn marked(app: &AppState, name: &str, label: &str) -> (String, String) {
    match overridden(app, name) {
        Some(unity) => (format!("{label} •"), format!("{name}\n{unity}")),
        None => (label.to_owned(), name.to_owned()),
    }
}

pub fn kind_label(lang: Lang, kind: LookKind) -> String {
    match kind {
        LookKind::Standard => lang.pick("標準（PBR）", "Standard (PBR)").into(),
        LookKind::LilToon => "lilToon".into(),
    }
}

fn choice(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    label: &str,
    value: &str,
    tooltip: &str,
    enabled: bool,
) -> Option<Rect> {
    let r = rows.row(t::ROW_HEIGHT, 4.0);
    // 名前が基準の幅に入らないとき（英語の「Rendering Mode」など）は、名前の幅まで広げる。ただし行の 6 割まで（値の箱を読めるように残す。それでも入らなければ
    // 名前を「…」で詰める）
    let label_width = (w::text_width(ui.painter(), label, t::LABEL) + 10.0)
        .clamp(LABEL_W, (r.width() * 0.6).max(LABEL_W));
    let shown = w::fit(ui.painter(), label, label_width - 10.0, t::LABEL);
    let (response, b) = w::dropdown(
        ui,
        r,
        id,
        Some(&shown),
        value,
        Some(tooltip),
        enabled,
        label_width,
    );
    response.clicked().then_some(b)
}

fn sub(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    key: &'static str,
    ja: &str,
    en: &str,
    section: Section,
) -> bool {
    let lang = app.lang;
    let (open, reset) = subsection(
        ui,
        app,
        rows,
        key,
        lang.pick(ja, en),
        Some(lang.pick("この節の値を既定に戻す", "Reset this section")),
    );
    if reset && !app.is_stroking() {
        app.apply(look_action(LookOp::Reset(section)));
    }
    open
}

/// 字下げを足して中身を描く（lilToon の内側の箱）。
fn inner(rows: &mut Rows, f: impl FnOnce(&mut Rows)) {
    rows.indent += INNER;
    f(rows);
    rows.indent -= INNER;
}

/// 機能の入切（その機能の頭の行）。入なら true。
fn feature(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    name: &'static str,
    free: bool,
) -> bool {
    toggle_prop(ui, app, rows, name, free);
    liltoon::on(app.doc.drawn_look(), name)
}

// ───────── 値の行 ─────────

fn float_row(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, name: &'static str, enabled: bool) {
    float_row_in(ui, app, rows, "", name, None, enabled);
}

/// `float_row` の、節の鍵（`scope`）を ID に混ぜ、名前を差し替えられる形。lilToon のインスペクターは同じプロパティを 2 つの節に出す
/// ことがある（`_ShadowEnvStrength` はライティングと影設定）ので、両方の節を開いても欄の ID が重ならないように。角度（`[lilAngle]`）は
/// 度で、ミップの段（`[lilLOD]`）は値の 4 乗根で見せる（lilToon の欄と同じ）。
fn float_row_in(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    scope: &'static str,
    name: &'static str,
    label: Option<&str>,
    enabled: bool,
) {
    let Some(prop) = liltoon::prop(name) else {
        return;
    };
    let (min, max, power) = match prop.kind {
        Kind::Slider { min, max, power } => (min, max, power),
        Kind::Number { min, max } => (min, max, 1.0),
        Kind::Angle => (-180.0, 180.0, 1.0),
        _ => (0.0, 1.0, 1.0),
    };
    let raw = liltoon::number(app.doc.drawn_look(), name);
    let shown = match prop.kind {
        Kind::Angle => raw.to_degrees(),
        Kind::Lod => raw.max(0.0).powf(0.25),
        _ => raw,
    };
    let (label, tip) = marked(app, name, label.unwrap_or(prop.label(app.lang)));
    let spec = SliderSpec::new(
        &label,
        min,
        max,
        NumberFormat {
            decimals: if max - min > 20.0 { 1 } else { 2 },
            trim: false,
            suffix: if prop.kind == Kind::Angle { "°" } else { "" },
        },
    )
    .tooltip(&tip)
    .enabled(enabled)
    .power(power);
    let out = w::slider(ui, rows.slider_row(), ("look.f", scope, name), shown, &spec);
    if out.changed {
        let value = match prop.kind {
            Kind::Angle => out.value.to_radians(),
            Kind::Lod => out.value.max(0.0).powi(4),
            _ => out.value,
        };
        app.apply(look_action(LookOp::Value {
            name,
            value: LookValue::Float(value),
            drag: true,
        }));
    }
}

/// 範囲を逆に見せる行（lilToon の `InvBorderGUI`: 欄の値は 1 − 値。リムライト・逆光ライトの範囲）。
fn inv_border_row(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    name: &'static str,
    enabled: bool,
) {
    let raw = liltoon::number(app.doc.drawn_look(), name);
    let (label, tip) = marked(app, name, app.lang.pick("範囲", "Border"));
    let spec = SliderSpec::new(
        &label,
        0.0,
        1.0,
        NumberFormat {
            decimals: 2,
            trim: false,
            suffix: "",
        },
    )
    .tooltip(&tip)
    .enabled(enabled);
    let out = w::slider(ui, rows.slider_row(), ("look.inv", name), 1.0 - raw, &spec);
    if out.changed {
        app.apply(look_action(LookOp::Value {
            name,
            value: LookValue::Float(1.0 - out.value),
            drag: true,
        }));
    }
}

fn toggle_prop(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    name: &'static str,
    enabled: bool,
) {
    let Some(prop) = liltoon::prop(name) else {
        return;
    };
    let on = liltoon::on(app.doc.drawn_look(), name);
    let (label, tip) = marked(app, name, prop.label(app.lang));
    if let Some(next) = toggle_row(
        ui,
        rows,
        &format!("look.t.{name}"),
        &label,
        on,
        Some(&tip),
        enabled,
    ) {
        app.apply(look_action(LookOp::Value {
            name,
            value: LookValue::Float(f32::from(next)),
            drag: false,
        }));
    }
}

fn choice_prop(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    name: &'static str,
    enabled: bool,
) {
    let Some(prop) = liltoon::prop(name) else {
        return;
    };
    let Kind::Choice(options) = prop.kind else {
        return;
    };
    let lang = app.lang;
    let at = liltoon::number(app.doc.drawn_look(), name).round().max(0.0) as usize;
    let mut value = options
        .get(at)
        .map_or("?", |o| lang.pick(o.0, o.1))
        .to_owned();
    let (label, mut tip) = marked(app, name, prop.label(lang));
    if !choice_drawn(name, at) {
        value = format!("{value}{}", lang.pick("（描かない）", " (not drawn)"));
        tip += &format!("\n{}", not_drawn_reason(lang, name));
    }
    if let Some(r) = choice(
        ui,
        rows,
        &format!("look.c.{name}"),
        &label,
        &value,
        &tip,
        enabled,
    ) {
        open_popup(
            app,
            &ui.ctx().clone(),
            Popup::Look(LookChoice::Prop(name)),
            r,
            r.width(),
        );
    }
}

fn to_gamma(c: [f32; 4]) -> [f32; 4] {
    [
        crate::view3d::brdf::linear_to_srgb(c[0]),
        crate::view3d::brdf::linear_to_srgb(c[1]),
        crate::view3d::brdf::linear_to_srgb(c[2]),
        c[3],
    ]
}

fn to_linear(c: [f32; 4]) -> [f32; 4] {
    [
        crate::view3d::brdf::srgb_to_linear(c[0]),
        crate::view3d::brdf::srgb_to_linear(c[1]),
        crate::view3d::brdf::srgb_to_linear(c[2]),
        c[3],
    ]
}

/// 色の行（見本を押すと色のウィンドウ、16 進で打つ）。`alpha` があれば、その名前で不透明度の行も出す。発光の色（Unity の [HDR]）は値が
/// リニアなので、見本と 16 進はガンマに直して見せ、明るさ（1 を超える倍率）の行も出す。`label` は名前の差し替え（無ければ表の名前）。
fn color_row(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    name: &'static str,
    label: Option<&str>,
    alpha: Option<(&str, &str)>,
    enabled: bool,
) {
    let Some(prop) = liltoon::prop(name) else {
        return;
    };
    let lang = app.lang;
    let raw = liltoon::value(app.doc.drawn_look(), name);
    let linear = liltoon::is_linear_color(name);
    let intensity = if linear {
        raw[0].max(raw[1]).max(raw[2]).max(1.0)
    } else {
        1.0
    };
    let unit = [
        raw[0] / intensity,
        raw[1] / intensity,
        raw[2] / intensity,
        raw[3],
    ];
    let shown = if linear { to_gamma(unit) } else { unit };
    let row = rows.row(t::ROW_HEIGHT, 2.0);
    let title = label.unwrap_or(prop.label(lang)).to_owned();
    let (label, unity) = marked(app, name, &title);
    let label_rect = Rect::from_min_size(row.min, vec2(LABEL_W, row.height()));
    w::text(
        ui.painter(),
        label_rect,
        &label,
        t::LABEL.with_color(w::label_color(ui, ("look.color.text", name), enabled)),
        w::Align::Left,
    );
    ui.interact(
        label_rect,
        ui.make_persistent_id(("look.color.label", name)),
        egui::Sense::hover(),
    )
    .on_hover_text(unity);
    let swatch = Rect::from_min_size(
        pos2(row.left() + LABEL_W, row.top() + 1.0),
        vec2(36.0, row.height() - 2.0),
    );
    let set = |app: &mut AppState, gamma: [f32; 3], drag: bool| {
        let mut c = [gamma[0], gamma[1], gamma[2], raw[3]];
        if linear {
            c = to_linear(c);
            c = [c[0] * intensity, c[1] * intensity, c[2] * intensity, c[3]];
        }
        app.apply(look_action(LookOp::Value {
            name,
            value: LookValue::Color(c),
            drag,
        }));
    };
    // 押すと色のウィンドウ（相手は文書とプロパティごと）。ウィンドウの変更はその場で当て、ドラッグ 1 回を 1 回の取り消しにまとめる
    let current = crate::panels::color_window::Pick::from_floats(shown, false);
    if let Some(u) = crate::panels::color_window::field(
        ui,
        swatch,
        egui::Id::new(("look.color", app.doc.id(), name)),
        &title,
        current,
        name,
        enabled,
    ) {
        if u.pick != current {
            set(app, u.pick.rgb_floats(), u.dragging);
        }
        if u.done {
            app.m2_end_drag();
        }
    }
    let hex_rect = Rect::from_min_max(
        pos2(swatch.right() + 4.0, row.top() + 1.0),
        pos2(row.right(), row.bottom() - 1.0),
    );
    // 押せない間は出さない。描いている間は、描き始める前に出ていたものを出したまま（押せない入れ物に入れて、打てなくする）
    let hex_shown = w::look_enabled(ui, ui.make_persistent_id(("look.hex.shown", name)), enabled);
    if hex_rect.width() > 40.0 && hex_shown {
        let current = crate::state::to_hex([shown[0], shown[1], shown[2], 1.0]);
        let out = w::enabled_scope(ui, ("look.hex.scope", name), enabled, |ui| {
            w::text_field(
                ui,
                hex_rect,
                ("look.hex", name),
                &current,
                Some(lang.pick("16 進（#RRGGBB）", "Hex (#RRGGBB)")),
                false,
            )
        })
        .inner;
        if let Some(text) = out.committed.filter(|_| enabled) {
            if let Some(rgb) = crate::state::parse_hex(&text) {
                set(app, rgb, false);
            }
        }
    }
    if linear {
        let spec = SliderSpec::new(
            lang.pick("明るさ", "Intensity"),
            0.0,
            8.0,
            NumberFormat {
                decimals: 2,
                trim: false,
                suffix: "",
            },
        )
        .tooltip(name)
        .enabled(enabled);
        let out = w::slider(
            ui,
            rows.slider_row(),
            ("look.intensity", name),
            intensity,
            &spec,
        );
        if out.changed {
            let k = out.value.max(0.0) / intensity;
            app.apply(look_action(LookOp::Value {
                name,
                value: LookValue::Color([raw[0] * k, raw[1] * k, raw[2] * k, raw[3]]),
                drag: true,
            }));
        }
    }
    if let Some((ja, en)) = alpha {
        let spec = SliderSpec::new(
            lang.pick(ja, en),
            0.0,
            1.0,
            NumberFormat {
                decimals: 2,
                trim: false,
                suffix: "",
            },
        )
        .tooltip(name)
        .enabled(enabled);
        let out = w::slider(ui, rows.slider_row(), ("look.alpha", name), raw[3], &spec);
        if out.changed {
            app.apply(look_action(LookOp::Value {
                name,
                value: LookValue::Color([raw[0], raw[1], raw[2], out.value]),
                drag: true,
            }));
        }
    }
}

/// lilToon の「不透明度」（`LocalizedPropertyAlpha`）の名前。
const ALPHA: Option<(&str, &str)> = Some(("不透明度", "Alpha"));

/// ベクトルの成分の行（色調補正の HSVG など）。
#[allow(clippy::too_many_arguments)]
fn vector_part(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    name: &'static str,
    index: usize,
    label: &str,
    range: (f32, f32),
    enabled: bool,
) {
    let v = liltoon::value(app.doc.drawn_look(), name);
    let (label, tip) = marked(app, name, label);
    let spec = SliderSpec::new(
        &label,
        range.0,
        range.1,
        NumberFormat {
            decimals: 2,
            trim: false,
            suffix: "",
        },
    )
    .tooltip(&tip)
    .enabled(enabled);
    let out = w::slider(
        ui,
        rows.slider_row(),
        ("look.v", name, index),
        v[index],
        &spec,
    );
    if out.changed {
        let mut next = v;
        next[index] = out.value;
        app.apply(look_action(LookOp::Value {
            name,
            value: LookValue::Vector(next),
            drag: true,
        }));
    }
}

/// ベクトルの成分の入切（`[lilFFFB]` の 4 つ目・`[lilVec3B]` の w）。
fn vector_toggle(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    name: &'static str,
    index: usize,
    label: &str,
    enabled: bool,
) {
    let v = liltoon::value(app.doc.drawn_look(), name);
    let (label, tip) = marked(app, name, label);
    if let Some(next) = toggle_row(
        ui,
        rows,
        &format!("look.vt.{name}.{index}"),
        &label,
        v[index] != 0.0,
        Some(&tip),
        enabled,
    ) {
        let mut out = v;
        out[index] = f32::from(next);
        app.apply(look_action(LookOp::Value {
            name,
            value: LookValue::Vector(out),
            drag: false,
        }));
    }
}

/// HSV / ガンマの 4 つの行（`[lilHSVG]`）。
fn hsvg(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, name: &'static str, enabled: bool) {
    let lang = app.lang;
    vector_part(
        ui,
        app,
        rows,
        name,
        0,
        lang.pick("色相", "Hue"),
        (-0.5, 0.5),
        enabled,
    );
    vector_part(
        ui,
        app,
        rows,
        name,
        1,
        lang.pick("彩度", "Saturation"),
        (0.0, 2.0),
        enabled,
    );
    vector_part(
        ui,
        app,
        rows,
        name,
        2,
        lang.pick("明度", "Value"),
        (0.0, 2.0),
        enabled,
    );
    vector_part(
        ui,
        app,
        rows,
        name,
        3,
        lang.pick("ガンマ", "Gamma"),
        (0.01, 2.0),
        enabled,
    );
}

/// タイリングとオフセット（Unity の `TextureScaleOffsetProperty`。`<名前>_ST`）。
fn tiling_offset(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    name: &'static str,
    enabled: bool,
) {
    vector_part(ui, app, rows, name, 0, "Tiling X", (-10.0, 10.0), enabled);
    vector_part(ui, app, rows, name, 1, "Tiling Y", (-10.0, 10.0), enabled);
    vector_part(ui, app, rows, name, 2, "Offset X", (-1.0, 1.0), enabled);
    vector_part(ui, app, rows, name, 3, "Offset Y", (-1.0, 1.0), enabled);
}

/// 角度（`_ScrollRotate` の z。ラジアンを度で見せる。スクロール・回転の速さは時間で動く値なので出さない）。
fn scroll_angle(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    name: &'static str,
    enabled: bool,
) {
    let v = liltoon::value(app.doc.drawn_look(), name);
    let (label, tip) = marked(app, name, app.lang.pick("角度", "Angle"));
    let spec = SliderSpec::new(
        &label,
        -180.0,
        180.0,
        NumberFormat {
            decimals: 1,
            trim: false,
            suffix: "°",
        },
    )
    .tooltip(&tip)
    .enabled(enabled);
    let out = w::slider(
        ui,
        rows.slider_row(),
        ("look.angle", name),
        v[2].to_degrees(),
        &spec,
    );
    if out.changed {
        let mut next = v;
        next[2] = out.value.to_radians();
        app.apply(look_action(LookOp::Value {
            name,
            value: LookValue::Vector(next),
            drag: true,
        }));
    }
}

/// スケールとオフセットを最小・最大で見せる 2 行（`fields::remap_shown`）。動かしたら新しい (最小, 最大)。
#[allow(clippy::too_many_arguments)]
fn remap_rows(
    ui: &mut Ui,
    rows: &mut Rows,
    id: (&'static str, usize),
    labels: (&str, &str),
    tip: &str,
    scale: f32,
    offset: f32,
    enabled: bool,
) -> Option<(f32, f32)> {
    let (min, max) = fields::remap_shown(scale, offset);
    let mut out = None;
    for (which, label, value) in [(0, labels.0, min), (1, labels.1, max)] {
        let spec = SliderSpec::new(
            label,
            -0.01,
            1.01,
            NumberFormat {
                decimals: 2,
                trim: false,
                suffix: "",
            },
        )
        .tooltip(tip)
        .enabled(enabled);
        let o = w::slider(
            ui,
            rows.slider_row(),
            ("look.remap", id.0, id.1, which),
            value,
            &spec,
        );
        if o.changed {
            let (mut lo, mut hi) = (min, max);
            if which == 0 {
                lo = o.value;
            } else {
                hi = o.value;
            }
            out = Some((lo, hi));
        }
    }
    out
}

/// AO の範囲（lilToon の 1st Min・Max のように、スケールとオフセットを最小・最大で見せる）。
fn ao_range(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, enabled: bool) {
    for (k, (min_label, max_label)) in [
        ("1st Min", "1st Max"),
        ("2nd Min", "2nd Max"),
        ("3rd Min", "3rd Max"),
    ]
    .iter()
    .enumerate()
    {
        let (name, i) = if k < 2 {
            ("_ShadowAOShift", k * 2)
        } else {
            ("_ShadowAOShift2", 0)
        };
        let v = liltoon::value(app.doc.drawn_look(), name);
        let (min_label, tip) = marked(app, name, min_label);
        let (max_label, _) = marked(app, name, max_label);
        if let Some((lo, hi)) = remap_rows(
            ui,
            rows,
            ("ao", k),
            (&min_label, &max_label),
            &tip,
            v[i],
            v[i + 1],
            enabled,
        ) {
            let (s, o) = fields::remap_values(lo, hi);
            let mut next = v;
            next[i] = s;
            next[i + 1] = o;
            app.apply(look_action(LookOp::Value {
                name,
                value: LookValue::Vector(next),
                drag: true,
            }));
        }
    }
}

// ───────── テクスチャのスロット ─────────

fn default_name(lang: Lang, d: SlotDefault) -> &'static str {
    match d {
        SlotDefault::White => lang.pick("白", "White"),
        SlotDefault::Black => lang.pick("黒", "Black"),
        SlotDefault::Bump => lang.pick("平ら", "Flat"),
    }
}

fn source_name(app: &AppState, slot: &liltoon::Slot, source: Option<&TextureSource>) -> String {
    let lang = app.lang;
    match source {
        None => default_name(lang, slot.default).into(),
        Some(TextureSource::Channel(c)) => {
            if app.doc.channel_info(*c).is_some() {
                channel_name(lang, &app.doc, *c)
            } else {
                lang.pick("（無いチャンネル）", "(missing channel)").into()
            }
        }
        Some(TextureSource::Packed(_)) => lang.pick("成分ごと", "Per Component").into(),
        Some(TextureSource::Image(id)) => {
            let rid = crate::fx::inputs::resource_id(*id);
            app.shelf.get(&rid).map_or_else(
                || lang.pick("（無い画像）", "(missing image)").to_owned(),
                |r| r.name.clone(),
            )
        }
    }
}

fn plane_name(app: &AppState, p: PlaneSource) -> String {
    match p {
        PlaneSource::Zero => "0".into(),
        PlaneSource::One => "1".into(),
        PlaneSource::Channel { channel, component } => {
            let name = channel_name(app.lang, &app.doc, channel);
            if app
                .doc
                .channel_info(channel)
                .is_some_and(|i| i.kind == ChannelKind::Scalar)
            {
                name
            } else {
                format!("{name} {}", ["R", "G", "B", "A"][component.min(3) as usize])
            }
        }
    }
}

/// スロットが読むユーザーチャンネルが、3D ビューの配列のレイヤーの上限（16）を超えて描かれないか（17 個目から）。
pub fn slot_over_layer_limit(doc: &yolu_core::Document, name: &str) -> bool {
    let look = doc.drawn_look();
    let Some(source) = look.textures.get(name) else {
        return false;
    };
    let dropped = crate::view3d::look_gpu::dropped_users(doc, look);
    !dropped.is_empty() && source.channels().iter().any(|c| dropped.contains(c))
}

/// スロットの Unity から受けた絵が、3D ビューの受けた絵の配列のレイヤーの上限（16）を超えて描かれないか（スロットの並びで 17 枚目から）。
pub fn slot_over_received_limit(doc: &yolu_core::Document, name: &str) -> bool {
    let look = doc.drawn_look();
    let source = look.textures.get(name);
    if source.is_some() && !crate::view3d::look_gpu::yields_to_received(doc, source) {
        return false;
    }
    crate::view3d::look_gpu::dropped_received(doc, look)
        .iter()
        .any(|s| s == name)
}

/// 今描いているチャンネルが、このスロットのチャンネルか（スロットの行の描く口の印）。
pub fn painting_slot(app: &AppState, name: &str) -> bool {
    super::slot_channel(&app.doc, name).is_some_and(|c| c == app.m2.paint_channel)
}

/// スロットの行: 名前（`label`。無ければスロットの名前）・割り当て（押すと選ぶ）・右に描く口のボタン（筆。マットキャップの絵の
/// スロットには無い）。成分ごとの詰め合わせは、その下に成分ごとの行。
fn slot_row(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    name: &'static str,
    label: Option<&str>,
    enabled: bool,
) {
    let Some(index) = liltoon::slot_index(name) else {
        return;
    };
    let slot = SLOTS[index];
    let lang = app.lang;
    let source = app.doc.drawn_look().textures.get(name).copied();
    let mut value = source_name(app, &slot, source.as_ref());
    let mut tip = lang.pick(
        format!(
            "{name}: 読むチャンネル（成分ごとに選ぶこともできる）。割り当てないと {} のテクスチャ",
            default_name(lang, slot.default)
        ),
        format!(
            "{name}: the channel it reads (or one per component). Unassigned reads a {} texture",
            default_name(lang, slot.default)
        ),
    );
    let yields = crate::view3d::look_gpu::yields_to_unity_texture(&app.doc, name, source.as_ref());
    if source.is_none() || yields {
        // 割り当てていないスロット（と、流し込み先でないスロットの、使っていない標準のチャンネルの割り当て）は、Unity から受けた絵が
        // あればそれで、無ければスロットの既定のテクスチャで描く（Unity も空のテクスチャを既定で読む）
        if let Some(received) = app.doc.received_look() {
            if let Some(image) = received.images.get(name) {
                value = lang.pick("Unity のテクスチャ", "Unity texture").into();
                tip += &format!("\n{}×{}", image.width, image.height);
            } else if let Some(why) = received.missing.get(name) {
                value = missing_short(lang, *why).into();
                tip += &format!("\n{}", missing_text(lang, *why));
            } else if yields {
                value = default_name(lang, slot.default).into();
                tip += &lang.pick(
                    format!(
                        "\nUnity ではテクスチャが空（{}で描く）",
                        default_name(lang, slot.default)
                    ),
                    format!(
                        "\nThe texture is empty in Unity (drawn {})",
                        default_name(lang, slot.default).to_lowercase()
                    ),
                );
            }
        }
    }
    if slot_over_layer_limit(&app.doc, name) {
        // 3D ビューが持つユーザーチャンネルは 16 個まで（スロットの並びで先のものから）。超えた分は割り当てのない既定で描く
        value = format!("{value}{}", lang.pick("（描かない）", " (not drawn)"));
        tip += lang.pick(
            "\n3D ビューで描かない: lilToon のスロットが読むユーザーチャンネルは 16 個まで",
            "\nNot drawn in the 3D View: lilToon slots read up to 16 user channels",
        );
    }
    if slot_over_received_limit(&app.doc, name) {
        // 3D ビューが持つ Unity のテクスチャも 16 枚まで（スロットの並びで先のものから）。超えた分は割り当てのない既定で描く
        value = format!("{value}{}", lang.pick("（描かない）", " (not drawn)"));
        tip += &lang.pick(
            format!(
                "\n3D ビューで描かない: Unity のテクスチャは 16 枚まで（{}で描く）",
                default_name(lang, slot.default)
            ),
            format!(
                "\nNot drawn in the 3D View: up to 16 Unity textures (drawn {})",
                default_name(lang, slot.default).to_lowercase()
            ),
        );
    }
    let row = rows.row(t::ROW_HEIGHT, 4.0);
    let paintable = slot.usage != SlotUse::Image;
    let field = if paintable {
        Rect::from_min_max(row.min, pos2(row.right() - PAINT_W - 4.0, row.bottom()))
    } else {
        row
    };
    let (response, b) = w::dropdown(
        ui,
        field,
        format!("look.slot.{name}"),
        Some(label.unwrap_or(slot.label(lang))),
        &value,
        Some(&tip),
        enabled,
        LABEL_W,
    );
    if response.clicked() {
        open_popup(
            app,
            &ui.ctx().clone(),
            Popup::Look(LookChoice::Slot(index)),
            b,
            b.width().max(180.0),
        );
    }
    if paintable {
        let painting = painting_slot(app, name);
        let button = Rect::from_min_size(
            pos2(row.right() - PAINT_W, row.top()),
            vec2(PAINT_W, row.height()),
        );
        let what = slot.label(lang);
        let tip = if super::slot_channel(&app.doc, name).is_some() {
            lang.pick(format!("{what}を描く"), format!("Paint {what}"))
        } else {
            lang.pick(
                format!("{what}を描く（描くチャンネルを作って割り当てる）"),
                format!("Paint {what} (creates and assigns a channel to paint)"),
            )
        };
        if w::icon_button(
            ui,
            button,
            ("look.paint", name),
            "paint_brush",
            &tip,
            painting,
            enabled,
            14.0,
        )
        .clicked()
        {
            app.apply(look_action(LookOp::PaintSlot(name)));
        }
    }
    if let Some(TextureSource::Packed(planes)) = source {
        for (k, plane) in planes.iter().enumerate() {
            let label = ["R", "G", "B", "A"][k];
            let value = plane_name(app, *plane);
            let r = rows.row(t::ROW_HEIGHT, 2.0);
            let r = Rect::from_min_max(pos2(r.left() + 14.0, r.top()), r.max);
            let (response, b) = w::dropdown(
                ui,
                r,
                ("look.plane", name, k),
                Some(label),
                &value,
                Some(name),
                enabled,
                LABEL_W - 14.0,
            );
            if response.clicked() {
                open_popup(
                    app,
                    &ui.ctx().clone(),
                    Popup::Look(LookChoice::Plane(index, k as u8)),
                    b,
                    b.width().max(180.0),
                );
            }
        }
    }
}

/// lilToon の「テクスチャと色」の 1 行（スロットの行と、その下の字下げした色の行）。
#[allow(clippy::too_many_arguments)]
fn texture_color(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    slot: &'static str,
    label: &str,
    color: &'static str,
    alpha: Option<(&str, &str)>,
    enabled: bool,
) {
    slot_row(ui, app, rows, slot, Some(label), enabled);
    inner(rows, |rows| {
        color_row(ui, app, rows, color, None, alpha, enabled)
    });
}

// ───────── 節 ─────────

/// 描画モード（lilToon は基本設定の見出しの上。Unity ではシェーダーの名前が変わる）。
fn rendering_mode(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    let lang = app.lang;
    let look = app.doc.drawn_look().clone();
    let info = liltoon::shader_info(&look);
    let mut mode = info.mode.label(lang).to_owned();
    if !info.exact {
        mode = format!("{mode}（{}）", look.shader_name());
    }
    let mut tip = if info.exact {
        lang.pick(
            "_TransparentMode（Unity ではシェーダーの名前）",
            "_TransparentMode (the shader name in Unity)",
        )
        .to_owned()
    } else {
        lang.pick(
            format!(
                "{}: この版の機能は描かない（近い描き方で描く）",
                look.shader_name()
            ),
            format!(
                "{}: this variant's own features are not drawn (closest look)",
                look.shader_name()
            ),
        )
    };
    let mut label = lang.pick("描画モード", "Rendering Mode").to_owned();
    // 欄で描画モードを変えている（シェーダーの名前が利用者の設定にある）なら、ほかの項目と同じく印と Unity の値
    if let Some(received) = app
        .doc
        .received_look()
        .filter(|_| !app.doc.look().shader.is_empty())
    {
        let unity = liltoon::shader_info(&received.look);
        if unity.mode != info.mode {
            label += " •";
            tip += &format!("\nUnity: {}", unity.mode.label(lang));
        }
    }
    rows.indent = t::SECTION_INDENT;
    if let Some(r) = choice(ui, rows, "look.mode", &label, &mode, &tip, free) {
        open_popup(
            app,
            &ui.ctx().clone(),
            Popup::Look(LookChoice::Mode),
            r,
            r.width(),
        );
    }
}

fn base(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.base",
        "基本設定",
        "Base Setting",
        Section::Base,
    ) {
        return;
    }
    let look = app.doc.drawn_look().clone();
    if liltoon::shader_info(&look).mode != RenderMode::Opaque {
        float_row(ui, app, rows, "_Cutoff", free);
    }
    choice_prop(ui, app, rows, "_Cull", free);
    if liltoon::number(&look, "_Cull").round() <= 1.0 {
        inner(rows, |rows| {
            toggle_prop(ui, app, rows, "_FlipNormal", free);
            float_row(ui, app, rows, "_BackfaceForceShadow", free);
            color_row(ui, app, rows, "_BackfaceColor", None, ALPHA, free);
        });
    }
    toggle_prop(ui, app, rows, "_Invisible", free);
    float_row(ui, app, rows, "_AAStrength", free);
}

fn lighting(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.lighting",
        "ライティング・明るさ設定",
        "Lighting",
        Section::Lighting,
    ) {
        return;
    }
    let lang = app.lang;
    group_label(ui, rows, lang.pick("基本設定", "Base Setting"));
    for name in [
        "_LightMinLimit",
        "_LightMaxLimit",
        "_MonochromeLighting",
        "_ShadowEnvStrength",
    ] {
        float_row_in(ui, app, rows, "lighting", name, None, free);
    }
    // プリセットを適用（lilToon の `ApplyLightingPreset`）
    let row = rows.row(24.0, 4.0);
    w::text(
        ui.painter(),
        Rect::from_min_size(row.min, vec2(LABEL_W, row.height())),
        lang.pick("プリセットを適用", "Apply Preset"),
        t::LABEL.with_color(w::label_color(ui, "look.preset.label", free)),
        w::Align::Left,
    );
    let buttons = Rows::split(
        Rect::from_min_max(pos2(row.left() + LABEL_W, row.top()), row.max),
        2,
        4.0,
    );
    for (b, preset, ja, en) in [
        (buttons[0], LightingPreset::Default, "通常", "Default"),
        (
            buttons[1],
            LightingPreset::SemiMonochrome,
            "半モノクロ",
            "Semi-monochrome",
        ),
    ] {
        if w::button(
            ui,
            b,
            ("look.preset", ja),
            lang.pick(ja, en),
            false,
            free,
            None,
            None,
        )
        .clicked()
        {
            app.apply(look_action(LookOp::LightingPreset(preset)));
        }
    }
    group_label(ui, rows, lang.pick("拡張設定", "Advanced"));
    float_row(ui, app, rows, "_AsUnlit", free);
    let label = lang.pick("ライト方向のオーバーライド", "Light Direction Override");
    for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
        vector_part(
            ui,
            app,
            rows,
            "_LightDirectionOverride",
            k,
            &format!("{label} {axis}"),
            (-1.0, 1.0),
            free,
        );
    }
    inner(rows, |rows| {
        vector_toggle(
            ui,
            app,
            rows,
            "_LightDirectionOverride",
            3,
            lang.pick("オブジェクトの向きに追従", "Object direction following"),
            free,
        );
    });
}

fn uv(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.uv",
        "UV設定",
        "UV Setting",
        Section::Uv,
    ) {
        return;
    }
    tiling_offset(ui, app, rows, "_MainTex_ST", free);
    scroll_angle(ui, app, rows, "_MainTex_ScrollRotate", free);
    toggle_prop(ui, app, rows, "_ShiftBackfaceUV", free);
}

fn main_color(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.main",
        "メインカラー / 透過設定",
        "Main Color / Alpha",
        Section::Main,
    ) {
        return;
    }
    let lang = app.lang;
    let mode = liltoon::shader_info(app.doc.drawn_look()).mode;
    group_label(
        ui,
        rows,
        if mode == RenderMode::Opaque {
            lang.pick("メインカラー", "Main Color")
        } else {
            lang.pick("メインカラー / 透過", "Main Color / Alpha")
        },
    );
    let main_label = if mode == RenderMode::Opaque {
        lang.pick("色", "Color")
    } else {
        lang.pick("色 / 透明度", "Color / Alpha")
    };
    texture_color(ui, app, rows, "_MainTex", main_label, "_Color", ALPHA, free);
    group_label(ui, rows, lang.pick("色調補正", "Color Adjust"));
    slot_row(
        ui,
        app,
        rows,
        "_MainColorAdjustMask",
        Some(lang.pick("マスク", "Mask")),
        free,
    );
    hsvg(ui, app, rows, "_MainTexHSVG", free);
    for (prefix, toggle) in [
        ("_Main2nd", "_UseMain2ndTex"),
        ("_Main3rd", "_UseMain3rdTex"),
    ] {
        rows.space(2.0);
        if feature(ui, app, rows, toggle, free) {
            inner(rows, |rows| layer(ui, app, rows, prefix, free));
        }
    }
    if mode != RenderMode::Opaque {
        rows.space(2.0);
        choice_prop(ui, app, rows, "_AlphaMaskMode", free);
        if liltoon::number(app.doc.drawn_look(), "_AlphaMaskMode").round() >= 1.0 {
            inner(rows, |rows| alpha_mask(ui, app, rows, free));
        }
    }
}

/// メインカラー 2nd・3rd の中身（`prefix` は `_Main2nd`・`_Main3rd`）。
fn layer(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, prefix: &'static str, free: bool) {
    let lang = app.lang;
    let second = prefix == "_Main2nd";
    let pick = |a: &'static str, b: &'static str| if second { a } else { b };
    texture_color(
        ui,
        app,
        rows,
        pick("_Main2ndTex", "_Main3rdTex"),
        lang.pick("色", "Color"),
        pick("_Color2nd", "_Color3rd"),
        ALPHA,
        free,
    );
    inner(rows, |rows| {
        toggle_prop(
            ui,
            app,
            rows,
            pick("_Main2ndTexIsMSDF", "_Main3rdTexIsMSDF"),
            free,
        );
        choice_prop(
            ui,
            app,
            rows,
            pick("_Main2ndTex_Cull", "_Main3rdTex_Cull"),
            free,
        );
    });
    float_row(
        ui,
        app,
        rows,
        pick("_Main2ndEnableLighting", "_Main3rdEnableLighting"),
        free,
    );
    choice_prop(
        ui,
        app,
        rows,
        pick("_Main2ndTexBlendMode", "_Main3rdTexBlendMode"),
        free,
    );
    choice_prop(
        ui,
        app,
        rows,
        pick("_Main2ndTexAlphaMode", "_Main3rdTexAlphaMode"),
        free,
    );
    rows.space(2.0);
    choice_prop(
        ui,
        app,
        rows,
        pick("_Main2ndTex_UVMode", "_Main3rdTex_UVMode"),
        free,
    );
    let decal = pick("_Main2ndTexIsDecal", "_Main3rdTexIsDecal");
    // 切にするとミラー・複製のフラグも 0 に（lilToon の `UV4Decal` と同じ。`fields::decal_op`）
    if let Some(prop) = liltoon::prop(decal) {
        let on = liltoon::on(app.doc.drawn_look(), decal);
        let (label, tip) = marked(app, decal, prop.label(app.lang));
        if let Some(next) = toggle_row(
            ui,
            rows,
            &format!("look.t.{decal}"),
            &label,
            on,
            Some(&tip),
            free,
        ) {
            app.apply(look_action(fields::decal_op(prefix, next)));
        }
    }
    let st = pick("_Main2ndTex_ST", "_Main3rdTex_ST");
    if liltoon::on(app.doc.drawn_look(), decal) {
        inner(rows, |rows| decal_rows(ui, app, rows, prefix, st, free));
    } else {
        tiling_offset(ui, app, rows, st, free);
    }
    float_row(
        ui,
        app,
        rows,
        pick("_Main2ndTexAngle", "_Main3rdTexAngle"),
        free,
    );
    rows.space(2.0);
    slot_row(
        ui,
        app,
        rows,
        pick("_Main2ndBlendMask", "_Main3rdBlendMask"),
        Some(lang.pick("マスク", "Mask")),
        free,
    );
    group_label(ui, rows, lang.pick("距離フェード", "Distance Fade"));
    let fade = pick("_Main2ndDistanceFade", "_Main3rdDistanceFade");
    inner(rows, |rows| {
        vector_part(
            ui,
            app,
            rows,
            fade,
            0,
            lang.pick("開始距離", "Start Distance"),
            (0.0, 10.0),
            free,
        );
        vector_part(
            ui,
            app,
            rows,
            fade,
            1,
            lang.pick("終了距離", "End Distance"),
            (0.0, 10.0),
            free,
        );
        vector_part(
            ui,
            app,
            rows,
            fade,
            2,
            lang.pick("強度", "Strength"),
            (0.0, 1.0),
            free,
        );
    });
}

/// デカールの置き方（lilToon の `UV4Decal`: ミラーモード・複製モードと、タイリング・オフセットを位置と大きさで見せる行）。
fn decal_rows(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    prefix: &'static str,
    st: &'static str,
    free: bool,
) {
    let lang = app.lang;
    let look = app.doc.drawn_look().clone();
    let p = |s: &str| format!("{prefix}Tex{s}");
    let on = |s: &str| liltoon::on(&look, &p(s));
    let mirror = mirror_mode(on("IsLeftOnly"), on("IsRightOnly"), on("ShouldFlipMirror"));
    let copy = copy_mode(on("ShouldCopy"), on("ShouldFlipCopy"));
    if let Some(r) = choice(
        ui,
        rows,
        &format!("look.mirror.{prefix}"),
        lang.pick("ミラーモード", "Mirror Mode"),
        MIRROR[mirror].pick(lang),
        &format!(
            "{}, {}, {}",
            p("IsLeftOnly"),
            p("IsRightOnly"),
            p("ShouldFlipMirror")
        ),
        free,
    ) {
        open_popup(
            app,
            &ui.ctx().clone(),
            Popup::Look(LookChoice::Mirror(prefix)),
            r,
            r.width(),
        );
    }
    if let Some(r) = choice(
        ui,
        rows,
        &format!("look.copy.{prefix}"),
        lang.pick("複製モード", "Copy Mode"),
        COPY[copy].pick(lang),
        &format!("{}, {}", p("ShouldCopy"), p("ShouldFlipCopy")),
        free,
    ) {
        open_popup(
            app,
            &ui.ctx().clone(),
            Popup::Look(LookChoice::Copy(prefix)),
            r,
            r.width(),
        );
    }
    // scale & offset → 大きさと位置（lilToon の欄と同じ換算）
    let mut next = fields::decal_shown(liltoon::value(&look, st), copy > 0);
    let mut changed = false;
    for (k, (ja, en), lo) in [
        (0, ("X座標", "Position X"), if copy > 0 { 0.5 } else { 0.0 }),
        (1, ("Y座標", "Position Y"), 0.0),
        (2, ("X軸サイズ", "Scale X"), -1.0),
        (3, ("Y軸サイズ", "Scale Y"), -1.0),
    ] {
        let (label, tip) = marked(app, st, lang.pick(ja, en));
        let spec = SliderSpec::new(
            &label,
            lo,
            1.0,
            NumberFormat {
                decimals: 3,
                trim: false,
                suffix: "",
            },
        )
        .tooltip(&tip)
        .enabled(free);
        let out = w::slider(ui, rows.slider_row(), ("look.decal", st, k), next[k], &spec);
        if out.changed {
            changed = true;
            next[k] = out.value;
        }
    }
    if changed {
        app.apply(look_action(fields::decal_st_op(st, next, true)));
    }
}

/// 日英の名前 1 つ。
#[derive(Clone, Copy)]
struct Name(&'static str, &'static str);

impl Name {
    fn pick(self, lang: Lang) -> &'static str {
        lang.pick(self.0, self.1)
    }
}

const MIRROR: [Name; 5] = [
    Name("通常", "Normal"),
    Name("反転", "Flip"),
    Name("左のみ", "Left Only"),
    Name("右のみ", "Right Only"),
    Name("右のみ・反転", "Flip Right Only"),
];
const COPY: [Name; 3] = [
    Name("通常", "Normal"),
    Name("左右対称", "Symmetry"),
    Name("反転", "Flip"),
];
const SPECULAR: [Name; 3] = [
    Name("無効", "None"),
    Name("リアル", "Realistic"),
    Name("トゥーン", "Toon"),
];

/// アルファマスクの中身（lilToon の欄: マスク・反転・透明度・Cutoff。スケールとオフセットはこの 2 つから決まる）。
fn alpha_mask(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    let lang = app.lang;
    slot_row(
        ui,
        app,
        rows,
        "_AlphaMask",
        Some(lang.pick("アルファマスク", "Alpha Mask")),
        free,
    );
    let look = app.doc.drawn_look().clone();
    let (invert, transparency) = fields::alpha_mask_shown(
        liltoon::number(&look, "_AlphaMaskScale"),
        liltoon::number(&look, "_AlphaMaskValue"),
    );
    let set = |app: &mut AppState, invert: bool, transparency: f32, drag: bool| {
        app.apply(look_action(fields::alpha_mask_op(
            invert,
            transparency,
            drag,
        )));
    };
    let (label, tip) = marked(app, "_AlphaMaskScale", "Invert");
    if let Some(next) = toggle_row(
        ui,
        rows,
        "look.alphamask.invert",
        &label,
        invert,
        Some(&tip),
        free,
    ) {
        set(app, next, transparency, false);
    }
    let (label, tip) = marked(app, "_AlphaMaskValue", "Transparency");
    let spec = SliderSpec::new(
        &label,
        -1.0,
        1.0,
        NumberFormat {
            decimals: 2,
            trim: false,
            suffix: "",
        },
    )
    .tooltip(&tip)
    .enabled(free);
    let out = w::slider(
        ui,
        rows.slider_row(),
        "look.alphamask.transparency",
        transparency,
        &spec,
    );
    if out.changed {
        set(app, invert, out.value, true);
    }
    float_row_in(ui, app, rows, "alphamask", "_Cutoff", None, free);
}

fn shadow(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.shadow",
        "影設定",
        "Shadow",
        Section::Shadow,
    ) {
        return;
    }
    let lang = app.lang;
    if !feature(ui, app, rows, "_UseShadow", free) {
        return;
    }
    inner(rows, |rows| {
        choice_prop(ui, app, rows, "_ShadowMaskType", free);
        let mask_type = liltoon::number(app.doc.drawn_look(), "_ShadowMaskType").round();
        if mask_type == 1.0 {
            slot_row(
                ui,
                app,
                rows,
                "_ShadowStrengthMask",
                Some(lang.pick("マスク", "Mask")),
                free,
            );
            inner(rows, |rows| {
                float_row(ui, app, rows, "_ShadowStrengthMaskLOD", free);
                float_row(ui, app, rows, "_ShadowFlatBorder", free);
                float_row(ui, app, rows, "_ShadowFlatBlur", free);
            });
            float_row(ui, app, rows, "_ShadowStrength", free);
        } else if mask_type == 2.0 {
            slot_row(ui, app, rows, "_ShadowStrengthMask", Some("SDF"), free);
            inner(rows, |rows| {
                float_row(ui, app, rows, "_ShadowStrengthMaskLOD", free);
                float_row_in(
                    ui,
                    app,
                    rows,
                    "",
                    "_ShadowFlatBlur",
                    Some("Blend Y Direction"),
                    free,
                );
            });
            float_row(ui, app, rows, "_ShadowStrength", free);
        } else {
            slot_row(
                ui,
                app,
                rows,
                "_ShadowStrengthMask",
                Some(lang.pick("マスクと強度", "Mask & Strength")),
                free,
            );
            inner(rows, |rows| {
                float_row(ui, app, rows, "_ShadowStrength", free);
                float_row(ui, app, rows, "_ShadowStrengthMaskLOD", free);
            });
        }
        rows.space(2.0);
        choice_prop(ui, app, rows, "_ShadowColorType", free);
        for (k, (tex, color, names)) in [
            (
                "_ShadowColorTex",
                "_ShadowColor",
                [
                    "_ShadowBorder",
                    "_ShadowBlur",
                    "_ShadowNormalStrength",
                    "_ShadowReceive",
                ],
            ),
            (
                "_Shadow2ndColorTex",
                "_Shadow2ndColor",
                [
                    "_Shadow2ndBorder",
                    "_Shadow2ndBlur",
                    "_Shadow2ndNormalStrength",
                    "_Shadow2ndReceive",
                ],
            ),
            (
                "_Shadow3rdColorTex",
                "_Shadow3rdColor",
                [
                    "_Shadow3rdBorder",
                    "_Shadow3rdBlur",
                    "_Shadow3rdNormalStrength",
                    "_Shadow3rdReceive",
                ],
            ),
        ]
        .into_iter()
        .enumerate()
        {
            rows.space(2.0);
            let label = liltoon::prop(color).map_or(color, |p| p.label(lang));
            slot_row(ui, app, rows, tex, Some(label), free);
            inner(rows, |rows| {
                // 1 影の色の A は使わない（lilToon の欄も出さない）。2 影・3 影は A が強さ
                let alpha = if k > 0 { ALPHA } else { None };
                color_row(
                    ui,
                    app,
                    rows,
                    color,
                    Some(lang.pick("色", "Color")),
                    alpha,
                    free,
                );
                if k == 0 || liltoon::value(app.doc.drawn_look(), color)[3] > 0.0 {
                    for name in names {
                        float_row(ui, app, rows, name, free);
                    }
                }
            });
        }
        rows.space(2.0);
        color_row(ui, app, rows, "_ShadowBorderColor", None, None, free);
        float_row(ui, app, rows, "_ShadowBorderRange", free);
        rows.space(2.0);
        float_row(ui, app, rows, "_ShadowMainStrength", free);
        float_row_in(ui, app, rows, "shadow", "_ShadowEnvStrength", None, free);
        rows.space(2.0);
        slot_row(ui, app, rows, "_ShadowBlurMask", None, free);
        inner(rows, |rows| {
            float_row(ui, app, rows, "_ShadowBlurMaskLOD", free)
        });
        rows.space(2.0);
        slot_row(ui, app, rows, "_ShadowBorderMask", None, free);
        inner(rows, |rows| {
            float_row(ui, app, rows, "_ShadowBorderMaskLOD", free);
            toggle_prop(ui, app, rows, "_ShadowPostAO", free);
            ao_range(ui, app, rows, free);
        });
    });
}

fn rim_shade(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.rimshade",
        "リムシェード",
        "RimShade",
        Section::RimShade,
    ) {
        return;
    }
    let lang = app.lang;
    if !feature(ui, app, rows, "_UseRimShade", free) {
        return;
    }
    inner(rows, |rows| {
        texture_color(
            ui,
            app,
            rows,
            "_RimShadeMask",
            lang.pick("色 / マスク", "Color / Mask"),
            "_RimShadeColor",
            ALPHA,
            free,
        );
        for name in [
            "_RimShadeNormalStrength",
            "_RimShadeBorder",
            "_RimShadeBlur",
            "_RimShadeFresnelPower",
        ] {
            float_row(ui, app, rows, name, free);
        }
    });
}

fn emission(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.emission",
        "発光設定",
        "Emission",
        Section::Emission,
    ) {
        return;
    }
    let lang = app.lang;
    let sets: [[&'static str; 9]; 2] = [
        [
            "_UseEmission",
            "_EmissionMap",
            "_EmissionColor",
            "_EmissionMap_UVMode",
            "_EmissionMainStrength",
            "_EmissionBlendMask",
            "_EmissionBlend",
            "_EmissionBlendMode",
            "_EmissionFluorescence",
        ],
        [
            "_UseEmission2nd",
            "_Emission2ndMap",
            "_Emission2ndColor",
            "_Emission2ndMap_UVMode",
            "_Emission2ndMainStrength",
            "_Emission2ndBlendMask",
            "_Emission2ndBlend",
            "_Emission2ndBlendMode",
            "_Emission2ndFluorescence",
        ],
    ];
    for (k, names) in sets.into_iter().enumerate() {
        if k > 0 {
            rows.space(2.0);
        }
        if !feature(ui, app, rows, names[0], free) {
            continue;
        }
        inner(rows, |rows| {
            texture_color(
                ui,
                app,
                rows,
                names[1],
                lang.pick("色 / マスク", "Color / Mask"),
                names[2],
                ALPHA,
                free,
            );
            inner(rows, |rows| choice_prop(ui, app, rows, names[3], free));
            float_row(ui, app, rows, names[4], free);
            rows.space(2.0);
            slot_row(
                ui,
                app,
                rows,
                names[5],
                Some(lang.pick("マスク", "Mask")),
                free,
            );
            inner(rows, |rows| float_row(ui, app, rows, names[6], free));
            choice_prop(ui, app, rows, names[7], free);
            float_row(ui, app, rows, names[8], free);
        });
    }
}

fn normal_map(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.normal",
        "ノーマルマップ設定",
        "Normal Map",
        Section::Normal,
    ) {
        return;
    }
    let lang = app.lang;
    let normal = lang.pick("ノーマルマップ", "Normal Map");
    if feature(ui, app, rows, "_UseBumpMap", free) {
        inner(rows, |rows| {
            slot_row(ui, app, rows, "_BumpMap", Some(normal), free);
            float_row(ui, app, rows, "_BumpScale", free);
        });
    }
    rows.space(2.0);
    if feature(ui, app, rows, "_UseBump2ndMap", free) {
        inner(rows, |rows| {
            slot_row(ui, app, rows, "_Bump2ndMap", Some(normal), free);
            float_row(ui, app, rows, "_Bump2ndScale", free);
            choice_prop(ui, app, rows, "_Bump2ndMap_UVMode", free);
            slot_row(
                ui,
                app,
                rows,
                "_Bump2ndScaleMask",
                Some(lang.pick("マスクと強度", "Mask & Strength")),
                free,
            );
        });
    }
    rows.space(2.0);
    if feature(ui, app, rows, "_UseAnisotropy", free) {
        inner(rows, |rows| {
            slot_row(ui, app, rows, "_AnisotropyTangentMap", Some(normal), free);
            slot_row(
                ui,
                app,
                rows,
                "_AnisotropyScaleMask",
                Some(lang.pick("マスクと強度", "Mask & Strength")),
                free,
            );
            float_row(ui, app, rows, "_AnisotropyScale", free);
            group_label(ui, rows, lang.pick("適用先", "Apply to"));
            toggle_prop(ui, app, rows, "_Anisotropy2Reflection", free);
            if liltoon::on(app.doc.drawn_look(), "_Anisotropy2Reflection") {
                inner(rows, |rows| {
                    group_label(ui, rows, "1st Specular");
                    for name in [
                        "_AnisotropyTangentWidth",
                        "_AnisotropyBitangentWidth",
                        "_AnisotropyShift",
                        "_AnisotropyShiftNoiseScale",
                        "_AnisotropySpecularStrength",
                    ] {
                        float_row(ui, app, rows, name, free);
                    }
                    group_label(ui, rows, "2nd Specular");
                    for name in [
                        "_Anisotropy2ndTangentWidth",
                        "_Anisotropy2ndBitangentWidth",
                        "_Anisotropy2ndShift",
                        "_Anisotropy2ndShiftNoiseScale",
                        "_Anisotropy2ndSpecularStrength",
                    ] {
                        float_row(ui, app, rows, name, free);
                    }
                    slot_row(
                        ui,
                        app,
                        rows,
                        "_AnisotropyShiftNoiseMask",
                        Some(lang.pick("ノイズ", "Noise")),
                        free,
                    );
                });
            }
            toggle_prop(ui, app, rows, "_Anisotropy2MatCap", free);
            toggle_prop(ui, app, rows, "_Anisotropy2MatCap2nd", free);
        });
    }
}

fn backlight(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.backlight",
        "逆光ライト",
        "Backlight",
        Section::Backlight,
    ) {
        return;
    }
    let lang = app.lang;
    if !feature(ui, app, rows, "_UseBacklight", free) {
        return;
    }
    inner(rows, |rows| {
        texture_color(
            ui,
            app,
            rows,
            "_BacklightColorTex",
            lang.pick("色 / マスク", "Color / Mask"),
            "_BacklightColor",
            ALPHA,
            free,
        );
        inner(rows, |rows| {
            float_row(ui, app, rows, "_BacklightMainStrength", free);
            toggle_prop(ui, app, rows, "_BacklightReceiveShadow", free);
            toggle_prop(ui, app, rows, "_BacklightBackfaceMask", free);
        });
        rows.space(2.0);
        float_row(ui, app, rows, "_BacklightNormalStrength", free);
        inv_border_row(ui, app, rows, "_BacklightBorder", free);
        for name in [
            "_BacklightBlur",
            "_BacklightDirectivity",
            "_BacklightViewStrength",
        ] {
            float_row(ui, app, rows, name, free);
        }
    });
}

fn reflection(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.reflection",
        "光沢設定",
        "Reflections",
        Section::Reflection,
    ) {
        return;
    }
    let lang = app.lang;
    if !feature(ui, app, rows, "_UseReflection", free) {
        return;
    }
    inner(rows, |rows| {
        slot_row(
            ui,
            app,
            rows,
            "_SmoothnessTex",
            Some(lang.pick("滑らかさ", "Smoothness")),
            free,
        );
        inner(rows, |rows| {
            float_row(ui, app, rows, "_Smoothness", free);
            float_row(ui, app, rows, "_GSAAStrength", free);
        });
        rows.space(2.0);
        slot_row(
            ui,
            app,
            rows,
            "_MetallicGlossMap",
            Some(lang.pick("金属度", "Metallic")),
            free,
        );
        inner(rows, |rows| float_row(ui, app, rows, "_Metallic", free));
        rows.space(2.0);
        texture_color(
            ui,
            app,
            rows,
            "_ReflectionColorTex",
            lang.pick("色 / マスク", "Color / Mask"),
            "_ReflectionColor",
            ALPHA,
            free,
        );
        inner(rows, |rows| float_row(ui, app, rows, "_Reflectance", free));
        rows.space(2.0);
        let look = app.doc.drawn_look().clone();
        let mode = specular_mode(
            liltoon::on(&look, "_ApplySpecular"),
            liltoon::on(&look, "_SpecularToon"),
        );
        let (label, _) = marked(
            app,
            "_ApplySpecular",
            lang.pick("光沢のタイプ", "Specular Mode"),
        );
        if let Some(r) = choice(
            ui,
            rows,
            "look.specular",
            &label,
            SPECULAR[mode].pick(lang),
            "_ApplySpecular, _SpecularToon",
            free,
        ) {
            open_popup(
                app,
                &ui.ctx().clone(),
                Popup::Look(LookChoice::SpecularMode),
                r,
                r.width(),
            );
        }
        if mode > 0 {
            inner(rows, |rows| {
                float_row(ui, app, rows, "_SpecularNormalStrength", free);
                if mode == 2 {
                    float_row(ui, app, rows, "_SpecularBorder", free);
                    float_row(ui, app, rows, "_SpecularBlur", free);
                }
            });
        }
        toggle_prop(ui, app, rows, "_ApplyReflection", free);
        if liltoon::on(app.doc.drawn_look(), "_ApplyReflection") {
            inner(rows, |rows| {
                float_row(ui, app, rows, "_ReflectionNormalStrength", free)
            });
        }
        if liltoon::shader_info(app.doc.drawn_look()).mode == RenderMode::Transparent {
            toggle_prop(ui, app, rows, "_ReflectionApplyTransparency", free);
        }
        choice_prop(ui, app, rows, "_ReflectionBlendMode", free);
    });
}

/// マットキャップ 1 つの名前（1st・2nd）。
struct MatCapNames {
    toggle: &'static str,
    tex: &'static str,
    color: &'static str,
    zrot: &'static str,
    perspective: &'static str,
    main: &'static str,
    normal: &'static str,
    mask: &'static str,
    blend: &'static str,
    lighting: &'static str,
    shadow: &'static str,
    backface: &'static str,
    lod: &'static str,
    mode: &'static str,
    transparency: &'static str,
    custom: &'static str,
    bump: &'static str,
    bump_scale: &'static str,
}

const MATCAPS: [MatCapNames; 2] = [
    MatCapNames {
        toggle: "_UseMatCap",
        tex: "_MatCapTex",
        color: "_MatCapColor",
        zrot: "_MatCapZRotCancel",
        perspective: "_MatCapPerspective",
        main: "_MatCapMainStrength",
        normal: "_MatCapNormalStrength",
        mask: "_MatCapBlendMask",
        blend: "_MatCapBlend",
        lighting: "_MatCapEnableLighting",
        shadow: "_MatCapShadowMask",
        backface: "_MatCapBackfaceMask",
        lod: "_MatCapLod",
        mode: "_MatCapBlendMode",
        transparency: "_MatCapApplyTransparency",
        custom: "_MatCapCustomNormal",
        bump: "_MatCapBumpMap",
        bump_scale: "_MatCapBumpScale",
    },
    MatCapNames {
        toggle: "_UseMatCap2nd",
        tex: "_MatCap2ndTex",
        color: "_MatCap2ndColor",
        zrot: "_MatCap2ndZRotCancel",
        perspective: "_MatCap2ndPerspective",
        main: "_MatCap2ndMainStrength",
        normal: "_MatCap2ndNormalStrength",
        mask: "_MatCap2ndBlendMask",
        blend: "_MatCap2ndBlend",
        lighting: "_MatCap2ndEnableLighting",
        shadow: "_MatCap2ndShadowMask",
        backface: "_MatCap2ndBackfaceMask",
        lod: "_MatCap2ndLod",
        mode: "_MatCap2ndBlendMode",
        transparency: "_MatCap2ndApplyTransparency",
        custom: "_MatCap2ndCustomNormal",
        bump: "_MatCap2ndBumpMap",
        bump_scale: "_MatCap2ndBumpScale",
    },
];

fn matcap(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.matcap",
        "マットキャップ設定",
        "MatCap",
        Section::MatCap,
    ) {
        return;
    }
    let lang = app.lang;
    let transparent = liltoon::shader_info(app.doc.drawn_look()).mode == RenderMode::Transparent;
    for (k, m) in MATCAPS.iter().enumerate() {
        if k > 0 {
            rows.space(2.0);
        }
        if !feature(ui, app, rows, m.toggle, free) {
            continue;
        }
        inner(rows, |rows| {
            texture_color(
                ui,
                app,
                rows,
                m.tex,
                lang.pick("マットキャップ", "MatCap"),
                m.color,
                ALPHA,
                free,
            );
            inner(rows, |rows| {
                toggle_prop(ui, app, rows, m.zrot, free);
                toggle_prop(ui, app, rows, m.perspective, free);
            });
            float_row(ui, app, rows, m.main, free);
            float_row(ui, app, rows, m.normal, free);
            rows.space(2.0);
            slot_row(
                ui,
                app,
                rows,
                m.mask,
                Some(lang.pick("マスク", "Mask")),
                free,
            );
            inner(rows, |rows| float_row(ui, app, rows, m.blend, free));
            float_row(ui, app, rows, m.lighting, free);
            float_row(ui, app, rows, m.shadow, free);
            toggle_prop(ui, app, rows, m.backface, free);
            float_row(ui, app, rows, m.lod, free);
            choice_prop(ui, app, rows, m.mode, free);
            if transparent {
                toggle_prop(ui, app, rows, m.transparency, free);
            }
            rows.space(2.0);
            toggle_prop(ui, app, rows, m.custom, free);
            if liltoon::on(app.doc.drawn_look(), m.custom) {
                inner(rows, |rows| {
                    slot_row(
                        ui,
                        app,
                        rows,
                        m.bump,
                        Some(lang.pick("ノーマルマップ", "Normal Map")),
                        free,
                    );
                    float_row(ui, app, rows, m.bump_scale, free);
                });
            }
        });
    }
}

fn rim(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.rim",
        "リムライト設定",
        "Rim Light",
        Section::Rim,
    ) {
        return;
    }
    let lang = app.lang;
    if !feature(ui, app, rows, "_UseRim", free) {
        return;
    }
    inner(rows, |rows| {
        texture_color(
            ui,
            app,
            rows,
            "_RimColorTex",
            lang.pick("色 / マスク", "Color / Mask"),
            "_RimColor",
            ALPHA,
            free,
        );
        for name in ["_RimMainStrength", "_RimEnableLighting", "_RimShadowMask"] {
            float_row(ui, app, rows, name, free);
        }
        toggle_prop(ui, app, rows, "_RimBackfaceMask", free);
        if liltoon::shader_info(app.doc.drawn_look()).mode == RenderMode::Transparent {
            toggle_prop(ui, app, rows, "_RimApplyTransparency", free);
        }
        choice_prop(ui, app, rows, "_RimBlendMode", free);
        rows.space(2.0);
        float_row(ui, app, rows, "_RimDirStrength", free);
        if liltoon::number(app.doc.drawn_look(), "_RimDirStrength") != 0.0 {
            inner(rows, |rows| {
                float_row(ui, app, rows, "_RimDirRange", free);
                inv_border_row(ui, app, rows, "_RimBorder", free);
                float_row(ui, app, rows, "_RimBlur", free);
                rows.space(2.0);
                float_row(ui, app, rows, "_RimIndirRange", free);
                color_row(ui, app, rows, "_RimIndirColor", None, ALPHA, free);
                inv_border_row(ui, app, rows, "_RimIndirBorder", free);
                float_row(ui, app, rows, "_RimIndirBlur", free);
            });
        } else {
            inv_border_row(ui, app, rows, "_RimBorder", free);
            float_row(ui, app, rows, "_RimBlur", free);
        }
        float_row(ui, app, rows, "_RimNormalStrength", free);
        float_row(ui, app, rows, "_RimFresnelPower", free);
    });
}

fn glitter(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.glitter",
        "ラメ設定",
        "Glitter",
        Section::Glitter,
    ) {
        return;
    }
    let lang = app.lang;
    if !feature(ui, app, rows, "_UseGlitter", free) {
        return;
    }
    inner(rows, |rows| {
        choice_prop(ui, app, rows, "_GlitterUVMode", free);
        texture_color(
            ui,
            app,
            rows,
            "_GlitterColorTex",
            lang.pick("色 / マスク", "Color / Mask"),
            "_GlitterColor",
            ALPHA,
            free,
        );
        inner(rows, |rows| {
            choice_prop(ui, app, rows, "_GlitterColorTex_UVMode", free);
            for name in [
                "_GlitterMainStrength",
                "_GlitterEnableLighting",
                "_GlitterShadowMask",
            ] {
                float_row(ui, app, rows, name, free);
            }
            toggle_prop(ui, app, rows, "_GlitterBackfaceMask", free);
            if liltoon::shader_info(app.doc.drawn_look()).mode == RenderMode::Transparent {
                toggle_prop(ui, app, rows, "_GlitterApplyTransparency", free);
            }
        });
        rows.space(2.0);
        glitter_params(ui, app, rows, free);
        let p2 = "_GlitterParams2";
        vector_part(
            ui,
            app,
            rows,
            p2,
            0,
            lang.pick("点滅の速度", "Blink Speed"),
            (0.0, 10.0),
            free,
        );
        vector_part(
            ui,
            app,
            rows,
            p2,
            1,
            lang.pick("角度制限", "Angle limit"),
            (0.0, 1.0),
            free,
        );
        vector_part(
            ui,
            app,
            rows,
            p2,
            2,
            lang.pick("ライト方向の影響度", "Light direction strength"),
            (0.0, 1.0),
            free,
        );
        vector_part(
            ui,
            app,
            rows,
            p2,
            3,
            lang.pick("ランダムカラー", "Color Randomness"),
            (0.0, 1.0),
            free,
        );
        float_row(ui, app, rows, "_GlitterNormalStrength", free);
        float_row(ui, app, rows, "_GlitterPostContrast", free);
    });
}

/// ラメの大きさ・密度（lilToon の欄の換算: `_GlitterParams1` は (256 / サイズ X, 256 / サイズ Y, パーティクルサイズ², 1 / (密度 × 1.5)²)、
/// 感度は `_GlitterSensitivity` / 密度）。
fn glitter_params(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    let lang = app.lang;
    let look = app.doc.drawn_look().clone();
    let mut next = fields::glitter_shown(
        liltoon::value(&look, "_GlitterParams1"),
        liltoon::number(&look, "_GlitterSensitivity"),
    );
    let mut changed = false;
    let specs: [(&str, &str, f32, f32); 5] = [
        ("サイズ X", "Scale X", 0.0, 10.0),
        ("サイズ Y", "Scale Y", 0.0, 10.0),
        ("パーティクルサイズ", "Particle Size", 0.0, 2.0),
        ("密度", "Density", 0.001, 1.0),
        ("感度", "Sensitivity", 0.0, 100.0),
    ];
    for (k, (ja, en, lo, hi)) in specs.into_iter().enumerate() {
        let (label, tip) = marked(app, "_GlitterParams1", lang.pick(ja, en));
        let spec = SliderSpec::new(
            &label,
            lo,
            hi,
            NumberFormat {
                decimals: 3,
                trim: false,
                suffix: "",
            },
        )
        .tooltip(&tip)
        .enabled(free);
        let out = w::slider(ui, rows.slider_row(), ("look.glitter", k), next[k], &spec);
        if out.changed {
            changed = true;
            next[k] = out.value;
        }
        if k == 2 {
            float_row(ui, app, rows, "_GlitterScaleRandomize", free);
        }
    }
    if changed {
        app.apply(look_action(fields::glitter_op(next, true)));
    }
}

fn outline(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.outline",
        "輪郭線設定",
        "Outline",
        Section::Outline,
    ) {
        return;
    }
    let lang = app.lang;
    // 節の頭の入切（Unity ではシェーダーの名前が変わる: lilToon ↔ lilToonOutline）
    let info = liltoon::shader_info(app.doc.drawn_look());
    let mut label = lang.pick("輪郭線", "Outline").to_owned();
    let mut tip = "lilToonOutline".to_owned();
    if let Some(received) = app
        .doc
        .received_look()
        .filter(|_| !app.doc.look().shader.is_empty())
    {
        let unity = liltoon::shader_info(&received.look);
        if unity.outline != info.outline {
            label += " •";
            tip += &format!(
                "\nUnity: {}",
                if unity.outline {
                    lang.pick("入", "On")
                } else {
                    lang.pick("切", "Off")
                }
            );
        }
    }
    if let Some(next) = toggle_row(
        ui,
        rows,
        "look.outline",
        &label,
        info.outline,
        Some(tip.as_str()),
        free,
    ) {
        app.apply(look_action(LookOp::Outline(next)));
    }
    if !info.outline {
        return;
    }
    inner(rows, |rows| {
        let label = if info.mode == RenderMode::Opaque {
            lang.pick("色", "Color")
        } else {
            lang.pick("色 / 透明度", "Color / Alpha")
        };
        texture_color(
            ui,
            app,
            rows,
            "_OutlineTex",
            label,
            "_OutlineColor",
            ALPHA,
            free,
        );
        inner(rows, |rows| hsvg(ui, app, rows, "_OutlineTexHSVG", free));
        rows.space(2.0);
        group_label(ui, rows, lang.pick("ハイライト", "Highlight"));
        inner(rows, |rows| {
            color_row(ui, app, rows, "_OutlineLitColor", None, ALPHA, free);
            if liltoon::value(app.doc.drawn_look(), "_OutlineLitColor")[3] > 0.0 {
                toggle_prop(ui, app, rows, "_OutlineLitApplyTex", free);
                let look = app.doc.drawn_look().clone();
                let scale = liltoon::number(&look, "_OutlineLitScale");
                let offset = liltoon::number(&look, "_OutlineLitOffset");
                let (min_label, tip) = marked(app, "_OutlineLitScale", "Min");
                let (max_label, _) = marked(app, "_OutlineLitScale", "Max");
                if let Some((lo, hi)) = remap_rows(
                    ui,
                    rows,
                    ("outline", 0),
                    (&min_label, &max_label),
                    &tip,
                    scale,
                    offset,
                    free,
                ) {
                    app.apply(look_action(fields::outline_lit_op(lo, hi, true)));
                }
                toggle_prop(ui, app, rows, "_OutlineLitShadowReceive", free);
            }
        });
        rows.space(2.0);
        float_row(ui, app, rows, "_OutlineEnableLighting", free);
        rows.space(2.0);
        slot_row(
            ui,
            app,
            rows,
            "_OutlineWidthMask",
            Some(lang.pick("マスクと太さ", "Mask & Width")),
            free,
        );
        inner(rows, |rows| {
            float_row_in(ui, app, rows, "", "_OutlineWidth", Some("Width"), free);
            float_row(ui, app, rows, "_OutlineFixWidth", free);
            choice_prop(ui, app, rows, "_OutlineVertexR2Width", free);
            toggle_prop(ui, app, rows, "_OutlineDeleteMesh", free);
            float_row(ui, app, rows, "_OutlineZBias", free);
        });
    });
}

fn distance_fade(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(
        ui,
        app,
        rows,
        "look.distancefade",
        "距離フェード",
        "Distance Fade",
        Section::DistanceFade,
    ) {
        return;
    }
    let lang = app.lang;
    color_row(ui, app, rows, "_DistanceFadeColor", None, ALPHA, free);
    inner(rows, |rows| {
        let name = "_DistanceFade";
        vector_part(
            ui,
            app,
            rows,
            name,
            0,
            lang.pick("開始距離", "Start Distance"),
            (0.0, 10.0),
            free,
        );
        vector_part(
            ui,
            app,
            rows,
            name,
            1,
            lang.pick("終了距離", "End Distance"),
            (0.0, 10.0),
            free,
        );
        vector_part(
            ui,
            app,
            rows,
            name,
            2,
            lang.pick("強度", "Strength"),
            (0.0, 1.0),
            free,
        );
        vector_toggle(
            ui,
            app,
            rows,
            name,
            3,
            lang.pick("裏面を影にする", "Backface Force Shadow"),
            free,
        );
        choice_prop(ui, app, rows, "_DistanceFadeMode", free);
    });
    rows.space(2.0);
    group_label(ui, rows, lang.pick("リムライト", "Rim Light"));
    inner(rows, |rows| {
        color_row(ui, app, rows, "_DistanceFadeRimColor", None, ALPHA, free);
        float_row(ui, app, rows, "_DistanceFadeRimFresnelPower", free);
    });
}

// ───────── ドロップダウンの項目 ─────────

/// `Popup::Look` の項目。
pub fn entries(app: &AppState, choice: LookChoice) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking() && app.read_only_reason().is_none();
    let look = app.doc.drawn_look();
    // 複数のプロパティを 1 回で決める項目（ミラーモード・複製モード・光沢のタイプ）は `fields` の操作
    // 接頭辞と名前の後ろからプロパティの名前（表の 'static の名前）
    let named = |prefix: &str, s: &str| -> &'static str {
        let name = format!("{prefix}Tex{s}");
        liltoon::prop(&name).map_or("", |p| p.name)
    };
    match choice {
        LookChoice::Kind => [LookKind::Standard, LookKind::LilToon]
            .into_iter()
            .map(|k| {
                Entry::item(kind_label(lang, k), look_action(LookOp::Kind(k)))
                    .radio(look.kind == k)
                    .enabled(free)
            })
            .collect(),
        LookChoice::Mode => {
            let current = liltoon::shader_info(look).mode;
            RenderMode::ALL
                .into_iter()
                .map(|m| {
                    Entry::item(m.label(lang), look_action(LookOp::Mode(m)))
                        .radio(current == m)
                        .enabled(free)
                })
                .collect()
        }
        LookChoice::Prop(name) => {
            let Some(Kind::Choice(options)) = liltoon::prop(name).map(|p| p.kind) else {
                return Vec::new();
            };
            let at = liltoon::number(look, name).round() as i64;
            options
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    let mut e = Entry::item(
                        lang.pick(o.0, o.1),
                        look_action(LookOp::Value {
                            name,
                            value: LookValue::Float(i as f32),
                            drag: false,
                        }),
                    )
                    .radio(at == i as i64)
                    .enabled(free && choice_drawn(name, i));
                    if !choice_drawn(name, i) {
                        e = e.tooltip(not_drawn_reason(lang, name));
                    }
                    e
                })
                .collect()
        }
        LookChoice::Slot(index) => slot_entries(app, index, free),
        LookChoice::Plane(index, k) => plane_entries(app, index, k, free),
        LookChoice::SpecularMode => {
            let current = specular_mode(
                liltoon::on(look, "_ApplySpecular"),
                liltoon::on(look, "_SpecularToon"),
            );
            SPECULAR
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    Entry::item(n.pick(lang), look_action(fields::specular_op(i)))
                        .radio(current == i)
                        .enabled(free)
                })
                .collect()
        }
        LookChoice::Mirror(prefix) => {
            let (left, right, flip) = (
                named(prefix, "IsLeftOnly"),
                named(prefix, "IsRightOnly"),
                named(prefix, "ShouldFlipMirror"),
            );
            let current = mirror_mode(
                liltoon::on(look, left),
                liltoon::on(look, right),
                liltoon::on(look, flip),
            );
            MIRROR
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    Entry::item(n.pick(lang), look_action(fields::mirror_op(prefix, i)))
                        .radio(current == i)
                        .enabled(free)
                })
                .collect()
        }
        LookChoice::Copy(prefix) => {
            let (copy, flip) = (named(prefix, "ShouldCopy"), named(prefix, "ShouldFlipCopy"));
            let current = copy_mode(liltoon::on(look, copy), liltoon::on(look, flip));
            COPY.iter()
                .enumerate()
                .map(|(i, n)| {
                    Entry::item(n.pick(lang), look_action(fields::copy_op(prefix, i)))
                        .radio(current == i)
                        .enabled(free)
                })
                .collect()
        }
    }
}

/// 選べる値のうち、再現が描くもの（描かないものは押せない）。UV1〜UV3 はモデルが持たない（UV0 で読む）。
fn choice_drawn(name: &str, value: usize) -> bool {
    match name {
        "_ShadowColorType" => value == 0,
        "_EmissionMap_UVMode" | "_Emission2ndMap_UVMode" => value == 0 || value == 4,
        "_Main2ndTex_UVMode" | "_Main3rdTex_UVMode" => value == 0 || value == 4,
        "_Bump2ndMap_UVMode" | "_GlitterUVMode" | "_GlitterColorTex_UVMode" => value == 0,
        "_OutlineVertexR2Width" => value == 0,
        _ => true,
    }
}

/// 描かない値の理由（ツールチップ）。
fn not_drawn_reason(lang: Lang, name: &str) -> &'static str {
    if name.ends_with("UVMode") {
        lang.pick(
            "描かない（モデルの UV は UV0 だけ。UV0 で読む）",
            "Not drawn (the model has UV0 only; reads UV0)",
        )
    } else if name == "_OutlineVertexR2Width" {
        lang.pick(
            "描かない（モデルの頂点カラーを持たない）",
            "Not drawn (the model has no vertex colors)",
        )
    } else {
        lang.pick("描かない（値は持つ）", "Not drawn (the value is kept)")
    }
}

fn slot_entries(app: &AppState, index: usize, free: bool) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let slot = SLOTS[index];
    let name = slot.name;
    // 選べる項目の印は利用者の割り当て（割り当てを外すと、Unity から受けた割り当て・絵か既定で描く）
    let current = app.doc.look().textures.get(name).copied();
    let set = |source: Option<TextureSource>| look_action(LookOp::Texture { slot: name, source });
    let received = app.doc.received_look();
    let unset = match received {
        Some(r) if r.look.textures.contains_key(name) => format!(
            "{}（Unity）",
            source_name(app, &slot, r.look.textures.get(name))
        ),
        Some(r) if r.images.contains_key(name) || r.missing.contains_key(name) => {
            lang.pick("Unity のテクスチャ", "Unity texture").to_owned()
        }
        _ => default_name(lang, slot.default).to_owned(),
    };
    let mut v = vec![Entry::item(unset, set(None))
        .radio(current.is_none())
        .enabled(free)];
    v.push(Entry::Separator);
    for c in app.doc.channels() {
        v.push(
            Entry::item(
                channel_name(lang, &app.doc, c),
                set(Some(TextureSource::Channel(c))),
            )
            .radio(current == Some(TextureSource::Channel(c)))
            .enabled(free),
        );
    }
    // チャンネルの選択肢の最後: スロットの名前で新しいチャンネルを作って割り当てる（マットキャップの絵のスロットは描くものではない）
    if let Some(spec) = super::paint_channel_spec(&slot, lang) {
        let made = super::free_name(&app.doc, &spec.name);
        v.push(new_channel_entry(
            lang,
            &made,
            LookOp::NewChannel {
                slot: name,
                plane: None,
            },
            free,
        ));
    }
    v.push(Entry::Separator);
    let packed = match current {
        Some(TextureSource::Packed(p)) => p,
        Some(TextureSource::Channel(c)) => {
            let scalar = app
                .doc
                .channel_info(c)
                .is_some_and(|i| i.kind == ChannelKind::Scalar);
            std::array::from_fn(|k| {
                if scalar && k == 3 {
                    PlaneSource::One
                } else {
                    PlaneSource::Channel {
                        channel: c,
                        component: if scalar { 0 } else { k as u8 },
                    }
                }
            })
        }
        _ => {
            let d = slot.default.rgba();
            std::array::from_fn(|k| {
                if d[k] > 0.5 {
                    PlaneSource::One
                } else {
                    PlaneSource::Zero
                }
            })
        }
    };
    v.push(
        Entry::item(
            lang.pick("成分ごと", "Per Component"),
            set(Some(TextureSource::Packed(packed))),
        )
        .radio(matches!(current, Some(TextureSource::Packed(_))))
        .enabled(free),
    );
    if slot.usage == SlotUse::Image {
        let images: Vec<_> = app
            .shelf
            .resources()
            .iter()
            .filter(|r| r.kind == "image")
            .collect();
        if !images.is_empty() {
            v.push(Entry::Separator);
        }
        for r in images {
            let Some(id) = crate::fx::inputs::image_id(&r.id) else {
                continue;
            };
            v.push(
                Entry::item(r.name.clone(), set(Some(TextureSource::Image(id))))
                    .radio(current == Some(TextureSource::Image(id)))
                    .enabled(free),
            );
        }
    }
    v
}

fn plane_entries(app: &AppState, index: usize, k: u8, free: bool) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let slot = SLOTS[index];
    let Some(TextureSource::Packed(planes)) = app.doc.drawn_look().textures.get(slot.name).copied()
    else {
        return Vec::new();
    };
    let set = |p: PlaneSource| {
        let mut next = planes;
        next[k as usize] = p;
        look_action(LookOp::Texture {
            slot: slot.name,
            source: Some(TextureSource::Packed(next)),
        })
    };
    let current = planes[k as usize];
    let mut v = vec![
        Entry::item("0", set(PlaneSource::Zero))
            .radio(current == PlaneSource::Zero)
            .enabled(free),
        Entry::item("1", set(PlaneSource::One))
            .radio(current == PlaneSource::One)
            .enabled(free),
        Entry::Separator,
    ];
    for c in app.doc.channels() {
        let scalar = app
            .doc
            .channel_info(c)
            .is_some_and(|i| i.kind == ChannelKind::Scalar);
        let components: &[u8] = if scalar { &[0] } else { &[0, 1, 2, 3] };
        for &component in components {
            let p = PlaneSource::Channel {
                channel: c,
                component,
            };
            let name = channel_name(lang, &app.doc, c);
            let label = if scalar {
                name
            } else {
                format!("{name} {}", ["R", "G", "B", "A"][component as usize])
            };
            v.push(Entry::item(label, set(p)).radio(current == p).enabled(free));
        }
    }
    // 最後: この成分だけの新しいスカラーのチャンネル（名前は描く口と同じ名前に成分の文字）
    if k < 4 {
        let base = super::paint_channel_spec(&slot, lang)
            .map_or_else(|| slot.label(lang).to_owned(), |spec| spec.name);
        let made = super::free_name(
            &app.doc,
            &format!("{base} {}", ["R", "G", "B", "A"][k as usize]),
        );
        v.push(new_channel_entry(
            lang,
            &made,
            LookOp::NewChannel {
                slot: slot.name,
                plane: Some(k),
            },
            free,
        ));
    }
    v
}

/// 「新しいチャンネル」の項目（押すと `made` の名前でチャンネルを作って割り当て、描くチャンネルにする。何が起きるかはツールチップ）。
fn new_channel_entry(lang: Lang, made: &str, op: LookOp, free: bool) -> Entry<Action> {
    Entry::item(
        lang.pick("新しいチャンネル", "New Channel"),
        look_action(op),
    )
    .enabled(free)
    .tooltip(lang.pick(
        format!("「{made}」を作って割り当てる（描くチャンネルにもなる）"),
        format!("Creates and assigns \"{made}\", and makes it the channel to paint"),
    ))
}
