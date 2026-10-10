//! Rust 版だけの色調補正（調整レイヤーとフィルターの段の種類 64〜69: グラデーションマップ・トーンカーブ・カラーバランス・明るさ/コントラスト・
//! 2 値化・ポスタリゼーション）の保存・復元（正本の版 24）。C# に対応する書き手は無いので、正解ファイルは無く、往復・版の選び方・読み手の拒否・
//! .ylp への保存・.ylsmart の断り・Unity 0.2.0 の読み手の記録を試す。
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::generator::{ColorStop, OpacityStop, Ramp};
use yolu_core::{
    AdjustmentSettings, BrightnessContrast, Channel, ChannelInfo, ChannelKind, ColorBalance,
    ColorSpace, Document, EffectSettings, FilterId, FilterSpec, FilterTarget, GradientMap, LayerId,
    Posterize, Rect, Rgba8, Threshold, ToneChannel, ToneCurves,
};
use yolu_io::smart::{SmartFile, REFUSAL_RUST_ADJUSTMENTS};
use yolu_io::{
    NativeDocument, NativeValue, Project, SaveTarget, SetSpec, WriterInfo, ADJUST_VERSION,
    MAX_NATIVE_VERSION, PROCEDURAL_VERSION, UNITY_NATIVE_VERSION, USER_CHANNELS_VERSION,
};

fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.1".into(),
        unity: "none".into(),
    }
}

fn points(list: &[(f64, f64)]) -> Curve {
    Curve::new(list.iter().map(|&(x, y)| CurvePoint { x, y }).collect()).unwrap()
}

/// 3 色・3 不透明度・中点・値のカーブ（直線でない）を持つランプ。
fn ramp() -> Ramp {
    let stop = |position, rgb: [u8; 3], midpoint| ColorStop {
        position,
        color: Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
        midpoint,
    };
    let opacity = |position, opacity, midpoint| OpacityStop {
        position,
        opacity,
        midpoint,
    };
    Ramp::new(
        vec![
            stop(0.0, [10, 20, 90], 0.4),
            stop(0.3515625, [200, 60, 30], 0.62),
            stop(1.0, [250, 240, 200], 0.5),
        ],
        vec![
            opacity(0.0, 1.0, 0.45),
            opacity(0.5, 0.8, 0.55),
            opacity(1.0, 0.2, 0.5),
        ],
        Some(
            points(&[(0.0, 0.05), (0.4, 0.55), (1.0, 0.95)])
                .points()
                .to_vec(),
        ),
    )
    .unwrap()
}

/// 6 種の調整（名前つき）。値は細かい小数・端の値を含む。
fn adjustments() -> Vec<(&'static str, AdjustmentSettings)> {
    vec![
        (
            "グラデーションマップ",
            AdjustmentSettings::gradient_map(GradientMap::new(ramp(), true)),
        ),
        (
            "トーンカーブ",
            AdjustmentSettings::tone_curve(ToneCurves::new(
                points(&[(0.0, 0.0), (0.26, 0.31), (0.7, 0.62), (1.0, 1.0)]),
                points(&[(0.0, 0.1), (1.0, 0.9)]),
                Curve::identity(),
                points(&[(0.0, 0.0), (0.5, 0.65), (1.0, 1.0)]),
            )),
        ),
        (
            "カラーバランス",
            AdjustmentSettings::color_balance(
                ColorBalance::new(
                    [-100.0, 12.5, 3.0],
                    [0.0, -40.25, 100.0],
                    [20.0, 0.0, -7.75],
                    false,
                )
                .unwrap(),
            ),
        ),
        (
            "明るさ・コントラスト",
            AdjustmentSettings::brightness_contrast(BrightnessContrast::new(-37.5, 81.25).unwrap()),
        ),
        (
            "2 値化",
            AdjustmentSettings::threshold(Threshold::new(77).unwrap()),
        ),
        (
            "ポスタリゼーション",
            AdjustmentSettings::posterize(Posterize::new(7).unwrap()),
        ),
    ]
}

fn effect(s: &AdjustmentSettings) -> EffectSettings {
    EffectSettings::from_color_adjust(s.color_adjust().unwrap())
}

