//! ポインターレイヤー（正本のバージョン 39。レイヤーの種類 4、続きの属性の印のビット 4 と `pointer` の塊）。ポインターレイヤーのある文書だけが 39 になり、
//! ほかは前のバージョンのまま同じバイト列。3 種の指す先と読む元の往復、古いバージョンの数では読まない、知らない指す先の種類・循環・深さの超えを
//! 断り、指す先が無いものは開く、.ylsmart を試す。
use std::sync::Arc;

use yolu_core::{
    Channel, Document, EffectInputs, ExternalSet, LayerId, LayerSetId, PointerInput, PointerIssue,
    PointerTarget, Rect, Rgba8,
};
use yolu_io::smart::SmartFile;
use yolu_io::{
    NativeDocument, NativeValue, WriterInfo, LAYER_FILL_MAPS_VERSION, LAYER_SETS_VERSION,
    MAX_NATIVE_VERSION, POINTER_LAYER_VERSION,
};

const W: u32 = 32;
const H: u32 = 16;
const KEY: ExternalSet = ExternalSet {
    texture_set: 0x1111_2222_3333_4444_5555_6666_7777_8888,
    layer_set: 0x9999_aaaa_bbbb_cccc_dddd_eeee_ffff_0001,
};

fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.1".into(),
        unity: "none".into(),
    }
}

fn inputs() -> EffectInputs {
    let px: Vec<u8> = (0..8 * 8 * 4).map(|i| (i * 37 % 256) as u8).collect();
    EffectInputs::new().with_pointer(
        KEY,
        Channel::Color,
        PointerInput::new(8, 8, Arc::from(px)).unwrap(),
    )
}

/// ラスター 1 枚・2 つ目のレイヤーセット（レイヤー 1 枚）と、その 3 種の指す先のポインターレイヤー（読む元を替えたチャンネル・無効のチャンネルつき）。
struct Fixture {
    doc: Document,
    base: LayerId,
    shelf: LayerSetId,
    to_layer: LayerId,
    to_set: LayerId,
    to_other: LayerId,
}

fn fixture() -> Fixture {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    let shelf = doc.add_layer_set("別").unwrap();
    let held = doc.add_layer_in_set("別の中", shelf, None).unwrap();
    let base = doc.add_layer("元").unwrap();
    for x in 0..W {
        for y in 0..H / 2 {
            doc.set_pixel(base, x, y, Rgba8::new(x as u8 * 8, 100, 30, 255))
                .unwrap();
            doc.set_pixel(held, x, y + H / 2, Rgba8::new(10, x as u8 * 7, 200, 180))
                .unwrap();
        }
    }
    let to_layer = doc
        .add_pointer_layer("レイヤーを指す", PointerTarget::Layer(base), None)
        .unwrap();
    doc.set_pointer_source(to_layer, Channel::Roughness, Some(Channel::Color))
        .unwrap();
    doc.set_channel_enabled(to_layer, Channel::Emission, false)
        .unwrap();
    let to_set = doc
        .add_pointer_layer("セットを指す", PointerTarget::LayerSet(shelf), None)
        .unwrap();
    doc.set_layer_opacity(to_set, 0.5, false).unwrap();
    let to_other = doc
        .add_pointer_layer("外を指す", PointerTarget::External(KEY), None)
        .unwrap();
    // 無効にしたうえで読む元だけ替えたチャンネルも書く
    doc.set_pointer_source(to_other, Channel::Height, Some(Channel::Color))
        .unwrap();
    doc.set_channel_enabled(to_other, Channel::Height, false)
        .unwrap();
    doc.set_effect_inputs(inputs()).unwrap();
    Fixture {
        doc,
        base,
        shelf,
        to_layer,
        to_set,
        to_other,
    }
}

