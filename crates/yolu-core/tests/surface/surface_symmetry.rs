//! 3D の面の対称（Unity 版の SurfaceSymmetryTests・SurfaceRadialSymmetryTests。値は C# の試験の期待値そのもの）: 面のいちばん近い点の
//! 問い合わせ、左右対称の箱でミラーした側の対応するテクセルに同じ覆いが入ること、面の近くで重なっても画素ごとに大きいほうで 1 回だけ
//! 塗ること、写しが別のマテリアル・面の無い所・カメラから見えない側なら塗らないこと、写しの側が予算を超えればダブごと断ること、
//! 放射状の写しが全部の花弁に届くこと。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::collections::HashMap;

use yolu_core::geometry::{
    build_expanded, build_mirrored, copy_hits, find_copy, CopyHit, DabRefusal, DabSide,
    MirrorOutcome, MirrorPlane, RadialSymmetry, Ray, SurfaceBrushBudget, SurfaceDabResult,
    SurfaceGeometry, SurfaceHit, SurfaceTriangle, SymmetryAxis, SymmetryError,
    MAX_CLOSEST_POINT_NODE_VISITS,
};
use yolu_core::glam::{Quat, Vec2, Vec3};

// ───────── 左右対称の箱 ─────────

/// 箱 [-1, 1]³ の三角形。right_cells / left_cells は面の一辺の分け方、left_slot は左の半分のスロット。左を作らないなら left_cells = 0。
fn box_triangles(right_cells: i32, left_cells: i32, left_slot: i32) -> Vec<SurfaceTriangle> {
    let mut list = half(right_cells, 0);
    if left_cells > 0 {
        list.extend(half(left_cells, left_slot).into_iter().map(mirror_triangle));
    }
    list
}

/// 右の半分（x ≥ 0）。面の UV は前 v 0.05–0.25、後ろ 0.30–0.50、上 0.55–0.75、下 0.78–0.98（u は x = 0 で 0.5 から 0.7 まで）、
/// 右の側面は u 0.75–0.95・v 0.30–0.70。
fn half(cells: i32, slot: i32) -> Vec<SurfaceTriangle> {
    let mut list = Vec::new();
    #[allow(clippy::too_many_arguments)]
    fn face(
        list: &mut Vec<SurfaceTriangle>,
        cells: i32,
        slot: i32,
        o: Vec3,
        es: Vec3,
        et: Vec3,
        outward: Vec3,
        uo: Vec2,
        us: Vec2,
        ut: Vec2,
    ) {
        let point = |s: f32, t: f32| o + es * s + et * t;
        let uv = |s: f32, t: f32| uo + us * s + ut * t;
        let mut add = |a: Vec3, b: Vec3, c: Vec3, ua: Vec2, ub: Vec2, uc: Vec2| {
            let (b, c, ub, uc) = if (b - a).cross(c - a).dot(outward) < 0.0 {
                (c, b, uc, ub)
            } else {
                (b, c, ub, uc)
            };
            list.push(SurfaceTriangle::new(a, b, c, ua, ub, uc).with_slot(0, slot, -1));
        };
        for i in 0..cells {
            for j in 0..cells {
                let (s0, s1) = (i as f32 / cells as f32, (i + 1) as f32 / cells as f32);
                let (t0, t1) = (j as f32 / cells as f32, (j + 1) as f32 / cells as f32);
                add(
                    point(s0, t0),
                    point(s1, t0),
                    point(s1, t1),
                    uv(s0, t0),
                    uv(s1, t0),
                    uv(s1, t1),
                );
                add(
                    point(s0, t0),
                    point(s1, t1),
                    point(s0, t1),
                    uv(s0, t0),
                    uv(s1, t1),
                    uv(s0, t1),
                );
            }
        }
    }
    let x = Vec3::X;
    let (su, tv) = (Vec2::new(0.2, 0.0), Vec2::new(0.0, 0.2));
    face(
        &mut list,
        cells,
        slot,
        Vec3::new(0.0, -1.0, -1.0),
        x,
        Vec3::new(0.0, 2.0, 0.0),
        Vec3::NEG_Z,
        Vec2::new(0.5, 0.05),
        su,
        tv,
    );
    face(
        &mut list,
        cells,
        slot,
        Vec3::new(0.0, -1.0, 1.0),
        x,
        Vec3::new(0.0, 2.0, 0.0),
        Vec3::Z,
        Vec2::new(0.5, 0.30),
        su,
        tv,
    );
    face(
        &mut list,
        cells,
        slot,
        Vec3::new(0.0, 1.0, -1.0),
        x,
        Vec3::new(0.0, 0.0, 2.0),
        Vec3::Y,
        Vec2::new(0.5, 0.55),
        su,
        tv,
    );
    face(
        &mut list,
        cells,
        slot,
        Vec3::new(0.0, -1.0, -1.0),
        x,
        Vec3::new(0.0, 0.0, 2.0),
        Vec3::NEG_Y,
        Vec2::new(0.5, 0.78),
        su,
        tv,
    );
    face(
        &mut list,
        cells,
        slot,
        Vec3::new(1.0, -1.0, -1.0),
        Vec3::new(0.0, 0.0, 2.0),
        Vec3::new(0.0, 2.0, 0.0),
        Vec3::X,
        Vec2::new(0.75, 0.3),
        Vec2::new(0.2, 0.0),
        Vec2::new(0.0, 0.4),
    );
    list
}