/// 40×28 の文書: 下地の絵・6 種の調整レイヤー（名前・不透明度・合成モードつき）・塗りつぶしレイヤーの内容に 6 種のフィルターの段
/// （色のチャンネル）とマスクに明るさ/コントラストの段。固定の ID（正解の正本が毎回同じバイト列になる）。
fn document() -> Document {
    let mut doc = Document::with_tile_size(40, 28, 8).unwrap();
    let base = doc.add_layer("下地").unwrap();
    for y in 0..28u32 {
        for x in 0..40u32 {
            let v = ((x * 13 + y * 29) % 256) as u8;
            doc.set_pixel(base, x, y, Rgba8::new(v, 255 - v, (x * 6) as u8, 255))
                .unwrap();
        }
    }
    for (i, (name, s)) in adjustments().into_iter().enumerate() {
        let id = doc.add_adjustment_layer(name, s, None, None).unwrap();
        doc.set_layer_opacity(id, 1.0 - 0.1 * i as f64, false)
            .unwrap();
    }
    let fill = doc
        .add_fill_layer(
            "塗り",
            &[(Channel::Color, Rgba8::new(180, 90, 40, 255))],
            None,
        )
        .unwrap();
    for (i, (_, s)) in adjustments().into_iter().enumerate() {
        doc.add_filter(
            fill,
            FilterTarget::Content,
            FilterSpec::new(effect(&s))
                .channels(&[Channel::Color])
                .strength(0.5 + 0.0625 * i as f64)
                .with_id(FilterId(0xA000 + i as u128)),
        )
        .unwrap();
    }
    doc.add_layer_mask(fill).unwrap();
    doc.add_filter(
        fill,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::brightness_contrast(
            BrightnessContrast::new(20.0, 10.0).unwrap(),
        ))
        .with_id(FilterId(0xA100)),
    )
    .unwrap();
    let ids: Vec<LayerId> = (0..doc.layers().len())
        .map(|i| LayerId(0x8000 + i as u128))
        .collect();
    doc.with_persistent_ids(0x8888, &ids).unwrap()
}

