use super::material::MaterialCommand;
use super::selection::{fill_pixel, mask_fill_pixel};
use super::*;
use crate::material::ChannelPaint;
use crate::material_triangles::{add_samples, coverage, PixelTriangle, Samples};
use crate::SelectionMask;
use rayon::prelude::*;
use std::collections::BTreeMap;
/// 増えていく三角形の和集合を塗る札。確定と取消は文書を通して行う。
pub struct TriangleFill {
    id: u64,
}
impl TriangleFill {
    pub fn add(
        &mut self,
        doc: &mut Document,
        triangles: &[PixelTriangle],
    ) -> Result<bool, CoreError> {
        doc.add_triangle_fill(self.id, triangles)
    }
    pub fn triangles_added(&self, doc: &Document) -> Option<u64> {
        doc.triangle_fill
            .as_ref()
            .filter(|s| s.id == self.id)
            .map(|s| s.count)
    }
    pub fn covered_tile_count(&self, doc: &Document) -> Option<usize> {
        doc.triangle_fill
            .as_ref()
            .filter(|s| s.id == self.id)
            .map(|s| s.samples.len())
    }
    pub fn commit(self, doc: &mut Document) -> Result<StrokeResult, CoreError> {
        doc.end_stroke(Stroke { id: self.id })
    }
    pub fn cancel(self, doc: &mut Document) {
        doc.cancel_stroke(Stroke { id: self.id });
    }
}
pub(super) struct TriangleState {
    id: u64,
    layer: LayerId,
    samples: Samples,
    count: u64,
    pub(super) rollback: u64,
    selection: Option<SelectionMask>,
    opacity: f64,
    erase: bool,
    /// 透明部分のロックでアルファを守る塗り（書き込みの関門が決めた値。マスクは常に false）。
    keep_alpha: bool,
    targets: Vec<(Target, Rgba8, BTreeMap<TileCoord, Option<Tile>>)>,
}
impl TriangleState {
    pub(super) fn tile_count(&self) -> usize {
        self.targets.iter().map(|(_, _, tiles)| tiles.len()).sum()
    }
}
impl Document {
    pub fn begin_material_triangle_fill(
        &mut self,
        layer: LayerId,
        channels: &[ChannelPaint],
        opacity: f64,
        erase: bool,
    ) -> Result<TriangleFill, CoreError> {
        // ストロークの札は借りるだけ（画素は和集合から自分の式で塗る）。ロックの検査はこの入口の中で済み、返る keep_alpha を式へ渡す。
        let (stroke, keep_alpha) = self.begin_guarded_material_stroke(
            layer,
            channels,
            &Brush::from(BrushSettings {
                opacity,
                erase,
                ..Default::default()
            }),
        )?;
        self.start_triangles(
            stroke,
            layer,
            channels
                .iter()
                .map(|m| (Target::Channel(m.channel), m.value, BTreeMap::new()))
                .collect(),
            opacity,
            erase,
            keep_alpha,
        )
    }
    pub fn begin_triangle_fill(
        &mut self,
        layer: LayerId,
        channel: Channel,
        color: Rgba8,
        opacity: f64,
        erase: bool,
    ) -> Result<TriangleFill, CoreError> {
        // まとめの中の断りは、チャンネルやレイヤーの確かめより先（C# の RefuseInBatch が ValidateChannel より先なのと同じ。
        // まとめの中で無効なチャンネルを渡しても、断る理由は BatchActive）
        self.ensure_loadable()?;
        self.require_channel(channel)?;
        if !self
            .layer(layer)
            .ok_or(CoreError::LayerNotFound)?
            .is_channel_enabled(channel)
        {
            return Err(CoreError::Unsupported("無効のチャンネルには塗れない"));
        }
        self.begin_material_triangle_fill(
            layer,
            &[ChannelPaint::new(channel, color)],
            opacity,
            erase,
        )
    }
    pub fn begin_mask_triangle_fill(
        &mut self,
        layer: LayerId,
        amount: f64,
        reveal: bool,
    ) -> Result<TriangleFill, CoreError> {
        // すべてのロックだけで断る（begin_brush_mask_stroke の中）。マスクは透明部分のロックの対象外
        let stroke = self.begin_mask_stroke(
            layer,
            &BrushSettings {
                opacity: amount,
                erase: reveal,
                ..Default::default()
            },
        )?;
        self.start_triangles(
            stroke,
            layer,
            vec![(Target::Mask, Rgba8::new(0, 0, 0, 255), BTreeMap::new())],
            amount,
            reveal,
            false,
        )
    }
    fn start_triangles(
        &mut self,
        stroke: Stroke,
        layer: LayerId,
        targets: Vec<(Target, Rgba8, BTreeMap<TileCoord, Option<Tile>>)>,
        opacity: f64,
        erase: bool,
        keep_alpha: bool,
    ) -> Result<TriangleFill, CoreError> {
        let id = stroke.id;
        self.triangle_fill = Some(TriangleState {
            id,
            layer,
            targets,
            opacity,
            erase,
            keep_alpha,
            selection: self.selection.clone(),
            samples: Samples::new(),
            count: 0,
            rollback: 0,
        });
        Ok(TriangleFill { id })
    }
    fn add_triangle_fill(
        &mut self,
        id: u64,
        triangles: &[PixelTriangle],
    ) -> Result<bool, CoreError> {
        if !matches!(&self.triangle_fill,Some(s) if s.id==id) {
            return Err(CoreError::NoActiveStroke);
        }
        let mut state = self.triangle_fill.take().unwrap();
        let result = self.recompute_triangles(&mut state, triangles);
        self.triangle_fill = Some(state);
        if result.is_err() {
            self.cancel_active_stroke();
        }
        result
    }
    /// 和集合が変わったタイルを、描く前の画素から計算し直す。新しい中身はタイルごとに並列に求め（描く前の画素・和集合の被覆率・
    /// 選択範囲だけで決まる）、巻き戻しと画素の予算の確かめ・書き込みはタイルの順にこのスレッドで行う（止まる所も、止まったときに
    /// 戻すものも並列の度合いによらず逐次と同じ）。
    fn recompute_triangles(
        &mut self,
        state: &mut TriangleState,
        triangles: &[PixelTriangle],
    ) -> Result<bool, CoreError> {
        let coords = add_samples(
            &mut state.samples,
            triangles,
            self.width,
            self.height,
            self.tile_size,
            self.stroke_budget,
            &mut state.rollback,
        )?;
        state.count += triangles.len() as u64;
        let index = self.index_of(state.layer)?;
        let (width, height, ts) = (self.width, self.height, self.tile_size);
        let batch = (rayon::current_num_threads() * 4).max(1);
        let mut any = false;
        for (target, color, originals) in &mut state.targets {
            let target = *target;
            // この対象の面だけが変わるので、ほかの面の分は対象ごとに 1 回数える
            let growth = self.growth_for(index, target);
            for chunk in coords.chunks(batch) {
                let computed: Vec<Option<Option<Tile>>> = {
                    let surface = self.target_surface(index, target).unwrap();
                    let (samples, selection, opacity, erase, keep_alpha) = (
                        &state.samples,
                        &state.selection,
                        state.opacity,
                        state.erase,
                        state.keep_alpha,
                    );
                    let originals = &*originals;
                    let color = *color;
                    chunk
                        .par_iter()
                        .map_init(
                            || vec![0u8; (ts * ts * 4) as usize],
                            |bytes, &coord| {
                                // 描く前の画素: まだ写しを取っていないタイルは、面の今の中身がそれ
                                let original = match originals.get(&coord) {
                                    Some(o) => o.as_ref(),
                                    None => surface.tile(coord),
                                };
                                let mut picked = None;
                                if let Some(sel) = selection {
                                    let mut a = vec![0u8; (ts * ts) as usize];
                                    if !sel.copy_tile(coord, &mut a).expect("文書の中のタイル")
                                    {
                                        return Ok(None); // 選ばれていないタイルは変わらない
                                    }
                                    picked = Some(a);
                                }
                                bytes.fill(0);
                                if let Some(t) = original {
                                    t.copy_to(bytes)?;
                                }
                                let bits = samples[&coord].as_deref();
                                let w = (width - coord.x * ts).min(ts);
                                let h = (height - coord.y * ts).min(ts);
                                for y in 0..h {
                                    for x in 0..w {
                                        let i = (y * ts + x) as usize;
                                        let mut amount = bits.map_or(255, |b| coverage(b[i]));
                                        if let Some(a) = &picked {
                                            amount = amount.min(a[i]);
                                        }
                                        if amount == 0 {
                                            continue;
                                        }
                                        let start = Rgba8::from_slice(&bytes[i * 4..i * 4 + 4]);
                                        let next = if target == Target::Mask {
                                            mask_fill_pixel(
                                                start,
                                                opacity,
                                                amount as f64 / 255.0,
                                                erase,
                                            )
                                        } else {
                                            fill_pixel(
                                                start,
                                                color,
                                                opacity,
                                                amount as f64 / 255.0,
                                                erase,
                                                keep_alpha,
                                            )
                                        };
                                        bytes[i * 4..i * 4 + 4].copy_from_slice(&next.to_array());
                                    }
                                }
                                Ok(Some(Tile::from_bytes(bytes)))
                            },
                        )
                        .collect::<Result<_, CoreError>>()?
                };
                for (&coord, after) in chunk.iter().zip(computed) {
                    // 選ばれていないタイルも、C# と同じく写しを取って巻き戻しの予算に数える
                    if let std::collections::btree_map::Entry::Vacant(e) = originals.entry(coord) {
                        let before = self
                            .target_surface(index, target)
                            .unwrap()
                            .tile(coord)
                            .cloned();
                        state.rollback += 64 + before.as_ref().map_or(0, Tile::byte_size);
                        if state.rollback > self.stroke_budget {
                            return Err(CoreError::StrokeBudgetExceeded);
                        }
                        e.insert(before);
                    }
                    let Some(after) = after else {
                        continue;
                    };
                    let surface = self.target_surface(index, target).unwrap();
                    if Tile::same(surface.tile(coord), after.as_ref()) {
                        continue;
                    }
                    growth.ensure(
                        surface.allocated_bytes(),
                        surface.growth_to(coord, after.as_ref()),
                    )?;
                    self.target_surface_mut(index, target)
                        .unwrap()
                        .restore(coord, after.as_ref());
                    self.mark_target_tile(index, target, coord);
                    any = true;
                }
            }
        }
        if any {
            self.revision += 1;
        }
        Ok(any)
    }
    pub(super) fn finish_triangles(&mut self, cancel: bool) -> StrokeResult {
        let state = self.triangle_fill.take().unwrap();
        let index = self.index_of(state.layer).unwrap();
        let mut parts = Vec::new();
        for (target, _, originals) in state.targets {
            let mut changes = Vec::new();
            for (coord, before) in originals {
                let after = self
                    .target_surface(index, target)
                    .unwrap()
                    .tile(coord)
                    .cloned();
                if cancel {
                    self.target_surface_mut(index, target)
                        .unwrap()
                        .restore(coord, before.as_ref());
                    self.mark_target_tile(index, target, coord);
                } else if !Tile::same(before.as_ref(), after.as_ref()) {
                    changes.push(TileChange {
                        coord,
                        before,
                        after,
                    });
                }
            }
            if !changes.is_empty() {
                parts.push((target, changes));
            }
        }
        let enabled = std::mem::take(&mut self.material.enabled);
        self.material = Default::default();
        self.active = None;
        let changed = !parts.is_empty();
        let command = MaterialCommand {
            layer: state.layer,
            parts,
            enabled,
        };
        if changed {
            self.push_material(command);
        } else {
            self.restore_material(&command, true).expect("元の状態");
        }
        if cancel {
            self.revision += 1;
        }
        StrokeResult {
            changed,
            stamps: 0,
            samples: 0,
            copy_note: None,
        }
    }
}
