use super::*;
use crate::generator::Kind as G;

fn given(items: &[(&str, ParamValue)]) -> BTreeMap<String, ParamValue> {
    items.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
}

fn num_v(v: f64) -> ParamValue {
    ParamValue::Number(v)
}

#[test]
fn every_parameter_has_a_default_inside_its_own_type() {
    for kind in kinds() {
        if !kind.addable {
            assert!(kind.params.is_empty(), "{} は足せないので欄を持たない", kind.id);
            assert!(!kind.opaque.is_empty(), "{} は足せない理由の中身を挙げる", kind.id);
        }
        assert!(kind.stack || kind.adjustment, "{}", kind.id);
        for param in &kind.params {
            check_value(param, &param.default)
                .unwrap_or_else(|e| panic!("{}.{} の既定が型の外: {e}", kind.id, param.name));
        }
        let names: std::collections::HashSet<_> = kind.params.iter().map(|p| p.name).collect();
        assert_eq!(names.len(), kind.params.len(), "{} の欄の名前が重なっている", kind.id);
    }
    let ids: std::collections::HashSet<_> = kinds().iter().map(|k| k.id).collect();
    assert_eq!(ids.len(), kinds().len());
}

#[test]
fn every_addable_kind_builds_from_defaults_and_reads_back_the_same_values() {
    for kind in kinds().iter().filter(|k| k.addable) {
        let defaults: Vec<(&str, ParamValue)> =
            kind.params.iter().map(|p| (p.name, p.default.clone())).collect();
        if kind.stack {
            let s = EffectSettings::from_catalog(kind.id, &BTreeMap::new())
                .unwrap_or_else(|e| panic!("{}: {e}", kind.id));
            assert_eq!(s.kind_id(), kind.id);
            assert_eq!(s.catalog_values(), defaults, "{}", kind.id);
            // 読んだ値をそのまま渡し直すと同じ設定
            let again = EffectSettings::from_catalog(kind.id, &given(&defaults)).unwrap();
            assert_eq!(again, s, "{}", kind.id);
            assert_eq!(s.with_catalog_values(&BTreeMap::new()).unwrap(), s, "{}", kind.id);
            assert_eq!(s.is_generator(), kind.generator, "{}", kind.id);
        }
        if kind.adjustment {
            let a = AdjustmentSettings::from_catalog(kind.id, &BTreeMap::new())
                .unwrap_or_else(|e| panic!("{}: {e}", kind.id));
            assert_eq!(a.kind_id(), kind.id);
            assert_eq!(a.catalog_values(), defaults, "{}", kind.id);
            assert_eq!(
                AdjustmentSettings::from_catalog(kind.id, &given(&defaults)).unwrap(),
                a,
                "{}",
                kind.id
            );
        }
    }
}

#[test]
fn defaults_are_the_settings_a_new_effect_gets() {
    assert_eq!(
        EffectSettings::from_catalog("blur", &BTreeMap::new()).unwrap(),
        EffectSettings::blur(4)
    );
    assert_eq!(
        EffectSettings::from_catalog("sharpen", &BTreeMap::new()).unwrap(),
        EffectSettings::sharpen(2, 1.0, 0)
    );
    assert_eq!(
        EffectSettings::from_catalog("edge_wear", &BTreeMap::new()).unwrap(),
        EffectSettings::generator(generator::Settings::new(G::EdgeWear))
    );
    assert_eq!(
        EffectSettings::from_catalog("grunge", &BTreeMap::new()).unwrap(),
        EffectSettings::generator(generator::Settings::new(G::Grunge))
    );
    assert_eq!(
        AdjustmentSettings::from_catalog("levels", &BTreeMap::new()).unwrap(),
        AdjustmentSettings::levels(0.0, 1.0, 1.0, 0.0, 1.0).unwrap()
    );
}

