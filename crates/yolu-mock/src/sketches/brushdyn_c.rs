//! ブラシの影響元の画面（案 C）: ブラシの詳細のウィンドウを、サブツール詳細の並びの区分（ブラシサイズ・インク・色の変化・アンチエイリアス・
//! ブラシ形状〔ブラシ先端・散布効果・ストローク・紙質〕・2-ブラシ形状〔同じく〕・水彩境界・消去・補正・入り抜き・はみ出し防止）にする。
//! 右の項目の行は、左端に目のボタン（ツールプロパティに出すか。出す物は押し込んだ形）、名前、スライダー、影響元を持つ項目は右に影響元のボタン。
//! 影響元のボタンを押すと「◯◯影響元設定」の小さなウィンドウが開く（入力ごとの使う・最小値・傾きは最大値、下に筆圧・傾き・速度の曲線）。
//! エンジンに無い区分（アンチエイリアス・散布効果・水彩境界・消去・はみ出し防止など）は名前だけで、中身は今ある項目だけ。
//! 絵: `brushdyn-c`（ブラシサイズ）・`brushdyn-c-color`（色の変化。影響元設定を開いた形）・`brushdyn-c-tip`（ブラシ先端。反転の選びを開いた形）・
//! `brushdyn-c-stroke`（ストローク）・`brushdyn-c-paper`（紙質）・`brushdyn-c-taper`（入り抜き）・`brushdyn-c-toolprops`（目のボタンで出した項目だけのツールプロパティ）。

use egui::{pos2, vec2, Rect, Ui, Vec2};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align, Rows};

use super::brushdyn_common::{self as c, Category, Field};
use crate::page::SketchCtx;

const fn cat(name: (&'static str, &'static str), indent: bool) -> Category {
    Category { name, indent }
}

const CATEGORIES: [Category; 19] = [
    cat(("ブラシサイズ", "Brush size"), false),
    cat(("インク", "Ink"), false),
    cat(("色の変化", "Color change"), false),
    cat(("アンチエイリアス", "Anti-aliasing"), false),
    cat(("ブラシ形状", "Brush shape"), false),
    cat(("ブラシ先端", "Brush tip"), true),
    cat(("散布効果", "Spray"), true),
    cat(("ストローク", "Stroke"), true),
    cat(("紙質", "Paper"), true),
    cat(("2-ブラシ形状", "2nd brush shape"), false),
    cat(("2-ブラシ先端", "2nd brush tip"), true),
    cat(("2-散布効果", "2nd spray"), true),
    cat(("2-ストローク", "2nd stroke"), true),
    cat(("2-紙質", "2nd paper"), true),
    cat(("水彩境界", "Watercolor edge"), false),
    cat(("消去", "Erase"), false),
    cat(("補正", "Stabilization"), false),
    cat(("入り抜き", "Taper"), false),
    cat(("はみ出し防止", "Overflow guard"), false),
];

const BRUSH_SIZE: usize = 0;
const INK: usize = 1;
const COLOR_CHANGE: usize = 2;
const TIP: usize = 5;
const STROKE: usize = 7;
const PAPER: usize = 8;
const STABILIZATION: usize = 16;
const TAPER: usize = 17;

const SIZE_FIELDS: [Field; 3] = [
    c::influence(c::SIZE, true),
    c::toggle_field(
        "screen-size",
        ("画面上のサイズで指定", "Specify by size on screen"),
        false,
        true,
    ),
    c::toggle_field(
        "min-one",
        ("最低 1 ピクセル", "At least 1 pixel"),
        false,
        false,
    ),
];

const INK_FIELDS: [Field; 4] = [
    c::influence(c::OPACITY, true),
    c::influence(c::FLOW, true),
    c::influence(c::PAINT, false),
    c::influence(c::DENSITY, false),
];

