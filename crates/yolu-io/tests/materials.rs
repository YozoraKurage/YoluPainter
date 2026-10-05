use serde_json::{json, Value};
use std::collections::BTreeMap;
use yolu_io::{Archive, MaterialAsset, MaterialRef, Project, WriterInfo};
fn fixture() -> BTreeMap<String, Vec<u8>> {
    Archive::read(include_bytes!("fixtures/m1-mode-00.ylp"))
        .unwrap()
        .entries()
        .iter()
        .map(|(k, v)| (k.clone(), v.to_vec()))
        .collect()
}
fn open(
    mut files: BTreeMap<String, Vec<u8>>,
    change: impl FnOnce(&mut Value),
) -> yolu_io::Result<Project> {
    let mut project = serde_json::from_slice(&files["project.json"]).unwrap();
    change(&mut project);
    files.insert("project.json".into(), serde_json::to_vec(&project).unwrap());
    Project::read(&Archive::from_entries(files)?.to_bytes()?)
}
#[test]
fn format7_all_material_variants_roundtrip_every_entry() {
    let original = include_bytes!("fixtures/m1-mode-00.ylp");
    let p = Project::read(original).unwrap();
    assert_eq!(p.info().format, 7);
    assert_ne!(p.current_set(), p.sets()[0].id);
    assert_eq!(p.sets()[0].material, MaterialRef::Unassigned);
    assert_eq!(
        p.sets()[1].material,
        MaterialRef::Material {
            name: "同名".into(),
            asset: None
        }
    );
    assert_eq!(
        p.sets()[2].material,
        MaterialRef::Material {
            name: "同名".into(),
            asset: Some(MaterialAsset {
                guid: "a".repeat(32),
                file_id: i64::MIN
            })
        }
    );
    assert_eq!(p.sets()[3].material, MaterialRef::PendingSlot(65535));
    let rewritten = Archive::read(&p.to_bytes().unwrap()).unwrap();
    let original = Archive::read(original).unwrap();
    assert_eq!(original.entries(), rewritten.entries());
    assert_eq!(original.manifest(), rewritten.manifest());
}
#[test]
fn format6_and_shared_materials_migrate_slots_preserving_other_entries() {
    for bytes in [
        include_bytes!("fixtures/format6.ylp").as_slice(),
        include_bytes!("fixtures/format5-shared-materials.ylp").as_slice(),
    ] {
        let p = Project::read(bytes).unwrap();
        let original: Value =
            serde_json::from_slice(&p.original_archive().entries()["project.json"].bytes().unwrap()).unwrap();
        let upgraded = p
            .upgraded(WriterInfo {
                app: "test".into(),
                version: "1".into(),
                unity: "none".into(),
            })
            .unwrap();
        assert_eq!(upgraded.info().format, 7);
        for (i, set) in upgraded.sets().iter().enumerate() {
            assert_eq!(
                set.material,
                MaterialRef::PendingSlot(
                    original["sets"][i]["materialSlot"].as_u64().unwrap() as u16
                )
            );
        }
        for (name, bytes) in p.original_archive().entries() {
            if name != "project.json" && name != "ylp.json" {
                assert_eq!(
                    upgraded.original_archive().entries()[name],
                    *bytes,
                    "{name}"
                );
            }
        }
    }
}
#[test]
fn legacy_accepts_material_first_but_format7_refuses_slot_only() {
    let mut files = fixture();
    let mut info: Value = serde_json::from_slice(&files["ylp.json"]).unwrap();
    info["format"] = 6.into();
    files.insert("ylp.json".into(), serde_json::to_vec(&info).unwrap());
    let p = open(files, |p| p["sets"][0]["materialSlot"] = json!(-1)).unwrap();
    assert_eq!(p.sets()[0].material, MaterialRef::Unassigned);
    assert!(open(fixture(), |p| {
        p["sets"][0].as_object_mut().unwrap().remove("material");
        p["sets"][0]["materialSlot"] = json!(0);
    })
    .is_err());
}
#[test]
fn material_invalid_shapes_and_exact_integer_ranges_are_refused() {
    for material in [
        Value::Null,
        json!({}),
        json!({"unassigned":false}),
        json!({"unassigned":1}),
        json!({"unassigned":true,"slot":0}),
        json!({"name":"x","slot":0}),
        json!({"slot":-1}),
        json!({"slot":65536}),
        json!({"slot":1.0}),
        json!({"name":null}),
        json!({"name":"bad\nname"}),
        json!({"name":"😀".repeat(129)}),
        json!({"name":"x","guid":"a".repeat(32)}),
        json!({"name":"x","fileId":1}),
        json!({"name":"x","guid":"A".repeat(32),"fileId":1}),
        json!({"name":"x","guid":"a".repeat(32),"fileId":1.5}),
        json!({"name":"x","guid":"a".repeat(32),"fileId":u64::MAX}),
    ] {
        assert!(
            open(fixture(), |p| p["sets"][0]["material"] = material.clone()).is_err(),
            "{material}"
        );
    }
    for material in [
        json!({"name":""}),
        json!({"name":" ","unassigned":false}),
        json!({"name":"😀".repeat(128)}),
        json!({"name":"x","guid":"b".repeat(32),"fileId":i64::MAX}),
    ] {
        open(fixture(), |p| p["sets"][0]["material"] = material).unwrap();
    }
}
#[test]
fn exclusive_references_reject_duplicates_but_plain_names_can_repeat() {
    for material in [
        json!({"unassigned":true}),
        json!({"slot":7}),
        json!({"name":"first","guid":"a".repeat(32),"fileId":42}),
    ] {
        assert!(open(fixture(), |p| {
            p["sets"][0]["material"] = material.clone();
            p["sets"][1]["material"] = material.clone();
            if material.get("name").is_some() {
                p["sets"][1]["material"]["name"] = json!("different");
            }
        })
        .is_err());
    }
    open(fixture(), |p| {
        p["sets"][0]["material"] = json!({"name":"same"});
        p["sets"][1]["material"] = json!({"name":"same"});
    })
    .unwrap();
}
#[test]
fn material_edit_is_validated_and_preserves_other_files_and_unknown_keys() {
    let p = open(fixture(), |p| {
        p["future"] = json!(19);
        p["sets"][0]["future"] = json!("kept");
        p["sets"][0]["material"]["future"] = json!({"extension": 17});
    })
    .unwrap();
    let material = MaterialRef::Material {
        name: "変更した参照".into(),
        asset: Some(MaterialAsset {
            guid: "b".repeat(32),
            file_id: 9007199254740993,
        }),
    };
    let updated = p.with_material(&p.sets()[0].id, material.clone()).unwrap();
    assert_eq!(updated.sets()[0].material, material);
    for (name, bytes) in p.original_archive().entries() {
        if name != "project.json" {
            assert_eq!(updated.original_archive().entries()[name], *bytes);
        }
    }
    let j: Value =
        serde_json::from_slice(&updated.original_archive().entries()["project.json"].bytes().unwrap()).unwrap();
    assert_eq!(j["future"], 19);
    assert_eq!(j["sets"][0]["future"], "kept");
    assert_eq!(j["sets"][0]["material"]["future"]["extension"], 17);
    assert!(p
        .with_material(&p.sets()[0].id, MaterialRef::PendingSlot(65535))
        .is_err());
    assert!(p.with_material("missing", MaterialRef::Unassigned).is_err());
    assert!(p
        .with_material(
            &p.sets()[0].id,
            MaterialRef::Material {
                name: "x".into(),
                asset: Some(MaterialAsset {
                    guid: "invalid".into(),
                    file_id: 0
                })
            }
        )
        .is_err());
    let legacy = Project::read(include_bytes!("fixtures/format6.ylp")).unwrap();
    assert!(legacy
        .with_material(legacy.current_set(), MaterialRef::Unassigned)
        .is_err());
}
