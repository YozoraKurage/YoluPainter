//! 対称のダブ（C# の BrushStroke.Symmetry）: 1 つのダブを対称の写し全部へ置く。
//!
//! 写しごとに、写したダブの中心のまわりの画素の中心を元へ戻し（転置）、元のダブの上で被覆率を測る。写しが重なる画素は大きい方の
//! 被覆率で 1 回だけ塗る。画素を塗る式は普通のダブと同じ（[`super::apply_at`]）。C# は画素を辞書に集めて番号の順に塗るが、
//! ここでは同じ画素の組をタイルごとにまとめて塗る（各画素は自分の値だけで決まるので結果は同じ。予算で止まるかどうかも、
//! 止まればストロークごと取り消すので同じ）。
//!
//! 3D の対称（[`Brush::model_symmetry`]）の写しは、ダブの中心の面の点から決めた 1 次の写像（`UvCopy`）で写し、同じく画素の中心を
//! 元へ戻して測る。2D の対称と両方あれば、3D の写しの後に 2D の写しを当てた写像（写しの数は掛け算）。2D の対称だけのダブは、
//! 今までと同じ変換と式（C# と同じ）。

use super::*;
use crate::geometry::{MirrorOutcome, UvCopy};
use crate::symmetry::SymmetryTransform;

/// 対称の 1 ダブで調べる候補の画素の上限（C# の MaxSymmetryCandidatePixels）。超えたらストロークごと取り消す。
pub const MAX_SYMMETRY_CANDIDATE_PIXELS: i64 = 4 << 20;

/// 写し 1 つの写像: 2D の対称の変換（直交。C# と同じ式）か、3D の対称の写し（とその後の 2D の対称）の 1 次の写像。
#[derive(Clone, Copy, Debug)]
pub(super) enum CopyMap {
    Canvas(SymmetryTransform),
    Uv(UvCopy),
}

impl CopyMap {
    #[inline]
    fn map(&self, x: f64, y: f64) -> (f64, f64) {
        match self {
            CopyMap::Canvas(t) => t.map(x, y),
            CopyMap::Uv(c) => c.map(x, y),
        }
    }
    #[inline]
    fn inverse(&self, x: f64, y: f64) -> (f64, f64) {
        match self {
            CopyMap::Canvas(t) => t.inverse(x, y),
            CopyMap::Uv(c) => c.inverse(x, y),
        }
    }
    /// 半径 extent の円の写しの、外接の箱の半分の幅と高さ（直交の変換は extent のまま）。
    fn reach(&self, extent: f64) -> (f64, f64) {
        match self {
            CopyMap::Canvas(_) => (extent, extent),
            CopyMap::Uv(c) => c.reach(extent),
        }
    }
}

/// 写しの写像と、キャンバスの中の外接の箱（x0, x1, y0, y1。両端を含む）。
type CopyBounds = (CopyMap, i64, i64, i64, i64);

/// 写しごとの、キャンバスの中の外接の箱（両端を含む）。候補の画素の数を先に数えて上限で断る（C# の VisitSymmetricPixels の前半）。
fn symmetric_bounds(
    transforms: &[CopyMap],
    x: f64,
    y: f64,
    extent: f64,
    width: i64,
    height: i64,
) -> Result<Vec<CopyBounds>, CoreError> {
    let mut work = 0i64;
    let mut bounds = Vec::with_capacity(transforms.len());
    for t in transforms {
        let (cx, cy) = t.map(x, y);
        let (ex, ey) = t.reach(extent);
        let x0 = ((cx - ex - 0.5).ceil() as i64).max(0);
        let x1 = ((cx + ex - 0.5).floor() as i64).min(width - 1);
        let y0 = ((cy - ey - 0.5).ceil() as i64).max(0);
        let y1 = ((cy + ey - 0.5).floor() as i64).min(height - 1);
        if x0 > x1 || y0 > y1 {
            continue;
        }
        work += (x1 - x0 + 1) * (y1 - y0 + 1);
        if work > MAX_SYMMETRY_CANDIDATE_PIXELS {
            return Err(CoreError::WorkingBudgetExceeded);
        }
        bounds.push((*t, x0, x1, y0, y1));
    }
    Ok(bounds)
}

