//! ブラシの影響元の下書きが使う、ダミーの項目・入力の入切と最小値と曲線・入り抜きの影響先・ブラシの詳細のウィンドウと影響元設定のウィンドウの枠。
//! 部品は `yolu_app::ui` の物だけ。値は `SketchCtx::values`（と曲線は egui の一時データ）に覚えて、触ると動く。

use egui::{pos2, vec2, Color32, Rect, Ui, Vec2};
use yolu_app::ui::curve;
use yolu_app::ui::menu::{self, Entry as MenuEntry, PopupOutcome, PopupState};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align, NumberFormat, Rows, SliderSpec};
use yolu_app::ui::window::{self, Spec};
use yolu_core::curve::{Curve, CurvePoint};

use crate::gallery::{remember, store};
use crate::page::SketchCtx;

/// 影響元の入力 4 つ（筆圧・傾き・速度・ランダム）。
pub const INPUTS: [(&str, &str); 4] = [
    ("筆圧", "Pressure"),
    ("傾き", "Tilt"),
    ("速度", "Speed"),
    ("ランダム", "Random"),
];

/// 影響元を持つ項目 1 つ（スライダーの範囲と単位）。
#[derive(Clone, Copy)]
pub struct Item {
    pub key: &'static str,
    pub name: (&'static str, &'static str),
    pub default: f32,
    pub lo: f32,
    pub hi: f32,
    pub unit: &'static str,
    /// 最小値が負にもなる（色相・彩度・明度）。
    pub signed: bool,
}

const fn item(
    key: &'static str,
    name: (&'static str, &'static str),
    default: f32,
    lo: f32,
    hi: f32,
    unit: &'static str,
    signed: bool,
) -> Item {
    Item {
        key,
        name,
        default,
        lo,
        hi,
        unit,
        signed,
    }
}

pub const SIZE: Item = item("size", ("直径", "Size"), 24.0, 1.0, 256.0, " px", false);
pub const OPACITY: Item = item(
    "opacity",
    ("不透明度", "Opacity"),
    85.0,
    0.0,
    100.0,
    "%",
    false,
);
pub const FLOW: Item = item("flow", ("流量", "Flow"), 60.0, 0.0, 100.0, "%", false);
pub const HARDNESS: Item = item(
    "hardness",
    ("硬さ", "Hardness"),
    70.0,
    0.0,
    100.0,
    "%",
    false,
);
pub const ROUNDNESS: Item = item(
    "roundness",
    ("真円率", "Roundness"),
    100.0,
    1.0,
    100.0,
    "%",
    false,
);
pub const TEXTURE: Item = item(
    "texture",
    ("質感の濃さ", "Texture depth"),
    60.0,
    0.0,
    100.0,
    "%",
    false,
);
pub const PAINT: Item = item(
    "paint",
    ("絵の具の量", "Paint amount"),
    50.0,
    0.0,
    100.0,
    "%",
    false,
);
pub const DENSITY: Item = item(
    "density",
    ("絵の具の濃さ", "Paint density"),
    50.0,
    0.0,
    100.0,
    "%",
    false,
);
pub const HUE: Item = item("hue", ("色相", "Hue"), 20.0, -180.0, 180.0, "°", true);
pub const SATURATION: Item = item(
    "saturation",
    ("彩度", "Saturation"),
    -10.0,
    -100.0,
    100.0,
    "%",
    true,
);
pub const VALUE: Item = item(
    "value",
    ("明度", "Brightness"),
    10.0,
    -100.0,
    100.0,
    "%",
    true,
);
pub const SUB: Item = item(
    "sub",
    ("サブカラー", "Sub color"),
    0.0,
    0.0,
    100.0,
    "%",
    false,
);

/// 入り抜きを掛けられる項目 8 つ。
pub const TAPER_TARGETS: [Item; 8] = [
    SIZE, OPACITY, FLOW, HARDNESS, ROUNDNESS, TEXTURE, PAINT, DENSITY,
];

