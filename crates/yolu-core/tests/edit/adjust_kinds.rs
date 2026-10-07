//! 色調補正の追加の 6 種（グラデーションマップ・トーンカーブ・カラーバランス・明るさ/コントラスト・2 値化・ポスタリゼーション。Rust 版だけの種類）を、
//! 文書の調整レイヤーとフィルターのスタックで使ったときの振る舞い: 合成の結果・使えるチャンネル・Undo・履歴の大きさ・領域の合成と 1 画素の参照の一致・
//! スレッド数に依らないこと・クリッピング・マスク・不透明度・Anchor。式そのものの固定は `adjust::ops` の単体の試験が持つ。

use sha2::{Digest, Sha256};
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::effects::{EffectSettings, FilterSpec, FilterTarget};
use yolu_core::generator::{anchor, Ramp, Source};
use yolu_core::{
    AdjustmentSettings, BlendMode, BrightnessContrast, Channel, ChannelKind, ColorBalance,
    CoreError, Document, GradientMap, LayerId, Posterize, Rect, Rgba8, Threshold, ToneChannel,
    ToneCurves,
};

fn inverted_curve() -> Curve {
    Curve::new(vec![
        CurvePoint { x: 0., y: 1. },
        CurvePoint { x: 1., y: 0. },
    ])
    .unwrap()
}

/// 6 種を 1 つずつ（名前つき）。
fn kinds() -> Vec<(&'static str, AdjustmentSettings)> {
    vec![
        (
            "グラデーションマップ",
            AdjustmentSettings::gradient_map(GradientMap::new(Ramp::default(), true)),
        ),
        (
            "トーンカーブ",
            AdjustmentSettings::tone_curve(
                ToneCurves::identity().with_curve(ToneChannel::Composite, inverted_curve()),
            ),
        ),
        (
            "カラーバランス",
            AdjustmentSettings::color_balance(
                ColorBalance::new([0.0; 3], [40.0, 0.0, -40.0], [0.0; 3], true).unwrap(),
            ),
        ),
        (
            "明るさ・コントラスト",
            AdjustmentSettings::brightness_contrast(BrightnessContrast::new(60.0, 30.0).unwrap()),
        ),
        (
            "2 値化",
            AdjustmentSettings::threshold(Threshold::new(100).unwrap()),
        ),
        (
            "ポスタリゼーション",
            AdjustmentSettings::posterize(Posterize::new(4).unwrap()),
        ),
    ]
}

/// 調整レイヤーの設定と同じ値のフィルターの段。
fn effect_of(s: &AdjustmentSettings) -> EffectSettings {
    EffectSettings::from_color_adjust(s.color_adjust().expect("64 からの種類"))
}

fn small() -> Document {
    Document::with_tile_size(16, 16, 8).unwrap()
}

fn one_pixel(c: Rgba8) -> (Document, LayerId) {
    let mut d = small();
    let l = d.add_layer("Paint").unwrap();
    d.set_pixel(l, 2, 2, c).unwrap();
    d.clear_history().unwrap();
    (d, l)
}

