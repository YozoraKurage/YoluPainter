#[path = "support/fill_cases.rs"]
mod cases;
use cases::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use yolu_core::{fill_image::*, Rgba8};

#[test]
fn csharp_all_modes_all_bytes_threads_and_tiles() {
    let fixture = Fixture::new(37, 29);
    let shape_data = picture(13, 17, true);
    let shape =
        ImageMipChain::build(&shape_data, 13, 17, Conversion::None, false, u64::MAX, None).unwrap();
    for threads in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            for mode in 0..6 {
                for v in 0..8 {
                    let (iw, ih) = if v == 5 { (1, 9) } else { (31, 23) };
                    let data = picture(iw, ih, false);
                    let chain = ImageMipChain::build(
                        &data,
                        iw as u32,
                        ih as u32,
                        if v == 6 {
                            Conversion::LinearToSrgb
                        } else {
                            Conversion::None
                        },
                        v == 7,
                        u64::MAX,
                        None,
                    )
                    .unwrap();
                    let sampler = fixture.sampler(mode, v, &chain, &shape);
                    let actual = sampler.render(0, 0, 37, 29, u64::MAX, None).unwrap();
                    let path = format!(
                        "{}/tests/golden/fill-image/{mode}-{v}.rgba",
                        env!("CARGO_MANIFEST_DIR")
                    );
                    let expected = std::fs::read(path).unwrap();
                    assert_eq!(actual.len(), expected.len());
                    let differences: Vec<_> = actual
                        .iter()
                        .zip(&expected)
                        .enumerate()
                        .filter(|(_, (a, b))| a != b)
                        .take(8)
                        .collect();
                    assert!(
                        differences.is_empty(),
                        "mode={mode} variant={v} threads={threads}: {differences:?}"
                    );
                    for y in (0..29).step_by(7) {
                        for x in (0..37).step_by(11) {
                            let w = 11.min(37 - x);
                            let h = 7.min(29 - y);
                            let tile = sampler.render(x, y, w, h, u64::MAX, None).unwrap();
                            for row in 0..h as usize {
                                let offset = ((y as usize + row) * 37 + x as usize) * 4;
                                assert_eq!(
                                    &tile[row * w as usize * 4..(row + 1) * w as usize * 4],
                                    &actual[offset..offset + w as usize * 4]
                                );
                            }
                        }
                    }
                }
            }
        });
    }
}
#[test]
fn uv_identity_preserves_transparent_rgb() {
    let data = picture(31, 23, false);
    let chain =
        ImageMipChain::build(&data, 31, 23, Conversion::None, false, u64::MAX, None).unwrap();
    let s = FillSampler::bind(FillInput {
        width: 31,
        height: 23,
        image: Some(&chain),
        ..FillInput::default()
    })
    .unwrap();
    assert_eq!(s.render(0, 0, 31, 23, u64::MAX, None).unwrap(), data);
}
#[test]
fn mip_budget_cancel_and_odd_sizes() {
    let d = vec![71; 5 * 3 * 4];
    assert_eq!(ImageMipChain::extra_bytes(5, 3).unwrap(), 12);
    assert!(matches!(
        ImageMipChain::build(&d, 5, 3, Conversion::None, false, 11, None),
        Err(FillError::OverBudget {
            needed: 12,
            budget: 11
        })
    ));
    let chain = ImageMipChain::build(&d, 5, 3, Conversion::None, false, 12, None).unwrap();
    assert_eq!(chain.bytes(), 12);
    assert_eq!(chain.level_count(), 3);
    assert_eq!(chain.size(1), Some((2, 1)));
    let stop = AtomicBool::new(true);
    assert!(matches!(
        ImageMipChain::build(&d, 5, 3, Conversion::None, false, 12, Some(&stop)),
        Err(FillError::Canceled)
    ));
    assert!(ImageMipChain::build(&d[..4], 5, 3, Conversion::None, false, 12, None).is_err());
    assert!(ImageMipChain::extra_bytes(0, 4).is_err());
    assert!(ImageMipChain::extra_bytes(8193, 4).is_err());
}
#[test]
fn minification_checker_has_known_average() {
    let mut d = vec![255; 8 * 8 * 4];
    for y in 0..8 {
        for x in 0..8 {
            let i = (y * 8 + x) * 4;
            d[i..i + 3].fill(if (x + y) % 2 == 0 { 0 } else { 255 });
        }
    }
    let c = ImageMipChain::build(&d, 8, 8, Conversion::None, false, 1000, None).unwrap();
    let s = FillSampler::bind(FillInput {
        image: Some(&c),
        ..FillInput::default()
    })
    .unwrap();
    assert_eq!(s.pixel(0, 0).unwrap(), Rgba8::new(128, 128, 128, 255));
}
#[test]
fn output_budget_cancel_and_bad_regions() {
    let s = FillSampler::bind(FillInput {
        width: 9,
        height: 7,
        ..FillInput::default()
    })
    .unwrap();
    let stop = AtomicBool::new(true);
    assert_eq!(
        s.render(0, 0, 9, 7, 251, None),
        Err(FillError::OverBudget {
            needed: 252,
            budget: 251
        })
    );
    assert_eq!(
        s.render(0, 0, 9, 7, 252, Some(&stop)),
        Err(FillError::Canceled)
    );
    assert!(s.render(u32::MAX, 0, 1, 1, 4, None).is_err());
    assert!(s.render(0, 0, 0, 1, 4, None).is_err());
    assert!(s.pixel(9, 0).is_err());
}
#[test]
fn invalid_settings_and_unknown_values_refused() {
    for field in 0..12 {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut p = Projection::default();
            match field {
                0 => p.tiles[0] = value,
                1 => p.tiles[1] = value,
                2 => p.offset[0] = value,
                3 => p.rotation = value,
                4 => p.blend_width = value,
                5 => p.depth_hardness = value,
                6 => p.backface_angle = value,
                7 => p.backface_hardness = value,
                8 => p.placement.center[0] = value,
                9 => p.placement.rotation[1] = value,
                10 => p.placement.size[2] = value,
                _ => p.offset[1] = value,
            }
            assert!(p.validate().is_err());
        }
    }
    for p in [
        Projection {
            tiles: [0., 1.],
            ..Projection::default()
        },
        Projection {
            rotation: 361.,
            ..Projection::default()
        },
        Projection {
            blend_width: 1.01,
            ..Projection::default()
        },
        Projection {
            depth_hardness: 1.,
            ..Projection::default()
        },
        Projection {
            placement: Placement {
                size: [-1., 1., 1.],
                ..Placement::default()
            },
            ..Projection::default()
        },
    ] {
        assert!(p.validate().is_err());
    }
    assert!(ProjectionMode::try_from(6).is_err());
    assert!(Wrap::try_from(3).is_err());
    assert!(FillSampler::bind(FillInput {
        frame: Some(ModelFrame {
            rotation: [0.; 4],
            ..ModelFrame::default()
        }),
        ..FillInput::default()
    })
    .is_err());
}
#[test]
fn inactive_projection_falls_back_and_decal_is_clear() {
    let d = [12, 34, 56, 255];
    let c = ImageMipChain::build(&d, 1, 1, Conversion::None, false, 0, None).unwrap();
    for mode in [
        ProjectionMode::Planar,
        ProjectionMode::Triplanar,
        ProjectionMode::Decal,
    ] {
        let fallback = Rgba8::new(9, 8, 7, 6);
        let s = FillSampler::bind(FillInput {
            image: Some(&c),
            fallback,
            projection: Projection {
                mode,
                ..Projection::default()
            },
            ..FillInput::default()
        })
        .unwrap();
        assert_eq!(s.reason(), Some(InactiveReason::MissingPosition));
        assert_eq!(
            s.pixel(0, 0).unwrap(),
            if mode == ProjectionMode::Decal {
                Rgba8::TRANSPARENT
            } else {
                fallback
            }
        );
    }
}
#[test]
fn map_reasons_and_bad_lengths() {
    let v = [0, 0, 0];
    let coverage = [1];
    let m = Map {
        width: 1,
        height: 1,
        values: &v,
        coverage: &coverage,
    };
    let base = || FillInput {
        projection: Projection {
            mode: ProjectionMode::Triplanar,
            ..Projection::default()
        },
        positions: Some(m),
        ..FillInput::default()
    };
    assert_eq!(
        FillSampler::bind(base()).unwrap().reason(),
        Some(InactiveReason::MissingNormal)
    );
    assert_eq!(
        FillSampler::bind(FillInput { width: 2, ..base() })
            .unwrap()
            .reason(),
        Some(InactiveReason::PositionSize)
    );
    assert_eq!(
        FillSampler::bind(FillInput {
            normals: Some(m),
            frame: None,
            ..base()
        })
        .unwrap()
        .reason(),
        Some(InactiveReason::UnknownModelFrame)
    );
    assert_eq!(
        FillSampler::bind(FillInput {
            stale_position: true,
            ..base()
        })
        .unwrap()
        .reason(),
        Some(InactiveReason::StalePosition)
    );
    assert_eq!(
        FillSampler::bind(FillInput {
            normals: Some(m),
            stale_normal: true,
            ..base()
        })
        .unwrap()
        .reason(),
        Some(InactiveReason::StaleNormal)
    );
    assert!(FillSampler::bind(FillInput {
        positions: Some(Map { values: &[], ..m }),
        ..base()
    })
    .is_err());
}
#[test]
fn geometry_uses_barycentric_and_face_normal_with_atomic_failure() {
    use yolu_core::{
        geometry::*,
        glam::{Vec2, Vec3},
    };
    let t = SurfaceTriangle::new(Vec3::ZERO, Vec3::X, Vec3::Y, Vec2::ZERO, Vec2::X, Vec2::Y);
    let g = SurfaceGeometry::new(vec![t], 17, DEFAULT_WELD_TOLERANCE).unwrap();
    let maps = ModelMaps::from_geometry(&g, 4, 4, 0, 208, 16, None).unwrap();
    assert_eq!(maps.revision, 17);
    assert_eq!(maps.positions().coverage[0], 1);
    assert_eq!(maps.positions().coverage[15], 0);
    assert_eq!(&maps.normals().values[..3], &[32768, 32768, 65535]);
    assert_eq!(&maps.positions().values[..2], &[8192, 8192]);
    assert!(matches!(
        ModelMaps::from_geometry(&g, 4, 4, 0, 207, 16, None),
        Err(FillError::OverBudget { .. })
    ));
    assert!(matches!(
        ModelMaps::from_geometry(&g, 4, 4, 0, 208, 0, None),
        Err(FillError::OverBudget { .. })
    ));
    assert!(matches!(
        ModelMaps::from_geometry(&g, 4, 4, 0, 208, 16, Some(&AtomicBool::new(true))),
        Err(FillError::Canceled)
    ));
}