/// 範囲の外は欄ごとに断る（切り詰めない）。どの欄も、範囲のすぐ外の値を表が断る。そして組む道（表を通さない）でも、core の検査が
/// 同じ値を断る（表と検査が同じ定義を使っている）。
#[test]
fn values_just_outside_a_range_are_refused_by_the_table_and_by_the_core_check() {
    for kind in kinds().iter().filter(|k| k.addable) {
        for param in &kind.params {
            let (lo, hi, step): (f64, f64, f64) = match &param.ty {
                ParamType::Integer { min, max } => (*min as f64, *max as f64, 1.0),
                ParamType::Number { min, max } => (*min, *max, 1e-3),
                _ => continue,
            };
            // 欄の型が扱える端（i32 のシードは範囲の外が f64 でも表せる）
            for outside in [lo - step, hi + step] {
                let err = if kind.stack {
                    EffectSettings::from_catalog(kind.id, &given(&[(param.name, num_v(outside))]))
                        .unwrap_err()
                } else {
                    AdjustmentSettings::from_catalog(kind.id, &given(&[(param.name, num_v(outside))]))
                        .unwrap_err()
                };
                assert!(
                    matches!(err, ParamError::OutOfRange { name, .. } if name == param.name),
                    "{}.{} = {outside}: {err}",
                    kind.id,
                    param.name
                );
                // 表の検査を通さずに組むと、core の検査が断る
                let mut bag = default_bag(kind.id);
                bag.insert(param.name, num_v(outside));
                let built = if kind.stack {
                    build_stack(kind.id, &bag, None).map(|_| ())
                } else {
                    build_adjustment(kind.id, &bag).map(|_| ())
                };
                // シードは i32 へ丸めるので範囲の外が通る（型が範囲）。それ以外は core が断る
                let is_seed = matches!(param.ty, ParamType::Integer { min, .. } if min == i64::from(i32::MIN));
                // 負の値は符号なしの欄へ丸められて（0）範囲に入るので、表の検査だけが断る
                let negative_into_unsigned = matches!(param.ty, ParamType::Integer { min, .. } if min >= 0) && outside < 0.0;
                if !is_seed && !negative_into_unsigned {
                    assert!(
                        matches!(built, Err(ParamError::Refused(_))),
                        "{}.{} = {outside} を core が断らない: {built:?}",
                        kind.id,
                        param.name
                    );
                }
            }
            // 範囲の端そのものは、表は断らない（組み合わせの条件で断られることはある）
            for edge in [lo, hi] {
                let r = if kind.stack {
                    EffectSettings::from_catalog(kind.id, &given(&[(param.name, num_v(edge))]))
                        .map(|_| ())
                } else {
                    AdjustmentSettings::from_catalog(kind.id, &given(&[(param.name, num_v(edge))]))
                        .map(|_| ())
                };
                assert!(
                    matches!(r, Ok(()) | Err(ParamError::Refused(_))),
                    "{}.{} の端 {edge}: {r:?}",
                    kind.id,
                    param.name
                );
            }
        }
    }
}

#[test]
fn the_edges_of_single_parameters_build_when_nothing_else_conflicts() {
    let blur = |r: f64| EffectSettings::from_catalog("blur", &given(&[("radius", num_v(r))]));
    assert!(blur(1.0).is_ok() && blur(256.0).is_ok());
    assert!(blur(0.0).is_err() && blur(257.0).is_err());
    assert!(matches!(blur(2.5), Err(ParamError::NotInteger { name: "radius" })));
    let thr = |v: f64| AdjustmentSettings::from_catalog("threshold", &given(&[("level", num_v(v))]));
    assert!(thr(1.0).is_ok() && thr(255.0).is_ok() && thr(0.0).is_err());
    let post = |v: f64| AdjustmentSettings::from_catalog("posterize", &given(&[("levels", num_v(v))]));
    assert!(post(2.0).is_ok() && post(255.0).is_ok() && post(1.0).is_err());
    let balance = |v: f64| {
        AdjustmentSettings::from_catalog("color_balance", &given(&[("midtones_cyan_red", num_v(v))]))
    };
    assert!(balance(-100.0).is_ok() && balance(100.0).is_ok() && balance(100.5).is_err());
}

#[test]
fn combinations_the_core_check_refuses_are_reported_as_refused() {
    // レベル補正は入力の幅が 1/255 以上
    let r = AdjustmentSettings::from_catalog(
        "levels",
        &given(&[("input_black", num_v(0.5)), ("input_white", num_v(0.5))]),
    );
    assert!(matches!(r, Err(ParamError::Refused(_))), "{r:?}");
    let r = EffectSettings::from_catalog(
        "levels",
        &given(&[("input_black", num_v(0.5)), ("input_white", num_v(0.5))]),
    );
    assert!(matches!(r, Err(ParamError::Refused(_))), "{r:?}");
    // Generator は高さが低さより 0.001 以上上
    let r = EffectSettings::from_catalog("dirt", &given(&[("low", num_v(0.5)), ("high", num_v(0.5))]));
    assert!(matches!(r, Err(ParamError::Refused(_))), "{r:?}");
    // 方向は零ベクトルにできない
    let r = EffectSettings::from_catalog(
        "direction",
        &given(&[
            ("direction_x", num_v(0.0)),
            ("direction_y", num_v(0.0)),
            ("direction_z", num_v(0.0)),
        ]),
    );
    assert!(matches!(r, Err(ParamError::Refused(_))), "{r:?}");
    // セルの出力は Worley だけ
    let r = EffectSettings::from_catalog(
        "procedural_noise",
        &given(&[("cell_output", ParamValue::Choice("f2".into()))]),
    );
    assert!(matches!(r, Err(ParamError::Refused(_))), "{r:?}");
    let ok = EffectSettings::from_catalog(
        "procedural_noise",
        &given(&[
            ("basis", ParamValue::Choice("worley".into())),
            ("cell_output", ParamValue::Choice("f2".into())),
        ]),
    );
    assert!(ok.is_ok(), "{ok:?}");
}