fn at(d: &Document, x: u32, y: u32) -> Rgba8 {
    d.composite_pixel(Channel::Color, x, y).unwrap()
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn each_kind_changes_the_colour_below_through_its_formula_and_never_the_alpha() {
    for (name, s) in kinds() {
        let c = Rgba8::new(120, 90, 200, 128);
        let (mut d, _) = one_pixel(c);
        d.add_adjustment_layer(name, s.clone(), None, None).unwrap();
        assert_eq!(at(&d, 2, 2), s.apply(c), "{name}");
        assert_eq!(at(&d, 2, 2).a, 128, "{name}: アルファはそのまま");
        assert_eq!(at(&d, 9, 9), Rgba8::TRANSPARENT, "{name}: 透明はそのまま");
        assert_ne!(at(&d, 2, 2), c, "{name}: 色は変わる");
    }
}

#[test]
fn opacity_mask_and_blend_mode_mix_every_kind_back() {
    for (name, s) in kinds() {
        let c = Rgba8::new(120, 90, 200, 255);
        let (mut d, _) = one_pixel(c);
        let adj = d.add_adjustment_layer(name, s.clone(), None, None).unwrap();
        let full = at(&d, 2, 2);
        d.set_layer_opacity(adj, 0.0, false).unwrap();
        assert_eq!(at(&d, 2, 2), c, "{name}: 不透明度 0 は元のまま");
        d.set_layer_opacity(adj, 0.5, false).unwrap();
        let half = at(&d, 2, 2);
        for (h, (a, b)) in [
            (half.r, (c.r, full.r)),
            (half.g, (c.g, full.g)),
            (half.b, (c.b, full.b)),
        ] {
            let (lo, hi) = (a.min(b), a.max(b));
            assert!(
                (lo..=hi).contains(&h),
                "{name}: 半分は元と全部の間 {c:?} {half:?} {full:?}"
            );
        }
        d.set_layer_opacity(adj, 1.0, false).unwrap();
        d.set_layer_blend_mode(adj, BlendMode::Multiply).unwrap();
        let mult = at(&d, 2, 2);
        assert!(
            mult.r <= c.r && mult.g <= c.g && mult.b <= c.b,
            "{name}: 乗算は暗くなる側 {mult:?}"
        );
    }
}

#[test]
fn normal_is_never_a_target_and_colour_only_kinds_skip_the_data_channels() {
    for (name, s) in kinds() {
        let mut d = small();
        // 名指しした法線は断る
        assert!(
            d.add_adjustment_layer(name, s.clone(), Some(&[Channel::Normal]), None)
                .is_err(),
            "{name}: 法線は断る"
        );
        let id = d.add_adjustment_layer(name, s.clone(), None, None).unwrap();
        let enabled = d.layer(id).unwrap().enabled_channels();
        assert!(!enabled.contains(&Channel::Normal), "{name}");
        assert!(enabled.contains(&Channel::Color), "{name}");
        assert!(
            d.set_channel_enabled(id, Channel::Normal, true).is_err(),
            "{name}"
        );
        let scalar_ok = s.applies_to(ChannelKind::Scalar);
        assert_eq!(
            enabled.contains(&Channel::Roughness),
            scalar_ok,
            "{name}: スカラーは色だけの種類では外れる"
        );
        assert_eq!(
            d.set_channel_enabled(id, Channel::Height, true).is_ok(),
            scalar_ok,
            "{name}"
        );
        if !scalar_ok {
            assert!(
                d.add_adjustment_layer(name, s.clone(), Some(&[Channel::Roughness]), None)
                    .is_err(),
                "{name}: 名指しも断る"
            );
        }
    }
}

#[test]
fn switching_a_layer_to_a_kind_that_does_not_fit_an_enabled_channel_is_refused() {
    let mut d = small();
    let id = d
        .add_adjustment_layer(
            "t",
            AdjustmentSettings::threshold(Threshold::new(10).unwrap()),
            None,
            None,
        )
        .unwrap();
    assert!(d.layer(id).unwrap().is_channel_enabled(Channel::Roughness));
    let before = d.layer(id).unwrap().adjustment().cloned();
    let gm = AdjustmentSettings::gradient_map(GradientMap::new(Ramp::default(), false));
    assert!(matches!(
        d.set_adjustment(id, gm.clone(), false),
        Err(CoreError::Unsupported(_))
    ));
    assert_eq!(
        d.layer(id).unwrap().adjustment().cloned(),
        before,
        "断ったら元のまま"
    );
    assert_eq!(d.undo_count(), 1, "断った操作は履歴に積まない");
    // 色だけのチャンネルにしてから替えられる
    for ch in [Channel::Roughness, Channel::Metallic, Channel::Height] {
        d.set_channel_enabled(id, ch, false).unwrap();
    }
    d.set_adjustment(id, gm, false).unwrap();
}

#[test]
fn changing_between_kinds_is_one_undo_and_the_history_counts_what_each_carries() {
    let (mut d, _) = one_pixel(Rgba8::new(100, 100, 100, 255));
    let adj = d
        .add_adjustment_layer(
            "a",
            AdjustmentSettings::threshold(Threshold::new(50).unwrap()),
            None,
            None,
        )
        .unwrap();
    d.clear_history().unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(255, 255, 255, 255));
    let gradient = AdjustmentSettings::tone_curve(
        ToneCurves::identity().with_curve(ToneChannel::Composite, inverted_curve()),
    );
    let old_size = d.layer(adj).unwrap().adjustment().unwrap().byte_size();
    d.set_adjustment(adj, gradient.clone(), false).unwrap();
    assert_eq!(d.undo_count(), 1);
    assert_eq!(at(&d, 2, 2), Rgba8::new(155, 155, 155, 255));
    // 履歴の見積りは前の値と後の値の大きさの和（ランプと表を持つ種類のほうが大きい）
    assert_eq!(d.history_bytes(), old_size + gradient.byte_size());
    assert!(d.history_bytes() > 128);
    d.undo().unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(255, 255, 255, 255));
    assert_eq!(
        d.layer(adj)
            .unwrap()
            .adjustment()
            .and_then(|a| a.threshold_value().map(|t| t.level())),
        Some(50)
    );
    d.redo().unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(155, 155, 155, 255));
    assert_eq!(d.layer(adj).unwrap().adjustment(), Some(&gradient));
}

