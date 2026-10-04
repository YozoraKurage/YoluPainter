//! 色の混ぜの、打点の前の仕事（ストロークの側）: 下地を凍結した枠に読み、筆の荷と今の打点の値を決める。画素ごとの式は
//! [`super::mix`]、画素へ置く道は [`super::apply_at`]（今までと同じ）。

use super::mix::{next_carry, MixDab};
use super::*;

/// 平均の箱の半径の上限（画素）。タイルのつながりを決めるときの、読む範囲の広がり。
const MAX_MIX_BLUR: i64 = 8;

/// 打点の画素の塊（[`StrokeState::begin_mix_regions`]）: 画素を含むタイルがつながったもの。
pub(super) struct MixRegion {
    /// 塊の画素の外接の箱（両端を含む）。
    pub xr: (i64, i64),
    pub yr: (i64, i64),
    /// 渡された画素の並びの番号（渡された順）。
    pub members: Vec<usize>,
}

impl StrokeState {
    /// 打点の前に、読み元を凍結する: 色の混ぜは下地を、効果のブラシは読み元を（[`StrokeState::prepare_effect_dab`]）。
    /// `canvas_shift` は伸ばすが画布の中の打点の動き（前の打点との差）を読む位置のずれに使うか（2D の 1 つながりのダブ。3D の面のダブと
    /// 対称のダブは [`StrokeState::begin_mix_regions`] で動きの向きなしに読む）。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_dab(
        &mut self,
        surface: &Surface,
        brush: &Brush,
        xr: (i64, i64),
        yr: (i64, i64),
        x: f64,
        y: f64,
        pressure: PressureScale,
        canvas_shift: bool,
    ) -> Result<Prepared, CoreError> {
        if self.mix_on {
            return self.prepare_mix_dab(surface, xr, yr, x, y, pressure, canvas_shift);
        }
        self.prepare_effect_dab(surface, brush, xr, yr, x, y)
    }

    /// 色の混ぜの打点（2D の 1 つながりのダブ）: 前の打点を荷へ畳み、下地を読む範囲（混ぜるは打点の画素、伸ばすはずらした先の箱ごと）を
    /// 凍結して、今の打点の値（荷・絵の具の量と濃さ・ずれ・箱）を決める。ストロークの予算（枠のバイト）を超えたら断る（文書がストロークごと
    /// 取り消す）。
    #[allow(clippy::too_many_arguments)]
    fn prepare_mix_dab(
        &mut self,
        surface: &Surface,
        xr: (i64, i64),
        yr: (i64, i64),
        x: f64,
        y: f64,
        pressure: PressureScale,
        canvas_shift: bool,
    ) -> Result<Prepared, CoreError> {
        let (mode, stretch) = (self.brush.mix.mode, self.brush.mix.stretch);
        let smear = mode == MixMode::Smear;
        self.mix_run.settle();
        self.effect.scratch = 0;
        self.effect.started = true;
        let size = (xr.1 - xr.0 + 1).max(yr.1 - yr.0 + 1);
        // 伸ばす: 前の打点から今の打点への動きを、(1 + 2 × 色延び) 倍だけ後ろを読む（長さは打点の大きさまで）
        let (mut sx, mut sy) = (0i64, 0i64);
        if canvas_shift {
            let first = !self.effect.has_position;
            let (dx, dy) = (self.effect.x - x, self.effect.y - y);
            self.effect.has_position = true;
            self.effect.x = x;
            self.effect.y = y;
            if smear && !first {
                let reach = 1.0 + 2.0 * stretch;
                let (mut ox, mut oy) = (dx * reach, dy * reach);
                let length = (ox * ox + oy * oy).sqrt();
                if length > size as f64 {
                    let k = size as f64 / length;
                    ox *= k;
                    oy *= k;
                }
                sx = ox.round() as i64;
                sy = oy.round() as i64;
            }
        }
        let blur = self.mix_blur(size);
        let Some(frame) = self.freeze_mix_box(surface, xr, yr, (sx, sy), blur)? else {
            return Ok(Prepared::Skip);
        };
        self.mix_run.dab = Some(self.mix_dab(pressure, (sx, sy), blur));
        Ok(Prepared::Effect(frame))
    }

    /// 色の混ぜの面のダブ（3D）: 画素を塊へ分け、塊ごとに下地を凍結して塗る（[`StrokeState::begin_mix_regions`]）。
    pub(super) fn apply_mix_dab(
        &mut self,
        surface: &mut Surface,
        brush: &Brush,
        pixels: &[BrushPixel],
        pressure: PressureScale,
        points: Option<&[StencilPoint]>,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let inside: Vec<usize> = (0..pixels.len())
            .filter(|&i| {
                let p = &pixels[i];
                p.x >= 0 && p.y >= 0 && p.x < self.width && p.y < self.height
            })
            .collect();
        let cells: Vec<(i64, i64)> = inside.iter().map(|&i| (pixels[i].x, pixels[i].y)).collect();
        let regions = self.begin_mix_regions(&cells, pressure);
        let mut any = false;
        for region in regions {
            let frame = self.freeze_mix_region(surface, &region)?;
            let paint = self.paint(brush, Some(&frame));
            let result = self.paint_listed_pixels(
                surface,
                &paint,
                pressure,
                pixels,
                points,
                region.members.iter().map(|&k| inside[k]),
                changed,
            );
            self.effect.scratch = 0;
            self.frame_cache = Some(frame);
            any |= result?;
        }
        Ok(any)
    }

    /// 平均の箱の半径: 伸ばすは打点の半径の 1 割（1〜8 画素）、混ぜるは箱なし。
    fn mix_blur(&self, size: i64) -> i64 {
        if self.brush.mix.mode == MixMode::Smear {
            (((size as f64 / 2.0) * 0.1).round() as i64).clamp(1, MAX_MIX_BLUR)
        } else {
            0
        }
    }

    /// 画素の範囲 (xr, yr) の下地を読むために凍結する枠を作る（伸ばすは、ずらした先と平均の箱の分だけ広げる）。画布の外へ出ると空
    /// （None）。枠のバイトがストロークの予算を超えたら断る。
    fn freeze_mix_box(
        &mut self,
        surface: &Surface,
        xr: (i64, i64),
        yr: (i64, i64),
        shift: (i64, i64),
        blur: i64,
    ) -> Result<Option<EffectFrame>, CoreError> {
        let (mode, composite) = {
            let m = &self.brush.mix;
            (m.mode, m.ground == MixGround::Composite)
        };
        let (w, h) = (self.width, self.height);
        let (x0, x1, y0, y1) = if mode == MixMode::Smear {
            // 画素の読みの中心は画布の中へ収める（端の外は端の画素）ので、収めた範囲に箱の半径を足す
            (
                (xr.0 + shift.0).clamp(0, w - 1) - blur,
                (xr.1 + shift.0).clamp(0, w - 1) + blur,
                (yr.0 + shift.1).clamp(0, h - 1) - blur,
                (yr.1 + shift.1).clamp(0, h - 1) + blur,
            )
        } else {
            (xr.0, xr.1, yr.0, yr.1)
        };
        let (x0, x1, y0, y1) = (x0.max(0), x1.min(w - 1), y0.max(0), y1.min(h - 1));
        if x1 < x0 || y1 < y0 {
            return Ok(None);
        }
        let cells = ((x1 - x0 + 1) * (y1 - y0 + 1)) as u64;
        let bytes = cells * 4
            + if blur > 0 {
                ((x1 - x0 + 2) * (y1 - y0 + 2) * 32) as u64
            } else {
                0
            };
        if self
            .rollback_bytes
            .saturating_add(self.source_bytes())
            .saturating_add(bytes)
            > self.budgets.stroke
        {
            return Err(CoreError::StrokeBudgetExceeded);
        }
        self.effect.scratch = bytes;
        let (fw, fh) = (x1 - x0 + 1, y1 - y0 + 1);
        let mut frame = match self.frame_cache.take() {
            Some(f) => f.reset(x0, y0, fw, fh),
            None => EffectFrame::new(x0, y0, fw, fh),
        };
        // 混ぜるはストロークを始める前の絵を読む（同じストロークが画素に何度重なっても、混ざり方が変わらない）。伸ばすは今の面を読む
        // （先に引きずった色を後の打点が運ぶ）
        self.read_ground(
            surface,
            &mut frame,
            (x0, x1),
            (y0, y1),
            composite,
            mode == MixMode::Mix,
        );
        if blur > 0 {
            frame.build_integral();
        }
        Ok(Some(frame))
    }

    /// 面のダブ（3D）と対称のダブ（2D）の色の混ぜの打点の前の仕事: 前の打点を荷へ畳み、今の打点の値を決め、画素 `cells`（画布の中の
    /// 画素の座標）を、下地を読む範囲が離れた塊へ分けて返す。動きの向きは使わない（面のダブの中心は UV の継ぎ目で飛び、対称の写しは
    /// 動きの向きが写しごとに違う）ので、伸ばすは同じ画素の周りの箱だけを読む。
    ///
    /// 塊ごとに枠を凍結して順に塗る（[`StrokeState::freeze_mix_region`]）。1 つの打点の画素が UV の離れた島や対称の離れた写しにまたがるとき、
    /// 外接の箱を丸ごと枠にすると、画布の大半を読んで予算を食う。塊は、画素を含むタイルが（平均の箱の半径が届く分まで）つながるものを 1 つに
    /// するので、別の塊の画素は、ある塊の読む範囲に入らない。だから 1 塊ずつ凍結して塗っても、全部をまとめて凍結してから塗ったのと同じ画素
    /// になる。1 つにつながるダブは、今までと同じ 1 つの枠。
    pub(super) fn begin_mix_regions(
        &mut self,
        cells: &[(i64, i64)],
        pressure: PressureScale,
    ) -> Vec<MixRegion> {
        self.mix_run.settle();
        self.effect.scratch = 0;
        self.effect.started = true;
        let regions = self.split_mix_regions(cells);
        // 平均の箱の大きさは、一番大きい塊の大きさから（離れた島の間の距離は入れない）
        let size = regions
            .iter()
            .map(|r| (r.xr.1 - r.xr.0 + 1).max(r.yr.1 - r.yr.0 + 1))
            .max()
            .unwrap_or(1);
        self.mix_run.dab = Some(self.mix_dab(pressure, (0, 0), self.mix_blur(size)));
        regions
    }

    /// 塊の下地を枠へ凍結する（予算を超えたら断る）。`begin_mix_regions` の後に塊ごとに呼ぶ。枠の領域は、塗り終わったら
    /// `frame_cache` へ戻して次の塊で使い回す。
    pub(super) fn freeze_mix_region(
        &mut self,
        surface: &Surface,
        region: &MixRegion,
    ) -> Result<EffectFrame, CoreError> {
        let blur = self.mix_run.dab.map_or(0, |d| d.blur);
        Ok(self
            .freeze_mix_box(surface, region.xr, region.yr, (0, 0), blur)?
            .expect("塊の画素は画布の中"))
    }

    /// 画素を、含むタイルがつながる塊へ分ける（タイルの 8 近傍と、平均の箱の半径が届くタイルまで）。塊は最初の画素が現れた順、塊の中の
    /// 画素は渡された順。
    fn split_mix_regions(&self, cells: &[(i64, i64)]) -> Vec<MixRegion> {
        struct Group {
            tile: (i64, i64),
            xr: (i64, i64),
            yr: (i64, i64),
        }
        let ts = self.tile_size.max(1);
        let mut groups: Vec<Group> = Vec::new();
        let mut index: HashMap<(i64, i64), usize> = HashMap::new();
        let mut group_of: Vec<usize> = Vec::with_capacity(cells.len());
        let mut last: Option<((i64, i64), usize)> = None;
        for &(px, py) in cells {
            let tile = (px / ts, py / ts);
            let g = match last {
                Some((t, g)) if t == tile => g,
                _ => *index.entry(tile).or_insert_with(|| {
                    groups.push(Group {
                        tile,
                        xr: (px, px),
                        yr: (py, py),
                    });
                    groups.len() - 1
                }),
            };
            last = Some((tile, g));
            let group = &mut groups[g];
            group.xr = (group.xr.0.min(px), group.xr.1.max(px));
            group.yr = (group.yr.0.min(py), group.yr.1.max(py));
            group_of.push(g);
        }
        // タイルのつながり（箱の半径がタイルより大きいときは、届く数のタイルまで）
        let reach = ((MAX_MIX_BLUR + ts - 1) / ts).max(1);
        let mut parent: Vec<usize> = (0..groups.len()).collect();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for (g, group) in groups.iter().enumerate() {
            let (tx, ty) = group.tile;
            for dy in 0..=reach {
                for dx in -reach..=reach {
                    if dy == 0 && dx <= 0 {
                        continue; // 前向きの隣だけ見る（後ろ向きは相手が見る）
                    }
                    if let Some(&h) = index.get(&(tx + dx, ty + dy)) {
                        let (a, b) = (root(&mut parent, g), root(&mut parent, h));
                        if a != b {
                            // 若い番号を根に（塊の順を最初の画素の順にそろえる）
                            let (lo, hi) = (a.min(b), a.max(b));
                            parent[hi] = lo;
                        }
                    }
                }
            }
        }
        let mut region_of_root: HashMap<usize, usize> = HashMap::new();
        let mut regions: Vec<MixRegion> = Vec::new();
        let mut region_of_group: Vec<usize> = Vec::with_capacity(groups.len());
        for (g, group) in groups.iter().enumerate() {
            let r = root(&mut parent, g);
            let id = *region_of_root.entry(r).or_insert_with(|| {
                regions.push(MixRegion {
                    xr: group.xr,
                    yr: group.yr,
                    members: Vec::new(),
                });
                regions.len() - 1
            });
            let region = &mut regions[id];
            region.xr = (region.xr.0.min(group.xr.0), region.xr.1.max(group.xr.1));
            region.yr = (region.yr.0.min(group.yr.0), region.yr.1.max(group.yr.1));
            region_of_group.push(id);
        }
        for (i, g) in group_of.into_iter().enumerate() {
            regions[region_of_group[g]].members.push(i);
        }
        regions
    }

    /// 今の打点の値（荷・筆圧を掛けた絵の具の量と濃さ・読む位置のずれ・平均の箱）。
    fn mix_dab(&self, pressure: PressureScale, shift: (i64, i64), blur: i64) -> MixDab {
        let m = &self.brush.mix;
        let draw = if self.tip_colors {
            self.dab_color
        } else {
            self.stroke_color
        };
        MixDab {
            mode: m.mode,
            carry: next_carry(self.mix_run.loaded, draw, m.stretch),
            paint: m.paint * pressure.mix_paint,
            density: m.density * pressure.mix_density,
            shift,
            blur,
        }
    }

    /// 3D の面の伸ばし（指先と同じ写像されたダブ）の打点: 前の打点を荷へ畳み、今の打点の値を決める。読み元は呼び手が決める参照の
    /// 色（`apply_mapped_dab` が書く前にまとめて凍結する）なので、枠も、ずれも、箱も無い。
    pub(super) fn begin_mapped_mix_dab(&mut self, pressure: PressureScale) {
        self.mix_run.settle();
        self.mix_run.dab = Some(self.mix_dab(pressure, (0, 0), 0));
    }

    /// 下地を枠へ読む: 見えている層の重なり（凍結した合成の参照元があれば）か、今の層。今の層は、`from_start` ならストロークを始める前の
    /// 画素（もう手を付けたタイルは巻き戻しの写しを読む）、そうでなければ今の面（このストロークが置いた分を含む）。
    fn read_ground(
        &self,
        surface: &Surface,
        frame: &mut EffectFrame,
        xr: (i64, i64),
        yr: (i64, i64),
        composite: bool,
        from_start: bool,
    ) {
        let ts = surface.tile_size() as i64;
        let source = if composite {
            self.source.as_ref()
        } else {
            None
        };
        for py in yr.0..=yr.1 {
            let ty = py / ts;
            let row = ((py - ty * ts) * ts) as usize;
            let mut px = xr.0;
            while px <= xr.1 {
                let tx = px / ts;
                let end = xr.1.min(tx * ts + ts - 1);
                let len = (end - px + 1) as usize;
                let out = frame.row_mut(py, px, len);
                if let Some(source) = source {
                    source.read_row(py, px, out);
                } else {
                    let coord = TileCoord::new(tx as u32, ty as u32);
                    let tile: Option<&Tile> = match (from_start, self.tiles.get(&coord)) {
                        (true, Some(held)) => held.before.as_ref(),
                        _ => surface.tile(coord),
                    };
                    match tile {
                        None => {} // 枠は透明で始まる
                        Some(Tile::Uniform(c)) => out.fill(*c),
                        Some(Tile::Data(d)) => {
                            let start = (row + (px - tx * ts) as usize) * 4;
                            for (o, p) in out
                                .iter_mut()
                                .zip(d[start..start + len * 4].chunks_exact(4))
                            {
                                *o = Rgba8::from_slice(p);
                            }
                        }
                    }
                }
                px = end + 1;
            }
        }
    }
}
