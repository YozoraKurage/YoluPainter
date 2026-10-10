//! 主の GPU（wgpu のデバイス）を失ったとき: 描いていた絵（文書）は触らず、復旧の書き置きを急いで取り、GPU の道（3D ビュー・キャンバスの GPU の表示）を
//! 手放して CPU の表示へ落とし、理由を出して、フレームに戻らずにプロセスを終える（失った GPU では 1 度も描かない）。書き置きの「保存していない作業」の印は
//! 残る（次の起動の復旧のウィンドウから開ける）。受け手の無い誤りは panic にせず記録する。失った知らせを入れる口（`GpuWatch::inject_loss`）と、
//! 本物のデバイスの破棄（`Device::destroy`）の両方で、受ける側の流れを通す。
//!
//! プロセスの終え方は `set_gpu_lost_exit` で替える: 本物（`std::process::exit`）は戻らないので、替える物も戻らず、巻き戻しで抜ける（`Exited`）。
//! `step` が巻き戻して返れば、`ui` の続き（描画）が走らなかったことになる。プロセスそのものが終わる道（終了コード・落ちた記録・普段のログ・書き置き）は、
//! `app::gpu_lost` の単体試験が子プロセスで確かめる。
//!
//! ウィンドウごとに自分のデバイスを作る（`.wgpu()`。共用の接続 `common::shared_gpu` は使わない）: デバイスを破棄する試験・見張りの受け口を付ける試験が、
//! 共用のデバイスを壊したり、ほかの試験の誤りの受け口を取り替えたりしないため。ウィンドウは `gpu_thread::builder` の貸し出しで 1 つずつ作る。
use crate::common;

use std::cell::RefCell;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use egui::vec2;
use egui_kittest::Harness;
use yolu_app::app::{FramePoint, GPU_LOST_EXIT_CODE};
use yolu_app::engine::{composite_pixel, DVec2};
use yolu_app::gpu_watch::GpuWatch;
use yolu_app::lang::Lang;
use yolu_app::pen::PenInput;
use yolu_app::recovery::{DiskSpace, RecoverySettings, SpaceProbe};
use yolu_app::state::{Action, AppState};
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

/// 終え方の代わりに巻き戻しで運ぶ、終了コード。
#[derive(Debug)]
struct Exited(i32);

/// 本物の `process::exit` と同じく戻らない終え方（巻き戻しで抜ける。`step` が返れば、`ui` の続きは走っていない）。
fn exit_by_unwinding(code: i32) {
    resume_unwind(Box::new(Exited(code)))
}

/// 見張りと、戻らない終え方（試験用）を付ける。
fn watched(mut app: YoluApp, rs: &eframe::egui_wgpu::RenderState, ctx: &egui::Context) -> YoluApp {
    app.watch_gpu(rs, ctx);
    app.set_gpu_lost_exit(exit_by_unwinding);
    app
}

/// フレームを 1 つ進める。GPU を失って終わる道に入れば、終え方が巻き戻すので、そのときの終了コードを返す（普通に進めば None）。
fn step(h: &mut H) -> Option<i32> {
    match catch_unwind(AssertUnwindSafe(|| h.step())) {
        Ok(()) => None,
        Err(payload) => match payload.downcast::<Exited>() {
            Ok(exit) => Some(exit.0),
            Err(other) => resume_unwind(other),
        },
    }
}

/// 復旧を動かし、見張りを付けたウィンドウ（描いて、保存していない印を付けてある）。デバイスは `device` へ控える。
fn window(lang: Lang, root: &std::path::Path, device: Arc<Mutex<Option<Device>>>) -> H {
    let mut state = AppState::new_in(64, 64, lang);
    let probe: SpaceProbe = Arc::new(|_| {
        Some(DiskSpace {
            total: 1 << 40,
            available: 1 << 39,
        })
    });
    state.recovery.set_space_probe(Some(probe));
    // 間隔は長く（GPU を失ったときに「急いで」書くことを確かめる。時間では書かれない）
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
            watched(
                with_render_state_cpu_canvas(
                    YoluApp::for_context(&cc.egui_ctx, state, PenInput::detached()),
                    cc.wgpu_render_state.as_ref(),
                ),
                rs,
                &cc.egui_ctx,
            )
        });
    h.state_mut().state.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    // 中央は 1 つの組（`common::app` と同じ並び）
    h.state_mut().dock = common::tabbed_center_dock(900.0);
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

