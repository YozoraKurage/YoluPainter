//! 速さの計測（tools/csharp-golden/run.sh bench の C# と同じ中身）。
//!   cargo run --release -p yolu-core --example bench [回数] [M2 の種類だけ: round|jitter|tip|texture|dual|color|all|blur|smudge]
//! 4096² の合成（全タイル乱数の 1 レイヤー、Normal + Multiply 0.7 の 2 レイヤー、グループ・マスク・調整・塗りつぶしの文書、Normal の合成と
//! Height → Normal の出力）と、101 点のストローク（半径 40 を空のレイヤー・乱数の画素の上、半径 200 を空のレイヤー）。
//! M2 のブラシ: 半径 40・200 で、ゆらぎ・筆先の画像・紙の質感・デュアル・ダブごとの色・全部・ぼかし・指先（効果は乱数の画素の上）。

use std::time::Instant;

use yolu_core::glam::DVec2;
use yolu_core::{
    builtin_tip, AdjustmentSettings, BlendMode, Brush, BrushEffect, BrushSettings, Channel,
    Document, DualBrush, HeightEdgeMode, LayerId, NormalSettings, NormalYDirection, PaperTexture,
    Rgba8, RowOrder, TileCoord,
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
    fill_random_in(doc, layer, Some(Channel::Color), seed)
}

/// 全タイルを乱数で（channel が None ならマスクへ、アルファだけ）。
fn fill_random_in(doc: &mut Document, layer: LayerId, channel: Option<Channel>, seed: u64) {
    let ts = doc.tile_size() as usize;
    let mut rng = SplitMix(seed);
    let mut bytes = vec![0u8; ts * ts * 4];
    for ty in 0..doc.height().div_ceil(ts as u32) {
        for tx in 0..doc.width().div_ceil(ts as u32) {
            for p in bytes.as_chunks_mut::<4>().0 {
                let (r, g, b, a) = (rng.channel(), rng.channel(), rng.channel(), rng.alpha());
                if channel.is_some() {
                    p.copy_from_slice(&[r, g, b, a]);
                } else {
                    p.copy_from_slice(&[0, 0, 0, a]);
                }
            }
            let coord = TileCoord::new(tx, ty);
            match channel {
                Some(c) => doc.import_tile(layer, c, coord, &bytes),
                None => doc.import_mask_tile(layer, coord, &bytes),
            }
            .unwrap();
        }
    }
    doc.clear_history().unwrap();
}

fn time_channel(doc: &Document, channel: Channel, runs: usize) -> String {
    let mut ms: Vec<f64> = (0..runs + 2)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(doc.composite_channel(channel, doc.bounds()).unwrap());
            t.elapsed().as_secs_f64() * 1000.0
        })
        .skip(2)
        .collect();
    stats(&mut ms)
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

