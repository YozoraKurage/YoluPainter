//! 編集できる 2D/3D の筆跡。評価は独立した面を返し、文書・選択・履歴を変更しない。
#[cfg(test)]
mod cancellation_tests;
mod render;
mod surface;
use crate::geometry::DabRefusal;
use crate::{BrushSettings, Channel, CoreError, Rgba8};
pub use render::{render_canvas, Options, Rendered};
pub use surface::{fingerprint, render_surface};

pub const ALGORITHM_VERSION: u32 = 1;
pub const MAX_POINTS: usize = 4096;
pub const MAX_CANVAS_SAMPLES: usize = 4_000_000;

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    Invalid(&'static str),
    ModelMismatch,
    MissingTriangle,
    TooManySamples,
    Canceled,
    Core(CoreError),
    Dab(DabRefusal),
}
impl From<CoreError> for Error {
    fn from(e: CoreError) -> Self {
        Self::Core(e)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) => f.write_str(s),
            Self::ModelMismatch => {
                f.write_str("モデルの三角形・UV・スロットの指紋が、パスを作ったときと違います")
            }
            Self::MissingTriangle => f.write_str("パスが参照する三角形がありません"),
            Self::TooManySamples => f.write_str("ブラシの間隔に対してパスが長すぎます"),
            Self::Canceled => f.write_str("パスの評価を取り消しました"),
            Self::Core(e) => e.fmt(f),
            Self::Dab(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasPoint {
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
}
impl CanvasPoint {
    pub fn new(x: f64, y: f64, pressure: f64) -> Result<Self, Error> {
        let p = Self { x, y, pressure };
        p.validate()?;
        Ok(p)
    }
    fn validate(self) -> Result<(), Error> {
        if !self.x.is_finite()
            || !self.y.is_finite()
            || self.x.abs() > 1e6
            || self.y.abs() > 1e6
            || !(0.0..=1.0).contains(&self.pressure)
        {
            return Err(Error::Invalid(
                "2D の点は有限の ±1000000 画素、筆圧は 0..1 です",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathPoint {
    pub triangle: u32,
    pub u: f64,
    pub v: f64,
    pub pressure: f64,
}
impl PathPoint {
    /// 重心座標の `-1e-9` 以上 `0` 未満は 0 に丸める。直に組んだ値は丸められず、`SurfacePath::validate` が断る。
    pub fn new(triangle: u32, u: f64, v: f64, pressure: f64) -> Result<Self, Error> {
        if triangle > i32::MAX as u32
            || !u.is_finite()
            || !v.is_finite()
            || u < -1e-9
            || v < -1e-9
            || u + v > 1.0 + 1e-9
            || !(0.0..=1.0).contains(&pressure)
        {
            return Err(Error::Invalid("3D の点は三角形の内側、筆圧は 0..1 です"));
        }
        Ok(Self {
            triangle,
            u: u.max(0.0),
            v: v.max(0.0),
            pressure,
        })
    }
}
/// 半径は 2D では画素、3D ではモデルの空間。ほかの欄は通常の丸いブラシと共通。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathBrush(pub BrushSettings);
impl Default for PathBrush {
    fn default() -> Self {
        Self(BrushSettings {
            radius: 0.01,
            ..Default::default()
        })
    }
}
impl PathBrush {
    fn validate(self, canvas: bool) -> Result<(), Error> {
        let b = self.0;
        if !b.radius.is_finite() || b.radius <= 0.0 || b.radius > if canvas { 4096.0 } else { 1e6 }
        {
            return Err(Error::Invalid("パスのブラシの半径が範囲外です"));
        }
        BrushSettings { radius: 1.0, ..b }.validate()?;
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelPaint {
    pub channel: Channel,
    pub color: Rgba8,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasPath {
    pub id: u128,
    pub channel: Channel,
    pub brush: PathBrush,
    pub points: Vec<CanvasPoint>,
    pub material: Option<Vec<ChannelPaint>>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SurfacePath {
    pub id: u128,
    pub channel: Channel,
    pub brush: PathBrush,
    pub points: Vec<PathPoint>,
    pub model_fingerprint: String,
    pub material: Option<Vec<ChannelPaint>>,
}
fn paints(
    channel: Channel,
    brush: PathBrush,
    material: &Option<Vec<ChannelPaint>>,
) -> Result<Vec<ChannelPaint>, Error> {
    if !channel.is_standard() {
        return Err(Error::Invalid("パスは標準の 6 チャンネルを使います"));
    }
    let fallback = [ChannelPaint {
        channel,
        color: brush.0.color,
    }];
    let p = material.as_deref().unwrap_or(&fallback);
    if p.is_empty() || p.len() > 6 {
        return Err(Error::Invalid("パスの組は 1..6 チャンネルです"));
    }
    let mut seen = 0u64;
    for m in p {
        if !m.channel.is_standard() || seen & (1 << m.channel.index()) != 0 {
            return Err(Error::Invalid("パスのチャンネルが不正か重複しています"));
        }
        seen |= 1 << m.channel.index();
    }
    Ok(p.to_vec())
}
impl CanvasPath {
    pub fn validate(&self) -> Result<(), Error> {
        self.brush.validate(true)?;
        paints(self.channel, self.brush, &self.material)?;
        if self.points.len() > MAX_POINTS {
            return Err(Error::Invalid("パスの点は 4096 個までです"));
        }
        for p in &self.points {
            p.validate()?;
        }
        Ok(())
    }
}
impl SurfacePath {
    pub fn validate(&self) -> Result<(), Error> {
        self.brush.validate(false)?;
        paints(self.channel, self.brush, &self.material)?;
        if self.points.len() > MAX_POINTS {
            return Err(Error::Invalid("パスの点は 4096 個までです"));
        }
        if self.model_fingerprint.is_empty() || self.model_fingerprint.encode_utf16().count() > 128
        {
            return Err(Error::Invalid("モデルの指紋は 1..128 文字です"));
        }
        for p in &self.points {
            let normalized = PathPoint::new(p.triangle, p.u, p.v, p.pressure)?;
            if normalized != *p {
                return Err(Error::Invalid(
                    "重心座標が未正規化です（-1e-9 以上 0 未満は 0 に丸める）",
                ));
            }
        }
        Ok(())
    }
}
