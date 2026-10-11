//! 効果のレイヤー（フィルターのスタック・Generator・Anchor）の画面の状態と操作（Unity 版の `TexturePaintWindow.Filters / Generators / Anchors`）。
//!
//! - 一覧: レイヤーの行の下に、対象の側のスタックだけを字下げした子の行で並べる（`panels::effect_rows`）。選んだレイヤーでマスクが対象なら
//!   マスクの Anchor・マスクの効果の段、それ以外（選んでいないレイヤーも）はレイヤーの Anchor・画素の効果の段（どちらも上が後に掛かる）。
//!   押すとその段を選び（`FxState::selected`。そのスタックの側が対象になる）、プロパティの欄にその段の設定が出る（`panels::effect_props`）。
//! - 操作: どれも `Action::Fx(FxOp)` を通り、core の編集の口（`add_filter` ほか）を 1 つ呼ぶ。1 つが 1 回の Undo で、スライダーのドラッグは
//!   core がまとめる。ロック・段の数・作業メモリの上限などの断りは core が決め、ここは理由を画面の言語で出すだけ。
//! - 入力: Generator と塗りつぶしの画像が読むメッシュマップ・モデルのルート・画像は文書の外のもので、`inputs` が毎フレーム
//!   セットごとに文書へ渡す（焼き直し・モデルの差し替えで読むレイヤーだけが描き直される）。入力がそろわない効果を持つ .ylp は読むだけにする。
//! - Anchor を読む Generator が使えなくなる操作（並べ替え・削除・結合）のあとは、新しく使えなくなった参照を状態の帯で知らせる。
//!   編集そのものは断らない（取り消せば戻る）。

pub mod inputs;
pub mod menu;
pub mod names;

use std::collections::HashSet;

use yolu_core::generator::{self, anchor::ReadMode, Kind};
use yolu_core::{
    AnchorId, AnchorInfo, AnchorIssueKind, AnchorPlacement, Channel, CoreError, Document,
    EffectSettings, FilterEffect, FilterId, FilterSpec, FilterTarget, ImageId, LayerId,
};

pub use names::FilterKind;

use crate::lang::Lang;
use crate::notice::Source;
use crate::state::AppState;

/// 選んでいる行（レイヤーの行の下の子の行）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selected {
    Filter { layer: LayerId, id: FilterId },
    Anchor { id: AnchorId },
}

/// 効果の画面の状態。
#[derive(Default)]
pub struct FxState {
    pub selected: Option<Selected>,
    /// 「ID の色」の Generator の色を、ID マップから選んでいる（押した所の ID の色をその段に足す）。段のレイヤーと ID。
    pub id_pick: Option<(LayerId, FilterId)>,
    pub inputs: inputs::InputsState,
    /// 前に見た文書の版と、そのとき使えなくなっていた Anchor の参照（新しく使えなくなったものだけを知らせる）。
    issues: AnchorIssuesSeen,
}

#[derive(Default)]
struct AnchorIssuesSeen {
    doc: Option<(u128, u64)>,
    known: HashSet<FilterId>,
}

