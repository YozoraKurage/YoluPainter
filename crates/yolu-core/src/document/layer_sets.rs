//! レイヤーセット: テクスチャセットの文書の中の、名前の付いたレイヤーの束と、チャンネルごとの出力の割り当て（最終の結果をどのレイヤーセット
//! から取るか）。
//!
//! - レイヤーの並びは文書に 1 本のまま（[`Layer::set`] でどれか 1 つのレイヤーセットに入る）。合成はレイヤーセットごとに、そのレイヤーセットの
//!   レイヤーだけで行い（[`Document::set_stack`]）、チャンネル c の最終の結果は、割り当てたレイヤーセットの c の合成。割り当てが無ければ c の既定値。
//! - 一番上の段の兄弟は、同じレイヤーセットのレイヤーだけ（グループの子は親と同じレイヤーセット）。並べ替え・親子・グループにまとめる・結合は、
//!   同じレイヤーセットの中だけで行い、またぐ要求は断る。
//! - 新しい文書・前の版の文書は、名前の空のレイヤーセット 1 つに全部のチャンネルを割り当てた形で、合成も保存のバイトもレイヤーセットの無かった
//!   ときと同じ。

use std::collections::HashMap;

use super::{Command, Document, Entry, HistoryKind, Order};
use crate::effects::EffectSettings;
use crate::error::CoreError;
use crate::generator;
use crate::layer::{Layer, LayerId, LayerSetId};
use crate::types::Channel;

/// レイヤーセットの数の上限（PSD の書き出しはレイヤーセットごとにグループを 1 つ足す）。
pub const MAX_LAYER_SETS: usize = 256;
/// レイヤーセットの名前の上限（UTF-8 のバイト。保存の文字列の上限と同じ）。
const MAX_NAME_BYTES: usize = 4096;

/// レイヤーセットの ID と名前（名前は空でもよい。空は画面が言語ごとの既定の名前で見せる）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerSetInfo {
    pub id: LayerSetId,
    pub name: String,
}

/// チャンネルの番号ごとの出力の割り当て。
pub(crate) type Route = [Option<LayerSetId>; Channel::MAX];

/// レイヤーセットの一覧と出力の割り当て（履歴の段の前後）。
#[derive(Clone, Debug)]
pub(crate) struct Table {
    sets: Vec<LayerSetInfo>,
    route: Route,
}

/// レイヤーセットの履歴の段。
pub(crate) enum SetCommand {
    /// 一覧と割り当ての入れ替え（追加・名前・並べ替え・割り当て・削除の一覧の側）。
    Table { old: Box<Table>, new: Box<Table> },
    /// レイヤーを別のレイヤーセットへ移す（並び・親と、移したレイヤーの所属の入れ替え）。
    Members {
        before: Order,
        after: Order,
        /// (レイヤー, 前のレイヤーセット, 後のレイヤーセット)。
        sets: Vec<(LayerId, LayerSetId, LayerSetId)>,
    },
}

fn check_name(name: &str) -> Result<(), CoreError> {
    if name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
        return Err(CoreError::InvalidArgument("レイヤーセットの名前"));
    }
    Ok(())
}

impl Document {
    // ───────── 読む ─────────

    /// レイヤーセット（1 つ以上。並びは画面の並び）。
    pub fn layer_sets(&self) -> &[LayerSetInfo] {
        &self.layer_sets
    }

    /// チャンネルの出力の割り当て（最終の結果を取るレイヤーセット。None はそのチャンネルの既定値）。文書に無いチャンネルは断る。
    pub fn channel_output(&self, channel: Channel) -> Result<Option<LayerSetId>, CoreError> {
        self.require_channel(channel)?;
        Ok(self.route[channel.index()])
    }

    /// レイヤーが入っているレイヤーセット。
    pub fn layer_set_of(&self, id: LayerId) -> Result<LayerSetId, CoreError> {
        Ok(self.layers[self.index_of(id)?].set)
    }

    /// レイヤーセットが 1 つで、名前が空で、文書の全部のチャンネルをそこへ割り当てた形か（新しい文書・前の版の文書の形。保存はこの形なら
    /// レイヤーセットを書かない）。
    pub fn has_default_layer_sets(&self) -> bool {
        let [only] = self.layer_sets.as_slice() else {
            return false;
        };
        only.name.is_empty()
            && self
                .channels()
                .into_iter()
                .all(|c| self.route[c.index()] == Some(only.id))
    }