impl DabShape<'_> {
    /// 中心からのずれ (dx, dy) の被覆率（C# の SymmetryCoverage。筆先の反転は拡張で、普通のダブと同じく効かせる）。
    #[inline(always)]
    fn coverage_at(&self, dx: f64, dy: f64) -> f64 {
        let mut u = (self.cos * dx + self.sin * dy) / self.radius;
        let mut v = (-self.sin * dx + self.cos * dy) / (self.radius * self.roundness);
        if let Some(tip) = self.tip {
            if self.flip_x {
                u = -u;
            }
            if self.flip_y {
                v = -v;
            }
            let c = tip.sample(
                (u / self.aspect_x + 1.0) * 0.5,
                (v / self.aspect_y + 1.0) * 0.5,
            );
            return if self.edge.density != 1.0 {
                c * self.edge.density
            } else {
                c
            };
        }
        let distance = if self.plain {
            (dx * dx + dy * dy).sqrt() / self.radius
        } else {
            (u * u + v * v).sqrt()
        };
        if !self.edge.is_off() {
            if self.plain {
                return super::edge::cover64(
                    distance,
                    self.hardness,
                    self.edge.band / self.radius,
                    self.edge.density,
                );
            }
            let minor = self.radius * self.roundness;
            let g = super::edge::ellipse_gradient(u, v, distance, self.radius, minor);
            let mid = 1.0 - (1.0 - self.hardness) * 0.5;
            let floored = super::edge::box_floor(
                distance,
                g,
                mid,
                (u * self.radius, v * minor),
                (self.radius * mid, minor * mid),
            );
            return super::edge::cover64_floored(
                distance,
                floored,
                self.hardness,
                self.edge.band * g,
                self.edge.density,
            );
        }
        if distance > 1.0 {
            return 0.0;
        }
        if distance <= self.hardness {
            return 1.0;
        }
        let t = (1.0 - distance) / (1.0 - self.hardness);
        t * t * (3.0 - 2.0 * t)
    }
}

impl StrokeState {
    /// 中心 (x, y)・半径 radius のダブの写しの写像（元の恒等を含む）。2D の対称も 3D の対称の写しも無いダブは None（普通のダブで
    /// 塗る）。3D の写しを作れなかった理由は `copy_note` に残す。
    pub(super) fn copy_maps(
        &mut self,
        x: f64,
        y: f64,
        radius: f64,
    ) -> Result<Option<Vec<CopyMap>>, CoreError> {
        let canvas = if self.brush.symmetry.enabled() {
            self.brush.symmetry.transforms()?
        } else {
            Vec::new()
        };
        let model = match self.brush.model_symmetry.clone() {
            Some(m) => {
                let c = m.copies(x, y, radius, self.width as u32, self.height as u32)?;
                if let Some(o) = c.outcome {
                    self.copy_note = Some(o);
                }
                c.copies
            }
            None => Vec::new(),
        };
        if model.is_empty() {
            return Ok(
                (!canvas.is_empty()).then(|| canvas.into_iter().map(CopyMap::Canvas).collect())
            );
        }
        // 3D の写しを先に、2D の写しを後に（元と 3D の写しのそれぞれを 2D の対称で写す）
        let canvas = if canvas.is_empty() {
            vec![SymmetryTransform::identity()]
        } else {
            canvas
        };
        let mut maps: Vec<CopyMap> = canvas.iter().map(|t| CopyMap::Canvas(*t)).collect();
        for c in &model {
            for t in &canvas {
                maps.push(CopyMap::Uv(if t.is_identity() { *c } else { c.then(t) }));
            }
        }
        Ok(Some(maps))
    }

    /// 3D の対称の写しを作れなかった最後の理由（面が無い・別のテクスチャセット）。
    pub(crate) fn copy_note(&self) -> Option<MirrorOutcome> {
        self.copy_note
    }