fn composite(doc: &Document) -> Vec<Vec<u8>> {
    doc.channels()
        .into_iter()
        .map(|c| doc.composite_channel(c, Rect::new(0, 0, W, H)).unwrap())
        .collect()
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

/// レイヤー `id` の正本の番号。
fn layer_index(doc: &Document, id: LayerId) -> usize {
    doc.layers().iter().position(|l| l.id() == id).unwrap()
}

fn guid_of(native: &NativeDocument, field: &str) -> NativeValue {
    native.field(field).cloned().unwrap()
}

#[test]
fn pointers_round_trip_in_version_39_with_the_same_composite() {
    assert_eq!(POINTER_LAYER_VERSION, 39);
    assert_eq!(MAX_NATIVE_VERSION, POINTER_LAYER_VERSION);
    let f = fixture();
    let native = NativeDocument::from_core(&f.doc).unwrap();
    assert_eq!(native.version(), POINTER_LAYER_VERSION);
    let mut back = native.to_core().unwrap();
    for id in [f.to_layer, f.to_set, f.to_other] {
        let (a, b) = (f.doc.layer(id).unwrap(), back.layer(id).unwrap());
        assert_eq!(a.pointer(), b.pointer(), "{}", a.name());
        assert_eq!(a.enabled_channels(), b.enabled_channels(), "{}", a.name());
        assert_eq!(a.opacity(), b.opacity());
    }
    assert_eq!(
        back.layer(f.to_layer).unwrap().pointer().unwrap().target(),
        PointerTarget::Layer(f.base)
    );
    assert_eq!(
        back.layer(f.to_set).unwrap().pointer().unwrap().target(),
        PointerTarget::LayerSet(f.shelf)
    );
    assert_eq!(
        back.layer(f.to_other)
            .unwrap()
            .pointer()
            .unwrap()
            .source(Channel::Height),
        Channel::Color
    );
    assert!(!back
        .layer(f.to_other)
        .unwrap()
        .is_channel_enabled(Channel::Height));
    back.set_effect_inputs(inputs()).unwrap();
    assert_eq!(composite(&back), composite(&f.doc));
    // 書き直しても、読み直しても同じバイト
    let bytes = native.to_bytes();
    assert_eq!(NativeDocument::from_core(&back).unwrap().to_bytes(), bytes);
    assert_eq!(NativeDocument::read(&bytes).unwrap().to_bytes(), bytes);
    // 書くチャンネルは、有効なものと読む元を替えたもの（無効で同じチャンネルを読む Emission は書かない）
    let to_layer = layer_index(&f.doc, f.to_layer);
    let count = format!("layers[{to_layer}].pointer.channel_count");
    assert_eq!(
        native.field(&count),
        Some(&NativeValue::Int(f.doc.channels().len() as i32 - 1))
    );
}

#[test]
fn only_documents_with_a_pointer_become_version_39_and_others_keep_their_bytes() {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    let base = doc.add_layer("元").unwrap();
    doc.set_pixel(base, 1, 1, Rgba8::new(1, 2, 3, 255)).unwrap();
    doc.add_layer_set("別").unwrap();
    let plain = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(plain.version(), LAYER_SETS_VERSION);
    let p = doc
        .add_pointer_layer("参照", PointerTarget::Layer(base), None)
        .unwrap();
    let with = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(with.version(), POINTER_LAYER_VERSION);
    assert_eq!(
        with.field(&path(&with, "pointer.target")),
        Some(&NativeValue::Int(0))
    );
    // 消すと前のバージョンで、前と同じバイト列（欄も書かない）
    doc.remove_layer(p).unwrap();
    let again = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(again.version(), LAYER_SETS_VERSION);
    assert!(again.fields().iter().all(|f| !f.path.contains("pointer")));
    assert_eq!(again.to_bytes(), plain.to_bytes());
}

#[test]
fn older_version_numbers_cannot_carry_pointers() {
    let f = fixture();
    let bytes = NativeDocument::from_core(&f.doc).unwrap().to_bytes();
    assert_eq!(
        i32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        POINTER_LAYER_VERSION
    );
    // 数だけを 38 に替えた正本は、レイヤーの種類 4 か続きの属性のビット 4 を知らずに断る（黙って欄を読み飛ばさない）
    let mut older = bytes.clone();
    older[8..12].copy_from_slice(&LAYER_FILL_MAPS_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&older).is_err());
    // 0.7.x の読み手はバージョンの数だけで断る。同じ形の断りを、この読み手の上限の次のバージョンで確かめる
    let mut newer = bytes;
    newer[8..12].copy_from_slice(&(MAX_NATIVE_VERSION + 1).to_le_bytes());
    let err = NativeDocument::read(&newer).unwrap_err().to_string();
    assert!(err.contains("未対応または範囲外"), "{err}");
}

#[test]
fn the_reader_refuses_unknown_targets_cycles_and_too_deep_chains() {
    let f = fixture();
    let native = NativeDocument::from_core(&f.doc).unwrap();
    let to_layer = layer_index(&f.doc, f.to_layer);
    // 知らない指す先の種類
    let target = format!("layers[{to_layer}].pointer.target");
    let err = native
        .with_value(&target, NativeValue::Int(3))
        .unwrap_err()
        .to_string();
    assert!(err.contains("target"), "{err}");
    // 空の欄（無効で、読む元も同じチャンネル）
    let source = format!("layers[{to_layer}].pointer.channels[1].source");
    let enabled = format!("layers[{to_layer}].pointer.channels[1].enabled");
    let channel = native
        .field(&format!("layers[{to_layer}].pointer.channels[1].channel"))
        .cloned()
        .unwrap();
    assert!(native
        .with_values(&[
            (source.clone(), channel),
            (enabled, NativeValue::Bool(false)),
        ])
        .is_err());
    // 自分を指す（循環）は、core にするときに断る
    let own = guid_of(&native, &format!("layers[{to_layer}].id"));
    let layer = format!("layers[{to_layer}].pointer.layer");
    let cyclic = native.with_value(&layer, own).unwrap();
    let err = cyclic.to_core().err().unwrap().to_string();
    assert!(err.contains("循環"), "{err}");
    // 文書に無いレイヤーを指すものは、断らず「指す先が無い」で開く
    let gone = native
        .with_value(&layer, NativeValue::Guid([7; 16]))
        .unwrap()
        .to_core()
        .unwrap();
    assert_eq!(
        gone.pointer_issue(f.to_layer).unwrap(),
        Some(PointerIssue::MissingTarget)
    );
}