#[test]
fn decal_depth_facing_and_shape_alpha_have_known_answers() {
    let pos = [32768, 32768, 32768];
    let normal = [32768, 32768, 0];
    let coverage = [1];
    let map = |values| Map {
        width: 1,
        height: 1,
        values,
        coverage: &coverage,
    };
    let d = [90, 120, 150, 128];
    let shape_data = [10, 20, 30, 128];
    let image = ImageMipChain::build(&d, 1, 1, Conversion::None, false, 0, None).unwrap();
    let shape = ImageMipChain::build(&shape_data, 1, 1, Conversion::None, false, 0, None).unwrap();
    let p = Projection {
        mode: ProjectionMode::Decal,
        depth_hardness: 1.,
        backface_angle: 180.,
        backface_hardness: 1.,
        ..Projection::default()
    };
    let base = || FillInput {
        projection: p,
        positions: Some(map(&pos)),
        normals: Some(map(&normal)),
        bounds_min: [-0.5; 3],
        bounds_max: [0.5; 3],
        fallback: Rgba8::new(90, 120, 150, 128),
        ..FillInput::default()
    };
    let own = FillSampler::bind(FillInput {
        image: Some(&image),
        ..base()
    })
    .unwrap();
    assert_eq!(own.pixel(0, 0).unwrap(), Rgba8::new(90, 120, 150, 128));
    assert_eq!(own.decal_coverage(0, 0).unwrap(), 1.);
    let other = FillSampler::bind(FillInput {
        image: Some(&image),
        shape: Some(&shape),
        ..base()
    })
    .unwrap();
    assert_eq!(other.pixel(0, 0).unwrap(), Rgba8::new(90, 120, 150, 64));
    let value = FillSampler::bind(FillInput {
        shape: Some(&shape),
        ..base()
    })
    .unwrap();
    assert_eq!(value.reason(), None);
    assert_eq!(value.pixel(0, 0).unwrap(), Rgba8::new(90, 120, 150, 64));
    let faded = FillSampler::bind(FillInput {
        projection: Projection {
            depth_hardness: 0.,
            ..p
        },
        bounds_min: [0., 0., 0.25],
        bounds_max: [0., 0., 0.25],
        ..base()
    })
    .unwrap();
    assert_eq!(faded.pixel(0, 0).unwrap().a, 64);
    let behind = [32768, 32768, 65535];
    let back = FillSampler::bind(FillInput {
        projection: Projection {
            backface_angle: 90.,
            ..p
        },
        normals: Some(map(&behind)),
        ..base()
    })
    .unwrap();
    assert_eq!(back.pixel(0, 0).unwrap(), Rgba8::TRANSPARENT);
    let outside = FillSampler::bind(FillInput {
        bounds_min: [2.; 3],
        bounds_max: [2.; 3],
        ..base()
    })
    .unwrap();
    assert_eq!(outside.pixel(0, 0).unwrap(), Rgba8::TRANSPARENT);
}

