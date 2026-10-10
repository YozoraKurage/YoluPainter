//! 履歴の種類と、保持している段の読み取り。表示名は画面の側で決める。

use super::{Command, Document};

/// 取り消しの一段の種類。保存形式や履歴の予算には含めない。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryKind {
    #[default]
    Other,
    Brush,
    Pixels,
    AddLayer,
    RemoveLayer,
    LayerOrder,
    LayerProperties,
    Channel,
    Fill,
    AddMask,
    RemoveMask,
    Mask,
    Selection,
    AddEffect,
    RemoveEffect,
    Effect,
    Anchor,
    Path,
    Batch,
    /// 見た目の設定（`Document::set_look`）。
    Look,
    /// 名前を付けて残した選択範囲の変更（`Document::save_selection` など）。
    SavedSelections,
    /// テキストレイヤーの値（`Document::set_text`・テキストの値を外す）。
    Text,
    /// 定規を作る・動かす・消す・移す・設定を変える（`Document::set_rulers`・`move_rulers`・`set_snap_ruler`）。
    Rulers,
}

impl Document {
    /// 保持している全段を古い順に読む。現在位置は `undo_count()`、後半はやり直しの段。
    /// 予算で破棄された段は含まない。先頭の段の直前を位置 0 とする。
    pub fn history(&self) -> impl DoubleEndedIterator<Item = HistoryKind> + '_ {
        self.undo
            .iter()
            .chain(self.redo.iter().rev())
            .map(|e| e.kind)
    }
}

impl Command {
    pub(super) fn history_kind(&self) -> HistoryKind {
        match self {
            Self::Stroke { .. } | Self::Material(_) => HistoryKind::Brush,
            Self::Insert { .. } => HistoryKind::AddLayer,
            Self::Remove { .. } => HistoryKind::RemoveLayer,
            Self::Structure { .. } => HistoryKind::LayerOrder,
            Self::Property { .. } => HistoryKind::LayerProperties,
            Self::ChannelEnabled { .. } | Self::ChannelInfo { .. } => HistoryKind::Channel,
            Self::FillValue { .. } | Self::FillChannel { .. } | Self::Projection { .. } => {
                HistoryKind::Fill
            }
            Self::AddMask { .. } => HistoryKind::AddMask,
            Self::RemoveMask { .. } => HistoryKind::RemoveMask,
            Self::SwapSmartMask { .. } => HistoryKind::Mask,
            Self::Selection { .. } => HistoryKind::Selection,
            Self::Stack { before, after, .. } if after.len() > before.len() => {
                HistoryKind::AddEffect
            }
            Self::Stack { before, after, .. } if after.len() < before.len() => {
                HistoryKind::RemoveEffect
            }
            Self::Stack { .. } | Self::FilterSeams { .. } => HistoryKind::Effect,
            Self::Anchor { .. } => HistoryKind::Anchor,
            Self::Path(_) => HistoryKind::Path,
            Self::Compound(_) => HistoryKind::Batch,
            Self::Look { .. } => HistoryKind::Look,
            Self::SavedSelections { .. } => HistoryKind::SavedSelections,
            Self::Text(_) => HistoryKind::Text,
            Self::Rulers { .. } => HistoryKind::Rulers,
            _ => HistoryKind::Other,
        }
    }
}
