use super::FillError;
use std::f64::consts::PI;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum ProjectionMode {
    #[default]
    Uv = 0,
    Triplanar = 1,
    Planar = 2,
    Spherical = 3,
    Cylindrical = 4,
    Decal = 5,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Wrap {
    #[default]
    Repeat = 0,
    Clamp = 1,
    None = 2,
}
impl TryFrom<u8> for ProjectionMode {
    type Error = FillError;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Uv),
            1 => Ok(Self::Triplanar),
            2 => Ok(Self::Planar),
            3 => Ok(Self::Spherical),
            4 => Ok(Self::Cylindrical),
            5 => Ok(Self::Decal),
            _ => Err(FillError::Invalid("未知の投影")),
        }
    }
}
impl TryFrom<u8> for Wrap {
    type Error = FillError;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Repeat),
            1 => Ok(Self::Clamp),
            2 => Ok(Self::None),
            _ => Err(FillError::Invalid("未知の繰り返し")),
        }
    }
}
/// モデルのルートの空間の箱。角度は度、Unity の Z → X → Y の順。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub center: [f64; 3],
    pub rotation: [f64; 3],
    pub size: [f64; 3],
}
impl Default for Placement {
    fn default() -> Self {
        Self {
            center: [0.; 3],
            rotation: [0.; 3],
            size: [1.; 3],
        }
    }
}
impl Placement {
    fn validate(&self) -> Result<(), FillError> {
        if self
            .center
            .iter()
            .chain(&self.rotation)
            .chain(&self.size)
            .any(|v| !v.is_finite())
        {
            return Err(FillError::Invalid("箱に有限でない値"));
        }
        if self.center.iter().any(|v| v.abs() > 1e6)
            || self.rotation.iter().any(|v| v.abs() > 360.)
            || self.size.iter().any(|v| *v < 1e-6 || *v > 1e6)
        {
            return Err(FillError::Invalid("箱の位置・回転・大きさの範囲"));
        }
        Ok(())
    }
    pub(crate) fn matrix(&self) -> [f64; 9] {
        let [a, b, c] = self.rotation.map(|v| v * (PI / 180.));
        let (ca, sa, cb, sb, cc, sc) = (a.cos(), a.sin(), b.cos(), b.sin(), c.cos(), c.sin());
        [
            cb * cc + sb * sa * sc,
            -cb * sc + sb * sa * cc,
            sb * ca,
            ca * sc,
            ca * cc,
            -sa,
            -sb * cc + cb * sa * sc,
            sb * sc + cb * sa * cc,
            cb * ca,
        ]
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projection {
    pub mode: ProjectionMode,
    pub wrap: Wrap,
    pub tiles: [f64; 2],
    pub offset: [f64; 2],
    pub rotation: f64,
    pub blend_width: f64,
    pub placement: Placement,
    pub depth_hardness: f64,
    pub backface_angle: f64,
    pub backface_hardness: f64,
}
impl Default for Projection {
    fn default() -> Self {
        Self {
            mode: ProjectionMode::Uv,
            wrap: Wrap::Repeat,
            tiles: [1.; 2],
            offset: [0.; 2],
            rotation: 0.,
            blend_width: 0.3,
            placement: Placement::default(),
            depth_hardness: 0.8,
            backface_angle: 90.,
            backface_hardness: 0.8,
        }
    }
}
impl Projection {
    pub const ALGORITHM_VERSION: u32 = 1;
    pub fn validate(&self) -> Result<(), FillError> {
        let s = self;
        if s.tiles
            .iter()
            .chain(&s.offset)
            .chain([
                &s.rotation,
                &s.blend_width,
                &s.depth_hardness,
                &s.backface_angle,
                &s.backface_hardness,
            ])
            .any(|v| !v.is_finite())
        {
            return Err(FillError::Invalid("投影に有限でない値"));
        }
        if s.tiles.iter().any(|v| *v < 1e-3 || *v > 1e4) {
            return Err(FillError::Invalid("繰り返しは 0.001..10000"));
        }
        if s.offset.iter().any(|v| v.abs() > 1e4) || s.rotation.abs() > 360. {
            return Err(FillError::Invalid("オフセット・回転の範囲"));
        }
        if [s.blend_width, s.depth_hardness, s.backface_hardness]
            .iter()
            .any(|v| *v < 0. || *v > 1.)
            || s.backface_angle < 0.
            || s.backface_angle > 180.
        {
            return Err(FillError::Invalid("混ぜ幅・減衰の範囲"));
        }
        if s.mode != ProjectionMode::Decal
            && (s.depth_hardness != 0.8 || s.backface_angle != 90. || s.backface_hardness != 0.8)
        {
            return Err(FillError::Invalid("減衰はデカールだけ"));
        }
        s.placement.validate()
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelFrame {
    pub position: [f64; 3],
    pub rotation: [f64; 4],
}
impl Default for ModelFrame {
    fn default() -> Self {
        Self {
            position: [0.; 3],
            rotation: [0., 0., 0., 1.],
        }
    }
}
impl ModelFrame {
    pub(crate) fn matrix(&self) -> Result<[f64; 9], FillError> {
        if self
            .position
            .iter()
            .chain(&self.rotation)
            .any(|v| !v.is_finite())
        {
            return Err(FillError::Invalid("モデルフレームに有限でない値"));
        }
        let [x, y, z, w] = self.rotation;
        let length = (x * x + y * y + z * z + w * w).sqrt();
        if length <= 1e-9 || !length.is_finite() {
            return Err(FillError::Invalid("モデルの回転の長さ"));
        }
        let (x, y, z, w) = (x / length, y / length, z / length, w / length);
        Ok([
            1. - 2. * (y * y + z * z),
            2. * (x * y - z * w),
            2. * (x * z + y * w),
            2. * (x * y + z * w),
            1. - 2. * (x * x + z * z),
            2. * (y * z - x * w),
            2. * (x * z - y * w),
            2. * (y * z + x * w),
            1. - 2. * (x * x + y * y),
        ])
    }
}