    /// 最終の結果を取るレイヤーセット（割り当てが無ければ None）。チャンネルが文書にあるかは見ない。
    pub(super) fn output_set(&self, channel: Channel) -> Result<Option<LayerSetId>, CoreError> {
        Ok(self.route.get(channel.index()).copied().flatten())
    }

    /// レイヤーセットの並びの番号（無ければ断る）。
    pub(super) fn require_layer_set(&self, set: LayerSetId) -> Result<usize, CoreError> {
        self.layer_sets
            .iter()
            .position(|s| s.id == set)
            .ok_or(CoreError::InvalidArgument("レイヤーセットが無い"))
    }

    /// 新しいレイヤーを入れる所: above のすぐ上（above と同じグループ・レイヤーセットの中）、無ければ最初のレイヤーセットの一番上。
    /// (並びの番号, 親, レイヤーセット)。
    pub(super) fn placement_above(
        &self,
        above: Option<LayerId>,
    ) -> Result<(usize, Option<LayerId>, LayerSetId), CoreError> {
        Ok(match above {
            Some(a) => {
                let i = self.index_of(a)?;
                (i + 1, self.layers[i].parent, self.layers[i].set)
            }
            None => (self.layers.len(), None, self.layer_sets[0].id),
        })
    }

    /// 新しいレイヤーセットの ID（文書の中で重ならない）。
    fn new_layer_set_id(&mut self) -> LayerSetId {
        loop {
            self.id_counter += 1;
            let id = LayerSetId(super::random_id(self.id_counter ^ 0x4c53_4554_0000_0000));
            if id.0 != 0 && self.require_layer_set(id).is_err() {
                return id;
            }
        }
    }

    fn table(&self) -> Table {
        Table {
            sets: self.layer_sets.clone(),
            route: self.route,
        }
    }

    fn table_entry(&self, new: Table) -> Entry {
        let cost = 64
            + new
                .sets
                .iter()
                .chain(self.layer_sets.iter())
                .map(|s| 32 + s.name.len() as u64)
                .sum::<u64>();
        Entry {
            kind: HistoryKind::Other,
            command: Command::LayerSets(SetCommand::Table {
                old: Box::new(self.table()),
                new: Box::new(new),
            }),
            cost,
        }
    }

    fn execute_table(&mut self, new: Table) -> Result<(), CoreError> {
        let entry = self.table_entry(new);
        self.execute(entry.command, entry.cost)
    }

    // ───────── 編集（どれも 1 回の Undo） ─────────

    /// 空のレイヤーセットを一覧の最後に足す。出力の割り当ては変えない。
    pub fn add_layer_set(&mut self, name: &str) -> Result<LayerSetId, CoreError> {
        self.ensure_no_stroke()?;
        check_name(name)?;
        if self.layer_sets.len() >= MAX_LAYER_SETS {
            return Err(CoreError::InvalidArgument("レイヤーセットの数の上限"));
        }
        let id = self.new_layer_set_id();
        let mut table = self.table();
        table.sets.push(LayerSetInfo {
            id,
            name: name.to_owned(),
        });
        self.execute_table(table)?;
        Ok(id)
    }

