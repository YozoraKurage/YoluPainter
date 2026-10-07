//! ブラシの 14 の場面の速さと画素（ブラシの計算の数の形を変える前後を、同じ入力で比べる台）。
//!   cargo run --release -p yolu-core --example brush_scenes -- time [回数]
//!   cargo run --release -p yolu-core --example brush_scenes -- dump <フォルダ>
//!   cargo run --release -p yolu-core --example brush_scenes -- compare <フォルダ A> <フォルダ B>
//! 場面は 384² の文書に、同じ所を何十回もなぞるストローク（薄い流量・不透明度、細かい間隔、下地の有無・半透明、筆先の画像、
//! 紙の質感、ダブごとの色、色の混ぜ、指先）。`time` は 1 スレッドの CPU 時間（ストロークの始めから確定まで）の最小の回、
//! `dump` は各場面のレイヤーの画素を `<番号>.rgba` へ書き、`compare` は 2 つのフォルダの画素の差（最大・分布・塗った画素に対する割合）を出す。
#[path = "stroke_support/mod.rs"]
mod stroke_support;

use stroke_support::{clock_ms, use_cpu_time};
use yolu_core::glam::DVec2;
use yolu_core::{
    builtin_tip, Brush, BrushEffect, BrushSettings, Channel, ColorDynamics, ColorMix, Document,
    LayerId, MixMode, PaperTexture, Rgba8, TileCoord,
};

const SIZE: u32 = 384;

/// 場面: 名前・ブラシ・下地を敷くか・下地を半透明にするか・入力の点。
type Scene = (&'static str, Brush, bool, bool, Vec<(f64, f64)>);

struct SplitMix(u64);
impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
}

/// 下地: なめらかな色の勾配に小さな乱れ（混ぜる・指先が拾う色）。アルファは不透明か、半分の画素が半透明。
fn fill(doc: &mut Document, layer: LayerId, translucent: bool) {
    let ts = doc.tile_size();
    let mut rng = SplitMix(5);
    let mut bytes = vec![0u8; (ts * ts * 4) as usize];
    for ty in 0..SIZE.div_ceil(ts) {
        for tx in 0..SIZE.div_ceil(ts) {
            for y in 0..ts {
                for x in 0..ts {
                    let (gx, gy) = (tx * ts + x, ty * ts + y);
                    let n = (rng.next() % 9) as i32 - 4;
                    let c = |v: u32| (v as i32 + n).clamp(0, 255) as u8;
                    let a = if translucent && (gx / 7 + gy / 5) % 2 == 0 {
                        90 + (rng.next() % 100) as u8
                    } else {
                        255
                    };
                    let i = ((y * ts + x) * 4) as usize;
                    bytes[i..i + 4].copy_from_slice(&[
                        c(gx * 255 / SIZE),
                        c(gy * 255 / SIZE),
                        c(128 + (gx + gy) % 64),
                        a,
                    ]);
                }
            }
            doc.import_tile(layer, Channel::Color, TileCoord::new(tx, ty), &bytes)
                .unwrap();
        }
    }
    doc.clear_history().unwrap();
}

/// 往復する線（同じ所を passes 回なぞる）。点は 4 画素ごと。
fn back_and_forth(passes: usize) -> Vec<(f64, f64)> {
    let mut v = Vec::new();
    for p in 0..passes {
        let forward = p % 2 == 0;
        for i in 0..=40 {
            let t = i as f64 / 40.0;
            let t = if forward { t } else { 1.0 - t };
            v.push((120.0 + 160.0 * t, 190.0 + 3.0 * (p as f64 * 0.7).sin()));
        }
    }
    v
}

/// 下地を敷いた文書（ストロークの前の画素も返す）。
fn document(filled: bool, translucent: bool) -> (Document, LayerId) {
    let mut doc = Document::new(SIZE, SIZE).unwrap();
    let layer = doc.add_layer("a").unwrap();
    if filled {
        fill(&mut doc, layer, translucent);
    }
    (doc, layer)
}