pub fn name(ctx: &SketchCtx, item: &Item) -> &'static str {
    ctx.pick(item.name.0, item.name.1)
}

pub fn input_name(ctx: &SketchCtx, i: usize) -> &'static str {
    ctx.pick(INPUTS[i].0, INPUTS[i].1)
}

fn default_on(key: &str, i: usize) -> bool {
    matches!(
        (key, i),
        ("size", 0)
            | ("size", 1)
            | ("opacity", 0)
            | ("flow", 2)
            | ("roundness", 1)
            | ("texture", 0)
            | ("hue", 3)
            | ("saturation", 3)
            | ("value", 3)
    )
}

fn default_min(key: &str, i: usize) -> f32 {
    match (key, i) {
        ("size", 1) => 30.0,
        ("flow", 2) => 20.0,
        ("hue", 3) => -100.0,
        ("saturation", 3) => -60.0,
        ("value", 3) => -40.0,
        _ => 0.0,
    }
}

pub fn on(ctx: &SketchCtx, key: &str, i: usize) -> bool {
    ctx.values
        .bool(&format!("bd.on.{key}.{i}"), default_on(key, i))
}

pub fn set_on(ctx: &mut SketchCtx, key: &str, i: usize, v: bool) {
    ctx.values.set_bool(&format!("bd.on.{key}.{i}"), v);
}

pub fn any_on(ctx: &SketchCtx, key: &str) -> bool {
    (0..4).any(|i| on(ctx, key, i))
}

pub fn min(ctx: &SketchCtx, key: &str, i: usize) -> f32 {
    ctx.values
        .float(&format!("bd.min.{key}.{i}"), default_min(key, i))
}

fn default_curve(key: &str, i: usize) -> Curve {
    let bent = |mid: f64| {
        Curve::new(vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.5, y: mid },
            CurvePoint { x: 1.0, y: 1.0 },
        ])
        .unwrap_or_else(|_| Curve::identity())
    };
    match (key, i) {
        ("size", 0) => bent(0.3),
        ("size", 1) => bent(0.65),
        _ => Curve::identity(),
    }
}

/// 影響元を持つ項目の全部（影響元設定のウィンドウが名前で引く）。
pub const ALL_ITEMS: [Item; 12] = [
    SIZE, OPACITY, FLOW, HARDNESS, ROUNDNESS, TEXTURE, PAINT, DENSITY, HUE, SATURATION, VALUE, SUB,
];

// ───────── 項目（ブラシの詳細の行とツールプロパティの行の両方が同じ物を描く）─────────

/// 項目の操作の種類。
#[derive(Clone, Copy)]
pub enum Control {
    /// 影響元を持つスライダー。
    Influence(Item),
    Slider {
        lo: f32,
        hi: f32,
        unit: &'static str,
        default: f32,
    },
    Toggle {
        default: bool,
    },
    Choice {
        options: &'static [(&'static str, &'static str)],
        default: usize,
    },
}

/// ブラシの設定の項目 1 つ。`shown` は、ツールプロパティに出す（目のボタンが押し込まれている）既定。
#[derive(Clone, Copy)]
pub struct Field {
    pub key: &'static str,
    pub name: (&'static str, &'static str),
    pub control: Control,
    pub shown: bool,
}

pub const fn influence(item: Item, shown: bool) -> Field {
    Field {
        key: item.key,
        name: item.name,
        control: Control::Influence(item),
        shown,
    }
}

pub const fn slider_field(
    key: &'static str,
    name: (&'static str, &'static str),
    range: (f32, f32),
    unit: &'static str,
    default: f32,
    shown: bool,
) -> Field {
    Field {
        key,
        name,
        control: Control::Slider {
            lo: range.0,
            hi: range.1,
            unit,
            default,
        },
        shown,
    }
}

