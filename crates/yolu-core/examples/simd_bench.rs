//! 合成・調整・フィルターの画素の計算の速さ（SIMD の前後を同じ表で比べる）。
//!   cargo run --release -p yolu-core --example simd_bench [blend|adjust|filter|normal|all] [回数]
//! 1 タイル（256²）と 4096² を、合成モードごと・調整の種類ごと・フィルターの種類ごとに測る。`normal` は Normal のチャンネルの
//! 合成（4096²・レイヤー 10。Normal と Overlay（RNM）を交互に、不透明度 0.45〜1）。スレッドは環境変数 SIMD_THREADS（既定 1。
//! 1 は 1 コアあたりの時間）。下のレイヤーの違い（不透明 / アルファ入り）も分ける。時間は最小の回の CPU 時間（ほかの負荷で待たされた分を除く。
//! スレッドが 1 本のときだけ意味がある）。
//! 道の切り替え: 環境変数 YOLU_SIMD=scalar|sse41|avx2（x86_64）・scalar|neon（aarch64。Apple Silicon の Mac など）。その CPU にない道の名前は
//! 無視する。使った道は出力の先頭の行に出る。`kernel` は行の核だけの時間（ns / 画素）。
#[path = "../tests/filter_support/mod.rs"]
mod filter_support;

use std::hint::black_box;
use std::time::Instant;

use filter_support::{pattern, stages};
use yolu_core::blend::{blend_row, clip_row, fade_row, RowAmount};
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::filter::{self, Image, Options, ValueType};
use yolu_core::generator::Ramp;
use yolu_core::{
    AdjustmentSettings, BlendMode, BrightnessContrast, Channel, ColorBalance, Document,
    GradientMap, LayerId, Posterize, Rect, Threshold, TileCoord, ToneChannel, ToneCurves,
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

fn fill_random(doc: &mut Document, layer: LayerId, seed: u64, opaque: bool) {
    let ts = doc.tile_size() as usize;
    let mut rng = SplitMix(seed);
    let mut bytes = vec![0u8; ts * ts * 4];
    for ty in 0..doc.height().div_ceil(ts as u32) {
        for tx in 0..doc.width().div_ceil(ts as u32) {
            for p in bytes.as_chunks_mut::<4>().0 {
                let (r, g, b, a) = (
                    rng.channel(),
                    rng.channel(),
                    rng.channel(),
                    rng.alpha(opaque),
                );
                p.copy_from_slice(&[r, g, b, a]);
            }
            doc.import_tile(layer, Channel::Color, TileCoord::new(tx, ty), &bytes)
                .unwrap();
        }
    }
    doc.clear_history().unwrap();
}

/// スレッドが複数のときは壁時計（CPU 時間は 1 本ぶんしか見えない）。
static WALL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "linux")]
mod thread_time {
    #[repr(C)]
    struct Timespec {
        sec: i64,
        nsec: i64,
    }
    extern "C" {
        fn clock_gettime(clock: i32, ts: *mut Timespec) -> i32;
    }
    /// いま走っているスレッドが CPU を使った時間（ミリ秒。ほかの負荷で待たされた時間を含まない）。
    pub fn now_ms() -> Option<f64> {
        const CLOCK_THREAD_CPUTIME_ID: i32 = 3;
        let mut ts = Timespec { sec: 0, nsec: 0 };
        // SAFETY: ts は書き込める Timespec（Linux の x86_64 / aarch64 の struct timespec と同じ並び）
        let ok = unsafe { clock_gettime(CLOCK_THREAD_CPUTIME_ID, &mut ts) } == 0;
        ok.then(|| ts.sec as f64 * 1000.0 + ts.nsec as f64 / 1e6)
    }
}
#[cfg(not(target_os = "linux"))]
mod thread_time {
    pub fn now_ms() -> Option<f64> {
        None
    }
}

