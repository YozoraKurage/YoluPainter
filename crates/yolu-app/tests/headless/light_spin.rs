//! 3D ビューの光を自動で回す（`view3d::light`）: 時間を進めると方位が回り（1 周 12 秒）、描き直しを頼み続ける。ストロークの間は止まり、
//! 切るとその時の方位で止まる。保存しない・文書を変えない。
use std::time::Duration;

use crate::common::drive::Drive;
use yolu_app::state::{Action, AppState};
use yolu_app::view3d::display::Op;
use yolu_app::view3d::light;

fn state() -> AppState {
    let mut app = AppState::new(32, 32);
    app.view3d_mut().load_demo();
    app.view3d_mut().visible = true;
    app
}

fn yaw(app: &AppState) -> f32 {
    app.view3d().view.display.light_yaw
}

fn spin(app: &mut AppState, on: bool) {
    app.apply(Action::View3d(Op::LightSpin(on)));
}

/// 1 周の角度の差（-180〜180）。
fn turned(from: f32, to: f32) -> f32 {
    (to - from + 180.0).rem_euclid(360.0) - 180.0
}

#[test]
fn the_azimuth_turns_one_round_in_twelve_seconds_and_the_height_stays() {
    let mut app = state();
    let (start, pitch) = (yaw(&app), app.view3d().view.display.light_pitch);
    assert!(!app.view3d().view.display.light_spin, "既定は切");
    // 切のあいだは進まない
    assert!(!light::advance(&mut app, 0.05));
    assert_eq!(yaw(&app), start);
    spin(&mut app, true);
    // 3 秒で 90°
    for _ in 0..60 {
        assert!(light::advance(&mut app, 0.05));
    }
    assert!(
        (turned(start, yaw(&app)) - 90.0).abs() < 0.01,
        "{}",
        yaw(&app)
    );
    // 12 秒で 1 周して元の方位（-180〜180 の中に畳まれる）
    for _ in 0..180 {
        light::advance(&mut app, 0.05);
        assert!((-180.0..=180.0).contains(&yaw(&app)));
    }
    assert!(turned(start, yaw(&app)).abs() < 0.05, "{}", yaw(&app));
    assert_eq!(app.view3d().view.display.light_pitch, pitch);
    // 止まったあとの最初のフレームの長い間（ウィンドウが隠れていた間など）では飛ばない: 1 フレームで進むのは 0.1 秒ぶんまで
    let before = yaw(&app);
    light::advance(&mut app, 30.0);
    assert!((turned(before, yaw(&app)) - 3.0).abs() < 0.01);
}

#[test]
fn it_stops_where_it_is_when_switched_off_and_while_a_stroke_is_drawn() {
    let mut app = state();
    spin(&mut app, true);
    light::advance(&mut app, 0.1);
    let stopped = yaw(&app);
    spin(&mut app, false);
    assert!(!light::advance(&mut app, 0.1));
    assert_eq!(yaw(&app), stopped, "切ると、その時の方位のまま");
    // 描いている最中（ストローク）は進めない
    spin(&mut app, true);
    app.view3d_mut().view.input.stroke = Some(yolu_app::state::StrokeSource::Mouse);
    assert!(!light::advance(&mut app, 0.1));
    assert_eq!(yaw(&app), stopped);
    // 離したあと塗り終えるまで（確定待ち）も進めない
    app.view3d_mut().view.input.stroke = None;
    app.view3d_mut().view.input.released = true;
    assert!(!light::advance(&mut app, 0.1));
    app.view3d_mut().view.input.released = false;
    assert!(light::advance(&mut app, 0.1));
    assert_ne!(yaw(&app), stopped);
}

#[test]
fn each_frame_asks_for_another_repaint_only_while_it_turns_and_the_3d_tab_is_shown() {
    let mut app = state();
    let ctx = egui::Context::default();
    let frame = |app: &mut AppState, time: f64| -> Option<Duration> {
        let input = egui::RawInput {
            time: Some(time),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| light::tick(ui.ctx(), app));
        output.textures_delta.clear();
        output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay)
    };
    // 作った直後の egui は、最初の何フレームかを自分で頼む。落ち着いてから見る
    for i in 0..6 {
        frame(&mut app, 0.5 + f64::from(i) * 0.1);
    }
    // 切: 描き直しを頼まない（次の入力まで待つ）
    let idle = frame(&mut app, 1.05).unwrap();
    assert!(idle > Duration::from_millis(100), "{idle:?}");
    spin(&mut app, true);
    let start = yaw(&app);
    let asked = frame(&mut app, 1.10).unwrap();
    assert_eq!(
        asked,
        Duration::ZERO,
        "回している間は、すぐ次のフレームを頼む"
    );
    // 前のフレームから 0.05 秒: 1.5°
    assert!(
        (turned(start, yaw(&app)) - 1.5).abs() < 0.01,
        "{}",
        yaw(&app)
    );
    // 3D のタブが見えていない間は進めず、頼まない
    app.view3d_mut().visible = false;
    let hidden_from = yaw(&app);
    // （前のフレームの頼みは、次のフレームの出力にも残るので、1 フレーム置いてから見る）
    frame(&mut app, 1.15);
    let hidden = frame(&mut app, 1.20).unwrap();
    assert!(hidden > Duration::from_millis(100), "{hidden:?}");
    assert_eq!(yaw(&app), hidden_from);
}

#[test]
fn the_switch_is_not_saved_and_the_document_is_not_changed() {
    let mut app = state();
    let modified = app.modified();
    spin(&mut app, true);
    light::advance(&mut app, 0.1);
    assert!(app.view3d().view.display.light_spin);
    assert!(!app.doc().can_undo(), "取り消しの段を作らない");
    assert_eq!(app.modified(), modified, "文書を変更ありにしない");
    // 設定のファイル（光の向きと同じく画面だけの状態）に書かない: 設定の中に光の回転の項目が無い
    let dir = std::env::temp_dir().join(format!("yolu-light-spin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.conf");
    yolu_app::settings::save(&path, &app.prefs().settings).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.to_lowercase().contains("light"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
    // 新しく始めた画面は切
    assert!(!state().view3d().view.display.light_spin);
}

#[test]
fn it_does_not_compete_with_a_hand_drag_of_the_light() {
    let mut app = state();
    spin(&mut app, true);
    app.view3d_mut().view.input.nav = Some((
        yolu_app::view3d::Nav::Light,
        yolu_app::state::key::PointerButton::Secondary,
    ));
    let held = yaw(&app);
    assert!(light::dragging(&app) && light::active(&app));
    assert!(!light::advance(&mut app, 0.1));
    assert_eq!(yaw(&app), held);
    // 離せば続きから回る
    app.view3d_mut().view.input.nav = None;
    assert!(light::advance(&mut app, 0.1));
    assert!(light::active(&app), "回している間は光の印を出す");
}