/// x → −x、u → 1 − u（向きが逆になるので B と C を入れ替えて表を外に保つ）。
fn mirror_triangle(t: SurfaceTriangle) -> SurfaceTriangle {
    let m = |p: Vec3| Vec3::new(-p.x, p.y, p.z);
    let w = |uv: Vec2| Vec2::new(1.0 - uv.x, uv.y);
    SurfaceTriangle::new(m(t.a), m(t.c), m(t.b), w(t.uv_a), w(t.uv_c), w(t.uv_b)).with_slot(
        t.renderer,
        t.material_slot,
        t.material,
    )
}

fn geometry(t: Vec<SurfaceTriangle>) -> SurfaceGeometry {
    SurfaceGeometry::new(t, 1, 0.000_001).unwrap()
}

// ───────── 最近点 ─────────

const SIZE: i32 = 128;
const FRONT_CAMERA: Vec3 = Vec3::new(0.0, 0.0, -5.0);

/// z = 0 の単位の正方形（表は −Z）。UV は x, y のまま。
fn quad() -> SurfaceGeometry {
    geometry(vec![
        SurfaceTriangle::new(Vec3::ZERO, Vec3::Y, Vec3::X, Vec2::ZERO, Vec2::Y, Vec2::X),
        SurfaceTriangle::new(
            Vec3::X,
            Vec3::Y,
            Vec3::new(1.0, 1.0, 0.0),
            Vec2::X,
            Vec2::Y,
            Vec2::ONE,
        ),
    ])
}
fn closest(g: &SurfaceGeometry, p: Vec3, max: f32, facing: Vec3) -> Option<SurfaceHit> {
    g.find_closest_point(p, max, facing, MAX_CLOSEST_POINT_NODE_VISITS, -1)
        .unwrap()
}

#[test]
fn the_closest_point_is_the_projection_inside_the_edge_outside_and_the_corner_beyond() {
    let g = quad();
    let inside = closest(&g, Vec3::new(0.3, 0.4, 0.25), 1.0, Vec3::ZERO).unwrap();
    assert!(inside.position.distance(Vec3::new(0.3, 0.4, 0.0)) < 1e-6);
    assert!((inside.distance - 0.25).abs() < 1e-6);
    assert!((inside.uv.x - 0.3).abs() < 1e-5 && (inside.uv.y - 0.4).abs() < 1e-5);
    let t = g.triangles()[inside.triangle as usize];
    let rebuilt =
        t.a * inside.barycentric.x + t.b * inside.barycentric.y + t.c * inside.barycentric.z;
    assert!(
        rebuilt.distance(inside.position) < 1e-6,
        "重心座標で点が戻る"
    );
    assert!((inside.normal.z - -1.0).abs() < 1e-6);

    let edge = closest(&g, Vec3::new(1.5, 0.5, 0.0), 1.0, Vec3::ZERO).unwrap();
    assert!(edge.position.distance(Vec3::new(1.0, 0.5, 0.0)) < 1e-6);
    assert!((edge.distance - 0.5).abs() < 1e-6);
    let corner = closest(&g, Vec3::new(1.3, 1.4, 0.0), 1.0, Vec3::ZERO).unwrap();
    assert!(corner.position.distance(Vec3::new(1.0, 1.0, 0.0)) < 1e-6);
    assert!((corner.distance - 0.5).abs() < 1e-6);

    assert!(
        closest(&g, Vec3::new(1.5, 0.5, 0.0), 0.49, Vec3::ZERO).is_none(),
        "上限より遠い"
    );
    assert!(
        closest(&g, Vec3::new(1.5, 0.5, 0.0), 0.5, Vec3::ZERO).is_some(),
        "上限ちょうど"
    );
}

