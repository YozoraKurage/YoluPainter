//! 見た目の設定（[`crate::look::MaterialLook`]）の読み書きと Undo。画素も合成も変えないので、変化の記録（タイル）は付けない。

use std::sync::Arc;

use super::{CoalesceKey, Command, Document};
use crate::look::MaterialLook;
use crate::CoreError;

impl Document {
    /// 見た目の設定（3D ビューの描き方と lilToon の値）。
    pub fn look(&self) -> &MaterialLook {
        &self.look
    }

    /// 見た目の設定を変える。1 回の Undo（`coalesce` ならスライダーのドラッグを 1 段にまとめる。まとめは `end_coalescing` まで）。
    /// 同じ設定なら何もしない。形の検査（[`MaterialLook::validate`]）に通らない設定は断り、何も変えない。
    pub fn set_look(&mut self, look: MaterialLook, coalesce: bool) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        look.validate()?;
        if *self.look == look {
            return Ok(());
        }
        let cost = self.look.history_cost() + look.history_cost();
        let old = self.look.clone();
        self.record(
            Command::Look {
                old,
                new: Arc::new(look),
            },
            cost,
            coalesce.then_some(CoalesceKey::Look),
        )
    }

    /// 読み込み直後に見た目の設定を戻す。履歴・版を増やさない（読み込み直後だけ。履歴があれば断る）。
    pub fn restore_look(&mut self, look: MaterialLook) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        if !self.undo.is_empty() || !self.redo.is_empty() {
            return Err(CoreError::Unsupported("見た目の設定の復元は読み込み直後だけ"));
        }
        look.validate()?;
        self.look = Arc::new(look);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::look::{LookKind, LookValue, MaterialLook, TextureSource};
    use crate::{Channel, ChannelInfo, ChannelKind, ColorSpace, Document, HistoryKind, Rgba8};

    fn lil(border: f32) -> MaterialLook {
        let mut look = MaterialLook {
            kind: LookKind::LilToon,
            ..MaterialLook::default()
        };
        look.properties
            .insert("_ShadowBorder".into(), LookValue::Float(border));
        look
    }

    #[test]
    fn a_look_change_is_one_undo_step_and_keeps_pixels_untouched() {
        let mut doc = Document::new(16, 16).unwrap();
        let serial = doc.change_serial();
        let revision = doc.revision();
        doc.set_look(lil(0.3), false).unwrap();
        assert_eq!(doc.look().kind, LookKind::LilToon);
        assert!(doc.revision() > revision, "版は進む（保存と描き直しの鍵）");
        assert_eq!(doc.change_serial(), serial, "タイルの変化は無い");
        assert_eq!(doc.history().last(), Some(HistoryKind::Look));
        // 同じ設定は段を積まない
        let steps = doc.undo_count();
        doc.set_look(lil(0.3), false).unwrap();
        assert_eq!(doc.undo_count(), steps);
        assert!(doc.undo().unwrap());
        assert!(doc.look().is_default());
        assert!(doc.redo().unwrap());
        assert_eq!(doc.look().float("_ShadowBorder", 0.5), 0.3);
    }

    #[test]
    fn slider_drags_coalesce_until_the_drag_ends() {
        let mut doc = Document::new(16, 16).unwrap();
        doc.set_look(lil(0.1), true).unwrap();
        doc.set_look(lil(0.2), true).unwrap();
        doc.set_look(lil(0.3), true).unwrap();
        assert_eq!(doc.undo_count(), 1);
        doc.end_coalescing();
        doc.set_look(lil(0.4), true).unwrap();
        assert_eq!(doc.undo_count(), 2);
        doc.undo().unwrap();
        assert_eq!(doc.look().float("_ShadowBorder", 0.5), 0.3);
        doc.undo().unwrap();
        assert!(doc.look().is_default(), "まとめた段の前（最初の変更の前）へ戻る");
        // Escape で止めたドラッグは段ごと捨てる
        doc.set_look(lil(0.7), true).unwrap();
        doc.set_look(lil(0.8), true).unwrap();
        assert!(doc.cancel_coalescing().unwrap());
        assert!(doc.look().is_default());
        assert_eq!(doc.undo_count(), 0);
    }

    #[test]
    fn invalid_looks_are_refused_without_change() {
        let mut doc = Document::new(16, 16).unwrap();
        let mut bad = lil(0.5);
        bad.properties
            .insert("_X".into(), LookValue::Float(f32::NAN));
        assert!(doc.set_look(bad.clone(), false).is_err());
        assert!(doc.look().is_default());
        assert_eq!(doc.undo_count(), 0);
        assert!(doc.restore_look(bad).is_err());
    }

    #[test]
    fn restore_is_only_for_a_fresh_document_and_adds_no_history() {
        let mut doc = Document::new(16, 16).unwrap();
        let revision = doc.revision();
        doc.restore_look(lil(0.4)).unwrap();
        assert_eq!(doc.revision(), revision);
        assert_eq!(doc.undo_count(), 0);
        assert_eq!(doc.look().float("_ShadowBorder", 0.5), 0.4);
        doc.add_layer("a").unwrap();
        assert!(doc.restore_look(lil(0.1)).is_err());
    }

    #[test]
    fn snapshots_and_batches_carry_the_look() {
        let mut doc = Document::new(16, 16).unwrap();
        doc.set_look(lil(0.25), false).unwrap();
        let copy = doc.capture_snapshot().unwrap();
        assert_eq!(copy.look(), doc.look());
        // ひな形のように、チャンネルを足して割り当てるのを 1 段に
        let before = doc.undo_count();
        doc.batch(|d| {
            let channel = d.add_channel(ChannelInfo {
                name: "影の強さ".into(),
                kind: ChannelKind::Scalar,
                color_space: ColorSpace::Linear,
                default: Rgba8::new(255, 255, 255, 255),
            })?;
            let mut look = d.look().clone();
            look.textures
                .insert("_ShadowStrengthMask".into(), TextureSource::Channel(channel));
            d.set_look(look, false)
        })
        .unwrap();
        assert_eq!(doc.undo_count(), before + 1);
        assert_eq!(doc.look().channels().len(), 1);
        doc.undo().unwrap();
        assert!(doc.look().textures.is_empty());
        assert_eq!(doc.channels().len(), Channel::STANDARD_COUNT);
    }

    #[test]
    fn layer_operations_do_not_touch_the_look() {
        let mut doc = Document::new(16, 16).unwrap();
        doc.set_look(lil(0.6), false).unwrap();
        let a = doc.add_layer("a").unwrap();
        doc.add_layer("b").unwrap();
        // 準備用の文書で行う操作（グループ化）
        doc.group_layers(&[a], "g").unwrap();
        doc.undo().unwrap();
        assert_eq!(doc.look().float("_ShadowBorder", 0.5), 0.6);
    }
}
