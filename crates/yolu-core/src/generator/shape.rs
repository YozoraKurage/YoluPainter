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
    pub(super) fn local_value(&self, p: [f64; 3]) -> f64 {
        self.local().value(p)
    }
    /// 形の座標での値の式を、点ごとに変わらない量（半分の大きさ・減衰の幅）を先に出した形で返す。
    pub(super) fn local(&self) -> Local {
        let [hx, hy, hz] = self.size.map(|s| s / 2.);
        Local {
            shape: self.shape,
            half: [hx, hy, hz],
            band: if self.shape == Shape::Sphere {
                self.falloff * hx
            } else {
                self.falloff * hx.min(hy.min(hz))
            },
            plane_scale: 1. / self.size[1],
        }
    }
}
/// `Volume::local` の結果。`value` は `Volume::local_value` と同じ式。
#[derive(Clone, Copy)]
pub(super) struct Local {
    shape: Shape,
    half: [f64; 3],
    band: f64,
    plane_scale: f64,
}
/// `Local::eval` の形の指定（`Shape` の番号）。
pub(super) const BOX: u8 = Shape::Box as u8;
pub(super) const SPHERE: u8 = Shape::Sphere as u8;
pub(super) const PLANE: u8 = Shape::Plane as u8;
impl Local {
    pub(super) fn shape(&self) -> Shape {
        self.shape
    }
    pub(super) fn value(&self, p: [f64; 3]) -> f64 {
        match self.shape {
            Shape::Box => self.eval::<BOX>(p),
            Shape::Sphere => self.eval::<SPHERE>(p),
            Shape::Plane => self.eval::<PLANE>(p),
        }
    }
    /// 形が決まっている版（行のループの外で形を選ぶ）。
    #[inline(always)]
    pub(super) fn eval<const SHAPE: u8>(&self, [x, y, z]: [f64; 3]) -> f64 {
        let [hx, hy, hz] = self.half;
        let d = match SHAPE {
            BOX => (hx - x.abs()).min(hy - y.abs()).min(hz - z.abs()),
            SPHERE => hx - (x * x + y * y + z * z).sqrt(),
            _ => return clamp01(0.5 + y * self.plane_scale),
        };
        let band = self.band;
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
