//! 画面のスクリーンショットのテストの撮る口。ウィンドウ全体は `window`（並びの見本だけに使う）、ほかは `region` で、部品・パネルの
//! 切り抜きを撮る。切り抜きにすると、ある部品の見た目を変えたときに変わる絵が、その部品の絵だけになる。
//!
//! 切り抜きの範囲は、その部品・パネルの範囲ちょうどにする（外へ余白を足さず、端は隣の画素を入れない向きに丸める）。範囲は、
//! 組は `group`（ドックの組の実際の範囲）、ポップアップは `popup`・`with_popup`、知らせは `toast_strip`、ウィンドウは `window_shot` で取る。
#![allow(dead_code)]

use super::drive::Drive;
use egui::Rect;
use egui_kittest::Harness;
use yolu_app::{Tab, YoluApp};

/// ウィンドウ全体を撮る。画面全体の並びを見る見本の絵だけに使う。
pub fn window<State>(harness: &mut Harness<'_, State>, name: &str) {
    harness.snapshot(name);
}

/// 画面の範囲 `rect`（論理の点）を切り抜いて撮る。範囲は画面の中に丸める（外にはみ出した分は撮らない）。
pub fn region<State>(harness: &mut Harness<'_, State>, name: &str, rect: Rect) {
    let image = harness.render().expect("画面を描く");
    let cropped = crop(&image, rect, harness.ctx.pixels_per_point());
    egui_kittest::image_snapshot(&cropped, name);
}

/// ポインタを画面の外へ出して 1 フレーム進めてから、範囲 `rect`（論理の点）を撮る（直前に押した所のポインタの印・ホバーがスクリーンショットに残らないように）。
pub fn clean_region<State>(h: &mut Harness<'_, State>, name: &str, rect: Rect) {
    h.event(egui::Event::PointerGone);
    h.step();
    region(h, name, rect);
}

/// `region` の、比べ方の設定 `options`（許す差の画素数など）を渡す形。
pub fn region_with<State>(
    h: &mut Harness<'_, State>,
    name: &str,
    rect: Rect,
    options: &egui_kittest::SnapshotOptions,
) {
    let image = h.render().expect("画面を描く");
    let cropped = crop(&image, rect, h.ctx.pixels_per_point());
    egui_kittest::image_snapshot_options(&cropped, name, options);
}

/// 名前のウィンドウ `window`（`yolu_app::windows::window_rect`）の中だけを撮る。
pub fn window_shot(h: &mut Harness<'_, YoluApp>, window: &str, name: &str) {
    let rect = yolu_app::windows::window_rect(&h.ctx, window)
        .unwrap_or_else(|| panic!("{window} を描いていない"));
    clean_region(h, name, rect);
}

/// ドックの組（タブの帯と中身）の範囲。`tab` を含む主の面の組で、egui_dock が毎フレーム控える実際の範囲（組と組の間の分け目は含まない）。
pub fn group(h: &Harness<'_, YoluApp>, tab: Tab) -> Rect {
    h.state()
        .dock
        .main_surface()
        .iter()
        .find_map(|node| match node {
            egui_dock::Node::Leaf(leaf) if leaf.tabs().contains(&tab) => Some(leaf.rect()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{tab:?} の組が無い"))
}

/// `first` の組から、同じ列の下へ続く組を `below` の組の手前まで（`None` なら列の最後まで）足した範囲。左の列のサブツール・ツールプロパティ・
/// ブラシサイズのように、縦に並ぶ組をまとめて撮るときに使う。組と組の間の分け目は含む。
pub fn group_span(h: &Harness<'_, YoluApp>, first: Tab, below: Option<Tab>) -> Rect {
    let top = group(h, first);
    let limit = below.map_or(f32::INFINITY, |tab| group(h, tab).top());
    h.state()
        .dock
        .main_surface()
        .iter()
        .filter_map(|node| match node {
            egui_dock::Node::Leaf(leaf) => Some(leaf.rect()),
            _ => None,
        })
        .filter(|r| r.top() >= top.top() - 0.5 && r.bottom() <= limit + 0.5)
        .filter(|r| r.left() < top.right() && r.right() > top.left())
        .fold(top, |all, r| all.union(r))
}

/// いま開いているポップアップ（メニュー・ドロップダウン。入れ子のメニューを含む）の本体の範囲。
pub fn popup(h: &Harness<'_, YoluApp>) -> Rect {
    h.popup()
        .as_ref()
        .expect("ポップアップが開いている")
        .state
        .rect
}

/// `base` に、開いているポップアップの本体の範囲を足した範囲（余白は足さない。ポップアップが無ければ `base` のまま）。
pub fn with_popup(h: &Harness<'_, YoluApp>, base: Rect) -> Rect {
    match h.popup().as_ref() {
        Some(open) => base.union(open.state.rect),
        None => base,
    }
}

/// 知らせ（トースト）を描く、状態の帯の左の範囲（右の項目の手前まで）。製品が知らせを置く範囲と同じ関数で取る。
pub fn toast_strip(h: &Harness<'_, YoluApp>) -> Rect {
    let free_right = yolu_app::shell::status_free_right(&h.ctx, h.app());
    yolu_app::toast::strip_rect(&h.ctx, free_right)
}

/// メニューバーの下の帯（オプションバー。ウィンドウの幅いっぱい）の範囲。
pub fn options_bar(h: &Harness<'_, YoluApp>) -> Rect {
    use yolu_app::ui::theme::{MENU_BAR_HEIGHT, OPTIONS_BAR_HEIGHT};
    Rect::from_min_max(
        egui::pos2(0.0, MENU_BAR_HEIGHT),
        egui::pos2(
            h.ctx.content_rect().right(),
            MENU_BAR_HEIGHT + OPTIONS_BAR_HEIGHT,
        ),
    )
}

/// 描いた画像 `image` から、論理の点の範囲 `rect` を切り抜く（拡大率 `pixels_per_point`）。端の画素が範囲に半分しか掛からないときは、
/// その画素を入れない（左上は切り上げ・右下は切り捨て）。範囲が空なら 1×1 にする。
pub fn crop(image: &image::RgbaImage, rect: Rect, pixels_per_point: f32) -> image::RgbaImage {
    let (w, h) = (image.width(), image.height());
    // 浮動小数の誤差（0.1 を 3 倍して 0.30000001 になる類）で、ちょうど画素の境の端が 1 画素ずれないようにする
    const EPS: f32 = 1e-3;
    let lo =
        |v: f32, limit: u32| (((v * pixels_per_point) - EPS).ceil().max(0.0) as u32).min(limit);
    let hi =
        |v: f32, limit: u32| (((v * pixels_per_point) + EPS).floor().max(0.0) as u32).min(limit);
    let (x0, y0) = (lo(rect.min.x, w), lo(rect.min.y, h));
    let (x1, y1) = (
        hi(rect.max.x, w).max(x0 + 1).min(w),
        hi(rect.max.y, h).max(y0 + 1).min(h),
    );
    image::imageops::crop_imm(
        image,
        x0.min(w - 1),
        y0.min(h - 1),
        x1 - x0.min(w - 1),
        y1 - y0.min(h - 1),
    )
    .to_image()
}
