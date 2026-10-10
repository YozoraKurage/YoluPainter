//! ストロークの段ごとの時間の計測（cargo の feature `stroke-profile` のときだけ中身がある。無いときは何も残らない）。
//!
//! 段の入口で [`scope`] を呼び、返った札を捨てると（スコープの終わり）、その間の時間と回数を段ごとに足す。時間は TSC（x86_64）か
//! `Instant`。段は入れ子になり得る（`Stamp` は `Prepare` と `Pixels` を含み、`Pixels` は `FirstTouch` を含む）。ワーカーで描く
//! ダブの `FirstTouch` は各ワーカーの時間の合計（壁時計ではなく CPU の合計）。計測の台（`examples/stroke_bench.rs`）が
//! `--features stroke-profile` のときに、[`reset`] と [`take`] で読む。

/// 計測する段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Stage {
    /// 1 つの描点（`stamp`）の全体: ダブの位置・ゆらぎ・乱数・形の決定と、その描点のダブ。
    Stamp = 0,
    /// ダブの前に読み元を凍結する仕事（混ぜ・指先・ぼかし・クローンの枠。塗るだけのブラシは空）。
    Prepare,
    /// ダブの画素（タイルの出し入れ・覆いの計算・画素への当て。ワーカーを待つ時間も含む）。
    Pixels,
    /// タイルを初めて触るときの巻き戻しの写し（覆いの確保と、描く前のタイルの写し）。
    FirstTouch,
    /// デュアルブラシの 2 つ目の筆先のダブ。
    Dual,
    /// 1 枚のタイルの画素の核（`dab_tile`。タイルの出し入れを除く）。
    Kernel,
    /// 3D のダブ 1 つ（`SurfaceStroke` の `paint_plan` の全体: 投影の塗りで画素を集めるところから、文書のストロークへ塗るところまで）。
    SurfaceDab,
    /// 3D のダブの投影の塗り（`build_dab`: 区画の作り直し・覆いの集め・並べ替え・対称の写し）。
    SurfaceProject,
    /// 投影の塗りのうち、覚えに無い区画を作る仕事（`build_bucket`。ワーカーを待つ時間も含む）。
    SurfaceBuckets,
    /// 投影の塗りのうち、円の中の投影の画素の覆いを出して並べる仕事（`gather`・区画ごとの位置順の列の併合・同じテクセルの 1 本化）。
    SurfaceGather,
    /// 3D のダブの画素を、文書のストロークへまとめて渡して塗る仕事（`Stroke::apply_dab`。渡す画素の並びの組み立ても含む）。
    SurfaceApply,
}

impl Stage {
    pub const ALL: [Stage; 11] = [
        Stage::Stamp,
        Stage::Prepare,
        Stage::Pixels,
        Stage::FirstTouch,
        Stage::Dual,
        Stage::Kernel,
        Stage::SurfaceDab,
        Stage::SurfaceProject,
        Stage::SurfaceBuckets,
        Stage::SurfaceGather,
        Stage::SurfaceApply,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Stage::Stamp => "描点全体",
            Stage::Prepare => "読み元の凍結",
            Stage::Pixels => "画素（覆い+当て）",
            Stage::FirstTouch => "初回の写し",
            Stage::Dual => "デュアルの溜まり",
            Stage::Kernel => "画素の核（タイル 1 枚分）",
            Stage::SurfaceDab => "3D ダブ全体",
            Stage::SurfaceProject => "3D 投影の塗り",
            Stage::SurfaceBuckets => "3D 区画の作り直し",
            Stage::SurfaceGather => "3D 覆いの集め+並べ替え",
            Stage::SurfaceApply => "3D のキャンバスへ塗る",
        }
    }
}

/// 段ごとの合計。
#[derive(Clone, Copy, Debug, Default)]
pub struct Total {
    pub nanos: f64,
    pub calls: u64,
}

#[cfg(feature = "stroke-profile")]
mod on {
    use super::{Stage, Total};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::Instant;

    const N: usize = Stage::ALL.len();
    static TICKS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
    static CALLS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
    static START: Mutex<Option<(Instant, u64)>> = Mutex::new(None);
    /// 3D の段は 1 回ごとの時間も残す（中央値・最大を出すため。1 回が ms の単位なので、ロックの時間は無視できる）。
    static SAMPLES: [Mutex<Vec<u64>>; N] = [const { Mutex::new(Vec::new()) }; N];