fn plain() -> Document {
    let mut doc = Document::with_tile_size(40, 28, 8).unwrap();
    doc.add_fill_layer("塗り", &[(Channel::Color, Rgba8::new(1, 2, 3, 255))], None)
        .unwrap();
    doc
}
fn user_channel(doc: &mut Document) {
    doc.add_channel(ChannelInfo {
        name: "傷".into(),
        kind: ChannelKind::Scalar,
        color_space: ColorSpace::Linear,
        default: Rgba8::new(0, 0, 0, 255),
    })
    .unwrap();
}
/// 調整レイヤーの設定と、フィルターの段の設定を、レイヤーの順に集める（往復で変わらないことを見る）。
fn settings(doc: &Document) -> (Vec<Option<AdjustmentSettings>>, Vec<(bool, EffectSettings)>) {
    let mut stages = Vec::new();
    for l in doc.layers() {
        for e in l.filters() {
            stages.push((false, e.settings().clone()));
        }
        for e in l.mask().into_iter().flat_map(|m| m.filters()) {
            stages.push((true, e.settings().clone()));
        }
    }
    (
        doc.layers()
            .iter()
            .map(|l| l.adjustment().cloned())
            .collect(),
        stages,
    )
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
fn the_version_follows_the_features_used() {
    assert_eq!(UNITY_NATIVE_VERSION, 21);
    assert_eq!(USER_CHANNELS_VERSION, 22);
    assert_eq!(PROCEDURAL_VERSION, 23);
    assert_eq!(ADJUST_VERSION, 24);
    // 版 25 は gradient_mixing_bridge、版 28 は image_generator と filters_v28、版 32 は seams_bridge、版 33 はベイクの優先の試験が固定する
    assert_eq!(yolu_io::EFFECTS_VERSION, 28);
    assert_eq!(MAX_NATIVE_VERSION, yolu_io::RULERS_VERSION);
    // 使わない文書の版は変わらない: 今の 3 種の調整・フィルターだけなら Unity 版と同じ 21
    let mut old = plain();
    for s in [
        AdjustmentSettings::invert(),
        AdjustmentSettings::levels(0.1, 0.9, 1.5, 0.0, 1.0).unwrap(),
        AdjustmentSettings::hue_saturation(30.0, 0.1, 0.0).unwrap(),
    ] {
        old.add_adjustment_layer("調整", s, None, None).unwrap();
    }
    let id = old.layers()[0].id();
    old.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::invert()).channels(&[Channel::Color]),
    )
    .unwrap();
    assert_eq!(
        NativeDocument::from_core(&old).unwrap().version(),
        UNITY_NATIVE_VERSION
    );
    // ユーザーチャンネルだけなら 22
    let mut with_user = plain();
    user_channel(&mut with_user);
    assert_eq!(
        NativeDocument::from_core(&with_user).unwrap().version(),
        USER_CHANNELS_VERSION
    );
    // 6 種のどれか 1 つでも、調整レイヤーでもフィルターの段でも（マスクの段でも）あれば 24
    for (name, s) in adjustments() {
        let mut layered = plain();
        layered
            .add_adjustment_layer(name, s.clone(), None, None)
            .unwrap();
        assert_eq!(
            NativeDocument::from_core(&layered).unwrap().version(),
            ADJUST_VERSION,
            "{name}: 調整レイヤー"
        );
        let mut staged = plain();
        let id = staged.layers()[0].id();
        staged
            .add_filter(
                id,
                FilterTarget::Content,
                FilterSpec::new(effect(&s)).channels(&[Channel::Color]),
            )
            .unwrap();
        assert_eq!(
            NativeDocument::from_core(&staged).unwrap().version(),
            ADJUST_VERSION,
            "{name}: フィルターの段"
        );
        // 無効にした段も数える（段が残っている限り、Unity 版が読めない版になる）
        let stage = staged.layers()[0].filters()[0].id();
        staged.set_filter_enabled(id, stage, false).unwrap();
        assert_eq!(
            NativeDocument::from_core(&staged).unwrap().version(),
            ADJUST_VERSION
        );
    }
    let mut masked = plain();
    let id = masked.layers()[0].id();
    masked.add_layer_mask(id).unwrap();
    masked
        .add_filter(
            id,
            FilterTarget::Mask,
            FilterSpec::new(EffectSettings::threshold(Threshold::new(9).unwrap())),
        )
        .unwrap();
    assert_eq!(
        NativeDocument::from_core(&masked).unwrap().version(),
        ADJUST_VERSION
    );
    // ユーザーチャンネルがあっても 24 で、ユーザーチャンネルも残る
    let mut both = document();
    user_channel(&mut both);
    let native = NativeDocument::from_core(&both).unwrap();
    assert_eq!(native.version(), ADJUST_VERSION);
    assert_eq!(native.to_core().unwrap().channels().len(), 7);
}

#[test]
fn every_setting_survives_the_native_round_trip_byte_for_byte() {
    let doc = document();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), ADJUST_VERSION);
    let back = native.to_core().unwrap();
    assert_eq!(settings(&back), settings(&doc), "設定が往復で変わった");
    // 書き直しても同じバイト（正本 → core → 正本）
    let again = NativeDocument::from_core(&back).unwrap();
    assert_eq!(again.to_bytes(), native.to_bytes());
    // 読み直し（バイト列 → 正本）も同じ
    let reread = NativeDocument::read(&native.to_bytes()).unwrap();
    assert_eq!(reread.to_bytes(), native.to_bytes());
    assert_eq!(reread.version(), ADJUST_VERSION);
    // レイヤーの不透明度・段の強さ・有効・ID も残る
    let mut edited = document();
    let fill = edited.layers().last().unwrap().id();
    let stage = edited.layers().last().unwrap().filters()[2].id();
    edited.set_filter_enabled(fill, stage, false).unwrap();
    edited
        .set_filter_strength(fill, stage, 0.375, false)
        .unwrap();
    let back = NativeDocument::from_core(&edited)
        .unwrap()
        .to_core()
        .unwrap();
    let e = &back.layers().last().unwrap().filters()[2];
    assert!(!e.enabled());
    assert_eq!(e.strength(), 0.375);
    assert_eq!(e.id(), stage);
    assert_eq!(back.layers()[2].opacity(), edited.layers()[2].opacity());
}

