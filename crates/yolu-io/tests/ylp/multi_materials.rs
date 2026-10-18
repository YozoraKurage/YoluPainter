//! 1 つのテクスチャセットが複数のマテリアルを持つ形（`project.json` の `sets[].materials`。.ylp の中身の形式 9）: 使うファイルだけが 9 に
//! なること、1 つに戻せば 7 に下がること、往復で鍵の並びが戻ること、決まりに合わない並びを断ること。
use serde_json::{json, Value};
use std::collections::BTreeMap;
use yolu_core::Document;
use yolu_io::{
    Archive, Error, MaterialAsset, MaterialRef, Project, SetSpec, WriterInfo, MAX_FORMAT,
    MULTI_MATERIAL_FORMAT, SAVED_SELECTIONS_FORMAT,
};

const A: &str = "5f7f1e2e-8d52-4b8e-9a31-0c0c0c0c0c11";
const B: &str = "5f7f1e2e-8d52-4b8e-9a31-0c0c0c0c0c12";

fn writer() -> WriterInfo {
    WriterInfo {
        app: "テスト".into(),
        version: "1".into(),
        unity: "なし".into(),
    }
}

fn named(name: &str) -> MaterialRef {
    MaterialRef::Material {
        name: name.into(),
        asset: None,
    }
}

fn asset(name: &str, file_id: i64) -> MaterialRef {
    MaterialRef::Material {
        name: name.into(),
        asset: Some(MaterialAsset {
            guid: "c".repeat(32),
            file_id,
        }),
    }
}

fn spec(id: &str, name: &str, materials: Vec<MaterialRef>) -> SetSpec {
    let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
    doc.add_layer("レイヤー 1").unwrap();
    SetSpec {
        id: id.into(),
        name: name.into(),
        materials,
        document: Some(yolu_io::NativeDocument::from_core(&doc).unwrap().into()),
        composites: Vec::new(),
    }
}

fn kept(p: &Project) -> Vec<SetSpec> {
    p.sets()
        .iter()
        .map(|s| SetSpec {
            id: s.id.clone(),
            name: s.name.clone(),
            materials: s.materials.clone(),
            document: None,
            composites: Vec::new(),
        })
        .collect()
}

fn project_json(p: &Project) -> Value {
    serde_json::from_slice(
        &p.original_archive().entries()["project.json"]
            .bytes()
            .unwrap(),
    )
    .unwrap()
}

fn reopened(p: &Project) -> Project {
    Project::read(&p.to_bytes().unwrap()).unwrap()
}

/// 形式 9 のファイルのエントリを直して読む（`ylp.json` の `format` と `project.json`）。
fn open_as(format: i32, change: impl FnOnce(&mut Value)) -> yolu_io::Result<Project> {
    let p = Project::create(
        writer(),
        &[
            spec(A, "Face", vec![named("Face"), named("Ear")]),
            spec(B, "Body", vec![named("Body")]),
        ],
        A,
    )
    .unwrap();
    let mut files: BTreeMap<String, Vec<u8>> = Archive::read(&p.to_bytes().unwrap())
        .unwrap()
        .entries()
        .iter()
        .map(|(k, v)| (k.clone(), v.to_vec()))
        .collect();
    let mut info: Value = serde_json::from_slice(&files["ylp.json"]).unwrap();
    info["format"] = format.into();
    files.insert("ylp.json".into(), serde_json::to_vec(&info).unwrap());
    let mut project: Value = serde_json::from_slice(&files["project.json"]).unwrap();
    change(&mut project);
    files.insert("project.json".into(), serde_json::to_vec(&project).unwrap());
    Project::read(&Archive::from_entries(files)?.to_bytes()?)
}

#[test]
fn a_set_with_two_or_more_materials_writes_format_9_and_reads_back_the_same_keys() {
    assert_eq!(MULTI_MATERIAL_FORMAT, 9);
    assert_eq!(MAX_FORMAT, MULTI_MATERIAL_FORMAT);
    let face = vec![asset("Face", 7), named("Ear"), MaterialRef::Unassigned];
    let p = Project::create(
        writer(),
        &[
            spec(A, "Face", face.clone()),
            spec(B, "Body", vec![MaterialRef::PendingSlot(3)]),
        ],
        A,
    )
    .unwrap();
    assert_eq!(p.info().format, MULTI_MATERIAL_FORMAT);
    let j = project_json(&p);
    assert!(
        j["sets"][0].get("material").is_none(),
        "2 個以上は material を書かない"
    );
    assert_eq!(
        j["sets"][0]["materials"],
        json!([
            {"name": "Face", "guid": "c".repeat(32), "fileId": 7},
            {"name": "Ear"},
            {"unassigned": true}
        ])
    );
    assert_eq!(j["sets"][1]["material"], json!({"slot": 3}));
    assert!(
        j["sets"][1].get("materials").is_none(),
        "1 個は今の形のまま"
    );
    let back = reopened(&p);
    assert_eq!(back.info().format, MULTI_MATERIAL_FORMAT);
    assert_eq!(back.sets()[0].materials, face, "鍵の並びはファイルの順");
    assert_eq!(back.sets()[1].materials, [MaterialRef::PendingSlot(3)]);
}