/// 測る時刻（ミリ秒）。スレッドが 1 本のときはそのスレッドの CPU 時間、複数のときや読めないときは壁時計。
fn cpu_ms() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    if !WALL.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(t) = thread_time::now_ms() {
            return t;
        }
    }
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// 最小（ミリ秒、CPU 時間）。
fn min_ms(runs: usize, mut f: impl FnMut()) -> f64 {
    f();
    (0..runs)
        .map(|_| {
            let t = cpu_ms();
            f();
            cpu_ms() - t
        })
        .fold(f64::INFINITY, f64::min)
}

/// 1 回が短い処理用: reps 回の合計の CPU 時間を reps で割り（スケジューラの時間の粒度より長くなる）、batches 組の最小（ミリ秒）。
fn min_batch_ms(batches: usize, reps: usize, mut f: impl FnMut()) -> f64 {
    f();
    (0..batches)
        .map(|_| {
            let t = cpu_ms();
            for _ in 0..reps {
                f();
            }
            (cpu_ms() - t) / reps as f64
        })
        .fold(f64::INFINITY, f64::min)
}

fn doc_pair(n: u32, base_opaque: bool, top_opaque: bool) -> (Document, LayerId, LayerId) {
    let mut doc = Document::new(n, n).unwrap();
    doc.set_source_budget_bytes(2 << 30).unwrap();
    let a = doc.add_layer("a").unwrap();
    fill_random(&mut doc, a, 1, base_opaque);
    let b = doc.add_layer("b").unwrap();
    fill_random(&mut doc, b, 2, top_opaque);
    (doc, a, b)
}

/// 環境変数 SIMD_FILTER（カンマ区切りの部分文字列）に当たる名前だけを測る。
fn wanted(name: &str) -> bool {
    match std::env::var("SIMD_FILTER") {
        Ok(f) if !f.is_empty() => f.split(',').any(|p| name.contains(p)),
        _ => true,
    }
}

fn row(section: &str, name: &str, tile_us: f64, full_ms: f64) {
    println!("{section}\t{name}\t{tile_us:.1}\t{full_ms:.2}");
}

fn blend_section(runs: usize) {
    for (label, base_opaque) in [("不透明の下", true), ("アルファ入りの下", false)] {
        let (mut doc, _a, b) = doc_pair(4096, base_opaque, false);
        doc.set_layer_opacity(b, 0.7, false).unwrap();
        let tile = Rect::new(0, 0, 256, 256);
        let full = doc.bounds();
        if wanted("基準") {
            // 上のレイヤーを隠して下のレイヤーだけ（合成の枠組みそのものの時間）
            doc.set_layer_visible(b, false).unwrap();
            let t = min_batch_ms(runs, 60, || {
                black_box(doc.composite(tile).unwrap());
            }) * 1000.0;
            let f = min_ms(runs, || {
                black_box(doc.composite(full).unwrap());
            });
            row(&format!("合成 {label}"), "基準（下のレイヤーだけ）", t, f);
            doc.set_layer_visible(b, true).unwrap();
        }
        let mut modes: Vec<BlendMode> = BlendMode::LAYER_MODES.to_vec();
        modes.dedup();
        for mode in modes {
            if !wanted(mode.name()) {
                continue;
            }
            doc.set_layer_blend_mode(b, mode).unwrap();
            let t = min_batch_ms(runs, 60, || {
                black_box(doc.composite(tile).unwrap());
            }) * 1000.0;
            let f = min_ms(runs, || {
                black_box(doc.composite(full).unwrap());
            });
            row(&format!("合成 {label}"), mode.name(), t, f);
        }
    }
}

