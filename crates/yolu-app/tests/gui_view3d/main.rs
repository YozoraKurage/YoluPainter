//! 画面の試験の束: 3D ビュー・マテリアルの見た目（lilToon を含む）・ポーズ・テクスチャセット・出力（`egui_kittest` の harness。描画は wgpu のソフトの描画）。
//!
//! ウィンドウ（harness）を持つ試験は `common::gpu_thread` の貸し出しで 1 つずつ走る（lavapipe の中で同時に装置を作ると落ちることがあったため）。
//! 画面を使わない試験が同じファイルに混ざっていてもよい（貸し出しを取らないので並んで走る）。ただしウィンドウを作らなくても GPU の装置を作る試験
//! （製品のスレッドで GPU の確認・ベイクをする、`common::canvas_device` など）は、先頭で `common::gpu_thread::lease()` を取る。GPU の装置は `common::shared_gpu` の共用を使う。
//! 新しい試験は、該当する束のフォルダにファイルを足し、この `main.rs` に `mod` を 1 行足す（`headless/bundle_layout.rs` が足し忘れを見つける）。
#[path = "../common/mod.rs"]
mod common;

use egui::{pos2, Rect};
use egui_kittest::Harness;
use yolu_app::YoluApp;

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

/// 3D ビューの左上の、表示の種類のドロップダウンの矩形（描いていなければ None）。絵の画素を比べる・何も無い所を押すテストは、この中を使わない。
pub fn view3d_shading_rect(h: &Harness<'_, YoluApp>) -> Option<Rect> {
    use common::drive::DriveApp;
    use egui_kittest::kittest::Queryable;
    let view = h.view3d_rect_of(yolu_app::view3d::ViewId::FIRST)?;
    h.query_all_by_role(egui::accesskit::Role::ComboBox)
        .map(|n| n.rect())
        .find(|r| view.contains(r.center()) && r.top() < view.top() + 40.0)
        .map(|r| r.expand(2.0))
}

/// 3D ビューの左下のシェルフ（目と ▽。開いていれば上の欄も）の矩形（描いていなければ None）。画像の画素を比べるテストは、この中を使わない。
pub fn view3d_shelf_rect(h: &Harness<'_, YoluApp>) -> Option<Rect> {
    use common::drive::{Drive, DriveApp};
    let view = h.view3d_rect_of(yolu_app::view3d::ViewId::FIRST)?;
    let open = h.view3d().view.hide.shelf_open;
    let (pill, body) = yolu_app::view3d::hide::shelf::rects(view, open);
    Some(body.map_or(pill, |b| b.union(pill)).expand(2.0))
}

mod gpu_memory;
mod hide;
mod liltoon;
mod liltoon_ortho;
mod liltoon_panel;
mod liltoon_reference;
mod modes;
mod multimat;
mod multiview;
mod normalset;
mod objects;
mod outputs;
mod overlap_uv_ui;
mod parity_input;
mod parity_review;
mod parity_symmetry;
mod parity_table;
mod parity_views;
mod pose;
mod pose_takes;
mod pose_ui;
mod replay_input;
mod sets;
mod tool_keys_3d;
mod view3d;
mod view3d_aniso;
mod view3d_axes;
mod view3d_brush;
mod view3d_continuous;
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
mod view3d_taper;

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