fn generations(h: &H) -> usize {
    GenerationStore::new(h.state().state.recovery.session_dir().unwrap())
        .list()
        .unwrap()
        .len()
}

/// 次の起動が復旧のウィンドウに並べる行の数（落ちた体の書き置きがあれば 1 以上）。
fn offered_rows(root: &std::path::Path) -> usize {
    let mut next = AppState::new_in(64, 64, Lang::Ja);
    let probe: SpaceProbe = Arc::new(|_| {
        Some(DiskSpace {
            total: 1 << 40,
            available: 1 << 39,
        })
    });
    next.recovery.set_space_probe(Some(probe));
    next.recovery
        .enable(root.to_path_buf(), RecoverySettings::default())
        .unwrap();
    next.recovery.window.as_ref().map_or(0, |w| w.rows.len())
}

/// 失った知らせを入れて、フレームを進める。フレームには戻らずに終える（終了コードが返る）。
fn lose(h: &mut H) -> i32 {
    h.state()
        .gpu_watch()
        .expect("見張り")
        .inject_loss("Unknown", "driver reset");
    step(h).expect("失ったら、フレームに戻らずに終える")
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

    let code = lose(&mut h);
    assert_eq!(code, GPU_LOST_EXIT_CODE, "決まった終了コードで終える");

    let app = h.state();
    assert_eq!(app.gpu_lost().map(|l| l.reason.as_str()), Some("Unknown"));
    assert_eq!(generations(&h), 1, "復旧の書き置きを急いで取る");
    assert!(
        app.state.recovery.is_marked_dirty(),
        "保存していない作業の印が残る（次の起動の復旧のウィンドウから開ける）"
    );
    let s = &app.state;
    assert_eq!(s.doc.revision(), revision, "文書は触らない");
    assert_eq!(composite_pixel(&s.doc, 20, 20), pixel, "描いた絵はそのまま");
    assert!(s.modified, "保存していない印も、そのまま");
    assert!(
        s.message
            .starts_with("GPU でエラーが起きたため、続けられません。"),
        "{}",
        s.message
    );
    assert!(s.message.contains("復旧用に保存しました"), "{}", s.message);
    assert!(
        app.view3d_stats().is_none() && app.view3d_adapter().is_none(),
        "GPU の道（3D ビュー）は手放す"
    );
    assert!(
        !s.quit,
        "閉じる頼みの流れ（描き終えてから閉じる）には戻らず、その場で終える"
    );
    assert_eq!(
        *asked.lock().unwrap(),
        0,
        "保存していない変更の確かめは聞かない（書き置きがある）"
    );
    // もう一度失った知らせが来ても、知らせは 1 度だけ（2 度は扱わない）
    let watch = h.state().gpu_watch().unwrap();
    watch.inject_loss("Destroyed", "again");
    assert_eq!(watch.take().lost, None);
    assert_eq!(generations(&h), 1);
}

