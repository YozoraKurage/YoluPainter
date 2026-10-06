use crate::filter_support;
use filter_support::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use yolu_core::{filter::*, Rect};
fn full(w: u32, h: u32) -> Rect {
    Rect::new(0, 0, w, h)
}
#[test]
fn csharp_all_bytes_and_parallelism_and_block_boundaries() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/filter");
    let cases = include_str!("../golden/filter/cases.txt");
    for degree in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(degree)
            .build()
            .unwrap();
        pool.install(|| {
            for line in cases
                .lines()
                .filter(|l| !l.starts_with('#') && !l.is_empty())
            {
                let p: Vec<_> = line.split_whitespace().collect();
                let w = p[2].parse().unwrap();
                let h = p[3].parse().unwrap();
                let ty = match p[1] {
                    "mask" => ValueType::Mask,
                    "normal" => ValueType::TangentNormal,
                    "scalar" => ValueType::Scalar,
                    _ => ValueType::Color,
                };
                let input = pattern(w, h, ty);
                let source = Image::new(&input, w, h).unwrap();
                let stack = stages(&p[4..].join(" "));
                let expected = std::fs::read(root.join(format!("{}.rgba", p[0]))).unwrap();
                let samples = Samples::load(&root, p[0], w);
                for block in [31, 128, 256] {
                    let options = Options {
                        block_size: block,
                        generators: samples.as_ref().map(|s| s as &dyn GeneratorInput),
                        ..Options::default()
                    };
                    let got = evaluate(&source, ty, &stack, full(w, h), &options).unwrap();
                    assert_eq!(got.len(), expected.len());
                    assert!(
                        got == expected,
                        "{} degree={} block={} first={:?}",
                        p[0],
                        degree,
                        block,
                        got.iter().zip(&expected).position(|(a, b)| a != b)
                    );
                    let r = Rect::new(w / 3, h / 3, (w - w / 3).min(29), (h - h / 3).min(37));
                    let crop = evaluate(&source, ty, &stack, r, &options).unwrap();
                    // 統計を先に求めて渡した評価（文書がタイルごとに呼ぶ形）も、走査しながらの評価と同じバイト
                    let stats = statistics(&source, ty, &stack, &options).unwrap();
                    let given = Options {
                        statistics: Some(&stats),
                        ..Options {
                            block_size: block,
                            generators: options.generators,
                            ..Options::default()
                        }
                    };
                    let cached = evaluate(&source, ty, &stack, r, &given).unwrap();
                    assert!(crop == cached, "統計を渡した評価 {}", p[0]);
                    for row in 0..r.height as usize {
                        let i = ((row + r.y as usize) * w as usize + r.x as usize) * 4;
                        assert_eq!(
                            &crop[row * r.width as usize * 4..(row + 1) * r.width as usize * 4],
                            &expected[i..i + r.width as usize * 4],
                            "領域 {}",
                            p[0]
                        );
                    }
                }
            }
        });
    }
}
#[test]
fn hand_blur_and_hidden_color_and_source_unchanged() {
    let mut data = vec![0; 32 * 32 * 4];
    data[(10 * 32 + 10) * 4..(10 * 32 + 10) * 4 + 4].copy_from_slice(&[255, 0, 0, 255]);
    data[(25 * 32 + 25) * 4..(25 * 32 + 25) * 4 + 4].copy_from_slice(&[7, 8, 9, 0]);
    let before = data.clone();
    let source = Image::new(&data, 32, 32).unwrap();
    let output = evaluate(
        &source,
        ValueType::Color,
        &stages("1 blur 1"),
        full(32, 32),
        &Options::default(),
    )
    .unwrap();
    assert_eq!(
        &output[(11 * 32 + 11) * 4..(11 * 32 + 11) * 4 + 4],
        &[255, 0, 0, 28]
    );
    assert_eq!(
        &output[(25 * 32 + 25) * 4..(25 * 32 + 25) * 4 + 4],
        &[7, 8, 9, 0]
    );
    assert_eq!(data, before);
}
#[test]
fn known_levels_noise_zero_normalize_constant_and_disabled() {
    let data = [17, 17, 17, 255, 99, 87, 63, 0];
    let src = Image::new(&data, 2, 1).unwrap();
    for spec in [
        "1 normalize",
        "1 noise 0 1 1",
        "0 blur 1",
        "1 levels 0 1 1 0 1",
    ] {
        assert_eq!(
            evaluate(
                &src,
                ValueType::Color,
                &stages(spec),
                full(2, 1),
                &Options::default()
            )
            .unwrap(),
            data
        );
    }
    let mut stack = stages("1 invert");
    stack[0].enabled = false;
    assert_eq!(
        evaluate(
            &src,
            ValueType::Color,
            &stack,
            full(2, 1),
            &Options::default()
        )
        .unwrap(),
        data
    );
}
#[test]
fn validation_refuses_invalid_settings_types_sizes_and_limits() {
    let data = [0; 4];
    let src = Image::new(&data, 1, 1).unwrap();
    let opt = Options::default();
    for spec in [
        "1 blur 0",
        "1 blur 257",
        "1 sharpen 65 1 0",
        "1 sharpen 1 5.1 0",
        "1 sharpen 1 1 256",
        "1 noise NaN 0 1",
        "1 noise 1.1 0 1",
        "1 levels 0.5 0.5 1 0 1",
        "1 levels 0 1 0 0 1",
        "1 levels 0 1 1 -0.1 1",
        "NaN invert",
        "1.1 invert",
    ] {
        assert!(
            evaluate(&src, ValueType::Color, &stages(spec), full(1, 1), &opt).is_err(),
            "{spec}"
        );
    }
    assert!(Image::new(&[], 0, 0).is_err());
    assert!(Image::new(&data, 2, 1).is_err());
    assert!(evaluate(&src, ValueType::Color, &[], Rect::new(1, 0, 1, 1), &opt).is_err());
    for ty in [ValueType::Scalar, ValueType::Mask] {
        assert!(evaluate(&src, ty, &stages("1 noise 0.1 1 0"), full(1, 1), &opt).is_err());
    }
    assert!(evaluate(
        &src,
        ValueType::TangentNormal,
        &stages("1 invert"),
        full(1, 1),
        &opt
    )
    .is_err());
    assert!(evaluate(
        &src,
        ValueType::Color,
        &vec![Stage::new(Settings::Invert); 33],
        full(1, 1),
        &opt
    )
    .is_err());
    assert!(evaluate(
        &src,
        ValueType::Color,
        &stages("1 blur 256;1 blur 256;1 blur 1"),
        full(1, 1),
        &opt
    )
    .is_err());
}
struct CountSource {
    reads: AtomicUsize,
    cancel: AtomicBool,
    trip: usize,
}
impl Source for CountSource {
    fn dimensions(&self) -> (u32, u32) {
        (129, 139)
    }
    fn pixel(&self, _: u32, _: u32) -> [u8; 4] {
        if self.reads.fetch_add(1, Ordering::Relaxed) >= self.trip {
            self.cancel.store(true, Ordering::Relaxed);
        }
        [3, 5, 8, 255]
    }
}
#[test]
fn budget_and_pre_cancel_do_not_read_source() {
    let source = CountSource {
        reads: AtomicUsize::new(0),
        cancel: AtomicBool::new(false),
        trip: usize::MAX,
    };
    let opts = Options {
        working_budget: 1,
        ..Options::default()
    };
    assert!(matches!(
        evaluate(
            &source,
            ValueType::Color,
            &stages("1 blur 4"),
            full(129, 139),
            &opts
        ),
        Err(Error::Budget { .. })
    ));
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    source.cancel.store(true, Ordering::Relaxed);
    let opts = Options {
        cancel: Some(&source.cancel),
        ..Options::default()
    };
    assert_eq!(
        evaluate(&source, ValueType::Color, &[], full(129, 139), &opts),
        Err(Error::Cancelled)
    );
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
}
/// 並列度 1 のプール。ブロックは順に 1 つずつ処理され、取消の位置と読み出しの数が決まる。
fn single<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
        .install(f)
}
fn single_opts<'a>(source: &'a CountSource, block: u32) -> Options<'a> {
    Options {
        block_size: block,
        cancel: Some(&source.cancel),
        ..Options::default()
    }
}
fn counting(trip: usize) -> CountSource {
    CountSource {
        reads: AtomicUsize::new(0),
        cancel: AtomicBool::new(false),
        trip,
    }
}
#[test]
fn cancel_while_loading_stops_at_the_end_of_the_row() {
    // 幅 129。300 回目の読み出しで立て、その行を読み終えたところ（3 行 = 387 回）で止まる。全画素は 17931 回。
    for spec in ["1 blur 4", "1 invert", "1 sharpen 3 1 0"] {
        let source = counting(300);
        let result = single(|| {
            evaluate(
                &source,
                ValueType::Color,
                &stages(spec),
                full(129, 139),
                &single_opts(&source, 256),
            )
        });
        assert_eq!(result, Err(Error::Cancelled), "{spec}");
        assert_eq!(source.reads.load(Ordering::Relaxed), 387, "{spec}");
    }
}
#[test]
fn cancel_in_the_global_scan_stops_at_the_block_boundary() {
    // ブロック 32 の走査。最初のブロックを読み終える最後の読み出しで立てると、次のブロックを始めずに止まる。
    // 統計を取る段（1 段目の Normalize、または 2 段目）の入力は、ブロックの左下の 32² か、ぼかしの半径 2 を足した 34²。
    for (spec, first_block) in [("1 normalize", 32 * 32), ("1 blur 2;1 normalize", 34 * 34)] {
        let source = counting(first_block - 1);
        let options = single_opts(&source, 32);
        let result = single(|| {
            evaluate(
                &source,
                ValueType::Color,
                &stages(spec),
                full(129, 139),
                &options,
            )
        });
        assert_eq!(result, Err(Error::Cancelled), "{spec}");
        assert_eq!(source.reads.load(Ordering::Relaxed), first_block, "{spec}");
        // 統計だけを求める口も同じ
        let source = counting(first_block - 1);
        let options = single_opts(&source, 32);
        let result = single(|| statistics(&source, ValueType::Color, &stages(spec), &options));
        assert_eq!(result, Err(Error::Cancelled), "{spec} statistics");
        assert_eq!(source.reads.load(Ordering::Relaxed), first_block);
    }
}
/// sample の呼び出しを数え、trip 回目で取消を立てる。
struct TripGen<'a> {
    calls: AtomicUsize,
    trip: usize,
    cancel: &'a AtomicBool,
}
impl GeneratorInput for TripGen<'_> {
    fn sample(&self, _: u32, _: u32, _: u32) -> Option<Generated> {
        if self.calls.fetch_add(1, Ordering::Relaxed) >= self.trip {
            self.cancel.store(true, Ordering::Relaxed);
        }
        Some(Generated::Scalar(0.5))
    }
}
#[test]
fn cancel_in_generator_sampling_stops_at_the_row_and_late_cancel_is_still_cancelled() {
    let data: Vec<u8> = [10, 20, 30, 255].repeat(129 * 139);
    let source = Image::new(&data, 129, 139).unwrap();
    // (段, ブロック, 立てる呼び出し番号, 止まるまでの呼び出し数)
    for (spec, block, trip, calls) in [
        // 5 行目の最後で立てる → 行頭の確認で止まり、5 行 = 645 回
        ("1 generator replace", 256, 129 * 5 - 1, 129 * 5),
        // 途中で立てる → その行を終えるまで
        ("1 generator replace", 256, 300, 387),
        // 統計の走査の中の Generator（ブロック 32 の 3 行目の最後）
        ("1 generator replace;1 normalize", 32, 32 * 3 - 1, 32 * 3),
        // 最後の 1 回で立てる → 全部の計算が済んでも、完成画像は返さない
        ("1 generator replace", 256, 129 * 139 - 1, 129 * 139),
    ] {
        let cancel = AtomicBool::new(false);
        let generator = TripGen {
            calls: AtomicUsize::new(0),
            trip,
            cancel: &cancel,
        };
        let options = Options {
            block_size: block,
            cancel: Some(&cancel),
            generators: Some(&generator),
            ..Options::default()
        };
        let result = single(|| {
            evaluate(
                &source,
                ValueType::Color,
                &stages(spec),
                full(129, 139),
                &options,
            )
        });
        assert_eq!(result, Err(Error::Cancelled), "{spec} {trip}");
        assert_eq!(
            generator.calls.load(Ordering::Relaxed),
            calls,
            "{spec} {trip}"
        );
    }
}
/// 同時に pixel を読む数を数える。並列性を要求する場合だけ最初の2ワーカーを待ち合わせる。
struct Probe {
    data: Vec<u8>,
    width: u32,
    height: u32,
    active: AtomicUsize,
    peak: AtomicUsize,
    rendezvous: Option<(std::sync::Mutex<usize>, std::sync::Condvar)>,
}
impl Probe {
    fn new(width: u32, height: u32) -> Self {
        Self {
            data: pattern(width, height, ValueType::Color),
            width,
            height,
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            rendezvous: None,
        }
    }
    fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
}
impl Source for Probe {
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        if let Some((arrived, ready)) = &self.rendezvous {
            let mut count = arrived.lock().unwrap();
            if *count < 2 {
                *count += 1;
                ready.notify_all();
                // 実行速度ではなく2ワーカーの同時到達を検証する。上限は直列化の退行で固まるのを防ぐだけ。
                let (count, timeout) = ready
                    .wait_timeout_while(count, Duration::from_secs(120), |n| *n < 2)
                    .unwrap();
                assert!(
                    !timeout.timed_out() || *count >= 2,
                    "2つ目のワーカーが入力に到達しない"
                );
            }
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.data[i..i + 4].try_into().unwrap()
    }
}
fn pool4<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap()
        .install(f)
}
#[test]
fn budget_sets_the_number_of_simultaneous_blocks_and_never_changes_bytes() {
    // 67×71、ブロック 16、blur 5 + normalize + invert。1 ブロックの作業量を手で数える:
    //   ぼかしの入力 a(5) = 26×26 = 676 画素、出力 a(0) = 16×16 = 256 画素、縦パスの行の累積 26×16 = 416 バイト
    //   → 676×28 + 256×4 + 416 = 20368（点の段は 256×4 = 1024 で下回る）。返却画像は 67×71×4 = 19028。
    const WORKING: u64 = 20368;
    const OUTPUT: u64 = 67 * 71 * 4;
    let stack = stages("1 blur 5;1 normalize;0.5 invert");
    assert_eq!(block_working_bytes(&stack, 16, 67, 71), Ok(WORKING));
    let reference = Probe::new(67, 71);
    let expected = evaluate(
        &reference,
        ValueType::Color,
        &stack,
        full(67, 71),
        &Options {
            block_size: 16,
            ..Options::default()
        },
    )
    .unwrap();
    for (budget, limit) in [
        (OUTPUT + WORKING, 1),
        (OUTPUT + 2 * WORKING, 2),
        (OUTPUT + 3 * WORKING, 3),
        (256 * 1024 * 1024, 4),
    ] {
        let mut probe = Probe::new(67, 71);
        if limit == 4 {
            probe.rendezvous = Some((std::sync::Mutex::new(0), std::sync::Condvar::new()));
        }
        let options = Options {
            block_size: 16,
            working_budget: budget,
            ..Options::default()
        };
        let got = pool4(|| evaluate(&probe, ValueType::Color, &stack, full(67, 71), &options));
        assert_eq!(got.unwrap(), expected, "budget {budget}");
        assert!(
            probe.peak() <= limit,
            "budget {budget}: 同時に {} ブロック（上限 {limit}）",
            probe.peak()
        );
        if limit == 4 {
            assert!(probe.peak() >= 2, "予算に余裕があれば並列に動く");
        }
    }
    // 見積りぴったりまでは通り、1 バイト足りなければ読む前に断る（根拠は上の手計算の値）
    let probe = Probe::new(67, 71);
    let options = Options {
        block_size: 16,
        working_budget: OUTPUT + WORKING - 1,
        ..Options::default()
    };
    assert_eq!(
        evaluate(&probe, ValueType::Color, &stack, full(67, 71), &options),
        Err(Error::Budget {
            needed: OUTPUT + WORKING,
            budget: OUTPUT + WORKING - 1
        })
    );
    assert_eq!(probe.peak(), 0);
}
#[test]
fn statistics_scan_runs_blocks_in_parallel_within_the_budget() {
    // normalize だけ: 1 ブロックの作業は 16×16×4 = 1024 バイト。25 ブロックを走査する。
    let stack = stages("1 normalize");
    assert_eq!(block_working_bytes(&stack, 16, 67, 71), Ok(1024));
    let mut results = Vec::new();
    for (budget, limit) in [(1024, 1), (2 * 1024, 2), (256 * 1024 * 1024, 4)] {
        let mut probe = Probe::new(67, 71);
        if limit == 4 {
            probe.rendezvous = Some((std::sync::Mutex::new(0), std::sync::Condvar::new()));
        }
        let options = Options {
            block_size: 16,
            working_budget: budget,
            ..Options::default()
        };
        results.push(pool4(|| statistics(&probe, ValueType::Color, &stack, &options)).unwrap());
        assert!(probe.peak() <= limit, "budget {budget}: {}", probe.peak());
        if limit == 4 {
            assert!(probe.peak() >= 2, "予算に余裕があれば走査は並列に動く");
        }
    }
    assert!(results.windows(2).all(|w| w[0] == w[1]));
    let probe = Probe::new(67, 71);
    let options = Options {
        block_size: 16,
        working_budget: 1023,
        ..Options::default()
    };
    assert_eq!(
        statistics(&probe, ValueType::Color, &stack, &options),
        Err(Error::Budget {
            needed: 1024,
            budget: 1023
        })
    );
}
#[test]
fn working_bytes_match_hand_counts() {
    // 1 段あたり 入力画素 × 28 + 出力画素 × 4 + 縦パスの行の累積（幅 × 16）。点の段は 入力画素 × 4。
    let cases = [
        // 1×1 の blur 1: 28 + 4 + 16
        ("1 blur 1", 256, 1, 1, 48),
        // 100×100、ブロック 16、blur 2: a(2) = 20² = 400、出力 16² = 256、行 20×16 = 320 → 11200 + 1024 + 320
        ("1 blur 2", 16, 100, 100, 12544),
        // 点の段だけ: 16² × 4
        ("1 invert", 16, 100, 100, 1024),
        // 無効な段（強さ 0）は数えない
        ("0 blur 5", 16, 100, 100, 1024),
        // 細長い画像（1000×1）では縦パスの行の累積が大きい: a(10) = 276×1、出力 256×1、行 276×16
        // → 276×28 + 256×4 + 4416 = 13168（行の累積を除くと 8752）
        ("1 blur 10", 256, 1000, 1, 13168),
        // 段が重なる: 半径の合計 5。最初のぼかしが最大（676×28 + 484×4 + 416 = 21280）。点の段・後のぼかしは下回る
        ("1 blur 2;1 invert;1 blur 3", 16, 100, 100, 21280),
    ];
    for (spec, block, w, h, expected) in cases {
        assert_eq!(
            block_working_bytes(&stages(spec), block, w, h),
            Ok(expected),
            "{spec} {w}×{h} block {block}"
        );
    }
}
struct Gen(Option<Generated>);
impl GeneratorInput for Gen {
    fn sample(&self, _: u32, _: u32, _: u32) -> Option<Generated> {
        self.0
    }
}
#[test]
fn generator_missing_values_alpha_mask_visibility_and_invalid_values() {
    let data = [100, 150, 200, 128, 7, 8, 9, 0];
    let src = Image::new(&data, 2, 1).unwrap();
    let stack = [Stage::new(Settings::Generator {
        slot: 0,
        blend: GeneratorBlend::Replace,
    })];
    assert!(evaluate(
        &src,
        ValueType::Color,
        &stack,
        full(2, 1),
        &Options::default()
    )
    .is_err());
    for (value, expected) in [
        (None, data),
        (
            Some(Generated::Scalar(0.5)),
            [128, 128, 128, 128, 7, 8, 9, 0],
        ),
        (
            Some(Generated::Mapped([21, 43, 65, 128])),
            [21, 43, 65, 64, 7, 8, 9, 0],
        ),
    ] {
        let gen = Gen(value);
        let opt = Options {
            generators: Some(&gen),
            ..Options::default()
        };
        assert_eq!(
            evaluate(&src, ValueType::Color, &stack, full(2, 1), &opt).unwrap(),
            expected
        );
    }
    let gen = Gen(Some(Generated::Scalar(0.5)));
    let opt = Options {
        generators: Some(&gen),
        ..Options::default()
    };
    assert_eq!(
        evaluate(&src, ValueType::Mask, &stack, full(2, 1), &opt).unwrap(),
        [0, 0, 0, 127, 0, 0, 0, 127]
    );
    let gen = Gen(Some(Generated::Scalar(f64::NAN)));
    let opt = Options {
        generators: Some(&gen),
        ..Options::default()
    };
    assert!(evaluate(&src, ValueType::Color, &stack, full(2, 1), &opt).is_err());
}

