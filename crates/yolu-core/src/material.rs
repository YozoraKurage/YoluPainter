//! 複数チャンネルを一回の Undo で塗るマテリアル。
use crate::{Channel, Rgba8};
/// チャンネルとそこへ描く straight RGBA8 の値。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelPaint {
    pub channel: Channel,
    pub value: Rgba8,
}
impl ChannelPaint {
    pub const fn new(channel: Channel, value: Rgba8) -> Self {
        Self { channel, value }
    }
}
/// グラデーションの形。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GradientShape {
    #[default]
    Linear,
    Radial,
}
/// 画素座標で指定する二色のグラデーション。補間は乗算済みアルファ。
#[derive(Clone, Copy, Debug)]
pub struct GradientSettings {
    pub shape: GradientShape,
    pub start: crate::glam::DVec2,
    pub end: crate::glam::DVec2,
    pub from: Rgba8,
    pub to: Rgba8,
    pub opacity: f64,
}
impl Default for GradientSettings {
    fn default() -> Self {
        Self {
            shape: GradientShape::Linear,
            start: crate::glam::DVec2::ZERO,
            end: crate::glam::DVec2::ZERO,
            from: Rgba8::new(0, 0, 0, 255),
            to: Rgba8::TRANSPARENT,
            opacity: 1.0,
        }
    }
}
impl GradientSettings {
    pub fn validate(&self) -> Result<(), crate::CoreError> {
        if !self.start.is_finite() || !self.end.is_finite() || !(0.0..=1.0).contains(&self.opacity)
        {
            return Err(crate::CoreError::InvalidArgument("グラデーション"));
        }
        Ok(())
    }
    pub fn color_at(&self, x: f64, y: f64) -> Rgba8 {
        let dx = self.end.x - self.start.x;
        let dy = self.end.y - self.start.y;
        let length2 = dx * dx + dy * dy;
        let x = x - self.start.x;
        let y = y - self.start.y;
        let t = if length2 <= 1e-12 {
            1.0
        } else if self.shape == GradientShape::Linear {
            (x * dx + y * dy) / length2
        } else {
            ((x * x + y * y) / length2).sqrt()
        };
        let t = crate::math::clamp01(t);
        let fa = self.from.a as f64 / 255.0 * (1.0 - t);
        let ta = self.to.a as f64 / 255.0 * t;
        let a = fa + ta;
        if a <= 0.0 {
            return Rgba8::TRANSPARENT;
        }
        let c = |f: u8, t: u8| {
            crate::math::to_byte((f as f64 / 255.0 * fa + t as f64 / 255.0 * ta) / a)
        };
        Rgba8::new(
            c(self.from.r, self.to.r),
            c(self.from.g, self.to.g),
            c(self.from.b, self.to.b),
            crate::math::to_byte(a),
        )
    }
}
