//! 書き出しの速さの計測（tools/csharp-golden/run.sh export-bench の C# と同じ中身）。
//!   cargo run --release -p yolu-core --example export_bench [回数]
//! 4096² で、全チャンネルに全タイル乱数のレイヤーを 1 つずつ持つ文書（Height → Normal 有効）の各テンプレートの画像の Build と、乱数の三角形
//! 20000 個の覆い、塗り広げ（16 テクセル・全部）、大きなアイランド（2 三角形）の覆いと外の塗り広げ。`RAYON_NUM_THREADS=1` で 1 スレッドの時間。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::time::Instant;

use yolu_core::export::{build, ExportImageKind, ExportScalar, ExportTemplate};
use yolu_core::glam::DVec2;
use yolu_core::padding::{coverage, dilate, Reach};
use yolu_core::{
    Channel, Document, HeightEdgeMode, LayerId, NormalSettings, NormalYDirection, TileCoord,
};

struct SplitMix(u64);
impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn u01(&mut self) -> f64 {
        (self.next() >> 11) as f64 * (1.0 / 9007199254740992.0)
    }
    fn channel(&mut self) -> u8 {
        let r = self.next();
        match r & 7 {
            0 => 0,
            1 => 255,
            _ => (r >> 8) as u8,
        }
    }
    fn alpha(&mut self) -> u8 {
        let r = self.next();
        match r & 7 {
            0 | 1 => 0,
            2 | 3 => 255,
            _ => (r >> 8) as u8,
        }
    }
}

/// 全タイルを乱数で埋める。
fn fill_random(doc: &mut Document, layer: LayerId, channel: Channel, seed: u64) {
    doc.set_channel_enabled(layer, channel, true).unwrap();
    let ts = doc.tile_size() as usize;
    let mut rng = SplitMix(seed);
    let mut bytes = vec![0u8; ts * ts * 4];
    for ty in 0..doc.height().div_ceil(ts as u32) {
        for tx in 0..doc.width().div_ceil(ts as u32) {
            for p in bytes.chunks_exact_mut(4) {
                let (r, g, b, a) = (rng.channel(), rng.channel(), rng.channel(), rng.alpha());
                p.copy_from_slice(&[r, g, b, a]);
            }
            doc.import_tile(layer, channel, TileCoord::new(tx, ty), &bytes)
                .unwrap();
        }
    }
}

fn stats(ms: &mut [f64]) -> String {
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    format!(
        "最小 {:.2} ms / 中央 {:.2} ms（{} 回）",
        ms[0],
        ms[ms.len() / 2],
        ms.len()
    )
}

fn time<T>(runs: usize, mut f: impl FnMut() -> T) -> String {
    std::hint::black_box(f());
    let mut ms: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(f());
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    stats(&mut ms)
}

fn main() {
    let runs: usize = std::env::args().nth(1).map_or(5, |s| s.parse().unwrap());
    println!(
        "Rust / 論理プロセッサ {} / rayon のスレッド {} / 回数 {runs}",
        std::thread::available_parallelism().map_or(0, |n| n.get()),
        rayon::current_num_threads()
    );
    const SIZE: usize = 4096;
    let mut doc = Document::with_tile_size(SIZE as u32, SIZE as u32, 128).unwrap();
    doc.set_source_budget_bytes(4 << 30).unwrap();
    let layer = doc.add_layer("a").unwrap();
    fill_random(&mut doc, layer, Channel::Color, 1);
    for (seed, channel) in [
        Channel::Roughness,
        Channel::Metallic,
        Channel::Height,
        Channel::Emission,
    ]
    .into_iter()
    .enumerate()
    {
        fill_random(&mut doc, layer, channel, seed as u64 + 2);
    }
    doc.set_normal_settings(
        NormalSettings::new(true, 4.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL).unwrap(),
        false,
    )
    .unwrap();
    doc.clear_history().unwrap();
    println!("Build 4096²（全チャンネルが全タイル乱数の 1 レイヤー、Height → Normal 有効）:");
    for template in ExportTemplate::built_in() {
        for image in &template.images {
            let ao_only = image.kind() == ExportImageKind::Packed
                && image.scalars().contains(&ExportScalar::Occlusion)
                && image
                    .scalars()
                    .iter()
                    .all(|s| matches!(s, ExportScalar::Occlusion | ExportScalar::One));
            if ao_only {
                continue; // AO だけの画像は詰める作業が無い
            }
            let occlusion = image
                .scalars()
                .contains(&ExportScalar::Occlusion)
                .then(|| vec![0u8; SIZE * SIZE]);
            println!(
                "  {}/{}: {}",
                template.id,
                image.suffix(),
                time(runs, || build(&doc, image, occlusion.as_deref(), u64::MAX)
                    .unwrap())
            );
        }
    }
    // パディング: 乱数の三角形（画像の中に大きさ 60 まで）で覆い、半径の違う塗り広げ
    let mut rng = SplitMix(7);
    let triangles: Vec<[DVec2; 3]> = (0..20000)
        .map(|_| {
            let (x0, y0) = (rng.u01() * SIZE as f64, rng.u01() * SIZE as f64);
            let mut p = |c: f64| c + (rng.u01() * 2.0 - 1.0) * 60.0;
            let (x1, y1, x2, y2) = (p(x0), p(y0), p(x0), p(y0));
            [DVec2::new(x0, y0), DVec2::new(x1, y1), DVec2::new(x2, y2)]
        })
        .collect();
    let s = SIZE as u32;
    println!(
        "覆い 4096²（乱数の三角形 20000 個、大きさ 60 まで）: {}",
        time(runs, || coverage(s, s, triangles.iter().copied()).unwrap())
    );
    let keep = coverage(s, s, triangles.iter().copied()).unwrap();
    let kept = keep.iter().filter(|&&k| k).count();
    println!(
        "  覆われたテクセル {kept}（{:.1}%）",
        100.0 * kept as f64 / (SIZE * SIZE) as f64
    );
    let mut prng = SplitMix(9);
    let mut pixels = vec![0u8; SIZE * SIZE * 4];
    for p in pixels.chunks_exact_mut(4) {
        p.copy_from_slice(&[prng.channel(), prng.channel(), prng.channel(), prng.alpha()]);
    }
    for reach in [Reach::Texels(16), Reach::Fill] {
        let label = match reach {
            Reach::Fill => "全部（既定）".to_string(),
            Reach::Texels(t) => format!("{t} テクセル"),
        };
        println!(
            "塗り広げ 4096² {label}: {}",
            time(runs, || dilate(&pixels, s, s, &keep, reach, u64::MAX)
                .unwrap())
        );
    }
    // 大きなアイランド（画像の 7 割を覆う 2 つの三角形）での覆い
    let island = [
        [
            DVec2::new(200.0, 200.0),
            DVec2::new(3800.0, 200.0),
            DVec2::new(3800.0, 3800.0),
        ],
        [
            DVec2::new(200.0, 200.0),
            DVec2::new(3800.0, 3800.0),
            DVec2::new(200.0, 3800.0),
        ],
    ];
    println!(
        "覆い 4096²（大きなアイランド 2 三角形）: {}",
        time(runs, || coverage(s, s, island.iter().copied()).unwrap())
    );
    let keep_island = coverage(s, s, island.iter().copied()).unwrap();
    println!(
        "塗り広げ 4096² 全部（大きなアイランドの外を埋める）: {}",
        time(runs, || dilate(
            &pixels,
            s,
            s,
            &keep_island,
            Reach::Fill,
            u64::MAX
        )
        .unwrap())
    );
}
