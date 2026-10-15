//! 画面の試験の束: キャンバス・ツール・ブラシ・選択・色・効果（`egui_kittest` の harness。描画は wgpu のソフトの描画）。
//!
//! ウィンドウ（harness）を持つ試験は `common::gpu_thread` の貸し出しで 1 つずつ走る（lavapipe の中で同時に装置を作ると落ちることがあったため）。
//! 画面を使わない試験が同じファイルに混ざっていてもよい（貸し出しを取らないので並んで走る）。ただしウィンドウを作らなくても GPU の装置を作る試験
//! （製品のスレッドで GPU の確認・ベイクをする、`common::canvas_device` など）は、先頭で `common::gpu_thread::lease()` を取る。GPU の装置は `common::shared_gpu` の共用を使う。
//! 新しい試験は、該当する束のフォルダにファイルを足し、この `main.rs` に `mod` を 1 行足す（`headless/bundle_layout.rs` が足し忘れを見つける）。
#[path = "../common/mod.rs"]
mod common;

use common::drive::Drive;
use egui::{pos2, Rect};
use egui_kittest::Harness;
use yolu_app::{Tab, YoluApp};

/// 範囲 `rect` を、小数の端を含む整数の点の範囲に広げる（左上は切り捨て・大きさは切り上げ）。
pub fn outward(rect: Rect) -> Rect {
    Rect::from_min_size(rect.min.floor(), rect.size().ceil())
}

/// ポインタを画面の外へ出して 1 フレーム進めてから、範囲 `rect`（論理の点）を撮る（直前に押した所のポインタの印・ホバーが絵に残らないように）。
pub fn clean_region<State>(h: &mut Harness<'_, State>, name: &str, rect: Rect) {
    h.event(egui::Event::PointerGone);
    h.step();
    common::shot::region(h, name, outward(rect));
}

/// `common::shot::region` の、比べ方の設定 `options`（許す差の画素数など）を渡す版。
pub fn region_with<State>(
    h: &mut Harness<'_, State>,
    name: &str,
    rect: Rect,
    options: &egui_kittest::SnapshotOptions,
) {
    let image = h.render().expect("画面を描く");
    let cropped = common::shot::crop(&image, rect, h.ctx.pixels_per_point());
    egui_kittest::image_snapshot_options(&cropped, name, options);
}

/// 名前のウィンドウ `window`（`yolu_app::windows::window_rect`）の中だけを撮る。
pub fn window_shot(h: &mut Harness<'_, YoluApp>, window: &str, name: &str) {
    let rect = yolu_app::windows::window_rect(&h.ctx, window)
        .unwrap_or_else(|| panic!("{window} を描いていない"));
    clean_region(h, name, rect);
}

/// 組の中身の範囲: 先頭のタブ `first` の帯から、下の組のタブ `below` の上まで（`None` ならウィンドウの下まで）。
/// 左のドックの組は幅 300 の列の右端まで、右の列の組はウィンドウの右端まで。
pub fn group_rect(h: &Harness<'_, YoluApp>, first: Tab, below: Option<Tab>) -> Rect {
    let tabs = &h.state().tab_rects;
    let tab = tabs[&first];
    let bottom = below.map_or(h.ctx.content_rect().bottom(), |b| tabs[&b].top());
    let right = if tab.left() < 100.0 {
        tab.left() + 300.0
    } else {
        h.ctx.content_rect().right()
    };
    Rect::from_min_max(pos2(tab.left() - 2.0, tab.top()), pos2(right, bottom))
}

/// オプションバー（メニューバーの下の帯。ウィンドウの幅いっぱい）の範囲。
pub fn options_bar(h: &Harness<'_, YoluApp>) -> Rect {
    Rect::from_min_max(pos2(0.0, 24.0), pos2(h.ctx.content_rect().right(), 62.0))
}

/// 知らせ（トースト）が出る、ウィンドウの左下の範囲。
pub fn toast_area(h: &Harness<'_, YoluApp>) -> Rect {
    let bottom = h.ctx.content_rect().bottom();
    Rect::from_min_max(pos2(44.0, bottom - 80.0), pos2(524.0, bottom - 20.0))
}

/// `base` に、開いているポップアップ（入れ子のメニューを含む）の範囲を足して、周りに 8 点の余白をつけた範囲。
pub fn with_popup(h: &Harness<'_, YoluApp>, base: Rect) -> Rect {
    match h.popup().as_ref() {
        Some(popup) => base.union(popup.state.rect).expand(8.0),
        None => base,
    }
}

/// ツールの帯・オプションバー・サブツールとツールプロパティ（左のドックの上の 2 つの組）の範囲。
pub fn tool_panels(h: &Harness<'_, YoluApp>) -> Rect {
    let tabs = &h.state().tab_rects;
    Rect::from_min_max(
        pos2(0.0, 24.0),
        pos2(
            tabs[&Tab::SubTools].left() + 300.0,
            tabs[&Tab::BrushSize].top(),
        ),
    )
}

/// ツールプロパティの組と、キャンバスを合わせた範囲（ツールの欄と、その操作の結果の線・印）。
pub fn tool_props_and_canvas(h: &Harness<'_, YoluApp>) -> Rect {
    let canvas = h
        .state()
        .canvas_view_rect()
        .expect("キャンバスを描いている");
    group_rect(h, Tab::ToolProperties, Some(Tab::BrushSize)).union(canvas)
}

mod anti_alias_ui;
mod brush_import_ui;
mod brush_menu;
mod brush_mix;
mod brushes;
mod canvas_gpu;
mod clipboard;
mod clipping_button;
mod clone_parity;
mod color_adjust;
mod color_adjust_app;
mod curve_editor;
mod default_layout;
mod distribute;
mod drafting;
mod effects_ui;
mod eyedrop;
mod fillfx_gui;
mod gestures;
mod grunge_picker;
mod history;
mod image_stage_ui;
mod layer_menu;
mod layerops;
mod pathtool;
mod pressure_adjust;
mod ramp_editor;
mod ramp_panel;
mod region;
mod seams_ui;
mod selection;
mod selection_bar;
mod selection_build;
mod stencil;
mod stroke_look;
mod text_tool;
mod tool_keys;
mod tool_layout_ui;
mod view_controls;
mod warp;
