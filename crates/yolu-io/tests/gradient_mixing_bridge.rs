//! グラデーションマップの混色（混色モード・輝度の補正）と区間ごとの混合率曲線の保存・復元（正本の版 25）。使う文書だけが版 25 になり、使わない
//! 文書（これまでのグラデーションマップを含む）は版 24 のまま。往復・版の選び方・読み手の拒否・.ylp への保存・塗りつぶしのグラデーションの断りを試す。
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::generator::{ColorStop, LuminanceCorrection, MixMode, OpacityStop, Ramp};
use yolu_core::{
    AdjustmentSettings, Channel, ColorAdjust, Document, EffectSettings, FilterSpec, FilterTarget,
    GradientMap, Rgba8,
};
use yolu_io::smart::{SmartFile, REFUSAL_RUST_ADJUSTMENTS};
use yolu_io::{
    NativeDocument, NativeValue, Project, SaveTarget, SetSpec, WriterInfo, ADJUST_VERSION,
    MIXING_VERSION,
};

fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.1".into(),
        unity: "none".into(),
    }
}

fn curve(list: &[(f64, f64)]) -> Curve {
    Curve::new(list.iter().map(|&(x, y)| CurvePoint { x, y }).collect()).unwrap()
}

fn base_ramp() -> Ramp {
    let stop = |position, rgb: [u8; 3], midpoint| ColorStop {
        position,
        color: Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
        midpoint,
    };
    let opacity = |position, opacity| OpacityStop {
        position,
        opacity,
        midpoint: 0.5,
    };
    Ramp::new(
        vec![
            stop(0.0, [10, 20, 90], 0.4),
            stop(0.35, [200, 60, 30], 0.62),
            stop(1.0, [250, 240, 200], 0.5),
        ],
        vec![opacity(0.0, 1.0), opacity(1.0, 0.5)],
        None,
    )
    .unwrap()
}

/// 知覚的・輝度の補正「低」・区間 1 だけに混合率曲線。
fn mixed_ramp() -> Ramp {
    base_ramp()
        .with_mixing(MixMode::Perceptual, LuminanceCorrection::Low)
        .with_segment_curve(1, Some(curve(&[(0.0, 0.0), (0.3, 0.7), (1.0, 1.0)])))
        .unwrap()
}

fn plain() -> Document {
    let mut doc = Document::with_tile_size(40, 28, 8).unwrap();
    doc.add_fill_layer("塗り", &[(Channel::Color, Rgba8::new(1, 2, 3, 255))], None)
        .unwrap();
    doc
}

fn with_map(ramp: Ramp) -> Document {
    let mut doc = plain();
    doc.add_adjustment_layer(
        "マップ",
        AdjustmentSettings::gradient_map(GradientMap::new(ramp, false)),
        None,
        None,
    )
    .unwrap();
    doc
}

fn gradient_ramps(doc: &Document) -> Vec<Ramp> {
    let mut list = Vec::new();
    let mut take = |c: Option<ColorAdjust>| {
        if let Some(ColorAdjust::GradientMap(g)) = c {
            list.push(g.ramp().clone());
        }
    };
    for l in doc.layers() {
        take(l.adjustment().and_then(|a| a.color_adjust()));
        for e in l.filters() {
            take(e.settings().color_adjust());
        }
        for e in l.mask().into_iter().flat_map(|m| m.filters()) {
            take(e.settings().color_adjust());
        }
    }
    list
}

