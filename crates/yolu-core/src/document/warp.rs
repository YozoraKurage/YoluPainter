//! 非アフィン変形。画素中心の逆写像と premultiplied 双線形補間。
use super::operations::Dirty;
use super::transform::{read, sample, selected_amount, selection_surface};
use super::{Document, LayerLocks, Resampling, Target};
use crate::math::to_byte;
use crate::surface::{Growth, PixelReader, Tile};
use crate::{CoreError, LayerId, Rgba8, SelectionMask, Surface, TileCoord};
use rayon::prelude::*;

pub type WarpPoint = (f64, f64);

#[derive(Clone, Debug, PartialEq)]
pub struct Homography {
    matrix: [f64; 9],
}
impl Homography {
    pub const IDENTITY: Self = Self {
        matrix: [1., 0., 0., 0., 1., 0., 0., 0., 1.],
    };
    /// 左下・右下・右上・左上。凹形・交差・潰れを拒否する。
    pub fn from_quads(from: [WarpPoint; 4], to: [WarpPoint; 4]) -> Result<Self, CoreError> {
        valid_quad(&from)?;
        valid_quad(&to)?;
        if from == to {
            return Ok(Self::IDENTITY);
        }
        let delta = (to[0].0 - from[0].0, to[0].1 - from[0].1);
        if from
            .iter()
            .zip(to)
            .all(|(p, q)| (p.0 + delta.0, p.1 + delta.1) == q)
        {
            return Ok(Self {
                matrix: [1., 0., delta.0, 0., 1., delta.1, 0., 0., 1.],
            });
        }
        let mut a = [[0.; 9]; 8];
        for i in 0..4 {
            let (x, y) = from[i];
            let (u, v) = to[i];
            a[2 * i] = [x, y, 1., 0., 0., 0., -u * x, -u * y, u];
            a[2 * i + 1] = [0., 0., 0., x, y, 1., -v * x, -v * y, v];
        }
        for col in 0..8 {
            let pivot = (col..8)
                .max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))
                .unwrap();
            a.swap(col, pivot);
            let d = a[col][col];
            if !d.is_finite() || d.abs() < 1e-12 {
                return Err(invalid());
            }
            for v in &mut a[col][col..] {
                *v /= d;
            }
            for row in 0..8 {
                if row != col {
                    let k = a[row][col];
                    let pivot = a[col];
                    for (value, reference) in a[row][col..].iter_mut().zip(&pivot[col..]) {
                        *value -= k * reference;
                    }
                }
            }
        }
        let mut matrix = [0.; 9];
        for i in 0..8 {
            matrix[i] = a[i][8];
        }
        matrix[8] = 1.;
        let result = Self { matrix };
        let denominators = from.map(|(x, y)| matrix[6] * x + matrix[7] * y + matrix[8]);
        if denominators
            .iter()
            .any(|d| !d.is_finite() || d * denominators[0] <= 0.)
        {
            return Err(invalid());
        }
        result.inverse()?;
        Ok(result)
    }
    pub fn apply(&self, x: f64, y: f64) -> WarpPoint {
        let m = self.matrix;
        let d = m[6] * x + m[7] * y + m[8];
        (
            (m[0] * x + m[1] * y + m[2]) / d,
            (m[3] * x + m[4] * y + m[5]) / d,
        )
    }
    pub fn inverse(&self) -> Result<Self, CoreError> {
        let [a, b, c, d, e, f, g, h, i] = self.matrix;
        let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
        if !det.is_finite() || det.abs() < 1e-12 {
            return Err(invalid());
        }
        let matrix = [
            e * i - f * h,
            c * h - b * i,
            b * f - c * e,
            f * g - d * i,
            a * i - c * g,
            c * d - a * f,
            d * h - e * g,
            b * g - a * h,
            a * e - b * d,
        ]
        .map(|v| v / det);
        if matrix.iter().any(|v| !v.is_finite()) {
            return Err(invalid());
        }
        Ok(Self { matrix })
    }
}
fn invalid() -> CoreError {
    CoreError::InvalidArgument("変形が潰れる、または範囲外")
}
fn cross(a: WarpPoint, b: WarpPoint, c: WarpPoint) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}
fn valid_quad(p: &[WarpPoint; 4]) -> Result<(), CoreError> {
    if p.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return Err(invalid());
    }
    let sign = cross(p[0], p[1], p[2]);
    if !sign.is_finite()
        || sign.abs() < 1e-9
        || (0..4).any(|i| {
            let turn = cross(p[i], p[(i + 1) % 4], p[(i + 2) % 4]);
            !turn.is_finite() || turn * sign <= 1e-12
        })
    {
        return Err(invalid());
    }
    Ok(())
}