/// 文書の効果を変える操作と、行を選ぶ操作。
#[derive(Clone, Debug, PartialEq)]
pub enum FxOp {
    /// 選んでいるレイヤーのスタックにフィルターを足す（画素なら描くチャンネルだけに掛かる）。
    AddFilter {
        target: FilterTarget,
        kind: FilterKind,
    },
    AddGenerator {
        target: FilterTarget,
        kind: Kind,
    },
    SetEnabled {
        layer: LayerId,
        id: FilterId,
        enabled: bool,
    },
    /// スタックの中の位置（0 が最初に当たる）へ。
    Move {
        layer: LayerId,
        id: FilterId,
        index: usize,
    },
    Remove {
        layer: LayerId,
        id: FilterId,
    },
    /// 設定。`coalesce` ならスライダーのドラッグの続きとして、core が 1 回の Undo にまとめる。
    SetSettings {
        layer: LayerId,
        id: FilterId,
        settings: EffectSettings,
        coalesce: bool,
    },
    SetStrength {
        layer: LayerId,
        id: FilterId,
        strength: f64,
        coalesce: bool,
    },
    SetChannels {
        layer: LayerId,
        id: FilterId,
        channels: Vec<Channel>,
    },
    AddAnchor {
        layer: LayerId,
        placement: AnchorPlacement,
    },
    RemoveAnchor(AnchorId),
    RenameAnchor {
        id: AnchorId,
        name: String,
    },
    /// Anchor の Generator が読むもの（Anchor・チャンネル・読み方）を選ぶ。
    SetAnchorRef {
        layer: LayerId,
        filter: FilterId,
        anchor: Option<AnchorId>,
        channel: Channel,
        read: ReadMode,
    },
    SelectFilter {
        layer: LayerId,
        id: FilterId,
    },
    SelectAnchor(AnchorId),
    Deselect,
    /// Anchor を置いたレイヤー（マスクの Anchor ならそのマスク）へ移る。
    GoToAnchor(AnchorId),
    /// 「ID の色」の Generator の色を、ID マップから選び始める・やめる（2D のキャンバスか 3D ビューを押すと、その所の ID の色を足す。
    /// Ctrl を押していれば外す）。「ID の色で選択」のツールの入力をそのまま使う。
    PickIdColors {
        layer: LayerId,
        id: FilterId,
        on: bool,
    },
    /// レイヤーのフィルターが UV の継ぎ目をまたぐか（文書の設定。テクスチャセットのすべてのフィルターに効く）。
    SetFilterSeams(bool),
    /// 画像の段が読むアセットの画像を差す・外す（差す前に復号して、読めなければ理由を添えて断る）。
    SetImage {
        layer: LayerId,
        id: FilterId,
        image: Option<ImageId>,
    },
}

impl FxOp {
    /// ツールを替える操作か（描いている間は断る。ツールを替えると途中のストロークの前提が変わる）。
    pub fn changes_tool(&self) -> bool {
        matches!(self, FxOp::PickIdColors { on: true, .. })
    }

    /// 文書を変える操作か（読むだけのセットでは断る。行を選ぶだけの操作は断らない）。
    pub fn edits_document(&self) -> bool {
        !matches!(
            self,
            FxOp::SelectFilter { .. }
                | FxOp::SelectAnchor(_)
                | FxOp::Deselect
                | FxOp::GoToAnchor(_)
                | FxOp::PickIdColors { .. }
        )
    }
}

impl FxState {
    /// 選んでいる段（レイヤー・マスクにまだあるもの）。
    pub fn filter<'a>(
        &self,
        doc: &'a Document,
    ) -> Option<(LayerId, &'a FilterEffect, FilterTarget)> {
        match self.selected {
            Some(Selected::Filter { layer, id }) => {
                doc.find_filter(id).filter(|(l, _, _)| *l == layer)
            }
            _ => None,
        }
    }

    /// 選んでいる Anchor（まだあるもの）。
    pub fn anchor<'a>(&self, doc: &'a Document) -> Option<AnchorInfo<'a>> {
        match self.selected {
            Some(Selected::Anchor { id }) => doc.find_anchor(id),
            _ => None,
        }
    }

    /// 選んでいる行が、マスクのスタックの行か（まだあるもの）。
    pub fn in_mask(&self, doc: &Document) -> bool {
        self.filter(doc)
            .is_some_and(|(_, _, target)| target == FilterTarget::Mask)
            || self
                .anchor(doc)
                .is_some_and(|info| info.placement == AnchorPlacement::Mask)
    }
}

/// マスクが対象のレイヤー（そのレイヤーを選んでマスクに描いているあいだだけ。マスクの無いレイヤーは対象にならない）。レイヤーの行の下の効果の行・
/// 効果の追加の先・レイヤーとマスクのサムネイルの青い枠が、同じこの結果を見る。
pub fn mask_target(app: &AppState) -> Option<LayerId> {
    let id = app.selected_layer?;
    let has_mask = app.doc.layer(id)?.mask().is_some();
    (app.m2.edit_mask && has_mask).then_some(id)
}

