//! 0.5.0 のフィルターの段（種類 70〜79）: 式どおりの値（小さな画像）、範囲の端、値の種類で断るもの、ブロックの分け方・並列度で
//! 同じバイト、到達半径と作業メモリの見積り、透明な画素の RGB。SIMD の道は使わない（スカラーの 1 本の道）ので、道ごとの比較は
//! 単体試験（`filter/tests.rs`）にある。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::filter::{
    block_working_bytes, evaluate, Error, Image, MorphologyMode, Options, Settings, SlopeMode,
    Stage, ValueType, MAX_HALO,
};
use yolu_core::Rect;

fn full(w: u32, h: u32) -> Rect {
    Rect::new(0, 0, w, h)
}

/// 灰色の画像（スカラー: R = G = B、A = 255）。`f(x, y)` が値。
fn gray(w: u32, h: u32, f: impl Fn(u32, u32) -> u8) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let g = f(x, y);
            v.extend_from_slice(&[g, g, g, 255]);
        }
    }
    v
}

/// 色と透明の混ざった画像（ブロックの境目の試験用）。
fn pattern(w: u32, h: u32) -> Vec<u8> {
    let mut b = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let a = if (x + y) % 7 == 0 {
                0
            } else if (x * 3 + y) % 5 == 0 {
                255
            } else {
                ((x * 19 + y * 23 + 41) % 256) as u8
            };
            b.extend_from_slice(&[
                ((x * 17 + y * 31 + 3) % 256) as u8,
                ((x * 7 + y * 13 + 71) % 256) as u8,
                ((x * 43 + y * 5 + 191) % 256) as u8,
                a,
            ]);
        }
    }
    b
}

fn run(data: &[u8], w: u32, h: u32, ty: ValueType, settings: Settings) -> Vec<u8> {
    let src = Image::new(data, w, h).unwrap();
    evaluate(
        &src,
        ty,
        &[Stage::new(settings)],
        full(w, h),
        &Options::default(),
    )
    .unwrap()
}

fn try_run(ty: ValueType, settings: Settings) -> Result<Vec<u8>, Error> {
    let data = gray(4, 4, |x, y| (x * 40 + y * 9) as u8);
    let src = Image::new(&data, 4, 4).unwrap();
    evaluate(
        &src,
        ty,
        &[Stage::new(settings)],
        full(4, 4),
        &Options::default(),
    )
}