#[test]
fn an_edit_between_the_three_older_kinds_costs_the_same_128_as_before() {
    // 反転・レベル補正・色相/彩度は中身を持たないので、どの組でも編集 1 回の見積りは旧来の 128 のまま
    // （ランプ・曲線・表を持つ種類を足したことで、旧 3 種の Undo の上限への当たり方を変えない）
    let older = [
        AdjustmentSettings::invert(),
        AdjustmentSettings::levels(0.1, 0.9, 1.2, 0.0, 1.0).unwrap(),
        AdjustmentSettings::hue_saturation(30.0, 0.2, 0.0).unwrap(),
    ];
    for from in &older {
        for to in &older {
            if from == to {
                continue;
            }
            let (mut d, _) = one_pixel(Rgba8::new(100, 100, 100, 255));
            let adj = d
                .add_adjustment_layer("a", from.clone(), Some(&[Channel::Color]), None)
                .unwrap();
            d.clear_history().unwrap();
            d.set_adjustment(adj, to.clone(), false).unwrap();
            assert_eq!(d.undo_count(), 1);
            assert_eq!(
                d.history_bytes(),
                128,
                "{:?} → {:?}",
                from.kind(),
                to.kind()
            );
        }
    }
}

#[test]
fn slider_edits_of_a_new_kind_coalesce_into_one_undo_step() {
    let (mut d, _) = one_pixel(Rgba8::new(100, 100, 100, 255));
    let adj = d
        .add_adjustment_layer(
            "bc",
            AdjustmentSettings::brightness_contrast(BrightnessContrast::new(0.0, 0.0).unwrap()),
            None,
            None,
        )
        .unwrap();
    d.clear_history().unwrap();
    for b in [10.0, 30.0, 70.0, 120.0] {
        d.set_adjustment(
            adj,
            AdjustmentSettings::brightness_contrast(BrightnessContrast::new(b, 0.0).unwrap()),
            true,
        )
        .unwrap();
    }
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(100, 100, 100, 255));
    d.redo().unwrap();
    assert_ne!(at(&d, 2, 2), Rgba8::new(100, 100, 100, 255));
}

#[test]
fn a_data_channel_takes_only_the_composite_tone_curve() {
    let mut d = small();
    let l = d.add_layer("P").unwrap();
    d.set_channel_pixel(l, Channel::Roughness, 2, 2, Rgba8::new(40, 40, 40, 255))
        .unwrap();
    let curves = ToneCurves::identity()
        .with_curve(ToneChannel::Red, inverted_curve())
        .with_curve(ToneChannel::Composite, inverted_curve());
    d.add_adjustment_layer("tone", AdjustmentSettings::tone_curve(curves), None, None)
        .unwrap();
    // 色: R の曲線を先に、全体の曲線を後に（R は 2 回反転して元へ、G・B は 1 回反転）
    d.set_pixel(l, 2, 2, Rgba8::new(40, 40, 40, 255)).unwrap();
    assert_eq!(at(&d, 2, 2), Rgba8::new(40, 215, 215, 255));
    // スカラー: 全体の曲線だけ（灰色のまま 1 回反転）
    assert_eq!(
        d.composite_pixel(Channel::Roughness, 2, 2).unwrap(),
        Rgba8::new(215, 215, 215, 255)
    );
}