pub const fn toggle_field(
    key: &'static str,
    name: (&'static str, &'static str),
    default: bool,
    shown: bool,
) -> Field {
    Field {
        key,
        name,
        control: Control::Toggle { default },
        shown,
    }
}

pub const fn choice_field(
    key: &'static str,
    name: (&'static str, &'static str),
    options: &'static [(&'static str, &'static str)],
    default: usize,
    shown: bool,
) -> Field {
    Field {
        key,
        name,
        control: Control::Choice { options, default },
        shown,
    }
}

/// ツールプロパティに出すか（目のボタン）。
pub fn is_shown(ctx: &SketchCtx, f: &Field) -> bool {
    ctx.values.bool(&format!("bd.eye.{}", f.key), f.shown)
}

fn field_name(ctx: &SketchCtx, f: &Field) -> &'static str {
    ctx.pick(f.name.0, f.name.1)
}

fn slider_value(ctx: &SketchCtx, key: &str, default: f32) -> f32 {
    ctx.values.float(&format!("bd.value.{key}"), default)
}

/// スライダー項目の値の箱。`label` は溝の中の名前（ブラシの詳細の行は名前を溝の外に出すので空）。
fn field_slider(ui: &mut Ui, ctx: &mut SketchCtx, r: Rect, f: &Field, label: &str) {
    let (lo, hi, unit, default) = match f.control {
        Control::Influence(i) => (i.lo, i.hi, i.unit, i.default),
        Control::Slider {
            lo,
            hi,
            unit,
            default,
        } => (lo, hi, unit, default),
        _ => return,
    };
    let spec = SliderSpec::new(label, lo, hi, NumberFormat::int(unit));
    let out = w::slider(
        ui,
        r,
        ("bd.slider", f.key),
        slider_value(ctx, f.key, default),
        &spec,
    );
    if out.changed {
        ctx.values
            .set_float(&format!("bd.value.{}", f.key), out.value);
    }
}

fn toggle_value(ctx: &SketchCtx, f: &Field) -> bool {
    match f.control {
        Control::Toggle { default } => ctx.values.bool(&format!("bd.toggle.{}", f.key), default),
        _ => false,
    }
}

fn field_toggle(ui: &mut Ui, ctx: &mut SketchCtx, r: Rect, f: &Field) {
    let now = toggle_value(ctx, f);
    let next = w::toggle(
        ui,
        r,
        ("bd.toggle", f.key),
        field_name(ctx, f),
        now,
        None,
        true,
    );
    ctx.values.set_bool(&format!("bd.toggle.{}", f.key), next);
}

/// 選びの箱（押すとメニュー。撮る間は `bd.menu` に名前を置いた箱だけ開いた形）。
fn field_choice(ui: &mut Ui, ctx: &mut SketchCtx, r: Rect, f: &Field, label: Option<&str>) {
    let Control::Choice { options, default } = f.control else {
        return;
    };
    let key = format!("bd.choice.{}", f.key);
    let current = ctx.values.index(&key, default);
    let (response, anchor) = w::dropdown(
        ui,
        r,
        ("bd.choice", f.key),
        label,
        ctx.pick(options[current].0, options[current].1),
        None,
        true,
        108.0,
    );
    let names: Vec<&'static str> = options.iter().map(|(ja, en)| ctx.pick(ja, en)).collect();
    let id = ui.id().with(("bd.popup", f.key));
    let once = format!("bd.opened-once.{}", f.key);
    let first = !remember(ui, &once, || false);
    store(ui, &once, true);
    let forced = ctx.still && ctx.values.text("bd.menu", "") == f.key && first;
    let mut state: Option<PopupState> = ui.data(|d| d.get_temp(id));
    if response.clicked() || forced {
        state = Some(PopupState::new(ui.ctx(), anchor).with_min_width(anchor.width()));
    }
    let Some(mut open) = state else {
        return;
    };
    let entries: Vec<MenuEntry<usize>> = names
        .iter()
        .enumerate()
        .map(|(i, n)| MenuEntry::item(*n, i).radio(i == current))
        .collect();
    match menu::show(ui.ctx(), id, &mut open, &entries, &[anchor]) {
        PopupOutcome::Chosen(i) => {
            ctx.values.set_index(&key, i);
            ui.data_mut(|d| {
                d.remove::<PopupState>(id);
            });
        }
        PopupOutcome::Open => ui.data_mut(|d| {
            d.insert_temp(id, open);
        }),
        _ if ctx.still => ui.data_mut(|d| {
            d.insert_temp(id, open);
        }),
        _ => ui.data_mut(|d| {
            d.remove::<PopupState>(id);
        }),
    }
}

