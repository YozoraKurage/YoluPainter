//! UV の位相: 島の番号・島の図（重なりの印）・継ぎ目の縁と、帯の写しが読む所（拡大率・向き・鏡映の違い、重なった UV、開いた縁、
//! 展開の届く所）。小さなモデルは試験で組む: 3D で 1 辺を共有する 2 つの四角（A は x 0..1、B は x 1..2）を、UV の別の所に置く。
use std::sync::Arc;

use yolu_core::geometry::{
    seam_band_width, SurfaceGeometry, SurfaceTriangle, UvTopology, UvTopologyError,
    DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Vec2, Vec3};

/// 3D の四角（x0..x0+1, y 0..1, z）の 2 つの三角形。uv は四角の中の (x, y)（0..1）から UV へ。右の辺は 1 つ目、左の辺は 2 つ目の三角形。
fn quad(x0: f32, z: f32, uv: impl Fn(f32, f32) -> Vec2) -> [SurfaceTriangle; 2] {
    let p = |x: f32, y: f32| Vec3::new(x0 + x, y, z);
    [
        SurfaceTriangle::new(
            p(0., 0.),
            p(1., 0.),
            p(1., 1.),
            uv(0., 0.),
            uv(1., 0.),
            uv(1., 1.),
        ),
        SurfaceTriangle::new(
            p(0., 0.),
            p(1., 1.),
            p(0., 1.),
            uv(0., 0.),
            uv(1., 1.),
            uv(0., 1.),
        ),
    ]
}

fn topology(triangles: Vec<SurfaceTriangle>) -> UvTopology {
    let g = SurfaceGeometry::new(triangles, 1, DEFAULT_WELD_TOLERANCE).unwrap();
    UvTopology::new(Arc::new(g), Some(0))
}

/// A は UV の (0.1..0.4, 0.1..0.4)、B は b の写像。
fn two(b: impl Fn(f32, f32) -> Vec2) -> UvTopology {
    let mut t = quad(0., 0., |x, y| Vec2::new(0.1 + 0.3 * x, 0.1 + 0.3 * y)).to_vec();
    t.extend(quad(1., 0., b));
    topology(t)
}

fn straight() -> UvTopology {
    two(|x, y| Vec2::new(0.6 + 0.3 * x, 0.1 + 0.3 * y))
}

/// 読む点の重心（重みつきの平均）。
fn center(taps: &[(u32, u32, f64)]) -> (f64, f64) {
    let x = taps.iter().map(|t| (t.0 as f64 + 0.5) * t.2).sum();
    let y = taps.iter().map(|t| (t.1 as f64 + 0.5) * t.2).sum();
    (x, y)
}

fn near(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 0.02 && (a.1 - b.1).abs() < 0.02
}

#[test]
fn two_islands_and_the_seam_between_them() {
    let t = straight();
    assert_eq!(t.island_count(), 2);
    assert_eq!(t.seam_edge_count(), 2, "A の右の辺と B の左の辺");
    assert_eq!(t.triangle_island(0), 1);
    assert_eq!(t.triangle_island(3), 2);
    let map = t.island_map(64, 64).unwrap();
    assert_eq!(map.island(10, 10), 1);
    assert_eq!(map.island(45, 20), 2);
    assert_eq!(map.island(32, 20), 0, "島の間");
    assert!(!map.overlapped(10, 10));
    // 行の連なり: A の 6..=25 と B の 38..=57
    let row = map.row(16);
    assert_eq!(row.len(), 2);
    assert_eq!((row[0].start, row[0].end, row[0].island), (6, 26, 1));
    assert_eq!((row[1].start, row[1].end, row[1].island), (38, 58, 2));
    // 同じ解像度は覚えている物
    assert!(Arc::ptr_eq(&map, &t.island_map(64, 64).unwrap()));
}