#[test]
fn cancellation_during_render_returns_no_partial_image() {
    let d = picture(31, 23, false);
    let c = ImageMipChain::build(&d, 31, 23, Conversion::None, false, u64::MAX, None).unwrap();
    let s = FillSampler::bind(FillInput {
        width: 4096,
        height: 4096,
        image: Some(&c),
        ..FillInput::default()
    })
    .unwrap();
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let task = scope.spawn(|| s.render(0, 0, 4096, 4096, u64::MAX, Some(&stop)));
        std::thread::sleep(std::time::Duration::from_millis(5));
        stop.store(true, Ordering::Relaxed);
        assert_eq!(task.join().unwrap(), Err(FillError::Canceled));
    });
}

#[test]
fn csharp_rejections_and_mip_budget_boundary_match() {
    let mut rejected = Vec::new();
    let p = Projection::default();
    for config in [
        Projection {
            tiles: [0., 1.],
            ..p
        },
        Projection {
            tiles: [f64::NAN, 1.],
            ..p
        },
        Projection {
            rotation: 400.,
            ..p
        },
        Projection {
            offset: [2e4, 0.],
            ..p
        },
        Projection {
            blend_width: 1.5,
            ..p
        },
    ] {
        rejected.push(config.validate().is_err());
    }
    rejected.push(ProjectionMode::try_from(9).is_err());
    rejected.push(Wrap::try_from(9).is_err());
    rejected.push(
        Projection {
            placement: Placement {
                size: [0., 1., 1.],
                ..Placement::default()
            },
            ..p
        }
        .validate()
        .is_err(),
    );
    rejected.push(
        Projection {
            depth_hardness: 1.,
            backface_hardness: 1.,
            ..p
        }
        .validate()
        .is_err(),
    );
    rejected.push(
        Projection {
            mode: ProjectionMode::Decal,
            depth_hardness: -0.1,
            backface_hardness: 1.,
            ..p
        }
        .validate()
        .is_err(),
    );
    rejected.push(
        Projection {
            mode: ProjectionMode::Decal,
            depth_hardness: 1.,
            backface_angle: 181.,
            backface_hardness: 1.,
            ..p
        }
        .validate()
        .is_err(),
    );
    rejected.push(
        FillSampler::bind(FillInput {
            frame: Some(ModelFrame {
                rotation: [0.; 4],
                ..ModelFrame::default()
            }),
            ..FillInput::default()
        })
        .is_err(),
    );
    let mut lines = String::new();
    for (i, refused) in rejected.into_iter().enumerate() {
        lines += &format!(
            "invalid-{i}:{}\n",
            if refused { "refused" } else { "accepted" }
        );
    }
    let d = picture(5, 3, false);
    for limit in [11, 12] {
        let result = ImageMipChain::build(&d, 5, 3, Conversion::None, false, limit, None);
        lines += &format!(
            "mip-budget-{limit}:{}\n",
            if matches!(result, Err(FillError::OverBudget { .. })) {
                "refused"
            } else {
                "accepted"
            }
        );
    }
    lines += &format!("mip-bytes:{}\n", ImageMipChain::extra_bytes(5, 3).unwrap());
    // 改行コードに依らない（.gitattributes で変換は止めてあるが、手元で変換された写しでも落とさない）
    let expected = include_str!("golden/fill-image/contracts.txt").replace("\r\n", "\n");
    assert_eq!(lines, expected);
}