/// 寸法だけを返し、画素を読まれたら失敗する。
struct Huge(u32, u32);
impl Source for Huge {
    fn dimensions(&self) -> (u32, u32) {
        (self.0, self.1)
    }
    fn pixel(&self, _: u32, _: u32) -> [u8; 4] {
        panic!("寸法が不正なら画素を読んではならない");
    }
}
#[test]
fn huge_dimensions_are_rejected_without_overflow_panic_or_reading() {
    let over = i32::MAX as u32 + 1;
    assert!(Image::new(&[], u32::MAX, u32::MAX).is_err());
    // block は正常値のまま、幅・高さだけが範囲外（i32::MAX を超える）
    for (w, h) in [
        (u32::MAX, u32::MAX),
        (over, 1),
        (1, over),
        (u32::MAX, 1),
        (0, 5),
        (5, 0),
    ] {
        assert!(
            matches!(
                block_working_bytes(&stages("1 blur 1"), 256, w, h),
                Err(Error::Invalid(_))
            ),
            "{w}×{h}"
        );
        let source = Huge(w, h);
        let opt = Options::default();
        assert!(
            matches!(
                evaluate(&source, ValueType::Color, &[], Rect::new(0, 0, 1, 1), &opt),
                Err(Error::Invalid(_))
            ),
            "evaluate {w}×{h}"
        );
        assert!(
            matches!(
                statistics(&source, ValueType::Color, &[], &opt),
                Err(Error::Invalid(_))
            ),
            "statistics {w}×{h}"
        );
    }
    // 範囲の上端（i32::MAX）は寸法として通り、見積りの乗算が溢れない。返却画像が予算を超えるので、画素を読む前に断る
    let max = i32::MAX as u32;
    assert!(block_working_bytes(&stages("1 blur 256"), 4096, max, max).is_ok());
    assert!(matches!(
        evaluate(
            &Huge(max, max),
            ValueType::Color,
            &stages("1 blur 1"),
            Rect::new(0, 0, max, max),
            &Options::default()
        ),
        Err(Error::Budget { .. })
    ));
    // 領域の端の足し算も溢れない
    let source = Huge(8, 8);
    for region in [
        Rect::new(u32::MAX, 0, 1, 1),
        Rect::new(0, u32::MAX, 1, 1),
        Rect::new(1, 1, u32::MAX, 1),
        Rect::new(7, 7, 2, 1),
    ] {
        assert!(matches!(
            evaluate(&source, ValueType::Color, &[], region, &Options::default()),
            Err(Error::Invalid(_))
        ));
    }
    // block の範囲
    for block in [0, 4097, u32::MAX] {
        assert!(block_working_bytes(&[], block, 1, 1).is_err());
        let opt = Options {
            block_size: block,
            ..Options::default()
        };
        assert!(evaluate(&Huge(1, 1), ValueType::Color, &[], full(1, 1), &opt).is_err());
    }
    for block in [1, 4096] {
        assert!(block_working_bytes(&[], block, 1, 1).is_ok());
    }
}
struct VaryingGenerator;
impl GeneratorInput for VaryingGenerator {
    fn sample(&self, _: u32, x: u32, _: u32) -> Option<Generated> {
        Some(Generated::Scalar(f64::from(x % 7) / 6.0))
    }
}
#[test]
fn csharp_generator_all_seven_blends_and_mask_visibility() {
    let expected = include_bytes!("../golden/filter/generator-combine.rgba");
    let data: Vec<_> = (0..256u32)
        .flat_map(|x| [x as u8, (255 - x) as u8, (x * 17 % 256) as u8, x as u8])
        .collect();
    let source = Image::new(&data, 256, 1).unwrap();
    let mut actual = Vec::new();
    for blend in [
        GeneratorBlend::Multiply,
        GeneratorBlend::Replace,
        GeneratorBlend::Screen,
        GeneratorBlend::Max,
        GeneratorBlend::Min,
        GeneratorBlend::Add,
        GeneratorBlend::Subtract,
    ] {
        for ty in [ValueType::Color, ValueType::Mask] {
            for strength in [0.01, 0.37, 0.5, 1.0] {
                let stack = [Stage {
                    settings: Settings::Generator { slot: 0, blend },
                    enabled: true,
                    strength,
                }];
                let opt = Options {
                    generators: Some(&VaryingGenerator),
                    ..Options::default()
                };
                actual.extend(evaluate(&source, ty, &stack, full(256, 1), &opt).unwrap());
            }
        }
    }
    assert_eq!(actual.as_slice(), expected);
}