/// プロパティの欄に出す効果の行を選んでいるか（選んだレイヤーが今のレイヤーで、選んだ行のスタックが今の対象の側）。マスクの効果の行を選ぶと
/// マスクが対象になる（`select_effect`）ので、レイヤーの画素が対象のあいだはマスクの効果の行を選んだ状態は残らない。
pub fn props_visible(app: &AppState) -> bool {
    let mask_side = app.m2.edit_mask;
    if let Some((layer, _, target)) = app.fx.filter(&app.doc) {
        return app.selected_layer == Some(layer) && (target == FilterTarget::Mask) == mask_side;
    }
    if let Some(info) = app.fx.anchor(&app.doc) {
        return app.selected_layer == Some(info.layer)
            && (info.placement == AnchorPlacement::Mask) == mask_side;
    }
    false
}

/// 新しい Anchor の名前（レイヤーの名前。同じ名前があれば番号を足す）。
fn unique_anchor_name(doc: &Document, base: &str, lang: Lang) -> String {
    let base = base.trim();
    let mut name = if base.is_empty() {
        lang.pick("アンカー", "Anchor").to_owned()
    } else {
        base.to_owned()
    };
    // 名前の上限（UTF-16 で 128）に番号の分を空ける
    while name.encode_utf16().count() > yolu_core::Anchor::MAX_NAME_UTF16 - 4 {
        name.pop();
    }
    let taken: HashSet<String> = doc
        .anchors()
        .iter()
        .map(|a| a.anchor.name().to_owned())
        .collect();
    if !taken.contains(&name) {
        return name;
    }
    (2..)
        .map(|i| format!("{name} {i}"))
        .find(|n| !taken.contains(n))
        .expect("どこかで空く")
}

impl AppState {
    /// 効果の操作を当てる（描いている間の文書の変更は断る。断られたら何も変えず、理由を状態の帯へ）。
    pub fn fx_apply(&mut self, op: FxOp) {
        if (op.edits_document() || op.changes_tool()) && self.is_stroking() {
            self.refuse(
                Source::Effect,
                crate::lang::refusals::during_stroke(self.lang),
            );
            return;
        }
        let revision = self.doc.revision();
        match self.fx_run(op) {
            Ok(Some(text)) => self.info(Source::Effect, text),
            Ok(None) => {}
            Err(e) => self.notify(
                crate::notice::Kind::of_core(&e),
                Source::Effect,
                self.lang.core_error(&e),
            ),
        }
        if self.doc.revision() != revision {
            self.modified = true;
        }
        self.ensure_selection();
    }

    fn selected_layer_or_refuse(&self) -> Result<LayerId, CoreError> {
        self.selected_layer
            .filter(|id| self.doc.layer(*id).is_some())
            .ok_or(CoreError::LayerNotFound)
    }

    /// 段を選ぶ（そのレイヤーを選び、その段のスタックの側を対象にする。画素の段ならレイヤーの画素、マスクの段ならマスク）。対象の側の効果の行だけが
    /// 一覧に出るので、選んだ行は選んだ直後も見えたまま残る。
    pub fn select_effect(&mut self, layer: LayerId, id: FilterId) {
        let mask = self
            .doc
            .find_filter(id)
            .is_some_and(|(_, _, target)| target == FilterTarget::Mask);
        self.selected_layer = Some(layer);
        self.set_edit_mask(mask);
        self.fx.selected = Some(Selected::Filter { layer, id });
    }

    pub fn select_anchor(&mut self, id: AnchorId) {
        if let Some(info) = self.doc.find_anchor(id) {
            let (layer, mask) = (info.layer, info.placement == AnchorPlacement::Mask);
            self.selected_layer = Some(layer);
            self.set_edit_mask(mask);
            self.fx.selected = Some(Selected::Anchor { id });
        }
    }

