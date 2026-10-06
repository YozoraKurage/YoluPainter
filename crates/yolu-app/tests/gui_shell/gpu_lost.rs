//! 主の wgpu の装置を失ったとき: 描いていた絵（文書）は触らず、復旧の書き置きを急いで取り、GPU の道（3D ビュー・キャンバスの GPU の表示）を
//! 手放して CPU の表示へ落とし、理由を出して、終わる。書き置きの「保存していない作業」の印は残る（次の起動の復旧の窓から開ける）。
//! 受け手の無い誤りは panic にせず記録する。失った知らせを入れる口（`GpuWatch::inject_loss`）と、本物の装置の破棄（`Device::destroy`）の
//! 両方で、受ける側の流れを通す。
//!
//! 窓ごとに自分の装置を作る（`.wgpu()`。共用の接続 `common::shared_gpu` は使わない）: 装置を破棄する試験・見張りの受け口を付ける試験が、
//! 共用の装置を壊したり、ほかの試験の誤りの受け口を取り替えたりしないため。窓は `gpu_thread::builder` の貸し出しで 1 つずつ作る。
use crate::common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use egui::{vec2, ViewportCommand, ViewportId};
use egui_kittest::Harness;
use yolu_app::engine::{composite_pixel, DVec2};
use yolu_app::lang::Lang;
use yolu_app::pen::PenInput;
use yolu_app::recovery::{DiskSpace, RecoverySettings, SpaceProbe};
use yolu_app::state::AppState;
use yolu_app::YoluApp;
use yolu_io::GenerationStore;

