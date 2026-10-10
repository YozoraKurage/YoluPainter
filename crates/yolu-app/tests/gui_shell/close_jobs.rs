//! 閉じる前の確かめ: 走っている仕事（利用者が結果を待っている書き出しなど）は、保存していない変更が無くても知らせる。
//! 閉じると取り消される仕事の一覧（`windows::close_jobs`）・確かめの文（`close_question`）・取り消して止まるのを待つ後始末（`stop_jobs`）。
//! 保存の仕事は一覧に入らない（保存は閉じる流れが待つ。`save_close.rs`）。
use crate::common;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use common::*;
use egui::vec2;
use egui_kittest::Harness;
use yolu_app::lang::Lang;
use yolu_app::pen::PenInput;
use yolu_app::psd::PsdAction;
use yolu_app::shelf::ShelfOp;
use yolu_app::state::{Action, AppState};
use yolu_app::windows::{close_jobs, close_question, stop_jobs, CloseJob};
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-closejobs-{tag}-{}-{}",
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

/// 何も描いていない（保存していない変更が無い）ウィンドウ。
fn window(lang: Lang) -> H {
    let mut h = gpu_thread::builder()
        .with_size(vec2(900.0, 600.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(shared_gpu::renderer())
        .build_eframe(move |cc| {
            with_render_state_cpu_canvas(
                YoluApp::for_context(
                    &cc.egui_ctx,
                    AppState::new_in(64, 64, lang),
                    PenInput::detached(),
                ),
                cc.wgpu_render_state.as_ref(),
            )
        });
    h.state_mut().state.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    // 中央は 1 つの組（`common::app` と同じ並び）
    h.state_mut().dock = common::tabbed_center_dock(900.0);
    h.run();
    assert!(!h.state().state.modified);
    h
}

/// 取消が来るまで始めない PSD の書き出しを走らせる。
fn start_parked_psd_export(s: &mut AppState, dir: &TempDir) {
    s.psd.park_next = true;
    s.apply(Action::Psd(PsdAction::Export(dir.0.join("out.psd"))));
    assert!(s.psd.is_busy());
}

#[test]
fn a_running_job_is_asked_about_before_closing_even_without_unsaved_changes() {
    let dir = TempDir::new("ask");
    let mut h = window(Lang::Ja);
    let asked: Rc<RefCell<Vec<Vec<CloseJob>>>> = Rc::default();
    let reply = Rc::new(Cell::new(false));
    let (seen, answer) = (asked.clone(), reply.clone());
    h.state_mut().answer_close_question(move |state| {
        seen.borrow_mut().push(close_jobs(state));
        answer.get()
    });
    start_parked_psd_export(&mut h.state_mut().state, &dir);
    h.state_mut().state.apply(Action::Quit);
    h.step();
    assert_eq!(
        *asked.borrow(),
        vec![vec![CloseJob::PsdExport]],
        "変更が無くても、走っている書き出しを問う"
    );
    assert!(!h.state().is_closing(), "「いいえ」なら閉じない");
    assert!(
        h.state().state.psd.is_busy(),
        "閉じないなら、仕事は走ったまま"
    );
    // 「はい」なら、仕事を取り消して閉じる
    reply.set(true);
    h.state_mut().state.apply(Action::Quit);
    h.step();
    assert_eq!(asked.borrow().len(), 2);
    assert!(h.state().is_closing());
    assert!(
        !h.state().state.psd.is_busy(),
        "閉じる前に、仕事を取り消して止める"
    );
}

#[test]
fn closing_with_no_job_and_no_change_asks_nothing() {
    let mut h = window(Lang::Ja);
    let asked = Rc::new(Cell::new(0u32));
    let seen = asked.clone();
    h.state_mut().answer_close_question(move |_| {
        seen.set(seen.get() + 1);
        false
    });
    h.state_mut().state.apply(Action::Quit);
    h.step();
    assert_eq!(asked.get(), 0, "仕事も変更も無ければ聞かない");
    assert!(h.state().is_closing());
}

#[test]
fn the_question_names_the_jobs_and_the_unsaved_changes_in_both_languages() {
    let jobs = [CloseJob::Export, CloseJob::PsdExport, CloseJob::Distribute];
    assert_eq!(close_question(Lang::Ja, false, &[]), "");
    assert_eq!(
        close_question(Lang::Ja, true, &[]),
        "保存していない変更があります。変更を捨てて終わりますか？",
        "仕事が無いときは、今までの問いのまま"
    );
    assert_eq!(
        close_question(Lang::En, true, &[]),
        "There are unsaved changes. Discard them and quit?"
    );
    assert_eq!(
        close_question(Lang::Ja, false, &jobs),
        "走っている仕事（書き出し・PSD の書き出し・配布用に保存）は取り消されます。終わりますか？"
    );
    assert_eq!(
        close_question(Lang::En, false, &jobs),
        "Running jobs (Export, PSD export, Save for distribution) will be cancelled. Quit?"
    );
    let both_ja = close_question(Lang::Ja, true, &jobs[..1]);
    assert!(
        both_ja.contains("保存していない変更")
            && both_ja.contains("書き出し")
            && both_ja.contains("変更を捨てて"),
        "{both_ja}"
    );
    let both_en = close_question(Lang::En, true, &jobs[..1]);
    assert!(
        both_en.contains("unsaved changes")
            && both_en.contains("Export")
            && both_en.contains("Discard"),
        "{both_en}"
    );
    // 日本語の問いに英語は、英語の問いに日本語は混ざらない
    for job in [
        CloseJob::Bake,
        CloseJob::Export,
        CloseJob::PsdImport,
        CloseJob::PsdExport,
        CloseJob::Distribute,
        CloseJob::UpdateDownload,
        CloseJob::BrushImport,
        CloseJob::LibraryWrite,
        CloseJob::ShelfSave,
        CloseJob::ShelfImport,
    ] {
        let (ja, en) = (job.label(Lang::Ja), job.label(Lang::En));
        assert!(!ja.is_empty() && !en.is_empty() && ja != en);
        assert!(en.is_ascii(), "{en}");
    }
}

/// 素材の保存（別のスレッド）は一覧に出て、後始末が取り消して、スレッドが終わるまで待つ。棚には入らない。
#[test]
fn a_material_save_is_listed_and_cancelled_and_waited_for_when_stopping_jobs() {
    let mut s = AppState::new(32, 32);
    let layer = s.selected_layer.unwrap();
    s.shelf.async_bytes = 0; // 小さな素材でも別のスレッドへ
    s.shelf.hold_saves(true);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(layer)));
    assert_eq!(s.shelf.pending_save(), Some(false));
    assert_eq!(close_jobs(&s), vec![CloseJob::ShelfSave]);
    assert_eq!(s.shelf.saves_running(), 1);
    s.shelf.hold_saves(false);
    stop_jobs(&mut s, Duration::from_secs(10));
    assert!(close_jobs(&s).is_empty(), "取り消した");
    assert_eq!(s.shelf.saves_running(), 0, "スレッドが終わるまで待つ");
    assert!(
        s.shelf.resources().is_empty(),
        "取り消した保存は棚に入れない"
    );
}

