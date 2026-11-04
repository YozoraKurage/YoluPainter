//! 参照レイヤー（[`crate::pointer`]）の文書の側: 作る・指す先と読む元を替える操作（どれも 1 回の取り消し）、循環と深さの解析、評価の鍵
//! （指した先の変化の印）、効いていない理由、テクスチャセットをまたぐ判定のためのアプリへの口。
//!
//! 解析は、レイヤーを節、「そのレイヤーの出力を評価するときに出力を読むレイヤー」を辺にした図を、参照レイヤーから辿る。辺は、参照レイヤーの
//! 指す先（レイヤー・グループの部分木、レイヤーセットの全部のレイヤー）と、フィルター・マスクのフィルターの Anchor の段が読む合成（Anchor の
//! レイヤーと、同じレイヤーセットでそれより下のレイヤー。マスクの Anchor はそのレイヤー）。輪にある・輪へ届く参照レイヤーは循環、参照レイヤーを
//! 数えた一番長い道が [`MAX_POINTER_DEPTH`] を超えるものは深すぎる。どちらも評価では読まずに既定値を出し（読み合って終わらないことが無い）、
//! 編集（[`Document`] の段を積む所）と読み込みは、新しく生じるなら断る。

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::{Command, Document};
use crate::effects::{EffectSettings, InactiveEffect, InactiveReason, InactiveTarget};
use crate::error::CoreError;
use crate::generator::{self, anchor};
use crate::layer::{Layer, LayerId, LayerSetId};
use crate::pointer::{
    ExternalReach, ExternalSet, PointerIssue, PointerReach, PointerSettings, PointerTarget,
    MAX_POINTER_DEPTH,
};
use crate::types::{Channel, LayerKind};
use crate::Reason;

/// 参照レイヤー 1 枚の解決（レイヤー全体の理由。チャンネルごとの理由は評価のときに見る）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PointerState {
    /// 読める（`depth` は自分を含めた参照の連なりの段の数）。
    Ready { depth: usize },
    /// 読まない（既定値を出す）。
    Issue(PointerIssue),
}

/// 解析を作り直すかの鍵（どれかが変われば図か署名が変わり得る）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct AnalysisKey {
    journal: u64,
    clock: u64,
    revision: u64,
    generation: u64,
    inputs: u64,
    topology: u64,
    pointers: u64,
    layers: usize,
    /// レイヤーセットの ID の並びの hash（空のレイヤーセットを消す・戻す段は、他の鍵を動かさずに指す先の有無だけを変える）。
    sets: u64,
}

/// 文書の参照レイヤーの解析（評価のキャッシュと同じく派生で、保存も Undo もしない）。
pub(crate) struct PointerAnalysis {
    key: AnalysisKey,
    pub(crate) states: HashMap<LayerId, PointerState>,
    /// 読める参照レイヤー（同じ文書の指す先）の、指した先の変化の印。指す先の部分木・レイヤーセットと、そこから辿れる全部のレイヤーの
    /// 変化の時計・並び・入れ子から作る。関係ないレイヤーの変化では変わらない。
    pub(crate) signatures: HashMap<LayerId, u64>,
    /// 読める参照レイヤー（同じ文書の指す先）が、評価のときに出力を読むレイヤー（入れ子の参照レイヤー・Anchor の段が読む合成も辿った全部）。
    /// 描いている間の下の覚えを使ってよいかを決める（描いているレイヤーがここにあれば、参照レイヤーの出力が描くと変わる）。
    pub(crate) reached: HashSet<LayerId>,
}

/// 辺（そのレイヤーの出力が読むレイヤー）の表。図は解析のたびに作る。
struct Reads<'d> {
    doc: &'d Document,
    index: HashMap<LayerId, usize>,
    by_set: HashMap<LayerSetId, Vec<usize>>,
    points: Vec<anchor::Point>,
    memo: Vec<Option<Arc<[usize]>>>,
    /// 確かめるだけの差し替え: この番号の参照レイヤーは、指す先をこれにしたものとして辿る（[`Document::pointer_target_issue`]）。
    swap: Option<(usize, PointerTarget)>,
}

impl<'d> Reads<'d> {
    fn new(doc: &'d Document) -> Self {
        let mut by_set: HashMap<LayerSetId, Vec<usize>> = HashMap::new();
        for (i, l) in doc.layers.iter().enumerate() {
            by_set.entry(l.set).or_default().push(i);
        }
        Reads {
            doc,
            index: doc
                .layers
                .iter()
                .enumerate()
                .map(|(i, l)| (l.id, i))
                .collect(),
            by_set,
            points: doc.anchor_points(),
            memo: vec![None; doc.layers.len()],
            swap: None,
        }
    }

    /// 参照レイヤー（番号 index）の指す先を target にしたものとして辿る形。
    fn swapped(doc: &'d Document, index: usize, target: PointerTarget) -> Self {
        let mut reads = Reads::new(doc);
        reads.swap = Some((index, target));
        reads
    }

