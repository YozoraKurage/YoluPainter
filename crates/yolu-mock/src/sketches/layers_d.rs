//! レイヤーの欄（案 D）: 上に細い合成方法と不透明度の帯、そのすぐ下に操作の帯、その下に行の一覧。
//! 操作の帯は、左端にクリッピング、右へ向かってレイヤーの追加・塗りつぶし・フォルダーの追加・調整レイヤー・削除。

use egui::{pos2, vec2, Rect, Ui};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, NumberFormat, SliderSpec};

use egui::Color32;

use super::layers_common::{self as common, DummyRow, Kind, ROW_HEIGHT};
use crate::page::SketchCtx;

/// クリッピングの印の色（薄い赤。テーマに合う色が無いので、この下書きの定数）と幅。
const CLIP_BAR: Color32 = Color32::from_rgb(0xF0, 0x8A, 0x8A);
const CLIP_BAR_WIDTH: f32 = 2.5;

const fn dummy(
    name: (&'static str, &'static str),
    kind: Kind,
    depth: usize,
    mask: bool,
    locked: bool,
    blend: (&'static str, &'static str),
    opacity: f32,
) -> DummyRow {
    DummyRow {
        name,
        kind,
        depth,
        mask,
        locked,
        blend,
        opacity,
    }
}

const NORMAL: (&str, &str) = ("通常", "Normal");

/// 行（下のレイヤーでクリッピングしている行は、`CLIPPED` の添え字）。
const ROWS: &[DummyRow] = &[
    dummy(
        ("レイヤー 5", "Layer 5"),
        Kind::Raster(Color32::from_rgb(222, 150, 80)),
        0,
        false,
        false,
        ("乗算", "Multiply"),
        80.0,
    ),
    dummy(
        ("レイヤー 4", "Layer 4"),
        Kind::Raster(Color32::from_rgb(200, 90, 130)),
        0,
        false,
        false,
        NORMAL,
        100.0,
    ),
    dummy(
        ("レイヤー 3", "Layer 3"),
        Kind::Raster(Color32::from_rgb(96, 170, 120)),
        0,
        true,
        false,
        NORMAL,
        100.0,
    ),
    dummy(
        ("グループ 1", "Group 1"),
        Kind::Group,
        0,
        false,
        false,
        NORMAL,
        100.0,
    ),
    dummy(
        ("レイヤー 2", "Layer 2"),
        Kind::Raster(Color32::from_rgb(70, 120, 200)),
        1,
        false,
        false,
        ("スクリーン", "Screen"),
        60.0,
    ),
    dummy(
        ("レイヤー 1", "Layer 1"),
        Kind::Raster(Color32::from_rgb(160, 160, 170)),
        1,
        false,
        false,
        NORMAL,
        100.0,
    ),
    dummy(
        ("調整 1", "Adjustment 1"),
        Kind::Adjustment,
        0,
        false,
        false,
        NORMAL,
        100.0,
    ),
    dummy(
        ("背景", "Background"),
        Kind::Fill(Color32::from_rgb(230, 230, 230)),
        0,
        false,
        true,
        NORMAL,
        100.0,
    ),
];
const CLIPPED: &[usize] = &[0, 1, 4];

/// 合成方法と不透明度の帯の高さ。
const TOP_HEIGHT: f32 = 20.0;
/// 操作の帯の高さ。
const BAR_HEIGHT: f32 = 26.0;

