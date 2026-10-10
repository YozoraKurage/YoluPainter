//! 全体の筆圧の調整（編集 → 設定… の「ペン」）: 設定の下限・上限・曲線がペンの筆圧に効いて（マウスは 1 のまま）ブラシへ渡ること、「ペン」の区分が枠の中の
//! ペンの点を集めて分布から調整を決めること、設定の保存と読み直し、日英。実機のペンは無いので、`PenSample` を差し込む。
//! `headless_` で始まる試験は画面を描かず、Wine でも回る。
#![allow(clippy::chunks_exact_to_as_chunks)]
use crate::common;

use std::path::{Path, PathBuf};

use common::*;
use egui::{pos2, vec2, Event, Pos2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::engine::Tilt;
use yolu_app::lang::Lang;
use yolu_app::m2::UiOp;
use yolu_app::pen::adjust::{fit, FitError, PressureAdjust};
use yolu_app::pen::window::{self, PressureAction};
use yolu_app::pen::{PenInput, PenSample};
use yolu_app::state::{Action, AppState};
use yolu_app::YoluApp;
use yolu_core::curve::{Curve, CurvePoint};

type H = Harness<'static, YoluApp>;

fn pt(x: f64, y: f64) -> CurvePoint {
    CurvePoint { x, y }
}

fn sample(at: Pos2, pressure: f32, contact: bool, time_ms: u32) -> PenSample {
    PenSample {
        pos: [at.x, at.y],
        pressure,
        tilt: Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 5,
        time_ms,
    }
}

/// ペンの点を 1 つ（1 フレーム）。
fn pen(h: &mut H, at: Pos2, pressure: f32, contact: bool, time_ms: u32) {
    h.state().pen().push(sample(at, pressure, contact, time_ms));
    h.step();
}

/// ペンで線を引く（触れる→動く→離す。筆圧は線の間ずっと同じ）。
fn pen_stroke(h: &mut H, from: Pos2, to: Pos2, pressure: f32) {
    pen(h, from, pressure, true, 0);
    for i in 1..=8 {
        let t = i as f32 / 8.0;
        pen(h, from + (to - from) * t, pressure, true, i * 10);
    }
    pen(h, to, 0.0, false, 100);
    h.run();
}

/// 文書の合成の全部（straight RGBA）。
fn pixels(h: &H) -> Vec<u8> {
    let doc = &h.state().state.doc;
    doc.composite(doc.bounds()).unwrap()
}

fn painted(bytes: &[u8]) -> usize {
    bytes.chunks_exact(4).filter(|p| p[3] != 0).count()
}

fn app_with_adjust(adjust: PressureAdjust) -> H {
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.prefs.settings.pressure = adjust;
    h.run();
    h
}

// ───────── 設定の調整が筆圧に効く ─────────

#[test]
fn headless_adjust_pressure_follows_the_settings_and_the_default_is_the_identity() {
    let mut s = AppState::new(64, 64);
    for p in [0.0f32, 0.3, 0.77, 1.0] {
        assert_eq!(s.adjust_pressure(p).to_bits(), p.to_bits());
    }
    s.prefs.settings.pressure = PressureAdjust::new(0.2, 0.6, vec![]).unwrap();
    assert!((s.adjust_pressure(0.4) - 0.5).abs() < 1e-6);
    assert_eq!(s.adjust_pressure(0.1), 0.0);
    assert_eq!(s.adjust_pressure(0.9), 1.0);
}

#[test]
fn the_global_adjustment_changes_what_the_pen_draws_and_not_the_mouse() {
    let at = |h: &H| {
        let r = canvas_rect(h);
        (r.center() - vec2(60.0, 0.0), r.center() + vec2(60.0, 0.0))
    };
    // 下限 0.25・上限 0.75 に直すと、ペンの筆圧 0.375 はブラシの 0.25 になる（直さないペンの 0.25 と同じ線。2 の冪の値なので丸めが無い）
    let mut a = app_with_adjust(PressureAdjust::new(0.25, 0.75, vec![]).unwrap());
    let (from, to) = at(&a);
    pen_stroke(&mut a, from, to, 0.375);
    let mut b = app_with_adjust(PressureAdjust::default());
    pen_stroke(&mut b, from, to, 0.25);
    let (adjusted, plain) = (pixels(&a), pixels(&b));
    assert!(painted(&plain) > 0);
    assert_eq!(adjusted, plain);
    // 直さなければ、ペンの 0.375 は 0.25 とは違う線
    let mut c = app_with_adjust(PressureAdjust::default());
    pen_stroke(&mut c, from, to, 0.375);
    assert_ne!(pixels(&c), plain);
    // 曲線も通る: 曲線で 0.5 を 0.75 に持ち上げると、ペンの 0.5 がブラシの 0.75 になる
    let lifted =
        PressureAdjust::new(0.0, 1.0, vec![pt(0.0, 0.0), pt(0.5, 0.75), pt(1.0, 1.0)]).unwrap();
    let mut d = app_with_adjust(lifted);
    pen_stroke(&mut d, from, to, 0.5);
    let mut e = app_with_adjust(PressureAdjust::default());
    pen_stroke(&mut e, from, to, 0.75);
    assert_eq!(pixels(&d), pixels(&e));

    // マウスの筆圧は 1 のまま: 調整を入れても入れなくても同じ線
    let mut m1 = app_with_adjust(PressureAdjust::new(0.3, 0.5, vec![]).unwrap());
    let mut m2 = app_with_adjust(PressureAdjust::default());
    for m in [&mut m1, &mut m2] {
        drag(m, &[from, from + (to - from) * 0.5, to]);
    }
    assert!(painted(&pixels(&m2)) > 0);
    assert_eq!(pixels(&m1), pixels(&m2));
}

#[test]
fn a_touch_force_goes_through_the_adjustment_like_a_pen_point() {
    let touch = |h: &mut H, at: Pos2, phase: egui::TouchPhase, force: f32| {
        h.event(Event::Touch {
            device_id: egui::TouchDeviceId(0),
            id: egui::TouchId(1),
            phase,
            pos: at,
            force: Some(force),
        });
    };
    let draw = |h: &mut H, force: f32| {
        let r = canvas_rect(h);
        let (from, to) = (r.center() - vec2(60.0, 0.0), r.center() + vec2(60.0, 0.0));
        touch(h, from, egui::TouchPhase::Start, force);
        press(h, from, egui::PointerButton::Primary);
        h.step();
        for i in 1..=8 {
            let at = from + (to - from) * (i as f32 / 8.0);
            touch(h, at, egui::TouchPhase::Move, force);
            move_to(h, at);
            h.step();
        }
        release(h, to, egui::PointerButton::Primary);
        touch(h, to, egui::TouchPhase::End, force);
        h.run();
    };
    let mut a = app_with_adjust(PressureAdjust::new(0.25, 0.75, vec![]).unwrap());
    draw(&mut a, 0.375);
    let mut b = app_with_adjust(PressureAdjust::default());
    draw(&mut b, 0.25);
    assert!(painted(&pixels(&b)) > 0);
    assert_eq!(pixels(&a), pixels(&b));
}

// ───────── ウィンドウ: 枠の中で描いて、分布から決める ─────────

/// 筆圧の調整を開く（設定のウィンドウが「ペン」の区分で開く）。
fn open_window(h: &mut H) {
    h.state_mut()
        .state
        .apply(Action::Pressure(PressureAction::Open));
    h.run();
}

/// 試験の設定の置き場（`target/pressure-tests/<pid>/<tag>`）。試験が落ちても Drop で消える。pid のフォルダと `pressure-tests` は、使っている
/// 置き場が 1 つも無くなったときに（空なら）消す。作る・消すを同じ錠の中で行い、並んで走る別の試験の作りかけを消さない。
struct SettingsDir(PathBuf);

/// 今ある置き場の数（錠の中身）。
static LIVE_DIRS: std::sync::Mutex<usize> = std::sync::Mutex::new(0);

fn settings_dir(tag: &str) -> SettingsDir {
    let mut live = LIVE_DIRS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/pressure-tests")
        .join(std::process::id().to_string())
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    *live += 1;
    SettingsDir(dir)
}

impl std::ops::Deref for SettingsDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for SettingsDir {
    fn drop(&mut self) {
        let mut live = LIVE_DIRS.lock().unwrap_or_else(|e| e.into_inner());
        let _ = std::fs::remove_dir_all(&self.0);
        *live = live.saturating_sub(1);
        if *live == 0 {
            // 空でなければ（別のプロセスの置き場など）失敗するだけで、何も消さない
            if let Some(pid) = self.0.parent() {
                let _ = std::fs::remove_dir(pid);
                if let Some(root) = pid.parent() {
                    let _ = std::fs::remove_dir(root);
                }
            }
        }
    }
}

fn app_with_settings(path: &Path) -> H {
    let path = path.to_path_buf();
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(common::shared_gpu::renderer())
        .build_eframe(move |cc| {
            let mut app =
                YoluApp::for_context_with_settings(&cc.egui_ctx, Some(path), PenInput::detached())
                    .with_render_state(cc.wgpu_render_state.as_ref());
            // 中央は 1 つの組（キャンバスだけが広く出る）
            app.dock = common::tabbed_center_dock(1280.0);
            app
        });
    h.run();
    h
}

/// 描く枠の矩形。
fn frame_of(h: &H) -> egui::Rect {
    h.state().state.pressure.frame.expect("ウィンドウを描いた")
}

/// 枠の中で、筆圧が lo〜hi に一様に変わる線を n 本引く（1 本は 12 点）。引いた筆圧を返す。
fn draw_strokes(h: &mut H, n: usize, lo: f32, hi: f32) -> Vec<f32> {
    let frame = frame_of(h);
    let mut all = Vec::new();
    for k in 0..n {
        let y = frame.top() + 10.0 + (k as f32 * 7.0) % (frame.height() - 20.0);
        for i in 0..12 {
            let t = i as f32 / 11.0;
            let pressure = lo + (hi - lo) * t;
            pen(
                h,
                pos2(frame.left() + 12.0 + t * (frame.width() - 24.0), y),
                pressure,
                true,
                i * 8,
            );
            all.push(pressure);
        }
        pen(h, pos2(frame.right() - 12.0, y), 0.0, false, 200);
    }
    h.run();
    all
}

#[test]
fn the_window_collects_strokes_in_its_frame_and_fits_the_adjustment_from_them() {
    let dir = settings_dir("window");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path);
    assert!(!h.state().state.pressure.open());
    open_window(&mut h);
    assert!(h.state().state.pressure.open());
    let rect = yolu_app::prefs::last_rect(&h.ctx).expect("ウィンドウが開いている");
    assert!(rect.contains_rect(frame_of(&h)));
    // 枠の外（ウィンドウの外のキャンバスの上）に描いても集めない（そこは普通にキャンバスへ描ける）
    let canvas = canvas_rect(&h);
    let outside = |p: Pos2| canvas.contains(p) && !rect.expand(4.0).contains(p);
    let (from, to) = [
        (
            canvas.center() - vec2(200.0, 0.0),
            canvas.center() - vec2(100.0, 0.0),
        ),
        (
            canvas.center() + vec2(100.0, 0.0),
            canvas.center() + vec2(200.0, 0.0),
        ),
        (
            canvas.left_top() + vec2(30.0, 30.0),
            canvas.left_top() + vec2(130.0, 30.0),
        ),
        (
            canvas.right_bottom() - vec2(130.0, 30.0),
            canvas.right_bottom() - vec2(30.0, 30.0),
        ),
        // ウィンドウの下の余白（ウィンドウが大きく、キャンバスのほぼ全面を覆うとき）
        {
            let y = (rect.bottom() + 4.0 + canvas.bottom()) / 2.0;
            (
                pos2(canvas.center().x - 100.0, y),
                pos2(canvas.center().x + 100.0, y),
            )
        },
    ]
    .into_iter()
    .find(|&(from, to)| outside(from) && outside(to))
    .expect("ウィンドウの外にキャンバスの上の点がある");
    pen_stroke(&mut h, from, to, 0.7);
    assert!(h.state().state.pressure.strokes.is_empty());
    let outside_painted = painted(&pixels(&h));
    assert!(outside_painted > 0, "ウィンドウの外はキャンバスへ描ける");

    // 足りない間は決められない（短い理由が出る）
    let drawn = draw_strokes(&mut h, 1, 0.2, 0.6);
    assert_eq!(h.state().state.pressure.strokes.len(), 1);
    assert_eq!(h.state().state.pressure.samples(), drawn);
    let fit_at = h.get_by_label("自動調整").rect().center();
    click(&mut h, fit_at);
    assert_eq!(h.state().state.pressure.note, Some(FitError::TooFew));
    assert!(h.state().state.prefs.settings.pressure.is_default());

    // 描き足すと決まる: 分布から作った調整と同じ。文書には何も描かれていない
    let mut drawn = drawn;
    drawn.extend(draw_strokes(&mut h, 3, 0.2, 0.6));
    assert_eq!(h.state().state.pressure.strokes.len(), 4);
    let before = pixels(&h);
    click(&mut h, fit_at);
    let fitted = fit(&drawn).unwrap();
    assert_eq!(h.state().state.prefs.settings.pressure, fitted);
    assert!(h.state().state.pressure.note.is_none());
    assert!(!fitted.is_default());
    assert_eq!(
        painted(&before),
        outside_painted,
        "枠で描いた線は文書に入らない（枠の外に描いた分だけ）"
    );
    assert_eq!(pixels(&h), before);

    // 元に戻す（開いたときへ）・既定（直線へ）
    h.get_by_label("元に戻す").click();
    h.run();
    assert!(h.state().state.prefs.settings.pressure.is_default());
    click(&mut h, fit_at);
    assert_eq!(h.state().state.prefs.settings.pressure, fitted);
    h.get_by_label("既定").click();
    h.run();
    assert!(h.state().state.prefs.settings.pressure.is_default());
    // 消す: 線が無くなり、自動調整は押せない
    h.get_by_label("消す").click();
    h.run();
    assert!(h.state().state.pressure.strokes.is_empty());

    // 調整は設定のファイルへ書かれる（既定でない値だけ）
    click(&mut h, fit_at);
    draw_strokes(&mut h, 4, 0.2, 0.6);
    let fit_at = h.get_by_label("自動調整").rect().center();
    click(&mut h, fit_at);
    h.run();
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.lines().any(|l| l.starts_with("pressure_low=")),
        "{written}"
    );
    assert!(
        written.lines().any(|l| l.starts_with("pressure_high=")),
        "{written}"
    );
    // 閉じると線を捨てる。次の起動は、書いた調整で始まる
    h.get_by_label("閉じる").click();
    h.run();
    assert!(!h.state().state.pressure.open());
    assert!(h.state().state.pressure.strokes.is_empty());
    let saved = h.state().state.prefs.settings.pressure.clone();
    drop(h);
    let h = app_with_settings(&path);
    assert_eq!(h.state().state.prefs.settings.pressure, saved);
    assert!(!saved.is_default());
}