/// 平面への射影が三角形の中ならその距離、外なら 3 辺への距離の最小（最近点とは別の素朴な計算）。
fn naive_distance(p: Vec3, t: &SurfaceTriangle) -> f32 {
    let n = (t.b - t.a).cross(t.c - t.a).normalize();
    let q = p - n * (p - t.a).dot(n);
    let inside = |a: Vec3, b: Vec3| (b - a).cross(q - a).dot(n) >= 0.0;
    if inside(t.a, t.b) && inside(t.b, t.c) && inside(t.c, t.a) {
        return (p - t.a).dot(n).abs();
    }
    let segment = |a: Vec3, b: Vec3| {
        let ab = b - a;
        let s = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
        p.distance(a + ab * s)
    };
    segment(t.a, t.b)
        .min(segment(t.b, t.c))
        .min(segment(t.c, t.a))
}

#[test]
fn the_search_matches_brute_force_on_a_closed_box_and_honours_the_facing() {
    let triangles = box_triangles(3, 3, 0);
    let g = geometry(triangles.clone());
    let mut seed = 7u64;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as f32 / (1u64 << 31) as f32) * 3.0 - 1.5
    };
    for _ in 0..200 {
        let p = Vec3::new(next(), next(), next());
        let hit = closest(&g, p, f32::INFINITY, Vec3::ZERO).unwrap();
        let brute = triangles
            .iter()
            .map(|t| naive_distance(p, t))
            .fold(f32::INFINITY, f32::min);
        assert!((hit.distance - brute).abs() < 1e-5, "{p}");
    }
    // 箱の中の点: 近いのは +X の面（0.5）。−X を向く面だけなら反対の面（1.5）
    let inside = Vec3::new(0.5, 0.2, 0.1);
    let near = closest(&g, inside, 10.0, Vec3::ZERO).unwrap();
    assert!((near.distance - 0.5).abs() < 1e-5 && (near.normal.x - 1.0).abs() < 1e-5);
    let facing = closest(&g, inside, 10.0, Vec3::NEG_X).unwrap();
    assert!((facing.distance - 1.5).abs() < 1e-5 && (facing.normal.x - -1.0).abs() < 1e-5);
    assert!(
        closest(&g, inside, 1.0, Vec3::NEG_X).is_none(),
        "向きの合う面は上限の外"
    );
}

#[test]
fn the_search_stops_at_its_node_budget() {
    let g = geometry(box_triangles(4, 4, 0));
    assert!(g
        .find_closest_point(Vec3::ZERO, 10.0, Vec3::ZERO, 1, -1)
        .is_err());
    assert!(matches!(
        g.find_closest_point(
            Vec3::ZERO,
            10.0,
            Vec3::ZERO,
            MAX_CLOSEST_POINT_NODE_VISITS,
            -1
        ),
        Ok(Some(_))
    ));
}

// ───────── ミラー ─────────

fn pick(g: &SurfaceGeometry, camera: Vec3, at: Vec3) -> SurfaceHit {
    g.raycast(Ray::new(camera, at - camera), true, f32::INFINITY)
        .expect("当たる")
}
fn plane_x() -> MirrorPlane {
    MirrorPlane::from_model(Vec3::ZERO, Quat::IDENTITY, SymmetryAxis::X, 0.0)
}
fn map(r: &SurfaceDabResult) -> HashMap<(i32, i32), f32> {
    r.pixels.iter().map(|p| ((p.x, p.y), p.coverage)).collect()
}
fn triples(r: &SurfaceDabResult) -> Vec<(i32, i32, f32)> {
    r.pixels.iter().map(|p| (p.x, p.y, p.coverage)).collect()
}
fn mirrored(
    g: &SurfaceGeometry,
    hit: &SurfaceHit,
    radius: f32,
    budget: &SurfaceBrushBudget,
) -> yolu_core::geometry::SymmetricSurfaceDab {
    build_mirrored(
        g,
        hit,
        &plane_x(),
        radius,
        SIZE,
        SIZE,
        FRONT_CAMERA,
        0.5,
        budget,
        None,
        false,
    )
}

/// a のどの画素も、b の映した画素（W − 1 − x）に同じ覆いがある（境の丸めで片方だけに出る、ほぼ 0 の画素は除く）。
fn assert_mirrored(a: &HashMap<(i32, i32), f32>, b: &HashMap<(i32, i32), f32>, what: &str) {
    let mut compared = 0;
    for (&(x, y), &coverage) in a {
        let other = b.get(&(SIZE - 1 - x, y)).copied().unwrap_or(0.0);
        if coverage < 1e-3 && other < 1e-3 {
            continue;
        }
        assert!(
            (other - coverage).abs() < 1e-4,
            "{what} at {x},{y}: {other} vs {coverage}"
        );
        compared += 1;
    }
    assert!(compared > 20, "{what}: ダブは実際の面積を覆う");
}

