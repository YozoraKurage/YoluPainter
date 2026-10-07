//! 操作の記録: `AppState::apply` の前後で、当たった操作（`Action`）を CLI・MCP と同じ命令（`yolu_ops::Command`）へ写して残す。
//!
//! - 写せる操作の表は [`plan`]。写し方は、その操作が画面でした事と同じ結果になる命令（起動中のアプリのホストに当てて同じ文書になることを
//!   試験が確かめる）。写せない操作（描く・選択範囲・変形・結合・複製・チャンネルの構成など、命令の無い物）は記録せず、文書を変えたら
//!   記録の窓に印を出す（[`Recorder::skipped`]。数は出さない）。画面だけの操作（表示・ツール・色など）は記録も印もしない。
//! - 相手の指し方: 記録を始めた時に選んでいた層は `$selected`、記録の中で作った層・効果は `$created:<n>`（作った順の通し番号）、
//!   ほかの層は、そのときに名前が 1 つに決まるときだけ名前。どれにも当たらない（同じ名前が複数・記録の前からある効果）操作は写さない。
//! - 1 つの操作が 1 段（`Entry`）。取り消し・やり直しは、記録した段が文書の履歴の一番上にあるとき（文書の変更番号で見る）だけ段を外す・戻す。
//!   スライダーのドラッグ（core が 1 段にまとめる変更）は、同じ相手・同じ欄なら 1 つの命令にまとめ、Esc で止めたドラッグは段ごと外す。
//! - 記録は、始めた時の文書（テクスチャセット）だけ。ほかのセット・開き直した文書での操作は記録しない。

use std::collections::{BTreeMap, HashSet};

use yolu_core::effects::{EffectSettings, FilterId, FilterTarget};
use yolu_core::{AdjustmentSettings, Channel, Document, LayerId, LayerKind, LayerLocks};
use yolu_ops::action::MAX_COMMANDS;
use yolu_ops::command::*;
use yolu_ops::refs::{channel_name, parse_id, parse_relative, CREATED_PREFIX, SELECTED};
use yolu_ops::value::{format_color, Value};
use yolu_ops::Command;

use crate::fx::FxOp;
use crate::m2::Edit;
use crate::state::{Action, AppState};

/// 作った物の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Made {
    Layer,
    Effect,
}

/// スライダーのドラッグで 1 つにまとめる変更の鍵（core のまとめと同じ相手・同じ欄）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MergeKey {
    Opacity(LayerId),
    MaskDensity(LayerId),
    Fill(LayerId, Channel),
    Adjust(LayerId),
    Settings(FilterId),
    Strength(FilterId),
}

/// 記録した 1 つの操作。
#[derive(Clone, Debug)]
struct Entry {
    commands: Vec<Command>,
    /// この操作で作った層・効果（`$created:<n>` の番号は、段の並びでこれを数える）。
    created: Vec<(Made, u128)>,
    /// 操作の前と後の文書の変更番号。
    rev_before: u64,
    rev_after: u64,
    merge: Option<MergeKey>,
}

/// 記録中の状態。
#[derive(Clone, Debug)]
pub struct Recorder {
    entries: Vec<Entry>,
    /// 取り消しで外した段と、外したかわりに次のやり直しで戻せるか（直前に外した段とつながっているか）。
    undone: Vec<(Entry, bool)>,
    /// 記録を始めた文書。
    doc_id: u128,
    /// 記録を始めた時に選んでいた層（`$selected`）。
    start_selected: Option<LayerId>,
    /// 最後の段が文書の履歴の一番上にあるときの変更番号。
    top: Option<u64>,
    /// 外した段の最後が、やり直しの一番上にあるときの変更番号。
    redo_top: Option<u64>,
    /// 記録が知っている文書の変更番号（違えば、記録の外で文書が変わった）。
    seen: u64,
    /// 記録しなかった、文書を変えた操作があった。
    pub skipped: bool,
    /// `apply` の中の `apply`（入れ子の操作は外の操作として 1 回だけ記録する）。
    nested: bool,
}

impl Recorder {
    /// 今の文書で記録を始める。
    pub fn start(app: &AppState) -> Recorder {
        Recorder {
            entries: Vec::new(),
            undone: Vec::new(),
            doc_id: app.doc.id(),
            start_selected: app.selected_layer.filter(|id| app.doc.layer(*id).is_some()),
            top: None,
            redo_top: None,
            seen: app.doc.revision(),
            skipped: false,
            nested: false,
        }
    }