/// 影響元のボタンの大きさ。
const DYNAMICS_BUTTON: f32 = 24.0;

/// 影響元のボタン（今のツールプロパティの筆圧のペンのボタンと同じアイコン。どれかの入力を使っていれば押し込んだ形、設定のウィンドウが開いていれば青い縁）。
/// ブラシの詳細では押すと「◯◯影響元設定」のウィンドウを開く。押したら true。
fn dynamics_button(ui: &mut Ui, ctx: &SketchCtx, r: Rect, key: &str) -> bool {
    let response = w::icon_button(
        ui,
        r,
        ("bd.dynamics", key),
        "stylus",
        ctx.pick("影響元", "Dynamics"),
        any_on(ctx, key),
        true,
        15.0,
    );
    if ctx.values.text("bd.infl", "") == key {
        w::outline(ui.painter(), r.expand(1.0), t::ACCENT, 1.5, 5.0);
    }
    response.clicked()
}

/// 開いている影響元設定のウィンドウの項目を替える（同じ項目をもう一度押すと閉じる）。
fn toggle_influence_window(ctx: &mut SketchCtx, key: &str) {
    let next = if ctx.values.text("bd.infl", "") == key {
        ""
    } else {
        key
    };
    ctx.values.set_text("bd.infl", next);
}

/// ブラシの詳細の行の高さ。
pub const ROW: f32 = 24.0;
const NAME_WIDTH: f32 = 116.0;

/// ブラシの詳細の項目の行: 左端に目のボタン（ツールプロパティに出すか）、名前、操作、右に影響元のボタン。
pub fn detail_row(ui: &mut Ui, ctx: &mut SketchCtx, r: Rect, f: &Field) {
    let eye = Rect::from_min_size(r.min, vec2(24.0, r.height()));
    let shown = is_shown(ctx, f);
    if w::icon_button(
        ui,
        eye,
        ("bd.eye", f.key),
        if shown {
            "visibility"
        } else {
            "visibility_off"
        },
        ctx.pick("ツールプロパティに表示", "Show in Tool Properties"),
        shown,
        true,
        16.0,
    )
    .clicked()
    {
        ctx.values.set_bool(&format!("bd.eye.{}", f.key), !shown);
    }
    let x = eye.right() + 8.0;
    let body = Rect::from_min_max(pos2(x, r.top()), r.max);
    let name_rect = Rect::from_min_size(body.min, vec2(NAME_WIDTH, r.height()));
    let control = Rect::from_min_max(
        pos2(name_rect.right() + 4.0, r.top()),
        pos2(r.right() - DYNAMICS_BUTTON - 6.0, r.bottom()),
    );
    let label = |ui: &Ui| {
        let shown = w::fit(
            ui.painter(),
            field_name(ctx, f),
            name_rect.width(),
            t::LABEL,
        );
        w::text(ui.painter(), name_rect, &shown, t::LABEL, Align::Left);
    };
    match f.control {
        Control::Influence(_) => {
            label(ui);
            field_slider(ui, ctx, control.shrink2(vec2(0.0, 1.0)), f, "");
            let button = Rect::from_min_size(
                pos2(r.right() - DYNAMICS_BUTTON, r.top()),
                vec2(DYNAMICS_BUTTON, r.height()),
            );
            if dynamics_button(ui, ctx, button, f.key) {
                toggle_influence_window(ctx, f.key);
            }
        }
        Control::Slider { .. } => {
            label(ui);
            field_slider(ui, ctx, control.shrink2(vec2(0.0, 1.0)), f, "");
        }
        Control::Toggle { .. } => field_toggle(ui, ctx, body, f),
        Control::Choice { .. } => {
            label(ui);
            field_choice(ui, ctx, control, f, None);
        }
    }
}

