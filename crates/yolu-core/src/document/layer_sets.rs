//! レイヤーセット: テクスチャセットの文書の中の、名前の付いたレイヤーの束と、チャンネルごとの出力の割り当て（最終の結果をどのレイヤーセット
//! から取るか）。
//!
//! - レイヤーの並びは文書に 1 本のまま（[`Layer::set`] でどれか 1 つのレイヤーセットに入る）。合成はレイヤーセットごとに、そのレイヤーセットの
//!   レイヤーだけで行い（[`Document::set_stack`]）、チャンネル c の最終の結果は、割り当てたレイヤーセットの c の合成。割り当てが無ければ c の既定値。
//! - 一番上の段の兄弟は、同じレイヤーセットのレイヤーだけ（グループの子は親と同じレイヤーセット）。並べ替え・親子・グループにまとめる・結合は、
//!   同じレイヤーセットの中だけで行い、またぐ要求は断る。
//! - レイヤーセットは扱うチャンネルの組（[`LayerSetInfo::channels`]）を持つ。組に無いチャンネルは、そのレイヤーセットの合成に入らない（計画が空で、
//!   レイヤーの入切もそのチャンネルでは数えない）。出力の割り当ては、そのチャンネルを組に持つレイヤーセットへだけ向けられる。組のビットは文書に在る
//!   チャンネルだけ。
//! - 新しい文書・前のバージョンの文書は、名前の空のレイヤーセット 1 つに全部のチャンネルを割り当てた形（組も全部のチャンネル）で、合成も保存のバイトも
//!   レイヤーセットの無かったときと同じ。

use crate::Reason;
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

/// レイヤーセットの ID・名前（空でもよい。空は画面が言語ごとの既定の名前で見せる）・扱うチャンネルの組。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerSetInfo {
    pub id: LayerSetId,
    pub name: String,
    /// 扱うチャンネルの組（チャンネルの番号のビット。[`Channel::bit`] と同じ並び）。空でもよい（どのチャンネルも合成しない下書き）。
    pub channels: u64,
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
        return Err(CoreError::InvalidArgument(Reason::LayerSetName));
    }
    Ok(())
}

