use super::mip::Acc;
use super::{
    budget, canceled, dimensions, zeroes, FillError, ImageMipChain, Map, ModelFrame, Projection,
    ProjectionMode as Mode, Wrap,
};
use crate::{math::to_byte, Rgba8};
use rayon::prelude::*;
use std::{f64::consts::PI, sync::atomic::AtomicBool};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InactiveReason {
    MissingImage,
    MissingPosition,
    MissingNormal,
    PositionSize,
    NormalSize,
    UnknownModelFrame,
    StalePosition,
    StaleNormal,
}
/// リソース・マップの解決は呼び手が行う。古いマップは stale を立てる。
pub struct FillInput<'a> {
    pub projection: Projection,
    pub width: u32,
    pub height: u32,
    pub image: Option<&'a ImageMipChain<'a>>,
    /// デカールの別チャンネルが形を持つ場合だけ渡す。自身が形なら None。
    pub shape: Option<&'a ImageMipChain<'a>>,
    pub fallback: Rgba8,
    /// デカールの画像が指定されているのに解決できなかった場合。値だけのチャンネルは false。
    pub missing_image: bool,
    pub positions: Option<Map<'a>>,
    pub normals: Option<Map<'a>>,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub frame: Option<ModelFrame>,
    pub stale_position: bool,
    pub stale_normal: bool,
}
impl Default for FillInput<'_> {
    fn default() -> Self {
        Self {
            projection: Projection::default(),
            width: 1,
            height: 1,
            image: None,
            shape: None,
            fallback: Rgba8::new(255, 255, 255, 255),
            missing_image: false,
            positions: None,
            normals: None,
            bounds_min: [0.; 3],
            bounds_max: [1.; 3],
            frame: Some(ModelFrame::default()),
            stale_position: false,
            stale_normal: false,
        }
    }
}
/// 不変のサンプラー。Rayon の現在のプールで行を並列評価する（順序で画素は変わらない）。
pub struct FillSampler<'a> {
    input: FillInput<'a>,
    reason: Option<InactiveReason>,
    placed: bool,
    a: [f64; 4],
    b: [f64; 2],
    uv: [f64; 6],
    rho: f64,
    m: [f64; 9],
    k: [f64; 3],
    nm: [f64; 9],
    inv: [f64; 3],
}
impl<'a> FillSampler<'a> {
    pub fn bind(input: FillInput<'a>) -> Result<Self, FillError> {
        dimensions(input.width, input.height)?;
        input.projection.validate()?;
        if let Some(m) = input.positions {
            m.validate()?;
        }
        if let Some(m) = input.normals {
            m.validate()?;
        }
        if input
            .bounds_min
            .iter()
            .chain(&input.bounds_max)
            .any(|v| !v.is_finite())
            || (0..3).any(|i| {
                input.bounds_max[i] < input.bounds_min[i]
                    || !(input.bounds_max[i] - input.bounds_min[i]).is_finite()
            })
        {
            return Err(FillError::Invalid("位置マップの箱"));
        }
        let r0 = input.frame.map(|f| f.matrix()).transpose()?;
        let p = input.projection;
        let mut problem = None;
        if p.mode != Mode::Uv {
            problem = if input.stale_position {
                Some(InactiveReason::StalePosition)
            } else {
                match input.positions {
                    None => Some(InactiveReason::MissingPosition),
                    Some(m) if m.width != input.width || m.height != input.height => {
                        Some(InactiveReason::PositionSize)
                    }
                    _ => None,
                }
            };
            if problem.is_none() && matches!(p.mode, Mode::Triplanar | Mode::Decal) {
                problem = if input.stale_normal {
                    Some(InactiveReason::StaleNormal)
                } else {
                    match input.normals {
                        None => Some(InactiveReason::MissingNormal),
                        Some(m) if m.width != input.width || m.height != input.height => {
                            Some(InactiveReason::NormalSize)
                        }
                        _ => None,
                    }
                };
            }
            if problem.is_none() && r0.is_none() {
                problem = Some(InactiveReason::UnknownModelFrame);
            }
        }
        let placed = problem.is_none();
        let reason = problem.or(
            if input.image.is_none() && (p.mode != Mode::Decal || input.missing_image) {
                Some(InactiveReason::MissingImage)
            } else {
                None
            },
        );
        let reference = input.image.or(input.shape);
        let (w, h) = reference.map(|c| c.wh(0)).unwrap_or((1, 1));
        let (w, h) = (w as f64, h as f64);
        let phi = -p.rotation * PI / 180.;
        let (c, sn) = (phi.cos(), phi.sin());
        let a = [
            p.tiles[0] * c,
            -p.tiles[0] * sn,
            p.tiles[1] * sn,
            p.tiles[1] * c,
        ];
        let b = [
            p.tiles[0] * (0.5 - 0.5 * c + 0.5 * sn) + p.offset[0],
            p.tiles[1] * (0.5 - 0.5 * sn - 0.5 * c) + p.offset[1],
        ];
        let uv = [
            w * a[0] / input.width as f64,
            w * a[1] / input.height as f64,
            w * b[0] - 0.5,
            h * a[2] / input.width as f64,
            h * a[3] / input.height as f64,
            h * b[1] - 0.5,
        ];
        let rho = (uv[0] * uv[0] + uv[3] * uv[3])
            .sqrt()
            .max((uv[1] * uv[1] + uv[4] * uv[4]).sqrt());
        let (mut m, mut k, mut nm) = ([0.; 9], [0.; 3], [0.; 9]);
        if p.mode != Mode::Uv && placed {
            let rs = p.placement.matrix();
            let r0 = r0.unwrap();
            let t0 = input.frame.unwrap().position;
            for i in 0..3 {
                for j in 0..3 {
                    nm[i * 3 + j] =
                        rs[i] * r0[j * 3] + rs[3 + i] * r0[j * 3 + 1] + rs[6 + i] * r0[j * 3 + 2];
                }
            }
            let center: [f64; 3] = std::array::from_fn(|q| {
                r0[q] * t0[0] + r0[3 + q] * t0[1] + r0[6 + q] * t0[2] + p.placement.center[q]
            });
            for i in 0..3 {
                let mut offset = 0.;
                for j in 0..3 {
                    m[i * 3 + j] =
                        nm[i * 3 + j] * ((input.bounds_max[j] - input.bounds_min[j]) / 65535.);
                    offset += nm[i * 3 + j] * input.bounds_min[j];
                }
                k[i] = offset - (rs[i] * center[0] + rs[3 + i] * center[1] + rs[6 + i] * center[2]);
            }
            if m.iter().chain(&k).any(|v| !v.is_finite()) {
                return Err(FillError::Invalid("位置変換の桁あふれ"));
            }
        }
        let sampler = Self {
            input,
            reason,
            placed,
            a,
            b,
            uv,
            rho,
            m,
            k,
            nm,
            inv: p.placement.size.map(|v| 1. / v),
        };
        sampler.validate_coordinate_range()?;
        Ok(sampler)
    }
    // C# の double → int が範囲外になる入力は実行環境依存なので、評価の前に明示して断る。
    fn validate_coordinate_range(&self) -> Result<(), FillError> {
        if !self.placed {
            return Ok(());
        }
        let p = self.input.projection;
        let mut bound = [1.0_f64; 2];
        if p.mode != Mode::Uv {
            let local: [f64; 3] = std::array::from_fn(|i| {
                self.m[i * 3].abs() * 65535.
                    + self.m[i * 3 + 1].abs() * 65535.
                    + self.m[i * 3 + 2].abs() * 65535.
                    + self.k[i].abs()
            });
            let scaled: [f64; 3] = std::array::from_fn(|i| local[i] * self.inv[i] + 0.5);
            bound = match p.mode {
                Mode::Planar | Mode::Decal => [scaled[0], scaled[1]],
                Mode::Spherical => [1.; 2],
                Mode::Cylindrical => [1., scaled[1]],
                _ => [scaled[0].max(scaled[2]), scaled[1].max(scaled[2])],
            };
            if local.iter().any(|v| !v.is_finite() || *v > 1e150) {
                return Err(FillError::Invalid("モデル位置の計算範囲"));
            }
        }
        for chain in [self.input.image, self.input.shape].into_iter().flatten() {
            let (w, h) = chain.wh(0);
            for (axis, size) in [w, h].into_iter().enumerate() {
                let reach = (self.a[axis * 2].abs() * bound[0]
                    + self.a[axis * 2 + 1].abs() * bound[1]
                    + self.b[axis].abs())
                    * size as f64
                    + 0.5;
                if !reach.is_finite() || reach > (i32::MAX - 2) as f64 {
                    return Err(FillError::Invalid(
                        "投影のテクセル座標が32 bit整数の範囲を超えます",
                    ));
                }
            }
        }
        Ok(())
    }
    pub fn reason(&self) -> Option<InactiveReason> {
        self.reason
    }
    pub fn placed(&self) -> bool {
        self.placed
    }
    /// 任意のタイル・領域を返す。予算は返す RGBA8 のバイト数。失敗時に途中の画像は公開しない。
    pub fn render(
        &self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        limit: u64,
        cancel: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, FillError> {
        self.render_with(x, y, width, height, limit, cancel, |col, row| {
            self.pixel_inner(x as usize + col, y as usize + row)
        })
    }
    /// C# `ApplyDecalToValue` の領域版。デカールの被覆（箱・奥行き・面の向き・別チャンネルの形）を、
    /// 呼び手が画素ごとに決めた値（グラデーションのランプの結果など）にかける。`values` は領域と同じ並びの
    /// straight RGBA8（左下から行優先、`width × height × 4` バイト）。自身の画像は使わず（C# と同じ）、
    /// 置けないデカールは透明。デカール以外の投影は拒否する。予算・取消・失敗時は `render` と同じ。
    #[allow(clippy::too_many_arguments)]
    pub fn render_decal_values(
        &self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        values: &[u8],
        limit: u64,
        cancel: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, FillError> {
        self.require_decal()?;
        let n = dimensions(width, height)?;
        if values.len() != n * 4 {
            return Err(FillError::Invalid("値のバイト数"));
        }
        self.render_with(x, y, width, height, limit, cancel, |col, row| {
            let o = (row * width as usize + col) * 4;
            self.decal_value_inner(
                x as usize + col,
                y as usize + row,
                Rgba8::from_slice(&values[o..o + 4]),
            )
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn render_with(
        &self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        limit: u64,
        cancel: Option<&AtomicBool>,
        pixel: impl Fn(usize, usize) -> Rgba8 + Sync,
    ) -> Result<Vec<u8>, FillError> {
        let n = dimensions(width, height)?;
        if x.checked_add(width).is_none_or(|v| v > self.input.width)
            || y.checked_add(height).is_none_or(|v| v > self.input.height)
        {
            return Err(FillError::Invalid("出力領域"));
        }
        budget(n as u64 * 4, limit)?;
        canceled(cancel)?;
        let mut result = zeroes(n * 4)?;
        result
            .par_chunks_mut(width as usize * 4)
            .enumerate()
            .try_for_each(|(row, buf)| -> Result<(), FillError> {
                canceled(cancel)?;
                for (col, out) in buf.chunks_exact_mut(4).enumerate() {
                    if col % 256 == 0 {
                        canceled(cancel)?;
                    }
                    out.copy_from_slice(&pixel(col, row).to_array());
                }
                Ok(())
            })?;
        canceled(cancel)?;
        Ok(result)
    }
    pub fn pixel(&self, x: u32, y: u32) -> Result<Rgba8, FillError> {
        if x >= self.input.width || y >= self.input.height {
            return Err(FillError::Invalid("画素の座標"));
        }
        Ok(self.pixel_inner(x as usize, y as usize))
    }
    /// C# `ApplyDecalToValue(x, y, value)`。デカールの 1 画素に、画素ごとの値へ被覆をかけた結果を返す。
    /// 自身の画像は使わず、別チャンネルの形があればそのアルファも掛ける。置けないデカールは透明。デカール以外は拒否。
    pub fn apply_decal_to_value(&self, x: u32, y: u32, value: Rgba8) -> Result<Rgba8, FillError> {
        self.require_decal()?;
        if x >= self.input.width || y >= self.input.height {
            return Err(FillError::Invalid("画素の座標"));
        }
        Ok(self.decal_value_inner(x as usize, y as usize, value))
    }
    fn require_decal(&self) -> Result<(), FillError> {
        if self.input.projection.mode == Mode::Decal {
            Ok(())
        } else {
            Err(FillError::Invalid("画素ごとの値はデカールの投影だけ"))
        }
    }
    fn decal_value_inner(&self, x: usize, y: usize, value: Rgba8) -> Rgba8 {
        if self.placed {
            self.decal_pixel(x, y, Some(value))
        } else {
            Rgba8::TRANSPARENT
        }
    }
    pub fn decal_coverage(&self, x: u32, y: u32) -> Result<f64, FillError> {
        if x >= self.input.width || y >= self.input.height {
            return Err(FillError::Invalid("画素の座標"));
        }
        if self.input.projection.mode != Mode::Decal || !self.placed {
            return Ok(0.);
        }
        let i = y as usize * self.input.width as usize + x as usize;
        Ok(self.decal_point(i).map_or(0., |(_, c)| c))
    }
    fn point(&self, i: usize) -> [f64; 3] {
        let v = self.input.positions.unwrap().values;
        let (px, py, pz) = (v[i * 3] as f64, v[i * 3 + 1] as f64, v[i * 3 + 2] as f64);
        std::array::from_fn(|j| {
            self.m[j * 3] * px + self.m[j * 3 + 1] * py + self.m[j * 3 + 2] * pz + self.k[j]
        })
    }
    fn normal(&self, i: usize) -> [f64; 3] {
        let v = self.input.normals.unwrap().values;
        let (x, y, z) = (
            v[i * 3] as f64 / 65535. * 2. - 1.,
            v[i * 3 + 1] as f64 / 65535. * 2. - 1.,
            v[i * 3 + 2] as f64 / 65535. * 2. - 1.,
        );
        std::array::from_fn(|j| {
            self.nm[j * 3] * x + self.nm[j * 3 + 1] * y + self.nm[j * 3 + 2] * z
        })
    }
    fn neighbor(
        &self,
        x: usize,
        y: usize,
        dx: i64,
        dy: i64,
        p: [f64; 3],
    ) -> Option<([f64; 3], f64)> {
        let mut best = None;
        let mut distance = f64::INFINITY;
        for side in [1, -1] {
            let nx = x as i64 + dx * side;
            let ny = y as i64 + dy * side;
            if nx < 0 || ny < 0 || nx >= self.input.width as i64 || ny >= self.input.height as i64 {
                continue;
            }
            let i = ny as usize * self.input.width as usize + nx as usize;
            if self.input.positions.unwrap().coverage[i] == 0 {
                continue;
            }
            let q = self.point(i);
            let d = (q[0] - p[0]) * (q[0] - p[0])
                + (q[1] - p[1]) * (q[1] - p[1])
                + (q[2] - p[2]) * (q[2] - p[2]);
            if d < distance {
                distance = d;
                best = Some((q, side as f64));
            }
        }
        best
    }
    fn project(&self, p: [f64; 3], axis: usize, positive: bool) -> [f64; 2] {
        let [x, y, z] = p;
        let [sx, sy, sz] = self.inv;
        match self.input.projection.mode {
            Mode::Planar | Mode::Decal => [x * sx + 0.5, y * sy + 0.5],
            Mode::Spherical => {
                let r = (x * x + y * y + z * z).sqrt();
                if r <= 0. {
                    return [0.5; 2];
                }
                [
                    x.atan2(-z) * (1. / (2. * PI)) + 0.5,
                    (y / r).clamp(-1., 1.).asin() * (1. / PI) + 0.5,
                ]
            }
            Mode::Cylindrical => [x.atan2(-z) * (1. / (2. * PI)) + 0.5, y * sy + 0.5],
            _ => match axis {
                0 => [(if positive { z } else { -z }) * sz + 0.5, y * sy + 0.5],
                1 => [(if positive { x } else { -x }) * sx + 0.5, z * sz + 0.5],
                _ => [(if positive { -x } else { x }) * sx + 0.5, y * sy + 0.5],
            },
        }
    }
    fn differences(
        &self,
        st: [f64; 2],
        n: Option<([f64; 3], f64)>,
        axis: usize,
        positive: bool,
    ) -> [f64; 2] {
        let Some((p, sign)) = n else { return [0.; 2] };
        let q = self.project(p, axis, positive);
        let mut ds = q[0] - st[0];
        if matches!(
            self.input.projection.mode,
            Mode::Spherical | Mode::Cylindrical
        ) {
            ds -= ds.round_ties_even();
        }
        [ds * sign, sign * (q[1] - st[1])]
    }
    /// 画像を投影した画素。画像の値が無い所（代わりの値 `fallback` を見せる所: 使えない入力・位置や法線の無い画素・向きの定まらない画素・
    /// 外側が透明の画像の外）は None。デカールは扱わない（None）。`pixel` はこれに代わりの値を当てたもの。
    pub fn projected(&self, x: u32, y: u32) -> Option<Rgba8> {
        if x >= self.input.width
            || y >= self.input.height
            || self.input.projection.mode == Mode::Decal
        {
            return None;
        }
        self.projected_inner(x as usize, y as usize)
    }
    fn pixel_inner(&self, x: usize, y: usize) -> Rgba8 {
        let input = &self.input;
        if input.projection.mode == Mode::Decal {
            return if self.placed {
                self.decal_pixel(x, y, None)
            } else {
                Rgba8::TRANSPARENT
            };
        }
        self.projected_inner(x, y).unwrap_or(input.fallback)
    }
    fn projected_inner(&self, x: usize, y: usize) -> Option<Rgba8> {
        let input = &self.input;
        let mode = input.projection.mode;
        if self.reason.is_some() {
            return None;
        }
        let chain = input.image.unwrap();
        let mut acc = Acc::default();
        if mode == Mode::Uv {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let u = self.uv;
            self.sample(
                chain,
                [u[0] * px + u[1] * py + u[2], u[3] * px + u[4] * py + u[5]],
                self.rho,
                1.,
                &mut acc,
            );
            return acc.resolved();
        }
        let i = y * input.width as usize + x;
        if input.positions.unwrap().coverage[i] == 0 {
            return None;
        }
        let p = self.point(i);
        let nx = self.neighbor(x, y, 1, 0, p);
        let ny = self.neighbor(x, y, 0, 1, p);
        if mode != Mode::Triplanar {
            let st = self.project(p, 0, true);
            let dx = self.differences(st, nx, 0, true);
            let dy = self.differences(st, ny, 0, true);
            self.sample_at(chain, st, [dx, dy], 1., &mut acc);
            return acc.resolved();
        }
        if input.normals.unwrap().coverage[i] == 0 {
            return None;
        }
        let n = self.normal(i);
        let a = n.map(f64::abs);
        let most = a[0].max(a[1].max(a[2]));
        if most <= 1e-9 {
            return None;
        }
        let cut = (1. - input.projection.blend_width) * most;
        let mut weights = a.map(|v| (v - cut).max(0.));
        let mut total = weights[0] + weights[1] + weights[2];
        if total <= 0. {
            weights = a.map(|v| if v >= most { 1. } else { 0. });
            total = weights[0] + weights[1] + weights[2];
        }
        for axis in 0..3 {
            let weight = weights[axis] / total;
            if weight <= 0. {
                continue;
            }
            let positive = n[axis] >= 0.;
            let st = self.project(p, axis, positive);
            self.sample_at(
                chain,
                st,
                [
                    self.differences(st, nx, axis, positive),
                    self.differences(st, ny, axis, positive),
                ],
                weight,
                &mut acc,
            );
        }
        acc.resolved()
    }
    fn decal_point(&self, i: usize) -> Option<([f64; 3], f64)> {
        if self.input.positions.unwrap().coverage[i] == 0
            || self.input.normals.unwrap().coverage[i] == 0
        {
            return None;
        }
        let p = self.point(i);
        let config = self.input.projection;
        let half = config.placement.size.map(|s| s * 0.5);
        if (0..3).any(|j| p[j].abs() > half[j]) {
            return None;
        }
        let band = 1. - config.depth_hardness;
        let d = p[2].abs() / half[2];
        let mut cover = if band <= 0. || d <= 1. - band {
            1.
        } else {
            (1. - d) / band
        };
        let [nx, ny, nz] = self.normal(i);
        let length = (nx * nx + ny * ny + nz * nz).sqrt();
        if length <= 1e-9 {
            return None;
        }
        let angle = (-nz / length).clamp(-1., 1.).acos() * (180. / PI);
        if angle > config.backface_angle {
            return None;
        }
        let back_band = (1. - config.backface_hardness) * config.backface_angle;
        if back_band > 0. && angle > config.backface_angle - back_band {
            cover *= (config.backface_angle - angle) / back_band;
        }
        if cover > 0. {
            Some((p, cover))
        } else {
            None
        }
    }
    /// `external` は呼び手が決めた画素ごとの値。あれば自身の画像の代わりにそれへ被覆をかける（C# の externalValue）。
    fn decal_pixel(&self, x: usize, y: usize, external: Option<Rgba8>) -> Rgba8 {
        let i = y * self.input.width as usize + x;
        let Some((p, mut cover)) = self.decal_point(i) else {
            return Rgba8::TRANSPARENT;
        };
        let st = self.project(p, 0, true);
        let own = if external.is_none() {
            self.input.image
        } else {
            None
        };
        let ds = if own.is_some() || self.input.shape.is_some() {
            [
                self.differences(st, self.neighbor(x, y, 1, 0, p), 0, true),
                self.differences(st, self.neighbor(x, y, 0, 1, p), 0, true),
            ]
        } else {
            [[0.; 2]; 2]
        };
        if let Some(shape) = self.input.shape {
            let mut acc = Acc::default();
            self.sample_at(shape, st, ds, 1., &mut acc);
            cover *= acc.alpha() / 255.;
            if cover <= 0. {
                return Rgba8::TRANSPARENT;
            }
        }
        if let Some(chain) = own {
            let mut acc = Acc::default();
            self.sample_at(chain, st, ds, 1., &mut acc);
            acc.scaled(cover)
        } else {
            let c = external.unwrap_or(self.input.fallback);
            Rgba8::new(c.r, c.g, c.b, to_byte(c.a as f64 / 255. * cover))
        }
    }
    fn sample_at(
        &self,
        chain: &ImageMipChain<'_>,
        st: [f64; 2],
        ds: [[f64; 2]; 2],
        weight: f64,
        acc: &mut Acc,
    ) {
        let (w, h) = chain.wh(0);
        let (w, h) = (w as f64, h as f64);
        let [s, t] = st;
        let [a, b, c, d] = self.a;
        let sp = a * s + b * t + self.b[0];
        let tp = c * s + d * t + self.b[1];
        let [[dsx, dtx], [dsy, dty]] = ds;
        let cx = w * (a * dsx + b * dtx);
        let cy = h * (c * dsx + d * dtx);
        let ex = w * (a * dsy + b * dty);
        let ey = h * (c * dsy + d * dty);
        let rho = (cx * cx + cy * cy).sqrt().max((ex * ex + ey * ey).sqrt());
        self.sample(chain, [sp * w - 0.5, tp * h - 0.5], rho, weight, acc);
    }
    fn sample(
        &self,
        chain: &ImageMipChain<'_>,
        uv: [f64; 2],
        rho: f64,
        weight: f64,
        acc: &mut Acc,
    ) {
        let last = chain.level_count() - 1;
        if rho <= 1. || last == 0 {
            self.bilinear(chain, 0, uv, weight, acc);
            return;
        }
        let lod = rho.ln() * std::f64::consts::LOG2_E;
        if lod >= last as f64 {
            self.bilinear(chain, last, level(chain, last, uv), weight, acc);
            return;
        }
        let k = lod as usize;
        let f = lod - k as f64;
        self.bilinear(chain, k, level(chain, k, uv), weight * (1. - f), acc);
        if f > 0. {
            self.bilinear(chain, k + 1, level(chain, k + 1, uv), weight * f, acc);
        }
    }
    fn bilinear(
        &self,
        chain: &ImageMipChain<'_>,
        k: usize,
        uv: [f64; 2],
        weight: f64,
        acc: &mut Acc,
    ) {
        let (w, h) = chain.wh(k);
        let [u, v] = uv;
        let (fu, fv) = (u.floor(), v.floor());
        let (fx, fy) = (u - fu, v - fv);
        for dy in 0..2 {
            let wy = if dy == 0 { 1. - fy } else { fy };
            if wy <= 0. {
                continue;
            }
            let ty = wrap(fv as i64 + dy, h, self.input.projection.wrap);
            for dx in 0..2 {
                let wg = weight * wy * if dx == 0 { 1. - fx } else { fx };
                if wg <= 0. {
                    continue;
                }
                let tx = wrap(fu as i64 + dx, w, self.input.projection.wrap);
                acc.add(
                    wg,
                    match (tx, ty) {
                        (Some(tx), Some(ty)) => chain.read(k, tx, ty),
                        _ => Rgba8::TRANSPARENT,
                    },
                );
            }
        }
    }
}
fn level(chain: &ImageMipChain<'_>, k: usize, uv: [f64; 2]) -> [f64; 2] {
    if k == 0 {
        return uv;
    }
    let (w, h) = chain.wh(k);
    let (w0, h0) = chain.wh(0);
    [
        (uv[0] + 0.5) * (w as f64 / w0 as f64) - 0.5,
        (uv[1] + 0.5) * (h as f64 / h0 as f64) - 0.5,
    ]
}
fn wrap(i: i64, n: usize, mode: Wrap) -> Option<usize> {
    match mode {
        Wrap::None => {
            if i < 0 || i >= n as i64 {
                None
            } else {
                Some(i as usize)
            }
        }
        Wrap::Clamp => Some(i.clamp(0, n as i64 - 1) as usize),
        Wrap::Repeat => Some(i.rem_euclid(n as i64) as usize),
    }
}
