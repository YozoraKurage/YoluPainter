//! 文書の定規の口（`crate::rulers` の型を、レイヤーと取り消しにつなぐ）。
//!
//! 定規はレイヤーが持つ（`Layer::rulers`）。編集は 1 つの段 [`Command::Rulers`]（レイヤーごとの前後の一覧）で、作る・動かす・消す・
//! 移す・設定を変えるが各 1 回の取り消し。画素も合成も変えないので、タイルの変化は記録しない（版は進める）。
//!
//! - 見える定規 [`Document::rulers_seen_from`]: 描くレイヤーから、持ち主ごとの表示の範囲・定規の表示・持ち主（とその親）の目で選ぶ。
//! - 効く特殊定規 [`Document::snapping_special_rulers`]: 見える特殊定規のうち `snap` の印のあるものを、描く所（2D・3D）ごとに 1 つ。
//! - レイヤーの操作との関係: 削除は定規ごと（取り消しで戻る）、複製は新しい ID の写し、並べ替え・グループへの出し入れは付いたまま、
//!   結合は結果のレイヤーへ全部移す、画像・キャンバスのサイズ変更は 2D の点だけ動かす、スマートマテリアルは外して知らせる。

use std::collections::{HashMap, HashSet};

use super::merge::MergeRefusal;
use super::{CoalesceKey, Command, Document};
use crate::error::CoreError;
use crate::layer::{Layer, LayerId};
use crate::rulers::{
    Ruler, RulerId, RulerKind, RulerRef, RulerScope, RulerSpace, SpecialRulers,
    MAX_RULERS_PER_LAYER,
};

/// 定規の一覧の入れ替え 1 つ（レイヤー、前、後）。
pub(crate) type RulerChange = (LayerId, Vec<Ruler>, Vec<Ruler>);

/// 定規 1 つが履歴で占める量。
const RULER_COST: u64 = std::mem::size_of::<Ruler>() as u64;

fn change_cost(changes: &[RulerChange]) -> u64 {
    64 + changes
        .iter()
        .map(|(_, old, new)| 32 + RULER_COST * (old.len() + new.len()) as u64)
        .sum::<u64>()
}

/// 一覧の確かめ（上限・各定規の値・一覧の中の ID の重なり）と、`up` を単位にした一覧。
fn checked(list: Vec<Ruler>) -> Result<Vec<Ruler>, CoreError> {
    if list.len() > MAX_RULERS_PER_LAYER {
        return Err(CoreError::InvalidArgument(
            "レイヤーに付けられる定規は 64 個まで",
        ));
    }
    let list: Vec<Ruler> = list.into_iter().map(Ruler::normalized).collect();
    let mut seen = HashSet::new();
    for r in &list {
        r.validate()?;
        if !seen.insert(r.id) {
            return Err(CoreError::InvalidArgument("定規の ID が重なっている"));
        }
    }
    Ok(list)
}

impl Document {
    /// 文書の中で重ならない新しい定規の ID。
    pub fn new_ruler_id(&mut self) -> RulerId {
        loop {
            self.id_counter += 1;
            let id = RulerId(super::random_id(self.id_counter));
            if id.0 != 0 && !self.ruler_id_in_use(id) {
                return id;
            }
        }
    }

    fn ruler_id_in_use(&self, id: RulerId) -> bool {
        self.layers
            .iter()
            .any(|l| l.rulers.iter().any(|r| r.id == id))
    }

    /// `layer` 以外のレイヤーが持つ定規の ID。
    fn ruler_ids_outside(&self, layers: &[LayerId]) -> HashSet<RulerId> {
        self.layers
            .iter()
            .filter(|l| !layers.contains(&l.id))
            .flat_map(|l| l.rulers.iter().map(|r| r.id))
            .collect()
    }

