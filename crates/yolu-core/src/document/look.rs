//! 見た目の設定（[`crate::look::MaterialLook`]）の読み書きと Undo。画素も合成も変えないので、変化の記録（タイル）は付けない。

use std::sync::Arc;

use super::{CoalesceKey, Command, Document};
use crate::look::{MaterialLook, ReceivedLook};
use crate::CoreError;

impl Document {
    /// 見た目の設定（利用者の設定。3D ビューの描き方と lilToon の値）。描くときは [`Document::drawn_look`]。
    pub fn look(&self) -> &MaterialLook {
        &self.look
    }

    /// 描く見た目: 受けた見た目（[`Document::received_look`]）があればその値の上に利用者の設定を重ねたもの、無ければ利用者の設定。
    pub fn drawn_look(&self) -> &MaterialLook {
        self.drawn_look.as_deref().unwrap_or(&self.look)
    }

    /// 外から受けた見た目（Live Link。無ければ None）。
    pub fn received_look(&self) -> Option<&ReceivedLook> {
        self.received_look.as_deref()
    }

    /// 描く見た目が変わるたびに増える番号（利用者の設定・受けた見た目・Undo・Redo のどれでも。描き直しの鍵）。
    pub fn look_serial(&self) -> u64 {
        self.look_serial
    }

    /// 外から受けた見た目を置き換える（None は外す）。Undo の段も版も増やさない（受け取りは利用者の操作ではなく、Undo で戻すと
    /// 外の本物と食い違う。保存するかは呼び手が決める）。描いている間も置き換えられる（画素には触らない）。同じものなら何もしない。
    /// 形の検査に通らないものは断り、何も変えない。変えたら true。
    pub fn set_received_look(&mut self, received: Option<ReceivedLook>) -> Result<bool, CoreError> {
        if let Some(r) = &received {
            r.validate()?;
        }
        if self.received_look.as_deref() == received.as_ref() {
            return Ok(false);
        }
        self.received_look = received.map(Arc::new);
        self.refresh_drawn_look();
        Ok(true)
    }

