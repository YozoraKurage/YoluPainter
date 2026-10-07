//! FBX を読むスレッドの控えと進み具合。読み込みは別のスレッドで走り、画面のスレッドは結果の受け口（`Loading`・`PrepareJob`）しか持たない。
//! 受け口が捨てられても（窓を閉じた・別のモデルを読み始めた）、スレッドは取消の旗を見て止まる。終わるときは、走っているスレッドが
//! 止まるのを待てるように、読み込みを始めるたびにここへ旗を登録する（結果の受け口とは別に持つ: 受け口を捨てたあとのスレッドも数える）。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// 読み込みの進み具合（0〜1。読み込みのスレッドが書き、画面のスレッドが読む）。
#[derive(Debug, Default)]
pub struct LoadProgress(AtomicU32);

impl LoadProgress {
    /// 進み具合を書く（戻さない）。
    pub(super) fn set(&self, fraction: f32) {
        // 0 は「まだ知らせていない」。知らせた値は 1 を足して持つ
        let stored = 1 + (fraction.clamp(0.0, 1.0) * 1000.0) as u32;
        self.0.fetch_max(stored, Ordering::Relaxed);
    }

    /// 進み具合（まだ知らせが来ていなければ None）。
    pub fn fraction(&self) -> Option<f32> {
        match self.0.load(Ordering::Relaxed) {
            0 => None,
            stored => Some((stored - 1) as f32 / 1000.0),
        }
    }
}

/// 走っている読み込みのスレッド 1 つ。
struct Thread {
    cancel: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}

/// このビューが始めた読み込みのスレッド（終わったものは次の登録のときに外す）。
#[derive(Default)]
pub struct Loads(Vec<Thread>);

impl Loads {
    /// スレッドの旗を登録して、終わったときに立てる札（スレッドが持つ）を返す。
    pub(super) fn register(&mut self, cancel: &Arc<AtomicBool>) -> Finished {
        self.0.retain(|t| !t.finished.load(Ordering::Acquire));
        let finished = Arc::new(AtomicBool::new(false));
        self.0.push(Thread {
            cancel: cancel.clone(),
            finished: finished.clone(),
        });
        Finished(finished)
    }

