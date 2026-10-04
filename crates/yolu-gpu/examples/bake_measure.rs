//! メッシュマップのベイクの計測。同じ入力を CPU（rayon）と GPU（BVH を compute でたどる道と、使えるアダプターでは ray query）で焼き、
//! 時間と CPU との差を出す。既定は 7 万三角形・2048²・光線 64 本。人工の立方体（分割して凹凸をつけたもの）を使うので、実モデルは要らない。
//!
//! ```text
//! cargo run --release -p yolu-gpu --example bake_measure
//! cargo run --release -p yolu-gpu --example bake_measure -- --triangles 70000 --size 2048 --samples 64 --runs 3
//! ```
//!
//! 引数: `--triangles N`・`--size N`・`--samples N`（AO と厚みの光線の数）・`--aa N`・`--runs N`（各条件の測定回数。1 回目は別に出す）・
//! `--software`（ソフトウェアの描画のアダプターも使う。ここでは速さの比べにならない）・`--no-cpu`（CPU を飛ばし、差も出さない）。
//! 結果はアダプターの名前・バックエンド・使った道（compute か ray query）と一緒に控える。
#[allow(dead_code)]
#[path = "../tests/support/meshes.rs"]
mod meshes;

use meshes::{cube, input_with_normals, skewed};
use std::time::Instant;
use yolu_core::mesh_maps::{
    bake, BakedMeshMap, MeshBakeBudget, MeshBakeInput, MeshBakeResult, MeshBakeSettings,
    MeshMapKind,
};
use yolu_gpu::{BakeGpu, GpuBakeMethod, GpuBakeOptions};

struct Args {
    triangles: usize,
    size: i32,
    samples: i32,
    aa: i32,
    runs: usize,
    software: bool,
    cpu: bool,
}

fn parse() -> Args {
    let mut a = Args {
        triangles: 70_000,
        size: 2048,
        samples: 64,
        aa: 1,
        runs: 3,
        software: false,
        cpu: true,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut number = |name: &str| -> usize {
            it.next()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| panic!("{name} に数を渡してください"))
        };
        match arg.as_str() {
            "--triangles" => a.triangles = number("--triangles"),
            "--size" => a.size = number("--size") as i32,
            "--samples" => a.samples = number("--samples") as i32,
            "--aa" => a.aa = number("--aa") as i32,
            "--runs" => a.runs = number("--runs").max(1),
            "--software" => a.software = true,
            "--no-cpu" => a.cpu = false,
            other => panic!("知らない引数: {other}"),
        }
    }
    a
}

/// 分割して凹凸をつけた立方体（頂点法線つき）。三角形の数は 12 × 分割数² なので、近い数にそろえる。
fn mesh(triangles: usize, bulge: f32, noise: f32, seed: u32) -> MeshBakeInput {
    let divisions = ((triangles as f64 / 12.0).sqrt().round() as usize).max(1);
    input_with_normals(&skewed(cube(divisions, bulge, noise, seed)))
}

fn settings(args: &Args, maps: &[MeshMapKind]) -> MeshBakeSettings {
    MeshBakeSettings {
        width: args.size,
        height: args.size,
        target_slot: -1,
        padding: 4,
        antialiasing: args.aa,
        maps: maps.to_vec(),
        ao_samples: args.samples,
        thickness_samples: args.samples,
        curvature_radius: 0.05,
        ..Default::default()
    }
}

/// 16 bit の値の CPU との差（最大・平均・0.5% を超えるテクセルの割合）。覆うテクセルだけを数える。
fn diff(cpu: &BakedMeshMap, gpu: &BakedMeshMap) -> (u32, f64, f64) {
    let ch = cpu.channels();
    let (mut max, mut sum, mut over, mut covered) = (0u32, 0f64, 0usize, 0usize);
    for i in 0..cpu.coverage().len() {
        if cpu.coverage()[i] == 0 || gpu.coverage()[i] == 0 {
            continue;
        }
        covered += 1;
        let mut worst = 0u32;
        for c in 0..ch {
            let e = u32::from(cpu.data()[i * ch + c].abs_diff(gpu.data()[i * ch + c]));
            sum += f64::from(e);
            worst = worst.max(e);
        }
        max = max.max(worst);
        over += usize::from(worst > 328);
    }
    (
        max,
        sum / (covered * ch).max(1) as f64,
        over as f64 / covered.max(1) as f64,
    )
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn ms(list: &[f64]) -> String {
    list.iter()
        .map(|v| format!("{v:.0}"))
        .collect::<Vec<_>>()
        .join(", ")
}

struct Case<'a> {
    name: &'a str,
    maps: Vec<MeshMapKind>,
    input: &'a MeshBakeInput,
    reference: Option<&'a MeshBakeInput>,
}