/// 0〜1 を 0〜255 へ（core の `to_byte` と同じ: floor(v × 255 + 0.5) を範囲へ）。
fn byte(v: f64) -> u8 {
    (v * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8
}

/// 種類ごとの既定（目録と同じ値）と、足せる値の種類。
fn kinds() -> Vec<(&'static str, Settings, [bool; 4])> {
    // [色, スカラー, 接空間法線, マスク]
    let all = [true, true, false, true];
    let scalar = [false, true, false, true];
    vec![
        (
            "histogram_scan",
            Settings::HistogramScan {
                position: 0.5,
                contrast: 0.0,
            },
            scalar,
        ),
        (
            "histogram_range",
            Settings::HistogramRange {
                range: 0.5,
                position: 0.5,
            },
            scalar,
        ),
        (
            "slope_blur",
            Settings::SlopeBlur {
                intensity: 8.0,
                samples: 8,
                mode: SlopeMode::Blur,
                scale: 16.0,
                seed: 0,
            },
            all,
        ),
        (
            "directional_blur",
            Settings::DirectionalBlur {
                angle: 0.0,
                distance: 8.0,
            },
            all,
        ),
        (
            "warp",
            Settings::Warp {
                intensity: 16.0,
                scale: 32.0,
                seed: 0,
            },
            all,
        ),
        (
            "morphology",
            Settings::Morphology {
                mode: MorphologyMode::Dilate,
                radius: 2,
            },
            scalar,
        ),
        (
            "edge_detect",
            Settings::EdgeDetect {
                width: 1,
                threshold: 0.1,
            },
            scalar,
        ),
        ("high_pass", Settings::HighPass { radius: 8 }, all),
        ("median", Settings::Median { radius: 1 }, all),
        (
            "glow",
            Settings::Glow {
                threshold: 0.7,
                radius: 16,
                intensity: 1.0,
            },
            [true, false, false, false],
        ),
    ]
}

const TYPES: [ValueType; 4] = [
    ValueType::Color,
    ValueType::Scalar,
    ValueType::TangentNormal,
    ValueType::Mask,
];

#[test]
fn each_kind_is_accepted_or_refused_per_value_type_with_a_reason() {
    for (name, settings, accepts) in kinds() {
        for (ty, ok) in TYPES.iter().zip(accepts) {
            let result = try_run(*ty, settings.clone());
            if ok {
                assert!(result.is_ok(), "{name} は {ty:?} に付けられる: {result:?}");
            } else {
                match result {
                    Err(Error::Invalid(m)) => assert!(!m.contains("ください"), "{m}"),
                    other => panic!("{name} は {ty:?} に付けられない: {other:?}"),
                }
            }
        }
    }
}

#[test]
fn histogram_scan_and_range_follow_their_formulas() {
    let (w, h) = (256, 1);
    let data = gray(w, h, |x, _| x as u8);
    for (position, contrast) in [(0.5, 0.0), (0.3, 0.5), (0.8, 0.9), (0.5, 1.0), (0.0, 0.25)] {
        let out = run(
            &data,
            w,
            h,
            ValueType::Scalar,
            Settings::HistogramScan { position, contrast },
        );
        let width: f64 = (1.0f64 - contrast).max(1.0 / 255.0);
        for x in 0..256usize {
            let v = x as f64 / 255.0;
            let expected = byte(((v - (position - width / 2.0)) / width).clamp(0.0, 1.0));
            assert_eq!(out[x * 4], expected, "scan {position} {contrast} {x}");
            assert_eq!(out[x * 4 + 3], 255);
        }
    }
    // コントラスト 0・位置 0.5 は何も変えない
    let same = run(
        &data,
        w,
        h,
        ValueType::Scalar,
        Settings::HistogramScan {
            position: 0.5,
            contrast: 0.0,
        },
    );
    assert_eq!(same, data);
    for (range, position) in [(0.5, 0.5), (1.0, 0.5), (0.0, 0.2), (0.3, 0.9)] {
        let out = run(
            &data,
            w,
            h,
            ValueType::Scalar,
            Settings::HistogramRange { range, position },
        );
        for x in 0..256usize {
            let v = x as f64 / 255.0;
            let expected = byte((position + (v - 0.5) * range).clamp(0.0, 1.0));
            assert_eq!(out[x * 4], expected, "range {range} {position} {x}");
        }
    }
}

#[test]
fn histogram_range_on_a_mask_maps_the_hidden_amount() {
    // マスクは A（隠す量）を値として読む
    let mut data = Vec::new();
    for x in 0..256u32 {
        data.extend_from_slice(&[0, 0, 0, x as u8]);
    }
    let src = Image::new(&data, 256, 1).unwrap();
    for (range, position) in [(0.5, 0.5), (1.0, 0.5), (0.0, 0.2), (0.3, 0.9)] {
        let out = evaluate(
            &src,
            ValueType::Mask,
            &[Stage::new(Settings::HistogramRange { range, position })],
            full(256, 1),
            &Options::default(),
        )
        .unwrap();
        for x in 0..256usize {
            let v = x as f64 / 255.0;
            let expected = byte((position + (v - 0.5) * range).clamp(0.0, 1.0));
            assert_eq!(out[x * 4 + 3], expected, "range {range} {position} {x}");
        }
    }
}

#[test]
fn morphology_grows_and_shrinks_a_round_window() {
    let (w, h) = (15, 15);
    let dot = gray(w, h, |x, y| if (x, y) == (7, 7) { 255 } else { 0 });
    for radius in [1u32, 2, 3, 5] {
        let out = run(
            &dot,
            w,
            h,
            ValueType::Scalar,
            Settings::Morphology {
                mode: MorphologyMode::Dilate,
                radius,
            },
        );
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as i64 - 7, y as i64 - 7);
                let inside = dx * dx + dy * dy <= i64::from(radius * radius);
                let v = out[((y * w + x) * 4) as usize];
                assert_eq!(v, if inside { 255 } else { 0 }, "r={radius} ({x},{y})");
            }
        }
        // 白地の黒い点は erode で同じ丸に広がる
        let hole = gray(w, h, |x, y| if (x, y) == (7, 7) { 0 } else { 255 });
        let out = run(
            &hole,
            w,
            h,
            ValueType::Scalar,
            Settings::Morphology {
                mode: MorphologyMode::Erode,
                radius,
            },
        );
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as i64 - 7, y as i64 - 7);
                let inside = dx * dx + dy * dy <= i64::from(radius * radius);
                assert_eq!(
                    out[((y * w + x) * 4) as usize],
                    if inside { 0 } else { 255 }
                );
            }
        }
    }
}