/// Windows Ink に掛かっていない環境のペンは egui の Touch の力として来る。ウィンドウが開いている間だけ、枠の中の点を集める。
#[test]
fn the_open_window_collects_a_touch_force_inside_its_frame_and_a_closed_window_collects_nothing() {
    let touch = |h: &mut H, at: Pos2, phase: egui::TouchPhase, force: Option<f32>| {
        h.event(Event::Touch {
            device_id: egui::TouchDeviceId(0),
            id: egui::TouchId(1),
            phase,
            pos: at,
            force,
        });
        h.step();
    };
    let line = |h: &mut H, y: f32, force: f32| {
        let frame = frame_of(h);
        let at = |t: f32| pos2(frame.left() + 12.0 + t * (frame.width() - 24.0), y);
        touch(h, at(0.0), egui::TouchPhase::Start, Some(force));
        touch(h, at(0.5), egui::TouchPhase::Move, Some(force * 0.5));
        touch(h, at(1.0), egui::TouchPhase::End, None);
        h.run();
    };
    let mut h = app_with_adjust(PressureAdjust::default());
    assert!(h.state().state.pressure.strokes.is_empty());
    // ウィンドウが閉じている間は何も集めない（開いたときは空から始まる）
    let (x, y) = (400.0, 400.0);
    touch(&mut h, pos2(x, y), egui::TouchPhase::Start, Some(0.5));
    touch(&mut h, pos2(x, y), egui::TouchPhase::End, None);
    assert!(h.state().state.pressure.strokes.is_empty());
    open_window(&mut h);
    let frame = frame_of(&h);
    assert!(h.state().state.pressure.strokes.is_empty());
    // 枠の中の線は 1 本ずつ集まる。力の無い点（End）は集めない
    line(&mut h, frame.center().y, 0.8);
    assert_eq!(h.state().state.pressure.strokes.len(), 1);
    assert_eq!(h.state().state.pressure.samples(), vec![0.8, 0.4]);
    line(&mut h, frame.center().y + 10.0, 0.6);
    assert_eq!(h.state().state.pressure.strokes.len(), 2);
    assert_eq!(h.state().state.pressure.samples(), vec![0.8, 0.4, 0.6, 0.3]);
    // 枠の外の点は集めない
    touch(
        &mut h,
        frame.right_bottom() + vec2(40.0, 40.0),
        egui::TouchPhase::Start,
        Some(0.9),
    );
    touch(
        &mut h,
        frame.right_bottom() + vec2(50.0, 40.0),
        egui::TouchPhase::End,
        None,
    );
    assert_eq!(h.state().state.pressure.strokes.len(), 2);
    // 閉じると線を捨て、そのあとの枠だった所の点も集めない
    h.get_by_label("閉じる").click();
    h.run();
    assert!(!h.state().state.pressure.open());
    touch(&mut h, frame.center(), egui::TouchPhase::Start, Some(0.7));
    touch(&mut h, frame.center(), egui::TouchPhase::End, None);
    assert!(h.state().state.pressure.strokes.is_empty());
}