fn one_pixel_result(settings: Settings, ty: ValueType) -> Result<Vec<u8>, Error> {
    let data = [100, 150, 200, 255];
    let src = Image::new(&data, 1, 1).unwrap();
    let generator = Gen(Some(Generated::Scalar(0.5)));
    let opt = Options {
        generators: Some(&generator),
        ..Options::default()
    };
    evaluate(&src, ty, &[Stage::new(settings)], full(1, 1), &opt)
}
fn every_settings() -> Vec<(&'static str, Settings)> {
    vec![
        ("blur", Settings::GaussianBlur { radius: 4 }),
        (
            "sharpen",
            Settings::Sharpen {
                radius: 2,
                amount: 1.0,
                threshold: 0,
            },
        ),
        (
            "noise_mono",
            Settings::Noise {
                amount: 0.2,
                seed: 1,
                monochrome: true,
            },
        ),
        (
            "noise_color",
            Settings::Noise {
                amount: 0.2,
                seed: 1,
                monochrome: false,
            },
        ),
        (
            "levels",
            Settings::Levels {
                input_black: 0.0,
                input_white: 1.0,
                gamma: 1.0,
                output_black: 0.0,
                output_white: 1.0,
            },
        ),
        ("invert", Settings::Invert),
        ("normalize", Settings::Normalize),
        (
            "generator",
            Settings::Generator {
                slot: 0,
                blend: GeneratorBlend::Replace,
            },
        ),
    ]
}
/// C# FilterSettings.Accepts と同じ表: 色はすべて、スカラーとマスクは色ノイズ以外、接空間法線はぼかしだけ。
fn accepted(name: &str, ty: ValueType) -> bool {
    match ty {
        ValueType::Color => true,
        ValueType::Scalar | ValueType::Mask => name != "noise_color",
        ValueType::TangentNormal => name == "blur",
    }
}
#[test]
fn every_setting_is_accepted_or_refused_per_value_type() {
    let mut messages = Vec::new();
    for ty in [
        ValueType::Color,
        ValueType::Scalar,
        ValueType::TangentNormal,
        ValueType::Mask,
    ] {
        for (name, settings) in every_settings() {
            let result = one_pixel_result(settings.clone(), ty);
            if accepted(name, ty) {
                assert!(result.is_ok(), "{name} は {ty:?} に付けられる: {result:?}");
            } else {
                match result {
                    Err(Error::Invalid(m)) => messages.push(m),
                    other => panic!("{name} は {ty:?} に付けられない: {other:?}"),
                }
                assert!(settings.validate(ty).is_err());
            }
        }
    }
    // 拒否の文は理由だけで、使い方の指示（〜してください）にしない
    assert!(!messages.is_empty());
    for m in &messages {
        assert!(!m.contains("ください"), "{m}");
    }
}
#[test]
fn setting_ranges_accept_their_limits_and_refuse_just_beyond() {
    let ok = |s: Settings| assert!(s.validate(ValueType::Color).is_ok(), "{s:?}");
    let ng = |s: Settings| match s.validate(ValueType::Color) {
        Err(Error::Invalid(m)) => assert!(!m.contains("ください"), "{m}"),
        other => panic!("{s:?}: {other:?}"),
    };
    let blur = |radius| Settings::GaussianBlur { radius };
    for r in [1, 2, 255, 256] {
        ok(blur(r));
    }
    for r in [0, 257, u32::MAX] {
        ng(blur(r));
    }
    let sharpen = |radius, amount, threshold| Settings::Sharpen {
        radius,
        amount,
        threshold,
    };
    for (r, a, t) in [(1, 0.0, 0), (64, 5.0, 255), (3, 2.5, 128)] {
        ok(sharpen(r, a, t));
    }
    for (r, a, t) in [
        (0, 1.0, 0),
        (65, 1.0, 0),
        (2, -0.001, 0),
        (2, 5.001, 0),
        (2, f64::NAN, 0),
        (2, f64::INFINITY, 0),
        (2, 1.0, 256),
    ] {
        ng(sharpen(r, a, t));
    }
    let noise = |amount| Settings::Noise {
        amount,
        seed: 0,
        monochrome: true,
    };
    for a in [0.0, 0.5, 1.0] {
        ok(noise(a));
    }
    for a in [-0.001, 1.001, f64::NAN, f64::INFINITY] {
        ng(noise(a));
    }
    let levels = |ib, iw, g, ob, ow| Settings::Levels {
        input_black: ib,
        input_white: iw,
        gamma: g,
        output_black: ob,
        output_white: ow,
    };
    // 入力の幅はちょうど 1/255 まで、ガンマは 0.1..9.99
    ok(levels(0.0, 1.0 / 255.0, 0.1, 0.0, 1.0));
    ok(levels(0.25, 1.0, 9.99, 1.0, 0.0));
    for bad in [
        levels(0.0, 1.0 / 255.0 - 1e-9, 1.0, 0.0, 1.0),
        levels(-0.001, 1.0, 1.0, 0.0, 1.0),
        levels(0.0, 1.001, 1.0, 0.0, 1.0),
        levels(0.0, 1.0, 0.0999, 0.0, 1.0),
        levels(0.0, 1.0, 10.0, 0.0, 1.0),
        levels(0.0, 1.0, f64::NAN, 0.0, 1.0),
        levels(0.0, 1.0, 1.0, -0.001, 1.0),
        levels(0.0, 1.0, 1.0, 0.0, 1.001),
        levels(f64::NAN, 1.0, 1.0, 0.0, 1.0),
    ] {
        ng(bad);
    }
    // 強さ: 0 と 1 は通り（0 は無効な段と同じ）、範囲外・非有限は断る
    let src_data = [1, 2, 3, 255];
    let src = Image::new(&src_data, 1, 1).unwrap();
    for (strength, accepted) in [
        (0.0, true),
        (1.0, true),
        (0.5, true),
        (-0.001, false),
        (1.001, false),
        (f64::NAN, false),
        (f64::INFINITY, false),
    ] {
        let stack = [Stage {
            settings: Settings::Invert,
            enabled: true,
            strength,
        }];
        let result = evaluate(
            &src,
            ValueType::Color,
            &stack,
            full(1, 1),
            &Options::default(),
        );
        assert_eq!(result.is_ok(), accepted, "強さ {strength}");
    }
}
#[test]
fn stack_halo_and_block_limits_accept_the_boundary_and_refuse_beyond() {
    let data = [10, 20, 30, 255, 40, 50, 60, 128];
    let src = Image::new(&data, 2, 1).unwrap();
    let run = |spec: &str, block: u32| {
        evaluate(
            &src,
            ValueType::Color,
            &stages(spec),
            full(2, 1),
            &Options {
                block_size: block,
                ..Options::default()
            },
        )
    };
    // 32 段ちょうどは通り、33 段は断る
    let stack32 = vec![Stage::new(Settings::Invert); MAX_STACK];
    let mut stack33 = stack32.clone();
    stack33.push(Stage::new(Settings::Invert));
    let go = |stack: &[Stage]| {
        evaluate(
            &src,
            ValueType::Color,
            stack,
            full(2, 1),
            &Options::default(),
        )
    };
    // 反転を偶数回: 元に戻る
    assert_eq!(go(&stack32).unwrap(), data);
    assert!(matches!(go(&stack33), Err(Error::Invalid(_))));
    // 有効な半径の合計 512 ちょうどは通り、513 は断る。無効な段・強さ 0 の段は数えない
    assert!(run("1 blur 256;1 blur 256", 256).is_ok());
    assert!(run("1 blur 256;1 sharpen 64 1 0;1 blur 192", 256).is_ok());
    assert!(matches!(
        run("1 blur 256;1 blur 256;1 blur 1", 256),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        run("1 blur 256;1 sharpen 64 1 0;1 blur 193", 256),
        Err(Error::Invalid(_))
    ));
    assert!(run("1 blur 256;1 blur 256;0 blur 5", 256).is_ok());
    let mut off = stages("1 blur 256;1 blur 256;1 blur 5");
    off[2].enabled = false;
    assert!(go(&off).is_ok());
    // ブロックは 1..=4096
    for block in [1, 31, 4096] {
        assert!(run("1 blur 3", block).is_ok(), "block {block}");
    }
    for block in [0, 4097] {
        assert!(matches!(run("1 blur 3", block), Err(Error::Invalid(_))));
    }
    // 領域は画像の内側にぴったり収まれば通る
    for region in [full(2, 1), Rect::new(1, 0, 1, 1), Rect::new(0, 0, 1, 1)] {
        assert!(evaluate(&src, ValueType::Color, &[], region, &Options::default()).is_ok());
    }
    for region in [
        Rect::new(0, 0, 3, 1),
        Rect::new(2, 0, 1, 1),
        Rect::new(0, 0, 2, 2),
        Rect::new(0, 0, 0, 1),
    ] {
        assert!(matches!(
            evaluate(&src, ValueType::Color, &[], region, &Options::default()),
            Err(Error::Invalid(_))
        ));
    }
}
#[test]
fn empty_and_inactive_stacks_return_the_input() {
    let (w, h) = (23, 17);
    let opt = Options::default();
    for ty in [
        ValueType::Color,
        ValueType::Scalar,
        ValueType::TangentNormal,
    ] {
        let input = pattern(w, h, ty);
        let src = Image::new(&input, w, h).unwrap();
        assert_eq!(
            evaluate(&src, ty, &[], full(w, h), &opt).unwrap(),
            input,
            "{ty:?}"
        );
        // 領域の切り出しも入力のまま
        let r = Rect::new(5, 4, 9, 7);
        let crop = evaluate(&src, ty, &[], r, &opt).unwrap();
        for row in 0..r.height as usize {
            let i = ((row + r.y as usize) * w as usize + r.x as usize) * 4;
            assert_eq!(
                &crop[row * r.width as usize * 4..(row + 1) * r.width as usize * 4],
                &input[i..i + r.width as usize * 4]
            );
        }
        // すべて無効（強さ 0・オフ）の段も同じ（接空間法線に付けられるのはぼかしだけ）
        let mut off = stages("0 blur 5;1 blur 3;1 blur 2");
        off[1].enabled = false;
        off[2].strength = 0.0;
        assert_eq!(evaluate(&src, ty, &off, full(w, h), &opt).unwrap(), input);
    }
    let input = pattern(w, h, ValueType::Color);
    let src = Image::new(&input, w, h).unwrap();
    let mut off =
        stages("0 blur 5;1 invert;1 normalize;1 levels 0.1 0.9 1.3 0.2 0.7;0 noise 0.5 3 0");
    off[1].enabled = false;
    off[2].enabled = false;
    off[3].strength = 0.0;
    assert_eq!(
        evaluate(&src, ValueType::Color, &off, full(w, h), &opt).unwrap(),
        input
    );
    // マスクは入力の A を隠す量として読み、RGB=0・A=隠す量で返す
    let input = pattern(w, h, ValueType::Mask);
    let src = Image::new(&input, w, h).unwrap();
    let got = evaluate(&src, ValueType::Mask, &[], full(w, h), &opt).unwrap();
    let want: Vec<u8> = input
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [0, 0, 0, p[3]])
        .collect();
    assert_eq!(got, want);
}

