//! 0.5.0 のフィルターの段（種類 70〜79）と Generator（66 模様・67 アイランドごとのばらつき・68 ライト・69 マスクの組み立て）の保存・復元（正本の版 28）。目録の全部の
//! 種類を既定値と既定でない値で往復させ、版の選び方、古い読み手（スタンドアロン 0.4.x・Unity 0.2.0）が版の数で断ること、欄の範囲・
//! チャンネル・到達半径の検査、.ylsmart の断りを試す。
use std::collections::BTreeMap;

use yolu_core::effects::catalog;
use yolu_core::filter::{MorphologyMode, Settings as F, SlopeMode};
use yolu_core::{Channel, Document, EffectSettings, FilterSpec, FilterTarget, LayerId};
use yolu_io::smart::{SmartFile, REFUSAL_NEW_FILTERS};
use yolu_io::{
    NativeDocument, NativeValue, WriterInfo, ADJUST_VERSION, EFFECTS_VERSION, MAX_NATIVE_VERSION,
    MIXING_VERSION, SPLIT_VERSION, UNITY_NATIVE_VERSION,
};

/// 版 28 で足した種類の目録の名前。
const KINDS: [&str; 10] = [
    "histogram_scan",
    "histogram_range",
    "slope_blur",
    "directional_blur",
    "warp",
    "morphology",
    "edge_detect",
    "high_pass",
    "median",
    "glow",
];

/// スカラーとマスクだけの種類。
const SCALAR_ONLY: [&str; 4] = [
    "histogram_scan",
    "histogram_range",
    "morphology",
    "edge_detect",
];

fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.1".into(),
        unity: "none".into(),
    }
}

fn plain() -> (Document, LayerId) {
    let mut doc = Document::with_tile_size(16, 16, 8).unwrap();
    let id = doc.add_layer("a").unwrap();
    (doc, id)
}

fn defaults(kind: &str) -> EffectSettings {
    EffectSettings::from_catalog(kind, &BTreeMap::new()).unwrap()
}

/// 既定でない値（端・細かい小数・負のシード）。
fn odd(kind: &str) -> EffectSettings {
    EffectSettings::Filter(match kind {
        "histogram_scan" => F::HistogramScan {
            position: 0.123456789,
            contrast: 1.0,
        },
        "histogram_range" => F::HistogramRange {
            range: 0.0,
            position: 0.987654321,
        },
        "slope_blur" => F::SlopeBlur {
            intensity: 63.75,
            samples: 32,
            mode: SlopeMode::Max,
            scale: 1.0,
            seed: i32::MIN,
        },
        "directional_blur" => F::DirectionalBlur {
            angle: 359.5,
            distance: 0.25,
        },
        "warp" => F::Warp {
            intensity: 0.0,
            scale: 256.0,
            seed: -7,
        },
        "morphology" => F::Morphology {
            mode: MorphologyMode::Erode,
            radius: 64,
        },
        "edge_detect" => F::EdgeDetect {
            width: 16,
            threshold: 1.0,
        },
        "high_pass" => F::HighPass { radius: 256 },
        "median" => F::Median { radius: 16 },
        _ => F::Glow {
            threshold: 0.0,
            radius: 1,
            intensity: 4.0,
        },
    })
}

/// 種類をレイヤーの内容（色の種類は Color、スカラーだけの種類は Roughness）と、置けるならマスクに 1 つずつ置いた文書。
fn document_with(kind: &str, settings: &EffectSettings) -> Document {
    let (mut doc, id) = plain();
    let channel = if SCALAR_ONLY.contains(&kind) {
        Channel::Roughness
    } else {
        Channel::Color
    };
    doc.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(settings.clone()).channels(&[channel]),
    )
    .unwrap();
    if kind != "glow" {
        doc.add_layer_mask(id).unwrap();
        doc.add_filter(id, FilterTarget::Mask, FilterSpec::new(settings.clone()))
            .unwrap();
    }
    doc
}