    /// 参照レイヤー i の指す先（差し替えがあればそれ）。
    fn target_of(&self, i: usize) -> Option<PointerTarget> {
        match self.swap {
            Some((k, t)) if k == i => Some(t),
            _ => self.doc.layers[i].pointer.as_ref().map(|p| p.target),
        }
    }

    /// レイヤー・グループ top の部分木（グループの中身はすぐ下に続けて並ぶ）。
    fn subtree(&self, top: usize) -> std::ops::RangeInclusive<usize> {
        let mut start = top;
        let l = &self.doc.layers[top];
        if l.is_group() {
            while start > 0 && self.doc.is_descendant(start - 1, l.id) {
                start -= 1;
            }
        }
        start..=top
    }

    /// 指す先が読むレイヤー（同じ文書の指す先が無ければ None。ほかのテクスチャセットは空）。
    fn target_members(&self, target: PointerTarget) -> Option<Vec<usize>> {
        match target {
            PointerTarget::Layer(id) => self.index.get(&id).map(|&t| self.subtree(t).collect()),
            PointerTarget::LayerSet(s) => {
                if !self.doc.layer_sets.iter().any(|i| i.id == s) {
                    return None;
                }
                Some(self.by_set.get(&s).cloned().unwrap_or_default())
            }
            PointerTarget::External(_) => Some(Vec::new()),
        }
    }

    /// レイヤー i の出力を評価するときに出力を読むレイヤー（自分の指す先と、Anchor の段の読む合成）。
    fn of(&mut self, i: usize) -> Arc<[usize]> {
        if let Some(v) = &self.memo[i] {
            return v.clone();
        }
        let l = &self.doc.layers[i];
        let mut out: Vec<usize> = Vec::new();
        if let Some(t) = self.target_of(i) {
            if let Some(m) = self.target_members(t) {
                out.extend(m);
            }
        }
        let stacks = l
            .filters
            .iter()
            .chain(l.mask.iter().flat_map(|m| m.filters.iter()));
        for e in stacks {
            let EffectSettings::Generator(g) = &e.settings else {
                continue;
            };
            if g.kind != generator::Kind::Anchor || !e.is_active() {
                continue;
            }
            let Ok(p) = self.doc.resolve_anchor(&self.points, g.anchor.id, i) else {
                continue;
            };
            match p.placement {
                anchor::Placement::Layer => {
                    let set = self.doc.layers[p.host].set;
                    if let Some(members) = self.by_set.get(&set) {
                        out.extend(members.iter().copied().filter(|&k| k <= p.host));
                    }
                }
                anchor::Placement::Mask => out.push(p.host),
            }
        }
        out.sort_unstable();
        out.dedup();
        let v: Arc<[usize]> = out.into();
        self.memo[i] = Some(v.clone());
        v
    }
}

/// 図の上の、根から辿った各節の段の数（参照レイヤーを数えた一番長い道。輪にある・輪へ届く節は None）。
struct Depths {
    /// 節ごとの、そこから先の一番長い道の段の数（自分を含む。辿っていない節・輪の節は None）。
    depth: Vec<Option<usize>>,
    /// 辿った節（後順）。
    order: Vec<usize>,
}

impl Document {
    /// 文書に参照レイヤーがあるか。
    pub(crate) fn has_pointers(&self) -> bool {
        self.layers.iter().any(|l| l.kind == LayerKind::Pointer)
    }

