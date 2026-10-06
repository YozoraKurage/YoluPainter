//! 合成済みの参照元と、写像されたダブ（Unity 版の C# の Core の BrushSourceTests。値は C# の試験の期待値そのもの）。
//!
//! 移していないもの: アンカーが作るマスクを、自分のマテリアルの書き込みの前に凍結する試験（CompositeCloneFreezesAnchorDrivenMasks…）。
//! 層に付く Filter・Generator・Anchor が `Document` にまだ無いので、合成を変える相手がいない（評価器は `filter`・`generator` に
//! ある）。層に付けられるようになったら、ここへ足す。

use std::sync::Arc;

use yolu_core::glam::DVec2;
use yolu_core::{
    material::ChannelPaint, Brush, BrushEffect, BrushMappedPixel, BrushPixel, BrushSample,
    BrushSettings, BrushSourceTap, BrushStencil, Channel, CoreError, Document, ImageColorSpace,
    LayerId, Rgba8, SelectionMask, StencilImage, StencilMode, StencilPoint, StencilTiling,
};

fn clone_brush() -> Brush {
    effect_brush(BrushEffect::Clone {
        offset: DVec2::ZERO,
    })
}
fn effect_brush(effect: BrushEffect) -> Brush {
    Brush {
        effect,
        ..Brush::from(BrushSettings {
            radius: 0.5,
            hardness: 1.0,
            spacing: 1.0,
            pressure_size: false,
            pressure_opacity: false,
            ..BrushSettings::default()
        })
    }
}
fn smudge() -> BrushEffect {
    BrushEffect::Smudge { strength: 1.0 }
}
/// 画素 (x, 0) を、画素 (source, 0) から写す（重み 1）。
fn m(x: i64, source: i64) -> BrushMappedPixel {
    BrushMappedPixel::single(
        BrushPixel {
            x,
            y: 0,
            coverage: 1.0,
        },
        source,
        0,
    )
}
/// 8 × 4 の画布の 1 行目に、R が 20 ずつ増える画素を置いた「paint」層。
fn make(tile: u32) -> (Document, LayerId) {
    let mut d = Document::with_tile_size(8, 4, tile).unwrap();
    let layer = d.add_layer("paint").unwrap();
    for x in 0..8 {
        d.set_pixel(layer, x, 0, Rgba8::new(20 * x as u8, 30, 50, 255))
            .unwrap();
    }
    d.clear_history().unwrap();
    (d, layer)
}
/// 文書の全層・全チャンネルの有効の印と画素（C# の DocumentBinary.Write の代わり）。
fn state(d: &Document) -> Vec<(u128, Channel, bool, Option<Vec<u8>>)> {
    let mut v = Vec::new();
    for layer in d.layers() {
        for c in d.channels() {
            v.push((
                layer.id().0,
                c,
                layer.is_channel_enabled(c),
                layer.surface(c).map(|s| s.to_canvas_bytes()),
            ));
        }
        v.push((
            layer.id().0,
            Channel::Color,
            layer.mask().is_some(),
            layer.mask().map(|m| m.surface().to_canvas_bytes()),
        ));
    }
    v
}
fn pixel(d: &Document, l: LayerId, c: Channel, x: u32, y: u32) -> Rgba8 {
    d.layer(l).unwrap().pixel(c, x, y).unwrap()
}