#[test]
fn the_reason_is_in_english_and_a_clean_document_needs_no_checkpoint() {
    let dir = TempDir::new("clean");
    let mut h = window(Lang::En, &dir.0.join("recovery"), Arc::default());
    assert!(!h.state().state.modified);
    assert_eq!(lose(&mut h), GPU_LOST_EXIT_CODE);
    let s = &h.state().state;
    assert_eq!(
        s.message,
        "A GPU error occurred, so YoluPainter cannot continue."
    );
    assert!(h.state().gpu_lost().is_some());
    // 変更が無ければ書き置きは要らない: 正しく閉じて、次の起動に復旧のウィンドウは出ない
    drop(h);
    assert_eq!(offered_rows(&dir.0.join("recovery")), 0);
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
            watched(
                with_render_state_cpu_canvas(
                    YoluApp::for_context(&cc.egui_ctx, state, PenInput::detached()),
                    cc.wgpu_render_state.as_ref(),
                ),
                cc.wgpu_render_state.as_ref().unwrap(),
                &cc.egui_ctx,
            )
        });
    // 中央は 1 つの組（`common::app` と同じ並び）
    h.state_mut().dock = common::tabbed_center_dock(900.0);
    h.run();
    assert_eq!(lose(&mut h), GPU_LOST_EXIT_CODE);
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
    assert_eq!(lose(&mut h), GPU_LOST_EXIT_CODE);
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

/// 本物のデバイスの破棄（`Device::destroy`）が、見張りの受け口を通って、同じ流れになる。
#[test]
fn destroying_the_real_device_runs_the_same_flow() {
    let dir = TempDir::new("destroy");
    let device: Arc<Mutex<Option<Device>>> = Arc::default();
    let mut h = window(Lang::Ja, &dir.0.join("recovery"), device.clone());
    paint(&mut h.state_mut().state, 20.0);
    let device = device.lock().unwrap().clone().expect("デバイス");
    device.destroy();
    // 破棄は、積んだ仕事が終わった時点で「失った」になり、受け口が呼ばれる（`poll` が進める）
    let deadline = Instant::now() + Duration::from_secs(30);
    let code = loop {
        let _ = device.poll(eframe::egui_wgpu::wgpu::PollType::wait_indefinitely());
        if let Some(code) = step(&mut h) {
            break code;
        }
        assert!(Instant::now() < deadline, "失った知らせが来ない");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(code, GPU_LOST_EXIT_CODE);
    assert_eq!(
        h.state().gpu_lost().map(|l| l.reason.as_str()),
        Some("Destroyed")
    );
    assert_eq!(generations(&h), 1);
}

/// 本物の検証の誤り（受け手の無いもの）は、wgpu の既定の panic にならず、見張りが受ける。
#[test]
fn a_real_validation_error_is_recorded_instead_of_panicking() {
    use eframe::egui_wgpu::wgpu;
    let dir = TempDir::new("validation");
    let device: Arc<Mutex<Option<Device>>> = Arc::default();
    let h = window(Lang::Ja, &dir.0.join("recovery"), device.clone());
    let device = device.lock().unwrap().clone().expect("デバイス");
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
        "検証の誤りだけでは、GPU は失われない"
    );
}

