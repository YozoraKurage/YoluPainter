//! ポインターレイヤー（core の `LayerKind::Pointer`）のアプリの側: 作る操作、指す先の候補と押せない理由、ほかのテクスチャセットの合成を
//! 渡す配線（`EffectInputs::with_pointer`）、テクスチャセットをまたぐ循環と深さの判定。
//!
//! - **渡す配線**: 毎フレームの効果の入力の同期（`fx::inputs`）の中で、各セットの文書の `external_pointers` を見て、指されたテクスチャセットの
//!   文書のレイヤーセットの合成（読む元のチャンネル）を作り、同じ入力を指す全部の文書へ渡す。作り直すのは向こうの文書が変わった時だけ
//!   （文書の ID・版・変化の通し番号）。変わったタイルが分かれば、前の画素を写してそのタイルだけ合成し直す。描いている間は作り直さない
//!   （`sync_effects` が描いている間は同期しない。ストロークの終わりにまとめて追従する）。
//! - **入力待ちで読むだけのセット**を指すときは、開く時と同じ画素の予算で正本を core の文書にしてから合成する（効果の入力は渡さない）。
//!   変換できなければ「予算に収まらない」の理由にする。
//! - **予算**: 渡す合成の画素の合計を、設定のレイヤーのメモリの 4 分の 1（256 MiB を下回らない）までにし、鍵・チャンネルの順に詰める。
//!   同じ合成は全部の文書で共有する（`Arc`）ので、数えるのは 1 度。core の各文書の予算も同じ値にする（各文書が持つのは共有の一部）。
//! - **テクスチャセットをまたぐ判定**: 節を（テクスチャセット・レイヤーセット）にし、core の `pointer_reach` の深さと、ほかのテクスチャセットを
//!   指す辺（`levels`）で、節ごとの一番長い連なりを求める。戻る道は循環、`MAX_POINTER_DEPTH` を超えるのは深すぎる。編集の後に新しく
//!   できたら戻して断り（`AppState::apply`）、.ylp を開く時にあれば開かない。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use yolu_core::{
    Channel, CoreError, Document, ExternalSet, InactiveEffect, InactiveReason, InactiveTarget,
    LayerId, LayerKind, LayerSetId, PointerInput, PointerIssue, PointerTarget, Reason, Rect,
    RowOrder, MAX_POINTER_DEPTH,
};

use crate::state::AppState;

/// 指す先を選んでいない（メニューから作った）ポインターレイヤーの指す先。どのテクスチャセットの ID とも重ならない、全部のビットが 1 の鍵
/// （.ylp の GUID は空を断るので 0 にしない。このまま書き、開くと「指す先が無い」）。
pub const UNSET: ExternalSet = ExternalSet {
    texture_set: u128::MAX,
    layer_set: u128::MAX,
};

/// ポインターレイヤーの操作（`FxOp::Pointer`。どれも 1 回の Undo）。
#[derive(Clone, Debug, PartialEq)]
pub enum PointerOp {
    /// 選んでいるレイヤーのすぐ上に作る（`Some` はそのレイヤー・グループを指す。`None` は指す先なしで作り、プロパティで選ぶ）。
    New(Option<LayerId>),
    /// 指す先を替える。
    Target {
        layer: LayerId,
        target: PointerTarget,
    },
    /// チャンネル `channel` が読む元を替える（None は同じチャンネル）。
    Source {
        layer: LayerId,
        channel: Channel,
        source: Option<Channel>,
    },
}

impl AppState {
    /// ポインターレイヤーの操作を当てる（`fx_apply` が描いている間を断り、変更の印と断りの理由を出す）。
    pub(crate) fn pointer_run(&mut self, op: PointerOp) -> Result<Option<String>, CoreError> {
        self.session.doc.end_coalescing();
        match op {
            PointerOp::New(target) => {
                let name = self.new_layer_name(LayerKind::Pointer);
                let target = target.map_or(PointerTarget::External(UNSET), PointerTarget::Layer);
                let id =
                    self.add_in_current_set(|d, above| d.add_pointer_layer(&name, target, above))?;
                self.select_new(id);
            }
            PointerOp::Target { layer, target } => {
                self.session.doc.set_pointer_target(layer, target)?
            }
            PointerOp::Source {
                layer,
                channel,
                source,
            } => self
                .session
                .doc
                .set_pointer_source(layer, channel, source)?,
        }
        Ok(None)
    }
}

/// 渡す合成の予算の下限（core の既定と同じ 256 MiB）。
const MIN_BUDGET: u64 = 256 * 1024 * 1024;

/// .ylp のセットの ID（ハイフン付きの GUID）から、ほかのテクスチャセットを指す鍵の数へ。
pub fn set_key(id: &str) -> Option<u128> {
    let hex = id.replace('-', "");
    (hex.len() == 32)
        .then(|| u128::from_str_radix(&hex, 16).ok())
        .flatten()
        .filter(|v| *v != 0)
}

// ───────── テクスチャセットをまたぐ判定 ─────────

/// 判定に使う 1 つのセット: 鍵の数と文書。
pub(crate) struct SetDoc<'a> {
    pub key: u128,
    pub doc: &'a Document,
}

/// 確かめるだけの差し替え（そのセットのそのポインターレイヤーの指す先を替えたものとして判定する）。
#[derive(Clone, Copy)]
pub(crate) struct Swap {
    pub key: u128,
    pub layer: LayerId,
    pub target: PointerTarget,
}

/// 節（テクスチャセット・レイヤーセット）の、この文書の中の深さと、ほかを指す辺。
struct Node {
    depth: usize,
    edges: Vec<(ExternalSet, usize)>,
}

/// 判定の結果: 循環・深すぎる節と、ほかのテクスチャセットを指すポインターレイヤー。
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub(crate) struct Cross {
    pub nodes: HashMap<ExternalSet, PointerIssue>,
    pub layers: HashMap<(u128, LayerId), PointerIssue>,
}