pub fn draw(ui: &mut Ui, ctx: &mut SketchCtx) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);

    // 上: 合成方法（左）と不透明度（右）
    let top = Rect::from_min_size(
        pos2(r.left() + t::PADDING, r.top() + 4.0),
        vec2(r.width() - 2.0 * t::PADDING, TOP_HEIGHT),
    );
    let left_w = ((top.width() - 6.0) * 0.42).floor();
    let blend = Rect::from_min_size(top.min, vec2(left_w, top.height()));
    let opacity = Rect::from_min_max(pos2(top.left() + left_w + 6.0, top.top()), top.max);
    let selected = ctx.values.index("layers.selected", 0).min(ROWS.len() - 1);
    let blend_name = ctx.pick(ROWS[selected].blend.0, ROWS[selected].blend.1);
    let _ = w::dropdown(ui, blend, "sketch.blend", None, blend_name, None, true, 0.0);
    let value = ctx
        .values
        .float("layers.top-opacity", ROWS[selected].opacity);
    let spec = SliderSpec::new(
        ctx.pick("不透明度", "Opacity"),
        0.0,
        100.0,
        NumberFormat::int("%"),
    );
    let out = w::slider(ui, opacity, "sketch.opacity", value, &spec);
    if out.changed {
        ctx.values.set_float("layers.top-opacity", out.value);
    }

    // 操作の帯（上の帯のすぐ下）
    let bar = Rect::from_min_size(
        pos2(r.left(), top.bottom() + 4.0),
        vec2(r.width(), BAR_HEIGHT),
    );
    toolbar(ui, ctx, bar);

    // 下: 行の一覧
    let list = Rect::from_min_max(pos2(r.left(), bar.bottom()), r.max);
    w::fill(ui.painter(), list, t::CONTROL_BG);
    for (i, d) in ROWS.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + i as f32 * ROW_HEIGHT),
            vec2(list.width(), ROW_HEIGHT),
        );
        if row.top() >= list.bottom() {
            break;
        }
        common::row(ui, ctx, list, row, i, d, 26.0);
        if CLIPPED.contains(&i) {
            // 字下げせず、サムネイルの左（目とサムネイルの間）に細い帯
            let thumb_left = row.left() + 32.0 + 14.0 * d.depth as f32;
            let bar = Rect::from_min_size(
                pos2(thumb_left - 3.0 - CLIP_BAR_WIDTH, row.top() + 3.0),
                vec2(CLIP_BAR_WIDTH, row.height() - 7.0),
            );
            w::rounded(&ui.painter_at(list), bar, CLIP_BAR, 1.0);
        }
    }
}

/// 操作の帯。左端にクリッピング（押すと入・切）、右端から順に削除・調整レイヤー・フォルダーの追加・塗りつぶし・レイヤーの追加。
fn toolbar(ui: &mut Ui, ctx: &mut SketchCtx, bar: Rect) {
    w::fill(ui.painter(), bar, t::PANEL_HEADER);
    let button =
        |x: f32| Rect::from_min_size(pos2(x, bar.top() + 1.0), vec2(26.0, bar.height() - 2.0));

    let clipping = ctx.values.bool("layers.clipping", false);
    let clip = w::icon_button(
        ui,
        button(bar.left() + 4.0),
        "sketch.toolbar.clipping",
        "clip_below",
        ctx.pick("下のレイヤーでクリッピング", "Clip to the Layer Below"),
        clipping,
        true,
        18.0,
    );
    if clip.clicked() {
        ctx.values.set_bool("layers.clipping", !clipping);
    }

    // 左から: レイヤーの追加・塗りつぶし・フォルダーの追加・調整レイヤー・削除（右端に詰める）
    let right: [(&str, &str, &str); 5] = [
        ("add", "新規レイヤー", "New Layer"),
        (
            "format_color_fill",
            "新規塗りつぶしレイヤー",
            "New Fill Layer",
        ),
        ("folder", "レイヤーをグループ化", "Group Layers"),
        ("texture", "新規調整レイヤー", "New Adjustment Layer"),
        ("delete", "レイヤーを削除", "Delete Layer"),
    ];
    let pitch = 27.0;
    let x0 = bar.right() - 4.0 - pitch * right.len() as f32;
    for (k, (icon, ja, en)) in right.iter().enumerate() {
        let _ = w::icon_button(
            ui,
            button(x0 + pitch * k as f32),
            ("sketch.toolbar.right", k),
            icon,
            ctx.pick(ja, en),
            false,
            true,
            18.0,
        );
    }
}
