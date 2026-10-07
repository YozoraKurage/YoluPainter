//! ストロークの速さを測る共通の部品（`stroke_bench` と、取り込んだブラシを測る台が使う）。
//!
//! 文書は 4096²（タイル 128）。決まった点の列（太さに応じた長さの正弦波の線、点は約 12 画素ごと）を同じ手順で描き、
//! 1 回ごとに新しい文書で測る。時間は、スレッドが 1 本のときはそのスレッドの CPU 時間（ほかの負荷で待たされた分を除く）、
//! 複数のときは壁時計。
#![allow(dead_code)]

use std::sync::OnceLock;
use std::time::Instant;

use yolu_core::glam::DVec2;
use yolu_core::{Brush, Channel, Document, TileCoord};

pub const CANVAS: u32 = 4096;

/// 入力の点どうしの間隔（画素。ペンの 1 イベントがおよそこの距離を進む）。
pub const INPUT_STEP: f64 = 12.0;

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
    pub fn now_ms() -> Option<f64> {
        const CLOCK_THREAD_CPUTIME_ID: i32 = 3;
        let mut ts = Timespec { sec: 0, nsec: 0 };
        // SAFETY: ts は書き込める Timespec（Linux の struct timespec と同じ並び）
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

static WALL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// 1 本のスレッドで描くとき（rayon のスレッドが 1）は CPU 時間で測る。
pub fn use_cpu_time(on: bool) {
    WALL.store(!on, std::sync::atomic::Ordering::Relaxed);
}

/// 測る時刻（ミリ秒）。
pub fn clock_ms() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    if !WALL.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(t) = thread_time::now_ms() {
            return t;
        }
    }
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// 入力の点の列（画素の座標）。太さが大きいほど長く（点の数は 120〜300）。
pub fn path(size: f64) -> Vec<(f64, f64)> {
    let length = (size * 10.0).clamp(1500.0, 3600.0);
    let n = (length / INPUT_STEP).round() as usize;
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            (
                250.0 + length * t,
                2048.0 + 400.0 * (t * std::f64::consts::TAU * 1.5).sin(),
            )
        })
        .collect()
}

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

/// 全タイルの乱数の画素（1 回だけ作って、文書ごとに読み込む）。
fn random_tiles(ts: usize) -> &'static Vec<Vec<u8>> {
    static TILES: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    TILES.get_or_init(|| {
        let mut rng = SplitMix(3);
        let n = (CANVAS as usize / ts) * (CANVAS as usize / ts);
        (0..n)
            .map(|_| {
                let mut bytes = vec![0u8; ts * ts * 4];
                for p in bytes.as_chunks_mut::<4>().0 {
                    p.copy_from_slice(&[rng.channel(), rng.channel(), rng.channel(), rng.alpha()]);
                }
                bytes
            })
            .collect()
    })
}

/// 4096² の文書と描くレイヤー。filled なら全タイルが乱数の画素。
pub fn new_document(filled: bool) -> (Document, yolu_core::LayerId) {
    let mut doc = Document::new(CANVAS, CANVAS).unwrap();
    // 既定（64 MiB）では、1000 画素の筆の混ぜる・指先が読み元の枠で予算を超える。測るのは時間なので、予算は広げる
    doc.set_stroke_budget_bytes(512 << 20).unwrap();
    let layer = doc.add_layer("a").unwrap();
    if filled {
        let ts = doc.tile_size() as usize;
        let per_row = CANVAS as usize / ts;
        let tiles = random_tiles(ts);
        for (i, bytes) in tiles.iter().enumerate() {
            let coord = TileCoord::new((i % per_row) as u32, (i / per_row) as u32);
            doc.import_tile(layer, Channel::Color, coord, bytes)
                .unwrap();
        }
        doc.clear_history().unwrap();
    }
    (doc, layer)
}

/// 1 回のストロークの時間（ミリ秒）。
#[derive(Clone, Debug, Default)]
pub struct Run {
    pub total: f64,
    /// 入力の点を足した時間の合計。
    pub add: f64,
    /// 確定（待たせたダブ・Undo の記録）。
    pub end: f64,
    /// 1 入力ごとの時間（足した順）。
    pub per_input: Vec<f64>,
    pub stamps: u64,
    pub inputs: usize,
    pub parallel_dabs: u64,
    pub tiles: usize,
    pub rollback_mib: f64,
    pub changed: bool,
}

