use yolu_core::{BlendMode, BrushSettings, Channel, Document, Rgba8, TileCoord};
#[path = "support/gpu_lease.rs"]
mod gpu_lease;
#[path = "support/require_gpu.rs"]
mod require_gpu;
use yolu_gpu::{Compositor, Dab, GpuPainter, Options};
fn gpu() -> Option<GpuPainter> {
    gpu_lease::lease();
    match GpuPainter::new(Options::default()) {
        Ok(g) => {
            eprintln!("GPU 試験: {:?}", g.adapter_info());
            Some(g)
        }
        Err(e) => {
            assert!(
                e.to_string().starts_with("GPU 利用不可:"),
                "GPU 実装の初期化失敗: {e}"
            );
            require_gpu::skipped("GPU 試験", &e.to_string());
            None
        }
    }
}
fn coords(d: &Document) -> Vec<TileCoord> {
    (0..d.height().div_ceil(d.tile_size()))
        .flat_map(|y| (0..d.width().div_ceil(d.tile_size())).map(move |x| TileCoord::new(x, y)))
        .collect()
}
fn diff(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0)
}
fn compare(g: &mut GpuPainter, d: &Document, c: &[TileCoord], tolerance: u8) -> u8 {
    let r = g.composite_tiles(d, Channel::Color, c).unwrap();
    assert!(r.is_current(d));
    let mut max = 0;
    for t in r.tiles {
        max = max.max(diff(&d.composite(t.rect).unwrap(), &t.pixels));
    }
    assert!(max <= tolerance, "最大差 {max} > {tolerance}");
    max
}
#[test]
fn all_modes_stack_and_clip() {
    let Some(mut g) = gpu() else { return };
    for mode in BlendMode::LAYER_MODES {
        let mut worst = 0;
        for clip in [false, true] {
            for opacity in [0.0, 0.37, 0.85, 1.0] {
                let mut d = Document::with_tile_size(67, 35, 16).unwrap();
                let a = d.add_layer("下地").unwrap();
                let b = d.add_layer("上").unwrap();
                let mut seed = 0x12345678u32;
                let mut byte = || {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    seed as u8
                };
                for y in 0..35 {
                    for x in 0..67 {
                        let alpha = [0, 1, 90, 120, 254, 255][(x % 6) as usize];
                        d.set_pixel(a, x, y, Rgba8::new(byte(), byte(), byte(), alpha))
                            .unwrap();
                        d.set_pixel(
                            b,
                            x,
                            y,
                            Rgba8::new(byte(), byte(), byte(), [0, 1, 90, 255][(y % 4) as usize]),
                        )
                        .unwrap();
                    }
                }
                d.set_layer_blend_mode(b, mode).unwrap();
                d.set_layer_opacity(b, opacity, false).unwrap();
                d.set_layer_clipping(b, clip).unwrap();
                worst = worst.max(compare(&mut g, &d, &coords(&d), 1));
            }
        }
        eprintln!("合成 {mode:?}: 最大差 {worst}");
    }
}
#[test]
fn exact_copy_hidden_base_and_incremental_undo() {
    let Some(mut g) = gpu() else { return };
    let mut d = Document::new(257, 129).unwrap();
    let a = d.add_layer("下地").unwrap();
    let b = d.add_layer("クリップ").unwrap();
    d.set_pixel(a, 256, 128, Rgba8::new(13, 45, 87, 255))
        .unwrap();
    d.set_pixel(b, 256, 128, Rgba8::new(255, 20, 60, 255))
        .unwrap();
    d.set_layer_clipping(b, true).unwrap();
    compare(&mut g, &d, &coords(&d), 0);
    d.set_layer_visible(a, false).unwrap();
    compare(&mut g, &d, &coords(&d), 0);
    d.set_layer_visible(a, true).unwrap();
    d.set_layer_opacity(a, 0.0, false).unwrap();
    compare(&mut g, &d, &coords(&d), 0);
    d.set_layer_opacity(a, 1.0, false).unwrap();
    let old = g.composite_tiles(&d, Channel::Color, &coords(&d)).unwrap();
    let serial = d.change_serial();
    let brush = BrushSettings {
        radius: 3.0,
        hardness: 1.0,
        pressure_size: false,
        pressure_opacity: false,
        ..Default::default()
    };
    let mut s = d.begin_stroke(a, &brush).unwrap();
    s.add_point(&mut d, 20.0, 20.0, 1.0, yolu_core::glam::DVec2::ZERO)
        .unwrap();
    assert!(!old.is_current(&d));
    let changed = d.changed_tiles(Channel::Color, serial).unwrap();
    assert_eq!(changed, vec![TileCoord::new(0, 0)]);
    compare(&mut g, &d, &changed, 0);
    d.cancel_stroke(s);
    compare(&mut g, &d, &changed, 0);
    let mut s = d.begin_stroke(a, &brush).unwrap();
    s.add_point(&mut d, 20.0, 20.0, 1.0, yolu_core::glam::DVec2::ZERO)
        .unwrap();
    d.end_stroke(s).unwrap();
    compare(&mut g, &d, &changed, 0);
    d.undo().unwrap();
    compare(&mut g, &d, &changed, 0);
    d.redo().unwrap();
    compare(&mut g, &d, &changed, 0);
    let empty = Document::new(17, 19).unwrap();
    compare(&mut g, &empty, &coords(&empty), 0);
}
#[test]
fn brush_matches_core_accumulation_and_erase() {
    let Some(mut g) = gpu() else { return };
    for erase in [false, true] {
        for hardness in [0.0, 0.4, 1.0] {
            for pressure_flags in [false, true] {
                let mut d = Document::with_tile_size(67, 35, 16).unwrap();
                let a = d.add_layer("ブラシ").unwrap();
                let mut start = Vec::new();
                for y in 0..35 {
                    for x in 0..67 {
                        let c = Rgba8::new(
                            (x * 3) as u8,
                            (y * 7) as u8,
                            137,
                            [0, 1, 128, 255][(x % 4) as usize],
                        );
                        d.set_pixel(a, x, y, c).unwrap();
                        start.extend(c.to_array());
                    }
                }
                let settings = BrushSettings {
                    radius: 12.3,
                    hardness,
                    opacity: 0.7,
                    flow: 0.6,
                    color: Rgba8::new(51, 153, 204, 180),
                    erase,
                    pressure_size: pressure_flags,
                    pressure_flow: pressure_flags,
                    pressure_opacity: pressure_flags,
                    ..Default::default()
                };
                let dabs: Vec<_> = (0..15)
                    .map(|i| Dab {
                        x: 3.25 + i as f64 * 2.5,
                        y: 15.75,
                        pressure: [0.0, 0.1, 0.5, 1.0][i % 4],
                    })
                    .collect();
                let mut stroke = d.begin_stroke(a, &settings).unwrap();
                // ダブの補間を介さず、core の ApplyPixel に同じ丸い覆いを渡す。
                for dab in &dabs {
                    let radius = settings.radius
                        * if settings.pressure_size {
                            dab.pressure
                        } else {
                            1.0
                        };
                    if radius <= 0.0 {
                        continue;
                    }
                    for y in 0..35 {
                        for x in 0..67 {
                            let dx = x as f64 + 0.5 - dab.x;
                            let dy = y as f64 + 0.5 - dab.y;
                            let distance = (dx * dx + dy * dy).sqrt() / radius;
                            if distance > 1.0 {
                                continue;
                            }
                            let coverage = if distance <= hardness {
                                1.0
                            } else {
                                let t = (1.0 - distance) / (1.0 - hardness);
                                t * t * (3.0 - 2.0 * t)
                            };
                            stroke
                                .apply_pixel(&mut d, x, y, coverage, dab.pressure)
                                .unwrap();
                        }
                    }
                }
                d.end_stroke(stroke).unwrap();
                let expected = d
                    .layer(a)
                    .unwrap()
                    .surface(Channel::Color)
                    .unwrap()
                    .to_canvas_bytes();
                let actual = g.brush_dabs(67, 35, &start, &settings, &dabs).unwrap();
                let max = diff(&expected, &actual);
                eprintln!("ブラシ erase={erase} hardness={hardness} pressure={pressure_flags}: 最大差 {max}");
                assert!(max <= 1);
                assert_eq!(g.brush_dabs(67, 35, &start, &settings, &[]).unwrap(), start);
            }
        }
    }
}
#[test]
fn invalid_input_budget_and_cpu_fallback() {
    let Some(mut g) = gpu() else { return };
    let d = Document::new(32, 32).unwrap();
    assert!(g
        .brush_dabs(u32::MAX, u32::MAX, &[], &BrushSettings::default(), &[])
        .is_err());
    // 縁のアンチエイリアスは core だけ: 段のあるブラシは断る（今の式で黙って描かない）
    for level in [
        yolu_core::AntiAlias::Weak,
        yolu_core::AntiAlias::Medium,
        yolu_core::AntiAlias::Strong,
    ] {
        let s = BrushSettings {
            anti_alias: level,
            ..BrushSettings::default()
        };
        assert!(g.brush_dabs(1, 1, &[0; 4], &s, &[]).is_err());
    }
    // 文書にないチャンネルは断る（法線の種類のチャンネルは合成できる）
    assert!(g
        .composite_tiles(
            &d,
            Channel::from_index(40).unwrap(),
            &[TileCoord::new(0, 0)]
        )
        .is_err());
    assert!(g
        .composite_tiles(&d, Channel::Normal, &[TileCoord::new(0, 0)])
        .is_ok());
    assert!(g
        .composite_tiles(&d, Channel::Color, &[TileCoord::new(u32::MAX, 0)])
        .is_err());
    assert!(g
        .brush_dabs(
            1,
            1,
            &[0; 4],
            &BrushSettings::default(),
            &[Dab {
                x: f64::NAN,
                y: 0.0,
                pressure: 1.0
            }]
        )
        .is_err());
    gpu_lease::lease();
    let mut small = GpuPainter::new(Options {
        budget_bytes: 1,
        ..Default::default()
    })
    .unwrap();
    assert!(small
        .composite_tiles(&d, Channel::Color, &coords(&d))
        .is_err());
    assert!(small
        .brush_dabs(1, 1, &[0; 4], &BrushSettings::default(), &[])
        .is_err());
    let mut fallback = Compositor::new(Options {
        budget_bytes: 1,
        ..Default::default()
    });
    let out = fallback
        .composite_tiles(&d, Channel::Color, &coords(&d))
        .unwrap();
    assert!(fallback.fallback_reason().unwrap().contains("予算"));
    assert_eq!(out.tiles[0].pixels, d.composite(d.bounds()).unwrap());
}

