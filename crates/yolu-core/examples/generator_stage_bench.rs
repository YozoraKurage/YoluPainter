//! フィルターのスタックの Generator の段の速さ。4096² の人工の文書（全面の Color と Roughness・人工のメッシュマップ）で、Generator の段を
//! 1 つ・3 つ積んだ層と、ランプの混色（通常・リニア・知覚的）の段の、キャッシュを捨てた合成の時間を測る。
//!
//! `cargo run --release -p yolu-core --example generator_stage_bench`。環境変数: `STAGE_SIZE`（既定 4096）・`STAGE_THREADS`（既定 8）・
//! `STAGE_RUNS`（既定 3。表の値は最良）・`STAGE_CASES`（名前の部分一致で絞る。カンマ区切り）・`STAGE_COARSE`（歩幅。粗い合成
//! `Document::composite_coarse_tiles` を全部のタイルで測る）。
//! 1 行に名前・最良の ms・合成の SHA-256 の頭 16 桁を出す。前後のコミットで組んだ例の出力（`YOLU_SIMD` を変えても）を比べれば、
//! 同じバイトかを確かめられる。
use sha2::{Digest, Sha256};
use std::time::Instant;
use yolu_core::generator::{
    Blend, ColorStop, Kind, LuminanceCorrection, MapKind, MapState, MixMode, OpacityStop, Ramp,
    Settings,
};
use yolu_core::{
    Channel, Document, EffectInputs, EffectSettings, FilterSpec, FilterTarget, MapInput,
    ModelFrame, Rect,
};

const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let threads = env("STAGE_THREADS", 8);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    println!(
        "大きさ {}²・スレッド {threads}・YOLU_SIMD={}",
        env("STAGE_SIZE", 4096),
        std::env::var("YOLU_SIMD").unwrap_or_default()
    );
    pool.install(run);
}

/// なめらかに動く位置・向き・曲率のマップ（被覆は全部 1。端の 1 列だけ 0）。
fn maps(n: u32) -> EffectInputs {
    let mut inputs = EffectInputs::new().with_frame(Some(ModelFrame::default()));
    for kind in [MapKind::Position, MapKind::WorldNormal, MapKind::Curvature] {
        let channels = kind.channels();
        let mut data = Vec::with_capacity((n * n) as usize * channels);
        let mut coverage = Vec::with_capacity((n * n) as usize);
        for y in 0..n {
            for x in 0..n {
                let (u, v) = (x as f64 / n as f64, y as f64 / n as f64);
                match kind {
                    MapKind::Position => data.extend([
                        (u * 65535.) as u16,
                        (v * 65535.) as u16,
                        ((0.5 + 0.4 * (u * 9.).sin() * (v * 7.).cos()) * 65535.) as u16,
                    ]),
                    MapKind::WorldNormal => {
                        let (a, b) = ((u * 13.).sin(), (v * 11.).cos());
                        let len = (a * a + b * b + 1.).sqrt();
                        data.extend(
                            [a / len, b / len, 1. / len].map(|c| ((c * 0.5 + 0.5) * 65535.) as u16),
                        );
                    }
                    _ => {
                        data.push(((0.5 + 0.5 * (u * 37.).sin() * (v * 29.).sin()) * 65535.) as u16)
                    }
                }
                coverage.push(u8::from(x != 0));
            }
        }
        inputs = inputs
            .with_map(
                MapInput::new(
                    kind,
                    n,
                    n,
                    data,
                    coverage,
                    [-1., -1., -1.],
                    [1., 1., 1.],
                    KEY,
                    MapState::Current,
                )
                .unwrap(),
            )
            .unwrap();
    }
    inputs
}

fn ramp(mode: MixMode) -> Ramp {
    let stop = |position, rgb: [u8; 3], midpoint| ColorStop {
        position,
        color: yolu_core::Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
        midpoint,
    };
    Ramp::new(
        vec![
            stop(0., [20, 40, 220], 0.5),
            stop(0.45, [250, 230, 30], 0.35),
            stop(1., [200, 20, 60], 0.6),
        ],
        vec![
            OpacityStop {
                position: 0.,
                opacity: 1.,
                midpoint: 0.5,
            },
            OpacityStop {
                position: 1.,
                opacity: 0.7,
                midpoint: 0.5,
            },
        ],
        None,
    )
    .unwrap()
    .with_mixing(mode, LuminanceCorrection::High)
}