impl Cross {
    /// `before` に無かった循環・深すぎるがあれば、その理由（循環が先）。
    pub fn new_issue(&self, before: &Cross) -> Option<PointerIssue> {
        let fresh: Vec<PointerIssue> = self
            .nodes
            .iter()
            .filter(|(k, i)| before.nodes.get(k) != Some(i))
            .map(|(_, i)| *i)
            .chain(
                self.layers
                    .iter()
                    .filter(|(k, i)| before.layers.get(k) != Some(i))
                    .map(|(_, i)| *i),
            )
            .collect();
        if fresh.contains(&PointerIssue::Cycle) {
            Some(PointerIssue::Cycle)
        } else {
            fresh.first().copied()
        }
    }
}

/// 外を指す辺が 1 つも無ければ（差し替えも外を指さなければ）、またぐ判定は要らない。
fn has_external(docs: &[SetDoc<'_>], swap: Option<&Swap>) -> bool {
    swap.is_some_and(|s| matches!(s.target, PointerTarget::External(_)))
        || docs.iter().any(|d| !d.doc.external_pointers().is_empty())
}

fn nodes_of(docs: &[SetDoc<'_>], swap: Option<&Swap>) -> HashMap<ExternalSet, Node> {
    let mut nodes = HashMap::new();
    for d in docs {
        for info in d.doc.layer_sets() {
            let reach = match swap.filter(|s| s.key == d.key) {
                Some(s) => d.doc.pointer_reach_if(info.id, s.layer, s.target),
                None => d.doc.pointer_reach(info.id),
            };
            let Ok(reach) = reach else { continue };
            nodes.insert(
                ExternalSet {
                    texture_set: d.key,
                    layer_set: info.id.0,
                },
                Node {
                    depth: reach.depth,
                    edges: reach.external.iter().map(|e| (e.key, e.levels)).collect(),
                },
            );
        }
    }
    nodes
}

/// 節から先の一番長い連なりの段の数（輪にある・輪へ届く節は None）。無い節（指す先が無い）は 0。
fn visit(
    key: ExternalSet,
    nodes: &HashMap<ExternalSet, Node>,
    state: &mut HashMap<ExternalSet, Option<Option<usize>>>,
) -> Option<usize> {
    match state.get(&key) {
        Some(None) => return None, // 辿っている途中へ戻った（輪）
        Some(Some(done)) => return *done,
        None => {}
    }
    let Some(node) = nodes.get(&key) else {
        return Some(0);
    };
    state.insert(key, None);
    let mut best = Some(node.depth);
    for (next, levels) in &node.edges {
        let d = visit(*next, nodes, state);
        best = match (best, d) {
            (Some(b), Some(d)) => Some(b.max(levels + d)),
            _ => None,
        };
    }
    state.insert(key, Some(best));
    best
}

/// テクスチャセットをまたぐ循環・深さの判定（`swap` があれば、その差し替えをしたものとして）。
pub(crate) fn cross_check(docs: &[SetDoc<'_>], swap: Option<&Swap>) -> Cross {
    let mut out = Cross::default();
    if !has_external(docs, swap) {
        return out;
    }
    let nodes = nodes_of(docs, swap);
    let mut state = HashMap::new();
    let mut keys: Vec<ExternalSet> = nodes.keys().copied().collect();
    keys.sort();
    for key in keys {
        match visit(key, &nodes, &mut state) {
            None => {
                out.nodes.insert(key, PointerIssue::Cycle);
            }
            Some(d) if d > MAX_POINTER_DEPTH => {
                out.nodes.insert(key, PointerIssue::TooDeep);
            }
            _ => {}
        }
    }
    for d in docs {
        for (layer, target) in external_targets(d, swap) {
            let issue = match visit(target, &nodes, &mut state) {
                None => Some(PointerIssue::Cycle),
                Some(depth) if depth + 1 > MAX_POINTER_DEPTH => Some(PointerIssue::TooDeep),
                _ => None,
            };
            if let Some(issue) = issue {
                out.layers.insert((d.key, layer), issue);
            }
        }
    }
    out
}

/// セットの、ほかのテクスチャセットを指すポインターレイヤーと鍵（差し替えを当てた形）。
fn external_targets(d: &SetDoc<'_>, swap: Option<&Swap>) -> Vec<(LayerId, ExternalSet)> {
    let mut list = d.doc.external_pointers();
    if let Some(s) = swap.filter(|s| s.key == d.key) {
        list.retain(|(l, _)| *l != s.layer);
        if let PointerTarget::External(k) = s.target {
            list.push((s.layer, k));
        }
    }
    list
}

// ───────── 渡す配線 ─────────

/// 作って持っている合成 1 枚（作った時の向こうの文書の印つき）。
struct Held {
    /// 作った文書（ID・版・変化の通し番号）。入力待ちの読むだけのセットから作ったものは None（正本は変わらない）。
    stamp: Option<(u128, u64, u64)>,
    input: PointerInput,
}

/// 判定の覚えの鍵: 今のセットの uid と、編集できる各セットの（uid・文書の ID・版）。文書を別の物に替えると捨てる（`forget`）。
type Stamp = (u32, Vec<(u32, u128, u64)>);

/// テクスチャセットをまたぐ判定の覚え（毎フレームの同期・操作の前後・候補ごとの確かめで、文書が変わっていなければ計算し直さない）。
#[derive(Default)]
struct Memo {
    stamp: Stamp,
    cross: Option<Cross>,
    /// （ポインターレイヤー・指す先）ごとの、選べない理由（候補の一覧）。
    refusals: HashMap<(LayerId, PointerTarget), Option<PointerIssue>>,
}

/// ほかのテクスチャセットの合成の覚え（`fx::inputs::InputsState` が持つ）。
#[derive(Default)]
pub struct PointerFeed {
    /// 判定の覚え（`pointer_cross`・`pointer_candidates`）。
    memo: Mutex<Memo>,
    held: HashMap<(ExternalSet, Channel), Held>,
    /// アプリが見つけた、core の「まだ渡されていない」より詳しい理由（セットの uid・レイヤー・チャンネル。None はレイヤー全体）。
    issues: HashMap<(u32, LayerId, Option<Channel>), PointerIssue>,
    /// 入力待ちの読むだけのセット（uid）の正本の変換の結果。
    waiting: HashMap<u32, Waiting>,
    /// 入力待ちのセットから作れなかった合成（指す先・チャンネルが無い・作れない。そのときの画素の予算つき）。
    unbuildable: HashMap<(ExternalSet, Channel), (u64, PointerIssue, bool)>,
    /// 試験用: 入力待ちのセットの正本を変換した回数。
    pub converted: u64,
    /// 試験用: 予算（None は設定から）。
    pub budget_override: Option<u64>,
    /// 試験用: 合成を作った回数（全部・変わったタイルだけ）。
    pub built_full: u64,
    pub built_tiles: u64,
}

impl PointerFeed {
    /// 判定の覚えを捨てる（セットの文書を別の物に替えたとき。同じ ID・版でも中身が違う）。
    pub(crate) fn forget_checks(&self) {
        *self.memo.lock().unwrap_or_else(PoisonError::into_inner) = Memo::default();
    }

    /// 鍵の入力（そのセットの文書へ渡す）。
    pub(crate) fn inputs_for<'a>(
        &'a self,
        doc: &Document,
    ) -> impl Iterator<Item = (ExternalSet, Channel, &'a PointerInput)> + 'a {
        // 渡すのは、文書のポインターレイヤーが今読む物だけ（core は読まない入力を持たずに外す）
        let reads = doc.external_pointer_reads();
        let mut list: Vec<(ExternalSet, Channel, &PointerInput)> = self
            .held
            .iter()
            .filter(|(k, _)| reads.contains(k))
            .map(|((k, c), h)| (*k, *c, &h.input))
            .collect();
        list.sort_by_key(|(k, c, _)| (*k, c.index()));
        list.into_iter()
    }

    /// 文書が、渡すべき入力を全部（同じ画素の実体で）持っているか。core は予算・読む側の変化で入力を外し、あとから自動では戻さないので、
    /// 欠けていれば渡し直す。
    pub(crate) fn delivered_to(&self, doc: &Document) -> bool {
        self.inputs_for(doc).all(|(k, c, input)| {
            doc.effect_inputs()
                .pointer(k, c)
                .is_some_and(|held| Arc::ptr_eq(&held.pixels, &input.pixels))
        })
    }

    /// 文書へ渡した入力の鍵（`fx::inputs` の鍵に入れる。画素は共有の実体の番地で比べる）。
    pub(crate) fn key_for(&self, doc: &Document) -> Vec<(ExternalSet, u8, usize)> {
        self.inputs_for(doc)
            .map(|(k, c, i)| (k, c.index() as u8, i.pixels.as_ptr() as usize))
            .collect()
    }

    /// 持っている合成の画素のバイト数。
    pub fn held_bytes(&self) -> u64 {
        self.held.values().map(|h| h.input.bytes()).sum()
    }
}

/// 合成を作る相手 1 つ（指されたテクスチャセットの文書の、レイヤーセット・チャンネル）と、読むポインターレイヤー。
struct Want {
    readers: Vec<(u32, LayerId, Channel)>,
}

/// `refresh_pointer_inputs` の 1 回の間の持ち物。
#[derive(Default)]
struct Run {
    /// 渡す合成の画素の合計。
    used: u64,
    keep: HashSet<(ExternalSet, Channel)>,
    /// 入力待ちのセット（番号）の正本の変換（この呼び出しの中だけ。結果の文書は持ち続けない）と、変換した回数。
    converted: HashMap<usize, Option<Document>>,
    count: u64,
    /// 入力待ちのセットから作れなかった合成と理由（理由がレイヤー全体か）。
    unbuildable: Vec<((ExternalSet, Channel), PointerIssue, bool)>,
}

/// 入力待ちのセットの合成を作る前の確かめの結果。
enum Step {
    /// 変換して作る。
    Convert,
    /// 作ってある合成を使う（数えて残した）。
    Held,
    /// 前に分かった理由を出す（レイヤー全体か）。
    Replay(PointerIssue, bool),
}

/// 入力待ちの読むだけのセットの正本の変換で分かったこと（そのときの画素の予算つき）。
struct Waiting {
    load_budget: u64,
    /// 画素の大きさ（変換できなければ None）。
    size: Option<(u32, u32)>,
}

/// 作る元の文書。
enum Origin {
    /// 編集できるセット（番号）。
    Set(usize),
    /// 入力待ちで読むだけのセット（番号）。正本を変換して読む。
    Waiting(usize),
    /// 読めない（core で扱えない中身の読むだけのセット）。
    Unreadable,
}

impl AppState {
    /// 渡す合成の予算（設定の効果のメモリの 4 分の 1、256 MiB を下回らない）。持っている合成の量は、効果のキャッシュの配り
    /// （`sync_budgets`）から引く。
    pub fn pointer_input_budget(&self) -> u64 {
        if let Some(b) = self.fx.inputs.pointers.budget_override {
            return b;
        }
        let effects = self.prefs.settings.budgets(self.prefs.ram_mib).effects;
        (effects / 4).max(MIN_BUDGET)
    }

    /// 判定に使うセット（編集できるセット。読むだけのセットの文書は保存した合成の 1 枚で、ポインターレイヤーを持たない）。
    pub(crate) fn pointer_set_docs(&self) -> Vec<SetDoc<'_>> {
        (0..self.session.sets.len())
            .filter_map(|i| {
                let set = self.session.sets.get(i)?;
                if set.read_only.is_some() {
                    return None;
                }
                Some(SetDoc {
                    key: set_key(&set.id)?,
                    doc: self.set_doc(i),
                })
            })
            .collect()
    }

    /// .ylp を開く時: 変換したセット（.ylp のセットの ID と文書）に、テクスチャセットをまたぐ循環・連なりの上限の超えがあれば、その理由の文
    /// （循環が先）。
    pub(crate) fn pointer_load_issue(&self, sets: &[(&str, &Document)]) -> Option<String> {
        let docs: Vec<SetDoc<'_>> = sets
            .iter()
            .filter_map(|(id, doc)| {
                Some(SetDoc {
                    key: set_key(id)?,
                    doc,
                })
            })
            .collect();
        let issue = cross_check(&docs, None).new_issue(&Cross::default())?;
        let reason = match issue {
            PointerIssue::Cycle => Reason::PointerCycle,
            _ => Reason::PointerTooDeep,
        };
        Some(self.lang.core_error(&CoreError::InvalidArgument(reason)))
    }

    /// 判定の覚えの鍵（今のセットと、編集できる各セットの文書の版）。
    fn pointer_stamp(&self) -> Stamp {
        let list = (0..self.session.sets.len())
            .filter_map(|i| {
                let set = self.session.sets.get(i)?;
                (set.read_only.is_none()).then(|| {
                    let doc = self.set_doc(i);
                    (set.uid, doc.id(), doc.revision())
                })
            })
            .collect();
        (self.session.sets.current().uid, list)
    }

    /// 覚えを今の鍵に合わせて、`f` に渡す（文書が変わっていれば捨てる）。
    fn with_memo<T>(&self, f: impl FnOnce(&mut Memo) -> T) -> T {
        let stamp = self.pointer_stamp();
        let mut memo = self
            .fx
            .inputs
            .pointers
            .memo
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if memo.stamp != stamp {
            *memo = Memo {
                stamp,
                ..Memo::default()
            };
        }
        f(&mut memo)
    }

    /// テクスチャセットをまたぐ判定（今の状態。文書が変わっていなければ前の結果）。
    pub(crate) fn pointer_cross(&self) -> Cross {
        if let Some(cross) = self.with_memo(|m| m.cross.clone()) {
            return cross;
        }
        let cross = cross_check(&self.pointer_set_docs(), None);
        self.with_memo(|m| m.cross = Some(cross.clone()));
        cross
    }

    /// 鍵の数のセットの番号。
    pub(crate) fn set_index_of_key(&self, key: u128) -> Option<usize> {
        (0..self.session.sets.len())
            .find(|&i| self.session.sets.get(i).and_then(|s| set_key(&s.id)) == Some(key))
    }

    /// 毎フレーム（効果の入力の同期の中）: 渡す合成を、指されたセットの今の文書に合わせる。変えた合成があれば true。
    pub(crate) fn refresh_pointer_inputs(&mut self) -> bool {
        // 描いている間は呼ばれない（`sync_effects` が描いている間は同期しない）
        let cross = self.pointer_cross();
        let mut issues = HashMap::new();
        let wants = self.pointer_wants(&cross, &mut issues);
        let budget = self.pointer_input_budget();
        let load_budget = self.load_source_bytes();
        let mut run = Run::default();
        let mut changed = false;
        for ((key, channel), want) in &wants {
            let Some(index) = self.set_index_of_key(key.texture_set) else {
                continue;
            };
            let origin = match self.session.sets.get(index) {
                Some(s) if s.read_only.is_none() => Origin::Set(index),
                Some(s) if s.waiting_inputs => Origin::Waiting(index),
                _ => Origin::Unreadable,
            };
            let doc: &Document = match origin {
                Origin::Set(i) => self.set_doc(i),
                Origin::Waiting(i) => {
                    match self.waiting_step(i, (*key, *channel), (budget, load_budget), &mut run) {
                        Step::Replay(issue, whole) => {
                            if whole {
                                mark_whole(&mut issues, want, issue)
                            } else {
                                mark(&mut issues, want, issue)
                            }
                            continue;
                        }
                        Step::Held => continue,
                        Step::Convert => {}
                    }
                    let made = run.converted.entry(i).or_insert_with(|| {
                        run.count += 1;
                        self.convert_waiting(i)
                    });
                    match made.as_ref() {
                        Some(d) => d,
                        None => {
                            mark(&mut issues, want, PointerIssue::InputOverBudget);
                            continue;
                        }
                    }
                }
                Origin::Unreadable => continue,
            };
            let waiting_uid = match origin {
                Origin::Waiting(i) => self.session.sets.get(i).map(|s| s.uid),
                _ => None,
            };
            let set = LayerSetId(key.layer_set);
            let missing = if !doc.layer_sets().iter().any(|s| s.id == set) {
                Some((PointerIssue::MissingTarget, true))
            } else if doc.channel_info(*channel).is_none() {
                Some((PointerIssue::MissingChannel(*channel), false))
            } else {
                None
            };
            if let Some((issue, whole)) = missing {
                if whole {
                    mark_whole(&mut issues, want, issue)
                } else {
                    mark(&mut issues, want, issue)
                }
                if waiting_uid.is_some() {
                    run.unbuildable.push(((*key, *channel), issue, whole));
                }
                continue;
            }
            let bytes = u64::from(doc.width()) * u64::from(doc.height()) * 4;
            if run.used.saturating_add(bytes) > budget {
                mark(&mut issues, want, PointerIssue::InputOverBudget);
                continue;
            }
            run.used += bytes;
            run.keep.insert((*key, *channel));
            let stamp = match origin {
                Origin::Set(_) => Some((doc.id(), doc.revision(), doc.change_serial())),
                _ => None,
            };
            let feed = &self.fx.inputs.pointers;
            if feed
                .held
                .get(&(*key, *channel))
                .is_some_and(|h| h.stamp == stamp)
            {
                continue;
            }
            let old = feed.held.get(&(*key, *channel));
            match build_input(doc, set, *channel, old) {
                Ok((input, built)) => {
                    let feed = &mut self.fx.inputs.pointers;
                    match built {
                        Built::Kept => {}
                        Built::Tiles => feed.built_tiles += 1,
                        Built::Full => feed.built_full += 1,
                    }
                    changed |= built != Built::Kept;
                    feed.held.insert((*key, *channel), Held { stamp, input });
                }
                Err(_) => {
                    mark(&mut issues, want, PointerIssue::MissingInput);
                    if waiting_uid.is_some() {
                        run.unbuildable
                            .push(((*key, *channel), PointerIssue::MissingInput, false));
                    }
                }
            }
        }
        changed
            | self.finish_pointer_inputs(run, issues, wants.keys().copied().collect(), load_budget)
    }

    /// 入力待ちで読むだけのセット `index` の、合成 `at` を作る前の確かめ。正本は読むだけのあいだ変わらないので、前に分かった結果（変換
    /// できなかった・指す先やチャンネルが無い・画素の大きさ）と作ってある合成を使い、4096² の正本を毎フレーム変換し直さない。覚えは画素の
    /// 予算（`budgets` の 2 つ目）が替わると使わない（予算で変換できなかったセットを、予算を上げれば試し直す）。
    fn waiting_step(
        &self,
        index: usize,
        at: (ExternalSet, Channel),
        budgets: (u64, u64),
        run: &mut Run,
    ) -> Step {
        let (budget, load_budget) = budgets;
        let Some(uid) = self.session.sets.get(index).map(|s| s.uid) else {
            return Step::Convert;
        };
        let feed = &self.fx.inputs.pointers;
        if run.converted.contains_key(&index) {
            return Step::Convert; // この呼び出しで変換した。そのまま使う
        }
        let known = feed
            .waiting
            .get(&uid)
            .filter(|w| w.load_budget == load_budget);
        if known.is_some_and(|w| w.size.is_none()) {
            return Step::Replay(PointerIssue::InputOverBudget, false);
        }
        if let Some((_, issue, whole)) = feed
            .unbuildable
            .get(&at)
            .filter(|(b, _, _)| *b == load_budget)
        {
            return Step::Replay(*issue, *whole);
        }
        let held = feed.held.get(&at).is_some_and(|h| h.stamp.is_none());
        match known.and_then(|w| w.size) {
            Some((w, h)) if held => {
                let bytes = u64::from(w) * u64::from(h) * 4;
                if run.used.saturating_add(bytes) > budget {
                    return Step::Replay(PointerIssue::InputOverBudget, false);
                }
                run.used += bytes;
                run.keep.insert(at);
                Step::Held
            }
            _ => Step::Convert,
        }
    }

    /// `refresh_pointer_inputs` の終わり: 変換の覚えと数を持ち、もう読まない合成を捨て、理由を置く。変えた合成があれば true。
    fn finish_pointer_inputs(
        &mut self,
        run: Run,
        issues: HashMap<(u32, LayerId, Option<Channel>), PointerIssue>,
        wanted: HashSet<(ExternalSet, Channel)>,
        load_budget: u64,
    ) -> bool {
        let waiting: HashSet<u32> = (0..self.session.sets.len())
            .filter_map(|i| self.session.sets.get(i))
            .filter(|s| s.waiting_inputs)
            .map(|s| s.uid)
            .collect();
        let outcomes: Vec<(u32, Option<(u32, u32)>)> = run
            .converted
            .iter()
            .filter_map(|(i, d)| {
                let uid = self.session.sets.get(*i)?.uid;
                Some((uid, d.as_ref().map(|d| (d.width(), d.height()))))
            })
            .collect();
        let feed = &mut self.fx.inputs.pointers;
        feed.converted += run.count;
        feed.waiting.retain(|uid, _| waiting.contains(uid));
        for (uid, size) in outcomes {
            feed.waiting.insert(uid, Waiting { load_budget, size });
        }
        feed.unbuildable.retain(|k, _| wanted.contains(k));
        for (k, issue, whole) in run.unbuildable {
            feed.unbuildable.insert(k, (load_budget, issue, whole));
        }
        let before = feed.held.len();
        feed.held.retain(|k, _| run.keep.contains(k));
        feed.issues = issues;
        feed.held.len() != before
    }

    /// 合成を作る相手と、読むポインターレイヤー（またぐ循環・深すぎる・指す先の無いものは理由を覚えて外す）。
    fn pointer_wants(
        &self,
        cross: &Cross,
        issues: &mut HashMap<(u32, LayerId, Option<Channel>), PointerIssue>,
    ) -> BTreeMap<(ExternalSet, Channel), Want> {
        let mut wants: BTreeMap<(ExternalSet, Channel), Want> = BTreeMap::new();
        for i in 0..self.session.sets.len() {
            let Some(set) = self.session.sets.get(i) else {
                continue;
            };
            if set.read_only.is_some() {
                continue;
            }
            let (uid, own) = (set.uid, set_key(&set.id).unwrap_or(0));
            let doc = self.set_doc(i);
            for (layer, key) in doc.external_pointers() {
                if let Some(issue) = cross.layers.get(&(own, layer)) {
                    issues.insert((uid, layer, None), *issue);
                    continue;
                }
                if key == UNSET || self.set_index_of_key(key.texture_set).is_none() {
                    issues.insert((uid, layer, None), PointerIssue::MissingTarget);
                    continue;
                }
                let Some(l) = doc.layer(layer) else { continue };
                let Some(p) = l.pointer() else { continue };
                for c in l.enabled_channels() {
                    wants
                        .entry((key, p.source(c)))
                        .or_insert_with(|| Want {
                            readers: Vec::new(),
                        })
                        .readers
                        .push((uid, layer, c));
                }
            }
        }
        wants
    }

    /// 入力待ちで読むだけのセットの正本を、開く時と同じ画素の予算で core の文書にする（できなければ None）。
    fn convert_waiting(&self, index: usize) -> Option<Document> {
        let set = self.session.sets.get(index)?;
        let project = self.session.project.as_ref()?;
        let native = project.project().sets().iter().find(|s| s.id == set.id)?;
        crate::project::to_core(&native.document, self.lang, self.load_source_bytes()).ok()
    }

    /// セット `index` の、効いていない効果の一覧（core の一覧の、ポインターレイヤーの「まだ渡されていない」を、アプリが分かった理由
    /// （指す先が無い・読む元のチャンネルが無い・予算・テクスチャセットをまたぐ循環と深さ）に置き換えたもの）。
    pub fn inactive_effects_of(&self, index: usize) -> Vec<InactiveEffect> {
        let doc = self.set_doc(index);
        let list = doc.inactive_effect_list();
        let Some(uid) = self.session.sets.get(index).map(|s| s.uid) else {
            return list;
        };
        fix_pointer_reasons(list, &self.fx.inputs.pointers.issues, uid)
    }

    /// 今のセットの、ポインターレイヤー `layer` が今は読めない理由（レイヤー全体。読めるなら None）。
    pub fn pointer_layer_issue(&self, layer: LayerId) -> Option<PointerIssue> {
        let index = self.session.sets.current_index();
        self.inactive_effects_of(index)
            .into_iter()
            .find(|e| e.layer == layer && e.target == InactiveTarget::Pointer(None))
            .and_then(|e| match e.reason {
                InactiveReason::Pointer(i) => Some(i),
                _ => None,
            })
    }
}

/// core の一覧のポインターレイヤーの「まだ渡されていない」を、アプリの理由に置き換える（レイヤー全体の理由があれば、そのレイヤーの
/// チャンネルごとの行を 1 つにまとめる）。
fn fix_pointer_reasons(
    list: Vec<InactiveEffect>,
    issues: &HashMap<(u32, LayerId, Option<Channel>), PointerIssue>,
    uid: u32,
) -> Vec<InactiveEffect> {
    let mut out: Vec<InactiveEffect> = Vec::with_capacity(list.len());
    let mut whole: HashSet<LayerId> = HashSet::new();
    for mut e in list {
        let InactiveTarget::Pointer(channel) = e.target else {
            out.push(e);
            continue;
        };
        if e.reason != InactiveReason::Pointer(PointerIssue::MissingInput) {
            out.push(e);
            continue;
        }
        if let Some(issue) = issues.get(&(uid, e.layer, None)) {
            if whole.insert(e.layer) {
                e.target = InactiveTarget::Pointer(None);
                e.reason = InactiveReason::Pointer(*issue);
                out.push(e);
            }
            continue;
        }
        if let Some(issue) = channel.and_then(|c| issues.get(&(uid, e.layer, Some(c)))) {
            e.reason = InactiveReason::Pointer(*issue);
        }
        out.push(e);
    }
    out
}

fn mark(
    issues: &mut HashMap<(u32, LayerId, Option<Channel>), PointerIssue>,
    want: &Want,
    issue: PointerIssue,
) {
    for (uid, layer, c) in &want.readers {
        issues.insert((*uid, *layer, Some(*c)), issue);
    }
}

fn mark_whole(
    issues: &mut HashMap<(u32, LayerId, Option<Channel>), PointerIssue>,
    want: &Want,
    issue: PointerIssue,
) {
    for (uid, layer, _) in &want.readers {
        issues.insert((*uid, *layer, None), issue);
    }
}

/// 2 つの矩形を囲む矩形。
fn union(a: Rect, b: Rect) -> Rect {
    let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
    let x1 = (a.x + a.width).max(b.x + b.width);
    let y1 = (a.y + a.height).max(b.y + b.height);
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// 合成の作り方。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Built {
    /// このチャンネルは変わっていない（前の画素をそのまま共有する）。
    Kept,
    /// 変わったタイルだけ合成し直した。
    Tiles,
    /// 全体を合成した。
    Full,
}

/// レイヤーセット `set` のチャンネル `channel` の合成を作る。前に同じ文書から作った物があり、変わったタイルが分かれば（それを囲む矩形が全体の
/// 半分より小さければ）、前の画素を写してその矩形だけ合成し直す（変わったタイルが無ければ前の画素を共有したまま）。
fn build_input(
    doc: &Document,
    set: LayerSetId,
    channel: Channel,
    old: Option<&Held>,
) -> Result<(PointerInput, Built), CoreError> {
    let (w, h) = (doc.width(), doc.height());
    let tiles = old.and_then(|o| {
        let (id, _, serial) = o.stamp?;
        (id == doc.id() && (o.input.width, o.input.height) == (w, h))
            .then(|| doc.changed_tiles(channel, serial))
            .flatten()
            .map(|t| (o, t))
    });
    // 変わったタイルを囲む矩形を 1 度で合成し直す（タイルごとに呼ぶと、1 回ごとの評価の支度がタイルの数だけ掛かる）
    let whole = u64::from(w) * u64::from(h);
    let bounds = tiles.map(|(o, list)| {
        let rect = list.iter().filter_map(|c| doc.tile_rect(*c)).reduce(union);
        (o, rect)
    });
    let small =
        |r: &Option<Rect>| r.is_none_or(|r| u64::from(r.width) * u64::from(r.height) * 2 < whole);
    if let Some((o, rect)) = bounds.filter(|(_, r)| small(r)) {
        let Some(rect) = rect else {
            return Ok((o.input.clone(), Built::Kept));
        };
        // 前の画素を 1 度だけ写して（`Arc` の中へ直に）、矩形の行を置き換える
        let mut pixels: Arc<[u8]> = Arc::from(&o.input.pixels[..]);
        let buf = Arc::get_mut(&mut pixels).expect("写したばかりの画素は共有していない");
        let part = doc.composite_set_channel(set, channel, rect)?;
        let row = rect.width as usize * 4;
        for r in 0..rect.height as usize {
            let at = ((rect.y as usize + r) * w as usize + rect.x as usize) * 4;
            buf[at..at + row].copy_from_slice(&part[r * row..(r + 1) * row]);
        }
        return Ok((PointerInput::new(w, h, pixels)?, Built::Tiles));
    }
    // `Arc` の中へ直に合成する（Vec から移すと、もう 1 度写す）
    let mut pixels: Arc<[u8]> = std::iter::repeat_n(0u8, whole as usize * 4).collect();
    let buf = Arc::get_mut(&mut pixels).expect("作ったばかりの画素は共有していない");
    doc.composite_set_into_cancellable(
        set,
        channel,
        Rect::new(0, 0, w, h),
        buf,
        RowOrder::BottomUp,
        None,
    )?;
    Ok((PointerInput::new(w, h, pixels)?, Built::Full))
}

// ───────── 指す先の候補 ─────────

/// 指す先の候補 1 つ（ドロップダウンの行）。
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub target: PointerTarget,
    pub label: String,
    /// 字下げの段（レイヤー・グループの入れ子）。
    pub depth: usize,
    /// 選べない理由（選べるなら None）。
    pub refused: Option<PointerIssue>,
}

/// 候補の節（見出しと候補）。
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub heading: String,
    pub items: Vec<Candidate>,
}

