//! 文書の操作の失敗。断った操作は何も変えない（ストロークの途中の失敗は、そのストロークを取り消してから返す）。

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreError {
    MergeRefused(crate::MergeRefusal),
    MergeAppearance(Box<crate::LayerMergeReport>),
    /// 取り消しの旗が立ったので、操作・効果の評価を止めた（途中の結果は公開しない）。
    Cancelled,
    LayerLocked {
        layer: crate::LayerId,
        holder: crate::LayerId,
        lock: crate::LayerLocks,
    },
    /// 入力のまま通している効果（使えるマップが無い Generator・出ていないデカール）を焼き込む操作（結合）は、効果を落とすので断る。
    /// `mask` はレイヤーのマスクのスタックの効果か。
    InactiveEffect {
        layer: crate::LayerId,
        mask: bool,
        reason: Box<crate::InactiveReason>,
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
    /// レイヤーの画素のコピー・カット・ペーストを断った理由（何も変えていない）。
    Clipboard(crate::ClipboardRefusal),
    /// `Document::batch` の編集の中では、ストローク・Undo・Redo・履歴を消す書き込みはできない（まとめは入れ子にもできない）。
    BatchActive,
    /// ディスクへ逃がしたタイルの中身を読み戻せない（キャッシュのファイルが読めない）。読めない中身は透明として扱わない。
    TileUnreadable,
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
            CoreError::LayerLocked { .. } => {
                write!(f, "レイヤーまたは親グループがロックされている")
            }
            CoreError::InactiveEffect { reason, .. } => {
                write!(f, "効いていない効果は焼き込めない: {reason}")
            }
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
            CoreError::Clipboard(reason) => write!(f, "{reason}"),
            CoreError::BatchActive => write!(f, "まとめた編集の途中ではできない"),
            CoreError::TileUnreadable => write!(f, "ディスクのキャッシュからタイルを読めない"),
        }
    }
}

impl std::error::Error for CoreError {}