#[test]
fn a_point_on_one_side_paints_the_mirrored_texels_with_the_same_coverage() {
    let g = geometry(box_triangles(1, 1, 0));
    let hit = pick(&g, FRONT_CAMERA, Vec3::new(0.4, 0.1, -1.0));
    let dab = mirrored(&g, &hit, 0.3, &SurfaceBrushBudget::default());
    assert_eq!(dab.outcome, MirrorOutcome::Painted);
    let mirror_hit = dab.mirror_hit.expect("映した点は面に落ちる");
    assert!(
        mirror_hit
            .position
            .distance(Vec3::new(-hit.position.x, hit.position.y, hit.position.z))
            < 1e-5
    );
    let original = map(&dab.original);
    let mirror = map(dab.mirror.as_ref().unwrap());
    assert!(
        original.keys().all(|k| k.0 >= SIZE / 2) && mirror.keys().all(|k| k.0 < SIZE / 2),
        "0.8 離れて半径 0.3: 2 つのダブは別々"
    );
    assert_mirrored(&original, &mirror, "元の写し");
    assert_mirrored(&mirror, &original, "写しの元");
    assert_eq!(dab.result.pixels.len(), original.len() + mirror.len());
    assert_eq!(dab.result.refusal, None);
}

#[test]
fn near_the_plane_the_two_dabs_are_joined_by_the_larger_coverage_once() {
    let g = geometry(box_triangles(1, 1, 0));
    let hit = pick(&g, FRONT_CAMERA, Vec3::new(0.1, 0.1, -1.0));
    let dab = mirrored(&g, &hit, 0.3, &SurfaceBrushBudget::default());
    assert_eq!(dab.outcome, MirrorOutcome::Painted);
    let original = map(&dab.original);
    let mirror = map(dab.mirror.as_ref().unwrap());
    let overlap = original.keys().filter(|k| mirror.contains_key(k)).count();
    assert!(overlap > 20, "ダブは面をまたいで重なる");
    let result = &dab.result.pixels;
    let distinct: std::collections::HashSet<_> = result.iter().map(|p| (p.x, p.y)).collect();
    assert_eq!(distinct.len(), result.len(), "どの画素も 1 回");
    let union: std::collections::HashSet<_> = original.keys().chain(mirror.keys()).collect();
    assert_eq!(result.len(), union.len());
    for p in result {
        let expected = original
            .get(&(p.x, p.y))
            .copied()
            .unwrap_or(0.0)
            .max(mirror.get(&(p.x, p.y)).copied().unwrap_or(0.0));
        assert_eq!(p.coverage, expected, "足さずに大きいほう: {},{}", p.x, p.y);
    }
    let keys: Vec<i32> = result.iter().map(|p| p.y * SIZE + p.x).collect();
    assert!(
        keys.windows(2).all(|w| w[0] <= w[1]),
        "どのダブとも同じ、左下からの行の順"
    );
    assert_mirrored(
        &map(&dab.result),
        &map(&dab.result),
        "合わせたダブは左右対称",
    );
}

#[test]
fn a_dab_centered_on_the_plane_is_painted_once() {
    let g = geometry(box_triangles(1, 1, 0));
    let hit = pick(&g, FRONT_CAMERA, Vec3::new(0.0, 0.1, -1.0));
    assert!(hit.position.x.abs() < 1e-6);
    let dab = mirrored(&g, &hit, 0.3, &SurfaceBrushBudget::default());
    assert_eq!(dab.outcome, MirrorOutcome::OnPlane);
    assert!(dab.mirror.is_none());
    let alone = g.build_surface_dabs(
        &hit,
        0.3,
        SIZE,
        SIZE,
        FRONT_CAMERA,
        0.5,
        &SurfaceBrushBudget::default(),
        None,
        false,
    );
    assert_eq!(triples(&dab.result), triples(&alone));
}

