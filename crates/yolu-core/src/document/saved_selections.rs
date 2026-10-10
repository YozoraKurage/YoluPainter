//! 名前を付けて文書に残す選択範囲。
//!
//! - 今の選択範囲に名前を付けて残し、あとで今の選択範囲と組み合わせて呼び戻す（置き換え・足す・引く・重なり。呼び戻しは
//!   [`Document::combine_selection`] と同じ 1 回の Undo）。残す・名前を変える・消すも、どれも 1 回の Undo（画素は変えない）。
//! - 文書ごとに最大 [`MAX_SAVED_SELECTIONS`] 個。名前は前後の空白を除いて 1〜[`MAX_SAVED_NAME_CHARS`] 文字、制御文字なし、
//!   文書の中で重ならない（完全に同じ名前だけ。大文字小文字は区別する）。同じ名前で残し直すと、その場所の選択範囲を入れ替える。
//! - 選択範囲は文書と同じ大きさ・タイルの大きさ。キャンバスの大きさを変える操作は、今の選択範囲と同じ道（`resize`）で、残した選択範囲も
//!   新しい大きさへ作り直す（取り消せば元の大きさのものへ戻る。縮小で何も選んでいない所だけが残るものは外し、報告に書く）。
//!   レイヤーの変形（移動・回転）は残した選択範囲を動かさない（残した選択範囲はレイヤーの画素と別のもの）。
//! - 履歴の重さは、入れ替わった選択範囲の大きさ（共有しているものは数えない）。写し（`capture_snapshot`）はそのまま持つ。
//!   画素の予算には数えない（呼び手が全体の予算を決める）。

use std::sync::Arc;

use super::{Command, Document};
use crate::error::CoreError;
use crate::selection::{SelectionCombine, SelectionMask};

/// 1 つの文書に残せる数。
pub const MAX_SAVED_SELECTIONS: usize = 32;
/// 名前の長さの上限（文字数）。
pub const MAX_SAVED_NAME_CHARS: usize = 256;

/// 名前つきの選択範囲 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct SavedSelection {
    pub name: String,
    pub mask: SelectionMask,
}

/// 文書が持つ並び（履歴と写しが共有する）。
pub(crate) type SavedList = Arc<Vec<SavedSelection>>;

/// 名前を整える（前後の空白を除く）。空・長すぎる・制御文字つきは断る。
pub fn clean_saved_name(name: &str) -> Result<String, CoreError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::InvalidArgument("選択範囲の名前が空"));
    }
    if name.chars().count() > MAX_SAVED_NAME_CHARS {
        return Err(CoreError::InvalidArgument("選択範囲の名前が長すぎる"));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(CoreError::InvalidArgument("選択範囲の名前に制御文字がある"));
    }
    Ok(name.to_owned())
}

impl Document {
    /// 名前を付けて残した選択範囲（並びの順）。
    pub fn saved_selections(&self) -> &[SavedSelection] {
        &self.saved_selections
    }

    /// 残した選択範囲が、今の文書の大きさに合うか。
    fn saved_fits(&self, mask: &SelectionMask) -> bool {
        mask.width() == self.width
            && mask.height() == self.height
            && mask.tile_size() == self.tile_size
    }

    /// 並びを入れ替える 1 段（同じ並びなら何もしない）。履歴の重さは、前後で共有していない選択範囲の大きさ。
    fn replace_saved(&mut self, next: Vec<SavedSelection>) -> Result<(), CoreError> {
        if next.len() == self.saved_selections.len()
            && next
                .iter()
                .zip(self.saved_selections.iter())
                .all(|(a, b)| a.name == b.name && a.mask.same_as(&b.mask))
        {
            return Ok(());
        }
        let only_in = |a: &[SavedSelection], b: &[SavedSelection]| -> u64 {
            a.iter()
                .filter(|s| !b.iter().any(|t| t.mask.same_as(&s.mask)))
                .map(|s| s.mask.history_bytes())
                .sum()
        };
        let cost =
            64 + only_in(&next, &self.saved_selections) + only_in(&self.saved_selections, &next);
        let old = self.saved_selections.clone();
        self.execute(
            Command::SavedSelections {
                old,
                new: Arc::new(next),
            },
            cost,
        )
    }