#[test]
fn morphology_skips_transparent_pixels_and_keeps_their_rgb() {
    // スカラーの層: 透明な画素（RGB 200）は窓に数えず、自分の RGB と A も変えない
    let (w, h) = (5, 1);
    let mut data = Vec::new();
    for x in 0..w {
        if x == 2 {
            data.extend_from_slice(&[200, 200, 200, 0]);
        } else {
            data.extend_from_slice(&[10 * x as u8, 10 * x as u8, 10 * x as u8, 255]);
        }
    }
    let out = run(
        &data,
        w,
        h,
        ValueType::Scalar,
        Settings::Morphology {
            mode: MorphologyMode::Dilate,
            radius: 1,
        },
    );
    assert_eq!(&out[8..12], &[200, 200, 200, 0]);
    // 画素 1 の窓は 0・1・(2 は透明)、画素 3 の窓は (2)・3・4
    assert_eq!(out[4], 10);
    assert_eq!(out[12], 40);
}

#[test]
fn edge_detect_finds_a_step_and_the_threshold_clears_it() {
    let (w, h) = (16, 8);
    let step = gray(w, h, |x, _| if x < 8 { 0 } else { 255 });
    let out = run(
        &step,
        w,
        h,
        ValueType::Scalar,
        Settings::EdgeDetect {
            width: 1,
            threshold: 0.0,
        },
    );
    for y in 0..h {
        for x in 0..w {
            let v = out[((y * w + x) * 4) as usize];
            if (6..=9).contains(&x) {
                assert!(v > 0, "縁の近く ({x},{y})");
            }
            if !(4..=11).contains(&x) {
                assert_eq!(v, 0, "縁から離れた所 ({x},{y})");
            }
        }
    }
    // 一様な画像は 0、しきい値 1 は全部 0
    let flat = gray(w, h, |_, _| 90);
    assert!(run(
        &flat,
        w,
        h,
        ValueType::Mask,
        Settings::EdgeDetect {
            width: 3,
            threshold: 0.0
        }
    )
    .chunks_exact(4)
    .all(|p| p[3] == 0));
    let cleared = run(
        &step,
        w,
        h,
        ValueType::Scalar,
        Settings::EdgeDetect {
            width: 1,
            threshold: 1.0,
        },
    );
    assert!(cleared.chunks_exact(4).all(|p| p[0] == 0 && p[3] == 255));
}

#[test]
fn high_pass_of_a_flat_image_is_middle_gray_and_keeps_alpha() {
    let (w, h) = (9, 9);
    let mut data = Vec::new();
    for i in 0..w * h {
        data.extend_from_slice(&[37, 140, 220, if i % 2 == 0 { 255 } else { 128 }]);
    }
    let out = run(
        &data,
        w,
        h,
        ValueType::Color,
        Settings::HighPass { radius: 3 },
    );
    for (p, q) in out.chunks_exact(4).zip(data.chunks_exact(4)) {
        assert_eq!(&p[..3], &[128, 128, 128]);
        assert_eq!(p[3], q[3]);
    }
}

