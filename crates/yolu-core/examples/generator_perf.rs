//! Generator の計測の表（種類 × 大きさ × スレッド）と、段ごとの内訳。
//!
//! `cargo run --release -p yolu-core --example generator_perf`
//!
//! 入力・設定は毎回同じで、出力は全画素の SHA-256（先頭 12 桁）で確かめる（時間だけが動く）。同じ行・同じ大きさの出力はスレッド数が違っても
//! 同じ（違えば止める）。時間は最良の回（ミリ秒）。スレッド 1 本で 2048² 以上の列は CPU で走った時間（他のプロセスと取り合っても
//! 待ちが入らない。カーネルの計上が数 ms 刻みなので小さい大きさでは使わない）、ほかは壁時計。入力画像・マップの作成は含めず、
//! 束縛と返却画像の確保を含む。
//!
//! 入力は 2 通り。`rand` は C# との一致試験と同じ合成入力（マップの値が画素ごとにばらつく）、`smooth` はメッシュのマップに近い
//! なめらかな入力（隣の画素の位置・向きがほぼ同じ。UV アイランドの外は被覆 0）。
//!
//! `GEN_FUZZ=件数` は計測の代わりに、乱数（固定のシード）で作った設定・入力・領域の組を評価して、1 件ごとの出力のハッシュを出す。
//! 前後のコミットで組んだ例の出力を並べて比べれば、設定をばらまいた入力でバイトが変わっていないことを確かめられる（`YOLU_SIMD` で道を変えても同じ）。
//!
//! 環境変数（絞り込み）: `GEN_SIZES=256,1024,4096`・`GEN_THREADS=1,8`・`GEN_ROWS=部分文字列`（行の名前に含むものだけ）・
//! `GEN_RUNS=回数`（大きさに依らず固定）・`GEN_BREAKDOWN=1`（4096²・スレッド 1 の段ごとの内訳だけを出す）。
#[path = "../tests/generator_support/mod.rs"]
mod support;
use sha2::{Digest, Sha256};
use std::time::Instant;
use yolu_core::{generator::*, Rect};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Data {
    Rand,
    Smooth,
}
impl Data {
    fn name(self) -> &'static str {
        match self {
            Data::Rand => "rand",
            Data::Smooth => "smooth",
        }
    }
}

struct Row {
    name: String,
    data: Data,
    settings: Settings,
    target: Target,
    strength: f64,
}

fn tri(x: f64) -> f64 {
    let f = x - x.floor();
    1. - (2. * f - 1.).abs()
}
fn smooth01(t: f64) -> f64 {
    t * t * (3. - 2. * t)
}
fn encode(v: f64) -> u16 {
    (v.clamp(0., 1.) * 65535. + 0.5).floor() as u16
}

/// メッシュのマップに近いなめらかな値（+ − × ÷ sqrt floor だけ。環境の libm に依らない）。
fn smooth_map(kind: MapKind, w: u32, h: u32) -> (Vec<u16>, Vec<u8>) {
    let ch = kind.channels();
    let mut data = Vec::with_capacity(w as usize * h as usize * ch);
    let mut cover = Vec::with_capacity(w as usize * h as usize);
    let palette = [
        0x20_80_40u32,
        0xab_c1_23,
        0x10_20_f0,
        0xe0_30_30,
        0x70_70_70,
    ];
    for y in 0..h {
        let v = (y as f64 + 0.5) / h as f64;
        for x in 0..w {
            let u = (x as f64 + 0.5) / w as f64;
            let r2 = (u - 0.5) * (u - 0.5) + (v - 0.5) * (v - 0.5);
            cover.push(if r2 < 0.23 { 1 + (x / 64 % 2) as u8 } else { 0 });
            match kind {
                MapKind::Position => {
                    data.push(encode(0.08 + 0.84 * u));
                    data.push(encode(0.08 + 0.84 * v + 0.06 * u * v));
                    data.push(encode(
                        0.5 + 0.35 * (u - 0.5) * (1. - v) + 0.1 * smooth01(u),
                    ));
                }
                MapKind::WorldNormal | MapKind::BentNormal | MapKind::TangentNormal => {
                    let n = [1.6 * (u - 0.5), 1.6 * (v - 0.5), 0.6 + 0.4 * u * v];
                    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    for c in n {
                        data.push(encode((c / len + 1.) * 0.5));
                    }
                }
                MapKind::Id => {
                    let i = ((x * 8 / w) + (y * 8 / h) * 3) as usize % palette.len();
                    let c = palette[i];
                    for s in [16, 8, 0] {
                        data.push((((c >> s) & 255) * 257) as u16);
                    }
                }
                _ => data.push(encode(0.35 + 0.5 * smooth01(tri(u * 3.3 + v * 1.7)))),
            }
        }
    }
    (data, cover)
}
fn maps_for(s: &Settings, data: Data, w: u32, h: u32) -> support::Maps {
    match data {
        Data::Rand => support::Maps::new(s, w, h),
        Data::Smooth => support::Maps {
            data: s
                .used_maps()
                .into_iter()
                .map(|k| {
                    let (d, c) = smooth_map(k, w, h);
                    (k, d, c)
                })
                .collect(),
            w,
            h,
        },
    }
}

