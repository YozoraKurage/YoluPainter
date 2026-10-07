//! 復旧用の世代の書き置きと起動時の復旧（アプリの状態だけ。画面を描かないので Wine でも回る）: 書く頃合い・ストロークの最中に
//! 取らない・失敗しても描ける・落ちた体の起動・開く（名称未設定（復旧）で、元の .ylp には書かない）・捨てる・整理。
//! 復旧の窓の画面は `recovery_ui.rs`。

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use yolu_app::engine::{composite_pixel, DVec2, Document, SelectionMask};
use yolu_app::lang::Lang;
use yolu_app::recovery::{
    DiskBudget, DiskSpace, Problem, RecoveryAction, RecoverySettings, SpaceProbe,
};
use yolu_app::state::{Action, AppState, DialogRequest};
use yolu_io::{Fault, GenerationStore, Project, INFO_NAME};

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-recovery-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn root(&self) -> PathBuf {
        self.0.join("recovery")
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn settings(interval: u32, keep: u32, strokes: u32) -> RecoverySettings {
    RecoverySettings {
        interval_seconds: interval,
        strokes_between: strokes,
        generations_to_keep: keep,
        directory: None,
        ..RecoverySettings::default()
    }
}
/// 空きがたっぷりあるディスク（試験が、置き場のあるディスクの本当の空きに左右されないように）。
fn plenty() -> SpaceProbe {
    Arc::new(|_| {
        Some(DiskSpace {
            total: 1000 * GIB,
            available: 900 * GIB,
        })
    })
}
const GIB: u64 = 1 << 30;
fn session_with(root: &Path, s: RecoverySettings, lang: Lang) -> AppState {
    let mut state = AppState::new_in(64, 64, lang);
    state.recovery.set_space_probe(Some(plenty()));
    state.recovery.enable(root.to_path_buf(), s).unwrap();
    state
}
fn session(root: &Path) -> AppState {
    session_with(root, settings(15, 3, 0), Lang::Ja)
}

/// 1 本のストロークを描いて終える（文書が変わり、保存していない印が付く）。
fn paint(s: &mut AppState, x: f64) {
    let stroke = begin(s, x);
    finish(s, stroke);
}
fn begin(s: &mut AppState, x: f64) -> yolu_app::engine::Stroke {
    let layer = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    let mut stroke = s.doc.begin_stroke(layer, &brush).unwrap();
    stroke
        .add_point(&mut s.doc, x, 20.0, 1.0, DVec2::ZERO)
        .unwrap();
    stroke
}
fn finish(s: &mut AppState, stroke: yolu_app::engine::Stroke) {
    s.doc.end_stroke(stroke).unwrap();
    s.modified = true;
}

/// 変更のあと、間隔が経つまで進めて、書き込みが終わるまで待つ（返すのは進めた先の時刻）。
fn write_after(s: &mut AppState, from: Instant) -> Instant {
    s.recovery_tick_at(from);
    let due = from + Duration::from_secs(s.recovery.settings().interval_seconds as u64 + 1);
    s.recovery_tick_at(due);
    s.recovery_wait();
    due
}

fn generations(s: &AppState) -> usize {
    GenerationStore::new(s.recovery.session_dir().unwrap())
        .list()
        .unwrap()
        .len()
}
fn pixel_in(doc: &Document, x: u32) -> [u8; 4] {
    composite_pixel(doc, x, 20)
}
/// この実行の置き場の最新の世代の、最初のセットの文書。
fn newest_doc(s: &AppState) -> Document {
    let store = GenerationStore::new(s.recovery.session_dir().unwrap());
    let mut files = store.load().unwrap().files;
    files.remove(INFO_NAME);
    let project = Project::from_entries(files).unwrap();
    project.sets()[0].document.to_core().unwrap()
}
fn fault(f: impl Fn(&str) -> io::Result<()> + Send + Sync + 'static) -> Fault {
    Arc::new(f)
}
fn hash(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[test]
fn headless_nothing_is_written_until_the_interval_has_passed_since_the_change() {
    let dir = TempDir::new("interval");
    let mut s = session(&dir.root());
    let t0 = Instant::now();
    // 変更が無い・保存した .ylp と同じなら、いくら待っても書かない
    s.recovery_tick_at(t0 + Duration::from_secs(3600));
    s.recovery_wait();
    assert_eq!((s.recovery.checkpoints(), generations(&s)), (0, 0));
    paint(&mut s, 10.0);
    s.recovery_tick_at(t0);
    s.recovery_tick_at(t0 + Duration::from_secs(14));
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 0, "14 秒ではまだ");
    s.recovery_tick_at(t0 + Duration::from_secs(16));
    s.recovery_wait();
    assert_eq!((s.recovery.checkpoints(), generations(&s)), (1, 1));
    assert!(
        s.recovery.is_marked_dirty(),
        "保存していない作業の世代がある印"
    );
    assert_eq!(pixel_in(&newest_doc(&s), 10), pixel_in(&s.doc, 10));
}

#[test]
fn headless_continuous_painting_is_written_every_interval_from_the_previous_write() {
    let dir = TempDir::new("period");
    let mut s = session(&dir.root());
    let t0 = Instant::now();
    paint(&mut s, 10.0);
    let first = write_after(&mut s, t0);
    // 続けて描く: 次の書き置きは、前の書き置きから間隔が経ってから（変更の見つけた時刻ではなく）
    paint(&mut s, 20.0);
    s.recovery_tick_at(first + Duration::from_secs(1));
    s.recovery_tick_at(first + Duration::from_secs(10));
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 1);
    s.recovery_tick_at(first + Duration::from_secs(16));
    s.recovery_wait();
    assert_eq!((s.recovery.checkpoints(), generations(&s)), (2, 2));
}

#[test]
fn headless_enough_finished_strokes_write_before_the_interval() {
    let dir = TempDir::new("strokes");
    let mut s = session_with(&dir.root(), settings(600, 3, 3), Lang::Ja);
    let t0 = Instant::now();
    for (i, x) in [10.0, 20.0, 30.0].into_iter().enumerate() {
        let stroke = begin(&mut s, x);
        s.recovery_tick_at(t0 + Duration::from_millis(i as u64)); // 描いている最中に見張りが回る
        finish(&mut s, stroke);
        s.recovery_tick_at(t0 + Duration::from_millis(i as u64 + 1));
        s.recovery_wait();
        assert_eq!(s.recovery.checkpoints(), (i == 2) as u64, "{i} 本目");
    }
    assert_eq!(generations(&s), 1);
    // 数を 0 にすれば、数では書かない
    let other = TempDir::new("strokes-off");
    let mut s = session_with(&other.root(), settings(600, 3, 0), Lang::Ja);
    for x in [10.0, 20.0, 30.0, 40.0] {
        let stroke = begin(&mut s, x);
        s.recovery_tick_at(t0);
        finish(&mut s, stroke);
        s.recovery_tick_at(t0 + Duration::from_millis(1));
    }
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 0);
}

#[test]
fn headless_nothing_is_captured_during_a_stroke_and_the_capture_follows_it() {
    let dir = TempDir::new("midstroke");
    let mut s = session(&dir.root());
    let t0 = Instant::now();
    let stroke = begin(&mut s, 10.0);
    s.modified = true;
    // 描いている最中は、時間が経っても取らない（材料を取る前に帰る。書き手も動かない）
    for secs in [0, 20, 40, 3600] {
        s.recovery_tick_at(t0 + Duration::from_secs(secs));
        assert!(s.recovery.is_idle(), "{secs} 秒");
    }
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 0);
    finish(&mut s, stroke);
    s.recovery_tick_at(t0 + Duration::from_secs(3601));
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 1, "描き終えた次の見張りで書く");
    assert_eq!(pixel_in(&newest_doc(&s), 10), pixel_in(&s.doc, 10));
}

#[test]
fn headless_an_unchanged_state_is_not_written_again() {
    let dir = TempDir::new("unchanged");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    let t = write_after(&mut s, Instant::now());
    for i in 1..5 {
        s.recovery_tick_at(t + Duration::from_secs(100 * i));
        s.recovery_wait();
    }
    assert_eq!((s.recovery.checkpoints(), generations(&s)), (1, 1));
    // セットの名前だけの変更（文書の版は変わらない）も、別の書き置きになる
    let uid = s.sets.current().uid;
    s.rename_set(uid, "名前だけ変えた").unwrap();
    s.modified = true;
    let t = write_after(&mut s, t + Duration::from_secs(1000));
    let _ = t;
    assert_eq!(s.recovery.checkpoints(), 2);
}

#[test]
fn headless_a_saved_project_is_not_written_and_saving_clears_the_mark() {
    let dir = TempDir::new("saved");
    let file = dir.0.join("作品.ylp");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    let t = write_after(&mut s, Instant::now());
    assert!(s.recovery.is_marked_dirty());
    s.apply(Action::SaveProjectAs(file.clone()));
    assert!(!s.modified, "{}", s.message);
    s.recovery_tick_at(t + Duration::from_secs(1));
    assert!(
        !s.recovery.is_marked_dirty(),
        "保存したので、落ちても知らせない印へ"
    );
    for i in 1..4 {
        s.recovery_tick_at(t + Duration::from_secs(100 * i));
        s.recovery_wait();
    }
    assert_eq!(
        s.recovery.checkpoints(),
        1,
        "保存した .ylp と同じなので書かない"
    );
    // 保存した後の変更は、また書く
    paint(&mut s, 30.0);
    write_after(&mut s, t + Duration::from_secs(1000));
    assert_eq!(s.recovery.checkpoints(), 2);
    assert!(s.recovery.is_marked_dirty());
}

#[test]
fn headless_only_the_latest_waiting_request_is_written_and_one_writer_runs() {
    let dir = TempDir::new("latest");
    let mut s = session(&dir.root());
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (started_tx, release_rx) = (Mutex::new(started_tx), Mutex::new(release_rx));
    let calls = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let widest = Arc::new(AtomicUsize::new(0));
    let (c, a, w) = (calls.clone(), active.clone(), widest.clone());
    s.recovery.set_fault(Some(fault(move |stage| {
        if stage == "snapshot" {
            w.fetch_max(a.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
            if c.fetch_add(1, Ordering::SeqCst) == 0 {
                started_tx.lock().unwrap().send(()).unwrap();
                release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(20))
                    .map_err(io::Error::other)?;
            }
        }
        if stage == "after-pointer" {
            a.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    })));
    let t0 = Instant::now();
    paint(&mut s, 10.0);
    s.recovery_tick_at(t0);
    s.recovery_tick_at(t0 + Duration::from_secs(16));
    started_rx.recv_timeout(Duration::from_secs(20)).unwrap();
    // 書き込みが止まっている間に、あと 2 回変えて、そのたびに書く頼みを出す（フォーカスを失った頼み。間隔を待たずに、実際に
    // 書き手へ渡る）。主のスレッドは待たない
    let began = Instant::now();
    paint(&mut s, 20.0);
    s.recovery_request_flush();
    s.recovery_tick_at(t0 + Duration::from_secs(17));
    assert!(!s.recovery.is_idle());
    paint(&mut s, 30.0);
    s.recovery_request_flush();
    s.recovery_tick_at(t0 + Duration::from_secs(18));
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "主のスレッドは書き込みを待たない"
    );
    release_tx.send(()).unwrap();
    s.recovery_wait();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "最初と、待っていた 2 つのうち最新の 1 回だけ（待っていた古い頼みは捨てる）"
    );
    assert_eq!(widest.load(Ordering::SeqCst), 1, "同時に書くのは 1 つ");
    assert_eq!(generations(&s), 2);
    let newest = newest_doc(&s);
    for x in [10, 20, 30] {
        assert_eq!(pixel_in(&newest, x), pixel_in(&s.doc, x), "最新の状態");
    }
}