#[test]
fn median_removes_a_lone_pixel_and_keeps_edges() {
    let (w, h) = (9, 9);
    let salt = gray(w, h, |x, y| if (x, y) == (4, 4) { 255 } else { 20 });
    let out = run(
        &salt,
        w,
        h,
        ValueType::Color,
        Settings::Median { radius: 1 },
    );
    assert!(out.chunks_exact(4).all(|p| p == [20, 20, 20, 255]));
    // 縦の段差は中央値でそのまま
    let step = gray(w, h, |x, _| if x < 4 { 0 } else { 200 });
    let out = run(
        &step,
        w,
        h,
        ValueType::Scalar,
        Settings::Median { radius: 2 },
    );
    assert_eq!(out, step);
}

#[test]
fn median_takes_rgb_only_from_visible_pixels() {
    // 透明（RGB 255）に囲まれた見える画素 1 つ: 窓の A の中央値は 0 なので、RGB は入力のまま・A は 0
    let (w, h) = (3, 3);
    let mut data = Vec::new();
    for i in 0..9 {
        if i == 4 {
            data.extend_from_slice(&[10, 20, 30, 255]);
        } else {
            data.extend_from_slice(&[255, 255, 255, 0]);
        }
    }
    let out = run(
        &data,
        w,
        h,
        ValueType::Color,
        Settings::Median { radius: 1 },
    );
    assert_eq!(&out[16..20], &[10, 20, 30, 0]);
    assert_eq!(&out[0..4], &[255, 255, 255, 0]);
}

#[test]
fn glow_brightens_around_light_pixels_and_keeps_alpha() {
    let (w, h) = (11, 11);
    let dot = gray(w, h, |x, y| if (x, y) == (5, 5) { 255 } else { 0 });
    let out = run(
        &dot,
        w,
        h,
        ValueType::Color,
        Settings::Glow {
            threshold: 0.5,
            radius: 3,
            intensity: 4.0,
        },
    );
    assert_eq!(
        &out[((5 * w + 5) * 4) as usize..][..4],
        &[255, 255, 255, 255]
    );
    assert!(out[((5 * w + 6) * 4) as usize] > 0, "隣が明るくなる");
    assert_eq!(out[0], 0, "半径の外は変わらない");
    assert!(out.chunks_exact(4).all(|p| p[3] == 255));
    // しきい値 1 は何も変えない
    let same = run(
        &dot,
        w,
        h,
        ValueType::Color,
        Settings::Glow {
            threshold: 1.0,
            radius: 3,
            intensity: 4.0,
        },
    );
    assert_eq!(same, dot);
}

#[test]
fn directional_blur_follows_its_angle() {
    let (w, h) = (24, 24);
    // 横の縞（行ごとに一様）は、角度 0（横）のぼかしで変わらず、角度 90（縦）で変わる
    let stripes = gray(w, h, |_, y| if y % 4 < 2 { 0 } else { 255 });
    let along = run(
        &stripes,
        w,
        h,
        ValueType::Color,
        Settings::DirectionalBlur {
            angle: 0.0,
            distance: 5.0,
        },
    );
    assert_eq!(along, stripes);
    let across = run(
        &stripes,
        w,
        h,
        ValueType::Color,
        Settings::DirectionalBlur {
            angle: 90.0,
            distance: 5.0,
        },
    );
    assert_ne!(across, stripes);
    // 180 度は 0 度と同じ線（両側）
    let back = run(
        &stripes,
        w,
        h,
        ValueType::Color,
        Settings::DirectionalBlur {
            angle: 270.0,
            distance: 5.0,
        },
    );
    assert_eq!(back, across);
}

