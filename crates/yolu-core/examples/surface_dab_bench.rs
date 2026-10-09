//! 3D ビューの速いストロークの重さ（ダブ 1 つの時間と内訳。同じ大きさの 2D のダブとの比べつき）。
//!   cargo run --release -p yolu-core --features stroke-profile --example surface_dab_bench -- [--threads N] [--models 18,130]
//!       [--docs 2048,4096] [--sizes 64,128,256] [--dabs 240] [--event 0] [--view 1600x900] [--skip-2d] [--frame 4] [--budget 8] [--runs 3]
//! 合成のモデルは立方体を n × n に分けて膨らませた球（`cube_sphere`。n=18 は 3,888 三角形、n=130 は 202,800 三角形）。カメラは app の
//! 既定と同じ全体表示（`OrbitCamera::framing`）。ブラシは既定の間隔（0.15）・硬さ 0.8 の丸、大きさは直径（文書の画素）。
//! 速いストロークは、球の面の上を横切る弦（1 本が 500〜700 画面の点）を行き来する折れ線の頂点を、1 回の入力ごとに足していく
//! （`--event` が正なら、弦をその長さごとの入力に刻む）。入力（`add_input`）は並べるだけで、フレームの終わりに時間の枠（`--budget` ms、
//! 最初の 1 つは必ず塗る）まで `paint_queued_until` で塗り、離した後（`end_input`）も同じ枠で残りを塗り切る。
//! `--frame k` は 1 フレームに入力が k 回来る見立て。`--runs` は繰り返し（1 回目は捨て、残りの 1 回ごとの時間を合わせて中央値を出す）。
//! `--features stroke-profile` のとき、ダブ 1 つ（`paint_plan` の全体）と内訳（投影の塗り・区画の作り直し・覆いの集め・文書へ塗る）の
//! 1 回ごとの時間から中央値・最大を出す。内訳は入れ子（投影の塗り ⊃ 区画の作り直し + 覆いの集め）。
//! 2D の比べは、同じ文書・同じブラシで、ダブの間隔ちょうどに点を足して 1 回の足しを 1 ダブとして測る。

use std::sync::Arc;
use std::time::Instant;

use yolu_core::geometry::{
    cube_sphere, model_triangles, CameraView, OrbitCamera, SurfaceGeometry, SurfaceInput,
    SurfaceStroke, SurfaceStrokeOptions, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{DVec2, Vec2};
use yolu_core::{Brush, BrushSettings, Document, Rgba8};

fn list(args: &[String], key: &str) -> Option<Vec<String>> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .map(|v| v.split(',').map(|s| s.to_string()).collect())
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[(((sorted.len() - 1) as f64) * p).round() as usize]
}

fn sorted(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(f64::total_cmp);
    v
}

fn settings(size: f64) -> BrushSettings {
    BrushSettings {
        radius: size / 2.0,
        color: Rgba8::new(200, 60, 30, 255),
        pressure_size: false,
        pressure_opacity: false,
        ..BrushSettings::default()
    }
}

fn document(side: u32) -> (Document, yolu_core::LayerId) {
    let mut doc = Document::new(side, side).unwrap();
    doc.set_stroke_budget_bytes(1 << 30).unwrap();
    let layer = doc.add_layer("a").unwrap();
    (doc, layer)
}

/// 球の上を横切る弦の頂点（画面の点）。1 本ずつ向きを替え、高さを散らす。弦は球の縁の手前（0.85）で止める。
fn chords(view: &CameraView, sphere_radius_px: f32, count: usize, event: f32) -> Vec<Vec2> {
    let c = Vec2::new(view.width * 0.5, view.height * 0.5);
    let mut points = vec![];
    let mut last: Option<Vec2> = None;
    for k in 0..count {
        let dy = sphere_radius_px * 0.8 * ((k as f32) * 1.7).sin();
        let half = (sphere_radius_px * sphere_radius_px - dy * dy).sqrt() * 0.85;
        let x = if k % 2 == 0 { -half } else { half };
        let to = c + Vec2::new(x, dy);
        let from = last.unwrap_or(c + Vec2::new(-x, dy));
        if last.is_none() {
            points.push(from);
        }
        if event > 0.0 {
            let n = ((to - from).length() / event).ceil().max(1.0) as usize;
            for i in 1..=n {
                points.push(from + (to - from) * (i as f32 / n as f32));
            }
        } else {
            points.push(to);
        }
        last = Some(to);
    }
    points
}