const COLOR_FIELDS: [Field; 11] = [
    c::toggle_field(
        "color-on",
        ("ブラシ先端色の変化", "Change the tip color"),
        true,
        true,
    ),
    c::influence(c::HUE, true),
    c::influence(c::SATURATION, false),
    c::influence(c::VALUE, false),
    c::influence(c::SUB, false),
    c::toggle_field(
        "stroke-on",
        ("ストロークごとのランダム変化", "Randomize per stroke"),
        false,
        false,
    ),
    c::slider_field(
        "stroke-hue",
        ("色相", "Hue"),
        (0.0, 180.0),
        "°",
        103.0,
        false,
    ),
    c::slider_field(
        "stroke-saturation",
        ("彩度", "Saturation"),
        (0.0, 100.0),
        "%",
        22.0,
        false,
    ),
    c::slider_field(
        "stroke-value",
        ("明度", "Brightness"),
        (0.0, 100.0),
        "%",
        64.0,
        false,
    ),
    c::slider_field(
        "stroke-sub",
        ("サブカラー", "Sub color"),
        (0.0, 100.0),
        "%",
        21.0,
        false,
    ),
    c::choice_field(
        "color-target",
        ("変化対象", "Target"),
        &c::COLOR_TARGETS,
        0,
        false,
    ),
];

const TIP_FIELDS: [Field; 6] = [
    c::influence(c::HARDNESS, true),
    c::influence(c::ROUNDNESS, false),
    c::slider_field(
        "tip-angle",
        ("角度", "Angle"),
        (-180.0, 180.0),
        "°",
        0.0,
        false,
    ),
    c::choice_field(
        "repeat",
        ("繰り返し方法", "Repeat method"),
        &c::REPEAT_METHODS,
        0,
        false,
    ),
    c::choice_field(
        "flip-h",
        ("左右反転", "Flip horizontal"),
        &c::FLIP_RULES,
        2,
        false,
    ),
    c::choice_field(
        "flip-v",
        ("上下反転", "Flip vertical"),
        &c::FLIP_RULES,
        0,
        false,
    ),
];

const STROKE_FIELDS: [Field; 2] = [
    c::slider_field(
        "spacing",
        ("間隔", "Spacing"),
        (1.0, 200.0),
        "%",
        15.0,
        false,
    ),
    c::toggle_field(
        "darken",
        ("ブラシ先端同士を比較暗で合成", "Blend tips with Darken"),
        false,
        false,
    ),
];

const PAPER_FIELDS: [Field; 4] = [
    c::influence(c::TEXTURE, false),
    c::slider_field(
        "paper-scale",
        ("拡大率", "Scale"),
        (5.0, 6400.0),
        "%",
        100.0,
        false,
    ),
    c::slider_field(
        "paper-angle",
        ("回転", "Rotation"),
        (-180.0, 180.0),
        "°",
        35.0,
        false,
    ),
    c::toggle_field("paper-invert", ("濃度反転", "Invert"), false, false),
];

const STABILIZATION_FIELDS: [Field; 1] = [c::slider_field(
    "stabilizer",
    ("手ぶれ補正", "Stabilizer"),
    (0.0, 100.0),
    " px",
    0.0,
    true,
)];

const TAPER_FIELDS: [Field; 3] = [
    c::slider_field(
        "taper-in",
        ("入りの長さ", "Start length"),
        (0.0, 500.0),
        " px",
        40.0,
        false,
    ),
    c::slider_field(
        "taper-out",
        ("抜きの長さ", "End length"),
        (0.0, 500.0),
        " px",
        60.0,
        false,
    ),
    c::toggle_field(
        "taper-speed",
        ("速度による入り抜き", "Taper by speed"),
        false,
        false,
    ),
];

/// 区分の項目（エンジンに無い区分は空）。
fn fields(index: usize) -> &'static [Field] {
    match index {
        BRUSH_SIZE => &SIZE_FIELDS,
        INK => &INK_FIELDS,
        COLOR_CHANGE => &COLOR_FIELDS,
        TIP => &TIP_FIELDS,
        STROKE => &STROKE_FIELDS,
        PAPER => &PAPER_FIELDS,
        STABILIZATION => &STABILIZATION_FIELDS,
        TAPER => &TAPER_FIELDS,
        _ => &[],
    }
}

