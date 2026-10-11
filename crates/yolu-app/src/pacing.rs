//! フレームの間隔の下限（垂直同期を待たない表示のとき。設定「垂直同期」が切のとき）。
//!
//! 垂直同期を待つ表示は、画面に出す呼び出しが次の垂直同期まで止まるので、フレームの数は画面の更新の数に収まる。待たない表示
//! （`PresentMode::AutoNoVsync`。`view3d::render::surface_config`）は止まらないので、描き直しを毎フレーム頼み続ける場所（慣性・進み具合の表示など。
//! `request_repaint()` を毎フレーム呼ぶ所）や、1000 Hz のマウスの動きがあると、フレームが上限なしに回り、CPU と GPU を使い切る。
//! そこで、フレームの初め（`eframe::App::raw_input_hook`。egui のパスの前）に、前のフレームの始まりからの間が下限より短ければ、残りを眠る。
//!
//! 下限は 2 つ。入力のあるフレーム（主のウィンドウの egui の事象・ペンの待ち行列。別ウィンドウは下の注）は [`INPUT_INTERVAL`] = 1/240 秒。
//! 入力の無いフレーム（描き直しの頼みだけ）は、ウィンドウのあるモニターのリフレッシュレートの逆数（[`repaint_interval`]。60〜240 Hz に収める）で、
//! 読めないとき（Windows 以外・読めなかったとき）は [`REPAINT_INTERVAL`] = 1/120 秒。垂直同期を待っていたときの、モニターの回数だけ描き直す動きに合わせる。
//! eframe はモニターのリフレッシュレートを渡さないので、Windows だけウィンドウのあるモニターから読む（`refresh`。1 秒に 1 度まで読み直す）。
//! 止まっていたあとの最初のフレームは、前の始まりから間が空いているので眠らない。
//!
//! 別ウィンドウの egui の事象は、主のフレームの初めには見えない（別ウィンドウの入力は、主のパスの中でそのウィンドウのパスが取る）。見えるのは
//! 別ウィンドウのペンの待ち行列と、前のフレームに別ウィンドウのパスが入力を見たかの印（[`Pacing::note_detached_input`]）で、別ウィンドウだけを操作している最初の
//! 1 フレームは「入力の無いフレーム」（下限が入力の無いフレームの値）に落ちる。そのとき増える遅れは、入力の無いフレームの下限 − 1/240 秒（120 Hz なら約 4.2 ミリ秒）で、
//! 次のフレームからは印で 1/240 秒になる。
//!
//! 眠る長さは純粋な関数 [`sleep_for`] で決める。待つ形のときは眠らない。

#[cfg(windows)]
pub mod refresh;

use std::time::{Duration, Instant};

/// 入力のあるフレームの間隔の下限（1/240 秒）。
pub const INPUT_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 240);
/// 入力の無いフレーム（描き直しの頼みだけ）の間隔の下限のうち、モニターのリフレッシュレートが読めないときの値（1/120 秒）。
pub const REPAINT_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 120);
/// リフレッシュレートから決める、入力の無いフレームの下限の範囲（Hz）。
pub const MIN_REFRESH_HZ: f64 = 60.0;
pub const MAX_REFRESH_HZ: f64 = 240.0;
/// リフレッシュレートを読み直す間隔の下限（モニターをまたいだ・表示の設定が変わったあとに追いつく）。
pub const REFRESH_RECHECK: Duration = Duration::from_secs(1);

/// 入力の無いフレームの間隔の下限: モニターのリフレッシュレート（Hz）の逆数。60〜240 Hz に収める（60 Hz より低いモニターも、240 Hz より高いモニターも、
/// 両端の値）。読めない（`None`）・数でない・0 以下は [`REPAINT_INTERVAL`]（1/120 秒）。
pub fn repaint_interval(refresh_hz: Option<f64>) -> Duration {
    match refresh_hz {
        Some(hz) if hz.is_finite() && hz > 0.0 => {
            Duration::from_secs_f64(1.0 / hz.clamp(MIN_REFRESH_HZ, MAX_REFRESH_HZ))
        }
        _ => REPAINT_INTERVAL,
    }
}

