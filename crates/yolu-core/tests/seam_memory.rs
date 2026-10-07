//! 継ぎ目をまたぐ評価と、島の図・帯の写しを作る間の、実際の確保（生きているバイト数の最大）が、見積り・予算に収まること。
//!
//! 確保を数える `#[global_allocator]` を置くので、この試験だけで 1 本の実行ファイルにする。ほかの試験の確保が混ざらないよう、試験は 1 つの関数で
//! 順に回す。モデルは「3D で隣どうしの小さな島が、UV では散らばって、島の間の隙間がぜんぶ帯のテクセルになる」並べ方（帯のテクセルが最も多く、
//! 読む相手の島がブロックの外にも出る）。
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::Arc;

use yolu_core::filter::{
    self, block_working_bytes, seam_working_bytes, Image, Options, Settings, Stage, ValueType,
};
use yolu_core::geometry::{
    SurfaceGeometry, SurfaceTriangle, UvTopology, UvTopologyError, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::Rect;

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(n: usize) {
    let now = LIVE.fetch_add(n, SeqCst) + n;
    PEAK.fetch_max(now, SeqCst);
}

// SAFETY: すべて `System` に任せ、生きているバイト数だけを数える。
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), SeqCst);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            if new_size >= layout.size() {
                grew(new_size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - new_size, SeqCst);
            }
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// f の間に生きていたバイト数の最大（始まりの時点からの増え）と、f の結果（まだ生きたまま）。
fn measure<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let base = LIVE.load(SeqCst);
    PEAK.store(base, SeqCst);
    let out = f();
    (out, PEAK.load(SeqCst).saturating_sub(base))
}

const SIZE: u32 = 256;

/// n × n の四角を 3D で隣どうしにつなぎ、UV では並びを散らした n × n の升に、升の半分の大きさで置く（島の間は島と同じ幅）。
fn atlas(n: u32) -> Arc<UvTopology> {
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
    let g = SurfaceGeometry::new(tris, 1, DEFAULT_WELD_TOLERANCE).unwrap();
    Arc::new(UvTopology::new(Arc::new(g), Some(0)))
}

fn picture() -> Vec<u8> {
    let mut image = vec![0u8; (SIZE * SIZE * 4) as usize];
    for (i, p) in image.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let h = (i as u32).wrapping_mul(2_654_435_761);
        *p = [(h >> 8) as u8, (h >> 16) as u8, (h >> 24) as u8, 255];
    }
    image
}

fn pool(threads: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
}

/// 帯の写しを予算 budget で作って、成功したなら、作る間の確保が予算に収まったか。結果と最大の確保。
fn build_within(
    threads: usize,
    n: u32,
    band: u32,
    budget: u64,
) -> (Result<(), UvTopologyError>, usize) {
    let topology = atlas(n);
    // 位相の対応（島・縁）は作る間の確保に数えない（解像度によらず、モデルごとに 1 つ）
    let _ = topology.seam_edge_count();
    let (r, peak) = pool(threads).install(|| {
        measure(|| {
            topology
                .seam_band_within(SIZE, SIZE, band, budget)
                .map(|_| ())
        })
    });
    (r, peak)
}

/// n × n の島（n が大きいほど小さな島が多く、小さいほど大きな島が少ない）と、近傍の段の半径。
const MODELS: [(u32, u32); 2] = [(32, 8), (4, 16)];

