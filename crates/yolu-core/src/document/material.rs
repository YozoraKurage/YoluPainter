use super::*;
use crate::material::ChannelPaint;

#[derive(Default)]
pub(super) struct MaterialState {
    pub started: bool,
    pub extra: Vec<StrokeState>,
    pub(super) enabled: Vec<(Channel, bool)>,
}
pub(crate) struct MaterialCommand {
    pub(super) layer: LayerId,
    pub(super) enabled: Vec<(Channel, bool)>,
    pub(super) parts: Vec<(Target, Vec<TileChange>)>,
}
impl Document {
    /// 手動の ID 色（塊の番号 → 0xRRGGBB とモデルの結び付け）。画素ではなく文書の状態で、Document 自身は保存しない:
    /// `.ylp` へは呼び手がこの値を書き出し、読み込み直後に [`Document::restore_id_colors`] で戻す。
    pub fn id_colors(&self) -> &crate::mesh_maps::IdColorAssignments {
        &self.id_colors
    }
    /// 手動 ID 色の変更。画素を変えず、一回の Undo にする（`coalesce` なら、間に何も無い手動 ID 色の変更へまとめる。色のウィンドウのドラッグ。
    /// まとめは `end_coalescing` まで。戻すと最初の変更の前へ）。同じ値なら何もしない。
    pub fn set_id_colors(
        &mut self,
        colors: crate::mesh_maps::IdColorAssignments,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        if self.id_colors.binding == colors.binding && self.id_colors.colors == colors.colors {
            return Ok(());
        }
        let cost = 128 + 16 * (self.id_colors.colors.len() + colors.colors.len()) as u64;
        self.record(
            Command::IdColors {
                old: self.id_colors.clone(),
                new: colors,
            },
            cost,
            coalesce.then_some(CoalesceKey::IdColors),
        )
    }
    /// 読み込み直後に手動 ID 色を復元する。履歴・リビジョンを増やさない。
    pub fn restore_id_colors(
        &mut self,
        colors: crate::mesh_maps::IdColorAssignments,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        if !self.undo.is_empty() || !self.redo.is_empty() {
            return Err(CoreError::Unsupported("ID 色の復元は読み込み直後だけ"));
        }
        self.id_colors = colors;
        Ok(())
    }
    /// 重なった UV のテクセルの持ち主の決め方（ベイクの優先。テクスチャセットごと）。画素ではなく文書の状態で、Document 自身は
    /// 保存しない: `.ylp` へは呼び手（yolu-io の正本の版 33）がこの値を書き出し、読み込み直後に [`Document::restore_bake_priority`] で戻す。
    pub fn bake_priority(&self) -> &crate::mesh_maps::MeshOverlapPriority {
        &self.bake_priority
    }
    /// ベイクの優先の変更。画素を変えず、1 回の Undo にする。範囲の外の値は断る。
    pub fn set_bake_priority(
        &mut self,
        priority: crate::mesh_maps::MeshOverlapPriority,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        priority
            .validate()
            .map_err(|_| CoreError::InvalidArgument("ベイクの優先の値が範囲外です"))?;
        if self.bake_priority == priority {
            return Ok(());
        }
        let cost = 192
            + 16 * (self.bake_priority.skipped().len()
                + self.bake_priority.preferred().len()
                + priority.skipped().len()
                + priority.preferred().len()) as u64;
        self.execute(
            Command::BakePriority {
                old: self.bake_priority.clone(),
                new: priority,
            },
            cost,
        )
    }
    /// 読み込み直後にベイクの優先を戻す。履歴・リビジョンを増やさない。
    pub fn restore_bake_priority(
        &mut self,
        priority: crate::mesh_maps::MeshOverlapPriority,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        if !self.undo.is_empty() || !self.redo.is_empty() {
            return Err(CoreError::Unsupported(
                "ベイクの優先の復元は読み込み直後だけ",
            ));
        }
        priority
            .validate()
            .map_err(|_| CoreError::InvalidArgument("ベイクの優先の値が範囲外です"))?;
        self.bake_priority = priority;
        Ok(())
    }
    pub fn begin_material_stroke(
        &mut self,
        layer: LayerId,
        channels: &[ChannelPaint],
        brush: &BrushSettings,
    ) -> Result<Stroke, CoreError> {
        self.begin_material_brush_stroke(layer, channels, &Brush::from(*brush))
    }
    /// 同じ入力を全チャンネルへ与える。覆い（画素ごとの被覆率）はチャンネルごとに持ち、巻き戻しの確保量は全チャンネルの実際の
    /// 合計を予算に数える。C# は覆いを全チャンネルで 1 枚共有するので、同じ入力でも確保量が多く、予算で断る側にずれる: 元が空の
    /// タイルを 6 チャンネルで塗ると 6 倍（既定の予算 64 MiB・タイル 128² で 1 回のストロークが通るタイルは 170 枚、C# は 1023 枚）、
    /// 全チャンネルに画素のあるタイルなら約 1.7 倍（85 枚と 146 枚）。1 チャンネルは C# と同じ。数は tests/reference/material_golden.rs が
    /// C# の測った値と照らして固定している。
    ///
    /// 画像・すべてのロックと、消すときの透明部分のロックで断る。何も変えない（無効のチャンネルを有効にするのも、断るより後）。
    /// 透明部分のロックでは、全チャンネルがアルファと透明画素の RGB を守る。
    pub fn begin_material_brush_stroke(
        &mut self,
        layer: LayerId,
        channels: &[ChannelPaint],
        brush: &Brush,
    ) -> Result<Stroke, CoreError> {
        self.begin_guarded_material_stroke(layer, channels, brush)
            .map(|(stroke, _)| stroke)
    }
    /// [`Document::begin_material_brush_stroke`] と、書き込みの関門が決めた透明部分のロック（`keep_alpha`）。三角形の塗りは
    /// ストロークの札を借りるだけで画素は自分の式で塗るので、その式へこの値を渡す。
    pub(super) fn begin_guarded_material_stroke(
        &mut self,
        layer: LayerId,
        channels: &[ChannelPaint],
        brush: &Brush,
    ) -> Result<(Stroke, bool), CoreError> {
        self.ensure_loadable()?;
        brush.validate()?;
        self.validate_material(channels)?;
        let index = self.index_of(layer)?;
        self.ensure_raster(index)?;
        // 無効のチャンネルを有効にする前に断る（断ったストロークが何も残さない）
        let keep_alpha = self.pixel_write_guard(layer, brush.base.erase)?;
        self.refuse_path_layer(index)?;
        let mut enabled = Vec::new();
        let id = self.next_stroke;
        self.next_stroke += 1;
        let mut states = Vec::new();
        for c in channels {
            if !self.layers[index].is_channel_enabled(c.channel) {
                let had = self.layers[index].surface(c.channel).is_some();
                self.switch_channel_enabled(layer, c.channel, true, had, false)?;
                enabled.push((c.channel, had));
            }
            self.ensure_surface(index, c.channel);
            let kind = self.channel_kind(c.channel)?;
            let mut brush = if crate::brush::carries_color(kind) {
                brush.clone()
            } else {
                brush.without_color_dynamics()
            };
            brush.base.color = c.value;
            states.push(
                StrokeState::new(
                    id,
                    layer,
                    index,
                    c.channel,
                    kind,
                    brush,
                    Budgets {
                        growth: self.growth_for(index, Target::Channel(c.channel)),
                        stroke: self.stroke_budget,
                    },
                    (self.width, self.height, self.tile_size),
                    keep_alpha,
                )
                .with_selection(self.selection.clone()),
            );
        }
        self.active = Some(states.remove(0));
        self.active_target = Target::Channel(channels[0].channel);
        self.material = MaterialState {
            started: true,
            extra: states,
            enabled,
        };
        Ok((Stroke { id }, keep_alpha))
    }
    pub(super) fn validate_material(&self, channels: &[ChannelPaint]) -> Result<(), CoreError> {
        if channels.is_empty() {
            return Err(CoreError::InvalidArgument("空のマテリアル"));
        }
        let mut seen = std::collections::HashSet::new();
        for c in channels {
            self.require_channel(c.channel)?;
            if !seen.insert(c.channel) {
                return Err(CoreError::InvalidArgument("重複したチャンネル"));
            }
        }
        Ok(())
    }
    pub(super) fn with_material_stroke<F>(&mut self, id: u64, mut f: F) -> Result<bool, CoreError>
    where
        F: FnMut(&mut StrokeState, &mut Surface, &mut Vec<TileCoord>) -> Result<bool, CoreError>,
    {
        if !matches!(&self.active, Some(s) if s.id == id) {
            return Err(CoreError::NoActiveStroke);
        }
        let mut states = std::mem::take(&mut self.material.extra);
        states.insert(0, self.active.take().unwrap());
        let mut result = Ok(false);
        for i in 0..states.len() {
            let others: u64 = states
                .iter()
                .enumerate()
                .filter(|(k, _)| *k != i)
                .map(|(_, s)| s.rollback_total())
                .sum();
            let state = &mut states[i];
            let index = state.layer_index;
            let target = Target::Channel(state.channel);
            state.set_budgets(Budgets {
                growth: self.growth_for(index, target),
                stroke: self.stroke_budget.saturating_sub(others),
            });
            let mut changed = Vec::new();
            let r = f(
                state,
                self.target_surface_mut(index, target).unwrap(),
                &mut changed,
            );
            self.journal.own = true; // ストロークが自分の面へ書いた変化（下の覚えは、これでは捨てない）
            for coord in changed {
                self.mark_target_tile(index, target, coord);
            }
            self.journal.own = false;
            match r {
                Ok(any) => {
                    if any {
                        self.revision += 1;
                        result = Ok(true);
                    }
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        self.active = Some(states.remove(0));
        self.material.extra = states;
        if result.is_err() {
            self.cancel_material();
        } else {
            self.fit_memo_to_budget();
        }
        result
    }
    pub(super) fn finish_material(&mut self) -> Result<StrokeResult, CoreError> {
        let first = self.active.take().unwrap();
        self.journal.memo.clear(); // 描き終えた（下の覚えを手放す）
        let result = StrokeResult {
            changed: false,
            stamps: first.stamp_count,
            samples: first.sample_count,
            copy_note: first.copy_note(),
        };
        let layer = first.layer;
        let index = first.layer_index;
        let mut states = std::mem::take(&mut self.material.extra);
        states.insert(0, first);
        let mut parts = Vec::new();
        for state in states {
            let target = Target::Channel(state.channel);
            let surface = self.target_surface_mut(index, target).unwrap();
            let mut coords: Vec<_> = state.tiles.keys().copied().collect();
            coords.sort();
            let mut changes = Vec::new();
            for coord in coords {
                surface.compact(coord);
                let before = state.tiles[&coord].before.clone();
                let after = surface.tile(coord).cloned();
                if !Tile::same(before.as_ref(), after.as_ref()) {
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
        self.material.started = false;
        if parts.is_empty() {
            for &(c, had) in enabled.iter().rev() {
                self.switch_channel_enabled(layer, c, true, had, true)?;
            }
            return Ok(result);
        }
        self.push_material(MaterialCommand {
            layer,
            enabled,
            parts,
        });
        Ok(StrokeResult {
            changed: true,
            ..result
        })
    }
    pub(super) fn push_material(&mut self, m: MaterialCommand) {
        let cost = m.enabled.len() as u64 * 64
            + m.parts
                .iter()
                .map(|(_, p)| {
                    64 + p
                        .iter()
                        .map(|c| {
                            16 + c.before.as_ref().map_or(0, Tile::byte_size)
                                + c.after.as_ref().map_or(0, Tile::byte_size)
                        })
                        .sum::<u64>()
                })
                .sum::<u64>();
        self.push(Entry {
            kind: super::HistoryKind::Brush,
            command: Command::Material(m),
            cost,
        });
    }
    pub(super) fn cancel_material(&mut self) -> bool {
        let Some(first) = self.active.take() else {
            return false;
        };
        self.journal.memo.clear(); // 取り消した（下の覚えを手放す）
        let layer = first.layer;
        let index = first.layer_index;
        let mut states = std::mem::take(&mut self.material.extra);
        states.insert(0, first);
        for state in states {
            let target = Target::Channel(state.channel);
            for (coord, tile) in state.tiles {
                self.target_surface_mut(index, target)
                    .unwrap()
                    .restore(coord, tile.before.as_ref());
                self.mark_target_tile(index, target, coord);
            }
        }
        for (c, had) in std::mem::take(&mut self.material.enabled).into_iter().rev() {
            self.switch_channel_enabled(layer, c, true, had, true)
                .expect("進行中のレイヤー");
        }
        self.material.started = false;
        self.revision += 1;
        true
    }
    pub(super) fn restore_material(
        &mut self,
        m: &MaterialCommand,
        backwards: bool,
    ) -> Result<(), CoreError> {
        let index = self.index_of(m.layer)?;
        let delta: i64 = m
            .parts
            .iter()
            .map(|(target, changes)| {
                changes
                    .iter()
                    .map(|c| {
                        let next = if backwards { &c.before } else { &c.after };
                        next.as_ref().map_or(0, Tile::byte_size) as i64
                            - self
                                .target_surface(index, *target)
                                .and_then(|s| s.tile(c.coord))
                                .map_or(0, Tile::byte_size) as i64
                    })
                    .sum::<i64>()
            })
            .sum();
        if delta > 0 {
            self.ensure_source_growth(delta as u64)?;
        }
        if !backwards {
            for &(c, had) in &m.enabled {
                self.switch_channel_enabled(m.layer, c, true, had, false)?;
            }
        }
        // 全体の増分を先に検証済み。個別の増分で再拒否しない。
        for (target, changes) in &m.parts {
            for c in changes {
                self.target_surface_mut(index, *target)
                    .expect("履歴の面")
                    .restore(
                        c.coord,
                        if backwards {
                            c.before.as_ref()
                        } else {
                            c.after.as_ref()
                        },
                    );
                self.mark_target_tile(index, *target, c.coord);
            }
        }
        if backwards {
            for &(c, had) in m.enabled.iter().rev() {
                self.switch_channel_enabled(m.layer, c, true, had, true)?;
            }
        }
        Ok(())
    }
}
