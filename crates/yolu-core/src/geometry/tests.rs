//! 面の計算の試験（C# との照合は tests/reference/surface_golden.rs）。

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
    // 立方体の UV は 3 × 2 のアイランド（横 1/3 ずつ）
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
    // UV は面 0 のアイランド（u 0.02〜0.31、v 0.03〜0.47）の中の、位置に比例した所
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
        "面 0（アイランド 0,0）と面 3（アイランド 0,1）"
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
    let left_island = |p: &SurfacePixel| island_of(p.x, 256) == 2 && p.y < 128; // 面 2 はアイランド (2, 0)
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
    assert!(
        s.projection_stats().bucket_reuses > 0,
        "重なるダブは覚えた区画の投影の画素を使う"
    );
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
fn surface_stroke_skips_other_texture_sets_and_cancels_when_a_dab_does_not_fit_in_memory() {
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
    // 投影の画素がメモリに入らないダブがあれば、2D と同じくストロークごと取り消す（塗り残しを作らない）
    let before = doc.composite(doc.bounds()).unwrap();
    let undo = doc.undo_count();
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
    assert_eq!(s.stats.dabs, 1);
    s.set_projection_memory(Some(
        s.projection_bytes() - s.projection_stats().cached_bytes,
    ));
    let mut refused = None;
    for i in 1..20 {
        if let Err(e) = s.add(
            &mut doc,
            &mut stroke,
            center + Vec2::new(i as f32 * 3.0, 0.0),
            1.0,
        ) {
            refused = Some(e);
            break;
        }
    }
    let refused = match refused {
        Some(e) => e,
        None => s.finish(&mut doc, &mut stroke).unwrap_err(),
    };
    assert_eq!(refused, SurfaceStrokeError::Dab(DabRefusal::MemoryBudget));
    doc.cancel_stroke(stroke);
    assert!(!doc.has_active_stroke());
    assert_eq!(
        doc.composite(doc.bounds()).unwrap(),
        before,
        "最初のダブも戻る"
    );
    assert_eq!(doc.undo_count(), undo, "履歴にも残らない");
}

/// ステンシルを通した 3D のストローク: 画面に貼り付いた画像の白い所だけが塗られる。
mod stencil {
    use super::*;
    use crate::brush::{BrushStencil, ImageColorSpace, StencilImage, StencilMapping, StencilPoint};
    use crate::brush::{StencilMode, StencilTiling};

    const VIEW: f32 = 400.0;

