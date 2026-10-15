//! 画面の絵の切り抜き先の範囲を求める補助。撮る口は `common::shot`（ここは範囲を決めるだけ）。
use crate::common::drive::Drive;
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
    h.popup()
        .as_ref()
        .expect("ポップアップが開いている")
        .state
        .rect
}

/// ドックの組 `tab` の中にある、名前 `label` の部品の範囲（ほかの組に同じ名前の部品があるときの探し方。プロパティのレイヤーのタブの
/// 「ロック」の「すべて」と、アセットの種類の「すべて」など）。組の中に無ければ `None`。
pub fn in_group(h: &Harness<'_, YoluApp>, tab: Tab, label: &str) -> Option<Rect> {
    use egui_kittest::kittest::Queryable;
    let group = group(h, tab);
    h.query_all_by_label(label)
        .map(|node| node.rect())
        .find(|r| group.contains_rect(*r))
}

/// 組 `tab` の中の `label` の部品を押す（見つからなければ落とす）。
pub fn click_in_group(h: &mut Harness<'_, YoluApp>, tab: Tab, label: &str) {
    let at = in_group(h, tab, label)
        .unwrap_or_else(|| panic!("{tab:?} の組に {label} が無い"))
        .center();
    crate::common::click(h, at);
    h.run();
}