    /// 描く見た目を作り直す（利用者の設定か受けた見た目が変わったら）。
    pub(super) fn refresh_drawn_look(&mut self) {
        self.drawn_look = self
            .received_look
            .as_ref()
            .map(|r| Arc::new(self.look.over(&r.look)));
        self.look_serial = self.look_serial.wrapping_add(1);
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
            return Err(CoreError::Unsupported(
                "見た目の設定の復元は読み込み直後だけ",
            ));
        }
        look.validate()?;
        self.look = Arc::new(look);
        self.refresh_drawn_look();
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
        assert!(
            doc.look().is_default(),
            "まとめた段の前（最初の変更の前）へ戻る"
        );
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
            look.textures.insert(
                "_ShadowStrengthMask".into(),
                TextureSource::Channel(channel),
            );
            d.set_look(look, false)
        })
        .unwrap();
        assert_eq!(doc.undo_count(), before + 1);
        assert_eq!(doc.look().channels().len(), 1);
        doc.undo().unwrap();
        assert!(doc.look().textures.is_empty());
        assert_eq!(doc.channels().len(), Channel::STANDARD_COUNT);
    }

    fn received(border: f32) -> crate::look::ReceivedLook {
        let mut look = lil(border);
        look.shader = "Hidden/lilToonTransparent".into();
        look.properties.insert(
            "_ShadowColor".into(),
            LookValue::Color([0.1, 0.2, 0.3, 1.0]),
        );
        look.textures
            .insert("_MainTex".into(), TextureSource::Channel(Channel::Color));
        crate::look::ReceivedLook {
            look,
            source: "lilToon 2.3.4".into(),
            ..Default::default()
        }
    }

    #[test]
    fn received_values_are_drawn_without_history_and_the_users_items_win() {
        let mut doc = Document::new(16, 16).unwrap();
        let (revision, serial) = (doc.revision(), doc.look_serial());
        assert!(doc.set_received_look(Some(received(0.2))).unwrap());
        assert_eq!(doc.undo_count(), 0, "受け取りは Undo に入らない");
        assert_eq!(
            doc.revision(),
            revision,
            "版も進めない（保存の鍵を変えない）"
        );
        assert!(doc.look_serial() > serial, "描き直しの鍵は進む");
        // 利用者の設定は既定のまま、描く見た目は受けた値（描き方も lilToon）
        assert!(doc.look().is_default());
        let drawn = doc.drawn_look();
        assert_eq!(drawn.kind, LookKind::LilToon);
        assert_eq!(drawn.shader, "Hidden/lilToonTransparent");
        assert_eq!(drawn.float("_ShadowBorder", 0.5), 0.2);
        // 同じものはもう一度受けても変えない
        let serial = doc.look_serial();
        assert!(!doc.set_received_look(Some(received(0.2))).unwrap());
        assert_eq!(doc.look_serial(), serial);

        // 利用者が欄で 1 項目だけ変える: その項目だけが勝ち、ほかは受けた値のまま
        let mut mine = doc.look().clone();
        mine.properties
            .insert("_ShadowBorder".into(), LookValue::Float(0.7));
        doc.set_look(mine, false).unwrap();
        assert_eq!(doc.drawn_look().float("_ShadowBorder", 0.5), 0.7);
        assert_eq!(
            doc.drawn_look().vec4("_ShadowColor", [0.0; 4]),
            [0.1, 0.2, 0.3, 1.0]
        );
        // 新しい値を受けても、利用者の項目は残る
        doc.set_received_look(Some(received(0.4))).unwrap();
        assert_eq!(doc.drawn_look().float("_ShadowBorder", 0.5), 0.7);
        // 利用者の変更を Undo しても、受けた値は残る（Undo は受けた値を戻さない）
        doc.undo().unwrap();
        assert_eq!(doc.drawn_look().float("_ShadowBorder", 0.5), 0.4);
        assert!(doc.received_look().is_some());
        // 外すと利用者の設定だけ（標準）
        doc.set_received_look(None).unwrap();
        assert_eq!(doc.drawn_look(), doc.look());
        assert_eq!(doc.drawn_look().kind, LookKind::Standard);
    }

    #[test]
    fn the_kind_follows_the_received_look_until_the_user_chooses() {
        let mut doc = Document::new(16, 16).unwrap();
        doc.set_received_look(Some(received(0.2))).unwrap();
        // 選んで標準にする: 受けた値があっても標準で描く
        let chosen = MaterialLook {
            kind: LookKind::Standard,
            kind_chosen: true,
            ..MaterialLook::default()
        };
        doc.set_look(chosen.clone(), false).unwrap();
        assert_eq!(doc.drawn_look().kind, LookKind::Standard);
        assert!(!doc.look().is_default(), "選んだことは保存する");
        // 利用者のシェーダー・キーワードは空でなければ勝つ
        let mut mine = chosen;
        mine.kind = LookKind::LilToon;
        mine.shader = "lilToon".into();
        mine.keywords = vec!["A".into()];
        doc.set_look(mine, false).unwrap();
        let drawn = doc.drawn_look();
        assert_eq!(
            (drawn.kind, drawn.shader.as_str()),
            (LookKind::LilToon, "lilToon")
        );
        assert_eq!(drawn.keywords, vec!["A".to_owned()]);
        // 受けたスロットのチャンネルは利用者のスロットと合わさる
        assert_eq!(
            drawn.textures.get("_MainTex"),
            Some(&TextureSource::Channel(Channel::Color))
        );
    }

    #[test]
    fn received_looks_are_checked_and_carried_by_snapshots() {
        let mut doc = Document::new(16, 16).unwrap();
        let mut bad = received(0.5);
        bad.images.insert(
            "_MatCapTex".into(),
            std::sync::Arc::new(crate::look::ReceivedImage {
                width: 2,
                height: 2,
                srgb: true,
                pixels: vec![0u8; 15].into(),
            }),
        );
        assert!(doc.set_received_look(Some(bad.clone())).is_err());
        assert!(doc.received_look().is_none());
        bad.images.insert(
            "_MatCapTex".into(),
            std::sync::Arc::new(crate::look::ReceivedImage {
                width: 2,
                height: 2,
                srgb: true,
                pixels: vec![0u8; 16].into(),
            }),
        );
        doc.set_received_look(Some(bad)).unwrap();
        assert_eq!(doc.received_look().unwrap().image_bytes(), 16);
        let copy = doc.capture_snapshot().unwrap();
        assert_eq!(copy.received_look(), doc.received_look());
        assert_eq!(copy.drawn_look(), doc.drawn_look());
        // 受けた見た目は描いている間も置き換えられる（画素に触らない）
        let layer = doc.add_layer("a").unwrap();
        let stroke = doc
            .begin_stroke(layer, &crate::BrushSettings::default())
            .unwrap();
        assert!(doc.set_received_look(None).unwrap());
        doc.cancel_stroke(stroke);
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
