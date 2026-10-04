//! 複数の編集を 1 回の Undo にまとめる（C# の `PaintDocument.Batch`）と、履歴の段の合成。

use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

use super::{Command, Document, Entry};
use crate::error::CoreError;

impl Document {
    /// edits の中の編集が履歴に積んだ段を 1 段にまとめる（Undo 1 回で全部戻り、Redo 1 回で全部また当たる）。edits が失敗する
    /// （`Err` を返す・panic する）と、積まれた段を新しい方から戻し、履歴（Undo・Redo・そのバイト数）を始める前と同じにする
    /// （変更番号だけは進めるので、表示は描き直される）。履歴の整理（予算を超えた古い段を落とす）は、まとめが終わってから 1 回だけ。
    /// 何も積まれなければ段は増えず、Redo も残る。
    ///
    /// 中ではストローク・Undo・Redo・履歴を消す書き込み（読み込み用の直接の書き込み）は `CoreError::BatchActive` で断り、まとめは
    /// 入れ子にできない。まとめの中の 1 つの操作がすでに複数の段を作る（貼り付けなど）のは構わない。
    pub fn batch<T>(
        &mut self,
        edits: impl FnOnce(&mut Document) -> Result<T, CoreError>,
    ) -> Result<T, CoreError> {
        self.ensure_no_stroke()?;
        self.ensure_not_batching()?;
        // 前の連続した変更（スライダーのドラッグ）へ、まとめの中の変更を混ぜない
        self.end_coalescing();
        let start = self.undo.len();
        // 新しい段を積むと Redo は消える（積まれなければ残す）ので、先に脇へ置いて、その分のバイト数を引いておく
        let saved_redo = std::mem::take(&mut self.redo);
        let redo_bytes: u64 = saved_redo.iter().map(|e| e.cost).sum();
        self.history_bytes -= redo_bytes;
        self.batching = true;
        let outcome = catch_unwind(AssertUnwindSafe(|| edits(self)));
        self.batching = false;
        match outcome {
            Ok(Ok(value)) => {
                self.finish_batch(start, saved_redo, redo_bytes);
                Ok(value)
            }
            Ok(Err(error)) => {
                self.rollback_batch(start, saved_redo, redo_bytes);
                Err(error)
            }
            Err(panic) => {
                self.rollback_batch(start, saved_redo, redo_bytes);
                resume_unwind(panic)
            }
        }
    }

    /// 積まれた段が 2 つ以上なら 1 段にして、履歴を整理する。
    fn finish_batch(&mut self, start: usize, saved_redo: Vec<Entry>, redo_bytes: u64) {
        let added = self.undo.len() - start;
        if added == 0 {
            self.redo = saved_redo;
            self.history_bytes += redo_bytes;
        } else if added > 1 {
            let steps: Vec<Entry> = self.undo.drain(start..).collect();
            // 合計の大きさは同じなので history_bytes はそのまま
            let cost = steps.iter().map(|e| e.cost).sum();
            self.undo.push(Entry {
                command: Command::Compound(steps),
                cost,
            });
        }
        self.end_coalescing();
        self.trim_history();
    }

    /// 積まれた段を新しい方から戻して取り除き、Redo と履歴のバイト数を始める前へ。
    fn rollback_batch(&mut self, start: usize, saved_redo: Vec<Entry>, redo_bytes: u64) {
        while self.undo.len() > start {
            let mut entry = self.undo.pop().expect("まとめの段");
            // 段を戻すのは、その段を当てる前の状態（予算の中だった）へ戻すこと
            self.revert(&mut entry.command)
                .expect("巻き戻しは元の予算内");
            self.history_bytes -= entry.cost;
            self.revision += 1;
        }
        self.redo = saved_redo;
        self.history_bytes += redo_bytes;
        self.end_coalescing();
    }

    /// 複数の段を 1 回の Undo にして実行する（貼り付けの「層を足す」と「選択を外す」）。1 つでも `Compound` に包み、費用は段の
    /// 合計に 32 を足す（C# の `CompositeCommand` の段の分。層を 1 つだけ足す貼り付けも同じ）。途中で断ったら済んだ分を戻して、
    /// 何も積まない。`batch` がまとめる段（費用は合計のまま。C# の `CompoundCommand`）とは別。
    pub(super) fn execute_group(&mut self, steps: Vec<Entry>) -> Result<(), CoreError> {
        let cost = 32 + steps.iter().map(|e| e.cost).sum::<u64>();
        self.execute(Command::Compound(steps), cost)
    }

    /// まとまった段を当てる（先頭から）・戻す（末尾から）。途中で断ったら、済んだ分を反対向きに戻してから断る。
    pub(super) fn switch_compound(
        &mut self,
        steps: &mut [Entry],
        backwards: bool,
    ) -> Result<(), CoreError> {
        let order: Vec<usize> = if backwards {
            (0..steps.len()).rev().collect()
        } else {
            (0..steps.len()).collect()
        };
        for (done, &i) in order.iter().enumerate() {
            if let Err(error) = self.switch(&mut steps[i].command, backwards) {
                for &j in order[..done].iter().rev() {
                    // 済んだ段の反対向きは、その段を当てる前の状態（予算の中だった）へ戻すこと
                    self.switch(&mut steps[j].command, !backwards)
                        .expect("巻き戻しは元の予算内");
                }
                return Err(error);
            }
        }
        Ok(())
    }
}