/// ウィンドウの大きさ（今のブラシの詳細と同じ）。
const SIZE: Vec2 = vec2(640.0, 540.0);
const GAP: f32 = 4.0;

/// 区分の項目の行を縦に並べて、下の端の y を返す。
fn rows(ui: &mut Ui, ctx: &mut SketchCtx, area: Rect, index: usize) -> f32 {
    let mut y = area.top();
    for f in fields(index) {
        let r = Rect::from_min_size(
            pos2(area.left() + 8.0, y),
            vec2(area.width() - 16.0, c::ROW),
        );
        c::detail_row(ui, ctx, r, f);
        y += c::ROW + GAP;
    }
    y
}

/// ブラシの詳細（`index` の区分）と、開いていれば影響元設定のウィンドウ。
fn window(ui: &mut Ui, ctx: &mut SketchCtx, index: usize) {
    c::detail_window(ui, ctx, SIZE, &CATEGORIES, index, |ui, ctx, area| {
        let y = rows(ui, ctx, area, index);
        if index == TAPER {
            // 入り抜きの影響先
            let label = Rect::from_min_size(pos2(area.left() + 8.0, y + 6.0), vec2(200.0, 16.0));
            w::text(
                ui.painter(),
                label,
                ctx.pick("影響先", "Applies to"),
                t::LABEL_DIM,
                Align::Left,
            );
            let list = Rect::from_min_size(
                pos2(area.left() + 8.0, label.bottom() + 4.0),
                vec2(area.width() - 16.0, c::taper_height() + 16.0),
            );
            w::rounded(ui.painter(), list, t::PANEL_HEADER, 5.0);
            w::outline(ui.painter(), list, t::BORDER, 1.0, 5.0);
            c::taper_rows(ui, ctx, list.shrink(8.0));
        }
    });
    c::influence_window(ui, ctx, vec2(150.0, 110.0));
}

/// 撮る間は、見せる物（影響元設定のウィンドウ・開いた選び）を置いて始める。
fn show(ctx: &mut SketchCtx, influence: &str, menu: &str) {
    if ctx.still {
        ctx.values.set_text("bd.infl", influence);
        ctx.values.set_text("bd.menu", menu);
    }
}

/// ブラシサイズ（直径・画面上のサイズで指定・最低 1 ピクセル）。
pub fn draw(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(ctx, "", "");
    window(ui, ctx, BRUSH_SIZE);
}

/// 色の変化。色相の影響元設定のウィンドウを開いた形。
pub fn draw_color(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(ctx, "hue", "");
    window(ui, ctx, COLOR_CHANGE);
}

/// ブラシ先端。左右反転の選び（4 種）を開いた形。
pub fn draw_tip(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(ctx, "", "flip-h");
    window(ui, ctx, TIP);
}

/// ストローク（間隔・比較暗）。
pub fn draw_stroke(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(ctx, "", "");
    window(ui, ctx, STROKE);
}

/// 紙質（質感の濃さ・拡大率・回転・濃度反転）。
pub fn draw_paper(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(ctx, "", "");
    window(ui, ctx, PAPER);
}

/// 入り抜き（長さと影響先）。
pub fn draw_taper(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(ctx, "", "");
    window(ui, ctx, TAPER);
}

/// ツールプロパティ: 目のボタンで出した項目だけが、区分の順に並ぶ。
pub fn draw_toolprops(ui: &mut Ui, ctx: &mut SketchCtx) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let mut rows = Rows::compact(r, 2.0);
    for index in 0..CATEGORIES.len() {
        for f in fields(index) {
            if c::is_shown(ctx, f) {
                c::tool_row(ui, ctx, &mut rows, f);
            }
        }
    }
}