    /// 左半分が白、右半分が黒の 64 × 8 の画像（量のモードで、左半分だけが通る）。
    fn half_image(white: bool, black: bool) -> Arc<StencilImage> {
        let mut rgba = Vec::new();
        for _ in 0..8 {
            for x in 0..64 {
                let v = if (x < 32 && white) || (x >= 32 && !black) {
                    255
                } else {
                    0
                };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        Arc::new(
            StencilImage::new(
                64,
                8,
                rgba,
                ImageColorSpace::Srgb,
                StencilImage::DEFAULT_MIP_BUDGET_BYTES,
            )
            .unwrap(),
        )
    }

    /// 表示域いっぱいに画像を貼る写し（画像の x は画面の x、画像の y は上向きなので画面の y と逆）。
    fn fill_view() -> SurfaceStencil {
        let to_image =
            StencilMapping::new(64.0 / VIEW as f64, 0.0, 0.0, 0.0, -8.0 / VIEW as f64, 8.0)
                .unwrap();
        SurfaceStencil::new(to_image, 64.0 / VIEW as f64).unwrap()
    }

    /// 上半分が白（`top_white` でなければ下半分が白）で、残りが黒の 8 × 64 の画像（行は下から。量のモードで、白い側だけが通る）。
    fn top_bottom_image(top_white: bool) -> Arc<StencilImage> {
        let mut rgba = Vec::new();
        for row in 0..64 {
            let v = if (row >= 32) == top_white { 255 } else { 0 };
            for _ in 0..8 {
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        Arc::new(
            StencilImage::new(
                8,
                64,
                rgba,
                ImageColorSpace::Srgb,
                StencilImage::DEFAULT_MIP_BUDGET_BYTES,
            )
            .unwrap(),
        )
    }

    /// `top_bottom_image` を表示域いっぱいに貼る写し。
    fn fill_view_tall() -> SurfaceStencil {
        let to_image =
            StencilMapping::new(8.0 / VIEW as f64, 0.0, 0.0, 0.0, -64.0 / VIEW as f64, 64.0)
                .unwrap();
        SurfaceStencil::new(to_image, 64.0 / VIEW as f64).unwrap()
    }

    /// 試験のカメラ（立方体を斜めから見る）。
    fn test_view() -> CameraView {
        let g = cube();
        let mut cam = OrbitCamera::framing(&g.bounds());
        cam.yaw = -40.0;
        cam.pitch = 15.0;
        cam.view(VIEW, VIEW)
    }

    /// 既定の線（手前の面から右の面へ、横に渡る）。
    fn across() -> (Vec3, Vec3) {
        (Vec3::new(-0.45, 0.0, -0.5), Vec3::new(0.5, 0.0, -0.25))
    }

    /// 手前の面を縦に渡る線（画面の上から下へ）。
    fn down() -> (Vec3, Vec3) {
        (Vec3::new(0.0, 0.45, -0.5), Vec3::new(0.0, -0.45, -0.5))
    }

    fn paint(
        image: Option<Arc<StencilImage>>,
        surface: Option<SurfaceStencil>,
    ) -> Result<std::collections::BTreeSet<(u32, u32)>, SurfaceStrokeError> {
        Ok(paint_alphas(image, surface)?.into_keys().collect())
    }

    /// 塗られた画素（覆いが 0 でない画素）と、その覆い。
    fn paint_alphas(
        image: Option<Arc<StencilImage>>,
        surface: Option<SurfaceStencil>,
    ) -> Result<std::collections::BTreeMap<(u32, u32), u8>, SurfaceStrokeError> {
        paint_line(image, surface, across(), 0.0, 1.0)
    }

    /// 線（モデルの 2 点）の `t0` から `t1`（0〜1。画面の点で内分）までを塗った画素の組。
    fn paint_part(
        image: Option<Arc<StencilImage>>,
        surface: Option<SurfaceStencil>,
        line: (Vec3, Vec3),
        t0: f32,
        t1: f32,
    ) -> std::collections::BTreeSet<(u32, u32)> {
        paint_line(image, surface, line, t0, t1)
            .unwrap()
            .into_keys()
            .collect()
    }

    fn paint_line(
        image: Option<Arc<StencilImage>>,
        surface: Option<SurfaceStencil>,
        line: (Vec3, Vec3),
        t0: f32,
        t1: f32,
    ) -> Result<std::collections::BTreeMap<(u32, u32), u8>, SurfaceStrokeError> {
        let mut doc = Document::new(256, 256).unwrap();
        let layer = doc.add_layer("1").unwrap();
        doc.clear_history().unwrap();
        let g = Arc::new(cube());
        let view = test_view();
        let settings = BrushSettings {
            radius: 16.0,
            color: Rgba8::new(220, 40, 30, 255),
            ..BrushSettings::default()
        };
        let mut brush = crate::Brush::from(settings);
        brush.stencil = image.map(|i| {
            Arc::new(BrushStencil::new(
                i,
                StencilMode::Mask,
                StencilTiling::None,
                false,
                None,
                &[],
            ))
        });
        let mut stroke = doc.begin_brush_stroke(layer, &brush).unwrap();
        let (a, b) = (
            view.to_screen(line.0).unwrap(),
            view.to_screen(line.1).unwrap(),
        );
        let (from, to) = (a + (b - a) * t0, a + (b - a) * t1);
        let begun = SurfaceStroke::begin_with_stencil(
            &mut doc,
            &mut stroke,
            g,
            view,
            &settings,
            Some(0),
            from,
            1.0,
            surface,
        );
        let mut s = match begun {
            Ok(s) => s,
            Err(e) => {
                doc.cancel_stroke(stroke);
                return Err(e);
            }
        };
        for i in 1..=12 {
            let p = from + (to - from) * (i as f32 / 12.0);
            s.add(&mut doc, &mut stroke, p, 1.0).unwrap();
        }
        s.finish(&mut doc, &mut stroke).unwrap();
        doc.end_stroke(stroke).unwrap();
        let mut painted = std::collections::BTreeMap::new();
        for y in 0..256 {
            for x in 0..256 {
                let a = doc.composite_pixel(crate::Channel::Color, x, y).unwrap().a;
                if a > 0 {
                    painted.insert((x, y), a);
                }
            }
        }
        Ok(painted)
    }

    #[test]
    fn only_the_open_part_of_the_stencil_is_painted() {
        let plain = paint(None, None).unwrap();
        assert!(plain.len() > 200, "{}", plain.len());
        // 全部が白: ステンシルを使わないのと同じ画素が塗られる
        let open = paint(Some(half_image(true, false)), Some(fill_view())).unwrap();
        assert_eq!(open, plain);
        // 全部が黒: 何も塗られない
        let shut = paint(Some(half_image(false, true)), Some(fill_view())).unwrap();
        assert!(shut.is_empty(), "{}", shut.len());
        // 半分: 左の画素だけ（使わないときに塗られる画素の一部で、画面の左へ寄る）
        let half = paint(Some(half_image(true, true)), Some(fill_view())).unwrap();
        assert!(
            half.len() > 20 && half.len() < plain.len(),
            "{}",
            half.len()
        );
        assert!(half.is_subset(&plain));
    }

    /// 線（画面へ写した 2 点）の `axis`（x が 0、y が 1）の座標が `at` になる所の `t`（0〜1）。
    fn t_at(line: (Vec3, Vec3), axis: usize, at: f32) -> f32 {
        let view = test_view();
        let (a, b) = (
            view.to_screen(line.0).unwrap(),
            view.to_screen(line.1).unwrap(),
        );
        (at - a[axis]) / (b[axis] - a[axis])
    }

    /// 通した画素が、画面の白い側の画素だけであること（黒い側の画素は 1 つも塗られず、白い側は大半が塗られる）。
    fn assert_only_the_white_side(
        through: &std::collections::BTreeSet<(u32, u32)>,
        white: &std::collections::BTreeSet<(u32, u32)>,
        black: &std::collections::BTreeSet<(u32, u32)>,
        what: &str,
    ) {
        assert!(
            white.len() > 50 && black.len() > 50,
            "{what}: {} / {}",
            white.len(),
            black.len()
        );
        assert!(
            white.is_disjoint(black),
            "{what}: 試験の線が境をまたいでいる"
        );
        assert_eq!(
            through.intersection(black).count(),
            0,
            "{what}: 黒い側が塗られた"
        );
        assert!(
            through.intersection(white).count() * 10 >= white.len() * 9,
            "{what}: 白い側が塗られない {} / {}",
            through.intersection(white).count(),
            white.len()
        );
    }

    #[test]
    fn the_open_side_of_the_stencil_is_the_side_of_the_screen_it_covers() {
        // 画像の白い左半分は画面の左半分に貼られる。線の画面の x が境（200）から 40 以上離れた左の部分と右の部分で、塗られる画素を
        // ステンシル無しで測っておき、通した結果はその左の画素だけ（右の画素が 1 つも無い）になる。左右が入れ替わる実装は落ちる
        let line = across();
        let view = test_view();
        let (a, b) = (
            view.to_screen(line.0).unwrap(),
            view.to_screen(line.1).unwrap(),
        );
        assert!(a.x < 160.0 && b.x > 240.0, "{a} {b}");
        let (t_left, t_right) = (t_at(line, 0, 160.0), t_at(line, 0, 240.0));
        let left = paint_part(None, None, line, 0.0, t_left);
        let right = paint_part(None, None, line, t_right, 1.0);
        let white_left = paint_part(
            Some(half_image(true, true)),
            Some(fill_view()),
            line,
            0.0,
            1.0,
        );
        assert_only_the_white_side(&white_left, &left, &right, "白が左");
        // 白と黒を入れ替えた画像（白が右）は、逆の側だけを通す
        let white_right = paint_part(
            Some(half_image(false, false)),
            Some(fill_view()),
            line,
            0.0,
            1.0,
        );
        assert_only_the_white_side(&white_right, &right, &left, "白が右");
        assert!(
            white_left.is_disjoint(&white_right)
                || white_left.intersection(&white_right).count() < white_left.len() / 4
        );
    }

    #[test]
    fn the_top_of_the_image_is_the_top_of_the_screen_in_3d() {
        // 8 × 64 の画像の上半分（画像の行は下から数えるので、後ろの行）が白。縦に渡る線の画面の y が境（200）から 40 以上離れた
        // 上の部分と下の部分で、通した画素が上だけ・（白と黒を替えると）下だけになる
        let line = down();
        let view = test_view();
        let (a, b) = (
            view.to_screen(line.0).unwrap(),
            view.to_screen(line.1).unwrap(),
        );
        assert!(a.y < 160.0 && b.y > 240.0, "{a} {b}");
        let (t_top, t_bottom) = (t_at(line, 1, 160.0), t_at(line, 1, 240.0));
        let top = paint_part(None, None, line, 0.0, t_top);
        let bottom = paint_part(None, None, line, t_bottom, 1.0);
        let white_top = paint_part(
            Some(top_bottom_image(true)),
            Some(fill_view_tall()),
            line,
            0.0,
            1.0,
        );
        assert_only_the_white_side(&white_top, &top, &bottom, "白が上");
        let white_bottom = paint_part(
            Some(top_bottom_image(false)),
            Some(fill_view_tall()),
            line,
            0.0,
            1.0,
        );
        assert_only_the_white_side(&white_bottom, &bottom, &top, "白が下");
    }

    #[test]
    fn the_stencil_follows_its_screen_placement_not_the_model() {
        // 画像を画面の右へずらすと、白い（左半分の）所が右へ動いて、塗られる画素の組が変わる
        let left = paint(Some(half_image(true, true)), Some(fill_view())).unwrap();
        let mut to_image = fill_view().screen_to_image();
        to_image.x0 = -32.0; // 画像の左端が画面の x = 200 になる（白い半分が右の半分を覆う）
        let moved = SurfaceStencil::new(to_image, 64.0 / VIEW as f64).unwrap();
        let right = paint(Some(half_image(true, true)), Some(moved)).unwrap();
        assert!(!right.is_empty() && right != left);
        assert!(left.is_disjoint(&right) || left.intersection(&right).count() < left.len() / 2);
    }

    #[test]
    fn a_stencil_brush_without_a_placement_is_refused() {
        // 画素ごとの点が無いので、文書が断る（黙って塗らない・ステンシルを無視して塗らない）
        let err = paint(Some(half_image(true, false)), None).unwrap_err();
        assert!(matches!(err, SurfaceStrokeError::Core(_)), "{err:?}");
    }

    // ───────── 置き場の口（足跡・点・断り）の直接の試験 ─────────

    /// XY 平面の三角形 1 つだけの面（カメラは −Z 側から +Z を見る）。
    fn single(a: Vec3, b: Vec3, c: Vec3, uv: [Vec2; 3]) -> SurfaceGeometry {
        SurfaceGeometry::new(
            vec![SurfaceTriangle::new(a, b, c, uv[0], uv[1], uv[2])],
            1,
            DEFAULT_WELD_TOLERANCE,
        )
        .unwrap()
    }

    /// 面積 2（(0,0)-(2,0)-(0,2)）、UV の面積は 64 × 64 の文書で 512 テクセル（(0,0)-(0.5,0)-(0,0.5)）の三角形。
    /// テクセルの大きさは √(2 / 512) = 1/16 モデルの単位。
    fn known_triangle() -> SurfaceGeometry {
        single(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
            [Vec2::ZERO, Vec2::new(0.5, 0.0), Vec2::new(0.0, 0.5)],
        )
    }

    fn hit_on(triangle: u32, position: Vec3) -> SurfaceHit {
        SurfaceHit {
            revision: 1,
            renderer: 0,
            material_slot: 0,
            material: 0,
            triangle,
            position,
            normal: Vec3::new(0.0, 0.0, -1.0),
            barycentric: Vec3::new(1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0),
            uv: Vec2::ZERO,
            distance: 0.0,
        }
    }

    /// (0.5, 0.5, 0) を真正面から、この距離で見る 400 × 400 のカメラ。
    fn camera_at(distance: f32) -> CameraView {
        OrbitCamera {
            target: Vec3::new(0.5, 0.5, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            distance,
            model_radius: 1.0,
            ..Default::default()
        }
        .view(400.0, 400.0)
    }

    fn surface(image_per_point: f64) -> SurfaceStencil {
        SurfaceStencil::new(fill_view().screen_to_image(), image_per_point).unwrap()
    }

    #[test]
    fn a_surface_stencil_refuses_a_non_finite_or_negative_ratio() {
        let to_image = fill_view().screen_to_image();
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.001] {
            assert!(
                matches!(
                    SurfaceStencil::new(to_image, bad),
                    Err(crate::error::CoreError::InvalidArgument(_))
                ),
                "{bad}"
            );
        }
        let zero = SurfaceStencil::new(to_image, 0.0).expect("0 は読める（いちばん細かい段）");
        assert_eq!(zero.image_per_point(), 0.0);
        assert_eq!(zero.screen_to_image(), to_image);
    }

    #[test]
    fn the_footprint_is_texel_size_in_screen_points_times_image_pixels_per_point() {
        let g = known_triangle();
        let hit = hit_on(0, Vec3::new(0.5, 0.5, 0.0));
        // テクセル 1/16 モデルの単位 → 距離 4 で 400 点の縦の画角 30° は 1 単位 = 400 / (2 × 4 × tan 15°) 点 → × 画像の画素 / 点
        let tan_half = 15.0f64.to_radians().tan();
        let expect = |distance: f64, per_point: f64| {
            (1.0 / 16.0) * 400.0 / (2.0 * distance * tan_half) * per_point
        };
        let near = surface(0.5).footprint(&g, &camera_at(4.0), &hit, 64, 64);
        assert!((near - expect(4.0, 0.5)).abs() < 1e-4, "{near}");
        // 比例: 画像の画素 / 点を 2 倍にすると 2 倍、距離を 2 倍にすると半分
        let double = surface(1.0).footprint(&g, &camera_at(4.0), &hit, 64, 64);
        assert!((double - 2.0 * near).abs() < 1e-4, "{double}");
        let far_hit = hit_on(0, Vec3::new(0.5, 0.5, 0.0));
        let far = surface(0.5).footprint(&g, &camera_at(8.0), &far_hit, 64, 64);
        assert!((far - expect(8.0, 0.5)).abs() < 1e-4 && far < near, "{far}");
        // 文書の画素が 4 倍（縦横 2 倍ずつ）なら、テクセルは半分
        let finer = surface(0.5).footprint(&g, &camera_at(4.0), &hit, 128, 128);
        assert!((finer - near / 2.0).abs() < 1e-4, "{finer}");
        // 0 は 0 のまま（負でも非有限でもない）。カメラの後ろ（近い面より手前）は画面の点が 0 になる（Unity 版の WorldRadiusToGuiPoints と同じ）
        assert_eq!(
            surface(0.0).footprint(&g, &camera_at(4.0), &hit, 64, 64),
            0.0
        );
        let behind = hit_on(0, Vec3::new(0.5, 0.5, -10.0));
        assert_eq!(
            surface(0.5).footprint(&g, &camera_at(4.0), &behind, 64, 64),
            0.0
        );
    }

    #[test]
    fn the_footprint_is_one_when_the_triangle_or_its_areas_are_unusable() {
        let view = camera_at(4.0);
        let at = Vec3::new(0.5, 0.5, 0.0);
        let uv = [Vec2::ZERO, Vec2::new(0.5, 0.0), Vec2::new(0.0, 0.5)];
        let point = |g: &SurfaceGeometry, triangle: u32| {
            surface(0.5).footprint(g, &view, &hit_on(triangle, at), 64, 64)
        };
        // 当たった三角形が無い
        assert_eq!(point(&known_triangle(), 7), 1.0);
        // 面積が 0（3 点が一直線）
        let line = single(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            uv,
        );
        assert_eq!(point(&line, 0), 1.0);
        // UV の面積が 0
        let no_uv = single(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
            [Vec2::new(0.3, 0.3); 3],
        );
        assert_eq!(point(&no_uv, 0), 1.0);
        // 外積は有限（面の組み立てを通る）が、長さを出すところで単精度が溢れて面積が有限でなくなる
        let huge = single(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0e15, 0.0, 0.0),
            Vec3::new(0.0, 2.0e15, 0.0),
            uv,
        );
        assert_eq!(point(&huge, 0), 1.0);
        // 同じ入力で正しいときは、1 ではない
        assert_ne!(point(&known_triangle(), 0), 1.0);
    }

    #[test]
    fn the_point_is_the_screen_position_mapped_into_the_image() {
        let view = camera_at(4.0);
        let to_image = StencilMapping::new(0.5, 0.0, 10.0, 0.0, -0.25, 7.0).unwrap();
        let s = SurfaceStencil::new(to_image, 0.5).unwrap();
        // 注視点は画面の中心（200, 200）に見える → 画像の (110, −43)
        let p = s.point(&view, Vec3::new(0.5, 0.5, 0.0), 2.5);
        assert!(
            (p.x - 110.0).abs() < 1e-3 && (p.y + 43.0).abs() < 1e-3,
            "{p:?}"
        );
        assert_eq!(p.footprint, 2.5, "足跡はそのまま添える");
    }

    #[test]
    fn a_point_behind_the_camera_or_too_far_is_read_from_nowhere() {
        let far = StencilImage::FAR_AWAY;
        let nowhere = |footprint: f64| StencilPoint {
            x: -far,
            y: -far,
            footprint,
        };
        let view = camera_at(4.0);
        let s = surface(0.5);
        // カメラの後ろ（距離 4 の位置の、さらに後ろ）
        assert_eq!(
            s.point(&view, Vec3::new(0.5, 0.5, -10.0), 3.5),
            nowhere(3.5)
        );
        // 写した先が FAR_AWAY 以上
        let wild = StencilMapping::new(1.0e12, 0.0, 0.0, 0.0, 1.0, 0.0).unwrap();
        let s = SurfaceStencil::new(wild, 0.5).unwrap();
        assert_eq!(s.point(&view, Vec3::new(0.5, 0.5, 0.0), 1.5), nowhere(1.5));
        let wild_y = StencilMapping::new(1.0, 0.0, 0.0, 0.0, -1.0e12, 0.0).unwrap();
        let s = SurfaceStencil::new(wild_y, 0.5).unwrap();
        assert_eq!(s.point(&view, Vec3::new(0.5, 0.5, 0.0), 1.5), nowhere(1.5));
    }

    /// 横 1 画素ごとに白と黒が入れ替わる 64 × 8 の縞（いちばん細かい段では縞、粗い段では灰色）。
    fn stripe_image() -> Arc<StencilImage> {
        let mut rgba = Vec::new();
        for _ in 0..8 {
            for x in 0..64 {
                let v = if x % 2 == 0 { 255 } else { 0 };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        Arc::new(
            StencilImage::new(
                64,
                8,
                rgba,
                ImageColorSpace::Srgb,
                StencilImage::DEFAULT_MIP_BUDGET_BYTES,
            )
            .unwrap(),
        )
    }

    #[test]
    fn a_larger_footprint_reads_a_coarser_mip_level_and_shows_in_the_painted_values() {
        // 置き場（画面 → 画像の写し）は同じで、足跡を決める画像の画素 / 点だけを変える。縞の画像は、細かい段では 0 か 255、粗い段では灰色
        let crisp = paint_alphas(Some(stripe_image()), Some(surface(0.0))).unwrap();
        let blurry = paint_alphas(Some(stripe_image()), Some(surface(3.0))).unwrap();
        let crisp_max = crisp.values().copied().max().unwrap();
        let blurry_max = blurry.values().copied().max().unwrap();
        assert_eq!(crisp_max, 255, "細かい段: 縞の白は全部通る");
        assert!(
            blurry_max < 200,
            "粗い段: 縞が灰色にならされて、全部は通らない（{blurry_max}）"
        );
        assert!(
            blurry.len() > crisp.len(),
            "灰色は縞の黒の所にも届く（{} と {}）",
            blurry.len(),
            crisp.len()
        );
    }
}
