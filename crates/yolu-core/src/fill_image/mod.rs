//! 塗りつぶし画像の評価。C# FillImageSampler の式・straight RGBA8 を保つ、文書に依存しない口。
//! 入力は左下原点。ミップは元画像を借用し、出力は成功した領域だけを返す。
#[cfg(test)]
mod cancellation_tests;
mod maps;
mod mip;
mod projection;
mod sampler;
pub use maps::{Map, ModelMaps};
pub use mip::{Conversion, ImageMipChain};
pub use projection::{ModelFrame, Placement, Projection, ProjectionMode, Wrap};
pub use sampler::{FillInput, FillSampler, InactiveReason, MAX_ANISOTROPY};

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FillError {
    Invalid(&'static str),
    OverBudget { needed: u64, budget: u64 },
    Canceled,
    Allocation,
}
impl std::fmt::Display for FillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) => write!(f, "塗りつぶしの入力が不正です: {s}"),
            Self::OverBudget { needed, budget } => write!(
                f,
                "塗りつぶしの予算を超えます: 必要 {needed}、上限 {budget}"
            ),
            Self::Canceled => f.write_str("塗りつぶしを取り消しました"),
            Self::Allocation => f.write_str("塗りつぶしのメモリを確保できません"),
        }
    }
}
impl std::error::Error for FillError {}
pub(crate) fn canceled(cancel: Option<&AtomicBool>) -> Result<(), FillError> {
    #[cfg(test)]
    cancellation_tests::checkpoint(cancel);
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        Err(FillError::Canceled)
    } else {
        Ok(())
    }
}
pub(crate) fn budget(needed: u64, limit: u64) -> Result<(), FillError> {
    if needed > limit {
        Err(FillError::OverBudget {
            needed,
            budget: limit,
        })
    } else {
        Ok(())
    }
}
pub(crate) fn dimensions(w: u32, h: u32) -> Result<usize, FillError> {
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        return Err(FillError::Invalid("大きさは 1..8192"));
    }
    Ok(w as usize * h as usize)
}
pub(crate) fn zeroes<T: Default + Clone>(n: usize) -> Result<Vec<T>, FillError> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| FillError::Allocation)?;
    v.resize(n, T::default());
    Ok(v)
}