fn busy_document() -> Document {
    let mut d = Document::with_tile_size(37, 29, 8).unwrap();
    let mut ids = vec![];
    for l in 0..3u32 {
        let id = d.add_layer(&format!("L{l}")).unwrap();
        for y in 0..29 {
            for x in 0..37 {
                if (x * 7 + y * 13 + l * 5) % 11 < 8 {
                    let v = ((x * 13 + y * 29 + l * 61) % 256) as u8;
                    d.set_pixel(
                        id,
                        x,
                        y,
                        Rgba8::new(
                            v,
                            255 - v,
                            (v / 2).wrapping_add(l as u8 * 40),
                            200 + (l as u8) * 20,
                        ),
                    )
                    .unwrap();
                }
            }
        }
        ids.push(id);
    }
    d.set_layer_clipping(ids[2], true).unwrap();
    for (i, (name, s)) in kinds().into_iter().enumerate() {
        let adj = d.add_adjustment_layer(name, s, None, None).unwrap();
        d.set_layer_opacity(adj, 0.5 + 0.08 * i as f64, false)
            .unwrap();
        if i == 2 {
            d.add_layer_mask(adj).unwrap();
            for k in 0..80 {
                d.set_mask_pixel(adj, (k * 7) % 37, (k * 3) % 29, (k * 41 % 256) as u8)
                    .unwrap();
            }
        }
        if i == 4 {
            d.set_layer_blend_mode(adj, BlendMode::Multiply).unwrap();
        }
    }
    let g = d.group_layers(&[ids[1], ids[2]], "G").unwrap();
    d.set_layer_opacity(g, 0.7, false).unwrap();
    d
}

#[test]
fn the_region_composite_equals_the_pixel_by_pixel_reference_in_every_channel() {
    let d = busy_document();
    for ch in d.channels() {
        let all = d.composite_channel(ch, d.bounds()).unwrap();
        for y in 0..d.height() {
            for x in 0..d.width() {
                let i = ((y * d.width() + x) * 4) as usize;
                assert_eq!(
                    &all[i..i + 4],
                    &d.composite_pixel(ch, x, y).unwrap().to_array(),
                    "{ch:?} ({x},{y})"
                );
            }
        }
        // 部分の領域も全体の一部と同じ（タイルの区切りを跨ぐ）
        let part = d.composite_channel(ch, Rect::new(5, 6, 20, 17)).unwrap();
        for y in 0..17usize {
            for x in 0..20usize {
                let i = ((y + 6) * 37 + x + 5) * 4;
                let j = (y * 20 + x) * 4;
                assert_eq!(&part[j..j + 4], &all[i..i + 4], "{ch:?} 領域 ({x},{y})");
            }
        }
    }
}

#[test]
fn the_composite_does_not_depend_on_the_thread_count() {
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let d = busy_document();
                d.channels()
                    .into_iter()
                    .map(|ch| hash(&d.composite_channel(ch, d.bounds()).unwrap()))
                    .collect::<Vec<_>>()
            })
    };
    let single = run(1);
    for threads in [2, 3, 8] {
        assert_eq!(run(threads), single, "スレッド {threads}");
    }
}

#[test]
fn a_clipped_adjustment_changes_only_the_layer_it_clips_to() {
    for (name, s) in kinds() {
        let mut d = small();
        let base = d.add_layer("base").unwrap();
        d.set_pixel(base, 3, 3, Rgba8::new(200, 40, 90, 255))
            .unwrap();
        let other = d.add_layer("other").unwrap();
        d.set_pixel(other, 9, 9, Rgba8::new(200, 40, 90, 255))
            .unwrap();
        // base の上（other の下）に置いたクリッピングの調整は base だけを変える
        let adj = d
            .add_adjustment_layer(name, s.clone(), None, Some(base))
            .unwrap();
        d.set_layer_clipping(adj, true).unwrap();
        let c = Rgba8::new(200, 40, 90, 255);
        assert_eq!(at(&d, 3, 3), s.apply(c), "{name}: クリップ先は変わる");
        assert_eq!(at(&d, 9, 9), c, "{name}: ほかのレイヤーは変わらない");
    }
}