#[test]
fn headless_painting_while_the_write_runs_does_not_change_what_was_captured() {
    let dir = TempDir::new("frozen");
    let mut s = session(&dir.root());
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (started_tx, release_rx) = (Mutex::new(started_tx), Mutex::new(release_rx));
    s.recovery.set_fault(Some(fault(move |stage| {
        if stage == "snapshot" {
            started_tx.lock().unwrap().send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(20))
                .map_err(io::Error::other)?;
        }
        Ok(())
    })));
    paint(&mut s, 10.0);
    let before = pixel_in(&s.doc, 10);
    write_after_no_wait(&mut s, Instant::now());
    started_rx.recv_timeout(Duration::from_secs(20)).unwrap();
    paint(&mut s, 40.0); // 材料を取ったあとの描き込み
    assert_ne!(pixel_in(&s.doc, 40), [0, 0, 0, 0]);
    release_tx.send(()).unwrap();
    s.recovery_wait();
    let saved = newest_doc(&s);
    assert_eq!(pixel_in(&saved, 10), before);
    assert_eq!(
        pixel_in(&saved, 40),
        [0, 0, 0, 0],
        "取ったあとの描き込みは入らない"
    );
    // 次の見張りで、あとの描き込みも書き置きに入る
    s.recovery.set_fault(None);
    s.recovery_tick_at(Instant::now() + Duration::from_secs(200));
    s.recovery_tick_at(Instant::now() + Duration::from_secs(400));
    s.recovery_wait();
    assert_eq!(pixel_in(&newest_doc(&s), 40), pixel_in(&s.doc, 40));
}
fn write_after_no_wait(s: &mut AppState, from: Instant) {
    s.recovery_tick_at(from);
    s.recovery_tick_at(from + Duration::from_secs(16));
}

#[test]
fn headless_a_failed_write_gives_a_short_reason_keeps_painting_and_tries_again() {
    for stage in [
        "snapshot",
        "file:",
        "verified",
        "before-pointer",
        "after-pointer",
    ] {
        let dir = TempDir::new("failure");
        let mut s = session(&dir.root());
        let failed = Arc::new(AtomicUsize::new(0));
        let f = failed.clone();
        s.recovery.set_fault(Some(fault(move |at| {
            if at.starts_with(stage) && f.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(io::Error::other("注入した書き込みの失敗"));
            }
            Ok(())
        })));
        paint(&mut s, 10.0);
        let t = write_after(&mut s, Instant::now());
        assert!(
            s.message.starts_with("復旧用の書き置きに失敗"),
            "{stage}: {}",
            s.message
        );
        assert_eq!(s.recovery.checkpoints(), 0, "{stage}");
        // 描くのは止まらない
        paint(&mut s, 20.0);
        assert!(s.doc.can_undo());
        // 次の頼みでやり直す（確定の後で失敗した after-pointer も、札を合わせて続けられる）
        let t = write_after(&mut s, t + Duration::from_secs(100));
        assert_eq!(s.message, "復旧用の書き置きが戻りました", "{stage}");
        assert_eq!(s.recovery.checkpoints(), 1, "{stage}");
        assert_eq!(
            pixel_in(&newest_doc(&s), 20),
            pixel_in(&s.doc, 20),
            "{stage}"
        );
        paint(&mut s, 30.0);
        write_after(&mut s, t + Duration::from_secs(100));
        assert_eq!(s.recovery.checkpoints(), 2, "{stage}");
    }
}

#[test]
fn headless_the_failure_reason_is_in_the_language_of_the_screen() {
    let dir = TempDir::new("failure-en");
    let mut s = session_with(&dir.root(), settings(15, 3, 0), Lang::En);
    s.recovery.set_fault(Some(fault(|at| {
        if at == "before-pointer" {
            return Err(io::Error::from(io::ErrorKind::StorageFull));
        }
        Ok(())
    })));
    paint(&mut s, 10.0);
    write_after(&mut s, Instant::now());
    assert_eq!(s.message, "Recovery checkpoint failed (Disk full).");
    let dir = TempDir::new("failure-ja");
    let mut s = session(&dir.root());
    s.recovery.set_fault(Some(fault(|at| {
        if at == "before-pointer" {
            return Err(io::Error::from(io::ErrorKind::StorageFull));
        }
        Ok(())
    })));
    paint(&mut s, 10.0);
    write_after(&mut s, Instant::now());
    assert_eq!(
        s.message,
        "復旧用の書き置きに失敗しました（ディスクの空きがありません）。"
    );
}

#[test]
fn headless_content_that_cannot_be_written_yet_is_reported_not_hidden() {
    let dir = TempDir::new("locked");
    let mut s = session(&dir.root());
    // .ylp にまだ書けない中身（手動の ID の色）は、黙って落とさず理由を出し、世代を書かない
    s.doc
        .set_id_colors(
            yolu_core::mesh_maps::IdColorAssignments::new(
                "a".repeat(64),
                [(0, 0x123456)].into_iter().collect(),
            )
            .unwrap(),
        )
        .unwrap();
    s.modified = true;
    write_after(&mut s, Instant::now());
    assert!(s.message.contains("手動の ID の色"), "{}", s.message);
    assert_eq!(s.recovery.checkpoints(), 0);
    assert!(s.recovery.is_idle());
    // 層のロックは .ylp に書けるようになったので、ロックのある文書は世代に書ける
    let dir = TempDir::new("locked-ok");
    let mut s = session(&dir.root());
    let layer = s.selected_layer.unwrap();
    s.doc
        .set_layer_locks(layer, yolu_core::LayerLocks::POSITION)
        .unwrap();
    s.modified = true;
    write_after(&mut s, Instant::now());
    assert_eq!(s.recovery.checkpoints(), 1, "{}", s.message);
}

#[test]
fn headless_the_chosen_number_of_generations_is_kept() {
    let dir = TempDir::new("keep");
    let mut s = session_with(&dir.root(), settings(15, 2, 0), Lang::Ja);
    let mut t = Instant::now();
    for i in 0..5 {
        paint(&mut s, 10.0 + 5.0 * i as f64);
        t = write_after(&mut s, t + Duration::from_secs(100));
    }
    assert_eq!(s.recovery.checkpoints(), 5);
    assert_eq!(generations(&s), 2);
    assert_eq!(pixel_in(&newest_doc(&s), 30), pixel_in(&s.doc, 30));
}

/// 落ちた体: `shutdown` を呼ばずに状態を捨てる（ロックは OS が手放し、印のファイルが残る）。
fn crash(s: AppState) {
    assert!(s.recovery.is_idle(), "書き込みが終わってから落とす");
    drop(s);
}

#[test]
fn headless_a_crash_shows_the_recovery_window_at_the_next_start_and_a_clean_close_does_not() {
    let dir = TempDir::new("crash");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    write_after(&mut s, Instant::now());
    let expected = pixel_in(&s.doc, 10);
    crash(s);
    // 次の起動: 落ちていたので窓が開き、世代の一覧がある（時刻・セットの数・名前）
    let s2 = session(&dir.root());
    let window = s2
        .recovery
        .window
        .as_ref()
        .expect("落ちた体の起動で窓が出る");
    assert_eq!(window.rows.len(), 1);
    let row = &window.rows[0];
    assert!(row.crashed && !row.own && row.problem.is_none());
    assert_eq!(
        (row.documents, row.name.as_str()),
        (1, ""),
        "名前の無いプロジェクトは空（画面が今の言語で出す）"
    );
    assert!(row.time_ms.unwrap() > 0);
    assert_eq!(window.selected, Some(0), "新しい読める世代を選んでおく");
    let _ = expected;
    // 正しく閉じた起動では出ない（世代は残る）
    let mut s2 = s2;
    s2.recovery_shutdown();
    let mut s3 = session(&dir.root());
    assert!(s3.recovery.window.is_none(), "前の実行は落ちていない");
    s3.recovery_apply(RecoveryAction::OpenWindow);
    assert_eq!(
        s3.recovery.window.as_ref().unwrap().rows.len(),
        1,
        "落ちたときの世代は、捨てるまで残る（正しく閉じた実行の世代は設定の数だけ）"
    );
    s3.recovery_shutdown();

    let clean = TempDir::new("clean");
    let mut s = session(&clean.root());
    paint(&mut s, 10.0);
    write_after(&mut s, Instant::now());
    s.recovery_shutdown();
    let mut next = session(&clean.root());
    assert!(next.recovery.window.is_none());
    next.recovery_apply(RecoveryAction::OpenWindow);
    let rows = &next.recovery.window.as_ref().unwrap().rows;
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].crashed, "正しく閉じたプール");
}

#[test]
fn headless_a_clean_close_writes_the_last_changes_first() {
    let dir = TempDir::new("final");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    write_after(&mut s, Instant::now());
    paint(&mut s, 40.0); // 次の書き置きの前に閉じる
    s.recovery_shutdown();
    let mut next = session(&dir.root());
    next.recovery_apply(RecoveryAction::OpenWindow);
    let rows = &next.recovery.window.as_ref().unwrap().rows;
    assert_eq!(rows.len(), 2, "閉じる前の最後の世代も残る");
    next.recovery_apply(RecoveryAction::Select(0));
    next.recovery_apply(RecoveryAction::Open);
    assert_ne!(pixel_in(&next.doc, 40), [0, 0, 0, 0]);
}

#[test]
fn headless_a_crash_with_everything_saved_or_nothing_written_is_not_announced() {
    // 保存した後に落ちた（保存していない作業の世代は無い）
    let dir = TempDir::new("saved-crash");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    let t = write_after(&mut s, Instant::now());
    s.apply(Action::SaveProjectAs(dir.0.join("a.ylp")));
    s.recovery_tick_at(t + Duration::from_secs(1));
    crash(s);
    let mut s2 = session(&dir.root());
    assert!(s2.recovery.window.is_none(), "保存済みなので窓は出さない");
    s2.recovery_apply(RecoveryAction::OpenWindow);
    let rows = &s2.recovery.window.as_ref().unwrap().rows;
    assert_eq!(rows.len(), 1, "でも一覧には残り、手で開ける");
    assert!(rows[0].crashed);
    // 最初の書き置きの前に落ちた
    let early = TempDir::new("early-crash");
    let mut s = session(&early.root());
    paint(&mut s, 10.0);
    s.recovery_tick_at(Instant::now());
    crash(s);
    let s2 = session(&early.root());
    assert!(s2.recovery.window.is_none());
    let pools = std::fs::read_dir(early.root()).unwrap().count();
    assert_eq!(pools, 1, "世代の無いプールは片付けて、この実行のものだけ");
}