#[test]
fn composite_clone_reads_frozen_visible_stack_and_round_trips_one_undo() {
    for mapped in [false, true] {
        let (mut d, _layer) = make(8);
        let top = d
            .add_fill_layer(
                "fill",
                &[(Channel::Color, Rgba8::new(90, 200, 20, 120))],
                None,
            )
            .unwrap();
        d.set_layer_opacity(top, 0.6, false).unwrap();
        d.add_layer_mask(top).unwrap();
        d.set_mask_pixel(top, 0, 0, 130).unwrap();
        let hidden = d
            .add_fill_layer(
                "hidden",
                &[(Channel::Color, Rgba8::new(255, 0, 0, 255))],
                None,
            )
            .unwrap();
        d.set_layer_visible(hidden, false).unwrap();
        let target = d.add_layer("clone").unwrap();
        d.clear_history().unwrap();
        let before = state(&d);
        let composite = d.composite(d.bounds()).unwrap();
        let brush = effect_brush(BrushEffect::Clone {
            offset: DVec2::new(-1.0, 0.0),
        });
        let mut stroke = d.begin_brush_stroke(target, &brush).unwrap();
        stroke.use_composite_clone_source(&mut d).unwrap();
        assert!(d.clone_source_bytes() > 0);
        if mapped {
            stroke
                .apply_mapped_dab(&mut d, &[m(1, 0)], 1.0, 0, None)
                .unwrap();
            stroke
                .apply_mapped_dab(&mut d, &[m(2, 1)], 1.0, 0, None)
                .unwrap();
        } else {
            stroke
                .add_sample(
                    &mut d,
                    BrushSample::new(1.5, 0.5, 1.0, 0.0, DVec2::ZERO).unwrap(),
                )
                .unwrap();
            stroke
                .add_sample(
                    &mut d,
                    BrushSample::new(2.5, 0.5, 1.0, 1.0, DVec2::ZERO).unwrap(),
                )
                .unwrap();
        }
        d.end_stroke(stroke).unwrap();
        assert_eq!(d.clone_source_bytes(), 0, "確定で参照元を手放す");
        assert!(d.active_stroke_stats().is_none());
        for x in 1..=2u32 {
            let i = (x as usize - 1) * 4;
            assert_eq!(
                pixel(&d, target, Channel::Color, x, 0),
                Rgba8::new(
                    composite[i],
                    composite[i + 1],
                    composite[i + 2],
                    composite[i + 3]
                ),
                "mapped={mapped} x={x}"
            );
        }
        let after = state(&d);
        assert_eq!(d.undo_count(), 1);
        d.undo().unwrap();
        assert_eq!(state(&d), before);
        d.redo().unwrap();
        assert_eq!(state(&d), after);
    }
}

#[test]
fn mapped_dab_freezes_every_read_before_writes_and_clones_keep_stroke_start() {
    for effect in [
        BrushEffect::Clone {
            offset: DVec2::ZERO,
        },
        smudge(),
    ] {
        let (mut d, layer) = make(4);
        let before = state(&d);
        let mut stroke = d.begin_brush_stroke(layer, &effect_brush(effect)).unwrap();
        stroke
            .apply_mapped_dab(&mut d, &[m(1, 0), m(2, 1)], 1.0, 0, None)
            .unwrap();
        assert_eq!(
            pixel(&d, layer, Channel::Color, 2, 0).r,
            20,
            "先に変えた画素を同じダブで読まない"
        );
        stroke
            .apply_mapped_dab(&mut d, &[m(3, 1)], 1.0, 0, None)
            .unwrap();
        assert_eq!(
            pixel(&d, layer, Channel::Color, 3, 0).r,
            if matches!(effect, BrushEffect::Clone { .. }) {
                20
            } else {
                0
            },
            "クローンはストロークの始めの画素を、指先は今の画素を読む"
        );
        d.cancel_stroke(stroke);
        assert!(d.active_stroke_stats().is_none());
        assert_eq!(state(&d), before);
        assert_eq!(d.undo_count(), 0);
    }
}

