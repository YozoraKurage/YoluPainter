//! M2 のレイヤー（グループ・マスク・塗りつぶし・調整・クリッピング・チャンネルごとの有効と合成・Normal の設定）の正本と core の行き来。
//! 正解は C# の実際の書き手（`tools/io-fixtures/M2Fixture.cs`）が作った正本と、同じ文書の全チャンネルの合成。
use yolu_core::{
    AdjustmentSettings, BlendMode, Channel, ChannelBlend, ChannelInfo, ChannelKind, ColorSpace,
    Document, HeightEdgeMode, LayerId, LayerKind, NormalSettings, NormalYDirection, Rect, Rgba8,
};
use yolu_io::{NativeDocument, NativeValue, Project, SetSpec, WriterInfo};

const FIXTURES: [&str; 6] = [
    "m2-groups",
    "m2-masks",
    "m2-channels",
    "m2-clipping",
    "m2-tiny",
    "locks-v21",
];

fn read(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn native(name: &str) -> NativeDocument {
    NativeDocument::read(&read(&format!("{name}.utpaint"))).unwrap()
}

/// C# の `M2Fixture.Save` と同じ並び: 全チャンネル（番号の順）の合成、続けて Normal のファイル出力。
fn composites(doc: &Document) -> Vec<u8> {
    let rect = Rect {
        x: 0,
        y: 0,
        width: doc.width(),
        height: doc.height(),
    };
    let mut out = Vec::new();
    for c in Channel::ALL {
        out.extend(doc.composite_channel(c, rect).unwrap());
    }
    out.extend(doc.normal_file_output(u64::MAX).unwrap());
    out
}

#[test]
fn csharp_m2_documents_roundtrip_byte_for_byte_and_composite_like_the_recorded_bytes() {
    for name in FIXTURES {
        let original = read(&format!("{name}.utpaint"));
        let native = NativeDocument::read(&original).unwrap();
        assert_eq!(native.version(), 21, "{name}");
        assert!(
            native.core_issues().is_empty(),
            "{name}: {:?}",
            native.core_issues()
        );
        let core = native.to_core().unwrap();
        assert_eq!(
            core.undo_count(),
            0,
            "{name}: 読み込みは Undo の履歴に残さない"
        );
        assert!(!core.can_undo() && !core.can_redo(), "{name}");
        assert_eq!(
            NativeDocument::from_core(&core).unwrap().to_bytes(),
            original,
            "{name}: C# の正本を core にして書き戻すとバイト一致"
        );
        let got = composites(&core);
        if std::env::var_os("YOLU_GOLDEN_UPDATE").is_some()
            && got != read(&format!("{name}.composite"))
        {
            let path = format!(
                "{}/tests/fixtures/{name}.composite",
                env!("CARGO_MANIFEST_DIR")
            );
            std::fs::write(path, &got).unwrap();
            continue;
        }
        // 正解の合成は、合成の式が f32 の core の式になってから core で撮り直した（`YOLU_GOLDEN_UPDATE=1`）
        assert_eq!(
            got,
            read(&format!("{name}.composite")),
            "{name}: 全チャンネルの合成と Normal のファイル出力が正解と全バイト一致"
        );
    }
}

// ───────── 補助 ─────────

fn index_of(native: &NativeDocument, name: &str) -> usize {
    (0..native.layer_count())
        .find(|i| {
            native.field(&format!("layers[{i}].name")) == Some(&NativeValue::Text(name.into()))
        })
        .unwrap_or_else(|| panic!("レイヤー「{name}」がありません"))
}

fn info(name: &str, kind: ChannelKind, color_space: ColorSpace, default: [u8; 4]) -> ChannelInfo {
    ChannelInfo {
        name: name.into(),
        kind,
        color_space,
        default: Rgba8::new(default[0], default[1], default[2], default[3]),
    }
}

/// 文書とレイヤーの ID を決まった値にする（正解のファイルに同じバイト列を出すため）。
fn fixed(doc: Document) -> Document {
    let ids: Vec<LayerId> = (0..doc.layers().len())
        .map(|i| LayerId(0x7000 + i as u128))
        .collect();
    doc.with_persistent_ids(0x7777, &ids).unwrap()
}

fn paint(doc: &mut Document, id: LayerId, channel: Channel, seed: u32) {
    for y in 0..doc.height() {
        for x in 0..doc.width() {
            if (x + 2 * y + seed).is_multiple_of(3) {
                continue;
            }
            let v = (x * 37 + y * 19 + seed * 57) as u8;
            let px = Rgba8::new(v, v.wrapping_mul(3), v.wrapping_add(90), v | 1);
            doc.set_channel_pixel(id, channel, x, y, px).unwrap();
        }
    }
}

/// ユーザーチャンネル 3 つ（番号 6・8・9。7 は足してから消した歯抜け）と、それを使う全部のレイヤーの種類。
fn user_channel_document() -> Document {
    let mut doc = Document::with_tile_size(20, 12, 8).unwrap();
    let ao = doc
        .add_channel(info(
            "AO",
            ChannelKind::Scalar,
            ColorSpace::Linear,
            [255, 255, 255, 255],
        ))
        .unwrap();
    let gap = doc
        .add_channel(info(
            "一時",
            ChannelKind::Color,
            ColorSpace::Srgb,
            [1, 2, 3, 4],
        ))
        .unwrap();
    let tint = doc
        .add_channel(info(
            "Tint",
            ChannelKind::Color,
            ColorSpace::Srgb,
            [10, 20, 30, 255],
        ))
        .unwrap();
    let detail = doc
        .add_channel(info(
            "Detail Normal",
            ChannelKind::Normal,
            ColorSpace::Linear,
            [128, 128, 255, 255],
        ))
        .unwrap();
    assert_eq!(
        [ao.index(), gap.index(), tint.index(), detail.index()],
        [6, 7, 8, 9]
    );
    doc.remove_channel(gap).unwrap();
    let raster = doc.add_layer("ユーザーの面").unwrap();
    paint(&mut doc, raster, Channel::Color, 1);
    paint(&mut doc, raster, ao, 2);
    paint(&mut doc, raster, tint, 3);
    paint(&mut doc, raster, detail, 4);
    doc.set_channel_enabled(raster, ao, false).unwrap();
    doc.set_channel_blend(
        raster,
        tint,
        ChannelBlend::new(Some(BlendMode::Multiply), Some(0.5)),
        false,
    )
    .unwrap();
    doc.add_layer_mask(raster).unwrap();
    doc.set_mask_pixel(raster, 3, 3, 200).unwrap();
    let fill = doc
        .add_fill_layer(
            "ユーザーの塗り",
            &[
                (Channel::Color, Rgba8::new(200, 30, 60, 255)),
                (ao, Rgba8::new(90, 90, 90, 255)),
                (tint, Rgba8::new(10, 220, 90, 128)),
            ],
            None,
        )
        .unwrap();
    doc.set_channel_enabled(fill, ao, false).unwrap();
    doc.add_adjustment_layer(
        "色相",
        AdjustmentSettings::hue_saturation(40.0, 0.25, -0.1).unwrap(),
        Some(&[Channel::Color, tint]),
        None,
    )
    .unwrap();
    doc.add_adjustment_layer(
        "レベル",
        AdjustmentSettings::levels(0.1, 0.9, 1.4, 0.0, 1.0).unwrap(),
        Some(&[ao, detail]),
        None,
    )
    .unwrap();
    let inner = doc.add_layer("グループの中").unwrap();
    paint(&mut doc, inner, tint, 5);
    let group = doc.group_layers(&[inner], "ユーザーのグループ").unwrap();
    doc.set_channel_blend(group, detail, ChannelBlend::new(None, Some(0.25)), false)
        .unwrap();
    fixed(doc)
}

fn without_user_channels() -> Document {
    let mut doc = Document::with_tile_size(20, 12, 8).unwrap();
    let layer = doc.add_layer("標準だけ").unwrap();
    paint(&mut doc, layer, Channel::Color, 9);
    fixed(doc)
}

/// 文書にあるチャンネル全部の合成（ユーザーチャンネルも）。
fn every_channel(doc: &Document) -> Vec<Vec<u8>> {
    let rect = doc.bounds();
    doc.channels()
        .into_iter()
        .map(|c| doc.composite_channel(c, rect).unwrap())
        .collect()
}

fn assert_same_document(a: &Document, b: &Document, what: &str) {
    assert_eq!(a.id(), b.id(), "{what}");
    assert_eq!(a.channels(), b.channels(), "{what}");
    for c in a.channels() {
        assert_eq!(a.channel_info(c), b.channel_info(c), "{what}: {c:?}");
    }
    assert_eq!(a.layers().len(), b.layers().len(), "{what}");
    for (x, y) in a.layers().iter().zip(b.layers()) {
        assert_eq!(x.id(), y.id(), "{what}");
        assert_eq!(x.name(), y.name(), "{what}");
        assert_eq!(x.kind(), y.kind(), "{what}");
        assert_eq!(x.parent(), y.parent(), "{what}");
        assert_eq!(x.enabled_channels(), y.enabled_channels(), "{what}");
        assert_eq!(
            x.channel_blends().collect::<Vec<_>>(),
            y.channel_blends().collect::<Vec<_>>(),
            "{what}"
        );
    }
    assert_eq!(every_channel(a), every_channel(b), "{what}: 合成");
}

// ───────── C# の正解の文書が持つもの ─────────

#[test]
fn m2_fixtures_hold_what_they_claim() {
    let count = |doc: &Document, kind| doc.layers().iter().filter(|l| l.kind() == kind).count();
    let groups = native("m2-groups").to_core().unwrap();
    assert!(count(&groups, LayerKind::Group) >= 7);
    assert!(count(&groups, LayerKind::Fill) >= 1);
    assert!(count(&groups, LayerKind::Adjustment) >= 1);
    let deepest = groups
        .layers()
        .iter()
        .map(|l| groups.depth_of(l.id()).unwrap())
        .max()
        .unwrap();
    assert!(deepest >= 3, "入れ子の深さ {deepest}");
    assert!(groups
        .layers()
        .iter()
        .any(|l| l.is_group() && l.blend_mode() == BlendMode::PassThrough));
    assert!(groups
        .layers()
        .iter()
        .any(|l| l.is_group() && l.blend_mode() != BlendMode::PassThrough));
    assert!(groups.layers().iter().any(|l| !l.visible()));
    assert!(groups.layers().iter().any(|l| l.clipping()));
    assert!(groups
        .layers()
        .iter()
        .any(|l| l.kind() == LayerKind::Group
            && groups.children_of(Some(l.id())).unwrap().is_empty()));

    let masks = native("m2-masks").to_core().unwrap();
    let masked: Vec<_> = masks.layers().iter().filter_map(|l| l.mask()).collect();
    assert_eq!(masked.len(), 7);
    assert!(masked.iter().any(|m| m.inverted()));
    assert!(masked.iter().any(|m| !m.enabled()));
    assert!(masked.iter().any(|m| m.density() < 1.0));
    assert!(masked.iter().any(|m| m.surface().tile_count() == 0));
    assert!(masks
        .layers()
        .iter()
        .any(|l| l.is_group() && l.mask().is_some()));

    let channels = native("m2-channels").to_core().unwrap();
    let settings = channels.normal_settings();
    assert!(settings.derive_from_height() && settings.strength() == -3.5);
    assert_eq!(
        channels
            .layers()
            .iter()
            .map(|l| l.channel_blends().count())
            .sum::<usize>(),
        7
    );
    let all = channels
        .layers()
        .iter()
        .find(|l| l.name() == "全チャンネル")
        .unwrap();
    assert_eq!(all.surface_channels().len(), 6);
    assert!(!all.is_channel_enabled(Channel::Metallic));
    let fill = channels
        .layers()
        .iter()
        .find(|l| l.name() == "塗り 3 つ")
        .unwrap();
    assert!(!fill.is_channel_enabled(Channel::Roughness));
    assert_eq!(
        fill.fill_value(Channel::Roughness),
        Some(Rgba8::new(200, 200, 200, 255))
    );
    assert_eq!(fill.fill_value(Channel::Metallic), None);
    let invert = channels
        .layers()
        .iter()
        .find(|l| l.name() == "反転（チャンネルなし）")
        .unwrap();
    assert!(invert.enabled_channels().is_empty());

    let clipping = native("m2-clipping").to_core().unwrap();
    assert!(clipping.layers().iter().filter(|l| l.clipping()).count() >= 8);
    assert!(clipping.is_effectively_clipped(1));
    assert!(!clipping.is_effectively_clipped(0), "一番下の印は効かない");

    let tiny = native("m2-tiny").to_core().unwrap();
    assert_eq!((tiny.width(), tiny.height()), (1, 1));
    assert!(!tiny.normal_settings().derive_from_height());
    assert_eq!(tiny.layers()[0].opacity(), 0.0);
}

#[test]
fn csharp_native_versions_with_m2_layers_convert_and_save_as_the_current_version() {
    // 旧版の基礎配置（版 1〜21。マスク・ノーマル設定などを含む）は全部 core にでき、今の版で書き戻しても絵が変わらない
    for v in 1..=21 {
        let name = format!("native-v{v}.utpaint");
        let native = NativeDocument::read(&read(&name)).unwrap();
        assert_eq!(native.version(), v, "{name}");
        assert!(
            native.core_issues().is_empty(),
            "{name}: {:?}",
            native.core_issues()
        );
        let core = native.to_core().unwrap();
        let saved = NativeDocument::from_core(&core).unwrap();
        assert_eq!(saved.version(), 21, "{name}");
        assert_same_document(&core, &saved.to_core().unwrap(), &name);
    }
}

/// C# の旧版の基礎配置（`tools/io-fixtures/Generate.cs` の `Baseline`）が持つ既知の値が、core に落ちずに届く。上の試験は core を通した
/// 書き戻しとの自己照合なので、読みが値を落としても両側が同じに落として通る。ここでは正解の値そのものを core の側で確かめる。
#[test]
fn csharp_native_versions_carry_their_known_values_into_core() {
    for v in 1..=21 {
        let name = format!("native-v{v}.utpaint");
        let core = NativeDocument::read(&read(&name))
            .unwrap()
            .to_core()
            .unwrap();
        assert_eq!((core.width(), core.height(), core.tile_size()), (9, 10, 8));
        assert_eq!(core.layers().len(), 1, "{name}");
        let layer = &core.layers()[0];
        assert_eq!(layer.name(), "日本語の層", "{name}");
        assert_eq!(layer.kind(), LayerKind::Raster, "{name}");
        assert!(layer.visible(), "{name}");
        assert_eq!(layer.opacity(), 0.625, "{name}");
        assert_eq!(layer.blend_mode(), BlendMode::ColorBurn, "{name}");
        assert_eq!(layer.parent(), None, "{name}");
        // クリッピングは版 5 から（版 5〜11 は bool、版 12 から属性の bit）
        assert_eq!(layer.clipping(), v >= 5, "{name}: クリッピング");
        assert_eq!(layer.surface_channels(), [Channel::Color], "{name}");
        assert_eq!(layer.enabled_channels(), [Channel::Color], "{name}");
        // 画素: タイル (0, 0) は ((i * 37 + 11) as u8)、最初の画素のアルファだけ 0（透明画素の RGB を保つ）
        for y in 0..8u32 {
            for x in 0..8u32 {
                let i = ((y * 8 + x) * 4) as usize;
                let byte = |k: usize| ((i + k) * 37 + 11) as u8;
                let alpha = if i == 0 { 0 } else { byte(3) };
                assert_eq!(
                    layer.pixel(Channel::Color, x, y).unwrap(),
                    Rgba8::new(byte(0), byte(1), byte(2), alpha),
                    "{name}: ({x}, {y})"
                );
            }
        }
        // マスクは版 2 から。有効・反転なし・濃度 0.75、タイル (1, 1) の先頭の画素 2 つだけ値がある
        match layer.mask() {
            None => assert!(v < 2, "{name}: マスクが落ちた"),
            Some(mask) => {
                assert!(v >= 2, "{name}");
                assert!(mask.enabled() && !mask.inverted(), "{name}");
                assert_eq!(mask.density(), 0.75, "{name}");
                assert_eq!(mask.surface().tile_count(), 1, "{name}");
                assert_eq!(mask.surface().pixel(8, 8).unwrap(), Rgba8::new(0, 0, 0, 93));
                assert_eq!(
                    mask.surface().pixel(8, 9).unwrap(),
                    Rgba8::new(0, 0, 0, 128)
                );
                assert_eq!(mask.surface().pixel(0, 0).unwrap().a, 0, "{name}");
            }
        }
        // Normal の設定は版 7 から（強さ -0.0・端は Clamp・ファイルは DirectX）。前は C# の既定（強さ 4・Clamp・OpenGL）
        let settings = core.normal_settings();
        if v >= 7 {
            assert!(!settings.derive_from_height(), "{name}");
            assert_eq!(settings.strength().to_bits(), (-0.0f64).to_bits(), "{name}");
            assert_eq!(settings.edges(), HeightEdgeMode::Clamp, "{name}");
            assert_eq!(
                settings.file_direction(),
                NormalYDirection::DirectX,
                "{name}"
            );
        } else {
            assert_eq!(settings, NormalSettings::DEFAULT, "{name}");
        }
    }
}

// ───────── ユーザーチャンネル（正本の版 22） ─────────

#[test]
fn user_channels_are_written_as_version_22_and_round_trip() {
    let doc = user_channel_document();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), 22);
    assert!(
        native.core_issues().is_empty(),
        "{:?}",
        native.core_issues()
    );
    let restored = native.to_core().unwrap();
    assert_eq!(restored.undo_count(), 0);
    assert_same_document(&doc, &restored, "版 22");
    assert_eq!(
        NativeDocument::from_core(&restored).unwrap().to_bytes(),
        native.to_bytes(),
        "読み直して書くとバイト一致"
    );
    // 歯抜けの番号・種類・色空間・既定の値がそのまま戻る
    let kinds: Vec<_> = restored
        .channels()
        .into_iter()
        .filter(|c| !c.is_standard())
        .map(|c| (c.index(), restored.channel_info(c).unwrap().clone()))
        .collect();
    assert_eq!(kinds.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [6, 8, 9]);
    assert_eq!(kinds[2].1.kind, ChannelKind::Normal);
    assert_eq!(kinds[1].1.default, Rgba8::new(10, 20, 30, 255));
    assert_eq!(kinds[0].1.color_space, ColorSpace::Linear);
    // ユーザーチャンネルを使うレイヤーの中身（面・有効・塗りつぶしの値・調整の対象・チャンネルごとの合成・マスク）
    let by_name = |n: &str| restored.layers().iter().find(|l| l.name() == n).unwrap();
    let ao = Channel::from_index(6).unwrap();
    let tint = Channel::from_index(8).unwrap();
    let detail = Channel::from_index(9).unwrap();
    let raster = by_name("ユーザーの面");
    assert!(raster.surface(detail).is_some() && !raster.is_channel_enabled(ao));
    assert_eq!(
        raster.channel_blend(tint),
        ChannelBlend::new(Some(BlendMode::Multiply), Some(0.5))
    );
    assert!(raster.mask().is_some());
    let fill = by_name("ユーザーの塗り");
    assert_eq!(fill.fill_value(tint), Some(Rgba8::new(10, 220, 90, 128)));
    assert!(!fill.is_channel_enabled(ao));
    assert_eq!(by_name("色相").enabled_channels(), [Channel::Color, tint]);
    assert_eq!(by_name("レベル").enabled_channels(), [ao, detail]);
    assert_eq!(
        by_name("ユーザーのグループ").channel_blend(detail),
        ChannelBlend::new(None, Some(0.25))
    );
}

