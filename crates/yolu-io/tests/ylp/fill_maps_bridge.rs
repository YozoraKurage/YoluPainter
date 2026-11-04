//! 塗りつぶしのベイクしたマップの源（正本のバージョン 38。続きの属性の印のビット 3 と `fill_map_count`・`fill_maps[i]`）。マップの源を持つ文書
//! だけが 38 になり、ほかは前のバージョンのまま。往復・バージョンの選び方・古いバージョンの数では読まない・決まりを破る正本の断り・.ylsmart を試す。
use yolu_core::fill_image::{Projection, ProjectionMode};
use yolu_core::generator::{MapKind, MapState};
use yolu_core::{
    Channel, Document, EffectInputs, ImageColorSpace, ImageId, ImageInput, LayerId, MapInput, Rect,
    Rgba8,
};
use yolu_io::smart::SmartFile;
use yolu_io::{
    NativeDocument, NativeValue, WriterInfo, LAYER_FILL_MAPS_VERSION, LAYER_SET_CHANNELS_VERSION,
    MAX_NATIVE_VERSION, UNITY_NATIVE_VERSION,
};

const W: u32 = 32;
const H: u32 = 16;
const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const IMAGE: ImageId = ImageId(0x1234_5678_9abc_4def_8123_4567_89ab_cdef);

fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.1".into(),
        unity: "none".into(),
    }
}

fn map(kind: MapKind) -> MapInput {
    let n = (W * H) as usize;
    let data = (0..n * kind.channels())
        .map(|i| ((i * 2731 + kind as usize * 977) % 65536) as u16)
        .collect();
    MapInput::new(
        kind,
        W,
        H,
        data,
        vec![1; n],
        [0.0; 3],
        [1.0; 3],
        KEY,
        MapState::Current,
    )
    .unwrap()
}

fn inputs() -> EffectInputs {
    EffectInputs::new()
        .with_map(map(MapKind::AmbientOcclusion))
        .unwrap()
        .with_map(map(MapKind::TangentNormal))
        .unwrap()
        .with_image(
            IMAGE,
            ImageInput::new(2, 2, vec![200; 16], ImageColorSpace::Unspecified).unwrap(),
        )
}

fn doc_with_fill() -> (Document, LayerId) {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    let fill = doc
        .add_fill_layer(
            "塗り",
            &[
                (Channel::Color, Rgba8::new(1, 2, 3, 255)),
                (Channel::Roughness, Rgba8::new(128, 128, 128, 255)),
                (Channel::Height, Rgba8::new(10, 10, 10, 255)),
                (Channel::Normal, Rgba8::new(128, 128, 255, 255)),
            ],
            None,
        )
        .unwrap();
    doc.set_effect_inputs(inputs()).unwrap();
    (doc, fill)
}

/// マップの源 2 つ（Color と Normal）と画像（Height）を持ち、投影を既定から変えた塗りつぶし。
fn doc_with_maps() -> (Document, LayerId) {
    let (mut doc, fill) = doc_with_fill();
    doc.set_fill_map(fill, Channel::Color, Some(MapKind::AmbientOcclusion))
        .unwrap();
    doc.set_fill_map(fill, Channel::Normal, Some(MapKind::TangentNormal))
        .unwrap();
    doc.set_fill_image(fill, Channel::Height, Some(IMAGE))
        .unwrap();
    doc.set_fill_projection(
        fill,
        Projection {
            tiles: [2.0, 1.0],
            ..Projection::default()
        },
        false,
    )
    .unwrap();
    (doc, fill)
}

fn composite(doc: &Document) -> Vec<Vec<u8>> {
    [
        Channel::Color,
        Channel::Roughness,
        Channel::Height,
        Channel::Normal,
    ]
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

#[test]
fn map_sources_round_trip_in_version_38_with_the_same_composite() {
    assert_eq!(LAYER_FILL_MAPS_VERSION, 38);
    assert_eq!(MAX_NATIVE_VERSION, LAYER_FILL_MAPS_VERSION);
    let (doc, fill) = doc_with_maps();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), LAYER_FILL_MAPS_VERSION);
    let mut back = native.to_core().unwrap();
    let layer = back.layer(fill).unwrap();
    assert_eq!(
        layer.fill_maps().collect::<Vec<_>>(),
        vec![
            (Channel::Color, MapKind::AmbientOcclusion),
            (Channel::Normal, MapKind::TangentNormal),
        ]
    );
    assert_eq!(layer.fill_image(Channel::Height), Some(IMAGE));
    assert_eq!(layer.projection().tiles, [2.0, 1.0]);
    back.set_effect_inputs(inputs()).unwrap();
    assert_eq!(composite(&back), composite(&doc));
    // 書き直しても、読み直しても同じバイト
    let bytes = native.to_bytes();
    assert_eq!(NativeDocument::from_core(&back).unwrap().to_bytes(), bytes);
    assert_eq!(NativeDocument::read(&bytes).unwrap().to_bytes(), bytes);
}