#[test]
fn slope_blur_and_warp_keep_a_flat_image_and_zero_length_does_nothing() {
    let (w, h) = (20, 20);
    let flat = gray(w, h, |_, _| 77);
    let busy = pattern(w, h);
    for mode in [SlopeMode::Blur, SlopeMode::Min, SlopeMode::Max] {
        let s = |intensity| Settings::SlopeBlur {
            intensity,
            samples: 6,
            mode,
            scale: 5.0,
            seed: 3,
        };
        assert_eq!(run(&flat, w, h, ValueType::Scalar, s(10.0)), flat);
        assert_eq!(run(&busy, w, h, ValueType::Color, s(0.0)), busy);
        assert_ne!(run(&busy, w, h, ValueType::Color, s(6.0)), busy);
    }
    let warp = |intensity| Settings::Warp {
        intensity,
        scale: 6.0,
        seed: 9,
    };
    assert_eq!(run(&flat, w, h, ValueType::Mask, warp(12.0)), {
        // マスクの結果は RGB = 0・A = 隠す量（入力の A = 255）
        flat.chunks_exact(4)
            .flat_map(|_| [0, 0, 0, 255])
            .collect::<Vec<u8>>()
    });
    assert_eq!(run(&busy, w, h, ValueType::Color, warp(0.0)), busy);
    assert_ne!(run(&busy, w, h, ValueType::Color, warp(8.0)), busy);
    // 最小は入力以下、最大は入力以上（A が 255 の灰色）
    let ramp = gray(w, h, |x, y| (x * 9 + y * 3) as u8);
    let lo = run(
        &ramp,
        w,
        h,
        ValueType::Scalar,
        Settings::SlopeBlur {
            intensity: 4.0,
            samples: 4,
            mode: SlopeMode::Min,
            scale: 7.0,
            seed: 1,
        },
    );
    let hi = run(
        &ramp,
        w,
        h,
        ValueType::Scalar,
        Settings::SlopeBlur {
            intensity: 4.0,
            samples: 4,
            mode: SlopeMode::Max,
            scale: 7.0,
            seed: 1,
        },
    );
    for ((l, h), r) in lo
        .chunks_exact(4)
        .zip(hi.chunks_exact(4))
        .zip(ramp.chunks_exact(4))
    {
        assert!(l[0] <= r[0] && r[0] <= h[0], "{l:?} {r:?} {h:?}");
    }
}

#[test]
fn the_same_pixel_has_the_same_bytes_for_any_block_size_region_and_thread_count() {
    let (w, h) = (97, 61);
    let data = pattern(w, h);
    let src = Image::new(&data, w, h).unwrap();
    let scalar = gray(w, h, |x, y| ((x * 13 + y * 7) % 256) as u8);
    let scalar_src = Image::new(&scalar, w, h).unwrap();
    for (name, settings, accepts) in kinds() {
        let (source, ty) = if accepts[0] {
            (&src, ValueType::Color)
        } else {
            (&scalar_src, ValueType::Scalar)
        };
        // 小さめの半径・長さにして、ブロック（31）の境目をまたぐ
        let settings = match settings {
            Settings::DirectionalBlur { .. } => Settings::DirectionalBlur {
                angle: 33.0,
                distance: 6.5,
            },
            Settings::Warp { .. } => Settings::Warp {
                intensity: 9.3,
                scale: 11.0,
                seed: -4,
            },
            Settings::SlopeBlur { .. } => Settings::SlopeBlur {
                intensity: 7.4,
                samples: 5,
                mode: SlopeMode::Max,
                scale: 9.0,
                seed: 12,
            },
            Settings::HighPass { .. } => Settings::HighPass { radius: 5 },
            Settings::Glow { .. } => Settings::Glow {
                threshold: 0.3,
                radius: 6,
                intensity: 2.5,
            },
            Settings::Morphology { .. } => Settings::Morphology {
                mode: MorphologyMode::Erode,
                radius: 4,
            },
            other => other,
        };
        let mut stage = Stage::new(settings);
        stage.strength = 0.75;
        let stack = [stage];
        let mut reference: Option<Vec<u8>> = None;
        for threads in [1, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            pool.install(|| {
                for block in [31, 64, 256] {
                    let options = Options {
                        block_size: block,
                        ..Options::default()
                    };
                    let got = evaluate(source, ty, &stack, full(w, h), &options).unwrap();
                    match &reference {
                        None => reference = Some(got),
                        Some(r) => {
                            assert!(&got == r, "{name}: threads={threads} block={block}")
                        }
                    }
                    // 一部の領域は、全体の結果の同じ所
                    let region = Rect::new(29, 17, 41, 23);
                    let part = evaluate(source, ty, &stack, region, &options).unwrap();
                    let whole = reference.as_ref().unwrap();
                    for y in 0..region.height {
                        let a = (((region.y + y) * w + region.x) * 4) as usize;
                        let b = (y * region.width * 4) as usize;
                        assert_eq!(
                            &whole[a..a + (region.width * 4) as usize],
                            &part[b..b + (region.width * 4) as usize],
                            "{name}: block={block} row={y}"
                        );
                    }
                }
            });
        }
    }
}

