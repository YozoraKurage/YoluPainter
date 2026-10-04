//! ポーズを 1 回変えてから描けるまでの時間（試しの人形 7 万三角形。実のアバターのデータは使わない）。
//!   cargo run --release -p yolu-core --example pose_bench [回数]
//! 1. 休みの形の組み立て（溶接・隣り合わせ・BVH。読み込んだときに 1 回）
//! 2. ポーズを変えたとき: スキニング（BlendShape を含む）→ 三角形の写し → BVH の refit か作り直し（隣り合わせは休みの形のまま）
//!    → 最初のダブ（2048²・半径 0.05）。比べに、全部を組み直す（Live Link のポーズの今の経路と同じ）時間も。
//! 3. 当たりの速さ: 乱数のレイ 2 万本の 1 本あたり（休みの形・refit・作り直し）と、BVH の箱の表面積の和（refit の木の合わなさ）。
//!
//! どれも回数の中央値。

use std::time::Instant;

use yolu_core::geometry::{
    model_triangles, BvhUpdate, Ray, SurfaceBrushBudget, SurfaceGeometry, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Quat, Vec3};
use yolu_core::skin::{demo_figure, FigureDetail, Pose, Rig};

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn ms(clock: Instant) -> f64 {
    clock.elapsed().as_secs_f64() * 1000.0
}

struct XorShift(u64);
impl XorShift {
    fn s(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    }
}

fn poses(rig: &Rig) -> Vec<(&'static str, Pose)> {
    let find = |name: &str| rig.bones().iter().position(|b| b.name == name).unwrap();
    let mut small = rig.rest_pose();
    small.locals[find("右前腕")].rotation = Quat::from_rotation_y(0.4);
    let mut large = rig.rest_pose();
    // 両腕を下ろし、肘と膝を曲げ、背を丸め、首を回す（T ポーズから大きく離れる）
    large.locals[find("右上腕")].rotation = Quat::from_rotation_z(-1.3);
    large.locals[find("左上腕")].rotation = Quat::from_rotation_z(1.3);
    large.locals[find("右前腕")].rotation = Quat::from_rotation_y(1.4);
    large.locals[find("左前腕")].rotation = Quat::from_rotation_y(-1.4);
    large.locals[find("右太もも")].rotation = Quat::from_rotation_x(-1.2);
    large.locals[find("右すね")].rotation = Quat::from_rotation_x(1.5);
    large.locals[find("左太もも")].rotation = Quat::from_rotation_x(0.5);
    large.locals[find("背骨")].rotation = Quat::from_rotation_x(0.4);
    large.locals[find("首")].rotation = Quat::from_rotation_y(0.8);
    large.blend_weights[0][0] = 100.0;
    let last = large.blend_weights.len() - 1;
    large.blend_weights[last][0] = 75.0;
    vec![("肘を少し", small), ("大きく", large)]
}

fn rays(g: &SurfaceGeometry, n: usize) -> Vec<Ray> {
    let mut r = XorShift(0x9E37_79B9_7F4A_7C15);
    let (c, e) = (g.bounds().center, g.bounds().extents.length());
    (0..n)
        .map(|_| {
            let o = c + Vec3::new(r.s(), r.s(), r.s()) * e * 3.0;
            let t = c + Vec3::new(r.s(), r.s(), r.s()) * e * 0.5;
            Ray::new(o, t - o)
        })
        .collect()
}

fn per_ray_us(g: &SurfaceGeometry, rays: &[Ray], repeat: usize) -> (f64, usize) {
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
    (median(per), hits)
}

/// カメラから人形の胸へ撃った当たりに、2048² の半径 0.05 のダブを置く時間。
fn first_dab(g: &SurfaceGeometry) -> (f64, usize) {
    let cam = Vec3::new(0.3, 1.4, 2.0);
    let target = Vec3::new(0.0, 1.25, 0.0);
    let hit = g
        .raycast(Ray::new(cam, target - cam), true, f32::INFINITY)
        .expect("胸に当たる");
    let clock = Instant::now();
    let d = g.build_surface_dabs(
        &hit,
        0.05,
        2048,
        2048,
        cam,
        0.8,
        &SurfaceBrushBudget::default(),
        None,
        false,
    );
    (ms(clock), d.pixels.len())
}