#[test]
fn only_documents_with_a_map_source_become_version_38() {
    let (mut doc, fill) = doc_with_fill();
    let plain = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(plain.version(), UNITY_NATIVE_VERSION);
    doc.set_fill_map(fill, Channel::Roughness, Some(MapKind::Curvature))
        .unwrap();
    let with = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(with.version(), LAYER_FILL_MAPS_VERSION);
    assert_eq!(
        with.field(&path(&with, "fill_maps[0].map")),
        Some(&NativeValue::Int(MapKind::Curvature as i32))
    );
    // 外すと前のバージョンで、前と同じバイト列（欄も書かない）
    doc.set_fill_map(fill, Channel::Roughness, None).unwrap();
    let again = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(again.version(), UNITY_NATIVE_VERSION);
    assert!(again.fields().iter().all(|f| !f.path.contains("fill_map")));
    assert_eq!(again.to_bytes(), plain.to_bytes());
}

#[test]
fn a_version_37_document_reads_as_before_without_map_sources() {
    // バージョン 37 の機能（扱うチャンネルの組を変えたレイヤーセット）と画像の塗りつぶしを持つ文書
    let (mut doc, fill) = doc_with_fill();
    doc.set_fill_image(fill, Channel::Color, Some(IMAGE))
        .unwrap();
    doc.add_layer_set_with("マスク", Channel::Roughness.bit())
        .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), LAYER_SET_CHANNELS_VERSION);
    let mut back = native.to_core().unwrap();
    assert_eq!(back.layer(fill).unwrap().fill_maps().count(), 0);
    back.set_effect_inputs(inputs()).unwrap();
    assert_eq!(composite(&back), composite(&doc));
}

#[test]
fn older_version_numbers_cannot_carry_map_sources() {
    let (doc, _) = doc_with_maps();
    let bytes = NativeDocument::from_core(&doc).unwrap().to_bytes();
    assert_eq!(
        i32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        LAYER_FILL_MAPS_VERSION
    );
    // 数だけを 37 に替えた正本は、続きの属性のビット 3 を知らずに断る（黙って欄を読み飛ばさない）
    let mut older = bytes.clone();
    older[8..12].copy_from_slice(&LAYER_SET_CHANNELS_VERSION.to_le_bytes());
    let err = NativeDocument::read(&older).unwrap_err().to_string();
    assert!(err.contains("未知の続きのレイヤー属性ビット"), "{err}");
    // 0.7.x の読み手は 37 までで、38 をバージョンの数だけで断る。同じ形の断りを、この読み手の上限の次のバージョンで確かめる
    let mut newer = bytes;
    newer[8..12].copy_from_slice(&(MAX_NATIVE_VERSION + 1).to_le_bytes());
    let err = NativeDocument::read(&newer).unwrap_err().to_string();
    assert!(err.contains("未対応または範囲外"), "{err}");
}

#[test]
fn the_reader_refuses_map_sources_that_break_the_rules() {
    let (doc, _) = doc_with_maps();
    let native = NativeDocument::from_core(&doc).unwrap();
    let channel = path(&native, "fill_maps[0].channel");
    // 画像のあるチャンネル（Height = 3）・値の無いチャンネル（Emission = 5）・同じチャンネルの 2 つ目（Normal = 4）
    for c in [3, 5, 4] {
        assert!(
            native.with_value(&channel, NativeValue::Int(c)).is_err(),
            "チャンネル {c}"
        );
    }
    // 知らない種類
    let kind = path(&native, "fill_maps[0].map");
    for k in [-1, 10] {
        assert!(
            native.with_value(&kind, NativeValue::Int(k)).is_err(),
            "{k}"
        );
    }
    assert!(native.with_value(&kind, NativeValue::Int(9)).is_ok());
    // 数の範囲
    let count = path(&native, "fill_map_count");
    assert!(native.with_value(&count, NativeValue::Int(0)).is_err());
}

#[test]
fn a_smart_material_keeps_map_sources_and_reads_the_maps_where_it_is_placed() {
    let (mut doc, fill) = doc_with_fill();
    doc.set_fill_map(fill, Channel::Color, Some(MapKind::AmbientOcclusion))
        .unwrap();
    let material = doc.capture_smart_material(&[fill], "素材").unwrap();
    let file = SmartFile::from_core(&material, &writer()).unwrap();
    assert_eq!(file.fragment().version(), LAYER_FILL_MAPS_VERSION);
    let read = SmartFile::read(file.file_bytes())
        .unwrap()
        .to_core()
        .unwrap();
    assert_eq!(
        read.layers()[0].fill_maps().collect::<Vec<_>>(),
        vec![(Channel::Color, MapKind::AmbientOcclusion)]
    );
    // UV 以外の投影でも同じ（投影は画像が無くても書く）
    let triplanar = Projection {
        mode: ProjectionMode::Triplanar,
        ..Projection::default()
    };
    doc.set_fill_projection(fill, triplanar, false).unwrap();
    let back = NativeDocument::from_core(&doc).unwrap().to_core().unwrap();
    assert_eq!(
        back.layer(fill).unwrap().projection().mode,
        ProjectionMode::Triplanar
    );
    assert_eq!(back.layer(fill).unwrap().fill_maps().count(), 1);
}