#[test]
fn headless_opening_a_generation_is_untitled_recovered_and_never_writes_the_original() {
    let dir = TempDir::new("open");
    let original = dir.0.join("作品.ylp");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    s.apply(Action::SaveProjectAs(original.clone()));
    assert!(!s.modified, "{}", s.message);
    let before = hash(&original);
    paint(&mut s, 30.0); // 保存していない作業
    write_after(&mut s, Instant::now());
    crash(s);

    let mut s2 = session(&dir.root());
    let row = s2.recovery.window.as_ref().unwrap().rows[0].clone();
    assert_eq!(row.name, "作品.ylp", "元の .ylp の名前");
    s2.recovery_apply(RecoveryAction::Open);
    assert_eq!(s2.project_name, "名称未設定（復旧）");
    assert!(s2.modified, "保存していない");
    let project = s2.project.as_ref().unwrap();
    assert!(!project.is_file(), "元の .ylp にはつながない");
    assert!(s2.recovery.window.is_none(), "開いたら窓を閉じる");
    assert!(s2.message.starts_with("復旧しました"), "{}", s2.message);
    assert_ne!(pixel_in(&s2.doc, 10), [0, 0, 0, 0]);
    assert_ne!(
        pixel_in(&s2.doc, 30),
        [0, 0, 0, 0],
        "保存していなかった作業が戻る"
    );
    assert!(!s2.doc.can_undo(), "復旧は正本を返す。途中の履歴は返さない");
    // 保存は元の .ylp に行かず、保存先を聞く
    s2.apply(Action::SaveProject);
    assert_eq!(s2.dialog_request, Some(DialogRequest::SaveAs));
    assert_eq!(hash(&original), before);
    let saved = dir.0.join("復旧した.ylp");
    s2.apply(Action::SaveProjectAs(saved.clone()));
    assert!(s2.message.starts_with("保存しました"), "{}", s2.message);
    assert!(!s2.modified);
    assert_eq!(hash(&original), before, "元の .ylp は変わらない");
    assert!(!dir.0.join("作品.ylp-backups~").exists());
    // 保存した .ylp は、正本と合成の PNG（派生）もそろっている
    let project = Project::read(&std::fs::read(&saved).unwrap()).unwrap();
    let id = project.sets()[0].id.clone();
    assert!(project
        .original_archive()
        .entries()
        .contains_key(&format!("sets/{id}/composite/Color.png")));
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(saved));
    assert_ne!(pixel_in(&again.doc, 30), [0, 0, 0, 0]);
}

#[test]
fn headless_opening_asks_before_replacing_unsaved_work_and_a_stroke_blocks_it() {
    let dir = TempDir::new("open-ask");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    write_after(&mut s, Instant::now());
    crash(s);
    let mut s2 = session(&dir.root());
    paint(&mut s2, 50.0); // 今の作業は保存していない
    let doc_id = s2.doc.id();
    s2.recovery_apply(RecoveryAction::Open);
    // 画面を持つ側が「変更を捨てますか？」と聞いてから開く（まだ開かない）
    assert_eq!(s2.doc.id(), doc_id);
    let request = s2.recovery.take_open_request().expect("確かめる頼み");
    let stroke = begin(&mut s2, 5.0);
    s2.recovery_open(request.clone());
    assert_eq!(s2.doc.id(), doc_id, "描いている間は開かない");
    finish(&mut s2, stroke);
    s2.recovery_open(request);
    assert_ne!(s2.doc.id(), doc_id);
    assert_eq!(s2.project_name, "名称未設定（復旧）");
}

#[test]
fn headless_a_damaged_generation_is_listed_with_a_reason_and_refused_without_changing_the_document()
{
    let dir = TempDir::new("damaged");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    let t = write_after(&mut s, Instant::now());
    paint(&mut s, 20.0);
    write_after(&mut s, t + Duration::from_secs(100));
    let pool = s.recovery.session_dir().unwrap().to_path_buf();
    // 新しい世代の正本の中身を同じ長さで書き換える（ハッシュが合わない）
    let store = GenerationStore::new(&pool);
    let list = store.list().unwrap();
    let newest = &list[0];
    let manifest = std::fs::read_to_string(
        pool.join("generations")
            .join(&newest.id)
            .join("manifest.sha256"),
    )
    .unwrap();
    let native = manifest
        .lines()
        .find(|l| l.ends_with("document.utpaint"))
        .and_then(|l| l.split(' ').next())
        .unwrap()
        .to_owned();
    let content = pool.join("contents").join(format!("{native}.bin"));
    let mut bytes = std::fs::read(&content).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&content, bytes).unwrap();
    crash(s);

    let mut s2 = session(&dir.root());
    let window = s2.recovery.window.as_ref().unwrap();
    assert_eq!(window.rows.len(), 2);
    assert!(
        window.rows.iter().all(|r| r.problem.is_none()),
        "ハッシュは開くときに確かめる"
    );
    // 新しい世代を開こうとすると断られ、文書は変わらない。古い世代は開ける
    let doc_id = s2.doc.id();
    s2.recovery_apply(RecoveryAction::Select(0));
    s2.recovery_apply(RecoveryAction::Open);
    assert_eq!(s2.doc.id(), doc_id);
    assert!(s2.message.starts_with("復旧を開けません"), "{}", s2.message);
    assert!(s2.recovery.window.as_ref().unwrap().error.is_some());
    s2.recovery_apply(RecoveryAction::Select(1));
    s2.recovery_apply(RecoveryAction::Open);
    assert_ne!(s2.doc.id(), doc_id);
    assert_ne!(pixel_in(&s2.doc, 10), [0, 0, 0, 0]);
}

#[test]
fn headless_a_generation_missing_its_content_is_listed_as_unreadable_and_can_be_discarded() {
    let dir = TempDir::new("missing");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    write_after(&mut s, Instant::now());
    let pool = s.recovery.session_dir().unwrap().to_path_buf();
    std::fs::remove_dir_all(pool.join("contents")).unwrap();
    crash(s);
    let mut s2 = session(&dir.root());
    // 落ちた体でも、読める世代が 1 つも無ければ窓は出さない…ことはなく、読めない世代を理由つきで見せる
    let window = s2.recovery.window.as_ref().expect("世代は残っている");
    assert_eq!(window.rows.len(), 1);
    assert!(window.rows[0].problem.is_some());
    assert_eq!(window.selected, None, "読める世代が無ければ何も選ばない");
    s2.recovery_apply(RecoveryAction::Select(0));
    s2.recovery_apply(RecoveryAction::Open);
    assert!(
        s2.recovery.take_open_request().is_none(),
        "読めない世代は開けない"
    );
    s2.recovery_apply(RecoveryAction::Discard);
    s2.recovery_apply(RecoveryAction::ConfirmDiscard);
    assert!(s2.recovery.window.as_ref().unwrap().rows.is_empty());
}

#[test]
fn headless_discarding_asks_first_and_removes_only_that_generation() {
    let dir = TempDir::new("discard");
    let original = dir.0.join("作品.ylp");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    s.apply(Action::SaveProjectAs(original.clone()));
    let before = hash(&original);
    paint(&mut s, 20.0);
    let t = write_after(&mut s, Instant::now());
    paint(&mut s, 30.0);
    write_after(&mut s, t + Duration::from_secs(100));
    crash(s);
    let mut s2 = session(&dir.root());
    assert_eq!(s2.recovery.window.as_ref().unwrap().rows.len(), 2);
    s2.recovery_apply(RecoveryAction::Select(0));
    s2.recovery_apply(RecoveryAction::Discard);
    assert!(
        s2.recovery.window.as_ref().unwrap().confirm.is_some(),
        "確かめる"
    );
    assert_eq!(
        s2.recovery.window.as_ref().unwrap().rows.len(),
        2,
        "確かめるまで消さない"
    );
    s2.recovery_apply(RecoveryAction::CancelDiscard);
    assert!(s2.recovery.window.as_ref().unwrap().confirm.is_none());
    assert_eq!(s2.recovery.window.as_ref().unwrap().rows.len(), 2);
    s2.recovery_apply(RecoveryAction::Discard);
    s2.recovery_apply(RecoveryAction::ConfirmDiscard);
    let window = s2.recovery.window.as_ref().unwrap();
    assert_eq!(window.rows.len(), 1);
    assert_eq!(window.selected, Some(0), "残った世代を選ぶ");
    s2.recovery_apply(RecoveryAction::Discard);
    s2.recovery_apply(RecoveryAction::ConfirmDiscard);
    assert!(s2.recovery.window.as_ref().unwrap().rows.is_empty());
    // 置き場には、この実行のプールだけが残る。保存した .ylp はそのまま
    assert_eq!(std::fs::read_dir(dir.root()).unwrap().count(), 1);
    assert_eq!(hash(&original), before);
    // 次の起動は、もう知らせることが無い
    s2.recovery_shutdown();
    assert!(session(&dir.root()).recovery.window.is_none());
}

#[test]
fn headless_discarding_this_sessions_own_generation_lets_the_writer_carry_on() {
    let dir = TempDir::new("discard-own");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    let t = write_after(&mut s, Instant::now());
    paint(&mut s, 20.0);
    let t = write_after(&mut s, t + Duration::from_secs(100));
    s.recovery_apply(RecoveryAction::OpenWindow);
    let rows = s.recovery.window.as_ref().unwrap().rows.clone();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.own));
    s.recovery_apply(RecoveryAction::Select(0)); // 最新（current）
    s.recovery_apply(RecoveryAction::Discard);
    s.recovery_apply(RecoveryAction::ConfirmDiscard);
    assert_eq!(generations(&s), 1);
    // 捨てた世代の分は書き直す（入っていないものとして）。札が合わなくて断られることはない
    s.recovery_tick_at(t + Duration::from_secs(100));
    s.recovery_tick_at(t + Duration::from_secs(200));
    s.recovery_wait();
    assert!(
        !s.message.starts_with("復旧用の書き置きに失敗"),
        "{}",
        s.message
    );
    assert_eq!(generations(&s), 2);
    assert_eq!(pixel_in(&newest_doc(&s), 20), pixel_in(&s.doc, 20));
}

#[test]
fn headless_other_folders_in_the_recovery_root_are_never_touched() {
    let dir = TempDir::new("foreign");
    let root = dir.root();
    std::fs::create_dir_all(root.join("my-notes")).unwrap();
    std::fs::write(root.join("my-notes/a.txt"), "a").unwrap();
    std::fs::write(root.join("readme.txt"), "r").unwrap();
    // 名前は似ているが、印も世代の置き場も無い
    std::fs::create_dir_all(root.join("20200101T000000000-abcdef")).unwrap();
    std::fs::write(root.join("20200101T000000000-abcdef/mine.txt"), "m").unwrap();
    let mut s = session_with(&root, settings(15, 2, 0), Lang::Ja);
    paint(&mut s, 10.0);
    let t = write_after(&mut s, Instant::now());
    s.recovery_apply(RecoveryAction::OpenWindow);
    assert_eq!(s.recovery.window.as_ref().unwrap().rows.len(), 1);
    for i in 0..4 {
        paint(&mut s, 20.0 + 5.0 * i as f64);
        write_after(&mut s, t + Duration::from_secs(100 * (i + 1)));
    }
    s.recovery_shutdown();
    let mut again = session_with(&root, settings(15, 2, 0), Lang::Ja);
    again.recovery_shutdown();
    assert_eq!(
        std::fs::read_to_string(root.join("my-notes/a.txt")).unwrap(),
        "a"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("readme.txt")).unwrap(),
        "r"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("20200101T000000000-abcdef/mine.txt")).unwrap(),
        "m"
    );
}

#[test]
fn headless_two_running_windows_do_not_touch_each_other_and_a_crash_is_only_found_after_the_process_is_gone(
) {
    let dir = TempDir::new("two");
    let mut a = session(&dir.root());
    paint(&mut a, 10.0);
    write_after(&mut a, Instant::now());
    // a が動いているあいだに始めた b は、a を「落ちた」とも「閉じた」とも見ない
    let mut b = session(&dir.root());
    assert!(b.recovery.window.is_none());
    b.recovery_apply(RecoveryAction::OpenWindow);
    assert!(
        b.recovery.window.as_ref().unwrap().rows.is_empty(),
        "動いている相手の世代は一覧に出さない"
    );
    b.recovery_shutdown();
    assert_eq!(generations(&a), 1, "a の置き場はそのまま");
    paint(&mut a, 20.0);
    write_after(&mut a, Instant::now() + Duration::from_secs(1000));
    assert_eq!(a.recovery.checkpoints(), 2);
    crash(a);
    let c = session(&dir.root());
    assert_eq!(
        c.recovery.window.as_ref().unwrap().rows.len(),
        2,
        "a が落ちたのは、a が終わってから分かる"
    );
}