impl AppState {
    /// 今のセットのポインターレイヤー `id` の、指す先の候補: このテクスチャセットのレイヤー・グループ（入れ子の字下げ）、このセットの
    /// レイヤーセット、ほかのテクスチャセットのレイヤーセット（セットの名前ごとの節）。選ぶと循環・深すぎる・自分自身のものは理由つき。
    pub fn pointer_candidates(&self, id: LayerId) -> Vec<Section> {
        let lang = self.lang;
        let doc = &self.session.doc;
        let index = self.session.sets.current_index();
        let own = self
            .session
            .sets
            .get(index)
            .and_then(|s| set_key(&s.id))
            .unwrap_or(0);
        let docs = self.pointer_set_docs();
        let before = self.pointer_cross();
        let refused = |target: PointerTarget| -> Option<PointerIssue> {
            if let Some(known) = self.with_memo(|m| m.refusals.get(&(id, target)).copied()) {
                return known;
            }
            let issue = if target == PointerTarget::Layer(id) {
                Some(PointerIssue::Cycle)
            } else if let Ok(Some(i)) = doc.pointer_target_issue(id, target) {
                Some(i)
            } else {
                let swap = Swap {
                    key: own,
                    layer: id,
                    target,
                };
                cross_check(&docs, Some(&swap)).new_issue(&before)
            };
            self.with_memo(|m| m.refusals.insert((id, target), issue));
            issue
        };
        let mut sections = Vec::new();
        // このテクスチャセットのレイヤー・グループ（上から、入れ子は字下げ）
        let mut layers = Vec::new();
        for info in doc.layer_sets() {
            for (layer, depth) in tree_of(doc, info.id) {
                let Some(l) = doc.layer(layer) else { continue };
                let target = PointerTarget::Layer(layer);
                layers.push(Candidate {
                    target,
                    label: l.name().to_owned(),
                    depth,
                    refused: refused(target),
                });
            }
        }
        sections.push(Section {
            heading: lang.t("fx-pointer-layers-and-groups").to_owned(),
            items: layers,
        });
        let sets: Vec<Candidate> = doc
            .layer_sets()
            .iter()
            .enumerate()
            .map(|(i, info)| {
                let target = PointerTarget::LayerSet(info.id);
                Candidate {
                    target,
                    label: crate::m2::layersets::display_name(lang, doc, i),
                    depth: 0,
                    refused: refused(target),
                }
            })
            .collect();
        sections.push(Section {
            heading: lang.t("fx-pointer-layer-sets").to_owned(),
            items: sets,
        });
        for other in 0..self.session.sets.len() {
            if other == index {
                continue;
            }
            let Some(set) = self.session.sets.get(other) else {
                continue;
            };
            // 読むだけのセットの文書は合成の PNG 1 枚の仮の文書で、レイヤーセットの ID が正本と違う（選ぶと、編集できるようになっても切れたまま）
            if set.read_only.is_some() {
                continue;
            }
            let Some(key) = set_key(&set.id) else {
                continue;
            };
            let odoc = self.set_doc(other);
            let items = odoc
                .layer_sets()
                .iter()
                .enumerate()
                .map(|(i, info)| {
                    let target = PointerTarget::External(ExternalSet {
                        texture_set: key,
                        layer_set: info.id.0,
                    });
                    Candidate {
                        target,
                        label: crate::m2::layersets::display_name(lang, odoc, i),
                        depth: 0,
                        refused: refused(target),
                    }
                })
                .collect();
            sections.push(Section {
                heading: set.name.clone(),
                items,
            });
        }
        sections
    }