/// 前段までの出力の RGB（アルファが正の画素。マスクは全画素の隠す量）の最小・最大を、評価の結果から手で求める。
fn range_of_prefix(src: &Image<'_>, ty: ValueType, prefix: &[Stage], w: u32, h: u32) -> Statistics {
    let out = evaluate(src, ty, prefix, full(w, h), &Options::default()).unwrap();
    let (mut min, mut max) = (255u8, 0u8);
    for p in out.as_chunks::<4>().0 {
        if ty == ValueType::Mask {
            min = min.min(p[3]);
            max = max.max(p[3]);
        } else if p[3] > 0 {
            for &v in &p[..3] {
                min = min.min(v);
                max = max.max(v);
            }
        }
    }
    if min > max {
        Statistics { min: 0, max: 0 }
    } else {
        Statistics { min, max }
    }
}
#[test]
fn statistics_are_the_range_of_the_previous_stages_and_do_not_depend_on_parallelism() {
    let (w, h) = (53, 47);
    let cases: [(ValueType, &str); 4] = [
        (ValueType::Color, "1 blur 3;1 normalize"),
        (
            ValueType::Color,
            "1 normalize;1 levels 0.1 0.9 1.3 0.2 0.7;1 invert;1 normalize",
        ),
        (ValueType::Mask, "0.7 invert;1 normalize;0.5 blur 2"),
        (
            ValueType::Scalar,
            "0 normalize;1 sharpen 2 1.5 3;1 normalize",
        ),
    ];
    for (ty, spec) in cases {
        let input = pattern(w, h, ty);
        let src = Image::new(&input, w, h).unwrap();
        let stack = stages(spec);
        let mut expected = vec![None; stack.len()];
        for (i, s) in stack.iter().enumerate() {
            if matches!(s.settings, Settings::Normalize) && s.strength > 0.0 {
                expected[i] = Some(range_of_prefix(&src, ty, &stack[..i], w, h));
            }
        }
        for degree in [1, 4] {
            for block in [7, 31, 256] {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(degree)
                    .build()
                    .unwrap();
                let options = Options {
                    block_size: block,
                    ..Options::default()
                };
                let got = pool
                    .install(|| statistics(&src, ty, &stack, &options))
                    .unwrap();
                assert_eq!(got, expected, "{spec} degree={degree} block={block}");
            }
        }
    }
}
#[test]
fn supplied_statistics_are_used_without_scanning_and_are_validated() {
    // 3 画素。渡した統計が評価に使われる（実際の範囲は 10..200、渡すのは 0..100）
    let data = [10, 10, 10, 255, 105, 105, 105, 255, 200, 200, 200, 255];
    let src = Image::new(&data, 3, 1).unwrap();
    let stack = stages("1 normalize");
    let own = evaluate(
        &src,
        ValueType::Color,
        &stack,
        full(3, 1),
        &Options::default(),
    )
    .unwrap();
    assert_eq!(&own[..4], &[0, 0, 0, 255]);
    assert_eq!(&own[8..], &[255, 255, 255, 255]);
    let fake = [Some(Statistics { min: 0, max: 100 })];
    let options = Options {
        statistics: Some(&fake),
        ..Options::default()
    };
    assert_eq!(
        evaluate(&src, ValueType::Color, &stack, full(3, 1), &options).unwrap(),
        [26, 26, 26, 255, 255, 255, 255, 255, 255, 255, 255, 255]
    );
    // 統計を求める口は、渡した段をそのまま返す
    assert_eq!(
        statistics(&src, ValueType::Color, &stack, &options).unwrap(),
        fake
    );
    // 渡した統計があれば全画像を走査せず、領域の分しか読まない
    let counting = counting(usize::MAX);
    let region = Rect::new(10, 20, 16, 16);
    let opts = |s| Options {
        statistics: s,
        ..Options::default()
    };
    let given = [Some(Statistics { min: 3, max: 3 })];
    evaluate(
        &counting,
        ValueType::Color,
        &stack,
        region,
        &opts(Some(&given)),
    )
    .unwrap();
    assert_eq!(counting.reads.load(Ordering::Relaxed), 16 * 16);
    counting.reads.store(0, Ordering::Relaxed);
    evaluate(&counting, ValueType::Color, &stack, region, &opts(None)).unwrap();
    assert_eq!(counting.reads.load(Ordering::Relaxed), 129 * 139 + 16 * 16);
    // Normalize の無いスタックは何も走査しない
    counting.reads.store(0, Ordering::Relaxed);
    assert_eq!(
        statistics(
            &counting,
            ValueType::Color,
            &stages("1 blur 3;1 invert"),
            &opts(None)
        ),
        Ok(vec![None, None])
    );
    assert_eq!(counting.reads.load(Ordering::Relaxed), 0);
    // 形の違う統計は断る: 数が違う・Normalize でない段・無効な段・最小 > 最大
    let two = [None, None];
    let on_invert = [Some(Statistics { min: 0, max: 9 })];
    let reversed = [Some(Statistics { min: 9, max: 0 })];
    let mut disabled = stages("1 normalize");
    disabled[0].enabled = false;
    for (spec, stack, given) in [
        ("数が違う", stack.clone(), &two[..]),
        ("反転の段", stages("1 invert"), &on_invert[..]),
        ("無効な段", disabled, &fake[..]),
        ("最小が最大より大きい", stack.clone(), &reversed[..]),
    ] {
        let options = Options {
            statistics: Some(given),
            ..Options::default()
        };
        assert!(
            matches!(
                evaluate(&src, ValueType::Color, &stack, full(3, 1), &options),
                Err(Error::Invalid(_))
            ),
            "{spec}"
        );
        assert!(
            matches!(
                statistics(&src, ValueType::Color, &stack, &options),
                Err(Error::Invalid(_))
            ),
            "{spec} statistics"
        );
    }
}
#[test]
fn statistics_refuse_before_reading_when_cancelled_or_over_budget() {
    let source = counting(usize::MAX);
    let stack = stages("1 blur 3;1 normalize");
    source.cancel.store(true, Ordering::Relaxed);
    let options = Options {
        cancel: Some(&source.cancel),
        ..Options::default()
    };
    assert_eq!(
        statistics(&source, ValueType::Color, &stack, &options),
        Err(Error::Cancelled)
    );
    let working = block_working_bytes(&stack, 256, 129, 139).unwrap();
    for (budget, ok) in [(working - 1, false), (working, true)] {
        let options = Options {
            working_budget: budget,
            ..Options::default()
        };
        let result = statistics(&source, ValueType::Color, &stack, &options);
        assert_eq!(result.is_ok(), ok, "budget {budget}");
        if !ok {
            assert_eq!(
                result,
                Err(Error::Budget {
                    needed: working,
                    budget
                })
            );
        }
    }
}