#[test]
fn headless_closed_sessions_keep_the_chosen_number_of_generations_in_total_but_crashes_are_kept() {
    let dir = TempDir::new("retention");
    let root = dir.root();
    let run = |count: usize, close: bool| {
        let mut s = session_with(&root, settings(15, 3, 0), Lang::Ja);
        let mut t = Instant::now();
        for i in 0..count {
            paint(&mut s, 10.0 + 5.0 * i as f64);
            t = write_after(&mut s, t + Duration::from_secs(100));
        }
        if close {
            s.recovery_shutdown();
        } else {
            crash(s);
        }
    };
    let rows = |root: &Path| {
        let mut s = session_with(root, settings(15, 3, 0), Lang::Ja);
        s.recovery_apply(RecoveryAction::OpenWindow);
        let rows = s.recovery.window.as_ref().unwrap().rows.clone();
        s.recovery_shutdown();
        rows
    };
    run(2, true);
    run(2, true);
    let after_two = rows(&root);
    assert_eq!(
        after_two.len(),
        3,
        "閉じた実行の世代は、合わせて設定の数（3）"
    );
    assert!(after_two.iter().all(|r| !r.crashed));
    // 落ちた実行の世代は、数に入れず、捨てるまで残す
    run(2, false);
    run(2, true);
    run(2, true);
    let rows = rows(&root);
    assert_eq!(
        rows.iter().filter(|r| r.crashed).count(),
        2,
        "落ちた実行の世代は残る"
    );
    assert_eq!(rows.iter().filter(|r| !r.crashed).count(), 3);
    // 新しい順
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(ids, sorted);
}

#[test]
fn headless_the_interval_and_kept_generations_are_chosen_and_saved() {
    let dir = TempDir::new("settings");
    let file = dir.0.join("recovery.conf");
    let mut s = session(&dir.root());
    s.recovery.set_settings_path(Some(file.clone()));
    s.recovery_apply(RecoveryAction::SetInterval(30));
    s.recovery_apply(RecoveryAction::SetKeep(5));
    assert_eq!(s.recovery.settings().interval_seconds, 30);
    assert_eq!(s.recovery.settings().generations_to_keep, 5);
    let (loaded, problems) = RecoverySettings::load(&file).unwrap();
    assert!(problems.is_empty());
    assert_eq!(
        (loaded.interval_seconds, loaded.generations_to_keep),
        (30, 5)
    );
    // 範囲の外は範囲に収める
    s.recovery_apply(RecoveryAction::SetInterval(1));
    s.recovery_apply(RecoveryAction::SetKeep(1));
    assert_eq!(s.recovery.settings().interval_seconds, 5);
    assert_eq!(s.recovery.settings().generations_to_keep, 2);
    // 新しい間隔で書く頃合いが決まる
    s.recovery_apply(RecoveryAction::SetInterval(60));
    paint(&mut s, 10.0);
    let t0 = Instant::now();
    s.recovery_tick_at(t0);
    s.recovery_tick_at(t0 + Duration::from_secs(30));
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 0);
    s.recovery_tick_at(t0 + Duration::from_secs(61));
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 1);
}

#[test]
fn headless_losing_focus_writes_without_waiting_for_the_interval() {
    let dir = TempDir::new("focus");
    let mut s = session_with(&dir.root(), settings(600, 3, 0), Lang::Ja);
    paint(&mut s, 10.0);
    let t0 = Instant::now();
    s.recovery_tick_at(t0);
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 0);
    s.recovery_request_flush();
    s.recovery_tick_at(t0 + Duration::from_secs(1));
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 1);
    // 変わっていなければ、フォーカスを失っても書かない
    s.recovery_request_flush();
    s.recovery_tick_at(t0 + Duration::from_secs(2));
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 1);
}

#[test]
fn headless_a_project_with_read_only_sets_is_recovered_and_can_be_saved_somewhere_else() {
    use yolu_io::{MaterialRef, NativeDocument, SaveTarget, SetSpec, WriterInfo};
    let dir = TempDir::new("readonly");
    let rich = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../yolu-io/tests/fixtures/native-rich-v21.utpaint"),
    )
    .unwrap();
    let rich = NativeDocument::read(&rich).unwrap();
    assert!(
        !rich.core_issues().is_empty(),
        "core で扱えない中身がある正本"
    );
    // 描けるセット 1 つと、読むだけになるセット 1 つの .ylp
    let (blank, _) = yolu_app::state::blank_document(64, 64);
    let editable_id = yolu_app::sets::guid_string(blank.id());
    let spec = |id: String, name: &str, slot: u16, doc: NativeDocument| SetSpec {
        id,
        name: name.into(),
        material: MaterialRef::PendingSlot(slot),
        document: Some(doc.into()),
        composites: vec![],
    };
    let readonly_id = "00000000-0000-4000-8000-0000000000aa".to_owned();
    let project = Project::create(
        WriterInfo {
            app: "試験".into(),
            version: "0".into(),
            unity: "standalone".into(),
        },
        &[
            spec(
                editable_id.clone(),
                "描ける",
                0,
                NativeDocument::from_core(&blank).unwrap(),
            ),
            spec(readonly_id.clone(), "読むだけ", 1, rich.clone()),
        ],
        &editable_id,
    )
    .unwrap();
    let path = dir.0.join("混ざり.ylp");
    SaveTarget::create(&path).unwrap().save(&project).unwrap();

    let mut s = session(&dir.root());
    s.apply(Action::OpenProject(path.clone()));
    assert!(s.sets.get(1).unwrap().read_only.is_some(), "{}", s.message);
    paint(&mut s, 10.0);
    write_after(&mut s, Instant::now());
    assert_eq!(s.recovery.checkpoints(), 1, "{}", s.message);
    crash(s);

    let mut s2 = session(&dir.root());
    s2.recovery_apply(RecoveryAction::Open);
    assert_eq!(s2.sets.len(), 2);
    assert!(
        s2.sets.get(1).unwrap().read_only.is_some(),
        "読むだけのセットは読むだけのまま"
    );
    assert_ne!(pixel_in(&s2.doc, 10), [0, 0, 0, 0]);
    let saved = dir.0.join("別の場所.ylp");
    s2.apply(Action::SaveProjectAs(saved.clone()));
    assert!(s2.message.starts_with("保存しました"), "{}", s2.message);
    let reopened = Project::read(&std::fs::read(&saved).unwrap()).unwrap();
    let kept = reopened
        .sets()
        .iter()
        .find(|x| x.id == readonly_id)
        .unwrap();
    assert_eq!(
        kept.document.to_bytes().unwrap(),
        rich.to_bytes(),
        "読むだけのセットの正本はバイト列のまま残る"
    );
}

/// 計測（時間は環境による。`cargo test -p yolu-app --test headless recovery::measure -- --ignored --nocapture`）: 主のスレッドが払う
/// 写しの時間と、別のスレッドの書き込みの時間。
#[test]
#[ignore]
fn measure_the_main_thread_cost_of_a_checkpoint() {
    for (size, layers) in [(2048u32, 2usize), (4096, 2), (4096, 6)] {
        let dir = TempDir::new("measure");
        let mut s = AppState::new(size, size);
        s.recovery.set_space_probe(Some(plenty()));
        s.recovery.enable(dir.root(), settings(15, 3, 0)).unwrap();
        let (mut doc, _) = yolu_app::state::blank_document(size, size);
        doc.set_source_budget_bytes(2 << 30).unwrap();
        doc.set_stroke_budget_bytes(1 << 30).unwrap();
        let brush = s.stroke_settings(false);
        let mut first = None;
        for l in 0..layers {
            let layer = if l == 0 {
                doc.layers()[0].id()
            } else {
                doc.add_layer(&format!("L{l}")).unwrap()
            };
            first.get_or_insert(layer);
            let mut y = 0.0;
            while y < size as f64 {
                let mut stroke = doc.begin_stroke(layer, &brush).unwrap();
                for x in (0..size).step_by(64) {
                    stroke
                        .add_point(&mut doc, x as f64, y, 1.0, DVec2::ZERO)
                        .unwrap();
                }
                doc.end_stroke(stroke).unwrap();
                y += 180.0;
            }
        }
        // 保存した .ylp を開き直して、基になるファイルがある状態でも測る
        if std::env::var_os("MEASURE_WITH_BASE").is_some() {
            // 描けるセットを MEASURE_SETS 個（既定 1）。2 つ目からは 1 つ目の写し（タイルは共有）
            let count: usize = std::env::var("MEASURE_SETS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let mut parts = Vec::new();
            for i in 1..count {
                parts.push((
                    format!("00000000-0000-4000-8000-0000000000{:02x}", 0xc0 + i),
                    format!("セット {i}"),
                    yolu_app::sets::MaterialRef::PendingSlot(i as u16),
                    None,
                    doc.capture_snapshot().unwrap(),
                ));
            }
            parts.insert(
                0,
                (
                    "00000000-0000-4000-8000-0000000000bb".into(),
                    "セット".into(),
                    yolu_app::sets::MaterialRef::PendingSlot(0),
                    None,
                    doc,
                ),
            );
            let (sets, d) = yolu_app::sets::TextureSets::from_parts(parts, 0);
            s.replace_sets(sets, d);
            let file = dir.0.join("big.ylp");
            s.apply(Action::SaveProjectAs(file.clone()));
            if !s.message.starts_with("保存しました") {
                // .ylp の予算（768 MiB）を超える大きさ。基になるファイルが作れないので、この構成は測れない
                println!(
                    "基の .ylp あり {size}² × {layers} 層 × {count} セット: 測れない（{}）",
                    s.message
                );
                continue;
            }
            s.apply(Action::OpenProject(file));
            let layer = s.selected_layer.unwrap();
            let brush = s.stroke_settings(false);
            let mut stroke = s.doc.begin_stroke(layer, &brush).unwrap();
            stroke
                .add_point(&mut s.doc, 100.0, 100.0, 1.0, DVec2::ZERO)
                .unwrap();
            s.doc.end_stroke(stroke).unwrap();
            s.modified = true;
            let t0 = Instant::now();
            s.recovery_tick_at(t0);
            let began = Instant::now();
            s.recovery_tick_at(t0 + Duration::from_secs(16));
            let main = began.elapsed();
            s.recovery_wait();
            println!(
                "基の .ylp あり {size}² × {layers} 層 × {count} セット: 主のスレッド {:.1} ms、別のスレッド {:.0} ms",
                main.as_secs_f64() * 1000.0,
                s.recovery.last_write_millis()
            );
            continue;
        }
        let (sets, doc) = yolu_app::sets::TextureSets::from_parts(
            vec![(
                "00000000-0000-4000-8000-0000000000bb".into(),
                "セット".into(),
                yolu_app::sets::MaterialRef::PendingSlot(0),
                None,
                doc,
            )],
            0,
        );
        s.replace_sets(sets, doc);
        s.modified = true;
        let t0 = Instant::now();
        s.recovery_tick_at(t0);
        let began = Instant::now();
        s.recovery_tick_at(t0 + Duration::from_secs(16));
        let main = began.elapsed();
        s.recovery_wait();
        println!(
            "{size}² × {layers} 層（{} MiB）: 主のスレッド {:.1} ms、別のスレッド {:.0} ms",
            s.doc.allocated_bytes() >> 20,
            main.as_secs_f64() * 1000.0,
            s.recovery.last_write_millis()
        );
        assert_eq!(s.recovery.checkpoints(), 1, "{}", s.message);
    }
}

#[test]
fn headless_the_asset_shelf_is_written_and_comes_back_after_a_crash() {
    use yolu_app::shelf::ShelfOp;
    let dir = TempDir::new("shelf");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    let t = write_after(&mut s, Instant::now());
    assert_eq!(s.recovery.checkpoints(), 1);
    // 絵を変えずに、棚へ素材を足すだけでも書き置きになる
    let layer = s.selected_layer.unwrap();
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(layer)));
    assert_eq!(s.shelf.resources().len(), 1, "{}", s.message);
    assert!(s.modified && s.shelf.changed);
    let name = s.shelf.resources()[0].name.clone();
    write_after(&mut s, t + Duration::from_secs(100));
    assert_eq!(s.recovery.checkpoints(), 2, "棚の変更も書き置きに入る");
    crash(s);

    let mut s2 = session(&dir.root());
    s2.recovery_apply(RecoveryAction::Open);
    assert_eq!(s2.shelf.resources().len(), 1, "{}", s2.message);
    assert_eq!(s2.shelf.resources()[0].name, name);
    assert!(
        !s2.shelf.changed,
        "復旧した棚は、保存でそのまま書く（変えていない）"
    );
    // 別の場所へ保存した .ylp に、棚の素材が入っている
    let saved = dir.0.join("棚つき.ylp");
    s2.apply(Action::SaveProjectAs(saved.clone()));
    assert!(s2.message.starts_with("保存しました"), "{}", s2.message);
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(saved));
    assert_eq!(again.shelf.resources().len(), 1, "{}", again.message);
}