fn rows() -> Vec<Row> {
    let mut v = Vec::new();
    let mut csharp = |name: &str, kind: Kind, variant: usize, data: Data| {
        v.push(Row {
            name: name.to_string(),
            data,
            settings: support::settings(kind, variant),
            target: Target::Color,
            strength: if variant.is_multiple_of(2) { 1. } else { 0.43 },
        });
    };
    for data in [Data::Rand, Data::Smooth] {
        for kind in support::KINDS {
            let variant = if kind == Kind::ShapeGradient { 5 } else { 2 };
            csharp(&format!("{kind:?}"), kind, variant, data);
        }
        csharp("ShapeBoxRamp", Kind::ShapeGradient, 3, data);
        csharp("ShapeSphereRamp", Kind::ShapeGradient, 4, data);
        csharp("EdgeWear.uvnoise", Kind::EdgeWear, 4, data);
        csharp("EdgeWear.nonoise", Kind::EdgeWear, 3, data);
    }
    let mut procedural = |name: &str, s: Settings| {
        v.push(Row {
            name: name.to_string(),
            data: Data::Smooth,
            settings: s,
            target: Target::Color,
            strength: 1.,
        });
    };
    let noise =
        |basis: NoiseBasis, cell: CellOutput, fractal: FractalMode, space: ProceduralSpace| {
            let mut s = Settings::new(Kind::Noise);
            s.procedural.basis = basis;
            s.procedural.cell_output = cell;
            s.procedural.fractal = fractal;
            s.procedural.space = space;
            s.procedural.seed = 7;
            s
        };
    use CellOutput::*;
    use FractalMode::*;
    use NoiseBasis::*;
    use ProceduralSpace::*;
    procedural("Noise.Value", noise(Value, F1, Fbm, Position));
    procedural("Noise.Perlin", noise(Perlin, F1, Fbm, Position));
    procedural("Noise.Perlin.ridged", noise(Perlin, F1, Ridged, Position));
    procedural("Noise.Worley", noise(Worley, F1, Fbm, Position));
    procedural("Noise.WorleyF2F1", noise(Worley, F2MinusF1, Fbm, Position));
    procedural("Noise.Perlin.uv", noise(Perlin, F1, Fbm, Uv));
    procedural("Noise.Worley.uv", noise(Worley, F1, Fbm, Uv));
    procedural("Noise.Perlin.triplanar", noise(Perlin, F1, Fbm, Triplanar));
    for preset in GrungePreset::ALL {
        let mut s = Settings::grunge(preset);
        s.procedural.seed = 7;
        procedural(&format!("Grunge.{}", preset.id()), s);
    }
    for preset in [
        GrungePreset::Stain,
        GrungePreset::Cracks,
        GrungePreset::Weave,
    ] {
        let mut s = Settings::grunge(preset);
        s.procedural.seed = 7;
        s.procedural.space = Uv;
        procedural(&format!("Grunge.{}.uv", preset.id()), s);
    }
    let mut s = Settings::grunge(GrungePreset::Stain);
    s.procedural.seed = 7;
    s.procedural.bleed = 0.5;
    procedural("Grunge.stain.bleed", s);
    let mut s = noise(Perlin, F1, Fbm, Position);
    s.procedural.rotation = [13., -27., 41.];
    procedural("Noise.Perlin.rot", s);
    let mut s = noise(Value, F1, Turbulence, Position);
    s.procedural.bleed = 0.5;
    s.procedural.octaves = 8;
    procedural("Noise.Value.bleed8", s);
    procedural("Noise.WorleyF2.tri", noise(Worley, F2, Ridged, Triplanar));
    for preset in [GrungePreset::Scratches, GrungePreset::Fingerprints] {
        let mut s = Settings::grunge(preset);
        s.procedural.seed = 7;
        s.procedural.space = Uv;
        procedural(&format!("Grunge.{}.uv", preset.id()), s);
    }
    v
}

