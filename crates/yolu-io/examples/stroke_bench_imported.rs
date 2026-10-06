//! 取り込んだブラシ（CLIP STUDIO の .sut・同梱の取り込み済みの筆先）の、1 ストロークの速さ（`yolu-core` の `stroke_bench` と同じ手順）。
//!
//!   cargo run -p yolu-io --release --example stroke_bench_imported -- [--threads N] [--sut-dir DIR] [--bundled N] [--sizes 8,64,256,1000]
//!
//! `--sut-dir` のフォルダの .sut を、ファイルの大きさの順に 1 から数える（名前は出さない）。ブラシごとに、取り込んだままの設定の大きさ
//! （`native`）と、半径だけを大きさへ替えた各 `--sizes` で測る。`--bundled N` は同梱の筆先の先頭 N 個（番号だけで出す）。
//! 文書は 4096²、空の層。時間の意味は `stroke_bench` と同じ（1 スレッドは CPU 時間、2 以上は壁時計）。
#[path = "../../yolu-core/examples/stroke_support/mod.rs"]
mod stroke_support;

use std::path::PathBuf;

use stroke_support::{header, measure, path, row, use_cpu_time};
use yolu_core::{Brush, Rgba8};
use yolu_io::brushes::{bundled, import};

fn list(args: &[String], key: &str) -> Option<Vec<String>> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .map(|v| v.split(',').map(|s| s.to_string()).collect())
}

/// `resize` が真なら、半径だけを `size / 2` に替えて測る（設定の残りは取り込んだまま）。
fn measure_one(label: &str, brush: &Brush, size: f64, tag: &str, resize: bool) {
    let mut brush = brush.clone();
    brush.base.color = Rgba8::new(200, 60, 30, 255);
    if resize {
        brush.base.radius = size / 2.0;
    }
    let points = path(size);
    let stats = measure(&brush, false, &points, 1.0, 9);
    #[cfg(feature = "stroke-profile")]
    stroke_support::stages(&brush, false, &points);
    println!(
        "{}",
        row(
            &format!("{label}{tag}"),
            size,
            brush.base.spacing,
            false,
            &stats
        )
    );
}

fn describe(brush: &Brush) -> String {
    format!(
        "効果 {:?}・混ぜ {}・色の変化 {}・ステンシル {}・対称 {}・消しゴム {}・硬さ {:.2}・不透明度 {:.2}・流量 {:.2}・直径 {:.0}・間隔 {:.2}・筆先 {}・質感 {}・デュアル {}・散布 {:.2}・数 {}",
        brush.effect,
        u8::from(brush.mix.is_active()),
        u8::from(brush.color.is_active()),
        u8::from(brush.stencil.is_some()),
        u8::from(brush.symmetry.enabled()),
        u8::from(brush.base.erase),
        brush.base.hardness,
        brush.base.opacity,
        brush.base.flow,
        brush.base.radius * 2.0,
        brush.base.spacing,
        brush.tip.images.len() + usize::from(brush.tip.image.is_some()),
        u8::from(brush.texture.is_some()),
        u8::from(brush.dual.is_some()),
        brush.jitter.scatter,
        brush.jitter.count
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let threads: usize = list(&args, "--threads").map_or(1, |v| v[0].parse().unwrap());
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()
        .unwrap();
    use_cpu_time(threads == 1);
    let sizes: Vec<f64> = list(&args, "--sizes").map_or(vec![8.0, 64.0, 256.0, 1000.0], |v| {
        v.iter().map(|s| s.parse().unwrap()).collect()
    });
    println!(
        "# スレッド {threads}（{}）・文書 4096²",
        if threads == 1 {
            "CPU 時間"
        } else {
            "壁時計"
        }
    );
    println!("{}", header());
    if let Some(dir) = list(&args, "--sut-dir") {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir[0])
            .expect("--sut-dir のフォルダ")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("sut")))
            .collect();
        files.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0));
        for (i, path) in files.iter().enumerate() {
            let label = format!("実物の .sut {}", i + 1);
            match import(path) {
                Ok(set) => match set.brushes.first() {
                    Some(b) => {
                        eprintln!("{label}: {}", describe(&b.brush));
                        measure_one(
                            &label,
                            &b.brush,
                            b.brush.base.radius * 2.0,
                            "（native）",
                            false,
                        );
                        for &size in &sizes {
                            measure_one(&label, &b.brush, size, "", true);
                        }
                    }
                    None => eprintln!("{label}: ブラシが取り込めなかった"),
                },
                Err(e) => eprintln!("{label}: 取り込めない（{e:?}）"),
            }
        }
    }
    if let Some(n) = list(&args, "--bundled") {
        let n: usize = n[0].parse().unwrap();
        for (i, b) in bundled::krita4().brushes.iter().take(n).enumerate() {
            let label = format!("同梱の筆先 {}", i + 1);
            eprintln!("{label}: {}", describe(&b.brush));
            measure_one(
                &label,
                &b.brush,
                b.brush.base.radius * 2.0,
                "（native）",
                false,
            );
            for &size in &sizes {
                measure_one(&label, &b.brush, size, "", true);
            }
        }
    }
}
