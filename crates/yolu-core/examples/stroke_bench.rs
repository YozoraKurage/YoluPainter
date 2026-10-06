//! 2D のストロークの速さ（ブラシの種類 × 大きさ × 間隔）。
//!   cargo run --release -p yolu-core --example stroke_bench -- [--threads N] [--kinds a,b] [--sizes 8,64] [--spacings 0.05,0.15]
//!       [--filled] [--runs N] [--flow F]
//! 文書は 4096²。決まった点の列（`stroke_support::path`）を毎回新しい文書へ描く。`--threads 1` はそのスレッドの CPU 時間
//! （ほかの負荷に左右されにくい）、2 以上は壁時計。種類: hard・soft・soft_flow・tip・texture・dual・mix・smear・smudge・blur・air・spray。
//! 混ぜる・指先・ぼかしは、全タイルが乱数の画素の層の上（それ以外は既定で空の層、`--filled` で乱数の画素の上）。
#[path = "stroke_support/mod.rs"]
mod stroke_support;

use stroke_support::{header, measure, path, row, use_cpu_time};
use yolu_core::{
    builtin_tip, Brush, BrushEffect, BrushSettings, ColorMix, DualBrush, MixMode, PaperTexture,
    Rgba8,
};

pub fn brush_for(kind: &str, size: f64, spacing: f64) -> Option<(Brush, bool)> {
    let radius = size / 2.0;
    let mut b = Brush::from(BrushSettings {
        radius,
        hardness: 1.0,
        spacing,
        color: Rgba8::new(200, 60, 30, 255),
        ..BrushSettings::default()
    });
    b.seed = 7;
    let mut filled = false;
    match kind {
        "hard" => {}
        "soft" => b.base.hardness = 0.0,
        "soft_flow" => {
            b.base.hardness = 0.0;
            b.base.flow = 0.3;
            b.base.opacity = 0.8;
        }
        "tip" => {
            b.base.hardness = 0.8;
            b.tip.image = builtin_tip("charcoal");
            b.tip.follow_direction = true;
        }
        "texture" => {
            b.base.hardness = 0.8;
            b.base.flow = 0.5;
            b.texture = Some(PaperTexture {
                scale: 2.0,
                ..PaperTexture::new(builtin_tip("grain").unwrap(), 0.6)
            });
        }
        "dual" => {
            b.base.hardness = 0.8;
            b.dual = Some(DualBrush {
                radius: radius / 4.0,
                spacing: 0.25,
                scatter: 0.5,
                count: 2,
                hardness: 0.5,
                ..DualBrush::default()
            });
        }
        "mix" | "smear" => {
            filled = true;
            b.base.hardness = 0.8;
            b.mix = ColorMix {
                mode: if kind == "mix" {
                    MixMode::Mix
                } else {
                    MixMode::Smear
                },
                ..ColorMix::default()
            };
        }
        "smudge" => {
            filled = true;
            b.base.hardness = 0.8;
            b.effect = BrushEffect::SMUDGE;
        }
        "blur" => {
            filled = true;
            b.base.hardness = 0.8;
            b.effect = BrushEffect::BLUR;
        }
        // エア: 柔らかい丸を低い流量で細かく重ねる
        "air" => {
            b.base.hardness = 0.0;
            b.base.flow = 0.05;
        }
        // スプレー: 粒の筆先を散らして、1 つの描点に数粒
        "spray" => {
            b.base.hardness = 0.8;
            b.base.flow = 0.5;
            b.tip.image = builtin_tip("dots");
            b.jitter.scatter = 1.0;
            b.jitter.count = 8;
            b.jitter.size = 0.5;
        }
        _ => return None,
    }
    Some((b, filled))
}

fn list(args: &[String], key: &str) -> Option<Vec<String>> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .map(|v| v.split(',').map(|s| s.to_string()).collect())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let threads: usize = list(&args, "--threads").map_or(1, |v| v[0].parse().unwrap());
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()
        .unwrap();
    use_cpu_time(threads == 1);
    let kinds = list(&args, "--kinds").unwrap_or_else(|| {
        [
            "hard",
            "soft",
            "soft_flow",
            "tip",
            "texture",
            "dual",
            "mix",
            "smear",
            "smudge",
            "blur",
            "air",
            "spray",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    });
    let sizes: Vec<f64> = list(&args, "--sizes").map_or(vec![8.0, 64.0, 256.0, 1000.0], |v| {
        v.iter().map(|s| s.parse().unwrap()).collect()
    });
    let spacings: Vec<f64> = list(&args, "--spacings").map_or(vec![0.05, 0.15, 0.5], |v| {
        v.iter().map(|s| s.parse().unwrap()).collect()
    });
    let max_runs: usize = list(&args, "--runs").map_or(9, |v| v[0].parse().unwrap());
    let force_filled = args.iter().any(|a| a == "--filled");
    // ワーカーで描くダブの外接の箱の下限（画素）。既定は組み込みの値
    if let Some(v) = list(&args, "--par-pixels") {
        yolu_core::brush::set_parallel_dab_pixels(v[0].parse().unwrap());
    }
    // 流量を上書きする（0 にすると画素への当てを省いた、覆いの計算だけの時間になる）
    let flow: Option<f64> = list(&args, "--flow").map(|v| v[0].parse().unwrap());
    let taper: Option<f64> = list(&args, "--taper").map(|v| v[0].parse().unwrap());
    println!(
        "# スレッド {threads}（{}）・文書 4096²",
        if threads == 1 {
            "CPU 時間"
        } else {
            "壁時計"
        }
    );
    println!("{}", header());
    for kind in &kinds {
        for &size in &sizes {
            for &spacing in &spacings {
                let Some((brush, filled)) = brush_for(kind, size, spacing) else {
                    eprintln!("知らない種類: {kind}");
                    continue;
                };
                let mut brush = brush;
                if let Some(f) = flow {
                    brush.base.flow = f;
                }
                // 入りと抜き（画素）を付ける
                if let Some(t) = taper {
                    brush.assist.taper_in = t;
                    brush.assist.taper_out = t;
                }
                let filled = filled || force_filled;
                let points = path(size);
                let stats = measure(&brush, filled, &points, 1.0, max_runs);
                println!("{}", row(kind, size, spacing, filled, &stats));
                #[cfg(feature = "stroke-profile")]
                stroke_support::stages(&brush, filled, &points);
            }
        }
    }
}