    /// 写し全部の画素と被覆率（同じ画素は大きい方）を、キャンバスの画素の番号（y × 幅 + x）の順に。
    fn symmetric_pixels(
        &self,
        transforms: &[CopyMap],
        x: f64,
        y: f64,
        extent: f64,
        coverage: impl Fn(f64, f64) -> f64,
    ) -> Result<Vec<(i64, f64)>, CoreError> {
        let bounds = symmetric_bounds(transforms, x, y, extent, self.width, self.height)?;
        let mut pixels = Vec::new();
        for (t, x0, x1, y0, y1) in bounds {
            for py in y0..=y1 {
                for px in x0..=x1 {
                    let (ox, oy) = t.inverse(px as f64 + 0.5, py as f64 + 0.5);
                    let c = coverage(ox - x, oy - y);
                    if c > 0.0 {
                        pixels.push((py * self.width + px, c));
                    }
                }
            }
        }
        pixels.sort_by_key(|p| p.0);
        pixels.dedup_by(|later, kept| {
            if later.0 != kept.0 {
                return false;
            }
            if later.1 > kept.1 {
                kept.1 = later.1;
            }
            true
        });
        Ok(pixels)
    }

    /// 対称のダブ（C# の SymmetricDab）。
    pub(super) fn symmetric_dab(
        &mut self,
        surface: &mut Surface,
        brush: &Brush,
        s: &DabShape<'_>,
        extent: f64,
        maps: &[CopyMap],
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let pixels =
            self.symmetric_pixels(maps, s.x, s.y, extent, |dx, dy| s.coverage_at(dx, dy))?;
        // 画素の集まりの場所（C# の辞書の容量と並べ替えのキー。保守的に 1 画素 96 バイト）もストロークの予算に数える
        // （C# は画素を 1 つ足すたびに確かめるので、画素が無ければ確かめない）
        if pixels.is_empty() {
            return Ok(false);
        }
        let scratch = 256 + pixels.len() as u64 * 96;
        if self.rollback_bytes + self.held_bytes() + scratch > self.budgets.stroke {
            return Err(CoreError::StrokeBudgetExceeded);
        }
        let w = self.width;
        if self.mix_on {
            return self.symmetric_mix_dab(surface, brush, s, &pixels, changed);
        }
        let (mut x0, mut y0, mut x1, mut y1) = (w, self.height, -1i64, -1i64);
        for &(key, _) in &pixels {
            let (px, py) = (key % w, key / w);
            x0 = x0.min(px);
            x1 = x1.max(px);
            y0 = y0.min(py);
            y1 = y1.max(py);
        }
        let frame = match self.prepare_dab(
            surface,
            brush,
            (x0, x1),
            (y0, y1),
            s.x,
            s.y,
            s.pressure,
            true,
        )? {
            Prepared::Skip => return Ok(false),
            Prepared::Paint => None,
            Prepared::Effect(f) => Some(f),
        };
        let result = self.symmetric_paint(surface, brush, s, frame.as_ref(), &pixels, changed);
        self.effect.scratch = 0;
        self.frame_cache = frame;
        result
    }

    /// 色の混ぜの対称のダブ: 写し全部の画素を、下地を読む範囲が離れた塊（離れた写しは別の塊）へ分け、塊ごとに枠を凍結して塗る。
    /// 写しは動きの向きが写しごとに違うので、伸ばすは動きの向きを使わず、同じ画素の周りの箱だけを読む（[`StrokeState::begin_mix_regions`]）。
    fn symmetric_mix_dab(
        &mut self,
        surface: &mut Surface,
        brush: &Brush,
        s: &DabShape<'_>,
        pixels: &[(i64, f64)],
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let w = self.width;
        let cells: Vec<(i64, i64)> = pixels.iter().map(|&(key, _)| (key % w, key / w)).collect();
        let regions = self.begin_mix_regions(&cells, s.pressure);
        let mut any = false;
        for region in regions {
            let frame = self.freeze_mix_region(surface, &region)?;
            let part: Vec<(i64, f64)> = region.members.iter().map(|&k| pixels[k]).collect();
            let result = self.symmetric_paint(surface, brush, s, Some(&frame), &part, changed);
            self.effect.scratch = 0;
            self.frame_cache = Some(frame);
            any |= result?;
        }
        Ok(any)
    }