/// GPU を失って終わっても、書き置きの「保存していない作業」の印は消えない: 次の起動に復旧のウィンドウが開き、書いた世代が絵と同じ。
/// 変更が無く終わるときは、今までどおり正しく閉じる（印を消す）。後始末（`on_exit`）は、終える処理の中で済む（試験から呼び直さない）。
#[test]
fn the_next_start_offers_the_checkpoint_taken_when_the_gpu_was_lost() {
    let dir = TempDir::new("next");
    let root = dir.0.join("recovery");
    let mut h = window(Lang::Ja, &root, Arc::default());
    paint(&mut h.state_mut().state, 20.0);
    let expected = composite_pixel(&h.state().state.doc, 20, 20);
    assert_eq!(lose(&mut h), GPU_LOST_EXIT_CODE);
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
        .expect("落ちた体と同じに、復旧のウィンドウが開く");
    assert_eq!(offered.rows.len(), 1);
    assert!(offered.rows[0].crashed && offered.rows[0].problem.is_none());
    // 開くと、描いた絵が戻る
    let request = yolu_app::recovery::OpenRequest {
        pool: offered.rows[0].pool.clone(),
        id: offered.rows[0].id.clone(),
    };
    next.recovery_open(request);
    assert_eq!(composite_pixel(&next.doc, 20, 20), expected);

    // 変更が無いまま GPU を失った: 正しく閉じる（次の起動にウィンドウは出ない）
    let clean_root = dir.0.join("clean");
    let mut h = window(Lang::Ja, &clean_root, Arc::default());
    assert_eq!(lose(&mut h), GPU_LOST_EXIT_CODE);
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

/// 書き置きの書き込みが終わらない（遅いディスク・止まったネットワークドライブ）ときも、GPU を失って終える処理は期限で返る。急いで取る側と、
/// 終える前に書き込み中の分を待つ側（`on_exit`）の両方が期限つきで、書き込みは走ったまま。置換は最後の 1 回なので、確定しなかった作りかけは
/// 残らず、止めた書き込みがあとで終われば世代が 1 つ増える（印も付く）。
#[test]
fn a_stalled_checkpoint_write_does_not_hold_the_exit() {
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
    assert_eq!(lose(&mut h), GPU_LOST_EXIT_CODE);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "急いで取る待ちも、終える前の待ちも期限で返る（期限は 300 ms ずつ。止めた書き手は 4 秒で放される）: {:?}",
        started.elapsed()
    );
    assert!(h.state().gpu_lost().is_some());
    assert!(
        h.state().state.message.contains("保存できませんでした"),
        "{}",
        h.state().state.message
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

/// 隠れているウィンドウ（最小化など）でも、失った知らせを受ければ、フレームに戻らずに終える（`ui` が回らない間の道 `tick_hidden`）。
#[test]
fn a_hidden_window_also_ends_without_returning() {
    let dir = TempDir::new("hidden");
    let mut h = window(Lang::En, &dir.0.join("recovery"), Arc::default());
    paint(&mut h.state_mut().state, 20.0);
    h.state()
        .gpu_watch()
        .unwrap()
        .inject_loss("Unknown", "while hidden");
    let ctx = h.ctx.clone();
    let result = catch_unwind(AssertUnwindSafe(|| h.state_mut().tick_hidden(&ctx)));
    let exited = result.expect_err("戻らずに終える");
    assert_eq!(exited.downcast::<Exited>().unwrap().0, GPU_LOST_EXIT_CODE);
    assert_eq!(generations(&h), 1, "隠れていても書き置きを取る");
}

/// 保存の途中で失ったら、書き置きの要不要を決める前に、保存が終わるまで待つ（保存は途中で止めず、置換で確定する）。
/// 保存が済んで変更が無ければ、書き置きは要らない（画面・ウィンドウの文もそう言う）。
#[test]
fn a_save_in_progress_is_waited_for_before_the_exit() {
    let dir = TempDir::new("saving");
    let path = dir.0.join("work.ylp");
    let root = dir.0.join("recovery");
    let mut h = window(Lang::En, &root, Arc::default());
    paint(&mut h.state_mut().state, 20.0);
    h.state_mut().state.save.background = true;
    let hold = h.state_mut().state.save.hold_next();
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    assert!(h.state().state.is_saving());
    // 保存は 0.3 秒あとに動き出す（待つ上限は 60 秒）
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        hold.release();
    });
    assert_eq!(lose(&mut h), GPU_LOST_EXIT_CODE);
    releaser.join().unwrap();
    assert!(
        !h.state().state.is_saving() && path.is_file(),
        "保存が終わってから終える"
    );
    assert_eq!(
        h.state().state.message,
        "A GPU error occurred, so YoluPainter cannot continue.",
        "保存が済んだので、書き置きは要らない"
    );
    drop(h);
    assert_eq!(
        offered_rows(&root),
        0,
        "保存が済んで変更が無いので、正しく閉じる"
    );
}