#[test]
fn only_documents_with_user_channels_leave_version_21() {
    let plain = without_user_channels();
    let native = NativeDocument::from_core(&plain).unwrap();
    assert_eq!(
        native.version(),
        21,
        "ユーザーチャンネルが無ければ Unity 0.2.0 が読める版"
    );
    // 全部消せば版 21 へ戻る（版 22 の 0 個の一覧は書かない）
    let mut doc = user_channel_document();
    for c in doc.channels().into_iter().filter(|c| !c.is_standard()) {
        doc.remove_channel(c).unwrap();
    }
    let back = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(back.version(), 21);
    assert!(back.to_core().unwrap().channels().len() == 6);
    // 取り消すと戻り、また版 22
    assert!(doc.undo().unwrap());
    assert_eq!(NativeDocument::from_core(&doc).unwrap().version(), 22);
}

#[test]
fn user_channel_limits_are_written_and_read() {
    let mut doc = Document::with_tile_size(8, 8, 8).unwrap();
    for i in 0..58 {
        doc.add_channel(info(
            &format!("チャンネル {i}"),
            if i % 3 == 0 {
                ChannelKind::Color
            } else {
                ChannelKind::Scalar
            },
            ColorSpace::Linear,
            [i as u8, 0, 0, 255],
        ))
        .unwrap();
    }
    assert_eq!(doc.channels().len(), 64);
    assert!(doc
        .add_channel(info(
            "あふれ",
            ChannelKind::Scalar,
            ColorSpace::Linear,
            [0; 4]
        ))
        .is_err());
    let layer = doc.add_layer("最後のチャンネル").unwrap();
    let last = Channel::from_index(63).unwrap();
    doc.set_channel_pixel(layer, last, 1, 1, Rgba8::new(5, 6, 7, 255))
        .unwrap();
    doc.set_channel_blend(
        layer,
        last,
        ChannelBlend::new(Some(BlendMode::Screen), None),
        false,
    )
    .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), 22);
    let back = native.to_core().unwrap();
    assert_eq!(back.channels().len(), 64);
    assert_eq!(
        back.layers()[0].pixel(last, 1, 1).unwrap(),
        Rgba8::new(5, 6, 7, 255)
    );
    assert_eq!(
        NativeDocument::from_core(&back).unwrap().to_bytes(),
        native.to_bytes()
    );
    // レイヤーごとのチャンネルの印（byte 1 つ）は標準を含む 64 まで数えられ、チャンネルごとの合成は 64 個でも byte に収まる
    let mut many = back;
    let id = many.layers()[0].id();
    for c in many.channels() {
        many.set_channel_blend(
            id,
            c,
            ChannelBlend::new(Some(BlendMode::Multiply), Some(0.5)),
            false,
        )
        .unwrap();
    }
    let packed = NativeDocument::from_core(&many).unwrap();
    assert_eq!(
        packed.to_core().unwrap().layers()[0]
            .channel_blends()
            .count(),
        64
    );
}