#[test]
fn a_chain_longer_than_four_in_the_file_is_refused() {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    let base = doc.add_layer("元").unwrap();
    let mut last = base;
    for k in 0..4 {
        last = doc
            .add_pointer_layer(&format!("{k}"), PointerTarget::Layer(last), None)
            .unwrap();
    }
    let fifth = doc
        .add_pointer_layer("5", PointerTarget::Layer(base), None)
        .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    native.to_core().unwrap();
    // 5 枚目の指す先を 4 段目へ替えた正本は、5 段になるので断る
    let fifth = layer_index(&doc, fifth);
    let fourth = guid_of(&native, &format!("layers[{}].id", layer_index(&doc, last)));
    let deep = native
        .with_value(&format!("layers[{fifth}].pointer.layer"), fourth)
        .unwrap();
    let err = deep.to_core().err().unwrap().to_string();
    assert!(err.contains("4 段"), "{err}");
}

#[test]
fn a_smart_material_keeps_pointers_and_points_inside_at_the_placed_copy() {
    let f = fixture();
    let mut doc = f.doc;
    let material = doc
        .capture_smart_material(&[f.base, f.to_layer], "素材")
        .unwrap();
    let file = SmartFile::from_core(&material, &writer()).unwrap();
    assert_eq!(file.fragment().version(), POINTER_LAYER_VERSION);
    let read = SmartFile::read(file.file_bytes())
        .unwrap()
        .to_core()
        .unwrap();
    let placed = doc
        .place_smart_material(&read, &yolu_core::smart::SmartPlacement::default())
        .unwrap();
    let pointer = placed
        .layers
        .iter()
        .find(|id| doc.layer(**id).unwrap().pointer().is_some())
        .copied()
        .unwrap();
    let copy_of_base = placed
        .layers
        .iter()
        .find(|id| **id != pointer)
        .copied()
        .unwrap();
    assert_eq!(
        doc.layer(pointer).unwrap().pointer().unwrap().target(),
        PointerTarget::Layer(copy_of_base)
    );
    assert_eq!(doc.pointer_issue(pointer).unwrap(), None);
}

/// レイヤーセットを保存しない形（既定の 1 つだけの文書は版 36 より前で書く）でも、最初のレイヤーセットの ID は文書の ID から決まり、開き直しても
/// 同じ（ほかのテクスチャセットのポインターレイヤーが指すレイヤーセットを見失わない）。
#[test]
fn the_only_layer_set_keeps_its_id_without_saving_layer_sets() {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    doc.add_layer("元").unwrap();
    assert!(doc.has_default_layer_sets());
    assert_eq!(doc.layer_sets()[0].id, LayerSetId::first_of(doc.id()));
    let native = NativeDocument::from_core(&doc).unwrap();
    assert!(
        native.version() < LAYER_SETS_VERSION,
        "{}",
        native.version()
    );
    let back = native.to_core().unwrap();
    assert_eq!(back.id(), doc.id());
    assert_eq!(back.layer_sets()[0].id, doc.layer_sets()[0].id);
    assert!(back.has_default_layer_sets());
    assert_eq!(
        NativeDocument::from_core(&back).unwrap().to_bytes(),
        native.to_bytes(),
        "同じバイト列"
    );
}

/// 最初のレイヤーセットを消し、名前の空いたレイヤーセット（ID は文書の ID から決まる物と違う）だけが残った文書は、見かけは既定の形でも
/// レイヤーセットを書き、開き直しても ID が同じ（欄を書かないと、開き直しで最初のレイヤーセットの ID に替わって指せなくなる）。
#[test]
fn a_lone_layer_set_with_another_id_is_saved_with_its_id() {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    doc.add_layer("元").unwrap();
    let other = doc.add_layer_set("").unwrap();
    for c in doc.channels() {
        doc.set_channel_output(c, Some(other)).unwrap();
    }
    let first = doc.layer_sets()[0].id;
    doc.remove_layer_set(first).unwrap();
    assert_eq!(doc.layer_sets().len(), 1);
    assert_eq!(doc.layer_sets()[0].id, other);
    assert_ne!(other, LayerSetId::first_of(doc.id()));
    assert!(!doc.has_default_layer_sets());
    let native = NativeDocument::from_core(&doc).unwrap();
    assert!(
        native.version() >= LAYER_SETS_VERSION,
        "{}",
        native.version()
    );
    let back = native.to_core().unwrap();
    assert_eq!(back.layer_sets().len(), 1);
    assert_eq!(back.layer_sets()[0].id, other);
    assert_eq!(
        NativeDocument::from_core(&back).unwrap().to_bytes(),
        native.to_bytes(),
        "同じバイト列"
    );
}