#[test]
fn headless_a_panic_while_writing_is_a_failure_not_a_stuck_writer() {
    let dir = TempDir::new("panic");
    let mut s = session(&dir.root());
    let panicked = Arc::new(AtomicUsize::new(0));
    let p = panicked.clone();
    s.recovery.set_fault(Some(fault(move |stage| {
        if stage == "verified" && p.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("注入した内部の失敗");
        }
        Ok(())
    })));
    paint(&mut s, 10.0);
    let t = write_after(&mut s, Instant::now()); // 待ちが固まらない
    assert_eq!(
        s.message,
        "復旧用の書き置きに失敗しました（書き込みの内部の失敗）。"
    );
    assert!(s.recovery.is_idle());
    let t = write_after(&mut s, t + Duration::from_secs(100));
    let _ = t;
    assert_eq!(
        s.recovery.checkpoints(),
        1,
        "次の頼みでやり直せる: {}",
        s.message
    );
    assert_eq!(pixel_in(&newest_doc(&s), 10), pixel_in(&s.doc, 10));
}

#[test]
fn headless_the_recovered_name_follows_the_document_until_it_is_saved() {
    let dir = TempDir::new("recovered-name");
    let original = dir.0.join("作品.ylp");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    s.apply(Action::SaveProjectAs(original));
    paint(&mut s, 30.0);
    let t = write_after(&mut s, Instant::now());
    crash(s);
    let mut s2 = session(&dir.root());
    s2.recovery_apply(RecoveryAction::Open);
    // 復旧した文書の書き置きは、元の名前で一覧に出る
    paint(&mut s2, 50.0);
    let t = write_after(&mut s2, t + Duration::from_secs(100));
    s2.recovery_apply(RecoveryAction::OpenWindow);
    let mine = s2
        .recovery
        .window
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .find(|r| r.own)
        .unwrap()
        .name
        .clone();
    assert_eq!(mine, "作品.ylp");
    // 別の名前で保存したら、そのファイルの名前になる
    s2.apply(Action::SaveProjectAs(dir.0.join("新しい.ylp")));
    paint(&mut s2, 20.0);
    write_after(&mut s2, t + Duration::from_secs(100));
    s2.recovery_apply(RecoveryAction::OpenWindow);
    let names: Vec<String> = s2
        .recovery
        .window
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .filter(|r| r.own)
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(names[0], "新しい.ylp", "{names:?}");
}

#[test]
fn headless_an_open_window_gains_new_generations_and_its_confirm_blocks_the_keys() {
    let dir = TempDir::new("live-list");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    let t = write_after(&mut s, Instant::now());
    s.recovery_apply(RecoveryAction::OpenWindow);
    assert_eq!(s.recovery.window.as_ref().unwrap().rows.len(), 1);
    s.recovery_apply(RecoveryAction::Select(0));
    paint(&mut s, 20.0);
    write_after(&mut s, t + Duration::from_secs(100));
    let window = s.recovery.window.as_ref().unwrap();
    assert_eq!(window.rows.len(), 2, "書き置きが増えたら一覧にも出る");
    assert_eq!(
        window.selected_row().map(|r| r.id.clone()),
        Some(window.rows[1].id.clone()),
        "選んでいた世代はそのまま"
    );
    // 捨てる前の確かめが出ている間は、キーを窓の下へ渡さない
    assert!(!yolu_app::windows::modal_open(&s));
    s.recovery_apply(RecoveryAction::Discard);
    assert!(yolu_app::windows::modal_open(&s));
    s.recovery_apply(RecoveryAction::CancelDiscard);
    assert!(!yolu_app::windows::modal_open(&s));
}

#[test]
fn headless_losing_focus_with_nothing_to_write_leaves_no_write_now_request_behind() {
    let dir = TempDir::new("focus-stale");
    let mut s = session_with(&dir.root(), settings(600, 3, 0), Lang::Ja);
    let t0 = Instant::now();
    // 変更なしでフォーカスを失い、戻ってきて最初のストロークを描く: 間隔（600 秒）を待つ。すぐには書かない
    s.recovery_request_flush();
    s.recovery_tick_at(t0);
    paint(&mut s, 10.0);
    s.recovery_tick_at(t0 + Duration::from_secs(1));
    s.recovery_wait();
    assert_eq!(
        s.recovery.checkpoints(),
        0,
        "変更が無いときの頼みを持ち越さない"
    );
    s.recovery_tick_at(t0 + Duration::from_secs(601));
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 1);
    // 書き置きが同じ（書くものが無い）ときにフォーカスを失い、描く: やはり間隔を待つ
    s.recovery_request_flush();
    s.recovery_tick_at(t0 + Duration::from_secs(602));
    paint(&mut s, 20.0);
    s.recovery_tick_at(t0 + Duration::from_secs(603));
    s.recovery_wait();
    assert_eq!(
        s.recovery.checkpoints(),
        1,
        "書き置き済みのときの頼みも持ち越さない"
    );
    s.recovery_tick_at(t0 + Duration::from_secs(603 + 601));
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 2);
    // 描いている最中に失った: その場では取らず、ストロークの終わりで書く（頼みは、書くものがあるあいだは残る）
    let stroke = begin(&mut s, 30.0);
    s.recovery_request_flush();
    s.recovery_tick_at(t0 + Duration::from_secs(1300));
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 2, "描いている最中は取らない");
    finish(&mut s, stroke);
    s.recovery_tick_at(t0 + Duration::from_secs(1301));
    s.recovery_wait();
    assert_eq!(
        s.recovery.checkpoints(),
        3,
        "描き終えた次の見張りで、間隔を待たずに書く"
    );
}