#[test]
fn composites_are_identical_after_a_round_trip_in_every_channel() {
    let doc = document();
    let back = NativeDocument::from_core(&doc).unwrap().to_core().unwrap();
    let all = Rect::new(0, 0, 40, 28);
    for c in doc.channels() {
        assert_eq!(
            doc.composite_channel(c, all).unwrap(),
            back.composite_channel(c, all).unwrap(),
            "{c:?}"
        );
    }
    // 効果が入っている（調整レイヤーと段を外した文書と違う）
    let mut bare = document();
    let fill = bare.layers().last().unwrap().id();
    let ids: Vec<_> = bare
        .layers()
        .last()
        .unwrap()
        .filters()
        .iter()
        .map(|e| e.id())
        .collect();
    for id in ids {
        bare.remove_filter(fill, id).unwrap();
    }
    assert_ne!(
        doc.composite_channel(Channel::Color, all).unwrap(),
        bare.composite_channel(Channel::Color, all).unwrap()
    );
}

#[test]
fn the_reader_refuses_what_the_format_does_not_allow() {
    let native = NativeDocument::from_core(&document()).unwrap();
    // 調整レイヤーの欄: 種類を入れ替える・空けてある番号・知らない番号は断る（欄の並びが合わなくなる）
    let adjustment_types = field_paths(&native, "adjustment.type");
    assert_eq!(adjustment_types.len(), 6);
    for path in &adjustment_types {
        for kind in [3, 4, 30, 63, 70, 100, -1] {
            assert!(
                native.with_value(path, NativeValue::Int(kind)).is_err(),
                "{path} 種類 {kind}"
            );
        }
        for kind in [0, 1, 2] {
            assert!(
                native.with_value(path, NativeValue::Int(kind)).is_err(),
                "{path} 種類 {kind}: 64 からの欄を持つので古い種類に変えられない"
            );
        }
    }
    // 64 の欄を別の 64 からの種類へ変えても、並びが合わない
    assert!(native
        .with_value(&adjustment_types[0], NativeValue::Int(65))
        .is_err());
    let one = |tail: &str, nth: usize| field_paths(&native, tail).remove(nth);
    let edit = |path: &str, v: NativeValue| native.with_value(path, v);
    // 範囲外の欄（調整レイヤー）
    for (tail, v) in [
        ("adjustment.detail.level", NativeValue::Int(0)),
        ("adjustment.detail.level", NativeValue::Int(256)),
        ("adjustment.detail.levels", NativeValue::Int(1)),
        ("adjustment.detail.levels", NativeValue::Int(256)),
        ("adjustment.detail.brightness", NativeValue::Float(150.5)),
        ("adjustment.detail.brightness", NativeValue::Float(-150.5)),
        ("adjustment.detail.contrast", NativeValue::Float(100.5)),
        ("adjustment.detail.contrast", NativeValue::Float(-50.5)),
        ("adjustment.detail.contrast", NativeValue::Float(f64::NAN)),
        (
            "adjustment.detail.shadows_cyan_red",
            NativeValue::Float(100.5),
        ),
        (
            "adjustment.detail.midtones_yellow_blue",
            NativeValue::Float(-101.0),
        ),
        (
            "adjustment.detail.highlights_magenta_green",
            NativeValue::Float(f64::INFINITY),
        ),
        ("adjustment.detail.ramp.colors_count", NativeValue::Int(1)),
        ("adjustment.detail.ramp.colors_count", NativeValue::Int(33)),
        (
            "adjustment.detail.ramp.opacities_count",
            NativeValue::Int(1),
        ),
        ("adjustment.detail.ramp.curve_count", NativeValue::Int(17)),
        ("adjustment.detail.composite_count", NativeValue::Int(1)),
        ("adjustment.detail.red_count", NativeValue::Int(17)),
    ] {
        assert!(edit(&one(tail, 0), v.clone()).is_err(), "{tail} {v:?}");
    }
    // 範囲内の変更は通る
    for (tail, v) in [
        ("adjustment.detail.level", NativeValue::Int(255)),
        ("adjustment.detail.levels", NativeValue::Int(255)),
        ("adjustment.detail.brightness", NativeValue::Float(150.0)),
        ("adjustment.detail.contrast", NativeValue::Float(-50.0)),
        (
            "adjustment.detail.shadows_cyan_red",
            NativeValue::Float(-100.0),
        ),
    ] {
        assert!(edit(&one(tail, 0), v.clone()).is_ok(), "{tail} {v:?}");
    }
    // ランプの点が昇順でない・近すぎる・端が 0 と 1 でない・中点が範囲外
    for (tail, v) in [
        (
            "adjustment.detail.ramp.colors[1].position",
            NativeValue::Float(0.00005),
        ),
        (
            "adjustment.detail.ramp.colors[1].midpoint",
            NativeValue::Float(1.0),
        ),
        (
            "adjustment.detail.ramp.opacities[1].opacity",
            NativeValue::Float(1.5),
        ),
        ("adjustment.detail.ramp.curve[0].x", NativeValue::Float(0.1)),
        ("adjustment.detail.ramp.curve[2].x", NativeValue::Float(0.9)),
        (
            "adjustment.detail.ramp.curve[1].y",
            NativeValue::Float(-0.1),
        ),
        (
            "adjustment.detail.ramp.curve[1].x",
            NativeValue::Float(0.005),
        ),
        ("adjustment.detail.composite[1].x", NativeValue::Float(0.0)),
        ("adjustment.detail.composite[2].y", NativeValue::Float(1.01)),
    ] {
        assert!(edit(&one(tail, 0), v.clone()).is_err(), "{tail} {v:?}");
    }
    // 調整が効くチャンネル: グラデーションマップ・カラーバランスは色のチャンネルだけ、ほかは法線に当てない
    let targets = field_paths(&native, ".channel");
    let adjustment_channels: Vec<&String> = targets
        .iter()
        .filter(|p| p.contains("adjustment.channels["))
        .collect();
    assert!(!adjustment_channels.is_empty());
    for (k, path) in adjustment_channels.iter().enumerate() {
        // レイヤーごとの最初のチャンネル（Color = 0）を法線 4 に変えると、どの種類でも断る
        if path.ends_with("channels[0].channel") {
            assert!(
                native.with_value(path, NativeValue::Int(4)).is_err(),
                "{path} ({k}): 法線は断る"
            );
        }
    }
}