#[test]
fn a_mirror_on_another_slot_or_with_no_surface_is_not_painted() {
    let other = geometry(box_triangles(1, 1, 1));
    let hit = pick(&other, FRONT_CAMERA, Vec3::new(0.4, 0.1, -1.0));
    let dab = mirrored(&other, &hit, 0.3, &SurfaceBrushBudget::default());
    assert_eq!(dab.outcome, MirrorOutcome::OtherSlot);
    assert_eq!(dab.mirror_hit.unwrap().material_slot, 1);
    assert_eq!(triples(&dab.result), triples(&dab.original), "こちら側だけ");

    // 左の半分が無い: 映した点 (−0.4, …) から向きの合う面はいちばん近くても 0.4 先（半径 0.3 より遠い）
    let half_box = geometry(box_triangles(1, 0, 0));
    let hit = pick(&half_box, FRONT_CAMERA, Vec3::new(0.4, 0.1, -1.0));
    let dab = mirrored(&half_box, &hit, 0.3, &SurfaceBrushBudget::default());
    assert_eq!(dab.outcome, MirrorOutcome::NoSurface);
    assert_eq!(dab.result.pixels.len(), dab.original.pixels.len());
    assert!(!dab.result.pixels.is_empty());
    // 半径がそれより大きければ、届く面（右の前の面の、面の際）に置く
    let dab = mirrored(&half_box, &hit, 0.45, &SurfaceBrushBudget::default());
    assert_eq!(dab.outcome, MirrorOutcome::Painted);
    assert!(dab.mirror_hit.unwrap().position.x.abs() < 1e-5);
}

#[test]
fn a_mirror_the_camera_cannot_see_is_not_painted() {
    let g = geometry(box_triangles(1, 1, 0));
    let camera = Vec3::new(5.0, 0.2, 0.3); // +X の側から見る: 映した −X の面は裏
    let hit = pick(&g, camera, Vec3::new(1.0, 0.2, 0.3));
    let dab = build_mirrored(
        &g,
        &hit,
        &plane_x(),
        0.3,
        SIZE,
        SIZE,
        camera,
        0.5,
        &SurfaceBrushBudget::default(),
        None,
        false,
    );
    assert_eq!(dab.outcome, MirrorOutcome::Hidden);
    assert!(
        (dab.mirror_hit.unwrap().normal.x - -1.0).abs() < 1e-5,
        "写しは向きの合う奥の面に落ちた"
    );
    assert!(dab.mirror.as_ref().unwrap().pixels.is_empty());
    assert_eq!(dab.result.pixels.len(), dab.original.pixels.len());
    assert!(!dab.result.pixels.is_empty());
}

#[test]
fn a_mirrored_dab_over_its_budget_refuses_the_whole_dab() {
    // 右は粗く（面ごとに 2 枚）、映した左は細かい（面ごとに 512 枚）。三角形の予算 64 は右なら足り、左では超える
    let g = geometry(box_triangles(1, 16, 0));
    let hit = pick(&g, FRONT_CAMERA, Vec3::new(0.4, 0.1, -1.0));
    let budget = SurfaceBrushBudget {
        max_triangles: 64,
        ..SurfaceBrushBudget::default()
    };
    let alone = g.build_surface_dabs(
        &hit,
        0.3,
        SIZE,
        SIZE,
        FRONT_CAMERA,
        0.5,
        &budget,
        None,
        false,
    );
    assert_eq!(alone.refusal, None);
    let dab = mirrored(&g, &hit, 0.3, &budget);
    assert!(dab.result.was_clipped());
    assert!(dab.result.pixels.is_empty(), "一部の覆いも返さない");
    assert_eq!(dab.result.refusal, Some(DabRefusal::TriangleBudget));
    assert_eq!(dab.refused_side, Some(DabSide::Mirror));
    // 予算が足りれば同じ所に塗れる（左右で三角形の分け方が違っても、覆いは同じ）
    let dab = mirrored(&g, &hit, 0.3, &SurfaceBrushBudget::default());
    assert_eq!(dab.outcome, MirrorOutcome::Painted);
    assert_mirrored(
        &map(&dab.original),
        &map(dab.mirror.as_ref().unwrap()),
        "細かい半分と粗い半分",
    );
}