    /// 指す先の名前（ドロップダウンに出す。指す先が無ければ空）。
    pub fn pointer_target_label(&self, target: PointerTarget) -> String {
        let lang = self.lang;
        let doc = &self.session.doc;
        match target {
            PointerTarget::Layer(l) => doc
                .layer(l)
                .map(|l| l.name().to_owned())
                .unwrap_or_default(),
            PointerTarget::LayerSet(s) => crate::m2::layersets::set_label(lang, doc, s),
            PointerTarget::External(k) => {
                let Some(i) = self.set_index_of_key(k.texture_set) else {
                    return String::new();
                };
                let name = self.session.sets.get(i).map_or("", |s| s.name.as_str());
                let set =
                    crate::m2::layersets::set_label(lang, self.set_doc(i), LayerSetId(k.layer_set));
                if set.is_empty() {
                    String::new()
                } else {
                    format!("{name} / {set}")
                }
            }
        }
    }

    /// 読む元に選べるチャンネル（指す先の文書のチャンネル。ほかのテクスチャセットなら向こうの文書の、無ければこの文書の）と名前。
    pub fn pointer_source_channels(&self, target: PointerTarget) -> Vec<(Channel, String)> {
        let lang = self.lang;
        let doc = match target {
            // 読むだけのセットの文書は仮の文書なので、並べない（指す先が無いときと同じく、この文書のチャンネル）
            PointerTarget::External(k) => match self.set_index_of_key(k.texture_set) {
                Some(i)
                    if self
                        .session
                        .sets
                        .get(i)
                        .is_some_and(|s| s.read_only.is_none()) =>
                {
                    self.set_doc(i)
                }
                _ => &self.session.doc,
            },
            _ => &self.session.doc,
        };
        doc.channels()
            .into_iter()
            .map(|c| (c, crate::m2::channel_name(lang, doc, c)))
            .collect()
    }
}