#[test]
fn mapped_taps_use_independent_premultiplied_equation_and_selection() {
    let mut d = Document::with_tile_size(8, 4, 4).unwrap();
    let layer = d.add_layer("paint").unwrap();
    d.set_pixel(layer, 0, 0, Rgba8::new(10, 20, 30, 255))
        .unwrap();
    d.set_pixel(layer, 1, 0, Rgba8::new(110, 120, 130, 85))
        .unwrap();
    d.set_pixel(layer, 2, 0, Rgba8::new(255, 0, 0, 0)).unwrap();
    let mut s = d.begin_brush_stroke(layer, &clone_brush()).unwrap();
    let pixel4 = BrushPixel {
        x: 4,
        y: 0,
        coverage: 1.0,
    };
    s.apply_mapped_dab(
        &mut d,
        &[BrushMappedPixel::new(
            pixel4,
            BrushSourceTap::new(0, 0, 0.25),
            BrushSourceTap::new(1, 0, 0.5),
            BrushSourceTap::new(2, 0, 0.25),
            BrushSourceTap::NONE,
        )],
        1.0,
        0,
        None,
    )
    .unwrap();
    d.end_stroke(s).unwrap();
    // a = 255/4 + 85/2 = 106.25、RGB = (10 * 63.75 + 110 * 42.5) / 106.25 など。
    assert_eq!(
        pixel(&d, layer, Channel::Color, 4, 0),
        Rgba8::new(50, 60, 70, 106)
    );
    let select = SelectionMask::rectangle(&d, 5, 0, 6, 1);
    d.set_selection(Some(select)).unwrap();
    let mut s = d.begin_brush_stroke(layer, &clone_brush()).unwrap();
    s.apply_mapped_dab(&mut d, &[m(5, 0), m(6, 0)], 1.0, 0, None)
        .unwrap();
    d.end_stroke(s).unwrap();
    assert_eq!(
        pixel(&d, layer, Channel::Color, 5, 0),
        pixel(&d, layer, Channel::Color, 0, 0)
    );
    assert_eq!(
        pixel(&d, layer, Channel::Color, 6, 0),
        Rgba8::TRANSPARENT,
        "選択の外は変わらない"
    );
}

/// 1 つの面に Color と Height を持つ層へ、クローンの複数チャンネルのストロークを始める。
fn material_stroke(d: &mut Document, l: LayerId) -> yolu_core::Stroke {
    d.begin_material_brush_stroke(
        l,
        &[
            ChannelPaint::new(Channel::Color, Rgba8::TRANSPARENT),
            ChannelPaint::new(Channel::Height, Rgba8::TRANSPARENT),
        ],
        &clone_brush(),
    )
    .unwrap()
}

#[test]
fn sampling_budgets_cancel_pixels_and_enabled_channels() {
    for phase in ["source", "mapped", "chart"] {
        let (mut d, layer) = make(4);
        let before = state(&d);
        d.set_stroke_budget_bytes(if phase == "source" { 1 } else { 500 })
            .unwrap();
        let mut s = material_stroke(&mut d, layer);
        let r = match phase {
            "source" => s.use_composite_clone_source(&mut d).map(|_| ()),
            "chart" => s
                .apply_mapped_dab(&mut d, &[m(4, 0)], 1.0, 1000, None)
                .map(|_| ()),
            _ => s
                .apply_mapped_dab(&mut d, &[m(1, 0)], 1.0, 0, None)
                .and_then(|_| s.apply_mapped_dab(&mut d, &[m(5, 1)], 1.0, 0, None))
                .map(|_| ()),
        };
        assert_eq!(r, Err(CoreError::StrokeBudgetExceeded), "{phase}");
        assert!(!d.has_active_stroke(), "{phase}");
        assert_eq!(d.clone_source_bytes(), 0);
        assert_eq!(
            state(&d),
            before,
            "{phase}: 画素と、ストロークが有効にしたチャンネルが戻る"
        );
        assert_eq!(d.undo_count(), 0);
    }
}

#[test]
fn invalid_mapped_input_cancels_the_entire_stroke() {
    for kind in ["duplicate", "nan", "outside", "no-source"] {
        let (mut d, layer) = make(4);
        let before = state(&d);
        let mut s = d.begin_brush_stroke(layer, &clone_brush()).unwrap();
        s.apply_mapped_dab(&mut d, &[m(1, 0)], 1.0, 0, None)
            .unwrap();
        let at2 = BrushPixel {
            x: 2,
            y: 0,
            coverage: 1.0,
        };
        let invalid = match kind {
            "nan" => BrushMappedPixel::new(
                at2,
                BrushSourceTap::new(0, 0, f64::NAN),
                BrushSourceTap::NONE,
                BrushSourceTap::NONE,
                BrushSourceTap::NONE,
            ),
            "outside" => m(2, -1),
            "no-source" => BrushMappedPixel::new(
                at2,
                BrushSourceTap::NONE,
                BrushSourceTap::NONE,
                BrushSourceTap::NONE,
                BrushSourceTap::NONE,
            ),
            _ => m(2, 0),
        };
        let plan = if kind == "duplicate" {
            vec![invalid, invalid]
        } else {
            vec![invalid]
        };
        let r = s.apply_mapped_dab(&mut d, &plan, 1.0, 0, None);
        assert!(
            matches!(r, Err(CoreError::InvalidArgument(_))),
            "{kind}: {r:?}"
        );
        assert!(!d.has_active_stroke());
        assert_eq!(state(&d), before, "{kind}");
    }
}