fn main() {
    let args = parse();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let low = mesh(args.triangles, 0.6, 0.02, 7);
    let high = mesh(args.triangles, 0.6, 0.05, 9);
    let proj_low = mesh(192, 0.8, 0.0, 3);
    println!(
        "入力: {} 三角形、{}²、AA {}、光線 {} 本、CPU のスレッド {threads}、測定 {} 回（1 回目は別）",
        low.triangle_count(),
        args.size,
        args.aa,
        args.samples,
        args.runs
    );
    let compute = BakeGpu::new(GpuBakeOptions {
        allow_software: args.software,
        ray_query: false,
        ..Default::default()
    });
    let mut compute = match compute {
        Ok(g) => Some(g),
        Err(e) => {
            println!("GPU: 使えません（{e}）。`--software` でソフトウェアの描画を許せます");
            None
        }
    };
    if let Some(g) = &compute {
        let a = g.adapter();
        println!("GPU: {}（{}、{}）", a.name, a.backend, a.device_type);
    }
    // ray query は、compute 用とは別に ray query つきでデバイスを作れたときだけ測る（compute 用の `ray_query: false` のデバイスは、
    // アダプターが対応していても `adapter().ray_query` が偽）。自己照合に通らなければ compute に戻るので、使った道は結果で確かめる。
    let mut rq = BakeGpu::new(GpuBakeOptions {
        allow_software: args.software,
        ray_query: true,
        ..Default::default()
    })
    .ok()
    .filter(|g| g.adapter().ray_query);
    match &rq {
        Some(g) => println!("ray query: 使えます（{}）", g.adapter().name),
        None => println!("ray query: このアダプターでは使えません（compute の道だけ測ります）"),
    }
    let raster = vec![
        MeshMapKind::WorldNormal,
        MeshMapKind::Position,
        MeshMapKind::Curvature,
        MeshMapKind::Height,
        MeshMapKind::TangentNormal,
        MeshMapKind::Id,
        MeshMapKind::Opacity,
    ];
    let rays = vec![
        MeshMapKind::AmbientOcclusion,
        MeshMapKind::Thickness,
        MeshMapKind::BentNormal,
    ];
    let cases = [
        Case {
            name: "光線の要らないマップ 7 種",
            maps: raster,
            input: &low,
            reference: None,
        },
        Case {
            name: "光線のマップ 3 種（AO・厚み・ベントノーマル）",
            maps: rays,
            input: &low,
            reference: None,
        },
        Case {
            name: "高ポリからの投影（低ポリ 192 面 ← 高ポリ）、全 10 種",
            maps: MeshMapKind::ALL.to_vec(),
            input: &proj_low,
            reference: Some(&high),
        },
    ];
    let budget = MeshBakeBudget::default();
    for case in &cases {
        let s = settings(&args, &case.maps);
        println!("\n■ {}", case.name);
        let mut cpu_result: Option<MeshBakeResult> = None;
        let mut cpu_median = None;
        if args.cpu {
            let mut times = Vec::new();
            for _ in 0..args.runs {
                let now = Instant::now();
                let r = bake(case.input, &s, &budget, None, case.reference, |_, _| true)
                    .expect("CPU のベイク");
                times.push(now.elapsed().as_secs_f64() * 1000.0);
                cpu_result = Some(r);
            }
            let m = median(&mut times.clone());
            println!("  CPU        中央値 {m:>8.0} ms   各回 [{}]", ms(&times));
            cpu_median = Some(m);
        }
        for (label, gpu) in [("GPU compute", &mut compute), ("GPU ray query", &mut rq)] {
            let Some(g) = gpu.as_mut() else { continue };
            let mut times = Vec::new();
            let mut first = 0.0;
            let mut last = None;
            for i in 0..=args.runs {
                let now = Instant::now();
                let baked = match g.bake(case.input, &s, &budget, None, case.reference, |_, _| true)
                {
                    Ok(b) => b,
                    Err(e) => {
                        println!("  {label}: 失敗（{e}）");
                        break;
                    }
                };
                let t = now.elapsed().as_secs_f64() * 1000.0;
                if i == 0 {
                    first = t;
                } else {
                    times.push(t);
                }
                last = Some(baked);
            }
            let Some(baked) = last else { continue };
            let m = median(&mut times.clone());
            let used = match baked.stats.method {
                GpuBakeMethod::Compute => "compute",
                GpuBakeMethod::RayQuery => "ray query",
            };
            let ratio =
                cpu_median.map_or(String::new(), |c| format!("  CPU の {:.2} 倍の時間", m / c));
            println!(
                "  {label:<13} 中央値 {m:>8.0} ms   1 回目 {first:.0}   各回 [{}]   使った道 {used}{ratio}",
                ms(&times)
            );
            println!(
                "      dispatch {} 回、最長 {:.1} ms（最大 {} テクセル）、帯 {}、入力 {} KiB、出力と読み戻し 1 帯 {} KiB{}",
                baked.stats.dispatches,
                baked.stats.max_dispatch_ms,
                baked.stats.max_dispatch_texels,
                baked.stats.bands,
                baked.stats.input_bytes >> 10,
                baked.stats.band_bytes >> 10,
                baked
                    .stats
                    .ray_query_note
                    .as_deref()
                    .map_or(String::new(), |n| format!("、{n}"))
            );
            let rep = &baked.result.report;
            println!(
                "      内訳 準備 {:.0} · 塗り {:.0} · 余白 {:.0} ms（CPU の余白と準備を含む）、レイ {}",
                rep.prepare_seconds * 1000.0,
                rep.raster_seconds * 1000.0,
                rep.padding_seconds * 1000.0,
                rep.rays
            );
            if let Some(cpu) = &cpu_result {
                for (c, g) in cpu.maps.iter().zip(&baked.result.maps) {
                    let (max, mean, over) = diff(c, g);
                    println!(
                        "      {:>16}: 最大差 {max:>5}（{:.3}%）、平均 {mean:.2}、0.5% 超 {:.4}%",
                        c.kind().name(),
                        max as f64 / 655.35,
                        over * 100.0
                    );
                }
            }
        }
    }
}