/// レイヤーセットのレイヤー・グループを、上から（グループの中身はその下に字下げで）。
fn tree_of(doc: &Document, set: LayerSetId) -> Vec<(LayerId, usize)> {
    let mut out = Vec::new();
    fn walk(
        doc: &Document,
        set: LayerSetId,
        parent: Option<LayerId>,
        depth: usize,
        out: &mut Vec<(LayerId, usize)>,
    ) {
        let Ok(children) = doc.children_in_set(set, parent) else {
            return;
        };
        for id in children.into_iter().rev() {
            out.push((id, depth));
            if doc.layer(id).is_some_and(|l| l.kind() == LayerKind::Group) {
                walk(doc, set, Some(id), depth + 1, out);
            }
        }
    }
    walk(doc, set, None, 0, &mut out);
    out
}

/// 操作の前後で、テクスチャセットをまたぐ循環・深すぎるが新しくできたら、そのセットの文書の操作を戻して理由を返す（`AppState::apply` と、外からの
/// 書き換え `OpHost::write_set` が使う）。
pub(crate) struct Guard {
    before: Option<GuardState>,
}

/// 操作の前の、セット（番号）の文書の印と、またぐ判定。
struct GuardState {
    index: usize,
    id: u128,
    revision: u64,
    undo: usize,
    redo: usize,
    cross: Cross,
}

