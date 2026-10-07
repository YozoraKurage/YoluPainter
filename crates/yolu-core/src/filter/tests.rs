//! 公開の口からは確かめられない内部の決まり: 近傍の段の取消の確認の置き場所と、同時に動かすブロック数の式。
use super::*;
use std::cell::Cell;

const W: u32 = 20;
const H: u32 = 30;

/// 画像全体を 1 つのブロックとして `neighborhood` を呼ぶ。`trip` 回目の確認で取り消し、確認の回数を返す。
fn run(settings: Settings, trip: Option<usize>) -> (Result<Vec<u8>, Error>, usize) {
    let area = Rect::new(0, 0, W, H);
    let mut buf = Vec::new();
    for i in 0..W * H {
        buf.extend_from_slice(&[
            (i * 7) as u8,
            (i * 13) as u8,
            (i * 29) as u8,
            255 - (i % 5) as u8 * 20,
        ]);
    }
    let calls = Cell::new(0usize);
    let check = || {
        calls.set(calls.get() + 1);
        if trip == Some(calls.get()) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    };
    let stage = Stage::new(settings);
    let result = pixels::neighborhood(&buf, area, area, W, H, &stage, ValueType::Color, &check);
    (result, calls.get())
}

#[test]
fn blur_and_sharpen_check_every_row_of_every_pass_and_stop_at_any_check() {
    for settings in [
        Settings::GaussianBlur { radius: 1 },
        Settings::GaussianBlur { radius: 6 },
        Settings::GaussianBlur { radius: 7 },
        Settings::Sharpen {
            radius: 4,
            amount: 1.5,
            threshold: 3,
        },
    ] {
        let radius = settings.halo();
        let passes = [radius.div_ceil(3), (radius + 1) / 3, radius / 3]
            .iter()
            .filter(|&&r| r > 0)
            .count();
        let (done, total) = run(settings.clone(), None);
        assert!(done.is_ok());
        // 箱ぼかしの 1 回ごとに、横の全行と縦の全行を 1 行ずつ確認し、最後に出力の全行を確認する。
        // 縦パス・横パス・出力のどれかの確認を消すと、この下限を割る。
        let rows = (H as usize) * (2 * passes + 1);
        assert!(total >= rows, "{settings:?}: {total} < {rows}");
        for n in 1..=total {
            let (result, calls) = run(settings.clone(), Some(n));
            assert_eq!(result, Err(Error::Cancelled), "{settings:?} {n}");
            assert_eq!(calls, n, "取り消した確認より先へ進んだ: {settings:?} {n}");
        }
    }
}

#[test]
fn worker_count_follows_free_budget_and_pool_size() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    pool.install(|| {
        assert_eq!(worker_count(0, 100), 1);
        assert_eq!(worker_count(99, 100), 1);
        assert_eq!(worker_count(100, 100), 1);
        assert_eq!(worker_count(250, 100), 2);
        assert_eq!(worker_count(399, 100), 3);
        assert_eq!(worker_count(10_000, 100), 4);
        assert_eq!(worker_count(10_000, 0), 4);
    });
}

/// 0.5.0 の近傍の段（スロープぼかし・方向のぼかし・ゆがみ・モルフォロジー・エッジ検出・ハイパス・メディアン・グロー）。
fn spatial_kinds() -> Vec<(Settings, ValueType)> {
    vec![
        (
            Settings::SlopeBlur {
                intensity: 3.5,
                samples: 4,
                mode: SlopeMode::Blur,
                scale: 6.0,
                seed: 2,
            },
            ValueType::Color,
        ),
        (
            Settings::SlopeBlur {
                intensity: 3.0,
                samples: 3,
                mode: SlopeMode::Min,
                scale: 5.0,
                seed: 7,
            },
            ValueType::Color,
        ),
        (
            Settings::DirectionalBlur {
                angle: 30.0,
                distance: 4.0,
            },
            ValueType::Color,
        ),
        (
            Settings::Warp {
                intensity: 5.0,
                scale: 4.0,
                seed: 1,
            },
            ValueType::Color,
        ),
        (
            Settings::Morphology {
                mode: MorphologyMode::Dilate,
                radius: 3,
            },
            ValueType::Scalar,
        ),
        (
            Settings::EdgeDetect {
                width: 2,
                threshold: 0.05,
            },
            ValueType::Scalar,
        ),
        (Settings::HighPass { radius: 4 }, ValueType::Color),
        (Settings::Median { radius: 2 }, ValueType::Color),
        (
            Settings::Glow {
                threshold: 0.4,
                radius: 4,
                intensity: 1.5,
            },
            ValueType::Color,
        ),
    ]
}