/// 文書の全部の段（レイヤーの内容とマスク）の設定・有効・強さ・チャンネル。
fn stages(doc: &Document) -> Vec<(EffectSettings, bool, f64, Vec<Channel>)> {
    doc.layers()
        .iter()
        .flat_map(|l| {
            l.filters()
                .iter()
                .chain(l.mask().into_iter().flat_map(|m| m.filters().iter()))
        })
        .map(|e| {
            (
                e.settings().clone(),
                e.enabled(),
                e.strength(),
                e.channels().to_vec(),
            )
        })
        .collect()
}

fn field_paths(native: &NativeDocument, tail: &str) -> Vec<String> {
    native
        .fields()
        .iter()
        .map(|f| f.path.clone())
        .filter(|p| p.ends_with(tail))
        .collect()
}

#[test]
fn the_catalog_has_exactly_the_new_kinds_with_their_numbers() {
    for (k, kind) in KINDS.iter().enumerate() {
        let entry = catalog::kind(kind).unwrap_or_else(|| panic!("{kind} が目録に無い"));
        assert!(entry.stack && entry.addable && entry.rust_only, "{kind}");
        assert_eq!(defaults(kind).type_index(), 70 + k as i32, "{kind}");
    }
}

#[test]
fn every_new_kind_round_trips_at_version_28_with_default_and_odd_values() {
    for kind in KINDS {
        for settings in [defaults(kind), odd(kind)] {
            let doc = document_with(kind, &settings);
            let native = NativeDocument::from_core(&doc).unwrap();
            assert_eq!(native.version(), EFFECTS_VERSION, "{kind}");
            let back = native.to_core().unwrap();
            assert_eq!(stages(&back), stages(&doc), "{kind}: 往復で変わった");
            // 正本 → core → 正本、バイト列 → 正本も同じバイト
            let again = NativeDocument::from_core(&back).unwrap();
            assert_eq!(again.to_bytes(), native.to_bytes(), "{kind}");
            let reread = NativeDocument::read(&native.to_bytes()).unwrap();
            assert_eq!(reread.to_bytes(), native.to_bytes(), "{kind}");
            assert_eq!(reread.version(), EFFECTS_VERSION);
        }
    }
    // 無効にした段・強さも残り、無効でも版 28（段が残っている限り）
    let (mut doc, id) = plain();
    let stage = doc
        .add_filter(
            id,
            FilterTarget::Content,
            FilterSpec::new(defaults("median"))
                .channels(&[Channel::Color, Channel::Height])
                .strength(0.375)
                .disabled(),
        )
        .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), EFFECTS_VERSION);
    let back = native.to_core().unwrap();
    let (_, e, _) = back.find_filter(stage).unwrap();
    assert!(!e.enabled());
    assert_eq!(e.strength(), 0.375);
    assert_eq!(e.channels(), &[Channel::Color, Channel::Height]);
}

#[test]
fn documents_without_the_new_kinds_keep_their_version() {
    assert_eq!(EFFECTS_VERSION, 28);
    // 読める一番新しい版はベイクの優先の版 33（版 28 の中身も読み書きできる）
    assert_eq!(MAX_NATIVE_VERSION, yolu_io::RULERS_VERSION);
    let (mut doc, id) = plain();
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().version(),
        UNITY_NATIVE_VERSION
    );
    doc.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(3)).channels(&[Channel::Color]),
    )
    .unwrap();
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().version(),
        UNITY_NATIVE_VERSION
    );
    let adjust = EffectSettings::from_catalog("posterize", &BTreeMap::new()).unwrap();
    doc.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(adjust).channels(&[Channel::Color]),
    )
    .unwrap();
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().version(),
        ADJUST_VERSION
    );
    // 新しい種類を消すと版が下がる
    let mut with = document_with("warp", &defaults("warp"));
    assert_eq!(
        NativeDocument::from_core(&with).unwrap().version(),
        EFFECTS_VERSION
    );
    let id = with.layers()[0].id();
    let content = with.layers()[0].filters()[0].id();
    let mask = with.layers()[0].mask().unwrap().filters()[0].id();
    with.remove_filter(id, content).unwrap();
    assert_eq!(
        NativeDocument::from_core(&with).unwrap().version(),
        EFFECTS_VERSION,
        "マスクの段も数える"
    );
    with.remove_filter(id, mask).unwrap();
    assert_eq!(
        NativeDocument::from_core(&with).unwrap().version(),
        UNITY_NATIVE_VERSION
    );
}