/// 保存の仕事は一覧に入らない（閉じる流れが待つ）。ベイクと書き出しが無いときは空。
#[test]
fn an_idle_state_has_no_jobs() {
    let s = AppState::new(32, 32);
    assert!(close_jobs(&s).is_empty());
}

fn files_under(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| {
                    if e.path().is_dir() {
                        files_under(&e.path())
                    } else {
                        1
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

/// ライブラリのフォルダだけを、この試験の使い捨てのフォルダにした状態（同梱の素材は並べない）。
fn library_state(dir: &TempDir) -> AppState {
    let mut s = AppState::new(32, 32);
    s.prefs.settings.library_folder = Some(dir.0.join("library"));
    s.shelf.show_builtin = false;
    s
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../yolu-io/tests/fixtures/smart")
        .join(name)
}

/// ライブラリへの書き込み（別のスレッド）は一覧に出て、後始末が取り消して、スレッドが終わるまで待つ。ライブラリには何も書かれない。
/// 棚の素材を入れる道（`ToLibrary`）と、外のファイルを足す道（`LibraryAddFiles`）の両方。
#[test]
fn a_library_write_is_listed_and_cancelled_and_waited_for_when_stopping_jobs() {
    type Start = fn(&mut AppState);
    let starts: [(&str, Start); 2] = [
        ("ToLibrary", |s| {
            let id = s
                .shelf
                .add_image(Lang::Ja, "試験の画像", &[1, 2, 3, 255], 1, 1)
                .unwrap();
            s.apply(Action::Shelf(ShelfOp::ToLibrary(id)));
        }),
        ("LibraryAddFiles", |s| {
            s.apply(Action::Shelf(ShelfOp::LibraryAddFiles(vec![fixture(
                "raster.ylsmart",
            )])));
        }),
    ];
    for (name, start) in starts {
        let dir = TempDir::new("libwrite");
        let mut s = library_state(&dir);
        s.library.hold_writes(true);
        start(&mut s);
        assert!(
            s.library.writing_name().is_some(),
            "{name}: 書き込みが始まる: {}",
            s.message
        );
        let jobs = close_jobs(&s);
        assert_eq!(jobs, vec![CloseJob::LibraryWrite], "{name}");
        assert!(
            close_question(Lang::Ja, false, &jobs).contains("ライブラリへの書き込み"),
            "{name}"
        );

        // 取り消しても、書き込みのスレッドが終わるまでは、期限まで待つ（止めたスレッドは、放すまで終われない）
        let started = Instant::now();
        stop_jobs(&mut s, Duration::from_millis(250));
        assert!(
            started.elapsed() >= Duration::from_millis(200),
            "{name}: スレッドが終わるまで待つ: {:?}",
            started.elapsed()
        );
        assert!(close_jobs(&s).is_empty(), "{name}: 取り消した");
        assert!(
            s.library.busy_reason(Lang::Ja).is_some(),
            "{name}: やめた書き込みのスレッドは、まだ終わっていない"
        );

        // スレッドを放せば、終わるまで待って返る（待つ期限より前に）
        s.library.hold_writes(false);
        let started = Instant::now();
        stop_jobs(&mut s, Duration::from_secs(10));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{name}: {:?}",
            started.elapsed()
        );
        assert!(
            s.library.busy_reason(Lang::Ja).is_none(),
            "{name}: スレッドが終わるまで待った"
        );
        assert_eq!(
            files_under(&dir.0.join("library")),
            0,
            "{name}: 取り消した書き込みはライブラリに残らない"
        );
    }
}

/// ライブラリのファイルの取り込み（別のスレッド）は一覧に出て、後始末が取り消して、スレッドが終わるまで待つ。棚には入らない。
#[test]
fn a_library_import_is_listed_and_cancelled_and_waited_for_when_stopping_jobs() {
    let dir = TempDir::new("libimport");
    std::fs::create_dir_all(dir.0.join("library/Smart")).unwrap();
    std::fs::copy(
        fixture("raster.ylsmart"),
        dir.0.join("library/Smart/raster.ylsmart"),
    )
    .unwrap();
    let mut s = library_state(&dir);
    s.shelf.hold_saves(true);
    s.apply(Action::Shelf(ShelfOp::UseFromLibrary(
        "Smart/raster.ylsmart".into(),
    )));
    assert_eq!(s.shelf.pending_save(), Some(true), "{}", s.message);
    let jobs = close_jobs(&s);
    assert_eq!(jobs, vec![CloseJob::ShelfImport]);
    assert!(close_question(Lang::En, false, &jobs).contains("Importing a material"));

    let started = Instant::now();
    stop_jobs(&mut s, Duration::from_millis(250));
    assert!(
        started.elapsed() >= Duration::from_millis(200),
        "スレッドが終わるまで待つ: {:?}",
        started.elapsed()
    );
    assert!(close_jobs(&s).is_empty(), "取り消した");
    assert_eq!(
        s.shelf.saves_running(),
        1,
        "止めたスレッドは、まだ終わっていない"
    );

    s.shelf.hold_saves(false);
    let started = Instant::now();
    stop_jobs(&mut s, Duration::from_secs(10));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(s.shelf.saves_running(), 0, "スレッドが終わるまで待った");
    s.shelf_poll();
    assert!(
        s.shelf.resources().is_empty(),
        "取り消した取り込みは棚に入れない"
    );
    assert!(!s.modified, "文書は変わらない");
}

/// FBX の読み込みは読むだけの仕事（何も書かない）なので、閉じる前の確かめには挙げない（保存していない変更が無ければ、聞かずに閉じる）。
/// 終わるときの取り消し・止まるのを待つ試験は、読み込みのスレッドを止めておける `view3d::pose::loads` の中にある。
#[test]
fn a_model_load_is_not_asked_about_before_closing() {
    use yolu_app::newproject::{NpAction, Prep};
    let mut s = AppState::new_in(64, 64, Lang::Ja);
    s.apply(Action::Project(NpAction::OpenNew));
    let (job, _hold) = yolu_app::view3d::pose::PrepareJob::parked();
    {
        let win = s.np.window.as_mut().unwrap();
        win.prep = Prep::Loading {
            path: PathBuf::from("m.fbx"),
            job,
        };
        assert!(win.is_loading());
    }
    assert!(close_jobs(&s).is_empty(), "{:?}", close_jobs(&s));
    assert!(close_question(Lang::Ja, false, &close_jobs(&s)).is_empty());
    assert!(
        close_question(Lang::En, true, &close_jobs(&s)).starts_with("There are unsaved changes.")
    );
}
