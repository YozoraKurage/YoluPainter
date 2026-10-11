//! 3D のストロークを塗るペース: 1 フレームに塗りと表示の同期に使ってよい時間（枠）を、溜まった仕事の見込みで決める。
//!
//! 枠が一定（8 ms）だと、大きいブラシ（1 ダブ 3〜4 ms）は 1 フレームに 2 ダブほどしか塗れず、ペンの動きが速く長いと待ち行列が伸び続ける。
//! 描いている間の遅れが伸び、離してから確定までも線の長さに比例して伸びる。そこで、溜まった仕事の見込み（待ちのダブの数 × 1 ダブの
//! 平均の時間）が目標（[`LAG_TARGET`]）を超えたら、超えた分に比例して枠を最大（[`BUDGET_MAX`]）まで広げる。
//!
//! 1 ダブの時間は、塗る時間と、その分の絵を 3D の表示へ上げる同期（変わったタイルの合成・上げ・ミップ）の時間の和で見積もる。同期は塗った
//! ダブの数にほぼ比例して重くなる（4096² の文書で、1 ダブあたり塗りと同じほどになる）ので、塗りだけを枠に収めてもフレームは枠を超える。
//!
//! - 1 フレームの塗り＋同期は、枠が最大でも 40 ms。フレーム全体は、そのほかの約 10 ms（画面の組み立て・描画）を足して約 50 ms
//!   （最初の 1 つのダブが枠より長いときだけ、その分延びる）。
//! - 描いている間の遅れは、塗る力（枠 ÷ (枠 + そのほかのフレームの時間)）が入ってくる仕事の量に届く間は、目標の近く（目標〜目標の 2 倍）に収まる。
//!   そのほかが 10 ms なら、最大の枠で届くのは、入ってくる仕事が 1 秒に 0.8 秒分までの速さ。それを超える速さの入力では、遅れが伸びる
//!   （塗る量を減らす手はない: ダブを飛ばすと結果が変わる）。
//! - 離したあとは入力が無いので、最大の枠で塗る。確定までは、残りの見込み × 1.25（そのほかが 10 ms のとき）。

use std::time::Duration;

/// 枠の最小。溜まった仕事が目標以下のあいだの 1 フレームの塗り＋同期の時間（60 Hz の 1 フレームの半分ほど）。
pub const BUDGET_MIN: Duration = Duration::from_millis(8);
/// 枠の最大。フレーム全体で約 50 ms（そのほかの約 10 ms を足して）になる値。
pub const BUDGET_MAX: Duration = Duration::from_millis(40);
/// 描いている間の遅れ（溜まった仕事の見込み）の目標。
pub const LAG_TARGET: Duration = Duration::from_millis(300);

/// 溜まった仕事の見込み `backlog` から、このフレームに塗り＋同期に使ってよい時間を決める。目標以下なら最小、目標の 2 倍以上なら最大、
/// その間は直線。`released`（離したあと）は、入力が無いので最大。
pub fn frame_budget(backlog: Duration, released: bool) -> Duration {
    if released {
        return BUDGET_MAX;
    }
    if backlog <= LAG_TARGET {
        return BUDGET_MIN;
    }
    let over = (backlog - LAG_TARGET).as_secs_f64() / LAG_TARGET.as_secs_f64();
    BUDGET_MIN + (BUDGET_MAX - BUDGET_MIN).mul_f64(over.min(1.0))
}

/// 1 ダブの平均の時間（塗りと、表示の同期の分）。フレームごとの平均の指数移動平均（場所によってダブの重さが変わる）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DabClock {
    paint: Option<Duration>,
    sync: Option<Duration>,
}

/// 新しいフレームの平均に掛ける重み。
const NEW_WEIGHT: f64 = 0.3;
/// 同期の 1 ダブあたりの時間の上限（描画のドライバーの 1 度きりの待ちで 1 フレームが数百 ミリ秒になっても、見積もりを引きずらない）。
const SYNC_PER_DAB_MAX: Duration = Duration::from_millis(20);

fn blend(old: Option<Duration>, sample: Duration) -> Option<Duration> {
    Some(match old {
        None => sample,
        Some(old) => old.mul_f64(1.0 - NEW_WEIGHT) + sample.mul_f64(NEW_WEIGHT),
    })
}

