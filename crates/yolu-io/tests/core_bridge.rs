use yolu_core::{BrushSettings, Channel, Document, LayerId, Rgba8, TileCoord};
use yolu_io::{composite_png, NativeDocument, NativeValue, Project};
fn fixture(mode: usize) -> Project {
    Project::read(
        &std::fs::read(format!(
            "{}/tests/fixtures/m1-mode-{mode:02}.ylp",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap()
}
fn current(p: &Project) -> &NativeDocument {
    &p.sets()
        .iter()
        .find(|s| s.id == p.current_set())
        .unwrap()
        .document
}
#[test]
fn csharp_m1_all_modes_composite_png_and_native_roundtrip() {
    for mode in 0..26 {
        let p = fixture(mode);
        let native = current(&p);
        assert!(
            native.core_issues().is_empty(),
            "{:?}",
            native.core_issues()
        );
        let core = native.to_core().unwrap();
        assert_eq!(core.undo_count(), 0);
        let back = NativeDocument::from_core(&core).unwrap();
        assert_eq!(
            native.to_bytes(),
            back.to_bytes(),
            "正本の往復: mode {mode}"
        );
        let png = composite_png(&core).unwrap();
        let expected = std::fs::read(format!(
            "{}/tests/fixtures/m1-mode-{mode:02}.png",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert_eq!(png, expected, "PNG全バイト: mode {mode}");
    }
}
#[test]
fn edit_core_save_and_reopen_keeps_identity_pixels_and_properties() {
    let p = fixture(0);
    let native = current(&p);
    let mut core = native.to_core().unwrap();
    let id = core.layers()[1].id();
    core.set_pixel(id, 16, 10, Rgba8::new(17, 51, 93, 0))
        .unwrap();
    core.set_layer_name(id, "変更した層").unwrap();
    core.set_layer_opacity(id, 0.123456789, false).unwrap();
    core.set_layer_visible(id, false).unwrap();
    core.set_layer_clipping(id, true).unwrap();
    let updated = NativeDocument::from_core(&core).unwrap();
    assert_eq!(updated.id(), native.id());
    assert_eq!(updated.field("layers[1].id"), native.field("layers[1].id"));
    let saved = p.with_document(p.current_set(), &updated).unwrap();
    let reopened = Project::read(&saved.to_bytes().unwrap()).unwrap();
    let restored = current(&reopened).to_core().unwrap();
    let layer = restored.layer(id).unwrap();
    assert_eq!(layer.name(), "変更した層");
    assert_eq!(layer.opacity(), 0.123456789);
    assert!(!layer.visible());
    assert!(layer.clipping());
    assert_eq!(
        layer.pixel(Channel::Color, 16, 10).unwrap(),
        Rgba8::new(17, 51, 93, 0)
    );
    for set in p.sets().iter().filter(|s| s.id != p.current_set()) {
        assert_eq!(
            set.document.to_bytes(),
            reopened
                .sets()
                .iter()
                .find(|s| s.id == set.id)
                .unwrap()
                .document
                .to_bytes()
        );
    }
}
#[test]
fn unsupported_fields_are_reported_and_original_stays_writable() {
    let rich = NativeDocument::read(include_bytes!("fixtures/native-rich-v21.utpaint")).unwrap();
    let issues = rich.core_issues().join("\n");
    for what in [
        "normal",
        "attributes",
        "mask",
        "channels[1]",
        "filters",
        "surface_path",
        "canvas_path",
        "manual_id_colors",
        "projection",
        "gradients",
        "kind",
    ] {
        assert!(issues.contains(what), "{what}");
    }
    assert!(rich.to_core().is_err());
    assert_eq!(
        rich.to_bytes(),
        include_bytes!("fixtures/native-rich-v21.utpaint")
    );
    let p = fixture(0);
    let native = current(&p);
    for (path, value) in [
        ("normal.strength", NativeValue::Float(5.0)),
        ("layers[0].channels[0].enabled", NativeValue::Bool(false)),
        ("layers[0].channels[0].channel", NativeValue::Int(1)),
    ] {
        let changed = native.with_value(path, value).unwrap();
        assert!(changed.core_issues().iter().any(|s| s.contains(path)));
        assert!(changed.to_core().is_err());
    }
}
#[test]
fn edge_padding_is_refused_without_losing_original_bytes() {
    let p = fixture(0);
    let native = current(&p);
    // 右端のタイルのキャンバス外にもRGBAがある場合、coreは保持できない。
    let path = "layers[0].channels[0].tiles[2].rgba";
    let Some(NativeValue::Bytes(bytes)) = native.field(path) else {
        panic!()
    };
    let mut bytes = bytes.to_vec();
    bytes[7 * 4] = 19;
    let changed = native
        .with_value(path, NativeValue::Bytes(bytes.into()))
        .unwrap();
    assert!(changed
        .to_core()
        .err()
        .unwrap()
        .to_string()
        .contains("tiles[2]"));
    assert_eq!(
        NativeDocument::read(&changed.to_bytes())
            .unwrap()
            .to_bytes(),
        changed.to_bytes()
    );
}
#[test]
fn from_core_refuses_extra_channels_size_tile_size_and_active_stroke() {
    let mut doc = Document::with_tile_size(8, 8, 8).unwrap();
    let id = doc.add_layer("層").unwrap();
    doc.import_tile(id, Channel::Height, TileCoord::new(0, 0), &[17; 256])
        .unwrap();
    assert!(NativeDocument::from_core(&doc)
        .unwrap_err()
        .to_string()
        .contains("Height"));
    assert!(NativeDocument::from_core(&Document::with_tile_size(8193, 8, 8).unwrap()).is_err());
    assert!(NativeDocument::from_core(&Document::with_tile_size(8, 8, 3).unwrap()).is_err());
    let mut doc = Document::with_tile_size(8, 8, 8).unwrap();
    let id = doc.add_layer("描画中").unwrap();
    let stroke = doc.begin_stroke(id, &BrushSettings::default()).unwrap();
    assert!(NativeDocument::from_core(&doc).is_err());
    doc.cancel_stroke(stroke);
    assert!(NativeDocument::from_core(&doc).is_ok());
}
#[test]
fn persistent_ids_validate_count_empty_duplicates_and_history() {
    for ids in [vec![], vec![LayerId(0)], vec![LayerId(1), LayerId(1)]] {
        let mut doc = Document::new(8, 8).unwrap();
        doc.add_layer("層").unwrap();
        if ids.len() == 2 {
            doc.add_layer("2").unwrap();
        }
        assert!(doc.with_persistent_ids(7, &ids).is_err());
    }
    assert!(Document::new(8, 8)
        .unwrap()
        .with_persistent_ids(0, &[])
        .is_err());
    let mut doc = Document::new(8, 8).unwrap();
    let id = doc.add_layer("層").unwrap();
    doc.set_layer_visible(id, false).unwrap();
    assert!(doc.can_undo());
    let mut restored = doc.with_persistent_ids(9, &[LayerId(17)]).unwrap();
    assert_eq!(restored.id(), 9);
    assert!(!restored.can_undo());
    restored.set_layer_name(LayerId(17), "復元後").unwrap();
    restored.undo().unwrap();
    assert_eq!(restored.layer(LayerId(17)).unwrap().name(), "層");
}
#[test]
fn empty_document_and_legacy_v1_are_convertible() {
    let doc = Document::with_tile_size(9, 10, 8).unwrap();
    let restored = NativeDocument::from_core(&doc).unwrap().to_core().unwrap();
    assert_eq!(restored.id(), doc.id());
    assert!(restored.layers().is_empty());
    let legacy = NativeDocument::read(include_bytes!("fixtures/native-v1.utpaint")).unwrap();
    let core = legacy.to_core().unwrap();
    assert_eq!(NativeDocument::from_core(&core).unwrap().id(), legacy.id());
    assert_eq!(core.layers()[0].name(), "日本語の層");
}

#[test]
fn csharp_compressible_pattern_matches_png_and_native_bytes() {
    let p = Project::read(include_bytes!("fixtures/m1-pattern.ylp")).unwrap();
    let native = current(&p);
    let core = native.to_core().unwrap();
    assert_eq!(
        NativeDocument::from_core(&core).unwrap().to_bytes(),
        native.to_bytes()
    );
    let png = composite_png(&core).unwrap();
    assert!(png.len() < 65 * 33);
    assert_eq!(png, include_bytes!("fixtures/m1-pattern.png"));
}