fn digest(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))[..12].to_string()
}

fn env_list(name: &str, default: &[u32]) -> Vec<u32> {
    std::env::var(name)
        .ok()
        .map(|s| s.split(',').filter_map(|v| v.trim().parse().ok()).collect())
        .unwrap_or_else(|| default.to_vec())
}

/// このプロセスの全スレッドが CPU で走った時間の合計（ミリ秒。Linux の `/proc/self/task/*/schedstat`）。取れなければ None。
/// 他のプロセスと CPU を取り合っていても、待たされた時間は入らない（スレッド 1 本の列は、待ちで揺れる壁時計の代わりにこれを使う）。
fn cpu_ms() -> Option<f64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir("/proc/self/task").ok()? {
        let text = std::fs::read_to_string(entry.ok()?.path().join("schedstat")).ok()?;
        total += text.split_whitespace().next()?.parse::<u64>().ok()?;
    }
    Some(total as f64 / 1e6)
}
/// 1 回ならしてから `runs` 回測り、最良の時間（ミリ秒）と最後の出力を返す。`cpu` が真なら CPU 時間（取れるとき）、偽なら壁時計。
fn best_ms<T>(runs: usize, cpu: bool, mut f: impl FnMut() -> T) -> (f64, T) {
    drop(f());
    let mut best = f64::MAX;
    let mut last = None;
    for _ in 0..runs {
        let (wall, cpu_before) = (Instant::now(), if cpu { cpu_ms() } else { None });
        let out = f();
        let elapsed = match (cpu_before, if cpu { cpu_ms() } else { None }) {
            (Some(a), Some(b)) => b - a,
            _ => wall.elapsed().as_secs_f64() * 1000.,
        };
        best = best.min(elapsed);
        last = Some(out);
    }
    (best, last.unwrap())
}

fn pool(threads: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
}

fn runs_for(size: u32) -> usize {
    std::env::var("GEN_RUNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(match size {
            0..=256 => 15,
            257..=1024 => 5,
            _ => 2,
        })
}

fn table() {
    let sizes = env_list("GEN_SIZES", &[256, 1024, 4096]);
    let threads = env_list("GEN_THREADS", &[1, 8]);
    let filter = std::env::var("GEN_ROWS").unwrap_or_default();
    let pools: Vec<_> = threads.iter().map(|t| (*t, pool(*t as usize))).collect();
    print!("{:<28}{:<7}", "row", "data");
    for s in &sizes {
        for t in &threads {
            print!("{:>10}", format!("{s}²/{t}t"));
        }
    }
    println!("  hash(per size)");
    for row in rows() {
        if !row.name.contains(&filter) {
            continue;
        }
        let mut cells = Vec::new();
        let mut hashes = Vec::new();
        for &size in &sizes {
            let (w, h) = (size, size);
            let owned = maps_for(&row.settings, row.data, w, h);
            let maps = owned.maps();
            let input = support::pixels(w, h, row.target);
            let image = Image::new(&input, w, h).unwrap();
            let anchor = anchor::LayerSample {
                source: &image,
                read: anchor::Read::Coverage,
            };
            let mut size_hash: Option<String> = None;
            for (t, p) in &pools {
                let run = || {
                    let b = BoundGenerator::bind(
                        &row.settings,
                        &maps,
                        Some(support::frame()),
                        (w, h),
                        Ok(&anchor),
                    )
                    .unwrap();
                    evaluate(
                        &image,
                        &b,
                        Rect::new(0, 0, w, h),
                        row.target,
                        row.strength,
                        &Options::default(),
                    )
                    .unwrap()
                    .pixels
                };
                let (ms, output) =
                    p.install(|| best_ms(runs_for(size), *t == 1 && size >= 2048, run));
                let d = digest(&output);
                match &size_hash {
                    Some(prev) => {
                        assert_eq!(prev, &d, "{} {size}²: スレッド {t} で出力が違う", row.name)
                    }
                    None => size_hash = Some(d),
                }
                cells.push(ms);
            }
            hashes.push(size_hash.unwrap());
        }
        print!("{:<28}{:<7}", row.name, row.data.name());
        for ms in cells {
            print!("{ms:>10.2}");
        }
        println!("  {}", hashes.join(" "));
    }
}