/// 法線の画素のレイヤー（上向きに寄った乱数の向き。アルファは opaque なら 255、ほかは 0・255・中間が混ざる）。
fn fill_normals(doc: &mut Document, layer: LayerId, seed: u64, opaque: bool) {
    let ts = doc.tile_size() as usize;
    let mut rng = SplitMix(seed);
    let mut bytes = vec![0u8; ts * ts * 4];
    for ty in 0..doc.height().div_ceil(ts as u32) {
        for tx in 0..doc.width().div_ceil(ts as u32) {
            for p in bytes.as_chunks_mut::<4>().0 {
                let r = rng.next();
                let (x, y, z) = (
                    68 + (r % 121) as u8,
                    68 + ((r >> 8) % 121) as u8,
                    190 + ((r >> 16) % 66) as u8,
                );
                p.copy_from_slice(&[x, y, z, rng.alpha(opaque)]);
            }
            doc.import_tile(layer, Channel::Normal, TileCoord::new(tx, ty), &bytes)
                .unwrap();
        }
    }
    doc.clear_history().unwrap();
}

/// Normal のチャンネルの合成（4096²・レイヤー 10）。
fn normal_section(runs: usize) {
    let mut doc = Document::new(4096, 4096).unwrap();
    doc.set_source_budget_bytes(4 << 30).unwrap();
    for k in 0..10u64 {
        let l = doc.add_layer("n").unwrap();
        fill_normals(&mut doc, l, 10 + k, k == 0);
        if k > 0 {
            doc.set_layer_opacity(l, 0.45 + 0.06 * k as f64, false)
                .unwrap();
            let mode = if k % 2 == 0 {
                BlendMode::Overlay
            } else {
                BlendMode::Normal
            };
            doc.set_layer_blend_mode(l, mode).unwrap();
        }
    }
    let tile = Rect::new(0, 0, 256, 256);
    let full = doc.bounds();
    let t = min_batch_ms(runs, 6, || {
        black_box(doc.composite_channel(Channel::Normal, tile).unwrap());
    }) * 1000.0;
    let f = min_ms(runs, || {
        black_box(doc.composite_channel(Channel::Normal, full).unwrap());
    });
    row(
        "Normal のチャンネル",
        "レイヤー 10（Normal・Overlay）",
        t,
        f,
    );
}

fn adjust_kinds() -> Vec<(&'static str, AdjustmentSettings)> {
    vec![
        ("反転", AdjustmentSettings::invert()),
        (
            "レベル補正",
            AdjustmentSettings::levels(0.1, 0.9, 1.4, 0.05, 0.95).unwrap(),
        ),
        (
            "色相・彩度",
            AdjustmentSettings::hue_saturation(40.0, 0.3, 0.1).unwrap(),
        ),
        (
            "グラデーションマップ",
            AdjustmentSettings::gradient_map(GradientMap::new(Ramp::default(), false)),
        ),
        (
            "トーンカーブ",
            AdjustmentSettings::tone_curve(
                ToneCurves::identity().with_curve(
                    ToneChannel::Composite,
                    Curve::new(vec![
                        CurvePoint { x: 0.0, y: 0.1 },
                        CurvePoint { x: 0.4, y: 0.6 },
                        CurvePoint { x: 1.0, y: 0.9 },
                    ])
                    .unwrap(),
                ),
            ),
        ),
        (
            "カラーバランス",
            AdjustmentSettings::color_balance(
                ColorBalance::new(
                    [10.0, -20.0, 30.0],
                    [40.0, 0.0, -30.0],
                    [0.0, 20.0, 50.0],
                    true,
                )
                .unwrap(),
            ),
        ),
        (
            "明るさ・コントラスト",
            AdjustmentSettings::brightness_contrast(BrightnessContrast::new(30.0, 20.0).unwrap()),
        ),
        (
            "2 値化",
            AdjustmentSettings::threshold(Threshold::new(128).unwrap()),
        ),
        (
            "ポスタリゼーション",
            AdjustmentSettings::posterize(Posterize::new(5).unwrap()),
        ),
    ]
}

