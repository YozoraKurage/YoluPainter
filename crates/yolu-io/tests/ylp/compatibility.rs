use yolu_core::{
    AdjustmentSettings, BrushSettings, Channel, Document, HeightEdgeMode, LayerKind,
    NormalSettings, NormalYDirection, Rgba8,
};
use yolu_io::{Archive, NativeDocument, NativeValue, Project, WriterInfo};

use crate::legacy_layout;
use legacy_layout::{as_version, NORMAL_SETTINGS_OFFSET};
fn fixture(n: usize) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/format{n}.ylp",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn unity_formats_1_to_6_roundtrip_every_entry() {
    for n in 1..=6 {
        let bytes = fixture(n);
        let p = Project::read(&bytes).unwrap_or_else(|e| panic!("形式{n}: {e}"));
        assert_eq!(p.info().format, n as i32);
        let read = Archive::read(&bytes).unwrap();
        let rewritten = Archive::read(&p.to_bytes().unwrap()).unwrap();
        assert_eq!(read.entries(), rewritten.entries());
        assert_eq!(read.manifest(), rewritten.manifest());
        for s in p.sets() {
            let native = &p.migrated_entries()[&format!("sets/{}/document.utpaint", s.id)];
            assert_eq!(
                &native.bytes().unwrap()[..],
                &s.document.to_bytes().unwrap()[..]
            );
            if let Some(sel) = &s.selection {
                assert_eq!(
                    &p.migrated_entries()[&format!("sets/{}/selection.bin", s.id)]
                        .bytes()
                        .unwrap()[..],
                    sel.to_bytes()
                );
            }
        }
        let upgraded = p
            .upgraded(WriterInfo {
                app: "yolu-io".into(),
                version: "test".into(),
                unity: "none".into(),
            })
            .unwrap();
        assert_eq!(upgraded.info().format, 7);
        assert_eq!(upgraded.sets().len(), p.sets().len());
        for (a, b) in p.sets().iter().zip(upgraded.sets()) {
            assert_eq!(
                a.document.to_bytes().unwrap(),
                b.document.to_bytes().unwrap()
            );
        }
    }
}
#[test]
fn native_header_and_truncation_refused() {
    let p = Project::read(&fixture(1)).unwrap();
    let original = p.sets()[0].document.to_bytes().unwrap();
    for offset in [0, 8, 28, 32, 36] {
        let mut b = original.clone();
        b[offset..offset + 4].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(NativeDocument::read(&b).is_err());
    }
    for n in [0, 1, 8, 12, 28, 40, original.len() - 1] {
        assert!(NativeDocument::read(&original[..n]).is_err());
    }
    let mut b = original;
    b.push(0);
    assert!(NativeDocument::read(&b).is_err());
}
#[test]
fn native_edit_is_reserialized_and_validated() {
    let p = Project::read(&fixture(1)).unwrap();
    let d = &p.sets()[0].document;
    let edited = d
        .with_value(
            "layers[0].name",
            NativeValue::Text("日本語のレイヤー".into()),
        )
        .unwrap();
    assert_eq!(
        NativeDocument::read(&edited.to_bytes())
            .unwrap()
            .field("layers[0].name"),
        edited.field("layers[0].name")
    );
    assert!(d
        .with_value("layers[0].opacity", NativeValue::Float(f64::NAN))
        .is_err());
    // 未知の版は断る（22 は読める版。その意味は m2_bridge の版 22 の試験）
    assert!(d.with_value("version", NativeValue::Int(23)).is_err());
}
#[test]
fn csharp_native_versions_1_to_21_roundtrip() {
    for v in 1..=21 {
        let b = std::fs::read(format!(
            "{}/tests/fixtures/native-v{v}.utpaint",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let d = NativeDocument::read(&b).unwrap_or_else(|e| panic!("正本{v}: {e}"));
        assert_eq!(d.version(), v);
        assert_eq!(d.to_bytes(), b);
        assert_eq!(d.layer_count(), 1);
    }
}
#[test]
fn csharp_rich_v21_and_selection_roundtrip() {
    let b = include_bytes!("../fixtures/native-rich-v21.utpaint");
    let d = NativeDocument::read(b).unwrap();
    assert_eq!(d.to_bytes(), b);
    assert!(d.fields().iter().any(|f| f.path.contains(".ramp.colors[")));
    assert!(d.fields().iter().any(|f| f.path.contains(".surface_path.")));
    assert!(d.fields().iter().any(|f| f.path.contains(".canvas_path.")));
    assert!(d.fields().iter().any(|f| f.path.contains(".anchor.id")));
    let b = include_bytes!("../fixtures/selection-v1.bin");
    let s = yolu_io::Selection::read(b, &d).unwrap();
    assert_eq!(s.to_bytes(), b);
}
#[test]
fn unity_shared_materials_fixture_preserves_all_three_sets() {
    let b = include_bytes!("../fixtures/format5-shared-materials.ylp");
    let p = Project::read(b).unwrap();
    assert_eq!(p.sets().len(), 3);
    let a = Archive::read(&p.to_bytes().unwrap()).unwrap();
    assert_eq!(a.entries(), Archive::read(b).unwrap().entries());
}
#[test]
fn format6_material_kind_preserves_smart_material_file() {
    let a = Archive::read(&fixture(5)).unwrap();
    let mut f: std::collections::BTreeMap<_, _> = a
        .entries()
        .iter()
        .map(|(n, b)| (n.clone(), b.to_vec()))
        .collect();
    let mut r: serde_json::Value = serde_json::from_slice(&f["resources.json"]).unwrap();
    for resource in r["resources"].as_array_mut().unwrap() {
        if resource["kind"] == "smartMaterial" {
            resource["kind"] = "material".into();
        }
    }
    f.insert("resources.json".into(), serde_json::to_vec(&r).unwrap());
    let mut info: serde_json::Value = serde_json::from_slice(&f["ylp.json"]).unwrap();
    info["format"] = 6.into();
    f.insert("ylp.json".into(), serde_json::to_vec(&info).unwrap());
    let b = Archive::from_entries(f).unwrap().to_bytes().unwrap();
    let p = Project::read(&b).unwrap();
    assert!(p.resources().iter().any(|r| r.kind == "material"));
    assert_eq!(
        Archive::read(&p.to_bytes().unwrap()).unwrap().entries(),
        Archive::read(&b).unwrap().entries()
    );
}

// ───────── 編集した内容の保存復元と、旧版の並びでの読み（C# の NativeArchiveRoundTrips〜 と ReadsVersionN） ─────────

fn save(doc: &Document) -> Vec<u8> {
    NativeDocument::from_core(doc).unwrap().to_bytes()
}
fn restore(bytes: &[u8]) -> Document {
    NativeDocument::read(bytes).unwrap().to_core().unwrap()
}
fn colour(doc: &Document) -> Vec<u8> {
    doc.composite(doc.bounds()).unwrap()
}
fn channel_composites(doc: &Document) -> Vec<Vec<u8>> {
    Channel::ALL
        .into_iter()
        .map(|c| doc.composite_channel(c, doc.bounds()).unwrap())
        .collect()
}
/// 1 層（名前 `name`）に 1 画素だけ描いた 16×16・タイル 8 の文書。
fn one_pixel(name: &str, colour: Rgba8) -> Document {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let id = d.add_layer(name).unwrap();
    d.set_pixel(id, 1, 2, colour).unwrap();
    d.clear_history().unwrap();
    d
}

/// Normal の出力設定は保存して復元でき（履歴は残らず、書き戻しも同じバイト列）、設定を持たない版 6 の並びは既定の設定で読める。
#[test]
fn normal_settings_survive_save_and_restore_and_version_6_reads_with_defaults() {
    let mut d = one_pixel("p", Rgba8::new(9, 8, 7, 255));
    let plain = save(&d);
    assert_eq!(i32::from_le_bytes(plain[8..12].try_into().unwrap()), 21);
    let restored = restore(&as_version(&plain, "p", 6));
    assert_eq!(
        restored.normal_settings(),
        NormalSettings::DEFAULT,
        "版 6 に Normal の設定は無い"
    );
    assert_eq!(colour(&restored), colour(&d));
    assert_eq!(save(&restored), plain, "今の版の既定の設定で書き直される");
    let settings =
        NormalSettings::new(true, 12.5, HeightEdgeMode::Wrap, NormalYDirection::DirectX).unwrap();
    d.set_normal_settings(settings, false).unwrap();
    let bytes = save(&d);
    assert_ne!(bytes, plain);
    // 設定は文書の頭（タイルの大きさの直後）に、アルゴリズム版・派生・強さ・端・向きの順
    let at = NORMAL_SETTINGS_OFFSET;
    assert_eq!(bytes[at + 4], 1, "高さから作る");
    assert_eq!(
        f64::from_le_bytes(bytes[at + 5..at + 13].try_into().unwrap()),
        12.5
    );
    let restored = restore(&bytes);
    assert_eq!(restored.normal_settings(), settings);
    assert!(!restored.can_undo(), "読み込みは履歴を残さない");
    assert_eq!(save(&restored), bytes);
    // 設定の変更は Undo で戻る（保存はそのときの設定）
    assert!(d.undo().unwrap());
    assert_eq!(save(&d), plain);
}

/// 無効にしたチャンネルの値と、マスクの付いた塗りつぶし（マスクの画素を含む）は、保存して復元しても変わらない。
#[test]
fn fill_layers_with_a_disabled_channel_and_a_mask_survive_save_and_restore() {
    let green = Rgba8::new(10, 200, 30, 255);
    let grey = Rgba8::new(90, 90, 90, 255);
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let fill = d
        .add_fill_layer(
            "Fill",
            &[(Channel::Color, green), (Channel::Height, grey)],
            None,
        )
        .unwrap();
    d.set_channel_enabled(fill, Channel::Height, false).unwrap();
    d.add_layer_mask(fill).unwrap();
    let brush = BrushSettings {
        radius: 1.0,
        hardness: 1.0,
        pressure_size: false,
        pressure_opacity: false,
        ..BrushSettings::default()
    };
    let mut stroke = d.begin_mask_stroke(fill, &brush).unwrap();
    stroke.apply_pixel(&mut d, 3, 3, 1.0, 1.0).unwrap();
    d.end_stroke(stroke).unwrap();
    let paint = d.add_layer("Paint").unwrap();
    d.set_pixel(paint, 1, 1, Rgba8::new(1, 2, 3, 255)).unwrap();
    let bytes = save(&d);
    let restored = restore(&bytes);
    assert_eq!(save(&restored), bytes);
    let r = restored.layer(fill).unwrap();
    assert_eq!(r.kind(), LayerKind::Fill);
    assert_eq!(r.fill_value(Channel::Color), Some(green));
    assert!(!r.is_channel_enabled(Channel::Height));
    assert_eq!(
        r.fill_value(Channel::Height),
        Some(grey),
        "無効にしても値は保つ"
    );
    let mask = r.mask().expect("マスクも戻る");
    assert_eq!(mask.surface().pixel(3, 3).unwrap().a, 255);
    assert_eq!(mask.surface().tile_count(), 1);
    assert_eq!(channel_composites(&restored), channel_composites(&d));
    assert!(!restored.can_undo());
}

/// 調整（対象のチャンネルと、無効にしたチャンネル）は保存して復元しても変わらない。版 3 の並び（調整の種類が無い）は塗りつぶしの文書として読める。
#[test]
fn adjustments_survive_save_and_restore_and_version_3_reads() {
    let mut d = one_pixel("base", Rgba8::new(100, 100, 100, 255));
    let hsl = AdjustmentSettings::hue_saturation(-45.0, 0.3, -0.2).unwrap();
    let hsl_layer = d
        .add_adjustment_layer("HSL", hsl.clone(), None, None)
        .unwrap();
    d.set_channel_enabled(hsl_layer, Channel::Emission, false)
        .unwrap();
    let levels = AdjustmentSettings::levels(0.1, 0.9, 2.5, 0.2, 0.8).unwrap();
    let levels_layer = d
        .add_adjustment_layer("Levels", levels.clone(), Some(&[Channel::Roughness]), None)
        .unwrap();
    let bytes = save(&d);
    let restored = restore(&bytes);
    assert_eq!(save(&restored), bytes);
    assert_eq!(restored.layer(hsl_layer).unwrap().adjustment(), Some(&hsl));
    assert_eq!(
        restored.layer(levels_layer).unwrap().adjustment(),
        Some(&levels)
    );
    assert_eq!(
        restored.layer(hsl_layer).unwrap().enabled_channels(),
        vec![Channel::Color]
    );
    assert_eq!(
        restored.layer(levels_layer).unwrap().enabled_channels(),
        vec![Channel::Roughness]
    );
    assert_eq!(channel_composites(&restored), channel_composites(&d));
    // 版 3 の並びは、各層のクリッピングの 1 バイト（版 5）と調整のブロック（版 4）が無い。塗りつぶしだけの文書は読める
    let mut plain = Document::with_tile_size(16, 16, 8).unwrap();
    plain
        .add_fill_layer("F", &[(Channel::Color, Rgba8::new(9, 9, 9, 9))], None)
        .unwrap();
    plain.clear_history().unwrap();
    let v3 = restore(&as_version(&save(&plain), "F", 3));
    assert_eq!(colour(&v3), colour(&plain));
    assert_eq!(save(&v3), save(&plain), "今の版で書き直される");
}

/// 版 3〜20 の並びは、今の書き手の出力から作った 1 層の文書を読め、今の版へ書き直すと元と同じバイト列になる。
/// 版 10 から先は版の数だけが違い、版 9 以前は各版が足した有無の 1 バイトと、Normal の設定・親グループ・クリッピングが無い。
#[test]
fn a_plain_document_is_read_from_every_older_layout_and_rewritten_as_the_current_version() {
    let mut d = one_pixel("p", Rgba8::new(40, 50, 60, 200));
    let hidden = d.layers()[0].id();
    d.set_layer_opacity(hidden, 0.5, false).unwrap();
    let plain = save(&d);
    for version in 3..=20 {
        let older = as_version(&plain, "p", version);
        let native = NativeDocument::read(&older).unwrap_or_else(|e| panic!("版 {version}: {e}"));
        assert_eq!(native.version(), version);
        assert_eq!(
            native.to_bytes(),
            older,
            "版 {version}: 読んで書き戻しても同じ"
        );
        let restored = native.to_core().unwrap();
        assert_eq!(colour(&restored), colour(&d), "版 {version}");
        assert_eq!(restored.layers()[0].opacity(), 0.5, "版 {version}");
        assert_eq!(
            save(&restored),
            plain,
            "版 {version}: 今の版へ書き直すと同じ"
        );
    }
}
