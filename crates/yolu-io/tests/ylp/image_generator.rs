//! 画像の Generator（種類 70）の保存・復元（正本の版 28）。既定と既定でない設定（投影・成分・反転・合成）をレイヤーの内容とマスクで往復させ、
//! 版の選び方、古い読み手（スタンドアロン 0.4.x・Unity 0.2.0）が版の数で断ること、欄の範囲・デカール・重ねるノイズの検査、
//! アセットに無い画像の ID を残すこと、.ylsmart の断りを試す。
use yolu_core::fill_image::{Projection, ProjectionMode, Wrap};
use yolu_core::generator::{Blend, ImageComponent, Kind, Settings};
use yolu_core::{Channel, Document, EffectSettings, FilterSpec, FilterTarget, LayerId};
use yolu_io::smart::{SmartFile, REFUSAL_IMAGE_GENERATORS};
use yolu_io::{
    NativeDocument, NativeValue, WriterInfo, EFFECTS_VERSION, MAX_NATIVE_VERSION, MIXING_VERSION,
    SEAMS_VERSION, SPLIT_VERSION, UNITY_NATIVE_VERSION,
};

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

/// 既定（画像をまだ選んでいない）。
fn chosen_none() -> Settings {
    Settings::new(Kind::Image)
}

/// 既定でない設定（画像・トライプラナーの置き場・成分・反転・範囲・合成）。
fn odd() -> Settings {
    let mut g = Settings::new(Kind::Image);
    g.image.image = 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210;
    g.image.component = ImageComponent::Alpha;
    g.image.projection = Projection {
        mode: ProjectionMode::Triplanar,
        wrap: Wrap::None,
        tiles: [2.5, 0.125],
        offset: [-0.3, 0.75],
        rotation: -42.5,
        blend_width: 0.6,
        ..Projection::default()
    };
    g.image.projection.placement.center = [0.1, -0.2, 0.3];
    g.image.projection.placement.rotation = [15.0, -30.0, 45.0];
    g.image.projection.placement.size = [1.5, 2.0, 0.75];
    g.invert = true;
    g.low = 0.125;
    g.high = 0.875;
    g.softness = 0.5;
    g.blend = Blend::Screen;
    g
}

/// レイヤーの内容（Color と Roughness）とマスクに 1 つずつ置いた文書。
fn document_with(g: &Settings) -> Document {
    let (mut doc, id) = plain();
    doc.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(g.clone()))
            .channels(&[Channel::Color, Channel::Roughness]),
    )
    .unwrap();
    doc.add_layer_mask(id).unwrap();
    doc.add_filter(
        id,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::generator(g.clone())),
    )
    .unwrap();
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
fn the_image_generator_round_trips_at_version_28() {
    for g in [chosen_none(), odd()] {
        assert!(g.validate().is_ok());
        let doc = document_with(&g);
        let native = NativeDocument::from_core(&doc).unwrap();
        assert_eq!(native.version(), EFFECTS_VERSION);
        let back = native.to_core().unwrap();
        assert_eq!(stages(&back), stages(&doc), "往復で変わった: {g:?}");
        // 正本 → core → 正本、バイト列 → 正本も同じバイト
        let again = NativeDocument::from_core(&back).unwrap();
        assert_eq!(again.to_bytes(), native.to_bytes());
        let reread = NativeDocument::read(&native.to_bytes()).unwrap();
        assert_eq!(reread.to_bytes(), native.to_bytes());
        assert_eq!(reread.version(), EFFECTS_VERSION);
    }
    // 欄の名前（`effect` の塊の中: アセットの画像の ID・投影・成分）
    let native = NativeDocument::from_core(&document_with(&odd())).unwrap();
    for tail in [
        "generator.effect.resource_id",
        "generator.effect.projection.mode",
        "generator.effect.projection.placement.size_z",
        "generator.effect.component",
    ] {
        assert_eq!(field_paths(&native, tail).len(), 2, "{tail}");
    }
    // 無効にした段でも版 28（段が残っている限り）
    let (mut doc, id) = plain();
    doc.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(odd()))
            .channels(&[Channel::Metallic])
            .strength(0.25)
            .disabled(),
    )
    .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), EFFECTS_VERSION);
    assert_eq!(stages(&native.to_core().unwrap()), stages(&doc));
}

#[test]
fn documents_without_the_image_generator_keep_their_version() {
    assert_eq!(EFFECTS_VERSION, 28);
    // 版 28 の上に、継ぎ目の設定（seams_bridge）の版 32 がある
    const { assert!(EFFECTS_VERSION < SEAMS_VERSION && SEAMS_VERSION < MAX_NATIVE_VERSION) };
    let mut with = document_with(&odd());
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
fn older_readers_refuse_the_image_generator_by_the_version() {
    // スタンドアロン 0.4.x の読み手は 1〜25 と分けた正本の 26 だけを読む（`.version の値 28 は未対応または範囲外です (1..25)`。
    // 分けた正本の中の版 28 は「分けた正本の中の版 28 は未対応です」）、Unity 0.2.0 は 1〜21 だけ
    const MAX_04X: i32 = 25;
    assert_eq!(MIXING_VERSION, MAX_04X);
    const { assert!(EFFECTS_VERSION > MAX_04X && EFFECTS_VERSION != SPLIT_VERSION) };
    let native = NativeDocument::from_core(&document_with(&odd())).unwrap();
    let bytes = native.to_bytes();
    assert_eq!(&bytes[..8], b"DOTPAINT");
    assert_eq!(bytes[8..12], EFFECTS_VERSION.to_le_bytes());
    // 版の数だけを 25 に戻した正本は、この読み手も断る（版 25 に種類 70 は無い）
    let mut older = bytes.clone();
    older[8..12].copy_from_slice(&MIXING_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&older).is_err());
    // 版 21 の正本の Generator を種類 70 に変えても断る
    let (mut doc, id) = plain();
    doc.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(Settings::new(Kind::Thickness)))
            .channels(&[Channel::Color]),
    )
    .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), UNITY_NATIVE_VERSION);
    let kind = field_paths(&native, "generator.type").remove(0);
    assert!(native.with_value(&kind, NativeValue::Int(70)).is_err());
}

