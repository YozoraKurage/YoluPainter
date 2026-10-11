//! 別のスレッドの仕事の共通の部品（結果の受け口と取り消しの旗 `Worker`）と、仕事の表 `JOBS`（動いている間の描き直し・進み具合の札・
//! 閉じる前の確かめ・終わる前に止める・確認のウィンドウ）。仕事を 1 つ足すときは、機能のモジュールに `JobSpec` を置き、`JOBS` に並べる。
//! 毎フレームの結果の受け取り（`poll_*`）は表に入れない（`app.rs` の並びの間に予算の同期などが挟まり、順に意味があるため）。
//!
//! `Worker` にしない仕事:
//! - `update/mod.rs` の `Job`: 取り消しの旗は通信の口（`update::http::Link`）が持ち、差し替えられる通信のレイヤーと共有する。
//! - `project/save.rs` の `Job`: 保存は途中で止めない（取り消しの旗が無い）。進み具合を画面と共有の `Shared` で持つ。
//! - `library/service.rs` の `Job<R>`: 待ち行列の 1 つの要素（スレッドを仕事ごとに作らず、取り消しの旗も列で 1 つ）。

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::lang::Lang;
use crate::state::{Action, AppState};
use crate::windows::CloseJob;

/// 仕事の中から見る取り消しの旗。
#[derive(Clone, Debug)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// 取り消しを頼まれたか。
    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// 旗そのもの（取り消しを見る core・io の関数へ渡す）。
    pub fn flag(&self) -> &AtomicBool {
        &self.0
    }
}

/// 受け口を 1 回見た結果。
#[derive(Debug)]
pub enum Polled<M> {
    /// まだ何も届いていない。
    Empty,
    Message(M),
    /// 仕事が知らせずに止まった（送り口が捨てられた。スレッドの異常など）。
    Lost,
}

/// 別のスレッドで走る仕事の、画面の側の口（結果の受け口と取り消しの旗）。
pub struct Worker<M> {
    rx: Receiver<M>,
    cancel: Arc<AtomicBool>,
    cancel_on_drop: bool,
}

impl<M: Send + 'static> Worker<M> {
    /// 名前を付けたスレッドで `work` を始める。スレッドを作れなければ誤りを返す（仕事は始まらない）。
    pub fn spawn(
        name: &str,
        work: impl FnOnce(Sender<M>, Cancel) + Send + 'static,
    ) -> io::Result<Worker<M>> {
        Self::spawn_on(
            std::thread::Builder::new().name(name.into()),
            Arc::default(),
            work,
        )
    }

    /// スレッドの作り方（名前・スタックの大きさ）と、取り消しの旗を渡して始める（旗をほかの所にも登録しておく仕事のため）。
    pub fn spawn_on(
        builder: std::thread::Builder,
        cancel: Arc<AtomicBool>,
        work: impl FnOnce(Sender<M>, Cancel) + Send + 'static,
    ) -> io::Result<Worker<M>> {
        let (tx, rx) = channel();
        let flag = Cancel(cancel.clone());
        builder.spawn(move || work(tx, flag))?;
        Ok(Worker {
            rx,
            cancel,
            cancel_on_drop: false,
        })
    }

    /// スレッドの無い受け口と、その送り口（試験用: 終わらない仕事を置く。送り口を持っているあいだは走っているまま）。
    #[doc(hidden)]
    pub fn parked() -> (Worker<M>, Sender<M>) {
        let (tx, rx) = channel();
        (
            Worker {
                rx,
                cancel: Arc::default(),
                cancel_on_drop: false,
            },
            tx,
        )
    }
}

impl<M> Worker<M> {
    /// 受け口を捨てたら、取り消しの旗も立てる（結果の行き先が無くなる仕事。走り続けても受ける者がいない）。
    pub fn cancel_on_drop(mut self) -> Self {
        self.cancel_on_drop = true;
        self
    }

    /// 取り消しを頼む（止まるのは仕事が次に旗を見たとき）。
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_canceled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// 届いた物を 1 つ受ける（待たない）。
    pub fn poll(&self) -> Polled<M> {
        match self.rx.try_recv() {
            Ok(m) => Polled::Message(m),
            Err(TryRecvError::Empty) => Polled::Empty,
            Err(TryRecvError::Disconnected) => Polled::Lost,
        }
    }