#[test]
fn window_actions_set_the_range_and_the_curve_and_the_collected_distribution_stays() {
    // Action を直接当てる（スライダーの操作は `dragging_a_range_slider_...`、曲線の枠の操作は `the_curve_frame_...`）
    let mut h = app_with_adjust(PressureAdjust::default());
    open_window(&mut h);
    let drawn = draw_strokes(&mut h, 4, 0.1, 0.5);
    // 動かしたほうが、相手の手前（最小の幅）で止まる。相手は動かさない
    h.state_mut()
        .state
        .apply(Action::Pressure(PressureAction::SetRange {
            low: 0.95,
            high: 1.0,
            moved_low: true,
        }));
    let p = &h.state().state.prefs.settings.pressure;
    assert!((p.high() - p.low() - yolu_app::pen::adjust::MIN_SPAN).abs() < 1e-6);
    assert_eq!(p.high(), 1.0);
    h.state_mut()
        .state
        .apply(Action::Pressure(PressureAction::SetRange {
            low: 0.2,
            high: 0.3,
            moved_low: true,
        }));
    h.state_mut()
        .state
        .apply(Action::Pressure(PressureAction::SetRange {
            low: 0.25,
            high: 0.3,
            moved_low: true,
        }));
    let p = &h.state().state.prefs.settings.pressure;
    assert!(
        (p.low() - 0.2).abs() < 1e-6 && p.high() == 0.3,
        "{} {}",
        p.low(),
        p.high()
    );
    let bent = Curve::new(vec![pt(0.0, 0.0), pt(0.5, 0.8), pt(1.0, 1.0)]).unwrap();
    h.state_mut()
        .state
        .apply(Action::Pressure(PressureAction::SetCurve(bent.clone())));
    assert_eq!(h.state().state.prefs.settings.pressure.curve().len(), 3);
    // 調整は曲線の点を画面の精度（f32）に丸めて持つ
    let held = h.state().state.prefs.settings.pressure.curve_shape();
    for (a, b) in held.points().iter().zip(bent.points()) {
        assert!(
            (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6,
            "{a:?} {b:?}"
        );
    }
    // 範囲外の曲線は、共通の曲線の型が作る時点で断る（調整へは届かない）
    assert!(Curve::new(vec![pt(0.0, 0.0), pt(0.5, 2.0), pt(1.0, 1.0)]).is_err());
    // 描いた分布は調整を変えても変わらない（調整を通す前の値を集めている）
    assert_eq!(h.state().state.pressure.samples(), drawn);
    h.run();
}

/// 曲線の枠（共通の編集の部品）: 何も無い所を押すと点を足して動かせ、右クリックで消し、Esc でやめられる。ドラッグの間は設定へ書かず（下書き）、
/// 離したとき 1 回の変更として調整に入り、ペンの筆圧がその曲線で曲がる。
#[test]
fn the_curve_frame_adds_moves_and_removes_points_and_the_curve_bends_the_pen_pressure() {
    let dir = settings_dir("curve-frame");
    let path = dir.join("settings.conf");
    std::fs::write(&path, "language=ja\n").unwrap();
    let mut h = app_with_settings(&path);
    open_window(&mut h);
    h.run();
    let frame = h
        .state()
        .state
        .pressure
        .curve_frame
        .expect("ウィンドウが曲線の枠を描いた");
    let g = frame.shrink(6.0);
    let at = |x: f32, y: f32| pos2(g.left() + x * g.width(), g.bottom() - y * g.height());
    let primary = egui::PointerButton::Primary;
    let curve = |h: &H| h.state().state.prefs.settings.pressure.curve_shape();
    let watch = || std::fs::write(&path, "watch\n").unwrap();
    let untouched = || {
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "watch\n",
            "書いてはいけない"
        )
    };
    assert!(curve(&h).is_identity());

    // 押して動かす間は下書き: 調整も設定のファイルも変わらない。離すと 1 回で (0.5, 0.8) の点が入る
    watch();
    move_to(&h, at(0.5, 0.5));
    h.step();
    press(&h, at(0.5, 0.5), primary);
    h.step();
    move_to(&h, at(0.5, 0.7));
    h.step();
    move_to(&h, at(0.5, 0.8));
    h.step();
    assert!(curve(&h).is_identity(), "ドラッグの間は書かない");
    untouched();
    release(&h, at(0.5, 0.8), primary);
    h.step();
    h.run();
    let bent = curve(&h);
    assert_eq!(bent.points().len(), 3);
    let p = bent.points()[1];
    assert!(
        (p.x - 0.5).abs() < 0.02 && (p.y - 0.8).abs() < 0.02,
        "{p:?}"
    );
    assert!(!h.state().state.prefs.settings.pressure.is_default());
    // ペンの筆圧がその曲線で曲がる（下限 0・上限 1 のまま）
    let bent_pressure = h.state().state.adjust_pressure(0.5);
    assert!((bent_pressure - 0.8).abs() < 0.03, "{bent_pressure}");
    assert_eq!(h.state().state.adjust_pressure(1.0), 1.0);
    assert_eq!(h.state().state.adjust_pressure(0.0), 0.0);
    // 設定のファイルには曲線が書かれる
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .any(|l| l.starts_with("pressure_curve=")));

    // Esc でやめると、押す前のまま（書かない）
    let before = h.state().state.prefs.settings.pressure.clone();
    watch();
    let mid = bent.points()[1];
    let mid_at = at(mid.x as f32, mid.y as f32);
    move_to(&h, mid_at);
    h.step();
    press(&h, mid_at, primary);
    h.step();
    move_to(&h, at(0.3, 0.2));
    h.step();
    key(&h, egui::Key::Escape, egui::Modifiers::NONE);
    h.step();
    release(&h, at(0.3, 0.2), primary);
    h.step();
    h.run();
    assert_eq!(h.state().state.prefs.settings.pressure, before);
    untouched();

    // 点を右クリックで消すと、直線（既定）に戻る
    move_to(&h, mid_at);
    h.step();
    press(&h, mid_at, egui::PointerButton::Secondary);
    h.step();
    release(&h, mid_at, egui::PointerButton::Secondary);
    h.step();
    h.run();
    assert!(curve(&h).is_identity());
    assert!(h.state().state.prefs.settings.pressure.is_default());

    // 隣との間隔は 0.02 以上: 点（0.5, 0.8）を足し、そこから縦に離れた（つかめない）所で、横の間隔が 0.02 未満の所を押しても点は増えない。
    // 0.02 以上離れた所なら増える
    click(&mut h, at(0.5, 0.8));
    assert_eq!(curve(&h).points().len(), 3);
    let three = h.state().state.prefs.settings.pressure.clone();
    for x in [0.5, 0.51, 0.49] {
        click(&mut h, at(x, 0.3));
        assert_eq!(h.state().state.prefs.settings.pressure, three, "x={x}");
    }
    click(&mut h, at(0.56, 0.3));
    assert_eq!(curve(&h).points().len(), 4, "間隔が足りていれば増える");
    // 点を 2 つとも消して直線へ戻す
    for (x, y) in [(0.56, 0.3), (0.5, 0.8)] {
        let on = at(x, y);
        move_to(&h, on);
        h.step();
        press(&h, on, egui::PointerButton::Secondary);
        h.step();
        release(&h, on, egui::PointerButton::Secondary);
        h.step();
        h.run();
    }
    assert!(h.state().state.prefs.settings.pressure.is_default());

    // 点は 16 個まで（足せない所では何も起きない）。どの点からも 0.02 以上離れた所を押して、数の上限で断られることを確かめる
    for i in 1..16 {
        let x = i as f32 / 16.0;
        move_to(&h, at(x, x));
        h.step();
        click(&mut h, at(x, 0.5));
    }
    assert_eq!(curve(&h).points().len(), Curve::MAX_POINTS);
    let full = h.state().state.prefs.settings.pressure.clone();
    click(&mut h, at(0.03, 0.9));
    assert_eq!(h.state().state.prefs.settings.pressure, full);
    // 既定へ戻す
    h.get_by_label("既定").click();
    h.run();
    assert!(h.state().state.prefs.settings.pressure.is_default());
}

