//! UDIM のシートの並び（正本のバージョン 40。頭の `udim`）。UDIM の文書だけが 40 になり、ほかは前のバージョンのまま。往復・前のバージョンを読む・
//! 古いバージョンの数では読まない・範囲の外の値を断る・.ylp の保存と開き直しを試す。
use yolu_core::udim::UdimLayout;
use yolu_core::{Channel, Document, PointerTarget, Rect, Rgba8};
use yolu_io::{
    MaterialRef, NativeDocument, NativeValue, Project, SetSpec, WriterInfo,
    LAYER_FILL_MAPS_VERSION, LAYER_SET_CHANNELS_VERSION, MAX_NATIVE_VERSION, POINTER_LAYER_VERSION,
    UDIM_VERSION, UNITY_NATIVE_VERSION,
};

const S: u32 = 16;

fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.1".into(),
        unity: "none".into(),
    }
}

/// 3 つのタイル（1001・1002・1011）のシート。1002 のセルに印。
fn sheet() -> Document {
    let mut doc = Document::with_tile_size(S, S, 8).unwrap();
    let id = doc.add_layer("a").unwrap();
    doc.set_pixel(id, 2, 3, Rgba8::new(200, 10, 10, 255))
        .unwrap();
    doc.enable_udim(vec![1001, 1002, 1011]).unwrap();
    doc.set_pixel(id, S + 4, 5, Rgba8::new(10, 200, 10, 255))
        .unwrap();
    doc
}

fn whole(doc: &Document) -> Vec<u8> {
    doc.composite_channel(Channel::Color, Rect::new(0, 0, doc.width(), doc.height()))
        .unwrap()
}

fn path(native: &NativeDocument, tail: &str) -> String {
    native
        .fields()
        .iter()
        .find(|f| f.path.ends_with(tail))
        .unwrap_or_else(|| panic!("{tail}"))
        .path
        .clone()
}

#[test]
fn a_udim_sheet_round_trips_in_version_40() {
    assert_eq!(UDIM_VERSION, 40);
    assert_eq!(MAX_NATIVE_VERSION, UDIM_VERSION);
    let doc = sheet();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), UDIM_VERSION);
    let back = native.to_core().unwrap();
    assert_eq!(back.udim(), doc.udim());
    assert_eq!(whole(&back), whole(&doc));
    let bytes = native.to_bytes();
    assert_eq!(NativeDocument::from_core(&back).unwrap().to_bytes(), bytes);
    assert_eq!(NativeDocument::read(&bytes).unwrap().to_bytes(), bytes);
}

#[test]
fn only_udim_sheets_become_version_40() {
    let mut doc = sheet();
    doc.disable_udim().unwrap();
    let plain = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(plain.version(), UNITY_NATIVE_VERSION);
    assert!(plain.fields().iter().all(|f| !f.path.starts_with("udim")));
}

#[test]
fn older_versions_read_as_documents_without_udim() {
    // バージョン 37・38 の文書は欄が無く、UDIM でない文書として読む
    let mut doc = Document::with_tile_size(S, S, 8).unwrap();
    doc.add_layer_set_with("マスク", Channel::Roughness.bit())
        .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), LAYER_SET_CHANNELS_VERSION);
    assert!(native.to_core().unwrap().udim().is_none());
    // 数だけを 38 に替えた UDIM の正本は、頭の後ろに読み残しがあるとして断る（黙って欄を読み飛ばさない）
    let bytes = NativeDocument::from_core(&sheet()).unwrap().to_bytes();
    let mut older = bytes.clone();
    older[8..12].copy_from_slice(&LAYER_FILL_MAPS_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&older).is_err());
    // 39（ポインターレイヤー）の数にしても、UDIM の欄を知らずに断る
    let mut pointer = bytes;
    pointer[8..12].copy_from_slice(&POINTER_LAYER_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&pointer).is_err());
}

#[test]
fn a_udim_sheet_with_a_pointer_layer_is_version_40_and_keeps_both() {
    // 40 は 39 の中身（ポインターレイヤー）も書ける。両方を持つ文書は 40 で、読み直すと並びとポインターレイヤーが残る
    let mut doc = sheet();
    let base = doc.layers()[0].id();
    let pointer = doc
        .add_pointer_layer("指す", PointerTarget::Layer(base), None)
        .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), UDIM_VERSION);
    let back = NativeDocument::read(&native.to_bytes())
        .unwrap()
        .to_core()
        .unwrap();
    assert_eq!(back.udim(), doc.udim());
    assert_eq!(
        back.layer(pointer)
            .and_then(|l| l.pointer())
            .map(|p| p.target()),
        Some(PointerTarget::Layer(base))
    );
    assert_eq!(whole(&back), whole(&doc));
}

#[test]
fn the_reader_refuses_udim_values_out_of_range() {
    let native = NativeDocument::from_core(&sheet()).unwrap();
    let tile = path(&native, "udim.tiles[1]");
    // 範囲の外の番号・昇順でない番号
    for t in [1000, 2001, 1001] {
        assert!(
            native.with_value(&tile, NativeValue::Int(t)).is_err(),
            "{t}"
        );
    }
    assert!(native.with_value(&tile, NativeValue::Int(1003)).is_ok());
    // 列の数: 0・タイルの数を超える・キャンバスを割り切らない（3 列は 32 を割り切らない）
    let columns = path(&native, "udim.columns");
    for c in [0, 4, 3] {
        assert!(
            native.with_value(&columns, NativeValue::Int(c)).is_err(),
            "{c}"
        );
    }
    let count = path(&native, "udim.tile_count");
    assert!(native.with_value(&count, NativeValue::Int(0)).is_err());
    assert!(native.with_value(&count, NativeValue::Int(101)).is_err());
}

#[test]
fn a_saved_project_opens_with_the_same_sheet() {
    let doc = sheet();
    let native = NativeDocument::from_core(&doc).unwrap();
    let spec = SetSpec {
        id: "0f0f0f0f-0000-4000-8000-0000000000d1".into(),
        name: "Set".into(),
        materials: vec![MaterialRef::Unassigned],
        document: Some(native.clone().into()),
        composites: vec![],
    };
    let project = Project::create(writer(), std::slice::from_ref(&spec), &spec.id).unwrap();
    let bytes = project.to_bytes().unwrap();
    let opened = Project::read(&bytes).unwrap();
    let reopened = &opened.sets()[0].document;
    assert_eq!(reopened.version(), UDIM_VERSION);
    let back = reopened.to_core().unwrap();
    assert_eq!(
        back.udim().map(|l| l.as_ref().clone()),
        Some(UdimLayout::new(vec![1001, 1002, 1011], 2, S, S).unwrap())
    );
    assert_eq!(whole(&back), whole(&doc));
}
