//! レイヤーの足し引き・並べ替え・グループ（C# の PaintDocument の AddLayer・AddGroup・GroupLayers・Ungroup・MoveLayerTo・RemoveLayer・
//! ValidateStructure）と、チャンネルの一覧の編集。
//!
//! 並びの計算は ID と親の列（[`Tree`]）の上で行い、結果の前後を 1 つの段（Structure）にする。グループの中身はいつもグループの
//! すぐ下に続けて並ぶ。

use std::collections::{HashMap, HashSet};

use super::{Command, Document, Order};
use crate::adjust::AdjustmentSettings;
use crate::error::CoreError;
use crate::layer::{ChannelBlend, Layer, LayerId};
use crate::surface::Surface;
use crate::types::{Channel, ChannelInfo, LayerKind, Rgba8};

/// グループの入れ子の上限（1 本の鎖に重なるグループの数。レイヤーはこの数のグループの中まで入れられる）。
///
/// 合成・面の計算・書き出しはグループの入れ子を再帰でたどる。スタックの実測は合成だけ・Linux だけ（合成を別のスレッドで動かし、
/// 足りる最小のスタックを 32KB 刻みで探した）: 64 段は dev（opt-level 1）も release も 288KB で溢れ 320KB で収まり、32 段は dev が
/// 160KB で溢れ 192KB で収まり、release が 128KB で溢れ 160KB で収まる（1 段あたり約 4KB）。Windows の画面のスレッドは 1MiB で、
/// 64 段はその約 3 分の 1 に収まるが、Windows での実測と、画面の枠組みが先に使う分は未確認。128 段（以前の PSD の書き出しの上限）は
/// 測っていない。実際の絵は 10 段も重ねない。.ylp の読み手・PSD の取り込み・編集（まとめる・動かす・グループを足す・スマート素材を
/// 置く）がこの値で断る。
pub const MAX_GROUP_DEPTH: usize = 64;

/// 並びと入れ子（ID と親、下から上）と、どれがグループか。
pub(crate) struct Tree {
    pub order: Order,
    groups: HashSet<LayerId>,
}

impl Tree {
    pub(super) fn index(&self, id: LayerId) -> usize {
        self.order
            .iter()
            .position(|e| e.0 == id)
            .expect("並びにある")
    }
    fn is_group(&self, id: LayerId) -> bool {
        self.groups.contains(&id)
    }
    /// 並びの i 番がグループ group の中（子・孫）にあるか。
    fn is_descendant(&self, i: usize, group: LayerId) -> bool {
        let mut parent = self.order[i].1;
        let mut hops = 0;
        while let Some(p) = parent {
            if p == group {
                return true;
            }
            hops += 1;
            if hops > self.order.len() {
                return false; // 入れ子が輪になっている（検査で断る）
            }
            parent = match self.order.iter().find(|e| e.0 == p) {
                Some(e) => e.1,
                None => return false,
            };
        }
        false
    }
    /// まとまり（top のレイヤーと、グループなら中身）の一番下の位置。
    pub(super) fn subtree_start(&self, top: usize) -> usize {
        let mut start = top;
        if self.is_group(self.order[top].0) {
            let id = self.order[top].0;
            while start > 0 && self.is_descendant(start - 1, id) {
                start -= 1;
            }
        }
        start
    }
    /// まとまりの ID（下から上）。
    pub(super) fn block(&self, id: LayerId) -> Vec<LayerId> {
        let top = self.index(id);
        let start = self.subtree_start(top);
        self.order[start..=top].iter().map(|e| e.0).collect()
    }
    /// まとまりを parent の position（その親の兄弟の中の位置、0 = 一番下）へ動かす（C# の MoveSubtreeInternal）。
    fn move_subtree(&mut self, id: LayerId, parent: Option<LayerId>, position: usize) {
        let top = self.index(id);
        let start = self.subtree_start(top);
        let mut block: Vec<_> = self.order.drain(start..=top).collect();
        let siblings: Vec<usize> = (0..self.order.len())
            .filter(|&i| self.order[i].1 == parent)
            .collect();
        let insert_at = if position < siblings.len() {
            self.subtree_start(siblings[position])
        } else {
            match parent {
                None => self.order.len(),
                Some(p) => self.index(p), // グループの記録のすぐ下 = 子の一番上
            }
        };
        block.last_mut().expect("まとまりは空でない").1 = parent;
        self.order.splice(insert_at..insert_at, block);
    }
}

