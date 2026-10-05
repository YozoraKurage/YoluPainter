//! 結合の前後をチャンネルごとに比較し、許容差内の結果だけを記録する。
use super::eval::EvalSet;
use super::operations::Dirty;
use super::Document;
use crate::composite::{self, Stack};
use crate::effects::EffectSettings;
use crate::surface::Tile;
use crate::{
    BlendMode, Channel, ChannelKind, CoreError, Layer, LayerId, LayerKind, LayerLocks, Rgba8,
    Surface, TileCoord,
};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMethod {
    IntoClippingBase,
    OntoLowerLayer,
    Isolated,
    Group,
    Visible,
    Layers,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerMergeReport {
    pub result_id: LayerId,
    pub method: MergeMethod,
    /// C# MergeNotes のビット（1: フィルター・Generator を画素へ焼いた、2: パスを画素にした、4: 隠した子を除去、8: 無効チャンネルを除去）。
    pub notes: u8,
    pub compared_pixels: u64,
    pub changed_pixels: u64,
    pub max_difference: u8,
    pub max_visible_difference: u8,
    pub changed_by_channel: BTreeMap<Channel, u64>,
}
impl LayerMergeReport {
    pub fn exact(&self) -> bool {
        self.changed_pixels == 0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeRefusal {
    NoLayerBelow,
    LayerBelowIsGroup,
    LayerBelowIsAdjustment,
    HiddenLayer,
    IsGroup,
    NotGroup,
    EmptyGroup,
    NothingVisible,
    DifferentGroups,
}
/// 利用者に見せる短い状態（どの結合を断ったかの理由。使い方の説明にはしない）。
impl std::fmt::Display for MergeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MergeRefusal::NoLayerBelow => "下に層が無い",
            MergeRefusal::LayerBelowIsGroup => "下がグループ",
            MergeRefusal::LayerBelowIsAdjustment => "下が調整層",
            MergeRefusal::HiddenLayer => "非表示の層がある",
            MergeRefusal::IsGroup => "対象がグループ",
            MergeRefusal::NotGroup => "グループではない",
            MergeRefusal::EmptyGroup => "グループが空",
            MergeRefusal::NothingVisible => "表示している層が無い",
            MergeRefusal::DifferentGroups => "親のグループが違う",
        })
    }
}

/// 段のスタックが段を持つか（有効かどうかは問わない。C# の MergeNotes が数える形）。
fn has_stack(stack: &[crate::FilterEffect]) -> bool {
    !stack.is_empty()
}

/// 結合が焼いた効果とパスの印（C# の MergeNotes: 1 はフィルター・Generator、2 はパス。層の画素とマスクのスタックを数える）。
fn effect_notes(l: &Layer) -> u8 {
    let mut notes = 0;
    if has_stack(&l.filters) || l.mask.as_ref().is_some_and(|m| has_stack(&m.filters)) {
        notes |= 1;
    }
    if l.path.is_some() {
        notes |= 2;
    }
    notes
}