/// 保存の途中に描き足してから失っても、保存が終わるのを待ってから書き置きを取る（保存の間は書き置きの見張りが止まるので、待たずに
/// 決めると、保存の間に描いた分が書き置きに入らない）。画面・ウィンドウの文は、取れた書き置きのとおり。
#[test]
fn drawing_during_a_save_is_in_the_checkpoint_taken_after_the_save() {
    let dir = TempDir::new("drawsave");
    let root = dir.0.join("recovery");
    let mut h = window(Lang::En, &root, Arc::default());
    paint(&mut h.state_mut().state, 20.0);
    h.state_mut().state.save.background = true;
    let hold = h.state_mut().state.save.hold_next();
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(dir.0.join("work.ylp")));
    assert!(h.state().state.is_saving());
    // 保存の間に描き足す
    paint(&mut h.state_mut().state, 40.0);
    assert!(h.state().state.modified);
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        hold.release();
    });
    assert_eq!(lose(&mut h), GPU_LOST_EXIT_CODE);
    releaser.join().unwrap();
    assert!(!h.state().state.is_saving());
    assert_eq!(generations(&h), 1, "保存のあとに書き置きを取る");
    assert!(
        h.state()
            .state
            .message
            .ends_with("Your work was saved for recovery."),
        "{}",
        h.state().state.message
    );
    drop(h);
    assert_eq!(offered_rows(&root), 1, "次の起動の復旧のウィンドウに並ぶ");
}

/// 保存が期限までに終わらなくても、終える処理は期限で返る（保存を待ち続けて固まらない）。そのときは、保存が間に合わなかったことを
/// 画面・記録に書き、書き置きの印は消さない（保存が途中で切れるので、次の起動で前の書き置きから続きを開ける）。
#[test]
fn a_save_that_does_not_finish_in_time_does_not_hold_the_exit_and_keeps_the_recovery_mark() {
    let dir = TempDir::new("slowsave");
    let root = dir.0.join("recovery");
    let mut h = window(Lang::En, &root, Arc::default());
    h.state_mut()
        .set_gpu_lost_save_wait(Duration::from_millis(300));
    paint(&mut h.state_mut().state, 20.0);
    // 前の書き置きがある（保存が切れても、これは残る）
    assert!(h.state_mut().state.recovery_flush());
    assert_eq!(generations(&h), 1);
    h.state_mut().state.save.background = true;
    let hold = h.state_mut().state.save.hold_next();
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(dir.0.join("slow.ylp")));
    let started = Instant::now();
    assert_eq!(lose(&mut h), GPU_LOST_EXIT_CODE);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert!(h.state().state.is_saving(), "保存はまだ終わっていない");
    assert!(
        h.state()
            .state
            .message
            .ends_with("Could not save for recovery."),
        "{}",
        h.state().state.message
    );
    assert!(
        h.state().state.recovery.is_enabled(),
        "保存の途中のまま終えるので、書き置きの印を正しく閉じる側へ進めない"
    );
    hold.release();
    h.state_mut().state.wait_save();
    drop(h);
    assert_eq!(
        offered_rows(&root),
        1,
        "印が残り、前の書き置きが次の起動の復旧のウィンドウに並ぶ"
    );
}

/// 保存の途中で失い、その保存が失敗して変更が戻ったときは、保存を待ってから書き置きを取って終える（保存の頼みは「変更あり」の印を
/// 下ろすので、待たずに決めると書き置きが要らないと見える）。ウィンドウの文も、取れた書き置きのとおり。
#[test]
fn a_failed_save_in_progress_still_leaves_a_checkpoint() {
    let dir = TempDir::new("failedsave");
    let target = dir.0.join("work.ylp");
    let mut h = window(Lang::En, &dir.0.join("recovery"), Arc::default());
    paint(&mut h.state_mut().state, 20.0);
    h.state_mut().state.save.background = true;
    let hold = h.state_mut().state.save.hold_next();
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(target.clone()));
    assert!(h.state().state.is_saving());
    assert!(!h.state().state.modified, "保存の頼みが印を下ろしている");
    // 書き込み先の名前にフォルダができて、置換できず保存は失敗する
    std::fs::create_dir(&target).unwrap();
    hold.release();
    assert_eq!(lose(&mut h), GPU_LOST_EXIT_CODE);
    assert!(h.state().state.modified, "保存に失敗したので、変更は戻る");
    assert_eq!(generations(&h), 1, "書き置きを取ってから終える");
    assert!(
        h.state()
            .state
            .message
            .ends_with("Your work was saved for recovery."),
        "{}",
        h.state().state.message
    );
}