/// 下限のスライダーをドラッグする: 動かす間は値が動くが設定のファイルへは書かず、離すと 1 回だけ書く（ドラッグの間じゅう同期付きの書き込みをしない）。
/// Esc で戻した値は、書いてある値と同じなので書かない。ウィンドウを閉じても 1 回書く。
#[test]
fn dragging_a_range_slider_writes_the_settings_once_on_release() {
    let dir = settings_dir("range-drag");
    let path = dir.join("settings.conf");
    std::fs::write(&path, "language=ja\n").unwrap();
    let mut h = app_with_settings(&path);
    open_window(&mut h);
    h.run();
    let slider = h
        .get_by_role_and_label(egui::accesskit::Role::Slider, "下限")
        .rect();
    let y = slider.bottom() - 4.0;
    let at = |fraction: f32| pos2(slider.left() + slider.width() * fraction, y);
    let low = |h: &H| h.state().state.prefs.settings.pressure.low();
    // 書いたかどうかは、ファイルを見張り用の中身に替えておき、書き換えられたかで見る
    let watch = || std::fs::write(&path, "watch\n").unwrap();
    let untouched = || {
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "watch\n",
            "書いてはいけない"
        )
    };
    let primary = egui::PointerButton::Primary;
    let written_low = || -> Option<f32> {
        std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .find_map(|l| l.strip_prefix("pressure_low=").and_then(|v| v.parse().ok()))
    };

    watch();
    press(&h, at(0.2), primary);
    h.step();
    assert!(h.state().state.pressure.dragging);
    assert!((low(&h) - 0.2).abs() < 0.02, "{}", low(&h));
    untouched();
    move_to(&h, at(0.4));
    h.step();
    assert!((low(&h) - 0.4).abs() < 0.02, "{}", low(&h));
    untouched();
    move_to(&h, at(0.3));
    h.step();
    untouched();
    // 離すと、そのときの値を 1 回だけ書く（そのあとのフレームでは書き直さない）
    release(&h, at(0.3), primary);
    h.step();
    h.run();
    assert!(!h.state().state.pressure.dragging);
    let value = low(&h);
    assert!((value - 0.3).abs() < 0.02, "{value}");
    assert_eq!(written_low(), Some(value));
    watch();
    h.step();
    h.step();
    untouched();

    // Esc で止めると、押す前の値に戻り、書かない
    press(&h, at(0.6), primary);
    h.step();
    move_to(&h, at(0.7));
    h.step();
    assert!((low(&h) - 0.7).abs() < 0.02);
    key(&h, egui::Key::Escape, egui::Modifiers::NONE);
    h.step();
    assert_eq!(low(&h), value, "押す前の値に戻る");
    assert!(!h.state().state.pressure.dragging);
    untouched();
    release(&h, at(0.7), primary);
    h.step();
    h.run();
    assert_eq!(low(&h), value);
    untouched();

    // ドラッグの途中でウィンドウを閉じると、そこまでの値を 1 回書く
    press(&h, at(0.5), primary);
    h.step();
    move_to(&h, at(0.55));
    h.step();
    untouched();
    let mid = low(&h);
    h.state_mut()
        .state
        .apply(Action::Prefs(yolu_app::prefs::PrefsAction::Close));
    h.step();
    h.run();
    assert!(!h.state().state.pressure.dragging);
    assert_eq!(written_low(), Some(mid));
    release(&h, at(0.55), primary);
    h.run();
}