#[test]
fn the_anchor_plan_applies_the_new_kinds_per_channel_kind() {
    use anchor::{Content, Layer, Plan};
    let curves = ToneCurves::identity()
        .with_curve(ToneChannel::Red, inverted_curve())
        .with_curve(ToneChannel::Composite, inverted_curve());
    let layers = vec![
        Layer::new(Content::Fill(Rgba8::new(40, 40, 40, 255))),
        Layer::new(Content::Adjustment(AdjustmentSettings::tone_curve(curves))),
    ];
    let color = Plan::new(&layers, 1, (1, 1), ChannelKind::Color).unwrap();
    assert_eq!(color.pixel(0, 0), Rgba8::new(40, 215, 215, 255));
    let scalar = Plan::new(&layers, 1, (1, 1), ChannelKind::Scalar).unwrap();
    assert_eq!(scalar.pixel(0, 0), Rgba8::new(215, 215, 215, 255));
    // 色だけの種類は、スカラーの計画では飛ばす（レイヤーごと入らない）
    let layers = vec![
        Layer::new(Content::Fill(Rgba8::new(40, 40, 40, 255))),
        Layer::new(Content::Adjustment(AdjustmentSettings::gradient_map(
            GradientMap::new(Ramp::default(), true),
        ))),
    ];
    let color = Plan::new(&layers, 1, (1, 1), ChannelKind::Color).unwrap();
    assert_eq!(color.pixel(0, 0), Rgba8::new(215, 215, 215, 255));
    let scalar = Plan::new(&layers, 1, (1, 1), ChannelKind::Scalar).unwrap();
    assert_eq!(scalar.pixel(0, 0), Rgba8::new(40, 40, 40, 255));
}

#[test]
fn filter_stages_of_the_new_kinds_follow_the_same_formulas_and_refuse_where_they_cannot_apply() {
    // レイヤーの内容のフィルターに置いた段は、同じ式で結果を変える
    for (name, s) in kinds() {
        let mut d = small();
        let l = d.add_layer("P").unwrap();
        let c = Rgba8::new(120, 90, 200, 255);
        d.set_pixel(l, 2, 2, c).unwrap();
        let effect = effect_of(&s);
        let index = effect.type_index();
        assert!((64..=69).contains(&index), "{name}: 段の種類は 64 から");
        d.add_filter(
            l,
            FilterTarget::Content,
            FilterSpec::new(effect.clone()).channels(&[Channel::Color]),
        )
        .unwrap();
        assert_eq!(at(&d, 2, 2), s.apply(c), "{name}");
        // 法線には置けない
        assert!(
            d.add_filter(
                l,
                FilterTarget::Content,
                FilterSpec::new(effect.clone()).channels(&[Channel::Normal])
            )
            .is_err(),
            "{name}: 法線"
        );
        // 色だけの種類はスカラー（Height）に置けない。ほかは置ける
        let on_height = d.add_filter(
            l,
            FilterTarget::Content,
            FilterSpec::new(effect.clone()).channels(&[Channel::Height]),
        );
        assert_eq!(
            on_height.is_ok(),
            s.applies_to(ChannelKind::Scalar),
            "{name}: スカラー"
        );
    }
}