#[test]
fn the_band_reads_the_other_island_across_the_seam() {
    let t = straight();
    let band = t.seam_band(64, 64, 8).unwrap();
    assert!(band.texel_count() > 0);
    // A の右の縁（25.6）の 0.9 外 → B の左の縁（38.4）の 0.9 内
    let taps = band.taps(26, 16).unwrap();
    assert!(near(center(&taps), (39.3, 16.5)), "{taps:?}");
    assert!((taps.iter().map(|t| t.2).sum::<f64>() - 1.0).abs() < 1e-9);
    // B の左の縁の外 → A の右の縁の内
    let taps = band.taps(37, 20).unwrap();
    assert!(near(center(&taps), (24.7, 20.5)), "{taps:?}");
    // 島の中は帯ではない
    assert!(band.taps(20, 16).is_none());
    // 開いた縁（A の左の辺）の外は埋めない
    assert!(band.taps(5, 16).is_none());
    // 同じ大きさ・幅は覚えている物
    assert!(Arc::ptr_eq(&band, &t.seam_band(64, 64, 8).unwrap()));
}

#[test]
fn scale_rotation_and_mirror_are_followed_by_the_unfolding() {
    // 拡大率: B は UV で 1.5 倍（0.45）。A の縁から d の所は、B の縁から 1.5d
    let scaled = two(|x, y| Vec2::new(0.5 + 0.45 * x, 0.1 + 0.45 * y));
    let band = scaled.seam_band(64, 64, 8).unwrap();
    let taps = band.taps(27, 16).unwrap(); // A の縁から 1.9
    assert!(
        near(center(&taps), (32.0 + 1.9 * 1.5, 6.4 + 10.1 * 1.5)),
        "{taps:?}"
    );
    // 向き: B は 90 度回して置く（B の左の辺が UV の上の辺。縁に沿って u が増える）
    let rotated = two(|x, y| Vec2::new(0.6 + 0.3 * y, 0.7 - 0.3 * x));
    let band = rotated.seam_band(64, 64, 8).unwrap();
    let taps = band.taps(27, 16).unwrap();
    // 縁に沿う位置 16.5 − 6.4 = 10.1 → u = 38.4 + 10.1、縁から 1.9 内 → v = 44.8 − 1.9
    assert!(near(center(&taps), (48.5, 42.9)), "{taps:?}");
    // 鏡映: B は左右を返す（B の左の辺が UV の右の縁 57.6）
    let mirrored = two(|x, y| Vec2::new(0.9 - 0.3 * x, 0.1 + 0.3 * y));
    let band = mirrored.seam_band(64, 64, 8).unwrap();
    let taps = band.taps(27, 16).unwrap();
    assert!(near(center(&taps), (57.6 - 1.9, 16.5)), "{taps:?}");
}

#[test]
fn overlapped_islands_with_different_partners_are_left_alone() {
    // A と A' は同じ UV（重なり）。A の相手は B、A' の相手は別の所の C。A の縁の外の帯は、どちらを読むか決まらないので埋めない
    let a = |x: f32, y: f32| Vec2::new(0.1 + 0.3 * x, 0.1 + 0.3 * y);
    let mut t = quad(0., 0., a).to_vec();
    t.extend(quad(1., 0., |x, y| Vec2::new(0.6 + 0.3 * x, 0.1 + 0.3 * y)));
    t.extend(quad(0., 5., a));
    t.extend(quad(1., 5., |x, y| Vec2::new(0.6 + 0.3 * x, 0.6 + 0.3 * y)));
    let topo = topology(t);
    // A と A' は UV の辺を共有するので 1 つの島
    assert_eq!(topo.island_count(), 3);
    let map = topo.island_map(64, 64).unwrap();
    assert!(map.overlapped(12, 18), "重なりの印");
    let band = topo.seam_band(64, 64, 8).unwrap();
    assert!(band.taps(26, 16).is_none());
    assert!(band.stats().conflicts > 0);
    // 相手が同じ所（ミラーで相手の島も重なる）なら埋める
    let b = |x: f32, y: f32| Vec2::new(0.6 + 0.3 * x, 0.1 + 0.3 * y);
    let mut t = quad(0., 0., a).to_vec();
    t.extend(quad(1., 0., b));
    t.extend(quad(0., 5., a));
    t.extend(quad(1., 5., b));
    let band = topology(t).seam_band(64, 64, 8).unwrap();
    let taps = band.taps(26, 16).unwrap();
    assert!(near(center(&taps), (39.3, 16.5)), "{taps:?}");
}