fn main() {
    let repeat: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(7);
    let rig = demo_figure(FigureDetail::AVATAR);
    println!(
        "試しの人形: 三角形 {}・頂点 {}・骨 {}・BlendShape {}・{} スレッド",
        rig.triangle_count(),
        rig.vertex_count(),
        rig.bones().len(),
        rig.blend_shape_count(),
        rayon::current_num_threads()
    );
    let rest_meshes = rig.deform(&rig.rest_pose()).unwrap();
    let rest_tri = model_triangles(&rest_meshes).unwrap();
    let mut times = Vec::new();
    let mut rest = None;
    for _ in 0..repeat {
        let clock = Instant::now();
        let g = SurfaceGeometry::new(rest_tri.clone(), 1, DEFAULT_WELD_TOLERANCE).unwrap();
        times.push(ms(clock));
        rest = Some(g);
    }
    let rest = rest.unwrap();
    println!(
        "休みの形の組み立て {:.2} ms（隣り合わせ {:.2}・BVH {:.2}）",
        median(times),
        rest.timings().adjacency_ms,
        rest.timings().bvh_ms
    );
    let probe = rays(&rest, 20_000);
    let (us, hits) = per_ray_us(&rest, &probe, repeat);
    println!(
        "  レイ 1 本 {us:.3} µs（当たり {hits}）・箱の表面積の和 {:.1}",
        rest.bvh_surface_area()
    );
    for (label, pose) in poses(&rig) {
        let (mut skin, mut copy, mut refit, mut rebuild, mut full) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (mut g_refit, mut g_rebuild) = (None, None);
        for _ in 0..repeat {
            let clock = Instant::now();
            let meshes = rig.deform(&pose).unwrap();
            skin.push(ms(clock));
            let clock = Instant::now();
            let tri = model_triangles(&meshes).unwrap();
            copy.push(ms(clock));
            let clock = Instant::now();
            g_refit = Some(rest.reposition(tri.clone(), 2, BvhUpdate::Refit).unwrap());
            refit.push(ms(clock));
            let clock = Instant::now();
            g_rebuild = Some(rest.reposition(tri.clone(), 3, BvhUpdate::Rebuild).unwrap());
            rebuild.push(ms(clock));
            let clock = Instant::now();
            let _ = SurfaceGeometry::new(tri, 4, DEFAULT_WELD_TOLERANCE).unwrap();
            full.push(ms(clock));
        }
        let (g_refit, g_rebuild) = (g_refit.unwrap(), g_rebuild.unwrap());
        let (skin, copy, refit, rebuild, full) = (
            median(skin),
            median(copy),
            median(refit),
            median(rebuild),
            median(full),
        );
        println!(
            "ポーズ「{label}」:（refit の内訳: 三角形の箱 {:.2} ms・節点 {:.2} ms）",
            g_refit.timings().snapshot_ms,
            g_refit.timings().bvh_ms
        );
        println!(
            "  スキニング {skin:.2} ms・三角形の写し {copy:.2} ms・refit {refit:.2} ms・BVH の作り直し {rebuild:.2} ms・全部を組み直す {full:.2} ms"
        );
        let mut dab_refit = Vec::new();
        let mut dab_rebuild = Vec::new();
        let mut pixels = 0;
        for _ in 0..repeat {
            let (t, p) = first_dab(&g_refit);
            dab_refit.push(t);
            pixels = p;
            dab_rebuild.push(first_dab(&g_rebuild).0);
        }
        let (dab_refit, dab_rebuild) = (median(dab_refit), median(dab_rebuild));
        println!(
            "  ポーズを変えてから描けるまで: refit {:.2} ms（+ 最初のダブ {dab_refit:.2} ms = {:.2} ms）・作り直し {:.2} ms（+ ダブ {dab_rebuild:.2} ms）・全部 {:.2} ms（ダブ {pixels} 画素）",
            skin + copy + refit,
            skin + copy + refit + dab_refit,
            skin + copy + rebuild,
            skin + copy + full
        );
        let probe = rays(&g_refit, 20_000);
        let (a, hits) = per_ray_us(&g_refit, &probe, repeat);
        let (b, _) = per_ray_us(&g_rebuild, &probe, repeat);
        println!(
            "  レイ 1 本: refit {a:.3} µs・作り直し {b:.3} µs（当たり {hits}）・箱の表面積の和 refit {:.1} / 作り直し {:.1}",
            g_refit.bvh_surface_area(),
            g_rebuild.bvh_surface_area()
        );
    }
}