#[test]
fn older_readers_refuse_version_28_by_the_number() {
    // スタンドアロン 0.4.x の読み手は 1〜25 と分けた正本の 26 だけを読む（`.version の値 28 は未対応または範囲外です (1..25)`）、
    // Unity 0.2.0 は 1〜21 だけ。版 28 はどちらの範囲の外で、分けた正本の識別の 26 とも重ならない
    const MAX_04X: i32 = 25;
    assert_eq!(MIXING_VERSION, MAX_04X);
    const { assert!(EFFECTS_VERSION > MAX_04X && EFFECTS_VERSION != SPLIT_VERSION) };
    let native = NativeDocument::from_core(&document_with("glow", &defaults("glow"))).unwrap();
    let bytes = native.to_bytes();
    assert_eq!(&bytes[..8], b"DOTPAINT");
    assert_eq!(bytes[8..12], EFFECTS_VERSION.to_le_bytes());
    // 版の数だけを 25 に戻した正本は、この読み手も断る（版 25 に種類 70〜79 は無い）
    let mut older = bytes.clone();
    older[8..12].copy_from_slice(&MIXING_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&older).is_err());
    // 版 25 の正本の段を種類 70 以上に変えても断る
    let (mut doc, id) = plain();
    doc.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::invert()).channels(&[Channel::Color]),
    )
    .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    let path = field_paths(&native, ".type")
        .into_iter()
        .find(|p| p.contains("filters.items[0]"))
        .unwrap();
    for kind in 70..=79 {
        assert!(native.with_value(&path, NativeValue::Int(kind)).is_err());
    }
}

#[test]
fn versions_without_a_meaning_are_refused_by_the_number() {
    // 読める版は 1〜25・26（分けた正本）・27（パスの一覧）・28・29（点のグラデーション）・30（テキストレイヤー）・32・33。間の 31 は番号だけで並びの定義が無い。読める範囲（1〜33）の中にあるので、
    // 断らないと版 25 の並びとして読み進める（色調補正のフィルターだけの文書は、版 24 の並びが版 25〜28 と同じなので、版の数だけを書き換えても中身は読める）
    let (mut doc, id) = plain();
    let posterize = EffectSettings::from_catalog("posterize", &BTreeMap::new()).unwrap();
    doc.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(posterize).channels(&[Channel::Color]),
    )
    .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), ADJUST_VERSION);
    let bytes = native.to_bytes();
    let numbered = |n: i32| {
        let mut b = bytes.clone();
        b[8..12].copy_from_slice(&n.to_le_bytes());
        b
    };
    // 意味の決まった版の数なら読める（対照。決まっていない版だけを断っていること）
    for n in [
        ADJUST_VERSION,
        MIXING_VERSION,
        yolu_io::PATHS_VERSION,
        EFFECTS_VERSION,
        yolu_io::POINT_GRADIENT_VERSION,
        yolu_io::TEXT_VERSION,
    ] {
        assert_eq!(NativeDocument::read(&numbered(n)).unwrap().version(), n);
    }
    // 意味の決まっていない版（31）は断る
    let n = 31;
    let e = NativeDocument::read(&numbered(n))
        .err()
        .map(|e| e.to_string());
    assert!(
        e.is_some_and(|e| e.contains(&n.to_string())),
        "版 {n} を断らなかった"
    );
    // 範囲の外
    for n in [0, -1, yolu_io::SEAMS_VERSION + 1, i32::MAX] {
        assert!(NativeDocument::read(&numbered(n)).is_err(), "版 {n}");
    }
    // 分けた正本の中の版（ヘッダーの中の版）は `bigdoc_tests` の `wrong_parts_are_refused` が断ることを確かめる
}

