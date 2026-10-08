//! UV の位相（位相の対応・アイランドの図・帯の写し）を並列（rayon）で作った結果と、今のスレッドで順に作った結果が同じこと。rayon のスレッドの中では
//! 並列にせずに作る（門を持ったまま rayon の仕事を待たない。`uv_topology::may_parallelize`）ので、どちらの道で作っても同じ物になる必要がある。

use std::sync::Arc;

use glam::{Vec2, Vec3};

use super::seam_band::{self, SeamBand};
use super::uv_topology::{build_island_map, may_parallelize, Parts};
use super::{
    SurfaceGeometry, SurfaceTriangle, UvTopology, UvTopologyError, DEFAULT_WELD_TOLERANCE,
};

/// n × n の四角を 3D で隣どうしにつなぎ、UV では並びを散らした升に置いたモデル（三角形が 2 n²。1024 を超えれば位相の対応の作りも並列に分かれる）。
fn atlas(n: u32) -> Arc<SurfaceGeometry> {
    let mut tris = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let c = u64::from(i + n * j);
            let cell = (c * 7919 + 13) % u64::from(n * n);
            let (cx, cy) = ((cell % u64::from(n)) as f32, (cell / u64::from(n)) as f32);
            let uv = |x: f32, y: f32| {
                Vec2::new(
                    (cx + 0.25 + 0.5 * x) / n as f32,
                    (cy + 0.25 + 0.5 * y) / n as f32,
                )
            };
            let p = |x: f32, y: f32| Vec3::new(i as f32 + x, j as f32 + y, 0.0);
            tris.push(SurfaceTriangle::new(
                p(0., 0.),
                p(1., 0.),
                p(1., 1.),
                uv(0., 0.),
                uv(1., 0.),
                uv(1., 1.),
            ));
            tris.push(SurfaceTriangle::new(
                p(0., 0.),
                p(1., 1.),
                p(0., 1.),
                uv(0., 0.),
                uv(1., 1.),
                uv(0., 1.),
            ));
        }
    }
    Arc::new(SurfaceGeometry::new(tris, 1, DEFAULT_WELD_TOLERANCE).unwrap())
}

/// 帯の写しの中身が同じか（作るのにかかった時間は見ない）。
fn same_band(a: &SeamBand, b: &SeamBand) -> bool {
    let (x, y) = (a.stats(), b.stats());
    a.same_content(b)
        && (
            x.seam_edges,
            x.chart_triangles,
            x.texels,
            x.conflicts,
            x.capped_edges,
        ) == (
            y.seam_edges,
            y.chart_triangles,
            y.texels,
            y.conflicts,
            y.capped_edges,
        )
}

#[test]
fn the_correspondence_the_island_map_and_the_band_are_the_same_built_in_parallel_or_in_order() {
    let g = atlas(24);
    assert!(
        g.triangle_count() > 1024,
        "位相の対応の作りが並列に分かれる大きさ"
    );
    for material in [Some(0), None] {
        let (parallel, in_order) = (
            Parts::new(&g, material, true),
            Parts::new(&g, material, false),
        );
        assert_eq!(parallel, in_order, "位相の対応 {material:?}");
        assert!(parallel.seams.len() > 1000 && parallel.island_count > 500);
        for size in [96u32, 256] {
            let a =
                build_island_map(&g, material, &parallel, size, size, u64::MAX / 4, true).unwrap();
            let b =
                build_island_map(&g, material, &in_order, size, size, u64::MAX / 4, false).unwrap();
            assert_eq!(a, b, "アイランドの図 {size}");
        }
    }
}

#[test]
fn the_band_and_its_refusals_are_the_same_built_in_parallel_or_in_order() {
    let g = atlas(16);
    let topology = UvTopology::new(g.clone(), Some(0));
    let size = 192;
    let islands = Arc::new(
        build_island_map(
            &g,
            Some(0),
            topology.parts(),
            size,
            size,
            u64::MAX / 4,
            false,
        )
        .unwrap(),
    );
    let mut built = 0;
    let mut refused = 0;
    // 予算を絞っていくと、どこかで断る。断る予算も、断る理由も、並列でも順にでも同じ
    for band in [8u32, 32] {
        for budget in [
            4u64 << 10,
            64 << 10,
            256 << 10,
            1 << 20,
            4 << 20,
            64 << 20,
            u64::MAX / 4,
        ] {
            let a = seam_band::build(&topology, islands.clone(), band, budget, true);
            let b = seam_band::build(&topology, islands.clone(), band, budget, false);
            match (a, b) {
                (Ok(a), Ok(b)) => {
                    assert!(same_band(&a, &b), "帯 {band}・予算 {budget}");
                    built += 1;
                }
                (Err(a), Err(b)) => {
                    assert_eq!(a, b, "帯 {band}・予算 {budget}");
                    assert!(matches!(a, UvTopologyError::Budget { .. }));
                    refused += 1;
                }
                (a, b) => panic!(
                    "帯 {band}・予算 {budget}: 並列は {:?}、順には {:?}",
                    a.map(|_| ()),
                    b.map(|_| ())
                ),
            }
        }
    }
    assert!(
        built >= 4 && refused >= 2,
        "作れた {built}・断った {refused}"
    );
}

/// rayon のスレッドの外では並列、中では並列にしない。
#[test]
fn only_threads_outside_rayon_build_in_parallel() {
    assert!(may_parallelize());
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();
    assert!(!pool.install(may_parallelize));
    let pool_of_one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    assert!(!pool_of_one.install(|| rayon::join(may_parallelize, may_parallelize).0));
}