    /// 届くまで、`timeout` まで待つ（試験の待ち）。待ちきれなければ `Empty`。
    #[doc(hidden)]
    pub fn wait(&self, timeout: std::time::Duration) -> Polled<M> {
        match self.rx.recv_timeout(timeout) {
            Ok(m) => Polled::Message(m),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Polled::Empty,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Polled::Lost,
        }
    }
}

impl<M> Drop for Worker<M> {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.cancel.store(true, Ordering::Relaxed);
        }
    }
}

// ───────── 仕事の表 ─────────

/// 進み具合の札の 1 行（右下に出す。文と進み具合と取消）。
pub struct JobCard {
    pub text: String,
    /// 割合（分からない仕事は None。往復する帯）。
    pub fraction: Option<f32>,
    /// 取り消す操作（保存は取り消せない）。
    pub cancel: Option<Action>,
    pub canceling: bool,
}

/// 仕事の表の 1 行。欄が None の物は、その並びに入らない。
pub struct JobSpec {
    /// 札の id（取消のボタンの id にも使う）。重ならない。
    pub id: &'static str,
    /// 動いている（`repaint` なら描き直し続ける。`cancel` があれば、終わる前に止まるまで待つ）。
    pub busy: fn(&AppState) -> bool,
    /// 動いている間は描き直し続ける（進み具合・終わりを受ける）。
    pub repaint: bool,
    /// 進み具合の札。
    pub card: Option<fn(&AppState, Lang) -> Option<JobCard>>,
    /// 閉じる前の確かめに挙げる（閉じると取り消される、利用者が結果を待っている仕事）。
    pub close: Option<fn(&AppState) -> Option<CloseJob>>,
    /// 終わる前に止める。
    pub cancel: Option<fn(&mut AppState)>,
    /// 止めて待つ間に結果を受ける。
    pub poll_while_stopping: Option<fn(&mut AppState)>,
    /// 確かめ・結果のウィンドウが開いている（キーの割り当てを止める）。
    pub modal: Option<fn(&AppState) -> bool>,
}

impl JobSpec {
    /// 欄を持たない行（`busy` だけ）。
    pub const fn new(id: &'static str, busy: fn(&AppState) -> bool) -> JobSpec {
        JobSpec {
            id,
            busy,
            repaint: false,
            card: None,
            close: None,
            cancel: None,
            poll_while_stopping: None,
            modal: None,
        }
    }
}

/// 動いていることの無い行（確認のウィンドウだけの行）の `busy`。
pub fn never(_: &AppState) -> bool {
    false
}

/// 仕事の表（札・閉じる前の確かめ・止める順はこの順）。
pub const JOBS: &[JobSpec] = &[
    crate::bake::JOB,
    crate::bake::CHECK_JOB,
    crate::export::JOB,
    crate::psd::JOB,
    crate::distribute::JOB,
    crate::brushes::import::JOB,
    crate::project::save::JOB,
    crate::newproject::JOB,
    crate::update::JOB,
    crate::brushes::clipstudio::JOB,
    crate::recovery::JOB,
    crate::layerops::JOB,
    crate::library::JOB,
    crate::shelf::JOB,
    crate::view3d::pose::JOB,
    crate::uv_wireframe::overlap::JOB,
    crate::textlayer::JOB,
];

/// 動いている間は描き直し続ける仕事が動いている。
pub fn repaint_needed(app: &AppState) -> bool {
    JOBS.iter().any(|j| j.repaint && (j.busy)(app))
}

/// 確認のウィンドウや結果のウィンドウが開いている。
pub fn modal_open(app: &AppState) -> bool {
    JOBS.iter().any(|j| j.modal.is_some_and(|m| m(app)))
}

/// 進み具合の札（表の順）と、その id。
pub fn cards(app: &AppState, lang: Lang) -> Vec<(&'static str, JobCard)> {
    JOBS.iter()
        .filter_map(|j| Some((j.id, j.card?(app, lang)?)))
        .collect()
}

/// 閉じると取り消される、走っている仕事（表の順）。
pub fn close_jobs(app: &AppState) -> Vec<CloseJob> {
    JOBS.iter().filter_map(|j| j.close?(app)).collect()
}