fn adjust_section(runs: usize) {
    let kinds = adjust_kinds();
    let tile = Rect::new(0, 0, 256, 256);
    for (mode_label, mode) in [
        ("Normal", BlendMode::Normal),
        ("Overlay", BlendMode::Overlay),
        ("Hue", BlendMode::Hue),
    ] {
        let mut doc = Document::new(4096, 4096).unwrap();
        doc.set_source_budget_bytes(2 << 30).unwrap();
        let a = doc.add_layer("a").unwrap();
        fill_random(&mut doc, a, 1, false);
        let full = doc.bounds();
        for (name, settings) in &kinds {
            if !wanted(name) {
                continue;
            }
            let l = doc
                .add_adjustment_layer("adj", settings.clone(), None, None)
                .unwrap();
            doc.set_layer_opacity(l, 0.7, false).unwrap();
            doc.set_layer_blend_mode(l, mode).unwrap();
            let t = min_batch_ms(runs, 60, || {
                black_box(doc.composite(tile).unwrap());
            }) * 1000.0;
            let f = min_ms(runs, || {
                black_box(doc.composite(full).unwrap());
            });
            row(&format!("調整 {mode_label}"), name, t, f);
            doc.remove_layer(l).unwrap();
        }
    }
}

fn filter_section(runs: usize) {
    let data = pattern(4096, 4096, ValueType::Color);
    let src = Image::new(&data, 4096, 4096).unwrap();
    let region = Rect::new(0, 0, 4096, 4096);
    let tile = Rect::new(1024, 2048, 256, 256);
    for spec in [
        "blur 8",
        "blur 40",
        "sharpen 3 1.7 12",
        "noise 0.47 -39 1",
        "noise 0.83 39 0",
        "levels 0.1 0.91 1.7 0.05 0.93",
        "invert",
        "normalize",
    ] {
        if !wanted(spec) {
            continue;
        }
        let stack = stages(&format!("1 {spec}"));
        let options = Options::default();
        let t = min_batch_ms(runs, 6, || {
            black_box(filter::evaluate(&src, ValueType::Color, &stack, tile, &options).unwrap());
        }) * 1000.0;
        let f = min_ms(runs, || {
            black_box(filter::evaluate(&src, ValueType::Color, &stack, region, &options).unwrap());
        });
        row("フィルター", spec, t, f);
    }
}