/// ツールプロパティの項目の行（狭い欄の形: 名前は溝や箱の中。影響元のボタンは右）。
pub fn tool_row(ui: &mut Ui, ctx: &mut SketchCtx, rows: &mut Rows, f: &Field) {
    let name = field_name(ctx, f);
    match f.control {
        Control::Influence(_) => {
            let r = rows.slider_row();
            let track =
                Rect::from_min_max(r.min, pos2(r.right() - DYNAMICS_BUTTON - 3.0, r.bottom()));
            field_slider(ui, ctx, track, f, name);
            let button = Rect::from_min_size(
                pos2(r.right() - DYNAMICS_BUTTON, r.top()),
                vec2(DYNAMICS_BUTTON, r.height()),
            );
            let _ = w::icon_button(
                ui,
                button,
                ("bd.tool-dynamics", f.key),
                "stylus",
                ctx.pick("影響元", "Dynamics"),
                any_on(ctx, f.key),
                true,
                15.0,
            );
        }
        Control::Slider { .. } => {
            let r = rows.slider_row();
            field_slider(ui, ctx, r, f, name);
        }
        Control::Toggle { .. } => {
            let r = rows.row(t::ROW_HEIGHT, 2.0);
            field_toggle(ui, ctx, r, f);
        }
        Control::Choice { .. } => {
            let r = rows.row(24.0, 4.0);
            field_choice(ui, ctx, r, f, Some(name));
        }
    }
}

// ───────── 影響元設定のウィンドウ ─────────

/// 曲線 1 つ（入力 `i` の応え）。使っていない入力は押せない。
fn curve_cell(ui: &mut Ui, ctx: &SketchCtx, r: Rect, item: &Item, i: usize) {
    let key = item.key;
    let id = format!("bd.curve.{key}.{i}");
    let now = remember(ui, &id, || default_curve(key, i));
    let tip = format!("{}{}", input_name(ctx, i), ctx.pick("の曲線", " curve"));
    if let Some(next) =
        curve::curve_editor(ui, r, ("bd.curve", key, i), &now, &tip, on(ctx, key, i))
    {
        store(ui, &id, next);
    }
}