#[test]
fn the_plane_follows_the_model_root_axis_and_offset() {
    // ルートのローカルの X は世界の −Z（Unity の Euler(0, 90, 0)）
    let rotation = Quat::from_axis_angle(Vec3::Y, 90f32.to_radians());
    let plane = MirrorPlane::from_model(Vec3::new(1.0, 2.0, 3.0), rotation, SymmetryAxis::X, 0.5);
    assert!(plane.normal.distance(rotation * Vec3::X) < 1e-6);
    assert!((plane.signed_distance(Vec3::new(1.0, 2.0, 3.0)) - -0.5).abs() < 1e-6);
    let p = Vec3::new(4.0, -1.0, 7.0);
    assert!(
        plane.reflect(plane.reflect(p)).distance(p) < 1e-5,
        "2 回映すと戻る"
    );
    assert!((plane.signed_distance(plane.reflect(p)) - -plane.signed_distance(p)).abs() < 1e-5);
    let y = MirrorPlane::from_model(Vec3::ZERO, Quat::IDENTITY, SymmetryAxis::Y, 0.0);
    assert_eq!(
        y.reflect(Vec3::new(1.0, 2.0, 3.0)),
        Vec3::new(1.0, -2.0, 3.0)
    );
    let z = MirrorPlane::from_model(Vec3::ZERO, Quat::IDENTITY, SymmetryAxis::Z, 1.0);
    assert_eq!(
        z.reflect(Vec3::new(1.0, 2.0, 3.0)),
        Vec3::new(1.0, 2.0, -1.0)
    );
    assert_eq!(
        MirrorPlane::new(Vec3::ZERO, Vec3::ZERO, Vec3::X, Vec3::Y).err(),
        Some(SymmetryError::Axis)
    );
    assert_eq!(
        MirrorPlane::new(Vec3::NAN, Vec3::X, Vec3::Y, Vec3::Z).err(),
        Some(SymmetryError::Origin)
    );
}

// ───────── 放射状 ─────────

/// 軸 +Z のまわりに count 枚の花弁（中心から 0.6〜1.4 の板。UV は枚ごとに別の帯）。occluder は −X の側の手前に置く遮蔽の板（別のスロット）。
fn petals(count: i32, other_slot: i32, occluder: bool) -> Vec<SurfaceTriangle> {
    let mut triangles = Vec::new();
    for i in 0..count {
        let r = Quat::from_axis_angle(Vec3::Z, (360.0 * i as f32 / count as f32).to_radians());
        let (a, b, c, d) = (
            r * Vec3::new(0.6, -0.3, 0.0),
            r * Vec3::new(1.4, -0.3, 0.0),
            r * Vec3::new(1.4, 0.3, 0.0),
            r * Vec3::new(0.6, 0.3, 0.0),
        );
        let (u, v) = (i as f32 / count as f32, (i as f32 + 1.0) / count as f32);
        let slot = if i == other_slot { 1 } else { 0 };
        triangles.push(
            SurfaceTriangle::new(
                a,
                c,
                b,
                Vec2::new(u, 0.1),
                Vec2::new(v, 0.9),
                Vec2::new(v, 0.1),
            )
            .with_slot(0, slot, -1),
        );
        triangles.push(
            SurfaceTriangle::new(
                a,
                d,
                c,
                Vec2::new(u, 0.1),
                Vec2::new(u, 0.9),
                Vec2::new(v, 0.9),
            )
            .with_slot(0, slot, -1),
        );
    }
    if occluder {
        triangles.push(
            SurfaceTriangle::new(
                Vec3::new(-1.5, -0.5, -0.2),
                Vec3::new(-1.5, 0.5, -0.2),
                Vec3::new(-0.5, 0.5, -0.2),
                Vec2::ZERO,
                Vec2::Y,
                Vec2::ONE,
            )
            .with_slot(1, 1, -1),
        );
        triangles.push(
            SurfaceTriangle::new(
                Vec3::new(-1.5, -0.5, -0.2),
                Vec3::new(-0.5, 0.5, -0.2),
                Vec3::new(-0.5, -0.5, -0.2),
                Vec2::ZERO,
                Vec2::ONE,
                Vec2::X,
            )
            .with_slot(1, 1, -1),
        );
    }
    triangles
}
fn pick_from(g: &SurfaceGeometry, at: Vec3, camera: Vec3) -> SurfaceHit {
    g.raycast(Ray::new(camera, at - camera), true, f32::INFINITY)
        .expect("当たる")
}
fn radial_of(count: u32) -> RadialSymmetry {
    RadialSymmetry::new(Vec3::ZERO, Vec3::Z, count).unwrap()
}
const EYE: Vec3 = Vec3::new(0.0, 0.0, -5.0);

#[test]
fn rotation_follows_root_axes_and_maintains_radius() {
    for (axis, count) in [
        (SymmetryAxis::X, 2u32),
        (SymmetryAxis::Y, 7),
        (SymmetryAxis::Z, 16),
    ] {
        let root = Vec3::new(3.0, 4.0, 5.0);
        let rotation = Quat::from_euler(
            yolu_core::glam::EulerRot::YXZ,
            35f32.to_radians(),
            20f32.to_radians(),
            12f32.to_radians(),
        );
        let s = RadialSymmetry::from_model(root, rotation, axis, count).unwrap();
        let local_axis = axis.direction();
        let local = Vec3::new(0.8, 0.3, -0.7);
        let p = root + rotation * local;
        for i in 0..count {
            let q =
                Quat::from_axis_angle(local_axis, (360.0 * i as f32 / count as f32).to_radians());
            let expected = root + rotation * (q * local);
            assert!(
                s.rotate_point(p, i).distance(expected) < 1e-5,
                "{axis:?} {i}"
            );
        }
    }
}

