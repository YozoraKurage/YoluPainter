//! 複数操作を準備用の文書で実行し、成功した状態だけを 1 段で交換する。
use super::{Command, Document};
use crate::surface::{Surface, Tile};
use crate::{CoreError, Layer, LayerId, NormalSettings, TileCoord};

/// 交換が合成を変え得る範囲。変化の記録（`changed_tiles`）の印を、交換の前後の両方で付ける範囲を決める。
#[derive(Clone, Debug)]
pub(crate) enum Dirty {
    /// 全層（寸法が変わるなど、どの層も変わり得る）。
    All,
    /// この層（グループなら中身ごと）。空なら合成は変わらない（ロックだけの変更）。層の出入りや並びで下地が変わるクリッピングの
    /// 組は、層が 1 つでもあれば交換のあとで印を付ける（単独の構造編集と同じ）。
    Layers(Vec<LayerId>),
}
impl Dirty {
    /// 合成が変わらない交換。
    pub(crate) const NOTHING: Dirty = Dirty::Layers(Vec::new());
}

/// 準備用の文書から本物の文書へ交換する状態。交換するのはここに書いた分だけで、どれを交換しないかは
/// [`Document::edit_copy`] と [`State::take`] の網羅した分解に理由がある（`Document` に項目が増えるとコンパイルが落ちる）。
pub(crate) struct State {
    layers: Vec<Layer>,
    width: u32,
    height: u32,
    normal: NormalSettings,
    selection: Option<crate::SelectionMask>,
    /// 名前を付けて残した選択範囲（画布の大きさを変える操作が作り直す。ほかの操作は文書のものをそのまま写している）。
    saved_selections: super::saved_selections::SavedList,
    dirty: Dirty,
}
impl State {
    fn take(doc: &mut Document, dirty: Dirty) -> Self {
        // 項目を全部挙げる（`..` を使わない）: 新しい項目は、交換するか、しない理由を書くかをここで決める。
        let Document {
            layers,
            width,
            height,
            normal_settings,
            selection,
            saved_selections,
            // 交換しない: 同じ文書の同じ値（準備用の文書は書き換えない）
            id: _,
            tile_size: _,
            id_counter: _,
            // 交換しない: 準備の中では操作しない（チャンネルの追加・削除・設定は専用の段で行う）
            channels: _,
            // 交換しない: 進行中のマテリアルのストロークと三角形の塗りは本物の文書のもの（一括操作は進行中のストロークがあると
            // 始まらず、準備用の文書では始めない）
            material: _,
            triangle_fill: _,
            // 交換しない: 手動の ID 色（塊の番号と色の結び付け）は画素でも層でもなく、層の操作・変形・サイズ変更では変わらない
            // （C# の Resampled は別の文書を返すので持ち越さないが、Rust は同じ文書の中で交換するので値がそのまま残る）
            id_colors: _,
            // 交換しない: 見た目の設定は画素でも層でもなく、層の操作・変形・サイズ変更では変わらない（専用の段で変える）。受けた見た目と
            // 描く見た目も同じ（受け取りは段の外）
            look: _,
            received_look: _,
            drawn_look: _,
            look_serial: _,
            // 交換しない: 効果の入力・予算・評価のキャッシュ・元画素の時計は本物の文書のもの。交換で入る層の出力は、`swap_state` が前後の
            // 両方で層に印を付けて作り直させる（準備用の文書の時計は本物と別の数え方なので、持ち込むと別の内容に同じ鍵が付き得る）
            effects: _,
            // 交換しない: 準備用の文書の履歴・進行中のストローク・変化の記録・予算は本物の文書のもの
            undo: _,
            redo: _,
            history_bytes: _,
            undo_budget: _,
            minimum_undo_steps: _,
            source_budget: _,
            stroke_budget: _,
            active: _,
            active_target: _,
            next_stroke: _,
            revision: _,
            journal: _,
            coalesce: _,
            trim_count: _,
            trimmed_bytes: _,
            // 交換しない: `batch` の編集の中かの印は本物の文書のもの（準備用の文書は自分の履歴を作るだけ）
            batching: _,
            // 交換しない: 継ぎ目の設定は専用の段で変える（準備の操作は変えない）
            filter_seams: _,
        } = doc;
        Self {
            layers: std::mem::take(layers),
            width: *width,
            height: *height,
            normal: *normal_settings,
            selection: selection.take(),
            saved_selections: std::mem::take(saved_selections),
            dirty,
        }
    }
}
impl Document {
    /// 層・チャンネル・選択範囲などを写した準備用の文書。タイルは共有し、履歴の予算は無制限（本物へ交換するときに確かめる）。
    pub(super) fn edit_copy(&self) -> Result<Document, CoreError> {
        // 項目を全部挙げる（`..` を使わない）: `Document` に項目が増えたら、準備用へ写すか、写さない理由を書くかをここで決める。
        // 準備で変わる項目は `State::take` でも交換するか決める。
        let Document {
            id,
            width,
            height,
            tile_size,
            layers,
            channels,
            normal_settings,
            selection,
            saved_selections,
            source_budget,
            stroke_budget,
            id_counter,
            // 写す: 準備の中で合成して前後を比べるので、本物と同じく継ぎ目をまたいで評価する
            filter_seams,
            // 効果は入力と予算・ブロックの大きさだけ写す（下で）。写さない: 評価のキャッシュ・元画素の時計・Anchor の解決の署名。準備用の
            // 文書は使い捨てで、時計は 0 から数え直す。本物のキャッシュや時計を共有すると、準備の中で進んだ時計の値が本物の別の編集と
            // 重なったとき、別の内容に同じ鍵が付いて古い出力を新しいものと取り違える
            effects,
            // 写さない: 進行中のマテリアルのストロークと三角形の塗り（準備の中では始めない）、手動の ID 色（準備の操作は読まず、
            // 交換もしない）
            material: _,
            triangle_fill: _,
            id_colors: _,
            // 写さない: 見た目の設定と受けた見た目（準備の操作は読まず、交換もしない）
            look: _,
            received_look: _,
            drawn_look: _,
            look_serial: _,
            // 写さない: 履歴・進行中のストローク・変化の記録は本物の文書のもの（準備の中の操作は自分の履歴を作るだけで、
            // 成功したら 1 段にまとめて本物へ交換する）
            undo: _,
            redo: _,
            history_bytes: _,
            undo_budget: _,
            minimum_undo_steps: _,
            active: _,
            active_target: _,
            next_stroke: _,
            revision: _,
            journal: _,
            coalesce: _,
            trim_count: _,
            trimmed_bytes: _,
            // 写さない: 準備用の文書は `batch` の外（本物の文書が `batch` の中でも、準備用の文書は自分の履歴を作ってよい。
            // 本物へは 1 段で交換し、それがまとめの中の 1 段になる）
            batching: _,
        } = self;
        let mut d = Document::with_tile_size(*width, *height, *tile_size)?;
        d.id = *id;
        d.layers = layers.clone();
        d.channels = channels.clone();
        d.normal_settings = *normal_settings;
        d.selection = selection.clone();
        d.saved_selections = saved_selections.clone();
        d.source_budget = *source_budget;
        d.stroke_budget = *stroke_budget;
        d.undo_budget = u64::MAX;
        d.id_counter = *id_counter;
        d.filter_seams = *filter_seams;
        // 準備の中で段を足す・合成して前後を比べる（結合・変形・大きさの変更）ので、画像・メッシュマップ・モデルのルートが無いと、
        // 本物では効く Generator や画像が入力のまま通り、本物と違う見た目で比べてしまう。予算は、準備の中の段の検査（到達半径・
        // 作業メモリ）を本物と同じ決まりにする
        d.effects.inputs = effects.inputs.clone();
        d.effects.inputs_revision = effects.inputs_revision;
        d.effects.topology_revision = effects.topology_revision;
        d.effects.working_budget = effects.working_budget;
        d.effects.cache_budget = effects.cache_budget;
        d.effects.image_cache_budget = effects.image_cache_budget;
        d.effects.seam_budget = effects.seam_budget;
        d.effects.block_pixels = effects.block_pixels;
        Ok(d)
    }
    pub(super) fn commit_copy(
        &mut self,
        mut copy: Document,
        cost: u64,
        dirty: Dirty,
    ) -> Result<(), CoreError> {
        self.execute(Command::Swap(Box::new(State::take(&mut copy, dirty))), cost)
    }
    /// [`Document::commit_copy`] と同じで、この段は履歴の予算を超えても残す（古い段から落とすが、この段は落とさない）。
    /// 前後の格納量が文書の 2 倍以上になるサイズ変更で、予算が小さいときに黙って Undo を失わないための入口。残るのはこの 1 段が
    /// 最新の間だけで、次の編集の整理では普通の段として落とし得る。
    pub(super) fn commit_copy_kept(
        &mut self,
        copy: Document,
        cost: u64,
        dirty: Dirty,
    ) -> Result<(), CoreError> {
        let saved = self.minimum_undo_steps;
        self.minimum_undo_steps = saved.max(1);
        let result = self.commit_copy(copy, cost, dirty);
        self.minimum_undo_steps = saved;
        result
    }
    pub(super) fn swap_state(&mut self, state: &mut State) -> Result<(), CoreError> {
        let incoming: u64 = state.layers.iter().map(Layer::allocated_bytes).sum();
        let current = self.allocated_bytes();
        self.ensure_source_growth(incoming.saturating_sub(current))?;
        // 交換の前は文書にある側と段が持つ側（これから文書へ入る層）、後は入れ替わった側に印を付ける。どちらかの時点で
        // 文書の中にある層は、グループの中身も含めて印が付く。
        self.mark_dirty(&state.dirty, &state.layers);
        let resized = self.width != state.width || self.height != state.height;
        std::mem::swap(&mut self.layers, &mut state.layers);
        std::mem::swap(&mut self.width, &mut state.width);
        std::mem::swap(&mut self.height, &mut state.height);
        if resized {
            // 画布の大きさが変わると、元の画素の無い層（Generator・塗りつぶし）の出力の鍵（時計）は変わらないまま内容が変わる:
            // 評価済みのものを全部捨てる
            self.effects.generation += 1;
            self.release_effect_cache();
        }
        std::mem::swap(&mut self.normal_settings, &mut state.normal);
        std::mem::swap(&mut self.selection, &mut state.selection);
        std::mem::swap(&mut self.saved_selections, &mut state.saved_selections);
        self.mark_dirty(&state.dirty, &state.layers);
        self.note_swapped_sources(&state.dirty, &state.layers);
        if matches!(&state.dirty, Dirty::Layers(ids) if !ids.is_empty()) {
            self.mark_clipped_layers();
        }
        Ok(())
    }
    /// 交換で画素が変わった層の、元画素の変化を覚える（評価のキャッシュの鍵）。`mark_dirty` の印は合成が変わり得るタイルの記録
    /// （`changed_tiles`）で、キャッシュの鍵（元画素のタイルごとの時計）は画素の変化の道（`mark_target_tile`）でしか進まない。変形や
    /// 結合のように、同じ ID の層の画素が交換で入れ替わると、時計が進まず古い評価の出力を新しい画素のものとして返してしまう。
    /// 交換の前後（`spare` は交換で外へ出た側の層）で同じ ID の層のタイルを比べ、**中身が違うタイルだけ**進める。複数選択の表示の
    /// 切り替え・並べ替え・複製・削除のように画素を変えない交換では進まず、評価したぼかしなどのキャッシュが生き残る（共有している
    /// タイルは `Tile::same` が安く同じと答える）。文書から無くなった層には何も足さない（戻すときは、前に層が無い側として全部進む）。
    fn note_swapped_sources(&mut self, dirty: &Dirty, spare: &[Layer]) {
        let parents: Vec<(LayerId, Option<LayerId>)> = self
            .layers
            .iter()
            .chain(spare)
            .map(|l| (l.id, l.parent))
            .collect();
        let mut wanted: std::collections::HashSet<LayerId> = match dirty {
            Dirty::All => parents.iter().map(|(id, _)| *id).collect(),
            Dirty::Layers(ids) => ids.iter().copied().collect(),
        };
        // グループは中身ごと（ID から親をたどって、増えなくなるまで）
        loop {
            let before = wanted.len();
            for (id, parent) in &parents {
                if parent.is_some_and(|p| wanted.contains(&p)) {
                    wanted.insert(*id);
                }
            }
            if wanted.len() == before {
                break;
            }
        }
        let was_by_id: std::collections::HashMap<LayerId, &Layer> =
            spare.iter().map(|l| (l.id, l)).collect();
        let mut changed: Vec<(LayerId, super::eval::SourceKey, TileCoord)> = Vec::new();
        for l in self.layers.iter().filter(|l| wanted.contains(&l.id)) {
            let was = was_by_id.get(&l.id).copied();
            let channels: std::collections::BTreeSet<crate::Channel> = l
                .surface_channels()
                .into_iter()
                .chain(was.into_iter().flat_map(Layer::surface_channels))
                .collect();
            let mut pairs: Vec<(super::eval::SourceKey, Option<&Surface>, Option<&Surface>)> =
                channels
                    .into_iter()
                    .map(|c| {
                        (
                            super::eval::SourceKey::Channel(c),
                            l.surface(c),
                            was.and_then(|w| w.surface(c)),
                        )
                    })
                    .collect();
            pairs.push((
                super::eval::SourceKey::Mask,
                l.mask.as_ref().map(|m| &m.surface),
                was.and_then(|w| w.mask.as_ref().map(|m| &m.surface)),
            ));
            for (key, now, before) in pairs {
                let coords: std::collections::BTreeSet<TileCoord> = now
                    .into_iter()
                    .chain(before)
                    .flat_map(Surface::tile_coords)
                    .collect();
                for coord in coords {
                    let same = Tile::same(
                        now.and_then(|s| s.tile(coord)),
                        before.and_then(|s| s.tile(coord)),
                    );
                    if !same {
                        changed.push((l.id, key, coord));
                    }
                }
            }
        }
        for (id, key, coord) in changed {
            self.effects.clock += 1;
            let serial = self.effects.clock;
            self.effects
                .source
                .entry((id, key))
                .or_default()
                .insert(coord, serial);
        }
    }
    fn mark_dirty(&mut self, dirty: &Dirty, spare: &[Layer]) {
        match dirty {
            Dirty::All => {
                for i in 0..self.layers.len() {
                    self.mark_layer(i, None);
                }
            }
            Dirty::Layers(ids) => {
                for &id in ids {
                    self.mark_layer_anywhere(id, spare, None);
                }
            }
        }
    }
    /// 準備用の文書で operation を実行し、何か変わったら 1 段で交換する。operation は結果と、合成が変わり得る範囲を返す。
    pub(super) fn batch_layers<T>(
        &mut self,
        operation: impl FnOnce(&mut Document) -> Result<(T, Dirty), CoreError>,
    ) -> Result<T, CoreError> {
        self.batch_layers_cost(None, operation)
    }
    pub(super) fn batch_layers_cost<T>(
        &mut self,
        cost: Option<u64>,
        operation: impl FnOnce(&mut Document) -> Result<(T, Dirty), CoreError>,
    ) -> Result<T, CoreError> {
        self.ensure_no_stroke()?;
        let mut copy = self.edit_copy()?;
        let (result, dirty) = operation(&mut copy)?;
        if !copy.undo.is_empty() {
            let cost = cost.unwrap_or_else(|| copy.undo.iter().map(|e| e.cost).sum::<u64>());
            self.commit_copy(copy, cost, dirty)?;
        }
        Ok(result)
    }
    /// 重複と選んだグループの子孫を除いた対象を下から上に返す。未知の ID は断る。
    pub fn topmost_of(&self, ids: &[LayerId]) -> Result<Vec<LayerId>, CoreError> {
        for &id in ids {
            self.index_of(id)?;
        }
        Ok(self
            .layers
            .iter()
            .enumerate()
            .filter(|(i, l)| {
                ids.contains(&l.id)
                    && !ids
                        .iter()
                        .any(|&id| id != l.id && self.is_descendant(*i, id))
            })
            .map(|(_, l)| l.id)
            .collect())
    }
    pub fn remove_layers(&mut self, ids: &[LayerId]) -> Result<(), CoreError> {
        let members = self.topmost_of(ids)?;
        let bytes = self
            .layers
            .iter()
            .enumerate()
            .filter(|(i, l)| {
                members
                    .iter()
                    .any(|&m| m == l.id || self.is_descendant(*i, m))
            })
            .map(|(_, l)| l.allocated_bytes())
            .sum::<u64>();
        self.batch_layers_cost(Some(128 + bytes), |d| {
            for &id in &members {
                d.remove_layer(id)?;
            }
            Ok(((), Dirty::Layers(members)))
        })
    }
    pub fn duplicate_layers(&mut self, ids: &[LayerId]) -> Result<Vec<LayerId>, CoreError> {
        let members = self.topmost_of(ids)?;
        self.batch_layers(|d| {
            let copies: Vec<LayerId> = members
                .iter()
                .map(|&id| d.duplicate_layer(id, None))
                .collect::<Result<_, _>>()?;
            let dirty = Dirty::Layers(copies.clone());
            Ok((copies, dirty))
        })
    }
    pub fn set_layers_visibility(
        &mut self,
        ids: &[LayerId],
        visible: bool,
    ) -> Result<(), CoreError> {
        self.batch_layers(|d| {
            for &id in ids {
                d.set_layer_visible(id, visible)?;
            }
            Ok(((), Dirty::Layers(ids.to_vec())))
        })
    }
    pub fn move_layers(
        &mut self,
        ids: &[LayerId],
        parent: Option<LayerId>,
        position: usize,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let members = self.topmost_of(ids)?;
        if members.is_empty() {
            return Ok(());
        }
        if let Some(p) = parent {
            let i = self.index_of(p)?;
            if !self.layers[i].is_group()
                || members.iter().any(|&m| m == p || self.is_descendant(i, m))
            {
                return Err(CoreError::InvalidArgument("移動先のグループ"));
            }
        }
        let mut tree = self.tree();
        let before = tree.order.clone();
        let mut blocks = Vec::new();
        for &m in &members {
            blocks.extend(tree.block(m));
        }
        tree.order.retain(|e| !blocks.contains(&e.0));
        let siblings: Vec<_> = tree
            .order
            .iter()
            .filter(|e| e.1 == parent)
            .map(|e| e.0)
            .collect();
        if position > siblings.len() {
            return Err(CoreError::InvalidArgument("position"));
        }
        let at = if position < siblings.len() {
            tree.subtree_start(tree.index(siblings[position]))
        } else {
            parent.map_or(tree.order.len(), |p| tree.index(p))
        };
        let moved: Vec<_> = before
            .iter()
            .filter(|e| blocks.contains(&e.0))
            .map(|&(id, p)| (id, if members.contains(&id) { parent } else { p }))
            .collect();
        tree.order.splice(at..at, moved);
        if before == tree.order {
            return Ok(());
        }
        Self::check_nesting(&tree)?;
        self.execute(
            Command::Structure {
                before,
                after: tree.order,
                moved: blocks,
                spare: Vec::new(),
            },
            64,
        )
    }
    pub fn step_layers(&mut self, ids: &[LayerId], up: bool) -> Result<bool, CoreError> {
        let chosen = self.topmost_of(ids)?;
        let mut members = chosen.clone();
        if up {
            members.reverse();
        }
        let moved = self.batch_layers_cost(Some(64), |d| {
            let mut moved = Vec::new();
            for id in members {
                let p = d.layer(id).expect("対象").parent;
                let siblings: Vec<_> = d
                    .layers
                    .iter()
                    .filter(|l| l.parent == p)
                    .map(|l| l.id)
                    .collect();
                let at = siblings.iter().position(|&l| l == id).expect("兄弟");
                let to = if up {
                    at.checked_add(1)
                } else {
                    at.checked_sub(1)
                };
                if let Some(to) =
                    to.filter(|&to| to < siblings.len() && !chosen.contains(&siblings[to]))
                {
                    d.move_layer_to(id, p, to)?;
                    moved.push(id);
                }
            }
            let dirty = Dirty::Layers(moved.clone());
            Ok((moved, dirty))
        })?;
        Ok(!moved.is_empty())
    }
}
