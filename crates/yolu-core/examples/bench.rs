//! 速さの計測（tools/csharp-golden/run.sh bench の C# と同じ中身）。
//!   cargo run --release -p yolu-core --example bench [回数]
//! 4096² の合成（全タイル乱数の 1 層、Normal + Multiply 0.7 の 2 層）と、101 点のストローク（半径 40 を空の層・乱数の画素の上、半径 200 を空の層）。

use std::time::Instant;

use yolu_core::glam::DVec2;
use yolu_core::{BlendMode, BrushSettings, Channel, Document, LayerId, Rgba8, RowOrder, TileCoord};

struct SplitMix(u64);
impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
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

fn fill_random(doc: &mut Document, layer: LayerId, seed: u64) {
    let ts = doc.tile_size() as usize;
    let mut rng = SplitMix(seed);
    let mut bytes = vec![0u8; ts * ts * 4];
    for ty in 0..doc.height().div_ceil(ts as u32) {
        for tx in 0..doc.width().div_ceil(ts as u32) {
            for p in bytes.as_chunks_mut::<4>().0 {
                let (r, g, b, a) = (rng.channel(), rng.channel(), rng.channel(), rng.alpha());
                p.copy_from_slice(&[r, g, b, a]);
            }
            doc.import_tile(layer, Channel::Color, TileCoord::new(tx, ty), &bytes)
                .unwrap();
        }
    }
    doc.clear_history().unwrap();
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

fn time_composite(doc: &Document, runs: usize) -> String {
    for _ in 0..2 {
        std::hint::black_box(doc.composite(doc.bounds()).unwrap());
    }
    let mut ms: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(doc.composite(doc.bounds()).unwrap());
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    stats(&mut ms)
}

fn main() {
    let runs: usize = std::env::args().nth(1).map_or(7, |s| s.parse().unwrap());
    println!(
        "Rust / 論理プロセッサ {} / rayon のスレッド {}",
        std::thread::available_parallelism().map_or(0, |n| n.get()),
        rayon::current_num_threads()
    );
    {
        let mut doc = Document::new(4096, 4096).unwrap();
        let a = doc.add_layer("a").unwrap();
        fill_random(&mut doc, a, 1);
        println!(
            "合成 4096² 1 層（全タイル乱数）: {}",
            time_composite(&doc, runs)
        );
        let mut buf = vec![0u8; 4096 * 4096 * 4];
        let mut ms: Vec<f64> = (0..runs + 2)
            .map(|_| {
                let t = Instant::now();
                doc.composite_into(Channel::Color, doc.bounds(), &mut buf, RowOrder::TopDown)
                    .unwrap();
                t.elapsed().as_secs_f64() * 1000.0
            })
            .skip(2)
            .collect();
        println!(
            "合成 4096² 1 層（composite_into・使い回しの領域・TopDown）: {}",
            stats(&mut ms)
        );
        let b = doc.add_layer("b").unwrap();
        fill_random(&mut doc, b, 2);
        doc.set_layer_blend_mode(b, BlendMode::Multiply).unwrap();
        doc.set_layer_opacity(b, 0.7, false).unwrap();
        println!(
            "合成 4096² 2 層（Normal + Multiply 0.7）: {}",
            time_composite(&doc, runs)
        );
    }
    for (radius, over) in [(40.0, false), (40.0, true), (200.0, false)] {
        let mut ms = Vec::new();
        let mut stamps = 0;
        for i in 0..runs + 2 {
            let mut doc = Document::new(4096, 4096).unwrap();
            let l = doc.add_layer("a").unwrap();
            if over {
                fill_random(&mut doc, l, 3);
            }
            let brush = BrushSettings {
                radius,
                hardness: 0.8,
                spacing: 0.15,
                color: Rgba8::new(200, 60, 30, 255),
                ..BrushSettings::default()
            };
            let t = Instant::now();
            let mut s = doc.begin_stroke(l, &brush).unwrap();
            for k in 0..=100 {
                let k = k as f64;
                let p = 0.5 + 0.5 * (k % 10.0) / 9.0;
                s.add_point(
                    &mut doc,
                    200.0 + 36.0 * k,
                    2048.0 + 600.0 * (k * 0.1).sin(),
                    p,
                    DVec2::ZERO,
                )
                .unwrap();
            }
            let r = doc.end_stroke(s).unwrap();
            if i >= 2 {
                ms.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            stamps = r.stamps;
        }
        println!(
            "ストローク 半径 {radius}・101 点・{stamps} ダブ（{}）: {}",
            if over {
                "乱数の画素の上"
            } else {
                "空の層"
            },
            stats(&mut ms)
        );
    }
}
