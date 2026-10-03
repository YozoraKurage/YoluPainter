//! 文書の操作の失敗。断った操作は何も変えない（ストロークの途中の失敗は、そのストロークを取り消してから返す）。

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreError {
    /// 引数が範囲外・有限でない など。中身は何の値か。
    InvalidArgument(&'static str),
    /// その ID のレイヤーが文書に無い。
    LayerNotFound,
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
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::InvalidArgument(what) => write!(f, "値が範囲外です: {what}"),
            CoreError::LayerNotFound => write!(f, "レイヤーが見つかりません"),
            CoreError::StrokeActive => write!(
                f,
                "描いている途中のストロークを先に終えるか取り消してください"
            ),
            CoreError::NoActiveStroke => write!(f, "このストロークはもう終わっています"),
            CoreError::Unsupported(what) => write!(f, "できない操作です: {what}"),
            CoreError::SourceBudgetExceeded => {
                write!(f, "画素の予算を超えるので取り消しました。予算を上げるか、文書の画素を減らしてください")
            }
            CoreError::StrokeBudgetExceeded => {
                write!(f, "ストロークの巻き戻し用の予算を超えるので取り消しました。短いストロークにするか、予算を上げてください")
            }
        }
    }
}

impl std::error::Error for CoreError {}