// ───────── 集める線の上限 ─────────

/// 枠を決めた状態（描く枠が出ている）。枠は画面の点で (100, 100) から 380 × 96。
fn window_state() -> (AppState, egui::Rect) {
    let mut s = AppState::new(64, 64);
    let frame = egui::Rect::from_min_size(pos2(100.0, 100.0), vec2(380.0, 96.0));
    s.pressure.frame = Some(frame);
    (s, frame)
}

/// 枠の中の点 index（筆圧は index の目印）。
fn inside(frame: egui::Rect, index: usize, pressure: f32, contact: bool) -> PenSample {
    sample(
        frame.min
            + vec2(
                5.0 + (index % 300) as f32,
                10.0 + (index / 300 % 60) as f32 * 0.5,
            ),
        pressure,
        contact,
        index as u32,
    )
}

#[test]
fn headless_the_window_keeps_at_most_the_stroke_limit_and_drops_the_oldest_strokes() {
    let (mut s, frame) = window_state();
    let total = window::MAX_STROKES + 6;
    for k in 0..total {
        // 1 本 = 3 点 + 離す。筆圧は線ごとの目印
        let mark = (k + 1) as f32 / 1000.0;
        let points: Vec<PenSample> = (0..3)
            .map(|i| inside(frame, i, mark, true))
            .chain([inside(frame, 3, 0.0, false)])
            .collect();
        s.pressure_observe(1.0, &points);
        assert!(s.pressure.strokes.len() <= window::MAX_STROKES);
    }
    assert_eq!(s.pressure.strokes.len(), window::MAX_STROKES);
    // 残るのは新しい 64 本（古い 6 本が落ちた）
    let first_mark = s.pressure.strokes[0][0].pressure;
    let last_mark = s.pressure.strokes.last().unwrap()[0].pressure;
    assert_eq!(
        first_mark,
        (total - window::MAX_STROKES + 1) as f32 / 1000.0
    );
    assert_eq!(last_mark, total as f32 / 1000.0);
}