#[test]
fn the_reader_refuses_out_of_range_fields_wrong_channels_and_too_long_reach() {
    // 範囲の外の欄
    for (kind, tail, value, ok) in [
        (
            "histogram_scan",
            "effect.position",
            NativeValue::Float(1.5),
            false,
        ),
        (
            "histogram_scan",
            "effect.contrast",
            NativeValue::Float(1.0),
            true,
        ),
        (
            "histogram_range",
            "effect.range",
            NativeValue::Float(-0.1),
            false,
        ),
        (
            "slope_blur",
            "effect.intensity",
            NativeValue::Float(64.5),
            false,
        ),
        ("slope_blur", "effect.samples", NativeValue::Int(0), false),
        ("slope_blur", "effect.mode", NativeValue::Int(3), false),
        ("slope_blur", "effect.scale", NativeValue::Float(0.5), false),
        (
            "directional_blur",
            "effect.angle",
            NativeValue::Float(360.5),
            false,
        ),
        (
            "directional_blur",
            "effect.distance",
            NativeValue::Float(f64::NAN),
            false,
        ),
        ("warp", "effect.intensity", NativeValue::Float(128.0), true),
        ("warp", "effect.intensity", NativeValue::Float(129.0), false),
        ("morphology", "effect.mode", NativeValue::Int(2), false),
        ("morphology", "effect.radius", NativeValue::Int(65), false),
        ("edge_detect", "effect.width", NativeValue::Int(0), false),
        ("high_pass", "effect.radius", NativeValue::Int(257), false),
        ("median", "effect.radius", NativeValue::Int(17), false),
        ("glow", "effect.intensity", NativeValue::Float(4.5), false),
        ("glow", "effect.threshold", NativeValue::Float(0.0), true),
    ] {
        let native = NativeDocument::from_core(&document_with(kind, &defaults(kind))).unwrap();
        let path = field_paths(&native, tail).remove(0);
        assert_eq!(
            native.with_value(&path, value.clone()).is_ok(),
            ok,
            "{kind} {tail} {value:?}"
        );
        // 共通の欄（半径など）は既定のまま
        let radius = field_paths(&native, "filters.items[0].radius").remove(0);
        assert!(
            native.with_value(&radius, NativeValue::Int(3)).is_err(),
            "{kind}"
        );
    }
    // チャンネル: スカラーだけの種類は色のチャンネルに、グローはスカラーのチャンネルに置けない。どれも Normal に置けない
    for kind in KINDS {
        let native = NativeDocument::from_core(&document_with(kind, &defaults(kind))).unwrap();
        let path = field_paths(&native, "channels[0].channel")
            .into_iter()
            .find(|p| p.contains("filters"))
            .unwrap();
        assert_eq!(
            native.with_value(&path, NativeValue::Int(3)).is_ok(),
            kind != "glow",
            "{kind}: Height"
        );
        assert_eq!(
            native.with_value(&path, NativeValue::Int(5)).is_ok(),
            !SCALAR_ONLY.contains(&kind),
            "{kind}: Emission"
        );
        assert!(
            native.with_value(&path, NativeValue::Int(4)).is_err(),
            "{kind}: Normal"
        );
    }
    // マスクにグローは置けない（マスクの段の種類を 79 に変えると断る）
    let native = NativeDocument::from_core(&document_with("median", &defaults("median"))).unwrap();
    let mask_type = field_paths(&native, ".type")
        .into_iter()
        .find(|p| p.contains("mask.filters"))
        .unwrap();
    assert!(native.with_value(&mask_type, NativeValue::Int(79)).is_err());
    // 到達半径の合計は 512 まで（方向のぼかし 255.5 → 256 + ハイパス 255 + メディアン 1 は通り、メディアンを 2 にすると断る）
    let (mut doc, id) = plain();
    for s in [
        EffectSettings::Filter(F::DirectionalBlur {
            angle: 0.0,
            distance: 255.5,
        }),
        EffectSettings::Filter(F::HighPass { radius: 255 }),
        EffectSettings::Filter(F::Median { radius: 1 }),
    ] {
        doc.add_filter(
            id,
            FilterTarget::Content,
            FilterSpec::new(s).channels(&[Channel::Color]),
        )
        .unwrap();
    }
    let native = NativeDocument::from_core(&doc).unwrap();
    let median = field_paths(&native, "effect.radius")
        .into_iter()
        .find(|p| p.contains("items[2]"))
        .unwrap();
    assert!(native.with_value(&median, NativeValue::Int(1)).is_ok());
    assert!(native.with_value(&median, NativeValue::Int(2)).is_err());
    // 無効の段は数えない
    let enabled = field_paths(&native, "items[2].enabled").remove(0);
    let off = native
        .with_value(&enabled, NativeValue::Bool(false))
        .unwrap();
    assert!(off.with_value(&median, NativeValue::Int(2)).is_ok());
}