#[test]
fn back_to_one_material_per_set_writes_format_7_and_the_material_key() {
    let p = Project::create(
        writer(),
        &[
            spec(A, "Face", vec![named("Face"), named("Ear")]),
            spec(B, "Body", vec![named("Body")]),
        ],
        A,
    )
    .unwrap();
    assert_eq!(p.info().format, MULTI_MATERIAL_FORMAT);
    let mut sets = kept(&p);
    sets[0].materials = vec![named("Face")];
    sets[1].materials = vec![named("Body"), named("Ear")];
    let moved = p.with_sets(writer(), &sets, A).unwrap();
    assert_eq!(moved.info().format, MULTI_MATERIAL_FORMAT);
    let j = project_json(&moved);
    assert_eq!(j["sets"][0]["material"], json!({"name": "Face"}));
    assert!(j["sets"][0].get("materials").is_none());
    assert_eq!(
        j["sets"][1]["materials"],
        json!([{"name": "Body"}, {"name": "Ear"}])
    );
    assert!(j["sets"][1].get("material").is_none());
    // 1 つずつに戻すと形式 7（0.6.x の読み手でも開ける）
    sets[1].materials = vec![named("Body")];
    let single = moved.with_sets(writer(), &sets, A).unwrap();
    assert_eq!(single.info().format, 7);
    let back = reopened(&single);
    assert_eq!(back.info().format, 7);
    assert_eq!(back.sets()[1].materials, [named("Body")]);
    // セットのマテリアルの口も同じ決まり
    let multi = back
        .with_materials(A, &[named("Face"), named("Ear")])
        .unwrap();
    assert_eq!(multi.info().format, MULTI_MATERIAL_FORMAT);
    assert_eq!(reopened(&multi).sets()[0].materials.len(), 2);
    let one = multi.with_materials(A, &[named("Face")]).unwrap();
    assert_eq!(one.info().format, 7);
}

#[test]
fn multi_material_sets_take_format_9_over_the_saved_selections_format_8() {
    use yolu_core::SelectionMask;
    use yolu_io::saved_selections::SavedSelection;
    let p = Project::create(
        writer(),
        &[
            spec(A, "Face", vec![named("Face")]),
            spec(B, "Body", vec![named("Body")]),
        ],
        A,
    )
    .unwrap();
    let doc = Document::with_tile_size(32, 32, 16).unwrap();
    let saved = [SavedSelection {
        name: "目".into(),
        selection: yolu_io::Selection::from_core(&SelectionMask::rectangle(&doc, 1, 1, 9, 9))
            .unwrap(),
    }];
    let with_saved = p.with_saved_selections(A, &saved).unwrap();
    assert_eq!(with_saved.info().format, SAVED_SELECTIONS_FORMAT);
    let both = with_saved
        .with_materials(B, &[named("Body"), named("Ear")])
        .unwrap();
    assert_eq!(both.info().format, MULTI_MATERIAL_FORMAT);
    let back = reopened(&both);
    assert_eq!(back.saved_selections(A).unwrap().items.len(), 1);
    // 複数のマテリアルをやめると、残した選択範囲の 8 に戻る
    let one = back.with_materials(B, &[named("Body")]).unwrap();
    assert_eq!(one.info().format, SAVED_SELECTIONS_FORMAT);
}

#[test]
fn a_set_keeps_its_unknown_keys_when_it_gains_and_loses_materials() {
    let p = open_as(MULTI_MATERIAL_FORMAT, |p| {
        p["sets"][1]["future"] = json!("kept");
        p["sets"][1]["material"]["future"] = json!(17);
    })
    .unwrap();
    let multi = p
        .with_materials(B, &[named("Body"), named("Tail")])
        .unwrap();
    let j = project_json(&multi);
    assert_eq!(j["sets"][1]["future"], "kept", "セットの知らないキーは残す");
    assert!(j["sets"][1].get("material").is_none());
    let one = multi.with_materials(B, &[named("Body")]).unwrap();
    assert_eq!(project_json(&one)["sets"][1]["future"], "kept");
}