#[test]
fn the_band_stops_where_the_unfolding_ends() {
    // B は UV で細い（3D の幅 1 が 3.2 画素）。A の縁から 19.2 より遠い所は、B を越えた先（開いた縁）なので埋めない
    let mut t = quad(0., 0., |x, y| Vec2::new(0.05 + 0.15 * x, 0.05 + 0.15 * y)).to_vec();
    t.extend(quad(1., 0., |x, y| {
        Vec2::new(0.9 + 0.025 * x, 0.05 + 0.15 * y)
    }));
    let topo = topology(t);
    let band = topo.seam_band(128, 128, 32).unwrap();
    assert!(band.taps(25 + 15, 16).is_some());
    assert!(band.taps(25 + 25, 16).is_none());
}

#[test]
fn band_widths_are_rounded_up_to_steps() {
    assert_eq!(seam_band_width(0), 0);
    assert_eq!(seam_band_width(1), 8);
    assert_eq!(seam_band_width(8), 8);
    assert_eq!(seam_band_width(9), 16);
    assert_eq!(seam_band_width(64), 64);
    assert_eq!(seam_band_width(500), 512);
}

#[test]
fn a_new_model_builds_new_tables() {
    let a = straight();
    let b = straight();
    let ta = a.seam_band(64, 64, 8).unwrap();
    let tb = b.seam_band(64, 64, 8).unwrap();
    assert!(!Arc::ptr_eq(&ta, &tb));
    // 解像度・幅が違えば別の写し
    assert!(!Arc::ptr_eq(&ta, &a.seam_band(64, 64, 16).unwrap()));
    assert!(!Arc::ptr_eq(&ta, &a.seam_band(128, 128, 8).unwrap()));
    assert!(a.cached_bytes() > 0);
}

#[test]
fn a_posed_model_keeps_the_same_layout() {
    let a = straight();
    // 位置だけ動かした（ポーズ）同じ並び・UV は同じ位相
    let mut moved = quad(0., 0., |x, y| Vec2::new(0.1 + 0.3 * x, 0.1 + 0.3 * y)).to_vec();
    moved.extend(quad(1., 0., |x, y| Vec2::new(0.6 + 0.3 * x, 0.1 + 0.3 * y)));
    for t in &mut moved {
        for p in [&mut t.a, &mut t.b, &mut t.c] {
            *p = Vec3::new(p.x * 1.5, p.y + 0.25, p.x * 0.3);
        }
    }
    let posed = SurfaceGeometry::new(moved, 2, DEFAULT_WELD_TOLERANCE).unwrap();
    assert!(a.same_layout(&posed));
    // UV が違えば別の位相
    let other = two(|x, y| Vec2::new(0.6 + 0.3 * x, 0.15 + 0.3 * y));
    assert!(!a.same_layout(other.geometry()));
}

// ───────── 作業予算 ─────────

#[test]
fn an_island_map_over_the_budget_is_refused_and_remembered() {
    let own = straight().island_map(64, 64).unwrap().bytes();
    // 行の入れ物だけで予算を超える
    let t = straight();
    assert_eq!(
        t.island_map_within(64, 64, 100).unwrap_err(),
        UvTopologyError::Budget { budget: 100 }
    );
    assert!(t.cached_seam_band(64, 64, 8).is_none());
    assert_eq!(t.cached_bytes(), 0);
    // 島の図を断ると、その大きさの帯の写しも断ったことになる（調べるだけ。作らない）
    assert_eq!(
        t.refusal(64, 64, 8),
        Some(UvTopologyError::Budget { budget: 100 })
    );
    assert_eq!(t.refusal(128, 128, 8), None, "ほかの大きさは断っていない");
    // 作るには、行ごとの連なりとまとめた写しの 2 つ分が要る: 図そのものの大きさの予算では足りない
    let t = straight();
    assert!(
        matches!(
            t.island_map_within(64, 64, own),
            Err(UvTopologyError::Budget { .. })
        ),
        "図 {own} バイトを、同じ大きさの予算では作れない"
    );
    assert_eq!(t.cached_bytes(), 0, "断ったものは覚えない");
    // 十分な予算なら作れて、断りの記録は消える
    let map = t.island_map_within(64, 64, own * 8).unwrap();
    assert_eq!(map.bytes(), own);
    assert_eq!(t.refusal(64, 64, 8), None);
    assert_eq!(t.cached_bytes(), own);
    // 覚えているものは、小さな予算で頼んでも返す（作らないので確保が増えない）
    assert!(Arc::ptr_eq(&map, &t.island_map_within(64, 64, 1).unwrap()));
}

