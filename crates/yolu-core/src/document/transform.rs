//! C# AffineResampler と同じ画素中心・丸め・premultiplied 補間。
use super::operations::Dirty;
use super::{Document, LayerLocks, Target};
use crate::math::to_byte;
use crate::surface::{Growth, PixelReader, Tile};
use crate::{CoreError, LayerId, LayerKind, Rgba8, SelectionMask, Surface, TileCoord};
use rayon::prelude::*;
use std::collections::BTreeSet;

/// x' = a x + b y + tx、y' = c x + d y + ty（左下原点）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine2D {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub tx: f64,
    pub ty: f64,
}
impl Affine2D {
    pub const IDENTITY: Self = Self {
        a: 1.,
        b: 0.,
        c: 0.,
        d: 1.,
        tx: 0.,
        ty: 0.,
    };
    pub fn translation(dx: f64, dy: f64) -> Self {
        Self {
            tx: dx,
            ty: dy,
            ..Self::IDENTITY
        }
    }
    /// 軸ごとの拡大、支点周りの反時計回りの回転（度）、移動の順。
    pub fn from_parts(
        pivot: (f64, f64),
        movement: (f64, f64),
        degrees: f64,
        scale: (f64, f64),
    ) -> Result<Self, CoreError> {
        let (px, py) = pivot;
        let (dx, dy) = movement;
        let (sx, sy) = scale;
        if ![px, py, dx, dy, degrees, sx, sy]
            .iter()
            .all(|v| v.is_finite())
        {
            return Err(CoreError::InvalidArgument("変形は有限値"));
        }
        let r = degrees * std::f64::consts::PI / 180.;
        let cos = r.cos();
        let sin = r.sin();
        let (a, b, c, d) = (cos * sx, -sin * sy, sin * sx, cos * sy);
        Ok(Self {
            a,
            b,
            c,
            d,
            tx: px + dx - (a * px + b * py),
            ty: py + dy - (c * px + d * py),
        })
    }
    pub fn apply(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.b * y + self.tx,
            self.c * x + self.d * y + self.ty,
        )
    }
    pub fn inverse(self) -> Result<Self, CoreError> {
        self.validate()?;
        let det = self.a * self.d - self.b * self.c;
        if !det.is_finite() || det.abs() < 1e-9 {
            return Err(CoreError::InvalidArgument("変形が潰れる、または範囲外"));
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        let inverse = Self {
            a,
            b,
            c,
            d,
            tx: -(a * self.tx + b * self.ty),
            ty: -(c * self.tx + d * self.ty),
        };
        inverse.validate()?;
        Ok(inverse)
    }
    fn validate(self) -> Result<(), CoreError> {
        if [self.a, self.b, self.c, self.d, self.tx, self.ty]
            .iter()
            .all(|v| v.is_finite())
        {
            Ok(())
        } else {
            Err(CoreError::InvalidArgument("変形は有限値"))
        }
    }
    fn integer_move(self) -> bool {
        self.a == 1.
            && self.b == 0.
            && self.c == 0.
            && self.d == 1.
            && self.tx == self.tx.round()
            && self.ty == self.ty.round()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resampling {
    Bilinear,
    Nearest,
}

/// 画素（キャンバスの外は透明）。ディスクから読めない画素は透明で、読み手が誤りを覚える（`PixelReader::finish`）。
#[inline]
pub(super) fn read(source: &mut PixelReader<'_>, x: i64, y: i64) -> Rgba8 {
    source.pixel(x, y)
}
pub(super) fn sample(
    source: &mut PixelReader<'_>,
    lifted: Option<&SelectionMask>,
    method: Resampling,
    sx: f64,
    sy: f64,
) -> Rgba8 {
    // 巨大な有限変換は外側の透明へ。整数変換時のオーバーフローも防ぐ。
    if !sx.is_finite()
        || !sy.is_finite()
        || sx < -1.
        || sy < -1.
        || sx > source.surface().width() as f64 + 1.
        || sy > source.surface().height() as f64 + 1.
    {
        return Rgba8::TRANSPARENT;
    }
    let (fx, fy) = (sx - 0.5, sy - 0.5);
    let (rx, ry) = (fx.round_ties_even(), fy.round_ties_even());
    if method == Resampling::Nearest || (fx - rx).abs() < 1e-6 && (fy - ry).abs() < 1e-6 {
        let (x, y) = if method == Resampling::Nearest {
            (sx.floor() as i64, sy.floor() as i64)
        } else {
            (rx as i64, ry as i64)
        };
        let amount = selected_amount(lifted, x, y);
        if amount == 0 {
            return Rgba8::TRANSPARENT;
        }
        let p = read(source, x, y);
        return if amount == 255 {
            p
        } else {
            Rgba8::new(
                p.r,
                p.g,
                p.b,
                to_byte(p.a as f64 / 255. * amount as f64 / 255.),
            )
        };
    }
    let (x, y) = (fx.floor() as i64, fy.floor() as i64);
    let (tx, ty) = (fx - x as f64, fy - y as f64);
    let (mut r, mut g, mut b, mut a) = (0., 0., 0., 0.);
    for (px, py, w) in [
        (x, y, (1. - tx) * (1. - ty)),
        (x + 1, y, tx * (1. - ty)),
        (x, y + 1, (1. - tx) * ty),
        (x + 1, y + 1, tx * ty),
    ] {
        if w <= 0. {
            continue;
        }
        let p = read(source, px, py);
        if p.a == 0 {
            continue;
        }
        let k = p.a as f64 / 255. * selected_amount(lifted, px, py) as f64 / 255. * w;
        r += p.r as f64 / 255. * k;
        g += p.g as f64 / 255. * k;
        b += p.b as f64 / 255. * k;
        a += k;
    }
    if a <= 1e-9 {
        Rgba8::TRANSPARENT
    } else {
        Rgba8::new(to_byte(r / a), to_byte(g / a), to_byte(b / a), to_byte(a))
    }
}
fn targets(source: &Surface, lifted: Option<&SelectionMask>, forward: Affine2D) -> Vec<TileCoord> {
    let mut out = BTreeSet::new();
    let tile = source.tile_size() as f64;
    let w = source.width() as f64;
    let h = source.height() as f64;
    for coord in source.tile_coords() {
        if lifted.is_some_and(|m| m.tile(coord).is_none()) {
            continue;
        }
        out.insert(coord);
        let (x0, y0) = (coord.x as f64 * tile, coord.y as f64 * tile);
        let (x1, y1) = ((x0 + tile).min(w), (y0 + tile).min(h));
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
            let (x, y) = forward.apply(x, y);
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
        }
        if maxx < -1. || maxy < -1. || minx > w + 1. || miny > h + 1. {
            continue;
        }
        let tx0 = (((minx.max(-1.) - 1.) / tile).floor() as i64).max(0);
        let ty0 = (((miny.max(-1.) - 1.) / tile).floor() as i64).max(0);
        let tx1 = (((maxx.min(w + 1.) + 1.) / tile).floor() as i64)
            .min((source.width() - 1) as i64 / source.tile_size() as i64);
        let ty1 = (((maxy.min(h + 1.) + 1.) / tile).floor() as i64)
            .min((source.height() - 1) as i64 / source.tile_size() as i64);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                out.insert(TileCoord::new(tx as u32, ty as u32));
            }
        }
    }
    out.into_iter().collect()
}
impl Document {
    /// 動く画素の外接矩形。全チャンネルとマスクのアルファを、指定範囲と文書の選択範囲で絞る。
    pub fn transform_bounds(
        &self,
        id: LayerId,
        region: Option<&SelectionMask>,
    ) -> Result<Option<crate::Rect>, CoreError> {
        let i = self.index_of(id)?;
        let effective = self.effective_region(region)?;
        let mut surfaces: Vec<_> = self.layers[i].surfaces.iter().flatten().collect();
        if let Some(mask) = &self.layers[i].mask {
            surfaces.push(&mask.surface);
        }
        let (mut x0, mut y0, mut x1, mut y1) = (self.width, self.height, 0, 0);
        for surface in surfaces {
            for coord in surface.tile_coords() {
                if effective.as_ref().is_some_and(|m| m.tile(coord).is_none()) {
                    continue;
                }
                let ts = self.tile_size;
                for y in coord.y * ts..(coord.y * ts + ts).min(self.height) {
                    for x in coord.x * ts..(coord.x * ts + ts).min(self.width) {
                        if surface.pixel(x, y)?.a != 0
                            && effective.as_ref().is_none_or(|m| m.amount(x, y) != 0)
                        {
                            x0 = x0.min(x);
                            y0 = y0.min(y);
                            x1 = x1.max(x + 1);
                            y1 = y1.max(y + 1);
                        }
                    }
                }
            }
        }
        Ok((x1 > x0 && y1 > y0).then(|| crate::Rect::new(x0, y0, x1 - x0, y1 - y0)))
    }
    /// 全チャンネルと、include_mask ならマスクを 1 回の Undo で変形する。
    pub fn transform_layer(
        &mut self,
        id: LayerId,
        transform: Affine2D,
        method: Resampling,
        include_mask: bool,
    ) -> Result<bool, CoreError> {
        self.transform_targets(&[id], transform, method, include_mask, None, &mut || false)
    }
    /// 選んだグループ内のラスターレイヤーもまとめて変形する。
    pub fn transform_layers(
        &mut self,
        ids: &[LayerId],
        transform: Affine2D,
        method: Resampling,
    ) -> Result<bool, CoreError> {
        self.transform_layers_cancellable(ids, transform, method, &mut || false)
    }
    /// 取消はタイルのまとまりごとに確認する。取消・拒否で画素・履歴・変更番号は変えない。
    pub fn transform_layers_cancellable(
        &mut self,
        ids: &[LayerId],
        transform: Affine2D,
        method: Resampling,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<bool, CoreError> {
        let members = self.topmost_of(ids)?;
        let targets: Vec<_> = self
            .layers
            .iter()
            .enumerate()
            .filter(|(i, l)| {
                l.kind == LayerKind::Raster
                    && members
                        .iter()
                        .any(|&m| m == l.id || self.is_descendant(*i, m))
            })
            .map(|(_, l)| l.id)
            .collect();
        self.transform_targets(&targets, transform, method, true, None, cancelled)
    }
    /// 渡した範囲と文書の選択範囲の交差だけを移す。明示した範囲では文書の選択範囲は動かさない。
    pub fn transform_layer_region(
        &mut self,
        id: LayerId,
        transform: Affine2D,
        method: Resampling,
        include_mask: bool,
        region: &SelectionMask,
    ) -> Result<bool, CoreError> {
        self.transform_targets(
            &[id],
            transform,
            method,
            include_mask,
            Some(region),
            &mut || false,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn transform_targets(
        &mut self,
        ids: &[LayerId],
        transform: Affine2D,
        method: Resampling,
        include_mask: bool,
        region: Option<&SelectionMask>,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<bool, CoreError> {
        self.ensure_no_stroke()?;
        transform.validate()?;
        if ids.is_empty() {
            return Err(CoreError::Unsupported("動かすラスターレイヤーが無い"));
        }
        for &id in ids {
            let index = self.index_of(id)?;
            self.ensure_raster(index)?;
            // パスで描かれたレイヤーを動かすと次の描き直しで元に戻るので断る（C# の RequireTransformable。ロックの検査より先）
            self.refuse_path_layer(index)?;
        }
        if transform == Affine2D::IDENTITY {
            return Ok(false);
        }
        let effective = self.effective_region(region)?;
        for &id in ids {
            self.refuse_lock(id, LayerLocks::ALL)?;
            self.refuse_lock(id, LayerLocks::POSITION)?;
            if effective.is_some() || !transform.integer_move() {
                self.refuse_lock(id, LayerLocks::PIXELS)?;
            }
            if effective.is_some() {
                self.refuse_lock(id, LayerLocks::TRANSPARENCY)?;
            }
        }
        let inverse = transform.inverse()?;
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
                    let source = self.target_surface(i, target).expect("面");
                    (
                        target,
                        source,
                        targets(source, effective.as_ref(), transform),
                    )
                })
                .collect();
            for (_, source, coords) in &plans {
                for &coord in coords {
                    estimate += 64 + source.tile(coord).map_or(0, Tile::byte_size);
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
                                    let (sx, sy) = inverse.apply(
                                        (coord.x * ts + x) as f64 + 0.5,
                                        (coord.y * ts + y) as f64 + 0.5,
                                    );
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
                                    let p = if moved.a == 0 {
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
                        .collect::<Vec<Result<_, CoreError>>>()
                        .into_iter()
                        .collect::<Result<Vec<_>, CoreError>>()?;
                    for (coord, after) in rendered {
                        let before = source.tile(coord);
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
            if region.is_none() {
                if let Some(selection) = &self.selection {
                    let source = selection_surface(selection);
                    let mut reader = PixelReader::new(&source);
                    let mut tiles = Vec::new();
                    for coord in targets(&source, None, transform) {
                        if cancelled() {
                            return Err(CoreError::Cancelled);
                        }
                        let ts = self.tile_size;
                        let mut bytes = vec![0; (ts * ts) as usize];
                        for y in 0..ts.min(self.height - coord.y * ts) {
                            for x in 0..ts.min(self.width - coord.x * ts) {
                                let (sx, sy) = inverse.apply(
                                    (coord.x * ts + x) as f64 + 0.5,
                                    (coord.y * ts + y) as f64 + 0.5,
                                );
                                bytes[(y * ts + x) as usize] =
                                    sample(&mut reader, None, method, sx, sy).a;
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

pub(super) fn selected_amount(lifted: Option<&SelectionMask>, x: i64, y: i64) -> u8 {
    if x < 0 || y < 0 {
        return 0;
    }
    lifted.map_or(255, |m| {
        if x >= m.width() as i64 || y >= m.height() as i64 {
            0
        } else {
            m.amount(x as u32, y as u32)
        }
    })
}
/// C# の選択範囲と同じ RGBA のビュー（元の 1 バイト面は変更しない）。
pub(super) fn selection_surface(mask: &SelectionMask) -> Surface {
    let mut surface = Surface::new(mask.width(), mask.height(), mask.tile_size());
    let n = (mask.tile_size() * mask.tile_size()) as usize;
    for coord in mask.tile_coords() {
        let mut amounts = vec![0; n];
        mask.copy_tile(coord, &mut amounts).expect("選択タイル");
        let mut rgba = vec![0; n * 4];
        for (i, a) in amounts.into_iter().enumerate() {
            rgba[i * 4 + 3] = a;
        }
        surface.restore(coord, Tile::from_bytes(&rgba).as_ref());
    }
    surface
}
