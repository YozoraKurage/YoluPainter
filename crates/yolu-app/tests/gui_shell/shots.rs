//! 画面のスクリーンショットの切り抜き先の範囲を求める補助。撮る口と、組・ポップアップの範囲は `common::shot`（ここは組の中の部品を探す所だけ）。
pub use crate::common::shot::{group, popup};
use egui::Rect;
use egui_kittest::Harness;
use yolu_app::{Tab, YoluApp};

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