#[test]
fn extreme_coordinates_are_refused_before_float_to_integer_overflow() {
    let d = [1, 2, 3, 4];
    let c = ImageMipChain::build(&d, 1, 1, Conversion::None, false, 0, None).unwrap();
    let values = [65535; 3];
    let coverage = [1];
    let map = Map {
        width: 1,
        height: 1,
        values: &values,
        coverage: &coverage,
    };
    let result = FillSampler::bind(FillInput {
        image: Some(&c),
        positions: Some(map),
        bounds_max: [1e100; 3],
        projection: Projection {
            mode: ProjectionMode::Planar,
            ..Projection::default()
        },
        ..FillInput::default()
    });
    assert!(matches!(result, Err(FillError::Invalid(_))));
}

#[test]
fn csharp_decal_values_all_bytes_threads_and_tiles() {
    let fixture = Fixture::new(37, 29);
    let shape_data = picture(13, 17, true);
    let shape =
        ImageMipChain::build(&shape_data, 13, 17, Conversion::None, false, u64::MAX, None).unwrap();
    let values = decal_values(37, 29);
    for threads in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            for v in 0..8 {
                let (iw, ih) = if v == 5 { (1, 9) } else { (31, 23) };
                let data = picture(iw, ih, false);
                let chain = ImageMipChain::build(
                    &data,
                    iw as u32,
                    ih as u32,
                    if v == 6 {
                        Conversion::LinearToSrgb
                    } else {
                        Conversion::None
                    },
                    v == 7,
                    u64::MAX,
                    None,
                )
                .unwrap();
                let sampler = fixture.sampler(5, v, &chain, &shape);
                let actual = sampler
                    .render_decal_values(0, 0, 37, 29, &values, u64::MAX, None)
                    .unwrap();
                let path = format!(
                    "{}/tests/golden/fill-image/decal-values-{v}.rgba",
                    env!("CARGO_MANIFEST_DIR")
                );
                let expected = std::fs::read(path).unwrap();
                assert_eq!(actual.len(), expected.len());
                let differences: Vec<_> = actual
                    .iter()
                    .zip(&expected)
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .take(8)
                    .collect();
                assert!(
                    differences.is_empty(),
                    "variant={v} threads={threads}: {differences:?}"
                );
                for y in (0..29usize).step_by(7) {
                    for x in (0..37usize).step_by(11) {
                        let (w, h) = (11.min(37 - x), 7.min(29 - y));
                        let mut part = Vec::new();
                        for row in 0..h {
                            let o = ((y + row) * 37 + x) * 4;
                            part.extend_from_slice(&values[o..o + w * 4]);
                        }
                        let tile = sampler
                            .render_decal_values(
                                x as u32,
                                y as u32,
                                w as u32,
                                h as u32,
                                &part,
                                u64::MAX,
                                None,
                            )
                            .unwrap();
                        for row in 0..h {
                            let o = ((y + row) * 37 + x) * 4;
                            assert_eq!(
                                &tile[row * w * 4..(row + 1) * w * 4],
                                &actual[o..o + w * 4]
                            );
                        }
                    }
                }
                for (x, y) in [(0usize, 0usize), (5, 3), (18, 14), (36, 28)] {
                    let o = (y * 37 + x) * 4;
                    let one = sampler
                        .apply_decal_to_value(
                            x as u32,
                            y as u32,
                            Rgba8::from_slice(&values[o..o + 4]),
                        )
                        .unwrap();
                    assert_eq!(one.to_array(), actual[o..o + 4]);
                }
            }
        });
    }
}