#[test]
fn radial_dabs_land_on_all_petals_with_matching_coverage() {
    for count in [2u32, 3, 4, 7, 16] {
        let g = geometry(petals(count as i32, -1, false));
        let hit = pick_from(&g, Vec3::X, EYE);
        let dab = build_expanded(
            &g,
            &hit,
            None,
            Some(&radial_of(count)),
            true,
            0.18,
            512,
            128,
            EYE,
            0.5,
            &SurfaceBrushBudget::default(),
            None,
        );
        assert_eq!(dab.result.refusal, None, "{count}");
        assert_eq!(dab.copies.len(), count as usize - 1);
        let bands: std::collections::HashSet<i32> = dab
            .result
            .pixels
            .iter()
            .map(|p| p.x * count as i32 / 512)
            .collect();
        assert_eq!(bands.len(), count as usize, "どの花弁にも塗る");
        for copy in &dab.copies {
            assert!((copy.position.length() - 1.0).abs() < 1e-5);
        }
    }
}

#[test]
fn mirror_and_radial_copies_merge_overlap_by_maximum() {
    let g = geometry(petals(4, -1, false));
    let hit = pick_from(&g, Vec3::X, EYE);
    let radial = radial_of(4);
    let mirror = MirrorPlane::from_model(Vec3::ZERO, Quat::IDENTITY, SymmetryAxis::Y, 0.0);
    let budget = SurfaceBrushBudget::default();
    let one = build_expanded(
        &g,
        &hit,
        None,
        Some(&radial),
        true,
        0.2,
        256,
        128,
        EYE,
        0.8,
        &budget,
        None,
    );
    let both = build_expanded(
        &g,
        &hit,
        Some(&mirror),
        Some(&radial),
        true,
        0.2,
        256,
        128,
        EYE,
        0.8,
        &budget,
        None,
    );
    assert_eq!(both.copies.len(), 3, "面の上の鏡映と回転の重複は省く");
    assert_eq!(triples(&both.result), triples(&one.result));
    let hit = pick_from(&g, Vec3::new(1.0, 0.1, 0.0), EYE);
    let both = build_expanded(
        &g,
        &hit,
        Some(&mirror),
        Some(&radial),
        true,
        0.2,
        256,
        128,
        EYE,
        0.8,
        &budget,
        None,
    );
    let mut maximum: HashMap<(i32, i32), f32> = HashMap::new();
    for point in std::iter::once(hit).chain(both.copies.iter().copied()) {
        let dab = g.build_surface_dabs(&point, 0.2, 256, 128, EYE, 0.8, &budget, None, true);
        for p in dab.pixels {
            let e = maximum.entry((p.x, p.y)).or_insert(0.0);
            *e = e.max(p.coverage);
        }
    }
    assert_eq!(both.copies.len(), 7);
    assert_eq!(both.result.pixels.len(), maximum.len());
    for p in &both.result.pixels {
        assert!((p.coverage - maximum[&(p.x, p.y)]).abs() < 1e-5);
    }
}

#[test]
fn ignore_visibility_paints_occluded_copies_and_keeps_the_original_visible_only() {
    let g = geometry(petals(4, -1, true));
    let hit = pick_from(&g, Vec3::X, EYE);
    let radial = radial_of(4);
    let budget = SurfaceBrushBudget::default();
    let hidden = build_expanded(
        &g,
        &hit,
        None,
        Some(&radial),
        false,
        0.2,
        256,
        128,
        EYE,
        0.8,
        &budget,
        None,
    );
    let painted = build_expanded(
        &g,
        &hit,
        None,
        Some(&radial),
        true,
        0.2,
        256,
        128,
        EYE,
        0.8,
        &budget,
        None,
    );
    let in_third = |r: &SurfaceDabResult| r.pixels.iter().any(|p| (128..192).contains(&p.x));
    assert_eq!(hidden.outcome, MirrorOutcome::Hidden);
    assert!(!in_third(&hidden.result));
    assert!(in_third(&painted.result));
    assert!(painted.result.visibility_rays < hidden.result.visibility_rays);
}