#[test]
fn filter_stage_strength_blends_the_result_and_the_mask_stack_takes_only_the_scalar_kinds() {
    let mut d = small();
    let l = d.add_layer("P").unwrap();
    let c = Rgba8::new(120, 90, 200, 255);
    d.set_pixel(l, 2, 2, c).unwrap();
    let posterize = EffectSettings::posterize(Posterize::new(2).unwrap());
    d.add_filter(
        l,
        FilterTarget::Content,
        FilterSpec::new(posterize.clone())
            .channels(&[Channel::Color])
            .strength(0.5),
    )
    .unwrap();
    let full = AdjustmentSettings::posterize(Posterize::new(2).unwrap()).apply(c);
    let got = at(&d, 2, 2);
    for (g, (o, f)) in [
        (got.r, (c.r, full.r)),
        (got.g, (c.g, full.g)),
        (got.b, (c.b, full.b)),
    ] {
        let want = (f64::from(o) + (f64::from(f) - f64::from(o)) * 0.5 + 0.5).floor() as u8;
        assert_eq!(g, want);
    }
    // マスクのスタックには、スカラーに使える種類だけ
    let m = d.add_layer("M").unwrap();
    d.add_layer_mask(m).unwrap();
    for (name, s) in kinds() {
        let effect = effect_of(&s);
        let added = d.add_filter(m, FilterTarget::Mask, FilterSpec::new(effect));
        assert_eq!(
            added.is_ok(),
            s.applies_to(ChannelKind::Scalar),
            "{name}: マスク"
        );
    }
}

#[test]
fn the_filter_engine_runs_the_new_stages_within_budget_and_cancel_and_not_by_block() {
    use std::sync::atomic::AtomicBool;
    use yolu_core::filter::{self, Image, Options, Settings, Stage, ValueType};
    let (w, h) = (45u32, 31u32);
    let mut data = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let v = ((x * 11 + y * 23) % 256) as u8;
            data.extend_from_slice(&[v, 255 - v, (x * 5) as u8, 255]);
        }
    }
    let image = Image::new(&data, w, h).unwrap();
    let stack: Vec<Stage> = kinds()
        .into_iter()
        .map(|(_, s)| match effect_of(&s) {
            EffectSettings::Filter(f) => Stage::new(f),
            other => panic!("{other:?}"),
        })
        .collect();
    let region = Rect::new(0, 0, w, h);
    let reference = filter::evaluate(
        &image,
        ValueType::Color,
        &stack,
        region,
        &Options::default(),
    )
    .unwrap();
    // ブロックの大きさ・並列度・領域の取り方で変わらない
    for block in [1u32, 7, 16, 256] {
        let options = Options {
            block_size: block,
            ..Options::default()
        };
        assert_eq!(
            filter::evaluate(&image, ValueType::Color, &stack, region, &options).unwrap(),
            reference,
            "ブロック {block}"
        );
    }
    let part = filter::evaluate(
        &image,
        ValueType::Color,
        &stack,
        Rect::new(9, 4, 20, 13),
        &Options::default(),
    )
    .unwrap();
    for y in 0..13usize {
        for x in 0..20usize {
            let (i, j) = (((y + 4) * w as usize + x + 9) * 4, (y * 20 + x) * 4);
            assert_eq!(&part[j..j + 4], &reference[i..i + 4]);
        }
    }
    // 予算が足りなければ入力は触らず断る。取消の旗が立っていれば取り消す
    let tiny = Options {
        working_budget: 16,
        ..Options::default()
    };
    assert!(matches!(
        filter::evaluate(&image, ValueType::Color, &stack, region, &tiny),
        Err(filter::Error::Budget { .. })
    ));
    let flag = AtomicBool::new(true);
    let cancelled = Options {
        cancel: Some(&flag),
        ..Options::default()
    };
    assert_eq!(
        filter::evaluate(&image, ValueType::Color, &stack, region, &cancelled),
        Err(filter::Error::Cancelled)
    );
    // 強さ 0 の段は何もしない
    let off: Vec<Stage> = stack
        .iter()
        .cloned()
        .map(|mut s| {
            s.strength = 0.0;
            s
        })
        .collect();
    let untouched =
        filter::evaluate(&image, ValueType::Color, &off, region, &Options::default()).unwrap();
    assert_eq!(untouched, data);
    // 型の拒否: 法線・（色だけの種類は）スカラー
    for s in &stack {
        assert!(s.settings.validate(ValueType::TangentNormal).is_err());
        let color_only = matches!(
            s.settings,
            Settings::GradientMap(_) | Settings::ColorBalance(_)
        );
        assert_eq!(s.settings.validate(ValueType::Scalar).is_err(), color_only);
        assert_eq!(s.settings.validate(ValueType::Mask).is_err(), color_only);
        assert!(s.settings.validate(ValueType::Color).is_ok());
    }
}