#[test]
fn decal_values_take_coverage_and_shape_but_never_the_own_image() {
    let pos = [32768, 32768, 32768];
    let normal = [32768, 32768, 0];
    let coverage = [1];
    let map = |values| Map {
        width: 1,
        height: 1,
        values,
        coverage: &coverage,
    };
    let d = [90, 120, 150, 128];
    let shape_data = [10, 20, 30, 128];
    let image = ImageMipChain::build(&d, 1, 1, Conversion::None, false, 0, None).unwrap();
    let shape = ImageMipChain::build(&shape_data, 1, 1, Conversion::None, false, 0, None).unwrap();
    let p = Projection {
        mode: ProjectionMode::Decal,
        depth_hardness: 1.,
        backface_angle: 180.,
        backface_hardness: 1.,
        ..Projection::default()
    };
    let base = || FillInput {
        projection: p,
        positions: Some(map(&pos)),
        normals: Some(map(&normal)),
        bounds_min: [-0.5; 3],
        bounds_max: [0.5; 3],
        fallback: Rgba8::new(1, 2, 3, 255),
        ..FillInput::default()
    };
    let value = Rgba8::new(10, 20, 30, 200);
    // 自身の画像は使わない。値がそのまま出る（被覆 1）
    let own = FillSampler::bind(FillInput {
        image: Some(&image),
        ..base()
    })
    .unwrap();
    assert_eq!(own.apply_decal_to_value(0, 0, value).unwrap(), value);
    // 別チャンネルの形のアルファ 128/255 が被覆にかかる: 200/255 × 128/255 → 100
    let other = FillSampler::bind(FillInput {
        image: Some(&image),
        shape: Some(&shape),
        ..base()
    })
    .unwrap();
    assert_eq!(
        other.apply_decal_to_value(0, 0, value).unwrap(),
        Rgba8::new(10, 20, 30, 100)
    );
    assert_eq!(
        other
            .render_decal_values(0, 0, 1, 1, &value.to_array(), 4, None)
            .unwrap(),
        [10, 20, 30, 100]
    );
    // 奥行きの縁で被覆 0.5 → 150 × 0.5 = 75
    let faded = FillSampler::bind(FillInput {
        projection: Projection {
            depth_hardness: 0.,
            ..p
        },
        bounds_min: [0., 0., 0.25],
        bounds_max: [0., 0., 0.25],
        ..base()
    })
    .unwrap();
    assert_eq!(
        faded
            .apply_decal_to_value(0, 0, Rgba8::new(10, 20, 30, 150))
            .unwrap(),
        Rgba8::new(10, 20, 30, 75)
    );
    // 裏向き・箱の外は透明
    let behind = [32768, 32768, 65535];
    let back = FillSampler::bind(FillInput {
        projection: Projection {
            backface_angle: 90.,
            ..p
        },
        normals: Some(map(&behind)),
        ..base()
    })
    .unwrap();
    assert_eq!(
        back.apply_decal_to_value(0, 0, value).unwrap(),
        Rgba8::TRANSPARENT
    );
    let outside = FillSampler::bind(FillInput {
        bounds_min: [2.; 3],
        bounds_max: [2.; 3],
        ..base()
    })
    .unwrap();
    assert_eq!(
        outside.apply_decal_to_value(0, 0, value).unwrap(),
        Rgba8::TRANSPARENT
    );
    // 置けない（マップが無い）デカールは透明
    let unplaced = FillSampler::bind(FillInput {
        projection: p,
        ..FillInput::default()
    })
    .unwrap();
    assert!(!unplaced.placed());
    assert_eq!(
        unplaced.apply_decal_to_value(0, 0, value).unwrap(),
        Rgba8::TRANSPARENT
    );
    // 予算・取消・長さ・座標・デカール以外
    let stop = AtomicBool::new(true);
    assert_eq!(
        own.render_decal_values(0, 0, 1, 1, &value.to_array(), 3, None),
        Err(FillError::OverBudget {
            needed: 4,
            budget: 3
        })
    );
    assert_eq!(
        own.render_decal_values(0, 0, 1, 1, &value.to_array(), 4, Some(&stop)),
        Err(FillError::Canceled)
    );
    assert!(matches!(
        own.render_decal_values(0, 0, 1, 1, &[1, 2, 3], 4, None),
        Err(FillError::Invalid(_))
    ));
    assert!(matches!(
        own.render_decal_values(0, 0, 2, 1, &[0; 8], 8, None),
        Err(FillError::Invalid(_))
    ));
    assert!(own.apply_decal_to_value(1, 0, value).is_err());
    let planar = FillSampler::bind(FillInput {
        projection: Projection {
            mode: ProjectionMode::Planar,
            ..Projection::default()
        },
        ..base()
    })
    .unwrap();
    assert!(matches!(
        planar.apply_decal_to_value(0, 0, value),
        Err(FillError::Invalid(_))
    ));
    assert!(matches!(
        planar.render_decal_values(0, 0, 1, 1, &value.to_array(), 4, None),
        Err(FillError::Invalid(_))
    ));
}

// ---- geometry で焼いたマップを、そのままサンプラーへ通す ----

