//! 文書の操作の失敗。断った操作は何も変えない（ストロークの途中の失敗は、そのストロークを取り消してから返す）。

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreError {
    MergeRefused(crate::MergeRefusal),
    MergeAppearance(Box<crate::LayerMergeReport>),
    Cancelled,
    LayerLocked {
        layer: crate::LayerId,
        holder: crate::LayerId,
        lock: crate::LayerLocks,
    },
    /// 引数が範囲外・有限でない など。中身は何の値か。
    InvalidArgument(&'static str),
    /// その ID のレイヤーが文書に無い。
    LayerNotFound,
    /// そのチャンネルが文書に無い（消したユーザーチャンネルなど）。
    ChannelNotFound,
    /// 進行中のストロークがあるので、ほかの編集・Undo・Redo はできない。
    StrokeActive,
    /// 札のストロークはもう終わっている（確定・取消、または途中の失敗で取り消された）。
    NoActiveStroke,
    /// その操作はこのレイヤー・チャンネルにはできない。
    Unsupported(&'static str),
    /// 画素の予算（SourceBudgetBytes）を超える。
    SourceBudgetExceeded,
    /// ストロークの巻き戻し用の写しの予算（ActiveStrokeBudgetBytes）を超える。ストロークは取り消した。
    StrokeBudgetExceeded,
    /// 作業のメモリの上限（Normal の出力など）を超えるので、確保の前に断った。
    WorkingBudgetExceeded,
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::MergeRefused(reason) => write!(f, "結合できない: {reason}"),
            CoreError::MergeAppearance(report) => write!(
                f,
                "結合による見た目の変化が許容差を超える: {}",
                report.max_visible_difference
            ),
            CoreError::Cancelled => write!(f, "操作を取り消した"),
            CoreError::LayerLocked { .. } => write!(f, "層または親グループがロックされている"),
            CoreError::InvalidArgument(what) => write!(f, "値が範囲外: {what}"),
            CoreError::LayerNotFound => write!(f, "レイヤーが無い"),
            CoreError::ChannelNotFound => write!(f, "チャンネルが無い"),
            CoreError::StrokeActive => write!(f, "ストロークの途中"),
            CoreError::NoActiveStroke => write!(f, "ストロークは終わっている"),
            CoreError::Unsupported(what) => write!(f, "できない: {what}"),
            CoreError::SourceBudgetExceeded => write!(f, "画素の予算を超える（取り消した）"),
            CoreError::WorkingBudgetExceeded => write!(f, "作業のメモリの上限を超える"),
            CoreError::StrokeBudgetExceeded => {
                write!(f, "ストロークの予算を超える（取り消した）")
            }
        }
    }
}

impl std::error::Error for CoreError {}