/// チャンネルを消すときに段が持つ、レイヤーのそのチャンネルの中身。
#[derive(Clone, Debug, Default)]
pub(crate) struct ChannelContents {
    surface: Option<Surface>,
    fill: Option<Rgba8>,
    enabled: bool,
    blend: ChannelBlend,
}

impl Document {
    pub(super) fn tree(&self) -> Tree {
        Tree {
            order: self.order(),
            groups: self
                .layers
                .iter()
                .filter(|l| l.is_group())
                .map(|l| l.id)
                .collect(),
        }
    }
    fn order(&self) -> Order {
        self.layers.iter().map(|l| (l.id, l.parent)).collect()
    }

    /// 並びの i 番のレイヤーがグループ group の中にあるか。
    pub(super) fn is_descendant(&self, i: usize, group: LayerId) -> bool {
        let mut parent = self.layers[i].parent;
        let mut hops = 0;
        while let Some(p) = parent {
            if p == group {
                return true;
            }
            hops += 1;
            if hops > self.layers.len() {
                return false;
            }
            parent = match self.layer(p) {
                Some(l) => l.parent,
                None => return false,
            };
        }
        false
    }

    /// 並び・入れ子を order にする。order に無いレイヤーは spare へ、spare のレイヤーで order にあるものは文書へ。
    pub(super) fn restore_structure(&mut self, order: &Order, spare: &mut Vec<Layer>) {
        let mut pool: std::collections::HashMap<LayerId, Layer> = self
            .layers
            .drain(..)
            .chain(spare.drain(..))
            .map(|l| (l.id, l))
            .collect();
        for (id, parent) in order {
            let mut l = pool.remove(id).expect("並びのレイヤーは文書か段にある");
            l.parent = *parent;
            self.layers.push(l);
        }
        spare.extend(pool.into_values());
    }

    /// まとまりを index へ入れる（予算を先に確かめる）。
    pub(super) fn put_block(
        &mut self,
        index: usize,
        block: &mut Option<Vec<Layer>>,
    ) -> Result<(), CoreError> {
        let layers = block.take().expect("段がまとまりを持つ");
        let bytes: u64 = layers.iter().map(|l| l.allocated_bytes()).sum();
        if let Err(e) = self.ensure_source_growth(bytes) {
            *block = Some(layers);
            return Err(e);
        }
        let n = layers.len();
        self.layers.splice(index..index, layers);
        for i in index..index + n {
            self.mark_layer(i, None);
        }
        self.mark_clipped_layers();
        Ok(())
    }

    /// まとまりを index から抜いて段に持たせる。
    pub(super) fn take_block(
        &mut self,
        index: usize,
        len: usize,
        block: &mut Option<Vec<Layer>>,
    ) -> Result<(), CoreError> {
        for i in index..index + len {
            self.mark_layer(i, None);
        }
        *block = Some(self.layers.drain(index..index + len).collect());
        self.mark_clipped_layers();
        Ok(())
    }

    /// 新しいレイヤーを above のすぐ上（同じグループの中）へ、なければ一番上へ入れる（1 回の Undo）。
    pub(super) fn insert_new(
        &mut self,
        layer: Layer,
        above: Option<LayerId>,
    ) -> Result<LayerId, CoreError> {
        self.insert_new_costed(layer, above, 128)
    }

    /// `insert_new` の、履歴の費用を指定する形（レイヤーが画素やパスを持って入るとき、その分も費用に数える）。
    pub(super) fn insert_new_costed(
        &mut self,
        mut layer: Layer,
        above: Option<LayerId>,
        cost: u64,
    ) -> Result<LayerId, CoreError> {
        self.ensure_no_stroke()?;
        let (index, parent) = match above {
            Some(a) => {
                let i = self.index_of(a)?;
                (i + 1, self.layers[i].parent)
            }
            None => (self.layers.len(), None),
        };
        layer.parent = parent;
        let id = layer.id;
        self.execute(
            Command::Insert {
                index,
                len: 1,
                block: Some(vec![layer]),
            },
            cost,
        )?;
        Ok(id)
    }