impl AppState {
    /// 今のセットの文書に当てる操作の前に覚える（外を指す辺が無ければ何もしない）。
    pub(crate) fn pointer_guard(&self) -> Guard {
        self.pointer_guard_at(self.session.sets.current_index())
    }

    /// セット `index` の文書に当てる操作の前に覚える。
    pub(crate) fn pointer_guard_at(&self, index: usize) -> Guard {
        // 文書にポインターレイヤーが 1 枚も無ければ、操作が外を指す辺を作ることは無い（取り消しで戻るのもポインターレイヤーの段）
        let any = (0..self.session.sets.len()).any(|i| {
            self.session
                .sets
                .get(i)
                .is_some_and(|s| s.read_only.is_none())
                && self
                    .set_doc(i)
                    .layers()
                    .iter()
                    .any(|l| l.kind() == LayerKind::Pointer)
        });
        let doc = self.set_doc(index);
        if !any && !doc.can_undo() && !doc.can_redo() {
            return Guard { before: None };
        }
        Guard {
            before: Some(GuardState {
                index,
                id: doc.id(),
                revision: doc.revision(),
                undo: doc.undo_count(),
                redo: doc.redo_count(),
                cross: self.pointer_cross(),
            }),
        }
    }

    /// 操作の後に確かめる。新しくできていれば戻し、理由を知らせる。
    pub(crate) fn pointer_settle(&mut self, guard: Guard) {
        if let Some(e) = self.pointer_revert(guard) {
            self.refuse(crate::notice::Source::Layer, self.lang.core_error(&e));
            self.ensure_selection();
        }
    }