    fn fx_run(&mut self, op: FxOp) -> Result<Option<String>, CoreError> {
        let lang = self.lang;
        match op {
            FxOp::AddFilter { target, kind } => {
                let layer = self.selected_layer_or_refuse()?;
                let channel = self.m2.paint_channel;
                let spec = FilterSpec::new(kind.settings());
                let spec = match target {
                    FilterTarget::Content => spec.channels(&[channel]),
                    FilterTarget::Mask => spec,
                };
                let id = self.doc.add_filter(layer, target, spec)?;
                self.select_effect(layer, id);
                Ok(Some(self.added_text(kind.name(lang), target)))
            }
            FxOp::AddGenerator { target, kind } => {
                let layer = self.selected_layer_or_refuse()?;
                let channel = self.m2.paint_channel;
                let mut settings = generator::Settings::new(kind);
                if kind == Kind::Anchor {
                    // すぐ下の Anchor を読む（無ければ選ばないまま）
                    if let Some(nearest) = self.doc.anchors_readable_from(layer)?.last() {
                        settings.anchor.id = nearest.anchor.id().0;
                    }
                }
                let spec = FilterSpec::new(EffectSettings::generator(settings));
                let spec = match target {
                    FilterTarget::Content => spec.channels(&[channel]),
                    FilterTarget::Mask => spec,
                };
                let id = self.doc.add_filter(layer, target, spec)?;
                self.select_effect(layer, id);
                let mut text = self.added_text(names::generator_name(lang, kind), target);
                // 読むものが使えないなら、効かない理由を添える。文書へ渡す効果の入力は今の効果が読むマップだけなので、足したジェネレーターが
                // 読むマップは、毎フレームの同期を待つと渡っておらず「マップがありません」と誤る。先に渡す
                self.sync_effect_inputs();
                if let Ok(Some(why)) = self.doc.generator_inactive(layer, id) {
                    text += &format!(
                        " {}",
                        lang.with_reason(
                            lang.pick("効果がありません", "It has no effect"),
                            lang.inactive_reason(&why),
                        )
                    );
                }
                Ok(Some(text))
            }
            FxOp::SetEnabled { layer, id, enabled } => {
                self.doc.set_filter_enabled(layer, id, enabled)?;
                Ok(None)
            }
            FxOp::Move { layer, id, index } => {
                self.doc.move_filter(layer, id, index)?;
                Ok(None)
            }
            FxOp::Remove { layer, id } => {
                self.doc.remove_filter(layer, id)?;
                if self.fx.selected == Some(Selected::Filter { layer, id }) {
                    self.fx.selected = None;
                }
                Ok(None)
            }
            FxOp::SetSettings {
                layer,
                id,
                settings,
                coalesce,
            } => {
                self.doc
                    .set_filter_settings(layer, id, settings, coalesce)?;
                Ok(None)
            }
            FxOp::SetStrength {
                layer,
                id,
                strength,
                coalesce,
            } => {
                self.doc
                    .set_filter_strength(layer, id, strength, coalesce)?;
                Ok(None)
            }
            FxOp::SetChannels {
                layer,
                id,
                channels,
            } => {
                self.doc.set_filter_channels(layer, id, &channels)?;
                Ok(None)
            }
            FxOp::AddAnchor { layer, placement } => {
                let l = self.doc.layer(layer).ok_or(CoreError::LayerNotFound)?;
                let base = match placement {
                    AnchorPlacement::Mask => lang.pick(
                        format!("{}（マスク）", l.name()),
                        format!("{} (mask)", l.name()),
                    ),
                    AnchorPlacement::Layer => l.name().to_owned(),
                };
                let name = unique_anchor_name(&self.doc, &base, lang);
                let id = self.doc.add_anchor(layer, placement, Some(&name), None)?;
                self.select_anchor(id);
                Ok(Some(lang.pick(
                    format!("アンカー「{name}」を置きました。"),
                    format!("Put the anchor \"{name}\"."),
                )))
            }
            FxOp::RemoveAnchor(id) => {
                let name = self
                    .doc
                    .find_anchor(id)
                    .map(|a| a.anchor.name().to_owned())
                    .unwrap_or_default();
                self.doc.remove_anchor(id)?;
                if self.fx.selected == Some(Selected::Anchor { id }) {
                    self.fx.selected = None;
                }
                Ok(Some(lang.pick(
                    format!("アンカー「{name}」を外しました。"),
                    format!("Removed the anchor \"{name}\"."),
                )))
            }
            FxOp::RenameAnchor { id, name } => {
                self.doc.rename_anchor(id, name.trim())?;
                Ok(None)
            }
            FxOp::SetAnchorRef {
                layer,
                filter,
                anchor,
                channel,
                read,
            } => {
                self.doc
                    .set_generator_anchor(layer, filter, anchor, channel, read, false)?;
                Ok(None)
            }
            FxOp::SetImage { layer, id, image } => {
                let Some(mut g) = self
                    .doc
                    .find_filter(id)
                    .filter(|(l, _, _)| *l == layer)
                    .and_then(|(_, e, _)| e.settings().generator_settings().cloned())
                else {
                    return Ok(None);
                };
                if let Some(image) = image {
                    if let Err(why) =
                        self.use_shelf_image(&crate::fillfx::inputs::resource_id(image))
                    {
                        self.fail(Source::Effect, why);
                        return Ok(None);
                    }
                }
                g.image.image = image.map_or(0, |i| i.0);
                self.doc.end_coalescing();
                if let Err(e) =
                    self.doc
                        .set_filter_settings(layer, id, EffectSettings::generator(g), false)
                {
                    // 画像は先に復号して文書へ渡してある。差さなかった画像は手放す
                    if let Some(image) = image {
                        self.release_shelf_image(image);
                    }
                    return Err(e);
                }
                Ok(Some(
                    match image.and_then(|i| {
                        self.shelf
                            .get(&crate::fillfx::inputs::resource_id(i))
                            .map(|r| r.name.clone())
                    }) {
                        Some(name) => {
                            format!("{}: {name}", lang.pick("画像を差しました", "Image set"))
                        }
                        None => lang.pick("画像を外しました", "Image removed").into(),
                    },
                ))
            }
            FxOp::SelectFilter { layer, id } => {
                if self.doc.find_filter(id).is_some_and(|(l, _, _)| l == layer) {
                    self.select_effect(layer, id);
                }
                Ok(None)
            }
            FxOp::SelectAnchor(id) => {
                self.select_anchor(id);
                Ok(None)
            }
            FxOp::Deselect => {
                self.fx.selected = None;
                Ok(None)
            }
            FxOp::PickIdColors { layer, id, on } => {
                if !on {
                    self.fx.id_pick = None;
                    return Ok(None);
                }
                // 押した所の ID の色を読むのは「ID の色で選択」の入力（2D・3D の押す・強調）。ツールの切り替えは「ツールを選ぶ」と同じ口を通し
                // （移動・パスの途中のドラッグと選んだ点を捨てる）、効果の欄は開いたまま・選ぶ状態は残す
                if !self.switch_tool(crate::state::Tool::IdSelect, true) {
                    return Ok(None);
                }
                self.fx.id_pick = Some((layer, id));
                Ok(None)
            }
            FxOp::GoToAnchor(id) => {
                if let Some(info) = self.doc.find_anchor(id) {
                    let (layer, mask) = (info.layer, info.placement == AnchorPlacement::Mask);
                    self.selected_layer = Some(layer);
                    self.fx.selected = None;
                    self.set_edit_mask(mask);
                }
                Ok(None)
            }
            FxOp::SetFilterSeams(on) => {
                self.doc.set_filter_seams(on)?;
                Ok(None)
            }
        }
    }