/// 眠る長さ: 前のフレームの始まりから `since_last_start` が経っていて、このフレームの入力が `input` のとき、下限に足りない分。
/// 入力の無いフレームの下限は `repaint_interval`（`repaint_interval(モニターのリフレッシュレート)`）。
/// 待つ形（`waits_for_vsync`）・初めてのフレーム（`since_last_start` が無い）・間が下限以上のときは 0。
pub fn sleep_for(
    waits_for_vsync: bool,
    since_last_start: Option<Duration>,
    input: bool,
    repaint_interval: Duration,
) -> Duration {
    if waits_for_vsync {
        return Duration::ZERO;
    }
    let Some(since) = since_last_start else {
        return Duration::ZERO;
    };
    interval_for(input, repaint_interval).saturating_sub(since)
}

/// `DEVMODEW::dmDisplayFrequency`（Hz）をリフレッシュレートにする。0 と 1 は「モニターの既定」で周波数ではないので読めない扱い。
pub fn refresh_hz_from_display_frequency(frequency: u32) -> Option<f64> {
    (frequency > 1).then_some(f64::from(frequency))
}

/// 読めたリフレッシュレートのうち、いちばん速いもの（どれも読めなければ `None`）。
pub fn fastest(rates: impl IntoIterator<Item = Option<f64>>) -> Option<f64> {
    rates
        .into_iter()
        .flatten()
        .filter(|hz| hz.is_finite())
        .max_by(f64::total_cmp)
}

/// このフレームの間隔の下限: 入力があれば [`INPUT_INTERVAL`]、無ければ `repaint_interval`。
pub fn interval_for(input: bool, repaint_interval: Duration) -> Duration {
    if input {
        INPUT_INTERVAL
    } else {
        repaint_interval
    }
}

/// フレームの間隔の下限を守る係。
#[derive(Clone, Debug)]
pub struct Pacing {
    /// 垂直同期を待つ形か（待つ形では何もしない。試験と、設定が待つ形のとき）。
    waits_for_vsync: bool,
    /// 前のフレームの始まり（眠ったあと、egui のパスを始めた時刻）。
    last_start: Option<Instant>,
    /// 前のフレームの別ウィンドウのパスが、入力（egui の事象・ペンの点）を見た。
    detached_input: bool,
    /// 入力の無いフレームの間隔の下限（モニターのリフレッシュレートの逆数。読めなければ 1/120 秒）。
    repaint_interval: Duration,
    /// 最後にフレームの間隔として選んだ下限（待つ形では選ばない）。試験が、入力のあるフレームに 1/240 秒を選んだことを見る。
    last_interval: Option<Duration>,
    /// リフレッシュレートを最後に読んだ時刻。
    refresh_read_at: Option<Instant>,
}

impl Default for Pacing {
    /// 何もしない形（待つ形と同じ）。
    fn default() -> Self {
        Pacing::new(true)
    }
}

impl Pacing {
    pub fn new(waits_for_vsync: bool) -> Pacing {
        Pacing {
            waits_for_vsync,
            last_start: None,
            detached_input: false,
            repaint_interval: REPAINT_INTERVAL,
            last_interval: None,
            refresh_read_at: None,
        }
    }

    /// 垂直同期を待つ形か。
    pub fn waits_for_vsync(&self) -> bool {
        self.waits_for_vsync
    }

    /// 別ウィンドウのパスが入力を見た（次のフレームの初めが、入力のあるフレームとして数える）。
    pub fn note_detached_input(&mut self) {
        self.detached_input = true;
    }

    /// 別ウィンドウの入力の印を取り出して消す。
    pub fn take_detached_input(&mut self) -> bool {
        std::mem::take(&mut self.detached_input)
    }

    /// モニターのリフレッシュレート（Hz。読めなければ `None`）から、入力の無いフレームの下限を決める（[`repaint_interval`]）。
    pub fn set_refresh_rate(&mut self, refresh_hz: Option<f64>) {
        self.repaint_interval = repaint_interval(refresh_hz);
    }

    /// 入力の無いフレームの今の下限。
    pub fn repaint_interval(&self) -> Duration {
        self.repaint_interval
    }

    /// リフレッシュレートを読み直す頃か（前に読んでから [`REFRESH_RECHECK`] 経った、または一度も読んでいない）。そうなら読んだ時刻を `now` にして true を返す。
    pub fn refresh_due(&mut self, now: Instant) -> bool {
        let due = self
            .refresh_read_at
            .is_none_or(|at| now.saturating_duration_since(at) >= REFRESH_RECHECK);
        if due {
            self.refresh_read_at = Some(now);
        }
        due
    }

    /// 最後にフレームの間隔として選んだ下限（待つ形・まだフレームが無いときは `None`）。
    pub fn last_interval(&self) -> Option<Duration> {
        self.last_interval
    }