#[test]
fn transparent_pixels_keep_their_rgb_when_they_stay_transparent() {
    let (w, h) = (17, 13);
    let data = pattern(w, h);
    for (name, settings, accepts) in kinds() {
        if !accepts[0] {
            continue;
        }
        let out = run(&data, w, h, ValueType::Color, settings);
        for (i, (p, q)) in out.chunks_exact(4).zip(data.chunks_exact(4)).enumerate() {
            if p[3] == 0 && q[3] == 0 {
                assert_eq!(&p[..3], &q[..3], "{name}: 画素 {i}");
            }
        }
    }
}

#[test]
fn halo_counts_toward_the_stack_limit_and_the_working_memory() {
    let halo = |s: Settings| s.halo();
    assert_eq!(
        halo(Settings::SlopeBlur {
            intensity: 7.2,
            samples: 2,
            mode: SlopeMode::Blur,
            scale: 4.0,
            seed: 0
        }),
        8
    );
    assert_eq!(
        halo(Settings::DirectionalBlur {
            angle: 10.0,
            distance: 0.0
        }),
        0
    );
    assert_eq!(
        halo(Settings::Warp {
            intensity: 128.0,
            scale: 1.0,
            seed: 0
        }),
        128
    );
    assert_eq!(
        halo(Settings::EdgeDetect {
            width: 16,
            threshold: 0.0
        }),
        17
    );
    assert_eq!(halo(Settings::Median { radius: 16 }), 16);
    assert_eq!(
        halo(Settings::HistogramScan {
            position: 0.1,
            contrast: 0.9
        }),
        0
    );
    // 到達半径の和は 512 まで（方向のぼかし 256 + ハイパス 256 + グロー 1 は断る）
    let stack = |extra: u32| {
        vec![
            Stage::new(Settings::DirectionalBlur {
                angle: 0.0,
                distance: 256.0,
            }),
            Stage::new(Settings::HighPass { radius: 256 }),
            Stage::new(Settings::Glow {
                threshold: 0.5,
                radius: extra,
                intensity: 1.0,
            }),
        ]
    };
    assert!(block_working_bytes(&stack(1)[..2], 256, 4096, 4096).is_ok());
    assert!(block_working_bytes(&stack(1), 256, 4096, 4096).is_err());
    assert_eq!(MAX_HALO, 512);
    // 作業メモリ: 近傍の段は入力（ブロック + 半径の 2 倍の正方形）に比例する。予算が足りなければ読まずに断る
    for (name, settings, accepts) in kinds() {
        if settings.halo() == 0 {
            continue;
        }
        let stack = [Stage::new(settings)];
        let bytes = block_working_bytes(&stack, 64, 1024, 1024).unwrap();
        let side = u64::from(64 + 2 * stack[0].settings.halo());
        assert!(bytes >= side * side * 4, "{name}: {bytes}");
        let data = gray(64, 64, |x, _| x as u8);
        let src = Image::new(&data, 64, 64).unwrap();
        let ty = if accepts[0] {
            ValueType::Color
        } else {
            ValueType::Scalar
        };
        let tight = Options {
            working_budget: 1024,
            block_size: 64,
            ..Options::default()
        };
        assert!(
            matches!(
                evaluate(&src, ty, &stack, full(64, 64), &tight),
                Err(Error::Budget { .. })
            ),
            "{name}"
        );
    }
}