#[test]
fn wrong_names_types_and_options_are_told_apart() {
    let e = |kind: &str, items: &[(&str, ParamValue)]| {
        EffectSettings::from_catalog(kind, &given(items)).unwrap_err()
    };
    assert!(matches!(e("nope", &[]), ParamError::UnknownKind(_)));
    assert!(matches!(e("blur", &[("size", num_v(1.0))]), ParamError::UnknownParam { .. }));
    assert!(matches!(
        e("blur", &[("radius", ParamValue::Bool(true))]),
        ParamError::WrongType { name: "radius", .. }
    ));
    assert!(matches!(
        e("edge_wear", &[("blend", ParamValue::Choice("xor".into()))]),
        ParamError::UnknownOption { name: "blend", .. }
    ));
    assert!(matches!(
        e("edge_wear", &[("invert", num_v(1.0))]),
        ParamError::WrongType { name: "invert", .. }
    ));
    assert!(matches!(
        e("noise", &[("amount", num_v(f64::NAN))]),
        ParamError::OutOfRange { name: "amount", .. }
    ));
    // 使い方が違う種類
    assert!(matches!(
        EffectSettings::from_catalog("hue_saturation", &BTreeMap::new()),
        Err(ParamError::WrongTarget { adjustment: false, .. })
    ));
    assert!(matches!(
        AdjustmentSettings::from_catalog("blur", &BTreeMap::new()),
        Err(ParamError::WrongTarget { adjustment: true, .. })
    ));
    // 足せない種類
    for id in ["gradient_map", "tone_curve", "shape_gradient", "id_color", "anchor"] {
        assert!(
            matches!(EffectSettings::from_catalog(id, &BTreeMap::new()), Err(ParamError::NotEditable { .. })),
            "{id}"
        );
    }
    assert!(matches!(
        AdjustmentSettings::from_catalog("tone_curve", &BTreeMap::new()),
        Err(ParamError::NotEditable { .. })
    ));
}

#[test]
fn an_existing_effect_with_a_list_part_is_readable_and_keeps_that_part() {
    let tone = EffectSettings::from_color_adjust(
        ColorAdjust::default_for(AdjustmentType::ToneCurve).unwrap(),
    );
    assert_eq!(tone.kind_id(), "tone_curve");
    assert_eq!(tone.opaque_parts(), &["curves"]);
    assert!(tone.catalog_values().is_empty());
    assert_eq!(tone.with_catalog_values(&BTreeMap::new()).unwrap(), tone);
    assert!(matches!(
        tone.with_catalog_values(&given(&[("x", num_v(1.0))])),
        Err(ParamError::NotEditable { kind: "tone_curve" })
    ));
    let gmap = EffectSettings::from_color_adjust(
        ColorAdjust::default_for(AdjustmentType::GradientMap).unwrap(),
    );
    assert_eq!(gmap.kind_id(), "gradient_map");
    assert_eq!(gmap.opaque_parts(), &["ramp"]);
    let anchor = EffectSettings::generator(generator::Settings::new(G::Anchor));
    assert_eq!(anchor.kind_id(), "anchor");
    assert_eq!(anchor.opaque_parts(), &["anchor"]);
    let adjust = AdjustmentSettings::gradient_map(crate::GradientMap::new(generator::Ramp::default(), true));
    assert_eq!(adjust.kind_id(), "gradient_map");
    assert_eq!(adjust.opaque_parts(), &["ramp"]);
}