    /// 時刻 `now` で、入力が `input` のフレームを始めるとき、眠る長さ。
    pub fn plan(&self, now: Instant, input: bool) -> Duration {
        let since = self.last_start.map(|t| now.saturating_duration_since(t));
        sleep_for(self.waits_for_vsync, since, input, self.repaint_interval)
    }

    /// フレームの始まりを覚える（眠ったあとの時刻）。
    pub fn started(&mut self, at: Instant) {
        self.last_start = Some(at);
    }

    /// 下限に足りない分を眠り、フレームの始まりを覚える。実際に眠った長さを返す（眠らなかったときは 0）。
    pub fn wait(&mut self, input: bool) -> Duration {
        let before = Instant::now();
        if !self.waits_for_vsync {
            self.last_interval = Some(interval_for(input, self.repaint_interval));
        }
        let sleep = self.plan(before, input);
        if sleep.is_zero() {
            self.started(before);
            return Duration::ZERO;
        }
        std::thread::sleep(sleep);
        let after = Instant::now();
        self.started(after);
        after.saturating_duration_since(before)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;
    const US: fn(u64) -> Duration = Duration::from_micros;

    #[test]
    fn the_input_interval_is_one_240th_and_the_unread_repaint_interval_one_120th() {
        assert_eq!(INPUT_INTERVAL, Duration::from_nanos(4_166_666));
        assert_eq!(REPAINT_INTERVAL, Duration::from_nanos(8_333_333));
        assert!(INPUT_INTERVAL < REPAINT_INTERVAL);
    }

    #[test]
    fn the_repaint_interval_follows_the_monitor_refresh_rate_within_60_to_240_hz() {
        let close = |a: Duration, hz: f64| {
            let want = 1.0 / hz;
            (a.as_secs_f64() - want).abs() < 1e-9
        };
        // 144 Hz・60 Hz: その逆数
        assert!(close(repaint_interval(Some(144.0)), 144.0));
        assert!(close(repaint_interval(Some(60.0)), 60.0));
        assert!(close(repaint_interval(Some(165.0)), 165.0));
        assert!(close(repaint_interval(Some(240.0)), 240.0));
        // 読めない: 1/120 秒
        assert_eq!(repaint_interval(None), REPAINT_INTERVAL);
        // 数でない・0 以下も読めない扱い
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -60.0] {
            assert_eq!(repaint_interval(Some(bad)), REPAINT_INTERVAL, "{bad}");
        }
        // 範囲の外は両端
        assert!(close(repaint_interval(Some(30.0)), 60.0));
        assert!(close(repaint_interval(Some(1.0)), 60.0));
        assert!(close(repaint_interval(Some(360.0)), 240.0));
        assert!(close(repaint_interval(Some(1000.0)), 240.0));
        // 速いモニターほど下限が短い
        assert!(repaint_interval(Some(144.0)) < repaint_interval(Some(60.0)));
        // 入力のあるフレームの下限（1/240 秒）より短くはならない
        assert!(repaint_interval(Some(1000.0)) >= INPUT_INTERVAL);
    }

    #[test]
    fn the_display_frequency_zero_and_one_mean_the_default_and_are_not_read() {
        assert_eq!(refresh_hz_from_display_frequency(0), None);
        assert_eq!(refresh_hz_from_display_frequency(1), None);
        assert_eq!(refresh_hz_from_display_frequency(60), Some(60.0));
        assert_eq!(refresh_hz_from_display_frequency(144), Some(144.0));
        assert_eq!(refresh_hz_from_display_frequency(239), Some(239.0));
    }

    #[test]
    fn the_fastest_of_the_windows_monitors_is_used_and_unread_ones_are_skipped() {
        assert_eq!(fastest([Some(60.0), Some(144.0), Some(75.0)]), Some(144.0));
        assert_eq!(fastest([None, Some(60.0), None]), Some(60.0));
        assert_eq!(fastest([None, None]), None);
        assert_eq!(fastest(std::iter::empty()), None);
        assert_eq!(fastest([Some(f64::NAN), Some(60.0)]), Some(60.0));
    }