#[test]
fn color_adjust_moves_between_layers_and_stages_without_loss() {
    use yolu_core::{AdjustmentType, ColorAdjust};
    for kind in [
        AdjustmentType::GradientMap,
        AdjustmentType::ToneCurve,
        AdjustmentType::ColorBalance,
        AdjustmentType::BrightnessContrast,
        AdjustmentType::Threshold,
        AdjustmentType::Posterize,
    ] {
        let ca = ColorAdjust::default_for(kind).unwrap();
        assert_eq!(ca.kind(), kind);
        let settings = ca.clone().into_settings();
        assert_eq!(settings.kind(), kind);
        assert_eq!(settings.color_adjust(), Some(ca.clone()));
        let stage = EffectSettings::from_color_adjust(ca.clone());
        assert_eq!(stage.color_adjust(), Some(ca.clone()));
        assert_eq!(stage.type_index(), kind as i32);
        assert_eq!(ca.byte_size() + 64, settings.byte_size());
        assert!(!ca.applies_to(ChannelKind::Normal));
    }
    for kind in [
        AdjustmentType::Invert,
        AdjustmentType::Levels,
        AdjustmentType::HueSaturation,
    ] {
        assert!(ColorAdjust::default_for(kind).is_none());
    }
    assert!(AdjustmentSettings::invert().color_adjust().is_none());
    assert!(EffectSettings::invert().color_adjust().is_none());
    assert!(EffectSettings::blur(3).color_adjust().is_none());
}

#[test]
fn composite_below_is_what_the_layers_under_an_adjustment_make() {
    let mut d = busy_document();
    let all = d.bounds();
    // 一番上に足した調整レイヤーの下の合成は、そのレイヤーを足す前の文書の合成と同じ（全チャンネル）
    let before: Vec<Vec<u8>> = d
        .channels()
        .into_iter()
        .map(|c| d.composite_channel(c, all).unwrap())
        .collect();
    let (name, s) = kinds().remove(1);
    let adj = d.add_adjustment_layer(name, s, None, None).unwrap();
    for (c, want) in d.channels().into_iter().zip(&before) {
        assert_eq!(&d.composite_below(adj, c, all).unwrap(), want, "{c:?}");
    }
    // 調整を含む合成は、下だけの合成と違う（Color）
    assert_ne!(d.composite_channel(Channel::Color, all).unwrap(), before[0]);
    // 一番下のレイヤーの下は何も無い（透明）。部分の領域も合成と同じ
    let bottom = d.layers()[0].id();
    assert!(d
        .composite_below(bottom, Channel::Color, all)
        .unwrap()
        .iter()
        .all(|b| *b == 0));
    let part = Rect::new(5, 6, 20, 17);
    let below = d.composite_below(adj, Channel::Color, part).unwrap();
    let full = d.composite_below(adj, Channel::Color, all).unwrap();
    for y in 0..17usize {
        for x in 0..20usize {
            let (i, j) = (((y + 6) * 37 + x + 5) * 4, (y * 20 + x) * 4);
            assert_eq!(&below[j..j + 4], &full[i..i + 4]);
        }
    }
    assert!(d
        .composite_below(LayerId(0xdead), Channel::Color, part)
        .is_err());
}

#[test]
fn a_filter_stage_edit_counts_what_the_stage_carries_in_the_history() {
    // 段を 1 つ足す編集の大きさ（前は空のスタック）: 64 + 96 × 段の数 + 色調補正の段の中身の大きさ
    let cost = |effect: EffectSettings| {
        let mut d = small();
        let l = d.add_layer("P").unwrap();
        d.clear_history().unwrap();
        d.add_filter(
            l,
            FilterTarget::Content,
            FilterSpec::new(effect).channels(&[Channel::Color]),
        )
        .unwrap();
        d.history_bytes()
    };
    assert_eq!(cost(EffectSettings::invert()), 64 + 96);
    for (name, s) in kinds() {
        let carried = effect_of(&s).color_adjust().unwrap().byte_size();
        assert_eq!(cost(effect_of(&s)), 64 + 96 + carried, "{name}");
    }
}