/// 4096²・スレッド 1 の段ごとの内訳（ミリ秒）。取り込みは強さ 0（入力を読んで書くだけ。出力の確保を含む）、値は `value()` を全画素に
/// 呼ぶ時間（1 画素ずつの API。ノイズなしの設定との差がノイズの重ね）、行は `sample_row` を全行に呼ぶ時間（行ごとの API。
/// ランプの色への変換を含む）、全体は `evaluate`。
fn breakdown() {
    let size = 4096;
    let filter = std::env::var("GEN_ROWS").unwrap_or_default();
    let p = pool(1);
    println!(
        "{:<24}{:<7}{:>9}{:>11}{:>9}{:>9}{:>9}{:>9}  (4096²・スレッド 1・ms)",
        "row", "data", "取り込み", "値(ノイズ無)", "値", "ノイズ", "行", "全体"
    );
    for row in rows() {
        if !row.name.contains(&filter) || row.settings.kind.is_procedural() {
            continue;
        }
        let (w, h) = (size, size);
        let owned = maps_for(&row.settings, row.data, w, h);
        let maps = owned.maps();
        let input = support::pixels(w, h, row.target);
        let image = Image::new(&input, w, h).unwrap();
        let anchor = anchor::LayerSample {
            source: &image,
            read: anchor::Read::Coverage,
        };
        let value_ms = |s: &Settings| {
            let owned = maps_for(s, row.data, w, h);
            let maps = owned.maps();
            let b = BoundGenerator::bind(s, &maps, Some(support::frame()), (w, h), Ok(&anchor))
                .unwrap();
            p.install(|| {
                best_ms(2, true, || {
                    let mut sum = 0.;
                    for y in 0..h {
                        for x in 0..w {
                            sum += b.value(x, y).unwrap_or(0.);
                        }
                    }
                    std::hint::black_box(sum)
                })
                .0
            })
        };
        let full = |strength: f64| {
            let b = BoundGenerator::bind(
                &row.settings,
                &maps,
                Some(support::frame()),
                (w, h),
                Ok(&anchor),
            )
            .unwrap();
            p.install(|| {
                best_ms(2, true, || {
                    evaluate(
                        &image,
                        &b,
                        Rect::new(0, 0, w, h),
                        row.target,
                        strength,
                        &Options::default(),
                    )
                    .unwrap()
                })
                .0
            })
        };
        let rows_ms = {
            let b = BoundGenerator::bind(
                &row.settings,
                &maps,
                Some(support::frame()),
                (w, h),
                Ok(&anchor),
            )
            .unwrap();
            p.install(|| {
                best_ms(2, true, || {
                    let mut buffer = vec![None; w as usize];
                    let mut count = 0usize;
                    for y in 0..h {
                        b.sample_row(0, y, row.target != Target::Color, &mut buffer);
                        count += buffer.iter().filter(|g| g.is_some()).count();
                    }
                    std::hint::black_box(count)
                })
                .0
            })
        };
        let copy = full(0.);
        let all = full(row.strength);
        let with = value_ms(&row.settings);
        let mut plain = row.settings.clone();
        plain.noise_amount = 0.;
        let without = if row.settings.noise_amount > 0. {
            value_ms(&plain)
        } else {
            with
        };
        println!(
            "{:<24}{:<7}{copy:>9.1}{without:>11.1}{with:>9.1}{:>9.1}{rows_ms:>9.1}{all:>9.1}",
            row.name,
            row.data.name(),
            with - without
        );
    }
}

