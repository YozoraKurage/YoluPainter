//! 見た目の設定の欄（右のプロパティの「マテリアル」のタブ）: 種類（標準・lilToon）と、lilToon のときはインスペクターと同じ節の順・
//! 名前の付け方の欄（見た目はよるぺの欄の部品）。値を変えると 3D ビューがその場で変わり、1 回の Undo（スライダーのドラッグは 1 段）。
//! 画面には名前と値だけを出し、説明はツールチップ（lilToon のプロパティの名前も）。再現しない機能の欄は出さない。
//!
//! Live Link で Unity の本物のマテリアルの値を受けたセット（`Document::received_look`）は、欄に描く見た目（Unity の値の上に欄で変えた値）
//! を見せ、欄で変えた項目の名前に印（•）を付けてツールチップに Unity の値を添える。「Unity の値」の行は、受けている・最後の値の様子と、
//! 欄で変えた値を外して Unity に合わせるボタン（`LookOp::FollowReceived`）。

use egui::{pos2, vec2, Rect, Ui};
use yolu_core::look::{LookKind, LookValue, PlaneSource, TextureSource};
use yolu_core::{Channel, ChannelKind};

use super::liltoon::{self, Kind, RenderMode, SlotDefault, SlotUse, SLOTS};
use super::{LookOp, Section};
use crate::lang::Lang;
use crate::m2::channel_name;
use crate::m2_menu::Popup;
use crate::panels::properties::{open_popup, section, subsection, toggle_row};
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
}

const LABEL_W: f32 = 92.0;

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
            "このテクスチャセットの面を 3D ビューで描く方式（標準は PBR、lilToon は lilToon 2.3 の基本の見た目の再現）",
            "How the 3D View draws this texture set (Standard is PBR, lilToon reproduces the basics of lilToon 2.3)",
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
            "入にしている機能（影・リム・マットキャップ・発光・輪郭線）のマスクのユーザーチャンネルを作って割り当てる。機能が 1 つも入でなければ影を入にする",
            "Creates user channels for the masks of enabled features (shadow, rim, MatCap, emission, outline) and assigns them; turns the shadow on when nothing is on",
        )),
        Some("auto_awesome"),
    )
    .clicked()
    {
        app.apply(look_action(LookOp::Template));
    }
    rows.space(2.0);
    base(ui, app, rows, free);
    lighting(ui, app, rows, free);
    main_color(ui, app, rows, free);
    shadow(ui, app, rows, free);
    emission(ui, app, rows, free);
    normal_map(ui, app, rows, free);
    matcap(ui, app, rows, free);
    rim(ui, app, rows, free);
    outline(ui, app, rows, free);
    rows.indent = t::SECTION_INDENT;
    rows.space(4.0);
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
        lang.pick("Unity のマテリアルの値", "Values of the Unity material").to_owned()
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
    let response = ui.interact(row, ui.make_persistent_id("look.received"), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{name}: {state}")));
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
        MissingImage::Pending => lang.pick("Unity（届いていない）", "Unity (not here yet)"),
        MissingImage::OverBudget => lang.pick("Unity（送らない）", "Unity (not sent)"),
        MissingImage::Unreadable => lang.pick("Unity（読めない）", "Unity (unreadable)"),
    }
}