type H = Harness<'static, YoluApp>;
type Device = eframe::egui_wgpu::wgpu::Device;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-gpulost-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 復旧を動かし、見張りを付けた窓（描いて、保存していない印を付けてある）。装置は `device` へ控える。
fn window(lang: Lang, root: &std::path::Path, device: Arc<Mutex<Option<Device>>>) -> H {
    let mut state = AppState::new_in(64, 64, lang);
    let probe: SpaceProbe = Arc::new(|_| {
        Some(DiskSpace {
            total: 1 << 40,
            available: 1 << 39,
        })
    });
    state.recovery.set_space_probe(Some(probe));
    // 間隔は長く（装置を失ったときに「急いで」書くことを確かめる。時間では書かれない）
    state
        .recovery
        .enable(
            root.to_path_buf(),
            RecoverySettings {
                interval_seconds: 3600,
                ..RecoverySettings::default()
            },
        )
        .unwrap();
    let mut h = gpu_thread::builder()
        .with_size(vec2(900.0, 600.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .with_render_options(render_options())
        .wgpu()
        .build_eframe(move |cc| {
            let rs = cc.wgpu_render_state.as_ref().expect("描画の状態");
            *device.lock().unwrap() = Some(rs.device.clone());
            let mut app = with_render_state_cpu_canvas(
                YoluApp::for_context(&cc.egui_ctx, state, PenInput::detached()),
                cc.wgpu_render_state.as_ref(),
            );
            app.watch_gpu(rs, &cc.egui_ctx);
            app
        });
    h.state_mut().state.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    h.run();
    h
}

fn paint(s: &mut AppState, x: f64) {
    let layer = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    let mut stroke = s.doc.begin_stroke(layer, &brush).unwrap();
    stroke
        .add_point(&mut s.doc, x, 20.0, 1.0, DVec2::ZERO)
        .unwrap();
    s.doc.end_stroke(stroke).unwrap();
    s.modified = true;
}

fn close_commands(h: &H) -> Vec<&'static str> {
    h.output()
        .viewport_output
        .get(&ViewportId::ROOT)
        .map(|v| {
            v.commands
                .iter()
                .filter_map(|c| match c {
                    ViewportCommand::Close => Some("Close"),
                    ViewportCommand::CancelClose => Some("CancelClose"),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn generations(h: &H) -> usize {
    GenerationStore::new(h.state().state.recovery.session_dir().unwrap())
        .list()
        .unwrap()
        .len()
}

/// 失った知らせを入れて、フレームを進める。
fn lose(h: &mut H) {
    h.state()
        .gpu_watch()
        .expect("見張り")
        .inject_loss("Unknown", "driver reset");
    h.step();
}

#[test]
fn losing_the_device_saves_a_checkpoint_keeps_the_document_and_closes_with_the_reason() {
    let dir = TempDir::new("lose");
    let mut h = window(Lang::Ja, &dir.0.join("recovery"), Arc::default());
    paint(&mut h.state_mut().state, 20.0);
    let (revision, pixel) = {
        let s = &h.state().state;
        (s.doc.revision(), composite_pixel(&s.doc, 20, 20))
    };
    assert_ne!(pixel, [0, 0, 0, 0], "描いてある");
    assert!(
        h.state().view3d_stats().is_some(),
        "失う前は 3D ビューを GPU で描く"
    );
    assert_eq!(generations(&h), 0, "時間では書かれていない");
    let asked = Arc::new(Mutex::new(0u32));
    let seen = asked.clone();
    h.state_mut().answer_close_question(move |_| {
        *seen.lock().unwrap() += 1;
        false
    });

    lose(&mut h);

    let app = h.state();
    assert_eq!(app.gpu_lost().map(|l| l.reason.as_str()), Some("Unknown"));
    assert_eq!(generations(&h), 1, "復旧の書き置きを急いで取る");
    assert!(
        app.state.recovery.is_marked_dirty(),
        "保存していない作業の印が残る（次の起動の復旧の窓から開ける）"
    );
    let s = &app.state;
    assert_eq!(s.doc.revision(), revision, "文書は触らない");
    assert_eq!(composite_pixel(&s.doc, 20, 20), pixel, "描いた絵はそのまま");
    assert!(s.modified, "保存していない印も、そのまま");
    assert!(
        s.message
            .starts_with("GPU の装置が失われたため、続けられません。"),
        "{}",
        s.message
    );
    assert!(s.message.contains("復旧用に保存しました"), "{}", s.message);
    assert!(
        app.view3d_stats().is_none() && app.view3d_adapter().is_none(),
        "GPU の道（3D ビュー）は手放す"
    );
    assert!(
        s.quit && close_commands(&h).contains(&"Close"),
        "終える: {:?}",
        close_commands(&h)
    );
    assert_eq!(
        *asked.lock().unwrap(),
        0,
        "保存していない変更の確かめは聞かない（書き置きがある）"
    );
    // 続けて、もう一度失った知らせが来ても、2 度は扱わない
    h.state()
        .gpu_watch()
        .unwrap()
        .inject_loss("Destroyed", "again");
    h.step();
    assert_eq!(generations(&h), 1);
}

#[test]
fn the_reason_is_in_english_and_a_clean_document_needs_no_checkpoint() {
    let dir = TempDir::new("clean");
    let mut h = window(Lang::En, &dir.0.join("recovery"), Arc::default());
    assert!(!h.state().state.modified);
    lose(&mut h);
    let s = &h.state().state;
    assert_eq!(
        s.message,
        "The GPU device was lost, so YoluPainter cannot continue."
    );
    assert_eq!(generations(&h), 0, "変更が無ければ書き置きは要らない");
    assert!(h.state().gpu_lost().is_some());
    assert!(close_commands(&h).contains(&"Close"));
}

#[test]
fn without_a_recovery_session_the_reason_says_it_could_not_be_saved() {
    let mut state = AppState::new_in(64, 64, Lang::En);
    paint(&mut state, 10.0);
    let mut h = gpu_thread::builder()
        .with_size(vec2(900.0, 600.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .with_render_options(render_options())
        .wgpu()
        .build_eframe(move |cc| {
            let mut app = with_render_state_cpu_canvas(
                YoluApp::for_context(&cc.egui_ctx, state, PenInput::detached()),
                cc.wgpu_render_state.as_ref(),
            );
            app.watch_gpu(cc.wgpu_render_state.as_ref().unwrap(), &cc.egui_ctx);
            app
        });
    h.run();
    lose(&mut h);
    let s = &h.state().state;
    assert!(
        s.message.ends_with("Could not save for recovery."),
        "{}",
        s.message
    );
    assert!(s.modified, "絵は消えない");
}

/// 描いている最中に失っても、描いた所までで確定して書き置きに入る（取り残さない）。
#[test]
fn a_stroke_in_progress_is_committed_before_the_checkpoint() {
    let dir = TempDir::new("stroke");
    let mut h = window(Lang::Ja, &dir.0.join("recovery"), Arc::default());
    {
        let s = &mut h.state_mut().state;
        let layer = s.selected_layer.unwrap();
        let brush = s.stroke_settings(false);
        let mut stroke = s.doc.begin_stroke(layer, &brush).unwrap();
        stroke
            .add_point(&mut s.doc, 30.0, 20.0, 1.0, DVec2::ZERO)
            .unwrap();
        // 札は state に預ける（キャンバスの入力が持つ形）。描いている最中の印が立つ
        s.stroke = Some(stroke);
        s.modified = true;
    }
    assert!(h.state().state.is_stroking());
    lose(&mut h);
    let s = &h.state().state;
    assert!(!s.is_stroking(), "ストロークは取り残さない");
    assert_ne!(
        composite_pixel(&s.doc, 30, 20),
        [0, 0, 0, 0],
        "描いた所は残る"
    );
    assert_eq!(generations(&h), 1);
}

/// 受け手の無い誤りは、落とさずに記録だけする（wgpu の既定は panic）。同じ文は 1 度。
#[test]
fn an_uncaptured_gpu_error_does_not_end_the_app() {
    let dir = TempDir::new("error");
    let mut h = window(Lang::Ja, &dir.0.join("recovery"), Arc::default());
    let watch = h.state().gpu_watch().unwrap().clone();
    watch.inject_error("Validation Error: buffer too large");
    h.step();
    assert!(h.state().gpu_lost().is_none(), "誤りだけでは終わらない");
    assert!(!h.state().state.quit);
    assert!(watch.take().errors.is_empty(), "フレームが読み取った");
}

/// 本物の装置の破棄（`Device::destroy`）が、見張りの受け口を通って、同じ流れになる。
#[test]
fn destroying_the_real_device_runs_the_same_flow() {
    let dir = TempDir::new("destroy");
    let device: Arc<Mutex<Option<Device>>> = Arc::default();
    let mut h = window(Lang::Ja, &dir.0.join("recovery"), device.clone());
    paint(&mut h.state_mut().state, 20.0);
    let device = device.lock().unwrap().clone().expect("装置");
    device.destroy();
    // 破棄は、積んだ仕事が終わった時点で「失った」になり、受け口が呼ばれる（`poll` が進める）
    let deadline = Instant::now() + Duration::from_secs(30);
    while h.state().gpu_lost().is_none() {
        let _ = device.poll(eframe::egui_wgpu::wgpu::PollType::wait_indefinitely());
        h.step();
        assert!(Instant::now() < deadline, "失った知らせが来ない");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        h.state().gpu_lost().map(|l| l.reason.as_str()),
        Some("Destroyed")
    );
    assert_eq!(generations(&h), 1);
    assert!(close_commands(&h).contains(&"Close"));
}

/// 本物の検証の誤り（受け手の無いもの）は、wgpu の既定の panic にならず、見張りが受ける。
#[test]
fn a_real_validation_error_is_recorded_instead_of_panicking() {
    use eframe::egui_wgpu::wgpu;
    let dir = TempDir::new("validation");
    let device: Arc<Mutex<Option<Device>>> = Arc::default();
    let h = window(Lang::Ja, &dir.0.join("recovery"), device.clone());
    let device = device.lock().unwrap().clone().expect("装置");
    let _buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("too-big"),
        size: u64::MAX / 4,
        usage: wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let errors = h.state().gpu_watch().unwrap().take().errors;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].to_lowercase().contains("buffer"), "{errors:?}");
    assert!(
        h.state().gpu_lost().is_none(),
        "検証の誤りだけでは、装置は失われない"
    );
}

/// 装置を失って終わっても、書き置きの「保存していない作業」の印は消えない: 次の起動に復旧の窓が開き、書いた世代が絵と同じ。
/// 変更が無く終わるときは、今までどおり正しく閉じる（印を消す）。
#[test]
fn the_next_start_offers_the_checkpoint_taken_when_the_device_was_lost() {
    use eframe::App;
    let dir = TempDir::new("next");
    let root = dir.0.join("recovery");
    let mut h = window(Lang::Ja, &root, Arc::default());
    paint(&mut h.state_mut().state, 20.0);
    let expected = composite_pixel(&h.state().state.doc, 20, 20);
    lose(&mut h);
    h.state_mut().on_exit();
    drop(h);
    let mut next = AppState::new_in(64, 64, Lang::Ja);
    let probe: SpaceProbe = Arc::new(|_| {
        Some(DiskSpace {
            total: 1 << 40,
            available: 1 << 39,
        })
    });
    next.recovery.set_space_probe(Some(probe));
    next.recovery
        .enable(root.clone(), RecoverySettings::default())
        .unwrap();
    let offered = next
        .recovery
        .window
        .as_ref()
        .expect("落ちた体と同じに、復旧の窓が開く");
    assert_eq!(offered.rows.len(), 1);
    assert!(offered.rows[0].crashed && offered.rows[0].problem.is_none());
    // 開くと、描いた絵が戻る
    let request = yolu_app::recovery::OpenRequest {
        pool: offered.rows[0].pool.clone(),
        id: offered.rows[0].id.clone(),
    };
    next.recovery_open(request);
    assert_eq!(composite_pixel(&next.doc, 20, 20), expected);

    // 変更が無いまま装置を失った: 正しく閉じる（次の起動に窓は出ない）
    let clean_root = dir.0.join("clean");
    let mut h = window(Lang::Ja, &clean_root, Arc::default());
    lose(&mut h);
    h.state_mut().on_exit();
    drop(h);
    let mut again = AppState::new_in(64, 64, Lang::Ja);
    let probe: SpaceProbe = Arc::new(|_| {
        Some(DiskSpace {
            total: 1 << 40,
            available: 1 << 39,
        })
    });
    again.recovery.set_space_probe(Some(probe));
    again
        .recovery
        .enable(clean_root, RecoverySettings::default())
        .unwrap();
    assert!(again.recovery.window.is_none());
}

/// 書き置きの書き込みが終わらない（遅いディスク・止まったネットワークドライブ）ときも、装置を失って終わる処理は期限で返る。急いで取る側
/// （`handle_gpu_lost`）も期限つきで、書き込みは走ったまま。置換は最後の 1 回なので、確定しなかった作りかけは残らず、止めた書き込みが
/// あとで終われば世代が 1 つ増える（印も付く）。
#[test]
fn a_stalled_checkpoint_write_does_not_hold_the_exit() {
    use eframe::App;
    let dir = TempDir::new("stalled");
    let mut h = window(Lang::Ja, &dir.0.join("recovery"), Arc::default());
    h.state_mut().set_gpu_lost_wait(Duration::from_millis(300));
    // 書き手を、組み立ての前で止める。止めたままにしても、直す前の（期限なしの）待ちが試験ごと固まらないよう、4 秒で放す
    let release = Arc::new(AtomicBool::new(false));
    let held = release.clone();
    h.state_mut()
        .state
        .recovery
        .set_fault(Some(Arc::new(move |stage: &str| {
            if stage == "snapshot" {
                while !held.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            Ok(())
        })));
    let releaser = {
        let release = release.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(4));
            release.store(true, Ordering::SeqCst);
        })
    };
    paint(&mut h.state_mut().state, 20.0);

    let started = Instant::now();
    lose(&mut h);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "急いで取る待ちも期限で返る: {:?}",
        started.elapsed()
    );
    assert!(h.state().gpu_lost().is_some());
    assert!(
        h.state().state.message.contains("保存できませんでした"),
        "{}",
        h.state().state.message
    );
    assert_eq!(generations(&h), 0, "書き込みはまだ確定していない");

    let started = Instant::now();
    h.state_mut().on_exit();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "終わる処理は、書き込みを期限までしか待たない: {:?}",
        started.elapsed()
    );
    assert_eq!(
        generations(&h),
        0,
        "止めた書き込みの作りかけは、確定していない世代として残らない"
    );

    // 止めていた書き込みが終われば、世代が 1 つ入って印が付く
    release.store(true, Ordering::SeqCst);
    releaser.join().unwrap();
    h.state_mut().state.recovery_wait();
    assert_eq!(generations(&h), 1);
    assert!(h.state().state.recovery.is_marked_dirty());
}