#[test]
fn wrong_source_types_and_late_composite_setup_refuse_without_leaving_a_stroke() {
    let (mut d, layer) = make(4);
    d.add_layer_mask(layer).unwrap();
    d.clear_history().unwrap();
    let before = state(&d);
    let mut s = d.begin_brush_mask_stroke(layer, &clone_brush()).unwrap();
    assert!(matches!(
        s.use_composite_clone_source(&mut d),
        Err(CoreError::Unsupported(_))
    ));
    assert!(!d.has_active_stroke());
    // 色を塗るブラシ（混ぜない）は写像されたダブを受けない（ぼかしは 3D の島の縁で受ける）
    let mut s = d
        .begin_brush_stroke(layer, &effect_brush(BrushEffect::Paint))
        .unwrap();
    assert!(matches!(
        s.apply_mapped_dab(&mut d, &[m(1, 0)], 1.0, 0, None),
        Err(CoreError::Unsupported(_))
    ));
    assert!(!d.has_active_stroke());
    let mut s = d.begin_brush_stroke(layer, &clone_brush()).unwrap();
    s.apply_mapped_dab(&mut d, &[m(1, 0)], 1.0, 0, None)
        .unwrap();
    assert!(matches!(
        s.use_composite_clone_source(&mut d),
        Err(CoreError::Unsupported(_))
    ));
    assert!(!d.has_active_stroke());
    // 描く前の効果の入力があっても、最初のダブの後は受けない
    let mut s = d.begin_brush_stroke(layer, &clone_brush()).unwrap();
    s.add_sample(
        &mut d,
        BrushSample::new(2.5, 0.5, 1.0, 0.0, DVec2::ZERO).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        s.use_composite_clone_source(&mut d),
        Err(CoreError::Unsupported(_))
    ));
    assert!(!d.has_active_stroke());
    assert_eq!(state(&d), before);
    // 取り消された札は、もう何も受けない
    assert_eq!(
        s.apply_mapped_dab(&mut d, &[m(1, 0)], 1.0, 0, None),
        Err(CoreError::NoActiveStroke)
    );
}

#[test]
fn composite_clone_freezes_each_material_channel_separately_and_cancels_all() {
    let (mut d, _) = make(4);
    let channels = [Channel::Color, Channel::Height, Channel::Normal];
    let values: Vec<(Channel, Rgba8)> = channels
        .iter()
        .enumerate()
        .map(|(i, c)| (*c, Rgba8::new(50 + i as u8 * 50, 90, 200, 255)))
        .collect();
    d.add_fill_layer("fill", &values, None).unwrap();
    let target = d.add_layer("clone").unwrap();
    let source: Vec<Rgba8> = channels
        .iter()
        .map(|c| d.composite_pixel(*c, 0, 0).unwrap())
        .collect();
    d.clear_history().unwrap();
    let before = state(&d);
    let paints: Vec<ChannelPaint> = channels
        .iter()
        .map(|c| ChannelPaint::new(*c, Rgba8::TRANSPARENT))
        .collect();
    let mut s = d
        .begin_material_brush_stroke(target, &paints, &clone_brush())
        .unwrap();
    s.use_composite_clone_source(&mut d).unwrap();
    s.apply_mapped_dab(&mut d, &[m(2, 0)], 1.0, 0, None)
        .unwrap();
    for (c, expected) in channels.iter().zip(&source) {
        assert_eq!(pixel(&d, target, *c, 2, 0), *expected, "{c:?}");
    }
    d.cancel_stroke(s);
    assert_eq!(state(&d), before);
    assert_eq!(d.undo_count(), 0);
}

