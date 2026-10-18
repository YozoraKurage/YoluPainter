//! 状態の側から画面への頼み（`AppState::outbox`）。egui の型を持たない（画面の状態は `UiState`）。
//!
//! 操作（`AppState::apply`・機能の `*_apply`）と知らせ（`notice`）が書き、画面（`YoluApp`・`toast::show`）が読んで当てる。

use super::DialogRequest;

/// 状態の側が画面へ出す頼み（ファイルのウィンドウを開く・パネルの並びを戻す・小さな知らせを出す）。
#[derive(Default)]
pub struct Outbox {
    /// ファイルを選ぶウィンドウを開く頼み（`YoluApp` が開く。テストでは開かない）。
    pub dialog_request: Option<DialogRequest>,
    /// パネルの並びを初めに戻す頼み（`YoluApp` が次のフレームで当てる）。
    pub reset_layout: bool,
    /// 小さな知らせの出し方の状態（どの文をいつから出したか・消したか）。描くのは `toast::show`。
    pub toast: crate::toast::Toast,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3 欄の初期値が、`AppState::new` が入れていた値（頼み無し・戻さない・知らせの既定）と同じ。
    #[test]
    fn the_initial_values_match_what_app_state_new_used_to_set() {
        let o = Outbox::default();
        assert!(o.dialog_request.is_none());
        assert!(!o.reset_layout);
        assert_eq!(o.toast, crate::toast::Toast::default());
    }
}