    /// 根から辿り、節ごとの先の段の数を求める（後順の深さ優先。輪は辺の先が辿っている途中の節であることで見つけ、途中の節を全部輪にする）。
    fn depths_from(&self, reads: &mut Reads<'_>, roots: &[usize]) -> Depths {
        let n = self.layers.len();
        let own = |i: usize| usize::from(self.layers[i].kind == LayerKind::Pointer);
        let mut color = vec![0u8; n];
        let mut on_stack = vec![usize::MAX; n];
        let mut cyclic = vec![false; n];
        let mut depth: Vec<Option<usize>> = vec![None; n];
        let mut order = Vec::new();
        for &root in roots {
            if color[root] != 0 {
                continue;
            }
            let mut stack: Vec<(usize, Arc<[usize]>, usize)> = Vec::new();
            color[root] = 1;
            on_stack[root] = 0;
            stack.push((root, reads.of(root), 0));
            while let Some(top) = stack.last_mut() {
                let (v, succ, k) = (top.0, top.1.clone(), top.2);
                if k < succ.len() {
                    top.2 += 1;
                    let s = succ[k];
                    match color[s] {
                        0 => {
                            color[s] = 1;
                            on_stack[s] = stack.len();
                            stack.push((s, reads.of(s), 0));
                        }
                        1 => {
                            for frame in &stack[on_stack[s]..] {
                                cyclic[frame.0] = true;
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                stack.pop();
                color[v] = 2;
                on_stack[v] = usize::MAX;
                let mut best = Some(0usize);
                if cyclic[v] {
                    best = None;
                } else {
                    for &s in succ.iter() {
                        match (best, depth[s]) {
                            (Some(b), Some(d)) => best = Some(b.max(d)),
                            _ => best = None,
                        }
                    }
                }
                depth[v] = best.map(|b| b + own(v));
                order.push(v);
            }
        }
        Depths { depth, order }
    }

    /// 参照レイヤーの解析（キャッシュにあって鍵が合えばそれ）。
    pub(crate) fn pointer_analysis(&self) -> Arc<PointerAnalysis> {
        let key = AnalysisKey {
            journal: self.journal.serial,
            clock: self.effects.clock,
            revision: self.revision,
            generation: self.effects.generation,
            inputs: self.effects.inputs_revision,
            topology: self.effects.topology_revision,
            pointers: self.effects.pointer_inputs_revision,
            layers: self.layers.len(),
            sets: {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                self.layer_sets.iter().for_each(|s| s.id.hash(&mut h));
                h.finish()
            },
        };
        {
            let cached = self
                .effects
                .pointer_analysis
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if let Some(a) = cached.as_ref().filter(|a| a.key == key) {
                return a.clone();
            }
        }
        let analysis = Arc::new(self.analyse_pointers(key));
        *self
            .effects
            .pointer_analysis
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(analysis.clone());
        analysis
    }

    fn analyse_pointers(&self, key: AnalysisKey) -> PointerAnalysis {
        let mut states = HashMap::new();
        let mut signatures = HashMap::new();
        let mut reached = HashSet::new();
        let pointers: Vec<usize> = (0..self.layers.len())
            .filter(|&i| self.layers[i].kind == LayerKind::Pointer)
            .collect();
        if pointers.is_empty() {
            return PointerAnalysis {
                key,
                states,
                signatures,
                reached,
            };
        }
        let mut reads = Reads::new(self);
        let depths = self.depths_from(&mut reads, &pointers);
        for &p in &pointers {
            let l = &self.layers[p];
            let target = l.pointer.as_ref().map(|s| s.target);
            let missing = match target {
                None => true,
                Some(t) => reads.target_members(t).is_none(),
            };
            let state = match depths.depth[p] {
                None => PointerState::Issue(PointerIssue::Cycle),
                Some(d) if d > MAX_POINTER_DEPTH => PointerState::Issue(PointerIssue::TooDeep),
                Some(_) if missing => PointerState::Issue(PointerIssue::MissingTarget),
                Some(d) => PointerState::Ready { depth: d },
            };
            states.insert(l.id, state);
            if let (PointerState::Ready { .. }, Some(t)) = (state, target) {
                if !matches!(t, PointerTarget::External(_)) {
                    let members = reads.target_members(t).unwrap_or_default();
                    let all = Self::read_closure(&mut reads, &members);
                    reached.extend(all.iter().map(|&i| self.layers[i].id));
                    signatures.insert(l.id, self.target_signature(&all));
                }
            }
        }
        PointerAnalysis {
            key,
            states,
            signatures,
            reached,
        }
    }

    /// 指す先（members）から、出力を読むレイヤーを辿った全部（members を含む。番号の昇順）。
    fn read_closure(reads: &mut Reads<'_>, members: &[usize]) -> Vec<usize> {
        let mut seen: HashSet<usize> = HashSet::new();
        let mut todo: Vec<usize> = members.to_vec();
        while let Some(v) = todo.pop() {
            if !seen.insert(v) {
                continue;
            }
            todo.extend(reads.of(v).iter().copied());
        }
        let mut all: Vec<usize> = seen.into_iter().collect();
        all.sort_unstable();
        all
    }

    /// 指す先から辿れる全部のレイヤー（`all`）の、変化の印。並び・入れ子・レイヤーセット・変化の時計と、辿った先の効果が読む入力・
    /// ほかのテクスチャセットの入力の通し番号・チャンネルの種類・レイヤーセットの組から作る。
    fn target_signature(&self, all: &[usize]) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut effects = false;
        for &i in all {
            let l = &self.layers[i];
            (i, l.id, l.parent, l.set).hash(&mut h);
            self.effects.layer_serials.get(&l.id).hash(&mut h);
            effects |= !l.filters.is_empty()
                || l.mask.as_ref().is_some_and(|m| !m.filters.is_empty())
                || l.reads_inputs_for_fill();
            if let Some(PointerTarget::External(key)) = l.pointer.as_ref().map(|p| p.target) {
                for ((k, c), s) in &self.effects.pointer_serials {
                    if *k == key {
                        (c, s).hash(&mut h);
                    }
                }
                for (k, c) in &self.effects.pointer_over {
                    if *k == key {
                        (c, 1u8).hash(&mut h);
                    }
                }
            }
        }
        if effects {
            (
                self.effects.inputs_revision,
                self.effects.topology_revision,
                self.filter_seams,
            )
                .hash(&mut h);
        }
        for (i, c) in self.channels.iter().enumerate() {
            if let Some(info) = c {
                (i, info.kind, info.default.to_array()).hash(&mut h);
            }
        }
        for s in &self.layer_sets {
            (s.id, s.channels).hash(&mut h);
        }
        self.effects.generation.hash(&mut h);
        h.finish()
    }

    /// レイヤー `id` を、読める参照レイヤーのどれかが評価のときに読むか（描いている間の下の覚えを使ってよいかの判定）。
    pub(super) fn pointer_reads_layer(&self, id: LayerId) -> bool {
        self.has_pointers() && self.pointer_analysis().reached.contains(&id)
    }

    /// 参照レイヤー（番号）のレイヤー全体の解決。参照レイヤーでなければ None。
    pub(crate) fn pointer_state(&self, index: usize) -> Option<PointerState> {
        let l = &self.layers[index];
        if l.kind != LayerKind::Pointer {
            return None;
        }
        Some(
            self.pointer_analysis()
                .states
                .get(&l.id)
                .copied()
                .unwrap_or(PointerState::Issue(PointerIssue::MissingTarget)),
        )
    }

    /// 参照レイヤー（番号）のチャンネル `channel` が読めない理由（読めるなら None）。レイヤー全体の理由が先。
    pub(crate) fn pointer_channel_issue(
        &self,
        index: usize,
        channel: Channel,
    ) -> Option<PointerIssue> {
        match self.pointer_state(index)? {
            PointerState::Issue(i) => return Some(i),
            PointerState::Ready { .. } => {}
        }
        let settings = self.layers[index].pointer.as_ref()?;
        let source = settings.source(channel);
        match settings.target {
            PointerTarget::External(key) => {
                if self.effects.pointer_over.contains(&(key, source)) {
                    Some(PointerIssue::InputOverBudget)
                } else if self.effects.inputs.pointer(key, source).is_none() {
                    Some(PointerIssue::MissingInput)
                } else {
                    None
                }
            }
            _ => self
                .channel_info(source)
                .is_none()
                .then_some(PointerIssue::MissingChannel(source)),
        }
    }

    /// 参照レイヤー（番号）の、同じ文書の指す先の変化の印（読めないなら 0）。
    pub(crate) fn pointer_target_signature(&self, index: usize) -> u64 {
        self.pointer_analysis()
            .signatures
            .get(&self.layers[index].id)
            .copied()
            .unwrap_or(0)
    }

    /// 循環・深すぎる参照レイヤー（編集の前後を比べる）。参照レイヤーが無ければ空。
    pub(super) fn pointer_violations(&self) -> HashSet<LayerId> {
        if !self.has_pointers() {
            return HashSet::new();
        }
        self.pointer_analysis()
            .states
            .iter()
            .filter(|(_, s)| {
                matches!(
                    s,
                    PointerState::Issue(PointerIssue::Cycle | PointerIssue::TooDeep)
                )
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// 編集の後に、前に無かった循環・深すぎる参照レイヤーがあれば、その理由（循環が先）。
    pub(super) fn new_pointer_violation(&self, before: &HashSet<LayerId>) -> Option<Reason> {
        if !self.has_pointers() {
            return None;
        }
        let analysis = self.pointer_analysis();
        let mut deep = false;
        for (id, s) in &analysis.states {
            if before.contains(id) {
                continue;
            }
            match s {
                PointerState::Issue(PointerIssue::Cycle) => return Some(Reason::PointerCycle),
                PointerState::Issue(PointerIssue::TooDeep) => deep = true,
                _ => {}
            }
        }
        deep.then_some(Reason::PointerTooDeep)
    }

    /// 段が参照の図（並び・入れ子・レイヤーセット・指す先・Anchor の段）を変え得るか。
    pub(super) fn may_change_pointers(command: &Command) -> bool {
        match command {
            Command::Insert { .. }
            | Command::Remove { .. }
            | Command::Structure { .. }
            | Command::Stack { .. }
            | Command::Anchor { .. }
            | Command::AddMask { .. }
            | Command::RemoveMask { .. }
            | Command::SwapSmartMask { .. }
            | Command::Swap(_)
            | Command::LayerSets(_)
            | Command::Pointer { .. } => true,
            Command::Compound(steps) => steps.iter().any(|e| Self::may_change_pointers(&e.command)),
            _ => false,
        }
    }

    /// 編集（Undo・Redo を含む）の後に: 参照レイヤーの解決（読めるか・理由・深さ）が変わっていれば、その参照レイヤーを全部変わったことにする
    /// （指した先を消した・戻した・循環になった。読めない参照レイヤーは既定値でキャンバス全体を覆うので、変わる所は指す先のタイルに限らない）。
    pub(super) fn refresh_pointer_readers(&mut self) {
        if !self.has_pointers() && self.effects.pointer_signature.is_empty() {
            return;
        }
        let analysis = self.pointer_analysis();
        let mut now: Vec<(LayerId, PointerState)> =
            analysis.states.iter().map(|(id, s)| (*id, *s)).collect();
        now.sort_by_key(|(id, _)| *id);
        if now == self.effects.pointer_signature {
            return;
        }
        let changed: Vec<LayerId> = now
            .iter()
            .filter(|e| !self.effects.pointer_signature.contains(e))
            .map(|(id, _)| *id)
            .collect();
        self.effects.pointer_signature = now;
        for id in changed {
            if let Some(i) = self.layer_index(id) {
                self.mark_layer(i, None);
            }
        }
    }

    /// チャンネル `channel` を読む元にしている参照レイヤー（同じ文書の指す先）の、そのチャンネルを読むチャンネルに印を付ける（読む元の
    /// チャンネルを消した・戻した・種類を変えたとき。読む元の側の印だけでは、読む側のタイルが変わった印にならない）。
    pub(super) fn mark_pointer_readers_of(&mut self, channel: Channel) {
        for i in 0..self.layers.len() {
            let Some(settings) = self.layers[i].pointer.as_ref() else {
                continue;
            };
            if matches!(settings.target, PointerTarget::External(_)) {
                continue;
            }
            let outs: Vec<Channel> = settings
                .sources()
                .filter(|&(_, s)| s == channel)
                .map(|(c, _)| c)
                .collect();
            for c in outs {
                self.mark_layer(i, Some(c));
            }
        }
    }

    // ───────── 操作 ─────────

    /// 参照レイヤー（番号）。参照レイヤーでなければ断る。
    fn pointer_layer_index(&self, id: LayerId) -> Result<usize, CoreError> {
        let index = self.index_of(id)?;
        if self.layers[index].kind != LayerKind::Pointer {
            return Err(CoreError::Unsupported(Reason::NotPointerLayer));
        }
        Ok(index)
    }

    /// 同じ文書の指す先があるか（レイヤーセット・レイヤー。ほかのテクスチャセットは確かめない）。
    fn require_pointer_target(&self, target: PointerTarget) -> Result<(), CoreError> {
        match target {
            PointerTarget::Layer(id) => self.index_of(id).map(|_| ()),
            PointerTarget::LayerSet(s) => self.require_layer_set(s).map(|_| ()),
            PointerTarget::External(_) => Ok(()),
        }
    }

    /// 参照レイヤーを above のすぐ上（同じグループ・レイヤーセットの中。None は最初のレイヤーセットの一番上）に足す。文書の全部のチャンネルを
    /// 有効にし、どのチャンネルも自分と同じチャンネルを読む。同じ文書の指す先が無い・循環になる（自分の入る所が指す先の中）・参照の連なりが
    /// 上限を超えるなら断り、何も変えない。1 回の Undo。
    pub fn add_pointer_layer(
        &mut self,
        name: &str,
        target: PointerTarget,
        above: Option<LayerId>,
    ) -> Result<LayerId, CoreError> {
        self.ensure_no_stroke()?;
        self.require_pointer_target(target)?;
        let id = self.new_layer_id();
        let mut layer = Layer::new(id, name, LayerKind::Pointer);
        layer.pointer = Some(PointerSettings::new(target));
        for c in self.channels() {
            layer.set_enabled(c, true);
        }
        self.insert_new(layer, above)
    }

    /// 参照レイヤーの指す先を替える（読む元のチャンネルは保つ）。同じ文書の指す先が無い・循環になる・参照の連なりが上限を超えるなら断る。
    /// 1 回の Undo。
    pub fn set_pointer_target(
        &mut self,
        id: LayerId,
        target: PointerTarget,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.pointer_layer_index(id)?;
        self.require_pointer_target(target)?;
        let old = self.layers[index]
            .pointer
            .clone()
            .unwrap_or_else(|| PointerSettings::new(target));
        let new = PointerSettings {
            target,
            sources: old.sources.clone(),
        };
        self.change_pointer(id, index, old, new)
    }

    /// 参照レイヤーのチャンネル `channel` が読む元を替える（None か `channel` 自身は既定の同じチャンネル）。`channel` は文書のチャンネル。
    /// 同じ文書の指す先なら、読む元も文書のチャンネルに限る。1 回の Undo。
    pub fn set_pointer_source(
        &mut self,
        id: LayerId,
        channel: Channel,
        source: Option<Channel>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.pointer_layer_index(id)?;
        self.require_channel(channel)?;
        let Some(old) = self.layers[index].pointer.clone() else {
            return Err(CoreError::Unsupported(Reason::NotPointerLayer));
        };
        if let Some(s) = source {
            if !matches!(old.target, PointerTarget::External(_)) {
                self.require_channel(s)?;
            }
        }
        let new = old.clone().with_source(channel, source);
        self.change_pointer(id, index, old, new)
    }

    fn change_pointer(
        &mut self,
        id: LayerId,
        index: usize,
        old: PointerSettings,
        new: PointerSettings,
    ) -> Result<(), CoreError> {
        if self.layers[index].pointer.as_ref() == Some(&new) {
            return Ok(());
        }
        self.refuse_lock(id, super::LayerLocks::ALL)?;
        self.execute(
            Command::Pointer {
                id,
                old: Box::new(old),
                new: Box::new(new),
            },
            96,
        )
    }

    /// 段: 参照レイヤーの設定を `new`（戻すなら `old`）にする。
    pub(super) fn switch_pointer(
        &mut self,
        id: LayerId,
        old: &PointerSettings,
        new: &PointerSettings,
        backwards: bool,
    ) -> Result<(), CoreError> {
        let settings = if backwards { old } else { new };
        let index = self.pointer_layer_index(id)?;
        self.layers[index].pointer = Some(settings.clone());
        self.mark_layer(index, None);
        self.mark_clipped_layers();
        Ok(())
    }

    /// 読み込み用: 指す先の無い参照レイヤーを一番上に足す（指す先は、レイヤーの ID が保存した物になってから
    /// [`Document::set_pointer_for_load`] で置く）。`enabled` は有効なチャンネル。履歴を消す。
    pub fn add_pointer_layer_for_load(
        &mut self,
        name: &str,
        enabled: &[Channel],
    ) -> Result<LayerId, CoreError> {
        self.ensure_loadable()?;
        for c in enabled {
            self.require_channel(*c)?;
        }
        let id = self.new_layer_id();
        let mut layer = Layer::new(id, name, LayerKind::Pointer);
        for c in enabled {
            layer.set_enabled(*c, true);
        }
        let (index, parent, set) = self.placement_in(None, None)?;
        layer.parent = parent;
        layer.set = set;
        self.layers.insert(index, layer);
        self.external_mutation();
        Ok(id)
    }

    /// 読み込み用: 参照レイヤーの指す先と読む元を置く（同じ文書の指す先は無くてもよい: 「指す先が無い」状態で開く）。読む元を替えるチャンネルは
    /// 文書のチャンネルに限る。循環・深さは、全部を置いてから [`Document::check_pointers_for_load`] で確かめる。履歴を消す。
    pub fn set_pointer_for_load(
        &mut self,
        id: LayerId,
        settings: PointerSettings,
    ) -> Result<(), CoreError> {
        self.ensure_loadable()?;
        let index = self.pointer_layer_index(id)?;
        for (c, _) in settings.sources() {
            self.require_channel(c)?;
        }
        self.layers[index].pointer = Some(settings);
        self.external_mutation();
        Ok(())
    }

    /// 読み込み用: 循環・参照の連なりの上限を超える参照レイヤーがあれば断る（循環が先）。
    pub fn check_pointers_for_load(&self) -> Result<(), CoreError> {
        let analysis = self.pointer_analysis();
        let mut deep = false;
        for s in analysis.states.values() {
            match s {
                PointerState::Issue(PointerIssue::Cycle) => {
                    return Err(CoreError::InvalidArgument(Reason::PointerCycle))
                }
                PointerState::Issue(PointerIssue::TooDeep) => deep = true,
                _ => {}
            }
        }
        if deep {
            return Err(CoreError::InvalidArgument(Reason::PointerTooDeep));
        }
        Ok(())
    }

    // ───────── 状態・アプリへの口 ─────────

    /// 参照レイヤーが今は指す先を読めない、レイヤー全体の理由（指す先が無い・循環・深すぎる。読めるなら None）。チャンネルごとの理由
    /// （読む元のチャンネルが無い・ほかのテクスチャセットの合成がまだ無い）は [`Document::inactive_effect_list`] が挙げる。
    pub fn pointer_issue(&self, id: LayerId) -> Result<Option<PointerIssue>, CoreError> {
        let index = self.pointer_layer_index(id)?;
        Ok(match self.pointer_state(index) {
            Some(PointerState::Issue(i)) => Some(i),
            _ => None,
        })
    }

    /// 参照レイヤーの、自分を含めた参照の連なりの段の数（この文書の中だけで数える。ほかのテクスチャセットを指すものは 1。循環なら None）。
    pub fn pointer_depth(&self, id: LayerId) -> Result<Option<usize>, CoreError> {
        let index = self.pointer_layer_index(id)?;
        let mut reads = Reads::new(self);
        Ok(self.depths_from(&mut reads, &[index]).depth[index])
    }

    /// この文書の、ほかのテクスチャセットを指す参照レイヤーと指す先の鍵（並びの順）。アプリが、どのテクスチャセットのどのレイヤーセットの
    /// 合成を渡すか（[`crate::EffectInputs::with_pointer`]）を決める。
    pub fn external_pointers(&self) -> Vec<(LayerId, ExternalSet)> {
        self.layers
            .iter()
            .filter_map(|l| match l.pointer.as_ref().map(|p| p.target) {
                Some(PointerTarget::External(k)) => Some((l.id, k)),
                _ => None,
            })
            .collect()
    }

    /// ほかのテクスチャセットを指す参照レイヤーが読む入力（鍵と、向こうのレイヤーセットの合成のチャンネル）。有効なチャンネルごとに、読む元のチャンネル。
    /// アプリはこれだけを渡せばよい（読まれない入力は [`Document::set_effect_inputs`] が持たずに外す）。参照レイヤーの追加・指す先と読む元の変更・
    /// Undo・Redo・読み込みで替わるので、替わったら渡し直す。
    pub fn external_pointer_reads(&self) -> std::collections::HashSet<(ExternalSet, Channel)> {
        let mut out = std::collections::HashSet::new();
        for l in &self.layers {
            let Some(p) = l.pointer.as_ref() else {
                continue;
            };
            if let PointerTarget::External(k) = p.target {
                for c in l.enabled_channels() {
                    out.insert((k, p.source(c)));
                }
            }
        }
        out
    }

    /// ほかの文書がこの文書のレイヤーセット `set` を指したときに辿る、この文書の中の参照の連なり。アプリは、指す側の段の数に `depth` と、
    /// `external` の各項目の `levels` + 向こうのレイヤーセットの深さを足して、テクスチャセットをまたぐ深さを求め、同じ（テクスチャセット・
    /// レイヤーセット）へ戻る道で循環を見つける。この文書の中に循環があれば、輪の先は数えない（編集と読み込みが断るので、ふつうは無い）。
    pub fn pointer_reach(&self, set: LayerSetId) -> Result<PointerReach, CoreError> {
        self.require_layer_set(set)?;
        self.reach_with(Reads::new(self), set)
    }

    /// [`Document::pointer_reach`] の、参照レイヤー `id` の指す先を `target` に替えたものとした形（文書は変えない）。画面が、指す先の候補が
    /// テクスチャセットをまたいで循環・深すぎるかを、替える前に確かめる。
    pub fn pointer_reach_if(
        &self,
        set: LayerSetId,
        id: LayerId,
        target: PointerTarget,
    ) -> Result<PointerReach, CoreError> {
        self.require_layer_set(set)?;
        let index = self.pointer_layer_index(id)?;
        self.reach_with(Reads::swapped(self, index, target), set)
    }

    /// 参照レイヤー `id` の指す先を `target` に替えたら、この文書の中で前に無かった循環・深すぎる参照レイヤーができるか（できるなら理由。
    /// 循環が先）。文書は変えない。画面が、押せない候補とその理由に使う（替える操作は同じ場合に断る）。ほかのテクスチャセットをまたぐ分は
    /// 見ない（[`Document::pointer_reach_if`]）。
    pub fn pointer_target_issue(
        &self,
        id: LayerId,
        target: PointerTarget,
    ) -> Result<Option<PointerIssue>, CoreError> {
        let index = self.pointer_layer_index(id)?;
        let before = self.pointer_violations();
        let mut reads = Reads::swapped(self, index, target);
        let pointers: Vec<usize> = (0..self.layers.len())
            .filter(|&i| self.layers[i].kind == LayerKind::Pointer)
            .collect();
        let depths = self.depths_from(&mut reads, &pointers);
        let mut deep = false;
        for &p in &pointers {
            if before.contains(&self.layers[p].id) {
                continue;
            }
            match depths.depth[p] {
                None => return Ok(Some(PointerIssue::Cycle)),
                Some(d) if d > MAX_POINTER_DEPTH => deep = true,
                _ => {}
            }
        }
        Ok(deep.then_some(PointerIssue::TooDeep))
    }

    fn reach_with(&self, mut reads: Reads<'_>, set: LayerSetId) -> Result<PointerReach, CoreError> {
        let roots = reads
            .target_members(PointerTarget::LayerSet(set))
            .unwrap_or_default();
        let depths = self.depths_from(&mut reads, &roots);
        let own = |i: usize| usize::from(self.layers[i].kind == LayerKind::Pointer);
        let depth = roots
            .iter()
            .filter_map(|&r| depths.depth[r])
            .max()
            .unwrap_or(0);
        // 根からその節までの段の数（その節を含む）: 後順の逆（辺の元が先）に広げる
        let mut levels: Vec<Option<usize>> = vec![None; self.layers.len()];
        for &r in &roots {
            levels[r] = Some(own(r));
        }
        for &v in depths.order.iter().rev() {
            let Some(lv) = levels[v] else { continue };
            for &s in reads.of(v).iter() {
                if depths.depth[s].is_none() {
                    continue;
                }
                let next = lv + own(s);
                if levels[s].is_none_or(|old| old < next) {
                    levels[s] = Some(next);
                }
            }
        }
        let mut external: Vec<ExternalReach> = Vec::new();
        for (i, l) in self.layers.iter().enumerate() {
            let (Some(lv), Some(PointerTarget::External(key))) = (levels[i], reads.target_of(i))
            else {
                continue;
            };
            match external.iter_mut().find(|e| e.key == key) {
                Some(e) if e.levels >= lv => {}
                Some(e) => {
                    e.levels = lv;
                    e.layer = l.id;
                }
                None => external.push(ExternalReach {
                    layer: l.id,
                    key,
                    levels: lv,
                }),
            }
        }
        Ok(PointerReach { depth, external })
    }

    /// 効いていない参照レイヤー（レイヤー全体の理由はチャンネル None で 1 件、チャンネルごとの理由は有効なチャンネルごと）。
    pub(super) fn pointer_inactive(&self, out: &mut Vec<InactiveEffect>) {
        for (i, l) in self.layers.iter().enumerate() {
            if l.kind != LayerKind::Pointer {
                continue;
            }
            let push = |out: &mut Vec<InactiveEffect>, c: Option<Channel>, issue: PointerIssue| {
                out.push(InactiveEffect {
                    layer: l.id,
                    layer_name: l.name.clone(),
                    target: InactiveTarget::Pointer(c),
                    reason: InactiveReason::Pointer(issue),
                });
            };
            if let Some(PointerState::Issue(issue)) = self.pointer_state(i) {
                if !l.enabled_channels().is_empty() {
                    push(out, None, issue);
                }
                continue;
            }
            for c in l.enabled_channels() {
                if let Some(issue) = self.pointer_channel_issue(i, c) {
                    push(out, Some(c), issue);
                }
            }
        }
    }

    /// 変化したタイルを、同じ文書を指す読める参照レイヤーの出力へ広げる（読む元のチャンネル `s` の変わったタイルが、参照レイヤーのチャンネル `c` の
    /// 同じタイルを変え、参照レイヤーのフィルターの半径だけ広がる）。どのレイヤーが変わったかは記録に無いので、`s` のどのタイルの変化も
    /// 指す先の変化として扱う（多めに描き直すだけ）。何か足したら true。sets はチャンネルの番号ごとの変わったタイル。
    pub(super) fn close_over_pointer_readers(
        &self,
        sets: &mut HashMap<Channel, HashSet<crate::types::TileCoord>>,
    ) -> bool {
        use crate::types::TileCoord;
        let (cols, rows) = (
            self.width.div_ceil(self.tile_size),
            self.height.div_ceil(self.tile_size),
        );
        let mut grew = false;
        for (i, l) in self.layers.iter().enumerate() {
            let Some(settings) = l.pointer.as_ref() else {
                continue;
            };
            if matches!(settings.target, PointerTarget::External(_))
                || !matches!(self.pointer_state(i), Some(PointerState::Ready { .. }))
            {
                continue;
            }
            for c in l.enabled_channels() {
                let s = settings.source(c);
                let source: Vec<TileCoord> = sets
                    .get(&s)
                    .map(|t| t.iter().copied().collect())
                    .unwrap_or_default();
                if source.is_empty() {
                    continue;
                }
                let chain = l.active_chain(c);
                let halo: u32 = chain.iter().map(|e| e.settings.halo()).sum();
                let global = chain.iter().any(|e| e.settings.is_global());
                let set = sets.entry(c).or_default();
                if global {
                    for y in 0..rows {
                        for x in 0..cols {
                            grew |= set.insert(TileCoord::new(x, y));
                        }
                    }
                    continue;
                }
                let m = halo.div_ceil(self.tile_size);
                for t in &source {
                    for y in t.y.saturating_sub(m)..=(t.y + m).min(rows - 1) {
                        for x in t.x.saturating_sub(m)..=(t.x + m).min(cols - 1) {
                            grew |= set.insert(TileCoord::new(x, y));
                        }
                    }
                }
                let seam = self.seam_shape(chain.iter().map(|e| e.settings.halo()));
                for t in self.seam_write_tiles(seam, halo, source) {
                    grew |= sets.entry(c).or_default().insert(t);
                }
            }
        }
        grew
    }

    /// 写し（複製・貼り付け）の中の参照レイヤーが、同じ写しの中のレイヤーを指していれば、写しの方を指すようにする（Anchor の段が写しの
    /// Anchor を読むのと同じ）。写しの外を指すものは、元の指す先のまま。`ids` は元の ID → 写しの ID。
    pub(super) fn remap_pointer_targets(copies: &mut [Layer], ids: &HashMap<LayerId, LayerId>) {
        for l in copies.iter_mut() {
            if let Some(p) = &mut l.pointer {
                if let PointerTarget::Layer(t) = p.target {
                    if let Some(n) = ids.get(&t) {
                        p.target = PointerTarget::Layer(*n);
                    }
                }
            }
        }
    }
}