/// 版 21 の正本（1 画素のもの）の版を 22 にし、標準の設定の後ろへユーザーチャンネルの一覧を差す。
fn with_user_list(count: i32, entries: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = read("m2-tiny.utpaint");
    bytes[8..12].copy_from_slice(&22i32.to_le_bytes());
    // 並び: 識別子 8 + 版 4 + ID 16 + 幅・高さ・タイル 12 + Normal の設定 21
    let at = 8 + 4 + 16 + 12 + 21;
    let mut list = count.to_le_bytes().to_vec();
    for e in entries {
        list.extend(e);
    }
    bytes.splice(at..at, list);
    bytes
}
fn entry(channel: i32, name: &str, kind: i32, space: i32) -> Vec<u8> {
    let mut e = channel.to_le_bytes().to_vec();
    e.extend((name.len() as i32).to_le_bytes());
    e.extend(name.as_bytes());
    e.extend(kind.to_le_bytes());
    e.extend(space.to_le_bytes());
    e.extend([1, 2, 3, 4]);
    e
}

#[test]
fn version_22_reader_rules() {
    let ok = with_user_list(1, &[entry(6, "余り", 1, 1)]);
    let native = NativeDocument::read(&ok).unwrap();
    assert_eq!(native.version(), 22);
    let core = native.to_core().unwrap();
    assert_eq!(core.channels().len(), 7);
    assert_eq!(
        NativeDocument::from_core(&core).unwrap().to_bytes(),
        ok,
        "レイヤーの使っていないチャンネルも往復する"
    );
    let full: Vec<_> = (6..64).map(|c| entry(c, &format!("C{c}"), 0, 0)).collect();
    assert!(NativeDocument::read(&with_user_list(58, &full)).is_ok());
    let refused: Vec<(&str, Vec<u8>, &str)> = vec![
        (
            "0 個の一覧は書かない",
            with_user_list(0, &[]),
            "user_channel_count",
        ),
        (
            "59 個",
            with_user_list(59, &[full.clone(), vec![entry(64, "C64", 0, 0)]].concat()),
            "user_channel_count",
        ),
        ("負の数", with_user_list(-1, &[]), "user_channel_count"),
        (
            "標準の番号 5",
            with_user_list(1, &[entry(5, "余り", 0, 0)]),
            "channel",
        ),
        (
            "番号 64",
            with_user_list(1, &[entry(64, "余り", 0, 0)]),
            "channel",
        ),
        (
            "負の番号",
            with_user_list(1, &[entry(-6, "余り", 0, 0)]),
            "channel",
        ),
        (
            "番号の重複",
            with_user_list(2, &[entry(7, "A", 0, 0), entry(7, "B", 0, 0)]),
            "番号の並び",
        ),
        (
            "番号が降順",
            with_user_list(2, &[entry(8, "A", 0, 0), entry(7, "B", 0, 0)]),
            "番号の並び",
        ),
        (
            "名前の重複",
            with_user_list(2, &[entry(6, "A", 0, 0), entry(7, "A", 0, 0)]),
            "名前が重複",
        ),
        (
            "標準の名前",
            with_user_list(1, &[entry(6, "Height", 1, 1)]),
            "名前が重複",
        ),
        (
            "空の名前",
            with_user_list(1, &[entry(6, "", 1, 1)]),
            "名前が不正",
        ),
        (
            "129 文字の名前",
            with_user_list(1, &[entry(6, &"あ".repeat(129), 1, 1)]),
            "名前が不正",
        ),
        (
            "制御文字の名前",
            with_user_list(1, &[entry(6, "A\nB", 1, 1)]),
            "名前が不正",
        ),
        (
            "未知の種類",
            with_user_list(1, &[entry(6, "A", 3, 0)]),
            "kind",
        ),
        (
            "負の種類",
            with_user_list(1, &[entry(6, "A", -1, 0)]),
            "kind",
        ),
        (
            "未知の色空間",
            with_user_list(1, &[entry(6, "A", 0, 2)]),
            "color_space",
        ),
        (
            "数が実際より多い",
            with_user_list(2, &[entry(6, "A", 0, 0)]),
            "user_channels[1]",
        ),
        (
            "数が実際より少ない",
            with_user_list(1, &[entry(6, "A", 0, 0), entry(7, "B", 0, 0)]),
            "layers[0]",
        ),
    ];
    for (what, bytes, fragment) in &refused {
        let message = NativeDocument::read(bytes).expect_err(what).to_string();
        assert!(message.contains(fragment), "{what}: {message}");
    }
    // 版 21 以前の並びに一覧は無い（版を 22 にしないで一覧を差した正本は、レイヤーの数として読まれて断られる）
    let mut stale = read("m2-tiny.utpaint");
    let at = 8 + 4 + 16 + 12 + 21;
    stale.splice(at..at, 1i32.to_le_bytes());
    assert!(NativeDocument::read(&stale).is_err());
    // 知らない新しい版は断る
    for version in [23i32, 24, 1000, 0, -1] {
        let mut bytes = read("m2-tiny.utpaint");
        bytes[8..12].copy_from_slice(&version.to_le_bytes());
        assert!(NativeDocument::read(&bytes).is_err(), "版 {version}");
    }
}

#[test]
fn user_channel_references_must_be_in_the_documents_list() {
    let bytes = NativeDocument::from_core(&user_channel_document())
        .unwrap()
        .to_bytes();
    let native = NativeDocument::read(&bytes).unwrap();
    let raster = index_of(&native, "ユーザーの面");
    let fill = index_of(&native, "ユーザーの塗り");
    let hue = index_of(&native, "色相");
    // 一覧にある番号は 6・8・9（7 は消してある）。レイヤーの持つチャンネル・塗りつぶしの値は、一覧に無い番号を指せない
    for path in [
        format!("layers[{raster}].channels[1].channel"),
        format!("layers[{fill}].fills[1].channel"),
        format!("layers[{hue}].adjustment.channels[1].channel"),
        format!("layers[{raster}].channel_blends[0].channel"),
    ] {
        for bad in [7, 10, 63, 64, -1] {
            let message = native
                .with_value(&path, NativeValue::Int(bad))
                .err()
                .unwrap_or_else(|| panic!("{path} = {bad}"))
                .to_string();
            assert!(
                message.contains("一覧にありません") || message.contains("範囲外"),
                "{path} = {bad}: {message}"
            );
        }
    }
    // 版 21 の正本に標準でないチャンネルの番号があれば、一覧が無いので断る（版 22 に一覧があれば通る）
    let tiny = NativeDocument::read(&read("m2-tiny.utpaint")).unwrap();
    assert!(tiny
        .with_value("layers[1].fills[5].channel", NativeValue::Int(6))
        .is_err());
    let listed = NativeDocument::read(&with_user_list(1, &[entry(6, "余り", 1, 1)])).unwrap();
    let moved = listed
        .with_value("layers[1].fills[5].channel", NativeValue::Int(6))
        .unwrap();
    let core = moved.to_core().unwrap();
    let extra = Channel::from_index(6).unwrap();
    assert_eq!(
        core.layers()[1].fill_value(extra),
        Some(Rgba8::new(6, 10, 250, 105))
    );
    assert_eq!(
        NativeDocument::from_core(&core).unwrap().to_bytes(),
        moved.to_bytes()
    );
    // 色相/彩度は色のチャンネルだけ（種類を変えて色でなくすと断る）
    let kind = |i: usize| format!("user_channels[{i}].kind");
    let message = native
        .with_value(&kind(1), NativeValue::Int(1))
        .expect_err("Tint を色でなくする")
        .to_string();
    assert!(message.contains("調整対象"), "{message}");
    assert!(
        native.with_value(&kind(0), NativeValue::Int(0)).is_ok(),
        "レベルの対象の AO は色にしても通る"
    );
    let hue_channel = format!("layers[{hue}].adjustment.channels[1].channel");
    assert!(
        native
            .with_value(&hue_channel, NativeValue::Int(6))
            .is_err(),
        "色相/彩度を Scalar の AO へ"
    );
    assert!(native.with_value(&hue_channel, NativeValue::Int(5)).is_ok());
}