#[test]
fn mixing_is_the_only_thing_that_makes_a_version_25_document() {
    assert_eq!(MIXING_VERSION, 25);
    // 昔のグラデーションマップは版 24 のまま。混色の欄を書かない
    let old = NativeDocument::from_core(&with_map(base_ramp())).unwrap();
    assert_eq!(old.version(), ADJUST_VERSION);
    assert!(old.fields().iter().all(|f| !f.path.ends_with(".mix")));
    // 混色モードだけでも、混合率曲線だけでも 25
    let mode = NativeDocument::from_core(&with_map(
        base_ramp().with_mixing(MixMode::Linear, LuminanceCorrection::default()),
    ))
    .unwrap();
    assert_eq!(mode.version(), MIXING_VERSION);
    let seg = NativeDocument::from_core(&with_map(
        base_ramp()
            .with_segment_curve(0, Some(Curve::identity()))
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(seg.version(), MIXING_VERSION);
    // 25 の文書の昔の形のグラデーションマップも、混色の欄を持って往復する
    let mut both = with_map(mixed_ramp());
    both.add_adjustment_layer(
        "昔のマップ",
        AdjustmentSettings::gradient_map(GradientMap::new(base_ramp(), true)),
        None,
        None,
    )
    .unwrap();
    let native = NativeDocument::from_core(&both).unwrap();
    assert_eq!(native.version(), MIXING_VERSION);
    let back = native.to_core().unwrap();
    assert_eq!(gradient_ramps(&back), vec![mixed_ramp(), base_ramp()]);
}

#[test]
fn the_mixing_survives_the_round_trip_byte_for_byte() {
    let doc = with_map(mixed_ramp());
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), MIXING_VERSION);
    let back = native.to_core().unwrap();
    let ramps = gradient_ramps(&back);
    assert_eq!(ramps, vec![mixed_ramp()]);
    assert_eq!(ramps[0].mix_mode(), MixMode::Perceptual);
    assert_eq!(ramps[0].luminance_correction(), LuminanceCorrection::Low);
    assert!(ramps[0].segment_curve(0).is_none());
    assert_eq!(
        ramps[0].segment_curve(1).unwrap().points().len(),
        3,
        "曲線の点"
    );
    // 書き直しても、読み直しても同じバイト
    assert_eq!(
        NativeDocument::from_core(&back).unwrap().to_bytes(),
        native.to_bytes()
    );
    assert_eq!(
        NativeDocument::read(&native.to_bytes()).unwrap().to_bytes(),
        native.to_bytes()
    );
}

#[test]
fn a_filter_stage_carries_the_mixing_too() {
    let mut doc = plain();
    let id = doc.layers()[0].id();
    doc.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::from_color_adjust(ColorAdjust::GradientMap(
            GradientMap::new(mixed_ramp(), false),
        )))
        .channels(&[Channel::Color]),
    )
    .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), MIXING_VERSION);
    assert_eq!(
        gradient_ramps(&native.to_core().unwrap()),
        vec![mixed_ramp()]
    );
    // 無効にした段も数える
    let stage = doc.layers()[0].filters()[0].id();
    doc.set_filter_enabled(id, stage, false).unwrap();
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().version(),
        MIXING_VERSION
    );
}

#[test]
fn a_user_channel_document_with_mixing_keeps_both() {
    let mut doc = with_map(mixed_ramp());
    doc.add_channel(yolu_core::ChannelInfo {
        name: "傷".into(),
        kind: yolu_core::ChannelKind::Scalar,
        color_space: yolu_core::ColorSpace::Linear,
        default: Rgba8::new(0, 0, 0, 255),
    })
    .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), MIXING_VERSION);
    let back = native.to_core().unwrap();
    assert_eq!(back.channels().len(), 7);
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
fn the_reader_refuses_values_the_writer_never_writes() {
    let native = NativeDocument::from_core(&with_map(mixed_ramp())).unwrap();
    let mix = field_paths(&native, ".mix").remove(0);
    let luminance = field_paths(&native, ".luminance").remove(0);
    let segments = field_paths(&native, ".segment_count").remove(0);
    // 範囲外
    assert!(native.with_value(&mix, NativeValue::Int(3)).is_err());
    assert!(native.with_value(&mix, NativeValue::Int(-1)).is_err());
    assert!(native.with_value(&luminance, NativeValue::Int(5)).is_err());
    // 区間の数が色の分岐点と合わない
    assert!(native.with_value(&segments, NativeValue::Int(1)).is_err());
    assert!(native.with_value(&segments, NativeValue::Int(0)).is_err());
    // 知覚的でない混色の輝度の補正は既定の値だけ
    assert!(native.with_value(&mix, NativeValue::Int(0)).is_err());
    let linear = NativeDocument::from_core(&with_map(
        base_ramp().with_mixing(MixMode::Linear, LuminanceCorrection::None),
    ))
    .unwrap();
    let luminance = field_paths(&linear, ".luminance").remove(0);
    assert_eq!(
        linear.field(&luminance),
        Some(&NativeValue::Int(i32::from(
            LuminanceCorrection::default().index()
        ))),
        "知覚的でなければ既定の値で書く"
    );
    assert!(linear.with_value(&luminance, NativeValue::Int(1)).is_err());
}