pub fn run_once(brush: &Brush, filled: bool, points: &[(f64, f64)], pressure: f64) -> Run {
    let (mut doc, layer) = new_document(filled);
    let mut run = Run::default();
    let t0 = clock_ms();
    let mut stroke = doc.begin_brush_stroke(layer, brush).unwrap();
    let mut last = clock_ms();
    for &(x, y) in points {
        stroke
            .add_point(&mut doc, x, y, pressure, DVec2::ZERO)
            .unwrap();
        let now = clock_ms();
        run.per_input.push(now - last);
        last = now;
    }
    run.add = last - t0;
    let stats = doc.active_stroke_stats().unwrap();
    run.parallel_dabs = stats.parallel_dabs;
    run.tiles = stats.tiles;
    run.rollback_mib = stats.rollback_bytes as f64 / (1024.0 * 1024.0);
    let t1 = clock_ms();
    let r = doc.end_stroke(stroke).unwrap();
    let t2 = clock_ms();
    run.end = t2 - t1;
    run.total = t2 - t0;
    run.stamps = r.stamps;
    run.inputs = r.samples as usize;
    run.changed = r.changed;
    std::hint::black_box(&doc);
    run
}

/// 繰り返しの結果（最小の回を代表にする）。
pub struct Stats {
    pub best: Run,
    pub median_total: f64,
    pub runs: usize,
}

/// 1 回目（捨てる）の時間から回数を決める: 0.8 秒を目安に 3〜9 回（1 回が 0.8 秒を超えるなら 1 回目だけ）。
pub fn measure(
    brush: &Brush,
    filled: bool,
    points: &[(f64, f64)],
    pressure: f64,
    max_runs: usize,
) -> Stats {
    let first = run_once(brush, filled, points, pressure);
    let mut all = vec![first];
    let t = all[0].total.max(0.01);
    let extra = if t > 800.0 {
        0
    } else {
        ((800.0 / t) as usize).clamp(2, max_runs.max(2))
    };
    for _ in 0..extra {
        all.push(run_once(brush, filled, points, pressure));
    }
    let mut totals: Vec<f64> = all.iter().map(|r| r.total).collect();
    totals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_total = totals[totals.len() / 2];
    let runs = all.len();
    // 1 回目は捨てる（ただし 1 回しか測らなかったときは使う）
    let pool: Vec<Run> = if runs > 1 {
        all.into_iter().skip(1).collect()
    } else {
        all
    };
    let best = pool
        .into_iter()
        .min_by(|a, b| a.total.partial_cmp(&b.total).unwrap())
        .unwrap();
    Stats {
        best,
        median_total,
        runs,
    }
}

pub fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s[(((s.len() - 1) as f64) * p).round() as usize]
}

/// 表の 1 行（TSV）の見出し。
pub fn header() -> &'static str {
    "ブラシ\t大きさ\t間隔\tレイヤー\tダブ\t入力\t全体ms\t中央ms\t1ダブus\t1入力us\t入力p95us\t入力最長us\t確定ms\t並列ダブ\t回数"
}

pub fn row(label: &str, size: f64, spacing: f64, filled: bool, s: &Stats) -> String {
    let r = &s.best;
    let dabs = r.stamps.max(1) as f64;
    let inputs = r.inputs.max(1) as f64;
    format!(
        "{label}\t{size}\t{spacing}\t{}\t{}\t{}\t{:.2}\t{:.2}\t{:.2}\t{:.1}\t{:.1}\t{:.1}\t{:.2}\t{}\t{}",
        if filled { "塗済" } else { "空" },
        r.stamps,
        r.inputs,
        r.total,
        s.median_total,
        r.total * 1000.0 / dabs,
        r.total * 1000.0 / inputs,
        percentile(&r.per_input, 0.95) * 1000.0,
        r.per_input.iter().cloned().fold(0.0, f64::max) * 1000.0,
        r.end,
        r.parallel_dabs,
        s.runs
    )
}

/// 段ごとの時間（`--features stroke-profile` のときだけ）。1 回描いて、各段の合計と 1 回あたりを出す。
#[cfg(feature = "stroke-profile")]
pub fn stages(brush: &Brush, filled: bool, points: &[(f64, f64)]) {
    use yolu_core::brush::profile;
    profile::reset();
    let run = run_once(brush, filled, points, 1.0);
    let totals = profile::take();
    let mut line = format!(
        "  段\t全体 {:.2} ms（入力 {:.2} 確定 {:.2}）",
        run.total, run.add, run.end
    );
    for (stage, t) in totals {
        if t.calls > 0 {
            line.push_str(&format!(
                "\t{} {:.2} ms（{} 回・1 回 {:.2} us）",
                stage.name(),
                t.nanos / 1e6,
                t.calls,
                t.nanos / 1e3 / t.calls as f64
            ));
        }
    }
    println!("{line}");
}