#[test]
fn merged_mapped_clone_source_growth_refusal_releases_the_snapshot_and_rolls_back_setup() {
    let (mut d, _) = make(4);
    let target = d.add_layer("clone").unwrap();
    d.clear_history().unwrap();
    let before = state(&d);
    d.set_source_budget_bytes(d.allocated_bytes()).unwrap();
    let mut s = material_stroke(&mut d, target);
    s.use_composite_clone_source(&mut d).unwrap();
    assert!(d.clone_source_bytes() > 0);
    let r = s.apply_mapped_dab(&mut d, &[m(4, 0)], 1.0, 0, None);
    assert_eq!(r, Err(CoreError::SourceBudgetExceeded));
    assert!(!d.has_active_stroke());
    assert_eq!(d.clone_source_bytes(), 0, "参照元を手放す");
    assert_eq!(state(&d), before);
    assert_eq!(d.undo_count(), 0);
}

#[test]
fn sparse_composite_clone_snapshot_scales_with_content_tiles_and_releases_its_payload() {
    let mut d = Document::with_tile_size(4096, 4096, 32).unwrap();
    let layer = d.add_layer("paint").unwrap();
    d.set_stroke_budget_bytes(8192).unwrap();
    let mut s = d.begin_brush_stroke(layer, &clone_brush()).unwrap();
    s.use_composite_clone_source(&mut d).unwrap();
    assert_eq!(
        d.clone_source_bytes(),
        0,
        "画素のある層が無ければ何も写さない"
    );
    d.cancel_stroke(s);
    d.set_pixel(layer, 2048, 2048, Rgba8::new(210, 30, 70, 255))
        .unwrap();
    let mut s = d.begin_brush_stroke(layer, &clone_brush()).unwrap();
    s.use_composite_clone_source(&mut d).unwrap();
    assert_eq!(
        d.clone_source_bytes(),
        4160,
        "32 角の RGBA8 の参照 1 枚 4096 バイトと、名目の索引 64 バイト"
    );
    assert_eq!(d.active_stroke_stats().unwrap().rollback_bytes, 4160);
    d.cancel_stroke(s);
    assert!(!d.has_active_stroke());
    assert_eq!(d.clone_source_bytes(), 0);
    // 画素のあるタイルが予算に入らないほど多ければ断る
    d.set_stroke_budget_bytes(4000).unwrap();
    let mut s = d.begin_brush_stroke(layer, &clone_brush()).unwrap();
    assert_eq!(
        s.use_composite_clone_source(&mut d),
        Err(CoreError::StrokeBudgetExceeded)
    );
    assert!(!d.has_active_stroke());
}

#[test]
fn mapped_effects_use_the_destination_stencil_amount_and_undo_exactly() {
    for effect in [
        BrushEffect::Clone {
            offset: DVec2::ZERO,
        },
        smudge(),
    ] {
        let (mut d, layer) = make(8);
        let before = state(&d);
        // 2 × 1 の画像: 左が白（量 1）、右が黒（量 0）
        let mut px = vec![255u8; 4];
        px.extend_from_slice(&[0, 0, 0, 255]);
        let image = Arc::new(
            StencilImage::new(
                2,
                1,
                px,
                ImageColorSpace::Srgb,
                StencilImage::DEFAULT_MIP_BUDGET_BYTES,
            )
            .unwrap(),
        );
        let mut brush = effect_brush(effect);
        brush.stencil = Some(Arc::new(BrushStencil::new(
            image,
            StencilMode::Mask,
            StencilTiling::default(),
            false,
            None,
            &[],
        )));
        let mut s = d.begin_brush_stroke(layer, &brush).unwrap();
        let plan = [m(5, 1), m(6, 1)];
        let points = [
            StencilPoint::new(0.5, 0.5, 0.0).unwrap(),
            StencilPoint::new(1.5, 0.5, 0.0).unwrap(),
        ];
        s.apply_mapped_dab(&mut d, &plan, 1.0, 0, Some(&points))
            .unwrap();
        s.apply_mapped_dab(&mut d, &plan, 1.0, 0, Some(&points))
            .unwrap();
        d.end_stroke(s).unwrap();
        assert_eq!(
            pixel(&d, layer, Channel::Color, 5, 0),
            Rgba8::new(20, 30, 50, 255),
            "白の所だけ参照を写す"
        );
        assert_eq!(
            pixel(&d, layer, Channel::Color, 6, 0),
            Rgba8::new(120, 30, 50, 255),
            "黒の所は変えない"
        );
        assert_eq!(d.undo_count(), 1);
        let after = state(&d);
        d.undo().unwrap();
        assert_eq!(state(&d), before);
        d.redo().unwrap();
        assert_eq!(state(&d), after);
        // 点の数が画素と合わなければ、ストロークごと取り消す
        let mut s = d.begin_brush_stroke(layer, &brush).unwrap();
        s.apply_mapped_dab(&mut d, &[m(4, 1)], 1.0, 0, Some(&points[..1]))
            .unwrap();
        let r = s.apply_mapped_dab(&mut d, &[m(7, 1)], 1.0, 0, Some(&[]));
        assert!(matches!(r, Err(CoreError::InvalidArgument(_))));
        assert!(!d.has_active_stroke());
        assert_eq!(d.active_stroke_stats(), None);
        assert_eq!(state(&d), after);
    }
}