#[test]
fn ranges_accept_their_limits_and_refuse_just_beyond() {
    let ok = |s: Settings| assert!(s.validate_values().is_ok(), "{s:?}");
    let ng = |s: Settings| assert!(s.validate_values().is_err(), "{s:?}");
    for v in [0.0, 1.0] {
        ok(Settings::HistogramScan {
            position: v,
            contrast: v,
        });
        ok(Settings::HistogramRange {
            range: v,
            position: v,
        });
    }
    for v in [-0.001, 1.001, f64::NAN] {
        ng(Settings::HistogramScan {
            position: v,
            contrast: 0.0,
        });
        ng(Settings::HistogramRange {
            range: 0.5,
            position: v,
        });
    }
    let slope = |intensity, samples, scale| Settings::SlopeBlur {
        intensity,
        samples,
        mode: SlopeMode::Blur,
        scale,
        seed: i32::MIN,
    };
    ok(slope(0.0, 1, 1.0));
    ok(slope(64.0, 32, 256.0));
    for s in [
        slope(-0.1, 8, 16.0),
        slope(64.1, 8, 16.0),
        slope(8.0, 0, 16.0),
        slope(8.0, 33, 16.0),
        slope(8.0, 8, 0.99),
        slope(8.0, 8, 256.5),
        slope(f64::INFINITY, 8, 16.0),
    ] {
        ng(s);
    }
    let dir = |angle, distance| Settings::DirectionalBlur { angle, distance };
    ok(dir(0.0, 0.0));
    ok(dir(360.0, 256.0));
    for s in [
        dir(-1.0, 8.0),
        dir(361.0, 8.0),
        dir(0.0, 256.1),
        dir(0.0, -1.0),
    ] {
        ng(s);
    }
    let warp = |intensity, scale| Settings::Warp {
        intensity,
        scale,
        seed: 0,
    };
    ok(warp(0.0, 1.0));
    ok(warp(128.0, 256.0));
    ng(warp(128.5, 32.0));
    ng(warp(16.0, 0.5));
    type Make = fn(u32) -> Settings;
    let int_kinds: [(Make, u32, u32); 5] = [
        (
            |r| Settings::Morphology {
                mode: MorphologyMode::Erode,
                radius: r,
            },
            1,
            64,
        ),
        (
            |r| Settings::EdgeDetect {
                width: r,
                threshold: 0.5,
            },
            1,
            16,
        ),
        (|r| Settings::HighPass { radius: r }, 1, 256),
        (|r| Settings::Median { radius: r }, 1, 16),
        (
            |r| Settings::Glow {
                threshold: 0.5,
                radius: r,
                intensity: 1.0,
            },
            1,
            256,
        ),
    ];
    for (make, lo, hi) in int_kinds {
        ok(make(lo));
        ok(make(hi));
        ng(make(lo - 1));
        ng(make(hi + 1));
    }
    for v in [0.0, 4.0] {
        ok(Settings::Glow {
            threshold: 1.0,
            radius: 1,
            intensity: v,
        });
    }
    ng(Settings::Glow {
        threshold: 0.5,
        radius: 1,
        intensity: 4.01,
    });
    ng(Settings::EdgeDetect {
        width: 1,
        threshold: 1.01,
    });
}

// ───────── 文書の中 ─────────