    #[test]
    fn it_sleeps_the_remainder_of_the_interval_for_the_kind_of_frame() {
        // 入力のあるフレーム: 1/240 秒に足りない分（入力の無いフレームの下限によらない）
        for repaint in [repaint_interval(Some(60.0)), REPAINT_INTERVAL] {
            assert_eq!(
                sleep_for(false, Some(US(1_000)), true, repaint),
                INPUT_INTERVAL - US(1_000)
            );
        }
        // 入力の無いフレーム: その下限に足りない分
        assert_eq!(
            sleep_for(false, Some(US(1_000)), false, REPAINT_INTERVAL),
            REPAINT_INTERVAL - US(1_000)
        );
        let at_60 = repaint_interval(Some(60.0));
        assert_eq!(
            sleep_for(false, Some(US(1_000)), false, at_60),
            at_60 - US(1_000)
        );
        // 同じ間でも、入力の無いフレームのほうが長く眠る
        assert!(
            sleep_for(false, Some(MS(2)), false, REPAINT_INTERVAL)
                > sleep_for(false, Some(MS(2)), true, REPAINT_INTERVAL)
        );
        // 間が 0（同じ時刻）なら、間隔そのもの
        assert_eq!(
            sleep_for(false, Some(Duration::ZERO), true, REPAINT_INTERVAL),
            INPUT_INTERVAL
        );
        assert_eq!(
            sleep_for(false, Some(Duration::ZERO), false, REPAINT_INTERVAL),
            REPAINT_INTERVAL
        );
    }

    #[test]
    fn it_does_not_sleep_when_the_gap_is_long_enough_or_the_frame_is_the_first() {
        let repaint = REPAINT_INTERVAL;
        // 止まっていたあとの最初のフレーム（間が空いている）
        assert_eq!(
            sleep_for(false, Some(MS(500)), true, repaint),
            Duration::ZERO
        );
        assert_eq!(
            sleep_for(false, Some(MS(500)), false, repaint),
            Duration::ZERO
        );
        // 間がちょうど下限・下限より少し長い
        assert_eq!(
            sleep_for(false, Some(INPUT_INTERVAL), true, repaint),
            Duration::ZERO
        );
        assert_eq!(
            sleep_for(
                false,
                Some(repaint + Duration::from_nanos(1)),
                false,
                repaint
            ),
            Duration::ZERO
        );
        // 1/240 秒を超えて入力の無い下限に届かない間: 入力があれば眠らず、無ければ眠る
        let between = MS(5);
        assert_eq!(
            sleep_for(false, Some(between), true, repaint),
            Duration::ZERO
        );
        assert_eq!(
            sleep_for(false, Some(between), false, repaint),
            repaint - between
        );
        // 起動して最初のフレーム（前が無い）
        assert_eq!(sleep_for(false, None, true, repaint), Duration::ZERO);
        assert_eq!(sleep_for(false, None, false, repaint), Duration::ZERO);
    }

    #[test]
    fn it_never_sleeps_when_waiting_for_vsync() {
        for repaint in [REPAINT_INTERVAL, repaint_interval(Some(60.0))] {
            for since in [None, Some(Duration::ZERO), Some(MS(1)), Some(MS(100))] {
                for input in [false, true] {
                    assert_eq!(sleep_for(true, since, input, repaint), Duration::ZERO);
                }
            }
        }
    }