/// 版 22 の正解の正本（Rust が書いたもの）。固定の ID で作り、ファイルと同じバイト列になる。
/// 作り直すときは `YOLU_UPDATE_FIXTURES=1 cargo test -p yolu-io --test ylp m2_bridge::`、続けて
/// `python3 tools/io-fixtures/generate.py --source <Unity 版> --user-channels` で Unity 版の読み手の記録を取り直す。
#[test]
fn version_22_fixture_is_what_this_writer_produces() {
    let bytes = NativeDocument::from_core(&user_channel_document())
        .unwrap()
        .to_bytes();
    let path = format!(
        "{}/tests/fixtures/user-channels-v22.utpaint",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var_os("YOLU_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &bytes).unwrap();
    }
    assert_eq!(bytes, std::fs::read(&path).unwrap());
}

#[test]
fn unity_0_2_0_reader_refuses_version_22_before_touching_the_file() {
    // Unity 0.2.0 の DocumentBinary・YlpFormat に Rust が書いた版 22 を読ませた記録（tools/io-fixtures --user-channels）
    let record = String::from_utf8(read("user-channels-v22.unity.txt")).unwrap();
    let lines: Vec<&str> = record.lines().collect();
    assert_eq!(lines[0], "DocumentBinary.CurrentVersion: 21");
    assert_eq!(lines[1], "YlpFormat.Current: 7");
    let refusal = "InvalidDataException: Unsupported archive version; source retained unchanged.";
    assert_eq!(lines[2], format!("DocumentBinary.Read: {refusal}"));
    assert_eq!(lines[3], format!("DocumentBinary.ReadId: {refusal}"));
    // 外側と project.json は読める（形式 7 のまま）。断るのは開く手順のセットの正本の読みで、全部のセットを読めてから入れ替えるので何も変わらない
    assert_eq!(lines[4], "YlpFormat.Open: OK");
    // 最後の行は、ウィンドウを動かした結果ではなく、ウィンドウが正本の読みの失敗に付ける文を同じ形に組み立てたもの（記録の行の名前が
    // そう言う）。Unity 0.2.0 が断った証拠として固定するのは、上の本物の呼び出し（3〜5 行目）だけ。この行は組み立ての形だけを確かめる
    assert!(
        lines[5].starts_with(
            "TexturePaintWindow.ReadTextureSets（文を組み立てたもの。ウィンドウは未実行）: InvalidDataException: Texture set \""
        ) && lines[5].ends_with("\": Unsupported archive version; source retained unchanged."),
        "{}",
        lines[5]
    );
    assert_eq!(lines.len(), 6);
    // 記録した正本は、この書き手が今書くものと同じ（記録が古くなっていない）
    assert_eq!(
        read("user-channels-v22.utpaint"),
        NativeDocument::from_core(&user_channel_document())
            .unwrap()
            .to_bytes()
    );
    // Rust の読み手は、同じ版の数の規則（今の最新より新しいものは断る）を持つ
    assert_eq!(yolu_io::UNITY_NATIVE_VERSION, 21);
    assert_eq!(yolu_io::USER_CHANNELS_VERSION, 22);
    // 版 23（Rust 版だけの Generator の種類）は procedural_bridge、版 24（色調補正の 6 種）は adjust_bridge の試験で固定する
    assert_eq!(yolu_io::EFFECTS_VERSION, 28);
    assert_eq!(yolu_io::MAX_NATIVE_VERSION, yolu_io::RULERS_VERSION);
}

/// Rust が書いた版 21 の正解の正本。m2-groups を編集して、塗りつぶし・調整・マスク・複製したグループを足したもの（固定の ID で作り、
/// ファイルと同じバイト列になる）。作り直すときは `YOLU_UPDATE_FIXTURES=1 cargo test -p yolu-io --test ylp m2_bridge::version_21`、続けて
/// `python3 tools/io-fixtures/generate.py --source <Unity 版> --rust-written` で Unity 版の読み手の記録を取り直す。
#[test]
fn version_21_fixture_is_what_this_writer_produces() {
    let bytes = NativeDocument::from_core(&edited_groups_document())
        .unwrap()
        .to_bytes();
    let path = format!(
        "{}/tests/fixtures/rust-written-v21.utpaint",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var_os("YOLU_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &bytes).unwrap();
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(bytes[8..12], 21i32.to_le_bytes());
}

/// 上の版 21 を Unity 0.2.0 の読み手（`DocumentBinary`、`Runtime/Core` をそのままコンパイル）に読ませた記録。読めて、書き直すと同じ
/// バイト列になる（画素の合成は Unity 版と揃えない。合成の式は f32 の core の式が正本）。
#[test]
fn unity_0_2_0_reads_and_resaves_the_version_21_this_writer_produces() {
    let edited = edited_groups_document();
    assert_eq!(
        read("rust-written-v21.utpaint"),
        NativeDocument::from_core(&edited).unwrap().to_bytes(),
        "記録した正本が今の書き手の出力と同じ（古くなっていない）"
    );
    assert!(edited
        .layers()
        .iter()
        .any(|l| l.kind() == LayerKind::Fill && l.name() == "新しい塗り"));
    let record = String::from_utf8(read("rust-written-v21.unity.txt")).unwrap();
    assert_eq!(
        record.lines().collect::<Vec<_>>(),
        [
            "DocumentBinary.CurrentVersion: 21".to_string(),
            "DocumentBinary.ReadId: OK".to_string(),
            "DocumentBinary.Read: OK".to_string(),
            "DocumentBinary.Write(Read) == input: True".to_string(),
            format!("Layers: {}", edited.layers().len()),
        ]
    );
}

fn writer() -> WriterInfo {
    WriterInfo {
        app: "試験の書き手".into(),
        version: "0.0.1".into(),
        unity: "standalone".into(),
    }
}

#[test]
fn a_project_keeps_user_channel_sets_in_format_7_and_other_sets_readable_by_unity() {
    const A: &str = "0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0";
    const B: &str = "11111111-2222-4333-8444-555555555555";
    let user = user_channel_document();
    let plain = without_user_channels();
    let spec = |id: &str, name: &str, doc: &Document| SetSpec {
        id: id.into(),
        name: name.into(),
        material: yolu_io::MaterialRef::Material {
            name: name.into(),
            asset: None,
        },
        document: Some(NativeDocument::from_core(doc).unwrap().into()),
        composites: yolu_io::composite_pngs(doc).unwrap(),
    };
    let project = Project::create(
        writer(),
        &[
            spec(A, "ユーザーチャンネル", &user),
            spec(B, "標準だけ", &plain),
        ],
        A,
    )
    .unwrap();
    assert_eq!(
        project.info().format,
        7,
        "ユーザーチャンネルがあっても .ylp の形式は 7"
    );
    let reopened = Project::read(&project.to_bytes().unwrap()).unwrap();
    let versions: Vec<i32> = reopened
        .sets()
        .iter()
        .map(|s| s.document.version())
        .collect();
    assert_eq!(
        versions,
        [22, 21],
        "版 22 になるのはユーザーチャンネルのあるセットだけ"
    );
    for set in reopened.sets() {
        assert!(set.document.core_issues().is_empty());
    }
    assert!(reopened
        .notes()
        .iter()
        .all(|n| !matches!(n, yolu_io::Note::SetNotConvertible { .. })));
    assert_same_document(
        &user,
        &reopened.sets()[0].document.to_core().unwrap(),
        "保存して開き直す",
    );
    // 標準だけのセットは、版 21 の正本として Unity 0.2.0 の読み手がそのまま読める並び（C# の書き手の版 21 と同じ構造）
    assert_eq!(
        reopened.sets()[1].document.to_bytes().unwrap()[8..12],
        21i32.to_le_bytes()
    );
}

// ───────── 断る ─────────

/// C# の書き手が作った見本（全機能入り。手動の ID の色つき）は、core に断られずに読め、手動の ID の色が文書へ戻り、書き戻すと元のバイト列になる。
#[test]
fn the_rich_native_document_with_manual_id_colors_reads_into_core_and_writes_back_the_same_bytes() {
    let rich = NativeDocument::read(include_bytes!("../fixtures/native-rich-v21.utpaint")).unwrap();
    // 効果（フィルター・Generator・Anchor・塗りつぶしの画像・投影・グラデーション）・パス・レイヤーのロック・手動の ID の色は core にあるので断らない
    let issues = rich.core_issues();
    assert!(issues.is_empty(), "{issues:?}");
    let core = rich.to_core().unwrap();
    // 手動の ID の色は、書いてあった指紋・番号・色のまま戻り、読み込みは取り消しの履歴に残らない
    let colors = core.id_colors();
    let count = match rich.field("manual_id_colors.count").unwrap() {
        NativeValue::Int(n) => *n as usize,
        other => panic!("{other:?}"),
    };
    assert!(count > 0 && colors.colors().len() == count, "{colors:?}");
    assert!(matches!(
        rich.field("manual_id_colors.binding"),
        Some(NativeValue::Text(t)) if t == colors.binding()
    ));
    for (i, (part, rgb)) in colors.colors().iter().enumerate() {
        let at = |leaf: &str| rich.field(&format!("manual_id_colors.colors[{i}].{leaf}"));
        assert_eq!(at("part"), Some(&NativeValue::Int(*part as i32)));
        assert_eq!(at("rgb"), Some(&NativeValue::Int(*rgb as i32)));
    }
    assert!(!core.can_undo() && !core.can_redo());
    // 書き戻すと元のバイト列（C# の書き手と同じ並び）
    assert_eq!(
        NativeDocument::from_core(&core).unwrap().to_bytes(),
        rich.to_bytes()
    );
    // 読んだだけの正本も、そのままのバイト列で書ける
    assert_eq!(
        NativeDocument::read(&rich.to_bytes()).unwrap().to_bytes(),
        rich.to_bytes()
    );
}

#[test]
fn adjustment_values_the_kind_does_not_use_must_be_default_or_the_document_is_refused() {
    // C# の読み手は種類ごとの作り方で使わない値を黙って既定に戻す。core へそのまま渡すと保存で値が変わるので断る
    let native = native("m2-channels");
    let invert = index_of(&native, "反転（チャンネルなし）");
    let levels = index_of(&native, "レベル");
    let hue = index_of(&native, "色相");
    for (layer, param, value, refused) in [
        (invert, "gamma", 2.0, true),
        (invert, "hue", 30.0, true),
        (levels, "hue", 30.0, true),
        (levels, "lightness", 0.5, true),
        (hue, "gamma", 0.5, true),
        (hue, "output_white", 0.5, true),
        (levels, "gamma", 2.0, false),
        (hue, "hue", 12.0, false),
        (levels, "hue", 0.0, false),
    ] {
        let path = format!("layers[{layer}].adjustment.{param}");
        let changed = native.with_value(&path, NativeValue::Float(value)).unwrap();
        let issues = changed.core_issues();
        assert_eq!(
            issues.iter().any(|i| i.contains(&path)),
            refused,
            "{path} = {value}: {issues:?}"
        );
        assert_eq!(changed.to_core().is_err(), refused, "{path} = {value}");
        // どちらでも元のバイト列として書ける
        assert_eq!(
            NativeDocument::read(&changed.to_bytes())
                .unwrap()
                .to_bytes(),
            changed.to_bytes()
        );
    }
}

/// レイヤーごとの断り（調整の種類が使わない値が既定ではない）は、レイヤーを非表示にしても、マスクを無効にしても変わらない。表示・有効を見て省くようになると、
/// 描かれないレイヤーの値が黙って捨てられ、保存で変わる（`yolu-io/README.md` の「非表示のレイヤー・無効にしたマスクの中身でも断る」の裏付け）。
#[test]
fn adjustment_values_the_kind_does_not_use_are_refused_even_when_the_layer_is_hidden_or_its_mask_disabled(
) {
    for (fixture, name, param, value) in [
        ("m2-channels", "反転（チャンネルなし）", "gamma", 2.0),
        ("m2-channels", "色相", "gamma", 0.5),
        // 有効なマスクのある調整レイヤー（隠す・マスクを無効にするの両方を試す）
        ("m2-masks", "レベル・マスク", "hue", 30.0),
        ("m2-masks", "レベル・マスク", "lightness", 0.5),
    ] {
        let original = native(fixture);
        assert!(original.core_issues().is_empty(), "{fixture}");
        let layer = index_of(&original, name);
        let path = format!("layers[{layer}].adjustment.{param}");
        let refused = original
            .with_value(&path, NativeValue::Float(value))
            .unwrap();
        let issues = refused.core_issues();
        assert!(
            issues.iter().any(|i| i.contains(&path)),
            "{path}: {issues:?}"
        );
        // 全部のレイヤーを非表示に、全部のマスクを無効にする
        let mut changed = refused.clone();
        let (mut hidden, mut disabled) = (0, 0);
        for i in 0..refused.layer_count() {
            for (field, count) in [
                (format!("layers[{i}].visible"), &mut hidden),
                (format!("layers[{i}].mask.enabled"), &mut disabled),
            ] {
                if refused.field(&field) == Some(&NativeValue::Bool(true)) {
                    changed = changed
                        .with_value(&field, NativeValue::Bool(false))
                        .unwrap();
                    *count += 1;
                }
            }
        }
        // 断る値のあるレイヤーが実際に非表示（マスクがあれば無効）へ変わっている（変える対象が空で素通りしていない）
        assert!(hidden > 0, "{fixture}");
        assert_eq!(
            changed.field(&format!("layers[{layer}].visible")),
            Some(&NativeValue::Bool(false)),
            "{path}"
        );
        if refused.field(&format!("layers[{layer}].has_mask")) == Some(&NativeValue::Bool(true)) {
            assert!(disabled > 0, "{fixture}");
            assert_eq!(
                changed.field(&format!("layers[{layer}].mask.enabled")),
                Some(&NativeValue::Bool(false)),
                "{path}"
            );
        }
        // 断る項目は 1 つも変わらず、変換も同じ理由で断る
        assert_eq!(changed.core_issues(), issues, "{path}");
        let message = changed.to_core().err().unwrap().to_string();
        assert!(message.contains(&path), "{message}");
        // どちらでも元のバイト列として書ける
        assert_eq!(
            NativeDocument::read(&changed.to_bytes())
                .unwrap()
                .to_bytes(),
            changed.to_bytes()
        );
    }
}

#[test]
fn broken_layer_structure_is_refused_when_reading() {
    let native = native("m2-groups");
    let groups: Vec<usize> = (0..native.layer_count())
        .filter(|i| native.field(&format!("layers[{i}].kind")) == Some(&NativeValue::Int(3)))
        .collect();
    assert!(groups.len() >= 7);
    let rasters = index_of(&native, "下地");
    let child = index_of(&native, "中 1");
    let id_of = |i: usize| match native.field(&format!("layers[{i}].id")) {
        Some(NativeValue::Guid(g)) => *g,
        other => panic!("{other:?}"),
    };
    let parent_path = |i: usize| format!("layers[{i}].parent");
    // 存在しない親・ラスターレイヤーを親にする・自分を親にする・下のレイヤーを親にする
    let unknown = native.with_value(&parent_path(child), NativeValue::Guid([9; 16]));
    assert!(unknown.err().unwrap().to_string().contains("親グループ"));
    let raster_parent = native.with_value(&parent_path(child), NativeValue::Guid(id_of(rasters)));
    assert!(raster_parent.is_err());
    let itself = native.with_value(&parent_path(groups[0]), NativeValue::Guid(id_of(groups[0])));
    assert!(itself.is_err());
    let below = native.with_value(&parent_path(groups[1]), NativeValue::Guid(id_of(groups[0])));
    assert!(below.is_err());
    // グループの外から入るレイヤー（子が連続していない）
    let gap = native.with_value(
        &parent_path(rasters),
        NativeValue::Guid(id_of(groups[groups.len() - 1])),
    );
    assert!(gap.is_err());
    // 版 5 以前は親の欄が無い。グループの種類も版 6 から
    assert!(native.with_value("version", NativeValue::Int(5)).is_err());
    // レイヤーの数・種類・合成モードの範囲
    assert!(native
        .with_value(&format!("layers[{child}].kind"), NativeValue::Int(4))
        .is_err());
    assert!(native
        .with_value(&format!("layers[{child}].blend"), NativeValue::Int(27))
        .is_err());
    assert!(native
        .with_value(&format!("layers[{child}].opacity"), NativeValue::Float(1.5))
        .is_err());
    assert!(native
        .with_value(
            &format!("layers[{child}].opacity"),
            NativeValue::Float(f64::NAN)
        )
        .is_err());
    // 通過はグループだけ
    let pass_through = (0..27)
        .find(|m| yolu_core::BlendMode::from_index(*m as u8) == Some(BlendMode::PassThrough))
        .unwrap();
    assert!(native
        .with_value(
            &format!("layers[{child}].blend"),
            NativeValue::Int(pass_through)
        )
        .is_err());
}

#[test]
fn truncated_and_corrupted_documents_are_refused_without_panicking() {
    for name in [
        "m2-tiny.utpaint",
        "m2-clipping.utpaint",
        "user-channels-v22.utpaint",
    ] {
        let bytes = read(name);
        for end in 0..bytes.len() {
            assert!(
                NativeDocument::read(&bytes[..end]).is_err(),
                "{name} を {end} で切る"
            );
        }
        // 末尾の余り
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(NativeDocument::read(&longer).is_err(), "{name}: 末尾の余り");
        // 1 バイトずつ壊す: 読めても core にできても、書き戻しは panic せず、できたものは読み直せる
        let flips: &[u8] = if bytes.len() < 1000 {
            &[0x01, 0x80, 0xff]
        } else {
            &[0xff]
        };
        for at in 0..bytes.len() {
            for &flip in flips {
                let mut broken = bytes.clone();
                broken[at] ^= flip;
                let Ok(native) = NativeDocument::read(&broken) else {
                    continue;
                };
                assert_eq!(
                    native.to_bytes(),
                    broken,
                    "{name}@{at}: 読めたものは同じバイト列"
                );
                if let Ok(core) = native.to_core() {
                    let saved = NativeDocument::from_core(&core)
                        .unwrap_or_else(|e| panic!("{name}@{at}: coreにできたものは書ける: {e}"));
                    assert!(
                        NativeDocument::read(&saved.to_bytes()).is_ok(),
                        "{name}@{at}"
                    );
                }
            }
        }
    }
}

#[test]
fn from_core_refuses_what_the_native_format_cannot_hold() {
    let mut doc = Document::with_tile_size(8, 8, 8).unwrap();
    for i in 0..2049 {
        doc.add_group(&format!("レイヤー {i}"), None).unwrap();
    }
    // 上限は壊れたデータではなく予算・上限の超過として返す（画面が言い分ける）
    let error = NativeDocument::from_core(&doc).err().unwrap();
    assert!(matches!(error, yolu_io::Error::Budget(_)), "{error:?}");
    let message = error.to_string();
    assert!(message.contains("2048"), "{message}");
    // 名前は UTF-8 で 4096 バイトまで（C# の読み手の ReadString の上限）
    let mut doc = Document::with_tile_size(8, 8, 8).unwrap();
    let id = doc.add_layer("レイヤー").unwrap();
    doc.set_layer_name(id, &"あ".repeat(1366)).unwrap();
    let error = NativeDocument::from_core(&doc).err().unwrap();
    assert!(matches!(error, yolu_io::Error::Budget(_)), "{error:?}");
    let message = error.to_string();
    assert!(message.contains("4096"), "{message}");
    doc.set_layer_name(id, &"あ".repeat(1365)).unwrap();
    assert!(NativeDocument::from_core(&doc).is_ok());
}

// ───────── 編集・Undo・保存 ─────────

/// m2-groups を開いた core に、新しいレイヤーの種類（塗りつぶし・調整・マスク・複製したグループ・チャンネルごとの合成）まで 10 回の編集をする。
fn edit_groups(core: &mut Document, native: &NativeDocument) {
    let ids: Vec<LayerId> = core.layers().iter().map(|l| l.id()).collect();
    let inner = ids[index_of(native, "中 1")];
    let group = ids[index_of(native, "分離（乗算）")];
    let top = ids[index_of(native, "上")];
    core.add_layer_above("グループの中に足す", Some(inner))
        .unwrap();
    core.add_layer_mask(top).unwrap();
    core.set_layer_mask_inverted(top, true).unwrap();
    core.set_layer_blend_mode(group, BlendMode::Screen).unwrap();
    core.set_layer_opacity(group, 0.25, false).unwrap();
    core.set_channel_blend(
        top,
        Channel::Roughness,
        ChannelBlend::new(Some(BlendMode::Multiply), None),
        false,
    )
    .unwrap();
    core.add_fill_layer(
        "新しい塗り",
        &[(Channel::Emission, Rgba8::new(1, 2, 3, 255))],
        None,
    )
    .unwrap();
    core.add_adjustment_layer("新しい反転", AdjustmentSettings::invert(), None, None)
        .unwrap();
    core.duplicate_layer(group, Some("グループの写し")).unwrap();
    core.ungroup(ids[index_of(native, "入れ子（通過）")])
        .unwrap();
}

/// 上の編集をした m2-groups（ID は決まった値）。C# の書き手に無いレイヤーの並びを Rust が書いた版 21 の正解の元。
fn edited_groups_document() -> Document {
    let native = native("m2-groups");
    let mut core = native.to_core().unwrap();
    edit_groups(&mut core, &native);
    fixed(core)
}

#[test]
fn edits_to_a_loaded_m2_document_undo_back_to_the_original_bytes_and_save() {
    let original = read("m2-groups.utpaint");
    let native = NativeDocument::read(&original).unwrap();
    let mut core = native.to_core().unwrap();
    edit_groups(&mut core, &native);
    let edits = core.undo_count();
    assert_eq!(edits, 10);
    let edited = NativeDocument::from_core(&core).unwrap();
    assert_ne!(edited.to_bytes(), original);
    assert!(edited.core_issues().is_empty());
    assert_same_document(&core, &edited.to_core().unwrap(), "編集した文書の往復");
    // .ylp へ保存して開き直す
    let (project, _) = {
        let p = Project::create(
            writer(),
            &[SetSpec {
                id: "0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0".into(),
                name: "M2".into(),
                material: yolu_io::MaterialRef::Unassigned,
                document: Some(edited.clone().into()),
                composites: yolu_io::composite_pngs(&core).unwrap(),
            }],
            "0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0",
        )
        .unwrap();
        (Project::read(&p.to_bytes().unwrap()).unwrap(), ())
    };
    assert_same_document(
        &core,
        &project.sets()[0].document.to_core().unwrap(),
        "保存して開き直す",
    );
    // 全部 Undo して元のバイト列に戻る。やり直しでまた編集後のバイト列に戻る
    for _ in 0..edits {
        assert!(core.undo().unwrap());
    }
    assert!(!core.can_undo());
    assert_eq!(
        NativeDocument::from_core(&core).unwrap().to_bytes(),
        original
    );
    for _ in 0..edits {
        assert!(core.redo().unwrap());
    }
    assert_eq!(
        NativeDocument::from_core(&core).unwrap().to_bytes(),
        edited.to_bytes()
    );
}

/// RGBA8 の PNG を、core の合成と同じ並び（下の行から）の画素にする。
fn decode_png_bottom_up(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    assert_eq!((reader.info().width, reader.info().height), (width, height));
    let mut top_down = vec![0; width as usize * height as usize * 4];
    reader.next_frame(&mut top_down).unwrap();
    top_down
        .chunks(width as usize * 4)
        .rev()
        .flatten()
        .copied()
        .collect()
}

fn composite_entries(project: &Project, set: &str) -> Vec<String> {
    let prefix = format!("sets/{set}/composite/");
    project
        .migrated_entries()
        .keys()
        .filter_map(|n| n.strip_prefix(&prefix))
        .map(String::from)
        .collect()
}

/// Unity 版のインポーターは `composite/<チャンネル>.png` の並びからセットのチャンネルを出すので、保存で Color 以外の合成が
/// 消えると、Roughness などを描いたセットが Color だけに見える。使っているチャンネルごとに書く。
#[test]
fn a_saved_set_carries_a_composite_png_for_every_channel_in_use() {
    const A: &str = "0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0";
    let spec = |doc: &Document, composites| SetSpec {
        id: A.into(),
        name: "合成".into(),
        material: yolu_io::MaterialRef::Unassigned,
        document: Some(NativeDocument::from_core(doc).unwrap().into()),
        composites,
    };
    // C# が書いた全チャンネルの文書。PNG は core の合成そのもの（Normal は Unity 向けの出力）で、使っているチャンネルだけ
    let core = native("m2-channels").to_core().unwrap();
    let pngs = yolu_io::composite_pngs(&core).unwrap();
    let channels: Vec<Channel> = pngs.iter().map(|(c, _)| *c).collect();
    let used: Vec<Channel> = Channel::ALL
        .into_iter()
        .filter(|c| core.layers().iter().any(|l| l.is_channel_enabled(*c)))
        .collect();
    assert!(used.len() >= 5, "{used:?}");
    assert_eq!(channels, used);
    for (channel, png) in &pngs {
        let expected = if *channel == Channel::Normal {
            core.normal_output(u64::MAX).unwrap()
        } else {
            core.composite_channel(*channel, core.bounds()).unwrap()
        };
        assert_eq!(
            decode_png_bottom_up(png, core.width(), core.height()),
            expected,
            "{channel:?}"
        );
    }
    let project = Project::create(writer(), &[spec(&core, pngs)], A).unwrap();
    let expected_names: Vec<String> = used
        .iter()
        .map(|c| format!("{}.png", c.standard_name().unwrap()))
        .collect();
    let mut names = composite_entries(&project, A);
    names.sort();
    let mut sorted = expected_names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    // Height → Normal を作る設定なら、Normal のレイヤーが無くても Normal を書く。Color は使うレイヤーが無くても書く
    let mut derived = Document::with_tile_size(12, 9, 8).unwrap();
    let layer = derived.add_layer("高さ").unwrap();
    paint(&mut derived, layer, Channel::Height, 4);
    derived
        .set_channel_enabled(layer, Channel::Color, false)
        .unwrap();
    let flat = yolu_io::composite_pngs(&derived).unwrap();
    assert_eq!(
        flat.iter().map(|(c, _)| *c).collect::<Vec<_>>(),
        [Channel::Color, Channel::Height]
    );
    derived
        .set_normal_settings(NormalSettings::DEFAULT.with_derive(true), false)
        .unwrap();
    let derived_pngs = yolu_io::composite_pngs(&derived).unwrap();
    assert_eq!(
        derived_pngs.iter().map(|(c, _)| *c).collect::<Vec<_>>(),
        [Channel::Color, Channel::Height, Channel::Normal]
    );
    assert_eq!(
        decode_png_bottom_up(&derived_pngs[2].1, 12, 9),
        derived.normal_output(u64::MAX).unwrap()
    );
    // 標準のチャンネルを使わなくなった保存では、残さない（中身と合わない派生は消える）。ほかのセットに触らない
    let plain = without_user_channels();
    let plain_pngs = yolu_io::composite_pngs(&plain).unwrap();
    let saved = project
        .with_sets(writer(), &[spec(&plain, plain_pngs)], A)
        .unwrap();
    assert_eq!(composite_entries(&saved, A), ["Color.png"]);
    // 標準でないチャンネル・同じチャンネルの重複は、書く前に理由を添えて断る
    let user = Channel::from_index(6).unwrap();
    let color = yolu_io::composite_png(&plain).unwrap();
    let message = Project::create(writer(), &[spec(&plain, vec![(user, color.clone())])], A)
        .err()
        .unwrap()
        .to_string();
    assert!(message.contains("標準ではありません"), "{message}");
    let twice = vec![(Channel::Color, color.clone()), (Channel::Color, color)];
    let message = Project::create(writer(), &[spec(&plain, twice)], A)
        .err()
        .unwrap()
        .to_string();
    assert!(message.contains("重複"), "{message}");
}

#[test]
fn removing_a_user_channel_drops_its_content_and_undo_restores_it_byte_for_byte() {
    let mut doc = user_channel_document();
    let before = NativeDocument::from_core(&doc).unwrap().to_bytes();
    let detail = Channel::from_index(9).unwrap();
    doc.remove_channel(detail).unwrap();
    let after = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(after.version(), 22, "ほかのユーザーチャンネルが残る");
    assert_ne!(after.to_bytes(), before);
    assert!(after.core_issues().is_empty());
    let restored = after.to_core().unwrap();
    assert_eq!(restored.channels().len(), 8);
    assert!(restored
        .layers()
        .iter()
        .all(|l| l.surface(detail).is_none()));
    assert!(doc.undo().unwrap());
    assert_eq!(NativeDocument::from_core(&doc).unwrap().to_bytes(), before);
}

#[test]
fn an_active_stroke_blocks_saving_a_document_with_layers_of_every_kind() {
    let native = native("m2-masks");
    let mut core = native.to_core().unwrap();
    let id = core.layers()[0].id();
    let stroke = core
        .begin_stroke(id, &yolu_core::BrushSettings::default())
        .unwrap();
    assert!(NativeDocument::from_core(&core).is_err());
    core.cancel_stroke(stroke);
    assert_eq!(
        NativeDocument::from_core(&core).unwrap().to_bytes(),
        read("m2-masks.utpaint")
    );
}

/// 色のウィンドウのドラッグでまとめた手動の ID の色は 1 段の取り消しで、保存すると最後の色が書かれ、開き直すと戻る。まとめた 1 段を戻すと
/// 手動の色の無い文書に戻り、色の無い文書と同じバイト列で保存できる。やり直すと、また最後の色で書ける。
#[test]
fn a_dragged_manual_id_color_is_one_step_and_saves_its_last_color() {
    let mut doc = yolu_core::Document::new(16, 16).unwrap();
    doc.add_layer("a").unwrap();
    doc.clear_history().unwrap();
    let plain = NativeDocument::from_core(&doc).unwrap().to_bytes();
    for rgb in [0x102030u32, 0x405060, 0x708090] {
        let colors = yolu_core::mesh_maps::IdColorAssignments::new(
            "0".repeat(64),
            std::collections::BTreeMap::from([(0usize, rgb)]),
        )
        .unwrap();
        doc.set_id_colors(colors, true).unwrap();
    }
    doc.end_coalescing();
    assert_eq!(doc.undo_count(), 1);
    let last_color_round_trips = |doc: &Document| {
        let bytes = NativeDocument::from_core(doc).unwrap().to_bytes();
        let reopened = NativeDocument::read(&bytes).unwrap().to_core().unwrap();
        assert_eq!(reopened.id_colors().colors().get(&0), Some(&0x708090));
        assert_eq!(reopened.id_colors().colors().len(), 1);
        assert_same_document(doc, &reopened, "ドラッグした文書");
    };
    last_color_round_trips(&doc);
    doc.undo().unwrap();
    let bytes = NativeDocument::from_core(&doc).unwrap().to_bytes();
    assert_eq!(bytes, plain);
    let reopened = NativeDocument::read(&bytes).unwrap().to_core().unwrap();
    assert!(reopened.id_colors().colors().is_empty());
    assert_same_document(&doc, &reopened, "戻した文書");
    doc.redo().unwrap();
    last_color_round_trips(&doc);
}

/// レイヤーのロック（正本の版 12）は黙って落とさず書き、読み戻せる。個別の 4 種・重ね・親のグループだけに掛けた場合のどれも、
/// 書いて読んで同じ自分のロックと効くロックに戻る。ロックを外せば、ロックの無い文書と同じバイト列に戻る。
#[test]
fn layer_locks_are_written_and_read_back() {
    use yolu_core::LayerLocks;
    let mut doc = yolu_core::Document::new(16, 16).unwrap();
    let id = doc.add_layer("レイヤー").unwrap();
    let group = doc.add_group("g", None).unwrap();
    let unlocked = NativeDocument::from_core(&doc).unwrap().to_bytes();
    for lock in [
        LayerLocks::TRANSPARENCY,
        LayerLocks::PIXELS,
        LayerLocks::POSITION,
        LayerLocks::ALL,
        LayerLocks::TRANSPARENCY | LayerLocks::POSITION,
        LayerLocks::from_bits(15).unwrap(),
    ] {
        for target in [id, group] {
            doc.set_layer_locks(target, lock).unwrap();
            let native = NativeDocument::from_core(&doc).unwrap();
            assert!(native.core_issues().is_empty(), "{lock:?}");
            assert_ne!(native.to_bytes(), unlocked, "{lock:?}");
            let back = native.to_core().unwrap();
            for l in doc.layers() {
                let again = back.layer(l.id()).unwrap();
                assert_eq!(again.locks(), l.locks(), "{lock:?} {}", l.name());
                assert_eq!(
                    back.effective_locks(l.id()).unwrap(),
                    doc.effective_locks(l.id()).unwrap(),
                    "{lock:?} {}",
                    l.name()
                );
            }
            // 書き直しても同じバイト列
            assert_eq!(
                NativeDocument::from_core(&back).unwrap().to_bytes(),
                native.to_bytes(),
                "{lock:?}"
            );
            doc.set_layer_locks(target, LayerLocks::NONE).unwrap();
        }
    }
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().to_bytes(),
        unlocked
    );
}

// ───────── PSD への書き出し: 表せないものは断る、表せるロックは書く ─────────

use yolu_io::psd::{self, CompatibilityMode, Limits};

/// 下から「下」「中」「上」の 3 レイヤー（すべてラスターの Color で、画素を持つ）。
fn psd_source() -> Document {
    let mut doc = Document::with_tile_size(16, 12, 8).unwrap();
    for (i, name) in ["下", "中", "上"].into_iter().enumerate() {
        let id = doc.add_layer(name).unwrap();
        paint(&mut doc, id, Channel::Color, i as u32 + 1);
    }
    doc
}

fn id_of(doc: &Document, name: &str) -> LayerId {
    doc.layers()
        .iter()
        .find(|l| l.name() == name)
        .unwrap_or_else(|| panic!("レイヤー「{name}」がありません"))
        .id()
}

fn psd_bytes(doc: &Document) -> Vec<u8> {
    psd::write(&psd::Document::from_core(doc).unwrap(), &Limits::default()).unwrap()
}

/// PSD への写しの断りの文（書けてしまったら、PSD の全画素を Debug で吐かずに、そのことだけを言って落ちる）。
fn psd_refusal(doc: &Document, what: &str) -> String {
    match psd::Document::from_core(doc) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("{what}: 断るはずが書けてしまった"),
    }
}

/// 「lspf」の中身（4 バイト）を、出てきた順に。
fn lspf_values(bytes: &[u8]) -> Vec<u32> {
    bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"lspf")
        .map(|(at, _)| u32::from_be_bytes(bytes[at + 8..at + 12].try_into().unwrap()))
        .collect()
}

