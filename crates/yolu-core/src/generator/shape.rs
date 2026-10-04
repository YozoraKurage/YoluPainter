//! ワールドの Position マップから、モデルのルートに置いた形へ変換する。
use super::{unit, Error};
use crate::math::clamp01;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Shape {
    Box = 0,
    Sphere = 1,
    Plane = 2,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    pub shape: Shape,
    pub center: [f64; 3],
    pub rotation: [f64; 3],
    pub size: [f64; 3],
    pub falloff: f64,
}
impl Default for Volume {
    fn default() -> Self {
        Self {
            shape: Shape::Box,
            center: [0.; 3],
            rotation: [0.; 3],
            size: [1.; 3],
            falloff: 0.5,
        }
    }
}
impl Volume {
    pub fn validate(&self) -> Result<(), Error> {
        if self.center.iter().any(|x| !x.is_finite() || x.abs() > 1e6)
            || self
                .rotation
                .iter()
                .any(|x| !x.is_finite() || x.abs() > 360.)
            || self
                .size
                .iter()
                .any(|x| !x.is_finite() || !(1e-6..=1e6).contains(x))
            || !unit(self.falloff)
        {
            Err(Error::Invalid("形の中心・角度・大きさ・減衰が範囲外です"))
        } else {
            Ok(())
        }
    }
    pub fn rotation_matrix(&self) -> [f64; 9] {
        let k = std::f64::consts::PI / 180.;
        let [a, b, c] = self.rotation.map(|v| v * k);
        let (ca, sa) = (a.cos(), a.sin());
        let (cb, sb) = (b.cos(), b.sin());
        let (cc, sc) = (c.cos(), c.sin());
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
    pub fn value_at(&self, p: [f64; 3]) -> Result<f64, Error> {
        self.validate()?;
        if p.iter().any(|x| !x.is_finite()) {
            return Err(Error::Invalid("形の入力は有限値が必要です"));
        }
        let r = self.rotation_matrix();
        let p = std::array::from_fn::<_, 3, _>(|i| p[i] - self.center[i]);
        Ok(self.local_value([
            r[0] * p[0] + r[3] * p[1] + r[6] * p[2],
            r[1] * p[0] + r[4] * p[1] + r[7] * p[2],
            r[2] * p[0] + r[5] * p[1] + r[8] * p[2],
        ]))
    }
    pub(super) fn local_value(&self, [x, y, z]: [f64; 3]) -> f64 {
        let [hx, hy, hz] = self.size.map(|s| s / 2.);
        let band = if self.shape == Shape::Sphere {
            self.falloff * hx
        } else {
            self.falloff * hx.min(hy.min(hz))
        };
        let d = match self.shape {
            Shape::Box => (hx - x.abs()).min(hy - y.abs()).min(hz - z.abs()),
            Shape::Sphere => hx - (x * x + y * y + z * z).sqrt(),
            Shape::Plane => return clamp01(0.5 + y * (1. / self.size[1])),
        };
        if d <= 0. {
            0.
        } else if band <= 0. || d >= band {
            1.
        } else {
            d / band
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelFrame {
    position: [f64; 3],
    rotation: [f64; 4],
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
    pub fn new(position: [f64; 3], rotation: [f64; 4]) -> Result<Self, Error> {
        let [x, y, z, w] = rotation;
        let len = (x * x + y * y + z * z + w * w).sqrt();
        if position
            .iter()
            .chain(rotation.iter())
            .any(|v| !v.is_finite())
            || !len.is_finite()
            || len <= 1e-9
        {
            return Err(Error::Invalid("モデルの位置・回転が不正です"));
        }
        Ok(Self {
            position,
            rotation: rotation.map(|v| v / len),
        })
    }
    pub fn position(&self) -> [f64; 3] {
        self.position
    }
    pub fn rotation_matrix(&self) -> [f64; 9] {
        let [x, y, z, w] = self.rotation;
        [
            1. - 2. * (y * y + z * z),
            2. * (x * y - z * w),
            2. * (x * z + y * w),
            2. * (x * y + z * w),
            1. - 2. * (x * x + z * z),
            2. * (y * z - x * w),
            2. * (x * z - y * w),
            2. * (y * z + x * w),
            1. - 2. * (x * x + y * y),
        ]
    }
}