impl DabClock {
    /// このフレームに `painted` 個を `elapsed` で塗った。何も塗っていなければ変えない。
    pub fn record(&mut self, painted: usize, elapsed: Duration) {
        if painted > 0 {
            self.paint = blend(self.paint, elapsed.div_f64(painted as f64));
        }
    }

    /// このフレームに塗った `painted` 個の絵を、3D の表示へ上げる同期に `sync` かかった。何も塗っていなければ変えない。
    pub fn record_sync(&mut self, painted: usize, sync: Duration) {
        if painted > 0 {
            let per_dab = sync.div_f64(painted as f64).min(SYNC_PER_DAB_MAX);
            self.sync = blend(self.sync, per_dab);
        }
    }

    /// 1 ダブの平均の時間（塗り＋同期。まだ 1 つも塗っていなければ None）。
    fn cost(&self) -> Option<Duration> {
        self.paint.map(|p| p + self.sync.unwrap_or_default())
    }

    /// 待ちのダブが `queued` 個あるときの、溜まった仕事の見込み（まだ 1 つも塗っていなければ 0）。
    pub fn backlog(&self, queued: usize) -> Duration {
        self.cost()
            .map_or(Duration::ZERO, |c| c.mul_f64(queued as f64))
    }

    /// 枠 `frame`（塗り＋同期）のうち、塗りに使う時間（同期の分を引く）。まだ測っていなければ枠のまま。
    pub fn paint_budget(&self, frame: Duration) -> Duration {
        match (self.paint, self.cost()) {
            (Some(paint), Some(cost)) if !cost.is_zero() => {
                frame.mul_f64(paint.as_secs_f64() / cost.as_secs_f64())
            }
            _ => frame,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: f64) -> Duration {
        Duration::from_secs_f64(v / 1000.0)
    }

    #[test]
    fn the_budget_is_the_minimum_up_to_the_target_and_grows_to_the_maximum_at_twice_the_target() {
        assert_eq!(frame_budget(Duration::ZERO, false), BUDGET_MIN);
        assert_eq!(frame_budget(LAG_TARGET, false), BUDGET_MIN);
        let mid = frame_budget(ms(450.0), false);
        assert!(
            (mid.as_secs_f64() - 0.024).abs() < 1e-9,
            "目標の 1.5 倍で枠の真ん中: {mid:?}"
        );
        assert_eq!(frame_budget(ms(600.0), false), BUDGET_MAX);
        assert_eq!(frame_budget(Duration::from_secs(60), false), BUDGET_MAX);
        // 単調
        let mut last = Duration::ZERO;
        for k in 0..=800 {
            let b = frame_budget(ms(k as f64), false);
            assert!(b >= last && b >= BUDGET_MIN && b <= BUDGET_MAX, "{k}");
            last = b;
        }
        // 離したあとは、溜まりが無くても最大
        assert_eq!(frame_budget(Duration::ZERO, true), BUDGET_MAX);
    }

    #[test]
    fn the_clock_averages_the_dabs_per_frame_and_estimates_the_backlog() {
        let mut clock = DabClock::default();
        assert_eq!(clock.backlog(100), Duration::ZERO, "まだ塗っていない");
        assert_eq!(clock.paint_budget(ms(20.0)), ms(20.0), "測るまでは枠のまま");
        clock.record(0, ms(5.0));
        clock.record_sync(0, ms(5.0));
        assert_eq!(
            clock.backlog(100),
            Duration::ZERO,
            "塗らなかったフレームは数えない"
        );
        clock.record(2, ms(8.0));
        assert_eq!(clock.backlog(10), ms(40.0));
        // 同期の分も見込みに入り、枠のうち塗りに使うのは塗りの割合だけ
        clock.record_sync(2, ms(4.0));
        assert_eq!(clock.backlog(10), ms(60.0));
        assert_eq!(
            clock.paint_budget(ms(30.0)),
            ms(20.0),
            "1 ダブ 6 ms のうち塗りが 4 ms"
        );
        // 重いダブが続くと、見込みが追いつく
        for _ in 0..30 {
            clock.record(1, ms(10.0));
        }
        let b = clock.backlog(10);
        assert!((b.as_secs_f64() - 0.12).abs() < 0.001, "{b:?}");
        // 同期の 1 度きりの長い待ちは、1 ダブあたりの上限で抑える
        let mut stalled = DabClock::default();
        stalled.record(1, ms(4.0));
        stalled.record_sync(1, ms(1300.0));
        assert!(
            stalled.backlog(1) <= ms(24.0) + Duration::from_micros(1),
            "{:?}",
            stalled.backlog(1)
        );
    }

    /// 入ってくる仕事の速さ（1 秒に作業が何秒分来るか）`rate` で、大きいブラシ（1 ダブ 塗り 3.5 ms + 同期 1.5 ms）を引き続けたときのペースの
    /// 見立て（そのほかのフレームを 10 ms と置く）。(最大の遅れ ms, 最長のフレーム ms, 離してから空になるまで ms) を返す。
    fn simulate(rate: f64, seconds: f64) -> (f64, f64, f64) {
        let (paint, sync, base) = (3.5, 1.5, 10.0);
        let mut clock = DabClock::default();
        let (mut queue, mut now, mut worst_lag, mut worst_frame) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        let mut incoming = 0.0f64;
        let mut step = |queue: &mut f64, now: &mut f64, released: bool, incoming: &mut f64| {
            let backlog = clock.backlog(queue.round() as usize);
            let budget = frame_budget(backlog, released);
            let paint_budget = clock.paint_budget(budget).as_secs_f64() * 1000.0;
            // 最初の 1 つは必ず塗り、次の 1 つが枠に収まるときだけ続ける
            let mut painted = 0usize;
            let mut spent = 0.0;
            while *queue >= 1.0 && (painted == 0 || spent + paint <= paint_budget) {
                *queue -= 1.0;
                painted += 1;
                spent += paint;
            }
            clock.record(painted, ms(spent));
            clock.record_sync(painted, ms(sync * painted as f64));
            let frame = base + spent + sync * painted as f64;
            *now += frame;
            if !released {
                *incoming += rate * frame / (paint + sync);
                *queue += incoming.floor();
                *incoming -= incoming.floor();
            }
            (backlog.as_secs_f64() * 1000.0, frame)
        };
        queue += 1.0;
        while now < seconds * 1000.0 {
            let (lag, frame) = step(&mut queue, &mut now, false, &mut incoming);
            worst_lag = worst_lag.max(lag);
            worst_frame = worst_frame.max(frame);
        }
        let release_at = now;
        let mut guard = 0;
        while queue >= 1.0 && guard < 100_000 {
            let (_, frame) = step(&mut queue, &mut now, true, &mut incoming);
            worst_frame = worst_frame.max(frame);
            guard += 1;
        }
        (worst_lag, worst_frame, now - release_at)
    }

    #[test]
    fn the_lag_stays_near_the_target_and_the_release_finishes_quickly_for_speeds_the_budget_can_keep_up_with(
    ) {
        // 塗る力は 枠 ÷ (枠 + 10 ms): 最小で 0.44、最大で 0.8。入ってくる仕事がそれ以下なら、遅れは目標の近くに収まる
        for rate in [0.2, 0.4, 0.6, 0.7] {
            let (lag, frame, settle) = simulate(rate, 20.0);
            assert!(frame <= 10.0 + 40.0 + 5.0, "{rate}: 1 フレーム {frame} ms");
            assert!(lag <= 650.0, "{rate}: 遅れ {lag} ms");
            assert!(settle <= 800.0, "{rate}: 離してから {settle} ms");
        }
        let (lag, _, settle) = simulate(0.6, 20.0);
        assert!(lag <= 450.0 && settle <= 650.0, "{lag} {settle}");
    }

    #[test]
    fn a_speed_beyond_the_maximum_budget_makes_the_lag_grow_without_dropping_dabs() {
        let (lag_short, frame, _) = simulate(1.5, 5.0);
        let (lag_long, _, settle) = simulate(1.5, 20.0);
        assert!(frame <= 10.0 + 40.0 + 5.0, "1 フレームは上限のまま {frame}");
        assert!(
            lag_long > lag_short && lag_long > 1000.0,
            "{lag_short} {lag_long}"
        );
        assert!(
            settle > 1000.0,
            "確定までは線の長さに比例して伸びる: {settle}"
        );
    }
}