#[test]
fn smart_files_do_not_carry_the_new_kinds() {
    for kind in KINDS {
        let doc = document_with(kind, &defaults(kind));
        let id = doc.layers()[0].id();
        let material = doc.capture_smart_material(&[id], "素材").unwrap();
        let err = SmartFile::from_core(&material, &writer()).unwrap_err();
        assert!(
            err.to_string().contains(REFUSAL_NEW_FILTERS),
            "{kind}: {err}"
        );
    }
}

/// 版 28 の Generator（模様・ライト・マスクの組み立て・アイランドごとのばらつき）。既定でない値で 1 つずつ。
fn generators() -> Vec<(&'static str, yolu_core::generator::Settings)> {
    use yolu_core::generator::{
        IslandVariation, Kind, Light, MaskCombine, MaskInput, Pattern, PatternShape, Settings,
    };
    let mut pattern = Settings::new(Kind::Pattern);
    pattern.low = 0.125;
    pattern.invert = true;
    pattern.pattern = Pattern {
        shape: PatternShape::Grid,
        scale: 511.5,
        angle: 12.345,
        width: 0.0625,
        softness: 1.0,
        offset: [0.3, 1.0],
    };
    let mut light = Settings::new(Kind::Light);
    light.light = Light {
        azimuth: 359.9,
        elevation: 0.0,
        softness: 0.75,
        ambient: 0.1,
    };
    let mut mask = Settings::new(Kind::MaskBuilder);
    mask.softness = 0.4;
    mask.mask_builder.combine = MaskCombine::Add;
    mask.mask_builder.inputs = [
        MaskInput {
            weight: 0.9,
            level: 0.1,
            contrast: 0.2,
            invert: true,
        },
        MaskInput {
            weight: 0.0,
            level: 0.7,
            contrast: 1.0,
            invert: false,
        },
        MaskInput {
            weight: 0.5,
            level: 0.5,
            contrast: 0.0,
            invert: false,
        },
        MaskInput {
            weight: 1.0,
            level: 0.0,
            contrast: 0.5,
            invert: true,
        },
    ];
    let mut island = Settings::new(Kind::UvIslandVariation);
    island.softness = 0.35;
    island.high = 0.875;
    island.island = IslandVariation {
        seed: i32::MIN,
        min: 0.123456789,
        max: 0.123456789,
    };
    vec![
        ("pattern", pattern),
        ("light", light),
        ("mask_builder", mask),
        ("uv_island_variation", island),
    ]
}

