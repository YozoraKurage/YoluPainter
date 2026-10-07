//! 層のフィルターが UV の継ぎ目をまたぐかの文書の設定（正本の版 32、頭の `filter_seams`）。既定の入の文書は前の版のまま欄を書かず、切った文書
//! だけが版 32 になる。往復・版の選び方・古い版の並びとの食い違いの拒否・.ylp への保存を試す。
use yolu_core::{Channel, Document, EffectSettings, FilterSpec, FilterTarget, Rgba8};
use yolu_io::{
    NativeDocument, NativeValue, Project, SaveTarget, SetSpec, WriterInfo, EFFECTS_VERSION,
    MIXING_VERSION, SEAMS_VERSION, SPLIT_VERSION, UNITY_NATIVE_VERSION,
};

fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.1".into(),
        unity: "none".into(),
    }
}

fn blurred() -> Document {
    let mut doc = Document::with_tile_size(40, 28, 8).unwrap();
    let layer = doc.add_layer("塗り").unwrap();
    doc.set_pixel(layer, 3, 4, Rgba8::new(200, 10, 10, 255))
        .unwrap();
    doc.add_filter(
        layer,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(3)).channels(&[Channel::Color]),
    )
    .unwrap();
    doc
}

#[test]
fn only_a_document_with_the_setting_off_is_version_32() {
    assert_eq!(SEAMS_VERSION, 32);
    // 既定の入: 版も並びも前のまま（Unity 版が読める 21）
    let on = blurred();
    assert!(on.filter_seams());
    let native = NativeDocument::from_core(&on).unwrap();
    assert_eq!(native.version(), UNITY_NATIVE_VERSION);
    assert!(native.fields().iter().all(|f| f.path != "filter_seams"));
    assert!(native.to_core().unwrap().filter_seams());
    // 切った文書は 32 で、欄を持って往復する
    let mut off = blurred();
    off.set_filter_seams(false).unwrap();
    let native = NativeDocument::from_core(&off).unwrap();
    assert_eq!(native.version(), SEAMS_VERSION);
    assert_eq!(
        native.field("filter_seams"),
        Some(&NativeValue::Bool(false))
    );
    let back = native.to_core().unwrap();
    assert!(!back.filter_seams());
    assert_eq!(back.layers()[0].filters().len(), 1);
    assert_eq!(
        NativeDocument::from_core(&back).unwrap().to_bytes(),
        native.to_bytes()
    );
    // 入に戻すと前の版に戻る
    off.set_filter_seams(true).unwrap();
    assert_eq!(
        NativeDocument::from_core(&off).unwrap().version(),
        UNITY_NATIVE_VERSION
    );
}

#[test]
fn version_32_is_outside_what_older_readers_accept() {
    // 0.4.x の読み手の上限は版 25（`MIXING_VERSION`）。版 32 の正本は版の数だけで断られる（`.version の値 32 は未対応または範囲外です (1..25)`）
    let mut off = blurred();
    off.set_filter_seams(false).unwrap();
    let native = NativeDocument::from_core(&off).unwrap();
    assert!(native.version() > MIXING_VERSION);
    // 版の数だけを 25 にした版 32 の正本は、欄が余って読めない（版 25 の意味は変えない）
    let mut bytes = native.to_bytes();
    assert_eq!(&bytes[8..12], &SEAMS_VERSION.to_le_bytes());
    bytes[8..12].copy_from_slice(&MIXING_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&bytes).is_err());
    // 版 21 の正本の版の数だけを 32 にしても、欄が足りず読めない
    let mut bytes = NativeDocument::from_core(&blurred()).unwrap().to_bytes();
    bytes[8..12].copy_from_slice(&SEAMS_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&bytes).is_err());
}

#[test]
fn only_versions_with_a_meaning_are_read() {
    // 読める版は 1〜25・26（分けた正本）・27（パスの一覧）・28（0.5.0 の効果）・32 だけ。間の 29〜31 と範囲の外は、意味が決まっていないので版の数で断る
    // （版 28 は image_generator の試験が読み書きを固定する）
    let mut off = blurred();
    off.set_filter_seams(false).unwrap();
    let bytes = NativeDocument::from_core(&off).unwrap().to_bytes();
    assert!(NativeDocument::read(&bytes).is_ok());
    for version in (SPLIT_VERSION + 1..SEAMS_VERSION)
        .filter(|v| ![yolu_io::PATHS_VERSION, EFFECTS_VERSION].contains(v))
        .chain([0, SEAMS_VERSION + 1, -1])
    {
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
fn the_reader_reads_both_values_in_version_32() {
    // 版 32 の欄は入・切のどちらも読む（ほかの新しい機能で版 32 以上になった文書は、入も書く）
    let mut off = blurred();
    off.set_filter_seams(false).unwrap();
    let native = NativeDocument::from_core(&off).unwrap();
    let on = native
        .with_value("filter_seams", NativeValue::Bool(true))
        .unwrap();
    assert!(on.to_core().unwrap().filter_seams());
}

#[test]
fn a_ylp_keeps_the_setting() {
    let mut doc = blurred();
    doc.set_filter_seams(false).unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    let spec = SetSpec {
        id: "0f0f0f0f-0000-4000-8000-000000000032".into(),
        name: "Set".into(),
        material: yolu_io::MaterialRef::Unassigned,
        document: Some(native.clone().into()),
        composites: vec![],
    };
    let project = Project::create(writer(), std::slice::from_ref(&spec), &spec.id).unwrap();
    let dir = std::env::temp_dir().join(format!("yolu-io-seams-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("seams.ylp");
    let _ = std::fs::remove_file(&path);
    SaveTarget::create(&path).unwrap().save(&project).unwrap();
    let (again, _) = SaveTarget::open(&path).unwrap();
    let reopened = &again.sets()[0].document;
    assert_eq!(reopened.version(), SEAMS_VERSION);
    assert!(!reopened.to_core().unwrap().filter_seams());
    std::fs::remove_dir_all(&dir).unwrap();
}