/// M2 のブラシの計測の設定（tools/csharp-golden/Golden.cs の bench と同じ）。
fn dynamic_brush(kind: &str, radius: f64) -> Brush {
    let mut b = Brush::from(BrushSettings {
        radius,
        hardness: 0.8,
        spacing: 0.15,
        color: Rgba8::new(200, 60, 30, 255),
        ..BrushSettings::default()
    });
    let jitter = |b: &mut Brush| {
        b.seed = 7;
        b.jitter.size = 0.3;
        b.jitter.angle = 0.5;
        b.jitter.roundness = 0.3;
        b.jitter.scatter = 0.3;
        b.jitter.opacity = 0.2;
        b.jitter.flow = 0.2;
    };
    let tip = |b: &mut Brush| {
        b.tip.image = builtin_tip("charcoal");
        b.tip.follow_direction = true;
    };
    let texture = |b: &mut Brush| {
        b.texture = Some(PaperTexture {
            scale: 2.0,
            ..PaperTexture::new(builtin_tip("grain").unwrap(), 0.6)
        })
    };
    let dual = |b: &mut Brush| {
        b.dual = Some(DualBrush {
            radius: radius / 4.0,
            spacing: 0.25,
            scatter: 0.5,
            count: 2,
            hardness: 0.5,
            ..DualBrush::default()
        })
    };
    let color = |b: &mut Brush| {
        b.seed = 3;
        b.color.hue = 0.3;
        b.color.brightness = 0.2;
    };
    match kind {
        "jitter" => jitter(&mut b),
        "tip" => tip(&mut b),
        "texture" => texture(&mut b),
        "dual" => dual(&mut b),
        "color" => color(&mut b),
        "all" => {
            jitter(&mut b);
            tip(&mut b);
            texture(&mut b);
            dual(&mut b);
            color(&mut b);
        }
        "blur" => b.effect = BrushEffect::BLUR,
        "smudge" => b.effect = BrushEffect::SMUDGE,
        _ => {}
    }
    b
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
            "合成 4096² 1 レイヤー（全タイル乱数）: {}",
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
            "合成 4096² 1 レイヤー（composite_into・使い回しの領域・TopDown）: {}",
            stats(&mut ms)
        );
        let b = doc.add_layer("b").unwrap();
        fill_random(&mut doc, b, 2);
        doc.set_layer_blend_mode(b, BlendMode::Multiply).unwrap();
        doc.set_layer_opacity(b, 0.7, false).unwrap();
        println!(
            "合成 4096² 2 レイヤー（Normal + Multiply 0.7）: {}",
            time_composite(&doc, runs)
        );
        // M2: 通過のグループ（不透明度 0.6 でフェード）の中に Multiply とマスク付きの Screen、分離のグループ（Overlay）、
        // レベル補正、塗りつぶし（Multiply 0.25）
        doc.set_source_budget_bytes(2 << 30).unwrap();
        let c = doc.add_layer("c").unwrap();
        fill_random(&mut doc, c, 3);
        doc.set_layer_blend_mode(c, BlendMode::Screen).unwrap();
        doc.set_layer_opacity(c, 0.8, false).unwrap();
        doc.add_layer_mask(c).unwrap();
        fill_random_in(&mut doc, c, None, 4);
        let g = doc.group_layers(&[b, c], "G").unwrap();
        doc.set_layer_opacity(g, 0.6, false).unwrap();
        let d = doc.add_layer("d").unwrap();
        fill_random(&mut doc, d, 5);
        let inner = doc.group_layers(&[d], "Inner").unwrap();
        doc.set_layer_blend_mode(inner, BlendMode::Overlay).unwrap();
        let lv = doc
            .add_adjustment_layer(
                "lv",
                AdjustmentSettings::levels(0.1, 0.9, 1.4, 0.0, 1.0).unwrap(),
                None,
                None,
            )
            .unwrap();
        doc.set_layer_opacity(lv, 0.8, false).unwrap();
        let f = doc
            .add_fill_layer("f", &[(Channel::Color, Rgba8::new(30, 90, 200, 255))], None)
            .unwrap();
        doc.set_layer_blend_mode(f, BlendMode::Multiply).unwrap();
        doc.set_layer_opacity(f, 0.25, false).unwrap();
        println!(
            "合成 4096² グループの文書（通過 0.6 に Multiply・マスク付き Screen、分離の Overlay、レベル補正、塗りつぶし）: {}",
            time_composite(&doc, runs)
        );
    }
    {
        // M2: Normal の 2 レイヤー（Normal + Overlay 0.6）と Height 1 レイヤー
        let mut doc = Document::new(4096, 4096).unwrap();
        doc.set_source_budget_bytes(2 << 30).unwrap();
        let a = doc.add_layer("a").unwrap();
        fill_random_in(&mut doc, a, Some(Channel::Normal), 6);
        let b = doc.add_layer("b").unwrap();
        fill_random_in(&mut doc, b, Some(Channel::Normal), 7);
        doc.set_layer_blend_mode(b, BlendMode::Overlay).unwrap();
        doc.set_layer_opacity(b, 0.6, false).unwrap();
        let h = doc.add_layer("h").unwrap();
        fill_random_in(&mut doc, h, Some(Channel::Height), 8);
        println!(
            "合成 4096² Normal 2 レイヤー（Normal + Overlay 0.6、ベクトル）: {}",
            time_channel(&doc, Channel::Normal, runs)
        );
        doc.set_normal_settings(
            NormalSettings::new(true, 4.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL)
                .unwrap(),
            false,
        )
        .unwrap();
        let mut ms: Vec<f64> = (0..runs + 2)
            .map(|_| {
                let t = Instant::now();
                std::hint::black_box(doc.normal_output(1 << 30).unwrap());
                t.elapsed().as_secs_f64() * 1000.0
            })
            .skip(2)
            .collect();
        println!(
            "Normal の出力 4096²（上の 2 レイヤー + Height → Normal）: {}",
            stats(&mut ms)
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
                "空のレイヤー"
            },
            stats(&mut ms)
        );
    }
    let only: Option<String> = std::env::args().nth(2);
    for radius in [40.0, 200.0] {
        for kind in [
            "round", "jitter", "tip", "texture", "dual", "color", "all", "blur", "smudge",
        ] {
            if only.as_deref().is_some_and(|o| o != kind) {
                continue;
            }
            let over = kind == "blur" || kind == "smudge";
            let brush = dynamic_brush(kind, radius);
            let mut ms = Vec::new();
            let (mut stamps, mut parallel) = (0, 0);
            for i in 0..runs + 2 {
                let mut doc = Document::new(4096, 4096).unwrap();
                let l = doc.add_layer("a").unwrap();
                if over {
                    fill_random(&mut doc, l, 3);
                }
                let t = Instant::now();
                let mut s = doc.begin_brush_stroke(l, &brush).unwrap();
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
                parallel = doc.active_stroke_stats().unwrap().parallel_dabs;
                let r = doc.end_stroke(s).unwrap();
                if i >= 2 {
                    ms.push(t.elapsed().as_secs_f64() * 1000.0);
                }
                stamps = r.stamps;
            }
            println!(
                "M2 {kind} 半径 {radius}・{stamps} ダブ（ワーカー {parallel}）{}: {}",
                if over { "・乱数の画素の上" } else { "" },
                stats(&mut ms)
            );
        }
    }
}