#[test]
fn colour_only_kinds_refuse_scalar_channels_in_the_format_too() {
    // グラデーションマップだけを、色のチャンネルだけに効くレイヤーとして書き、効くチャンネルを Roughness（1）に替えると断る
    let mut doc = plain();
    let s = AdjustmentSettings::gradient_map(GradientMap::new(Ramp::default(), false));
    doc.add_adjustment_layer("gm", s, Some(&[Channel::Color]), None)
        .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    let path = field_paths(&native, "adjustment.channels[0].channel").remove(0);
    assert!(native.with_value(&path, NativeValue::Int(1)).is_err());
    assert!(
        native.with_value(&path, NativeValue::Int(5)).is_ok(),
        "Emission は色"
    );
    // トーンカーブは Roughness に効いてよく、法線は断る
    let mut doc = plain();
    let s = AdjustmentSettings::tone_curve(ToneCurves::identity());
    doc.add_adjustment_layer("tc", s, Some(&[Channel::Color]), None)
        .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    let path = field_paths(&native, "adjustment.channels[0].channel").remove(0);
    assert!(native.with_value(&path, NativeValue::Int(1)).is_ok());
    assert!(native.with_value(&path, NativeValue::Int(4)).is_err());
    // フィルターの段: 色だけの種類は Height（3）に、どの種類も Normal（4）に置けない
    for (name, s) in adjustments() {
        let mut doc = plain();
        let id = doc.layers()[0].id();
        doc.add_filter(
            id,
            FilterTarget::Content,
            FilterSpec::new(effect(&s)).channels(&[Channel::Color]),
        )
        .unwrap();
        let native = NativeDocument::from_core(&doc).unwrap();
        let path = field_paths(&native, "channels[0].channel")
            .into_iter()
            .find(|p| p.contains("filters"))
            .unwrap();
        let scalar_ok = s.applies_to(ChannelKind::Scalar);
        assert_eq!(
            native.with_value(&path, NativeValue::Int(3)).is_ok(),
            scalar_ok,
            "{name}: Height"
        );
        assert!(
            native.with_value(&path, NativeValue::Int(4)).is_err(),
            "{name}: Normal"
        );
    }
    // マスクのスタックには、色だけの種類は置けない（フィルターの型を替えて確かめる: 2 値化の段を色だけの種類へ）
    let mut doc = plain();
    let id = doc.layers()[0].id();
    doc.add_layer_mask(id).unwrap();
    doc.add_filter(
        id,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::threshold(Threshold::new(9).unwrap())),
    )
    .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    let path = field_paths(&native, ".type")
        .into_iter()
        .find(|p| p.starts_with("layers[0].mask.filters"))
        .or_else(|| {
            field_paths(&native, ".type")
                .into_iter()
                .find(|p| p.contains("mask.filters"))
        })
        .unwrap();
    for kind in [64, 66] {
        assert!(native.with_value(&path, NativeValue::Int(kind)).is_err());
    }
}