/// フレームの終わり（`ui` の終わり。eframe がそのフレームを描く直前）でも見張りを読む: フレームの途中で失った知らせが入っても、
/// そのフレームを描く前に終える。
#[test]
fn a_loss_found_at_the_end_of_the_frame_ends_before_the_frame_is_painted() {
    let dir = TempDir::new("uiend");
    let mut h = window(Lang::En, &dir.0.join("recovery"), Arc::default());
    paint(&mut h.state_mut().state, 20.0);
    let watch = h.state().gpu_watch().unwrap().clone();
    let seen: Rc<RefCell<Vec<FramePoint>>> = Rc::default();
    let log = seen.clone();
    h.state_mut().set_frame_probe(move |point| {
        log.borrow_mut().push(point);
        // フレームの本体を描き終えたあとで失った（読む直前に入れる）
        if point == FramePoint::UiEnd {
            watch.inject_loss("Unknown", "during the frame");
        }
    });
    let code = step(&mut h).expect("フレームの終わりで終える");
    assert_eq!(code, GPU_LOST_EXIT_CODE);
    assert_eq!(*seen.borrow(), vec![FramePoint::UiEnd]);
    assert_eq!(generations(&h), 1);
}

/// 別ウィンドウ（外へ出したパネル）が OS のウィンドウとして描かれるのは、メインウィンドウの `ui` の途中（`show_viewport_immediate`）。
/// 失った知らせが入ったら、次の別ウィンドウを描く前に終える。別ウィンドウを取り出している間に終えても、並びのファイルは別ウィンドウを
/// 残したまま（終えるとき、いまの状態から並びを書き直さない）。
/// 試験のウィンドウは別ウィンドウを egui のウィンドウとしてメインウィンドウの中に描く（OS のウィンドウは出せない）ので、読む場所（`FramePoint`）
/// に着いた順と、その読みで抜けたことで確かめる。`show_viewport_immediate` から戻った直後の読みは、この描き方でも同じ位置で通る。
mod detached_windows {
    use super::*;
    use yolu_app::detach::DockOp;
    use yolu_app::layout;
    use yolu_app::Tab;

    struct Fixture {
        h: H,
        layout: PathBuf,
        before: String,
        serials: [u64; 2],
        seen: Rc<RefCell<Vec<FramePoint>>>,
        _dir: TempDir,
    }

    /// 別ウィンドウが 2 つある状態で、並びのファイルが書いてあり、`at` に着いたときに失った知らせを入れる。
    fn fixture(tag: &str, at: impl Fn(&[u64; 2]) -> FramePoint) -> Fixture {
        let dir = TempDir::new(tag);
        let settings = dir.0.join("settings.conf");
        let layout = dir.0.join("layout.json");
        let mut h = gpu_thread::builder()
            .with_size(vec2(1280.0, 800.0))
            .with_pixels_per_point(1.0)
            .with_step_dt(1.0 / 60.0)
            .with_max_steps(120)
            .renderer(shared_gpu::renderer())
            .build_eframe(move |cc| {
                with_render_state_cpu_canvas(
                    YoluApp::for_context_with_settings(
                        &cc.egui_ctx,
                        Some(settings),
                        PenInput::detached(),
                    ),
                    cc.wgpu_render_state.as_ref(),
                )
            });
        h.run();
        h.state_mut()
            .state
            .apply(Action::Dock(DockOp::Detach(Tab::History)));
        h.run();
        h.state_mut()
            .state
            .apply(Action::Dock(DockOp::Detach(Tab::Channels)));
        h.run();
        h.run();
        assert_eq!(h.state().detached.windows.len(), 2);
        let serials = [
            h.state().detached.windows[0].serial,
            h.state().detached.windows[1].serial,
        ];
        // 失う前の通常の終わりと同じ書き方で、並びを書いておく（別ウィンドウが 2 つ入っている）
        eframe::App::on_exit(h.state_mut());
        let before = std::fs::read_to_string(&layout).expect("並びのファイル");
        assert_eq!(
            layout::parse(&before).detached.len(),
            2,
            "並びのファイルに別ウィンドウが入っている"
        );
        let watch = GpuWatch::default();
        h.state_mut().set_gpu_watch(watch.clone());
        h.state_mut().set_gpu_lost_exit(exit_by_unwinding);
        let seen: Rc<RefCell<Vec<FramePoint>>> = Rc::default();
        let log = seen.clone();
        let target = at(&serials);
        h.state_mut().set_frame_probe(move |point| {
            log.borrow_mut().push(point);
            if point == target {
                watch.inject_loss("Unknown", "while drawing");
            }
        });
        Fixture {
            h,
            layout,
            before,
            serials,
            seen,
            _dir: dir,
        }
    }