    /// 集めた画素をタイルごとにまとめて塗る。
    fn symmetric_paint(
        &mut self,
        surface: &mut Surface,
        brush: &Brush,
        s: &DabShape<'_>,
        frame: Option<&EffectFrame>,
        pixels: &[(i64, f64)],
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let (w, ts) = (self.width, self.tile_size);
        let mut items: Vec<(TileCoord, usize, i64, i64, f64)> = pixels
            .iter()
            .map(|&(key, c)| {
                let (px, py) = (key % w, key / w);
                let coord = TileCoord::new((px / ts) as u32, (py / ts) as u32);
                (coord, ((py % ts) * ts + px % ts) as usize, px, py, c)
            })
            .collect();
        items.sort_by_key(|i| (i.0, i.1));
        let paint = self.paint(brush, frame);
        let mut any = false;
        let mut start = 0;
        while start < items.len() {
            let coord = items[start].0;
            let end = start + items[start..].iter().take_while(|i| i.0 == coord).count();
            let run = &items[start..end];
            start = end;
            let dual = self.dual_coverage.remove(&coord);
            let r = self.with_tile(surface, &paint, coord, |cx, held, live| {
                let mut painted = false;
                for &(_, local, px, py, c) in run {
                    let mut coverage = c;
                    if let Some(mode) = s.dual {
                        coverage =
                            mode.combine(coverage, dual.as_ref().map_or(0.0, |d| d[local] as f64));
                    }
                    let mut ceiling = s.opacity_scale;
                    let mut paper = None;
                    if let Some(tex) = s.texture {
                        let grain = tex.image.sample_tiled(
                            (px as f64 + 0.5) / tex.scale,
                            (py as f64 + 0.5) / tex.scale,
                        );
                        match tex.mode.as_blend() {
                            None => ceiling *= 1.0 - tex.depth * (1.0 - grain),
                            Some(m) => paper = Some((m, grain, tex.depth)),
                        }
                    }
                    if coverage > 0.0 && ceiling > 0.0 {
                        painted |= apply_at::<false>(
                            cx,
                            held,
                            live,
                            coord,
                            local,
                            coverage as f32,
                            s.pressure,
                            (ceiling as f32, s.flow_scale as f32),
                            paper.map(|(m, g, d)| (m, g as f32, d as f32)),
                            None,
                        )?;
                    }
                }
                Ok(painted)
            });
            if let Some(d) = dual {
                self.dual_coverage.insert(coord, d);
            }
            if r? {
                changed.push(coord);
                any = true;
            }
        }
        Ok(any)
    }

    /// 対称のデュアルのダブ（C# の DualDabAt の対称の枝）: 写し全部の画素の溜まりへ最大で置く。
    pub(super) fn symmetric_dual_dab(
        &mut self,
        shape: &DualShape<'_>,
        extent: f64,
        maps: &[CopyMap],
    ) -> Result<(), CoreError> {
        let bounds = symmetric_bounds(maps, shape.x, shape.y, extent, self.width, self.height)?;
        let ts = self.tile_size;
        for (t, x0, x1, y0, y1) in bounds {
            for py in y0..=y1 {
                for px in x0..=x1 {
                    let (ox, oy) = t.inverse(px as f64 + 0.5, py as f64 + 0.5);
                    let coverage = shape.coverage_at(ox - shape.x, oy - shape.y);
                    if coverage <= 0.0 {
                        continue;
                    }
                    let coord = TileCoord::new((px / ts) as u32, (py / ts) as u32);
                    if !self.dual_coverage.contains_key(&coord) {
                        let next = self.rollback_bytes + 64 + (ts * ts * 4) as u64;
                        self.ensure_budget(next)?;
                        self.dual_coverage
                            .insert(coord, vec![0.0; (ts * ts) as usize]);
                        self.rollback_bytes = next;
                    }
                    let cells = self.dual_coverage.get_mut(&coord).expect("直前に作った");
                    let local = ((py % ts) * ts + px % ts) as usize;
                    cells[local] = cells[local].max(coverage as f32);
                }
            }
        }
        Ok(())
    }
}
