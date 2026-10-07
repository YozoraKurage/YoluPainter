//! 塗りつぶしの点のグラデーションと、塗りつぶしの画像ごとの異方性のフィルターの入・切の保存・復元（正本の版 29）。使う文書だけが版 29 に
//! なり、使わない文書は前の版のまま。往復・版の選び方・古い版の読み手の断り・.ylsmart の断りを試す。
use yolu_core::fill_points::{GradientPoint, PointGradient, PointSpace};
use yolu_core::generator::{ColorStop, LuminanceCorrection, MixMode, OpacityStop, Ramp};
use yolu_core::{
    AdjustmentSettings, Channel, Document, EffectInputs, GradientMap, ImageId, ImageInput, LayerId,
    Rgba8,
};
use yolu_io::smart::REFUSAL_POINT_GRADIENTS;
use yolu_io::{
    NativeDocument, NativeValue, MAX_NATIVE_VERSION, MIXING_VERSION, POINT_GRADIENT_VERSION,
    UNITY_NATIVE_VERSION,
};

const IMAGE: ImageId = ImageId(0x1234_5678_9abc_4def_8123_4567_89ab_cdef);

fn doc_with_fill() -> (Document, LayerId) {
    let mut doc = Document::with_tile_size(32, 16, 8).unwrap();
    let fill = doc
        .add_fill_layer(
            "塗り",
            &[
                (Channel::Color, Rgba8::new(1, 2, 3, 255)),
                (Channel::Roughness, Rgba8::new(128, 128, 128, 255)),
                (Channel::Height, Rgba8::new(10, 10, 10, 255)),
            ],
            None,
        )
        .unwrap();
    (doc, fill)
}

fn points() -> PointGradient {
    PointGradient {
        space: PointSpace::Model,
        spread: 0.25,
        points: vec![
            GradientPoint {
                position: [0.125, -3.5, 1e-3],
                color: Rgba8::new(250, 10, 20, 200),
            },
            GradientPoint {
                position: [1.5, 2.0, -0.75],
                color: Rgba8::new(0, 0, 255, 0),
            },
        ],
    }
}

fn uv_points() -> PointGradient {
    PointGradient {
        space: PointSpace::Uv,
        spread: 0.0,
        points: vec![GradientPoint {
            position: [0.3, 1.25, 0.0],
            color: Rgba8::new(77, 77, 77, 255),
        }],
    }
}

fn with_image(doc: &mut Document, fill: LayerId) {
    let inputs = EffectInputs::new().with_image(
        IMAGE,
        ImageInput::new(2, 2, vec![255; 16], yolu_core::ImageColorSpace::Unspecified).unwrap(),
    );
    doc.set_effect_inputs(inputs).unwrap();
    doc.set_fill_image(fill, Channel::Height, Some(IMAGE))
        .unwrap();
}

#[test]
fn point_gradients_and_isotropic_images_round_trip_in_version_29() {
    assert_eq!(POINT_GRADIENT_VERSION, 29);
    let (mut doc, fill) = doc_with_fill();
    doc.set_fill_points(fill, Channel::Color, Some(points()), false)
        .unwrap();
    doc.set_fill_points(fill, Channel::Roughness, Some(uv_points()), false)
        .unwrap();
    with_image(&mut doc, fill);
    doc.set_fill_anisotropic(fill, Channel::Height, false)
        .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), POINT_GRADIENT_VERSION);
    let back = native.to_core().unwrap();
    let layer = back.layer(fill).unwrap();
    assert_eq!(layer.fill_points(Channel::Color), Some(&points()));
    assert_eq!(layer.fill_points(Channel::Roughness), Some(&uv_points()));
    assert!(!layer.fill_anisotropic(Channel::Height));
    assert_eq!(layer.fill_image(Channel::Height), Some(IMAGE));
    // 書き直しても、読み直しても同じバイト
    let bytes = native.to_bytes();
    assert_eq!(NativeDocument::from_core(&back).unwrap().to_bytes(), bytes);
    assert_eq!(NativeDocument::read(&bytes).unwrap().to_bytes(), bytes);
}

#[test]
fn only_documents_that_use_them_become_version_29() {
    let (mut doc, fill) = doc_with_fill();
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().version(),
        UNITY_NATIVE_VERSION
    );
    // 異方性で読む画像（既定）は前の版のまま、欄も書かない
    with_image(&mut doc, fill);
    let plain = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(plain.version(), UNITY_NATIVE_VERSION);
    assert!(plain
        .fields()
        .iter()
        .all(|f| !f.path.ends_with(".anisotropic")));
    // 切ると 29、戻すと前の版
    doc.set_fill_anisotropic(fill, Channel::Height, false)
        .unwrap();
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().version(),
        POINT_GRADIENT_VERSION
    );
    doc.undo().unwrap();
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().version(),
        UNITY_NATIVE_VERSION
    );
    // 点のグラデーションを置くと 29、外すと前の版
    doc.set_fill_points(fill, Channel::Color, Some(uv_points()), false)
        .unwrap();
    let with_points = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(with_points.version(), POINT_GRADIENT_VERSION);
    // 版 29 の文書では、異方性で読む画像も欄を書く（真）
    let path = with_points
        .fields()
        .iter()
        .find(|f| f.path.ends_with("images[0].anisotropic"))
        .expect("画像の読み方の欄")
        .path
        .clone();
    assert_eq!(with_points.field(&path), Some(&NativeValue::Bool(true)));
    doc.set_fill_points(fill, Channel::Color, None, false)
        .unwrap();
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().version(),
        UNITY_NATIVE_VERSION
    );
}