    /// 今の選択範囲に名前を付けて残す（1 回の Undo）。同じ名前があれば、その場所の選択範囲を入れ替える。選択が無い・上限を
    /// 超える（新しい名前のとき）・描いている最中は断る。残した場所（並びの番号）を返す。
    pub fn save_selection(&mut self, name: &str) -> Result<usize, CoreError> {
        self.ensure_no_stroke()?;
        let name = clean_saved_name(name)?;
        let mask = self
            .selection
            .clone()
            .ok_or(CoreError::InvalidArgument("選択範囲が無い"))?;
        let mut next: Vec<SavedSelection> = self.saved_selections.as_ref().clone();
        let at = match next.iter().position(|s| s.name == name) {
            Some(i) => {
                next[i].mask = mask;
                i
            }
            None => {
                if next.len() >= MAX_SAVED_SELECTIONS {
                    return Err(CoreError::InvalidArgument("残せる選択範囲の数の上限"));
                }
                next.push(SavedSelection { name, mask });
                next.len() - 1
            }
        };
        self.replace_saved(next)?;
        Ok(at)
    }

    /// 残した選択範囲を消す（1 回の Undo）。
    pub fn delete_saved_selection(&mut self, index: usize) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let mut next: Vec<SavedSelection> = self.saved_selections.as_ref().clone();
        if index >= next.len() {
            return Err(CoreError::InvalidArgument("残した選択範囲の番号"));
        }
        next.remove(index);
        self.replace_saved(next)
    }

    /// 残した選択範囲の名前を変える（1 回の Undo）。ほかの選択範囲と同じ名前は断る。今の名前と同じなら何もしない。
    pub fn rename_saved_selection(&mut self, index: usize, name: &str) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let name = clean_saved_name(name)?;
        let mut next: Vec<SavedSelection> = self.saved_selections.as_ref().clone();
        if index >= next.len() {
            return Err(CoreError::InvalidArgument("残した選択範囲の番号"));
        }
        if next
            .iter()
            .enumerate()
            .any(|(i, s)| i != index && s.name == name)
        {
            return Err(CoreError::InvalidArgument("同じ名前の選択範囲がある"));
        }
        next[index].name = name;
        self.replace_saved(next)
    }

    /// 残した選択範囲を今の選択範囲と組み合わせる（置き換え・足す・引く・重なり。1 回の Undo）。文書の大きさと合わないものは断る
    /// （キャンバスの大きさを変える操作は残した選択範囲も作り直すので、合わないのは読み込みの不整合だけ）。
    pub fn recall_saved_selection(
        &mut self,
        index: usize,
        mode: SelectionCombine,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let saved = self
            .saved_selections
            .get(index)
            .ok_or(CoreError::InvalidArgument("残した選択範囲の番号"))?;
        if !self.saved_fits(&saved.mask) {
            return Err(CoreError::InvalidArgument(
                "選択範囲の大きさがキャンバスと違う",
            ));
        }
        let mask = saved.mask.clone();
        self.combine_selection(&mask, mode)
    }

    /// 文書と一緒に読んだ残した選択範囲を戻す（`selections.json`）: Undo の段も版も増やさない（開いた直後の文書は保存済みのまま）。
    /// 履歴ができた後は断る。数・名前・大きさが決まりに合わなければ、何も戻さずに断る。
    pub fn restore_saved_selections(&mut self, list: Vec<SavedSelection>) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        if !self.undo.is_empty() || !self.redo.is_empty() {
            return Err(CoreError::Unsupported(
                "残した選択範囲を戻せるのは読み込みの直後だけ",
            ));
        }
        if list.len() > MAX_SAVED_SELECTIONS {
            return Err(CoreError::InvalidArgument("残せる選択範囲の数の上限"));
        }
        for (i, s) in list.iter().enumerate() {
            if clean_saved_name(&s.name)? != s.name {
                return Err(CoreError::InvalidArgument("選択範囲の名前が整っていない"));
            }
            if list[..i].iter().any(|t| t.name == s.name) {
                return Err(CoreError::InvalidArgument("同じ名前の選択範囲がある"));
            }
            if !self.saved_fits(&s.mask) {
                return Err(CoreError::InvalidArgument(
                    "選択範囲の大きさがキャンバスと違う",
                ));
            }
        }
        self.saved_selections = Arc::new(list);
        Ok(())
    }

    /// 段を当てる・戻す（画素も合成も変えないので、タイルの変化は記録しない）。
    pub(super) fn switch_saved_selections(
        &mut self,
        old: &SavedList,
        new: &SavedList,
        backwards: bool,
    ) {
        self.saved_selections = if backwards { old.clone() } else { new.clone() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HistoryKind;

    fn doc() -> Document {
        Document::with_tile_size(40, 30, 16).unwrap()
    }
    fn select(doc: &mut Document, x0: i64, y0: i64, x1: i64, y1: i64) {
        let mask = SelectionMask::rectangle(doc, x0, y0, x1, y1);
        doc.set_selection(Some(mask)).unwrap();
    }

    #[test]
    fn saving_renaming_and_deleting_are_one_undo_step_each() {
        let mut d = doc();
        select(&mut d, 2, 2, 12, 12);
        let revision = d.revision();
        assert_eq!(d.save_selection("  髪  ").unwrap(), 0);
        assert_eq!(d.saved_selections()[0].name, "髪", "前後の空白は除く");
        assert!(d.revision() > revision, "版が進む（保存の鍵）");
        assert_eq!(d.history().last(), Some(HistoryKind::SavedSelections));
        select(&mut d, 20, 5, 30, 25);
        assert_eq!(d.save_selection("服").unwrap(), 1);
        let steps = d.undo_count();
        d.rename_saved_selection(0, "前髪").unwrap();
        assert_eq!(d.undo_count(), steps + 1);
        assert_eq!(d.saved_selections()[0].name, "前髪");
        d.delete_saved_selection(1).unwrap();
        assert_eq!(d.saved_selections().len(), 1);
        // 取り消しは 1 段ずつ（消す → 名前 → 服を残す）
        assert!(d.undo().unwrap());
        assert_eq!(d.saved_selections().len(), 2);
        assert!(d.undo().unwrap());
        assert_eq!(d.saved_selections()[0].name, "髪");
        assert!(d.undo().unwrap());
        assert_eq!(d.saved_selections().len(), 1);
        assert!(d.redo().unwrap());
        assert_eq!(d.saved_selections().len(), 2);
    }

    #[test]
    fn saving_the_same_name_again_replaces_in_place_and_nothing_changed_adds_no_step() {
        let mut d = doc();
        select(&mut d, 0, 0, 8, 8);
        d.save_selection("a").unwrap();
        select(&mut d, 4, 4, 20, 20);
        d.save_selection("b").unwrap();
        let steps = d.undo_count();
        // 同じ選択範囲を同じ名前で残し直しても段を積まない
        assert_eq!(d.save_selection("b").unwrap(), 1);
        assert_eq!(d.undo_count(), steps);
        select(&mut d, 10, 10, 30, 25);
        assert_eq!(
            d.save_selection("a").unwrap(),
            0,
            "同じ名前はその場所を入れ替える"
        );
        assert_eq!(d.saved_selections().len(), 2);
        assert_eq!(d.saved_selections()[0].mask.amount(15, 15), 255);
        assert_eq!(d.saved_selections()[0].mask.amount(2, 2), 0);
        assert!(d.undo().unwrap());
        assert_eq!(
            d.saved_selections()[0].mask.amount(2, 2),
            255,
            "取り消すと前の選択範囲"
        );
    }

    #[test]
    fn refusals_change_nothing() {
        let mut d = doc();
        // 選択が無い
        assert!(d.save_selection("x").is_err());
        select(&mut d, 0, 0, 8, 8);
        assert!(d.save_selection("   ").is_err(), "空の名前");
        assert!(d.save_selection("a\nb").is_err(), "制御文字");
        assert!(
            d.save_selection(&"あ".repeat(MAX_SAVED_NAME_CHARS + 1))
                .is_err(),
            "長すぎる"
        );
        assert!(
            d.save_selection(&"あ".repeat(MAX_SAVED_NAME_CHARS)).is_ok(),
            "上限ちょうどは通る"
        );
        assert_eq!(d.saved_selections().len(), 1);
        d.save_selection("b").unwrap();
        let steps = d.undo_count();
        assert!(
            d.rename_saved_selection(1, &"あ".repeat(MAX_SAVED_NAME_CHARS))
                .is_err(),
            "同じ名前"
        );
        assert!(d.rename_saved_selection(5, "z").is_err());
        assert!(d.delete_saved_selection(5).is_err());
        assert!(d
            .recall_saved_selection(5, SelectionCombine::Replace)
            .is_err());
        assert_eq!(d.undo_count(), steps);
        // 上限
        for i in 0..MAX_SAVED_SELECTIONS {
            let _ = d.save_selection(&format!("n{i}"));
        }
        assert_eq!(d.saved_selections().len(), MAX_SAVED_SELECTIONS);
        let steps = d.undo_count();
        assert!(d.save_selection("one too many").is_err());
        assert_eq!(d.undo_count(), steps);
        assert!(
            d.save_selection("b").is_ok(),
            "同じ名前の入れ替えは上限でもできる"
        );
    }

    #[test]
    fn recall_combines_four_ways_in_one_undo_step() {
        let mut d = doc();
        select(&mut d, 0, 0, 20, 20);
        d.save_selection("a").unwrap();
        select(&mut d, 10, 10, 30, 30);
        let amount = |d: &Document, x, y| d.selection().map_or(0, |m| m.amount(x, y));
        // 置き換え
        let steps = d.undo_count();
        d.recall_saved_selection(0, SelectionCombine::Replace)
            .unwrap();
        assert_eq!(d.undo_count(), steps + 1);
        assert_eq!((amount(&d, 5, 5), amount(&d, 25, 25)), (255, 0));
        assert!(d.undo().unwrap());
        assert_eq!(
            (amount(&d, 5, 5), amount(&d, 25, 25)),
            (0, 255),
            "取り消すと今の選択範囲"
        );
        // 足す
        d.recall_saved_selection(0, SelectionCombine::Add).unwrap();
        assert_eq!(
            (amount(&d, 5, 5), amount(&d, 15, 15), amount(&d, 25, 25)),
            (255, 255, 255)
        );
        assert!(d.undo().unwrap());
        // 引く
        d.recall_saved_selection(0, SelectionCombine::Subtract)
            .unwrap();
        assert_eq!((amount(&d, 15, 15), amount(&d, 25, 25)), (0, 255));
        assert!(d.undo().unwrap());
        // 重なり
        d.recall_saved_selection(0, SelectionCombine::Intersect)
            .unwrap();
        assert_eq!(
            (amount(&d, 5, 5), amount(&d, 15, 15), amount(&d, 25, 25)),
            (0, 255, 0)
        );
    }

    #[test]
    fn restore_after_load_adds_no_step_and_checks_the_rules() {
        let mut d = doc();
        let mask = SelectionMask::rectangle(&d, 1, 1, 9, 9);
        let item = |name: &str| SavedSelection {
            name: name.into(),
            mask: mask.clone(),
        };
        let revision = d.revision();
        d.restore_saved_selections(vec![item("a"), item("b")])
            .unwrap();
        assert_eq!(
            (d.undo_count(), d.revision()),
            (0, revision),
            "段も版も増えない"
        );
        assert_eq!(d.saved_selections().len(), 2);
        // 決まりに合わない並びは断り、何も変えない
        let mut e = doc();
        assert!(
            e.restore_saved_selections(vec![item("a"), item("a")])
                .is_err(),
            "名前の重なり"
        );
        assert!(
            e.restore_saved_selections(vec![item(" a")]).is_err(),
            "整っていない名前"
        );
        let other = Document::with_tile_size(41, 30, 16).unwrap();
        let wrong = SavedSelection {
            name: "w".into(),
            mask: SelectionMask::rectangle(&other, 0, 0, 5, 5),
        };
        assert!(
            e.restore_saved_selections(vec![wrong]).is_err(),
            "大きさが違う"
        );
        let many: Vec<_> = (0..=MAX_SAVED_SELECTIONS)
            .map(|i| item(&format!("n{i}")))
            .collect();
        assert!(e.restore_saved_selections(many).is_err(), "上限");
        assert!(e.saved_selections().is_empty());
        // 履歴ができた後は断る
        select(&mut e, 0, 0, 4, 4);
        assert!(e.restore_saved_selections(vec![item("a")]).is_err());
    }

    #[test]
    fn history_cost_counts_only_masks_that_are_not_shared() {
        let mut d = doc();
        select(&mut d, 0, 0, 30, 25);
        d.save_selection("a").unwrap();
        let before = d.history_bytes;
        // 名前の変更は選択範囲を共有する（重さは小さな定数だけ）
        d.rename_saved_selection(0, "b").unwrap();
        assert_eq!(d.history_bytes - before, 64);
    }

    #[test]
    fn a_snapshot_carries_the_saved_selections_and_does_not_follow_the_source() {
        let mut d = doc();
        select(&mut d, 0, 0, 8, 8);
        d.save_selection("a").unwrap();
        let snap = d.capture_snapshot().unwrap();
        assert_eq!(snap.saved_selections(), d.saved_selections());
        d.delete_saved_selection(0).unwrap();
        assert_eq!(snap.saved_selections().len(), 1, "写しは動かない");
    }

    #[test]
    fn a_batch_edit_keeps_the_saved_selections() {
        // 準備用の文書を経由する操作（結合・変形など）が、残した選択範囲を失わない
        let mut d = doc();
        select(&mut d, 0, 0, 8, 8);
        d.save_selection("a").unwrap();
        let layer = d.add_layer("L").unwrap();
        d.set_layer_name(layer, "M").unwrap();
        assert_eq!(d.saved_selections().len(), 1);
        assert!(d.undo().unwrap());
        assert_eq!(d.saved_selections().len(), 1);
    }

    #[test]
    fn resizing_the_canvas_resamples_the_saved_selections_and_undo_brings_the_old_size_back() {
        use crate::CanvasResampling;
        let mut d = Document::with_tile_size(40, 30, 16).unwrap();
        select(&mut d, 0, 0, 20, 20);
        d.save_selection("a").unwrap();
        select(&mut d, 30, 22, 31, 23);
        d.save_selection("tiny").unwrap();
        let revision = d.revision();
        let report = d.resize_image(20, 15, CanvasResampling::Nearest).unwrap();
        assert!(d.revision() > revision);
        for s in d.saved_selections() {
            assert_eq!((s.mask.width(), s.mask.height()), (20, 15), "{}", s.name);
        }
        assert!(d.saved_selections()[0].mask.amount(5, 5) > 0);
        assert!(d.saved_selections()[0].mask.amount(15, 12) == 0);
        // 取り消すと、元の大きさの選択範囲へ戻る（作り直したものを引きずらない）
        assert!(d.undo().unwrap());
        assert_eq!(d.saved_selections().len(), 2);
        for s in d.saved_selections() {
            assert_eq!((s.mask.width(), s.mask.height()), (40, 30), "{}", s.name);
        }
        d.recall_saved_selection(0, SelectionCombine::Replace)
            .unwrap();
        // 縮小で何も残らないもの（量 1 の 1 画素を面積で平均すると 0 になる）は外し、報告に書く
        let mut e = Document::with_tile_size(40, 30, 16).unwrap();
        let mut amounts = vec![0u8; 16 * 16];
        amounts[0] = 1;
        let faint =
            SelectionMask::from_amount_tiles(40, 30, 16, [(crate::TileCoord::new(0, 0), amounts)])
                .unwrap();
        e.set_selection(Some(faint)).unwrap();
        e.save_selection("faint").unwrap();
        select(&mut e, 0, 0, 20, 20);
        e.save_selection("big").unwrap();
        let report2 = e.resize_image(10, 8, CanvasResampling::Area).unwrap();
        let _ = report;
        assert_eq!(e.saved_selections().len(), 1, "{:?}", e.saved_selections());
        assert_eq!(e.saved_selections()[0].name, "big");
        assert!(
            report2.notes.iter().any(|n| n.contains("残した選択範囲")),
            "{:?}",
            report2.notes
        );
        assert_eq!(report2.dropped_saved_selections, 1, "数も型で返す");
        assert_eq!(
            report.dropped_saved_selections, 1,
            "近傍の縮小で拾われない 1 画素（tiny）だけが外れる"
        );
        assert!(e.undo().unwrap());
        assert_eq!(e.saved_selections().len(), 2, "取り消すと戻る");
        // キャンバスだけの変更（切り落とし）でも大きさが合う
        let mut f = Document::with_tile_size(40, 30, 16).unwrap();
        select(&mut f, 2, 2, 12, 12);
        f.save_selection("a").unwrap();
        f.resize_canvas(60, 50, (5, 5)).unwrap();
        assert_eq!(
            (
                f.saved_selections()[0].mask.width(),
                f.saved_selections()[0].mask.height()
            ),
            (60, 50)
        );
        f.recall_saved_selection(0, SelectionCombine::Replace)
            .unwrap();
    }
}