#[test]
fn integer_modes_opaque_bytes_are_exact() {
    let Some(mut g) = gpu() else { return };
    let mut d = Document::new(256, 256).unwrap();
    let a = d.add_layer("下地").unwrap();
    let b = d.add_layer("上").unwrap();
    for y in 0..256 {
        for x in 0..256 {
            d.set_pixel(a, x, y, Rgba8::new(x as u8, y as u8, (255 - x) as u8, 255))
                .unwrap();
            d.set_pixel(b, x, y, Rgba8::new(y as u8, x as u8, (255 - y) as u8, 255))
                .unwrap();
        }
    }
    for mode in [
        BlendMode::Normal,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::LinearDodge,
        BlendMode::LinearBurn,
        BlendMode::LinearLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::Difference,
        BlendMode::Subtract,
        BlendMode::DarkerColor,
        BlendMode::LighterColor,
    ] {
        d.set_layer_blend_mode(b, mode).unwrap();
        compare(&mut g, &d, &coords(&d), 0);
        eprintln!("不透明・量1 {mode:?}: 65536 画素の最大差 0");
    }
}

#[test]
fn multiple_clip_groups_and_channels() {
    let Some(mut g) = gpu() else { return };
    let mut d = Document::with_tile_size(33, 19, 16).unwrap();
    let mut ids = Vec::new();
    for k in 0..6 {
        let l = d.add_layer("組").unwrap();
        ids.push(l);
        for y in 0..19 {
            for x in 0..33 {
                if (x + y + k) % 3 != 0 {
                    d.set_pixel(
                        l,
                        x,
                        y,
                        Rgba8::new((k * 35) as u8, (x * 7) as u8, (y * 11) as u8, 170),
                    )
                    .unwrap();
                }
            }
        }
        d.set_layer_clipping(l, k % 3 != 0).unwrap();
        d.set_layer_opacity(l, 0.7, false).unwrap();
    }
    for hide in [None, Some(ids[0]), Some(ids[1]), Some(ids[3])] {
        if let Some(l) = hide {
            d.set_layer_visible(l, false).unwrap();
        }
        compare(&mut g, &d, &coords(&d), 1);
        if let Some(l) = hide {
            d.set_layer_visible(l, true).unwrap();
        }
    }
    let mut bytes = vec![0; 16 * 16 * 4];
    for p in bytes.as_chunks_mut::<4>().0 {
        p.copy_from_slice(&[33, 44, 55, 255]);
    }
    d.import_tile(ids[0], Channel::Emission, TileCoord::new(0, 0), &bytes)
        .unwrap();
    let r = g
        .composite_tiles(&d, Channel::Emission, &coords(&d))
        .unwrap();
    for t in r.tiles {
        let mut expected = vec![0; t.pixels.len()];
        d.composite_into(
            Channel::Emission,
            t.rect,
            &mut expected,
            yolu_core::RowOrder::BottomUp,
        )
        .unwrap();
        assert_eq!(expected, t.pixels);
    }
}

