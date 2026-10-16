//! 裏で動く仕事の状態（`AppState::services`）。egui の型を持たない（画面の状態は `UiState`）。

use crate::livelink::{LinkRequest, LinkView};

/// 裏で動く仕事の状態（保存・復旧の書き置き・Live Link・外からの操作・更新・クラッシュの報告・使っているメモリ）。
pub struct Services {
    /// .ylp の保存（裏のスレッドの仕事と進み具合。画面のスレッドは頼みと結果の受けだけ）。
    pub save: crate::project::SaveState,
    /// 復旧用の世代の書き置きと復旧のウィンドウ（`recovery`。動かすまでは何もしない）。
    pub recovery: crate::recovery::RecoveryState,
    /// Live Link の様子（毎フレーム `LiveLink` から写す。状態の帯とメニューが読む）。
    pub link: LinkView,
    /// Live Link を始める・やめる頼み（`YoluApp` が次に当てる）。
    pub link_request: Option<LinkRequest>,
    /// 今の文書の Live Link の相手（当てた頼み・まとめた Rig との対応。保存・送り直し・書き出しの返事が読む）。
    pub link_target: Option<crate::livelink::LinkTarget>,
    /// 開いた .ylp に残っていた Live Link の相手（`livelink.json`）。`LiveLink` が次のフレームで開き直す。
    pub link_reopen: Option<yolu_protocol::files::Request>,
    /// Live Link で入れた元の画像のレイヤーの印（レイヤーの欄が読む。保存しない）。
    pub link_originals: crate::livelink::OriginalMarks,
    /// 外からの操作（MCP のクライアント・コマンドライン）を受けている様子（毎フレーム `McpServer` から写す。状態の帯の印が読む）。
    pub ops: crate::mcp_server::OpsView,
    /// 自動更新（公開鍵を組み込んだビルドだけで動く。聞かずに通信しない）。
    pub update: crate::update::UpdateState,
    /// 前の実行のクラッシュの報告（`crash::window`）。
    pub crash: crate::crash::window::Report,
    /// 状態の帯の右端のバージョン・ビルドと使っているメモリ。
    pub usage: crate::usage::Usage,
}

impl Default for Services {
    /// `AppState::new` の初期値。更新は組み込んだ公開鍵と実行ファイルの入れ方から作り（`UpdateState::detect`）、ほかは各型の既定。
    fn default() -> Self {
        Services {
            save: Default::default(),
            recovery: Default::default(),
            link: LinkView::default(),
            link_request: None,
            link_target: None,
            link_reopen: None,
            link_originals: Default::default(),
            ops: Default::default(),
            update: crate::update::UpdateState::detect(),
            crash: Default::default(),
            usage: crate::usage::Usage::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 11 欄の初期値が、`AppState::new` が入れていた値（各型の既定と `UpdateState::detect`）と同じ。
    #[test]
    fn the_initial_values_match_what_app_state_new_used_to_set() {
        let s = Services::default();
        assert!(!s.save.background, "保存は呼び手のスレッドで終える形");
        assert!(s.save.progress().is_none());
        assert!(!s.recovery.is_enabled(), "復旧は動かしていない");
        assert!(s.recovery.root().is_none());
        assert_eq!(
            s.recovery.settings(),
            &crate::recovery::RecoverySettings::default()
        );
        assert_eq!(s.link, LinkView::default());
        assert!(s.link_request.is_none());
        assert!(s.link_target.is_none());
        assert!(s.link_reopen.is_none());
        assert!(s.link_originals.is_empty());
        assert_eq!(s.ops, crate::mcp_server::OpsView::default());
        let detected = crate::update::UpdateState::detect();
        assert_eq!(s.update.enabled(), detected.enabled());
        assert_eq!(s.update.mode(), detected.mode());
        assert_eq!(s.update.preference(), detected.preference());
        assert_eq!(s.update.beta(), detected.beta());
        assert!(!s.update.is_busy());
        assert!(s.update.offer().is_none());
        assert!(s.update.ready().is_none());
        assert!(!s.update.is_asking());
        assert!(!s.update.is_ready_open());
        assert!(!s.update.is_blocked());
        assert!(s.crash.text.is_empty());
        assert!(!s.crash.unread);
        assert!(!s.crash.open);
        assert!(s.crash.request.is_none());
        assert!(s.crash.error.is_none());
        assert_eq!(s.usage, crate::usage::Usage::default());
    }
}