#[test]
fn materials_shapes_against_the_rules_are_refused() {
    // 形の正しい形式 9 のファイルは開ける
    let p = open_as(MULTI_MATERIAL_FORMAT, |_| {}).unwrap();
    assert_eq!(p.sets()[0].materials, [named("Face"), named("Ear")]);
    let refused =
        |format: i32, change: &dyn Fn(&mut Value)| open_as(format, |p| change(p)).is_err();
    // material と materials の両方
    assert!(refused(MULTI_MATERIAL_FORMAT, &|p| p["sets"][0]
        ["material"] =
        json!({"name": "Face"})));
    // 2 個より少ない materials（1 個は material に書く）
    assert!(refused(MULTI_MATERIAL_FORMAT, &|p| p["sets"][0]
        ["materials"] =
        json!([{"name": "Face"}])));
    assert!(refused(MULTI_MATERIAL_FORMAT, &|p| p["sets"][0]
        ["materials"] =
        json!([])));
    assert!(refused(MULTI_MATERIAL_FORMAT, &|p| p["sets"][0]
        ["materials"] =
        json!({"name": "Face"})));
    // 要素の形が違う
    assert!(refused(MULTI_MATERIAL_FORMAT, &|p| p["sets"][0]
        ["materials"] =
        json!([{"name": "Face"}, {"slot": -1}])));
    // 同じセットの中の同じ鍵（名前だけの鍵も）
    assert!(refused(MULTI_MATERIAL_FORMAT, &|p| p["sets"][0]
        ["materials"] =
        json!([{"name": "Face"}, {"name": "Face"}])));
    // セットをまたぐ排他の鍵の重なり
    assert!(refused(MULTI_MATERIAL_FORMAT, &|p| {
        p["sets"][0]["materials"] = json!([{"name": "Face"}, {"unassigned": true}]);
        p["sets"][1]["material"] = json!({"unassigned": true});
    }));
    assert!(refused(MULTI_MATERIAL_FORMAT, &|p| {
        p["sets"][0]["materials"] = json!([{"name": "Face"}, {"slot": 2}]);
        p["sets"][1]["materials"] = json!([{"name": "Body"}, {"slot": 2}]);
    }));
    // 名前だけの鍵は、セットをまたげば重なってよい（今と同じ）
    assert!(!refused(MULTI_MATERIAL_FORMAT, &|p| p["sets"][1]
        ["material"] =
        json!({"name": "Ear"})));
    // 形式 8 までのファイルの materials は断る（黙って読み飛ばさない）
    for format in [7, SAVED_SELECTIONS_FORMAT] {
        let err = open_as(format, |_| {}).unwrap_err();
        assert!(err.to_string().contains("materials"), "{format}: {err}");
    }
    // 形式 9 より新しいファイルは書いたアプリを添えて断る
    assert!(matches!(
        open_as(MAX_FORMAT + 1, |_| {}),
        Err(Error::UnsupportedFormat { format, .. }) if format == MAX_FORMAT + 1
    ));
}

#[test]
fn writing_a_set_without_a_material_key_or_with_a_repeated_key_is_refused() {
    let err = Project::create(writer(), &[spec(A, "Face", Vec::new())], A).unwrap_err();
    assert!(
        err.to_string().contains("マテリアルの鍵がありません"),
        "{err}"
    );
    let err = Project::create(
        writer(),
        &[spec(A, "Face", vec![named("Face"), named("Face")])],
        A,
    )
    .unwrap_err();
    assert!(err.to_string().contains("重なっています"), "{err}");
    // セットをまたぐ排他の鍵は、書いた後の読み直しが断る
    assert!(Project::create(
        writer(),
        &[
            spec(A, "Face", vec![named("Face"), asset("Ear", 1)]),
            spec(B, "Body", vec![asset("Other name", 1)]),
        ],
        A,
    )
    .is_err());
    let p = Project::create(writer(), &[spec(A, "Face", vec![named("Face")])], A).unwrap();
    assert!(p.with_materials(A, &[]).is_err());
    assert_eq!(p.info().format, 7, "断った操作は元を変えない");
}

#[test]
fn a_project_json_over_its_budget_is_refused_when_saving() {
    // 長い名前のマテリアルをたくさん持つセット: project.json の予算（64 KiB）を超えるので、書いた後の読み直しが断る
    let many: Vec<MaterialRef> = (0..400)
        .map(|i| named(&format!("{i:03}{}", "m".repeat(250))))
        .collect();
    let err = Project::create(writer(), &[spec(A, "Face", many)], A).unwrap_err();
    assert!(err.to_string().contains("予算"), "{err}");
}
