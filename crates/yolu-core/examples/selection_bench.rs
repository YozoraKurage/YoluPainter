//! 選択範囲の速さの計測（tools/csharp-golden/run.sh selection-bench の C# と同じ操作・同じ入力）。
//!   cargo run --release -p yolu-core --example selection_bench [回数]
//! 4096²・タイル 128 の文書で、楕円 1800×1500 px（半径 900・750）の選択範囲を作る、拡張 10/50/200、縮小 50、境界 20、
//! ぼかし 5/50/200、鋭く。半径 200 px の楕円の拡張 50・ぼかし 50。各操作は元の選択範囲（不変）から作り直す。
//! スレッド数は RAYON_NUM_THREADS で変える（C# は BENCH_THREADS）。

use std::time::Instant;

use yolu_core::selection::DEFAULT_WORKING_BUDGET_BYTES as BUDGET;
use yolu_core::{Document, SelectionMask};

fn stats(ms: &mut [f64]) -> String {
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    format!(
        "最小 {:.2} ms / 中央 {:.2} ms（{} 回）",
        ms[0],
        ms[ms.len() / 2],
        ms.len()
    )
}

/// 2 回は捨てて runs 回測る。
fn time<R>(label: &str, runs: usize, mut f: impl FnMut() -> R) {
    let mut ms: Vec<f64> = (0..runs + 2)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(f());
            t.elapsed().as_secs_f64() * 1000.0
        })
        .skip(2)
        .collect();
    println!("{label}: {}", stats(&mut ms));
}

fn main() {
    let runs: usize = std::env::args().nth(1).map_or(5, |s| s.parse().unwrap());
    println!(
        "Rust / 論理プロセッサ {} / rayon のスレッド {}",
        std::thread::available_parallelism().map_or(0, |n| n.get()),
        rayon::current_num_threads()
    );
    let doc = Document::with_tile_size(4096, 4096, 128).unwrap();
    let ellipse = |rx: f64, ry: f64| SelectionMask::ellipse(&doc, 2048.0, 2048.0, rx, ry).unwrap();
    time(
        "選択範囲 4096² 楕円 1800×1500 を作る",
        runs,
        || ellipse(900.0, 750.0),
    );
    let big = ellipse(900.0, 750.0);
    for r in [10, 50, 200] {
        time(&format!("選択範囲 拡張 {r} px"), runs, || {
            big.grow(r, BUDGET).unwrap()
        });
    }
    time("選択範囲 縮小 50 px", runs, || {
        big.shrink(50, false, BUDGET).unwrap()
    });
    time("選択範囲 境界 20 px", runs, || {
        big.border(20, false, BUDGET).unwrap()
    });
    for r in [5.0, 50.0, 200.0] {
        time(&format!("選択範囲 ぼかし {r} px"), runs, || {
            big.feather(r, false, BUDGET).unwrap()
        });
    }
    time("選択範囲 鋭く", runs, || big.sharpen());
    let small = ellipse(200.0, 200.0);
    time(
        "選択範囲 半径 200 px の楕円の拡張 50",
        runs,
        || small.grow(50, BUDGET).unwrap(),
    );
    time(
        "選択範囲 半径 200 px の楕円のぼかし 50",
        runs,
        || small.feather(50.0, false, BUDGET).unwrap(),
    );
}