/// x, y が -1..1 の四角形（三角形 2 枚）。UV は 0..1 の正方形で、位置は u, v に線形（z は v に比例して 0..top）。
/// `toward_minus_z` なら面法線は -Z 寄り（デカールが見る側）、でなければ +Z 寄り。
fn board(top: f32, toward_minus_z: bool) -> yolu_core::geometry::SurfaceGeometry {
    use yolu_core::{
        geometry::*,
        glam::{Vec2, Vec3},
    };
    let corner = |u: f32, v: f32| Vec3::new(-1. + 2. * u, -1. + 2. * v, top * v);
    let (p00, p10, p11, p01) = (
        corner(0., 0.),
        corner(1., 0.),
        corner(1., 1.),
        corner(0., 1.),
    );
    let (u00, u10, u11, u01) = (Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y);
    let triangles = if toward_minus_z {
        vec![
            SurfaceTriangle::new(p00, p11, p10, u00, u11, u10),
            SurfaceTriangle::new(p00, p01, p11, u00, u01, u11),
        ]
    } else {
        vec![
            SurfaceTriangle::new(p00, p10, p11, u00, u10, u11),
            SurfaceTriangle::new(p00, p11, p01, u00, u11, u01),
        ]
    };
    SurfaceGeometry::new(triangles, 5, DEFAULT_WELD_TOLERANCE).unwrap()
}
/// 不透明で、画素ごとに違う色の w × h。
fn ramp(w: usize, h: usize) -> Vec<u8> {
    let mut d = vec![255; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            d[i] = (x * 31 + 3) as u8;
            d[i + 1] = (y * 29 + 5) as u8;
            d[i + 2] = ((x + y) * 7) as u8;
        }
    }
    d
}
fn texel(d: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
    d[(y * w + x) * 4..(y * w + x) * 4 + 4].try_into().unwrap()
}
/// 焼いたマップの箱と値をそのままサンプラーの入力に写す（呼び手がやる受け渡しを 1 か所にそろえる）。
fn input_from<'a>(
    maps: &'a ModelMaps,
    projection: Projection,
    image: Option<&'a ImageMipChain<'a>>,
) -> FillInput<'a> {
    FillInput {
        projection,
        width: 8,
        height: 8,
        image,
        positions: Some(maps.positions()),
        normals: Some(maps.normals()),
        bounds_min: maps.bounds_min,
        bounds_max: maps.bounds_max,
        ..FillInput::default()
    }
}

#[test]
fn geometry_maps_drive_planar_triplanar_and_decal_to_known_pixels() {
    let image = ramp(8, 8);
    let chain =
        ImageMipChain::build(&image, 8, 8, Conversion::None, false, u64::MAX, None).unwrap();
    let render = |s: &FillSampler| s.render(0, 0, 8, 8, u64::MAX, None).unwrap();

    // 平らな板（z の幅が 0 の軸）: 位置は 0.5 → 32768 に符号化され、復号で bounds の最小へ戻る
    let flat = board(0., false);
    let maps = ModelMaps::from_geometry(&flat, 8, 8, 0, u64::MAX, u64::MAX, None).unwrap();
    assert_eq!(maps.bounds_min, [-1., -1., 0.]);
    assert_eq!(maps.bounds_max, [1., 1., 0.]);
    assert!(maps.positions().values.chunks(3).all(|p| p[2] == 32768));
    let planar_projection = Projection {
        mode: ProjectionMode::Planar,
        placement: Placement {
            size: [2., 2., 1.],
            ..Placement::default()
        },
        ..Projection::default()
    };
    let planar = FillSampler::bind(input_from(&maps, planar_projection, Some(&chain))).unwrap();
    assert_eq!(planar.reason(), None);
    let out = render(&planar);
    for y in 0..8 {
        for x in 0..8 {
            assert_eq!(
                texel(&out, 8, x, y),
                texel(&image, 8, x, y),
                "平面 ({x},{y})"
            );
        }
    }
    // bounds を写し忘れる（既定の 0..1）と別の画素を読む。上の一致が箱の受け渡しを確かめている
    let forgotten = FillSampler::bind(FillInput {
        bounds_min: [0.; 3],
        bounds_max: [1.; 3],
        ..input_from(&maps, planar_projection, Some(&chain))
    })
    .unwrap();
    assert_ne!(render(&forgotten), out);

    // トライプラナー: +Z 向きの面は X が反転、-Z 向きの面は反転しない（法線の符号化と復号が合っている）
    let triplanar_projection = Projection {
        mode: ProjectionMode::Triplanar,
        ..planar_projection
    };
    let plus = FillSampler::bind(input_from(&maps, triplanar_projection, Some(&chain))).unwrap();
    let out = render(&plus);
    for y in 0..8 {
        for x in 0..8 {
            assert_eq!(
                texel(&out, 8, x, y),
                texel(&image, 8, 7 - x, y),
                "+Z 面 ({x},{y})"
            );
        }
    }
    let minus_board = board(0., true);
    let minus_maps =
        ModelMaps::from_geometry(&minus_board, 8, 8, 0, u64::MAX, u64::MAX, None).unwrap();
    let minus =
        FillSampler::bind(input_from(&minus_maps, triplanar_projection, Some(&chain))).unwrap();
    let out = render(&minus);
    for y in 0..8 {
        for x in 0..8 {
            assert_eq!(
                texel(&out, 8, x, y),
                texel(&image, 8, x, y),
                "-Z 面 ({x},{y})"
            );
        }
    }

    // デカール: 斜めの板（z が 0..1、-Z 寄りの面）。箱は x が -0.5..0.5、z が 0.375..0.625 の帯
    // （y は広く取る）。z の復号がずれると行の範囲が変わる
    let slanted = board(1., true);
    let maps = ModelMaps::from_geometry(&slanted, 8, 8, 0, u64::MAX, u64::MAX, None).unwrap();
    assert_eq!(maps.bounds_max, [1., 1., 1.]);
    let decal_image = ramp(4, 8);
    let decal_chain =
        ImageMipChain::build(&decal_image, 4, 8, Conversion::None, false, u64::MAX, None).unwrap();
    let decal_projection = Projection {
        mode: ProjectionMode::Decal,
        placement: Placement {
            center: [0., 0., 0.5],
            size: [1., 2., 0.25],
            ..Placement::default()
        },
        ..Projection::default()
    };
    let decal = FillSampler::bind(input_from(&maps, decal_projection, Some(&decal_chain))).unwrap();
    assert_eq!(decal.reason(), None);
    let out = render(&decal);
    for y in 0..8 {
        for x in 0..8 {
            let inside = (2..6).contains(&x) && (3..5).contains(&y);
            let expected = if inside {
                texel(&decal_image, 4, x - 2, y)
            } else {
                [0; 4]
            };
            assert_eq!(texel(&out, 8, x, y), expected, "デカール ({x},{y})");
            assert_eq!(
                decal.decal_coverage(x as u32, y as u32).unwrap(),
                f64::from(u8::from(inside)),
                "デカールの被覆 ({x},{y})"
            );
        }
    }
    // 画素ごとの値にも同じ被覆がかかる
    let values = vec![200; 8 * 8 * 4];
    let valued = decal
        .render_decal_values(0, 0, 8, 8, &values, u64::MAX, None)
        .unwrap();
    for y in 0..8 {
        for x in 0..8 {
            let inside = (2..6).contains(&x) && (3..5).contains(&y);
            assert_eq!(
                texel(&valued, 8, x, y),
                if inside { [200; 4] } else { [0; 4] },
                "デカールの値 ({x},{y})"
            );
        }
    }
}

