//! クローンの合成の参照元の組み立て（C# の BrushStroke.UseCompositeCloneSource の、文書を読む部分）。

use std::collections::{BTreeSet, HashMap};

use super::*;
use crate::brush::{CloneSource, CompositeTile};

impl Document {
    pub(super) fn use_composite_clone_source(&mut self, id: u64) -> Result<(), CoreError> {
        if !matches!(&self.active, Some(a) if a.id == id) {
            return Err(CoreError::NoActiveStroke);
        }
        match self.build_clone_sources() {
            Ok(sources) => {
                let mut sources = sources.into_iter();
                self.active
                    .as_mut()
                    .expect("確かめた")
                    .set_source(sources.next().expect("先頭のチャンネル"));
                for (state, source) in self.material.extra.iter_mut().zip(sources) {
                    state.set_source(source);
                }
                Ok(())
            }
            Err(e) => {
                self.cancel_active_stroke();
                Err(e)
            }
        }
    }

    /// 凍結した合成の参照元のバイトの合計（全チャンネル。ストロークの予算に数える分）。
    pub fn clone_source_bytes(&self) -> u64 {
        self.active
            .iter()
            .chain(self.material.extra.iter())
            .map(|s| s.clone_source_bytes())
            .sum()
    }

    /// ストロークの各チャンネルの参照元（先頭が active、続きが material.extra の並び）。文書は変えない。
    fn build_clone_sources(&self) -> Result<Vec<CloneSource>, CoreError> {
        if self.active_target == Target::Mask {
            return Err(CoreError::Unsupported(
                "マスクへのストロークはチャンネルの合成を読めない",
            ));
        }
        let states: Vec<&StrokeState> = self
            .active
            .iter()
            .chain(self.material.extra.iter())
            .collect();
        if states.iter().any(|s| !s.can_take_source()) {
            return Err(CoreError::Unsupported(
                "合成の参照元はクローンの最初のダブの前にだけ決められる",
            ));
        }
        let ts = self.tile_size;
        let mut total = 0u64; // C# の cloneSourceBytes（これまでのチャンネルの分）
        let mut sources = Vec::with_capacity(states.len());
        for state in states {
            let channel = state.channel;
            let kind = self.channel_kind(channel)?;
            // 層が画素を持ち得るタイル（見えていて、そのチャンネルが有効な層。塗りつぶし・調整は画布全体）
            let mut coords: BTreeSet<TileCoord> = BTreeSet::new();
            for layer in &self.layers {
                if !layer.visible || !layer.is_channel_enabled(channel) {
                    continue;
                }
                let found: Vec<TileCoord> = match layer.kind {
                    LayerKind::Raster => layer
                        .surface(channel)
                        .map_or_else(Vec::new, |s| s.tile_coords()),
                    LayerKind::Group => Vec::new(),
                    _ => {
                        let applies = layer
                            .adjustment
                            .as_ref()
                            .is_some_and(|a| a.applies_to(kind));
                        if layer.has_content(channel, applies) {
                            self.canvas_tiles().collect()
                        } else {
                            Vec::new()
                        }
                    }
                };
                for coord in found {
                    if coords.insert(coord) {
                        self.ensure_clone_source_budget(total + coords.len() as u64 * 64)?;
                    }
                }
            }
            let rects: Vec<(TileCoord, Rect)> = coords
                .into_iter()
                .map(|c| {
                    let (x, y) = (c.x * ts, c.y * ts);
                    (
                        c,
                        Rect::new(x, y, ts.min(self.width - x), ts.min(self.height - y)),
                    )
                })
                .collect();
            let bytes: u64 = rects
                .iter()
                .map(|(_, r)| 64 + r.width as u64 * r.height as u64 * 4)
                .sum();
            self.ensure_clone_source_budget(total + bytes)?;
            total += bytes;
            let mut tiles = HashMap::with_capacity(rects.len());
            for (coord, rect) in rects {
                let mut pixels = vec![0u8; rect.width as usize * rect.height as usize * 4];
                self.composite_into(channel, rect, &mut pixels, RowOrder::BottomUp)?;
                tiles.insert(
                    coord,
                    CompositeTile {
                        x: rect.x as i64,
                        width: rect.width as i64,
                        pixels,
                    },
                );
            }
            sources.push(CloneSource::new(ts, tiles, bytes));
        }
        Ok(sources)
    }

    fn ensure_clone_source_budget(&self, bytes: u64) -> Result<(), CoreError> {
        if bytes > self.stroke_budget {
            Err(CoreError::StrokeBudgetExceeded)
        } else {
            Ok(())
        }
    }
}