fn missing_text(lang: Lang, why: yolu_core::look::MissingImage) -> &'static str {
    use yolu_core::look::MissingImage;
    match why {
        MissingImage::Pending => lang.pick("Unity のテクスチャ（届いていない）", "Unity texture (not here yet)"),
        MissingImage::OverBudget => lang.pick("Unity のテクスチャ（予算を超えたので送らない）", "Unity texture (over the budget, not sent)"),
        MissingImage::Unreadable => lang.pick("Unity のテクスチャ（読めない）", "Unity texture (unreadable)"),
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
                let c = if liltoon::is_linear_color(name) { to_gamma(c) } else { c };
                format!("{} · {:.2}", crate::state::to_hex([c[0], c[1], c[2], 1.0]), c[3])
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

fn choice(ui: &mut Ui, rows: &mut Rows, id: &str, label: &str, value: &str, tooltip: &str, enabled: bool) -> Option<Rect> {
    let r = rows.row(t::ROW_HEIGHT, 4.0);
    let (response, b) = w::dropdown(ui, r, id, Some(label), value, Some(tooltip), enabled, LABEL_W);
    response.clicked().then_some(b)
}

fn sub(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, key: &'static str, ja: &str, en: &str, section: Section) -> bool {
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

// ───────── 値の行 ─────────

fn float_row(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, name: &'static str, enabled: bool) {
    float_row_in(ui, app, rows, "", name, enabled);
}

/// `float_row` の、節の鍵（`scope`）を ID に混ぜる形。lilToon のインスペクターは同じプロパティを 2 つの節に出すことがある
/// （`_ShadowEnvStrength` はライティング設定と影設定）ので、両方の節を開いても欄の ID が重ならないように。
fn float_row_in(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, scope: &'static str, name: &'static str, enabled: bool) {
    let Some(prop) = liltoon::prop(name) else {
        return;
    };
    let (min, max, power) = match prop.kind {
        Kind::Slider { min, max, power } => (min, max, power),
        Kind::Number { min, max } => (min, max, 1.0),
        _ => (0.0, 1.0, 1.0),
    };
    let value = liltoon::number(app.doc.drawn_look(), name);
    let (label, tip) = marked(app, name, prop.label(app.lang));
    let spec = SliderSpec::new(
        &label,
        min,
        max,
        NumberFormat {
            decimals: if max - min > 20.0 { 1 } else { 2 },
            trim: false,
            suffix: "",
        },
    )
    .tooltip(&tip)
    .enabled(enabled)
    .power(power);
    let out = w::slider(ui, rows.slider_row(), ("look.f", scope, name), value, &spec);
    if out.changed {
        app.apply(look_action(LookOp::Value {
            name,
            value: LookValue::Float(out.value),
            drag: true,
        }));
    }
}

fn toggle_prop(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, name: &'static str, enabled: bool) {
    let Some(prop) = liltoon::prop(name) else {
        return;
    };
    let on = liltoon::on(app.doc.drawn_look(), name);
    let (label, tip) = marked(app, name, prop.label(app.lang));
    if let Some(next) = toggle_row(ui, rows, &format!("look.t.{name}"), &label, on, Some(&tip), enabled) {
        app.apply(look_action(LookOp::Value {
            name,
            value: LookValue::Float(f32::from(next)),
            drag: false,
        }));
    }
}

fn choice_prop(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, name: &'static str, enabled: bool) {
    let Some(prop) = liltoon::prop(name) else {
        return;
    };
    let Kind::Choice(options) = prop.kind else {
        return;
    };
    let lang = app.lang;
    let at = liltoon::number(app.doc.drawn_look(), name).round().max(0.0) as usize;
    let value = options.get(at).map_or("?", |o| lang.pick(o.0, o.1));
    let (label, tip) = marked(app, name, prop.label(lang));
    if let Some(r) = choice(ui, rows, &format!("look.c.{name}"), &label, value, &tip, enabled) {
        open_popup(app, &ui.ctx().clone(), Popup::Look(LookChoice::Prop(name)), r, r.width());
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

/// 色の行（見本を押すと描画色、16 進で打つ）。`alpha` があれば、その名前で不透明度の行も出す。発光の色（Unity の [HDR]）は値が
/// リニアなので、見本と 16 進はガンマに直して見せ、明るさ（1 を超える倍率）の行も出す。
fn color_row(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    name: &'static str,
    alpha: Option<(&str, &str)>,
    enabled: bool,
) {
    let Some(prop) = liltoon::prop(name) else {
        return;
    };
    let lang = app.lang;
    let raw = liltoon::value(app.doc.drawn_look(), name);
    let linear = liltoon::is_linear_color(name);
    let intensity = if linear { raw[0].max(raw[1]).max(raw[2]).max(1.0) } else { 1.0 };
    let unit = [raw[0] / intensity, raw[1] / intensity, raw[2] / intensity, raw[3]];
    let shown = if linear { to_gamma(unit) } else { unit };
    let row = rows.row(t::ROW_HEIGHT, 2.0);
    let (label, unity) = marked(app, name, prop.label(lang));
    let label_rect = Rect::from_min_size(row.min, vec2(LABEL_W, row.height()));
    w::text(
        ui.painter(),
        label_rect,
        &label,
        t::LABEL.with_color(if enabled { t::TEXT } else { t::TEXT_DISABLED }),
        w::Align::Left,
    );
    if unity != name {
        ui.interact(label_rect, ui.make_persistent_id(("look.color.label", name)), egui::Sense::hover())
            .on_hover_text(unity);
    }
    let swatch = Rect::from_min_size(pos2(row.left() + LABEL_W, row.top() + 1.0), vec2(36.0, row.height() - 2.0));
    let tip = lang.pick(format!("{name}（押すと描画色）"), format!("{name} (click to take the paint color)"));
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
    if w::color_swatch(ui, swatch, ("look.swatch", name), [shown[0], shown[1], shown[2], 1.0], &tip, enabled).clicked() && enabled {
        let m = app.color.main;
        set(app, [m[0], m[1], m[2]], false);
    }
    let hex_rect = Rect::from_min_max(pos2(swatch.right() + 4.0, row.top() + 1.0), pos2(row.right(), row.bottom() - 1.0));
    if hex_rect.width() > 40.0 && enabled {
        let current = crate::state::to_hex([shown[0], shown[1], shown[2], 1.0]);
        let out = w::text_field(ui, hex_rect, ("look.hex", name), &current, Some(lang.pick("16 進（#RRGGBB）", "Hex (#RRGGBB)")), false);
        if let Some(text) = out.committed {
            if let Some(rgb) = crate::state::parse_hex(&text) {
                set(app, rgb, false);
            }
        }
    }
    if linear {
        let spec = SliderSpec::new(lang.pick("明るさ", "Intensity"), 0.0, 8.0, NumberFormat { decimals: 2, trim: false, suffix: "" })
            .tooltip(name)
            .enabled(enabled);
        let out = w::slider(ui, rows.slider_row(), ("look.intensity", name), intensity, &spec);
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
        let spec = SliderSpec::new(lang.pick(ja, en), 0.0, 1.0, NumberFormat { decimals: 2, trim: false, suffix: "" })
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
    let spec = SliderSpec::new(&label, range.0, range.1, NumberFormat { decimals: 2, trim: false, suffix: "" })
        .tooltip(&tip)
        .enabled(enabled);
    let out = w::slider(ui, rows.slider_row(), ("look.v", name, index), v[index], &spec);
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

/// AO の範囲（lilToon の 1st Min・Max のように、スケールとオフセットを最小・最大で見せる）。
fn ao_range(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, enabled: bool) {
    let lang = app.lang;
    for (k, (ja, en)) in [("1 影", "1st"), ("2 影", "2nd"), ("3 影", "3rd")].iter().enumerate() {
        let (name, i) = if k < 2 { ("_ShadowAOShift", k * 2) } else { ("_ShadowAOShift2", 0) };
        let v = liltoon::value(app.doc.drawn_look(), name);
        let (scale, offset) = (v[i], v[i + 1]);
        // value = saturate(ao × scale + offset): min は値 0 になる AO、max は値 1 になる AO
        let min = if scale.abs() < 1e-6 { 0.0 } else { -offset / scale };
        let max = if scale.abs() < 1e-6 { 1.0 } else { (1.0 - offset) / scale };
        for (which, label, value) in [(0, lang.pick(format!("{ja} 最小"), format!("{en} Min")), min), (1, lang.pick(format!("{ja} 最大"), format!("{en} Max")), max)] {
            let (label, tip) = marked(app, name, &label);
            let spec = SliderSpec::new(&label, -0.01, 1.01, NumberFormat { decimals: 2, trim: false, suffix: "" })
                .tooltip(&tip)
                .enabled(enabled);
            let out = w::slider(ui, rows.slider_row(), ("look.ao", k, which), value, &spec);
            if out.changed {
                let (mut lo, mut hi) = (min, max);
                if which == 0 {
                    lo = out.value;
                } else {
                    hi = out.value;
                }
                if (hi - lo).abs() < 0.001 {
                    hi = lo + 0.001;
                }
                let s = 1.0 / (hi - lo);
                let mut next = v;
                next[i] = s;
                next[i + 1] = -lo * s;
                app.apply(look_action(LookOp::Value {
                    name,
                    value: LookValue::Vector(next),
                    drag: true,
                }));
            }
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
            app.shelf
                .get(&rid)
                .map_or_else(|| lang.pick("（無い画像）", "(missing image)").to_owned(), |r| r.name.clone())
        }
    }
}

fn plane_name(app: &AppState, p: PlaneSource) -> String {
    match p {
        PlaneSource::Zero => "0".into(),
        PlaneSource::One => "1".into(),
        PlaneSource::Channel { channel, component } => {
            let name = channel_name(app.lang, &app.doc, channel);
            if app.doc.channel_info(channel).is_some_and(|i| i.kind == ChannelKind::Scalar) {
                name
            } else {
                format!("{name} {}", ["R", "G", "B", "A"][component.min(3) as usize])
            }
        }
    }
}

/// スロットが読むユーザーチャンネルが、3D ビューの配列の層の上限（16）を超えて描かれないか（17 個目から）。
pub fn slot_over_layer_limit(doc: &yolu_core::Document, name: &str) -> bool {
    let look = doc.drawn_look();
    let Some(source) = look.textures.get(name) else {
        return false;
    };
    let dropped = crate::view3d::look_gpu::dropped_users(doc, look);
    !dropped.is_empty() && source.channels().iter().any(|c| dropped.contains(c))
}

/// スロットの Unity から受けた絵が、3D ビューの受けた絵の配列の層の上限（16）を超えて描かれないか（スロットの並びで 17 枚目から）。
pub fn slot_over_received_limit(doc: &yolu_core::Document, name: &str) -> bool {
    let look = doc.drawn_look();
    if look.textures.contains_key(name) {
        return false;
    }
    crate::view3d::look_gpu::dropped_received(doc, look)
        .iter()
        .any(|s| s == name)
}

fn slot_row(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, name: &'static str, enabled: bool) {
    let Some(index) = liltoon::slot_index(name) else {
        return;
    };
    let slot = SLOTS[index];
    let lang = app.lang;
    let source = app.doc.drawn_look().textures.get(name).copied();
    let mut value = source_name(app, &slot, source.as_ref());
    let mut tip = lang.pick(
        format!("{name}: 読むチャンネル（成分ごとに選ぶこともできる）。割り当てないと {} のテクスチャ", default_name(lang, slot.default)),
        format!("{name}: the channel it reads (or one per component). Unassigned reads a {} texture", default_name(lang, slot.default)),
    );
    if source.is_none() {
        // 割り当てていないスロットは、Unity から受けた絵があればそれで描く
        if let Some(received) = app.doc.received_look() {
            if let Some(image) = received.images.get(name) {
                value = lang.pick("Unity のテクスチャ", "Unity texture").into();
                tip += &format!("\n{}×{}", image.width, image.height);
            } else if let Some(why) = received.missing.get(name) {
                value = missing_short(lang, *why).into();
                tip += &format!("\n{}", missing_text(lang, *why));
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
            format!("\n3D ビューで描かない: Unity のテクスチャは 16 枚まで（{}で描く）", default_name(lang, slot.default)),
            format!("\nNot drawn in the 3D View: up to 16 Unity textures (drawn {})", default_name(lang, slot.default).to_lowercase()),
        );
    }
    if let Some(r) = choice(ui, rows, &format!("look.slot.{name}"), slot.label(lang), &value, &tip, enabled) {
        open_popup(app, &ui.ctx().clone(), Popup::Look(LookChoice::Slot(index)), r, r.width().max(180.0));
    }
    if let Some(TextureSource::Packed(planes)) = source {
        for (k, plane) in planes.iter().enumerate() {
            let label = ["R", "G", "B", "A"][k];
            let value = plane_name(app, *plane);
            let r = rows.row(t::ROW_HEIGHT, 2.0);
            let r = Rect::from_min_max(pos2(r.left() + 14.0, r.top()), r.max);
            let (response, b) = w::dropdown(ui, r, ("look.plane", name, k), Some(label), &value, Some(name), enabled, LABEL_W - 14.0);
            if response.clicked() {
                open_popup(app, &ui.ctx().clone(), Popup::Look(LookChoice::Plane(index, k as u8)), b, b.width().max(180.0));
            }
        }
    }
}

// ───────── 節 ─────────

fn base(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(ui, app, rows, "look.base", "基本設定", "Base Setting", Section::Base) {
        return;
    }
    let lang = app.lang;
    let look = app.doc.drawn_look().clone();
    let info = liltoon::shader_info(&look);
    let mut mode = info.mode.label(lang).to_owned();
    if !info.exact {
        mode = format!("{mode}（{}）", look.shader_name());
    }
    let mut tip = if info.exact {
        lang.pick("_TransparentMode（Unity ではシェーダーの名前）", "_TransparentMode (the shader name in Unity)").to_owned()
    } else {
        lang.pick(
            format!("{}: この版の機能は描かない（近い描き方で描く）", look.shader_name()),
            format!("{}: this variant's own features are not drawn (closest look)", look.shader_name()),
        )
    };
    let mut mode_label = lang.pick("描画モード", "Rendering Mode").to_owned();
    let mut outline_label = lang.pick("輪郭線", "Outline").to_owned();
    let mut outline_tip = "lilToonOutline".to_owned();
    // 欄で描画モード・輪郭線を変えている（シェーダーの名前が利用者の設定にある）なら、ほかの項目と同じく印と Unity の値
    if let Some(received) = app.doc.received_look().filter(|_| !app.doc.look().shader.is_empty()) {
        let unity = liltoon::shader_info(&received.look);
        if unity.mode != info.mode {
            mode_label += " •";
            tip += &format!("\nUnity: {}", unity.mode.label(lang));
        }
        if unity.outline != info.outline {
            outline_label += " •";
            outline_tip += &format!("\nUnity: {}", if unity.outline { lang.pick("入", "On") } else { lang.pick("切", "Off") });
        }
    }
    if let Some(r) = choice(ui, rows, "look.mode", &mode_label, &mode, &tip, free) {
        open_popup(app, &ui.ctx().clone(), Popup::Look(LookChoice::Mode), r, r.width());
    }
    if let Some(next) = toggle_row(ui, rows, "look.outline", &outline_label, info.outline, Some(outline_tip.as_str()), free) {
        app.apply(look_action(LookOp::Outline(next)));
    }
    if info.mode != RenderMode::Opaque {
        float_row(ui, app, rows, "_Cutoff", free);
    }
    choice_prop(ui, app, rows, "_Cull", free);
    if liltoon::number(&look, "_Cull").round() <= 1.0 {
        toggle_prop(ui, app, rows, "_FlipNormal", free);
        float_row(ui, app, rows, "_BackfaceForceShadow", free);
        color_row(ui, app, rows, "_BackfaceColor", Some(("裏面の色の強さ", "Backface Color Strength")), free);
    }
    toggle_prop(ui, app, rows, "_Invisible", free);
}

fn lighting(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(ui, app, rows, "look.lighting", "ライティング設定", "Lighting", Section::Lighting) {
        return;
    }
    for name in ["_LightMinLimit", "_LightMaxLimit", "_MonochromeLighting", "_ShadowEnvStrength", "_AsUnlit", "_AAStrength"] {
        float_row_in(ui, app, rows, "lighting", name, free);
    }
}

fn main_color(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(ui, app, rows, "look.main", "メインカラー / 透過設定", "Main Color / Alpha", Section::Main) {
        return;
    }
    let lang = app.lang;
    color_row(ui, app, rows, "_Color", Some(("不透明度", "Opacity")), free);
    slot_row(ui, app, rows, "_MainTex", free);
    crate::panels::properties::group_label(ui, rows, lang.pick("色調補正", "Color Adjustment"));
    vector_part(ui, app, rows, "_MainTexHSVG", 0, lang.pick("色相", "Hue"), (-0.5, 0.5), free);
    vector_part(ui, app, rows, "_MainTexHSVG", 1, lang.pick("彩度", "Saturation"), (0.0, 2.0), free);
    vector_part(ui, app, rows, "_MainTexHSVG", 2, lang.pick("明度", "Value"), (0.0, 2.0), free);
    vector_part(ui, app, rows, "_MainTexHSVG", 3, lang.pick("ガンマ", "Gamma"), (0.01, 2.0), free);
    slot_row(ui, app, rows, "_MainColorAdjustMask", free);
    let mode = liltoon::shader_info(app.doc.drawn_look()).mode;
    if mode != RenderMode::Opaque {
        choice_prop(ui, app, rows, "_AlphaMaskMode", free);
        if liltoon::number(app.doc.drawn_look(), "_AlphaMaskMode").round() >= 1.0 {
            slot_row(ui, app, rows, "_AlphaMask", free);
            float_row(ui, app, rows, "_AlphaMaskScale", free);
            float_row(ui, app, rows, "_AlphaMaskValue", free);
        }
    }
}

fn shadow(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(ui, app, rows, "look.shadow", "影設定", "Shadow", Section::Shadow) {
        return;
    }
    let lang = app.lang;
    toggle_prop(ui, app, rows, "_UseShadow", free);
    if !liltoon::on(app.doc.drawn_look(), "_UseShadow") {
        return;
    }
    choice_prop(ui, app, rows, "_ShadowMaskType", free);
    let mask_type = liltoon::number(app.doc.drawn_look(), "_ShadowMaskType").round();
    slot_row(ui, app, rows, "_ShadowStrengthMask", free);
    if mask_type == 1.0 {
        float_row(ui, app, rows, "_ShadowFlatBorder", free);
        float_row(ui, app, rows, "_ShadowFlatBlur", free);
    } else if mask_type == 2.0 {
        float_row(ui, app, rows, "_ShadowFlatBlur", free);
    }
    float_row(ui, app, rows, "_ShadowStrength", free);
    float_row(ui, app, rows, "_ShadowStrengthMaskLOD", free);
    crate::panels::properties::group_label(ui, rows, lang.pick("1 影", "1st Shadow"));
    color_row(ui, app, rows, "_ShadowColor", None, free);
    slot_row(ui, app, rows, "_ShadowColorTex", free);
    for name in ["_ShadowBorder", "_ShadowBlur", "_ShadowNormalStrength", "_ShadowReceive"] {
        float_row(ui, app, rows, name, free);
    }
    for (prefix, ja, en) in [("_Shadow2nd", "2 影", "2nd Shadow"), ("_Shadow3rd", "3 影", "3rd Shadow")] {
        crate::panels::properties::group_label(ui, rows, lang.pick(ja, en));
        let color: &'static str = if prefix == "_Shadow2nd" { "_Shadow2ndColor" } else { "_Shadow3rdColor" };
        let tex: &'static str = if prefix == "_Shadow2nd" { "_Shadow2ndColorTex" } else { "_Shadow3rdColorTex" };
        color_row(ui, app, rows, color, Some(("強度", "Strength")), free);
        slot_row(ui, app, rows, tex, free);
        if liltoon::value(app.doc.drawn_look(), color)[3] > 0.0 {
            let names: [&'static str; 4] = if prefix == "_Shadow2nd" {
                ["_Shadow2ndBorder", "_Shadow2ndBlur", "_Shadow2ndNormalStrength", "_Shadow2ndReceive"]
            } else {
                ["_Shadow3rdBorder", "_Shadow3rdBlur", "_Shadow3rdNormalStrength", "_Shadow3rdReceive"]
            };
            for name in names {
                float_row(ui, app, rows, name, free);
            }
        }
    }
    rows.space(2.0);
    color_row(ui, app, rows, "_ShadowBorderColor", None, free);
    for name in ["_ShadowBorderRange", "_ShadowMainStrength", "_ShadowEnvStrength"] {
        float_row(ui, app, rows, name, free);
    }
    slot_row(ui, app, rows, "_ShadowBlurMask", free);
    float_row(ui, app, rows, "_ShadowBlurMaskLOD", free);
    slot_row(ui, app, rows, "_ShadowBorderMask", free);
    float_row(ui, app, rows, "_ShadowBorderMaskLOD", free);
    toggle_prop(ui, app, rows, "_ShadowPostAO", free);
    ao_range(ui, app, rows, free);
    if liltoon::number(app.doc.drawn_look(), "_ShadowColorType").round() == 1.0 {
        // LUT は描かない（値は持つ）。選んであるときだけ、通常に戻せるように欄を出す
        choice_prop(ui, app, rows, "_ShadowColorType", free);
    }
}

fn emission(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(ui, app, rows, "look.emission", "発光設定", "Emission", Section::Emission) {
        return;
    }
    for (toggle, color, map, uv, mask, blend, mode, main, fluor) in [
        (
            "_UseEmission",
            "_EmissionColor",
            "_EmissionMap",
            "_EmissionMap_UVMode",
            "_EmissionBlendMask",
            "_EmissionBlend",
            "_EmissionBlendMode",
            "_EmissionMainStrength",
            "_EmissionFluorescence",
        ),
        (
            "_UseEmission2nd",
            "_Emission2ndColor",
            "_Emission2ndMap",
            "_Emission2ndMap_UVMode",
            "_Emission2ndBlendMask",
            "_Emission2ndBlend",
            "_Emission2ndBlendMode",
            "_Emission2ndMainStrength",
            "_Emission2ndFluorescence",
        ),
    ] {
        toggle_prop(ui, app, rows, toggle, free);
        if !liltoon::on(app.doc.drawn_look(), toggle) {
            continue;
        }
        color_row(ui, app, rows, color, Some(("不透明度", "Opacity")), free);
        slot_row(ui, app, rows, map, free);
        choice_prop(ui, app, rows, uv, free);
        slot_row(ui, app, rows, mask, free);
        choice_prop(ui, app, rows, mode, free);
        for name in [blend, main, fluor] {
            float_row(ui, app, rows, name, free);
        }
        rows.space(2.0);
    }
}

fn normal_map(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(ui, app, rows, "look.normal", "ノーマルマップ設定", "Normal Map", Section::Normal) {
        return;
    }
    toggle_prop(ui, app, rows, "_UseBumpMap", free);
    if liltoon::on(app.doc.drawn_look(), "_UseBumpMap") {
        slot_row(ui, app, rows, "_BumpMap", free);
        float_row(ui, app, rows, "_BumpScale", free);
    }
}

fn matcap(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(ui, app, rows, "look.matcap", "マットキャップ設定", "MatCap", Section::MatCap) {
        return;
    }
    let sets: [[&'static str; 16]; 2] = [
        [
            "_UseMatCap",
            "_MatCapColor",
            "_MatCapTex",
            "_MatCapBlendMode",
            "_MatCapBlend",
            "_MatCapBlendMask",
            "_MatCapMainStrength",
            "_MatCapEnableLighting",
            "_MatCapShadowMask",
            "_MatCapBackfaceMask",
            "_MatCapLod",
            "_MatCapNormalStrength",
            "_MatCapZRotCancel",
            "_MatCapPerspective",
            "_MatCapApplyTransparency",
            "",
        ],
        [
            "_UseMatCap2nd",
            "_MatCap2ndColor",
            "_MatCap2ndTex",
            "_MatCap2ndBlendMode",
            "_MatCap2ndBlend",
            "_MatCap2ndBlendMask",
            "_MatCap2ndMainStrength",
            "_MatCap2ndEnableLighting",
            "_MatCap2ndShadowMask",
            "_MatCap2ndBackfaceMask",
            "_MatCap2ndLod",
            "_MatCap2ndNormalStrength",
            "_MatCap2ndZRotCancel",
            "_MatCap2ndPerspective",
            "_MatCap2ndApplyTransparency",
            "",
        ],
    ];
    let transparent = liltoon::shader_info(app.doc.drawn_look()).mode == RenderMode::Transparent;
    for names in sets {
        toggle_prop(ui, app, rows, names[0], free);
        if !liltoon::on(app.doc.drawn_look(), names[0]) {
            continue;
        }
        color_row(ui, app, rows, names[1], Some(("不透明度", "Opacity")), free);
        slot_row(ui, app, rows, names[2], free);
        choice_prop(ui, app, rows, names[3], free);
        float_row(ui, app, rows, names[4], free);
        slot_row(ui, app, rows, names[5], free);
        for name in [names[6], names[7], names[8]] {
            float_row(ui, app, rows, name, free);
        }
        toggle_prop(ui, app, rows, names[9], free);
        for name in [names[10], names[11]] {
            float_row(ui, app, rows, name, free);
        }
        toggle_prop(ui, app, rows, names[12], free);
        toggle_prop(ui, app, rows, names[13], free);
        if transparent {
            toggle_prop(ui, app, rows, names[14], free);
        }
        rows.space(2.0);
    }
}

fn rim(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !sub(ui, app, rows, "look.rim", "リムライト設定", "Rim Light", Section::Rim) {
        return;
    }
    toggle_prop(ui, app, rows, "_UseRim", free);
    if !liltoon::on(app.doc.drawn_look(), "_UseRim") {
        return;
    }
    color_row(ui, app, rows, "_RimColor", Some(("不透明度", "Opacity")), free);
    slot_row(ui, app, rows, "_RimColorTex", free);
    choice_prop(ui, app, rows, "_RimBlendMode", free);
    for name in [
        "_RimMainStrength",
        "_RimNormalStrength",
        "_RimBorder",
        "_RimBlur",
        "_RimFresnelPower",
        "_RimEnableLighting",
        "_RimShadowMask",
    ] {
        float_row(ui, app, rows, name, free);
    }
    toggle_prop(ui, app, rows, "_RimBackfaceMask", free);
    if liltoon::shader_info(app.doc.drawn_look()).mode == RenderMode::Transparent {
        toggle_prop(ui, app, rows, "_RimApplyTransparency", free);
    }
    float_row(ui, app, rows, "_RimDirStrength", free);
    if liltoon::number(app.doc.drawn_look(), "_RimDirStrength") > 0.0 {
        float_row(ui, app, rows, "_RimDirRange", free);
        float_row(ui, app, rows, "_RimIndirRange", free);
        color_row(ui, app, rows, "_RimIndirColor", Some(("間接光の不透明度", "Indirect Opacity")), free);
        float_row(ui, app, rows, "_RimIndirBorder", free);
        float_row(ui, app, rows, "_RimIndirBlur", free);
    }
}

fn outline(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    if !liltoon::shader_info(app.doc.drawn_look()).outline {
        return;
    }
    if !sub(ui, app, rows, "look.outline", "輪郭線設定", "Outline", Section::Outline) {
        return;
    }
    let lang = app.lang;
    color_row(ui, app, rows, "_OutlineColor", Some(("不透明度", "Opacity")), free);
    slot_row(ui, app, rows, "_OutlineTex", free);
    vector_part(ui, app, rows, "_OutlineTexHSVG", 0, lang.pick("色相", "Hue"), (-0.5, 0.5), free);
    vector_part(ui, app, rows, "_OutlineTexHSVG", 1, lang.pick("彩度", "Saturation"), (0.0, 2.0), free);
    vector_part(ui, app, rows, "_OutlineTexHSVG", 2, lang.pick("明度", "Value"), (0.0, 2.0), free);
    vector_part(ui, app, rows, "_OutlineTexHSVG", 3, lang.pick("ガンマ", "Gamma"), (0.01, 2.0), free);
    float_row(ui, app, rows, "_OutlineWidth", free);
    slot_row(ui, app, rows, "_OutlineWidthMask", free);
    for name in ["_OutlineFixWidth", "_OutlineEnableLighting", "_OutlineZBias"] {
        float_row(ui, app, rows, name, free);
    }
    toggle_prop(ui, app, rows, "_OutlineDeleteMesh", free);
    crate::panels::properties::group_label(ui, rows, lang.pick("ライトの色", "Lit Color"));
    color_row(ui, app, rows, "_OutlineLitColor", Some(("強度", "Strength")), free);
    toggle_prop(ui, app, rows, "_OutlineLitApplyTex", free);
    float_row(ui, app, rows, "_OutlineLitScale", free);
    float_row(ui, app, rows, "_OutlineLitOffset", free);
    toggle_prop(ui, app, rows, "_OutlineLitShadowReceive", free);
}

// ───────── ドロップダウンの項目 ─────────

/// `Popup::Look` の項目。
pub fn entries(app: &AppState, choice: LookChoice) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking() && app.read_only_reason().is_none();
    let look = app.doc.drawn_look();
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
                        e = e.tooltip(lang.pick("描かない（値は持つ）", "Not drawn (the value is kept)"));
                    }
                    e
                })
                .collect()
        }
        LookChoice::Slot(index) => slot_entries(app, index, free),
        LookChoice::Plane(index, k) => plane_entries(app, index, k, free),
    }
}

/// 選べる値のうち、再現が描くもの（描かないものは押せない）。
fn choice_drawn(name: &str, value: usize) -> bool {
    match name {
        "_ShadowColorType" => value == 0,
        "_EmissionMap_UVMode" | "_Emission2ndMap_UVMode" => value == 0 || value == 4,
        "_OutlineVertexR2Width" => value == 0,
        _ => true,
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
            Entry::item(channel_name(lang, &app.doc, c), set(Some(TextureSource::Channel(c))))
                .radio(current == Some(TextureSource::Channel(c)))
                .enabled(free),
        );
    }
    v.push(Entry::Separator);
    let packed = match current {
        Some(TextureSource::Packed(p)) => p,
        Some(TextureSource::Channel(c)) => {
            let scalar = app.doc.channel_info(c).is_some_and(|i| i.kind == ChannelKind::Scalar);
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
            std::array::from_fn(|k| if d[k] > 0.5 { PlaneSource::One } else { PlaneSource::Zero })
        }
    };
    v.push(
        Entry::item(lang.pick("成分ごと", "Per Component"), set(Some(TextureSource::Packed(packed))))
            .radio(matches!(current, Some(TextureSource::Packed(_))))
            .enabled(free),
    );
    if slot.usage == SlotUse::Image {
        let images: Vec<_> = app.shelf.resources().iter().filter(|r| r.kind == "image").collect();
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
    let Some(TextureSource::Packed(planes)) = app.doc.drawn_look().textures.get(slot.name).copied() else {
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
        Entry::item("0", set(PlaneSource::Zero)).radio(current == PlaneSource::Zero).enabled(free),
        Entry::item("1", set(PlaneSource::One)).radio(current == PlaneSource::One).enabled(free),
        Entry::Separator,
    ];
    for c in app.doc.channels() {
        let scalar = app.doc.channel_info(c).is_some_and(|i| i.kind == ChannelKind::Scalar);
        let components: &[u8] = if scalar { &[0] } else { &[0, 1, 2, 3] };
        for &component in components {
            let p = PlaneSource::Channel { channel: c, component };
            let name = channel_name(lang, &app.doc, c);
            let label = if scalar { name } else { format!("{name} {}", ["R", "G", "B", "A"][component as usize]) };
            v.push(Entry::item(label, set(p)).radio(current == p).enabled(free));
        }
    }
    let _ = Channel::Color;
    v
}
