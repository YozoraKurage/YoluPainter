//! 保存用の写し（C# の `PaintDocument.CaptureSnapshot`）。
//!
//! 復旧の書き置きは、描いている最中ではない区切りで文書全体の一貫した写しを**安く**取り、正本への詰め直し（`yolu-io` の
//! `NativeDocument::from_core`）と書き込みは別のスレッドで行う。写しは履歴を持たず、画素のタイルは `Arc` を共有する
//! （共有したタイルは、元が次に書くときに元の側が自分のものへ複製する。写しは動かない）。

use super::*;

impl Document {
    /// 履歴を持たない保存用の写しを取る。文書・層・チャンネルの ID と属性・マスク・選択範囲・Normal の設定・手動の ID 色・見た目の設定・
    /// `revision` を保ち、タイルは共有する（画素のコピーはしない）。呼んだあとは元を編集してよく、写しは別のスレッドへ
    /// 渡して読める。進行中のストロークがあれば断り、元は何も変えない。
    pub fn capture_snapshot(&self) -> Result<Document, CoreError> {
        self.ensure_no_stroke()?;
        // 項目を全部挙げる（`..` を使わない）: `Document` に項目が増えたら、写すか、写さない理由を書くかをここで決める。
        let Document {
            id,
            width,
            height,
            tile_size,
            layers,
            channels,
            normal_settings,
            selection,
            id_colors,
            look,
            source_budget,
            stroke_budget,
            undo_budget,
            minimum_undo_steps,
            id_counter,
            revision,
            effects,
            // 写さない: batch の最中の印（写しは batch の外）
            batching: _,
            // 写さない: 履歴とその予算の使用量・進行中のストロークの状態・変化の記録（写しは読むだけで、編集も Undo もしない。
            // 変化の記録は元の合成のためのもので、写しの読み手は使わない）
            undo: _,
            redo: _,
            history_bytes: _,
            active: _,
            material: _,
            triangle_fill: _,
            active_target: _,
            next_stroke: _,
            journal: _,
            coalesce: _,
            trim_count: _,
            trimmed_bytes: _,
        } = self;
        let mut copy = Document::with_tile_size(*width, *height, *tile_size)?;
        copy.id = *id;
        copy.layers = layers.clone();
        copy.channels = channels.clone();
        copy.normal_settings = *normal_settings;
        copy.selection = selection.clone();
        copy.id_colors = id_colors.clone();
        copy.look = look.clone();
        copy.source_budget = *source_budget;
        copy.stroke_budget = *stroke_budget;
        copy.undo_budget = *undo_budget;
        copy.minimum_undo_steps = *minimum_undo_steps;
        copy.id_counter = *id_counter;
        copy.revision = *revision;
        // 効果の入力（メッシュマップ・モデル・画像）と予算は写す（写しで合成しても元と同じ値になる）。評価のキャッシュ・元画素の時計・
        // Anchor の署名は写さない（写しは空のキャッシュから評価し直す。持ち込むと別の内容に同じ鍵が付き得る — edit_copy と同じ決まり）
        copy.effects.inputs = effects.inputs.clone();
        copy.effects.inputs_revision = effects.inputs_revision;
        copy.effects.working_budget = effects.working_budget;
        copy.effects.cache_budget = effects.cache_budget;
        copy.effects.image_cache_budget = effects.image_cache_budget;
        copy.effects.block_pixels = effects.block_pixels;
        Ok(copy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::Tile;
    use crate::{BrushSettings, ChannelInfo, ChannelKind, ColorSpace, Rgba8};
    use glam::DVec2;

    /// 層の Color のタイルの画素の持ち主（共有しているかを `Arc` の同一性で見る）。
    fn buffer(doc: &Document, layer: LayerId, coord: TileCoord) -> std::sync::Arc<Vec<u8>> {
        let surface = doc.layer(layer).unwrap().surface(Channel::Color).unwrap();
        match surface.tiles.get(&coord) {
            Some(Tile::Data(d)) => d.clone(),
            other => panic!("画素のタイルのはず: {other:?}"),
        }
    }
    fn dab(doc: &mut Document, layer: LayerId, x: f64, y: f64, color: Rgba8) {
        let brush = BrushSettings {
            color,
            radius: 6.0,
            ..BrushSettings::default()
        };
        let mut stroke = doc.begin_stroke(layer, &brush).unwrap();
        stroke.add_point(doc, x, y, 1.0, DVec2::ZERO).unwrap();
        doc.end_stroke(stroke).unwrap();
    }

    #[test]
    fn the_snapshot_shares_frozen_tile_buffers_and_the_source_copies_on_its_next_write() {
        let mut doc = Document::with_tile_size(64, 64, 16).unwrap();
        let layer = doc.add_layer("絵").unwrap();
        dab(&mut doc, layer, 5.0, 5.0, Rgba8::new(200, 30, 30, 255));
        dab(&mut doc, layer, 40.0, 40.0, Rgba8::new(30, 30, 200, 255));
        let snap = doc.capture_snapshot().unwrap();
        for coord in [TileCoord::new(0, 0), TileCoord::new(2, 2)] {
            assert!(
                std::sync::Arc::ptr_eq(&buffer(&doc, layer, coord), &buffer(&snap, layer, coord)),
                "{coord:?}: 画素をコピーせず共有する"
            );
        }
        let frozen = snap.composite(snap.bounds()).unwrap();
        // 元が片方のタイルへ書く: 写しの画素は動かず、書かなかったタイルは共有のまま
        dab(&mut doc, layer, 5.0, 5.0, Rgba8::new(10, 240, 10, 255));
        assert_eq!(snap.composite(snap.bounds()).unwrap(), frozen);
        assert_ne!(doc.composite(doc.bounds()).unwrap(), frozen);
        assert!(!std::sync::Arc::ptr_eq(
            &buffer(&doc, layer, TileCoord::new(0, 0)),
            &buffer(&snap, layer, TileCoord::new(0, 0))
        ));
        assert!(std::sync::Arc::ptr_eq(
            &buffer(&doc, layer, TileCoord::new(2, 2)),
            &buffer(&snap, layer, TileCoord::new(2, 2))
        ));
    }

    #[test]
    fn the_snapshot_is_refused_during_a_stroke_and_leaves_the_source_alone() {
        let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
        let layer = doc.add_layer("絵").unwrap();
        let brush = BrushSettings::default();
        let mut stroke = doc.begin_stroke(layer, &brush).unwrap();
        stroke.add_point(&mut doc, 8.0, 8.0, 1.0, DVec2::ZERO).unwrap();
        let revision = doc.revision();
        assert!(matches!(
            doc.capture_snapshot(),
            Err(CoreError::StrokeActive)
        ));
        assert_eq!(doc.revision(), revision);
        assert!(doc.has_active_stroke());
        assert!(doc.end_stroke(stroke).unwrap().changed, "ストロークは続けて終われる");
        assert!(doc.capture_snapshot().is_ok());
    }

    #[test]
    fn a_snapshot_keeps_everything_about_the_document_except_the_history() {
        let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
        let paint = doc.add_layer("絵").unwrap();
        dab(&mut doc, paint, 10.0, 10.0, Rgba8::new(20, 30, 40, 255));
        doc.set_layer_opacity(paint, 0.7, false).unwrap();
        doc.set_layer_clipping(paint, true).unwrap();
        doc.set_channel_opacity(paint, Channel::Color, Some(0.4), false).unwrap();
        doc.add_layer_mask(paint).unwrap();
        doc.set_mask_pixel(paint, 1, 2, 200).unwrap();
        doc.set_layer_mask_density(paint, 0.3, false).unwrap();
        let fill = doc.add_fill_layer("塗り", &[(Channel::Color, Rgba8::new(9, 8, 7, 255))], None).unwrap();
        let group = doc.group_layers(&[fill], "組").unwrap();
        doc.set_layer_locks(paint, crate::LayerLocks::POSITION).unwrap();
        let user = doc
            .add_channel(ChannelInfo {
                name: "Extra".into(),
                kind: ChannelKind::Scalar,
                color_space: ColorSpace::Linear,
                default: Rgba8::new(5, 5, 5, 255),
            })
            .unwrap();
        doc.set_selection(Some(crate::SelectionMask::rectangle(&doc, 0, 0, 9, 10))).unwrap();
        doc.set_id_colors(
            crate::mesh_maps::IdColorAssignments::new("a".repeat(64), [(0, 0x123456)].into_iter().collect()).unwrap(),
        )
        .unwrap();
        assert!(doc.can_undo());
        let snap = doc.capture_snapshot().unwrap();
        assert_eq!((snap.id(), snap.revision()), (doc.id(), doc.revision()));
        assert_eq!(
            (snap.width(), snap.height(), snap.tile_size()),
            (32, 32, 16)
        );
        assert!(!snap.can_undo() && !snap.can_redo() && snap.history_bytes() == 0);
        assert_eq!(snap.channels(), doc.channels());
        assert_eq!(snap.channel_info(user), doc.channel_info(user));
        assert_eq!(snap.selection().unwrap(), doc.selection().unwrap());
        assert_eq!(snap.id_colors().colors(), doc.id_colors().colors());
        assert_eq!(snap.layers().len(), doc.layers().len());
        for (a, b) in snap.layers().iter().zip(doc.layers()) {
            assert_eq!(
                (a.id(), a.name(), a.parent(), a.opacity(), a.locks(), a.clipping()),
                (b.id(), b.name(), b.parent(), b.opacity(), b.locks(), b.clipping())
            );
        }
        assert_eq!(snap.layer(group).unwrap().name(), "組");
        let mask = snap.layer(paint).unwrap().mask().unwrap();
        assert_eq!(mask.density(), 0.3);
        assert_eq!(snap.composite(snap.bounds()).unwrap(), doc.composite(doc.bounds()).unwrap());
        for channel in [Channel::Color, Channel::Roughness, user] {
            assert_eq!(
                snap.composite_channel(channel, snap.bounds()).unwrap(),
                doc.composite_channel(channel, doc.bounds()).unwrap()
            );
        }
    }

    #[test]
    fn later_edits_and_undo_of_the_source_do_not_reach_the_snapshot() {
        let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
        let layer = doc.add_layer("絵").unwrap();
        dab(&mut doc, layer, 10.0, 10.0, Rgba8::new(20, 30, 40, 255));
        doc.add_layer_mask(layer).unwrap();
        doc.set_selection(Some(crate::SelectionMask::rectangle(&doc, 0, 0, 9, 10))).unwrap();
        let snap = doc.capture_snapshot().unwrap();
        let (pixels, selection, revision) = (
            snap.composite(snap.bounds()).unwrap(),
            snap.selection().unwrap().clone(),
            snap.revision(),
        );
        // マスクの画素を直に書く口は履歴を消すので、先に書く（このあとの編集は Undo できる）
        doc.set_mask_pixel(layer, 3, 3, 90).unwrap();
        dab(&mut doc, layer, 20.0, 20.0, Rgba8::new(255, 0, 0, 255));
        doc.set_layer_name(layer, "変えた").unwrap();
        doc.clear_selection().unwrap();
        doc.remove_layer(layer).unwrap();
        assert!(doc.undo().unwrap());
        assert!(doc.undo().unwrap());
        assert_eq!(snap.composite(snap.bounds()).unwrap(), pixels);
        assert_eq!(snap.selection().unwrap(), &selection);
        assert_eq!(snap.layer(layer).unwrap().name(), "絵");
        assert_eq!(snap.revision(), revision);
        assert!(snap.revision() < doc.revision());
        assert!(!snap.can_undo());
    }

    #[test]
    fn a_snapshot_can_be_read_on_another_thread_while_the_source_keeps_painting() {
        fn assert_send<T: Send + 'static>() {}
        assert_send::<Document>();
        let mut doc = Document::with_tile_size(64, 64, 16).unwrap();
        let layer = doc.add_layer("絵").unwrap();
        dab(&mut doc, layer, 10.0, 10.0, Rgba8::new(20, 30, 40, 255));
        let snap = doc.capture_snapshot().unwrap();
        let expected = snap.composite(snap.bounds()).unwrap();
        let reader = std::thread::spawn(move || {
            (0..20)
                .all(|_| snap.composite(snap.bounds()).unwrap() == expected)
        });
        for i in 0..20 {
            dab(&mut doc, layer, 10.0 + i as f64, 10.0, Rgba8::new(200, 0, 0, 255));
        }
        assert!(reader.join().unwrap());
    }

    #[test]
    fn the_snapshot_copies_no_pixels_however_large_the_document() {
        let mut doc = Document::with_tile_size(2048, 2048, 128).unwrap();
        let layer = doc.add_layer("絵").unwrap();
        for i in 0..16 {
            dab(&mut doc, layer, 64.0 + 128.0 * i as f64, 64.0 + 128.0 * i as f64, Rgba8::new(i as u8 * 10, 90, 30, 255));
        }
        let snap = doc.capture_snapshot().unwrap();
        let shared = snap
            .layer(layer)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .tile_coords()
            .into_iter()
            .filter(|c| std::sync::Arc::ptr_eq(&buffer(&doc, layer, *c), &buffer(&snap, layer, *c)))
            .count();
        assert_eq!(shared, snap.layer(layer).unwrap().surface(Channel::Color).unwrap().tile_count());
        assert!(shared >= 16);
    }
}