    #[inline(always)]
    fn now() -> u64 {
        #[cfg(target_arch = "x86_64")]
        {
            // SAFETY: rdtsc は x86_64 のどの CPU にもある
            unsafe { core::arch::x86_64::_rdtsc() }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            ORIGIN.get_or_init(Instant::now).elapsed().as_nanos() as u64
        }
    }

    pub struct Guard {
        stage: usize,
        start: u64,
    }

    #[inline(always)]
    pub fn scope(stage: Stage) -> Guard {
        Guard {
            stage: stage as usize,
            start: now(),
        }
    }

    impl Drop for Guard {
        #[inline(always)]
        fn drop(&mut self) {
            let dt = now().wrapping_sub(self.start);
            TICKS[self.stage].fetch_add(dt, Ordering::Relaxed);
            CALLS[self.stage].fetch_add(1, Ordering::Relaxed);
            if self.stage >= Stage::SurfaceDab as usize {
                SAMPLES[self.stage].lock().unwrap().push(dt);
            }
        }
    }

    pub fn reset() {
        for i in 0..N {
            TICKS[i].store(0, Ordering::Relaxed);
            CALLS[i].store(0, Ordering::Relaxed);
            SAMPLES[i].lock().unwrap().clear();
        }
        *START.lock().unwrap() = Some((Instant::now(), now()));
    }

    /// 段の 1 回ごとの時間（ナノ秒、短い順。3D の段だけ。[`take`] の前に読む）。
    pub fn samples(stage: Stage) -> Vec<f64> {
        let (t0, c0) = START.lock().unwrap().expect("reset の後");
        let nanos_per_tick =
            t0.elapsed().as_secs_f64() * 1e9 / now().wrapping_sub(c0).max(1) as f64;
        let mut v: Vec<f64> = SAMPLES[stage as usize]
            .lock()
            .unwrap()
            .iter()
            .map(|&t| t as f64 * nanos_per_tick)
            .collect();
        v.sort_by(f64::total_cmp);
        v
    }

    pub fn take() -> Vec<(Stage, Total)> {
        let (t0, c0) = START.lock().unwrap().take().expect("reset の後");
        let seconds = t0.elapsed().as_secs_f64();
        let ticks = now().wrapping_sub(c0) as f64;
        let nanos_per_tick = seconds * 1e9 / ticks.max(1.0);
        Stage::ALL
            .iter()
            .map(|&s| {
                (
                    s,
                    Total {
                        nanos: TICKS[s as usize].load(Ordering::Relaxed) as f64 * nanos_per_tick,
                        calls: CALLS[s as usize].load(Ordering::Relaxed),
                    },
                )
            })
            .collect()
    }
}

#[cfg(feature = "stroke-profile")]
pub use on::{reset, samples, scope, take, Guard};

#[cfg(not(feature = "stroke-profile"))]
mod off {
    use super::Stage;
    pub struct Guard;
    #[inline(always)]
    pub fn scope(_: Stage) -> Guard {
        Guard
    }
}
#[cfg(not(feature = "stroke-profile"))]
pub(crate) use off::scope;

#[cfg(all(test, feature = "stroke-profile"))]
mod tests {
    use super::*;
    use crate::{Brush, BrushSettings, Document, Rgba8};
    use glam::DVec2;

    #[test]
    fn a_stroke_adds_time_and_calls_to_every_stage_it_passes() {
        reset();
        let mut doc = Document::new(128, 128).unwrap();
        let layer = doc.add_layer("a").unwrap();
        let brush = Brush::from(BrushSettings {
            radius: 8.0,
            color: Rgba8::new(10, 20, 30, 255),
            ..BrushSettings::default()
        });
        let mut stroke = doc.begin_brush_stroke(layer, &brush).unwrap();
        for i in 0..20 {
            stroke
                .add_point(&mut doc, 10.0 + 5.0 * i as f64, 40.0, 1.0, DVec2::ZERO)
                .unwrap();
        }
        let stamps = doc.end_stroke(stroke).unwrap().stamps;
        let totals = take();
        let of = |stage: Stage| totals.iter().find(|(s, _)| *s == stage).unwrap().1;
        assert!(of(Stage::Stamp).calls >= stamps, "描点ごとに数える");
        assert!(of(Stage::Pixels).calls > 0 && of(Stage::Pixels).nanos > 0.0);
        assert!(of(Stage::Kernel).calls > 0);
        assert!(of(Stage::FirstTouch).calls > 0, "最初のタイルの写し");
        assert_eq!(of(Stage::Dual).calls, 0, "デュアルでないブラシは通らない");
    }
}
