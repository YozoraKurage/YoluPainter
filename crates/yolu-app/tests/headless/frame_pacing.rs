//! フレームの間隔の下限（垂直同期を待たない表示のとき。`pacing`・`YoluApp::pace_frame`）: eframe が egui のパスの前に呼ぶ `raw_input_hook` が、
//! 前のフレームの始まりからの間を下限（入力のあるフレームは 1/240 秒、無いフレームはモニターのリフレッシュレートの逆数。読めなければ 1/120 秒）まで
//! 眠って埋め、眠った分だけフレームの時刻を進める。待つ形・隠れている間は眠らない。
//! 眠りの長さの計算は `pacing` の単体試験。ここは、フックが実際に眠ること（時計の下限だけを見る。上限は負荷で揺れるので見ない）・選んだ間隔・入力の見方。
use std::time::{Duration, Instant};

use egui::{pos2, Event, RawInput, ViewportId};
use yolu_app::pacing::{repaint_interval, INPUT_INTERVAL, REPAINT_INTERVAL};
use yolu_app::pen::{PenInput, PenSample};
use yolu_app::state::AppState;
use yolu_app::YoluApp;

fn app(waits_for_vsync: bool) -> (YoluApp, PenInput) {
    let pen = PenInput::detached();
    let mut app = YoluApp::with_state(AppState::new(64, 64), pen.clone());
    app.set_frame_pacing(waits_for_vsync);
    (app, pen)
}

fn raw(time: f64, events: usize) -> RawInput {
    RawInput {
        time: Some(time),
        events: vec![Event::PointerMoved(pos2(1.0, 1.0)); events],
        ..Default::default()
    }
}

fn pen_sample() -> PenSample {
    PenSample {
        pos: [4.0, 5.0],
        pressure: 0.5,
        tilt: Default::default(),
        rotation: None,
        contact: true,
        eraser: false,
        barrel: false,
        pointer_id: 1,
        time_ms: 0,
    }
}

/// 続けた 2 つのフレームの、2 つ目の結果。
struct Second {
    /// 2 つ目のフックが返るまでにかかった時間。
    spent: Duration,
    /// 2 つ目のフレームの時刻（`raw_input.time`）が、渡した値から進んだ量（秒）。
    advanced: f64,
}

/// フレームを 2 つ続けて始める（`events` 個の egui の事象つき。`prepare` で、フレームの前に app を整える）。
/// 1 つ目のフックが返ってから 2 つ目を呼ぶまでの間が長いと（負荷で止められた）、2 つ目は眠らなくてよいので、その試行は捨てて作り直す。
fn second_frame(events: usize, prepare: impl Fn(&mut YoluApp, &PenInput)) -> (Second, YoluApp) {
    let ctx = egui::Context::default();
    for _ in 0..50 {
        let (mut app, pen) = app(false);
        prepare(&mut app, &pen);
        let mut first = raw(10.0, events);
        eframe::App::raw_input_hook(&mut app, &ctx, &mut first);
        assert_eq!(
            first.time,
            Some(10.0),
            "最初のフレームは眠らず、時刻もそのまま"
        );
        let returned = Instant::now();
        let mut second = raw(20.0, events);
        let called = Instant::now();
        if called.duration_since(returned) > Duration::from_micros(300) {
            continue;
        }
        eframe::App::raw_input_hook(&mut app, &ctx, &mut second);
        let spent = called.elapsed();
        let advanced = second.time.unwrap() - 20.0;
        return (Second { spent, advanced }, app);
    }
    panic!("負荷が高く、続けた 2 つのフレームを作れなかった");
}

/// 2 つ目のフレームが、`interval` まで実際に眠り、眠った分だけ時刻を進めた。
fn assert_slept(second: &Second, interval: Duration, what: &str) {
    // 1 つ目の始まりからの間は（試行の条件で）高々 1 ms 弱なので、眠りは間隔 − 1 ms 以上
    let least = interval.saturating_sub(Duration::from_millis(1));
    assert!(
        second.spent >= least,
        "{what}: {:?} < {least:?}",
        second.spent
    );
    // 眠った分が、そのフレームの時刻に足されている（足さないと進みは 0）
    assert!(second.advanced > 0.0, "{what}: 時刻が進んでいない");
    let spent = second.spent.as_secs_f64();
    assert!(
        second.advanced <= spent + 1e-3,
        "{what}: 進みが眠りより大きい {} {spent}",
        second.advanced
    );
    assert!(
        second.advanced >= spent - 0.003,
        "{what}: 進みが眠りに足りない {} {spent}",
        second.advanced
    );
}

#[test]
fn consecutive_frames_with_input_sleep_to_one_240th_of_a_second_and_advance_the_frame_time() {
    let (second, app) = second_frame(3, |_, _| {});
    assert_slept(&second, INPUT_INTERVAL, "入力あり");
    assert_eq!(
        app.frame_interval(),
        Some(INPUT_INTERVAL),
        "入力のあるフレームは 1/240 秒を選ぶ"
    );
}