// ---- 取り消しは途中でも効く ----

/// 取消なしで掛かる時間 T の 1/3 で取消を立て、`Canceled` で、立ててから戻るまでが T の 1/4 より短いこと。
/// 行・三角形ごとの確認が無く、段や終わりだけの確認だと、立てたあとも T の半分ほど走ってこの時間に収まらない。
fn assert_cancels_promptly(run: impl Fn(&AtomicBool) -> Result<(), FillError> + Sync) {
    let start = Instant::now();
    run(&AtomicBool::new(false)).unwrap();
    let full = start.elapsed();
    assert!(
        full > Duration::from_millis(10),
        "取消なしの時間が短すぎて測れない: {full:?}"
    );
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let task = scope.spawn(|| run(&stop));
        std::thread::sleep(full / 3);
        let raised = Instant::now();
        stop.store(true, Ordering::Relaxed);
        let result = task.join().unwrap();
        let latency = raised.elapsed();
        assert_eq!(result, Err(FillError::Canceled), "全体 {full:?}");
        assert!(
            latency < full / 4,
            "取消を立ててから戻るまで {latency:?}（全体 {full:?}）"
        );
    });
}

#[test]
fn cancellation_during_mip_build_stops_early_without_a_chain() {
    let pixels = vec![0u8; 8192 * 8192 * 4];
    assert_cancels_promptly(|cancel| {
        ImageMipChain::build(
            &pixels,
            8192,
            8192,
            Conversion::None,
            false,
            u64::MAX,
            Some(cancel),
        )
        .map(|_| ())
    });
}

#[test]
fn cancellation_during_geometry_bake_stops_early_without_maps() {
    use yolu_core::{
        geometry::*,
        glam::{Vec2, Vec3},
    };
    // どれも UV の箱が全面で、覆う画素はごくわずか。覆われていない画素を三角形のたびに調べ直す
    let triangles = (0..40)
        .map(|i| {
            let edge = 0.002 * (i + 1) as f32;
            SurfaceTriangle::new(
                Vec3::new(i as f32 * 0.01, 0., 0.),
                Vec3::X,
                Vec3::Y,
                Vec2::ZERO,
                Vec2::ONE,
                Vec2::new(1., 1. - edge),
            )
        })
        .collect();
    let geometry = SurfaceGeometry::new(triangles, 1, DEFAULT_WELD_TOLERANCE).unwrap();
    assert_cancels_promptly(|cancel| {
        ModelMaps::from_geometry(&geometry, 1024, 1024, 0, u64::MAX, u64::MAX, Some(cancel))
            .map(|_| ())
    });
}

// ---- 画像が無い・読めない・大きさが違う ----