/// レイヤーの記録の印（opacity・clipping の次の 1 バイト。ビット 1 が非表示、ビット 0 が透明部分のロック）を、合成モードの印・不透明度・
/// クリッピングが一致する記録について、出てきた順（下から上）に。lspf を介さず、書き出したバイト列のレイヤーの記録そのものを見る。
fn layer_record_flags(bytes: &[u8], key: &[u8; 4], opacity: u8, clipping: u8) -> Vec<u8> {
    let mut head = b"8BIM".to_vec();
    head.extend(key);
    head.extend([opacity, clipping]);
    let n = head.len();
    bytes
        .windows(n + 2)
        .filter(|w| w.starts_with(&head) && w[n + 1] == 0)
        .map(|w| w[n])
        .collect()
}

/// 厳密な書き出し（`psd::Document::from_core`。C# の ExportRefusesWhatPsdCannotRepresentInsteadOfFlattening と対）は、PSD に表せない中身を、
/// 平らにも黙って落とすこともせず、機能ごとの理由で断る。断る理由はレイヤーの名前と機能を言い、どの位置のレイヤーでも、非表示でも変わらない。
/// 書くのは Color のチャンネルだけ（C# の PsdBridge.Export(document, channel) と同じ）なので、ほかのチャンネルの中身・有効の印・合成は Color の PSD を
/// 変えず、Color を無効にしたレイヤーは隠したレイヤーになる。マスク（無効・濃度）・グループ・塗りつぶし・調整・Color の合成は PSD の形があり、書く（往復は psd_m2.rs）。
#[test]
fn psd_export_refuses_what_psd_cannot_hold_by_feature_instead_of_dropping_it() {
    type Setup = fn(&mut Document, LayerId);
    let sets: [(&str, &str, Setup); 2] = [
        ("反転したマスク", "反転", |d, id| {
            d.add_layer_mask(id).unwrap();
            d.set_layer_mask_inverted(id, true).unwrap();
        }),
        ("非表示の反転したマスク", "反転", |d, id| {
            d.set_layer_visible(id, false).unwrap();
            d.add_layer_mask(id).unwrap();
            d.set_layer_mask_inverted(id, true).unwrap();
        }),
    ];
    // Color 以外のチャンネルの中身・有効の印は、Color の PSD には効かない（C# の PsdBridge.Export(document, channel) と同じく、書くのは
    // そのチャンネルだけ。ほかのチャンネルは、書き出しのウィンドウでチャンネルを選んで別の PSD に書く）。Color の PSD は何も足さない文書と同じバイト列
    let others: [(&str, Setup); 6] = [
        ("Metallic の画素", |d, id| {
            d.set_channel_pixel(id, Channel::Metallic, 3, 3, Rgba8::new(9, 9, 9, 255))
                .unwrap();
        }),
        ("Metallic を有効にしただけ", |d, id| {
            d.set_channel_enabled(id, Channel::Metallic, true).unwrap()
        }),
        ("無効にした Roughness の画素", |d, id| {
            d.set_channel_pixel(id, Channel::Roughness, 1, 1, Rgba8::new(9, 9, 9, 255))
                .unwrap();
            d.set_channel_enabled(id, Channel::Roughness, false)
                .unwrap();
        }),
        ("ユーザーチャンネルの画素", |d, id| {
            let user = d
                .add_channel(info(
                    "AO",
                    ChannelKind::Scalar,
                    ColorSpace::Linear,
                    [255, 255, 255, 255],
                ))
                .unwrap();
            assert!(!user.is_standard());
            d.set_channel_pixel(id, user, 3, 3, Rgba8::new(9, 9, 9, 255))
                .unwrap();
        }),
        (
            "ユーザーチャンネルを有効にしただけ",
            |d, id| {
                let user = d
                    .add_channel(info(
                        "Tint",
                        ChannelKind::Color,
                        ColorSpace::Srgb,
                        [10, 20, 30, 255],
                    ))
                    .unwrap();
                d.set_channel_enabled(id, user, true).unwrap();
            },
        ),
        (
            "無効にしたユーザーチャンネルの画素",
            |d, id| {
                let user = d
                    .add_channel(info(
                        "Detail",
                        ChannelKind::Normal,
                        ColorSpace::Linear,
                        [128, 128, 255, 255],
                    ))
                    .unwrap();
                d.set_channel_pixel(id, user, 1, 1, Rgba8::new(9, 9, 9, 255))
                    .unwrap();
                d.set_channel_enabled(id, user, false).unwrap();
            },
        ),
    ];
    let control = psd_source();
    psd::Document::from_core(&control).expect("何も足さなければ書ける");
    for (label, word, setup) in sets {
        for name in ["下", "中", "上"] {
            let mut doc = psd_source();
            let id = id_of(&doc, name);
            setup(&mut doc, id);
            let err = psd_refusal(&doc, &format!("{label}（{name}）"));
            assert!(
                err.contains(word) && err.contains(&format!("レイヤー「{name}」")),
                "{label}（{name}）: {err}"
            );
        }
    }
    for (label, setup) in others {
        for name in ["下", "中", "上"] {
            let mut doc = psd_source();
            let id = id_of(&doc, name);
            let before = psd_bytes(&doc);
            setup(&mut doc, id);
            assert!(
                psd_bytes(&doc) == before,
                "{label}（{name}）: Color の PSD が変わった"
            );
        }
    }
    // Color を無効にしたレイヤーは、Color の PSD では隠したレイヤー（C# と同じ。断らない）
    for name in ["下", "中", "上"] {
        let mut doc = psd_source();
        let id = id_of(&doc, name);
        doc.set_channel_enabled(id, Channel::Color, false).unwrap();
        let back = psd::Document::from_core(&doc).expect("Color を無効にしたレイヤーも書ける");
        let layer = back.layers.iter().find(|l| l.name == name).expect(name);
        assert!(!layer.visible, "Color を無効（{name}）");
    }
    // グループ・塗りつぶし・調整は PSD の形で書ける（断らない）。Color の合成と、マスクの有効・濃度も同じ
    type AddLayer = fn(&mut Document) -> LayerId;
    let kinds: [(&str, LayerKind, AddLayer); 3] = [
        ("グループ", LayerKind::Group, |d| {
            d.add_group("種類", None).unwrap()
        }),
        ("塗りつぶし", LayerKind::Fill, |d| {
            d.add_fill_layer("種類", &[(Channel::Color, Rgba8::new(1, 2, 3, 255))], None)
                .unwrap()
        }),
        ("調整", LayerKind::Adjustment, |d| {
            d.add_adjustment_layer("種類", AdjustmentSettings::invert(), None, None)
                .unwrap()
        }),
    ];
    for (word, kind, add) in kinds {
        let mut doc = psd_source();
        let id = add(&mut doc);
        assert_eq!(doc.layer(id).unwrap().kind(), kind);
        let projected = psd::Document::from_core(&doc).unwrap_or_else(|e| panic!("{word}: {e}"));
        assert_eq!(projected.layers[0].name, "種類", "{word}");
        // 非表示でも書ける
        doc.set_layer_visible(id, false).unwrap();
        assert!(!psd::Document::from_core(&doc).unwrap().layers[0].visible);
    }
    let mut doc = psd_source();
    let id = id_of(&doc, "中");
    doc.add_layer_mask(id).unwrap();
    doc.set_layer_mask_enabled(id, false).unwrap();
    doc.set_layer_mask_density(id, 0.5, false).unwrap();
    doc.set_channel_blend(
        id,
        Channel::Color,
        ChannelBlend::new(Some(BlendMode::Multiply), Some(0.5)),
        false,
    )
    .unwrap();
    let projected = psd::Document::from_core(&doc).unwrap();
    let mid = projected.layers.iter().find(|l| l.name == "中").unwrap();
    assert_eq!(
        (
            mid.blend_mode,
            mid.opacity,
            mid.mask.as_ref().map(|m| (m.enabled, m.density))
        ),
        (psd::BlendMode::Multiply, 128, Some((false, 128)))
    );
    // 断っても文書は変わらない（履歴も合成も）
    let mut doc = psd_source();
    let id = id_of(&doc, "中");
    doc.add_layer_mask(id).unwrap();
    doc.set_layer_mask_inverted(id, true).unwrap();
    let (undo, before) = (doc.undo_count(), composites(&doc));
    psd_refusal(&doc, "反転したマスク");
    assert_eq!((doc.undo_count(), composites(&doc)), (undo, before));
    // 外せば書ける（反転を Undo で戻した文書。マスクそのものは書ける）
    assert!(doc.undo().unwrap());
    psd::Document::from_core(&doc).unwrap();
}