    /// レイヤーの定規の一覧を替える（作る・動かす・消す・設定を変える）。1 回の取り消し。一覧は 64 個まで、各定規は
    /// [`Ruler::validate`] に通り、ID は文書の中で重ならない（新しい ID は [`Document::new_ruler_id`]）。`up` は単位にして持つ。
    /// 今の一覧と同じなら何もしない。`coalesce` なら、直前の段がこのレイヤーのまとめている定規の段のとき、その段の「後」だけを新しくする
    /// （ドラッグ・スライダーを 1 回の取り消しにする。終わりに `end_coalescing`、Escape は `cancel_coalescing`）。
    /// 断ったときは何も変えない。
    pub fn set_rulers(
        &mut self,
        layer: LayerId,
        rulers: Vec<Ruler>,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(layer)?;
        let new = checked(rulers)?;
        let elsewhere = self.ruler_ids_outside(&[layer]);
        if new.iter().any(|r| elsewhere.contains(&r.id)) {
            return Err(CoreError::InvalidArgument(
                "定規の ID がほかのレイヤーの定規と重なっている",
            ));
        }
        let old = self.layers[index].rulers.clone();
        if old == new {
            return Ok(());
        }
        let changes = vec![(layer, old, new)];
        let cost = change_cost(&changes);
        self.record(
            Command::Rulers { changes },
            cost,
            coalesce.then_some(CoalesceKey::Rulers(layer)),
        )
    }

    /// 定規を別のレイヤー（グループ）へ移す。1 回の取り消し（2 つのレイヤーの入れ替えを 1 段に）。移す定規の並びは元の並びのまま、
    /// 移し先の一覧の後ろへ付く。同じレイヤー・無い ID・移し先の上限（64）を超えるときは、何も変えずに断る。
    pub fn move_rulers(
        &mut self,
        from: LayerId,
        to: LayerId,
        ids: &[RulerId],
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let fi = self.index_of(from)?;
        let ti = self.index_of(to)?;
        if from == to {
            return Err(CoreError::InvalidArgument("移し先が同じレイヤー"));
        }
        if ids.is_empty() {
            return Ok(());
        }
        let source = &self.layers[fi].rulers;
        if ids.iter().any(|id| !source.iter().any(|r| r.id == *id)) {
            return Err(CoreError::InvalidArgument("移す定規がそのレイヤーに無い"));
        }
        let moved: Vec<Ruler> = source
            .iter()
            .filter(|r| ids.contains(&r.id))
            .cloned()
            .collect();
        let kept: Vec<Ruler> = source
            .iter()
            .filter(|r| !ids.contains(&r.id))
            .cloned()
            .collect();
        let mut target = self.layers[ti].rulers.clone();
        if target.len() + moved.len() > MAX_RULERS_PER_LAYER {
            return Err(CoreError::InvalidArgument(
                "レイヤーに付けられる定規は 64 個まで",
            ));
        }
        let (old_from, old_to) = (self.layers[fi].rulers.clone(), target.clone());
        target.extend(moved);
        let changes = vec![(from, old_from, kept), (to, old_to, target)];
        let cost = change_cost(&changes);
        self.record(Command::Rulers { changes }, cost, None)
    }

    /// 定規を、`drawing`（描くレイヤー）から見える同じ空間の特殊定規の中の「スナップする特殊定規」にする。同じ空間で見えるほかの
    /// 特殊定規の印は外す（2D と 3D は別に数える）。1 回の取り消し。直線定規は断る。すでにそうなら何もしない。
    pub fn set_snap_ruler(
        &mut self,
        owner: LayerId,
        id: RulerId,
        drawing: Option<LayerId>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let oi = self.index_of(owner)?;
        let Some(target) = self.layers[oi].rulers.iter().find(|r| r.id == id) else {
            return Err(CoreError::InvalidArgument("その定規がそのレイヤーに無い"));
        };
        if !target.is_special() {
            return Err(CoreError::InvalidArgument(
                "直線定規はスナップする特殊定規にならない",
            ));
        }
        let space = target.space();
        let others: HashSet<RulerId> = self
            .rulers_seen_from(drawing)
            .into_iter()
            .filter(|r| r.ruler.is_special() && r.ruler.space() == space && r.ruler.id != id)
            .map(|r| r.ruler.id)
            .collect();
        let mut changes = Vec::new();
        for layer in &self.layers {
            let old = &layer.rulers;
            let mut new = old.clone();
            for r in &mut new {
                if r.id == id {
                    r.snap = true;
                } else if others.contains(&r.id) {
                    r.snap = false;
                }
            }
            if *old != new {
                changes.push((layer.id, old.clone(), new));
            }
        }
        if changes.is_empty() {
            return Ok(());
        }
        let cost = change_cost(&changes);
        self.record(Command::Rulers { changes }, cost, None)
    }