/// 固定のシードの乱数（`GEN_FUZZ` の入力を毎回同じにする）。
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
}

fn fuzz_settings(rng: &mut Rng) -> Settings {
    use FractalMode::*;
    let kinds = [
        Kind::EdgeWear,
        Kind::Dirt,
        Kind::PositionGradient,
        Kind::Thickness,
        Kind::Direction,
        Kind::ShapeGradient,
        Kind::IdColor,
        Kind::Anchor,
        Kind::Noise,
        Kind::Noise,
        Kind::Grunge,
        Kind::Grunge,
        Kind::Grunge,
    ];
    let kind = kinds[rng.below(kinds.len())];
    let mut s = Settings::new(kind);
    s.low = rng.range(0., 0.6);
    s.high = (s.low + rng.range(0.05, 0.4)).min(1.);
    if s.high - s.low < 0.001 {
        s.low = 0.;
    }
    s.softness = if rng.chance(0.5) {
        0.
    } else {
        rng.range(0., 1.)
    };
    s.invert = rng.chance(0.3);
    s.blend = [
        Blend::Multiply,
        Blend::Replace,
        Blend::Screen,
        Blend::Max,
        Blend::Min,
        Blend::Add,
        Blend::Subtract,
    ][rng.below(7)];
    if kind.is_procedural() {
        let p = &mut s.procedural;
        p.space = [
            ProceduralSpace::Position,
            ProceduralSpace::Triplanar,
            ProceduralSpace::Uv,
        ][rng.below(3)];
        p.scale = [0.004, 0.01, 0.03, 0.08, 0.2, 0.6, 1.][rng.below(7)];
        p.seed = rng.next() as i32;
        p.rotation = if rng.chance(0.5) {
            [0.; 3]
        } else {
            [
                rng.range(-360., 360.),
                rng.range(-360., 360.),
                rng.range(-360., 360.),
            ]
        };
        p.bleed = if rng.chance(0.5) {
            0.
        } else {
            rng.range(0., 1.)
        };
        p.blend_width = rng.range(0., 1.);
        if kind == Kind::Noise {
            p.basis = [NoiseBasis::Value, NoiseBasis::Perlin, NoiseBasis::Worley][rng.below(3)];
            if p.basis == NoiseBasis::Worley {
                p.cell_output =
                    [CellOutput::F1, CellOutput::F2, CellOutput::F2MinusF1][rng.below(3)];
            }
            p.fractal = [Fbm, Ridged, Turbulence][rng.below(3)];
            p.octaves = 1 + rng.below(8) as u32;
            p.lacunarity = rng.range(1., 4.);
            p.gain = rng.range(0., 1.);
        } else {
            let preset = GrungePreset::ALL[rng.below(GrungePreset::ALL.len())];
            let (low, high) = (s.low, s.high);
            s.set_preset(preset);
            if rng.chance(0.5) {
                (s.low, s.high) = (low, high);
            }
            s.procedural.scale = [0.004, 0.02, 0.05, 0.1, 0.3, 1.][rng.below(6)];
        }
        return s;
    }
    s.noise_amount = if rng.chance(0.4) {
        0.
    } else {
        rng.range(0.05, 1.)
    };
    s.noise_scale = [0.001, 0.01, 0.05, 0.2, 1.][rng.below(5)];
    s.noise_seed = rng.next() as i32;
    s.noise_space = if rng.chance(0.5) {
        NoiseSpace::Model
    } else {
        NoiseSpace::Uv
    };
    match kind {
        Kind::Dirt => s.balance = [0., 0.3, 1.][rng.below(3)],
        Kind::PositionGradient => s.axis = rng.below(3),
        Kind::Direction => {
            s.direction = [rng.range(-1., 1.), rng.range(-1., 1.), rng.range(0.1, 1.)];
            s.use_bent_normal = rng.chance(0.5);
        }
        Kind::ShapeGradient => {
            s.volume = Volume {
                shape: [Shape::Box, Shape::Sphere, Shape::Plane][rng.below(3)],
                center: [rng.range(-1., 1.), rng.range(-1., 1.), rng.range(-1., 1.)],
                rotation: [
                    rng.range(-90., 90.),
                    rng.range(-90., 90.),
                    rng.range(-90., 90.),
                ],
                size: [rng.range(0.5, 5.), rng.range(0.5, 5.), rng.range(0.5, 5.)],
                falloff: if rng.chance(0.3) {
                    0.
                } else {
                    rng.range(0., 1.)
                },
            };
            if rng.chance(0.6) {
                s.ramp = Some(support::ramp());
            }
        }
        Kind::IdColor => {
            s.id_colors = vec![0x112233, 0xabc123, 0x20_80_40];
            s.id_tolerance = rng.below(80) as u8;
        }
        _ => {}
    }
    s
}