#[test]
fn headless_one_long_stroke_keeps_at_most_the_dot_limit_by_dropping_its_oldest_dots() {
    let (mut s, frame) = window_state();
    let n = window::MAX_DOTS + 700;
    for i in 0..n {
        // 筆圧は点の番号の目印（0 でない値）
        s.pressure_observe(1.0, &[inside(frame, i, (i + 1) as f32 / 100_000.0, true)]);
        let dots: usize = s.pressure.strokes.iter().map(Vec::len).sum();
        assert!(dots <= window::MAX_DOTS, "{i}: {dots}");
    }
    assert_eq!(s.pressure.strokes.len(), 1, "1 本の線のまま");
    let stroke = &s.pressure.strokes[0];
    assert_eq!(stroke.len(), window::MAX_DOTS);
    // 古い点から捨てている: 先頭は 701 番目、最後は最後の点
    assert_eq!(stroke[0].pressure, 701.0 / 100_000.0);
    assert_eq!(stroke.last().unwrap().pressure, n as f32 / 100_000.0);
    assert_eq!(s.pressure.samples().len(), window::MAX_DOTS);
}

#[test]
fn headless_several_long_strokes_drop_whole_old_strokes_first_and_then_the_head_of_the_oldest_kept()
{
    let (mut s, frame) = window_state();
    let each = 8_000;
    for k in 0..3 {
        for i in 0..each {
            s.pressure_observe(1.0, &[inside(frame, i, (k + 1) as f32 / 10.0, true)]);
        }
        s.pressure_observe(1.0, &[inside(frame, 0, 0.0, false)]);
    }
    let lens: Vec<usize> = s.pressure.strokes.iter().map(Vec::len).collect();
    // 24000 点 → 4000 点を古い線の頭から捨てる
    assert_eq!(lens, [4_000, 8_000, 8_000]);
    assert_eq!(lens.iter().sum::<usize>(), window::MAX_DOTS);
    // もう 1 本長く引くと、最初の線がまるごと落ち、次の線の頭が削られる
    for i in 0..each {
        s.pressure_observe(1.0, &[inside(frame, i, 0.4, true)]);
    }
    let lens: Vec<usize> = s.pressure.strokes.iter().map(Vec::len).collect();
    assert_eq!(lens, [4_000, 8_000, 8_000]);
    assert_eq!(s.pressure.strokes[0][0].pressure, 0.2);
    assert_eq!(s.pressure.strokes.last().unwrap()[0].pressure, 0.4);
}

