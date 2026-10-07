//! レイヤーのフィルターが UV の継ぎ目をまたぐときの速さと大きさ。人工のモデル（立方体の 6 面を n × n に分けて球へ膨らませ、面を k × k のアイランドに
//! 切って 4096² の UV に並べる。アイランドの間は `SEAM_GAP` 画素、既定 8）と、全面を描いた 4096² のレイヤーのガウスぼかし（半径 8・64）を、またがない・またぐで測る。
//! アイランドの図・帯の写しを作る時間と大きさも出す。
//!
//! `cargo run --release -p yolu-core --example seam_bench`（`SEAM_N`・`SEAM_K`・`SEAM_GAP`・`SEAM_SIZE`・`EFFECT_THREADS` で変えられる）
use std::sync::Arc;
use std::time::Instant;

use yolu_core::geometry::{SurfaceGeometry, SurfaceTriangle, UvTopology, DEFAULT_WELD_TOLERANCE};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::{
    Channel, Document, EffectInputs, EffectSettings, FilterSpec, FilterTarget, Rect, TileCoord,
};

fn env(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let threads = env("EFFECT_THREADS", 8) as usize;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    println!("スレッド {threads}");
    pool.install(run);
}

fn time<T>(label: &str, f: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let v = f();
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("{label}: {ms:.1} ms");
    (v, ms)
}

/// 立方体の面 f の (s, t)（−1..1）の点を球へ。
fn sphere(f: usize, s: f32, t: f32) -> Vec3 {
    let (n, u, v) = [
        (Vec3::Z * -1.0, Vec3::X, Vec3::Y),
        (Vec3::Z, Vec3::X * -1.0, Vec3::Y),
        (Vec3::X * -1.0, Vec3::Z * -1.0, Vec3::Y),
        (Vec3::X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::Z * -1.0),
        (Vec3::Y * -1.0, Vec3::X, Vec3::Z),
    ][f];
    (n + u * s + v * t).normalize()
}