#[test]
fn changing_values_keeps_the_pinned_maps_of_a_generator() {
    let mut g = generator::Settings::new(G::EdgeWear);
    let key = "a".repeat(64);
    g.pins.insert(generator::MapKind::Curvature, key.clone());
    let existing = EffectSettings::generator(g);
    let changed = existing.with_catalog_values(&given(&[("low", num_v(0.1))])).unwrap();
    let g = changed.generator_settings().unwrap();
    assert_eq!(g.low, 0.1);
    assert_eq!(g.pins.get(&generator::MapKind::Curvature), Some(&key));
    // 渡さなかった欄は今の値のまま
    assert_eq!(g.high, 0.3);
}

#[test]
fn changing_the_grunge_preset_takes_the_presets_size_and_levels_unless_given() {
    let stain = EffectSettings::from_catalog("grunge", &BTreeMap::new()).unwrap();
    let peeling = stain
        .with_catalog_values(&given(&[("preset", ParamValue::Choice("peeling".into()))]))
        .unwrap();
    let g = peeling.generator_settings().unwrap();
    assert_eq!(g.procedural.preset, GrungePreset::Peeling);
    assert_eq!(g.procedural.scale, GrungePreset::Peeling.default_scale());
    assert_eq!((g.low, g.high), GrungePreset::Peeling.default_levels());
    // 一緒に渡した値が先
    let explicit = stain
        .with_catalog_values(&given(&[
            ("preset", ParamValue::Choice("peeling".into())),
            ("scale", num_v(0.5)),
        ]))
        .unwrap();
    assert_eq!(explicit.generator_settings().unwrap().procedural.scale, 0.5);
    // 同じプリセットなら何も変えない
    let same = peeling
        .with_catalog_values(&given(&[("preset", ParamValue::Choice("peeling".into()))]))
        .unwrap();
    assert_eq!(same, peeling);
}

#[test]
fn a_kind_table_row_exists_for_every_generator_kind_and_adjustment_type() {
    for kind in [
        G::EdgeWear,
        G::Dirt,
        G::PositionGradient,
        G::Thickness,
        G::Direction,
        G::ShapeGradient,
        G::IdColor,
        G::Anchor,
        G::Noise,
        G::Grunge,
    ] {
        let id = generator_kind_id(kind);
        let row = super::kind(id).unwrap_or_else(|| panic!("{id} が表に無い"));
        assert!(row.generator && row.stack && !row.adjustment, "{id}");
        assert_eq!(row.needs_maps, !kind.is_procedural(), "{id}");
    }
    for t in [
        AdjustmentType::Invert,
        AdjustmentType::Levels,
        AdjustmentType::HueSaturation,
        AdjustmentType::GradientMap,
        AdjustmentType::ToneCurve,
        AdjustmentType::ColorBalance,
        AdjustmentType::BrightnessContrast,
        AdjustmentType::Threshold,
        AdjustmentType::Posterize,
    ] {
        let settings = match ColorAdjust::default_for(t) {
            Some(c) => c.into_settings(),
            None => default_adjustment(match t {
                AdjustmentType::Invert => "invert",
                AdjustmentType::Levels => "levels",
                _ => "hue_saturation",
            })
            .unwrap(),
        };
        let row = super::kind(settings.kind_id()).unwrap_or_else(|| panic!("{t:?} が表に無い"));
        assert!(row.adjustment, "{}", row.id);
    }
}

#[test]
fn a_built_effect_can_be_added_to_a_document_and_read_back() {
    use crate::effects::{FilterSpec, FilterTarget};
    let mut doc = crate::Document::new(16, 16).unwrap();
    let layer = doc.add_layer("a").unwrap();
    for kind in kinds().iter().filter(|k| k.addable && k.stack) {
        let settings = EffectSettings::from_catalog(kind.id, &BTreeMap::new()).unwrap();
        let id = doc
            .add_filter(layer, FilterTarget::Content, FilterSpec::new(settings.clone()))
            .unwrap_or_else(|e| panic!("{}: {e}", kind.id));
        let (_, effect, _) = doc.find_filter(id).unwrap();
        assert_eq!(effect.settings(), &settings, "{}", kind.id);
    }
}

#[test]
fn the_rust_only_kinds_are_the_ones_unity_cannot_read() {
    let rust_only: Vec<&str> = kinds().iter().filter(|k| k.rust_only).map(|k| k.id).collect();
    assert_eq!(
        rust_only,
        [
            "gradient_map",
            "tone_curve",
            "color_balance",
            "brightness_contrast",
            "threshold",
            "posterize",
            "procedural_noise",
            "grunge"
        ]
    );
}