/// 描いている最中のストロークは、PSD への写しも断る（確定・取消のあとは書ける）。
#[test]
fn psd_export_refuses_an_active_stroke_until_it_ends() {
    let mut doc = psd_source();
    let id = id_of(&doc, "上");
    let stroke = doc
        .begin_stroke(id, &yolu_core::BrushSettings::default())
        .unwrap();
    let err = psd_refusal(&doc, "ストローク中");
    assert!(err.contains("ストローク"), "{err}");
    doc.cancel_stroke(stroke);
    psd::Document::from_core(&doc).unwrap();
}

/// PSD が表せるロック（透明部分・画素・位置・すべて）は、断らずに lspf へ書き、取り込めば core のロックに戻る（C# の PsdLockTests。ビットは lspf と同じ: 0 透明部分・1 画素・
/// 2 位置・31 すべて）。すべては 0x80000000 だけで書き、その下の個別のビットは足さない（効くロックは同じ）。
#[test]
fn psd_export_writes_the_locks_a_psd_can_hold() {
    use yolu_core::LayerLocks;
    let all_four = LayerLocks::from_bits(15).unwrap();
    // （ロック, 投影の locks, lspf に書く値, レイヤーの記録の印のビット 0）。ビット 0 は透明部分のロックだけが立ち、すべてを重ねたときは立てない
    // （読み手が印を lspf に足すので、立てるとすべてに透明部分が増えて戻る）
    let cases = [
        (LayerLocks::TRANSPARENCY, 1u32, 1u32, 1u8),
        (LayerLocks::PIXELS, 2, 2, 0),
        (LayerLocks::POSITION, 4, 4, 0),
        (LayerLocks::ALL, 0x8000_0000, 0x8000_0000, 0),
        (LayerLocks::TRANSPARENCY | LayerLocks::PIXELS, 3, 3, 1),
        (
            LayerLocks::TRANSPARENCY | LayerLocks::PIXELS | LayerLocks::POSITION,
            7,
            7,
            1,
        ),
        // すべてと個別を重ねても、投影も書くのも 0x80000000 だけ
        (all_four, 0x8000_0000, 0x8000_0000, 0),
    ];
    let unlocked = psd_source();
    let plain = psd_bytes(&unlocked);
    assert!(
        lspf_values(&plain).is_empty(),
        "ロックの無い文書は lspf を書かない"
    );
    for (lock, in_memory, on_disk, record_bit) in cases {
        let mut doc = psd_source();
        doc.set_layer_locks(id_of(&doc, "中"), lock).unwrap();
        let projected = psd::Document::from_core(&doc).unwrap();
        // 上から「上」「中」「下」。ロックしたのは「中」だけ
        assert_eq!(
            projected.layers.iter().map(|l| l.locks).collect::<Vec<_>>(),
            vec![0, in_memory, 0],
            "{lock:?}"
        );
        // ロックは描画に関わらない（画素も、貼り合わせた画像も同じ）
        let base = psd::Document::from_core(&unlocked).unwrap();
        assert_eq!(projected.composite_rgba, base.composite_rgba, "{lock:?}");
        for (a, b) in projected.layers.iter().zip(&base.layers) {
            assert_eq!(a.pixels_rgba, b.pixels_rgba, "{lock:?}");
        }
        let bytes = psd::write(&projected, &Limits::default()).unwrap();
        assert_eq!(
            lspf_values(&bytes),
            vec![on_disk],
            "{lock:?}: ロックしたレイヤーにだけ書く"
        );
        // レイヤーの記録の印のビット 0 も、下から「下」「中」「上」の順に、ロックした「中」だけ
        assert_eq!(
            layer_record_flags(&bytes, b"norm", 255, 0),
            vec![0, record_bit, 0],
            "{lock:?}: レイヤーの記録の印のビット 0"
        );
        let read = psd::read(&bytes, &Limits::default()).unwrap();
        assert_eq!(read.mode(), CompatibilityMode::EditableRaster, "{lock:?}");
        assert!(
            read.diagnostics().is_empty(),
            "{lock:?}: {:?}",
            read.diagnostics()
        );
        let again = read.document().unwrap();
        assert_eq!(
            again.layers.iter().map(|l| l.locks).collect::<Vec<_>>(),
            vec![0, on_disk, 0],
            "{lock:?}"
        );
        // 取り込み側は、ロックを core のレイヤーへ入れる（効くロックは書き出す前と同じ。すべては個別の下のビットを足さずに書くので、
        // 自分のロックは「すべて」だけに畳まれるが、効くロックは変わらない）
        assert!(again.core_issues().is_empty(), "{lock:?}");
        let back = again.to_core().unwrap();
        assert!(
            !back.can_undo(),
            "{lock:?}: 読み込みは Undo の履歴に残さない"
        );
        let names = ["下", "中", "上"];
        for name in names {
            let id = id_of(&back, name);
            let before = if name == "中" {
                lock
            } else {
                LayerLocks::NONE
            };
            assert_eq!(
                back.effective_locks(id).unwrap(),
                doc.effective_locks(id_of(&doc, name)).unwrap(),
                "{lock:?} {name}"
            );
            assert_eq!(
                back.layer(id).unwrap().locks().contains(LayerLocks::ALL),
                before.contains(LayerLocks::ALL),
                "{lock:?} {name}"
            );
        }
        // すべてが立てば、書くのは 0x80000000 だけなので、戻る自分のロックもすべてだけ
        let expected = if lock.contains(LayerLocks::ALL) {
            LayerLocks::ALL
        } else {
            lock
        };
        assert_eq!(
            back.layer(id_of(&back, "中")).unwrap().locks(),
            expected,
            "{lock:?}"
        );
        // 取り込んだ文書を書き出し直すと、同じ lspf（C# の PsdLockTests と同じ往復）
        let again_bytes = psd_bytes(&back);
        assert_eq!(lspf_values(&again_bytes), vec![on_disk], "{lock:?}");
    }
}