    fn added_text(&self, name: &str, target: FilterTarget) -> String {
        let lang = self.lang;
        match target {
            FilterTarget::Mask => lang.pick(
                format!("マスクに {name} を追加しました。"),
                format!("Added {name} to the mask."),
            ),
            FilterTarget::Content => {
                let channel = crate::m2::channel_name(lang, &self.doc, self.m2.paint_channel);
                lang.pick(
                    format!("{channel} の画素に {name} を追加しました。"),
                    format!("Added {name} to the {channel} pixels."),
                )
            }
        }
    }

    /// 「ID の色」の Generator の色を選んでいる間に、2D のキャンバスか 3D ビューで押した所の ID の色（`rgb`）を、その段に足す
    /// （Ctrl を押していれば外す）。選んでいなければ false（呼んだ側が選択のツールとして扱う）。
    pub fn pick_id_color(&mut self, rgb: u32) -> bool {
        let Some((layer, id)) = self.fx.id_pick else {
            return false;
        };
        let lang = self.lang;
        let Some(g) = self
            .doc
            .find_filter(id)
            .and_then(|(_, e, _)| e.settings().generator_settings().cloned())
        else {
            self.fx.id_pick = None;
            return false;
        };
        let remove = self.region.modifiers.command;
        let mut next = g.clone();
        let hex = format!("{rgb:06X}");
        if remove {
            next.id_colors.retain(|c| *c != rgb);
        } else if !next.id_colors.contains(&rgb) {
            if next.id_colors.len() >= 32 {
                self.refuse(
                    Source::Effect,
                    lang.pick("ID の色は 32 個までです。", "At most 32 ID colors."),
                );
                return true;
            }
            next.id_colors.push(rgb);
        }
        if next == g {
            // 足す色がもう入っている・外す色が入っていない
            self.refuse(
                Source::Effect,
                if remove {
                    lang.pick(
                        format!("ID の色 {hex} は入っていません。"),
                        format!("{hex} is not in the ID colors."),
                    )
                } else {
                    lang.pick(
                        format!("ID の色 {hex} は入っています。"),
                        format!("{hex} is already in the ID colors."),
                    )
                },
            );
        } else {
            let revision = self.doc.revision();
            match self
                .doc
                .set_filter_settings(layer, id, EffectSettings::generator(next), false)
            {
                Ok(()) => {
                    self.info(
                        Source::Effect,
                        if remove {
                            lang.pick(
                                format!("ID の色から {hex} を外しました。"),
                                format!("Took {hex} out of the ID colors."),
                            )
                        } else {
                            lang.pick(
                                format!("ID の色に {hex} を追加しました。"),
                                format!("Added {hex} to the ID colors."),
                            )
                        },
                    );
                }
                Err(e) => self.notify(
                    crate::notice::Kind::of_core(&e),
                    Source::Effect,
                    lang.core_error(&e),
                ),
            }
            if self.doc.revision() != revision {
                self.modified = true;
            }
        }
        true
    }

