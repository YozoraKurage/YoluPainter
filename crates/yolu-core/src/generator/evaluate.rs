use super::*;
use crate::{
    math::{clamp01, to_byte, UNIT},
    Rect, Rgba8,
};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Color,
    Scalar,
    Mask,
}
/// 1 画素の Generator の出力。`Scalar` は有限の 0..1。`Mapped` はランプ評価済みの straight RGBA8（[r, g, b, a]）。
/// rsfilter の `filter::Generated` と同じ形なので、`GeneratorInput` の実装は `BoundGenerator::sample` の結果をそのまま返せる。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Generated {
    Scalar(f64),
    Mapped([u8; 4]),
}
/// 入力は外部所有。予算は返却する RGBA8 のみ（作業用画素バッファは持たない）。
pub struct Options<'a> {
    pub budget_bytes: u64,
    pub cancel: Option<&'a AtomicBool>,
}
impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            budget_bytes: 256 * 1024 * 1024,
            cancel: None,
        }
    }
}
impl Options<'_> {
    pub(super) fn check(&self) -> Result<(), Error> {
        if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
pub struct Output {
    pub pixels: Vec<u8>,
    pub inactive: Option<Inactive>,
}
/// 検査済みの入力と前計算。設定と画像を借用するため、評価中には変更できない。
pub struct BoundGenerator<'a> {
    g: &'a Settings,
    width: u32,
    height: u32,
    maps: [Option<&'a Map<'a>>; 10],
    anchor: Option<&'a dyn anchor::ValueSource>,
    inactive: Option<Inactive>,
    direction: [f64; 3],
    noise_scale: [f64; 3],
    seeds: [u32; 4],
    matrix: [f64; 9],
    offset: [f64; 3],
}
impl<'a> BoundGenerator<'a> {
    pub fn bind(
        g: &'a Settings,
        maps: &'a [Map<'a>],
        frame: Option<ModelFrame>,
        dimensions: (u32, u32),
        anchor: Result<&'a dyn anchor::ValueSource, anchor::Issue>,
    ) -> Result<Self, Error> {
        g.validate()?;
        let (width, height) = dimensions;
        if width == 0 || height == 0 {
            return Err(Error::Invalid("Generator の大きさは正数が必要です"));
        }
        let mut table = [None; 10];
        for map in maps {
            map.validate()?;
            if table[map.kind as usize].replace(map).is_some() {
                return Err(Error::Invalid("マップの種類が重複しています"));
            }
        }
        let mut inactive = None;
        for kind in g.used_maps() {
            let why = match table[kind as usize] {
                None => Some(Inactive::MissingMap(kind)),
                Some(m) => {
                    if m.state == MapState::Stale {
                        Some(Inactive::StaleMap(kind))
                    } else if m.state == MapState::Unverified {
                        Some(Inactive::UnverifiedMap(kind))
                    } else if (m.width, m.height) != (width, height) {
                        Some(Inactive::MapSize(kind))
                    } else if g.pins.get(&kind).is_some_and(|p| p != m.condition_key) {
                        Some(Inactive::PinMismatch(kind))
                    } else {
                        None
                    }
                }
            };
            if why.is_some() {
                inactive = why;
                break;
            }
        }
        if inactive.is_none() {
            inactive = match g.kind {
                Kind::ShapeGradient if frame.is_none() => Some(Inactive::MissingFrame),
                Kind::IdColor if g.id_colors.is_empty() => Some(Inactive::NoIdColors),
                Kind::Anchor => anchor.as_ref().err().map(|e| Inactive::Anchor(e.clone())),
                _ => None,
            };
        }
        if g.kind == Kind::Anchor {
            if let Ok(a) = anchor {
                if a.dimensions() != (width, height) {
                    return Err(Error::Invalid("Anchor と Generator の画像サイズが違います"));
                }
            }
        }
        let [x, y, z] = g.direction;
        let len = (x * x + y * y + z * z).sqrt();
        let mut b = Self {
            g,
            width,
            height,
            maps: table,
            anchor: anchor.ok(),
            inactive,
            direction: [x / len, y / len, z / len],
            noise_scale: [0.; 3],
            seeds: noise::seeds(g.noise_seed),
            matrix: [0.; 9],
            offset: [0.; 3],
        };
        if b.inactive.is_some() {
            return Ok(b);
        }
        if g.noise_amount > 0. && g.noise_space == NoiseSpace::Model {
            let p = b.map(MapKind::Position);
            let e = std::array::from_fn::<_, 3, _>(|i| p.bounds_max[i] - p.bounds_min[i]);
            let diag = (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt();
            if diag <= 0. {
                b.inactive = Some(Inactive::EmptyBounds);
                return Ok(b);
            }
            if !diag.is_finite() {
                return Err(Error::Invalid("Position の境界箱が大きすぎます"));
            }
            b.noise_scale = e.map(|e| e / diag / g.noise_scale / 65535.);
        }
        if g.kind == Kind::ShapeGradient {
            let p = b.map(MapKind::Position);
            let frame = frame.unwrap();
            let r0 = frame.rotation_matrix();
            let rs = g.volume.rotation_matrix();
            let mut a = [0.; 9];
            for i in 0..3 {
                for j in 0..3 {
                    a[i * 3 + j] =
                        rs[i] * r0[j * 3] + rs[3 + i] * r0[j * 3 + 1] + rs[6 + i] * r0[j * 3 + 2];
                }
            }
            let t = frame.position();
            let root: [f64; 3] = std::array::from_fn(|k| {
                r0[k] * t[0] + r0[3 + k] * t[1] + r0[6 + k] * t[2] + g.volume.center[k]
            });
            for i in 0..3 {
                let mut offset = 0.;
                for j in 0..3 {
                    b.matrix[i * 3 + j] =
                        a[i * 3 + j] * ((p.bounds_max[j] - p.bounds_min[j]) / 65535.);
                    offset += a[i * 3 + j] * p.bounds_min[j];
                }
                b.offset[i] =
                    offset - (rs[i] * root[0] + rs[3 + i] * root[1] + rs[6 + i] * root[2]);
            }
            if b.matrix
                .iter()
                .chain(b.offset.iter())
                .any(|v| !v.is_finite())
            {
                return Err(Error::Invalid("形の座標変換が有限値を超えます"));
            }
        }
        Ok(b)
    }
    fn map(&self, k: MapKind) -> &'a Map<'a> {
        self.maps[k as usize].unwrap()
    }
    pub fn inactive(&self) -> Option<&Inactive> {
        self.inactive.as_ref()
    }
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn value(&self, x: u32, y: u32) -> Option<f64> {
        if x >= self.width || y >= self.height || self.inactive.is_some() {
            return None;
        }
        let i = y as usize * self.width as usize + x as usize;
        let g = self.g;
        let scalar = |k| {
            let m = self.map(k);
            if m.coverage[i] == 0 {
                None
            } else {
                Some(m.data[i] as f64 / 65535.)
            }
        };
        let vector = |k| {
            let m = self.map(k);
            if m.coverage[i] == 0 {
                None
            } else {
                Some([
                    m.data[i * 3] as f64,
                    m.data[i * 3 + 1] as f64,
                    m.data[i * 3 + 2] as f64,
                ])
            }
        };
        let base = match g.kind {
            Kind::EdgeWear => clamp01(2. * (scalar(MapKind::Curvature)? - 0.5)),
            Kind::Dirt => {
                let mut b = 0.;
                if g.balance < 1. {
                    b += (1. - g.balance) * (1. - scalar(MapKind::AmbientOcclusion)?);
                }
                if g.balance > 0. {
                    b += g.balance * clamp01(2. * (0.5 - scalar(MapKind::Curvature)?));
                }
                b
            }
            Kind::PositionGradient => vector(MapKind::Position)?[g.axis] / 65535.,
            Kind::Thickness => scalar(MapKind::Thickness)?,
            Kind::Direction => {
                let [nx, ny, nz] = vector(if g.use_bent_normal {
                    MapKind::BentNormal
                } else {
                    MapKind::WorldNormal
                })?
                .map(|v| v / 65535. * 2. - 1.);
                let len = (nx * nx + ny * ny + nz * nz).sqrt();
                let [dx, dy, dz] = self.direction;
                let dot = if len > 1e-9 {
                    (nx * dx + ny * dy + nz * dz) / len
                } else {
                    0.
                };
                clamp01((dot + 1.) * 0.5)
            }
            Kind::IdColor => {
                let rgb = vector(MapKind::Id)?.map(|v| ((v as u32 + 128) / 257) as i32);
                if g.id_colors.iter().any(|c| {
                    [
                        ((*c >> 16) & 255) as i32,
                        ((*c >> 8) & 255) as i32,
                        (*c & 255) as i32,
                    ]
                    .iter()
                    .zip(rgb)
                    .all(|(c, v)| (c - v).abs() <= g.id_tolerance as i32)
                }) {
                    1.
                } else {
                    0.
                }
            }
            Kind::Anchor => self.anchor?.value(x, y)?,
            Kind::ShapeGradient => {
                let [x, y, z] = vector(MapKind::Position)?;
                let m = self.matrix;
                let k = self.offset;
                g.volume.local_value([
                    m[0] * x + m[1] * y + m[2] * z + k[0],
                    m[3] * x + m[4] * y + m[5] * z + k[1],
                    m[6] * x + m[7] * y + m[8] * z + k[2],
                ])
            }
        };
        let mut t = clamp01((base - g.low) / (g.high - g.low));
        if g.softness > 0. {
            t += g.softness * (t * t * (3. - 2. * t) - t);
        }
        if g.invert {
            t = 1. - t;
        }
        if g.noise_amount > 0. {
            let p = if g.noise_space == NoiseSpace::Uv {
                [
                    (x as f64 + 0.5) * (1. / self.width as f64 / g.noise_scale),
                    (y as f64 + 0.5) * (1. / self.height as f64 / g.noise_scale),
                    0.,
                ]
            } else {
                let p = vector(MapKind::Position)?;
                std::array::from_fn(|i| p[i] * self.noise_scale[i])
            };
            let m = clamp01((noise::fractal(p, self.seeds) - 0.3) / 0.4);
            t *= 1. - g.noise_amount * (m * m * (3. - 2. * m));
        }
        Some(t)
    }
    pub fn sample(&self, x: u32, y: u32, scalar: bool) -> Option<Generated> {
        let value = self.value(x, y)?;
        Some(match &self.g.ramp {
            None => Generated::Scalar(value),
            Some(r) => Generated::Mapped(r.evaluate_unchecked(value, scalar).to_array()),
        })
    }
    fn apply(&self, source: Rgba8, x: u32, y: u32, target: Target, strength: f64) -> Rgba8 {
        if target != Target::Mask && source.a == 0 {
            return source;
        }
        let Some(value) = self.sample(x, y, target != Target::Color) else {
            return source;
        };
        let u = |b: u8| UNIT[b as usize];
        if target == Target::Mask {
            let v = match value {
                Generated::Scalar(v) => v,
                Generated::Mapped(p) => u(p[0]) * u(p[3]),
            };
            return Rgba8::new(
                0,
                0,
                0,
                255 - to_byte(combine(self.g.blend, 1. - u(source.a), v, strength)),
            );
        }
        let (rgb, a) = match value {
            Generated::Scalar(v) => ([v; 3], source.a),
            Generated::Mapped(p) => (
                [u(p[0]), u(p[1]), u(p[2])],
                to_byte(u(source.a) * (1. - strength + strength * u(p[3]))),
            ),
        };
        Rgba8::new(
            to_byte(combine(self.g.blend, u(source.r), rgb[0], strength)),
            to_byte(combine(self.g.blend, u(source.g), rgb[1], strength)),
            to_byte(combine(self.g.blend, u(source.b), rgb[2], strength)),
            a,
        )
    }
}
pub fn combine(blend: Blend, s: f64, v: f64, strength: f64) -> f64 {
    let c = match blend {
        Blend::Multiply => s * v,
        Blend::Replace => v,
        Blend::Screen => 1. - (1. - s) * (1. - v),
        Blend::Max => s.max(v),
        Blend::Min => s.min(v),
        Blend::Add => (s + v).min(1.),
        Blend::Subtract => (s - v).max(0.),
    };
    if strength >= 1. {
        c
    } else {
        s + (c - s) * strength
    }
}
pub(super) fn allocate(
    region: Rect,
    dimensions: (u32, u32),
    options: &Options<'_>,
) -> Result<Vec<u8>, Error> {
    options.check()?;
    if region.is_empty()
        || region
            .x
            .checked_add(region.width)
            .is_none_or(|v| v > dimensions.0)
        || region
            .y
            .checked_add(region.height)
            .is_none_or(|v| v > dimensions.1)
    {
        return Err(Error::Invalid("評価領域が画像の範囲外です"));
    }
    let needed = (u64::from(region.width) * u64::from(region.height))
        .checked_mul(4)
        .ok_or(Error::Invalid("評価画像が大きすぎます"))?;
    if needed > options.budget_bytes {
        return Err(Error::Budget {
            needed,
            budget: options.budget_bytes,
        });
    }
    let size = usize::try_from(needed).map_err(|_| Error::Allocation)?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(size)
        .map_err(|_| Error::Allocation)?;
    pixels.resize(size, 0);
    Ok(pixels)
}
pub fn evaluate(
    source: &dyn Source,
    g: &BoundGenerator<'_>,
    region: Rect,
    target: Target,
    strength: f64,
    options: &Options<'_>,
) -> Result<Output, Error> {
    if !unit(strength) || source.dimensions() != g.dimensions() {
        return Err(Error::Invalid(
            "Generator の強さまたは画像の大きさが不正です",
        ));
    }
    let mut pixels = allocate(region, source.dimensions(), options)?;
    pixels
        .par_chunks_mut(region.width as usize * 4)
        .enumerate()
        .try_for_each(|(row, bytes)| -> Result<(), Error> {
            options.check()?;
            let y = region.y + row as u32;
            for (col, dst) in bytes.chunks_exact_mut(4).enumerate() {
                let x = region.x + col as u32;
                let mut src = source.pixel(x, y);
                if target == Target::Mask {
                    src.r = 0;
                    src.g = 0;
                    src.b = 0;
                }
                let p = if strength == 0. {
                    src
                } else {
                    g.apply(src, x, y, target, strength)
                };
                dst.copy_from_slice(&p.to_array());
            }
            Ok(())
        })?;
    options.check()?;
    Ok(Output {
        pixels,
        inactive: g.inactive.clone(),
    })
}
