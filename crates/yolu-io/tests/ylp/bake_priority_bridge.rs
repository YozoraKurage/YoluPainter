//! 重なった UV のテクセルの持ち主の決め方（ベイクの優先。正本の版 33、頭の `bake_priority`）。既定の文書は前の版のまま欄を書かず、変えた文書
//! だけが版 33 になる。往復・版の選び方・古い版の並びとの食い違いと範囲の外の値の拒否・.ylp への保存を試す。
use std::collections::BTreeSet;
use yolu_core::mesh_maps::{MeshOverlapPriority, MeshOverlapRule};
use yolu_core::{Document, Rgba8};
use yolu_io::{
    NativeDocument, NativeValue, Project, SaveTarget, SetSpec, WriterInfo, BAKE_PRIORITY_VERSION,
    MIXING_VERSION, UNITY_NATIVE_VERSION,
};

const BINDING: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.1".into(),
        unity: "none".into(),
    }
}

fn painted() -> Document {
    let mut doc = Document::with_tile_size(40, 28, 8).unwrap();
    let layer = doc.add_layer("塗り").unwrap();
    doc.set_pixel(layer, 3, 4, Rgba8::new(200, 10, 10, 255))
        .unwrap();
    doc
}

fn priority() -> MeshOverlapPriority {
    MeshOverlapPriority::new(
        MeshOverlapRule::PositiveX,
        true,
        BINDING.into(),
        BTreeSet::from([4, 90]),
        BTreeSet::from([12]),
    )
    .unwrap()
}

fn with_priority() -> Document {
    let mut doc = painted();
    doc.set_bake_priority(priority()).unwrap();
    doc
}

#[test]
fn only_a_document_with_a_changed_priority_is_version_33() {
    assert_eq!(BAKE_PRIORITY_VERSION, 33);
    // 既定: 版も並びも前のまま（Unity 版が読める 21）
    let plain = painted();
    let native = NativeDocument::from_core(&plain).unwrap();
    assert_eq!(native.version(), UNITY_NATIVE_VERSION);
    assert!(native
        .fields()
        .iter()
        .all(|f| !f.path.starts_with("bake_priority")));
    assert!(native.to_core().unwrap().bake_priority().is_default());
    // 変えた文書は 33 で、欄を持って往復する
    let doc = with_priority();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), BAKE_PRIORITY_VERSION);
    assert_eq!(
        native.field("bake_priority.rule"),
        Some(&NativeValue::Int(2))
    );
    assert_eq!(
        native.field("bake_priority.prefer[0]"),
        Some(&NativeValue::Int(12))
    );
    let back = native.to_core().unwrap();
    assert_eq!(back.bake_priority(), &priority());
    assert_eq!(back.layers().len(), 1);
    assert!(back.undo_count() == 0, "読み込みは履歴に残さない");
    assert_eq!(
        NativeDocument::from_core(&back).unwrap().to_bytes(),
        native.to_bytes()
    );
    // 決め方だけ・0〜1 の外だけを変えた文書も 33（一覧は空で指紋も空）
    for p in [
        MeshOverlapPriority::new(
            MeshOverlapRule::LargerArea,
            false,
            String::new(),
            BTreeSet::new(),
            BTreeSet::new(),
        )
        .unwrap(),
        MeshOverlapPriority::new(
            MeshOverlapRule::LowestIndex,
            true,
            String::new(),
            BTreeSet::new(),
            BTreeSet::new(),
        )
        .unwrap(),
    ] {
        let mut d = painted();
        d.set_bake_priority(p.clone()).unwrap();
        let native = NativeDocument::from_core(&d).unwrap();
        assert_eq!(native.version(), BAKE_PRIORITY_VERSION);
        assert_eq!(
            native.field("bake_priority.binding"),
            Some(&NativeValue::Text(String::new()))
        );
        assert_eq!(native.to_core().unwrap().bake_priority(), &p);
    }
    // 既定に戻すと前の版に戻る。取り消しでも戻る
    let mut doc = with_priority();
    doc.set_bake_priority(MeshOverlapPriority::default())
        .unwrap();
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().version(),
        UNITY_NATIVE_VERSION
    );
    doc.undo().unwrap();
    assert_eq!(doc.bake_priority(), &priority());
    doc.undo().unwrap();
    assert!(doc.bake_priority().is_default());
}