/// ストロークを描いて、レイヤーの画素・ダブの数・かかった時間（ミリ秒）を返す。
fn stroke(
    brush: &Brush,
    filled: bool,
    translucent: bool,
    points: &[(f64, f64)],
) -> (Vec<u8>, u64, f64) {
    let (mut doc, layer) = document(filled, translucent);
    let t0 = clock_ms();
    let mut s = doc.begin_brush_stroke(layer, brush).unwrap();
    for (i, &(x, y)) in points.iter().enumerate() {
        let pressure = 0.55 + 0.4 * ((i as f64) * 0.13).sin();
        s.add_point(&mut doc, x, y, pressure, DVec2::ZERO).unwrap();
    }
    let r = doc.end_stroke(s).unwrap();
    let ms = clock_ms() - t0;
    (
        doc.composite_channel(Channel::Color, doc.bounds()).unwrap(),
        r.stamps,
        ms,
    )
}

fn base(radius: f64, hardness: f64, spacing: f64, opacity: f64, flow: f64) -> Brush {
    let mut b = Brush::from(BrushSettings {
        radius,
        hardness,
        spacing,
        opacity,
        flow,
        color: Rgba8::new(200, 60, 30, 255),
        ..BrushSettings::default()
    });
    b.seed = 7;
    b
}

fn scenes() -> Vec<Scene> {
    vec![
        (
            "硬い丸・流量 0.05・往復 20",
            base(20.0, 1.0, 0.05, 1.0, 0.05),
            false,
            false,
            back_and_forth(20),
        ),
        (
            "柔らかい丸・流量 0.02・不透明度 0.3・往復 20",
            base(20.0, 0.0, 0.05, 0.3, 0.02),
            false,
            false,
            back_and_forth(20),
        ),
        (
            "硬い丸・流量 0.005・間隔 0.01・往復 40",
            base(20.0, 1.0, 0.01, 1.0, 0.005),
            false,
            false,
            back_and_forth(40),
        ),
        (
            "柔らかい丸・不透明度 0.02・流量 0.01・間隔 0.01・往復 40・下地あり",
            base(20.0, 0.0, 0.01, 0.02, 0.01),
            true,
            false,
            back_and_forth(40),
        ),
        (
            "柔らかい丸・不透明度 0.05・流量 1・往復 10",
            base(24.0, 0.0, 0.1, 0.05, 1.0),
            false,
            false,
            back_and_forth(10),
        ),
        (
            "柔らかい丸・流量 0.01・往復 40・下地あり",
            base(16.0, 0.0, 0.03, 0.6, 0.01),
            true,
            false,
            back_and_forth(40),
        ),
        (
            "柔らかい丸・流量 0.05・往復 20・半透明の下地",
            base(16.0, 0.0, 0.05, 0.8, 0.05),
            true,
            true,
            back_and_forth(20),
        ),
        (
            "筆先の画像（木炭）・流量 0.1・往復 20",
            {
                let mut b = base(20.0, 0.8, 0.05, 0.8, 0.1);
                b.tip.image = builtin_tip("charcoal");
                b.tip.follow_direction = true;
                b
            },
            false,
            false,
            back_and_forth(20),
        ),
        (
            "紙の質感・流量 0.1・往復 20",
            {
                let mut b = base(20.0, 0.8, 0.05, 0.8, 0.1);
                b.texture = Some(PaperTexture {
                    scale: 2.0,
                    ..PaperTexture::new(builtin_tip("grain").unwrap(), 0.6)
                });
                b
            },
            false,
            false,
            back_and_forth(20),
        ),
        (
            "ダブごとの色（色相のゆらぎ）・流量 0.2・往復 20",
            {
                let mut b = base(18.0, 0.5, 0.05, 1.0, 0.2);
                b.color = ColorDynamics {
                    hue: 0.3,
                    brightness: 0.2,
                    per_tip: true,
                    ..ColorDynamics::default()
                };
                b
            },
            false,
            false,
            back_and_forth(20),
        ),
        (
            "色を混ぜる（混ぜる）・往復 20",
            {
                let mut b = base(18.0, 0.8, 0.05, 1.0, 1.0);
                b.mix = ColorMix {
                    mode: MixMode::Mix,
                    ..ColorMix::default()
                };
                b
            },
            true,
            false,
            back_and_forth(20),
        ),
        (
            "色を混ぜる（混ぜる・絵の具 0.1・流量 0.3）・往復 40",
            {
                let mut b = base(18.0, 0.5, 0.05, 1.0, 0.3);
                b.mix = ColorMix {
                    mode: MixMode::Mix,
                    paint: 0.1,
                    ..ColorMix::default()
                };
                b
            },
            true,
            true,
            back_and_forth(40),
        ),
        (
            "色を混ぜる（伸ばす）・往復 20",
            {
                let mut b = base(18.0, 0.8, 0.05, 1.0, 1.0);
                b.mix = ColorMix {
                    mode: MixMode::Smear,
                    ..ColorMix::default()
                };
                b
            },
            true,
            false,
            back_and_forth(20),
        ),
        (
            "指先・往復 20",
            {
                let mut b = base(18.0, 0.8, 0.05, 1.0, 0.8);
                b.effect = BrushEffect::SMUDGE;
                b
            },
            true,
            false,
            back_and_forth(20),
        ),
    ]
}