#[test]
fn headless_a_write_over_the_budget_gives_a_short_reason_keeps_the_last_generation_and_painting_goes_on(
) {
    for (lang, expected) in [
        (
            Lang::Ja,
            "復旧用の書き置きに失敗しました（書き置きが作業の予算を超えています）。",
        ),
        (
            Lang::En,
            "Recovery checkpoint failed (Size limit exceeded).",
        ),
    ] {
        let dir = TempDir::new("budget");
        let mut s = session_with(&dir.root(), settings(15, 3, 0), lang);
        paint(&mut s, 10.0);
        let t = write_after(&mut s, Instant::now());
        assert_eq!(s.recovery.checkpoints(), 1, "{}", s.message);
        let pool = s.recovery.session_dir().unwrap().to_path_buf();
        let listing = |pool: &Path| {
            let mut names: Vec<String> = std::fs::read_dir(pool)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        let before = listing(&pool);
        // 合計の上限・1 エントリの上限のどちらでも、断って、前の世代はそのまま
        let mut when = t;
        for (budget, expected) in [
            ((1 << 20, 64), expected),
            (
                (8, 1 << 30),
                lang.pick(
                    "復旧用の書き置きに失敗しました（エントリの予算を超えています）。",
                    "Recovery checkpoint failed (Size limit exceeded).",
                ),
            ),
        ] {
            s.recovery.set_budget(Some(budget));
            paint(&mut s, 50.0);
            when += Duration::from_secs(1000);
            write_after(&mut s, when);
            assert_eq!(s.message, expected, "{budget:?}");
            assert!(
                !s.message
                    .chars()
                    .any(|c| c.is_ascii_digit() && lang == Lang::En),
                "開発用の数を出さない: {}",
                s.message
            );
            assert_eq!(s.recovery.checkpoints(), 1);
            assert_eq!(generations(&s), 1);
            assert_eq!(listing(&pool), before, "作りかけも残さない");
            assert_eq!(
                pixel_in(&newest_doc(&s), 50),
                [0, 0, 0, 0],
                "前の世代のまま"
            );
            // 描くのは止まらない
            paint(&mut s, 58.0);
            assert!(s.doc.can_undo());
        }
        // 予算が戻れば、次の頼みで書ける
        s.recovery.set_budget(None);
        write_after(&mut s, when + Duration::from_secs(1000));
        assert_eq!(s.message, lang.recovery_working_again());
        assert_eq!(s.recovery.checkpoints(), 2);
        assert_eq!(pixel_in(&newest_doc(&s), 50), pixel_in(&s.doc, 50));
        assert_eq!(pixel_in(&newest_doc(&s), 58), pixel_in(&s.doc, 58));
    }
}

/// 閉じた実行を `count` 回（世代を 1 つずつ。置き場に残す数は 20）。
fn seed_closed_generations(root: &Path, count: usize) {
    for i in 0..count {
        let mut s = session_with(root, settings(15, 20, 0), Lang::Ja);
        paint(&mut s, 10.0 + 4.0 * i as f64);
        write_after(&mut s, Instant::now());
        s.recovery_shutdown();
    }
}
fn listed_generations(root: &Path) -> usize {
    let mut s = session_with(root, settings(15, 1000, 0), Lang::Ja);
    s.recovery_apply(RecoveryAction::OpenWindow);
    let count = s.recovery.window.as_ref().unwrap().rows.len();
    s.recovery_shutdown();
    count
}

#[test]
fn headless_unreadable_settings_start_with_the_defaults_without_trimming_generations_or_overwriting_the_file(
) {
    for lang in Lang::ALL {
        let dir = TempDir::new("unreadable");
        let root = dir.root();
        seed_closed_generations(&root, 5);
        // 読めない設定（4096 バイトを超える）
        let conf = dir.0.join("recovery.conf");
        std::fs::write(&conf, vec![b'a'; 5000]).unwrap();
        let bytes_before = std::fs::read(&conf).unwrap();
        let mut s = AppState::new_in(64, 64, lang);
        s.recovery.set_space_probe(Some(plenty()));
        let problems = s.recovery.start_from(Some(conf.clone())).unwrap();
        assert!(
            matches!(problems.first(), Some(Problem::Unreadable(_))),
            "{problems:?}"
        );
        let reason = lang.recovery_settings_problem(&problems[0]);
        assert_eq!(
            reason,
            lang.pick(
                "復旧の設定を読めないので、世代は整理しません（ファイルのデータが不正です）",
                "Generations are not trimmed because the recovery settings cannot be read (Invalid file data)"
            ),
        );
        // 既定の間隔で動く。利用者が選んだ数（20）を知らないので、既定の数（3）に整理して世代を消さない
        assert_eq!(s.recovery.settings(), &RecoverySettings::default());
        assert!(s.recovery.is_enabled());
        s.recovery_apply(RecoveryAction::OpenWindow);
        assert_eq!(
            s.recovery.window.as_ref().unwrap().rows.len(),
            5,
            "起動で整理しない"
        );
        // 窓で間隔を選んでも、読めない設定のファイルは既定で上書きしない（書けなかったことを帯に出す）
        s.recovery_apply(RecoveryAction::SetInterval(30));
        assert_eq!(
            s.recovery.settings().interval_seconds,
            30,
            "この実行のあいだは選んだ間隔で動く"
        );
        assert_eq!(std::fs::read(&conf).unwrap(), bytes_before);
        assert_eq!(
            s.message,
            lang.pick(
                "復旧の設定を保存できません。",
                "Cannot save the recovery settings."
            )
        );
        // この実行の置き場も、数が分からないあいだは整理しない（既定の数を超えて書いても残る）
        let mut t = Instant::now();
        for i in 0..5 {
            paint(&mut s, 10.0 + 5.0 * i as f64);
            t = write_after(&mut s, t + Duration::from_secs(100));
        }
        assert_eq!(generations(&s), 5);
        s.recovery_shutdown();
        assert_eq!(
            listed_generations(&root),
            10,
            "閉じても、閉じた実行の世代を整理しない"
        );
        assert_eq!(std::fs::read(&conf).unwrap(), bytes_before);
    }
}

#[test]
fn headless_a_number_chosen_in_the_window_while_the_settings_are_unreadable_applies_for_this_run() {
    let dir = TempDir::new("unreadable-chosen");
    let root = dir.root();
    seed_closed_generations(&root, 6);
    let conf = dir.0.join("recovery.conf");
    std::fs::write(&conf, [0xff, 0xfe, 0xfd]).unwrap(); // UTF-8 ではない
    let mut s = AppState::new_in(64, 64, Lang::En);
    s.recovery.set_space_probe(Some(plenty()));
    let problems = s.recovery.start_from(Some(conf.clone())).unwrap();
    assert!(matches!(problems[0], Problem::Unreadable(_)));
    s.recovery_apply(RecoveryAction::SetKeep(2));
    s.recovery_shutdown();
    assert_eq!(listed_generations(&root), 2, "利用者が選んだ数で整理する");
    assert_eq!(
        std::fs::read(&conf).unwrap(),
        [0xff, 0xfe, 0xfd],
        "読めない設定のファイルは書き換えない"
    );
}

#[test]
fn headless_readable_settings_choose_the_folder_and_are_saved_beside_it() {
    let dir = TempDir::new("conf-ok");
    let conf = dir.0.join("recovery.conf");
    let elsewhere = dir.0.join("別の置き場");
    std::fs::write(
        &conf,
        format!(
            "interval=20\ngenerations=4\ndirectory={}\n",
            elsewhere.display()
        ),
    )
    .unwrap();
    let mut s = AppState::new_in(64, 64, Lang::Ja);
    s.recovery.set_space_probe(Some(plenty()));
    let problems = s.recovery.start_from(Some(conf.clone())).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        (
            s.recovery.settings().interval_seconds,
            s.recovery.settings().generations_to_keep
        ),
        (20, 4)
    );
    assert!(s.recovery.session_dir().unwrap().starts_with(&elsewhere));
    s.recovery_apply(RecoveryAction::SetKeep(10));
    let (saved, _) = RecoverySettings::load(&conf).unwrap();
    assert_eq!(
        (saved.generations_to_keep, saved.directory),
        (10, Some(elsewhere))
    );
    // 設定のファイルが無ければ、同じフォルダの recovery が置き場（既定）
    let fresh = TempDir::new("conf-none");
    let mut s = AppState::new_in(64, 64, Lang::Ja);
    s.recovery.set_space_probe(Some(plenty()));
    assert!(s
        .recovery
        .start_from(Some(fresh.0.join("recovery.conf")))
        .unwrap()
        .is_empty());
    assert!(s.recovery.session_dir().unwrap().starts_with(fresh.root()));
}

#[test]
fn headless_a_previous_run_marker_that_cannot_be_settled_is_reported_and_its_generations_are_still_offered(
) {
    let dir = TempDir::new("unsettled");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    write_after(&mut s, Instant::now());
    let pool = s.recovery.session_dir().unwrap().to_path_buf();
    crash(s);
    // 落ちた実行の印は `session.lock` のまま。`crashed` を書けない（同じ名前のフォルダがある）ので片付けられない
    assert!(pool.join("session.lock").is_file());
    std::fs::create_dir(pool.join("crashed")).unwrap();
    let mut s2 = AppState::new_in(64, 64, Lang::Ja);
    s2.recovery.set_space_probe(Some(plenty()));
    let problems = s2.recovery.enable(dir.root(), settings(15, 3, 0)).unwrap();
    assert!(
        matches!(problems.first(), Some(Problem::PreviousRun(_))),
        "{problems:?}"
    );
    let text = Lang::Ja.recovery_settings_problem(&problems[0]);
    assert!(
        text.starts_with("前回の復旧の印を片付けられません（"),
        "{text}"
    );
    let en = Lang::En.recovery_settings_problem(&problems[0]);
    assert!(
        en.starts_with("Cannot settle the previous recovery marker (") && en.is_ascii(),
        "{en}"
    );
    // 世代を見せない方へは倒さない: 窓が出て、世代を開ける
    let window = s2
        .recovery
        .window
        .as_ref()
        .expect("落ちた実行の世代があるので窓を出す");
    assert_eq!(window.rows.len(), 1);
    s2.recovery_apply(RecoveryAction::Open);
    assert_ne!(pixel_in(&s2.doc, 10), [0, 0, 0, 0]);
    // 印は残る（次の起動がやり直す）
    assert!(pool.join("session.lock").is_file());
}

/// 描ける 2 つのセット（層の名前・不透明度・マスク・選択範囲つき）の .ylp。返すのは、ファイル・1 つ目と 2 つ目のセットの ID。
fn two_set_file(dir: &TempDir) -> (PathBuf, String, String) {
    use yolu_io::{MaterialRef, NativeDocument, SaveTarget, Selection, SetSpec, WriterInfo};
    let make = |name: &str, opacity: f64, selection: (i64, i64, i64, i64)| {
        let (mut doc, layer) = yolu_app::state::blank_document(64, 64);
        let layer = layer.unwrap();
        doc.set_layer_name(layer, name).unwrap();
        doc.set_layer_opacity(layer, opacity, false).unwrap();
        doc.add_layer_mask(layer).unwrap();
        doc.set_layer_mask_density(layer, 0.3, false).unwrap();
        let mask =
            SelectionMask::rectangle(&doc, selection.0, selection.1, selection.2, selection.3);
        (doc, mask)
    };
    let (doc_a, mask_a) = make("A の層", 0.5, (0, 0, 30, 30));
    let (doc_b, mask_b) = make("B の層", 0.6, (20, 20, 40, 40));
    let (id_a, id_b) = (
        yolu_app::sets::guid_string(doc_a.id()),
        yolu_app::sets::guid_string(doc_b.id()),
    );
    let spec = |id: &str, name: &str, slot: u16, doc: &Document| SetSpec {
        id: id.into(),
        name: name.into(),
        material: MaterialRef::PendingSlot(slot),
        document: Some(NativeDocument::from_core(doc).unwrap().into()),
        composites: vec![],
    };
    let project = Project::create(
        WriterInfo {
            app: "試験".into(),
            version: "0".into(),
            unity: "standalone".into(),
        },
        &[
            spec(&id_a, "セット A", 0, &doc_a),
            spec(&id_b, "セット B", 1, &doc_b),
        ],
        &id_a,
    )
    .unwrap()
    .with_selection(&id_a, Some(&Selection::from_core(&mask_a).unwrap()))
    .unwrap()
    .with_selection(&id_b, Some(&Selection::from_core(&mask_b).unwrap()))
    .unwrap();
    let path = dir.0.join("二つ.ylp");
    SaveTarget::create(&path).unwrap().save(&project).unwrap();
    (path, id_a, id_b)
}

#[test]
fn headless_only_the_changed_set_is_rewritten_and_both_sets_selections_and_layers_come_back() {
    let dir = TempDir::new("two-sets");
    let (path, id_a, id_b) = two_set_file(&dir);
    let base = Project::read(&std::fs::read(&path).unwrap()).unwrap();
    let base_entries = base.original_archive().entries().clone();
    let entry = |id: &str, leaf: &str| format!("sets/{id}/{leaf}");

    let mut s = session(&dir.root());
    s.apply(Action::OpenProject(path.clone()));
    assert!(s.message.starts_with("開きました"), "{}", s.message);
    assert_eq!(s.sets.len(), 2);
    // 1 つ目のセットだけを変える: 絵と、選択範囲
    paint(&mut s, 10.0);
    let wider = SelectionMask::rectangle(&s.doc, 0, 0, 50, 50);
    s.doc.set_selection(Some(wider)).unwrap();
    s.modified = true;
    let t = write_after(&mut s, Instant::now());
    assert_eq!(s.recovery.checkpoints(), 1, "{}", s.message);
    let store = GenerationStore::new(s.recovery.session_dir().unwrap());
    let files = store.load().unwrap().files;
    // 変えていないセットは、開いた時のバイト列のまま（正本も選択範囲も）
    for leaf in ["document.utpaint", "selection.bin"] {
        assert_eq!(
            &files[&entry(&id_b, leaf)].bytes().unwrap()[..],
            &base_entries[&entry(&id_b, leaf)].bytes().unwrap()[..],
            "変えていないセットの {leaf} はバイト列のまま"
        );
        assert_ne!(
            &files[&entry(&id_a, leaf)].bytes().unwrap()[..],
            &base_entries[&entry(&id_a, leaf)].bytes().unwrap()[..],
            "変えたセットの {leaf} は書き直す"
        );
    }
    // 選択範囲だけの変更も、別の書き置きになる（画素は変えていない）
    let narrower = SelectionMask::rectangle(&s.doc, 0, 0, 45, 45);
    s.doc.set_selection(Some(narrower)).unwrap();
    s.modified = true;
    write_after(&mut s, t + Duration::from_secs(100));
    assert_eq!(s.recovery.checkpoints(), 2, "選択範囲だけの変更");
    let written = Project::from_entries({
        let mut f = store.load().unwrap().files;
        f.remove(INFO_NAME);
        f
    })
    .unwrap();
    let selection_of = |p: &Project, id: &str| {
        p.sets()
            .iter()
            .find(|x| x.id == id)
            .unwrap()
            .selection
            .as_ref()
            .unwrap()
            .to_core()
            .unwrap()
    };
    assert_eq!(selection_of(&written, &id_a).amount(47, 47), 0);
    assert_eq!(selection_of(&written, &id_a).amount(40, 40), 255);
    crash(s);

    let mut s2 = session(&dir.root());
    s2.recovery_apply(RecoveryAction::Open);
    assert_eq!(s2.sets.len(), 2, "{}", s2.message);
    assert_eq!(s2.sets.current().id, id_a);
    // 1 つ目: 絵・層の属性・選択範囲
    assert_ne!(pixel_in(s2.set_doc(0), 10), [0, 0, 0, 0]);
    let layer = &s2.set_doc(0).layers()[0];
    assert_eq!((layer.name(), layer.opacity()), ("A の層", 0.5));
    assert_eq!(layer.mask().unwrap().density(), 0.3);
    let selection = s2.set_doc(0).selection().expect("選択範囲が戻る");
    assert_eq!(
        (selection.amount(40, 40), selection.amount(47, 47)),
        (255, 0)
    );
    // 2 つ目: 変えていないので、開いた時のまま
    let layer = &s2.set_doc(1).layers()[0];
    assert_eq!((layer.name(), layer.opacity()), ("B の層", 0.6));
    assert_eq!(layer.mask().unwrap().density(), 0.3);
    let selection = s2
        .set_doc(1)
        .selection()
        .expect("変えていないセットの選択範囲も戻る");
    assert_eq!((selection.amount(30, 30), selection.amount(5, 5)), (255, 0));
    // 保存した .ylp にも選択範囲が入る
    let saved = dir.0.join("復旧した二つ.ylp");
    s2.apply(Action::SaveProjectAs(saved.clone()));
    assert!(s2.message.starts_with("保存しました"), "{}", s2.message);
    let reopened = Project::read(&std::fs::read(&saved).unwrap()).unwrap();
    assert_eq!(selection_of(&reopened, &id_b).amount(30, 30), 255);
}