/// 止める仕事を取り消し（表の順）、止まるのを `wait` まで待つ（待つ間も結果を受ける）。
pub fn stop(app: &mut AppState, wait: Duration) {
    request_stop(app);
    wait_stopped(app, wait);
}

/// 止める仕事の取り消しを頼むだけ（待たない）。先に頼んでおけば、利用者の答えを待つウィンドウを出している間も仕事が止まっていく。
/// 止まるのを待つのは [`wait_stopped`]。
pub fn request_stop(app: &mut AppState) {
    for job in JOBS {
        if let Some(cancel) = job.cancel {
            cancel(app);
        }
    }
}

/// 止める仕事が止まるのを `wait` まで待つ（待つ間も結果を受ける）。取り消しは [`request_stop`] が頼む。
pub fn wait_stopped(app: &mut AppState, wait: Duration) {
    let start = Instant::now();
    while JOBS.iter().any(|j| j.cancel.is_some() && (j.busy)(app)) && start.elapsed() < wait {
        for job in JOBS {
            if let Some(poll) = job.poll_while_stopping {
                poll(app);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// 試験の待ち: `busy` が下りるまで `poll` で受け続ける（上限は 120 秒。越えたら `what` を添えて panic）。
#[doc(hidden)]
pub fn wait_until_idle(
    app: &mut AppState,
    what: &str,
    busy: impl Fn(&AppState) -> bool,
    mut poll: impl FnMut(&mut AppState),
) {
    let start = Instant::now();
    while busy(app) {
        poll(app);
        if !busy(app) {
            break;
        }
        assert!(start.elapsed().as_secs() < 120, "{what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worker_delivers_its_messages_then_reports_lost_when_the_thread_ends() {
        let w = Worker::spawn("yolu-test-worker", |tx, _| {
            tx.send(1).unwrap();
            tx.send(2).unwrap();
        })
        .unwrap();
        assert!(matches!(
            w.wait(Duration::from_secs(10)),
            Polled::Message(1)
        ));
        assert!(matches!(
            w.wait(Duration::from_secs(10)),
            Polled::Message(2)
        ));
        assert!(matches!(w.wait(Duration::from_secs(10)), Polled::Lost));
        assert!(matches!(w.poll(), Polled::Lost));
    }

    #[test]
    fn cancel_reaches_the_work_and_drop_raises_the_flag_only_when_chosen() {
        let (seen_tx, seen_rx) = channel();
        let w: Worker<()> = Worker::spawn("yolu-test-cancel", move |_tx, cancel| {
            while !cancel.is_set() {
                std::thread::sleep(Duration::from_millis(1));
            }
            seen_tx.send(cancel.flag().load(Ordering::Relaxed)).unwrap();
        })
        .unwrap();
        assert!(!w.is_canceled());
        assert!(matches!(w.poll(), Polled::Empty));
        w.cancel();
        assert!(w.is_canceled());
        assert!(seen_rx.recv_timeout(Duration::from_secs(10)).unwrap());

        // 捨てても、選ばなければ旗は立たない
        let flag = Arc::new(AtomicBool::new(false));
        let plain: Worker<()> =
            Worker::spawn_on(std::thread::Builder::new(), flag.clone(), |_tx, _cancel| {}).unwrap();
        drop(plain);
        assert!(!flag.load(Ordering::Relaxed));
        let chosen: Worker<()> =
            Worker::spawn_on(std::thread::Builder::new(), flag.clone(), |_tx, _cancel| {})
                .unwrap()
                .cancel_on_drop();
        drop(chosen);
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn a_parked_worker_stays_empty_until_its_sender_sends_or_goes() {
        let (w, tx) = Worker::<u8>::parked();
        assert!(matches!(w.poll(), Polled::Empty));
        tx.send(7).unwrap();
        assert!(matches!(w.poll(), Polled::Message(7)));
        drop(tx);
        assert!(matches!(w.poll(), Polled::Lost));
    }

    // ───────── 仕事の表 ─────────

    #[test]
    fn job_ids_are_unique_and_every_job_asked_about_before_closing_can_be_stopped() {
        let mut ids: Vec<&str> = JOBS.iter().map(|j| j.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "札の id が重なる: {ids:?}");
        for job in JOBS {
            assert!(
                job.close.is_none() || job.cancel.is_some(),
                "{}: 閉じる前に挙げる仕事は、止める欄も持つ",
                job.id
            );
        }
    }

    /// 表に寄せる前の、描き直しの条件（`windows::show`）。
    fn before_repaint(app: &AppState) -> bool {
        app.bake.is_baking()
            || app.bake.is_checking()
            || app.bake.is_probing_gpu()
            || app.export.is_exporting()
            || app.psd.is_busy()
            || app.distribute.is_busy()
            || app.np_is_busy()
            || app.update.is_busy()
            || app.brushes.import.is_busy()
            || app.is_saving()
            || app.brushes.csp.is_busy()
    }

    /// 表に寄せる前の `windows::modal_open`。
    fn before_modal_open(app: &AppState) -> bool {
        app.export.confirm.is_some()
            || app.psd.confirm.is_some()
            || app.psd.options_open
            || app.psd.notes_confirm.is_some()
            || app.psd.import_check.is_some()
            || app.distribute.window_visible()
            || app.distribute.replace.is_some()
            || app.update.window_open()
            || app
                .recovery
                .window
                .as_ref()
                .is_some_and(|w| w.confirm.is_some())
            || app.np.window.is_some()
            || app.np.remove_confirm.is_some()
            || app.layer_ops.merge_confirm.is_some()
            || app.brushes.csp.open
            || (app.quit && app.is_saving())
    }

    /// 表に寄せる前の `windows::close_jobs`。
    fn before_close_jobs(app: &AppState) -> Vec<CloseJob> {
        let mut jobs = Vec::new();
        if app.bake.is_baking() {
            jobs.push(CloseJob::Bake);
        }
        if app.export.is_exporting() {
            jobs.push(CloseJob::Export);
        }
        if let Some(progress) = app.psd.progress() {
            jobs.push(if progress.importing {
                CloseJob::PsdImport
            } else {
                CloseJob::PsdExport
            });
        }
        if app.distribute.is_busy() {
            jobs.push(CloseJob::Distribute);
        }
        if app.update.progress().is_some() {
            jobs.push(CloseJob::UpdateDownload);
        }
        if app.brushes.import.is_busy() {
            jobs.push(CloseJob::BrushImport);
        }
        if app.library.write.is_some() {
            jobs.push(CloseJob::LibraryWrite);
        }
        match app.shelf.pending_save() {
            Some(true) => jobs.push(CloseJob::ShelfImport),
            Some(false) => jobs.push(CloseJob::ShelfSave),
            None => {}
        }
        jobs
    }

    /// 表に寄せる前の札の id（`windows::job_card` の並び。文は表の札の試験が比べる）。
    fn before_card_ids(app: &AppState) -> Vec<&'static str> {
        let mut ids = Vec::new();
        if app.bake.window.is_none() && app.bake.progress().is_some() {
            ids.push("bake");
        }
        if app.export.progress().is_some() {
            ids.push("export");
        }
        if app.psd.progress().is_some() {
            ids.push("psd");
        }
        if app.distribute.progress().is_some() {
            ids.push("distribute");
        }
        if app.brushes.import.progress().is_some() {
            ids.push("brush-import");
        }
        if app.save_progress().is_some() {
            ids.push("save");
        }
        if app.np.reopening.is_some() {
            ids.push("model");
        }
        if app.update.progress().is_some() {
            ids.push("update");
        }
        ids
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-jobs-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// 仕事を 1 つずつ動いている（ウィンドウを 1 つずつ開いた）形にして、表から作った並びと、寄せる前の並びを比べる。
    ///
    /// 外した物と理由:
    /// - ウィンドウの状態表示の確かめ（`bake.check`）: ベイクのウィンドウを描いたときに始まり、止めておくツールが無い（すぐ終わる）。
    /// - 更新のダウンロード（`update`）: 署名した更新の情報と通信のレイヤーの差し替えが要る（`tests/gui_shell/update.rs` が札・取り消し・終わる前の
    ///   止め方を確かめている）。
    /// - 開いたあとのモデルの読み直し（`model` の札）と FBX の読み込み（`fbx`）: FBX のファイルと、読み込みを止めておくツール
    ///   （`view3d::pose::loads` の試験の中）が要る。止める・待つは `view3d::pose::loads` の試験が確かめている。
    #[test]
    fn the_table_gives_the_same_lists_as_before_for_each_running_job() {
        use crate::bake::BakeAction;
        use crate::brushes::BrushAction;
        use crate::distribute::DistributeAction;
        use crate::export::ExportAction;
        use crate::newproject::{NpAction, Prep};
        use crate::psd::PsdAction;
        use crate::shelf::ShelfOp;
        use yolu_core::mesh_maps::MeshMapKind;

        type Make = fn(&std::path::Path) -> (AppState, Option<crate::project::SaveHold>);
        let cases: Vec<(&str, Make)> = vec![
            ("idle", |_| (AppState::new(32, 32), None)),
            ("bake", |_| {
                let mut s = AppState::new(64, 64);
                s.bake.backend = crate::bake::BakeBackend::Cpu;
                s.apply(Action::LoadDemoModel);
                s.bake.settings.maps = vec![MeshMapKind::WorldNormal];
                s.bake.park_next = true;
                s.apply(Action::Bake(BakeAction::Start));
                assert!(s.bake.is_baking(), "{}", s.message);
                (s, None)
            }),
            ("export", |dir| {
                let mut s = AppState::new(32, 32);
                s.export.park_next = true;
                s.apply(Action::Export(ExportAction::ChannelsTo {
                    dir: dir.to_path_buf(),
                    sets: None,
                }));
                assert!(s.export.is_exporting(), "{}", s.message);
                (s, None)
            }),
            ("export.confirm", |dir| {
                let mut s = AppState::new(32, 32);
                s.export.confirm = Some(crate::export::Confirm {
                    what: crate::export::What::Channels { sets: None },
                    dir: dir.to_path_buf(),
                    existing: vec!["a.png".into()],
                    total: 1,
                });
                (s, None)
            }),
            ("psd", |dir| {
                let mut s = AppState::new(32, 32);
                s.psd.park_next = true;
                s.apply(Action::Psd(PsdAction::Export(dir.join("out.psd"))));
                assert!(s.psd.is_busy(), "{}", s.message);
                (s, None)
            }),
            ("psd.options", |_| {
                let mut s = AppState::new(32, 32);
                s.psd.options_open = true;
                (s, None)
            }),
            ("distribute", |_| {
                let mut s = AppState::new(32, 32);
                s.distribute.park_next = true;
                s.apply(Action::Distribute(DistributeAction::Start));
                assert!(s.distribute.is_busy(), "{}", s.message);
                (s, None)
            }),
            ("distribute.replace", |dir| {
                let mut s = AppState::new(32, 32);
                s.distribute.replace = Some(dir.join("a.ylp"));
                (s, None)
            }),
            ("brush-import", |dir| {
                let mut s = AppState::new(32, 32);
                s.attach_brush_store(dir.join("brushes"));
                s.brushes.import.park_next = true;
                s.apply(Action::Brush(BrushAction::Import(vec![dir.join("a.gbr")])));
                assert!(s.is_brush_importing(), "{}", s.message);
                (s, None)
            }),
            ("save", |dir| {
                let mut s = AppState::new(32, 32);
                s.save.background = true;
                let hold = s.save.hold_next();
                crate::project::save_from(&mut s, &dir.join("a.ylp"));
                assert!(s.is_saving(), "{}", s.message);
                (s, Some(hold))
            }),
            ("save.quit", |dir| {
                let mut s = AppState::new(32, 32);
                s.save.background = true;
                let hold = s.save.hold_next();
                crate::project::save_from(&mut s, &dir.join("a.ylp"));
                assert!(s.is_saving(), "{}", s.message);
                s.quit = true;
                (s, Some(hold))
            }),
            ("model.window", |_| {
                let mut s = AppState::new(32, 32);
                s.apply(Action::Project(NpAction::OpenNew));
                assert!(s.np.window.is_some());
                (s, None)
            }),
            ("model.loading", |_| {
                let mut s = AppState::new(32, 32);
                s.apply(Action::Project(NpAction::OpenNew));
                let (job, hold) = crate::view3d::pose::PrepareJob::parked();
                // 送り口を捨てない（読み込み中のまま）。試験の終わりまで持つ
                std::mem::forget(hold);
                s.np.window.as_mut().unwrap().prep = Prep::Loading {
                    path: "m.fbx".into(),
                    job,
                };
                assert!(s.np_is_busy());
                (s, None)
            }),
            ("model.remove", |_| {
                let mut s = AppState::new(32, 32);
                s.np.remove_confirm = Some(vec![1]);
                (s, None)
            }),
            ("brushes.csp", |dir| {
                let mut s = AppState::new(32, 32);
                s.brushes.csp.park_next = true;
                s.apply(Action::Brush(BrushAction::ClipStudioFolder(
                    dir.to_path_buf(),
                )));
                assert!(s.brushes.csp.is_busy());
                (s, None)
            }),
            ("brushes.csp.open", |_| {
                let mut s = AppState::new(32, 32);
                s.brushes.csp.open = true;
                (s, None)
            }),
            ("recovery", |dir| {
                let mut s = AppState::new(32, 32);
                s.recovery.window = Some(crate::recovery::window::WindowState {
                    confirm: Some(crate::recovery::Row {
                        pool: dir.to_path_buf(),
                        id: "1".into(),
                        time_ms: None,
                        documents: 1,
                        name: String::new(),
                        crashed: false,
                        own: false,
                        problem: None,
                    }),
                    ..Default::default()
                });
                (s, None)
            }),
            ("layers.merge", |_| {
                let mut s = AppState::new(32, 32);
                s.layer_ops.merge_confirm = Some(crate::layerops::MergeConfirm {
                    op: crate::layerops::MergeOp::Visible,
                    channels: Vec::new(),
                });
                (s, None)
            }),
            ("library.write", |dir| {
                let mut s = AppState::new(32, 32);
                s.prefs.settings.library_folder = Some(dir.join("library"));
                s.shelf.show_builtin = false;
                s.library.hold_writes(true);
                let id = s
                    .shelf
                    .add_image(Lang::Ja, "試験の画像", &[1, 2, 3, 255], 1, 1)
                    .unwrap();
                s.apply(Action::Shelf(ShelfOp::ToLibrary(id)));
                assert!(s.library.writing_name().is_some(), "{}", s.message);
                (s, None)
            }),
            ("shelf", |_| {
                let mut s = AppState::new(32, 32);
                let layer = s.selected_layer.unwrap();
                s.shelf.async_bytes = 0;
                s.shelf.hold_saves(true);
                s.apply(Action::Shelf(ShelfOp::SaveMaterial(layer)));
                assert_eq!(s.shelf.pending_save(), Some(false), "{}", s.message);
                (s, None)
            }),
        ];
        for (name, make) in cases {
            let dir = temp_dir(name);
            let (mut s, hold) = make(&dir);
            assert_eq!(
                close_jobs(&s),
                before_close_jobs(&s),
                "{name}: 閉じる前の確かめ"
            );
            assert_eq!(repaint_needed(&s), before_repaint(&s), "{name}: 描き直し");
            assert_eq!(
                modal_open(&s),
                before_modal_open(&s),
                "{name}: 確認のウィンドウ"
            );
            let ids: Vec<&str> = cards(&s, Lang::Ja).into_iter().map(|(id, _)| id).collect();
            assert_eq!(ids, before_card_ids(&s), "{name}: 札");
            // 後始末: 止めた仕事・止めておいた手を放す
            s.library.hold_writes(false);
            s.shelf.hold_saves(false);
            if let Some(hold) = hold {
                hold.release();
                s.wait_save();
            }
            stop(&mut s, Duration::from_secs(30));
            assert!(close_jobs(&s).is_empty(), "{name}: 止めた");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// 札の文は、寄せる前の `windows::job_card` と同じ（表の札の行を、走っている仕事で読む）。
    #[test]
    fn the_cards_read_like_before() {
        use crate::psd::PsdAction;
        let dir = temp_dir("cards");
        let mut s = AppState::new(32, 32);
        s.psd.park_next = true;
        s.apply(Action::Psd(PsdAction::Export(dir.join("out.psd"))));
        let cards = cards(&s, Lang::En);
        assert_eq!(cards.len(), 1);
        let (id, card) = &cards[0];
        assert_eq!(*id, "psd");
        assert_eq!(card.text, "Writing PSD — out.psd");
        assert_eq!(card.fraction, None);
        assert!(!card.canceling);
        assert!(matches!(card.cancel, Some(Action::Psd(PsdAction::Cancel))));
        stop(&mut s, Duration::from_secs(30));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