#[test]
fn a_version_21_to_23_document_cannot_carry_the_new_kinds() {
    // 版 21 の正本は、種類 64〜69 を持たない。版 21 のまま種類だけを変えても通らない
    let mut old = plain();
    old.add_adjustment_layer("調整", AdjustmentSettings::invert(), None, None)
        .unwrap();
    let native = NativeDocument::from_core(&old).unwrap();
    assert_eq!(native.version(), UNITY_NATIVE_VERSION);
    let path = field_paths(&native, "adjustment.type").remove(0);
    for kind in 64..=69 {
        assert!(native.with_value(&path, NativeValue::Int(kind)).is_err());
    }
    // 版の数だけを 24 に書き換えた正本は、並び（ユーザーチャンネルの数が増える）が合わず読めない
    let mut bytes = native.to_bytes();
    bytes[8..12].copy_from_slice(&ADJUST_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&bytes).is_err());
    // 版 22・23 のまま 64 からの種類を書いた正本も断る（版 22・23 の意味は変えない）
    for version in [USER_CHANNELS_VERSION, PROCEDURAL_VERSION] {
        let mut bytes = NativeDocument::from_core(&document()).unwrap().to_bytes();
        bytes[8..12].copy_from_slice(&version.to_le_bytes());
        assert!(NativeDocument::read(&bytes).is_err(), "版 {version}");
    }
    // 版 24 のまま種類 64 以上を使わない文書は書かない（使う文書だけが 24）
    assert_eq!(
        NativeDocument::from_core(&plain()).unwrap().version(),
        UNITY_NATIVE_VERSION
    );
}