#[test]
fn headless_a_selection_is_written_and_comes_back_after_a_crash() {
    let dir = TempDir::new("selection");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    let mask = SelectionMask::rectangle(&s.doc, 4, 4, 20, 20);
    s.doc.set_selection(Some(mask)).unwrap();
    s.modified = true;
    write_after(&mut s, Instant::now());
    assert_eq!(s.recovery.checkpoints(), 1, "{}", s.message);
    crash(s);
    let mut s2 = session(&dir.root());
    s2.recovery_apply(RecoveryAction::Open);
    let selection = s2.doc.selection().expect("選択範囲が戻る");
    assert_eq!(
        (selection.amount(10, 10), selection.amount(40, 40)),
        (255, 0)
    );
    assert!(!s2.doc.can_undo(), "復旧は選択範囲の履歴を返さない");
}

#[test]
fn headless_the_recovered_name_follows_the_screen_language_until_it_is_saved() {
    let dir = TempDir::new("recovered-language");
    let mut s = session(&dir.root());
    paint(&mut s, 10.0);
    write_after(&mut s, Instant::now());
    crash(s);
    let mut s2 = session(&dir.root());
    s2.recovery_apply(RecoveryAction::Open);
    assert_eq!(s2.project_name, "名称未設定（復旧）");
    s2.set_language(Lang::En);
    assert_eq!(
        s2.project_name, "Untitled (Recovered)",
        "保存先が無いあいだは、言語に追従する"
    );
    s2.set_language(Lang::Ja);
    assert_eq!(s2.project_name, "名称未設定（復旧）");
    // 利用者が付けた名前（保存したファイルの名前）は、言語を替えても変えない
    s2.apply(Action::SaveProjectAs(dir.0.join("作品.ylp")));
    assert_eq!(s2.project_name, "作品");
    s2.set_language(Lang::En);
    assert_eq!(s2.project_name, "作品");
    // 復旧から開いたのではない新しい文書の既定の名前は、今までどおり
    let mut fresh = AppState::new_in(64, 64, Lang::Ja);
    fresh.set_language(Lang::En);
    assert_eq!(fresh.project_name, "Untitled");
}

// ───────── ディスクの使いすぎを防ぐ（使う量の上限・空きの守り・落ちた実行の最新） ─────────

/// 空きを後から変えられる、偽のディスク（容量 1000 GiB。空けておく量は 10 GiB）。
fn disk_with_free(available: Arc<AtomicU64>) -> SpaceProbe {
    Arc::new(move |_| {
        Some(DiskSpace {
            total: 1000 * GIB,
            available: available.load(Ordering::SeqCst),
        })
    })
}
fn used(root: &Path) -> u64 {
    yolu_app::recovery::usage(root, None).total()
}
/// 落ちた実行を作る: 世代を `count` 個書いて、閉じずに捨てる。
fn crashed_run(root: &Path, count: usize) {
    let mut s = session_with(root, settings(15, 20, 0), Lang::Ja);
    let mut t = Instant::now();
    for i in 0..count {
        paint(&mut s, 8.0 + 4.0 * i as f64);
        t = write_after(&mut s, t + Duration::from_secs(100));
    }
    crash(s);
}
fn rows_of(root: &Path) -> Vec<yolu_app::recovery::Row> {
    let mut s = session_with(root, settings(15, 1000, 0), Lang::Ja);
    s.recovery_apply(RecoveryAction::OpenWindow);
    let rows = s.recovery.window.as_ref().unwrap().rows.clone();
    s.recovery_shutdown();
    rows
}

#[test]
fn headless_a_nearly_full_disk_skips_the_checkpoint_with_a_short_reason_and_it_resumes_when_space_returns(
) {
    for lang in Lang::ALL {
        let dir = TempDir::new("lowdisk");
        let mut s = session_with(&dir.root(), settings(15, 3, 0), lang);
        let free = Arc::new(AtomicU64::new(10 * GIB - 1));
        s.recovery
            .set_space_probe(Some(disk_with_free(free.clone())));
        paint(&mut s, 10.0);
        let t = write_after(&mut s, Instant::now());
        // 空けておく量（10 GiB）を割っている: 書かず、短い理由を帯に出す（数は出さない）。失敗ではなく見送り
        assert_eq!(
            (s.recovery.checkpoints(), generations(&s)),
            (0, 0),
            "{}",
            s.message
        );
        assert_eq!(
            s.message,
            lang.pick(
                "復旧用の書き置きを見送りました（ディスクの空きが少ない）。",
                "Recovery checkpoint skipped (Low disk space)."
            )
        );
        assert!(
            !s.recovery.is_marked_dirty(),
            "書いていないので、保存していない作業の世代の印は立てない"
        );
        let session = s.recovery.session_dir().unwrap().to_path_buf();
        assert_eq!(
            std::fs::read_dir(&session)
                .unwrap()
                .filter(|e| {
                    let name = e
                        .as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .into_owned();
                    name.starts_with(".staging-") || name == "contents"
                })
                .count(),
            0,
            "作りかけも中身も置かない"
        );
        // 描くのは止まらない
        paint(&mut s, 20.0);
        assert!(s.doc.can_undo());
        // 空けておく量は割らないが、新しく書く量を足すと割る: これも書かない（書く量を見ている）
        free.store(10 * GIB + 10, Ordering::SeqCst);
        let t = write_after(&mut s, t + Duration::from_secs(100));
        assert_eq!(
            s.recovery.checkpoints(),
            0,
            "10 バイトの余裕では、世代の中身が入らない"
        );
        // 空きが戻れば、次の頼みで書く。戻ったことを知らせる
        free.store(900 * GIB, Ordering::SeqCst);
        write_after(&mut s, t + Duration::from_secs(100));
        assert_eq!(s.recovery.checkpoints(), 1);
        assert_eq!(s.message, lang.recovery_working_again());
        assert_eq!(pixel_in(&newest_doc(&s), 20), pixel_in(&s.doc, 20));
    }
}

#[test]
fn headless_a_checkpoint_refused_for_low_space_does_nothing_but_look_at_the_free_space() {
    let dir = TempDir::new("lowdisk-cheap");
    let mut s = session_with(&dir.root(), settings(15, 3, 0), Lang::Ja);
    let free = Arc::new(AtomicU64::new(900 * GIB));
    s.recovery
        .set_space_probe(Some(disk_with_free(free.clone())));
    paint(&mut s, 10.0);
    let mut t = write_after(&mut s, Instant::now());
    assert_eq!(s.recovery.checkpoints(), 1);
    // 書き込みの段（組み立ての前の `snapshot` から、確定の前の読み直しの `verified` まで）を数える
    let stages = Arc::new(AtomicUsize::new(0));
    let seen = stages.clone();
    s.recovery.set_fault(Some(fault(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })));
    // 直前の世代の中身が外で変わっていて、読み直せば「外で変わった」になる置き場でも、空きが少ないあいだは、それを読みに行かず
    // 空きの断りを答える（毎回の間隔で来る断るほうの道が、組み立てもハッシュも読み直しもしない）
    let pool = s.recovery.session_dir().unwrap().to_path_buf();
    let newest = GenerationStore::new(&pool).list().unwrap().remove(0);
    let manifest = std::fs::read_to_string(
        pool.join("generations")
            .join(&newest.id)
            .join("manifest.sha256"),
    )
    .unwrap();
    let native = manifest
        .lines()
        .find(|l| l.ends_with("document.utpaint"))
        .and_then(|l| l.split(' ').next())
        .unwrap()
        .to_owned();
    let content = pool.join("contents").join(format!("{native}.bin"));
    let mut bytes = std::fs::read(&content).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&content, bytes).unwrap();
    free.store(10 * GIB - 1, Ordering::SeqCst);
    for round in 0..3 {
        paint(&mut s, 20.0 + 5.0 * round as f64);
        t = write_after(&mut s, t + Duration::from_secs(100));
        assert_eq!(
            s.message, "復旧用の書き置きを見送りました（ディスクの空きが少ない）。",
            "{round}: 外で変わったという失敗ではなく、空きの断り"
        );
        assert_eq!(
            stages.load(Ordering::SeqCst),
            0,
            "{round}: 断る回は書き込みの段に入らない"
        );
    }
    assert_eq!(s.recovery.checkpoints(), 1);
    // 空きが戻れば書き込みの段に入る（ここでは、変えた中身が見つかって失敗する）
    free.store(900 * GIB, Ordering::SeqCst);
    paint(&mut s, 50.0);
    write_after(&mut s, t + Duration::from_secs(100));
    assert!(stages.load(Ordering::SeqCst) > 0);
}

#[test]
fn headless_the_disk_guard_also_stops_a_clean_close_from_writing_and_the_last_generation_stays() {
    let dir = TempDir::new("lowdisk-close");
    let mut s = session_with(&dir.root(), settings(15, 3, 0), Lang::Ja);
    let free = Arc::new(AtomicU64::new(900 * GIB));
    s.recovery
        .set_space_probe(Some(disk_with_free(free.clone())));
    paint(&mut s, 10.0);
    let t = write_after(&mut s, Instant::now());
    assert_eq!(s.recovery.checkpoints(), 1);
    paint(&mut s, 20.0);
    free.store(GIB, Ordering::SeqCst);
    let _ = t;
    // 終わるときの最後の書き置きも、空きが少なければ書かない（前の世代は残り、終了は固まらない）
    s.recovery_shutdown();
    let rows = rows_of(&dir.root());
    assert_eq!(rows.len(), 1, "前の世代は残る");
}