fn noise() -> Settings {
    let mut s = Settings::new(Kind::Noise);
    s.procedural.seed = 5;
    s.procedural.scale = 0.08;
    s
}
fn edge() -> Settings {
    let mut s = Settings::new(Kind::EdgeWear);
    s.blend = Blend::Screen;
    s
}
fn shape(mode: Option<MixMode>) -> Settings {
    let mut s = Settings::new(Kind::ShapeGradient);
    s.volume.rotation = [10., 20., 30.];
    s.blend = Blend::Replace;
    s.ramp = mode.map(ramp);
    s
}

/// 1 つの場合: (名前, チャンネル, 段の設定と強さ)
type Case = (&'static str, Channel, Vec<(Settings, f64)>);

fn cases() -> Vec<Case> {
    vec![
        ("色・効果なし", Channel::Color, vec![]),
        ("色・ノイズ 1 段", Channel::Color, vec![(noise(), 1.)]),
        ("色・エッジの摩耗 1 段", Channel::Color, vec![(edge(), 1.)]),
        (
            "色・3 段（ノイズ・摩耗・形とランプ）",
            Channel::Color,
            vec![
                (noise(), 1.),
                (edge(), 0.7),
                (shape(Some(MixMode::Standard)), 0.5),
            ],
        ),
        (
            "スカラー・3 段（ノイズ・摩耗・形）",
            Channel::Roughness,
            vec![(noise(), 1.), (edge(), 0.7), (shape(None), 0.5)],
        ),
        (
            "色・ランプ 通常",
            Channel::Color,
            vec![(shape(Some(MixMode::Standard)), 1.)],
        ),
        (
            "色・ランプ リニア",
            Channel::Color,
            vec![(shape(Some(MixMode::Linear)), 1.)],
        ),
        (
            "色・ランプ 知覚的",
            Channel::Color,
            vec![(shape(Some(MixMode::Perceptual)), 1.)],
        ),
    ]
}

fn run() {
    let n = env("STAGE_SIZE", 4096) as u32;
    let runs = env("STAGE_RUNS", 3).max(1);
    let only: Vec<String> = std::env::var("STAGE_CASES")
        .map(|v| v.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    // 粗い合成（歩幅で拾う。操作中の仮の絵）を測るなら歩幅
    let coarse = std::env::var("STAGE_COARSE")
        .ok()
        .and_then(|v| v.parse::<u32>().ok());
    let inputs = maps(n);
    let rect = Rect::new(0, 0, n, n);
    for (name, channel, stages) in cases() {
        if !only.is_empty() && !only.iter().any(|o| name.contains(o.as_str())) {
            continue;
        }
        let mut doc = Document::new(n, n).unwrap();
        doc.set_source_budget_bytes(1 << 30).unwrap();
        let layer = doc.add_layer("層").unwrap();
        doc.set_channel_enabled(layer, channel, true).unwrap();
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
                        if (x + y) % 23 == 0 { 0 } else { 255 },
                    ]);
                }
                doc.import_tile(layer, channel, yolu_core::TileCoord::new(tx, ty), &bytes)
                    .unwrap();
            }
        }
        doc.set_effect_inputs(inputs.clone()).unwrap();
        let tiles: Vec<yolu_core::TileCoord> = (0..n.div_ceil(tile))
            .flat_map(|y| (0..n.div_ceil(tile)).map(move |x| yolu_core::TileCoord::new(x, y)))
            .collect();
        for (settings, strength) in stages {
            doc.add_filter(
                layer,
                FilterTarget::Content,
                FilterSpec::new(EffectSettings::generator(settings))
                    .channels(&[channel])
                    .strength(strength),
            )
            .unwrap();
        }
        let mut best = f64::INFINITY;
        let mut hash = String::new();
        for _ in 0..runs {
            doc.release_effect_cache();
            let start = Instant::now();
            let out = match coarse {
                None => doc.composite_channel(channel, rect).unwrap(),
                Some(stride) => doc
                    .composite_coarse_tiles(channel, &tiles, stride)
                    .unwrap()
                    .into_iter()
                    .flat_map(|t| t.pixels)
                    .collect(),
            };
            best = best.min(start.elapsed().as_secs_f64() * 1000.);
            let digest = Sha256::digest(&out);
            hash = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
        }
        println!("{name}\t{best:.1} ms\t{hash}");
    }
}
