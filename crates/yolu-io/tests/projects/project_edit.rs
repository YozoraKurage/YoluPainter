//! 新しいプロジェクトを作る（Project::create）と、セットの並び・名前・マテリアル・今のセットを置き換える（Project::with_sets）。
use std::collections::BTreeMap;

use yolu_core::{BrushSettings, Document, Rgba8};
use yolu_io::{
    composite_png, composite_pngs, Archive, MaterialAsset, MaterialRef, NativeDocument, Project,
    SaveTarget, SetSpec, WriterInfo,
};

fn writer() -> WriterInfo {
    WriterInfo {
        app: "試験の書き手".into(),
        version: "0.0.1".into(),
        unity: "standalone".into(),
    }
}

fn fixture(name: &str) -> Project {
    Project::read(
        &std::fs::read(format!(
            "{}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap()
}

/// 線を 1 本引いた core の文書。
fn painted(size: u32, color: Rgba8) -> Document {
    let mut doc = Document::new(size, size).unwrap();
    let layer = doc.add_layer("レイヤー 1").unwrap();
    let brush = BrushSettings {
        color,
        radius: 6.0,
        ..BrushSettings::default()
    };
    let mut s = doc.begin_stroke(layer, &brush).unwrap();
    s.add_point(&mut doc, 10.0, 10.0, 1.0, Default::default())
        .unwrap();
    s.add_point(&mut doc, 50.0, 30.0, 1.0, Default::default())
        .unwrap();
    doc.end_stroke(s).unwrap();
    doc
}

fn spec(id: &str, name: &str, material: MaterialRef, doc: Option<&Document>) -> SetSpec {
    SetSpec {
        id: id.into(),
        name: name.into(),
        material,
        document: doc.map(|d| NativeDocument::from_core(d).unwrap().into()),
        composites: doc.map(|d| composite_pngs(d).unwrap()).unwrap_or_default(),
    }
}

const A: &str = "0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0";
const B: &str = "11111111-2222-4333-8444-555555555555";
const C: &str = "22222222-3333-4444-8555-666666666666";

#[test]
fn a_new_project_round_trips_and_saves() {
    let body = painted(64, Rgba8::new(200, 30, 30, 255));
    let hair = painted(32, Rgba8::new(30, 30, 200, 255));
    let skin = MaterialRef::Material {
        name: "Skin".into(),
        asset: Some(MaterialAsset {
            guid: "0123456789abcdef0123456789abcdef".into(),
            file_id: -2100000,
        }),
    };
    let p = Project::create(
        writer(),
        &[
            spec(A, "Body", skin.clone(), Some(&body)),
            spec(B, "Hair", MaterialRef::Unassigned, Some(&hair)),
        ],
        B,
    )
    .unwrap();
    assert_eq!(p.info().format, 7);
    assert_eq!(p.info().saved_by, Some(writer()));
    assert_eq!(p.info().created_by, Some(writer()));
    assert_eq!(p.current_set(), B);
    let sets = p.sets();
    assert_eq!((sets[0].name.as_str(), &sets[0].material), ("Body", &skin));
    assert_eq!(sets[1].material, MaterialRef::Unassigned);
    let back = sets[0].document.to_core().unwrap();
    for (x, y) in [(10, 10), (30, 20), (60, 60)] {
        assert_eq!(
            back.composite_pixel(yolu_core::Channel::Color, x, y)
                .unwrap(),
            body.composite_pixel(yolu_core::Channel::Color, x, y)
                .unwrap()
        );
    }
    assert_eq!(
        p.migrated_entries()[&format!("sets/{A}/composite/Color.png")]
            .bytes()
            .unwrap()[..],
        composite_png(&body).unwrap()[..]
    );
    // 安全な保存で書いて、読み直す
    let dir = std::env::temp_dir().join(format!("yolu-io-create-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("new.ylp");
    let _ = std::fs::remove_file(&path);
    SaveTarget::create(&path).unwrap().save(&p).unwrap();
    let (again, _) = SaveTarget::open(&path).unwrap();
    assert_eq!(again.sets().len(), 2);
    assert_eq!(again.sets()[1].name, "Hair");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn create_refuses_what_the_format_refuses() {
    let doc = painted(16, Rgba8::new(1, 2, 3, 255));
    let same_name = Project::create(
        writer(),
        &[
            spec(A, "Body", MaterialRef::PendingSlot(0), Some(&doc)),
            spec(B, "BODY", MaterialRef::PendingSlot(1), Some(&doc)),
        ],
        A,
    );
    assert!(same_name.is_err(), "名前は大文字小文字を区別せずに重ねない");
    let two_unassigned = Project::create(
        writer(),
        &[
            spec(A, "a", MaterialRef::Unassigned, Some(&doc)),
            spec(B, "b", MaterialRef::Unassigned, Some(&doc)),
        ],
        A,
    );
    assert!(two_unassigned.is_err());
    let no_document = Project::create(writer(), &[spec(A, "a", MaterialRef::Unassigned, None)], A);
    assert!(no_document
        .unwrap_err()
        .to_string()
        .contains("正本がありません"));
    let bad_current = Project::create(
        writer(),
        &[spec(A, "a", MaterialRef::Unassigned, Some(&doc))],
        C,
    );
    assert!(bad_current.is_err());
}

#[test]
fn with_sets_renames_rebinds_adds_and_keeps_the_rest_byte_for_byte() {
    let old = fixture("format6.ylp");
    assert!(
        old.with_sets(writer(), &[], old.current_set()).is_err(),
        "旧形式は先に上げる"
    );
    let p = old.upgraded(writer()).unwrap();
    let ids: Vec<String> = p.sets().iter().map(|s| s.id.clone()).collect();
    let before = p.migrated_entries().clone();
    // 1 つ目: 名前とマテリアルだけ、2 つ目: 正本を替える、3 つ目: そのまま、4 つ目: 新しいセット
    let edited = painted(512, Rgba8::new(0, 160, 0, 255));
    let added = painted(128, Rgba8::new(9, 9, 9, 255));
    let specs = vec![
        spec(
            &ids[0],
            "肌",
            MaterialRef::Material {
                name: "Skin".into(),
                asset: None,
            },
            None,
        ),
        SetSpec {
            composites: Vec::new(),
            ..spec(&ids[1], "Cloth", MaterialRef::PendingSlot(1), Some(&edited))
        },
        spec(&ids[2], "Skin 2", MaterialRef::PendingSlot(2), None),
        spec(C, "Hair", MaterialRef::Unassigned, Some(&added)),
    ];
    let q = p.with_sets(writer(), &specs, C).unwrap();
    assert_eq!(q.current_set(), C);
    let names: Vec<&str> = q.sets().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["肌", "Cloth", "Skin 2", "Hair"]);
    assert_eq!(
        q.sets()[0].material,
        MaterialRef::Material {
            name: "Skin".into(),
            asset: None
        }
    );
    let after = q.migrated_entries();
    for id in [&ids[0], &ids[2]] {
        for leaf in [
            "document.utpaint",
            "composite/Color.png",
            "meshmap-Position.bin",
        ] {
            let n = format!("sets/{id}/{leaf}");
            assert_eq!(after[&n], before[&n], "正本を替えないセットはそのまま: {n}");
        }
    }
    let changed = format!("sets/{}/", ids[1]);
    assert_ne!(
        after[&format!("{changed}document.utpaint")],
        before[&format!("{changed}document.utpaint")]
    );
    assert!(
        !after.contains_key(&format!("{changed}composite/Color.png")),
        "正本を替えたら古い合成は残さない"
    );
    assert_eq!(
        after[&format!("{changed}meshmap-Position.bin")],
        before[&format!("{changed}meshmap-Position.bin")],
        "メッシュマップはモデルから決まるので残す"
    );
    for n in ["brush.json", "view.json", "thumbnail.png", "resources.json"] {
        assert_eq!(after[n], before[n], "根のエントリは残す: {n}");
    }
    assert_eq!(q.info().saved_by, Some(writer()));
    assert_eq!(q.info().created_by, p.info().created_by);
    // 元のセットを並びから外すのは断る（黙って消さない）
    assert!(q
        .with_sets(writer(), &specs[1..], C)
        .unwrap_err()
        .to_string()
        .contains("並びにありません"));
}

#[test]
fn with_sets_keeps_keys_it_does_not_know() {
    let p = fixture("format6.ylp").upgraded(writer()).unwrap();
    // 未来の書き手が足したキーを入れる
    let mut files: BTreeMap<String, Vec<u8>> = p
        .original_archive()
        .entries()
        .iter()
        .map(|(k, v)| (k.clone(), v.bytes().unwrap().to_vec()))
        .collect();
    let mut project: serde_json::Value = serde_json::from_slice(&files["project.json"]).unwrap();
    project["future"] = serde_json::json!({"x": 1});
    project["sets"][0]["future"] = serde_json::json!(true);
    project["sets"][0]["material"]["future"] = serde_json::json!("m");
    files.insert("project.json".into(), serde_json::to_vec(&project).unwrap());
    let p = Project::read(&Archive::from_entries(files).unwrap().to_bytes().unwrap()).unwrap();
    let specs: Vec<SetSpec> = p
        .sets()
        .iter()
        .map(|s| SetSpec {
            id: s.id.clone(),
            name: format!("{} 改", s.name),
            material: s.material.clone(),
            document: None,
            composites: Vec::new(),
        })
        .collect();
    let q = p.with_sets(writer(), &specs, p.current_set()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(
        &q.original_archive().entries()["project.json"]
            .bytes()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(json["future"], serde_json::json!({"x": 1}));
    assert_eq!(json["sets"][0]["future"], serde_json::json!(true));
    assert_eq!(
        json["sets"][0]["material"]["future"],
        serde_json::json!("m")
    );
    assert_eq!(json["sets"][0]["name"], serde_json::json!("Skin 改"));
}

#[test]
fn with_sets_dropping_removes_the_set_with_every_entry_and_keeps_the_rest_byte_for_byte() {
    let p = fixture("format6.ylp").upgraded(writer()).unwrap();
    let ids: Vec<String> = p.sets().iter().map(|s| s.id.clone()).collect();
    assert_eq!(ids.len(), 3);
    let before = p.migrated_entries().clone();
    let keep: Vec<SetSpec> = [0usize, 2]
        .iter()
        .map(|&i| {
            spec(
                &ids[i],
                &p.sets()[i].name,
                p.sets()[i].material.clone(),
                None,
            )
        })
        .collect();
    let dropped = [ids[1].as_str()];
    let q = p
        .with_sets_dropping(writer(), &keep, &ids[0], &dropped)
        .unwrap();
    assert_eq!(q.sets().len(), 2);
    assert_eq!(q.current_set(), ids[0]);
    let after = q.migrated_entries();
    let gone = format!("sets/{}/", ids[1]);
    assert!(
        !after.keys().any(|n| n.starts_with(&gone)),
        "消したセットのエントリ（正本・合成・メッシュマップ）は全部消える: {:?}",
        after.keys().collect::<Vec<_>>()
    );
    assert!(
        q.unknown_entries().is_empty(),
        "取り残したエントリが知らないエントリにならない"
    );
    for n in before.keys().filter(|n| !n.starts_with(&gone)) {
        if n == "project.json" || n == "ylp.json" {
            continue;
        }
        assert_eq!(after[n], before[n], "ほかのエントリはそのまま: {n}");
    }
    // 保存して読み直しても同じ
    let again = Project::read(&q.to_bytes().unwrap()).unwrap();
    assert_eq!(again.sets().len(), 2);
    assert!(again.unknown_entries().is_empty());
}

#[test]
fn with_sets_dropping_refuses_what_would_lose_a_set_silently() {
    let p = fixture("format6.ylp").upgraded(writer()).unwrap();
    let ids: Vec<String> = p.sets().iter().map(|s| s.id.clone()).collect();
    let all: Vec<SetSpec> = p
        .sets()
        .iter()
        .map(|s| spec(&s.id, &s.name, s.material.clone(), None))
        .collect();
    // どちらにも無いセットは黙って消さない
    let lost = p.with_sets_dropping(writer(), &all[..2], &ids[0], &[]);
    assert!(lost.unwrap_err().to_string().contains("並びにありません"));
    // 並びにも消すセットにもあるのは断る
    let both = p.with_sets_dropping(writer(), &all, &ids[0], &[ids[2].as_str()]);
    assert!(both.unwrap_err().to_string().contains("並びにもあります"));
    // ファイルに無い ID は消せない
    let unknown = p.with_sets_dropping(writer(), &all, &ids[0], &[C]);
    assert!(unknown
        .unwrap_err()
        .to_string()
        .contains("ファイルにありません"));
    // 全部は消せない（セットは 1 つは要る）
    let everything: Vec<&str> = ids.iter().map(String::as_str).collect();
    assert!(p
        .with_sets_dropping(writer(), &[], &ids[0], &everything)
        .is_err());
    // 今のセットを消して、今のセットを並びの外に残すのも断る
    let current_gone = p.with_sets_dropping(writer(), &all[1..], &ids[0], &[ids[0].as_str()]);
    assert!(current_gone.is_err());
    // 旧形式は先に上げる
    let old = fixture("format6.ylp");
    assert!(old
        .with_sets_dropping(writer(), &all[1..], &ids[1], &[ids[0].as_str()])
        .is_err());
}

#[test]
fn the_model_reference_lives_in_view_json_without_touching_the_rest_of_it() {
    // Unity 版が書いた view.json（モデルの GUID と選んだチャンネル）に、参照を足しても元の項目はそのまま
    let p = fixture("format6.ylp").upgraded(writer()).unwrap();
    assert_eq!(p.view_model().unwrap(), None);
    let q = p.with_view_model(Some("../models/body.fbx")).unwrap();
    assert_eq!(
        q.view_model().unwrap().as_deref(),
        Some("../models/body.fbx")
    );
    let view: serde_json::Value =
        serde_json::from_slice(&q.migrated_entries()["view.json"].bytes().unwrap()).unwrap();
    assert_eq!(view["modelAssetGuid"], "0ad8236696f48ed92abbf8de9e04a6fb");
    assert_eq!(view["selectedChannel"], 0);
    assert_eq!(view["standaloneModel"]["path"], "../models/body.fbx");
    // 正本・セット・ほかのエントリは 1 バイトも変わらない（形式も変えない）
    for (name, bytes) in p.migrated_entries() {
        if name != "view.json" && name != "ylp.json" {
            assert_eq!(q.migrated_entries()[name], *bytes, "{name}");
        }
    }
    assert_eq!(q.info().format, 7);
    // 書き換える・外す
    let r = q.with_view_model(Some("C:/models/other.fbx")).unwrap();
    assert_eq!(
        r.view_model().unwrap().as_deref(),
        Some("C:/models/other.fbx")
    );
    let s = r.with_view_model(None).unwrap();
    assert_eq!(s.view_model().unwrap(), None);
    let view: serde_json::Value =
        serde_json::from_slice(&s.migrated_entries()["view.json"].bytes().unwrap()).unwrap();
    assert!(view.get("standaloneModel").is_none());
    assert_eq!(view["modelAssetGuid"], "0ad8236696f48ed92abbf8de9e04a6fb");
    // 保存して読み直しても残る
    let again = Project::read(&q.to_bytes().unwrap()).unwrap();
    assert_eq!(
        again.view_model().unwrap().as_deref(),
        Some("../models/body.fbx")
    );
}

#[test]
fn a_project_without_view_json_gets_the_smallest_state_unity_accepts() {
    let doc = painted(16, Rgba8::new(1, 2, 3, 255));
    let p = Project::create(
        writer(),
        &[spec(A, "a", MaterialRef::PendingSlot(0), Some(&doc))],
        A,
    )
    .unwrap();
    assert_eq!(p.view_model().unwrap(), None);
    assert!(
        !p.with_view_model(None)
            .unwrap()
            .migrated_entries()
            .contains_key("view.json"),
        "外すだけなら view.json を作らない"
    );
    let q = p.with_view_model(Some("model.fbx")).unwrap();
    let view: serde_json::Value =
        serde_json::from_slice(&q.migrated_entries()["view.json"].bytes().unwrap()).unwrap();
    // Unity 版の ViewState（モデルの GUID・選んだチャンネル）の形で、モデルは無し・Color
    assert_eq!(view["modelAssetGuid"], "");
    assert_eq!(view["selectedChannel"], 0);
    assert_eq!(q.view_model().unwrap().as_deref(), Some("model.fbx"));
}

#[test]
fn the_model_reference_refuses_what_it_cannot_keep() {
    let doc = painted(16, Rgba8::new(1, 2, 3, 255));
    let p = Project::create(
        writer(),
        &[spec(A, "a", MaterialRef::PendingSlot(0), Some(&doc))],
        A,
    )
    .unwrap();
    assert!(p.with_view_model(Some("")).is_err());
    assert!(p.with_view_model(Some("a\nb.fbx")).is_err());
    assert!(p
        .with_view_model(Some(&"x".repeat(yolu_io::MODEL_PATH_MAX + 1)))
        .is_err());
    assert!(p
        .with_view_model(Some(&"x".repeat(yolu_io::MODEL_PATH_MAX)))
        .is_ok());
    // 読めない view.json は、読むとき断る。参照を外すだけなら触らず、参照を書くときは最小の形に作り直す（状態であって正本ではない）
    let mut files: BTreeMap<String, Vec<u8>> = p
        .original_archive()
        .entries()
        .iter()
        .map(|(k, v)| (k.clone(), v.bytes().unwrap().to_vec()))
        .collect();
    files.insert("view.json".into(), b"not json".to_vec());
    let broken = Project::read(
        &Archive::from_entries(files.clone())
            .unwrap()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    assert!(broken.view_model().is_err());
    let untouched = broken.with_view_model(None).unwrap();
    assert_eq!(
        untouched.migrated_entries()["view.json"].bytes().unwrap()[..],
        b"not json"[..]
    );
    let rebuilt = broken.with_view_model(Some("a.fbx")).unwrap();
    assert_eq!(rebuilt.view_model().unwrap().as_deref(), Some("a.fbx"));
    // 形の違う参照
    files.insert(
        "view.json".into(),
        br#"{"standaloneModel": {"path": 3}}"#.to_vec(),
    );
    let odd = Project::read(&Archive::from_entries(files).unwrap().to_bytes().unwrap()).unwrap();
    assert!(odd.view_model().is_err());
}