#[test]
fn consecutive_frames_without_input_sleep_to_one_120th_when_the_refresh_rate_is_unknown() {
    let (second, app) = second_frame(0, |_, _| {});
    assert_slept(&second, REPAINT_INTERVAL, "入力なし・読めない");
    assert_eq!(app.frame_interval(), Some(REPAINT_INTERVAL));
}

#[test]
fn the_gap_without_input_follows_the_monitor_refresh_rate_but_with_input_it_does_not() {
    // 144 Hz・60 Hz: 入力の無いフレームの間隔がそのモニターの 1 回の長さになる
    for hz in [144.0, 60.0] {
        let (second, app) = second_frame(0, |app, _| app.set_monitor_refresh_hz(Some(hz)));
        let want = repaint_interval(Some(hz));
        assert_slept(&second, want, &format!("入力なし・{hz} Hz"));
        assert_eq!(app.frame_interval(), Some(want), "{hz} Hz");
    }
    // 読めない: 1/120 秒（`None` を入れ直しても同じ）
    let (second, app) = second_frame(0, |app, _| {
        app.set_monitor_refresh_hz(Some(60.0));
        app.set_monitor_refresh_hz(None);
    });
    assert_slept(&second, REPAINT_INTERVAL, "入力なし・読めなくなった");
    assert_eq!(app.frame_interval(), Some(REPAINT_INTERVAL));
    // 入力のあるフレームは、どのモニターでも 1/240 秒
    for hz in [Some(60.0), Some(144.0), None] {
        let (second, app) = second_frame(2, |app, _| app.set_monitor_refresh_hz(hz));
        assert_slept(&second, INPUT_INTERVAL, &format!("入力あり・{hz:?}"));
        assert_eq!(app.frame_interval(), Some(INPUT_INTERVAL), "{hz:?}");
    }
}

#[test]
fn a_pen_sample_in_the_queue_makes_the_frame_an_input_frame() {
    // egui の事象が無くても、ペンの待ち行列に点があれば入力のあるフレーム（1/240 秒）。フックは待ち行列を取り出さない
    let (second, app) = second_frame(0, |app, pen| {
        app.set_monitor_refresh_hz(Some(60.0));
        pen.push(pen_sample());
    });
    assert_slept(&second, INPUT_INTERVAL, "ペンの点あり");
    assert_eq!(app.frame_interval(), Some(INPUT_INTERVAL));
}

#[test]
fn waiting_for_vsync_never_sleeps_or_moves_the_frame_time() {
    let (mut app, _) = app(true);
    assert!(!app.paces_frames());
    let ctx = egui::Context::default();
    for events in [0, 5] {
        for _ in 0..4 {
            let mut input = raw(7.0, events);
            eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
            assert_eq!(input.time, Some(7.0), "待つ形は何もしない");
        }
    }
    assert_eq!(app.frame_interval(), None, "待つ形は間隔を選ばない");
}

#[test]
fn a_hidden_window_is_left_to_eframe() {
    let (mut app, _) = app(false);
    assert!(app.paces_frames());
    let ctx = egui::Context::default();
    // 最小化している主のウィンドウ（見える別ウィンドウは無い）: eframe が間隔を空けるので、眠らない
    for _ in 0..4 {
        let mut input = raw(3.0, 2);
        input
            .viewports
            .entry(ViewportId::ROOT)
            .or_default()
            .minimized = Some(true);
        input
            .viewports
            .entry(ViewportId::ROOT)
            .or_default()
            .occluded = Some(false);
        eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
        assert_eq!(input.time, Some(3.0));
    }
    assert_eq!(app.frame_interval(), None, "隠れている間は間隔を選ばない");
}

#[test]
fn input_is_seen_in_the_windows_events_and_in_the_pen_queue_until_the_frame_takes_it() {
    let (mut app, pen) = app(false);
    assert!(
        !app.frame_has_input(&RawInput::default()),
        "何も無ければ入力なし"
    );
    assert!(
        app.frame_has_input(&raw(0.0, 1)),
        "主のウィンドウの egui の事象"
    );
    // ペンの点（取り出されるまで残る。見るだけでは消えない）
    pen.push(pen_sample());
    assert!(app.frame_has_input(&RawInput::default()));
    assert!(app.frame_has_input(&RawInput::default()));
    pen.drain();
    assert!(
        !app.frame_has_input(&RawInput::default()),
        "取り出したら入力なし"
    );
}

#[test]
fn a_state_without_settings_does_not_pace_frames_until_it_is_switched() {
    // 設定を読まない作り（試験・既定）は、何もしない形
    let pen = PenInput::detached();
    let mut app = YoluApp::with_state(AppState::new(64, 64), pen);
    assert!(!app.paces_frames());
    app.set_frame_pacing(false);
    assert!(app.paces_frames());
    app.set_frame_pacing(true);
    assert!(!app.paces_frames());
}