    fn assert_layout_unchanged(f: &Fixture) {
        let after = std::fs::read_to_string(&f.layout).expect("並びのファイル");
        assert_eq!(
            after, f.before,
            "終えるとき、別ウィンドウを取り出している間の状態から並びを書き直さない"
        );
    }

    #[test]
    fn a_loss_before_the_outside_windows_are_drawn_ends_without_drawing_any() {
        let mut f = fixture("before", |_| FramePoint::BeforeDetached);
        assert_eq!(step(&mut f.h), Some(GPU_LOST_EXIT_CODE));
        assert_eq!(*f.seen.borrow(), vec![FramePoint::BeforeDetached]);
        assert_eq!(
            f.h.state().detached.windows.len(),
            2,
            "まだ取り出していない"
        );
        assert_layout_unchanged(&f);
    }

    #[test]
    fn a_loss_at_the_end_of_an_outside_window_ends_before_it_is_painted() {
        let mut f = fixture("passend", |s| FramePoint::DetachedPassEnd(s[0]));
        assert_eq!(step(&mut f.h), Some(GPU_LOST_EXIT_CODE));
        let s = f.serials;
        assert_eq!(
            *f.seen.borrow(),
            vec![
                FramePoint::BeforeDetached,
                FramePoint::DetachedPassEnd(s[0])
            ],
            "最初の別ウィンドウの描画の終わりで抜け、次の別ウィンドウには進まない"
        );
        assert_eq!(
            f.h.state().detached.windows.len(),
            0,
            "別ウィンドウを取り出している間に終えた"
        );
        assert_layout_unchanged(&f);
    }

    #[test]
    fn a_loss_while_one_outside_window_is_painted_ends_before_the_next_one() {
        let mut f = fixture("after", |s| FramePoint::AfterDetached(s[0]));
        assert_eq!(step(&mut f.h), Some(GPU_LOST_EXIT_CODE));
        let s = f.serials;
        assert_eq!(
            *f.seen.borrow(),
            vec![
                FramePoint::BeforeDetached,
                FramePoint::DetachedPassEnd(s[0]),
                FramePoint::AfterDetached(s[0]),
            ],
            "1 つ目を描いた直後に抜け、2 つ目は描かない"
        );
        assert_layout_unchanged(&f);
    }

    #[test]
    fn a_loss_after_every_outside_window_still_ends_at_the_end_of_the_frame() {
        let mut f = fixture("uiend", |_| FramePoint::UiEnd);
        assert_eq!(step(&mut f.h), Some(GPU_LOST_EXIT_CODE));
        let s = f.serials;
        let seen = f.seen.borrow();
        assert_eq!(seen.last(), Some(&FramePoint::UiEnd));
        assert!(
            seen.contains(&FramePoint::AfterDetached(s[1])),
            "2 つとも描いたあと: {seen:?}"
        );
        drop(seen);
        assert_layout_unchanged(&f);
    }
}