#[test]
fn headless_after_each_checkpoint_the_oldest_generations_beyond_the_disk_limit_go_and_the_newest_stays(
) {
    let dir = TempDir::new("limit-own");
    let root = dir.root();
    // 数の整理（20）には掛からない設定で、上限だけを効かせる
    let mut s = session_with(&root, settings(15, 20, 0), Lang::Ja);
    let mut t = Instant::now();
    paint(&mut s, 10.0);
    t = write_after(&mut s, t + Duration::from_secs(100));
    let one = s.recovery.usage().unwrap().own;
    assert!(one > 0);
    // 世代 2 つぶんの少し上（世代ごとに正本を新しく書くので、3 つ目で超える）
    s.recovery.set_disk_cap(Some(one * 5 / 2));
    for i in 1..6 {
        paint(&mut s, 10.0 + 5.0 * i as f64);
        t = write_after(&mut s, t + Duration::from_secs(100));
        assert!(
            s.recovery.usage().unwrap().total() <= one * 5 / 2 + one / 4,
            "{i}: 上限のあたりに収まる"
        );
    }
    let kept = generations(&s);
    assert!((1..=2).contains(&kept), "上限に収まるぶんだけ残る: {kept}");
    assert!(
        s.recovery.trimmed_generations() >= 3,
        "{}",
        s.recovery.trimmed_generations()
    );
    assert_eq!(
        pixel_in(&newest_doc(&s), 35),
        pixel_in(&s.doc, 35),
        "最新は読めて、いまの絵と同じ"
    );
    // 上限がどれだけ小さくても、この実行の最新の世代は残る（超えたままだと知らせる）
    s.recovery.set_disk_cap(Some(1));
    paint(&mut s, 50.0);
    write_after(&mut s, t + Duration::from_secs(100));
    assert_eq!(generations(&s), 1);
    assert!(s.recovery.last_trimmed().over);
    assert_eq!(pixel_in(&newest_doc(&s), 50), pixel_in(&s.doc, 50));
}

#[test]
fn headless_a_crashed_writes_leftover_is_cleared_before_the_limit_removes_any_generation() {
    let dir = TempDir::new("limit-leftover");
    let mut s = session_with(&dir.root(), settings(15, 20, 0), Lang::Ja);
    paint(&mut s, 10.0);
    let mut t = write_after(&mut s, Instant::now());
    let one = s.recovery.usage().unwrap().own;
    // 落ちた書き込みの残り（manifest の無い作りかけ。大きい）。世代の整理はこれを数えるが、世代を消しても減らない
    let pool = s.recovery.session_dir().unwrap().to_path_buf();
    let stale = pool.join(".staging-20200101T000000000-leftover");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("y.pending"), vec![9u8; 10 * one as usize]).unwrap();
    let base = s.recovery.usage().unwrap().own - 10 * one;
    // 世代 3 つぶんの余裕: 残りを片付ければ、次の世代を足しても収まる
    s.recovery.set_disk_cap(Some(base + 3 * one));
    for i in 1..3 {
        paint(&mut s, 10.0 + 5.0 * i as f64);
        t = write_after(&mut s, t + Duration::from_secs(100));
    }
    assert!(!stale.exists(), "残りは片付いた");
    assert_eq!(generations(&s), 3, "世代は 1 つも消さない");
    assert_eq!(s.recovery.trimmed_generations(), 0);
    assert!(s.recovery.usage().unwrap().total() <= base + 3 * one + one / 2);
    assert_eq!(pixel_in(&newest_doc(&s), 20), pixel_in(&s.doc, 20));
    let _ = t;
}

#[test]
fn headless_choosing_a_smaller_amount_removes_old_generations_everywhere_but_each_crashs_newest_stays(
) {
    let dir = TempDir::new("limit-choose");
    let root = dir.root();
    seed_closed_generations(&root, 3);
    crashed_run(&root, 3);
    crashed_run(&root, 2);
    let before = rows_of(&root);
    assert_eq!(
        before.len(),
        3 + 3 + 2,
        "上限を選ぶ前は、数の整理に掛かるぶんだけ"
    );
    let crashed_newest: Vec<(std::path::PathBuf, String)> = {
        let mut newest: Vec<(std::path::PathBuf, String)> = Vec::new();
        for pool in before
            .iter()
            .filter(|r| r.crashed)
            .map(|r| r.pool.clone())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let id = before
                .iter()
                .filter(|r| r.pool == pool)
                .map(|r| r.id.clone())
                .max()
                .unwrap();
            newest.push((pool, id));
        }
        newest
    };
    assert_eq!(crashed_newest.len(), 2);
    let conf = dir.0.join("recovery.conf");
    let mut s = session_with(&root, settings(15, 20, 0), Lang::Ja);
    s.recovery.set_settings_path(Some(conf.clone()));
    s.recovery_apply(RecoveryAction::OpenWindow);
    // 試験では、本当の量（最低 1 GB）ではなく、小さな上限に置き換える
    s.recovery.set_disk_cap(Some(1));
    s.recovery_apply(RecoveryAction::SetDisk(DiskBudget::Low));
    let after: Vec<(std::path::PathBuf, String)> = s
        .recovery
        .window
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .map(|r| (r.pool.clone(), r.id.clone()))
        .collect();
    assert_eq!(
        after.len(),
        2,
        "閉じた実行の世代と、落ちた実行の古い世代が消え、落ちた実行ごとの最新が残る"
    );
    for newest in &crashed_newest {
        assert!(after.contains(newest), "{newest:?}");
    }
    // 選んだ量は設定のファイルに書かれ、窓の表示が数え直される
    assert_eq!(
        RecoverySettings::load(&conf).unwrap().0.disk,
        DiskBudget::Low
    );
    assert_eq!(s.recovery.window.as_ref().unwrap().cap, 1);
    assert_eq!(
        s.recovery.window.as_ref().unwrap().usage.total(),
        used(&root)
    );
    // 残った世代は、共有の中身ごと開ける
    s.recovery_apply(RecoveryAction::Select(0));
    s.recovery_apply(RecoveryAction::Open);
    assert!(
        s.message.is_empty() || !s.message.contains("開けません"),
        "{}",
        s.message
    );
    assert_eq!(s.project_name, "名称未設定（復旧）");
}

#[test]
fn headless_a_started_run_trims_what_the_limit_no_longer_allows_but_not_while_the_settings_are_unreadable(
) {
    // 設定を読める: 起動で、上限を超えた古い世代を消す
    let dir = TempDir::new("limit-start");
    let root = dir.root();
    seed_closed_generations(&root, 4);
    crashed_run(&root, 3);
    let mut s = AppState::new_in(64, 64, Lang::Ja);
    s.recovery.set_space_probe(Some(plenty()));
    s.recovery.set_disk_cap(Some(1));
    let conf = dir.0.join("recovery.conf");
    assert!(s.recovery.start_from(Some(conf)).unwrap().is_empty());
    s.recovery_apply(RecoveryAction::OpenWindow);
    let rows = &s.recovery.window.as_ref().unwrap().rows;
    assert_eq!(rows.len(), 1, "落ちた実行の最新だけが残る");
    assert!(rows[0].crashed);
    s.recovery_shutdown();
    // 設定を読めない: 利用者が選んだ量が分からないので、上限では消さない。窓で量を選べば、その量で整理する
    let dir = TempDir::new("limit-unreadable");
    let root = dir.root();
    seed_closed_generations(&root, 4);
    let conf = dir.0.join("recovery.conf");
    std::fs::write(&conf, vec![b'a'; 5000]).unwrap();
    let mut s = AppState::new_in(64, 64, Lang::Ja);
    s.recovery.set_space_probe(Some(plenty()));
    s.recovery.set_disk_cap(Some(1));
    let problems = s.recovery.start_from(Some(conf.clone())).unwrap();
    assert!(matches!(problems.first(), Some(Problem::Unreadable(_))));
    s.recovery_apply(RecoveryAction::OpenWindow);
    assert_eq!(
        s.recovery.window.as_ref().unwrap().rows.len(),
        4,
        "起動で消さない"
    );
    let mut t = Instant::now();
    paint(&mut s, 10.0);
    t = write_after(&mut s, t + Duration::from_secs(100));
    paint(&mut s, 20.0);
    write_after(&mut s, t + Duration::from_secs(100));
    assert_eq!(
        s.recovery.trimmed_generations(),
        0,
        "書き置きのあとも消さない"
    );
    assert_eq!(generations(&s), 2);
    s.recovery_apply(RecoveryAction::SetDisk(DiskBudget::Standard));
    assert!(
        s.recovery.window.as_ref().unwrap().rows.len() <= 2,
        "選んだ量で、この実行のあいだ整理する"
    );
    assert_eq!(
        std::fs::read(&conf).unwrap(),
        vec![b'a'; 5000],
        "読めない設定のファイルは書き換えない"
    );
    s.recovery_shutdown();
}

#[test]
fn headless_the_disk_amount_is_chosen_saved_clamped_and_read_back() {
    let dir = TempDir::new("disk-settings");
    let file = dir.0.join("recovery.conf");
    let mut s = session(&dir.root());
    s.recovery.set_settings_path(Some(file.clone()));
    assert_eq!(s.recovery.settings().disk, DiskBudget::Auto);
    for (choice, read) in [
        (DiskBudget::High, DiskBudget::High),
        (DiskBudget::Low, DiskBudget::Low),
        (DiskBudget::Gib(40), DiskBudget::Gib(40)),
        (DiskBudget::Gib(9999), DiskBudget::Gib(256)),
        (DiskBudget::Gib(0), DiskBudget::Gib(1)),
        (DiskBudget::Auto, DiskBudget::Auto),
    ] {
        s.recovery_apply(RecoveryAction::SetDisk(choice));
        assert_eq!(s.recovery.settings().disk, read);
        let (loaded, problems) = RecoverySettings::load(&file).unwrap();
        assert!(problems.is_empty());
        assert_eq!(loaded.disk, read, "書いた量は読み戻せる");
        assert_eq!(
            (loaded.interval_seconds, loaded.generations_to_keep),
            (15, 3),
            "ほかの設定は変わらない"
        );
    }
    // 「詳しく」の開け閉めは設定に書かない（窓の中だけ）
    s.recovery_apply(RecoveryAction::OpenWindow);
    s.recovery_apply(RecoveryAction::DiskDetails(true));
    assert!(s.recovery.window.as_ref().unwrap().details);
    let before = std::fs::read(&file).unwrap();
    s.recovery_apply(RecoveryAction::DiskDetails(false));
    assert!(!s.recovery.window.as_ref().unwrap().details);
    assert_eq!(std::fs::read(&file).unwrap(), before);
    // 自動の上限は、空きに合わせて小さくなる（空き 4 GiB なら、復旧が使える量の 10%）
    let mut s = session(&dir.root());
    s.recovery
        .set_space_probe(Some(disk_with_free(Arc::new(AtomicU64::new(4 * GIB)))));
    let cap = s.recovery.disk_cap().unwrap();
    assert!((GIB / 4..=GIB / 2).contains(&cap), "{cap}");
}

#[test]
fn headless_the_window_counts_what_recovery_uses_by_the_kind_of_session() {
    let dir = TempDir::new("usage");
    let root = dir.root();
    seed_closed_generations(&root, 2);
    crashed_run(&root, 2);
    let mut s = session_with(&root, settings(15, 3, 0), Lang::Ja);
    paint(&mut s, 10.0);
    write_after(&mut s, Instant::now());
    s.recovery_apply(RecoveryAction::OpenWindow);
    let window = s.recovery.window.as_ref().unwrap();
    let usage = window.usage;
    assert!(
        usage.own > 0 && usage.crashed > 0 && usage.closed > 0,
        "{usage:?}"
    );
    assert_eq!(usage.others, 0);
    assert_eq!(
        usage.total(),
        used(&root),
        "窓の数と、置き場のファイルの合計は同じ"
    );
    assert!(window.free.is_some() && window.cap > 0);
    // 書き置きが増えれば、窓の数も増える（開いたまま）
    let before = usage.total();
    paint(&mut s, 30.0);
    write_after(&mut s, Instant::now() + Duration::from_secs(1000));
    assert!(s.recovery.window.as_ref().unwrap().usage.total() > before);
}