#[test]
fn missing_image_reason_and_decal_that_draws_its_value() {
    let fallback = Rgba8::new(9, 8, 7, 200);
    // 画像が無い通常の投影は代替値で、理由は MissingImage（UV でも、置けた平面でも）
    let uv = FillSampler::bind(FillInput {
        fallback,
        ..FillInput::default()
    })
    .unwrap();
    assert_eq!(uv.reason(), Some(InactiveReason::MissingImage));
    assert_eq!(uv.pixel(0, 0).unwrap(), fallback);
    let pos = [32768, 32768, 32768];
    let normal = [32768, 32768, 0];
    let coverage = [1];
    let map = |values| Map {
        width: 1,
        height: 1,
        values,
        coverage: &coverage,
    };
    let planar = FillSampler::bind(FillInput {
        projection: Projection {
            mode: ProjectionMode::Planar,
            ..Projection::default()
        },
        positions: Some(map(&pos)),
        fallback,
        ..FillInput::default()
    })
    .unwrap();
    assert!(planar.placed());
    assert_eq!(planar.reason(), Some(InactiveReason::MissingImage));
    assert_eq!(planar.pixel(0, 0).unwrap(), fallback);

    let projection = Projection {
        mode: ProjectionMode::Decal,
        depth_hardness: 1.,
        backface_angle: 180.,
        backface_hardness: 1.,
        ..Projection::default()
    };
    let base = || FillInput {
        projection,
        positions: Some(map(&pos)),
        normals: Some(map(&normal)),
        bounds_min: [-0.5; 3],
        bounds_max: [0.5; 3],
        fallback,
        ..FillInput::default()
    };
    // 値だけのデカール（image も shape も無い）: 理由は無く、箱の中で値をそのまま描く
    let value_only = FillSampler::bind(base()).unwrap();
    assert_eq!(value_only.reason(), None);
    assert!(value_only.placed());
    assert_eq!(value_only.pixel(0, 0).unwrap(), fallback);
    // 奥行きの縁では値のアルファに被覆がかかる: 200/255 × 0.5 → 100
    let faded = FillSampler::bind(FillInput {
        projection: Projection {
            depth_hardness: 0.,
            ..projection
        },
        bounds_min: [0., 0., 0.25],
        bounds_max: [0., 0., 0.25],
        ..base()
    })
    .unwrap();
    assert_eq!(faded.pixel(0, 0).unwrap(), Rgba8::new(9, 8, 7, 100));
    // 画像を指定したのに読めなかったデカール: 理由は MissingImage だが、箱の中では値を描く（透明にしない）
    let unreadable = FillSampler::bind(FillInput {
        missing_image: true,
        ..base()
    })
    .unwrap();
    assert_eq!(unreadable.reason(), Some(InactiveReason::MissingImage));
    assert!(unreadable.placed());
    assert_eq!(unreadable.pixel(0, 0).unwrap(), fallback);
    assert_eq!(unreadable.decal_coverage(0, 0).unwrap(), 1.);
    // 箱の外は透明
    let outside = FillSampler::bind(FillInput {
        missing_image: true,
        bounds_min: [2.; 3],
        bounds_max: [2.; 3],
        ..base()
    })
    .unwrap();
    assert_eq!(outside.reason(), Some(InactiveReason::MissingImage));
    assert_eq!(outside.pixel(0, 0).unwrap(), Rgba8::TRANSPARENT);
    // 別チャンネルの形があれば、読めない画像のデカールにも形がかかる（128/255 → 200 × 0.502 → 100）
    let shape_data = [10, 20, 30, 128];
    let shape = ImageMipChain::build(&shape_data, 1, 1, Conversion::None, false, 0, None).unwrap();
    let shaped = FillSampler::bind(FillInput {
        missing_image: true,
        shape: Some(&shape),
        ..base()
    })
    .unwrap();
    assert_eq!(shaped.pixel(0, 0).unwrap(), Rgba8::new(9, 8, 7, 100));
    // 位置が置けないときは、画像を指定していてもいなくても、理由は置けない側が優先で透明
    let unplaced = FillSampler::bind(FillInput {
        missing_image: true,
        positions: None,
        ..base()
    })
    .unwrap();
    assert_eq!(unplaced.reason(), Some(InactiveReason::MissingPosition));
    assert_eq!(unplaced.pixel(0, 0).unwrap(), Rgba8::TRANSPARENT);
}

#[test]
fn normal_map_of_another_size_is_named_and_not_used() {
    let positions = [0u16; 3];
    let normals = [0u16; 6];
    let position_coverage = [1];
    let normal_coverage = [1, 1];
    let position_map = Map {
        width: 1,
        height: 1,
        values: &positions,
        coverage: &position_coverage,
    };
    let normal_map = Map {
        width: 2,
        height: 1,
        values: &normals,
        coverage: &normal_coverage,
    };
    let image = [1, 2, 3, 255];
    let chain = ImageMipChain::build(&image, 1, 1, Conversion::None, false, 0, None).unwrap();
    for mode in [ProjectionMode::Triplanar, ProjectionMode::Decal] {
        let s = FillSampler::bind(FillInput {
            projection: Projection {
                mode,
                ..Projection::default()
            },
            image: Some(&chain),
            positions: Some(position_map),
            normals: Some(normal_map),
            fallback: Rgba8::new(9, 8, 7, 6),
            ..FillInput::default()
        })
        .unwrap();
        assert_eq!(s.reason(), Some(InactiveReason::NormalSize), "{mode:?}");
        assert!(!s.placed());
        assert_eq!(
            s.pixel(0, 0).unwrap(),
            if mode == ProjectionMode::Decal {
                Rgba8::TRANSPARENT
            } else {
                Rgba8::new(9, 8, 7, 6)
            }
        );
    }
    // 平面は法線を読まないので、同じ入力でも置ける
    let planar = FillSampler::bind(FillInput {
        projection: Projection {
            mode: ProjectionMode::Planar,
            ..Projection::default()
        },
        image: Some(&chain),
        positions: Some(position_map),
        normals: Some(normal_map),
        ..FillInput::default()
    })
    .unwrap();
    assert_eq!(planar.reason(), None);
    assert!(planar.placed());
}
