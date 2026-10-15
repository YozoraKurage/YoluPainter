//! 画面の試験の束: 3D ビュー・マテリアルの見た目（lilToon を含む）・ポーズ・テクスチャセット・出力（`egui_kittest` の harness。描画は wgpu のソフトの描画）。
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

/// `base` に、開いているポップアップ（入れ子のメニューを含む）の範囲を足して、周りに 8 点の余白をつけた範囲。
pub fn with_popup(h: &Harness<'_, YoluApp>, base: Rect) -> Rect {
    match h.popup().as_ref() {
        Some(popup) => base.union(popup.state.rect).expand(8.0),
        None => base,
    }
}

/// 3D ビューの範囲。
pub fn view3d_area(h: &Harness<'_, YoluApp>) -> Rect {
    h.state().view3d_rect().expect("3D のタブを描いている")
}

/// 3D ビューの右上の隅から開く、設定のパネル（表示・画質・視点）が入る範囲。
pub fn settings_corner(h: &Harness<'_, YoluApp>) -> Rect {
    let area = view3d_area(h);
    Rect::from_min_size(
        pos2(area.right() - 330.0, area.top()),
        egui::vec2(330.0, 520.0),
    )
}

mod gpu_memory;
mod liltoon;
mod liltoon_ortho;
mod liltoon_panel;
mod liltoon_reference;
mod modes;
mod normalset;
mod objects;
mod outputs;
mod overlap_uv_ui;
mod parity_input;
mod parity_review;
mod parity_symmetry;
mod parity_views;
mod pose;
mod pose_takes;
mod pose_ui;
mod sets;
mod tool_keys_3d;
mod view3d;
mod view3d_aniso;
mod view3d_axes;
mod view3d_brush;
mod view3d_draft;
mod view3d_fx;
mod view3d_lag;
mod view3d_look;
mod view3d_navigation;
mod view3d_padding;
mod view3d_path_rect;
mod view3d_rulers;
mod view3d_rulers_edit;
mod view3d_select;
mod view3d_select_overlay;
mod view3d_select_pen;
mod view3d_sets;

/// 画面の外枠（`YoluApp`）を直に持つテストも、AppState の欄を運転の口で読み書きする。
impl common::drive::Drive for YoluApp {
    fn app(&self) -> &yolu_app::state::AppState {
        &self.state
    }
    fn app_mut(&mut self) -> &mut yolu_app::state::AppState {
        &mut self.state
    }
}

/// AppState だけを持つ画面のテスト（パネル 1 枚を描く `Harness<AppState>`）の口。
impl common::drive::Drive for Harness<'_, yolu_app::state::AppState> {
    fn app(&self) -> &yolu_app::state::AppState {
        self.state()
    }
    fn app_mut(&mut self) -> &mut yolu_app::state::AppState {
        self.state_mut()
    }
}