#[test]
fn a_refusal_is_not_retried_under_the_same_budget_but_under_a_larger_one() {
    let t = straight();
    // 島の図は作れるが、帯の写しの作業は収まらない予算
    let map = straight().island_map(64, 64).unwrap().bytes();
    let tight = map * 8;
    t.island_map_within(64, 64, tight).unwrap();
    let e = t.seam_band_within(64, 64, 8, tight / 4).unwrap_err();
    assert!(
        matches!(e, UvTopologyError::Budget { budget } if budget == tight / 4),
        "{e}"
    );
    assert_eq!(
        t.refusal(64, 64, 8),
        Some(UvTopologyError::Budget { budget: tight / 4 })
    );
    assert_eq!(t.refusal(64, 64, 16), None, "帯の幅が違えば別");
    // 同じか小さい予算では、作り直さずに断る
    assert!(t.seam_band_within(64, 64, 8, tight / 8).is_err());
    assert_eq!(
        t.refusal(64, 64, 8),
        Some(UvTopologyError::Budget { budget: tight / 4 }),
        "断った予算は、大きい方のまま"
    );
    // 大きい予算で作れて、断りの記録は消える
    let band = t.seam_band_within(64, 64, 8, 64 << 20).unwrap();
    assert_eq!(t.refusal(64, 64, 8), None);
    assert!(Arc::ptr_eq(&band, &t.cached_seam_band(64, 64, 8).unwrap()));
    // 小さな予算で頼んでも、覚えているものは返る
    assert!(Arc::ptr_eq(
        &band,
        &t.seam_band_within(64, 64, 8, 1).unwrap()
    ));
}

#[test]
fn the_remembered_tables_stay_inside_the_budget() {
    let t = straight();
    // 大きな島の図を先に作り、次に小さな島の図を、覚えている合計より 1 バイト小さい予算で作る: 古い大きな図が捨てられて収まる
    let big = t.island_map_within(2048, 2048, 64 << 20).unwrap();
    let held = t.cached_bytes();
    let budget = held - 1;
    let small = t
        .island_map_within(256, 256, budget)
        .expect("小さな島の図は、その予算で作れる");
    assert!(
        big.bytes() > small.bytes() * 4,
        "前提: 大きな図 {} は小さな図 {} よりずっと大きい",
        big.bytes(),
        small.bytes()
    );
    assert!(
        t.cached_bytes() <= budget,
        "{} > {budget}",
        t.cached_bytes()
    );
    assert_eq!(
        t.cached_bytes(),
        small.bytes(),
        "古い大きな図を捨て、直近の図は残す"
    );
    // 帯の写しは、帯の写しが持つ島の図と合わせて数える: 予算を小さくして縮めると、収まるまで古いものから（直近のものも）捨てる
    let t = straight();
    let wide = t.seam_band_within(128, 128, 32, 64 << 20).unwrap();
    let narrow = t.seam_band_within(128, 128, 8, 64 << 20).unwrap();
    let map = wide.islands().bytes();
    let held = t.cached_bytes();
    assert_eq!(
        held,
        map + wide.bytes() + narrow.bytes(),
        "島の図は 1 度だけ数える"
    );
    t.shrink(held - 1);
    assert!(t.cached_bytes() < held);
    assert!(
        t.cached_seam_band(128, 128, 32).is_none(),
        "古い帯から捨てる"
    );
    assert!(t.cached_seam_band(128, 128, 8).is_some());
    t.shrink(0);
    assert_eq!(t.cached_bytes(), 0);
    // 捨てても、また作れる
    assert!(t.seam_band(128, 128, 8).is_ok());
}

#[test]
fn many_threads_asking_for_the_same_table_get_the_same_one() {
    let t = Arc::new(straight());
    let got: Vec<_> = std::thread::scope(|scope| {
        (0..8)
            .map(|_| scope.spawn(|| t.seam_band_within(128, 128, 16, 64 << 20).unwrap()))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect()
    });
    for b in &got[1..] {
        assert!(Arc::ptr_eq(&got[0], b));
    }
}
