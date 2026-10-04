//! 合成入力4096²、ブロック256、並列度4。cargo run -p yolu-core --release --example filter_bench
//! 後半は Normalize を 256² のタイル単位で呼ぶ場合（文書がタイルごとに evaluate する形）の時間。
#[path = "../tests/filter_support/mod.rs"]
mod filter_support;
use filter_support::{pattern, stages};
use std::{hint::black_box, time::Instant};
/// 1 回ならし、3 回の中央値（ミリ秒）。
fn median_ms(mut f: impl FnMut()) -> f64 {
    f();
    let mut times: Vec<f64> = (0..3)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[1]
}
use yolu_core::{filter::*, Rect};
fn main() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    pool.install(|| {
        let data = pattern(4096, 4096, ValueType::Color);
        let src = Image::new(&data, 4096, 4096).unwrap();
        let region = Rect::new(0, 0, 4096, 4096);
        println!("Rust release / 4096² / block=256 / degree=4 / warmup=1 / samples=3");
        for spec in [
            "blur 8",
            "sharpen 3 1.7 12",
            "noise 0.47 -39 1",
            "noise 0.83 39 0",
            "levels 0.1 0.91 1.7 0.05 0.93",
            "invert",
            "normalize",
        ] {
            let stack = stages(&format!("1 {spec}"));
            let options = Options::default();
            black_box(evaluate(&src, ValueType::Color, &stack, region, &options).unwrap());
            let mut times = Vec::new();
            let mut sum = 0u64;
            for _ in 0..3 {
                let start = Instant::now();
                let output = evaluate(&src, ValueType::Color, &stack, region, &options).unwrap();
                times.push(start.elapsed().as_secs_f64() * 1000.0);
                sum += u64::from(output[output.len() / 2]);
                black_box(output);
            }
            times.sort_by(f64::total_cmp);
            println!("{spec}\t{:.3}\t{sum}", times[1]);
        }
        println!("--- Normalize を 256² のタイル 1 枚ずつ呼ぶ（4096²、中央値 ms）---");
        let tile = Rect::new(1024, 2048, 256, 256);
        let serial = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        for (label, spec) in [("normalize", "1 normalize"), ("blur 8 → normalize", "1 blur 8;1 normalize")] {
            let stack = stages(spec);
            let options = Options::default();
            let scan4 = median_ms(|| {
                black_box(statistics(&src, ValueType::Color, &stack, &options).unwrap());
            });
            let scan1 = serial.install(|| {
                median_ms(|| {
                    black_box(statistics(&src, ValueType::Color, &stack, &options).unwrap());
                })
            });
            let stats = statistics(&src, ValueType::Color, &stack, &options).unwrap();
            let given = Options {
                statistics: Some(&stats),
                ..Options::default()
            };
            let cached = median_ms(|| {
                black_box(evaluate(&src, ValueType::Color, &stack, tile, &given).unwrap());
            });
            let uncached = median_ms(|| {
                black_box(evaluate(&src, ValueType::Color, &stack, tile, &options).unwrap());
            });
            println!(
                "{label}\t統計の走査 並列度4 {scan4:.3}\t並列度1（直列）{scan1:.3}\tタイル 統計を渡す {cached:.3}\t渡さない {uncached:.3}"
            );
        }
    });
}