    /// 描くレイヤー `drawing`（無ければ None）から見える定規（下のレイヤーから順、同じレイヤーの中は並びの順）。
    ///
    /// 持ち主 O の「定規のグループ」C は、O がグループなら O、そうでなければ O の親（無ければ文書の一番上）。
    ///
    /// | 表示の範囲 | 描くレイヤー L から見えるとき |
    /// |---|---|
    /// | すべてのレイヤー | いつも |
    /// | 同じグループの中 | L が C の中（入れ子の下まで。C が一番上なら文書の全部）、または L が O |
    /// | 選んでいるときだけ | L が O、または O がグループで L がその中（入れ子の下まで） |
    ///
    /// 表示が偽の定規、持ち主（とその親）の目を閉じた定規は見えない。L が無いときは「すべてのレイヤー」の定規だけ。
    /// （グループには描けないので、L が持ち主のグループそのものになるのは、グループを選んでいるときの編集の見え方のため。）
    pub fn rulers_seen_from(&self, drawing: Option<LayerId>) -> Vec<RulerRef<'_>> {
        if self.layers.iter().all(|l| l.rulers.is_empty()) {
            return Vec::new();
        }
        let index: HashMap<LayerId, usize> = self
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| (l.id, i))
            .collect();
        // 描くレイヤーの親の鎖（近い方から）
        let mut ancestors = Vec::new();
        if let Some(l) = drawing
            .and_then(|d| index.get(&d))
            .map(|&i| &self.layers[i])
        {
            self.collect_ancestors(l, &index, &mut ancestors);
        }
        let mut out = Vec::new();
        for owner in &self.layers {
            if owner.rulers.is_empty() || !self.layer_eye_open(owner, &index) {
                continue;
            }
            // 定規のグループ C。None は文書の一番上
            let cluster = if owner.is_group() {
                Some(owner.id)
            } else {
                owner.parent
            };
            for ruler in &owner.rulers {
                if !ruler.visible {
                    continue;
                }
                let seen = match ruler.scope {
                    RulerScope::All => true,
                    RulerScope::Group => drawing.is_some_and(|d| {
                        d == owner.id || cluster.is_none_or(|c| ancestors.contains(&c))
                    }),
                    RulerScope::Selected => drawing.is_some_and(|d| {
                        d == owner.id || owner.is_group() && ancestors.contains(&owner.id)
                    }),
                };
                if seen {
                    out.push(RulerRef {
                        owner: owner.id,
                        ruler,
                    });
                }
            }
        }
        out
    }

    /// `layer` の親の鎖（近い方から）。入れ子が輪になっていても止まる。
    fn collect_ancestors(
        &self,
        layer: &Layer,
        index: &HashMap<LayerId, usize>,
        out: &mut Vec<LayerId>,
    ) {
        let mut parent = layer.parent;
        while let Some(p) = parent {
            if out.len() > self.layers.len() {
                return;
            }
            out.push(p);
            parent = index.get(&p).and_then(|&i| self.layers[i].parent);
        }
    }

    /// レイヤーとその親の目がすべて開いているか。
    fn layer_eye_open(&self, layer: &Layer, index: &HashMap<LayerId, usize>) -> bool {
        if !layer.visible {
            return false;
        }
        let mut chain = Vec::new();
        self.collect_ancestors(layer, index, &mut chain);
        chain
            .iter()
            .all(|p| index.get(p).is_some_and(|&i| self.layers[i].visible))
    }

    /// 描くレイヤーから見える特殊定規のうち、`space` の空間のもの（候補。下のレイヤーから順、同じレイヤーの中は並びの順）。
    pub fn special_ruler_candidates(
        &self,
        drawing: Option<LayerId>,
        space: RulerSpace,
    ) -> Vec<RulerRef<'_>> {
        self.rulers_seen_from(drawing)
            .into_iter()
            .filter(|r| r.ruler.is_special() && r.ruler.space() == space)
            .collect()
    }

    /// 描くレイヤーで効く特殊定規（2D・3D それぞれ 1 つまで）。候補（[`Document::special_ruler_candidates`]）のうち `snap` の印のある
    /// ものを、上のレイヤー、同じレイヤーなら一覧の後ろのものを先にして 1 つ選ぶ。印のあるものが無ければ、その空間は効かない
    /// （勝手に選ばない）。同じ空間の中では、対称とパースなど特殊定規は同時に効かない。
    pub fn snapping_special_rulers(&self, drawing: Option<LayerId>) -> SpecialRulers<'_> {
        let pick = |space| {
            self.special_ruler_candidates(drawing, space)
                .into_iter()
                .rfind(|r| r.ruler.snap)
        };
        SpecialRulers {
            canvas: pick(RulerSpace::Canvas),
            model: pick(RulerSpace::Model),
        }
    }

    /// 描くレイヤーから見える直線定規。
    pub fn straight_rulers_seen_from(&self, drawing: Option<LayerId>) -> Vec<RulerRef<'_>> {
        self.rulers_seen_from(drawing)
            .into_iter()
            .filter(|r| r.ruler.kind == RulerKind::Line)
            .collect()
    }

    /// 読み込み用: 履歴なしでレイヤーの定規を付ける（保存から戻すとき。履歴・まとめの中では断る）。値の確かめは
    /// [`Document::set_rulers`] と同じ。
    pub fn set_rulers_for_load(
        &mut self,
        layer: LayerId,
        rulers: Vec<Ruler>,
    ) -> Result<(), CoreError> {
        self.ensure_loadable()?;
        let index = self.index_of(layer)?;
        let new = checked(rulers)?;
        let elsewhere = self.ruler_ids_outside(&[layer]);
        if new.iter().any(|r| elsewhere.contains(&r.id)) {
            return Err(CoreError::InvalidArgument(
                "定規の ID がほかのレイヤーの定規と重なっている",
            ));
        }
        self.layers[index].rulers = new;
        self.external_mutation();
        Ok(())
    }

    /// 段を当てる・戻す（画素も合成も変えないので、タイルの変化は記録しない）。先に全部のレイヤーがあることを確かめ、無ければ
    /// 何も変えずに断る。
    pub(super) fn switch_rulers(
        &mut self,
        changes: &[RulerChange],
        backwards: bool,
    ) -> Result<(), CoreError> {
        let mut indices = Vec::with_capacity(changes.len());
        for (id, _, _) in changes {
            indices.push(self.index_of(*id)?);
        }
        for (&i, (_, old, new)) in indices.iter().zip(changes) {
            self.layers[i].rulers = if backwards { old.clone() } else { new.clone() };
        }
        Ok(())
    }

    /// レイヤーの写し（文書に入れる前）の定規に新しい ID を付ける（複製・スマートマテリアルの配置）。
    pub(super) fn renew_ruler_ids(&mut self, copies: &mut [Layer]) {
        let mut fresh: HashSet<RulerId> = HashSet::new();
        for l in copies.iter_mut() {
            for r in &mut l.rulers {
                let id = loop {
                    self.id_counter += 1;
                    let id = RulerId(super::random_id(self.id_counter));
                    if id.0 != 0 && !self.ruler_id_in_use(id) && !fresh.contains(&id) {
                        break id;
                    }
                };
                fresh.insert(id);
                r.id = id;
            }
        }
    }

    /// 結合で外すレイヤー（`removed`）の定規を、文書の並びの順に集める。結果のレイヤーが 64 個を超えて持つことになるときは断る。
    pub(super) fn rulers_for_merge(&self, removed: &[LayerId]) -> Result<Vec<Ruler>, CoreError> {
        let moved: Vec<Ruler> = self
            .layers
            .iter()
            .filter(|l| removed.contains(&l.id))
            .flat_map(|l| l.rulers.iter().cloned())
            .collect();
        if moved.len() > MAX_RULERS_PER_LAYER {
            return Err(CoreError::MergeRefused(MergeRefusal::TooManyRulers));
        }
        Ok(moved)
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;

    use super::*;

    fn ruler(id: u128) -> Ruler {
        Ruler::canvas(RulerId(id), RulerKind::Line, DVec2::ZERO, DVec2::X * 10.0)
    }

    #[test]
    fn a_change_costs_a_base_plus_a_share_per_layer_and_ruler() {
        assert_eq!(change_cost(&[]), 64);
        let one = [(LayerId(1), vec![ruler(1)], vec![ruler(1), ruler(2)])];
        assert_eq!(change_cost(&one), 64 + 32 + 3 * RULER_COST);
        let two = [one[0].clone(), (LayerId(2), Vec::new(), vec![ruler(3)])];
        assert_eq!(
            change_cost(&two),
            64 + (32 + 3 * RULER_COST) + (32 + RULER_COST)
        );
    }

    #[test]
    fn only_ids_held_by_some_layer_are_in_use() {
        let mut d = Document::new(8, 8).unwrap();
        let (a, b) = (d.add_layer("a").unwrap(), d.add_layer("b").unwrap());
        assert!(!d.ruler_id_in_use(RulerId(7)));
        d.set_rulers(a, vec![ruler(7)], false).unwrap();
        d.set_rulers(b, vec![ruler(9)], false).unwrap();
        assert!(d.ruler_id_in_use(RulerId(7)));
        assert!(d.ruler_id_in_use(RulerId(9)));
        assert!(!d.ruler_id_in_use(RulerId(8)));
    }

    /// ID の素の値は乱数だが、種の数え上げは 1 回の発行ごとに 1 つ進む（進まないと、同じ種から作り続ける）。
    #[test]
    fn issuing_ids_advances_the_counter_once_per_id() {
        let mut d = Document::new(8, 8).unwrap();
        let start = d.id_counter;
        let first = d.new_ruler_id();
        assert_eq!(d.id_counter, start + 1);
        let second = d.new_ruler_id();
        assert_eq!(d.id_counter, start + 2);
        assert_ne!(first, second);
        assert_ne!(first.0, 0);

        // 複製の新しい ID: 定規 1 つにつき 1 つ進み、どれも元と、互いに違う
        let layer = d.add_layer("x").unwrap();
        let list: Vec<Ruler> = (1..=3).map(ruler).collect();
        d.set_rulers(layer, list, false).unwrap();
        let original = d.layer(layer).unwrap().clone();
        let mut copies = vec![original.clone(), original.clone()];
        let before = d.id_counter;
        d.renew_ruler_ids(&mut copies);
        assert_eq!(d.id_counter, before + 6, "6 個の定規に 6 回");
        let mut all: Vec<RulerId> = copies
            .iter()
            .flat_map(|l| l.rulers.iter().map(|r| r.id))
            .collect();
        assert_eq!(all.len(), 6);
        assert!(all.iter().all(|id| id.0 != 0 && !d.ruler_id_in_use(*id)));
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 6, "写しの中でも重ならない");
        assert_eq!(
            d.layer(layer).unwrap().rulers,
            original.rulers,
            "元は変わらない"
        );
    }
}