#[test]
fn older_readers_refuse_version_29_by_its_number_and_older_numbers_cannot_carry_the_fields() {
    let (mut doc, fill) = doc_with_fill();
    doc.set_fill_points(fill, Channel::Color, Some(points()), false)
        .unwrap();
    let bytes = NativeDocument::from_core(&doc).unwrap().to_bytes();
    assert_eq!(
        i32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        POINT_GRADIENT_VERSION
    );
    // 0.4.x の読み手は版の上限が 25（`MIXING_VERSION`）で、版の数だけを見てファイルに触れずに断る。同じ形の断りを、この読み手の上限の
    // 次の版で確かめる
    const { assert!(POINT_GRADIENT_VERSION > MIXING_VERSION) };
    let mut newer = bytes.clone();
    newer[8..12].copy_from_slice(&(MAX_NATIVE_VERSION + 1).to_le_bytes());
    let err = NativeDocument::read(&newer).unwrap_err().to_string();
    assert!(err.contains("未対応または範囲外"), "{err}");
    // 版の数だけを 25 に書き換えた版 29 の正本は、属性のビット 6 が読めずに断る
    let mut older = bytes.clone();
    older[8..12].copy_from_slice(&MIXING_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&older).is_err());
}

/// 読める版は 1〜25・26（分けた正本）・29 の集合で、間の 27・28 は意味が無い。版 25 の正本（混色のあるグラデーションマップ）の版の数だけを
/// 27・28 に替えても、版 25 の並びとして読まずにファイルに触れず断る（範囲 1..=29 で検査すると、この文書は読めてしまう）。
#[test]
fn versions_between_the_known_ones_are_refused_instead_of_read_as_version_25() {
    let stop = |position, rgb: [u8; 3]| ColorStop {
        position,
        color: Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
        midpoint: 0.5,
    };
    let ramp = Ramp::new(
        vec![stop(0.0, [10, 20, 90]), stop(1.0, [250, 240, 200])],
        vec![
            OpacityStop {
                position: 0.0,
                opacity: 1.0,
                midpoint: 0.5,
            },
            OpacityStop {
                position: 1.0,
                opacity: 1.0,
                midpoint: 0.5,
            },
        ],
        None,
    )
    .unwrap()
    .with_mixing(MixMode::Linear, LuminanceCorrection::default());
    let mut doc = Document::with_tile_size(16, 16, 8).unwrap();
    doc.add_adjustment_layer(
        "マップ",
        AdjustmentSettings::gradient_map(GradientMap::new(ramp, false)),
        None,
        None,
    )
    .unwrap();
    let bytes = NativeDocument::from_core(&doc).unwrap().to_bytes();
    assert_eq!(
        i32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        MIXING_VERSION
    );
    assert!(NativeDocument::read(&bytes).is_ok());
    // 意味の決まっていない版（31）は、版 25 の並びとして読み進めずに断る
    let gap = 31_i32;
    let mut between = bytes;
    between[8..12].copy_from_slice(&gap.to_le_bytes());
    let err = NativeDocument::read(&between).unwrap_err().to_string();
    assert!(
        err.contains(&format!("{gap} は未対応または範囲外")),
        "版 {gap}: {err}"
    );
}

#[test]
fn the_reader_refuses_values_the_writer_never_writes() {
    let (mut doc, fill) = doc_with_fill();
    doc.set_fill_points(fill, Channel::Color, Some(uv_points()), false)
        .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    let path = |tail: &str| {
        native
            .fields()
            .iter()
            .find(|f| f.path.ends_with(tail))
            .unwrap_or_else(|| panic!("{tail}"))
            .path
            .clone()
    };
    assert!(native
        .with_value(&path(".space"), NativeValue::Int(2))
        .is_err());
    assert!(native
        .with_value(&path(".spread"), NativeValue::Float(1.5))
        .is_err());
    assert!(native
        .with_value(&path(".algorithm"), NativeValue::Int(2))
        .is_err());
    assert!(native
        .with_value(&path("points[0].z"), NativeValue::Float(0.5))
        .is_err());
    assert!(native
        .with_value(&path(".point_count"), NativeValue::Int(0))
        .is_err());
    assert!(native
        .with_value(&path("points[0].x"), NativeValue::Float(2e6))
        .is_err());
}

#[test]
fn a_smart_material_refuses_point_gradients() {
    let (mut doc, fill) = doc_with_fill();
    doc.set_fill_points(fill, Channel::Color, Some(uv_points()), false)
        .unwrap();
    let material = doc.capture_smart_material(&[fill], "素材").unwrap();
    let writer = yolu_io::WriterInfo {
        app: "試験".into(),
        version: "1".into(),
        unity: "なし".into(),
    };
    let err = yolu_io::smart::SmartFile::from_core(&material, &writer)
        .unwrap_err()
        .to_string();
    assert!(err.contains(REFUSAL_POINT_GRADIENTS), "{err}");
}
