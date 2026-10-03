//! 面の計算の試験（C# との照合は tests/surface_golden.rs）。

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use glam::{Vec2, Vec3};

use super::*;
use crate::{BrushSettings, Document, Rgba8};

fn cube() -> SurfaceGeometry {
    SurfaceGeometry::new(
        model_triangles(&[demo_cube()]).unwrap(),
        1,
        DEFAULT_WELD_TOLERANCE,
    )
    .unwrap()
}

/// 立方体の手前（−Z）と右（+X）の面が見える位置のカメラ。
fn corner_camera() -> Vec3 {
    Vec3::new(1.6, 0.4, -2.0)
}

fn hit_towards(g: &SurfaceGeometry, from: Vec3, to: Vec3) -> SurfaceHit {
    g.raycast(Ray::new(from, to - from), true, f32::INFINITY)
        .expect("当たる")
}

fn island_of(x: i32, width: i32) -> i32 {
    // 立方体の UV は 3 × 2 の島（横 1/3 ずつ）
    (x * 3) / width
}

#[test]
fn cube_is_closed_and_every_triangle_has_three_neighbours() {
    let g = cube();
    assert_eq!(g.triangle_count(), 12);
    assert_eq!(g.non_manifold_edge_count(), 0);
    for i in 0..12 {
        assert_eq!(g.neighbors(i).len(), 3, "三角形 {i}");
        for &n in g.neighbors(i) {
            assert!(
                g.neighbors(n as usize).contains(&(i as u32)),
                "隣り合わせは対称"
            );
        }
    }
    assert_eq!(g.bounds().size(), Vec3::ONE);
}

#[test]
fn sphere_seams_weld_exactly() {
    let m = cube_sphere(8, 0.5);
    let g =
        SurfaceGeometry::new(model_triangles(&[m]).unwrap(), 1, DEFAULT_WELD_TOLERANCE).unwrap();
    assert_eq!(g.triangle_count(), 12 * 64);
    assert_eq!(g.non_manifold_edge_count(), 0);
    assert!(
        (0..g.triangle_count()).all(|i| g.neighbors(i).len() == 3),
        "継ぎ目も全部つながる"
    );
}