/// 常駐しない合成（`composite_tiles` と `Compositor`）も、独立して合成するグループ・調整レイヤー・効果のあるレイヤー・法線のチャンネルを CPU と同じ画素にする。
#[test]
fn composite_tiles_handles_isolated_groups_adjustments_effects_and_normals() {
    use yolu_core::effects::{EffectSettings, FilterSpec, FilterTarget};
    use yolu_core::AdjustmentSettings;
    let Some(mut g) = gpu() else { return };
    let mut d = Document::with_tile_size(67, 35, 16).unwrap();
    let mut seed = 0x2468ace1u32;
    let mut byte = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as u8
    };
    let mut layers = Vec::new();
    for k in 0..4 {
        let l = d.add_layer("レイヤー").unwrap();
        d.set_channel_enabled(l, Channel::Normal, true).unwrap();
        for y in 0..35 {
            for x in 0..67 {
                let alpha = [255, 200, 0, 90][((x + y + k) % 4) as usize];
                d.set_pixel(l, x, y, Rgba8::new(byte(), byte(), byte(), alpha))
                    .unwrap();
                d.set_channel_pixel(
                    l,
                    Channel::Normal,
                    x,
                    y,
                    Rgba8::new(128 + byte() / 8, 128 + byte() / 8, 200 + byte() / 5, alpha),
                )
                .unwrap();
            }
        }
        layers.push(l);
    }
    d.set_layer_blend_mode(layers[1], BlendMode::Multiply)
        .unwrap();
    let group = d.group_layers(&[layers[1], layers[2]], "組").unwrap();
    d.set_layer_blend_mode(group, BlendMode::Normal).unwrap();
    d.set_layer_opacity(group, 0.7, false).unwrap();
    d.add_adjustment_layer(
        "色相",
        AdjustmentSettings::hue_saturation(40.0, 0.2, 0.0).unwrap(),
        None,
        Some(layers[0]),
    )
    .unwrap();
    d.add_filter(
        layers[3],
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(2)).channels(&[Channel::Color]),
    )
    .unwrap();
    let all = coords(&d);
    assert!(compare(&mut g, &d, &all, 2) <= 2);
    // 法線のチャンネル
    let r = g.composite_tiles(&d, Channel::Normal, &all).unwrap();
    for t in r.tiles {
        let mut expected = vec![0; t.pixels.len()];
        d.composite_into(
            Channel::Normal,
            t.rect,
            &mut expected,
            yolu_core::RowOrder::BottomUp,
        )
        .unwrap();
        assert!(diff(&expected, &t.pixels) <= 2, "法線");
    }
    // 予算で GPU が使えない呼び出しは、CPU の合成が同じ文書を返す
    let mut fallback = Compositor::new(Options {
        budget_bytes: 1,
        ..Default::default()
    });
    let out = fallback.composite_tiles(&d, Channel::Color, &all).unwrap();
    assert_eq!(out.tiles[0].pixels, d.composite(out.tiles[0].rect).unwrap());
}
