//! 面の計算の速さ（前半は tools/csharp-golden/run.sh surface-bench の C# と同じ中身）。
//!   cargo run --release -p yolu-core --example surface_bench [回数]
//! 立方体を 76 × 76 に分けて膨らませた球（69,312 三角形。Unity 版で測った実のアバターの 69,535 三角形に近い数）で、組み立て（溶接・隣り合わせ・
//! BVH）、乱数のレイ 2 万本の 1 本あたり、2048² の面の上のダブ（半径 0.05・0.15、遮蔽のレイは並列）。どれも回数の中央値。
//! 後半は、3D のストロークの今の道（投影の塗り）と前の道（面を辿って遮蔽のレイ）を並べる: 同じカメラ・同じ半径で、ストロークの始めの準備、
//! 最初のダブ（区画を作る）、24 個のダブを並べたストローク（前の道は遮蔽の覚えあり）。重なった面の多い蛇腹の場面も。

use std::time::Instant;

use std::sync::Arc;

use yolu_core::geometry::{
    cube_sphere, model_triangles, pick, CameraView, OrbitCamera, ProjectionSettings, Ray,
    SurfaceBrushBudget, SurfaceGeometry, SurfaceProjector, SurfaceTriangle, SurfaceVisibilityCache,
    DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Vec2, Vec3};

struct SplitMix(u64);
impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn s(&mut self) -> f32 {
        (((self.next() >> 11) as f64 * (1.0 / 9007199254740992.0)) * 2.0 - 1.0) as f32
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let repeat: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let triangles = model_triangles(&[cube_sphere(76, 0.5)]).unwrap();
    let (mut build, mut adjacency, mut bvh) = (Vec::new(), Vec::new(), Vec::new());
    let mut g = None;
    for _ in 0..repeat {
        let t = triangles.clone();
        let clock = Instant::now();
        let geometry = SurfaceGeometry::new(t, 1, DEFAULT_WELD_TOLERANCE).unwrap();
        build.push(clock.elapsed().as_secs_f64() * 1000.0);
        adjacency.push(geometry.timings().adjacency_ms);
        bvh.push(geometry.timings().bvh_ms);
        g = Some(geometry);
    }
    let g = g.unwrap();
    println!(
        "Rust 球 {} 三角形: 組み立て {:.2} ms（隣り合わせ {:.2} ms・BVH {:.2} ms）、{} 回の中央値",
        g.triangle_count(),
        median(build),
        median(adjacency),
        median(bvh),
        repeat
    );
    let mut r = SplitMix(7);
    let center = g.bounds().center;
    let radius = g.bounds().extents.length();
    let rays: Vec<Ray> = (0..20000)
        .map(|_| {
            let (ox, oy, oz, tx, ty, tz) = (r.s(), r.s(), r.s(), r.s(), r.s(), r.s());
            let origin = center + Vec3::new(ox, oy, oz) * (radius * 3.0);
            let target = center + Vec3::new(tx, ty, tz) * (radius * 0.5);
            Ray::new(origin, target - origin)
        })
        .collect();
    let mut per = Vec::new();
    let mut hits = 0;
    for _ in 0..repeat {
        let clock = Instant::now();
        hits = rays
            .iter()
            .filter(|ray| g.raycast(**ray, true, f32::INFINITY).is_some())
            .count();
        per.push(clock.elapsed().as_secs_f64() * 1e6 / rays.len() as f64);
    }
    println!(
        "Rust レイ 1 本 {:.3} µs（{} 本・当たり {hits}。1 本のスレッド）",
        median(per),
        rays.len()
    );
    let cam = Vec3::new(0.3, 0.6, -2.0);
    let hit = g.raycast(Ray::new(cam, -cam), true, f32::INFINITY).unwrap();
    for radius in [0.05f32, 0.15] {
        let mut times = Vec::new();
        let mut pixels = 0;
        for _ in 0..repeat {
            let clock = Instant::now();
            let d = g.build_surface_dabs(
                &hit,
                radius,
                2048,
                2048,
                cam,
                0.8,
                &SurfaceBrushBudget::default(),
                None,
                false,
            );
            times.push(clock.elapsed().as_secs_f64() * 1000.0);
            pixels = d.pixels.len();
        }
        println!(
            "Rust ダブ 2048² 半径 {radius}: {:.2} ms（{pixels} 画素、{} スレッド）",
            median(times),
            rayon::current_num_threads()
        );
    }
    let view = OrbitCamera {
        target: Vec3::ZERO,
        yaw: 25.0,
        pitch: 10.0,
        distance: 2.0,
        model_radius: 0.5,
        ..Default::default()
    }
    .view(1280.0, 800.0);
    for radius in [0.05f32, 0.15, 0.3] {
        // 半径ごとに作り直したスナップショット（UV の覆いの覚えの無い所から）
        let fresh =
            Arc::new(SurfaceGeometry::new(triangles.clone(), 1, DEFAULT_WELD_TOLERANCE).unwrap());
        compare(&fresh, &view, "球", radius, repeat);
    }
    let crowded = crowded();
    let view = OrbitCamera {
        target: Vec3::new(0.0, 0.0, -0.8),
        yaw: 0.0,
        pitch: 0.0,
        distance: 1.6,
        model_radius: 0.6,
        ..Default::default()
    }
    .view(1280.0, 800.0);
    for radius in [0.05f32, 0.15] {
        let fresh =
            Arc::new(SurfaceGeometry::new(crowded.clone(), 1, DEFAULT_WELD_TOLERANCE).unwrap());
        compare(&fresh, &view, "蛇腹", radius, repeat);
    }
}

