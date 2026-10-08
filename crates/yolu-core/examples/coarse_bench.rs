//! 歩幅つきの粗い合成（`Document::composite_coarse_tiles`。ドラッグの間の仮の絵）の速さ。
//!   cargo run --release -p yolu-core --example coarse_bench [回数]
//! 辺 × レイヤー数 × 歩幅を、場面（plain: 合成モードと不透明度の混ざり／rich: マスク・クリッピング・通過と独立のグループ・調整つき）ごとに測り、
//! 全タイルを束（既定は 64 枚）ずつ合成する 1 回の時間（ミリ秒。最小と中央値）を
//! `場面<TAB>辺<TAB>レイヤー<TAB>歩幅<TAB>スレッド<TAB>最小<TAB>中央値` で出す。スレッドは 1（1 コアあたり）と 既定（rayon の本数）。
//! 組は環境変数で替える（カンマ区切り）: COARSE_SCENE（既定 plain,rich）・COARSE_SIZE（既定 2048,4096）・COARSE_LAYERS（既定 1,8,32）・
//! COARSE_STRIDE（既定 2,4,8。1 は粗くしない比べで、歩幅はタイルの一辺 128 の約数）。束の枚数は COARSE_CHUNK。
//! 道の切り替え: 環境変数 YOLU_SIMD（その CPU にある道）。

use std::hint::black_box;
use std::time::Instant;

use yolu_core::{AdjustmentSettings, BlendMode, Channel, Document, LayerId, TileCoord};

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
    fn alpha(&mut self, opaque: bool) -> u8 {
        let r = self.next();
        if opaque {
            return 255;
        }
        match r & 7 {
            0 | 1 => 0,
            2 | 3 => 255,
            _ => (r >> 8) as u8,
        }
    }
}

fn fill_random(doc: &mut Document, layer: LayerId, rng: &mut SplitMix, opaque: bool, mask: bool) {
    let ts = doc.tile_size() as usize;
    let mut bytes = vec![0u8; ts * ts * 4];
    for ty in 0..doc.height().div_ceil(ts as u32) {
        for tx in 0..doc.width().div_ceil(ts as u32) {
            for p in bytes.as_chunks_mut::<4>().0 {
                if mask {
                    p.copy_from_slice(&[0, 0, 0, rng.alpha(false)]);
                } else {
                    let (r, g, b) = (rng.channel(), rng.channel(), rng.channel());
                    p.copy_from_slice(&[r, g, b, rng.alpha(opaque)]);
                }
            }
            let c = TileCoord::new(tx, ty);
            if mask {
                doc.import_mask_tile(layer, c, &bytes).unwrap();
            } else {
                doc.import_tile(layer, Channel::Color, c, &bytes).unwrap();
            }
        }
    }
}

/// 合成モードと不透明度が混ざった `layers` 枚のレイヤー。`rich` ならマスク・クリッピング・調整・グループも混ぜる。
fn build(size: u32, layers: usize, rich: bool) -> Document {
    let mut doc = Document::new(size, size).unwrap();
    doc.set_source_budget_bytes(4 << 30).unwrap();
    let mut rng = SplitMix(7);
    let modes = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::ColorDodge,
        BlendMode::Hue,
        BlendMode::Luminosity,
    ];
    let opacities = [1.0, 0.7, 0.35, 0.999];
    let mut ids = Vec::new();
    for i in 0..layers {
        let l = doc.add_layer(&format!("l{i}")).unwrap();
        fill_random(&mut doc, l, &mut rng, i == 0, false);
        if i > 0 {
            doc.set_layer_blend_mode(l, modes[i % modes.len()]).unwrap();
            doc.set_layer_opacity(l, opacities[i % opacities.len()], false)
                .unwrap();
        }
        if rich && i > 0 {
            if i % 3 == 1 {
                doc.add_layer_mask(l).unwrap();
                fill_random(&mut doc, l, &mut rng, false, true);
            }
            if i % 4 == 2 {
                doc.set_layer_clipping(l, true).unwrap();
            }
        }
        ids.push(l);
    }
    if rich && layers >= 8 {
        let a = doc
            .add_adjustment_layer(
                "levels",
                AdjustmentSettings::levels(0.1, 0.9, 1.4, 0.05, 0.95).unwrap(),
                None,
                None,
            )
            .unwrap();
        doc.set_layer_opacity(a, 0.8, false).unwrap();
        let n = ids.len();
        // 通過のグループ（不透明度 0.6 でフェード）と独立のグループ（Normal）
        let g1 = doc.group_layers(&ids[n - 3..n], "G1").unwrap();
        doc.set_layer_opacity(g1, 0.6, false).unwrap();
        let g2 = doc.group_layers(&ids[n - 7..n - 4], "G2").unwrap();
        doc.set_layer_blend_mode(g2, BlendMode::Normal).unwrap();
        doc.set_layer_opacity(g2, 0.85, false).unwrap();
    }
    doc.clear_history().unwrap();
    doc
}

/// 環境変数 `var`（カンマ区切り）の値。無ければ `default`。
fn list<T: std::str::FromStr + Copy>(var: &str, default: &[T]) -> Vec<T> {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => v.split(',').filter_map(|p| p.trim().parse().ok()).collect(),
        _ => default.to_vec(),
    }
}

fn main() {
    let runs: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(9);
    let chunk: usize = std::env::var("COARSE_CHUNK")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64);
    let scenes: Vec<String> = match std::env::var("COARSE_SCENE") {
        Ok(v) if !v.is_empty() => v.split(',').map(str::to_string).collect(),
        _ => vec!["plain".into(), "rich".into()],
    };
    println!("道 {}", yolu_core::blend::simd_level_name());
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    for scene in &scenes {
        for size in list("COARSE_SIZE", &[2048u32, 4096]) {
            for layers in list("COARSE_LAYERS", &[1usize, 8, 32]) {
                let doc = build(size, layers, scene == "rich");
                let all: Vec<TileCoord> = doc.canvas_tiles().collect();
                for stride in list("COARSE_STRIDE", &[2u32, 4, 8]) {
                    for (threads, pool) in [("1", Some(&one)), ("既定", None)] {
                        let run = || {
                            for tiles in all.chunks(chunk) {
                                black_box(
                                    doc.composite_coarse_tiles(Channel::Color, tiles, stride)
                                        .unwrap(),
                                );
                            }
                        };
                        let mut times: Vec<f64> = Vec::new();
                        // 1 回目は予熱（捨てる）
                        for k in 0..=runs {
                            let t = Instant::now();
                            match pool {
                                Some(p) => p.install(run),
                                None => run(),
                            }
                            if k > 0 {
                                times.push(t.elapsed().as_secs_f64() * 1000.0);
                            }
                        }
                        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
                        println!(
                            "{scene}\t{size}\t{layers}\t{stride}\t{threads}\t{:.2}\t{:.2}",
                            times[0],
                            times[times.len() / 2]
                        );
                    }
                }
            }
        }
    }
}