struct Case<'a> {
    geometry: &'a Arc<SurfaceGeometry>,
    view: CameraView,
    side: u32,
    size: f64,
    target_dabs: usize,
    event: f32,
    frame: usize,
    budget_ms: f64,
    sphere_radius_px: f32,
}

#[derive(Default)]
struct Outcome {
    begin_ms: f64,
    calls: Vec<f64>,
    painted_in_calls: Vec<usize>,
    drain_ms: f64,
    drain_frames: usize,
    dabs: usize,
    missed: usize,
    pixels: usize,
    frames: Vec<f64>,
    built: u64,
    reuses: u64,
    evictions: u64,
    skipped: u64,
    cached_mib: f64,
}

fn run_3d(case: &Case) -> Outcome {
    let (mut doc, layer) = document(case.side);
    let brush = Brush::from(settings(case.size));
    let points = chords(
        &case.view,
        case.sphere_radius_px,
        400,
        if case.event > 0.0 { case.event } else { 0.0 },
    );
    let mut out = Outcome::default();
    let mut stroke = doc.begin_brush_stroke(layer, &brush).unwrap();
    let t0 = Instant::now();
    let mut s = SurfaceStroke::begin_input(
        &mut doc,
        &mut stroke,
        case.geometry.clone(),
        case.view,
        &brush.base,
        Some(0),
        SurfaceInput::new(points[0], 1.0),
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    out.begin_ms = t0.elapsed().as_secs_f64() * 1000.0;
    // フレームの見立て: 1 フレームに入力が frame 回来て（並べるだけ）、フレームの終わりに時間の枠まで塗る
    let mut next = 1;
    let mut painted_total = 0;
    let budget = std::time::Duration::from_secs_f64(case.budget_ms / 1000.0);
    let mut frame = |s: &mut SurfaceStroke,
                     doc: &mut Document,
                     stroke: &mut yolu_core::Stroke,
                     adds: &[Vec2]| {
        let t = Instant::now();
        for &p in adds {
            let ta = Instant::now();
            s.add_input(doc, stroke, SurfaceInput::new(p, 1.0)).unwrap();
            out.calls.push(ta.elapsed().as_secs_f64() * 1000.0);
        }
        let n = s
            .paint_queued_until(doc, stroke, Instant::now() + budget)
            .unwrap();
        painted_total += n;
        out.painted_in_calls.push(n);
        out.frames.push(t.elapsed().as_secs_f64() * 1000.0);
    };
    while next < points.len() && s.stats.dabs + s.stats.missed + s.queued() < case.target_dabs {
        let end = (next + case.frame).min(points.len());
        frame(&mut s, &mut doc, &mut stroke, &points[next..end]);
        next = end;
    }
    let t = Instant::now();
    s.end_input().unwrap();
    let mut drain_frames = 0;
    while s.queued() > 0 {
        frame(&mut s, &mut doc, &mut stroke, &[]);
        drain_frames += 1;
    }
    out.drain_ms = t.elapsed().as_secs_f64() * 1000.0;
    out.drain_frames = drain_frames;
    if std::env::var_os("BENCH_WORST").is_some() {
        let (i, ms) = out
            .frames
            .iter()
            .enumerate()
            .fold((0, 0.0f64), |a, (i, &m)| if m > a.1 { (i, m) } else { a });
        eprintln!(
            "  最も長いフレーム: {i} 番目 {ms:.1} ms（ダブ {} 個）",
            out.painted_in_calls.get(i).copied().unwrap_or(0)
        );
    }
    out.dabs = s.stats.dabs;
    out.missed = s.stats.missed;
    out.pixels = s.stats.pixels;
    let p = s.projection_stats();
    out.built = p.buckets_built;
    out.reuses = p.bucket_reuses;
    out.evictions = p.evictions;
    out.skipped = p.skipped_dabs;
    out.cached_mib = p.peak_bytes as f64 / (1024.0 * 1024.0);
    let result = doc.end_stroke(stroke).unwrap();
    std::hint::black_box(&result);
    out
}

/// 同じ文書・同じブラシの 2D のダブ（間隔ちょうどに点を足し、足し 1 回を 1 ダブとする）の時間（ミリ秒）。
fn run_2d(side: u32, size: f64, dabs: usize) -> Vec<f64> {
    let (mut doc, layer) = document(side);
    let brush = Brush::from(settings(size));
    let gap = size * brush.base.spacing;
    let mut stroke = doc.begin_brush_stroke(layer, &brush).unwrap();
    let margin = size;
    let length = side as f64 - 2.0 * margin;
    let mut times = vec![];
    let mut x = margin;
    let mut y = margin;
    let mut dir = 1.0;
    stroke.add_point(&mut doc, x, y, 1.0, DVec2::ZERO).unwrap();
    while times.len() < dabs {
        x += dir * gap;
        if x > margin + length || x < margin {
            dir = -dir;
            y += size * 0.5;
            if y > side as f64 - margin {
                y = margin;
            }
            x = x.clamp(margin, margin + length);
        }
        let t = Instant::now();
        stroke.add_point(&mut doc, x, y, 1.0, DVec2::ZERO).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    doc.end_stroke(stroke).unwrap();
    times
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(n) = list(&args, "--threads") {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n[0].parse().unwrap())
            .build_global()
            .unwrap();
    }
    let models: Vec<u32> = list(&args, "--models").map_or(vec![18, 130], |v| {
        v.iter().map(|s| s.parse().unwrap()).collect()
    });
    let docs: Vec<u32> = list(&args, "--docs").map_or(vec![2048, 4096], |v| {
        v.iter().map(|s| s.parse().unwrap()).collect()
    });
    let sizes: Vec<f64> = list(&args, "--sizes").map_or(vec![64.0, 128.0, 256.0], |v| {
        v.iter().map(|s| s.parse().unwrap()).collect()
    });
    let target_dabs: usize = list(&args, "--dabs").map_or(240, |v| v[0].parse().unwrap());
    let event: f32 = list(&args, "--event").map_or(0.0, |v| v[0].parse().unwrap());
    let frame: usize = list(&args, "--frame").map_or(1, |v| v[0].parse().unwrap());
    let (vw, vh): (f32, f32) = list(&args, "--view").map_or((1600.0, 900.0), |v| {
        let (w, h) = v[0].split_once('x').unwrap();
        (w.parse().unwrap(), h.parse().unwrap())
    });
    let budget_ms: f64 = list(&args, "--budget").map_or(8.0, |v| v[0].parse().unwrap());
    let runs: usize = list(&args, "--runs").map_or(3, |v| v[0].parse().unwrap());
    let skip_2d = args.iter().any(|a| a == "--skip-2d");
    println!(
        "スレッド {}、表示域 {vw}x{vh}、目標のダブ {target_dabs}、入力の刻み {} 、1 フレームの入力 {frame}",
        rayon::current_num_threads(),
        if event > 0.0 {
            format!("{event} 点")
        } else {
            "弦ごと".to_string()
        }
    );
    for &n in &models {
        let triangles = model_triangles(&[cube_sphere(n, 0.5)]).unwrap();
        let count = triangles.len();
        let geometry =
            Arc::new(SurfaceGeometry::new(triangles, 1, DEFAULT_WELD_TOLERANCE).unwrap());
        let camera = OrbitCamera::framing(&geometry.bounds());
        let view = camera.view(vw, vh);
        let sphere_radius_px = view.world_radius_to_screen(geometry.bounds().center, 0.5);
        for &side in &docs {
            for &size in &sizes {
                // 2D の比べ
                let (two_median, two_max) = if skip_2d {
                    (0.0, 0.0)
                } else {
                    let t = sorted(run_2d(side, size, target_dabs));
                    (percentile(&t, 0.5), *t.last().unwrap())
                };
                let case = Case {
                    geometry: &geometry,
                    view,
                    side,
                    size,
                    target_dabs,
                    event,
                    frame,
                    budget_ms,
                    sphere_radius_px,
                };
                // 1 回目は捨てる（UV の覆いの覚え・ページの確保）
                let _ = run_3d(&case);
                #[cfg(feature = "stroke-profile")]
                yolu_core::brush::profile::reset();
                let mut o = run_3d(&case);
                for _ in 1..runs {
                    o = run_3d(&case);
                }
                let world = yolu_core::geometry::world_radius(&geometry, size / 2.0, side);
                let screen = view.world_radius_to_screen(geometry.bounds().center, world);
                #[cfg(feature = "stroke-profile")]
                let (dab, parts) = {
                    use yolu_core::brush::profile::{samples, Stage};
                    let stat = |stage: Stage| {
                        let v = samples(stage);
                        let mean = v.iter().sum::<f64>() / v.len().max(1) as f64 / 1e6;
                        (
                            percentile(&v, 0.5) / 1e6,
                            mean,
                            v.last().copied().unwrap_or(0.0) / 1e6,
                        )
                    };
                    let project = stat(Stage::SurfaceProject);
                    let buckets = stat(Stage::SurfaceBuckets);
                    let gather = stat(Stage::SurfaceGather);
                    let apply = stat(Stage::SurfaceApply);
                    (
                        stat(Stage::SurfaceDab),
                        [
                            (project.0, project.1),
                            (buckets.0, buckets.1),
                            (gather.0, gather.1),
                            (apply.0, apply.1),
                        ],
                    )
                };
                #[cfg(not(feature = "stroke-profile"))]
                let (dab, parts) = ((0.0, 0.0, 0.0), [(0.0, 0.0); 4]);
                let calls = sorted(o.calls.clone());
                let frames = sorted(o.frames.clone());
                let handled = o.dabs + o.missed;
                let total_buckets = (o.built + o.reuses).max(1) as f64;
                println!(
                    "球{n}({count}三角形) 文書{side}² 大きさ{size} 画面半径{screen:.0} 間隔{:.1}点 | 始め {:.1} ms | 3D 1ダブ 中央 {:.2} 平均 {:.2} 最大 {:.2} ms | 2D 1ダブ 中央 {:.2} 最大 {:.2} ms | \
                     入力1回(並べるだけ) 最大 {:.2} ms | フレーム({frame}入力+枠{budget_ms}ms) 中央 {:.1} 最大 {:.1} ms（フレームで塗ったダブ 中央 {} 最大 {}）| 離した後の残り {} フレーム {:.0} ms | ダブ {handled}（飛ばし {}）画素/ダブ {:.0} | 区画 作り{} 再利用{} 当たり率{:.2} 捨て{} ピーク{:.1}MiB | \
                     内訳(中央/平均 ms) 投影 {:.2}/{:.2} 区画 {:.2}/{:.2} 集め {:.2}/{:.2} 塗り {:.2}/{:.2}",
                    screen as f64 * 2.0 * 0.15,
                    o.begin_ms,
                    dab.0,
                    dab.1,
                    dab.2,
                    two_median,
                    two_max,
                    calls.last().copied().unwrap_or(0.0),
                    percentile(&frames, 0.5),
                    frames.last().copied().unwrap_or(0.0),
                    {
                        let mut v: Vec<usize> = o.painted_in_calls.clone();
                        v.sort();
                        v.get(v.len() / 2).copied().unwrap_or(0)
                    },
                    o.painted_in_calls.iter().copied().max().unwrap_or(0),
                    o.drain_frames,
                    o.drain_ms,
                    o.missed,
                    o.pixels as f64 / o.dabs.max(1) as f64,
                    o.built,
                    o.reuses,
                    o.reuses as f64 / total_buckets,
                    o.evictions,
                    o.cached_mib,
                    parts[0].0,
                    parts[0].1,
                    parts[1].0,
                    parts[1].1,
                    parts[2].0,
                    parts[2].1,
                    parts[3].0,
                    parts[3].1,
                );
            }
        }
    }
}