/// 今の道と前の道を、同じカメラ・半径・2048² で並べる（ストロークは画面の中心を横切る 24 個のダブ。間隔は直径の 15%）。
fn compare(g: &Arc<SurfaceGeometry>, view: &CameraView, name: &str, radius: f32, repeat: usize) {
    let center = Vec2::new(view.width * 0.5, view.height * 0.5);
    let Some(hit) = pick(g, view, center) else {
        println!("{name}: 中心が面に当たらない");
        return;
    };
    let screen = view.world_radius_to_screen(hit.position, radius);
    let step = 2.0 * screen * 0.15;
    let points: Vec<Vec2> = (0..24)
        .map(|i| center + Vec2::new((i as f32 - 12.0) * step, 0.0))
        .collect();
    let (mut setup, mut first, mut stroke) = (Vec::new(), Vec::new(), Vec::new());
    let mut pixels = 0;
    for _ in 0..repeat {
        let clock = Instant::now();
        let mut p = SurfaceProjector::new(
            g.clone(),
            view,
            None,
            2048,
            2048,
            screen,
            ProjectionSettings::default(),
        )
        .unwrap();
        setup.push(clock.elapsed().as_secs_f64() * 1000.0);
        let clock = Instant::now();
        let d = p.dab(center, screen, 0.8, u64::MAX);
        first.push(clock.elapsed().as_secs_f64() * 1000.0);
        pixels = d.pixels.len();
        let clock = Instant::now();
        for &at in &points {
            p.dab(at, screen, 0.8, u64::MAX);
        }
        stroke.push(clock.elapsed().as_secs_f64() * 1000.0);
    }
    let cold = setup[0];
    println!(
        "{name} 半径 {radius}（画面 {screen:.0}）今の道: 準備 {cold:.2} ms（UV の覆いを覚えた後 {:.2} ms）・最初のダブ {:.2} ms（{pixels} 画素）・24 個のストローク {:.2} ms",
        median(setup[1..].to_vec()),
        median(first),
        median(stroke)
    );
    let (mut first, mut stroke) = (Vec::new(), Vec::new());
    let mut outcome = String::new();
    for _ in 0..repeat {
        let mut cache = SurfaceVisibilityCache::new();
        let clock = Instant::now();
        let d = g.build_surface_dabs(
            &hit,
            radius,
            2048,
            2048,
            view.position,
            0.8,
            &SurfaceBrushBudget::default(),
            Some(&mut cache),
            false,
        );
        first.push(clock.elapsed().as_secs_f64() * 1000.0);
        outcome = match d.refusal {
            Some(r) => format!("断り {r:?}"),
            None => format!("{} 画素", d.pixels.len()),
        };
        let clock = Instant::now();
        let mut refused = 0;
        for &at in &points {
            if let Some(h) = pick(g, view, at) {
                let d = g.build_surface_dabs(
                    &h,
                    radius,
                    2048,
                    2048,
                    view.position,
                    0.8,
                    &SurfaceBrushBudget::default(),
                    Some(&mut cache),
                    false,
                );
                refused += d.refusal.is_some() as usize;
            }
        }
        stroke.push(clock.elapsed().as_secs_f64() * 1000.0);
        if refused > 0 {
            outcome += &format!("・ストロークで {refused} 個を断る（取り消し）");
        }
    }
    println!(
        "{name} 半径 {radius} 前の道: 最初のダブ {:.2} ms（{outcome}）・24 個のストローク {:.2} ms（{} スレッド）",
        median(first),
        median(stroke),
        rayon::current_num_threads()
    );
}

/// 細かく折り返す蛇腹（40 レイヤー・つながった 1 つのメッシュ、76,800 三角形）。
fn crowded() -> Vec<SurfaceTriangle> {
    let mut t = Vec::new();
    let folds = 40;
    let (cols, rows) = (24, 40);
    for f in 0..folds {
        let z0 = -0.6 - f as f32 * 0.01;
        let z1 = z0 - 0.01;
        for j in 0..rows {
            for i in 0..cols {
                let (u0, u1) = (i as f32 / cols as f32, (i + 1) as f32 / cols as f32);
                let (v0, v1) = (j as f32 / rows as f32, (j + 1) as f32 / rows as f32);
                let x = |u: f32| {
                    if f % 2 == 0 {
                        -0.15 + 0.3 * u
                    } else {
                        0.15 - 0.3 * u
                    }
                };
                let z = |u: f32| z0 + (z1 - z0) * u;
                let uv = |u: f32, v: f32| Vec2::new((f as f32 + u) / folds as f32, v);
                let p = |u: f32, v: f32| Vec3::new(x(u), -0.25 + 0.5 * v, z(u));
                let (a, b, c, d) = (p(u0, v0), p(u0, v1), p(u1, v0), p(u1, v1));
                let (ua, ub, uc, ud) = (uv(u0, v0), uv(u0, v1), uv(u1, v0), uv(u1, v1));
                if f % 2 == 0 {
                    t.push(SurfaceTriangle::new(a, b, c, ua, ub, uc));
                    t.push(SurfaceTriangle::new(c, b, d, uc, ub, ud));
                } else {
                    t.push(SurfaceTriangle::new(a, c, b, ua, uc, ub));
                    t.push(SurfaceTriangle::new(c, d, b, uc, ud, ub));
                }
            }
        }
    }
    t
}