    /// 記録した命令（記録した順）。
    pub fn commands(&self) -> Vec<Command> {
        self.entries
            .iter()
            .flat_map(|e| e.commands.iter().cloned())
            .collect()
    }

    /// 記録した命令の数。
    pub fn len(&self) -> usize {
        self.entries.iter().map(|e| e.commands.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 記録の中で作った物の番号（1 から）。
    fn created_number(&self, kind: Made, id: u128) -> Option<usize> {
        self.entries
            .iter()
            .flat_map(|e| e.created.iter())
            .position(|c| *c == (kind, id))
            .map(|i| i + 1)
    }

    /// 記録の外で文書が変わっていたら印を出す（描く操作・外からの操作など）。
    fn sync(&mut self, doc: &Document) {
        if doc.id() == self.doc_id && doc.revision() != self.seen {
            self.skipped = true;
            self.seen = doc.revision();
        }
    }

    /// 最後の段を外す（取り消し・止めたドラッグ）。外した後に残った最後の段が一番上かは、段どうしの間に何も無かったかで決める。
    fn pop(&mut self, now: u64) -> Option<Entry> {
        let entry = self.entries.pop()?;
        self.top = match self.entries.last() {
            Some(prev) if prev.rev_after == entry.rev_before => Some(now),
            None => None,
            _ => None,
        };
        Some(entry)
    }
}

/// 記録の外で文書が変わったか（記録の窓が毎フレーム見る）。
pub fn sync(app: &mut AppState) {
    let doc_rev = (app.doc.id(), app.doc.revision());
    if let Some(rec) = app.automation.recorder.as_mut() {
        if doc_rev.0 == rec.doc_id && doc_rev.1 != rec.seen {
            rec.skipped = true;
            rec.seen = doc_rev.1;
        }
    }
}

/// 操作をどう記録するか。
#[derive(Clone, Debug)]
enum Plan {
    /// 画面だけの操作（文書を変えない）。
    Ui,
    /// 文書を変えるが、命令にできない。
    Skip,
    Undo,
    Redo,
    Record {
        commands: Vec<Command>,
        creates: Option<Made>,
        merge: Option<MergeKey>,
    },
}

/// 操作の前の様子（`after` が比べる）。
pub struct Pending {
    plan: Plan,
    doc_id: u128,
    rev_before: u64,
    undo_before: usize,
    trims_before: u64,
    /// 作る操作のときだけ、前からあった層・効果。
    before_ids: Option<(HashSet<u128>, HashSet<u128>)>,
}

/// 操作の前に呼ぶ（記録していなければ None）。
pub fn before(app: &mut AppState, action: &Action) -> Option<Pending> {
    let plan = {
        let rec = app.automation.recorder.as_ref()?;
        if rec.nested {
            return None;
        }
        if app.doc.id() == rec.doc_id {
            plan(app, rec, action)
        } else {
            Plan::Skip
        }
    };
    begin(app, plan)
}

/// レイヤーの名前を変える前に呼ぶ（名前の変更は、レイヤーの欄が文書へ直に当てる）。
pub fn before_rename(app: &mut AppState, id: LayerId, name: &str) -> Option<Pending> {
    let plan = {
        let rec = app.automation.recorder.as_ref()?;
        if rec.nested || app.doc.id() != rec.doc_id {
            Plan::Skip
        } else {
            match layer_ref(app, rec, id) {
                Some(layer) => record(
                    vec![Command::LayerSet(LayerSetArgs {
                        name: Some(name.to_owned()),
                        ..layer_set(layer)
                    })],
                    None,
                ),
                None => Plan::Skip,
            }
        }
    };
    begin(app, plan)
}

fn begin(app: &mut AppState, plan: Plan) -> Option<Pending> {
    let doc = &app.doc;
    let before_ids = matches!(
        plan,
        Plan::Record {
            creates: Some(_),
            ..
        }
    )
    .then(|| (layer_ids(doc), effect_ids(doc)));
    let pending = Pending {
        plan,
        doc_id: doc.id(),
        rev_before: doc.revision(),
        undo_before: doc.undo_count(),
        trims_before: doc.history_trimmed().0,
        before_ids,
    };
    let rec = app.automation.recorder.as_mut()?;
    rec.sync(&app.doc);
    rec.nested = true;
    Some(pending)
}

/// 操作の後に呼ぶ。
pub fn after(app: &mut AppState, pending: Option<Pending>) {
    let Some(p) = pending else { return };
    let doc = &app.doc;
    let Some(rec) = app.automation.recorder.as_mut() else {
        return;
    };
    rec.nested = false;
    // ほかの文書での操作（記録しない。変えたなら印）
    if p.doc_id != rec.doc_id {
        if doc.id() == p.doc_id && doc.revision() != p.rev_before {
            rec.skipped = true;
        }
        return;
    }
    // 文書を替えた操作（セットの切り替えなど）か、何も変えなかった（断った・同じ値）
    if doc.id() != p.doc_id || doc.revision() == p.rev_before {
        return;
    }
    let now = doc.revision();
    rec.seen = now;
    match p.plan {
        Plan::Ui | Plan::Skip => rec.skipped = true,
        Plan::Undo => {
            if rec.top == Some(p.rev_before) {
                let linked = rec.redo_top == Some(p.rev_before);
                if let Some(entry) = rec.pop(now) {
                    rec.undone.push((entry, linked));
                    rec.redo_top = Some(now);
                }
            } else {
                // 記録の外の段（描いた線など）を取り消した
                rec.top = None;
                rec.redo_top = None;
            }
        }
        Plan::Redo => {
            if rec.redo_top == Some(p.rev_before) {
                if let Some((entry, linked)) = rec.undone.pop() {
                    rec.entries.push(entry);
                    rec.top = Some(now);
                    rec.redo_top = linked.then_some(now);
                }
            } else {
                rec.top = None;
                rec.redo_top = None;
            }
        }
        Plan::Record {
            commands,
            creates,
            merge,
        } => {
            let created = match (creates, &p.before_ids) {
                (Some(Made::Layer), Some((layers, _))) => layer_ids(doc)
                    .into_iter()
                    .filter(|id| !layers.contains(id))
                    .map(|id| (Made::Layer, id))
                    .collect(),
                (Some(Made::Effect), Some((_, effects))) => effect_ids(doc)
                    .into_iter()
                    .filter(|id| !effects.contains(id))
                    .map(|id| (Made::Effect, id))
                    .collect(),
                _ => Vec::new(),
            };
            // 作ったはずの物が 1 つに決まらなければ、あとの指し方が狂うので記録しない
            if creates.is_some() && created.len() != 1 {
                rec.skipped = true;
                rec.top = None;
                return;
            }
            // core が同じ段にまとめた変更（スライダーのドラッグの続き）は、命令もまとめる
            let merged_in_core = doc.undo_count() == p.undo_before
                && doc.history_trimmed().0 == p.trims_before
                && rec.top == Some(p.rev_before);
            if let (true, Some(key)) = (merged_in_core, merge) {
                if let Some(last) = rec.entries.last_mut().filter(|e| e.merge == Some(key)) {
                    for (old, new) in last.commands.iter_mut().zip(commands) {
                        merge_into(old, new);
                    }
                    last.rev_after = now;
                    rec.top = Some(now);
                    return;
                }
            }
            if rec.len() + commands.len() > MAX_COMMANDS {
                rec.skipped = true;
                rec.top = None;
                return;
            }
            rec.undone.clear();
            rec.redo_top = None;
            rec.entries.push(Entry {
                commands,
                created,
                rev_before: p.rev_before,
                rev_after: now,
                merge,
            });
            rec.top = Some(now);
        }
    }
}

/// スライダーのドラッグを Esc で止めた（core がまとめた段を捨てた）: 最後の段がそのドラッグなら外す。
pub fn drag_cancelled(app: &mut AppState, rev_before: u64) {
    let now = app.doc.revision();
    let doc_id = app.doc.id();
    let Some(rec) = app.automation.recorder.as_mut() else {
        return;
    };
    if doc_id != rec.doc_id {
        return;
    }
    rec.seen = now;
    let ours = rec.top == Some(rev_before) && rec.entries.last().is_some_and(|e| e.merge.is_some());
    if ours {
        rec.pop(now);
    } else {
        rec.top = None;
    }
    rec.redo_top = None;
}

fn layer_ids(doc: &Document) -> HashSet<u128> {
    doc.layers().iter().map(|l| l.id().0).collect()
}

fn effect_ids(doc: &Document) -> HashSet<u128> {
    doc.layers()
        .iter()
        .flat_map(|l| {
            l.filters()
                .iter()
                .chain(l.mask().into_iter().flat_map(|m| m.filters().iter()))
                .map(|f| f.id().0)
        })
        .collect()
}

// ───────── 相手の指し方 ─────────

/// 層を指す文字列（`$selected`・`$created:<n>`・名前が 1 つに決まるときだけ名前）。指せなければ None。
fn layer_ref(app: &AppState, rec: &Recorder, id: LayerId) -> Option<String> {
    if rec.start_selected == Some(id) {
        return Some(SELECTED.to_owned());
    }
    if let Some(n) = rec.created_number(Made::Layer, id.0) {
        return Some(format!("{CREATED_PREFIX}{n}"));
    }
    let name = app.doc.layer(id)?.name();
    let unique = app.doc.layers().iter().filter(|l| l.name() == name).count() == 1;
    // 相対の指し方・ID と読める名前は、名前として引かれないので使わない
    let plain = parse_relative(name).is_ok_and(|r| r.is_none()) && parse_id(name).is_none();
    (unique && plain).then(|| name.to_owned())
}

/// 効果を指す文字列（記録の中で作った効果だけ）。
fn effect_ref(rec: &Recorder, id: FilterId) -> Option<String> {
    rec.created_number(Made::Effect, id.0)
        .map(|n| format!("{CREATED_PREFIX}{n}"))
}

// ───────── 写し方 ─────────

fn record(commands: Vec<Command>, merge: Option<MergeKey>) -> Plan {
    Plan::Record {
        commands,
        creates: None,
        merge,
    }
}

fn layer_set(layer: String) -> LayerSetArgs {
    LayerSetArgs {
        set: None,
        layer,
        name: None,
        visible: None,
        opacity: None,
        blend_mode: None,
        clipping: None,
        locks: None,
        channels: BTreeMap::new(),
        fill: BTreeMap::new(),
        adjustment: None,
        points: BTreeMap::new(),
    }
}

fn mask_set(layer: String) -> MaskSetArgs {
    MaskSetArgs {
        set: None,
        layer,
        enabled: None,
        inverted: None,
        density: None,
    }
}

fn effect_set(layer: String, effect: String) -> EffectSetArgs {
    EffectSetArgs {
        set: None,
        layer,
        effect,
        kind: None,
        values: BTreeMap::new(),
        channels: Vec::new(),
        strength: None,
        enabled: None,
        index: None,
    }
}

fn layer_add(kind: NewLayerKind, name: String, above: Option<String>) -> LayerAddArgs {
    LayerAddArgs {
        set: None,
        kind,
        name: Some(name),
        above,
        fill: BTreeMap::new(),
        adjustment: None,
        channels: Vec::new(),
    }
}

fn values_map(
    pairs: Vec<(&'static str, yolu_core::effects::catalog::ParamValue)>,
) -> BTreeMap<String, Value> {
    pairs
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.into()))
        .collect()
}

fn param_map(
    values: &BTreeMap<String, Value>,
) -> BTreeMap<String, yolu_core::effects::catalog::ParamValue> {
    values
        .iter()
        .map(|(k, v)| (k.clone(), v.clone().into()))
        .collect()
}

/// `now` の値のうち、`base` と違う物。
fn changed_values(
    base: Vec<(&'static str, yolu_core::effects::catalog::ParamValue)>,
    now: Vec<(&'static str, yolu_core::effects::catalog::ParamValue)>,
) -> BTreeMap<String, Value> {
    let base = values_map(base);
    values_map(now)
        .into_iter()
        .filter(|(k, v)| base.get(k) != Some(v))
        .collect()
}

/// 効果の設定を、種類と値（既定と違う物）で表す。値だけで同じ設定に戻せなければ None（値の欄に無い中身 — ランプ・曲線・焼いたマップの
/// 固定など — が既定でない設定。足せない種類は `from_catalog` が断る）。
fn effect_spec(settings: &EffectSettings) -> Option<EffectSpec> {
    let kind = settings.kind_id();
    let defaults = EffectSettings::from_catalog(kind, &BTreeMap::new()).ok()?;
    let values = changed_values(defaults.catalog_values(), settings.catalog_values());
    let back = EffectSettings::from_catalog(kind, &param_map(&values)).ok()?;
    (back == *settings).then(|| EffectSpec {
        kind: kind.to_owned(),
        values,
    })
}

/// 調整の設定を、種類と値（既定と違う物）で表す。値だけで同じ設定に戻せなければ None。
fn adjustment_spec(settings: &AdjustmentSettings) -> Option<EffectSpec> {
    let kind = settings.kind_id();
    let defaults = AdjustmentSettings::from_catalog(kind, &BTreeMap::new()).ok()?;
    let values = changed_values(defaults.catalog_values(), settings.catalog_values());
    let back = AdjustmentSettings::from_catalog(kind, &param_map(&values)).ok()?;
    (back == *settings).then(|| EffectSpec {
        kind: kind.to_owned(),
        values,
    })
}

/// 選んでいる層（まだある物）。
fn selected(app: &AppState) -> Option<LayerId> {
    app.selected_layer.filter(|id| app.doc.layer(*id).is_some())
}

/// 選んでいる層の上に足す、の `above`（選んでいなければ一番上で None）。指せなければ Err。
fn above(app: &AppState, rec: &Recorder) -> Result<Option<String>, ()> {
    match selected(app) {
        None => Ok(None),
        Some(id) => layer_ref(app, rec, id).map(Some).ok_or(()),
    }
}

fn creating(command: Command, made: Made) -> Plan {
    Plan::Record {
        commands: vec![command],
        creates: Some(made),
        merge: None,
    }
}

/// 操作を、同じ結果になる命令へ写す（写せる操作の表）。
fn plan(app: &AppState, rec: &Recorder, action: &Action) -> Plan {
    let lref = |id: LayerId| layer_ref(app, rec, id);
    let doc = &app.doc;
    let one = |command: Option<Command>, merge: Option<MergeKey>| match command {
        Some(c) => record(vec![c], merge),
        None => Plan::Skip,
    };
    match action {
        Action::Undo => Plan::Undo,
        Action::Redo => Plan::Redo,
        Action::NewLayer => match above(app, rec) {
            Ok(above) => creating(
                Command::LayerAdd(layer_add(
                    NewLayerKind::Paint,
                    app.layer_name_for_new(),
                    above,
                )),
                Made::Layer,
            ),
            Err(()) => Plan::Skip,
        },
        Action::DeleteLayer => {
            if app.has_multiple_layers_selected() {
                return Plan::Skip;
            }
            one(
                selected(app).and_then(lref).map(|layer| {
                    Command::LayerDelete(LayerDeleteArgs {
                        set: None,
                        layer,
                        confirm: true,
                    })
                }),
                None,
            )
        }
        Action::LayerUp | Action::LayerDown => {
            if app.has_multiple_layers_selected() {
                return Plan::Skip;
            }
            let Some(id) = selected(app) else {
                return Plan::Ui;
            };
            let parent = doc.layer(id).and_then(|l| l.parent());
            let siblings = doc.children_of(parent).unwrap_or_default();
            let Some(i) = siblings.iter().position(|v| *v == id) else {
                return Plan::Skip;
            };
            let to = if *action == Action::LayerUp {
                i + 1
            } else {
                i.wrapping_sub(1)
            };
            if to >= siblings.len() {
                return Plan::Ui;
            }
            one(
                lref(id).map(|layer| {
                    Command::LayerMove(LayerMoveArgs {
                        set: None,
                        layer,
                        parent: None,
                        to_root: false,
                        index: Some(to),
                    })
                }),
                None,
            )
        }
        Action::ToggleVisible(id) => one(
            doc.layer(*id).and_then(|l| {
                let visible = !l.visible();
                lref(*id).map(|layer| {
                    Command::LayerSet(LayerSetArgs {
                        visible: Some(visible),
                        ..layer_set(layer)
                    })
                })
            }),
            None,
        ),
        Action::SetBlend(id, mode) => one(
            lref(*id).map(|layer| {
                Command::LayerSet(LayerSetArgs {
                    blend_mode: Some(mode.name().to_owned()),
                    ..layer_set(layer)
                })
            }),
            None,
        ),
        Action::M2(edit) => plan_edit(app, rec, edit),
        Action::Fx(op) => plan_fx(app, rec, op),
        // 文書を変えるが、命令の無い操作
        Action::Mat(_)
        | Action::Region(_)
        | Action::Shelf(_)
        | Action::Sel(_)
        | Action::Path(_)
        | Action::Fill(_)
        | Action::LayerMenu(_)
        | Action::Gradient(_)
        | Action::Clip(_)
        | Action::Look(_)
        | Action::Psd(_)
        | Action::Project(_)
        | Action::Recovery(_)
        | Action::Bake(_) => Plan::Skip,
        // 画面だけの操作（文書を替える操作は、記録の外の文書になるので記録しない）
        _ => Plan::Ui,
    }
}

fn plan_edit(app: &AppState, rec: &Recorder, edit: &Edit) -> Plan {
    let lref = |id: LayerId| layer_ref(app, rec, id);
    let doc = &app.doc;
    let set = |id: LayerId, f: &dyn Fn(LayerSetArgs) -> LayerSetArgs, merge: Option<MergeKey>| {
        match lref(id) {
            Some(layer) => record(vec![Command::LayerSet(f(layer_set(layer)))], merge),
            None => Plan::Skip,
        }
    };
    let mask =
        |id: LayerId, f: &dyn Fn(MaskSetArgs) -> MaskSetArgs, merge: Option<MergeKey>| match lref(
            id,
        ) {
            Some(layer) => record(vec![Command::MaskSet(f(mask_set(layer)))], merge),
            None => Plan::Skip,
        };
    match edit {
        Edit::NewGroup => match above(app, rec) {
            Ok(above) => creating(
                Command::LayerAdd(layer_add(
                    NewLayerKind::Group,
                    app.new_layer_name(LayerKind::Group),
                    above,
                )),
                Made::Layer,
            ),
            Err(()) => Plan::Skip,
        },
        Edit::NewFill => match above(app, rec) {
            Ok(above) => {
                let value = crate::m2::fill_from_color(app.color.main);
                let mut args = layer_add(
                    NewLayerKind::Fill,
                    app.new_layer_name(LayerKind::Fill),
                    above,
                );
                args.fill
                    .insert(channel_name(doc, app.m2.paint_channel), format_color(value));
                creating(Command::LayerAdd(args), Made::Layer)
            }
            Err(()) => Plan::Skip,
        },
        Edit::NewAdjustment(kind) => {
            let (Ok(above), Some(spec)) = (above(app, rec), adjustment_spec(&kind.settings()))
            else {
                return Plan::Skip;
            };
            let mut args = layer_add(
                NewLayerKind::Adjustment,
                kind.name(app.lang).to_owned(),
                above,
            );
            args.adjustment = Some(spec);
            creating(Command::LayerAdd(args), Made::Layer)
        }
        Edit::Move {
            id,
            parent,
            position,
        } => {
            let parent_ref = match parent {
                None => None,
                Some(p) => match lref(*p) {
                    Some(r) => Some(r),
                    None => return Plan::Skip,
                },
            };
            match lref(*id) {
                Some(layer) => record(
                    vec![Command::LayerMove(LayerMoveArgs {
                        set: None,
                        layer,
                        to_root: parent_ref.is_none(),
                        parent: parent_ref,
                        index: Some(*position),
                    })],
                    None,
                ),
                None => Plan::Skip,
            }
        }
        Edit::Clipping(id, v) => set(
            *id,
            &|a| LayerSetArgs {
                clipping: Some(*v),
                ..a
            },
            None,
        ),
        Edit::Opacity {
            id,
            channel: None,
            value,
        } => set(
            *id,
            &|a| LayerSetArgs {
                opacity: Some(*value),
                ..a
            },
            Some(MergeKey::Opacity(*id)),
        ),
        Edit::BlendMode {
            id,
            channel: None,
            mode,
        } => set(
            *id,
            &|a| LayerSetArgs {
                blend_mode: Some(mode.name().to_owned()),
                ..a
            },
            None,
        ),
        Edit::ChannelEnabled {
            id,
            channel,
            enabled,
        } => {
            let name = channel_name(doc, *channel);
            set(
                *id,
                &|mut a| {
                    a.channels.insert(name.clone(), *enabled);
                    a
                },
                None,
            )
        }
        Edit::AddMask(id) => match lref(*id) {
            Some(layer) => record(
                vec![Command::MaskAdd(MaskAddArgs { set: None, layer })],
                None,
            ),
            None => Plan::Skip,
        },
        Edit::RemoveMask(id) => match lref(*id) {
            Some(layer) => record(
                vec![Command::MaskDelete(MaskDeleteArgs {
                    set: None,
                    layer,
                    confirm: true,
                })],
                None,
            ),
            None => Plan::Skip,
        },
        Edit::MaskEnabled(id, v) => mask(
            *id,
            &|a| MaskSetArgs {
                enabled: Some(*v),
                ..a
            },
            None,
        ),
        Edit::MaskInverted(id, v) => mask(
            *id,
            &|a| MaskSetArgs {
                inverted: Some(*v),
                ..a
            },
            None,
        ),
        Edit::MaskDensity(id, v) => mask(
            *id,
            &|a| MaskSetArgs {
                density: Some(*v),
                ..a
            },
            Some(MergeKey::MaskDensity(*id)),
        ),
        Edit::FillValue { id, channel, value } => {
            let name = channel_name(doc, *channel);
            let color = value.map(format_color);
            set(
                *id,
                &|mut a| {
                    a.fill.insert(name.clone(), color.clone());
                    a
                },
                Some(MergeKey::Fill(*id, *channel)),
            )
        }
        Edit::Adjust { id, settings } => {
            let Some(current) = doc.layer(*id).and_then(|l| l.adjustment()) else {
                return Plan::Skip;
            };
            let spec = if current.kind_id() == settings.kind_id() {
                // 同じ種類: 今の値と違う欄だけ（命令は今の値に重ねる。値の欄に無い中身が変わったなら、重ねても同じにならないので写さない）
                let values = changed_values(current.catalog_values(), settings.catalog_values());
                match current.with_catalog_values(&param_map(&values)) {
                    Ok(back) if back == *settings => EffectSpec {
                        kind: settings.kind_id().to_owned(),
                        values,
                    },
                    _ => return Plan::Skip,
                }
            } else {
                match adjustment_spec(settings) {
                    Some(spec) => spec,
                    None => return Plan::Skip,
                }
            };
            set(
                *id,
                &|a| LayerSetArgs {
                    adjustment: Some(spec.clone()),
                    ..a
                },
                Some(MergeKey::Adjust(*id)),
            )
        }
        Edit::ToggleSelectedVisible => {
            let ids = app.selected_layers();
            match ids.as_slice() {
                [id] => match doc.layer(*id) {
                    Some(l) => {
                        let visible = !l.visible();
                        set(
                            *id,
                            &|a| LayerSetArgs {
                                visible: Some(visible),
                                ..a
                            },
                            None,
                        )
                    }
                    None => Plan::Skip,
                },
                _ => Plan::Skip,
            }
        }
        Edit::Lock { ids, flag, on } => {
            let patch = locks_patch(*flag, *on);
            let mut commands = Vec::new();
            for id in ids {
                match lref(*id) {
                    Some(layer) => commands.push(Command::LayerSet(LayerSetArgs {
                        locks: Some(patch.clone()),
                        ..layer_set(layer)
                    })),
                    None => return Plan::Skip,
                }
            }
            record(commands, None)
        }
        // 命令の無い操作（チャンネルごとの合成・複製・グループ化・結合・変形・チャンネルの構成・ノーマルの設定・複数のレイヤーの移動）
        _ => Plan::Skip,
    }
}

/// ロックの印 1 つ（全部を外すときは 4 つ全部）を、命令のロックの欄へ。
fn locks_patch(flag: LayerLocks, on: bool) -> LocksPatch {
    let has = |f: LayerLocks| (flag.bits() & f.bits() != 0).then_some(on);
    LocksPatch {
        transparency: has(LayerLocks::TRANSPARENCY),
        pixels: has(LayerLocks::PIXELS),
        position: has(LayerLocks::POSITION),
        all: has(LayerLocks::ALL),
    }
}

fn plan_fx(app: &AppState, rec: &Recorder, op: &FxOp) -> Plan {
    let lref = |id: LayerId| layer_ref(app, rec, id);
    let doc = &app.doc;
    let target_of = |t: FilterTarget| match t {
        FilterTarget::Content => EffectTarget::Content,
        FilterTarget::Mask => EffectTarget::Mask,
    };
    let add = |target: FilterTarget, settings: EffectSettings| {
        let (Some(layer), Some(spec)) = (selected(app).and_then(lref), effect_spec(&settings))
        else {
            return Plan::Skip;
        };
        let channels = match target {
            FilterTarget::Content => vec![channel_name(doc, app.m2.paint_channel)],
            FilterTarget::Mask => Vec::new(),
        };
        creating(
            Command::EffectAdd(EffectAddArgs {
                set: None,
                layer,
                target: target_of(target),
                kind: spec.kind,
                values: spec.values,
                channels,
                strength: None,
                enabled: None,
                index: None,
            }),
            Made::Effect,
        )
    };
    let change = |layer: LayerId,
                  id: FilterId,
                  f: &dyn Fn(EffectSetArgs) -> EffectSetArgs,
                  merge: Option<MergeKey>| {
        match (lref(layer), effect_ref(rec, id)) {
            (Some(l), Some(e)) => record(vec![Command::EffectSet(f(effect_set(l, e)))], merge),
            _ => Plan::Skip,
        }
    };
    match op {
        FxOp::AddFilter { target, kind } => add(*target, kind.settings()),
        FxOp::AddGenerator { target, kind } => {
            // Anchor は読む Anchor（ID）を持つので、値だけでは写せない
            if *kind == yolu_core::generator::Kind::Anchor {
                return Plan::Skip;
            }
            add(
                *target,
                EffectSettings::generator(yolu_core::generator::Settings::new(*kind)),
            )
        }
        FxOp::SetEnabled { layer, id, enabled } => change(
            *layer,
            *id,
            &|a| EffectSetArgs {
                enabled: Some(*enabled),
                ..a
            },
            None,
        ),
        FxOp::Move { layer, id, index } => change(
            *layer,
            *id,
            &|a| EffectSetArgs {
                index: Some(*index),
                ..a
            },
            None,
        ),
        FxOp::Remove { layer, id } => match (lref(*layer), effect_ref(rec, *id)) {
            (Some(layer), Some(effect)) => record(
                vec![Command::EffectDelete(EffectDeleteArgs {
                    set: None,
                    layer,
                    effect,
                    confirm: true,
                })],
                None,
            ),
            _ => Plan::Skip,
        },
        FxOp::SetSettings {
            layer,
            id,
            settings,
            ..
        } => {
            let Some(current) = doc.find_filter(*id).map(|(_, fe, _)| fe.settings().clone()) else {
                return Plan::Skip;
            };
            let (kind, values) = if current.kind_id() == settings.kind_id() {
                let values = changed_values(current.catalog_values(), settings.catalog_values());
                match current.with_catalog_values(&param_map(&values)) {
                    Ok(back) if back == *settings => (None, values),
                    _ => return Plan::Skip,
                }
            } else {
                match effect_spec(settings) {
                    Some(spec) => (Some(spec.kind), spec.values),
                    None => return Plan::Skip,
                }
            };
            change(
                *layer,
                *id,
                &|a| EffectSetArgs {
                    kind: kind.clone(),
                    values: values.clone(),
                    ..a
                },
                Some(MergeKey::Settings(*id)),
            )
        }
        FxOp::SetStrength {
            layer,
            id,
            strength,
            ..
        } => change(
            *layer,
            *id,
            &|a| EffectSetArgs {
                strength: Some(*strength),
                ..a
            },
            Some(MergeKey::Strength(*id)),
        ),
        FxOp::SetChannels {
            layer,
            id,
            channels,
        } => {
            if channels.is_empty() {
                return Plan::Skip;
            }
            let names: Vec<String> = channels.iter().map(|c| channel_name(doc, *c)).collect();
            change(
                *layer,
                *id,
                &|a| EffectSetArgs {
                    channels: names.clone(),
                    ..a
                },
                None,
            )
        }
        // 継ぎ目をまたぐ読みの入り切り・画像の段が読む画像の差し替えは、命令が無い（画像は文書のアセットの ID で、別の文書に無い）
        FxOp::SetFilterSeams(_) | FxOp::SetImage { .. } => Plan::Skip,
        // 行を選ぶ・Anchor を見る（文書を変えない）
        FxOp::SelectFilter { .. }
        | FxOp::SelectAnchor(_)
        | FxOp::Deselect
        | FxOp::GoToAnchor(_)
        | FxOp::PickIdColors { .. } => Plan::Ui,
        // Anchor の置き場・名前・読む物（命令が無い）
        FxOp::AddAnchor { .. }
        | FxOp::RemoveAnchor(_)
        | FxOp::RenameAnchor { .. }
        | FxOp::SetAnchorRef { .. } => Plan::Skip,
    }
}

/// まとめたドラッグの続きの命令を、前の命令へ重ねる（同じ相手・同じ欄。値は後の物）。
fn merge_into(old: &mut Command, new: Command) {
    match (old, new) {
        (Command::LayerSet(a), Command::LayerSet(b)) => {
            a.opacity = b.opacity.or(a.opacity);
            a.fill.extend(b.fill);
            match (&mut a.adjustment, b.adjustment) {
                (Some(x), Some(y)) if x.kind == y.kind => x.values.extend(y.values),
                (slot, Some(y)) => *slot = Some(y),
                _ => {}
            }
        }
        (Command::MaskSet(a), Command::MaskSet(b)) => {
            a.density = b.density.or(a.density);
        }
        (Command::EffectSet(a), Command::EffectSet(b)) => {
            a.strength = b.strength.or(a.strength);
            if b.kind.is_some() {
                a.kind = b.kind;
                a.values = b.values;
            } else {
                a.values.extend(b.values);
            }
        }
        (old, new) => *old = new,
    }
}
