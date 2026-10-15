//! 画面の絵の切り抜き先の範囲を求める補助。撮る口は `common::shot`（ここは範囲を決めるだけ）。
use egui::Rect;
use egui_kittest::Harness;
use yolu_app::{Tab, YoluApp};

/// ドックの組（タブの帯と中身）の範囲に、周りの余白 4 点を足した物。`tab` を含む組。
pub fn group(h: &Harness<'_, YoluApp>, tab: Tab) -> Rect {
    h.state()
        .dock
        .main_surface()
        .iter()
        .find_map(|node| match node {
            egui_dock::Node::Leaf(leaf) if leaf.tabs().contains(&tab) => {
                Some(leaf.rect().expand(4.0))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("{tab:?} の組が無い"))
}

/// いま開いているポップアップ（メニュー・ドロップダウン）の本体の範囲。
pub fn popup(h: &Harness<'_, YoluApp>) -> Rect {
    h.state()
        .state
        .popup
        .as_ref()
        .expect("ポップアップが開いている")
        .state
        .rect
}