#[test]
fn spatial_stages_check_every_row_and_stop_at_any_check() {
    for (settings, ty) in spatial_kinds() {
        let area = Rect::new(0, 0, W, H);
        let mut buf = Vec::new();
        for i in 0..W * H {
            buf.extend_from_slice(&[(i * 7) as u8, (i * 13) as u8, (i * 29) as u8, 255]);
        }
        let stage = Stage::new(settings.clone());
        let calls = Cell::new(0usize);
        let trip = Cell::new(None::<usize>);
        let check = || {
            calls.set(calls.get() + 1);
            if trip.get() == Some(calls.get()) {
                Err(Error::Cancelled)
            } else {
                Ok(())
            }
        };
        assert!(pixels::neighborhood(&buf, area, area, W, H, &stage, ty, &check).is_ok());
        let total = calls.get();
        // 出力の行ごとに 1 回は確かめる
        assert!(total >= H as usize, "{settings:?}: {total}");
        for n in [1, total / 2, total] {
            calls.set(0);
            trip.set(Some(n));
            let result = pixels::neighborhood(&buf, area, area, W, H, &stage, ty, &check);
            assert_eq!(result, Err(Error::Cancelled), "{settings:?} {n}");
            assert_eq!(
                calls.get(),
                n,
                "取り消した確認より先へ進んだ: {settings:?} {n}"
            );
        }
    }
}

#[test]
fn new_stages_give_the_same_bytes_on_every_simd_path() {
    use crate::math::simd::forced;
    let (w, h) = (41u32, 37u32);
    let mut data = Vec::new();
    for i in 0..w * h {
        data.extend_from_slice(&[
            (i * 7) as u8,
            (i * 13 + 5) as u8,
            (i * 29) as u8,
            if i % 6 == 0 { 0 } else { (i * 11) as u8 },
        ]);
    }
    let src = Image::new(&data, w, h).unwrap();
    // 値の表の 2 種（ヒストグラムスキャン・レンジ）も、表を引く行の道が SIMD なので同じ形で確かめる
    let point = [
        (
            Settings::HistogramScan {
                position: 0.4,
                contrast: 0.7,
            },
            ValueType::Mask,
        ),
        (
            Settings::HistogramRange {
                range: 0.3,
                position: 0.65,
            },
            ValueType::Scalar,
        ),
    ];
    for (settings, ty) in spatial_kinds().into_iter().chain(point) {
        // 前後に SIMD の道を持つ段（レベル補正・ぼかし）を挟む
        let mut stage = Stage::new(settings.clone());
        stage.strength = 0.6;
        let stack = [
            Stage::new(Settings::Levels {
                input_black: 0.1,
                input_white: 0.9,
                gamma: 1.3,
                output_black: 0.0,
                output_white: 1.0,
            }),
            stage,
            Stage::new(Settings::GaussianBlur { radius: 2 }),
        ];
        let run = || {
            evaluate(
                &src,
                ty,
                &stack,
                Rect::new(0, 0, w, h),
                &Options {
                    block_size: 16,
                    ..Options::default()
                },
            )
            .unwrap()
        };
        let expected = forced::with_level(crate::math::simd::Level::Scalar, run);
        for level in forced::supported() {
            assert!(
                forced::with_level(level, run) == expected,
                "{settings:?}: {level:?}"
            );
        }
    }
}

#[test]
fn coarse_spatial_stages_stay_in_range_or_drop_out() {
    let edges = [
        Settings::SlopeBlur {
            intensity: 64.0,
            samples: 1,
            mode: SlopeMode::Max,
            scale: 1.0,
            seed: 0,
        },
        Settings::Warp {
            intensity: 0.5,
            scale: 1.0,
            seed: 3,
        },
        Settings::DirectionalBlur {
            angle: 360.0,
            distance: 256.0,
        },
        Settings::Morphology {
            mode: MorphologyMode::Erode,
            radius: 1,
        },
        Settings::EdgeDetect {
            width: 1,
            threshold: 1.0,
        },
        Settings::HighPass { radius: 1 },
        Settings::Median { radius: 16 },
        Settings::Glow {
            threshold: 0.0,
            radius: 1,
            intensity: 4.0,
        },
    ];
    for s in edges {
        for stride in [1, 2, 3, 8, 64] {
            match s.coarse(stride) {
                Some(c) => {
                    assert!(c.validate_values().is_ok(), "{s:?} /{stride}: {c:?}");
                    assert!(c.halo() <= s.halo(), "{s:?} /{stride}");
                }
                // 半径が 0 に丸まった段は外す（半径 1 の段だけ）
                None => assert!(stride > 2, "{s:?} /{stride}"),
            }
        }
        assert_eq!(s.coarse(1), Some(s.clone()));
    }
}