#[test]
fn the_new_generators_round_trip_at_version_28() {
    for (name, g) in generators() {
        for settings in [
            EffectSettings::from_catalog(name, &BTreeMap::new()).unwrap(),
            EffectSettings::generator(g.clone()),
        ] {
            let (mut doc, id) = plain();
            doc.add_filter(
                id,
                FilterTarget::Content,
                FilterSpec::new(settings.clone()).channels(&[Channel::Color]),
            )
            .unwrap();
            doc.add_layer_mask(id).unwrap();
            doc.add_filter(id, FilterTarget::Mask, FilterSpec::new(settings.clone()))
                .unwrap();
            let native = NativeDocument::from_core(&doc).unwrap();
            assert_eq!(native.version(), EFFECTS_VERSION, "{name}");
            let back = native.to_core().unwrap();
            assert_eq!(stages(&back), stages(&doc), "{name}");
            assert_eq!(
                NativeDocument::from_core(&back).unwrap().to_bytes(),
                native.to_bytes(),
                "{name}"
            );
        }
    }
}

#[test]
fn the_reader_refuses_swapped_kinds_wrong_pins_and_out_of_range_generator_fields() {
    for (name, g) in generators() {
        let own = g.kind as i32;
        let (mut doc, id) = plain();
        doc.add_filter(
            id,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Color]),
        )
        .unwrap();
        let native = NativeDocument::from_core(&doc).unwrap();
        let kind = field_paths(&native, "generator.type").remove(0);
        // 種類を入れ替えると欄の並びが合わない
        for other in [66, 67, 68, 69, 70, 64, 0] {
            if other == own {
                continue;
            }
            assert!(
                native.with_value(&kind, NativeValue::Int(other)).is_err(),
                "{name} → {other}"
            );
        }
        let (tail, bad) = match name {
            "pattern" => ("effect.scale", NativeValue::Float(512.5)),
            "light" => ("effect.elevation", NativeValue::Float(90.5)),
            "uv_island_variation" => ("effect.max", NativeValue::Float(1.5)),
            _ => ("effect.thickness.weight", NativeValue::Float(1.5)),
        };
        let path = field_paths(&native, tail).remove(0);
        assert!(native.with_value(&path, bad).is_err(), "{name} {tail}");
    }
    // アイランドごとのばらつき: 最小が最大を超えるのは断る（等しいのは読める）。シードは i32 の全域
    let (_, island) = generators().pop().unwrap();
    let (mut doc, id) = plain();
    doc.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(island)).channels(&[Channel::Color]),
    )
    .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    let min = field_paths(&native, "effect.min").remove(0);
    assert!(native.with_value(&min, NativeValue::Float(0.5)).is_err());
    assert!(native.with_value(&min, NativeValue::Float(0.0)).is_ok());
    let seed = field_paths(&native, "effect.seed").remove(0);
    let back = native
        .with_value(&seed, NativeValue::Int(i32::MAX))
        .unwrap()
        .to_core()
        .unwrap();
    assert_eq!(
        stages(&back)[0].0.generator_settings().unwrap().island.seed,
        i32::MAX
    );

    // 版 25 の正本の Generator を 66 にしても断る
    let (mut doc, id) = plain();
    doc.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(
            yolu_core::generator::Settings::new(yolu_core::generator::Kind::Thickness),
        ))
        .channels(&[Channel::Color]),
    )
    .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), UNITY_NATIVE_VERSION);
    let kind = field_paths(&native, "generator.type").remove(0);
    for other in [66, 68, 69] {
        assert!(native.with_value(&kind, NativeValue::Int(other)).is_err());
    }
}

#[test]
fn smart_files_do_not_carry_the_new_generators() {
    for (name, g) in generators() {
        let (mut doc, id) = plain();
        doc.add_filter(
            id,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Color]),
        )
        .unwrap();
        let material = doc.capture_smart_material(&[id], "素材").unwrap();
        let err = SmartFile::from_core(&material, &writer()).unwrap_err();
        assert!(
            err.to_string().contains(REFUSAL_NEW_FILTERS),
            "{name}: {err}"
        );
    }
}