fn model(n: u32, k: u32, size: u32, gap: u32) -> Vec<SurfaceTriangle> {
    let charts = 6 * k * k;
    let grid = (charts as f32).sqrt().ceil() as u32;
    let cell = 1.0 / grid as f32;
    let margin = gap as f32 / 2.0 / size as f32;
    let per = n / k;
    let mut out = Vec::new();
    for f in 0..6 {
        for j in 0..n {
            for i in 0..n {
                let (pi, pj) = (i / per, j / per);
                let chart = (f as u32) * k * k + pj * k + pi;
                let (cx, cy) = ((chart % grid) as f32 * cell, (chart / grid) as f32 * cell);
                let uv = |a: u32, b: u32| {
                    let lx = (a - pi * per) as f32 / per as f32;
                    let ly = (b - pj * per) as f32 / per as f32;
                    Vec2::new(
                        cx + margin + lx * (cell - 2.0 * margin),
                        cy + margin + ly * (cell - 2.0 * margin),
                    )
                };
                let p = |a: u32, b: u32| {
                    sphere(
                        f,
                        a as f32 / n as f32 * 2.0 - 1.0,
                        b as f32 / n as f32 * 2.0 - 1.0,
                    )
                };
                let (a, b, c, d) = (p(i, j), p(i + 1, j), p(i + 1, j + 1), p(i, j + 1));
                let (ua, ub, uc, ud) = (uv(i, j), uv(i + 1, j), uv(i + 1, j + 1), uv(i, j + 1));
                out.push(SurfaceTriangle::new(a, b, c, ua, ub, uc));
                out.push(SurfaceTriangle::new(a, c, d, ua, uc, ud));
            }
        }
    }
    out
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn run() {
    let size = env("SEAM_SIZE", 4096);
    let n = env("SEAM_N", 64);
    let k = env("SEAM_K", 4);
    let gap = env("SEAM_GAP", 8);
    let triangles = model(n, k, size, gap);
    println!(
        "モデル: 三角形 {}、アイランド {}（面 {} × {}）、アイランドの間 {gap} 画素",
        triangles.len(),
        6 * k * k,
        k,
        k
    );
    let geometry = Arc::new(SurfaceGeometry::new(triangles, 1, DEFAULT_WELD_TOLERANCE).unwrap());
    let topology = Arc::new(UvTopology::new(geometry.clone(), None));
    let (_, _) = time("アイランドと縁の対応", || {
        topology.seam_edge_count()
    });
    println!("継ぎ目の縁 {}", topology.seam_edge_count());
    let (map, _) = time("アイランドの図 4096²", || {
        topology.island_map(size, size).unwrap()
    });
    println!(
        "アイランドの図: {:.2} MiB、アイランドの中のテクセル {:.1} %",
        mib(map.bytes()),
        map.covered_texels() as f64 * 100.0 / (size as f64 * size as f64)
    );
    for band in [8, 64] {
        let (table, _) = time(&format!("帯の写し 幅 {band}"), || {
            topology.seam_band(size, size, band).unwrap()
        });
        let s = table.stats();
        println!(
            "  帯のテクセル {}（{:.1} %）、{:.1} MiB、展開した三角形 {}、埋めなかった {}、上限の縁 {}、作る時間（中）{:.1} ms（展開 {:.1}・埋める {:.1}）",
            s.texels,
            s.texels as f64 * 100.0 / (size as f64 * size as f64),
            mib(table.bytes()),
            s.chart_triangles,
            s.conflicts,
            s.capped_edges,
            s.build_ms,
            s.chart_ms,
            s.fill_ms
        );
    }

    // 全面を描いたレイヤー（タイルごとに縞）
    let mut doc = Document::new(size, size).unwrap();
    doc.set_source_budget_bytes(1 << 30).unwrap();
    let layer = doc.add_layer("レイヤー").unwrap();
    let tile = doc.tile_size();
    let mut bytes = vec![0u8; (tile * tile * 4) as usize];
    for ty in 0..size / tile {
        for tx in 0..size / tile {
            for (i, p) in bytes.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (x, y) = (i as u32 % tile, i as u32 / tile);
                p.copy_from_slice(&[
                    (x * 3 + tx * 11) as u8,
                    (y * 5 + ty * 7) as u8,
                    (x + y) as u8,
                    255,
                ]);
            }
            doc.import_tile(layer, Channel::Color, TileCoord::new(tx, ty), &bytes)
                .unwrap();
        }
    }
    let whole = Rect::new(0, 0, size, size);
    let mut filter = None;
    for radius in [8u32, 64] {
        let spec = FilterSpec::new(EffectSettings::blur(radius)).channels(&[Channel::Color]);
        match filter {
            None => filter = Some(doc.add_filter(layer, FilterTarget::Content, spec).unwrap()),
            Some(id) => doc
                .set_filter_settings(layer, id, EffectSettings::blur(radius), false)
                .unwrap(),
        }
        // 3 回ずつ交互に測って、それぞれの最小
        let (mut flat, mut across) = (f64::INFINITY, f64::INFINITY);
        for _ in 0..3 {
            doc.set_effect_inputs(EffectInputs::new()).unwrap();
            doc.release_effect_cache();
            let start = Instant::now();
            doc.composite_channel(Channel::Color, whole).unwrap();
            flat = flat.min(start.elapsed().as_secs_f64() * 1000.0);
            doc.set_effect_inputs(EffectInputs::new().with_topology(Some(topology.clone())))
                .unwrap();
            doc.release_effect_cache();
            let start = Instant::now();
            doc.composite_channel(Channel::Color, whole).unwrap();
            across = across.min(start.elapsed().as_secs_f64() * 1000.0);
        }
        println!(
            "半径 {radius}: またがない {flat:.1} ms、またぐ {across:.1} ms（帯の写しは作ってある）、比 {:.2}",
            across / flat
        );
    }
    println!(
        "覚えているアイランドの図と帯の写し: {:.1} MiB",
        mib(topology.cached_bytes())
    );
}