#[test]
fn older_versions_cannot_carry_the_mixing_fields() {
    // 版の数だけを 24 に書き換えた版 25 の正本は、混色の欄が余って読めない（版 24 の意味は変えない）
    let native = NativeDocument::from_core(&with_map(mixed_ramp())).unwrap();
    let mut bytes = native.to_bytes();
    bytes[8..12].copy_from_slice(&ADJUST_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&bytes).is_err());
    // 版 24 の正本の版の数だけを 25 に書き換えても、混色の欄が足りず読めない
    let old = NativeDocument::from_core(&with_map(base_ramp())).unwrap();
    let mut bytes = old.to_bytes();
    bytes[8..12].copy_from_slice(&MIXING_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&bytes).is_err());
    // 版 26 は読まない
    let mut bytes = native.to_bytes();
    bytes[8..12].copy_from_slice(&26i32.to_le_bytes());
    assert!(NativeDocument::read(&bytes).is_err());
}

#[test]
fn a_ylp_keeps_the_mixing_and_the_version() {
    let doc = with_map(mixed_ramp());
    let native = NativeDocument::from_core(&doc).unwrap();
    let spec = SetSpec {
        id: "0f0f0f0f-0000-4000-8000-000000000025".into(),
        name: "Set".into(),
        material: yolu_io::MaterialRef::Unassigned,
        document: Some(native.clone()),
        composites: vec![],
    };
    let project = Project::create(writer(), std::slice::from_ref(&spec), &spec.id).unwrap();
    let dir = std::env::temp_dir().join(format!("yolu-io-mixing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mixing.ylp");
    let _ = std::fs::remove_file(&path);
    SaveTarget::create(&path).unwrap().save(&project).unwrap();
    let (again, _) = SaveTarget::open(&path).unwrap();
    let reopened = &again.sets()[0].document;
    assert_eq!(reopened.version(), MIXING_VERSION);
    assert_eq!(reopened.to_bytes(), native.to_bytes());
    assert_eq!(
        gradient_ramps(&reopened.to_core().unwrap()),
        vec![mixed_ramp()]
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_gradient_map_with_mixing_is_not_put_on_the_shelf() {
    // .ylsmart（形式 1）は Unity 版と共有で、Rust 版だけの調整は入れない（版 24 のグラデーションマップと同じ断り）
    let mut doc = plain();
    let base = doc.layers()[0].id();
    let adj = doc
        .add_adjustment_layer(
            "マップ",
            AdjustmentSettings::gradient_map(GradientMap::new(mixed_ramp(), false)),
            None,
            None,
        )
        .unwrap();
    let material = doc.capture_smart_material(&[base, adj], "素材").unwrap();
    let err = SmartFile::from_core(&material, &writer()).unwrap_err();
    assert!(err.to_string().contains(REFUSAL_RUST_ADJUSTMENTS), "{err}");
}

#[test]
fn a_fill_gradient_ramp_with_mixing_is_refused_instead_of_dropping_it() {
    use yolu_core::generator::{Blend, Kind, Settings};
    // 塗りつぶしのグラデーションのランプは Unity 版と共有の並び。混色を持つランプは黙って落とさず、保存を断る
    let mut doc = plain();
    let id = doc.layers()[0].id();
    let mut g = Settings::new(Kind::ShapeGradient);
    g.blend = Blend::Replace;
    g.ramp = Some(base_ramp());
    doc.set_fill_gradient(id, Channel::Color, Some(g.clone()), false)
        .unwrap();
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().version(),
        yolu_io::UNITY_NATIVE_VERSION,
        "混色の無いランプは今までどおり"
    );
    g.ramp = Some(mixed_ramp());
    doc.set_fill_gradient(id, Channel::Color, Some(g), false)
        .unwrap();
    let err = NativeDocument::from_core(&doc).unwrap_err();
    assert!(matches!(
        err,
        yolu_io::Error::Unwritable(yolu_io::Unwritable::GeneratorRampMixing)
    ));
}