/// 「◯◯影響元設定」のウィンドウ（`bd.infl` に項目の名前が入っているとき）。上に入力ごとの行（使うか・最小値・傾きは最大値も）、
/// 下に筆圧・傾き・速度の曲線を横に 3 つ。`default_offset` は画面の真ん中からの位置。
pub fn influence_window(ui: &mut Ui, ctx: &mut SketchCtx, default_offset: Vec2) {
    let open = ctx.values.text("bd.infl", "");
    let Some(item) = ALL_ITEMS.iter().find(|i| i.key == open).copied() else {
        return;
    };
    let lang = ctx.lang;
    let id = ui.id().with("bd.influence-window");
    let mut offset: Vec2 = remember(ui, "bd.infl.offset", || default_offset);
    let title = format!(
        "{}{}",
        name(ctx, &item),
        lang.pick("影響元設定", " dynamics")
    );
    let spec = Spec {
        title: &title,
        icon: None,
        size: vec2(480.0, 350.0),
        modal: false,
        close_label: lang.pick("閉じる", "Close"),
    };
    let closed = window::show(ui.ctx(), id, &spec, &mut offset, false, |wui, frame| {
        let area = frame.body.shrink(10.0);
        let mut y = area.top();
        let heading = format!(
            "{}{}",
            name(ctx, &item),
            lang.pick("に影響させる入力", ": inputs")
        );
        w::text(
            wui.painter(),
            Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), 18.0)),
            &heading,
            t::LABEL_DIM,
            Align::Left,
        );
        y += 24.0;
        let (lo, hi) = if item.signed {
            (-100.0, 100.0)
        } else {
            (0.0, 100.0)
        };
        for i in 0..4 {
            let row = Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), 22.0));
            let left = Rect::from_min_size(row.min, vec2(104.0, 22.0));
            let now = on(ctx, item.key, i);
            let next = w::toggle(
                wui,
                left,
                ("bd.inf.on", item.key, i),
                input_name(ctx, i),
                now,
                None,
                true,
            );
            if next != now {
                set_on(ctx, item.key, i, next);
            }
            let track = Rect::from_min_max(pos2(left.right() + 6.0, y), row.max);
            minmax_slider(
                wui,
                ctx,
                track,
                &format!("bd.min.{}.{i}", item.key),
                ctx.pick("最小値", "Minimum"),
                min(ctx, item.key, i),
                (lo, hi),
                next,
            );
            y += 26.0;
            if i == 1 {
                // 傾きだけ最大値もある
                let track =
                    Rect::from_min_max(pos2(left.right() + 6.0, y), pos2(area.right(), y + 22.0));
                minmax_slider(
                    wui,
                    ctx,
                    track,
                    &format!("bd.max.{}.{i}", item.key),
                    ctx.pick("最大値", "Maximum"),
                    ctx.values.float(&format!("bd.max.{}.{i}", item.key), 100.0),
                    (0.0, 100.0),
                    next,
                );
                y += 26.0;
            }
        }
        y += 6.0;
        let gap = 10.0;
        let column = (area.width() - 2.0 * gap) / 3.0;
        for (n, caption) in [
            lang.pick("筆圧設定", "Pressure curve"),
            lang.pick("傾き設定", "Tilt curve"),
            lang.pick("速度設定", "Speed curve"),
        ]
        .iter()
        .enumerate()
        {
            let x = area.left() + n as f32 * (column + gap);
            w::text(
                wui.painter(),
                Rect::from_min_size(pos2(x, y), vec2(column, 18.0)),
                caption,
                t::LABEL_DIM,
                Align::Center,
            );
            let h = (area.bottom() - y - 20.0).min(curve::HEIGHT);
            curve_cell(
                wui,
                ctx,
                Rect::from_min_size(pos2(x, y + 20.0), vec2(column, h)),
                &item,
                n,
            );
        }
    });
    store(ui, "bd.infl.offset", offset);
    if closed && !ctx.still {
        ctx.values.set_text("bd.infl", "");
    }
}

#[allow(clippy::too_many_arguments)]
fn minmax_slider(
    ui: &mut Ui,
    ctx: &mut SketchCtx,
    r: Rect,
    key: &str,
    label: &str,
    value: f32,
    range: (f32, f32),
    enabled: bool,
) {
    let spec = SliderSpec::new(label, range.0, range.1, NumberFormat::int("%")).enabled(enabled);
    let out = w::slider(ui, r, ("bd.minmax", key), value, &spec);
    if out.changed {
        ctx.values.set_float(key, out.value);
    }
}

// ───────── ブラシの詳細のウィンドウの枠 ─────────

/// 左の区分の一覧の 1 つ（`indent` は字下げした区分）。
pub struct Category {
    pub name: (&'static str, &'static str),
    pub indent: bool,
}

const SIDEBAR: f32 = 168.0;
const CATEGORY_HEIGHT: f32 = 22.0;
const TITLE_HEIGHT: f32 = 26.0;
const SAMPLE_HEIGHT: f32 = 50.0;