/// `channel` の計画の中に、実際の中身（画素のあるタイル・塗りつぶしの値・フィルターや Generator の出力になるラスターのレイヤー）が在るか。
/// 計画に出るだけの空のラスターの面（新しいレイヤーが持つ）は中身に数えない。
pub(crate) fn plan_holds_content(
    layers: &[Layer],
    plan: &[crate::composite::Entry],
    channel: Channel,
) -> bool {
    plan.iter().any(|e| {
        let l = &layers[e.layer];
        let own = match l.kind() {
            crate::LayerKind::Fill | crate::LayerKind::Pointer => true,
            crate::LayerKind::Raster => {
                !l.filters.is_empty() || l.surface(channel).is_some_and(|s| s.tile_count() > 0)
            }
            _ => false,
        };
        own || plan_holds_content(layers, &e.children, channel)
            || plan_holds_content(layers, &e.clips, channel)
    })
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

    /// レイヤーセットの扱うチャンネルの組（チャンネルの番号のビット。[`Channel::bit`] と同じ並び）。無いレイヤーセットは断る。
    pub fn layer_set_channels(&self, set: LayerSetId) -> Result<u64, CoreError> {
        Ok(self.layer_sets[self.require_layer_set(set)?].channels)
    }

    /// 文書に在るチャンネルのビット。
    pub fn channel_bits(&self) -> u64 {
        self.channels().into_iter().fold(0, |b, c| b | c.bit())
    }

    /// 文書に在る標準のチャンネル（番号が [`Channel::STANDARD_COUNT`] 未満）のビット。新しいレイヤーセットの既定の組。
    pub fn standard_channel_bits(&self) -> u64 {
        self.channels()
            .into_iter()
            .filter(|c| c.is_standard())
            .fold(0, |b, c| b | c.bit())
    }

    /// レイヤーセットの合成にチャンネルが入るか（組にあるか）。無いレイヤーセットは false。
    pub(crate) fn set_has_channel(&self, set: LayerSetId, channel: Channel) -> bool {
        self.layer_sets
            .iter()
            .any(|s| s.id == set && s.channels & channel.bit() != 0)
    }

    /// レイヤーセット `set` のレイヤーが、組に無い `channel` に表示の中身（画素・塗りつぶしの値）を持つか。持てば、そのチャンネルの合成に入らない
    /// （書き出しや結合は、その中身を黙って捨てないよう知らせる）。組にあるチャンネルは false。結合と同じ物差し（そのレイヤーセットの計画に、画素のあるタイル・塗りつぶしの値・効果の出力のどれかを持つレイヤーが在るか。空の面だけのレイヤーは数えない）。
    pub fn layer_set_has_content_outside_channel(&self, set: LayerSetId, channel: Channel) -> bool {
        if self.set_has_channel(set, channel) {
            return false;
        }
        self.channel_kind(channel).is_ok_and(|kind| {
            let stack = crate::composite::Stack::in_set(&self.layers, set, channel, kind, None);
            plan_holds_content(&self.layers, &stack.plan(), channel)
        })
    }

    /// `channel` の最終の結果を取るレイヤーセットに、そのチャンネルの実際の中身（画素のあるタイル・塗りつぶしの値・効果の出力）が在るか。
    /// 割り当てが無ければ false（既定値だけ）。空の面だけのレイヤー（チャンネルを有効にしただけ）は数えない。書き出しが「描いていない
    /// チャンネルの画像を書かない」ための問い（[`crate::export::uses`] は有効の印だけを見る）。
    pub fn channel_output_has_content(&self, channel: Channel) -> bool {
        let Ok(Some(set)) = self.output_set(channel) else {
            return false;
        };
        self.channel_kind(channel).is_ok_and(|kind| {
            let stack = crate::composite::Stack::in_set(&self.layers, set, channel, kind, None);
            plan_holds_content(&self.layers, &stack.plan(), channel)
        })
    }

    /// レイヤーが入っているレイヤーセット。
    pub fn layer_set_of(&self, id: LayerId) -> Result<LayerSetId, CoreError> {
        Ok(self.layers[self.index_of(id)?].set)
    }

    /// レイヤーセットが 1 つで、ID が文書の最初のレイヤーセットの ID（[`LayerSetId::first_of`]）で、名前が空で、組が文書の全部のチャンネルで、全部の
    /// チャンネルをそこへ割り当てた形か（新しい文書・前のバージョンの文書の形。保存はこの形ならレイヤーセットを書かず、開くと同じ ID に戻る。
    /// 最初のレイヤーセットを消して別の ID が 1 つ残った形は、ID を書くためにこの形に入らない）。
    pub fn has_default_layer_sets(&self) -> bool {
        let [only] = self.layer_sets.as_slice() else {
            return false;
        };
        only.id == LayerSetId::first_of(self.id())
            && only.name.is_empty()
            && only.channels == self.channel_bits()
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
            .ok_or(CoreError::InvalidArgument(Reason::LayerSetDoesNotExist))
    }

    /// 新しいレイヤーを入れる所: above のすぐ上（above と同じグループ・レイヤーセットの中）、無ければ最初のレイヤーセットの一番上。
    /// (並びの番号, 親, レイヤーセット)。
    pub(super) fn placement_above(
        &self,
        above: Option<LayerId>,
    ) -> Result<(usize, Option<LayerId>, LayerSetId), CoreError> {
        self.placement_in(above, None)
    }

    /// `placement_above` の、above が無いときの入れ先のレイヤーセットを選べる形（None は最初のレイヤーセット）。above があるときは
    /// above のレイヤーセットで、`set` を渡していれば同じでなければ断る。
    pub(super) fn placement_in(
        &self,
        above: Option<LayerId>,
        set: Option<LayerSetId>,
    ) -> Result<(usize, Option<LayerId>, LayerSetId), CoreError> {
        if let Some(s) = set {
            self.require_layer_set(s)?;
        }
        Ok(match above {
            Some(a) => {
                let i = self.index_of(a)?;
                if set.is_some_and(|s| s != self.layers[i].set) {
                    return Err(CoreError::InvalidArgument(Reason::AboveInOtherLayerSet));
                }
                (i + 1, self.layers[i].parent, self.layers[i].set)
            }
            None => (
                self.layers.len(),
                None,
                set.unwrap_or(self.layer_sets[0].id),
            ),
        })
    }

    /// レイヤーセットの、グループ parent の直下の子（下から上。parent が None なら、そのレイヤーセットの一番上の段）。
    /// `children_of(None)` は全部のレイヤーセットの一番上の段を返すので、レイヤーセットごとに並べ替えの位置を数えるときはこちらを使う。
    /// グループの子はグループと同じレイヤーセットなので、parent が別のレイヤーセットのグループなら空になる。
    pub fn children_in_set(
        &self,
        set: LayerSetId,
        parent: Option<LayerId>,
    ) -> Result<Vec<LayerId>, CoreError> {
        self.require_layer_set(set)?;
        if let Some(p) = parent {
            if !self.layer(p).ok_or(CoreError::LayerNotFound)?.is_group() {
                return Err(CoreError::InvalidArgument(Reason::NotGroup));
            }
        }
        Ok(self
            .layers
            .iter()
            .filter(|l| l.set == set && l.parent == parent)
            .map(|l| l.id)
            .collect())
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

    pub(super) fn table_entry(&self, new: Table) -> Entry {
        let cost = 64
            + new
                .sets
                .iter()
                .chain(self.layer_sets.iter())
                .map(|s| 32 + s.name.len() as u64)
                .sum::<u64>();
        Entry {
            kind: HistoryKind::LayerSets,
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

    /// 段が 1 つならそのまま、2 つ以上なら 1 回の Undo にまとめて当てる。
    pub(super) fn execute_steps(
        &mut self,
        kind: HistoryKind,
        mut steps: Vec<Entry>,
    ) -> Result<(), CoreError> {
        if steps.len() == 1 {
            let only = steps.pop().expect("1 つ");
            return self.execute(only.command, only.cost);
        }
        self.execute_group_as(kind, steps)
    }

    /// チャンネルを足すときの一覧と割り当て（`set` があればその組に入れて割り当てを `set` に、無ければ番号の今の割り当て先の組に入れる）。
    /// 変わらなければ None。
    pub(super) fn table_with_channel(
        &self,
        channel: Channel,
        set: Option<LayerSetId>,
    ) -> Option<Table> {
        let mut table = self.table();
        let target = set.or(table.route[channel.index()])?;
        if let Some(s) = set {
            table.route[channel.index()] = Some(s);
        }
        let info = table.sets.iter_mut().find(|s| s.id == target)?;
        info.channels |= channel.bit();
        (info.channels != self.layer_sets.iter().find(|s| s.id == target)?.channels
            || table.route != self.route)
            .then_some(table)
    }

    /// チャンネルを消すときの一覧（全部のレイヤーセットの組からビットを外す。割り当ては変えない）。変わらなければ None。
    pub(super) fn table_without_channel(&self, channel: Channel) -> Option<Table> {
        let mut table = self.table();
        let mut changed = false;
        for s in &mut table.sets {
            changed |= s.channels & channel.bit() != 0;
            s.channels &= !channel.bit();
        }
        changed.then_some(table)
    }

    /// 読み込み用: 足したチャンネルを、番号の今の割り当て先の組に入れる（履歴なし）。
    pub(super) fn add_channel_to_route_set(&mut self, channel: Channel) {
        if let Some(set) = self.route[channel.index()] {
            if let Some(info) = self.layer_sets.iter_mut().find(|s| s.id == set) {
                info.channels |= channel.bit();
            }
        }
    }

    /// 組が文書に在るチャンネルだけか（無いチャンネルのビットは断る）。
    fn check_channels(&self, channels: u64) -> Result<(), CoreError> {
        if channels & !self.channel_bits() != 0 {
            return Err(CoreError::InvalidArgument(Reason::LayerSetChannels));
        }
        Ok(())
    }

    /// 一覧と割り当ての決まり: 組は文書に在るチャンネルだけ、文書に在るチャンネルの割り当て先はそのチャンネルを組に持つ。
    fn check_table(&self, sets: &[LayerSetInfo], route: &Route) -> Result<(), CoreError> {
        for s in sets {
            self.check_channels(s.channels)?;
        }
        for c in self.channels() {
            if let Some(s) = route[c.index()] {
                if !sets.iter().any(|t| t.id == s && t.channels & c.bit() != 0) {
                    return Err(CoreError::InvalidArgument(Reason::ChannelNotInLayerSet));
                }
            }
        }
        Ok(())
    }

    // ───────── 編集（どれも 1 回の Undo） ─────────

    /// 空のレイヤーセットを一覧の最後に足す。組は文書の標準のチャンネル（[`standard_channel_bits`](Self::standard_channel_bits)）。
    /// 出力の割り当ては変えない。
    pub fn add_layer_set(&mut self, name: &str) -> Result<LayerSetId, CoreError> {
        let channels = self.standard_channel_bits();
        self.add_layer_set_with(name, channels)
    }

    /// 空のレイヤーセットを、扱うチャンネルの組 `channels`（文書に在るチャンネルのビット。空でもよい）で一覧の最後に足す。出力の割り当ては変えない。
    pub fn add_layer_set_with(
        &mut self,
        name: &str,
        channels: u64,
    ) -> Result<LayerSetId, CoreError> {
        self.ensure_no_stroke()?;
        check_name(name)?;
        self.check_channels(channels)?;
        if self.layer_sets.len() >= MAX_LAYER_SETS {
            return Err(CoreError::InvalidArgument(Reason::TooManyLayerSets));
        }
        let id = self.new_layer_set_id();
        let mut table = self.table();
        table.sets.push(LayerSetInfo {
            id,
            name: name.to_owned(),
            channels,
        });
        self.execute_table(table)?;
        Ok(id)
    }

    /// レイヤーセットの扱うチャンネルの組を替える（文書に在るチャンネルのビット。空でもよい）。外したチャンネルの出力の割り当てがこのレイヤーセット
    /// なら、割り当てを外す（None。最終の結果はそのチャンネルの既定値。ほかのレイヤーセットへは移さない）。1 回の Undo。
    pub fn set_layer_set_channels(
        &mut self,
        set: LayerSetId,
        channels: u64,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let at = self.require_layer_set(set)?;
        self.check_channels(channels)?;
        if self.layer_sets[at].channels == channels {
            return Ok(());
        }
        let mut table = self.table();
        table.sets[at].channels = channels;
        for c in self.channels() {
            if channels & c.bit() == 0 && table.route[c.index()] == Some(set) {
                table.route[c.index()] = None;
            }
        }
        self.execute_table(table)
    }

    /// 空のレイヤーを `set` に足す（Color のチャンネルを持ち、Color を有効にする。1 回の Undo）。`above` があればそのすぐ上（`above` と同じグループの中。
    /// `above` は `set` のレイヤー）、無ければ `set` の一番上の段の一番上。空のレイヤーセットへレイヤーを足す口は、これだけ
    /// （`add_layer_above` の `above` を省くと、最初のレイヤーセットへ入る）。
    pub fn add_layer_in_set(
        &mut self,
        name: &str,
        set: LayerSetId,
        above: Option<LayerId>,
    ) -> Result<LayerId, CoreError> {
        self.add_layer_in_set_on(name, set, above, Channel::Color)
    }

    /// [`add_layer_in_set`](Self::add_layer_in_set) の、持つ・有効にするチャンネルを選ぶ形（文書に無いチャンネルは断る）。
    pub fn add_layer_in_set_on(
        &mut self,
        name: &str,
        set: LayerSetId,
        above: Option<LayerId>,
        channel: Channel,
    ) -> Result<LayerId, CoreError> {
        self.ensure_no_stroke()?;
        self.require_channel(channel)?;
        // 足し先を先に確かめる（レイヤーの ID を無駄に振らない）
        self.placement_in(above, Some(set))?;
        let id = self.new_layer_id();
        let mut layer = Layer::new(id, name, crate::types::LayerKind::Raster);
        layer.put_surface(
            channel,
            Some(crate::Surface::new(self.width, self.height, self.tile_size)),
        );
        layer.set_enabled(channel, true);
        self.insert_new_in(layer, above, Some(set), 128)
    }

    /// レイヤーセットを画素ごと写し、元のすぐ後に置く（レイヤー・効果・Anchor・定規の ID は新しく振る。写しの中の Anchor を読む段は写しの
    /// Anchor を読む）。name が None なら同じ名前。タイルは写し元と共有し（書いたときに複製）、画素の予算には全部数え、超えるなら何も変えずに断る。
    /// 扱うチャンネルの組は写す。出力の割り当ては変えない。
    pub fn duplicate_layer_set(
        &mut self,
        set: LayerSetId,
        name: Option<&str>,
    ) -> Result<LayerSetId, CoreError> {
        self.ensure_no_stroke()?;
        let at = self.require_layer_set(set)?;
        let name = name.unwrap_or(&self.layer_sets[at].name).to_owned();
        let channels = self.layer_sets[at].channels;
        check_name(&name)?;
        if self.layer_sets.len() >= MAX_LAYER_SETS {
            return Err(CoreError::InvalidArgument(Reason::TooManyLayerSets));
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
        // 写しの中を指す参照レイヤーは写しを指す
        Self::remap_pointer_targets(&mut copies, &ids);
        let mut table = self.table();
        table
            .sets
            .insert(at + 1, LayerSetInfo { id, name, channels });
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
        self.execute_group_as(HistoryKind::LayerSets, steps)?;
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
            return Err(CoreError::InvalidArgument(Reason::LayerSetPosition));
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
            return Err(CoreError::InvalidArgument(
                Reason::LastLayerSetCannotBeRemoved,
            ));
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
        self.execute_group_as(HistoryKind::LayerSets, steps)
    }

    /// チャンネルの出力の割り当てを変える（None はそのチャンネルの既定値）。最終の結果がそのチャンネルのキャンバス全体で変わる。`set` がそのチャンネルを
    /// 組に持たなければ断る。
    pub fn set_channel_output(
        &mut self,
        channel: Channel,
        set: Option<LayerSetId>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        self.require_channel(channel)?;
        if let Some(s) = set {
            self.require_layer_set(s)?;
            if !self.set_has_channel(s, channel) {
                return Err(CoreError::InvalidArgument(Reason::ChannelNotInLayerSet));
            }
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

    // ───────── ほかの文書のレイヤーを入れる ─────────

    /// ほかの文書（PSD を読んだ文書など）のレイヤーを全部（グループ・マスク・合成の方法・ロック・クリッピングごと）写し、レイヤーセット `set` の
    /// 一番上の段の一番上へ入れる（1 回の Undo）。入れたレイヤーの ID（下から上）を返す。並びと親子は `source` のまま、レイヤー・効果・Anchor・
    /// 定規の ID は新しく振る。チャンネルは番号のまま入る（Color は Color。ユーザーチャンネルは、同じ番号に同じ情報のチャンネルが無ければ断る）。
    /// `source` のレイヤーセットの分けは持たない（全部 `set` に入る）。大きさがキャンバスと違えば断り（大きさを変えて入れない）、レイヤーの数・
    /// 入れ子・画素の予算を超えるなら何も変えずに断る。`set` に一番上の段のレイヤーがあるときは、入れる一番上の段の一番下のレイヤーの
    /// クリッピングの印（`source` では下に何も無く、下地として合成される）を外す（入れた先の今のレイヤーへ、黙ってクリッピングしない）。
    pub fn insert_layers_from(
        &mut self,
        source: &Document,
        set: LayerSetId,
    ) -> Result<Vec<LayerId>, CoreError> {
        self.ensure_no_stroke()?;
        self.require_layer_set(set)?;
        let occupied = self
            .layers
            .iter()
            .any(|l| l.set == set && l.parent.is_none());
        let copies = self.layers_from(source, set, occupied)?;
        let ids: Vec<LayerId> = copies.iter().map(|l| l.id).collect();
        if copies.is_empty() {
            return Ok(ids);
        }
        let cost = 128 + copies.iter().map(Layer::allocated_bytes).sum::<u64>();
        self.execute(
            Command::Insert {
                index: self.layers.len(),
                len: copies.len(),
                block: Some(copies),
            },
            cost,
        )?;
        Ok(ids)
    }

    /// [`insert_layers_from`](Self::insert_layers_from) の、入れ先を新しいレイヤーセット（名前 `name`、一覧の最後）にする形。レイヤーセットを
    /// 足すのとレイヤーを入れるのは 1 回の Undo。新しいレイヤーセットの組は文書の標準のチャンネル。出力の割り当ては変えない。新しいレイヤーセットと、
    /// 入れたレイヤーの ID を返す。
    pub fn insert_layers_as_layer_set(
        &mut self,
        source: &Document,
        name: &str,
    ) -> Result<(LayerSetId, Vec<LayerId>), CoreError> {
        self.ensure_no_stroke()?;
        check_name(name)?;
        if self.layer_sets.len() >= MAX_LAYER_SETS {
            return Err(CoreError::InvalidArgument(Reason::TooManyLayerSets));
        }
        let id = self.new_layer_set_id();
        let copies = self.layers_from(source, id, false)?;
        let ids: Vec<LayerId> = copies.iter().map(|l| l.id).collect();
        let mut table = self.table();
        table.sets.push(LayerSetInfo {
            id,
            name: name.to_owned(),
            channels: self.standard_channel_bits(),
        });
        let mut steps = vec![self.table_entry(table)];
        if !copies.is_empty() {
            let cost = 128 + copies.iter().map(Layer::allocated_bytes).sum::<u64>();
            steps.push(Entry {
                kind: HistoryKind::AddLayer,
                command: Command::Insert {
                    index: self.layers.len(),
                    len: copies.len(),
                    block: Some(copies),
                },
                cost,
            });
        }
        self.execute_group_as(HistoryKind::LayerSets, steps)?;
        Ok((id, ids))
    }

    /// `source` のレイヤーの、この文書の `set` へ入れる写し（まだ入れていない）。断る理由はここで全部確かめる。`occupied` は、入れ先の一番上の段に
    /// もうレイヤーがあること（一番上の段の一番下の、効いていないクリッピングの印を外す）。
    fn layers_from(
        &mut self,
        source: &Document,
        set: LayerSetId,
        occupied: bool,
    ) -> Result<Vec<Layer>, CoreError> {
        if source.width != self.width || source.height != self.height {
            return Err(CoreError::InvalidArgument(
                Reason::LayersSizeDiffersFromCanvas,
            ));
        }
        // ユーザーチャンネルは番号だけで別の意味へ結び付けない（スマートマテリアルを置くときと同じ）
        for i in Channel::STANDARD_COUNT..Channel::MAX {
            let channel = Channel::from_index(i).expect("チャンネル番号");
            if !source.layers.iter().any(|l| l.has_channel_data(channel)) {
                continue;
            }
            let info = source.channel_info(channel);
            if info.is_none() || self.channel_info(channel) != info {
                return Err(CoreError::Unsupported(
                    Reason::UserChannelMappingDoesNotMatch,
                ));
            }
        }
        if self.layers.len() + source.layers.len() > 2048 {
            return Err(CoreError::InvalidArgument(Reason::TooManyLayers));
        }
        self.ensure_nesting_room(None, super::structure::group_chain_height(&source.layers))?;
        let mut copies = source.layers.clone();
        // タイルの大きさが違う文書からは、同じ画素のままタイルを切り直す（大きさは同じなので値は変わらない）
        if source.tile_size != self.tile_size {
            let mut remaining = self.source_budget.saturating_sub(self.allocated_bytes());
            let mut retile =
                |s: &crate::Surface, normal: bool| -> Result<crate::Surface, CoreError> {
                    let out = super::smart_resample::surface(
                        s,
                        self.width,
                        self.height,
                        self.tile_size,
                        crate::smart::SmartResampling::Nearest,
                        normal,
                        remaining,
                    )?;
                    remaining = remaining
                        .checked_sub(out.allocated_bytes())
                        .ok_or(CoreError::SourceBudgetExceeded)?;
                    Ok(out)
                };
            for layer in &mut copies {
                for (i, s) in layer.surfaces.iter_mut().enumerate() {
                    if let Some(surface) = s {
                        let normal = source
                            .channels
                            .get(i)
                            .and_then(Option::as_ref)
                            .is_some_and(|c| c.kind == crate::ChannelKind::Normal);
                        *surface = retile(surface, normal)?;
                    }
                }
                if let Some(mask) = &mut layer.mask {
                    mask.surface = retile(&mask.surface, false)?;
                }
            }
        }
        let bytes: u64 = copies.iter().map(Layer::allocated_bytes).sum();
        self.ensure_source_growth(bytes)?;
        let mut ids: HashMap<LayerId, LayerId> = HashMap::with_capacity(copies.len());
        for l in &copies {
            let new = self.new_layer_id();
            ids.insert(l.id, new);
        }
        for l in &mut copies {
            l.id = ids[&l.id];
            l.parent = l.parent.map(|p| ids[&p]);
            l.set = set;
        }
        self.renew_effect_ids(&mut copies);
        Self::remap_pointer_targets(&mut copies, &ids);
        self.check_layer_effects(&copies)?;
        // 元の一番上の段の一番下は、印があっても下地（何にもクリッピングされず、上の印のレイヤーはそれにクリッピングされる）
        if occupied {
            if let Some(l) = copies.iter_mut().find(|l| l.parent.is_none()) {
                l.clipping = false;
            }
        }
        Ok(copies)
    }

    // ───────── 書き出し ─────────

    /// 書き出し用の写し（履歴なし、タイルは共有）: レイヤーセットごとのレイヤーを、一番上の段のグループ 1 つずつに包み、レイヤーセット 1 つにする。
    /// グループは分離（Normal）で、中身はそのレイヤーセットの合成と同じになる。並びはレイヤーセットの並びの最初が一番上。名前は `names[i]`、表示は
    /// `visible[i]`（`i` はレイヤーセットの並びの番号）。出力の割り当てのあるチャンネルは、写しの 1 つのレイヤーセット（組は文書の全部のチャンネル）へ
    /// 割り当てる。書き出すチャンネル `channel` を組に持たないレイヤーセットは、中身を入れない空のグループにする（そのレイヤーセットの `channel` の
    /// 合成は空なので、グループの合成も同じ）。グループを足して入れ子が上限を超えるなら断る。
    pub fn layer_sets_as_groups(
        &self,
        names: &[String],
        visible: &[bool],
        channel: Channel,
    ) -> Result<Document, CoreError> {
        if names.len() != self.layer_sets.len() || visible.len() != self.layer_sets.len() {
            return Err(CoreError::InvalidArgument(
                Reason::LayerSetNamesAndVisibilityMismatch,
            ));
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
            let has = info.channels & channel.bit() != 0;
            for l in self.layers.iter().filter(|l| has && l.set == info.id) {
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
        copy.layer_sets[0].channels = copy.channel_bits();
        for r in copy.route.iter_mut() {
            *r = r.map(|_| only);
        }
        copy.validate_structure()?;
        copy.external_mutation();
        Ok(copy)
    }

    // ───────── 読み込み ─────────

    /// 読み込み用: レイヤーセットの一覧（扱うチャンネルの組を含む）・出力の割り当て・レイヤーの所属を置く（履歴を消す）。`members` はレイヤーの並びの順の所属。
    /// 割り当ての無いチャンネル（`route` に挙げないチャンネル）は最初のレイヤーセットへ。一覧が空・ID が 0 か重なる・在らないレイヤーセットを
    /// 指す・グループの子が親と違うレイヤーセット・文書に無いチャンネル・同じチャンネルを 2 度挙げる・組に文書に無いチャンネルがある・割り当て先が
    /// そのチャンネルを組に持たない、のどれかなら断って何も変えない。組の欄の無い前のバージョンは、組を [`channel_bits`](Self::channel_bits) にして渡す。
    pub fn set_layer_sets_for_load(
        &mut self,
        sets: Vec<LayerSetInfo>,
        route: &[(Channel, Option<LayerSetId>)],
        members: &[LayerSetId],
    ) -> Result<(), CoreError> {
        self.ensure_loadable()?;
        if sets.is_empty() || sets.len() > MAX_LAYER_SETS {
            return Err(CoreError::InvalidArgument(Reason::LayerSetCount));
        }
        for (i, s) in sets.iter().enumerate() {
            check_name(&s.name)?;
            if s.id.0 == 0 || sets[..i].iter().any(|t| t.id == s.id) {
                return Err(CoreError::InvalidArgument(
                    Reason::LayerSetIdEmptyOrRepeated,
                ));
            }
        }
        let known = |id: LayerSetId| sets.iter().any(|s| s.id == id);
        if members.len() != self.layers.len() || !members.iter().all(|m| known(*m)) {
            return Err(CoreError::InvalidArgument(Reason::LayerSetOfLayer));
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
                        Reason::GroupChildInOtherLayerSet,
                    ));
                }
            }
        }
        let mut table: Route = [Some(sets[0].id); Channel::MAX];
        let mut seen = 0u64;
        for &(c, s) in route {
            self.require_channel(c)?;
            if seen & c.bit() != 0 || s.is_some_and(|s| !known(s)) {
                return Err(CoreError::InvalidArgument(Reason::OutputAssignment));
            }
            seen |= c.bit();
            table[c.index()] = s;
        }
        self.check_table(&sets, &table)?;
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

    // ───────── 変化の記録 ─────────

    /// チャンネルの出力の割り当てが `from` から `to` へ変わったときの、最終の結果の変化の印。どちらかが無し（既定値で塗る）なら、そのチャンネルの
    /// キャンバス全体（既定値とレイヤーセットの合成は、レイヤーの無い所でも違い得る）。どちらもレイヤーセットなら、その 2 つのレイヤーセットの
    /// レイヤーが持つタイルだけ（レイヤーの表示を切り替えたときと同じ印。レイヤーの無い所は、どちらの合成も透明なので変わらない）。
    fn mark_route_change(
        &mut self,
        channel: Channel,
        from: Option<LayerSetId>,
        to: Option<LayerSetId>,
    ) {
        let (Some(a), Some(b)) = (from, to) else {
            let tiles: Vec<_> = self.canvas_tiles().collect();
            for coord in tiles {
                self.journal.mark(channel, coord);
            }
            return;
        };
        for i in 0..self.layers.len() {
            let l = &self.layers[i];
            if l.parent.is_none() && (l.set == a || l.set == b) {
                self.mark_layer(i, Some(channel));
            }
        }
    }

    /// レイヤーセットの組が変わったときの印: 組に入った・外れたチャンネルごとに、そのレイヤーセットのレイヤーが持つタイル（そのレイヤーセットの
    /// 合成が、レイヤーの合成と透明の間で変わる所。レイヤーの表示を切り替えたときと同じ印）。
    fn mark_channels_change(&mut self, before: &[LayerSetInfo]) {
        let changed: Vec<(LayerSetId, u64)> = self
            .layer_sets
            .iter()
            .filter_map(|s| {
                let old = before.iter().find(|b| b.id == s.id)?.channels;
                (old != s.channels).then_some((s.id, old ^ s.channels))
            })
            .collect();
        for (set, bits) in changed {
            for i in 0..self.layers.len() {
                let l = &self.layers[i];
                if l.parent.is_some() || l.set != set {
                    continue;
                }
                for c in (0..Channel::MAX).filter(|c| bits & (1u64 << c) != 0) {
                    let channel = Channel::from_index(c).expect("64 未満");
                    self.mark_layer(i, Some(channel));
                }
            }
        }
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
                let changed: Vec<(Channel, Option<LayerSetId>, Option<LayerSetId>)> = (0
                    ..Channel::MAX)
                    .filter(|&i| self.route[i] != to.route[i])
                    .filter_map(|i| Some((Channel::from_index(i)?, self.route[i], to.route[i])))
                    .collect();
                let before = std::mem::replace(&mut self.layer_sets, to.sets.clone());
                self.route = to.route;
                for (c, from, to) in changed {
                    self.mark_route_change(c, from, to);
                }
                self.mark_channels_change(&before);
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