#[test]
fn version_33_is_outside_what_older_readers_accept() {
    // 0.4.x の読み手の上限は版 25（`MIXING_VERSION`）。版 33 の正本は版の数だけで断られる（`.version の値 33 は未対応または範囲外です (1..25)`）
    let native = NativeDocument::from_core(&with_priority()).unwrap();
    assert!(native.version() > MIXING_VERSION);
    // 版の数だけを 25 にした版 33 の正本は、欄が余って読めない（版 25 の意味は変えない）
    let mut bytes = native.to_bytes();
    assert_eq!(&bytes[8..12], &BAKE_PRIORITY_VERSION.to_le_bytes());
    bytes[8..12].copy_from_slice(&MIXING_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&bytes).is_err());
    // 版 21 の正本の版の数だけを 33 にしても、欄が足りず読めない
    let mut bytes = NativeDocument::from_core(&painted()).unwrap().to_bytes();
    bytes[8..12].copy_from_slice(&BAKE_PRIORITY_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&bytes).is_err());
}

#[test]
fn only_versions_with_a_meaning_are_read() {
    // 読める版は 1〜25・26（分けた正本）・27〜30・32・33 だけ。間の 31 と範囲の外は、意味が決まっていないので版の数で断る
    let bytes = NativeDocument::from_core(&with_priority())
        .unwrap()
        .to_bytes();
    assert!(NativeDocument::read(&bytes).is_ok());
    for version in [31, 0, yolu_io::MAX_NATIVE_VERSION + 1, -1] {
        let mut bytes = bytes.clone();
        bytes[8..12].copy_from_slice(&version.to_le_bytes());
        let Err(e) = NativeDocument::read(&bytes) else {
            panic!("版 {version} を読めた");
        };
        assert!(
            e.to_string()
                .contains(&format!(".version の値 {version} は未対応または範囲外です")),
            "版 {version} の断り: {e}"
        );
    }
}

#[test]
fn out_of_range_values_are_refused_before_core() {
    let native = NativeDocument::from_core(&with_priority()).unwrap();
    for (path, value) in [
        ("bake_priority.rule", NativeValue::Int(4)),
        ("bake_priority.rule", NativeValue::Int(-1)),
        ("bake_priority.binding", NativeValue::Text("nothex".into())),
        ("bake_priority.binding", NativeValue::Text(String::new())),
        // 並びが昇順でない・両方の一覧に同じアイランド
        ("bake_priority.skip[1]", NativeValue::Int(3)),
        ("bake_priority.prefer[0]", NativeValue::Int(90)),
        ("bake_priority.skip[0]", NativeValue::Int(4_000_000)),
    ] {
        assert!(
            native.with_value(path, value.clone()).is_err(),
            "{path} = {value:?} を通した"
        );
    }
    // 一覧が空なのに指紋がある
    let mut d = painted();
    d.set_bake_priority(
        MeshOverlapPriority::new(
            MeshOverlapRule::NegativeX,
            false,
            String::new(),
            BTreeSet::new(),
            BTreeSet::new(),
        )
        .unwrap(),
    )
    .unwrap();
    let native = NativeDocument::from_core(&d).unwrap();
    assert!(native
        .with_value("bake_priority.binding", NativeValue::Text(BINDING.into()))
        .is_err());
    // 欄の範囲の中の値は読める（版 33 の読み手は決め方の 4 つを全部読む）
    for rule in 0..4 {
        let changed = native
            .with_value("bake_priority.rule", NativeValue::Int(rule))
            .unwrap();
        assert_eq!(
            changed.to_core().unwrap().bake_priority().rule,
            MeshOverlapRule::from_index(rule).unwrap()
        );
    }
}

#[test]
fn a_ylp_keeps_the_priority() {
    let native = NativeDocument::from_core(&with_priority()).unwrap();
    let spec = SetSpec {
        id: "0f0f0f0f-0000-4000-8000-000000000033".into(),
        name: "Set".into(),
        material: yolu_io::MaterialRef::Unassigned,
        document: Some(native.clone().into()),
        composites: vec![],
    };
    let project = Project::create(writer(), std::slice::from_ref(&spec), &spec.id).unwrap();
    let dir = std::env::temp_dir().join(format!("yolu-io-bake-priority-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("priority.ylp");
    let _ = std::fs::remove_file(&path);
    SaveTarget::create(&path).unwrap().save(&project).unwrap();
    let (again, _) = SaveTarget::open(&path).unwrap();
    let reopened = &again.sets()[0].document;
    assert_eq!(reopened.version(), BAKE_PRIORITY_VERSION);
    assert_eq!(reopened.to_core().unwrap().bake_priority(), &priority());
    std::fs::remove_dir_all(&dir).unwrap();
}
