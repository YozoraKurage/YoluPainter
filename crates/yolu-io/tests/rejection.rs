use std::collections::BTreeMap;
use yolu_io::{Archive, NativeDocument, NativeValue, Project, Selection, WriterInfo};
fn files(n: usize) -> BTreeMap<String, Vec<u8>> {
    Archive::read(
        &std::fs::read(format!(
            "{}/tests/fixtures/format{n}.ylp",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap()
    .entries()
    .iter()
    .map(|(n, b)| (n.clone(), b.to_vec()))
    .collect()
}
fn open(f: BTreeMap<String, Vec<u8>>) -> yolu_io::Result<Project> {
    Project::read(&Archive::from_entries(f)?.to_bytes()?)
}
fn mutate_json(
    f: &mut BTreeMap<String, Vec<u8>>,
    name: &str,
    change: impl FnOnce(&mut serde_json::Value),
) {
    let mut j = serde_json::from_slice(&f[name]).unwrap();
    change(&mut j);
    f.insert(name.into(), serde_json::to_vec(&j).unwrap());
}
#[test]
fn future_format_error_identifies_writer() {
    let mut f = files(6);
    mutate_json(&mut f, "ylp.json", |j| {
        j["format"] = 99.into();
        j["savedBy"]["app"] = "FuturePainter".into();
        j["savedBy"]["version"] = "9.0".into();
    });
    let e = open(f).unwrap_err().to_string();
    assert!(e.contains("99") && e.contains("FuturePainter") && e.contains("9.0"));
}
#[test]
fn invalid_json_and_duplicate_keys_are_refused() {
    for b in [
        b"{\"format\":2,\"format\":6}".to_vec(),
        b"{\xff}".to_vec(),
        vec![b' '; 65537],
        b"{\"format\":6.0}".to_vec(),
    ] {
        let mut f = files(6);
        f.insert("ylp.json".into(), b);
        assert!(open(f).is_err());
    }
}
#[test]
fn missing_document_bad_current_and_duplicate_sets_are_refused() {
    for case in 0..4 {
        let mut f = files(3);
        mutate_json(&mut f, "project.json", |j| match case {
            0 => j["current"] = "00000000-0000-0000-0000-000000000000".into(),
            1 => {
                let v = j["sets"][0].clone();
                j["sets"].as_array_mut().unwrap().push(v);
            }
            2 => j["sets"][0]["materialSlot"] = (-1).into(),
            _ => j["sets"][0]["id"] = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
        });
        assert!(open(f).is_err());
    }
}
#[test]
fn unknown_entries_and_json_keys_survive_both_save_modes() {
    let mut f = files(3);
    f.insert("future.bin".into(), vec![7, 3, 9]);
    mutate_json(&mut f, "project.json", |j| j["future"] = "保持する".into());
    mutate_json(&mut f, "ylp.json", |j| {
        j["future"] = serde_json::json!({"mode":13})
    });
    let p = open(f.clone()).unwrap();
    assert!(p.unknown_entries().contains(&"future.bin".into()));
    // 知らせは種類で持つ（画面が言語ごとの文を作る）。形式 3 の読み込みはマテリアル参照のメモリ上の移行も知らせる
    assert!(p
        .notes()
        .contains(&yolu_io::Note::UnknownEntryKept("future.bin".into())));
    assert!(p
        .notes()
        .contains(&yolu_io::Note::MaterialRefsMigrated { format: 3 }));
    let a = Archive::read(&p.to_bytes().unwrap()).unwrap();
    for (k, v) in &f {
        assert_eq!(a.entries()[k].as_ref(), v);
    }
    let u = p
        .upgraded(WriterInfo {
            app: "test".into(),
            version: "1".into(),
            unity: "none".into(),
        })
        .unwrap();
    let a = Archive::read(&u.to_bytes().unwrap()).unwrap();
    assert_eq!(a.entries()["future.bin"].as_ref(), [7, 3, 9]);
    let project: serde_json::Value = serde_json::from_slice(&a.entries()["project.json"]).unwrap();
    assert_eq!(project["future"], "保持する");
    assert!(project["sets"][0].get("materialSlot").is_none());
    assert!(project["sets"][0].get("material").is_some());
    let info: serde_json::Value = serde_json::from_slice(&a.entries()["ylp.json"]).unwrap();
    assert_eq!(info["future"]["mode"], 13);
}
#[test]
fn legacy_material_slot_fallback_is_reported() {
    let mut f = files(1);
    f.insert("view.json".into(), b"{\"materialSlot\":-1}".to_vec());
    let p = open(f).unwrap();
    assert_eq!(p.sets()[0].material, yolu_io::MaterialRef::PendingSlot(0));
    assert!(p
        .notes()
        .iter()
        .any(|n| matches!(n, yolu_io::Note::ViewSlotUnreadable(_)) && n.to_string().contains("スロット")));
}
#[test]
fn resource_unknown_kind_origin_and_missing_pixels_are_refused() {
    for case in 0..4 {
        let mut f = files(4);
        mutate_json(&mut f, "resources.json", |j| match case {
            0 => j["resources"][0]["kind"] = "future".into(),
            1 => j["resources"][0]["origin"] = serde_json::json!({"type":"future"}),
            2 => j["resources"][0]["content"] = "0".repeat(64).into(),
            _ => j["resources"][0]["width"] = 8193.into(),
        });
        assert!(open(f).is_err());
    }
}
#[test]
fn resource_pixels_hash_and_dimensions_are_verified() {
    for case in 0..2 {
        let mut f = files(4);
        if case == 0 {
            mutate_json(&mut f, "resources.json", |j| {
                j["resources"][0]["width"] = 8192.into()
            });
        } else {
            let png = f
                .keys()
                .find(|k| k.starts_with("resources/") && k.ends_with(".png"))
                .unwrap()
                .clone();
            f.get_mut(&png).unwrap()[0] = 0;
        }
        assert!(open(f).is_err());
    }
}
#[test]
fn resource_local_file_id_is_exact_and_unsafe_library_path_is_refused() {
    let mut f = files(4);
    mutate_json(
        &mut f,
        "resources.json",
        |j| j["resources"][0]["origin"] = serde_json::json!({"type":"unityAsset","guid":"0123456789abcdef0123456789abcdef","path":"Assets/Textures/Sample.png","localFileID":i64::MIN+37}),
    );
    let p = open(f.clone()).unwrap();
    assert_eq!(
        p.resources()[0].metadata["origin"]["localFileID"].as_i64(),
        Some(i64::MIN + 37)
    );
    mutate_json(
        &mut f,
        "resources.json",
        |j| j["resources"][0]["origin"] = serde_json::json!({"type":"library","file":"../bad","sha256":"a".repeat(64),"length":0}),
    );
    assert!(open(f).is_err());
}
#[test]
fn shared_smart_material_bytes_cannot_be_claimed_as_mask() {
    let mut f = files(5);
    mutate_json(&mut f, "resources.json", |j| {
        let resources = j["resources"].as_array_mut().unwrap();
        let mut duplicate = resources
            .iter()
            .find(|r| r["kind"] == "smartMaterial")
            .unwrap()
            .clone();
        duplicate["id"] = "aabbccdd-0000-4000-8000-000000000001".into();
        duplicate["kind"] = "smartMask".into();
        resources.push(duplicate);
    });
    assert!(open(f).is_err());
}
#[test]
fn native_unknown_values_and_invalid_ranges_are_refused() {
    let d = NativeDocument::read(include_bytes!("fixtures/native-rich-v21.utpaint")).unwrap();
    let mut checked = 0;
    for f in d.fields() {
        let invalid = match &f.value {
            NativeValue::Float(_) => Some(NativeValue::Float(f64::NAN)),
            NativeValue::Int(_)
                if [
                    ".type",
                    ".algorithm",
                    ".blend",
                    ".kind",
                    ".channel",
                    ".anchor_read",
                    ".wrap",
                    ".mode",
                    ".shape",
                ]
                .iter()
                .any(|s| f.path.ends_with(s)) =>
            {
                Some(NativeValue::Int(9999))
            }
            _ => None,
        };
        if let Some(value) = invalid {
            assert!(d.with_value(&f.path, value).is_err(), "{}", f.path);
            checked += 1;
        }
    }
    assert!(checked > 150);
    for (path, v) in [
        ("layers[0].attributes", NativeValue::Byte(128)),
        ("layers[0].anchor_flags", NativeValue::Byte(4)),
        ("layers[0].locks", NativeValue::Int(16)),
        ("layers[1].gradients[0].blend", NativeValue::Int(0)),
    ] {
        assert!(d.with_value(path, v).is_err(), "{path}");
    }
}
#[test]
fn duplicate_layer_filter_anchor_and_parent_cycles_are_refused() {
    let d = NativeDocument::read(include_bytes!("fixtures/native-rich-v21.utpaint")).unwrap();
    for (source, dest) in [
        ("layers[0].id", "layers[1].id"),
        (
            "layers[0].filters.items[0].id",
            "layers[0].mask.filters.items[0].id",
        ),
        ("layers[0].anchor.id", "layers[0].mask.anchor.id"),
        ("layers[0].id", "layers[0].parent"),
    ] {
        assert!(
            d.with_value(dest, d.field(source).unwrap().clone())
                .is_err(),
            "{dest}"
        );
    }
}
#[test]
fn every_truncated_rich_document_is_refused() {
    let b = include_bytes!("fixtures/native-rich-v21.utpaint");
    let manual_offset = b.windows(4).rposition(|v| v == b"YLID").unwrap();
    for end in 0..b.len() {
        if end == manual_offset {
            assert!(NativeDocument::read(&b[..end]).is_ok());
        } else {
            assert!(NativeDocument::read(&b[..end]).is_err(), "{end}");
        }
    }
}
#[test]
fn selection_version_order_empty_padding_and_size_are_checked() {
    let d = NativeDocument::read(include_bytes!("fixtures/native-v21.utpaint")).unwrap();
    let mut b = b"YLSL".to_vec();
    for v in [1i32, 9, 10, 8, 1, 1, 1] {
        b.extend(v.to_le_bytes());
    }
    let mut tile = vec![0; 64];
    tile[0] = 7;
    b.extend(tile);
    let good = Selection::read(&b, &d).unwrap();
    assert_eq!(good.to_bytes(), b);
    for (offset, value) in [(4, 2), (8, 16), (24, 2), (32, 0), (33, 1)] {
        let mut bad = b.clone();
        bad[offset] = value;
        assert!(Selection::read(&bad, &d).is_err(), "{offset}");
    }
    b.push(0);
    assert!(Selection::read(&b, &d).is_err());
}