#[test]
fn ignore_visibility_paints_the_mirrored_back_face_without_camera_rays() {
    let g = geometry(box_triangles(1, 1, 0));
    let eye = Vec3::new(5.0, 0.0, 0.0);
    let hit = pick_from(&g, Vec3::new(1.0, 0.2, 0.3), eye);
    let dab = build_mirrored(
        &g,
        &hit,
        &plane_x(),
        0.3,
        256,
        256,
        eye,
        0.8,
        &SurfaceBrushBudget::default(),
        None,
        true,
    );
    assert_eq!(dab.outcome, MirrorOutcome::Painted);
    let mirror = dab.mirror.unwrap();
    assert!(!mirror.pixels.is_empty());
    assert_eq!(mirror.visibility_rays, 0);
    assert!(dab.original.visibility_rays > 0);
}

#[test]
fn another_slot_and_missing_petal_are_skipped_with_a_reason() {
    let g = geometry(petals(4, 2, false));
    let hit = pick_from(&g, Vec3::X, EYE);
    let radial = radial_of(4);
    let budget = SurfaceBrushBudget::default();
    let dab = build_expanded(
        &g,
        &hit,
        None,
        Some(&radial),
        true,
        0.2,
        256,
        128,
        EYE,
        0.8,
        &budget,
        None,
    );
    assert_eq!(dab.outcome, MirrorOutcome::OtherSlot);
    assert!(!dab.result.pixels.iter().any(|p| (128..192).contains(&p.x)));
    let g = geometry(petals(4, -1, false).into_iter().take(6).collect());
    let hit = pick_from(&g, Vec3::X, EYE);
    let dab = build_expanded(
        &g,
        &hit,
        None,
        Some(&radial),
        true,
        0.2,
        256,
        128,
        EYE,
        0.8,
        &budget,
        None,
    );
    assert_eq!(dab.outcome, MirrorOutcome::NoSurface);
}

#[test]
fn combined_triangle_budget_refuses_all_pixels() {
    let g = geometry(petals(4, -1, false));
    let hit = pick_from(&g, Vec3::X, EYE);
    let budget = SurfaceBrushBudget {
        max_triangles: 3,
        ..SurfaceBrushBudget::default()
    };
    assert_eq!(
        g.build_surface_dabs(&hit, 0.2, 256, 128, EYE, 0.5, &budget, None, false)
            .refusal,
        None
    );
    let dab = build_expanded(
        &g,
        &hit,
        None,
        Some(&radial_of(4)),
        true,
        0.2,
        256,
        128,
        EYE,
        0.5,
        &budget,
        None,
    );
    assert!(dab.result.was_clipped());
    assert!(dab.result.pixels.is_empty());
    assert_eq!(dab.result.refusal, Some(DabRefusal::TriangleBudget));
    assert_eq!(dab.refused_side, Some(DabSide::Copy));
}

#[test]
fn invalid_copy_counts_are_refused() {
    for count in [1, 17, 0] {
        assert_eq!(
            RadialSymmetry::new(Vec3::ZERO, Vec3::Y, count).err(),
            Some(SymmetryError::Count)
        );
    }
    assert_eq!(
        RadialSymmetry::new(Vec3::ZERO, Vec3::ZERO, 4).err(),
        Some(SymmetryError::Axis)
    );
    assert_eq!(
        RadialSymmetry::new(Vec3::INFINITY, Vec3::Y, 4).err(),
        Some(SymmetryError::Origin)
    );
}

// ───────── 写しの探索（カーソルと共有） ─────────

#[test]
fn copy_hits_list_the_copies_the_dab_would_paint() {
    let g = geometry(petals(4, 2, false));
    let hit = pick_from(&g, Vec3::X, EYE);
    let radial = radial_of(4);
    // 別のスロットの花弁は、塗らないのでカーソルにも出さない
    let hits = copy_hits(&g, &hit, None, Some(&radial), 0.2);
    assert_eq!(hits.len(), 2);
    let dab = build_expanded(
        &g,
        &hit,
        None,
        Some(&radial),
        true,
        0.2,
        256,
        128,
        EYE,
        0.8,
        &SurfaceBrushBudget::default(),
        None,
    );
    assert_eq!(dab.copies.len(), 2);
    for (a, b) in hits.iter().zip(&dab.copies) {
        assert_eq!(a.triangle, b.triangle);
    }
    // 元と同じ位置に落ちる写しは、重複として飛ばす
    let mut positions = vec![hit.position];
    assert_eq!(
        find_copy(&g, &hit, None, Some(&radial), 0, true, 0.2, &mut positions),
        CopyHit::Duplicate
    );
}
