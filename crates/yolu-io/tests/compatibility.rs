use yolu_io::{Archive, NativeDocument, NativeValue, Project, WriterInfo};
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
            assert_eq!(native.as_ref(), s.document.to_bytes());
            if let Some(sel) = &s.selection {
                assert_eq!(
                    p.migrated_entries()[&format!("sets/{}/selection.bin", s.id)].as_ref(),
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
            assert_eq!(a.document.to_bytes(), b.document.to_bytes());
        }
    }
}
#[test]
fn native_header_and_truncation_refused() {
    let p = Project::read(&fixture(1)).unwrap();
    let original = p.sets()[0].document.to_bytes();
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
    assert!(d.with_value("version", NativeValue::Int(22)).is_err());
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
    let b = include_bytes!("fixtures/native-rich-v21.utpaint");
    let d = NativeDocument::read(b).unwrap();
    assert_eq!(d.to_bytes(), b);
    assert!(d.fields().iter().any(|f| f.path.contains(".ramp.colors[")));
    assert!(d.fields().iter().any(|f| f.path.contains(".surface_path.")));
    assert!(d.fields().iter().any(|f| f.path.contains(".canvas_path.")));
    assert!(d.fields().iter().any(|f| f.path.contains(".anchor.id")));
    let b = include_bytes!("fixtures/selection-v1.bin");
    let s = yolu_io::Selection::read(b, &d).unwrap();
    assert_eq!(s.to_bytes(), b);
}
#[test]
fn unity_shared_materials_fixture_preserves_all_three_sets() {
    let b = include_bytes!("fixtures/format5-shared-materials.ylp");
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