#[test]
fn ray_hits_the_front_face_with_uv_and_culls_from_inside() {
    let g = cube();
    let h = hit_towards(&g, Vec3::new(0.1, 0.2, -3.0), Vec3::new(0.1, 0.2, 0.0));
    assert!((h.distance - 2.5).abs() < 1e-5);
    assert!((h.position.z + 0.5).abs() < 1e-6);
    assert_eq!(h.normal, Vec3::new(0.0, 0.0, -1.0));
    assert!(h.triangle < 2, "手前の面は 0・1 番");
    // UV は面 0 の島（u 0.02〜0.31、v 0.03〜0.47）の中の、位置に比例した所
    let expect_u = 0.02 + (1.0 / 3.0 - 0.04) * 0.6;
    let expect_v = 0.03 + 0.44 * 0.7;
    assert!(
        (h.uv - Vec2::new(expect_u, expect_v)).length() < 1e-5,
        "{}",
        h.uv
    );
    // 中から外へは裏なので、裏を見ないなら当たらない（裏も見るなら当たる）
    let inside = Ray::new(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
    assert!(g.raycast(inside, true, f32::INFINITY).is_none());
    assert!(g.raycast(inside, false, f32::INFINITY).is_some());
    // 届く距離より遠い面は当たらない
    assert!(g
        .raycast(Ray::new(Vec3::new(0.0, 0.0, -3.0), Vec3::Z), true, 2.0)
        .is_none());
}

#[test]
fn dab_crosses_the_seam_and_leaves_hidden_faces() {
    let g = cube();
    let camera = corner_camera();
    // 手前の面の右の縁の近く
    let hit = hit_towards(&g, camera, Vec3::new(0.45, 0.1, -0.5));
    let dab = g.build_surface_dabs(
        &hit,
        0.2,
        256,
        256,
        camera,
        0.8,
        &SurfaceBrushBudget::default(),
        None,
        false,
    );
    assert_eq!(dab.refusal, None);
    let islands: std::collections::BTreeSet<(i32, i32)> = dab
        .pixels
        .iter()
        .map(|p| (island_of(p.x, 256), p.y * 2 / 256))
        .collect();
    assert_eq!(
        islands,
        [(0, 0), (0, 1)].into_iter().collect(),
        "面 0（島 0,0）と面 3（島 0,1）"
    );
    // 並びは下の行から、画素は重ならない
    for w in dab.pixels.windows(2) {
        assert!((w[0].y, w[0].x) < (w[1].y, w[1].x));
    }
    assert!(dab
        .pixels
        .iter()
        .all(|p| p.coverage > 0.0 && p.coverage <= 1.0));
    assert!(dab.pixels.iter().any(|p| p.coverage == 1.0));
}

#[test]
fn hidden_texels_are_not_painted_but_ignore_visibility_reaches_them() {
    let g = cube();
    // 手前の面の左の縁: 左の面（−X）は角のカメラから見えない
    let camera = corner_camera();
    let hit = hit_towards(&g, camera, Vec3::new(-0.45, 0.0, -0.5));
    let budget = SurfaceBrushBudget::default();
    let seen = g.build_surface_dabs(&hit, 0.2, 256, 256, camera, 0.8, &budget, None, false);
    let left_island = |p: &SurfacePixel| island_of(p.x, 256) == 2 && p.y < 128; // 面 2 は島 (2, 0)
    assert!(!seen.pixels.is_empty());
    assert!(
        !seen.pixels.iter().any(left_island),
        "見えない左の面は塗らない"
    );
    // カメラを立方体の裏に置くと、見える側の経路は手前の面を塗らない。カメラによらない足跡（当たりの法線と同じ側を向く面）は塗る
    let behind = Vec3::new(0.0, 0.0, 3.0);
    let none = g.build_surface_dabs(&hit, 0.2, 256, 256, behind, 0.8, &budget, None, false);
    assert!(none.pixels.is_empty() && none.refusal.is_none());
    let any = g.build_surface_dabs(&hit, 0.2, 256, 256, behind, 0.8, &budget, None, true);
    assert!(
        !any.pixels.is_empty()
            && any
                .pixels
                .iter()
                .all(|p| island_of(p.x, 256) == 0 && p.y < 128)
    );
    assert!(any.pixels.iter().all(|p| p.triangle.is_none()));
    // 直角の隣の面（左の面）へは、カメラによらない足跡でも進まない（法線の内積が 0。C# と同じ）
    assert!(!any.pixels.iter().any(left_island));
}

#[test]
fn budgets_refuse_the_whole_dab() {
    let g = cube();
    let camera = corner_camera();
    let hit = hit_towards(&g, camera, Vec3::new(0.45, 0.1, -0.5));
    let base = SurfaceBrushBudget::default();
    let cases = [
        (
            SurfaceBrushBudget {
                max_triangles: 1,
                ..base
            },
            DabRefusal::TriangleBudget,
        ),
        (
            SurfaceBrushBudget {
                max_candidate_pixels: 10,
                ..base
            },
            DabRefusal::PixelBudget,
        ),
        (
            SurfaceBrushBudget {
                max_visibility_rays: 10,
                ..base
            },
            DabRefusal::VisibilityBudget,
        ),
        (
            SurfaceBrushBudget {
                max_ray_node_visits: 1,
                ..base
            },
            DabRefusal::BvhBudget,
        ),
        (
            SurfaceBrushBudget {
                max_ray_triangle_tests: 1,
                ..base
            },
            DabRefusal::BvhBudget,
        ),
    ];
    for (budget, why) in cases {
        let dab = g.build_surface_dabs(&hit, 0.2, 256, 256, camera, 0.8, &budget, None, false);
        assert_eq!(dab.refusal, Some(why));
        assert!(dab.pixels.is_empty() && dab.was_clipped());
    }
    // 世代の違う当たり・範囲外の値は予算ではない（ストロークは取り消さない）
    let mut old = hit;
    old.revision = 2;
    let dab = g.build_surface_dabs(&old, 0.2, 256, 256, camera, 0.8, &base, None, false);
    assert_eq!(dab.refusal, Some(DabRefusal::SnapshotChanged));
    assert!(!dab.was_clipped());
    let dab = g.build_surface_dabs(&hit, -1.0, 256, 256, camera, 0.8, &base, None, false);
    assert_eq!(dab.refusal, Some(DabRefusal::InvalidArguments));
}

#[test]
fn parallel_rays_and_the_cache_give_the_same_pixels() {
    let m = cube_sphere(16, 0.5);
    let g =
        SurfaceGeometry::new(model_triangles(&[m]).unwrap(), 3, DEFAULT_WELD_TOLERANCE).unwrap();
    let camera = Vec3::new(0.3, 0.6, -2.0);
    let hit = hit_towards(&g, camera, Vec3::ZERO);
    let budget = SurfaceBrushBudget::default();
    let run = || g.build_surface_dabs(&hit, 0.3, 1024, 1024, camera, 0.5, &budget, None, false);
    let parallel = run();
    assert!(
        parallel.visibility_rays > 4096,
        "組を何回かまたぐ: {}",
        parallel.visibility_rays
    );
    let single = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
        .install(run);
    assert_eq!(parallel, single);
    let mut cache = SurfaceVisibilityCache::new();
    let first = g.build_surface_dabs(
        &hit,
        0.3,
        1024,
        1024,
        camera,
        0.5,
        &budget,
        Some(&mut cache),
        false,
    );
    assert_eq!(first.pixels, parallel.pixels);
    let second = g.build_surface_dabs(
        &hit,
        0.3,
        1024,
        1024,
        camera,
        0.5,
        &budget,
        Some(&mut cache),
        false,
    );
    assert_eq!(second.pixels, parallel.pixels);
    assert!(
        cache.hits > 0 && second.ray_triangle_tests == 0,
        "覚えたレイは撃たない"
    );
}

#[test]
fn closest_point_respects_facing_and_material() {
    let g = cube();
    let h = g
        .find_closest_point(Vec3::new(0.1, 0.0, -0.9), 1.0, Vec3::ZERO, 10_000, -1)
        .unwrap()
        .unwrap();
    assert!((h.distance - 0.4).abs() < 1e-6 && (h.position.z + 0.5).abs() < 1e-6);
    // 向きが反対の面だけを許すと、手前の面は選ばない
    let back = g
        .find_closest_point(
            Vec3::new(0.1, 0.0, -0.9),
            10.0,
            Vec3::new(0.0, 0.0, 1.0),
            10_000,
            -1,
        )
        .unwrap()
        .unwrap();
    assert!(back.normal.z > 0.5);
    assert!(g
        .find_closest_point(Vec3::ZERO, 10.0, Vec3::ZERO, 10_000, 5)
        .unwrap()
        .is_none());
    assert_eq!(
        g.find_closest_point(Vec3::ZERO, 10.0, Vec3::ZERO, 0, -1),
        Err(NodeBudgetExceeded)
    );
}

#[test]
fn regions_follow_islands_parts_and_materials() {
    let g = cube();
    assert_eq!(region(&g, 4, SurfaceRegionKind::Triangle), vec![4]);
    assert_eq!(region(&g, 4, SurfaceRegionKind::UvIsland), vec![4, 5]);
    assert_eq!(
        region(&g, 4, SurfaceRegionKind::MeshPart),
        (0..12).collect::<Vec<_>>()
    );
    assert_eq!(
        region(&g, 4, SurfaceRegionKind::Material),
        (0..12).collect::<Vec<_>>()
    );
}

#[test]
fn rejects_non_finite_and_cancels() {
    let mut t = model_triangles(&[demo_cube()]).unwrap();
    t[3].uv_b.x = f32::NAN;
    assert_eq!(
        SurfaceGeometry::new(t, 1, 1e-6).err(),
        Some(GeometryError::NonFinite)
    );
    let t = model_triangles(&[demo_cube()]).unwrap();
    assert_eq!(
        SurfaceGeometry::new(t.clone(), 1, 0.0).err(),
        Some(GeometryError::InvalidTolerance)
    );
    let cancel = AtomicBool::new(true);
    assert_eq!(
        SurfaceGeometry::build_cancelable(t, 1, 1e-6, &cancel).err(),
        Some(GeometryError::Canceled)
    );
    let empty = SurfaceGeometry::new(Vec::new(), 1, 1e-6).unwrap();
    assert!(empty
        .raycast(Ray::new(Vec3::ZERO, Vec3::Z), false, f32::INFINITY)
        .is_none());
}

#[test]
fn surface_stroke_paints_across_the_seam_and_undoes_in_one_step() {
    let mut doc = Document::new(256, 256).unwrap();
    let layer = doc.add_layer("1").unwrap();
    doc.clear_history().unwrap();
    let g = Arc::new(cube());
    let mut cam = OrbitCamera::framing(&g.bounds());
    cam.yaw = -40.0; // 右（+X）と手前（−Z）の面が見える
    cam.pitch = 15.0;
    let view = cam.view(400.0, 400.0);
    let brush = BrushSettings {
        radius: 12.0,
        color: Rgba8::new(220, 40, 30, 255),
        ..BrushSettings::default()
    };
    let mut stroke = doc.begin_stroke(layer, &brush).unwrap();
    // 手前の面から右の面へ、横に
    let from = view.to_screen(Vec3::new(0.2, 0.0, -0.5)).unwrap();
    let to = view.to_screen(Vec3::new(0.5, 0.0, -0.2)).unwrap();
    let mut s = SurfaceStroke::begin(
        &mut doc,
        &mut stroke,
        g.clone(),
        view,
        &brush,
        Some(0),
        from,
        1.0,
    )
    .unwrap();
    for i in 1..=8 {
        let p = from + (to - from) * (i as f32 / 8.0);
        s.add(&mut doc, &mut stroke, p, 1.0).unwrap();
    }
    s.finish(&mut doc, &mut stroke).unwrap();
    assert!(s.stats.dabs > 5 && s.stats.pixels > 100, "{:?}", s.stats);
    assert!(s.cache().hits > 0, "重なるダブは覚えた遮蔽を使う");
    assert!(doc.end_stroke(stroke).unwrap().changed);
    let painted = |doc: &Document| {
        let mut islands = std::collections::BTreeSet::new();
        for y in 0..256 {
            for x in 0..256 {
                if doc.composite_pixel(crate::Channel::Color, x, y).unwrap().a > 0 {
                    islands.insert((island_of(x as i32, 256), y as i32 * 2 / 256));
                }
            }
        }
        islands
    };
    assert_eq!(painted(&doc), [(0, 0), (0, 1)].into_iter().collect());
    doc.undo().unwrap();
    assert!(painted(&doc).is_empty(), "1 回の Undo で両方の面が戻る");
}

#[test]
fn surface_stroke_skips_other_texture_sets_and_refuses_on_budget() {
    let mut doc = Document::new(128, 128).unwrap();
    let layer = doc.add_layer("1").unwrap();
    let g = Arc::new(cube());
    let view = OrbitCamera::framing(&g.bounds()).view(300.0, 300.0);
    let brush = BrushSettings {
        radius: 8.0,
        ..BrushSettings::default()
    };
    let center = Vec2::new(150.0, 150.0);
    let mut stroke = doc.begin_stroke(layer, &brush).unwrap();
    let s = SurfaceStroke::begin(
        &mut doc,
        &mut stroke,
        g.clone(),
        view,
        &brush,
        Some(7),
        center,
        1.0,
    )
    .unwrap();
    assert_eq!(
        (s.stats.dabs, s.stats.missed),
        (0, 1),
        "ほかのテクスチャセットの面は塗らない"
    );
    doc.cancel_stroke(stroke);
    let mut stroke = doc.begin_stroke(layer, &brush).unwrap();
    let mut s = SurfaceStroke::begin(
        &mut doc,
        &mut stroke,
        g.clone(),
        view,
        &brush,
        None,
        center,
        1.0,
    )
    .unwrap();
    s.set_budget(SurfaceBrushBudget {
        max_candidate_pixels: 1,
        ..SurfaceBrushBudget::default()
    });
    let mut err = None;
    for i in 1..20 {
        if let Err(e) = s.add(
            &mut doc,
            &mut stroke,
            center + Vec2::new(i as f32 * 3.0, 0.0),
            1.0,
        ) {
            err = Some(e);
            break;
        }
    }
    assert_eq!(err, Some(SurfaceStrokeError::Dab(DabRefusal::PixelBudget)));
}