/// 乱数で作った設定・マップ（なめらか・ばらばら・被覆の穴・古い状態・境界箱が 0 の幅）・入力・領域の組を評価して、出力のハッシュを 1 件ずつ出す。
fn fuzz(count: u64) {
    let mut rng = Rng(0x00C0_FFEE);
    for i in 0..count {
        let (w, h) = (20 + rng.below(70) as u32, 10 + rng.below(40) as u32);
        let s = fuzz_settings(&mut rng);
        let smooth = rng.chance(0.5);
        let target = [Target::Color, Target::Scalar, Target::Mask][rng.below(3)];
        let strength = [1., 0.43, 0.9, 0.][rng.below(4)];
        let region = {
            let rw = 1 + rng.below(w as usize) as u32;
            let rh = 1 + rng.below(h as usize) as u32;
            Rect::new(
                rng.below((w - rw + 1) as usize) as u32,
                rng.below((h - rh + 1) as usize) as u32,
                rw,
                rh,
            )
        };
        let (min, extent) = (
            [rng.range(-5., 5.), rng.range(-5., 5.), rng.range(-5., 5.)],
            [
                if rng.chance(0.05) {
                    0.
                } else {
                    rng.range(0.1, 6.)
                },
                rng.range(0.1, 6.),
                rng.range(0.1, 6.),
            ],
        );
        let state = if rng.chance(0.08) {
            MapState::Stale
        } else {
            MapState::Current
        };
        let owned = maps_for(&s, if smooth { Data::Smooth } else { Data::Rand }, w, h);
        let maps: Vec<Map<'_>> = owned
            .data
            .iter()
            .map(|(k, d, c)| Map {
                kind: *k,
                width: w,
                height: h,
                data: d,
                coverage: c,
                bounds_min: min,
                bounds_max: [min[0] + extent[0], min[1] + extent[1], min[2] + extent[2]],
                condition_key: support::KEY,
                state,
            })
            .collect();
        let input = support::pixels(w, h, target);
        let image = Image::new(&input, w, h).unwrap();
        let read = [
            anchor::Read::Color,
            anchor::Read::Scalar,
            anchor::Read::Coverage,
        ][rng.below(3)];
        let layer = anchor::LayerSample {
            source: &image,
            read,
        };
        let frame = ModelFrame::new(
            [rng.range(-1., 1.), rng.range(-1., 1.), rng.range(-1., 1.)],
            [
                rng.range(-1., 1.),
                rng.range(-1., 1.),
                rng.range(-1., 1.),
                1.,
            ],
        )
        .unwrap();
        let line = match BoundGenerator::bind(&s, &maps, Some(frame), (w, h), Ok(&layer)) {
            Err(e) => format!("error {e}"),
            Ok(b) => match evaluate(&image, &b, region, target, strength, &Options::default()) {
                Err(e) => format!("error {e}"),
                Ok(out) => format!("{} {:?}", digest(&out.pixels), out.inactive),
            },
        };
        println!("{i} {:?} {w}x{h} {target:?} {strength} {line}", s.kind);
    }
}

fn main() {
    if let Some(count) = std::env::var("GEN_FUZZ").ok().and_then(|v| v.parse().ok()) {
        fuzz(count);
    } else if std::env::var("GEN_BREAKDOWN").is_ok() {
        breakdown();
    } else {
        table();
    }
}