/// ロックは非表示・クリッピング・不透明度・合成モードと重ねても、レイヤーの属性として一緒に書かれて読み戻る。透明部分のロックだけはレイヤーの印のビット 0 にも書く。
#[test]
fn psd_locks_travel_with_the_other_layer_attributes() {
    use yolu_core::LayerLocks;
    let mut doc = psd_source();
    let (mid, top) = (id_of(&doc, "中"), id_of(&doc, "上"));
    doc.set_layer_locks(mid, LayerLocks::TRANSPARENCY).unwrap();
    doc.set_layer_visible(mid, false).unwrap();
    doc.set_layer_clipping(mid, true).unwrap();
    doc.set_layer_opacity(mid, 0.5, false).unwrap();
    doc.set_layer_blend_mode(mid, BlendMode::Multiply).unwrap();
    doc.set_layer_locks(top, LayerLocks::ALL).unwrap();
    let bytes = psd_bytes(&doc);
    // PSD のレイヤーの記録は下から上の順（「中」、「上」の順に出る）
    assert_eq!(
        lspf_values(&bytes),
        vec![1, 0x8000_0000],
        "ロックした 2 レイヤーにだけ"
    );
    // レイヤーの記録の印は、「中」が非表示（ビット 1）と透明部分のロック（ビット 0）で 3。読み手は印を lspf に足して戻すので、書き手が
    // 印のビット 0 を書かなくなっても読み戻しは通る。そこを読み戻しに頼らず、バイト列の記録で見る。
    // 「下」「上」は通常・不透明・クリッピング無しで、「上」のすべては印のビット 0 を立てない
    assert_eq!(
        layer_record_flags(&bytes, b"mul ", 128, 1),
        vec![3],
        "「中」の記録の印"
    );
    assert_eq!(
        layer_record_flags(&bytes, b"norm", 255, 0),
        vec![0, 0],
        "「下」「上」の記録の印"
    );
    let read = psd::read(&bytes, &Limits::default()).unwrap();
    assert_eq!(read.mode(), CompatibilityMode::EditableRaster);
    assert!(read.diagnostics().is_empty(), "{:?}", read.diagnostics());
    let layers = &read.document().unwrap().layers;
    let by_name = |n: &str| layers.iter().find(|l| l.name == n).unwrap();
    assert_eq!(by_name("上").locks, 0x8000_0000);
    let m = by_name("中");
    assert_eq!(
        (m.locks, m.visible, m.clipping, m.opacity, m.blend_mode),
        (1, false, true, 128, psd::BlendMode::Multiply)
    );
    assert_eq!(by_name("下").locks, 0);
}

