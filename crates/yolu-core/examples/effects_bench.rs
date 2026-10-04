//! 効果（フィルター・Generator・Anchor）を持つ文書の合成の速さ。4096² の人工の文書で、最初の合成・同じものの 2 回目（キャッシュ）・
//! 1 タイルを描いたあと・Anchor を読む層の最初の合成を測る。`cargo run --release -p yolu-core --example effects_bench`
use std::time::Instant;
use yolu_core::generator::{self, Settings};
use yolu_core::{
    AnchorPlacement, BrushSettings, Channel, Document, EffectSettings, FilterSpec, FilterTarget,
    Rect, Rgba8,
};

fn main() {
    let threads: usize = std::env::var("EFFECT_THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(4);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    pool.install(run);
}

fn time<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let v = f();
    println!("{label}: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    v
}

fn run() {
    let n = 4096u32;
    let mut doc = Document::new(n, n).unwrap();
    doc.set_source_budget_bytes(1 << 30).unwrap();
    let base = doc.add_layer("下").unwrap();
    // 全面を描く（タイルごとに 1 色の縞）
    let tile = doc.tile_size();
    let mut bytes = vec![0u8; (tile * tile * 4) as usize];
    for ty in 0..n / tile {
        for tx in 0..n / tile {
            for (i, p) in bytes.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (x, y) = (i as u32 % tile, i as u32 / tile);
                p.copy_from_slice(&[
                    (x * 3 + tx * 11) as u8,
                    (y * 5 + ty * 7) as u8,
                    (x + y) as u8,
                    255,
                ]);
            }
            doc.import_tile(
                base,
                Channel::Color,
                yolu_core::TileCoord::new(tx, ty),
                &bytes,
            )
            .unwrap();
        }
    }
    let rect = Rect::new(0, 0, n, n);
    let plain = time("効果なし", || doc.composite(rect).unwrap());
    doc.add_filter(
        base,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(8)).channels(&[Channel::Color]),
    )
    .unwrap();
    doc.add_filter(
        base,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::noise(0.3, 1, true)).channels(&[Channel::Color]),
    )
    .unwrap();
    doc.add_filter(
        base,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::levels(0.1, 0.9, 1.4, 0.0, 1.0))
            .channels(&[Channel::Color]),
    )
    .unwrap();
    let first = time(
        "ぼかし 8・ノイズ・レベル補正: 最初の合成",
        || doc.composite(rect).unwrap(),
    );
    let second = time("同じものの 2 回目（キャッシュ）", || {
        doc.composite(rect).unwrap()
    });
    assert_eq!(first, second);
    assert_ne!(first, plain);
    let brush = BrushSettings {
        radius: 6.0,
        color: Rgba8::new(255, 0, 0, 255),
        ..Default::default()
    };
    let mut s = doc.begin_stroke_in(base, Channel::Color, &brush).unwrap();
    s.add_point(&mut doc, 2000.0, 2000.0, 1.0, yolu_core::glam::DVec2::ZERO)
        .unwrap();
    s.add_point(&mut doc, 2010.0, 2005.0, 1.0, yolu_core::glam::DVec2::ZERO)
        .unwrap();
    doc.end_stroke(s).unwrap();
    let since = doc.change_serial();
    let _ = since;
    time(
        "1 筆のあとの全面の合成（変わったブロックだけ評価）",
        || doc.composite(rect).unwrap(),
    );
    println!(
        "評価したブロック: {}",
        doc.effect_counters().blocks_evaluated
    );

    // Anchor: 下の層の Color の Anchor を、上の層の Height（出力は別のチャンネル）が読む
    let a = doc
        .add_anchor(base, AnchorPlacement::Layer, None, None)
        .unwrap();
    let top = doc.add_layer("上").unwrap();
    doc.set_channel_enabled(top, Channel::Height, true).unwrap();
    let mut g = Settings::new(generator::Kind::Anchor);
    g.blend = generator::Blend::Replace;
    let stage = doc
        .add_filter(
            top,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Height]),
        )
        .unwrap();
    doc.set_generator_anchor(
        top,
        stage,
        Some(a),
        Channel::Color,
        generator::anchor::ReadMode::Value,
        false,
    )
    .unwrap();
    let paint_top = Rgba8::new(255, 255, 255, 255);
    for ty in 0..n / tile {
        for tx in 0..n / tile {
            let mut b = vec![0u8; (tile * tile * 4) as usize];
            for p in b.as_chunks_mut::<4>().0 {
                p.copy_from_slice(&paint_top.to_array());
            }
            doc.import_tile(top, Channel::Height, yolu_core::TileCoord::new(tx, ty), &b)
                .unwrap();
        }
    }
    time(
        "Anchor を読む層（Height）の最初の合成（Anchor のタイルを合成して読む）",
        || doc.composite_channel(Channel::Height, rect).unwrap(),
    );
    time("2 回目", || {
        doc.composite_channel(Channel::Height, rect).unwrap()
    });
    println!(
        "Anchor のタイルを合成した数: {}",
        doc.effect_counters().anchor_tiles_composited
    );
}