/// ブラシの詳細のウィンドウ（見出しの帯・左に区分の一覧・右の上に見本・区分の名前と既定に戻す・欄）。`body` は欄の矩形を受けて描く。
/// 見本のストロークは本物の描画が要るので、名前だけの灰色の四角。撮る間は画面の真ん中に置く。
pub fn detail_window(
    ui: &mut Ui,
    ctx: &mut SketchCtx,
    size: Vec2,
    cats: &[Category],
    active: usize,
    body: impl FnOnce(&mut Ui, &mut SketchCtx, Rect),
) {
    let lang = ctx.lang;
    let id = ui.id().with("brushdyn.window");
    let mut offset: Vec2 = if ctx.still {
        Vec2::ZERO
    } else {
        remember(ui, "brushdyn.offset", || Vec2::ZERO)
    };
    let spec = Spec {
        title: lang.pick("ブラシの詳細 — ペン", "Brush details — Pen"),
        icon: Some("tune"),
        size,
        modal: false,
        close_label: lang.pick("閉じる", "Close"),
    };
    let closed = window::show(ui.ctx(), id, &spec, &mut offset, false, |wui, frame| {
        let side = Rect::from_min_size(frame.body.min, vec2(SIDEBAR, frame.body.height()));
        let pane = Rect::from_min_max(pos2(side.right(), frame.body.top()), frame.body.max);
        sidebar(wui, ctx, side, cats, active);
        let sample = Rect::from_min_size(
            pos2(pane.left() + 10.0, pane.top() + 8.0),
            vec2(pane.width() - 20.0, SAMPLE_HEIGHT),
        );
        w::rounded(wui.painter(), sample, t::CONTROL_BG, 4.0);
        w::outline(wui.painter(), sample, t::BORDER, 1.0, 4.0);
        w::text(
            wui.painter(),
            sample,
            lang.pick("見本", "Preview"),
            t::LABEL_DIM,
            Align::Center,
        );
        let title = Rect::from_min_size(
            pos2(pane.left() + 10.0, sample.bottom() + 6.0),
            vec2(pane.width() - 20.0, TITLE_HEIGHT),
        );
        let cat = &cats[active];
        w::text(
            wui.painter(),
            Rect::from_min_max(title.min, pos2(title.right() - 30.0, title.bottom())),
            lang.pick(cat.name.0, cat.name.1),
            t::HEADER,
            Align::Left,
        );
        let _ = w::icon_button(
            wui,
            Rect::from_min_size(
                pos2(title.right() - 26.0, title.top() + 1.0),
                vec2(26.0, 24.0),
            ),
            "brushdyn.reset",
            "restart_alt",
            lang.pick("既定に戻す", "Reset to defaults"),
            false,
            true,
            15.0,
        );
        w::hline(
            wui.painter(),
            pane.left(),
            pane.right(),
            title.bottom() + 3.0,
            t::BORDER,
        );
        let area = Rect::from_min_max(pos2(pane.left(), title.bottom() + 8.0), pane.max);
        body(wui, ctx, area);
    });
    if !ctx.still {
        store(ui, "brushdyn.offset", offset);
        let _ = closed;
    }
}