#[test]
fn estimates_and_budgets_bound_the_real_allocations() {
    let image = picture();
    let source = Image::new(&image, SIZE, SIZE).unwrap();
    let region = Rect::new(0, 0, SIZE, SIZE);
    let output = u64::from(SIZE) * u64::from(SIZE) * 4;
    for (n, radius) in MODELS {
        let topology = atlas(n);
        let band = topology.seam_band(SIZE, SIZE, radius).unwrap();
        eprintln!(
            "島 {n}×{n}・半径 {radius}: 継ぎ目の縁 {}・帯のテクセル {}",
            topology.seam_edge_count(),
            band.texel_count()
        );
        assert!(
            band.texel_count() > 5_000,
            "帯のテクセルが多い並べ方のはず: {}",
            band.texel_count()
        );

        // ── 評価: 近傍の段 1 つ・2 つ（前の段にも近傍の段があれば、相手の島を読む矩形の中でも継ぎ目をまたぐ）。ブロックの大きさも 2 通り
        let one = [Stage::new(Settings::GaussianBlur { radius })];
        let two = [
            Stage::new(Settings::GaussianBlur { radius }),
            Stage::new(Settings::Sharpen {
                radius: radius / 2,
                amount: 1.0,
                threshold: 0,
            }),
        ];
        for (name, stages) in [("近傍 1 段", &one[..]), ("近傍 2 段", &two[..])] {
            for block in [32u32, 64] {
                let options = |seams| Options {
                    block_size: block,
                    seams,
                    ..Options::default()
                };
                let run = |seams| {
                    pool(1).install(|| {
                        measure(|| {
                            filter::evaluate(
                                &source,
                                ValueType::Color,
                                stages,
                                region,
                                &options(seams),
                            )
                            .unwrap()
                        })
                    })
                };
                let (flat, flat_peak) = run(None);
                let (across, across_peak) = run(Some(&*band));
                assert_ne!(flat, across, "{name}: 継ぎ目をまたいで値が変わるはず");
                let base = block_working_bytes(stages, block, SIZE, SIZE).unwrap();
                let seam = seam_working_bytes(stages, block, SIZE, SIZE, band.texel_count() as u64);
                let seam_worst = seam_working_bytes(stages, block, SIZE, SIZE, u64::MAX);
                eprintln!(
                    "  {name} ブロック {block}: 最大 2D {flat_peak}・またぐ {across_peak}、\
                     見積り 2D {base}・帯 {seam}（最悪 {seam_worst}）・返す画像 {output}"
                );
                // 2D の見積りは今までどおり収まる。またぐ分は、帯の見積りの中に収まる（最悪の見積りはそれ以上）
                assert!(
                    flat_peak as u64 <= base + output,
                    "{name} {block}: 2D の確保 {flat_peak} > 見積り {}",
                    base + output
                );
                assert!(
                    across_peak as u64 <= base + seam + output,
                    "{name} {block}: またぐ確保 {across_peak} > 見積り {}",
                    base + seam + output
                );
                assert!(seam <= seam_worst);
            }
        }

        // ── 帯の写しを作る間: 成功した予算では、確保の最大が予算に収まる。収まらない予算では断る
        for threads in [1usize, 4] {
            for budget in [4u64 << 20, 16 << 20, 64 << 20, 256 << 20] {
                let (r, peak) = build_within(threads, n, radius, budget);
                eprintln!("  帯を作る（{threads} スレッド）予算 {budget}: {r:?} 最大 {peak}");
                match r {
                    Ok(()) => assert!(
                        peak as u64 <= budget,
                        "{threads} スレッド 予算 {budget}: 確保 {peak}"
                    ),
                    Err(e) => assert!(matches!(e, UvTopologyError::Budget { .. }), "{e}"),
                }
            }
            // 通る一番小さい予算（二分探索。作る間の確保の見積りが実際より小さければ、ここで予算を超える）
            let (mut lo, mut hi) = (0u64, 256u64 << 20);
            assert!(build_within(threads, n, radius, hi).0.is_ok());
            while hi - lo > 64 * 1024 {
                let mid = (lo + hi) / 2;
                if build_within(threads, n, radius, mid).0.is_ok() {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            let (r, peak) = build_within(threads, n, radius, hi);
            eprintln!("  帯を作る（{threads} スレッド）通る一番小さい予算 {hi}: 最大 {peak}");
            assert!(r.is_ok());
            assert!(
                peak as u64 <= hi,
                "{threads} スレッド: 通る最小の予算 {hi} で確保 {peak} が予算を超えた"
            );
        }
    }
}