    /// 空のレイヤーを一番上に足す（Color のチャンネルを持つ）。1 回の Undo。
    pub fn add_layer(&mut self, name: &str) -> Result<LayerId, CoreError> {
        self.add_layer_above(name, None)
    }

    /// 空のレイヤーを above のすぐ上（above と同じグループの中。グループの上ならグループと中身の上）に足す（None なら一番上）。
    pub fn add_layer_above(
        &mut self,
        name: &str,
        above: Option<LayerId>,
    ) -> Result<LayerId, CoreError> {
        self.ensure_no_stroke()?;
        let id = self.new_layer_id();
        let mut layer = Layer::new(id, name, LayerKind::Raster);
        layer.put_surface(
            Channel::Color,
            Some(Surface::new(self.width, self.height, self.tile_size)),
        );
        layer.set_enabled(Channel::Color, true);
        self.insert_new(layer, above)
    }

    /// 空のグループを足す（既定は通過。ほかのモードにすると分離）。足し先（above と同じグループの中）の入れ子が上限を超えるなら
    /// 断り、何も変えない。
    pub fn add_group(&mut self, name: &str, above: Option<LayerId>) -> Result<LayerId, CoreError> {
        self.ensure_no_stroke()?;
        let parent = match above {
            Some(a) => self.layers[self.index_of(a)?].parent,
            None => None,
        };
        self.ensure_nesting_room(parent, 1)?;
        let id = self.new_layer_id();
        self.insert_new(Layer::new(id, name, LayerKind::Group), above)
    }

    /// 塗りつぶしレイヤーを足す。値はチャンネルごと（値のあるチャンネルは有効）で、値のあるチャンネルのキャンバス全体を覆う。見せる所は
    /// マスクで絞る。
    pub fn add_fill_layer(
        &mut self,
        name: &str,
        values: &[(Channel, Rgba8)],
        above: Option<LayerId>,
    ) -> Result<LayerId, CoreError> {
        self.ensure_no_stroke()?;
        for (c, _) in values {
            self.require_channel(*c)?;
        }
        let id = self.new_layer_id();
        let mut layer = Layer::new(id, name, LayerKind::Fill);
        for (c, v) in values {
            layer.fill.insert(*c, *v);
            layer.set_enabled(*c, true);
        }
        self.insert_new(layer, above)
    }

    /// 調整レイヤーを足す。channels のチャンネルで下の合成を変える（None なら、その調整を使えるチャンネル全部）。色相/彩度は
    /// 色のチャンネルだけ（使えないチャンネルを名指ししたら断る）。
    pub fn add_adjustment_layer(
        &mut self,
        name: &str,
        settings: AdjustmentSettings,
        channels: Option<&[Channel]>,
        above: Option<LayerId>,
    ) -> Result<LayerId, CoreError> {
        self.ensure_no_stroke()?;
        settings.validate()?;
        let id = self.new_layer_id();
        let mut layer = Layer::new(id, name, LayerKind::Adjustment);
        layer.adjustment = Some(settings.clone());
        let all = self.channels();
        for &c in channels.unwrap_or(&all) {
            let kind = self.channel_kind(c)?;
            if !settings.applies_to(kind) {
                if channels.is_none() {
                    continue;
                }
                return Err(CoreError::Unsupported("色相/彩度は色のチャンネルだけ"));
            }
            layer.set_enabled(c, true);
        }
        self.insert_new(layer, above)
    }