fn sidebar(ui: &mut Ui, ctx: &SketchCtx, pane: Rect, cats: &[Category], active: usize) {
    w::fill(ui.painter(), pane, t::PANEL_HEADER);
    w::vline(
        ui.painter(),
        pane.right() - 1.0,
        pane.top(),
        pane.bottom(),
        t::BORDER,
    );
    for (i, cat) in cats.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(pane.left(), pane.top() + 6.0 + i as f32 * CATEGORY_HEIGHT),
            vec2(pane.width() - 1.0, CATEGORY_HEIGHT),
        );
        let response = ui.interact(
            row,
            ui.make_persistent_id(("brushdyn.category", i)),
            egui::Sense::click(),
        );
        let on = i == active;
        let p = ui.painter();
        if on {
            w::fill(p, row, t::ACCENT_SOFT);
            w::fill(
                p,
                Rect::from_min_size(row.min, vec2(3.0, row.height())),
                t::ACCENT,
            );
        } else if response.hovered() {
            w::fill(p, row, t::CONTROL_HOVER);
        }
        let x = row.left() + 12.0 + if cat.indent { 14.0 } else { 0.0 };
        let label = ctx.pick(cat.name.0, cat.name.1);
        let shown = w::fit(p, label, row.right() - x - 6.0, t::LABEL);
        w::text(
            p,
            Rect::from_min_max(pos2(x, row.top()), row.max),
            &shown,
            t::LABEL.with_color(if on { Color32::WHITE } else { t::TEXT }),
            Align::Left,
        );
    }
}

/// 入り抜きの影響先の欄（項目ごとに、掛けるかと最小値）。高さは [`taper_height`]。
pub const TAPER_ROW: f32 = 24.0;

pub fn taper_height() -> f32 {
    TAPER_TARGETS.len() as f32 * TAPER_ROW
}

pub fn taper_on(ctx: &SketchCtx, key: &str) -> bool {
    ctx.values
        .bool(&format!("bd.taper.{key}"), matches!(key, "size" | "flow"))
}

pub fn taper_min(ctx: &SketchCtx, key: &str) -> f32 {
    ctx.values.float(
        &format!("bd.taper.min.{key}"),
        if key == "flow" { 25.0 } else { 0.0 },
    )
}

/// 入り抜きの影響先を縦に並べる（左にチェックと名前、右に最小値）。
pub fn taper_rows(ui: &mut Ui, ctx: &mut SketchCtx, r: Rect) {
    for (n, target) in TAPER_TARGETS.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(r.left(), r.top() + n as f32 * TAPER_ROW),
            vec2(r.width(), TAPER_ROW),
        );
        let left = Rect::from_min_size(row.min, vec2(row.width() * 0.46, row.height()));
        let right = Rect::from_min_max(
            pos2(left.right() + 6.0, row.top() + 2.0),
            pos2(row.right(), row.bottom() - 2.0),
        );
        let is_on = taper_on(ctx, target.key);
        let next = w::toggle(
            ui,
            left,
            ("bd.taper", target.key),
            name(ctx, target),
            is_on,
            None,
            true,
        );
        if next != is_on {
            ctx.values
                .set_bool(&format!("bd.taper.{}", target.key), next);
        }
        let key = format!("bd.taper.min.{}", target.key);
        let out = w::slider(
            ui,
            right,
            ("bd.taper.min", target.key),
            taper_min(ctx, target.key),
            &SliderSpec::new(
                ctx.pick("最小値", "Minimum"),
                0.0,
                100.0,
                NumberFormat::int("%"),
            )
            .enabled(next),
        );
        if out.changed {
            ctx.values.set_float(&key, out.value);
        }
    }
}

/// 繰り返し方法（6 種）の名前。
pub const REPEAT_METHODS: [(&str, &str); 6] = [
    ("繰り返し", "Repeat"),
    ("折り返し", "Ping-pong"),
    ("繰り返さない", "Hold last"),
    ("ランダム", "Random"),
    ("一巡のみ", "Once"),
    ("ランダムに一巡", "Shuffle once"),
];

/// 反転の決まり（4 種）の名前。
pub const FLIP_RULES: [(&str, &str); 4] = [
    ("なし", "None"),
    ("反転", "Flip"),
    ("ランダム", "Random"),
    ("折り返し時に反転", "Flip on fold"),
];

/// 変化対象（色の変化を当てる描画色）の名前。
pub const COLOR_TARGETS: [(&str, &str); 3] = [
    ("メイン描画色", "Main color"),
    ("メインとサブ描画色", "Main and sub colors"),
    ("サブ描画色", "Sub color"),
];