/// 文書（タイル 32、画素の層 1 つ）に段を置いて合成すると、同じ段を画像全体へ掛けた結果と同じ（タイルの境目・領域で変わらない）。
/// 拡大は長さ・半径を倍率に合わせ、範囲の端で止めたものは知らせる。取り消しで足す前へ戻る。
#[test]
fn in_a_document_the_stages_composite_like_the_whole_image_and_follow_resizes() {
    use yolu_core::{
        CanvasResampling, Channel, Document, EffectSettings, FilterSpec, FilterTarget, TileCoord,
    };
    let (w, h) = (96u32, 64u32);
    let image = pattern(w, h);
    let mut doc = Document::with_tile_size(w, h, 32).unwrap();
    let layer = doc.add_layer("a").unwrap();
    for ty in 0..h / 32 {
        for tx in 0..w / 32 {
            let mut tile = Vec::new();
            for y in 0..32 {
                let row = ((ty * 32 + y) * w + tx * 32) as usize * 4;
                tile.extend_from_slice(&image[row..row + 32 * 4]);
            }
            doc.import_tile(layer, Channel::Color, TileCoord::new(tx, ty), &tile)
                .unwrap();
        }
    }
    let plain = doc.composite(Rect::new(0, 0, w, h)).unwrap();
    for settings in [
        Settings::DirectionalBlur {
            angle: 20.0,
            distance: 7.5,
        },
        Settings::Median { radius: 3 },
        Settings::Warp {
            intensity: 6.0,
            scale: 10.0,
            seed: 5,
        },
    ] {
        let before = doc.undo_count();
        let id = doc
            .add_filter(
                layer,
                FilterTarget::Content,
                FilterSpec::new(EffectSettings::Filter(settings.clone()))
                    .channels(&[Channel::Color]),
            )
            .unwrap();
        let composite = doc.composite(Rect::new(0, 0, w, h)).unwrap();
        // 同じ段を画像全体へ（合成は下地が透明の上に通常で 1 枚なので、段の結果と同じ）
        let direct = run(&image, w, h, ValueType::Color, settings.clone());
        let part = doc.composite(Rect::new(17, 9, 50, 40)).unwrap();
        for y in 0..40u32 {
            let a = (((9 + y) * w + 17) * 4) as usize;
            assert_eq!(
                &composite[a..a + 200],
                &part[(y * 50 * 4) as usize..(y * 50 * 4 + 200) as usize],
                "{settings:?}"
            );
        }
        for (i, (c, d)) in composite
            .chunks_exact(4)
            .zip(direct.chunks_exact(4))
            .enumerate()
        {
            if d[3] > 0 {
                assert_eq!(c, d, "{settings:?}: 画素 {i}");
            }
        }
        doc.remove_filter(layer, id).unwrap();
        assert_eq!(doc.composite(Rect::new(0, 0, w, h)).unwrap(), plain);
        while doc.undo_count() > before {
            doc.undo().unwrap();
        }
        assert!(doc.layer(layer).unwrap().filters().is_empty());
    }
    // 拡大: 長さ・半径は倍率に合わせる（範囲の端で止めたものは知らせる）
    for s in [
        Settings::SlopeBlur {
            intensity: 40.0,
            samples: 4,
            mode: SlopeMode::Blur,
            scale: 8.0,
            seed: 0,
        },
        Settings::Median { radius: 3 },
    ] {
        doc.add_filter(
            layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::Filter(s)).channels(&[Channel::Color]),
        )
        .unwrap();
    }
    let report = doc
        .resize_image(w * 2, h * 2, CanvasResampling::Nearest)
        .unwrap();
    let stages: Vec<Settings> = doc
        .layer(layer)
        .unwrap()
        .filters()
        .iter()
        .map(|e| match e.settings() {
            EffectSettings::Filter(f) => f.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        stages[0],
        Settings::SlopeBlur {
            intensity: 64.0,
            samples: 4,
            mode: SlopeMode::Blur,
            scale: 16.0,
            seed: 0,
        },
        "長さは 80 を範囲の端 64 で止め、ノイズの大きさは 2 倍"
    );
    assert_eq!(stages[1], Settings::Median { radius: 6 });
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("64") && n.contains("ノイズに沿ったぼかし")),
        "知らせは範囲の端の値と、メニューと同じ段の名前を載せる: {:?}",
        report.notes
    );
}