    /// レイヤーセットを画素ごと写し、元のすぐ後に置く（レイヤー・効果・Anchor・定規の ID は新しく振る。写しの中の Anchor を読む段は写しの
    /// Anchor を読む）。name が None なら同じ名前。タイルは写し元と共有し（書いたときに複製）、画素の予算には全部数え、超えるなら何も変えずに断る。
    /// 出力の割り当ては変えない。
    pub fn duplicate_layer_set(
        &mut self,
        set: LayerSetId,
        name: Option<&str>,
    ) -> Result<LayerSetId, CoreError> {
        self.ensure_no_stroke()?;
        let at = self.require_layer_set(set)?;
        let name = name.unwrap_or(&self.layer_sets[at].name).to_owned();
        check_name(&name)?;
        if self.layer_sets.len() >= MAX_LAYER_SETS {
            return Err(CoreError::InvalidArgument("レイヤーセットの数の上限"));
        }
        let mut copies: Vec<Layer> = self
            .layers
            .iter()
            .filter(|l| l.set == set)
            .cloned()
            .collect();
        let bytes: u64 = copies.iter().map(Layer::allocated_bytes).sum();
        self.ensure_source_growth(bytes)?;
        let id = self.new_layer_set_id();
        let mut ids: HashMap<LayerId, LayerId> = HashMap::with_capacity(copies.len());
        for l in &copies {
            let new = self.new_layer_id();
            ids.insert(l.id, new);
        }
        for l in &mut copies {
            l.id = ids[&l.id];
            l.parent = l.parent.map(|p| ids[&p]);
            l.set = id;
        }
        self.renew_effect_ids(&mut copies);
        let mut table = self.table();
        table.sets.insert(at + 1, LayerSetInfo { id, name });
        let mut steps = vec![self.table_entry(table)];
        if !copies.is_empty() {
            steps.push(Entry {
                kind: HistoryKind::AddLayer,
                command: Command::Insert {
                    index: self.layers.len(),
                    len: copies.len(),
                    block: Some(copies),
                },
                cost: 128 + bytes,
            });
        }
        self.execute_group(steps)?;
        Ok(id)
    }