    /// 走っている読み込みを全部取り消す（止まるのは次の区切り）。
    pub fn cancel_all(&self) {
        for t in &self.0 {
            t.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// まだ終わっていないスレッドの数。
    pub fn running(&self) -> usize {
        self.0
            .iter()
            .filter(|t| !t.finished.load(Ordering::Acquire))
            .count()
    }
}

/// スレッドの終わりに立つ札（panic で抜けても立つ）。
pub(super) struct Finished(Arc<AtomicBool>);

impl Drop for Finished {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// 試験用: 読み込みを、放すか取り消されるまで、最初の区切りで止めておく（大きなファイルの読み込みが長いことの代わり。
/// 読み込みの速さに頼らずに、途中で取り消したことを確かめる）。止めるのはパスで指したものだけなので、同じ試験の中の別のファイルや、
/// 並んで走る別の試験の読み込みは止まらない。
#[cfg(test)]
pub(crate) mod hold {
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    static HELD: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

    /// このパスの読み込みを止めておく（放すまで・取り消しが来るまで）。
    pub(crate) fn hold(path: &Path) {
        HELD.lock()
            .unwrap()
            .get_or_insert_with(HashSet::new)
            .insert(path.to_path_buf());
    }

    pub(crate) fn release(path: &Path) {
        if let Some(held) = HELD.lock().unwrap().as_mut() {
            held.remove(path);
        }
    }

    fn held(path: &Path) -> bool {
        HELD.lock()
            .unwrap()
            .as_ref()
            .is_some_and(|held| held.contains(path))
    }

    /// 読み込みの区切りで呼ぶ。止めてあるパスなら、放される・取り消される（最大 30 秒）まで待つ。
    pub(crate) fn tick(path: &Path, cancel: &AtomicBool) {
        let start = std::time::Instant::now();
        while held(path) && !cancel.load(Ordering::Relaxed) && start.elapsed().as_secs() < 30 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use super::super::tests::TRIANGLE_FBX;
    use super::super::{
        open_fbx, poll, prepare_fbx, wait_for_load, ModelLimits, PoseAction, MODEL_SHARE,
    };
    use super::hold;
    use crate::newproject::{NpAction, Prep};
    use crate::state::{Action, AppState};
    use crate::view3d::model::ViewError;
    use crate::windows::{close_jobs, stop_jobs};

    /// 試験ごとの作業フォルダ（FBX を書く）。
    struct Dir(PathBuf);
    impl Dir {
        fn new(tag: &str) -> Dir {
            let dir =
                std::env::temp_dir().join(format!("yolu-app-loads-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Dir(dir)
        }
        /// 三角形 1 つの FBX（名前は拡張子の前がモデルの名前になる）。
        fn fbx(&self, name: &str) -> PathBuf {
            let path = self.0.join(format!("{name}.fbx"));
            std::fs::write(&path, TRIANGLE_FBX).unwrap();
            path
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 放すか取り消されるまで止めておく読み込み（パスで指す。同じ試験の別のファイルや、並んで走る別の試験は止まらない）。
    /// 試験が途中で落ちても、止めたままにしない。
    struct Held(PathBuf);
    impl Held {
        fn new(path: &Path) -> Held {
            hold::hold(path);
            Held(path.to_path_buf())
        }
        fn release(&self) {
            hold::release(&self.0);
        }
    }
    impl Drop for Held {
        fn drop(&mut self) {
            hold::release(&self.0);
        }
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "待ちきれない: {what}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// 前のモデルが入っている状態（試しの人形）。
    fn app_with_model() -> AppState {
        let mut app = AppState::new(64, 64);
        app.apply(Action::Pose(PoseAction::LoadFigure));
        assert!(app.view3d.pose.session.is_some(), "{}", app.message);
        app
    }

    #[test]
    fn cancelling_the_load_keeps_the_current_model_and_stops_the_thread() {
        let dir = Dir::new("cancel");
        let path = dir.fbx("新");
        let held = Held::new(&path);
        let mut app = app_with_model();
        let (rig, revision) = {
            let s = app.view3d.pose.session.as_ref().unwrap();
            (s.rig.clone(), app.view3d.model.as_ref().unwrap().revision())
        };
        open_fbx(&mut app.view3d, &path);
        assert!(app.view3d.pose.is_loading());
        assert_eq!(app.view3d.pose.loads_running(), 1);
        app.apply(Action::Pose(PoseAction::CancelLoad));
        assert!(!app.view3d.pose.is_loading());
        assert!(
            app.message.contains("取り消しました") && app.message.contains("新.fbx"),
            "{}",
            app.message
        );
        // スレッドは止まり（放さなくても。取り消しで止まる）、何も入らない
        wait_until("読み込みのスレッドが止まる", || {
            app.view3d.pose.loads_running() == 0
        });
        let (message, installed) = poll(&mut app.view3d);
        assert!(!installed && message.is_none(), "{message:?}");
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert!(std::sync::Arc::ptr_eq(&s.rig, &rig), "今のモデルは前のまま");
        assert_eq!(app.view3d.model.as_ref().unwrap().revision(), revision);
        held.release();
    }

    #[test]
    fn a_cancelled_prepare_job_stops_before_the_model_is_built_and_reports_cancelled() {
        let dir = Dir::new("job");
        let path = dir.fbx("甲");
        let held = Held::new(&path);
        let mut app = AppState::new(64, 64);
        let job = prepare_fbx(&mut app.view3d, &path, ModelLimits::default());
        assert!(job.poll().is_none(), "止めてあるので終わらない");
        assert_eq!(app.view3d.pose.loads_running(), 1);
        job.cancel();
        let mut result = None;
        wait_until("取り消しの結果", || {
            result = job.poll();
            result.is_some()
        });
        assert!(matches!(result, Some(Err(ViewError::Cancelled))));
        assert!(
            job.fraction().is_none_or(|f| f < MODEL_SHARE),
            "休みの形の組み立てまで進まない: {:?}",
            job.fraction()
        );
        wait_until("スレッドが止まる", || {
            app.view3d.pose.loads_running() == 0
        });
        assert!(app.view3d.model.is_none() && app.view3d.pose.session.is_none());
        held.release();
    }

    #[test]
    fn dropping_the_result_receiver_cancels_the_thread() {
        let dir = Dir::new("drop");
        let path = dir.fbx("甲");
        let held = Held::new(&path);
        let mut app = AppState::new(64, 64);
        let job = prepare_fbx(&mut app.view3d, &path, ModelLimits::default());
        assert_eq!(app.view3d.pose.loads_running(), 1);
        drop(job);
        wait_until("スレッドが止まる", || {
            app.view3d.pose.loads_running() == 0
        });
        held.release();
    }

    #[test]
    fn starting_another_model_cancels_the_earlier_one_and_only_the_last_is_installed() {
        let dir = Dir::new("replace");
        let first = dir.fbx("甲");
        let second = dir.fbx("乙");
        let held = Held::new(&first);
        let mut app = AppState::new(64, 64);
        open_fbx(&mut app.view3d, &first);
        assert_eq!(app.view3d.pose.loads_running(), 1);
        open_fbx(&mut app.view3d, &second);
        let (_, installed) = wait_for_load(&mut app.view3d);
        assert!(installed);
        assert_eq!(app.view3d.pose.session.as_ref().unwrap().rig.name(), "乙");
        // 前の読み込みは放さなくても止まり、あとから入らない
        wait_until("前のスレッドが止まる", || {
            app.view3d.pose.loads_running() == 0
        });
        held.release();
        let (message, installed) = poll(&mut app.view3d);
        assert!(!installed && message.is_none());
        assert_eq!(app.view3d.pose.session.as_ref().unwrap().rig.name(), "乙");
    }

    #[test]
    fn in_the_window_choosing_another_model_cancels_the_earlier_load() {
        let dir = Dir::new("window");
        let first = dir.fbx("甲");
        let second = dir.fbx("乙");
        let held = Held::new(&first);
        let mut app = AppState::new(64, 64);
        app.np_apply(NpAction::OpenModel(first.clone()));
        assert!(app.np.window.as_ref().unwrap().is_loading());
        assert_eq!(app.view3d.pose.loads_running(), 1);
        app.np_apply(NpAction::ChooseModel(second.clone()));
        wait_until("前のスレッドが止まる", || {
            app.view3d.pose.loads_running() <= 1
        });
        wait_until("後のモデルが読めた", || {
            app.poll_newproject();
            matches!(app.np.window.as_ref().unwrap().prep, Prep::Ready { .. })
        });
        assert_eq!(
            app.np.window.as_ref().unwrap().model_path(),
            Some(second.as_path())
        );
        wait_until("前のスレッドが止まった", || {
            app.view3d.pose.loads_running() == 0
        });
        held.release();
        assert!(
            app.view3d.pose.session.is_none(),
            "窓で決めるまで 3D ビューには入れない"
        );
    }

    #[test]
    fn cancelling_the_preparation_in_the_window_stops_the_thread_and_changes_nothing() {
        let dir = Dir::new("prepare");
        let path = dir.fbx("甲");
        let held = Held::new(&path);
        let mut app = app_with_model();
        let revision = app.view3d.model.as_ref().unwrap().revision();
        app.np_apply(NpAction::OpenModel(path.clone()));
        // 何も触っていないプロジェクトではないので、構成の窓で読む
        assert!(
            app.np.window.as_ref().unwrap().is_loading(),
            "{}",
            app.message
        );
        assert_eq!(app.view3d.pose.loads_running(), 1);
        app.np_apply(NpAction::CancelPrepare);
        assert!(matches!(
            app.np.window.as_ref().unwrap().prep,
            Prep::Canceled { .. }
        ));
        wait_until("スレッドが止まる", || {
            app.view3d.pose.loads_running() == 0
        });
        assert_eq!(app.view3d.model.as_ref().unwrap().revision(), revision);
        assert_eq!(
            app.view3d.pose.session.as_ref().unwrap().rig.name(),
            "試しの人形"
        );
        held.release();
    }

    #[test]
    fn closing_the_window_cancels_the_load() {
        let dir = Dir::new("close-window");
        let path = dir.fbx("甲");
        let held = Held::new(&path);
        let mut app = AppState::new(64, 64);
        app.np_apply(NpAction::OpenModel(path.clone()));
        assert_eq!(app.view3d.pose.loads_running(), 1);
        app.np_apply(NpAction::Close);
        wait_until("スレッドが止まる", || {
            app.view3d.pose.loads_running() == 0
        });
        held.release();
    }

    #[test]
    fn cancelling_the_reopen_stops_the_thread() {
        let dir = Dir::new("reopen");
        let path = dir.fbx("甲");
        let held = Held::new(&path);
        let mut app = AppState::new(64, 64);
        let note = crate::newproject::reopen::start(
            &mut app,
            &dir.0.join("p.ylp"),
            "甲.fbx",
            "",
            crate::notice::Kind::Info,
        );
        assert!(note.is_none());
        // ファイルの確かめのあと、読み始める
        wait_until("読み始める", || {
            app.poll_newproject();
            app.view3d.pose.loads_running() == 1
        });
        assert!(app.np.reopening.is_some());
        // 札の進み具合: 読み込みのスレッドが知らせるまでは分からない
        wait_until("進み具合が来る", || {
            app.np.reopening.as_ref().unwrap().fraction().is_some()
        });
        app.np_apply(NpAction::CancelReopen);
        assert!(app.np.reopening.is_none());
        wait_until("スレッドが止まる", || {
            app.view3d.pose.loads_running() == 0
        });
        app.poll_newproject();
        assert!(
            app.view3d.pose.session.is_none(),
            "取り消した読み込みは入らない"
        );
        held.release();
    }

    #[test]
    fn stopping_jobs_cancels_every_model_load_and_waits_for_the_threads_without_asking_first() {
        let dir = Dir::new("stop");
        let in_window = dir.fbx("甲");
        let in_panel = dir.fbx("乙");
        let held = [Held::new(&in_window), Held::new(&in_panel)];
        let mut app = AppState::new(64, 64);
        app.np_apply(NpAction::OpenModel(in_window.clone()));
        open_fbx(&mut app.view3d, &in_panel);
        assert_eq!(app.view3d.pose.loads_running(), 2);
        // 読むだけの仕事なので、閉じる前の確かめには挙げない
        assert!(close_jobs(&app).is_empty());
        let started = Instant::now();
        stop_jobs(&mut app, Duration::from_secs(10));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "放さなくても取り消しで止まり、待ちが期限まで続かない: {:?}",
            started.elapsed()
        );
        assert_eq!(app.view3d.pose.loads_running(), 0, "止まるまで待った");
        assert!(!app.view3d.pose.is_loading());
        assert!(app.view3d.pose.session.is_none());
        for h in &held {
            h.release();
        }
    }

    #[test]
    fn a_thread_whose_receiver_was_already_dropped_is_still_waited_for() {
        let dir = Dir::new("stop-dropped");
        let path = dir.fbx("甲");
        let held = Held::new(&path);
        let mut app = AppState::new(64, 64);
        app.np_apply(NpAction::OpenModel(path.clone()));
        // 受け口を捨てる（窓の状態を空にする）。スレッドは登録で数えている
        app.np.window.as_mut().unwrap().prep = Prep::Idle;
        let started = Instant::now();
        stop_jobs(&mut app, Duration::from_secs(10));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(app.view3d.pose.loads_running(), 0);
        held.release();
    }

    #[test]
    fn progress_rises_to_the_end_and_a_finished_load_installs_normally() {
        let dir = Dir::new("progress");
        let path = dir.fbx("甲");
        let mut app = AppState::new(64, 64);
        let job = prepare_fbx(&mut app.view3d, &path, ModelLimits::default());
        let mut result = None;
        wait_until("読み終わる", || {
            result = job.poll();
            result.is_some()
        });
        assert!(matches!(result, Some(Ok(_))));
        assert_eq!(job.fraction(), Some(1.0));
        wait_until("スレッドが止まる", || {
            app.view3d.pose.loads_running() == 0
        });
    }
}
