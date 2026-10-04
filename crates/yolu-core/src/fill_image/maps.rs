use super::{budget, canceled, dimensions, zeroes, FillError};
use crate::geometry::{unity::mix3, uv_barycentric, SurfaceGeometry};
use glam::Vec2;
use std::sync::atomic::AtomicBool;

/// C# BakedMeshMap の 16 bit RGB と覆い。法線は n/2+1/2、位置は bounds 内で正規化。
#[derive(Clone, Copy)]
pub struct Map<'a> {
    pub width: u32,
    pub height: u32,
    pub values: &'a [u16],
    pub coverage: &'a [u8],
}
impl Map<'_> {
    pub(crate) fn validate(&self) -> Result<(), FillError> {
        let n = dimensions(self.width, self.height)?;
        if self.values.len() != n * 3 || self.coverage.len() != n {
            return Err(FillError::Invalid("マップの要素数"));
        }
        Ok(())
    }
}
/// geometry の三角形の面法線と UV 重心座標で作るマップ。重複 UV は入力順で最初の面。
/// スムーズ法線・余白埋め・アンチエイリアスを行う C# の完全なベイカーの代替ではない。
pub struct ModelMaps {
    width: u32,
    height: u32,
    positions: Vec<u16>,
    normals: Vec<u16>,
    coverage: Vec<u8>,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub revision: u32,
}
impl ModelMaps {
    /// 予算はマップ 13 バイト/画素と、UV の三角形照合数。取消は行ごとに見る。
    pub fn from_geometry(
        geometry: &SurfaceGeometry,
        width: u32,
        height: u32,
        material: i32,
        memory_budget: u64,
        work_budget: u64,
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, FillError> {
        let n = dimensions(width, height)?;
        budget(n as u64 * 13, memory_budget)?;
        canceled(cancel)?;
        let bounds = geometry.bounds();
        let min = bounds.min().to_array().map(f64::from);
        let max = bounds.max().to_array().map(f64::from);
        let mut result = Self {
            width,
            height,
            positions: zeroes(n * 3)?,
            normals: zeroes(n * 3)?,
            coverage: zeroes(n)?,
            bounds_min: min,
            bounds_max: max,
            revision: geometry.revision(),
        };
        let mut work = 0u64;
        for t in geometry
            .triangles()
            .iter()
            .filter(|t| t.material == material)
        {
            canceled(cancel)?;
            let us = [t.uv_a, t.uv_b, t.uv_c];
            let lo = us[0].min(us[1]).min(us[2]);
            let hi = us[0].max(us[1]).max(us[2]);
            // geometry の境界の許し 1e-6 を取りこぼさない。
            let x0 = ((lo.x as f64 * width as f64 - 1.).floor().max(0.) as u32).min(width);
            let y0 = ((lo.y as f64 * height as f64 - 1.).floor().max(0.) as u32).min(height);
            let x1 = ((hi.x as f64 * width as f64 + 1.).ceil().max(0.) as u32).min(width);
            let y1 = ((hi.y as f64 * height as f64 + 1.).ceil().max(0.) as u32).min(height);
            let normal = t.normal().to_array();
            for y in y0..y1 {
                canceled(cancel)?;
                for x in x0..x1 {
                    let i = (y * width + x) as usize;
                    if result.coverage[i] != 0 {
                        continue;
                    }
                    work = work
                        .checked_add(1)
                        .ok_or(FillError::Invalid("作業数の桁あふれ"))?;
                    budget(work, work_budget)?;
                    let uv = Vec2::new(
                        (x as f32 + 0.5) / width as f32,
                        (y as f32 + 0.5) / height as f32,
                    );
                    if let Some(bary) = uv_barycentric(uv, t) {
                        let p = mix3(t.a, t.b, t.c, bary).to_array();
                        result.coverage[i] = 1;
                        for c in 0..3 {
                            let extent = max[c] - min[c];
                            let v = if extent > 0. {
                                (p[c] as f64 - min[c]) / extent
                            } else {
                                0.5
                            };
                            result.positions[i * 3 + c] = quantize(v);
                            result.normals[i * 3 + c] = quantize(normal[c] as f64 * 0.5 + 0.5);
                        }
                    }
                }
            }
        }
        canceled(cancel)?;
        Ok(result)
    }
    pub fn positions(&self) -> Map<'_> {
        Map {
            width: self.width,
            height: self.height,
            values: &self.positions,
            coverage: &self.coverage,
        }
    }
    pub fn normals(&self) -> Map<'_> {
        Map {
            width: self.width,
            height: self.height,
            values: &self.normals,
            coverage: &self.coverage,
        }
    }
}
fn quantize(v: f64) -> u16 {
    (v * 65535. + 0.5).clamp(0., 65535.) as u16
}