    /// レイヤーセットの名前を変える（空でもよい）。
    pub fn rename_layer_set(&mut self, set: LayerSetId, name: &str) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let at = self.require_layer_set(set)?;
        check_name(name)?;
        if self.layer_sets[at].name == name {
            return Ok(());
        }
        let mut table = self.table();
        table.sets[at].name = name.to_owned();
        self.execute_table(table)
    }

    /// レイヤーセットを一覧の index（0 が最初）へ動かす。
    pub fn move_layer_set(&mut self, set: LayerSetId, index: usize) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let at = self.require_layer_set(set)?;
        if index >= self.layer_sets.len() {
            return Err(CoreError::InvalidArgument("レイヤーセットの位置"));
        }
        if index == at {
            return Ok(());
        }
        let mut table = self.table();
        let moved = table.sets.remove(at);
        table.sets.insert(index, moved);
        self.execute_table(table)
    }

    /// レイヤーセットを中のレイヤーごと消す。最後の 1 つは消せない。そのレイヤーセットから出していたチャンネルは、ほかのレイヤーセットへ移さず割り当てを無し（None）にする
    /// （最終の結果はそのチャンネルの既定値。ほかのレイヤーセットの中身が黙って結果に入らない）。
    pub fn remove_layer_set(&mut self, set: LayerSetId) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let at = self.require_layer_set(set)?;
        if self.layer_sets.len() == 1 {
            return Err(CoreError::InvalidArgument("最後のレイヤーセットは消せない"));
        }
        let mut table = self.table();
        table.sets.remove(at);
        for r in table.route.iter_mut() {
            if *r == Some(set) {
                *r = None;
            }
        }
        // 一番上の段のまとまり（グループは中身ごと）を、上から抜く（下の番号は変わらない）
        let tree = self.tree();
        let mut steps = Vec::new();
        for top in (0..self.layers.len()).rev() {
            let l = &self.layers[top];
            if l.set != set || l.parent.is_some() {
                continue;
            }
            let start = tree.subtree_start(top);
            let bytes: u64 = self.layers[start..=top]
                .iter()
                .map(Layer::allocated_bytes)
                .sum();
            steps.push(Entry {
                kind: HistoryKind::RemoveLayer,
                command: Command::Remove {
                    index: start,
                    len: top - start + 1,
                    block: None,
                },
                cost: 128 + bytes,
            });
        }
        steps.push(self.table_entry(table));
        self.execute_group(steps)
    }

    /// チャンネルの出力の割り当てを変える（None はそのチャンネルの既定値）。最終の結果がそのチャンネルのキャンバス全体で変わる。
    pub fn set_channel_output(
        &mut self,
        channel: Channel,
        set: Option<LayerSetId>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        self.require_channel(channel)?;
        if let Some(s) = set {
            self.require_layer_set(s)?;
        }
        if self.route[channel.index()] == set {
            return Ok(());
        }
        let mut table = self.table();
        table.route[channel.index()] = set;
        self.execute_table(table)
    }

    /// レイヤー（グループなら中身ごと）を別のレイヤーセットの一番上の段の一番上へ移す（選んだ順でなく、今の並びの下から上の順を保つ）。
    /// もうそのレイヤーセットにあるレイヤーは、その場に残す。
    pub fn move_layers_to_set(
        &mut self,
        ids: &[LayerId],
        set: LayerSetId,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        self.require_layer_set(set)?;
        let members: Vec<LayerId> = self
            .topmost_of(ids)?
            .into_iter()
            .filter(|id| self.layer(*id).is_some_and(|l| l.set != set))
            .collect();
        if members.is_empty() {
            return Ok(());
        }
        let tree = self.tree();
        let before = tree.order.clone();
        let mut blocks: Vec<LayerId> = Vec::new();
        for &m in &members {
            blocks.extend(tree.block(m));
        }
        let mut after: Order = before
            .iter()
            .filter(|e| !blocks.contains(&e.0))
            .copied()
            .collect();
        after.extend(
            before
                .iter()
                .filter(|e| blocks.contains(&e.0))
                .map(|&(id, p)| (id, if members.contains(&id) { None } else { p })),
        );
        let sets = blocks
            .iter()
            .map(|&id| (id, self.layer(id).expect("並びにある").set, set))
            .collect();
        self.execute(
            Command::LayerSets(SetCommand::Members {
                before,
                after,
                sets,
            }),
            64 + 48 * blocks.len() as u64,
        )
    }

    // ───────── 書き出し ─────────

    /// 書き出し用の写し（履歴なし、タイルは共有）: レイヤーセットごとのレイヤーを、一番上の段のグループ 1 つずつに包み、レイヤーセット 1 つにする。
    /// グループは分離（Normal）で、中身はそのレイヤーセットの合成と同じになる。並びはレイヤーセットの並びの最初が一番上。名前は `names[i]`、表示は
    /// `visible[i]`（`i` はレイヤーセットの並びの番号）。出力の割り当てのあるチャンネルは、写しの 1 つのレイヤーセットへ割り当てる。グループを足して
    /// 入れ子が上限を超えるなら断る。
    pub fn layer_sets_as_groups(
        &self,
        names: &[String],
        visible: &[bool],
    ) -> Result<Document, CoreError> {
        if names.len() != self.layer_sets.len() || visible.len() != self.layer_sets.len() {
            return Err(CoreError::InvalidArgument("レイヤーセットの名前・表示の数"));
        }
        let mut copy = self.capture_snapshot()?;
        let only = copy.layer_sets[0].id;
        // 元の文書で別のレイヤーセットの Anchor を指していた読み手の参照（`resolve_anchor` が `Missing` にして入力を通す）は、
        // 写しでは同じレイヤーセットに見えて解決されてしまうので、未選択（ID 0）にして同じく入力を通す。
        let points = self.anchor_points();
        let cross_set = |set: LayerSetId, id: u128| {
            id != 0
                && points
                    .iter()
                    .find(|p| p.id == id)
                    .is_some_and(|p| self.layers[p.host].set != set)
        };
        let mut layers = Vec::with_capacity(self.layers.len() + self.layer_sets.len());
        for (k, info) in self.layer_sets.iter().enumerate().rev() {
            let gid = copy.new_layer_id();
            for l in self.layers.iter().filter(|l| l.set == info.id) {
                let mut l = l.clone();
                let own = l.set;
                let masked = l.mask.as_mut().map(|m| &mut m.filters);
                for e in l.filters.iter_mut().chain(masked.into_iter().flatten()) {
                    if let EffectSettings::Generator(g) = &mut e.settings {
                        if g.kind == generator::Kind::Anchor && cross_set(own, g.anchor.id) {
                            g.anchor.id = 0;
                        }
                    }
                }
                l.parent = l.parent.or(Some(gid));
                l.set = only;
                layers.push(l);
            }
            let mut group = Layer::new(gid, &names[k], crate::types::LayerKind::Group);
            group.blend_mode = crate::types::BlendMode::Normal;
            group.visible = visible[k];
            group.set = only;
            layers.push(group);
        }
        copy.layers = layers;
        copy.layer_sets.truncate(1);
        copy.layer_sets[0].name.clear();
        for r in copy.route.iter_mut() {
            *r = r.map(|_| only);
        }
        copy.validate_structure()?;
        copy.external_mutation();
        Ok(copy)
    }

    // ───────── 読み込み ─────────

    /// 読み込み用: レイヤーセットの一覧・出力の割り当て・レイヤーの所属を置く（履歴を消す）。`members` はレイヤーの並びの順の所属。
    /// 割り当ての無いチャンネル（`route` に挙げないチャンネル）は最初のレイヤーセットへ。一覧が空・ID が 0 か重なる・在らないレイヤーセットを
    /// 指す・グループの子が親と違うレイヤーセット・文書に無いチャンネル・同じチャンネルを 2 度挙げる、のどれかなら断って何も変えない。
    pub fn set_layer_sets_for_load(
        &mut self,
        sets: Vec<LayerSetInfo>,
        route: &[(Channel, Option<LayerSetId>)],
        members: &[LayerSetId],
    ) -> Result<(), CoreError> {
        self.ensure_loadable()?;
        if sets.is_empty() || sets.len() > MAX_LAYER_SETS {
            return Err(CoreError::InvalidArgument("レイヤーセットの数"));
        }
        for (i, s) in sets.iter().enumerate() {
            check_name(&s.name)?;
            if s.id.0 == 0 || sets[..i].iter().any(|t| t.id == s.id) {
                return Err(CoreError::InvalidArgument(
                    "レイヤーセットの ID が空か重なっている",
                ));
            }
        }
        let known = |id: LayerSetId| sets.iter().any(|s| s.id == id);
        if members.len() != self.layers.len() || !members.iter().all(|m| known(*m)) {
            return Err(CoreError::InvalidArgument("レイヤーの所属のレイヤーセット"));
        }
        let position: HashMap<LayerId, usize> = self
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| (l.id, i))
            .collect();
        for (i, l) in self.layers.iter().enumerate() {
            if let Some(p) = l.parent {
                let pi = position[&p];
                if members[pi] != members[i] {
                    return Err(CoreError::InvalidArgument(
                        "グループの子が親と違うレイヤーセットにある",
                    ));
                }
            }
        }
        let mut table: Route = [Some(sets[0].id); Channel::MAX];
        let mut seen = 0u64;
        for &(c, s) in route {
            self.require_channel(c)?;
            if seen & c.bit() != 0 || s.is_some_and(|s| !known(s)) {
                return Err(CoreError::InvalidArgument("出力の割り当て"));
            }
            seen |= c.bit();
            table[c.index()] = s;
        }
        self.layer_sets = sets;
        self.route = table;
        for (l, m) in self.layers.iter_mut().zip(members) {
            l.set = *m;
        }
        for i in 0..self.layers.len() {
            self.mark_layer_alone(i, None);
        }
        self.external_mutation();
        Ok(())
    }

    // ───────── 段を当てる ─────────

    pub(super) fn switch_layer_sets(
        &mut self,
        command: &mut SetCommand,
        backwards: bool,
    ) -> Result<(), CoreError> {
        match command {
            SetCommand::Table { old, new } => {
                let to = if backwards { old } else { new };
                let changed: Vec<Channel> = (0..Channel::MAX)
                    .filter(|&i| self.route[i] != to.route[i])
                    .filter_map(Channel::from_index)
                    .collect();
                self.layer_sets = to.sets.clone();
                self.route = to.route;
                // 割り当てが変わったチャンネルは、最終の結果がキャンバス全体で変わる
                let tiles: Vec<_> = self.canvas_tiles().collect();
                for c in changed {
                    for coord in &tiles {
                        self.journal.mark(c, *coord);
                    }
                }
                Ok(())
            }
            SetCommand::Members {
                before,
                after,
                sets,
            } => {
                let to = if backwards { before } else { after };
                for (id, _, _) in sets.iter() {
                    let i = self.index_of(*id)?;
                    self.mark_layer_alone(i, None);
                }
                self.restore_structure(to, &mut Vec::new());
                for (id, old, new) in sets.iter() {
                    let i = self.index_of(*id)?;
                    self.layers[i].set = if backwards { *old } else { *new };
                    self.mark_layer_alone(i, None);
                }
                self.mark_clipped_layers();
                Ok(())
            }
        }
    }
}