/// 読める版は、割り振られた番号（1〜25 と 28・32。26 は分けた正本の識別）だけ。割り振られていない番号は、上限の内側でも断る。
#[test]
fn versions_nobody_assigned_are_refused() {
    // 25 より上で割り振られた番号。ほかの機能の版を取り込んだら、末尾に自分の行だけ足す（読む版の判定 `is_known_version` と揃える）
    const ASSIGNED_ABOVE_25: [i32; 9] = [
        SPLIT_VERSION,
        yolu_io::PATHS_VERSION,
        EFFECTS_VERSION,
        yolu_io::POINT_GRADIENT_VERSION,
        SEAMS_VERSION,
        yolu_io::BAKE_PRIORITY_VERSION,
        yolu_io::TEXT_VERSION,
        yolu_io::ANTI_ALIAS_VERSION,
        yolu_io::RULERS_VERSION,
    ];
    let bytes = NativeDocument::from_core(&document_with(&odd()))
        .unwrap()
        .to_bytes();
    for v in [0, -1]
        .into_iter()
        .chain(MIXING_VERSION + 1..=MAX_NATIVE_VERSION + 2)
        .filter(|v| !ASSIGNED_ABOVE_25.contains(v))
    {
        let mut other = bytes.clone();
        other[8..12].copy_from_slice(&v.to_le_bytes());
        let e = NativeDocument::read(&other).unwrap_err().to_string();
        assert!(
            e.contains(&format!(".version の値 {v} は未対応")),
            "{v}: {e}"
        );
    }
    // 割り振った番号は、番号を書き換えない限り読める
    assert_eq!(
        NativeDocument::read(&bytes).unwrap().version(),
        EFFECTS_VERSION
    );
}

/// 継ぎ目の設定を切った（版 32）文書も、画像の Generator（版 28）を落とさず、設定と一緒に往復する。版 32 は版 28 の中身を含む。
#[test]
fn a_document_with_the_seam_setting_off_keeps_its_image_generators() {
    let mut doc = document_with(&odd());
    doc.set_filter_seams(false).unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), SEAMS_VERSION);
    let back = native.to_core().unwrap();
    assert!(!back.filter_seams());
    assert_eq!(stages(&back), stages(&doc));
    let reread = NativeDocument::read(&native.to_bytes()).unwrap();
    assert_eq!(reread.to_bytes(), native.to_bytes());
    // 入に戻すと画像の段が要る版 28 に戻る
    doc.set_filter_seams(true).unwrap();
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().version(),
        EFFECTS_VERSION
    );
}

#[test]
fn the_reader_refuses_a_decal_a_component_out_of_range_and_overlay_noise() {
    let native = NativeDocument::from_core(&document_with(&odd())).unwrap();
    let first = |tail: &str| field_paths(&native, tail).remove(0);
    // 画像の段はデカールに投影できない（塗りつぶしレイヤーの投影の 5）
    assert!(native
        .with_value(
            &first("generator.effect.projection.mode"),
            NativeValue::Int(5)
        )
        .is_err());
    // 投影の種類 0〜4 は読める
    for mode in 0..=4 {
        assert!(native
            .with_value(
                &first("generator.effect.projection.mode"),
                NativeValue::Int(mode)
            )
            .is_ok());
    }
    // 成分は 0〜4
    for (component, ok) in [(0, true), (4, true), (5, false), (-1, false)] {
        assert_eq!(
            native
                .with_value(
                    &first("generator.effect.component"),
                    NativeValue::Int(component)
                )
                .is_ok(),
            ok,
            "{component}"
        );
    }
    // 重ねるノイズを持たない
    assert!(native
        .with_value(&first("generator.noise_amount"), NativeValue::Float(0.5))
        .is_err());
    // 投影の範囲の外
    assert!(native
        .with_value(
            &first("generator.effect.projection.tile_u"),
            NativeValue::Float(0.0)
        )
        .is_err());
    // ほかの種類にすると欄の並びが合わない
    for other in [0, 5, 64, 65] {
        assert!(native
            .with_value(&first("generator.type"), NativeValue::Int(other))
            .is_err());
    }
}

#[test]
fn an_image_missing_from_the_assets_keeps_its_id() {
    // 正本はアセットの一覧を持たない。指す画像がアセットに無くても開くのを断らず、ID を残す（段は入力を通して理由を出す）
    let doc = document_with(&odd());
    let back = NativeDocument::from_core(&doc).unwrap().to_core().unwrap();
    let ids: Vec<_> = back.layers()[0].generator_images().collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().all(|i| i.0 == odd().image.image));
    // 選んでいない段は画像を指さない
    let none = document_with(&chosen_none());
    let back = NativeDocument::from_core(&none).unwrap().to_core().unwrap();
    assert_eq!(back.layers()[0].generator_images().count(), 0);
}

#[test]
fn smart_files_do_not_carry_the_image_generator() {
    let (mut doc, id) = plain();
    doc.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(odd())).channels(&[Channel::Color]),
    )
    .unwrap();
    let material = doc.capture_smart_material(&[id], "素材").unwrap();
    let err = SmartFile::from_core(&material, &writer()).unwrap_err();
    assert!(err.to_string().contains(REFUSAL_IMAGE_GENERATORS), "{err}");
}