    #[test]
    fn the_sleep_never_exceeds_the_interval_of_the_frame_kind() {
        let at_60 = repaint_interval(Some(60.0));
        for micros in [0u64, 1, 100, 4_000, 8_000, 8_333, 10_000, 1_000_000] {
            for input in [false, true] {
                for repaint in [REPAINT_INTERVAL, at_60] {
                    let slept = sleep_for(false, Some(US(micros)), input, repaint);
                    assert!(
                        slept <= interval_for(input, repaint),
                        "{micros} µs: {slept:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn frames_are_spaced_by_the_interval_of_their_kind_and_a_pause_resets_the_gap() {
        let base = Instant::now();
        let mut pacing = Pacing::new(false);
        // 最初のフレームは眠らない
        assert_eq!(pacing.plan(base, true), Duration::ZERO);
        pacing.started(base);
        // すぐ次のフレーム: 入力があれば 1/240、無ければ 1/120 まで眠る
        assert_eq!(pacing.plan(base, true), INPUT_INTERVAL);
        assert_eq!(pacing.plan(base, false), REPAINT_INTERVAL);
        // 3 ms 後
        let later = base + MS(3);
        assert_eq!(pacing.plan(later, true), INPUT_INTERVAL - MS(3));
        // 眠った分だけ進めた時刻が次の始まりになる（間隔は始まりから始まりで数える）
        pacing.started(base + INPUT_INTERVAL);
        assert_eq!(
            pacing.plan(base + INPUT_INTERVAL + MS(1), true),
            INPUT_INTERVAL - MS(1)
        );
        // 止まっていたあとは眠らない
        assert_eq!(pacing.plan(base + MS(2_000), false), Duration::ZERO);
        // 時刻が戻って見えても（比べる相手より前）、あふれず眠る長さは間隔以下
        assert!(pacing.plan(base, false) <= REPAINT_INTERVAL);
    }

    #[test]
    fn the_monitor_refresh_rate_changes_the_gap_without_input_but_not_with_input() {
        let base = Instant::now();
        let mut pacing = Pacing::new(false);
        pacing.started(base);
        // 読めない: 1/120 秒
        assert_eq!(pacing.repaint_interval(), REPAINT_INTERVAL);
        assert_eq!(pacing.plan(base, false), REPAINT_INTERVAL);
        // 144 Hz・60 Hz: 入力の無いフレームの下限が変わる
        pacing.set_refresh_rate(Some(144.0));
        assert_eq!(pacing.plan(base, false), repaint_interval(Some(144.0)));
        assert!(pacing.plan(base, false) < REPAINT_INTERVAL);
        pacing.set_refresh_rate(Some(60.0));
        assert_eq!(pacing.plan(base, false), repaint_interval(Some(60.0)));
        assert!(pacing.plan(base, false) > REPAINT_INTERVAL);
        // 入力のあるフレームは、どのモニターでも 1/240 秒
        for hz in [Some(60.0), Some(144.0), None] {
            pacing.set_refresh_rate(hz);
            assert_eq!(pacing.plan(base, true), INPUT_INTERVAL, "{hz:?}");
        }
        // 読めなくなったら 1/120 秒へ戻る
        pacing.set_refresh_rate(Some(60.0));
        pacing.set_refresh_rate(None);
        assert_eq!(pacing.repaint_interval(), REPAINT_INTERVAL);
    }

    #[test]
    fn the_refresh_rate_is_due_the_first_time_and_then_once_a_second() {
        let base = Instant::now();
        let mut pacing = Pacing::new(false);
        assert!(pacing.refresh_due(base), "初めて");
        assert!(!pacing.refresh_due(base), "読んだ直後");
        assert!(!pacing.refresh_due(base + MS(999)));
        assert!(
            pacing.refresh_due(base + REFRESH_RECHECK),
            "1 秒たったら読み直す"
        );
        assert!(!pacing.refresh_due(base + REFRESH_RECHECK + MS(500)));
        assert!(pacing.refresh_due(base + REFRESH_RECHECK * 2 + MS(1)));
    }

    #[test]
    fn the_wait_form_ignores_the_frames_entirely() {
        let base = Instant::now();
        let mut pacing = Pacing::new(true);
        assert!(pacing.waits_for_vsync());
        pacing.started(base);
        assert_eq!(pacing.plan(base, true), Duration::ZERO);
        assert_eq!(pacing.plan(base, false), Duration::ZERO);
        // 待つ形は間隔を選ばない
        assert_eq!(pacing.wait(true), Duration::ZERO);
        assert_eq!(pacing.last_interval(), None);
        // 何もしない形が既定
        assert!(Pacing::default().waits_for_vsync());
    }

    #[test]
    fn wait_sleeps_at_least_the_planned_time_and_remembers_the_interval_it_chose() {
        let mut pacing = Pacing::new(false);
        assert_eq!(pacing.last_interval(), None);
        // 最初は眠らない（間隔は選ぶ）
        assert!(pacing.wait(false) < MS(5));
        assert_eq!(pacing.last_interval(), Some(REPAINT_INTERVAL));
        // すぐ次: 入力の無いフレームは 1/120 秒まで眠る（眠りは短すぎない）
        let started = Instant::now();
        let slept = pacing.wait(false);
        let total = started.elapsed();
        assert!(total >= REPAINT_INTERVAL - MS(1), "{total:?}");
        assert!(slept >= REPAINT_INTERVAL - MS(2), "{slept:?}");
        assert!(slept <= total, "{slept:?} {total:?}");
        // 入力のあるフレームは 1/240 秒を選ぶ
        pacing.wait(true);
        assert_eq!(pacing.last_interval(), Some(INPUT_INTERVAL));
        // 60 Hz のモニター
        pacing.set_refresh_rate(Some(60.0));
        pacing.wait(false);
        assert_eq!(pacing.last_interval(), Some(repaint_interval(Some(60.0))));
    }

    #[test]
    fn the_detached_input_mark_is_taken_once() {
        let mut pacing = Pacing::new(false);
        assert!(!pacing.take_detached_input());
        pacing.note_detached_input();
        assert!(pacing.take_detached_input());
        assert!(!pacing.take_detached_input(), "取り出したら消える");
    }
}
