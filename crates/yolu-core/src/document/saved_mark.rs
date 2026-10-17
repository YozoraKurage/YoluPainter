//! 保存した所の目印。保存した文書と同じ中身かを、Undo・Redo で保存した所へ戻ったときも含めて答える。
//!
//! 履歴の段は、Undo・Redo で行き来できる並び。保存した時点の履歴の位置（`undo` の段数）を覚え、今の位置と同じなら同じ中身とみなす。
//! 位置だけでは足りない場合を、次の 2 つで外す:
//! - 履歴を通らない変更（読み込み・書き込み・巻き戻したまとめなど。`revision` は進むが [`Document::history_bump`] を通らない）が
//!   あれば、保存し直すまで同じ中身とは言わない（`revision` と `modeled` がずれる）。
//! - 保存した位置に戻れなくなったとき（新しい段を積んで Redo を捨てた・古い段を予算で落とした・保存した位置の段へまとめを足した）は
//!   位置を忘れる。
//!
//! 断ったときの向きは常に「同じ中身ではない」（保存の確かめを増やす側）。同じ中身と誤って言うのは、保存していない変更を捨てさせるので避ける。

use super::Document;

/// 保存した所の目印（[`Document`] が 1 つ持つ）。
pub(super) struct SavedMark {
    /// 保存した時点の履歴の位置（`undo` の段数）。戻れなくなったら `None`。
    at: Option<usize>,
    /// 履歴の操作（積む・Undo・Redo）だけで進めた `revision`。ずれたら、履歴を通らない変更がある。
    modeled: u64,
    /// 進行中のストロークの最初の筆跡の時点で、履歴を通らない変更が無かったか（筆跡は `revision` を進めるが、確定か取り消しで履歴に
    /// 収まるか元へ戻る）。
    stroke_sync: Option<bool>,
}

impl Default for SavedMark {
    /// 作ったばかりの文書は、保存した所（空の履歴）にいる。
    fn default() -> Self {
        SavedMark {
            at: Some(0),
            modeled: 0,
            stroke_sync: None,
        }
    }
}

impl SavedMark {
    pub(super) fn is_at(&self, position: usize) -> bool {
        self.at == Some(position)
    }

    /// 新しい段を積む前: Redo を捨てるので、Redo の側にあった保存した位置は戻れなくなる。
    pub(super) fn drop_redo_side(&mut self, undo_len: usize) {
        if self.at.is_some_and(|at| at > undo_len) {
            self.at = None;
        }
    }

    /// 予算で古い Undo の段を `dropped_undo` 段、遠い Redo の段を落とした後（残りの段は合わせて `kept`）。
    pub(super) fn history_trimmed(&mut self, dropped_undo: usize, kept: usize) {
        self.at = self
            .at
            .and_then(|at| at.checked_sub(dropped_undo))
            .filter(|at| *at <= kept);
    }

    /// 履歴を空にした後。空にする前に保存した位置にいたなら、空の位置が保存した所になる。
    pub(super) fn history_cleared(&mut self, was_at_saved: bool) {
        self.at = was_at_saved.then_some(0);
    }

    pub(super) fn forget_position(&mut self) {
        self.at = None;
    }

    /// 2D のストローク以外（3D のマテリアルのストローク・三角形の塗り）が終わった後: 筆跡の控えを下ろす。
    pub(super) fn stroke_ended(&mut self) {
        self.stroke_sync = None;
    }

    /// ストロークの筆跡が `revision` を進める直前。
    pub(super) fn stroke_dab(&mut self, revision: u64) {
        if self.stroke_sync.is_none() {
            self.stroke_sync = Some(revision == self.modeled);
        }
    }
}

impl Document {
    /// 履歴の操作（積む・Undo・Redo）で `revision` を進める。履歴を通らない変更がまだ無ければ、その分も数え済みにする。
    pub(super) fn history_bump(&mut self) {
        let in_sync = self.revision == self.saved.modeled;
        self.revision += 1;
        if in_sync {
            self.saved.modeled = self.revision;
        }
    }

    /// 直前の段へまとめを足して `revision` を進める。保存した位置の段へ足すと、その位置の中身が変わる。
    pub(super) fn merge_bump(&mut self) {
        if self.saved.is_at(self.undo.len()) {
            self.saved.at = None;
        }
        self.history_bump();
    }

    /// ストロークを確定・取り消した後: 筆跡の分の `revision` は、確定なら積んだ段、取り消しなら元の画素に収まった。
    pub(super) fn settle_stroke_mark(&mut self) {
        if self.saved.stroke_sync.take() == Some(true) {
            self.saved.modeled = self.revision;
        }
    }

    /// 今の中身を保存した所として覚える（保存の頼みを出すとき・開いたとき）。
    pub fn mark_saved(&mut self) {
        self.saved.at = Some(self.undo.len());
        self.saved.modeled = self.revision;
        self.saved.stroke_sync = None;
    }

    /// 最後に [`mark_saved`](Self::mark_saved) した（作った）ときと同じ中身か。Undo・Redo で保存した所へ戻っていれば true。
    /// ストロークの途中は、画素を変えた筆跡があれば false（`revision` が進む。筆跡が画素を変えなければ、描き始めただけでは true）。履歴を通らない
    /// 変更の後・保存した所に戻れなくなった後も false（保存し直すまで）。
    pub fn is_clean(&self) -> bool {
        self.triangle_fill.is_none()
            && !self.material.started
            && self.revision == self.saved.modeled
            && self.saved.is_at(self.undo.len())
    }
}