/// core の合成モードの番号と PSD 側の列挙は同じ並び（`from_core` は番号で対応づける。27 個の保存値は yolu-core の blend の試験が固定する）。
#[test]
fn psd_blend_modes_line_up_with_the_cores_stored_values() {
    for i in 0..27u8 {
        let core = BlendMode::from_index(i).unwrap();
        assert_eq!(
            format!("{:?}", psd::BlendMode::ALL[i as usize]),
            core.name(),
            "番号 {i}"
        );
    }
    assert_eq!(psd::BlendMode::ALL.len(), 27);
}

/// 通過は PSD でもグループだけ。ほかのモードに言い換えて書かず、書き手が断る（C# の「どのレイヤーでも、PSD に対応が無いモードは別のモードで書かない」）。
/// core も、グループでないレイヤーに通過を付けるのを断る（だから `from_core` はこの断りに届かない）。
#[test]
fn psd_never_writes_pass_through_as_another_mode_for_a_layer_that_is_not_a_group() {
    let mut core = psd_source();
    let id = id_of(&core, "上");
    assert!(core
        .set_layer_blend_mode(id, BlendMode::PassThrough)
        .is_err());
    assert!(core
        .set_channel_blend_mode(id, Channel::Color, Some(BlendMode::PassThrough))
        .is_err());
    for kind in [
        psd::LayerKind::Raster,
        psd::LayerKind::Adjustment(psd::Adjustment::Invert),
        psd::LayerKind::SolidColor([1, 2, 3]),
    ] {
        let mut doc = psd::Document::from_core(&psd_source()).unwrap();
        doc.layers[0].kind = kind.clone();
        doc.layers[0].blend_mode = psd::BlendMode::PassThrough;
        let err = psd::write(&doc, &Limits::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("通過"), "{kind:?}: {err}");
    }
    // グループなら書ける
    let mut doc = psd::Document::from_core(&psd_source()).unwrap();
    doc.layers[0].kind = psd::LayerKind::Group {
        children: vec![],
        divider_id: 0,
    };
    doc.layers[0].blend_mode = psd::BlendMode::PassThrough;
    doc.layers[0].width = 0;
    doc.layers[0].height = 0;
    doc.layers[0].pixels_rgba = vec![];
    psd::write(&doc, &Limits::default()).unwrap();
}