#[test]
fn headless_fitting_a_distribution_clamped_on_one_side_does_not_panic_and_gives_a_valid_adjustment()
{
    // 強く押して飽和するペン（p10 = 0.915・p90 = 1.0）と、軽くしか押さないペン（p10 = 0.001・p90 = 0.086）。ウィンドウの「自動調整」と同じ Action
    for (lo, hi) in [(0.915f32, 1.0f32), (0.001, 0.086)] {
        let (mut s, frame) = window_state();
        let mut points: Vec<PenSample> = (0..20).map(|i| inside(frame, i, lo, true)).collect();
        points.extend((20..100).map(|i| inside(frame, i, hi, true)));
        s.pressure_observe(1.0, &points);
        s.apply(Action::Pressure(PressureAction::Fit));
        assert!(
            s.pressure.note.is_none(),
            "{lo} {hi}: {:?}",
            s.pressure.note
        );
        let a = &s.prefs.settings.pressure;
        assert!(
            a.low() >= 0.0
                && a.high() <= 1.0
                && a.high() - a.low() >= yolu_app::pen::adjust::MIN_SPAN - 1e-6,
            "{lo} {hi}"
        );
        assert!(!a.is_default());
    }
}

#[test]
fn the_window_is_in_both_languages_and_its_text_names_things_without_instructions() {
    let mut h = app_with_adjust(PressureAdjust::default());
    open_window(&mut h);
    for label in ["自動調整", "消す", "元に戻す", "既定", "下限", "上限"] {
        let _ = h.get_by_label(label);
    }
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    for label in ["Auto", "Clear", "Revert", "Default", "Low", "High"] {
        let _ = h.get_by_label(label);
    }
    // 英語では日本語が残らない
    for label in ["自動調整", "元に戻す", "下限", "上限"] {
        assert!(h.query_by_label_contains(label).is_none(), "{label}");
    }
}

/// ウィンドウの中だけを撮って、正解の絵と比べる（ほかのパネルの変更で壊れない）。
fn shot(h: &mut H, name: &str) {
    let rect = yolu_app::prefs::last_rect(&h.ctx).expect("ウィンドウが開いている");
    h.event(egui::Event::PointerGone);
    h.step();
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        rect.left().floor() as u32,
        rect.top().floor() as u32,
        rect.width().ceil() as u32,
        rect.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

#[test]
fn the_window_looks_the_same_before_and_after_fitting_and_in_english() {
    let mut h = app_with_adjust(PressureAdjust::default());
    open_window(&mut h);
    shot(&mut h, "pressure_window_empty");
    draw_strokes(&mut h, 4, 0.15, 0.55);
    let fit_at = h.get_by_label("自動調整").rect().center();
    click(&mut h, fit_at);
    shot(&mut h, "pressure_window_fitted");
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    shot(&mut h, "pressure_window_english");
}
