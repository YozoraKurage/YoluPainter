//! 面の計算の速さ（tools/csharp-golden/run.sh surface-bench の C# と同じ中身）。
//!   cargo run --release -p yolu-core --example surface_bench [回数]
//! 立方体を 76 × 76 に分けて膨らませた球（69,312 三角形。Unity 版で測った実のアバターの 69,535 三角形に近い数）で、組み立て（溶接・隣り合わせ・
//! BVH）、乱数のレイ 2 万本の 1 本あたり、2048² の面の上のダブ（半径 0.05・0.15、遮蔽のレイは並列）。どれも回数の中央値。

use std::time::Instant;

use yolu_core::geometry::{
    cube_sphere, model_triangles, Ray, SurfaceBrushBudget, SurfaceGeometry, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::Vec3;

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
}