impl Document {
    pub const MERGE_ROUNDING_TOLERANCE: u8 = 2;
    pub fn merge_down_refusal(&self, id: LayerId) -> Result<Option<MergeRefusal>, CoreError> {
        let i = self.index_of(id)?;
        let l = &self.layers[i];
        if l.is_group() {
            return Ok(Some(MergeRefusal::IsGroup));
        }
        let Some(lower) = self.layers[..i].iter().rev().find(|b| b.parent == l.parent) else {
            return Ok(Some(MergeRefusal::NoLayerBelow));
        };
        Ok(if lower.is_group() {
            Some(MergeRefusal::LayerBelowIsGroup)
        } else if lower.kind == LayerKind::Adjustment {
            Some(MergeRefusal::LayerBelowIsAdjustment)
        } else if !l.visible || !lower.visible {
            Some(MergeRefusal::HiddenLayer)
        } else {
            None
        })
    }
    /// 層がそのチャンネルに出し得るタイル。ぼかしは元のタイルの外へ届く分だけ広げる（C# の OutputContentTiles）。Generator など
    /// 元の画素が無い所へ出す効果のタイルは、評価した出力の面（`merge_eval`）が持つので、結合は面のタイルも足す。
    fn content_coords(&self, l: &Layer, c: Channel) -> Vec<TileCoord> {
        match l.kind {
            LayerKind::Raster => {
                let raw = l.surface(c).map_or_else(Vec::new, Surface::tile_coords);
                let reach: u32 = l
                    .active_chain(c)
                    .iter()
                    .filter(|e| e.settings.expands_coverage())
                    .map(|e| e.settings.halo())
                    .sum();
                if reach == 0 {
                    raw
                } else {
                    self.grown_tiles(&raw, reach)
                }
            }
            LayerKind::Group => Vec::new(),
            _ if l.has_content(
                c,
                l.adjustment
                    .as_ref()
                    .is_some_and(|a| a.applies_to(self.channel_kind(c).expect("チャンネル"))),
            ) =>
            {
                self.canvas_tiles().collect()
            }
            _ => Vec::new(),
        }
    }
    /// タイルの並びを、画素の距離 `reach` が届く分だけ周りへ広げる（画布の外へは出ない。並びは Y・X の順）。
    fn grown_tiles(&self, tiles: &[TileCoord], reach: u32) -> Vec<TileCoord> {
        let ts = self.tile_size;
        let m = reach.div_ceil(ts);
        let (columns, rows) = (self.width.div_ceil(ts), self.height.div_ceil(ts));
        let set: BTreeSet<TileCoord> = tiles
            .iter()
            .flat_map(|c| {
                (c.y.saturating_sub(m)..=(c.y + m).min(rows - 1)).flat_map(move |y| {
                    (c.x.saturating_sub(m)..=(c.x + m).min(columns - 1))
                        .map(move |x| TileCoord::new(x, y))
                })
            })
            .collect();
        set.into_iter().collect()
    }
    /// 結合で読む層（`layers`。準備のために並べ直した写しでよい）の、チャンネルの評価した出力。`doc_index[k]` は `layers[k]` の文書での
    /// 番号。効果が無ければ空（結合は保存している画素をそのまま読む）。評価した面のタイル（効果が元の画素の無い所へ出したものも）を
    /// 結合が読むタイルに足せるよう、面のタイルの並びも返す。
    fn merge_eval(
        &self,
        layers: &[Layer],
        doc_index: &[usize],
        c: Channel,
        kind: ChannelKind,
    ) -> Result<(EvalSet, BTreeSet<TileCoord>), CoreError> {
        let eval = self.evaluate_slice(layers, doc_index, c, kind, None)?;
        let tiles = eval
            .content
            .values()
            .flat_map(Surface::tile_coords)
            .collect();
        Ok((eval, tiles))
    }
    /// 効果を焼き込む結合の前に、効いていない効果（使えるマップが無い Generator・出ていないデカール）が無いか確かめる。焼き込むと
    /// その効果が落ちるので断る（C# の RefuseInactiveGenerators）。`content` は層の画素のスタックとデカール、`mask` はマスクのスタック。
    fn refuse_inactive_effects(
        &self,
        index: usize,
        content: bool,
        mask: bool,
    ) -> Result<(), CoreError> {
        let l = &self.layers[index];
        if content && l.is_decal() && l.fill.keys().any(|c| l.is_channel_enabled(*c)) {
            if let Some(why) = self.decal_problem(index) {
                return Err(CoreError::InactiveEffect {
                    layer: l.id,
                    mask: false,
                    reason: Box::new(why),
                });
            }
        }
        let stacks = [
            (content, false, l.filters.as_slice()),
            (
                mask,
                true,
                l.mask.as_ref().map_or(&[][..], |m| m.filters.as_slice()),
            ),
        ];
        for (wanted, in_mask, stack) in stacks {
            if !wanted {
                continue;
            }
            for e in stack {
                if let (EffectSettings::Generator(g), true) = (&e.settings, e.is_active()) {
                    if let Some(reason) = self.generator_reason(g, index) {
                        return Err(CoreError::InactiveEffect {
                            layer: l.id,
                            mask: in_mask,
                            reason: Box::new(reason),
                        });
                    }
                }
            }
        }
        Ok(())
    }
    /// 画素ごとの式 render(x, y) で組んだ面（結合の方法ごとに式が違う `merge_down` 用）。
    fn merge_surface(
        &self,
        coords: &BTreeSet<TileCoord>,
        budget: &mut u64,
        render: impl Fn(u32, u32) -> Rgba8 + Sync,
    ) -> Result<Surface, CoreError> {
        let ts = self.tile_size;
        self.merge_surface_tiles(coords, budget, |batch| {
            batch
                .par_iter()
                .map(|&coord| {
                    let mut bytes = vec![0; (ts * ts * 4) as usize];
                    for y in 0..ts.min(self.height - coord.y * ts) {
                        for x in 0..ts.min(self.width - coord.x * ts) {
                            let p = render(coord.x * ts + x, coord.y * ts + y);
                            let at = ((y * ts + x) * 4) as usize;
                            bytes[at..at + 4].copy_from_slice(&p.to_array());
                        }
                    }
                    Tile::from_bytes(&bytes)
                })
                .collect()
        })
    }
    /// 計画（plan）の合成だけを焼く面（表示に寄与する層・複数の層・グループの結合）。バッチ（`merge_surface_tiles`）ごとに計画を 1 回組み、
    /// バッチのタイルをまとめてワーカーへ分ける（`composite::composite_tiles_with`。タイルごとに計画を組み直さない。組む回数はバッチの数）。
    /// 合成・アルファ 0 の画素の RGB の 0 揃え・タイルの圧縮は、タイルを合成したワーカーが続けて行う（並列の段は 1 つ）。
    /// 画素は、画素ごとの `composite::evaluate_pixel` と同じバイトになる。
    fn merge_surface_of_plan(
        &self,
        coords: &BTreeSet<TileCoord>,
        budget: &mut u64,
        stack: &Stack<'_>,
    ) -> Result<Surface, CoreError> {
        let ts = self.tile_size as usize;
        self.merge_surface_tiles(coords, budget, |batch| {
            let regions: Vec<crate::Rect> = batch
                .iter()
                .map(|&c| self.tile_rect(c).expect("結合が読むタイルは画布の中にある"))
                .collect();
            composite::composite_tiles_with(stack, self.tile_size, &regions, None, |i, image| {
                // 矩形の画素を、ts × ts の 0 埋めの行（下から）に置く
                let row = regions[i].width as usize * 4;
                let mut bytes = vec![0u8; ts * ts * 4];
                for (dst, src) in bytes.chunks_exact_mut(ts * 4).zip(image.chunks_exact(row)) {
                    dst[..row].copy_from_slice(src);
                }
                for p in bytes.as_chunks_mut::<4>().0 {
                    if p[3] == 0 {
                        *p = [0; 4];
                    }
                }
                Tile::from_bytes(&bytes)
            })
        })
    }
    /// タイルのバッチごとに render(座標の並び) → 座標ごとのタイル（画素が無ければ None）で組んだ面。タイルの計算はバッチの中で並列に行い、
    /// 予算の確かめと書き込みは座標の順にこのスレッドで行う。
    fn merge_surface_tiles(
        &self,
        coords: &BTreeSet<TileCoord>,
        budget: &mut u64,
        render: impl Fn(&[TileCoord]) -> Vec<Option<Tile>>,
    ) -> Result<Surface, CoreError> {
        let mut out = Surface::new(self.width, self.height, self.tile_size);
        let coords: Vec<_> = coords.iter().copied().collect();
        for batch in coords.chunks(rayon::current_num_threads().clamp(1, 64) * 2) {
            let tiles = render(batch);
            debug_assert_eq!(tiles.len(), batch.len());
            for (&coord, t) in batch.iter().zip(tiles) {
                if let Some(t) = t {
                    *budget += 64 + t.byte_size();
                    if *budget > self.stroke_budget {
                        return Err(CoreError::StrokeBudgetExceeded);
                    }
                    out.restore(coord, Some(&t));
                }
            }
        }
        Ok(out)
    }
    fn plain_layer(&self, l: &Layer) -> bool {
        l.blend_mode == BlendMode::Normal
            && l.opacity == 1.
            && l.channel_blends()
                .all(|(c, _)| l.blend_mode_in(c) == BlendMode::Normal && l.opacity_in(c) == 1.)
            && l.mask.as_ref().is_none_or(|m| m.is_neutral())
    }
    fn nothing_below(&self, i: usize) -> bool {
        let l = &self.layers[i];
        if let Some(p) = l.parent {
            let p = self.layer(p).expect("親");
            if self
                .channels()
                .iter()
                .any(|&c| p.blend_mode_in(c) == BlendMode::PassThrough)
            {
                return false;
            }
        }
        !self.layers[..i]
            .iter()
            .any(|b| b.parent == l.parent && b.visible)
    }
    pub fn merge_down(
        &mut self,
        id: LayerId,
        tolerance: u8,
    ) -> Result<LayerMergeReport, CoreError> {
        self.ensure_no_stroke()?;
        if let Some(r) = self.merge_down_refusal(id)? {
            return Err(CoreError::MergeRefused(r));
        }
        let ui = self.index_of(id)?;
        let upper = &self.layers[ui];
        let li = (0..ui)
            .rev()
            .find(|&i| self.layers[i].parent == upper.parent)
            .expect("下の層");
        let lower = &self.layers[li];
        self.ensure_pixels_editable(id, false)?;
        self.ensure_pixels_editable(lower.id, false)?;
        // 効果は焼き込まれる（結果は効果を持たない）ので、効いていない効果があれば落とさず断る。下の層のマスクは、残すとき
        // （分離の結合でないとき）は効果ごと結果へ写るので、ここでは見ない（C# の RefuseInactiveGenerators(upper, 内容, マスク)・(lower, 内容)）。
        // 分離の結合のときだけ、方法が決まったあとで見る（下）
        self.refuse_inactive_effects(ui, true, true)?;
        self.refuse_inactive_effects(li, true, false)?;
        let uc = self.is_effectively_clipped(ui);
        let lc = self.is_effectively_clipped(li);
        let method = if uc && !lc {
            MergeMethod::IntoClippingBase
        } else if !uc && !lc && !self.plain_layer(lower) && self.nothing_below(li) {
            MergeMethod::Isolated
        } else {
            MergeMethod::OntoLowerLayer
        };
        // 分離の結合は下の層のマスクのフィルターも画素へ焼き（結果はマスクを持たない）、焼けば効いていない Generator の設定が落ちる。
        // C# はこの検査を持たず入力のまま通して黙って落とす。Rust は意図して断る（OntoLowerLayer・IntoClippingBase はマスクが効果ごと残る）
        if method == MergeMethod::Isolated {
            self.refuse_inactive_effects(li, false, true)?;
        }
        if method != MergeMethod::IntoClippingBase {
            self.refuse_lock(lower.id, LayerLocks::TRANSPARENCY)?;
        }
        let mut result = Layer::new(
            LayerId(super::random_id(self.id_counter)),
            &lower.name,
            LayerKind::Raster,
        );
        result.parent = lower.parent;
        result.locks = lower.locks;
        if method != MergeMethod::Isolated {
            result.visible = lower.visible;
            result.opacity = lower.opacity;
            result.blend_mode = lower.blend_mode;
            result.clipping = lower.clipping;
            result.blends = lower.blends.clone();
            result.mask = lower.mask.clone();
        }
        let mut notes = 0;
        // 焼いた効果とパスの印（C# の MergeNotes。数えるのは段を持つかで、効いているかではない）
        if has_stack(upper.filters.as_slice())
            || has_stack(lower.filters.as_slice())
            || upper.mask.as_ref().is_some_and(|m| has_stack(&m.filters))
            || method == MergeMethod::Isolated
                && lower.mask.as_ref().is_some_and(|m| has_stack(&m.filters))
        {
            notes |= 1;
        }
        if upper.path.is_some() || lower.path.is_some() {
            notes |= 2;
        }
        let mut budget = 0;
        let mut output_tiles: BTreeMap<Channel, BTreeSet<TileCoord>> = BTreeMap::new();
        for c in self.channels() {
            let kind = self.channel_kind(c)?;
            let normal = kind == ChannelKind::Normal;
            let has = lower.surface(c).is_some() || lower.fill.contains_key(&c);
            let on = has && lower.is_channel_enabled(c);
            let mut upper_shell = upper.clone();
            upper_shell.parent = None;
            upper_shell.clipping = false;
            let upper_layers = [upper_shell];
            // 計画（どの層が出るか）は評価に依らない。評価した出力は、効果を使う所で（下で）作る
            let upper_plan = Stack::new(&upper_layers, c, kind, None).plan();
            let upper_on = !upper_plan.is_empty()
                && (method != MergeMethod::IntoClippingBase || on && lower.opacity_in(c) > 0.);
            if upper.surface(c).is_some_and(|s| s.tile_count() > 0) && !upper.is_channel_enabled(c)
            {
                notes |= 8;
            }
            if !has && !upper_on {
                continue;
            }
            let mut coords: BTreeSet<_> = self.content_coords(lower, c).into_iter().collect();
            if upper_on {
                coords.extend(self.content_coords(upper, c));
            }
            if !on && has && !upper_on {
                if lower.kind == LayerKind::Fill {
                    notes |= 8;
                }
                let surface = lower
                    .surface(c)
                    .cloned()
                    .unwrap_or_else(|| Surface::new(self.width, self.height, self.tile_size));
                for coord in surface.tile_coords() {
                    budget += 64 + surface.tile(coord).expect("タイル").byte_size();
                }
                if budget > self.stroke_budget {
                    return Err(CoreError::StrokeBudgetExceeded);
                }
                result.put_surface(c, Some(surface));
                result.set_enabled(c, false);
                continue;
            }
            if !on && has {
                notes |= 8;
            }
            let mut isolated = vec![lower.clone(), upper.clone()];
            for l in &mut isolated {
                l.parent = None;
                l.clipping = false;
            }
            // 下の層（0）と上の層（1）の評価した出力。結合は保存している画素でなく、合成が読む出力を焼く
            let (eval, evaluated_tiles) = self.merge_eval(&isolated, &[li, ui], c, kind)?;
            if upper_on {
                coords.extend(
                    eval.content
                        .get(&1)
                        .map_or_else(Vec::new, Surface::tile_coords),
                );
            }
            if on {
                coords.extend(
                    eval.content
                        .get(&0)
                        .map_or_else(Vec::new, Surface::tile_coords),
                );
            }
            output_tiles.entry(c).or_default().extend(evaluated_tiles);
            let upper_eval = eval.only(1, 0);
            let upper_stack = Stack::new(&upper_layers, c, kind, Some(&upper_eval));
            let isolated_stack = Stack::new(&isolated, c, kind, Some(&eval));
            let isolated_plan = isolated_stack.plan();
            let surface = self.merge_surface(&coords, &mut budget, |x, y| {
                // 保存している画素（透明の下に残る RGB を守る所で使う）と、層の出力（効果を通した画素）
                let raw = lower.pixel_or_transparent(c, x, y);
                let below = if on {
                    isolated_stack.layer_pixel(0, x, y)
                } else {
                    Rgba8::TRANSPARENT
                };
                let p = match method {
                    MergeMethod::Isolated => composite::evaluate_pixel(
                        &isolated_stack,
                        &isolated_plan,
                        Rgba8::TRANSPARENT,
                        x,
                        y,
                    ),
                    MergeMethod::IntoClippingBase
                        if upper_on
                            && (lower.kind == LayerKind::Fill
                                || eval.content.get(&0).map_or_else(
                                    || {
                                        lower.surface(c).is_some_and(|s| {
                                            s.has_tile(TileCoord::new(
                                                x / self.tile_size,
                                                y / self.tile_size,
                                            ))
                                        })
                                    },
                                    |s| {
                                        s.has_tile(TileCoord::new(
                                            x / self.tile_size,
                                            y / self.tile_size,
                                        ))
                                    },
                                )) =>
                    {
                        let amount = upper.opacity_in(c) * upper_stack.mask_factor(0, x, y);
                        if let Some(a) = &upper.adjustment {
                            a.composite(below, amount, upper.blend_mode_in(c))
                        } else {
                            let p = upper_stack.layer_pixel(0, x, y);
                            if normal {
                                crate::normal::clip_onto(below, p, amount, upper.blend_mode_in(c))
                            } else {
                                crate::blend::clip_onto(below, p, amount, upper.blend_mode_in(c))
                            }
                        }
                    }
                    MergeMethod::OntoLowerLayer if upper_on => {
                        composite::evaluate_pixel(&upper_stack, &upper_plan, below, x, y)
                    }
                    _ => below,
                };
                if p.a != 0 {
                    p
                } else if on && raw.a == 0 {
                    raw
                } else {
                    Rgba8::TRANSPARENT
                }
            })?;
            if !has && surface.tile_count() == 0 {
                continue;
            }
            result.put_surface(c, Some(surface));
            result.set_enabled(c, on || upper_on);
        }
        // Anchor: 結果までの合成は上の層までの合成と同じなので、上の層の Anchor を結果へ移す（読む段はそのまま使える）。残したマスクの
        // Anchor は、マスクごと写っている（lower.mask の複製）。下の層の Anchor（上の層を含まない合成）は表せないので無くなり、
        // 読む段は理由を出して入力のまま通す
        result.anchor = upper.anchor.clone();
        let removed = vec![lower.id, upper.id];
        let mut copy = self.edit_copy()?;
        copy.renew_kept_mask_filter_ids(&mut result);
        copy.layers.remove(ui);
        copy.layers[li] = result;
        self.finish_merge(copy, &removed, method, notes, tolerance, &output_tiles)
    }
    /// 表示に寄与する層を結合。非表示・不透明度ゼロの層と、それを持つグループは残す。
    pub fn merge_visible(
        &mut self,
        name: &str,
        tolerance: u8,
    ) -> Result<LayerMergeReport, CoreError> {
        self.ensure_no_stroke()?;
        fn walk(entries: &[composite::Entry], layers: &[Layer], out: &mut HashSet<LayerId>) {
            for e in entries {
                out.insert(layers[e.layer].id);
                walk(&e.children, layers, out);
                walk(&e.clips, layers, out);
            }
        }
        let mut removed = HashSet::new();
        for c in self.channels() {
            walk(
                &Stack::new(&self.layers, c, self.channel_kind(c)?, None).plan(),
                &self.layers,
                &mut removed,
            );
        }
        if removed.is_empty() {
            return Err(CoreError::MergeRefused(MergeRefusal::NothingVisible));
        }
        removed.retain(|id| !self.layer(*id).expect("層").is_group());
        self.ensure_members_editable(&removed)?;
        let mut notes = self.disabled_notes(&removed);
        for (i, l) in self.layers.iter().enumerate() {
            if removed.contains(&l.id) {
                self.refuse_inactive_effects(i, true, true)?;
                notes |= effect_notes(l);
            }
        }
        for (i, l) in self.layers.iter().enumerate() {
            if !l.is_group() {
                continue;
            }
            let descendants: Vec<_> = (0..i).filter(|&j| self.is_descendant(j, l.id)).collect();
            if !descendants.is_empty()
                && descendants
                    .iter()
                    .all(|&j| removed.contains(&self.layers[j].id))
            {
                removed.insert(l.id);
            }
        }
        let mut result = Layer::new(
            LayerId(super::random_id(self.id_counter)),
            name,
            LayerKind::Raster,
        );
        let mut budget = 0;
        let in_place: Vec<usize> = (0..self.layers.len()).collect();
        let mut output_tiles: BTreeMap<Channel, BTreeSet<TileCoord>> = BTreeMap::new();
        for c in self.channels() {
            let kind = self.channel_kind(c)?;
            if Stack::new(&self.layers, c, kind, None).plan().is_empty() {
                continue;
            }
            // 全部の層の評価した出力（効果を焼く）。評価した面のタイルは、元の画素の無い所へ出した効果の分も読むタイルに足す
            let (eval, evaluated_tiles) = self.merge_eval(&self.layers, &in_place, c, kind)?;
            let mut coords: BTreeSet<_> = self
                .layers
                .iter()
                .flat_map(|l| self.content_coords(l, c))
                .collect();
            coords.extend(evaluated_tiles.iter().copied());
            output_tiles.entry(c).or_default().extend(evaluated_tiles);
            let stack = Stack::new(&self.layers, c, kind, Some(&eval));
            let surface = self.merge_surface_of_plan(&coords, &mut budget, &stack)?;
            if surface.tile_count() > 0 {
                result.put_surface(c, Some(surface));
                result.set_enabled(c, true);
            }
        }
        let top = (0..self.layers.len())
            .rev()
            .find(|&i| {
                self.layers[i].parent.is_none()
                    && self.layers.iter().enumerate().any(|(j, l)| {
                        removed.contains(&l.id)
                            && (i == j || self.is_descendant(j, self.layers[i].id))
                    })
            })
            .expect("最上位");
        let insert = self.layers[..=top]
            .iter()
            .filter(|l| !removed.contains(&l.id))
            .count();
        let mut copy = self.edit_copy()?;
        copy.layers.retain(|l| !removed.contains(&l.id));
        copy.layers.insert(insert, result);
        self.finish_merge(
            copy,
            &removed.into_iter().collect::<Vec<_>>(),
            MergeMethod::Visible,
            notes,
            tolerance,
            &output_tiles,
        )
    }
    /// 外す層の画素が編集できるか（ロック）を、文書の並びの順（下から）に確かめる。最初に断った層を返すので、どの層を名指すかは
    /// 集合の並びで変わらない。
    fn ensure_members_editable(&self, removed: &HashSet<LayerId>) -> Result<(), CoreError> {
        self.layers
            .iter()
            .filter(|l| removed.contains(&l.id))
            .try_for_each(|l| self.ensure_pixels_editable(l.id, false))
    }
    fn disabled_notes(&self, removed: &HashSet<LayerId>) -> u8 {
        if self
            .layers
            .iter()
            .filter(|l| removed.contains(&l.id))
            .any(|l| {
                l.surface_channels().iter().any(|&c| {
                    !l.is_channel_enabled(c) && l.surface(c).is_some_and(|s| s.tile_count() > 0)
                })
            })
        {
            8
        } else {
            0
        }
    }
    pub fn merge_layers(
        &mut self,
        ids: &[LayerId],
        tolerance: u8,
    ) -> Result<LayerMergeReport, CoreError> {
        self.ensure_no_stroke()?;
        let members = self.topmost_of(ids)?;
        if members.len() < 2 {
            return Err(CoreError::InvalidArgument("結合は 2 層以上"));
        }
        let parent = self.layer(members[0]).expect("対象").parent;
        if members
            .iter()
            .any(|&id| self.layer(id).expect("対象").parent != parent)
        {
            return Err(CoreError::MergeRefused(MergeRefusal::DifferentGroups));
        }
        if members
            .iter()
            .any(|&id| !self.layer(id).expect("対象").visible)
        {
            return Err(CoreError::MergeRefused(MergeRefusal::HiddenLayer));
        }
        let bases: Vec<_> = members
            .iter()
            .map(|&id| {
                let i = self.index_of(id).expect("対象");
                if !self.is_effectively_clipped(i) {
                    return None;
                }
                let mut base = None;
                for l in self.layers[..i].iter().rev().filter(|l| l.parent == parent) {
                    base = Some(l.id);
                    if !l.clipping {
                        break;
                    }
                }
                base
            })
            .collect();
        let clipped = bases[0].is_some() && bases.iter().all(|b| *b == bases[0]);
        self.merge_blocks(&members, None, clipped, tolerance)
    }
    pub fn merge_group(
        &mut self,
        id: LayerId,
        tolerance: u8,
    ) -> Result<LayerMergeReport, CoreError> {
        self.ensure_no_stroke()?;
        let l = self.layer(id).ok_or(CoreError::LayerNotFound)?;
        if !l.is_group() {
            return Err(CoreError::MergeRefused(MergeRefusal::NotGroup));
        }
        if !self.layers.iter().any(|l| l.parent == Some(id)) {
            return Err(CoreError::MergeRefused(MergeRefusal::EmptyGroup));
        }
        self.merge_blocks(&[id], Some(id), false, tolerance)
    }
    fn merge_blocks(
        &mut self,
        members: &[LayerId],
        group: Option<LayerId>,
        clipped: bool,
        tolerance: u8,
    ) -> Result<LayerMergeReport, CoreError> {
        let removed: HashSet<_> = self
            .layers
            .iter()
            .enumerate()
            .filter(|(i, l)| {
                members
                    .iter()
                    .any(|&id| id == l.id || self.is_descendant(*i, id))
            })
            .map(|(_, l)| l.id)
            .collect();
        self.ensure_members_editable(&removed)?;
        // 効果は焼き込まれる: 効いていない効果は落とさず断る。グループの結合では、グループ自身のマスクは結果へ効果ごと写るので
        // 焼かない（見ない）。選んだ層の結合では、選んだグループのマスクも焼かれる（C# の MergeGroup・MergeLayers）
        let mut effect_bits = 0;
        for (i, l) in self.layers.iter().enumerate() {
            if !removed.contains(&l.id) || Some(l.id) == group {
                continue;
            }
            self.refuse_inactive_effects(i, group.is_some() || !l.is_group(), true)?;
            effect_bits |= effect_notes(l);
        }
        let top = self.layer(*members.last().expect("対象")).expect("層");
        let mut result = Layer::new(
            LayerId(super::random_id(self.id_counter)),
            &top.name,
            LayerKind::Raster,
        );
        result.parent = top.parent;
        result.clipping = clipped;
        if group.is_some() {
            result.opacity = top.opacity;
            result.blend_mode = if top.blend_mode == BlendMode::PassThrough {
                BlendMode::Normal
            } else {
                top.blend_mode
            };
            result.visible = top.visible;
            result.clipping = top.clipping;
            result.mask = top.mask.clone();
            result.locks = top.locks;
            // Anchor: 結果までの合成はグループまでの合成と同じなので、グループの Anchor を結果へ移す（マスクの Anchor はマスクごと写る）。
            // 中の層の Anchor は無くなる
            result.anchor = top.anchor.clone();
            result.blends = top.blends.clone();
            for b in result.blends.values_mut() {
                if b.mode == Some(BlendMode::PassThrough) {
                    b.mode = Some(BlendMode::Normal);
                }
            }
        }
        let (doc_index, mut isolated): (Vec<usize>, Vec<Layer>) = self
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| removed.contains(&l.id) && Some(l.id) != group)
            .map(|(i, l)| (i, l.clone()))
            .unzip();
        for l in &mut isolated {
            if group.is_some() && l.parent == group || group.is_none() && members.contains(&l.id) {
                l.parent = None;
                if clipped {
                    l.clipping = false;
                }
            }
        }
        let mut budget = 0;
        let mut output_tiles: BTreeMap<Channel, BTreeSet<TileCoord>> = BTreeMap::new();
        for c in self.channels() {
            let kind = self.channel_kind(c)?;
            if Stack::new(&isolated, c, kind, None).plan().is_empty() {
                continue;
            }
            let (eval, evaluated_tiles) = self.merge_eval(&isolated, &doc_index, c, kind)?;
            let mut coords: BTreeSet<_> = isolated
                .iter()
                .flat_map(|l| self.content_coords(l, c))
                .collect();
            coords.extend(evaluated_tiles.iter().copied());
            output_tiles.entry(c).or_default().extend(evaluated_tiles);
            let stack = Stack::new(&isolated, c, kind, Some(&eval));
            let surface = self.merge_surface_of_plan(&coords, &mut budget, &stack)?;
            if surface.tile_count() > 0 {
                result.put_surface(c, Some(surface));
                result.set_enabled(c, true);
            }
        }
        let mut notes = self.disabled_notes(&removed) | effect_bits;
        if self
            .layers
            .iter()
            .any(|l| removed.contains(&l.id) && !members.contains(&l.id) && !l.visible)
        {
            notes |= 4;
        }
        let topid = top.id;
        let mut copy = self.edit_copy()?;
        copy.renew_kept_mask_filter_ids(&mut result);
        let insert = copy
            .layers
            .iter()
            .take_while(|l| l.id != topid)
            .filter(|l| !removed.contains(&l.id))
            .count();
        copy.layers.retain(|l| !removed.contains(&l.id));
        copy.layers.insert(insert, result);
        self.finish_merge(
            copy,
            &removed.into_iter().collect::<Vec<_>>(),
            if group.is_some() {
                MergeMethod::Group
            } else {
                MergeMethod::Layers
            },
            notes,
            tolerance,
            &output_tiles,
        )
    }
    fn finish_merge(
        &mut self,
        copy: Document,
        removed: &[LayerId],
        method: MergeMethod,
        notes: u8,
        tolerance: u8,
        output_tiles: &BTreeMap<Channel, BTreeSet<TileCoord>>,
    ) -> Result<LayerMergeReport, CoreError> {
        // 表示に寄与する層の結合だけは、結合前の合成でなく結果の層の画素と比べる（結合した層が全部の見た目を持つ）
        let visible = method == MergeMethod::Visible;
        let result = copy
            .layers
            .iter()
            .find(|l| self.layer(l.id).is_none())
            .expect("結合結果");
        self.ensure_source_growth(
            copy.allocated_bytes()
                .saturating_sub(self.allocated_bytes()),
        )?;
        let mut report = LayerMergeReport {
            result_id: result.id,
            method,
            notes,
            compared_pixels: 0,
            changed_pixels: 0,
            max_difference: 0,
            max_visible_difference: 0,
            changed_by_channel: BTreeMap::new(),
        };
        let parent = result.parent;
        for c in self.channels() {
            let mut coords: BTreeSet<_> = self
                .layers
                .iter()
                .filter(|l| removed.contains(&l.id))
                .flat_map(|l| self.content_coords(l, c))
                .collect();
            coords.extend(self.content_coords(result, c));
            // 結合した層の出力は、元の画素の無いタイルへも届く（Generator など）。比べる所に足す
            if let Some(tiles) = output_tiles.get(&c) {
                coords.extend(tiles.iter().copied());
            }
            if visible || method == MergeMethod::Layers {
                for (i, l) in self.layers.iter().enumerate() {
                    if visible
                        || parent.is_none()
                        || l.parent == parent
                        || parent.is_some_and(|p| self.is_descendant(i, p))
                    {
                        coords.extend(self.content_coords(l, c));
                    }
                }
            }
            for coord in coords {
                let rect = self.tile_rect(coord).expect("タイル");
                let before = if visible {
                    let mut v = Vec::new();
                    for y in rect.y..rect.y + rect.height {
                        for x in rect.x..rect.x + rect.width {
                            v.extend_from_slice(&result.pixel_or_transparent(c, x, y).to_array());
                        }
                    }
                    v
                } else {
                    self.composite_channel(c, rect)?
                };
                let after = copy.composite_channel(c, rect)?;
                for (a, b) in after.chunks_exact(4).zip(before.chunks_exact(4)) {
                    report.compared_pixels += 1;
                    if a[3] == 0 && b[3] == 0 {
                        continue;
                    }
                    let delta = (0..4).map(|q| a[q].abs_diff(b[q])).max().expect("RGBA");
                    if delta == 0 {
                        continue;
                    }
                    report.changed_pixels += 1;
                    *report.changed_by_channel.entry(c).or_default() += 1;
                    report.max_difference = report.max_difference.max(delta);
                    let mut v = a[3].abs_diff(b[3]);
                    for q in 0..3 {
                        let d = ((a[q] as i32 * a[3] as i32 - b[q] as i32 * b[3] as i32).abs()
                            + 127)
                            / 255;
                        v = v.max(d as u8);
                    }
                    report.max_visible_difference = report.max_visible_difference.max(v);
                }
            }
        }
        if report.max_visible_difference > tolerance {
            return Err(CoreError::MergeAppearance(Box::new(report)));
        }
        let cost = 128
            + result.allocated_bytes()
            + self
                .layers
                .iter()
                .filter(|l| removed.contains(&l.id))
                .map(Layer::allocated_bytes)
                .sum::<u64>();
        // 変わり得るのは外した層と結果の層（結果の見た目の差は許容差の内で、外から見える所は変えない）
        let mut dirty = removed.to_vec();
        dirty.push(report.result_id);
        self.commit_copy(copy, cost, Dirty::Layers(dirty))?;
        Ok(report)
    }
}