fn compare(a: &str, b: &str) {
    println!("場面\t最大\t1\t2\t3〜4\t5〜8\t9〜\t違うバイト%\t違う画素%");
    for (n, (name, _, filled, translucent, _)) in scenes().into_iter().enumerate() {
        let read = |dir: &str| std::fs::read(format!("{dir}/{n}.rgba")).unwrap();
        let (p, q) = (read(a), read(b));
        let mut hist = [0usize; 5];
        let (mut worst, mut bytes, mut pixels) = (0u8, 0usize, 0usize);
        for (x, y) in p.as_chunks::<4>().0.iter().zip(q.as_chunks::<4>().0) {
            let mut any = false;
            for (s, t) in x.iter().zip(y) {
                let d = s.abs_diff(*t);
                if d == 0 {
                    continue;
                }
                any = true;
                bytes += 1;
                worst = worst.max(d);
                hist[match d {
                    1 => 0,
                    2 => 1,
                    3..=4 => 2,
                    5..=8 => 3,
                    _ => 4,
                }] += 1;
            }
            pixels += usize::from(any);
        }
        // 塗った画素（下地と違う画素）の数を分母にする
        let (doc, _) = document(filled, translucent);
        let before = doc.composite_channel(Channel::Color, doc.bounds()).unwrap();
        let painted = before
            .as_chunks::<4>()
            .0
            .iter()
            .zip(p.as_chunks::<4>().0)
            .filter(|(s, t)| s != t)
            .count()
            .max(1);
        println!(
            "{name}\t{worst}\t{}\t{}\t{}\t{}\t{}\t{:.4}\t{:.4}",
            hist[0],
            hist[1],
            hist[2],
            hist[3],
            hist[4],
            bytes as f64 * 100.0 / (painted * 4) as f64,
            pixels as f64 * 100.0 / painted as f64
        );
    }
    println!("# 割合の分母は、A で塗って下地から変わった画素（とそのバイト）");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build_global()
        .unwrap();
    use_cpu_time(true);
    match args.first().map(String::as_str) {
        Some("dump") => {
            let dir = &args[1];
            std::fs::create_dir_all(dir).unwrap();
            for (n, (name, brush, filled, translucent, points)) in scenes().into_iter().enumerate()
            {
                let (pixels, stamps, _) = stroke(&brush, filled, translucent, &points);
                std::fs::write(format!("{dir}/{n}.rgba"), pixels).unwrap();
                println!("{n}\t{name}\t{stamps}");
            }
        }
        Some("compare") => compare(&args[1], &args[2]),
        _ => {
            let runs: usize = args.get(1).map_or(5, |v| v.parse().unwrap());
            println!("場面\tダブ\t最小ms");
            for (name, brush, filled, translucent, points) in scenes() {
                let mut best = f64::INFINITY;
                let mut stamps = 0;
                for _ in 0..runs {
                    let (_, s, ms) = stroke(&brush, filled, translucent, &points);
                    best = best.min(ms);
                    stamps = s;
                }
                println!("{name}\t{stamps}\t{best:.2}");
            }
        }
    }
}