/// 行の核だけの時間（ns / 画素）。256 画素の行 × 256 行（1 タイル）を繰り返す。道は YOLU_SIMD で選ぶ（プロセスごとに 1 回決まる）。
fn kernel_section(runs: usize) {
    const W: usize = 256;
    const ROWS: usize = 256;
    let mut rng = SplitMix(99);
    let mut make = |opaque: bool| -> Vec<u8> {
        let mut v = vec![0u8; W * ROWS * 4];
        for p in v.as_chunks_mut::<4>().0 {
            p.copy_from_slice(&[
                rng.channel(),
                rng.channel(),
                rng.channel(),
                rng.alpha(opaque),
            ]);
        }
        v
    };
    let below_opaque = make(true);
    let below_alpha = make(false);
    let over = make(false);
    let mut factor = [0.0f64; 256];
    for (h, f) in factor.iter_mut().enumerate() {
        *f = 1.0 - 0.8 * (h as f64 / 255.0);
    }
    let amount = RowAmount {
        opacity: 0.7,
        mask: None,
    };
    let per_pixel = |ms: f64| ms * 1e6 / (W * ROWS) as f64;
    let mut buf = vec![0u8; W * ROWS * 4];
    for (label, below) in [
        ("不透明の下", &below_opaque),
        ("アルファ入りの下", &below_alpha),
    ] {
        for mode in [
            BlendMode::Normal,
            BlendMode::Multiply,
            BlendMode::Overlay,
            BlendMode::SoftLight,
            BlendMode::VividLight,
            BlendMode::Hue,
            BlendMode::Color,
            BlendMode::DarkerColor,
        ] {
            if !wanted(mode.name()) {
                continue;
            }
            let ms = min_batch_ms(runs, 20, || {
                buf.copy_from_slice(below);
                for r in 0..ROWS {
                    let a = r * W * 4;
                    blend_row(&mut buf[a..a + W * 4], &over[a..a + W * 4], 4, amount, mode);
                }
                black_box(&buf);
            });
            row_ns(&format!("核 blend {label}"), mode.name(), per_pixel(ms));
        }
        let ms = min_batch_ms(runs, 20, || {
            buf.copy_from_slice(below);
            for r in 0..ROWS {
                let a = r * W * 4;
                clip_row(
                    &mut buf[a..a + W * 4],
                    &over[a..a + W * 4],
                    4,
                    amount,
                    BlendMode::Multiply,
                );
            }
            black_box(&buf);
        });
        row_ns(&format!("核 clip {label}"), "Multiply", per_pixel(ms));
        let ms = min_batch_ms(runs, 20, || {
            buf.copy_from_slice(below);
            for r in 0..ROWS {
                let a = r * W * 4;
                fade_row(&mut buf[a..a + W * 4], &over[a..a + W * 4], amount);
            }
            black_box(&buf);
        });
        row_ns(&format!("核 fade {label}"), "通過のグループ", per_pixel(ms));
    }
    let _ = &factor;
    for (label, below) in [
        ("不透明の下", &below_opaque),
        ("アルファ入りの下", &below_alpha),
    ] {
        for mode in [BlendMode::Normal, BlendMode::Overlay] {
            let ms = min_batch_ms(runs, 20, || {
                buf.copy_from_slice(below);
                for r in 0..ROWS {
                    let a = r * W * 4;
                    yolu_core::normal::blend_row(
                        &mut buf[a..a + W * 4],
                        &over[a..a + W * 4],
                        4,
                        amount,
                        mode,
                    );
                }
                black_box(&buf);
            });
            row_ns(
                &format!("核 Normal のチャンネル {label}"),
                mode.name(),
                per_pixel(ms),
            );
        }
    }
    // コピーだけの床（行の複写）
    let ms = min_batch_ms(runs, 20, || {
        buf.copy_from_slice(&below_opaque);
        black_box(&buf);
    });
    row_ns("核", "基準（複写だけ）", per_pixel(ms));
    for (name, settings) in adjust_kinds() {
        if !wanted(name) {
            continue;
        }
        for mode in [BlendMode::Normal, BlendMode::Overlay] {
            // 表を作る種類（レベル補正）の表の構築が 1 画素あたりに効かないよう、タイル全体を 1 回の呼び出しで
            let ms = min_batch_ms(runs, 20, || {
                buf.copy_from_slice(&below_alpha);
                settings.composite_row(yolu_core::ChannelKind::Color, &mut buf, amount, mode);
                black_box(&buf);
            });
            row_ns(&format!("核 調整 {}", mode.name()), name, per_pixel(ms));
        }
    }
}

fn row_ns(section: &str, name: &str, ns: f64) {
    println!("{section}\t{name}\t{ns:.2}\t");
}

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    let runs: usize = std::env::args().nth(2).map_or(5, |s| s.parse().unwrap());
    let threads: usize = std::env::var("SIMD_THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(1);
    WALL.store(threads > 1, std::sync::atomic::Ordering::Relaxed);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    println!(
        "# スレッド {threads} / 最小の回 / SIMD の道 {} / 列: 区分 名前 1 タイル(256²)µs 4096² ms",
        yolu_core::blend::simd_level_name()
    );
    pool.install(|| {
        #[cfg(target_arch = "x86_64")]
        if std::env::var("SIMD_FTZ").is_ok() {
            #[allow(deprecated)]
            // SAFETY: MXCSR の FTZ・DAZ を立てるだけ（調査用）
            unsafe {
                use std::arch::x86_64::{_mm_getcsr, _mm_setcsr};
                _mm_setcsr(_mm_getcsr() | 0x8040);
            }
        }
        if which == "kernel" {
            kernel_section(runs);
        }
        if which == "all" || which == "blend" {
            blend_section(runs);
        }
        if which == "all" || which == "adjust" {
            adjust_section(runs);
        }
        if which == "all" || which == "filter" {
            filter_section(runs);
        }
        if which == "all" || which == "normal" {
            normal_section(runs);
        }
    });
}