    /// 毎フレーム: 効果の入力を文書へ渡し、入力がそろった読むだけのセットを編集できるようにし、新しく使えなくなった Anchor の参照を知らせる。
    pub fn sync_effects(&mut self) {
        if self.is_stroking() {
            return; // 描いている間は入力を替えない（ストロークの途中で合成の意味を変えない）
        }
        // ID の色を選ぶのは、その段の欄が開いていて「ID の色で選択」のツールのあいだだけ
        if self.fx.id_pick.is_some()
            && (self.tool != crate::state::Tool::IdSelect
                || !self.fx.id_pick.is_some_and(|(_, id)| {
                    matches!(self.fx.selected, Some(Selected::Filter { id: s, .. }) if s == id)
                }))
        {
            self.fx.id_pick = None;
        }
        self.sync_effect_inputs();
        self.note_new_anchor_issues();
    }

    /// 操作のあと、新しく使えなくなった Anchor の参照（並べ替えで上下が逆になった・消えた）があれば、状態の帯に理由を添える。
    /// 文書が変わったときだけ見る。選んでいない（NotChosen）のは知らせない。
    fn note_new_anchor_issues(&mut self) {
        if self.is_stroking() {
            return;
        }
        let key = (self.doc.id(), self.doc.revision());
        if self.fx.issues.doc == Some(key) {
            return;
        }
        let same_doc = self.fx.issues.doc.is_some_and(|(id, _)| id == key.0);
        let issues = self.doc.anchor_issues();
        let fresh: Vec<_> = if same_doc {
            issues
                .iter()
                .filter(|i| {
                    !self.fx.issues.known.contains(&i.filter)
                        && i.kind != AnchorIssueKind::NotChosen
                })
                .collect()
        } else {
            Vec::new()
        };
        let lang = self.lang;
        if let Some(first) = fresh.first() {
            let reason = match first.kind {
                AnchorIssueKind::Missing => {
                    lang.pick("読むアンカーがありません", "The anchor to read is gone")
                }
                AnchorIssueKind::NotBelow => lang.pick(
                    "アンカーが自分のレイヤーより下にありません",
                    "The anchor is not below its layer",
                ),
                AnchorIssueKind::NotChosen => "",
            };
            let note = lang.with_reason(
                lang.pick(
                    format!(
                        "アンカーを読むジェネレーター {} 段が、入力をそのまま通すようになりました",
                        fresh.len()
                    ),
                    format!(
                        "{} anchor generator(s) now pass their input through",
                        fresh.len()
                    ),
                ),
                reason,
            );
            // 直前の知らせ（あれば）に、アンカーの但し書きを添える（気をつけること）
            self.amend(crate::notice::Kind::Warning, Source::Effect, " ", &note);
        }
        let known: HashSet<FilterId> = issues.iter().map(|i| i.filter).collect();
        self.fx.issues = AnchorIssuesSeen {
            doc: Some(key),
            known,
        };
    }
}