/// 列×行はセル数。各セルを左下→右上の対角線で分けた区分線形写像。
#[derive(Clone, Debug, PartialEq)]
pub struct WarpMesh {
    pub columns: usize,
    pub rows: usize,
    pub source: Vec<WarpPoint>,
    pub points: Vec<WarpPoint>,
}
impl WarpMesh {
    pub fn new(bounds: [f64; 4], columns: usize, rows: usize) -> Result<Self, CoreError> {
        let [x, y, w, h] = bounds;
        if !(1..=32).contains(&columns)
            || !(1..=32).contains(&rows)
            || !bounds.iter().all(|v| v.is_finite())
            || w <= 0.
            || h <= 0.
        {
            return Err(invalid());
        }
        let source: Vec<_> = (0..=rows)
            .flat_map(|j| {
                (0..=columns).map(move |i| {
                    (
                        x + w * i as f64 / columns as f64,
                        y + h * j as f64 / rows as f64,
                    )
                })
            })
            .collect();
        Ok(Self {
            columns,
            rows,
            points: source.clone(),
            source,
        })
    }
    fn cells(&self) -> impl Iterator<Item = [usize; 4]> + '_ {
        (0..self.rows).flat_map(move |j| {
            (0..self.columns).map(move |i| {
                let a = j * (self.columns + 1) + i;
                [a, a + 1, a + self.columns + 2, a + self.columns + 1]
            })
        })
    }
    fn validate(&self) -> Result<(), CoreError> {
        if !(1..=32).contains(&self.columns)
            || !(1..=32).contains(&self.rows)
            || self.source.len() != (self.columns + 1) * (self.rows + 1)
            || self.points.len() != self.source.len()
        {
            return Err(invalid());
        }
        for cell in self.cells() {
            let s = cell.map(|i| self.source[i]);
            let p = cell.map(|i| self.points[i]);
            valid_quad(&s)?;
            valid_quad(&p)?;
            if cross(s[0], s[1], s[2]) * cross(p[0], p[1], p[2]) <= 0. {
                return Err(invalid());
            }
        }
        Ok(())
    }
    fn map_point(&self, x: f64, y: f64, from: &[WarpPoint], to: &[WarpPoint]) -> Option<WarpPoint> {
        for c in self.cells() {
            for t in [[c[0], c[1], c[2]], [c[0], c[2], c[3]]] {
                let [a, b, c] = t.map(|i| from[i]);
                let p = (x, y);
                let d = cross(a, b, c);
                let u = cross(p, b, c) / d;
                let v = cross(a, p, c) / d;
                let w = 1. - u - v;
                if u >= -1e-9 && v >= -1e-9 && w >= -1e-9 {
                    let [a, b, c] = t.map(|i| to[i]);
                    return Some((u * a.0 + v * b.0 + w * c.0, u * a.1 + v * b.1 + w * c.1));
                }
            }
        }
        None
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiquifyMode {
    Push,
    Clockwise,
    CounterClockwise,
    Pinch,
    Expand,
    Restore,
}
#[derive(Clone, Debug, PartialEq)]
pub struct LiquifyDab {
    pub center: WarpPoint,
    pub delta: WarpPoint,
    pub radius: f64,
    pub strength: f64,
    pub mode: LiquifyMode,
}
impl LiquifyDab {
    /// 開始元から再評価する1セッションの描点数の上限。
    pub const MAX_COUNT: usize = 16_384;
    fn weight(&self, p: WarpPoint) -> f64 {
        let d = ((p.0 - self.center.0).hypot(p.1 - self.center.1) / self.radius).min(1.);
        (1. - d * d).powi(2) * self.strength
    }
    fn apply(&self, p: WarpPoint) -> WarpPoint {
        let k = self.weight(p);
        let (x, y) = (p.0 - self.center.0, p.1 - self.center.1);
        let (x, y) = match self.mode {
            LiquifyMode::Push => return (p.0 - self.delta.0 * k, p.1 - self.delta.1 * k),
            LiquifyMode::Clockwise | LiquifyMode::CounterClockwise => {
                let a = k
                    * 0.25
                    * if self.mode == LiquifyMode::Clockwise {
                        1.
                    } else {
                        -1.
                    };
                (x * a.cos() - y * a.sin(), x * a.sin() + y * a.cos())
            }
            LiquifyMode::Pinch => (x * (1. + k * 0.25), y * (1. + k * 0.25)),
            LiquifyMode::Expand => (x / (1. + k * 0.25), y / (1. + k * 0.25)),
            LiquifyMode::Restore => return p,
        };
        (x + self.center.0, y + self.center.1)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Warp {
    Projective(Homography),
    Mesh(WarpMesh),
    Liquify(Vec<LiquifyDab>),
}
impl Warp {
    fn inverse(&self) -> Result<Self, CoreError> {
        Ok(match self {
            Self::Projective(h) => Self::Projective(h.inverse()?),
            Self::Mesh(m) => {
                m.validate()?;
                Self::Mesh(m.clone())
            }
            Self::Liquify(dabs) => {
                if dabs.len() > LiquifyDab::MAX_COUNT
                    || dabs.iter().any(|d| {
                        ![
                            d.center.0, d.center.1, d.delta.0, d.delta.1, d.radius, d.strength,
                        ]
                        .iter()
                        .all(|v| v.is_finite())
                            || d.radius <= 0.
                            || !(0. ..=1.).contains(&d.strength)
                    })
                {
                    return Err(invalid());
                }
                Self::Liquify(dabs.clone())
            }
        })
    }
    /// None は元と変形後の格子の両方の外側。補間や選択量の合成をせず元画素を残す。
    fn apply(&self, x: f64, y: f64) -> Option<WarpPoint> {
        match self {
            Self::Projective(h) => Some(h.apply(x, y)),
            Self::Mesh(m) => m.map_point(x, y, &m.points, &m.source).or_else(|| {
                // 元の格子だけに含まれる画素は、移動して空いた場所なので透明にする。
                m.map_point(x, y, &m.source, &m.points)
                    .map(|_| (f64::NAN, f64::NAN))
            }),
            Self::Liquify(dabs) => {
                let mut p = (x, y);
                let mut retained = 1.;
                for d in dabs.iter().rev() {
                    if d.mode == LiquifyMode::Restore {
                        retained *= 1. - d.weight(p);
                    } else {
                        let q = d.apply(p);
                        p = (p.0 + (q.0 - p.0) * retained, p.1 + (q.1 - p.1) * retained);
                    }
                }
                Some(p)
            }
        }
    }
    fn identity(&self) -> bool {
        match self {
            Self::Projective(h) => *h == Homography::IDENTITY,
            Self::Mesh(m) => m.source == m.points,
            Self::Liquify(d) => d.is_empty(),
        }
    }
}
fn targets(source: &Surface) -> Vec<TileCoord> {
    (0..source.height().div_ceil(source.tile_size()))
        .flat_map(|y| {
            (0..source.width().div_ceil(source.tile_size())).map(move |x| TileCoord::new(x, y))
        })
        .collect()
}

impl Document {
    pub fn warp_layers_cancellable(
        &mut self,
        ids: &[LayerId],
        transform: &Warp,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<bool, CoreError> {
        self.warp_targets(ids, transform, None, cancelled)
    }
    /// ゆがみの開始時の写しから再標本化する。確定済みの履歴は現在の文書に残る。
    pub fn liquify_from_snapshot_cancellable(
        &mut self,
        ids: &[LayerId],
        dabs: &[LiquifyDab],
        source: &Document,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<bool, CoreError> {
        if dabs.len() > LiquifyDab::MAX_COUNT {
            return Err(invalid());
        }
        if (std::mem::size_of_val(dabs) as u64) > self.stroke_budget {
            return Err(CoreError::StrokeBudgetExceeded);
        }
        if self.id != source.id
            || self.width != source.width
            || self.height != source.height
            || self.tile_size != source.tile_size
        {
            return Err(invalid());
        }
        for &id in ids {
            let i = self.index_of(id)?;
            let j = source.index_of(id)?;
            if self.layers[i].surface_channels() != source.layers[j].surface_channels()
                || self.layers[i].mask.is_some() != source.layers[j].mask.is_some()
            {
                return Err(invalid());
            }
        }
        self.warp_targets(ids, &Warp::Liquify(dabs.to_vec()), Some(source), cancelled)
    }
    fn warp_targets(
        &mut self,
        ids: &[LayerId],
        transform: &Warp,
        snapshot: Option<&Document>,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<bool, CoreError> {
        self.ensure_no_stroke()?;
        if cancelled() {
            return Err(CoreError::Cancelled);
        }
        let inverse = transform.inverse()?;
        let method = Resampling::Bilinear;
        let region = None;
        let include_mask = true;
        if ids.is_empty() {
            return Err(CoreError::Unsupported("動かすラスターレイヤーが無い"));
        }
        for &id in ids {
            let index = self.index_of(id)?;
            self.ensure_raster(index)?;
            // パスで描かれたレイヤーを動かすと次の描き直しで元に戻るので断る（C# の RequireTransformable。ロックの検査より先）
            self.refuse_path_layer(index)?;
        }
        if transform.identity() {
            return Ok(false);
        }
        let effective = self.effective_region(region)?;
        for &id in ids {
            self.refuse_lock(id, LayerLocks::ALL)?;
            self.refuse_lock(id, LayerLocks::POSITION)?;
            {
                self.refuse_lock(id, LayerLocks::PIXELS)?;
            }
            if effective.is_some() {
                self.refuse_lock(id, LayerLocks::TRANSPARENCY)?;
            }
        }
        let tiles = (self.width as u64).div_ceil(self.tile_size as u64)
            * (self.height as u64).div_ceil(self.tile_size as u64);
        if tiles.saturating_mul(self.tile_size as u64 * self.tile_size as u64 * 4 + 64)
            > self.stroke_budget
        {
            return Err(CoreError::StrokeBudgetExceeded);
        }
        let mut copy = self.edit_copy()?;
        let mut estimate = 0;
        let mut cost = 32;
        let mut changed = false;
        for &id in ids {
            let i = self.index_of(id)?;
            let mut surfaces: Vec<_> = self.layers[i]
                .surface_channels()
                .into_iter()
                .map(Target::Channel)
                .collect();
            if include_mask && self.layers[i].mask.is_some() {
                surfaces.push(Target::Mask);
            }
            let plans: Vec<_> = surfaces
                .into_iter()
                .map(|target| {
                    let document = snapshot.unwrap_or(self);
                    let index = document.index_of(id).expect("検査済み");
                    let source = document.target_surface(index, target).expect("面");
                    (target, source, targets(source))
                })
                .collect();
            for (_, source, coords) in &plans {
                for &coord in coords {
                    estimate += 64
                        + source.tile_bytes() as u64
                        + source.tile(coord).map_or(0, Tile::byte_size);
                }
            }
            if estimate > self.stroke_budget {
                return Err(CoreError::StrokeBudgetExceeded);
            }
            for (target, source, coords) in plans {
                let mut surface_cost = 0;
                for batch in coords.chunks(rayon::current_num_threads().clamp(1, 64) * 4) {
                    if cancelled() {
                        return Err(CoreError::Cancelled);
                    }
                    let rendered: Vec<_> = batch
                        .par_iter()
                        .map(|&coord| {
                            let ts = source.tile_size();
                            let mut bytes = vec![0; source.tile_bytes()];
                            let mut reader = PixelReader::new(source);
                            for y in 0..ts.min(source.height() - coord.y * ts) {
                                for x in 0..ts.min(source.width() - coord.x * ts) {
                                    let mapping = inverse.apply(
                                        (coord.x * ts + x) as f64 + 0.5,
                                        (coord.y * ts + y) as f64 + 0.5,
                                    );
                                    let (sx, sy) = mapping.unwrap_or((f64::NAN, f64::NAN));
                                    let px = coord.x * ts + x;
                                    let py = coord.y * ts + y;
                                    let original = read(&mut reader, px as i64, py as i64);
                                    let here =
                                        selected_amount(effective.as_ref(), px as i64, py as i64);
                                    let remaining = if here == 0 {
                                        original
                                    } else if here == 255 {
                                        Rgba8::TRANSPARENT
                                    } else {
                                        Rgba8::new(
                                            original.r,
                                            original.g,
                                            original.b,
                                            to_byte(
                                                original.a as f64 / 255. * (255 - here) as f64
                                                    / 255.,
                                            ),
                                        )
                                    };
                                    let moved =
                                        sample(&mut reader, effective.as_ref(), method, sx, sy);
                                    let p = if mapping.is_none() {
                                        original
                                    } else if matches!(transform, Warp::Liquify(_)) {
                                        let moved = sample(&mut reader, None, method, sx, sy);
                                        mix(original, moved, here)
                                    } else if moved.a == 0 {
                                        if here == 255 {
                                            moved
                                        } else {
                                            remaining
                                        }
                                    } else if remaining.a == 0 || moved.a == 255 {
                                        moved
                                    } else {
                                        crate::blend::blend(
                                            remaining,
                                            moved,
                                            1.,
                                            crate::BlendMode::Normal,
                                        )
                                    };
                                    let at = ((y * ts + x) * 4) as usize;
                                    bytes[at..at + 4].copy_from_slice(&p.to_array());
                                }
                            }
                            reader.finish()?;
                            Ok((coord, Tile::from_vec(bytes)))
                        })
                        .collect::<Result<_, CoreError>>()?;
                    for (coord, after) in rendered {
                        let before = self.target_surface(i, target).expect("面").tile(coord);
                        if Tile::same(before, after.as_ref()) {
                            continue;
                        }
                        let growth = Growth {
                            budget: copy.source_budget,
                            others: copy.allocated_bytes()
                                - copy
                                    .target_surface(i, target)
                                    .expect("面")
                                    .allocated_bytes(),
                        };
                        let surface = copy.target_surface_mut(i, target).expect("面");
                        growth.ensure(
                            surface.allocated_bytes(),
                            surface.growth_to(coord, after.as_ref()),
                        )?;
                        surface.restore(coord, after.as_ref());
                        changed = true;
                        surface_cost += 16
                            + before.map_or(0, Tile::byte_size)
                            + after.as_ref().map_or(0, Tile::byte_size);
                    }
                }
                if surface_cost > 0 {
                    cost += 64 + surface_cost;
                }
            }
        }
        if cancelled() {
            return Err(CoreError::Cancelled);
        }
        if changed {
            if !matches!(transform, Warp::Liquify(_)) {
                if let Some(selection) = &self.selection {
                    let source = selection_surface(selection);
                    let mut reader = PixelReader::new(&source);
                    let mut tiles = Vec::new();
                    for coord in targets(&source) {
                        if cancelled() {
                            return Err(CoreError::Cancelled);
                        }
                        let ts = self.tile_size;
                        let mut bytes = vec![0; (ts * ts) as usize];
                        for y in 0..ts.min(self.height - coord.y * ts) {
                            for x in 0..ts.min(self.width - coord.x * ts) {
                                let px = coord.x * ts + x;
                                let py = coord.y * ts + y;
                                bytes[(y * ts + x) as usize] = match inverse
                                    .apply(px as f64 + 0.5, py as f64 + 0.5)
                                {
                                    Some((sx, sy)) => sample(&mut reader, None, method, sx, sy).a,
                                    None => read(&mut reader, px as i64, py as i64).a,
                                };
                            }
                        }
                        if bytes.iter().any(|&a| a != 0) {
                            tiles.push((coord, bytes));
                        }
                    }
                    reader.finish()?;
                    let moved = SelectionMask::from_amount_tiles(
                        self.width,
                        self.height,
                        self.tile_size,
                        tiles,
                    )?;
                    cost += 64 + moved.history_bytes();
                    copy.selection = (!moved.is_empty()).then_some(moved);
                }
            }
            self.commit_copy(copy, cost, Dirty::Layers(ids.to_vec()))?;
        }
        Ok(changed)
    }
}

fn mix(a: Rgba8, b: Rgba8, amount: u8) -> Rgba8 {
    if amount == 0 {
        return a;
    }
    if amount == 255 {
        return b;
    }
    let t = amount as f64 / 255.;
    let alpha = a.a as f64 * (1. - t) + b.a as f64 * t;
    if alpha <= 1e-9 {
        return a;
    }
    let color = |x: u8, y: u8| {
        to_byte((x as f64 * a.a as f64 * (1. - t) + y as f64 * b.a as f64 * t) / alpha / 255.)
    };
    Rgba8::new(
        color(a.r, b.r),
        color(a.g, b.g),
        color(a.b, b.b),
        to_byte(alpha / 255.),
    )
}