    /// レイヤーを取り除く。グループは中身ごと。1 回の Undo で、同じレイヤー（ID・画素・属性）が同じ所へ戻る。
    pub fn remove_layer(&mut self, id: LayerId) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let top = self.index_of(id)?;
        let tree = self.tree();
        let start = tree.subtree_start(top);
        let bytes: u64 = self.layers[start..=top]
            .iter()
            .map(|l| l.allocated_bytes())
            .sum();
        self.execute(
            Command::Remove {
                index: start,
                len: top - start + 1,
                block: None,
            },
            128 + bytes,
        )
    }

    /// レイヤーを写す（グループなら中身ごと）: 属性・チャンネルの面と値・有効・合成の設定・マスクを、新しい ID で元のすぐ上（同じグループ）へ。
    /// タイルは写し元と共有し（書いたときに複製）、画素の予算には全部数える。name が None なら同じ名前。1 回の Undo。写しの ID を返す。
    pub fn duplicate_layer(
        &mut self,
        id: LayerId,
        name: Option<&str>,
    ) -> Result<LayerId, CoreError> {
        self.ensure_no_stroke()?;
        let top = self.index_of(id)?;
        let start = self.tree().subtree_start(top);
        let mut ids = std::collections::HashMap::new();
        for i in start..=top {
            let new = self.new_layer_id();
            ids.insert(self.layers[i].id, new);
        }
        let mut copies: Vec<Layer> = self.layers[start..=top].to_vec();
        let mut bytes = 0;
        for l in &mut copies {
            l.id = ids[&l.id];
            if let Some(p) = l.parent {
                if let Some(np) = ids.get(&p) {
                    l.parent = Some(*np);
                }
            }
            bytes += l.allocated_bytes();
        }
        // 段・Anchor は新しい ID（写しの中の Anchor を読む段は写しの Anchor を読む）
        self.renew_effect_ids(&mut copies);
        let copy_id = ids[&id];
        if let Some(n) = name {
            copies.last_mut().expect("空でない").name = n.to_string();
        }
        let len = copies.len();
        self.execute(
            Command::Insert {
                index: top + 1,
                len,
                block: Some(copies),
            },
            128 + bytes,
        )?;
        Ok(copy_id)
    }

    /// レイヤー（グループなら中身ごと）を、今のグループの兄弟の中の new_index（0 = 一番下）へ動かす。グループが無ければ並びの番号。
    pub fn move_layer(&mut self, id: LayerId, new_index: usize) -> Result<(), CoreError> {
        let parent = self.layer(id).ok_or(CoreError::LayerNotFound)?.parent;
        self.move_layer_to(id, parent, new_index)
    }

    /// レイヤー（グループなら中身ごと）をグループ parent（None は一番上の段）の子の position（0 = 一番下）へ動かす。
    /// グループを自分の中へは入れられない。同じ所なら何もしない。
    pub fn move_layer_to(
        &mut self,
        id: LayerId,
        parent: Option<LayerId>,
        position: usize,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        self.index_of(id)?;
        if let Some(p) = parent {
            let pl = self.layer(p).ok_or(CoreError::LayerNotFound)?;
            if !pl.is_group() {
                return Err(CoreError::InvalidArgument("グループにだけ入れられる"));
            }
            let mut cur = Some(p);
            while let Some(c) = cur {
                if c == id {
                    return Err(CoreError::InvalidArgument(
                        "グループを自分の中へは入れられない",
                    ));
                }
                cur = self.layer(c).and_then(|l| l.parent);
            }
        }
        let siblings = self
            .layers
            .iter()
            .filter(|l| l.parent == parent && l.id != id)
            .count();
        if position > siblings {
            return Err(CoreError::InvalidArgument("new_index"));
        }
        let mut tree = self.tree();
        let before = tree.order.clone();
        let moved = tree.block(id);
        tree.move_subtree(id, parent, position);
        if tree.order == before {
            return Ok(());
        }
        Self::check_nesting(&tree)?;
        self.execute(
            Command::Structure {
                before,
                after: tree.order,
                moved,
                spare: Vec::new(),
            },
            64,
        )
    }

    /// 兄弟のレイヤーをまとめて新しいグループ（通過）に入れる。グループは一番上の対象の所に置き、対象の並びは保つ。1 回の Undo。
    pub fn group_layers(&mut self, ids: &[LayerId], name: &str) -> Result<LayerId, CoreError> {
        self.ensure_no_stroke()?;
        if ids.is_empty() {
            return Err(CoreError::InvalidArgument("まとめるレイヤーが無い"));
        }
        let mut members: Vec<usize> = Vec::new();
        for id in ids {
            let i = self.index_of(*id)?;
            if !members.contains(&i) {
                members.push(i);
            }
        }
        let parent = self.layers[members[0]].parent;
        if members.iter().any(|&i| self.layers[i].parent != parent) {
            return Err(CoreError::InvalidArgument(
                "同じグループの中のレイヤーだけをまとめられる",
            ));
        }
        members.sort();
        let gid = self.new_layer_id();
        let mut group = Layer::new(gid, name, LayerKind::Group);
        group.parent = parent;
        let mut tree = self.tree();
        let before = tree.order.clone();
        let member_ids: Vec<LayerId> = members.iter().map(|&i| self.layers[i].id).collect();
        let mut moved = Vec::new();
        for m in &member_ids {
            moved.extend(tree.block(*m));
        }
        // グループを一番上の対象の上に置き、対象を下から順に中へ入れる
        let insert_at = tree.index(*member_ids.last().expect("空でない")) + 1;
        tree.order.insert(insert_at, (gid, parent));
        tree.groups.insert(gid);
        for (k, m) in member_ids.iter().enumerate() {
            tree.move_subtree(*m, Some(gid), k);
        }
        Self::check_nesting(&tree)?;
        self.execute(
            Command::Structure {
                before,
                after: tree.order,
                moved,
                spare: vec![group],
            },
            128,
        )?;
        Ok(gid)
    }

    /// グループを外して中身を残す（中身はグループのいた所へ）。1 回の Undo。
    pub fn ungroup(&mut self, id: LayerId) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(id)?;
        if !self.layers[index].is_group() {
            return Err(CoreError::Unsupported("グループではない"));
        }
        let tree = self.tree();
        let before = tree.order.clone();
        let moved = tree.block(id);
        let parent = self.layers[index].parent;
        // 外したグループのレイヤー（マスクの画素も）は段が持つ
        let held = self.layers[index].allocated_bytes();
        // 子はもうグループのすぐ下にあるので、グループの記録を抜くだけで位置が保たれる
        let after: Order = before
            .iter()
            .filter(|e| e.0 != id)
            .map(|&(l, p)| (l, if p == Some(id) { parent } else { p }))
            .collect();
        self.execute(
            Command::Structure {
                before,
                after,
                moved,
                spare: Vec::new(),
            },
            128 + held,
        )
    }

    /// 全部のレイヤーがラスターで、グループ・マスク・チャンネルごとの合成・結果を変えるフィルターが無いか（M1 の合成の形。これが偽なら、
    /// レイヤーの種類や評価済みの効果を知らない合成（GPU の M1 の経路など）では同じ絵にならない）。
    pub fn is_plain_stack(&self) -> bool {
        self.layers.iter().all(|l| {
            l.kind == LayerKind::Raster
                && l.parent.is_none()
                && l.mask.is_none()
                && l.blends.is_empty()
                && !l.filters.iter().any(|e| e.is_active())
        })
    }

    /// グループの直下の子（下から上。None は一番上の段）。
    pub fn children_of(&self, parent: Option<LayerId>) -> Result<Vec<LayerId>, CoreError> {
        if let Some(p) = parent {
            if !self.layer(p).ok_or(CoreError::LayerNotFound)?.is_group() {
                return Err(CoreError::InvalidArgument("グループではない"));
            }
        }
        Ok(self
            .layers
            .iter()
            .filter(|l| l.parent == parent)
            .map(|l| l.id)
            .collect())
    }

    /// 入れ子の深さ（一番上の段は 0）。
    pub fn depth_of(&self, id: LayerId) -> Result<usize, CoreError> {
        let mut layer = self.layer(id).ok_or(CoreError::LayerNotFound)?;
        let mut depth = 0;
        while let Some(p) = layer.parent {
            layer = self.layer(p).ok_or(CoreError::LayerNotFound)?;
            depth += 1;
            if depth > self.layers.len() {
                return Err(CoreError::InvalidArgument("入れ子が輪になっている"));
            }
        }
        Ok(depth)
    }

    /// 入れ子を確かめる: 親はあってグループ、輪が無い、グループの中身はグループのすぐ下に続く（C# の ValidateStructure）。
    pub fn validate_structure(&self) -> Result<(), CoreError> {
        Self::validate_order(&self.tree())
    }

    /// 並びを 1 回なめて確かめる（上のレイヤーから下へ。開いているグループの鎖を持ち、親でない所へ戻れば鎖を閉じる）。
    /// 親はあってグループ・親は子の上に並ぶ・閉じたグループへ戻らない（= 子が連続している）・グループの入れ子が上限以内。
    /// レイヤーの数 n に対して O(n)（親の鎖をたどる確かめをレイヤーごとにしない）。
    fn validate_order(tree: &Tree) -> Result<(), CoreError> {
        let order = &tree.order;
        let index_of: HashMap<LayerId, usize> =
            order.iter().enumerate().map(|(i, e)| (e.0, i)).collect();
        let mut open: Vec<LayerId> = Vec::new();
        for i in (0..order.len()).rev() {
            let (id, parent) = order[i];
            match parent {
                None => open.clear(),
                Some(p) => {
                    let Some(&pi) = index_of.get(&p) else {
                        return Err(CoreError::InvalidArgument("無いグループに入っている"));
                    };
                    if !tree.is_group(p) {
                        return Err(CoreError::InvalidArgument(
                            "グループでないレイヤーの中に入っている",
                        ));
                    }
                    if pi == i {
                        return Err(CoreError::InvalidArgument(
                            "グループの入れ子が輪になっている",
                        ));
                    }
                    if pi < i {
                        return Err(CoreError::InvalidArgument("レイヤーはグループの下に並ぶ"));
                    }
                    // p が開いていなければ、p の子が途切れてから戻ってきた（子が連続していない）
                    while open.last() != Some(&p) {
                        if open.pop().is_none() {
                            return Err(CoreError::InvalidArgument("グループの中身が続いていない"));
                        }
                    }
                }
            }
            if tree.is_group(id) {
                if open.len() >= MAX_GROUP_DEPTH {
                    return Err(CoreError::InvalidArgument("グループの入れ子が深すぎる"));
                }
                open.push(id);
            }
        }
        Ok(())
    }

    /// 並び（親は子の上に並ぶ）の入れ子が上限以内か。レイヤーの動かし方・まとめ方を決めたあと、段にする前に確かめる。
    pub(super) fn check_nesting(tree: &Tree) -> Result<(), CoreError> {
        let mut depth: HashMap<LayerId, usize> = HashMap::with_capacity(tree.order.len());
        for &(id, parent) in tree.order.iter().rev() {
            let d = parent.map_or(0, |p| depth.get(&p).map_or(0, |d| d + 1));
            if tree.is_group(id) && d >= MAX_GROUP_DEPTH {
                return Err(CoreError::InvalidArgument("グループの入れ子が深すぎる"));
            }
            depth.insert(id, d);
        }
        Ok(())
    }

    /// 親 `parent`（None は一番上の段）の中に、`inner` 本のグループが重なる鎖を持つまとまりを置いても、入れ子が上限以内か。
    pub(super) fn ensure_nesting_room(
        &self,
        parent: Option<LayerId>,
        inner: usize,
    ) -> Result<(), CoreError> {
        let mut above = 0;
        let mut cur = parent;
        while let Some(p) = cur {
            above += 1;
            if above > MAX_GROUP_DEPTH {
                break;
            }
            cur = self.layer(p).ok_or(CoreError::LayerNotFound)?.parent;
        }
        if above + inner > MAX_GROUP_DEPTH {
            return Err(CoreError::InvalidArgument("グループの入れ子が深すぎる"));
        }
        Ok(())
    }

    /// 読み込み用: レイヤーの親を今の並びの順にまとめて置く（履歴を消す）。入れ子が正しくなければ断って何も変えない。
    pub fn set_structure_for_load(&mut self, parents: &[Option<LayerId>]) -> Result<(), CoreError> {
        self.ensure_loadable()?;
        if parents.len() != self.layers.len() {
            return Err(CoreError::InvalidArgument("親の数がレイヤーの数と違う"));
        }
        let mut tree = self.tree();
        for (e, p) in tree.order.iter_mut().zip(parents) {
            e.1 = *p;
        }
        Self::validate_order(&tree)?;
        for (l, p) in self.layers.iter_mut().zip(parents) {
            l.parent = *p;
        }
        // 全部のレイヤーを 1 枚ずつ（グループの子孫もここで全部通るので、グループごとに子孫を数え直さない）
        for i in 0..self.layers.len() {
            self.mark_layer_alone(i, None);
        }
        self.external_mutation();
        Ok(())
    }

    // ───────── チャンネルの一覧 ─────────

    /// ユーザーチャンネルを足す（番号は 6 からの空き。名前は文書の中で重ならない）。1 回の Undo。
    pub fn add_channel(&mut self, info: ChannelInfo) -> Result<Channel, CoreError> {
        self.ensure_no_stroke()?;
        info.validate()?;
        if self.channels.iter().flatten().any(|c| c.name == info.name) {
            return Err(CoreError::InvalidArgument("同じ名前のチャンネルがある"));
        }
        let index = (Channel::STANDARD_COUNT..Channel::MAX)
            .find(|&i| self.channels.get(i).is_none_or(|c| c.is_none()))
            .ok_or(CoreError::InvalidArgument("チャンネルは 64 まで"))?;
        let channel = Channel::from_index(index).expect("64 未満");
        self.execute(
            Command::ChannelInfo {
                channel,
                old: None,
                new: Some(info),
                contents: Vec::new(),
            },
            64,
        )?;
        Ok(channel)
    }

    /// 読み込み用: ユーザーチャンネルを番号を指定して足す（保存した番号のまま戻すため。履歴を消す）。標準の番号・使っている番号・
    /// 重なる名前は断る。
    pub fn insert_channel_for_load(
        &mut self,
        channel: Channel,
        info: ChannelInfo,
    ) -> Result<(), CoreError> {
        self.ensure_loadable()?;
        info.validate()?;
        if channel.is_standard() || self.channel_info(channel).is_some() {
            return Err(CoreError::InvalidArgument("その番号のチャンネルはもうある"));
        }
        if self.channels.iter().flatten().any(|c| c.name == info.name) {
            return Err(CoreError::InvalidArgument("同じ名前のチャンネルがある"));
        }
        let i = channel.index();
        if self.channels.len() <= i {
            self.channels.resize(i + 1, None);
        }
        self.channels[i] = Some(info);
        self.external_mutation();
        Ok(())
    }

    /// ユーザーチャンネルの情報を変える（名前・種類・色空間・既定の値）。標準のチャンネルは Unity 版の番号と意味に結び付くので
    /// 変えられない。1 回の Undo。
    pub fn set_channel_info(
        &mut self,
        channel: Channel,
        info: ChannelInfo,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let old = self.require_channel(channel)?.clone();
        if channel.is_standard() {
            return Err(CoreError::Unsupported("標準のチャンネルは変えられない"));
        }
        info.validate()?;
        if old == info {
            return Ok(());
        }
        if info.kind != old.kind {
            // 有効な調整がその種類に使えなくなる（色相/彩度は色だけ）なら断る: 先にそのレイヤーのチャンネルを無効に
            let stuck = self.layers.iter().any(|l| {
                l.is_channel_enabled(channel)
                    && l.adjustment
                        .as_ref()
                        .is_some_and(|a| !a.applies_to(info.kind))
            });
            if stuck {
                return Err(CoreError::Unsupported("色相/彩度のレイヤーが有効"));
            }
        }
        if self
            .channels
            .iter()
            .enumerate()
            .any(|(i, c)| i != channel.index() && c.as_ref().is_some_and(|c| c.name == info.name))
        {
            return Err(CoreError::InvalidArgument("同じ名前のチャンネルがある"));
        }
        let cost = 64 + 2 * (old.name.len() + info.name.len()) as u64;
        self.execute(
            Command::ChannelInfo {
                channel,
                old: Some(old),
                new: Some(info),
                contents: Vec::new(),
            },
            cost,
        )
    }

    /// ユーザーチャンネルを消す。レイヤーのそのチャンネルの中身（面・塗りつぶしの値・有効・合成の設定）も消え、1 回の Undo で戻る。
    pub fn remove_channel(&mut self, channel: Channel) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let old = self.require_channel(channel)?.clone();
        if channel.is_standard() {
            return Err(CoreError::Unsupported("標準のチャンネルは消せない"));
        }
        let bytes: u64 = self
            .layers
            .iter()
            .filter_map(|l| l.surface(channel))
            .map(|s| s.allocated_bytes())
            .sum();
        self.execute(
            Command::ChannelInfo {
                channel,
                old: Some(old),
                new: None,
                contents: Vec::new(),
            },
            64 + bytes,
        )
    }

    /// チャンネルの一覧の段を当てる（backwards なら戻す）。
    pub(super) fn switch_channel_info(
        &mut self,
        channel: Channel,
        old: &Option<ChannelInfo>,
        new: &Option<ChannelInfo>,
        contents: &mut Vec<(LayerId, ChannelContents)>,
        backwards: bool,
    ) -> Result<(), CoreError> {
        let (from, to) = if backwards { (new, old) } else { (old, new) };
        let i = channel.index();
        if from.is_some() && to.is_none() {
            // 消す: レイヤーの中身を段へ
            for l in 0..self.layers.len() {
                self.mark_layer(l, Some(channel));
            }
            contents.clear();
            for layer in &mut self.layers {
                let c = ChannelContents {
                    surface: layer.surfaces.get_mut(i).and_then(|s| s.take()),
                    fill: layer.fill.remove(&channel),
                    enabled: layer.is_channel_enabled(channel),
                    blend: layer.channel_blend(channel),
                };
                layer.put_surface(channel, None);
                layer.set_enabled(channel, false);
                layer.blends.remove(&channel);
                if c.surface.is_some() || c.fill.is_some() || c.enabled || !c.blend.is_empty() {
                    contents.push((layer.id, c));
                }
            }
            self.channels[i] = None;
        } else if from.is_none() && to.is_some() {
            // 足す（消したのを戻すなら、レイヤーの中身も戻す）
            let bytes: u64 = contents
                .iter()
                .filter_map(|(_, c)| c.surface.as_ref())
                .map(|s| s.allocated_bytes())
                .sum();
            self.ensure_source_growth(bytes)?;
            if self.channels.len() <= i {
                self.channels.resize(i + 1, None);
            }
            self.channels[i] = to.clone();
            for (id, c) in contents.drain(..) {
                let index = self.index_of(id)?;
                let layer = &mut self.layers[index];
                layer.put_surface(channel, c.surface);
                if let Some(v) = c.fill {
                    layer.fill.insert(channel, v);
                }
                layer.set_enabled(channel, c.enabled);
                layer.set_channel_blend(channel, c.blend);
            }
            for l in 0..self.layers.len() {
                self.mark_layer(l, Some(channel));
            }
        } else {
            // 情報だけを変える（種類が変われば合成の式が変わる）
            self.channels[i] = to.clone();
            for l in 0..self.layers.len() {
                self.mark_layer(l, Some(channel));
            }
        }
        while matches!(self.channels.last(), Some(None)) {
            self.channels.pop();
        }
        Ok(())
    }
}

/// レイヤーの並び（親は子の上に並ぶ。スマート素材の断片など）の中で、グループが重なる鎖の一番長い数。
pub(super) fn group_chain_height(layers: &[Layer]) -> usize {
    let mut depth: HashMap<LayerId, usize> = HashMap::with_capacity(layers.len());
    let mut height = 0;
    for l in layers.iter().rev() {
        let d = l.parent.map_or(0, |p| depth.get(&p).map_or(0, |d| d + 1));
        if l.is_group() {
            height = height.max(d + 1);
        }
        depth.insert(l.id, d);
    }
    height
}