/// 参照の読み・凍結した合成・ダブの塗りは、タイルの大きさとスレッドの数によらず同じ画素になる（合成は並列、塗りは逐次）。
#[test]
fn mapped_dabs_and_composite_sources_are_identical_across_tile_sizes_and_thread_counts() {
    let run = |tile: u32, threads: usize, composite: bool, effect: BrushEffect| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let mut d = Document::with_tile_size(40, 24, tile).unwrap();
                let below = d.add_layer("below").unwrap();
                for y in 0..24 {
                    for x in 0..40 {
                        d.set_pixel(
                            below,
                            x,
                            y,
                            Rgba8::new((x * 6) as u8, (y * 10) as u8, ((x + y) * 3) as u8, 255),
                        )
                        .unwrap();
                    }
                }
                let fill = d
                    .add_fill_layer(
                        "fill",
                        &[(Channel::Color, Rgba8::new(10, 220, 40, 90))],
                        None,
                    )
                    .unwrap();
                d.set_layer_opacity(fill, 0.5, false).unwrap();
                // 合成の参照元は、別の層（空）へ見えている重なりを写す。そうでなければ、模様のある層そのものを読む
                let target = if composite {
                    d.add_layer("target").unwrap()
                } else {
                    below
                };
                d.clear_history().unwrap();
                let mut s = d.begin_brush_stroke(target, &effect_brush(effect)).unwrap();
                if composite {
                    s.use_composite_clone_source(&mut d).unwrap();
                }
                // 画素 (x, y) を、左へ 7・下へ 2 の画素から、重み付きの 4 点で読む
                let plan: Vec<BrushMappedPixel> = (8..36)
                    .flat_map(|x| (4..20).map(move |y| (x, y)))
                    .map(|(x, y)| {
                        BrushMappedPixel::new(
                            BrushPixel {
                                x,
                                y,
                                coverage: 0.25 + ((x + y) % 4) as f64 * 0.25,
                            },
                            BrushSourceTap::new(x - 7, y - 2, 0.4),
                            BrushSourceTap::new(x - 6, y - 2, 0.3),
                            BrushSourceTap::new(x - 7, y - 1, 0.2),
                            BrushSourceTap::new(x - 6, y - 1, 0.1),
                        )
                    })
                    .collect();
                for chunk in plan.chunks(97) {
                    s.apply_mapped_dab(&mut d, chunk, 0.8, 0, None).unwrap();
                }
                d.end_stroke(s).unwrap();
                d.layer(target)
                    .unwrap()
                    .surface(Channel::Color)
                    .unwrap()
                    .to_canvas_bytes()
            })
    };
    for effect in [
        BrushEffect::Clone {
            offset: DVec2::ZERO,
        },
        smudge(),
    ] {
        for composite in [false, true] {
            if composite && !matches!(effect, BrushEffect::Clone { .. }) {
                continue; // 合成の参照元はクローンだけ
            }
            let expected = run(8, 1, composite, effect);
            assert!(expected.chunks(4).any(|p| p[3] > 0));
            for (tile, threads) in [(4, 1), (16, 4), (40, 3), (5, 2)] {
                assert!(
                    run(tile, threads, composite, effect) == expected,
                    "{effect:?} composite={composite} tile={tile} threads={threads}"
                );
            }
        }
    }
}