#[test]
fn a_ylp_keeps_the_stages_and_the_version() {
    let doc = document();
    let native = NativeDocument::from_core(&doc).unwrap();
    let spec = SetSpec {
        id: "0f0f0f0f-0000-4000-8000-000000000002".into(),
        name: "Set".into(),
        material: yolu_io::MaterialRef::Unassigned,
        document: Some(native.clone().into()),
        composites: vec![],
    };
    let project = Project::create(writer(), std::slice::from_ref(&spec), &spec.id).unwrap();
    let dir = std::env::temp_dir().join(format!("yolu-io-adjust-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("adjust.ylp");
    let _ = std::fs::remove_file(&path);
    SaveTarget::create(&path).unwrap().save(&project).unwrap();
    let (again, _) = SaveTarget::open(&path).unwrap();
    let reopened = &again.sets()[0].document;
    assert_eq!(reopened.version(), ADJUST_VERSION);
    assert_eq!(reopened.to_bytes().unwrap(), native.to_bytes());
    assert_eq!(settings(&reopened.to_core().unwrap()), settings(&doc));
    // 開いたまま何も変えずに書き直しても、正本のバイトは変わらない
    SaveTarget::create(dir.join("again.ylp"))
        .unwrap()
        .save(&again)
        .unwrap();
    let (third, _) = SaveTarget::open(dir.join("again.ylp")).unwrap();
    assert_eq!(
        third.sets()[0].document.to_bytes().unwrap(),
        native.to_bytes()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn smart_files_do_not_carry_the_new_adjustments_or_stages() {
    // .ylsmart の形式 1 は Unity 版と共有なので、Rust 版だけの調整・段は保存せず、理由を言って断る
    for (name, s) in adjustments() {
        let mut doc = plain();
        let base = doc.layers()[0].id();
        let adj = doc
            .add_adjustment_layer(name, s.clone(), None, None)
            .unwrap();
        let material = doc.capture_smart_material(&[base, adj], "素材").unwrap();
        let err = SmartFile::from_core(&material, &writer()).unwrap_err();
        assert!(
            err.to_string().contains(REFUSAL_RUST_ADJUSTMENTS),
            "{name}: {err}"
        );
        // フィルターの段も同じ（無効にした段も）
        let mut doc = plain();
        let id = doc.layers()[0].id();
        doc.add_filter(
            id,
            FilterTarget::Content,
            FilterSpec::new(effect(&s))
                .channels(&[Channel::Color])
                .disabled(),
        )
        .unwrap();
        let material = doc.capture_smart_material(&[id], "素材").unwrap();
        let err = SmartFile::from_core(&material, &writer()).unwrap_err();
        assert!(
            err.to_string().contains(REFUSAL_RUST_ADJUSTMENTS),
            "{name}: {err}"
        );
    }
    // 今までの種類だけの素材は書ける
    let mut old = plain();
    let base = old.layers()[0].id();
    let adj = old
        .add_adjustment_layer("反転", AdjustmentSettings::invert(), None, None)
        .unwrap();
    let material = old.capture_smart_material(&[base, adj], "素材").unwrap();
    assert!(SmartFile::from_core(&material, &writer()).is_ok());
}

fn read_fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// 版 24 の正解の正本（Rust が書いたもの。固定の ID で作り、ファイルと同じバイト列になる）。
/// 作り直すときは `YOLU_UPDATE_FIXTURES=1 cargo test -p yolu-io --test ylp adjust_bridge::`、続けて
/// `python3 tools/io-fixtures/generate.py --source <Unity 版> --adjust` で Unity 版の読み手の記録を取り直す。
#[test]
fn version_24_fixture_is_what_this_writer_produces() {
    let bytes = NativeDocument::from_core(&document()).unwrap().to_bytes();
    let path = format!(
        "{}/tests/fixtures/adjust-v24.utpaint",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var_os("YOLU_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &bytes).unwrap();
    }
    assert_eq!(bytes, std::fs::read(&path).unwrap());
    assert_eq!(bytes[8..12], 24i32.to_le_bytes());
}

#[test]
fn unity_0_2_0_reader_refuses_version_24_before_touching_the_file() {
    // Unity 0.2.0 の DocumentBinary・YlpFormat に Rust が書いた版 24 を読ませた記録（tools/io-fixtures --adjust）。
    // 版 22・23 と同じく「Unsupported archive version」で断り、Unity 版は種類 64〜69 を別の意味で読まない
    let record = String::from_utf8(read_fixture("adjust-v24.unity.txt")).unwrap();
    let lines: Vec<&str> = record.lines().collect();
    assert_eq!(lines[0], "DocumentBinary.CurrentVersion: 21");
    assert_eq!(lines[1], "YlpFormat.Current: 7");
    let refusal = "InvalidDataException: Unsupported archive version; source retained unchanged.";
    assert_eq!(lines[2], format!("DocumentBinary.Read: {refusal}"));
    assert_eq!(lines[3], format!("DocumentBinary.ReadId: {refusal}"));
    assert_eq!(lines[4], "YlpFormat.Open: OK");
    assert!(
        lines[5].starts_with("TexturePaintWindow.ReadTextureSets（文を組み立てたもの。ウィンドウは未実行）: InvalidDataException: Texture set \"")
            && lines[5].ends_with("\": Unsupported archive version; source retained unchanged."),
        "{}",
        lines[5]
    );
    assert_eq!(lines.len(), 6);
    // 記録した正本は、この書き手が今書くものと同じ（記録が古くなっていない）
    assert_eq!(
        read_fixture("adjust-v24.utpaint"),
        NativeDocument::from_core(&document()).unwrap().to_bytes()
    );
}

#[test]
fn tone_channels_and_default_values_are_stored_independently() {
    // 4 本の曲線は別々に残る（R だけ違う・全体だけ違う、が取り違えられない）
    for channel in ToneChannel::ALL {
        let curves = ToneCurves::identity()
            .with_curve(channel, points(&[(0.0, 0.2), (0.5, 0.45), (1.0, 0.85)]));
        let mut doc = plain();
        doc.add_adjustment_layer(
            "tone",
            AdjustmentSettings::tone_curve(curves.clone()),
            None,
            None,
        )
        .unwrap();
        let back = NativeDocument::from_core(&doc).unwrap().to_core().unwrap();
        let got = back.layers()[1]
            .adjustment()
            .unwrap()
            .tone_curve_value()
            .unwrap();
        assert_eq!(got, &curves, "{channel:?}");
        for other in ToneChannel::ALL {
            assert_eq!(got.curve(other).is_identity(), other != channel);
        }
    }
}