    /// 操作の後に確かめる。テクスチャセットをまたぐ循環・深すぎるが新しくできていれば、積んだ段は捨て、取り消し・やり直しは逆へ戻して、断りの
    /// 理由を返す（`CoreError::InvalidArgument`）。できていなければ（戻せなくても）None。
    pub(crate) fn pointer_revert(&mut self, guard: Guard) -> Option<CoreError> {
        let g = guard.before?;
        if self.session.sets.get(g.index)?.read_only.is_some() {
            return None;
        }
        let doc = self.set_doc(g.index);
        if doc.id() != g.id || doc.revision() == g.revision {
            return None;
        }
        let issue = self.pointer_cross().new_issue(&g.cross)?;
        let (u, r) = (doc.undo_count(), doc.redo_count());
        let doc = self.set_doc_mut(g.index);
        let undone: Result<(), CoreError> = if r > g.redo && u < g.undo {
            // 取り消し（履歴の一覧で戻ったのを含む）: 同じ数だけやり直す
            (0..r - g.redo).try_for_each(|_| doc.redo().map(|_| ()))
        } else if r < g.redo && u > g.undo {
            // やり直し: 同じ数だけ取り消す（やり直しの側に残る）
            (0..u - g.undo).try_for_each(|_| doc.undo().map(|_| ()))
        } else {
            // 1 回の操作が積んだ段の数だけ捨てる（履歴の上限で古い段が落ちて数が増えないときは、1 段）
            let pushed = u.saturating_sub(g.undo).max(1);
            (0..pushed).try_for_each(|_| doc.discard_last_step().map(|_| ()))
        };
        undone.ok()?;
        let reason = match issue {
            PointerIssue::Cycle => Reason::PointerCycle,
            _ => Reason::PointerTooDeep,
        };
        Some(CoreError::InvalidArgument(reason))
    }
}

#[cfg(test)]
mod tests;
